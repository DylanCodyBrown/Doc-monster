//! The animated monster: a greedy green slime ghost that gulps documents while they are
//! converted. It hovers (bobs up and down), drips slime, and chews with its mouth wide open.
//!
//! Drawn on standard error, and only when standard error is an interactive terminal, so
//! scripts, pipes and agents get clean output automatically.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(80);
const LANE: usize = 22; // width of the space the document slides across
const BODY: usize = 31; // width of the ghost

#[derive(Clone, Copy, PartialEq)]
enum Mouth {
    Wide,
    Half,
    Shut,
    Full,
}

/// The ghost, 11 rows. Each row is written as just its shape and centred in `BODY` columns, so
/// the side walls of the face always line up whatever the mouth is doing.
fn body(mouth: Mouth, happy: bool, drip: usize) -> [String; 11] {
    let (left, right) = if happy {
        ("(^)", "(^)")
    } else {
        ("(o)", "(O)")
    };
    let m: [&str; 3] = match mouth {
        Mouth::Wide => [
            r"  .'^^^^^^^^^^^^^'.  ",
            r"  (    G U L P    )  ",
            r"  '.vvvvvvvvvvvvv.'  ",
        ],
        Mouth::Half => [
            r"   .-'''''''''''-.   ",
            r"  (_______________)  ",
            r"   '-.vvvvvvvvv.-'   ",
        ],
        Mouth::Shut => [
            r"                     ",
            r"   \_____________/   ",
            r"                     ",
        ],
        Mouth::Full => [
            r"                     ",
            r"   \_____________/   ",
            r"       *burp*        ",
        ],
    };
    // Slime dripping off the tail, three positions that cycle.
    let drips = [
        r"  '    :     '  :  ",
        r"  :    '     :  '  ",
        r"  .    :     .  :  ",
    ][drip % 3];
    let rows: [String; 11] = [
        ".-~~~~~~~~~-.".into(),
        ".'             '.".into(),
        format!(r"/  {left}         {right}  \"),
        "|                     |".into(),
        format!("|{}|", m[0]),
        format!("|{}|", m[1]),
        format!("|{}|", m[2]),
        "o=|                     |=o".into(),
        r"\   ~   ~   ~   ~   /".into(),
        "'~._  _.~~~._  _.~'".into(),
        drips.into(),
    ];
    rows.map(|r| format!("{r:^BODY$}"))
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

    /// One frame: the document slides toward the open mouth, then the ghost chews, all while
    /// hovering up and down and dripping slime.
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
        let splats = step >= APPROACH && (step - APPROACH).is_multiple_of(2);
        // Hover: the ghost sits one row lower every other pair of frames.
        let bob = (t / 2) % 2;
        let mut rows: Vec<String> = Vec::with_capacity(12);
        if bob == 1 {
            rows.push(" ".repeat(BODY));
        }
        rows.extend(body(mouth, false, t));
        if bob == 0 {
            rows.push(" ".repeat(BODY));
        }
        let mut lines = Vec::with_capacity(13);
        for (row, b) in rows.iter().enumerate() {
            let ghost_row = row.wrapping_sub(bob);
            let mut lane = " ".repeat(LANE);
            if let Some(x) = doc_x
                && (3..7).contains(&ghost_row)
            {
                lane.replace_range(x..x + DOC[0].len(), DOC[ghost_row - 3]);
            }
            let lane = self.paint(&lane, "33");
            let mut b = b.clone();
            if splats && ghost_row == 6 {
                b.push_str(" ~*.");
            }
            lines.push(format!("{lane}{}", self.paint(&b, "32")));
        }
        let shown: String = name.chars().take(LANE + BODY - 14).collect();
        lines.push(format!(
            "  {} {shown}",
            self.paint(["gulp", "gulp slurp", "gulp slurp munch"][t / 3 % 3], "1")
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

    /// The hungry ghost, shown when waiting for files to be dropped in.
    pub fn hello(&self) {
        if !self.enabled {
            return;
        }
        let mut out = String::new();
        for b in body(Mouth::Wide, false, 0) {
            out.push_str(&self.paint(&b, "32"));
            out.push('\n');
        }
        out.push_str(&format!("{}\n", self.paint("   Feed me documents!", "1")));
        let _ = std::io::stderr().write_all(out.as_bytes());
    }

    /// The satisfied ghost, shown once at the end.
    pub fn burp(&self, eaten: usize) {
        if !self.enabled || eaten == 0 {
            return;
        }
        let mut out = String::new();
        for b in body(Mouth::Full, true, 2) {
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
    fn the_face_walls_line_up_for_every_mouth() {
        for mouth in [Mouth::Wide, Mouth::Half, Mouth::Shut, Mouth::Full] {
            let rows = body(mouth, false, 0);
            for (r, row) in rows.iter().enumerate().take(8).skip(3) {
                let chars: Vec<char> = row.chars().collect();
                assert_eq!(chars.len(), BODY, "row {r}: {row:?}");
                assert_eq!((chars[4], chars[26]), ('|', '|'), "row {r} walls: {row:?}");
            }
        }
    }

    #[test]
    fn frames_are_rectangular_bob_and_the_document_reaches_the_mouth() {
        let m = Monster {
            enabled: true,
            color: false,
        };
        for t in 0..30 {
            let f = m.frame(t, "report.docx");
            assert_eq!(
                f.len(),
                13,
                "12 rows of ghost and hover space, then the caption"
            );
            for l in &f[..12] {
                assert!(l.chars().count() <= LANE + BODY + 4, "{l}");
            }
        }
        // It hovers: the top of the head moves between frames.
        let top = |t| {
            m.frame(t, "x")
                .iter()
                .position(|l| l.contains(".-~~~~~~~~~-."))
                .unwrap()
        };
        assert_ne!(top(0), top(2));
        // Mid-approach the document is visible; while chewing it is gone.
        assert!(m.frame(3, "x").iter().any(|l| l.contains("|=- |")));
        assert!(!m.frame(9, "x").iter().any(|l| l.contains("|=- |")));
    }
}
