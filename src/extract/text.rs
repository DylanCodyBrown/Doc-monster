use super::{Format, first_heading};
use anyhow::{Context, Result};
use std::path::Path;

pub fn convert(bytes: &[u8], _format: Format, fallback: &str) -> Result<(String, String)> {
    let text = String::from_utf8(bytes.to_vec()).context("file is not valid UTF-8 text")?;
    let text = text.trim_start_matches('\u{feff}').to_string();
    let title = first_heading(&text).unwrap_or_else(|| fallback.to_string());
    Ok((title, text))
}

pub fn convert_code(bytes: &[u8], path: &Path, fallback: &str) -> Result<(String, String)> {
    let text = String::from_utf8(bytes.to_vec()).context("file is not valid UTF-8 text")?;
    let lang = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    // Use a fence longer than any backtick run inside the file.
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0)
        .max(2);
    let fence = "`".repeat(longest + 1);
    let md = format!("{fence}{lang}\n{}\n{fence}\n", text.trim_end());
    Ok((fallback.to_string(), md))
}
