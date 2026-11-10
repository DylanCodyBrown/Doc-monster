# Doc Monster for Linux

`doc-monster` in this folder is a ready-to-run Linux executable. Copy it anywhere, make it
executable, and run it. Nothing else is needed for documents.

```bash
chmod +x doc-monster
./doc-monster --version
./doc-monster feed report.docx
```

## What it targets

| | |
| --- | --- |
| Operating system | Linux, any distribution |
| Processor | 64-bit Intel or AMD (`x86_64`) |
| Rust target | `x86_64-unknown-linux-musl` |
| Linking | Fully static: it uses no system libraries, so it runs on old and new distributions alike, including minimal containers and Azure Function Apps |
| Not for | Windows, macOS, or ARM processors (Raspberry Pi, Graviton, Apple chips) |

## This build

| | |
| --- | --- |
| Source revision | `92b74d8e88cb0e4ad3e15b11b121a939aa8d259f` |
| Version | `doc-monster 0.1.0` |
| Compiler | `rustc 1.99.1 (58cf8f9c3 2026-10-13)` |
| Size | 7,636,256 bytes |
| SHA-256 | `ad3b16fd03100943123939d76db242986fac1c1ac2293006f5f0c3a22bde1aa0` |

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
doc-monster 0.1.0
```

- Converting a small HTML page with `-c 6` wrote the expected Markdown with front matter.
- `regurgitate --keep-duplicates` on two Markdown files wrote an `INDEX.md` listing both.
- The full test suite passed on this target: 88 tests, 0 failures.

## Videos

Videos need FFmpeg and Tesseract. This build calls them from the system, so install them first
if you want video support (see [INSTALL.md](../INSTALL.md)). Every other format works without
them.

## How it was built

```bash
rustup target add x86_64-unknown-linux-musl
sudo apt-get install -y musl-tools
git clone https://github.com/DylanCodyBrown/Doc-monster
cd Doc-monster
git checkout 92b74d8e88cb0e4ad3e15b11b121a939aa8d259f
cargo build --release --locked --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/doc-monster "Linux Binary/"
```

`--locked` builds with the exact library versions in `Cargo.lock`, so the same revision always
gives the same program.
