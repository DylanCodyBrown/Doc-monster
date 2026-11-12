#!/usr/bin/env bash
# Builds "Linux Binary/doc-monster": one static Linux executable with FFmpeg, ffprobe and
# Tesseract built in. Run from anywhere on an x86_64 Linux machine with Rust installed:
#
#   bash "Linux Binary/build.sh"
#
# Needs: git, a C compiler, make, cmake, nasm, musl-tools, python3 with pip.
# On Ubuntu or Debian: sudo apt-get install -y build-essential cmake nasm musl-tools python3-pip
set -euo pipefail

TARGET=x86_64-unknown-linux-musl
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${WORK:-$REPO/target/bundled-tools}"
export WORK

# FFmpeg, Leptonica and Tesseract, static for Linux (see scripts/build-tools.sh).
bash "$REPO/scripts/build-tools.sh" linux

rustup target add "$TARGET"
cd "$REPO"
DOC_MONSTER_TOOLS="$WORK/linux/tools" cargo build --release --locked --target "$TARGET" --features bundled-tools,model
cp "target/$TARGET/release/doc-monster" "Linux Binary/doc-monster"
file "Linux Binary/doc-monster"
"Linux Binary/doc-monster" --version
sha256sum "Linux Binary/doc-monster"
