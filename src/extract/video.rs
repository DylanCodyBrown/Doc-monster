//! Video files of any common container (mp4, mov, mkv, webm, avi, wmv, flv, mpeg, ...).
//!
//! Details come from `ffprobe` when it is installed (all containers, chapters, tags), with a
//! built-in mp4 reader as the fallback. A transcript comes from a subtitle file beside the video,
//! or from a subtitle track inside it (via `ffmpeg`). Speech is NOT transcribed: that needs a
//! speech model, which this tool avoids by default.

use super::{Out, screen, subtitles};
use anyhow::{Context, Result, bail};
use mp4::{MediaType, Mp4Reader, TrackType};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};

fn clock(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

enum Probe {
    Found(Value),
    /// ffprobe ran and rejected the file.
    Unreadable,
    /// ffprobe is not installed.
    Missing,
}

fn probe(path: &Path) -> Probe {
    let Ok(out) = Command::new("ffprobe")
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
    else {
        return Probe::Missing;
    };
    match out
        .status
        .success()
        .then(|| serde_json::from_slice(&out.stdout).ok())
        .flatten()
    {
        Some(v) => Probe::Found(v),
        None => Probe::Unreadable,
    }
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
    let chapters: Vec<String> = p
        .get("chapters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            let start = s(c, "start_time")
                .and_then(|t| t.parse::<f64>().ok())
                .unwrap_or(0.0) as u64;
            format!(
                "- {} – {}",
                clock(start),
                tag(c, "title").unwrap_or("Untitled chapter")
            )
        })
        .collect();
    if !chapters.is_empty() {
        md.push_str(&format!("## Chapters\n\n{}\n\n", chapters.join("\n")));
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

/// Built-in mp4/mov reader, used when ffprobe is not installed.
fn native_mp4(path: &Path, fallback: &str, size: u64) -> Option<Details> {
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let reader = Mp4Reader::read_header(file, size).ok()?;
    let mut md = format!(
        "{fallback} is a video file (mp4, {}, {:.1} megabytes).\n\n",
        clock(reader.duration().as_secs()),
        size as f64 / 1_048_576.0
    );
    md.push_str("| Track | Kind | Details |\n| --- | --- | --- |\n");
    let mut ids: Vec<_> = reader.tracks().keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let t = &reader.tracks()[&id];
        let (kind, detail) = match t.track_type() {
            Ok(TrackType::Video) => (
                "video",
                format!(
                    "{} {}x{} at {:.2} frames per second",
                    match t.media_type() {
                        Ok(MediaType::H264) => "h264",
                        Ok(MediaType::H265) => "hevc",
                        Ok(MediaType::VP9) => "vp9",
                        _ => "unknown codec",
                    },
                    t.width(),
                    t.height(),
                    t.frame_rate()
                ),
            ),
            Ok(TrackType::Audio) => (
                "audio",
                match t.media_type() {
                    Ok(MediaType::AAC) => "aac".to_string(),
                    _ => "unknown codec".to_string(),
                },
            ),
            Ok(TrackType::Subtitle) => ("subtitle", t.language().to_string()),
            Err(_) => ("other", String::new()),
        };
        md.push_str(&format!("| {id} | {kind} | {detail} |\n"));
    }
    md.push('\n');
    let tech = format!("mp4 video, {}.", clock(reader.duration().as_secs()));
    Some(Details {
        markdown: md,
        title: None,
        description: None,
        has_text_subtitles: false,
        tech,
    })
}

fn embedded_subtitles(path: &Path) -> Option<String> {
    let out = Command::new("ffmpeg")
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

pub fn convert(path: &Path, fallback: &str, opts: &super::Options) -> Result<Out> {
    let size = std::fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let details = match probe(path) {
        Probe::Found(p) => from_probe(&p, fallback, size, &ext),
        Probe::Unreadable => bail!("not a readable video file"),
        Probe::Missing => native_mp4(path, fallback, size).unwrap_or_else(|| Details {
            markdown: format!(
                "{fallback} is a video file ({:.1} megabytes). Install `ffprobe` (part of FFmpeg) to read its duration, tracks and chapters.\n\n",
                size as f64 / 1_048_576.0
            ),
            title: None,
            description: None,
            has_text_subtitles: false,
            tech: format!("{ext} video."),
        }),
    };
    let title = details
        .title
        .clone()
        .unwrap_or_else(|| fallback.to_string());
    let mut md = format!("# {title}\n\n## Video details\n\n{}", details.markdown);

    let sidecar = ["srt", "vtt", "en.srt", "en.vtt"]
        .iter()
        .map(|ext| path.with_extension(ext))
        .find(|p| p.exists());
    let transcript = match sidecar {
        Some(p) => Some(super::text::decode(
            &std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?,
        )),
        None if details.has_text_subtitles => embedded_subtitles(path),
        None => None,
    };
    let cues = transcript
        .as_deref()
        .map(subtitles::cues)
        .unwrap_or_default();

    let mut omitted: Vec<String> = Vec::new();
    if cues.is_empty() {
        omitted.push("speech (no subtitles found, and speech is not transcribed)".into());
    }
    let screens = if opts.screen_text {
        match screen::capture(
            path,
            &opts.screen_lang,
            opts.max_frames,
            opts.screen_thorough,
        ) {
            screen::Capture::Found(s) => s,
            screen::Capture::Missing(tool) => {
                omitted.push(format!("on-screen text (install {tool} to capture it)"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // One timeline in two-minute sections, in time order: what was said, and what was on screen.
    const WINDOW: u32 = 120;
    enum Event<'a> {
        Said(&'a str),
        Shown(&'a screen::Screen),
    }
    let mut events: Vec<(u32, u8, Event)> =
        cues.iter().map(|(t, l)| (*t, 1, Event::Said(l))).collect();
    // A screen and a line starting at the same second: show the screen first.
    events.extend(screens.iter().map(|sc| (sc.at, 0, Event::Shown(sc))));
    events.sort_by_key(|e| (e.0, e.1));
    if !events.is_empty() {
        md.push_str("## Timeline\n\n");
        let mut window = u32::MAX;
        let mut said: Vec<&str> = Vec::new();
        let flush = |said: &mut Vec<&str>, md: &mut String| {
            if !said.is_empty() {
                md.push_str(&format!("**Spoken**\n\n{}\n\n", said.join(" ")));
                said.clear();
            }
        };
        for (t, _, ev) in &events {
            if t / WINDOW != window {
                flush(&mut said, &mut md);
                window = t / WINDOW;
                md.push_str(&format!(
                    "### {} – {}\n\n",
                    subtitles::stamp(window * WINDOW),
                    subtitles::stamp((window + 1) * WINDOW)
                ));
            }
            match ev {
                Event::Said(l) => said.push(l),
                Event::Shown(sc) => {
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
    }

    // The video's own description, else what is said, else what is shown, else technical facts.
    let spoken = cues
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
        .or(spoken)
        .or(shown)
        .unwrap_or(details.tech);
    let mut out = Out::new(title, md).with_description(Some(description));
    out.omitted = omitted;
    Ok(out)
}
