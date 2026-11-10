use crate::profile::{Column, group};
use anyhow::{Context, Result};
use calamine::{Data, Reader, open_workbook_auto_from_rs};
use std::io::Cursor;

fn add_cell(col: &mut Column, cell: &Data) {
    match cell {
        Data::Empty => {}
        Data::Float(f) => col.add_number(*f),
        Data::Int(i) => col.add_number(*i as f64),
        Data::Bool(b) => col.add_bool(*b),
        other => col.add_text(&other.to_string()),
    }
}

fn cell_text(c: &Data) -> String {
    c.to_string().replace('|', "\\|").replace(['\n', '\r'], " ")
}

pub fn convert(bytes: &[u8], fallback: &str) -> Result<(String, String)> {
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec())).context("not a readable spreadsheet")?;
    let mut md = String::new();
    for name in wb.sheet_names() {
        let range = match wb.worksheet_range(&name) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut rows = range.rows();
        let Some(header) = rows.next() else { continue };
        let width = range.width();
        let mut cols: Vec<Column> = (0..width)
            .map(|i| {
                let h = header.get(i).map(|c| c.to_string()).unwrap_or_default();
                Column::new(if h.trim().is_empty() { format!("col {}", i + 1) } else { h }, "Column")
            })
            .collect();
        let mut table = String::new();
        table.push_str(&format!("| {} |\n|{}\n", cols.iter().map(|c| c.name.replace('|', "\\|")).collect::<Vec<_>>().join(" | "), " --- |".repeat(width)));
        let mut n = 0usize;
        for row in rows {
            n += 1;
            for (i, col) in cols.iter_mut().enumerate() {
                if let Some(c) = row.get(i) {
                    add_cell(col, c);
                }
            }
            table.push_str(&format!("| {} |\n", (0..width).map(|i| row.get(i).map(cell_text).unwrap_or_default()).collect::<Vec<_>>().join(" | ")));
        }
        md.push_str(&format!("## Sheet: {name}\n\n"));
        md.push_str(&format!("Sheet \"{name}\" has {} rows and {width} columns.\n\n", group(&n.to_string())));
        for c in &cols {
            md.push_str(&format!("- {}\n", c.describe(n)));
        }
        md.push('\n');
        md.push_str(&table);
        md.push('\n');
    }
    if md.is_empty() {
        anyhow::bail!("spreadsheet has no data");
    }
    Ok((fallback.to_string(), md))
}
