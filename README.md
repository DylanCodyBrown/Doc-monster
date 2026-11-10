# Doc Monster

A Rust command line tool that eats documents and produces clean, readable Markdown with a
Notebook-style hierarchical summary. **No artificial intelligence service, no cost, fully offline.**

```
doc-monster ./inbox report.doc settings.yaml recording.mkv -o ./markdown   # summary + full cleaned content
doc-monster huge-ugly-notes.md --condensed                                  # summary only
```

## Output

Every file gets the same minimal front matter, then a summary, then the full cleaned content:

```markdown
---
# Condensed with Doc Monster
title: "Aurora Operations Guide"
description: "One line, at most 200 characters."
---

## Summary
...
---

## Full content
...
```

The title and description come from the file's own metadata when it has any (Word and OpenDocument
properties, Markdown front matter, video tags, a `description` key in data files); otherwise from the
first heading and the document's most representative sentence. Original Markdown front matter keys
other than title and description are kept at the end of the content so nothing is lost.
The comment says "Converted" instead of "Condensed" when no summary was made.

## What it eats

| Family | Formats | Notes |
| --- | --- | --- |
| **Word** | `.docx` `.docm` `.dotx` `.dotm`, legacy `.doc` `.dot`, `.rtf`, OpenDocument `.odt` `.ott` `.odp` `.fodt` | Headings, lists, tables, special characters. The legacy binary `.doc` is read natively (headings, lists and tables included) with no external program. |
| **Data structures** | JSON (comments and trailing commas allowed), JSON Lines, YAML (multi-document), TOML, XML (also SVG, RSS, plist, `.rbxlx`), INI / `.env` / properties | A plain-English description of the shape: keys, list lengths, value ranges, most common values. The original source follows, unchanged. |
| **Text documents** | `.md` `.mdx`, `.txt`, reStructuredText, AsciiDoc, Org, LaTeX, HTML, and any text encoding (UTF-8, UTF-16, Windows-1252) | Headings, lists, code, links and emphasis are converted. Logs and source code are fenced and never reflowed. |
| **Tables** | `.xlsx` `.xls` `.ods`, `.csv` `.tsv` | Every sheet as a table plus a profile of each column. |
| **Video** | `.mp4` `.mov` `.mkv` `.webm` `.avi` `.wmv` `.flv` `.mpg` and more | Tracks, codecs, resolution, duration, tags, chapters, and a timed transcript from a subtitle file beside the video or a subtitle track inside it. Uses `ffprobe` / `ffmpeg` when installed; a built-in reader handles mp4 without them. |
| **Other** | Portable Document Format, `.srt` / `.vtt` subtitles | |

Folders are searched recursively. Speech inside videos is **not** transcribed (that needs a speech
model); supply a subtitle file and it is included and summarized.

## Cleanup pass (on by default, `--raw` to skip)

Fixes line endings and invisible characters, strips hidden HTML comments and `<br>` litter,
repairs `##Heading` and underlined headings, repairs skipped heading levels, unifies bullet styles,
joins hard-wrapped paragraphs, repairs ragged tables, collapses runs of blank lines, and removes page
numbers and headers/footers repeated on every page. Repeated lines that read as sentences are kept,
since they are content. Fenced code is never touched. The summary reports what was removed.

## The hierarchical digest

Computed locally with plain algorithms, in about a second for hundreds of thousands of words:

1. **Overview**: the most representative sentence from each top-level part.
2. **Key terms**: frequent phrases and distinctive words.
3. **Outline**: nested sections, each with keywords, word count and its best sentences. Long lists of
   sibling sections are grouped into chapters at the points where the vocabulary shifts. Text with no
   headings is cut into topical segments the same way. Depth adapts to document size.

Sentences are scored by how distinctive their words are to their section, near duplicates are skipped
across the whole digest, and generic filler is filtered out. Small sections are shown in full.
Data files skip the digest: their shape description is the summary.

## Optional: local model

`--backend ollama --model qwen2.5:1.5b` uses a model running on your own machine through
[Ollama](https://ollama.com). Free, but heavier and slower than the default digest.
`--backend none` converts and cleans without summarizing.

## Known limits

- Legacy `.doc`: headings and lists are recovered from Word's built-in styles; custom styles that
  are not based on Heading 1-9 are treated as body text. Images, text boxes, headers and footers,
  and footnotes are not extracted.
- RTF, LaTeX, Org, AsciiDoc and reStructuredText converters keep structure, not layout; exotic
  macros and directives are dropped.
- Scanned pages and images need text recognition, which is not included yet.

## Test corpus

`python3 scripts/make_corpus.py OUT_DIR [--scale N]` generates large, deliberately ugly test files
(markdown, text, Word, Excel) about Roblox game development. Needs `python-docx` and `openpyxl`.
`tests/fixtures/sample.doc` is a real legacy Word file made by LibreOffice.

## Roadmap

- Scanned documents and images (optical character recognition)
- Slide decks (`.pptx`), e-books (`.epub`)
- Optional offline speech-to-text for video, using a small local model
- Parallel processing, caching by content hash
