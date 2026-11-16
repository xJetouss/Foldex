//! Verknüpfungen zu ausführbaren Dateien anlegen – wie „Verknüpfung erstellen“ im Explorer.
//! Windows: echte .lnk-Datei (Shell-Link mit Programmsymbol und Arbeitsordner). Linux: .desktop-Starter.

use std::path::{Path, PathBuf};

/// Lässt sich die Datei starten (Programm, Skript, Installer)?
pub fn is_executable(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    #[cfg(windows)]
    {
        matches!(ext.as_str(), "exe" | "com" | "bat" | "cmd" | "msi" | "ps1" | "vbs" | "jar" | "scr" | "appref-ms")
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        if matches!(ext.as_str(), "appimage" | "sh" | "run" | "jar") {
            return true;
        }
        // Linux: ausführbare Datei (x-Recht), keine Ordner
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
}

/// freier Name im Zielordner: „Name - Verknüpfung.lnk“, sonst „… (2)“ usw.
fn free_name(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Verknüpfung zu `target` in `dir` anlegen; Rückgabe: Pfad der neuen Verknüpfung
pub fn create(target: &Path, dir: &Path) -> Result<PathBuf, String> {
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).ok_or("ungültiger Dateiname")?;
    #[cfg(windows)]
    {
        let lnk = free_name(dir, &format!("{stem} - Verknüpfung"), ".lnk");
        create_lnk(target, &lnk)?;
        Ok(lnk)
    }
    #[cfg(not(windows))]
    {
        let file = free_name(dir, &stem, ".desktop");
        let work = target.parent().map(|p| p.display().to_string()).unwrap_or_default();
        let ext = target.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        // Skripte im Terminal, damit Ausgaben sichtbar bleiben; Java-Programme über java -jar
        let terminal = matches!(ext.as_str(), "sh" | "run");
        let exec = if ext == "jar" { format!("java -jar \"{}\"", target.display()) } else { format!("\"{}\"", target.display()) };
        let text = format!(
            "[Desktop Entry]\nType=Application\nName={stem}\nExec={exec}\nPath={work}\nIcon=application-x-executable\nTerminal={terminal}\n"
        );
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755));
        // GNOME startet Starter auf dem Desktop erst, wenn sie als vertrauenswürdig markiert sind
        let _ = std::process::Command::new("gio").args(["set"]).arg(&file).args(["metadata::trusted", "true"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        Ok(file)
    }
}

/// Windows-Shell-Link (.lnk) schreiben – mit Programmsymbol und dem Programmordner als Arbeitsordner
#[cfg(windows)]
fn create_lnk(target: &Path, lnk: &Path) -> Result<(), String> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        link.SetPath(&HSTRING::from(target.as_os_str())).map_err(|e| e.to_string())?;
        if let Some(dir) = target.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str())).map_err(|e| e.to_string())?;
        }
        let ext = target.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if matches!(ext.as_str(), "exe" | "com" | "scr") {
            link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0).map_err(|e| e.to_string())?;
        }
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        file.Save(&HSTRING::from(lnk.as_os_str()), true).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_to_program() {
        let dir = std::env::temp_dir().join(format!("foldex-lnk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // ein echtes Programm als Ziel
        let target = std::env::current_exe().unwrap();
        assert!(is_executable(&target));
        let a = create(&target, &dir).expect("Verknüpfung");
        let b = create(&target, &dir).expect("zweite Verknüpfung");
        assert!(a.is_file() && b.is_file() && a != b, "{a:?} {b:?}");
        #[cfg(windows)]
        {
            assert!(a.to_string_lossy().ends_with(" - Verknüpfung.lnk"));
            assert!(b.to_string_lossy().ends_with(" - Verknüpfung (2).lnk"));
            // .lnk beginnt mit Kopfgröße 0x4C und der Shell-Link-Kennung
            let bytes = std::fs::read(&a).unwrap();
            assert_eq!(&bytes[..4], &[0x4C, 0, 0, 0]);
        }
        assert!(!is_executable(&dir.join("bild.png")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod manual {
    /// Verknüpfung für Prüfungen von außen anlegen: FOLDEX_LNK_TARGET=<Datei> FOLDEX_LNK_DIR=<Ordner>
    #[test]
    #[ignore]
    fn make_lnk() {
        let t = std::path::PathBuf::from(std::env::var("FOLDEX_LNK_TARGET").unwrap());
        let d = std::path::PathBuf::from(std::env::var("FOLDEX_LNK_DIR").unwrap());
        let p = super::create(&t, &d).unwrap();
        println!("LNK={}", p.display());
    }
}
