use anyhow::{Context, Result};

pub fn convert(bytes: &[u8], fallback: &str) -> Result<(String, String)> {
    let raw = pdf_extract::extract_text_from_mem(bytes).context("reading PDF text")?;
    if raw.trim().is_empty() {
        anyhow::bail!("PDF has no extractable text (scanned image? OCR is not supported yet)");
    }
    Ok((fallback.to_string(), reflow(&raw)))
}

/// PDFs hard-wrap lines. Join wrapped lines into paragraphs, keep blank lines
/// as paragraph breaks, and repair words hyphenated across line ends.
fn reflow(raw: &str) -> String {
    let mut out = String::new();
    let mut para = String::new();
    let flush = |para: &mut String, out: &mut String| {
        if !para.trim().is_empty() {
            out.push_str(para.trim());
            out.push_str("\n\n");
        }
        para.clear();
    };
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush(&mut para, &mut out);
        } else if para.ends_with('-') && line.chars().next().is_some_and(char::is_lowercase) {
            para.pop();
            para.push_str(line);
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(line);
        }
    }
    flush(&mut para, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::reflow;

    #[test]
    fn joins_wrapped_lines_and_hyphens() {
        let out = reflow("The quick brown\nfox jumps over the lazy do-\ngs today.\n\nNext paragraph.\n");
        assert_eq!(out, "The quick brown fox jumps over the lazy dogs today.\n\nNext paragraph.\n\n");
    }
}
