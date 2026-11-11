# Doc Monster for Windows

One program, `doc-monster.exe`, with **FFmpeg and Tesseract built in**, so documents and videos
work with nothing else to install. It installs with **uv** or **winget** and is then available in
PowerShell, Command Prompt and Windows Terminal as `doc-monster`.

Needs 64-bit Windows 10 (version 1809) or later, or Windows 11, on an Intel or AMD processor.

## Install with uv

[uv](https://docs.astral.sh/uv/) installs command line tools into their own folder and puts them
on your `PATH`.

```powershell
uv tool install https://github.com/DylanCodyBrown/Doc-monster/releases/download/v0.1.0/doc_monster-0.1.0-py3-none-win_amd64.whl
doc-monster --version
```

If `doc-monster` is not found afterwards, run `uv tool update-shell` once and open a new terminal.

- **Update:** run the install command again with the newer version's link, adding `--force`.
- **Remove:** `uv tool uninstall doc-monster`.

## Install with winget

Once Doc Monster is accepted into the winget catalogue:

```powershell
winget install DylanCodyBrown.DocMonster
```

Until then, install from the manifest files attached to the release
(`winget-manifests.zip`). Local manifests have to be allowed once, from an administrator
terminal:

```powershell
winget settings --enable LocalManifestFiles
```

Then, in a normal terminal, unzip `winget-manifests.zip` into a folder and run:

```powershell
winget install --manifest .\winget-manifests
```

winget unzips the program, keeps it in its own folder, and adds the `doc-monster` command. Open a
new terminal afterwards.

- **Update:** `winget upgrade DylanCodyBrown.DocMonster`.
- **Remove:** `winget uninstall DylanCodyBrown.DocMonster`.

## Without an installer

Download `doc-monster-windows-x86_64.zip` from the
[releases page](https://github.com/DylanCodyBrown/Doc-monster/releases), unzip it anywhere, and
run `doc-monster.exe`. You can also drag files onto it in File Explorer.

## Where things go

- uv puts the program in `%USERPROFILE%\.local\bin`; winget in its own packages folder.
- The first time a video is converted, FFmpeg and Tesseract are unpacked once into
  `%LOCALAPPDATA%\doc-monster` (a few seconds), and reused after that. Removing Doc Monster does
  not remove that folder; delete it by hand if you like.
- Only English text is read on screen out of the box. For other languages, put their
  `.traineddata` files (from
  [tessdata_fast](https://github.com/tesseract-ocr/tessdata_fast)) in a folder with
  `eng.traineddata` and set `TESSDATA_PREFIX` to that folder.

## How it is built

[`.github/workflows/windows.yml`](../.github/workflows/windows.yml) runs after every push to
`main`:

1. **Tools:** [`scripts/build-tools.sh windows`](../scripts/build-tools.sh) cross-compiles FFmpeg
   7.1.1, Leptonica 1.85.0 and Tesseract 5.5.1 on Linux with Zig, as Windows programs that need
   only what every Windows 10 and 11 machine already has.
2. **Doc Monster:** built on Windows with `--features bundled-tools`, which packs the tools inside.
3. **Checks:** the test suite runs on Windows; a test video is converted with the `PATH` emptied,
   so no other FFmpeg or Tesseract could be used; and the wheel is installed with uv.
4. **Packages:** the uv wheel (built with [maturin](https://www.maturin.rs/)), the winget zip,
   and the three winget manifest files with the zip's fingerprint filled in.
5. **Releases:** each version gets a release `v<version>` once, which never changes afterwards
   because winget checks the zip's fingerprint. Every push also refreshes the `windows-latest`
   pre-release with the newest build. To publish a new version, raise `version` in `Cargo.toml`.

The manifest templates are in [`winget/`](winget). To list Doc Monster in the public winget
catalogue, submit them to [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) once
per version, for example with
[wingetcreate](https://github.com/microsoft/winget-create):
`wingetcreate submit --token <your GitHub token> <folder with the three files>`.

## Licenses

Doc Monster is MIT or Apache 2.0. FFmpeg (GPL 2.0 or later, because the `mpdecimate` filter is
GPL), Tesseract (Apache 2.0) and Leptonica (BSD 2-clause) keep their own licenses; they are
included in the zip under `licenses/`, and the exact sources are the versions listed above.
