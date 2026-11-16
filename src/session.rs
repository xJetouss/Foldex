//! Sitzung: Jedes Fenster speichert laufend seinen Zustand – Position und Größe, Tabs in Reihenfolge, aktiver
//! Tab, Kachel-Aufteilung (mit Verhältnissen), Tabs innerhalb von Kacheln sowie die Vorschau jeder Kachel
//! (an/aus, Lage, Größe). Beim nächsten Start ohne andere offene Fenster werden die Fenster wiederhergestellt.
//!
//! Welche Fenster? Wer geschlossen wird, während noch andere laufen, merkt sich den Zeitpunkt. Wiederhergestellt
//! werden die Fenster, die zuletzt (innerhalb weniger Sekunden) geschlossen wurden – also die bis zum Schluss
//! offenen –, und nach einem Absturz alle.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PaneState {
    /// Beschreibung wie beim Verschieben zwischen Fenstern (Ordner, Terminal, Git, KI, Kachel-Gruppe)
    pub spec: String,
    pub preview: bool,
    /// „l“, „r“, „t“, „b“
    pub preview_side: String,
    pub preview_frac: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TabState {
    /// Kachel-Layout (tiling::Node::serialize)
    pub layout: String,
    pub active: usize,
    pub panes: Vec<PaneState>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WinState {
    /// Fensterposition (außen) und Innengröße in Punkten
    pub pos: Option<[f32; 2]>,
    pub size: Option<[f32; 2]>,
    pub maximized: bool,
    pub cur: usize,
    pub tabs: Vec<TabState>,
    /// geschlossen um (ms seit 1970), 0 = lief noch (Absturz) bzw. noch offen
    #[serde(default)]
    pub closed: u64,
}

/// Lage des zuletzt benutzten Fensters – unabhängig vom Wiederherstellen der Sitzung, damit Foldex immer an
/// derselben Stelle und in derselben Größe startet. `pos` außen, `size` innen (Punkte); bei maximiertem
/// Fenster die normale Lage davor. `ppp` = Skalierung des Bildschirms (Punkte → Pixel) für die Prüfung beim Start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WinGeo {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub maximized: bool,
    #[serde(default)]
    pub ppp: f32,
}

fn geo_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("window.json")
}

pub fn save_geo(g: &WinGeo) {
    if let Ok(j) = serde_json::to_string(g) {
        let _ = crate::themes::write_atomic(&geo_file(), j.as_bytes());
    }
}

pub fn load_geo() -> Option<WinGeo> {
    serde_json::from_slice(&fs::read(geo_file()).ok()?).ok()
}

fn dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("session")
}

fn own() -> PathBuf {
    dir().join(format!("win-{}.json", std::process::id()))
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Zustand dieses Fensters schreiben (atomar).
pub fn save(s: &WinState) {
    let _ = fs::create_dir_all(dir());
    if let Ok(j) = serde_json::to_string(s) {
        let _ = crate::themes::write_atomic(&own(), j.as_bytes());
    }
}

/// Fenster wird geschlossen: Zeitpunkt vermerken (entscheidet beim Start, ob es wiederhergestellt wird).
pub fn close(mut s: WinState) {
    s.closed = now_ms();
    save(&s);
}

/// Sitzung dieses Fensters verwerfen (Wiederherstellen ist ausgeschaltet).
pub fn remove() {
    let _ = fs::remove_file(own());
}

pub fn load(p: &Path) -> Option<WinState> {
    serde_json::from_slice(&fs::read(p).ok()?).ok()
}

/// Alle gespeicherten Fenster einsammeln (und die Dateien entfernen). Zurück kommen die, die zuletzt offen waren.
pub fn take_all() -> Vec<WinState> {
    let mut v: Vec<WinState> = Vec::new();
    for e in fs::read_dir(dir()).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") {
            if let Some(s) = load(&p) {
                if !s.tabs.is_empty() {
                    v.push(s);
                }
            }
            let _ = fs::remove_file(&p);
        }
    }
    select(v)
}

/// Auswahl: abgestürzte Fenster (closed = 0) immer; geschlossene nur, wenn sie höchstens 10 s vor dem zuletzt
/// geschlossenen zugingen. Reihenfolge: zuletzt geschlossenes zuerst (wird das Hauptfenster).
fn select(mut v: Vec<WinState>) -> Vec<WinState> {
    let last = v.iter().map(|s| s.closed).max().unwrap_or(0);
    v.retain(|s| s.closed == 0 || s.closed + 10_000 >= last);
    v.sort_by(|a, b| b.closed.cmp(&a.closed));
    v
}

/// Zustand für ein weiteres Fenster ablegen, das beim Start mit `--session <Datei>` übernimmt.
pub fn handoff(s: &WinState, k: usize) -> Option<PathBuf> {
    let p = dir().join(format!("restore-{}-{k}.state", std::process::id()));
    let _ = fs::create_dir_all(dir());
    fs::write(&p, serde_json::to_string(s).ok()?).ok()?;
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_last_open_windows() {
        let w = |closed: u64| WinState { closed, tabs: vec![TabState::default()], ..Default::default() };
        // A früh geschlossen, B und C zusammen zuletzt → B, C
        let r = select(vec![w(1_000), w(100_000), w(95_000)]);
        assert_eq!(r.iter().map(|s| s.closed).collect::<Vec<_>>(), vec![100_000, 95_000]);
        // Absturz: alle
        assert_eq!(select(vec![w(0), w(0)]).len(), 2);
        let s = WinState { pos: Some([10.0, 20.0]), cur: 1, tabs: vec![TabState { layout: "H(0,1,0.5)".into(), active: 1, panes: vec![PaneState { spec: "C:\\x".into(), preview: true, preview_side: "b".into(), preview_frac: 0.3 }] }], ..Default::default() };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<WinState>(&j).unwrap(), s);
    }
}
