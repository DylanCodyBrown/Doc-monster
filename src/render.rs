use crate::Document;

/// Final Markdown file: front matter, summary (if any), then the complete
/// converted content so nothing from the source is lost.
pub fn render(doc: &Document, summary: Option<(&str, &str)>, include_full: bool) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_string(&doc.title)));
    out.push_str(&format!(
        "source: {}\n",
        yaml_string(&doc.source.display().to_string())
    ));
    out.push_str(&format!("format: {}\n", doc.format.label()));
    if let Some((method, _)) = summary {
        out.push_str(&format!("summarized_with: {}\n", yaml_string(method)));
    }
    out.push_str("---\n\n");

    if let Some((_, text)) = summary {
        out.push_str("## Summary\n\n");
        out.push_str(text.trim());
        out.push('\n');
        if include_full {
            out.push_str("\n---\n\n## Full content\n\n");
        }
    }
    if include_full || summary.is_none() {
        out.push_str(doc.markdown.trim_end());
        out.push('\n');
    }
    out
}

fn yaml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Format;

    fn doc() -> Document {
        Document {
            title: "T \"q\"".into(),
            source: "a.txt".into(),
            format: Format::PlainText,
            markdown: "body text\n".into(),
        }
    }

    #[test]
    fn always_includes_full_content() {
        let out = render(&doc(), Some(("m", "short")), true);
        assert!(out.contains("## Summary\n\nshort"));
        assert!(out.contains("body text"));
        assert!(out.contains("title: \"T \\\"q\\\"\""));
    }

    #[test]
    fn no_summary_still_valid() {
        let out = render(&doc(), None, true);
        assert!(!out.contains("## Summary") && out.contains("body text"));
    }
}
