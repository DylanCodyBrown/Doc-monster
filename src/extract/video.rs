//! Video files of any common container (mp4, mov, mkv, webm, avi, wmv, flv, mpeg, ...).
//!
//! A video is scored like a piece of music. Each instrument is a track of timed events, played
//! on its own thread at the same time as the others; the bars (two-minute sections) and the
//! timestamps then line them up into one timeline:
//! - chapters, from the container (ffprobe)
//! - spoken, from a subtitle file beside the video or a subtitle track inside it (ffmpeg)
//! - on screen, the text shown on screen (ffmpeg frames read by Tesseract, see `screen`)
//!
//! FFmpeg and Tesseract are required. Speech is not transcribed; without subtitles that is said.

use super::{Out, screen, subtitles};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;

fn clock(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn probe(path: &Path) -> Option<Value> {
    let out = crate::tools::command("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_chapters",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| serde_json::from_slice(&out.stdout).ok())
        .flatten()
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|x| !x.is_empty())
}

fn tag<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    let tags = v.get("tags")?;
    // Tag names differ in case between containers.
    tags.as_object()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, v)| v.as_str())
        .filter(|x| !x.is_empty())
}

fn fps(rate: &str) -> Option<f64> {
    let (n, d) = rate.split_once('/')?;
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (d > 0.0 && n > 0.0).then(|| n / d)
}

struct Details {
    markdown: String,
    title: Option<String>,
    description: Option<String>,
    has_text_subtitles: bool,
    /// Plain technical summary, the last-resort description.
    tech: String,
}

fn from_probe(p: &Value, fallback: &str, size: u64, ext: &str) -> Details {
    let fmt = p.get("format").cloned().unwrap_or(Value::Null);
    let secs = s(&fmt, "duration")
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;
    let container = if ext.is_empty() {
        s(&fmt, "format_name")
            .map(|n| n.split(',').next().unwrap_or(n))
            .unwrap_or("unknown")
    } else {
        ext
    };
    let title = tag(&fmt, "title").map(str::to_string);
    let name = title.as_deref().unwrap_or(fallback);
    let mut md = format!(
        "{name} is a video file ({container}, {}, {:.1} megabytes).\n\n",
        clock(secs),
        size as f64 / 1_048_576.0
    );
    let dims = p
        .get("streams")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|st| s(st, "codec_type") == Some("video")))
        .and_then(|st| {
            Some(format!(
                ", {}x{}",
                st.get("width")?.as_u64()?,
                st.get("height")?.as_u64()?
            ))
        })
        .unwrap_or_default();
    let tech = format!("{container} video, {}{dims}.", clock(secs));
    let mut has_text_subtitles = false;
    md.push_str("| Track | Kind | Details |\n| --- | --- | --- |\n");
    for st in p
        .get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = s(st, "codec_type").unwrap_or("other");
        let codec = s(st, "codec_name").unwrap_or("unknown codec");
        let lang = tag(st, "language").filter(|l| *l != "und");
        let kbps = s(st, "bit_rate")
            .and_then(|b| b.parse::<u64>().ok())
            .filter(|b| *b > 0)
            .map(|b| format!(", {} kilobits per second", b / 1000));
        let detail = match kind {
            "video" => {
                let dims = match (
                    st.get("width").and_then(Value::as_u64),
                    st.get("height").and_then(Value::as_u64),
                ) {
                    (Some(w), Some(h)) => format!(" {w}x{h}"),
                    _ => String::new(),
                };
                let rate = s(st, "avg_frame_rate")
                    .and_then(fps)
                    .map(|f| format!(" at {f:.2} frames per second"))
                    .unwrap_or_default();
                format!("{codec}{dims}{rate}{}", kbps.unwrap_or_default())
            }
            "audio" => {
                let ch = st
                    .get("channels")
                    .and_then(Value::as_u64)
                    .map(|c| format!(", {c} channel{}", if c == 1 { "" } else { "s" }))
                    .unwrap_or_default();
                let hz = s(st, "sample_rate")
                    .map(|r| format!(", {r} hertz"))
                    .unwrap_or_default();
                format!(
                    "{codec}{ch}{hz}{}{}",
                    kbps.unwrap_or_default(),
                    lang.map(|l| format!(", language {l}")).unwrap_or_default()
                )
            }
            "subtitle" => {
                has_text_subtitles |= matches!(
                    codec,
                    "subrip" | "srt" | "ass" | "ssa" | "mov_text" | "webvtt" | "text"
                );
                format!(
                    "{codec}{}",
                    lang.map(|l| format!(", language {l}")).unwrap_or_default()
                )
            }
            _ => codec.to_string(),
        };
        md.push_str(&format!(
            "| {} | {kind} | {detail} |\n",
            st.get("index").and_then(Value::as_u64).unwrap_or(0)
        ));
    }
    md.push('\n');

    let mut meta = Vec::new();
    for key in [
        "artist",
        "album",
        "date",
        "genre",
        "comment",
        "description",
        "synopsis",
    ] {
        if let Some(v) = tag(&fmt, key) {
            meta.push(format!(
                "- {}: {}",
                key[..1].to_uppercase() + &key[1..],
                v.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
        }
    }
    if !meta.is_empty() {
        md.push_str(&format!("## About\n\n{}\n\n", meta.join("\n")));
    }
    let description = ["description", "synopsis", "comment"]
        .iter()
        .find_map(|k| tag(&fmt, k))
        .map(str::to_string);
    Details {
        markdown: md,
        title,
        description,
        has_text_subtitles,
        tech,
    }
}

fn embedded_subtitles(path: &Path) -> Option<String> {
    let out = crate::tools::command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:s:0", "-f", "srt", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    (out.status.success() && text.contains("-->")).then_some(text)
}

/// One timed event on a track.
enum Part<'a> {
    Chapter(&'a str),
    Spoken(&'a str),
    Shown(&'a screen::Screen),
}

impl Part<'_> {
    /// Order within the same second: a chapter opens, then what is shown, then what is said.
    fn rank(&self) -> u8 {
        match self {
            Part::Chapter(_) => 0,
            Part::Shown(_) => 1,
            Part::Spoken(_) => 2,
        }
    }
}

/// Bars of the score: two-minute sections.
const BAR: u32 = 120;

/// Lay every track onto one timeline, bar by bar, in time order.
fn assemble(notes: &mut [(u32, Part)]) -> String {
    notes.sort_by_key(|(t, p)| (*t, p.rank()));
    let mut md = String::new();
    if notes.is_empty() {
        return md;
    }
    md.push_str("## Timeline\n\n");
    let mut bar = u32::MAX;
    let mut said: Vec<&str> = Vec::new();
    let flush = |said: &mut Vec<&str>, md: &mut String| {
        if !said.is_empty() {
            md.push_str(&format!("**Spoken**\n\n{}\n\n", said.join(" ")));
            said.clear();
        }
    };
    for (t, part) in notes.iter() {
        if t / BAR != bar {
            flush(&mut said, &mut md);
            bar = t / BAR;
            md.push_str(&format!(
                "### {} – {}\n\n",
                subtitles::stamp(bar * BAR),
                subtitles::stamp((bar + 1) * BAR)
            ));
        }
        match part {
            Part::Spoken(l) => said.push(l),
            Part::Chapter(title) => {
                flush(&mut said, &mut md);
                md.push_str(&format!(
                    "**Chapter at {}: {title}**\n\n",
                    subtitles::stamp(*t)
                ));
            }
            Part::Shown(sc) => {
                flush(&mut said, &mut md);
                md.push_str(&format!(
                    "**On screen at {}**\n\n{}\n\n",
                    subtitles::stamp(sc.at),
                    sc.block
                ));
            }
        }
    }
    flush(&mut said, &mut md);
    md
}

/// The chapter track, from the container's chapter list.
fn chapters(p: &Value) -> Vec<(u32, String)> {
    p.get("chapters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            let start = s(c, "start_time")
                .and_then(|t| t.parse::<f64>().ok())
                .unwrap_or(0.0) as u32;
            (
                start,
                tag(c, "title").unwrap_or("Untitled chapter").to_string(),
            )
        })
        .collect()
}

/// The spoken track: a subtitle file beside the video, or a subtitle track inside it.
fn spoken(path: &Path, embedded: bool) -> Result<Vec<(u32, String)>> {
    let sidecar = ["srt", "vtt", "en.srt", "en.vtt"]
        .iter()
        .map(|ext| path.with_extension(ext))
        .find(|p| p.exists());
    let text = match sidecar {
        Some(p) => Some(super::text::decode(
            &std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?,
        )),
        None if embedded => embedded_subtitles(path),
        None => None,
    };
    Ok(text.as_deref().map(subtitles::cues).unwrap_or_default())
}

pub fn convert(path: &Path, fallback: &str, opts: &super::Options) -> Result<Out> {
    let missing = screen::missing_tools();
    if !missing.is_empty() {
        bail!(
            "videos need {} installed (see INSTALL.md)",
            missing.join(" and ")
        );
    }
    let size = std::fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let Some(probe) = probe(path) else {
        bail!("not a readable video file")
    };
    let details = from_probe(&probe, fallback, size, &ext);
    let title = details
        .title
        .clone()
        .unwrap_or_else(|| fallback.to_string());
    let mut md = format!("# {title}\n\n## Video details\n\n{}", details.markdown);

    // Play every instrument at once, each on its own thread: the screen and the speech.
    let (spoken, screens) = std::thread::scope(|scope| {
        let speech = scope.spawn(|| spoken(path, details.has_text_subtitles));
        let screen = scope.spawn(|| screen::capture(path, &opts.screen_lang, opts.max_frames));
        (
            speech
                .join()
                .map_err(|_| anyhow!("subtitle reader crashed")),
            screen
                .join()
                .map_err(|_| anyhow!("on-screen text reader crashed")),
        )
    });
    let (spoken, captured) = (spoken??, screens??);
    if captured.changed > 0 {
        md.push_str(&format!(
            "The screen changed {} times; text was read in {} of those frames (the rest showed a screen already read).\n\n",
            captured.changed, captured.read
        ));
    }
    let screens = captured.screens;
    let chapters = chapters(&probe);

    // Then assemble the score.
    let mut notes: Vec<(u32, Part)> = Vec::new();
    notes.extend(chapters.iter().map(|(t, c)| (*t, Part::Chapter(c))));
    notes.extend(spoken.iter().map(|(t, l)| (*t, Part::Spoken(l))));
    notes.extend(screens.iter().map(|sc| (sc.at, Part::Shown(sc))));
    md.push_str(&assemble(&mut notes));

    let mut out_omitted = Vec::new();
    if captured.skipped_by_limit > 0 {
        out_omitted.push(format!(
            "on-screen text at {} of {} changed moments (limit set with --max-frames)",
            captured.skipped_by_limit, captured.changed
        ));
    }
    if spoken.is_empty() {
        out_omitted.push("speech (no subtitles found, and speech is not transcribed)".to_string());
    }
    // The video's own description, else what is said, else what is shown, else technical facts.
    let said = spoken
        .iter()
        .map(|(_, l)| l.as_str())
        .find(|l| l.split_whitespace().count() >= 4)
        .map(str::to_string);
    let shown = screens
        .iter()
        .map(|s| s.first_line.clone())
        .find(|l| !l.is_empty());
    let description = details
        .description
        .or(said)
        .or(shown)
        .unwrap_or(details.tech);
    let mut out = Out::new(title, md).with_description(Some(description));
    out.omitted = out_omitted;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_are_assembled_bar_by_bar_in_time_order() {
        let sc = screen::Screen::for_test(5, "> Saving slide");
        let late = screen::Screen::for_test(130, "> Second bar slide");
        let mut notes = vec![
            (6, Part::Spoken("First line.")),
            (8, Part::Spoken("Second line.")),
            (5, Part::Shown(&sc)),
            (0, Part::Chapter("Intro")),
            (130, Part::Shown(&late)),
            (125, Part::Spoken("In the next bar.")),
        ];
        let md = assemble(&mut notes);
        let order = [
            "### 0:00 – 2:00",
            "**Chapter at 0:00: Intro**",
            "**On screen at 0:05**",
            "**Spoken**\n\nFirst line. Second line.",
            "### 2:00 – 4:00",
            "In the next bar.",
            "> Second bar slide",
        ];
        let mut at = 0;
        for piece in order {
            let found = md[at..]
                .find(piece)
                .unwrap_or_else(|| panic!("{piece:?} out of order in:\n{md}"));
            at += found + piece.len();
        }
    }
}
