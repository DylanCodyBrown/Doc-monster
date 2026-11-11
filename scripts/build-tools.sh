#!/usr/bin/env bash
# Builds the programs Doc Monster carries inside itself for videos: FFmpeg, ffprobe and
# Tesseract (with its English text model), as static programs for Linux or for Windows.
# Runs on x86_64 Linux in both cases; Windows programs are cross-compiled with Zig.
#
#   bash scripts/build-tools.sh linux      # -> $WORK/linux/tools
#   bash scripts/build-tools.sh windows    # -> $WORK/windows/tools
#
# Needs: git, make, cmake, nasm, python3 with pip; for Linux also musl-tools; for Windows also
# LLVM 18 (llvm-18). On Ubuntu: sudo apt-get install -y build-essential cmake nasm musl-tools llvm-18
# Then build Doc Monster with DOC_MONSTER_TOOLS=$WORK/<os>/tools and --features bundled-tools.
set -euo pipefail

OS="${1:?usage: build-tools.sh linux|windows}"
FFMPEG_TAG=n7.1.1
LEPTONICA_TAG=1.85.0
TESSERACT_TAG=5.5.1
TESSDATA_TAG=4.1.0   # tessdata_fast: small, fast English model
ENG_SHA256=7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2
ZIG_VERSION=0.17.1

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${WORK:-$REPO/target/bundled-tools}"
JOBS="$(nproc)"
OUT="$WORK/$OS/out"
TOOLS="$WORK/$OS/tools"
mkdir -p "$WORK/bin" "$OUT" "$TOOLS/tessdata"
cd "$WORK"

# Zig compiles C and C++ for musl Linux and for Windows, bringing its own static C++ library.
python3 -m pip install --quiet "ziglang==$ZIG_VERSION" 2>/dev/null \
  || python3 -m pip install --quiet --break-system-packages "ziglang==$ZIG_VERSION"
ZIG="$(python3 -c 'import ziglang, os; print(os.path.join(os.path.dirname(ziglang.__file__), "zig"))')"
WININC="$(dirname "$ZIG")/lib/libc/include/any-windows-any"
cat > bin/zcc-linux <<EOF
#!/bin/sh
exec "$ZIG" cc -target x86_64-linux-musl "\$@"
EOF
cat > bin/zcxx-linux <<EOF
#!/bin/sh
exec "$ZIG" c++ -target x86_64-linux-musl "\$@"
EOF
# Zig's Windows linker sets these itself and rejects them when asked, so they are dropped.
cat > bin/zcc-windows <<EOF
#!/bin/sh
for a do
  shift
  case "\$a" in
    -Wl,--nxcompat,--dynamicbase|-Wl,--pic-executable,-e,mainCRTStartup|-Wl,--image-base,*) ;;
    *) set -- "\$@" "\$a" ;;
  esac
done
exec "$ZIG" cc -target x86_64-windows-gnu "\$@"
EOF
cat > bin/zcxx-windows <<EOF
#!/bin/sh
exec "$ZIG" c++ -target x86_64-windows-gnu "\$@"
EOF
cat > bin/zar <<EOF
#!/bin/sh
exec "$ZIG" ar "\$@"
EOF
cat > bin/zranlib <<EOF
#!/bin/sh
exec "$ZIG" ranlib "\$@"
EOF
cat > bin/zwindres <<EOF
#!/bin/sh
exec llvm-windres-18 --target=pe-x86-64 -I "$WININC" "\$@"
EOF
chmod +x bin/*

fetch() { # tag url folder
  [[ -d "$3" ]] || git clone --quiet --depth 1 -b "$1" "$2" "$3"
}
fetch "$FFMPEG_TAG" https://github.com/FFmpeg/FFmpeg ffmpeg
fetch "$LEPTONICA_TAG" https://github.com/DanBloomberg/leptonica leptonica
fetch "$TESSERACT_TAG" https://github.com/tesseract-ocr/tesseract tesseract
# Windows names its networking library in lower case; Linux file names are case-sensitive.
sed -i 's/set(LIB_Ws2_32 Ws2_32)/set(LIB_Ws2_32 ws2_32)/' tesseract/CMakeLists.txt

if [[ "$OS" == linux ]]; then
  EXE=""
  FF_TARGET=(--cc=musl-gcc --extra-ldflags=-static)
  CMAKE_TARGET=(-DCMAKE_C_COMPILER="$WORK/bin/zcc-linux" -DCMAKE_CXX_COMPILER="$WORK/bin/zcxx-linux"
                -DCMAKE_EXE_LINKER_FLAGS=-static)
  STRIP=strip
else
  EXE=".exe"
  FF_TARGET=(--target-os=mingw32 --arch=x86_64 --enable-cross-compile --enable-w32threads
             --cc="$WORK/bin/zcc-windows" --cxx="$WORK/bin/zcxx-windows"
             --ar="$WORK/bin/zar" --ranlib="$WORK/bin/zranlib"
             --nm=llvm-nm-18 --strip=llvm-strip-18 --windres="$WORK/bin/zwindres")
  # The processor must be named, or Tesseract leaves out its fast AVX and SSE code.
  CMAKE_TARGET=(-DCMAKE_SYSTEM_NAME=Windows -DCMAKE_SYSTEM_PROCESSOR=x86_64
                -DCMAKE_C_COMPILER="$WORK/bin/zcc-windows" -DCMAKE_CXX_COMPILER="$WORK/bin/zcxx-windows"
                -DCMAKE_RC_COMPILER="$WORK/bin/zwindres" -DLEPT_TIFF_RESULT=1)
  STRIP=llvm-strip-18
fi
CMAKE_COMMON=(-DCMAKE_BUILD_TYPE=Release -DCMAKE_AR="$WORK/bin/zar" -DCMAKE_RANLIB="$WORK/bin/zranlib"
              -DCMAKE_INSTALL_PREFIX="$OUT" -DCMAKE_PREFIX_PATH="$OUT" -DBUILD_SHARED_LIBS=OFF)

# 1. FFmpeg and ffprobe: only what Doc Monster uses (reading common videos and subtitles,
#    the frame filters, PGM and raw output). GPL because the mpdecimate filter is GPL.
if [[ ! -x "$OUT/bin/ffmpeg$EXE" ]]; then
  mkdir -p "ffmpeg-build-$OS"
  (cd "ffmpeg-build-$OS" && ../ffmpeg/configure --prefix="$OUT" "${FF_TARGET[@]}" --enable-gpl \
    --enable-static --disable-shared \
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
if [[ ! -f "$OUT/lib/libleptonica.a" ]]; then
  cmake -S leptonica -B "leptonica/build-$OS" "${CMAKE_COMMON[@]}" "${CMAKE_TARGET[@]}" \
    -DBUILD_PROG=OFF -DSW_BUILD=OFF -DENABLE_ZLIB=OFF -DENABLE_PNG=OFF -DENABLE_GIF=OFF \
    -DENABLE_JPEG=OFF -DENABLE_TIFF=OFF -DENABLE_WEBP=OFF -DENABLE_OPENJPEG=OFF
  cmake --build "leptonica/build-$OS" -j"$JOBS" && cmake --install "leptonica/build-$OS"
fi

# 3. Tesseract, without network downloads, training tools or threads.
if [[ ! -x "tesseract/build-$OS/bin/tesseract$EXE" ]]; then
  cmake -S tesseract -B "tesseract/build-$OS" "${CMAKE_COMMON[@]}" "${CMAKE_TARGET[@]}" \
    -DBUILD_TRAINING_TOOLS=OFF -DBUILD_TESTS=OFF -DDISABLE_ARCHIVE=ON -DDISABLE_CURL=ON \
    -DDISABLE_TIFF=ON -DOPENMP_BUILD=OFF -DGRAPHICS_DISABLED=ON -DENABLE_NATIVE=OFF -DSW_BUILD=OFF
  cmake --build "tesseract/build-$OS" -j"$JOBS" --target tesseract
fi

# 4. English language data, checked against a known fingerprint.
if [[ ! -f tessdata_fast/eng.traineddata ]]; then
  git clone --quiet --depth 1 -b "$TESSDATA_TAG" --filter=blob:none --no-checkout \
    https://github.com/tesseract-ocr/tessdata_fast
  git -C tessdata_fast checkout --quiet "$TESSDATA_TAG" -- eng.traineddata
fi
echo "$ENG_SHA256  tessdata_fast/eng.traineddata" | sha256sum --check --quiet

# 5. The folder Doc Monster packs in.
cp "$OUT/bin/ffmpeg$EXE" "$OUT/bin/ffprobe$EXE" "$TOOLS/"
cp "tesseract/build-$OS/bin/tesseract$EXE" "$TOOLS/" && "$STRIP" "$TOOLS/tesseract$EXE"
cp tessdata_fast/eng.traineddata "$TOOLS/tessdata/"
ls -la "$TOOLS"
