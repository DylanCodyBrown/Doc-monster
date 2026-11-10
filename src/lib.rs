//! Doc Monster: eats documents, produces Markdown.
//!
//! Pipeline: detect format -> extract (structure preserving) -> clean ->
//! digest (hierarchical, computed locally, no model) -> render Markdown.
//! Separately, `index` builds one INDEX.md for a folder of Markdown files.
//!
//! Quality rule: the summary is *added* to the output, never substituted for
//! the extracted content. Nothing from the source is thrown away.

pub mod clean;
pub mod digest;
pub mod extract;
pub mod index;
pub mod profile;
pub mod render;
pub mod tools;

use std::path::PathBuf;

/// A document after extraction, before summarization.
#[derive(Debug, Clone)]
pub struct Document {
    pub title: String,
    /// From the file's own metadata, when it has any.
    pub description: Option<String>,
    /// A summary made by the reader itself (e.g. the shape of a data file).
    pub summary: Option<String>,
    /// Things in the source that are not in the output, reported so nothing is lost silently.
    pub omitted: Vec<String>,
    pub source: PathBuf,
    pub format: extract::Format,
    /// Full-fidelity Markdown conversion of the source.
    pub markdown: String,
}
