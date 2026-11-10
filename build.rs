//! With the `bundled-tools` feature, packs FFmpeg, ffprobe, Tesseract and its language data
//! into the program, so videos work without installing anything.
//!
//! Set `DOC_MONSTER_TOOLS` to a folder holding `ffmpeg`, `ffprobe`, `tesseract` and
//! `tessdata/*.traineddata`. `Linux Binary/build.sh` makes that folder.

use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DOC_MONSTER_TOOLS");
    if std::env::var_os("CARGO_FEATURE_BUNDLED_TOOLS").is_none() {
        return;
    }
    let dir = PathBuf::from(std::env::var_os("DOC_MONSTER_TOOLS").expect(
        "the bundled-tools feature needs DOC_MONSTER_TOOLS: a folder with ffmpeg, ffprobe, \
         tesseract and tessdata/ (see Linux Binary/README.md)",
    ));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());

    let mut files: Vec<(String, PathBuf)> = ["ffmpeg", "ffprobe", "tesseract"]
        .iter()
        .map(|n| (n.to_string(), dir.join(n)))
        .collect();
    let mut langs: Vec<PathBuf> = std::fs::read_dir(dir.join("tessdata"))
        .expect("DOC_MONSTER_TOOLS/tessdata is missing")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "traineddata"))
        .collect();
    langs.sort();
    assert!(
        !langs.is_empty(),
        "no .traineddata files in DOC_MONSTER_TOOLS/tessdata"
    );
    for p in langs {
        let name = format!("tessdata/{}", p.file_name().unwrap().to_string_lossy());
        files.push((name, p));
    }

    // Each file is compressed into OUT_DIR; the list goes into bundled.rs for include_bytes!.
    let mut fingerprint: u64 = 0xcbf2_9ce4_8422_2325;
    let mut list = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for (i, (name, src)) in files.iter().enumerate() {
        println!("cargo:rerun-if-changed={}", src.display());
        let bytes = std::fs::read(src).unwrap_or_else(|e| panic!("reading {}: {e}", src.display()));
        for b in name.bytes().chain(bytes.iter().copied()) {
            fingerprint = (fingerprint ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
        let packed = out.join(format!("tool{i}.gz"));
        compress(&bytes, &packed);
        list.push_str(&format!(
            "    ({name:?}, include_bytes!({:?})),\n",
            packed.display().to_string()
        ));
    }
    list.push_str("];\n");
    list.push_str(&format!(
        "pub const FINGERPRINT: &str = \"{fingerprint:016x}\";\n"
    ));
    std::fs::write(out.join("bundled.rs"), list).unwrap();
}

fn compress(bytes: &[u8], to: &Path) {
    let file = std::fs::File::create(to).unwrap();
    let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::best());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap();
}
