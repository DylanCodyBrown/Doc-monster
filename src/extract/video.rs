use super::subtitles;
use anyhow::{Context, Result};
use mp4::{MediaType, Mp4Reader, TrackType};
use std::io::Cursor;
use std::path::Path;

fn clock(secs: u64) -> String {
    if secs >= 3600 { format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60) } else { format!("{}:{:02}", secs / 60, secs % 60) }
}

/// Technical facts about the video, plus a timed transcript if a subtitle file sits beside it.
/// Speech is not transcribed: that needs a speech model, which this tool avoids by default.
pub fn convert(bytes: &[u8], path: &Path, fallback: &str) -> Result<(String, String)> {
    let size = bytes.len() as u64;
    let reader = Mp4Reader::read_header(Cursor::new(bytes), size).context("not a readable mp4 video")?;
    let secs = reader.duration().as_secs();

    let mut md = format!("# {fallback}\n\n## Video details\n\n");
    md.push_str(&format!(
        "{fallback} is a video file of {} ({:.1} megabytes), brand {}.\n\n",
        clock(secs),
        size as f64 / 1_048_576.0,
        reader.major_brand()
    ));
    md.push_str("| Track | Kind | Details |\n| --- | --- | --- |\n");
    let mut ids: Vec<_> = reader.tracks().keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let t = &reader.tracks()[&id];
        let (kind, detail) = match t.track_type() {
            Ok(TrackType::Video) => (
                "video",
                format!(
                    "{} {}x{} at {:.2} frames per second, {} kilobits per second",
                    match t.media_type() { Ok(MediaType::H264) => "H.264", Ok(MediaType::H265) => "H.265", Ok(MediaType::VP9) => "VP9", _ => "unknown codec" },
                    t.width(), t.height(), t.frame_rate(), t.bitrate() / 1000
                ),
            ),
            Ok(TrackType::Audio) => {
                let mut d = match t.media_type() { Ok(MediaType::AAC) => "AAC".to_string(), _ => "unknown codec".to_string() };
                if t.bitrate() > 0 {
                    d.push_str(&format!(", {} kilobits per second", t.bitrate() / 1000));
                }
                if t.language() != "und" {
                    d.push_str(&format!(", language {}", t.language()));
                }
                ("audio", d)
            }
            Ok(TrackType::Subtitle) => ("subtitle", t.language().to_string()),
            Err(_) => ("other", String::new()),
        };
        md.push_str(&format!("| {id} | {kind} | {detail} |\n"));
    }
    md.push('\n');

    let sidecar = ["srt", "vtt", "en.srt", "en.vtt"]
        .iter()
        .map(|ext| path.with_extension(ext))
        .find(|p| p.exists());
    match sidecar {
        Some(p) => {
            let text = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
            let (_, t) = subtitles::convert(&text, "Transcript")?;
            // Demote the transcript's own title so the video keeps a single top heading.
            md.push_str(&t.replacen("# Transcript (transcript)", "## Transcript", 1).replace("\n## ", "\n### "));
        }
        None => md.push_str("_No transcript: place a `.srt` or `.vtt` file with the same name next to the video to include one._\n"),
    }
    Ok((fallback.to_string(), md))
}
