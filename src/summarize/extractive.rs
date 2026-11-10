use super::Summarizer;
use anyhow::Result;
use std::collections::HashMap;

/// Offline fallback that needs no model: picks the highest-scoring sentences
/// by word frequency and returns them in their original order.
pub struct Extractive {
    pub sentences: usize,
}

impl Summarizer for Extractive {
    fn name(&self) -> String {
        "extractive (no model)".into()
    }

    fn summarize(&self, text: &str) -> Result<String> {
        let sentences = split_sentences(text);
        if sentences.len() <= self.sentences {
            return Ok(sentences.join(" "));
        }
        let mut freq: HashMap<String, f64> = HashMap::new();
        for w in sentences.iter().flat_map(|s| words(s)) {
            *freq.entry(w).or_default() += 1.0;
        }
        let mut scored: Vec<(usize, f64)> = sentences
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let ws = words(s);
                let score = ws.iter().map(|w| freq[w]).sum::<f64>() / (ws.len().max(1) as f64).sqrt();
                (i, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut keep: Vec<usize> = scored.into_iter().take(self.sentences).map(|(i, _)| i).collect();
        keep.sort_unstable();
        Ok(keep.iter().map(|&i| sentences[i].as_str()).collect::<Vec<_>>().join(" "))
    }
}

fn words(s: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "is", "are", "was", "were",
        "it", "that", "this", "with", "as", "by", "be", "at", "from",
    ];
    s.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() > 2 && !STOP.contains(&w.as_str()))
        .collect()
}

fn split_sentences(text: &str) -> Vec<String> {
    // Drop markup-only lines (headings markers, table rules, fences) before splitting.
    let flat: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("```") && !l.trim_start().starts_with("| ---"))
        .map(|l| l.trim().trim_start_matches(['#', '-', '*', '>', ' ']))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in flat.chars() {
        cur.push(c);
        if matches!(c, '.' | '!' | '?') {
            let s = cur.trim().to_string();
            if s.len() > 1 {
                out.push(s);
            }
            cur.clear();
        }
    }
    let rest = cur.trim();
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_requested_number_in_original_order() {
        let text = "Rust is fast. Rust is safe. Cats sleep. Rust has a borrow checker. Rust compiles.";
        let out = Extractive { sentences: 2 }.summarize(text).unwrap();
        assert_eq!(out.matches('.').count(), 2);
        assert!(!out.contains("Cats"));
    }
}
