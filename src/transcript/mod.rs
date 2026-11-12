//! Transcripts: meetings and conversations as "who said what, when".
//!
//! Reads Teams and Zoom captions (.vtt), Teams .docx transcripts (after the Word reader has
//! turned them into text) and plain text with "Name: ..." lines, then summarises them by topic,
//! with decisions, action items, open questions and who said what (see `analyze`).

pub mod analyze;

use regex::Regex;
use std::sync::OnceLock;

/// One turn in the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    /// Seconds from the start, when the transcript has times.
    pub at: Option<u32>,
    pub speaker: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Transcript {
    pub utterances: Vec<Utterance>,
}

/// "1:02:03", "02:03", "0:03", "00:01:02.500", "0:0:2.30": seconds.
pub fn parse_time(s: &str) -> Option<u32> {
    let s = s
        .trim()
        .trim_matches(|c| matches!(c, '[' | ']' | '(' | ')'));
    let s = s.split(['.', ',']).next()?;
    let parts: Vec<u32> = s
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    match parts[..] {
        [m, sec] if sec < 60 => Some(m * 60 + sec),
        [h, m, sec] if m < 60 && sec < 60 => Some(h * 3600 + m * 60 + sec),
        _ => None,
    }
}

/// Words that start a "Label:" line in ordinary documents and are never speakers.
const NOT_SPEAKERS: &[&str] = &[
    "note",
    "notes",
    "warning",
    "tip",
    "example",
    "important",
    "caution",
    "summary",
    "step",
    "source",
    "title",
    "description",
    "date",
    "time",
    "location",
    "attendees",
    "agenda",
    "subject",
    "re",
    "to",
    "from",
    "cc",
    "bcc",
    "update",
    "result",
    "results",
    "answer",
    "question",
    "status",
    "goal",
    "goals",
    "todo",
    "fix",
    "output",
    "input",
    "error",
    "http",
    "https",
    "file",
    "files",
    "see",
    "also",
    "author",
    "version",
    "license",
];

fn speaker_ok(name: &str) -> bool {
    let n = name.trim();
    let words = n.split_whitespace().count();
    (1..=4).contains(&words)
        && n.chars().count() <= 40
        && n.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() && !c.is_lowercase())
        && !n.contains(['*', '#', '`', '[', ']', '|', '/', '='])
        && !NOT_SPEAKERS.contains(&n.to_lowercase().as_str())
}

fn line_speaker() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // [0:01:23] Name: text   |   0:01:23 Name: text   |   Name (0:01:23): text   |   Name: text
    R.get_or_init(|| {
        Regex::new(
            r"^\s*(?:\*\*)?(?:[\[(]?(?P<t1>\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d+)?)[\])]?\s+)?(?P<name>[^:\d\s][^:]{0,40}?)(?:\s*[\[(](?P<t2>\d{1,2}:\d{2}(?::\d{2})?)[\])])?(?:\*\*)?:(?:\*\*)?\s+(?P<text>\S.*)$",
        )
        .unwrap()
    })
}

fn header_speaker() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // Teams .docx: a line with the name and a time, the words on the following lines.
    R.get_or_init(|| {
        Regex::new(r"^\s*(?:\*\*)?(?P<name>[^\d:][^:]{0,40}?)(?:\*\*)?\s{1,}(?P<t>\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d+)?)\s*$")
            .unwrap()
    })
}

fn vtt_voice() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"<v(?:\.[\w.]+)?\s+([^>]+)>").unwrap())
}

fn strip_tags(s: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"</?[^>]+>").unwrap())
        .replace_all(s, "")
        .trim()
        .to_string()
}

/// Captions (.vtt, .srt): Teams marks speakers with <v Name>, Zoom writes "Name: text".
pub fn from_captions(text: &str) -> Vec<Utterance> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<Utterance> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((start, _)) = lines[i].split_once("-->") else {
            i += 1;
            continue;
        };
        let at = parse_time(start.split_whitespace().last().unwrap_or(""));
        i += 1;
        let mut body: Vec<&str> = Vec::new();
        while i < lines.len() && !lines[i].trim().is_empty() {
            body.push(lines[i]);
            i += 1;
        }
        let raw = body.join(" ");
        let (speaker, said) = if let Some(c) = vtt_voice().captures(&raw) {
            (c[1].trim().to_string(), strip_tags(&raw))
        } else {
            let plain = strip_tags(&raw);
            match line_speaker().captures(&plain) {
                Some(c) if speaker_ok(&c["name"]) => {
                    (c["name"].trim().to_string(), c["text"].trim().to_string())
                }
                _ => (String::new(), plain),
            }
        };
        if !said.is_empty() {
            out.push(Utterance {
                at,
                speaker,
                text: said,
            });
        }
    }
    out
}

/// Text (plain, Markdown, or a Word transcript turned into text).
pub fn from_text(text: &str) -> Vec<Utterance> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.trim().is_empty())
        .collect();
    let mut out: Vec<Utterance> = Vec::new();
    // Which turns came from "Name: text" lines (the only kind a stray colon can fake).
    let mut from_colon: Vec<bool> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if let Some(c) = line_speaker().captures(line)
            && speaker_ok(&c["name"])
        {
            let at = c
                .name("t1")
                .or(c.name("t2"))
                .and_then(|t| parse_time(t.as_str()));
            out.push(Utterance {
                at,
                speaker: c["name"].trim().to_string(),
                text: c["text"].trim().to_string(),
            });
            from_colon.push(true);
        } else if let Some(c) = header_speaker().captures(line)
            && speaker_ok(&c["name"])
        {
            // The name and time on one line, then the words until the next speaker.
            let mut said = Vec::new();
            while i + 1 < lines.len() {
                let next = lines[i + 1].trim();
                if header_speaker()
                    .captures(next)
                    .is_some_and(|n| speaker_ok(&n["name"]))
                    || line_speaker()
                        .captures(next)
                        .is_some_and(|n| speaker_ok(&n["name"]))
                {
                    break;
                }
                said.push(next);
                i += 1;
            }
            if !said.is_empty() {
                out.push(Utterance {
                    at: parse_time(&c["t"]),
                    speaker: c["name"].trim().to_string(),
                    text: said.join(" "),
                });
                from_colon.push(false);
            }
        } else if let Some(last) = out.last_mut() {
            // A continuation of the last speaker's words.
            last.text.push(' ');
            last.text.push_str(line);
        }
        i += 1;
    }
    fold_one_off_speakers(out, &from_colon)
}

/// In a conversation with regular speakers, a "speaker" heard only once is usually a sentence
/// with a colon in it ("So the plan: ship Saturday"), so it rejoins the turn before it.
fn fold_one_off_speakers(turns: Vec<Utterance>, from_colon: &[bool]) -> Vec<Utterance> {
    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    for u in &turns {
        *counts.entry(u.speaker.clone()).or_default() += 1;
    }
    if counts.values().filter(|&&n| n >= 2).count() < 2 {
        return turns;
    }
    let mut out: Vec<Utterance> = Vec::new();
    for (u, &colon) in turns.into_iter().zip(from_colon) {
        match out.last_mut() {
            Some(prev)
                if colon
                    && counts[&u.speaker] == 1
                    && u.speaker.split_whitespace().count() >= 2 =>
            {
                prev.text.push(' ');
                prev.text.push_str(&format!("{}: {}", u.speaker, u.text));
            }
            _ => out.push(u),
        }
    }
    out
}

/// True when the turns read like a conversation: enough turns, at least two speakers who each
/// speak more than once, and the speaker lines make up a good share of the text.
pub fn looks_like_conversation(turns: &[Utterance], text_lines: usize) -> bool {
    let named: Vec<&Utterance> = turns.iter().filter(|u| !u.speaker.is_empty()).collect();
    let mut counts: std::collections::HashMap<&str, usize> = Default::default();
    for u in &named {
        *counts.entry(u.speaker.as_str()).or_default() += 1;
    }
    let regulars = counts.values().filter(|&&n| n >= 2).count();
    named.len() >= 6 && regulars >= 2 && named.len() * 4 >= text_lines.max(1)
}

/// Read `text` as a transcript. `captions` for .vtt/.srt. With `force`, any text with speaker
/// lines counts, even a short one; without it, only text that reads like a conversation.
pub fn detect(text: &str, captions: bool, force: bool) -> Option<Transcript> {
    let utterances = if captions {
        from_captions(text)
    } else {
        from_text(text)
    };
    let lines = if captions {
        utterances.len()
    } else {
        text.lines().filter(|l| !l.trim().is_empty()).count()
    };
    let named = utterances.iter().filter(|u| !u.speaker.is_empty()).count();
    let ok = if force {
        named >= 1
    } else {
        looks_like_conversation(&utterances, lines)
    };
    ok.then_some(Transcript { utterances })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("1:02:03"), Some(3723));
        assert_eq!(parse_time("02:03"), Some(123));
        assert_eq!(parse_time("00:01:02.500"), Some(62));
        assert_eq!(parse_time("[0:41:10]"), Some(2470));
        assert_eq!(parse_time("12:75"), None);
    }

    #[test]
    fn teams_and_zoom_captions() {
        let teams = "WEBVTT\n\n0f2a/12-0\n00:00:01.000 --> 00:00:04.000\n<v Maya Lopez>Okay, the main thing today.</v>\n\n00:00:05.000 --> 00:00:07.000\n<v Jordan Lee>I looked at it.</v>\n";
        let t = from_captions(teams);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].speaker.as_str(), t[0].at), ("Maya Lopez", Some(1)));
        assert_eq!(t[1].text, "I looked at it.");
        let zoom =
            "WEBVTT\n\n1\n00:01:05.000 --> 00:01:08.000\nSam Park: Can we still ship Saturday?\n";
        let z = from_captions(zoom);
        assert_eq!((z[0].speaker.as_str(), z[0].at), ("Sam Park", Some(65)));
        assert_eq!(z[0].text, "Can we still ship Saturday?");
    }

    #[test]
    fn name_lines_headers_and_continuations() {
        let text = "Weekly sync\n\n[00:00:05] Maya: Okay so the main thing today.\nIt is the pet update.\nJordan (0:12): I looked at it.\nMaya: Thanks.\n";
        let t = from_text(text);
        assert_eq!(t.len(), 3);
        assert_eq!(
            t[0].text,
            "Okay so the main thing today. It is the pet update."
        );
        assert_eq!((t[1].speaker.as_str(), t[1].at), ("Jordan", Some(12)));
        let teams_docx = "Maya Lopez   0:03\nOkay so the main thing today.\nJordan Lee   0:12\nI looked at it.\nSo did I.\n";
        let d = from_text(teams_docx);
        assert_eq!(d.len(), 2);
        assert_eq!((d[1].speaker.as_str(), d[1].at), ("Jordan Lee", Some(12)));
        assert_eq!(d[1].text, "I looked at it. So did I.");
        let colon = "Maya: hi\nJordan: hello\nMaya: the update\nSo the plan: ship Saturday\nJordan: Friday\n";
        let c = from_text(colon);
        assert_eq!(c.len(), 4, "{c:?}");
        assert_eq!(c[2].text, "the update So the plan: ship Saturday");
    }

    #[test]
    fn ordinary_documents_are_not_transcripts() {
        let doc = "Note: back up first.\nWarning: this deletes files.\nStep 1: open the app.\nTip: use the menu.\nExample: run it twice.\nNote: done.\n";
        assert!(detect(doc, false, false).is_none());
        let chat =
            "Maya: hi\nJordan: hello\nMaya: the update\nJordan: Friday\nMaya: ok\nJordan: done\n";
        assert!(detect(chat, false, false).is_some());
    }
}
