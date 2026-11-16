//! Live-Ordnersuche (ohne Index): läuft in einem Hintergrund-Thread, liefert Treffer in Paketen
//! und bricht sofort ab, wenn sich die Eingabe ändert.
//! - Versteckte Dateien und Ordner werden immer mit durchsucht, auch beliebig tief verschachtelt.
//! - System- und Cache-Ordner (siehe `index::excluded_dir`) werden übersprungen – außer die Suche startet darin.
//! - Inhaltssuche (`inhalt:wort`): Dateien werden parallel gelesen; Binärdateien werden übersprungen.

use crate::index::{self, Query};
use eframe::egui;
use rayon::prelude::*;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

pub struct LiveHit {
    pub path: PathBuf,
    pub dir: bool,
    /// Inhaltssuche: (Zeile, Ausschnitt) des ersten Treffers
    pub snippet: Option<(usize, String)>,
}

pub struct Live {
    pub rx: mpsc::Receiver<Vec<(i32, LiveHit)>>,
    pub cancel: Arc<AtomicBool>,
    pub done: Arc<AtomicBool>,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.cancel.store(true, Relaxed);
    }
}

/// Höchstzahl Treffer bzw. besuchter Einträge (schützt vor endlosen Läufen über ganze Laufwerke).
const MAX_HITS: usize = 20_000;
const MAX_VISITED: usize = 3_000_000;

/// Sammelt Treffer und schickt sie in Paketen (höchstens alle 90 ms bzw. je 200).
struct Sender {
    tx: mpsc::Sender<Vec<(i32, LiveHit)>>,
    batch: Vec<(i32, LiveHit)>,
    last: Instant,
    total: usize,
    ctx: egui::Context,
}

impl Sender {
    /// `false` = Empfänger weg oder Höchstzahl erreicht → aufhören.
    fn push(&mut self, sc: i32, h: LiveHit) -> bool {
        self.batch.push((sc, h));
        self.total += 1;
        self.flush(false) && self.total < MAX_HITS
    }
    fn flush(&mut self, force: bool) -> bool {
        if !self.batch.is_empty() && (force || self.batch.len() >= 200 || self.last.elapsed() > Duration::from_millis(90)) {
            if self.tx.send(std::mem::take(&mut self.batch)).is_err() {
                return false;
            }
            self.last = Instant::now();
            self.ctx.request_repaint();
        }
        true
    }
}

fn path_ok(q: &Query, path: &std::path::Path) -> bool {
    if !q.has_path_terms() {
        return true;
    }
    let p = path.to_string_lossy().to_ascii_lowercase().replace('\\', "/");
    q.ptes.iter().all(|t| memchr::memmem::find(p.as_bytes(), t).is_some())
}

/// Durchsucht `root`. `skip` = Pfade, die schon (aus dem Index) angezeigt werden – nur Neues wird gemeldet.
pub fn start(root: PathBuf, q: Query, skip: HashSet<PathBuf>, ctx: egui::Context) -> Live {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let (c2, d2) = (cancel.clone(), done.clone());
    std::thread::spawn(move || {
        let finders = q.finders();
        let name_filter = !q.terms.is_empty() || !q.exts.is_empty() || q.dir.is_some() || !q.negs.is_empty() || q.has_path_terms();
        let mut out = Sender { tx, batch: Vec::new(), last: Instant::now(), total: 0, ctx };
        let mut stack = vec![root];
        let mut visited = 0usize;
        'walk: while let Some(dir) = stack.pop() {
            if c2.load(Relaxed) {
                break;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut files: Vec<(i32, PathBuf)> = Vec::new();
            for e in rd.flatten() {
                visited += 1;
                let fname = e.file_name();
                let name = fname.to_string_lossy();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let lower = name.to_ascii_lowercase();
                if is_dir && !index::excluded_dir(&lower, false) {
                    stack.push(e.path());
                }
                let Some(sc) = q.score_name(lower.as_bytes(), is_dir, &finders) else { continue };
                let path = e.path();
                if !path_ok(&q, &path) || skip.contains(&path) {
                    continue;
                }
                if q.has_content() {
                    // Inhalt: nur Dateien; ohne Namensfilter zählt jede Datei als Kandidat
                    if !is_dir {
                        files.push((if name_filter { sc } else { 0 }, path));
                    }
                } else if !out.push(sc, LiveHit { path, dir: is_dir, snippet: None }) {
                    break 'walk;
                }
            }
            if !files.is_empty() && !content_pass(files, &q.content, &c2, &mut out) {
                break;
            }
            if visited >= MAX_VISITED || !out.flush(false) {
                break;
            }
        }
        out.flush(true);
        d2.store(true, Relaxed);
        out.ctx.request_repaint();
    });
    Live { rx, cancel, done }
}

/// Prüft feste Kandidaten auf ihren Inhalt (PC-Suche: Namenstreffer aus dem Index).
pub fn start_content(files: Vec<PathBuf>, q: Query, ctx: egui::Context) -> Live {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let (c2, d2) = (cancel.clone(), done.clone());
    std::thread::spawn(move || {
        let mut out = Sender { tx, batch: Vec::new(), last: Instant::now(), total: 0, ctx };
        for chunk in files.chunks(512) {
            let v: Vec<(i32, PathBuf)> = chunk.iter().map(|p| (0, p.clone())).collect();
            if !content_pass(v, &q.content, &c2, &mut out) {
                break;
            }
        }
        out.flush(true);
        d2.store(true, Relaxed);
        out.ctx.request_repaint();
    });
    Live { rx, cancel, done }
}

/// Liest die Dateien parallel und meldet die, deren Inhalt passt. `false` = abbrechen.
fn content_pass(files: Vec<(i32, PathBuf)>, terms: &[Vec<u8>], cancel: &AtomicBool, out: &mut Sender) -> bool {
    let hits: Vec<(i32, LiveHit)> = files
        .into_par_iter()
        .filter_map(|(sc, path)| {
            if cancel.load(Relaxed) {
                return None;
            }
            let m = index::content_match(&path, terms)?;
            Some((sc + 500, LiveHit { path, dir: false, snippet: Some(m) }))
        })
        .collect();
    for (sc, h) in hits {
        if !out.push(sc, h) {
            return false;
        }
    }
    !cancel.load(Relaxed)
}
