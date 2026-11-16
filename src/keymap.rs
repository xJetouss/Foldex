//! Alle Befehle an einer Stelle: Command Palette, Menüs und Tastenkürzel nutzen dieselbe Liste.
//! Eigene Tastenkürzel überschreiben die Standardbelegung und werden als versioniertes JSON gespeichert.

use crate::ui::Ic;
use eframe::egui::Key;
use serde_json::{json, Map, Value};
use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Shortcut {
    /// Speicherformat: „Ctrl+Shift+N“, „F2“, „Alt+ArrowLeft“ (Tastennamen von egui).
    pub fn parse(s: &str) -> Option<Shortcut> {
        let mut sc = Shortcut { ctrl: false, shift: false, alt: false, key: Key::A };
        let mut key = None;
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "strg" | "cmd" => sc.ctrl = true,
                "shift" | "umschalt" => sc.shift = true,
                "alt" => sc.alt = true,
                _ => key = Key::from_name(part),
            }
        }
        sc.key = key?;
        Some(sc)
    }

    pub fn store(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        s.push_str(self.key.name());
        s
    }

    /// Anzeige: „Strg+Umschalt+N“, „Alt+←“.
    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Strg+");
        }
        if self.shift {
            s.push_str("Umschalt+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        s.push_str(match self.key {
            Key::ArrowLeft => "←",
            Key::ArrowRight => "→",
            Key::ArrowUp => "↑",
            Key::ArrowDown => "↓",
            Key::Space => "Leertaste",
            Key::Delete => "Entf",
            Key::Enter => "Enter",
            Key::Escape => "Esc",
            Key::Backspace => "Rück",
            Key::Comma => ",",
            Key::Tab => "Tab",
            k => k.name(),
        });
        s
    }

    /// Kürzel, die fest vergeben sind und nicht neu belegt werden können.
    pub fn reserved(&self) -> bool {
        let plain = !self.ctrl && !self.alt;
        (plain && matches!(self.key, Key::Enter | Key::Escape | Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight | Key::Home | Key::End | Key::PageUp | Key::PageDown | Key::Backspace | Key::Tab | Key::Space))
            || (plain && !self.shift && self.key.name().len() == 1) // Buchstaben/Ziffern = Tippen zum Springen
            || (self.ctrl && !self.shift && !self.alt && matches!(self.key, Key::C | Key::X | Key::V))
    }
}

pub struct Cmd {
    pub id: &'static str,
    pub label: &'static str,
    pub cat: &'static str,
    pub default: &'static str,
    pub ic: Option<Ic>,
    /// fest belegt (Kopieren/Ausschneiden/Einfügen), nur zur Anzeige
    pub fixed: bool,
}

const fn c(id: &'static str, cat: &'static str, label: &'static str, default: &'static str, ic: Option<Ic>) -> Cmd {
    Cmd { id, label, cat, default, ic, fixed: false }
}

pub const CMDS: &[Cmd] = &[
    // Navigation
    c("go.back", "Navigation", "Zurück", "Alt+ArrowLeft", Some(Ic::Back)),
    c("go.forward", "Navigation", "Vorwärts", "Alt+ArrowRight", Some(Ic::Forward)),
    c("go.up", "Navigation", "Übergeordneter Ordner", "Alt+ArrowUp", Some(Ic::Up)),
    c("go.home", "Navigation", "Benutzerordner öffnen", "Alt+Home", Some(Ic::Home)),
    c("go.address", "Navigation", "Adressleiste bearbeiten", "Ctrl+L", None),
    c("go.search", "Navigation", "In diesem Ordner suchen", "Ctrl+F", Some(Ic::Search)),
    c("go.refresh", "Navigation", "Aktualisieren", "F5", Some(Ic::Refresh)),
    // Tabs
    c("tab.new", "Tabs", "Neuer Tab", "Ctrl+T", Some(Ic::Plus)),
    c("tab.close", "Tabs", "Tab schließen", "Ctrl+W", Some(Ic::Close)),
    c("tab.reopen", "Tabs", "Geschlossenen Tab wieder öffnen", "Ctrl+Shift+T", Some(Ic::Undo)),
    c("tab.next", "Tabs", "Nächster Tab", "Ctrl+Tab", Some(Ic::ChevR)),
    c("tab.prev", "Tabs", "Vorheriger Tab", "Ctrl+Shift+Tab", None),
    c("tab.dup", "Tabs", "Tab duplizieren", "", Some(Ic::Copy)),
    c("tab.detach", "Tabs", "Tab in neues Fenster verschieben", "", Some(Ic::Window)),
    c("tile.newtab", "Tabs", "Neuer Tab in dieser Kachel", "Ctrl+Alt+T", Some(Ic::Plus)),
    c("tile.next", "Tabs", "Nächster Tab in der Kachel", "Ctrl+PageDown", Some(Ic::ChevR)),
    c("tile.prev", "Tabs", "Vorheriger Tab in der Kachel", "Ctrl+PageUp", None),
    c("tile.close", "Tabs", "Tab in der Kachel schließen", "Ctrl+Alt+W", Some(Ic::Close)),
    // Fenster
    c("win.new", "Fenster", "Neues Fenster", "Ctrl+N", Some(Ic::Window)),
    c("win.split", "Fenster", "Ansicht teilen / Teilung aufheben", "Ctrl+Shift+D", None),
    c("win.pane", "Fenster", "Zwischen geteilten Ansichten wechseln", "F6", None),
    c("win.sidebar", "Fenster", "Seitenleiste ein-/ausblenden", "Ctrl+B", Some(Ic::Sidebar)),
    // Dateien
    Cmd { id: "file.copy", label: "Kopieren", cat: "Dateien", default: "Ctrl+C", ic: Some(Ic::Copy), fixed: true },
    Cmd { id: "file.cut", label: "Ausschneiden", cat: "Dateien", default: "Ctrl+X", ic: Some(Ic::Cut), fixed: true },
    Cmd { id: "file.paste", label: "Einfügen", cat: "Dateien", default: "Ctrl+V", ic: Some(Ic::Paste), fixed: true },
    c("file.newfolder", "Dateien", "Neuer Ordner", "Ctrl+Shift+N", Some(Ic::NewFolder)),
    c("file.newfile", "Dateien", "Neue Textdatei", "", Some(Ic::Docs)),
    c("file.rename", "Dateien", "Umbenennen", "F2", Some(Ic::Rename)),
    c("file.trash", "Dateien", "In den Papierkorb", "Delete", Some(Ic::Trash)),
    c("file.delete", "Dateien", "Endgültig löschen", "Shift+Delete", Some(Ic::Trash)),
    c("file.selectall", "Dateien", "Alles auswählen", "Ctrl+A", Some(Ic::Select)),
    c("file.selectrules", "Dateien", "Auswahl nach Regeln …", "Ctrl+Shift+A", Some(Ic::Select)),
    c("file.invert", "Dateien", "Auswahl umkehren", "", Some(Ic::Select)),
    c("file.undo", "Dateien", "Rückgängig", "Ctrl+Z", Some(Ic::Undo)),
    c("file.redo", "Dateien", "Wiederholen", "Ctrl+Y", Some(Ic::Redo)),
    c("file.copypath", "Dateien", "Vollständigen Pfad kopieren", "Ctrl+Shift+C", Some(Ic::Copy)),
    c("file.props", "Dateien", "Eigenschaften", "Alt+Enter", Some(Ic::Info)),
    c("file.share", "Dateien", "Teilen … (LocalSend, Upload-Links, Messenger)", "Ctrl+Shift+S", Some(Ic::Share)),
    c("file.print", "Dateien", "Drucken", "Ctrl+P", Some(Ic::Docs)),
    c("file.vscode", "Dateien", "In Visual Studio Code öffnen", "", Some(Ic::Code)),
    c("file.zed", "Dateien", "In Zed öffnen", "", Some(Ic::Code)),
    c("file.openwith", "Dateien", "Öffnen mit …", "", Some(Ic::Open)),
    // Ansicht
    c("view.details", "Ansicht", "Ansicht: Details", "Ctrl+1", Some(Ic::List)),
    c("view.list", "Ansicht", "Ansicht: Liste", "Ctrl+2", Some(Ic::List)),
    c("view.grid", "Ansicht", "Ansicht: Raster", "Ctrl+3", Some(Ic::Grid)),
    c("view.large", "Ansicht", "Ansicht: Große Symbole", "Ctrl+4", Some(Ic::Grid)),
    c("view.hidden", "Ansicht", "Versteckte Dateien ein/aus", "Ctrl+H", Some(Ic::Eye)),
    c("view.preview", "Ansicht", "Vorschau ein/aus", "Alt+P", Some(Ic::Docs)),
    c("view.start", "Ansicht", "Startseite öffnen", "", Some(Ic::Home)),
    // Werkzeuge
    c("tools.compare", "Werkzeuge", "Ordner vergleichen", "", Some(Ic::Compare)),
    c("tools.dups", "Werkzeuge", "Duplikate finden", "", Some(Ic::Copy)),
    c("tools.space", "Werkzeuge", "Speicherplatz analysieren", "", Some(Ic::Chart)),
    c("tools.large", "Werkzeuge", "Große Dateien anzeigen", "", Some(Ic::Chart)),
    // Terminal
    c("term.new", "Terminal", "Terminal-Tab hier öffnen", "Alt+T", Some(Ic::Terminal)),
    // KI
    c("ai.assistant", "KI", "Explorer Assistent ein/aus (Seitenleiste)", "Ctrl+Shift+I", Some(Ic::Spark)),
    c("ai.agent", "KI", "Explorer Agent ein/aus (bedient Tabs und Dateien)", "Ctrl+Shift+K", Some(Ic::Spark)),
    c("ai.chat", "KI", "KI-Chat ein/aus (Seitenleiste, ohne Zugriff auf Explorer und Dateien)", "Ctrl+Shift+J", Some(Ic::Spark)),
    // Git
    c("git.view", "Git", "Git-Repository anzeigen", "Ctrl+Shift+G", Some(Ic::Git)),
    c("git.refresh", "Git", "Git-Status aktualisieren", "", Some(Ic::Refresh)),
    c("git.pull", "Git", "Pull", "", Some(Ic::Download)),
    c("git.push", "Git", "Push", "", Some(Ic::Push)),
    c("git.fetch", "Git", "Fetch", "", Some(Ic::Refresh)),
    // Programm
    c("app.palette", "Programm", "Command Palette", "Ctrl+Space", Some(Ic::Command)),
    c("app.settings", "Programm", "Einstellungen", "Ctrl+Comma", Some(Ic::Gear)),
    c("app.keys", "Programm", "Tastenkürzel anpassen", "", Some(Ic::Keyboard)),
    c("app.clearrecent", "Programm", "Verlauf zuletzt verwendeter Ordner löschen", "", Some(Ic::Clock)),
];

pub fn cmd(id: &str) -> Option<&'static Cmd> {
    CMDS.iter().find(|c| c.id == id)
}

fn file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("keys.json")
}

/// Aktive Belegung = Standard + eigene Änderungen (`None` = Kürzel entfernt).
#[derive(Clone, Default, PartialEq)]
pub struct Keymap {
    pub custom: HashMap<String, Option<Shortcut>>,
}

impl Keymap {
    pub fn load() -> Keymap {
        fs::read_to_string(file())
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| Keymap::from_json(&v).ok())
            .map(|(k, _)| k)
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let data = serde_json::to_vec_pretty(&self.to_json()).unwrap_or_default();
        let _ = crate::themes::write_atomic(&file(), &data);
    }

    pub fn get(&self, id: &str) -> Option<Shortcut> {
        match self.custom.get(id) {
            Some(o) => *o,
            None => cmd(id).and_then(|c| Shortcut::parse(c.default)),
        }
    }

    pub fn label(&self, id: &str) -> String {
        self.get(id).map(|s| s.label()).unwrap_or_default()
    }

    pub fn lookup(&self, sc: Shortcut) -> Option<&'static str> {
        CMDS.iter().find(|c| !c.fixed && self.get(c.id) == Some(sc)).map(|c| c.id)
    }

    /// Befehl, der dieses Kürzel schon benutzt (außer `except`).
    pub fn conflict(&self, sc: Shortcut, except: &str) -> Option<&'static Cmd> {
        CMDS.iter().find(|c| c.id != except && self.get(c.id) == Some(sc))
    }

    pub fn set(&mut self, id: &str, sc: Option<Shortcut>) {
        let default = cmd(id).and_then(|c| Shortcut::parse(c.default));
        if sc == default {
            self.custom.remove(id);
        } else {
            self.custom.insert(id.to_string(), sc);
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        let mut keys: Vec<&String> = self.custom.keys().collect();
        keys.sort();
        for k in keys {
            m.insert(k.clone(), self.custom[k].map_or(Value::Null, |s| Value::String(s.store())));
        }
        json!({ "version": 1, "bindings": m })
    }

    /// Prüft eine Belegung (aus keys.json oder einem Einstellungs-Export).
    pub fn from_json(v: &Value) -> Result<(Keymap, Vec<String>), Vec<String>> {
        let mut errs = Vec::new();
        let mut warns = Vec::new();
        if v.get("version").and_then(Value::as_u64).is_some_and(|n| n > 1) {
            return Err(vec!["Tastenkürzel stammen aus einer neueren Foldex-Version.".into()]);
        }
        let mut k = Keymap::default();
        let Some(b) = v.get("bindings").and_then(Value::as_object) else {
            return Err(vec!["„bindings“ fehlt oder ist kein Objekt.".into()]);
        };
        for (id, val) in b {
            let Some(c) = cmd(id) else {
                warns.push(format!("Unbekannter Befehl „{id}“ wird ignoriert."));
                continue;
            };
            if c.fixed {
                warns.push(format!("„{}“ ist fest belegt und wird ignoriert.", c.label));
                continue;
            }
            match val {
                Value::Null => {
                    k.custom.insert(id.clone(), None);
                }
                Value::String(s) if s.is_empty() => {
                    k.custom.insert(id.clone(), None);
                }
                Value::String(s) => match Shortcut::parse(s) {
                    Some(sc) if sc.reserved() => errs.push(format!("„{s}“ für „{}“ ist reserviert.", c.label)),
                    Some(sc) => {
                        k.custom.insert(id.clone(), Some(sc));
                    }
                    None => errs.push(format!("„{s}“ für „{}“ ist kein gültiges Tastenkürzel.", c.label)),
                },
                _ => errs.push(format!("Tastenkürzel für „{id}“ muss ein Text oder null sein.")),
            }
        }
        // doppelte Belegungen
        let mut seen: HashMap<Shortcut, &str> = HashMap::new();
        for c in CMDS {
            if let Some(sc) = k.get(c.id) {
                if let Some(other) = seen.insert(sc, c.label) {
                    errs.push(format!("„{}“ ist doppelt belegt: „{other}“ und „{}“.", sc.label(), c.label));
                }
            }
        }
        if errs.is_empty() {
            Ok((k, warns))
        } else {
            Err(errs)
        }
    }
}

/// Unscharfe Suche für die Palette: alle Zeichen in Reihenfolge; Wortanfänge und Nähe zählen mehr.
pub fn fuzzy(hay: &str, needle: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let h: Vec<char> = hay.to_lowercase().chars().collect();
    let mut score = 0;
    let mut pos = 0usize;
    let mut last: Option<usize> = None;
    for n in needle.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let i = (pos..h.len()).find(|&i| h[i] == n)?;
        score += 10;
        if i == 0 || !h[i - 1].is_alphanumeric() {
            score += 15;
        }
        if last.is_some_and(|l| l + 1 == i) {
            score += 12;
        }
        score -= (i - pos).min(10) as i32;
        last = Some(i);
        pos = i + 1;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_unique_and_parse() {
        let k = Keymap::default();
        let mut seen = std::collections::HashSet::new();
        for c in CMDS {
            if !c.default.is_empty() {
                let sc = Shortcut::parse(c.default).unwrap_or_else(|| panic!("{}", c.default));
                assert!(seen.insert(sc), "doppelt: {}", c.default);
                assert_eq!(Shortcut::parse(&sc.store()), Some(sc));
            }
        }
        assert_eq!(k.lookup(Shortcut::parse("Ctrl+T").unwrap()), Some("tab.new"));
        assert_eq!(k.lookup(Shortcut::parse("Ctrl+C").unwrap()), None); // fest belegt, läuft über Ereignisse
    }

    #[test]
    fn conflicts_and_json() {
        let mut k = Keymap::default();
        let t = Shortcut::parse("Ctrl+T").unwrap();
        assert_eq!(k.conflict(t, "win.new").map(|c| c.id), Some("tab.new"));
        k.set("tab.new", None);
        k.set("win.new", Some(t));
        assert_eq!(k.lookup(t), Some("win.new"));
        let (back, _) = Keymap::from_json(&k.to_json()).unwrap();
        assert!(back == k);
        let bad = json!({"version": 1, "bindings": {"tab.new": "Ctrl+N"}});
        assert!(Keymap::from_json(&bad).is_err()); // Strg+N gehört schon zu „Neues Fenster“
        assert!(fuzzy("Neuer Tab", "ntab").is_some());
        assert!(fuzzy("Neuer Tab", "xyz").is_none());
    }
}
