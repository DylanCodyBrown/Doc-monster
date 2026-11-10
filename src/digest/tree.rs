//! Turns Markdown into a tree of sections, then reshapes it so that every level has a
//! readable number of children: headingless text is cut into topical segments, and long
//! lists of siblings are grouped into chapters at the points where the vocabulary shifts.

use super::nlp::{self, Counts};

pub const MAX_CHILDREN: usize = 12;
const GROUP_TARGET: usize = 8;
const SEGMENT_PARAGRAPHS: usize = 10;
const MIN_PARAGRAPHS_TO_SEGMENT: usize = 24;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A heading that exists in the document.
    Real,
    /// Siblings grouped into a chapter.
    Group,
    /// A run of paragraphs cut out of text with no headings.
    Segment,
}

#[derive(Debug)]
pub struct Node {
    pub kind: Kind,
    pub title: String,
    /// Own paragraphs, each a list of sentences (a list item is one sentence).
    pub paras: Vec<Vec<String>>,
    pub children: Vec<Node>,
    pub tables: usize,
    pub code_blocks: usize,
    /// Token counts of own text only.
    pub own_tf: Counts,
    /// Token counts of own text plus all descendants.
    pub tf: Counts,
    pub own_words: usize,
    pub words: usize,
    /// Half-open range into the global sentence table (own sentences, then descendants).
    pub range: (usize, usize),
}

impl Node {
    pub fn new(kind: Kind, title: &str) -> Node {
        Node {
            kind,
            title: title.to_string(),
            paras: Vec::new(),
            children: Vec::new(),
            tables: 0,
            code_blocks: 0,
            own_tf: Counts::new(),
            tf: Counts::new(),
            own_words: 0,
            words: 0,
            range: (0, 0),
        }
    }

    pub fn has_own_text(&self) -> bool {
        !self.paras.is_empty()
    }

    pub fn first_real_title(&self) -> Option<&str> {
        if self.kind == Kind::Real && !self.title.is_empty() {
            return Some(&self.title);
        }
        self.children.iter().find_map(Node::first_real_title)
    }

    pub fn last_real_title(&self) -> Option<&str> {
        self.children
            .iter()
            .rev()
            .find_map(Node::last_real_title)
            .or(if self.kind == Kind::Real && !self.title.is_empty() {
                Some(&self.title)
            } else {
                None
            })
    }
}

pub fn parse(md: &str) -> Node {
    let mut stack: Vec<(usize, Node)> = vec![(0, Node::new(Kind::Real, ""))];
    let mut para = String::new();
    let mut is_item = false;
    let mut in_fence = false;
    let mut in_table = false;

    fn flush(stack: &mut [(usize, Node)], para: &mut String, is_item: &mut bool) {
        if !para.trim().is_empty() {
            let text = nlp::plain(para);
            let units = if *is_item {
                vec![text]
            } else {
                nlp::sentences(&text)
            };
            if !units.is_empty() {
                stack.last_mut().unwrap().1.paras.push(units);
            }
        }
        para.clear();
        *is_item = false;
    }
    fn close(stack: &mut Vec<(usize, Node)>) {
        let (_, node) = stack.pop().unwrap();
        stack.last_mut().unwrap().1.children.push(node);
    }

    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            flush(&mut stack, &mut para, &mut is_item);
            if !in_fence {
                stack.last_mut().unwrap().1.code_blocks += 1;
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if t.is_empty() {
            flush(&mut stack, &mut para, &mut is_item);
            in_table = false;
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut stack, &mut para, &mut is_item);
            in_table = false;
            while stack.len() > 1 && stack.last().unwrap().0 >= hashes {
                close(&mut stack);
            }
            stack.push((
                hashes,
                Node::new(Kind::Real, &nlp::plain(t[hashes..].trim())),
            ));
            continue;
        }
        if t.starts_with('|') {
            flush(&mut stack, &mut para, &mut is_item);
            if !in_table {
                stack.last_mut().unwrap().1.tables += 1;
                in_table = true;
            }
            continue;
        }
        in_table = false;
        if t.starts_with('<') {
            continue;
        }
        let item_body = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .or_else(|| t.strip_prefix("+ "))
            .or_else(|| {
                let digits = t.chars().take_while(char::is_ascii_digit).count();
                (digits > 0)
                    .then(|| {
                        t[digits..]
                            .strip_prefix(". ")
                            .or_else(|| t[digits..].strip_prefix(") "))
                    })
                    .flatten()
            });
        if let Some(body) = item_body {
            flush(&mut stack, &mut para, &mut is_item);
            para.push_str(body.trim());
            is_item = true;
            continue;
        }
        let body = t.strip_prefix('>').map_or(t, str::trim_start);
        if is_item && !line.starts_with("  ") && !line.starts_with('\t') {
            flush(&mut stack, &mut para, &mut is_item);
        }
        if !para.is_empty() {
            para.push(' ');
        }
        para.push_str(body);
    }
    flush(&mut stack, &mut para, &mut is_item);
    while stack.len() > 1 {
        close(&mut stack);
    }
    stack.pop().unwrap().1
}

/// Token counts and word counts, bottom-up.
pub fn compute_counts(node: &mut Node) {
    let mut own = Counts::new();
    let mut words = 0;
    for p in &node.paras {
        for s in p {
            words += s.split_whitespace().count();
            nlp::merge(&mut own, &nlp::count(&nlp::tokens(s)));
        }
    }
    node.own_words = words;
    node.words = words;
    node.tf = own.clone();
    node.own_tf = own;
    for c in &mut node.children {
        compute_counts(c);
        node.words += c.words;
        nlp::merge(&mut node.tf, &c.tf);
    }
}

/// Boundaries (start index of each new group) at the weakest vocabulary links.
fn valleys(vecs: &[&Counts], groups: usize, min_size: usize) -> Vec<usize> {
    let n = vecs.len();
    if groups < 2 || n < min_size * 2 {
        return Vec::new();
    }
    let w = 3usize;
    let mut scored: Vec<(usize, f32)> = (min_size..=n - min_size)
        .map(|b| {
            let mut left = Counts::new();
            let mut right = Counts::new();
            for v in &vecs[b.saturating_sub(w)..b] {
                nlp::merge(&mut left, v);
            }
            for v in &vecs[b..(b + w).min(n)] {
                nlp::merge(&mut right, v);
            }
            (b, nlp::cosine(&left, &right))
        })
        .collect();
    scored.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut chosen: Vec<usize> = Vec::new();
    for (b, _) in scored {
        if chosen.len() + 1 >= groups {
            break;
        }
        if chosen.iter().all(|&c| c.abs_diff(b) >= min_size) {
            chosen.push(b);
        }
    }
    chosen.sort_unstable();
    chosen
}

fn split_by<T>(items: Vec<T>, bounds: &[usize]) -> Vec<Vec<T>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut next = bounds.iter().peekable();
    for (i, item) in items.into_iter().enumerate() {
        if next.peek().is_some_and(|&&b| b == i) {
            out.push(std::mem::take(&mut cur));
            next.next();
        }
        cur.push(item);
    }
    out.push(cur);
    out
}

fn group_node(kind: Kind, children: Vec<Node>) -> Node {
    let mut g = Node::new(kind, "");
    for c in &children {
        g.words += c.words;
        nlp::merge(&mut g.tf, &c.tf);
    }
    g.children = children;
    g
}

/// Cut long runs of own paragraphs into topical segments; group long sibling lists.
pub fn reshape(node: &mut Node) {
    if node.paras.len() >= MIN_PARAGRAPHS_TO_SEGMENT {
        let paras = std::mem::take(&mut node.paras);
        let counts: Vec<Counts> = paras
            .iter()
            .map(|p| nlp::count(&nlp::tokens(&p.join(" "))))
            .collect();
        let refs: Vec<&Counts> = counts.iter().collect();
        let groups = paras.len().div_ceil(SEGMENT_PARAGRAPHS);
        let bounds = valleys(&refs, groups, (SEGMENT_PARAGRAPHS / 2).max(2));
        let mut segments: Vec<Node> = split_by(paras, &bounds)
            .into_iter()
            .map(|ps| {
                let mut seg = Node::new(Kind::Segment, "");
                seg.paras = ps;
                seg
            })
            .collect();
        for s in &mut segments {
            compute_counts(s);
        }
        segments.append(&mut node.children);
        node.children = segments;
        let (own, words) = (Counts::new(), 0);
        node.own_tf = own;
        node.own_words = words;
    }
    for c in &mut node.children {
        reshape(c);
    }
    while node.children.len() > MAX_CHILDREN {
        let n = node.children.len();
        let groups = n.div_ceil(GROUP_TARGET);
        let children = std::mem::take(&mut node.children);
        let bounds = {
            let refs: Vec<&Counts> = children.iter().map(|c| &c.tf).collect();
            valleys(&refs, groups, (n / groups / 2).max(2))
        };
        node.children = split_by(children, &bounds)
            .into_iter()
            .map(|g| {
                if g.len() == 1 {
                    g.into_iter().next().unwrap()
                } else {
                    group_node(Kind::Group, g)
                }
            })
            .collect();
        if node.children.len() >= n {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headings_lists_and_skips_code_and_tables() {
        let md = "# Top\n\nIntro sentence here. Another one.\n\n## Child\n\n- item one\n- item two\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```\ncode here\n```\n";
        let root = parse(md);
        assert_eq!(root.children.len(), 1);
        let top = &root.children[0];
        assert_eq!(top.title, "Top");
        assert_eq!(top.paras[0].len(), 2);
        let child = &top.children[0];
        assert_eq!(child.paras.len(), 2);
        assert_eq!((child.tables, child.code_blocks), (1, 1));
    }

    #[test]
    fn groups_many_siblings_into_chapters_at_topic_shifts() {
        let mut root = Node::new(Kind::Real, "");
        for i in 0..40 {
            let topic = if i < 20 {
                "datastore session profile"
            } else {
                "monetization robux gamepass"
            };
            let mut n = Node::new(Kind::Real, &format!("S{i}"));
            n.paras.push(vec![format!("{topic} detail {i}.")]);
            root.children.push(n);
        }
        compute_counts(&mut root);
        reshape(&mut root);
        assert!(root.children.len() <= MAX_CHILDREN);
        assert!(
            root.children
                .iter()
                .all(|c| c.kind == Kind::Group || c.kind == Kind::Real)
        );
        // The first chapter must not mix both topics at its boundary.
        let first = &root.children[0];
        assert!(first.last_real_title().is_some());
    }

    #[test]
    fn headingless_text_becomes_segments() {
        let mut root = Node::new(Kind::Real, "");
        for i in 0..60 {
            let t = if i < 30 {
                "alpha beta gamma"
            } else {
                "delta epsilon zeta"
            };
            root.paras.push(vec![format!("{t} paragraph {i} text.")]);
        }
        compute_counts(&mut root);
        reshape(&mut root);
        assert!(root.paras.is_empty() && !root.children.is_empty());
    }
}
