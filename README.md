# Doc Monster

```
  Doc Monster 0.1.0                                   .-~~~-.
  drop files or folders here, then press Enter       ( o   o )
  hungry. empty line quits                           (  \_/  )
  › report.docx                                       '~^~^~'
```

**Doc Monster eats documents and leaves clean, readable Markdown beside them, with a
notebook-style summary on top.** Word files, spreadsheets, PDFs, data files, videos, ugly
Markdown: point it at a file or a folder and it gobbles everything.

- **Free and offline.** No artificial intelligence service, no account, no internet. The summary
  is computed on your machine with plain algorithms.
- **Fast.** Written in Rust and uses every processor core: about 3,300 real-world test documents
  in under 4 seconds on a 4-core machine.
- **Honest.** The summary only quotes the document, so it can never invent a fact or a number. The
  full cleaned content follows it, and anything that could not be included is listed, never
  silently dropped.

## Quick start

```bash
cargo install --git https://github.com/DylanCodyBrown/Doc-monster --locked
doc-monster feed report.docx              # writes report.md beside it
doc-monster gorge a.pdf b.xlsx ./inbox    # many files and folders at once
doc-monster feed notes.md -c 5            # compression 5 of 10: about 45% of the words
doc-monster regurgitate ./notes           # one INDEX.md for every Markdown file in the folder
doc-monster                               # drop mode: drag files onto the window, press Enter
```

| Command | What it does |
| --- | --- |
| `feed <path>` | Eat one file or folder (folders are searched recursively). |
| `gorge <paths...>` | Eat many files and folders at once (`gourge` works too). |
| `regurgitate <folders...>` | Build `INDEX.md` for a folder of Markdown, merging duplicates (`index` works too). |

`feed` and `gorge` take the same options: `-c 1-10` compression, `--index` to build the index
afterwards, `-o folder` to collect outputs elsewhere, `--overwrite`, `--no-summary`, `--raw`,
`--max-rows`, `--json`, `--quiet`, `--no-anim`. `doc-monster <command> --help` lists them all.

### Drag and drop

Run `doc-monster` (or `feed` / `gorge` without a path) and the monster, a small slime pet, waits
at the right of the prompt: drag files or folders onto the terminal window and press Enter. Its
face shows how the last meal went. It understands the way every terminal writes a dropped file: backslash-escaped spaces (macOS Terminal, iTerm), single quotes
(Linux terminals), double quotes (Windows Terminal, PowerShell, Command Prompt) and `file://`
addresses. Drop several at once if you like; an empty line quits.

On Windows you can also drag files straight onto `doc-monster.exe` (or a shortcut to it) in
File Explorer; the window stays open until you press Enter so you can read the results.

New to Rust? [INSTALL.md](INSTALL.md) has copy-paste commands for macOS, Linux and Windows
(bash, PowerShell and Command Prompt).

## What you get (shortened)

```markdown
---
# Condensed with Doc Monster
title: "Aurora Operations Guide"
description: "Player data is saved to a DataStore every minute and again when the player leaves."
---

> **Not included:** 3 images.

## Summary

### At a glance
3,149 words (about 15 min read), 29 sections, 5 tables · Aurora Operations Guide

### Overview
- Session locking prevents two servers from writing the same profile and corrupting inventory.
- Game passes unlock permanent perks while developer products sell repeatable boosts.
...

### Key terms
`session locking` · `game passes` · `remote calls` · `datastore` · ...

### Outline
- **1 DataStore saving** · _datastore, pcall, writes_ · 403 words
  - Always wrap DataStore calls in pcall because the service throttles at 10 requests per minute.
  - **1.1 Checklist** · ...

---

## Full content
(the whole document, cleaned)
```

## What it eats

| Kind | Formats |
| --- | --- |
| Word | `.docx` `.docm` `.dotx` `.doc` (Word 6 to 2003, read natively) `.rtf` `.odt` `.odp` `.fodt` |
| Spreadsheets | `.xlsx` `.xlsm` `.xls` `.xlsb` `.ods` `.csv` `.tsv` (first 5,000 rows per sheet; `--max-rows` to change) |
| Data | JSON (comments allowed), JSON Lines, YAML, TOML, XML, INI and `.env` |
| Text | Markdown, plain text in any encoding, HTML, reStructuredText, AsciiDoc, Org, LaTeX, logs and code |
| Video | `.mp4` `.mov` `.mkv` `.webm` `.avi` and more: on-screen text, subtitles, chapters, tags and tracks |
| Other | PDF, `.srt` and `.vtt` subtitles |

Footnotes, endnotes, comments, and page headers and footers are kept. Spreadsheet dates stay dates,
hidden sheets are labelled, and merged table cells keep columns aligned.

## Videos: what was said and what was on screen

A video is handled like a piece of music. Each **instrument** is its own track of timed events,
and all of them play at once on separate threads:

- **On screen**: the text shown on screen (slides, code, title cards, credits, interface labels).
  FFmpeg takes a frame each time the picture changes, and
  [Tesseract](https://github.com/tesseract-ocr/tesseract) reads each frame as soon as it is
  written, while FFmpeg keeps decoding. Misread bullet symbols are repaired, the same screen shown
  twice is listed once, and code on screen becomes a code block.
- **Spoken**: the subtitles, from a `.srt` / `.vtt` file beside the video or a subtitle track
  inside it.
- **Chapters**: the video's chapter markers.

The **bars** are two-minute sections and the **timestamps** are the beat: the tracks are then
assembled into one timeline, in time order.

```markdown
**On screen at 0:04**

> DataStore saving
> - Autosave every 60 seconds
> - Wrap every call in pcall

**Spoken**

First, how we save player data safely.
```

Every frame is decoded (encoded video stores changes between frames, so there is no way to
know what is on screen without decoding up to it), but text is only read where the picture
changed, and a **hash table** of every screen already read means a screen shown again later (a
slide revisited, a repeated title card) reuses its text instead of being read again. Each video's
details say how many changes there were and how many were actually read.

Speed: a 10-minute 720p film takes about 30 seconds; slides and screen recordings, which change
less often, are much faster. `--screen-lang spa` (or `eng+spa`) reads other languages.
`--max-frames N` reads at most N frames; anything skipped that way is listed under Not included. FFmpeg and Tesseract are required for videos (see [INSTALL.md](INSTALL.md)), except with the
[Linux binary](<Linux Binary/README.md>), which has them built in.
Speech itself is not transcribed; without subtitles the output says so.

## How the summary works

1. **Clean** the text: broken headings, hard-wrapped lines, page numbers, repeated headers and
   footers.
2. **Build a tree** from the headings. Files without headings are split where the vocabulary
   shifts, and long lists of sections are grouped into chapters.
3. **Score every sentence** by how distinctive its words are to its section, favouring topic
   sentences and skipping filler and near-duplicates.
4. **Pick the best sentences at each level**: an overview, key terms and a nested outline.
   Sentences that start with "This" or "However" bring the sentence they refer to.

Data files get a plain-English description of their shape instead ("`players` is a list of 500
items; `coins` ranges from 42 to 9,998").

Spreadsheets and CSV files are summarised by their **structure only**, never their values: each
sheet's size and its column names ("Spreadsheet with 2 sheets: "Revenue" (3 rows × 4 columns;
columns Name, Region, Revenue, Q1)"). Title rows above the header are found and kept as text, and
a sheet whose first row is already data is reported as having no header row rather than having its
values called column names. The tables themselves follow in full.

## How compressed: `--compress 1` to `10`

| Level | You get |
| --- | --- |
| **1** (default) | The summary, then the whole cleaned document. Nothing is removed. |
| **2 to 9** | The summary, then the document itself, shortened: its most informative sentences from every section, in their original order, with headings and formatting kept. |
| **10** | The summary only. |

Approximate share of the words kept, measured on the 45,000-word Pandoc manual:

| Level | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Kept | 84% | 72% | 56% | 46% | 37% | 28% | 22% | 18% |

Every section keeps at least one sentence, so documents with many short sections stay a little
longer at the high levels. Repeats go first, more strictly as the level rises: from level 2,
sentences repeated word for word; from 5, the same sentence with different numbers; from 7,
near-duplicates. Tables are kept up to level 3, code up to level 4. The front matter comment says
the level (`# Condensed with Doc Monster, compression 5 of 10`) and the "Not included" line says
how much was cut.

## One index for a whole folder

```bash
doc-monster regurgitate ./notes     # just build the index
doc-monster gorge ./inbox --index   # convert everything, then build the index
```

`INDEX.md` is written once, at the top of the folder (the one file that is not placed beside a
source). It lists every Markdown file inside, grouped by subfolder, with its title, description,
size and keywords, and **dedups everything it can**:

- **Duplicate files** become one entry, even when one copy was cleaned or rewrapped
  (`notes.md` and its cleaned `notes.v2.md` match).
- **Rerun copies** that Doc Monster made of the same source (`notes.v3.md`, `notes.v4.md`) are
  deleted. Your own files are never deleted; `--keep-duplicates` keeps the copies too.
- **Repeated passages** that appear in several documents are listed once, with where they appear.
- **Key terms** from all documents are merged into one list, counted by how many documents use
  them.

Running it again updates the same `INDEX.md`. If you already have your own `INDEX.md`, Doc Monster
writes `INDEX.v2.md` instead and leaves yours alone.

## Good to know

- Output goes **beside each source**; if the name is taken it writes `name.v2.md`, `name.v3.md`.
  `--overwrite` replaces Doc Monster's own earlier output, and files it did not write are never
  touched. Its own outputs (and its indexes) are skipped when you run it on the same folder again.
- `--no-summary` converts and cleans without a summary.
- **Scripts and agents:** `--json` prints a machine-readable report, exit codes are `0` / `1` / `2`,
  and the pet, colours and progress line only appear in an interactive terminal (`NO_COLOR` turns
  colours off).
- **Not yet:** scanned pages and images (needs text recognition), PDF headings and tables, slide
  decks (`.pptx`), e-books, speech in videos without subtitles, merged cells in legacy `.doc`
  tables.

## Development

```bash
cargo test                                          # unit and end-to-end tests
cargo run --release --example make_corpus -- out    # generate big, ugly test documents
```

`tests/fixtures/sample.doc` is a real legacy Word file made by LibreOffice. The readers have also
been run against the Pandoc and Apache POI test suites (about 3,300 real documents from many
different programs).

## License

Doc Monster is free to use, change and share, under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in
this project, as defined in the Apache 2.0 license, is dual licensed as above, without any
additional terms or conditions.

FFmpeg and Tesseract are separate programs with their own licenses; Doc Monster runs them. The
[Linux binary](<Linux Binary/README.md>) carries copies of them, and their licenses are in
[`Linux Binary/licenses`](<Linux Binary/licenses>).
