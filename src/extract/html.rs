use super::first_heading;
use anyhow::Result;

pub fn convert(bytes: &[u8], fallback: &str) -> Result<(String, String)> {
    let html = String::from_utf8_lossy(bytes);
    let md = htmd::convert(&strip_head(&html)).map_err(|e| anyhow::anyhow!("html conversion failed: {e}"))?;
    let title = html_title(&html)
        .or_else(|| first_heading(&md))
        .unwrap_or_else(|| fallback.to_string());
    Ok((title, md))
}

fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title>")? + open_end;
    let t = html[open_end..close].trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// The title is captured separately; drop `<head>` so it does not leak into the body.
fn strip_head(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    match (lower.find("<head"), lower.find("</head>")) {
        (Some(a), Some(b)) if b > a => format!("{}{}", &html[..a], &html[b + "</head>".len()..]),
        _ => html.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_not_duplicated_in_body() {
        let (title, md) = convert(b"<html><head><title>T</title></head><body><h1>H</h1></body></html>", "x").unwrap();
        assert_eq!(title, "T");
        assert!(!md.contains("T\n") && md.contains("# H"));
    }
}
