//! Farbthemes: mitgelieferte Standard-Themes + eigene Themes aus JSON-Dateien.
//!
//! Eigene Themes liegen getrennt von den Standard-Themes in `<config>/foldex/themes/*.json`.
//! Format (Version 1) – Pflicht sind nur `name`, `type`, `colors.background`, `colors.text`
//! und `colors.accent`; alles andere wird daraus bzw. aus dem Standard-Theme gleichen Typs abgeleitet:
//!
//! ```json
//! {
//!   "format": "foldex-theme", "version": 1,
//!   "name": "Mein Theme", "type": "dark",
//!   "colors": { "background": "#202020", "panel": "#272727", "field": "#2d2d2d", "text": "#f0f0f0",
//!               "textMuted": "#a0a0a0", "accent": "#60cdff", "border": "#3a3a3a",
//!               "hover": "#ffffff0e", "selection": "#60cdff28" },
//!   "editor": { "background": "#1c1c1c", "text": "#e8e8e8", "cursor": "#60cdff", "selection": "#60cdff40",
//!               "lineNumbers": "#6e6e6e", "lineHighlight": "#ffffff0a", "findMatch": "#e0b04060" },
//!   "syntax": { "keyword": "#c586c0", "string": "#ce9178", "number": "#b5cea8", "comment": "#6a9955",
//!               "function": "#dcdcaa", "type": "#4ec9b0" }
//! }
//! ```
//! Farben: `#RGB`, `#RRGGBB` oder `#RRGGBBAA` (Alpha nicht vormultipliziert).

use eframe::egui::Color32;
use serde_json::{json, Map, Value};
use std::{fs, path::PathBuf};

pub const FORMAT: &str = "foldex-theme";
pub const VERSION: u64 = 1;
pub const DEFAULT_LIGHT: &str = "foldex-light";
pub const DEFAULT_DARK: &str = "foldex-dark";

/// Alle Farben, mit denen gezeichnet wird. `Copy`, damit sie günstig in jede Zeichenfunktion passen.
#[derive(Clone, Copy, PartialEq)]
pub struct Theme {
    pub dark: bool,
    // Oberfläche
    pub bg: Color32,
    pub panel: Color32,
    pub field: Color32,
    pub text: Color32,
    pub sub: Color32,
    pub accent: Color32,
    pub hover: Color32,
    /// gedrückter Knopf
    pub press: Color32,
    pub sel: Color32,
    pub line: Color32,
    /// Fehler, Warnfarbe (Löschen, Konflikte)
    pub danger: Color32,
    // Editor / Textvorschau
    pub ed_bg: Color32,
    pub ed_text: Color32,
    pub cursor: Color32,
    pub ed_sel: Color32,
    pub line_no: Color32,
    pub line_hl: Color32,
    pub find_hl: Color32,
    // Syntax
    pub keyword: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub function: Color32,
    pub ty: Color32,
}

impl Theme {
    /// Schriftfarbe auf der Akzentfarbe: dunkel bei hellen Akzenten (z. B. Hellblau), sonst weiß
    pub fn on_accent(&self) -> Color32 {
        let c = self.accent;
        let l = 0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32;
        if l > 150.0 {
            Color32::from_rgb(16, 20, 26)
        } else {
            Color32::WHITE
        }
    }
}

#[derive(Clone)]
pub struct Palette {
    /// Standard-Themes: fester Name (z. B. „foldex-dark“); eigene: „custom:<dateiname>“
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub t: Theme,
}

// ───────────────────────────── Farben ─────────────────────────────

pub fn parse_color(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
    let (r, g, b, a) = match h.len() {
        3 => (v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17, 255),
        6 => (v(0, 2)?, v(2, 2)?, v(4, 2)?, 255),
        8 => (v(0, 2)?, v(2, 2)?, v(4, 2)?, v(6, 2)?),
        _ => return None,
    };
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

pub fn hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

fn h(s: &str) -> Color32 {
    parse_color(s).expect("Standard-Theme: ungültige Farbe")
}

fn alpha(c: Color32, a: u8) -> Color32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Lineare Mischung zweier deckender Farben (k = 0 → a, k = 1 → b).
fn mix(a: Color32, b: Color32, k: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn luminance(c: Color32) -> f32 {
    let ch = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
}

/// Kontrastverhältnis nach WCAG (1 … 21). Lesbarer Fließtext braucht mindestens 4,5.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

// ───────────────────────────── Standard-Themes ─────────────────────────────

/// Basisfarben → vollständiges Theme; Hover, Auswahl und Editorfarben werden abgeleitet.
/// `ui` = Hintergrund, Panel, Feld, Text, Text gedämpft, Akzent, Rahmen
/// `syn` = Schlüsselwort, Zeichenkette, Zahl, Kommentar, Funktion, Typ
fn derive(dark: bool, ui: [&str; 7], syn: [&str; 6]) -> Theme {
    let [bg, panel, field, text, sub, accent, line] = ui.map(h);
    let [keyword, string, number, comment, function, ty] = syn.map(h);
    Theme {
        dark,
        bg,
        panel,
        field,
        text,
        sub,
        accent,
        hover: if dark { Color32::from_white_alpha(22) } else { Color32::from_black_alpha(22) },
        press: if dark { Color32::from_white_alpha(34) } else { Color32::from_black_alpha(35) },
        danger: if dark { h("#ff7b72") } else { h("#c42b1c") },
        sel: alpha(accent, if dark { 40 } else { 36 }),
        line,
        ed_bg: bg,
        ed_text: text,
        cursor: accent,
        ed_sel: alpha(accent, if dark { 64 } else { 56 }),
        line_no: mix(sub, bg, 0.3),
        line_hl: if dark { Color32::from_white_alpha(8) } else { Color32::from_black_alpha(8) },
        find_hl: if dark { h("#d9a62e55") } else { h("#f2c94c80") },
        keyword,
        string,
        number,
        comment,
        function,
        ty,
    }
}

pub fn builtins() -> Vec<Palette> {
    let p = |id: &str, name: &str, t: Theme| Palette { id: id.into(), name: name.into(), builtin: true, t };
    // Syntaxfarben je Grundhelligkeit; einzelne Themes ersetzen sie passend zur Stimmung.
    let syn_dark = ["#c792ea", "#c3e88d", "#f78c6c", "#7f8c98", "#82aaff", "#ffcb6b"];
    let syn_light = ["#8a2bb9", "#2f7d32", "#b4530a", "#6a737d", "#1f5fbf", "#9a6700"];
    vec![
        p(DEFAULT_LIGHT, "Foldex Hell", derive(false, ["#ffffff", "#f3f3f3", "#ffffff", "#1b1b1b", "#6b6b6b", "#0067c0", "#e0e0e0"], syn_light)),
        p(DEFAULT_DARK, "Foldex Dunkel", derive(true, ["#191919", "#202020", "#2b2b2b", "#f2f2f2", "#9d9d9d", "#60cdff", "#333333"], syn_dark)),
        p("graphite", "Graphit", derive(true, ["#1b1d21", "#22252a", "#2a2d33", "#e6e8eb", "#9aa1ab", "#6ea8fe", "#33373e"], syn_dark)),
        p(
            "midnight",
            "Mitternacht",
            derive(true, ["#0f1724", "#142033", "#1a2840", "#e3eaf5", "#93a4bd", "#5ccfe6", "#24344d"], ["#c3a6ff", "#bae67e", "#ffae57", "#6c7f99", "#73d0ff", "#ffd580"]),
        ),
        p(
            "forest",
            "Wald",
            derive(true, ["#18201b", "#1e2822", "#24302a", "#e4ece6", "#98aa9e", "#7fd18b", "#2e3b33"], ["#d699b6", "#a7c080", "#e69875", "#7a8b80", "#83c092", "#dbbc7f"]),
        ),
        p(
            "sepia",
            "Sepia",
            derive(false, ["#fbf7ef", "#f2ecdf", "#f6f1e6", "#2d2618", "#6f6450", "#9a5b13", "#e2d8c4"], ["#8f3f71", "#557a1f", "#a6461b", "#7c6f5a", "#2f6493", "#8a6100"]),
        ),
        p("mist", "Nebel", derive(false, ["#f7f9fc", "#eef2f7", "#f2f5f9", "#1c2430", "#5a6676", "#3657c9", "#dde3ec"], syn_light)),
        p(
            "hc-dark",
            "Hoher Kontrast Dunkel",
            derive(true, ["#000000", "#0a0a0a", "#111111", "#ffffff", "#d0d0d0", "#ffd400", "#6b6b6b"], ["#ff9cf4", "#9cff9c", "#ffb86b", "#b8b8b8", "#7fd4ff", "#ffe066"]),
        ),
        p(
            "hc-light",
            "Hoher Kontrast Hell",
            derive(false, ["#ffffff", "#f4f4f4", "#fafafa", "#000000", "#333333", "#0037a6", "#8a8a8a"], ["#7a0099", "#005c00", "#8f3000", "#4d4d4d", "#0033a0", "#6b4a00"]),
        ),
    ]
}

// ───────────────────────────── JSON ─────────────────────────────

/// Ergebnis einer Prüfung: Theme oder verständliche Fehlerliste; Warnungen verhindern den Import nicht.
pub struct Checked {
    pub palette: Result<Palette, Vec<String>>,
    pub warnings: Vec<String>,
}

const UI_KEYS: [&str; 9] = ["background", "panel", "field", "text", "textMuted", "accent", "border", "hover", "selection"];
const ED_KEYS: [&str; 7] = ["background", "text", "cursor", "selection", "lineNumbers", "lineHighlight", "findMatch"];
const SYN_KEYS: [&str; 6] = ["keyword", "string", "number", "comment", "function", "type"];

pub fn to_json(p: &Palette) -> Value {
    let t = &p.t;
    json!({
        "format": FORMAT,
        "version": VERSION,
        "name": p.name,
        "type": if t.dark { "dark" } else { "light" },
        "colors": {
            "background": hex(t.bg), "panel": hex(t.panel), "field": hex(t.field), "text": hex(t.text),
            "textMuted": hex(t.sub), "accent": hex(t.accent), "border": hex(t.line),
            "hover": hex(t.hover), "selection": hex(t.sel)
        },
        "editor": {
            "background": hex(t.ed_bg), "text": hex(t.ed_text), "cursor": hex(t.cursor), "selection": hex(t.ed_sel),
            "lineNumbers": hex(t.line_no), "lineHighlight": hex(t.line_hl), "findMatch": hex(t.find_hl)
        },
        "syntax": {
            "keyword": hex(t.keyword), "string": hex(t.string), "number": hex(t.number),
            "comment": hex(t.comment), "function": hex(t.function), "type": hex(t.ty)
        }
    })
}

/// Prüft ein Theme-JSON. `id` wird erst beim Speichern vergeben.
pub fn from_json(v: &Value) -> Checked {
    let mut errs = Vec::new();
    let mut warns = Vec::new();
    let Some(obj) = v.as_object() else {
        return Checked { palette: Err(vec!["Die Datei enthält kein JSON-Objekt ({ … }).".into()]), warnings: warns };
    };
    match obj.get("format").and_then(Value::as_str) {
        // „filecore-theme“: Dateien aus der Zeit vor der Umbenennung in Foldex
        Some(FORMAT) | Some("filecore-theme") => {}
        Some(other) => errs.push(format!("„format“ ist „{other}“, erwartet wird „{FORMAT}“.")),
        None => errs.push(format!("„format“ fehlt – ist das eine Foldex-Theme-Datei? (erwartet: \"format\": \"{FORMAT}\")")),
    }
    match obj.get("version").map(|v| v.as_u64()) {
        Some(Some(n)) if n > VERSION => errs.push(format!("Theme-Version {n} ist neuer als diese Foldex-Version (unterstützt bis {VERSION}).")),
        Some(Some(0)) | Some(None) => errs.push("„version“ muss eine positive ganze Zahl sein.".into()),
        Some(Some(_)) => {}
        None => errs.push("„version“ fehlt.".into()),
    }
    let name = match obj.get("name").and_then(Value::as_str).map(str::trim) {
        Some(n) if !n.is_empty() && n.chars().count() <= 60 => n.to_string(),
        Some(n) if !n.is_empty() => {
            errs.push("„name“ ist länger als 60 Zeichen.".into());
            String::new()
        }
        _ => {
            errs.push("„name“ fehlt oder ist leer.".into());
            String::new()
        }
    };
    let dark = match obj.get("type").and_then(Value::as_str) {
        Some("dark") => true,
        Some("light") => false,
        Some(o) => {
            errs.push(format!("„type“ ist „{o}“, erlaubt sind „dark“ und „light“."));
            true
        }
        None => {
            errs.push("„type“ fehlt (\"dark\" oder \"light\").".into());
            true
        }
    };
    for k in obj.keys() {
        if !["format", "version", "name", "type", "colors", "editor", "syntax", "author", "description"].contains(&k.as_str()) {
            warns.push(format!("Unbekannter Eintrag „{k}“ wird ignoriert."));
        }
    }
    // Abschnitt lesen: fehlende Felder → None, ungültige Farben → Fehler
    let mut section = |name: &str, keys: &[&str], required: &[&str]| -> Vec<Option<Color32>> {
        let empty = Map::new();
        let sec = match obj.get(name) {
            Some(Value::Object(m)) => m,
            Some(_) => {
                errs.push(format!("„{name}“ muss ein Objekt sein."));
                &empty
            }
            None => &empty,
        };
        for k in sec.keys() {
            if !keys.contains(&k.as_str()) {
                warns.push(format!("Unbekannte Farbe „{name}.{k}“ wird ignoriert (bekannt: {}).", keys.join(", ")));
            }
        }
        keys.iter()
            .map(|k| match sec.get(*k) {
                None if required.contains(k) => {
                    errs.push(format!("Pflichtfarbe „{name}.{k}“ fehlt."));
                    None
                }
                None => None,
                Some(Value::String(s)) => parse_color(s).or_else(|| {
                    errs.push(format!("„{name}.{k}“: „{s}“ ist keine gültige Farbe (erwartet #RRGGBB oder #RRGGBBAA)."));
                    None
                }),
                Some(_) => {
                    errs.push(format!("„{name}.{k}“ muss eine Zeichenkette wie \"#1e1e1e\" sein."));
                    None
                }
            })
            .collect()
    };
    let ui = section("colors", &UI_KEYS, &["background", "text", "accent"]);
    let ed = section("editor", &ED_KEYS, &[]);
    let syn = section("syntax", &SYN_KEYS, &[]);
    if !errs.is_empty() {
        return Checked { palette: Err(errs), warnings: warns };
    }

    // Fehlendes aus den Pflichtfarben bzw. dem Standard-Theme gleichen Typs ableiten
    let base = builtins().into_iter().find(|p| p.id == if dark { DEFAULT_DARK } else { DEFAULT_LIGHT }).unwrap().t;
    let (bg, text, accent) = (ui[0].unwrap(), ui[3].unwrap(), ui[5].unwrap());
    let or = |c: Option<Color32>, d: Color32| c.unwrap_or(d);
    let t = Theme {
        dark,
        bg,
        panel: or(ui[1], mix(bg, text, 0.035)),
        field: or(ui[2], mix(bg, text, 0.06)),
        text,
        sub: or(ui[4], mix(text, bg, 0.38)),
        accent,
        line: or(ui[6], mix(bg, text, 0.13)),
        hover: or(ui[7], alpha(text, 22)),
        press: alpha(text, 34),
        danger: base.danger,
        sel: or(ui[8], alpha(accent, 40)),
        ed_bg: or(ed[0], bg),
        ed_text: or(ed[1], text),
        cursor: or(ed[2], accent),
        ed_sel: or(ed[3], alpha(accent, 64)),
        line_no: or(ed[4], mix(text, bg, 0.55)),
        line_hl: or(ed[5], alpha(text, 8)),
        find_hl: or(ed[6], base.find_hl),
        keyword: or(syn[0], base.keyword),
        string: or(syn[1], base.string),
        number: or(syn[2], base.number),
        comment: or(syn[3], base.comment),
        function: or(syn[4], base.function),
        ty: or(syn[5], base.ty),
    };
    let checks = [
        ("Text auf Hintergrund", t.text, t.bg, 4.5),
        ("Gedämpfter Text auf Hintergrund", t.sub, t.bg, 3.0),
        ("Editor-Text auf Editor-Hintergrund", t.ed_text, t.ed_bg, 4.5),
        ("Akzent auf Hintergrund", t.accent, t.bg, 2.0),
    ];
    for (what, a, b, min) in checks {
        let c = contrast(a, b);
        if c < min {
            warns.push(format!("Geringer Kontrast: {what} {c:.1}:1 (empfohlen mindestens {min}:1)."));
        }
    }
    if luminance(bg) > 0.4 && dark {
        warns.push("„type“ ist „dark“, der Hintergrund ist aber hell.".into());
    } else if luminance(bg) < 0.2 && !dark {
        warns.push("„type“ ist „light“, der Hintergrund ist aber dunkel.".into());
    }
    Checked { palette: Ok(Palette { id: String::new(), name, builtin: false, t }), warnings: warns }
}

/// Liest und prüft eine Theme-Datei; Lese- und Syntaxfehler werden in verständliche Meldungen übersetzt.
pub fn check_file(path: &std::path::Path) -> Checked {
    let fail = |m: String| Checked { palette: Err(vec![m]), warnings: vec![] };
    let text = match fs::read(path) {
        Ok(b) if b.len() > 256 * 1024 => return fail("Die Datei ist größer als 256 KB – das ist kein Theme.".into()),
        Ok(b) => match String::from_utf8(b) {
            Ok(s) => s,
            Err(_) => return fail("Die Datei ist kein UTF-8-Text.".into()),
        },
        Err(e) => return fail(format!("Datei kann nicht gelesen werden: {e}")),
    };
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(v) => from_json(&v),
        Err(e) => fail(format!("Kein gültiges JSON (Zeile {}, Spalte {}): {e}", e.line(), e.column())),
    }
}

// ───────────────────────────── Speicher ─────────────────────────────

pub fn custom_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("themes")
}

/// Schreibt erst in eine temporäre Datei und benennt dann um: ein Absturz hinterlässt nie eine halbe Datei.
pub fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    // Cloud-Laufwerk: direkt schreiben – rclone lädt erst nach dem Schließen hoch, ein Umbenennen über die
    // Datei hinweg kollidierte dort mit dem laufenden Upload
    if crate::trashx::on_cloud(path) {
        return fs::write(path, data);
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    // Umbenennen klappt nicht immer (z. B. umgeleitete Ordner: „anderes Laufwerk“) – dann direkt schreiben
    if fs::rename(&tmp, path).is_err() {
        let r = fs::write(path, data);
        let _ = fs::remove_file(&tmp);
        return r;
    }
    Ok(())
}

/// ID, unter der ein eigenes Theme mit diesem Namen gespeichert wird (Dateiname = Teil nach „custom:“).
pub fn custom_id(name: &str) -> String {
    let slug = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("custom:{}", if slug.is_empty() { "theme" } else { &slug })
}

pub struct Store {
    pub builtin: Vec<Palette>,
    pub custom: Vec<Palette>,
    /// Eigene Theme-Dateien, die beim Laden ungültig waren: (Datei, erste Fehlermeldung)
    pub broken: Vec<(String, String)>,
}

impl Store {
    pub fn load() -> Self {
        let mut s = Store { builtin: builtins(), custom: Vec::new(), broken: Vec::new() };
        if let Ok(rd) = fs::read_dir(custom_dir()) {
            let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
            files.sort();
            for f in files {
                let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                match check_file(&f).palette {
                    Ok(mut p) => {
                        p.id = format!("custom:{stem}");
                        s.custom.push(p);
                    }
                    Err(e) => s.broken.push((stem, e.into_iter().next().unwrap_or_default())),
                }
            }
        }
        s
    }

    pub fn get(&self, id: &str) -> Option<&Palette> {
        self.builtin.iter().chain(&self.custom).find(|p| p.id == id)
    }

    /// Theme für die gewünschte Helligkeit; unbekannte oder unpassende IDs fallen auf den Standard zurück.
    pub fn resolve(&self, id: &str, dark: bool) -> Theme {
        match self.get(id) {
            Some(p) if p.t.dark == dark => p.t,
            _ => self.get(if dark { DEFAULT_DARK } else { DEFAULT_LIGHT }).unwrap().t,
        }
    }

    /// Speichert ein geprüftes Theme als eigenes Theme (gleicher Name → wird ersetzt). Gibt die ID zurück.
    pub fn add_custom(&mut self, mut p: Palette) -> Result<String, String> {
        p.id = custom_id(&p.name);
        let slug = p.id["custom:".len()..].to_string();
        p.builtin = false;
        let data = serde_json::to_vec_pretty(&to_json(&p)).map_err(|e| e.to_string())?;
        write_atomic(&custom_dir().join(format!("{slug}.json")), &data).map_err(|e| format!("Theme konnte nicht gespeichert werden: {e}"))?;
        self.custom.retain(|c| c.id != p.id);
        self.broken.retain(|(s, _)| *s != slug);
        let id = p.id.clone();
        self.custom.push(p);
        Ok(id)
    }

    pub fn remove_custom(&mut self, id: &str) -> Result<(), String> {
        let Some(slug) = id.strip_prefix("custom:") else { return Err("Standard-Themes können nicht gelöscht werden.".into()) };
        match fs::remove_file(custom_dir().join(format!("{slug}.json"))) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Theme-Datei konnte nicht entfernt werden: {e}")),
        }
        self.custom.retain(|p| p.id != id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff"), Some(Color32::WHITE));
        assert_eq!(parse_color("#000000"), Some(Color32::BLACK));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("123456"), None);
        // Halbtransparente Farben speichert egui vormultipliziert (±1 Rundung) – Export/Import muss aber stabil sein
        let once = hex(h("#60cdff28"));
        assert_eq!(hex(h(&once)), once);
    }

    #[test]
    fn builtins_roundtrip_and_contrast() {
        for p in builtins() {
            let c = from_json(&to_json(&p));
            let back = c.palette.unwrap_or_else(|e| panic!("{}: {e:?}", p.name));
            assert_eq!(back.t.dark, p.t.dark);
            assert_eq!(hex(back.t.text), hex(p.t.text));
            assert!(contrast(p.t.text, p.t.bg) >= 7.0, "{}", p.name);
            assert!(contrast(p.t.sub, p.t.bg) >= 4.5, "{}", p.name);
            assert!(c.warnings.is_empty(), "{}: {:?}", p.name, c.warnings);
        }
    }

    #[test]
    fn minimal_and_invalid() {
        let ok = from_json(&json!({"format": FORMAT, "version": 1, "name": "X", "type": "dark",
            "colors": {"background": "#101010", "text": "#eeeeee", "accent": "#66aaff"}}));
        assert!(ok.palette.is_ok());
        let bad = from_json(&json!({"format": FORMAT, "version": 9, "type": "grau",
            "colors": {"background": "rot", "text": 5}}));
        let errs = bad.palette.err().unwrap();
        assert!(errs.len() >= 5, "{errs:?}");
    }
}
