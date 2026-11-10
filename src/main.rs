use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use doc_monster::{
    extract, render,
    summarize::{Extractive, Ollama, Summarizer, summarize_document},
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Backend {
    /// Use Ollama if it is running with the model, otherwise fall back offline.
    Auto,
    Ollama,
    /// Offline sentence ranking, no model needed.
    Extractive,
    /// Convert only, no summary.
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
    #[arg(long, value_enum, default_value = "auto")]
    backend: Backend,
    /// Local model name as known to Ollama.
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
    let ollama = || Ollama {
        url: args.ollama_url.trim_end_matches('/').to_string(),
        model: args.model.clone(),
    };
    Ok(match args.backend {
        Backend::None => None,
        Backend::Extractive => Some(Box::new(Extractive { sentences: 5 })),
        Backend::Ollama => {
            let o = ollama();
            if !o.is_ready()? {
                bail!("model '{}' is not pulled; run: ollama pull {}", o.model, o.model);
            }
            Some(Box::new(o))
        }
        Backend::Auto => {
            let o = ollama();
            match o.is_ready() {
                Ok(true) => Some(Box::new(o)),
                other => {
                    let why = match other {
                        Ok(_) => format!("model '{}' not pulled", args.model),
                        Err(e) => format!("{e:#}"),
                    };
                    eprintln!("note: using offline extractive summaries ({why})");
                    Some(Box::new(Extractive { sentences: 5 }))
                }
            }
        }
    })
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
    files
}

fn process(
    path: &Path,
    args: &Args,
    summarizer: Option<&dyn Summarizer>,
    used: &mut HashSet<String>,
) -> Result<PathBuf> {
    let doc = extract::extract(path)?;
    if doc.markdown.trim().is_empty() {
        bail!("document is empty");
    }
    let summary = match summarizer {
        Some(s) => {
            let text = summarize_document(s, &doc.markdown, args.chunk_chars)
                .context("summarizing")?;
            Some((s.name(), text))
        }
        None => None,
    };
    let rendered = render::render(
        &doc,
        summary.as_ref().map(|(m, t)| (m.as_str(), t.as_str())),
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
