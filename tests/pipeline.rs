//! End-to-end checks through the public API with generated fixtures.

use doc_monster::{clean, digest, extract};
use std::io::Write;

fn minimal_xlsx() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        let mut put = |name: &str, body: &str| {
            z.start_file(name, o).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        };
        put(
            "[Content_Types].xml",
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        );
        put(
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        );
        put(
            "xl/workbook.xml",
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sales" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        );
        put(
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        );
        put(
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>item</t></is></c><c r="B1" t="inlineStr"><is><t>robux</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>pass</t></is></c><c r="B2"><v>1200</v></c></row>
<row r="3"><c r="A3" t="inlineStr"><is><t>pass</t></is></c><c r="B3"><v>800</v></c></row>
<row r="4"><c r="A4" t="inlineStr"><is><t>pet</t></is></c><c r="B4"><v>400</v></c></row>
</sheetData></worksheet>"#,
        );
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn spreadsheet_summary_is_structure_and_column_names_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sales.xlsx");
    std::fs::write(&path, minimal_xlsx()).unwrap();
    let doc = extract::extract(&path).unwrap();
    assert!(doc.markdown.contains("## Sheet: Sales"), "{}", doc.markdown);
    assert!(doc.markdown.contains("| pass | 1200 |"));
    assert_eq!(
        doc.description.as_deref(),
        Some("Spreadsheet, 3 rows × 2 columns; columns item, robux.")
    );
    let summary = doc.summary.unwrap();
    assert!(
        summary.contains("- **Sales**: 3 rows × 2 columns"),
        "{summary}"
    );
    assert!(summary.contains("Columns: `item`, `robux`"), "{summary}");
    // Nothing about the values: no statistics, no most common values.
    for told in [&summary, &doc.markdown] {
        for word in ["smallest", "largest", "average", "most common", "distinct"] {
            assert!(!told.contains(word), "{word} in {told}");
        }
    }
}

#[test]
fn non_video_bytes_with_mp4_extension_are_rejected() {
    // A tiny but valid mp4 is not practical to hand-build; subtitle handling is
    // covered by unit tests, so here we check non-video bytes are rejected cleanly.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fake.mp4");
    std::fs::write(&path, b"not really a video").unwrap();
    assert!(extract::extract(&path).is_err());
}

#[test]
fn ugly_markdown_becomes_clean_and_digestible() {
    let mut ugly = String::from("Handbook\r\n========\r\n\r\n");
    for i in 0..30 {
        ugly.push_str(&format!("##Chapter {i}\r\n\r\n<!-- note -->\r\n"));
        ugly.push_str("Player data is saved to a DataStore regularly and\r\nsession locking prevents corrupted profiles for players.  \r\n\r\n\r\n\r\n");
        ugly.push_str("*   Rate limit remote calls per player\r\n+   Validate every argument on the server\r\n\r\n");
        ugly.push_str(&format!(
            "Page {i} of 30\r\n\r\nCONFIDENTIAL - internal handbook\r\n\r\n"
        ));
    }
    let (md, report) = clean::clean(&ugly);
    assert!(
        md.starts_with("# Handbook\n\n## Chapter 0\n"),
        "{}",
        &md[..80]
    );
    assert!(
        !md.contains('\r')
            && !md.contains("<!--")
            && !md.contains("CONFIDENTIAL")
            && !md.contains("Page 3 of")
    );
    assert!(md.contains("saved to a DataStore regularly and session locking"));
    assert!(md.contains("- Rate limit remote calls per player\n- Validate every argument"));
    assert_eq!(report.boilerplate[0].1, 30);
    // Cleaning must not lose any real sentence.
    assert_eq!(
        md.matches("session locking prevents corrupted profiles")
            .count(),
        30
    );

    let d = digest::digest(&md, report.describe().as_deref()).expect("long enough to digest");
    assert!(d.markdown.contains("### Outline") && d.markdown.contains("Cleanup: removed"));
    assert!(
        d.markdown.len() < md.len() / 2,
        "digest should be much shorter than the source"
    );
}
