mod csv_table;
mod data;
mod doc;
mod docx;
mod html;
mod markup;
mod odf;
mod pdf;
mod rtf;
mod subtitles;
mod text;
mod video;
mod xlsx;

use crate::Document;
use anyhow::{Context, Result, bail};
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Markdown,
    PlainText,
    /// reStructuredText, AsciiDoc, Org mode, LaTeX
    Markup(markup::Kind),
    Html,
    Csv,
    /// JSON, JSON with comments, JSON Lines, YAML, TOML, XML, INI-style settings
    Data(data::Kind),
    /// Word: .docx .docm .dotx .dotm
    Docx,
    /// OpenDocument text and presentations (.odt .ott .odp .fodt)
    Odf,
    /// Rich Text Format
    Rtf,
    /// Legacy binary Word (.doc)
    LegacyDoc,
    Pdf,
    Xlsx,
    Video,
    Subtitles,
    /// Source code and logs. Wrapped in a code fence so they are never reflowed.
    Code,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Markdown => "markdown",
            Format::PlainText => "text",
            Format::Markup(k) => k.label(),
            Format::Html => "html",
            Format::Csv => "csv",
            Format::Data(k) => k.label(),
            Format::Docx => "word",
            Format::Odf => "opendocument",
            Format::Rtf => "rtf",
            Format::LegacyDoc => "word (legacy)",
            Format::Pdf => "pdf",
            Format::Xlsx => "spreadsheet",
            Format::Video => "video",
            Format::Subtitles => "subtitles",
            Format::Code => "code",
        }
    }
}

/// What a converter produces.
pub struct Out {
    pub title: String,
    pub markdown: String,
    /// A description taken from the file's own metadata, when it has one.
    pub description: Option<String>,
    /// A summary the reader produced itself (e.g. the shape of a data file); replaces the digest.
    pub summary: Option<String>,
}

impl Out {
    pub fn new(title: impl Into<String>, markdown: impl Into<String>) -> Out {
        Out { title: title.into(), markdown: markdown.into(), description: None, summary: None }
    }
    pub fn with_summary(mut self, s: String) -> Out {
        self.summary = Some(s);
        self
    }
    pub fn with_description(mut self, d: Option<String>) -> Out {
        self.description = d.map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
        self
    }
}

impl From<(String, String)> for Out {
    fn from((title, markdown): (String, String)) -> Out {
        Out::new(title, markdown)
    }
}

const CODE: &[&str] = &[
    "rs", "py", "pyw", "js", "mjs", "cjs", "ts", "tsx", "jsx", "lua", "luau", "c", "h", "cc", "cpp", "cxx", "hpp",
    "cs", "java", "kt", "kts", "go", "rb", "php", "swift", "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "sql",
    "css", "scss", "sass", "less", "vue", "svelte", "dart", "r", "pl", "pm", "scala", "clj", "ex", "exs", "erl",
    "hs", "ml", "fs", "zig", "nim", "gradle", "cmake", "mk", "proto", "graphql", "gql", "glsl", "hlsl", "wgsl",
    "asm", "s", "vb", "vbs", "m", "mm", "tf", "hcl", "log", "out", "err", "diff", "patch", "gitignore", "lock",
];
const PLAIN: &[&str] = &["txt", "text", "nfo", "diz", "me", "1st", "asc", "readme", "wiki", "mediawiki", "textile", "creole"];
const PLAIN_NAMES: &[&str] = &[
    "readme", "license", "licence", "changelog", "changes", "authors", "contributors", "notice", "todo", "copying",
    "install", "news", "notes", "history", "credits", "thanks", "version",
];

/// Decide the format from the extension, falling back to content sniffing.
pub fn detect(path: &Path, head: &[u8]) -> Result<Format> {
    use data::Kind as D;
    use markup::Kind as M;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let by_ext = match ext.as_str() {
        "md" | "markdown" | "mdown" | "mkd" | "mdx" | "qmd" | "rmd" => Some(Format::Markdown),
        "rst" | "rest" => Some(Format::Markup(M::Rst)),
        "adoc" | "asciidoc" => Some(Format::Markup(M::Adoc)),
        "org" => Some(Format::Markup(M::Org)),
        "tex" | "latex" | "ltx" => Some(Format::Markup(M::Tex)),
        "html" | "htm" | "xhtml" | "mhtml" => Some(Format::Html),
        "csv" | "tsv" | "psv" => Some(Format::Csv),
        "json" | "jsonc" | "json5" | "geojson" | "webmanifest" | "har" | "mcmeta" => Some(Format::Data(D::Json)),
        "jsonl" | "ndjson" => Some(Format::Data(D::JsonLines)),
        "yaml" | "yml" => Some(Format::Data(D::Yaml)),
        "toml" => Some(Format::Data(D::Toml)),
        "xml" | "xsd" | "xsl" | "xslt" | "rss" | "atom" | "svg" | "plist" | "kml" | "gpx" | "rdf" | "opml" | "rbxlx"
        | "rbxmx" | "csproj" | "resx" | "xaml" => Some(Format::Data(D::Xml)),
        "ini" | "cfg" | "conf" | "properties" | "env" | "editorconfig" | "gitconfig" | "desktop" => Some(Format::Data(D::Ini)),
        "docx" | "docm" | "dotx" | "dotm" => Some(Format::Docx),
        "odt" | "ott" | "odp" | "otp" | "fodt" | "fodp" => Some(Format::Odf),
        "rtf" => Some(Format::Rtf),
        "doc" | "dot" => Some(Format::LegacyDoc),
        "pdf" => Some(Format::Pdf),
        "xlsx" | "xlsm" | "xls" | "xlsb" | "ods" | "xltx" => Some(Format::Xlsx),
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "flv" | "mpg" | "mpeg" | "ogv" | "3gp" | "m2ts"
        | "mts" | "vob" | "qt" => Some(Format::Video),
        "srt" | "vtt" => Some(Format::Subtitles),
        e if PLAIN.contains(&e) => Some(Format::PlainText),
        e if CODE.contains(&e) => Some(Format::Code),
        _ => None,
    };
    if let Some(f) = by_ext {
        return Ok(f);
    }
    if matches!(name.as_str(), ".env" | ".gitconfig" | ".editorconfig") || name.starts_with(".env.") {
        return Ok(Format::Data(D::Ini));
    }
    if ext.is_empty() && PLAIN_NAMES.contains(&name.as_str()) {
        return Ok(Format::PlainText);
    }
    if head.starts_with(b"%PDF-") {
        return Ok(Format::Pdf);
    }
    if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Ok(Format::LegacyDoc);
    }
    if head.starts_with(b"{\\rtf") {
        return Ok(Format::Rtf);
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return Ok(Format::Video);
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Ok(Format::Video);
    }
    if head.starts_with(b"PK\x03\x04") {
        bail!("zip-based file with unsupported extension '.{ext}'");
    }
    if text::looks_like_text(head) {
        return Ok(Format::Code);
    }
    bail!("unrecognized binary format '.{ext}'")
}

fn read_head(path: &Path) -> Result<Vec<u8>> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut buf = vec![0u8; 8192];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Read a file and convert it to full-fidelity Markdown.
pub fn extract(path: &Path) -> Result<Document> {
    let head = read_head(path)?;
    let format = detect(path, &head)?;
    let fallback = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .or_else(|| path.file_name().and_then(|s| s.to_str()))
        .unwrap_or("untitled")
        .to_string();

    // Videos can be gigabytes: never read them whole.
    let out: Out = if format == Format::Video {
        video::convert(path, &fallback)?
    } else {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        match format {
            Format::Markdown => text::convert_markdown(&bytes, &fallback)?,
            Format::PlainText => text::convert(&bytes, &fallback)?.into(),
            Format::Code => text::convert_code(&bytes, path, &fallback)?.into(),
            Format::Markup(k) => markup::convert(k, &bytes, &fallback)?.into(),
            Format::Html => html::convert(&bytes, &fallback)?.into(),
            Format::Csv => csv_table::convert(&bytes, path, &fallback)?.into(),
            Format::Data(k) => data::convert(k, &bytes, path, &fallback)?,
            Format::Docx => docx::convert(&bytes, &fallback)?,
            Format::Odf => odf::convert(&bytes, &fallback)?,
            Format::Rtf => rtf::convert(&bytes, &fallback)?.into(),
            Format::LegacyDoc => doc::convert(&bytes, &fallback)?.into(),
            Format::Pdf => pdf::convert(&bytes, &fallback)?.into(),
            Format::Xlsx => xlsx::convert(&bytes, &fallback)?.into(),
            Format::Subtitles => subtitles::convert(&bytes, &fallback)?.into(),
            Format::Video => unreachable!(),
        }
    };

    Ok(Document {
        title: out.title,
        description: out.description,
        summary: out.summary,
        source: path.to_path_buf(),
        format,
        markdown: out.markdown,
    })
}

/// First level-one heading in Markdown, if any.
pub(crate) fn first_heading(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find_map(|l| l.strip_prefix("# ").map(|t| t.trim().to_string()))
        .filter(|t| !t.is_empty())
}

/// Text of the first element with this local name in an XML snippet (for document properties).
pub(crate) fn xml_field(xml: &str, local: &str) -> Option<String> {
    use quick_xml::{Reader, events::Event};
    let mut r = Reader::from_str(xml);
    let mut inside = false;
    let mut out = String::new();
    loop {
        match r.read_event().ok()? {
            Event::Start(e) if e.local_name().as_ref() == local => inside = true,
            Event::Text(t) if inside => out.push_str(&t),
            Event::End(e) if e.local_name().as_ref() == local => {
                let v = out.trim().to_string();
                return (!v.is_empty()).then_some(v);
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}
