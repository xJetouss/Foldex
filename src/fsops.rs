use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const NO_TIME: i64 = i64::MIN;

/// Änderungszeit in Nanosekunden seit 1970 (i64 reicht bis ins Jahr 2262). Sekundengenau würde
/// Dateien, die in derselben Sekunde geändert wurden, beim Sortieren durcheinanderwürfeln.
pub fn mtime_ns(t: Option<SystemTime>) -> i64 {
    match t.map(|t| t.duration_since(UNIX_EPOCH)) {
        Some(Ok(d)) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Some(Err(e)) => -i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX),
        None => NO_TIME,
    }
}

pub fn fmt_time(ns: i64) -> String {
    if ns == NO_TIME {
        return String::new();
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp(ns.div_euclid(1_000_000_000), 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Größenspalte wie im Windows-Explorer: immer in KB, aufgerundet.
pub fn size_col(n: u64) -> String {
    format!("{} KB", thousands(n.div_ceil(1024).max(1)))
}

pub fn human(n: u64) -> String {
    const U: [&str; 5] = ["Bytes", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < 4 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} Bytes")
    } else {
        format!("{:.1} {}", v, U[u]).replace('.', ",")
    }
}

pub fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(p) if p > 0 && p + 1 < name.len() => name[p + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

pub fn unique(dst: PathBuf) -> PathBuf {
    if fs::symlink_metadata(&dst).is_err() {
        return dst;
    }
    let stem = dst
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = dst.extension().map(|s| s.to_string_lossy().into_owned());
    let mut n = 2;
    loop {
        let name = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let c = dst.with_file_name(name);
        if fs::symlink_metadata(&c).is_err() {
            return c;
        }
        n += 1;
    }
}

pub fn copy_rec(src: &Path, dst: &Path) -> io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        #[cfg(unix)]
        {
            return std::os::unix::fs::symlink(fs::read_link(src)?, dst);
        }
        #[cfg(not(unix))]
        {
            // Windows: Ziel der Verknüpfung kopieren (Ordner-Verknüpfungen werden nicht rekursiv verfolgt)
            return if fs::metadata(src)?.is_dir() {
                fs::create_dir_all(dst)
            } else {
                fs::copy(src, dst).map(|_| ())
            };
        }
    }
    if md.is_dir() {
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_rec(&e.path(), &dst.join(e.file_name()))?;
        }
    } else {
        fs::copy(src, dst)?;
    }
    Ok(())
}

pub fn remove_any(p: &Path) -> io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

/// Liegen beide Pfade auf demselben Datenträger? (Explorer-Regel: gleicher Datenträger = verschieben, sonst kopieren)
pub fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::metadata(a), fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev(),
            _ => true,
        }
    }
    #[cfg(not(unix))]
    {
        let root = |p: &Path| {
            p.components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        root(a) == root(b)
    }
}

pub fn folder_size(
    root: &Path,
    bytes: &std::sync::atomic::AtomicU64,
    files: &std::sync::atomic::AtomicU64,
) {
    use std::sync::atomic::Ordering::Relaxed;
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&d) {
            for e in rd.flatten() {
                match e.file_type() {
                    Ok(t) if t.is_dir() => stack.push(e.path()),
                    Ok(_) => {
                        files.fetch_add(1, Relaxed);
                        if let Ok(m) = e.metadata() {
                            bytes.fetch_add(m.len(), Relaxed);
                        }
                    }
                    Err(_) => {}
                }
            }
        }
    }
}

/// Kleingeschriebene Endung ohne Speicherzuweisung (für Zeilen, die jede Frame gezeichnet werden).
pub struct ExtBuf {
    buf: [u8; 16],
    len: usize,
    long: Option<String>,
}

impl ExtBuf {
    pub fn as_str(&self) -> &str {
        match &self.long {
            Some(s) => s,
            None => std::str::from_utf8(&self.buf[..self.len]).unwrap_or(""),
        }
    }
}

pub fn ext_buf(name: &str) -> ExtBuf {
    let ext = match name.rfind('.') {
        Some(p) if p > 0 && p + 1 < name.len() => &name[p + 1..],
        _ => "",
    };
    let mut b = ExtBuf {
        buf: [0; 16],
        len: 0,
        long: None,
    };
    if ext.len() <= 16 {
        for (i, c) in ext.bytes().enumerate() {
            b.buf[i] = c.to_ascii_lowercase();
        }
        b.len = ext.len();
    } else {
        b.long = Some(ext.to_ascii_lowercase());
    }
    b
}
