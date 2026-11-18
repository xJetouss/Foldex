use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Entry {
    pub label: String,
    pub path: PathBuf,
}

/// Anpassbare Seitenleiste: angeheftete Ordner (frei sortier-, umbenenn- und entfernbar) + „Dieser PC“.
pub struct SidebarCfg {
    pub pins: Vec<Entry>,
    pub show_pc: bool,
    /// ausgeblendete Laufwerke und Cloud-Einträge
    pub hidden: Vec<PathBuf>,
}

fn cfg_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("sidebar.txt")
}

fn label_of(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

impl SidebarCfg {
    pub fn defaults() -> Self {
        let mut pins = Vec::new();
        let mut add = |label: &str, p: Option<PathBuf>| {
            if let Some(p) = p {
                pins.push(Entry { label: label.to_string(), path: p });
            }
        };
        add("Desktop", dirs::desktop_dir());
        add("Downloads", dirs::download_dir());
        add("Dokumente", dirs::document_dir());
        add("Bilder", dirs::picture_dir());
        add("Musik", dirs::audio_dir());
        add("Videos", dirs::video_dir());
        if pins.is_empty() {
            if let Some(h) = dirs::home_dir() {
                pins.push(Entry { label: "Home".into(), path: h });
            }
        }
        SidebarCfg { pins, show_pc: true, hidden: Vec::new() }
    }

    pub fn load() -> Self {
        let Ok(text) = fs::read_to_string(cfg_file()) else { return Self::defaults() };
        let mut cfg = SidebarCfg { pins: Vec::new(), show_pc: true, hidden: Vec::new() };
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("show_pc=") {
                cfg.show_pc = v.trim() == "1";
            } else if let Some(p) = line.strip_prefix("hide\t") {
                cfg.hidden.push(PathBuf::from(p));
            } else if let Some(rest) = line.strip_prefix("pin\t") {
                if let Some((label, path)) = rest.split_once('\t') {
                    cfg.pins.push(Entry { label: label.to_string(), path: PathBuf::from(path) });
                }
            }
        }
        cfg
    }

    pub fn save(&self) {
        // Bildschirmfoto-Läufe (FOLDEX_SHOT) ändern nie die echten Einstellungen
        if std::env::var_os("FOLDEX_SHOT").is_some() {
            return;
        }
        let mut s = format!("show_pc={}\n", if self.show_pc { 1 } else { 0 });
        for e in &self.pins {
            s.push_str(&format!("pin\t{}\t{}\n", e.label.replace('\t', " "), e.path.display()));
        }
        for p in &self.hidden {
            s.push_str(&format!("hide\t{}\n", p.display()));
        }
        let _ = crate::themes::write_atomic(&cfg_file(), s.as_bytes());
    }

    pub fn pin(&mut self, p: &Path) -> bool {
        if !p.is_dir() || self.pins.iter().any(|e| e.path == p) {
            return false;
        }
        self.pins.push(Entry { label: label_of(p), path: p.to_path_buf() });
        self.save();
        true
    }

    pub fn move_pin(&mut self, from: usize, to: usize) {
        if from < self.pins.len() {
            let e = self.pins.remove(from);
            let to = to.min(self.pins.len());
            self.pins.insert(to, e);
            self.save();
        }
    }
}

/// Laufwerke werden in Hintergrund-Threads gesucht: ein hängendes Netzlaufwerk oder leeres Wechsellaufwerk
/// kann so nie den Programmstart oder die Oberfläche blockieren. Gefundene Laufwerke treffen einzeln ein.
pub fn probe_drives(ctx: &eframe::egui::Context) -> (std::sync::mpsc::Sender<Entry>, std::sync::mpsc::Receiver<Entry>) {
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(windows)]
    for c in 'A'..='Z' {
        probe_letter(&tx, ctx, c);
    }
    #[cfg(not(windows))]
    {
        let (ctx, tx) = (ctx.clone(), tx.clone());
        std::thread::spawn(move || {
            for e in linux_drives() {
                let _ = tx.send(e);
            }
            ctx.request_repaint();
        });
    }
    (tx, rx)
}

/// Linux: Dateisystem, Persönlicher Ordner und eingehängte Datenträger (USB-Sticks, Partitionen, DVDs) –
/// aus /proc/mounts, damit auch /run/media/<Benutzer>/<Stick> und /media/<Benutzer>/<Stick> erscheinen
#[cfg(not(windows))]
pub fn linux_drives() -> Vec<Entry> {
    let mut v = vec![Entry { label: "Dateisystem (/)".into(), path: PathBuf::from("/") }];
    // echte Datenträger, keine System- oder Netz-Pseudodateisysteme (und keine Cloud-Laufwerke von rclone)
    const REAL: [&str; 13] = ["ext4", "ext3", "ext2", "btrfs", "xfs", "vfat", "exfat", "ntfs", "ntfs3", "fuseblk", "iso9660", "udf", "f2fs"];
    let text = fs::read_to_string("/proc/mounts").unwrap_or_default();
    for l in text.lines() {
        let f: Vec<&str> = l.split(' ').collect();
        let (Some(mp), Some(ty)) = (f.get(1), f.get(2)) else { continue };
        let mp = mp.replace("\\040", " ");
        let under = ["/media/", "/mnt/", "/run/media/"].iter().any(|b| mp.starts_with(b));
        if under && REAL.contains(ty) && !v.iter().any(|e| e.path == Path::new(&mp)) {
            let p = PathBuf::from(&mp);
            v.push(Entry { label: label_of(&p), path: p });
        }
    }
    v
}

/// Linux: Fingerabdruck der eingehängten Datenträger (ändert sich beim Einstecken/Auswerfen)
#[cfg(not(windows))]
pub fn mounts_sig() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    linux_drives().iter().for_each(|e| e.path.hash(&mut h));
    h.finish()
}

/// ein Laufwerk prüfen (im Hintergrund) und bei Erfolg melden
pub fn probe_letter(tx: &std::sync::mpsc::Sender<Entry>, ctx: &eframe::egui::Context, c: char) {
    let (tx, ctx) = (tx.clone(), ctx.clone());
    std::thread::spawn(move || {
        let p = PathBuf::from(format!("{c}:\\"));
        if fs::metadata(&p).is_ok() {
            let _ = tx.send(Entry { label: format!("Lokaler Datenträger ({c}:)"), path: p });
            ctx.request_repaint();
        }
    });
}

/// belegte Laufwerksbuchstaben als Bitmaske (Bit 0 = A:) – sofort, ohne Laufwerke anzufassen
pub fn drive_mask() -> u32 {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::Storage::FileSystem::GetLogicalDrives()
    }
    #[cfg(not(windows))]
    0
}

pub struct GitRepo {
    pub name: String,
    pub path: PathBuf,
    /// aktueller Branch bzw. gekürzter Commit (losgelöster HEAD)
    pub branch: Option<String>,
}

/// Sucht Git-Repositories in den angegebenen Ordnern (Tiefe ≤ 3) in einem Hintergrund-Thread.
/// Begrenzt auf wenige tausend Ordner, damit große Home-Verzeichnisse oder Netzlaufwerke nicht lange laufen.
pub fn probe_git(ctx: &eframe::egui::Context, roots: Vec<PathBuf>) -> std::sync::mpsc::Receiver<Vec<GitRepo>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        const SKIP: [&str; 9] = ["node_modules", "target", "AppData", "Library", "build", "dist", "vendor", "venv", "__pycache__"];
        let mut found: Vec<GitRepo> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut budget = 6000usize;
        let mut queue: std::collections::VecDeque<(PathBuf, u8)> = roots.into_iter().map(|r| (r, 0)).collect();
        while let Some((dir, depth)) = queue.pop_front() {
            if found.len() >= 60 || budget == 0 {
                break;
            }
            budget -= 1;
            if !seen.insert(dir.clone()) {
                continue;
            }
            let git = dir.join(".git");
            if git.exists() {
                found.push(GitRepo { name: label_of(&dir), branch: head_of(&git), path: dir });
                continue;
            }
            if depth >= 3 {
                continue;
            }
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                // Symlinks/Junctions nicht verfolgen (Schleifen), versteckte und Build-Ordner überspringen
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir && !name.starts_with('.') && !SKIP.contains(&name.as_ref()) {
                    queue.push_back((e.path(), depth + 1));
                }
            }
        }
        found.sort_by_key(|r| r.name.to_lowercase());
        let _ = tx.send(found);
        ctx.request_repaint();
    });
    rx
}

/// Liest den aktuellen Branch aus `.git/HEAD` (auch für Worktrees/Submodule mit `.git`-Datei).
fn head_of(git: &Path) -> Option<String> {
    let dir = if git.is_file() {
        let t = fs::read_to_string(git).ok()?;
        let p = PathBuf::from(t.trim().strip_prefix("gitdir:")?.trim());
        if p.is_absolute() { p } else { git.parent()?.join(p) }
    } else {
        git.to_path_buf()
    };
    let head = fs::read_to_string(dir.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(r) => Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string()),
        None => Some(head.chars().take(7).collect()),
    }
}

/// Zuletzt verwendete Ordner (neueste zuerst). Wird nur gespeichert, solange der Verlauf eingeschaltet ist.
pub struct Recent {
    pub dirs: Vec<PathBuf>,
}

const RECENT_MAX: usize = 12;

fn recent_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("recent.txt")
}

impl Recent {
    pub fn load() -> Self {
        let dirs = fs::read_to_string(recent_file())
            .map(|t| t.lines().filter(|l| !l.is_empty()).map(PathBuf::from).take(RECENT_MAX).collect())
            .unwrap_or_default();
        Recent { dirs }
    }

    pub fn push(&mut self, p: &Path) {
        if self.dirs.first().is_some_and(|d| d == p) {
            return;
        }
        self.dirs.retain(|d| d != p);
        self.dirs.insert(0, p.to_path_buf());
        self.dirs.truncate(RECENT_MAX);
        self.save();
    }

    pub fn clear(&mut self) {
        self.dirs.clear();
        let _ = fs::remove_file(recent_file());
    }

    fn save(&self) {
        if std::env::var_os("FOLDEX_SHOT").is_some() {
            return;
        }
        let s: String = self.dirs.iter().map(|d| format!("{}\n", d.display())).collect();
        let _ = crate::themes::write_atomic(&recent_file(), s.as_bytes());
    }
}
