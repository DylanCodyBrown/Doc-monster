mod dropped;

// The static Linux build uses musl, whose memory allocator is slow under heavy use (the
// language model); mimalloc makes it as fast as the usual Linux build.
#[cfg(target_env = "musl")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;
mod ui;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use doc_monster::{clean, digest, extract, index, render, transcript};
use rayon::prelude::*;
use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

thread_local! {
    /// Message of the last crash inside a reader on this thread, reported for that file alone.
    static LAST_CRASH: RefCell<String> = const { RefCell::new(String::new()) };
}

use walkdir::WalkDir;

/// Doc Monster: eats documents, writes clean Markdown with a hierarchical summary beside each one.
///
///   doc-monster feed report.docx          eat one file or folder
///   doc-monster gorge a.pdf b.xlsx ./docs eat many at once
///   doc-monster regurgitate ./docs        build INDEX.md for a folder of Markdown
///   doc-monster                           drop mode: drag files onto the window, press Enter
///
/// Exit codes: 0 everything worked, 1 some files failed, 2 bad arguments or nothing to do.
#[derive(Parser)]
#[command(
    version,
    verbatim_doc_comment,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    eat: Args,
}

#[derive(Subcommand)]
enum Command {
    /// Eat one file or folder. Without a path, waits for you to drag one onto the window.
    Feed(Args),
    /// Eat many files and folders at once. Without paths, waits for you to drag them in.
    #[command(alias = "gourge")]
    Gorge(Args),
    /// Build INDEX.md at the top of each folder, listing every Markdown file inside once,
    /// with duplicates merged and Doc Monster's own duplicate copies removed.
    #[command(alias = "index")]
    Regurgitate(IndexArgs),
}

#[derive(clap::Args)]
struct IndexArgs {
    /// Folders to index (searched recursively).
    #[arg(required = true)]
    folders: Vec<PathBuf>,
    /// Keep Doc Monster's duplicate outputs on disk (they are still merged in the index).
    #[arg(long)]
    keep_duplicates: bool,
    /// Print a JSON report on standard output instead of text.
    #[arg(long)]
    json: bool,
    /// Print only errors.
    #[arg(short, long)]
    quiet: bool,
    /// Never draw the pet or the progress line.
    #[arg(long)]
    no_anim: bool,
}

#[derive(clap::Args, Clone)]
struct Args {
    /// Files or folders to eat (folders are searched recursively). Leave out to drag and drop.
    inputs: Vec<PathBuf>,
    /// Write all Markdown into this folder instead of beside each source file.
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// Replace an earlier Doc Monster output instead of writing a new .v2, .v3 copy.
    /// Files Doc Monster did not write are never replaced.
    #[arg(long)]
    overwrite: bool,
    /// Afterwards, build INDEX.md for each folder given (or for the --out folder).
    #[arg(long)]
    index: bool,
    /// Convert and clean only, without the summary.
    #[arg(long)]
    no_summary: bool,
    /// How compressed the output is, 1 to 10. 1: summary and the full document (default).
    /// 2-9: summary and the document shortened (roughly 85% of the words at 2, 45% at 5,
    /// 20% at 9; repeats removed first). 10: summary only.
    #[arg(short, long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=10))]
    compress: u8,
    /// Skip the Markdown cleanup pass (keep the converted text exactly as extracted).
    #[arg(long)]
    raw: bool,
    /// Most rows read per spreadsheet sheet or CSV file.
    #[arg(long, default_value_t = 5000)]
    max_rows: usize,
    /// Language of text in videos and pictures, as Tesseract codes: eng, spa, deu, or several
    /// like eng+spa.
    #[arg(long, default_value = "eng")]
    screen_lang: String,
    /// Read on-screen text in at most this many video frames (default: every frame where the
    /// screen changed; anything skipped is listed under Not included).
    #[arg(long)]
    max_frames: Option<usize>,
    /// Read the file as a meeting transcript ("Name: ..." lines, Teams or Zoom captions) even
    /// when it does not look like one. Longer transcripts are recognised automatically.
    #[arg(long)]
    transcript: bool,
    /// How transcripts are summarised. rules: topics, decisions, action items and open
    /// questions, found in the words people said. model: the same, plus a summary of each topic
    /// written by a language model running on this computer (Qwen2.5 1.5B Instruct, about
    /// 1.1 GB, downloaded once; a few seconds per topic).
    #[arg(long, value_enum, default_value_t = Summarizer::Rules)]
    summarizer: Summarizer,
    /// A folder that already holds the model files, for computers without internet access.
    #[arg(long, value_name = "FOLDER")]
    model_dir: Option<PathBuf>,
    /// Print a JSON report on standard output instead of text.
    #[arg(long)]
    json: bool,
    /// Print only errors and the final count.
    #[arg(short, long)]
    quiet: bool,
    /// Never draw the pet or the progress line (they only appear in an interactive terminal anyway).
    #[arg(long)]
    no_anim: bool,
}

#[derive(Serialize)]
struct Indexed {
    folder: String,
    index: String,
    files: usize,
    unique: usize,
    deleted: Vec<String>,
}

#[derive(Serialize)]
struct Outcome {
    source: String,
    output: Option<String>,
    ok: bool,
    /// True when a summary was written (front matter says "Condensed").
    condensed: bool,
    error: Option<String>,
    /// Left alone on purpose, not a failure (a picture with no text in it).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    skipped: bool,
}

#[derive(Serialize)]
struct Report {
    converted: usize,
    failed: usize,
    /// Earlier Doc Monster outputs found among the inputs and left alone.
    skipped: usize,
    /// Pictures with no readable text, left alone.
    #[serde(skip_serializing_if = "is_zero")]
    without_text: usize,
    results: Vec<Outcome>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    indexes: Vec<Indexed>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Summarizer {
    Rules,
    Model,
}

/// The language model for transcript summaries: loaded (and downloaded) once, on first use.
#[cfg(feature = "model")]
fn model_for(args: &Args) -> Result<Option<&'static dyn transcript::analyze::Rewriter>> {
    use doc_monster::model::Qwen;
    static MODEL: std::sync::OnceLock<std::result::Result<Qwen, String>> =
        std::sync::OnceLock::new();
    if args.summarizer != Summarizer::Model {
        return Ok(None);
    }
    match MODEL.get_or_init(|| Qwen::load(args.model_dir.as_deref()).map_err(|e| format!("{e:#}")))
    {
        Ok(m) => Ok(Some(m)),
        Err(e) => bail!("the summarizing model could not be loaded: {e}"),
    }
}

#[cfg(not(feature = "model"))]
fn model_for(args: &Args) -> Result<Option<&'static dyn transcript::analyze::Rewriter>> {
    if args.summarizer == Summarizer::Model {
        bail!(
            "this copy of Doc Monster was built without the language model; the releases and \
             `uv tool install doc-monster` include it (or build with --features model)"
        );
    }
    Ok(None)
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            2
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    install_crash_hook();
    let code = match cli.command {
        Some(Command::Regurgitate(a)) => run_index(&a),
        Some(Command::Feed(a)) if a.inputs.len() > 1 => {
            bail!("feed eats one file or folder; use gorge for several")
        }
        Some(Command::Feed(a)) | Some(Command::Gorge(a)) => eat_or_wait(a),
        // Bare paths, e.g. files dropped onto the program's icon: same as gorge.
        None => eat_or_wait(cli.eat),
    };
    hold_window_if_opened_by_drop();
    code
}

/// With paths, eat them. Without: drop mode in a terminal, or read paths piped in.
fn eat_or_wait(args: Args) -> Result<i32> {
    if !args.inputs.is_empty() {
        return run_eat(&args);
    }
    if std::io::stdin().is_terminal() {
        return drop_mode(args);
    }
    let mut piped = args.clone();
    for line in std::io::stdin().lock().lines() {
        piped.inputs.extend(dropped::parse(&line?));
    }
    if piped.inputs.is_empty() {
        bail!("nothing to eat: give a path, or run in a terminal and drag files in");
    }
    run_eat(&piped)
}

/// Interactive: the pet waits beside the prompt; each line of dragged-in paths is eaten when
/// Enter is pressed.
fn drop_mode(args: Args) -> Result<i32> {
    let ui = ui::Ui::new(!args.no_anim && !args.json && !args.quiet);
    if !ui.shows_pet() {
        println!(
            "Drag files or folders onto this window and press Enter. Empty line or q to quit."
        );
    }
    let mut worst = 0;
    let (mut mood, mut status) = (ui::Mood::Hungry, ui.hungry());
    let stdin = std::io::stdin();
    loop {
        ui.pet_prompt(mood, &status);
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            // End of input (Ctrl+D): the cursor is still on the prompt row.
            println!();
            ui.clear_pet("");
            break;
        }
        // One pet only: erase it here; it is drawn again below what this batch prints.
        ui.clear_pet(&line);
        if matches!(line.trim(), "" | "q" | "quit" | "exit") {
            break;
        }
        let mut batch = args.clone();
        batch.inputs = dropped::parse(&line);
        ui.fed(&batch.inputs);
        match eat(&batch, &ui) {
            Ok(b) => {
                worst = worst.max(b.code);
                (mood, status) = ui.after_meal(b.converted, b.failed);
            }
            Err(e) => {
                eprintln!("error: {e:#}");
                worst = worst.max(1);
                (mood, status) = ui.after_meal(0, 0);
            }
        }
    }
    Ok(worst)
}

/// On Windows, files dropped onto the program's icon open a console that would close the
/// moment the work is done. Keep it open so the results can be read.
fn hold_window_if_opened_by_drop() {
    if console_is_ours_alone() && std::io::stdin().is_terminal() {
        print!("Press Enter to close.");
        std::io::stdout().flush().ok();
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
    }
}

#[cfg(windows)]
fn console_is_ours_alone() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    let mut ids = [0u32; 4];
    // Only this process attached means Windows made the console for us (no shell behind it).
    unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) == 1 }
}

#[cfg(not(windows))]
fn console_is_ours_alone() -> bool {
    false
}

fn install_crash_hook() {
    // A bug in any reader (ours or a library's) must cost one file, never the whole batch.
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown error".into());
        let at = info
            .location()
            .map(|l| {
                format!(
                    " at {}:{}",
                    l.file().rsplit(['/', '\\']).next().unwrap_or(""),
                    l.line()
                )
            })
            .unwrap_or_default();
        LAST_CRASH.with(|last| *last.borrow_mut() = format!("{msg}{at}"));
    }));
}

/// Build (and write) the index for each folder; deletes our duplicate outputs unless told not to.
fn build_indexes(
    folders: &[PathBuf],
    keep_duplicates: bool,
    quiet: bool,
    json: bool,
    ui: &ui::Ui,
) -> Result<Vec<Indexed>> {
    let mut done = Vec::new();
    for folder in folders {
        let plan = index::plan(folder)?;
        let mut deleted = Vec::new();
        if !keep_duplicates {
            for p in &plan.duplicates_to_delete {
                // Belt and braces: only ever delete files Doc Monster wrote.
                if is_ours(p) && std::fs::remove_file(p).is_ok() {
                    deleted.push(p.display().to_string());
                }
            }
        }
        let markdown = if keep_duplicates {
            plan.markdown
                .replace("Duplicate copies removed:", "Duplicate copies:")
                .replace(
                    "duplicate Doc Monster outputs removed",
                    "duplicate Doc Monster outputs kept",
                )
        } else {
            plan.markdown
        };
        std::fs::write(&plan.path, markdown)
            .with_context(|| format!("writing {}", plan.path.display()))?;
        if !quiet && !json {
            let extra = if deleted.is_empty() {
                String::new()
            } else {
                format!(", {} duplicate outputs removed", deleted.len())
            };
            ui.indexed(&plan.path, plan.files, plan.unique, &extra);
        }
        done.push(Indexed {
            folder: folder.display().to_string(),
            index: plan.path.display().to_string(),
            files: plan.files,
            unique: plan.unique,
            deleted,
        });
    }
    Ok(done)
}

fn run_index(a: &IndexArgs) -> Result<i32> {
    let ui = ui::Ui::new(!a.no_anim && !a.json && !a.quiet);
    let done = ui.working(a.folders.len(), |t| {
        t.start("Markdown files");
        build_indexes(&a.folders, a.keep_duplicates, a.quiet, a.json, &ui)
    })?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&done)?);
    }
    Ok(0)
}

/// What a batch did, for the exit code and the pet's mood.
struct Batch {
    code: i32,
    converted: usize,
    failed: usize,
}

fn run_eat(args: &Args) -> Result<i32> {
    let ui = ui::Ui::new(!args.no_anim && !args.json && !args.quiet);
    Ok(eat(args, &ui)?.code)
}

fn eat(args: &Args, ui: &ui::Ui) -> Result<Batch> {
    let found = collect_files(&args.inputs);
    if found.files.is_empty() && found.missing.is_empty() {
        bail!("no documents found in the given inputs");
    }

    let started = std::time::Instant::now();
    let opts = extract::Options {
        max_rows: args.max_rows,
        screen_lang: args.screen_lang.clone(),
        max_frames: args.max_frames.map(|m| m.max(1)),
        transcript: args.transcript,
    };

    let mut results: Vec<Outcome> = found
        .missing
        .iter()
        .map(|p| Outcome {
            source: p.display().to_string(),
            output: None,
            ok: false,
            condensed: false,
            error: Some("not found".into()),
            skipped: false,
        })
        .collect();
    for p in &found.missing {
        if !args.json {
            ui.spat(p, "not found");
        }
    }

    // Output names are decided up front, so files converted in parallel never race for a name.
    let plan = plan_outputs(&found.files, args);
    let convert_one = |(path, out): (&PathBuf, &Result<PathBuf>)| -> Result<(PathBuf, bool)> {
        let out = out.as_ref().map_err(|e| anyhow!("{e:#}"))?;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process(path, out, args, &opts)
        }))
        .unwrap_or_else(|_| {
            let why = LAST_CRASH.with(|m| m.borrow().clone());
            Err(anyhow!("the reader crashed on this file ({why})"))
        })
    };
    // Every processor core at once, with one progress line for the batch.
    let done: Vec<Result<(PathBuf, bool)>> = ui.working(found.files.len(), |tracker| {
        found
            .files
            .par_iter()
            .zip(plan.par_iter())
            .map(|job| {
                tracker.start(
                    job.0
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or("document"),
                );
                let r = convert_one(job);
                tracker.finish();
                r
            })
            .collect()
    });
    for (path, result) in found.files.iter().zip(done) {
        results.push(match result {
            Ok((out, condensed)) => {
                if !args.json && !args.quiet {
                    ui.ate(path, &out);
                }
                Outcome {
                    source: path.display().to_string(),
                    output: Some(out.display().to_string()),
                    ok: true,
                    condensed,
                    error: None,
                    skipped: false,
                }
            }
            Err(e) if e.downcast_ref::<extract::Skipped>().is_some() => {
                if !args.json && !args.quiet {
                    ui.skipped(path, &format!("{e:#}"));
                }
                Outcome {
                    source: path.display().to_string(),
                    output: None,
                    ok: false,
                    condensed: false,
                    error: Some(format!("{e:#}")),
                    skipped: true,
                }
            }
            Err(e) => {
                if !args.json {
                    ui.spat(path, &format!("{e:#}"));
                }
                Outcome {
                    source: path.display().to_string(),
                    output: None,
                    ok: false,
                    condensed: false,
                    error: Some(format!("{e:#}")),
                    skipped: false,
                }
            }
        });
    }

    let converted = results.iter().filter(|r| r.ok).count();
    let without_text = results.iter().filter(|r| r.skipped).count();
    let failed = results.len() - converted - without_text;
    // With --index, each folder given (or the --out folder) gets its INDEX.md afterwards.
    let indexes = if args.index {
        let folders: Vec<PathBuf> = match &args.out {
            Some(o) => vec![o.clone()],
            None => args.inputs.iter().filter(|p| p.is_dir()).cloned().collect(),
        };
        build_indexes(&folders, false, args.quiet, args.json, ui)?
    } else {
        Vec::new()
    };
    if args.json {
        let report = Report {
            converted,
            failed,
            skipped: found.skipped,
            without_text,
            results,
            indexes,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        ui.done(
            converted,
            failed,
            found.skipped,
            without_text,
            started.elapsed(),
        );
    }
    Ok(Batch {
        code: if failed > 0 { 1 } else { 0 },
        converted,
        failed,
    })
}

const VIDEO: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "mpg", "mpeg", "ogv",
];

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// True for Markdown that Doc Monster wrote itself (recognized by its front matter comment).
fn is_ours(path: &Path) -> bool {
    use std::io::Read;
    if !matches!(ext_of(path).as_str(), "md" | "markdown") {
        return false;
    }
    let mut head = [0u8; 64];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    let text = String::from_utf8_lossy(&head[..n]).replace("\r\n", "\n");
    text.starts_with("---\n# Condensed with Doc Monster")
        || text.starts_with("---\n# Converted with Doc Monster")
        || text.starts_with(&format!("---\n{}", index::MARKER))
}

struct Found {
    files: Vec<PathBuf>,
    missing: Vec<PathBuf>,
    skipped: usize,
}

fn collect_files(inputs: &[PathBuf]) -> Found {
    let mut found = Found {
        files: Vec::new(),
        missing: Vec::new(),
        skipped: 0,
    };
    for input in inputs {
        if !input.exists() {
            found.missing.push(input.clone());
            continue;
        }
        for entry in WalkDir::new(input)
            .sort_by_file_name()
            .into_iter()
            .flatten()
        {
            // Links to files count; links to folders are not followed (they can loop).
            if entry.file_type().is_file() || (entry.path_is_symlink() && entry.path().is_file()) {
                found.files.push(entry.into_path());
            }
        }
    }
    // Our own earlier outputs are not documents to eat again.
    let before = found.files.len();
    found.files.retain(|p| !is_ours(p));
    found.skipped = before - found.files.len();
    // A subtitle file beside a video of the same name is that video's transcript.
    let videos: std::collections::HashSet<PathBuf> = found
        .files
        .iter()
        .filter(|p| VIDEO.contains(&ext_of(p).as_str()))
        .map(|p| p.with_extension(""))
        .collect();
    found.files.retain(|p| {
        let is_sub = matches!(ext_of(p).as_str(), "srt" | "vtt");
        !(is_sub && videos.contains(&p.with_extension("")))
    });
    found
}

fn plan_outputs(files: &[PathBuf], args: &Args) -> Vec<Result<PathBuf>> {
    let mut reserved = HashSet::new();
    files
        .iter()
        .map(|f| output_path(f, args, &mut reserved))
        .collect()
}

/// `report.md` beside the source; `report.v2.md`, `report.v3.md` ... when the name is taken.
fn output_path(source: &Path, args: &Args, reserved: &mut HashSet<PathBuf>) -> Result<PathBuf> {
    let dir = match &args.out {
        Some(d) => {
            std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
            d.clone()
        }
        None => source.parent().map_or_else(PathBuf::new, Path::to_path_buf),
    };
    // ".env" has no stem; hidden files should not produce hidden output files.
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.trim_start_matches('.'))
        .filter(|s| !s.is_empty())
        .unwrap_or("document");
    for n in 1.. {
        let candidate = if n == 1 {
            dir.join(format!("{stem}.md"))
        } else {
            dir.join(format!("{stem}.v{n}.md"))
        };
        let same_as_source =
            candidate.exists() && candidate.canonicalize().ok() == source.canonicalize().ok();
        let free =
            !candidate.exists() || (args.overwrite && !same_as_source && is_ours(&candidate));
        if free && reserved.insert(candidate.clone()) {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn process(
    path: &Path,
    out: &Path,
    args: &Args,
    opts: &extract::Options,
) -> Result<(PathBuf, bool)> {
    let mut doc = extract::extract_with(path, opts)?;
    // A document holding only pictures or equations still gets a file saying so.
    if doc.markdown.trim().is_empty() && doc.summary.is_none() && doc.omitted.is_empty() {
        bail!("document has no text");
    }
    // A transcript: summarised as a conversation; the body becomes the turns by topic.
    let conversation = match doc.transcript.take() {
        Some(t) if !args.no_summary => {
            let s = transcript::analyze::summarize(&t, model_for(args)?);
            doc.markdown = s.body;
            doc.omitted.extend(s.notes);
            Some((s.markdown, Some(s.description)))
        }
        _ => None,
    };
    let mut note = None;
    if !args.raw && conversation.is_none() {
        let (cleaned, report) = clean::clean(&doc.markdown);
        doc.markdown = cleaned;
        note = report.describe();
    }
    // (summary text, most representative sentence)
    let summary: Option<(String, Option<String>)> = if args.no_summary {
        None
    } else if conversation.is_some() {
        conversation.clone()
    } else if let Some(s) = doc.summary.clone() {
        // The reader already described the file (e.g. the shape of a data file).
        Some((s, None))
    } else {
        digest::digest(&doc.markdown, note.as_deref()).map(|d| (d.markdown, d.lead))
    };
    let description = conversation
        .as_ref()
        .and_then(|(_, lead)| lead.clone())
        .or_else(|| doc.description.clone())
        .or_else(|| summary.as_ref().and_then(|(_, lead)| lead.clone()))
        .or_else(|| digest::first_sentence(&doc.markdown))
        .map(|d| render::tidy_description(&d))
        .unwrap_or_default();
    // The body at the requested compression: whole, shortened, or (level 10) none.
    let level = args.compress;
    let body: Option<String> = match level {
        1 => Some(doc.markdown.clone()),
        10 if summary.is_some() => None,
        // Too short to summarise: the whole (short) text is already as small as it gets.
        10 => Some(doc.markdown.clone()),
        _ => {
            let c = digest::compress(&doc.markdown, level);
            // Nothing left but headings (a spreadsheet is only tables): leave the body out.
            let only_headings = c
                .markdown
                .lines()
                .all(|l| l.trim().is_empty() || l.starts_with('#'));
            if only_headings && summary.is_some() {
                doc.omitted.push(format!(
                    "the document body: its {} (compression {level} of 10 keeps tables up to level 3)",
                    if c.tables_dropped == 1 {
                        "table".to_string()
                    } else {
                        format!("{} tables", c.tables_dropped)
                    }
                ));
                None
            } else {
                Some(compressed_body(&mut doc, c, level))
            }
        }
    };
    if level == 10 && body.is_none() {
        doc.omitted
            .push("the document body (compression 10 of 10: summary only)".into());
    }
    let rendered = render::render(
        &doc,
        &description,
        summary.as_ref().map(|(t, _)| t.as_str()),
        body.as_deref(),
        level,
    );

    std::fs::write(out, rendered).with_context(|| format!("writing {}", out.display()))?;
    Ok((out.to_path_buf(), summary.is_some()))
}

/// The shortened body, noting in the document what was cut.
fn compressed_body(doc: &mut doc_monster::Document, c: digest::Compressed, level: u8) -> String {
    if c.words_after < c.words_before && c.words_before > 0 {
        let cut = 100 - (c.words_after * 100 / c.words_before);
        doc.omitted.push(format!(
            "about {cut}% of the words (compression {level} of 10)"
        ));
    }
    if c.tables_dropped > 0 {
        doc.omitted.push(format!(
            "{} table{}",
            c.tables_dropped,
            if c.tables_dropped == 1 { "" } else { "s" }
        ));
    }
    if c.code_dropped > 0 {
        doc.omitted.push(format!(
            "{} code block{}",
            c.code_dropped,
            if c.code_dropped == 1 { "" } else { "s" }
        ));
    }
    c.markdown
}
