//! Cloud-Speicher: Google Drive, iCloud Drive, OneDrive, Dropbox, MEGA, Proton Drive, pCloud, Box, Nextcloud.
//!
//! Unter Windows bringen die Anbieter eigene Programme mit, die einen Ordner bzw. ein Laufwerk anlegen. Die
//! Dateien sind dort ganz normale Dateien (Windows lädt „nur online“ verfügbare bei Bedarf nach), deshalb
//! arbeitet Foldex direkt mit diesen Ordnern: erkennen, in der Seitenleiste zeigen, Sync-Status anzeigen und
//! „Immer auf diesem Gerät behalten“ bzw. „Speicherplatz freigeben“ setzen. Fehlt ein Anbieter, führt
//! „Einrichten“ zu seinem offiziellen Programm.

use eframe::egui::{self, pos2, vec2, Color32, Painter, Rect, Shape, Stroke};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Provider {
    pub key: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    /// offizielle Download-Seite bzw. Store-Eintrag
    pub setup: &'static str,
}

pub static PROVIDERS: &[Provider] = &[
    Provider { key: "gdrive", name: "Google Drive", desc: "Mit dem Google-Konto anmelden (Browser)", setup: "https://www.google.com/drive/download/" },
    Provider { key: "icloud", name: "iCloud Drive", desc: "Mit der Apple-ID anmelden, danach Bestätigungscode", setup: "ms-windows-store://pdp/?productid=9PKTQ5699M62" },
    Provider { key: "onedrive", name: "OneDrive", desc: "Mit dem Microsoft-Konto anmelden (Browser)", setup: "https://www.microsoft.com/microsoft-365/onedrive/download" },
    Provider { key: "dropbox", name: "Dropbox", desc: "Mit dem Dropbox-Konto anmelden (Browser)", setup: "https://www.dropbox.com/install" },
    Provider { key: "mega", name: "MEGA", desc: "Mit E-Mail und Passwort anmelden (verschlüsselt, 20 GB frei)", setup: "https://mega.io/desktop" },
    Provider { key: "proton", name: "Proton Drive", desc: "Mit Proton-Konto anmelden (Ende-zu-Ende-verschlüsselt)", setup: "https://proton.me/drive/download" },
    Provider { key: "pcloud", name: "pCloud", desc: "Mit dem pCloud-Konto anmelden (Browser)", setup: "https://www.pcloud.com/download-free-online-cloud-file-storage.html" },
    Provider { key: "box", name: "Box", desc: "Mit dem Box-Konto anmelden (Browser)", setup: "https://www.box.com/resources/downloads" },
    Provider { key: "nextcloud", name: "Nextcloud", desc: "Server-Adresse, Benutzer und App-Passwort", setup: "https://nextcloud.com/install/#install-clients" },
];

pub fn provider(key: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.key == key)
}

/// ein erkannter Cloud-Ordner
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// Schlüssel des Anbieters (oder „other“)
    pub key: &'static str,
    pub name: String,
    pub path: PathBuf,
    /// in Foldex angemeldetes Konto (rclone-Laufwerk) – sonst Ordner eines Anbieter-Programms
    pub remote: Option<String>,
}

/// erkannte Cloud-Ordner. Ermittelt wird im Hintergrund (alle 30 s) – Laufwerksabfragen können bei
/// getrennten Netzlaufwerken dauern und sollen die Oberfläche nicht anhalten.
struct Cache {
    at: Option<Instant>,
    list: Vec<Place>,
    busy: bool,
}
static PLACES: Mutex<Cache> = Mutex::new(Cache { at: None, list: Vec::new(), busy: false });

pub fn places() -> Vec<Place> {
    let mut g = PLACES.lock().unwrap_or_else(|e| e.into_inner());
    if !g.busy && g.at.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
        g.busy = true;
        std::thread::spawn(|| {
            let list = detect();
            let mut g = PLACES.lock().unwrap_or_else(|e| e.into_inner());
            g.list = list;
            g.at = Some(Instant::now());
            g.busy = false;
        });
    }
    let mut list = g.list.clone();
    drop(g);
    // angemeldete Konten sofort zeigen – auch bevor die Suche fertig bzw. das Laufwerk eingebunden ist
    let mut own: Vec<Place> = Vec::new();
    for a in crate::cloudlogin::accounts().into_iter().filter(|a| !a.off) {
        if !list.iter().any(|p| p.remote.as_deref() == Some(a.remote.as_str())) {
            if let Some(pr) = provider(&a.key) {
                own.push(Place { key: pr.key, name: a.name.clone(), path: a.path(), remote: Some(a.remote.clone()) });
            }
        }
    }
    if !own.is_empty() {
        list.retain(|p| !own.iter().any(|o| o.path == p.path || o.key == p.key && p.remote.is_none()));
        own.extend(list);
        list = own;
    }
    sort_by_order(&mut list);
    list
}

/// liegt der Pfad in einem Cloud-Ordner?
pub fn place_of(p: &Path) -> Option<Place> {
    let l = p.to_string_lossy().to_lowercase();
    places().into_iter().find(|c| {
        let r = c.path.to_string_lossy().to_lowercase();
        l == r || l.starts_with(&format!("{}\\", r.trim_end_matches('\\'))) || l.starts_with(&format!("{}/", r.trim_end_matches('/')))
    })
}

fn detect() -> Vec<Place> {
    let mut v: Vec<Place> = Vec::new();
    let mut add = |key: &'static str, name: String, path: PathBuf| {
        if path.is_dir() && !v.iter().any(|x| x.path == path) {
            v.push(Place { key, name, path, remote: None });
        }
    };
    // 0. in Foldex angemeldete Konten (als Laufwerk eingebunden)
    let mut own: Vec<Place> = Vec::new();
    for a in crate::cloudlogin::mounted() {
        if let Some(pr) = provider(&a.key) {
            own.push(Place { key: pr.key, name: a.name.clone(), path: a.path(), remote: Some(a.remote.clone()) });
        }
    }
    // 1. Sync-Ordner, die Windows kennt (Cloud-Files-API: OneDrive, iCloud, Dropbox, Box, Proton …)
    for (id, path) in sync_roots() {
        let pre = id.split('!').next().unwrap_or(&id).to_lowercase();
        let (key, name): (&'static str, String) = if pre.contains("onedrive") {
            ("onedrive", if id.to_lowercase().contains("business") { "OneDrive – Geschäftlich".into() } else { "OneDrive".into() })
        } else if pre.contains("icloud") {
            ("icloud", "iCloud Drive".into())
        } else if pre.contains("dropbox") {
            ("dropbox", "Dropbox".into())
        } else if pre.contains("box") {
            ("box", "Box".into())
        } else if pre.contains("proton") {
            ("proton", "Proton Drive".into())
        } else if pre.contains("google") || pre.contains("drivefs") {
            ("gdrive", "Google Drive".into())
        } else if pre.contains("nextcloud") {
            ("nextcloud", "Nextcloud".into())
        } else if pre.contains("mega") {
            ("mega", "MEGA".into())
        } else {
            ("other", id.split('!').next().unwrap_or("Cloud").to_string())
        };
        add(key, name, path);
    }
    let home = dirs::home_dir().unwrap_or_default();
    // 2. bekannte Orte der Programme
    for (var, name) in [("OneDriveConsumer", "OneDrive"), ("OneDriveCommercial", "OneDrive – Geschäftlich")] {
        if let Ok(p) = std::env::var(var) {
            add("onedrive", name.into(), PathBuf::from(p));
        }
    }
    add("icloud", "iCloud Drive".into(), home.join("iCloudDrive"));
    add("icloud", "iCloud-Fotos".into(), home.join("Pictures").join("iCloud Photos"));
    for f in [dirs::data_local_dir(), dirs::data_dir()].into_iter().flatten().map(|d| d.join("Dropbox").join("info.json")) {
        if let Some(j) = std::fs::read_to_string(f).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) {
            for (k, n) in [("personal", "Dropbox"), ("business", "Dropbox – Business")] {
                if let Some(p) = j.pointer(&format!("/{k}/path")).and_then(|v| v.as_str()) {
                    add("dropbox", n.into(), PathBuf::from(p));
                }
            }
        }
    }
    add("dropbox", "Dropbox".into(), home.join("Dropbox"));
    add("gdrive", "Google Drive".into(), home.join("My Drive"));
    add("gdrive", "Google Drive".into(), home.join("Meine Ablage"));
    add("mega", "MEGA".into(), home.join("MEGA"));
    add("mega", "MEGA".into(), home.join("Documents").join("MEGA"));
    add("box", "Box".into(), home.join("Box"));
    add("nextcloud", "Nextcloud".into(), home.join("Nextcloud"));
    add("proton", "Proton Drive".into(), home.join("Proton Drive"));
    // 3. Laufwerke mit Namen eines Anbieters (Google Drive, pCloud)
    let mine = crate::cloudlogin::letters();
    for (letter, label) in volume_labels() {
        if mine.contains(&letter) {
            continue; // in Foldex angemeldetes Konto (heißt z. B. auch „Google Drive“)
        }
        let l = label.to_lowercase();
        let root = PathBuf::from(format!("{letter}:\\"));
        if l.contains("google drive") {
            // „My Drive“ bzw. „Meine Ablage“ ist der eigentliche Speicher
            let inner = ["My Drive", "Meine Ablage"].iter().map(|n| root.join(n)).find(|p| p.is_dir()).unwrap_or(root.clone());
            add("gdrive", "Google Drive".into(), inner);
            for n in ["Shared drives", "Geteilte Ablagen"] {
                add("gdrive", "Google Drive – geteilt".into(), root.join(n));
            }
        } else if l.contains("pcloud") {
            add("pcloud", "pCloud".into(), root);
        }
    }
    v.retain(|x| !own.iter().any(|o| o.path == x.path));
    // in Foldex angemeldet → der Ordner des Anbieter-Programms ist überflüssig (Foldex-Laufwerk ist immer aktuell)
    v.retain(|x| !own.iter().any(|o| o.key == x.key));
    own.extend(v);
    own
}

// ───────────────────────── Reihenfolge in der Seitenleiste ─────────────────────────

fn order_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("cloud").join("order.txt")
}

static ORDER: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

fn order() -> Vec<PathBuf> {
    let mut g = ORDER.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(|| std::fs::read_to_string(order_file()).map(|s| s.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect()).unwrap_or_default()).clone()
}

/// nach der gespeicherten Reihenfolge sortieren; Neue kommen ans Ende
fn sort_by_order(list: &mut [Place]) {
    let o = order();
    list.sort_by_key(|p| o.iter().position(|x| *x == p.path).unwrap_or(usize::MAX));
}

/// Eintrag `from` an Stelle `to` verschieben (Positionen wie in `places()`) und merken
pub fn move_place(mut list: Vec<PathBuf>, from: usize, to: usize) {
    if from >= list.len() {
        return;
    }
    let p = list.remove(from);
    list.insert(to.min(list.len()), p);
    // Unbekannte frühere Einträge (z. B. gerade getrennte Laufwerke) hinten behalten
    for old in order() {
        if !list.contains(&old) {
            list.push(old);
        }
    }
    let text: String = list.iter().map(|p| format!("{}
", p.display())).collect();
    let f = order_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, text);
    *ORDER.lock().unwrap_or_else(|e| e.into_inner()) = Some(list);
}

/// läuft das Programm des Anbieters? (unbekannte Anbieter: ja) – Prozessliste, 5 s gemerkt
pub fn app_running(key: &str) -> bool {
    let exes: &[&str] = match key {
        "onedrive" => &["onedrive.exe"],
        "icloud" => &["iclouddrive.exe", "icloudservices.exe", "icloudhome.exe"],
        "dropbox" => &["dropbox.exe"],
        "gdrive" => &["googledrivefs.exe"],
        "mega" => &["megasync.exe"],
        "box" => &["box.exe", "boxsync.exe"],
        "proton" => &["protondrive.exe", "proton drive.exe"],
        "nextcloud" => &["nextcloud.exe"],
        "pcloud" => &["pcloud.exe"],
        _ => return true,
    };
    static P: Mutex<Option<(Instant, Vec<(u32, String)>)>> = Mutex::new(None);
    let mut g = P.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_none_or(|(t, _)| t.elapsed() > Duration::from_secs(5)) {
        *g = Some((Instant::now(), processes()));
    }
    // Linux heißen die Programme ohne „.exe“ (onedrive, dropbox, megasync …)
    g.as_ref().is_some_and(|(_, v)| v.iter().any(|(_, p)| exes.iter().any(|e| *e == p || e.trim_end_matches(".exe") == p)))
}

/// Name des Programms mit dieser Prozess-ID (klein geschrieben)
pub fn process_name(pid: u32) -> Option<String> {
    processes().into_iter().find(|(id, _)| *id == pid).map(|(_, n)| n)
}

/// (Prozess-ID, Name klein geschrieben) aller laufenden Programme
fn processes() -> Vec<(u32, String)> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut v = Vec::new();
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let n = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            v.push((e.th32ProcessID, String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase()));
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
        v
    }
    // Linux: /proc/<pid>/comm
    #[cfg(not(windows))]
    {
        let mut v = Vec::new();
        for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if let Ok(n) = std::fs::read_to_string(e.path().join("comm")) {
                v.push((pid, n.trim().to_lowercase()));
            }
        }
        v
    }
}

/// Liste beim nächsten Abruf neu ermitteln (nach An- bzw. Abmelden)
pub fn refresh() {
    PLACES.lock().unwrap_or_else(|e| e.into_inner()).at = None;
}

/// Sync-Ordner aus der Registry: (Kennung, Ordner) für den angemeldeten Benutzer
fn sync_roots() -> Vec<(String, PathBuf)> {
    #[cfg(windows)]
    {
        // über reg.exe gelesen: kein zusätzlicher Registry-Code, und es ist nur alle 30 s
        let mut c = std::process::Command::new("reg");
        c.args(["query", r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager", "/s", "/f", "S-1-", "/d"]);
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        let Ok(out) = c.output() else { return Vec::new() };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut v = Vec::new();
        let mut key = String::new();
        for line in text.lines() {
            if line.starts_with("HKEY_") {
                // …\SyncRootManager\<Kennung>\UserSyncRoots
                key = line.split('\\').rev().nth(1).unwrap_or_default().to_string();
            } else if let Some(i) = line.find("REG_SZ") {
                let path = line[i + 6..].trim();
                if !path.is_empty() && !key.is_empty() {
                    v.push((key.clone(), PathBuf::from(path)));
                }
            }
        }
        v
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// Laufwerksbuchstaben mit ihrem Namen (z. B. G → „Google Drive“)
pub fn volume_labels() -> Vec<(char, String)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;
        let mut v = Vec::new();
        // Cloud-Laufwerke nicht anfragen: die Abfrage kann dort lange hängen
        let skip = crate::cloudlogin::letters();
        for l in ('C'..='Z').filter(|l| !skip.contains(l)) {
            let root: Vec<u16> = format!("{l}:\\").encode_utf16().chain(std::iter::once(0)).collect();
            let mut name = [0u16; 128];
            let ok = unsafe { GetVolumeInformationW(root.as_ptr(), name.as_mut_ptr(), name.len() as u32, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0) };
            if ok != 0 {
                let n = String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(0)]);
                if !n.is_empty() {
                    v.push((l, n));
                }
            }
        }
        v
    }
    #[cfg(not(windows))]
    Vec::new()
}

// ───────────────────────── Sync-Status und Behalten/Freigeben ─────────────────────────

/// Windows-Attribute der Cloud-Dateien
pub const ATTR_PINNED: u32 = 0x0008_0000;
pub const ATTR_UNPINNED: u32 = 0x0010_0000;
pub const ATTR_RECALL_ON_OPEN: u32 = 0x0004_0000;
pub const ATTR_RECALL_ON_DATA: u32 = 0x0040_0000;
pub const ATTR_OFFLINE: u32 = 0x1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sync {
    /// nur in der Cloud, wird beim Öffnen geladen
    Online,
    /// auf diesem Gerät verfügbar
    Local,
    /// immer auf diesem Gerät behalten
    Pinned,
}

pub fn sync_of(attrs: u32) -> Sync {
    if attrs & ATTR_PINNED != 0 {
        Sync::Pinned
    } else if attrs & (ATTR_RECALL_ON_DATA | ATTR_RECALL_ON_OPEN | ATTR_OFFLINE) != 0 {
        Sync::Online
    } else {
        Sync::Local
    }
}

/// „Immer auf diesem Gerät behalten“ (true) bzw. „Speicherplatz freigeben“ (false) – das Sync-Programm lädt
/// dann herunter bzw. entfernt die lokale Kopie (wie im Windows-Explorer)
pub fn set_keep(paths: &[PathBuf], keep: bool) -> Result<usize, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, SetFileAttributesW, INVALID_FILE_ATTRIBUTES};
        let mut n = 0;
        for p in paths {
            let w: Vec<u16> = p.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
            let a = unsafe { GetFileAttributesW(w.as_ptr()) };
            if a == INVALID_FILE_ATTRIBUTES {
                continue;
            }
            let b = if keep { (a | ATTR_PINNED) & !ATTR_UNPINNED } else { (a | ATTR_UNPINNED) & !ATTR_PINNED };
            // nur änderbare Attribute weitergeben (sonst lehnt Windows ab)
            let keepable = 0x1 | 0x2 | 0x4 | 0x20 | 0x80 | 0x100 | 0x2000 | ATTR_PINNED | ATTR_UNPINNED;
            if unsafe { SetFileAttributesW(w.as_ptr(), b & keepable) } != 0 {
                n += 1;
            }
        }
        if n == 0 {
            return Err("Konnte nicht geändert werden – unterstützt dieser Cloud-Dienst das?".into());
        }
        Ok(n)
    }
    #[cfg(not(windows))]
    {
        let _ = (paths, keep);
        Err("nur unter Windows".into())
    }
}

// ───────────────────────── Logos ─────────────────────────

fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// einfaches Logo des Anbieters (für Einträge ohne Windows-Symbol)
pub fn logo(p: &Painter, r: Rect, key: &str) {
    let s = r.width().min(r.height());
    let o = r.center() - vec2(s / 2.0, s / 2.0);
    let pt = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
    let cloud = |col: Color32| {
        p.circle_filled(pt(0.36, 0.56), s * 0.2, col);
        p.circle_filled(pt(0.56, 0.44), s * 0.25, col);
        p.circle_filled(pt(0.74, 0.6), s * 0.17, col);
        p.rect_filled(Rect::from_min_max(pt(0.18, 0.56), pt(0.88, 0.77)), s * 0.1, col);
    };
    match key {
        "gdrive" => {
            // drei Bänder: gelb, grün, blau
            p.add(Shape::convex_polygon(vec![pt(0.38, 0.12), pt(0.62, 0.12), pt(0.92, 0.64), pt(0.68, 0.64)], hex(0xFFBA00), Stroke::NONE));
            p.add(Shape::convex_polygon(vec![pt(0.38, 0.12), pt(0.5, 0.33), pt(0.2, 0.86), pt(0.08, 0.64)], hex(0x00A45F), Stroke::NONE));
            p.add(Shape::convex_polygon(vec![pt(0.32, 0.64), pt(0.92, 0.64), pt(0.8, 0.86), pt(0.2, 0.86)], hex(0x2684FC), Stroke::NONE));
        }
        "icloud" => cloud(hex(0x3693F3)),
        "onedrive" => cloud(hex(0x0F78D4)),
        "dropbox" => {
            let d = |cx: f32, cy: f32| {
                p.add(Shape::convex_polygon(vec![pt(cx, cy - 0.13), pt(cx + 0.2, cy), pt(cx, cy + 0.13), pt(cx - 0.2, cy)], hex(0x0061FE), Stroke::NONE));
            };
            d(0.3, 0.3);
            d(0.7, 0.3);
            d(0.3, 0.58);
            d(0.7, 0.58);
            d(0.5, 0.8);
        }
        "mega" => {
            p.circle_filled(r.center(), s * 0.44, hex(0xD9272E));
            p.text(r.center(), egui::Align2::CENTER_CENTER, "M", egui::FontId::proportional(s * 0.52), Color32::WHITE);
        }
        "proton" => {
            p.rect_filled(Rect::from_min_max(pt(0.12, 0.2), pt(0.88, 0.82)), s * 0.14, hex(0x6D4AFF));
            p.rect_filled(Rect::from_min_max(pt(0.12, 0.2), pt(0.5, 0.36)), s * 0.08, hex(0x8C6BFF));
        }
        "pcloud" => {
            cloud(hex(0x17BED0));
            p.text(pt(0.53, 0.66), egui::Align2::CENTER_CENTER, "p", egui::FontId::proportional(s * 0.38), Color32::WHITE);
        }
        "box" => {
            p.rect_filled(Rect::from_min_max(pt(0.1, 0.22), pt(0.9, 0.78)), s * 0.12, hex(0x0061D5));
            p.text(r.center(), egui::Align2::CENTER_CENTER, "box", egui::FontId::proportional(s * 0.3), Color32::WHITE);
        }
        "nextcloud" => {
            p.circle_stroke(pt(0.5, 0.5), s * 0.16, Stroke::new(s * 0.08, hex(0x0082C9)));
            p.circle_stroke(pt(0.2, 0.5), s * 0.09, Stroke::new(s * 0.06, hex(0x0082C9)));
            p.circle_stroke(pt(0.8, 0.5), s * 0.09, Stroke::new(s * 0.06, hex(0x0082C9)));
        }
        _ => cloud(hex(0x8A94A6)),
    }
}

/// kleines Zeichen für den Sync-Status (unten links am Datei-Symbol)
pub fn badge(p: &Painter, icon: Rect, st: Sync, bg: Color32) {
    let c = pos2(icon.left() + 3.0, icon.bottom() - 3.0);
    let r = 5.5;
    match st {
        Sync::Online => {
            p.circle_filled(c, r, bg);
            let s = Stroke::new(1.2, hex(0x3693F3));
            // kleine Wolke als Umriss
            p.circle_stroke(c + vec2(-1.4, 0.6), 1.9, s);
            p.circle_stroke(c + vec2(1.3, -0.4), 2.3, s);
        }
        Sync::Local => {
            p.circle_filled(c, r, bg);
            p.circle_stroke(c, r - 1.0, Stroke::new(1.2, hex(0x2EA043)));
            check(p, c, hex(0x2EA043));
        }
        Sync::Pinned => {
            p.circle_filled(c, r, hex(0x2EA043));
            check(p, c, Color32::WHITE);
        }
    }
}

fn check(p: &Painter, c: egui::Pos2, col: Color32) {
    p.add(Shape::line(vec![c + vec2(-2.4, 0.1), c + vec2(-0.6, 1.9), c + vec2(2.6, -1.6)], Stroke::new(1.4, col)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_states() {
        assert_eq!(sync_of(0x20), Sync::Local);
        assert_eq!(sync_of(ATTR_RECALL_ON_DATA | ATTR_UNPINNED), Sync::Online);
        assert_eq!(sync_of(ATTR_PINNED), Sync::Pinned);
        assert!(provider("icloud").is_some() && provider("gdrive").is_some());
    }

    /// erkannte Cloud-Ordner auf diesem PC: cargo test cloud_places -- --ignored --nocapture
    #[test]
    #[ignore]
    fn cloud_places() {
        for p in detect() {
            eprintln!("{} · {} · {}", p.key, p.name, p.path.display());
        }
    }
}
