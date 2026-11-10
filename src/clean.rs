//! Repairs ugly Markdown so it is consistent and readable, without changing meaning.
//!
//! Fixes: line endings, invisible characters, HTML comments, `<br>` litter, trailing
//! spaces, `##Heading` without a space, underlined (setext) headings, skipped heading
//! levels, mixed bullet styles, hard-wrapped paragraphs, ragged tables, runs of blank
//! lines, page-number markers, and headers/footers repeated on every page.
//! Fenced code blocks are never touched.

use regex::Regex;
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct CleanReport {
    /// Repeated standalone lines that were removed, with how many times each appeared.
    pub boilerplate: Vec<(String, usize)>,
    pub comments_removed: usize,
    pub page_markers: usize,
    pub headings_fixed: usize,
    pub tables_fixed: usize,
}

impl CleanReport {
    pub fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        let bp: usize = self.boilerplate.iter().map(|(_, n)| n).sum();
        if bp > 0 {
            parts.push(format!("{bp} repeated header/footer lines"));
        }
        if self.page_markers > 0 {
            parts.push(format!("{} page markers", self.page_markers));
        }
        if self.comments_removed > 0 {
            parts.push(format!("{} hidden comments", self.comments_removed));
        }
        let mut s = String::new();
        if !parts.is_empty() {
            s.push_str(&format!("removed {}", parts.join(", ")));
        }
        let mut fixed = Vec::new();
        if self.headings_fixed > 0 {
            fixed.push(format!("{} headings", self.headings_fixed));
        }
        if self.tables_fixed > 0 {
            fixed.push(format!("{} tables", self.tables_fixed));
        }
        if !fixed.is_empty() {
            if !s.is_empty() {
                s.push_str("; ");
            }
            s.push_str(&format!("repaired {}", fixed.join(", ")));
        }
        (!s.is_empty()).then_some(s)
    }
}

struct Line {
    text: String,
    fenced: bool,
}

pub fn clean(input: &str) -> (String, CleanReport) {
    let mut report = CleanReport::default();
    let mut lines = normalize(input, &mut report);
    drop_page_markers(&mut lines, &mut report);
    drop_boilerplate(&mut lines, &mut report);
    fix_headings(&mut lines, &mut report);
    fix_bullets(&mut lines);
    fix_tables(&mut lines, &mut report);
    let lines = unwrap_paragraphs(lines);
    (finish(lines), report)
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Line endings, invisible characters, comments, `<br>`, trailing whitespace.
fn normalize(input: &str, report: &mut CleanReport) -> Vec<Line> {
    let text = input
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{a0}', " ")
        .replace(['\u{200b}', '\u{feff}', '\u{ad}'], "");
    let br = Regex::new(r"(?i)<br\s*/?>").unwrap();
    let mut out = Vec::new();
    let (mut in_fence, mut in_comment) = (false, false);
    for raw in text.lines() {
        if in_fence {
            if is_fence(raw) {
                in_fence = false;
            }
            out.push(Line {
                text: raw.to_string(),
                fenced: true,
            });
            continue;
        }
        let mut line = raw.to_string();
        // Strip HTML comments, possibly spanning lines.
        loop {
            if in_comment {
                match line.find("-->") {
                    Some(end) => {
                        line = line[end + 3..].to_string();
                        in_comment = false;
                    }
                    None => {
                        line.clear();
                        break;
                    }
                }
            } else if let Some(start) = line.find("<!--") {
                report.comments_removed += 1;
                match line[start..].find("-->") {
                    Some(rel) => line = format!("{}{}", &line[..start], &line[start + rel + 3..]),
                    None => {
                        line.truncate(start);
                        in_comment = true;
                        break;
                    }
                }
            } else {
                break;
            }
        }
        if is_fence(&line) {
            in_fence = true;
            out.push(Line {
                text: line.trim_end().to_string(),
                fenced: true,
            });
            continue;
        }
        let line = br.replace_all(&line, " ").trim_end().to_string();
        out.push(Line {
            text: line,
            fenced: false,
        });
    }
    out
}

fn drop_page_markers(lines: &mut Vec<Line>, report: &mut CleanReport) {
    let re = Regex::new(r"(?i)^\s*(page\s+\d+(\s+(of|/)\s+\d+)?|-\s*\d+\s*-|\d+\s*/\s*\d+)\s*$")
        .unwrap();
    let before = lines.len();
    lines.retain(|l| l.fenced || !re.is_match(&l.text));
    report.page_markers += before - lines.len();
}

/// A header or footer printed on every page: the same standalone paragraph, many times.
fn drop_boilerplate(lines: &mut Vec<Line>, report: &mut CleanReport) {
    let standalone = |i: usize, lines: &[Line]| -> Option<String> {
        let l = &lines[i];
        let t = l.text.trim();
        if l.fenced || t.len() < 12 || t.len() > 200 {
            return None;
        }
        if t.starts_with(['#', '-', '*', '+', '|', '>', '<', '`'])
            || t.chars().next()?.is_ascii_digit()
        {
            return None;
        }
        // Page headers and footers are labels, not sentences: a repeated line that ends like a
        // sentence is real content (a refrain, a repeated warning) and must be kept.
        if t.ends_with(['.', '!', '?', '"', '\u{201d}', ')']) {
            return None;
        }
        let blank = |j: Option<usize>| {
            j.is_none_or(|j| lines.get(j).is_none_or(|x| x.text.trim().is_empty()))
        };
        (blank(i.checked_sub(1)) && blank(Some(i + 1))).then(|| t.to_lowercase())
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    for i in 0..lines.len() {
        if let Some(k) = standalone(i, lines) {
            *counts.entry(k).or_default() += 1;
        }
    }
    counts.retain(|_, n| *n >= 4);
    if counts.is_empty() {
        return;
    }
    // Real page furniture is spread through the document (a gap of several lines between
    // repeats), not clustered in one spot.
    let spread = |key: &str| -> bool {
        let at: Vec<usize> = (0..lines.len())
            .filter(|&i| standalone(i, lines).as_deref() == Some(key))
            .collect();
        at.last()
            .zip(at.first())
            .is_some_and(|(l, f)| (l - f) >= 5 * (at.len() - 1))
    };
    let keys: Vec<String> = counts.keys().cloned().collect();
    for k in keys {
        if !spread(&k) {
            counts.remove(&k);
        }
    }
    if counts.is_empty() {
        return;
    }
    let mut originals: HashMap<String, String> = HashMap::new();
    let mut keep = Vec::with_capacity(lines.len());
    for i in 0..lines.len() {
        match standalone(i, lines) {
            Some(k) if counts.contains_key(&k) => {
                originals
                    .entry(k)
                    .or_insert_with(|| lines[i].text.trim().to_string());
                keep.push(false);
            }
            _ => keep.push(true),
        }
    }
    let mut it = keep.iter();
    lines.retain(|_| *it.next().unwrap());
    let mut found: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(k, n)| (originals.remove(&k).unwrap_or(k), n))
        .collect();
    found.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    report.boilerplate = found;
}

/// `##Title`, closing hashes, setext headings, skipped levels.
fn fix_headings(lines: &mut Vec<Line>, report: &mut CleanReport) {
    let atx = Regex::new(r"^(#{1,6})\s*(\S.*?)\s*#*\s*$").unwrap();
    let eq = Regex::new(r"^=+\s*$").unwrap();
    let dash = Regex::new(r"^-{3,}\s*$").unwrap();

    // Setext -> ATX.
    let mut i = 0;
    while i + 1 < lines.len() {
        let (a, b) = (&lines[i], &lines[i + 1]);
        let prev_blank = i == 0 || lines[i - 1].text.trim().is_empty();
        let plain = !a.fenced
            && !a.text.trim().is_empty()
            && a.text.len() <= 100
            && !a
                .text
                .trim_start()
                .starts_with(['#', '-', '*', '+', '|', '>', '<', '`'])
            && prev_blank;
        if plain && !b.fenced {
            let level = if eq.is_match(&b.text) {
                Some(1)
            } else if dash.is_match(&b.text) {
                Some(2)
            } else {
                None
            };
            if let Some(level) = level {
                lines[i].text = format!("{} {}", "#".repeat(level), lines[i].text.trim());
                lines.remove(i + 1);
                report.headings_fixed += 1;
            }
        }
        i += 1;
    }

    // ATX repair + level normalization.
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut base: Option<usize> = None;
    for l in lines.iter_mut().filter(|l| !l.fenced) {
        if !l.text.starts_with('#') {
            continue;
        }
        let Some(c) = atx.captures(&l.text) else {
            continue;
        };
        let (hashes, title) = (c[1].len(), c[2].to_string());
        let well_formed = l.text.starts_with(&format!("{} ", &c[1]));
        if !well_formed && !title.chars().next().is_some_and(char::is_alphanumeric) {
            continue;
        }
        let base_level = *base.get_or_insert(hashes);
        while stack.last().is_some_and(|&(orig, _)| orig >= hashes) {
            stack.pop();
        }
        let new = stack.last().map_or(base_level, |&(_, n)| (n + 1).min(6));
        stack.push((hashes, new));
        let fixed = format!("{} {}", "#".repeat(new), title);
        if fixed != l.text {
            report.headings_fixed += 1;
            l.text = fixed;
        }
    }
}

fn fix_bullets(lines: &mut [Line]) {
    let bullet = Regex::new(r"^(\s*)[*+•·‣▪]\s+(\S.*)$").unwrap();
    let wide = Regex::new(r"^(\s*)-\s{2,}(\S.*)$").unwrap();
    for l in lines.iter_mut().filter(|l| !l.fenced) {
        if let Some(c) = bullet.captures(&l.text).or_else(|| wide.captures(&l.text)) {
            l.text = format!("{}- {}", &c[1], &c[2]);
        }
    }
}

fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            cur.push_str("\\|");
            chars.next();
        } else if c == '|' {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn fix_tables(lines: &mut Vec<Line>, report: &mut CleanReport) {
    let sep = Regex::new(r"^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$").unwrap();
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if lines[i].fenced || !lines[i].text.trim_start().starts_with('|') {
            out.push(std::mem::replace(
                &mut lines[i],
                Line {
                    text: String::new(),
                    fenced: false,
                },
            ));
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && !lines[i].fenced && lines[i].text.trim_start().starts_with('|') {
            i += 1;
        }
        let rows: Vec<Vec<String>> = lines[start..i]
            .iter()
            .enumerate()
            .filter(|(k, l)| {
                !(*k == 1 && sep.is_match(&l.text)) && !(*k > 1 && sep.is_match(&l.text))
            })
            .map(|(_, l)| split_row(&l.text))
            .collect();
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut block = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            let mut cells = row.clone();
            cells.resize(width, String::new());
            block.push(format!("| {} |", cells.join(" | ")));
            if r == 0 {
                block.push(format!("|{}", " --- |".repeat(width)));
            }
        }
        let original: Vec<&str> = lines[start..i].iter().map(|l| l.text.as_str()).collect();
        if original != block.iter().map(String::as_str).collect::<Vec<_>>() {
            report.tables_fixed += 1;
        }
        // A table must start on its own paragraph.
        if out.last().is_some_and(|l| !l.text.trim().is_empty()) {
            out.push(Line {
                text: String::new(),
                fenced: false,
            });
        }
        out.extend(block.into_iter().map(|text| Line {
            text,
            fenced: false,
        }));
    }
    *lines = out;
}

/// Join hard-wrapped paragraph lines into single lines.
fn unwrap_paragraphs(lines: Vec<Line>) -> Vec<String> {
    let list = Regex::new(r"^\s*([-*+]|\d+[.)])\s").unwrap();
    let is_plain = |l: &Line| -> bool {
        let t = l.text.as_str();
        !l.fenced
            && !t.trim().is_empty()
            && !t.starts_with("    ")
            && !t.starts_with('\t')
            && !t.trim_start().starts_with(['#', '|', '>', '<'])
            && !list.is_match(t)
            && !t.trim_start().starts_with("---")
    };
    let mut out: Vec<String> = Vec::new();
    let mut prev_plain = false;
    for l in lines {
        if is_plain(&l) {
            let collapsed = l.text.split_whitespace().collect::<Vec<_>>().join(" ");
            if prev_plain {
                let last = out.last_mut().unwrap();
                last.push(' ');
                last.push_str(&collapsed);
            } else {
                out.push(collapsed);
            }
            prev_plain = true;
        } else {
            out.push(l.text);
            prev_plain = false;
        }
    }
    out
}

/// Single blank lines, blank lines around headings, one trailing newline.
fn finish(lines: Vec<String>) -> String {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut in_fence = false;
    for line in lines {
        if is_fence(&line) {
            in_fence = !in_fence;
        }
        let blank = line.trim().is_empty() && !in_fence;
        let heading = !in_fence && line.starts_with('#');
        if blank {
            if out.last().is_none_or(|l| l.is_empty()) {
                continue;
            }
            out.push(String::new());
            continue;
        }
        if heading && out.last().is_some_and(|l| !l.is_empty()) {
            out.push(String::new());
        }
        let after_heading = out
            .last()
            .is_some_and(|l| l.starts_with("# ") || l.starts_with("##"));
        if after_heading && !heading {
            out.push(String::new());
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_headings_levels_and_setext() {
        let (md, r) = clean("Title\n=====\n\n##Sub\n\ntext\n\n#### Deep\n\nbody\n");
        assert!(md.starts_with("# Title\n\n## Sub\n"), "{md}");
        assert!(md.contains("\n### Deep\n"), "{md}");
        assert!(r.headings_fixed >= 3);
    }

    #[test]
    fn keeps_relative_levels_when_doc_starts_deep() {
        let (md, _) = clean("## A\n\nx\n\n## B\n\ny\n");
        assert!(md.contains("## A") && md.contains("## B"));
    }

    #[test]
    fn unwraps_and_normalizes() {
        let (md, _) = clean("one two  \nthree\n\n\n\n* a\n+ b\n•   c\n");
        assert_eq!(md, "one two three\n\n- a\n- b\n- c\n");
    }

    #[test]
    fn removes_comments_markers_and_repeated_footers() {
        let mut src = String::from("<!-- hidden -->\nreal text here\n\n");
        for i in 1..=5 {
            src.push_str(&format!(
                "Page {i} of 9\n\nCONFIDENTIAL - do not share\n\nbody {i}\n\n"
            ));
        }
        let (md, r) = clean(&src);
        assert!(!md.contains("CONFIDENTIAL") && !md.contains("Page 1") && !md.contains("hidden"));
        assert!(md.contains("body 3"));
        assert_eq!(r.boilerplate[0].1, 5);
        assert_eq!(r.page_markers, 5);
    }

    #[test]
    fn repeated_sentences_are_content_not_page_furniture() {
        let mut src = String::new();
        for i in 0..8 {
            src.push_str(&format!(
                "Always wrap every call in pcall before saving.\n\nSection {i} body text here.\n\n"
            ));
        }
        let (md, r) = clean(&src);
        assert_eq!(
            md.matches("Always wrap every call in pcall before saving.")
                .count(),
            8,
            "{md}"
        );
        assert!(r.boilerplate.is_empty());
    }

    #[test]
    fn never_touches_code_fences() {
        let src = "```\n<!-- keep -->\n  *  spaced   \n```\n";
        let (md, _) = clean(src);
        assert!(md.contains("<!-- keep -->") && md.contains("  *  spaced   "));
    }

    #[test]
    fn repairs_ragged_tables() {
        let (md, r) = clean("| a | b | c\n|---|---|\n| 1 | 2 |\n");
        assert_eq!(md, "| a | b | c |\n| --- | --- | --- |\n| 1 | 2 |  |\n");
        assert_eq!(r.tables_fixed, 1);
    }
}
