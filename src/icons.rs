//! Icons.
//!
//! * Standard: die mitgelieferten Icons aus `assets/icons` (Material Icon Theme, siehe `matico.rs`).
//!   Dateien nach Name/Endung, Ordner nach Ordnername. Nur wenn ein Icon fehlt, greifen die selbst
//!   gezeichneten Notfall-Icons (Farbe je Dateityp + Kürzel der Endung).
//! * System-Icons (nur Windows/macOS, per Einstellung): Wenn das System für die Endung ein EIGENES Icon hat,
//!   wird es verwendet; sonst bleibt das Standard-Icon.
//!   Die System-Icons werden in einem Hintergrund-Thread geholt – die Oberfläche wartet nie darauf.

use eframe::egui::{self, pos2, vec2, Color32, FontId, Painter, Rect, Shape, Stroke};
use std::{collections::HashMap, path::PathBuf, sync::mpsc};

#[derive(Clone, Copy, PartialEq)]
pub enum IconKind {
    Dir,
    File,
    Drive,
}

#[allow(dead_code)]
enum Slot {
    Pending,
    Custom,
    Tex(egui::TextureHandle),
}

#[derive(Clone)]
#[allow(dead_code)]
enum Target {
    Dir,
    NoExt,
    Ext(String),
    Path(String),
}

#[allow(dead_code)]
struct Req {
    target: Target,
    path: PathBuf,
    kind: IconKind,
    per_file: bool,
}

#[allow(dead_code)]
struct Rep {
    target: Target,
    img: Option<(usize, usize, Vec<u8>)>,
}

#[allow(dead_code)]
pub struct Icons {
    sys: bool,
    dir: Option<Slot>,
    noext: Option<Slot>,
    by_ext: HashMap<String, Slot>,
    by_path: HashMap<String, Slot>,
    tx: Option<mpsc::Sender<Req>>,
    rx: Option<mpsc::Receiver<Rep>>,
    /// Gerenderte Standard-Icons: Pixelgröße → Icon-Name → Textur
    mat: HashMap<u32, HashMap<String, Option<egui::TextureHandle>>>,
    dark: bool,
    ppp: f32,
    budget: u32,
    /// Icon-Theme für Dateien (und bei Catppuccin/vscode-icons auch Ordner)
    set: crate::matico::Set,
    /// eigene Ordner-Icons: (Vorlage/Bild, Pixelgröße) → Textur (None = ließ sich nicht laden)
    custom: HashMap<(crate::foldericons::Spec, u32), Option<egui::TextureHandle>>,
    /// Papierkorb-Symbol von Windows: [leer, voll] (None = noch nicht geladen, Some(None) = gibt es nicht)
    trash: [Option<Option<egui::TextureHandle>>; 2],
}

fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let (i, f) = ((h * 6.0).floor(), h * 6.0 - (h * 6.0).floor());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Farbe je Dateityp; unbekannte Endungen bekommen eine stabile Farbe aus dem Hash der Endung.
fn color_for(ext: &str) -> Color32 {
    match ext {
        "pdf" => rgb(220, 60, 50),
        "doc" | "docx" | "odt" | "rtf" => rgb(43, 87, 154),
        "xls" | "xlsx" | "ods" | "csv" => rgb(33, 115, 70),
        "ppt" | "pptx" | "odp" => rgb(210, 71, 38),
        "txt" | "log" | "md" | "ini" | "cfg" => rgb(120, 130, 140),
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tif" | "tiff"
        | "heic" | "avif" | "raw" | "psd" => rgb(46, 170, 120),
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" => rgb(130, 80, 200),
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" => rgb(224, 80, 150),
        "zip" | "rar" | "7z" | "tar" | "gz" | "xz" | "bz2" | "zst" | "iso" | "tgz" | "cab" => {
            rgb(220, 160, 40)
        }
        "exe" | "msi" | "bat" | "cmd" | "com" | "scr" | "appimage" | "ps1" | "sh" => {
            rgb(70, 110, 190)
        }
        "dll" | "sys" | "so" | "dylib" | "lib" | "a" => rgb(110, 120, 140),
        "rs" => rgb(222, 99, 45),
        "py" => rgb(55, 118, 171),
        "js" | "mjs" | "jsx" => rgb(212, 180, 30),
        "ts" | "tsx" => rgb(49, 120, 198),
        "html" | "htm" | "css" | "xml" => rgb(228, 110, 50),
        "json" | "yaml" | "yml" | "toml" => rgb(90, 150, 150),
        "c" | "h" | "cpp" | "hpp" | "cs" | "java" | "go" | "kt" | "swift" | "php" | "rb"
        | "sql" => rgb(60, 150, 170),
        "ttf" | "otf" | "woff" | "woff2" => rgb(150, 100, 100),
        "" => rgb(140, 148, 160),
        _ => {
            // FNV-1a
            let mut h: u32 = 2166136261;
            for b in ext.bytes() {
                h ^= b as u32;
                h = h.wrapping_mul(16777619);
            }
            hsv((h % 360) as f32 / 360.0, 0.55, 0.72)
        }
    }
}

/// Deutsche Typbezeichnung wie in der Explorer-Spalte „Typ“.
pub fn type_name(ext: &str, is_dir: bool) -> String {
    if is_dir {
        return "Dateiordner".into();
    }
    let t = match ext {
        "" => return "Datei".into(),
        "txt" | "log" => "Textdokument",
        "md" => "Markdown-Datei",
        "pdf" => "PDF-Dokument",
        "doc" | "docx" => "Microsoft Word-Dokument",
        "xls" | "xlsx" => "Microsoft Excel-Tabelle",
        "ppt" | "pptx" => "Microsoft PowerPoint-Präsentation",
        "jpg" | "jpeg" => "JPEG-Bild",
        "png" => "PNG-Bild",
        "gif" => "GIF-Bild",
        "svg" => "SVG-Grafik",
        "mp3" => "MP3-Audiodatei",
        "wav" | "flac" | "ogg" | "m4a" => "Audiodatei",
        "mp4" | "mkv" | "avi" | "mov" | "webm" => "Videodatei",
        "zip" => "ZIP-Archiv",
        "rar" => "RAR-Archiv",
        "7z" => "7-Zip-Archiv",
        "tar" | "gz" | "bz2" | "xz" | "zst" | "tgz" => "Komprimiertes Archiv",
        "exe" => "Anwendung",
        "dll" => "Anwendungserweiterung",
        "lnk" => "Verknüpfung",
        "rs" => "Rust-Quelldatei",
        "json" => "JSON-Datei",
        "html" | "htm" => "HTML-Dokument",
        _ => return format!("{}-Datei", ext.to_uppercase()),
    };
    t.into()
}

impl Icons {
    pub fn new() -> Self {
        Icons {
            sys: false,
            dir: None,
            noext: None,
            by_ext: HashMap::new(),
            by_path: HashMap::new(),
            tx: None,
            rx: None,
            mat: HashMap::new(),
            custom: HashMap::new(),
            trash: [None, None],
            dark: true,
            ppp: 1.0,
            budget: 48,
            set: crate::matico::Set::Material,
        }
    }

    pub fn set_theme(&mut self, set: crate::matico::Set) {
        if set != self.set {
            self.set = set;
            self.mat.clear();
        }
    }

    /// Zeichnet das Standard-Icon (Material Icon Theme). `false` = nicht verfügbar → Notfall-Icon.
    fn paint_material(&mut self, p: &Painter, r: Rect, kind: IconKind, name: &str, ext: &str) -> bool {
        use crate::matico::{self, Set};
        let icon = match kind {
            // Ordner: bei Material das klassische gelbe Windows-Ordnersymbol (selbst gezeichnet)
            IconKind::Dir if self.set == Set::Material => return false,
            IconKind::Dir => matico::folder_icon(name),
            IconKind::File => matico::file_icon(name, ext),
            IconKind::Drive => return false,
        };
        let px = matico::bucket(r.width().max(r.height()) * self.ppp);
        let Some((key, data)) = matico::icon_data(self.set, self.dark, icon, ext, kind == IconKind::Dir) else { return false };
        let hit = self.mat.get(&px).and_then(|m| m.get(&key));
        match hit {
            Some(Some(t)) => {
                p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                return true;
            }
            Some(None) => return false,
            None => {}
        }
        if self.budget == 0 {
            p.ctx().request_repaint(); // pro Bild nur begrenzt viele neue Icons rendern
            return false;
        }
        self.budget -= 1;
        let tex = matico::raster_data(data, px).map(|img| p.ctx().load_texture(format!("mat-{key}-{px}"), img, egui::TextureOptions::LINEAR));
        let ok = tex.is_some();
        if let Some(t) = &tex {
            p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        self.mat.entry(px).or_default().insert(key, tex);
        ok
    }

    /// Eigenes Ordner-Icon bzw. Vorlage zeichnen; false = ließ sich nicht laden (dann das normale Icon)
    pub fn paint_spec(&mut self, p: &Painter, r: Rect, spec: &crate::foldericons::Spec) -> bool {
        use crate::foldericons::Spec;
        if let Spec::Color(c) = spec {
            paint_folder_color(p, r, *c);
            return true;
        }
        let px = crate::matico::bucket(r.width().max(r.height()) * self.ppp);
        let k = (spec.clone(), px);
        if !self.custom.contains_key(&k) {
            if self.budget == 0 {
                p.ctx().request_repaint();
                return false;
            }
            self.budget -= 1;
            let img = match spec {
                Spec::Preset(n) => crate::matico::preset_svg(n, self.dark).and_then(|d| crate::matico::raster_data(d, px)),
                Spec::Image(f) if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) => std::fs::read(f).ok().and_then(|d| crate::matico::raster_data(&d, px)),
                Spec::Color(_) => None,
                Spec::Image(f) => image::open(f).ok().map(|i| {
                    // quadratisch einpassen, Seitenverhältnis bleibt
                    let t = i.thumbnail(px, px).to_rgba8();
                    let mut sq = image::RgbaImage::new(px, px);
                    image::imageops::overlay(&mut sq, &t, ((px - t.width()) / 2) as i64, ((px - t.height()) / 2) as i64);
                    egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], sq.as_raw())
                }),
            };
            let tex = img.map(|i| p.ctx().load_texture(format!("ordner-{spec:?}-{px}"), i, egui::TextureOptions::LINEAR));
            self.custom.insert(k.clone(), tex);
        }
        match self.custom.get(&k) {
            Some(Some(t)) => {
                p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                true
            }
            _ => false,
        }
    }

    /// Papierkorb: das Symbol von Windows (leer bzw. voll), sonst ein gezeichnetes
    pub fn paint_trash(&mut self, p: &Painter, r: Rect, full: bool) {
        let slot = &mut self.trash[full as usize];
        if slot.is_none() {
            *slot = Some(stock_recycler(full).map(|img| p.ctx().load_texture(format!("papierkorb-{full}"), img, egui::TextureOptions::LINEAR)));
        }
        match slot {
            Some(Some(t)) if self.sys => {
                p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            _ => paint_trash(p, r, full),
        };
    }

    /// Bild eines eigenen Icons neu laden (Datei geändert)
    pub fn forget_spec(&mut self, spec: &crate::foldericons::Spec) {
        self.custom.retain(|k, _| k.0 != *spec);
    }

    /// System-Icons ein-/ausschalten (wirkt nur unter Windows/macOS).
    pub fn set_system(&mut self, on: bool, ctx: &egui::Context) {
        let _ = &ctx;
        self.sys = on;
        self.dir = None;
        self.noext = None;
        self.by_ext.clear();
        self.by_path.clear();
        #[cfg(any(windows, target_os = "macos"))]
        if on && self.tx.is_none() {
            let (rq_tx, rq_rx) = mpsc::channel::<Req>();
            let (rp_tx, rp_rx) = mpsc::channel::<Rep>();
            self.tx = Some(rq_tx);
            self.rx = Some(rp_rx);
            let ctx = ctx.clone();
            std::thread::spawn(move || worker(rq_rx, rp_tx, ctx));
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = ctx;
    }

    /// Fertige System-Icons aus dem Hintergrund-Thread übernehmen (einmal pro Frame aufrufen).
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.ppp = ctx.pixels_per_point();
        self.budget = 48;
        let dark = ctx.style().visuals.dark_mode;
        if dark != self.dark {
            self.dark = dark;
            self.mat.clear();
        }
        let Some(rx) = &self.rx else { return };
        let mut n = 0;
        while let Ok(rep) = rx.try_recv() {
            let slot = match rep.img {
                Some((w, h, px)) => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
                    Slot::Tex(ctx.load_texture("sysicon", img, egui::TextureOptions::LINEAR))
                }
                None => Slot::Custom,
            };
            match rep.target {
                Target::Dir => self.dir = Some(slot),
                Target::NoExt => self.noext = Some(slot),
                Target::Ext(e) => {
                    self.by_ext.insert(e, slot);
                }
                Target::Path(p) => {
                    if self.by_path.len() > 300 {
                        self.by_path.clear();
                    }
                    self.by_path.insert(p, slot);
                }
            }
            n += 1;
            if n >= 16 {
                ctx.request_repaint();
                break;
            }
        }
    }

    /// Zeichnet ein Icon. `path` wird nur aufgerufen, wenn wirklich ein System-Icon angefordert werden muss.
    pub fn paint(
        &mut self,
        p: &Painter,
        r: Rect,
        kind: IconKind,
        name: &str,
        ext: &str,
        unique: bool,
        path: impl FnOnce() -> PathBuf,
    ) {
        let mut src = Some(path);
        let mut memo: Option<PathBuf> = None;
        let mut path = move || -> PathBuf {
            if memo.is_none() {
                memo = src.take().map(|f| f());
            }
            memo.clone().unwrap_or_default()
        };
        // eigenes Ordner-Icon geht vor
        if kind != IconKind::File && !crate::foldericons::is_empty() {
            if let Some(spec) = crate::foldericons::get(&path()) {
                if self.paint_spec(p, r, &spec) {
                    return;
                }
            }
        }
        #[cfg(any(windows, target_os = "macos"))]
        if self.sys && self.tx.is_some() {
            const PER_FILE: [&str; 9] = [
                "exe",
                "lnk",
                "ico",
                "msi",
                "scr",
                "url",
                "cur",
                "ani",
                "appref-ms",
            ];
            let per_file = unique
                || kind == IconKind::Drive
                || (kind == IconKind::File && PER_FILE.contains(&ext));
            // `path` is `FnOnce` and may be called at most once. We wrap it in an
            // `Option` so we can "take" it exactly where we actually need it —
            // either in the `per_file` branch below, or (if that branch did not
            // run) later when a request has to be queued.
            let mut path_opt = None;
            let (slot, target): (Option<&Slot>, Target) = if per_file {
                let pb = path();
                let key = pb.to_string_lossy().into_owned();
                path_opt = Some(pb);
                (self.by_path.get(&key), Target::Path(key))
            } else if kind == IconKind::Dir {
                (self.dir.as_ref(), Target::Dir)
            } else if ext.is_empty() {
                (self.noext.as_ref(), Target::NoExt)
            } else {
                (self.by_ext.get(ext), Target::Ext(ext.to_string()))
            };
            match slot {
                Some(Slot::Tex(t)) => {
                    p.image(
                        t.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    return;
                }
                Some(_) => {}
                None => {
                    let pb = path_opt.unwrap_or_else(&mut path);
                    let slot = Slot::Pending;
                    match &target {
                        Target::Dir => self.dir = Some(slot),
                        Target::NoExt => self.noext = Some(slot),
                        Target::Ext(e) => {
                            self.by_ext.insert(e.clone(), slot);
                        }
                        Target::Path(k) => {
                            self.by_path.insert(k.clone(), slot);
                        }
                    }
                    if let Some(tx) = &self.tx {
                        let _ = tx.send(Req {
                            target,
                            path: pb,
                            kind,
                            per_file,
                        });
                    }
                }
            }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = (&mut path, unique);
        if self.paint_material(p, r, kind, name, ext) {
            return;
        }
        match kind {
            IconKind::Dir => paint_folder(p, r),
            IconKind::Drive => paint_drive(p, r),
            IconKind::File => paint_file(p, r, ext),
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn worker(rx: mpsc::Receiver<Req>, tx: mpsc::Sender<Rep>, ctx: egui::Context) {
    // Standard-Icon des Systems für „unbekannte Endung“ ermitteln, um es später zu erkennen.
    let probe = std::env::temp_dir().join("foldex_probe.fcprobe");
    let _ = std::fs::write(&probe, b"");
    let generic = file_icon_provider::get_file_icon(&probe, 32)
        .ok()
        .map(|i| i.pixels);
    for req in rx {
        let ic = file_icon_provider::get_file_icon(&req.path, 32).ok();
        let img = ic.and_then(|ic| {
            let is_generic =
                req.kind == IconKind::File && !req.per_file && generic.as_ref() == Some(&ic.pixels);
            if is_generic {
                None // System hat kein eigenes Icon für diese Endung → eigenes Icon verwenden
            } else {
                Some((ic.width as usize, ic.height as usize, ic.pixels))
            }
        });
        let _ = tx.send(Rep {
            target: req.target,
            img,
        });
        ctx.request_repaint();
    }
}

/// Papierkorb-Symbol aus der Windows-Shell (wie im Explorer), als Bild
#[cfg(windows)]
fn stock_recycler(full: bool) -> Option<egui::ColorImage> {
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::UI::Shell::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL};
    const PX: i32 = 32;
    unsafe {
        let mut sii: SHSTOCKICONINFO = std::mem::zeroed();
        sii.cbSize = std::mem::size_of::<SHSTOCKICONINFO>() as u32;
        if SHGetStockIconInfo(if full { SIID_RECYCLERFULL } else { SIID_RECYCLER }, SHGSI_ICON | SHGSI_LARGEICON, &mut sii) != 0 || sii.hIcon.is_null() {
            return None;
        }
        // in ein 32-Bit-Bild (von oben nach unten) zeichnen, Alpha bleibt erhalten
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = PX;
        bi.bmiHeader.biHeight = -PX;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let mut out = None;
        if !bmp.is_null() && !bits.is_null() {
            let old = SelectObject(dc, bmp);
            DrawIconEx(dc, 0, 0, sii.hIcon, PX, PX, 0, std::ptr::null_mut(), DI_NORMAL);
            let px = std::slice::from_raw_parts(bits as *const u8, (PX * PX * 4) as usize);
            // BGRA (vormultipliziert) → RGBA
            let rgba: Vec<u8> = px.chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect();
            if rgba.chunks_exact(4).any(|c| c[3] > 0) {
                out = Some(egui::ColorImage::from_rgba_premultiplied([PX as usize, PX as usize], &rgba));
            }
            SelectObject(dc, old);
            DeleteObject(bmp);
        }
        DeleteDC(dc);
        DestroyIcon(sii.hIcon);
        out
    }
}

#[cfg(not(windows))]
fn stock_recycler(_full: bool) -> Option<egui::ColorImage> {
    None
}

/// Papierkorb (Seitenleiste): Eimer mit Deckel, gefüllt mit Papier, wenn etwas drin ist
pub fn paint_trash(p: &Painter, r: Rect, full: bool) {
    let (w, h) = (r.width(), r.height());
    let x = |f: f32| r.left() + w * f;
    let y = |f: f32| r.top() + h * f;
    let body = vec![pos2(x(0.2), y(0.3)), pos2(x(0.8), y(0.3)), pos2(x(0.73), y(0.94)), pos2(x(0.27), y(0.94))];
    if full {
        // Papier ragt oben heraus
        p.add(Shape::convex_polygon(vec![pos2(x(0.3), y(0.3)), pos2(x(0.38), y(0.08)), pos2(x(0.55), y(0.14)), pos2(x(0.5), y(0.3))], rgb(245, 245, 240), Stroke::new(0.8, rgb(170, 170, 160))));
        p.add(Shape::convex_polygon(vec![pos2(x(0.48), y(0.3)), pos2(x(0.6), y(0.06)), pos2(x(0.74), y(0.16)), pos2(x(0.68), y(0.3))], rgb(225, 235, 250), Stroke::new(0.8, rgb(150, 165, 190))));
    }
    p.add(Shape::convex_polygon(body, rgb(120, 170, 220), Stroke::new(1.0, rgb(60, 105, 160))));
    // Rillen
    for f in [0.38, 0.5, 0.62] {
        p.line_segment([pos2(x(f), y(0.4)), pos2(x(f), y(0.86))], Stroke::new(1.0, rgb(80, 130, 185)));
    }
    // Rand oben
    p.rect_filled(Rect::from_min_max(pos2(x(0.14), y(0.24)), pos2(x(0.86), y(0.33))), 2.0, rgb(85, 135, 195));
}

/// klassischer Ordner in einer Farbe (Rücken dunkler, Vorderseite heller)
pub fn paint_folder_color(p: &Painter, r: Rect, c: u32) {
    let (cr, cg, cb) = (((c >> 16) & 255) as f32, ((c >> 8) & 255) as f32, (c & 255) as f32);
    let shade = |k: f32| {
        let f = |v: f32| if k < 1.0 { v * k } else { v + (255.0 - v) * (k - 1.0) };
        Color32::from_rgb(f(cr).clamp(0.0, 255.0) as u8, f(cg).clamp(0.0, 255.0) as u8, f(cb).clamp(0.0, 255.0) as u8)
    };
    let (w, h) = (r.width(), r.height());
    let back = Rect::from_min_size(r.min + vec2(w * 0.04, h * 0.14), vec2(w * 0.44, h * 0.24));
    p.rect_filled(back, 2.0, shade(0.82));
    let body = Rect::from_min_max(r.min + vec2(w * 0.04, h * 0.27), r.max - vec2(w * 0.04, h * 0.14));
    p.rect_filled(body, 2.5, shade(1.0));
    let front = Rect::from_min_max(pos2(body.left(), body.top() + h * 0.12), body.max);
    p.rect_filled(front, 2.5, shade(1.18));
}

fn paint_folder(p: &Painter, r: Rect) {
    let (w, h) = (r.width(), r.height());
    let back = Rect::from_min_size(r.min + vec2(w * 0.04, h * 0.14), vec2(w * 0.44, h * 0.24));
    p.rect_filled(back, 2.0, rgb(222, 164, 36));
    let body = Rect::from_min_max(
        r.min + vec2(w * 0.04, h * 0.27),
        r.max - vec2(w * 0.04, h * 0.14),
    );
    p.rect_filled(body, 2.5, rgb(250, 196, 66));
    let front = Rect::from_min_max(pos2(body.left(), body.top() + h * 0.12), body.max);
    p.rect_filled(front, 2.5, rgb(255, 214, 102));
}

fn paint_drive(p: &Painter, r: Rect) {
    let (w, h) = (r.width(), r.height());
    let body = Rect::from_min_max(
        r.min + vec2(w * 0.06, h * 0.30),
        r.max - vec2(w * 0.06, h * 0.28),
    );
    p.rect_filled(body, 3.0, rgb(150, 158, 170));
    p.rect_filled(
        Rect::from_min_max(body.min + vec2(0.0, body.height() * 0.55), body.max),
        3.0,
        rgb(112, 120, 134),
    );
    p.circle_filled(
        pos2(
            body.right() - w * 0.14,
            body.bottom() - body.height() * 0.22,
        ),
        w * 0.045,
        rgb(90, 220, 120),
    );
}

fn paint_file(p: &Painter, r: Rect, ext: &str) {
    let (w, h) = (r.width(), r.height());
    let col = color_for(ext);
    let pg = Rect::from_min_max(
        r.min + vec2(w * 0.14, h * 0.02),
        r.max - vec2(w * 0.14, h * 0.02),
    );
    let edge = Stroke::new(1.0, rgb(176, 182, 192));
    p.rect_filled(pg, 2.0, Color32::WHITE);
    p.rect_stroke(pg, 2.0, edge);
    for k in 0..2 {
        let y = pg.top() + h * (0.16 + 0.09 * k as f32);
        p.line_segment(
            [
                pos2(pg.left() + w * 0.08, y),
                pos2(pg.right() - w * 0.16, y),
            ],
            Stroke::new(1.0, rgb(208, 213, 220)),
        );
    }
    // Farbband mit Endung
    let band = Rect::from_min_max(
        pos2(pg.left() + 1.0, pg.bottom() - h * 0.40),
        pos2(pg.right() - 1.0, pg.bottom() - 1.0),
    );
    p.rect_filled(band, 1.5, col);
    let c = w * 0.22;
    p.add(Shape::convex_polygon(
        vec![
            pos2(pg.right() - c, pg.top()),
            pos2(pg.right(), pg.top() + c),
            pos2(pg.right() - c, pg.top() + c),
        ],
        rgb(228, 232, 238),
        edge,
    ));
    if !ext.is_empty() && w >= 16.0 {
        let label: String = ext.chars().take(4).collect::<String>().to_uppercase();
        let n = label.chars().count() as f32;
        let size = (band.width() / (n * 0.62))
            .min(band.height() * 0.78)
            .max(5.0);
        p.text(
            band.center(),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(size),
            Color32::WHITE,
        );
    }
}
