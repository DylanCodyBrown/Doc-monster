//! The on-screen text track of a video: slides, code, title cards, credits, interface labels.
//!
//! FFmpeg decodes every frame, samples one per second and keeps only frames that differ from the
//! previous one (plus one at least every 30 seconds). Tesseract reads each kept frame as soon as
//! FFmpeg has written it, in parallel, so reading overlaps decoding instead of waiting for it.
//! Bullet symbols misread as letters are repaired, noise is dropped, consecutive frames showing
//! the same screen are merged, and code-looking text becomes a code block.

use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

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

#[cfg(test)]
impl Screen {
    pub fn for_test(at: u32, block: &str) -> Screen {
        Screen {
            at,
            block: block.to_string(),
            first_line: String::new(),
            words: HashSet::new(),
            word_count: 0,
        }
    }
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

/// Required programs that are not installed (checked once per run).
pub fn missing_tools() -> Vec<&'static str> {
    static MISSING: OnceLock<Vec<&'static str>> = OnceLock::new();
    MISSING
        .get_or_init(|| {
            ["ffmpeg", "ffprobe", "tesseract"]
                .into_iter()
                .filter(|p| !installed(p))
                .map(|p| {
                    if p == "tesseract" {
                        "Tesseract"
                    } else {
                        "FFmpeg"
                    }
                })
                .fold(Vec::new(), |mut v, p| {
                    if !v.contains(&p) {
                        v.push(p);
                    }
                    v
                })
        })
        .clone()
}

fn scratch_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "doc-monster-frames-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
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

pub fn capture(video: &Path, lang: &str, max_frames: usize) -> Result<Vec<Screen>> {
    let dir = scratch_dir();
    std::fs::create_dir_all(&dir).context("creating a folder for video frames")?;
    let result = read_while_decoding(video, &dir, lang, max_frames);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(merge(result?))
}

fn frame_path(dir: &Path, n: usize) -> PathBuf {
    dir.join(format!("f{n:06}.png"))
}

/// Run FFmpeg, and read each frame with Tesseract as soon as FFmpeg has finished writing it.
/// Returns (seconds, cleaned lines) in frame order.
fn read_while_decoding(
    video: &Path,
    dir: &Path,
    lang: &str,
    max_frames: usize,
) -> Result<Vec<(u32, Vec<String>)>> {
    let mut child = Command::new("ffmpeg")
        .args(["-v", "info", "-nostdin", "-i"])
        .arg(video)
        .args(["-an", "-sn", "-vf", "fps=1,mpdecimate=hi=768:lo=320:frac=0.33:max=30,showinfo,scale='min(1920,iw)':-2,format=gray"])
        .args(["-fps_mode", "vfr", "-frames:v", &max_frames.to_string()])
        .arg(dir.join("f%06d.png"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")?;
    // FFmpeg reports each kept frame's time on its error stream; drain it as it comes.
    let stderr = child.stderr.take().expect("piped");
    let times = std::thread::spawn(move || {
        use std::io::BufRead;
        std::io::BufReader::new(stderr)
            .lines()
            .map_while(|l| l.ok())
            .filter(|l| l.contains("showinfo"))
            .filter_map(|l| {
                l.split("pts_time:")
                    .nth(1)?
                    .split_whitespace()
                    .next()?
                    .parse::<f64>()
                    .ok()
            })
            .map(|t| t.max(0.0) as u32)
            .collect::<Vec<u32>>()
    });

    let read: Mutex<Vec<(usize, Vec<String>)>> = Mutex::new(Vec::new());
    rayon::scope(|workers| -> Result<()> {
        let mut sent = 0usize;
        loop {
            let finished = child.try_wait()?.is_some();
            // Frame n is complete once frame n+1 exists, or once FFmpeg has exited.
            while frame_path(dir, sent + 1).exists()
                && (finished || frame_path(dir, sent + 2).exists())
            {
                sent += 1;
                let (frame, read) = (frame_path(dir, sent), &read);
                workers.spawn(move |_| {
                    let lines = clean(&ocr(&frame, lang));
                    read.lock().unwrap().push((sent, lines));
                });
            }
            if finished {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    })?;
    let times = times.join().unwrap_or_default();
    let mut read = read.into_inner().unwrap();
    read.sort_by_key(|r| r.0);
    Ok(read
        .into_iter()
        .filter_map(|(n, lines)| Some((*times.get(n - 1)?, lines)))
        .collect())
}

/// Drop frames without real text and merge consecutive frames of the same screen.
fn merge(read: Vec<(u32, Vec<String>)>) -> Vec<Screen> {
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
    screens
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
