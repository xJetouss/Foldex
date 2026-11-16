//! Foldex vollständig entfernen (Einstellungen → „Deinstallieren“): Einstellungen, Verlauf, alle KI-Modelle
//! und -Programme, Cloud-Anmeldungen und -Zwischenspeicher, Suchindex und Caches, Verknüpfungen und – auf
//! Wunsch bzw. wenn installiert – das Programm selbst.
//!
//! Ein laufendes Programm kann sich nicht selbst löschen: Foldex schreibt ein kleines Aufräum-Skript, startet
//! es unsichtbar und beendet sich. Das Skript wartet, bis Foldex zu ist, und löscht dann alles.

use std::path::{Path, PathBuf};

/// Ein Teil, der entfernt wird
pub struct Item {
    pub label: &'static str,
    pub path: PathBuf,
}

fn config() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex")
}

/// Ordner, in den der Installer Foldex legt
pub fn install_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return dirs::data_local_dir().map(|d| d.join("Programs").join("Foldex"));
    #[cfg(not(windows))]
    return dirs::home_dir().map(|h| h.join(".local").join("bin"));
}

/// Läuft diese Foldex.exe aus der Installation (Installer)? Auch in einem selbst gewählten Ordner – dort liegt
/// das Deinstallations-Programm des Installers daneben.
pub fn installed() -> bool {
    let exe = std::env::current_exe().ok();
    #[cfg(windows)]
    return exe.as_ref().and_then(|e| e.parent()).is_some_and(|d| d.join("Foldex-Deinstallation.exe").is_file()) || exe.zip(install_dir()).is_some_and(|(e, d)| e.starts_with(d));
    #[cfg(not(windows))]
    return exe.zip(install_dir()).is_some_and(|(e, d)| e.parent() == Some(d.as_path()));
}

/// Was entfernt wird (Ordner mit Daten)
pub fn items() -> Vec<Item> {
    let c = config();
    let mut v = vec![
        Item { label: "KI-Modelle und KI-Programme", path: c.join("models") },
        Item { label: "Bildeditor-KI", path: c.join("ai") },
        Item { label: "Cloud-Anmeldungen und Zwischenspeicher", path: c.join("cloud") },
        Item { label: "Einstellungen, Themes, Verlauf, KI-Chats", path: c.clone() },
    ];
    if let Some(cache) = dirs::cache_dir().map(|d| d.join("foldex")) {
        v.push(Item { label: "Suchindex und Caches", path: cache });
    }
    v
}

/// Größe eines Ordners (alle Dateien, rekursiv)
pub fn size_of(p: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map_or(0, |m| m.len()),
                _ => {}
            }
        }
    }
    total
}

/// Verknüpfungen und Einträge im System (Startmenü, Desktop, „Apps“-Liste bzw. Anwendungsmenü)
fn shortcuts() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    {
        if let Some(d) = dirs::desktop_dir() {
            v.push(d.join("Foldex.lnk"));
        }
        if let Some(a) = dirs::data_dir() {
            v.push(a.join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("Foldex.lnk"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(d) = dirs::data_dir() {
            v.push(d.join("applications").join("foldex.desktop"));
            v.push(d.join("icons").join("hicolor").join("256x256").join("apps").join("foldex.png"));
        }
    }
    v
}

/// Aufräumen vorbereiten und starten; danach muss Foldex sich sofort beenden.
/// `remove_exe`: auch die Programmdatei löschen (bei installiertem Foldex immer der ganze Programmordner).
pub fn run(remove_exe: bool) -> Result<(), String> {
    // Cloud-Laufwerke trennen (rclone hält sonst Dateien im Zwischenspeicher offen)
    crate::cloudlogin::stop_all();
    let mut dirs: Vec<PathBuf> = items().into_iter().map(|i| i.path).collect();
    let mut files = shortcuts();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if installed() {
        #[cfg(windows)]
        if let Some(d) = exe.parent() {
            dirs.push(d.to_path_buf());
        }
        #[cfg(not(windows))]
        files.push(exe.clone());
    } else if remove_exe {
        files.push(exe.clone());
    }
    // Linux: leere Einhängeordner der Cloud-Laufwerke (~/Cloud/<Name>) – nur wenn leer
    #[cfg(not(windows))]
    let empty_dirs: Vec<PathBuf> = crate::cloudlogin::accounts().iter().map(|a| a.path()).collect();
    #[cfg(windows)]
    let empty_dirs: Vec<PathBuf> = Vec::new();
    let pid = std::process::id();
    launch(&script(pid, &dirs, &files, &empty_dirs, true), pid)
}

/// Aufräum-Skript: wartet auf Prozess `pid`, löscht Ordner und Dateien. `system`: auch weitere Foldex-Fenster
/// schließen und den Eintrag unter „Apps“ entfernen (im Test aus).
fn script(pid: u32, dirs: &[PathBuf], files: &[PathBuf], empty_dirs: &[PathBuf], system: bool) -> String {
    // Windows: PowerShell (cmd-Skripte scheitern an Umlauten in Pfaden, z. B. „Verknüpfung“)
    #[cfg(windows)]
    {
        let _ = empty_dirs;
        let q = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
        let mut s = String::from("$ErrorActionPreference = 'SilentlyContinue'\n");
        // warten, bis dieses Foldex beendet ist (Wait-Process blockiert zuverlässig, auch wenn PowerShell gerade
        // erst Module vorbereitet); weitere Foldex-Fenster schließen (sonst bleiben Dateien offen)
        s += &format!("Wait-Process -Id {pid}\n");
        if system {
            s += "Get-Process foldex | Stop-Process -Force\nStart-Sleep -Seconds 1\n";
        }
        // jedes Ziel bis zu 10-mal versuchen: eine Datei kann kurz noch gesperrt sein
        for d in dirs {
            let p = q(d);
            s += &format!("for ($i = 0; $i -lt 10 -and (Test-Path -LiteralPath {p}); $i++) {{ Remove-Item -LiteralPath {p} -Recurse -Force; if (Test-Path -LiteralPath {p}) {{ Start-Sleep -Milliseconds 400 }} }}\n");
        }
        for f in files {
            let p = q(f);
            s += &format!("for ($i = 0; $i -lt 10 -and (Test-Path -LiteralPath {p}); $i++) {{ Remove-Item -LiteralPath {p} -Force; if (Test-Path -LiteralPath {p}) {{ Start-Sleep -Milliseconds 400 }} }}\n");
        }
        if system {
            // Eintrag in „Apps & Features“ und „Mit Foldex öffnen“ im Explorer (vom Installer)
            for k in [
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Foldex",
                "Software\\Classes\\Directory\\shell\\Foldex",
                "Software\\Classes\\Directory\\Background\\shell\\Foldex",
                "Software\\Classes\\Drive\\shell\\Foldex",
            ] {
                s += &format!("Remove-Item -LiteralPath 'HKCU:\\{k}' -Recurse -Force\n");
            }
        }
        s
    }
    #[cfg(not(windows))]
    {
        let q = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
        let mut s = String::from("#!/bin/sh\n");
        s += &format!("while kill -0 {pid} 2>/dev/null; do sleep 0.5; done\n");
        if system {
            s += "pkill -x foldex 2>/dev/null; sleep 1\n";
        }
        for d in dirs {
            s += &format!("rm -rf {}\n", q(d));
        }
        for f in files {
            s += &format!("rm -f {}\n", q(f));
        }
        for d in empty_dirs {
            s += &format!("rmdir {} 2>/dev/null\n", q(d));
        }
        s += "rm -f \"$0\"\n";
        s
    }
}

/// Skript schreiben und unsichtbar, unabhängig von Foldex starten; Rückgabe erst nach dem Start
fn launch(text: &str, tag: u32) -> Result<(), String> {
    // Windows: als kodierter Befehl (UTF-16, Base64) – keine Skriptdatei, keine Probleme mit Umlauten
    #[cfg(windows)]
    {
        let _ = tag;
        let utf16: Vec<u8> = text.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: unsichtbar; läuft weiter, wenn Foldex endet (DETACHED_PROCESS ließe PowerShell nicht starten)
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand", &base64(&utf16)])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let script = std::env::temp_dir().join(format!("foldex-deinstallieren-{tag}.sh"));
        std::fs::write(&script, text).map_err(|e| e.to_string())?;
        std::process::Command::new("sh").arg(&script).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            s.push(if k <= c.len() { T[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skript an Testordnern: wartet auf einen Prozess, löscht dann Ordner (mit Umlauten) und Dateien, sich selbst auch
    #[test]
    fn cleanup_script() {
        let base = std::env::temp_dir().join(format!("foldex-deinst-test-{}", std::process::id()));
        let d1 = base.join("Daten mit Umlauten äöü").join("models");
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::write(d1.join("modell.gguf"), b"x").unwrap();
        let f1 = base.join("Foldex - Verknüpfung.lnk");
        std::fs::write(&f1, b"x").unwrap();
        let keep = base.join("bleibt.txt");
        std::fs::write(&keep, b"x").unwrap();
        // ein Prozess, der kurz läuft: das Skript muss auf ihn warten
        #[cfg(windows)]
        let mut child = std::process::Command::new("cmd").args(["/c", "ping -n 3 127.0.0.1 >nul"]).spawn().unwrap();
        #[cfg(not(windows))]
        let mut child = std::process::Command::new("sleep").arg("2").spawn().unwrap();
        let pid = child.id();
        let tag = pid + 7_000_000;
        launch(&script(pid, &[base.join("Daten mit Umlauten äöü")], &[f1.clone()], &[], false), tag).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(600));
        assert!(d1.exists(), "darf erst nach dem Prozess löschen");
        child.wait().unwrap();
        let t0 = std::time::Instant::now();
        while (d1.exists() || f1.exists()) && t0.elapsed().as_secs() < 15 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(!d1.exists() && !f1.exists(), "nicht gelöscht");
        assert!(keep.exists(), "zu viel gelöscht");
        let _ = std::fs::remove_dir_all(&base);
        // Linux: Skriptdatei räumt sich selbst weg (Windows braucht keine Datei)
        #[cfg(not(windows))]
        {
            std::thread::sleep(std::time::Duration::from_millis(500));
            assert!(!std::env::temp_dir().join(format!("foldex-deinstallieren-{tag}.sh")).exists(), "Skript nicht entfernt");
        }
    }
}
