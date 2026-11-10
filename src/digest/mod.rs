//! Notebook-style hierarchical digest, computed locally with no model and no cost.
//!
//! How it works
//! 1. Parse the Markdown into a section tree (or cut headingless text into topical
//!    segments, and group very long sibling lists into chapters).
//! 2. Weigh every word by how distinctive it is to its section (term frequency x
//!    inverse section frequency), then score each sentence by the weight of its words.
//! 3. At every level, pick the top sentences while skipping near duplicates of anything
//!    already chosen anywhere in the digest.
//! 4. Render an overview, key phrases, and a nested outline whose depth adapts to size.

mod nlp;
mod tree;

use std::collections::{HashMap, HashSet};
use tree::{Kind, Node};

pub use nlp::plain as plain_text;

pub struct Digest {
    /// Markdown for the summary section.
    pub markdown: String,
    pub words_in: usize,
    /// The single most representative sentence of the whole document.
    pub lead: Option<String>,
}

/// First real sentence of a document, for when there is no digest.
pub fn first_sentence(markdown: &str) -> Option<String> {
    fn find(n: &Node) -> Option<String> {
        n.paras
            .iter()
            .flatten()
            .find(|s| s.split_whitespace().count() >= 4)
            .cloned()
            .or_else(|| n.children.iter().find_map(find))
    }
    find(&tree::parse(markdown))
}

struct Sent {
    text: String,
    toks: HashSet<String>,
    words: usize,
    first_in_para: bool,
}

struct Engine {
    sents: Vec<Sent>,
    idf: HashMap<String, f32>,
    /// Token sets of every sentence chosen so far, to avoid repeating ourselves.
    chosen: Vec<HashSet<String>>,
    used: HashSet<usize>,
    /// For each stem, how often each written form appeared (to show "always", not "alway").
    surface: HashMap<String, HashMap<String, u32>>,
    /// A sentence needs at least one word rarer than this, or it is generic filler.
    min_idf: f32,
}

const SIMILAR: f32 = 0.6;
const MAX_NODES_SHOWN: usize = 120;

type Surface = HashMap<String, HashMap<String, u32>>;

fn index(node: &mut Node, sents: &mut Vec<Sent>, bigram_counts: &mut nlp::Counts, surface: &mut Surface) {
    let start = sents.len();
    for para in &node.paras {
        for (j, s) in para.iter().enumerate() {
            for b in nlp::bigrams(s) {
                *bigram_counts.entry(b).or_default() += 1;
            }
            let pairs = nlp::tokens_surface(s);
            for (stem, written) in &pairs {
                *surface.entry(stem.clone()).or_default().entry(written.clone()).or_default() += 1;
            }
            sents.push(Sent {
                text: s.clone(),
                toks: pairs.into_iter().map(|(stem, _)| stem).collect(),
                words: s.split_whitespace().count(),
                first_in_para: j == 0,
            });
        }
    }
    for c in &mut node.children {
        index(c, sents, bigram_counts, surface);
    }
    node.range = (start, sents.len());
}

fn collect_df(node: &Node, df: &mut HashMap<String, u32>, docs: &mut u32) {
    if node.has_own_text() {
        *docs += 1;
        for t in node.own_tf.keys() {
            *df.entry(t.clone()).or_default() += 1;
        }
    }
    for c in &node.children {
        collect_df(c, df, docs);
    }
}

impl Engine {
    fn score(&self, i: usize, node: &Node) -> f32 {
        let s = &self.sents[i];
        if s.toks.is_empty() {
            return 0.0;
        }
        let sum: f32 = s
            .toks
            .iter()
            .map(|t| {
                let tf = node.tf.get(t).copied().unwrap_or(1) as f32;
                (1.0 + tf.ln()) * self.idf.get(t).copied().unwrap_or(0.5)
            })
            .sum();
        let mut score = sum / (s.toks.len() as f32).powf(0.6);
        if s.words < 5 {
            score *= 0.2;
        } else if s.words > 45 {
            score *= 0.5;
        }
        if s.first_in_para {
            score *= 1.1;
        }
        score
    }

    /// Highest scoring sentence of a subtree, without marking anything as used.
    fn best(&self, node: &Node) -> Option<String> {
        let (a, b) = node.range;
        (a..b)
            .filter(|&i| self.informative(i) && (6..=40).contains(&self.sents[i].words))
            .map(|i| (i, self.score(i, node)))
            .max_by(|x, y| x.1.total_cmp(&y.1).then(y.0.cmp(&x.0)))
            .map(|(i, _)| self.sents[i].text.clone())
    }

    fn informative(&self, i: usize) -> bool {
        self.min_idf == 0.0
            || self.sents[i]
                .toks
                .iter()
                .any(|t| self.idf.get(t).copied().unwrap_or(0.0) >= self.min_idf)
    }

    /// Readable form of a stem, or of a two-word phrase of stems.
    fn show(&self, stems: &str) -> String {
        stems
            .split(' ')
            .map(|st| {
                self.surface
                    .get(st)
                    .and_then(|m| m.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))))
                    .map_or_else(|| st.to_string(), |(w, _)| w.clone())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Choose up to `k` informative, mutually distinct sentences from a subtree.
    fn pick(&mut self, node: &Node, k: usize) -> Vec<usize> {
        let (a, b) = node.range;
        let mut cands: Vec<(usize, f32)> = (a..b)
            .filter(|i| !self.used.contains(i) && self.informative(*i))
            .map(|i| (i, self.score(i, node)))
            .filter(|(_, s)| *s > 0.0)
            .collect();
        cands.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
        cands.truncate(80);
        let mut picked: Vec<usize> = Vec::new();
        for (i, _) in cands {
            if picked.len() >= k {
                break;
            }
            let toks = &self.sents[i].toks;
            let dup = self.chosen.iter().any(|c| nlp::jaccard(c, toks) > SIMILAR);
            if !dup {
                self.chosen.push(toks.clone());
                picked.push(i);
            }
        }
        for i in &picked {
            self.used.insert(*i);
        }
        picked.sort_unstable();
        picked
    }

    fn keywords(&self, node: &Node, n: usize) -> Vec<String> {
        let mut v: Vec<(&String, f32)> = node
            .tf
            .iter()
            .filter(|(_, c)| **c >= 2 || node.tf.len() < 8)
            .map(|(t, &c)| (t, c as f32 * self.idf.get(t).copied().unwrap_or(0.5)))
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        v.into_iter().take(n).map(|(t, _)| self.show(t)).collect()
    }

    fn title(&self, node: &Node, ordinal: usize) -> String {
        match node.kind {
            Kind::Real => node.title.clone(),
            Kind::Group => match (node.first_real_title(), node.last_real_title()) {
                (Some(a), Some(b)) if a != b => format!("{a} → {b}"),
                (Some(a), _) => a.to_string(),
                _ => format!("Part {ordinal}: {}", self.keywords(node, 3).join(", ")),
            },
            Kind::Segment => format!("Part {ordinal}: {}", self.keywords(node, 3).join(", ")),
        }
    }
}

fn depth_budget(root: &Node) -> usize {
    let mut per_depth: Vec<usize> = Vec::new();
    fn walk(n: &Node, d: usize, per: &mut Vec<usize>) {
        if per.len() <= d {
            per.resize(d + 1, 0);
        }
        per[d] += 1;
        for c in &n.children {
            walk(c, d + 1, per);
        }
    }
    for c in &root.children {
        walk(c, 0, &mut per_depth);
    }
    let mut total = 0;
    let mut deepest = 0;
    for (d, n) in per_depth.iter().enumerate() {
        total += n;
        if total > MAX_NODES_SHOWN && d > 0 {
            break;
        }
        deepest = d;
    }
    deepest
}

fn render_node(e: &mut Engine, node: &Node, depth: usize, max_depth: usize, path: &str, ordinal: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let kws = e.keywords(node, 4);
    let title = e.title(node, ordinal);
    // Segments and chapters are already titled by their keywords.
    let kw = if kws.is_empty() || title.starts_with("Part ") { String::new() } else { format!(" · _{}_", kws.join(", ")) };
    out.push_str(&format!("{indent}- **{path} {title}**{kw} · {} words\n", fmt_num(node.words)));
    // A small section is already short: show all of it instead of summarizing it.
    let size = node.range.1 - node.range.0;
    let k = if size <= 10 && node.words < 200 { size } else { [3, 2, 1][depth.min(2)] };
    for i in e.pick(node, k) {
        out.push_str(&format!("{indent}  - {}\n", e.sents[i].text));
    }
    if depth < max_depth {
        for (n, c) in node.children.iter().enumerate() {
            render_node(e, c, depth + 1, max_depth, &format!("{path}.{}", n + 1), n + 1, out);
        }
    } else if !node.children.is_empty() {
        let titles: Vec<String> = node.children.iter().enumerate().map(|(n, c)| e.title(c, n + 1)).collect();
        let shown = titles.iter().take(8).cloned().collect::<Vec<_>>().join("; ");
        let more = titles.len().saturating_sub(8);
        let tail = if more > 0 { format!("; … (+{more} more)") } else { String::new() };
        out.push_str(&format!("{indent}  - _Also covers:_ {shown}{tail}\n"));
    }
}

fn describe(node: &Node) -> String {
    fn totals(n: &Node, acc: &mut (usize, usize, usize)) {
        acc.0 += usize::from(n.kind == Kind::Real && !n.title.is_empty());
        acc.1 += n.tables;
        acc.2 += n.code_blocks;
        for c in &n.children {
            totals(c, acc);
        }
    }
    let mut acc = (0, 0, 0);
    totals(node, &mut acc);
    let mins = (node.words as f32 / 220.0).ceil() as usize;
    let mut s = format!("{} words (about {} min read)", fmt_num(node.words), fmt_num(mins.max(1)));
    if acc.0 > 0 {
        s.push_str(&format!(", {} sections", acc.0));
    }
    if acc.1 > 0 {
        s.push_str(&format!(", {} tables", acc.1));
    }
    if acc.2 > 0 {
        s.push_str(&format!(", {} code blocks", acc.2));
    }
    s
}

fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Build the digest. Returns `None` for documents too short to be worth condensing.
pub fn digest(markdown: &str, cleanup_note: Option<&str>) -> Option<Digest> {
    let mut root = tree::parse(markdown);
    tree::compute_counts(&mut root);
    if root.words < 120 {
        return None;
    }
    tree::reshape(&mut root);

    let mut sents = Vec::new();
    let mut bigram_counts = nlp::Counts::new();
    let mut surface = Surface::new();
    index(&mut root, &mut sents, &mut bigram_counts, &mut surface);

    let (mut df, mut docs) = (HashMap::new(), 0u32);
    collect_df(&root, &mut df, &mut docs);
    let docs = docs.max(1);
    let idf: HashMap<String, f32> = df
        .into_iter()
        .map(|(t, d)| (t, (1.0 + docs as f32 / d as f32).ln()))
        .collect();
    let min_idf = if docs >= 6 { 4f32.ln() } else { 0.0 };
    let mut e = Engine { sents, idf, chosen: Vec::new(), used: HashSet::new(), surface, min_idf };

    // Skip wrapper nodes (a lone title heading holding everything).
    let mut eff = &root;
    while !eff.has_own_text() && eff.children.len() == 1 {
        eff = &eff.children[0];
    }
    let lead = e.best(&root);
    let doc_title = eff.first_real_title().unwrap_or("").to_string();

    let mut out = String::new();
    out.push_str("### At a glance\n\n");
    out.push_str(&format!("{}", describe(&root)));
    if !doc_title.is_empty() {
        out.push_str(&format!(" · {doc_title}"));
    }
    out.push_str("\n\n");

    // Overview: the single most representative sentence from each top-level part.
    out.push_str("### Overview\n\n");
    let parts: Vec<&Node> = if eff.children.len() >= 3 { eff.children.iter().collect() } else { vec![eff] };
    let mut overview: Vec<usize> = Vec::new();
    let want = (root.words / 1500).clamp(4, 10);
    if parts.len() == 1 {
        overview = e.pick(parts[0], want.min(6));
    } else {
        let mut by_size: Vec<&Node> = parts.clone();
        by_size.sort_by(|a, b| b.words.cmp(&a.words));
        // One sentence per part first, then extra ones from the biggest parts.
        for p in &parts {
            overview.extend(e.pick(p, 1));
        }
        'extra: loop {
            let before = overview.len();
            for p in &by_size {
                if overview.len() >= want {
                    break 'extra;
                }
                overview.extend(e.pick(p, 1));
            }
            if overview.len() == before {
                break;
            }
        }
        overview.sort_unstable();
    }
    for i in &overview {
        out.push_str(&format!("- {}\n", e.sents[*i].text));
    }
    out.push('\n');

    // Key phrases: frequent two-word phrases, then distinctive single words.
    let phrase_score = |p: &str, c: u32| -> f32 {
        let w: Vec<&str> = p.split(' ').collect();
        c as f32 * (w.iter().map(|x| e.idf.get(*x).copied().unwrap_or(0.5)).sum::<f32>() / w.len() as f32).sqrt()
    };
    let mut phrases: Vec<(&String, f32)> = bigram_counts
        .iter()
        .filter(|(_, c)| **c >= 3)
        .map(|(p, &c)| (p, phrase_score(p, c)))
        .collect();
    phrases.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    let top_phrases: Vec<&String> = phrases.into_iter().take(8).map(|(p, _)| p).collect();
    let covered: HashSet<&str> = top_phrases.iter().flat_map(|t| t.split(' ')).collect();
    let mut terms: Vec<String> = top_phrases.iter().map(|p| e.show(p)).collect();
    let single: Vec<String> = e
        .keywords(&root, 24)
        .into_iter()
        .filter(|k| !covered.contains(k.as_str()))
        .take(8)
        .collect();
    terms.extend(single);
    if !terms.is_empty() {
        out.push_str("### Key terms\n\n");
        out.push_str(&terms.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(" · "));
        out.push_str("\n\n");
    }

    // Outline.
    let max_depth = depth_budget(eff);
    out.push_str("### Outline\n\n");
    if eff.children.is_empty() {
        for i in e.pick(eff, 6) {
            out.push_str(&format!("- {}\n", e.sents[i].text));
        }
    } else {
        for (n, c) in eff.children.iter().enumerate() {
            render_node(&mut e, c, 0, max_depth, &format!("{}", n + 1), n + 1, &mut out);
        }
    }
    if let Some(note) = cleanup_note {
        out.push_str(&format!("\n_Cleanup: {note}._\n"));
    }
    Some(Digest { markdown: out, words_in: root.words, lead })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        let mut md = String::from("# Handbook\n\n");
        for (title, body) in [
            ("Saving data", "Player data is saved to a DataStore every minute. Session locking prevents corruption of profiles. Always wrap DataStore calls in pcall because requests are throttled."),
            ("Money", "Game passes unlock permanent perks for buyers. Developer products sell repeatable boosts to players. The economy needs coin sinks to avoid inflation of prices."),
            ("Security", "Validate every RemoteEvent argument on the server always. Rate limit remote calls per player to stop exploit spam. Never trust the client for purchases or damage."),
        ] {
            md.push_str(&format!("## {title}\n\n"));
            for _ in 0..6 {
                md.push_str(body);
                md.push_str("\n\n");
            }
        }
        md
    }

    #[test]
    fn digest_has_overview_terms_and_outline_per_section() {
        let d = digest(&sample(), None).unwrap();
        for needle in ["### Overview", "### Key terms", "### Outline", "Saving data", "Money", "Security"] {
            assert!(d.markdown.contains(needle), "missing {needle}\n{}", d.markdown);
        }
    }

    #[test]
    fn duplicate_sentences_are_not_repeated() {
        let d = digest(&sample(), None).unwrap();
        assert_eq!(d.markdown.matches("Session locking prevents").count(), 1, "{}", d.markdown);
    }

    #[test]
    fn short_documents_are_skipped() {
        assert!(digest("# Tiny\n\nJust a little text.\n", None).is_none());
    }
}
