//! How Doc Monster looks in a terminal: a neon green accent, a one-line progress indicator while
//! it eats, and a small pet monster that sits beside the input prompt in drop mode.
//!
//! Styling only happens on an interactive terminal. When output goes to a file, a pipe, a
//! script or an agent, every line is the same plain text as always.

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The pet's mood, shown on its face.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mood {
    Hungry,
    Happy,
    Queasy,
}

/// Characters for terminals that show Unicode, and plain ones for the old Windows console.
struct Glyphs {
    ok: &'static str,
    fail: &'static str,
    arrow: &'static str,
    dot: &'static str,
    prompt: &'static str,
    spin: &'static [&'static str],
}

const FANCY: Glyphs = Glyphs {
    ok: "✓",
    fail: "✗",
    arrow: "→",
    dot: "·",
    prompt: "›",
    spin: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
};

const PLAIN: Glyphs = Glyphs {
    ok: "+",
    fail: "x",
    arrow: "->",
    dot: "-",
    prompt: ">",
    spin: &["|", "/", "-", "\\"],
};

pub struct Ui {
    /// Style standard output (result lines, the pet and its prompt).
    out: bool,
    /// Style standard error (failures, the progress line).
    err: bool,
    /// Draw the progress line and the pet at all.
    live: bool,
    truecolor: bool,
    glyphs: &'static Glyphs,
}

fn styled(stream_is_terminal: bool) -> bool {
    stream_is_terminal
        && std::env::var("TERM").map_or(true, |t| t != "dumb")
        && std::env::var_os("NO_COLOR").is_none()
}

impl Ui {
    /// `wanted` is false for --no-anim, --quiet and --json.
    pub fn new(wanted: bool) -> Ui {
        // Windows consoles need escape codes switched on; if that fails, stay plain.
        let ansi = enable_ansi_support::enable_ansi_support().is_ok();
        let out = ansi && styled(std::io::stdout().is_terminal());
        let err = ansi && styled(std::io::stderr().is_terminal());
        // The old Windows console font has no check marks or spinner dots; Windows Terminal does.
        let modern = !cfg!(windows) || std::env::var_os("WT_SESSION").is_some();
        Ui {
            out,
            err,
            live: wanted && err,
            truecolor: std::env::var("COLORTERM")
                .is_ok_and(|c| c.contains("truecolor") || c.contains("24bit"))
                || std::env::var_os("WT_SESSION").is_some(),
            glyphs: if modern { &FANCY } else { &PLAIN },
        }
    }

    /// True when drop mode draws the pet (which carries the instructions itself).
    pub fn shows_pet(&self) -> bool {
        self.out && self.live
    }

    #[cfg(test)]
    fn for_test(styled: bool) -> Ui {
        Ui {
            out: styled,
            err: styled,
            live: styled,
            truecolor: true,
            glyphs: &FANCY,
        }
    }

    fn neon(&self, s: &str) -> String {
        if self.truecolor {
            format!("\x1b[38;2;57;255;20m{s}\x1b[0m")
        } else {
            format!("\x1b[38;5;82m{s}\x1b[0m")
        }
    }
    fn bold_neon(&self, s: &str) -> String {
        format!("\x1b[1m{}", self.neon(s))
    }
    fn dim(&self, s: &str) -> String {
        format!("\x1b[2m{s}\x1b[0m")
    }
    fn red(&self, s: &str) -> String {
        format!("\x1b[38;5;203m{s}\x1b[0m")
    }

    /// A file was eaten.
    pub fn ate(&self, source: &Path, output: &Path) {
        if !self.out {
            println!("ate   {} -> {}", source.display(), output.display());
            return;
        }
        let name = |p: &Path| {
            p.file_name()
                .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into())
        };
        let g = self.glyphs;
        let mut line = format!(
            "  {} {} {} {}",
            self.neon(g.ok),
            name(source),
            self.dim(g.arrow),
            self.neon(&name(output))
        );
        // The folder, dimmed after the names; in full when the output went somewhere else.
        let folder = if output.parent() == source.parent() {
            source.parent().map(short).unwrap_or_default()
        } else {
            output.parent().map(short).unwrap_or_default()
        };
        if !folder.is_empty() {
            line.push_str(&format!("  {}", self.dim(&folder)));
        }
        println!("{line}");
    }

    /// A file could not be eaten.
    pub fn spat(&self, source: &Path, why: &str) {
        if !self.err {
            eprintln!("spat  {}: {why}", source.display());
            return;
        }
        let name = source.file_name().map_or_else(
            || source.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        eprintln!(
            "  {} {}  {}",
            self.red(self.glyphs.fail),
            name,
            self.dim(why)
        );
    }

    /// An index was written.
    pub fn indexed(&self, index: &Path, files: usize, unique: usize, extra: &str) {
        if !self.out {
            println!(
                "index {} ({files} files, {unique} unique{extra})",
                index.display()
            );
            return;
        }
        println!(
            "  {} {} {}",
            self.neon(self.glyphs.ok),
            index.display(),
            self.dim(&format!("{files} files, {unique} unique{extra}"))
        );
    }

    /// The closing line of a batch.
    pub fn done(&self, converted: usize, failed: usize, skipped: usize, took: Duration) {
        if !self.out {
            let skipped = if skipped > 0 {
                format!(", {skipped} earlier outputs skipped")
            } else {
                String::new()
            };
            println!("done: {converted} converted, {failed} failed{skipped}");
            return;
        }
        let d = self.dim(self.glyphs.dot);
        let mut parts = vec![self.bold_neon(&format!("{converted} eaten"))];
        if failed > 0 {
            parts.push(self.red(&format!("{failed} failed")));
        }
        if skipped > 0 {
            parts.push(self.dim(&format!("{skipped} earlier outputs skipped")));
        }
        parts.push(self.dim(&format!("{:.1}s", took.as_secs_f64())));
        println!("  {}", parts.join(&format!(" {d} ")));
    }

    /// Run `work` with a one-line progress indicator on standard error. `work` reports each
    /// file it starts and finishes through the tracker.
    pub fn working<T>(&self, total: usize, work: impl FnOnce(&Tracker) -> T) -> T {
        let tracker = Tracker {
            done: AtomicUsize::new(0),
            current: Mutex::new(String::new()),
        };
        if !self.live {
            return work(&tracker);
        }
        let stop = AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut err = std::io::stderr();
                let start = Instant::now();
                let mut t = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    // Quick jobs finish without the line ever flashing up.
                    if start.elapsed() >= Duration::from_millis(150) {
                        let line = self.progress_line(t, &tracker, total);
                        let _ = write!(err, "\r\x1b[2K{line}");
                        let _ = err.flush();
                        t += 1;
                    }
                    std::thread::sleep(Duration::from_millis(80));
                }
                let _ = write!(err, "\r\x1b[2K");
                let _ = err.flush();
            });
            let out = work(&tracker);
            stop.store(true, Ordering::Relaxed);
            out
        })
    }

    fn progress_line(&self, t: usize, tracker: &Tracker, total: usize) -> String {
        let g = self.glyphs;
        let spin = g.spin[t % g.spin.len()];
        let done = tracker.done.load(Ordering::Relaxed);
        let name: String = tracker.current.lock().unwrap().chars().take(48).collect();
        let count = if total > 1 {
            format!(" {}/{total}", done.min(total))
        } else {
            String::new()
        };
        format!(
            "  {} {}{} {}",
            self.neon(spin),
            self.neon("eating"),
            self.dim(&count),
            self.dim(&name)
        )
    }

    /// Drop mode: the pet, a few words beside it, and the prompt on its last row. Returns
    /// without a newline so the input is typed right next to the pet.
    pub fn pet_prompt(&self, mood: Mood, status: &str) {
        let mut out = std::io::stdout();
        if !self.shows_pet() {
            let _ = write!(out, "feed me > ");
            let _ = out.flush();
            return;
        }
        let rows = pet(mood);
        let title = format!(
            "{} {}",
            self.bold_neon("Doc Monster"),
            self.dim(env!("CARGO_PKG_VERSION"))
        );
        let help = self.dim("drop files or folders here, then press Enter");
        let words = [
            title,
            help,
            status.to_string(),
            self.neon(self.glyphs.prompt),
        ];
        // A blank row above the pet keeps it apart from the results printed before it.
        let mut text = String::from("\n");
        for (i, (row, word)) in rows.iter().zip(words).enumerate() {
            text.push_str(&format!("  {}  {word}", self.neon(row)));
            if i < 3 {
                text.push('\n');
            } else {
                text.push(' ');
            }
        }
        let _ = write!(out, "{text}");
        let _ = out.flush();
    }

    /// Erase the pet and the line typed beside it, once Enter is pressed, so there is only
    /// ever one pet on screen: it is drawn again below whatever the batch prints.
    pub fn clear_pet(&self, typed: &str) {
        if !self.shows_pet() {
            return;
        }
        let width = terminal_size::terminal_size_of(std::io::stdout())
            .map_or(80, |(w, _)| usize::from(w.0).max(20));
        // The blank row and three pet rows above the prompt, then the prompt and what was typed.
        let up = 4 + rows_taken(PROMPT_COLUMN + typed.trim_end().chars().count(), width);
        let mut out = std::io::stdout();
        let _ = write!(out, "\x1b[{up}A\r\x1b[J");
        let _ = out.flush();
    }

    /// One dim line recording what was dropped, in place of the erased input.
    pub fn fed(&self, inputs: &[std::path::PathBuf]) {
        if !self.shows_pet() || inputs.is_empty() {
            return;
        }
        let names: Vec<String> = inputs
            .iter()
            .map(|p| {
                p.file_name()
                    .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into())
            })
            .collect();
        // Where the pet sat: a blank row, then what was dropped.
        println!(
            "\n  {} {}",
            self.neon(self.glyphs.prompt),
            self.dim(&names.join("  "))
        );
    }

    /// What the pet says after a batch.
    pub fn after_meal(&self, eaten: usize, failed: usize) -> (Mood, String) {
        let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
        match (eaten, failed) {
            (0, 0) => (Mood::Hungry, self.dim("nothing in there to eat")),
            (_, 0) => (
                Mood::Happy,
                self.neon(&format!("burp. ate {}", plural(eaten, "document"))),
            ),
            (_, f) => (
                Mood::Queasy,
                self.red(&format!("{} didn't agree with me", plural(f, "file"))),
            ),
        }
    }

    pub fn hungry(&self) -> String {
        self.dim("hungry. empty line quits")
    }
}

/// A folder for display: the home folder written as `~`, the current folder as `.`.
fn short(dir: &Path) -> String {
    if dir.as_os_str().is_empty() {
        return String::new();
    }
    if let Ok(here) = std::env::current_dir()
        && let Ok(rest) = dir.strip_prefix(&here)
    {
        let rest = rest.display().to_string();
        return if rest.is_empty() { String::new() } else { rest };
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
        && let Ok(rest) = dir.strip_prefix(&home)
    {
        return format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display());
    }
    dir.display().to_string()
}

/// Progress shared with the work: how many files are done, and which one is being eaten.
pub struct Tracker {
    done: AtomicUsize,
    current: Mutex<String>,
}

impl Tracker {
    pub fn start(&self, name: &str) {
        *self.current.lock().unwrap() = name.to_string();
    }
    pub fn finish(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }
}

/// Where typing starts on the prompt row: indent, pet, gap, prompt sign and a space.
const PROMPT_COLUMN: usize = 2 + 9 + 2 + 2;

/// Screen rows a line of `columns` characters takes, wrapping at `width`.
fn rows_taken(columns: usize, width: usize) -> usize {
    columns.max(1).div_ceil(width)
}

/// The pet: a little slime ghost, four rows of eleven columns.
fn pet(mood: Mood) -> [&'static str; 4] {
    let (eyes, mouth) = match mood {
        Mood::Hungry => ("( o   o )", "(  \\_/  )"),
        Mood::Happy => ("( ^   ^ )", "(  \\_/  )"),
        Mood::Queasy => ("( ;   ; )", "(  ~~~  )"),
    };
    [" .-~~~-. ", eyes, mouth, " '~^~^~' "]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pet_is_small_and_square_in_every_mood() {
        for mood in [Mood::Hungry, Mood::Happy, Mood::Queasy] {
            let rows = pet(mood);
            assert_eq!(rows.len(), 4);
            for r in rows {
                assert_eq!(r.chars().count(), 9, "{r:?}");
            }
        }
        assert_ne!(pet(Mood::Hungry), pet(Mood::Queasy));
    }

    #[test]
    fn plain_output_is_unchanged_for_scripts_and_agents() {
        let ui = Ui::for_test(true);
        let line = ui.progress_line(
            0,
            &Tracker {
                done: AtomicUsize::new(2),
                current: Mutex::new("report.docx".into()),
            },
            5,
        );
        assert!(line.contains("eating") && line.contains("2/5") && line.contains("report.docx"));
        assert!(
            line.contains("\x1b[38;2;57;255;20m"),
            "neon green: {line:?}"
        );
    }

    #[test]
    fn erasing_the_pet_counts_wrapped_input_lines() {
        assert_eq!(rows_taken(PROMPT_COLUMN, 80), 1);
        assert_eq!(rows_taken(PROMPT_COLUMN + 65, 80), 1);
        assert_eq!(rows_taken(PROMPT_COLUMN + 66, 80), 2);
        assert_eq!(rows_taken(PROMPT_COLUMN + 300, 80), 4);
    }

    #[test]
    fn the_pets_mood_follows_the_meal() {
        let ui = Ui::for_test(false);
        assert_eq!(ui.after_meal(3, 0).0, Mood::Happy);
        assert_eq!(ui.after_meal(3, 1).0, Mood::Queasy);
        assert_eq!(ui.after_meal(0, 0).0, Mood::Hungry);
    }
}
