//! Dateiinfos für die Oberfläche, ohne auf Cloud-Laufwerke zu warten.
//!
//! Ein Zugriff auf ein rclone-Laufwerk kann viele Sekunden hängen (Anbieter bremst, Verzeichnis wird neu
//! gelesen) – und Windows kann ihn nicht abbrechen. Die Oberfläche fragt deshalb hier: lokale Pfade wie
//! gewohnt direkt, Cloud-Pfade aus einem Zwischenspeicher, den das Ordner-Laden füllt. Fehlt ein Eintrag,
//! wird er im Hintergrund nachgeschlagen und die Oberfläche neu gezeichnet, sobald er da ist.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Stat {
    pub dir: bool,
    pub len: u64,
}

/// so lange gilt ein Eintrag
const TTL: Duration = Duration::from_secs(30);

struct Cache {
    map: HashMap<PathBuf, (Instant, Option<Stat>)>,
    busy: HashSet<PathBuf>,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache { map: HashMap::new(), busy: HashSet::new() }))
}

static CTX: OnceLock<eframe::egui::Context> = OnceLock::new();

/// einmal beim Start: zum Neuzeichnen, wenn nachgeschlagene Infos eintreffen
pub fn init(ctx: &eframe::egui::Context) {
    let _ = CTX.set(ctx.clone());
}

/// liegt der Pfad auf einem Cloud-Laufwerk (Zugriffe können hängen)?
pub fn slow(p: &Path) -> bool {
    crate::trashx::on_cloud(p)
}

/// Inhalt eines frisch gelesenen Ordners merken: (Name, Ordner?, Größe)
pub fn note_listing<'a>(dir: &Path, items: impl Iterator<Item = (&'a str, bool, u64)>) {
    if !slow(dir) {
        return;
    }
    let now = Instant::now();
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if c.map.len() > 50_000 {
        c.map.retain(|_, (t, _)| t.elapsed() < TTL);
    }
    c.map.insert(dir.to_path_buf(), (now, Some(Stat { dir: true, len: 0 })));
    for (name, d, len) in items {
        c.map.insert(dir.join(name), (now, Some(Stat { dir: d, len })));
    }
}

/// Eintrag nach eigener Änderung setzen bzw. entfernen (None = gibt es nicht mehr)
pub fn note(p: &Path, s: Option<Stat>) {
    if slow(p) {
        cache().lock().unwrap_or_else(|e| e.into_inner()).map.insert(p.to_path_buf(), (Instant::now(), s));
    }
}

/// Some(Some) = vorhanden, Some(None) = gibt es nicht, None = noch unbekannt (wird nachgeschlagen)
pub fn stat(p: &Path) -> Option<Option<Stat>> {
    if !slow(p) {
        return Some(std::fs::metadata(p).ok().map(|m| Stat { dir: m.is_dir(), len: m.len() }));
    }
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    let hit = c.map.get(p).copied();
    let fresh = hit.is_some_and(|(t, _)| t.elapsed() < TTL);
    if !fresh && !c.busy.contains(p) {
        c.busy.insert(p.to_path_buf());
        let p2 = p.to_path_buf();
        std::thread::spawn(move || {
            let s = std::fs::metadata(&p2).ok().map(|m| Stat { dir: m.is_dir(), len: m.len() });
            let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
            c.busy.remove(&p2);
            c.map.insert(p2, (Instant::now(), s));
            drop(c);
            if let Some(ctx) = CTX.get() {
                ctx.request_repaint();
            }
        });
    }
    // ein älterer Wert ist besser als keiner
    hit.map(|(_, s)| s)
}

pub fn is_dir(p: &Path) -> bool {
    matches!(stat(p), Some(Some(Stat { dir: true, .. })))
}

/// unbekannt zählt als Datei, wenn der Name eine Endung hat (meist richtig, wird gleich nachgeschlagen)
pub fn is_file(p: &Path) -> bool {
    match stat(p) {
        Some(Some(s)) => !s.dir,
        Some(None) => false,
        None => p.extension().is_some(),
    }
}

/// unbekannt zählt als vorhanden
pub fn exists(p: &Path) -> bool {
    !matches!(stat(p), Some(None))
}
