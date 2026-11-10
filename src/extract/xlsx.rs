use anyhow::{Context, Result};
use calamine::{Data, Reader, open_workbook_auto_from_rs};
use std::collections::HashMap;
use std::io::Cursor;

const MAX_DISTINCT_TRACKED: usize = 5000;

#[derive(Default)]
struct Column {
    name: String,
    filled: usize,
    numbers: usize,
    sum: f64,
    min: f64,
    max: f64,
    bools: usize,
    trues: usize,
    texts: HashMap<String, usize>,
    overflow: bool,
}

impl Column {
    fn add(&mut self, cell: &Data) {
        match cell {
            Data::Empty => {}
            Data::Float(_) | Data::Int(_) => {
                let v = match cell {
                    Data::Float(f) => *f,
                    Data::Int(i) => *i as f64,
                    _ => unreachable!(),
                };
                if self.numbers == 0 {
                    self.min = v;
                    self.max = v;
                } else {
                    self.min = self.min.min(v);
                    self.max = self.max.max(v);
                }
                self.numbers += 1;
                self.sum += v;
                self.filled += 1;
            }
            Data::Bool(b) => {
                self.bools += 1;
                self.trues += usize::from(*b);
                self.filled += 1;
            }
            other => {
                self.filled += 1;
                let s = other.to_string();
                if self.texts.len() < MAX_DISTINCT_TRACKED || self.texts.contains_key(&s) {
                    *self.texts.entry(s).or_default() += 1;
                } else {
                    self.overflow = true;
                }
            }
        }
    }

    fn describe(&self, rows: usize) -> String {
        let name = &self.name;
        if self.filled == 0 {
            return format!("Column \"{name}\" is empty.");
        }
        if self.numbers * 2 >= self.filled {
            return format!(
                "Column \"{name}\" is numeric: smallest {}, largest {}, average {}, total {}.",
                num(self.min),
                num(self.max),
                num(self.sum / self.numbers as f64),
                num(self.sum)
            );
        }
        if self.bools * 2 >= self.filled {
            return format!(
                "Column \"{name}\" is true in {:.1}% of {} rows.",
                100.0 * self.trues as f64 / self.bools as f64,
                self.bools
            );
        }
        let mut top: Vec<(&String, &usize)> = self.texts.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let shown: Vec<String> = top.iter().take(3).map(|(v, c)| format!("{v} ({c})")).collect();
        let distinct = if self.overflow { format!("over {MAX_DISTINCT_TRACKED}") } else { self.texts.len().to_string() };
        let fill = if self.filled < rows { format!(", filled in {} of {rows} rows", self.filled) } else { String::new() };
        format!("Column \"{name}\" has {distinct} distinct values{fill}; most common: {}.", shown.join(", "))
    }
}

fn group(digits: &str) -> String {
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn num(v: f64) -> String {
    let text = if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v.abs() as i64) } else { format!("{:.2}", v.abs()) };
    let (int, frac) = text.split_once('.').map_or((text.as_str(), None), |(a, b)| (a, Some(b)));
    let sign = if v < 0.0 { "-" } else { "" };
    match frac {
        Some(f) => format!("{sign}{}.{f}", group(int)),
        None => format!("{sign}{}", group(int)),
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
                Column { name: if h.trim().is_empty() { format!("col {}", i + 1) } else { h }, ..Default::default() }
            })
            .collect();
        let mut table = String::new();
        table.push_str(&format!("| {} |\n|{}\n", cols.iter().map(|c| c.name.replace('|', "\\|")).collect::<Vec<_>>().join(" | "), " --- |".repeat(width)));
        let mut n = 0usize;
        for row in rows {
            n += 1;
            for (i, col) in cols.iter_mut().enumerate() {
                if let Some(c) = row.get(i) {
                    col.add(c);
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
