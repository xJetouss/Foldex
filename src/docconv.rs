//! Dokumente umwandeln: TXT, MD, HTML, DOCX, ODT, RTF, EPUB, CSV, JSON, PDF (und PAGES/DOC lesen).
//! Eigene Leser/Schreiber über ein einfaches Zwischenformat (Überschriften, Absätze, Listen, Code, Tabellen).
//! Sind LibreOffice oder pandoc installiert, übernehmen sie Paare, die sie besser können (z. B. DOC, DOCX → PDF
//! mit Layout); pdftotext (Poppler) macht PDFs als Text lesbar.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    H(u8, String),
    P(String),
    Li(String),
    Code(String),
    Table(Vec<Vec<String>>),
}

const OWN_READ: &[&str] = &["txt", "md", "markdown", "html", "htm", "docx", "odt", "rtf", "epub", "csv", "json", "xlsx"];
const OWN_WRITE: &[&str] = &["txt", "md", "html", "docx", "odt", "rtf", "pdf", "epub", "csv", "json", "xlsx"];
/// Tabellen und Präsentationen, die nur LibreOffice lesen bzw. schreiben kann
const SO_SHEETS: &[&str] = &["xls", "ods"];
const SO_SLIDES: &[&str] = &["pptx", "ppt", "odp"];
/// Tabellenartige Formate (Zeilen und Spalten)
const TABLES: &[&str] = &["csv", "json", "xlsx"];
const SOFFICE_READ: &[&str] = &["doc", "docx", "odt", "rtf", "html", "htm", "txt"];
const SOFFICE_WRITE: &[&str] = &["pdf", "docx", "doc", "odt", "rtf", "html", "txt", "epub"];
const PANDOC_READ: &[&str] = &["md", "markdown", "html", "htm", "docx", "odt", "rtf", "epub", "txt", "csv"];
const PANDOC_WRITE: &[&str] = &["md", "html", "docx", "odt", "rtf", "epub", "txt"];

/// Alle Dokument-Endungen, die als Ziel angeboten werden.
pub const DOC_OUT: &[&str] = &["pdf", "docx", "doc", "odt", "rtf", "txt", "html", "md", "epub", "csv", "json", "xlsx", "xls", "ods", "pptx", "ppt", "odp"];

fn hidden(c: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

fn tool(cell: &'static OnceLock<Option<PathBuf>>, names: &[&str], extra: &[&str]) -> Option<PathBuf> {
    cell.get_or_init(|| names.iter().find_map(|n| crate::editors::which(n)).or_else(|| extra.iter().map(PathBuf::from).find(|p| p.is_file()))).clone()
}

pub fn soffice() -> Option<PathBuf> {
    static C: OnceLock<Option<PathBuf>> = OnceLock::new();
    tool(&C, &["soffice"], &[r"C:\Program Files\LibreOffice\program\soffice.exe", r"C:\Program Files (x86)\LibreOffice\program\soffice.exe"])
}

pub fn pandoc() -> Option<PathBuf> {
    static C: OnceLock<Option<PathBuf>> = OnceLock::new();
    tool(&C, &["pandoc"], &[])
}

pub fn pdftotext() -> Option<PathBuf> {
    static C: OnceLock<Option<PathBuf>> = OnceLock::new();
    tool(&C, &["pdftotext"], &[])
}

fn norm(e: &str) -> &str {
    match e {
        "htm" => "html",
        "markdown" => "md",
        other => other,
    }
}

/// Lässt sich das Dokument `from` nach `to` umwandeln (mit den vorhandenen Programmen)?
pub fn can(from: &str, to: &str) -> bool {
    let (from, to) = (norm(from), norm(to));
    if from == to || to == "pages" {
        return false;
    }
    if from == "pages" {
        return matches!(to, "pdf" | "png" | "jpg");
    }
    let so = soffice().is_some();
    // Präsentationen: nur mit LibreOffice, nur untereinander und nach PDF/HTML
    if SO_SLIDES.contains(&from) {
        return so && matches!(to, "pdf" | "pptx" | "ppt" | "odp" | "html");
    }
    if SO_SLIDES.contains(&to) {
        return false;
    }
    // XLS/ODS: über LibreOffice, untereinander, mit XLSX/CSV und nach PDF/HTML
    if SO_SHEETS.contains(&from) {
        return so && matches!(to, "pdf" | "xlsx" | "xls" | "ods" | "csv" | "html");
    }
    if SO_SHEETS.contains(&to) {
        return so && (TABLES.contains(&from) || SO_SHEETS.contains(&from));
    }
    // CSV/JSON/XLSX als Ziel nur aus Tabellen bzw. strukturierten Daten
    if TABLES.contains(&to) && !TABLES.contains(&from) {
        return false;
    }
    let readable = OWN_READ.contains(&from) || (from == "pdf" && pdftotext().is_some()) || (from == "doc" && soffice().is_some());
    let writable = OWN_WRITE.contains(&to) || (to == "doc" && soffice().is_some());
    readable && writable && (to != "doc" || SOFFICE_READ.contains(&from))
}

/// Umwandeln (blockiert). Ziel existiert noch nicht.
pub fn convert(src: &Path, dst: &Path) -> Result<(), String> {
    let (from, to) = (norm(&crate::convert::ext_of(src)).to_string(), norm(&crate::convert::ext_of(dst)).to_string());
    if from == "pages" {
        return pages_preview(src, dst);
    }
    // Tabellen und Präsentationen, die nur LibreOffice kennt
    if [SO_SHEETS, SO_SLIDES].iter().any(|g| g.contains(&from.as_str()) || g.contains(&to.as_str())) {
        let so = soffice().ok_or("Für dieses Format wird LibreOffice benötigt (libreoffice.org)")?;
        return via_soffice(&so, src, dst, &to);
    }
    // Tabellen untereinander: CSV, JSON, XLSX
    if TABLES.contains(&from.as_str()) && TABLES.contains(&to.as_str()) {
        let rows = table_rows(src, &from)?;
        let bytes = match to.as_str() {
            "csv" => rows_to_csv(&rows).into_bytes(),
            "json" => serde_json::to_string_pretty(&csv_to_json(&rows_to_csv(&rows))).unwrap_or_default().into_bytes(),
            _ => to_xlsx(&rows)?,
        };
        return fs::write(dst, bytes).map_err(|e| e.to_string());
    }
    // LibreOffice: altes Word und Office → PDF/DOC mit echtem Layout
    if let Some(so) = soffice() {
        let office = matches!(from.as_str(), "doc" | "docx" | "odt" | "rtf");
        if SOFFICE_READ.contains(&from.as_str()) && SOFFICE_WRITE.contains(&to.as_str()) && (from == "doc" || to == "doc" || (office && matches!(to.as_str(), "pdf" | "docx" | "odt" | "rtf"))) {
            return via_soffice(&so, src, dst, &to);
        }
    }
    // pandoc: gute Umsetzung zwischen Markdown, HTML, DOCX, ODT, RTF, EPUB
    if let Some(pd) = pandoc() {
        if PANDOC_READ.contains(&from.as_str()) && PANDOC_WRITE.contains(&to.as_str()) && from != "csv" {
            let mut c = Command::new(pd);
            c.arg(src).arg("-o").arg(dst).arg("--standalone");
            if from == "txt" {
                c.args(["-f", "markdown"]);
            }
            if to == "txt" {
                c.args(["-t", "plain"]);
            }
            let out = hidden(&mut c).output().map_err(|e| e.to_string())?;
            if out.status.success() {
                return Ok(());
            }
        }
    }
    // Strukturierte Daten
    match (from.as_str(), to.as_str()) {
        ("csv", "json") => return fs::write(dst, serde_json::to_string_pretty(&csv_to_json(&read_text(src)?)).unwrap_or_default()).map_err(|e| e.to_string()),
        ("json", "csv") => {
            let v: serde_json::Value = serde_json::from_str(&read_text(src)?).map_err(|e| format!("JSON ungültig: {e}"))?;
            return fs::write(dst, json_to_csv(&v)?).map_err(|e| e.to_string());
        }
        _ => {}
    }
    let blocks = read(src, &from)?;
    let title = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    write(dst, &to, &blocks, &title)
}

fn via_soffice(so: &Path, src: &Path, dst: &Path, to: &str) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!("foldex-soffice-{}-{}", std::process::id(), crate::convert::ext_of(dst)));
    let _ = fs::create_dir_all(&tmp);
    let filter = match to {
        "txt" => "txt:Text (encoded):UTF8".to_string(),
        other => other.to_string(),
    };
    let mut c = Command::new(so);
    c.arg("--headless");
    // HTML öffnet LibreOffice sonst als Webseite – dafür gibt es keine Word-Exportfilter
    if matches!(crate::convert::ext_of(src).as_str(), "html" | "htm") {
        c.arg("--infilter=HTML (StarWriter)");
    }
    let out = hidden(c.args(["--convert-to", &filter, "--outdir"]).arg(&tmp).arg(src)).output().map_err(|e| e.to_string())?;
    let made = tmp.join(format!("{}.{to}", src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()));
    let r = if made.is_file() {
        fs::copy(&made, dst).map(|_| ()).map_err(|e| e.to_string())
    } else {
        Err(format!("LibreOffice konnte nicht umwandeln: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")))
    };
    let _ = fs::remove_dir_all(&tmp);
    r
}

fn read_text(p: &Path) -> Result<String, String> {
    let b = fs::read(p).map_err(|e| e.to_string())?;
    Ok(match String::from_utf8(b) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(e) => e.into_bytes().iter().map(|&c| cp1252(c)).collect(),
    })
}

fn zip_text(p: &Path, name: &str) -> Result<String, String> {
    let f = fs::File::open(p).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("Kein gültiges Dokument: {e}"))?;
    let mut s = String::new();
    z.by_name(name).map_err(|_| format!("„{name}“ fehlt im Dokument"))?.read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}

// ───────────────────────────── Lesen ─────────────────────────────

pub fn read(src: &Path, ext: &str) -> Result<Vec<Block>, String> {
    Ok(match ext {
        "txt" => from_txt(&read_text(src)?),
        "md" => from_md(&read_text(src)?),
        "html" => from_html(&read_text(src)?),
        "docx" => from_docx(&zip_text(src, "word/document.xml")?),
        "odt" => from_odt(&zip_text(src, "content.xml")?),
        "rtf" => from_txt(&rtf_text(&read_text(src)?)),
        "epub" => from_epub(src)?,
        "csv" => vec![Block::Table(parse_csv(&read_text(src)?))],
        "xlsx" => vec![Block::Table(xlsx_rows(src)?)],
        "json" => {
            let t = read_text(src)?;
            let v: serde_json::Value = serde_json::from_str(&t).map_err(|e| format!("JSON ungültig: {e}"))?;
            vec![Block::Code(serde_json::to_string_pretty(&v).unwrap_or(t))]
        }
        "pdf" => {
            let pt = pdftotext().ok_or("PDF-Text lesen braucht pdftotext (Poppler) oder LibreOffice")?;
            let out = hidden(Command::new(pt).args(["-layout", "-enc", "UTF-8"]).arg(src).arg("-")).output().map_err(|e| e.to_string())?;
            from_txt(&String::from_utf8_lossy(&out.stdout))
        }
        "doc" => {
            let so = soffice().ok_or("Alte Word-Dateien (.doc) brauchen LibreOffice")?;
            let tmp = std::env::temp_dir().join(format!("foldex-doc-{}.txt", std::process::id()));
            via_soffice(&so, src, &tmp, "txt")?;
            let t = read_text(&tmp);
            let _ = fs::remove_file(&tmp);
            from_txt(&t?)
        }
        other => return Err(format!(".{other} kann nicht als Dokument gelesen werden")),
    })
}

fn from_txt(t: &str) -> Vec<Block> {
    t.replace("\r\n", "\n").split("\n\n").map(|p| p.trim_matches('\n')).filter(|p| !p.trim().is_empty()).map(|p| Block::P(p.to_string())).collect()
}

fn from_md(t: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::P(para.join(" ")));
            para.clear();
        }
    };
    let lines: Vec<&str> = t.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        let tr = l.trim();
        if let Some(c) = &mut code {
            if tr.starts_with("```") {
                out.push(Block::Code(c.join("\n")));
                code = None;
            } else {
                c.push(l);
            }
        } else if tr.starts_with("```") {
            flush(&mut para, &mut out);
            code = Some(Vec::new());
        } else if tr.starts_with('|') && lines.get(i + 1).is_some_and(|n| n.trim().starts_with('|') && n.contains("--")) {
            flush(&mut para, &mut out);
            let row = |s: &str| s.trim().trim_matches('|').split('|').map(|c| c.trim().to_string()).collect::<Vec<_>>();
            let mut rows = vec![row(l)];
            i += 2;
            while i < lines.len() && lines[i].trim().starts_with('|') {
                rows.push(row(lines[i]));
                i += 1;
            }
            out.push(Block::Table(rows));
            continue;
        } else if let Some(h) = tr.strip_prefix('#') {
            flush(&mut para, &mut out);
            let level = 1 + h.chars().take_while(|&c| c == '#').count();
            out.push(Block::H(level.min(6) as u8, inline_md(h.trim_start_matches('#').trim())));
        } else if let Some(li) = tr.strip_prefix("- ").or_else(|| tr.strip_prefix("* ")).or_else(|| tr.split_once(". ").filter(|(n, _)| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty()).map(|x| x.1)) {
            flush(&mut para, &mut out);
            out.push(Block::Li(inline_md(li)));
        } else if tr.is_empty() {
            flush(&mut para, &mut out);
        } else {
            para.push(tr);
        }
        i += 1;
    }
    if let Some(c) = code {
        out.push(Block::Code(c.join("\n")));
    }
    let rest: Vec<&str> = para.iter().copied().collect();
    if !rest.is_empty() {
        out.push(Block::P(inline_md(&rest.join(" "))));
    }
    out.iter_mut().for_each(|b| {
        if let Block::P(t) = b {
            *t = inline_md(t);
        }
    });
    out
}

/// Markdown-Hervorhebungen entfernen (**fett**, `code`, [Text](url)).
fn inline_md(t: &str) -> String {
    let mut s = t.replace("**", "").replace("__", "").replace('`', "");
    while let (Some(a), Some(m)) = (s.find('['), s.find("](")) {
        let Some(e) = s[m..].find(')').map(|e| m + e) else { break };
        if a > m {
            break;
        }
        s = format!("{}{}{}", &s[..a], &s[a + 1..m], &s[e + 1..]);
    }
    s
}

/// Einfacher XML/HTML-Zerleger: (Tag-Name klein, ist schließend, Attribute) oder Text.
enum Tok<'a> {
    Tag(String, bool, &'a str),
    Text(&'a str),
}

fn tokens(s: &str) -> Vec<Tok<'_>> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with("<!--") {
            i = s[i..].find("-->").map_or(s.len(), |e| i + e + 3);
            continue;
        }
        if s.as_bytes()[i] == b'<' {
            let e = s[i..].find('>').map_or(s.len(), |e| i + e);
            let inner = &s[i + 1..e.min(s.len())];
            let close = inner.starts_with('/');
            let body = inner.trim_start_matches('/');
            let name_end = body.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(body.len());
            v.push(Tok::Tag(body[..name_end].to_ascii_lowercase(), close, &body[name_end..]));
            i = e + 1;
        } else {
            let e = s[i..].find('<').map_or(s.len(), |e| i + e);
            v.push(Tok::Text(&s[i..e]));
            i = e;
        }
    }
    v
}

fn entities(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut rest = t;
    while let Some(a) = rest.find('&') {
        out.push_str(&rest[..a]);
        let tail = &rest[a..];
        let Some(e) = tail.find(';').filter(|&e| e < 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let name = &tail[1..e];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            n if n.starts_with("#x") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=e]),
        }
        rest = &tail[e + 1..];
    }
    out.push_str(rest);
    out
}

fn from_html(s: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut kind = "p".to_string();
    let mut skip: u32 = 0;
    let mut pre = false;
    let mut table: Option<Vec<Vec<String>>> = None;
    let flush = |cur: &mut String, kind: &str, out: &mut Vec<Block>, pre: bool| {
        let t = if pre { cur.trim_matches('\n').to_string() } else { cur.split_whitespace().collect::<Vec<_>>().join(" ") };
        cur.clear();
        if t.is_empty() {
            return;
        }
        out.push(match kind {
            k if k.len() == 2 && k.starts_with('h') && k.as_bytes()[1].is_ascii_digit() => Block::H(k.as_bytes()[1] - b'0', t),
            "li" => Block::Li(t),
            "pre" => Block::Code(t),
            _ => Block::P(t),
        });
    };
    for tok in tokens(s) {
        match tok {
            Tok::Tag(n, close, _) if matches!(n.as_str(), "script" | "style" | "head" | "title") => {
                skip = if close { skip.saturating_sub(1) } else { skip + 1 };
            }
            _ if skip > 0 => {}
            Tok::Tag(n, close, _) => match n.as_str() {
                "table" if !close => table = Some(Vec::new()),
                "table" => {
                    if let Some(t) = table.take().filter(|t| !t.is_empty()) {
                        out.push(Block::Table(t));
                    }
                }
                "tr" if !close => {
                    if let Some(t) = &mut table {
                        t.push(Vec::new());
                    }
                }
                "td" | "th" if close => {
                    if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                        row.push(entities(&cur).split_whitespace().collect::<Vec<_>>().join(" "));
                        cur.clear();
                    }
                }
                "br" => cur.push('\n'),
                "p" | "div" | "li" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "section" | "article" if table.is_none() => {
                    flush(&mut cur, &kind, &mut out, pre);
                    pre = n == "pre" && !close;
                    kind = if close { "p".into() } else { n };
                }
                _ => {}
            },
            Tok::Text(t) => cur.push_str(&entities(t)),
        }
    }
    flush(&mut cur, &kind, &mut out, pre);
    out
}

/// Attributwert aus einem Tag-Rest lesen.
fn attr<'a>(rest: &'a str, name: &str) -> Option<&'a str> {
    let i = rest.find(&format!("{name}=\""))? + name.len() + 2;
    rest[i..].find('"').map(|e| &rest[i..i + e])
}

fn from_docx(xml: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let (mut cur, mut style, mut list, mut in_t) = (String::new(), String::new(), false, false);
    for tok in tokens(xml) {
        match tok {
            Tok::Tag(n, false, rest) if n == "w:p" && !rest.ends_with('/') => {
                cur.clear();
                style.clear();
                list = false;
            }
            Tok::Tag(n, true, _) if n == "w:p" => {
                let t = std::mem::take(&mut cur);
                if !t.trim().is_empty() {
                    let s = style.to_lowercase();
                    let level = s.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u8>().ok();
                    out.push(if s.contains("heading") || s.contains("berschrift") || s == "title" {
                        Block::H(level.unwrap_or(1).clamp(1, 6), t)
                    } else if list || s.contains("list") {
                        Block::Li(t)
                    } else {
                        Block::P(t)
                    });
                }
            }
            Tok::Tag(n, false, rest) if n == "w:pstyle" => style = attr(rest, "w:val").unwrap_or("").to_string(),
            Tok::Tag(n, false, _) if n == "w:numpr" => list = true,
            Tok::Tag(n, close, rest) if n == "w:t" => in_t = !close && !rest.ends_with('/'),
            Tok::Tag(n, false, _) if n == "w:tab" => cur.push('\t'),
            Tok::Tag(n, false, _) if n == "w:br" => cur.push('\n'),
            Tok::Text(t) if in_t => cur.push_str(&entities(t)),
            _ => {}
        }
    }
    out
}

fn from_odt(xml: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let (mut cur, mut kind, mut depth_item) = (String::new(), Block::P(String::new()), 0usize);
    let mut open = false;
    for tok in tokens(xml) {
        match tok {
            Tok::Tag(n, false, rest) if (n == "text:p" || n == "text:h") && !rest.ends_with('/') => {
                cur.clear();
                open = true;
                kind = if n == "text:h" {
                    Block::H(attr(rest, "text:outline-level").and_then(|l| l.parse().ok()).unwrap_or(1), String::new())
                } else if depth_item > 0 {
                    Block::Li(String::new())
                } else {
                    Block::P(String::new())
                };
            }
            Tok::Tag(n, true, _) if n == "text:p" || n == "text:h" => {
                open = false;
                let t = std::mem::take(&mut cur);
                if !t.trim().is_empty() {
                    out.push(match kind {
                        Block::H(l, _) => Block::H(l, t),
                        Block::Li(_) => Block::Li(t),
                        _ => Block::P(t),
                    });
                }
            }
            Tok::Tag(n, close, _) if n == "text:list-item" => depth_item = if close { depth_item.saturating_sub(1) } else { depth_item + 1 },
            Tok::Tag(n, false, _) if n == "text:s" => cur.push(' '),
            Tok::Tag(n, false, _) if n == "text:tab" => cur.push('\t'),
            Tok::Tag(n, false, _) if n == "text:line-break" => cur.push('\n'),
            Tok::Text(t) if open => cur.push_str(&entities(t)),
            _ => {}
        }
    }
    out
}

fn from_epub(p: &Path) -> Result<Vec<Block>, String> {
    let container = zip_text(p, "META-INF/container.xml")?;
    let opf_path = tokens(&container)
        .into_iter()
        .find_map(|t| if let Tok::Tag(n, false, rest) = t { (n == "rootfile").then(|| attr(rest, "full-path").map(str::to_string)).flatten() } else { None })
        .ok_or("EPUB ohne Inhaltsverzeichnis")?;
    let opf = zip_text(p, &opf_path)?;
    let base = opf_path.rsplit_once('/').map_or(String::new(), |(b, _)| format!("{b}/"));
    let mut items = std::collections::HashMap::new();
    let mut spine = Vec::new();
    for t in tokens(&opf) {
        if let Tok::Tag(n, false, rest) = t {
            match n.as_str() {
                "item" => {
                    if let (Some(id), Some(href)) = (attr(rest, "id"), attr(rest, "href")) {
                        items.insert(id.to_string(), href.to_string());
                    }
                }
                "itemref" => spine.extend(attr(rest, "idref").map(str::to_string)),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for id in spine {
        if let Some(href) = items.get(&id) {
            if let Ok(x) = zip_text(p, &format!("{base}{href}")) {
                out.extend(from_html(&x));
            }
        }
    }
    Ok(out)
}

/// RTF → Text: Steuerwörter entfernen, Absätze und Sonderzeichen übernehmen.
fn rtf_text(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i, mut skip_depth, mut depth) = (String::new(), 0, None::<usize>, 0usize);
    let mut uc_skip = 0;
    while i < b.len() {
        match b[i] {
            b'{' => {
                depth += 1;
                if skip_depth.is_none() && s[i + 1..].starts_with("\\*") {
                    skip_depth = Some(depth);
                }
                i += 1;
            }
            b'}' => {
                if skip_depth == Some(depth) {
                    skip_depth = None;
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= b.len() {
                    break;
                }
                let c = b[i];
                if c == b'\'' && i + 2 < b.len() {
                    if skip_depth.is_none() {
                        if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                            out.push(cp1252(v));
                        }
                    }
                    i += 3;
                    continue;
                }
                if !c.is_ascii_alphabetic() {
                    if skip_depth.is_none() && matches!(c, b'\\' | b'{' | b'}') {
                        out.push(c as char);
                    }
                    i += 1;
                    continue;
                }
                let st = i;
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word = &s[st..i];
                let ns = i;
                if i < b.len() && (b[i] == b'-' || b[i].is_ascii_digit()) {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let num: Option<i32> = s[ns..i].parse().ok();
                if i < b.len() && b[i] == b' ' {
                    i += 1;
                }
                if matches!(word, "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "header" | "footer") && skip_depth.is_none() {
                    skip_depth = Some(depth);
                }
                if skip_depth.is_some() {
                    continue;
                }
                match word {
                    "par" | "line" => out.push('\n'),
                    "tab" => out.push('\t'),
                    "u" => {
                        if let Some(n) = num {
                            out.extend(char::from_u32(if n < 0 { (n + 65536) as u32 } else { n as u32 }));
                            uc_skip = 1;
                        }
                    }
                    _ => {}
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                let ch = s[i..].chars().next().unwrap_or(' ');
                if skip_depth.is_none() {
                    if uc_skip > 0 {
                        uc_skip -= 1;
                    } else {
                        out.push(ch);
                    }
                }
                i += ch.len_utf8();
            }
        }
    }
    // Absätze: Zeilenumbrüche aus \par zu Leerzeilen
    out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n\n")
}

// ───────────────────────────── CSV / JSON ─────────────────────────────

pub fn parse_csv(t: &str) -> Vec<Vec<String>> {
    let first = t.lines().next().unwrap_or("");
    let sep = [',', ';', '\t'].into_iter().max_by_key(|c| first.matches(*c).count()).unwrap_or(',');
    let (mut rows, mut row, mut cell, mut q) = (Vec::new(), Vec::new(), String::new(), false);
    let mut it = t.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' if q && it.peek() == Some(&'"') => {
                cell.push('"');
                it.next();
            }
            '"' => q = !q,
            c if c == sep && !q => row.push(std::mem::take(&mut cell)),
            '\r' if !q => {}
            '\n' if !q => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// CSV mit korrektem Maskieren (Komma, Anführungszeichen, Zeilenumbruch in Zellen).
fn rows_to_csv(rows: &[Vec<String>]) -> String {
    let cell = |s: &String| if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.clone() };
    rows.iter().map(|r| r.iter().map(cell).collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join("\n") + "\n"
}

/// Zeilen einer Tabelle (CSV, JSON-Liste von Objekten, erstes Blatt einer XLSX-Datei).
fn table_rows(src: &Path, ext: &str) -> Result<Vec<Vec<String>>, String> {
    match ext {
        "csv" => Ok(parse_csv(&read_text(src)?)),
        "json" => {
            let v: serde_json::Value = serde_json::from_str(&read_text(src)?).map_err(|e| format!("JSON ungültig: {e}"))?;
            Ok(parse_csv(&json_to_csv(&v)?))
        }
        "xlsx" => xlsx_rows(src),
        other => Err(format!(".{other} ist keine Tabelle")),
    }
}

/// Spaltennummer aus einem Zellbezug („C12“ → 2).
fn col_index(r: &str) -> usize {
    r.chars().take_while(|c| c.is_ascii_alphabetic()).fold(0usize, |n, c| n * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1)).saturating_sub(1)
}

/// Text aller <t>-Elemente in einem XML-Ausschnitt.
fn xml_texts(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find("<t") {
        let after = &rest[a + 2..];
        // <t> oder <t xml:space=…>, nicht <tc…>/<tab…>
        if !after.starts_with('>') && !after.starts_with(' ') {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            rest = &after[gt + 1..];
            continue;
        }
        let body = &after[gt + 1..];
        let end = body.find("</t>").unwrap_or(body.len());
        out.push_str(&entities(&body[..end]));
        rest = &body[end..];
    }
    out
}

/// Erstes Arbeitsblatt einer XLSX-Datei als Zeilen (gemeinsame Zeichenketten, Inline-Text, Zahlen, Wahrheitswerte).
fn xlsx_rows(src: &Path) -> Result<Vec<Vec<String>>, String> {
    let shared: Vec<String> = zip_text(src, "xl/sharedStrings.xml")
        .map(|x| x.split("<si").skip(1).map(|si| xml_texts(si.split("</si>").next().unwrap_or(""))).collect())
        .unwrap_or_default();
    let sheet = zip_text(src, "xl/worksheets/sheet1.xml")?;
    let mut rows: Vec<Vec<String>> = Vec::new();
    for row in sheet.split("<row").skip(1) {
        let row = row.split("</row>").next().unwrap_or("");
        let mut cells: Vec<String> = Vec::new();
        for c in row.split("<c ").skip(1) {
            let (head, body) = c.split_once('>').unwrap_or((c, ""));
            if head.ends_with('/') {
                continue; // leere Zelle
            }
            let body = body.split("</c>").next().unwrap_or("");
            let col = attr(head, "r").map_or(cells.len(), col_index);
            let v = body.split_once("<v>").and_then(|(_, r)| r.split_once("</v>")).map(|(v, _)| entities(v)).unwrap_or_default();
            let val = match attr(head, "t") {
                Some("s") => v.parse::<usize>().ok().and_then(|i| shared.get(i).cloned()).unwrap_or_default(),
                Some("inlineStr") => xml_texts(body),
                Some("b") => if v == "1" { "TRUE".into() } else { "FALSE".into() },
                _ => v,
            };
            if cells.len() <= col {
                cells.resize(col + 1, String::new());
            }
            cells[col] = val;
        }
        rows.push(cells);
    }
    Ok(rows)
}

/// XLSX mit einem Blatt; Zahlen bleiben Zahlen, alles andere wird Text.
fn to_xlsx(rows: &[Vec<String>]) -> Result<Vec<u8>, String> {
    let col_name = |mut i: usize| {
        let mut s = String::new();
        loop {
            s.insert(0, (b'A' + (i % 26) as u8) as char);
            if i < 26 {
                break s;
            }
            i = i / 26 - 1;
        }
    };
    let mut data = String::new();
    for (r, row) in rows.iter().enumerate() {
        data.push_str(&format!("<row r=\"{}\">", r + 1));
        for (c, v) in row.iter().enumerate() {
            let at = format!("{}{}", col_name(c), r + 1);
            // Zahl – außer mit führender Null (Postleitzahlen, Nummern bleiben Text)
            let leading_zero = v.len() > 1 && v.starts_with('0') && !v.starts_with("0.");
            if v.parse::<f64>().is_ok_and(|f| f.is_finite()) && !leading_zero {
                data.push_str(&format!("<c r=\"{at}\"><v>{v}</v></c>"));
            } else if !v.is_empty() {
                data.push_str(&format!("<c r=\"{at}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>", xml_esc(v)));
            }
        }
        data.push_str("</row>");
    }
    let ns = "http://schemas.openxmlformats.org";
    zip_build(&[
        ("[Content_Types].xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"{ns}/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>").into_bytes(), false),
        ("_rels/.rels", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"{ns}/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{ns}/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>").into_bytes(), false),
        ("xl/workbook.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><workbook xmlns=\"{ns}/spreadsheetml/2006/main\" xmlns:r=\"{ns}/officeDocument/2006/relationships\"><sheets><sheet name=\"Tabelle1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>").into_bytes(), false),
        ("xl/_rels/workbook.xml.rels", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"{ns}/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{ns}/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>").into_bytes(), false),
        ("xl/worksheets/sheet1.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"{ns}/spreadsheetml/2006/main\"><sheetData>{data}</sheetData></worksheet>").into_bytes(), false),
    ])
}

fn csv_to_json(t: &str) -> serde_json::Value {
    let rows = parse_csv(t);
    let Some((head, body)) = rows.split_first() else { return serde_json::json!([]) };
    serde_json::Value::Array(
        body.iter()
            .map(|r| {
                let mut m = serde_json::Map::new();
                for (i, h) in head.iter().enumerate() {
                    let v = r.get(i).cloned().unwrap_or_default();
                    let val = v.parse::<i64>().map(serde_json::Value::from).or_else(|_| v.parse::<f64>().map(serde_json::Value::from)).unwrap_or(serde_json::Value::String(v));
                    m.insert(h.clone(), val);
                }
                serde_json::Value::Object(m)
            })
            .collect(),
    )
}

fn json_to_csv(v: &serde_json::Value) -> Result<String, String> {
    let arr = v.as_array().or_else(|| v.as_object().and_then(|o| o.values().find_map(|x| x.as_array()))).ok_or("JSON → CSV braucht eine Liste von Objekten")?;
    let mut head: Vec<String> = Vec::new();
    for o in arr.iter().filter_map(|x| x.as_object()) {
        for k in o.keys() {
            if !head.contains(k) {
                head.push(k.clone());
            }
        }
    }
    let cell = |s: String| if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s };
    let mut out = head.iter().cloned().map(cell).collect::<Vec<_>>().join(",") + "\n";
    for x in arr {
        let line: Vec<String> = if let Some(o) = x.as_object() {
            head.iter().map(|k| cell(o.get(k).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| if v.is_null() { String::new() } else { v.to_string() })).unwrap_or_default())).collect()
        } else {
            vec![cell(x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string()))]
        };
        out.push_str(&line.join(","));
        out.push('\n');
    }
    Ok(out)
}

// ───────────────────────────── Schreiben ─────────────────────────────

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn write(dst: &Path, ext: &str, blocks: &[Block], title: &str) -> Result<(), String> {
    let bytes: Vec<u8> = match ext {
        "txt" => to_txt(blocks).into_bytes(),
        "md" => to_md(blocks).into_bytes(),
        "html" => to_html(blocks, title).into_bytes(),
        "rtf" => to_rtf(blocks).into_bytes(),
        "docx" => to_docx(blocks)?,
        "odt" => to_odt(blocks)?,
        "epub" => to_epub(blocks, title)?,
        "pdf" => crate::pdf::text_pdf(blocks),
        "csv" => match blocks {
            [Block::Table(rows)] => rows_to_csv(rows).into_bytes(),
            _ => return Err("Nur Tabellen lassen sich als CSV speichern".into()),
        },
        "xlsx" => match blocks {
            [Block::Table(rows)] => to_xlsx(rows)?,
            _ => return Err("Nur Tabellen lassen sich als XLSX speichern".into()),
        },
        "doc" => {
            let so = soffice().ok_or("Für .doc wird LibreOffice benötigt")?;
            let tmp = std::env::temp_dir().join(format!("foldex-{}.docx", std::process::id()));
            fs::write(&tmp, to_docx(blocks)?).map_err(|e| e.to_string())?;
            let r = via_soffice(&so, &tmp, dst, "doc");
            let _ = fs::remove_file(&tmp);
            return r;
        }
        other => return Err(format!(".{other} wird als Ziel nicht unterstützt")),
    };
    fs::write(dst, bytes).map_err(|e| e.to_string())
}

fn to_txt(b: &[Block]) -> String {
    let mut s = String::new();
    for x in b {
        match x {
            Block::H(_, t) => s.push_str(&format!("{t}\n\n")),
            Block::P(t) => s.push_str(&format!("{t}\n\n")),
            Block::Li(t) => s.push_str(&format!("• {t}\n")),
            Block::Code(t) => s.push_str(&format!("{t}\n\n")),
            Block::Table(rows) => {
                for r in rows {
                    s.push_str(&r.join("\t"));
                    s.push('\n');
                }
                s.push('\n');
            }
        }
    }
    s.trim_end().to_string() + "\n"
}

/// Blöcke als Markdown (auch für die Schnellansicht von Office-Dateien)
pub fn to_markdown(b: &[Block]) -> String {
    to_md(b)
}

fn to_md(b: &[Block]) -> String {
    let mut s = String::new();
    for x in b {
        match x {
            Block::H(l, t) => s.push_str(&format!("{} {t}\n\n", "#".repeat(*l as usize))),
            Block::P(t) => s.push_str(&format!("{t}\n\n")),
            Block::Li(t) => s.push_str(&format!("- {t}\n")),
            Block::Code(t) => s.push_str(&format!("```\n{t}\n```\n\n")),
            Block::Table(rows) => {
                for (i, r) in rows.iter().enumerate() {
                    s.push_str(&format!("| {} |\n", r.iter().map(|c| c.replace('|', "\\|")).collect::<Vec<_>>().join(" | ")));
                    if i == 0 {
                        s.push_str(&format!("|{}\n", " --- |".repeat(r.len().max(1))));
                    }
                }
                s.push('\n');
            }
        }
    }
    s.trim_end().to_string() + "\n"
}

fn html_body(b: &[Block]) -> String {
    let mut s = String::new();
    let mut in_list = false;
    for x in b {
        if in_list && !matches!(x, Block::Li(_)) {
            s.push_str("</ul>\n");
            in_list = false;
        }
        match x {
            Block::H(l, t) => s.push_str(&format!("<h{l}>{}</h{l}>\n", esc(t))),
            Block::P(t) => s.push_str(&format!("<p>{}</p>\n", esc(t).replace('\n', "<br/>"))),
            Block::Li(t) => {
                if !in_list {
                    s.push_str("<ul>\n");
                    in_list = true;
                }
                s.push_str(&format!("<li>{}</li>\n", esc(t)));
            }
            Block::Code(t) => s.push_str(&format!("<pre>{}</pre>\n", esc(t))),
            Block::Table(rows) => {
                s.push_str("<table>\n");
                for (i, r) in rows.iter().enumerate() {
                    let tag = if i == 0 { "th" } else { "td" };
                    s.push_str(&format!("<tr>{}</tr>\n", r.iter().map(|c| format!("<{tag}>{}</{tag}>", esc(c))).collect::<String>()));
                }
                s.push_str("</table>\n");
            }
        }
    }
    if in_list {
        s.push_str("</ul>\n");
    }
    s
}

fn to_html(b: &[Block], title: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"de\">\n<head>\n<meta charset=\"utf-8\"/>\n<title>{}</title>\n<style>body{{font-family:Segoe UI,sans-serif;max-width:46em;margin:2em auto;line-height:1.5;padding:0 1em}}pre{{background:#f4f4f4;padding:.8em;overflow:auto}}table{{border-collapse:collapse}}td,th{{border:1px solid #ccc;padding:.3em .6em}}</style>\n</head>\n<body>\n{}</body>\n</html>\n",
        esc(title),
        html_body(b)
    )
}

fn rtf_esc(t: &str) -> String {
    let mut s = String::new();
    for c in t.chars() {
        match c {
            '\\' | '{' | '}' => {
                s.push('\\');
                s.push(c);
            }
            '\n' => s.push_str("\\line "),
            '\t' => s.push_str("\\tab "),
            c if (c as u32) < 128 => s.push(c),
            c => {
                let n = c as u32;
                if n < 0x10000 {
                    s.push_str(&format!("\\u{}?", n as i16));
                } else {
                    s.push('?');
                }
            }
        }
    }
    s
}

fn to_rtf(b: &[Block]) -> String {
    let mut s = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0 Calibri;}{\\f1 Consolas;}}\\f0\\fs22\n");
    for x in b {
        match x {
            Block::H(l, t) => s.push_str(&format!("{{\\b\\fs{} {}}}\\par\\par\n", 36 - (*l as usize).min(4) * 3, rtf_esc(t))),
            Block::P(t) => s.push_str(&format!("{}\\par\\par\n", rtf_esc(t))),
            Block::Li(t) => s.push_str(&format!("\\bullet\\tab {}\\par\n", rtf_esc(t))),
            Block::Code(t) => s.push_str(&format!("{{\\f1\\fs20 {}}}\\par\\par\n", rtf_esc(t))),
            Block::Table(rows) => {
                for r in rows {
                    s.push_str(&format!("{}\\par\n", r.iter().map(|c| rtf_esc(c)).collect::<Vec<_>>().join("\\tab ")));
                }
                s.push_str("\\par\n");
            }
        }
    }
    s.push('}');
    s
}

fn zip_build(files: &[(&str, Vec<u8>, bool)]) -> Result<Vec<u8>, String> {
    use zip::write::SimpleFileOptions;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, data, stored) in files {
            let opt = SimpleFileOptions::default().compression_method(if *stored { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated });
            z.start_file(*name, opt).map_err(|e| e.to_string())?;
            z.write_all(data).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn to_docx(b: &[Block]) -> Result<Vec<u8>, String> {
    let run = |t: &str, extra: &str| -> String {
        t.split('\n').enumerate().map(|(i, l)| format!("{}<w:r>{extra}<w:t xml:space=\"preserve\">{}</w:t></w:r>", if i > 0 { "<w:r><w:br/></w:r>" } else { "" }, xml_esc(l))).collect()
    };
    let mut body = String::new();
    for x in b {
        match x {
            Block::H(l, t) => body.push_str(&format!("<w:p><w:pPr><w:pStyle w:val=\"Heading{}\"/></w:pPr>{}</w:p>", (*l).min(3), run(t, ""))),
            Block::P(t) => body.push_str(&format!("<w:p>{}</w:p>", run(t, ""))),
            Block::Li(t) => body.push_str(&format!("<w:p><w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr>{}</w:p>", run(&format!("•\t{t}"), ""))),
            Block::Code(t) => body.push_str(&format!("<w:p>{}</w:p>", run(t, "<w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/><w:sz w:val=\"20\"/></w:rPr>"))),
            Block::Table(rows) => {
                body.push_str("<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\"/><w:left w:val=\"single\" w:sz=\"4\"/><w:bottom w:val=\"single\" w:sz=\"4\"/><w:right w:val=\"single\" w:sz=\"4\"/><w:insideH w:val=\"single\" w:sz=\"4\"/><w:insideV w:val=\"single\" w:sz=\"4\"/></w:tblBorders></w:tblPr>");
                for r in rows {
                    body.push_str("<w:tr>");
                    for c in r {
                        body.push_str(&format!("<w:tc><w:p>{}</w:p></w:tc>", run(c, "")));
                    }
                    body.push_str("</w:tr>");
                }
                body.push_str("</w:tbl><w:p/>");
            }
        }
    }
    let doc = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1417\" w:right=\"1417\" w:bottom=\"1134\" w:left=\"1417\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>");
    let heading = |n: u8, sz: u32| format!("<w:style w:type=\"paragraph\" w:styleId=\"Heading{n}\"><w:name w:val=\"heading {n}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"120\"/><w:outlineLvl w:val=\"{}\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"{sz}\"/></w:rPr></w:style>", n - 1);
    let styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:cs=\"Calibri\"/><w:sz w:val=\"22\"/><w:lang w:val=\"de-DE\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"264\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>{}{}{}<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/></w:style><w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/></w:style></w:styles>",
        heading(1, 36),
        heading(2, 30),
        heading(3, 26)
    );
    zip_build(&[
        ("[Content_Types].xml", b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/></Types>".to_vec(), false),
        ("_rels/.rels", b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".to_vec(), false),
        ("word/_rels/document.xml.rels", b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>".to_vec(), false),
        ("word/document.xml", doc.into_bytes(), false),
        ("word/styles.xml", styles.into_bytes(), false),
    ])
}

fn to_odt(b: &[Block]) -> Result<Vec<u8>, String> {
    let line = |t: &str| t.split('\n').map(xml_esc).collect::<Vec<_>>().join("<text:line-break/>");
    let mut body = String::new();
    let mut in_list = false;
    for x in b {
        if in_list && !matches!(x, Block::Li(_)) {
            body.push_str("</text:list>");
            in_list = false;
        }
        match x {
            Block::H(l, t) => body.push_str(&format!("<text:h text:style-name=\"Heading_20_{l}\" text:outline-level=\"{l}\">{}</text:h>", line(t))),
            Block::P(t) => body.push_str(&format!("<text:p text:style-name=\"Standard\">{}</text:p>", line(t))),
            Block::Li(t) => {
                if !in_list {
                    body.push_str("<text:list>");
                    in_list = true;
                }
                body.push_str(&format!("<text:list-item><text:p>{}</text:p></text:list-item>", line(t)));
            }
            Block::Code(t) => body.push_str(&format!("<text:p text:style-name=\"Preformatted_20_Text\">{}</text:p>", line(t))),
            Block::Table(rows) => {
                let cols = rows.iter().map(Vec::len).max().unwrap_or(1);
                body.push_str(&format!("<table:table><table:table-column table:number-columns-repeated=\"{cols}\"/>"));
                for r in rows {
                    body.push_str("<table:table-row>");
                    for c in r {
                        body.push_str(&format!("<table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell>", line(c)));
                    }
                    body.push_str("</table:table-row>");
                }
                body.push_str("</table:table>");
            }
        }
    }
    if in_list {
        body.push_str("</text:list>");
    }
    let content = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" office:version=\"1.3\"><office:body><office:text>{body}</office:text></office:body></office:document-content>");
    zip_build(&[
        ("mimetype", b"application/vnd.oasis.opendocument.text".to_vec(), true),
        ("META-INF/manifest.xml", b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>".to_vec(), false),
        ("content.xml", content.into_bytes(), false),
    ])
}

fn to_epub(b: &[Block], title: &str) -> Result<Vec<u8>, String> {
    let t = esc(title);
    let id = format!("urn:foldex:{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    let chapter = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"de\"><head><title>{t}</title></head><body>\n{}</body></html>", html_body(b).replace("<br/>", "<br />"));
    let opf = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"uid\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"uid\">{id}</dc:identifier><dc:title>{t}</dc:title><dc:language>de</dc:language><meta property=\"dcterms:modified\">2026-01-01T00:00:00Z</meta></metadata><manifest><item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/><item id=\"c1\" href=\"text.xhtml\" media-type=\"application/xhtml+xml\"/></manifest><spine><itemref idref=\"c1\"/></spine></package>");
    let nav = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>{t}</title></head><body><nav epub:type=\"toc\"><ol><li><a href=\"text.xhtml\">{t}</a></li></ol></nav></body></html>");
    zip_build(&[
        ("mimetype", b"application/epub+zip".to_vec(), true),
        ("META-INF/container.xml", b"<?xml version=\"1.0\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>".to_vec(), false),
        ("OEBPS/content.opf", opf.into_bytes(), false),
        ("OEBPS/nav.xhtml", nav.into_bytes(), false),
        ("OEBPS/text.xhtml", chapter.into_bytes(), false),
    ])
}

/// Apple Pages: die mitgespeicherte Vorschau (PDF bzw. JPG) herausholen.
fn pages_preview(src: &Path, dst: &Path) -> Result<(), String> {
    let f = fs::File::open(src).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|_| "Diese Pages-Datei enthält keine lesbare Vorschau (älteres Format)".to_string())?;
    let want_pdf = crate::convert::ext_of(dst) == "pdf";
    let names: Vec<String> = z.file_names().map(str::to_string).collect();
    let pick = |pats: &[&str]| names.iter().find(|n| pats.iter().any(|p| n.to_lowercase().ends_with(p))).cloned();
    let name = if want_pdf { pick(&["preview.pdf"]) } else { pick(&["preview.jpg", "preview-web.jpg", "preview.png"]) }.ok_or(if want_pdf { "Keine PDF-Vorschau in der Pages-Datei" } else { "Kein Vorschaubild in der Pages-Datei" })?;
    let mut data = Vec::new();
    z.by_name(&name).map_err(|e| e.to_string())?.read_to_end(&mut data).map_err(|e| e.to_string())?;
    if want_pdf || crate::convert::ext_of(dst) == crate::convert::ext_of(Path::new(&name)) {
        return fs::write(dst, data).map_err(|e| e.to_string());
    }
    let img = image::load_from_memory(&data).map_err(|e| e.to_string())?;
    img.save(dst).map_err(|e| e.to_string())
}

/// Windows-1252 → Unicode (für alte Textdateien und RTF).
pub fn cp1252(b: u8) -> char {
    const HI: [u16; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122,
        0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    if (0x80..0xA0).contains(&b) {
        char::from_u32(HI[(b - 0x80) as usize] as u32).unwrap_or('?')
    } else {
        b as char
    }
}

/// Unicode → Windows-1252 (für PDF-Standardschriften); unbekannte Zeichen werden „?“.
pub fn to_cp1252(c: char) -> u8 {
    let n = c as u32;
    if n < 0x80 || (0xA0..=0xFF).contains(&n) {
        return n as u8;
    }
    (0x80u8..0xA0).find(|&b| cp1252(b) == c).unwrap_or(b'?')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let d = std::env::temp_dir().join(format!("fx-doc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let md = "# Titel\n\nEin **Absatz** mit Ümlaut.\n\n- eins\n- zwei\n\n```\ncode()\n```\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        fs::write(d.join("a.md"), md).unwrap();
        let blocks = read(&d.join("a.md"), "md").unwrap();
        assert_eq!(blocks[0], Block::H(1, "Titel".into()));
        assert_eq!(blocks[1], Block::P("Ein Absatz mit Ümlaut.".into()));
        assert!(matches!(blocks.last(), Some(Block::Table(t)) if t.len() == 2));
        // eigene Schreiber und wieder lesen
        for ext in ["docx", "odt", "html", "rtf", "epub", "txt"] {
            let p = d.join(format!("a.{ext}"));
            write(&p, ext, &blocks, "Titel").unwrap();
            let back = read(&p, ext).unwrap();
            let text = to_txt(&back);
            assert!(text.contains("Titel") && text.contains("Ümlaut"), "{ext}: {text}");
        }
        write(&d.join("a.pdf"), "pdf", &blocks, "Titel").unwrap();
        assert!(fs::read(d.join("a.pdf")).unwrap().starts_with(b"%PDF-1.4"));
        // CSV ↔ JSON
        let j = csv_to_json("name;alter\nAnna;31\n\"B; C\";x\n");
        assert_eq!(j[0]["alter"], 31);
        assert_eq!(j[1]["name"], "B; C");
        // serde_json sortiert die Schlüssel
        assert_eq!(json_to_csv(&j).unwrap(), "alter,name\n31,Anna\nx,B; C\n");
        assert_eq!(rtf_text("{\\rtf1{\\fonttbl{\\f0 Arial;}}Gr\\'fc\\'dfe\\par Welt}"), "Grüße\n\nWelt");
        assert!(can("md", "docx") && can("docx", "pdf") && can("csv", "json") && !can("txt", "csv") && can("pages", "pdf"));
        let _ = fs::remove_dir_all(&d);
    }
}
