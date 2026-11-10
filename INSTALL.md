# Installing Doc Monster

Doc Monster is a single program with no runtime dependencies. You build it once with Rust's
package manager, Cargo, and it lands on your `PATH`.

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

## 2. Install Doc Monster

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

## 3. Check it

```
doc-monster --version
doc-monster --help
```

On Windows you can also pin `%USERPROFILE%\.cargo\bin\doc-monster.exe` to the desktop or
make a shortcut to it: dragging files onto it converts them.

## 4. Optional extra

Doc Monster works without this; it adds capabilities.

| Extra | What it adds | macOS | Linux (Debian / Ubuntu) | Windows |
| --- | --- | --- | --- | --- |
| FFmpeg (`ffprobe`, `ffmpeg`) | Details, chapters and embedded subtitles for every video format (mp4 works without it) | `brew install ffmpeg` | `sudo apt install ffmpeg` | `winget install --id Gyan.FFmpeg -e` |

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
- **No monster on Windows**: the animation needs Windows 10 or later; on older consoles Doc Monster
  simply runs without it.
