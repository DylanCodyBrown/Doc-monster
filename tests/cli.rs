//! Runs the real program and checks what it writes.

use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_doc-monster")).args(args).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn long_markdown() -> String {
    let mut md = String::from("# Aurora Handbook\n\n");
    for (title, body) in [
        ("Saving data", "Player data is saved to a DataStore every minute. Session locking prevents corruption of profiles. Always wrap DataStore calls in pcall because requests are throttled."),
        ("Money", "Game passes unlock permanent perks for buyers. Developer products sell repeatable boosts to players. The economy needs coin sinks to avoid inflation of prices."),
        ("Security", "Validate every RemoteEvent argument on the server always. Rate limit remote calls per player to stop exploit spam. Never trust the client for purchases or damage."),
    ] {
        md.push_str(&format!("## {title}\n\n"));
        for _ in 0..8 {
            md.push_str(body);
            md.push_str("\n\n");
        }
    }
    md
}

/// Front matter must be exactly: a comment, a title and a description.
fn front_matter(text: &str) -> Vec<&str> {
    let rest = text.strip_prefix("---\n").expect("starts with front matter");
    let end = rest.find("\n---\n").expect("front matter closes");
    rest[..end].lines().collect()
}

#[test]
fn front_matter_is_a_comment_a_title_and_a_description_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("handbook.md"), long_markdown()).unwrap();
    std::fs::write(dir.path().join("settings.json"), r#"{"game":"Aurora","players":[{"id":1},{"id":2}]}"#).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "A short note about pets.\n").unwrap();
    let out = dir.path().join("out");
    let (ok, log) = run(&[dir.path().to_str().unwrap(), "-o", out.to_str().unwrap()]);
    assert!(ok, "{log}");

    for name in ["handbook.md", "settings.md", "notes.md"] {
        let text = std::fs::read_to_string(out.join(name)).unwrap();
        let fm = front_matter(&text);
        assert_eq!(fm.len(), 3, "{name}: {fm:?}");
        assert!(fm[0].starts_with("# ") && fm[0].ends_with("with Doc Monster"), "{name}: {fm:?}");
        assert!(fm[1].starts_with("title: \""), "{name}: {fm:?}");
        assert!(fm[2].starts_with("description: \""), "{name}: {fm:?}");
    }
    let handbook = std::fs::read_to_string(out.join("handbook.md")).unwrap();
    assert!(handbook.contains("# Condensed with Doc Monster") && handbook.contains("title: \"Aurora Handbook\""));
    let notes = std::fs::read_to_string(out.join("notes.md")).unwrap();
    assert!(notes.contains("# Converted with Doc Monster"), "{notes}");
    assert!(notes.contains("description: \"A short note about pets.\""), "{notes}");
    let settings = std::fs::read_to_string(out.join("settings.md")).unwrap();
    assert!(settings.contains("`game` is \"Aurora\".") && settings.contains("```json\n{"), "{settings}");
}

#[test]
fn condensed_flag_leaves_only_the_summary() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("handbook.md"), long_markdown()).unwrap();
    let out = dir.path().join("out");
    let (ok, log) = run(&[dir.path().join("handbook.md").to_str().unwrap(), "-o", out.to_str().unwrap(), "--condensed"]);
    assert!(ok, "{log}");
    let text = std::fs::read_to_string(out.join("handbook.md")).unwrap();
    assert!(text.contains("## Summary") && !text.contains("## Full content"));
    assert!(text.len() < long_markdown().len() / 2);
}

#[test]
fn legacy_binary_word_document_keeps_headings_lists_and_tables() {
    let out = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.doc");
    let (ok, log) = run(&[fixture.to_str().unwrap(), "-o", out.path().to_str().unwrap(), "--backend", "none"]);
    assert!(ok, "{log}");
    let text = std::fs::read_to_string(out.path().join("sample.md")).unwrap();
    for needle in ["# Fixture Guide", "# Saving data", "## Checklist", "- Wrap calls in pcall\n- Version every save", "| Metric | Target |", "| Retention | 30 |", "# Money"] {
        assert!(text.contains(needle), "missing {needle:?} in\n{text}");
    }
}

#[test]
fn every_new_text_and_data_format_converts() {
    let dir = tempfile::tempdir().unwrap();
    let files: &[(&str, &str)] = &[
        ("a.yaml", "name: demo\nports: [1, 2]\n"),
        ("b.toml", "title = \"x\"\n[s]\nport = 1\n"),
        ("c.xml", "<r><i id=\"1\">5</i></r>"),
        ("d.ini", "[a]\nk = v\n"),
        ("e.jsonl", "{\"a\":1}\n{\"a\":2}\n"),
        ("f.rst", "Title\n=====\n\nBody text.\n"),
        ("g.adoc", "= Title\n\nBody text.\n"),
        ("h.org", "* Title\nBody text.\n"),
        ("i.tex", "\\section{Title}\nBody text.\n"),
        ("j.rtf", r"{\rtf1\ansi Body text.\par}"),
        ("k.log", "line one\nline two\n"),
        ("LICENSE", "Permission is granted.\n"),
    ];
    for (name, body) in files {
        std::fs::write(dir.path().join(name), body).unwrap();
    }
    let out = dir.path().join("out");
    let (ok, log) = run(&[dir.path().to_str().unwrap(), "-o", out.to_str().unwrap()]);
    assert!(ok, "{log}");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), files.len(), "{log}");
}
