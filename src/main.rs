mod monster;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use doc_monster::{
    clean, digest, extract, render,
    summarize::{Ollama, Summarizer, summarize_document},
};
use rayon::prelude::*;
use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

thread_local! {
    /// Message of the last crash inside a reader on this thread, reported for that file alone.
    static LAST_CRASH: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Batches up to this size get a chomp per file; bigger ones get one monster for the lot.
const ANIMATE_EACH_UP_TO: usize = 12;
use walkdir::WalkDir;

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Backend {
    /// Notebook-style hierarchical digest computed locally: free, offline, no model.
    Digest,
    /// Summaries from a local Ollama model (optional, still free and offline).
    Ollama,
    /// Convert and clean only, no summary.
    None,
}

/// Doc Monster: eats documents, writes clean Markdown with a hierarchical summary beside each one.
///
/// Exit codes: 0 every file converted, 1 some files failed, 2 bad arguments or nothing to do.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Files or folders to consume (folders are searched recursively).
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Write all Markdown into this folder instead of beside each source file.
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// Replace an earlier Doc Monster output instead of writing a new .v2, .v3 copy.
    /// Files Doc Monster did not write are never replaced.
    #[arg(long)]
    overwrite: bool,
    #[arg(long, value_enum, default_value = "digest")]
    backend: Backend,
    /// Write only the summary, leaving out the full converted content.
    #[arg(long)]
    condensed: bool,
    /// Skip the Markdown cleanup pass (keep the converted text exactly as extracted).
    #[arg(long)]
    raw: bool,
    /// Most rows read per spreadsheet sheet or CSV file.
    #[arg(long, default_value_t = 5000)]
    max_rows: usize,
    /// Print a JSON report on standard output instead of text (for scripts and agents).
    #[arg(long)]
    json: bool,
    /// Print only errors and the final count.
    #[arg(short, long)]
    quiet: bool,
    /// Never draw the animated monster (it is only drawn in an interactive terminal anyway).
    #[arg(long)]
    no_anim: bool,
    /// Local model name as known to Ollama (only with --backend ollama).
    #[arg(long, default_value = "qwen2.5:1.5b")]
    model: String,
    #[arg(long, default_value = "http://localhost:11434")]
    ollama_url: String,
    /// Maximum characters sent to the model per request.
    #[arg(long, default_value_t = 6000)]
    chunk_chars: usize,
}

#[derive(Serialize)]
struct Outcome {
    source: String,
    output: Option<String>,
    ok: bool,
    /// True when a summary was written (front matter says "Condensed").
    condensed: bool,
    error: Option<String>,
}

#[derive(Serialize)]
struct Report {
    converted: usize,
    failed: usize,
    /// Earlier Doc Monster outputs found among the inputs and left alone.
    skipped: usize,
    results: Vec<Outcome>,
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
    let args = Args::parse();
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
    let summarizer = pick_summarizer(&args)?;
    let found = collect_files(&args.inputs);
    if found.files.is_empty() && found.missing.is_empty() {
        bail!("no documents found in the given inputs");
    }

    let monster = monster::Monster::new(!args.no_anim && !args.json && !args.quiet);
    let opts = extract::Options {
        max_rows: args.max_rows,
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
        })
        .collect();
    for p in &found.missing {
        if !args.json {
            eprintln!("spat  {}: not found", p.display());
        }
    }

    // Output names are decided up front, so files converted in parallel never race for a name.
    let plan = plan_outputs(&found.files, &args);
    let convert_one = |(path, out): (&PathBuf, &Result<PathBuf>)| -> Result<(PathBuf, bool)> {
        let out = out.as_ref().map_err(|e| anyhow!("{e:#}"))?;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process(path, out, &args, &opts, summarizer.as_deref())
        }))
        .unwrap_or_else(|_| {
            let why = LAST_CRASH.with(|m| m.borrow().clone());
            Err(anyhow!("the reader crashed on this file ({why})"))
        })
    };
    let n = found.files.len();
    let done: Vec<Result<(PathBuf, bool)>> = if monster.enabled() && n <= ANIMATE_EACH_UP_TO {
        // Keep the show to a few seconds: a short chomp per file.
        let per_file = Duration::from_millis((2500 / n.max(1) as u64).clamp(200, 900));
        found
            .files
            .iter()
            .zip(&plan)
            .map(|job| {
                let name = job
                    .0
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("document");
                monster.eat(name, per_file, || convert_one(job))
            })
            .collect()
    } else {
        // Every processor core at once; one monster eats the whole batch.
        let all = || {
            found
                .files
                .par_iter()
                .zip(plan.par_iter())
                .map(convert_one)
                .collect()
        };
        monster.eat(&format!("{n} documents"), Duration::from_millis(900), all)
    };
    for (path, result) in found.files.iter().zip(done) {
        results.push(match result {
            Ok((out, condensed)) => {
                if !args.json && !args.quiet {
                    println!("ate   {} -> {}", path.display(), out.display());
                }
                Outcome {
                    source: path.display().to_string(),
                    output: Some(out.display().to_string()),
                    ok: true,
                    condensed,
                    error: None,
                }
            }
            Err(e) => {
                if !args.json {
                    eprintln!("spat  {}: {e:#}", path.display());
                }
                Outcome {
                    source: path.display().to_string(),
                    output: None,
                    ok: false,
                    condensed: false,
                    error: Some(format!("{e:#}")),
                }
            }
        });
    }

    let converted = results.iter().filter(|r| r.ok).count();
    let failed = results.len() - converted;
    monster.burp(converted);
    if args.json {
        let report = Report {
            converted,
            failed,
            skipped: found.skipped,
            results,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        let skipped = if found.skipped > 0 {
            format!(", {} earlier outputs skipped", found.skipped)
        } else {
            String::new()
        };
        println!("done: {converted} converted, {failed} failed{skipped}");
    }
    Ok(if failed > 0 { 1 } else { 0 })
}

fn pick_summarizer(args: &Args) -> Result<Option<Box<dyn Summarizer>>> {
    if args.backend != Backend::Ollama {
        return Ok(None);
    }
    let o = Ollama {
        url: args.ollama_url.trim_end_matches('/').to_string(),
        model: args.model.clone(),
    };
    if !o.is_ready()? {
        bail!(
            "model '{}' is not pulled; run: ollama pull {}",
            o.model,
            o.model
        );
    }
    Ok(Some(Box::new(o)))
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
            if entry.file_type().is_file() {
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
    summarizer: Option<&dyn Summarizer>,
) -> Result<(PathBuf, bool)> {
    let mut doc = extract::extract_with(path, opts)?;
    // A document holding only pictures or equations still gets a file saying so.
    if doc.markdown.trim().is_empty() && doc.summary.is_none() && doc.omitted.is_empty() {
        bail!("document has no text");
    }
    let mut note = None;
    if !args.raw {
        let (cleaned, report) = clean::clean(&doc.markdown);
        doc.markdown = cleaned;
        note = report.describe();
    }
    // (summary text, most representative sentence)
    let summary: Option<(String, Option<String>)> = match (args.backend, summarizer) {
        (Backend::None, _) => None,
        // The reader already described the file (e.g. the shape of a data file).
        _ if doc.summary.is_some() => doc.summary.clone().map(|s| (s, None)),
        (Backend::Ollama, Some(s)) => {
            let text =
                summarize_document(s, &doc.markdown, args.chunk_chars).context("summarizing")?;
            let lead = text
                .lines()
                .map(|l| l.trim_start_matches(['-', '*', ' ']))
                .find(|l| !l.is_empty())
                .map(str::to_string);
            Some((text, lead))
        }
        _ => digest::digest(&doc.markdown, note.as_deref()).map(|d| (d.markdown, d.lead)),
    };
    let description = doc
        .description
        .clone()
        .or_else(|| summary.as_ref().and_then(|(_, lead)| lead.clone()))
        .or_else(|| digest::first_sentence(&doc.markdown))
        .map(|d| render::tidy_description(&d))
        .unwrap_or_default();
    let rendered = render::render(
        &doc,
        &description,
        summary.as_ref().map(|(t, _)| t.as_str()),
        !args.condensed || summary.is_none(),
    );

    std::fs::write(out, rendered).with_context(|| format!("writing {}", out.display()))?;
    Ok((out.to_path_buf(), summary.is_some()))
}
