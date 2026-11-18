//! Ordnergrößen: sofort ein Wert aus dem Cache (auch über Programmstarts hinweg), im Hintergrund
//! wird genauer gerechnet. Ein einziger Arbeits-Thread – niedrige CPU-Last, die Oberfläche blockiert nie.
//! Beim Ordnerwechsel werden wartende Aufträge des alten Ordners verworfen; laufende brechen ab.
//! Symbolische Links und Junctions werden nicht verfolgt (keine Schleifen, keine Doppelzählung).

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering::Relaxed},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

/// Nach dieser Zeit wird ein Wert im Hintergrund neu berechnet (der alte bleibt bis dahin sichtbar).
const TTL_SECS: i64 = 15 * 60;
const CACHE_MAX: usize = 4000;

#[derive(Clone, Copy, Debug)]
pub struct Size {
    pub bytes: u64,
    pub files: u64,
    /// false = Zwischenstand (Berechnung läuft noch oder wurde abgebrochen)
    pub exact: bool,
    /// Unterordner ohne Zugriffsrecht wurden übersprungen
    pub partial: bool,
    pub at: i64,
}

fn now() -> i64 {
    chrono::Local::now().timestamp()
}

fn cache_file() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("dirsizes.txt")
}

struct Job {
    path: PathBuf,
    gen: u64,
}

pub struct DirSizes {
    map: HashMap<PathBuf, Size>,
    pending: HashSet<PathBuf>,
    tx: mpsc::Sender<Job>,
    rx: mpsc::Receiver<(PathBuf, Size)>,
    gen: Arc<AtomicU64>,
    dirty: bool,
    /// zählt hoch, sobald neue Werte da sind (Sortieren nach Größe frischt dann auf)
    pub ver: u64,
}

impl DirSizes {
    pub fn new(ctx: &eframe::egui::Context) -> Self {
        let (tx, jobs) = mpsc::channel::<Job>();
        let (res_tx, rx) = mpsc::channel();
        let gen = Arc::new(AtomicU64::new(0));
        let (g, ctx) = (gen.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("dirsize".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    if job.gen != g.load(Relaxed) {
                        continue; // veraltet: Ordner wurde inzwischen gewechselt
                    }
                    let path = job.path.clone();
                    let send = |s: Size| {
                        let _ = res_tx.send((path.clone(), s));
                        ctx.request_repaint();
                    };
                    walk(&job.path, || g.load(Relaxed) != job.gen, send);
                }
            })
            .ok();
        let mut map = HashMap::new();
        if let Ok(t) = fs::read_to_string(cache_file()) {
            for l in t.lines() {
                let mut it = l.splitn(4, '\t');
                if let (Some(b), Some(f), Some(at), Some(p)) = (it.next(), it.next(), it.next(), it.next()) {
                    if let (Ok(bytes), Ok(files), Ok(at)) = (b.parse(), f.parse(), at.parse()) {
                        map.insert(PathBuf::from(p), Size { bytes, files, exact: true, partial: false, at });
                    }
                }
            }
        }
        DirSizes { map, pending: HashSet::new(), tx, rx, gen, dirty: false, ver: 0 }
    }

    /// Größe für die Anzeige; stößt bei Bedarf eine (Neu-)Berechnung an.
    pub fn get(&mut self, p: &Path) -> Option<Size> {
        let s = self.map.get(p).copied();
        let stale = match s {
            None => true,
            Some(s) => !s.exact || now() - s.at > TTL_SECS,
        };
        if stale && !self.pending.contains(p) {
            self.pending.insert(p.to_path_buf());
            let _ = self.tx.send(Job { path: p.to_path_buf(), gen: self.gen.load(Relaxed) });
        }
        s
    }

    /// Ergebnisse des Arbeits-Threads übernehmen (jedes Frame, sehr günstig).
    pub fn poll(&mut self) {
        while let Ok((p, s)) = self.rx.try_recv() {
            if s.exact {
                self.pending.remove(&p);
                self.dirty = true;
            }
            self.map.insert(p, s);
            self.ver += 1;
        }
    }

    /// Neuer Ordner angezeigt: wartende und laufende Berechnungen des alten Ordners abbrechen.
    pub fn new_generation(&mut self) {
        self.gen.fetch_add(1, Relaxed);
        self.pending.clear();
    }

    /// Nach Dateioperationen in `dir`: diesen Ordner und alle übergeordneten als veraltet markieren
    /// (die alten Werte bleiben sichtbar, bis neu gerechnet ist). Unterordner sind nicht betroffen.
    pub fn invalidate(&mut self, dir: &Path) {
        for (k, v) in self.map.iter_mut() {
            if dir.starts_with(k) {
                v.at = 0;
            }
        }
    }

    pub fn save(&self) {
        if !self.dirty {
            return;
        }
        let mut v: Vec<(&PathBuf, &Size)> = self.map.iter().filter(|(_, s)| s.exact && s.at > 0).collect();
        v.sort_by_key(|(_, s)| std::cmp::Reverse(s.at));
        v.truncate(CACHE_MAX);
        let mut out = String::with_capacity(v.len() * 64);
        for (p, s) in v {
            let ps = p.to_string_lossy();
            if !ps.contains('\n') {
                out.push_str(&format!("{}\t{}\t{}\t{}\n", s.bytes, s.files, s.at, ps));
            }
        }
        let _ = crate::themes::write_atomic(&cache_file(), out.as_bytes());
    }
}

/// Summiert einen Ordner iterativ; meldet alle ~150 ms einen Zwischenstand.
pub fn walk(root: &Path, cancelled: impl Fn() -> bool, mut send: impl FnMut(Size)) {
    // Cloud-Laufwerk: nicht über das Laufwerk durchlaufen (viele Einzelabfragen, bremst alles) – rclone zählt
    if crate::trashx::on_cloud(root) {
        if let Some((bytes, files)) = crate::cloudlogin::size_of(root) {
            send(Size { bytes, files, exact: true, partial: false, at: now() });
        }
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    let (mut bytes, mut files, mut partial) = (0u64, 0u64, false);
    let mut last = Instant::now();
    let mut n = 0u32;
    while let Some(d) = stack.pop() {
        let rd = match fs::read_dir(&d) {
            Ok(rd) => rd,
            Err(_) => {
                partial = true;
                continue;
            }
        };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(e.path());
            } else if let Ok(m) = e.metadata() {
                bytes += m.len();
                files += 1;
            }
            n += 1;
            if n % 512 == 0 {
                if cancelled() {
                    send(Size { bytes, files, exact: false, partial, at: now() });
                    return;
                }
                if last.elapsed() > Duration::from_millis(150) {
                    last = Instant::now();
                    send(Size { bytes, files, exact: false, partial, at: now() });
                }
            }
        }
    }
    send(Size { bytes, files, exact: true, partial, at: now() });
}
