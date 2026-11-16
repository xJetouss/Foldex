//! Videoeditor im großen Preview (Vorschau als eigener Tab) – bewusst einfach und schnell: Bereich in der
//! Zeitleiste (mit Vorschaubildern) markieren, zuschneiden (Auswahl behalten) oder herausschneiden; Ton stumm
//! oder lauter/leiser, Geschwindigkeit, Drehen, Auflösung, Format (MP4, MKV, MOV, WebM).
//!
//! Nicht zerstörend: Bearbeitet wird eine Liste der behaltenen Abschnitte; die Wiedergabe springt über
//! herausgeschnittene Stellen. Beim Speichern rechnet ffmpeg alles in einem Durchgang. Ist nur ein Stück
//! herausgeschnitten und sonst nichts geändert, geht es ohne Neukodierung (sekundenschnell, Schnitt an
//! Schlüsselbildern).

use crate::edicons::{self, E};
use crate::media::{self, Player};
use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, FontId, Key, Rect, Stroke, TextureHandle};
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicU32, Ordering},
    sync::mpsc::{self, Receiver},
    sync::Arc,
    time::Duration,
};

const FORMATS: &[(&str, &str)] = &[("mp4", "MP4 (H.264)"), ("mkv", "MKV (H.264)"), ("mov", "MOV (H.264)"), ("webm", "WebM (VP9)")];
const HEIGHTS: &[(u32, &str)] = &[(0, "Original"), (2160, "2160p"), (1440, "1440p"), (1080, "1080p"), (720, "720p"), (480, "480p"), (360, "360p")];
/// Vorschaubilder der Zeitleiste
const THUMBS: usize = 24;

/// Bearbeitungsstand (für Rückgängig)
#[derive(Clone, PartialEq, Debug)]
struct State {
    /// behaltene Abschnitte der Datei (Sekunden), in Abspielreihenfolge
    segs: Vec<(f64, f64)>,
    mute: bool,
    gain_db: f32,
    speed: f32,
    /// Drehung im Uhrzeigersinn: 0, 90, 180, 270
    rotate: u32,
    /// Höhe des Ergebnisses (0 = wie die Datei)
    height: u32,
}

impl State {
    fn total(&self) -> f64 {
        self.segs.iter().map(|s| s.1 - s.0).sum()
    }
    fn to_src(&self, t: f64) -> f64 {
        let mut acc = 0.0;
        for s in &self.segs {
            let len = s.1 - s.0;
            if t <= acc + len {
                return s.0 + (t - acc).max(0.0);
            }
            acc += len;
        }
        self.segs.last().map_or(0.0, |s| s.1)
    }
    fn to_edit(&self, src: f64) -> Option<f64> {
        let mut acc = 0.0;
        for s in &self.segs {
            if src >= s.0 - 1e-6 && src <= s.1 + 1e-6 {
                return Some(acc + (src - s.0).clamp(0.0, s.1 - s.0));
            }
            acc += s.1 - s.0;
        }
        None
    }
    fn delete(&mut self, a: f64, b: f64) {
        let mut out = Vec::new();
        let mut acc = 0.0;
        for &(s0, s1) in &self.segs {
            let (e0, e1) = (acc, acc + (s1 - s0));
            if a > e0 {
                out.push((s0, s0 + (a.min(e1) - e0)));
            }
            if b < e1 {
                out.push((s0 + (b.max(e0) - e0), s1));
            }
            acc = e1;
        }
        self.segs = out.into_iter().filter(|s| s.1 - s.0 > 1e-3).collect();
    }
    fn keep(&mut self, a: f64, b: f64) {
        let total = self.total();
        self.delete(b, total);
        self.delete(0.0, a);
    }
    /// ohne Neukodierung möglich: ein Abschnitt, sonst nichts geändert
    fn copyable(&self) -> bool {
        self.segs.len() == 1 && !self.mute && self.gain_db == 0.0 && self.speed == 1.0 && self.rotate == 0 && self.height == 0
    }
}

pub enum VideoOut {
    Close,
    /// Datei überschrieben – Vorschau neu laden
    Saved(PathBuf),
}

struct Job {
    rx: Receiver<Result<PathBuf, String>>,
    /// Fortschritt in Promille
    prog: Arc<AtomicU32>,
    overwrite: bool,
}

pub struct VideoEditor {
    path: PathBuf,
    info: media::Info,
    player: Player,
    thumbs: Vec<Option<TextureHandle>>,
    thumbs_rx: Option<Receiver<(usize, egui::ColorImage)>>,
    st: State,
    undo: Vec<State>,
    redo: Vec<State>,
    sel: Option<(f64, f64)>,
    drag_from: Option<f64>,
    play_to: Option<f64>,
    cursor: f64,
    format: String,
    /// ohne Neukodierung speichern, wenn möglich
    fast: bool,
    fx_session: bool,
    job: Option<Job>,
    msg: Option<(bool, String)>,
    pub modified: bool,
}

impl VideoEditor {
    /// `poster`: Standbild der Vorschau (bis das erste Bild dekodiert ist)
    pub fn open(path: &Path, info: &media::Info, ctx: &egui::Context, volume: f32, poster: Option<TextureHandle>) -> Result<VideoEditor, String> {
        if crate::convert::ffmpeg().is_none() {
            return Err("Der Videoeditor braucht ffmpeg.".into());
        }
        if info.duration <= 0.0 {
            return Err("Die Länge des Videos ist unbekannt.".into());
        }
        let ext = crate::convert::ext_of(path);
        let format = if FORMATS.iter().any(|f| f.0 == ext) { ext } else { "mp4".into() };
        let mut player = Player::new(path.to_path_buf(), info.clone(), ctx, volume);
        // im Editor kein Vollbild/Bild-in-Bild (der Doppelklick ins Bild bleibt ohne Wirkung)
        player.pip = true;
        player.poster = poster;
        Ok(VideoEditor {
            path: path.to_path_buf(),
            info: info.clone(),
            player,
            thumbs: vec![None; THUMBS],
            thumbs_rx: Some(thumbnails(path, info.duration, ctx)),
            st: State { segs: vec![(0.0, info.duration)], mute: false, gain_db: 0.0, speed: 1.0, rotate: 0, height: 0 },
            undo: Vec::new(),
            redo: Vec::new(),
            sel: None,
            drag_from: None,
            play_to: None,
            cursor: 0.0,
            format,
            fast: true,
            fx_session: false,
            job: None,
            msg: None,
            modified: false,
        })
    }

    /// Nur für Bildschirmfotos: ein Stück herausschneiden, Auswahl setzen
    pub fn dev_demo(&mut self) {
        let t = self.st.total();
        self.edit(|s| s.delete(t * 0.55, t * 0.7));
        self.sel = Some((t * 0.15, t * 0.35));
        self.show_at(t * 0.2);
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        if self.player.last_frame + 1 < ctx.cumulative_pass_nr() {
            self.player.pause();
        }
    }

    fn edit(&mut self, f: impl FnOnce(&mut State)) {
        let before = self.st.clone();
        f(&mut self.st);
        if self.st != before {
            self.undo.push(before);
            self.redo.clear();
            self.modified = true;
            self.sel = None;
            self.cursor = self.cursor.min(self.st.total());
        }
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.st, s));
            self.modified = true;
            self.sel = None;
            self.cursor = self.cursor.min(self.st.total());
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.st, s));
            self.modified = true;
            self.sel = None;
            self.cursor = self.cursor.min(self.st.total());
        }
    }

    fn pos(&self) -> f64 {
        if self.player.is_paused() {
            self.cursor
        } else {
            self.st.to_edit(self.player.pos()).unwrap_or(self.cursor)
        }
    }

    fn play_from(&mut self, t: f64, until: Option<f64>) {
        self.cursor = t;
        self.play_to = until;
        self.player.seek(self.st.to_src(t));
        if self.player.is_paused() {
            self.player.toggle();
        }
    }

    fn toggle(&mut self) {
        if self.player.is_paused() {
            let total = self.st.total();
            let t = if self.cursor >= total - 0.05 { 0.0 } else { self.cursor };
            let until = self.sel.filter(|s| t >= s.0 - 1e-3 && t < s.1).map(|s| s.1);
            self.play_from(t, until);
        } else {
            self.cursor = self.pos();
            self.player.pause();
        }
    }

    /// angehalten an eine Stelle: Standbild dort zeigen
    fn show_at(&mut self, t: f64) {
        self.cursor = t;
        self.player.seek(self.st.to_src(t));
    }

    /// Wiedergabe über herausgeschnittene Stellen springen lassen, am Ende/Auswahlende anhalten
    fn follow(&mut self, ctx: &egui::Context) {
        if self.player.is_paused() {
            return;
        }
        let src = self.player.pos();
        match self.st.to_edit(src) {
            Some(t) => {
                self.cursor = t;
                if self.play_to.is_some_and(|e| t >= e) || t >= self.st.total() - 0.02 {
                    self.player.pause();
                    self.cursor = self.play_to.take().unwrap_or(self.st.total()).min(self.st.total());
                }
            }
            None => match self.st.segs.iter().find(|s| s.0 > src) {
                Some(s) => self.player.seek(s.0),
                None => {
                    self.player.pause();
                    self.cursor = self.st.total();
                }
            },
        }
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme, rect: Rect) -> Option<VideoOut> {
        let ctx = ui.ctx().clone();
        self.player.last_frame = ctx.cumulative_pass_nr();
        self.follow(&ctx);
        if let Some(rx) = &self.thumbs_rx {
            loop {
                match rx.try_recv() {
                    Ok((i, img)) => self.thumbs[i] = Some(ctx.load_texture(format!("vthumb{i}"), img, egui::TextureOptions::LINEAR)),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.thumbs_rx = None;
                        break;
                    }
                }
            }
        }
        let mut out = None;
        if let Some(j) = &self.job {
            if let Ok(r) = j.rx.try_recv() {
                let overwrite = j.overwrite;
                self.job = None;
                match r {
                    Ok(p) if overwrite => {
                        self.modified = false;
                        out = Some(VideoOut::Saved(p));
                    }
                    Ok(p) => {
                        self.modified = false;
                        self.msg = Some((false, format!("Gespeichert: {}", p.display())));
                    }
                    Err(e) => self.msg = Some((true, e)),
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
        ui.painter().rect_filled(rect, 0.0, th.panel);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(14.0)));
        let ui = &mut child;
        // Kopfzeile
        ui.horizontal(|ui| {
            let name = self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            ui.label(egui::RichText::new(name).size(15.0).strong());
            if self.modified {
                ui.label(egui::RichText::new("●").color(th.accent)).on_hover_text("Ungespeicherte Änderungen");
            }
            ui.label(egui::RichText::new(format!("{}  →  {}", hms(self.info.duration), hms(self.st.total() / self.st.speed as f64))).color(th.sub)).on_hover_text("Länge vorher → nachher");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if edicons::btn(ui, th, E::Close, if self.modified { "Schließen – ungespeicherte Änderungen gehen verloren" } else { "Schließen" }, true).clicked() {
                    self.player.pause();
                    out = Some(VideoOut::Close);
                }
                let busy = self.job.is_some();
                if edicons::labeled(ui, th, E::SaveAs, "Speichern unter", "Als neue Datei", !busy, false).clicked() {
                    let stem = self.path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    if let Some(p) = rfd::FileDialog::new()
                        .set_directory(self.path.parent().unwrap_or(Path::new(".")))
                        .set_file_name(format!("{stem} (bearbeitet).{}", self.format))
                        .add_filter("Video", &FORMATS.iter().map(|f| f.0).collect::<Vec<_>>())
                        .save_file()
                    {
                        self.export(p, false);
                    }
                }
                let same = crate::convert::ext_of(&self.path) == self.format;
                if edicons::labeled(ui, th, E::Save, "Speichern", if same { "Datei überschreiben (Strg+S)" } else { "Anderes Format: „Speichern unter“" }, self.modified && same && !busy, true).clicked() {
                    self.export(self.path.clone(), true);
                }
                if let Some(j) = &self.job {
                    let f = j.prog.load(Ordering::Relaxed) as f32 / 1000.0;
                    ui.add(egui::ProgressBar::new(f).desired_width(160.0).show_percentage());
                }
            });
        });
        ui.add_space(8.0);
        // Video
        let video_h = (ui.available_height() - 210.0).max(120.0);
        let (vr, _) = ui.allocate_exact_size(vec2(ui.available_width(), video_h), egui::Sense::hover());
        let a = self.player.aspect();
        let (w, h) = if vr.width() / vr.height() > a { (vr.height() * a, vr.height()) } else { (vr.width(), vr.width() / a) };
        let vbox = Rect::from_center_size(vr.center(), vec2(w, h));
        ui.painter().rect_filled(vr, 8.0, Color32::BLACK);
        let was = self.player.is_paused();
        self.player.video(ui, vbox);
        // Klick ins Bild hat Abspielen/Pause umgeschaltet: Stelle im bearbeiteten Stück mitführen
        if was && !self.player.is_paused() {
            let t = self.cursor;
            self.play_from(t, None);
        } else if !was && self.player.is_paused() {
            self.cursor = self.st.to_edit(self.player.pos()).unwrap_or(self.cursor);
        }
        ui.add_space(8.0);
        // Zeitleiste
        let (tr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 74.0), egui::Sense::click_and_drag());
        self.timeline(ui, th, tr, &resp);
        ui.add_space(8.0);
        // Transport und Bearbeiten
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let playing = !self.player.is_paused();
            let (r, pr) = ui.allocate_exact_size(vec2(40.0, 40.0), egui::Sense::click());
            ui.painter().circle_filled(r.center(), 20.0, if pr.hovered() { th.accent.gamma_multiply(0.85) } else { th.accent });
            edicons::draw(ui.painter(), Rect::from_center_size(r.center() + vec2(if playing { 0.0 } else { 1.5 }, 0.0), vec2(16.0, 16.0)), if playing { E::Pause } else { E::Play }, Color32::WHITE);
            if pr.on_hover_text(if playing { "Pause (Leertaste)" } else { "Abspielen (Leertaste) – mit Auswahl nur die Auswahl" }).clicked() {
                self.toggle();
            }
            if edicons::btn(ui, th, E::ToStart, "Zum Anfang (Pos1)", true).clicked() {
                if playing {
                    self.play_from(0.0, None);
                } else {
                    self.show_at(0.0);
                }
            }
            egui::Frame::none().fill(th.field).rounding(6.0).inner_margin(egui::Margin::symmetric(10.0, 6.0)).show(ui, |ui| {
                ui.label(egui::RichText::new(format!("{}  /  {}", hms_ms(self.pos()), hms_ms(self.st.total()))).monospace().size(13.0));
            });
            vsep(ui, th);
            let has = self.sel.is_some_and(|s| s.1 - s.0 > 0.04);
            if edicons::labeled(ui, th, E::Keep, "Zuschneiden", "Nur die Auswahl behalten", has, false).clicked() {
                if let Some((a, b)) = self.sel {
                    self.edit(|s| s.keep(a, b));
                }
            }
            if edicons::labeled(ui, th, E::Cut, "Ausschneiden", "Auswahl entfernen (Entf)", has, false).clicked() {
                if let Some((a, b)) = self.sel {
                    self.edit(|s| s.delete(a, b));
                }
            }
            if edicons::btn(ui, th, E::SelectAll, "Alles auswählen (Strg+A)", true).clicked() {
                self.sel = Some((0.0, self.st.total()));
            }
            vsep(ui, th);
            if edicons::btn(ui, th, E::Undo, "Rückgängig (Strg+Z)", !self.undo.is_empty()).clicked() {
                self.undo();
            }
            if edicons::btn(ui, th, E::Redo, "Wiederholen (Strg+Y)", !self.redo.is_empty()).clicked() {
                self.redo();
            }
            vsep(ui, th);
            // Drehen
            if edicons::btn(ui, th, E::RotL, "Nach links drehen", true).clicked() {
                let r = (self.st.rotate + 270) % 360;
                self.edit(|s| s.rotate = r);
            }
            if edicons::btn(ui, th, E::RotR, "Nach rechts drehen", true).clicked() {
                let r = (self.st.rotate + 90) % 360;
                self.edit(|s| s.rotate = r);
            }
            if self.st.rotate != 0 {
                ui.label(egui::RichText::new(format!("{}°", self.st.rotate)).color(th.accent)).on_hover_text("Wird beim Speichern angewendet");
            }
        });
        ui.add_space(8.0);
        // Ton, Geschwindigkeit, Ausgabe
        let mut st = self.st.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add_enabled_ui(self.info.has_audio, |ui| {
                ui.checkbox(&mut st.mute, "Stumm");
                ui.add_enabled(!st.mute, egui::Slider::new(&mut st.gain_db, -30.0..=20.0).suffix(" dB").text("Lautstärke"));
            });
            ui.add(egui::Slider::new(&mut st.speed, 0.25..=4.0).logarithmic(true).suffix("×").fixed_decimals(2).text("Tempo"));
            egui::ComboBox::from_id_salt("vid_h").selected_text(HEIGHTS.iter().find(|h| h.0 == st.height).map_or("", |h| h.1)).show_ui(ui, |ui| {
                for (hh, n) in HEIGHTS.iter().filter(|h| h.0 == 0 || h.0 <= self.info.height.max(self.info.width)) {
                    ui.selectable_value(&mut st.height, *hh, *n);
                }
            });
            egui::ComboBox::from_id_salt("vid_fmt").selected_text(FORMATS.iter().find(|f| f.0 == self.format).map_or("", |f| f.1)).show_ui(ui, |ui| {
                for (e, n) in FORMATS {
                    ui.selectable_value(&mut self.format, e.to_string(), *n);
                }
            });
            let copy = st.copyable() && FORMATS.iter().any(|f| f.0 == crate::convert::ext_of(&self.path)) && self.format == crate::convert::ext_of(&self.path);
            ui.add_enabled(copy, egui::Checkbox::new(&mut self.fast, "Schnell"))
                .on_hover_text("Ohne Neukodierung – in Sekunden fertig. Geschnitten wird am Schlüsselbild vor dem Anfang: Je nach Video beginnt das Ergebnis etwas (bis einige Sekunden) früher. Nur bei einem Abschnitt ohne weitere Änderungen.");
        });
        if st != self.st {
            if !self.fx_session {
                self.undo.push(self.st.clone());
                self.redo.clear();
                self.fx_session = true;
            }
            self.st = st;
            self.modified = true;
        }
        if !ui.input(|i| i.pointer.any_down()) {
            self.fx_session = false;
        }
        if let Some((err, m)) = &self.msg {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(m).color(if *err { th.danger } else { th.accent }));
        }
        self.keys(ui);
        out
    }

    fn keys(&mut self, ui: &egui::Ui) {
        if ui.ctx().wants_keyboard_input() {
            return;
        }
        let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
        if pressed(Key::Space) {
            self.toggle();
        } else if cmd && pressed(Key::Z) {
            if shift {
                self.redo();
            } else {
                self.undo();
            }
        } else if cmd && pressed(Key::Y) {
            self.redo();
        } else if cmd && pressed(Key::A) {
            self.sel = Some((0.0, self.st.total()));
        } else if cmd && pressed(Key::S) {
            if self.modified && crate::convert::ext_of(&self.path) == self.format && self.job.is_none() {
                self.export(self.path.clone(), true);
            }
        } else if pressed(Key::Delete) {
            if let Some((a, b)) = self.sel.filter(|s| s.1 - s.0 > 0.04) {
                self.edit(|s| s.delete(a, b));
            }
        } else if pressed(Key::Home) {
            self.show_at(0.0);
        } else if pressed(Key::Escape) {
            self.sel = None;
        } else if pressed(Key::ArrowLeft) || pressed(Key::ArrowRight) {
            // ein Bild bzw. mit Umschalt eine Sekunde weiter
            let step = if shift { 1.0 } else { 1.0 / self.info.fps.max(1.0) };
            let t = (self.pos() + if pressed(Key::ArrowRight) { step } else { -step }).clamp(0.0, self.st.total());
            self.player.pause();
            self.show_at(t);
        }
    }

    /// Zeitleiste: Vorschaubilder im bearbeiteten Ablauf, Schnittstellen, Auswahl, Abspielstelle
    fn timeline(&mut self, ui: &egui::Ui, th: &Theme, r: Rect, resp: &egui::Response) {
        let p = ui.painter_at(r);
        p.rect_filled(r, 8.0, th.field);
        let strip = r.shrink2(vec2(8.0, 18.0)).translate(vec2(0.0, 6.0));
        let total = self.st.total().max(0.001);
        let to_x = |t: f64| strip.left() + (t / total) as f32 * strip.width();
        let to_t = |x: f32| (((x - strip.left()) / strip.width()) as f64 * total).clamp(0.0, total);
        // Zeitmarken
        let step = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1200.0].into_iter().find(|s| total / s <= 10.0).unwrap_or(1800.0);
        let mut t = 0.0;
        while t <= total {
            let x = to_x(t);
            p.line_segment([pos2(x, r.top() + 12.0), pos2(x, r.top() + 16.0)], Stroke::new(1.0, th.sub));
            p.text(pos2(x + 2.0, r.top() + 2.0), egui::Align2::LEFT_TOP, hms(t), FontId::proportional(10.0), th.sub);
            t += step;
        }
        // Vorschaubilder: je Feld das zur Stelle passende Bild der Datei
        let cells = ((strip.width() / (strip.height() * 16.0 / 9.0)).ceil() as usize).max(1);
        let cw = strip.width() / cells as f32;
        for i in 0..cells {
            let cell = Rect::from_min_size(pos2(strip.left() + i as f32 * cw, strip.top()), vec2(cw, strip.height()));
            let src = self.st.to_src(to_t(cell.center().x));
            let k = ((src / self.info.duration.max(0.001)) * THUMBS as f64).floor().clamp(0.0, (THUMBS - 1) as f64) as usize;
            match &self.thumbs[k] {
                Some(t) => {
                    let s = t.size_vec2();
                    // mittig zuschneiden, damit das Feld gefüllt ist
                    let ka = (cell.width() / cell.height()) / (s.x / s.y);
                    let uv = if ka < 1.0 { Rect::from_min_max(pos2(0.5 - ka / 2.0, 0.0), pos2(0.5 + ka / 2.0, 1.0)) } else { Rect::from_min_max(pos2(0.0, 0.5 - 0.5 / ka), pos2(1.0, 0.5 + 0.5 / ka)) };
                    p.image(t.id(), cell.shrink(0.5), uv, Color32::WHITE);
                }
                None => {
                    p.rect_filled(cell.shrink(0.5), 0.0, th.bg);
                }
            }
        }
        p.rect_stroke(strip, 4.0, Stroke::new(1.0, th.line));
        // Schnittstellen
        let mut acc = 0.0;
        for s in &self.st.segs[..self.st.segs.len().saturating_sub(1)] {
            acc += s.1 - s.0;
            let x = to_x(acc);
            p.line_segment([pos2(x, strip.top() - 4.0), pos2(x, strip.bottom() + 4.0)], Stroke::new(2.0, th.danger));
        }
        // Auswahl
        if let Some((a, b)) = self.sel {
            let sr = Rect::from_min_max(pos2(to_x(a), strip.top()), pos2(to_x(b), strip.bottom()));
            p.rect_filled(sr, 0.0, th.accent.gamma_multiply(0.3));
            p.rect_stroke(sr, 0.0, Stroke::new(2.0, th.accent));
            p.text(sr.center_bottom() + vec2(0.0, 1.0), egui::Align2::CENTER_TOP, hms_ms(b - a), FontId::proportional(10.5), th.text);
        }
        // Abspielstelle
        let px = to_x(self.pos());
        p.line_segment([pos2(px, r.top() + 10.0), pos2(px, r.bottom() - 2.0)], Stroke::new(2.0, Color32::WHITE));
        p.add(egui::Shape::convex_polygon(vec![pos2(px - 5.0, r.top() + 8.0), pos2(px + 5.0, r.top() + 8.0), pos2(px, r.top() + 14.0)], Color32::WHITE, Stroke::NONE));
        // Eingaben: ziehen = auswählen, Klick = Stelle
        if let Some(h) = resp.hover_pos() {
            p.line_segment([pos2(h.x, strip.top()), pos2(h.x, strip.bottom())], Stroke::new(1.0, Color32::from_white_alpha(120)));
            egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), ui.id().with("vid_tip"), |ui| {
                ui.label(hms_ms(to_t(h.x)));
            });
        }
        if resp.drag_started() {
            self.drag_from = resp.interact_pointer_pos().map(|p| to_t(p.x));
        }
        if resp.dragged() {
            if let (Some(a), Some(p)) = (self.drag_from, resp.interact_pointer_pos()) {
                let b = to_t(p.x);
                self.sel = Some((a.min(b), a.max(b)));
            }
        }
        if resp.drag_stopped() {
            self.drag_from = None;
            if self.sel.is_some_and(|s| s.1 - s.0 < 0.04) {
                self.sel = None;
            }
        }
        if resp.clicked() {
            if let Some(pt) = resp.interact_pointer_pos() {
                let t = to_t(pt.x);
                self.sel = None;
                if self.player.is_paused() {
                    self.show_at(t);
                } else {
                    self.play_from(t, None);
                }
            }
        }
    }

    /// Mit ffmpeg speichern. Schnell (ohne Neukodierung), wenn möglich und gewünscht; sonst neu kodieren.
    /// `overwrite`: erst in eine temporäre Datei, dann ersetzen.
    fn export(&mut self, dst: PathBuf, overwrite: bool) {
        let Some(ff) = crate::convert::ffmpeg() else {
            self.msg = Some((true, "ffmpeg ist nicht installiert".into()));
            return;
        };
        self.player.pause();
        let ext = crate::convert::ext_of(&dst);
        let same_container = ext == crate::convert::ext_of(&self.path);
        let tmp = if overwrite { dst.with_extension(format!("foldex-tmp.{ext}")) } else { dst.clone() };
        let src = self.path.clone();
        let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into(), "-y".into(), "-nostats".into(), "-progress".into(), "pipe:1".into()];
        let out_len = self.st.total() / self.st.speed as f64;
        if self.fast && self.st.copyable() && same_container {
            let (a, b) = self.st.segs[0];
            args.extend(["-ss".into(), format!("{a:.3}"), "-to".into(), format!("{b:.3}"), "-i".into(), src.to_string_lossy().into_owned()]);
            args.extend(["-map".into(), "0".into(), "-c".into(), "copy".into(), "-avoid_negative_ts".into(), "make_zero".into()]);
        } else {
            let audio = self.info.has_audio && !self.st.mute;
            args.extend(["-i".into(), src.to_string_lossy().into_owned(), "-filter_complex".into(), filter_graph(&self.st, audio), "-map".into(), "[vout]".into()]);
            if audio {
                args.extend(["-map".into(), "[aout]".into()]);
            }
            match ext.as_str() {
                "webm" => {
                    args.extend(["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-row-mt", "1", "-b:v", "0", "-crf", "32"].map(String::from));
                    if audio {
                        args.extend(["-c:a", "libopus", "-b:a", "128k"].map(String::from));
                    }
                }
                _ => {
                    args.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p"].map(String::from));
                    if audio {
                        args.extend(["-c:a", "aac", "-b:a", "160k"].map(String::from));
                    }
                    if ext == "mp4" || ext == "mov" {
                        args.extend(["-movflags", "+faststart"].map(String::from));
                    }
                }
            }
        }
        args.push(tmp.to_string_lossy().into_owned());
        let prog = Arc::new(AtomicU32::new(0));
        let (tx, rx) = mpsc::channel();
        let (p2, final_dst) = (prog.clone(), dst.clone());
        std::thread::spawn(move || {
            let r = (|| -> Result<PathBuf, String> {
                let mut child = crate::convert::quiet(&ff).args(&args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("ffmpeg startet nicht: {e}"))?;
                // Fortschritt: „out_time_us=…“ im Verhältnis zur Länge
                if let Some(out) = child.stdout.take() {
                    for line in BufReader::new(out).lines().map_while(Result::ok) {
                        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                            let f = (us / 1e6 / out_len.max(0.001)).clamp(0.0, 1.0);
                            p2.store((f * 1000.0) as u32, Ordering::Relaxed);
                        }
                    }
                }
                let o = child.wait_with_output().map_err(|e| e.to_string())?;
                if !o.status.success() {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("Speichern fehlgeschlagen: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("ffmpeg-Fehler")));
                }
                if overwrite {
                    std::fs::rename(&tmp, &final_dst).map_err(|e| {
                        let _ = std::fs::remove_file(&tmp);
                        format!("Datei lässt sich nicht ersetzen: {e}")
                    })?;
                }
                Ok(final_dst)
            })();
            let _ = tx.send(r);
        });
        self.job = Some(Job { rx, prog, overwrite });
        self.msg = None;
    }
}

/// ffmpeg-Filter: Abschnitte schneiden und verbinden, dann Tempo, Drehen, Größe, Lautstärke
fn filter_graph(st: &State, audio: bool) -> String {
    let mut f = String::new();
    for (i, (a, b)) in st.segs.iter().enumerate() {
        f.push_str(&format!("[0:v]trim=start={a:.4}:end={b:.4},setpts=PTS-STARTPTS[v{i}];"));
        if audio {
            f.push_str(&format!("[0:a]atrim=start={a:.4}:end={b:.4},asetpts=PTS-STARTPTS[a{i}];"));
        }
    }
    for i in 0..st.segs.len() {
        f.push_str(&format!("[v{i}]"));
        if audio {
            f.push_str(&format!("[a{i}]"));
        }
    }
    f.push_str(&format!("concat=n={}:v=1:a={}", st.segs.len().max(1), u8::from(audio)));
    f.push_str(if audio { "[cv][ca];" } else { "[cv];" });
    let mut v: Vec<String> = Vec::new();
    if (st.speed - 1.0).abs() > 0.001 {
        v.push(format!("setpts=PTS/{:.4}", st.speed));
    }
    match st.rotate {
        90 => v.push("transpose=1".into()),
        180 => v.push("transpose=1,transpose=1".into()),
        270 => v.push("transpose=2".into()),
        _ => {}
    }
    if st.height > 0 {
        v.push(format!("scale=-2:{}", st.height));
    }
    if v.is_empty() {
        v.push("null".into());
    }
    f.push_str(&format!("[cv]{}[vout]", v.join(",")));
    if audio {
        let mut a: Vec<String> = Vec::new();
        if (st.speed - 1.0).abs() > 0.001 {
            a.push(media::atempo(st.speed));
        }
        if st.gain_db != 0.0 {
            a.push(format!("volume={:.2}dB", st.gain_db));
        }
        if a.is_empty() {
            a.push("anull".into());
        }
        f.push_str(&format!(";[ca]{}[aout]", a.join(",")));
    }
    f
}

/// Vorschaubilder gleichmäßig über das Video (im Hintergrund, eines nach dem anderen)
fn thumbnails(path: &Path, duration: f64, ctx: &egui::Context) -> Receiver<(usize, egui::ColorImage)> {
    let (tx, rx) = mpsc::channel();
    let (p, c) = (path.to_path_buf(), ctx.clone());
    std::thread::spawn(move || {
        let Some(ff) = crate::convert::ffmpeg() else { return };
        for i in 0..THUMBS {
            let t = duration * (i as f64 + 0.5) / THUMBS as f64;
            let out = crate::convert::quiet(&ff)
                .args(["-v", "error", "-ss", &format!("{t:.3}"), "-i"])
                .arg(&p)
                .args(["-frames:v", "1", "-vf", "scale=160:-2", "-f", "image2pipe", "-vcodec", "png", "-"])
                .stdin(Stdio::null())
                .output();
            let Some(img) = out.ok().filter(|o| o.status.success()).and_then(|o| image::load_from_memory(&o.stdout).ok()) else { continue };
            let rgba = img.to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
            if tx.send((i, ci)).is_err() {
                return;
            }
            c.request_repaint();
        }
    });
    rx
}

fn vsep(ui: &mut egui::Ui, th: &Theme) {
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 24.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, th.line);
    ui.add_space(6.0);
}

fn hms(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn hms_ms(s: f64) -> String {
    let s = s.max(0.0);
    let m = (s / 60.0) as u64;
    format!("{m}:{:05.2}", s - m as f64 * 60.0).replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(len: f64) -> State {
        State { segs: vec![(0.0, len)], mute: false, gain_db: 0.0, speed: 1.0, rotate: 0, height: 0 }
    }

    #[test]
    fn cut_and_copyable() {
        let mut s = st(12.0);
        assert!(s.copyable());
        s.keep(2.0, 9.0);
        assert_eq!(s.segs, vec![(2.0, 9.0)]);
        assert!(s.copyable());
        s.delete(3.0, 4.0);
        assert_eq!(s.segs, vec![(2.0, 5.0), (6.0, 9.0)]);
        assert!(!s.copyable());
        assert!((s.to_src(3.5) - 6.5).abs() < 1e-9);
    }

    #[test]
    fn graph() {
        let mut s = st(10.0);
        s.delete(2.0, 4.0);
        s.rotate = 90;
        s.speed = 2.0;
        let g = filter_graph(&s, true);
        assert!(g.contains("[v0][a0][v1][a1]concat=n=2:v=1:a=1[cv][ca];"), "{g}");
        assert!(g.contains("[cv]setpts=PTS/2.0000,transpose=1[vout]"), "{g}");
        assert!(g.contains("[ca]atempo=2.0000[aout]"), "{g}");
        let g = filter_graph(&st(5.0), false);
        assert!(g.ends_with("concat=n=1:v=1:a=0[cv];[cv]null[vout]"), "{g}");
    }
}
