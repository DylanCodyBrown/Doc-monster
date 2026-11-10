/// Split Markdown into chunks of at most `max_chars`, breaking only on
/// paragraph boundaries (blank lines) and never inside a fenced code block.
/// A single paragraph longer than the limit is kept whole rather than cut.
pub fn chunk_markdown(text: &str, max_chars: usize) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_fence = false;

    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        if line.trim().is_empty() && !in_fence {
            if !current.trim().is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.trim().is_empty() {
        blocks.push(current);
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut acc = String::new();
    for block in blocks {
        if !acc.is_empty() && acc.len() + block.len() + 1 > max_chars {
            chunks.push(std::mem::take(&mut acc));
        }
        if !acc.is_empty() {
            acc.push('\n');
        }
        acc.push_str(&block);
    }
    if !acc.is_empty() {
        chunks.push(acc);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_paragraphs_under_limit() {
        let text = "aaaa\n\nbbbb\n\ncccc\n";
        let chunks = chunk_markdown(text, 12);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("aaaa") && chunks[0].contains("bbbb"));
        assert!(chunks[1].contains("cccc"));
    }

    #[test]
    fn never_splits_code_fence() {
        let text = "```\nline one\n\nline two\n```\n\nafter\n";
        let chunks = chunk_markdown(text, 10);
        assert!(chunks[0].contains("line one") && chunks[0].contains("line two"));
    }

    #[test]
    fn keeps_every_character_of_content() {
        let text = "one\n\ntwo\n\nthree\n\nfour\n";
        let joined = chunk_markdown(text, 8).join("\n");
        for word in ["one", "two", "three", "four"] {
            assert!(joined.contains(word));
        }
    }
}
