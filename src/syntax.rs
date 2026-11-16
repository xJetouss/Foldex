//! Syntaxfarben für Quelltext in Vorschau und Viewer: Sublime-Grammatiken (syntect + two-face, rund 200
//! Sprachen von Rust, C/C++, C#, Java, Kotlin, Go, Python, JS/TS, PHP, Ruby, Swift, Zig, Haskell, SQL, Shell,
//! PowerShell bis TOML/YAML/Dockerfile …). Die Farben kommen aus dem App-Theme: jede Grammatik-Kategorie
//! (Kommentar, Zeichenkette, Schlüsselwort …) wird auf die Editor-Farben des Themes abgebildet.
//!
//! Hervorgehoben wird im Hintergrund, Zeile für Zeile; die Oberfläche zeigt bereits fertige Zeilen farbig
//! und alle anderen schlicht, bis sie an der Reihe sind – auch sehr große Dateien öffnen sofort.

use crate::themes::Theme;
use eframe::egui::{self, Color32};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use syntect::parsing::{ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxReference, SyntaxSet};

/// Farbklasse eines Abschnitts (wird erst beim Zeichnen in eine Theme-Farbe übersetzt).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cls {
    Plain,
    Comment,
    Str,
    Num,
    Keyword,
    Func,
    Type,
}

impl Cls {
    pub fn color(self, th: &Theme) -> Color32 {
        match self {
            Cls::Plain => th.ed_text,
            Cls::Comment => th.comment,
            Cls::Str => th.string,
            Cls::Num => th.number,
            Cls::Keyword => th.keyword,
            Cls::Func => th.function,
            Cls::Type => th.ty,
        }
    }
}

/// Abschnitte einer Zeile: (Start, Ende) in Bytes der Zeile, Klasse.
pub type Spans = Vec<(u32, u32, Cls)>;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// Grammatik zu Dateiname/Endung bzw. erster Zeile (Shebang, `<?xml`, …).
fn find<'a>(ss: &'a SyntaxSet, name: &str, first_line: &str) -> Option<&'a SyntaxReference> {
    let lower = name.to_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    // Endungen, die syntect unter anderem Namen führt
    let alias = match ext {
        "h" | "hh" | "hxx" => "hpp",
        "cjs" | "mjs" => "js",
        "cts" | "mts" => "ts",
        "jsonc" | "json5" => "json",
        "psm1" | "psd1" => "ps1",
        "yml" => "yaml",
        "htm" | "xhtml" => "html",
        "csproj" | "vcxproj" | "props" | "targets" | "xaml" | "resx" | "plist" => "xml",
        "kts" => "kt",
        "conf" | "cfg" | "inf" => "ini",
        e => e,
    };
    ss.find_syntax_by_extension(alias)
        .or_else(|| ss.find_syntax_by_extension(&lower))
        .or_else(|| ss.find_syntax_by_extension(name))
        .or_else(|| ss.find_syntax_by_first_line(first_line))
        .filter(|s| s.name != "Plain Text")
}

/// Kategorie einer einzelnen Grammatik-Kategorie wie „keyword.control.rust“ (None = neutral).
fn scope_cls(s: &str) -> Option<Cls> {
    let starts = |p: &str| s.starts_with(p);
    Some(if starts("comment") {
        Cls::Comment
    } else if starts("string") || starts("markup.raw") || (starts("constant.character") && !starts("constant.character.escape")) {
        Cls::Str
    } else if starts("constant.numeric") || starts("constant.character.escape") {
        Cls::Num
    } else if starts("entity.name.function") || starts("support.function") || starts("variable.function") || starts("entity.other.attribute-name") {
        Cls::Func
    } else if starts("entity.name.type")
        || starts("entity.name.class")
        || starts("entity.name.struct")
        || starts("entity.name.enum")
        || starts("entity.name.trait")
        || starts("entity.name.interface")
        || starts("entity.name.namespace")
        || starts("entity.other.inherited-class")
        || starts("support.type")
        || starts("support.class")
        || starts("storage.type.primitive")
        || starts("storage.type.builtin")
    {
        Cls::Type
    } else if starts("keyword") || starts("storage") || starts("constant.language") || starts("variable.language") || starts("entity.name.tag") || starts("support.constant") {
        Cls::Keyword
    } else {
        return None;
    })
}

/// Klasse eines ganzen Kategorie-Stapels: die innerste Kategorie mit Bedeutung gewinnt.
fn stack_cls(stack: &ScopeStack, cache: &mut HashMap<Scope, Option<Cls>>) -> Cls {
    for s in stack.as_slice().iter().rev() {
        let c = *cache.entry(*s).or_insert_with(|| scope_cls(&s.build_string()));
        if let Some(c) = c {
            return c;
        }
    }
    Cls::Plain
}

/// Laufende bzw. fertige Hervorhebung eines Textes.
pub struct Highlight {
    lines: Arc<Mutex<Vec<Spans>>>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    /// Name der erkannten Sprache (z. B. „Rust“); None = keine passende Grammatik
    pub lang: Arc<Mutex<Option<String>>>,
}

impl Drop for Highlight {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
    }
}

impl Highlight {
    /// Hervorhebung für `text` (Dateiname `name` bestimmt die Sprache) im Hintergrund starten.
    pub fn start(text: Arc<str>, name: &str, ctx: &egui::Context) -> Highlight {
        let h = Highlight { lines: Arc::default(), stop: Arc::default(), done: Arc::default(), lang: Arc::default() };
        let (lines, stop, done, lang, name, ctx) = (h.lines.clone(), h.stop.clone(), h.done.clone(), h.lang.clone(), name.to_string(), ctx.clone());
        std::thread::spawn(move || {
            run(&text, &name, &lines, &stop, &lang, &ctx);
            done.store(true, Relaxed);
            ctx.request_repaint();
        });
        h
    }

    /// Ist die Hervorhebung vollständig berechnet?
    pub fn done(&self) -> bool {
        self.done.load(Relaxed)
    }

    /// Erkannte Sprache (für die Kopfzeile), falls vorhanden.
    pub fn language(&self) -> Option<String> {
        self.lang.lock().ok().and_then(|l| l.clone())
    }

    /// Wie viele Zeilen schon berechnet sind (ändert sich, solange die Hervorhebung läuft).
    pub fn progress(&self) -> usize {
        self.lines.lock().map_or(0, |l| l.len())
    }

    /// Farbabschnitte der Zeile `i`, falls schon berechnet.
    pub fn line(&self, i: usize) -> Option<Spans> {
        self.lines.lock().ok().and_then(|l| l.get(i).cloned())
    }
}

/// Zeilen nacheinander parsen und als Farbabschnitte ablegen; `stop` bricht sofort ab (Datei gewechselt).
fn run(text: &str, name: &str, out: &Arc<Mutex<Vec<Spans>>>, stop: &Arc<AtomicBool>, lang: &Arc<Mutex<Option<String>>>, ctx: &egui::Context) {
    let ss = syntaxes();
    let first_line = text.lines().next().unwrap_or("");
    let Some(syntax) = find(ss, name, first_line) else { return };
    if let Ok(mut l) = lang.lock() {
        *l = Some(syntax.name.clone());
    }
    let mut parse = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut cache: HashMap<Scope, Option<Cls>> = HashMap::new();
    // in Blöcken sammeln und seltener sperren – flüssige Oberfläche, wenig Overhead
    let mut batch: Vec<Spans> = Vec::new();
    for (n, line) in text.split_inclusive('\n').enumerate() {
        if stop.load(Relaxed) {
            return;
        }
        let ops = parse.parse_line(line, ss).unwrap_or_default();
        batch.push(spans_for(line, &ops, &mut stack, &mut cache));
        if batch.len() >= 400 {
            if let Ok(mut o) = out.lock() {
                o.append(&mut batch);
            }
            ctx.request_repaint();
        }
        let _ = n;
    }
    if !batch.is_empty() {
        if let Ok(mut o) = out.lock() {
            o.append(&mut batch);
        }
    }
    ctx.request_repaint();
}

/// Eine Zeile in Farbabschnitte zerlegen (zusammenhängende gleiche Klassen werden verschmolzen).
fn spans_for(line: &str, ops: &[(usize, ScopeStackOp)], stack: &mut ScopeStack, cache: &mut HashMap<Scope, Option<Cls>>) -> Spans {
    let mut spans: Spans = Vec::new();
    let mut pos = 0usize;
    let line_len = line.trim_end_matches('\n').trim_end_matches('\r').len();
    let mut push = |from: usize, to: usize, cls: Cls, spans: &mut Spans| {
        let to = to.min(line_len);
        if from >= to {
            return;
        }
        if let Some(last) = spans.last_mut() {
            if last.2 == cls && last.1 as usize == from {
                last.1 = to as u32;
                return;
            }
        }
        spans.push((from as u32, to as u32, cls));
    };
    for (idx, op) in ops {
        let cls = stack_cls(stack, cache);
        push(pos, *idx, cls, &mut spans);
        pos = *idx;
        stack.apply(op).ok();
    }
    let cls = stack_cls(stack, cache);
    push(pos, line_len, cls, &mut spans);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rust-Quelltext wird erkannt und Schlüsselwörter, Typen, Zeichenketten und Kommentare eingefärbt.
    #[test]
    fn rust_line_is_classified() {
        let ss = syntaxes();
        let syntax = find(ss, "main.rs", "fn main() {}").expect("Rust-Grammatik");
        assert_eq!(syntax.name, "Rust");
        let mut parse = ParseState::new(syntax);
        let mut stack = ScopeStack::new();
        let mut cache = HashMap::new();
        let line = "let s = \"hi\"; // c\n";
        let ops = parse.parse_line(line, ss).unwrap();
        let spans = spans_for(line, &ops, &mut stack, &mut cache);
        let has = |c: Cls| spans.iter().any(|&(_, _, x)| x == c);
        assert!(has(Cls::Keyword), "let = Schlüsselwort");
        assert!(has(Cls::Str), "\"hi\" = Zeichenkette");
        assert!(has(Cls::Comment), "// c = Kommentar");
        // kein Abschnitt reicht über das Zeilenende (ohne \n) hinaus
        let end = line.trim_end().len() as u32;
        assert!(spans.iter().all(|&(_, to, _)| to <= end));
    }

    /// Unbekannte Endungen liefern keine Grammatik (die Vorschau nutzt dann die einfache Hervorhebung).
    #[test]
    fn unknown_extension_has_no_syntax() {
        assert!(find(syntaxes(), "file.zzz", "nichts").is_none());
    }
}
