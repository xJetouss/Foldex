//! Dateioperationen mit Rückgängig/Wiederholen, sicherem Überschreiben und Komprimieren.
//!
//! Grundsätze:
//! * Nichts wird stillschweigend überschrieben. Bei Namenskonflikten entscheidet der Benutzer
//!   (Ersetzen / Beide behalten / Überspringen).
//! * „Ersetzen“ ist rückgängig machbar: Das alte Element kommt in den Papierkorb, und zwar erst NACHDEM
//!   die neue Datei vollständig unter einem Temp-Namen geschrieben wurde. Schlägt etwas fehl, bleibt das
//!   Original unverändert.
//! * Kopien werden zuerst unter einem Temp-Namen angelegt und dann umbenannt – ein Abbruch hinterlässt
//!   keine halbfertigen Dateien mit dem Zielnamen.
//! * Rückgängig-Operationen löschen nie endgültig, sondern verschieben in den Papierkorb.

use crate::fsops::{copy_rec, remove_any, unique};
use crate::trashx;
use std::{
    fs::{self, File},
    io::{self, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Policy {
    /// Vorhandenes Element in den Papierkorb, neues an dessen Stelle
    Replace,
    /// Neues Element unter freiem Namen anlegen („Name (2)“)
    KeepBoth,
    /// Konfliktelemente auslassen
    Skip,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Zip,
    SevenZ,
    TarGz,
    Tar,
    TarBz2,
    TarXz,
    TarZst,
}

impl Format {
    /// Alle Formate, in die gepackt werden kann (Reihenfolge für Menüs).
    pub const ALL: [Format; 7] = [Format::Zip, Format::SevenZ, Format::TarGz, Format::Tar, Format::TarBz2, Format::TarXz, Format::TarZst];

    pub fn ext(self) -> &'static str {
        match self {
            Format::Zip => ".zip",
            Format::SevenZ => ".7z",
            Format::TarGz => ".tar.gz",
            Format::Tar => ".tar",
            Format::TarBz2 => ".tar.bz2",
            Format::TarXz => ".tar.xz",
            Format::TarZst => ".tar.zst",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP-Datei",
            Format::SevenZ => "7z-Archiv",
            Format::TarGz => "TAR.GZ-Archiv",
            Format::Tar => "TAR-Archiv (unkomprimiert)",
            Format::TarBz2 => "TAR.BZ2-Archiv",
            Format::TarXz => "TAR.XZ-Archiv (klein)",
            Format::TarZst => "TAR.ZST-Archiv (schnell)",
        }
    }
    /// Format zu einem Dateinamen (an der Endung) – für „Konvertieren in …“ bei Archiven.
    pub fn from_name(name: &str) -> Option<Format> {
        let l = name.to_lowercase();
        Some(match () {
            _ if l.ends_with(".tar.gz") || l.ends_with(".tgz") => Format::TarGz,
            _ if l.ends_with(".tar.bz2") || l.ends_with(".tbz2") || l.ends_with(".tbz") => Format::TarBz2,
            _ if l.ends_with(".tar.xz") || l.ends_with(".txz") => Format::TarXz,
            _ if l.ends_with(".tar.zst") || l.ends_with(".tzst") => Format::TarZst,
            _ if l.ends_with(".tar") => Format::Tar,
            _ if l.ends_with(".zip") => Format::Zip,
            _ if l.ends_with(".7z") => Format::SevenZ,
            _ => return None,
        })
    }
}

// ───────────────────────────── Verlauf ─────────────────────────────

#[derive(Clone, Debug)]
pub enum Op {
    Copy {
        pairs: Vec<(PathBuf, PathBuf)>,
        replaced: Vec<PathBuf>,
    },
    Move {
        pairs: Vec<(PathBuf, PathBuf)>,
        replaced: Vec<PathBuf>,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Create {
        path: PathBuf,
        dir: bool,
    },
    Trash {
        paths: Vec<PathBuf>,
    },
    /// in den Papierkorb von Google Drive gelegt (Konto, Kennungen, frühere Pfade)
    CloudTrash {
        remote: String,
        ids: Vec<String>,
        paths: Vec<PathBuf>,
    },
    Extract {
        created: Vec<PathBuf>,
    },
    Compress {
        out: PathBuf,
        srcs: Vec<PathBuf>,
        fmt: Format,
    },
    /// Umbenennen mit Formatwechsel: Original liegt im Papierkorb, `out` ist die konvertierte Datei
    Convert {
        src: PathBuf,
        out: PathBuf,
    },
    /// Inhalt einer Datei geändert (KI-Agent): Kopien vorher/nachher für Rückgängig/Wiederholen
    Edit {
        path: PathBuf,
        before: PathBuf,
        after: PathBuf,
    },
}

impl Op {
    pub fn label(&self) -> String {
        match self {
            Op::Copy { pairs, .. } => format!("Kopieren ({})", pairs.len()),
            Op::Move { pairs, .. } => format!("Verschieben ({})", pairs.len()),
            Op::Rename { .. } => "Umbenennen".into(),
            Op::Create { dir: true, .. } => "Neuer Ordner".into(),
            Op::Create { .. } => "Neue Datei".into(),
            Op::Trash { paths } => format!("Löschen ({})", paths.len()),
            Op::CloudTrash { paths, .. } => format!("Löschen in Google Drive ({})", paths.len()),
            Op::Extract { .. } => "Entpacken".into(),
            Op::Compress { .. } => "Komprimieren".into(),
            Op::Convert { out, .. } => format!("Konvertieren ({})", out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            Op::Edit { path, .. } => format!("Bearbeiten ({})", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        }
    }
}

/// Ergebnis eines Hintergrundauftrags für den Verlauf.
pub enum Hist {
    None,
    /// Neue Aktion → auf den Rückgängig-Stapel, Wiederholen-Stapel leeren
    Record(Op),
    Undone(Op),
    Redone(Op),
    UndoFailed(Op),
    RedoFailed(Op),
}

#[derive(Default)]
pub struct History {
    pub undo: Vec<Op>,
    pub redo: Vec<Op>,
}

const MAX_HIST: usize = 100;

impl History {
    pub fn apply(&mut self, h: Hist) {
        match h {
            Hist::None => {}
            Hist::Record(op) => {
                self.undo.push(op);
                self.redo.clear();
            }
            Hist::Undone(op) => self.redo.push(op),
            Hist::Redone(op) => self.undo.push(op),
            // Eine Aktion, die sich nicht mehr rückgängig machen/wiederholen lässt (z. B. weil die Dateien
            // inzwischen fehlen), wird verworfen – sonst bliebe der Verlauf an ihr hängen.
            Hist::UndoFailed(_) | Hist::RedoFailed(_) => {}
        }
        if self.undo.len() > MAX_HIST {
            self.undo.remove(0);
        }
        if self.redo.len() > MAX_HIST {
            self.redo.remove(0);
        }
    }
}

// ───────────────────────────── Hilfsfunktionen ─────────────────────────────

pub fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

/// `name (2).ext` – `ext` darf mehrteilig sein (z. B. „.tar.gz“).
pub fn unique_with_ext(dir: &Path, base: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{base}{ext}"));
    if !exists(&first) {
        return first;
    }
    let mut n = 2;
    loop {
        let c = dir.join(format!("{base} ({n}){ext}"));
        if !exists(&c) {
            return c;
        }
        n += 1;
    }
}

fn temp_path(dst: &Path) -> PathBuf {
    let name = dst
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut n = 0u32;
    loop {
        let c = dst.with_file_name(format!(".{name}.fc-tmp{}", if n == 0 { String::new() } else { n.to_string() }));
        if !exists(&c) {
            return c;
        }
        n += 1;
    }
}

/// Legt `src` unter `dst` ab. `dst` darf nur existieren, wenn `replace` gesetzt ist (dann → Papierkorb).
/// Rückgabe: wurde ein vorhandenes Element ersetzt?
fn place(src: &Path, dst: &Path, copy: bool, replace: bool) -> Result<bool, String> {
    let replacing = replace && exists(dst);
    if !exists(dst) && !copy {
        // schneller Weg: Verschieben auf demselben Datenträger
        if fs::rename(src, dst).is_ok() {
            return Ok(false);
        }
    }
    let tmp = temp_path(dst);
    if let Err(e) = copy_rec(src, &tmp) {
        let _ = remove_any(&tmp);
        return Err(e.to_string());
    }
    if replacing {
        if let Err(e) = trashx::delete(&[dst.to_path_buf()]) {
            let _ = remove_any(&tmp);
            return Err(format!(
                "Vorhandenes Element konnte nicht in den Papierkorb verschoben werden: {e}"
            ));
        }
    } else if exists(dst) {
        let _ = remove_any(&tmp);
        return Err("Ziel existiert bereits".into());
    }
    if let Err(e) = fs::rename(&tmp, dst) {
        let _ = remove_any(&tmp);
        return Err(e.to_string());
    }
    if !copy {
        remove_any(src).map_err(|e| format!("kopiert, aber das Original ließ sich nicht entfernen: {e}"))?;
    }
    Ok(replacing)
}

// ───────────────────────────── Kopieren / Verschieben ─────────────────────────────

pub struct Conflict {
    pub src: PathBuf,
    pub dst: PathBuf,
}

/// Welche Elemente würden im Zielordner ein vorhandenes Element überschreiben?
pub fn find_conflicts(srcs: &[PathBuf], dest: &Path, _copy: bool) -> Vec<Conflict> {
    let mut v = Vec::new();
    for s in srcs {
        let Some(name) = s.file_name() else { continue };
        let dst = dest.join(name);
        if dst == *s {
            continue; // Kopie im selben Ordner → „Name (2)“, Verschieben → nichts zu tun
        }
        if exists(&dst) {
            v.push(Conflict {
                src: s.clone(),
                dst,
            });
        }
    }
    v
}

#[derive(Default)]
pub struct Transfer {
    pub pairs: Vec<(PathBuf, PathBuf)>,
    pub replaced: Vec<PathBuf>,
    pub skipped: usize,
    pub errors: Vec<String>,
}

pub fn transfer(srcs: &[PathBuf], dest: &Path, copy: bool, policy: Policy, prog: &AtomicU64, cancel: &AtomicBool) -> Transfer {
    let mut t = Transfer::default();
    for s in srcs {
        if cancel.load(Relaxed) {
            break;
        }
        let Some(fname) = s.file_name() else { continue };
        let label = fname.to_string_lossy().into_owned();
        if !exists(s) {
            t.errors.push(format!("{label}: Quelle existiert nicht mehr"));
            continue;
        }
        if s.is_dir() && dest.starts_with(s) {
            t.errors.push(format!("{label}: Ordner kann nicht in sich selbst kopiert/verschoben werden"));
            continue;
        }
        let mut dst = dest.join(fname);
        if dst == *s {
            if copy {
                dst = unique(dst);
            } else {
                t.skipped += 1;
                continue;
            }
        }
        let mut replace = false;
        if exists(&dst) {
            match policy {
                Policy::Skip => {
                    t.skipped += 1;
                    continue;
                }
                Policy::KeepBoth => dst = unique(dst),
                Policy::Replace => replace = true,
            }
        }
        match place(s, &dst, copy, replace) {
            Ok(was_replaced) => {
                if was_replaced {
                    t.replaced.push(dst.clone());
                }
                t.pairs.push((s.clone(), dst));
                prog.fetch_add(1, Relaxed);
            }
            Err(e) => t.errors.push(format!("{label}: {e}")),
        }
    }
    t
}

/// Endgültiges Löschen (nur nach ausdrücklicher Bestätigung). Rückgabe: (gelöscht, Fehler).
pub fn delete_permanent(paths: &[PathBuf]) -> (usize, usize) {
    let (mut ok, mut fail) = (0, 0);
    for p in paths {
        if remove_any(p).is_ok() {
            ok += 1;
        } else {
            fail += 1;
        }
    }
    (ok, fail)
}

// ───────────────────────────── Namen ─────────────────────────────

/// Prüft einen Datei-/Ordnernamen. `Err` enthält die Begründung.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("Ungültiger Name".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| matches!(c, '/' | '\\' | '\0') || (cfg!(windows) && matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')) || c.is_control())
    {
        return Err(format!("Der Name darf das Zeichen „{}“ nicht enthalten", if c.is_control() { ' ' } else { c }));
    }
    if cfg!(windows) {
        if name.ends_with('.') || name.ends_with(' ') {
            return Err("Der Name darf nicht mit Punkt oder Leerzeichen enden".into());
        }
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = ["CON", "PRN", "AUX", "NUL"];
        let com_lpt = (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit();
        if reserved.contains(&stem.as_str()) || com_lpt {
            return Err("Dieser Name ist vom System reserviert".into());
        }
    }
    Ok(())
}

/// Benennt um und überschreibt dabei NIE ein vorhandenes Element.
pub fn rename_safe(from: &Path, to: &Path) -> Result<(), String> {
    if exists(to) {
        // reine Groß-/Kleinschreibungsänderung auf Dateisystemen ohne Unterscheidung ist erlaubt
        let same_file = from.to_string_lossy().to_lowercase() == to.to_string_lossy().to_lowercase();
        if !same_file {
            return Err("Ein Element mit diesem Namen existiert bereits".into());
        }
    }
    fs::rename(from, to).map_err(|e| e.to_string())
}

// ───────────────────────────── Rückgängig / Wiederholen ─────────────────────────────

fn trash_existing(paths: impl IntoIterator<Item = PathBuf>) -> Result<usize, String> {
    let v: Vec<PathBuf> = paths.into_iter().filter(|p| exists(p)).collect();
    trashx::delete(&v)?;
    Ok(v.len())
}

fn restore_replaced(replaced: &[PathBuf]) -> Result<(), String> {
    if replaced.is_empty() {
        return Ok(());
    }
    trashx::restore_paths(replaced)
        .map(|_| ())
        .map_err(|e| format!("Ersetzte Elemente konnten nicht aus dem Papierkorb zurückgeholt werden: {e}"))
}

pub fn undo(op: &Op) -> Result<String, String> {
    match op {
        Op::Copy { pairs, replaced } => {
            let n = trash_existing(pairs.iter().map(|(_, d)| d.clone()))?;
            restore_replaced(replaced)?;
            Ok(format!("Kopieren rückgängig gemacht ({n} Element(e) im Papierkorb)"))
        }
        Op::Move { pairs, replaced } => {
            let mut errs = Vec::new();
            for (src, dst) in pairs.iter().rev() {
                if !exists(dst) {
                    errs.push(format!("{} fehlt", dst.display()));
                    continue;
                }
                if let Some(par) = src.parent() {
                    let _ = fs::create_dir_all(par);
                }
                let target = if exists(src) { unique(src.clone()) } else { src.clone() };
                if let Err(e) = place(dst, &target, false, false) {
                    errs.push(e);
                }
            }
            if !errs.is_empty() {
                return Err(errs.join("; "));
            }
            restore_replaced(replaced)?;
            Ok("Verschieben rückgängig gemacht".into())
        }
        Op::Rename { from, to } => {
            if !exists(to) {
                return Err("Das umbenannte Element existiert nicht mehr".into());
            }
            if exists(from) && from.to_string_lossy().to_lowercase() != to.to_string_lossy().to_lowercase() {
                return Err("Der ursprüngliche Name ist inzwischen belegt".into());
            }
            fs::rename(to, from).map_err(|e| e.to_string())?;
            Ok("Umbenennen rückgängig gemacht".into())
        }
        Op::Create { path, .. } => {
            trash_existing([path.clone()])?;
            Ok("Erstellen rückgängig gemacht".into())
        }
        Op::Trash { paths } => {
            let n = trashx::restore_paths(paths)?;
            Ok(format!("{n} Element(e) aus dem Papierkorb wiederhergestellt"))
        }
        Op::CloudTrash { remote, ids, paths } => {
            let n = crate::cloudtrash::restore(remote, ids)?;
            for p in paths {
                crate::cloudlogin::forget_parent(p);
            }
            Ok(format!("{n} Element(e) aus dem Papierkorb von Google Drive zurückgeholt"))
        }
        Op::Extract { created } => {
            let n = trash_existing(created.iter().cloned())?;
            Ok(format!("Entpacken rückgängig gemacht ({n} Element(e) im Papierkorb)"))
        }
        Op::Compress { out, .. } => {
            trash_existing([out.clone()])?;
            Ok("Komprimieren rückgängig gemacht (Archiv im Papierkorb)".into())
        }
        Op::Edit { path, before, .. } => {
            fs::copy(before, path).map_err(|e| format!("Ursprünglicher Inhalt nicht wiederherstellbar: {e}"))?;
            Ok("Bearbeiten rückgängig gemacht".into())
        }
        Op::Convert { src, out } => {
            trash_existing([out.clone()])?;
            trashx::restore_paths(std::slice::from_ref(src))?;
            Ok("Konvertieren rückgängig gemacht (Original wiederhergestellt)".into())
        }
    }
}

/// Sicherungskopie einer Datei für Op::Edit (im Cache-Ordner, nie im Projekt).
pub fn snapshot(path: &Path) -> Result<PathBuf, String> {
    let dir = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("edits");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let snap = dir.join(format!("{n}-{name}"));
    fs::copy(path, &snap).map_err(|e| format!("Sicherungskopie fehlgeschlagen: {e}"))?;
    Ok(snap)
}

pub fn redo(op: &Op) -> Result<String, String> {
    match op {
        Op::Copy { pairs, .. } | Op::Move { pairs, .. } => {
            let copy = matches!(op, Op::Copy { .. });
            let mut errs = Vec::new();
            for (src, dst) in pairs {
                if !exists(src) {
                    errs.push(format!("{} existiert nicht mehr", src.display()));
                    continue;
                }
                if let Some(par) = dst.parent() {
                    let _ = fs::create_dir_all(par);
                }
                if let Err(e) = place(src, dst, copy, true) {
                    errs.push(e);
                }
            }
            if errs.is_empty() {
                Ok(format!("{} wiederholt", if copy { "Kopieren" } else { "Verschieben" }))
            } else {
                Err(errs.join("; "))
            }
        }
        Op::Rename { from, to } => {
            rename_safe(from, to)?;
            Ok("Umbenennen wiederholt".into())
        }
        Op::Create { path, dir } => {
            if !exists(path) {
                if *dir {
                    fs::create_dir(path)
                } else {
                    File::create(path).map(|_| ())
                }
                .map_err(|e| e.to_string())?;
            }
            Ok("Erstellen wiederholt".into())
        }
        Op::Trash { paths } => {
            let n = trash_existing(paths.iter().cloned())?;
            Ok(format!("{n} Element(e) erneut in den Papierkorb verschoben"))
        }
        Op::CloudTrash { remote, ids, paths } => {
            let n = crate::cloudtrash::trash(remote, ids)?;
            for p in paths {
                crate::cloudlogin::forget_parent(p);
            }
            Ok(format!("{n} Element(e) erneut in den Papierkorb von Google Drive gelegt"))
        }
        Op::Extract { created } => {
            trashx::restore_paths(created)?;
            Ok("Entpacken wiederholt".into())
        }
        Op::Compress { out, srcs, fmt } => {
            if exists(out) {
                return Err("Das Archiv existiert bereits".into());
            }
            compress(srcs, out, *fmt, &AtomicU64::new(0), &AtomicBool::new(false))?;
            Ok("Komprimieren wiederholt".into())
        }
        Op::Edit { path, after, .. } => {
            fs::copy(after, path).map_err(|e| e.to_string())?;
            Ok("Bearbeiten wiederholt".into())
        }
        Op::Convert { src, out } => {
            trash_existing([src.clone()])?;
            trashx::restore_paths(std::slice::from_ref(out))?;
            Ok("Konvertieren wiederholt".into())
        }
    }
}

// ───────────────────────────── Komprimieren ─────────────────────────────

/// Vorschlag für den Archivnamen: ein Element → dessen Name, mehrere → Name des Elternordners.
pub fn archive_path(srcs: &[PathBuf], fmt: Format) -> Option<PathBuf> {
    let first = srcs.first()?;
    let dir = first.parent()?.to_path_buf();
    let base = if srcs.len() == 1 {
        if first.is_dir() {
            first.file_name()?.to_string_lossy().into_owned()
        } else {
            first
                .file_stem()
                .or_else(|| first.file_name())?
                .to_string_lossy()
                .into_owned()
        }
    } else {
        dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Archiv".into())
    };
    Some(unique_with_ext(&dir, &base, fmt.ext()))
}

type Entry = (PathBuf, String, bool);

fn walk(p: &Path, rel: &str, skip: &[PathBuf], out: &mut Vec<Entry>, bad: &mut usize) {
    if skip.iter().any(|s| s == p) {
        return;
    }
    let Ok(md) = fs::symlink_metadata(p) else {
        *bad += 1;
        return;
    };
    if md.file_type().is_symlink() {
        return; // Verknüpfungen werden nicht mitgepackt (verhindert Endlosschleifen)
    }
    if md.is_dir() {
        out.push((p.to_path_buf(), rel.to_string(), true));
        match fs::read_dir(p) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    walk(&e.path(), &format!("{rel}/{n}"), skip, out, bad);
                }
            }
            Err(_) => *bad += 1,
        }
    } else {
        out.push((p.to_path_buf(), rel.to_string(), false));
    }
}

fn already_compressed(rel: &str) -> bool {
    let ext = rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "zip" | "7z" | "rar" | "gz" | "bz2" | "xz" | "zst" | "tgz" | "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "avif" | "mp3" | "m4a" | "aac" | "ogg" | "opus" | "flac" | "mp4" | "mkv" | "mov" | "webm" | "avi" | "docx" | "xlsx" | "pptx" | "jar" | "apk" | "woff2"
    )
}

fn zip_time(t: std::time::SystemTime) -> Option<zip::DateTime> {
    use chrono::{Datelike, Timelike};
    let d: chrono::DateTime<chrono::Local> = t.into();
    zip::DateTime::from_date_and_time(
        u16::try_from(d.year()).ok()?,
        d.month() as u8,
        d.day() as u8,
        d.hour() as u8,
        d.minute() as u8,
        d.second().min(59) as u8,
    )
    .ok()
}

/// Packt `srcs` nach `out` (Format `fmt`). Es wird zuerst in `<out>.part` geschrieben; ein vorhandenes `out`
/// wird nie angetastet. Rückgabe: Anzahl nicht lesbarer Elemente, die übersprungen wurden.
pub fn compress(srcs: &[PathBuf], out: &Path, fmt: Format, prog: &AtomicU64, cancel: &AtomicBool) -> Result<usize, String> {
    if exists(out) {
        return Err("Das Archiv existiert bereits".into());
    }
    let part = PathBuf::from(format!("{}.part", out.display()));
    let _ = remove_any(&part);
    let skip = [part.clone(), out.to_path_buf()];
    let mut list: Vec<Entry> = Vec::new();
    let mut bad = 0usize;
    for s in srcs {
        let Some(n) = s.file_name() else { continue };
        walk(s, &n.to_string_lossy(), &skip, &mut list, &mut bad);
    }
    if list.is_empty() {
        return Err("Nichts zum Komprimieren".into());
    }
    let res = match fmt {
        Format::Zip => write_zip(&list, &part, prog, cancel, &mut bad),
        Format::SevenZ => write_7z(&list, &part, prog, cancel, &mut bad),
        tar_fmt => write_tar(&list, &part, tar_fmt, prog, cancel, &mut bad),
    };
    match res {
        Ok(()) => {
            if exists(out) {
                let _ = remove_any(&part);
                return Err("Das Archiv existiert bereits".into());
            }
            fs::rename(&part, out).map_err(|e| {
                let _ = remove_any(&part);
                e.to_string()
            })?;
            Ok(bad)
        }
        Err(e) => {
            let _ = remove_any(&part);
            Err(e)
        }
    }
}

fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn write_zip(list: &[Entry], part: &Path, prog: &AtomicU64, cancel: &AtomicBool, bad: &mut usize) -> Result<(), String> {
    use zip::{write::SimpleFileOptions, CompressionMethod};
    let mut zw = zip::ZipWriter::new(BufWriter::with_capacity(1 << 18, File::create(part).map_err(e2s)?));
    for (p, rel, is_dir) in list {
        if cancel.load(Relaxed) {
            return Err("abgebrochen".into());
        }
        let Ok(md) = fs::metadata(p) else {
            *bad += 1;
            continue;
        };
        let method = if already_compressed(rel) { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        let mut opts = SimpleFileOptions::default().compression_method(method).large_file(md.len() > 0xFFFF_FFFF);
        if let Some(t) = md.modified().ok().and_then(zip_time) {
            opts = opts.last_modified_time(t);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            opts = opts.unix_permissions(md.permissions().mode() & 0o777);
        }
        if *is_dir {
            zw.add_directory(format!("{rel}/"), opts).map_err(e2s)?;
        } else {
            let Ok(f) = File::open(p) else {
                *bad += 1;
                continue;
            };
            zw.start_file(rel.as_str(), opts).map_err(e2s)?;
            io::copy(&mut BufReader::with_capacity(1 << 18, f), &mut zw).map_err(e2s)?;
            prog.fetch_add(1, Relaxed);
        }
    }
    zw.finish().map_err(e2s)?;
    Ok(())
}

/// TAR, wahlweise mit gzip, bzip2, xz oder zstd komprimiert.
fn write_tar(list: &[Entry], part: &Path, fmt: Format, prog: &AtomicU64, cancel: &AtomicBool, bad: &mut usize) -> Result<(), String> {
    let file = BufWriter::with_capacity(1 << 18, File::create(part).map_err(e2s)?);
    let w: Box<dyn Write> = match fmt {
        Format::TarGz => Box::new(flate2::write::GzEncoder::new(file, flate2::Compression::fast())),
        Format::TarBz2 => Box::new(bzip2::write::BzEncoder::new(file, bzip2::Compression::default())),
        Format::TarXz => Box::new(xz2::write::XzEncoder::new(file, 6)),
        Format::TarZst => Box::new(zstd::stream::write::Encoder::new(file, 3).map_err(e2s)?.auto_finish()),
        _ => Box::new(file),
    };
    let mut tb = tar::Builder::new(w);
    tb.follow_symlinks(false);
    for (p, rel, is_dir) in list {
        if cancel.load(Relaxed) {
            return Err("abgebrochen".into());
        }
        let r = if *is_dir { tb.append_dir(rel, p) } else { tb.append_path_with_name(p, rel) };
        match r {
            Ok(_) => {
                if !*is_dir {
                    prog.fetch_add(1, Relaxed);
                }
            }
            Err(_) => *bad += 1,
        }
    }
    // Encoder schließen (schreibt das Ende des komprimierten Stroms) und alles auf die Platte bringen
    let mut w = tb.into_inner().map_err(e2s)?;
    w.flush().map_err(e2s)?;
    drop(w);
    Ok(())
}

fn write_7z(list: &[Entry], part: &Path, prog: &AtomicU64, cancel: &AtomicBool, bad: &mut usize) -> Result<(), String> {
    let mut sz = sevenz_rust::SevenZWriter::create(part).map_err(e2s)?;
    for (p, rel, is_dir) in list {
        if cancel.load(Relaxed) {
            return Err("abgebrochen".into());
        }
        let entry = sevenz_rust::SevenZArchiveEntry::from_path(p, rel.clone());
        if *is_dir {
            sz.push_archive_entry::<&[u8]>(entry, None).map_err(e2s)?;
        } else {
            let Ok(f) = File::open(p) else {
                *bad += 1;
                continue;
            };
            sz.push_archive_entry(entry, Some(f)).map_err(e2s)?;
            prog.fetch_add(1, Relaxed);
        }
    }
    sz.finish().map_err(e2s)?;
    Ok(())
}

// ───────────────────────────── Tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fc-ops-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn p() -> (AtomicU64, AtomicBool) {
        (AtomicU64::new(0), AtomicBool::new(false))
    }

    #[test]
    fn keep_both_never_overwrites() {
        let d = tmp("keep");
        fs::create_dir_all(d.join("a")).unwrap();
        fs::create_dir_all(d.join("b")).unwrap();
        fs::write(d.join("a/x.txt"), "neu").unwrap();
        fs::write(d.join("b/x.txt"), "alt").unwrap();
        let srcs = vec![d.join("a/x.txt")];
        assert_eq!(find_conflicts(&srcs, &d.join("b"), true).len(), 1);
        let (pr, c) = p();
        let t = transfer(&srcs, &d.join("b"), true, Policy::KeepBoth, &pr, &c);
        assert!(t.errors.is_empty(), "{:?}", t.errors);
        assert_eq!(fs::read_to_string(d.join("b/x.txt")).unwrap(), "alt");
        assert_eq!(fs::read_to_string(d.join("b/x (2).txt")).unwrap(), "neu");
        // Rückgängig: Kopie weg (Papierkorb), Original unberührt
        let op = Op::Copy { pairs: t.pairs, replaced: t.replaced };
        let _ = undo(&op);
        assert_eq!(fs::read_to_string(d.join("b/x.txt")).unwrap(), "alt");
        assert_eq!(fs::read_to_string(d.join("a/x.txt")).unwrap(), "neu");
    }

    #[test]
    fn skip_policy_and_self_move() {
        let d = tmp("skip");
        fs::create_dir_all(d.join("b")).unwrap();
        fs::write(d.join("x.txt"), "1").unwrap();
        fs::write(d.join("b/x.txt"), "2").unwrap();
        let (pr, c) = p();
        let t = transfer(&[d.join("x.txt")], &d.join("b"), false, Policy::Skip, &pr, &c);
        assert_eq!((t.pairs.len(), t.skipped), (0, 1));
        assert_eq!(fs::read_to_string(d.join("b/x.txt")).unwrap(), "2");
        // in denselben Ordner verschieben = nichts
        let t = transfer(&[d.join("x.txt")], &d, false, Policy::KeepBoth, &pr, &c);
        assert_eq!((t.pairs.len(), t.skipped), (0, 1));
        // Ordner nicht in sich selbst
        let t = transfer(&[d.join("b")], &d.join("b"), true, Policy::KeepBoth, &pr, &c);
        assert_eq!(t.errors.len(), 1);
    }

    #[test]
    fn move_and_undo_redo() {
        let d = tmp("move");
        fs::create_dir_all(d.join("dst")).unwrap();
        fs::write(d.join("f.txt"), "x").unwrap();
        let (pr, c) = p();
        let t = transfer(&[d.join("f.txt")], &d.join("dst"), false, Policy::KeepBoth, &pr, &c);
        let op = Op::Move { pairs: t.pairs, replaced: t.replaced };
        assert!(d.join("dst/f.txt").exists() && !d.join("f.txt").exists());
        undo(&op).unwrap();
        assert!(d.join("f.txt").exists() && !d.join("dst/f.txt").exists());
        redo(&op).unwrap();
        assert!(d.join("dst/f.txt").exists() && !d.join("f.txt").exists());
    }

    #[test]
    fn rename_and_names() {
        let d = tmp("ren");
        fs::write(d.join("a"), "1").unwrap();
        fs::write(d.join("b"), "2").unwrap();
        assert!(rename_safe(&d.join("a"), &d.join("b")).is_err());
        assert_eq!(fs::read_to_string(d.join("b")).unwrap(), "2");
        assert!(rename_safe(&d.join("a"), &d.join("c")).is_ok());
        let op = Op::Rename { from: d.join("a"), to: d.join("c") };
        undo(&op).unwrap();
        assert!(d.join("a").exists());
        redo(&op).unwrap();
        assert!(d.join("c").exists());
        assert!(check_name("a/b").is_err() && check_name("").is_err() && check_name("ok.txt").is_ok());
    }

    #[test]
    fn compress_all_formats_roundtrip() {
        let d = tmp("zip");
        fs::create_dir_all(d.join("src/sub")).unwrap();
        fs::write(d.join("src/a.txt"), "hallo".repeat(500)).unwrap();
        fs::write(d.join("src/sub/b.bin"), vec![3u8; 50_000]).unwrap();
        for fmt in Format::ALL {
            let out = archive_path(&[d.join("src")], fmt).unwrap();
            assert!(out.to_string_lossy().ends_with(fmt.ext()));
            let (pr, c) = p();
            compress(&[d.join("src")], &out, fmt, &pr, &c).unwrap();
            assert!(out.exists() && !PathBuf::from(format!("{}.part", out.display())).exists());
            let dest = d.join(format!("out{}", fmt.ext()));
            crate::extract::extract(&out, &dest, &AtomicU64::new(0), &AtomicBool::new(false)).unwrap();
            assert_eq!(fs::read(dest.join("src/sub/b.bin")).unwrap().len(), 50_000, "{fmt:?}");
            assert_eq!(fs::read_to_string(dest.join("src/a.txt")).unwrap(), "hallo".repeat(500), "{fmt:?}");
            // zweites Komprimieren überschreibt nie
            let out2 = archive_path(&[d.join("src")], fmt).unwrap();
            assert_ne!(out, out2);
        }
    }
}
