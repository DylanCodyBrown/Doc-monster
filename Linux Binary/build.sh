#!/usr/bin/env bash
# Builds "Linux Binary/doc-monster": one static Linux executable with FFmpeg, ffprobe and
# Tesseract built in. Run from anywhere on an x86_64 Linux machine with Rust installed:
#
#   bash "Linux Binary/build.sh"
#
# Needs: git, a C compiler, make, cmake, nasm, musl-tools, python3 with pip.
# On Ubuntu or Debian: sudo apt-get install -y build-essential cmake nasm musl-tools python3-pip
set -euo pipefail

FFMPEG_TAG=n7.1.1
LEPTONICA_TAG=1.85.0
TESSERACT_TAG=5.5.1
TESSDATA_TAG=4.1.0   # tessdata_fast: small, fast English model
ENG_SHA256=7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2
TARGET=x86_64-unknown-linux-musl

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${WORK:-$REPO/target/bundled-tools}"
JOBS="$(nproc)"
mkdir -p "$WORK/bin" "$WORK/out" "$WORK/tools/tessdata"
cd "$WORK"

# Zig is used as the C and C++ compiler for musl: it brings its own static C++ library.
python3 -m pip install --quiet "ziglang==0.17.1" || python3 -m pip install --quiet --break-system-packages "ziglang==0.17.1"
ZIG="$(python3 -c 'import ziglang, os; print(os.path.join(os.path.dirname(ziglang.__file__), "zig"))')"
for tool in "cc:zcc" "c++:zcxx" "ar:zar" "ranlib:zranlib"; do
  sub="${tool%%:*}"; name="${tool##*:}"
  extra=""; [[ "$sub" == cc || "$sub" == c++ ]] && extra="-target x86_64-linux-musl"
  printf '#!/bin/sh\nexec "%s" %s %s "$@"\n' "$ZIG" "$sub" "$extra" > "bin/$name"
  chmod +x "bin/$name"
done

fetch() { # tag url folder
  [[ -d "$3" ]] || git clone --quiet --depth 1 -b "$1" "$2" "$3"
}
fetch "$FFMPEG_TAG" https://github.com/FFmpeg/FFmpeg ffmpeg
fetch "$LEPTONICA_TAG" https://github.com/DanBloomberg/leptonica leptonica
fetch "$TESSERACT_TAG" https://github.com/tesseract-ocr/tesseract tesseract

# 1. FFmpeg and ffprobe: only what Doc Monster uses (reading common videos and subtitles,
#    the frame filters, PGM and raw output). GPL because the mpdecimate filter is GPL.
if [[ ! -x out/bin/ffmpeg ]]; then
  (cd ffmpeg && ./configure --prefix="$WORK/out" --cc=musl-gcc --enable-gpl \
    --enable-static --disable-shared --extra-ldflags=-static \
    --disable-everything --disable-autodetect --disable-doc --disable-debug --disable-network \
    --disable-ffplay --disable-avdevice --disable-swresample --enable-ffmpeg --enable-ffprobe \
    --enable-protocol=file,pipe \
    --enable-demuxer=mov,matroska,avi,flv,mpegts,mpegps,asf,ogg,mpegvideo,h264,hevc,m4v,ivf,srt,webvtt,ass,image2 \
    --enable-parser=h264,hevc,mpeg4video,mpegvideo,vp8,vp9,av1,mjpeg,vc1 \
    --enable-decoder=h264,hevc,mpeg4,mpeg2video,mpeg1video,msmpeg4v1,msmpeg4v2,msmpeg4v3,wmv1,wmv2,wmv3,vc1,vp8,vp9,theora,flv,h263,mjpeg,prores,rawvideo,dvvideo,subrip,srt,ass,ssa,webvtt,mov_text,text \
    --enable-encoder=pgm,rawvideo,subrip,srt \
    --enable-muxer=image2,rawvideo,srt,null \
    --enable-filter=fps,mpdecimate,showinfo,split,scale,format,null \
    --enable-bsf=h264_mp4toannexb,hevc_mp4toannexb,vp9_superframe_split \
    && make -j"$JOBS" && make install)
fi

# 2. Leptonica, Tesseract's image library, with no image format libraries: frames are PGM.
if [[ ! -f out/lib/libleptonica.a ]]; then
  cmake -S leptonica -B leptonica/build -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_COMPILER="$WORK/bin/zcc" -DCMAKE_AR="$WORK/bin/zar" -DCMAKE_RANLIB="$WORK/bin/zranlib" \
    -DCMAKE_INSTALL_PREFIX="$WORK/out" -DBUILD_SHARED_LIBS=OFF -DBUILD_PROG=OFF -DSW_BUILD=OFF \
    -DENABLE_ZLIB=OFF -DENABLE_PNG=OFF -DENABLE_GIF=OFF -DENABLE_JPEG=OFF -DENABLE_TIFF=OFF \
    -DENABLE_WEBP=OFF -DENABLE_OPENJPEG=OFF
  cmake --build leptonica/build -j"$JOBS" && cmake --install leptonica/build
fi

# 3. Tesseract, fully static.
if [[ ! -x tesseract/build/bin/tesseract ]]; then
  cmake -S tesseract -B tesseract/build -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_COMPILER="$WORK/bin/zcc" -DCMAKE_CXX_COMPILER="$WORK/bin/zcxx" \
    -DCMAKE_AR="$WORK/bin/zar" -DCMAKE_RANLIB="$WORK/bin/zranlib" \
    -DCMAKE_INSTALL_PREFIX="$WORK/out" -DCMAKE_PREFIX_PATH="$WORK/out" -DBUILD_SHARED_LIBS=OFF \
    -DBUILD_TRAINING_TOOLS=OFF -DBUILD_TESTS=OFF -DDISABLE_ARCHIVE=ON -DDISABLE_CURL=ON \
    -DDISABLE_TIFF=ON -DOPENMP_BUILD=OFF -DGRAPHICS_DISABLED=ON -DENABLE_NATIVE=OFF \
    -DCMAKE_EXE_LINKER_FLAGS=-static
  cmake --build tesseract/build -j"$JOBS" --target tesseract
fi

# 4. English language data, checked against a known fingerprint.
if [[ ! -f tessdata_fast/eng.traineddata ]]; then
  git clone --quiet --depth 1 -b "$TESSDATA_TAG" --filter=blob:none --no-checkout \
    https://github.com/tesseract-ocr/tessdata_fast
  git -C tessdata_fast checkout --quiet "$TESSDATA_TAG" -- eng.traineddata
fi
echo "$ENG_SHA256  tessdata_fast/eng.traineddata" | sha256sum --check --quiet

# 5. The folder Doc Monster packs in, then Doc Monster itself.
cp out/bin/ffmpeg out/bin/ffprobe tools/
cp tesseract/build/bin/tesseract tools/ && strip tools/tesseract
cp tessdata_fast/eng.traineddata tools/tessdata/

rustup target add "$TARGET"
cd "$REPO"
DOC_MONSTER_TOOLS="$WORK/tools" cargo build --release --locked --target "$TARGET" --features bundled-tools
cp "target/$TARGET/release/doc-monster" "Linux Binary/doc-monster"
file "Linux Binary/doc-monster"
"Linux Binary/doc-monster" --version
sha256sum "Linux Binary/doc-monster"
