# Installing Doc Monster

Four things, installed once, in this order: Rust (to build Doc Monster), FFmpeg (reads videos),
Tesseract (reads the text shown on screen in videos), and Doc Monster itself. All are free and run
on your machine.

## Quickest: download a ready-made program

From the [Releases page](https://github.com/DylanCodyBrown/Doc-monster/releases), download the
file for your computer (`doc-monster-windows-x86_64.zip`, `doc-monster-macos-arm64.tar.gz` or
`doc-monster-linux-x86_64.tar.gz`), unpack it, and put `doc-monster` (or `doc-monster.exe`)
somewhere on your `PATH`. You still need step 2 below (FFmpeg and Tesseract) for videos; you do
not need Rust.

macOS may refuse to open a downloaded program the first time; allow it once with
`xattr -d com.apple.quarantine ./doc-monster`.

To build it yourself instead, follow the steps below.

## 1. Install Rust (one time)

**macOS / Linux (bash or zsh)**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

**Windows (PowerShell or Command Prompt)**

```powershell
winget install --id Rustlang.Rustup -e
```

The Rust installer asks to install the Visual Studio C++ Build Tools if they are missing: say yes,
Rust needs their linker on Windows. Open a new terminal afterwards so `cargo` is found.

Check it worked, in any shell:

```
cargo --version
```

## 2. Install FFmpeg and Tesseract

**macOS (Homebrew, from [brew.sh](https://brew.sh))**

```bash
brew install ffmpeg tesseract
```

**Linux (Debian / Ubuntu)**

```bash
sudo apt install ffmpeg tesseract-ocr
```

Fedora: `sudo dnf install ffmpeg tesseract`. Arch: `sudo pacman -S ffmpeg tesseract tesseract-data-eng`.

**Windows (PowerShell or Command Prompt)**

```powershell
winget install --id Gyan.FFmpeg -e
winget install --id UB-Mannheim.TesseractOCR -e
```

The Tesseract installer does not add itself to `PATH`. Add it once, then open a new terminal:

| Shell | Command |
| --- | --- |
| PowerShell | `[Environment]::SetEnvironmentVariable("Path", $env:Path + ";C:\Program Files\Tesseract-OCR", "User")` |
| Command Prompt | `setx PATH "%PATH%;C:\Program Files\Tesseract-OCR"` |

**Other languages** for text on screen: `sudo apt install tesseract-ocr-spa` (Spanish; use the
language's three-letter code), `brew install tesseract-lang` (every language on macOS), or tick
the languages in the Windows installer. Then pass `--screen-lang spa` (or `eng+spa`).

## 3. Install Doc Monster

**Straight from GitHub (any shell)**

```
cargo install --git https://github.com/DylanCodyBrown/Doc-monster --locked
```

**Or from a local clone**

```
git clone https://github.com/DylanCodyBrown/Doc-monster
cd Doc-monster
cargo install --path . --locked
```

The first build takes about a minute. The program is copied to `~/.cargo/bin` (macOS / Linux) or
`%USERPROFILE%\.cargo\bin` (Windows), which the Rust installer already put on your `PATH`.

If the repository is private, let Cargo use your normal git login:

| Shell | Command |
| --- | --- |
| bash / zsh | `export CARGO_NET_GIT_FETCH_WITH_CLI=true` |
| PowerShell | `$env:CARGO_NET_GIT_FETCH_WITH_CLI = "true"` |
| Command Prompt | `set CARGO_NET_GIT_FETCH_WITH_CLI=true` |

## 4. Check everything

```
ffmpeg -version
tesseract --version
doc-monster --version
```

All three should print a version. If FFmpeg or Tesseract is missing, Doc Monster still converts
documents but refuses videos with a message naming what to install.

On Windows you can also pin `%USERPROFILE%\.cargo\bin\doc-monster.exe` to the desktop or
make a shortcut to it: dragging files onto it converts them.

## Using it from scripts and agents

The same command works in bash, PowerShell and the Command Prompt:

```
doc-monster gorge ./inbox --json
```

- **Output files:** each document's Markdown is written beside it (`report.docx` → `report.md`;
  `report.v2.md` if that name is taken). Add `-o folder` to collect them elsewhere, or
  `--overwrite` to replace Doc Monster's own earlier outputs instead of adding versions.
- **`--json`:** standard output is a single JSON report listing every file's source, output and
  error, and nothing else.
- **Exit codes:** `0` everything converted, `1` some files failed (the rest were still written),
  `2` bad arguments or nothing to do.
- **No animation when piped:** the monster is drawn only in an interactive terminal; `--no-anim`
  turns it off there too, and `--quiet` prints only errors and the final count.
- Paths with spaces need quotes, as usual in every shell: `doc-monster feed "My Documents/report.docx"`.
- Paths can also be piped in, one per line: `ls *.docx | doc-monster gorge` (bash) or
  `Get-ChildItem *.docx | % FullName | doc-monster gorge` (PowerShell).

## Update or remove

```
cargo install --git https://github.com/DylanCodyBrown/Doc-monster --locked --force   # update
cargo uninstall doc-monster                                                           # remove
```

## Troubleshooting

- **`doc-monster: command not found` / `is not recognized`**: open a new terminal, or add
  `~/.cargo/bin` (macOS / Linux) or `%USERPROFILE%\.cargo\bin` (Windows) to your `PATH`.
- **`linker 'link.exe' not found` on Windows**: install the Visual Studio C++ Build Tools
  (`winget install --id Microsoft.VisualStudio.2022.BuildTools -e`, then pick "Desktop development
  with C++"), then run the install again.
- **`videos need FFmpeg installed` / `videos need Tesseract installed`**: do step 2, open a new
  terminal, and check with `ffmpeg -version` and `tesseract --version`.
- **No monster on Windows**: the animation needs Windows 10 or later; on older consoles Doc Monster
  simply runs without it.
