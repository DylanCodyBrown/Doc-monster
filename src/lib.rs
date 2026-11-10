//! Doc Monster: eats documents, produces Markdown.
//!
//! Pipeline: detect format -> extract (structure preserving) -> chunk ->
//! summarize (local model, or offline fallback) -> render Markdown.
//!
//! Quality rule: the summary is *added* to the output, never substituted for
//! the extracted content. Nothing from the source is thrown away.

pub mod chunk;
pub mod extract;
pub mod render;
pub mod summarize;

use std::path::PathBuf;

/// A document after extraction, before summarization.
#[derive(Debug, Clone)]
pub struct Document {
    pub title: String,
    pub source: PathBuf,
    pub format: extract::Format,
    /// Full-fidelity Markdown conversion of the source.
    pub markdown: String,
}
