//! Compression levels: the document itself, shortened, keeping its headings and formatting.
//!
//! Level 1 keeps everything. Levels 2-9 keep the most informative sentences of every section
//! (scored the same way as the summary), in their original order, until the section reaches its
//! share of words. Repeated sentences go first, tables are kept up to level 3 and code up to level
//! 4. Level 10 keeps no body at all (summary only); callers handle that case.

use super::{nlp, points_back};
use std::collections::{HashMap, HashSet};

/// Share of words kept at each level, 1 to 10.
pub const KEEP: [f64; 10] = [1.0, 0.8, 0.65, 0.5, 0.4, 0.3, 0.2, 0.12, 0.07, 0.0];
const TABLES_UP_TO: u8 = 3;
/// From this level, sentences with the same words but different numbers count as repeats.
const SAME_WORDS_FROM: u8 = 5;
/// From this level, near-duplicates (mostly the same words) count as repeats.
const NEAR_FROM: u8 = 7;
const CODE_UP_TO: u8 = 4;

pub struct Compressed {
    pub markdown: String,
    pub words_before: usize,
    pub words_after: usize,
    pub tables_dropped: usize,
    pub code_dropped: usize,
}

enum Block {
    Heading {
        level: usize,
        line: String,
    },
    /// A paragraph (several sentences) or one list item (one unit).
    Prose {
        units: Vec<String>,
        item: Option<String>,
    },
    Table(Vec<String>),
    Code(Vec<String>),
    Other(String),
}

fn word_count(s: &str) -> usize {
    s.split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
}

fn blocks(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut lines = md.lines().peekable();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            let text = para.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
            out.push(Block::Prose {
                units: nlp::sentences(&text),
                item: None,
            });
            para.clear();
        }
    };
    while let Some(line) = lines.next() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            flush(&mut para, &mut out);
            let fence = &t[..3];
            let mut code = vec![line.to_string()];
            for l in lines.by_ref() {
                code.push(l.to_string());
                if l.trim_start().starts_with(fence) {
                    break;
                }
            }
            out.push(Block::Code(code));
            continue;
        }
        if t.is_empty() {
            flush(&mut para, &mut out);
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut para, &mut out);
            out.push(Block::Heading {
                level: hashes,
                line: line.to_string(),
            });
            continue;
        }
        if t.starts_with('|') {
            flush(&mut para, &mut out);
            let mut rows = vec![line.to_string()];
            while lines
                .peek()
                .is_some_and(|l| l.trim_start().starts_with('|'))
            {
                rows.push(lines.next().unwrap().to_string());
            }
            out.push(Block::Table(rows));
            continue;
        }
        let marker = ["- ", "* ", "+ "]
            .iter()
            .find(|m| t.starts_with(**m))
            .map(|m| m.len())
            .or_else(|| {
                let d = t.chars().take_while(char::is_ascii_digit).count();
                (d > 0 && (t[d..].starts_with(". ") || t[d..].starts_with(") "))).then_some(d + 2)
            });
        if let Some(m) = marker {
            flush(&mut para, &mut out);
            let indent = &line[..line.len() - t.len()];
            let mut text = t[m..].trim().to_string();
            // Continuation lines of the same item.
            while lines.peek().is_some_and(|l| {
                l.starts_with("  ")
                    && !l.trim().is_empty()
                    && !l.trim_start().starts_with(['-', '*', '+', '|', '#'])
            }) {
                text.push(' ');
                text.push_str(lines.next().unwrap().trim());
            }
            out.push(Block::Prose {
                units: vec![text],
                item: Some(format!("{indent}{}", &t[..m])),
            });
            continue;
        }
        if t.starts_with('<') {
            flush(&mut para, &mut out);
            out.push(Block::Other(line.to_string()));
            continue;
        }
        para.push(line);
    }
    flush(&mut para, &mut out);
    out
}

pub fn compress(md: &str, level: u8) -> Compressed {
    let level = level.clamp(1, 10);
    let keep = KEEP[usize::from(level - 1)];
    let blocks = blocks(md);
    let words_before = word_count(md);
    if level == 1 {
        return Compressed {
            markdown: md.to_string(),
            words_before,
            words_after: words_before,
            tables_dropped: 0,
            code_dropped: 0,
        };
    }

    // Every prose unit: (block, index in block, section, tokens, words).
    struct Unit {
        block: usize,
        pos: usize,
        section: usize,
        toks: HashSet<String>,
        words: usize,
        first: bool,
        pointer: bool,
        /// Exact text (numbers included) and word-set keys, for spotting repeats.
        exact: String,
        words_key: Vec<String>,
    }
    let mut units: Vec<Unit> = Vec::new();
    let mut section = 0usize;
    for (b, block) in blocks.iter().enumerate() {
        match block {
            Block::Heading { .. } => section += 1,
            Block::Prose { units: us, .. } => {
                for (pos, u) in us.iter().enumerate() {
                    units.push(Unit {
                        block: b,
                        pos,
                        section,
                        toks: nlp::tokens(&nlp::plain(u)).into_iter().collect(),
                        words: word_count(u),
                        first: pos == 0,
                        pointer: points_back(&nlp::plain(u)),
                        exact: nlp::plain(u)
                            .to_lowercase()
                            .split(|c: char| !c.is_alphanumeric())
                            .filter(|w| !w.is_empty())
                            .collect::<Vec<_>>()
                            .join(" "),
                        words_key: {
                            let mut k: Vec<String> = nlp::tokens(&nlp::plain(u));
                            k.sort_unstable();
                            k.dedup();
                            k
                        },
                    });
                }
            }
            _ => {}
        }
    }

    // Word weights: frequency within the section, rarity across sections.
    let mut tf: HashMap<(usize, &str), u32> = HashMap::new();
    let mut df: HashMap<&str, usize> = HashMap::new();
    let mut seen: HashSet<(usize, &str)> = HashSet::new();
    for u in &units {
        for t in &u.toks {
            *tf.entry((u.section, t)).or_default() += 1;
            if seen.insert((u.section, t)) {
                *df.entry(t).or_default() += 1;
            }
        }
    }
    let sections = units
        .iter()
        .map(|u| u.section)
        .collect::<HashSet<_>>()
        .len()
        .max(1);
    let score = |u: &Unit| -> f64 {
        if u.toks.is_empty() {
            return 0.0;
        }
        let sum: f64 = u
            .toks
            .iter()
            .map(|t| {
                (1.0 + f64::from(tf[&(u.section, t.as_str())]).ln())
                    * (1.0 + sections as f64 / df[t.as_str()] as f64).ln()
            })
            .sum();
        let mut s = sum / (u.toks.len() as f64).powf(0.6);
        if u.words < 5 {
            s *= 0.4;
        } else if u.words > 45 {
            s *= 0.6;
        }
        if u.first {
            s *= 1.3;
        }
        if u.pointer {
            s *= 0.6;
        }
        s
    };

    // Repeats are decided once for the whole document (the first copy wins), so a higher level
    // never keeps more than a lower one. Strictness grows with the level.
    let mut repeat = vec![false; units.len()];
    let mut copies: HashMap<usize, usize> = HashMap::new();
    let mut first_exact: HashMap<&str, usize> = HashMap::new();
    let mut first_words: HashMap<&[String], usize> = HashMap::new();
    for (i, u) in units.iter().enumerate() {
        if u.exact.is_empty() {
            continue;
        }
        let earlier = first_exact.get(u.exact.as_str()).copied().or_else(|| {
            (level >= SAME_WORDS_FROM && u.words_key.len() >= 3)
                .then(|| first_words.get(u.words_key.as_slice()).copied())
                .flatten()
        });
        match earlier {
            Some(f) => {
                repeat[i] = true;
                *copies.entry(f).or_default() += 1;
            }
            None => {
                first_exact.insert(&u.exact, i);
                first_words.entry(&u.words_key).or_insert(i);
            }
        }
    }
    let boost = |i: usize| 1.0 + (0.2 * copies.get(&i).copied().unwrap_or(0) as f64).min(0.6);

    // Pick per section, best first, until the section reaches its share.
    let mut by_section: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, u) in units.iter().enumerate() {
        by_section.entry(u.section).or_default().push(i);
    }
    let mut kept: HashSet<usize> = HashSet::new();
    let mut sections_sorted: Vec<usize> = by_section.keys().copied().collect();
    sections_sorted.sort_unstable();
    for s in sections_sorted {
        let ids = &by_section[&s];
        let total: usize = ids.iter().map(|&i| units[i].words).sum();
        let target = (total as f64 * keep).ceil() as usize;
        let mut ranked: Vec<usize> = ids.clone();
        ranked.retain(|&i| !repeat[i]);
        ranked.sort_by(|&a, &b| {
            (score(&units[b]) * boost(b))
                .total_cmp(&(score(&units[a]) * boost(a)))
                .then(a.cmp(&b))
        });
        let mut got = 0usize;
        let mut chosen_here: Vec<&HashSet<String>> = Vec::new();
        for i in ranked {
            if got >= target && got > 0 {
                break;
            }
            let u = &units[i];
            if level >= NEAR_FROM && chosen_here.iter().any(|c| nlp::jaccard(c, &u.toks) > 0.6) {
                continue;
            }
            chosen_here.push(&u.toks);
            kept.insert(i);
            got += u.words;
        }
    }
    // A kept sentence that points back keeps the sentence it points to.
    for i in 0..units.len() {
        if kept.contains(&i) && units[i].pointer && !units[i].first && i > 0 {
            kept.insert(i - 1);
        }
    }

    // Which blocks keep content, so empty headings disappear.
    let mut keep_unit: HashMap<usize, Vec<usize>> = HashMap::new();
    for &i in &kept {
        keep_unit
            .entry(units[i].block)
            .or_default()
            .push(units[i].pos);
    }
    let block_has = |b: usize| -> bool {
        match &blocks[b] {
            Block::Prose { .. } => keep_unit.contains_key(&b),
            Block::Table(_) => level <= TABLES_UP_TO,
            Block::Code(_) => level <= CODE_UP_TO,
            Block::Other(_) => level <= TABLES_UP_TO,
            Block::Heading { .. } => false,
        }
    };
    let mut out: Vec<String> = Vec::new();
    let (mut tables_dropped, mut code_dropped) = (0, 0);
    let mut prev_item = false;
    for (b, block) in blocks.iter().enumerate() {
        match block {
            Block::Heading { level: hl, line } => {
                // Keep a heading only if something below it (before the next peer heading) survives.
                let has = (b + 1..blocks.len())
                    .take_while(
                        |&j| !matches!(blocks[j], Block::Heading { level: l, .. } if l <= *hl),
                    )
                    .any(block_has);
                if has {
                    out.push(String::new());
                    out.push(line.clone());
                    out.push(String::new());
                }
                prev_item = false;
            }
            Block::Prose { units: us, item } => {
                let Some(pos) = keep_unit.get(&b) else {
                    continue;
                };
                let mut pos = pos.clone();
                pos.sort_unstable();
                let text = pos
                    .iter()
                    .map(|&p| us[p].as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                match item {
                    Some(marker) => {
                        if !prev_item {
                            out.push(String::new());
                        }
                        out.push(format!("{marker}{text}"));
                        prev_item = true;
                    }
                    None => {
                        out.push(String::new());
                        out.push(text);
                        prev_item = false;
                    }
                }
            }
            Block::Table(rows) => {
                if level <= TABLES_UP_TO {
                    out.push(String::new());
                    out.extend(rows.iter().cloned());
                } else {
                    tables_dropped += 1;
                }
                prev_item = false;
            }
            Block::Code(lines) => {
                if level <= CODE_UP_TO {
                    out.push(String::new());
                    out.extend(lines.iter().cloned());
                } else {
                    code_dropped += 1;
                }
                prev_item = false;
            }
            Block::Other(line) => {
                if level <= TABLES_UP_TO {
                    out.push(String::new());
                    out.push(line.clone());
                }
                prev_item = false;
            }
        }
    }
    let mut markdown = String::new();
    let mut blank = true;
    for l in out {
        if l.is_empty() {
            if !blank {
                markdown.push('\n');
            }
            blank = true;
        } else {
            markdown.push_str(&l);
            markdown.push('\n');
            blank = false;
        }
    }
    let words_after = word_count(&markdown);
    Compressed {
        markdown: markdown.trim().to_string() + "\n",
        words_before,
        words_after,
        tables_dropped,
        code_dropped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> String {
        let mut md = String::from("# Handbook\n\n");
        let topics = [
            (
                "Saving",
                "Player data is saved to a **DataStore** every minute. Session locking prevents corrupted profiles across servers. Always wrap calls in [pcall](https://create.roblox.com) because requests are throttled. Budget checks avoid dropped writes during shutdown. A migration table versions every save safely.",
            ),
            (
                "Money",
                "Game passes unlock permanent perks for buyers. Developer products sell repeatable boosts to players. The economy needs coin sinks to avoid inflation. Price anchoring makes bundles feel cheap. Limited offers need a visible countdown.",
            ),
            (
                "Security",
                "Validate every RemoteEvent argument on the server. Rate limit remote calls per player to stop spam. Never trust the client for purchases or damage. Log failures so exploit attempts become visible. Sanity check reported speed against limits.",
            ),
        ];
        for (t, body) in topics {
            md.push_str(&format!("## {t}\n\n{body}\n\n- first {t} point about details\n- second {t} point about more\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```\ncode {t}\n```\n\n"));
        }
        md
    }

    #[test]
    fn level_one_keeps_everything() {
        let c = compress(&doc(), 1);
        assert_eq!(c.markdown, doc());
    }

    #[test]
    fn higher_levels_keep_fewer_words_but_every_section_keeps_something() {
        let mut last = usize::MAX;
        for level in 2..=9 {
            let c = compress(&doc(), level);
            assert!(
                c.words_after <= last,
                "level {level}: {} > {last}",
                c.words_after
            );
            last = c.words_after;
            for h in ["## Saving", "## Money", "## Security"] {
                assert!(
                    c.markdown.contains(h),
                    "level {level} lost {h}:\n{}",
                    c.markdown
                );
            }
        }
        assert!(compress(&doc(), 9).words_after * 3 < compress(&doc(), 2).words_after);
    }

    #[test]
    fn formatting_survives_and_tables_then_code_are_dropped() {
        let c = compress(&doc(), 2);
        assert!(
            c.markdown.contains("**DataStore**")
                || c.markdown.contains("[pcall](https://create.roblox.com)"),
            "{}",
            c.markdown
        );
        assert!(c.markdown.contains("| a | b |") && c.markdown.contains("```"));
        let c = compress(&doc(), 4);
        assert!(
            !c.markdown.contains("| a | b |")
                && c.markdown.contains("```")
                && c.tables_dropped == 3
        );
        let c = compress(&doc(), 5);
        assert!(!c.markdown.contains("```") && c.code_dropped == 3);
    }

    #[test]
    fn same_words_with_different_numbers_are_kept_at_low_levels() {
        let md = "## A\n\nThe service throttles at 15 requests per minute today.\n\n## B\n\nThe service throttles at 5 requests per minute today.\n";
        assert!(
            compress(md, 2).markdown.contains("at 5 requests"),
            "different data is not a repeat at level 2"
        );
        assert!(
            !compress(md, 5).markdown.contains("at 5 requests"),
            "from level 5 it is"
        );
    }

    #[test]
    fn repeated_sentences_are_kept_once() {
        let md = "## A\n\nSession locking prevents corrupted profiles across servers.\n\n## B\n\nSession locking prevents corrupted profiles across servers. Game passes unlock permanent perks for buyers.\n";
        let c = compress(md, 2);
        assert_eq!(
            c.markdown.matches("Session locking").count(),
            1,
            "{}",
            c.markdown
        );
    }
}
