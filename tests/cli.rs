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

// Builds a PATH holding FFmpeg but not Tesseract out of symbolic links, which needs Unix.
// A build with its own FFmpeg and Tesseract never misses them.
#[cfg(all(unix, not(feature = "bundled-tools")))]
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

#[cfg(unix)]
#[test]
fn files_reached_through_a_link_are_eaten() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::create_dir(d.join("real")).unwrap();
    std::fs::create_dir(d.join("links")).unwrap();
    std::fs::write(d.join("real/pets.txt"), "A short report about pets.\n").unwrap();
    std::os::unix::fs::symlink(d.join("real/pets.txt"), d.join("links/pets.txt")).unwrap();
    // Given directly, and found inside a folder.
    let (ok, log) = run(&["feed", d.join("links/pets.txt").to_str().unwrap(), "-q"]);
    assert!(ok && d.join("links/pets.md").exists(), "{log}");
    std::fs::remove_file(d.join("links/pets.md")).unwrap();
    let (ok, log) = run(&["feed", d.join("links").to_str().unwrap(), "-q"]);
    assert!(ok && d.join("links/pets.md").exists(), "{log}");
}

#[test]
fn spreadsheets_and_csv_files_are_described_by_structure_never_by_their_values() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(
        d.join("sales.csv"),
        "Name,Region,Revenue\nAcme,North,1200\nGlobex,South,900\n",
    )
    .unwrap();
    let (ok, log) = run(&["feed", d.join("sales.csv").to_str().unwrap(), "-q"]);
    assert!(ok, "{log}");
    let md = std::fs::read_to_string(d.join("sales.md")).unwrap();
    assert!(
        md.contains(
            "description: \"CSV file, 2 rows × 3 columns; columns Name, Region, Revenue.\""
        ),
        "{md}"
    );
    let summary = &md[md.find("## Summary").unwrap()..md.find("## Full content").unwrap()];
    assert!(
        summary.contains("Columns: `Name`, `Region`, `Revenue`"),
        "{summary}"
    );
    for value in ["Acme", "1200", "900", "average", "most common"] {
        assert!(!summary.contains(value), "{value} in {summary}");
    }
    // The table itself is still in the body.
    assert!(md.contains("| Acme | North | 1200 |"), "{md}");

    // Compressed past the level that keeps tables: no empty body, and it says why.
    let (ok, log) = run(&[
        "feed",
        d.join("sales.csv").to_str().unwrap(),
        "-q",
        "-c",
        "5",
        "--overwrite",
    ]);
    assert!(ok, "{log}");
    let md = std::fs::read_to_string(d.join("sales.md")).unwrap();
    assert!(!md.contains("## Compressed content"), "{md}");
    assert!(md.contains("the document body: its table"), "{md}");
}

#[test]
fn text_in_pictures_is_read_and_pictures_without_text_are_skipped() {
    if !has("ffmpeg") || !has("tesseract") {
        eprintln!("skipped: FFmpeg or Tesseract not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::copy(fixtures.join("sign.png"), d.join("sign.png")).unwrap();
    std::fs::copy(fixtures.join("sign.jpg"), d.join("notice.jpg")).unwrap();
    // A plain picture with no text in it.
    let made = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=skyblue:s=200x120",
        ])
        .args(["-frames:v", "1"])
        .arg(d.join("photo.png"))
        .status()
        .unwrap();
    assert!(made.success());

    let (ok, log) = run(&["gorge", d.to_str().unwrap(), "--json"]);
    assert!(ok, "a picture without text is not a failure: {log}");
    let report: serde_json::Value = serde_json::from_str(&log).unwrap();
    assert_eq!(report["converted"], 2, "{log}");
    assert_eq!(report["failed"], 0, "{log}");
    assert_eq!(report["without_text"], 1, "{log}");
    assert!(!d.join("photo.md").exists(), "nothing is written for it");

    for name in ["sign.md", "notice.md"] {
        let md = std::fs::read_to_string(d.join(name)).unwrap();
        assert!(
            md.contains("title: \"Aurora Pet Shop Notice\""),
            "{name}: {md}"
        );
        assert!(md.contains("# Aurora Pet Shop Notice"), "{name}: {md}");
        assert!(
            md.contains("Pets hatch from eggs bought with coins earned in the arena."),
            "{name}: wrapped lines joined: {md}"
        );
        assert!(md.contains("the picture itself"), "{name}: {md}");
    }
}

/// The test meeting as text, Teams captions and Zoom captions: the same summary from each.
#[test]
fn meeting_transcripts_from_text_teams_and_zoom_are_summarised_alike() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for f in ["meeting.txt", "meeting-teams.vtt", "meeting-zoom.vtt"] {
        std::fs::copy(fixtures.join(f), d.join(f)).unwrap();
    }
    let (ok, log) = run(&["gorge", d.to_str().unwrap(), "-q"]);
    assert!(ok, "{log}");
    let summary = |name: &str| {
        let md = std::fs::read_to_string(d.join(name)).unwrap();
        md[md.find("## Goal").unwrap()..md.find("## Transcript").unwrap()].to_string()
    };
    let text = summary("meeting.md");
    assert_eq!(text, summary("meeting-teams.md"));
    assert_eq!(text, summary("meeting-zoom.md"));
    for expected in [
        "## Goal\n\n“Three things today: the DataStore saving outage from Tuesday",
        "## Outcome\n\n**2 decisions · 6 action items · 1 open question · 100% on topic**",
        "### Decisions\n- “We decided to give every affected player a free rare egg.” · Maya · 2:26",
        "- **Jordan**: “I'll have the session locking done by Thursday.” · due Thursday · about outage",
        "- **Lee**: “Lee, can you set that up this week?” → “Sure, I'll wire it into the dashboard by Wednesday.” · due Wednesday",
        "### Open questions\n- “Do rare pets keep their stats after trading?” · Sam · 4:20",
        "### Outage · 0:04–2:33 · Maya, Jordan, Sam, Lee",
        "### Rare pet balance · 3:02–4:36",
        "### Hatching update release · 5:10–6:15",
        "### Who spoke\n- **Jordan Lee** · 33% of the words",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
    // The front matter: the meeting's details, read from the lines above the conversation.
    let md = std::fs::read_to_string(d.join("meeting.md")).unwrap();
    for expected in [
        "title: \"Aurora weekly sync\"\n",
        "description: \"Aurora weekly sync (6 min, 4 attendees) on outage, rare pet balance, hatching update release: 2 decisions, 6 action items and 1 open question; 100% on topic.\"\n",
        "date: \"2026-11-10\"\ntime: \"09:30\"\nduration: \"6 min\"\nplace: \"\"\n",
        "topic: \"Outage, rare pet balance, hatching update release\"\n",
        "attendees:\n  - \"Maya Lopez\"\n  - \"Jordan Lee\"\n  - \"Sam Park\"\n  - \"Lee Chen\"\n---",
    ] {
        assert!(md.contains(expected), "missing {expected:?} in:\n{md}");
    }
    // Teams captions say where the meeting was held.
    assert!(
        std::fs::read_to_string(d.join("meeting-teams.md"))
            .unwrap()
            .contains("place: \"Microsoft Teams\"")
    );
    // The full conversation follows, by subject.
    assert!(
        md.contains(
            "## Transcript\n\n### Outage (0:04–2:33)\n\n**Maya Lopez** (0:04): Morning everyone."
        ),
        "{md}"
    );
    assert!(
        md.contains("### Rare pet balance (3:02–4:36)\n\n**Sam Park** (3:02): Next topic"),
        "{md}"
    );
}

/// Meetings that wander: the football and the lunch plans go under Miscellaneous, the real
/// outcome stays on top, and a meeting about nothing says so.
#[test]
fn off_topic_and_empty_meetings() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for f in ["meeting-offtopic.txt", "meeting-nothing.txt"] {
        std::fs::copy(fixtures.join(f), d.join(f)).unwrap();
    }
    let (ok, log) = run(&["gorge", d.to_str().unwrap(), "-q"]);
    assert!(ok, "{log}");
    let wandering = std::fs::read_to_string(d.join("meeting-offtopic.md")).unwrap();
    for expected in [
        "place: \"Microsoft Teams\"",
        "## Goal\n\nAgree the Q3 roadmap priorities and the launch budget _(from the meeting's agenda)_",
        "**2 decisions · 1 action item · 1 open question · 33% on topic**",
        "- “Trading moves to Q4.” · Priya · 2:25",
        "- **Dana**: “Dana, can you write up the guild system scope?” → “Sure, I'll send the scope document by Friday.” · due Friday",
        "- “What's the launch budget looking like? Do we have numbers from finance yet?” · Tom · 2:38",
        "### Off topic · 50% of the meeting\n- **0:03–0:50** · ",
        "\n- **2:51–3:41** · ",
        "### Buzzwords · 16% of the meeting was mostly buzzwords",
    ] {
        assert!(
            wandering.contains(expected),
            "missing {expected:?} in:\n{wandering}"
        );
    }
    let outcome = &wandering
        [wandering.find("## Outcome").unwrap()..wandering.find("## Miscellaneous").unwrap()];
    assert!(
        !outcome.contains("match") && !outcome.contains("pizza") && !outcome.contains("table"),
        "{outcome}"
    );

    let nothing = std::fs::read_to_string(d.join("meeting-nothing.md")).unwrap();
    for expected in [
        "description: \"Weekly touch base (2 min, 3 attendees): no decisions, action items or open questions; 0% on topic.\"",
        "_Stated only in general terms: no concrete subject._",
        "**No decisions, action items or open questions · 0% on topic**",
        "Most of the meeting (100%) was small talk or buzzwords.",
        "_No on-topic discussion to summarize._",
        "### Buzzwords · 66% of the meeting was mostly buzzwords\n`touch base` ×2",
    ] {
        assert!(
            nothing.contains(expected),
            "missing {expected:?} in:\n{nothing}"
        );
    }
}

#[test]
fn teams_word_transcripts_and_short_chats_with_transcript_flag() {
    use docx_rs::{Docx, Paragraph, Run};
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    // A Teams .docx transcript: the speaker and time on one line, their words below.
    let mut doc =
        Docx::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text("Weekly sync")));
    for (who, at, said) in [
        ("Maya Lopez", "0:03", "Morning. The outage first."),
        (
            "Jordan Lee",
            "0:10",
            "Saves were throttled on Tuesday. I'll have the fix done by Thursday.",
        ),
        (
            "Maya Lopez",
            "0:25",
            "Good. Let's go with the free rare egg for affected players.",
        ),
        ("Sam Park", "0:31", "I'll post the announcement tomorrow."),
        ("Jordan Lee", "0:40", "Do traded pets keep their stats?"),
        ("Maya Lopez", "0:44", "Good question, nobody knows yet."),
    ] {
        doc = doc
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text(format!("{who}   {at}"))))
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text(said)));
    }
    doc.build()
        .pack(std::fs::File::create(d.join("teams.docx")).unwrap())
        .unwrap();
    let (ok, log) = run(&["feed", d.join("teams.docx").to_str().unwrap(), "-q"]);
    assert!(ok, "{log}");
    let md = std::fs::read_to_string(d.join("teams.md")).unwrap();
    assert!(
        md.contains("title: \"Weekly sync\"\n") && md.contains("duration: \"41 s\"\n"),
        "{md}"
    );
    assert!(
        md.contains("**1 decision · 2 action items · 1 open question · 100% on topic**"),
        "{md}"
    );
    assert!(
        md.contains("- **Jordan**: “I'll have the fix done by Thursday.” · due Thursday"),
        "{md}"
    );
    assert!(
        md.contains("- “Let's go with the free rare egg for affected players.” · Maya · 0:25"),
        "{md}"
    );
    assert!(
        md.contains("- “Do traded pets keep their stats?” · Jordan · 0:40"),
        "{md}"
    );

    // Too short to be recognised on its own; --transcript reads it as a conversation anyway.
    std::fs::write(
        d.join("chat.txt"),
        "Maya: Can you send the notes?\nJordan: Sure, I'll send them tonight.\n",
    )
    .unwrap();
    let (ok, log) = run(&["feed", d.join("chat.txt").to_str().unwrap(), "-q"]);
    assert!(ok, "{log}");
    assert!(
        !std::fs::read_to_string(d.join("chat.md"))
            .unwrap()
            .contains("## Outcome")
    );
    let (ok, log) = run(&[
        "feed",
        d.join("chat.txt").to_str().unwrap(),
        "-q",
        "--transcript",
        "--overwrite",
    ]);
    assert!(ok, "{log}");
    let md = std::fs::read_to_string(d.join("chat.md")).unwrap();
    assert!(
        md.contains(
            "- **Jordan**: “Can you send the notes?” → “Sure, I'll send them tonight.” · due tonight"
        ),
        "{md}"
    );
}

/// The language model, when this build has it and the model files are on hand
/// (DOC_MONSTER_MODEL_DIR): every topic gets a model summary, labelled as such.
#[test]
fn model_summaries_when_the_model_is_available() {
    let Some(model_dir) = std::env::var_os("DOC_MONSTER_MODEL_DIR") else {
        eprintln!("skipped: set DOC_MONSTER_MODEL_DIR to a folder with the model files");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::copy(fixtures.join("meeting.txt"), d.join("meeting.txt")).unwrap();
    let (ok, log) = run(&[
        "feed",
        d.join("meeting.txt").to_str().unwrap(),
        "-q",
        "--summarizer",
        "model",
        "--model-dir",
        Path::new(&model_dir).to_str().unwrap(),
    ]);
    if log.contains("built without the language model") {
        eprintln!("skipped: this build has no model");
        return;
    }
    assert!(ok, "{log}");
    let md = std::fs::read_to_string(d.join("meeting.md")).unwrap();
    assert!(
        md.contains("Written by a language model (Qwen2.5 1.5B Instruct)"),
        "{md}"
    );
    // The model's prose replaces the quoted key sentences under each subject.
    let topics = &md[md.find("## Summary").unwrap()..md.find("## Miscellaneous").unwrap()];
    assert_eq!(topics.matches("\n### ").count(), 3, "{topics}");
    assert!(
        !topics.contains("\n- "),
        "prose, not quoted lines: {topics}"
    );
    assert!(topics.to_lowercase().contains("saturday"), "{topics}");
}
