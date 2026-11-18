//! Stapelverarbeitung für Bilder: mehrere Bilder markieren → Größe ändern, Format umwandeln, Hintergrund
//! entfernen, hochskalieren. Läuft im Hintergrund mit Fortschritt in der Statusleiste; die Ergebnisse landen
//! neben den Originalen (Name mit Zusatz), nichts wird überschrieben.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

use crate::themes::Theme;
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Resize {
    Keep,
    /// längste Seite in Pixeln
    MaxSide(u32),
    /// Prozent
    Percent(u32),
}

#[derive(Clone)]
pub struct Opts {
    pub paths: Vec<PathBuf>,
    pub resize: Resize,
    /// Zielformat ("" = wie das Original)
    pub format: String,
    pub quality: u8,
    pub remove_bg: bool,
    pub upscale: u32,
    pub drawing: bool,
    /// in einen Unterordner „Bearbeitet“ statt daneben
    pub subfolder: bool,
}

impl Opts {
    pub fn new(paths: Vec<PathBuf>) -> Opts {
        Opts { paths, resize: Resize::Keep, format: String::new(), quality: 90, remove_bg: false, upscale: 1, drawing: false, subfolder: true }
    }

    fn is_empty(&self) -> bool {
        self.resize == Resize::Keep && self.format.is_empty() && !self.remove_bg && self.upscale <= 1
    }

    pub fn label(&self) -> String {
        let mut v = Vec::new();
        if self.remove_bg {
            v.push("Hintergrund entfernen".to_string());
        }
        if self.upscale > 1 {
            v.push(format!("{}× hochskalieren", self.upscale));
        }
        match self.resize {
            Resize::MaxSide(px) => v.push(format!("max. {px} px")),
            Resize::Percent(p) => v.push(format!("{p} %")),
            Resize::Keep => {}
        }
        if !self.format.is_empty() {
            v.push(format!("→ .{}", self.format));
        }
        v.join(", ")
    }
}

pub fn is_image(p: &Path) -> bool {
    crate::convert::class_of(p) == crate::convert::Class::Image && !matches!(crate::convert::ext_of(p).as_str(), "svg")
}

/// Ziel einer Datei: Unterordner „Bearbeitet“ oder daneben mit „-bearbeitet“
fn dest(src: &Path, o: &Opts, ext: &str) -> PathBuf {
    let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (dir, stem) = if o.subfolder { (dir.join("Bearbeitet"), stem) } else { (dir, format!("{stem}-bearbeitet")) };
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join(format!("{stem}.{ext}"));
    if !p.exists() {
        return p;
    }
    (2..).map(|i| dir.join(format!("{stem} ({i}).{ext}"))).find(|p| !p.exists()).unwrap_or(p)
}

/// ein Bild bearbeiten
fn one(src: &Path, o: &Opts, p: &crate::localai::Progress) -> Result<PathBuf, String> {
    let img = crate::convert::load_image(src, u32::MAX).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut rgba = img.to_rgba8();
    // KI-Schritte zuerst (Freistellen braucht das Original, Hochskalieren vor dem Verkleinern ergibt keinen Sinn –
    // wer beides wählt, bekommt erst das große Bild und dann die gewünschte Größe)
    if o.remove_bg || o.upscale > 1 {
        let chain = crate::localai::Chain { bg: if o.remove_bg { crate::localai::Bg::Remove } else { crate::localai::Bg::Keep }, upscale: o.upscale, drawing: o.drawing, ..Default::default() };
        rgba = crate::localai::run_chain(rgba, None, &chain, p)?;
    }
    let (w, h) = rgba.dimensions();
    let (nw, nh) = match o.resize {
        Resize::Keep => (w, h),
        Resize::MaxSide(m) => {
            let s = (m as f32 / w.max(h) as f32).min(1.0);
            ((w as f32 * s).round().max(1.0) as u32, (h as f32 * s).round().max(1.0) as u32)
        }
        Resize::Percent(pc) => ((w * pc / 100).max(1), (h * pc / 100).max(1)),
    };
    if (nw, nh) != (w, h) {
        rgba = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Lanczos3);
    }
    let src_ext = crate::convert::ext_of(src);
    let mut ext = if o.format.is_empty() { src_ext.clone() } else { o.format.clone() };
    // Formate, die das Programm nicht schreiben kann, und JPEG nach dem Freistellen (keine Transparenz) → PNG
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" | "gif" | "tga" | "qoi" | "ico") || (o.remove_bg && matches!(ext.as_str(), "jpg" | "jpeg" | "bmp")) {
        ext = "png".into();
    }
    let out = dest(src, o, &ext);
    let dynimg = image::DynamicImage::ImageRgba8(rgba);
    match ext.as_str() {
        "jpg" | "jpeg" => {
            let f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), o.quality.clamp(10, 100));
            dynimg.to_rgb8().write_with_encoder(enc).map_err(|e| e.to_string())?;
        }
        "bmp" => dynimg.to_rgb8().save(&out).map_err(|e| e.to_string())?,
        "ico" => {
            let small = if dynimg.width() > 256 || dynimg.height() > 256 { dynimg.thumbnail(256, 256) } else { dynimg };
            small.save(&out).map_err(|e| e.to_string())?
        }
        _ => dynimg.save(&out).map_err(|e| e.to_string())?,
    }
    Ok(out)
}

/// alles bearbeiten; `done` zählt fertige Bilder. Ergebnis: Statusmeldung
pub fn run(o: Opts, done: &AtomicU64, cancel: &AtomicBool) -> String {
    let p = crate::localai::Progress::default();
    let (mut ok, mut errs) = (0, Vec::new());
    for src in &o.paths {
        if cancel.load(Relaxed) {
            break;
        }
        match one(src, &o, &p) {
            Ok(_) => ok += 1,
            Err(e) => errs.push(e),
        }
        done.fetch_add(1, Relaxed);
    }
    let mut msg = format!("{ok} von {} Bildern bearbeitet ({})", o.paths.len(), o.label());
    if cancel.load(Relaxed) {
        msg.push_str(" – abgebrochen");
    }
    if let Some(e) = errs.first() {
        msg.push_str(&format!(" – Fehler: {e}"));
    }
    msg
}

/// Fenster „Bilder bearbeiten“. Rückgabe Some(true) = starten, Some(false) = schließen
pub fn window(ctx: &egui::Context, th: &Theme, o: &mut Opts) -> Option<bool> {
    let mut open = true;
    let mut res = None;
    egui::Window::new(format!("{} Bilder bearbeiten", o.paths.len()))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(420.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(egui::RichText::new("Größe").strong());
            ui.horizontal_wrapped(|ui| {
                for (r, l) in [
                    (Resize::Keep, "Unverändert"),
                    (Resize::MaxSide(3840), "4K"),
                    (Resize::MaxSide(1920), "1920 px"),
                    (Resize::MaxSide(1280), "1280 px"),
                    (Resize::MaxSide(800), "800 px"),
                    (Resize::Percent(50), "50 %"),
                    (Resize::Percent(25), "25 %"),
                ] {
                    if ui.selectable_label(o.resize == r, l).on_hover_text(match r {
                        Resize::MaxSide(_) => "Längste Seite höchstens so groß (kleinere bleiben, wie sie sind)",
                        _ => "",
                    }).clicked() {
                        o.resize = r;
                    }
                }
            });
            ui.label(egui::RichText::new("Format").strong());
            ui.horizontal_wrapped(|ui| {
                for (f, l) in [("", "Wie Original"), ("jpg", "JPEG"), ("png", "PNG"), ("webp", "WebP"), ("bmp", "BMP"), ("tif", "TIFF"), ("ico", "ICO")] {
                    if ui.selectable_label(o.format == f, l).clicked() {
                        o.format = f.to_string();
                    }
                }
            });
            if matches!(o.format.as_str(), "jpg") || (o.format.is_empty() && o.paths.iter().any(|p| matches!(crate::convert::ext_of(p).as_str(), "jpg" | "jpeg"))) {
                ui.horizontal(|ui| {
                    ui.label("JPEG-Qualität");
                    ui.add(egui::Slider::new(&mut o.quality, 40..=100));
                });
            }
            ui.label(egui::RichText::new("KI (lokal)").strong());
            let ai = crate::localai::supported();
            ui.add_enabled_ui(ai, |ui| {
                ui.checkbox(&mut o.remove_bg, "Hintergrund entfernen (transparentes PNG)");
                ui.horizontal(|ui| {
                    ui.label("Hochskalieren");
                    for (f, l) in [(1, "Aus"), (2, "2×"), (4, "4×")] {
                        if ui.selectable_label(o.upscale == f, l).clicked() {
                            o.upscale = f;
                        }
                    }
                    if o.upscale > 1 {
                        ui.checkbox(&mut o.drawing, "Zeichnung/Anime");
                    }
                });
            });
            if !ai {
                ui.label(egui::RichText::new("Die KI-Schritte gibt es nur unter Windows (x64).").size(12.0).color(th.sub));
            } else if o.remove_bg || o.upscale > 1 {
                ui.label(egui::RichText::new("Fehlende Modelle werden beim ersten Mal automatisch geladen.").size(12.0).color(th.sub));
            }
            ui.separator();
            ui.checkbox(&mut o.subfolder, "In Unterordner „Bearbeitet“ speichern (sonst daneben mit „-bearbeitet“)");
            ui.horizontal(|ui| {
                if ui.add_enabled(!o.is_empty(), egui::Button::new(egui::RichText::new("Starten").strong())).clicked() {
                    res = Some(true);
                }
                if ui.button("Abbrechen").clicked() {
                    res = Some(false);
                }
                if !o.is_empty() {
                    ui.label(egui::RichText::new(o.label()).size(12.0).color(th.sub));
                }
            });
        });
    if !open {
        res = Some(false);
    }
    res
}
