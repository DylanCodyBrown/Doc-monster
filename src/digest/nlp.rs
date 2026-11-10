//! Small, dependency-free language helpers: tokens, sentences, similarity.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub type Counts = HashMap<String, u32>;

const STOP: &str = "a about above after again against all also am an and any are as at be because been before \
being below between both but by can could did do does doing down during each few for from further had has have \
having he her here hers him his how i if in into is it its itself just may me more most must my no nor not now \
of off on once only or other our out over own same shall she should so some such than that the their them then \
there these they this those through to too under until up upon us very was we were what when where which while \
who whom why will with would you your yours also etc see per via within without across among however thus \
therefore furthermore note always never every get got one two three new many much like even still using use used make made";

fn stop_words() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| STOP.split_whitespace().collect())
}

/// Light plural stemming so "servers" and "server" count together.
fn stem(w: &str) -> String {
    if w.len() > 4 && w.ends_with("ies") {
        format!("{}y", &w[..w.len() - 3])
    } else if w.len() > 4 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") {
        w[..w.len() - 1].to_string()
    } else {
        w.to_string()
    }
}

/// Meaningful lowercase words: no stop words, no bare numbers, at least 3 letters.
pub fn tokens(text: &str) -> Vec<String> {
    tokens_surface(text).into_iter().map(|(stem, _)| stem).collect()
}

/// Like `tokens`, but also returns the word as written (lowercased) for display.
pub fn tokens_surface(text: &str) -> Vec<(String, String)> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| w.chars().count() >= 3)
        .map(str::to_lowercase)
        .filter(|w| !stop_words().contains(w.as_str()) && w.chars().any(char::is_alphabetic))
        .map(|w| (stem(&w), w))
        .collect()
}

pub fn count(tokens: &[String]) -> Counts {
    let mut c = Counts::new();
    for t in tokens {
        *c.entry(t.clone()).or_default() += 1;
    }
    c
}

pub fn merge(into: &mut Counts, other: &Counts) {
    for (k, v) in other {
        *into.entry(k.clone()).or_default() += v;
    }
}

pub fn cosine(a: &Counts, b: &Counts) -> f32 {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let dot: f64 = small
        .iter()
        .filter_map(|(k, v)| large.get(k).map(|w| f64::from(*v) * f64::from(*w)))
        .sum();
    let na: f64 = a.values().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.values().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { (dot / (na * nb)) as f32 }
}

pub fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    inter / ((a.len() + b.len()) as f32 - inter)
}

const ABBREV: &[&str] = &["e.g", "i.e", "vs", "etc", "mr", "mrs", "dr", "fig", "no", "approx", "inc", "st"];

/// Split prose into sentences without breaking on decimals or common abbreviations.
pub fn sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], '.' | '!' | '?') {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], '.' | '!' | '?' | '"' | '\'' | ')' | '”' | '’') {
                j += 1;
            }
            let at_end = j >= chars.len();
            let spaced = !at_end && chars[j].is_whitespace();
            if at_end || spaced {
                let mut k = j;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
                let next_ok = k >= chars.len()
                    || chars[k].is_uppercase()
                    || chars[k].is_ascii_digit()
                    || matches!(chars[k], '"' | '“' | '(' | '[' | '`' | '*');
                let word: String = chars[start..i]
                    .iter()
                    .rev()
                    .take_while(|c| !c.is_whitespace())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>()
                    .to_lowercase();
                let abbrev = chars[i] == '.' && (ABBREV.contains(&word.as_str()) || (word.chars().count() == 1 && word.chars().all(char::is_alphabetic)));
                if next_ok && !abbrev {
                    let s: String = chars[start..j].iter().collect();
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(s.to_string());
                    }
                    start = k;
                    i = k;
                    continue;
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    let rest: String = chars[start.min(chars.len())..].iter().collect();
    if !rest.trim().is_empty() {
        out.push(rest.trim().to_string());
    }
    out
}

/// Remove emphasis markers and link syntax so text reads cleanly in a summary.
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '*' | '_' if i + 1 < b.len() && b[i + 1] == b[i] => i += 2,
            '*' => i += 1,
            '[' => {
                // [text](url) -> text
                if let Some(close) = b[i..].iter().position(|&c| c == ']') {
                    let after = i + close + 1;
                    if after < b.len() && b[after] == '(' {
                        if let Some(p) = b[after..].iter().position(|&c| c == ')') {
                            out.extend(&b[i + 1..i + close]);
                            i = after + p + 1;
                            continue;
                        }
                    }
                }
                out.push('[');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sentences_but_not_decimals_or_abbreviations() {
        let s = sentences("Version 3.5 shipped. See e.g. the guide. It works! Really? Yes.");
        assert_eq!(s, vec!["Version 3.5 shipped.", "See e.g. the guide.", "It works!", "Really?", "Yes."]);
    }

    #[test]
    fn tokens_drop_stop_words_and_stem_plurals() {
        assert_eq!(tokens("The servers are saving 100 entries"), vec!["server", "saving", "entry"]);
        assert_eq!(sentences("One in 3. Next one."), vec!["One in 3.", "Next one."]);
    }

    #[test]
    fn jaccard_basic() {
        let a: HashSet<String> = ["x", "y"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["y", "z"].iter().map(|s| s.to_string()).collect();
        assert!((jaccard(&a, &b) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn plain_strips_markup() {
        assert_eq!(plain("**Bold** and [a link](http://x.y) here"), "Bold and a link here");
    }
}

/// Adjacent pairs of meaningful words ("session locking"), used to find key phrases.
pub fn bigrams(text: &str) -> Vec<String> {
    let words: Vec<Option<String>> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(|w| tokens(w).into_iter().next())
        .collect();
    words
        .windows(2)
        .filter_map(|p| match (&p[0], &p[1]) {
            (Some(a), Some(b)) if a != b => Some(format!("{a} {b}")),
            _ => None,
        })
        .collect()
}
