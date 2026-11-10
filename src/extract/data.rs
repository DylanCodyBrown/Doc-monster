//! Structured data (JSON, YAML, TOML, XML, INI-style settings).
//!
//! Output is a plain-English description of the *shape* of the data (keys, list lengths,
//! value ranges, most common values) followed by the original source, so the digest has real
//! sentences to work with and nothing is lost.

use super::{Out, text};
use crate::profile::{Column, group};
use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Json,
    JsonLines,
    Yaml,
    Toml,
    Xml,
    Ini,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Json => "json",
            Kind::JsonLines => "json lines",
            Kind::Yaml => "yaml",
            Kind::Toml => "toml",
            Kind::Xml => "xml",
            Kind::Ini => "settings",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Kind::Json => "JSON",
            Kind::JsonLines => "JSON Lines",
            Kind::Yaml => "YAML",
            Kind::Toml => "TOML",
            Kind::Xml => "XML",
            Kind::Ini => "Settings",
        }
    }
    fn lang(self) -> &'static str {
        match self {
            Kind::Json | Kind::JsonLines => "json",
            Kind::Yaml => "yaml",
            Kind::Toml => "toml",
            Kind::Xml => "xml",
            Kind::Ini => "ini",
        }
    }
}

const MAX_LINES: usize = 80;
const MAX_DEPTH: usize = 7;
const MAX_KEYS_SHOWN: usize = 12;

pub fn convert(kind: Kind, bytes: &[u8], _path: &Path, fallback: &str) -> Result<Out> {
    let source = text::decode(bytes);
    let mut lines: Vec<String> = Vec::new();
    let mut shown_source = source.trim_end().to_string();

    let description: Option<String>;
    match kind {
        Kind::Xml => {
            let root = xml_outline(&source, &mut lines);
            description = root.map(|(name, n)| format!("XML file with root element <{name}> and {n} distinct element paths."));
        }
        _ => {
            let value = match kind {
                Kind::Json => {
                    let v = parse_json(&source)?;
                    // Strict JSON is re-printed in a consistent layout; JSON with comments is kept as written.
                    if serde_json::from_str::<Value>(&source).is_ok() {
                        shown_source = serde_json::to_string_pretty(&v)?;
                    }
                    v
                }
                Kind::JsonLines => parse_jsonl(&source)?,
                Kind::Yaml => parse_yaml(&source)?,
                Kind::Toml => parse_toml(&source)?,
                Kind::Ini => parse_ini(&source),
                Kind::Xml => unreachable!(),
            };
            description = describe_root(kind, &value);
            let mut o = Outline { lines: &mut lines, truncated: false };
            o.root(&value, kind == Kind::JsonLines);
            if o.truncated {
                lines.push("- More fields exist than are described here; see the source below.".into());
            }
        }
    }

    let longest = shown_source.split(|c| c != '`').map(str::len).max().unwrap_or(0).max(2);
    let fence = "`".repeat(longest + 1);
    // The description of the shape is the summary; the source is the full content.
    let md = format!("{fence}{}\n{shown_source}\n{fence}\n", kind.lang());
    Ok(Out::new(fallback, md).with_summary(lines.join("\n")).with_description(description))
}

// ---------------------------------------------------------------------------------------------
// Parsing into a common value model

fn strip_jsonc(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut in_str, mut esc) = (0, false, false);
    while i < c.len() {
        let ch = c[i];
        if in_str {
            out.push(ch);
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
        } else if ch == '"' {
            in_str = true;
            out.push(ch);
        } else if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if ch == ',' {
            // Drop trailing commas before a closing bracket.
            let next = c[i + 1..].iter().find(|x| !x.is_whitespace());
            if !matches!(next, Some('}' | ']')) {
                out.push(ch);
            }
        } else {
            out.push(ch);
        }
        i += 1;
    }
    out
}

fn parse_json(s: &str) -> Result<Value> {
    let s = s.trim_start_matches('\u{feff}');
    serde_json::from_str(s)
        .or_else(|_| serde_json::from_str(&strip_jsonc(s)))
        .context("invalid JSON (comments and trailing commas are accepted, other JSON5 syntax is not)")
}

fn parse_jsonl(s: &str) -> Result<Value> {
    let mut items = Vec::new();
    for (n, line) in s.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        items.push(serde_json::from_str(line).with_context(|| format!("invalid JSON on line {}", n + 1))?);
    }
    Ok(Value::Array(items))
}

fn yaml_to_json(v: serde_norway::Value) -> Value {
    use serde_norway::Value as Y;
    match v {
        Y::Null => Value::Null,
        Y::Bool(b) => Value::Bool(b),
        Y::Number(n) => n
            .as_i64()
            .map(Value::from)
            .or_else(|| n.as_u64().map(Value::from))
            .or_else(|| n.as_f64().and_then(serde_json::Number::from_f64).map(Value::Number))
            .unwrap_or(Value::Null),
        Y::String(s) => Value::String(s),
        Y::Sequence(s) => Value::Array(s.into_iter().map(yaml_to_json).collect()),
        Y::Mapping(m) => {
            let mut out = Map::new();
            for (k, v) in m {
                let key = match k {
                    Y::String(s) => s,
                    Y::Number(n) => n.to_string(),
                    Y::Bool(b) => b.to_string(),
                    Y::Null => "null".into(),
                    other => format!("{other:?}"),
                };
                out.insert(key, yaml_to_json(v));
            }
            Value::Object(out)
        }
        Y::Tagged(t) => yaml_to_json(t.value),
    }
}

fn parse_yaml(s: &str) -> Result<Value> {
    use serde::Deserialize;
    let mut docs = Vec::new();
    for d in serde_norway::Deserializer::from_str(s) {
        let v = serde_norway::Value::deserialize(d).context("invalid YAML")?;
        docs.push(yaml_to_json(v));
    }
    Ok(match docs.len() {
        0 => Value::Null,
        1 => docs.remove(0),
        _ => Value::Array(docs),
    })
}

fn toml_to_json(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::from(i),
        toml::Value::Float(f) => serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(t.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect()),
    }
}

fn parse_toml(s: &str) -> Result<Value> {
    let table: toml::Table = toml::from_str(s).context("invalid TOML")?;
    Ok(toml_to_json(toml::Value::Table(table)))
}

fn scalar(v: &str) -> Value {
    let t = v.trim().trim_matches(['"', '\'']);
    if let Ok(i) = t.parse::<i64>() {
        return Value::from(i);
    }
    if let Some(f) = t.parse::<f64>().ok().and_then(serde_json::Number::from_f64) {
        return Value::Number(f);
    }
    match t.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Value::Bool(true),
        "false" | "no" | "off" => Value::Bool(false),
        _ => Value::String(t.to_string()),
    }
}

fn parse_ini(s: &str) -> Value {
    let mut root = Map::new();
    let mut section: Option<String> = None;
    for line in s.lines() {
        let t = line.trim();
        let t = t.strip_prefix("export ").unwrap_or(t);
        if t.is_empty() || t.starts_with(['#', ';', '!']) {
            continue;
        }
        if let Some(name) = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
            section = Some(name.trim().to_string());
            root.entry(name.trim().to_string()).or_insert_with(|| Value::Object(Map::new()));
            continue;
        }
        let Some(split) = t.find(['=', ':']) else { continue };
        let (k, v) = (t[..split].trim().to_string(), scalar(&t[split + 1..]));
        match &section {
            Some(sec) => {
                if let Some(Value::Object(m)) = root.get_mut(sec) {
                    m.insert(k, v);
                }
            }
            None => {
                root.insert(k, v);
            }
        }
    }
    Value::Object(root)
}

// ---------------------------------------------------------------------------------------------
// Describing the shape

struct Outline<'a> {
    lines: &'a mut Vec<String>,
    truncated: bool,
}

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "nothing",
        Value::Bool(_) => "true/false values",
        Value::Number(_) => "numbers",
        Value::String(_) => "text",
        Value::Array(_) => "lists",
        Value::Object(_) => "objects",
    }
}

impl Outline<'_> {
    fn push(&mut self, depth: usize, text: String) -> bool {
        if self.lines.len() >= MAX_LINES {
            self.truncated = true;
            return false;
        }
        self.lines.push(format!("{}- {text}", "  ".repeat(depth)));
        true
    }

    fn root(&mut self, v: &Value, is_jsonl: bool) {
        match v {
            Value::Object(m) => {
                let keys = key_list(m.keys().map(String::as_str));
                self.push(0, format!("The document is an object with {} {}: {keys}.", m.len(), plural(m.len(), "key")));
                for (k, child) in m.iter().take(MAX_KEYS_SHOWN) {
                    self.walk(&format!("{k}"), &[child], 1);
                }
            }
            Value::Array(items) => {
                let what = if is_jsonl { "The file has" } else { "The document is a list of" };
                self.push(0, format!("{what} {} {}.", group(&items.len().to_string()), if is_jsonl { "records" } else { "items" }));
                let refs: Vec<&Value> = items.iter().collect();
                if !refs.is_empty() {
                    self.walk("[]", &refs, 1);
                }
            }
            other => {
                self.push(0, format!("The document is a single value of {}.", kind_name(other)));
            }
        }
    }

    fn walk(&mut self, path: &str, vals: &[&Value], depth: usize) {
        if depth > MAX_DEPTH || self.truncated {
            return;
        }
        let objs: Vec<&Map<String, Value>> = vals.iter().filter_map(|v| v.as_object()).collect();
        let arrs: Vec<&Vec<Value>> = vals.iter().filter_map(|v| v.as_array()).collect();
        let scalars: Vec<&Value> = vals.iter().copied().filter(|v| !v.is_object() && !v.is_array()).collect();
        let kinds = usize::from(!objs.is_empty()) + usize::from(!arrs.is_empty()) + usize::from(!scalars.is_empty());
        let seen = if vals.len() > 1 { format!(" (seen {} times)", group(&vals.len().to_string())) } else { String::new() };

        if kinds > 1 {
            if !self.push(depth, format!("`{path}` holds a mix of objects, lists and plain values{seen}.")) {
                return;
            }
        }
        if scalars.len() == 1 && kinds == 1 {
            let text = match scalars[0] {
                Value::String(s) => format!("\"{}\"", s.replace('"', "'").chars().take(120).collect::<String>()),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => "empty".to_string(),
            };
            self.push(depth, format!("`{path}` is {text}."));
        } else if !scalars.is_empty() {
            let nulls = scalars.iter().filter(|v| v.is_null()).count();
            let mut col = Column::new(path, "Field");
            for v in &scalars {
                match v {
                    Value::Number(n) => col.add_number(n.as_f64().unwrap_or(0.0)),
                    Value::Bool(b) => col.add_bool(*b),
                    Value::String(s) if !s.is_empty() => col.add_text(s),
                    _ => {}
                }
            }
            let mut line = if col.filled == 0 { format!("Field \"{path}\" is always empty or null.") } else { col.describe(scalars.len()) };
            if nulls > 0 && col.filled > 0 {
                line.push_str(&format!(" {} are null.", group(&nulls.to_string())));
            }
            self.push(depth, line);
        }
        if !objs.is_empty() {
            let mut keys: Vec<&str> = Vec::new();
            for m in &objs {
                for k in m.keys() {
                    if !keys.contains(&k.as_str()) {
                        keys.push(k);
                    }
                }
            }
            let shown = key_list(keys.iter().copied());
            let line = if path.ends_with("[]") || depth <= 1 && path == "[]" {
                format!("Each item in `{path}` is an object with {} {}: {shown}.{seen}", keys.len(), plural(keys.len(), "key"))
            } else {
                format!("`{path}` is an object with {} {}: {shown}.{seen}", keys.len(), plural(keys.len(), "key"))
            };
            if !self.push(depth, line) {
                return;
            }
            for k in keys.iter().take(MAX_KEYS_SHOWN) {
                let children: Vec<&Value> = objs.iter().filter_map(|m| m.get(*k)).collect();
                self.walk(&format!("{path}.{k}").trim_start_matches('.').to_string(), &children, depth + 1);
            }
        }
        if !arrs.is_empty() {
            let (min, max) = (arrs.iter().map(|a| a.len()).min().unwrap_or(0), arrs.iter().map(|a| a.len()).max().unwrap_or(0));
            let total: usize = arrs.iter().map(|a| a.len()).sum();
            let size = if min == max { format!("{}", group(&min.to_string())) } else { format!("{} to {}", group(&min.to_string()), group(&max.to_string())) };
            let line = if arrs.len() == 1 {
                format!("`{path}` is a list of {size} items.")
            } else {
                format!("`{path}` is a list with {size} items{seen}.")
            };
            if !self.push(depth, line) {
                return;
            }
            let items: Vec<&Value> = arrs.iter().flat_map(|a| a.iter()).collect();
            if total > 0 {
                self.walk(&format!("{path}[]"), &items, depth + 1);
            }
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { word.to_string() } else { format!("{word}s") }
}

/// A one-line description of a parsed data file for the front matter.
fn describe_root(kind: Kind, v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => {
            for key in ["description", "summary", "about"] {
                if let Some(Value::String(s)) = m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v) {
                    if !s.trim().is_empty() {
                        return Some(s.clone());
                    }
                }
            }
            let keys: Vec<&str> = m.keys().map(String::as_str).take(6).collect();
            let more = if m.len() > 6 { ", …" } else { "" };
            Some(format!("{} file with {} top-level {}: {}{more}.", kind.title(), m.len(), plural(m.len(), "key"), keys.join(", ")))
        }
        Value::Array(a) => Some(format!(
            "{} file holding a list of {} {}.",
            kind.title(),
            group(&a.len().to_string()),
            if kind == Kind::JsonLines { "records" } else { "items" }
        )),
        _ => None,
    }
}

fn key_list<'a>(keys: impl Iterator<Item = &'a str>) -> String {
    let all: Vec<&str> = keys.collect();
    let shown: Vec<String> = all.iter().take(MAX_KEYS_SHOWN).map(|k| format!("`{k}`")).collect();
    let more = all.len().saturating_sub(MAX_KEYS_SHOWN);
    if more > 0 { format!("{}, … (+{more} more)", shown.join(", ")) } else { shown.join(", ") }
}

// ---------------------------------------------------------------------------------------------
// XML

struct Elem {
    path: String,
    count: usize,
    attrs: Vec<String>,
    children: Vec<String>,
    text: Column,
}

/// Records one element occurrence and returns its full path.
fn register(
    e: &quick_xml::events::BytesStart,
    parent: Option<&String>,
    elems: &mut Vec<Elem>,
    index: &mut HashMap<String, usize>,
    root: &mut Option<String>,
) -> String {
    let name = e.name().as_ref().to_string();
    let path = format!("{}/{name}", parent.map_or("", String::as_str));
    root.get_or_insert_with(|| name.clone());
    let i = *index.entry(path.clone()).or_insert_with(|| {
        elems.push(Elem { path: path.clone(), count: 0, attrs: Vec::new(), children: Vec::new(), text: Column::new(path.clone(), "Element") });
        elems.len() - 1
    });
    elems[i].count += 1;
    for a in e.attributes().flatten() {
        let k = a.key.as_ref().to_string();
        if !elems[i].attrs.contains(&k) {
            elems[i].attrs.push(k);
        }
    }
    if let Some(p) = parent {
        if let Some(&pi) = index.get(p) {
            if !elems[pi].children.contains(&name) {
                elems[pi].children.push(name);
            }
        }
    }
    path
}

fn xml_outline(source: &str, lines: &mut Vec<String>) -> Option<(String, usize)> {
    use quick_xml::{Reader, events::Event};
    let mut reader = Reader::from_str(source);
    reader.config_mut().trim_text(true);
    let mut elems: Vec<Elem> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    // Full path of every currently open element.
    let mut open: Vec<String> = Vec::new();
    let mut error: Option<String> = None;
    let mut root: Option<String> = None;

    loop {
        match reader.read_event() {
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
            Ok(Event::Start(e)) => {
                let path = register(&e, open.last(), &mut elems, &mut index, &mut root);
                open.push(path);
            }
            Ok(Event::Empty(e)) => {
                register(&e, open.last(), &mut elems, &mut index, &mut root);
            }
            Ok(Event::Text(t)) => {
                if let Some(path) = open.last() {
                    if let Some(&i) = index.get(path) {
                        elems[i].text.add_raw(&t);
                    }
                }
            }
            Ok(Event::End(_)) => {
                open.pop();
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
    }

    if let Some(r) = &root {
        lines.push(format!("- The root element is `<{r}>`; {} distinct element paths were found.", group(&elems.len().to_string())));
    }
    for e in elems.iter().filter(|e| e.path.matches('/').count() <= MAX_DEPTH) {
        if lines.len() >= MAX_LINES {
            lines.push("- More elements exist than are described here; see the source below.".into());
            break;
        }
        let depth = e.path.matches('/').count().saturating_sub(1);
        let mut s = format!("`{}` appears {} time{}", e.path, group(&e.count.to_string()), if e.count == 1 { "" } else { "s" });
        if !e.attrs.is_empty() {
            s.push_str(&format!("; attributes: {}", e.attrs.iter().take(8).map(|a| format!("`{a}`")).collect::<Vec<_>>().join(", ")));
        }
        if !e.children.is_empty() {
            s.push_str(&format!("; contains: {}", key_list(e.children.iter().map(String::as_str))));
        }
        s.push('.');
        if e.text.filled > 0 {
            s.push(' ');
            s.push_str(&e.text.describe(e.count));
        }
        lines.push(format!("{}- {s}", "  ".repeat(depth.min(4))));
    }
    if let Some(err) = error {
        lines.push(format!("- The file could not be fully parsed ({err}); the description above covers what was readable."));
    }
    root.map(|r| (r, elems.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Summary (shape description) followed by source, so tests can look at both.
    fn md(kind: Kind, src: &str) -> String {
        let out = convert(kind, src.as_bytes(), Path::new("x"), "x").unwrap();
        format!("{}\n{}", out.summary.unwrap(), out.markdown)
    }

    #[test]
    fn json_shape_is_described_in_words() {
        let m = md(Kind::Json, r#"{"game":"Aurora","players":[{"id":1,"coins":50,"vip":true},{"id":2,"coins":150,"vip":false}]}"#);
        assert!(m.contains("The document is an object with 2 keys: `game`, `players`."), "{m}");
        assert!(m.contains("`game` is \"Aurora\"."), "{m}");
        assert!(m.contains("`players` is a list of 2 items."));
        assert!(m.contains("Each item in `players[]` is an object with 3 keys"));
        assert!(m.contains("Field \"players[].coins\" is numeric: smallest 50, largest 150, average 100, total 200."), "{m}");
        assert!(m.contains("```json\n{"));
    }

    #[test]
    fn json_with_comments_and_trailing_commas_is_accepted_and_kept_verbatim() {
        let src = "{\n  // note\n  \"a\": 1, /* x */\n  \"b\": [1,2,],\n}\n";
        let m = md(Kind::Json, src);
        assert!(m.contains("`a`, `b`") && m.contains("// note"));
    }

    #[test]
    fn jsonl_yaml_toml_ini_parse() {
        assert!(md(Kind::JsonLines, "{\"a\":1}\n{\"a\":2}\n").contains("The file has 2 records."));
        let y = md(Kind::Yaml, "name: demo\nports: [80, 443]\nnested:\n  1: one\n---\nsecond: doc\n");
        assert!(y.contains("The document is a list of 2 items."), "{y}");
        let t = md(Kind::Toml, "title = \"x\"\n[server]\nport = 8080\nwhen = 1979-05-27T07:32:00Z\n");
        assert!(t.contains("`server` is an object with 2 keys"), "{t}");
        let i = md(Kind::Ini, "# c\nkey=1\n[db]\nhost = localhost\nssl = true\n");
        assert!(i.contains("`db` is an object with 2 keys: `host`, `ssl`."), "{i}");
        assert!(i.contains("`db.ssl` is true."), "{i}");
    }

    #[test]
    fn xml_elements_and_attributes_are_listed() {
        let m = md(Kind::Xml, "<root><item id=\"1\">5</item><item id=\"2\">7</item><empty/></root>");
        assert!(m.contains("The root element is `<root>`"), "{m}");
        assert!(m.contains("`/root/item` appears 2 times; attributes: `id`"), "{m}");
        assert!(m.contains("`/root/empty` appears 1 time"));
    }

    #[test]
    fn description_prefers_the_files_own_and_falls_back_to_its_shape() {
        let own = convert(Kind::Json, br#"{"description":"Game settings for Aurora","a":1}"#, Path::new("x"), "x").unwrap();
        assert_eq!(own.description.as_deref(), Some("Game settings for Aurora"));
        let shape = convert(Kind::Yaml, b"a: 1\nb: 2\n", Path::new("x"), "x").unwrap();
        assert_eq!(shape.description.as_deref(), Some("YAML file with 2 top-level keys: a, b."));
        let xml = convert(Kind::Xml, b"<r><x/><x/></r>", Path::new("x"), "x").unwrap();
        assert_eq!(xml.description.as_deref(), Some("XML file with root element <r> and 2 distinct element paths."));
    }

    #[test]
    fn bad_json_reports_an_error() {
        assert!(convert(Kind::Json, b"{nope", Path::new("x"), "x").is_err());
    }
}
