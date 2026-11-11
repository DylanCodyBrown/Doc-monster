//! Pictures (.png .jpg .gif .bmp .tif .webp): the text in them, read with Tesseract.
//!
//! FFmpeg opens the picture, whatever its format, and writes it as a plain grayscale PGM file,
//! enlarged when small, because Tesseract reads small text much better at a larger size. The
//! recognised text keeps its paragraphs; a short first line becomes the heading, and pictures
//! of source code keep their line breaks in a code block.

use super::{Out, Skipped, screen};
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Stdio;

/// Pictures narrower than this are enlarged to twice their size before reading.
const SMALL: u32 = 1200;

pub fn convert(path: &Path, fallback: &str, lang: &str) -> Result<Out> {
    let missing = screen::missing_tools();
    if !missing.is_empty() {
        bail!(
            "reading text in pictures needs {} installed (see INSTALL.md)",
            missing.join(" and ")
        );
    }
    let dir = screen::scratch_dir();
    std::fs::create_dir_all(&dir).context("creating a folder for the picture")?;
    let result = read(path, &dir, lang);
    let _ = std::fs::remove_dir_all(&dir);
    let text = result?;
    // A picture without text (a photo, an icon) is left alone rather than written out empty.
    to_out(&text, fallback).ok_or_else(|| Skipped("no readable text in this picture".into()).into())
}

fn read(path: &Path, dir: &Path, lang: &str) -> Result<String> {
    let gray = dir.join("picture.pgm");
    let scale = format!("scale='if(lt(iw,{SMALL}),iw*2,iw)':-2:flags=bicubic,format=gray");
    let out = crate::tools::command("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        // Only the first frame of an animated or multi-page picture.
        .args(["-frames:v", "1", "-vf", &scale, "-y"])
        .arg(&gray)
        .stdin(Stdio::null())
        .output()
        .context("starting ffmpeg")?;
    if !out.status.success() || !gray.is_file() {
        let why = String::from_utf8_lossy(&out.stderr);
        let why = why.lines().last().unwrap_or("unknown error").trim();
        bail!("could not open this picture ({why})");
    }
    Ok(screen::ocr(&gray, lang))
}

/// Recognised text as Markdown: paragraphs kept, noise lines dropped, bullets repaired.
/// None when there is no readable text.
fn to_out(text: &str, fallback: &str) -> Option<Out> {
    let mut paragraphs: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else if let Some(l) = screen::clean_line(line) {
            current.push(l);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }

    if paragraphs.is_empty() {
        return None;
    }
    let mut out = Out::new(fallback, String::new())
        .omitting("the picture itself (only the text in it is included)".to_string());

    let mut md = String::new();
    // A short first line on its own, without a full stop, is the picture's heading.
    if let [only] = paragraphs[0].as_slice()
        && only.split_whitespace().count() <= 8
        && !only.trim_end().ends_with(['.', ':', ';', ','])
        && !only.trim_start().starts_with("- ")
    {
        let heading = only.trim().to_string();
        md.push_str(&format!("# {heading}\n\n"));
        out.title = heading;
        paragraphs.remove(0);
    }
    for p in &paragraphs {
        md.push_str(&block(p));
        md.push_str("\n\n");
    }
    out.markdown = md.trim_end().to_string() + "\n";
    Some(out)
}

/// One paragraph: code keeps its lines in a fence, list items stay one per line, and text
/// that Tesseract broke across lines is joined back into one line.
fn block(lines: &[String]) -> String {
    if lines.len() > 1 && screen::mostly_code(lines) {
        return format!("```\n{}\n```", lines.join("\n"));
    }
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        let t = l.trim();
        match out.last_mut() {
            Some(prev) if !t.starts_with("- ") && !prev.is_empty() => {
                prev.push(' ');
                prev.push_str(t);
            }
            _ => out.push(t.to_string()),
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_heading_and_wrapped_lines() {
        let text = "Aurora Pet Shop Notice\n\nPets hatch from eggs bought with coins earned in the\narena.\n\n\u{2022} Rare pets appear rarely\n\u{2022} Trading opens Saturday\n\n~~~ ;;; ~~~\n";
        let out = to_out(text, "sign").unwrap();
        assert_eq!(out.title, "Aurora Pet Shop Notice");
        assert!(
            out.markdown.starts_with("# Aurora Pet Shop Notice\n\n"),
            "{}",
            out.markdown
        );
        assert!(
            out.markdown
                .contains("Pets hatch from eggs bought with coins earned in the arena."),
            "{}",
            out.markdown
        );
        assert!(
            out.markdown
                .contains("- Rare pets appear rarely\n- Trading opens Saturday"),
            "{}",
            out.markdown
        );
        assert!(
            !out.markdown.contains(";;;"),
            "noise lines are dropped: {}",
            out.markdown
        );
        assert!(out.omitted[0].contains("picture itself"));
    }

    #[test]
    fn code_keeps_its_lines_and_no_text_is_nothing() {
        let out = to_out(
            "local coins = 0\nfor i = 1, 10 do\n  coins += i\nend\n",
            "code",
        )
        .unwrap();
        assert!(
            out.markdown.contains("```\nlocal coins = 0\n"),
            "{}",
            out.markdown
        );
        assert!(
            to_out("  \n|||\n", "blank").is_none(),
            "no text: nothing to write"
        );
    }
}
