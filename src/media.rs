//! Medien-Player der Vorschau: Video und Audio direkt im Explorer abspielen.
//! - Video: ffmpeg dekodiert ab der gewünschten Stelle Bilder (RGBA, höchstens 1280 × 720, ≤ 30 fps) in eine
//!   Pipe; angezeigt wird immer das Bild, das zur aktuellen Abspielzeit passt (späte Bilder werden übersprungen).
//! - Audio: rodio spielt ab – MP3, WAV, FLAC, AAC/M4A direkt (symphonia), alles andere und die Tonspur von Videos
//!   über ffmpeg (PCM-Pipe). Die Tonausgabe ist die Uhr, nach der sich das Bild richtet; ohne Ton eine Stoppuhr.
//! - Bedienung: Abspielen/Pause, Zeitleiste (Klicken/Ziehen), Lautstärke/Stumm, Einstellungen (Qualität,
//!   Geschwindigkeit), Vollbild; Tasten: Leertaste, ←/→ (±5 s), ↑/↓ (Lautstärke), M (stumm), F (Vollbild),
//!   Esc (Vollbild verlassen).
//! - Geschwindigkeit: über ffmpeg mit „atempo“ (Tonhöhe bleibt), beim eingebauten Dekoder ohne ffmpeg über rodio.
//! - Wellenform (Audio): Spitzen- und Effektivpegel je 20 ms aus dem echten Signal, im Hintergrund berechnet.
//! Springen startet die Dekoder an der neuen Stelle neu; laufende ffmpeg-Prozesse werden dabei beendet.

use crate::convert;
use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke};
use rodio::Source;
use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const RATE: u32 = 48_000;
const MAX_W: u32 = 1280;
const MAX_H: u32 = 720;
/// Rastpunkte des Geschwindigkeitsreglers (0–2×), Grenzen und wählbare Qualitätsstufen (kürzere Bildseite)
const SPEEDS: [f32; 7] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
/// kleinster Wert am Regler (0× wäre Stillstand – dafür gibt es Pause)
const MIN_SPEED: f32 = 0.1;
const MIN_CUSTOM: f32 = 0.05;
const MAX_SPEED: f32 = 16.0;
/// so lange gedrückt halten, bis 2× greift
const HOLD: Duration = Duration::from_millis(400);
const QUALITIES: [u32; 6] = [1440, 1080, 720, 480, 360, 240];
/// Länge eines Wellenform-Abschnitts in Sekunden
pub const WAVE_BLOCK: f64 = 0.02;

/// Kenndaten einer Mediendatei (über ffprobe; ohne ffprobe für Audio über den eingebauten Dekoder).
#[derive(Clone, Default, Debug)]
pub struct Info {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_video: bool,
    pub has_audio: bool,
    /// (Bezeichnung, Wert) für die Vorschau: Format, Dauer, Auflösung, Codecs, Bitrate …
    pub rows: Vec<(String, String)>,
}

fn hms(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Endungen, die der eingebaute Dekoder ohne ffmpeg abspielt.
fn native_audio(p: &Path) -> bool {
    matches!(convert::ext_of(p).as_str(), "mp3" | "wav" | "flac" | "m4a" | "aac" | "m4b")
}

/// Lässt sich die Datei abspielen? `Err` = Grund (z. B. ffmpeg fehlt).
pub fn playable(p: &Path, info: &Info) -> Result<(), String> {
    if convert::ffmpeg().is_some() {
        return Ok(());
    }
    if !info.has_video && native_audio(p) {
        return Ok(());
    }
    Err(if info.has_video || convert::class_of(p) == convert::Class::Video {
        "Zum Abspielen von Videos wird ffmpeg benötigt.".into()
    } else {
        format!("Für „.{}“ wird ffmpeg benötigt (MP3, WAV, FLAC und M4A gehen ohne).", convert::ext_of(p))
    })
}

fn ratio(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => a.parse::<f64>().unwrap_or(0.0) / b.parse::<f64>().unwrap_or(1.0).max(1e-9),
        None => s.parse().unwrap_or(0.0),
    }
}

/// Kenndaten lesen (blockiert kurz – im Hintergrund aufrufen).
pub fn probe(p: &Path) -> Info {
    let mut info = Info::default();
    let is_video = convert::class_of(p) == convert::Class::Video || convert::ext_of(p) == "gif";
    if let Some(fp) = convert::ffprobe() {
        let out = convert::quiet(&fp)
            .args(["-v", "error", "-show_entries", "format=duration,bit_rate,format_long_name:stream=codec_type,codec_name,width,height,r_frame_rate,avg_frame_rate,sample_rate,channels,channel_layout:stream_disposition=attached_pic", "-of", "json"])
            .arg(p)
            .stdin(Stdio::null())
            .output();
        if let Some(v) = out.ok().and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok()) {
            let f = &v["format"];
            info.duration = f["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
            if let Some(n) = f["format_long_name"].as_str() {
                info.rows.push(("Format".into(), n.to_string()));
            }
            if info.duration > 0.0 {
                info.rows.push(("Dauer".into(), hms(info.duration)));
            }
            for st in v["streams"].as_array().into_iter().flatten() {
                let codec = st["codec_name"].as_str().unwrap_or("?").to_uppercase();
                match st["codec_type"].as_str() {
                    // eingebettetes Cover (MP3/M4A) ist kein Video
                    Some("video") if st["disposition"]["attached_pic"].as_i64() != Some(1) && !info.has_video => {
                        info.has_video = true;
                        info.width = st["width"].as_u64().unwrap_or(0) as u32;
                        info.height = st["height"].as_u64().unwrap_or(0) as u32;
                        let fps = [st["avg_frame_rate"].as_str(), st["r_frame_rate"].as_str()].into_iter().flatten().map(ratio).find(|f| *f > 0.5 && *f < 400.0);
                        info.fps = fps.unwrap_or(25.0);
                        info.rows.push(("Auflösung".into(), format!("{} × {} px", info.width, info.height)));
                        info.rows.push(("Video".into(), format!("{codec}, {:.3} fps", info.fps).replace(".000 fps", " fps")));
                    }
                    Some("audio") if !info.has_audio => {
                        info.has_audio = true;
                        let hz = st["sample_rate"].as_str().and_then(|s| s.parse::<f64>().ok()).map(|h| format!("{:.1} kHz", h / 1000.0).replace(".0 kHz", " kHz")).unwrap_or_default();
                        let ch = match st["channels"].as_u64() {
                            Some(1) => "Mono".to_string(),
                            Some(2) => "Stereo".to_string(),
                            Some(n) => st["channel_layout"].as_str().map_or(format!("{n} Kanäle"), |l| format!("{n} Kanäle ({l})")),
                            None => String::new(),
                        };
                        info.rows.push(("Audio".into(), [codec, hz, ch].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")));
                    }
                    _ => {}
                }
            }
            if let Some(b) = f["bit_rate"].as_str().and_then(|b| b.parse::<f64>().ok()) {
                info.rows.push(("Bitrate".into(), if b >= 1e6 { format!("{:.1} Mbit/s", b / 1e6) } else { format!("{:.0} kbit/s", b / 1e3) }));
            }
            return info;
        }
    }
    // ohne ffprobe: Audio über den eingebauten Dekoder
    if !is_video {
        if let Ok(d) = std::fs::File::open(p).map_err(|e| e.to_string()).and_then(|f| rodio::Decoder::new(BufReader::new(f)).map_err(|e| e.to_string())) {
            info.has_audio = true;
            info.duration = d.total_duration().map_or(0.0, |t| t.as_secs_f64());
            if info.duration > 0.0 {
                info.rows.push(("Dauer".into(), hms(info.duration)));
            }
            info.rows.push(("Audio".into(), format!("{:.1} kHz, {}", d.sample_rate() as f64 / 1000.0, if d.channels() == 1 { "Mono".into() } else { format!("{} Kanäle", d.channels()) }).replace(".0 kHz", " kHz")));
        }
    } else {
        info.has_video = true;
    }
    info.rows.push(("Hinweis".into(), "Für alle Details und das Abspielen von Videos ffmpeg installieren".into()));
    info
}

/// ffmpeg-Prozess, der beim Verwerfen beendet wird.
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Tonquelle aus einer PCM-Pipe (Stereo, 16 Bit, 48 kHz). Endet, wenn ffmpeg fertig ist oder beendet wird.
struct Pipe {
    rx: Receiver<Vec<i16>>,
    buf: Vec<i16>,
    pos: usize,
}

impl Iterator for Pipe {
    type Item = i16;
    fn next(&mut self) -> Option<i16> {
        while self.pos >= self.buf.len() {
            self.buf = self.rx.recv().ok()?;
            self.pos = 0;
        }
        self.pos += 1;
        Some(self.buf[self.pos - 1])
    }
}

impl Source for Pipe {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn ffmpeg_cmd(args: &[&str], src: &Path, at: f64) -> Result<Child, String> {
    let ff = convert::ffmpeg().ok_or("ffmpeg ist nicht installiert")?;
    let mut c = convert::quiet(&ff);
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &format!("{at:.3}"), "-i"]).arg(src).args(args).arg("-");
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    c.spawn().map_err(|e| format!("ffmpeg konnte nicht gestartet werden: {e}"))
}

/// atempo-Filterkette für `s` (ein Glied kann nur 0,5–2,0).
pub fn atempo(s: f32) -> String {
    let mut s = s as f64;
    let mut parts = Vec::new();
    while s < 0.5 {
        parts.push("atempo=0.5".to_string());
        s /= 0.5;
    }
    while s > 2.0 {
        parts.push("atempo=2.0".to_string());
        s /= 2.0;
    }
    parts.push(format!("atempo={s:.4}"));
    parts.join(",")
}

/// „1,5×“, „0,25×“, „3×“ (höchstens zwei Nachkommastellen)
fn speed_label(s: f32) -> String {
    let t = format!("{s:.2}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    format!("{t}×").replace('.', ",")
}

/// Eigene Geschwindigkeit aus der Eingabe („2,5“, „3x“, „0.8×“); nur positive Werte.
fn parse_speed(s: &str) -> Option<f32> {
    let v: f32 = s.trim().trim_end_matches(['×', 'x', 'X']).trim().replace(',', ".").parse().ok()?;
    (v.is_finite() && v > 0.0).then(|| v.clamp(MIN_CUSTOM, MAX_SPEED))
}

/// Gedrückthalten im Bild bzw. in der Wellenform: Beginn, vorherige Geschwindigkeit, 2× schon aktiv?
struct Hold {
    since: Instant,
    prev: f32,
    active: bool,
}

#[derive(Default)]
struct WaveData {
    /// (Spitze, Effektivwert) je Abschnitt von WAVE_BLOCK Sekunden, 0..1
    blocks: Vec<[f32; 2]>,
    max_peak: f32,
    done: bool,
}

/// Wellenform einer Audiodatei, wird im Hintergrund aus dem echten Signal berechnet (zuerst sichtbar, was
/// schon dekodiert ist). Beim Verwerfen endet die Berechnung.
pub struct Waveform {
    data: Arc<Mutex<WaveData>>,
    stop: Arc<AtomicBool>,
}

impl Waveform {
    /// Kopie der bisher berechneten Pegel (Spitze, Effektivwert je WAVE_BLOCK), größte Spitze, fertig?
    pub fn snapshot(&self) -> (Vec<[f32; 2]>, f32, bool) {
        let d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        (d.blocks.clone(), d.max_peak, d.done)
    }

    pub fn start(path: &Path, ctx: &egui::Context) -> Waveform {
        let data: Arc<Mutex<WaveData>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (d, s, p, c) = (data.clone(), stop.clone(), path.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let native = native_audio(&p)
                .then(|| std::fs::File::open(&p).ok().and_then(|f| rodio::Decoder::new(BufReader::new(f)).ok()))
                .flatten();
            match native {
                Some(dec) => {
                    let (ch, rate) = (dec.channels().max(1) as usize, dec.sample_rate());
                    scan(dec, ch, rate, &d, &s, &c);
                }
                None => {
                    if let Ok(mut child) = ffmpeg_cmd(&["-vn", "-sn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-acodec", "pcm_s16le"], &p, 0.0) {
                        if let Some(out) = child.stdout.take() {
                            let _proc = Proc(child);
                            let mut r = BufReader::with_capacity(64 * 1024, out);
                            let samples = std::iter::from_fn(move || {
                                let mut b = [0u8; 2];
                                r.read_exact(&mut b).ok().map(|_| i16::from_le_bytes(b))
                            });
                            scan(samples, 1, 8000, &d, &s, &c);
                        }
                    }
                }
            }
            if let Ok(mut w) = d.lock() {
                w.done = true;
            }
            c.request_repaint();
        });
        Waveform { data, stop }
    }
}

impl Drop for Waveform {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Pegel je Abschnitt berechnen und in Paketen veröffentlichen.
fn scan(samples: impl Iterator<Item = i16>, ch: usize, rate: u32, data: &Mutex<WaveData>, stop: &AtomicBool, ctx: &egui::Context) {
    let per = ((rate as f64 * WAVE_BLOCK).round() as usize).max(1) * ch;
    let (mut n, mut peak, mut sq) = (0usize, 0f32, 0f64);
    let mut batch: Vec<[f32; 2]> = Vec::new();
    let flush = |batch: &mut Vec<[f32; 2]>| {
        if let Ok(mut w) = data.lock() {
            for b in batch.iter() {
                w.max_peak = w.max_peak.max(b[0]);
            }
            w.blocks.append(batch);
        }
        ctx.request_repaint();
    };
    for x in samples {
        let v = x as f32 / 32768.0;
        peak = peak.max(v.abs());
        sq += (v * v) as f64;
        n += 1;
        if n == per {
            batch.push([peak, (sq / n as f64).sqrt() as f32]);
            (n, peak, sq) = (0, 0.0, 0.0);
            if batch.len() >= 250 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                flush(&mut batch);
            }
        }
    }
    if n > 0 {
        batch.push([peak, (sq / n as f64).sqrt() as f32]);
    }
    flush(&mut batch);
}

pub struct Player {
    pub path: PathBuf,
    pub info: Info,
    ctx: egui::Context,
    // Ton
    stream: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,
    sink: Option<rodio::Sink>,
    // Uhr: Medienzeit am Beginn des Abschnitts + vergangene Zeit (Tonausgabe bzw. Stoppuhr)
    base: f64,
    wall: Instant,
    paused: bool,
    paused_at: f64,
    ended: bool,
    pub volume: f32,
    pub muted: bool,
    // Bild
    frames: Option<Receiver<Vec<u8>>>,
    frame_size: (u32, u32),
    out_fps: f64,
    frame_no: u64,
    next: Option<(f64, Vec<u8>)>,
    video_done: bool,
    /// in der Pause gesprungen: ein Standbild der neuen Stelle holen
    want_still: bool,
    tex: Option<egui::TextureHandle>,
    procs: Vec<Proc>,
    // Oberfläche
    /// Standbild bis zum ersten dekodierten Bild
    pub poster: Option<egui::TextureHandle>,
    /// Dekoder wurden schon einmal gestartet (vorher: Standbild, nichts läuft)
    armed: bool,
    speed: f32,
    /// kürzere Bildseite in Pixeln, 0 = automatisch (höchstens 1280 × 720)
    quality: u32,
    menu: bool,
    /// Regler wird gezogen (Wert wird beim Loslassen übernommen) bzw. eigene Eingabe offen (Text)
    speed_drag: Option<f32>,
    custom: Option<String>,
    hold: Option<Hold>,
    /// spielt im Bild-in-Bild-Fenster; `pip_request` = Knopf gedrückt (Besitzer wechselt den Ort)
    pub pip: bool,
    pub pip_request: bool,
    /// letzte Mausbewegung im Vollbild (Leiste blendet sich danach aus)
    last_move: Instant,
    // Oberfläche
    pub fullscreen: bool,
    fs_sent: bool,
    drag: Option<f64>,
    pub last_frame: u64,
    pub err: Option<String>,
    /// Untertitel (.srt/.vtt mit gleichem Namen): Datei, Änderungszeit, Einträge; ein-/ausblendbar
    subs: Option<(PathBuf, Option<std::time::SystemTime>, Vec<crate::transcribe::Cue>)>,
    pub subs_on: bool,
    subs_look: Option<Instant>,
}

impl Player {
    /// Untertitel neben dem Video suchen bzw. nach Änderung neu lesen (alle 2 s)
    fn load_subs(&mut self) {
        if self.subs_look.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        self.subs_look = Some(Instant::now());
        let Some(f) = crate::transcribe::subs_for(&self.path) else {
            self.subs = None;
            return;
        };
        let mt = std::fs::metadata(&f).and_then(|m| m.modified()).ok();
        if self.subs.as_ref().is_some_and(|(p, m, _)| *p == f && *m == mt) {
            return;
        }
        let cues = std::fs::read_to_string(&f).map(|t| crate::transcribe::parse_srt(&t)).unwrap_or_default();
        self.subs = Some((f, mt, cues));
    }

    /// aktueller Untertitel unten im Bild
    fn paint_subs(&self, ui: &egui::Ui, r: Rect) {
        let Some((_, _, cues)) = self.subs.as_ref().filter(|_| self.subs_on) else { return };
        let t = self.pos();
        let Some(c) = cues.iter().find(|c| t >= c.0 && t <= c.1) else { return };
        let size = (r.height() / 22.0).clamp(13.0, 34.0);
        let g = ui.fonts(|f| f.layout(c.2.clone(), FontId::proportional(size), Color32::WHITE, r.width() * 0.86));
        let bottom = r.bottom() - if self.fullscreen { 90.0 } else { r.height() * 0.08 + 10.0 };
        let box_ = Rect::from_center_size(pos2(r.center().x, bottom - g.size().y / 2.0), g.size() + vec2(18.0, 8.0));
        ui.painter().rect_filled(box_, 5.0, Color32::from_black_alpha(170));
        ui.painter().galley(box_.min + vec2(9.0, 4.0), g, Color32::WHITE);
    }

    pub fn has_subs(&self) -> bool {
        self.subs.as_ref().is_some_and(|s| !s.2.is_empty())
    }

    /// Neuer Player, angehalten am Anfang; die Dekoder starten erst mit dem ersten Abspielen.
    pub fn new(path: PathBuf, info: Info, ctx: &egui::Context, volume: f32) -> Player {
        Player {
            path,
            info,
            ctx: ctx.clone(),
            stream: None,
            sink: None,
            base: 0.0,
            wall: Instant::now(),
            paused: true,
            paused_at: 0.0,
            ended: false,
            volume,
            muted: false,
            frames: None,
            frame_size: (0, 0),
            out_fps: 25.0,
            frame_no: 0,
            next: None,
            video_done: true,
            want_still: false,
            tex: None,
            procs: Vec::new(),
            poster: None,
            armed: false,
            speed: 1.0,
            quality: 0,
            menu: false,
            speed_drag: None,
            custom: None,
            hold: None,
            pip: false,
            pip_request: false,
            last_move: Instant::now(),
            fullscreen: false,
            fs_sent: false,
            drag: None,
            last_frame: ctx.cumulative_pass_nr(),
            err: None,
            subs: None,
            subs_on: true,
            subs_look: None,
        }
    }

    /// Als gerade gezeigt markieren (Zählung des Hauptfensters).
    pub fn touch(&mut self) {
        self.last_frame = self.ctx.cumulative_pass_nr();
    }

    /// angehalten?
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn has_video(&self) -> bool {
        self.info.has_video
    }

    /// Aktuelle Abspielzeit in Sekunden.
    pub fn pos(&self) -> f64 {
        if self.paused {
            return self.paused_at;
        }
        let t = match &self.sink {
            // Ausgabezeit × Geschwindigkeit = Medienzeit
            Some(s) => self.base + s.get_pos().as_secs_f64() * self.speed as f64,
            None => self.base + self.wall.elapsed().as_secs_f64() * self.speed as f64,
        };
        if self.info.duration > 0.0 {
            t.min(self.info.duration)
        } else {
            t
        }
    }

    fn apply_volume(&self) {
        if let Some(s) = &self.sink {
            // quadratisch wirkt für das Ohr gleichmäßiger
            s.set_volume(if self.muted { 0.0 } else { self.volume * self.volume });
        }
    }

    /// Dekoder ab `at` (Sekunden) neu starten.
    fn start(&mut self, at: f64) {
        self.procs.clear(); // beendet laufende ffmpeg-Prozesse → alte Pipes enden
        self.armed = true;
        self.sink = None;
        self.frames = None;
        self.next = None;
        self.err = None;
        self.ended = false;
        self.base = at;
        self.frame_no = 0;
        self.video_done = !self.info.has_video;
        // Ton
        if self.info.has_audio {
            if self.stream.is_none() {
                match rodio::OutputStream::try_default() {
                    Ok(s) => self.stream = Some(s),
                    Err(e) => self.err = Some(format!("Keine Tonausgabe: {e}")),
                }
            }
            if let Some((_, h)) = &self.stream {
                match rodio::Sink::try_new(h) {
                    Ok(sink) => {
                        // eingebauter Dekoder für reine Audiodateien; was er nicht kann, übernimmt ffmpeg
                        // (bei anderer Geschwindigkeit bevorzugt ffmpeg, dort bleibt die Tonhöhe)
                        let native = if !self.info.has_video && native_audio(&self.path) && (self.speed == 1.0 || convert::ffmpeg().is_none()) {
                            std::fs::File::open(&self.path).map_err(|e| e.to_string()).and_then(|f| rodio::Decoder::new(BufReader::new(f)).map_err(|e| e.to_string()))
                        } else {
                            Err(String::new())
                        };
                        let ok = match native {
                            Ok(mut d) => {
                                if at > 0.0 {
                                    let _ = d.try_seek(Duration::from_secs_f64(at));
                                }
                                sink.append(d);
                                sink.set_speed(self.speed);
                                true
                            }
                            Err(e) if convert::ffmpeg().is_none() => {
                                self.err = Some(if e.is_empty() { "ffmpeg ist nicht installiert".into() } else { format!("Audio kann nicht gelesen werden: {e}") });
                                false
                            }
                            Err(_) => match self.spawn_audio(at) {
                                Ok(src) => {
                                    sink.append(src);
                                    true
                                }
                                Err(e) => {
                                    self.err = Some(e);
                                    false
                                }
                            },
                        };
                        if ok {
                            if self.paused {
                                sink.pause();
                            }
                            self.sink = Some(sink);
                            self.apply_volume();
                        }
                    }
                    Err(e) => self.err = Some(format!("Keine Tonausgabe: {e}")),
                }
            }
        }
        // Bild
        if self.info.has_video {
            if let Err(e) = self.spawn_video(at) {
                self.err = Some(e);
                self.video_done = true;
            }
        }
        self.wall = Instant::now();
        if self.paused {
            self.paused_at = at;
            self.want_still = self.info.has_video;
        }
        self.ctx.request_repaint();
    }

    fn spawn_audio(&mut self, at: f64) -> Result<Pipe, String> {
        let rate = RATE.to_string();
        let tempo = atempo(self.speed);
        let mut args = vec!["-vn", "-sn"];
        if self.speed != 1.0 {
            args.extend(["-af", tempo.as_str()]);
        }
        args.extend(["-f", "s16le", "-acodec", "pcm_s16le", "-ac", "2", "-ar", rate.as_str()]);
        let mut child = ffmpeg_cmd(&args, &self.path, at)?;
        let mut out = child.stdout.take().ok_or("keine Ausgabe")?;
        let (tx, rx): (SyncSender<Vec<i16>>, _) = mpsc::sync_channel(24);
        std::thread::spawn(move || {
            // 0,1 s je Paket
            let mut bytes = vec![0u8; (RATE as usize / 10) * 4];
            loop {
                let mut n = 0;
                while n < bytes.len() {
                    match out.read(&mut bytes[n..]) {
                        Ok(0) | Err(_) => break,
                        Ok(k) => n += k,
                    }
                }
                let n = n - n % 2;
                if n == 0 {
                    break;
                }
                let samples: Vec<i16> = bytes[..n].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
                if tx.send(samples).is_err() || n < bytes.len() {
                    break;
                }
            }
        });
        self.procs.push(Proc(child));
        Ok(Pipe { rx, buf: Vec::new(), pos: 0 })
    }

    fn spawn_video(&mut self, at: f64) -> Result<(), String> {
        let (w0, h0) = (self.info.width.max(2), self.info.height.max(2));
        let k = if self.quality == 0 {
            (MAX_W as f64 / w0 as f64).min(MAX_H as f64 / h0 as f64)
        } else {
            self.quality as f64 / w0.min(h0) as f64
        }
        .min(1.0);
        let even = |v: f64| ((v.round() as u32) / 2 * 2).max(2);
        let (w, h) = (even(w0 as f64 * k), even(h0 as f64 * k));
        self.out_fps = self.info.fps.clamp(1.0, 30.0);
        self.frame_size = (w, h);
        // setpts: schneller/langsamer; Bilder kommen danach im Takt der Ausgabezeit
        let vf = format!("setpts=PTS/{:.4},fps={:.3},scale={w}:{h}:flags=bilinear", self.speed, self.out_fps);
        let mut child = ffmpeg_cmd(&["-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba"], &self.path, at)?;
        let mut out = child.stdout.take().ok_or("keine Ausgabe")?;
        let (tx, rx) = mpsc::sync_channel(4);
        let frame = (w * h * 4) as usize;
        let ctx = self.ctx.clone();
        std::thread::spawn(move || loop {
            let mut buf = vec![0u8; frame];
            if out.read_exact(&mut buf).is_err() || tx.send(buf).is_err() {
                break;
            }
            ctx.request_repaint();
        });
        self.procs.push(Proc(child));
        self.frames = Some(rx);
        Ok(())
    }

    pub fn toggle(&mut self) {
        if !self.armed {
            self.paused = false;
            self.start(self.paused_at);
            return;
        }
        if self.ended {
            self.paused = false;
            self.start(0.0);
            return;
        }
        if self.paused {
            self.paused = false;
            match &self.sink {
                Some(s) => s.play(),
                None => {
                    self.base = self.paused_at;
                    self.wall = Instant::now();
                }
            }
        } else {
            self.paused_at = self.pos();
            self.paused = true;
            if let Some(s) = &self.sink {
                s.pause();
            }
        }
        self.ctx.request_repaint();
    }

    pub fn pause(&mut self) {
        if !self.paused {
            self.toggle();
        }
    }

    pub fn seek(&mut self, t: f64) {
        let t = if self.info.duration > 0.0 { t.clamp(0.0, (self.info.duration - 0.05).max(0.0)) } else { t.max(0.0) };
        if self.paused {
            self.paused_at = t;
        }
        self.start(t);
    }

    /// Dekoder an der aktuellen Stelle mit den neuen Einstellungen neu starten (falls sie schon liefen).
    fn restart_at(&mut self, t: f64) {
        if self.armed {
            if self.paused {
                self.paused_at = t;
            }
            self.start(t);
        }
    }

    pub fn set_speed(&mut self, s: f32) {
        if s != self.speed {
            // Stelle mit der alten Geschwindigkeit bestimmen, sonst springt die Uhr
            let t = self.pos();
            self.speed = s;
            self.restart_at(t);
        }
    }

    pub fn set_quality(&mut self, q: u32) {
        if q != self.quality {
            self.quality = q;
            let t = self.pos();
            self.restart_at(t);
        }
    }

    /// Nur für Bildschirmfotos: „<Sekunden>[,menu][,custom][,pip][,s=<Geschwindigkeit>]“ springt angehalten an
    /// die Stelle, öffnet Menü/eigene Eingabe, startet Bild-in-Bild bzw. stellt die Geschwindigkeit ein.
    pub fn dev_shot(&mut self, v: &str) {
        let mut it = v.split(',').map(str::trim);
        if let Some(t) = it.next().and_then(|s| s.parse::<f64>().ok()) {
            self.seek(t);
        }
        for tok in it {
            match tok {
                "menu" => self.menu = true,
                "custom" => self.custom = Some(String::new()),
                "pip" => self.pip_request = true,
                "fs" => self.fullscreen = true,
                _ => {
                    if let Some(s) = tok.strip_prefix("s=").and_then(parse_speed) {
                        self.speed = s;
                    }
                }
            }
        }
    }

    /// Qualitätsstufen, die das Video hergibt (kürzere Bildseite).
    fn qualities(&self) -> Vec<u32> {
        let short = self.info.width.min(self.info.height);
        QUALITIES.into_iter().filter(|q| *q <= short).collect()
    }

    /// Fällige Bilder übernehmen, Ende erkennen. Jedes Frame aufrufen.
    fn update(&mut self) {
        if self.paused {
            // nach dem Springen in der Pause: das erste Bild der neuen Stelle zeigen
            if self.want_still {
                if let Some(Ok(b)) = self.frames.as_ref().map(|rx| rx.try_recv()) {
                    self.frame_no += 1;
                    self.upload(&b);
                    self.want_still = false;
                } else {
                    self.ctx.request_repaint_after(Duration::from_millis(30));
                }
            }
            return;
        }
        let t = self.pos();
        if let Some(rx) = &self.frames {
            let mut due: Option<Vec<u8>> = None;
            loop {
                if self.next.is_none() {
                    match rx.try_recv() {
                        Ok(b) => {
                            if self.frame_no == 0 && self.sink.is_none() {
                                // ohne Ton: Uhr erst mit dem ersten Bild starten (ffmpeg braucht einen Moment)
                                self.wall = Instant::now();
                            }
                            let ts = self.base + self.frame_no as f64 / self.out_fps * self.speed as f64;
                            self.frame_no += 1;
                            self.next = Some((ts, b));
                        }
                        Err(TryRecvError::Disconnected) => {
                            self.video_done = true;
                            break;
                        }
                        Err(TryRecvError::Empty) => break,
                    }
                }
                match &self.next {
                    Some((ts, _)) if *ts <= t + 0.008 => due = self.next.take().map(|n| n.1),
                    _ => break,
                }
            }
            if let Some(px) = due {
                self.upload(&px);
            }
        }
        let audio_done = self.sink.as_ref().is_none_or(|s| s.empty());
        let past_end = self.info.duration > 0.0 && t >= self.info.duration - 0.03;
        if (audio_done && self.video_done && self.next.is_none()) || (past_end && self.next.is_none() && audio_done) {
            self.ended = true;
            self.paused = true;
            self.paused_at = if self.info.duration > 0.0 { self.info.duration } else { t };
        }
        let wait = match &self.next {
            Some((ts, _)) => (ts - t).clamp(0.004, 0.05),
            None => 0.03,
        };
        self.ctx.request_repaint_after(Duration::from_secs_f64(wait));
    }

    fn upload(&mut self, px: &[u8]) {
        let (w, h) = self.frame_size;
        let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], px);
        match &mut self.tex {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => self.tex = Some(self.ctx.load_texture("player", img, egui::TextureOptions::LINEAR)),
        }
    }

    /// Bildgröße (Seitenverhältnis) des Videos.
    pub fn aspect(&self) -> f32 {
        if self.info.width > 0 && self.info.height > 0 {
            self.info.width as f32 / self.info.height as f32
        } else {
            16.0 / 9.0
        }
    }

    /// Video in `r` zeichnen (Seitenverhältnis bleibt, Rest schwarz). Klick = Pause, Doppelklick = Vollbild.
    pub fn video(&mut self, ui: &mut egui::Ui, r: Rect) {
        self.update();
        self.last_frame = ui.ctx().cumulative_pass_nr();
        ui.painter().rect_filled(r, if self.fullscreen { 0.0 } else { 6.0 }, Color32::BLACK);
        if let Some(t) = self.tex.as_ref().or(self.poster.as_ref()) {
            let a = self.aspect();
            let (w, h) = if r.width() / r.height() > a { (r.height() * a, r.height()) } else { (r.width(), r.width() / a) };
            ui.painter().image(t.id(), Rect::from_center_size(r.center(), vec2(w, h)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if self.tex.is_none() && self.err.is_none() && !self.paused {
            ui.put(Rect::from_center_size(r.center(), vec2(24.0, 24.0)), egui::Spinner::new().color(Color32::from_gray(200)));
        }
        let resp = ui.interact(r, ui.id().with("pl_video"), Sense::click());
        if self.hold_boost(ui, &resp) {
            // Gedrückthalten = 2×, kein Klick
        } else if resp.double_clicked() {
            if !self.pip {
                self.fullscreen = !self.fullscreen;
            }
        } else if resp.clicked() {
            self.toggle();
        }
        self.boost_badge(ui, r);
        self.load_subs();
        self.paint_subs(ui, r);
        // kein zusätzliches Abspielsymbol im Bild: gestartet wird über die Steuerleiste (oder Klick ins Bild)
    }

    /// Gedrückthalten (Bild, Wellenform): nach kurzer Zeit 2× – nur bei laufender Wiedergabe unter 2×, eine
    /// höher eingestellte Geschwindigkeit bleibt unberührt. Loslassen (oder Wegziehen) stellt die vorherige
    /// Geschwindigkeit wieder her. `true` = dieser Druck gehört dem Gedrückthalten (kein Klick, kein Springen).
    fn hold_boost(&mut self, ui: &egui::Ui, resp: &egui::Response) -> bool {
        let (moved, down) = ui.input(|i| {
            let moved = match (i.pointer.press_origin(), i.pointer.latest_pos()) {
                (Some(a), Some(b)) => a.distance(b) > 6.0,
                _ => false,
            };
            (moved, i.pointer.primary_down())
        });
        // Beginn: Druck auf diesem Bereich. Danach zählt nur noch die gedrückte Taste – egui gibt einen reinen
        // Klick-Bereich nach knapp einer Sekunde frei, das Gedrückthalten soll aber bis zum Loslassen gelten.
        let pressing = if self.hold.is_some() { down } else { resp.is_pointer_button_down_on() };
        if pressing && !moved {
            let start = match &self.hold {
                None if !self.paused && self.speed < 2.0 => {
                    self.hold = Some(Hold { since: Instant::now(), prev: self.speed, active: false });
                    false
                }
                Some(h) => !h.active && h.since.elapsed() >= HOLD,
                None => false,
            };
            if start {
                if let Some(h) = &mut self.hold {
                    h.active = true;
                }
                self.set_speed(2.0);
            }
            if self.hold.as_ref().is_some_and(|h| !h.active) {
                ui.ctx().request_repaint_after(Duration::from_millis(40));
            }
            return self.hold.as_ref().is_some_and(|h| h.active);
        }
        match self.hold.take() {
            Some(h) if h.active => {
                self.set_speed(h.prev);
                true
            }
            _ => false,
        }
    }

    /// Hinweis „2×“ oben in der Mitte, solange gedrückt gehalten wird.
    fn boost_badge(&self, ui: &egui::Ui, r: Rect) {
        if self.hold.as_ref().is_some_and(|h| h.active) {
            let pill = Rect::from_center_size(pos2(r.center().x, r.top() + 24.0), vec2(66.0, 26.0));
            let p = ui.painter();
            p.rect_filled(pill, 13.0, Color32::from_black_alpha(170));
            p.text(pos2(pill.left() + 14.0, pill.center().y), egui::Align2::LEFT_CENTER, "2×", FontId::proportional(13.0), Color32::WHITE);
            // Vorspul-Symbol als Form (die Schrift hat kein ▸)
            for k in 0..2 {
                let x = pill.right() - 30.0 + k as f32 * 8.0;
                let c = pill.center().y;
                p.add(Shape::convex_polygon(vec![pos2(x, c - 5.0), pos2(x + 8.0, c), pos2(x, c + 5.0)], Color32::WHITE, Stroke::NONE));
            }
        }
    }

    /// Bedienleiste: Abspielen/Pause, Zeit, Zeitleiste, Lautstärke, Vollbild. `dark` = über dem Video.
    pub fn controls(&mut self, ui: &mut egui::Ui, th: &Theme, r: Rect, dark: bool) {
        if !self.info.has_video {
            self.update();
        }
        self.last_frame = ui.ctx().cumulative_pass_nr();
        let (fg, sub, track) = if dark { (Color32::WHITE, Color32::from_gray(190), Color32::from_white_alpha(70)) } else { (th.text, th.sub, th.line) };
        let p = ui.painter().clone();
        let mid = r.center().y;
        let mut x = r.left();
        // Abspielen/Pause
        let b = Rect::from_center_size(pos2(x + 16.0, mid), vec2(32.0, 32.0));
        let resp = ui.interact(b, ui.id().with("pl_play"), Sense::click());
        if resp.hovered() {
            p.circle_filled(b.center(), 16.0, if dark { Color32::from_white_alpha(40) } else { th.hover });
        }
        let ic = Rect::from_center_size(b.center(), vec2(14.0, 14.0));
        if self.paused {
            play_icon(&p, ic.translate(vec2(1.5, 0.0)), fg);
        } else {
            pause_icon(&p, ic, fg);
        }
        if resp.on_hover_text(if self.paused { "Abspielen (Leertaste)" } else { "Pause (Leertaste)" }).clicked() {
            self.toggle();
        }
        x += 38.0;
        // Platz einteilen, damit auch in einer schmalen Vorschau nichts überlappt: reicht er nicht, wird zuerst
        // der Lautstärkeregler zum Symbol (Mausrad darauf ändert die Lautstärke), dann die Zeit kürzer
        // (nur aktuelle Zeit), zuletzt entfällt sie – die Zeitleiste behält immer mindestens 50 px
        let cur = self.drag.unwrap_or_else(|| self.pos());
        let buttons = 1 + usize::from(self.info.has_video && !self.fullscreen) + usize::from(self.info.has_video && !self.pip) + usize::from(self.info.has_video && self.has_subs());
        let room = r.right() - x - buttons as f32 * 34.0;
        let width = |s: &str| ui.fonts(|f| f.layout_no_wrap(s.to_string(), FontId::monospace(11.5), sub).size().x);
        let full = if self.info.duration > 0.0 { format!("{} / {}", hms(cur), hms(self.info.duration)) } else { hms(cur) };
        let short = hms(cur);
        const TRACK: f32 = 50.0;
        const VOL_FULL: f32 = 101.0;
        const VOL_ICON: f32 = 38.0;
        let (time, slider) = if room >= width(&full) + 12.0 + TRACK + VOL_FULL {
            (Some(full), true)
        } else if room >= width(&full) + 12.0 + TRACK + VOL_ICON {
            (Some(full), false)
        } else if room >= width(&short) + 12.0 + TRACK + VOL_ICON {
            (Some(short), false)
        } else {
            (None, false)
        };
        if let Some(time) = time {
            let g = ui.fonts(|f| f.layout_no_wrap(time, FontId::monospace(11.5), sub));
            p.galley(pos2(x, mid - g.size().y / 2.0), g.clone(), sub);
            x += g.size().x + 12.0;
        }
        // rechts: Vollbild, Lautstärke
        let mut right = r.right();
        // Bild-in-Bild: schwebendes Fenster bzw. zurück in die Vorschau (nicht aus dem Vollbild heraus)
        if self.info.has_video && !self.fullscreen {
            let pb = Rect::from_center_size(pos2(right - 14.0, mid), vec2(28.0, 28.0));
            let resp = ui.interact(pb, ui.id().with("pl_pip"), Sense::click());
            if resp.hovered() || self.pip {
                p.rect_filled(pb, 6.0, if dark { Color32::from_white_alpha(40) } else { th.hover });
            }
            pip_icon(&p, Rect::from_center_size(pb.center(), vec2(16.0, 13.0)), fg, self.pip);
            if resp.on_hover_text(if self.pip { "Zurück in die Vorschau" } else { "Bild-in-Bild" }).clicked() {
                self.pip_request = true;
            }
            right -= 34.0;
        }
        if self.info.has_video && !self.pip {
            let fb = Rect::from_center_size(pos2(right - 14.0, mid), vec2(28.0, 28.0));
            let resp = ui.interact(fb, ui.id().with("pl_fs"), Sense::click());
            if resp.hovered() {
                p.rect_filled(fb, 6.0, if dark { Color32::from_white_alpha(40) } else { th.hover });
            }
            fullscreen_icon(&p, Rect::from_center_size(fb.center(), vec2(14.0, 14.0)), fg, self.fullscreen);
            if resp.on_hover_text(if self.fullscreen { "Vollbild verlassen (Esc)" } else { "Vollbild (F, Doppelklick)" }).clicked() {
                self.fullscreen = !self.fullscreen;
            }
            right -= 34.0;
        }
        // Untertitel ein/aus (nur wenn eine .srt/.vtt daneben liegt)
        if self.info.has_video && self.has_subs() {
            let cb = Rect::from_center_size(pos2(right - 14.0, mid), vec2(28.0, 28.0));
            let resp = ui.interact(cb, ui.id().with("pl_cc"), Sense::click());
            if resp.hovered() {
                p.rect_filled(cb, 6.0, if dark { Color32::from_white_alpha(40) } else { th.hover });
            }
            let cr = Rect::from_center_size(cb.center(), vec2(20.0, 14.0));
            let col = if self.subs_on { fg } else { sub };
            if self.subs_on {
                p.rect_filled(cr, 3.0, col);
                p.text(cr.center(), egui::Align2::CENTER_CENTER, "CC", FontId::proportional(9.5), if dark { Color32::BLACK } else { th.bg });
            } else {
                p.rect_stroke(cr, 3.0, Stroke::new(1.3, col));
                p.text(cr.center(), egui::Align2::CENTER_CENTER, "CC", FontId::proportional(9.5), col);
            }
            if resp.on_hover_text(if self.subs_on { "Untertitel ausblenden (C)" } else { "Untertitel einblenden (C)" }).clicked() {
                self.subs_on = !self.subs_on;
            }
            right -= 34.0;
        }
        // Einstellungen: Qualität (Video) und Geschwindigkeit
        let gb = Rect::from_center_size(pos2(right - 14.0, mid), vec2(28.0, 28.0));
        let resp = ui.interact(gb, ui.id().with("pl_gear"), Sense::click());
        if resp.hovered() || self.menu {
            p.rect_filled(gb, 6.0, if dark { Color32::from_white_alpha(40) } else { th.hover });
        }
        gear_icon(&p, Rect::from_center_size(gb.center(), vec2(15.0, 15.0)), fg);
        if self.speed != 1.0 {
            // aktuelle Geschwindigkeit als kleines Abzeichen
            p.text(gb.right_bottom() + vec2(1.0, 1.0), egui::Align2::RIGHT_BOTTOM, speed_label(self.speed), FontId::proportional(8.5), th.accent);
        }
        if resp.on_hover_text(format!("Einstellungen · Geschwindigkeit {}", speed_label(self.speed))).clicked() {
            self.menu = !self.menu;
        }
        if self.menu {
            self.settings_menu(ui, th, gb);
        }
        right -= 34.0;
        // Lautstärkeregler (bei wenig Platz nur das Lautsprechersymbol)
        let vol_w = if slider { 64.0 } else { 0.0 };
        let vr = Rect::from_min_max(pos2(right - vol_w, mid - 8.0), pos2(right - 4.0, mid + 8.0));
        if slider {
            let resp = ui.interact(vr, ui.id().with("pl_vol"), Sense::click_and_drag());
            if let Some(pt) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
                self.volume = ((pt.x - vr.left()) / vr.width()).clamp(0.0, 1.0);
                self.muted = false;
                self.apply_volume();
            }
            let shown = if self.muted { 0.0 } else { self.volume };
            let line = Rect::from_min_max(pos2(vr.left(), mid - 2.0), pos2(vr.right(), mid + 2.0));
            p.rect_filled(line, 2.0, track);
            p.rect_filled(Rect::from_min_max(line.min, pos2(line.left() + line.width() * shown, line.bottom())), 2.0, th.accent);
            p.circle_filled(pos2(line.left() + line.width() * shown, mid), if resp.hovered() || resp.dragged() { 6.0 } else { 4.5 }, fg);
            resp.on_hover_text(format!("Lautstärke {} % (↑/↓)", (self.volume * 100.0).round()));
        }
        let sb = Rect::from_center_size(pos2(vr.left() - if slider { 16.0 } else { 13.0 }, mid), vec2(26.0, 26.0));
        let resp = ui.interact(sb, ui.id().with("pl_mute"), Sense::click());
        if resp.hovered() {
            p.rect_filled(sb, 6.0, if dark { Color32::from_white_alpha(40) } else { th.hover });
            // ohne Regler: Mausrad über dem Symbol ändert die Lautstärke
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if !slider && scroll != 0.0 {
                self.volume = (self.volume + scroll.signum() * 0.05).clamp(0.0, 1.0);
                self.muted = false;
                self.apply_volume();
            }
        }
        speaker_icon(&p, Rect::from_center_size(sb.center(), vec2(15.0, 15.0)), fg, if self.muted { 0.0 } else { self.volume });
        let tip = if slider {
            if self.muted { "Ton an (M)".to_string() } else { "Stumm (M)".to_string() }
        } else {
            format!("Lautstärke {} % – Mausrad ändert, Klick = {} (M)", (self.volume * 100.0).round(), if self.muted { "Ton an" } else { "stumm" })
        };
        if resp.on_hover_text(tip).clicked() {
            self.muted = !self.muted;
            self.apply_volume();
        }
        right = sb.left() - 8.0;
        // Zeitleiste – in der Leiste, oder bei sehr wenig Platz als schmale Linie über die ganze Breite am oberen Rand
        let inline = right - x > 30.0;
        if self.info.duration > 0.0 {
            let tr = if inline { Rect::from_min_max(pos2(x, mid - 9.0), pos2(right, mid + 9.0)) } else { Rect::from_min_max(pos2(r.left() + 6.0, r.top() - 1.0), pos2(r.right() - 6.0, r.top() + 5.0)) };
            let mid = tr.center().y;
            let resp = ui.interact(tr, ui.id().with("pl_seek"), Sense::click_and_drag());
            let dur = self.info.duration;
            let at = move |pt: Pos2| ((pt.x - tr.left()) / tr.width()).clamp(0.0, 1.0) as f64 * dur;
            if resp.dragged() {
                if let Some(pt) = resp.interact_pointer_pos() {
                    self.drag = Some(at(pt));
                }
            }
            if resp.drag_stopped() || resp.clicked() {
                let target = self.drag.take().or_else(|| resp.interact_pointer_pos().map(at));
                if let Some(t) = target {
                    self.seek(t);
                }
            }
            let frac = (cur / self.info.duration).clamp(0.0, 1.0) as f32;
            let h = if resp.hovered() || resp.dragged() { 6.0 } else { 4.0 };
            let line = Rect::from_min_max(pos2(tr.left(), mid - h / 2.0), pos2(tr.right(), mid + h / 2.0));
            p.rect_filled(line, h / 2.0, track);
            p.rect_filled(Rect::from_min_max(line.min, pos2(line.left() + line.width() * frac, line.bottom())), h / 2.0, th.accent);
            if resp.hovered() || resp.dragged() {
                p.circle_filled(pos2(line.left() + line.width() * frac, mid), 7.0, fg);
                if let Some(hp) = resp.hover_pos() {
                    let t = at(hp);
                    egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), ui.id().with("pl_tip"), |ui| {
                        ui.label(hms(t));
                    });
                }
            }
        }
        if let Some(e) = &self.err {
            p.text(pos2(r.left(), r.bottom() + 2.0), egui::Align2::LEFT_TOP, e, FontId::proportional(11.5), th.danger);
        }
    }

    /// Aufklappmenü über dem Zahnrad: Qualität (nur Video) und Wiedergabegeschwindigkeit.
    fn settings_menu(&mut self, ui: &egui::Ui, th: &Theme, anchor: Rect) {
        let qualities = if self.info.has_video { self.qualities() } else { Vec::new() };
        let (mut speed, mut quality) = (self.speed, self.quality);
        let (mut drag, mut custom) = (self.speed_drag, self.custom.take());
        let has_video = self.info.has_video;
        let area = egui::Area::new(ui.id().with("pl_menu"))
            .order(egui::Order::Tooltip)
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(pos2(anchor.right(), anchor.top() - 6.0))
            .constrain(true)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(214.0);
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    if has_video {
                        ui.label(egui::RichText::new("Qualität").size(12.0).strong());
                        ui.horizontal_wrapped(|ui| {
                            ui.selectable_value(&mut quality, 0, "Auto");
                            for q in &qualities {
                                ui.selectable_value(&mut quality, *q, format!("{q}p"));
                            }
                        });
                        ui.add_space(6.0);
                    }
                    speed_ui(ui, th, &mut speed, &mut drag, &mut custom);
                });
            });
        self.speed_drag = drag;
        self.custom = custom;
        self.set_quality(quality);
        self.set_speed(speed);
        // Klick daneben oder Esc schließt das Menü
        let (pressed, at, esc) = ui.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos(), i.key_pressed(egui::Key::Escape)));
        if esc || (pressed && at.is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p))) {
            self.menu = false;
            self.speed_drag = None;
            self.custom = None;
        }
    }

    /// Wellenform in `r`: Spitzenpegel (hell) und Effektivpegel (kräftig) aus dem echten Signal; der gespielte
    /// Teil ist bis zur genauen Abspielstelle eingefärbt. Klicken/Ziehen springt.
    pub fn waveform(&mut self, ui: &mut egui::Ui, th: &Theme, r: Rect, wave: &Waveform) {
        self.last_frame = ui.ctx().cumulative_pass_nr();
        let dur = self.info.duration;
        let inner = r.shrink2(vec2(10.0, 8.0));
        let resp = ui.interact(r, ui.id().with("pl_wave"), Sense::click_and_drag());
        let at = move |pt: Pos2| ((pt.x - inner.left()) / inner.width()).clamp(0.0, 1.0) as f64 * dur;
        if self.hold_boost(ui, &resp) {
            // Gedrückthalten = 2×: nicht springen
            self.drag = None;
        } else if dur > 0.0 {
            if resp.dragged() {
                if let Some(pt) = resp.interact_pointer_pos() {
                    self.drag = Some(at(pt));
                }
            }
            if resp.drag_stopped() || resp.clicked() {
                if let Some(t) = self.drag.take().or_else(|| resp.interact_pointer_pos().map(at)) {
                    self.seek(t);
                }
            }
        }
        let p = ui.painter_at(r);
        p.rect_filled(r, 8.0, th.field);
        let (mid, half) = (inner.center().y, inner.height() / 2.0);
        let d = wave.data.lock().unwrap_or_else(|e| e.into_inner());
        let total = if dur > 0.0 { (dur / WAVE_BLOCK).ceil() as usize } else { d.blocks.len() }.max(1);
        // eine Säule je 3 px; jede fasst die Abschnitte ihres Zeitraums zusammen (Maximum)
        let n = ((inner.width() / 3.0).floor() as usize).max(1);
        let col = inner.width() / n as f32;
        let bw = (col * 0.68).max(1.0);
        let scale = if d.max_peak > 0.0 { 1.0 / d.max_peak } else { 0.0 };
        let bars: Vec<(f32, Option<(f32, f32)>)> = (0..n)
            .map(|i| {
                let x = inner.left() + i as f32 * col + (col - bw) / 2.0;
                let a = i * total / n;
                let b = ((i + 1) * total / n).max(a + 1).min(d.blocks.len());
                let v = (a < b).then(|| d.blocks[a..b].iter().fold((0f32, 0f32), |m, v| (m.0.max(v[0]), m.1.max(v[1]))));
                (x, v.map(|(pk, rms)| (pk * scale, rms * scale)))
            })
            .collect();
        let done = d.done;
        drop(d);
        let draw = |p: &egui::Painter, peak_c: Color32, rms_c: Color32| {
            for (x, v) in &bars {
                match v {
                    Some((pk, rms)) => {
                        let hp = (pk * half).max(1.0);
                        let hr = (rms * half).max(1.0).min(hp);
                        p.rect_filled(Rect::from_min_max(pos2(*x, mid - hp), pos2(x + bw, mid + hp)), bw / 2.0, peak_c);
                        p.rect_filled(Rect::from_min_max(pos2(*x, mid - hr), pos2(x + bw, mid + hr)), bw / 2.0, rms_c);
                    }
                    // noch nicht berechnet
                    None => {
                        p.rect_filled(Rect::from_min_max(pos2(*x, mid - 0.75), pos2(x + bw, mid + 0.75)), 0.0, th.line);
                    }
                }
            }
        };
        draw(&p, th.sub.gamma_multiply(0.3), th.sub.gamma_multiply(0.75));
        if dur > 0.0 {
            let cur = self.drag.unwrap_or_else(|| self.pos());
            let px = inner.left() + inner.width() * (cur / dur).clamp(0.0, 1.0) as f32;
            // gespielter Teil: dieselben Säulen, auf die Abspielstelle beschnitten
            let played = p.with_clip_rect(Rect::from_min_max(r.min, pos2(px, r.bottom())).intersect(p.clip_rect()));
            draw(&played, th.accent.gamma_multiply(0.4), th.accent);
            if let Some(hp) = resp.hover_pos().filter(|_| !resp.dragged()) {
                p.line_segment([pos2(hp.x, inner.top()), pos2(hp.x, inner.bottom())], Stroke::new(1.0, th.sub.gamma_multiply(0.6)));
                let t = at(hp);
                egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), ui.id().with("pl_wave_tip"), |ui| {
                    ui.label(hms(t));
                });
            }
            p.line_segment([pos2(px, r.top() + 3.0), pos2(px, r.bottom() - 3.0)], Stroke::new(2.0, th.text));
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        self.boost_badge(ui, r);
        if !done {
            ui.ctx().request_repaint_after(Duration::from_millis(120));
        }
    }

    /// Tastatur (wenn der Player bedient wird): Leertaste, Pfeile, M, F, Esc.
    pub fn keys(&mut self, ui: &egui::Ui) {
        // Eingabe im Menü (eigene Geschwindigkeit): Tasten gehören dem Textfeld
        if ui.ctx().wants_keyboard_input() {
            return;
        }
        let (space, left, rightk, up, down, m, f, esc) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::M),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if space {
            self.toggle();
        }
        if left || rightk {
            let t = self.pos() + if rightk { 5.0 } else { -5.0 };
            self.seek(t);
        }
        if up || down {
            self.volume = (self.volume + if up { 0.1 } else { -0.1 }).clamp(0.0, 1.0);
            self.muted = false;
            self.apply_volume();
        }
        if m {
            self.muted = !self.muted;
            self.apply_volume();
        }
        if ui.input(|i| i.key_pressed(egui::Key::C)) && self.has_subs() {
            self.subs_on = !self.subs_on;
        }
        if f && self.info.has_video && !self.pip {
            self.fullscreen = !self.fullscreen;
        }
        if esc && self.fullscreen && !self.menu {
            self.fullscreen = false;
        }
    }

    /// Vollbild über das ganze Fenster (und Fenster im Vollbildmodus). Jedes Frame aufrufen.
    pub fn fullscreen_ui(&mut self, ctx: &egui::Context, th: &Theme) {
        if self.fullscreen != self.fs_sent {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            self.fs_sent = self.fullscreen;
        }
        if !self.fullscreen {
            return;
        }
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("pl_fullscreen")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.painter().rect_filled(screen, 0.0, Color32::BLACK);
            self.video(ui, screen);
            // Bedienleiste: immer sichtbar, solange pausiert ist; während der Wiedergabe bei jeder Mausbewegung
            // (egal wo) und erst nach 2,5 s ohne Bewegung ausgeblendet (offenes Menü, gedrückte Maustaste oder
            // Maus über der Leiste halten sie)
            let (moved, down, at) = ui.input(|i| (i.pointer.delta() != egui::Vec2::ZERO, i.pointer.any_down(), i.pointer.hover_pos()));
            if moved || down {
                self.last_move = Instant::now();
            }
            let over_bar = at.is_some_and(|p| p.y >= screen.bottom() - 90.0);
            let recent = self.last_move.elapsed() < Duration::from_millis(2500);
            let show = self.paused || self.ended || self.menu || self.speed_drag.is_some() || recent || down || over_bar;
            if show {
                let bar = Rect::from_min_max(pos2(screen.left(), screen.bottom() - 64.0), screen.max);
                ui.painter().rect_filled(bar, 0.0, Color32::from_black_alpha(170));
                let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.painter().text(pos2(bar.left() + 20.0, bar.top() - 10.0), egui::Align2::LEFT_BOTTOM, name, FontId::proportional(14.0), Color32::from_white_alpha(200));
                self.controls(ui, th, bar.shrink2(vec2(20.0, 14.0)), true);
                // nach Ablauf der Zeit ausblenden, auch ohne weiteres Ereignis
                ui.ctx().request_repaint_after(Duration::from_millis(300));
            } else if !recent {
                ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(300));
            }
            self.keys(ui);
        });
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if self.fs_sent {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
    }
}

/// Geschwindigkeit im Einstellungsmenü: Überschrift mit aktuellem Wert und dezentem Stift für eine eigene
/// Eingabe, darunter ein Regler 0–2× mit Rastpunkten. Gezogen wird ohne Neustart der Dekoder; übernommen wird
/// beim Loslassen. Werte über 2× (eigene Eingabe) stellt der Regler am rechten Rand hohl dar.
fn speed_ui(ui: &mut egui::Ui, th: &Theme, speed: &mut f32, drag: &mut Option<f32>, custom: &mut Option<String>) {
    let shown = drag.unwrap_or(*speed);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Wiedergabegeschwindigkeit").size(12.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (b, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
            if resp.hovered() || custom.is_some() {
                ui.painter().rect_filled(b, 4.0, th.hover);
            }
            pencil_icon(ui.painter(), b.shrink(5.0), if custom.is_some() { th.accent } else { th.sub });
            if resp.on_hover_text("Eigene Geschwindigkeit eingeben").clicked() {
                *custom = match custom {
                    Some(_) => None,
                    None => Some(String::new()),
                };
            }
            ui.label(egui::RichText::new(speed_label(shown)).size(12.5).strong().color(th.accent));
        });
    });
    // Regler
    let (sr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click_and_drag());
    let track = Rect::from_min_max(pos2(sr.left() + 8.0, sr.top() + 6.0), pos2(sr.right() - 8.0, sr.top() + 18.0));
    let to_x = |v: f32| track.left() + track.width() * (v / 2.0).clamp(0.0, 1.0);
    if let Some(pt) = resp.interact_pointer_pos().filter(|_| resp.is_pointer_button_down_on() || resp.dragged() || resp.clicked()) {
        // nahe an einem Rastpunkt: einrasten, sonst in 0,05er-Schritten
        let v = match SPEEDS.iter().find(|s| (to_x(**s) - pt.x).abs() <= 7.0) {
            Some(s) => *s,
            None => (((pt.x - track.left()) / track.width()).clamp(0.0, 1.0) * 40.0).round() / 20.0,
        };
        *drag = Some(v.max(MIN_SPEED));
    }
    if resp.drag_stopped() || resp.clicked() {
        if let Some(v) = drag.take() {
            *speed = v;
        }
    }
    let p = ui.painter();
    let mid = track.center().y;
    let line = Rect::from_min_max(pos2(track.left(), mid - 2.0), pos2(track.right(), mid + 2.0));
    p.rect_filled(line, 2.0, th.line);
    let kx = to_x(shown);
    p.rect_filled(Rect::from_min_max(line.min, pos2(kx, line.bottom())), 2.0, th.accent);
    for s in SPEEDS.iter().copied().chain([0.0]) {
        let x = to_x(s);
        let major = s == 0.0 || s == 1.0 || s == 2.0;
        p.line_segment([pos2(x, mid + 5.0), pos2(x, mid + if major { 10.0 } else { 8.0 })], Stroke::new(1.0, th.sub.gamma_multiply(0.7)));
        if major {
            p.text(pos2(x, sr.bottom()), egui::Align2::CENTER_BOTTOM, speed_label(s), FontId::proportional(9.5), th.sub);
        }
    }
    let big = resp.hovered() || resp.dragged();
    if shown > 2.0 {
        // eigener Wert außerhalb des Reglers: hohler Knopf am Ende
        p.circle(pos2(kx, mid), 6.0, th.panel, Stroke::new(2.0, th.accent));
    } else {
        p.circle_filled(pos2(kx, mid), if big { 7.0 } else { 6.0 }, th.accent);
    }
    // eigene Eingabe
    if let Some(text) = custom {
        let mut close = false;
        ui.horizontal(|ui| {
            let te = ui.add(egui::TextEdit::singleline(text).desired_width(64.0).hint_text("z. B. 3"));
            if !te.has_focus() && !te.lost_focus() && text.is_empty() {
                te.request_focus();
            }
            ui.label("×");
            let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ok = ui.add_enabled(parse_speed(text).is_some(), egui::Button::new("Übernehmen").small()).clicked();
            if enter || ok {
                if let Some(v) = parse_speed(text) {
                    *speed = v;
                    close = true;
                }
            }
        });
        if !text.trim().is_empty() && parse_speed(text).is_none() {
            ui.label(egui::RichText::new("Bitte eine positive Zahl eingeben").size(11.0).color(th.danger));
        } else {
            ui.label(egui::RichText::new(format!("{}× bis {}×, auch über 2×", speed_label(MIN_CUSTOM).trim_end_matches('×'), MAX_SPEED)).size(11.0).color(th.sub));
        }
        if close {
            *custom = None;
        }
    }
}

// ───────────────────────────── Symbole ─────────────────────────────

fn pencil_icon(p: &egui::Painter, r: Rect, c: Color32) {
    // schräger Stift: Schaft und Spitze unten links
    let (a, b) = (pos2(r.left() + 0.5, r.bottom() - 0.5), pos2(r.right() - 0.5, r.top() + 0.5));
    let d = (b - a).normalized();
    let tip = a + d * 3.0;
    p.line_segment([tip, b], Stroke::new(2.4, c));
    p.add(Shape::convex_polygon(vec![a, tip + d.rot90() * 1.3, tip - d.rot90() * 1.3], c, Stroke::NONE));
}

fn pip_icon(p: &egui::Painter, r: Rect, c: Color32, active: bool) {
    p.rect_stroke(r, 2.0, Stroke::new(1.4, c));
    let small = Rect::from_min_max(pos2(r.center().x + 0.5, r.center().y + 0.5), r.max - vec2(2.5, 2.5));
    if active {
        // im Bild-in-Bild: kleines Fenster oben links (= zurück in die große Ansicht)
        p.rect_filled(Rect::from_min_max(r.min + vec2(2.5, 2.5), pos2(r.center().x - 0.5, r.center().y - 0.5)), 1.0, c);
    } else {
        p.rect_filled(small, 1.0, c);
    }
}

fn play_icon(p: &egui::Painter, r: Rect, c: Color32) {
    p.add(Shape::convex_polygon(vec![pos2(r.left() + r.width() * 0.12, r.top()), pos2(r.right(), r.center().y), pos2(r.left() + r.width() * 0.12, r.bottom())], c, Stroke::NONE));
}

fn pause_icon(p: &egui::Painter, r: Rect, c: Color32) {
    let w = r.width() * 0.3;
    p.rect_filled(Rect::from_min_size(pos2(r.left() + r.width() * 0.1, r.top()), vec2(w, r.height())), 1.5, c);
    p.rect_filled(Rect::from_min_size(pos2(r.right() - r.width() * 0.1 - w, r.top()), vec2(w, r.height())), 1.5, c);
}

fn speaker_icon(p: &egui::Painter, r: Rect, c: Color32, level: f32) {
    let s = r.width();
    let pt = |x: f32, y: f32| pos2(r.left() + x * s, r.top() + y * s);
    p.add(Shape::convex_polygon(vec![pt(0.05, 0.36), pt(0.28, 0.36), pt(0.55, 0.1), pt(0.55, 0.9), pt(0.28, 0.64), pt(0.05, 0.64)], c, Stroke::NONE));
    let st = Stroke::new(1.4, c);
    if level <= 0.001 {
        p.line_segment([pt(0.7, 0.35), pt(0.98, 0.65)], st);
        p.line_segment([pt(0.7, 0.65), pt(0.98, 0.35)], st);
        return;
    }
    let arc = |rad: f32| {
        let pts: Vec<Pos2> = (0..=10).map(|i| {
            let a = -0.85 + 1.7 * i as f32 / 10.0;
            pos2(r.left() + 0.5 * s + rad * s * a.cos(), r.center().y + rad * s * a.sin())
        }).collect();
        p.add(Shape::line(pts, st));
    };
    arc(0.22);
    if level > 0.5 {
        arc(0.42);
    }
}

fn gear_icon(p: &egui::Painter, r: Rect, c: Color32) {
    let (cx, rad) = (r.center(), r.width() / 2.0);
    // Zahnkranz mit 8 Zähnen, innen ein Loch
    let pts: Vec<Pos2> = (0..64)
        .map(|i| {
            let a = i as f32 / 64.0 * std::f32::consts::TAU;
            let tooth = if (i / 4) % 2 == 0 { 1.0 } else { 0.0 };
            cx + vec2(a.cos(), a.sin()) * rad * (0.72 + 0.28 * tooth)
        })
        .collect();
    p.add(Shape::closed_line(pts, Stroke::new(1.4, c)));
    p.circle_stroke(cx, rad * 0.28, Stroke::new(1.4, c));
}

fn fullscreen_icon(p: &egui::Painter, r: Rect, c: Color32, exit: bool) {
    let st = Stroke::new(1.6, c);
    let k = r.width() * 0.32;
    for (corner, dx, dy) in [(r.left_top(), 1.0, 1.0), (r.right_top(), -1.0, 1.0), (r.left_bottom(), 1.0, -1.0), (r.right_bottom(), -1.0, -1.0)] {
        // Vollbild verlassen: Ecken zeigen nach innen
        let o = if exit { corner + vec2(dx * k, dy * k) } else { corner };
        let (sx, sy) = if exit { (-dx, -dy) } else { (dx, dy) };
        p.add(Shape::line(vec![o + vec2(sx * k, 0.0), o, o + vec2(0.0, sy * k)], st));
    }
}
