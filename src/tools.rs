//! Finding the outside programs videos need: FFmpeg, ffprobe and Tesseract.
//!
//! In order: copies built into Doc Monster (the `bundled-tools` feature, used for the Linux
//! binary), then copies beside the `doc-monster` executable, then whatever is on the PATH.

use std::path::PathBuf;
use std::process::Command;

/// A command for `name` ("ffmpeg", "ffprobe" or "tesseract"), ready to add arguments to.
pub fn command(name: &str) -> Command {
    let mut cmd = Command::new(program(name));
    if name == "tesseract"
        && std::env::var_os("TESSDATA_PREFIX").is_none()
        && let Some(dir) = bundled::dir()
    {
        cmd.env("TESSDATA_PREFIX", dir.join("tessdata"));
    }
    cmd
}

fn program(name: &str) -> PathBuf {
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    if let Some(dir) = bundled::dir() {
        return dir.join(exe);
    }
    if let Some(beside) = std::env::current_exe()
        .ok()
        .and_then(|e| Some(e.parent()?.join(&exe)))
        .filter(|p| p.is_file())
    {
        return beside;
    }
    PathBuf::from(name)
}

/// Writable places for Doc Monster's own files (unpacked tools, downloaded models), most
/// permanent first, so later runs reuse them: $DOC_MONSTER_CACHE, %LOCALAPPDATA%,
/// $XDG_CACHE_HOME, ~/.cache, then the temporary folder.
pub fn cache_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = std::env::var_os("DOC_MONSTER_CACHE") {
        v.push(PathBuf::from(p));
    }
    if let Some(p) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(p).join("doc-monster"));
    }
    if let Some(p) = std::env::var_os("XDG_CACHE_HOME") {
        v.push(PathBuf::from(p).join("doc-monster"));
    }
    if let Some(p) = std::env::var_os("HOME") {
        v.push(PathBuf::from(p).join(".cache").join("doc-monster"));
    }
    v.push(std::env::temp_dir().join("doc-monster"));
    v
}

#[cfg(not(feature = "bundled-tools"))]
mod bundled {
    pub fn dir() -> Option<std::path::PathBuf> {
        None
    }
}

#[cfg(feature = "bundled-tools")]
mod bundled {
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

    /// The folder the built-in tools were unpacked into (unpacked on first use).
    pub fn dir() -> Option<PathBuf> {
        static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
        DIR.get_or_init(|| {
            super::cache_dirs()
                .into_iter()
                .find_map(|base| unpack(&base.join(format!("tools-{FINGERPRINT}"))).ok())
        })
        .clone()
    }

    fn unpack(dir: &Path) -> std::io::Result<PathBuf> {
        let done = dir.join(".complete");
        if done.is_file() {
            return Ok(dir.to_path_buf());
        }
        std::fs::create_dir_all(dir.join("tessdata"))?;
        for (name, packed) in FILES {
            let target = dir.join(name);
            let mut bytes = Vec::new();
            flate2::read::GzDecoder::new(*packed).read_to_end(&mut bytes)?;
            // Write under a private name, then rename: other runs never see a half-written file.
            let temp = dir.join(format!(
                ".{}.{}",
                std::process::id(),
                name.replace('/', "_")
            ));
            std::fs::write(&temp, &bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))?;
            }
            std::fs::rename(&temp, &target)?;
        }
        // Some systems mount temporary folders so nothing in them can run; check it does.
        let ffprobe = format!("ffprobe{}", std::env::consts::EXE_SUFFIX);
        let runs = std::process::Command::new(dir.join(ffprobe))
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !runs {
            return Err(std::io::Error::other("unpacked tools cannot run here"));
        }
        std::fs::write(done, FINGERPRINT)?;
        Ok(dir.to_path_buf())
    }
}
