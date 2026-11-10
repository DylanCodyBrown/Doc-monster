//! Runs the real program and checks what it writes.

use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_doc-monster"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn long_markdown() -> String {
    let mut md = String::from("# Aurora Handbook\n\n");
    for (title, body) in [
        (
            "Saving data",
            "Player data is saved to a DataStore every minute. Session locking prevents corruption of profiles. Always wrap DataStore calls in pcall because requests are throttled.",
        ),
        (
            "Money",
            "Game passes unlock permanent perks for buyers. Developer products sell repeatable boosts to players. The economy needs coin sinks to avoid inflation of prices.",
        ),
        (
            "Security",
            "Validate every RemoteEvent argument on the server always. Rate limit remote calls per player to stop exploit spam. Never trust the client for purchases or damage.",
        ),
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
    let rest = text
        .strip_prefix("---\n")
        .expect("starts with front matter");
    let end = rest.find("\n---\n").expect("front matter closes");
    rest[..end].lines().collect()
}

#[test]
fn front_matter_is_a_comment_a_title_and_a_description_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("handbook.md"), long_markdown()).unwrap();
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{"game":"Aurora","players":[{"id":1},{"id":2}]}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("notes.txt"), "A short note about pets.\n").unwrap();
    let out = dir.path().join("out");
    let (ok, log) = run(&[dir.path().to_str().unwrap(), "-o", out.to_str().unwrap()]);
    assert!(ok, "{log}");

    for name in ["handbook.md", "settings.md", "notes.md"] {
        let text = std::fs::read_to_string(out.join(name)).unwrap();
        let fm = front_matter(&text);
        assert_eq!(fm.len(), 3, "{name}: {fm:?}");
        assert!(
            fm[0].starts_with("# ") && fm[0].ends_with("with Doc Monster"),
            "{name}: {fm:?}"
        );
        assert!(fm[1].starts_with("title: \""), "{name}: {fm:?}");
        assert!(fm[2].starts_with("description: \""), "{name}: {fm:?}");
    }
    let handbook = std::fs::read_to_string(out.join("handbook.md")).unwrap();
    assert!(
        handbook.contains("# Condensed with Doc Monster")
            && handbook.contains("title: \"Aurora Handbook\"")
    );
    let notes = std::fs::read_to_string(out.join("notes.md")).unwrap();
    assert!(notes.contains("# Converted with Doc Monster"), "{notes}");
    assert!(
        notes.contains("description: \"A short note about pets.\""),
        "{notes}"
    );
    let settings = std::fs::read_to_string(out.join("settings.md")).unwrap();
    assert!(
        settings.contains("`game` is \"Aurora\".") && settings.contains("```json\n{"),
        "{settings}"
    );
}

#[test]
fn compression_levels_shrink_the_output_and_ten_leaves_only_the_summary() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("handbook.md");
    std::fs::write(&src, long_markdown()).unwrap();
    let mut sizes = Vec::new();
    for level in ["1", "5", "10"] {
        let out = dir.path().join(format!("out{level}"));
        let (ok, log) = run(&[
            src.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--compress",
            level,
        ]);
        assert!(ok, "{log}");
        let text = std::fs::read_to_string(out.join("handbook.md")).unwrap();
        match level {
            "1" => assert!(
                text.starts_with("---\n# Condensed with Doc Monster\n")
                    && text.contains("## Full content"),
                "{text}"
            ),
            "5" => {
                assert!(
                    text.starts_with("---\n# Condensed with Doc Monster, compression 5 of 10\n"),
                    "{text}"
                );
                assert!(
                    text.contains("## Compressed content")
                        && text.contains("% of the words (compression 5 of 10)"),
                    "{text}"
                );
                for h in ["## Saving data", "## Money", "## Security"] {
                    assert!(text.contains(h), "every section survives: {text}");
                }
            }
            _ => assert!(
                text.contains("## Summary")
                    && !text.contains("## Full content")
                    && !text.contains("## Compressed content")
            ),
        }
        sizes.push(text.len());
    }
    assert!(sizes[0] > sizes[1] && sizes[1] > sizes[2], "{sizes:?}");
}

#[test]
fn compression_outside_one_to_ten_is_refused() {
    for bad in ["0", "11", "x"] {
        let out = Command::new(env!("CARGO_BIN_EXE_doc-monster"))
            .args(["README.md", "--compress", bad])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "--compress {bad}");
    }
}

#[test]
fn legacy_binary_word_document_keeps_headings_lists_and_tables() {
    let out = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.doc");
    let (ok, log) = run(&[
        fixture.to_str().unwrap(),
        "-o",
        out.path().to_str().unwrap(),
        "--no-summary",
    ]);
    assert!(ok, "{log}");
    let text = std::fs::read_to_string(out.path().join("sample.md")).unwrap();
    for needle in [
        "# Fixture Guide",
        "# Saving data",
        "## Checklist",
        "- Wrap calls in pcall\n- Version every save",
        "| Metric | Target |",
        "| Retention | 30 |",
        "# Money",
    ] {
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
    assert_eq!(
        std::fs::read_dir(&out).unwrap().count(),
        files.len(),
        "{log}"
    );
}

#[test]
fn output_goes_beside_the_source_and_conflicts_get_v2() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("notes.md"), long_markdown()).unwrap(); // the source itself is notes.md
    std::fs::write(d.join("report.txt"), "A short report about pets.\n").unwrap();
    std::fs::write(d.join("report.csv"), "a,b\n1,2\n").unwrap();
    let (ok, log) = run(&[d.to_str().unwrap()]);
    assert!(ok, "{log}");
    // The original Markdown is never overwritten.
    assert_eq!(
        std::fs::read_to_string(d.join("notes.md")).unwrap(),
        long_markdown()
    );
    assert!(d.join("notes.v2.md").exists(), "{log}");
    // Two sources with the same name: the second gets .v2.
    assert!(
        d.join("report.md").exists() && d.join("report.v2.md").exists(),
        "{log}"
    );

    // Running again does not eat its own outputs; it writes new versions of the real sources.
    let (ok, log) = run(&[d.to_str().unwrap()]);
    assert!(ok, "{log}");
    assert!(log.contains("earlier outputs skipped"), "{log}");
    assert!(d.join("notes.v3.md").exists(), "{log}");

    // --overwrite replaces our own earlier output instead of adding versions.
    let before = std::fs::read_dir(d).unwrap().count();
    let (ok, log) = run(&[d.join("report.txt").to_str().unwrap(), "--overwrite"]);
    assert!(ok, "{log}");
    assert_eq!(std::fs::read_dir(d).unwrap().count(), before, "{log}");
}

#[test]
fn json_report_is_valid_and_lists_failures_and_missing_inputs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("good.txt"), "A short note about pets.\n").unwrap();
    std::fs::write(dir.path().join("bad.docx"), b"not a zip").unwrap();
    let missing = dir.path().join("nope.pdf");
    let out = Command::new(env!("CARGO_BIN_EXE_doc-monster"))
        .args([
            dir.path().to_str().unwrap(),
            missing.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "some failed -> exit 1");
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is pure JSON");
    assert_eq!(report["converted"], 1);
    assert_eq!(report["failed"], 2);
    let errors: Vec<&str> = report["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["error"].as_str())
        .collect();
    assert!(errors.iter().any(|e| e.contains("not found")), "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("DOCX")), "{errors:?}");
}

#[test]
fn no_animation_or_escape_codes_when_not_in_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "A short note about pets.\n").unwrap();
    let (ok, log) = run(&[dir.path().to_str().unwrap()]);
    assert!(ok);
    assert!(
        !log.contains('\x1b'),
        "piped output must be plain text: {log:?}"
    );
}

#[test]
fn index_command_writes_one_index_at_the_top_merges_and_removes_only_our_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::create_dir_all(d.join("team")).unwrap();
    std::fs::write(d.join("handbook.md"), long_markdown()).unwrap();
    std::fs::write(d.join("team/copy-of-handbook.md"), long_markdown()).unwrap(); // a user's own duplicate
    // Convert twice: the second run adds .v3 copies, which are Doc Monster duplicates.
    assert!(run(&[d.to_str().unwrap()]).0);
    assert!(run(&[d.to_str().unwrap()]).0);
    assert!(d.join("handbook.v3.md").exists());

    let (ok, log) = run(&["index", d.to_str().unwrap()]);
    assert!(ok, "{log}");
    let index = std::fs::read_to_string(d.join("INDEX.md")).unwrap();
    assert!(
        index.starts_with("---\n# Indexed with Doc Monster\n"),
        "{index}"
    );
    // Only one index, at the top; none in the subfolder.
    assert!(!d.join("team/INDEX.md").exists());
    // All handbook copies are one document.
    assert!(index.contains("1 unique documents"), "{index}");
    // User files are never deleted; our extra copies are.
    assert!(d.join("handbook.md").exists() && d.join("team/copy-of-handbook.md").exists());
    assert!(
        d.join("handbook.v2.md").exists(),
        "one Doc Monster copy is kept"
    );
    assert!(
        !d.join("handbook.v3.md").exists() && !d.join("team/copy-of-handbook.v3.md").exists(),
        "{log}"
    );

    // The index is never eaten as a document, and rebuilding reuses INDEX.md.
    let (ok, log) = run(&[d.to_str().unwrap(), "--index"]);
    assert!(ok, "{log}");
    assert!(!d.join("INDEX.v2.md").exists(), "{log}");
    assert!(!log.contains("INDEX.md ->"), "{log}");
}

#[test]
fn feed_gorge_and_regurgitate_commands() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(
            d.join(name),
            format!("A short note called {name} about pets.\n"),
        )
        .unwrap();
    }
    // feed: one file.
    let (ok, log) = run(&["feed", d.join("a.txt").to_str().unwrap(), "-c", "10"]);
    assert!(ok && d.join("a.md").exists(), "{log}");
    // feed refuses several paths and points at gorge.
    let (ok, log) = run(&[
        "feed",
        d.join("a.txt").to_str().unwrap(),
        d.join("b.txt").to_str().unwrap(),
    ]);
    assert!(!ok && log.contains("use gorge"), "{log}");
    // gorge (and the "gourge" spelling): several paths.
    let (ok, log) = run(&["gorge", d.join("b.txt").to_str().unwrap()]);
    assert!(ok && d.join("b.md").exists(), "{log}");
    let (ok, log) = run(&["gourge", d.join("c.txt").to_str().unwrap()]);
    assert!(ok && d.join("c.md").exists(), "{log}");
    // regurgitate: the index.
    let (ok, log) = run(&["regurgitate", d.to_str().unwrap()]);
    assert!(ok && d.join("INDEX.md").exists(), "{log}");
}

#[test]
fn paths_can_be_piped_in_the_way_terminals_write_dropped_files() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().unwrap();
    let spaced = dir.path().join("My Report.txt");
    std::fs::write(&spaced, "A short report about pets.\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_doc-monster"))
        .arg("gorge")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Quoted, as Linux and Windows terminals write a dropped file.
    writeln!(child.stdin.take().unwrap(), "'{}'", spaced.display()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.path().join("My Report.md").exists());
}

#[test]
fn nothing_to_eat_is_an_error_not_a_hang() {
    let out = Command::new(env!("CARGO_BIN_EXE_doc-monster"))
        .arg("gorge")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

fn has(program: &str) -> bool {
    Command::new(program).arg("-version").output().is_ok()
        || Command::new(program).arg("--version").output().is_ok()
}

#[test]
fn video_on_screen_text_is_read_and_merged_with_what_is_said() {
    if !has("ffmpeg") || !has("tesseract") {
        eprintln!("skipped: needs FFmpeg and Tesseract");
        return;
    }
    let out = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/slides.mkv");
    let (ok, log) = run(&[
        fixture.to_str().unwrap(),
        "-o",
        out.path().to_str().unwrap(),
    ]);
    assert!(ok, "{log}");
    let md = std::fs::read_to_string(out.path().join("slides.md")).unwrap();
    // Every slide, once (the slide shown twice is merged), with misread bullets repaired.
    for needle in [
        "> Aurora Dev Update",
        "> DataStore saving",
        "> - Wrap every call in pcall",
        "> Pet trading",
        "> - Two step confirmation",
    ] {
        assert!(md.contains(needle), "missing {needle:?}:\n{md}");
    }
    assert_eq!(md.matches("> DataStore saving").count(), 1, "{md}");
    // Code on screen becomes a code block.
    assert!(md.contains("```\nlocal ok, err = pcall(function()"), "{md}");
    // Speech and screens interleaved in time order.
    let said = md.find("First, how we save player data safely.").unwrap();
    let saving = md.find("> DataStore saving").unwrap();
    let trading = md.find("> Pet trading").unwrap();
    assert!(saving < said && said < trading, "{md}");
}

#[test]
fn missing_tesseract_is_a_clear_error() {
    if !has("ffmpeg") {
        eprintln!("skipped: needs FFmpeg");
        return;
    }
    // A PATH holding FFmpeg but not Tesseract.
    let bin = tempfile::tempdir().unwrap();
    for tool in ["ffmpeg", "ffprobe"] {
        let real = String::from_utf8(
            Command::new("sh")
                .args(["-c", &format!("command -v {tool}")])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(real.trim(), bin.path().join(tool)).unwrap();
    }
    let out = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/slides.mkv");
    let result = Command::new(env!("CARGO_BIN_EXE_doc-monster"))
        .args([
            fixture.to_str().unwrap(),
            "-o",
            out.path().to_str().unwrap(),
        ])
        .env("PATH", bin.path())
        .output()
        .unwrap();
    // Tesseract is required: the video fails with a message that says what to install.
    assert_eq!(result.status.code(), Some(1));
    let log = String::from_utf8_lossy(&result.stderr);
    assert!(
        log.contains("videos need Tesseract installed (see INSTALL.md)"),
        "{log}"
    );
    assert!(!out.path().join("slides.md").exists());
}

#[test]
fn a_frame_limit_is_reported_never_silent() {
    if !has("ffmpeg") || !has("tesseract") {
        eprintln!("skipped: needs FFmpeg and Tesseract");
        return;
    }
    let out = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/slides.mkv");
    let (ok, log) = run(&[
        fixture.to_str().unwrap(),
        "-o",
        out.path().to_str().unwrap(),
        "--max-frames",
        "1",
    ]);
    assert!(ok, "{log}");
    let md = std::fs::read_to_string(out.path().join("slides.md")).unwrap();
    assert!(
        md.contains("on-screen text at 3 of 4 changed moments (limit set with --max-frames)"),
        "{md}"
    );
    // Without the limit every screen is read.
    let out = tempfile::tempdir().unwrap();
    let (ok, log) = run(&[
        fixture.to_str().unwrap(),
        "-o",
        out.path().to_str().unwrap(),
    ]);
    assert!(ok, "{log}");
    let md = std::fs::read_to_string(out.path().join("slides.md")).unwrap();
    assert!(
        md.contains("changed 4 times; text was read in 4") && !md.contains("--max-frames"),
        "{md}"
    );
}
