//! Externe Editoren/IDEs, KI-Agenten für die Kommandozeile und Shells für das Terminal:
//! Installationspfade automatisch erkennen; eigene Programmpfade aus den Einstellungen haben Vorrang.

use std::path::{Path, PathBuf};

/// Ein bekannter Editor: wo er üblicherweise installiert ist.
pub struct EditorDef {
    pub key: &'static str,
    pub name: &'static str,
    /// im Menü angeboten, solange der Benutzer nichts anderes einstellt
    pub default_on: bool,
    /// relativ zu %LOCALAPPDATA%
    local: &'static [&'static str],
    /// relativ zu %ProgramFiles% bzw. %ProgramFiles(x86)%
    pf: &'static [&'static str],
    /// JetBrains: (Ordner-Präfix, Programm in bin\, Toolbox-Skript)
    jb: Option<(&'static str, &'static str, &'static str)>,
    /// Befehle im PATH
    cmds: &'static [&'static str],
    /// macOS-Pfade
    mac: &'static [&'static str],
}

const fn ed(key: &'static str, name: &'static str, local: &'static [&'static str], pf: &'static [&'static str], cmds: &'static [&'static str], mac: &'static [&'static str]) -> EditorDef {
    EditorDef { key, name, default_on: true, local, pf, jb: None, cmds, mac }
}

const fn jb(key: &'static str, name: &'static str, prefix: &'static str, exe: &'static str, script: &'static str) -> EditorDef {
    EditorDef { key, name, default_on: true, local: &[], pf: &[], jb: Some((prefix, exe, script)), cmds: &[], mac: &[] }
}

pub const EDITORS: &[EditorDef] = &[
    ed("vscode", "Visual Studio Code", &[r"Programs\Microsoft VS Code\Code.exe"], &[r"Microsoft VS Code\Code.exe"], &["code"], &["/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"]),
    ed(
        "vscode-insiders",
        "VS Code Insiders",
        &[r"Programs\Microsoft VS Code Insiders\Code - Insiders.exe"],
        &[r"Microsoft VS Code Insiders\Code - Insiders.exe"],
        &["code-insiders"],
        &[],
    ),
    ed("cursor", "Cursor", &[r"Programs\cursor\Cursor.exe"], &[r"Cursor\Cursor.exe"], &["cursor"], &["/Applications/Cursor.app/Contents/Resources/app/bin/cursor"]),
    ed("windsurf", "Windsurf", &[r"Programs\Windsurf\Windsurf.exe"], &[r"Windsurf\Windsurf.exe"], &["windsurf"], &[]),
    ed("zed", "Zed", &[r"Programs\Zed\Zed.exe", r"Zed\zed.exe"], &[r"Zed\Zed.exe"], &["zed"], &["/Applications/Zed.app/Contents/MacOS/cli"]),
    ed("sublime", "Sublime Text", &[], &[r"Sublime Text\sublime_text.exe", r"Sublime Text 3\sublime_text.exe"], &["subl"], &["/Applications/Sublime Text.app/Contents/SharedSupport/bin/subl"]),
    ed("notepadpp", "Notepad++", &[], &[r"Notepad++\notepad++.exe"], &["notepad++"], &[]),
    jb("idea", "IntelliJ IDEA", "IntelliJ IDEA", "idea64.exe", "idea"),
    jb("rustrover", "RustRover", "RustRover", "rustrover64.exe", "rustrover"),
    jb("webstorm", "WebStorm", "WebStorm", "webstorm64.exe", "webstorm"),
    jb("pycharm", "PyCharm", "PyCharm", "pycharm64.exe", "pycharm"),
    jb("clion", "CLion", "CLion", "clion64.exe", "clion"),
    jb("goland", "GoLand", "GoLand", "goland64.exe", "goland"),
    jb("rider", "Rider", "JetBrains Rider", "rider64.exe", "rider"),
    EditorDef { key: "notepad", name: "Windows-Editor", default_on: false, local: &[], pf: &[], jb: None, cmds: &["notepad"], mac: &[] },
];

pub fn def(key: &str) -> Option<&'static EditorDef> {
    EDITORS.iter().find(|d| d.key == key)
}

/// Sucht ein Programm im PATH (unter Windows auch mit .exe/.cmd).
pub fn which(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat"] } else { &[""] };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|d| exts.iter().map(|e| d.join(format!("{name}{e}"))).find(|p| p.is_file()))
    })
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

/// Automatisch erkannter Pfad eines bekannten Editors (ohne Benutzereinstellung).
pub fn detect(key: &str) -> Option<PathBuf> {
    let d = def(key)?;
    let mut cands: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Some(l) = env_dir("LOCALAPPDATA") {
            cands.extend(d.local.iter().map(|r| l.join(r)));
        }
        for v in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(p) = env_dir(v) {
                cands.extend(d.pf.iter().map(|r| p.join(r)));
            }
        }
        if let Some((prefix, exe, script)) = d.jb {
            // JetBrains Toolbox (Skript bzw. %LOCALAPPDATA%\Programs\<Produkt>) und Einzelinstallation (%ProgramFiles%\JetBrains\<Produkt Jahr>)
            if let Some(l) = env_dir("LOCALAPPDATA") {
                cands.push(l.join(format!(r"JetBrains\Toolbox\scripts\{script}.cmd")));
                cands.extend(product_dirs(&l.join("Programs"), prefix).into_iter().map(|p| p.join("bin").join(exe)));
            }
            if let Some(p) = env_dir("ProgramFiles") {
                cands.extend(product_dirs(&p.join("JetBrains"), prefix).into_iter().map(|p| p.join("bin").join(exe)));
            }
        }
    } else if cfg!(target_os = "macos") {
        cands.extend(d.mac.iter().map(PathBuf::from));
    }
    for c in d.cmds {
        if let Some(p) = which(c) {
            cands.push(p);
        }
    }
    cands.into_iter().find(|p| p.is_file())
}

/// Unterordner, deren Name mit `prefix` beginnt – neueste Version zuerst.
fn product_dirs(base: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(base)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix))).collect())
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

/// Wirksamer Pfad: eigener Pfad (falls gesetzt und vorhanden) sonst automatisch erkannt.
pub fn resolve(key: &str, custom: Option<&String>) -> Option<PathBuf> {
    match custom.map(|c| c.trim()).filter(|c| !c.is_empty()) {
        Some(c) => {
            let p = PathBuf::from(c);
            p.is_file().then_some(p)
        }
        None => detect(key),
    }
}

/// Öffnet Datei oder Ordner im Editor (ohne Konsolenfenster unter Windows).
pub fn open(exe: &Path, target: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(target);
    if let Some(dir) = if target.is_dir() { Some(target) } else { target.parent() } {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW (für code.cmd)
    }
    cmd.spawn().map(|_| ()).map_err(|e| format!("{} konnte nicht gestartet werden: {e}", exe.display()))
}

/// Windows-Dialog „Öffnen mit“ für eine Datei.
pub fn open_with_dialog(target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32.exe")
            .arg("shell32.dll,OpenAs_RunDLL")
            .arg(target)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        open::that(target).map_err(|e| e.to_string())
    }
}

/// Typische Textdateien (für „Öffnen mit“ im Kontextmenü).
pub fn is_text(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    const T: &[&str] = &[
        "txt", "md", "markdown", "rs", "toml", "json", "jsonc", "yaml", "yml", "xml", "html", "htm", "css", "scss", "js", "mjs", "cjs",
        "ts", "tsx", "jsx", "py", "pyw", "java", "kt", "kts", "go", "c", "h", "cpp", "hpp", "cc", "cs", "php", "rb", "lua", "sh", "bash",
        "zsh", "ps1", "psm1", "bat", "cmd", "ini", "cfg", "conf", "env", "log", "csv", "tsv", "sql", "gradle", "properties", "lock",
        "gitignore", "gitattributes", "editorconfig", "dockerfile", "svelte", "vue", "astro", "dart", "swift", "zig", "nim", "r", "tex",
    ];
    T.contains(&ext.as_str()) || matches!(name, "Dockerfile" | "Makefile" | "LICENSE" | "README" | ".env" | ".gitignore")
}

// ───────────────────────────── Shells ─────────────────────────────

/// Verfügbare Shells: (Anzeigename, Programm). Die erste ist der Standard.
pub fn shells() -> Vec<(String, String)> {
    let mut v = Vec::new();
    if cfg!(windows) {
        if let Some(p) = which("pwsh") {
            v.push(("PowerShell 7".to_string(), p.to_string_lossy().into_owned()));
        }
        v.push(("Windows PowerShell".to_string(), "powershell.exe".to_string()));
        v.push(("Eingabeaufforderung (CMD)".to_string(), "cmd.exe".to_string()));
        if let Some(p) = env_dir("ProgramFiles").map(|p| p.join(r"Git\bin\bash.exe")).filter(|p| p.is_file()) {
            v.push(("Git Bash".to_string(), p.to_string_lossy().into_owned()));
        }
        if let Some(p) = which("wsl") {
            v.push(("WSL".to_string(), p.to_string_lossy().into_owned()));
        }
    } else {
        let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        v.push((Path::new(&sh).file_name().map_or("Shell".into(), |n| n.to_string_lossy().into_owned()), sh));
        for s in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
            if Path::new(s).is_file() && !v.iter().any(|(_, p)| p == s) {
                v.push((s.rsplit('/').next().unwrap().to_string(), s.to_string()));
            }
        }
    }
    v
}

// ───────────────────────────── Monitore ─────────────────────────────

/// Angeschlossene Monitore: (Name, Arbeitsbereich in physischen Pixeln x, y, b, h, primär?).
#[cfg(windows)]
pub fn monitors() -> Vec<(String, [i32; 4], bool)> {
    use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW};
    unsafe extern "system" fn cb(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let out = &mut *(data as *mut Vec<(String, [i32; 4], bool)>);
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(m, &mut info as *mut MONITORINFOEXW as *mut _) != 0 {
            let r = info.monitorInfo.rcWork;
            let name = String::from_utf16_lossy(&info.szDevice).trim_end_matches('\0').trim_start_matches(r"\\.\").to_string();
            out.push((name, [r.left, r.top, r.right - r.left, r.bottom - r.top], info.monitorInfo.dwFlags & 1 != 0));
        }
        1
    }
    let mut v: Vec<(String, [i32; 4], bool)> = Vec::new();
    unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(cb), &mut v as *mut _ as LPARAM);
    }
    // Primärer zuerst, dann von links nach rechts
    v.sort_by_key(|m| (!m.2, m.1[0], m.1[1]));
    v
}

#[cfg(not(windows))]
pub fn monitors() -> Vec<(String, [i32; 4], bool)> {
    Vec::new()
}

/// Monitor unter einem Bildschirmpunkt (physische Pixel): Arbeitsbereich und Skalierung (1.0 = 100 %).
#[cfg(windows)]
pub fn monitor_at(px: [i32; 2]) -> Option<([i32; 4], f32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    unsafe {
        let m = MonitorFromPoint(POINT { x: px[0], y: px[1] }, MONITOR_DEFAULTTONEAREST);
        if m.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(m, &mut info) == 0 {
            return None;
        }
        let r = info.rcWork;
        let (mut dx, mut dy) = (96u32, 96u32);
        let scale = if GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) == 0 { dx as f32 / 96.0 } else { 1.0 };
        Some(([r.left, r.top, r.right - r.left, r.bottom - r.top], scale))
    }
}

#[cfg(not(windows))]
pub fn monitor_at(_px: [i32; 2]) -> Option<([i32; 4], f32)> {
    None
}

/// Fensterrechteck (physische Pixel) für ein neues Fenster, das am Punkt `px` erscheinen soll:
/// Größe passend zur Skalierung dieses Monitors, vollständig im Arbeitsbereich.
pub fn window_at_point(px: [i32; 2], grab: [f32; 2]) -> Option<[i32; 4]> {
    let (work, s) = monitor_at(px)?;
    let w = ((1100.0 * s) as i32).min(work[2]);
    let h = ((700.0 * s) as i32).min(work[3]);
    let x = (px[0] - (grab[0] * s) as i32).clamp(work[0], work[0] + work[2] - w);
    let y = (px[1] - (grab[1] * s) as i32).clamp(work[1], work[1] + work[3] - h);
    Some([x, y, w, h])
}

/// Verschiebt das eigene Fenster (physische Pixel) – genau, auch bei unterschiedlicher Skalierung je Monitor.
#[cfg(windows)]
pub fn place_window(frame: &eframe::Frame, rect: [i32; 4]) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};
    let Ok(h) = frame.window_handle() else { return false };
    let RawWindowHandle::Win32(w) = h.as_raw() else { return false };
    unsafe { SetWindowPos(w.hwnd.get() as _, std::ptr::null_mut(), rect[0], rect[1], rect[2], rect[3], SWP_NOZORDER | SWP_NOACTIVATE) != 0 }
}

/// Fenster-Handle (Windows), 0 wenn unbekannt.
#[cfg(windows)]
pub fn hwnd(frame: &eframe::Frame) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(w)) => w.hwnd.get(),
        _ => 0,
    }
}

#[cfg(not(windows))]
pub fn hwnd(_frame: &eframe::Frame) -> isize {
    0
}

#[cfg(not(windows))]
pub fn place_window(_frame: &eframe::Frame, _rect: [i32; 4]) -> bool {
    false
}

/// Fensterrechteck für einen Monitor-Arbeitsbereich: 80 % der Fläche, zentriert.
pub fn window_on(work: [i32; 4]) -> [i32; 4] {
    let (w, h) = ((work[2] as f32 * 0.8) as i32, (work[3] as f32 * 0.8) as i32);
    [work[0] + (work[2] - w) / 2, work[1] + (work[3] - h) / 2, w, h]
}
