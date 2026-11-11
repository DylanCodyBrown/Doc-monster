# Doc Monster for Linux

`doc-monster` in this folder is a ready-to-run Linux executable with **FFmpeg and Tesseract
built in**. Copy this one file anywhere, make it executable, and run it. Nothing else needs to be
installed, not even for videos.

```bash
chmod +x doc-monster
./doc-monster --version
./doc-monster feed report.docx
./doc-monster feed talk.mp4          # on-screen text and subtitles, no FFmpeg install needed
./doc-monster feed sign.png          # the text in a picture
```

## What it targets

| | |
| --- | --- |
| Operating system | Linux, any distribution |
| Processor | 64-bit Intel or AMD (`x86_64`) |
| Rust target | `x86_64-unknown-linux-musl` |
| Linking | Fully static: no system libraries at all, so it runs on old and new distributions alike, minimal containers, and Azure Function Apps |
| Not for | Windows, macOS, or ARM processors (Raspberry Pi, Graviton, Apple chips) |

## This build

| | |
| --- | --- |
| Doc Monster | `doc-monster 0.2.0`, built from this repository with `--features bundled-tools` |
| Compiler | `rustc 1.99.1 (58cf8f9c3 2026-10-13)` |
| Size | 18,044,224 bytes |
| SHA-256 | `40d1797f8a94c94fcdb049a1b36b327eb8f1ff54472dae4d8a783f10153c0f34` |

Built in:

| Tool | Version | Source revision | License |
| --- | --- | --- | --- |
| FFmpeg and ffprobe | 7.1.1 | `db69d06eeeab4f46da15030a80d539efb4503ca8` | GPL 2.0 or later |
| Tesseract | 5.5.1 | `3b7c70e34dea179549ed3e995872e2e019eb8477` | Apache 2.0 |
| Leptonica (Tesseract's image library) | 1.85.0 | `63aef18d98432b8582a1565e241f7bd2ee9cc8d9` | BSD 2-clause |
| zlib (lets FFmpeg read PNG pictures) | 1.3.1 | tag `v1.3.1` | zlib license |
| English text model (`tessdata_fast`) | 4.1.0 | SHA-256 `7d4322bd…556427170b2` | Apache 2.0 |

Check your copy matches:

```bash
sha256sum doc-monster
```

### Checks it passed

```text
$ file doc-monster
ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, stripped

$ ldd doc-monster
statically linked

$ doc-monster --version
doc-monster 0.2.0
```

- Converting a small HTML page with `-c 6` wrote the expected Markdown with front matter.
- `regurgitate --keep-duplicates` on two Markdown files wrote an `INDEX.md` listing both.
- The test suite passed on this target with the tools built in: 98 tests, 0 failures.
- With the `PATH` emptied, so no system FFmpeg or Tesseract could be found, the test slides video
  converted to the same Markdown as a build using the system's tools.
- A 10-minute 720p video: 598 screen changes found, 467 read, in 20.5 seconds (21.8 seconds with
  the system's tools).

## How the built-in tools work

The three programs and the English text model are stored compressed inside `doc-monster`. The
first time a video is converted they are unpacked once, which takes a few seconds, into the first
of these folders that can be written to and can run programs:

1. `$DOC_MONSTER_CACHE`, if you set it
2. `$XDG_CACHE_HOME/doc-monster`
3. `~/.cache/doc-monster`
4. the system's temporary folder (`/tmp/doc-monster`)

Later runs reuse them. The folder name includes a fingerprint of the tools, so a newer
`doc-monster` never uses an older one's copies. Documents that are not videos never unpack
anything.

**Other languages:** only English is built in. For another language, download its
`.traineddata` file (for example `spa.traineddata` from
[tessdata_fast](https://github.com/tesseract-ocr/tessdata_fast)) into a folder, together with
`eng.traineddata`, then run with that folder:

```bash
TESSDATA_PREFIX=/path/to/tessdata ./doc-monster feed video.mp4 --screen-lang spa
```

**Videos it reads:** H.264, H.265, MPEG-4, MPEG-1 and 2, VP8, VP9, Theora, Windows Media, Flash,
ProRes, Motion JPEG and DV, in `.mp4` `.mov` `.mkv` `.webm` `.avi` `.flv` `.mpg` `.wmv`
`.ogv`, plus subtitle tracks and `.srt` / `.vtt` / `.ass` files. AV1 video is not included.

## How to build it

[`build.sh`](build.sh) does everything: fetches the exact source revisions above, builds FFmpeg,
Leptonica and Tesseract as static programs, checks the English model's fingerprint, and builds
Doc Monster with them packed in.

```bash
sudo apt-get install -y build-essential cmake nasm musl-tools python3-pip
bash "Linux Binary/build.sh"      # about 10 minutes on 4 cores; writes Linux Binary/doc-monster
```

What it does, step by step:

1. **Compilers.** FFmpeg is built with `musl-gcc`. Tesseract is C++, so it is built with Zig
   (installed with `pip install ziglang`), which can compile C++ into a fully static musl program.
2. **FFmpeg** is configured with everything off, then only what Doc Monster uses switched on:
   the video and subtitle readers listed above, the `fps`, `mpdecimate`, `showinfo`, `split`,
   `scale` and `format` filters, and PGM and raw output. That keeps it small (about 7.7 MB).
3. **Leptonica** is built without any image format libraries. Frames are written as PGM, a plain
   grayscale format it reads by itself, and each frame is deleted as soon as it is read.
4. **Tesseract** is built without network downloads, training tools, or threads (Doc Monster runs
   many Tesseracts in parallel instead).
5. **Doc Monster** is built with `DOC_MONSTER_TOOLS` pointing at the folder of tools and
   `--features bundled-tools`. `build.rs` compresses each tool into the program.

Without the feature (a normal `cargo build`), Doc Monster uses FFmpeg and Tesseract from beside
the executable or from the `PATH`, exactly as before.

## Licenses

Doc Monster itself is MIT or Apache 2.0. The built-in tools keep their own licenses, which are in
[`licenses/`](licenses). They are separate programs that Doc Monster runs, not part of its code.

FFmpeg is built with `--enable-gpl` because the `mpdecimate` filter, which finds the moments the
screen changes, is GPL. Its complete source is the FFmpeg revision listed above, and `build.sh`
is the exact configuration used. Tesseract and Leptonica are statically linked with the musl C
library (MIT) and the LLVM C++ library (Apache 2.0 with LLVM exception).
