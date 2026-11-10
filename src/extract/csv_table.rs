use crate::profile::{Column, group};
use anyhow::Result;
use std::path::Path;

pub fn convert(bytes: &[u8], path: &Path, fallback: &str) -> Result<(String, String)> {
    let tab = path.extension().and_then(|e| e.to_str()) == Some("tsv");
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(if tab { b'\t' } else { b',' })
        .from_reader(bytes);

    let rows: Vec<Vec<String>> = reader
        .records()
        .map(|r| r.map(|rec| rec.iter().map(escape_cell).collect()))
        .collect::<std::result::Result<_, _>>()?;

    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return Ok((fallback.to_string(), String::new()));
    }

    let mut out = String::new();
    // Profile each column so the summary can say something about the data itself.
    if rows.len() > 1 {
        let n = rows.len() - 1;
        out.push_str(&format!("Table \"{fallback}\" has {} rows and {width} columns.\n\n", group(&n.to_string())));
        for c in 0..width {
            let name = rows[0].get(c).filter(|h| !h.trim().is_empty()).cloned().unwrap_or_else(|| format!("col {}", c + 1));
            let mut col = Column::new(name, "Column");
            for row in &rows[1..] {
                if let Some(cell) = row.get(c) {
                    col.add_raw(cell);
                }
            }
            out.push_str(&format!("- {}\n", col.describe(n)));
        }
        out.push('\n');
    }
    for (i, row) in rows.iter().enumerate() {
        out.push('|');
        for c in 0..width {
            out.push(' ');
            out.push_str(row.get(c).map(String::as_str).unwrap_or(""));
            out.push_str(" |");
        }
        out.push('\n');
        if i == 0 {
            out.push('|');
            out.push_str(&" --- |".repeat(width));
            out.push('\n');
        }
    }
    Ok((fallback.to_string(), out))
}

fn escape_cell(cell: &str) -> String {
    cell.replace('|', "\\|").replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_markdown_table() {
        let (_, md) = convert(b"a,b\n1,2\n", Path::new("x.csv"), "x").unwrap();
        assert!(md.ends_with("| a | b |\n| --- | --- |\n| 1 | 2 |\n"), "{md}");
        assert!(md.contains("Column \"b\" is numeric"));
    }

    #[test]
    fn escapes_pipes() {
        let (_, md) = convert(b"a\nx|y\n", Path::new("x.csv"), "x").unwrap();
        assert!(md.contains("x\\|y"));
    }
}
