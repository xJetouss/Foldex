//! Startseite: erscheint, wenn alle Tabs geschlossen sind (oder per „Startseite“). Kacheln für Schnellzugriff,
//! Cloud, Dieser PC, zuletzt verwendete Ordner und eigene Links – alles ein-/ausblendbar, sortierbar, in drei
//! Größen. Die Einstellungen liegen in `%APPDATA%\foldex\start.json`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Section {
    pub key: String,
    pub on: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Link {
    pub label: String,
    pub path: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct StartCfg {
    /// Reihenfolge und Sichtbarkeit der Bereiche
    pub sections: Vec<Section>,
    /// Kachelgröße: 0 klein, 1 mittel, 2 groß
    pub tile: u8,
    /// Begrüßung mit Datum und Uhrzeit
    pub greeting: bool,
    /// Suchfeld (ganzer PC)
    pub search: bool,
    /// Speicherbalken bei Laufwerken und Cloud
    pub space: bool,
    /// alt (ersetzt durch `new_tab`)
    #[serde(skip_serializing)]
    pub new_tab_start: bool,
    /// neuer Tab öffnet: 0 Benutzerordner, 1 Startseite, 2 aktuellen Ordner
    pub new_tab: u8,
    /// letzten Tab schließen: 0 Startseite, 1 Benutzerordner
    pub close_last: u8,
    /// Foldex startet mit der Startseite
    pub on_launch: bool,
    /// „Start“ oben in der Seitenleiste
    pub sidebar_entry: bool,
    /// Name in der Begrüßung („Guten Tag, …“)
    pub name: String,
    /// Datum bzw. Uhrzeit unter der Begrüßung
    pub date: bool,
    pub clock: bool,
    /// Darstellung: 0 Kacheln, 1 Liste
    pub style: u8,
    /// Spalten: 0 automatisch, sonst fest
    pub columns: u8,
    /// Kacheln leicht in der Akzentfarbe
    pub colorful: bool,
    /// Überschriften über den Bereichen
    pub headings: bool,
    /// Hintergrund: 0 schlicht, 1 Verlauf, 2 Bild
    pub background: u8,
    pub bg_image: Option<PathBuf>,
    /// Bild abdunkeln (0 … 0,9)
    pub bg_dim: f32,
    /// ausgeblendete Kacheln
    pub hidden: Vec<PathBuf>,
    /// eigene Links
    pub links: Vec<Link>,
    /// höchstens so viele zuletzt verwendete Ordner
    pub recent_max: usize,
}

/// (Schlüssel, Überschrift) aller Bereiche in der Standard-Reihenfolge
pub const SECTIONS: [(&str, &str); 5] = [("pins", "Schnellzugriff"), ("cloud", "Cloud"), ("pc", "Dieser PC"), ("recent", "Zuletzt verwendet"), ("links", "Eigene Links")];

pub fn title(key: &str) -> &'static str {
    SECTIONS.iter().find(|(k, _)| *k == key).map_or("", |(_, t)| t)
}

impl Default for StartCfg {
    fn default() -> Self {
        StartCfg {
            sections: SECTIONS.iter().map(|(k, _)| Section { key: k.to_string(), on: true }).collect(),
            tile: 1,
            greeting: true,
            search: true,
            space: true,
            new_tab_start: false,
            new_tab: 0,
            close_last: 0,
            on_launch: false,
            sidebar_entry: true,
            name: String::new(),
            date: true,
            clock: true,
            style: 0,
            columns: 0,
            colorful: false,
            headings: true,
            background: 0,
            bg_image: None,
            bg_dim: 0.45,
            hidden: Vec::new(),
            links: Vec::new(),
            recent_max: 8,
        }
    }
}

fn file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("start.json")
}

impl StartCfg {
    pub fn load() -> Self {
        let mut c: StartCfg = std::fs::read_to_string(file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        // neue Bereiche aus späteren Versionen ergänzen, unbekannte entfernen
        c.sections.retain(|s| SECTIONS.iter().any(|(k, _)| *k == s.key));
        for (k, _) in SECTIONS {
            if !c.sections.iter().any(|s| s.key == k) {
                c.sections.push(Section { key: k.to_string(), on: true });
            }
        }
        c.tile = c.tile.min(2);
        if c.new_tab_start {
            c.new_tab = 1;
            c.new_tab_start = false;
        }
        c.bg_dim = c.bg_dim.clamp(0.0, 0.9);
        c
    }

    pub fn save(&self) {
        let _ = crate::themes::write_atomic(&file(), serde_json::to_string_pretty(self).unwrap_or_default().as_bytes());
    }

    /// Kachelgröße in Punkten
    pub fn tile_size(&self) -> (f32, f32) {
        if self.style == 1 {
            return match self.tile {
                0 => (280.0, 38.0),
                2 => (380.0, 58.0),
                _ => (330.0, 46.0),
            };
        }
        match self.tile {
            0 => (168.0, 52.0),
            2 => (260.0, 92.0),
            _ => (210.0, 70.0),
        }
    }
}

/// Begrüßung (mit Name) und Zeile darunter (Datum und/oder Uhrzeit) nach den Einstellungen
pub fn header(c: &StartCfg, now: &chrono::DateTime<chrono::Local>) -> (String, String) {
    use chrono::Timelike;
    let (g, d) = greeting(now);
    let g = if c.name.trim().is_empty() { g } else { format!("{g}, {}", c.name.trim()) };
    let date = d.split(" · ").next().unwrap_or("").to_string();
    let time = format!("{:02}:{:02}", now.hour(), now.minute());
    let line = match (c.date, c.clock) {
        (true, true) => format!("{date} · {time}"),
        (true, false) => date,
        (false, true) => time,
        _ => String::new(),
    };
    (g, line)
}

/// „Guten Morgen“ usw. und das Datum auf Deutsch
pub fn greeting(now: &chrono::DateTime<chrono::Local>) -> (String, String) {
    use chrono::{Datelike, Timelike};
    let g = match now.hour() {
        5..=10 => "Guten Morgen",
        11..=17 => "Guten Tag",
        _ => "Guten Abend",
    };
    const WD: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];
    const MO: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
    let d = format!("{}, {}. {} {} · {:02}:{:02}", WD[now.weekday().num_days_from_monday() as usize], now.day(), MO[now.month0() as usize], now.year(), now.hour(), now.minute());
    (g.to_string(), d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_upgrade() {
        let c = StartCfg::default();
        assert_eq!(c.sections.len(), SECTIONS.len());
        // alte Datei ohne „links“, mit unbekanntem Bereich
        let old = r#"{"sections":[{"key":"pc","on":false},{"key":"xyz","on":true}],"tile":7}"#;
        let mut c: StartCfg = serde_json::from_str(old).unwrap();
        c.sections.retain(|s| SECTIONS.iter().any(|(k, _)| *k == s.key));
        for (k, _) in SECTIONS {
            if !c.sections.iter().any(|s| s.key == k) {
                c.sections.push(Section { key: k.to_string(), on: true });
            }
        }
        assert_eq!(c.sections[0].key, "pc");
        assert!(!c.sections[0].on);
        assert_eq!(c.sections.len(), 5);
        assert!(c.greeting && c.search);
        let now = chrono::Local::now();
        assert!(!greeting(&now).0.is_empty());
    }
}
