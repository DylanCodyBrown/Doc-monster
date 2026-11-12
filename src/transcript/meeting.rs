//! A meeting's details for the front matter: title, date, time, place, topic and attendees,
//! read from the lines above the conversation (Teams and Zoom write them there), from the
//! file name (Zoom recordings are named after their start time) and from the speakers.

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Details {
    pub title: Option<String>,
    /// YYYY-MM-DD.
    pub date: Option<String>,
    /// HH:MM, 24-hour; " UTC" added when the source says so.
    pub time: Option<String>,
    pub place: Option<String>,
    /// As stated ("Topic: ..."), when it is.
    pub topic: Option<String>,
    /// Stated attendees first, then everyone who spoke.
    pub attendees: Vec<String>,
    /// Agenda or goal lines written above the conversation ("Agenda: ...").
    pub agenda: Option<String>,
}

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap()
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn month(name: &str) -> Option<u32> {
    let n = name.to_lowercase();
    MONTHS
        .iter()
        .position(|m| n.starts_with(m))
        .map(|i| i as u32 + 1)
}

/// A date anywhere in the text, as YYYY-MM-DD.
pub fn find_date(text: &str) -> Option<String> {
    static ISO: OnceLock<Regex> = OnceLock::new();
    static WORDS: OnceLock<Regex> = OnceLock::new();
    static DAY_FIRST: OnceLock<Regex> = OnceLock::new();
    let iso = ISO.get_or_init(|| re(r"\b(20\d\d)-(\d{2})-(\d{2})\b"));
    if let Some(c) = iso.captures(text) {
        return Some(format!("{}-{}-{}", &c[1], &c[2], &c[3]));
    }
    // "November 10, 2026" / "Nov 10 2026"
    let words = WORDS.get_or_init(|| {
        re(r"(?i)\b(jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\.?\s+(\d{1,2})(?:st|nd|rd|th)?,?\s+(20\d\d)\b")
    });
    if let Some(c) = words.captures(text) {
        let d: u32 = c[2].parse().ok()?;
        return Some(format!("{}-{:02}-{d:02}", &c[3], month(&c[1])?));
    }
    // "10 November 2026"
    let day_first = DAY_FIRST.get_or_init(|| {
        re(r"(?i)\b(\d{1,2})(?:st|nd|rd|th)?\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\.?,?\s+(20\d\d)\b")
    });
    let c = day_first.captures(text)?;
    let d: u32 = c[1].parse().ok()?;
    Some(format!("{}-{:02}-{d:02}", &c[3], month(&c[2])?))
}

/// A clock time ("9:30 AM", "14:05") anywhere in the text, as HH:MM.
pub fn find_time(text: &str) -> Option<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| {
        re(r"(?i)\b(\d{1,2}):(\d{2})(?::\d{2})?\s*(am|pm|a\.m\.|p\.m\.)?(?:\s|$|,)")
    });
    let c = r.captures(text)?;
    let mut h: u32 = c[1].parse().ok()?;
    let m: u32 = c[2].parse().ok()?;
    match c.get(3).map(|x| x.as_str().to_lowercase()) {
        Some(s) if s.starts_with('p') && h < 12 => h += 12,
        Some(s) if s.starts_with('a') && h == 12 => h = 0,
        _ => {}
    }
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

fn labelled(line: &str, labels: &[&str]) -> Option<String> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim().trim_matches('*').trim().to_lowercase();
    labels
        .contains(&key.as_str())
        .then(|| value.trim().trim_matches('*').trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Read the details. `header`: lines above the conversation; `file_name`: without folders;
/// `teams`: the captions carried Teams' speaker tags; `speakers`: everyone who spoke, in order.
pub fn details(header: &[String], file_name: &str, teams: bool, speakers: &[String]) -> Details {
    let mut d = Details::default();
    for line in header {
        let l = line.trim().trim_start_matches('#').trim();
        if l.is_empty() || l.eq_ignore_ascii_case("WEBVTT") {
            continue;
        }
        if let Some(v) = labelled(l, &["location", "place", "room", "where", "venue"]) {
            d.place.get_or_insert(v);
        } else if let Some(v) = labelled(l, &["topic", "subject", "re"]) {
            d.topic.get_or_insert(v);
        } else if let Some(v) = labelled(
            l,
            &[
                "agenda",
                "goal",
                "goals",
                "purpose",
                "objective",
                "objectives",
            ],
        ) {
            d.agenda.get_or_insert(v);
        } else if let Some(v) = labelled(l, &["attendees", "participants", "present", "invitees"]) {
            for a in v.split([',', ';']) {
                let a = a.trim().to_string();
                if !a.is_empty() && !d.attendees.contains(&a) {
                    d.attendees.push(a);
                }
            }
        } else if let Some(v) = labelled(l, &["title", "meeting"]) {
            d.title.get_or_insert(v);
        } else if let Some(date) = find_date(l) {
            d.date.get_or_insert(date);
            if let Some(t) = find_time(l) {
                d.time.get_or_insert(t);
            }
        } else if l.to_lowercase().contains("started transcription") {
            d.place.get_or_insert("Microsoft Teams".into());
        } else if d.title.is_none()
            && l.chars().count() <= 80
            && l.chars().any(char::is_alphabetic)
            && !l.contains("-->")
            && find_time(l).is_none()
        {
            d.title = Some(l.to_string());
        }
    }
    // Zoom names recordings after their start: GMT20261110-093012_Recording...
    static ZOOM: OnceLock<Regex> = OnceLock::new();
    let zoom = ZOOM.get_or_init(|| re(r"GMT(20\d\d)(\d\d)(\d\d)-(\d\d)(\d\d)\d\d"));
    if let Some(c) = zoom.captures(file_name) {
        d.date
            .get_or_insert(format!("{}-{}-{}", &c[1], &c[2], &c[3]));
        d.time.get_or_insert(format!("{}:{} UTC", &c[4], &c[5]));
        d.place.get_or_insert("Zoom".into());
    } else if d.date.is_none() {
        d.date = find_date(&file_name.replace(['_', '.'], " "));
    }
    if teams {
        d.place.get_or_insert("Microsoft Teams".into());
    }
    for s in speakers {
        if s != "Unknown speaker" && !d.attendees.iter().any(|a| a.eq_ignore_ascii_case(s)) {
            d.attendees.push(s.clone());
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(String::from).collect()
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(
            find_date("November 10, 2026, 9:30AM").as_deref(),
            Some("2026-11-10")
        );
        assert_eq!(find_date("10 Nov 2026").as_deref(), Some("2026-11-10"));
        assert_eq!(find_date("2026-11-10 notes").as_deref(), Some("2026-11-10"));
        assert_eq!(
            find_time("November 10, 2026, 9:30 AM").as_deref(),
            Some("09:30")
        );
        assert_eq!(find_time("at 2:15 pm").as_deref(), Some("14:15"));
    }

    #[test]
    fn header_lines_file_names_and_speakers() {
        let d = details(
            &lines(
                "Aurora weekly sync\nNovember 10, 2026, 9:30 AM\nLocation: Room 4B\nAttendees: Maya Lopez, Priya Shah",
            ),
            "sync.txt",
            false,
            &["Maya Lopez".into(), "Jordan Lee".into()],
        );
        assert_eq!(d.title.as_deref(), Some("Aurora weekly sync"));
        assert_eq!(d.date.as_deref(), Some("2026-11-10"));
        assert_eq!(d.time.as_deref(), Some("09:30"));
        assert_eq!(d.place.as_deref(), Some("Room 4B"));
        assert_eq!(d.attendees, vec!["Maya Lopez", "Priya Shah", "Jordan Lee"]);
        let z = details(
            &[],
            "GMT20261110-093012_Recording.transcript.vtt",
            false,
            &[],
        );
        assert_eq!(
            (z.date.as_deref(), z.time.as_deref(), z.place.as_deref()),
            (Some("2026-11-10"), Some("09:30 UTC"), Some("Zoom"))
        );
        let t = details(&[], "standup.vtt", true, &[]);
        assert_eq!(t.place.as_deref(), Some("Microsoft Teams"));
    }
}
