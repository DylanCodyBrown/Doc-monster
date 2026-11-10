//! INDEX.md for a folder of Markdown files.
//!
//! Every Markdown file under the folder is listed once, grouped by subfolder, with its title,
//! description, size and keywords. Duplicates are merged:
//! - **Duplicate files**: documents with (nearly) the same text, found with MinHash
//!   fingerprints of overlapping five-word runs, so a source and its cleaned copy still match.
//! - **Repeated passages**: paragraphs that appear in several documents are listed once.
//! - **Key terms**: terms shared across documents are merged into one list.
//!
//! The plan also names Doc Monster's own duplicate outputs that can be deleted. Files Doc Monster
//! did not write are never on that list.

use crate::digest::{self, nlp};
use crate::profile::group;
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Front matter comment of every index file.
pub const MARKER: &str = "# Indexed with Doc Monster";

const SIGNATURE: usize = 64;
const BANDS: usize = 16;
const SHINGLE: usize = 5;
/// Estimated share of five-word runs two files must have in common to count as one document.
const SAME_DOCUMENT: f64 = 0.85;
const PASSAGE_MIN_WORDS: usize = 12;
/// Files shorter than this (in words) are only duplicates when their text is identical.
const SHORT: usize = 50;
const MAX_PASSAGES: usize = 25;
const MAX_TERMS: usize = 16;

pub struct Plan {
    /// Where the index should be written.
    pub path: PathBuf,
    pub markdown: String,
    /// Doc Monster's own duplicate outputs (never user files).
    pub duplicates_to_delete: Vec<PathBuf>,
    pub files: usize,
    pub unique: usize,
}

struct Doc {
    path: PathBuf,
    rel: String,
    title: String,
    description: String,
    /// Written by Doc Monster (front matter says Condensed / Converted).
    ours: bool,
    words: usize,
    /// Hash of the whole normalized text, for exact matches of short files.
    exact: u64,
    signature: [u64; SIGNATURE],
    /// (hash of normalized text, original text) for paragraphs long enough to matter.
    passages: Vec<(u64, String)>,
    tokens: nlp::Counts,
    phrases: nlp::Counts,
}

/// FNV-1a: small, fast and the same on every machine and every run.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn signature(words: &[String]) -> [u64; SIGNATURE] {
    let mut sig = [u64::MAX; SIGNATURE];
    let mut add = |shingle: u64| {
        for (i, slot) in sig.iter_mut().enumerate() {
            let v = mix(shingle ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            if v < *slot {
                *slot = v;
            }
        }
    };
    if words.len() < SHINGLE {
        if !words.is_empty() {
            add(fnv(words.join(" ").as_bytes()));
        }
    } else {
        for w in words.windows(SHINGLE) {
            add(fnv(w.join(" ").as_bytes()));
        }
    }
    sig
}

fn similarity(a: &[u64; SIGNATURE], b: &[u64; SIGNATURE]) -> f64 {
    a.iter()
        .zip(b)
        .filter(|(x, y)| x == y && **x != u64::MAX)
        .count() as f64
        / SIGNATURE as f64
}

/// What kind of Markdown file this is, from its front matter comment.
pub fn front_comment(text: &str) -> Option<&str> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    rest.lines()
        .next()
        .filter(|l| l.starts_with("# "))
        .map(str::trim)
}

pub fn is_index_file(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 64];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    front_comment(&String::from_utf8_lossy(&head[..n])) == Some(MARKER)
}

fn parse(root: &Path, path: &Path) -> Result<Doc> {
    let text = String::from_utf8_lossy(
        &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .replace("\r\n", "\n");
    let comment = front_comment(&text).map(str::to_string);
    let ours = matches!(
        comment.as_deref(),
        Some("# Condensed with Doc Monster" | "# Converted with Doc Monster")
    );
    let (fm, body) = crate::extract::split_front_matter(&text);
    let field = |key: &str| -> Option<String> {
        fm?.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim().eq_ignore_ascii_case(key))
                .then(|| v.trim().trim_matches('"').replace("\\\"", "\""))
                .filter(|v| !v.is_empty())
        })
    };
    // For our own outputs compare the document itself, not the summary written on top of it.
    let content = if ours {
        match body.split_once("\n## Full content\n") {
            Some((_, full)) => full,
            None => body,
        }
    } else {
        body
    };
    let content: String = content
        .lines()
        .filter(|l| !l.starts_with("> **Not included:**"))
        .collect::<Vec<_>>()
        .join("\n");
    // Compare everything in cleaned form, so an ugly original matches its cleaned copy.
    let (content, _) = crate::clean::clean(&content);

    let words = words_of(&content);
    let mut passages = Vec::new();
    let mut in_fence = false;
    for block in content.split("\n\n") {
        let t = block.trim();
        let fences = t
            .lines()
            .filter(|l| l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~"))
            .count();
        if in_fence || fences > 0 {
            if fences % 2 == 1 {
                in_fence = !in_fence;
            }
            continue;
        }
        if t.starts_with(['#', '|', '>', '<']) {
            continue;
        }
        let plain = t.trim_start_matches(['-', '*', ' ']);
        let w = words_of(plain);
        if w.len() >= PASSAGE_MIN_WORDS {
            passages.push((
                fnv(w.join(" ").as_bytes()),
                plain.split_whitespace().collect::<Vec<_>>().join(" "),
            ));
        }
    }
    let mut phrases = nlp::Counts::new();
    for p in nlp::bigrams(&content) {
        *phrases.entry(p).or_default() += 1;
    }
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document")
        .to_string();
    Ok(Doc {
        title: field("title")
            .or_else(|| crate::extract::first_heading(&content))
            .unwrap_or(stem),
        description: field("description")
            .or_else(|| digest::first_sentence(&content))
            .map(|d| crate::render::tidy_description(&d))
            .unwrap_or_default(),
        ours,
        words: words.len(),
        exact: fnv(words.join(" ").as_bytes()),
        signature: signature(&words),
        passages,
        tokens: nlp::count(&nlp::tokens(&content)),
        phrases,
        path: path.to_path_buf(),
        rel,
    })
}

fn markdown_files(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            // Skip hidden folders and build output, but never the root itself.
            e.depth() == 0
                || !(e.file_type().is_dir()
                    && (e.file_name().to_string_lossy().starts_with('.')
                        || matches!(e.file_name().to_str(), Some("target" | "node_modules"))))
        })
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            matches!(
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("md" | "markdown")
            )
        })
        .filter(|p| !is_index_file(p))
        .collect()
}

struct Groups(Vec<usize>);
impl Groups {
    fn find(&mut self, i: usize) -> usize {
        let mut r = i;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut i = i;
        while self.0[i] != r {
            let next = self.0[i];
            self.0[i] = r;
            i = next;
        }
        r
    }
    fn join(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra.max(rb)] = ra.min(rb);
        }
    }
}

/// Plain link to a file, safe for names with spaces.
fn link(text: &str, rel: &str) -> String {
    format!("[{}](<{rel}>)", text.replace(['[', ']'], ""))
}

/// Folder and base name shared by report.md, report.v2.md, report.v3.md: one source's outputs.
fn family(p: &Path) -> (PathBuf, String) {
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let base = match stem.rsplit_once(".v") {
        Some((b, n)) if n.parse::<usize>().is_ok() => b,
        _ => stem,
    };
    (
        p.parent().map(Path::to_path_buf).unwrap_or_default(),
        base.to_string(),
    )
}

/// Number in "report.v3.md" (1 for "report.md"), so the earliest copy is kept.
fn version_of(p: &Path) -> usize {
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    stem.rsplit_once(".v")
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(1)
}

/// Where the index goes: INDEX.md at the top, or INDEX.v2.md if a file Doc Monster did not
/// write already has that name.
fn index_path(root: &Path) -> PathBuf {
    for n in 1.. {
        let p = if n == 1 {
            root.join("INDEX.md")
        } else {
            root.join(format!("INDEX.v{n}.md"))
        };
        if !p.exists() || is_index_file(&p) {
            return p;
        }
    }
    unreachable!()
}

pub fn plan(root: &Path) -> Result<Plan> {
    anyhow::ensure!(root.is_dir(), "{} is not a folder", root.display());
    let files = markdown_files(root);
    let docs: Vec<Doc> = files
        .par_iter()
        .filter_map(|p| parse(root, p).ok())
        .collect();
    let n = docs.len();

    // 1. Duplicate files: candidate pairs share a band of their fingerprint, then are checked.
    let mut groups = Groups((0..n).collect());
    let rows = SIGNATURE / BANDS;
    for band in 0..BANDS {
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, d) in docs.iter().enumerate() {
            if d.words == 0 {
                continue;
            }
            let key = d.signature[band * rows..(band + 1) * rows]
                .iter()
                .fold(band as u64, |h, v| mix(h ^ v));
            buckets.entry(key).or_default().push(i);
        }
        for members in buckets.values().filter(|m| m.len() > 1) {
            for &j in &members[1..] {
                let (a, b) = (&docs[members[0]], &docs[j]);
                // Short files carry too little text for a reliable estimate: they must match exactly.
                let same = if a.words.min(b.words) < SHORT {
                    a.exact == b.exact
                } else {
                    similarity(&a.signature, &b.signature) >= SAME_DOCUMENT
                };
                if same {
                    groups.join(members[0], j);
                }
            }
        }
    }
    let mut by_group: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        by_group.entry(groups.find(i)).or_default().push(i);
    }
    // Each group shows one entry: a Doc Monster output if there is one (it carries the summary),
    // the earliest version first.
    let mut entries: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut delete: Vec<PathBuf> = Vec::new();
    for members in by_group.into_values() {
        let mut members = members;
        members.sort_by_key(|&i| {
            (
                !docs[i].ours,
                version_of(&docs[i].path),
                docs[i].rel.clone(),
            )
        });
        let keep = members[0];
        let others: Vec<usize> = members[1..].to_vec();
        // Delete only reruns: a later copy (name.vN.md) of a kept Doc Monster output of the same
        // source. Outputs of different sources that happen to match are merged but kept on disk.
        let mut kept_families: HashSet<(PathBuf, String)> = HashSet::new();
        for &i in &members {
            if !docs[i].ours {
                continue;
            }
            let fam = family(&docs[i].path);
            if !kept_families.insert(fam) {
                delete.push(docs[i].path.clone());
            }
        }
        entries.push((keep, others));
    }
    entries.sort_by(|a, b| docs[a.0].rel.cmp(&docs[b.0].rel));
    let unique = entries.len();
    let deleted: HashSet<&PathBuf> = delete.iter().collect();

    // 2. Collection-wide word statistics over unique documents only.
    let mut df: HashMap<&str, usize> = HashMap::new();
    let mut phrase_df: HashMap<&str, (usize, u32)> = HashMap::new();
    for (k, _) in &entries {
        for t in docs[*k].tokens.keys() {
            *df.entry(t).or_default() += 1;
        }
        for (p, c) in &docs[*k].phrases {
            let e = phrase_df.entry(p).or_default();
            e.0 += 1;
            e.1 += c;
        }
    }
    let idf = |t: &str| (1.0 + unique as f64 / *df.get(t).unwrap_or(&1) as f64).ln();
    let keywords = |d: &Doc| -> Vec<String> {
        let mut v: Vec<(&String, f64)> = d
            .tokens
            .iter()
            .filter(|(_, c)| **c >= 2)
            .map(|(t, c)| (t, f64::from(*c) * idf(t)))
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        v.into_iter().take(4).map(|(t, _)| t.clone()).collect()
    };

    // 3. Merged key terms: phrases found in several documents, then common single words.
    let mut terms: Vec<(&str, usize, u32)> = phrase_df
        .iter()
        .filter(|(_, (d, _))| *d >= 2 || unique == 1)
        .map(|(p, (d, c))| (*p, *d, *c))
        .collect();
    terms.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(b.0)));
    let mut shown_terms: Vec<String> = terms
        .iter()
        .take(MAX_TERMS)
        .map(|(p, d, _)| format!("`{p}` ({d})"))
        .collect();
    if shown_terms.len() < MAX_TERMS {
        let mut singles: Vec<(&&str, &usize)> = df.iter().filter(|(_, d)| **d >= 2).collect();
        singles.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        shown_terms.extend(
            singles
                .into_iter()
                .take(MAX_TERMS - shown_terms.len())
                .map(|(t, d)| format!("`{t}` ({d})")),
        );
    }

    // 4. Repeated passages across different documents.
    let entry_of: HashMap<usize, usize> = entries
        .iter()
        .enumerate()
        .flat_map(|(e, (k, o))| {
            std::iter::once(*k)
                .chain(o.iter().copied())
                .map(move |i| (i, e))
        })
        .collect();
    let mut passages: HashMap<u64, (String, HashSet<usize>)> = HashMap::new();
    for (i, d) in docs.iter().enumerate() {
        for (h, text) in &d.passages {
            let p = passages
                .entry(*h)
                .or_insert_with(|| (text.clone(), HashSet::new()));
            p.1.insert(entry_of[&i]);
        }
    }
    let mut shared: Vec<(String, Vec<usize>)> = passages
        .into_values()
        .filter(|(_, e)| e.len() >= 2)
        .map(|(t, e)| {
            let mut e: Vec<usize> = e.into_iter().collect();
            e.sort_unstable();
            (t, e)
        })
        .collect();
    shared.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then(b.0.len().cmp(&a.0.len()))
            .then(a.0.cmp(&b.0))
    });

    // Render.
    let root_name = root
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "folder".into());
    let total_words: usize = entries.iter().map(|(k, _)| docs[*k].words).sum();
    let duplicates = n - unique;
    let top: Vec<&str> = terms.iter().take(3).map(|t| t.0).collect();
    let description = if top.is_empty() {
        format!("{unique} documents.")
    } else {
        format!(
            "{unique} documents; most common topics: {}.",
            top.join(", ")
        )
    };

    let mut md = format!(
        "---\n{MARKER}\ntitle: \"Index of {}\"\ndescription: \"{}\"\n---\n\n",
        root_name.replace('"', "'"),
        crate::render::tidy_description(&description).replace('"', "'")
    );
    md.push_str("## At a glance\n\n");
    md.push_str(&format!(
        "- {} Markdown files: {} unique documents",
        group(&n.to_string()),
        group(&unique.to_string())
    ));
    if duplicates > 0 {
        md.push_str(&format!(
            ", {} duplicates merged",
            group(&duplicates.to_string())
        ));
        if !delete.is_empty() {
            md.push_str(&format!(
                " ({} duplicate Doc Monster outputs removed)",
                group(&delete.len().to_string())
            ));
        }
    }
    md.push_str(".\n");
    md.push_str(&format!(
        "- {} words (about {} min read).\n",
        group(&total_words.to_string()),
        group(&total_words.div_ceil(220).to_string())
    ));
    if !shared.is_empty() {
        md.push_str(&format!(
            "- {} passages appear in more than one document.\n",
            group(&shared.len().to_string())
        ));
    }
    if !shown_terms.is_empty() {
        md.push_str("\n## Key terms across the collection\n\n");
        md.push_str(&shown_terms.join(" · "));
        md.push_str("\n\n_The number is how many documents use the term._\n");
    }

    md.push_str("\n## Documents\n");
    let mut by_folder: BTreeMap<String, Vec<&(usize, Vec<usize>)>> = BTreeMap::new();
    for e in &entries {
        let folder = Path::new(&docs[e.0].rel)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        by_folder.entry(folder).or_default().push(e);
    }
    for (folder, list) in by_folder {
        md.push_str(&format!(
            "\n### {}\n\n",
            if folder.is_empty() {
                "Top level".to_string()
            } else {
                format!("{folder}/")
            }
        ));
        for (k, others) in list {
            let d = &docs[*k];
            let kws = keywords(d);
            let mut line = format!("- **{}**", link(&d.title, &d.rel));
            if !d.description.is_empty() {
                line.push_str(&format!(" — {}", d.description));
            }
            line.push_str(&format!(" · {} words", group(&d.words.to_string())));
            if !kws.is_empty() {
                line.push_str(&format!(" · _{}_", kws.join(", ")));
            }
            md.push_str(&line);
            md.push('\n');
            let kept: Vec<String> = others
                .iter()
                .filter(|&&i| !deleted.contains(&docs[i].path))
                .map(|&i| link(&docs[i].rel, &docs[i].rel))
                .collect();
            let removed: Vec<String> = others
                .iter()
                .filter(|&&i| deleted.contains(&docs[i].path))
                .map(|&i| format!("`{}`", docs[i].rel))
                .collect();
            if !kept.is_empty() {
                md.push_str(&format!("  - Same content: {}\n", kept.join(", ")));
            }
            if !removed.is_empty() {
                md.push_str(&format!(
                    "  - Duplicate copies removed: {}\n",
                    removed.join(", ")
                ));
            }
        }
    }

    if !shared.is_empty() {
        md.push_str("\n## Shared passages\n\nText that appears word for word in more than one document, listed once.\n\n");
        for (text, where_) in shared.iter().take(MAX_PASSAGES) {
            let clipped = if text.chars().count() > 220 {
                format!("{}…", text.chars().take(217).collect::<String>().trim_end())
            } else {
                text.clone()
            };
            let places: Vec<String> = where_
                .iter()
                .map(|&e| link(&docs[entries[e].0].title, &docs[entries[e].0].rel))
                .collect();
            md.push_str(&format!("- \"{clipped}\" — in {}\n", places.join(", ")));
        }
        if shared.len() > MAX_PASSAGES {
            md.push_str(&format!("- … and {} more.\n", shared.len() - MAX_PASSAGES));
        }
    }

    Ok(Plan {
        path: index_path(root),
        markdown: md,
        duplicates_to_delete: delete,
        files: n,
        unique,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours(title: &str, body: &str) -> String {
        format!(
            "---\n# Condensed with Doc Monster\ntitle: \"{title}\"\ndescription: \"About {title}.\"\n---\n\n## Summary\n\n- a summary line\n\n---\n\n## Full content\n\n{body}"
        )
    }

    const SAVING: &str = "Player data is saved to a DataStore every minute and again when the player leaves the game server.\n\nSession locking prevents two servers from writing the same profile and corrupting inventory for everyone.\n";
    const MONEY: &str = "Game passes unlock permanent perks while developer products sell repeatable boosts to players.\n\nSession locking prevents two servers from writing the same profile and corrupting inventory for everyone.\n";

    #[test]
    fn duplicates_are_merged_and_only_our_extra_copies_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("sub")).unwrap();
        // The user's original, our cleaned copy of it (rewrapped), and a rerun copy.
        std::fs::write(
            d.join("saving.md"),
            SAVING.replace("every minute and", "every minute\nand"),
        )
        .unwrap();
        std::fs::write(d.join("saving.v2.md"), ours("Saving", SAVING)).unwrap();
        std::fs::write(d.join("saving.v3.md"), ours("Saving", SAVING)).unwrap();
        std::fs::write(d.join("sub/money.md"), ours("Money", MONEY)).unwrap();
        let plan = plan(d).unwrap();
        assert_eq!(plan.files, 4);
        assert_eq!(plan.unique, 2, "{}", plan.markdown);
        assert_eq!(plan.duplicates_to_delete, vec![d.join("saving.v3.md")]);
        let md = &plan.markdown;
        assert!(
            md.starts_with("---\n# Indexed with Doc Monster\ntitle: \"Index of"),
            "{md}"
        );
        assert!(
            md.contains("[Saving](<saving.v2.md>)")
                && md.contains("Same content: [saving.md](<saving.md>)"),
            "{md}"
        );
        assert!(
            md.contains("Duplicate copies removed: `saving.v3.md`"),
            "{md}"
        );
        assert!(
            md.contains("### sub/") && md.contains("[Money](<sub/money.md>)"),
            "{md}"
        );
        // The shared paragraph is listed once, with both documents.
        assert!(
            md.contains("## Shared passages")
                && md.contains("Session locking prevents")
                && md.contains("— in [Saving](<saving.v2.md>), [Money](<sub/money.md>)"),
            "{md}"
        );
        assert!(md.contains("`session locking` (2)"), "{md}");
        assert_eq!(plan.path, d.join("INDEX.md"));
    }

    #[test]
    fn matching_outputs_of_different_sources_are_merged_but_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), ours("A", SAVING)).unwrap();
        std::fs::write(dir.path().join("b.md"), ours("B", SAVING)).unwrap();
        std::fs::write(
            dir.path().join("tiny1.md"),
            ours("T1", "Only an image here."),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("tiny2.md"),
            ours("T2", "A different picture."),
        )
        .unwrap();
        let p = plan(dir.path()).unwrap();
        assert!(
            p.duplicates_to_delete.is_empty(),
            "{:?}",
            p.duplicates_to_delete
        );
        assert_eq!(p.unique, 3, "a and b merge; the two short files differ");
        assert!(
            p.markdown.contains("Same content: [b.md](<b.md>)"),
            "{}",
            p.markdown
        );
    }

    #[test]
    fn a_users_own_index_file_is_never_overwritten_and_ours_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("INDEX.md"), "# My own index\n").unwrap();
        std::fs::write(dir.path().join("a.md"), SAVING).unwrap();
        assert_eq!(
            plan(dir.path()).unwrap().path,
            dir.path().join("INDEX.v2.md")
        );
        std::fs::write(
            dir.path().join("INDEX.v2.md"),
            "---\n# Indexed with Doc Monster\n---\n",
        )
        .unwrap();
        let p = plan(dir.path()).unwrap();
        assert_eq!(p.path, dir.path().join("INDEX.v2.md"));
        assert_eq!(
            p.files, 2,
            "our own index is not indexed, the user's INDEX.md is"
        );
    }
}
