//! Generate big, ugly, deterministic test documents for Doc Monster.
//!
//!     cargo run --release --example make_corpus -- OUT_DIR [--scale N]
//!
//! Makes: ugly.md, huge.md, big.txt, big.docx, big.xlsx. The text is a fake Roblox
//! game-development handbook so results are easy to eyeball. Same output on every run.

use docx_rs::{Docx, Paragraph, Run, Table, TableCell, TableRow};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const TOPICS: &[(&str, &[&str])] = &[
    (
        "DataStore saving",
        &[
            "Player data is saved to a DataStore every {n} seconds and again when the player leaves.",
            "Always wrap DataStore calls in pcall because the service throttles at {n} requests per minute.",
            "Session locking prevents two servers from writing the same profile and corrupting inventory.",
            "A migration table versions every save so old profiles upgrade without data loss.",
            "Budget checks with GetRequestBudgetForRequestType avoid dropped writes during server shutdown.",
            "Never store Instances in a DataStore; store plain tables of numbers and strings.",
            "Autosave jitter spreads writes so {n} servers do not hit the limit at the same moment.",
        ],
    ),
    (
        "Remote events and security",
        &[
            "Every RemoteEvent handler must validate the type and range of each argument on the server.",
            "The client is never trusted: purchases, damage and teleports are decided by the server only.",
            "Rate limit each player to {n} remote calls per second and kick repeat offenders.",
            "Sanity checks compare reported speed against the maximum walk speed of {n} studs per second.",
            "Use a single RemoteFunction wrapper that logs failures so exploit attempts become visible.",
            "Deprecated BodyMovers hide speed exploits; prefer LinearVelocity with server ownership.",
        ],
    ),
    (
        "Monetization and economy",
        &[
            "Game passes unlock permanent perks while developer products sell repeatable boosts.",
            "A healthy economy removes about {n} percent of earned coins through sinks like upgrades and rebirths.",
            "Price anchoring places a {n} Robux bundle next to a single item so the bundle feels cheap.",
            "Never sell power that makes free players quit; sell speed, cosmetics and convenience.",
            "ProcessReceipt must return PurchaseGranted only after the grant is saved, or the purchase repeats.",
            "Limited time offers raise conversion but need a visible countdown to feel fair.",
        ],
    ),
    (
        "Level design and pacing",
        &[
            "The first {n} minutes must teach the core loop with no text walls.",
            "Landmarks guide players: a tall tower or glowing tree gives direction without a map.",
            "Difficulty ramps in waves: a challenge, then a rest area, then a slightly harder challenge.",
            "Checkpoints every {n} studs keep failure cheap and curiosity high.",
            "Streaming Enabled reduces memory on phones, so critical gameplay parts need persistent models.",
            "Lighting and color palettes set mood faster than any quest text.",
        ],
    ),
    (
        "Pet and collection systems",
        &[
            "Pets roll from egg tiers with weighted odds; the rarest pet should hit about 1 in {n}.",
            "Pity counters guarantee a rare drop after {n} hatches and soften bad luck.",
            "Equip limits create inventory decisions and drive demand for extra slots.",
            "Trading needs a two step confirmation and a server side lock to stop scams.",
            "Pet stats scale with a multiplier table stored in a ModuleScript for easy balancing.",
        ],
    ),
    (
        "Performance and mobile",
        &[
            "Aim for 60 frames per second on a mid range phone; test with the device emulator weekly.",
            "Pool and reuse projectiles instead of creating and destroying {n} parts per second.",
            "Heavy loops belong in task.spawn with task.wait to avoid freezing the heartbeat.",
            "Reduce draw calls by merging static meshes and sharing materials across the map.",
            "Server memory above {n} megabytes causes instance shutdowns on busy servers.",
            "Use ContentProvider PreloadAsync behind a loading screen, never silently.",
        ],
    ),
    (
        "Live operations and analytics",
        &[
            "Track day one, day seven and day thirty retention; below {n} percent on day one needs onboarding fixes.",
            "AnalyticsService custom events reveal where players quit in the tutorial funnel.",
            "Ship updates on a fixed weekly cadence so returning players know when to come back.",
            "A/B test thumbnails and icons; a better icon can lift visits by {n} percent.",
            "Feature flags in a DataStore let you disable a broken feature without republishing.",
        ],
    ),
];
const FILLER: &[&str] = &[
    "Furthermore, this has been verified across several internal playtests.",
    "Note that results vary with player count and device mix.",
    "See the appendix for implementation details.",
    "This section is subject to revision.",
];
const NUMBERS: &[u32] = &[3, 5, 10, 15, 30, 60, 100, 250];

/// Small deterministic generator (xorshift), so every run makes the same files.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next() % u64::from(hi - lo + 1)) as u32
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() % 10_000) as f64 / 10_000.0 < p
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn sentence(r: &mut Rng, topic: usize) -> String {
    let n = *r.pick(NUMBERS);
    r.pick(TOPICS[topic].1).replace("{n}", &n.to_string())
}

fn paragraph(r: &mut Rng, topic: usize) -> String {
    let k = r.range(3, 6);
    let mut out: Vec<String> = (0..k).map(|_| sentence(r, topic)).collect();
    if r.chance(0.5) {
        out.push(r.pick(FILLER).to_string());
    }
    out.join(" ")
}

/// Every kind of mess the cleaner is meant to repair.
fn ugly_markdown(r: &mut Rng, sections: usize, paras: usize) -> String {
    let mut l: Vec<String> = Vec::new();
    let mut page = 1;
    for s in 0..sections {
        let t = s % TOPICS.len();
        let title = format!("{} part {}", TOPICS[t].0, s / TOPICS.len() + 1);
        match r.below(4) {
            0 => l.push(format!("## {title}")),
            1 => l.push(format!("##{title}")),
            2 => l.extend([title.clone(), "-".repeat(title.len())]),
            _ => l.push(format!("#### {title}")),
        }
        l.extend([
            "".into(),
            "<!-- TODO: reviewer comment, remove before launch -->".into(),
            "".into(),
        ]);
        for _ in 0..paras {
            // Hard-wrap at random widths with stray trailing spaces.
            let mut line = String::new();
            for w in paragraph(r, t).split(' ') {
                if line.len() + w.len() > *r.pick(&[55, 70, 90]) {
                    l.push(format!("{}{}", line.trim_end(), r.pick(&["", "  ", " "])));
                    line.clear();
                }
                line.push_str(w);
                line.push(' ');
            }
            l.push(line.trim_end().to_string());
            l.extend(std::iter::repeat_n(String::new(), *r.pick(&[1, 1, 2, 5])));
            if r.chance(0.3) {
                let bullet = *r.pick(&["*", "+", "-", "•"]);
                for _ in 0..r.range(2, 4) {
                    let n = r.range(2, 99);
                    l.push(format!(
                        "{bullet}   {}\u{a0} ",
                        r.pick(TOPICS[t].1).replace("{n}", &n.to_string())
                    ));
                }
                l.push(String::new());
            }
            if r.chance(0.15) {
                let (a, b) = (r.range(1, 99), r.range(1, 999));
                l.extend([
                    "| Metric | Value | Note".into(),
                    "|---|---|".into(),
                    format!("| target | {a} | ok | extra"),
                    format!("| limit | {b} |"),
                    "".into(),
                ]);
            }
            if r.chance(0.2) {
                l.extend(["<br>".into(), "<br/>".into(), "".into()]);
            }
            if r.chance(0.08) {
                l.extend([
                    "".into(),
                    format!("Page {page} of 340"),
                    "CONFIDENTIAL - Project Aurora - Do not distribute".into(),
                    "".into(),
                ]);
                page += 1;
            }
        }
        l.extend(["".into(), "".into(), "".into()]);
    }
    l.join("\n")
}

fn write(dir: &Path, name: &str, text: &str) -> std::io::Result<()> {
    std::fs::write(dir.join(name), text)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    let mut scale = 1usize;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--scale" => scale = args.next().ok_or("--scale needs a number")?.parse()?,
            _ => out = Some(PathBuf::from(a)),
        }
    }
    let out = out.ok_or("usage: make_corpus OUT_DIR [--scale N]")?;
    std::fs::create_dir_all(&out)?;
    let mut r = Rng(42);

    write(
        &out,
        "ugly.md",
        &format!(
            "Aurora Handbook\n===============\n\n{}",
            ugly_markdown(&mut r, 40 * scale, 6)
        ),
    )?;
    write(
        &out,
        "huge.md",
        &format!(
            "# Aurora Handbook, complete edition\n\n{}",
            ugly_markdown(&mut r, 600 * scale, 8)
        ),
    )?;

    let mut txt = String::new();
    for i in 0..1500 * scale {
        let _ = writeln!(
            txt,
            "[{i:05}] {}",
            paragraph(&mut r, (i / 40) % TOPICS.len())
        );
        if i % 7 == 0 {
            txt.push('\n');
        }
    }
    write(&out, "big.txt", &txt)?;

    // Word document: headings, bullets, tables.
    let mut doc = Docx::new().add_paragraph(
        Paragraph::new()
            .style("Title")
            .add_run(Run::new().add_text("Aurora Operations Manual")),
    );
    for s in 0..150 * scale {
        let t = s % TOPICS.len();
        doc = doc.add_paragraph(Paragraph::new().style("Heading1").add_run(
            Run::new().add_text(format!("{} chapter {}", TOPICS[t].0, s / TOPICS.len() + 1)),
        ));
        for _ in 0..3 {
            doc = doc
                .add_paragraph(Paragraph::new().add_run(Run::new().add_text(paragraph(&mut r, t))));
        }
        doc = doc.add_paragraph(
            Paragraph::new()
                .style("Heading2")
                .add_run(Run::new().add_text("Checklist")),
        );
        for _ in 0..4 {
            let n = r.range(2, 99);
            let text = r.pick(TOPICS[t].1).replace("{n}", &n.to_string());
            doc = doc.add_paragraph(
                Paragraph::new()
                    .style("ListBullet")
                    .add_run(Run::new().add_text(text)),
            );
        }
        if s % 5 == 0 {
            let rows = (0..4)
                .map(|i| {
                    TableRow::new(
                        (0..3)
                            .map(|j| {
                                let text = if i == 0 {
                                    ["Metric", "Target", "Owner"][j].to_string()
                                } else {
                                    format!("{}-{i}-{j}", &TOPICS[t].0[..8])
                                };
                                TableCell::new().add_paragraph(
                                    Paragraph::new().add_run(Run::new().add_text(text)),
                                )
                            })
                            .collect(),
                    )
                })
                .collect();
            doc = doc.add_table(Table::new(rows));
        }
    }
    doc.build()
        .pack(std::fs::File::create(out.join("big.docx"))?)?;

    // Workbook: a big purchases sheet (over the 5,000-row cap), dates, and a hidden sheet.
    let mut wb = Workbook::new();
    let date = Format::new().set_num_format("yyyy-mm-dd");
    let items = [
        "2x Coins",
        "VIP Pass",
        "Pet Slot",
        "Speed Boost",
        "Mega Egg",
        "Starter Pack",
    ];
    let countries = ["US", "BR", "DE", "PH", "GB", "MX"];
    {
        let ws = wb.add_worksheet().set_name("Purchases")?;
        for (c, h) in ["player_id", "item", "robux", "country", "date", "converted"]
            .iter()
            .enumerate()
        {
            ws.write(0, c as u16, *h)?;
        }
        for i in 1..=(30_000 * scale) as u32 {
            ws.write(i, 0, f64::from(100_000 + r.range(0, 9999)))?;
            ws.write(i, 1, *r.pick(&items))?;
            ws.write(i, 2, f64::from(*r.pick(&[25u32, 49, 99, 199, 499, 999])))?;
            ws.write(i, 3, *r.pick(&countries))?;
            let day = ExcelDateTime::from_ymd(2026, 8, 1)?.to_excel() + f64::from(r.range(0, 89));
            ws.write_with_format(i, 4, day, &date)?;
            ws.write(i, 5, r.chance(0.18))?;
        }
    }
    {
        let ws = wb.add_worksheet().set_name("Retention")?;
        for (c, h) in ["day", "returning", "cohort"].iter().enumerate() {
            ws.write(0, c as u16, *h)?;
        }
        let mut row = 1u32;
        for d in [1u32, 7, 30] {
            for c in 0..200 * scale {
                ws.write(row, 0, f64::from(d))?;
                ws.write(
                    row,
                    1,
                    f64::from(if d == 1 {
                        r.range(5, 60)
                    } else {
                        r.range(1, 30)
                    }),
                )?;
                ws.write(row, 2, format!("week-{c}"))?;
                row += 1;
            }
        }
    }
    {
        let ws = wb.add_worksheet().set_name("Internal")?;
        ws.write(0, 0, "note")?;
        ws.write(1, 0, "Prices under review, do not ship")?;
        ws.set_hidden(true);
    }
    wb.save(out.join("big.xlsx"))?;

    println!(
        "wrote ugly.md, huge.md, big.txt, big.docx, big.xlsx to {}",
        out.display()
    );
    Ok(())
}
