# Doc Monster

A Rust command line tool that eats documents and produces clean, readable Markdown with a
Notebook-style hierarchical summary. **No artificial intelligence service, no cost, fully offline.**

```
doc-monster ./inbox report.docx recording.mp4 -o ./markdown            # summary + full cleaned content
doc-monster huge-ugly-notes.md --condensed                              # summary only
```

## What it eats

| Format | How |
| --- | --- |
| Markdown, plain text | cleaned (see below) |
| HTML | converted with structure kept (headings, lists, links, tables) |
| Word (`.docx`) | headings, lists, tables, special characters |
| Portable Document Format | text extracted, wrapped lines re-joined into paragraphs |
| Excel / OpenDocument spreadsheets (`.xlsx`, `.xls`, `.ods`) | every sheet as a table, plus a profile of each column (range, average, most common values) |
| CSV / TSV | Markdown table |
| JSON | pretty printed code block |
| `.mp4` video | tracks, codecs, resolution, duration, and a timed transcript if a `.srt` / `.vtt` file with the same name sits beside it |
| `.srt` / `.vtt` subtitles | timed transcript in two-minute sections |
| Any other text file | fenced code block |

Folders are searched recursively. Speech inside videos is **not** transcribed (that needs a speech
model); supply a subtitle file and it is included and summarized.

## Cleanup pass (on by default, `--raw` to skip)

Fixes line endings and invisible characters, strips hidden HTML comments and `<br>` litter,
repairs `##Heading` and underlined headings, repairs skipped heading levels, unifies bullet styles,
joins hard-wrapped paragraphs, repairs ragged tables, collapses runs of blank lines, and removes page
numbers and headers/footers repeated on every page. Fenced code is never touched. The summary
reports what was removed.

## The hierarchical digest

Computed locally with plain algorithms, in well under a second for hundreds of thousands of words:

1. **Overview**: the most representative sentence from each top-level part.
2. **Key terms**: frequent phrases and distinctive words.
3. **Outline**: nested sections, each with keywords, word count and its best sentences. Long lists of
   sibling sections are grouped into chapters at the points where the vocabulary shifts. Text with no
   headings is cut into topical segments the same way. Depth adapts so the outline stays about one
   screen per hundred sections.

Sentences are scored by how distinctive their words are to their section, near duplicates are skipped
across the whole digest, and generic filler is filtered out. Small sections are shown in full.

Quality rule: the digest is **added** to the output; unless you pass `--condensed`, the complete
cleaned content follows it, so nothing is lost.

## Optional: local model

`--backend ollama --model qwen2.5:1.5b` uses a model running on your own machine through
[Ollama](https://ollama.com). Free, but heavier and slower than the default digest.
`--backend none` converts and cleans without summarizing.

## Test corpus

`python3 scripts/make_corpus.py OUT_DIR [--scale N]` generates large, deliberately ugly test files
(markdown, text, Word, Excel) about Roblox game development. Needs `python-docx` and `openpyxl`.

## Roadmap

- Scanned documents and images (optical character recognition)
- Slide decks (`.pptx`), e-books (`.epub`)
- Optional offline speech-to-text for video, using a small local model
- Parallel processing, caching by content hash
