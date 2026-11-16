//! Spracheingabe für die KI-Chats: Mikrofon aufnehmen (cpal), lokal mit whisper.cpp in Text umwandeln. Der
//! Knopf sitzt neben „Senden“; ein Klick startet, ein zweiter beendet die Aufnahme, der Text landet im
//! Eingabefeld. Fehlt Whisper, fragt ein kleines Fenster, welches Modell geladen werden soll.

use crate::localllm as ll;
use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, Rect, Sense, Stroke};
use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// längste Aufnahme
const MAX: Duration = Duration::from_secs(180);

type Samples = Result<(Vec<f32>, u32), String>;

#[derive(Default)]
enum State {
    #[default]
    Idle,
    /// läuft: Stopp-Signal, Ergebnis, Beginn, Lautstärke (für die Anzeige)
    Rec { stop: Arc<AtomicBool>, rx: mpsc::Receiver<Samples>, since: Instant, level: Arc<AtomicU32> },
    /// Aufnahme fertig, wird umgewandelt
    Busy(mpsc::Receiver<Result<String, String>>),
    /// Whisper fehlt: Fenster zum Einrichten
    Setup,
}

#[derive(Default)]
pub struct Mic {
    state: State,
    err: Option<String>,
}

impl Mic {
    pub fn busy(&self) -> bool {
        matches!(self.state, State::Rec { .. } | State::Busy(_))
    }

    /// Mikrofon-Knopf (36 × 36). Erkannter Text wird an `input` angehängt.
    pub fn button(&mut self, ui: &mut egui::Ui, th: &Theme, input: &mut String, enabled: bool) {
        let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
        let ctx = ui.ctx().clone();
        // fertige Aufnahme → umwandeln
        if let State::Rec { stop, rx, since, .. } = &self.state {
            if since.elapsed() > MAX {
                stop.store(true, Ordering::Relaxed);
            }
            if let Ok(r) = rx.try_recv() {
                self.state = match r {
                    Ok((s, rate)) => start_transcribe(s, rate, &ctx),
                    Err(e) => {
                        self.err = Some(e);
                        State::Idle
                    }
                };
            }
        }
        if let State::Busy(rx) = &self.state {
            if let Ok(r) = rx.try_recv() {
                match r {
                    Ok(t) if !t.trim().is_empty() => {
                        if !input.is_empty() && !input.ends_with([' ', '\n']) {
                            input.push(' ');
                        }
                        input.push_str(t.trim());
                    }
                    Ok(_) => self.err = Some("Nichts verstanden".into()),
                    Err(e) => self.err = Some(e),
                }
                self.state = State::Idle;
            }
        }
        let hover = resp.hovered();
        let p = ui.painter();
        match &self.state {
            State::Rec { since, level, .. } => {
                // rot, pulsiert mit der Lautstärke
                let lv = f32::from_bits(level.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                p.circle_filled(r.center(), 18.0, th.danger);
                p.circle_stroke(r.center(), 18.0 + lv * 5.0, Stroke::new(2.0, th.danger.gamma_multiply(0.5)));
                p.rect_filled(Rect::from_center_size(r.center(), vec2(11.0, 11.0)), 2.0, Color32::WHITE);
                let secs = since.elapsed().as_secs();
                resp.clone().on_hover_text(format!("Aufnahme läuft ({}:{:02}) – klicken zum Beenden", secs / 60, secs % 60));
                ctx.request_repaint_after(Duration::from_millis(60));
            }
            State::Busy(_) => {
                p.circle_filled(r.center(), 18.0, th.field);
                let t = ui.input(|i| i.time) as f32;
                let a = t * 6.0;
                p.add(egui::Shape::line((0..=12).map(|k| { let w = a + k as f32 * 0.35; r.center() + vec2(w.cos(), w.sin()) * 9.0 }).collect(), Stroke::new(2.0, th.accent)));
                resp.clone().on_hover_text("Wird in Text umgewandelt …");
                ctx.request_repaint_after(Duration::from_millis(40));
            }
            _ => {
                p.circle_filled(r.center(), 18.0, if hover && enabled { th.hover } else { th.field });
                draw_mic(p, r.center(), if enabled { th.text } else { th.sub });
                resp.clone().on_hover_text(match &self.err {
                    Some(e) => format!("Spracheingabe (lokal)\nZuletzt: {e}\nRechtsklick: Modell wählen"),
                    None => format!("Spracheingabe – lokal mit {}, nichts verlässt den PC\nRechtsklick: Modell wählen", ll::stt_in_use().map_or("Whisper", |m| m.name)),
                });
            }
        }
        // Rechtsklick: Modell wählen, laden oder löschen
        if resp.secondary_clicked() && !self.busy() {
            self.state = State::Setup;
        }
        if resp.clicked() && enabled {
            self.err = None;
            self.state = match std::mem::take(&mut self.state) {
                State::Rec { stop, rx, since, level } => {
                    stop.store(true, Ordering::Relaxed);
                    State::Rec { stop, rx, since, level }
                }
                State::Busy(rx) => State::Busy(rx),
                State::Idle | State::Setup if ll::stt_ready().is_none() => State::Setup,
                State::Idle | State::Setup => start_rec(&ctx),
            };
        }
        if matches!(self.state, State::Setup) {
            self.setup_window(&ctx, th);
        }
    }

    /// kleines Fenster: Whisper-Modell aussuchen und laden
    fn setup_window(&mut self, ctx: &egui::Context, th: &Theme) {
        let mut open = true;
        let mut start = false;
        egui::Window::new("Spracheingabe").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.set_max_width(400.0);
            ui.label("Gesprochenes wird lokal mit Whisper in Text umgewandelt – kein Ton verlässt den PC. Modell wählen oder laden:");
            ui.add_space(6.0);
            model_rows(ui, th);
            if !ll::downloads().is_empty() {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
            if ll::stt_ready().is_some() {
                ui.add_space(6.0);
                if ui.add(egui::Button::new(egui::RichText::new("Aufnahme starten").strong())).clicked() {
                    start = true;
                }
            }
        });
        if start {
            self.state = start_rec(ctx);
        } else if !open {
            self.state = State::Idle;
        }
    }
}

/// Whisper-Modelle: laden, verwenden, löschen (Fenster am Mikrofon und Einstellungen)
pub fn model_rows(ui: &mut egui::Ui, th: &Theme) {
    let (names, auto) = mics();
    let chosen = chosen_mic();
    ui.horizontal(|ui| {
        ui.label("Mikrofon");
        let auto_label = format!("Automatisch ({})", auto.as_deref().unwrap_or("keins gefunden"));
        let shown = chosen.clone().unwrap_or_else(|| auto_label.clone());
        egui::ComboBox::from_id_salt("stt_mic").selected_text(shown).width((ui.available_width() - 8.0).max(160.0)).show_ui(ui, |ui| {
            if ui.selectable_label(chosen.is_none(), &auto_label).on_hover_text("Windows-Standard; virtuelle Geräte wie Voicemod werden übersprungen").clicked() {
                set_mic(None);
            }
            for n in &names {
                let label = if is_virtual(n) { format!("{n}  (virtuell)") } else { n.clone() };
                if ui.selectable_label(chosen.as_deref() == Some(n.as_str()), label).clicked() {
                    set_mic(Some(n));
                }
            }
        });
    });
    if let Some(c) = &chosen {
        if !names.contains(c) {
            ui.label(egui::RichText::new(format!("„{c}“ ist gerade nicht angeschlossen – es wird automatisch gewählt")).size(11.5).color(th.danger));
        }
    }
    ui.add_space(4.0);
    let in_use = ll::stt_in_use();
    for m in ll::STT {
        let dest = ll::stt_path(m);
        let active = in_use.is_some_and(|u| std::ptr::eq(u, m));
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if dest.is_file() {
                    if ui.small_button("Löschen").clicked() {
                        let _ = ll::delete_stt(m);
                    }
                    if active {
                        ui.label(egui::RichText::new("✓ wird benutzt").color(th.accent));
                    } else if ui.button("Verwenden").clicked() {
                        ll::set_stt_active(m);
                    }
                } else if let Some((f, _)) = ll::queued(&dest) {
                    if f >= 0.0 {
                        ui.add(egui::ProgressBar::new(f).desired_width(90.0).show_percentage());
                    } else {
                        ui.spinner();
                    }
                } else if ui.button("Laden").clicked() {
                    // nach dem Laden gleich dieses benutzen
                    ll::set_stt_active(m);
                    ll::install_stt(m);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(m.name).strong());
                        let extra = if ll::exe(&ll::WHISPER).is_none() { ll::WHISPER.mb } else { 0 };
                        ui.add(egui::Label::new(egui::RichText::new(format!("{} · {} MB", m.desc, m.mb() + extra)).size(12.0).color(th.sub)).truncate());
                    });
                });
            });
        });
    }
}

fn draw_mic(p: &egui::Painter, c: egui::Pos2, col: Color32) {
    let st = Stroke::new(1.8, col);
    p.rect_stroke(Rect::from_center_size(c + vec2(0.0, -3.0), vec2(7.0, 11.0)), 3.5, st);
    let arc: Vec<egui::Pos2> = (0..=12).map(|k| { let a = std::f32::consts::PI * k as f32 / 12.0; c + vec2(-a.cos() * 6.5, a.sin() * 6.0 - 1.0) }).collect();
    p.add(egui::Shape::line(arc, st));
    p.line_segment([c + vec2(0.0, 5.0), c + vec2(0.0, 8.0)], st);
    p.line_segment([pos2(c.x - 3.5, c.y + 8.0), pos2(c.x + 3.5, c.y + 8.0)], st);
}

/// Aufnahme in eigenem Thread (der Audio-Strom darf den Thread nicht wechseln)
fn start_rec(ctx: &egui::Context) -> State {
    let stop = Arc::new(AtomicBool::new(false));
    let level = Arc::new(AtomicU32::new(0));
    let (tx, rx) = mpsc::channel();
    let (st, lv, ctx) = (stop.clone(), level.clone(), ctx.clone());
    std::thread::spawn(move || {
        let r = record(&st, &lv);
        let _ = tx.send(r);
        ctx.request_repaint();
    });
    State::Rec { stop, rx, since: Instant::now(), level }
}

fn record(stop: &AtomicBool, level: &Arc<AtomicU32>) -> Samples {
    record_from(pick_device(), stop, level)
}

// ───────────────────────── Mikrofon wählen ─────────────────────────

fn mic_file() -> PathBuf {
    ll::root().join("speech").join("mikrofon.txt")
}

/// gewähltes Mikrofon (None = automatisch)
pub fn chosen_mic() -> Option<String> {
    std::fs::read_to_string(mic_file()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn set_mic(name: Option<&str>) {
    let _ = std::fs::create_dir_all(ll::root().join("speech"));
    let _ = std::fs::write(mic_file(), name.unwrap_or(""));
}

/// virtuelle Geräte (Stimmverzerrer, Audio-Kabel, Stereomix) liefern oft nur Stille
fn is_virtual(name: &str) -> bool {
    let n = name.to_lowercase();
    ["voicemod", "virtual", "cable", "vb-audio", "voicemeeter", "stereomix", "stereo mix", "wave link", "obs-"].iter().any(|x| n.contains(x))
}

/// Mikrofone und das automatisch gewählte (kurz zwischengespeichert – das Abfragen kostet etwas Zeit)
pub fn mics() -> (Vec<String>, Option<String>) {
    static C: Mutex<Option<(Instant, Vec<String>, Option<String>)>> = Mutex::new(None);
    let mut g = C.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_none_or(|c| c.0.elapsed() > Duration::from_secs(5)) {
        let host = rodio::cpal::default_host();
        let names: Vec<String> = host.input_devices().map(|d| d.filter_map(|d| d.name().ok()).collect()).unwrap_or_default();
        let def = host.default_input_device().and_then(|d| d.name().ok());
        let auto = auto_pick(&names, def.as_deref());
        *g = Some((Instant::now(), names, auto));
    }
    let c = g.as_ref().expect("gerade gefüllt");
    (c.1.clone(), c.2.clone())
}

/// Automatik: Windows-Standard, außer er ist virtuell und es gibt ein echtes Mikrofon
fn auto_pick(names: &[String], default: Option<&str>) -> Option<String> {
    match default {
        Some(d) if !is_virtual(d) => Some(d.to_string()),
        _ => names.iter().find(|n| !is_virtual(n)).cloned().or_else(|| default.map(str::to_string)),
    }
}

fn pick_device() -> Option<rodio::cpal::Device> {
    let host = rodio::cpal::default_host();
    let want = chosen_mic().or_else(|| mics().1);
    if let Some(w) = want {
        if let Some(d) = host.input_devices().ok()?.find(|d| d.name().is_ok_and(|n| n == w)) {
            return Some(d);
        }
    }
    host.default_input_device()
}

fn record_from(dev: Option<rodio::cpal::Device>, stop: &AtomicBool, level: &Arc<AtomicU32>) -> Samples {
    let dev = dev.ok_or("Kein Mikrofon gefunden")?;
    let dev_name = dev.name().unwrap_or_else(|_| "Mikrofon".into());
    let cfg = dev.default_input_config().map_err(|e| format!("Mikrofon: {e}"))?;
    let (rate, ch) = (cfg.sample_rate().0, cfg.channels() as usize);
    let buf: Arc<Mutex<Vec<f32>>> = Arc::default();
    let err = |e| eprintln!("Mikrofon: {e}");
    // Kanäle mitteln, Pegel für die Anzeige merken
    let push = {
        let (buf, level) = (buf.clone(), level.clone());
        move |frames: Vec<f32>| {
            let peak = frames.iter().fold(0f32, |a, v| a.max(v.abs()));
            level.store((peak * 3.0).min(1.0).to_bits(), Ordering::Relaxed);
            if let Ok(mut b) = buf.lock() {
                b.extend(frames);
            }
        }
    };
    let mono = move |data: &[f32]| data.chunks(ch.max(1)).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect::<Vec<f32>>();
    let sc: rodio::cpal::StreamConfig = cfg.clone().into();
    let stream = match cfg.sample_format() {
        rodio::cpal::SampleFormat::F32 => {
            let push = push.clone();
            dev.build_input_stream(&sc, move |d: &[f32], _: &_| push(mono(d)), err, None)
        }
        rodio::cpal::SampleFormat::I16 => {
            let push = push.clone();
            dev.build_input_stream(&sc, move |d: &[i16], _: &_| push(mono(&d.iter().map(|s| *s as f32 / 32768.0).collect::<Vec<_>>())), err, None)
        }
        rodio::cpal::SampleFormat::U16 => {
            let push = push.clone();
            dev.build_input_stream(&sc, move |d: &[u16], _: &_| push(mono(&d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0).collect::<Vec<_>>())), err, None)
        }
        f => return Err(format!("Mikrofon-Format {f:?} wird nicht unterstützt")),
    }
    .map_err(|e| format!("Mikrofon: {e}"))?;
    stream.play().map_err(|e| format!("Mikrofon: {e}"))?;
    let t0 = Instant::now();
    while !stop.load(Ordering::Relaxed) && t0.elapsed() < MAX {
        std::thread::sleep(Duration::from_millis(30));
    }
    drop(stream);
    let samples = std::mem::take(&mut *buf.lock().map_err(|_| "Aufnahme gesperrt".to_string())?);
    // nur Stille: falsches oder stummgeschaltetes Gerät (z. B. Voicemod ohne laufendes Programm)
    if samples.iter().fold(0f32, |a, v| a.max(v.abs())) < 1e-4 {
        return Err(format!("„{dev_name}“ liefert nur Stille – Rechtsklick auf das Mikrofon und ein anderes wählen"));
    }
    Ok((samples, rate))
}

fn start_transcribe(samples: Vec<f32>, rate: u32, ctx: &egui::Context) -> State {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let r = (|| {
            if samples.len() < rate as usize / 3 {
                return Err("Zu kurz".to_string());
            }
            let wav = std::env::temp_dir().join(format!("foldex_sprache_{}.wav", std::process::id()));
            write_wav(&wav, &resample(&samples, rate, 16_000))?;
            let t = ll::transcribe(&wav);
            let _ = std::fs::remove_file(&wav);
            t
        })();
        let _ = tx.send(r);
        ctx.request_repaint();
    });
    State::Busy(rx)
}

/// lineare Umrechnung auf die Abtastrate von Whisper
fn resample(s: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || s.is_empty() {
        return s.to_vec();
    }
    let n = (s.len() as u64 * to as u64 / from as u64) as usize;
    let k = from as f64 / to as f64;
    (0..n)
        .map(|i| {
            let x = i as f64 * k;
            let (a, f) = (x.floor() as usize, (x - x.floor()) as f32);
            let (va, vb) = (s[a.min(s.len() - 1)], s[(a + 1).min(s.len() - 1)]);
            va + (vb - va) * f
        })
        .collect()
}

/// 16-Bit-PCM-WAV, mono
fn write_wav(path: &PathBuf, s: &[f32]) -> Result<(), String> {
    let rate = 16_000u32;
    let data_len = (s.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for v in s {
        b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_and_wav() {
        let s: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let r = resample(&s, 48_000, 16_000);
        assert_eq!(r.len(), 16_000);
        let p = std::env::temp_dir().join("foldex_stt_test.wav");
        write_wav(&p, &r).unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(b.len(), 44 + 32_000);
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod probe {
    use super::*;

    /// Mikrofone auflisten und 3 s vom Standardgerät aufnehmen: cargo test mic_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn mic_probe() {
        let host = rodio::cpal::default_host();
        for d in host.input_devices().unwrap() {
            eprintln!("Gerät: {:?} {:?}", d.name(), d.default_input_config().map(|c| (c.sample_rate().0, c.channels(), c.sample_format())));
        }
        eprintln!("Standard: {:?}", host.default_input_device().and_then(|d| d.name().ok()));
        let stop = Arc::new(AtomicBool::new(false));
        let lv = Arc::new(AtomicU32::new(0));
        let s = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            s.store(true, Ordering::Relaxed);
        });
        let r = record_from(host.input_devices().unwrap().find(|d| d.name().is_ok_and(|n| n.contains("CORSAIR"))), &stop, &lv);
        match r {
            Ok((v, rate)) => eprintln!("Aufnahme: {} Werte bei {rate} Hz, Spitze {:.4}", v.len(), v.iter().fold(0f32, |a, x| a.max(x.abs()))),
            Err(e) => eprintln!("Fehler: {e}"),
        }
    }
}

#[cfg(test)]
mod pipeline {
    use super::*;

    /// Erkennung über denselben Weg wie die Aufnahme: FOLDEX_STT_WAV=<16-kHz-WAV> cargo test stt_pipeline -- --ignored --nocapture
    #[test]
    #[ignore]
    fn stt_pipeline() {
        let (names, auto) = mics();
        eprintln!("Mikrofone: {names:?} → automatisch: {auto:?}");
        let Ok(f) = std::env::var("FOLDEX_STT_WAV") else { return };
        let b = std::fs::read(f).unwrap();
        // Daten nach dem 44-Byte-Kopf (SAPI schreibt PCM 16 Bit mono)
        let pcm: Vec<f32> = b[44..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
        let mic48 = resample(&pcm, 16_000, 48_000);
        let wav = std::env::temp_dir().join("foldex_stt_pipeline.wav");
        write_wav(&wav, &resample(&mic48, 48_000, 16_000)).unwrap();
        let t = Instant::now();
        eprintln!("Erkannt in {:?}: {:?}", t.elapsed(), ll::transcribe(&wav));
    }
}
