# Doc Monster

A Rust command line tool that eats documents and produces Markdown.

```
doc-monster ./inbox report.docx paper.pdf -o ./markdown
```

## What it eats

| Format | How |
| --- | --- |
| Markdown, plain text | passed through unchanged |
| HTML | converted with structure kept (headings, lists, links, tables) |
| Word (`.docx`) | headings, lists, tables, special characters |
| Portable Document Format | text extracted, wrapped lines re-joined into paragraphs |
| CSV / TSV | Markdown table |
| JSON | pretty printed code block |
| Any other text file (source code, logs) | fenced code block |

Folders are searched recursively.

## Quality rule

The summary is **added**, never substituted. Every output file has front matter,
a `## Summary` section, then `## Full content` holding the complete conversion.

## Summaries

- `--backend auto` (default): uses a local [Ollama](https://ollama.com) model if one is running
  (`ollama pull qwen2.5:1.5b`), otherwise falls back to offline sentence ranking.
- `--backend ollama` / `extractive` / `none` force a choice.
- Long documents are summarized map-reduce style: chunks first, then summaries of summaries,
  so small models with small context windows still handle big files.

## Roadmap

- Scanned documents (optical character recognition), images
- Slide decks (`.pptx`), spreadsheets (`.xlsx`), e-books (`.epub`)
- Embedded in-process model (no Ollama server) via `llama.cpp` bindings
- Parallel processing, caching by content hash
