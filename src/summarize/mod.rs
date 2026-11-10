mod ollama;

pub use ollama::Ollama;

use crate::chunk::chunk_markdown;
use anyhow::Result;

pub trait Summarizer {
    /// Short name recorded in the output so readers know how the summary was made.
    fn name(&self) -> String;
    fn summarize(&self, text: &str) -> Result<String>;
}

/// Map-reduce summarization so documents of any length fit a small model's
/// context window: summarize each chunk, then summarize the summaries until
/// the result fits in one pass.
pub fn summarize_document(
    s: &dyn Summarizer,
    markdown: &str,
    chunk_chars: usize,
) -> Result<String> {
    let mut chunks = chunk_markdown(markdown, chunk_chars);
    if chunks.is_empty() {
        return Ok(String::new());
    }
    // Each round must shrink the chunk count or we stop to avoid looping forever.
    while chunks.len() > 1 {
        let before = chunks.len();
        let partials = chunks
            .iter()
            .map(|c| s.summarize(c))
            .collect::<Result<Vec<_>>>()?;
        chunks = chunk_markdown(&partials.join("\n\n"), chunk_chars);
        if chunks.len() >= before {
            chunks = vec![partials.join("\n\n")];
        }
    }
    s.summarize(&chunks[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Shout;
    impl Summarizer for Shout {
        fn name(&self) -> String {
            "test".into()
        }
        fn summarize(&self, text: &str) -> Result<String> {
            Ok(text.chars().take(5).collect())
        }
    }

    #[test]
    fn reduces_long_documents_to_one_summary() {
        let doc = (0..50).map(|i| format!("paragraph number {i}")).collect::<Vec<_>>().join("\n\n");
        let out = summarize_document(&Shout, &doc, 60).unwrap();
        assert!(!out.is_empty() && out.len() <= 5);
    }
}
