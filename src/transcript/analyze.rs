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
    /// The meeting's title, when the transcript states one.
    pub title: Option<String>,
    /// Front matter after the description: date, time, duration, place, topic, attendees.
    pub meta: Vec<(String, crate::Meta)>,
}

/// A turn after merging: one speaker's consecutive lines.
#[derive(Clone)]
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

/// What a turn is about: the meeting's work, small talk, or buzzwords with nothing concrete.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Work,
    SmallTalk,
    Buzz,
}

/// Small talk: weekends, weather, sport, food, commutes, the office.
fn small_talk() -> &'static HashSet<String> {
    static S: OnceLock<HashSet<String>> = OnceLock::new();
    S.get_or_init(|| {
        nlp::tokens(
            "weekend weekends weather rain rained raining sunny snow hike hiking dog cat kids \
             traffic commute bridge coffee cafe latte espresso lunch dinner winner \
             breakfast burrito pizza thai sushi restaurant reviews drinks beer wine night \
             match football soccer basketball baseball hockey striker volley stoppage league \
             fantasy playoffs highlights tv show binged binge netflix movie movies vacation \
             holiday birthday party gym laundry neighbours neighbors parking cousin yelling \
             hungry tired sleep sleeping wedding concert festival season stuck",
        )
        .into_iter()
        .collect()
    })
}

/// Office buzzwords: phrases that sound like work but say nothing concrete.
const BUZZWORDS: &[&str] = &[
    "leverage",
    "synergy",
    "synergies",
    "align on",
    "aligned on",
    "all aligned",
    "alignment",
    "holistic",
    "north star",
    "boil the ocean",
    "boiling the ocean",
    "double-click",
    "double click",
    "low-hanging fruit",
    "move the needle",
    "moving the needle",
    "going forward",
    "ecosystem",
    "paradigm",
    "empower",
    "stakeholders",
    "circle back",
    "deliverables",
    "touch base",
    "take this offline",
    "take it offline",
    "big picture",
    "strategic perspective",
    "core competencies",
    "core competency",
    "stay agile",
    "at the end of the day",
    "deep dive",
    "learnings",
    "adding value",
    "add value",
    "value add",
    "one hundred percent",
    "great energy",
    "bandwidth",
    "best practices",
    "actionable",
    "mindshare",
    "thought leadership",
    "game changer",
    "win-win",
    "think outside the box",
    "on the same page",
    "reach out",
    "drill down",
    "net-net",
];

fn buzzwords_in(text: &str) -> Vec<&'static str> {
    let t = text.to_lowercase();
    BUZZWORDS
        .iter()
        .copied()
        .filter(|b| t.contains(b))
        .collect()
}

/// Sort every turn: small talk when its words are mostly about life outside work, buzzwords
/// when it is mostly phrases with nothing concrete, work otherwise. Short replies ("Ha, nice.")
/// belong with whatever was being talked about.
fn classify(turns: &[Turn]) -> Vec<Kind> {
    // Signs of real work: a number, a decision, a request or a commitment.
    let working = |t: &str| {
        t.chars().any(|c| c.is_ascii_digit())
            || decision().is_match(t)
            || nlp::sentences(t)
                .iter()
                .any(|s| request().is_match(s) || commitment().is_match(s))
    };
    let scores: Vec<(usize, usize, usize, bool)> = turns
        .iter()
        .map(|t| {
            let content = words(&t.text);
            let small = content.iter().filter(|w| small_talk().contains(*w)).count();
            (
                content.len(),
                small,
                buzzwords_in(&t.text).len(),
                working(&t.text),
            )
        })
        .collect();
    let mut kinds: Vec<Kind> = Vec::with_capacity(turns.len());
    for &(content, small, buzz, work) in &scores {
        // One small-talk word in five is enough, unless the turn also does real work.
        let chatty = small > 0 && small * 5 >= content && (!work || small * 2 >= content);
        let kind = if content < 3 && buzz == 0 && small == 0 {
            kinds.last().copied().unwrap_or(Kind::Work)
        } else if chatty {
            Kind::SmallTalk
        } else if !work && (buzz >= 2 || (buzz >= 1 && content <= 10)) {
            Kind::Buzz
        } else {
            Kind::Work
        };
        kinds.push(kind);
    }
    // A turn beside small talk, with a small-talk word and nothing decided or asked, is part
    // of the same digression ("Fine, pizza it is. I'll book a table."). Repeated, so a
    // digression grows outwards turn by turn.
    let decides =
        |t: &str| decision().is_match(t) || nlp::sentences(t).iter().any(|s| request().is_match(s));
    for _ in 0..3 {
        for i in 0..kinds.len() {
            let (_, small, _, _) = scores[i];
            let beside = (i > 0 && kinds[i - 1] == Kind::SmallTalk)
                || (i + 1 < kinds.len() && kinds[i + 1] == Kind::SmallTalk);
            if kinds[i] == Kind::Work && small > 0 && beside && !decides(&turns[i].text) {
                kinds[i] = Kind::SmallTalk;
            }
        }
    }
    // Short replies ("Great, thanks.") go with what was being talked about.
    for i in 1..kinds.len() {
        let (content, small, buzz, _) = scores[i];
        if content < 3 && small == 0 && buzz == 0 {
            kinds[i] = kinds[i - 1];
        }
    }
    kinds
}

/// "Agreed." / "Okay, agreed." / "Deal." on its own.
fn is_agreement(sentence: &str) -> bool {
    let t = sentence.trim().trim_end_matches(['.', '!']).to_lowercase();
    let t = t
        .trim_start_matches("okay, ")
        .trim_start_matches("ok, ")
        .trim_start_matches("yes, ");
    matches!(t, "agreed" | "agreed then" | "agreed, then" | "deal")
}

fn goal_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(r"(?i)\b(?:the (?:goal|purpose|point|aim|objective|agenda)(?: of| for)?(?: (?:this|today'?s|the) (?:meeting|call|sync|session))?(?: today)? (?:is|was)|(?:today|this meeting|this call) (?:we(?:'re| are) (?:going to|here to)|is (?:about|for))|we(?:'re| are) here to|the main (?:thing|topic|point|item|question)s? (?:today|for today|this week)? ?(?:is|are)|(?:one|two|three|four|five|a few|a couple of) things (?:today|to cover|on the agenda)|(?:(?:i|we) )?(?:just )?want(?:ed)? to (?:talk about|go over|discuss|sync on|touch base|make sure|decide|agree|review)|let'?s (?:go over|discuss|review|agree))")
    })
}

/// What the meeting was for: the agenda line above the conversation, or the first early
/// sentence that says so ("The goal today is ...", "Three things today: ...").
fn find_goal(turns: &[Turn], agenda: Option<&str>) -> Option<(Option<usize>, String)> {
    if let Some(a) = agenda {
        return Some((None, a.to_string()));
    }
    for (i, t) in turns.iter().enumerate().take(12) {
        for s in nlp::sentences(&tidy(&t.text)) {
            if goal_pattern().is_match(&s) && s.split_whitespace().count() >= 5 {
                return Some((Some(i), s));
            }
        }
    }
    None
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
        re(r"(?i)\b(we(?:'ve| have)? (?:decided|agreed)|(?:it's|it is|that's) (?:decided|agreed)|let'?s go (?:with|ahead)|let'?s (?:do|ship|use|keep|stick with|move forward|launch|release)\b|we(?:'ll| will| are going to|'re going to) (?:go with|ship|use|launch|release|keep|move)|the (?:decision|plan) is|decision(?: is|:)|(?:is|are) approved|final answer|(?:we|everyone|all) agreed\b)")
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
    let all = merge_turns(&t.utterances);
    let kinds = classify(&all);
    // Everything below works on the meeting's real work; small talk and buzzwords are
    // reported separately, under Miscellaneous.
    let work_at: Vec<usize> = (0..all.len()).filter(|&i| kinds[i] == Kind::Work).collect();
    let turns: Vec<Turn> = work_at.iter().map(|&i| all[i].clone()).collect();
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
    let mut after_agreed = false;
    let mut last_turn = usize::MAX;
    for s in &sentences {
        let speaker = &turns[s.turn].speaker;
        if s.turn != last_turn {
            after_agreed = false;
            last_turn = s.turn;
        }
        // "Agreed." on its own: the statement after it, in the same turn, is the decision.
        let agreed_here = std::mem::take(&mut after_agreed);
        if is_agreement(&s.text) {
            after_agreed = true;
            continue;
        }
        let text = s.text.trim();
        let words_n = text.split_whitespace().count();
        let is_question = text.ends_with('?');
        let due_of = |x: &str| {
            due()
                .captures(x)
                .and_then(|c| c.name("d").or(c.name("n")).map(|m| m.as_str().to_string()))
        };
        let concrete_enough =
            buzzwords_in(text).is_empty() || text.chars().any(|c| c.is_ascii_digit());
        if !is_question
            && words_n >= 3
            && concrete_enough
            && (decision().is_match(text) || agreed_here)
        {
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
    // Two questions in one breath are one item.
    let mut merged: Vec<Found> = Vec::new();
    for q in questions {
        match merged.last_mut() {
            Some(p) if p.turn == q.turn => {
                p.text.push(' ');
                p.text.push_str(&q.text);
            }
            _ => merged.push(q),
        }
    }
    let mut questions = merged;
    for list in [&mut decisions, &mut actions, &mut questions] {
        let mut seen = HashSet::new();
        list.retain(|f| seen.insert(f.text.to_lowercase()));
        list.truncate(15);
    }

    // Each topic: a model summary, or its most informative sentences.
    let mut spoke: Vec<String> = Vec::new();
    for t in &all {
        if !spoke.contains(&t.speaker) {
            spoke.push(t.speaker.clone());
        }
    }
    let details = super::meeting::details(&t.header, &t.file_name, t.teams, &spoke);
    let goal = find_goal(&all, details.agenda.as_deref().or(details.topic.as_deref()));
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
                    let purpose = goal
                        .as_ref()
                        .map(|(_, g)| format!("The meeting's stated goal: {g}\n"))
                        .unwrap_or_default();
                    m.complete(&format!(
                        "{purpose}Here is part of a meeting, about the {}.\n\n{said}\n\nFacts already checked:\n{facts}\n\nSummarize this part in two or three sentences: the problem, what was decided, and who will do what. Only the decisions listed above were made; if none are listed, say that nothing was decided yet. Do not state anything that is still unknown as a fact.",
                        tp.label.to_lowercase()
                    ))
                });
                let no_decisions = mine(&decisions).is_empty();
                let owners: Vec<(String, String)> = actions
                    .iter()
                    .filter(|f| f.turn >= tp.first && f.turn <= tp.last)
                    .map(|f| (first_name(&f.who).to_string(), f.text.clone()))
                    .collect();
                let result = result.map(|s| fix_owners(&s, &owners));
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
                    format!("{}\n", s.trim())
                }
                None => key_sentences(&turns, &sentences, tp, 2)
                    .into_iter()
                    .map(|(who, s)| format!("- {}: {}\n", first_name(&who), s))
                    .collect(),
            }
        })
        .collect();

    // Who spoke, over the whole meeting.
    let total_words: usize = all.iter().map(|t| t.text.split_whitespace().count()).sum();
    let mut speakers: Vec<(String, usize, usize)> = Vec::new(); // name, words, turns
    for t in &all {
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
    let share = |kind: Kind| -> usize {
        let n: usize = all
            .iter()
            .zip(&kinds)
            .filter(|(_, k)| **k == kind)
            .map(|(t, _)| t.text.split_whitespace().count())
            .sum();
        (n * 100 + total_words / 2) / total_words.max(1)
    };
    let (on_topic, small_share, buzz_share) =
        (share(Kind::Work), share(Kind::SmallTalk), share(Kind::Buzz));

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
    let span = |a: Option<u32>, b: Option<u32>| match (a, b) {
        (Some(a), Some(b)) if b > a => format!("{}–{}", stamp(a), stamp(b)),
        (Some(a), _) => stamp(a),
        _ => String::new(),
    };

    // Goal.
    let mut md = String::from("## Goal\n\n");
    match &goal {
        Some((Some(i), g)) => {
            let vague = buzzwords_in(g).len() >= 2 || kinds[*i] == Kind::Buzz;
            md.push_str(&format!(
                "“{g}” · {}{}{}\n",
                first_name(&all[*i].speaker),
                all[*i]
                    .at
                    .map(|a| format!(" · {}", stamp(a)))
                    .unwrap_or_default(),
                if vague {
                    "\n\n_Stated only in general terms: no concrete subject._"
                } else {
                    ""
                }
            ));
        }
        Some((None, g)) => md.push_str(&format!("{g} _(from the meeting's agenda)_\n")),
        None => md.push_str("_No goal was stated._\n"),
    }

    // Outcome: what people act on.
    let mut verdict: Vec<String> = Vec::new();
    if decisions.is_empty() && actions.is_empty() && questions.is_empty() {
        verdict.push("No decisions, action items or open questions".into());
    } else {
        verdict.push(plural(decisions.len(), "decision"));
        verdict.push(plural(actions.len(), "action item"));
        verdict.push(plural(questions.len(), "open question"));
    }
    verdict.push(format!("{on_topic}% on topic"));
    md.push_str(&format!("\n## Outcome\n\n**{}**\n", verdict.join(" · ")));
    if decisions.is_empty() && actions.is_empty() && questions.is_empty() {
        let drift = 100 - on_topic;
        md.push_str(&format!(
            "\n_Nothing was decided or assigned, and no questions were left open.{}_\n",
            if drift >= 50 {
                format!(" Most of the meeting ({drift}%) was small talk or buzzwords.")
            } else {
                String::new()
            }
        ));
    }
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
        md.push_str("\n### Open questions\n");
        for q in &questions {
            md.push_str(&format!(
                "- “{}” · {}{}\n",
                q.text,
                first_name(&q.who),
                when(q.turn)
            ));
        }
    }

    // Summary: the on-topic discussion, by subject.
    md.push_str("\n## Summary\n\n");
    if topics.is_empty() {
        md.push_str("_No on-topic discussion to summarize._\n");
    }
    if let Some(m) = model.filter(|_| written_by_model > 0) {
        md.push_str(&format!(
            "_Written by a language model ({}) on this computer from the on-topic conversation and the outcome above; check details against the transcript below._\n\n",
            m.name()
        ));
    }
    for (tp, summary) in topics.iter().zip(&per_topic) {
        let mut who: Vec<&str> = Vec::new();
        for t in &turns[tp.first..=tp.last] {
            let f = first_name(&t.speaker);
            if !who.contains(&f) {
                who.push(f);
            }
        }
        let when = span(turns[tp.first].at, turns[tp.last].at);
        let when = if when.is_empty() {
            String::new()
        } else {
            format!(" · {when}")
        };
        md.push_str(&format!(
            "### {}{when} · {}\n\n{}\n\n",
            tp.label,
            who.join(", "),
            summary.trim_end()
        ));
    }

    // Miscellaneous: off-topic stretches, buzzwords, who spoke.
    md = md.trim_end().to_string() + "\n";
    md.push_str("\n## Miscellaneous\n");
    let mut stretches: Vec<(usize, usize)> = Vec::new();
    for (i, k) in kinds.iter().enumerate() {
        if *k != Kind::SmallTalk {
            continue;
        }
        match stretches.last_mut() {
            // Consecutive small talk (a one-turn interruption does not split it).
            Some(s) if i <= s.1 + 2 => s.1 = i,
            _ => stretches.push((i, i)),
        }
    }
    if !stretches.is_empty() {
        md.push_str(&format!(
            "\n### Off topic · {small_share}% of the meeting\n"
        ));
        for &(a, b) in &stretches {
            let text: String = all[a..=b]
                .iter()
                .zip(&kinds[a..=b])
                .filter(|(_, k)| **k == Kind::SmallTalk)
                .map(|(t, _)| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let mut c: Vec<(u32, String)> = nlp::count(&words(&text))
                .into_iter()
                .filter(|(w, _)| small_talk().contains(w))
                .map(|(w, k)| (k, w))
                .collect();
            c.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
            let subjects: Vec<String> = c
                .into_iter()
                .take(4)
                .map(|(_, w)| surface_word(&text, &w))
                .collect();
            let mut who: Vec<&str> = Vec::new();
            for t in &all[a..=b] {
                let f = first_name(&t.speaker);
                if !who.contains(&f) {
                    who.push(f);
                }
            }
            let when = span(all[a].at, all[b].at);
            md.push_str(&format!(
                "- {}{} · {}\n",
                if when.is_empty() {
                    String::new()
                } else {
                    format!("**{when}** · ")
                },
                subjects.join(", "),
                who.join(", ")
            ));
        }
    }
    let mut buzz: HashMap<&str, usize> = HashMap::new();
    for t in &all {
        for b in buzzwords_in(&t.text) {
            *buzz.entry(b).or_default() += 1;
        }
    }
    if !buzz.is_empty() {
        let mut b: Vec<(&str, usize)> = buzz.into_iter().collect();
        b.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(y.0)));
        md.push_str(&format!(
            "\n### Buzzwords · {buzz_share}% of the meeting was mostly buzzwords\n"
        ));
        md.push_str(
            &b.iter()
                .take(12)
                .map(|(w, n)| {
                    if *n > 1 {
                        format!("`{w}` ×{n}")
                    } else {
                        format!("`{w}`")
                    }
                })
                .collect::<Vec<_>>()
                .join(" · "),
        );
        md.push('\n');
    }
    md.push_str("\n### Who spoke\n");
    let everyone = nlp::count(&words(
        &all.iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    ));
    for (name, n, turns_n) in &speakers {
        let pct = n * 100 / total_words.max(1);
        let theirs: String = all
            .iter()
            .zip(&kinds)
            .filter(|(t, k)| &t.speaker == name && **k == Kind::Work)
            .map(|(t, _)| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        // Words this person used more than the others did, in their on-topic talk.
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
        let said = if top.is_empty() {
            String::new()
        } else {
            format!(" · _{}_", top.join(", "))
        };
        md.push_str(&format!(
            "- **{name}** · {pct}% of the words, {}{said}\n",
            plural(*turns_n, "turn")
        ));
    }

    // Front matter.
    let labels: Vec<String> = topics.iter().map(|t| t.label.clone()).collect();
    let topic_line = details.topic.clone().unwrap_or_else(|| {
        let mut l = labels.join(", ");
        if let Some(f) = l.get(..1) {
            l = f.to_uppercase() + &l[1..].to_lowercase();
        }
        l
    });
    let what = if decisions.is_empty() && actions.is_empty() && questions.is_empty() {
        "no decisions, action items or open questions".to_string()
    } else {
        format!(
            "{}, {} and {}",
            plural(decisions.len(), "decision"),
            plural(actions.len(), "action item"),
            plural(questions.len(), "open question")
        )
    };
    let head = details.title.clone().unwrap_or_else(|| "Meeting".into());
    let len = length.clone().map(|l| format!("{l}, ")).unwrap_or_default();
    let subject = if labels.is_empty() {
        String::new()
    } else {
        format!(
            " on {}",
            labels
                .iter()
                .map(|l| l.to_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let description = format!(
        "{head} ({len}{}){subject}: {what}; {on_topic}% on topic.",
        plural(speakers.len(), "attendee")
    );
    let text = |v: Option<String>| crate::Meta::Text(v.unwrap_or_default());
    let meta = vec![
        ("date".to_string(), text(details.date.clone())),
        ("time".to_string(), text(details.time.clone())),
        ("duration".to_string(), text(length)),
        ("place".to_string(), text(details.place.clone())),
        ("topic".to_string(), text(Some(topic_line))),
        (
            "attendees".to_string(),
            crate::Meta::List(details.attendees.clone()),
        ),
    ];

    // The full conversation, in order, marked where each subject and each digression starts.
    let section_of = |i: usize| -> String {
        match kinds[i] {
            Kind::SmallTalk => "Off topic".to_string(),
            _ => work_at
                .iter()
                .rposition(|&w| w <= i)
                .and_then(topic_of)
                .or(if topics.is_empty() { None } else { Some(0) })
                .map(|k| topics[k].label.clone())
                .unwrap_or_else(|| "Conversation".into()),
        }
    };
    let mut body = String::new();
    let mut current = String::new();
    for (i, turn) in all.iter().enumerate() {
        let section = section_of(i);
        if section != current {
            let end = (i..all.len())
                .take_while(|&j| section_of(j) == section)
                .last()
                .unwrap_or(i);
            let when = span(all[i].at, all[end].at);
            let when = if when.is_empty() {
                String::new()
            } else {
                format!(" ({when})")
            };
            body.push_str(&format!("### {section}{when}\n\n"));
            current = section;
        }
        let at = turn
            .at
            .map(|a| format!(" ({})", stamp(a)))
            .unwrap_or_default();
        body.push_str(&format!(
            "**{}**{at}: {}\n\n",
            turn.speaker,
            turn.text.trim()
        ));
    }
    Summary {
        markdown: md,
        description,
        body,
        notes,
        title: details.title,
        meta,
    }
}

/// "Jordan will model the numbers" when the task is Lee's: a small model sometimes mixes up
/// names. A "<Name> will ..." sentence that clearly matches a known action item gets that
/// item's owner.
fn fix_owners(text: &str, owners: &[(String, String)]) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| re(r"\b([A-Z][a-z]+) (will|is going to|agreed to|is to) ([^.]+)"));
    r.replace_all(text, |c: &regex::Captures| {
        let said: HashSet<String> = words(&c[3]).into_iter().collect();
        let best = owners
            .iter()
            .map(|(who, task)| {
                let shared = words(task).into_iter().filter(|w| said.contains(w)).count();
                (shared, who)
            })
            .max_by_key(|(shared, _)| *shared);
        match best {
            Some((shared, who)) if shared >= 2 && who != &c[1] => {
                format!("{who} {} {}", &c[2], &c[3])
            }
            _ => c[0].to_string(),
        }
    })
    .to_string()
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
            ..Default::default()
        }
    }

    #[test]
    fn finds_decisions_action_items_and_open_questions() {
        let s = summarize(&meeting(), None);
        let md = &s.markdown;
        for expected in [
            "## Goal\n\n“Okay so, the main thing today is the pet hatching update.",
            "## Outcome\n\n**1 decision · 3 action items · 1 open question · 100% on topic**",
            "### Decisions\n- “Let's go with Saturday then",
            "- **Jordan**: “I'll have it done by Friday.” · due Friday",
            "- **Maya**: “I'll write the patch notes.”",
            // A request and its owner's reply are one item.
            "- **Lee**: “Lee, can you put together the numbers for next week?” → “Sure, I'll bring a spreadsheet to Tuesday's meeting.” · due next week",
            "### Open questions\n- “Do rare pets keep their stats after trading?” · Sam · 2:10",
            "## Summary\n\n### ",
            "## Miscellaneous\n\n### Who spoke\n- **Maya**",
        ] {
            assert!(md.contains(expected), "missing {expected:?} in:\n{md}");
        }
        // Answered questions and tag questions are not open; nothing here is off topic.
        assert!(!md.contains("How long would that take"), "{md}");
        assert!(!md.contains("ship Saturday, right?"), "{md}");
        assert!(!md.contains("### Off topic"), "{md}");
        assert!(
            s.description
                .starts_with("Meeting (2 min, 4 attendees) on "),
            "{}",
            s.description
        );
        assert!(
            s.description
                .ends_with(": 1 decision, 3 action items and 1 open question; 100% on topic."),
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
    fn small_talk_and_buzzwords_are_kept_apart_from_the_work() {
        let text = "\
[0:00:03] Priya: Did anyone catch the match last night? What a finish.
[0:00:09] Tom: Unreal, two goals in stoppage time. I was yelling at the TV.
[0:00:18] Dana: I missed it, I was stuck in traffic for an hour.
[0:00:30] Priya: Okay. The goal today is to agree the Q3 roadmap priorities.
[0:00:40] Tom: We need to leverage our synergies and align on a holistic north star.
[0:00:52] Priya: We have three candidates for Q3: dark mode, the guild system and trading.
[0:01:01] Dana: The guild system came up in eighty percent of the survey answers.
[0:01:10] Priya: We decided to drop dark mode and put the guild system first.
[0:01:18] Dana: Agreed. Trading moves to Q4.
[0:01:30] Tom: Are we doing the team lunch on Friday? The pizza place has good reviews.
[0:01:38] Dana: Fine, pizza it is. I'll book a table.
";
        let t = Transcript {
            utterances: from_text(text),
            ..Default::default()
        };
        let s = summarize(&t, None);
        let md = &s.markdown;
        assert!(
            md.contains("“The goal today is to agree the Q3 roadmap priorities.” · Priya · 0:30"),
            "{md}"
        );
        assert!(
            md.contains("- “We decided to drop dark mode and put the guild system first.” · Priya"),
            "{md}"
        );
        assert!(
            md.contains("- “Trading moves to Q4.” · Dana"),
            "“Agreed.” marks the decision after it: {md}"
        );
        // The football, the traffic and the lunch booking are not work.
        assert!(
            !md.contains("match last night") && !md.contains("book a table"),
            "{md}"
        );
        assert!(md.contains("### Off topic"), "{md}");
        assert!(md.contains("- **0:03–0:18** · "), "{md}");
        assert!(md.contains("- **1:30–1:38** · "), "{md}");
        assert!(
            md.contains("### Buzzwords · ") && md.contains("`leverage`"),
            "{md}"
        );
        // The transcript keeps every turn, marking the digressions.
        assert!(
            s.body.starts_with("### Off topic (0:03–0:18)"),
            "{}",
            s.body
        );
    }

    #[test]
    fn a_meeting_about_nothing_says_so() {
        let text = "\
[0:00:02] Alex: Hey all, how was everyone's weekend?
[0:00:07] Jamie: Good, went hiking with the dog, then it rained all Sunday.
[0:00:24] Alex: Just wanted to touch base and make sure we're all aligned on the big picture going forward.
[0:00:33] Jamie: Totally. We need to leverage our core competencies.
[0:00:42] Riley: Agreed. We just need to stay agile and keep the synergies flowing across the ecosystem.
[0:01:15] Alex: Love that. Let's take this offline and circle back next week.
";
        let t = Transcript {
            utterances: from_text(text),
            ..Default::default()
        };
        let s = summarize(&t, None);
        let md = &s.markdown;
        assert!(
            md.contains("_Stated only in general terms: no concrete subject._"),
            "{md}"
        );
        assert!(
            md.contains("**No decisions, action items or open questions · 0% on topic**"),
            "{md}"
        );
        assert!(
            md.contains("Most of the meeting (100%) was small talk or buzzwords."),
            "{md}"
        );
        assert!(
            md.contains("_No on-topic discussion to summarize._"),
            "{md}"
        );
        assert!(
            s.description
                .contains("no decisions, action items or open questions; 0% on topic"),
            "{}",
            s.description
        );
    }

    #[test]
    fn a_task_keeps_its_owner_in_model_text() {
        let owners = vec![
            (
                "Lee".to_string(),
                "Lee, can you model the numbers for a guarantee at forty versus fifty hatches?"
                    .to_string(),
            ),
            (
                "Sam".to_string(),
                "I'll post it on the forum tomorrow.".to_string(),
            ),
        ];
        assert_eq!(
            fix_owners(
                "Jordan will model the numbers for a guarantee on Tuesday. Sam will post it on the forum.",
                &owners
            ),
            "Lee will model the numbers for a guarantee on Tuesday. Sam will post it on the forum."
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
            s.markdown.contains("Written by a language model (Echo)"),
            "{}",
            s.markdown
        );
        assert!(
            s.markdown.contains("\n\n[") && !s.markdown.contains("### Overview"),
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
