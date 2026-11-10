//! The summary and description of spreadsheets and CSV files: their structure only (tables,
//! sizes, column names), never what the cells contain.

use crate::profile::group;

/// One table: a sheet of a workbook, or a whole CSV file.
pub struct Shape {
    pub name: String,
    pub hidden: bool,
    pub width: usize,
    /// Column names, only when the table has a real header row.
    pub columns: Option<Vec<String>>,
    /// Data rows (the header row and any title rows above it not counted). For CSV files cut
    /// off by the row limit, the number read: the true total is unknown, which `more` says.
    pub rows: usize,
    pub more: bool,
    /// Rows included in the output (the row limit can cut a table short).
    pub kept: usize,
}

impl Shape {
    pub fn new(name: String, width: usize) -> Shape {
        Shape {
            name,
            hidden: false,
            width,
            columns: None,
            rows: 0,
            more: false,
            kept: 0,
        }
    }

    fn size(&self) -> String {
        let rows = if self.more {
            format!("more than {} rows", group(&self.rows.to_string()))
        } else {
            plural(self.rows, "row")
        };
        let header = if self.columns.is_some() {
            ""
        } else {
            ", no header row"
        };
        format!("{rows} × {}{header}", plural(self.width, "column"))
    }
}

/// Column names from a header row: cells arrive escaped for Markdown tables and are shown
/// unescaped; blank ones are named by position.
pub fn names(header: &[String]) -> Vec<String> {
    header
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let h = h.replace("\\|", "|");
            let h = h.split_whitespace().collect::<Vec<_>>().join(" ");
            if h.is_empty() {
                format!("column {}", i + 1)
            } else {
                h
            }
        })
        .collect()
}

/// Names for a table without a header row: "column 1", "column 2", ...
pub fn by_position(width: usize) -> Vec<String> {
    (1..=width).map(|i| format!("column {i}")).collect()
}

/// A cell that holds a value (number, date, time, true or false) rather than a name.
fn is_value(cell: &str) -> bool {
    let c = cell.trim();
    let digits = c
        .trim_start_matches(['-', '+', '$', '€', '£'])
        .trim_end_matches('%');
    !digits.is_empty() && digits.replace(',', "").parse::<f64>().is_ok()
        || c.len() >= 8
            && c.as_bytes()[..4].iter().all(u8::is_ascii_digit)
            && c.as_bytes()[4] == b'-'
        || c.len() >= 4 && c.contains(':') && c.chars().all(|ch| ch.is_ascii_digit() || ch == ':')
        || c.eq_ignore_ascii_case("true")
        || c.eq_ignore_ascii_case("false")
}

/// Which of the first rows holds column names, if any. Title rows may sit above the header;
/// the search stops at the first row holding a value, because that row is already data. A
/// header is text only, fills at least half the width, has no repeated names, short names,
/// and at least one row below it.
pub fn find_header(first_rows: &[Vec<String>], width: usize) -> Option<usize> {
    for (i, row) in first_rows.iter().enumerate().take(10) {
        let filled: Vec<&str> = row
            .iter()
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .collect();
        if filled.iter().any(|c| is_value(c)) {
            return None;
        }
        if i + 1 >= first_rows.len() || filled.is_empty() {
            continue;
        }
        let mut seen = std::collections::HashSet::new();
        let unique = filled.iter().all(|c| seen.insert(c.to_lowercase()));
        // Labels ("Comment:") and sentences ("Tests for the function.") are not names.
        let short = filled.iter().all(|c| {
            let words = c.split_whitespace().count();
            c.chars().count() <= 50
                && words <= 6
                && !c.ends_with(':')
                && !(c.ends_with('.') && words >= 3)
        });
        let wide = filled.len() * 2 >= width && (filled.len() >= 2 || width == 1);
        if unique && short && wide {
            return Some(i);
        }
    }
    None
}

fn plural(n: usize, word: &str) -> String {
    format!(
        "{} {word}{}",
        group(&n.to_string()),
        if n == 1 { "" } else { "s" }
    )
}

/// `kind` is "Spreadsheet" or "CSV file" (or "TSV file").
pub fn summary(kind: &str, tables: &[Shape]) -> String {
    let rows: usize = tables.iter().map(|t| t.rows).sum();
    let mut s = String::from("### At a glance\n");
    let mut glance = vec![kind.to_string()];
    if tables.len() > 1 {
        glance.push(plural(tables.len(), "sheet"));
    }
    glance.push(if tables.iter().any(|t| t.more) {
        format!("more than {}", plural(rows, "row"))
    } else {
        plural(rows, "row")
    });
    s.push_str(&glance.join(" · "));
    s.push_str("\n\n### Tables\n");
    for t in tables {
        let hidden = if t.hidden { " (hidden sheet)" } else { "" };
        s.push_str(&format!("- **{}**{hidden}: {}", t.name, t.size()));
        if t.kept < t.rows || t.more {
            s.push_str(&format!(
                "; the first {} are included",
                group(&t.kept.to_string())
            ));
        }
        s.push('\n');
        if let Some(columns) = &t.columns {
            let names: Vec<String> = columns.iter().map(|c| code(c)).collect();
            s.push_str(&format!("  - Columns: {}\n", names.join(", ")));
        }
    }
    s
}

/// One line: the kind, the tables, and their first column names.
pub fn description(kind: &str, tables: &[Shape]) -> String {
    const COLS: usize = 8;
    let one = |t: &Shape| {
        let Some(columns) = &t.columns else {
            return t.size();
        };
        let list: Vec<&str> = columns.iter().take(COLS).map(String::as_str).collect();
        let rest = columns.len().saturating_sub(COLS);
        let more = if rest > 0 {
            format!(" and {rest} more")
        } else {
            String::new()
        };
        format!("{}; columns {}{more}", t.size(), list.join(", "))
    };
    match tables {
        [t] => format!("{kind}, {}.", one(t)),
        _ => {
            let parts: Vec<String> = tables
                .iter()
                .map(|t| format!("\"{}\" ({})", t.name, one(t)))
                .collect();
            format!(
                "{kind} with {}: {}.",
                plural(tables.len(), "sheet"),
                parts.join("; ")
            )
        }
    }
}

/// A column name in backticks, safe even when the name contains backticks.
fn code(name: &str) -> String {
    if name.contains('`') {
        format!("`` {name} ``")
    } else {
        format!("`{name}`")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| c.to_string()).collect()
    }

    fn shape(name: &str, rows: usize, cols: &[&str]) -> Shape {
        let mut s = Shape::new(name.into(), cols.len());
        s.columns = Some(names(&row(cols)));
        s.rows = rows;
        s.kept = rows;
        s
    }

    #[test]
    fn describes_structure_and_column_names_only() {
        let t = [shape("Revenue", 3, &["Name", "Region", "", "Q1"])];
        assert_eq!(
            description("Spreadsheet", &t),
            "Spreadsheet, 3 rows × 4 columns; columns Name, Region, column 3, Q1."
        );
        let s = summary("Spreadsheet", &t);
        assert!(s.contains("- **Revenue**: 3 rows × 4 columns\n"), "{s}");
        assert!(
            s.contains("Columns: `Name`, `Region`, `column 3`, `Q1`"),
            "{s}"
        );
    }

    #[test]
    fn several_sheets_long_headers_and_row_limits() {
        let wide: Vec<String> = (1..=10).map(|i| format!("c{i}")).collect();
        let wide: Vec<&str> = wide.iter().map(String::as_str).collect();
        let mut big = shape("Data", 9000, &wide);
        big.kept = 5000;
        let mut notes = Shape::new("Notes".into(), 1);
        notes.rows = 1;
        notes.kept = 1;
        notes.hidden = true;
        let t = [big, notes];
        let d = description("Spreadsheet", &t);
        assert!(
            d.starts_with("Spreadsheet with 2 sheets: \"Data\" (9,000 rows"),
            "{d}"
        );
        assert!(d.contains("c8 and 2 more"), "{d}");
        assert!(
            d.contains("\"Notes\" (1 row × 1 column, no header row)"),
            "{d}"
        );
        let s = summary("Spreadsheet", &t);
        assert!(s.contains("Spreadsheet · 2 sheets · 9,001 rows"), "{s}");
        assert!(s.contains("the first 5,000 are included"), "{s}");
        assert!(s.contains("**Notes** (hidden sheet)"), "{s}");
    }

    #[test]
    fn finds_real_headers_and_never_takes_data_for_names() {
        // A plain header.
        let t = [
            row(&["Name", "Region", "Q1"]),
            row(&["Acme", "North", "300"]),
        ];
        assert_eq!(find_header(&t, 3), Some(0));
        // A title row above the header.
        let t = [
            row(&["PARTS LIST", "", ""]),
            row(&["Part", "Value", "Qty"]),
            row(&["C1", "1.2pF", "4"]),
        ];
        assert_eq!(find_header(&t, 3), Some(1));
        // The first row is already data: numbers, dates, or repeated values.
        let t = [
            row(&["0", "0", "123", "2018-02-01"]),
            row(&["1", "2", "3", "4"]),
        ];
        assert_eq!(find_header(&t, 4), None);
        let t = [
            row(&["Albatross", "2.99", "On a stick!"]),
            row(&["x", "1", "y"]),
        ];
        assert_eq!(find_header(&t, 3), None);
        let t = [row(&["Alice", "Alice"]), row(&["Bob", "Bob"])];
        assert_eq!(find_header(&t, 2), None);
        // A sentence is content, not a column name; a lone row has nothing below it.
        let t = [
            row(&["This sheet lists the test cases for the ROW function"]),
            row(&["a"]),
        ];
        assert_eq!(find_header(&t, 1), None);
        assert_eq!(find_header(&[row(&["Name", "Region"])], 2), None);
        let t = [
            row(&["Sheet Comment:", "Tests for the FIXED function."]),
            row(&["a", "b"]),
        ];
        assert_eq!(find_header(&t, 2), None);
        // Short abbreviations with a full stop are still names.
        let t = [row(&["No.", "Qty.", "Item"]), row(&["1", "4", "bolt"])];
        assert_eq!(find_header(&t, 3), Some(0));
    }
}
