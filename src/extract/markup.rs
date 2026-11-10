//! Lightweight converters from other text-document markups to Markdown.
//! They keep what the digest and a reader need (headings, lists, code, emphasis, links) and
//! drop layout-only directives.

use super::text;
use anyhow::Result;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rst,
    Adoc,
    Org,
    Tex,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Rst => "restructuredtext",
            Kind::Adoc => "asciidoc",
            Kind::Org => "org",
            Kind::Tex => "latex",
        }
    }
}

pub fn convert(kind: Kind, bytes: &[u8], fallback: &str) -> Result<(String, String)> {
    let src = text::decode(bytes).replace("\r\n", "\n");
    let md = match kind {
        Kind::Rst => rst(&src),
        Kind::Adoc => adoc(&src),
        Kind::Org => org(&src),
        Kind::Tex => tex(&src),
    };
    let title = super::first_heading(&md).unwrap_or_else(|| fallback.to_string());
    Ok((title, md))
}

fn fence(out: &mut Vec<String>, lang: &str, body: &[String]) {
    out.push(String::new());
    out.push(format!("```{lang}"));
    out.extend(body.iter().cloned());
    out.push("```".into());
    out.push(String::new());
}

fn is_adornment(l: &str) -> Option<char> {
    let t = l.trim_end();
    let c = t.chars().next()?;
    (t.len() >= 3 && "=-`:'\"~^_*+#<>".contains(c) && t.chars().all(|x| x == c)).then_some(c)
}

fn indent_of(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

fn rst_inline(s: &str) -> String {
    let role = Regex::new(r":[a-zA-Z:]+:`([^`]+)`").unwrap();
    let link = Regex::new(r"`([^`<]+?)\s*<([^>]+)>`_{1,2}").unwrap();
    // Protect inline code first so its backticks are not mistaken for links.
    let plain_ref = Regex::new(r"`([^`]+)`_{1,2}").unwrap();
    let dbl = Regex::new(r"``([^`]+)``").unwrap();
    let s = dbl.replace_all(s, "\u{1}$1\u{2}");
    let s = link.replace_all(&s, "[$1]($2)");
    let s = role.replace_all(&s, "\u{1}$1\u{2}");
    let s = plain_ref.replace_all(&s, "$1");
    s.replace('\u{1}', "`").replace('\u{2}', "`")
}

fn rst(src: &str) -> String {
    // Link text may wrap across lines, so links are converted on the whole text first.
    let wrapped_link = Regex::new(r"`([^`<]+?)\s*<([^>]+)>`_{1,2}").unwrap();
    let src = wrapped_link.replace_all(src, |c: &regex::Captures| {
        format!("[{}]({})", c[1].split_whitespace().collect::<Vec<_>>().join(" "), c[2].split_whitespace().collect::<String>())
    });
    let lines: Vec<&str> = src.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut levels: Vec<(char, bool)> = Vec::new();
    let level_of = |levels: &mut Vec<(char, bool)>, key: (char, bool)| -> usize {
        match levels.iter().position(|k| *k == key) {
            Some(p) => p + 1,
            None => {
                levels.push(key);
                levels.len()
            }
        }
    };
    let admonitions = ["note", "warning", "tip", "important", "caution", "danger", "attention", "hint", "error", "seealso"];
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        // Overlined then underlined title.
        if let (Some(c), Some(t), Some(u)) = (is_adornment(l), lines.get(i + 1), lines.get(i + 2)) {
            if !t.trim().is_empty() && is_adornment(u) == Some(c) && is_adornment(t).is_none() {
                let lvl = level_of(&mut levels, (c, true)).min(6);
                out.push(format!("{} {}", "#".repeat(lvl), t.trim()));
                i += 3;
                continue;
            }
        }
        // Underlined title.
        if !l.trim().is_empty() && is_adornment(l).is_none() && indent_of(l) == 0 {
            if let Some(c) = lines.get(i + 1).and_then(|u| is_adornment(u)) {
                if lines[i + 1].trim_end().len() >= l.trim_end().chars().count().min(3) {
                    let lvl = level_of(&mut levels, (c, false)).min(6);
                    out.push(format!("{} {}", "#".repeat(lvl), rst_inline(l.trim())));
                    i += 2;
                    continue;
                }
            }
        }
        let trimmed = l.trim_start();
        if indent_of(l) == 0 && trimmed.starts_with(".. ") {
            let directive = trimmed[3..].trim();
            let (name, arg) = directive.split_once("::").map(|(n, a)| (n.trim(), a.trim())).unwrap_or(("", ""));
            // Collect the indented body.
            let mut j = i + 1;
            let mut body: Vec<String> = Vec::new();
            while j < lines.len() && (lines[j].trim().is_empty() || indent_of(lines[j]) > 0) {
                body.push(lines[j].trim_start().to_string());
                j += 1;
            }
            while body.last().is_some_and(|b| b.is_empty()) {
                body.pop();
            }
            match name {
                "code-block" | "code" | "sourcecode" | "highlight" if name != "highlight" => {
                    let content: Vec<String> = body.iter().skip_while(|b| b.starts_with(':') || b.is_empty()).cloned().collect();
                    fence(&mut out, arg, &content);
                }
                n if admonitions.contains(&n) => {
                    out.push(String::new());
                    let text = format!("{} {}", arg, body.join(" ")).trim().to_string();
                    out.push(format!("> **{}:** {}", n[..1].to_uppercase() + &n[1..], rst_inline(&text)));
                    out.push(String::new());
                }
                _ => {} // comments, targets, toctree, images, includes: layout only
            }
            i = j;
            continue;
        }
        // Literal block introduced by a paragraph ending in `::`.
        if l.trim_end().ends_with("::") && indent_of(l) == 0 {
            let head = l.trim_end().trim_end_matches(':').trim_end();
            if !head.is_empty() {
                out.push(rst_inline(&format!("{head}:")));
            }
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            let mut body = Vec::new();
            while j < lines.len() && (lines[j].trim().is_empty() || indent_of(lines[j]) > 0) {
                body.push(lines[j].to_string());
                j += 1;
            }
            while body.last().is_some_and(|b| b.trim().is_empty()) {
                body.pop();
            }
            let min = body.iter().filter(|b| !b.trim().is_empty()).map(|b| indent_of(b)).min().unwrap_or(0);
            let body: Vec<String> = body.iter().map(|b| b.get(min..).unwrap_or("").to_string()).collect();
            fence(&mut out, "", &body);
            i = j;
            continue;
        }
        out.push(rst_inline(l));
        i += 1;
    }
    out.join("\n")
}

fn adoc(src: &str) -> String {
    let heading = Regex::new(r"^(=+)\s+(\S.*)$").unwrap();
    let ulist = Regex::new(r"^(\*+)\s+(.*)$").unwrap();
    let olist = Regex::new(r"^(\.+)\s+(.*)$").unwrap();
    let bold = Regex::new(r"\*([^\s*][^*]*?)\*").unwrap();
    let link = Regex::new(r"(?:link:)?(https?://[^\s\[]+)\[([^\]]*)\]").unwrap();
    let attribute = Regex::new(r"^:[\w-]+!?:(\s.*)?$").unwrap();
    let mut out: Vec<String> = Vec::new();
    let mut lang = String::new();
    let mut block: Option<(String, Vec<String>)> = None;
    for l in src.lines() {
        if let Some((delim, body)) = &mut block {
            if l.trim_end() == delim {
                let (_, b) = block.take().unwrap();
                fence(&mut out, &lang, &b);
                lang.clear();
            } else {
                body.push(l.to_string());
            }
            continue;
        }
        let t = l.trim_end();
        if t == "----" || t == "...." {
            block = Some((t.to_string(), Vec::new()));
            continue;
        }
        if t.starts_with("//") && !t.starts_with("///") || attribute.is_match(t) || t == "====" || t == "****" || t == "--" {
            continue;
        }
        if let Some(rest) = t.strip_prefix("[source,") {
            lang = rest.trim_end_matches(']').split(',').next().unwrap_or("").trim().to_string();
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            continue;
        }
        if t.starts_with("image::") || t.starts_with("include::") {
            continue;
        }
        let converted = if let Some(c) = heading.captures(t) {
            format!("{} {}", "#".repeat(c[1].len().min(6)), &c[2])
        } else if let Some(c) = ulist.captures(t) {
            format!("{}- {}", "  ".repeat(c[1].len() - 1), &c[2])
        } else if let Some(c) = olist.captures(t) {
            format!("{}1. {}", "  ".repeat(c[1].len() - 1), &c[2])
        } else {
            t.to_string()
        };
        let converted = bold.replace_all(&converted, "**$1**");
        let converted = link.replace_all(&converted, "[$2]($1)");
        out.push(converted.into_owned());
    }
    out.join("\n")
}

fn org(src: &str) -> String {
    let heading = Regex::new(r"^(\*+)\s+(?:(?:TODO|DONE|NEXT|WAITING)\s+)?(\S.*?)(?:\s+:[\w:@]+:)?$").unwrap();
    let title = Regex::new(r"(?i)^#\+TITLE:\s*(.*)$").unwrap();
    let begin = Regex::new(r"(?i)^#\+BEGIN_SRC\s*(\S*)").unwrap();
    let link = Regex::new(r"\[\[([^\]]+)\]\[([^\]]+)\]\]").unwrap();
    let bare = Regex::new(r"\[\[([^\]]+)\]\]").unwrap();
    let bold = Regex::new(r"(^|\s)\*([^\s*][^*]*?)\*(\s|$|[.,;:])").unwrap();
    let code = Regex::new(r"(^|\s)[=~]([^\s=~][^=~]*?)[=~](\s|$|[.,;:])").unwrap();
    let italic = Regex::new(r"(^|\s)/([^\s/][^/]*?)/(\s|$|[.,;:])").unwrap();
    let mut out: Vec<String> = Vec::new();
    let (mut src_lang, mut body, mut in_src, mut in_drawer) = (String::new(), Vec::<String>::new(), false, false);
    for l in src.lines() {
        let t = l.trim_end();
        if in_src {
            if t.to_ascii_uppercase().starts_with("#+END_SRC") {
                fence(&mut out, &src_lang, &body);
                body.clear();
                in_src = false;
            } else {
                body.push(l.to_string());
            }
            continue;
        }
        if let Some(c) = begin.captures(t) {
            src_lang = c[1].to_string();
            in_src = true;
            continue;
        }
        if let Some(c) = title.captures(t) {
            out.push(format!("# {}", c[1].trim()));
            continue;
        }
        if t.eq_ignore_ascii_case(":PROPERTIES:") || t.eq_ignore_ascii_case(":LOGBOOK:") {
            in_drawer = true;
            continue;
        }
        if in_drawer {
            if t.eq_ignore_ascii_case(":END:") {
                in_drawer = false;
            }
            continue;
        }
        if t.starts_with("#+") || t.starts_with("# ") && !t.starts_with("# #") {
            continue; // other keywords and comments
        }
        let line = if let Some(c) = heading.captures(t) {
            // Org top level is "*"; shift under a #+TITLE if one exists by simply mapping stars to hashes.
            format!("{} {}", "#".repeat(c[1].len().min(6)), &c[2])
        } else {
            t.to_string()
        };
        let line = link.replace_all(&line, "[$2]($1)");
        let line = bare.replace_all(&line, "<$1>");
        let line = bold.replace_all(&line, "$1**$2**$3");
        let line = italic.replace_all(&line, "$1*$2*$3");
        let line = code.replace_all(&line, "$1`$2`$3");
        out.push(line.into_owned());
    }
    out.join("\n")
}

fn tex(src: &str) -> String {
    // Only the document body matters; the preamble holds packages and macros.
    let body = match (src.find("\\begin{document}"), src.rfind("\\end{document}")) {
        (Some(a), Some(b)) if b > a => &src[a + "\\begin{document}".len()..b],
        (Some(a), None) => &src[a + "\\begin{document}".len()..],
        _ => src,
    };
    let title = Regex::new(r"\\title\{([^}]*)\}").unwrap().captures(src).map(|c| c[1].trim().to_string());
    let comment = Regex::new(r"(?m)(^|[^\\])%.*$").unwrap();
    let mut s = comment.replace_all(body, "$1").into_owned();

    // Code first, swapped for placeholders so later rewrites cannot damage it.
    let mut code: Vec<String> = Vec::new();
    let token = Regex::new(r"\\[A-Za-z]+Tok\{((?:[^{}\\]|\\.)*)\}").unwrap();
    let shaded = Regex::new(r"(?s)\\begin\{Shaded\}\s*\\begin\{Highlighting\}(?:\[[^\]]*\])?(.*?)\\end\{Highlighting\}\s*\\end\{Shaded\}").unwrap();
    s = shaded
        .replace_all(&s, |c: &regex::Captures| {
            let mut inner = unescape_tex(&c[1]);
            loop {
                let next = token.replace_all(&inner, "$1").into_owned();
                if next == inner {
                    break;
                }
                inner = next;
            }
            code.push(inner.trim_matches('\n').replace('\u{3}', "{").replace('\u{4}', "}"));
            format!("\n\u{5}{}\u{5}\n", code.len() - 1)
        })
        .into_owned();
    let verbatim = Regex::new(r"(?s)\\begin\{(verbatim|lstlisting|minted)\}(?:\[[^\]]*\])?(?:\{[^}]*\})?(.*?)\\end\{(?:verbatim|lstlisting|minted)\}").unwrap();
    s = verbatim
        .replace_all(&s, |c: &regex::Captures| {
            code.push(c[2].trim_matches('\n').to_string());
            format!("\n\u{5}{}\u{5}\n", code.len() - 1)
        })
        .into_owned();

    // Lists: remember whether each environment is numbered.
    let marks = Regex::new(r"\\begin\{(itemize|enumerate|description)\}|\\end\{(?:itemize|enumerate|description)\}|\\item(?:\[[^\]]*\])?\s*").unwrap();
    let mut stack: Vec<(bool, usize)> = Vec::new(); // (numbered, count)
    s = marks
        .replace_all(&s, |c: &regex::Captures| {
            let m = &c[0];
            if let Some(kind) = c.get(1) {
                stack.push((kind.as_str() == "enumerate", 0));
                "\n".to_string()
            } else if m.starts_with("\\end") {
                stack.pop();
                "\n".to_string()
            } else {
                let depth = stack.len().saturating_sub(1);
                match stack.last_mut() {
                    Some((numbered, n)) => {
                        *n += 1;
                        let bullet = if *numbered { format!("{}.", n) } else { "-".to_string() };
                        format!("\n{}{bullet} ", "  ".repeat(depth))
                    }
                    None => "\n- ".to_string(),
                }
            }
        })
        .into_owned();

    let subs: &[(&str, &str)] = &[
        (r"\\def\\[a-zA-Z@]+\{(?:[^{}]|\{[^{}]*\})*\}", ""),
        (r"\\(?:tightlist|hypertarget\{[^}]*\}|bookmark\{[^}]*\})", ""),
        (r"\\chapter\*?\{([^}]*)\}", "\n# $1\n"),
        (r"\\section\*?\{([^}]*)\}", "\n## $1\n"),
        (r"\\subsection\*?\{([^}]*)\}", "\n### $1\n"),
        (r"\\subsubsection\*?\{([^}]*)\}", "\n#### $1\n"),
        (r"\\paragraph\*?\{([^}]*)\}", "\n##### $1\n"),
        (r"\\textbf\{([^}]*)\}", "**$1**"),
        (r"\\(?:emph|textit)\{([^}]*)\}", "*$1*"),
        (r"\\(?:texttt|verb)\{([^}]*)\}", "`$1`"),
        (r"\\href\{([^}]*)\}\{([^}]*)\}", "[$2]($1)"),
        (r"\\url\{([^}]*)\}", "<$1>"),
        (r"\\cite[tp]?\{([^}]*)\}", "[$1]"),
        (r"\\(?:label|ref|eqref|index|vspace|hspace|newpage|clearpage|maketitle|tableofcontents|centering|noindent)\*?(?:\{[^}]*\})?", ""),
        (r"\\\\", "\n"),
        (r"~", " "),
        (r"``", "\u{201c}"),
        (r"''", "\u{201d}"),
    ];
    for (pat, rep) in subs {
        s = Regex::new(pat).unwrap().replace_all(&s, *rep).into_owned();
    }
    s = unescape_tex(&s);
    // Remaining commands: keep their argument text, drop the command name.
    s = Regex::new(r"\\[a-zA-Z]+\*?(\[[^\]]*\])?").unwrap().replace_all(&s, "").into_owned();
    s = s.replace(['{', '}'], "").replace('\u{3}', "{").replace('\u{4}', "}");
    // Tighten lists and collapse blank runs.
    s = Regex::new(r"\n[ \t]*\n(\s*(?:- |\d+\. ))").unwrap().replace_all(&s, "\n$1").into_owned();
    for (i, c) in code.iter().enumerate() {
        s = s.replace(&format!("\u{5}{i}\u{5}"), &format!("```\n{c}\n```"));
    }
    let mut out = String::new();
    if let Some(t) = title {
        out.push_str(&format!("# {t}\n\n"));
    }
    out.push_str(&s);
    out
}

/// Common LaTeX escapes; braces become placeholders so brace stripping leaves them alone.
fn unescape_tex(s: &str) -> String {
    s.replace("\\textquotesingle{}", "'")
        .replace("\\textbackslash{}", "\\")
        .replace("\\{", "\u{3}")
        .replace("\\}", "\u{4}")
        .replace("\\&", "&")
        .replace("\\%", "%")
        .replace("\\$", "$")
        .replace("\\#", "#")
        .replace("\\_", "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(kind: Kind, s: &str) -> String {
        convert(kind, s.as_bytes(), "f").unwrap().1
    }

    #[test]
    fn rst_headings_code_and_inline() {
        let md = c(Kind::Rst, "Title\n=====\n\nSub\n---\n\nUse ``pcall`` and :func:`wait`.\n\nExample::\n\n    local x = 1\n    print(x)\n\n.. note:: Careful here.\n\n.. code-block:: lua\n\n   return 2\n");
        assert!(md.contains("# Title") && md.contains("## Sub"), "{md}");
        assert!(md.contains("Use `pcall` and `wait`."), "{md}");
        assert!(md.contains("```\nlocal x = 1\nprint(x)\n```"), "{md}");
        assert!(md.contains("> **Note:** Careful here."));
        assert!(md.contains("```lua\nreturn 2\n```"));
    }

    #[test]
    fn rst_links_with_double_underscore() {
        let md = c(Kind::Rst, "See `the\ndocs <https://x.dev>`__ now, and ``code`` here.\n");
        assert!(md.contains("[the docs](https://x.dev)") && md.contains("`code`"), "{md}");
    }

    #[test]
    fn pandoc_latex_boilerplate_and_numbered_lists() {
        let md = c(Kind::Tex, "\\begin{document}\n\\hypertarget{intro}{%\n\\section{Intro}\\label{intro}}\n\n\\begin{enumerate}\n\\def\\labelenumi{\\arabic{enumi}.}\n\\tightlist\n\\item one\n\\item two\n\\end{enumerate}\n\n\\begin{Shaded}\n\\begin{Highlighting}[]\n\\KeywordTok{local} \\NormalTok{x }\\OperatorTok{=} \\DecValTok{1}\n\\end{Highlighting}\n\\end{Shaded}\n\\end{document}\n");
        assert!(md.contains("## Intro"), "{md}");
        assert!(md.contains("1. one\n2. two"), "{md}");
        assert!(md.contains("```\nlocal x = 1\n```"), "{md}");
        assert!(!md.contains("enumi") && !md.contains("intro}") && !md.contains("Tok"), "{md}");
    }

    #[test]
    fn asciidoc_headings_lists_code() {
        let md = c(Kind::Adoc, "= Guide\n:author: me\n\n== Setup\n\n* one\n** nested\n\n[source,lua]\n----\nprint(1)\n----\n\nSee https://x.dev[the site] and *bold*.\n");
        assert!(md.contains("# Guide") && md.contains("## Setup") && !md.contains(":author:"), "{md}");
        assert!(md.contains("- one\n  - nested") && md.contains("```lua\nprint(1)\n```"), "{md}");
        assert!(md.contains("[the site](https://x.dev)") && md.contains("**bold**"));
    }

    #[test]
    fn org_headings_code_links() {
        let md = c(Kind::Org, "#+TITLE: Notes\n* TODO Plan :work:\n:PROPERTIES:\n:ID: 1\n:END:\nSee [[https://a.b][site]] with =code= and *bold*.\n#+BEGIN_SRC lua\nprint(1)\n#+END_SRC\n");
        assert!(md.contains("# Notes") && md.contains("# Plan"), "{md}");
        assert!(!md.contains("PROPERTIES"));
        assert!(md.contains("[site](https://a.b)") && md.contains("`code`") && md.contains("**bold**"));
        assert!(md.contains("```lua\nprint(1)\n```"));
    }

    #[test]
    fn latex_body_headings_lists() {
        let md = c(Kind::Tex, "\\documentclass{article}\n\\title{Report}\n\\begin{document}\n\\maketitle\n\\section{Intro}\nSome \\textbf{bold} and \\emph{soft} text. % hidden\n\\begin{itemize}\n\\item First\n\\item Second\n\\end{itemize}\n\\end{document}\n");
        assert!(md.starts_with("# Report"), "{md}");
        assert!(md.contains("## Intro") && md.contains("**bold**") && md.contains("*soft*"), "{md}");
        assert!(md.contains("- First") && md.contains("- Second") && !md.contains("hidden") && !md.contains("documentclass"));
    }
}
