use crate::Document;

/// Final Markdown file: a minimal front matter (title, description, and a comment saying how it
/// was made), then the summary (if any), then the body: the full cleaned document at compression
/// level 1, a shortened one at levels 2-9, none at level 10 (`body` is `None`).
pub fn render(
    doc: &Document,
    description: &str,
    summary: Option<&str>,
    body: Option<&str>,
    level: u8,
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(if summary.is_some() {
        "# Condensed with Doc Monster"
    } else {
        "# Converted with Doc Monster"
    });
    if level > 1 {
        out.push_str(&format!(", compression {level} of 10"));
    }
    out.push('\n');
    out.push_str(&format!("title: {}\n", yaml_string(&doc.title)));
    out.push_str(&format!("description: {}\n", yaml_string(description)));
    out.push_str("---\n\n");
    if !doc.omitted.is_empty() {
        out.push_str(&format!(
            "> **Not included:** {}.\n\n",
            doc.omitted.join("; ")
        ));
    }
    if let Some(text) = summary {
        out.push_str("## Summary\n\n");
        out.push_str(text.trim());
        out.push('\n');
        if body.is_some() {
            out.push_str(if level == 1 {
                "\n---\n\n## Full content\n\n"
            } else {
                "\n---\n\n## Compressed content\n\n"
            });
        }
    }
    if let Some(b) = body {
        out.push_str(b.trim_end());
        out.push('\n');
    }
    out
}

/// One line, at most 200 characters, no Markdown markup.
pub fn tidy_description(s: &str) -> String {
    let plain = crate::digest::plain_text(s);
    let one_line = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= 200 {
        return one_line;
    }
    let cut: String = one_line.chars().take(197).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{}…", cut.trim_end_matches([',', ';', ':', ' ']))
}

fn yaml_string(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\n', '\r'], " ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Format;

    fn doc() -> Document {
        Document {
            title: "T \"q\"".into(),
            description: None,
            summary: None,
            omitted: Vec::new(),
            source: "a.txt".into(),
            format: Format::PlainText,
            markdown: "body text\n".into(),
        }
    }

    #[test]
    fn front_matter_is_title_description_and_comment_only() {
        let out = render(&doc(), "About it.", Some("short"), Some("body text\n"), 1);
        assert!(out.starts_with("---\n# Condensed with Doc Monster\ntitle: \"T \\\"q\\\"\"\ndescription: \"About it.\"\n---\n\n"), "{out}");
        assert!(out.contains("## Summary\n\nshort") && out.contains("body text"));
        assert_eq!(
            out.matches("\n---\n").count(),
            2,
            "front matter close + summary divider: {out}"
        );
        assert!(!out.contains("source:") && !out.contains("format:"));
    }

    #[test]
    fn level_ten_omits_the_body_and_no_summary_says_converted() {
        let out = render(&doc(), "d", Some("short"), None, 10);
        assert!(!out.contains("body text") && !out.contains("Full content"));
        assert!(
            out.starts_with("---\n# Condensed with Doc Monster, compression 10 of 10\n"),
            "{out}"
        );
        let out = render(&doc(), "d", None, Some("body text\n"), 1);
        assert!(
            out.contains("# Converted with Doc Monster")
                && out.contains("body text")
                && !out.contains("## Summary")
        );
    }

    #[test]
    fn description_is_one_plain_line_under_200_characters() {
        let long = format!("**Bold** start. {}", "word ".repeat(80));
        let d = tidy_description(&long);
        assert!(
            d.chars().count() <= 200 && d.ends_with('…') && d.starts_with("Bold start."),
            "{d}"
        );
        assert_eq!(tidy_description("a\nb  [x](http://y)"), "a b x");
    }
}
