//! Bildeditor im großen Preview (Vorschau als eigener Tab). Werkzeuge mit Icons (Verschieben/Zoomen, Auswahl,
//! Zuschneiden mit Seitenverhältnis, Pinsel, Radierer, Linie, Pfeil, Rechteck, Ellipse, Text, Füllen, Pipette,
//! Weichzeichnen und Verpixeln von Bereichen), Anpassungen mit Live-Vorschau (Belichtung, Helligkeit, Kontrast,
//! Gamma, Sättigung, Dynamik, Farbton, Temperatur, Tönung, Schärfen, Weichzeichnen, Vignette), Filter
//! (Graustufen, Sepia, Invertieren, Auto-Kontrast, Auto-Weißabgleich, Schärfen, Weichzeichnen, Posterisieren),
//! Drehen/Spiegeln, Größe und Arbeitsfläche ändern, Histogramm, Rückgängig/Wiederholen und Speichern
//! (PNG, JPEG mit Qualität, WebP, BMP, TIFF, TGA, QOI).
//!
//! Gerechnet wird in voller Auflösung, parallel (rayon). Hochgeladen wird nur der geänderte Bereich; die
//! Anpassungs-Vorschau rechnet auf einer verkleinerten Kopie.

use crate::edicons::{self, E};
use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, Key, Pos2, Rect, Stroke, TextureHandle, Vec2};
use image::{Rgba, RgbaImage};
use rayon::prelude::*;
use crate::localai::{self, Bg, Chain};
use image::GrayImage;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Move,
    Select,
    Crop,
    Brush,
    Eraser,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Fill,
    Picker,
    Blur,
    Pixelate,
    Objects,
}

/// Werkzeug, Icon, Name, Taste, Hinweis
const TOOLS: &[(Tool, E, &str, Key, &str)] = &[
    (Tool::Move, E::Move, "Verschieben", Key::V, "Ansicht ziehen, Mausrad zoomt"),
    (Tool::Select, E::Select, "Auswahl", Key::M, "Rechteck auswählen – Aktionen rechts"),
    (Tool::Crop, E::Crop, "Zuschneiden", Key::C, "Bereich ziehen, Enter schneidet zu"),
    (Tool::Brush, E::Brush, "Pinsel", Key::B, "Freihand malen"),
    (Tool::Eraser, E::Eraser, "Radierer", Key::E, "Radieren (durchsichtig)"),
    (Tool::Line, E::Line, "Linie", Key::L, "Gerade Linie (Umschalt: 45°-Schritte)"),
    (Tool::Arrow, E::Arrow, "Pfeil", Key::A, "Pfeil zum Markieren"),
    (Tool::Rect, E::Rect, "Rechteck", Key::R, "Rechteck (Umschalt: Quadrat)"),
    (Tool::Ellipse, E::Ellipse, "Ellipse", Key::O, "Ellipse (Umschalt: Kreis)"),
    (Tool::Text, E::Text, "Text", Key::T, "Ins Bild klicken, Text rechts eingeben"),
    (Tool::Fill, E::Fill, "Füllen", Key::G, "Fläche mit Farbe füllen"),
    (Tool::Picker, E::Picker, "Pipette", Key::I, "Farbe aus dem Bild aufnehmen"),
    (Tool::Blur, E::Blur, "Weichzeichnen", Key::U, "Bereich ziehen: weichzeichnen"),
    (Tool::Pixelate, E::Pixelate, "Verpixeln", Key::P, "Bereich ziehen: unkenntlich machen"),
    (Tool::Objects, E::Wand, "Objekte auswählen", Key::W, "KI: Objekt anklicken – weitere Klicks fügen Teile hinzu"),
];

/// Anpassungen; alles 0 = unverändert
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct Adjust {
    pub exposure: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub vibrance: f32,
    pub hue: f32,
    pub temperature: f32,
    pub tint: f32,
    pub gamma: f32,
    pub sharpen: f32,
    pub blur: f32,
    pub vignette: f32,
}

/// Ganzzahliges Rechteck in Bildpunkten (x0, y0, x1, y1; x1/y1 exklusiv)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct IRect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl IRect {
    fn full(w: u32, h: u32) -> IRect {
        IRect { x0: 0, y0: 0, x1: w, y1: h }
    }
    fn union(self, o: IRect) -> IRect {
        IRect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    /// aus Gleitkomma-Bildkoordinaten, auf das Bild begrenzt
    fn from_f(a: Pos2, b: Pos2, w: u32, h: u32) -> IRect {
        let (x0, x1) = (a.x.min(b.x).floor().max(0.0) as u32, a.x.max(b.x).ceil().max(0.0) as u32);
        let (y0, y1) = (a.y.min(b.y).floor().max(0.0) as u32, a.y.max(b.y).ceil().max(0.0) as u32);
        IRect { x0: x0.min(w), y0: y0.min(h), x1: x1.min(w), y1: y1.min(h) }
    }
}

/// Höchstens so viel Speicher für Rückgängig-Schritte
const HISTORY_BYTES: usize = 768 * 1024 * 1024;

pub enum EditOut {
    /// Editor schließen
    Close,
    Saved(PathBuf),
}

pub struct ImageEditor {
    path: PathBuf,
    w: u32,
    h: u32,
    img: RgbaImage,
    tex: Option<TextureHandle>,
    /// neu hochzuladen (None = nichts)
    dirty: Option<IRect>,
    /// Änderungszähler (Vorschau, Histogramm)
    ver: u64,
    tool: Tool,
    color: Color32,
    color2: Color32,
    size: f32,
    hardness: f32,
    flow: f32,
    filled: bool,
    tolerance: f32,
    font_px: f32,
    text: String,
    text_at: Option<Pos2>,
    zoom: f32,
    pan: Vec2,
    fit_next: bool,
    undo: Vec<RgbaImage>,
    redo: Vec<RgbaImage>,
    /// Pinsel: letzter Punkt; Formen/Auswahl/Zuschneiden: Startpunkt (Bildkoordinaten)
    last: Option<Pos2>,
    drag_from: Option<Pos2>,
    sel: Option<IRect>,
    crop: Option<IRect>,
    crop_ratio: Option<f32>,
    adjust: Adjust,
    adj_preview: bool,
    /// Vorschau der Anpassungen: (Version, verkleinertes Bild, Maßstab) und Textur
    small: Option<(u64, RgbaImage, f32)>,
    adj_tex: Option<(Adjust, u64, TextureHandle)>,
    hist: Option<(u64, [[u32; 256]; 3])>,
    resize: Option<(u32, u32, bool)>,
    canvas: Option<(u32, u32)>,
    jpeg_q: u8,
    pub modified: bool,
    msg: Option<(bool, String)>,
    font: Option<ab_glyph::FontVec>,
    cursor_img: Option<Pos2>,
    // ── lokale KI ──
    ai: Option<AiJob>,
    /// Rückfrage, bevor Modelle geladen werden
    ai_ask: Option<(AiTask, Vec<&'static localai::Asset>)>,
    /// Objektmaske (255 = ausgewählt), Klickpunkte (true = dazu) und ihre Überlagerung
    mask: Option<GrayImage>,
    mask_tex: Option<TextureHandle>,
    points: Vec<(Pos2, bool)>,
    points_pending: bool,
    /// kodiertes Bild für die Objektauswahl (gilt für diese Version)
    embed: Option<(u64, Arc<localai::SamEmbed>)>,
    /// grobe Maske des letzten Klicks (verfeinert den nächsten)
    sam_low: Option<Vec<f32>>,
    /// Breite der rechten Spalte (ziehbar)
    panel_w: f32,
    chain: Chain,
    /// „Objekt entfernen“: Lücke aus der Umgebung füllen statt durchsichtig
    obj_fill: bool,
    /// KI-Bearbeitung per Text: Anweisung und gewähltes Modell (Index in `localllm::EDIT`)
    txt_prompt: String,
    txt_model: usize,
}

#[derive(Clone)]
enum AiTask {
    Chain(Chain),
    /// Hauptmotiv als Objektmaske
    Subject,
    /// Maske aus den Klickpunkten
    Points,
    /// Objekt entfernen und die Stelle neu erzeugen
    Inpaint,
    /// Bild nach Text-Anweisung ändern (Modell, Anweisung)
    TextEdit(usize, String),
}

/// zuletzt gewählte Breite der rechten Spalte (für alle Editoren dieser Sitzung)
static PANEL_W: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(320);

enum AiOut {
    Image(RgbaImage),
    Mask(GrayImage, Option<(u64, Arc<localai::SamEmbed>)>, Option<Vec<f32>>),
}

struct AiJob {
    what: String,
    prog: localai::Progress,
    rx: mpsc::Receiver<Result<AiOut, String>>,
    /// danach die Objektmaske verwerfen (das Objekt ist weg)
    clear_mask: bool,
}

impl Drop for ImageEditor {
    fn drop(&mut self) {
        if let Some(j) = &self.ai {
            j.prog.cancel();
        }
        // Modelle belegen einige hundert MB
        std::thread::spawn(localai::unload);
    }
}

fn c32(c: Rgba<u8>) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

impl ImageEditor {
    pub fn open(path: &Path) -> Result<ImageEditor, String> {
        let img = crate::convert::load_image(path, 4096)?.to_rgba8();
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return Err("Bild ist leer".into());
        }
        if w > 16384 || h > 16384 {
            return Err(format!("Zu groß zum Bearbeiten ({w} × {h}, höchstens 16384 px je Seite)"));
        }
        Ok(ImageEditor {
            path: path.to_path_buf(),
            w,
            h,
            img,
            tex: None,
            dirty: Some(IRect::full(w, h)),
            ver: 1,
            tool: Tool::Brush,
            color: Color32::from_rgb(230, 57, 70),
            color2: Color32::WHITE,
            size: 8.0,
            hardness: 0.8,
            flow: 1.0,
            filled: false,
            tolerance: 32.0,
            font_px: 48.0,
            text: String::new(),
            text_at: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit_next: true,
            undo: Vec::new(),
            redo: Vec::new(),
            last: None,
            drag_from: None,
            sel: None,
            crop: None,
            crop_ratio: None,
            adjust: Adjust::default(),
            adj_preview: true,
            small: None,
            adj_tex: None,
            hist: None,
            resize: None,
            canvas: None,
            jpeg_q: 90,
            modified: false,
            msg: None,
            font: load_font(),
            cursor_img: None,
            ai: None,
            ai_ask: None,
            mask: None,
            mask_tex: None,
            points: Vec::new(),
            points_pending: false,
            embed: None,
            sam_low: None,
            panel_w: PANEL_W.load(std::sync::atomic::Ordering::Relaxed) as f32,
            chain: Chain::default(),
            obj_fill: true,
            txt_prompt: String::new(),
            txt_model: 0,
        })
    }

    /// Nur für Bildschirmfotos: ein paar Bearbeitungen vorführen (Pfeil, Rahmen, Text, Auswahl, Anpassung in
    /// der Vorschau).
    pub fn dev_demo(&mut self) {
        let (w, h) = (self.w as f32, self.h as f32);
        // FOLDEX_SHOT_EDITOR=ai: Objektauswahl mit Maske und die Rückfrage vor dem Laden der Modelle
        if std::env::var("FOLDEX_SHOT_EDITOR").as_deref() == Ok("ai") {
            self.tool = Tool::Objects;
            let (cx, cy, rx, ry) = (w * 0.5, h * 0.55, w * 0.22, h * 0.3);
            self.set_mask(GrayImage::from_fn(self.w, self.h, |x, y| {
                let d = ((x as f32 - cx) / rx).powi(2) + ((y as f32 - cy) / ry).powi(2);
                image::Luma([if d < 1.0 { 255 } else { 0 }])
            }));
            self.points = vec![(pos2(cx, cy), true), (pos2(w * 0.15, h * 0.2), false)];
            let miss = localai::missing(&[localai::Need::Inpaint, localai::Need::Objects]);
            if !miss.is_empty() {
                self.ai_ask = Some((AiTask::Inpaint, miss));
            }
            return;
        }
        self.push_undo();
        let red = [230, 57, 70, 255];
        let img = &mut self.img;
        draw_line(img, pos2(w * 0.1, h * 0.85), pos2(w * 0.45, h * 0.55), 6.0, red);
        for (x, y) in arrow_head(pos2(w * 0.1, h * 0.85), pos2(w * 0.45, h * 0.55), 6.0) {
            draw_line(img, pos2(w * 0.45, h * 0.55), pos2(x, y), 6.0, red);
        }
        let (a, b) = (pos2(w * 0.04, h * 0.06), pos2(w * 0.6, h * 0.25));
        let (p0, p1, p2, p3) = (a, pos2(b.x, a.y), b, pos2(a.x, b.y));
        for (s, e) in [(p0, p1), (p1, p2), (p2, p3), (p3, p0)] {
            draw_line(img, s, e, 4.0, red);
        }
        if let Some(f) = self.font.as_ref() {
            draw_text(&mut self.img, f, "Geprüft", pos2(w * 0.55, h * 0.7), (h * 0.09).max(14.0), [20, 120, 60, 255]);
        }
        self.sel = Some(IRect { x0: (w * 0.62) as u32, y0: (h * 0.05) as u32, x1: (w * 0.95) as u32, y1: (h * 0.4) as u32 });
        self.adjust.saturation = 0.4;
        self.touch(IRect::full(self.w, self.h));
    }

    // ───────────────────────── Verlauf ─────────────────────────

    fn push_undo(&mut self) {
        self.undo.push(self.img.clone());
        self.redo.clear();
        // Speicher begrenzen: älteste Schritte verwerfen
        while self.undo.len() > 1 && self.undo.iter().map(|i| i.as_raw().len()).sum::<usize>() > HISTORY_BYTES {
            self.undo.remove(0);
        }
        self.modified = true;
    }

    fn restore(&mut self, img: RgbaImage) {
        self.img = img;
        let (w, h) = self.img.dimensions();
        self.resized(w, h);
        self.modified = true;
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.img, RgbaImage::new(1, 1)));
            self.restore(s);
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.img, RgbaImage::new(1, 1)));
            self.restore(s);
        }
    }

    /// nach Größenänderung (Zuschneiden, Drehen, Größe): alles neu
    fn resized(&mut self, w: u32, h: u32) {
        if (w, h) != (self.w, self.h) {
            // Maske passt nicht mehr
            self.drop_mask();
        }
        self.w = w;
        self.h = h;
        self.tex = None;
        self.sel = None;
        self.crop = None;
        self.resize = None;
        self.canvas = None;
        self.touch(IRect::full(w, h));
    }

    fn touch(&mut self, r: IRect) {
        let r = IRect { x0: r.x0.min(self.w), y0: r.y0.min(self.h), x1: r.x1.min(self.w), y1: r.y1.min(self.h) };
        if r.is_empty() {
            return;
        }
        self.dirty = Some(self.dirty.map_or(r, |d| d.union(r)));
        self.ver += 1;
    }

    /// geänderten Bereich hochladen (ganz oder nur das Rechteck)
    fn upload(&mut self, ctx: &egui::Context) {
        let Some(r) = self.dirty.take() else { return };
        let opts = egui::TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Linear, ..Default::default() };
        match &mut self.tex {
            Some(t) if r != IRect::full(self.w, self.h) => {
                let sub = image::imageops::crop_imm(&self.img, r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0).to_image();
                t.set_partial([r.x0 as usize, r.y0 as usize], egui::ColorImage::from_rgba_unmultiplied([sub.width() as usize, sub.height() as usize], sub.as_raw()), opts);
            }
            _ => {
                let ci = egui::ColorImage::from_rgba_unmultiplied([self.w as usize, self.h as usize], self.img.as_raw());
                self.tex = Some(ctx.load_texture("imgedit", ci, opts));
            }
        }
    }

    // ───────────────────────── Koordinaten ─────────────────────────

    fn origin(&self, canvas: Rect) -> Pos2 {
        canvas.center() + self.pan - vec2(self.w as f32, self.h as f32) * self.zoom / 2.0
    }
    fn to_img(&self, canvas: Rect, p: Pos2) -> Pos2 {
        let o = self.origin(canvas);
        pos2((p.x - o.x) / self.zoom, (p.y - o.y) / self.zoom)
    }
    fn to_screen(&self, canvas: Rect, p: Pos2) -> Pos2 {
        self.origin(canvas) + p.to_vec2() * self.zoom
    }
    fn fit(&mut self, canvas: Rect) {
        let k = ((canvas.width() - 40.0) / self.w as f32).min((canvas.height() - 40.0) / self.h as f32);
        self.zoom = k.clamp(0.02, 1.0);
        self.pan = Vec2::ZERO;
    }

    fn rgba(c: Color32) -> [u8; 4] {
        c.to_srgba_unmultiplied()
    }

    // ───────────────────────── Oberfläche ─────────────────────────

    /// Editor in `rect` zeichnen. Gibt zurück, ob geschlossen oder gespeichert wurde.
    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme, rect: Rect) -> Option<EditOut> {
        let ctx = ui.ctx().clone();
        self.ai_poll(&ctx);
        self.upload(&ctx);
        let mut out = None;
        // Handy (schmal): Einstellungen unter dem Bild statt rechts daneben, Kopfzeile nur mit Symbolen
        let phone = rect.width() < 600.0;
        let top = Rect::from_min_size(rect.min, vec2(rect.width(), 46.0));
        let left = Rect::from_min_max(pos2(rect.left(), top.bottom()), pos2(rect.left() + 96.0, rect.bottom() - 26.0));
        let status = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 26.0), rect.max);
        let (right, canvas) = if phone {
            let panel_h = (rect.height() * 0.42).max(220.0);
            let right = Rect::from_min_max(pos2(left.right(), status.top() - panel_h), pos2(rect.right(), status.top()));
            (right, Rect::from_min_max(pos2(left.right(), top.bottom()), pos2(rect.right(), right.top())))
        } else {
            // rechte Spalte: am linken Rand ziehbar, die Zeichenfläche behält mindestens 240 px
            let panel_w = self.panel_w.clamp(200.0, (rect.width() - 96.0 - 240.0).max(200.0));
            let right = Rect::from_min_max(pos2(rect.right() - panel_w, top.bottom()), pos2(rect.right(), rect.bottom() - 26.0));
            (right, Rect::from_min_max(pos2(left.right(), top.bottom()), pos2(right.left(), status.top())))
        };
        ui.painter().rect_filled(rect, 0.0, th.panel);
        if self.fit_next {
            self.fit(canvas);
            self.fit_next = false;
        }
        // Kopfzeile: Datei · Rückgängig/Wiederholen · Zoom · Drehen/Spiegeln · Speichern/Schließen
        ui.painter().rect_filled(top, 0.0, th.bg);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(top.shrink2(vec2(10.0, 8.0))), |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if !phone {
                    let name = self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    ui.label(egui::RichText::new(name).strong());
                    if self.modified {
                        ui.label(egui::RichText::new(" ●").color(th.accent)).on_hover_text("Ungespeicherte Änderungen");
                    }
                    ui.add_space(10.0);
                    sep(ui, th);
                }
                if edicons::btn(ui, th, E::Undo, "Rückgängig (Strg+Z)", !self.undo.is_empty()).clicked() {
                    self.undo();
                }
                if edicons::btn(ui, th, E::Redo, "Wiederholen (Strg+Y)", !self.redo.is_empty()).clicked() {
                    self.redo();
                }
                sep(ui, th);
                if edicons::btn(ui, th, E::ZoomOut, "Verkleinern (Strg+−)", true).clicked() {
                    self.zoom_at(canvas, canvas.center(), 1.0 / 1.25);
                }
                if !phone {
                    ui.add_sized(vec2(48.0, 30.0), egui::Label::new(egui::RichText::new(format!("{:.0} %", self.zoom * 100.0)).monospace()));
                }
                if edicons::btn(ui, th, E::ZoomIn, "Vergrößern (Strg++)", true).clicked() {
                    self.zoom_at(canvas, canvas.center(), 1.25);
                }
                if edicons::btn(ui, th, E::Fit, "Einpassen (Strg+0)", true).clicked() {
                    self.fit(canvas);
                }
                if !phone && edicons::btn(ui, th, E::Actual, "Originalgröße 100 % (Strg+1)", true).clicked() {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                }
                sep(ui, th);
                let mut op: Option<fn(&RgbaImage) -> RgbaImage> = None;
                if edicons::btn(ui, th, E::RotL, "Nach links drehen", true).clicked() {
                    op = Some(image::imageops::rotate270);
                }
                if edicons::btn(ui, th, E::RotR, "Nach rechts drehen", true).clicked() {
                    op = Some(image::imageops::rotate90);
                }
                if !phone && edicons::btn(ui, th, E::FlipH, "Horizontal spiegeln", true).clicked() {
                    op = Some(image::imageops::flip_horizontal);
                }
                if !phone && edicons::btn(ui, th, E::FlipV, "Vertikal spiegeln", true).clicked() {
                    op = Some(image::imageops::flip_vertical);
                }
                if let Some(op) = op {
                    self.push_undo();
                    self.img = op(&self.img);
                    let (w, h) = self.img.dimensions();
                    self.resized(w, h);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if edicons::btn(ui, th, E::Close, if self.modified { "Schließen – ungespeicherte Änderungen gehen verloren" } else { "Schließen" }, true).clicked() {
                        out = Some(EditOut::Close);
                    }
                    if !cfg!(target_os = "android") && edicons::labeled(ui, th, E::SaveAs, "Speichern unter", "Als neue Datei speichern", true, false).clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_directory(self.path.parent().unwrap_or(Path::new(".")))
                            .set_file_name(self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()))
                            .add_filter("Bild", &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "tga", "qoi"])
                            .save_file()
                        {
                            if let Some(o) = self.save(&p) {
                                out = Some(o);
                            }
                        }
                    }
                    let save = if phone { edicons::btn(ui, th, E::Save, "Speichern (Datei überschreiben)", self.modified) } else { edicons::labeled(ui, th, E::Save, "Speichern", "Datei überschreiben (Strg+S)", self.modified, true) };
                    if save.clicked() {
                        let p = self.path.clone();
                        if let Some(o) = self.save(&p) {
                            out = Some(o);
                        }
                    }
                });
            });
        });
        ui.painter().line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(1.0, th.line));
        self.tools_ui(ui, th, left);
        self.panel_ui(ui, th, right);
        self.canvas_ui(ui, th, canvas);
        // Statuszeile
        ui.painter().rect_filled(status, 0.0, th.bg);
        ui.painter().line_segment([status.left_top(), status.right_top()], Stroke::new(1.0, th.line));
        let mut s = format!("{} × {} px  ·  {}", self.w, self.h, TOOLS.iter().find(|t| t.0 == self.tool).map_or("", |t| t.4));
        if let Some(p) = self.cursor_img.filter(|p| p.x >= 0.0 && p.y >= 0.0 && p.x < self.w as f32 && p.y < self.h as f32) {
            let c = self.img.get_pixel(p.x as u32, p.y as u32);
            s = format!("x {:.0}  y {:.0}  ·  RGBA {} {} {} {}  ·  {s}", p.x, p.y, c[0], c[1], c[2], c[3]);
            // Farbe unter dem Zeiger als kleines Feld
            let sw = Rect::from_center_size(status.left_center() + vec2(16.0, 0.0), vec2(12.0, 12.0));
            ui.painter().rect_filled(sw, 2.0, c32(*c));
            ui.painter().rect_stroke(sw, 2.0, Stroke::new(1.0, th.line));
        }
        if let Some(r) = self.sel {
            s = format!("Auswahl {} × {}  ·  {s}", r.x1 - r.x0, r.y1 - r.y0);
        }
        ui.painter().text(status.left_center() + vec2(30.0, 0.0), egui::Align2::LEFT_CENTER, s, egui::FontId::proportional(11.5), th.sub);
        if let Some((err, m)) = &self.msg {
            ui.painter().text(status.right_center() - vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, m, egui::FontId::proportional(11.5), if *err { th.danger } else { th.accent });
        }
        if !phone {
            self.panel_grip(ui, th, right, rect);
        }
        self.ai_ask_popup(&ctx, th, rect);
        self.keys(ui, canvas, &mut out);
        out
    }

    /// Griff am linken Rand der rechten Spalte: ziehen ändert die Breite, Doppelklick setzt sie zurück
    fn panel_grip(&mut self, ui: &mut egui::Ui, th: &Theme, right: Rect, rect: Rect) {
        let grip = Rect::from_min_max(pos2(right.left() - 4.0, right.top()), pos2(right.left() + 4.0, right.bottom()));
        let resp = ui.interact(grip, ui.id().with("imgedit_grip"), egui::Sense::click_and_drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().line_segment([right.left_top(), right.left_bottom()], Stroke::new(2.0, th.accent));
        }
        if resp.dragged() {
            let max = (rect.width() - 96.0 - 240.0).max(200.0);
            self.panel_w = (right.width() - resp.drag_delta().x).clamp(200.0, max);
        }
        if resp.double_clicked() {
            self.panel_w = 320.0;
        }
        PANEL_W.store(self.panel_w as u32, std::sync::atomic::Ordering::Relaxed);
    }

    fn keys(&mut self, ui: &egui::Ui, canvas: Rect, out: &mut Option<EditOut>) {
        if ui.ctx().wants_keyboard_input() {
            return;
        }
        let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
        if cmd && pressed(Key::Z) {
            if shift {
                self.redo();
            } else {
                self.undo();
            }
        } else if cmd && pressed(Key::Y) {
            self.redo();
        } else if cmd && pressed(Key::S) {
            let p = self.path.clone();
            if let Some(o) = self.save(&p) {
                *out = Some(o);
            }
        } else if cmd && pressed(Key::Num0) {
            self.fit(canvas);
        } else if cmd && pressed(Key::Num1) {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        } else if cmd && (pressed(Key::Plus) || pressed(Key::Equals)) {
            self.zoom_at(canvas, canvas.center(), 1.25);
        } else if cmd && pressed(Key::Minus) {
            self.zoom_at(canvas, canvas.center(), 0.8);
        } else if cmd && pressed(Key::A) {
            self.sel = Some(IRect::full(self.w, self.h));
        } else if cmd && pressed(Key::C) {
            self.copy_selection();
        } else if pressed(Key::Escape) {
            self.sel = None;
            if self.tool == Tool::Objects {
                self.drop_mask();
            }
            self.crop = None;
            self.text_at = None;
        } else if pressed(Key::Enter) && self.crop.is_some() {
            self.apply_crop();
        } else if pressed(Key::Delete) && self.sel.is_some() {
            self.clear_selection();
        } else if pressed(Key::X) && !cmd {
            std::mem::swap(&mut self.color, &mut self.color2);
        } else if pressed(Key::OpenBracket) {
            self.size = (self.size / 1.2).max(1.0);
        } else if pressed(Key::CloseBracket) {
            self.size = (self.size * 1.2).min(500.0);
        } else if !cmd {
            for (t, _, _, k, _) in TOOLS {
                if pressed(*k) {
                    self.tool = *t;
                }
            }
        }
    }

    fn zoom_at(&mut self, canvas: Rect, at: Pos2, k: f32) {
        let before = self.to_img(canvas, at);
        self.zoom = (self.zoom * k).clamp(0.02, 32.0);
        let after = self.to_screen(canvas, before);
        self.pan += at - after;
    }

    /// Werkzeugleiste: Icons in zwei Spalten, darunter Farbe/Zweitfarbe
    fn tools_ui(&mut self, ui: &mut egui::Ui, th: &Theme, r: Rect) {
        ui.painter().rect_filled(r, 0.0, th.bg);
        ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.0, th.line));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(r.shrink2(vec2(8.0, 10.0))), |ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for row in TOOLS.chunks(2) {
                ui.horizontal(|ui| {
                    for (t, e, name, k, tip) in row {
                        if edicons::tool(ui, th, *e, self.tool == *t, &format!("{name} ({})\n{tip}", k.name())).clicked() {
                            self.tool = *t;
                        }
                    }
                });
            }
            ui.add_space(12.0);
            // Farbfelder übereinander versetzt wie in Bildprogrammen; Klick öffnet die Farbwahl
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.color_edit_button_srgba(&mut self.color).on_hover_text("Farbe");
                    ui.color_edit_button_srgba(&mut self.color2).on_hover_text("Zweitfarbe (Füllung von Formen)");
                });
                if edicons::btn(ui, th, E::Swap, "Farben tauschen (X)", true).clicked() {
                    std::mem::swap(&mut self.color, &mut self.color2);
                }
            });
        });
    }

    // ───────────────────────── rechte Seite ─────────────────────────

    fn panel_ui(&mut self, ui: &mut egui::Ui, th: &Theme, r: Rect) {
        ui.painter().rect_filled(r, 0.0, th.bg);
        ui.painter().line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0, th.line));
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, th.line));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(r.shrink2(vec2(12.0, 10.0))), |ui| {
            egui::ScrollArea::vertical().id_salt("imgedit_panel").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width() - 6.0);
                self.ai_status_ui(ui, th);
                self.tool_options(ui, th);
                ui.add_space(4.0);
                self.histogram(ui, th);
                ui.add_space(4.0);
                let ai = !cfg!(target_os = "android");
                if ai {
                    section(ui, th, "img_txt", E::Sparkle, "KI-Bearbeitung (Text)", true, |ui| self.text_edit_ui(ui, th));
                }
                section(ui, th, "img_adj", E::Sliders, "Anpassen", true, |ui| self.adjust_ui(ui, th));
                section(ui, th, "img_flt", E::Sparkle, "Filter", false, |ui| {
                    self.filters_ui(ui);
                    if ai {
                        ui.add_space(6.0);
                        self.ai_filters_ui(ui, th);
                    }
                });
                if ai {
                    section(ui, th, "img_chain", E::Wand, "KI-Kette (kombinieren)", false, |ui| self.chain_ui(ui, th));
                }
                section(ui, th, "img_size", E::Resize, "Größe", false, |ui| self.image_ui(ui, th));
                section(ui, th, "img_save", E::Save, "Speichern", false, |ui| {
                    ui.add(egui::Slider::new(&mut self.jpeg_q, 40..=100).text("JPEG-Qualität"));
                    ui.label(egui::RichText::new("PNG, WebP und TIFF behalten Durchsichtigkeit; JPEG und BMP bekommen weißen Hintergrund.").size(11.0).color(th.sub));
                });
            });
        });
    }

    fn tool_options(&mut self, ui: &mut egui::Ui, th: &Theme) {
        if let Some((_, e, name, _, _)) = TOOLS.iter().find(|t| t.0 == self.tool) {
            edicons::heading(ui, th, *e, name);
        }
        match self.tool {
            Tool::Brush | Tool::Eraser => {
                ui.add(egui::Slider::new(&mut self.size, 1.0..=500.0).logarithmic(true).text("Größe  [ ]"));
                ui.add(egui::Slider::new(&mut self.hardness, 0.0..=1.0).text("Härte"));
                ui.add(egui::Slider::new(&mut self.flow, 0.02..=1.0).text("Deckkraft"));
            }
            Tool::Line | Tool::Arrow => {
                ui.add(egui::Slider::new(&mut self.size, 1.0..=200.0).logarithmic(true).text("Stärke"));
            }
            Tool::Rect | Tool::Ellipse => {
                ui.add(egui::Slider::new(&mut self.size, 1.0..=200.0).logarithmic(true).text("Randstärke"));
                ui.checkbox(&mut self.filled, "Gefüllt (Zweitfarbe)");
            }
            Tool::Fill => {
                ui.add(egui::Slider::new(&mut self.tolerance, 0.0..=255.0).text("Toleranz"));
            }
            Tool::Blur | Tool::Pixelate => {
                ui.add(egui::Slider::new(&mut self.size, 2.0..=100.0).text(if self.tool == Tool::Blur { "Stärke" } else { "Blockgröße" }));
            }
            Tool::Text => {
                ui.add(egui::Slider::new(&mut self.font_px, 8.0..=400.0).logarithmic(true).text("Schriftgröße"));
                ui.add(egui::TextEdit::multiline(&mut self.text).desired_rows(3).desired_width(f32::INFINITY).hint_text("Text …"));
                let can = self.text_at.is_some() && !self.text.trim().is_empty();
                ui.horizontal(|ui| {
                    if edicons::labeled(ui, th, E::Check, "Einfügen", "Text ins Bild setzen", can, true).clicked() {
                        self.place_text();
                    }
                    if self.text_at.is_none() {
                        ui.label(egui::RichText::new("Erst ins Bild klicken").color(th.sub));
                    }
                });
                if self.font.is_none() {
                    ui.label(egui::RichText::new("Keine Schrift gefunden").color(th.danger));
                }
            }
            Tool::Crop => {
                ui.horizontal_wrapped(|ui| {
                    let orig = self.w as f32 / self.h as f32;
                    for (label, r) in [("Frei", None), ("Original", Some(orig)), ("1:1", Some(1.0)), ("4:3", Some(4.0 / 3.0)), ("3:2", Some(1.5)), ("16:9", Some(16.0 / 9.0)), ("9:16", Some(9.0 / 16.0))] {
                        if ui.selectable_label(self.crop_ratio == r, label).clicked() {
                            self.crop_ratio = r;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if edicons::labeled(ui, th, E::Crop, "Zuschneiden", "Enter", self.crop.is_some(), true).clicked() {
                        self.apply_crop();
                    }
                    if edicons::btn(ui, th, E::Close, "Abbrechen (Esc)", self.crop.is_some()).clicked() {
                        self.crop = None;
                    }
                });
            }
            Tool::Objects => self.objects_ui(ui, th),
            Tool::Move | Tool::Select | Tool::Picker => {
                ui.label(egui::RichText::new(TOOLS.iter().find(|t| t.0 == self.tool).map_or("", |t| t.4)).color(th.sub));
            }
        }
        if let Some(s) = self.sel {
            ui.add_space(6.0);
            edicons::heading(ui, th, E::Select, &format!("Auswahl {} × {}", s.x1 - s.x0, s.y1 - s.y0));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if edicons::btn(ui, th, E::Crop, "Auf Auswahl zuschneiden", true).clicked() {
                    self.crop = Some(s);
                    self.apply_crop();
                }
                if edicons::btn(ui, th, E::Copy, "Kopieren (Strg+C)", true).clicked() {
                    self.copy_selection();
                }
                if edicons::btn(ui, th, E::Trash, "Löschen – durchsichtig (Entf)", true).clicked() {
                    self.clear_selection();
                }
                if edicons::btn(ui, th, E::Fill, "Mit Farbe füllen", true).clicked() {
                    self.push_undo();
                    let c = Self::rgba(self.color);
                    for y in s.y0..s.y1 {
                        for x in s.x0..s.x1 {
                            self.img.put_pixel(x, y, Rgba(c));
                        }
                    }
                    self.touch(s);
                }
                if edicons::btn(ui, th, E::Blur, "Weichzeichnen", true).clicked() {
                    self.region_effect(s, false, 12.0);
                }
                if edicons::btn(ui, th, E::Pixelate, "Verpixeln", true).clicked() {
                    self.region_effect(s, true, 16.0);
                }
                if edicons::btn(ui, th, E::Close, "Auswahl aufheben (Esc)", true).clicked() {
                    self.sel = None;
                }
            });
        }
        ui.separator();
    }

    fn histogram(&mut self, ui: &mut egui::Ui, th: &Theme) {
        if self.hist.as_ref().is_none_or(|h| h.0 != self.ver) {
            // grob über jeden n-ten Bildpunkt
            let step = ((self.w as usize * self.h as usize) / 200_000).max(1);
            let mut hst = [[0u32; 256]; 3];
            for px in self.img.as_raw().chunks_exact(4).step_by(step) {
                if px[3] > 0 {
                    for c in 0..3 {
                        hst[c][px[c] as usize] += 1;
                    }
                }
            }
            self.hist = Some((self.ver, hst));
        }
        let Some((_, hst)) = &self.hist else { return };
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 6.0, th.field);
        let max = hst.iter().flat_map(|c| c.iter()).copied().max().unwrap_or(1).max(1) as f32;
        let cols = [Color32::from_rgba_unmultiplied(230, 70, 70, 140), Color32::from_rgba_unmultiplied(70, 200, 90, 140), Color32::from_rgba_unmultiplied(70, 120, 240, 140)];
        for (c, col) in cols.iter().enumerate() {
            let pts: Vec<Pos2> = (0..256).map(|i| pos2(r.left() + 4.0 + (r.width() - 8.0) * i as f32 / 255.0, r.bottom() - 3.0 - (hst[c][i] as f32 / max).sqrt() * (r.height() - 8.0))).collect();
            ui.painter().add(egui::Shape::line(pts, Stroke::new(1.2, *col)));
        }
    }

    fn adjust_ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        let a = &mut self.adjust;
        let s = |ui: &mut egui::Ui, v: &mut f32, r: std::ops::RangeInclusive<f32>, label: &str| {
            let resp = ui.add(egui::Slider::new(v, r).text(label).fixed_decimals(2));
            // Doppelklick setzt den Regler zurück
            if resp.double_clicked() {
                *v = 0.0;
            }
        };
        s(ui, &mut a.exposure, -3.0..=3.0, "Belichtung");
        s(ui, &mut a.brightness, -1.0..=1.0, "Helligkeit");
        s(ui, &mut a.contrast, -1.0..=1.0, "Kontrast");
        s(ui, &mut a.gamma, -1.0..=1.0, "Gamma");
        s(ui, &mut a.saturation, -1.0..=1.0, "Sättigung");
        s(ui, &mut a.vibrance, -1.0..=1.0, "Dynamik");
        s(ui, &mut a.hue, -180.0..=180.0, "Farbton");
        s(ui, &mut a.temperature, -1.0..=1.0, "Temperatur");
        s(ui, &mut a.tint, -1.0..=1.0, "Tönung");
        s(ui, &mut a.sharpen, 0.0..=3.0, "Schärfen");
        s(ui, &mut a.blur, 0.0..=40.0, "Weichzeichnen");
        s(ui, &mut a.vignette, -1.0..=1.0, "Vignette");
        ui.horizontal(|ui| {
            let changed = self.adjust != Adjust::default();
            if edicons::labeled(ui, th, E::Check, "Anwenden", "Anpassungen ins Bild übernehmen", changed, true).clicked() {
                self.push_undo();
                let a = self.adjust;
                apply_adjust(&mut self.img, &a);
                self.adjust = Adjust::default();
                self.touch(IRect::full(self.w, self.h));
            }
            if edicons::btn(ui, th, E::Undo, "Regler zurücksetzen", changed).clicked() {
                self.adjust = Adjust::default();
            }
            ui.checkbox(&mut self.adj_preview, "Vorschau");
        });
    }

    fn filters_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let mut f: Option<fn(&mut RgbaImage)> = None;
            if ui.button("Graustufen").clicked() {
                f = Some(|i| per_px(i, |p| {
                    let l = luma(p);
                    [l, l, l]
                }));
            }
            if ui.button("Sepia").clicked() {
                f = Some(|i| per_px(i, |p| [0.393 * p[0] + 0.769 * p[1] + 0.189 * p[2], 0.349 * p[0] + 0.686 * p[1] + 0.168 * p[2], 0.272 * p[0] + 0.534 * p[1] + 0.131 * p[2]]));
            }
            if ui.button("Invertieren").clicked() {
                f = Some(|i| per_px(i, |p| [1.0 - p[0], 1.0 - p[1], 1.0 - p[2]]));
            }
            if ui.button("Auto-Kontrast").clicked() {
                f = Some(auto_contrast);
            }
            if ui.button("Auto-Weißabgleich").clicked() {
                f = Some(auto_white);
            }
            if ui.button("Schärfen").clicked() {
                f = Some(|i| unsharp(i, 1.0, 1.5));
            }
            if ui.button("Weichzeichnen").clicked() {
                f = Some(|i| box_blur(i, 3));
            }
            if ui.button("Posterisieren").clicked() {
                f = Some(|i| per_px(i, |p| p.map(|c| (c * 5.0).round() / 5.0)));
            }
            if let Some(f) = f {
                self.push_undo();
                f(&mut self.img);
                self.touch(IRect::full(self.w, self.h));
            }
        });
    }

    fn image_ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        // Bildgröße
        let (w0, h0) = (self.w, self.h);
        let rs = self.resize.get_or_insert((w0, h0, true));
        ui.label(egui::RichText::new("Bildgröße").color(th.sub));
        ui.horizontal(|ui| {
            let (mut w, mut h) = (rs.0, rs.1);
            let cw = ui.add(egui::DragValue::new(&mut w).range(1..=16384).suffix(" px")).changed();
            ui.label("×");
            let ch = ui.add(egui::DragValue::new(&mut h).range(1..=16384).suffix(" px")).changed();
            ui.checkbox(&mut rs.2, "Verhältnis").on_hover_text("Seitenverhältnis beibehalten");
            if rs.2 && cw {
                h = ((w as f32 * h0 as f32 / w0 as f32).round() as u32).max(1);
            } else if rs.2 && ch {
                w = ((h as f32 * w0 as f32 / h0 as f32).round() as u32).max(1);
            }
            rs.0 = w;
            rs.1 = h;
        });
        let (nw, nh, _) = *rs;
        ui.horizontal(|ui| {
            for p in [25u32, 50, 75, 200] {
                if ui.small_button(format!("{p} %")).clicked() {
                    self.resize = Some(((w0 * p / 100).max(1), (h0 * p / 100).max(1), true));
                }
            }
        });
        if edicons::labeled(ui, th, E::Resize, "Größe ändern", "Neu berechnen (Lanczos)", (nw, nh) != (w0, h0), true).clicked() {
            self.push_undo();
            self.img = image::imageops::resize(&self.img, nw, nh, image::imageops::FilterType::Lanczos3);
            self.resized(nw, nh);
        }
        ui.add_space(6.0);
        // Arbeitsfläche (mittig, neu = durchsichtig)
        let cv = self.canvas.get_or_insert((w0, h0));
        ui.label(egui::RichText::new("Arbeitsfläche (mittig)").color(th.sub));
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut cv.0).range(1..=16384).suffix(" px"));
            ui.label("×");
            ui.add(egui::DragValue::new(&mut cv.1).range(1..=16384).suffix(" px"));
        });
        let (cw, ch) = *cv;
        if edicons::labeled(ui, th, E::Fit, "Arbeitsfläche ändern", "Rand hinzufügen oder abschneiden", (cw, ch) != (w0, h0), false).clicked() {
            self.push_undo();
            let (dx, dy) = ((cw as i64 - w0 as i64) / 2, (ch as i64 - h0 as i64) / 2);
            let mut n = RgbaImage::new(cw, ch);
            image::imageops::overlay(&mut n, &self.img, dx, dy);
            self.img = n;
            self.resized(cw, ch);
        }
        self.upscale_ui(ui, th);
    }

    // ───────────────────────── Zeichenfläche ─────────────────────────

    fn canvas_ui(&mut self, ui: &mut egui::Ui, th: &Theme, canvas: Rect) {
        let resp = ui.interact(canvas, ui.id().with("imgedit_canvas"), egui::Sense::click_and_drag());
        let p = ui.painter_at(canvas);
        p.rect_filled(canvas, 0.0, if th.dark { Color32::from_gray(28) } else { Color32::from_gray(210) });
        let img_rect = Rect::from_min_size(self.origin(canvas), vec2(self.w as f32, self.h as f32) * self.zoom);
        // Schatten und Schachbrett hinter durchsichtigen Stellen
        p.rect_filled(img_rect.translate(vec2(0.0, 3.0)).expand(2.0), 2.0, Color32::from_black_alpha(60));
        let vis = img_rect.intersect(canvas);
        if vis.is_positive() {
            let cell = 10.0;
            p.rect_filled(vis, 0.0, Color32::from_gray(200));
            let mut y = (vis.top() / cell).floor() * cell;
            while y < vis.bottom() {
                let mut x = (vis.left() / cell).floor() * cell;
                while x < vis.right() {
                    if (((x / cell) as i64 + (y / cell) as i64) & 1) == 0 {
                        p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(cell, cell)).intersect(vis), 0.0, Color32::from_gray(240));
                    }
                    x += cell;
                }
                y += cell;
            }
        }
        // Bild (bzw. Vorschau der Anpassungen)
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let adj_id = self.adjust_preview(ui.ctx());
        if let Some(id) = adj_id.or(self.tex.as_ref().map(|t| t.id())) {
            p.image(id, img_rect, uv, Color32::WHITE);
        }
        self.mask_overlay(ui.ctx(), &p, img_rect);

        // Eingaben
        let hover = resp.hover_pos();
        self.cursor_img = hover.map(|h| self.to_img(canvas, h));
        let (scroll, shift, space, mid) = ui.input(|i| (i.raw_scroll_delta.y, i.modifiers.shift, i.key_down(Key::Space), i.pointer.middle_down()));
        if let Some(h) = hover {
            if scroll != 0.0 {
                self.zoom_at(canvas, h, if scroll > 0.0 { 1.15 } else { 1.0 / 1.15 });
            }
        }
        let panning = self.tool == Tool::Move || space || mid;
        if panning && resp.dragged() {
            self.pan += resp.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if !panning {
            self.tool_input(&resp, canvas, shift);
        }
        if resp.hovered() && !panning {
            ui.ctx().set_cursor_icon(match self.tool {
                Tool::Text => egui::CursorIcon::Text,
                Tool::Picker | Tool::Fill => egui::CursorIcon::Cell,
                _ => egui::CursorIcon::Crosshair,
            });
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        // Überlagerungen: Pinselgröße, Vorschau von Formen, Auswahl, Zuschneiden, Textposition
        if let (Some(h), Tool::Brush | Tool::Eraser) = (hover, self.tool) {
            p.circle_stroke(h, self.size / 2.0 * self.zoom, Stroke::new(1.0, Color32::from_white_alpha(200)));
            p.circle_stroke(h, self.size / 2.0 * self.zoom + 1.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
        }
        if let (Some(a), Some(b)) = (self.drag_from, hover.map(|h| self.to_img(canvas, h))) {
            let b = constrain(self.tool, a, b, shift);
            let (sa, sb) = (self.to_screen(canvas, a), self.to_screen(canvas, b));
            let st = Stroke::new((self.size * self.zoom).max(1.0), self.color);
            match self.tool {
                Tool::Line => {
                    p.line_segment([sa, sb], st);
                }
                Tool::Arrow => {
                    p.line_segment([sa, sb], st);
                    for (x, y) in arrow_head(a, b, self.size) {
                        p.line_segment([sb, self.to_screen(canvas, pos2(x, y))], st);
                    }
                }
                Tool::Rect => {
                    let r = Rect::from_two_pos(sa, sb);
                    if self.filled {
                        p.rect_filled(r, 0.0, self.color2);
                    }
                    p.rect_stroke(r, 0.0, st);
                }
                Tool::Ellipse => {
                    let r = Rect::from_two_pos(sa, sb);
                    let pts: Vec<Pos2> = (0..72)
                        .map(|i| {
                            let t = i as f32 / 72.0 * std::f32::consts::TAU;
                            r.center() + vec2(t.cos() * r.width() / 2.0, t.sin() * r.height() / 2.0)
                        })
                        .collect();
                    if self.filled {
                        p.add(egui::Shape::convex_polygon(pts.clone(), self.color2, Stroke::NONE));
                    }
                    p.add(egui::Shape::closed_line(pts, st));
                }
                Tool::Select | Tool::Crop | Tool::Blur | Tool::Pixelate => {
                    p.rect_stroke(Rect::from_two_pos(sa, sb), 0.0, Stroke::new(1.5, th.accent));
                }
                _ => {}
            }
        }
        if let Some(s) = self.sel {
            let r = Rect::from_two_pos(self.to_screen(canvas, pos2(s.x0 as f32, s.y0 as f32)), self.to_screen(canvas, pos2(s.x1 as f32, s.y1 as f32)));
            // „Ameisenstraße“: schwarz-weiß gestrichelt, läuft
            let t = (ui.input(|i| i.time) * 8.0) as f32;
            dashed(&p, r, t);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
        }
        if let Some(c) = self.crop {
            let r = Rect::from_two_pos(self.to_screen(canvas, pos2(c.x0 as f32, c.y0 as f32)), self.to_screen(canvas, pos2(c.x1 as f32, c.y1 as f32)));
            let dim = Color32::from_black_alpha(140);
            for o in [
                Rect::from_min_max(img_rect.min, pos2(img_rect.right(), r.top())),
                Rect::from_min_max(pos2(img_rect.left(), r.bottom()), img_rect.max),
                Rect::from_min_max(pos2(img_rect.left(), r.top()), pos2(r.left(), r.bottom())),
                Rect::from_min_max(pos2(r.right(), r.top()), pos2(img_rect.right(), r.bottom())),
            ] {
                p.rect_filled(o.intersect(img_rect), 0.0, dim);
            }
            p.rect_stroke(r, 0.0, Stroke::new(1.5, Color32::WHITE));
            // Drittel-Linien und Eckgriffe
            for k in 1..3 {
                let f = k as f32 / 3.0;
                p.line_segment([pos2(r.left() + r.width() * f, r.top()), pos2(r.left() + r.width() * f, r.bottom())], Stroke::new(0.8, Color32::from_white_alpha(120)));
                p.line_segment([pos2(r.left(), r.top() + r.height() * f), pos2(r.right(), r.top() + r.height() * f)], Stroke::new(0.8, Color32::from_white_alpha(120)));
            }
            for c in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()] {
                p.rect_filled(Rect::from_center_size(c, vec2(8.0, 8.0)), 1.0, Color32::WHITE);
            }
            p.text(r.left_top() + vec2(4.0, -16.0), egui::Align2::LEFT_TOP, format!("{} × {}", c.x1 - c.x0, c.y1 - c.y0), egui::FontId::proportional(12.0), Color32::WHITE);
        }
        if let (Some(at), Tool::Text) = (self.text_at, self.tool) {
            let s = self.to_screen(canvas, at);
            p.line_segment([s, s + vec2(0.0, self.font_px * self.zoom)], Stroke::new(2.0, th.accent));
            if !self.text.is_empty() {
                p.text(s, egui::Align2::LEFT_TOP, &self.text, egui::FontId::proportional(self.font_px * self.zoom * 0.8), self.color);
            }
        }
    }

    fn tool_input(&mut self, resp: &egui::Response, canvas: Rect, shift: bool) {
        let at = resp.interact_pointer_pos().map(|p| self.to_img(canvas, p));
        match self.tool {
            Tool::Brush | Tool::Eraser => {
                if resp.drag_started() || (resp.is_pointer_button_down_on() && self.last.is_none()) {
                    if let Some(a) = at {
                        self.push_undo();
                        self.last = Some(a);
                        self.stamp_line(a, a);
                    }
                } else if resp.dragged() {
                    if let (Some(a), Some(b)) = (self.last, at) {
                        self.stamp_line(a, b);
                        self.last = Some(b);
                    }
                }
                if !resp.is_pointer_button_down_on() && !resp.dragged() {
                    self.last = None;
                }
            }
            Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse | Tool::Select | Tool::Crop | Tool::Blur | Tool::Pixelate => {
                if resp.drag_started() {
                    self.drag_from = at;
                }
                if resp.drag_stopped() {
                    if let (Some(a), Some(b)) = (self.drag_from.take(), at) {
                        let b = constrain(self.tool, a, b, shift);
                        self.finish_shape(a, b);
                    }
                }
                // Seitenverhältnis beim Zuschneiden schon während des Ziehens
                if self.tool == Tool::Crop && resp.dragged() {
                    if let (Some(a), Some(b)) = (self.drag_from, at) {
                        self.crop = Some(self.crop_rect(a, b));
                    }
                }
                if resp.clicked() && self.tool == Tool::Select {
                    self.sel = None;
                }
            }
            Tool::Text => {
                if resp.clicked() {
                    self.text_at = at;
                }
            }
            Tool::Fill => {
                if let (true, Some(a)) = (resp.clicked(), at) {
                    if a.x >= 0.0 && a.y >= 0.0 && (a.x as u32) < self.w && (a.y as u32) < self.h {
                        self.push_undo();
                        let c = Self::rgba(self.color);
                        if let Some(r) = flood_fill(&mut self.img, a.x as u32, a.y as u32, c, self.tolerance) {
                            self.touch(r);
                        }
                    }
                }
            }
            Tool::Picker => {
                if let (true, Some(a)) = (resp.clicked() || resp.dragged(), at) {
                    if a.x >= 0.0 && a.y >= 0.0 && (a.x as u32) < self.w && (a.y as u32) < self.h {
                        self.color = c32(*self.img.get_pixel(a.x as u32, a.y as u32));
                    }
                }
            }
            Tool::Objects => {
                // nur Linksklick: jeder Klick fügt dem Objekt etwas hinzu
                if let (true, Some(a)) = (resp.clicked(), at) {
                    if a.x >= 0.0 && a.y >= 0.0 && a.x < self.w as f32 && a.y < self.h as f32 {
                        self.points.push((a, true));
                        self.ai_start(AiTask::Points);
                    }
                }
            }
            Tool::Move => {}
        }
    }

    fn crop_rect(&self, a: Pos2, b: Pos2) -> IRect {
        let mut b = b;
        if let Some(r) = self.crop_ratio {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let w = dx.abs().max(dy.abs() * r);
            let h = w / r;
            b = pos2(a.x + w * dx.signum(), a.y + h * dy.signum());
        }
        IRect::from_f(a, b, self.w, self.h)
    }

    fn finish_shape(&mut self, a: Pos2, b: Pos2) {
        match self.tool {
            Tool::Select => {
                let r = IRect::from_f(a, b, self.w, self.h);
                self.sel = (!r.is_empty()).then_some(r);
            }
            Tool::Crop => {
                let r = self.crop_rect(a, b);
                self.crop = (!r.is_empty()).then_some(r);
            }
            Tool::Blur | Tool::Pixelate => {
                let r = IRect::from_f(a, b, self.w, self.h);
                if !r.is_empty() {
                    self.region_effect(r, self.tool == Tool::Pixelate, self.size);
                }
            }
            Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse => {
                self.push_undo();
                let c = Self::rgba(self.color);
                let c2 = Self::rgba(self.color2);
                let w = self.size;
                let img = &mut self.img;
                let r = match self.tool {
                    Tool::Line => draw_line(img, a, b, w, c),
                    Tool::Arrow => {
                        let mut r = draw_line(img, a, b, w, c);
                        for (x, y) in arrow_head(a, b, w) {
                            r = merge_rect(r, draw_line(img, b, pos2(x, y), w, c));
                        }
                        r
                    }
                    Tool::Rect => {
                        let mut r = None;
                        if self.filled {
                            r = merge_rect(r, fill_rect(img, a, b, c2));
                        }
                        let (p0, p1, p2, p3) = (a, pos2(b.x, a.y), b, pos2(a.x, b.y));
                        for (s, e) in [(p0, p1), (p1, p2), (p2, p3), (p3, p0)] {
                            r = merge_rect(r, draw_line(img, s, e, w, c));
                        }
                        r
                    }
                    _ => draw_ellipse(img, a, b, w, c, self.filled.then_some(c2)),
                };
                if let Some(r) = r {
                    self.touch(r);
                }
            }
            _ => {}
        }
    }

    /// Pinsel/Radierer: Stempel entlang der Strecke a→b
    fn stamp_line(&mut self, a: Pos2, b: Pos2) {
        let rad = (self.size / 2.0).max(0.5);
        let step = (rad * 0.25).max(0.5);
        let n = ((b - a).length() / step).ceil().max(1.0) as usize;
        let c = Self::rgba(self.color);
        let (hard, flow, erase) = (self.hardness, self.flow, self.tool == Tool::Eraser);
        let mut dirty: Option<IRect> = None;
        for k in 0..=n {
            let p = a + (b - a) * (k as f32 / n as f32);
            dirty = merge_rect(dirty, stamp(&mut self.img, p, rad, hard, flow, c, erase));
        }
        if let Some(r) = dirty {
            self.modified = true;
            self.touch(r);
        }
    }

    fn region_effect(&mut self, r: IRect, pixelate: bool, amount: f32) {
        self.push_undo();
        let mut sub = image::imageops::crop_imm(&self.img, r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0).to_image();
        if pixelate {
            let b = amount.max(2.0) as u32;
            let (w, h) = sub.dimensions();
            for by in (0..h).step_by(b as usize) {
                for bx in (0..w).step_by(b as usize) {
                    let (x1, y1) = ((bx + b).min(w), (by + b).min(h));
                    let mut acc = [0u64; 4];
                    let mut n = 0u64;
                    for y in by..y1 {
                        for x in bx..x1 {
                            let p = sub.get_pixel(x, y);
                            for c in 0..4 {
                                acc[c] += p[c] as u64;
                            }
                            n += 1;
                        }
                    }
                    let avg = Rgba([(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, (acc[3] / n) as u8]);
                    for y in by..y1 {
                        for x in bx..x1 {
                            sub.put_pixel(x, y, avg);
                        }
                    }
                }
            }
        } else {
            box_blur(&mut sub, amount.max(1.0) as u32);
        }
        image::imageops::replace(&mut self.img, &sub, r.x0 as i64, r.y0 as i64);
        self.touch(r);
    }

    fn clear_selection(&mut self) {
        let Some(s) = self.sel else { return };
        self.push_undo();
        for y in s.y0..s.y1 {
            for x in s.x0..s.x1 {
                self.img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
        self.touch(s);
    }

    fn copy_selection(&mut self) {
        let s = self.sel.unwrap_or(IRect::full(self.w, self.h));
        let sub = image::imageops::crop_imm(&self.img, s.x0, s.y0, s.x1 - s.x0, s.y1 - s.y0).to_image();
        self.msg = Some(match crate::clip::set_image(&image::DynamicImage::ImageRgba8(sub)) {
            Ok(()) => (false, if self.sel.is_some() { "Auswahl kopiert".into() } else { "Bild kopiert".into() }),
            Err(e) => (true, e),
        });
    }

    fn apply_crop(&mut self) {
        let Some(c) = self.crop.take() else { return };
        if c.is_empty() {
            return;
        }
        self.push_undo();
        self.img = image::imageops::crop_imm(&self.img, c.x0, c.y0, c.x1 - c.x0, c.y1 - c.y0).to_image();
        self.resized(c.x1 - c.x0, c.y1 - c.y0);
    }

    fn place_text(&mut self) {
        let (Some(at), true) = (self.text_at, self.font.is_some()) else { return };
        let text = self.text.clone();
        self.push_undo();
        let c = Self::rgba(self.color);
        let Some(font) = self.font.as_ref() else { return };
        if let Some(r) = draw_text(&mut self.img, font, &text, at, self.font_px, c) {
            self.touch(r);
        }
        self.text.clear();
        self.text_at = None;
    }

    /// Vorschau der Anpassungen (verkleinert): Textur-ID, solange etwas eingestellt ist
    fn adjust_preview(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        if !self.adj_preview || self.adjust == Adjust::default() {
            self.adj_tex = None;
            return None;
        }
        if self.small.as_ref().is_none_or(|s| s.0 != self.ver) {
            let k = (1400.0 / self.w.max(self.h) as f32).min(1.0);
            let (sw, sh) = (((self.w as f32 * k) as u32).max(1), ((self.h as f32 * k) as u32).max(1));
            self.small = Some((self.ver, image::imageops::resize(&self.img, sw, sh, image::imageops::FilterType::Triangle), k));
            self.adj_tex = None;
        }
        if let Some((a, v, t)) = &self.adj_tex {
            if *a == self.adjust && *v == self.ver {
                return Some(t.id());
            }
        }
        let (_, small, k) = self.small.as_ref()?;
        let mut adj = self.adjust;
        // Weichzeichnen an die Verkleinerung anpassen
        adj.blur *= k;
        let mut img = small.clone();
        apply_adjust(&mut img, &adj);
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let t = ctx.load_texture("imgedit_adj", ci, egui::TextureOptions::LINEAR);
        let id = t.id();
        self.adj_tex = Some((self.adjust, self.ver, t));
        Some(id)
    }

    /// Bild speichern (Format nach Endung). Danach gilt der Stand als gespeichert.
    // ───────────────────────── lokale KI ─────────────────────────

    fn drop_mask(&mut self) {
        self.mask = None;
        self.mask_tex = None;
        self.points.clear();
        self.points_pending = false;
        self.sam_low = None;
    }

    fn set_mask(&mut self, m: GrayImage) {
        if m.dimensions() == (self.w, self.h) {
            self.mask = Some(m);
            self.mask_tex = None;
        }
    }

    /// KI-Aufgabe starten – fehlen Modelle, erst nachfragen
    fn ai_start(&mut self, task: AiTask) {
        if self.ai.is_some() {
            if matches!(task, AiTask::Points) {
                self.points_pending = true;
            }
            return;
        }
        let needs = match &task {
            AiTask::Chain(c) => c.needs(self.mask.is_some()),
            AiTask::Subject => vec![localai::Need::Subject],
            AiTask::Points => vec![localai::Need::Objects],
            AiTask::Inpaint => vec![localai::Need::Inpaint],
            // eigene Downloads (Lokale Modelle) – keine Rückfrage hier
            AiTask::TextEdit(..) => vec![],
        };
        let miss = localai::missing(&needs);
        if !localai::supported() {
            self.msg = Some((true, "Lokale KI gibt es bisher nur für Windows (x64)".into()));
        } else if !miss.is_empty() {
            self.ai_ask = Some((task, miss));
        } else {
            self.ai_run(task);
        }
    }

    fn ai_run(&mut self, task: AiTask) {
        self.ai_ask = None;
        self.points_pending = false;
        let clear_mask = matches!(task, AiTask::Inpaint);
        let prog = localai::Progress::default();
        let (tx, rx) = mpsc::channel();
        let p = prog.clone();
        let what = match task {
            AiTask::Chain(c) => {
                let (img, mask) = (self.img.clone(), self.mask.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(localai::run_chain(img, mask, &c, &p).map(AiOut::Image));
                });
                c.label()
            }
            AiTask::Subject => {
                let img = self.img.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(localai::subject_mask(&img, &p).map(|m| AiOut::Mask(m, None, None)));
                });
                "Hauptmotiv erkennen".into()
            }
            AiTask::Points => {
                let pts: Vec<(f32, f32, bool)> = self.points.iter().map(|(q, pos)| (q.x, q.y, *pos)).collect();
                let ver = self.ver;
                let known = self.embed.as_ref().filter(|e| e.0 == ver).map(|e| e.1.clone());
                // das Bild nur kopieren, wenn es neu kodiert werden muss
                let img = known.is_none().then(|| self.img.clone());
                // nach neuem Kodieren passt die alte grobe Maske nicht mehr
                let prev = known.as_ref().and(self.sam_low.clone());
                std::thread::spawn(move || {
                    let r = (|| {
                        let (e, fresh) = match (known, img) {
                            (Some(e), _) => (e, None),
                            (None, Some(img)) => {
                                let e = Arc::new(localai::sam_embed(&img, &p)?);
                                (e.clone(), Some((ver, e)))
                            }
                            (None, None) => return Err("Kein Bild".to_string()),
                        };
                        p.set(-1.0, "Objekt auswählen …");
                        localai::sam_mask(&e, &pts, prev.as_deref()).map(|(m, low)| AiOut::Mask(m, fresh, Some(low)))
                    })();
                    let _ = tx.send(r);
                });
                "Objekt auswählen".into()
            }
            AiTask::TextEdit(i, prompt) => {
                let img = self.img.clone();
                let seed = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) % 2_147_483_647) as i64;
                std::thread::spawn(move || {
                    let m = &crate::localllm::EDIT[i.min(crate::localllm::EDIT.len() - 1)];
                    let _ = tx.send(crate::localllm::edit_image(m, &img, &prompt, seed, &p).map(AiOut::Image));
                });
                "KI-Bearbeitung".into()
            }
            AiTask::Inpaint => {
                let (img, mask) = (self.img.clone(), self.mask.clone().unwrap_or_else(|| GrayImage::new(self.w, self.h)));
                std::thread::spawn(move || {
                    let _ = tx.send(localai::inpaint(&img, &mask, &p).map(AiOut::Image));
                });
                "Objekt entfernen".into()
            }
        };
        self.ai = Some(AiJob { what, prog, rx, clear_mask });
    }

    /// Ergebnis einer laufenden KI-Aufgabe übernehmen
    fn ai_poll(&mut self, ctx: &egui::Context) {
        let Some(j) = &self.ai else { return };
        let r = match j.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err("KI-Aufgabe abgebrochen".into()),
        };
        let job = self.ai.take().expect("läuft");
        let what = job.what.clone();
        match r {
            Ok(AiOut::Image(img)) => {
                self.push_undo();
                let (w, h) = img.dimensions();
                let same = (w, h) == (self.w, self.h);
                self.img = img;
                if same {
                    self.touch(IRect::full(w, h));
                } else {
                    self.resized(w, h);
                    self.fit_next = true;
                }
                if job.clear_mask {
                    self.drop_mask();
                }
                self.msg = Some((false, format!("{what}: fertig")));
            }
            Ok(AiOut::Mask(m, fresh, low)) => {
                if let Some(e) = fresh {
                    self.embed = Some(e);
                }
                self.sam_low = low.filter(|l| !l.is_empty());
                self.set_mask(m);
                // inzwischen weitere Punkte gesetzt?
                if self.points_pending && !self.points.is_empty() {
                    self.ai_run(AiTask::Points);
                }
            }
            Err(e) => {
                self.points_pending = false;
                self.msg = Some((true, e));
            }
        }
    }

    /// Fortschritt/Abbrechen und die Rückfrage vor dem Herunterladen – oben in der rechten Spalte
    fn ai_status_ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        if let Some(j) = &self.ai {
            let (frac, text) = j.prog.get();
            let mut cancel = false;
            egui::Frame::none().fill(th.field).rounding(6.0).inner_margin(8.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    edicons::heading(ui, th, E::Sparkle, &j.what);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        cancel = edicons::btn(ui, th, E::Close, "Abbrechen", true).clicked();
                    });
                });
                if frac >= 0.0 {
                    ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_height(6.0));
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                    });
                }
                ui.label(egui::RichText::new(if text.is_empty() { "Startet …" } else { &text }).size(11.0).color(th.sub));
            });
            if cancel {
                j.prog.cancel();
            }
            ui.add_space(6.0);
        }
    }

    /// Popup, wenn eine Funktion Modelle braucht, die noch fehlen
    fn ai_ask_popup(&mut self, ctx: &egui::Context, th: &Theme, rect: Rect) {
        let Some((task, miss)) = &self.ai_ask else { return };
        let what = match task {
            AiTask::Chain(c) => c.label(),
            AiTask::Subject => "Hauptmotiv erkennen".into(),
            AiTask::Points => "Objekte auswählen".into(),
            AiTask::Inpaint => "Objekt entfernen".into(),
            AiTask::TextEdit(..) => "KI-Bearbeitung".into(),
        };
        let mb: u32 = miss.iter().map(|a| a.mb).sum();
        // abgedunkelter Hintergrund fängt alle Klicks ab
        egui::Area::new(egui::Id::new("imgedit_ai_dim")).order(egui::Order::Middle).fixed_pos(rect.min).show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(rect.size(), egui::Sense::click_and_drag());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(140));
        });
        let (mut go, mut no) = (false, ctx.input(|i| i.key_pressed(Key::Escape)));
        egui::Window::new("imgedit_ai_ask")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .fixed_size(vec2(420.0, 0.0))
            .frame(egui::Frame::window(&ctx.style()).fill(th.bg).stroke(Stroke::new(1.0, th.line)).rounding(10.0).inner_margin(18.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), egui::Sense::hover());
                    edicons::draw(ui.painter(), r, E::Sparkle, th.accent);
                    ui.label(egui::RichText::new("KI-Modelle benötigt").size(17.0).strong());
                });
                ui.add_space(4.0);
                ui.label(format!("Für „{what}“ fehlen noch:"));
                ui.add_space(4.0);
                egui::Frame::none().fill(th.field).rounding(6.0).inner_margin(10.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for a in miss {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(a.name).strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(format!("{} MB", a.mb)).color(th.sub));
                            });
                        });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Zusammen");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(format!("{mb} MB")).strong());
                        });
                    });
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new(format!("Einmaliger Download von GitHub bzw. Hugging Face nach {}. Danach läuft alles offline auf diesem PC – kein Bild wird hochgeladen. Verwalten unter Einstellungen → Bildeditor-KI.", localai::dir().display())).size(11.5).color(th.sub));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    go = edicons::labeled(ui, th, E::Check, "Herunterladen und starten", "", true, true).clicked();
                    no |= edicons::labeled(ui, th, E::Close, "Abbrechen", "Esc", true, false).clicked();
                });
            });
        if go {
            let t = task.clone();
            self.ai_run(t);
        } else if no {
            self.ai_ask = None;
            self.points_pending = false;
        }
    }

    fn ai_busy(&self) -> bool {
        self.ai.is_some() || self.ai_ask.is_some()
    }

    /// KI-Bearbeitung: schreiben, was sich ändern soll – ein lokales Bearbeitungsmodell setzt es um
    fn text_edit_ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        use crate::localllm as ll;
        ui.label(egui::RichText::new("Beschreibe, was sich ändern soll – z. B. „mach daraus ein Ölgemälde“, „Himmel bei Sonnenuntergang“, „entferne die Person links“. Englisch klappt meist am besten.").size(11.5).color(th.sub));
        ui.horizontal_wrapped(|ui| {
            for (i, m) in ll::EDIT.iter().enumerate() {
                let tip = format!("{}\n{} · {}", m.desc, crate::localui::gb(m.total_mb() as u64), m.fit().label());
                if ui.selectable_label(self.txt_model == i, m.name).on_hover_text(tip).clicked() {
                    self.txt_model = i;
                }
            }
        });
        let m = &ll::EDIT[self.txt_model.min(ll::EDIT.len() - 1)];
        if m.installed() {
            let r = ui.add(egui::TextEdit::multiline(&mut self.txt_prompt).desired_rows(3).desired_width(f32::INFINITY).hint_text("Was soll anders werden? (Strg+Enter)"));
            let go = r.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter));
            let can = !self.ai_busy() && !self.txt_prompt.trim().is_empty();
            ui.horizontal(|ui| {
                if (edicons::labeled(ui, th, E::Sparkle, "Anwenden", "Bild nach der Anweisung ändern (ein Rückgängig-Schritt)", can, true).clicked() || go) && can {
                    let t = self.txt_prompt.trim().to_string();
                    self.ai_start(AiTask::TextEdit(self.txt_model, t));
                }
                ui.label(egui::RichText::new(format!("{} Schritte · {}", m.steps, m.fit().label())).size(11.0).color(th.sub));
            });
        } else if let Some((f, t)) = m.progress() {
            if f >= 0.0 {
                ui.add(egui::ProgressBar::new(f.clamp(0.0, 1.0)).desired_height(6.0));
            } else {
                ui.spinner();
            }
            ui.label(egui::RichText::new(t).size(11.0).color(th.sub));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            ui.label(egui::RichText::new(format!("{} einmalig laden: {} (mit Zusatzdateien) – {}", m.name, crate::localui::gb(m.missing_mb() as u64), m.fit().label())).size(12.0));
            if edicons::labeled(ui, th, E::Check, "Laden", "Von Hugging Face laden; läuft danach lokal", true, true).clicked() {
                ll::install_edit(m);
            }
        }
    }

    /// Einstellungen und Aktionen des Werkzeugs „Objekte auswählen“
    fn objects_ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        ui.label(egui::RichText::new("Objekt anklicken – jeder weitere Klick fügt Teile hinzu. Die Auswahl ist blau markiert.").size(11.5).color(th.sub));
        let free = !self.ai_busy();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if edicons::labeled(ui, th, E::Subject, "Hauptmotiv", "Hauptmotiv automatisch auswählen (KI)", free, true).clicked() {
                self.points.clear();
                self.sam_low = None;
                self.ai_start(AiTask::Subject);
            }
            if edicons::btn(ui, th, E::Undo, "Letzten Klick zurücknehmen", !self.points.is_empty() && free).clicked() {
                self.points.pop();
                // aus den übrigen Klicks neu berechnen
                self.sam_low = None;
                if self.points.is_empty() {
                    self.drop_mask();
                } else {
                    self.ai_start(AiTask::Points);
                }
            }
            if edicons::btn(ui, th, E::Close, "Auswahl verwerfen (Esc)", self.mask.is_some()).clicked() {
                self.drop_mask();
            }
        });
        let Some(m) = &self.mask else { return };
        ui.add_space(6.0);
        let bounds = localai::mask_bounds(m);
        ui.label(egui::RichText::new(match bounds {
            Some((x0, y0, x1, y1)) => format!("Objekt {} × {} px", x1 - x0, y1 - y0),
            None => "Nichts ausgewählt".into(),
        }).color(th.sub));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            if edicons::labeled(ui, th, E::Subject, "Freistellen", "Alles außer dem Objekt durchsichtig machen", free, true).clicked() {
                self.mask_op(|img, m, _| localai::remove_bg(img, m));
            }
            if edicons::labeled(ui, th, E::Eraser, "Objekt entfernen", if self.obj_fill { "Objekt entfernen – KI erzeugt an der Stelle passenden Inhalt (LaMa)" } else { "Objekt durchsichtig machen" }, free, false).clicked() {
                if self.obj_fill {
                    self.ai_start(AiTask::Inpaint);
                } else {
                    self.mask_op(|img, m, _| localai::remove_bg(img, &invert(m)));
                    self.drop_mask();
                }
            }
            if edicons::labeled(ui, th, E::Focus, "Hintergrund unscharf", "Alles außer dem Objekt weichzeichnen (Stärke unter KI-Kette)", free, false).clicked() {
                let c = self.chain;
                self.mask_op(move |img, m, _| {
                    let r = c.blur_radius(img.width(), img.height());
                    localai::blur_background(img, m, r);
                });
            }
        });
        ui.checkbox(&mut self.obj_fill, "Beim Entfernen passenden Inhalt erzeugen (KI)");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if edicons::btn(ui, th, E::Copy, "Objekt kopieren (freigestellt)", bounds.is_some()).clicked() {
                self.copy_object();
            }
            if edicons::btn(ui, th, E::Invert, "Auswahl umkehren", true).clicked() {
                if let Some(m) = self.mask.take() {
                    self.set_mask(invert(&m));
                }
                self.points.iter_mut().for_each(|p| p.1 = !p.1);
                self.sam_low = None;
            }
            if edicons::btn(ui, th, E::Select, "Als Auswahlrechteck übernehmen", bounds.is_some()).clicked() {
                if let Some((x0, y0, x1, y1)) = bounds {
                    self.sel = Some(IRect { x0, y0, x1, y1 });
                }
            }
            if edicons::btn(ui, th, E::Crop, "Auf das Objekt zuschneiden", bounds.is_some()).clicked() {
                if let Some((x0, y0, x1, y1)) = bounds {
                    self.crop = Some(IRect { x0, y0, x1, y1 });
                    self.apply_crop();
                }
            }
        });
    }

    /// Bildoperation mit der Objektmaske (ein Rückgängig-Schritt)
    fn mask_op(&mut self, f: impl FnOnce(&mut RgbaImage, &GrayImage, u32)) {
        let Some(m) = self.mask.clone() else { return };
        self.push_undo();
        let w = self.w.max(self.h);
        f(&mut self.img, &m, w);
        self.touch(IRect::full(self.w, self.h));
    }

    fn copy_object(&mut self) {
        let Some(m) = &self.mask else { return };
        let Some((x0, y0, x1, y1)) = localai::mask_bounds(m) else { return };
        let mut sub = image::imageops::crop_imm(&self.img, x0, y0, x1 - x0, y1 - y0).to_image();
        let sm = image::imageops::crop_imm(m, x0, y0, x1 - x0, y1 - y0).to_image();
        localai::remove_bg(&mut sub, &sm);
        self.msg = Some(match crate::clip::set_image(&image::DynamicImage::ImageRgba8(sub)) {
            Ok(()) => (false, "Objekt kopiert".into()),
            Err(e) => (true, e),
        });
    }

    /// KI-Filter (Kolorieren, Hintergrund unscharf) und die kombinierbare Kette
    fn ai_filters_ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        let free = !self.ai_busy();
        ui.label(egui::RichText::new("Lokale KI").color(th.sub));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            if edicons::labeled(ui, th, E::Palette, "Kolorieren", "Schwarz-Weiß-Bild einfärben (KI, DeOldify)", free, false).clicked() {
                self.ai_start(AiTask::Chain(Chain { colorize: true, ..Chain::default() }));
            }
            let tip = if self.mask.is_some() { "Alles außer dem ausgewählten Objekt weichzeichnen" } else { "Hauptmotiv erkennen und den Hintergrund weichzeichnen (KI)" };
            if edicons::labeled(ui, th, E::Focus, "Hintergrund unscharf", tip, free, false).clicked() {
                self.ai_start(AiTask::Chain(Chain { bg: Bg::Blur, blur: self.chain.blur, ..Chain::default() }));
            }
        });
    }

    fn chain_ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        ui.label(egui::RichText::new("Schritte wählen – sie laufen nacheinander und lassen sich mit einem Schritt rückgängig machen.").size(11.0).color(th.sub));
        let c = &mut self.chain;
        ui.checkbox(&mut c.colorize, "1  Schwarz-Weiß kolorieren");
        ui.horizontal(|ui| {
            ui.label("2  Hintergrund");
            for (b, l) in [(Bg::Keep, "lassen"), (Bg::Blur, "unscharf"), (Bg::Remove, "entfernen")] {
                if ui.selectable_label(c.bg == b, l).clicked() {
                    c.bg = b;
                }
            }
        });
        if c.bg == Bg::Blur {
            ui.add(egui::Slider::new(&mut c.blur, 1.0..=40.0).text("Unschärfe").fixed_decimals(0));
        }
        ui.horizontal(|ui| {
            ui.label("3  Hochskalieren");
            for (f, l) in [(1u32, "aus"), (2, "2×"), (4, "4×")] {
                if ui.selectable_label(c.upscale == f, l).clicked() {
                    c.upscale = f;
                }
            }
        });
        if c.upscale > 1 {
            ui.checkbox(&mut c.drawing, "Zeichnung / Anime statt Foto");
        }
        if c.bg != Bg::Keep {
            ui.label(egui::RichText::new(if self.mask.is_some() { "Hintergrund = alles außer dem ausgewählten Objekt" } else { "Hintergrund wird automatisch erkannt (oder vorher mit „Objekte auswählen“ festlegen)" }).size(11.0).color(th.sub));
        }
        let c = self.chain;
        let too_big = c.upscale > 1 && self.w.max(self.h) * c.upscale > 16384;
        if too_big {
            ui.label(egui::RichText::new("Zu groß zum Hochskalieren (höchstens 16384 px je Seite)").size(11.0).color(th.danger));
        }
        let ok = !c.is_empty() && !too_big && !self.ai_busy();
        if edicons::labeled(ui, th, E::Sparkle, "Kette anwenden", &c.label(), ok, true).clicked() {
            self.ai_start(AiTask::Chain(c));
        }
    }

    /// KI-Hochskalieren unter „Größe“
    fn upscale_ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("KI-Hochskalieren (Real-ESRGAN, lokal)").color(th.sub));
        let mut f = self.chain.upscale.max(2);
        ui.horizontal(|ui| {
            for k in [2u32, 4] {
                if ui.selectable_label(f == k, format!("{k}×  →  {} × {}", self.w * k, self.h * k)).clicked() {
                    f = k;
                }
            }
        });
        self.chain.upscale = f;
        ui.checkbox(&mut self.chain.drawing, "Zeichnung / Anime statt Foto");
        let fits = self.w.max(self.h) * f <= 16384;
        if !fits {
            ui.label(egui::RichText::new("Zu groß (höchstens 16384 px je Seite)").size(11.0).color(th.danger));
        }
        if edicons::labeled(ui, th, E::Upscale, "Hochskalieren", "Mehr Bildpunkte mit echten Details statt nur Vergrößern (nutzt die Grafikkarte)", fits && !self.ai_busy(), true).clicked() {
            self.ai_start(AiTask::Chain(Chain { upscale: f, drawing: self.chain.drawing, ..Chain::default() }));
        }
    }

    /// Objektmaske bläulich und halbdurchsichtig über dem Bild
    fn mask_overlay(&mut self, ctx: &egui::Context, p: &egui::Painter, img_rect: Rect) {
        let Some(m) = &self.mask else { return };
        if self.mask_tex.is_none() {
            let px: Vec<Color32> = m.as_raw().iter().map(|v| Color32::from_rgba_unmultiplied(58, 134, 255, (*v as u32 * 120 / 255) as u8)).collect();
            let ci = egui::ColorImage { size: [m.width() as usize, m.height() as usize], pixels: px };
            self.mask_tex = Some(ctx.load_texture("imgedit_mask", ci, egui::TextureOptions::LINEAR));
        }
        if let Some(t) = &self.mask_tex {
            p.image(t.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    }

    /// Bild speichern (Format nach Endung). Danach gilt der Stand als gespeichert.
    fn save(&mut self, p: &Path) -> Option<EditOut> {
        let ext = crate::convert::ext_of(p);
        let mut bytes = Vec::new();
        let r = match ext.as_str() {
            "jpg" | "jpeg" => {
                // JPEG kennt keine Durchsichtigkeit: auf Weiß legen
                let rgb = flatten_white(&self.img);
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, self.jpeg_q).encode_image(&rgb).map_err(|e| e.to_string())
            }
            "bmp" => image::DynamicImage::ImageRgb8(flatten_white(&self.img)).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Bmp).map_err(|e| e.to_string()),
            other => match image::ImageFormat::from_extension(other) {
                Some(f) => image::DynamicImage::ImageRgba8(self.img.clone()).write_to(&mut std::io::Cursor::new(&mut bytes), f).map_err(|e| e.to_string()),
                None => Err(format!("Format .{other} kann nicht gespeichert werden")),
            },
        };
        let r = r.and_then(|()| crate::themes::write_atomic(p, &bytes).map_err(|e| e.to_string()));
        match r {
            Ok(()) => {
                self.modified = false;
                self.msg = Some((false, format!("Gespeichert: {}", p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()))));
                Some(EditOut::Saved(p.to_path_buf()))
            }
            Err(e) => {
                self.msg = Some((true, format!("Speichern fehlgeschlagen: {e}")));
                None
            }
        }
    }
}

/// senkrechter Trenner in Leisten
/// Maske umkehren (ausgewählt ↔ nicht ausgewählt)
fn invert(m: &GrayImage) -> GrayImage {
    let mut o = m.clone();
    o.iter_mut().for_each(|v| *v = 255 - *v);
    o
}

/// Einstellungen → KI-Modelle: alle lokalen Modelle mit Zweck, Größe und Installieren/Entfernen
pub fn models_settings_ui(ui: &mut egui::Ui, th: &Theme) {
    static CLEANED: std::sync::Once = std::sync::Once::new();
    CLEANED.call_once(localai::remove_obsolete);
    let msg_id = egui::Id::new("ai_models_msg");
    if let Some(r) = localai::install_result() {
        let m = match r {
            Ok(()) => (false, "Installiert".to_string()),
            Err(e) => (true, e),
        };
        ui.ctx().data_mut(|d| d.insert_temp(msg_id, m));
    }
    let running = localai::install_status();
    ui.label(egui::RichText::new("Lokale KI für den Bildeditor").size(14.5).strong());
    ui.label(egui::RichText::new(format!("Alles läuft auf diesem PC, kein Bild wird hochgeladen. Gespeichert in {}.", localai::dir().display())).size(12.0).color(th.sub));
    ui.add_space(6.0);
    let cat = localai::catalog();
    let missing: Vec<&'static localai::Asset> = cat.iter().filter(|(a, _)| !localai::installed(a)).map(|(a, _)| *a).collect();
    let mut set_msg: Option<(bool, String)> = None;
    for (a, used) in cat {
        let on = localai::installed(a);
        egui::Frame::none().fill(th.bg).stroke(Stroke::new(1.0, th.line)).rounding(8.0).inner_margin(egui::Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
                edicons::draw(ui.painter(), r, E::Sparkle, if on { th.accent } else { th.sub });
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(egui::RichText::new(a.name).strong());
                    let state = if on { format!("Installiert · {:.0} MB", localai::disk_size(a) as f64 / 1e6) } else { format!("Nicht installiert · {} MB", a.mb) };
                    ui.label(egui::RichText::new(format!("{used}  ·  {state}")).size(12.0).color(if on { th.accent } else { th.sub }));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if on {
                        if edicons::labeled(ui, th, E::Trash, "Entfernen", "Modell löschen (wird bei Bedarf neu geladen)", running.is_none(), false).clicked() {
                            set_msg = Some(match localai::uninstall(a) {
                                Ok(()) => (false, format!("{} entfernt", a.name)),
                                Err(e) => (true, e),
                            });
                        }
                    } else if edicons::labeled(ui, th, E::Check, "Installieren", &format!("{} MB herunterladen", a.mb), running.is_none() && localai::supported(), true).clicked() {
                        // ohne Laufzeit läuft kein ONNX-Modell
                        let mut list = vec![a];
                        if !std::ptr::eq(a, &localai::ESRGAN) && !localai::installed(&localai::ORT) {
                            list.insert(0, &localai::ORT);
                        }
                        localai::install(list);
                    }
                });
            });
        });
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let mb: u32 = missing.iter().map(|a| a.mb).sum();
        if edicons::labeled(ui, th, E::Check, &format!("Alle installieren ({mb} MB)"), "Alle fehlenden Modelle herunterladen", !missing.is_empty() && running.is_none() && localai::supported(), true).clicked() {
            localai::install(missing.clone());
        }
        let any = cat.iter().any(|(a, _)| localai::installed(a));
        if edicons::labeled(ui, th, E::Trash, "Alle entfernen", "Alle lokalen KI-Modelle löschen", any && running.is_none(), false).clicked() {
            let errs: Vec<String> = cat.iter().filter(|(a, _)| localai::installed(a)).filter_map(|(a, _)| localai::uninstall(a).err()).collect();
            set_msg = Some(if errs.is_empty() { (false, "Alle Modelle entfernt".into()) } else { (true, errs.join(" · ")) });
        }
    });
    if let Some((frac, text)) = &running {
        ui.horizontal(|ui| {
            if *frac >= 0.0 {
                ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_width((ui.available_width() - 120.0).max(80.0)).desired_height(8.0));
            } else {
                ui.spinner();
            }
            if ui.button("Abbrechen").clicked() {
                localai::cancel_install();
            }
        });
        ui.label(egui::RichText::new(text).size(12.0).color(th.sub));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
    }
    if let Some(m) = set_msg {
        ui.ctx().data_mut(|d| d.insert_temp(msg_id, m));
    }
    if let Some((err, m)) = ui.ctx().data(|d| d.get_temp::<(bool, String)>(msg_id)) {
        ui.label(egui::RichText::new(m).size(12.0).color(if err { th.danger } else { th.accent }));
    }
}

fn sep(ui: &mut egui::Ui, th: &Theme) {
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 22.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, th.line);
    ui.add_space(6.0);
}

/// aufklappbarer Abschnitt mit Icon in der Überschrift
fn section(ui: &mut egui::Ui, th: &Theme, id: &str, e: E, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    let id = ui.make_persistent_id(id);
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open)
        .show_header(ui, |ui| edicons::heading(ui, th, e, title))
        .body(body);
}

// ───────────────────────── Bildoperationen ─────────────────────────

fn merge_rect(a: Option<IRect>, b: Option<IRect>) -> Option<IRect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    }
}

/// Umschalt: Linien in 45°-Schritten, Rechteck/Ellipse quadratisch
fn constrain(tool: Tool, a: Pos2, b: Pos2, shift: bool) -> Pos2 {
    if !shift {
        return b;
    }
    let d = b - a;
    match tool {
        Tool::Line | Tool::Arrow => {
            let ang = (d.y.atan2(d.x) / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
            a + vec2(ang.cos(), ang.sin()) * d.length()
        }
        Tool::Rect | Tool::Ellipse | Tool::Select => {
            let s = d.x.abs().max(d.y.abs());
            a + vec2(s * d.x.signum(), s * d.y.signum())
        }
        _ => b,
    }
}

fn arrow_head(a: Pos2, b: Pos2, w: f32) -> [(f32, f32); 2] {
    let d = (b - a).normalized();
    let len = (w * 4.0).max(14.0);
    let ang = 0.5f32;
    let rot = |v: Vec2, t: f32| vec2(v.x * t.cos() - v.y * t.sin(), v.x * t.sin() + v.y * t.cos());
    let l = b - rot(d, ang) * len;
    let r = b - rot(d, -ang) * len;
    [(l.x, l.y), (r.x, r.y)]
}

/// Bildpunkt `s` (mit Deckkraft) über `d` legen – gerade Alphawerte
fn blend_px(d: &mut [u8], s: &[u8], opacity: f32) {
    let sa = s[3] as f32 / 255.0 * opacity;
    if sa <= 0.0 {
        return;
    }
    let da = d[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    for c in 0..3 {
        let (sc, dc) = (s[c] as f32 / 255.0, d[c] as f32 / 255.0);
        let oc = (sc * sa + dc * da * (1.0 - sa)) / oa.max(1e-6);
        d[c] = (oc * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    d[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// runden, weichen Stempel setzen (Pinsel) bzw. Deckkraft abziehen (Radierer)
fn stamp(img: &mut RgbaImage, p: Pos2, rad: f32, hard: f32, flow: f32, c: [u8; 4], erase: bool) -> Option<IRect> {
    let (w, h) = img.dimensions();
    let r = IRect::from_f(p - vec2(rad + 1.0, rad + 1.0), p + vec2(rad + 1.0, rad + 1.0), w, h);
    if r.is_empty() {
        return None;
    }
    let inner = rad * hard.clamp(0.0, 0.99);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let d = (pos2(x as f32 + 0.5, y as f32 + 0.5) - p).length();
            let cov = if d <= inner { 1.0 } else if d >= rad + 0.5 { 0.0 } else { (1.0 - (d - inner) / (rad + 0.5 - inner)).clamp(0.0, 1.0) };
            if cov <= 0.0 {
                continue;
            }
            let px = img.get_pixel_mut(x, y);
            let a = cov * flow;
            if erase {
                px[3] = (px[3] as f32 * (1.0 - a)).round() as u8;
            } else {
                let mut s = c;
                s[3] = (c[3] as f32 * a).round() as u8;
                blend_px(&mut px.0, &s, 1.0);
            }
        }
    }
    Some(r)
}

/// Linie mit runden Enden und geglättetem Rand
fn draw_line(img: &mut RgbaImage, a: Pos2, b: Pos2, width: f32, c: [u8; 4]) -> Option<IRect> {
    let (w, h) = img.dimensions();
    let rad = (width / 2.0).max(0.5);
    let r = IRect::from_f(pos2(a.x.min(b.x) - rad - 1.0, a.y.min(b.y) - rad - 1.0), pos2(a.x.max(b.x) + rad + 1.0, a.y.max(b.y) + rad + 1.0), w, h);
    if r.is_empty() {
        return None;
    }
    let ab = b - a;
    let len2 = ab.length_sq().max(1e-6);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
            let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
            let d = (p - (a + ab * t)).length();
            let cov = (rad + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                let mut s = c;
                s[3] = (c[3] as f32 * cov).round() as u8;
                blend_px(&mut img.get_pixel_mut(x, y).0, &s, 1.0);
            }
        }
    }
    Some(r)
}

fn fill_rect(img: &mut RgbaImage, a: Pos2, b: Pos2, c: [u8; 4]) -> Option<IRect> {
    let (w, h) = img.dimensions();
    let r = IRect::from_f(a, b, w, h);
    if r.is_empty() {
        return None;
    }
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            blend_px(&mut img.get_pixel_mut(x, y).0, &c, 1.0);
        }
    }
    Some(r)
}

/// Ellipse im Rechteck a–b: Rand der Stärke `width`, optional gefüllt
fn draw_ellipse(img: &mut RgbaImage, a: Pos2, b: Pos2, width: f32, c: [u8; 4], fill: Option<[u8; 4]>) -> Option<IRect> {
    let (w, h) = img.dimensions();
    let half = width / 2.0;
    let r = IRect::from_f(pos2(a.x.min(b.x) - half - 1.0, a.y.min(b.y) - half - 1.0), pos2(a.x.max(b.x) + half + 1.0, a.y.max(b.y) + half + 1.0), w, h);
    if r.is_empty() {
        return None;
    }
    let ctr = pos2((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    let (rx, ry) = (((b.x - a.x) / 2.0).abs().max(0.5), ((b.y - a.y) / 2.0).abs().max(0.5));
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let (dx, dy) = ((x as f32 + 0.5 - ctr.x) / rx, (y as f32 + 0.5 - ctr.y) / ry);
            let k = (dx * dx + dy * dy).sqrt();
            // genäherter Abstand zum Rand in Bildpunkten
            let dist = (k - 1.0) * rx.min(ry);
            if let Some(f) = fill {
                let cov = (0.5 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let mut s = f;
                    s[3] = (f[3] as f32 * cov).round() as u8;
                    blend_px(&mut img.get_pixel_mut(x, y).0, &s, 1.0);
                }
            }
            let cov = (half + 0.5 - dist.abs()).clamp(0.0, 1.0);
            if cov > 0.0 {
                let mut s = c;
                s[3] = (c[3] as f32 * cov).round() as u8;
                blend_px(&mut img.get_pixel_mut(x, y).0, &s, 1.0);
            }
        }
    }
    Some(r)
}

/// Fläche ab (x, y) füllen: Nachbarn mit ähnlicher Farbe (Abstand ≤ Toleranz)
fn flood_fill(img: &mut RgbaImage, x: u32, y: u32, c: [u8; 4], tol: f32) -> Option<IRect> {
    let (w, h) = img.dimensions();
    let seed = img.get_pixel(x, y).0;
    if seed == c {
        return None;
    }
    let near = |p: &[u8; 4]| {
        let d: f32 = (0..4).map(|k| (p[k] as f32 - seed[k] as f32).powi(2)).sum::<f32>().sqrt();
        d <= tol
    };
    let mut seen = vec![false; (w * h) as usize];
    let mut stack = vec![(x, y)];
    let mut r = IRect { x0: x, y0: y, x1: x + 1, y1: y + 1 };
    while let Some((x, y)) = stack.pop() {
        let i = (y * w + x) as usize;
        if seen[i] || !near(&img.get_pixel(x, y).0) {
            continue;
        }
        seen[i] = true;
        img.put_pixel(x, y, Rgba(c));
        r = r.union(IRect { x0: x, y0: y, x1: x + 1, y1: y + 1 });
        if x > 0 {
            stack.push((x - 1, y));
        }
        if x + 1 < w {
            stack.push((x + 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        if y + 1 < h {
            stack.push((x, y + 1));
        }
    }
    Some(r)
}

/// Farbe je Bildpunkt (0..1) umrechnen, Alpha bleibt
fn per_px(img: &mut RgbaImage, f: impl Fn([f32; 3]) -> [f32; 3] + Sync) {
    img.as_mut().par_chunks_mut(4).for_each(|p| {
        let v = f([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]);
        for c in 0..3 {
            p[c] = (v[c] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    });
}

fn luma(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// Anpassungen in einem Durchgang (plus Weichzeichnen/Schärfen/Vignette)
pub fn apply_adjust(img: &mut RgbaImage, a: &Adjust) {
    if a.blur > 0.0 {
        box_blur(img, a.blur.round() as u32);
    }
    if a.sharpen > 0.0 {
        unsharp(img, a.sharpen, 1.5);
    }
    let (w, h) = img.dimensions();
    let exp = 2f32.powf(a.exposure);
    let contrast = if a.contrast >= 0.0 { 1.0 + a.contrast * 2.0 } else { 1.0 + a.contrast };
    let gamma = 2f32.powf(-a.gamma);
    let (hs, hc) = (a.hue.to_radians().sin(), a.hue.to_radians().cos());
    let a = *a;
    img.as_mut().par_chunks_mut(w as usize * 4).enumerate().for_each(|(y, row)| {
        for (x, p) in row.chunks_exact_mut(4).enumerate() {
            let mut c = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
            for v in &mut c {
                *v *= exp;
            }
            c[0] += a.temperature * 0.12;
            c[2] -= a.temperature * 0.12;
            c[1] -= a.tint * 0.1;
            for v in &mut c {
                *v += a.brightness * 0.35;
                *v = (*v - 0.5) * contrast + 0.5;
                *v = v.clamp(0.0, 1.0).powf(gamma);
            }
            let l = luma(c);
            // Dynamik: wenig gesättigte Farben stärker
            let sat_now = c.iter().copied().fold(0.0f32, f32::max) - c.iter().copied().fold(1.0f32, f32::min);
            let s = 1.0 + a.saturation + a.vibrance * (1.0 - sat_now);
            for v in &mut c {
                *v = l + (*v - l) * s;
            }
            if a.hue != 0.0 {
                // Drehung im YIQ-Farbraum
                let yy = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
                let i = 0.596 * c[0] - 0.274 * c[1] - 0.322 * c[2];
                let q = 0.211 * c[0] - 0.523 * c[1] + 0.312 * c[2];
                let (i2, q2) = (i * hc - q * hs, i * hs + q * hc);
                c = [yy + 0.956 * i2 + 0.621 * q2, yy - 0.272 * i2 - 0.647 * q2, yy - 1.106 * i2 + 1.703 * q2];
            }
            if a.vignette != 0.0 {
                let (dx, dy) = (x as f32 / w as f32 - 0.5, y as f32 / h as f32 - 0.5);
                let d = ((dx * dx + dy * dy).sqrt() / 0.7071).clamp(0.0, 1.0);
                let f = 1.0 - a.vignette * d * d;
                for v in &mut c {
                    *v *= f;
                }
            }
            for k in 0..3 {
                p[k] = (c[k] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    });
}

/// Kastenweichzeichner, drei Durchgänge (≈ Gauß), getrennt nach Zeilen und Spalten
pub fn box_blur(img: &mut RgbaImage, radius: u32) {
    if radius == 0 {
        return;
    }
    let r = radius as usize;
    // eine Zeile: gleitende Summe über das Fenster [x−r, x+r] (am Rand kürzer) – Aufwand unabhängig vom Radius
    let pass = |buf: &mut [u8], w: usize| {
        buf.par_chunks_mut(w * 4).for_each(|row| {
            let src = row.to_vec();
            let mut s = [0u32; 4];
            for i in 0..=r.min(w - 1) {
                for c in 0..4 {
                    s[c] += src[i * 4 + c] as u32;
                }
            }
            for x in 0..w {
                let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
                let n = (b - a + 1) as u32;
                for c in 0..4 {
                    row[x * 4 + c] = (s[c] / n) as u8;
                }
                // Fenster eins weiter schieben
                if x + r + 1 < w {
                    for c in 0..4 {
                        s[c] += src[(x + r + 1) * 4 + c] as u32;
                    }
                }
                if x >= r {
                    for c in 0..4 {
                        s[c] -= src[(x - r) * 4 + c] as u32;
                    }
                }
            }
        });
    };
    for _ in 0..3 {
        // Zeilen
        let w = img.width() as usize;
        pass(img.as_mut(), w);
        // Spalten: über das gedrehte Bild
        let mut t = image::imageops::rotate90(img);
        let tw = t.width() as usize;
        pass(t.as_mut(), tw);
        *img = image::imageops::rotate270(&t);
    }
}

/// Unscharf maskieren: Bild + Stärke × (Bild − weichgezeichnet)
fn unsharp(img: &mut RgbaImage, amount: f32, radius: f32) {
    let mut blurred = img.clone();
    box_blur(&mut blurred, radius.round().max(1.0) as u32);
    img.as_mut().par_chunks_mut(4).zip(blurred.as_raw().par_chunks(4)).for_each(|(p, b)| {
        for c in 0..3 {
            let v = p[c] as f32 + amount * (p[c] as f32 - b[c] as f32);
            p[c] = v.round().clamp(0.0, 255.0) as u8;
        }
    });
}

/// Tonwerte strecken (je Kanal 0,5 % abschneiden)
fn auto_contrast(img: &mut RgbaImage) {
    let mut hist = [[0u32; 256]; 3];
    for p in img.as_raw().chunks_exact(4) {
        for c in 0..3 {
            hist[c][p[c] as usize] += 1;
        }
    }
    let total = (img.width() * img.height()) as u32;
    let cut = total / 200;
    let mut lo = [0u8; 3];
    let mut hi = [255u8; 3];
    for c in 0..3 {
        let mut acc = 0;
        for (i, v) in hist[c].iter().enumerate() {
            acc += v;
            if acc > cut {
                lo[c] = i as u8;
                break;
            }
        }
        acc = 0;
        for (i, v) in hist[c].iter().enumerate().rev() {
            acc += v;
            if acc > cut {
                hi[c] = i as u8;
                break;
            }
        }
    }
    img.as_mut().par_chunks_mut(4).for_each(|p| {
        for c in 0..3 {
            let span = (hi[c] as f32 - lo[c] as f32).max(1.0);
            p[c] = (((p[c] as f32 - lo[c] as f32) / span) * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    });
}

/// Grauwelt-Weißabgleich: Kanäle auf denselben Mittelwert bringen
fn auto_white(img: &mut RgbaImage) {
    let mut sum = [0f64; 3];
    let mut n = 0f64;
    for p in img.as_raw().chunks_exact(4) {
        if p[3] > 0 {
            for c in 0..3 {
                sum[c] += p[c] as f64;
            }
            n += 1.0;
        }
    }
    if n == 0.0 {
        return;
    }
    let avg = [sum[0] / n, sum[1] / n, sum[2] / n];
    let grey = (avg[0] + avg[1] + avg[2]) / 3.0;
    let k = [grey / avg[0].max(1.0), grey / avg[1].max(1.0), grey / avg[2].max(1.0)];
    img.as_mut().par_chunks_mut(4).for_each(|p| {
        for c in 0..3 {
            p[c] = (p[c] as f64 * k[c]).round().clamp(0.0, 255.0) as u8;
        }
    });
}

fn flatten_white(img: &RgbaImage) -> image::RgbImage {
    image::RgbImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y).0;
        let a = p[3] as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        image::Rgb([mix(p[0]), mix(p[1]), mix(p[2])])
    })
}

/// Schrift für das Text-Werkzeug: Segoe UI, sonst die eingebaute Schrift von egui
fn load_font() -> Option<ab_glyph::FontVec> {
    #[cfg(windows)]
    if let Some(f) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf").ok().and_then(|b| ab_glyph::FontVec::try_from_vec(b).ok()) {
        return Some(f);
    }
    let defs = egui::FontDefinitions::default();
    defs.font_data.values().find_map(|d| ab_glyph::FontVec::try_from_vec(d.font.to_vec()).ok())
}

/// Text (mehrzeilig) ab `at` (linke obere Ecke) mit Kantenglättung einzeichnen
fn draw_text(img: &mut RgbaImage, font: &ab_glyph::FontVec, text: &str, at: Pos2, px: f32, c: [u8; 4]) -> Option<IRect> {
    use ab_glyph::{Font, ScaleFont};
    let (w, h) = img.dimensions();
    let sf = font.as_scaled(px);
    let mut dirty: Option<IRect> = None;
    let mut y = at.y + sf.ascent();
    for line in text.lines() {
        let mut x = at.x;
        let mut prev: Option<ab_glyph::GlyphId> = None;
        for ch in line.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(px, ab_glyph::point(x, y));
            x += sf.h_advance(id);
            prev = Some(id);
            if let Some(o) = font.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|gx, gy, cov| {
                    let (ix, iy) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if ix < 0 || iy < 0 || ix as u32 >= w || iy as u32 >= h || cov <= 0.0 {
                        return;
                    }
                    let mut s = c;
                    s[3] = (c[3] as f32 * cov.min(1.0)).round() as u8;
                    blend_px(&mut img.get_pixel_mut(ix as u32, iy as u32).0, &s, 1.0);
                });
                dirty = merge_rect(dirty, Some(IRect::from_f(pos2(b.min.x, b.min.y), pos2(b.max.x, b.max.y), w, h)));
            }
        }
        y += sf.height() + sf.line_gap();
    }
    dirty
}

/// gestricheltes, laufendes Rechteck (Auswahl)
fn dashed(p: &egui::Painter, r: Rect, t: f32) {
    let edges = [(r.left_top(), r.right_top()), (r.right_top(), r.right_bottom()), (r.right_bottom(), r.left_bottom()), (r.left_bottom(), r.left_top())];
    p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK));
    for (a, b) in edges {
        let len = (b - a).length();
        let d = (b - a) / len.max(1.0);
        let mut s = -(t % 12.0);
        while s < len {
            let (s0, s1) = (s.max(0.0), (s + 6.0).min(len));
            if s1 > s0 {
                p.line_segment([a + d * s0, a + d * s1], Stroke::new(1.0, Color32::WHITE));
            }
            s += 12.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_normal_over_transparent() {
        let mut d = [0, 0, 0, 0];
        blend_px(&mut d, &[200, 100, 50, 255], 1.0);
        assert_eq!(d, [200, 100, 50, 255]);
        let mut d = [0, 0, 255, 255];
        blend_px(&mut d, &[255, 0, 0, 255], 0.5);
        assert!((d[0] as i32 - 128).abs() <= 1 && (d[2] as i32 - 128).abs() <= 1 && d[3] == 255, "{d:?}");
    }

    #[test]
    fn fill_and_line() {
        let mut img = RgbaImage::from_pixel(20, 20, Rgba([255, 255, 255, 255]));
        draw_line(&mut img, pos2(0.0, 10.0), pos2(20.0, 10.0), 2.0, [0, 0, 0, 255]);
        assert!(img.get_pixel(10, 9)[0] < 60 || img.get_pixel(10, 10)[0] < 60);
        // oberhalb der Linie füllen: unterhalb bleibt weiß
        flood_fill(&mut img, 5, 2, [255, 0, 0, 255], 10.0);
        assert_eq!(img.get_pixel(5, 2).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(5, 18).0, [255, 255, 255, 255]);
    }

    #[test]
    fn adjust_neutral_is_identity() {
        let mut img = RgbaImage::from_fn(8, 8, |x, y| Rgba([(x * 30) as u8, (y * 30) as u8, 77, 255]));
        let before = img.clone();
        apply_adjust(&mut img, &Adjust::default());
        assert_eq!(img, before);
    }

    #[test]
    fn blur_keeps_flat_color() {
        let mut img = RgbaImage::from_pixel(16, 9, Rgba([10, 20, 30, 255]));
        box_blur(&mut img, 3);
        assert_eq!(img.get_pixel(5, 5).0, [10, 20, 30, 255]);
        assert_eq!(img.dimensions(), (16, 9));
    }
}
