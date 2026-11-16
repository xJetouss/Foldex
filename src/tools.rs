//! Analyse-Werkzeuge: Ordner vergleichen, Duplikate finden, Speicherplatz analysieren, große Dateien,
//! Auswahl nach Regeln. Alles Langlaufende läuft in einem Hintergrund-Thread mit Fortschritt und Abbruch.

use regex::Regex;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    hash::Hasher,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        mpsc, Arc,
    },
};

// ───────────────────────────── Hintergrund-Aufträge ─────────────────────────────

pub struct Task {
    pub label: String,
    pub progress: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
    pub rx: mpsc::Receiver<Output>,
}

pub enum Output {
    Compare(Vec<Diff>),
    Dups(Vec<DupGroup>),
    Space(Space),
    Error(String),
}

impl Task {
    pub fn spawn(label: &str, ctx: &eframe::egui::Context, f: impl FnOnce(&AtomicU64, &AtomicBool) -> Output + Send + 'static) -> Task {
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c, ctx) = (progress.clone(), cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let out = f(&p, &c);
            let _ = tx.send(out);
            ctx.request_repaint();
        });
        Task { label: label.into(), progress, cancel, rx }
    }
}

/// Alle Dateien unter `root` (relativer Pfad, Größe, Änderungszeit). Symlinks werden nicht verfolgt.
fn files_under(root: &Path, progress: &AtomicU64, cancel: &AtomicBool) -> Vec<(PathBuf, u64, i64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if cancel.load(Relaxed) {
            break;
        }
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
            } else if let Ok(m) = e.metadata() {
                let mt = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
                out.push((p.strip_prefix(root).unwrap_or(&p).to_path_buf(), m.len(), mt));
                progress.fetch_add(1, Relaxed);
            }
        }
    }
    out
}

fn hash_file(p: &Path, limit: Option<u64>, cancel: &AtomicBool) -> Option<u64> {
    let mut f = fs::File::open(p).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut left = limit.unwrap_or(u64::MAX);
    while left > 0 {
        if cancel.load(Relaxed) {
            return None;
        }
        let want = buf.len().min(left as usize);
        let n = f.read(&mut buf[..want]).ok()?;
        if n == 0 {
            break;
        }
        h.write(&buf[..n]);
        left -= n as u64;
    }
    Some(h.finish())
}

// ───────────────────────────── Ordner vergleichen ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiffKind {
    OnlyLeft,
    OnlyRight,
    Different,
}

pub struct Diff {
    pub rel: PathBuf,
    pub kind: DiffKind,
    pub left: Option<(u64, i64)>,
    pub right: Option<(u64, i64)>,
}

/// Vergleicht rekursiv nach relativem Pfad; gleiche Größe → optional Inhaltsvergleich (sonst Änderungszeit).
pub fn compare(a: &Path, b: &Path, content: bool, progress: &AtomicU64, cancel: &AtomicBool) -> Vec<Diff> {
    let la: HashMap<PathBuf, (u64, i64)> = files_under(a, progress, cancel).into_iter().map(|(p, s, m)| (p, (s, m))).collect();
    let lb: HashMap<PathBuf, (u64, i64)> = files_under(b, progress, cancel).into_iter().map(|(p, s, m)| (p, (s, m))).collect();
    let mut out = Vec::new();
    for (rel, &l) in &la {
        if cancel.load(Relaxed) {
            break;
        }
        match lb.get(rel) {
            None => out.push(Diff { rel: rel.clone(), kind: DiffKind::OnlyLeft, left: Some(l), right: None }),
            Some(&r) => {
                let differs = if l.0 != r.0 {
                    true
                } else if content {
                    hash_file(&a.join(rel), None, cancel) != hash_file(&b.join(rel), None, cancel)
                } else {
                    (l.1 - r.1).abs() > 2 // FAT/Netzlaufwerke runden auf 2 s
                };
                if differs {
                    out.push(Diff { rel: rel.clone(), kind: DiffKind::Different, left: Some(l), right: Some(r) });
                }
            }
        }
    }
    for (rel, &r) in &lb {
        if !la.contains_key(rel) {
            out.push(Diff { rel: rel.clone(), kind: DiffKind::OnlyRight, left: None, right: Some(r) });
        }
    }
    out.sort_by(|x, y| x.rel.cmp(&y.rel));
    out
}

// ───────────────────────────── Duplikate ─────────────────────────────

pub struct DupGroup {
    pub size: u64,
    pub files: Vec<PathBuf>,
}

/// Duplikate in drei Stufen: gleiche Größe → gleicher Anfang (64 KB) → gleicher Gesamtinhalt.
pub fn duplicates(root: &Path, min_size: u64, progress: &AtomicU64, cancel: &AtomicBool) -> Vec<DupGroup> {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for (rel, size, _) in files_under(root, progress, cancel) {
        if size >= min_size.max(1) {
            by_size.entry(size).or_default().push(root.join(rel));
        }
    }
    let mut groups = Vec::new();
    for (size, v) in by_size.into_iter().filter(|(_, v)| v.len() > 1) {
        let mut by_head: HashMap<u64, Vec<PathBuf>> = HashMap::new();
        for p in v {
            if let Some(h) = hash_file(&p, Some(64 * 1024), cancel) {
                by_head.entry(h).or_default().push(p);
            }
        }
        for (_, v) in by_head.into_iter().filter(|(_, v)| v.len() > 1) {
            let mut by_full: HashMap<u64, Vec<PathBuf>> = HashMap::new();
            for p in v {
                let h = if size <= 64 * 1024 { Some(0) } else { hash_file(&p, None, cancel) };
                if let Some(h) = h {
                    by_full.entry(h).or_default().push(p);
                }
            }
            for (_, mut files) in by_full.into_iter().filter(|(_, v)| v.len() > 1) {
                files.sort();
                groups.push(DupGroup { size, files });
            }
        }
        if cancel.load(Relaxed) {
            break;
        }
    }
    // größte Ersparnis zuerst
    groups.sort_by_key(|g| std::cmp::Reverse(g.size * (g.files.len() as u64 - 1)));
    groups
}

// ───────────────────────────── Speicherplatz ─────────────────────────────

pub struct Space {
    pub root: PathBuf,
    pub total: u64,
    /// direkte Unterordner/Dateien mit Gesamtgröße, größte zuerst
    pub children: Vec<(PathBuf, u64, bool)>,
    /// größte Dateien im gesamten Baum
    pub largest: Vec<(PathBuf, u64)>,
}

pub fn space(root: &Path, progress: &AtomicU64, cancel: &AtomicBool) -> Space {
    let mut children: BTreeMap<PathBuf, (u64, bool)> = BTreeMap::new();
    let mut largest: Vec<(PathBuf, u64)> = Vec::new();
    let mut total = 0;
    for (rel, size, _) in files_under(root, progress, cancel) {
        total += size;
        let mut comps = rel.components();
        let first = comps.next().map(|c| PathBuf::from(c.as_os_str())).unwrap_or_default();
        let is_dir = comps.next().is_some();
        let e = children.entry(root.join(first)).or_insert((0, is_dir));
        e.0 += size;
        e.1 |= is_dir;
        largest.push((root.join(&rel), size));
        if largest.len() > 2000 {
            largest.sort_by_key(|x| std::cmp::Reverse(x.1));
            largest.truncate(200);
        }
    }
    largest.sort_by_key(|x| std::cmp::Reverse(x.1));
    largest.truncate(200);
    let mut children: Vec<(PathBuf, u64, bool)> = children.into_iter().map(|(p, (s, d))| (p, s, d)).collect();
    children.sort_by_key(|c| std::cmp::Reverse(c.1));
    Space { root: root.to_path_buf(), total, children, largest }
}

// ───────────────────────────── Auswahl nach Regeln ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum RuleKind {
    #[default]
    Name,
    Ext,
    SizeMin,
    SizeMax,
    NewerDays,
    OlderDays,
    Regex,
}

impl RuleKind {
    pub const ALL: [RuleKind; 7] =
        [RuleKind::Name, RuleKind::Ext, RuleKind::SizeMin, RuleKind::SizeMax, RuleKind::NewerDays, RuleKind::OlderDays, RuleKind::Regex];
    pub fn label(self) -> &'static str {
        match self {
            RuleKind::Name => "Name enthält / Muster (*, ?)",
            RuleKind::Ext => "Dateiendung (z. B. jpg, png)",
            RuleKind::SizeMin => "Größe mindestens (z. B. 10 MB)",
            RuleKind::SizeMax => "Größe höchstens (z. B. 500 KB)",
            RuleKind::NewerDays => "Geändert in den letzten … Tagen",
            RuleKind::OlderDays => "Älter als … Tage",
            RuleKind::Regex => "Regulärer Ausdruck (Name)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SelMode {
    #[default]
    Replace,
    Add,
    Remove,
}

/// „10 MB“, „1,5gb“, „300k“, „42“ (Bytes) → Bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase().replace(',', ".");
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let num: f64 = s[..split].trim().parse().ok()?;
    let mul = match s[split..].trim().trim_end_matches('b').trim() {
        "" => 1.0,
        "k" | "ki" => 1024.0,
        "m" | "mi" => 1024.0 * 1024.0,
        "g" | "gi" => 1024.0 * 1024.0 * 1024.0,
        "t" | "ti" => 1024.0f64.powi(4),
        _ => return None,
    };
    Some((num * mul) as u64)
}

fn glob_to_regex(pat: &str) -> String {
    let mut r = String::from("(?i)^");
    for c in pat.chars() {
        match c {
            '*' => r.push_str(".*"),
            '?' => r.push('.'),
            c => r.push_str(&regex::escape(&c.to_string())),
        }
    }
    r.push('$');
    r
}

/// Prüft eine Regel; Fehler sind verständliche Meldungen für den Dialog.
pub fn rule_matcher(kind: RuleKind, value: &str) -> Result<Box<dyn Fn(&str, bool, u64, i64) -> bool>, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err("Bitte einen Wert eingeben.".into());
    }
    let now = chrono::Local::now().timestamp();
    Ok(match kind {
        RuleKind::Name => {
            if v.contains('*') || v.contains('?') {
                let re = Regex::new(&glob_to_regex(v)).map_err(|e| e.to_string())?;
                Box::new(move |n, _, _, _| re.is_match(n))
            } else {
                let needle = v.to_lowercase();
                Box::new(move |n, _, _, _| n.to_lowercase().contains(&needle))
            }
        }
        RuleKind::Ext => {
            let exts: Vec<String> = v.split([',', ' ', ';']).map(|e| e.trim().trim_start_matches("*.").trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()).collect();
            Box::new(move |n, dir, _, _| {
                !dir && n.rsplit_once('.').is_some_and(|(_, e)| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
            })
        }
        RuleKind::SizeMin | RuleKind::SizeMax => {
            let lim = parse_size(v).ok_or("Größe nicht erkannt – Beispiele: 10 MB, 500 KB, 2 GB.")?;
            let min = kind == RuleKind::SizeMin;
            Box::new(move |_, dir, size, _| !dir && if min { size >= lim } else { size <= lim })
        }
        RuleKind::NewerDays | RuleKind::OlderDays => {
            let days: f64 = v.replace(',', ".").parse().map_err(|_| "Bitte eine Anzahl Tage eingeben, z. B. 7.")?;
            let cut = now - (days * 86400.0) as i64;
            let newer = kind == RuleKind::NewerDays;
            Box::new(move |_, _, _, mt| if newer { mt >= cut } else { mt < cut })
        }
        RuleKind::Regex => {
            let re = Regex::new(v).map_err(|e| format!("Ungültiger regulärer Ausdruck: {e}"))?;
            Box::new(move |n, _, _, _| re.is_match(n))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_rules() {
        assert_eq!(parse_size("10 MB"), Some(10 * 1024 * 1024));
        assert_eq!(parse_size("1,5k"), Some(1536));
        assert_eq!(parse_size("42"), Some(42));
        assert_eq!(parse_size("zehn"), None);
        let m = rule_matcher(RuleKind::Name, "*.RS").unwrap();
        assert!(m("main.rs", false, 0, 0));
        assert!(!m("main.rsx", false, 0, 0));
        let e = rule_matcher(RuleKind::Ext, "jpg, .png").unwrap();
        assert!(e("a.PNG", false, 0, 0) && !e("png", true, 0, 0));
        assert!(rule_matcher(RuleKind::Regex, "(").is_err());
    }

    #[test]
    fn compare_and_dups() {
        let base = std::env::temp_dir().join(format!("fc-tools-{}", std::process::id()));
        let (a, b) = (base.join("a"), base.join("b"));
        fs::create_dir_all(a.join("sub")).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("same.txt"), "x").unwrap();
        fs::write(a.join("sub/dup.txt"), "x").unwrap();
        fs::write(a.join("only.txt"), "1").unwrap();
        fs::write(b.join("same.txt"), "y").unwrap();
        let (p, c) = (AtomicU64::new(0), AtomicBool::new(false));
        let d = compare(&a, &b, true, &p, &c);
        assert!(d.iter().any(|d| d.kind == DiffKind::Different && d.rel == Path::new("same.txt")));
        assert!(d.iter().any(|d| d.kind == DiffKind::OnlyLeft && d.rel == Path::new("only.txt")));
        let g = duplicates(&a, 1, &p, &c);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].files.len(), 2);
        let s = space(&a, &p, &c);
        assert_eq!(s.total, 3);
        let _ = fs::remove_dir_all(base);
    }
}
