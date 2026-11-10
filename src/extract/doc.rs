//! Legacy binary Word (.doc, Word 6 through Word 2003) read natively from the
//! Compound File Binary container, with no external programs.
//!
//! Recovered: text, paragraphs, tables, hyperlink text, Heading 1-9 styles and bulleted or
//! numbered list membership (read from the paragraph property tables). Not recovered: images,
//! text boxes, headers/footers, footnotes, character formatting.

use anyhow::{Context, Result, bail};
use std::io::{Cursor, Read};

fn u16_at(b: &[u8], o: usize) -> Result<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).context("truncated Word file")
}
fn u32_at(b: &[u8], o: usize) -> Result<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).context("truncated Word file")
}

fn stream(cf: &mut cfb::CompoundFile<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    cf.open_stream(name).with_context(|| format!("missing stream {name}"))?.read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn convert(bytes: &[u8], fallback: &str) -> Result<(String, String)> {
    let mut cf = cfb::CompoundFile::open(Cursor::new(bytes)).context("not a Word binary file")?;
    let word = stream(&mut cf, "/WordDocument")?;
    if u16_at(&word, 0)? != 0xA5EC {
        bail!("not a Word document");
    }
    let flags = u16_at(&word, 0x0A)?;
    if flags & 0x0100 != 0 {
        bail!("document is password protected");
    }
    let nfib = u16_at(&word, 2)?;

    let mut pieces: Vec<Piece> = Vec::new();
    let mut runs: Vec<(usize, usize, ParaInfo)> = Vec::new();
    let raw: Vec<char> = if nfib < 0x00C1 {
        // Word 6 / 95: text is one contiguous run of Windows-1252 bytes.
        let (min, mac) = (u32_at(&word, 0x18)? as usize, u32_at(&word, 0x1C)? as usize);
        let run = word.get(min..mac.min(word.len())).context("bad text range")?;
        pieces.push(Piece { cp0: 0, off: min, wide: false });
        run.iter().map(|&b| encoding_rs::WINDOWS_1252.decode_without_bom_handling(&[b]).0.chars().next().unwrap_or(' ')).collect()
    } else {
        let table = stream(&mut cf, if flags & 0x0200 != 0 { "/1Table" } else { "/0Table" })?;
        let ccp_text = u32_at(&word, 0x4C)? as usize;
        let (fc_clx, lcb_clx) = (u32_at(&word, 0x1A2)? as usize, u32_at(&word, 0x1A6)? as usize);
        let clx = table.get(fc_clx..fc_clx + lcb_clx).context("bad piece table location")?;
        let mut pos = 0;
        let plc = loop {
            match clx.get(pos) {
                Some(0x01) => pos += 3 + u16_at(clx, pos + 1)? as usize,
                Some(0x02) => {
                    let lcb = u32_at(clx, pos + 1)? as usize;
                    break clx.get(pos + 5..pos + 5 + lcb).context("bad piece table")?;
                }
                _ => bail!("piece table not found"),
            }
        };
        let n = (plc.len().saturating_sub(4)) / 12;
        let mut chars: Vec<char> = Vec::new();
        for k in 0..n {
            let (cp0, cp1) = (u32_at(plc, 4 * k)? as usize, u32_at(plc, 4 * (k + 1))? as usize);
            if cp0 >= ccp_text {
                break;
            }
            let len = cp1.saturating_sub(cp0).min(ccp_text - cp0);
            let fc = u32_at(plc, 4 * (n + 1) + 8 * k + 2)?;
            if fc & 0x4000_0000 != 0 {
                let off = ((fc & 0x3FFF_FFFF) / 2) as usize;
                let run = word.get(off..off + len).context("piece outside document")?;
                pieces.push(Piece { cp0, off, wide: false });
                chars.extend(run.iter().map(|&b| encoding_rs::WINDOWS_1252.decode_without_bom_handling(&[b]).0.chars().next().unwrap_or(' ')));
            } else {
                let off = fc as usize;
                let run = word.get(off..off + 2 * len).context("piece outside document")?;
                pieces.push(Piece { cp0, off, wide: true });
                // One character per 16-bit unit keeps character positions aligned with the file.
                chars.extend(run.chunks_exact(2).map(|u| char::from_u32(u16::from_le_bytes([u[0], u[1]]) as u32).unwrap_or('\u{fffd}')));
            }
        }
        // Paragraph styles and lists.
        if let (Ok(fc), Ok(lcb)) = (u32_at(&word, 0x102), u32_at(&word, 0x106)) {
            runs = read_runs(&word, &table, fc as usize, lcb as usize);
        }
        chars
    };

    let md = to_markdown(&raw, &pieces, &runs);
    // A short first line without sentence punctuation is almost always the title.
    let mut blocks = md.split("\n\n");
    let first = blocks.next().unwrap_or("").trim().to_string();
    let rest: Vec<&str> = blocks.collect();
    let (title, md) = if !first.is_empty() && first.chars().count() <= 80 && !first.contains('\n') && !first.ends_with(['.', ':', ';', ',']) && !first.starts_with(['|', '-']) {
        (first.clone(), format!("# {first}\n\n{}", rest.join("\n\n")))
    } else {
        (fallback.to_string(), md)
    };
    Ok((title, md))
}

#[derive(Clone, Copy, Default)]
struct ParaInfo {
    /// 1-9 for Heading 1-9, otherwise 0.
    heading: u8,
    /// Nesting level when the paragraph belongs to a bulleted or numbered list.
    list: Option<u8>,
}

struct Piece {
    cp0: usize,
    off: usize,
    wide: bool,
}

/// Paragraph property runs: (first file offset, end file offset, properties), sorted.
fn read_runs(word: &[u8], table: &[u8], fc: usize, lcb: usize) -> Vec<(usize, usize, ParaInfo)> {
    let mut runs = Vec::new();
    let Some(plcf) = table.get(fc..fc.saturating_add(lcb)) else { return runs };
    if lcb < 8 {
        return runs;
    }
    let n = (lcb - 4) / 8;
    for k in 0..n {
        let Ok(pn) = u32_at(plcf, 4 * (n + 1) + 4 * k) else { break };
        let base = (pn & 0x3F_FFFF) as usize * 512;
        let Some(fkp) = word.get(base..base + 512) else { continue };
        let crun = fkp[511] as usize;
        for i in 0..crun {
            let (Ok(a), Ok(b)) = (u32_at(fkp, 4 * i), u32_at(fkp, 4 * (i + 1))) else { break };
            let boff = fkp.get(4 * (crun + 1) + 13 * i).copied().unwrap_or(0) as usize;
            let info = if boff == 0 { ParaInfo::default() } else { parse_papx(fkp, boff * 2).unwrap_or_default() };
            runs.push((a as usize, b as usize, info));
        }
    }
    runs.sort_by_key(|r| r.0);
    runs
}

fn parse_papx(fkp: &[u8], off: usize) -> Option<ParaInfo> {
    let cb = *fkp.get(off)?;
    let (start, len) = if cb == 0 { (off + 2, 2 * *fkp.get(off + 1)? as usize) } else { (off + 1, (2 * cb as usize).checked_sub(1)?) };
    let data = fkp.get(start..start + len)?;
    let istd = u16_at(data, 0).ok()?;
    let mut info = ParaInfo { heading: if (1..=9).contains(&istd) { istd as u8 } else { 0 }, list: None };
    let (mut ilfo, mut ilvl) = (0i16, 0u8);
    let mut pos = 2;
    while pos + 2 <= data.len() {
        let op = u16_at(data, pos).ok()?;
        pos += 2;
        let size = match op >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            7 => 3,
            6 => {
                let n = *data.get(pos)? as usize;
                if n == 255 {
                    break;
                }
                n + 1
            }
            _ => break,
        };
        let operand = data.get(pos..pos + size)?;
        match op {
            0x460B if size >= 2 => ilfo = i16::from_le_bytes([operand[0], operand[1]]),
            0x2602 => ilvl = operand[0],
            _ => {}
        }
        pos += size;
    }
    if ilfo > 0 {
        info.list = Some(ilvl.min(8));
    }
    Some(info)
}

fn para_info(pieces: &[Piece], runs: &[(usize, usize, ParaInfo)], cp: usize) -> ParaInfo {
    if runs.is_empty() {
        return ParaInfo::default();
    }
    let idx = pieces.partition_point(|p| p.cp0 <= cp).saturating_sub(1);
    let Some(p) = pieces.get(idx) else { return ParaInfo::default() };
    let fc = p.off + (cp - p.cp0) * if p.wide { 2 } else { 1 };
    let r = runs.partition_point(|r| r.0 <= fc);
    r.checked_sub(1).and_then(|i| runs.get(i)).filter(|r| fc < r.1).map_or(ParaInfo::default(), |r| r.2)
}

fn flush_table(rows: &mut Vec<Vec<String>>, blocks: &mut Vec<String>) {
    if rows.is_empty() {
        return;
    }
    let w = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut t = String::new();
    for (i, row) in rows.iter().enumerate() {
        let mut cells: Vec<String> = row.iter().map(|c| c.replace('|', "\\|")).collect();
        cells.resize(w, String::new());
        t.push_str(&format!("| {} |\n", cells.join(" | ")));
        if i == 0 {
            t.push_str(&format!("|{}\n", " --- |".repeat(w)));
        }
    }
    blocks.push(t.trim_end().to_string());
    rows.clear();
}

/// Word control characters -> Markdown paragraphs and tables.
fn to_markdown(raw: &[char], pieces: &[Piece], runs: &[(usize, usize, ParaInfo)]) -> String {
    let mut blocks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cells: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut last_was_cell = false;
    // One entry per open field; true while we are inside its instruction part.
    let mut fields: Vec<bool> = Vec::new();

    let finish = |cur: &mut String, blocks: &mut Vec<String>, rows: &mut Vec<Vec<String>>, info: ParaInfo| {
        let t = cur.split_whitespace().collect::<Vec<_>>().join(" ");
        cur.clear();
        if t.is_empty() {
            return;
        }
        flush_table(rows, blocks);
        if info.heading > 0 {
            blocks.push(format!("{} {t}", "#".repeat(info.heading as usize)));
        } else if let Some(level) = info.list {
            let item = format!("{}- {t}", "  ".repeat(level as usize));
            match blocks.last_mut() {
                Some(last) if last.trim_start().starts_with("- ") => {
                    last.push('\n');
                    last.push_str(&item);
                }
                _ => blocks.push(item),
            }
        } else {
            blocks.push(t);
        }
    };

    for (cp, &c) in raw.iter().enumerate() {
        match c {
            '\x13' => fields.push(true),
            '\x14' => {
                if let Some(f) = fields.last_mut() {
                    *f = false;
                }
            }
            '\x15' => {
                fields.pop();
            }
            _ if fields.iter().any(|&in_instruction| in_instruction) => {}
            '\r' | '\x0c' => {
                let info = if c == '\r' { para_info(pieces, runs, cp) } else { ParaInfo::default() };
                finish(&mut cur, &mut blocks, &mut rows, info);
                last_was_cell = false;
            }
            '\x07' => {
                if cur.trim().is_empty() && last_was_cell {
                    let expected = rows.first().map_or(0, Vec::len);
                    if !rows.is_empty() && cells.len() < expected {
                        cells.push(String::new());
                    } else {
                        rows.push(std::mem::take(&mut cells));
                        last_was_cell = false;
                        continue;
                    }
                } else {
                    cells.push(cur.split_whitespace().collect::<Vec<_>>().join(" "));
                    cur.clear();
                }
                last_was_cell = true;
            }
            '\x0b' => cur.push(' '),
            '\x1e' => cur.push('-'),
            '\t' | '\u{a0}' => cur.push(' '),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => {
                if !cells.is_empty() && cur.is_empty() && !last_was_cell {
                    cells.clear();
                }
                cur.push(c);
                last_was_cell = false;
            }
        }
    }
    finish(&mut cur, &mut blocks, &mut rows, ParaInfo::default());
    flush_table(&mut rows, &mut blocks);
    blocks.join("\n\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_become_paragraphs_tables_and_field_results() {
        let raw = "Title\rBody \x13 HYPERLINK \"http://x\" \x14link text\x15 here.\rA\x07B\x07\x071\x072\x07\x07After table.\r";
        let chars: Vec<char> = raw.chars().collect();
        let md = to_markdown(&chars, &[], &[]);
        assert!(md.starts_with("Title\n\nBody link text here.\n\n"), "{md}");
        assert!(md.contains("| A | B |\n| --- | --- |\n| 1 | 2 |"), "{md}");
        assert!(md.trim_end().ends_with("After table."), "{md}");
        assert!(!md.contains("HYPERLINK"));
    }

    #[test]
    fn rejects_other_files() {
        assert!(convert(b"not a compound file", "f").is_err());
    }
}
