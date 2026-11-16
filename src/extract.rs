//! Schneller Entpacker für alle gängigen Archivformate.
//!
//! zip/jar/apk/cbz (parallel, mehrere Threads), 7z, rar, tar sowie tar.gz/.bz2/.xz/.zst
//! und einzeln komprimierte Dateien (.gz/.bz2/.xz/.zst). Das Format wird am Inhalt (Magic Bytes)
//! erkannt, nicht an der Endung.

use rayon::prelude::*;
use std::{
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
};

const SUFFIXES: [&str; 22] = [
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".tbz", ".txz", ".tzst", ".zip", ".jar", ".apk", ".cbz", ".7z", ".rar", ".cbr", ".tar", ".gz", ".bz2", ".xz", ".zst", ".war",
];

pub fn is_archive(name: &str) -> bool {
    let l = name.to_lowercase();
    SUFFIXES.iter().any(|s| l.ends_with(s))
}

/// Ordnername für „In Ordner entpacken“: Dateiname ohne Archiv-Endung.
pub fn folder_name(name: &str) -> String {
    let l = name.to_lowercase();
    for s in SUFFIXES {
        if l.ends_with(s) && name.len() > s.len() {
            return name[..name.len() - s.len()].to_string();
        }
    }
    format!("{name} (entpackt)")
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Codec {
    Plain,
    Gz,
    Bz2,
    Xz,
    Zst,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Zip,
    SevenZ,
    Rar,
    Compressed(Codec),
    Unknown,
}

fn sniff(path: &Path) -> io::Result<Kind> {
    let mut f = File::open(path)?;
    let mut h = [0u8; 8];
    let n = f.read(&mut h)?;
    let h = &h[..n];
    Ok(if h.starts_with(b"PK\x03\x04") || h.starts_with(b"PK\x05\x06") {
        Kind::Zip
    } else if h.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        Kind::SevenZ
    } else if h.starts_with(b"Rar!\x1a\x07") {
        Kind::Rar
    } else if h.starts_with(&[0x1f, 0x8b]) {
        Kind::Compressed(Codec::Gz)
    } else if h.starts_with(b"BZh") {
        Kind::Compressed(Codec::Bz2)
    } else if h.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0]) {
        Kind::Compressed(Codec::Xz)
    } else if h.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Kind::Compressed(Codec::Zst)
    } else {
        // reines tar?
        let mut buf = [0u8; 5];
        if f.seek(SeekFrom::Start(257)).is_ok() && f.read_exact(&mut buf).is_ok() && &buf == b"ustar" {
            Kind::Compressed(Codec::Plain)
        } else {
            Kind::Unknown
        }
    })
}

fn open_codec(path: &Path, c: Codec) -> io::Result<Box<dyn Read>> {
    let f = BufReader::with_capacity(1 << 18, File::open(path)?);
    Ok(match c {
        Codec::Plain => Box::new(f),
        Codec::Gz => Box::new(flate2::read::MultiGzDecoder::new(f)),
        Codec::Bz2 => Box::new(bzip2::read::MultiBzDecoder::new(f)),
        Codec::Xz => Box::new(xz2::read::XzDecoder::new_multi_decoder(f)),
        Codec::Zst => Box::new(zstd::stream::read::Decoder::new(f)?),
    })
}

fn safe_rel(p: &Path) -> bool {
    p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn e<E: std::fmt::Display>(x: E) -> String {
    x.to_string()
}

/// Entpackt `src` nach `dest`. Fortschritt (Anzahl Dateien) in `prog`. Rückgabe: Anzahl entpackter Dateien.
pub fn extract(src: &Path, dest: &Path, prog: &AtomicU64, cancel: &AtomicBool) -> Result<u64, String> {
    fs::create_dir_all(dest).map_err(e)?;
    let kind = sniff(src).map_err(e)?;
    match kind {
        Kind::Zip => extract_zip(src, dest, prog, cancel)?,
        Kind::SevenZ => extract_7z(src, dest, prog, cancel)?,
        Kind::Rar => extract_rar(src, dest, prog, cancel)?,
        Kind::Compressed(c) => {
            // enthält der Strom ein tar-Archiv?
            let mut head = [0u8; 512];
            let mut rd = open_codec(src, c).map_err(e)?;
            let mut got = 0;
            while got < 512 {
                match rd.read(&mut head[got..]).map_err(e)? {
                    0 => break,
                    n => got += n,
                }
            }
            if got >= 262 && &head[257..262] == b"ustar" {
                let rd = open_codec(src, c).map_err(e)?;
                let mut ar = tar::Archive::new(rd);
                ar.set_preserve_permissions(false);
                for ent in ar.entries().map_err(e)? {
                    if cancel.load(Relaxed) {
                        return Err("abgebrochen".into());
                    }
                    let mut ent = ent.map_err(e)?;
                    // Vorhandene Dateien werden NIE überschrieben: neue Datei bekommt „Name (2)“.
                    let rel = ent.path().map_err(e)?.into_owned();
                    let target = dest.join(&rel);
                    if safe_rel(&rel) && !ent.header().entry_type().is_dir() && fs::symlink_metadata(&target).is_ok() {
                        ent.unpack(crate::fsops::unique(target)).map_err(e)?;
                    } else {
                        ent.unpack_in(dest).map_err(e)?; // unpack_in verhindert Pfade außerhalb von `dest`
                    }
                    prog.fetch_add(1, Relaxed);
                }
            } else if c == Codec::Plain {
                return Err("Unbekanntes tar-Format".into());
            } else {
                // einzelne komprimierte Datei: Endung abschneiden
                let name = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "entpackt".into());
                let out = crate::fsops::unique(dest.join(name));
                let mut rd = open_codec(src, c).map_err(e)?;
                let mut w = BufWriter::with_capacity(1 << 18, File::create(out).map_err(e)?);
                io::copy(&mut rd, &mut w).map_err(e)?;
                prog.fetch_add(1, Relaxed);
            }
        }
        Kind::Unknown => return Err("Unbekanntes Archivformat".into()),
    }
    Ok(prog.load(Relaxed))
}

fn extract_zip(src: &Path, dest: &Path, prog: &AtomicU64, cancel: &AtomicBool) -> Result<(), String> {
    let n = {
        let f = BufReader::new(File::open(src).map_err(e)?);
        zip::ZipArchive::new(f).map_err(e)?.len()
    };
    if n == 0 {
        return Ok(());
    }
    let threads = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(4).clamp(1, 8).min(n);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).stack_size(1 << 20).build().map_err(e)?;
    let per = n.div_ceil(threads);
    let ranges: Vec<(usize, usize)> = (0..threads).map(|t| (t * per, ((t + 1) * per).min(n))).filter(|(a, b)| a < b).collect();
    let res: Result<(), String> = pool.install(|| {
        ranges.par_iter().try_for_each(|&(a, b)| -> Result<(), String> {
            // jeder Thread öffnet das Archiv selbst → echte Parallelität ohne gemeinsames Lock
            let mut ar = zip::ZipArchive::new(BufReader::with_capacity(1 << 18, File::open(src).map_err(e)?)).map_err(e)?;
            for i in a..b {
                if cancel.load(Relaxed) {
                    return Err("abgebrochen".into());
                }
                let mut zf = ar.by_index(i).map_err(|x| format!("{x} (Passwortgeschützte ZIPs werden nicht unterstützt)"))?;
                let Some(rel) = zf.enclosed_name() else { continue }; // Zip-Slip-Schutz
                if zf.is_symlink() {
                    continue;
                }
                let out = dest.join(rel);
                // vorhandene Dateien nie überschreiben
                let out = if !zf.is_dir() && fs::symlink_metadata(&out).is_ok() { crate::fsops::unique(out) } else { out };
                if zf.is_dir() {
                    fs::create_dir_all(&out).map_err(e)?;
                    continue;
                }
                if let Some(p) = out.parent() {
                    fs::create_dir_all(p).map_err(e)?;
                }
                let mut w = BufWriter::with_capacity(1 << 18, File::create(&out).map_err(e)?);
                io::copy(&mut zf, &mut w).map_err(e)?;
                #[cfg(unix)]
                if let Some(mode) = zf.unix_mode() {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o777));
                }
                prog.fetch_add(1, Relaxed);
            }
            Ok(())
        })
    });
    drop(pool);
    res
}

fn extract_7z(src: &Path, dest: &Path, prog: &AtomicU64, cancel: &AtomicBool) -> Result<(), String> {
    let f = File::open(src).map_err(e)?;
    sevenz_rust::decompress_with_extract_fn(f, dest, |entry, reader, path| {
        if cancel.load(Relaxed) {
            return Err(sevenz_rust::Error::io(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen")));
        }
        if !safe_rel(Path::new(entry.name())) {
            return Ok(true); // Pfad aus dem Zielordner heraus → überspringen
        }
        prog.fetch_add(1, Relaxed);
        let mut target = PathBuf::from(path);
        if !entry.is_directory() && fs::symlink_metadata(&target).is_ok() {
            target = crate::fsops::unique(target); // vorhandene Dateien nie überschreiben
        }
        sevenz_rust::default_entry_extract_fn(entry, reader, &target)
    })
    .map_err(e)
}

#[cfg(feature = "rar")]
fn extract_rar(src: &Path, dest: &Path, prog: &AtomicU64, cancel: &AtomicBool) -> Result<(), String> {
    let mut ar = unrar::Archive::new(src).open_for_processing().map_err(e)?;
    while let Some(h) = ar.read_header().map_err(e)? {
        if cancel.load(Relaxed) {
            return Err("abgebrochen".into());
        }
        if h.entry().is_file() {
            prog.fetch_add(1, Relaxed);
        }
        let target = dest.join(&h.entry().filename);
        ar = if h.entry().is_file() && fs::symlink_metadata(&target).is_ok() {
            h.extract_to(crate::fsops::unique(target)).map_err(e)? // nie überschreiben
        } else {
            h.extract_with_base(dest).map_err(e)?
        };
    }
    Ok(())
}

#[cfg(not(feature = "rar"))]
fn extract_rar(_: &Path, _: &Path, _: &AtomicU64, _: &AtomicBool) -> Result<(), String> {
    Err("RAR-Unterstützung wurde nicht mitkompiliert (Feature \"rar\")".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{compress, Format};
    use std::io::Write;

    #[test]
    fn all_formats() {
        let base = std::env::temp_dir().join(format!("fc-ex-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("src/sub/deep")).unwrap();
        for i in 0..50 {
            fs::write(base.join(format!("src/sub/f{i}.txt")), format!("inhalt {i}").repeat(100)).unwrap();
        }
        fs::write(base.join("src/sub/deep/x.bin"), vec![7u8; 100_000]).unwrap();
        fs::write(base.join("single.txt"), "nur eine Datei").unwrap();
        // Testarchive mit Foldex' eigenem Packer erzeugen – keine Kommandozeilenprogramme (zip, tar, 7z) nötig
        let mut cases = Vec::new();
        for (fmt, name) in [(Format::Zip, "a.zip"), (Format::TarGz, "a.tar.gz"), (Format::TarBz2, "a.tar.bz2"), (Format::TarXz, "a.tar.xz"), (Format::Tar, "a.tar"), (Format::TarZst, "a.tar.zst"), (Format::SevenZ, "a.7z")] {
            compress(&[base.join("src")], &base.join(name), fmt, &AtomicU64::new(0), &AtomicBool::new(false)).unwrap_or_else(|x| panic!("{name} packen: {x}"));
            cases.push(name);
        }
        // einzelne Dateien: gz, bz2, xz
        let data = fs::read(base.join("single.txt")).unwrap();
        let mut gz = flate2::write::GzEncoder::new(fs::File::create(base.join("single.txt.gz")).unwrap(), flate2::Compression::default());
        gz.write_all(&data).unwrap();
        gz.finish().unwrap();
        let mut bz = bzip2::write::BzEncoder::new(fs::File::create(base.join("single.txt.bz2")).unwrap(), bzip2::Compression::default());
        bz.write_all(&data).unwrap();
        bz.finish().unwrap();
        let mut xz = xz2::write::XzEncoder::new(fs::File::create(base.join("single.txt.xz")).unwrap(), 6);
        xz.write_all(&data).unwrap();
        xz.finish().unwrap();
        for c in cases {
            let out = base.join(format!("out-{c}"));
            let prog = AtomicU64::new(0);
            extract(&base.join(c), &out, &prog, &AtomicBool::new(false)).unwrap_or_else(|x| panic!("{c}: {x}"));
            assert_eq!(fs::read(out.join("src/sub/deep/x.bin")).unwrap().len(), 100_000, "{c}");
            assert_eq!(fs::read_to_string(out.join("src/sub/f7.txt")).unwrap(), "inhalt 7".repeat(100), "{c}");
            assert!(prog.load(Relaxed) >= 51, "{c}: {}", prog.load(Relaxed));
        }
        for c in ["single.txt.gz", "single.txt.bz2", "single.txt.xz"] {
            let out = base.join(format!("out-{c}"));
            extract(&base.join(c), &out, &AtomicU64::new(0), &AtomicBool::new(false)).unwrap();
            assert_eq!(fs::read_to_string(out.join("single.txt")).unwrap(), "nur eine Datei", "{c}");
        }
        assert!(is_archive("Foo.TAR.GZ") && !is_archive("foo.txt"));
        assert_eq!(folder_name("foo.tar.gz"), "foo");
        let _ = fs::remove_dir_all(&base);
    }
}
