mod csv_table;
mod docx;
mod html;
mod json;
mod pdf;
mod subtitles;
mod text;
mod video;
mod xlsx;

use crate::Document;
use anyhow::{Context, Result, bail};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Markdown,
    PlainText,
    Html,
    Csv,
    Json,
    Docx,
    Pdf,
    Xlsx,
    Video,
    Subtitles,
    /// Any other text file (source code, config, logs). Wrapped in a code fence.
    Code,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Markdown => "markdown",
            Format::PlainText => "text",
            Format::Html => "html",
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Docx => "docx",
            Format::Pdf => "pdf",
            Format::Xlsx => "spreadsheet",
            Format::Video => "video",
            Format::Subtitles => "subtitles",
            Format::Code => "code",
        }
    }
}

/// Decide the format from the extension, falling back to content sniffing.
pub fn detect(path: &Path, bytes: &[u8]) -> Result<Format> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let by_ext = match ext.as_str() {
        "md" | "markdown" => Some(Format::Markdown),
        "txt" | "text" => Some(Format::PlainText),
        "html" | "htm" | "xhtml" => Some(Format::Html),
        "csv" | "tsv" => Some(Format::Csv),
        "json" => Some(Format::Json),
        "docx" => Some(Format::Docx),
        "pdf" => Some(Format::Pdf),
        "xlsx" | "xlsm" | "xls" | "xlsb" | "ods" => Some(Format::Xlsx),
        "mp4" | "m4v" | "mov" => Some(Format::Video),
        "srt" | "vtt" => Some(Format::Subtitles),
        _ => None,
    };
    if let Some(f) = by_ext {
        return Ok(f);
    }
    if bytes.starts_with(b"%PDF-") {
        return Ok(Format::Pdf);
    }
    if bytes.starts_with(b"PK\x03\x04") {
        bail!("zip-based file with unsupported extension '.{ext}'");
    }
    if std::str::from_utf8(bytes).is_ok() && !bytes.contains(&0) {
        return Ok(Format::Code);
    }
    bail!("unrecognized binary format '.{ext}'")
}

/// Read a file and convert it to full-fidelity Markdown.
pub fn extract(path: &Path) -> Result<Document> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let format = detect(path, &bytes)?;
    let fallback_title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();

    let (title, markdown) = match format {
        Format::Markdown | Format::PlainText => text::convert(&bytes, format, &fallback_title)?,
        Format::Code => text::convert_code(&bytes, path, &fallback_title)?,
        Format::Html => html::convert(&bytes, &fallback_title)?,
        Format::Csv => csv_table::convert(&bytes, path, &fallback_title)?,
        Format::Json => json::convert(&bytes, &fallback_title)?,
        Format::Docx => docx::convert(&bytes, &fallback_title)?,
        Format::Pdf => pdf::convert(&bytes, &fallback_title)?,
        Format::Xlsx => xlsx::convert(&bytes, &fallback_title)?,
        Format::Video => video::convert(&bytes, path, &fallback_title)?,
        Format::Subtitles => subtitles::convert(&bytes, &fallback_title)?,
    };

    Ok(Document {
        title,
        source: path.to_path_buf(),
        format,
        markdown,
    })
}

/// First level-one heading in Markdown, if any.
pub(crate) fn first_heading(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find_map(|l| l.strip_prefix("# ").map(|t| t.trim().to_string()))
        .filter(|t| !t.is_empty())
}
