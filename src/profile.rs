//! Statistics about one column or field, written as plain sentences so the digest can use them.

use std::collections::HashMap;

const MAX_DISTINCT_TRACKED: usize = 5000;

#[derive(Default)]
pub struct Column {
    pub name: String,
    /// "Column" for tables, "Field" for data structures.
    pub noun: &'static str,
    pub filled: usize,
    numbers: usize,
    sum: f64,
    min: f64,
    max: f64,
    bools: usize,
    trues: usize,
    texts: HashMap<String, usize>,
    overflow: bool,
    dates: usize,
    first_date: String,
    last_date: String,
}

impl Column {
    pub fn new(name: impl Into<String>, noun: &'static str) -> Column {
        Column {
            name: name.into(),
            noun,
            ..Default::default()
        }
    }

    pub fn add_number(&mut self, v: f64) {
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

    /// A date or time already written as ISO text ("2026-11-10"), which sorts correctly as text.
    pub fn add_date(&mut self, iso: &str) {
        if self.dates == 0 || iso < self.first_date.as_str() {
            self.first_date = iso.to_string();
        }
        if self.dates == 0 || iso > self.last_date.as_str() {
            self.last_date = iso.to_string();
        }
        self.dates += 1;
        self.filled += 1;
    }

    pub fn add_bool(&mut self, b: bool) {
        self.bools += 1;
        self.trues += usize::from(b);
        self.filled += 1;
    }

    pub fn add_text(&mut self, s: &str) {
        self.filled += 1;
        if self.texts.len() < MAX_DISTINCT_TRACKED || self.texts.contains_key(s) {
            *self.texts.entry(s.to_string()).or_default() += 1;
        } else {
            self.overflow = true;
        }
    }

    /// Classify a raw text cell (as found in CSV files) and record it.
    pub fn add_raw(&mut self, s: &str) {
        let t = s.trim();
        if t.is_empty() {
            return;
        }
        if let Ok(v) = t.replace(',', "").parse::<f64>()
            && v.is_finite()
        {
            return self.add_number(v);
        }
        match t.to_ascii_lowercase().as_str() {
            "true" | "false" => self.add_bool(t.eq_ignore_ascii_case("true")),
            _ => self.add_text(t),
        }
    }

    pub fn describe(&self, rows: usize) -> String {
        let (noun, name) = (self.noun, &self.name);
        if self.filled == 0 {
            return format!("{noun} \"{name}\" is empty.");
        }
        let kinds = usize::from(self.numbers > 0)
            + usize::from(self.bools > 0)
            + usize::from(!self.texts.is_empty())
            + usize::from(self.dates > 0);
        if self.dates * 2 >= self.filled {
            return if self.first_date == self.last_date {
                format!("{noun} \"{name}\" holds the date {}.", self.first_date)
            } else {
                format!(
                    "{noun} \"{name}\" holds dates from {} to {}.",
                    self.first_date, self.last_date
                )
            };
        }
        if self.numbers * 2 >= self.filled {
            let mut s = format!(
                "{noun} \"{name}\" is numeric: smallest {}, largest {}, average {}, total {}.",
                num(self.min),
                num(self.max),
                num(self.sum / self.numbers as f64),
                num(self.sum)
            );
            if kinds > 1 {
                s.push_str(" Some values are not numbers.");
            }
            return s;
        }
        if self.bools * 2 >= self.filled {
            let unit = if noun == "Column" { "rows" } else { "values" };
            return format!(
                "{noun} \"{name}\" is true in {:.1}% of {} {unit}.",
                100.0 * self.trues as f64 / self.bools as f64,
                group(&self.bools.to_string())
            );
        }
        let mut top: Vec<(&String, &usize)> = self.texts.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let shown: Vec<String> = top
            .iter()
            .take(3)
            .map(|(v, c)| format!("{} ({})", clip(v, 40), group(&c.to_string())))
            .collect();
        let distinct = if self.overflow {
            format!("over {MAX_DISTINCT_TRACKED}")
        } else {
            group(&self.texts.len().to_string())
        };
        let values = if self.texts.len() == 1 && !self.overflow {
            "value"
        } else {
            "values"
        };
        let fill = if self.filled < rows {
            format!(
                ", filled in {} of {} rows",
                self.filled,
                group(&rows.to_string())
            )
        } else {
            String::new()
        };
        format!(
            "{noun} \"{name}\" has {distinct} distinct {values}{fill}; most common: {}.",
            shown.join(", ")
        )
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

pub fn group(digits: &str) -> String {
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn num(v: f64) -> String {
    let text = if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v.abs() as i64)
    } else {
        format!("{:.2}", v.abs())
    };
    let (int, frac) = text
        .split_once('.')
        .map_or((text.as_str(), None), |(a, b)| (a, Some(b)));
    let sign = if v < 0.0 { "-" } else { "" };
    match frac {
        Some(f) => format!("{sign}{}.{f}", group(int)),
        None => format!("{sign}{}", group(int)),
    }
}
