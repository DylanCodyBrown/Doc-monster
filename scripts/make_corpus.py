#!/usr/bin/env python3
"""Generate big, ugly, deterministic test documents for Doc Monster.

usage: make_corpus.py OUT_DIR [--scale N]
Makes: ugly.md, huge.md, big.txt, big.docx, big.xlsx
Topic text is a fake Roblox game-development handbook so results are easy to eyeball.
"""
import argparse, os, random

TOPICS = {
 "DataStore saving": [
  "Player data is saved to a DataStore every {n} seconds and again when the player leaves.",
  "Always wrap DataStore calls in pcall because the service throttles at {n} requests per minute.",
  "Session locking prevents two servers from writing the same profile and corrupting inventory.",
  "A migration table versions every save so old profiles upgrade without data loss.",
  "Budget checks with GetRequestBudgetForRequestType avoid dropped writes during server shutdown.",
  "Never store Instances in a DataStore; store plain tables of numbers and strings.",
  "Autosave jitter spreads writes so {n} servers do not hit the limit at the same moment."],
 "Remote events and security": [
  "Every RemoteEvent handler must validate the type and range of each argument on the server.",
  "The client is never trusted: purchases, damage and teleports are decided by the server only.",
  "Rate limit each player to {n} remote calls per second and kick repeat offenders.",
  "Sanity checks compare reported speed against the maximum walk speed of {n} studs per second.",
  "Use a single RemoteFunction wrapper that logs failures so exploit attempts become visible.",
  "Deprecated BodyMovers hide speed exploits; prefer LinearVelocity with server ownership."],
 "Monetization and economy": [
  "Game passes unlock permanent perks while developer products sell repeatable boosts.",
  "A healthy economy removes about {n} percent of earned coins through sinks like upgrades and rebirths.",
  "Price anchoring places a {n} Robux bundle next to a single item so the bundle feels cheap.",
  "Never sell power that makes free players quit; sell speed, cosmetics and convenience.",
  "ProcessReceipt must return PurchaseGranted only after the grant is saved, or the purchase repeats.",
  "Limited time offers raise conversion but need a visible countdown to feel fair."],
 "Level design and pacing": [
  "The first {n} minutes must teach the core loop with no text walls.",
  "Landmarks guide players: a tall tower or glowing tree gives direction without a map.",
  "Difficulty ramps in waves: a challenge, then a rest area, then a slightly harder challenge.",
  "Checkpoints every {n} studs keep failure cheap and curiosity high.",
  "Streaming Enabled reduces memory on phones, so critical gameplay parts need persistent models.",
  "Lighting and color palettes set mood faster than any quest text."],
 "Pet and collection systems": [
  "Pets roll from egg tiers with weighted odds; the rarest pet should hit about 1 in {n}.",
  "Pity counters guarantee a rare drop after {n} hatches and soften bad luck.",
  "Equip limits create inventory decisions and drive demand for extra slots.",
  "Trading needs a two step confirmation and a server side lock to stop scams.",
  "Pet stats scale with a multiplier table stored in a ModuleScript for easy balancing."],
 "Performance and mobile": [
  "Aim for 60 frames per second on a mid range phone; test with the device emulator weekly.",
  "Pool and reuse projectiles instead of creating and destroying {n} parts per second.",
  "Heavy loops belong in task.spawn with task.wait to avoid freezing the heartbeat.",
  "Reduce draw calls by merging static meshes and sharing materials across the map.",
  "Server memory above {n} megabytes causes instance shutdowns on busy servers.",
  "Use ContentProvider PreloadAsync behind a loading screen, never silently."],
 "Live operations and analytics": [
  "Track day one, day seven and day thirty retention; below {n} percent on day one needs onboarding fixes.",
  "AnalyticsService custom events reveal where players quit in the tutorial funnel.",
  "Ship updates on a fixed weekly cadence so returning players know when to come back.",
  "A/B test thumbnails and icons; a better icon can lift visits by {n} percent.",
  "Feature flags in a DataStore let you disable a broken feature without republishing."],
}
FILLER = ["Furthermore, this has been verified across several internal playtests.",
          "Note that results vary with player count and device mix.",
          "See the appendix for implementation details.",
          "This section is subject to revision."]

def paragraph(r, topic, sentences=None):
    pool = TOPICS[topic]
    k = sentences or r.randint(3, 6)
    out = [r.choice(pool).format(n=r.choice([3,5,10,15,30,60,100,250])) for _ in range(k)]
    if r.random() < 0.5: out.append(r.choice(FILLER))
    return " ".join(out)

def ugly_markdown(r, sections, para_per=6, wrap=True):
    L = []
    page = 1
    def junk():
        nonlocal page
        L.append(""); L.append(f"Page {page} of 340"); L.append("CONFIDENTIAL - Project Aurora - Do not distribute"); L.append(""); page += 1
    topics = list(TOPICS)
    for s in range(sections):
        t = topics[s % len(topics)]
        style = r.choice(["atx", "nospace", "setext", "skip"])
        title = f"{t} part {s//len(topics)+1}"
        if style == "atx": L.append(f"## {title}")
        elif style == "nospace": L.append(f"##{title}")
        elif style == "setext": L += [title, "-" * len(title)]
        else: L.append(f"#### {title}")
        L += ["", "<!-- TODO: reviewer comment, remove before launch -->", ""]
        for p in range(para_per):
            para = paragraph(r, t)
            if wrap:
                words, line = para.split(), ""
                for w in words:
                    if len(line) + len(w) > r.choice([55, 70, 90]): L.append(line.rstrip() + r.choice(["", "  ", " "])); line = ""
                    line += w + " "
                L.append(line.rstrip())
            else: L.append(para)
            L += [""] * r.choice([1, 1, 2, 5])
            if r.random() < 0.3:
                bullet = r.choice(["*", "+", "-", "•"])
                for _ in range(r.randint(2, 4)):
                    L.append(f"{bullet}   {r.choice(TOPICS[t]).format(n=r.randint(2,99))}  ")
                L.append("")
            if r.random() < 0.15:
                L += ["| Metric | Value | Note", "|---|---|", f"| target | {r.randint(1,99)} | ok | extra", f"| limit | {r.randint(1,999)} |", ""]
            if r.random() < 0.2: L += ["<br>", "<br/>", ""]
            if r.random() < 0.08: junk()
        L += ["", "", ""]
    return "\r\n".join(L) if r.random() < 0 else "\n".join(L)

def main():
    ap = argparse.ArgumentParser(); ap.add_argument("out"); ap.add_argument("--scale", type=int, default=1)
    a = ap.parse_args(); os.makedirs(a.out, exist_ok=True); r = random.Random(42)
    def w(name, s):
        open(os.path.join(a.out, name), "w", encoding="utf-8", newline="").write(s)
    w("ugly.md", "Aurora Handbook\n===============\n\n" + ugly_markdown(r, 40 * a.scale))
    w("huge.md", "# Aurora Handbook, complete edition\n\n" + ugly_markdown(r, 600 * a.scale, para_per=8))
    topics = list(TOPICS)
    txt = []
    for i in range(1500 * a.scale):
        txt.append(f"[{i:05d}] " + paragraph(r, topics[(i // 40) % len(topics)]))
        if i % 7 == 0: txt.append("")
    w("big.txt", "\n".join(txt))

    from docx import Document
    d = Document(); d.add_heading("Aurora Operations Manual", 0)
    for s in range(150 * a.scale):
        t = topics[s % len(topics)]
        d.add_heading(f"{t} chapter {s//len(topics)+1}", 1)
        for _ in range(3): d.add_paragraph(paragraph(r, t))
        d.add_heading("Checklist", 2)
        for _ in range(4): d.add_paragraph(r.choice(TOPICS[t]).format(n=r.randint(2, 99)), style="List Bullet")
        if s % 5 == 0:
            tb = d.add_table(rows=4, cols=3); tb.style = "Table Grid"
            for i, row in enumerate(tb.rows):
                for j, c in enumerate(row.cells): c.text = ["Metric", "Target", "Owner"][j] if i == 0 else f"{t[:8]}-{i}-{j}"
    d.save(os.path.join(a.out, "big.docx"))

    from openpyxl import Workbook
    wb = Workbook(); ws = wb.active; ws.title = "Purchases"
    ws.append(["player_id", "item", "robux", "country", "day", "converted"])
    items = ["2x Coins", "VIP Pass", "Pet Slot", "Speed Boost", "Mega Egg", "Starter Pack"]
    for i in range(30000 * a.scale):
        ws.append([100000 + r.randint(0, 9999), r.choice(items), r.choice([25, 49, 99, 199, 499, 999]), r.choice(["US", "BR", "DE", "PH", "GB", "MX"]), r.randint(1, 90), r.random() < 0.18])
    ws2 = wb.create_sheet("Retention"); ws2.append(["day", "returning", "cohort"])
    for dday in (1, 7, 30):
        for c in range(200 * a.scale): ws2.append([dday, r.randint(5, 60) if dday == 1 else r.randint(1, 30), f"week-{c}"])
    wb.save(os.path.join(a.out, "big.xlsx"))

if __name__ == "__main__": main()
