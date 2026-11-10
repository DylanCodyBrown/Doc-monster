//! Rich Text Format reader: text, paragraphs, headings (from Word's heading styles),
//! tables, Unicode escapes and Windows-1252 escapes.

use super::{first_heading, text};
use anyhow::{Result, bail};
use regex::Regex;
use std::collections::HashMap;

const SKIP_DESTINATIONS: &[&str] = &[
    "fonttbl", "colortbl", "stylesheet", "info", "pict", "header", "footer", "headerl", "headerr", "headerf", "footerl",
    "footerr", "footerf", "footnote", "themedata", "datastore", "latentstyles", "listtable", "listoverridetable",
    "rsidtbl", "generator", "xmlnstbl", "colorschememapping", "fldinst", "object", "bkmkstart", "bkmkend", "pntext",
    "pnseclvl", "filetbl", "revtbl", "mmathPr", "nonshppict", "shpinst",
];

struct Frame {
    skip: bool,
    uc: usize,
}

pub fn convert(bytes: &[u8], fallback: &str) -> Result<(String, String)> {
    if !bytes.starts_with(b"{\\rtf") {
        bail!("not an RTF file");
    }
    // Latin-1 view of the bytes: RTF is 7-bit ASCII with escapes.
    let src: Vec<char> = bytes.iter().map(|&b| b as char).collect();
    let raw: String = src.iter().collect();

    // Word heading styles: {\s1 ... heading 1;}
    let style_re = Regex::new(r"(?i)\{\\s(\d+)[^{}]*?\bheading (\d)\s*;").unwrap();
    let headings: HashMap<u32, usize> = style_re
        .captures_iter(&raw)
        .filter_map(|c| Some((c[1].parse().ok()?, c[2].parse().ok()?)))
        .collect();

    let mut stack: Vec<Frame> = vec![Frame { skip: false, uc: 1 }];
    let mut para = String::new();
    let mut para_style: Option<u32> = None;
    let mut para_list = false;
    let mut in_tbl = false;
    let mut blocks: Vec<String> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut table: Vec<Vec<String>> = Vec::new();
    let mut skip_chars = 0usize;
    let mut i = 0;

    let flush_table = |table: &mut Vec<Vec<String>>, blocks: &mut Vec<String>| {
        if table.is_empty() {
            return;
        }
        let w = table.iter().map(Vec::len).max().unwrap_or(0);
        let mut t = String::new();
        for (r, cells) in table.iter().enumerate() {
            let mut cells = cells.clone();
            cells.resize(w, String::new());
            t.push_str(&format!("| {} |\n", cells.join(" | ")));
            if r == 0 {
                t.push_str(&format!("|{}\n", " --- |".repeat(w)));
            }
        }
        blocks.push(t);
        table.clear();
    };
    let end_para = |para: &mut String, style: &mut Option<u32>, list: &mut bool, blocks: &mut Vec<String>| {
        let t = para.split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() {
            match style.and_then(|s| headings.get(&s)) {
                Some(&lvl) => blocks.push(format!("{} {t}", "#".repeat(lvl.min(6)))),
                None if *list => {
                    // Consecutive list items form one block.
                    match blocks.last_mut() {
                        Some(last) if last.starts_with("- ") => last.push_str(&format!("\n- {t}")),
                        _ => blocks.push(format!("- {t}")),
                    }
                }
                None => blocks.push(t),
            }
        }
        para.clear();
        *style = None;
        *list = false;
    };

    while i < src.len() {
        let c = src[i];
        let skipping = stack.last().is_some_and(|f| f.skip);
        match c {
            '{' => {
                let uc = stack.last().map_or(1, |f| f.uc);
                // Peek: `{\*` or a known skippable destination starts a hidden group.
                let rest: String = src[i + 1..(i + 40).min(src.len())].iter().collect();
                let hidden = rest.starts_with("\\*")
                    || SKIP_DESTINATIONS.iter().any(|d| {
                        rest.strip_prefix('\\')
                            .is_some_and(|r| r.starts_with(d) && !r[d.len()..].starts_with(|ch: char| ch.is_ascii_alphabetic()))
                    });
                stack.push(Frame { skip: skipping || hidden, uc });
                i += 1;
            }
            '}' => {
                if stack.len() > 1 {
                    stack.pop();
                }
                i += 1;
            }
            '\\' => {
                i += 1;
                let Some(&n) = src.get(i) else { break };
                if !n.is_ascii_alphabetic() {
                    i += 1;
                    match n {
                        '\'' => {
                            let hex: String = src[i..(i + 2).min(src.len())].iter().collect();
                            i += 2;
                            if skip_chars > 0 {
                                skip_chars -= 1;
                            } else if !skipping {
                                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                                    para.push_str(&encoding_rs::WINDOWS_1252.decode_without_bom_handling(&[b]).0);
                                }
                            }
                        }
                        '~' if !skipping => para.push('\u{a0}'),
                        '_' if !skipping => para.push('-'),
                        '{' | '}' | '\\' if !skipping => para.push(n),
                        '\n' | '\r' if !skipping => end_para(&mut para, &mut para_style, &mut para_list, &mut blocks),
                        _ => {}
                    }
                    continue;
                }
                let start = i;
                while i < src.len() && src[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word: String = src[start..i].iter().collect();
                let neg = src.get(i) == Some(&'-');
                if neg {
                    i += 1;
                }
                let ds = i;
                while i < src.len() && src[i].is_ascii_digit() {
                    i += 1;
                }
                let param: Option<i64> = src[ds..i].iter().collect::<String>().parse().ok().map(|v: i64| if neg { -v } else { v });
                if src.get(i) == Some(&' ') {
                    i += 1;
                }
                if skipping {
                    continue;
                }
                match word.as_str() {
                    "par" | "sect" | "page" => {
                        if in_tbl {
                            para.push(' '); // several paragraphs inside one table cell
                        } else {
                            end_para(&mut para, &mut para_style, &mut para_list, &mut blocks);
                        }
                    }
                    "intbl" => in_tbl = true,
                    "ls" => para_list = true,
                    "line" => para.push(' '),
                    "tab" => para.push(' '),
                    "pard" => {
                        para_style = None;
                        para_list = false;
                        in_tbl = false;
                    }
                    "s" => para_style = param.map(|p| p as u32),
                    "uc" => {
                        if let (Some(p), Some(f)) = (param, stack.last_mut()) {
                            f.uc = p.max(0) as usize;
                        }
                    }
                    "u" => {
                        if let Some(p) = param {
                            let code = if p < 0 { (p + 65536) as u32 } else { p as u32 };
                            if let Some(ch) = char::from_u32(code) {
                                para.push(ch);
                            }
                            skip_chars = stack.last().map_or(1, |f| f.uc);
                        }
                    }
                    "emdash" => para.push('—'),
                    "endash" => para.push('–'),
                    "lquote" => para.push('\u{2018}'),
                    "rquote" => para.push('\u{2019}'),
                    "ldblquote" => para.push('\u{201c}'),
                    "rdblquote" => para.push('\u{201d}'),
                    "bullet" => para.push('•'),
                    "cell" => {
                        row.push(para.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|"));
                        para.clear();
                    }
                    "row" => {
                        table.push(std::mem::take(&mut row));
                    }
                    _ => {}
                }
            }
            '\r' | '\n' => i += 1,
            _ => {
                if skip_chars > 0 {
                    skip_chars -= 1;
                } else if !skipping {
                    if !in_tbl && !table.is_empty() && row.is_empty() && !c.is_whitespace() {
                        flush_table(&mut table, &mut blocks);
                    }
                    para.push(c);
                }
                i += 1;
            }
        }
    }
    end_para(&mut para, &mut para_style, &mut para_list, &mut blocks);
    flush_table(&mut table, &mut blocks);

    // Characters above 0x7F that were not escaped came through as Latin-1; re-decode if the text is UTF-8.
    let md = blocks.join("\n\n") + "\n";
    let md = text::decode(md.chars().map(|c| c as u32).all(|c| c < 256).then(|| md.chars().map(|c| c as u8).collect::<Vec<u8>>()).as_deref().unwrap_or(md.as_bytes()));
    let title = first_heading(&md).unwrap_or_else(|| fallback.to_string());
    Ok((title, md))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_unicode_and_skips_tables_of_fonts() {
        let rtf = br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}}{\colortbl;\red0\green0\blue0;}{\stylesheet{\s1 heading 1;}}\pard\s1 Big Title\par\pard Caf\'e9 \u8364? and {\b bold} text.\par Second paragraph.\par}";
        let (title, md) = convert(rtf, "f").unwrap();
        assert_eq!(title, "Big Title", "{md}");
        assert!(md.contains("# Big Title"), "{md}");
        assert!(md.contains("Café € and bold text."), "{md}");
        assert!(md.contains("Second paragraph."));
        assert!(!md.contains("Arial") && !md.contains("colortbl"));
    }

    #[test]
    fn rejects_non_rtf() {
        assert!(convert(b"plain", "f").is_err());
    }
}
