use anyhow::Result;

/// Parse SubRip (.srt) or WebVTT (.vtt) into timed transcript sections.
pub fn convert(bytes: &[u8], fallback: &str) -> Result<(String, String)> {
    let text = String::from_utf8_lossy(bytes);
    Ok((fallback.to_string(), transcript(&text, fallback)))
}

fn parse_time(s: &str) -> Option<u32> {
    let s = s.trim().replace(',', ".");
    let main = s.split('.').next()?;
    let parts: Vec<u32> = main
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [h, m, s] => Some(h * 3600 + m * 60 + s),
        [m, s] => Some(m * 60 + s),
        _ => None,
    }
}

fn stamp(secs: u32) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Group cues into windows of two minutes, one section each.
pub fn transcript(text: &str, title: &str) -> String {
    const WINDOW: u32 = 120;
    let lines: Vec<&str> = text.lines().collect();
    let mut cues: Vec<(u32, String)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((start, _)) = lines[i].split_once("-->")
            && let Some(t) = parse_time(start.split_whitespace().last().unwrap_or(""))
        {
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().is_empty() {
                body.push(strip_tags(lines[i]).trim().to_string());
                i += 1;
            }
            let joined = body.join(" ");
            if !joined.trim().is_empty() {
                cues.push((t, joined));
            }
            continue;
        }
        i += 1;
    }
    let mut out = format!("# {title} (transcript)\n\n");
    let mut window = u32::MAX;
    let mut para = String::new();
    for (t, line) in cues {
        let w = t / WINDOW;
        if w != window {
            if !para.is_empty() {
                out.push_str(&format!("{}\n\n", para.trim()));
                para.clear();
            }
            window = w;
            out.push_str(&format!(
                "## {} – {}\n\n",
                stamp(w * WINDOW),
                stamp((w + 1) * WINDOW)
            ));
        }
        para.push_str(&line);
        para.push(' ');
    }
    if !para.is_empty() {
        out.push_str(&format!("{}\n", para.trim()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_srt_and_vtt_into_windows() {
        let srt = "1\n00:00:01,000 --> 00:00:03,000\nHello <i>there</i>\n\n2\n00:02:05,500 --> 00:02:07,000\nSecond window\n";
        let md = transcript(srt, "clip");
        assert!(md.contains("## 0:00 – 2:00\n\nHello there"), "{md}");
        assert!(md.contains("## 2:00 – 4:00\n\nSecond window"), "{md}");
        let vtt = "WEBVTT\n\n00:01.000 --> 00:02.000\nShort form\n";
        assert!(transcript(vtt, "c").contains("Short form"));
    }
}
