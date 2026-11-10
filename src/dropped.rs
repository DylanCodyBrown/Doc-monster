//! Paths that were dragged and dropped onto a terminal.
//!
//! Each terminal writes a dropped file differently:
//! - macOS Terminal and iTerm: `/Users/me/My\ Report.docx` (backslash before spaces)
//! - GNOME Terminal and many Linux terminals: `'/home/me/My Report.docx'`
//! - Windows Terminal, PowerShell, Command Prompt: `"C:\Users\me\My Report.docx"`,
//!   sometimes with a leading `& ` in PowerShell
//! - some Linux terminals: `file:///home/me/My%20Report.docx`
//!
//! Several files dropped at once arrive on one line separated by spaces.

use std::path::{Path, PathBuf};

/// Characters a shell-style backslash can escape. A backslash before anything else is kept,
/// so Windows paths like `C:\Users\me` survive.
const ESCAPABLE: &str = " \t'\"()[]{}&;!$#*?|<>`~=";

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_uri(token: &str) -> String {
    match token.strip_prefix("file://") {
        // file:///home/me -> /home/me ; file://localhost/home/me -> /home/me ; file:///C:/x -> C:/x
        Some(rest) => {
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            let decoded = percent_decode(rest);
            let b = decoded.as_bytes();
            if b.len() > 3 && b[0] == b'/' && b[2] == b':' {
                decoded[1..].to_string()
            } else {
                decoded
            }
        }
        None => token.to_string(),
    }
}

/// Split one dropped line into tokens, honouring quotes and backslash escapes.
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                has = true;
                while let Some(q) = chars.next() {
                    if q == c {
                        // PowerShell writes a literal ' inside '...' as ''.
                        if c == '\'' && chars.peek() == Some(&'\'') {
                            chars.next();
                            cur.push('\'');
                            continue;
                        }
                        break;
                    }
                    cur.push(q);
                }
            }
            '\\' if chars.peek().is_some_and(|n| ESCAPABLE.contains(*n)) => {
                has = true;
                cur.push(chars.next().unwrap());
            }
            c if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                has = true;
                cur.push(c);
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// The paths in one line typed or dropped by the user.
pub fn parse(line: &str) -> Vec<PathBuf> {
    let line = line.trim();
    // PowerShell sometimes prefixes a dropped path with the call operator.
    let line = line.strip_prefix("& ").unwrap_or(line).trim();
    if line.is_empty() {
        return Vec::new();
    }
    // A single unquoted path with spaces (typed or pasted) is still one path.
    if Path::new(line).exists() {
        return vec![PathBuf::from(line)];
    }
    tokens(line)
        .into_iter()
        .map(|t| PathBuf::from(from_uri(&t)))
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: &str) -> Vec<String> {
        parse(line)
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn macos_terminal_escapes_spaces() {
        assert_eq!(
            p(r"/Users/me/My\ Report\ (final).docx"),
            vec!["/Users/me/My Report (final).docx"]
        );
        assert_eq!(
            p(r"/a/one\ file.pdf /b/two.xlsx"),
            vec!["/a/one file.pdf", "/b/two.xlsx"]
        );
    }

    #[test]
    fn linux_terminals_single_quote_or_use_file_uris() {
        assert_eq!(
            p("'/home/me/My Report.docx' '/home/me/b.md'"),
            vec!["/home/me/My Report.docx", "/home/me/b.md"]
        );
        assert_eq!(
            p("file:///home/me/My%20Report.docx"),
            vec!["/home/me/My Report.docx"]
        );
    }

    #[test]
    fn windows_terminals_double_quote_and_keep_backslashes() {
        assert_eq!(
            p(r#""C:\Users\me\My Report.docx" C:\data\b.xlsx"#),
            vec![r"C:\Users\me\My Report.docx", r"C:\data\b.xlsx"]
        );
        assert_eq!(
            p(r"& 'C:\Users\me\it''s here.docx'"),
            vec![r"C:\Users\me\it's here.docx"]
        );
        assert_eq!(
            p("file:///C:/Users/me/a%20b.docx"),
            vec!["C:/Users/me/a b.docx"]
        );
    }

    #[test]
    fn an_existing_path_with_spaces_is_one_path() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("has space.txt");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(parse(&format!("  {}  ", f.display())), vec![f]);
    }

    #[test]
    fn blank_lines_have_no_paths() {
        assert!(parse("   ").is_empty());
    }
}
