//! Papierkorb: Löschen, Auflisten, Wiederherstellen und endgültig Löschen.
//! Auflisten/Wiederherstellen gibt es nur unter Windows und Linux (die `trash`-Bibliothek kann das unter macOS nicht).

use std::path::PathBuf;

/// Kann dieses System den Papierkorb auflisten und daraus wiederherstellen?
pub const CAN_LIST: bool = cfg!(any(windows, all(unix, not(target_os = "macos"))));

/// Verschiebt alles in den Papierkorb (niemals endgültig löschen).
/// Ausnahme: Cloud-Laufwerke (rclone) haben keinen Windows-Papierkorb – dort wird direkt gelöscht, der Anbieter
/// (Google Drive, iCloud, OneDrive, Dropbox …) legt die Dateien dann in seinen eigenen Papierkorb.
pub fn delete(paths: &[PathBuf]) -> Result<(), String> {
    delete_ex(paths).0
}

/// In den Google-Drive-Papierkorb gelegt: (Konto, Kennungen, Pfade) – für „Rückgängig“
pub struct CloudTrashed {
    pub remote: String,
    pub ids: Vec<String>,
    pub paths: Vec<PathBuf>,
}

/// wie `delete`, meldet zusätzlich, was in Cloud-Papierkörben liegt (zum Zurückholen)
pub fn delete_ex(paths: &[PathBuf]) -> (Result<(), String>, Vec<CloudTrashed>) {
    let (cloud, local): (Vec<PathBuf>, Vec<PathBuf>) = paths.iter().cloned().partition(|p| on_cloud(p));
    let mut err = None;
    let mut done: Vec<CloudTrashed> = Vec::new();
    for p in &cloud {
        match crate::cloudlogin::cloud_delete(p) {
            Ok(Some((remote, id))) => match done.iter_mut().find(|d| d.remote == remote) {
                Some(d) => {
                    d.ids.push(id);
                    d.paths.push(p.clone());
                }
                None => done.push(CloudTrashed { remote, ids: vec![id], paths: vec![p.clone()] }),
            },
            Ok(None) => {}
            Err(e) => err = Some(format!("{}: {e}", p.display())),
        }
    }
    if !local.is_empty() {
        if let Err(e) = trash::delete_all(&local) {
            return (Err(e.to_string()), done);
        }
    }
    (err.map_or(Ok(()), Err), done)
}

/// liegt der Pfad auf einem in Foldex angemeldeten Cloud-Laufwerk?
pub fn on_cloud(p: &std::path::Path) -> bool {
    // Linux: Cloud-Laufwerke sind Ordner (~/Cloud/<Name>)
    #[cfg(not(windows))]
    return crate::cloudlogin::accounts().iter().any(|a| p.starts_with(a.path()));
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        let mut c = s.chars();
        match (c.next(), c.next()) {
            (Some(l), Some(':')) => crate::cloudlogin::letters().contains(&l.to_ascii_uppercase()),
            _ => false,
        }
    }
}

#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
mod imp {
    use std::path::PathBuf;
    use trash::os_limited as os;

    pub struct Row {
        pub name: String,
        pub parent: PathBuf,
        /// Sekunden seit 1970
        pub deleted: i64,
        pub size: Option<u64>,
        pub dir: bool,
        item: trash::TrashItem,
    }

    fn why(e: &trash::Error) -> String {
        match e {
            trash::Error::RestoreCollision { .. } => {
                "am ursprünglichen Ort existiert bereits ein Element mit diesem Namen".into()
            }
            e => e.to_string(),
        }
    }

    pub fn list() -> Result<Vec<Row>, String> {
        let items = os::list().map_err(|e| e.to_string())?;
        let mut rows: Vec<Row> = items
            .into_iter()
            .map(|item| {
                let (size, dir) = match os::metadata(&item) {
                    Ok(m) => match m.size {
                        trash::TrashItemSize::Bytes(b) => (Some(b), false),
                        trash::TrashItemSize::Entries(_) => (None, true),
                    },
                    Err(_) => (None, false),
                };
                Row {
                    name: item.name.to_string_lossy().into_owned(),
                    parent: item.original_parent.clone(),
                    deleted: item.time_deleted,
                    size,
                    dir,
                    item,
                }
            })
            .collect();
        rows.sort_by(|a, b| b.deleted.cmp(&a.deleted).then_with(|| a.name.cmp(&b.name)));
        Ok(rows)
    }

    /// Stellt einzeln wieder her; Rückgabe: (erfolgreich, Fehlertexte).
    pub fn restore(rows: &[&Row]) -> (usize, Vec<String>) {
        let (mut ok, mut errs) = (0, Vec::new());
        for r in rows {
            match os::restore_all([r.item.clone()]) {
                Ok(_) => ok += 1,
                Err(e) => errs.push(format!("{}: {}", r.name, why(&e))),
            }
        }
        (ok, errs)
    }

    pub fn purge(rows: &[&Row]) -> Result<(), String> {
        os::purge_all(rows.iter().map(|r| r.item.clone())).map_err(|e| e.to_string())
    }

    /// Stellt die zuletzt gelöschten Einträge mit diesen Originalpfaden wieder her (für „Rückgängig“).
    pub fn restore_paths(paths: &[PathBuf]) -> Result<usize, String> {
        let all = os::list().map_err(|e| e.to_string())?;
        let mut pick = Vec::new();
        for p in paths {
            if let Some(best) = all
                .iter()
                .filter(|i| i.original_path() == *p)
                .max_by_key(|i| i.time_deleted)
            {
                pick.push(best.clone());
            }
        }
        if pick.is_empty() {
            return Err("nicht mehr im Papierkorb gefunden".into());
        }
        let n = pick.len();
        os::restore_all(pick).map_err(|e| why(&e))?;
        Ok(n)
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
mod imp {
    use std::path::PathBuf;

    pub struct Row {
        pub name: String,
        pub parent: PathBuf,
        pub deleted: i64,
        pub size: Option<u64>,
        pub dir: bool,
    }

    const NO: &str = "Wiederherstellen aus dem Papierkorb wird auf diesem System nicht unterstützt";

    pub fn list() -> Result<Vec<Row>, String> {
        Err(NO.into())
    }
    pub fn restore(_: &[&Row]) -> (usize, Vec<String>) {
        (0, vec![NO.into()])
    }
    pub fn purge(_: &[&Row]) -> Result<(), String> {
        Err(NO.into())
    }
    pub fn restore_paths(_: &[PathBuf]) -> Result<usize, String> {
        Err(NO.into())
    }
}

pub use imp::*;

/// Gesamtgröße des Papierkorbs in Bytes (läuft im Hintergrund; nicht lesbare Teile zählen nicht).
pub fn total_size() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    let (bytes, files) = (AtomicU64::new(0), AtomicU64::new(0));
    let mut roots: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    for c in b'A'..=b'Z' {
        let rb = PathBuf::from(format!("{}:\\$Recycle.Bin", c as char));
        if let Ok(rd) = std::fs::read_dir(&rb) {
            roots.extend(rd.flatten().map(|e| e.path()));
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(h) = dirs::home_dir() {
        roots.push(h.join(".Trash"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(d) = dirs::data_dir() {
        roots.push(d.join("Trash").join("files"));
    }
    for r in roots {
        crate::fsops::folder_size(&r, &bytes, &files);
    }
    bytes.load(Relaxed)
}

#[cfg(test)]
mod tests {
    #[test]
    fn space_and_trash_size() {
        let (f, t) = (fs4::available_space("/").unwrap(), fs4::total_space("/").unwrap());
        assert!(t > 0 && f <= t);
        let _ = super::total_size();
    }
}
