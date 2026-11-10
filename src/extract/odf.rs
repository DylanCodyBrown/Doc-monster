//! OpenDocument text and presentations (.odt .ott .odp .fodt .fodp).

use super::{Out, first_heading, text, xml_field};
use anyhow::{Context, Result};
use quick_xml::{Reader, events::Event};
use std::io::{Cursor, Read};

pub fn convert(bytes: &[u8], fallback: &str) -> Result<Out> {
    let (content, meta) = if bytes.starts_with(b"PK") {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("not a valid OpenDocument archive")?;
        let mut content = String::new();
        zip.by_name("content.xml").context("missing content.xml")?.read_to_string(&mut content)?;
        let mut meta = String::new();
        if let Ok(mut f) = zip.by_name("meta.xml") {
            let _ = f.read_to_string(&mut meta);
        }
        (content, meta)
    } else {
        let flat = text::decode(bytes);
        (flat.clone(), flat)
    };
    let md = content_to_markdown(&content)?;
    let title = xml_field(&meta, "title").or_else(|| first_heading(&md)).unwrap_or_else(|| fallback.to_string());
    let description = xml_field(&meta, "description").or_else(|| xml_field(&meta, "subject"));
    Ok(Out::new(title, md).with_description(description))
}

fn attr(e: &quick_xml::events::BytesStart, local: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.local_name().as_ref() == local).map(|a| a.value.to_string())
}

fn push_ref(r: &quick_xml::events::BytesRef, into: &mut String) {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        into.push(c);
        return;
    }
    match &**r {
        "amp" => into.push('&'),
        "lt" => into.push('<'),
        "gt" => into.push('>'),
        "quot" => into.push('"'),
        "apos" => into.push('\''),
        _ => {}
    }
}

fn content_to_markdown(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut out = String::new();
    let mut notes: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut heading: Option<usize> = None;
    let mut in_text = false; // inside a paragraph or heading
    let mut skip = 0usize; // inside annotations, tracked changes, note citations
    let mut list_depth = 0usize;
    let mut in_item_para = false;
    let mut in_note = 0usize;
    let mut slide = 0usize;
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut cell = String::new();

    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                "annotation" | "tracked-changes" | "note-citation" => skip += 1,
                "h" if skip == 0 => {
                    in_text = true;
                    text.clear();
                    heading = Some(attr(&e, "outline-level").and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, 6));
                }
                "p" if skip == 0 => {
                    in_text = true;
                    text.clear();
                    in_item_para = list_depth > 0;
                }
                "list" => list_depth += 1,
                "note-body" => in_note += 1,
                "s" => {
                    if in_text {
                        text.push_str(&" ".repeat(attr(&e, "c").and_then(|c| c.parse().ok()).unwrap_or(1)));
                    }
                }
                "tab" if in_text => text.push(' '),
                "line-break" if in_text => text.push(' '),
                "table" => table = Some(Vec::new()),
                "table-row" => {
                    if let Some(t) = table.as_mut() {
                        t.push(Vec::new());
                    }
                }
                "page" if attr(&e, "name").is_some() || xml.contains("<office:presentation") => {
                    slide += 1;
                    out.push_str(&format!("## Slide {slide}\n\n"));
                }
                _ => {}
            },
            Event::Text(t) if in_text && skip == 0 => text.push_str(&t),
            Event::GeneralRef(r) if in_text && skip == 0 => push_ref(&r, &mut text),
            Event::End(e) => match e.local_name().as_ref() {
                "annotation" | "tracked-changes" | "note-citation" => skip = skip.saturating_sub(1),
                "list" => list_depth = list_depth.saturating_sub(1),
                "note-body" => in_note = in_note.saturating_sub(1),
                "h" | "p" if skip == 0 => {
                    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    text.clear();
                    in_text = false;
                    let h = heading.take();
                    if table.is_some() {
                        if !cell.is_empty() && !t.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(&t);
                    } else if !t.is_empty() {
                        if in_note > 0 {
                            notes.push(t);
                        } else if let Some(level) = h {
                            out.push_str(&format!("{} {t}\n\n", "#".repeat(level)));
                        } else if in_item_para {
                            out.push_str(&format!("{}- {t}\n", "  ".repeat(list_depth.saturating_sub(1))));
                        } else {
                            out.push_str(&format!("{t}\n\n"));
                        }
                    }
                    if e.local_name().as_ref() == "p" && list_depth == 0 {
                        in_item_para = false;
                    }
                }
                "table-cell" => {
                    if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                        row.push(cell.replace('|', "\\|"));
                    }
                    cell.clear();
                }
                "table" => {
                    if let Some(rows) = table.take() {
                        let w = rows.iter().map(Vec::len).max().unwrap_or(0);
                        for (i, row) in rows.iter().enumerate() {
                            let mut cells = row.clone();
                            cells.resize(w, String::new());
                            out.push_str(&format!("| {} |\n", cells.join(" | ")));
                            if i == 0 {
                                out.push_str(&format!("|{}\n", " --- |".repeat(w)));
                            }
                        }
                        out.push('\n');
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if !notes.is_empty() {
        out.push_str("## Notes\n\n");
        for (i, n) in notes.iter().enumerate() {
            out.push_str(&format!("{}. {n}\n", i + 1));
        }
    }
    // Lists emit single newlines; make sure a blank line follows each list block.
    let mut tidy = String::new();
    let mut prev_item = false;
    for line in out.lines() {
        let item = line.trim_start().starts_with("- ");
        if prev_item && !item && !line.is_empty() {
            tidy.push('\n');
        }
        tidy.push_str(line);
        tidy.push('\n');
        prev_item = item;
    }
    Ok(tidy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_headings_lists_tables_and_skips_comments() {
        let xml = r#"<office:document-content xmlns:office="o" xmlns:text="t" xmlns:table="b"><office:body><office:text>
<text:h text:outline-level="1">Report</text:h>
<text:p>Fish &amp; <text:span>chips</text:span><text:s text:c="2"/>here<office:annotation><text:p>HIDDEN</text:p></office:annotation></text:p>
<text:list><text:list-item><text:p>one</text:p></text:list-item><text:list-item><text:p>two</text:p></text:list-item></text:list>
<table:table><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell><table:table-cell><text:p>B</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell><text:p>1</text:p></table:table-cell><table:table-cell><text:p>2</text:p></table:table-cell></table:table-row></table:table>
</office:text></office:body></office:document-content>"#;
        let md = content_to_markdown(xml).unwrap();
        assert!(md.contains("# Report\n"), "{md}");
        assert!(md.contains("Fish & chips here"), "{md}");
        assert!(!md.contains("HIDDEN"));
        assert!(md.contains("- one\n- two\n"), "{md}");
        assert!(md.contains("| A | B |\n| --- | --- |\n| 1 | 2 |"), "{md}");
    }
}
