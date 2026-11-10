use super::Out;
use super::tables::{self, Shape};
use crate::profile::group;
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
    // The summary describes the structure only: size and column names, never the values.
    for r in rows.iter_mut() {
        r.resize(width, String::new());
    }
    let header_at = tables::find_header(&rows[..rows.len().min(11)], width);
    let start = header_at.map_or(0, |k| k + 1);
    let mut shape = Shape::new(fallback.to_string(), width);
    shape.columns = header_at.map(|k| tables::names(&rows[k]));
    shape.rows = rows.len() - start;
    shape.kept = shape.rows;
    shape.more = more;
    if more {
        out = out.omitting(format!(
            "rows after the first {} (limit set with --max-rows)",
            group(&shape.rows.to_string())
        ));
    }
    let kind = match delimiter {
        b'\t' => "TSV file",
        _ => "CSV file",
    };
    let shapes = [shape];
    out = out
        .with_summary(tables::summary(kind, &shapes))
        .with_description(Some(tables::description(kind, &shapes)));
    let mut md = String::new();
    // Rows above a header row (titles, notes) stay in the body as text.
    for r in &rows[..header_at.unwrap_or(0)] {
        let line: Vec<&str> = r
            .iter()
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .collect();
        if !line.is_empty() {
            md.push_str(&format!("{}\n\n", line.join(" · ")));
        }
    }
    let header = match header_at {
        Some(k) => rows[k].clone(),
        None => tables::by_position(width),
    };
    md.push_str(&format!(
        "| {} |\n|{}\n",
        header.join(" | "),
        " --- |".repeat(width)
    ));
    for row in &rows[start..] {
        md.push_str(&format!("| {} |\n", row.join(" | ")));
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
        // Structure only: no statistics about the values anywhere.
        assert!(!out.markdown.contains("numeric"), "{}", out.markdown);
        assert_eq!(
            out.description.as_deref(),
            Some("CSV file, 1 row × 2 columns; columns a, b.")
        );
        assert!(out.summary.unwrap().contains("Columns: `a`, `b`"));
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
        assert!(out.summary.unwrap().contains("more than 2 rows"));
        assert!(out.omitted[0].contains("rows after the first 2"));
    }
}
