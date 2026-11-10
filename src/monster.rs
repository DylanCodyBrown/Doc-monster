//! The animated fat monster that chomps documents while they are converted.
//!
//! Drawn on standard error, and only when standard error is an interactive terminal, so
//! scripts, pipes and agents get clean output automatically.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(80);
const LANE: usize = 22; // width of the space the document slides across
const BODY: usize = 29; // width of the monster

#[derive(Clone, Copy, PartialEq)]
enum Mouth {
    Wide,
    Half,
    Shut,
    Full,
}

fn body(mouth: Mouth, happy: bool) -> [String; 11] {
    let eyes = if happy {
        ("(^)", "(^)")
    } else {
        ("(o)", "(o)")
    };
    let m: [&str; 3] = match mouth {
        Mouth::Wide => [
            r"   |    .-'''''''-.    |",
            r"   |   (   N O M   )   |",
            r"   |    '-._____.-'    |",
        ],
        Mouth::Half => [
            r"   |    .-.......-.    |",
            r"   |   (___________)   |",
            r"   |    '---------'    |",
        ],
        Mouth::Shut => [
            r"   |                   |",
            r"   |   (===========)   |",
            r"   |                   |",
        ],
        Mouth::Full => [
            r"   |                   |",
            r"   |   \___________/   |",
            r"   |       burp!       |",
        ],
    };
    let belly = if happy {
        r"    \ (  o  o  o  o ) /"
    } else {
        r"    \ (             ) /"
    };
    [
        r"       .-'''''''''''-.".into(),
        r"     .'               '.".into(),
        format!(r"    /   {}     {}   \", eyes.0, eyes.1),
        r"   |                   |".into(),
        m[0].into(),
        m[1].into(),
        m[2].into(),
        r"   (   .-----------.   )".into(),
        belly.into(),
        r"     '.'-.._____..-'.'".into(),
        r"       /_/       \_\".into(),
    ]
}

const DOC: [&str; 4] = [" ___ ", "|=- |", "|-= |", "|___|"];

pub struct Monster {
    enabled: bool,
    color: bool,
}

impl Monster {
    pub fn new(wanted: bool) -> Monster {
        let terminal =
            std::io::stderr().is_terminal() && std::env::var("TERM").map_or(true, |t| t != "dumb");
        // Windows consoles need escape codes switched on; if that fails, stay quiet.
        let ansi = enable_ansi_support::enable_ansi_support().is_ok();
        let enabled = wanted && terminal && ansi;
        Monster {
            enabled,
            color: enabled && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    fn paint(&self, s: &str, code: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    /// One frame: the document slides toward the mouth, then the monster chews.
    fn frame(&self, t: usize, name: &str) -> Vec<String> {
        const APPROACH: usize = 7;
        let cycle = APPROACH + 6;
        let step = t % cycle;
        let doc_x = (step < APPROACH).then(|| step * (LANE - DOC[0].len()) / (APPROACH - 1));
        let mouth = match step {
            s if s < APPROACH => {
                if s >= APPROACH - 3 {
                    Mouth::Wide
                } else {
                    Mouth::Half
                }
            }
            s => [
                Mouth::Shut,
                Mouth::Half,
                Mouth::Shut,
                Mouth::Half,
                Mouth::Shut,
                Mouth::Wide,
            ][(s - APPROACH) % 6],
        };
        let crumbs = step >= APPROACH && (step - APPROACH).is_multiple_of(2);
        let mut lines = Vec::with_capacity(13);
        for (row, b) in body(mouth, false).iter().enumerate() {
            let mut lane = " ".repeat(LANE);
            if let Some(x) = doc_x
                && (3..7).contains(&row)
            {
                lane.replace_range(x..x + DOC[0].len(), DOC[row - 3]);
            }
            let lane = self.paint(&lane, "33");
            let mut b = format!("{b:<BODY$}");
            if crumbs && row == 6 {
                b.push_str(" * .");
            }
            lines.push(format!("{lane}{}", self.paint(&b, "32")));
        }
        let shown: String = name.chars().take(LANE + BODY - 14).collect();
        lines.push(format!(
            "  {} {shown}",
            self.paint(["nom", "nom nom", "nom nom nom"][t / 3 % 3], "1")
        ));
        lines
    }

    /// Run `work` while the monster eats `name`; the animation lasts at least `min`.
    pub fn eat<T>(&self, name: &str, min: Duration, work: impl FnOnce() -> T) -> T {
        if !self.enabled {
            return work();
        }
        let done = AtomicBool::new(false);
        std::thread::scope(|s| {
            let anim = s.spawn(|| {
                let start = Instant::now();
                let mut err = std::io::stderr();
                let mut drawn = 0usize;
                let mut t = 0usize;
                loop {
                    let lines = self.frame(t, name);
                    let mut buf = String::new();
                    if drawn > 0 {
                        buf.push_str(&format!("\x1b[{drawn}A"));
                    }
                    for l in &lines {
                        buf.push_str("\x1b[2K");
                        buf.push_str(l);
                        buf.push('\n');
                    }
                    let _ = err.write_all(buf.as_bytes());
                    let _ = err.flush();
                    drawn = lines.len();
                    std::thread::sleep(FRAME);
                    t += 1;
                    if done.load(Ordering::Relaxed) && start.elapsed() >= min {
                        break;
                    }
                }
                // Wipe the monster so the result line takes its place.
                let _ = err.write_all(format!("\x1b[{drawn}A\x1b[J").as_bytes());
                let _ = err.flush();
            });
            let out = work();
            done.store(true, Ordering::Relaxed);
            let _ = anim.join();
            out
        })
    }

    /// The hungry monster, shown when waiting for files to be dropped in.
    pub fn hello(&self) {
        if !self.enabled {
            return;
        }
        let mut out = String::new();
        for b in body(Mouth::Wide, false) {
            out.push_str(&self.paint(&b, "32"));
            out.push('\n');
        }
        out.push_str(&format!("{}\n", self.paint("   Feed me documents!", "1")));
        let _ = std::io::stderr().write_all(out.as_bytes());
    }

    /// The satisfied monster, shown once at the end.
    pub fn burp(&self, eaten: usize) {
        if !self.enabled || eaten == 0 {
            return;
        }
        let mut out = String::new();
        for b in body(Mouth::Full, true) {
            out.push_str(&" ".repeat(LANE));
            out.push_str(&self.paint(&b, "32"));
            out.push('\n');
        }
        let plural = if eaten == 1 { "document" } else { "documents" };
        out.push_str(&format!(
            "{}{}\n",
            " ".repeat(LANE),
            self.paint(&format!("  Burp. Ate {eaten} {plural}."), "1")
        ));
        let _ = std::io::stderr().write_all(out.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_rectangular_and_the_document_reaches_the_mouth() {
        let m = Monster {
            enabled: true,
            color: false,
        };
        for t in 0..30 {
            let f = m.frame(t, "report.docx");
            assert_eq!(f.len(), 12);
            for l in &f[..11] {
                assert!(l.chars().count() <= LANE + BODY + 4, "{l}");
            }
        }
        // Mid-approach the document is visible; while chewing it is gone.
        assert!(m.frame(3, "x")[4].contains("|=- |"));
        assert!(!m.frame(9, "x").iter().any(|l| l.contains("|=- |")));
    }
}
