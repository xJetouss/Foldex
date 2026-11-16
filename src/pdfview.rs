//! PDF-Vorschau: alle Seiten untereinander, scharf in der Breite der Vorschau. Gerendert wird mit dem in
//! Windows eingebauten Renderer (Windows.Data.Pdf) in einem eigenen Thread, und nur, was sichtbar ist (plus
//! eine Seite Vorlauf). Seiten weit außerhalb der Sicht werden wieder freigegeben.
//!
//! Kopieren: Text der aktuellen bzw. aller Seiten (mit pdftotext, falls installiert – sonst über die
//! Texterkennung von Windows aus dem Seitenbild) und die aktuelle Seite als Bild.

use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, Rect, Stroke, TextureHandle};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    time::Instant,
};

/// breiteste gerenderte Seite in Pixeln
const MAX_PX: u32 = 2000;
const GAP: f32 = 10.0;
/// Seitenbreite für die Texterkennung (größer = genauer, besonders bei kleiner Schrift)
const OCR_PX: u32 = 2400;

enum Req {
    Render(u32, u32),
    /// Text kopieren: eine Seite oder alle
    Text(Option<u32>),
    Image(u32),
    /// Texterkennung einer Seite (Scan oder ohne pdftotext)
    Ocr(u32),
}

enum Msg {
    /// Seitengrößen in Punkten
    Info(Vec<[f32; 2]>),
    Page(u32, u32, egui::ColorImage),
    Err(String),
    /// Text zum Kopieren bzw. warum nicht
    Text(Result<String, String>),
    Note(String),
    Words(u32, Vec<crate::ocr::Word>),
    BboxDone,
}

pub struct PdfView {
    req: Sender<Req>,
    rx: Receiver<Msg>,
    pub pages: Vec<[f32; 2]>,
    pub err: Option<String>,
    /// Seite → (Breite in Pixeln, Textur)
    tex: HashMap<u32, (u32, TextureHandle)>,
    /// angefordert: (Seite, Breite)
    pending: HashSet<(u32, u32)>,
    /// Seite in der Mitte der Ansicht (ab 0)
    current: usize,
    /// Rückmeldung in der Kopfzeile; `busy` = Kopieren läuft
    note: Option<(String, Instant)>,
    busy: bool,
    /// erkannte bzw. gelesene Wörter je Seite (markierbarer Text)
    words: HashMap<u32, Vec<crate::ocr::Word>>,
    ocr_pending: HashSet<u32>,
    /// pdftotext -bbox ist fertig (oder fehlt): Seiten ohne Wörter bekommen die Texterkennung
    bbox_done: bool,
    /// Markierung: Seite und Wörter
    sel: (Option<u32>, crate::ocr::Selection),
}

impl PdfView {
    pub fn open(path: &Path, ctx: &egui::Context) -> PdfView {
        let (req, req_rx) = mpsc::channel::<Req>();
        let (tx, rx) = mpsc::channel();
        let (p, c) = (path.to_path_buf(), ctx.clone());
        let tx1 = tx.clone();
        std::thread::spawn(move || worker(p, req_rx, tx1, c));
        // nur für Bildschirmfotos (FOLDEX_SHOT): FOLDEX_SHOT_PDFCOPY=1 (bzw. =img) kopiert gleich Text (bzw. Bild) von Seite 1
        if std::env::var_os("FOLDEX_SHOT").is_some() && std::env::var_os("FOLDEX_SHOT_PDFCOPY").is_some() {
            let img = std::env::var("FOLDEX_SHOT_PDFCOPY").is_ok_and(|v| v == "img");
            let _ = req.send(if img { Req::Image(0) } else { Req::Text(Some(0)) });
        }
        // Wortlagen aller Seiten (für markierbaren Text): mit pdftotext genau; sonst später Texterkennung je Seite
        let bbox = crate::docconv::pdftotext().filter(|_| std::env::var_os("FOLDEX_SHOT_OCR").is_none());
        let bbox_done = bbox.is_none();
        if let Some(pt) = bbox {
            let (p, tx2, c) = (path.to_path_buf(), tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                if let Ok(pages) = crate::ocr::pdf_words(&pt, &p) {
                    for (i, w) in pages.into_iter().enumerate() {
                        if !w.is_empty() && tx2.send(Msg::Words(i as u32, w)).is_err() {
                            return;
                        }
                    }
                }
                let _ = tx2.send(Msg::BboxDone);
                c.request_repaint();
            });
        }
        PdfView {
            req,
            rx,
            pages: Vec::new(),
            err: None,
            tex: HashMap::new(),
            pending: HashSet::new(),
            current: 0,
            note: None,
            busy: false,
            words: HashMap::new(),
            ocr_pending: HashSet::new(),
            bbox_done,
            sel: (None, crate::ocr::Selection::default()),
        }
    }

    /// Seiten anzeigen (füllt `avail` aus, scrollt senkrecht).
    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme, avail: egui::Vec2) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Info(p) => self.pages = p,
                Msg::Page(i, w, img) => {
                    self.pending.remove(&(i, w));
                    let t = ui.ctx().load_texture(format!("pdf{i}"), img, egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR });
                    self.tex.insert(i, (w, t));
                }
                Msg::Err(e) => self.err = Some(e),
                Msg::Text(r) => {
                    self.busy = false;
                    let msg = match r {
                        Ok(t) if t.trim().is_empty() => "Kein Text gefunden".to_string(),
                        Ok(t) => {
                            ui.ctx().copy_text(t);
                            "Text kopiert".to_string()
                        }
                        Err(e) => e,
                    };
                    self.note = Some((msg, Instant::now()));
                }
                Msg::Note(n) => {
                    self.busy = false;
                    self.note = Some((n, Instant::now()));
                }
                Msg::Words(i, w) => {
                    // nur für Bildschirmfotos: FOLDEX_SHOT_SELECT=1 markiert die erste Seite und kopiert ihren Text
                    if i == 0 && std::env::var_os("FOLDEX_SHOT").is_some() && std::env::var_os("FOLDEX_SHOT_SELECT").is_some() {
                        self.sel = (Some(0), crate::ocr::Selection::all(w.len()));
                        ui.ctx().copy_text(crate::ocr::all_text(&w));
                    }
                    self.words.insert(i, w);
                }
                Msg::BboxDone => self.bbox_done = true,
            }
        }
        if let Some(e) = &self.err {
            ui.label(egui::RichText::new(format!("PDF kann nicht angezeigt werden: {e}")).color(th.danger));
            return;
        }
        if self.pages.is_empty() {
            ui.spinner();
            return;
        }
        let n = self.pages.len();
        // Kopfzeile: aktuelle Seite, Kopieren
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("Seite {} von {n}", self.current + 1)).size(12.0).color(th.sub));
            if self.busy {
                ui.spinner();
            }
            if let Some((msg, at)) = &self.note {
                if at.elapsed().as_secs_f32() < 3.0 {
                    ui.label(egui::RichText::new(msg).size(12.0).color(th.accent));
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let resp = crate::ui::cmd_btn(ui, th, Some(crate::ui::Ic::Copy), "Kopieren", !self.busy, true);
                let pid = ui.id().with("pdf_copy");
                if resp.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(pid));
                }
                let cur = self.current as u32;
                egui::popup_below_widget(ui, pid, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    ui.set_min_width(220.0);
                    let mut send = |r: Req| {
                        self.busy = true;
                        let _ = self.req.send(r);
                    };
                    if ui.button(format!("Text von Seite {}", cur + 1)).clicked() {
                        send(Req::Text(Some(cur)));
                    }
                    if ui.button("Text aller Seiten").clicked() {
                        send(Req::Text(None));
                    }
                    if ui.button(format!("Seite {} als Bild", cur + 1)).clicked() {
                        send(Req::Image(cur));
                    }
                });
            });
        });
        let h = (ui.available_height()).max(60.0);
        let (area, _) = ui.allocate_exact_size(vec2(avail.x, h), egui::Sense::hover());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
        let w = (area.width() - 14.0).max(40.0);
        let tops: Vec<f32> = self
            .pages
            .iter()
            .scan(0.0, |y, p| {
                let t = *y;
                *y += w * p[1] / p[0].max(1.0) + GAP;
                Some(t)
            })
            .collect();
        let total = tops.last().copied().unwrap_or(0.0) + w * self.pages[n - 1][1] / self.pages[n - 1][0].max(1.0);
        let px = ((w * ui.ctx().pixels_per_point()).round() as u32).clamp(64, MAX_PX);
        let mut current = self.current;
        let mut visible = (u32::MAX, 0u32);
        let mut copied = None;
        // Ziehen markiert Text (statt zu scrollen)
        egui::ScrollArea::vertical().id_salt("pv_pdf").auto_shrink([false, false]).drag_to_scroll(false).show_viewport(&mut child, |ui, vp| {
            ui.set_min_height(total);
            ui.set_min_width(ui.available_width());
            let origin = ui.max_rect().min;
            let first = tops.partition_point(|&y| y <= vp.min.y).saturating_sub(1);
            current = tops.partition_point(|&y| y <= vp.center().y).saturating_sub(1);
            for i in first..n {
                let y = tops[i];
                if y > vp.max.y {
                    break;
                }
                let ph = w * self.pages[i][1] / self.pages[i][0].max(1.0);
                let r = Rect::from_min_size(origin + vec2(0.0, y), vec2(w, ph));
                // Schatten und weißes Blatt, darauf die Seite (notfalls in niedriger Auflösung, bis die scharfe da ist)
                ui.painter().rect_filled(r.translate(vec2(0.0, 2.0)), 2.0, Color32::from_black_alpha(40));
                ui.painter().rect_filled(r, 2.0, Color32::WHITE);
                match self.tex.get(&(i as u32)) {
                    Some((_, t)) => {
                        ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.put(Rect::from_center_size(r.center(), vec2(22.0, 22.0)), egui::Spinner::new().color(Color32::from_gray(150)));
                    }
                }
                ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, th.line));
                // markierbarer Text der Seite; die Markierung gilt immer für eine Seite
                if let Some(ws) = self.words.get(&(i as u32)) {
                    let mine = self.sel.0 == Some(i as u32);
                    let mut s = if mine { self.sel.1 } else { crate::ocr::Selection::default() };
                    if let Some(m) = crate::ocr::text_layer(ui, th, ui.id().with(("pdf_text", i)), r, ws, &mut s) {
                        copied = Some(m);
                    }
                    if mine || s.range().is_some() {
                        self.sel = (Some(i as u32), s);
                    }
                }
                visible = (visible.0.min(i as u32), visible.1.max(i as u32));
            }
        });
        if let Some(m) = copied {
            self.note = Some((m, Instant::now()));
        }
        self.current = current;
        if visible.0 != u32::MAX {
            // sichtbare Seiten (und je eine davor/danach) in der passenden Breite anfordern
            let (a, b) = (visible.0.saturating_sub(1), (visible.1 + 1).min(n as u32 - 1));
            for i in a..=b {
                let have = self.tex.get(&i).map(|t| t.0);
                let stale = have.is_none_or(|hw| hw < px * 9 / 10 || hw > px * 2);
                if stale && !self.pending.contains(&(i, px)) {
                    self.pending.insert((i, px));
                    let _ = self.req.send(Req::Render(i, px));
                }
            }
            // Seiten ohne Text (Scan) bzw. ohne pdftotext: Texterkennung der sichtbaren Seiten
            if self.bbox_done {
                for i in visible.0..=visible.1 {
                    if !self.words.contains_key(&i) && self.ocr_pending.insert(i) {
                        let _ = self.req.send(Req::Ocr(i));
                    }
                }
            }
            // weit entfernte Seiten freigeben
            self.tex.retain(|i, _| *i + 4 >= a && *i <= b + 4);
        }
    }
}

#[cfg(windows)]
fn worker(path: PathBuf, req: Receiver<Req>, tx: Sender<Msg>, ctx: egui::Context) {
    use windows::core::HSTRING;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let doc = (|| -> windows::core::Result<PdfDocument> {
        // WinRT verlangt einen absoluten Pfad mit Backslashes
        let full = std::path::absolute(&path).unwrap_or(path.clone()).to_string_lossy().replace('/', "\\");
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(full))?.join()?;
        PdfDocument::LoadFromFileAsync(&file)?.join()
    })();
    let doc = match doc {
        Ok(d) => d,
        Err(e) => {
            let msg = if doc_is_protected(&e) { "Das PDF ist kennwortgeschützt".to_string() } else { e.message().to_string() };
            let _ = tx.send(Msg::Err(msg));
            ctx.request_repaint();
            return;
        }
    };
    let count = doc.PageCount().unwrap_or(0);
    let sizes: Vec<[f32; 2]> = (0..count).map(|i| doc.GetPage(i).and_then(|p| p.Size()).map_or([595.0, 842.0], |s| [s.Width.max(1.0), s.Height.max(1.0)])).collect();
    if sizes.is_empty() {
        let _ = tx.send(Msg::Err("keine Seiten".into()));
        ctx.request_repaint();
        return;
    }
    let _ = tx.send(Msg::Info(sizes));
    ctx.request_repaint();
    // Seite als PNG in einen Speicher-Stream rendern
    let render = |i: u32, w: u32| -> windows::core::Result<InMemoryRandomAccessStream> {
        let page = doc.GetPage(i)?;
        let opts = PdfPageRenderOptions::new()?;
        opts.SetDestinationWidth(w)?;
        let stream = InMemoryRandomAccessStream::new()?;
        page.RenderWithOptionsToStreamAsync(&stream, &opts)?.join()?;
        Ok(stream)
    };
    let bytes = |stream: &InMemoryRandomAccessStream| -> windows::core::Result<Vec<u8>> {
        let n = stream.Size()? as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        reader.LoadAsync(n)?.join()?;
        let mut buf = vec![0u8; n as usize];
        reader.ReadBytes(&mut buf)?;
        Ok(buf)
    };
    // Seite als Bild (für Kopieren und Texterkennung)
    let page_img = |i: u32, w: u32| -> Result<image::DynamicImage, String> {
        let b = render(i, w).and_then(|s| bytes(&s)).map_err(|e| e.message().to_string())?;
        image::load_from_memory(&b).map_err(|e| e.to_string())
    };
    let send = |m: Msg| {
        let ok = tx.send(m).is_ok();
        ctx.request_repaint();
        ok
    };
    while let Ok(first) = req.recv() {
        // aufgelaufene Wünsche zusammenfassen; je Seite zählt die neueste Breite
        let mut queue = vec![first];
        queue.extend(req.try_iter());
        let mut seen = HashSet::new();
        let mut renders: Vec<(u32, u32)> = Vec::new();
        for r in queue.into_iter().rev() {
            match r {
                Req::Render(i, w) => {
                    if seen.insert(i) {
                        renders.insert(0, (i, w));
                    }
                }
                Req::Text(page) => {
                    let pages: Vec<u32> = page.map_or_else(|| (0..count).collect(), |p| vec![p]);
                    // genau mit pdftotext, sonst Texterkennung Seite für Seite
                    let r = pdftotext(&path, page).or_else(|_| {
                        let mut out = Vec::new();
                        for &i in &pages {
                            out.push(crate::ocr::all_text(&page_img(i, OCR_PX).and_then(|img| crate::ocr::recognize(&img))?));
                        }
                        Ok(out.join("\n\n"))
                    });
                    if !send(Msg::Text(r)) {
                        return;
                    }
                }
                Req::Ocr(i) => {
                    // auch ohne Treffer melden, damit die Seite nicht erneut angefragt wird
                    let w = page_img(i, OCR_PX).and_then(|img| crate::ocr::recognize(&img)).unwrap_or_default();
                    if !send(Msg::Words(i, w)) {
                        return;
                    }
                }
                Req::Image(i) => {
                    let r = page_img(i, MAX_PX).and_then(|img| crate::clip::set_image(&img));
                    if !send(Msg::Note(match r {
                        Ok(()) => format!("Seite {} als Bild kopiert", i + 1),
                        Err(e) => format!("Kopieren fehlgeschlagen: {e}"),
                    })) {
                        return;
                    }
                }
            }
        }
        for (i, w) in renders {
            if let Some(img) = render(i, w).and_then(|s| bytes(&s)).ok().and_then(|b| image::load_from_memory(&b).ok()) {
                let rgba = img.to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                if !send(Msg::Page(i, w, ci)) {
                    return; // Vorschau geschlossen
                }
            }
        }
    }
}

/// Text mit pdftotext (Poppler), wenn installiert: genau und schnell. `page` ab 0.
/// Seiten eines PDFs nacheinander als Bild in der Breite `w` rendern (zum Drucken). `f` bekommt
/// (Seite, Seitenzahl, Bild) und gibt false zurück, um abzubrechen.
#[cfg(windows)]
pub fn each_page(path: &Path, w: u32, f: impl FnMut(u32, u32, image::RgbaImage) -> bool) -> Result<u32, String> {
    pdf_pages(path, w, |_| true, f)
}

/// wie `each_page`, rendert aber nur die Seiten, für die `want` true liefert (0-basiert). Ohne gewünschte
/// Seiten (`want` immer false) liefert es nur die Seitenzahl.
#[cfg(windows)]
pub fn pdf_pages(path: &Path, w: u32, want: impl Fn(u32) -> bool, mut f: impl FnMut(u32, u32, image::RgbaImage) -> bool) -> Result<u32, String> {
    use windows::core::HSTRING;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let me = |e: windows::core::Error| if doc_is_protected(&e) { "Das PDF ist kennwortgeschützt".to_string() } else { e.message().to_string() };
    let full = std::path::absolute(path).unwrap_or(path.to_path_buf()).to_string_lossy().replace('/', "\\");
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(full)).and_then(|a| a.join()).map_err(me)?;
    let doc = PdfDocument::LoadFromFileAsync(&file).and_then(|a| a.join()).map_err(me)?;
    let count = doc.PageCount().map_err(me)?;
    for i in (0..count).filter(|i| want(*i)) {
        let img = (|| -> windows::core::Result<Vec<u8>> {
            let page = doc.GetPage(i)?;
            let opts = PdfPageRenderOptions::new()?;
            opts.SetDestinationWidth(w)?;
            let stream = InMemoryRandomAccessStream::new()?;
            page.RenderWithOptionsToStreamAsync(&stream, &opts)?.join()?;
            let n = stream.Size()? as u32;
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
            reader.LoadAsync(n)?.join()?;
            let mut buf = vec![0u8; n as usize];
            reader.ReadBytes(&mut buf)?;
            Ok(buf)
        })()
        .map_err(me)?;
        let img = image::load_from_memory(&img).map_err(|e| e.to_string())?.to_rgba8();
        if !f(i, count, img) {
            break;
        }
    }
    Ok(count)
}

fn pdftotext(path: &Path, page: Option<u32>) -> Result<String, String> {
    // nur für Bildschirmfotos: FOLDEX_SHOT_OCR=1 testet den Weg über die Texterkennung
    if std::env::var_os("FOLDEX_SHOT").is_some() && std::env::var_os("FOLDEX_SHOT_OCR").is_some() {
        return Err("pdftotext übersprungen".into());
    }
    let pt = crate::docconv::pdftotext().ok_or("pdftotext fehlt")?;
    let mut c = crate::convert::quiet(&pt);
    c.args(["-layout", "-enc", "UTF-8"]);
    if let Some(p) = page {
        c.args(["-f", &(p + 1).to_string(), "-l", &(p + 1).to_string()]);
    }
    let out = c.arg(path).arg("-").output().map_err(|e| e.to_string())?;
    let t = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() && !t.trim().is_empty() {
        Ok(t)
    } else {
        Err("pdftotext lieferte keinen Text".into())
    }
}

#[cfg(windows)]
fn doc_is_protected(e: &windows::core::Error) -> bool {
    // HRESULT_FROM_WIN32(ERROR_INVALID_PASSWORD)
    e.code().0 as u32 == 0x8007_0056
}

#[cfg(not(windows))]
fn worker(_path: PathBuf, _req: Receiver<Req>, tx: Sender<Msg>, ctx: egui::Context) {
    let _ = tx.send(Msg::Err("PDF-Vorschau gibt es nur unter Windows".into()));
    ctx.request_repaint();
}
