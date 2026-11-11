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
use std::process::Stdio;
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
    crate::tools::command(program)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
        || crate::tools::command(program)
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

pub(crate) fn scratch_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "doc-monster-frames-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

pub(crate) fn ocr(frame: &Path, lang: &str) -> String {
    crate::tools::command("tesseract")
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
    text.lines().filter_map(clean_line).collect()
}

/// One recognised line: None when it is noise (stray symbols, no real words), otherwise the
/// line with a misread bullet symbol turned back into a list marker.
pub(crate) fn clean_line(l: &str) -> Option<String> {
    let l = l.trim_end();
    let t = l.trim();
    let total = t.chars().count();
    if total == 0 {
        return None;
    }
    let sensible = t
        .chars()
        .filter(|c| {
            c.is_alphanumeric()
                || c.is_whitespace()
                || ".,:;!?'\"()[]{}<>/\\&%$#@+=_*-–—|~`^".contains(*c)
        })
        .count();
    if sensible * 100 / total < 85 || real_words(t) == 0 {
        return None;
    }
    let indent = &l[..l.len() - l.trim_start().len()];
    Some(match bullet_junk().find(t) {
        Some(m) if real_words(&t[m.end()..]) > 0 => format!("{indent}- {}", &t[m.end()..]),
        _ => l.to_string(),
    })
}

/// True when most lines look like source code (so they keep their line breaks).
pub(crate) fn mostly_code(lines: &[String]) -> bool {
    lines.iter().filter(|l| code_like().is_match(l)).count() * 2 >= lines.len()
}

fn to_block(lines: &[String]) -> String {
    if mostly_code(lines) {
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

/// What the screen track found, with counts that say how much work the hash table saved.
pub struct Captured {
    pub screens: Vec<Screen>,
    /// Frames where the picture changed (after FFmpeg's change detection).
    pub changed: usize,
    /// Of those, frames whose text was actually read by Tesseract.
    pub read: usize,
    /// Changed frames not read because of `--max-frames`.
    pub skipped_by_limit: usize,
}

/// `max_frames`: most frames read by Tesseract (`None` = every changed frame).
pub fn capture(video: &Path, lang: &str, max_frames: Option<usize>) -> Result<Captured> {
    let dir = scratch_dir();
    std::fs::create_dir_all(&dir).context("creating a folder for video frames")?;
    let result = read_while_decoding(video, &dir, lang, max_frames);
    let _ = std::fs::remove_dir_all(&dir);
    let (frames, stats) = result?;
    Ok(Captured {
        screens: merge(frames),
        changed: stats.0,
        read: stats.1,
        skipped_by_limit: stats.2,
    })
}

fn frame_path(dir: &Path, n: usize) -> PathBuf {
    dir.join(format!("f{n:06}.pgm"))
}

/// Size of the thumbnail used as each frame's fingerprint.
const THUMB_W: usize = 64;
const THUMB_H: usize = 36;
const THUMB: usize = THUMB_W * THUMB_H;

/// Hash-table key of a thumbnail: one bit per cell of an 8x4 grid, set where the cell is
/// brighter than the thumbnail's average. Identical screens always share a key.
fn bucket(thumb: &[u8]) -> u32 {
    let mean = thumb.iter().map(|&p| u32::from(p)).sum::<u32>() / THUMB as u32;
    let mut key = 0u32;
    for gy in 0..4 {
        for gx in 0..8 {
            let mut sum = 0u32;
            for y in gy * 9..(gy + 1) * 9 {
                for x in gx * 8..(gx + 1) * 8 {
                    sum += u32::from(thumb[y * THUMB_W + x]);
                }
            }
            if sum / 72 > mean {
                key |= 1 << (gy * 8 + gx);
            }
        }
    }
    key
}

/// Two thumbnails show the same screen only if no area differs noticeably. Strict on purpose:
/// two slides on the same template but with different text must count as different.
fn same_screen(a: &[u8], b: &[u8]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 6)
}

/// Run FFmpeg; read each changed frame with Tesseract as soon as FFmpeg has written it, unless
/// the hash table of screens already seen says its text is known.
/// Returns (seconds, cleaned lines) in frame order and (changed, read, skipped by limit).
#[allow(clippy::type_complexity)]
fn read_while_decoding(
    video: &Path,
    dir: &Path,
    lang: &str,
    max_frames: Option<usize>,
) -> Result<(Vec<(u32, Vec<String>)>, (usize, usize, usize))> {
    let graph = "[0:v]fps=1,mpdecimate=hi=768:lo=320:frac=0.33:max=30,showinfo,split=2[f][t];\
                 [f]scale='min(1920,iw)':-2,format=gray[full];[t]scale=64:36:flags=area,format=gray[thumb]";
    let mut child = crate::tools::command("ffmpeg")
        .args(["-v", "info", "-nostdin", "-i"])
        .arg(video)
        .args(["-filter_complex", graph])
        .args(["-map", "[full]", "-fps_mode", "vfr"])
        .arg(dir.join("f%06d.pgm"))
        .args([
            "-map",
            "[thumb]",
            "-fps_mode",
            "vfr",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
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
    // ...and each kept frame's thumbnail on standard output.
    let thumbs: std::sync::Arc<Mutex<Vec<Vec<u8>>>> = Default::default();
    let thumbs_in = thumbs.clone();
    let mut stdout = child.stdout.take().expect("piped");
    let thumbs_done = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = vec![0u8; THUMB];
        while stdout.read_exact(&mut buf).is_ok() {
            thumbs_in.lock().unwrap().push(buf.clone());
        }
    });

    let read: Mutex<Vec<(usize, Vec<String>)>> = Mutex::new(Vec::new());
    // Hash table of screens already sent to Tesseract: bucket key -> frame numbers.
    let mut seen: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
    let mut copy_of: Vec<(usize, usize)> = Vec::new();
    let (mut sent, mut ocr_count, mut skipped) = (0usize, 0usize, 0usize);
    rayon::scope(|workers| -> Result<()> {
        loop {
            let finished = child.try_wait()?.is_some();
            if finished {
                let _ = thumbs_done.is_finished();
            }
            // Frame n is complete once frame n+1 exists, or once FFmpeg has exited.
            while frame_path(dir, sent + 1).exists()
                && (finished || frame_path(dir, sent + 2).exists())
            {
                let thumb = thumbs.lock().unwrap().get(sent).cloned();
                if thumb.is_none() && !finished {
                    break; // its thumbnail has not arrived yet
                }
                sent += 1;
                if let Some(t) = &thumb {
                    let key = bucket(t);
                    let known = seen.get(&key).and_then(|list| {
                        let all = thumbs.lock().unwrap();
                        list.iter().copied().find(|&k| same_screen(&all[k - 1], t))
                    });
                    if let Some(k) = known {
                        copy_of.push((sent, k));
                        let _ = std::fs::remove_file(frame_path(dir, sent));
                        continue;
                    }
                    seen.entry(key).or_default().push(sent);
                }
                if max_frames.is_some_and(|m| ocr_count >= m) {
                    skipped += 1;
                    let _ = std::fs::remove_file(frame_path(dir, sent));
                    continue;
                }
                ocr_count += 1;
                let (frame, read, n) = (frame_path(dir, sent), &read, sent);
                workers.spawn(move |_| {
                    let lines = clean(&ocr(&frame, lang));
                    // Frames are uncompressed; remove each once read so they never pile up.
                    let _ = std::fs::remove_file(&frame);
                    read.lock().unwrap().push((n, lines));
                });
            }
            if finished && !frame_path(dir, sent + 1).exists() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    })?;
    let _ = thumbs_done.join();
    let times = times.join().unwrap_or_default();
    let mut read = read.into_inner().unwrap();
    // Screens seen before get the text read the first time.
    let known: std::collections::HashMap<usize, Vec<String>> = read.iter().cloned().collect();
    read.extend(
        copy_of
            .into_iter()
            .filter_map(|(n, k)| Some((n, known.get(&k)?.clone()))),
    );
    read.sort_by_key(|r| r.0);
    let frames = read
        .into_iter()
        .filter_map(|(n, lines)| Some((*times.get(n - 1)?, lines)))
        .collect();
    Ok((frames, (sent, ocr_count, skipped)))
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
    fn the_hash_table_matches_the_same_screen_but_not_a_different_slide() {
        let blank = vec![240u8; THUMB];
        let mut slide_a = blank.clone();
        let mut slide_b = blank.clone();
        // Two "slides" on the same template: a title bar, then text in different rows.
        for x in 4..60 {
            slide_a[3 * THUMB_W + x] = 20;
            slide_b[3 * THUMB_W + x] = 20;
            slide_a[12 * THUMB_W + x] = 60;
            slide_b[20 * THUMB_W + x] = 60;
        }
        let mut a_again = slide_a.clone();
        a_again[100] = a_again[100].saturating_sub(3); // compression noise
        assert!(same_screen(&slide_a, &a_again) && bucket(&slide_a) == bucket(&a_again));
        assert!(
            !same_screen(&slide_a, &slide_b),
            "different text on the same template is a different screen"
        );
    }

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
