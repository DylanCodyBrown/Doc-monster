use super::{Out, first_heading};
use anyhow::Result;
use std::path::Path;

/// Decode bytes of unknown encoding: byte order marks first, then UTF-8, then Windows-1252.
pub fn decode(bytes: &[u8]) -> String {
    if let Some((enc, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc.decode_without_bom_handling(&bytes[bom_len..]).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

pub fn looks_like_text(head: &[u8]) -> bool {
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) || head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return true;
    }
    let sample = &head[..head.len().min(4096)];
    !sample.contains(&0)
        && sample.iter().filter(|&&b| b < 9 || (b > 13 && b < 32)).count() * 100 <= sample.len().max(1)
}

pub fn convert(bytes: &[u8], fallback: &str) -> Result<(String, String)> {
    let text = decode(bytes);
    let title = first_heading(&text).unwrap_or_else(|| fallback.to_string());
    Ok((title, text))
}

/// Split a leading YAML front matter block off a Markdown file.
pub fn split_front_matter(text: &str) -> (Option<&str>, &str) {
    let rest = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n"));
    let Some(rest) = rest else { return (None, text) };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" || line.trim_end() == "..." {
            return (Some(&rest[..offset]), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, text)
}

/// Markdown keeps its own title and description; any other front matter is preserved
/// at the end of the document inside a code block so nothing is lost.
pub fn convert_markdown(bytes: &[u8], fallback: &str) -> Result<Out> {
    let text = decode(bytes);
    let (fm, body) = split_front_matter(&text);
    let (mut title, mut description) = (None, None);
    let mut leftover = String::new();
    if let Some(fm) = fm {
        for line in fm.lines() {
            let key = line.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().trim_matches(['"', '\''])));
            match key {
                Some((k, v)) if k == "title" && !v.is_empty() && title.is_none() => title = Some(v.to_string()),
                Some((k, v)) if k == "description" && !v.is_empty() && description.is_none() => description = Some(v.to_string()),
                _ => {
                    leftover.push_str(line);
                    leftover.push('\n');
                }
            }
        }
    }
    let mut md = body.trim_start_matches('\n').to_string();
    if !leftover.trim().is_empty() {
        md.push_str(&format!("\n\n## Original front matter\n\n```yaml\n{}\n```\n", leftover.trim_end()));
    }
    let title = title
        .or_else(|| first_heading(&md))
        .unwrap_or_else(|| fallback.to_string());
    Ok(Out::new(title, md).with_description(description))
}

pub fn convert_code(bytes: &[u8], path: &Path, fallback: &str) -> Result<(String, String)> {
    let text = decode(bytes);
    let lang = match path.extension().and_then(|e| e.to_str()) {
        Some("log" | "out" | "err") | None => "text",
        Some(e) => e,
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16_and_windows_1252() {
        let mut utf16 = vec![0xFF, 0xFE];
        for u in "héllo".encode_utf16() {
            utf16.extend(u.to_le_bytes());
        }
        assert_eq!(decode(&utf16), "héllo");
        assert_eq!(decode(&[b'c', b'a', b'f', 0xE9]), "café");
    }

    #[test]
    fn markdown_front_matter_is_used_and_preserved() {
        let src = "---\ntitle: \"My Doc\"\ndescription: About stuff\ntags: [a, b]\n---\n\n# Heading\n\nBody.\n";
        let out = convert_markdown(src.as_bytes(), "file").unwrap();
        assert_eq!(out.title, "My Doc");
        assert_eq!(out.description.as_deref(), Some("About stuff"));
        assert!(out.markdown.starts_with("# Heading"));
        assert!(out.markdown.contains("## Original front matter") && out.markdown.contains("tags: [a, b]"));
        assert!(!out.markdown.contains("title: \"My Doc\""));
    }

    #[test]
    fn fence_survives_backticks_in_content() {
        let (_, md) = convert_code(b"a ``` b", Path::new("x.rs"), "x").unwrap();
        assert!(md.starts_with("````rs\n"));
    }
}
