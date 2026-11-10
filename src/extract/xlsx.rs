use super::Out;
use crate::profile::{Column, group};
use anyhow::{Context, Result};
use calamine::{Data, Reader, SheetVisible, open_workbook_auto_from_rs};
use std::io::Cursor;

/// Days from 1970-01-01 to a calendar date (Howard Hinnant's algorithm, inverted).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Excel stores dates as days since 1899-12-30 (the 1900 date system).
pub fn serial_to_iso(serial: f64) -> String {
    let days = serial.floor() as i64;
    let secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    let time = format!("{:02}:{:02}", secs / 3600, secs / 60 % 60);
    if days == 0 {
        return time;
    }
    let (y, m, d) = civil_from_days(days - 25_569);
    if secs == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {time}")
    }
}

fn duration(serial: f64) -> String {
    let mins = (serial * 1440.0).round() as i64;
    format!("{}:{:02}", mins / 60, mins % 60)
}

/// Text for a cell, with dates shown as dates rather than serial numbers.
fn cell_text(c: &Data) -> String {
    let s = match c {
        Data::DateTime(dt) if dt.is_duration() => duration(dt.as_f64()),
        Data::DateTime(dt) => serial_to_iso(dt.as_f64()),
        other => other.to_string(),
    };
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn add_cell(col: &mut Column, cell: &Data) {
    match cell {
        Data::Empty => {}
        Data::Float(f) => col.add_number(*f),
        Data::Int(i) => col.add_number(*i as f64),
        Data::Bool(b) => col.add_bool(*b),
        Data::DateTime(dt) if dt.is_duration() => col.add_text(&duration(dt.as_f64())),
        Data::DateTime(dt) => col.add_date(&serial_to_iso(dt.as_f64())),
        Data::DateTimeIso(s) => col.add_date(s),
        other => col.add_text(&other.to_string()),
    }
}

/// Charts and pictures live in separate parts of the workbook archive; count them for the report.
fn count_parts(bytes: &[u8], prefix: &str) -> usize {
    zip::ZipArchive::new(Cursor::new(bytes)).map_or(0, |z| {
        z.file_names()
            .filter(|n| n.as_ref().is_ok_and(|n| n.starts_with(prefix)))
            .count()
    })
}

pub fn convert(bytes: &[u8], fallback: &str, max_rows: usize) -> Result<Out> {
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec())).context(
        "could not read this spreadsheet (it may be encrypted, damaged, or from Excel 5 or older)",
    )?;
    let sheets: Vec<(String, bool)> = wb
        .sheets_metadata()
        .iter()
        .map(|s| (s.name.clone(), s.visible != SheetVisible::Visible))
        .collect();
    let mut md = String::new();
    let mut omitted: Vec<String> = Vec::new();
    for (name, hidden) in sheets {
        let Ok(range) = wb.worksheet_range(&name) else {
            continue;
        };
        let mut rows = range.rows();
        let Some(header) = rows.next() else { continue };
        let width = range.width();
        let total = range.height().saturating_sub(1);
        let kept = total.min(max_rows);
        let mut cols: Vec<Column> = (0..width)
            .map(|i| {
                let h = header.get(i).map(cell_text).unwrap_or_default();
                Column::new(
                    if h.trim().is_empty() {
                        format!("col {}", i + 1)
                    } else {
                        h
                    },
                    "Column",
                )
            })
            .collect();
        let mut table = format!(
            "| {} |\n|{}\n",
            cols.iter()
                .map(|c| c.name.replace('|', "\\|"))
                .collect::<Vec<_>>()
                .join(" | "),
            " --- |".repeat(width)
        );
        for row in rows.take(kept) {
            for (i, col) in cols.iter_mut().enumerate() {
                if let Some(c) = row.get(i) {
                    add_cell(col, c);
                }
            }
            table.push_str(&format!(
                "| {} |\n",
                (0..width)
                    .map(|i| row.get(i).map(cell_text).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        let label = if hidden {
            format!("{name} (hidden sheet)")
        } else {
            name.clone()
        };
        md.push_str(&format!("## Sheet: {label}\n\n"));
        md.push_str(&format!(
            "Sheet \"{name}\" has {} row{} and {width} column{}",
            group(&total.to_string()),
            if total == 1 { "" } else { "s" },
            if width == 1 { "" } else { "s" }
        ));
        if kept < total {
            md.push_str(&format!(
                "; the first {} are included below and in the statistics",
                group(&kept.to_string())
            ));
            omitted.push(format!(
                "{} rows of sheet \"{name}\" beyond the {}-row limit",
                group(&(total - kept).to_string()),
                group(&max_rows.to_string())
            ));
        }
        md.push_str(".\n\n");
        if hidden {
            md.push_str(&format!("Sheet \"{name}\" is hidden in the workbook.\n\n"));
        }
        for c in &cols {
            md.push_str(&format!("- {}\n", c.describe(kept)));
        }
        md.push('\n');
        md.push_str(&table);
        md.push('\n');
    }
    if md.is_empty() {
        anyhow::bail!("spreadsheet has no data");
    }
    let charts = count_parts(bytes, "xl/charts/chart");
    let images = count_parts(bytes, "xl/media/");
    if charts > 0 {
        omitted.push(format!(
            "{charts} chart{}",
            if charts == 1 { "" } else { "s" }
        ));
    }
    if images > 0 {
        omitted.push(format!(
            "{images} image{}",
            if images == 1 { "" } else { "s" }
        ));
    }
    let mut out = Out::new(fallback, md);
    out.omitted = omitted;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excel_serial_numbers_become_dates() {
        assert_eq!(serial_to_iso(46336.0), "2026-11-10");
        assert_eq!(serial_to_iso(1.0), "1899-12-31");
        assert_eq!(serial_to_iso(45000.75), "2023-03-15 18:00");
        assert_eq!(serial_to_iso(0.5), "12:00");
    }
}
