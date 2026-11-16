//! Wer hält eine Datei offen? Windows Restart Manager: Programme ermitteln, die Dateien sperren, und sie
//! auf Wunsch schließen (erst freundlich, auf ausdrücklichen Wunsch erzwungen).

use std::path::PathBuf;

/// Sperrendes Programm: (Prozess-ID, Anzeigename)
pub type Locker = (u32, String);

/// Dateien eines Pfads (Ordner: rekursiv, höchstens `max` Dateien) – der Restart Manager prüft nur Dateien.
fn files_of(paths: &[PathBuf], max: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(p) = stack.pop() {
        if out.len() >= max {
            break;
        }
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.is_dir() => {
                if let Ok(rd) = std::fs::read_dir(&p) {
                    stack.extend(rd.flatten().map(|e| e.path()));
                }
            }
            Ok(_) => out.push(p),
            Err(_) => {}
        }
    }
    out
}

#[cfg(windows)]
mod imp {
    use super::Locker;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows_sys::Win32::System::RestartManager::*;

    /// Session mit registrierten Dateien; wird beim Verlassen beendet.
    struct Session(u32);

    impl Session {
        fn new(files: &[PathBuf]) -> Option<Session> {
            let mut h = 0u32;
            let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
            if unsafe { RmStartSession(&mut h, 0, key.as_mut_ptr()) } != 0 {
                return None;
            }
            let s = Session(h);
            let wide: Vec<Vec<u16>> = files.iter().map(|p| p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()).collect();
            let ptrs: Vec<*const u16> = wide.iter().map(|w| w.as_ptr()).collect();
            let r = unsafe { RmRegisterResources(h, ptrs.len() as u32, ptrs.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null()) };
            (r == 0).then_some(s)
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            unsafe { RmEndSession(self.0) };
        }
    }

    pub fn lockers(files: &[PathBuf]) -> Vec<Locker> {
        let Some(s) = Session::new(files) else { return Vec::new() };
        let (mut needed, mut count, mut reasons) = (0u32, 0u32, 0u32);
        unsafe { RmGetList(s.0, &mut needed, &mut count, std::ptr::null_mut(), &mut reasons) };
        if needed == 0 {
            return Vec::new();
        }
        let mut infos: Vec<RM_PROCESS_INFO> = vec![unsafe { std::mem::zeroed() }; needed as usize];
        count = needed;
        if unsafe { RmGetList(s.0, &mut needed, &mut count, infos.as_mut_ptr(), &mut reasons) } != 0 {
            return Vec::new();
        }
        let mut v: Vec<Locker> = infos[..count as usize]
            .iter()
            .map(|i| {
                let n = String::from_utf16_lossy(&i.strAppName);
                (i.Process.dwProcessId, n.trim_end_matches('\0').to_string())
            })
            .collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn shutdown(files: &[PathBuf], force: bool) -> Result<(), String> {
        let s = Session::new(files).ok_or("Restart Manager nicht verfügbar")?;
        let r = unsafe { RmShutdown(s.0, if force { RmForceShutdown as u32 } else { 0 }, None) };
        match r {
            0 => Ok(()),
            5 => Err("Zugriff verweigert – das Programm läuft mit höheren Rechten.".into()),
            _ => Err(if force {
                format!("Programm konnte nicht beendet werden (Fehler {r}).")
            } else {
                "Das Programm hat das Schließen abgelehnt (z. B. wegen ungespeicherter Änderungen).".into()
            }),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Locker;
    use std::path::PathBuf;
    pub fn lockers(_files: &[PathBuf]) -> Vec<Locker> {
        Vec::new()
    }
    pub fn shutdown(_files: &[PathBuf], _force: bool) -> Result<(), String> {
        Err("Nur unter Windows verfügbar".into())
    }
}

/// Programme, die eine der Dateien (bzw. Dateien in den Ordnern) offen halten.
pub fn lockers(paths: &[PathBuf]) -> Vec<Locker> {
    let files = files_of(paths, 2000);
    if files.is_empty() {
        return Vec::new();
    }
    imp::lockers(&files)
}

/// Sperrende Programme schließen: freundlich (Programm darf nachfragen) oder erzwungen.
pub fn close(paths: &[PathBuf], force: bool) -> Result<(), String> {
    imp::shutdown(&files_of(paths, 2000), force)
}


#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Write;

    /// Eine von diesem Prozess geöffnete Datei wird als gesperrt erkannt.
    #[test]
    fn detects_own_lock() {
        let p = std::env::temp_dir().join(format!("fx-lock-{}.txt", std::process::id()));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"x").unwrap();
        let l = lockers(&[p.clone()]);
        assert!(l.iter().any(|(pid, _)| *pid == std::process::id()), "{l:?}");
        drop(f);
        assert!(lockers(&[p.clone()]).is_empty());
        let _ = std::fs::remove_file(p);
    }
}
