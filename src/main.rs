use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use doc_monster::{
    clean, digest, extract, render,
    summarize::{Ollama, Summarizer, summarize_document},
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
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

/// Doc Monster: eats documents, produces Markdown.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Files or folders to consume (folders are searched recursively).
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Folder for the Markdown output.
    #[arg(short, long, default_value = "monster-out")]
    out: PathBuf,
    #[arg(long, value_enum, default_value = "digest")]
    backend: Backend,
    /// Write only the summary, leaving out the full converted content.
    #[arg(long)]
    condensed: bool,
    /// Skip the Markdown cleanup pass (keep the converted text exactly as extracted).
    #[arg(long)]
    raw: bool,
    /// Local model name as known to Ollama (only with --backend ollama).
    #[arg(long, default_value = "qwen2.5:1.5b")]
    model: String,
    #[arg(long, default_value = "http://localhost:11434")]
    ollama_url: String,
    /// Maximum characters sent to the model per request.
    #[arg(long, default_value_t = 6000)]
    chunk_chars: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let summarizer = pick_summarizer(&args)?;
    std::fs::create_dir_all(&args.out)?;

    let files = collect_files(&args.inputs);
    if files.is_empty() {
        bail!("no files found in the given inputs");
    }

    let mut used: HashSet<String> = HashSet::new();
    let (mut ok, mut failed) = (0, 0);
    for path in files {
        match process(&path, &args, summarizer.as_deref(), &mut used) {
            Ok(out) => {
                ok += 1;
                println!("ate   {} -> {}", path.display(), out.display());
            }
            Err(e) => {
                failed += 1;
                eprintln!("spat  {}: {e:#}", path.display());
            }
        }
    }
    println!("done: {ok} converted, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
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
        bail!("model '{}' is not pulled; run: ollama pull {}", o.model, o.model);
    }
    Ok(Some(Box::new(o)))
}

fn collect_files(inputs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for input in inputs {
        for entry in WalkDir::new(input).sort_by_file_name().into_iter().flatten() {
            if entry.file_type().is_file() {
                files.push(entry.into_path());
            }
        }
    }
    // A subtitle file beside a video of the same name is that video's transcript:
    // it is folded into the video's output instead of converted on its own.
    let videos: std::collections::HashSet<PathBuf> = files
        .iter()
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("mp4" | "m4v" | "mov")))
        .map(|p| p.with_extension(""))
        .collect();
    files.retain(|p| {
        let is_sub = matches!(p.extension().and_then(|e| e.to_str()), Some("srt" | "vtt"));
        !(is_sub && videos.contains(&p.with_extension("")))
    });
    files
}

fn process(
    path: &Path,
    args: &Args,
    summarizer: Option<&dyn Summarizer>,
    used: &mut HashSet<String>,
) -> Result<PathBuf> {
    let mut doc = extract::extract(path)?;
    if doc.markdown.trim().is_empty() {
        bail!("document is empty");
    }
    let mut note = None;
    if !args.raw {
        let (cleaned, report) = clean::clean(&doc.markdown);
        doc.markdown = cleaned;
        note = report.describe();
    }
    let summary: Option<(String, String)> = match (args.backend, summarizer) {
        (Backend::None, _) => None,
        (Backend::Ollama, Some(s)) => {
            let text = summarize_document(s, &doc.markdown, args.chunk_chars).context("summarizing")?;
            Some((s.name(), text))
        }
        _ => digest::digest(&doc.markdown, note.as_deref())
            .map(|d| ("hierarchical digest (no model)".to_string(), d.markdown)),
    };
    let rendered = render::render(
        &doc,
        summary.as_ref().map(|(m, t)| (m.as_str(), t.as_str())),
        !args.condensed || summary.is_none(),
    );

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("document");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let mut name = format!("{stem}.md");
    if !used.insert(name.clone()) {
        name = format!("{stem}-{ext}.md");
        used.insert(name.clone());
    }
    let out = args.out.join(name);
    std::fs::write(&out, rendered)?;
    Ok(out)
}
