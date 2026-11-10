//! On-screen text in videos: slides, code, captions, interface labels.
//!
//! FFmpeg samples one frame per second and keeps only frames that differ from the previous one
//! (plus one at least every 30 seconds). Every frame is decoded by default: encoders do not
//! reliably put keyframes where slides change, so a keyframes-only pass (not `thorough`) is faster
//! but misses text; Tesseract reads the text in each kept frame, in
//! parallel. Bullet symbols misread as letters are repaired, noise is dropped, consecutive frames
//! showing the same screen are merged, and code-looking text becomes a code block.
//! Both programs run locally; when either is missing the caller reports it instead of failing.

use rayon::prelude::*;
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Screen {
    /// Seconds from the start when this screen first appears.
    pub at: u32,
    /// Markdown for the screen: quoted lines, or a code block.
    pub block: String,
    /// First meaningful line, used as a description when nothing better exists.
    pub first_line: String,
    words: HashSet<String>,
    word_count: usize,
}

pub enum Capture {
    Found(Vec<Screen>),
    /// A required program is not installed.
    Missing(&'static str),
}

/// Two kept frames whose words overlap this much show the same screen.
const SAME_SCREEN: f32 = 0.7;
/// A frame needs at least this many real words to count as having text.
const MIN_WORDS: usize = 3;

fn installed(program: &str) -> bool {
    Command::new(program)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
        || Command::new(program)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
}

fn scratch_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "doc-monster-frames-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// One slice of the video, decoded by its own FFmpeg process.
struct Slice {
    start: f64,
    len: Option<f64>,
    max_frames: usize,
    tag: usize,
}

/// Grab changed frames of one slice as grayscale PNGs; returns (frame file, seconds) in order.
fn grab_frames(
    video: &Path,
    dir: &Path,
    slice: &Slice,
    thorough: bool,
) -> Option<Vec<(PathBuf, u32)>> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "info", "-nostdin"]);
    if !thorough {
        cmd.args(["-skip_frame", "nokey"]);
    }
    if slice.start > 0.0 {
        cmd.args(["-ss", &format!("{:.3}", slice.start)]);
    }
    cmd.arg("-i").arg(video);
    if let Some(len) = slice.len {
        cmd.args(["-t", &format!("{len:.3}")]);
    }
    let pattern = format!("s{:03}-%06d.png", slice.tag);
    let out = cmd
        .args(["-an", "-sn", "-vf", "fps=1,mpdecimate=hi=768:lo=320:frac=0.33:max=30,showinfo,scale='min(1920,iw)':-2,format=gray"])
        .args(["-fps_mode", "vfr", "-frames:v", &slice.max_frames.to_string()])
        .arg(dir.join(pattern))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .ok()?;
    let log = String::from_utf8_lossy(&out.stderr);
    let times: Vec<u32> = log
        .lines()
        .filter(|l| l.contains("showinfo") || l.contains("Parsed_showinfo"))
        .filter_map(|l| l.split("pts_time:").nth(1))
        .filter_map(|t| t.split_whitespace().next()?.parse::<f64>().ok())
        .map(|t| (t.max(0.0) + slice.start) as u32)
        .collect();
    let prefix = format!("s{:03}-", slice.tag);
    let mut frames: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "png")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
        })
        .collect();
    frames.sort();
    Some(frames.into_iter().zip(times).collect())
}

fn ocr(frame: &Path, lang: &str) -> String {
    Command::new("tesseract")
        .arg(frame)
        .args(["stdout", "-l", lang, "--psm", "3"])
        // Frames are already read in parallel; one thread per Tesseract avoids oversubscription.
        .env("OMP_THREAD_LIMIT", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn bullet_junk() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // Bullet symbols that text recognition reads as a stray letter or sign.
    R.get_or_init(|| Regex::new(r"^(?:[•·●▪■◦○*¢«»©°>~—–-]|[eoa]|\(?[a-z0-9]\))\s+").unwrap())
}

fn code_like() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"[(){};=]|^\s*(local|function|end|if|for|while|return|def|class|import|from|let|const|var|fn|pub|print)\b|:\w+\(").unwrap())
}

fn real_words(line: &str) -> usize {
    line.split_whitespace()
        .filter(|w| w.chars().filter(|c| c.is_alphanumeric()).count() >= 2)
        .count()
}

/// Keep lines that read as text; turn misread bullets back into list markers.
pub fn clean(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let t = l.trim();
            let total = t.chars().count();
            let sensible = t
                .chars()
                .filter(|c| {
                    c.is_alphanumeric()
                        || c.is_whitespace()
                        || ".,:;!?'\"()[]{}<>/\\&%$#@+=_*-–—|~`^".contains(*c)
                })
                .count();
            if total == 0 || sensible * 100 / total < 85 || real_words(t) == 0 {
                return None;
            }
            let indent = &l[..l.len() - l.trim_start().len()];
            Some(match bullet_junk().find(t) {
                Some(m) if real_words(&t[m.end()..]) > 0 => format!("{indent}- {}", &t[m.end()..]),
                _ => l.to_string(),
            })
        })
        .collect()
}

fn to_block(lines: &[String]) -> String {
    let code = lines.iter().filter(|l| code_like().is_match(l)).count() * 2 >= lines.len();
    if code {
        format!("```\n{}\n```", lines.join("\n"))
    } else {
        lines
            .iter()
            .map(|l| format!("> {}", l.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn word_set(lines: &[String]) -> HashSet<String> {
    lines
        .iter()
        .flat_map(|l| l.split(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn overlap(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    a.intersection(b).count() as f32 / a.union(b).count() as f32
}

pub fn capture(video: &Path, lang: &str, max_frames: usize, thorough: bool) -> Capture {
    if !installed("ffmpeg") {
        return Capture::Missing("FFmpeg");
    }
    if !installed("tesseract") {
        return Capture::Missing("Tesseract");
    }
    let dir = scratch_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return Capture::Found(Vec::new());
    }
    // One pass over the whole video. (Cutting it into slices decoded in parallel was measured:
    // about 10% faster, because FFmpeg's decoder already uses several cores, and text at slice
    // boundaries was lost. Not worth it.)
    let whole = Slice {
        start: 0.0,
        len: None,
        max_frames,
        tag: 0,
    };
    let frames: Vec<(PathBuf, u32)> =
        grab_frames(video, &dir, &whole, thorough).unwrap_or_default();
    let read: Vec<(u32, Vec<String>)> = frames
        .par_iter()
        .map(|(f, t)| (*t, clean(&ocr(f, lang))))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);

    let mut screens: Vec<Screen> = Vec::new();
    for (at, lines) in read {
        let word_count: usize = lines.iter().map(|l| real_words(l)).sum();
        if word_count < MIN_WORDS {
            continue;
        }
        let words = word_set(&lines);
        let first_line = lines
            .iter()
            .map(|l| l.trim().trim_start_matches("- "))
            .find(|l| real_words(l) >= 2)
            .unwrap_or("")
            .to_string();
        let screen = Screen {
            at,
            block: to_block(&lines),
            first_line,
            words,
            word_count,
        };
        match screens.last_mut() {
            // The same screen again (still on it, or text being typed): keep the fuller reading.
            Some(prev) if overlap(&prev.words, &screen.words) >= SAME_SCREEN => {
                if screen.word_count > prev.word_count {
                    let at = prev.at;
                    *prev = screen;
                    prev.at = at;
                }
            }
            _ => screens.push(screen),
        }
    }
    Capture::Found(screens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn misread_bullets_become_list_items_and_noise_is_dropped() {
        let raw = "DataStore saving\ne Autosave every 60 seconds\n¢ Wrap every call in pcall\n\n~~|| ;;\ne\n";
        assert_eq!(
            clean(raw),
            vec![
                "DataStore saving",
                "- Autosave every 60 seconds",
                "- Wrap every call in pcall"
            ]
        );
    }

    #[test]
    fn code_becomes_a_code_block_and_slides_become_quotes() {
        let code = clean("local ok, err = pcall(function()\n    store:SetAsync(key, data)\nend)\n");
        assert!(
            to_block(&code).starts_with("```\nlocal ok"),
            "{}",
            to_block(&code)
        );
        let slide = clean("Pet trading\ne Two step confirmation\n");
        assert_eq!(to_block(&slide), "> Pet trading\n> - Two step confirmation");
    }
}
