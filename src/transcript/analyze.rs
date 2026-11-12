//! Summarising a transcript: topics with times, decisions, action items, questions left open,
//! and who said what. Everything here is rules over the words people said; an optional
//! `Rewriter` (a local language model) can also write each topic's summary as prose.

use super::{Transcript, Utterance};
use crate::digest::nlp;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// Writes a short summary of a piece of conversation (implemented by the optional model).
pub trait Rewriter {
    /// The model's answer to `prompt` (a request plus the text it is about).
    fn complete(&self, prompt: &str) -> anyhow::Result<String>;
    /// Shown beside model-written text, e.g. "Qwen2.5 1.5B Instruct".
    fn name(&self) -> String;
    /// Longest piece of conversation, in words, given to the model in one go.
    fn max_words(&self) -> usize {
        1200
    }
}

pub struct Summary {
    /// The "## Summary" content (without the heading).
    pub markdown: String,
    /// One sentence for the front matter.
    pub description: String,
    /// The whole conversation, grouped by topic.
    pub body: String,
    /// Model problems that were worked around (for the "Not included" line).
    pub notes: Vec<String>,
}

/// A turn after merging: one speaker's consecutive lines.
struct Turn {
    at: Option<u32>,
    speaker: String,
    text: String,
}

struct Sentence {
    turn: usize,
    text: String,
}

struct Topic {
    /// Turns first..last (inclusive).
    first: usize,
    last: usize,
    label: String,
    keywords: Vec<String>,
}

pub fn stamp(secs: u32) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn duration(secs: u32) -> String {
    if secs < 60 {
        return format!("{secs} s");
    }
    let (h, m) = (secs / 3600, (secs % 3600 + 30) / 60);
    match (h, m) {
        (0, 0) => format!("{secs} s"),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn plural(n: usize, w: &str) -> String {
    format!(
        "{} {w}{}",
        crate::profile::group(&n.to_string()),
        if n == 1 { "" } else { "s" }
    )
}

/// Spoken filler, removed before analysis (the full transcript keeps every word).
fn tidy(text: &str) -> String {
    static FILLER: OnceLock<Regex> = OnceLock::new();
    let f = FILLER.get_or_init(|| {
        Regex::new(r"(?i)\b(um+|uh+|erm+|hmm+|mm+|ah+|you know|i mean|sort of|kind of)\b[,.]?\s*")
            .unwrap()
    });
    let t = f.replace_all(text, "");
    // A stutter ("the the") keeps one copy.
    let mut kept: Vec<&str> = Vec::new();
    for w in t.split_whitespace() {
        if kept.last().is_some_and(|p| p.eq_ignore_ascii_case(w)) {
            continue;
        }
        kept.push(w);
    }
    let t = kept.join(" ");
    // A sentence that now starts in lower case after its filler was removed.
    let mut c = t.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => t,
    }
}

/// Conversational words that say nothing about the topic.
fn chatter() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        "okay yeah yes yep yup think know really going gonna wanna want right thing things kind \
         sort actually probably maybe good great sure well mean said say let lets need people time \
         lot little bit guy guys today stuff just could would should look looks looked around \
         pretty basically literally anyway alright cool fine thanks thank sorry hey hello question \
         talk talked talking tell told saying see seen come came give take took put done doing \
         first last next week day back part point way something anything everything someone \
         one two three four five six seven eight nine ten eleven twelve twenty thirty forty fifty \
         hundred thousand second seconds minute minutes hour hours morning"
            .split_whitespace()
            .collect()
    })
}

fn words(text: &str) -> Vec<String> {
    nlp::tokens(text)
        .into_iter()
        .filter(|w| !chatter().contains(w.as_str()))
        .collect()
}

fn merge_turns(utterances: &[Utterance]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for u in utterances {
        let speaker = if u.speaker.is_empty() {
            "Unknown speaker".to_string()
        } else {
            u.speaker.clone()
        };
        // The same speaker continuing: one turn, unless they paused or opened a new topic.
        let continues = |t: &Turn| {
            let paused = match (t.at, u.at) {
                (Some(a), Some(b)) => {
                    let spoken = t.text.split_whitespace().count() as u32 * 2 / 5;
                    b.saturating_sub(a) > spoken + 10
                }
                _ => false,
            };
            t.speaker == speaker && !paused && !starts_topic(&u.text)
        };
        match turns.last_mut() {
            Some(t) if continues(t) => {
                t.text.push(' ');
                t.text.push_str(u.text.trim());
            }
            _ => turns.push(Turn {
                at: u.at,
                speaker,
                text: u.text.trim().to_string(),
            }),
        }
    }
    turns
}

/// Split the conversation where the vocabulary changes (TextTiling): blocks of about 80 words,
/// compared two at a time on each side of every gap; the deepest valleys become boundaries.
fn segment(turns: &[Turn]) -> Vec<(usize, usize)> {
    if turns.is_empty() {
        return Vec::new();
    }
    let mut blocks: Vec<(usize, usize, nlp::Counts)> = Vec::new();
    let (mut start, mut n, mut acc) = (0usize, 0usize, Vec::new());
    for (i, t) in turns.iter().enumerate() {
        let w = words(&t.text);
        n += t.text.split_whitespace().count();
        acc.extend(w);
        if n >= 80 || i + 1 == turns.len() {
            blocks.push((start, i, nlp::count(&acc)));
            start = i + 1;
            n = 0;
            acc.clear();
        }
    }
    let total: usize = turns
        .iter()
        .map(|t| t.text.split_whitespace().count())
        .sum();
    // About one topic per 220 words said (a few minutes of talk), at most 15.
    let wanted = ((total + 100) / 220).clamp(1, 15);
    if (blocks.len() < 4 || wanted == 1) && !turns.iter().skip(1).any(|t| starts_topic(&t.text)) {
        return vec![(0, turns.len() - 1)];
    }
    let window = |from: usize, to: usize| {
        let mut c = nlp::Counts::new();
        for b in &blocks[from..=to] {
            nlp::merge(&mut c, &b.2);
        }
        c
    };
    // Similarity across each gap (gap g lies between block g and g + 1).
    let sims: Vec<f32> = (0..blocks.len() - 1)
        .map(|g| {
            let left = window(g.saturating_sub(1), g);
            let right = window(g + 1, (g + 2).min(blocks.len() - 1));
            nlp::cosine(&left, &right)
        })
        .collect();
    let depth = |g: usize| {
        let left = sims[..=g].iter().rev().take_while_peak();
        let right = sims[g..].iter().take_while_peak();
        (left - sims[g]) + (right - sims[g])
    };
    // Topics people announced ("next topic, ...") count first; the vocabulary fills the rest.
    let announced: Vec<usize> = (1..turns.len())
        .filter(|&i| starts_topic(&turns[i].text))
        .collect();
    let extra = wanted.saturating_sub(1 + announced.len());
    let mut gaps: Vec<(f32, usize)> = (0..sims.len()).map(|g| (depth(g), g)).collect();
    gaps.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut chosen: Vec<usize> = Vec::new();
    for (d, g) in gaps {
        if chosen.len() >= extra || d <= 0.05 {
            break;
        }
        let cut = blocks[g].1 + 1;
        if chosen.iter().all(|&c| c.abs_diff(g) >= 2)
            && announced.iter().all(|&a| a.abs_diff(cut) >= 4)
        {
            chosen.push(g);
        }
    }
    // Each cut moves to the nearest natural break: someone announcing a new topic, or a pause.
    let mut starts: Vec<usize> = chosen
        .iter()
        .map(|&g| {
            let cut = blocks[g].1 + 1;
            let lo = cut.saturating_sub(4).max(1);
            let hi = (cut + 4).min(turns.len() - 1);
            (lo..=hi)
                .max_by(|&a, &b| {
                    let score =
                        |i: usize| break_strength(turns, i) - (i.abs_diff(cut) as f32) * 0.05;
                    score(a).total_cmp(&score(b))
                })
                .unwrap_or(cut)
        })
        .collect();
    // An announced topic ("next topic, ...") always starts a new one.
    for (i, t) in turns.iter().enumerate().skip(1) {
        if starts_topic(&t.text) && starts.iter().all(|&s| s.abs_diff(i) >= 3) {
            starts.push(i);
        }
    }
    starts.sort_unstable();
    starts.dedup();
    let mut out = Vec::new();
    let mut first = 0;
    for s in starts {
        if s > first {
            out.push((first, s - 1));
            first = s;
        }
    }
    out.push((first, turns.len() - 1));
    // A "topic" of one short turn (an agenda line, a greeting) joins the topic after it.
    let words = |r: &(usize, usize)| -> usize {
        turns[r.0..=r.1]
            .iter()
            .map(|t| t.text.split_whitespace().count())
            .sum()
    };
    let mut merged: Vec<(usize, usize)> = Vec::new();
    let mut carry: Option<usize> = None;
    for r in out {
        let r = (carry.take().unwrap_or(r.0), r.1);
        if r.1 == r.0 && words(&r) < 60 {
            carry = Some(r.0);
        } else {
            merged.push(r);
        }
    }
    if let Some(c) = carry {
        match merged.last_mut() {
            Some(last) => last.1 = turns.len() - 1,
            None => merged.push((c, turns.len() - 1)),
        }
    }
    merged
}

fn topic_cue() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(r"(?i)^(?:okay,?\s+|ok,?\s+|so,?\s+|right,?\s+|alright,?\s+)?(?:(?:next|last|final|another|the next|our next|new) (?:topic|thing|item|point|question)|moving on|on to|switching to|let'?s (?:move on|talk about|turn to)|(?:the )?(?:first|second|third) (?:topic|thing|item))\b[,:]?\s*(?:is\s+|to\s+)?(?:the\s+)?(?P<what>[A-Za-z][\w\s'-]{2,40}?)?(?:[.?!,]|$)")
    })
}

fn first_cue() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // "Okay, the outage first." / "First, the budget."
    R.get_or_init(|| {
        re(r"(?i)^(?:okay,?\s+|ok,?\s+|so,?\s+|right,?\s+|alright,?\s+)?(?:the\s+)?(?P<what>[a-z][\w\s'-]{2,30}?) first[.,!]|^(?:okay,?\s+|so,?\s+)?first(?: up)?,\s+(?:the\s+)?(?P<what2>[a-z][\w\s'-]{2,30}?)[.?!]")
    })
}

/// True when the turn opens a new topic ("Next topic, the rare pet balance.").
fn starts_topic(text: &str) -> bool {
    let t = text.trim();
    topic_cue().is_match(t) || first_cue().is_match(t)
}

/// What an announcing turn names as its topic ("the rare pet balance").
fn announced_topic(text: &str) -> Option<String> {
    let first = text.split(['.', '?', '!']).next()?.trim().to_string() + ".";
    topic_cue()
        .captures(&first)
        .and_then(|c| c.name("what").map(|m| m.as_str().to_string()))
        .or_else(|| {
            first_cue().captures(&first).and_then(|c| {
                c.name("what")
                    .or(c.name("what2"))
                    .map(|m| m.as_str().to_string())
            })
        })
        .map(|w| w.trim().to_string())
        .filter(|w| !w.is_empty() && w.split_whitespace().count() <= 5)
}

/// How strongly a new topic seems to start at turn `i`: an announcement, or a long pause.
fn break_strength(turns: &[Turn], i: usize) -> f32 {
    let cue = if starts_topic(&turns[i].text) {
        2.0
    } else {
        0.0
    };
    let pause = match (turns[i - 1].at, turns[i].at) {
        (Some(a), Some(b)) if b > a => {
            // The gap beyond what the previous turn took to say (about 2.5 words a second).
            let spoken = turns[i - 1].text.split_whitespace().count() as f32 / 2.5;
            ((b - a) as f32 - spoken).max(0.0) / 15.0
        }
        _ => 0.0,
    };
    cue + pause.min(2.0)
}

/// The highest value reached while climbing from the start (TextTiling's peak search).
trait Peak {
    fn take_while_peak(self) -> f32;
}
impl<'a, I: Iterator<Item = &'a f32>> Peak for I {
    fn take_while_peak(self) -> f32 {
        let mut best = f32::MIN;
        for &v in self {
            if v < best {
                break;
            }
            best = v;
        }
        best
    }
}

/// Words distinctive of each topic (frequent here, rare elsewhere), and a short label.
fn describe_topics(turns: &[Turn], ranges: &[(usize, usize)]) -> Vec<Topic> {
    let texts: Vec<String> = ranges
        .iter()
        .map(|&(a, b)| {
            turns[a..=b]
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let counts: Vec<nlp::Counts> = texts.iter().map(|t| nlp::count(&words(t))).collect();
    let mut df: HashMap<&str, usize> = HashMap::new();
    for c in &counts {
        for w in c.keys() {
            *df.entry(w.as_str()).or_default() += 1;
        }
    }
    let n = counts.len() as f32;
    ranges
        .iter()
        .zip(&texts)
        .zip(&counts)
        .map(|((&(first, last), text), c)| {
            let weight = |w: &str, tf: u32| tf as f32 * ((n + 1.0) / df[w] as f32).ln().max(0.3);
            let mut kw: Vec<(f32, String)> = c
                .iter()
                .map(|(w, &tf)| (weight(w, tf), w.clone()))
                .collect();
            kw.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            let keywords: Vec<String> = kw.into_iter().take(5).map(|(_, w)| w).collect();
            // A two-word phrase said at least twice makes the best label, preferring phrases
            // this topic has and the others do not.
            let pairs = pairs_all(text);
            // The topic as someone announced it, in its first two turns.
            let announced = turns[first..=last.min(first + 1)]
                .iter()
                .find_map(|t| announced_topic(&t.text));
            let elsewhere = |p: &str| {
                texts
                    .iter()
                    .filter(|t| pairs_all(t).contains_key(p))
                    .count()
            };
            let phrase = pairs
                .iter()
                .filter(|(p, k)| {
                    **k >= 2 && p.split(' ').any(|w| keywords.contains(&w.to_string()))
                })
                .map(|(p, k)| (*k as f32 / elsewhere(p).max(1) as f32, p.clone()))
                .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))
                .map(|(_, p)| surface_phrase(text, &p));
            // One topic for the whole conversation: name its main subjects instead.
            let label = if ranges.len() == 1 {
                let mut top: Vec<(u32, String)> = pairs_all(text)
                    .into_iter()
                    .filter(|(_, k)| *k >= 2)
                    .map(|(p, k)| (k, p))
                    .collect();
                top.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                let mut names: Vec<String> = Vec::new();
                for (_, p) in top {
                    if names.len() < 3 && !names.iter().any(|n| p.split(' ').any(|w| n.contains(w)))
                    {
                        names.push(p);
                    }
                }
                if names.is_empty() {
                    names.extend(keywords.iter().take(3).cloned());
                }
                names.join(", ")
            } else {
                announced
                    .or(phrase)
                    .or_else(|| keywords.first().cloned())
                    .unwrap_or_else(|| "Conversation".into())
            };
            let label = if label.is_empty() {
                "Conversation".into()
            } else {
                label
            };
            let mut l = label.chars();
            let label = l
                .next()
                .map(|f| f.to_uppercase().collect::<String>() + l.as_str())
                .unwrap_or_default();
            Topic {
                first,
                last,
                label,
                keywords,
            }
        })
        .collect()
}

/// A phrase of word stems ("rare pet") as people said it ("rare pets").
fn surface_phrase(text: &str, stems: &str) -> String {
    let wanted: Vec<&str> = stems.split(' ').collect();
    let said = nlp::tokens_surface(text);
    said.windows(wanted.len())
        .find(|w| w.iter().zip(&wanted).all(|((stem, _), want)| stem == want))
        .map(|w| {
            w.iter()
                .map(|(_, s)| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| stems.to_string())
}

/// A word stem as people said it most often ("bonuse" -> "bonuses").
fn surface_word(text: &str, stem: &str) -> String {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for (s, w) in nlp::tokens_surface(text) {
        if s == stem {
            *counts.entry(w).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(w, _)| w)
        .unwrap_or_else(|| stem.to_string())
}

/// Two-word phrases (without chatter) and how often each was said.
fn pairs_all(text: &str) -> HashMap<String, u32> {
    let mut pairs: HashMap<String, u32> = HashMap::new();
    for p in nlp::bigrams(&tidy(text)) {
        if p.split(' ').all(|w| !chatter().contains(w)) {
            *pairs.entry(p).or_default() += 1;
        }
    }
    pairs
}

fn sentences_of(turns: &[Turn]) -> Vec<Sentence> {
    turns
        .iter()
        .enumerate()
        .flat_map(|(i, t)| {
            nlp::sentences(&tidy(&t.text))
                .into_iter()
                .map(move |s| Sentence { turn: i, text: s })
        })
        .collect()
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

fn decision() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(r"(?i)\b(we(?:'ve| have)? (?:decided|agreed)|(?:it's|it is|that's) (?:decided|agreed)|let'?s go (?:with|ahead)|let'?s (?:do|ship|use|keep|stick with|move forward|launch|release)\b|we(?:'ll| will| are going to|'re going to) (?:go with|ship|use|launch|release|keep|move)|the (?:decision|plan) is|decision(?: is|:)|(?:is|are) approved|final answer|agreed\b)")
    })
}

fn commitment() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(r"(?i)^(?:okay,?\s|ok,?\s|sure,?\s|yes,?\s|yeah,?\s|so,?\s|and\s|then\s|will do,?\s|no problem,?\s|on it,?\s|absolutely,?\s|of course,?\s)*(?:i'?ll|i will|i'?m going to|i can take|i'?ll take|let me)\b(?:\s+(\w+))?")
    })
}

fn request() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(r"(?i)^(?:(?P<name>[A-Z][a-z]+),\s+)?(?:can|could|would|will) you\b|^(?:(?P<name2>[A-Z][a-z]+),\s+)?please\b|\baction items?:")
    })
}

fn due() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(r"(?i)\b(?:by|before|on|for|until) (?P<d>(?:next |this )?(?:monday|tuesday|wednesday|thursday|friday|saturday|sunday)(?:'s meeting)?|tomorrow|tonight|today|end of (?:the )?(?:day|week|month|sprint)|eod|eow|next week|next month|the \d{1,2}(?:st|nd|rd|th)?|(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]* \d{1,2})\b|\b(?P<n>next week|tomorrow|tonight|today)\b")
    })
}

/// Commitments that are only ways of speaking.
const NOT_TASKS: &[&str] = &[
    "be", "say", "admit", "tell", "just", "think", "guess", "assume", "let", "see", "know", "bet",
    "start", "keep", "stop", "go", "pass", "jump", "give",
];

fn affirmative(text: &str) -> bool {
    let t = text.trim_start().to_lowercase();
    [
        "sure",
        "yes",
        "yeah",
        "yep",
        "ok",
        "okay",
        "will do",
        "on it",
        "i'll",
        "i will",
        "i can",
        "no problem",
        "absolutely",
        "of course",
        "definitely",
    ]
    .iter()
    .any(|a| t.starts_with(a))
}

fn unsure(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "don't know",
        "dont know",
        "not sure",
        "no idea",
        "good question",
        "find out",
        "we'll check",
        "i'll check",
        "let me check",
        "need to check",
        "unclear",
        "no one knows",
        "nobody knows",
    ]
    .iter()
    .any(|p| t.contains(p))
}

fn first_name(speaker: &str) -> &str {
    speaker.split_whitespace().next().unwrap_or(speaker)
}

struct Found {
    turn: usize,
    who: String,
    text: String,
    due: Option<String>,
}

pub fn summarize(t: &Transcript, model: Option<&dyn Rewriter>) -> Summary {
    let turns = merge_turns(&t.utterances);
    let sentences = sentences_of(&turns);
    let ranges = segment(&turns);
    let topics = describe_topics(&turns, &ranges);
    let topic_of = |turn: usize| {
        topics
            .iter()
            .position(|tp| turn >= tp.first && turn <= tp.last)
    };
    let mut notes = Vec::new();

    // Decisions, action items and questions, each tied to its turn.
    let mut decisions: Vec<Found> = Vec::new();
    let mut actions: Vec<Found> = Vec::new();
    let mut questions: Vec<Found> = Vec::new();
    for s in &sentences {
        let speaker = &turns[s.turn].speaker;
        let text = s.text.trim();
        let words_n = text.split_whitespace().count();
        let is_question = text.ends_with('?');
        let due_of = |x: &str| {
            due()
                .captures(x)
                .and_then(|c| c.name("d").or(c.name("n")).map(|m| m.as_str().to_string()))
        };
        if !is_question && words_n >= 3 && decision().is_match(text) {
            decisions.push(Found {
                turn: s.turn,
                who: speaker.clone(),
                text: text.to_string(),
                due: None,
            });
        }
        if !is_question
            && words_n >= 4
            && let Some(c) = commitment().captures(text)
            && !c
                .get(1)
                .is_some_and(|w| NOT_TASKS.contains(&w.as_str().to_lowercase().as_str()))
            && !text.to_lowercase().contains("let me know")
        {
            // The reply to a request made of this speaker: one item, request then answer.
            match actions.last_mut() {
                Some(a) if a.who == *speaker && a.turn + 1 == s.turn && a.text.ends_with('?') => {
                    a.text = format!("{}” → “{}", a.text, text);
                    a.turn = s.turn;
                    if a.due.is_none() {
                        a.due = due_of(text);
                    }
                }
                _ => actions.push(Found {
                    turn: s.turn,
                    who: speaker.clone(),
                    text: text.to_string(),
                    due: due_of(text),
                }),
            }
        } else if let Some(c) = request().captures(text) {
            // A request: the person named, or the next speaker if they agree.
            let named = c
                .name("name")
                .or(c.name("name2"))
                .map(|m| m.as_str().to_string());
            let next = turns.get(s.turn + 1);
            let owner = named
                .and_then(|n| {
                    turns
                        .iter()
                        .map(|t| t.speaker.as_str())
                        .find(|sp| first_name(sp).eq_ignore_ascii_case(&n))
                        .map(str::to_string)
                })
                .or_else(|| {
                    next.filter(|n| n.speaker != *speaker && affirmative(&n.text))
                        .map(|n| n.speaker.clone())
                });
            if let Some(owner) = owner {
                let reply_due = next.and_then(|n| due_of(&n.text));
                actions.push(Found {
                    turn: s.turn,
                    who: owner,
                    text: text.to_string(),
                    due: due_of(text).or(reply_due),
                });
            }
        }
        if is_question && words_n >= 5 && request().captures(text).is_none() {
            let tail = text.to_lowercase();
            let tag = [
                "right?",
                "okay?",
                "ok?",
                "you know?",
                "yeah?",
                "correct?",
                "no?",
            ]
            .iter()
            .any(|t| tail.ends_with(t) && words_n < 8);
            // Open when the next other speaker does not answer it, or says they do not know.
            let answer = turns[s.turn + 1..]
                .iter()
                .take(2)
                .find(|n| n.speaker != *speaker);
            // Answered unless the reply opens with another question or says nobody knows.
            let open = match answer {
                None => true,
                Some(a) => {
                    let opening = nlp::sentences(&tidy(&a.text))
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    unsure(&a.text) || opening.trim_end().ends_with('?')
                }
            };
            if !tag && open {
                questions.push(Found {
                    turn: s.turn,
                    who: speaker.clone(),
                    text: text.to_string(),
                    due: None,
                });
            }
        }
    }
    for list in [&mut decisions, &mut actions, &mut questions] {
        let mut seen = HashSet::new();
        list.retain(|f| seen.insert(f.text.to_lowercase()));
        list.truncate(15);
    }

    // Each topic: a model summary, or its most informative sentences.
    let mut written_by_model = 0usize;
    let per_topic: Vec<String> = topics
        .iter()
        .map(|tp| {
            let model_text = model.and_then(|m| {
                // The model reads the conversation and the facts the rules already checked,
                // so it does not mistake an open question or a suggestion for a decision.
                let mine = |list: &[Found]| -> Vec<String> {
                    list.iter()
                        .filter(|f| f.turn >= tp.first && f.turn <= tp.last)
                        .map(|f| format!("{}: {}", first_name(&f.who), f.text.replace(['“', '”'], "")))
                        .collect()
                };
                let listed = |items: Vec<String>| {
                    if items.is_empty() {
                        "none".to_string()
                    } else {
                        items.join("; ")
                    }
                };
                let facts = format!(
                    "Decisions: {}\nAction items: {}\nStill unknown: {}",
                    listed(mine(&decisions)),
                    listed(mine(&actions)),
                    listed(mine(&questions))
                );
                let said = conversation(&turns[tp.first..=tp.last]);
                let result = condense(m, &said).and_then(|said| {
                    m.complete(&format!(
                        "Here is part of a meeting, about the {}.\n\n{said}\n\nFacts already checked:\n{facts}\n\nSummarize this part in two or three sentences: the problem, what was decided, and who will do what. Only the decisions listed above were made; if none are listed, say that nothing was decided yet. Do not state anything that is still unknown as a fact.",
                        tp.label.to_lowercase()
                    ))
                });
                let no_decisions = mine(&decisions).is_empty();
                match result {
                    // The rules found no decision here: the model may not claim one.
                    Ok(s) if !s.trim().is_empty() && no_decisions => Some(soften_decisions(&s)),
                    Ok(s) if !s.trim().is_empty() => Some(s),
                    Ok(_) => None,
                    Err(e) => {
                        notes.push(format!("a model summary ({e:#})"));
                        None
                    }
                }
            });
            match model_text {
                Some(s) => {
                    written_by_model += 1;
                    format!("  - {}\n", s.trim())
                }
                None => key_sentences(&turns, &sentences, tp, 2)
                    .into_iter()
                    .map(|(who, s)| format!("  - {}: {}\n", first_name(&who), s))
                    .collect(),
            }
        })
        .collect();

    // At a glance.
    let total_words: usize = turns
        .iter()
        .map(|t| t.text.split_whitespace().count())
        .sum();
    let mut speakers: Vec<(String, usize, usize)> = Vec::new(); // name, words, turns
    for t in &turns {
        let n = t.text.split_whitespace().count();
        match speakers.iter_mut().find(|s| s.0 == t.speaker) {
            Some(s) => {
                s.1 += n;
                s.2 += 1;
            }
            None => speakers.push((t.speaker.clone(), n, 1)),
        }
    }
    speakers.sort_by_key(|s| std::cmp::Reverse(s.1));
    let times: Vec<u32> = t.utterances.iter().filter_map(|u| u.at).collect();
    let length = match (times.first(), times.last()) {
        (Some(a), Some(b)) if b > a => Some(duration(b - a)),
        _ => None,
    };
    let mut glance = vec!["Meeting transcript".to_string()];
    glance.extend(length.clone());
    glance.push(plural(speakers.len(), "speaker"));
    glance.push(plural(topics.len(), "topic"));
    glance.push(plural(total_words, "word"));

    let when = |turn: usize| {
        turns[turn]
            .at
            .map(|a| format!(" · {}", stamp(a)))
            .unwrap_or_default()
    };
    let about = |turn: usize| {
        topic_of(turn)
            .filter(|_| topics.len() > 1)
            .map(|i| format!(" · about {}", topics[i].label.to_lowercase()))
            .unwrap_or_default()
    };
    let mut md = format!("### At a glance\n{}\n", glance.join(" · "));
    if !decisions.is_empty() {
        md.push_str("\n### Decisions\n");
        for d in &decisions {
            md.push_str(&format!(
                "- “{}” · {}{}\n",
                d.text,
                first_name(&d.who),
                when(d.turn)
            ));
        }
    }
    if !actions.is_empty() {
        md.push_str("\n### Action items\n");
        for a in &actions {
            let due = a
                .due
                .as_ref()
                .map(|d| format!(" · due {d}"))
                .unwrap_or_default();
            md.push_str(&format!(
                "- **{}**: “{}”{due}{}{}\n",
                first_name(&a.who),
                a.text,
                about(a.turn),
                when(a.turn)
            ));
        }
    }
    if !questions.is_empty() {
        md.push_str("\n### Questions left open\n");
        for q in &questions {
            md.push_str(&format!(
                "- “{}” · {}{}\n",
                q.text,
                first_name(&q.who),
                when(q.turn)
            ));
        }
    }
    md.push_str("\n### Topics\n");
    if let Some(m) = model.filter(|_| written_by_model > 0) {
        md.push_str(&format!(
            "_Topic summaries written by a language model ({}) on this computer from the notes found in the transcript; check details against the transcript below._\n\n",
            m.name()
        ));
    }
    for (tp, summary) in topics.iter().zip(&per_topic) {
        let span = match (turns[tp.first].at, turns[tp.last].at) {
            (Some(a), Some(b)) => format!("{}–{} · ", stamp(a), stamp(b)),
            (Some(a), None) => format!("{} · ", stamp(a)),
            _ => String::new(),
        };
        let mut who: Vec<&str> = Vec::new();
        for t in &turns[tp.first..=tp.last] {
            let f = first_name(&t.speaker);
            if !who.contains(&f) {
                who.push(f);
            }
        }
        md.push_str(&format!(
            "- **{span}{}** · {}\n{summary}",
            tp.label,
            who.join(", ")
        ));
    }
    md.push_str("\n### Who said what\n");
    let everyone = nlp::count(&words(
        &turns
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    ));
    for (name, n, turns_n) in &speakers {
        let share = n * 100 / total_words.max(1);
        let theirs: String = turns
            .iter()
            .filter(|t| &t.speaker == name)
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        // Words this person used more than the others did.
        let mine = nlp::count(&words(&theirs));
        let mut c: Vec<(f32, String)> = mine
            .iter()
            .filter(|(w, _)| w.chars().count() >= 4)
            .map(|(w, &k)| {
                let others = everyone.get(w).copied().unwrap_or(0).saturating_sub(k);
                (k as f32 / (1.0 + others as f32), w.clone())
            })
            .collect();
        c.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let top: Vec<String> = c
            .into_iter()
            .take(3)
            .map(|(_, w)| surface_word(&theirs, &w))
            .collect();
        let topics_said = if top.is_empty() {
            String::new()
        } else {
            format!(" · _{}_", top.join(", "))
        };
        md.push_str(&format!(
            "- **{name}** · {share}% of the words, {}{topics_said}\n",
            plural(*turns_n, "turn")
        ));
    }

    // The front matter description.
    let names: Vec<&str> = speakers.iter().map(|s| first_name(&s.0)).take(5).collect();
    let description = {
        {
            let labels: Vec<String> = topics
                .iter()
                .take(4)
                .map(|t| t.label.to_lowercase())
                .collect();
            let len = length.map(|l| format!("{l}, ")).unwrap_or_default();
            format!(
                "Meeting transcript ({len}{}: {}) covering {}.",
                plural(speakers.len(), "speaker"),
                names.join(", "),
                labels.join(", ")
            )
        }
    };

    // The full conversation, by topic.
    let mut body = String::new();
    for tp in &topics {
        let span = match (turns[tp.first].at, turns[tp.last].at) {
            (Some(a), Some(b)) => format!(" ({}–{})", stamp(a), stamp(b)),
            _ => String::new(),
        };
        if topics.len() > 1 {
            body.push_str(&format!("## {}{span}\n\n", tp.label));
        }
        for t in &turns[tp.first..=tp.last] {
            let at = t.at.map(|a| format!(" ({})", stamp(a))).unwrap_or_default();
            body.push_str(&format!("**{}**{at}: {}\n\n", t.speaker, t.text.trim()));
        }
    }
    Summary {
        markdown: md,
        description,
        body,
        notes,
    }
}

/// "The decision was made to add X" -> "They discussed whether to add X", for topics where
/// nothing was decided: a small model sometimes reports a suggestion as a decision.
fn soften_decisions(text: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| {
        re(r"(?i)\b(?:the\s+)?(?:decision\s+(?:was\s+made|was)|team\s+decided|they\s+decided|it\s+was\s+decided|(?:\w+\s+)?decided)\s+to\s+")
    });
    let out = r
        .replace_all(text, "they discussed whether to ")
        .to_string();
    // Sentence starts stay capitalised.
    out.replace(". they discussed", ". They discussed")
        .replacen(
            "they discussed",
            "They discussed",
            usize::from(out.starts_with("they")),
        )
}

/// Turns as "Name: words", filler removed, for the model.
fn conversation(turns: &[Turn]) -> String {
    turns
        .iter()
        .map(|t| format!("{}: {}", first_name(&t.speaker), tidy(&t.text)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A conversation short enough for the model: as it is when it fits, otherwise each piece
/// summarised in a few sentences (and those summaries again, if still too long).
fn condense(m: &dyn Rewriter, text: &str) -> anyhow::Result<String> {
    let limit = m.max_words().max(50);
    let mut text = text.to_string();
    for _ in 0..4 {
        if text.split_whitespace().count() <= limit {
            return Ok(text);
        }
        // Pieces of whole turns, so each speaker stays attached to their words.
        let mut pieces: Vec<String> = Vec::new();
        let mut cur = String::new();
        for line in text.lines() {
            for chunk in line
                .split_whitespace()
                .collect::<Vec<_>>()
                .chunks(limit)
                .map(|c| c.join(" "))
            {
                if cur.split_whitespace().count() + chunk.split_whitespace().count() > limit
                    && !cur.is_empty()
                {
                    pieces.push(std::mem::take(&mut cur));
                }
                if !cur.is_empty() {
                    cur.push('\n');
                }
                cur.push_str(&chunk);
            }
        }
        if !cur.is_empty() {
            pieces.push(cur);
        }
        let mut parts = Vec::new();
        for p in &pieces {
            parts.push(m.complete(&format!(
                "Summarize this part of a meeting in three or four sentences, keeping who said what, every decision and every task:\n\n{p}"
            ))?);
        }
        text = parts.join("\n");
    }
    Ok(text)
}

/// The topic's most informative sentences (by its distinctive words), in the order said.
fn key_sentences(
    turns: &[Turn],
    sentences: &[Sentence],
    tp: &Topic,
    how_many: usize,
) -> Vec<(String, String)> {
    let weights: HashMap<&str, f32> = tp
        .keywords
        .iter()
        .enumerate()
        .map(|(i, w)| (w.as_str(), 1.0 / (1.0 + i as f32)))
        .collect();
    let mut scored: Vec<(f32, usize)> = sentences
        .iter()
        .enumerate()
        .filter(|(_, s)| s.turn >= tp.first && s.turn <= tp.last)
        .filter(|(_, s)| {
            let n = s.text.split_whitespace().count();
            (6..=45).contains(&n) && !s.text.ends_with('?')
        })
        .map(|(i, s)| {
            let toks = words(&s.text);
            let score: f32 = toks
                .iter()
                .map(|w| weights.get(w.as_str()).copied().unwrap_or(0.0))
                .sum::<f32>()
                + toks.len() as f32 * 0.05;
            (score / (s.text.split_whitespace().count() as f32).sqrt(), i)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut pick: Vec<usize> = scored.into_iter().take(how_many).map(|(_, i)| i).collect();
    pick.sort_unstable();
    pick.into_iter()
        .map(|i| {
            (
                turns[sentences[i].turn].speaker.clone(),
                sentences[i].text.clone(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::from_text;

    fn meeting() -> Transcript {
        let text = "\
[0:00:05] Maya: Okay so, um, the main thing today is the pet hatching update. We said Saturday last week but the trading bug is still there.
[0:00:20] Jordan: Yeah, I looked at it. Players can duplicate rare pets if they trade during a server shutdown. I think I can fix it by locking the profile while the trade saves.
[0:00:41] Maya: How long would that take?
[0:00:45] Jordan: Probably two days. I'll have it done by Friday.
[0:00:52] Sam: If it's Friday then we can still ship Saturday, right?
[0:01:00] Maya: Yes. Let's go with Saturday then, as long as Jordan's fix is in by Friday. I'll write the patch notes.
[0:01:30] Sam: Another thing, the rare pet rate feels too low. People are complaining on the forum that they hatch a hundred eggs and get nothing.
[0:01:45] Lee: The rate is one in fifty right now. We could add a pity counter so you're guaranteed a rare after forty hatches.
[0:02:00] Maya: I like the pity counter. Lee, can you put together the numbers for next week?
[0:02:05] Lee: Sure, I'll bring a spreadsheet to Tuesday's meeting.
[0:02:10] Sam: Do rare pets keep their stats after trading?
[0:02:15] Maya: Good question, I don't actually know. Let's find out.
";
        Transcript {
            utterances: from_text(text),
        }
    }

    #[test]
    fn finds_decisions_action_items_and_open_questions() {
        let s = summarize(&meeting(), None);
        let md = &s.markdown;
        assert!(
            md.contains("Meeting transcript · 2 min · 4 speakers"),
            "{md}"
        );
        assert!(
            md.contains("### Decisions\n- “Let's go with Saturday then"),
            "{md}"
        );
        assert!(
            md.contains("- **Jordan**: “I'll have it done by Friday.” · due Friday"),
            "{md}"
        );
        assert!(
            md.contains("- **Maya**: “I'll write the patch notes.”"),
            "{md}"
        );
        // A request and its owner's reply are one item.
        assert!(md.contains("- **Lee**: “Lee, can you put together the numbers for next week?” → “Sure, I'll bring a spreadsheet to Tuesday's meeting.” · due next week"), "{md}");
        assert!(md.contains("### Questions left open\n- “Do rare pets keep their stats after trading?” · Sam · 2:10"), "{md}");
        // Answered questions and tag questions are not open.
        assert!(!md.contains("How long would that take"), "{md}");
        assert!(!md.contains("ship Saturday, right?"), "{md}");
        assert!(md.contains("### Who said what\n- **Maya**"), "{md}");
        assert!(
            s.description
                .starts_with("Meeting transcript (2 min, 4 speakers: Maya"),
            "{}",
            s.description
        );
        assert!(
            s.body.contains("**Jordan** (0:20): Yeah, I looked at it."),
            "{}",
            s.body
        );
    }

    #[test]
    fn a_suggestion_is_not_reported_as_a_decision() {
        assert_eq!(
            soften_decisions(
                "The problem is rare pets. The decision was made to add a pity counter."
            ),
            "The problem is rare pets. They discussed whether to add a pity counter."
        );
        assert_eq!(
            soften_decisions("The team decided to wait."),
            "They discussed whether to wait."
        );
    }

    #[test]
    fn filler_is_removed_for_analysis() {
        assert_eq!(
            tidy("um, so the the saves, uh, failed"),
            "So the saves, failed"
        );
    }

    struct Echo;
    impl Rewriter for Echo {
        fn complete(&self, prompt: &str) -> anyhow::Result<String> {
            Ok(format!("[{} words]", prompt.split_whitespace().count()))
        }
        fn name(&self) -> String {
            "Echo".into()
        }
        fn max_words(&self) -> usize {
            40
        }
    }

    #[test]
    fn a_model_writes_the_topic_summaries_and_is_labelled() {
        let s = summarize(&meeting(), Some(&Echo));
        assert!(
            s.markdown.contains("written by a language model (Echo)"),
            "{}",
            s.markdown
        );
        assert!(
            s.markdown.contains("  - [") && !s.markdown.contains("### Overview"),
            "{}",
            s.markdown
        );
        // Longer than the model's window: summarised in pieces, then the pieces summarised.
        let long = vec!["word"; 500].join(" ");
        let out = condense(&Echo, &long).unwrap();
        assert!(
            out.lines().count() > 1,
            "summarised in several pieces: {out}"
        );
        assert!(
            out.starts_with("[") && out.split_whitespace().count() <= 40,
            "{out}"
        );
        // Short enough: passed through as it is.
        assert_eq!(condense(&Echo, "Maya: hi").unwrap(), "Maya: hi");
    }
}
