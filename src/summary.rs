//! Zusammenfassung per Rechtsklick: Ordner (was liegt hier, was ist groß, was ist doppelt) und Textdateien
//! (Umfang, Anfang). Die Zahlen rechnet Foldex selbst im Hintergrund; dazu schreibt die KI (Modell der
//! Explorer-KI oder ein lokales Modell) ein paar Sätze, worum es geht.

use crate::ai::{self, Cfg};
use crate::themes::Theme;
use eframe::egui::{self, vec2, Color32};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

/// höchstens so viele Einträge durchgehen (sehr große Ordner)
const MAX_ENTRIES: u64 = 400_000;
/// höchstens so viel lesen, um Duplikate zu prüfen
const MAX_HASH_BYTES: u64 = 4_000_000_000;

#[derive(Default)]
pub struct FolderStats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Art → (Anzahl, Bytes)
    pub kinds: Vec<(&'static str, u64, u64)>,
    pub largest: Vec<(PathBuf, u64)>,
    pub newest: Vec<(PathBuf, std::time::SystemTime)>,
    /// je Gruppe: Größe einer Datei und die gleichen Dateien
    pub dupes: Vec<(u64, Vec<PathBuf>)>,
    pub truncated: bool,
    /// einige Namen für die KI
    sample: Vec<String>,
}

pub struct TextStats {
    pub bytes: u64,
    pub lines: usize,
    pub words: usize,
    pub chars: usize,
    pub head: String,
    text: String,
}

pub enum Stats {
    Folder(FolderStats),
    Text(TextStats),
}

pub struct Summary {
    pub path: PathBuf,
    stats: Option<Result<Stats, String>>,
    rx: Option<mpsc::Receiver<Result<Stats, String>>>,
    progress: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    ai: Option<Result<String, String>>,
    ai_rx: Option<mpsc::Receiver<Result<String, String>>>,
    cfg: Option<Cfg>,
}

fn kind_of(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "svg" | "ico" | "raw" | "cr2" | "nef" | "psd" => "Bilder",
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" | "flv" => "Videos",
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" | "wma" => "Musik",
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx" | "ods" | "csv" | "ppt" | "pptx" | "odp" | "txt" | "md" | "epub" => "Dokumente",
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" | "iso" => "Archive",
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "hpp" | "cs" | "java" | "kt" | "go" | "rb" | "php" | "swift" | "html" | "css" | "scss" | "json" | "toml" | "yaml" | "yml" | "xml" | "sh" | "ps1" | "bat" | "sql" | "lua" => "Code",
        "exe" | "msi" | "dll" | "sys" => "Programme",
        "gguf" | "safetensors" | "ckpt" | "onnx" | "bin" | "pt" => "KI-Modelle",
        _ => "Sonstiges",
    }
}

fn is_text_file(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if matches!(kind_of(&ext), "Code") || matches!(ext.as_str(), "txt" | "md" | "csv" | "log" | "ini" | "cfg" | "conf" | "rst" | "tex" | "srt" | "env" | "gitignore") {
        return true;
    }
    // ohne bekannte Endung: kurze Probe ohne Nullbytes
    let mut b = [0u8; 4096];
    std::fs::File::open(p).and_then(|mut f| f.read(&mut b)).is_ok_and(|n| n > 0 && !b[..n].contains(&0) && std::str::from_utf8(&b[..n.min(4000)]).is_ok())
}

/// gibt es für diese Auswahl eine Zusammenfassung?
pub fn supported(p: &Path) -> bool {
    p.is_dir() || (p.is_file() && std::fs::metadata(p).is_ok_and(|m| m.len() < 50_000_000) && is_text_file(p))
}

impl Summary {
    pub fn new(path: PathBuf, cfg: Option<Cfg>, ctx: &egui::Context) -> Summary {
        let (tx, rx) = mpsc::channel();
        let (progress, cancel) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
        let (p, pr, c, ctx) = (path.clone(), progress.clone(), cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = if p.is_dir() { scan_folder(&p, &pr, &c).map(Stats::Folder) } else { read_text(&p).map(Stats::Text) };
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        Summary { path, stats: None, rx: Some(rx), progress, cancel, ai: None, ai_rx: None, cfg }
    }

    fn start_ai(&mut self, ctx: &egui::Context) {
        let (Some(cfg), Some(Ok(st))) = (self.cfg.clone(), &self.stats) else { return };
        let name = self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let user = match st {
            Stats::Folder(f) => {
                let kinds: Vec<String> = f.kinds.iter().map(|(k, n, b)| format!("{k}: {n} ({})", human(*b))).collect();
                let big: Vec<String> = f.largest.iter().take(8).map(|(p, b)| format!("{} ({})", rel(&self.path, p), human(*b))).collect();
                format!(
                    "Ordner „{name}“: {} Dateien in {} Unterordnern, zusammen {}.\nArten: {}\nGrößte: {}\nDuplikat-Gruppen: {}\nBeispiele für Namen:\n{}",
                    f.files,
                    f.dirs,
                    human(f.bytes),
                    kinds.join(", "),
                    big.join(", "),
                    f.dupes.len(),
                    f.sample.join("\n")
                )
            }
            Stats::Text(t) => format!("Datei „{name}“ ({} Zeilen). Inhalt (Anfang):\n\n{}", t.lines, t.text),
        };
        let system = match st {
            Stats::Folder(_) => "Du fasst den Inhalt eines Ordners für den Benutzer zusammen: worum es geht (Projekt, Fotos, Dokumente …), was auffällt und 1–3 konkrete Tipps zum Aufräumen. Kurz, auf Deutsch, höchstens 6 Sätze, keine Überschriften.",
            Stats::Text(_) => "Du fasst eine Textdatei für den Benutzer zusammen: worum es geht, die wichtigsten Punkte als kurze Aufzählung, bei Code was er tut. Kurz und auf Deutsch, höchstens 8 Zeilen.",
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let system = system.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(ai::quick(&cfg, &system, &user));
            ctx.request_repaint();
        });
        self.ai_rx = Some(rx);
    }

    /// Fenster; false = geschlossen. `open` meldet eine angeklickte Datei (im Explorer zeigen).
    pub fn ui(&mut self, ctx: &egui::Context, th: &Theme, open_path: &mut Option<PathBuf>) -> bool {
        if let Some(rx) = &self.rx {
            if let Ok(r) = rx.try_recv() {
                self.rx = None;
                self.stats = Some(r);
                self.start_ai(ctx);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }
        if let Some(rx) = &self.ai_rx {
            if let Ok(r) = rx.try_recv() {
                self.ai_rx = None;
                self.ai = Some(r);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
        let mut open = true;
        let name = self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned());
        egui::Window::new(format!("Zusammenfassung – {name}")).id(egui::Id::new(("summary", &self.path))).open(&mut open).default_size(vec2(560.0, 620.0)).resizable(true).collapsible(false).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width());
                // KI-Text oben
                egui::Frame::none().fill(th.field).rounding(8.0).inner_margin(egui::Margin::same(12.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        crate::ui::draw(ui.painter(), egui::Rect::from_min_size(ui.cursor().min, vec2(16.0, 16.0)), crate::ui::Ic::Spark, th.accent);
                        ui.add_space(20.0);
                        ui.label(egui::RichText::new("KI-Zusammenfassung").strong());
                    });
                    match (&self.ai, &self.ai_rx, &self.cfg) {
                        (Some(Ok(t)), _, _) => {
                            ui.add(egui::Label::new(egui::RichText::new(t.trim()).size(13.5)).wrap());
                        }
                        (Some(Err(e)), _, _) => {
                            ui.label(egui::RichText::new(e).size(12.5).color(th.danger));
                        }
                        (None, Some(_), _) => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(egui::RichText::new("schreibt …").color(th.sub));
                            });
                        }
                        (None, None, None) => {
                            ui.label(egui::RichText::new("Kein KI-Modell eingerichtet – Einstellungen → KI oder → Lokale Modelle. Die Zahlen unten gibt es trotzdem.").size(12.5).color(th.sub));
                        }
                        (None, None, Some(_)) => {
                            if self.rx.is_some() {
                                ui.label(egui::RichText::new("wartet auf die Auswertung …").color(th.sub));
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                match &self.stats {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(format!("Wird ausgewertet … {} Einträge", self.progress.load(Ordering::Relaxed)));
                        });
                    }
                    Some(Err(e)) => {
                        ui.label(egui::RichText::new(e).color(th.danger));
                    }
                    Some(Ok(Stats::Text(t))) => text_ui(ui, th, t),
                    Some(Ok(Stats::Folder(f))) => folder_ui(ui, th, f, &self.path, open_path),
                }
            });
        });
        if !open {
            self.cancel.store(true, Ordering::Relaxed);
        }
        open
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).size(13.5).strong());
    ui.add_space(2.0);
}

fn text_ui(ui: &mut egui::Ui, th: &Theme, t: &TextStats) {
    ui.horizontal_wrapped(|ui| {
        for (k, v) in [("Zeilen", t.lines.to_string()), ("Wörter", t.words.to_string()), ("Zeichen", t.chars.to_string()), ("Größe", human(t.bytes))] {
            stat(ui, th, k, &v);
        }
    });
    section(ui, "Anfang");
    egui::Frame::none().fill(th.field).rounding(6.0).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(egui::RichText::new(&t.head).monospace().size(12.0)).wrap());
    });
}

fn stat(ui: &mut egui::Ui, th: &Theme, k: &str, v: &str) {
    egui::Frame::none().fill(th.field).rounding(8.0).inner_margin(egui::Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(v).size(17.0).strong());
            ui.label(egui::RichText::new(k).size(11.5).color(th.sub));
        });
    });
}

fn folder_ui(ui: &mut egui::Ui, th: &Theme, f: &FolderStats, base: &Path, open_path: &mut Option<PathBuf>) {
    ui.horizontal_wrapped(|ui| {
        stat(ui, th, "Dateien", &f.files.to_string());
        stat(ui, th, "Ordner", &f.dirs.to_string());
        stat(ui, th, "Größe", &human(f.bytes));
        let waste: u64 = f.dupes.iter().map(|(s, v)| s * (v.len() as u64 - 1)).sum();
        stat(ui, th, "doppelt", &human(waste));
    });
    if f.truncated {
        ui.label(egui::RichText::new(format!("Sehr großer Ordner – die ersten {MAX_ENTRIES} Einträge ausgewertet.")).size(11.5).color(th.sub));
    }
    section(ui, "Nach Art");
    let max = f.kinds.iter().map(|k| k.2).max().unwrap_or(1).max(1);
    let colors = [Color32::from_rgb(58, 134, 255), Color32::from_rgb(230, 90, 120), Color32::from_rgb(46, 184, 120), Color32::from_rgb(240, 170, 40), Color32::from_rgb(160, 110, 240), Color32::from_rgb(40, 190, 210), Color32::from_rgb(140, 150, 160)];
    for (i, (k, n, b)) in f.kinds.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized(vec2(90.0, 18.0), egui::Label::new(egui::RichText::new(*k).size(12.5)));
            let w = (ui.available_width() - 150.0).max(40.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 5.0, th.field);
            ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2((w * *b as f32 / max as f32).max(3.0), 10.0)), 5.0, colors[i % colors.len()]);
            ui.label(egui::RichText::new(format!("{n} · {}", human(*b))).size(12.0).color(th.sub));
        });
    }
    let file_row = |ui: &mut egui::Ui, p: &PathBuf, right: String, open_path: &mut Option<PathBuf>| {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(right).size(12.0).color(th.sub));
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if ui.add(egui::Label::new(egui::RichText::new(rel(base, p)).size(12.5)).truncate().sense(egui::Sense::click())).on_hover_text("Im Explorer zeigen").clicked() {
                        *open_path = Some(p.clone());
                    }
                });
            });
        });
    };
    if !f.largest.is_empty() {
        section(ui, "Größte Dateien");
        for (p, b) in &f.largest {
            file_row(ui, p, human(*b), open_path);
        }
    }
    section(ui, if f.dupes.is_empty() { "Keine Duplikate gefunden" } else { "Doppelte Dateien (gleicher Inhalt)" });
    for (size, v) in f.dupes.iter().take(12) {
        egui::Frame::none().fill(th.field).rounding(6.0).inner_margin(egui::Margin::symmetric(8.0, 4.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(format!("{}× je {}", v.len(), human(*size))).size(11.5).color(th.sub));
            for p in v {
                file_row(ui, p, String::new(), open_path);
            }
        });
        ui.add_space(3.0);
    }
    if !f.newest.is_empty() {
        section(ui, "Zuletzt geändert");
        for (p, t) in &f.newest {
            let when = chrono::DateTime::<chrono::Local>::from(*t).format("%d.%m.%Y %H:%M").to_string();
            file_row(ui, p, when, open_path);
        }
    }
}

fn rel(base: &Path, p: &Path) -> String {
    p.strip_prefix(base).map_or_else(|_| p.display().to_string(), |r| r.display().to_string())
}

pub fn human(b: u64) -> String {
    let f = b as f64;
    if f >= 1e9 {
        format!("{:.1} GB", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.1} MB", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.0} KB", f / 1e3)
    } else {
        format!("{b} B")
    }
}

fn read_text(p: &Path) -> Result<TextStats, String> {
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
    let s = String::from_utf8_lossy(&bytes).into_owned();
    let head: String = s.lines().take(12).collect::<Vec<_>>().join("\n");
    let text: String = s.chars().take(14_000).collect();
    Ok(TextStats { bytes: bytes.len() as u64, lines: s.lines().count(), words: s.split_whitespace().count(), chars: s.chars().count(), head, text })
}

fn scan_folder(root: &Path, progress: &AtomicU64, cancel: &AtomicBool) -> Result<FolderStats, String> {
    let mut st = FolderStats::default();
    let mut kinds: HashMap<&'static str, (u64, u64)> = HashMap::new();
    let mut all: Vec<(PathBuf, u64)> = Vec::new();
    let mut newest: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0u64;
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            if cancel.load(Ordering::Relaxed) {
                return Err("abgebrochen".into());
            }
            seen += 1;
            if seen % 500 == 0 {
                progress.store(seen, Ordering::Relaxed);
            }
            if seen > MAX_ENTRIES {
                st.truncated = true;
                break;
            }
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                st.dirs += 1;
                stack.push(p);
            } else if ft.is_file() {
                let Ok(m) = e.metadata() else { continue };
                let len = m.len();
                st.files += 1;
                st.bytes += len;
                let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                let k = kinds.entry(kind_of(&ext)).or_default();
                k.0 += 1;
                k.1 += len;
                if st.sample.len() < 120 {
                    st.sample.push(rel(root, &p));
                }
                if let Ok(t) = m.modified() {
                    newest.push((p.clone(), t));
                }
                all.push((p, len));
            }
        }
        if st.truncated {
            break;
        }
    }
    let mut kv: Vec<(&'static str, u64, u64)> = kinds.into_iter().map(|(k, (n, b))| (k, n, b)).collect();
    kv.sort_by(|a, b| b.2.cmp(&a.2));
    st.kinds = kv;
    newest.sort_by(|a, b| b.1.cmp(&a.1));
    newest.truncate(8);
    st.newest = newest;
    let mut by_size = all.clone();
    by_size.sort_by(|a, b| b.1.cmp(&a.1));
    st.largest = by_size.iter().take(10).cloned().collect();
    st.dupes = duplicates(&all, cancel);
    Ok(st)
}

/// gleiche Dateien: erst gleiche Größe, dann gleicher Anfang (64 KB), dann gleiche Prüfsumme
fn duplicates(all: &[(PathBuf, u64)], cancel: &AtomicBool) -> Vec<(u64, Vec<PathBuf>)> {
    let mut by_len: HashMap<u64, Vec<&PathBuf>> = HashMap::new();
    for (p, l) in all.iter().filter(|(_, l)| *l >= 1024) {
        by_len.entry(*l).or_default().push(p);
    }
    let mut groups: Vec<(u64, Vec<&PathBuf>)> = by_len.into_iter().filter(|(_, v)| v.len() > 1).collect();
    // große zuerst – dort lohnt sich Aufräumen am meisten
    groups.sort_by(|a, b| (b.0 * b.1.len() as u64).cmp(&(a.0 * a.1.len() as u64)));
    let mut budget = MAX_HASH_BYTES;
    let mut out = Vec::new();
    for (len, files) in groups {
        if cancel.load(Ordering::Relaxed) || budget == 0 {
            break;
        }
        let hash = |p: &Path, limit: Option<u64>| -> Option<[u8; 32]> {
            let mut f = std::fs::File::open(p).ok()?;
            let mut h = Sha256::new();
            let mut buf = vec![0u8; 1 << 16];
            let mut left = limit.unwrap_or(u64::MAX);
            while left > 0 {
                let take = left.min(buf.len() as u64) as usize;
                let n = f.read(&mut buf[..take]).ok()?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
                left -= n as u64;
            }
            Some(h.finalize().into())
        };
        let mut by_head: HashMap<[u8; 32], Vec<&PathBuf>> = HashMap::new();
        for p in files {
            if let Some(k) = hash(p, Some(65_536)) {
                by_head.entry(k).or_default().push(p);
            }
        }
        for (_, same) in by_head.into_iter().filter(|(_, v)| v.len() > 1) {
            let mut by_full: HashMap<[u8; 32], Vec<PathBuf>> = HashMap::new();
            for p in same {
                budget = budget.saturating_sub(len);
                if let Some(k) = if len <= 65_536 { hash(p, Some(65_536)) } else { hash(p, None) } {
                    by_full.entry(k).or_default().push(p.clone());
                }
            }
            out.extend(by_full.into_values().filter(|v| v.len() > 1).map(|v| (len, v)));
        }
        if out.len() >= 50 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_scan_finds_dupes() {
        let d = std::env::temp_dir().join(format!("fc-sum-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("a/x.jpg"), vec![1u8; 5000]).unwrap();
        std::fs::write(d.join("a/b/y.jpg"), vec![1u8; 5000]).unwrap();
        std::fs::write(d.join("z.txt"), vec![2u8; 5000]).unwrap();
        std::fs::write(d.join("klein.rs"), "fn main() {}").unwrap();
        let st = scan_folder(&d, &AtomicU64::new(0), &AtomicBool::new(false)).unwrap();
        assert_eq!((st.files, st.dirs), (4, 2));
        assert_eq!(st.dupes.len(), 1);
        assert_eq!(st.dupes[0].1.len(), 2);
        assert_eq!(st.kinds[0].0, "Bilder");
        assert!(supported(&d.join("klein.rs")) && supported(&d));
        let t = read_text(&d.join("klein.rs")).unwrap();
        assert_eq!((t.lines, t.words), (1, 3));
        let _ = std::fs::remove_dir_all(&d);
    }
}
