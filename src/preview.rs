//! Vorschau-Panel rechts neben der Dateiliste. Alles wird im Hintergrund geladen; die Oberfläche
//! blockiert nie. Unterstützt:
//! - Bilder (PNG, JPG, GIF, BMP, ICO, WebP, TIFF, SVG) mit Abmessungen
//! - Text und Quellcode mit Zeilennummern und Syntaxfarben, Markdown formatiert
//! - ZIP/TAR-Archive (Inhaltsliste)
//! - Audio/Video: integrierter Player (siehe media.rs) mit Play/Pause, Zeitleiste, Lautstärke und Vollbild;
//!   technische Daten (Format, Dauer, Auflösung, Codecs, Bitrate) und Standbild bzw. Cover
//! - alles andere (PDF, Office, Schriften …): Miniaturansicht von Windows, sonst Hex-Ansicht
//! - Ordner: Anzahl Unterordner/Dateien und die ersten Einträge

use crate::convert::{self, Class};
use crate::media::{self, Player};
use crate::fsops::ext_buf;
use crate::icons::{IconKind, Icons};
use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, FontId, Margin, Rect, Rounding, Stroke, TextureHandle, Vec2};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    sync::Arc,
};

enum Data {
    Image(egui::ColorImage, Vec<(String, String)>),
    Text(String, String, bool),
    Archive(Vec<(String, u64, bool)>),
    Folder { dirs: usize, files: usize, bytes: u64, names: Vec<(String, bool)> },
    Media(media::Info, Option<egui::ColorImage>),
    Hex(Vec<u8>),
    /// PDF: Seiten rendert `pdfview` im Hintergrund
    Pdf,
    Error(String),
}

pub struct Preview {
    target: Option<PathBuf>,
    rx: Option<Receiver<(PathBuf, Data)>>,
    /// Cloud: Eckdaten (Größe, Datum) kommen aus dem Hintergrund
    info_rx: Option<Receiver<Vec<(String, String)>>>,
    data: Option<Data>,
    tex: Option<TextureHandle>,
    info: Vec<(String, String)>,
    md_source: bool,
    /// laufende Wiedergabe der angezeigten Datei
    player: Option<Player>,
    /// Lautstärke bleibt beim Dateiwechsel erhalten
    volume: f32,
    /// Maus über dem Player → Tasten (Leertaste, Pfeile) gehören dem Player
    hot: bool,
    /// Syntaxfarben für Quelltext (im Hintergrund, Zeile für Zeile)
    hl: Option<crate::syntax::Highlight>,
    /// Wellenform der angezeigten Audiodatei
    wave: Option<media::Waveform>,
    /// das Video dieser Vorschau läuft gerade im Bild-in-Bild-Fenster
    pip_out: bool,
    /// PDF-Seiten der angezeigten Datei
    pdf: Option<crate::pdfview::PdfView>,
    /// Text- und Quelltextansicht (markieren, kopieren; im großen Preview auch bearbeiten)
    code: crate::codeedit::CodeEdit,
    /// Bearbeitung läuft (nur im großen Preview)
    edit: Option<Edit>,
    /// große Vorschau (eigener Tab): dort lässt sich Text bearbeiten
    pub big: bool,
    /// erkannter Text im angezeigten Bild (markierbar)
    ocr: Option<Ocr>,
    /// Bild- bzw. Audioeditor (nur im großen Preview)
    img_edit: Option<crate::imgedit::ImageEditor>,
    aud_edit: Option<crate::audioedit::AudioEditor>,
    vid_edit: Option<crate::videoedit::VideoEditor>,
    /// Bildzoom im großen Preview (None = einpassen) und Verschiebung der Bildmitte
    img_zoom: Option<f32>,
    /// beim nächsten Zeichnen gleich den Bildeditor öffnen (z. B. „Im Editor“ aus dem KI-Chat)
    pub want_editor: bool,
    img_pan: Vec2,
    /// kurze Rückmeldung („Kopiert“) mit Zeitpunkt; auch aus Hintergrund-Threads gesetzt
    note: Arc<std::sync::Mutex<Option<(String, std::time::Instant)>>>,
}

impl Default for Preview {
    fn default() -> Self {
        Preview { target: None, rx: None, info_rx: None, data: None, tex: None, info: Vec::new(), md_source: false, player: None, volume: 0.8, hot: false, hl: None, wave: None, pip_out: false, pdf: None, code: crate::codeedit::CodeEdit::new(""), edit: None, big: false, ocr: None, img_edit: None, aud_edit: None, vid_edit: None, img_zoom: None, want_editor: false, img_pan: Vec2::ZERO, note: Arc::default() }
    }
}

/// Bearbeitung einer Textdatei im großen Preview. Zeilenenden (CRLF) und BOM bleiben beim Speichern erhalten.
/// Texterkennung im angezeigten Bild (läuft im Hintergrund an, sobald das Bild da ist).
struct Ocr {
    rx: Option<Receiver<Result<Vec<crate::ocr::Word>, String>>>,
    words: Vec<crate::ocr::Word>,
    err: Option<String>,
    sel: crate::ocr::Selection,
}

impl Ocr {
    fn start(img: &egui::ColorImage, ctx: &egui::Context) -> Ocr {
        let (tx, rx) = mpsc::channel();
        let px: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        let (w, h) = (img.size[0] as u32, img.size[1] as u32);
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = image::RgbaImage::from_raw(w, h, px).ok_or_else(|| "Bild ungültig".to_string()).and_then(|i| crate::ocr::recognize(&image::DynamicImage::ImageRgba8(i)));
            let _ = tx.send(r);
            c.request_repaint();
        });
        Ocr { rx: Some(rx), words: Vec::new(), err: None, sel: Default::default() }
    }
}

struct Edit {
    text: String,
    /// Prüfsumme des zuletzt gespeicherten Stands (zeigt „geändert“)
    saved: u64,
    crlf: bool,
    bom: bool,
    /// Syntaxfarben; nach einer Tipppause neu berechnet
    hl: Option<crate::syntax::Highlight>,
    hl_of: u64,
    changed_at: std::time::Instant,
    err: Option<String>,
}

fn text_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Datei zum Bearbeiten ganz lesen (nur UTF-8, höchstens 8 MB).
fn open_edit(p: &Path, ctx: &egui::Context) -> Result<Edit, String> {
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Zu groß zum Bearbeiten (über 8 MB)".into());
    }
    let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let raw = std::str::from_utf8(&bytes[if bom { 3 } else { 0 }..]).map_err(|_| "Keine UTF-8-Textdatei – Bearbeiten würde sie beschädigen".to_string())?;
    let crlf = raw.contains("\r\n");
    let text = raw.replace("\r\n", "\n");
    let name = p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let h = text_hash(&text);
    Ok(Edit { hl: Some(crate::syntax::Highlight::start(Arc::from(text.as_str()), &name, ctx)), hl_of: h, saved: h, text, crlf, bom, changed_at: std::time::Instant::now(), err: None })
}

fn set_note(note: &std::sync::Mutex<Option<(String, std::time::Instant)>>, msg: &str) {
    if let Ok(mut n) = note.lock() {
        *n = Some((msg.to_string(), std::time::Instant::now()));
    }
}

/// Bild in die Zwischenablage (im Hintergrund, Rückmeldung über `note`).
fn copy_image_bg(p: PathBuf, note: Arc<std::sync::Mutex<Option<(String, std::time::Instant)>>>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let r = convert::load_image(&p, 2048).and_then(|img| crate::clip::set_image(&img));
        if let Ok(mut n) = note.lock() {
            *n = Some((if r.is_ok() { "Bild kopiert".into() } else { format!("Kopieren fehlgeschlagen: {}", r.err().unwrap_or_default()) }, std::time::Instant::now()));
        }
        ctx.request_repaint();
    });
}

/// Was der Explorer nach einer Aktion in der Vorschau tun soll.
pub enum PreviewOut {
    /// ffmpeg installieren (winget in einem Terminal-Tab)
    InstallFfmpeg,
    /// Video ins Bild-in-Bild-Fenster geben (der Explorer übernimmt den Player mit `take_player`)
    Pip,
    /// Bild-in-Bild beenden und hier weiterspielen
    PipBack,
}

fn human(n: u64) -> String {
    crate::fsops::human(n)
}

fn meta_info(p: &Path) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Ok(m) = std::fs::metadata(p) {
        if m.is_file() {
            v.push(("Größe".into(), human(m.len())));
        }
        if let Ok(t) = m.modified() {
            v.push(("Geändert".into(), chrono::DateTime::<chrono::Local>::from(t).format("%d.%m.%Y %H:%M").to_string()));
        }
    }
    let e = convert::ext_of(p);
    if !e.is_empty() {
        v.push(("Typ".into(), format!("{} (.{e})", convert::class_name(convert::class(&e)))));
    }
    v
}

fn color_image(img: &image::DynamicImage) -> egui::ColorImage {
    let img = if img.width() > 2048 || img.height() > 2048 { img.thumbnail(2048, 2048) } else { img.clone() };
    let rgba = img.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}

fn run(cmd: &str, args: &[&std::ffi::OsStr]) -> Option<Vec<u8>> {
    let mut c = std::process::Command::new(cmd);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let o = c.output().ok()?;
    o.status.success().then_some(o.stdout)
}

/// Audio/Video: Kenndaten (ffprobe bzw. eingebauter Dekoder), Standbild über ffmpeg, sonst Vorschau/Cover von Windows.
fn media(p: &Path) -> Data {
    let info = media::probe(p);
    let frame = match convert::ffmpeg() {
        Some(ff) if info.has_video => {
            let at = if info.duration > 3.0 { "1" } else { "0" };
            let head = ["-v", "error", "-ss", at, "-i"];
            let a: Vec<&std::ffi::OsStr> = head
                .iter()
                .map(std::ffi::OsStr::new)
                .chain(std::iter::once(p.as_os_str()))
                .chain(["-frames:v", "1", "-vf", "scale=640:-2", "-f", "image2pipe", "-vcodec", "png", "-"].iter().map(std::ffi::OsStr::new))
                .collect();
            run(&ff.to_string_lossy(), &a).and_then(|b| image::load_from_memory(&b).ok()).map(|i| color_image(&i)).or_else(|| thumbnail(p))
        }
        _ => thumbnail(p),
    };
    Data::Media(info, frame)
}

/// Miniaturansicht von Windows (PDF, Office, Videos, Schriften …).
fn thumbnail(p: &Path) -> Option<egui::ColorImage> {
    shell_image(p, 512)
}

/// Bild von Windows (Shell/WIC): Vorschauen, aber auch HEIC, AVIF, PDF (erste Seite) in `px` Größe.
#[cfg(windows)]
pub fn shell_image(p: &Path, px: i32) -> Option<egui::ColorImage> {
    shell_bitmap(&p.to_string_lossy(), px, false)
}

/// Programmsymbol über die Shell: Pfad einer EXE oder „shell:AppsFolder\<App-ID>“ (Store-Apps).
#[cfg(windows)]
pub fn shell_icon(name: &str, px: i32) -> Option<egui::ColorImage> {
    shell_bitmap(name, px, true)
}

#[cfg(not(windows))]
pub fn shell_icon(_name: &str, _px: i32) -> Option<egui::ColorImage> {
    None
}

#[cfg(windows)]
fn shell_bitmap(name: &str, px: i32, icon: bool) -> Option<egui::ColorImage> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY, SIIGBF_THUMBNAILONLY};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let res = (|| {
            let f: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(name), None).ok()?;
            let flags = if icon { SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK } else { SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK };
            let hb = f.GetImage(SIZE { cx: px, cy: px }, flags).ok()?;
            let mut bmp: BITMAP = std::mem::zeroed();
            let ok = GetObjectW(hb.into(), std::mem::size_of::<BITMAP>() as i32, Some((&raw mut bmp).cast())) != 0;
            let (w, h) = (bmp.bmWidth.max(0) as usize, bmp.bmHeight.unsigned_abs() as usize);
            let mut px = vec![0u8; w * h * 4];
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w as i32;
            bi.bmiHeader.biHeight = -(h as i32);
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB.0;
            let dc = CreateCompatibleDC(None);
            let lines = if ok && w > 0 && h > 0 { GetDIBits(dc, hb, 0, h as u32, Some(px.as_mut_ptr().cast()), &raw mut bi, DIB_RGB_COLORS) } else { 0 };
            let _ = DeleteDC(dc);
            let _ = DeleteObject(hb.into());
            if lines == 0 {
                return None;
            }
            let no_alpha = px.chunks_exact(4).all(|c| c[3] == 0);
            for c in px.chunks_exact_mut(4) {
                c.swap(0, 2);
                if no_alpha {
                    c[3] = 255;
                }
            }
            Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &px))
        })();
        CoUninitialize();
        res
    }
}

#[cfg(not(windows))]
pub fn shell_image(_p: &Path, _px: i32) -> Option<egui::ColorImage> {
    None
}

fn load(p: &Path) -> Data {
    let md = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(e) => return Data::Error(e.to_string()),
    };
    if md.is_dir() {
        let (mut dirs, mut files, mut bytes, mut names) = (0, 0, 0u64, Vec::new());
        match std::fs::read_dir(p) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let d = e.file_type().is_ok_and(|t| t.is_dir());
                    if d {
                        dirs += 1;
                    } else {
                        files += 1;
                        bytes += e.metadata().map_or(0, |m| m.len());
                    }
                    if names.len() < 60 {
                        names.push((e.file_name().to_string_lossy().into_owned(), d));
                    }
                }
            }
            Err(e) => return Data::Error(e.to_string()),
        }
        names.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        return Data::Folder { dirs, files, bytes, names };
    }
    let ext = convert::ext_of(p);
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Schnellansicht: Schriften, 3D-Modelle, Office-Dokumente
    match ext.as_str() {
        "ttf" | "otf" | "ttc" => {
            return match crate::quicklook::font_specimen(p) {
                Ok((img, info)) => Data::Image(img, info),
                Err(e) => Data::Error(e),
            }
        }
        "obj" | "stl" | "glb" | "gltf" if md.len() < 400 << 20 => {
            return match crate::quicklook::load_model(p, &ext) {
                Ok(t) => Data::Image(crate::quicklook::render_model(&t, 900, 680, 35.0, 22.0, [90, 150, 255]), crate::quicklook::model_info(&t)),
                Err(e) => Data::Error(e),
            }
        }
        "docx" | "odt" | "rtf" | "epub" | "xlsx" | "pptx" | "odp" => {
            if let Some(md) = crate::quicklook::office_markdown(p, &ext) {
                return Data::Text(md, "md".into(), false);
            }
        }
        _ => {}
    }
    match convert::class(&ext) {
        Class::Image => match convert::load_image(p, 1200) {
            Ok(img) => {
                let info = vec![("Abmessungen".to_string(), format!("{} × {} px", img.width(), img.height()))];
                Data::Image(color_image(&img), info)
            }
            Err(e) => thumbnail(p).map(|i| Data::Image(i, vec![])).unwrap_or(Data::Error(e)),
        },
        Class::Audio | Class::Video => media(p),
        Class::Document if ext == "pdf" && cfg!(windows) => Data::Pdf,
        Class::Archive if ext == "zip" => match std::fs::File::open(p).map_err(|e| e.to_string()).and_then(|f| zip::ZipArchive::new(f).map_err(|e| e.to_string())) {
            Ok(mut z) => Data::Archive(
                (0..z.len().min(2000)).filter_map(|i| z.by_index(i).ok().map(|f| (f.name().to_string(), f.size(), f.is_dir()))).collect(),
            ),
            Err(e) => Data::Error(e),
        },
        Class::Archive if ext == "tar" || name.ends_with(".tar.gz") || ext == "tgz" => {
            let f = match std::fs::File::open(p) {
                Ok(f) => f,
                Err(e) => return Data::Error(e.to_string()),
            };
            let r: Box<dyn std::io::Read> = if ext == "tar" { Box::new(f) } else { Box::new(flate2::read::GzDecoder::new(f)) };
            let mut a = tar::Archive::new(r);
            match a.entries() {
                Ok(es) => Data::Archive(
                    es.flatten()
                        .take(2000)
                        .filter_map(|e| Some((e.path().ok()?.to_string_lossy().into_owned(), e.header().size().unwrap_or(0), e.header().entry_type().is_dir())))
                        .collect(),
                ),
                Err(e) => Data::Error(e.to_string()),
            }
        }
        _ => {
            // Text erkennen: die ersten Bytes ohne Nullbytes und gültiges UTF-8
            let mut buf = vec![0u8; 256 * 1024];
            let n = match std::fs::File::open(p).and_then(|mut f| std::io::Read::read(&mut f, &mut buf)) {
                Ok(n) => n,
                Err(e) => return Data::Error(e.to_string()),
            };
            buf.truncate(n);
            let texty = !buf.iter().take(8192).any(|&b| b == 0);
            if texty && (crate::editors::is_text(&name) || std::str::from_utf8(&buf[..buf.len().min(8192)]).is_ok() || ext.is_empty()) {
                let truncated = md.len() > buf.len() as u64;
                // Zeilenenden vereinheitlichen (Markieren/Kopieren, Syntaxfarben je Zeile)
                return Data::Text(String::from_utf8_lossy(&buf).replace("\r\n", "\n"), ext, truncated);
            }
            match thumbnail(p) {
                Some(i) => Data::Image(i, vec![]),
                None => {
                    buf.truncate(1024);
                    Data::Hex(buf)
                }
            }
        }
    }
}

impl Preview {
    /// Neue Datei anzeigen: alles zurücksetzen und im Hintergrund laden.
    fn retarget(&mut self, target: Option<&Path>, ctx: &egui::Context) {
        if let Some(p) = self.player.take() {
            self.volume = p.volume;
        }
        self.hot = false;
        self.target = target.map(Path::to_path_buf);
        self.data = None;
        self.tex = None;
        self.hl = None;
        self.img_zoom = None;
        self.img_pan = Vec2::ZERO;
        self.wave = None;
        self.edit = None;
        self.code = crate::codeedit::CodeEdit::new(&target.map_or_else(String::new, |p| p.display().to_string()));
        self.pip_out = false;
        self.pdf = None;
        self.ocr = None;
        self.img_edit = None;
        self.aud_edit = None;
        self.vid_edit = None;
        self.info.clear();
        if let Some(p) = &self.target {
            let (tx, rx) = mpsc::channel();
            let (p2, c2) = (p.clone(), ctx.clone());
            let cloud = crate::fsx::slow(p);
            let (itx, irx) = mpsc::channel();
            std::thread::spawn(move || {
                // Cloud: auch die Eckdaten erst hier lesen – das Laufwerk kann gerade hängen
                if cloud {
                    let _ = itx.send(meta_info(&p2));
                    c2.request_repaint();
                }
                let d = load(&p2);
                let _ = tx.send((p2, d));
                c2.request_repaint();
            });
            self.rx = Some(rx);
            if cloud {
                self.info_rx = Some(irx);
            } else {
                self.info = meta_info(p);
            }
        }
    }

    /// Vorschau für die Datei eines Players aus dem Bild-in-Bild, die ihn sofort weiterspielen lässt.
    pub fn with_player(pl: Player, ctx: &egui::Context) -> Preview {
        let mut pv = Preview::default();
        pv.retarget(Some(&pl.path.clone()), ctx);
        let _ = pv.adopt(pl);
        pv
    }

    /// Inhalt neu laden lassen (z. B. nach Änderungen von außen) – nur, wenn nichts bearbeitet wird
    pub fn invalidate(&mut self) {
        if !self.unsaved() && self.player.is_none() {
            self.target = None;
        }
    }

    /// Gibt es ungespeicherte Änderungen im Bearbeitungsmodus?
    pub fn unsaved(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| text_hash(&e.text) != e.saved) || self.img_edit.as_ref().is_some_and(|e| e.modified) || self.aud_edit.as_ref().is_some_and(|e| e.modified) || self.vid_edit.as_ref().is_some_and(|e| e.modified)
    }

    /// Bild- oder Audioeditor geöffnet (übernimmt Tasten und den ganzen Bereich)
    pub fn editing_media(&self) -> bool {
        self.img_edit.is_some() || self.aud_edit.is_some() || self.vid_edit.is_some()
    }

    /// Kurze Rückmeldung („Kopiert“) unten rechts, zwei Sekunden lang.
    fn draw_note(&self, ui: &egui::Ui, rect: Rect, th: &Theme) {
        let Some((msg, at)) = self.note.lock().ok().and_then(|n| n.clone()) else { return };
        let age = at.elapsed().as_secs_f32();
        if age > 2.2 {
            return;
        }
        let g = ui.fonts(|f| f.layout_no_wrap(msg, FontId::proportional(12.5), th.text));
        let r = Rect::from_min_size(rect.right_bottom() - g.size() - vec2(34.0, 30.0), g.size() + vec2(20.0, 10.0));
        let p = ui.painter();
        p.rect(r, 6.0, th.bg, Stroke::new(1.0, th.line));
        p.galley(r.min + vec2(10.0, 5.0), g, th.text);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }

    /// Text und Quelltext: markieren/kopieren; im großen Preview bearbeiten (Vorschläge, Symbole, Strg+S).
    fn text_ui(&mut self, ui: &mut egui::Ui, th: &Theme, p: &Path) {
        use crate::ui::{cmd_btn, Ic};
        let Some(Data::Text(mut t, ext, mut truncated)) = self.data.take() else { return };
        let ctx = ui.ctx().clone();
        let md = matches!(ext.as_str(), "md" | "markdown");
        let modified = self.unsaved();
        let (mut start, mut stop, mut save, mut jump) = (false, false, false, None);
        if self.edit.is_some() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            save = true;
        }
        // nur für Bildschirmfotos (FOLDEX_SHOT): FOLDEX_SHOT_EDIT="<Zeile>:<Text>" öffnet die Bearbeitung und tippt
        static DEV_EDIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if self.big && std::env::var_os("FOLDEX_SHOT").is_some() && !DEV_EDIT.swap(true, std::sync::atomic::Ordering::Relaxed) {
            if let Some((l, s)) = std::env::var("FOLDEX_SHOT_EDIT").ok().as_deref().and_then(|v| v.split_once(':')) {
                start = true;
                self.code.dev_type = l.parse().ok().map(|l: usize| (l, s.to_string()));
            }
        }
        ui.horizontal(|ui| {
            if let Some(e) = &self.edit {
                ui.label(egui::RichText::new("Bearbeiten").size(13.0).strong());
                if modified {
                    ui.label(egui::RichText::new("● geändert").size(12.0).color(th.accent));
                }
                if let Some((l, c)) = self.code.caret {
                    ui.label(egui::RichText::new(format!("Z. {l}, Sp. {c}")).size(12.0).color(th.sub));
                }
                if let Some(err) = &e.err {
                    ui.label(egui::RichText::new(err).size(12.0).color(th.danger));
                }
            } else if md {
                ui.selectable_value(&mut self.md_source, false, "Formatiert");
                ui.selectable_value(&mut self.md_source, true, "Quelltext");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.edit.is_some() {
                    if cmd_btn(ui, th, Some(Ic::Check), "Speichern", modified, false).on_hover_text("Strg+S").clicked() {
                        save = true;
                    }
                    let close = cmd_btn(ui, th, Some(Ic::Close), if modified { "Verwerfen" } else { "Fertig" }, true, false);
                    if close.on_hover_text(if modified { "Änderungen verwerfen und zurück zur Ansicht" } else { "Zurück zur Ansicht" }).clicked() {
                        stop = true;
                    }
                    // Definitionen der Datei (Funktionen, Typen, Variablen) – Klick springt hin
                    let resp = cmd_btn(ui, th, Some(Ic::Code), "Symbole", true, true);
                    let pid = ui.id().with("pv_syms");
                    if resp.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(pid));
                    }
                    egui::popup_below_widget(ui, pid, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        ui.set_min_width(260.0);
                        let defs = self.code.definitions();
                        if defs.is_empty() {
                            ui.label(egui::RichText::new("Keine Definitionen erkannt").color(th.sub));
                        }
                        egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                            for s in defs {
                                let what = match s.kind {
                                    crate::codeedit::Kind::Function => "ƒ",
                                    crate::codeedit::Kind::Type => "T",
                                    _ => "v",
                                };
                                let line = s.line.unwrap_or(0);
                                if ui.selectable_label(false, format!("{what}  {}   · Z. {}", s.name, line + 1)).clicked() {
                                    jump = Some(line);
                                }
                            }
                        });
                    });
                } else {
                    if cmd_btn(ui, th, Some(Ic::Copy), "Kopieren", true, false).on_hover_text("Ganzen Text kopieren – oder Text markieren und Strg+C").clicked() {
                        ctx.copy_text(t.clone());
                        set_note(&self.note, "Text kopiert");
                    }
                    if self.big && cmd_btn(ui, th, Some(Ic::Rename), "Bearbeiten", true, false).on_hover_text("Text hier bearbeiten – mit Vorschlägen beim Schreiben").clicked() {
                        start = true;
                    }
                }
            });
        });
        if let Some(l) = jump {
            self.code.jump_to(l);
        }
        // Platz für den Hinweis unten freihalten, damit er nicht aus dem Bereich rutscht
        let note_h = if truncated && self.edit.is_none() { 20.0 } else { 0.0 };
        let size = vec2(ui.available_width(), (ui.available_height() - note_h).max(40.0));
        if let Some(e) = &mut self.edit {
            if self.code.show(ui, th, &mut e.text, true, &ext, e.hl.as_ref(), size) {
                e.changed_at = std::time::Instant::now();
                e.err = None;
            }
            // Syntaxfarben nach einer kurzen Tipppause neu berechnen
            let h = text_hash(&e.text);
            if h != e.hl_of {
                if e.changed_at.elapsed() > std::time::Duration::from_millis(600) {
                    let name = p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    e.hl = Some(crate::syntax::Highlight::start(Arc::from(e.text.as_str()), &name, &ctx));
                    e.hl_of = h;
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(650));
                }
            }
        } else if md && !self.md_source {
            egui::ScrollArea::vertical().id_salt("pv_md").auto_shrink([false, false]).max_height(size.y).show(ui, |ui| markdown(ui, th, &t));
        } else {
            let mut s: &str = &t;
            self.code.show(ui, th, &mut s, false, &ext, self.hl.as_ref(), size);
        }
        if truncated && self.edit.is_none() {
            ui.label(egui::RichText::new("Nur die ersten 256 KB werden angezeigt.").size(11.5).color(th.sub));
        }
        if start {
            match open_edit(p, &ctx) {
                Ok(e) => self.edit = Some(e),
                Err(err) => set_note(&self.note, &err),
            }
        }
        if save {
            if let Some(e) = &mut self.edit {
                let body = if e.crlf { e.text.replace('\n', "\r\n") } else { e.text.clone() };
                let mut bytes = if e.bom { vec![0xEF, 0xBB, 0xBF] } else { Vec::new() };
                bytes.extend_from_slice(body.as_bytes());
                // Cloud: im Hintergrund speichern (das Laufwerk kann gerade hängen), Fehler kommen als Hinweis
                let r = if crate::fsx::slow(p) {
                    let (p2, note) = (p.to_path_buf(), self.note.clone());
                    std::thread::spawn(move || {
                        if let Err(e) = crate::themes::write_atomic(&p2, &bytes) {
                            set_note(&note, &format!("Speichern fehlgeschlagen: {e}"));
                        }
                    });
                    Ok(())
                } else {
                    crate::themes::write_atomic(p, &bytes)
                };
                match r {
                    Ok(()) => {
                        e.saved = text_hash(&e.text);
                        // die Ansicht zeigt ab jetzt den gespeicherten Stand
                        t = e.text.clone();
                        truncated = false;
                        let name = p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        self.hl = Some(crate::syntax::Highlight::start(Arc::from(t.as_str()), &name, &ctx));
                        if !crate::fsx::slow(p) {
                            self.info = meta_info(p);
                        }
                        set_note(&self.note, "Gespeichert");
                    }
                    Err(err) => e.err = Some(format!("Speichern fehlgeschlagen: {err}")),
                }
            }
        }
        if stop {
            self.edit = None;
        }
        self.data = Some(Data::Text(t, ext, truncated));
    }

    /// Player für das Bild-in-Bild-Fenster herausgeben; die Vorschau zeigt solange einen Hinweis.
    pub fn take_player(&mut self) -> Option<Player> {
        let mut pl = self.player.take()?;
        self.volume = pl.volume;
        self.pip_out = true;
        self.hot = false;
        pl.pip = true;
        pl.pip_request = false;
        Some(pl)
    }

    /// Player aus dem Bild-in-Bild zurücknehmen – nur, wenn diese Vorschau dieselbe Datei zeigt
    /// (dann läuft die Wiedergabe hier ohne Unterbrechung weiter); sonst kommt er zurück.
    pub fn adopt(&mut self, mut pl: Player) -> Result<(), Player> {
        if self.target.as_deref() != Some(pl.path.as_path()) {
            return Err(pl);
        }
        pl.pip = false;
        pl.pip_request = false;
        // gilt ab jetzt als in dieser Vorschau gezeigt (sonst hielte `tick` ihn sofort an)
        pl.touch();
        self.pip_out = false;
        self.player = Some(pl);
        Ok(())
    }

    /// Gehören die Tasten gerade dem Player (Vollbild oder Maus über dem Player)?
    pub fn wants_keys(&self) -> bool {
        self.editing_media() || self.player.as_ref().is_some_and(|p| p.fullscreen) || (self.hot && self.player.is_some())
    }

    /// Nach dem Zeichnen jedes Frames: Wurde die Vorschau nicht gezeigt (ausgeblendet, anderer Tab), pausiert
    /// die Wiedergabe und das Vollbild endet – sonst liefe der Ton unsichtbar weiter.
    pub fn tick(&mut self, ctx: &egui::Context, th: &Theme) {
        if let Some(e) = &mut self.vid_edit {
            e.tick(ctx);
        }
        if let Some(e) = &mut self.aud_edit {
            e.tick(ctx);
        }
        if let Some(p) = &mut self.player {
            if p.last_frame + 1 < ctx.cumulative_pass_nr() {
                p.pause();
                p.fullscreen = false;
                p.fullscreen_ui(ctx, th);
            }
        }
    }

    /// Vorschau für `target` anzeigen (lädt bei Wechsel neu). `multi` = (Anzahl, Größe) bei Mehrfachauswahl.
    /// `icons`: dieselben Symbole wie in der Dateiliste (Ordner- und Dateitypen einheitlich).
    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme, icons: &mut Icons, target: Option<&Path>, multi: Option<(usize, u64)>) -> Option<PreviewOut> {
        let ctx = ui.ctx().clone();
        let mut out = None;
        if target.map(Path::to_path_buf) != self.target {
            self.retarget(target, &ctx);
        }
        if let Some(Ok(v)) = self.info_rx.as_ref().map(|r| r.try_recv()) {
            // vorne einsortieren; was die Vorschau schon ergänzt hat, bleibt dahinter
            let extra = std::mem::take(&mut self.info);
            self.info = v;
            for x in extra {
                if !self.info.contains(&x) {
                    self.info.push(x);
                }
            }
            self.info_rx = None;
        }
        if let Some(rx) = &self.rx {
            if let Ok((p, d)) = rx.try_recv() {
                if Some(&p) == self.target.as_ref() {
                    match &d {
                        Data::Image(img, extra) => {
                            // kleine Bilder (Symbole, Pixelgrafik) scharf vergrößern; große mit Mipmaps glatt verkleinern
                            let small = img.size[0].max(img.size[1]) <= 256;
                            let opts = egui::TextureOptions {
                                magnification: if small { egui::TextureFilter::Nearest } else { egui::TextureFilter::Linear },
                                mipmap_mode: Some(egui::TextureFilter::Linear),
                                ..egui::TextureOptions::LINEAR
                            };
                            self.tex = Some(ctx.load_texture("preview", img.clone(), opts));
                            // Text im Bild erkennen (winzige Symbole lohnen nicht)
                            if img.size[0].max(img.size[1]) >= 48 {
                                self.ocr = Some(Ocr::start(img, &ctx));
                            }
                            for x in extra {
                                if !self.info.contains(x) {
                                    self.info.insert(1.min(self.info.len()), x.clone());
                                }
                            }
                        }
                        Data::Pdf => self.pdf = Some(crate::pdfview::PdfView::open(&p, &ctx)),
                        Data::Media(info, frame) => {
                            if let Some(f) = frame {
                                self.tex = Some(ctx.load_texture("preview", f.clone(), egui::TextureOptions::LINEAR));
                            }
                            self.info.extend(info.rows.iter().cloned());
                            if info.has_audio && !info.has_video {
                                self.wave = Some(media::Waveform::start(&p, &ctx));
                            }
                        }
                        Data::Text(t, ext, _) if !matches!(ext.as_str(), "md" | "markdown") && !plain_text(ext) => {
                            // Syntaxfarben im Hintergrund berechnen; der Name bestimmt die Sprache
                            let name = p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                            self.hl = Some(crate::syntax::Highlight::start(Arc::from(t.as_str()), &name, &ctx));
                        }
                        _ => {}
                    }
                    self.data = Some(d);
                }
                self.rx = None;
            }
        }

        let rect = ui.available_rect_before_wrap();
        // nur für Bildschirmfotos (FOLDEX_SHOT): FOLDEX_SHOT_EDITOR=1 öffnet den Bild- bzw. Audioeditor mit Beispielen
        if self.big && !self.editing_media() && std::env::var_os("FOLDEX_SHOT").is_some() && std::env::var_os("FOLDEX_SHOT_EDITOR").is_some() {
            static DEV: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            match (&self.data, &self.target) {
                (Some(Data::Image(..)), Some(p)) if !DEV.swap(true, std::sync::atomic::Ordering::Relaxed) => {
                    if let Ok(mut e) = crate::imgedit::ImageEditor::open(p) {
                        e.dev_demo();
                        self.img_edit = Some(e);
                    }
                }
                (Some(Data::Media(info, _)), Some(p)) if !info.has_video && !DEV.swap(true, std::sync::atomic::Ordering::Relaxed) => {
                    if let Ok(mut e) = crate::audioedit::AudioEditor::open(p, info, &ctx, self.volume) {
                        e.dev_demo();
                        self.aud_edit = Some(e);
                    }
                }
                (Some(Data::Media(info, _)), Some(p)) if info.has_video && !DEV.swap(true, std::sync::atomic::Ordering::Relaxed) => {
                    if let Ok(mut e) = crate::videoedit::VideoEditor::open(p, info, &ctx, self.volume, self.tex.clone()) {
                        e.dev_demo();
                        self.vid_edit = Some(e);
                    }
                }
                _ => {}
            }
        }
        // Bild-/Audio-/Videoeditor nutzt den ganzen Bereich (nur im großen Preview)
        if self.big && self.editing_media() {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            // (neu laden?, Bildeditor danach offen lassen?)
            let (reload, keep_img) = if let Some(e) = &mut self.img_edit {
                match e.ui(ui, th, rect) {
                    Some(crate::imgedit::EditOut::Close) => {
                        self.img_edit = None;
                        (false, false)
                    }
                    Some(crate::imgedit::EditOut::Saved(p)) => (Some(p) == self.target, true),
                    None => (false, true),
                }
            } else if let Some(e) = &mut self.aud_edit {
                match e.ui(ui, th, rect) {
                    Some(crate::audioedit::AudioOut::Close) => {
                        self.aud_edit = None;
                        (false, false)
                    }
                    // Datei überschrieben: Editor zu, Vorschau zeigt den neuen Stand
                    Some(crate::audioedit::AudioOut::Saved(p)) => {
                        self.aud_edit = None;
                        (Some(p) == self.target, false)
                    }
                    None => (false, false),
                }
            } else if let Some(e) = &mut self.vid_edit {
                match e.ui(ui, th, rect) {
                    Some(crate::videoedit::VideoOut::Close) => {
                        self.vid_edit = None;
                        (false, false)
                    }
                    Some(crate::videoedit::VideoOut::Saved(p)) => {
                        self.vid_edit = None;
                        (Some(p) == self.target, false)
                    }
                    None => (false, false),
                }
            } else {
                (false, false)
            };
            if reload {
                // Vorschau neu laden; ein Bildeditor bleibt dabei offen
                let keep = if keep_img { self.img_edit.take() } else { None };
                let t = self.target.clone();
                self.retarget(t.as_deref(), &ctx);
                self.img_edit = keep;
                set_note(&self.note, "Gespeichert");
            }
            return out;
        }
        ui.painter().rect_filled(rect, 0.0, th.panel);
        self.hot = false;
        // nichts zeichnet über den Vorschaubereich hinaus
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::Frame::none().inner_margin(Margin::same(14.0)).show(ui, |ui| {
            ui.set_width((rect.width() - 28.0).max(1.0));
            ui.set_max_height((rect.height() - 28.0).max(1.0));
            // Name, Kenndaten, formatiertes Markdown usw. lassen sich markieren und kopieren
            ui.style_mut().interaction.selectable_labels = true;
            if let Some((n, bytes)) = multi {
                ui.label(egui::RichText::new(format!("{n} Elemente ausgewählt")).size(16.0).strong());
                if bytes > 0 {
                    ui.label(egui::RichText::new(format!("Dateien zusammen {}", human(bytes))).color(th.sub));
                }
                return;
            }
            let Some(p) = self.target.clone() else {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("Keine Auswahl").size(15.0).color(th.sub));
                    ui.label(egui::RichText::new("Eine Datei auswählen, um sie hier anzusehen.").size(12.5).color(th.sub));
                });
                return;
            };
            let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
            ui.horizontal(|ui| {
                let is_dir = crate::fsx::is_dir(&p);
                let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::hover());
                icons.paint(ui.painter(), r, if is_dir { IconKind::Dir } else { IconKind::File }, &name, ext_buf(&name).as_str(), false, || p.clone());
                ui.add(egui::Label::new(egui::RichText::new(&name).size(15.0).strong()).wrap());
            });
            ui.add_space(4.0);
            egui::Grid::new("pv_info").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
                for (k, v) in &self.info {
                    ui.label(egui::RichText::new(k).size(12.0).color(th.sub));
                    ui.add(egui::Label::new(egui::RichText::new(v).size(12.0)).wrap());
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            let avail = ui.available_size();
            match &self.data {
                None => {
                    ui.spinner();
                }
                Some(Data::Error(e)) => {
                    ui.label(egui::RichText::new(e).color(th.danger));
                }
                Some(Data::Media(info, _)) => {
                    let info = info.clone();
                    if let Some(o) = self.media_ui(ui, th, &p, &info, avail) {
                        out = Some(o);
                    }
                }
                Some(Data::Image(..)) => {
                    // Textebene: erkannter Text lässt sich direkt im Bild markieren und kopieren
                    if let Some(o) = &mut self.ocr {
                        if let Some(r) = o.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
                            o.rx = None;
                            match r {
                                Ok(w) => o.words = w,
                                Err(e) => o.err = Some(e),
                            }
                            // nur für Bildschirmfotos: FOLDEX_SHOT_SELECT=1 markiert und kopiert den erkannten Text
                            if std::env::var_os("FOLDEX_SHOT").is_some() && std::env::var_os("FOLDEX_SHOT_SELECT").is_some() {
                                o.sel = crate::ocr::Selection::all(o.words.len());
                                ui.ctx().copy_text(crate::ocr::all_text(&o.words));
                            }
                        }
                    }
                    let words_n = self.ocr.as_ref().map_or(0, |o| o.words.len());
                    // Zoom-Wunsch aus den Knöpfen: Faktor, 0 = einpassen
                    let mut zoom_req: Option<f32> = None;
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Bildeditor nur im großen Preview (eigener Tab)
                            if self.big && std::mem::take(&mut self.want_editor) {
                                match crate::imgedit::ImageEditor::open(&p) {
                                    Ok(e) => self.img_edit = Some(e),
                                    Err(e) => set_note(&self.note, &e),
                                }
                            }
                            if self.big && crate::ui::cmd_btn(ui, th, Some(crate::ui::Ic::Rename), "Bearbeiten", true, false).on_hover_text("Bildeditor: Werkzeuge, Anpassungen, Filter").clicked() {
                                match crate::imgedit::ImageEditor::open(&p) {
                                    Ok(e) => self.img_edit = Some(e),
                                    Err(e) => set_note(&self.note, &e),
                                }
                            }
                            if crate::ui::cmd_btn(ui, th, Some(crate::ui::Ic::Copy), "Bild kopieren", true, false).on_hover_text("Bild in die Zwischenablage").clicked() {
                                set_note(&self.note, "Bild wird kopiert …");
                                copy_image_bg(p.clone(), self.note.clone(), ui.ctx().clone());
                            }
                            if words_n > 0 && crate::ui::cmd_btn(ui, th, Some(crate::ui::Ic::Docs), "Text kopieren", true, false).on_hover_text("Gesamten erkannten Text kopieren").clicked() {
                                if let Some(o) = &self.ocr {
                                    ui.ctx().copy_text(crate::ocr::all_text(&o.words));
                                    set_note(&self.note, "Text kopiert");
                                }
                            }
                            if self.big {
                                ui.add_space(8.0);
                                if ui.add_enabled(self.img_zoom.is_some(), egui::Button::new("Einpassen")).on_hover_text("Ganzes Bild zeigen (Doppelklick ins Bild)").clicked() {
                                    zoom_req = Some(0.0);
                                }
                                if ui.button("+").on_hover_text("Vergrößern (Mausrad)").clicked() {
                                    zoom_req = Some(1.25);
                                }
                                ui.label(egui::RichText::new(self.img_zoom.map_or_else(|| "Eingepasst".to_string(), |z| format!("{:.0} %", z * 100.0))).size(12.0).color(th.sub));
                                if ui.add_enabled(self.img_zoom.is_some(), egui::Button::new("−")).on_hover_text("Verkleinern (Mausrad)").clicked() {
                                    zoom_req = Some(0.8);
                                }
                            }
                        });
                    });
                    let avail = ui.available_size();
                    if let Some(t) = &self.tex {
                        // eingepasst: so groß wie der freie Platz erlaubt (auch vergrößert), Seitenverhältnis bleibt
                        let s = t.size_vec2().max(vec2(1.0, 1.0));
                        let fit = (avail.x / s.x).min(avail.y / s.y).max(0.0);
                        let k = if self.big { self.img_zoom.unwrap_or(fit) } else { fit };
                        let size = s * k;
                        let tex = t.id();
                        let (ocr, note) = (&mut self.ocr, &self.note);
                        let mut draw = |ui: &mut egui::Ui, r: Rect| {
                            checker(ui, r, th);
                            ui.painter().image(tex, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            if let Some(o) = ocr.as_mut() {
                                if let Some(m) = crate::ocr::text_layer(ui, th, ui.id().with("pv_ocr"), r, &o.words, &mut o.sel) {
                                    set_note(note, &m);
                                }
                            }
                        };
                        if !self.big {
                            // kleine Vorschau: eingepasst, oben ausgerichtet
                            let (area, _) = ui.allocate_exact_size(vec2(avail.x, size.y), egui::Sense::hover());
                            let r = ui.painter().round_rect_to_pixels(Rect::from_min_size(pos2(area.center().x - size.x / 2.0, area.top()), size));
                            draw(ui, r);
                        } else {
                            // großes Preview: nur zoomen – Mausrad rein/raus am Zeiger, vergrößert mit der Maus ziehen
                            let (view, _) = ui.allocate_exact_size(avail.max(vec2(1.0, 1.0)), egui::Sense::hover());
                            // Verschiebung so begrenzen, dass kein leerer Rand ins Bild wandert
                            let clamp = |pan: Vec2, size: Vec2| {
                                let lim = ((size - view.size()) / 2.0).max(Vec2::ZERO);
                                vec2(pan.x.clamp(-lim.x, lim.x), pan.y.clamp(-lim.y, lim.y))
                            };
                            let pan = clamp(self.img_pan, size);
                            let r = ui.painter().round_rect_to_pixels(Rect::from_center_size(view.center() + pan, size));
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(view));
                            child.set_clip_rect(view.intersect(ui.clip_rect()));
                            draw(&mut child, r);
                            let zoomed = self.img_zoom.is_some();
                            // vergrößert: Ziehen verschiebt (liegt über der Textebene)
                            let drag = ui.interact(view, ui.id().with("pv_img_pan"), if zoomed { egui::Sense::click_and_drag() } else { egui::Sense::hover() });
                            if zoomed {
                                if drag.dragged() {
                                    self.img_pan = clamp(pan + drag.drag_delta(), size);
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                } else if drag.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                                }
                            }
                            let hover = ui.input(|i| i.pointer.hover_pos()).filter(|h| view.contains(*h));
                            let mut f = 1.0;
                            if hover.is_some() {
                                let wheel = ui.input(|i| i.raw_scroll_delta.y);
                                f = ui.input(|i| i.zoom_delta()) * if wheel > 0.0 { 1.2 } else if wheel < 0.0 { 1.0 / 1.2 } else { 1.0 };
                            }
                            let dbl = hover.is_some() && ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
                            let at = hover.unwrap_or(view.center());
                            let new_k = match zoom_req {
                                Some(z) if z == 0.0 => Some(fit),
                                Some(z) => Some(k * z),
                                None if dbl => Some(if zoomed { fit } else { 1.0f32.max(fit * 2.0) }),
                                None if (f - 1.0).abs() > 1e-3 => Some(k * f),
                                None => None,
                            };
                            if let Some(nk) = new_k {
                                let nk = nk.min(16.0);
                                if nk <= fit + 1e-4 {
                                    self.img_zoom = None;
                                    self.img_pan = Vec2::ZERO;
                                } else {
                                    // Punkt unter dem Zeiger bleibt stehen
                                    let u = (at - r.min) / k;
                                    let nsize = s * nk;
                                    let center = at - u * nk + nsize / 2.0;
                                    self.img_pan = clamp(center - view.center(), nsize);
                                    self.img_zoom = Some(nk);
                                }
                                ui.ctx().request_repaint();
                            }
                        }
                    }
                }
                Some(Data::Text(..)) => self.text_ui(ui, th, &p),
                Some(Data::Archive(v)) => {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{} Einträge", v.len())).size(12.5).color(th.sub));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if crate::ui::cmd_btn(ui, th, Some(crate::ui::Ic::Copy), "Liste kopieren", true, false).clicked() {
                                ui.ctx().copy_text(v.iter().map(|e| e.0.as_str()).collect::<Vec<_>>().join("\n"));
                                set_note(&self.note, "Liste kopiert");
                            }
                        });
                    });
                    egui::ScrollArea::vertical().id_salt("pv_arc").auto_shrink([false, false]).show_rows(ui, 20.0, v.len(), |ui, range| {
                        for (n, s, d) in &v[range] {
                            let short = n.trim_end_matches('/').rsplit('/').next().unwrap_or(n);
                            entry_row(ui, th, icons, short, n, *d, (!d).then(|| human(*s)), || PathBuf::new());
                        }
                    });
                }
                Some(Data::Folder { dirs, files, bytes, names }) => {
                    ui.horizontal(|ui| {
                        // schmale Vorschau: nur das Symbol, sonst überdeckt der Knopf den Text
                        let narrow = ui.available_width() < 340.0;
                        ui.label(format!("{dirs} Ordner, {files} Dateien"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if crate::ui::cmd_btn(ui, th, Some(crate::ui::Ic::Copy), if narrow { "" } else { "Namen kopieren" }, true, false).on_hover_text("Namen der angezeigten Einträge kopieren").clicked() {
                                ui.ctx().copy_text(names.iter().map(|n| n.0.as_str()).collect::<Vec<_>>().join("\n"));
                                set_note(&self.note, "Namen kopiert");
                            }
                        });
                    });
                    if *bytes > 0 {
                        ui.label(egui::RichText::new(format!("Dateien direkt darin: {}", human(*bytes))).size(12.5).color(th.sub));
                    }
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().id_salt("pv_dir").auto_shrink([false, false]).show_rows(ui, 22.0, names.len(), |ui, range| {
                        for (n, d) in &names[range] {
                            entry_row(ui, th, icons, n, n, *d, None, || p.join(n));
                        }
                    });
                }
                Some(Data::Pdf) => {
                    if let Some(v) = &mut self.pdf {
                        v.ui(ui, th, avail);
                        if !v.pages.is_empty() && !self.info.iter().any(|(k, _)| k == "Seiten") {
                            self.info.insert(1.min(self.info.len()), ("Seiten".into(), v.pages.len().to_string()));
                        }
                    }
                }
                Some(Data::Hex(b)) => {
                    ui.label(egui::RichText::new("Keine Vorschau verfügbar – die ersten Bytes:").size(12.5).color(th.sub));
                    let mut s = String::new();
                    for (i, chunk) in b.chunks(8).enumerate() {
                        let hex: Vec<String> = chunk.iter().map(|x| format!("{x:02x}")).collect();
                        let asc: String = chunk.iter().map(|&x| if (32..127).contains(&x) { x as char } else { '.' }).collect();
                        s.push_str(&format!("{:04x}  {:<24} {asc}\n", i * 8, hex.join(" ")));
                    }
                    if crate::ui::cmd_btn(ui, th, Some(crate::ui::Ic::Copy), "Kopieren", true, false).clicked() {
                        ui.ctx().copy_text(s.clone());
                        set_note(&self.note, "Kopiert");
                    }
                    egui::ScrollArea::vertical().id_salt("pv_hex").auto_shrink([false, false]).show(ui, |ui| {
                        ui.label(egui::RichText::new(&s).monospace().size(11.5).color(th.sub));
                    });
                }
            }
        });
        self.draw_note(ui, rect, th);
        if let Some(pl) = &mut self.player {
            pl.fullscreen_ui(&ctx, th);
        }
        out
    }

    /// Audio/Video: Bild bzw. Cover und Wellenform, darunter immer die Bedienleiste des Players. Der Player
    /// steht sofort bereit (angehalten); abgespielt wird über die Leiste oder einen Klick ins Bild.
    fn media_ui(&mut self, ui: &mut egui::Ui, th: &Theme, p: &Path, info: &media::Info, avail: egui::Vec2) -> Option<PreviewOut> {
        const BAR: f32 = 36.0;
        let w = avail.x;
        if self.pip_out && self.player.is_none() {
            // läuft im Bild-in-Bild-Fenster: Standbild und „zurückholen“
            let mut out = None;
            if let Some(t) = &self.tex {
                let s = t.size_vec2().max(vec2(1.0, 1.0));
                let h = (w * s.y / s.x).min((avail.y - 70.0).max(40.0));
                let (r, resp) = ui.allocate_exact_size(vec2(w, h), egui::Sense::click());
                ui.painter().rect_filled(r, 6.0, Color32::BLACK);
                let k = (r.width() / s.x).min(r.height() / s.y);
                ui.painter().image(t.id(), Rect::from_center_size(r.center(), s * k), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_gray(90));
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Läuft im Bild-in-Bild-Fenster", FontId::proportional(13.0), Color32::from_gray(225));
                if resp.on_hover_text("Hier weiterspielen").clicked() {
                    out = Some(PreviewOut::PipBack);
                }
                ui.add_space(8.0);
            }
            if ui.button("Hier weiterspielen").clicked() {
                out = Some(PreviewOut::PipBack);
            }
            return out;
        }
        if self.player.is_none() {
            if let Err(why) = media::playable(p, info) {
                // nicht abspielbar: Standbild bzw. Cover und der Grund
                let mut out = None;
                if let Some(t) = &self.tex {
                    fit(ui, t, w, (avail.y - 70.0).max(40.0));
                    ui.add_space(8.0);
                }
                ui.add(egui::Label::new(egui::RichText::new(why).size(12.5).color(th.sub)).wrap());
                if cfg!(windows) && ui.button("ffmpeg installieren …").on_hover_text("Öffnet ein Terminal mit „winget install Gyan.FFmpeg“").clicked() {
                    out = Some(PreviewOut::InstallFfmpeg);
                }
                return out;
            }
            let mut pl = Player::new(p.to_path_buf(), info.clone(), ui.ctx(), self.volume);
            pl.poster = self.tex.clone().filter(|_| info.has_video);
            // nur für Bildschirmfotos (FOLDEX_SHOT): FOLDEX_SHOT_PLAYER="<Sekunden>[,menu]"
            if let Some(v) = std::env::var("FOLDEX_SHOT_PLAYER").ok().filter(|_| std::env::var("FOLDEX_SHOT").is_ok()) {
                pl.dev_shot(&v);
            }
            self.player = Some(pl);
        }
        let pl = self.player.as_mut()?;
        let top = ui.cursor().min;
        // Platz über der Leiste (Fehlermeldung des Players steht unter der Leiste)
        // Platz für den „Bearbeiten“-Knopf im großen Preview freihalten
        let edit_h = if self.big { 44.0 } else { 0.0 };
        let room = (avail.y - BAR - 8.0 - edit_h - if pl.err.is_some() { 18.0 } else { 0.0 }).max(48.0);
        if pl.has_video() {
            let h = (w / pl.aspect()).min(room);
            let (r, _) = ui.allocate_exact_size(vec2(w, h), egui::Sense::hover());
            if pl.fullscreen {
                ui.painter().rect_filled(r, 6.0, Color32::BLACK);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Wiedergabe im Vollbild – Esc beendet", FontId::proportional(13.0), Color32::from_gray(200));
            } else {
                pl.video(ui, r);
            }
        } else {
            // Cover (falls vorhanden) oben, die Wellenform darunter
            let mut wave_h = if self.tex.is_some() { (room * 0.3).clamp(56.0, 110.0).min(room) } else { room.min(180.0) };
            if let Some(t) = &self.tex {
                let cover_h = room - wave_h - 8.0;
                if cover_h >= 48.0 {
                    fit(ui, t, w, cover_h);
                    ui.add_space(8.0);
                } else {
                    wave_h = room.min(180.0);
                }
            }
            let (r, _) = ui.allocate_exact_size(vec2(w, wave_h), egui::Sense::hover());
            match &self.wave {
                Some(wv) => pl.waveform(ui, th, r, wv),
                None => {
                    ui.painter().rect_filled(r, 8.0, th.field);
                }
            }
        }
        ui.add_space(8.0);
        let (cr, _) = ui.allocate_exact_size(vec2(w, BAR), egui::Sense::hover());
        ui.painter().rect_filled(cr, 8.0, th.field);
        pl.controls(ui, th, cr.shrink2(vec2(6.0, 2.0)), false);
        let area = Rect::from_min_max(top, pos2(top.x + w, cr.bottom()));
        self.hot = ui.rect_contains_pointer(area);
        if self.hot && !pl.fullscreen {
            pl.keys(ui);
        }
        // Audio- bzw. Videoeditor nur im großen Preview (eigener Tab)
        if self.big {
            ui.add_space(8.0);
            let tip = if pl.has_video() { "Videoeditor: zuschneiden, ausschneiden, Ton, Tempo, Drehen, Auflösung, Format" } else { "Audioeditor: schneiden, ein-/ausblenden, Lautstärke, Normalisieren, Format" };
            if crate::ui::cmd_btn(ui, th, Some(crate::ui::Ic::Rename), "Bearbeiten", true, false).on_hover_text(tip).clicked() {
                pl.pause();
                let vol = pl.volume;
                if pl.has_video() {
                    match crate::videoedit::VideoEditor::open(p, info, ui.ctx(), vol, self.tex.clone()) {
                        Ok(e) => self.vid_edit = Some(e),
                        Err(e) => set_note(&self.note, &e),
                    }
                } else {
                    match crate::audioedit::AudioEditor::open(p, info, ui.ctx(), vol) {
                        Ok(e) => self.aud_edit = Some(e),
                        Err(e) => set_note(&self.note, &e),
                    }
                }
            }
        }
        if std::mem::take(&mut pl.pip_request) {
            return Some(PreviewOut::Pip);
        }
        None
    }
}

/// Bild (Cover, Standbild) in voller Breite `w` und höchstens `max_h` Höhe einpassen: Seitenverhältnis bleibt,
/// waagerecht zentriert.
fn fit(ui: &mut egui::Ui, t: &TextureHandle, w: f32, max_h: f32) {
    let s = t.size_vec2().max(vec2(1.0, 1.0));
    let size = s * (w / s.x).min(max_h / s.y).max(0.0);
    let (area, _) = ui.allocate_exact_size(vec2(w, size.y), egui::Sense::hover());
    let r = Rect::from_min_size(pos2(area.center().x - size.x / 2.0, area.top()), size);
    ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

/// Zeile einer Inhaltsliste (Ordner, Archiv): Symbol wie in der Dateiliste, Name, rechts optional Größe.
#[allow(clippy::too_many_arguments)]
fn entry_row(ui: &mut egui::Ui, th: &Theme, icons: &mut Icons, name: &str, tip: &str, dir: bool, right: Option<String>, path: impl FnOnce() -> PathBuf) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), egui::Sense::hover());
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, th.hover);
    }
    let ir = Rect::from_min_size(pos2(r.left() + 4.0, r.center().y - 8.0), vec2(16.0, 16.0));
    icons.paint(ui.painter(), ir, if dir { IconKind::Dir } else { IconKind::File }, name, ext_buf(name).as_str(), false, path);
    let rw = right.as_ref().map_or(0.0, |s| ui.fonts(|f| f.layout_no_wrap(s.clone(), FontId::proportional(11.5), th.sub).size().x) + 10.0);
    let text = Rect::from_min_max(pos2(ir.right() + 8.0, r.top()), pos2(r.right() - rw, r.bottom()));
    ui.painter().with_clip_rect(text).text(pos2(text.left(), r.center().y), egui::Align2::LEFT_CENTER, name, FontId::proportional(12.5), if dir { th.text } else { th.sub });
    if let Some(s) = right {
        ui.painter().text(pos2(r.right() - 4.0, r.center().y), egui::Align2::RIGHT_CENTER, s, FontId::proportional(11.5), th.sub);
    }
    if tip != name {
        resp.on_hover_text(tip);
    }
}

// ───────────────────────────── Text ─────────────────────────────

pub(crate) const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait", "match", "if", "else", "for", "while", "loop", "return", "break", "continue",
    "const", "static", "async", "await", "move", "where", "type", "self", "Self", "true", "false", "None", "Some", "Ok", "Err", "def", "class", "import", "from",
    "as", "in", "is", "not", "and", "or", "pass", "with", "yield", "lambda", "function", "var", "new", "this", "export", "default", "interface", "extends",
    "implements", "public", "private", "protected", "void", "int", "float", "double", "bool", "string", "null", "undefined", "package", "func", "go", "defer",
    "switch", "case", "try", "catch", "finally", "throw", "throws", "elif", "except", "raise", "nil", "end", "then", "local", "echo", "do", "done", "fi",
];

/// Sehr einfache, sprachübergreifende Hervorhebung: Kommentare, Zeichenketten, Zahlen, Schlüsselwörter,
/// Funktionsaufrufe und großgeschriebene Typen.
/// reiner Text: keine Farben (sonst würden Wörter, Zahlen und Anführungszeichen wie Code eingefärbt)
pub(crate) fn plain_text(ext: &str) -> bool {
    matches!(ext, "txt" | "text" | "log" | "nfo" | "out" | "me" | "")
}

pub(crate) fn highlight(line: &str, ext: &str, th: &Theme, font: &FontId) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let fmt = |c: Color32| egui::TextFormat::simple(font.clone(), c);
    if plain_text(ext) {
        job.append(line, 0.0, fmt(th.ed_text));
        return job;
    }
    let hash_comment = matches!(ext, "py" | "sh" | "bash" | "toml" | "yaml" | "yml" | "rb" | "ps1" | "conf" | "ini" | "env" | "gitignore" | "r");
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let rest: String = chars[i..].iter().collect();
        if rest.starts_with("//") || (hash_comment && c == '#') || rest.starts_with("--") && matches!(ext, "sql" | "lua") {
            job.append(&rest, 0.0, fmt(th.comment));
            break;
        }
        if c == '"' || c == '\'' || c == '`' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            job.append(&chars[i..end].iter().collect::<String>(), 0.0, fmt(th.string));
            i = end;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.' || chars[j] == '_') {
                j += 1;
            }
            job.append(&chars[i..j].iter().collect::<String>(), 0.0, fmt(th.number));
            i = j;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let w: String = chars[i..j].iter().collect();
            let col = if KEYWORDS.contains(&w.as_str()) {
                th.keyword
            } else if chars.get(j) == Some(&'(') {
                th.function
            } else if w.chars().next().is_some_and(|f| f.is_uppercase()) {
                th.ty
            } else {
                th.ed_text
            };
            job.append(&w, 0.0, fmt(col));
            i = j;
            continue;
        }
        let mut j = i;
        while j < chars.len() && !(chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '"' || chars[j] == '\'' || chars[j] == '`' || chars[j] == '/' || chars[j] == '#' || chars[j] == '-') {
            j += 1;
        }
        let j = j.max(i + 1);
        job.append(&chars[i..j].iter().collect::<String>(), 0.0, fmt(th.ed_text));
        i = j;
    }
    job
}

fn markdown(ui: &mut egui::Ui, th: &Theme, text: &str) {
    ui.spacing_mut().item_spacing.y = 4.0;
    for (i, part) in text.split("```").enumerate() {
        if i % 2 == 1 {
            let code = part.split_once('\n').map_or(part, |(_, rest)| rest);
            egui::Frame::none().fill(th.ed_bg).rounding(Rounding::same(6.0)).inner_margin(Margin::same(8.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(egui::RichText::new(code.trim_end()).monospace().size(12.0)).wrap());
            });
            continue;
        }
        for line in part.lines() {
            let t = line.trim_end();
            let (size, txt) = if let Some(h) = t.strip_prefix("# ") {
                (20.0, h)
            } else if let Some(h) = t.strip_prefix("## ") {
                (17.0, h)
            } else if let Some(h) = t.strip_prefix("### ") {
                (15.0, h)
            } else {
                (0.0, t)
            };
            if size > 0.0 {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(txt).size(size).strong());
            } else if let Some(b) = t.trim_start().strip_prefix("- ").or_else(|| t.trim_start().strip_prefix("* ")) {
                ui.horizontal_top(|ui| {
                    ui.label(egui::RichText::new("•").color(th.accent));
                    ui.add(egui::Label::new(b.replace("**", "")).wrap());
                });
            } else if t.starts_with("> ") {
                ui.label(egui::RichText::new(&t[2..]).italics().color(th.sub));
            } else if t.is_empty() {
                ui.add_space(4.0);
            } else {
                ui.add(egui::Label::new(t.replace("**", "")).wrap());
            }
        }
    }
}

/// Schachbrett hinter durchsichtigen Bildern – nur der sichtbare Teil (vergrößerte Bilder sind riesig)
fn checker(ui: &egui::Ui, r: Rect, th: &Theme) {
    let vis = r.intersect(ui.clip_rect());
    if !vis.is_positive() {
        return;
    }
    let cell = 8.0;
    let (c1, c2) = if th.dark { (Color32::from_gray(48), Color32::from_gray(60)) } else { (Color32::from_gray(235), Color32::from_gray(250)) };
    let p = ui.painter_at(vis);
    p.rect_filled(vis, 0.0, c1);
    let (cx0, cy0) = (((vis.left() - r.left()) / cell).floor() as i64, ((vis.top() - r.top()) / cell).floor() as i64);
    let mut cy = cy0;
    while r.top() + cy as f32 * cell < vis.bottom() {
        let mut cx = cx0;
        while r.left() + cx as f32 * cell < vis.right() {
            if (cx + cy) % 2 == 1 {
                p.rect_filled(Rect::from_min_size(pos2(r.left() + cx as f32 * cell, r.top() + cy as f32 * cell), vec2(cell, cell)), 0.0, c2);
            }
            cx += 1;
        }
        cy += 1;
    }
}
