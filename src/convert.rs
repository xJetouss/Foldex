//! Dateiformate: Einordnung nach Art (Bild, Audio, Video, Text …) und Konvertierung.
//! - Bilder: JPG, PNG, WebP, GIF, BMP, TIFF, ICO, SVG direkt; AVIF über ffmpeg; HEIC/AVIF/PDF werden über
//!   Windows gelesen (erste Seite bei PDF). Ziel PDF: Bild als PDF-Seite. SVG als Ziel: Bild eingebettet.
//! - Video (MP4, MOV, MKV, WebM, AVI, WMV, FLV, MPEG, 3GP, OGV, M4V, TS, GIF) und Audio (MP3, WAV, FLAC,
//!   AAC, M4A, OGG, Opus, AIFF, WMA, AMR, AC3): über ffmpeg, sofern installiert. MIDI enthält Noten statt
//!   Klang und lässt sich ohne Synthesizer nicht umwandeln.
//! - Dokumente (PDF, DOCX, DOC, ODT, RTF, TXT, HTML, MD, PAGES, EPUB, CSV, JSON): siehe docconv.rs.
//! - Archive (ZIP, 7z, RAR, TAR, TAR.GZ/BZ2/XZ/ZST): werden entpackt und im Zielformat neu gepackt (RAR nur lesend).
//! - Video → Bild: Standbild (PNG/JPG/WebP/BMP) aus der ersten Sekunde.
//! Jede Konvertierung meldet ihren Fortschritt (0 … 1000, bei ffmpeg anhand der Laufzeit) und lässt sich abbrechen;
//! halbfertige Zieldateien werden dann entfernt.
//! Wird beim Umbenennen die Endung in ein konvertierbares Format geändert, konvertiert Foldex automatisch;
//! bei einem Wechsel in eine fremde Dateiart fragt es vorher nach.

use crate::ops::Format;
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Image,
    Audio,
    Video,
    Text,
    Archive,
    Document,
    Other,
}

pub const IMAGE_OUT: &[&str] = &["jpg", "png", "webp", "avif", "gif", "bmp", "tiff", "tga", "qoi", "ppm", "hdr", "svg", "ico", "pdf"];
pub const AUDIO_OUT: &[&str] = &["mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "aiff", "wma", "amr", "ac3", "mp2", "mka", "caf", "au"];
pub const VIDEO_OUT: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "wmv", "flv", "mpeg", "3gp", "ogv", "m4v", "ts", "m2ts", "gif"];
/// Standbild aus einem Video
pub const FRAME_OUT: &[&str] = &["png", "jpg", "webp", "bmp"];
/// Archiv-Zielformate (Endungen ohne Punkt, wie `Format::ext`)
pub const ARCHIVE_OUT: &[&str] = &["zip", "7z", "tar", "tar.gz", "tar.bz2", "tar.xz", "tar.zst"];

pub fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn class(ext: &str) -> Class {
    match ext {
        "png" | "jpg" | "jpeg" | "jfif" | "gif" | "bmp" | "ico" | "webp" | "tif" | "tiff" | "svg" | "avif" | "heic" | "heif" | "tga" | "qoi" | "ppm" | "pgm" | "pbm" | "pnm" | "hdr" | "dds"
        // Kamera-RAW: gelesen über Windows (WIC) bzw. ffmpeg
        | "cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2" => Class::Image,
        "mp3" | "wav" | "flac" | "ogg" | "oga" | "m4a" | "m4b" | "aac" | "opus" | "wma" | "aiff" | "aif" | "amr" | "ac3" | "eac3" | "dts" | "mka" | "caf" | "au" | "mp2" | "mid" | "midi" => Class::Audio,
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "m4v" | "flv" | "mpeg" | "mpg" | "3gp" | "ogv" | "m2ts" | "mts" | "vob" | "mxf" => Class::Video,
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "tbz2" | "xz" | "txz" | "zst" | "tzst" | "cbz" | "cbr" | "jar" => Class::Archive,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "rtf" | "epub" | "pages" => Class::Document,
        e if crate::editors::is_text(&format!("x.{e}")) => Class::Text,
        _ => Class::Other,
    }
}

/// Art einer Datei; „.ts“ ist meist TypeScript, als MPEG-Transportstrom (Sync-Byte 0x47) aber Video.
pub fn class_of(p: &Path) -> Class {
    let e = ext_of(p);
    if e == "ts" {
        let mut b = [0u8; 189];
        let n = std::fs::File::open(p).and_then(|mut f| std::io::Read::read(&mut f, &mut b)).unwrap_or(0);
        if n >= 189 && b[0] == 0x47 && b[188] == 0x47 {
            return Class::Video;
        }
    }
    class(&e)
}

pub fn class_name(c: Class) -> &'static str {
    match c {
        Class::Image => "Bild",
        Class::Audio => "Audio",
        Class::Video => "Video",
        Class::Text => "Text",
        Class::Archive => "Archiv",
        Class::Document => "Dokument",
        Class::Other => "Datei",
    }
}

/// Programm suchen: PATH und die üblichen Installationsorte (winget, Scoop, Chocolatey, C:\ffmpeg) – so wird
/// ein frisch per winget installiertes ffmpeg auch ohne geänderten PATH gefunden.
fn find_tool(name: &str) -> Option<PathBuf> {
    if let Some(p) = crate::editors::which(name) {
        return Some(p);
    }
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(l) = dirs::data_local_dir() {
        let wg = l.join("Microsoft").join("WinGet");
        dirs.push(wg.join("Links"));
        // winget legt das Paket unter Packages\Gyan.FFmpeg_…\ffmpeg-…\bin ab
        for e in std::fs::read_dir(wg.join("Packages")).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().to_lowercase().contains("ffmpeg") {
                dirs.extend(std::fs::read_dir(e.path()).into_iter().flatten().flatten().map(|i| i.path().join("bin")));
            }
        }
    }
    if let Some(h) = dirs::home_dir() {
        dirs.push(h.join("scoop").join("shims"));
    }
    dirs.extend([r"C:\ProgramData\chocolatey\bin", r"C:\ffmpeg\bin", r"C:\Program Files\ffmpeg\bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// ffmpeg-Pfad. Fehlt es, wird höchstens alle 5 s erneut gesucht – nach einer Installation ist es so ohne
/// Neustart nutzbar, ohne dass jedes Bild die Platte durchsucht.
pub fn ffmpeg() -> Option<PathBuf> {
    static C: std::sync::Mutex<(Option<PathBuf>, Option<std::time::Instant>)> = std::sync::Mutex::new((None, None));
    let mut g = C.lock().ok()?;
    if g.0.is_none() && g.1.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(5)) {
        *g = (find_tool("ffmpeg"), Some(std::time::Instant::now()));
    }
    g.0.clone()
}

/// ffprobe liegt normalerweise neben ffmpeg.
pub fn ffprobe() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" };
    ffmpeg().and_then(|f| f.parent().map(|d| d.join(exe))).filter(|p| p.is_file()).or_else(|| find_tool("ffprobe"))
}

/// Befehl ohne Konsolenfenster.
pub fn quiet(prog: &Path) -> Command {
    let mut c = Command::new(prog);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Laufzeit in Sekunden (über ffprobe).
pub fn duration(src: &Path) -> Option<f64> {
    let out = quiet(&ffprobe()?).args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"]).arg(src).stdin(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok().filter(|d| *d > 0.0)
}

/// Ist die Datei ein Archiv, das sich in ein anderes Archivformat umpacken lässt?
fn archive_src(p: &Path) -> bool {
    p.file_name().is_some_and(|n| crate::extract::is_archive(&n.to_string_lossy()))
}

/// Name ohne (ggf. doppelte) Endung: „a.tar.gz“ → „a“, „bild.png“ → „bild“.
pub fn base_name(p: &Path) -> String {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if archive_src(p) {
        return crate::extract::folder_name(&name);
    }
    p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name)
}

/// Gleiches Format unter anderem Namen?
fn same(a: &str, b: &str) -> bool {
    const GROUPS: &[&[&str]] = &[&["jpg", "jpeg", "jfif"], &["tif", "tiff"], &["htm", "html"], &["mpeg", "mpg"], &["aif", "aiff"], &["mid", "midi"], &["heic", "heif"], &["md", "markdown"], &["ogg", "oga"]];
    a == b || GROUPS.iter().any(|g| g.contains(&a) && g.contains(&b))
}

/// Dokument-artig (für docconv): Office, PDF, E-Books und Text-Dokumente.
fn doc_like(e: &str) -> bool {
    matches!(e, "pdf" | "docx" | "doc" | "odt" | "rtf" | "txt" | "html" | "htm" | "md" | "markdown" | "pages" | "epub" | "csv" | "json" | "xlsx" | "xls" | "ods" | "pptx" | "ppt" | "odp")
}

/// Lässt sich `from` nach `to` konvertieren? (`ts` als Quelle hier immer Video – siehe `class_of`)
pub fn can_convert(from: &str, to: &str) -> bool {
    if same(from, to) {
        return false;
    }
    let ff = || ffmpeg().is_some();
    let cf = if from == "ts" { Class::Video } else { class(from) };
    let ct = if to == "ts" { Class::Video } else { class(to) };
    match (cf, ct) {
        _ if matches!(from, "mid" | "midi") || matches!(to, "mid" | "midi" | "heic" | "heif") => false,
        // PDF/Pages → Bild: erste Seite bzw. Vorschau
        (Class::Document, Class::Image) if from == "pdf" => cfg!(windows) && IMAGE_OUT.contains(&to) && to != "pdf" && to != "svg",
        (Class::Document, Class::Image) if from == "pages" => matches!(to, "png" | "jpg" | "jpeg"),
        (Class::Image, _) if to == "pdf" => true,
        (Class::Image, Class::Image) => (IMAGE_OUT.contains(&to) || matches!(to, "jpeg" | "tif")) && (to != "avif" || ff()),
        (Class::Image, Class::Video) if from == "gif" => ff(),
        (Class::Audio, Class::Audio) | (Class::Video, Class::Video) | (Class::Video, Class::Audio) => ff(),
        (Class::Video, Class::Image) if to == "gif" || to == "jpeg" || FRAME_OUT.contains(&to) => ff(),
        _ if doc_like(from) && doc_like(to) => crate::docconv::can(from, to),
        _ => false,
    }
}
/// Was passiert, wenn die Endung beim Umbenennen geändert wird?
#[derive(Debug, PartialEq)]
pub enum RenameKind {
    /// keine oder unbedenkliche Änderung (z. B. txt → md)
    Plain,
    /// automatisch konvertieren
    Convert,
    /// nachfragen: Datei könnte unbrauchbar werden
    Warn(String),
}

pub fn rename_kind(old: &str, new: &str, is_dir: bool) -> RenameKind {
    // Archive: Format am ganzen Namen erkennen („.tar.gz“ ist eine Endung)
    if !is_dir && crate::extract::is_archive(old) && crate::extract::is_archive(new) {
        let (fa, fb) = (Format::from_name(old), Format::from_name(new));
        return match fb {
            _ if fa == fb && fa.is_some() => RenameKind::Plain,
            Some(_) => RenameKind::Convert,
            None => RenameKind::Warn(format!("„{new}“: In dieses Archivformat kann Foldex nicht packen (z. B. RAR) – nur umbenannt wird die Datei unbrauchbar.")),
        };
    }
    let (a, b) = (ext_of(Path::new(old)), ext_of(Path::new(new)));
    if is_dir || same(&a, &b) || a.is_empty() {
        return RenameKind::Plain;
    }
    let (ca, cb) = (class(&a), class(&b));
    // Text bleibt Text: txt ↔ md (gleicher Inhalt) und Quelltext nur umbenennen; echte Formatwechsel wie
    // md → html, csv → json oder html → txt werden umgewandelt
    if ca == Class::Text && cb == Class::Text {
        let plain_pair = matches!(a.as_str(), "txt" | "md" | "markdown") && matches!(b.as_str(), "txt" | "md" | "markdown");
        return if !plain_pair && doc_like(&a) && doc_like(&b) && can_convert(&a, &b) { RenameKind::Convert } else { RenameKind::Plain };
    }
    if can_convert(&a, &b) {
        return RenameKind::Convert;
    }
    let why = if b.is_empty() {
        format!("Die Endung „.{a}“ wird entfernt. Windows weiß danach nicht mehr, mit welchem Programm die Datei geöffnet wird.")
    } else if matches!(a.as_str(), "mid" | "midi") || matches!(b.as_str(), "mid" | "midi") {
        format!("„.{a}“ → „.{b}“: MIDI enthält Noten, keine Tonaufnahme – das lässt sich nicht umwandeln. Nur umbenannt wird die Datei unbrauchbar.")
    } else if matches!(b.as_str(), "heic" | "heif") {
        format!("„.{a}“ → „.{b}“: HEIC kann nur gelesen, nicht geschrieben werden. Nur umbenannt wird die Datei unbrauchbar.")
    } else if ca == cb && ca != Class::Other {
        format!("„.{a}“ → „.{b}“: Beides ist {}, aber der Inhalt wird dadurch nicht umgewandelt – die Datei lässt sich danach evtl. nicht mehr öffnen.", class_name(ca))
    } else if (ca == Class::Audio || ca == Class::Video) && (cb == Class::Audio || cb == Class::Video) {
        format!("„.{a}“ → „.{b}“: Zum Umwandeln wird ffmpeg benötigt (nicht gefunden). Nur umbenannt bleibt der Inhalt {}.", class_name(ca))
    } else {
        format!("„.{a}“ ({}) → „.{b}“ ({}): Der Inhalt bleibt {} – die Datei lässt sich danach evtl. nicht mehr öffnen.", class_name(ca), class_name(cb), class_name(ca))
    };
    RenameKind::Warn(why)
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Bild laden; SVG wird gerendert (längste Seite höchstens `svg_px`), ICO liefert das größte Bild.
/// Was das image-Paket nicht kennt (HEIC, AVIF, PDF …), liest Windows bzw. ffmpeg.
pub fn load_image(src: &Path, svg_px: u32) -> Result<image::DynamicImage, String> {
    let e = ext_of(src);
    if e == "svg" {
        use resvg::{tiny_skia, usvg};
        let data = std::fs::read(src).map_err(|e| e.to_string())?;
        let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).map_err(|e| format!("SVG ungültig: {e}"))?;
        let sz = tree.size();
        let scale = (svg_px as f32 / sz.width().max(sz.height()).max(1.0)).min(8.0);
        let (w, h) = (((sz.width() * scale).ceil() as u32).max(1), ((sz.height() * scale).ceil() as u32).max(1));
        let mut pm = tiny_skia::Pixmap::new(w, h).ok_or("SVG zu groß")?;
        resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pm.as_mut());
        let px: Vec<u8> = pm.pixels().iter().flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        }).collect();
        let img = image::RgbaImage::from_raw(w, h, px).ok_or("SVG konnte nicht gerendert werden")?;
        return Ok(image::DynamicImage::ImageRgba8(img));
    }
    if e == "pages" {
        let tmp = std::env::temp_dir().join(format!("foldex-pages-{}.jpg", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        crate::docconv::convert(src, &tmp)?;
        let r = image::open(&tmp).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&tmp);
        return r;
    }
    let direct = image::ImageReader::open(src)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| format!("Bild kann nicht gelesen werden: {e}"));
    if direct.is_ok() {
        return direct;
    }
    // Windows (WIC/Shell): HEIC, AVIF, PDF (erste Seite), RAW …
    if let Some(ci) = crate::preview::shell_image(src, if e == "pdf" { 2480 } else { 4096 }) {
        let px: Vec<u8> = ci.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        if let Some(img) = image::RgbaImage::from_raw(ci.size[0] as u32, ci.size[1] as u32, px) {
            return Ok(image::DynamicImage::ImageRgba8(img));
        }
    }
    // ffmpeg: erstes Bild als PNG
    if ffmpeg().is_some() {
        let tmp = std::env::temp_dir().join(format!("foldex-frame-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        if run_ffmpeg(&[src.as_os_str().into(), "-frames:v".into(), "1".into(), tmp.as_os_str().into()]).is_ok() {
            let r = image::open(&tmp).map_err(|e| e.to_string());
            let _ = std::fs::remove_file(&tmp);
            return r;
        }
    }
    direct
}

fn flatten_white(img: &image::DynamicImage) -> image::RgbImage {
    let rgba = img.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (o, p) in rgb.pixels_mut().zip(rgba.pixels()) {
        let a = p[3] as u32;
        *o = image::Rgb([0, 1, 2].map(|i| ((p[i] as u32 * a + 255 * (255 - a)) / 255) as u8));
    }
    rgb
}

fn jpeg_bytes(img: &image::DynamicImage, q: u8) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q).encode_image(&flatten_white(img)).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Bild in ein anderes Format umwandeln.
pub fn convert_image(src: &Path, dst: &Path) -> Result<(), String> {
    let to = ext_of(dst);
    let img = load_image(src, if to == "ico" { 256 } else { 2048 })?;
    match to.as_str() {
        "svg" => {
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
            let (w, h) = (img.width(), img.height());
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n  <image width=\"{w}\" height=\"{h}\" xlink:href=\"data:image/png;base64,{}\"/>\n</svg>\n",
                base64(&png)
            );
            std::fs::write(dst, svg).map_err(|e| e.to_string())
        }
        "ico" => {
            // ICO: höchstens 256 × 256
            let img = if img.width() > 256 || img.height() > 256 { img.resize(256, 256, image::imageops::FilterType::Lanczos3) } else { img };
            image::DynamicImage::ImageRgba8(img.to_rgba8()).save_with_format(dst, image::ImageFormat::Ico).map_err(|e| e.to_string())
        }
        // JPG kennt keine Transparenz → auf Weiß legen
        "jpg" | "jpeg" | "jfif" => std::fs::write(dst, jpeg_bytes(&img, 92)?).map_err(|e| e.to_string()),
        "pdf" => {
            let jpg = jpeg_bytes(&img, 90)?;
            std::fs::write(dst, crate::pdf::image_pdf(&[(jpg, img.width(), img.height())])).map_err(|e| e.to_string())
        }
        "avif" => {
            // AVIF schreibt ffmpeg (AV1-Encoder)
            let tmp = std::env::temp_dir().join(format!("foldex-avif-{}.png", std::process::id()));
            img.save_with_format(&tmp, image::ImageFormat::Png).map_err(|e| e.to_string())?;
            let r = run_ffmpeg(&[tmp.as_os_str().into(), "-still-picture".into(), "1".into(), dst.as_os_str().into()]);
            let _ = std::fs::remove_file(&tmp);
            r
        }
        _ => {
            let fmt = image::ImageFormat::from_extension(&to).ok_or_else(|| format!("Format „{to}“ wird nicht unterstützt"))?;
            // jedes Format bekommt die Pixelart, die sein Encoder schreiben kann
            let img = match fmt {
                image::ImageFormat::Bmp | image::ImageFormat::Gif | image::ImageFormat::Tga | image::ImageFormat::Qoi => image::DynamicImage::ImageRgba8(img.to_rgba8()),
                image::ImageFormat::Pnm => image::DynamicImage::ImageRgb8(flatten_white(&img)),
                image::ImageFormat::Hdr => image::DynamicImage::ImageRgb32F(img.to_rgb32f()),
                _ => img,
            };
            img.save_with_format(dst, fmt).map_err(|e| e.to_string())
        }
    }
}

const NO_FFMPEG: &str = "ffmpeg ist nicht installiert (z. B. „winget install Gyan.FFmpeg“)";

/// ffmpeg aufrufen (blockiert): `args` beginnt mit der Eingabedatei. Fehlermeldung = letzte Zeile.
fn run_ffmpeg(args: &[OsString]) -> Result<(), String> {
    let mut a: Vec<OsString> = vec!["-i".into()];
    a.extend_from_slice(args);
    run_ffmpeg_raw(&a, None, &AtomicU64::new(0), &AtomicBool::new(false))
}

/// ffmpeg mit vollständiger Argumentliste (inkl. „-i“). `-progress` meldet die verarbeitete Zeit; mit der
/// Laufzeit `dur` wird daraus der Anteil (0 … 1000) in `prog`. `cancel` beendet ffmpeg sofort.
fn run_ffmpeg_raw(args: &[OsString], dur: Option<f64>, prog: &AtomicU64, cancel: &AtomicBool) -> Result<(), String> {
    let ff = ffmpeg().ok_or(NO_FFMPEG)?;
    let mut c = quiet(&ff);
    c.args(["-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-n"]).args(args);
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("ffmpeg konnte nicht gestartet werden: {e}"))?;
    let mut stderr = child.stderr.take().ok_or("keine Fehlerausgabe")?;
    let errt = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let pid = child.id();
    // Abbruch auch dann sofort, wenn ffmpeg gerade nichts ausgibt
    let stop = AtomicBool::new(false);
    let watch = std::thread::scope(|sc| {
        sc.spawn(|| {
            while !stop.load(Relaxed) {
                if cancel.load(Relaxed) {
                    kill_pid(pid);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
        });
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                // out_time_us (neuere) bzw. out_time_ms (ältere ffmpeg-Versionen) – beides Mikrosekunden
                let us = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")).and_then(|v| v.trim().parse::<f64>().ok());
                if let (Some(us), Some(d)) = (us, dur) {
                    prog.store(((us / 1e6 / d) * 1000.0).clamp(0.0, 999.0) as u64, Relaxed);
                }
            }
        }
        let st = child.wait();
        stop.store(true, Relaxed);
        st
    });
    let status = watch.map_err(|e| e.to_string())?;
    let err = errt.join().unwrap_or_default();
    if cancel.load(Relaxed) {
        return Err("abgebrochen".into());
    }
    if status.success() {
        prog.store(1000, Relaxed);
        Ok(())
    } else {
        Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg-Fehler").to_string())
    }
}

/// Prozess beenden (für den Abbruch aus einem anderen Thread, ohne `Child` zu teilen).
fn kill_pid(pid: u32) {
    #[cfg(windows)]
    {
        let _ = quiet(Path::new("taskkill")).args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

/// Audio/Video mit ffmpeg umwandeln (blockiert – im Hintergrund aufrufen). Codecs je Ziel so gewählt,
/// dass die Datei überall abspielbar ist.
pub fn convert_media(src: &Path, dst: &Path, prog: &AtomicU64, cancel: &AtomicBool) -> Result<(), String> {
    let to = ext_of(dst);
    let video_src = class_of(src) == Class::Video || ext_of(src) == "gif";
    if video_src && class(&to) == Class::Image && to != "gif" {
        // Standbild aus der ersten Sekunde (bei sehr kurzen Videos vom Anfang)
        let at = if duration(src).is_some_and(|d| d > 2.0) { "1" } else { "0" };
        let mut a: Vec<OsString> = ["-ss", at, "-i"].map(OsString::from).to_vec();
        a.push(src.as_os_str().into());
        a.extend(["-frames:v", "1"].map(OsString::from));
        if matches!(to.as_str(), "jpg" | "jpeg") {
            a.extend(["-q:v", "2"].map(OsString::from));
        }
        a.push(dst.as_os_str().into());
        return run_ffmpeg_raw(&a, None, prog, cancel);
    }
    let mut a: Vec<OsString> = vec!["-i".into(), src.as_os_str().into()];
    let mut push = |v: &[&str]| a.extend(v.iter().map(OsString::from));
    if class(&to) == Class::Audio {
        push(&["-vn"]);
        match to.as_str() {
            "mp3" => push(&["-c:a", "libmp3lame", "-q:a", "2"]),
            "aac" | "m4a" => push(&["-c:a", "aac", "-b:a", "192k"]),
            "ogg" | "oga" => push(&["-c:a", "libvorbis", "-q:a", "5"]),
            "opus" => push(&["-c:a", "libopus", "-b:a", "128k"]),
            "amr" => push(&["-ar", "8000", "-ac", "1", "-c:a", "libopencore_amrnb", "-b:a", "12.2k"]),
            "wma" => push(&["-c:a", "wmav2", "-b:a", "192k"]),
            "ac3" => push(&["-c:a", "ac3", "-b:a", "384k"]),
            "aiff" | "aif" => push(&["-c:a", "pcm_s16be"]),
            "wav" => push(&["-c:a", "pcm_s16le"]),
            "mka" => push(&["-c:a", "libopus", "-b:a", "160k"]),
            "caf" => push(&["-c:a", "alac"]),
            "au" => push(&["-c:a", "pcm_s16be"]),
            "mp2" => push(&["-c:a", "mp2", "-b:a", "224k"]),
            _ => {}
        }
    } else if to == "gif" {
        push(&["-vf", "fps=12,scale=480:-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse", "-loop", "0"]);
    } else if video_src {
        // gerade Abmessungen (H.264 verlangt sie) und Standard-Farbformat
        let even = "scale=trunc(iw/2)*2:trunc(ih/2)*2";
        match to.as_str() {
            "mp4" | "m4v" | "mov" => push(&["-c:v", "libx264", "-preset", "medium", "-crf", "21", "-pix_fmt", "yuv420p", "-vf", even, "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart"]),
            "mkv" => push(&["-c:v", "libx264", "-preset", "medium", "-crf", "21", "-pix_fmt", "yuv420p", "-vf", even, "-c:a", "aac", "-b:a", "160k"]),
            "ts" | "m2ts" => push(&["-c:v", "libx264", "-crf", "21", "-pix_fmt", "yuv420p", "-vf", even, "-c:a", "aac", "-f", "mpegts"]),
            "webm" => push(&["-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0", "-pix_fmt", "yuv420p", "-c:a", "libopus"]),
            "ogv" => push(&["-c:v", "libtheora", "-q:v", "7", "-c:a", "libvorbis"]),
            "avi" => push(&["-c:v", "mpeg4", "-q:v", "4", "-c:a", "libmp3lame"]),
            "wmv" => push(&["-c:v", "wmv2", "-b:v", "3M", "-c:a", "wmav2"]),
            "flv" => push(&["-c:v", "libx264", "-pix_fmt", "yuv420p", "-vf", even, "-c:a", "aac"]),
            "mpeg" | "mpg" => push(&["-c:v", "mpeg2video", "-q:v", "4", "-c:a", "mp2", "-f", "mpeg"]),
            "3gp" => push(&["-c:v", "libx264", "-profile:v", "baseline", "-pix_fmt", "yuv420p", "-vf", "scale=-2:480", "-c:a", "aac", "-ac", "1", "-b:a", "64k"]),
            _ => {}
        }
    }
    a.push(dst.as_os_str().into());
    run_ffmpeg_raw(&a, duration(src), prog, cancel)
}

/// Archiv umpacken: in einen temporären Ordner entpacken (Fortschritt 0–50 %), dann im Zielformat packen (50–100 %).
fn convert_archive(src: &Path, dst: &Path, prog: &AtomicU64, cancel: &AtomicBool) -> Result<(), String> {
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let fmt = Format::from_name(&name).ok_or_else(|| format!("„{name}“: in dieses Archivformat kann nicht gepackt werden"))?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let tmp = std::env::temp_dir().join(format!("foldex-repack-{}-{nanos}", std::process::id()));
    let result = (|| {
        let total = crate::extract::extract(src, &tmp, &AtomicU64::new(0), cancel)?.max(1);
        prog.store(500, Relaxed);
        if cancel.load(Relaxed) {
            return Err("abgebrochen".to_string());
        }
        let srcs: Vec<PathBuf> = std::fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
        let (packed, done) = (AtomicU64::new(0), AtomicBool::new(false));
        std::thread::scope(|sc| {
            sc.spawn(|| {
                while !done.load(Relaxed) {
                    prog.store(500 + (packed.load(Relaxed) * 499 / total).min(499), Relaxed);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });
            let r = crate::ops::compress(&srcs, dst, fmt, &packed, cancel);
            done.store(true, Relaxed);
            r
        })?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Konvertiert `src` nach `dst` (Ziel darf nicht existieren).
pub fn convert(src: &Path, dst: &Path) -> Result<(), String> {
    convert_progress(src, dst, &AtomicU64::new(0), &AtomicBool::new(false))
}

/// Wie `convert`, mit Fortschritt (0 … 1000) und Abbruch. Bei Fehler/Abbruch wird das halbe Ziel entfernt.
pub fn convert_progress(src: &Path, dst: &Path, prog: &AtomicU64, cancel: &AtomicBool) -> Result<(), String> {
    if dst.exists() {
        return Err(format!("{} existiert bereits", dst.display()));
    }
    let (from, to) = (ext_of(src), ext_of(dst));
    let dst_name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (cf, ct) = (class_of(src), if to == "ts" { Class::Video } else { class(&to) });
    let r = match (cf, ct) {
        _ if archive_src(src) && Format::from_name(&dst_name).is_some() => convert_archive(src, dst, prog, cancel),
        (Class::Image, Class::Image) | (Class::Image, Class::Document) => convert_image(src, dst),
        (Class::Document, Class::Image) if from == "pdf" => convert_image(src, dst),
        (Class::Image, _) => convert_media(src, dst, prog, cancel),
        (Class::Video | Class::Audio, _) => convert_media(src, dst, prog, cancel),
        _ => crate::docconv::convert(src, dst),
    };
    match &r {
        Ok(()) => prog.store(1000, Relaxed),
        Err(_) => {
            let _ = std::fs::remove_file(dst);
        }
    }
    r
}

/// Mögliche Zielformate für „Konvertieren in …“.
pub fn targets(src: &Path) -> Vec<&'static str> {
    let e = ext_of(src);
    let cls = class_of(src);
    let from = e.as_str();
    if archive_src(src) {
        let own = src.file_name().and_then(|n| Format::from_name(&n.to_string_lossy()));
        return ARCHIVE_OUT.iter().copied().filter(|t| Format::from_name(&format!("x.{t}")) != own).collect();
    }
    let mut list: Vec<&'static str> = match cls {
        Class::Image if from == "gif" => IMAGE_OUT.iter().chain(["mp4", "webm"].iter()).copied().collect(),
        Class::Image => IMAGE_OUT.to_vec(),
        Class::Audio => AUDIO_OUT.to_vec(),
        Class::Video => VIDEO_OUT.iter().chain(["mp3", "wav", "m4a", "flac", "ogg", "opus"].iter()).chain(FRAME_OUT).copied().collect(),
        Class::Document if from == "pdf" => crate::docconv::DOC_OUT.iter().chain(["png", "jpg", "webp"].iter()).copied().collect(),
        Class::Document if from == "pages" => vec!["pdf", "png", "jpg"],
        _ if doc_like(from) => crate::docconv::DOC_OUT.to_vec(),
        _ => Vec::new(),
    };
    list.retain(|t| can_convert(from, t));
    let mut seen = std::collections::HashSet::new();
    list.retain(|t| seen.insert(*t));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_images() {
        assert_eq!(rename_kind("icon.ico", "icon.svg", false), RenameKind::Convert);
        assert!(matches!(rename_kind("icon.ico", "icon.mp3", false), RenameKind::Warn(_)));
        assert_eq!(rename_kind("a.txt", "a.md", false), RenameKind::Plain);
        assert_eq!(rename_kind("ordner.alt", "ordner.neu", true), RenameKind::Plain);
        assert!(matches!(rename_kind("a.png", "a", false), RenameKind::Warn(_)));
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        // PNG → ICO → SVG → PNG
        let d = std::env::temp_dir().join(format!("fx-conv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut img = image::RgbaImage::new(300, 200);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([x as u8, 80, 160, 255]);
        }
        img.save(d.join("a.png")).unwrap();
        convert(&d.join("a.png"), &d.join("a.ico")).unwrap();
        let ico = load_image(&d.join("a.ico"), 256).unwrap();
        assert!(ico.width() <= 256);
        convert(&d.join("a.ico"), &d.join("a.svg")).unwrap();
        assert!(std::fs::read_to_string(d.join("a.svg")).unwrap().contains("data:image/png;base64,"));
        convert(&d.join("a.svg"), &d.join("b.jpg")).unwrap();
        assert!(load_image(&d.join("b.jpg"), 0).unwrap().width() > 0);
        assert!(convert(&d.join("a.png"), &d.join("a.ico")).is_err()); // nie überschreiben
        // Bild → PDF, Dokumente, Regeln
        convert(&d.join("a.png"), &d.join("a.pdf")).unwrap();
        assert!(std::fs::read(d.join("a.pdf")).unwrap().starts_with(b"%PDF"));
        std::fs::write(d.join("n.md"), "# Hallo\n\nWelt").unwrap();
        convert(&d.join("n.md"), &d.join("n.docx")).unwrap();
        convert(&d.join("n.docx"), &d.join("n2.txt")).unwrap();
        assert!(std::fs::read_to_string(d.join("n2.txt")).unwrap().contains("Welt"));
        assert_eq!(rename_kind("a.md", "a.txt", false), RenameKind::Plain);
        assert_eq!(rename_kind("a.md", "a.html", false), RenameKind::Convert);
        assert_eq!(rename_kind("a.csv", "a.json", false), RenameKind::Convert);
        assert_eq!(rename_kind("main.rs", "main.txt", false), RenameKind::Plain);
        assert_eq!(rename_kind("t.csv", "t.xlsx", false), RenameKind::Convert);
        assert_eq!(rename_kind("a.docx", "a.pdf", false), RenameKind::Convert);
        assert!(matches!(rename_kind("a.mid", "a.mp3", false), RenameKind::Warn(_)));
        assert!(targets(&d.join("n.md")).contains(&"epub"));
        assert!(targets(&d.join("a.png")).contains(&"pdf"));
        assert!(targets(&d.join("a.png")).contains(&"tga"));
        // neue Bildformate
        for e in ["tga", "qoi", "ppm", "hdr"] {
            convert(&d.join("a.png"), &d.join(format!("q.{e}"))).unwrap();
            assert_eq!(load_image(&d.join(format!("q.{e}")), 0).unwrap().width(), 300, "{e}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Jedes Ziel, das „Konvertieren in …“ anbietet, muss auch wirklich funktionieren.
    #[test]
    fn every_offered_target_works() {
        let d = std::env::temp_dir().join(format!("fx-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut img = image::RgbaImage::new(64, 48);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([x as u8 * 4, y as u8 * 5, 120, 255]);
        }
        img.save(d.join("s.png")).unwrap();
        std::fs::write(d.join("s.md"), "# Titel\n\nAbsatz mit **fett**.\n\n- Punkt eins\n- Punkt zwei\n").unwrap();
        std::fs::write(d.join("s.csv"), "Name,Wert,PLZ\n\"A, B\",1.5,01234\nC,2,99999\n").unwrap();
        // weitere Quellen aus den ersten erzeugen
        for (from, to) in [("s.png", "s.jpg"), ("s.png", "s.gif"), ("s.png", "s.bmp"), ("s.png", "s.webp"), ("s.png", "s.tiff"), ("s.png", "s.ico"), ("s.png", "s.svg"), ("s.png", "s.tga"), ("s.md", "s.docx"), ("s.md", "s.odt"), ("s.md", "s.rtf"), ("s.md", "s.html"), ("s.md", "s.txt"), ("s.md", "s.epub"), ("s.csv", "s.json"), ("s.csv", "s.xlsx")] {
            convert(&d.join(from), &d.join(to)).unwrap_or_else(|e| panic!("{from} → {to}: {e}"));
        }
        let mut failed = Vec::new();
        let mut n = 0;
        for src in ["s.png", "s.jpg", "s.gif", "s.bmp", "s.webp", "s.tiff", "s.ico", "s.svg", "s.tga", "s.md", "s.docx", "s.odt", "s.rtf", "s.html", "s.txt", "s.epub", "s.csv", "s.json", "s.xlsx"] {
            let sp = d.join(src);
            for t in targets(&sp) {
                let dst = d.join(format!("out-{}-to.{t}", src.replace('.', "_")));
                n += 1;
                match convert(&sp, &dst) {
                    Ok(()) if std::fs::metadata(&dst).is_ok_and(|m| m.len() > 0) => {}
                    Ok(()) => failed.push(format!("{src} → {t}: leer")),
                    Err(e) => failed.push(format!("{src} → {t}: {e}")),
                }
            }
        }
        assert!(n > 150, "nur {n} Kombinationen geprüft");
        assert!(failed.is_empty(), "{} von {n} fehlgeschlagen:\n{}", failed.len(), failed.join("\n"));
        // Tabellen: Inhalt bleibt erhalten (Komma in Zelle, führende Null)
        let csv = std::fs::read_to_string(d.join("out-s_xlsx-to.csv")).unwrap();
        assert!(csv.contains("\"A, B\"") && csv.contains("01234") && csv.contains("1.5"), "{csv}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn archives_repack_with_progress() {
        let d = std::env::temp_dir().join(format!("fx-arc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src/sub")).unwrap();
        std::fs::write(d.join("src/a.txt"), "hallo ".repeat(300)).unwrap();
        std::fs::write(d.join("src/sub/b.bin"), vec![7u8; 40_000]).unwrap();
        let zip = d.join("x.zip");
        crate::ops::compress(&[d.join("src")], &zip, Format::Zip, &AtomicU64::new(0), &AtomicBool::new(false)).unwrap();
        // Ziele: alles außer dem eigenen Format; Name ohne Doppelendung
        let t = targets(&zip);
        assert!(t.contains(&"tar.xz") && t.contains(&"7z") && !t.contains(&"zip"), "{t:?}");
        assert_eq!(base_name(&d.join("a.tar.gz")), "a");
        assert_eq!(rename_kind("x.zip", "x.tar.zst", false), RenameKind::Convert);
        assert_eq!(rename_kind("x.tgz", "x.tar.gz", false), RenameKind::Plain);
        assert!(matches!(rename_kind("x.zip", "x.rar", false), RenameKind::Warn(_)));
        let mut prev = zip.clone();
        for ext in ["tar.xz", "7z", "tar.zst", "tar.bz2", "tar", "zip"] {
            let out = d.join(format!("y{}.{ext}", ext.len()));
            let prog = AtomicU64::new(0);
            convert_progress(&prev, &out, &prog, &AtomicBool::new(false)).unwrap();
            assert_eq!(prog.load(Relaxed), 1000, "{ext}");
            let dest = d.join(format!("check-{}", ext.replace('.', "-")));
            crate::extract::extract(&out, &dest, &AtomicU64::new(0), &AtomicBool::new(false)).unwrap();
            assert_eq!(std::fs::read(dest.join("src/sub/b.bin")).unwrap().len(), 40_000, "{ext}");
            assert_eq!(std::fs::read_to_string(dest.join("src/a.txt")).unwrap(), "hallo ".repeat(300), "{ext}");
            prev = out;
        }
        // Abbruch: kein halbes Ziel bleibt liegen
        let cancel = AtomicBool::new(true);
        assert!(convert_progress(&zip, &d.join("z.7z"), &AtomicU64::new(0), &cancel).is_err());
        assert!(!d.join("z.7z").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
