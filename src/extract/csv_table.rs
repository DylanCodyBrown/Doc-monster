use super::Out;
use crate::profile::{Column, group};
use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;

/// Stream a CSV/TSV file: only the first `max_rows` data rows are ever read.
pub fn convert(path: &Path, fallback: &str, max_rows: usize) -> Result<Out> {
    let delimiter = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("tsv") => b'\t',
        Some("psv") => b'|',
        _ => b',',
    };
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    convert_reader(file, delimiter, fallback, max_rows)
}

pub fn convert_reader(
    reader: impl Read,
    delimiter: u8,
    fallback: &str,
    max_rows: usize,
) -> Result<Out> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(reader);

    // Header row + max_rows data rows + one more to learn whether the file goes on.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut more = false;
    for rec in reader.records() {
        let rec = rec?;
        if rows.len() > max_rows {
            more = true;
            break;
        }
        rows.push(rec.iter().map(escape_cell).collect());
    }

    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = Out::new(fallback, String::new());
    if width == 0 {
        return Ok(out);
    }
    let mut md = String::new();
    if rows.len() > 1 {
        let n = rows.len() - 1;
        if more {
            md.push_str(&format!(
                "Table \"{fallback}\" has more than {} rows and {width} columns; the first {} are included below and in the statistics.\n\n",
                group(&n.to_string()),
                group(&n.to_string())
            ));
            out = out.omitting(format!(
                "rows after the first {} (limit set with --max-rows)",
                group(&n.to_string())
            ));
        } else {
            md.push_str(&format!(
                "Table \"{fallback}\" has {} rows and {width} columns.\n\n",
                group(&n.to_string())
            ));
        }
        for c in 0..width {
            let name = rows[0]
                .get(c)
                .filter(|h| !h.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("col {}", c + 1));
            let mut col = Column::new(name, "Column");
            for row in &rows[1..] {
                if let Some(cell) = row.get(c) {
                    col.add_raw(cell);
                }
            }
            md.push_str(&format!("- {}\n", col.describe(n)));
        }
        md.push('\n');
    }
    for (i, row) in rows.iter().enumerate() {
        md.push('|');
        for c in 0..width {
            md.push(' ');
            md.push_str(row.get(c).map(String::as_str).unwrap_or(""));
            md.push_str(" |");
        }
        md.push('\n');
        if i == 0 {
            md.push('|');
            md.push_str(&" --- |".repeat(width));
            md.push('\n');
        }
    }
    out.markdown = md;
    Ok(out)
}

fn escape_cell(cell: &str) -> String {
    cell.replace('|', "\\|").replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(src: &[u8], max: usize) -> Out {
        convert_reader(src, b',', "x", max).unwrap()
    }

    #[test]
    fn builds_markdown_table() {
        let out = md(b"a,b\n1,2\n", 10);
        assert!(
            out.markdown
                .ends_with("| a | b |\n| --- | --- |\n| 1 | 2 |\n"),
            "{}",
            out.markdown
        );
        assert!(out.markdown.contains("Column \"b\" is numeric"));
        assert!(out.omitted.is_empty());
    }

    #[test]
    fn escapes_pipes() {
        assert!(md(b"a\nx|y\n", 10).markdown.contains("x\\|y"));
    }

    #[test]
    fn stops_reading_at_the_row_limit_and_says_so() {
        let out = md(b"n\n1\n2\n3\n4\n5\n", 2);
        assert!(
            out.markdown.contains("| 2 |") && !out.markdown.contains("| 3 |"),
            "{}",
            out.markdown
        );
        assert!(out.markdown.contains("more than 2 rows"));
        assert!(out.omitted[0].contains("rows after the first 2"));
    }
}
