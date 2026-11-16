//! Drucken in Foldex selbst (Strg+P, Rechtsklick → „Drucken“) – eigenes Druckfenster mit Vorschau, Drucker,
//! Kopien, Seitenbereich, Ausrichtung und Schwarzweiß. Es wird kein anderes Programm geöffnet.
//!
//! - PDF: eigener PDF-Renderer (Windows.Data.Pdf)
//! - Bilder (auch HEIC, WebP, SVG): Foldex' Bildleser
//! - Dokumente und Text (Word, ODT, RTF, Markdown, HTML, CSV, Excel, Quelltext …): erst mit Foldex' eigenem
//!   Umwandler unsichtbar in ein PDF, dann wie ein PDF
//!
//! Windows druckt über GDI (Seite als Bild, ins bedruckbare Feld eingepasst). Linux gibt die Datei an CUPS
//! (`lp`) mit denselben Einstellungen.

use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, Rect, Sense, Stroke};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{mpsc, Arc, Mutex};

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Bilder, die Foldex selbst lesen kann
const IMAGES: [&str; 18] = ["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "ico", "tga", "qoi", "pnm", "svg", "avif", "heic", "heif", "jfif", "dds"];

fn is_text(p: &Path) -> bool {
    crate::editors::is_text(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Lässt sich diese Datei drucken?
pub fn supported(p: &Path) -> bool {
    let ext = ext_of(p);
    !ext.is_empty() && (ext == "pdf" || IMAGES.contains(&ext.as_str()) || is_text(p) || crate::docconv::can(&ext, "pdf"))
}

/// Was gedruckt wird: ein PDF (auch aus einem Dokument umgewandelt) oder ein Bild
#[derive(Clone)]
pub enum Source {
    /// Seitenzahl 0 = unbekannt (Linux ohne eigenen PDF-Renderer)
    Pdf { path: PathBuf, pages: u32, temp: bool },
    Image(PathBuf),
}

impl Source {
    fn pages(&self) -> u32 {
        match self {
            Source::Pdf { pages, .. } => *pages,
            Source::Image(_) => 1,
        }
    }
}

impl Drop for PrintWin {
    fn drop(&mut self) {
        // umgewandelte Dokumente wieder löschen
        for s in self.prep.iter().flatten().flatten() {
            if let Source::Pdf { path, temp: true, .. } = s {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Datei für den Druck vorbereiten (kann dauern: Dokumente werden umgewandelt)
fn prepare(p: &Path) -> Result<Source, String> {
    static N: AtomicU32 = AtomicU32::new(0);
    let ext = ext_of(p);
    if ext == "pdf" {
        return Ok(Source::Pdf { path: p.to_path_buf(), pages: page_count(p)?, temp: false });
    }
    if IMAGES.contains(&ext.as_str()) {
        return Ok(Source::Image(p.to_path_buf()));
    }
    // Dokumente und Text: unsichtbar in ein PDF umwandeln
    let tmp_dir = std::env::temp_dir();
    let k = N.fetch_add(1, Relaxed);
    let pdf = tmp_dir.join(format!("foldex-druck-{}-{k}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&pdf);
    if crate::docconv::can(&ext, "pdf") {
        crate::docconv::convert(p, &pdf)?;
    } else if is_text(p) {
        // Quelltext & Co.: als reiner Text
        let txt = tmp_dir.join(format!("foldex-druck-{}-{k}.txt", std::process::id()));
        std::fs::copy(p, &txt).map_err(|e| e.to_string())?;
        let r = crate::docconv::convert(&txt, &pdf);
        let _ = std::fs::remove_file(&txt);
        r?;
    } else {
        return Err("Dieses Format lässt sich nicht drucken".into());
    }
    let pages = page_count(&pdf).unwrap_or(0);
    Ok(Source::Pdf { path: pdf, pages, temp: true })
}

fn page_count(p: &Path) -> Result<u32, String> {
    #[cfg(windows)]
    return crate::pdfview::pdf_pages(p, 100, |_| false, |_, _, _| true);
    #[cfg(not(windows))]
    {
        let _ = p;
        Ok(0)
    }
}

/// eine Seite als Bild in der Breite `w` (0-basiert)
fn render(src: &Source, page: u32, w: u32) -> Result<image::RgbaImage, String> {
    match src {
        Source::Image(p) => {
            let img = crate::convert::load_image(p, w.max(1200))?.to_rgba8();
            Ok(if img.width() > w * 2 { image::imageops::resize(&img, w * 2, (img.height() as u64 * (w as u64 * 2) / img.width().max(1) as u64) as u32, image::imageops::FilterType::Triangle) } else { img })
        }
        Source::Pdf { path, .. } => {
            #[cfg(windows)]
            {
                let mut out = None;
                crate::pdfview::pdf_pages(path, w, |i| i == page, |_, _, img| {
                    out = Some(img);
                    false
                })?;
                out.ok_or_else(|| "Seite nicht gefunden".into())
            }
            #[cfg(not(windows))]
            {
                let _ = (path, page);
                Err("Vorschau für PDF gibt es unter Linux noch nicht – gedruckt wird trotzdem".into())
            }
        }
    }
}

/// Drucker: (Namen, Standarddrucker)
pub fn printers() -> (Vec<String>, Option<String>) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Graphics::Printing::{EnumPrintersW, GetDefaultPrinterW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_4W};
        let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
        let (mut need, mut n) = (0u32, 0u32);
        EnumPrintersW(flags, std::ptr::null(), 4, std::ptr::null_mut(), 0, &mut need, &mut n);
        let mut buf = vec![0u8; need as usize + 16];
        let mut list = Vec::new();
        if need > 0 && EnumPrintersW(flags, std::ptr::null(), 4, buf.as_mut_ptr(), buf.len() as u32, &mut need, &mut n) != 0 {
            let infos = std::slice::from_raw_parts(buf.as_ptr() as *const PRINTER_INFO_4W, n as usize);
            for i in infos {
                list.push(wide(i.pPrinterName));
            }
        }
        let mut len = 512u32;
        let mut d = vec![0u16; 512];
        let def = (GetDefaultPrinterW(d.as_mut_ptr(), &mut len) != 0).then(|| String::from_utf16_lossy(&d[..len.saturating_sub(1) as usize]));
        (list, def)
    }
    #[cfg(not(windows))]
    {
        let run = |a: &[&str]| std::process::Command::new("lpstat").args(a).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        let list: Vec<String> = run(&["-e"]).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        let def = run(&["-d"]).rsplit(':').next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && list.contains(s));
        (list, def)
    }
}

#[cfg(windows)]
unsafe fn wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

/// „1-3, 5“ → 0-basierte Seiten; leer = alle (None)
pub fn parse_range(s: &str, count: u32) -> Result<Option<Vec<u32>>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let mut v = Vec::new();
    for part in s.split([',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (part, part),
        };
        let a: u32 = a.parse().map_err(|_| format!("„{part}“ ist keine Seitenangabe"))?;
        let b: u32 = if b.is_empty() { count.max(a) } else { b.parse().map_err(|_| format!("„{part}“ ist keine Seitenangabe"))? };
        if a == 0 || b < a {
            return Err(format!("„{part}“ ist kein gültiger Bereich"));
        }
        for p in a..=b {
            if count == 0 || p <= count {
                v.push(p - 1);
            }
        }
    }
    v.dedup();
    if v.is_empty() {
        return Err("Keine Seite im Bereich".into());
    }
    Ok(Some(v))
}

/// Einstellungen eines Druckauftrags
#[derive(Clone)]
pub struct Opts {
    pub printer: String,
    pub copies: u32,
    pub range: String,
    /// 0 automatisch (nach der ersten Seite), 1 Hochformat, 2 Querformat
    pub orient: u8,
    pub gray: bool,
}

/// Fortschritt eines laufenden Auftrags
#[derive(Default)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
    pub finished: bool,
    pub err: Option<String>,
    pub cancel: bool,
}

/// Auftrag ausführen (im Hintergrund)
fn run_job(files: Vec<(PathBuf, Source)>, o: Opts, prog: Arc<Mutex<Progress>>) -> Result<(), String> {
    #[cfg(windows)]
    {
        gdi::job(files, &o, &prog)
    }
    #[cfg(not(windows))]
    {
        // CUPS druckt PDF, Bilder und Text selbst
        let n = files.len() as u32;
        prog.lock().unwrap().total = n;
        for (_, src) in &files {
            if prog.lock().unwrap().cancel {
                break;
            }
            let file = match src {
                Source::Pdf { path, .. } | Source::Image(path) => path,
            };
            let mut c = std::process::Command::new("lp");
            if !o.printer.is_empty() {
                c.args(["-d", &o.printer]);
            }
            c.args(["-n", &o.copies.max(1).to_string()]);
            match o.orient {
                1 => {
                    c.args(["-o", "orientation-requested=3"]);
                }
                2 => {
                    c.args(["-o", "orientation-requested=4"]);
                }
                _ => {}
            }
            c.args(["-o", "fit-to-page"]);
            if o.gray {
                c.args(["-o", "print-color-mode=monochrome", "-o", "ColorModel=Gray"]);
            }
            if !o.range.trim().is_empty() && matches!(src, Source::Pdf { .. }) {
                c.args(["-P", o.range.trim()]);
            }
            let out = c.arg("--").arg(file).output().map_err(|e| format!("lp: {e}"))?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
            }
            prog.lock().unwrap().done += 1;
        }
        Ok(())
    }
}

/// Windows: Druck über GDI – Gerät mit gewünschter Ausrichtung/Farbe, Seiten als Bild
#[cfg(windows)]
mod gdi {
    use super::*;
    use windows_sys::Win32::Graphics::Gdi::{CreateDCW, DeleteDC, GetDeviceCaps, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DEVMODEW, DIB_RGB_COLORS, DMCOLOR_MONOCHROME, DMORIENT_LANDSCAPE, DMORIENT_PORTRAIT, DM_COLOR, DM_IN_BUFFER, DM_ORIENTATION, DM_OUT_BUFFER, HDC, HORZRES, SRCCOPY, VERTRES};
    use windows_sys::Win32::Graphics::Printing::{ClosePrinter, DocumentPropertiesW, OpenPrinterW};
    use windows_sys::Win32::Storage::Xps::{EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Druckergerät mit Ausrichtung und Farbe öffnen
    pub(super) fn open(printer: &str, landscape: bool, gray: bool) -> Result<HDC, String> {
        let name = w(printer);
        unsafe {
            let mut h: windows_sys::Win32::Foundation::HANDLE = std::ptr::null_mut();
            if OpenPrinterW(name.as_ptr(), &mut h, std::ptr::null()) == 0 {
                return Err(format!("Drucker „{printer}“ nicht erreichbar"));
            }
            let size = DocumentPropertiesW(std::ptr::null_mut(), h, name.as_ptr(), std::ptr::null_mut(), std::ptr::null(), 0);
            let mut buf = vec![0u8; size.max(std::mem::size_of::<DEVMODEW>() as i32) as usize + 64];
            let dm = buf.as_mut_ptr() as *mut DEVMODEW;
            let ok = size > 0 && DocumentPropertiesW(std::ptr::null_mut(), h, name.as_ptr(), dm, std::ptr::null(), DM_OUT_BUFFER) >= 0;
            if ok {
                (*dm).Anonymous1.Anonymous1.dmOrientation = if landscape { DMORIENT_LANDSCAPE } else { DMORIENT_PORTRAIT } as i16;
                (*dm).dmFields |= DM_ORIENTATION;
                if gray {
                    (*dm).dmColor = DMCOLOR_MONOCHROME;
                    (*dm).dmFields |= DM_COLOR;
                }
                DocumentPropertiesW(std::ptr::null_mut(), h, name.as_ptr(), dm, dm, DM_IN_BUFFER | DM_OUT_BUFFER);
            }
            ClosePrinter(h);
            let drv = w("WINSPOOL");
            let hdc = CreateDCW(drv.as_ptr(), name.as_ptr(), std::ptr::null(), if ok { dm } else { std::ptr::null() });
            if hdc.is_null() {
                return Err(format!("Drucker „{printer}“ lässt sich nicht öffnen"));
            }
            Ok(hdc)
        }
    }

    /// eine Seite (Bild) ins bedruckbare Feld einpassen und drucken
    pub(super) fn page(hdc: HDC, img: image::RgbaImage, gray: bool) -> Result<(), String> {
        let (pw, ph) = unsafe { (GetDeviceCaps(hdc, HORZRES as i32), GetDeviceCaps(hdc, VERTRES as i32)) };
        let (iw, ih) = (img.width() as i32, img.height() as i32);
        let s = (pw as f32 / iw as f32).min(ph as f32 / ih as f32);
        let (dw, dh) = ((iw as f32 * s) as i32, (ih as f32 * s) as i32);
        let (x, y) = ((pw - dw) / 2, (ph - dh) / 2);
        let mut px = img.into_raw();
        for c in px.chunks_exact_mut(4) {
            // Transparenz auf Weiß (Papier), RGBA → BGRA, auf Wunsch Graustufen
            let a = c[3] as u32;
            let bl = |v: u8| ((v as u32 * a + 255 * (255 - a)) / 255) as u8;
            let (mut r, mut g, mut b) = (bl(c[0]), bl(c[1]), bl(c[2]));
            if gray {
                let l = ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8;
                (r, g, b) = (l, l, l);
            }
            c[0] = b;
            c[1] = g;
            c[2] = r;
            c[3] = 255;
        }
        let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
        bmi.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: iw, biHeight: -ih, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, ..unsafe { std::mem::zeroed() } };
        unsafe {
            StartPage(hdc);
            let n = StretchDIBits(hdc, x, y, dw, dh, 0, 0, iw, ih, px.as_ptr().cast(), &bmi, DIB_RGB_COLORS, SRCCOPY);
            EndPage(hdc);
            if n == 0 {
                return Err("Seite konnte nicht an den Drucker gegeben werden".into());
            }
        }
        Ok(())
    }

    /// Auftrag beginnen; `output` nur für Tests (z. B. „Microsoft Print to PDF“ in eine Datei)
    pub(super) fn start(hdc: HDC, title: &str, output: Option<&Path>) -> Result<(), String> {
        let t = w(title);
        let out = output.map(|o| w(&o.to_string_lossy()));
        let di = DOCINFOW { cbSize: std::mem::size_of::<DOCINFOW>() as i32, lpszDocName: t.as_ptr(), lpszOutput: out.as_ref().map_or(std::ptr::null(), |o| o.as_ptr()), lpszDatatype: std::ptr::null(), fwType: 0 };
        if unsafe { StartDocW(hdc, &di) } <= 0 {
            unsafe { DeleteDC(hdc) };
            return Err("Der Drucker hat den Auftrag abgelehnt".into());
        }
        Ok(())
    }

    pub(super) fn finish(hdc: HDC) {
        unsafe {
            EndDoc(hdc);
            DeleteDC(hdc);
        }
    }

    /// ganzer Auftrag: alle Dateien, gewünschte Seiten, Kopien (sortiert)
    pub(super) fn job(files: Vec<(PathBuf, Source)>, o: &Opts, prog: &Arc<Mutex<Progress>>) -> Result<(), String> {
        let landscape = match o.orient {
            1 => false,
            2 => true,
            // automatisch: wie die erste Seite
            _ => files.first().and_then(|(_, s)| render(s, 0, 300).ok()).is_some_and(|i| i.width() > i.height()),
        };
        // Seiten je Datei (Bereich gilt für jede Datei)
        let plan: Vec<(usize, Vec<u32>)> = files
            .iter()
            .enumerate()
            .map(|(k, (_, s))| {
                let n = s.pages().max(1);
                let pages = parse_range(&o.range, n).ok().flatten().unwrap_or_else(|| (0..n).collect());
                (k, pages.into_iter().filter(|p| *p < n).collect())
            })
            .collect();
        let copies = o.copies.clamp(1, 99);
        prog.lock().unwrap().total = plan.iter().map(|(_, p)| p.len() as u32).sum::<u32>() * copies;
        let hdc = open(&o.printer, landscape, o.gray)?;
        let title = files.first().and_then(|(p, _)| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Foldex".into());
        start(hdc, &title, None)?;
        let width = (unsafe { GetDeviceCaps(hdc, HORZRES as i32) } as u32).clamp(800, 3000);
        let mut res = Ok(());
        'out: for _ in 0..copies {
            for (k, pages) in &plan {
                for &p in pages {
                    if prog.lock().unwrap().cancel {
                        break 'out;
                    }
                    match render(&files[*k].1, p, width).and_then(|img| page(hdc, img, o.gray)) {
                        Ok(()) => prog.lock().unwrap().done += 1,
                        Err(e) => {
                            res = Err(e);
                            break 'out;
                        }
                    }
                }
            }
        }
        finish(hdc);
        res
    }
}

// ───────────────────────── Druckfenster ─────────────────────────

/// kleine Überschrift über einer Einstellung
fn label(ui: &mut egui::Ui, th: &Theme, t: &str) {
    ui.label(egui::RichText::new(t).size(12.0).strong().color(th.sub));
    ui.add_space(4.0);
}

/// Auswahl als Segmente über die ganze Breite; true = geändert
fn segmented(ui: &mut egui::Ui, th: &Theme, id: &str, sel: &mut u8, opts: &[&str]) -> bool {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
    ui.painter().rect(rect, 9.0, th.field, Stroke::new(1.0, th.line));
    let sw = (w - 6.0) / opts.len() as f32;
    let mut changed = false;
    for (i, o) in opts.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 3.0 + sw * i as f32, rect.top() + 3.0), vec2(sw, 28.0));
        let resp = ui.interact(r, ui.id().with(("seg", id, i)), Sense::click());
        let on = *sel as usize == i;
        if on {
            ui.painter().rect_filled(r, 7.0, th.accent);
        } else if resp.hovered() && ui.is_enabled() {
            ui.painter().rect_filled(r, 7.0, th.hover);
        }
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *o, egui::FontId::proportional(13.0), if on { th.on_accent() } else { th.text });
        if resp.clicked() && !on && ui.is_enabled() {
            *sel = i as u8;
            changed = true;
        }
    }
    changed
}

/// runder −/+ Knopf; true = geklickt
fn step(ui: &mut egui::Ui, th: &Theme, t: &str, enabled: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    ui.painter().rect(r, 8.0, if enabled && resp.hovered() { th.hover } else { th.field }, Stroke::new(1.0, th.line));
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(17.0), if enabled { th.text } else { th.line });
    enabled && resp.clicked()
}

/// Pfeil zum Blättern; true = geklickt
fn chevron(ui: &mut egui::Ui, th: &Theme, r: Rect, right: bool, enabled: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(("blaettern", right)), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().circle_filled(r.center(), 14.0, th.hover);
    }
    let c = r.center();
    let d = if right { 1.0 } else { -1.0 };
    let col = if enabled { th.text } else { th.line };
    ui.painter().add(egui::Shape::line(vec![c + vec2(-2.5 * d, -5.0), c + vec2(2.5 * d, 0.0), c + vec2(-2.5 * d, 5.0)], Stroke::new(1.8, col)));
    enabled && resp.clicked()
}

pub struct PrintWin {
    files: Vec<PathBuf>,
    /// vorbereitete Dateien (None = läuft noch)
    prep: Vec<Option<Result<Source, String>>>,
    prep_rx: mpsc::Receiver<(usize, Result<Source, String>)>,
    printers: Vec<String>,
    printer: usize,
    copies: u32,
    range: String,
    orient: u8,
    color: u8,
    /// angezeigte Datei und Seite (0-basiert)
    cur: usize,
    page: u32,
    /// Vorschau: Seite, Textur in Farbe, Textur in Graustufen, Größe
    preview: Option<((usize, u32), egui::TextureHandle, egui::TextureHandle, [u32; 2])>,
    /// „Bereich“ gewählt (auch solange das Feld noch leer ist)
    range_mode: bool,
    preview_rx: Option<mpsc::Receiver<((usize, u32), Result<image::RgbaImage, String>)>>,
    preview_err: Option<String>,
    job: Option<Arc<Mutex<Progress>>>,
    /// Meldung nach dem Druck (für die Statuszeile)
    pub result: Option<String>,
}

impl PrintWin {
    pub fn new(files: Vec<PathBuf>, ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        for (k, f) in files.iter().enumerate() {
            let (tx, f, ctx) = (tx.clone(), f.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send((k, prepare(&f)));
                ctx.request_repaint();
            });
        }
        let (list, def) = printers();
        let printer = def.and_then(|d| list.iter().position(|p| *p == d)).unwrap_or(0);
        PrintWin { prep: vec![None; files.len()], files, prep_rx: rx, printers: list, printer, copies: 1, range: String::new(), range_mode: false, orient: 0, color: 0, cur: 0, page: 0, preview: None, preview_rx: None, preview_err: None, job: None, result: None }
    }

    fn src(&self, k: usize) -> Option<&Source> {
        self.prep.get(k).and_then(|p| p.as_ref()).and_then(|r| r.as_ref().ok())
    }

    /// Vorschau der aktuellen Seite im Hintergrund rendern
    fn want_preview(&mut self, ctx: &egui::Context) {
        let key = (self.cur, self.page);
        if self.preview.as_ref().is_some_and(|p| p.0 == key) || self.preview_rx.is_some() {
            return;
        }
        let Some(src) = self.src(self.cur).cloned() else { return };
        let (tx, rx) = mpsc::channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((key, render(&src, key.1, 900)));
            ctx2.request_repaint();
        });
        self.preview_rx = Some(rx);
    }

    /// Fenster zeichnen; false = geschlossen
    pub fn ui(&mut self, ctx: &egui::Context, th: &Theme) -> bool {
        while let Ok((k, r)) = self.prep_rx.try_recv() {
            if let Some(slot) = self.prep.get_mut(k) {
                *slot = Some(r);
            }
        }
        if let Some(Ok((key, r))) = self.preview_rx.as_ref().map(|rx| rx.try_recv()) {
            self.preview_rx = None;
            match r {
                Ok(img) => {
                    let size = [img.width(), img.height()];
                    let ci = egui::ColorImage::from_rgba_unmultiplied([size[0] as usize, size[1] as usize], img.as_raw());
                    // dieselbe Seite in Graustufen (wie gedruckt) – Umschalten ohne neues Rendern
                    let mut g = img;
                    for p in g.pixels_mut() {
                        let l = ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as u8;
                        p[0] = l;
                        p[1] = l;
                        p[2] = l;
                    }
                    let gi = egui::ColorImage::from_rgba_unmultiplied([size[0] as usize, size[1] as usize], g.as_raw());
                    let opts = egui::TextureOptions::LINEAR;
                    self.preview = Some((key, ctx.load_texture("druckvorschau", ci, opts), ctx.load_texture("druckvorschau-sw", gi, opts), size));
                    self.preview_err = None;
                }
                Err(e) => self.preview_err = Some(e),
            }
        }
        self.want_preview(ctx);
        // laufender Auftrag fertig?
        if let Some(j) = &self.job {
            let g = j.lock().unwrap();
            if g.finished {
                self.result = Some(match &g.err {
                    Some(e) => format!("Drucken fehlgeschlagen: {e}"),
                    None if g.cancel => "Drucken abgebrochen".into(),
                    None => format!("{} Seite(n) an „{}“ gegeben", g.done, self.printers.get(self.printer).cloned().unwrap_or_default()),
                });
                return false;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.job.is_none();
        let busy = self.job.is_some();
        let screen = ctx.screen_rect();
        let size = vec2((screen.width() - 80.0).clamp(640.0, 940.0), (screen.height() - 80.0).clamp(440.0, 640.0));
        // abgedunkelter Hintergrund: das Druckfenster steht im Mittelpunkt
        egui::Area::new(egui::Id::new("druck-dimmer")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
            let r = ui.allocate_rect(screen, Sense::click());
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(110));
            if r.clicked() && !busy {
                close = true;
            }
        });
        egui::Area::new(egui::Id::new("druckfenster")).order(egui::Order::Foreground).fixed_pos(screen.center() - size / 2.0).show(ctx, |ui| {
            let frame = Rect::from_min_size(ui.cursor().min, size);
            let sh = egui::epaint::Shadow { offset: vec2(0.0, 10.0), blur: 40.0, spread: 0.0, color: Color32::from_black_alpha(120) };
            ui.painter().add(sh.as_shape(frame, 14.0));
            ui.painter().rect(frame, 14.0, th.bg, Stroke::new(1.0, th.line));
            ui.allocate_ui_at_rect(frame, |ui| {
                ui.set_clip_rect(frame);
                // ── Vorschau (links, dunkler Bereich) ──
                let side_w = 330.0;
                let left = Rect::from_min_max(frame.min, pos2(frame.right() - side_w, frame.bottom()));
                ui.painter().rect_filled(left, egui::Rounding { nw: 14.0, sw: 14.0, ne: 0.0, se: 0.0 }, th.panel);
                let area = Rect::from_min_max(left.min + vec2(28.0, 28.0), left.max - vec2(28.0, 70.0));
                let page_land = self.preview.as_ref().is_some_and(|p| p.3[0] > p.3[1]);
                let land = match self.orient {
                    1 => false,
                    2 => true,
                    _ => page_land,
                };
                let ratio = if land { 297.0 / 210.0 } else { 210.0 / 297.0 };
                let (mut paw, mut pah) = (area.width(), area.width() / ratio);
                if pah > area.height() {
                    pah = area.height();
                    paw = pah * ratio;
                }
                let paper = Rect::from_center_size(area.center(), vec2(paw, pah));
                let psh = egui::epaint::Shadow { offset: vec2(0.0, 4.0), blur: 18.0, spread: 0.0, color: Color32::from_black_alpha(90) };
                ui.painter().add(psh.as_shape(paper, 3.0));
                ui.painter().rect_filled(paper, 3.0, Color32::WHITE);
                match (&self.preview, &self.preview_err) {
                    (Some((key, tex, gray, sz)), _) if *key == (self.cur, self.page) => {
                        // Rand wie beim Drucker (bedruckbarer Bereich), Seite eingepasst
                        let inner = paper.shrink(paper.width() * 0.035);
                        let s = (inner.width() / sz[0] as f32).min(inner.height() / sz[1] as f32);
                        let r = Rect::from_center_size(inner.center(), vec2(sz[0] as f32 * s, sz[1] as f32 * s));
                        let t = if self.color == 1 { gray } else { tex };
                        ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    (_, Some(e)) => {
                        let g = ui.painter().layout(e.clone(), egui::FontId::proportional(12.5), Color32::from_gray(110), paper.width() - 40.0);
                        ui.painter().galley(paper.center() - g.size() / 2.0, g, Color32::from_gray(110));
                    }
                    _ => {
                        ui.put(Rect::from_center_size(paper.center(), vec2(26.0, 26.0)), egui::Spinner::new().color(Color32::from_gray(150)));
                    }
                }
                // Seiten blättern: Pille unter dem Blatt
                let n = self.src(self.cur).map_or(0, |s| s.pages());
                let pill = Rect::from_center_size(pos2(left.center().x, left.bottom() - 36.0), vec2(200.0, 34.0));
                ui.painter().rect(pill, 17.0, th.field, Stroke::new(1.0, th.line));
                let txt = if n > 0 { format!("Seite {} von {n}", self.page + 1) } else { format!("Seite {}", self.page + 1) };
                ui.painter().text(pill.center(), egui::Align2::CENTER_CENTER, txt, egui::FontId::proportional(13.0), th.text);
                let prev = Rect::from_center_size(pos2(pill.left() + 18.0, pill.center().y), vec2(30.0, 30.0));
                let nxt = Rect::from_center_size(pos2(pill.right() - 18.0, pill.center().y), vec2(30.0, 30.0));
                if chevron(ui, th, prev, false, self.page > 0) {
                    self.page -= 1;
                }
                if chevron(ui, th, nxt, true, n == 0 || self.page + 1 < n) {
                    self.page += 1;
                }
                if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) && self.page > 0 {
                    self.page -= 1;
                }
                if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) && (n == 0 || self.page + 1 < n) {
                    self.page += 1;
                }

                // ── Einstellungen (rechts) ──
                let right = Rect::from_min_max(pos2(frame.right() - side_w, frame.top()), frame.max).shrink2(vec2(24.0, 22.0));
                ui.allocate_ui_at_rect(right, |ui| {
                    ui.set_width(right.width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Drucken").size(22.0).strong().color(th.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (r, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().circle_filled(r.center(), 15.0, th.hover);
                            }
                            let c = r.center();
                            let s = Stroke::new(1.6, th.sub);
                            ui.painter().line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], s);
                            ui.painter().line_segment([c + vec2(5.0, -5.0), c + vec2(-5.0, 5.0)], s);
                            if resp.on_hover_text("Schließen (Esc)").clicked() && !busy {
                                close = true;
                            }
                        });
                    });
                    // Dateiname bzw. Anzahl
                    let name = if self.files.len() == 1 { self.files[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { format!("{} Dateien", self.files.len()) };
                    ui.label(egui::RichText::new(name).size(12.5).color(th.sub));
                    ui.add_space(14.0);
                    ui.add_enabled_ui(!busy, |ui| {
                        label(ui, th, "Drucker");
                        if self.printers.is_empty() {
                            ui.label(egui::RichText::new("Kein Drucker gefunden").color(th.danger));
                        } else {
                            let cur = self.printers[self.printer.min(self.printers.len() - 1)].clone();
                            egui::ComboBox::from_id_salt("drucker").width(right.width()).height(300.0).selected_text(egui::RichText::new(cur).size(13.5)).show_ui(ui, |ui| {
                                for (i, p) in self.printers.iter().enumerate() {
                                    ui.selectable_value(&mut self.printer, i, p);
                                }
                            });
                        }
                        ui.add_space(12.0);
                        label(ui, th, "Kopien");
                        ui.horizontal(|ui| {
                            if step(ui, th, "−", self.copies > 1) {
                                self.copies -= 1;
                            }
                            ui.add(egui::DragValue::new(&mut self.copies).range(1..=99).speed(0.1));
                            if step(ui, th, "+", self.copies < 99) {
                                self.copies += 1;
                            }
                        });
                        ui.add_space(12.0);
                        label(ui, th, "Seiten");
                        let mut all = self.range.trim().is_empty() && !self.range_mode;
                        let mut m = if all { 0u8 } else { 1 };
                        if segmented(ui, th, "seiten", &mut m, &["Alle", "Bereich"]) {
                            all = m == 0;
                            self.range_mode = !all;
                            if all {
                                self.range.clear();
                            }
                        }
                        if !all {
                            ui.add_space(6.0);
                            ui.add(egui::TextEdit::singleline(&mut self.range).hint_text("z. B. 1-3, 5").desired_width(right.width()));
                            let count = self.src(self.cur).map_or(0, |s| s.pages());
                            if let Err(e) = parse_range(&self.range, count) {
                                ui.label(egui::RichText::new(e).size(11.5).color(th.danger));
                            }
                        }
                        ui.add_space(12.0);
                        label(ui, th, "Ausrichtung");
                        segmented(ui, th, "ausrichtung", &mut self.orient, &["Automatisch", "Hoch", "Quer"]);
                        ui.add_space(12.0);
                        label(ui, th, "Farbe");
                        segmented(ui, th, "farbe", &mut self.color, &["Farbe", "Schwarzweiß"]);
                        // mehrere Dateien: Liste zum Durchsehen
                        if self.files.len() > 1 {
                            ui.add_space(12.0);
                            label(ui, th, "Dateien");
                            egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
                                for k in 0..self.files.len() {
                                    let name = self.files[k].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    let state = match &self.prep[k] {
                                        None => " – wird vorbereitet …".to_string(),
                                        Some(Err(_)) => " – nicht druckbar".to_string(),
                                        Some(Ok(s)) if s.pages() > 1 => format!(" – {} Seiten", s.pages()),
                                        _ => String::new(),
                                    };
                                    if ui.selectable_label(self.cur == k, format!("{name}{state}")).clicked() {
                                        self.cur = k;
                                        self.page = 0;
                                    }
                                }
                            });
                        }
                    });
                    if let Some(Some(Err(e))) = self.prep.get(self.cur) {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(e).size(12.0).color(th.danger));
                    }
                    // unten: Zusammenfassung, Fortschritt, Knöpfe
                    let bottom = right.bottom();
                    let ready = self.prep.iter().all(|p| p.is_some()) && self.prep.iter().any(|p| matches!(p, Some(Ok(_))));
                    let can = ready && !busy && !self.printers.is_empty() && parse_range(&self.range, 0).is_ok();
                    let sheets: u32 = self.prep.iter().flatten().flatten().map(|s| {
                        let n = s.pages().max(1);
                        parse_range(&self.range, n).ok().flatten().map_or(n, |v| v.len() as u32)
                    }).sum::<u32>() * self.copies.max(1);
                    let info = if !ready { "Wird vorbereitet …".to_string() } else { format!("{sheets} Blatt · {} · {}", ["Ausrichtung automatisch", "Hochformat", "Querformat"][self.orient.min(2) as usize], if self.color == 1 { "Schwarzweiß" } else { "Farbe" }) };
                    let info_pos = pos2(right.left(), bottom - 76.0);
                    if let Some(j) = &self.job {
                        let g = j.lock().unwrap();
                        let f = if g.total > 0 { g.done as f32 / g.total as f32 } else { 0.0 };
                        let bar = Rect::from_min_size(info_pos, vec2(right.width(), 6.0));
                        ui.painter().rect_filled(bar, 3.0, th.line);
                        ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, 6.0)), 3.0, th.accent);
                        ui.painter().text(bar.left_bottom() + vec2(0.0, 8.0), egui::Align2::LEFT_TOP, format!("Seite {} von {} wird gedruckt …", (g.done + 1).min(g.total.max(1)), g.total.max(1)), egui::FontId::proportional(12.0), th.sub);
                    } else {
                        ui.painter().text(info_pos, egui::Align2::LEFT_TOP, info, egui::FontId::proportional(12.5), th.sub);
                    }
                    let bw = (right.width() - 10.0) / 2.0;
                    let cancel_r = Rect::from_min_size(pos2(right.left(), bottom - 42.0), vec2(bw, 42.0));
                    let print_r = Rect::from_min_size(pos2(right.left() + bw + 10.0, bottom - 42.0), vec2(bw, 42.0));
                    let cr = ui.put(cancel_r, egui::Button::new(egui::RichText::new(if busy { "Abbrechen" } else { "Schließen" }).size(14.0)).stroke(Stroke::new(1.0, th.line)).rounding(10.0));
                    if cr.clicked() {
                        match &self.job {
                            Some(j) => j.lock().unwrap().cancel = true,
                            None => close = true,
                        }
                    }
                    let pb = egui::Button::new(egui::RichText::new("Drucken").size(14.0).strong().color(if can { th.on_accent() } else { th.sub })).fill(if can { th.accent } else { th.field }).rounding(10.0);
                    if ui.put(print_r, pb).clicked() && can {
                        self.start(ctx);
                    }
                    if can && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ui.memory(|m| m.focused().is_some()) {
                        self.start(ctx);
                    }
                });
            });
        });
        !close
    }

    /// nur für Bildschirmfotos: Schwarzweiß vorwählen
    pub fn dev_gray(&mut self) {
        self.color = 1;
    }

    fn start(&mut self, ctx: &egui::Context) {
        let files: Vec<(PathBuf, Source)> = self.files.iter().cloned().zip(self.prep.iter()).filter_map(|(f, p)| p.as_ref().and_then(|r| r.as_ref().ok()).map(|s| (f, s.clone()))).collect();
        let o = Opts { printer: self.printers.get(self.printer).cloned().unwrap_or_default(), copies: self.copies, range: self.range.clone(), orient: self.orient, gray: self.color == 1 };
        let prog = Arc::new(Mutex::new(Progress::default()));
        let (p2, ctx2) = (prog.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = run_job(files, o, p2.clone());
            let mut g = p2.lock().unwrap();
            g.err = r.err();
            g.finished = true;
            drop(g);
            ctx2.request_repaint();
        });
        self.job = Some(prog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_ranges() {
        assert!(!supported(Path::new("C:/x/ordner")));
        assert!(supported(Path::new("C:/x/brief.pdf")));
        assert!(supported(Path::new("C:/x/notiz.txt")));
        assert!(supported(Path::new("C:/x/liste.csv")));
        assert!(supported(Path::new("C:/x/brief.docx")));
        assert!(supported(Path::new("C:/x/foto.webp")));
        assert!(supported(Path::new("C:/x/main.rs")));
        assert!(!supported(Path::new("C:/x/lied.mp3")));
        assert_eq!(parse_range("", 5).unwrap(), None);
        assert_eq!(parse_range("1-3, 5", 9).unwrap(), Some(vec![0, 1, 2, 4]));
        assert_eq!(parse_range("2-", 4).unwrap(), Some(vec![1, 2, 3]));
        assert!(parse_range("0", 4).is_err() && parse_range("3-1", 4).is_err() && parse_range("x", 4).is_err());
    }
}

#[cfg(test)]
mod manual {
    /// ohne Papier: Markdown (eigener Umwandler) und Bild → „Microsoft Print to PDF“, Seiten zählen
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn print_to_pdf() {
        use super::*;
        let dir = std::env::temp_dir().join(format!("foldex-druck-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let md = dir.join("brief.md");
        std::fs::write(&md, "# Überschrift\n\nEin Absatz mit Umlauten: äöü ß.\n\n- Punkt eins\n- Punkt zwei\n").unwrap();
        let src = prepare(&md).expect("vorbereiten");
        let pages = src.pages();
        let out = dir.join("ergebnis.pdf");
        let hdc = gdi::open("Microsoft Print to PDF", false, true).expect("Drucker „Microsoft Print to PDF“");
        gdi::start(hdc, "Test", Some(&out)).unwrap();
        for p in 0..pages.max(1) {
            gdi::page(hdc, render(&src, p, 1240).unwrap(), true).unwrap();
        }
        let png = dir.join("bild.png");
        image::RgbaImage::from_fn(300, 200, |x, _| image::Rgba([x as u8, 80, 200, 255])).save(&png).unwrap();
        gdi::page(hdc, render(&Source::Image(png), 0, 1240).unwrap(), false).unwrap();
        gdi::finish(hdc);
        let t0 = std::time::Instant::now();
        while std::fs::metadata(&out).map_or(0, |m| m.len()) == 0 && t0.elapsed().as_secs() < 20 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let n = std::fs::metadata(&out).map_or(0, |m| m.len());
        let back = page_count(&out).unwrap_or(0);
        println!("Markdown → {pages} Seite(n); gedrucktes PDF: {n} Bytes, {back} Seite(n); Drucker: {:?}", printers());
        if let Source::Pdf { path, temp: true, .. } = &src {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(n > 1000 && back == pages.max(1) + 1);
    }
}
