//! Audioeditor im großen Preview (Vorschau als eigener Tab): Bereich in der Wellenform markieren, abspielen,
//! zuschneiden (Auswahl behalten) oder herausschneiden (Auswahl löschen); Ein-/Ausblenden, Lautstärke (dB),
//! Normalisieren, Geschwindigkeit (Tonhöhe bleibt), Mono, Rückwärts; Zoom in der Wellenform; Rückgängig/
//! Wiederholen; Speichern als MP3, M4A, OGG, Opus, FLAC oder WAV.
//!
//! Nicht zerstörend: Bearbeitet wird eine Liste der behaltenen Abschnitte der Originaldatei; die Wiedergabe
//! springt über herausgeschnittene Stellen. Erst beim Speichern rechnet ffmpeg alles in einem Durchgang.

use crate::edicons::{self, E};
use crate::media::{self, Player, Waveform, WAVE_BLOCK};
use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, FontId, Key, Rect, Stroke};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

const FORMATS: &[(&str, &str)] = &[("mp3", "MP3"), ("m4a", "M4A (AAC)"), ("ogg", "OGG (Vorbis)"), ("opus", "Opus"), ("flac", "FLAC (verlustfrei)"), ("wav", "WAV (unkomprimiert)")];

/// Bearbeitungsstand (für Rückgängig)
#[derive(Clone, PartialEq, Debug)]
struct State {
    /// behaltene Abschnitte der Originaldatei (Sekunden), in Abspielreihenfolge
    segs: Vec<(f64, f64)>,
    fade_in: f32,
    fade_out: f32,
    gain_db: f32,
    normalize: bool,
    speed: f32,
    mono: bool,
    reverse: bool,
}

impl State {
    fn total(&self) -> f64 {
        self.segs.iter().map(|s| s.1 - s.0).sum()
    }
    /// Zeit im bearbeiteten Stück → Zeit in der Datei
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
    /// Zeit in der Datei → bearbeitetes Stück (None = herausgeschnitten)
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
    /// Bereich a..b (bearbeitete Zeit) entfernen
    fn delete(&mut self, a: f64, b: f64) {
        let mut out = Vec::new();
        let mut acc = 0.0;
        for &(s0, s1) in &self.segs {
            let (e0, e1) = (acc, acc + (s1 - s0));
            // Teil vor a
            if a > e0 {
                out.push((s0, s0 + (a.min(e1) - e0)));
            }
            // Teil nach b
            if b < e1 {
                out.push((s0 + (b.max(e0) - e0), s1));
            }
            acc = e1;
        }
        self.segs = out.into_iter().filter(|s| s.1 - s.0 > 1e-4).collect();
    }
    /// nur a..b behalten
    fn keep(&mut self, a: f64, b: f64) {
        let total = self.total();
        self.delete(b, total);
        self.delete(0.0, a);
    }
}

pub enum AudioOut {
    Close,
    /// Datei überschrieben – Vorschau neu laden
    Saved(PathBuf),
}

pub struct AudioEditor {
    path: PathBuf,
    info: media::Info,
    player: Player,
    wave: Waveform,
    blocks: Vec<[f32; 2]>,
    max_peak: f32,
    wave_done: bool,
    wave_at: Instant,
    st: State,
    undo: Vec<State>,
    redo: Vec<State>,
    /// Auswahl in bearbeiteter Zeit
    sel: Option<(f64, f64)>,
    drag_from: Option<f64>,
    /// sichtbarer Ausschnitt: Beginn und Länge (bearbeitete Zeit)
    view: (f64, f64),
    /// Abspielen endet hier (Ende der Auswahl)
    play_to: Option<f64>,
    /// Abspielstelle, solange angehalten (bearbeitete Zeit)
    cursor: f64,
    format: String,
    bitrate: u32,
    job: Option<(Receiver<Result<PathBuf, String>>, bool)>,
    msg: Option<(bool, String)>,
    pub modified: bool,
    /// laufender Zug an einem Effekt-Regler (ein Rückgängig-Schritt)
    fx_session: bool,
}

impl AudioEditor {
    pub fn open(path: &Path, info: &media::Info, ctx: &egui::Context, volume: f32) -> Result<AudioEditor, String> {
        if crate::convert::ffmpeg().is_none() {
            return Err("Der Audioeditor braucht ffmpeg (zum Speichern).".into());
        }
        if info.duration <= 0.0 {
            return Err("Die Länge der Datei ist unbekannt.".into());
        }
        let ext = crate::convert::ext_of(path);
        let format = if FORMATS.iter().any(|f| f.0 == ext) { ext } else { "mp3".into() };
        Ok(AudioEditor {
            path: path.to_path_buf(),
            info: info.clone(),
            player: Player::new(path.to_path_buf(), info.clone(), ctx, volume),
            wave: Waveform::start(path, ctx),
            blocks: Vec::new(),
            max_peak: 0.0,
            wave_done: false,
            wave_at: Instant::now() - Duration::from_secs(1),
            st: State { segs: vec![(0.0, info.duration)], fade_in: 0.0, fade_out: 0.0, gain_db: 0.0, normalize: false, speed: 1.0, mono: false, reverse: false },
            undo: Vec::new(),
            redo: Vec::new(),
            sel: None,
            drag_from: None,
            view: (0.0, info.duration),
            play_to: None,
            cursor: 0.0,
            format,
            bitrate: 192,
            job: None,
            msg: None,
            modified: false,
            fx_session: false,
        })
    }

    /// Nur für Bildschirmfotos: Ausschnitt löschen, Einblenden, Auswahl setzen
    pub fn dev_demo(&mut self) {
        let t = self.st.total();
        self.edit(|s| s.delete(t * 0.6, t * 0.7));
        self.st.fade_in = (t * 0.1) as f32;
        self.st.gain_db = 3.0;
        self.sel = Some((t * 0.2, t * 0.35));
        self.cursor = t * 0.25;
    }

    /// Nicht gezeigt (anderer Tab): Wiedergabe anhalten
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
            // Auswahl und Ansicht an die neue Länge anpassen
            let total = self.st.total();
            self.sel = None;
            self.cursor = self.cursor.min(total);
            self.view = (0.0, total.max(0.05));
        }
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.st, s));
            self.after_history();
        }
    }
    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.st, s));
            self.after_history();
        }
    }
    fn after_history(&mut self) {
        self.modified = true;
        self.sel = None;
        let total = self.st.total();
        self.view = (0.0, total.max(0.05));
        self.cursor = self.cursor.min(total);
    }

    /// Abspielstelle in bearbeiteter Zeit
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

    /// Wiedergabe über herausgeschnittene Stellen springen lassen, am Ende/Auswahlende anhalten
    fn follow(&mut self, ctx: &egui::Context) {
        if self.player.is_paused() {
            return;
        }
        let src = self.player.pos();
        match self.st.to_edit(src) {
            Some(t) => {
                self.cursor = t;
                if self.play_to.is_some_and(|e| t >= e) || t >= self.st.total() - 0.01 {
                    self.player.pause();
                    self.cursor = self.play_to.take().unwrap_or(self.st.total()).min(self.st.total());
                }
            }
            None => {
                // in einer Lücke: zum nächsten Abschnitt
                match self.st.segs.iter().find(|s| s.0 > src) {
                    Some(s) => self.player.seek(s.0),
                    None => {
                        self.player.pause();
                        self.cursor = self.st.total();
                    }
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme, rect: Rect) -> Option<AudioOut> {
        let ctx = ui.ctx().clone();
        self.player.last_frame = ctx.cumulative_pass_nr();
        self.follow(&ctx);
        if self.wave_at.elapsed() > Duration::from_millis(250) && !self.wave_done {
            let (b, m, d) = self.wave.snapshot();
            (self.blocks, self.max_peak, self.wave_done) = (b, m, d);
            self.wave_at = Instant::now();
            ctx.request_repaint_after(Duration::from_millis(260));
        }
        let mut out = None;
        if let Some((rx, overwrite)) = &self.job {
            if let Ok(r) = rx.try_recv() {
                let overwrite = *overwrite;
                self.job = None;
                match r {
                    Ok(p) if overwrite => {
                        self.modified = false;
                        out = Some(AudioOut::Saved(p));
                    }
                    Ok(p) => {
                        self.msg = Some((false, format!("Gespeichert: {}", p.display())));
                        self.modified = false;
                    }
                    Err(e) => self.msg = Some((true, e)),
                }
            }
        }
        ui.painter().rect_filled(rect, 0.0, th.panel);
        let inner = rect.shrink(14.0);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let ui = &mut child;
        // Kopfzeile: Datei und Länge · Speichern, Speichern unter, Schließen
        ui.horizontal(|ui| {
            let name = self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            ui.label(egui::RichText::new(name).size(15.0).strong());
            if self.modified {
                ui.label(egui::RichText::new("●").color(th.accent)).on_hover_text("Ungespeicherte Änderungen");
            }
            ui.label(egui::RichText::new(format!("{}  →  {}", hms(self.info.duration), hms(self.st.total() / self.st.speed as f64))).color(th.sub))
                .on_hover_text("Länge vorher → nachher");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if edicons::btn(ui, th, E::Close, if self.modified { "Schließen – ungespeicherte Änderungen gehen verloren" } else { "Schließen" }, true).clicked() {
                    self.player.pause();
                    out = Some(AudioOut::Close);
                }
                let busy = self.job.is_some();
                if edicons::labeled(ui, th, E::SaveAs, "Speichern unter", "Als neue Datei (Format wählbar)", !busy, false).clicked() {
                    let stem = self.path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    if let Some(p) = rfd::FileDialog::new()
                        .set_directory(self.path.parent().unwrap_or(Path::new(".")))
                        .set_file_name(format!("{stem} (bearbeitet).{}", self.format))
                        .add_filter("Audio", &FORMATS.iter().map(|f| f.0).collect::<Vec<_>>())
                        .save_file()
                    {
                        self.export(p, false);
                    }
                }
                let same = crate::convert::ext_of(&self.path) == self.format;
                if edicons::labeled(ui, th, E::Save, "Speichern", if same { "Datei überschreiben (Strg+S)" } else { "Anderes Format: „Speichern unter“" }, self.modified && same && !busy, true).clicked() {
                    self.export(self.path.clone(), true);
                }
                if busy {
                    ui.spinner();
                    ui.label(egui::RichText::new("Wird gespeichert …").color(th.sub));
                }
            });
        });
        ui.add_space(8.0);
        // Wellenform mit Zeitleiste
        let wave_h = (ui.available_height() * 0.45).clamp(140.0, 360.0);
        let (wr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), wave_h), egui::Sense::click_and_drag());
        self.wave_ui(ui, th, wr, &resp);
        ui.add_space(8.0);
        // Transport und Bearbeiten
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let playing = !self.player.is_paused();
            // großer runder Abspielknopf
            let (r, pr) = ui.allocate_exact_size(vec2(40.0, 40.0), egui::Sense::click());
            ui.painter().circle_filled(r.center(), 20.0, if pr.hovered() { th.accent.gamma_multiply(0.85) } else { th.accent });
            let ic = Rect::from_center_size(r.center() + vec2(if playing { 0.0 } else { 1.5 }, 0.0), vec2(16.0, 16.0));
            edicons::draw(ui.painter(), ic, if playing { E::Pause } else { E::Play }, Color32::WHITE);
            if pr.on_hover_text(if playing { "Pause (Leertaste)" } else { "Abspielen (Leertaste) – mit Auswahl nur die Auswahl" }).clicked() {
                self.toggle();
            }
            if edicons::btn(ui, th, E::ToStart, "Zum Anfang (Pos1)", true).clicked() {
                self.cursor = 0.0;
                if playing {
                    self.play_from(0.0, None);
                }
            }
            ui.add_space(4.0);
            egui::Frame::none().fill(th.field).rounding(6.0).inner_margin(egui::Margin::symmetric(10.0, 6.0)).show(ui, |ui| {
                ui.label(egui::RichText::new(format!("{}  /  {}", hms_ms(self.pos()), hms_ms(self.st.total()))).monospace().size(13.0));
            });
            vsep(ui, th);
            let has = self.sel.is_some_and(|s| s.1 - s.0 > 0.01);
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
            if edicons::btn(ui, th, E::ZoomSel, "Auf Auswahl zoomen", has).clicked() {
                if let Some((a, b)) = self.sel {
                    self.view = (a, (b - a).max(0.05));
                }
            }
            if edicons::btn(ui, th, E::ZoomAll, "Ganzes Stück zeigen", true).clicked() {
                self.view = (0.0, self.st.total().max(0.05));
            }
        });
        ui.add_space(10.0);
        // Effekte
        let mut st = self.st.clone();
        let total = st.total();
        edicons::heading(ui, th, E::Sliders, "Effekte");
        ui.add_space(4.0);
        egui::Grid::new("aud_fx").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label("Einblenden");
            ui.add(egui::Slider::new(&mut st.fade_in, 0.0..=(total as f32 / 2.0).max(0.1)).suffix(" s"));
            ui.end_row();
            ui.label("Ausblenden");
            ui.add(egui::Slider::new(&mut st.fade_out, 0.0..=(total as f32 / 2.0).max(0.1)).suffix(" s"));
            ui.end_row();
            ui.label("Lautstärke");
            ui.add(egui::Slider::new(&mut st.gain_db, -30.0..=20.0).suffix(" dB"));
            ui.end_row();
            ui.label("Geschwindigkeit");
            ui.add(egui::Slider::new(&mut st.speed, 0.5..=2.0).suffix("×").fixed_decimals(2)).on_hover_text("Tonhöhe bleibt gleich (beim Speichern)");
            ui.end_row();
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(&mut st.normalize, "Normalisieren").on_hover_text("Gleichmäßige Lautheit (EBU R128, −16 LUFS)");
                ui.checkbox(&mut st.mono, "Mono");
                ui.checkbox(&mut st.reverse, "Rückwärts");
            });
            ui.end_row();
            ui.label("Format");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("aud_fmt").selected_text(FORMATS.iter().find(|f| f.0 == self.format).map_or("", |f| f.1)).show_ui(ui, |ui| {
                    for (e, n) in FORMATS {
                        ui.selectable_value(&mut self.format, e.to_string(), *n);
                    }
                });
                if matches!(self.format.as_str(), "mp3" | "m4a" | "ogg" | "opus") {
                    egui::ComboBox::from_id_salt("aud_br").selected_text(format!("{} kbit/s", self.bitrate)).show_ui(ui, |ui| {
                        for b in [96, 128, 160, 192, 256, 320] {
                            ui.selectable_value(&mut self.bitrate, b, format!("{b} kbit/s"));
                        }
                    });
                }
            });
            ui.end_row();
        });
        // Regler/Schalter: sofort wirksam, ein Rückgängig-Schritt je Zug (bis zum Loslassen)
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
            ui.add_space(8.0);
            ui.label(egui::RichText::new(m).color(if *err { th.danger } else { th.accent }));
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new("In der Wellenform ziehen = auswählen · Klick = Abspielstelle · Mausrad = zoomen, Umschalt+Rad = verschieben · Leertaste spielt ab").size(11.5).color(th.sub));
        self.keys(ui, &mut out);
        out
    }

    fn keys(&mut self, ui: &egui::Ui, out: &mut Option<AudioOut>) {
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
            if let Some((a, b)) = self.sel.filter(|s| s.1 - s.0 > 0.01) {
                self.edit(|s| s.delete(a, b));
            }
        } else if pressed(Key::Home) {
            self.cursor = 0.0;
        } else if pressed(Key::Escape) {
            self.sel = None;
        }
        let _ = out;
    }

    /// größte Pegel (Spitze, Effektiv) der Datei im bearbeiteten Zeitraum e0..e1
    fn range_max(&self, e0: f64, e1: f64) -> Option<(f32, f32)> {
        let mut acc = 0.0;
        let mut best: Option<(f32, f32)> = None;
        for &(s0, s1) in &self.st.segs {
            let len = s1 - s0;
            let (a, b) = (e0.max(acc), e1.min(acc + len));
            if a < b {
                let (src_a, src_b) = (s0 + a - acc, s0 + b - acc);
                let (i0, i1) = ((src_a / WAVE_BLOCK) as usize, ((src_b / WAVE_BLOCK).ceil() as usize).max((src_a / WAVE_BLOCK) as usize + 1));
                for v in self.blocks.get(i0..i1.min(self.blocks.len())).unwrap_or(&[]) {
                    best = Some(best.map_or((v[0], v[1]), |m| (m.0.max(v[0]), m.1.max(v[1]))));
                }
            }
            acc += len;
        }
        best
    }

    fn wave_ui(&mut self, ui: &egui::Ui, th: &Theme, r: Rect, resp: &egui::Response) {
        let p = ui.painter_at(r);
        p.rect_filled(r, 8.0, th.field);
        let ruler = Rect::from_min_size(r.min, vec2(r.width(), 18.0));
        let w = Rect::from_min_max(pos2(r.left() + 8.0, ruler.bottom() + 4.0), pos2(r.right() - 8.0, r.bottom() - 6.0));
        let total = self.st.total().max(0.001);
        let (v0, vl) = (self.view.0.clamp(0.0, total), self.view.1.clamp(0.01, total));
        let to_x = |t: f64| w.left() + ((t - v0) / vl) as f32 * w.width();
        let to_t = |x: f32| (v0 + ((x - w.left()) / w.width()) as f64 * vl).clamp(0.0, total);
        // Zeitleiste
        let step = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0].into_iter().find(|s| vl / s <= 12.0).unwrap_or(1200.0);
        let mut t = (v0 / step).ceil() * step;
        while t <= v0 + vl {
            let x = to_x(t);
            p.line_segment([pos2(x, ruler.bottom() - 4.0), pos2(x, ruler.bottom())], Stroke::new(1.0, th.sub));
            p.text(pos2(x + 2.0, ruler.top() + 2.0), egui::Align2::LEFT_TOP, if step < 1.0 { hms_ms(t) } else { hms(t) }, FontId::proportional(10.5), th.sub);
            t += step;
        }
        // Wellenform (Lautstärke-Änderung sichtbar; über 0 dBFS rot)
        let gain = 10f32.powf(self.st.gain_db / 20.0);
        let scale = if self.max_peak > 0.0 { 1.0 / self.max_peak.max(0.05) } else { 0.0 };
        let (mid, half) = (w.center().y, w.height() / 2.0);
        let n = ((w.width() / 2.0) as usize).max(1);
        let col = w.width() / n as f32;
        for i in 0..n {
            let (e0, e1) = (v0 + vl * i as f64 / n as f64, v0 + vl * (i + 1) as f64 / n as f64);
            let x = w.left() + i as f32 * col;
            let Some((pk, rms)) = self.range_max(e0, e1) else {
                p.line_segment([pos2(x, mid), pos2(x + col, mid)], Stroke::new(1.0, th.line));
                continue;
            };
            // Ein-/Ausblenden einrechnen
            let tm = (e0 + e1) / 2.0;
            let mut f = gain;
            if self.st.fade_in > 0.0 {
                f *= ((tm / self.st.fade_in as f64) as f32).clamp(0.0, 1.0);
            }
            if self.st.fade_out > 0.0 {
                f *= (((total - tm) / self.st.fade_out as f64) as f32).clamp(0.0, 1.0);
            }
            let clip = pk * gain > 1.0;
            let (hp, hr) = ((pk * scale * f).min(1.0) * half, (rms * scale * f).min(1.0) * half);
            let bw = (col * 0.8).max(1.0);
            p.rect_filled(Rect::from_min_max(pos2(x, mid - hp.max(0.5)), pos2(x + bw, mid + hp.max(0.5))), 0.0, if clip { th.danger.gamma_multiply(0.5) } else { th.sub.gamma_multiply(0.35) });
            p.rect_filled(Rect::from_min_max(pos2(x, mid - hr.max(0.5)), pos2(x + bw, mid + hr.max(0.5))), 0.0, if clip { th.danger } else { th.accent.gamma_multiply(0.85) });
        }
        // Schnittstellen
        let mut acc = 0.0;
        for s in &self.st.segs[..self.st.segs.len().saturating_sub(1)] {
            acc += s.1 - s.0;
            let x = to_x(acc);
            if w.x_range().contains(x) {
                let mut y = w.top();
                while y < w.bottom() {
                    p.line_segment([pos2(x, y), pos2(x, (y + 4.0).min(w.bottom()))], Stroke::new(1.0, th.danger));
                    y += 8.0;
                }
            }
        }
        // Auswahl
        if let Some((a, b)) = self.sel {
            let sr = Rect::from_min_max(pos2(to_x(a).max(w.left()), w.top()), pos2(to_x(b).min(w.right()), w.bottom()));
            if sr.width() > 0.0 {
                p.rect_filled(sr, 0.0, th.accent.gamma_multiply(0.18));
                p.rect_stroke(sr, 0.0, Stroke::new(1.0, th.accent));
                p.text(sr.left_bottom() + vec2(3.0, -3.0), egui::Align2::LEFT_BOTTOM, hms_ms(b - a), FontId::proportional(11.0), th.text);
            }
        }
        // Abspielstelle
        let px = to_x(self.pos());
        if w.x_range().contains(px) {
            p.line_segment([pos2(px, ruler.bottom()), pos2(px, r.bottom())], Stroke::new(2.0, th.text));
        }
        // Eingaben: ziehen = auswählen, Klick = Abspielstelle, Rad = Zoom/Verschieben
        if let Some(h) = resp.hover_pos() {
            let (scroll, shift) = ui.input(|i| (i.raw_scroll_delta, i.modifiers.shift));
            let dy = if scroll.y != 0.0 { scroll.y } else { scroll.x };
            if dy != 0.0 {
                if shift || scroll.x != 0.0 {
                    let d = -(dy as f64 / w.width() as f64) * vl;
                    self.view.0 = (v0 + d).clamp(0.0, (total - vl).max(0.0));
                } else {
                    let at = to_t(h.x);
                    let nl = (vl * if dy > 0.0 { 0.8 } else { 1.25 }).clamp(0.05, total);
                    let frac = (at - v0) / vl;
                    self.view = ((at - frac * nl).clamp(0.0, (total - nl).max(0.0)), nl);
                }
            }
            p.line_segment([pos2(h.x, w.top()), pos2(h.x, w.bottom())], Stroke::new(1.0, th.sub.gamma_multiply(0.5)));
            egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), ui.id().with("aud_tip"), |ui| {
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
            if self.sel.is_some_and(|s| s.1 - s.0 < 0.01) {
                self.sel = None;
            }
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let t = to_t(p.x);
                self.sel = None;
                if self.player.is_paused() {
                    self.cursor = t;
                } else {
                    self.play_from(t, None);
                }
            }
        }
        if !self.wave_done {
            p.text(w.right_top() + vec2(-4.0, 2.0), egui::Align2::RIGHT_TOP, "Wellenform wird berechnet …", FontId::proportional(11.0), th.sub);
        }
    }

    /// Mit ffmpeg speichern: Abschnitte aneinander, dann Effekte. `overwrite`: erst in eine temporäre Datei,
    /// dann ersetzen.
    fn export(&mut self, dst: PathBuf, overwrite: bool) {
        let Some(ff) = crate::convert::ffmpeg() else {
            self.msg = Some((true, "ffmpeg ist nicht installiert".into()));
            return;
        };
        self.player.pause();
        let ext = crate::convert::ext_of(&dst);
        let filter = filter_graph(&self.st);
        let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into(), "-y".into(), "-i".into()];
        let src = self.path.clone();
        let tmp = if overwrite { dst.with_extension(format!("foldex-tmp.{ext}")) } else { dst.clone() };
        let mut tail: Vec<String> = vec!["-filter_complex".into(), filter, "-map".into(), "[out]".into(), "-map_metadata".into(), "0".into()];
        let br = format!("{}k", self.bitrate);
        let codec: &[&str] = match ext.as_str() {
            "mp3" => &["-c:a", "libmp3lame", "-b:a"],
            "m4a" | "aac" => &["-c:a", "aac", "-b:a"],
            "ogg" => &["-c:a", "libvorbis", "-b:a"],
            "opus" => &["-c:a", "libopus", "-b:a"],
            "flac" => &["-c:a", "flac"],
            _ => &["-c:a", "pcm_s16le"],
        };
        tail.extend(codec.iter().map(|s| s.to_string()));
        if codec.last() == Some(&"-b:a") {
            tail.push(br);
        }
        let (tx, rx) = mpsc::channel();
        let final_dst = dst.clone();
        std::thread::spawn(move || {
            let mut c = crate::convert::quiet(&ff);
            c.args(&args.drain(..).collect::<Vec<_>>()).arg(&src).args(&tail).arg(&tmp);
            let r = c.output().map_err(|e| format!("ffmpeg startet nicht: {e}")).and_then(|o| {
                if o.status.success() {
                    Ok(())
                } else {
                    let _ = std::fs::remove_file(&tmp);
                    Err(format!("Speichern fehlgeschlagen: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("ffmpeg-Fehler")))
                }
            });
            let r = r.and_then(|()| {
                if overwrite {
                    std::fs::rename(&tmp, &final_dst).map_err(|e| {
                        let _ = std::fs::remove_file(&tmp);
                        format!("Datei lässt sich nicht ersetzen: {e}")
                    })?;
                }
                Ok(final_dst)
            });
            let _ = tx.send(r);
        });
        self.job = Some((rx, overwrite));
        self.msg = None;
    }
}

/// ffmpeg-Filter: Abschnitte schneiden und verbinden, dann Ein-/Ausblenden, Lautstärke, Normalisieren,
/// Geschwindigkeit, Mono, Rückwärts
fn filter_graph(st: &State) -> String {
    let mut f = String::new();
    for (i, (a, b)) in st.segs.iter().enumerate() {
        f.push_str(&format!("[0:a]atrim=start={a:.4}:end={b:.4},asetpts=PTS-STARTPTS[s{i}];"));
    }
    for i in 0..st.segs.len() {
        f.push_str(&format!("[s{i}]"));
    }
    f.push_str(&format!("concat=n={}:v=0:a=1", st.segs.len().max(1)));
    let total = st.total();
    let mut chain: Vec<String> = Vec::new();
    if st.reverse {
        chain.push("areverse".into());
    }
    if st.fade_in > 0.0 {
        chain.push(format!("afade=t=in:st=0:d={:.3}", st.fade_in));
    }
    if st.fade_out > 0.0 {
        chain.push(format!("afade=t=out:st={:.3}:d={:.3}", (total - st.fade_out as f64).max(0.0), st.fade_out));
    }
    if st.gain_db != 0.0 {
        chain.push(format!("volume={:.2}dB", st.gain_db));
    }
    if st.normalize {
        chain.push("loudnorm=I=-16:TP=-1.5:LRA=11".into());
    }
    if (st.speed - 1.0).abs() > 0.001 {
        chain.push(media::atempo(st.speed));
    }
    if st.mono {
        chain.push("pan=mono|c0=0.5*c0+0.5*c1".into());
    }
    if !chain.is_empty() {
        f.push(',');
        f.push_str(&chain.join(","));
    }
    f.push_str("[out]");
    f
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
        State { segs: vec![(0.0, len)], fade_in: 0.0, fade_out: 0.0, gain_db: 0.0, normalize: false, speed: 1.0, mono: false, reverse: false }
    }

    #[test]
    fn delete_and_keep() {
        let mut s = st(10.0);
        s.delete(2.0, 4.0);
        assert_eq!(s.segs, vec![(0.0, 2.0), (4.0, 10.0)]);
        assert!((s.total() - 8.0).abs() < 1e-9);
        // bearbeitete Zeit 3 s liegt in der Datei bei 5 s
        assert!((s.to_src(3.0) - 5.0).abs() < 1e-9);
        assert_eq!(s.to_edit(3.0), None);
        assert!((s.to_edit(5.0).unwrap() - 3.0).abs() < 1e-9);
        // nochmal über die Schnittstelle hinweg löschen
        s.delete(1.0, 3.0);
        assert_eq!(s.segs, vec![(0.0, 1.0), (5.0, 10.0)]);
        s.keep(0.5, 2.0);
        assert_eq!(s.segs, vec![(0.5, 1.0), (5.0, 6.0)]);
    }

    #[test]
    fn graph() {
        let mut s = st(10.0);
        s.delete(2.0, 4.0);
        s.fade_in = 1.0;
        s.gain_db = 3.0;
        let g = filter_graph(&s);
        assert!(g.contains("atrim=start=0.0000:end=2.0000"), "{g}");
        assert!(g.contains("concat=n=2:v=0:a=1,afade=t=in:st=0:d=1.000,volume=3.00dB[out]"), "{g}");
    }
}

/// senkrechter Trenner in Leisten
fn vsep(ui: &mut egui::Ui, th: &Theme) {
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 24.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, th.line);
    ui.add_space(6.0);
}
