use super::Out;
use super::tables::{self, Shape};
use crate::profile::group;
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
    let mut shapes: Vec<Shape> = Vec::new();
    for (name, hidden) in sheets {
        let Ok(range) = wb.worksheet_range(&name) else {
            continue;
        };
        let width = range.width();
        let text = |r: &[Data]| -> Vec<String> {
            (0..width)
                .map(|i| r.get(i).map(cell_text).unwrap_or_default())
                .collect()
        };
        let first: Vec<Vec<String>> = range.rows().take(11).map(text).collect();
        if first.is_empty() {
            continue;
        }
        let mut shape = Shape::new(name.clone(), width);
        shape.hidden = hidden;
        // Rows above a header row (titles, notes) stay in the body as text.
        let header_at = tables::find_header(&first, width);
        let start = header_at.map_or(0, |k| k + 1);
        let mut body = String::new();
        for r in &first[..header_at.unwrap_or(0)] {
            let line: Vec<&str> = r
                .iter()
                .map(|c| c.trim())
                .filter(|c| !c.is_empty())
                .collect();
            if !line.is_empty() {
                body.push_str(&format!("{}\n\n", line.join(" · ")));
            }
        }
        let header = match header_at {
            Some(k) => {
                let names = tables::names(&first[k]);
                shape.columns = Some(names.clone());
                names
            }
            None => tables::by_position(width),
        };
        shape.rows = range.height().saturating_sub(start);
        shape.kept = shape.rows.min(max_rows);
        let mut table = format!(
            "| {} |\n|{}\n",
            header
                .iter()
                .map(|c| c.replace('|', "\\|"))
                .collect::<Vec<_>>()
                .join(" | "),
            " --- |".repeat(width)
        );
        for row in range.rows().skip(start).take(shape.kept) {
            table.push_str(&format!("| {} |\n", text(row).join(" | ")));
        }
        if shape.kept < shape.rows {
            omitted.push(format!(
                "{} rows of sheet \"{name}\" beyond the {}-row limit",
                group(&(shape.rows - shape.kept).to_string()),
                group(&max_rows.to_string())
            ));
        }
        let label = if hidden {
            format!("{name} (hidden sheet)")
        } else {
            name.clone()
        };
        md.push_str(&format!("## Sheet: {label}\n\n{body}{table}\n"));
        shapes.push(shape);
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
    // The summary describes the structure only: sheets, sizes and column names.
    let mut out = Out::new(fallback, md)
        .with_summary(tables::summary("Spreadsheet", &shapes))
        .with_description(Some(tables::description("Spreadsheet", &shapes)));
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
