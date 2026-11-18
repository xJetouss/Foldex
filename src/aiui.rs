//! KI-Chats:
//! - **Tabs mit fester KI**: Claude Code, Codex, Gemini CLI … (ohne Terminalfenster, Schritte erscheinen im Chat)
//!   und jedes eigene Modell (Anthropic-API, LM Studio, OpenAI-kompatibel) mit den Explorer-Werkzeugen aus ai.rs.
//! - **Explorer Assistent / Explorer Agent**: eine Seitenleiste, die beim Tabwechsel sichtbar bleibt. Sie nutzt ein eigenes
//!   Modell und kann zusätzlich Tabs öffnen, wechseln, schließen und Explorer-Befehle ausführen.
//!
//! Modus „Assistent“: nur ansehen und antworten. Modus „Agent“: darf Dateien bearbeiten, erstellen, verschieben,
//! Befehle im Hintergrund ausführen (Shell, Git) … Modus „Chat“: ein normaler KI-Chat – keine Werkzeuge, kein Zugriff
//! auf Explorer, Ordner oder Dateien; die KI sieht nur, was man schreibt.
//! Bei eigenen Modellen fragt jede Änderung nach „Übernehmen“, außer die Kategorie ist in den Einstellungen
//! freigegeben. Agenten bekommen im Agent-Modus Schreibrechte im aktuellen Ordner (Befehle nur, wenn erlaubt).
//!
//! Jedes Gespräch wird unter config/foldex/chats/<id>.json gespeichert (Verlauf-Knopf im Kopf des Chats).

use crate::agents::{self, Access, Ev};
use crate::ai::{self, Call, Cfg, Effect, Kind, ModelProfile, TabInfo, Turn};
use crate::ops::Op;
use crate::themes::Theme;
use crate::ui::{self, Ic};
use eframe::egui::{self, pos2, vec2, Color32, FontId, Margin, Rect, Rounding, Sense, Stroke};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

/// Was der Chat vom Explorer braucht (jedes Frame neu übergeben).
pub struct Env<'a> {
    pub profiles: &'a [ModelProfile],
    pub keys: &'a HashMap<String, Option<String>>,
    /// eingeschaltete, installierte Agenten: (Schlüssel, Name, Programm)
    pub agents: &'a [(String, String, String)],
    pub agent_model: &'a BTreeMap<String, String>,
    pub auto: &'a BTreeSet<String>,
    pub cli_full: bool,
    pub cwd: &'a Path,
    pub selection: &'a [String],
    /// offene Tabs (für die Explorer-KI)
    pub tabs: &'a [TabInfo],
}

/// Was der Explorer nach einer Aktion tun soll.
/// Stil-Vorlagen im Bild-Modus: (Name, Zusatz zum Prompt, negativer Prompt)
pub const STYLES: &[(&str, &str, &str)] = &[
    ("Foto", "professional photograph, natural light, sharp focus, 35mm, high detail, realistic skin texture", "illustration, painting, drawing, cartoon, anime, 3d render, cgi, oversaturated, blurry, deformed, lowres"),
    ("Anime", "anime style, cel shading, clean line art, vibrant colors, studio anime key visual", "photo, realistic, 3d render, photorealistic, blurry, deformed hands, lowres, watermark"),
    ("Ölgemälde", "oil painting on canvas, visible brush strokes, rich texture, classical fine art, warm palette", "photo, digital art, 3d render, flat colors, text, watermark, blurry"),
    ("Aquarell", "watercolor painting, soft washes, paper texture, delicate pigments, loose brushwork", "photo, 3d render, harsh lines, dark, oversaturated, text, watermark"),
    ("Logo", "minimal flat vector logo, simple geometric shapes, centered, plain white background, bold, scalable icon", "photo, realistic, gradient mess, cluttered background, many details, text artifacts, blurry, 3d"),
    ("Pixel-Art", "pixel art, 16-bit retro game sprite, limited color palette, crisp pixels, no anti-aliasing", "photo, realistic, smooth gradients, blurry, high resolution detail, 3d render"),
    ("3D", "3d render, octane, soft studio lighting, subsurface scattering, highly detailed, depth of field", "photo, flat, 2d, sketch, drawing, lowres, noisy"),
    ("Comic", "comic book style, bold ink outlines, halftone shading, dynamic composition, vivid flat colors", "photo, realistic, 3d render, blurry, muted colors"),
    ("Skizze", "pencil sketch, graphite on paper, cross-hatching, hand drawn, monochrome", "color, photo, 3d render, digital painting, blurry"),
    ("Kino", "cinematic film still, anamorphic lens, dramatic lighting, color graded, shallow depth of field", "cartoon, anime, illustration, flat lighting, lowres, overexposed"),
    ("Fantasy", "epic fantasy concept art, magical atmosphere, intricate details, volumetric light, matte painting", "photo, mundane, lowres, blurry, text, watermark"),
    ("Isometrisch", "isometric illustration, clean vector style, soft pastel colors, miniature diorama", "photo, perspective distortion, realistic, blurry, cluttered"),
];

pub enum AiOut {
    /// Änderung in den Rückgängig-Verlauf aufnehmen und Ansichten auffrischen
    Done(Op),
    /// Agent ist fertig – Dateien können sich geändert haben
    Changed,
    Effect(Effect),
    OpenSettings,
    KeySaved(String),
    SetAgentModel(String, String),
    SetProfileModel(String, String),
    /// Explorer-Seitenleiste schließen
    Close,
    /// Datei als Vorschau in einem neuen Tab öffnen
    Preview(PathBuf),
    /// Bild im Bildeditor öffnen
    Edit(PathBuf),
}

#[derive(Clone, Copy, PartialEq)]
enum CallState {
    Ask,
    Running,
    Done,
    Failed,
    Rejected,
}

struct CallView {
    call: Call,
    title: String,
    lines: Vec<String>,
    needs_ok: bool,
    auto: bool,
    state: CallState,
    result: Option<(String, bool)>,
    /// Bild zum Ergebnis (Bildschirmfoto, JPEG Base64)
    image: Option<String>,
    /// wartet auf die Oberfläche (Bildschirmfoto, Terminaltext) seit …
    waiting: Option<std::time::Instant>,
    /// erzeugte Bilder (generate_image) – als Vorschau unter der Karte
    files: Vec<PathBuf>,
}

enum Item {
    User(String),
    Assistant(String),
    /// Werkzeugaufrufe eines eigenen Modells (mit Bestätigung)
    Calls(Vec<CallView>),
    /// Schritt eines Agenten: Werkzeug, Detail, Ergebnis
    Step(String, String, Option<(String, bool)>),
    Note(String, bool),
    /// Trennmarke nach abgeschlossener Werkzeugrunde
    Mark,
    /// erzeugte Bilder (Modus „Bild“)
    Images(Vec<PathBuf>),
    /// an eine Nachricht angehängte Dateien (Bilder als Vorschau)
    Ref(Vec<PathBuf>),
}

enum Backend {
    Agent { key: String, name: String, program: String },
    Model(ModelProfile),
}

/// Eintrag der Verlaufsliste.
#[derive(Clone)]
pub struct ChatMeta {
    pub id: String,
    pub title: String,
    pub backend: String,
    pub editor: bool,
    pub updated: u64,
    pub messages: usize,
    pub agent_mode: bool,
    pub plain: bool,
    pub image: bool,
    /// Modell, mit dem das Gespräch lief (Profil bzw. Bildmodell)
    pub model_key: String,
}

impl ChatMeta {
    fn mode(&self) -> Mode {
        if self.image {
            Mode::Image
        } else if self.plain {
            Mode::Chat
        } else if self.agent_mode {
            Mode::Agent
        } else {
            Mode::Assistant
        }
    }
}

pub struct ChatView {
    /// „agent:<schlüssel>“ oder „model:<id>“
    pub backend: String,
    pub agent_mode: bool,
    /// Reiner Chat (ohne Explorer und Dateien); schließt `agent_mode` aus
    pub plain: bool,
    pub cwd: PathBuf,
    /// Explorer-Seitenleiste (Tabs/Befehle als Werkzeuge, nur eigene Modelle)
    pub editor: bool,
    /// gespeichertes Gespräch
    pub chat_id: String,
    title: String,
    title_custom: bool,
    created: u64,
    saved: std::cell::Cell<(usize, usize)>,
    label: String,
    items: Vec<Item>,
    input: String,
    // eigene Modelle
    history: Vec<Value>,
    history_kind: Option<Kind>,
    busy: Option<Receiver<Result<Turn, String>>>,
    exec: Option<Receiver<Vec<(usize, ai::Outcome)>>>,
    key_input: String,
    models: Option<Result<Vec<String>, String>>,
    models_rx: Option<Receiver<Result<Vec<String>, String>>>,
    /// Modell für dieses Gespräch (leer = Vorgabe des Profils bzw. Agenten)
    model: String,
    /// Denkaufwand (Claude-API)
    effort: String,
    effects: Vec<Effect>,
    // Agenten
    session: Option<String>,
    cli_rx: Option<Receiver<Ev>>,
    cli: Option<agents::Handle>,
    cost: f64,
    model_edit: String,
    // Verlauf
    hist: Vec<ChatMeta>,
    hist_query: String,
    /// Verlauf: angezeigter Modus und „nur dieses Modell“
    hist_mode: Option<Mode>,
    hist_model_only: bool,
    hist_rename: Option<(String, String)>,
    // Modus „Bild“: lokales Bildmodell statt Sprachmodell
    pub image: bool,
    img_model: Option<crate::localllm::ImgChoice>,
    /// gewählter Eintrag in der Kurzbefehl-Liste
    skill_sel: usize,
    /// Stil-Vorlage im Bild-Modus (Index in `STYLES`)
    img_style: Option<usize>,
    img_job: Option<(crate::localai::Progress, Receiver<Result<Vec<PathBuf>, String>>)>,
    thumbs: std::cell::RefCell<HashMap<PathBuf, Option<egui::TextureHandle>>>,
    /// Spracheingabe neben „Senden“
    mic: crate::stt::Mic,
    /// Anhänge für die nächste Nachricht (Bild-Modus: Vorlage zum Bearbeiten)
    attach: Vec<PathBuf>,
    /// Bearbeitungsmodell für Vorlagen (id aus localllm::EDIT)
    edit_model: Option<&'static str>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn chats_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("chats")
}

fn chat_file(id: &str) -> PathBuf {
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    chats_dir().join(format!("{safe}.json"))
}

/// Alle gespeicherten Gespräche, neueste zuerst.
pub fn list_chats() -> Vec<ChatMeta> {
    let mut v: Vec<ChatMeta> = fs::read_dir(chats_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice::<Value>(&fs::read(e.path()).ok()?).ok())
        .map(|j| ChatMeta {
            id: j["id"].as_str().unwrap_or_default().to_string(),
            title: j["title"].as_str().unwrap_or("Gespräch").to_string(),
            backend: j["backend"].as_str().unwrap_or_default().to_string(),
            editor: j["editor"].as_bool().unwrap_or(false),
            updated: j["updated"].as_u64().unwrap_or(0),
            messages: j["items"].as_array().map_or(0, |a| a.iter().filter(|i| i["t"] == "user").count()),
            agent_mode: j["agent_mode"].as_bool().unwrap_or(false),
            plain: j["plain"].as_bool().unwrap_or(false),
            image: j["image"].as_bool().unwrap_or(false),
            model_key: if j["image"].as_bool().unwrap_or(false) { j["img_model"].as_str().unwrap_or_default().to_string() } else { j["backend"].as_str().unwrap_or_default().to_string() },
        })
        .filter(|m| !m.id.is_empty())
        .collect();
    v.sort_by(|a, b| b.updated.cmp(&a.updated));
    v
}

pub fn delete_chat(id: &str) {
    let _ = fs::remove_file(chat_file(id));
}

fn rename_chat(id: &str, title: &str) {
    let p = chat_file(id);
    if let Some(mut j) = fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
        j["title"] = json!(title.trim());
        j["title_custom"] = json!(true);
        let _ = crate::themes::write_atomic(&p, j.to_string().as_bytes());
    }
}

fn fmt_time(secs: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs as i64, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%d.%m.%Y %H:%M").to_string())
        .unwrap_or_default()
}

/// Verlauf kürzen, bis er mit einer vollständigen Antwort endet (keine offenen Werkzeugaufrufe).
fn sanitize(h: &mut Vec<Value>) {
    while let Some(last) = h.last() {
        let assistant = last["role"] == "assistant";
        let open_calls = last["content"].as_array().is_some_and(|c| c.iter().any(|b| b["type"] == "tool_use")) || last["tool_calls"].as_array().is_some_and(|c| !c.is_empty());
        if assistant && !open_calls {
            break;
        }
        h.pop();
    }
}

impl ChatView {
    fn blank(cwd: PathBuf, backend: String, agent_mode: bool, editor: bool) -> ChatView {
        ChatView {
            backend: backend.clone(),
            agent_mode,
            plain: false,
            cwd,
            editor,
            chat_id: format!("{:x}{:04x}", now(), std::process::id() as u16 ^ (rand_u16())),
            title: String::new(),
            title_custom: false,
            created: now(),
            saved: std::cell::Cell::new((0, 0)),
            label: "KI-Chat".into(),
            items: Vec::new(),
            input: String::new(),
            history: Vec::new(),
            history_kind: None,
            busy: None,
            exec: None,
            key_input: String::new(),
            models: None,
            models_rx: None,
            model: String::new(),
            // Agenten: leer = Vorgabe des Agenten; eigene Modelle: „medium“
            effort: if backend.starts_with("agent:") { String::new() } else { "medium".into() },
            effects: Vec::new(),
            session: None,
            cli_rx: None,
            cli: None,
            cost: 0.0,
            model_edit: String::new(),
            hist: Vec::new(),
            hist_query: String::new(),
            hist_mode: None,
            hist_model_only: true,
            hist_rename: None,
            image: false,
            img_model: None,
            skill_sel: 0,
            img_style: None,
            img_job: None,
            thumbs: Default::default(),
            mic: crate::stt::Mic::default(),
            attach: Vec::new(),
            edit_model: None,
        }
    }

    /// Chat-Tab mit fester KI (Agent oder eigenes Modell).
    pub fn new(cwd: PathBuf, backend: String, agent_mode: bool) -> ChatView {
        let mut c = ChatView::blank(cwd, backend, agent_mode, false);
        c.resume_last();
        c
    }

    /// Explorer Assistent (`agent_mode` = false) bzw. Explorer Agent in der Seitenleiste.
    pub fn editor(cwd: PathBuf, backend: String, agent_mode: bool) -> ChatView {
        let mut c = ChatView::blank(cwd, backend, agent_mode, true);
        c.resume_last();
        c
    }

    fn mode(&self) -> Mode {
        if self.image {
            Mode::Image
        } else if self.plain {
            Mode::Chat
        } else if self.agent_mode {
            Mode::Agent
        } else {
            Mode::Assistant
        }
    }

    /// Modell dieses Gesprächs (für den getrennten Verlauf)
    fn model_key(&self) -> String {
        if self.image {
            self.img_model.as_ref().map(img_key).unwrap_or_default()
        } else {
            self.backend.clone()
        }
    }

    fn img_label(&self) -> String {
        let choices = crate::localllm::image_choices();
        self.img_model.as_ref().and_then(|m| choices.into_iter().find(|c| c.1 == *m)).map_or_else(|| "dieses Bildmodell".into(), |c| c.0)
    }

    /// Das zuletzt benutzte Gespräch dieses Modus (und Modells) fortsetzen – „+“ beginnt ein neues.
    /// Agent und Assistent teilen sich die Gespräche.
    fn resume_last(&mut self) {
        if !self.items.is_empty() {
            return;
        }
        let mode = self.mode();
        let key = self.model_key();
        let same = |m: Mode| if matches!(mode, Mode::Agent | Mode::Assistant) { matches!(m, Mode::Agent | Mode::Assistant) } else { m == mode };
        let last = list_chats().into_iter().find(|c| c.editor == self.editor && same(c.mode()) && (mode == Mode::Image || c.backend == self.backend) && (key.is_empty() || c.model_key == key));
        if let Some(c) = last {
            let (agent, plain, image) = (self.agent_mode, self.plain, self.image);
            self.open_saved(&c.id);
            // Modus bleibt der gewählte (z. B. Agent statt Assistent)
            self.agent_mode = agent && !plain;
            self.plain = plain;
            self.image = image;
        }
    }

    /// Gespeichertes Gespräch öffnen.
    pub fn restore(id: &str, cwd: PathBuf) -> Option<ChatView> {
        let j: Value = serde_json::from_slice(&fs::read(chat_file(id)).ok()?).ok()?;
        let mut c = ChatView::blank(cwd, j["backend"].as_str()?.to_string(), j["agent_mode"].as_bool().unwrap_or(false), j["editor"].as_bool().unwrap_or(false));
        c.plain = j["plain"].as_bool().unwrap_or(false) && !c.agent_mode;
        c.load(&j);
        Some(c)
    }

    fn load(&mut self, j: &Value) {
        self.chat_id = j["id"].as_str().unwrap_or(&self.chat_id).to_string();
        self.title = j["title"].as_str().unwrap_or_default().to_string();
        self.title_custom = j["title_custom"].as_bool().unwrap_or(false);
        self.created = j["created"].as_u64().unwrap_or_else(now);
        self.session = j["session"].as_str().map(str::to_string);
        self.cost = j["cost"].as_f64().unwrap_or(0.0);
        self.model = j["model"].as_str().unwrap_or_default().to_string();
        if let Some(e) = j["effort"].as_str() {
            self.effort = e.to_string();
        }
        self.history_kind = match j["history_kind"].as_str() {
            Some("anthropic") => Some(Kind::Anthropic),
            Some("openai") => Some(Kind::OpenAi),
            _ => None,
        };
        self.image = j["image"].as_bool().unwrap_or(false);
        if let Some(k) = j["img_model"].as_str() {
            if let Some(c) = crate::localllm::image_choices().into_iter().find(|c| img_key(&c.1) == k) {
                self.img_model = Some(c.1);
            }
        }
        self.history = j["history"].as_array().cloned().unwrap_or_default();
        sanitize(&mut self.history);
        let s = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
        self.items = j["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| {
                Some(match i["t"].as_str()? {
                    "user" => Item::User(s(i, "x")),
                    "asst" => Item::Assistant(s(i, "x")),
                    "note" => Item::Note(s(i, "x"), i["err"].as_bool().unwrap_or(false)),
                    "step" => Item::Step(s(i, "tool"), s(i, "detail"), i.get("res").and_then(Value::as_str).map(|r| (r.to_string(), i["err"].as_bool().unwrap_or(false)))),
                    "imgs" => Item::Images(i["files"].as_array().into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).collect()),
                    "ref" => Item::Ref(i["files"].as_array().into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).collect()),
                    _ => return None,
                })
            })
            .collect();
        self.saved.set((self.items.len(), self.history.len()));
    }

    fn to_json(&self) -> Value {
        let mut items = Vec::new();
        for it in &self.items {
            match it {
                Item::User(t) => items.push(json!({ "t": "user", "x": t })),
                Item::Assistant(t) => items.push(json!({ "t": "asst", "x": t })),
                Item::Note(t, e) => items.push(json!({ "t": "note", "x": t, "err": e })),
                Item::Step(tool, d, r) => items.push(json!({ "t": "step", "tool": tool, "detail": d, "res": r.as_ref().map(|r| r.0.clone()), "err": r.as_ref().is_some_and(|r| r.1) })),
                Item::Calls(v) => {
                    for c in v {
                        let res = c.result.clone().unwrap_or(("abgebrochen".into(), true));
                        items.push(json!({ "t": "step", "tool": c.title, "detail": c.lines.join("  "), "res": res.0, "err": res.1 }));
                        if !c.files.is_empty() {
                            items.push(json!({ "t": "imgs", "files": c.files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>() }));
                        }
                    }
                }
                Item::Mark => {}
                Item::Images(v) => items.push(json!({ "t": "imgs", "files": v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>() })),
                Item::Ref(v) => items.push(json!({ "t": "ref", "files": v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>() })),
            }
        }
        json!({
            "image": self.image,
            "img_model": self.img_model.as_ref().map(img_key),
            "id": self.chat_id,
            "title": self.auto_title(),
            "title_custom": self.title_custom,
            "backend": self.backend,
            "editor": self.editor,
            "agent_mode": self.agent_mode,
            "plain": self.plain,
            "created": self.created,
            "updated": now(),
            "session": self.session,
            "cost": self.cost,
            "model": self.model,
            "effort": self.effort,
            "history_kind": match self.history_kind { Some(Kind::Anthropic) => json!("anthropic"), Some(Kind::OpenAi) => json!("openai"), None => Value::Null },
            "history": self.history,
            "items": items,
        })
    }

    fn auto_title(&self) -> String {
        if self.title_custom && !self.title.is_empty() {
            return self.title.clone();
        }
        let first = self.items.iter().find_map(|i| if let Item::User(t) = i { Some(t.as_str()) } else { None }).unwrap_or("Gespräch");
        let one: String = first.split_whitespace().collect::<Vec<_>>().join(" ");
        if one.chars().count() > 60 {
            format!("{} …", one.chars().take(58).collect::<String>())
        } else {
            one
        }
    }

    /// Gespräch speichern (nur wenn es Nachrichten hat und sich etwas geändert hat).
    pub fn persist(&self) {
        if self.items.iter().any(|i| matches!(i, Item::User(_))) && self.saved.get() != (self.items.len(), self.history.len()) {
            let mut j = self.to_json();
            if let Some(h) = j["history"].as_array_mut() {
                sanitize(h);
            }
            let _ = crate::themes::write_atomic(&chat_file(&self.chat_id), j.to_string().as_bytes());
            self.saved.set((self.items.len(), self.history.len()));
        }
    }

    pub fn title(&self) -> String {
        self.label.clone()
    }

    fn running(&self) -> bool {
        self.busy.is_some() || self.cli_rx.is_some() || self.exec.is_some() || self.img_job.is_some()
    }

    fn reset(&mut self) {
        self.persist();
        if let Some(h) = &self.cli {
            h.cancel();
        }
        let keep = (self.backend.clone(), self.agent_mode, self.editor, self.model.clone(), self.effort.clone(), self.plain);
        let img = (self.image, self.img_model.take());
        *self = ChatView::blank(self.cwd.clone(), keep.0, keep.1, keep.2);
        self.model = keep.3;
        self.effort = keep.4;
        self.plain = keep.5;
        self.image = img.0;
        self.img_model = img.1;
    }

    /// Dateien anhängen (aus der Dateiliste oder dem Windows-Explorer gezogen)
    /// nur für Bildschirmfotos: Eingabe und Stil-Vorlage setzen
    pub fn dev_set(&mut self, input: Option<String>, style: Option<usize>) {
        if let Some(i) = input {
            self.input = i;
        }
        if style.is_some() {
            self.img_style = style;
        }
    }

    pub fn attach(&mut self, paths: Vec<PathBuf>) {
        for p in paths.into_iter().filter(|p| p.is_file()) {
            if !self.attach.contains(&p) {
                self.attach.push(p);
            }
        }
        if self.image {
            // Vorlage: nur ein Bild
            self.attach.retain(|p| is_image(p));
            self.attach.truncate(1);
        }
    }

    /// in den Modus „Bild“ wechseln und dort das letzte Gespräch fortsetzen
    pub fn switch_to_image(&mut self) {
        self.set_image(true);
        self.resume_last();
    }

    /// Modus „Bild“ ein/aus – ein Gespräch passt nur zu einem der beiden Modi
    pub fn set_image(&mut self, on: bool) {
        if on != self.image && !self.items.is_empty() {
            self.reset();
        }
        self.image = on;
    }

    /// Modus wechseln. In den reinen Chat bzw. aus ihm heraus beginnt ein neues Gespräch – ein Verlauf mit
    /// Werkzeugaufrufen passt nicht zu einem Chat ohne Werkzeuge.
    pub fn set_mode(&mut self, agent: bool, plain: bool) {
        if plain != self.plain && !self.items.is_empty() {
            self.reset();
        }
        self.agent_mode = agent && !plain;
        self.plain = plain;
    }

    /// Gespeichertes Gespräch in diesem Chat öffnen.
    fn open_saved(&mut self, id: &str) {
        let Some(j) = fs::read(chat_file(id)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return };
        self.reset();
        if let Some(b) = j["backend"].as_str() {
            self.backend = b.to_string();
        }
        self.agent_mode = j["agent_mode"].as_bool().unwrap_or(self.agent_mode);
        self.plain = j["plain"].as_bool().unwrap_or(false) && !self.agent_mode;
        self.load(&j);
    }

    fn resolve(&self, env: &Env) -> Option<Backend> {
        if let Some(k) = self.backend.strip_prefix("agent:") {
            if self.editor {
                return None;
            }
            env.agents.iter().find(|a| a.0 == k).map(|a| Backend::Agent { key: a.0.clone(), name: a.1.clone(), program: a.2.clone() })
        } else {
            let id = self.backend.strip_prefix("model:").unwrap_or(&self.backend);
            env.profiles.iter().find(|p| p.id == id).cloned().map(Backend::Model)
        }
    }

    fn cfg(&self, p: &ModelProfile, env: &Env) -> Cfg {
        let mut c = Cfg::from_profile(p, env.keys.get(&p.id).cloned().flatten());
        if !self.model.trim().is_empty() {
            c.model = self.model.trim().to_string();
        }
        c.editor = self.editor;
        c.effort = self.effort.clone();
        c.plain = self.plain;
        c
    }

    /// Wartet eine Werkzeugrunde auf Bestätigung?
    fn pending(&self) -> bool {
        matches!(self.items.last(), Some(Item::Calls(v)) if v.iter().any(|c| matches!(c.state, CallState::Ask | CallState::Running)))
    }

    fn cancel(&mut self) {
        if let Some(h) = self.cli.take() {
            h.cancel();
        }
        if self.busy.take().is_some() && self.history.last().is_some_and(|h| h.get("role").and_then(Value::as_str) == Some("user")) {
            // Nachricht ohne Antwort entfernen, damit der Verlauf gültig bleibt
            self.history.pop();
        }
        self.cli_rx = None;
        self.items.push(Item::Note("Abgebrochen.".into(), false));
    }

    fn send(&mut self, b: &Backend, env: &Env, ctx: &egui::Context) {
        let mut text = self.input.trim().to_string();
        if text.is_empty() || self.running() || self.pending() {
            return;
        }
        // Kurzbefehl („/commit“, „/erklären …“) → fester Prompt
        if let Some(p) = crate::skills::expand(&text) {
            text = p;
        }
        self.input.clear();
        let attach = std::mem::take(&mut self.attach);
        let imgs: Vec<String> = if matches!(b, Backend::Model(_)) { attach.iter().filter(|p| is_image(p)).filter_map(|p| ai::image_b64(p)).collect() } else { Vec::new() };
        // Dateien, die nicht als Bild mitgehen, als Pfade im Text
        let paths: Vec<String> = attach.iter().filter(|p| !(matches!(b, Backend::Model(_)) && is_image(p))).map(|p| p.display().to_string()).collect();
        if !paths.is_empty() {
            text = format!("{text}\n\n[Angehängte Dateien: {}]", paths.join(", "));
        }
        match b {
            Backend::Agent { key, program, .. } => {
                // Agenten ohne Sitzungsfortsetzung bekommen die letzten Nachrichten als Kontext
                let transcript = if matches!(key.as_str(), "claude" | "codex") && self.session.is_some() {
                    String::new()
                } else {
                    let mut t: Vec<String> = Vec::new();
                    for it in self.items.iter().rev() {
                        match it {
                            Item::User(u) => t.push(format!("User: {u}")),
                            Item::Assistant(a) => t.push(format!("Assistant: {a}")),
                            _ => {}
                        }
                        if t.len() >= 6 {
                            break;
                        }
                    }
                    t.reverse();
                    t.join("\n")
                };
                self.items.push(Item::User(text.clone()));
                if !attach.is_empty() {
                    self.items.push(Item::Ref(attach.clone()));
                }
                let access = if self.plain {
                    Access::Chat
                } else if !self.agent_mode {
                    Access::Read
                } else if env.cli_full {
                    Access::Full
                } else {
                    Access::Edit
                };
                // reiner Chat: der Agent läuft in einem leeren Ordner und sieht so keine Dateien des Benutzers
                let dir = if self.plain { agents::chat_dir() } else { env.cwd.to_path_buf() };
                let model = if self.model.trim().is_empty() { env.agent_model.get(key).cloned().unwrap_or_default() } else { self.model.trim().to_string() };
                let (tx, rx) = mpsc::channel();
                let effort = if matches!(key.as_str(), "claude" | "codex") { self.effort.clone() } else { String::new() };
                match agents::run(key, program, &text, &transcript, &dir, &model, &effort, access, self.session.as_deref(), tx, ctx) {
                    Ok(h) => {
                        self.cli = Some(h);
                        self.cli_rx = Some(rx);
                    }
                    Err(e) => self.items.push(Item::Note(e, true)),
                }
            }
            Backend::Model(p) => {
                if self.history_kind.is_some_and(|k| k != p.kind) {
                    self.history.clear();
                    self.items.push(Item::Note("Andere Schnittstelle – neues Gespräch begonnen.".into(), false));
                }
                self.history_kind = Some(p.kind);
                let entry = if self.plain { ai::plain_entry(&text) } else { ai::user_entry(&text, env.cwd, env.selection, self.agent_mode) };
                self.history.push(ai::with_images(entry, p.kind, &imgs));
                self.items.push(Item::User(text));
                if !attach.is_empty() {
                    self.items.push(Item::Ref(attach));
                }
                self.ask_model(&self.cfg(p, env), ctx);
            }
        }
    }

    fn ask_model(&mut self, cfg: &Cfg, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let (cfg, hist, ctx) = (cfg.clone(), self.history.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(ai::request(&cfg, &hist));
            ctx.request_repaint();
        });
        self.busy = Some(rx);
    }

    fn run_ready(&mut self, env: &Env, ctx: &egui::Context) {
        if self.exec.is_some() {
            return;
        }
        let Some(Item::Calls(v)) = self.items.last_mut() else { return };
        // Tabs und Befehle sofort hier (brauchen die Oberfläche), Dateiwerkzeuge im Hintergrund
        for c in v.iter_mut().filter(|c| c.state == CallState::Running && c.result.is_none() && c.waiting.is_none() && ai::is_editor_tool(&c.call.name)) {
            let o = ai::execute_editor(&c.call, env.cwd, env.tabs);
            if ai::is_async_tool(&c.call.name) && !o.is_error {
                // Ergebnis kommt einige Bilder später von der Oberfläche
                c.waiting = Some(std::time::Instant::now());
            } else {
                c.state = if o.is_error { CallState::Failed } else { CallState::Done };
                c.result = Some((o.text, o.is_error));
            }
            if let Some(e) = o.effect {
                self.effects.push(e);
            }
        }
        // Antworten der Oberfläche abholen
        for c in v.iter_mut().filter(|c| c.result.is_none()) {
            let Some(t0) = c.waiting else { continue };
            if let Some((text, err, img)) = ai::take_reply(&c.call.id) {
                c.state = if err { CallState::Failed } else { CallState::Done };
                c.result = Some((text, err));
                c.image = img;
                c.waiting = None;
            } else if t0.elapsed() > std::time::Duration::from_secs(10) {
                c.state = CallState::Failed;
                c.result = Some(("Keine Antwort der Oberfläche (Fenster minimiert?)".into(), true));
                c.waiting = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        let ready: Vec<(usize, Call)> = v
            .iter()
            .enumerate()
            .filter(|(_, c)| c.state == CallState::Running && c.result.is_none() && c.waiting.is_none() && !ai::is_editor_tool(&c.call.name))
            .map(|(i, c)| (i, c.call.clone()))
            .collect();
        if ready.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (cwd, sel, ctx) = (env.cwd.to_path_buf(), env.selection.to_vec(), ctx.clone());
        std::thread::spawn(move || {
            let out: Vec<(usize, ai::Outcome)> = ready.into_iter().map(|(i, c)| (i, ai::execute(&c, &cwd, &sel))).collect();
            let _ = tx.send(out);
            ctx.request_repaint();
        });
        self.exec = Some(rx);
    }

    fn poll(&mut self, b: Option<&Backend>, env: &Env, ctx: &egui::Context, out: &mut Vec<AiOut>) {
        if let Some(rx) = &self.models_rx {
            if let Ok(r) = rx.try_recv() {
                self.models = Some(r);
                self.models_rx = None;
            }
        }
        // Agent: Ereignisse
        let mut finished = false;
        if let Some(rx) = &self.cli_rx {
            while let Ok(e) = rx.try_recv() {
                match e {
                    Ev::Session(id) => self.session = Some(id),
                    Ev::Text(t) => self.items.push(Item::Assistant(t.trim().to_string())),
                    Ev::Tool(n, d) => self.items.push(Item::Step(n, d, None)),
                    Ev::ToolResult(t, err) => {
                        if let Some(Item::Step(_, _, r @ None)) = self.items.iter_mut().rev().find(|i| matches!(i, Item::Step(_, _, None))) {
                            *r = Some((t, err));
                        }
                    }
                    Ev::Done(cost, err) => {
                        if let Some(e) = err {
                            self.items.push(Item::Note(e, true));
                        }
                        if let Some(c) = cost {
                            self.cost += c;
                        }
                        finished = true;
                    }
                }
            }
        }
        if finished {
            self.cli_rx = None;
            self.cli = None;
            out.push(AiOut::Changed);
        }
        // eigenes Modell: Antwort
        if let Some(rx) = &self.busy {
            if let Ok(r) = rx.try_recv() {
                self.busy = None;
                match r {
                    Err(e) => {
                        // unbeantwortete Nachricht entfernen, damit „Nochmal senden“ einen gültigen Verlauf hat
                        if self.history.last().is_some_and(|h| h["role"] == "user") {
                            self.history.pop();
                        }
                        self.items.push(Item::Note(e, true))
                    }
                    Ok(turn) => {
                        self.history.push(turn.history_entry.clone());
                        if !turn.text.trim().is_empty() {
                            self.items.push(Item::Assistant(turn.text.trim().to_string()));
                        }
                        match turn.stop.as_str() {
                            "refusal" => self.items.push(Item::Note("Das Modell hat diese Anfrage abgelehnt.".into(), true)),
                            "max_tokens" if !turn.calls.is_empty() => self.items.push(Item::Note("Antwort abgeschnitten – Aktionen wurden nicht ausgeführt.".into(), true)),
                            _ if !turn.calls.is_empty() => {
                                let views = turn
                                    .calls
                                    .into_iter()
                                    .map(|c| {
                                        let needs_ok = ai::needs_ok(&c);
                                        let (title, lines) = ai::describe(&c, env.cwd);
                                        // riskante Shell-Befehle immer bestätigen lassen, auch wenn Befehle freigegeben sind
                                        let risky = c.name == "run_shell" && ai::risky_command(c.input.get("command").and_then(Value::as_str).unwrap_or("")).is_some();
                                        let auto = needs_ok && self.agent_mode && !risky && ai::tool_category(&c.name).is_some_and(|k| env.auto.contains(k));
                                        let (state, result) = if self.plain {
                                            (CallState::Rejected, Some(("Im Chat-Modus gibt es keine Werkzeuge.".to_string(), true)))
                                        } else if needs_ok && !self.agent_mode {
                                            (CallState::Rejected, Some(("Im Chat-/Assistent-Modus sind Änderungen nicht erlaubt – der Benutzer kann in den Agent-Modus wechseln.".to_string(), true)))
                                        } else if needs_ok && !auto {
                                            (CallState::Ask, None)
                                        } else {
                                            (CallState::Running, None)
                                        };
                                        CallView { title, lines, needs_ok, auto, state, result, call: c, image: None, waiting: None, files: Vec::new() }
                                    })
                                    .collect();
                                self.items.push(Item::Calls(views));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if let Some(rx) = &self.exec {
            // laufende Befehle zeigen ihre Ausgabe live
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
            if let Ok(res) = rx.try_recv() {
                self.exec = None;
                if let Some(Item::Calls(v)) = self.items.last_mut() {
                    for (i, o) in res {
                        if let Some(c) = v.get_mut(i) {
                            c.state = if o.is_error { CallState::Failed } else { CallState::Done };
                            c.result = Some((o.text.clone(), o.is_error));
                            if matches!(c.call.name.as_str(), "generate_image" | "edit_image") && !o.is_error {
                                c.files = image_paths(&o.text);
                            }
                            if c.call.name == "view_image" {
                                // das Bild geht mit dem Ergebnis an das Modell
                                if let Some((_, _, img)) = ai::take_reply(&c.call.id) {
                                    c.image = img;
                                }
                            }
                            if c.call.name == "run_shell" {
                                // Befehle können Dateien und Git-Stand verändert haben
                                out.push(AiOut::Changed);
                            }
                        }
                        if let Some(op) = o.op {
                            out.push(AiOut::Done(op));
                        }
                        if let Some(e) = o.effect {
                            out.push(AiOut::Effect(e));
                        }
                    }
                }
            }
        }
        self.run_ready(env, ctx);
        let round_done = matches!(self.items.last(), Some(Item::Calls(v)) if !v.is_empty() && v.iter().all(|c| c.result.is_some()));
        if round_done && self.busy.is_none() {
            if let (Some(Backend::Model(p)), Some(Item::Calls(v))) = (b, self.items.last()) {
                let results: Vec<(String, String, bool, Option<String>)> = v
                    .iter()
                    .map(|c| {
                        let (t, e) = c.result.clone().unwrap_or_default();
                        (c.call.id.clone(), t, e, c.image.clone())
                    })
                    .collect();
                self.history.extend(ai::results_entries(p.kind, &results));
                self.items.push(Item::Mark);
                let cfg = self.cfg(p, env);
                self.ask_model(&cfg, ctx);
            }
        }
        out.extend(self.effects.drain(..).map(AiOut::Effect));
        // fertige Runde speichern
        if !self.running() && !self.pending() {
            self.persist();
        }
    }

    /// Kopf: Avatar, wer antwortet, Modell/Stufe.
    /// Überschrift: Symbol und Name des Modus bzw. der KI
    fn head_title(&self, ui: &mut egui::Ui, th: &Theme) {
        avatar(ui, th, 26.0);
        let t = if self.image {
            "Bild".to_string()
        } else if self.editor {
            (if self.plain { "Chat" } else if self.agent_mode { "Explorer Agent" } else { "Explorer Assistent" }).to_string()
        } else {
            self.label.clone()
        };
        ui.label(egui::RichText::new(t).size(15.0).strong());
    }

    /// Modellwahl (Anbieter, Modell und Denkaufwand bzw. Bildmodell)
    fn head_left(&mut self, ui: &mut egui::Ui, th: &Theme, env: &Env, backend: Option<&Backend>, out: &mut Vec<AiOut>) {
        if self.image {
            self.image_head(ui, th, out);
            return;
        }
        if self.editor {
            // Anbieter: nur eigene Modelle (nur die können Tabs und Befehle bedienen)
            let shown = match backend {
                Some(Backend::Model(p)) => p.name.clone(),
                _ => "Modell wählen".into(),
            };
            let r = ui::cmd_btn(ui, th, None, &shown, true, true).on_hover_text("Welche KI antwortet (eigene Modelle aus Einstellungen → KI)");
            let pid = ui.id().with("ed_backend");
            if r.clicked() {
                ui.memory_mut(|m| m.toggle_popup(pid));
            }
            egui::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(260.0);
                ui::menu_label(ui, "Eigene Modelle");
                for p in env.profiles {
                    let id = format!("model:{}", p.id);
                    let hint = if p.model.is_empty() { "kein Modell".to_string() } else { p.model.clone() };
                    if ui::menu_entry(ui, Some(Ic::Spark), &p.name, &hint, true, self.backend == id) && self.backend != id {
                        self.reset();
                        self.backend = id;
                        self.model.clear();
                    }
                }
                ui::menu_sep(ui);
                if ui::menu_entry(ui, Some(Ic::Gear), "Modelle verwalten …", "", true, false) {
                    out.push(AiOut::OpenSettings);
                }
            });
        }
        // Modell bzw. Modellstufe
        match backend {
            Some(Backend::Agent { key, .. }) => {
                let cur = if self.model.is_empty() { env.agent_model.get(key).cloned().unwrap_or_default() } else { self.model.clone() };
                let has_effort = matches!(key.as_str(), "claude" | "codex");
                let shown = match ai::EFFORTS.iter().find(|e| e.0 == self.effort) {
                    Some(e) if has_effort => format!("{} · {}", agent_model_label(&cur), e.1),
                    _ => agent_model_label(&cur),
                };
                let r = ui::cmd_btn(ui, th, None, &shown, !self.running(), true).on_hover_text("Modell und Denkaufwand des Agenten für dieses Gespräch");
                let pid = ui.id().with("ai_agent_model");
                if r.clicked() {
                    self.model_edit = cur.clone();
                    ui.memory_mut(|m| m.toggle_popup(pid));
                }
                egui::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                    ui.set_min_width(280.0);
                    ui::menu_label(ui, "Modellstufe");
                    for m in agents::def(key).map_or(&[""][..], |d| d.models) {
                        if ui::menu_entry(ui, None, &agent_model_label(m), agent_model_hint(m), true, cur == *m) {
                            self.model = m.to_string();
                            out.push(AiOut::SetAgentModel(key.clone(), m.to_string()));
                            ui.memory_mut(|mm| mm.close_popup());
                        }
                    }
                    if has_effort {
                        ui::menu_sep(ui);
                        ui::menu_label(ui, "Denkaufwand");
                        if ui::menu_entry(ui, None, "Standard", "wie im Agenten eingestellt", true, self.effort.is_empty()) {
                            self.effort.clear();
                        }
                        for (e, n, hint) in ai::EFFORTS {
                            if ui::menu_entry(ui, None, n, hint, true, self.effort == *e) {
                                self.effort = e.to_string();
                            }
                        }
                    }
                    ui::menu_sep(ui);
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.add(egui::TextEdit::singleline(&mut self.model_edit).hint_text("eigene Modell-ID").desired_width(160.0));
                        if ui.button("Setzen").clicked() {
                            self.model = self.model_edit.trim().to_string();
                            out.push(AiOut::SetAgentModel(key.clone(), self.model.clone()));
                            ui.memory_mut(|mm| mm.close_popup());
                        }
                    });
                });
            }
            Some(Backend::Model(p)) => {
                let model = if self.model.is_empty() { p.model.clone() } else { self.model.clone() };
                let name = ai::CLAUDE_MODELS.iter().find(|m| m.0 == model).map_or(model.clone(), |m| m.1.trim_start_matches("Claude ").to_string());
                let shown = match p.kind {
                    Kind::Anthropic => format!("{} · {}", if name.is_empty() { "kein Modell" } else { &name }, ai::EFFORTS.iter().find(|e| e.0 == self.effort).map_or("Normal", |e| e.1)),
                    Kind::OpenAi => if name.is_empty() { "kein Modell".into() } else { name },
                };
                let r = ui::cmd_btn(ui, th, None, &shown, !self.running(), true).on_hover_text("Modell und Denkaufwand für dieses Gespräch");
                let pid = ui.id().with("ai_model_tier");
                if r.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(pid));
                    if p.kind == Kind::OpenAi && self.models.is_none() && self.models_rx.is_none() {
                        self.fetch_models(p, env, ui.ctx());
                    }
                }
                egui::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                    ui.set_min_width(300.0);
                    match p.kind {
                        Kind::Anthropic => {
                            ui::menu_label(ui, "Modell");
                            for (id, n) in ai::CLAUDE_MODELS {
                                let hint = match *id {
                                    m if m.contains("opus") => "am stärksten",
                                    m if m.contains("sonnet") => "schnell und stark",
                                    m if m.contains("haiku") => "am schnellsten, günstig",
                                    _ => "",
                                };
                                if ui::menu_entry(ui, None, n, hint, true, model == *id) {
                                    self.model = id.to_string();
                                }
                            }
                            ui::menu_sep(ui);
                            ui::menu_label(ui, "Denkaufwand");
                            for (e, n, hint) in ai::EFFORTS {
                                if ui::menu_entry(ui, None, n, hint, true, self.effort == *e) {
                                    self.effort = e.to_string();
                                }
                            }
                        }
                        Kind::OpenAi => {
                            ui::menu_label(ui, "Modell (vom Server)");
                            if self.models_rx.is_some() {
                                ui.spinner();
                            }
                            match self.models.clone() {
                                Some(Ok(v)) => {
                                    for m in v {
                                        if ui::menu_entry(ui, None, &m, "", true, model == m) {
                                            self.model = m;
                                            ui.memory_mut(|mm| mm.close_popup());
                                        }
                                    }
                                }
                                Some(Err(e)) => {
                                    ui.label(egui::RichText::new(e).size(12.0).color(th.danger));
                                }
                                None => {}
                            }
                            ui::menu_sep(ui);
                            if ui::menu_entry(ui, Some(Ic::Refresh), "Neu abrufen", "", self.models_rx.is_none(), false) {
                                self.fetch_models(p, env, ui.ctx());
                            }
                        }
                    }
                    if !self.model.is_empty() && self.model != p.model {
                        ui::menu_sep(ui);
                        if ui::menu_entry(ui, Some(Ic::Check), "Als Standard für dieses Profil", "", true, false) {
                            out.push(AiOut::SetProfileModel(p.id.clone(), self.model.clone()));
                            ui.memory_mut(|mm| mm.close_popup());
                        }
                    }
                });
            }
            None => {}
        }
    }

    fn fetch_models(&mut self, p: &ModelProfile, env: &Env, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let (c, ctx) = (Cfg::from_profile(p, env.keys.get(&p.id).cloned().flatten()), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(ai::list_models(&c));
            ctx.request_repaint();
        });
        self.models_rx = Some(rx);
    }

    /// Kopf rechts: Verlauf, neues Gespräch, Modus (und Schließen in der Seitenleiste).
    fn head_right(&mut self, ui: &mut egui::Ui, th: &Theme, out: &mut Vec<AiOut>, with_modes: bool) {
        if self.editor && ui::tool_btn(ui, th, Ic::Close, "Explorer-KI schließen", true, false).clicked() {
            out.push(AiOut::Close);
        }
        if ui::tool_btn(ui, th, Ic::Plus, "Neues Gespräch (+)", !self.items.is_empty(), false).clicked() {
            self.reset();
        }
        let r = ui::tool_btn(ui, th, Ic::Clock, "Verlauf", true, false);
        let pid = ui.id().with("ai_hist");
        if r.clicked() {
            self.persist();
            self.hist = list_chats().into_iter().filter(|c| c.editor == self.editor && (self.editor || c.backend == self.backend || c.image)).collect();
            self.hist_rename = None;
            self.hist_mode = Some(self.mode());
            ui.memory_mut(|m| m.toggle_popup(pid));
        }
        let mut open: Option<String> = None;
        egui::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            // feste Größe – viele Gespräche scrollen, statt das Fenster zu sprengen
            let w = 380.0;
            ui.set_min_width(w);
            ui.set_max_width(w);
            ui::menu_label(ui, "Verlauf");
            let mode = self.hist_mode.unwrap_or(self.mode());
            let key = self.model_key();
            let same_model = |c: &ChatMeta| c.model_key == key;
            // Reiter je Modus, mit Anzahl (für das aktuelle Modell bzw. alle)
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                for (m, l) in [(Mode::Agent, "Agent"), (Mode::Assistant, "Assistent"), (Mode::Chat, "Chat"), (Mode::Image, "Bild")] {
                    let n = self.hist.iter().filter(|c| c.mode() == m && (!self.hist_model_only || same_model(c))).count();
                    if ui.selectable_label(mode == m, format!("{l} {n}")).clicked() {
                        self.hist_mode = Some(m);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                let label = if mode == Mode::Image { self.img_label() } else { self.label.clone() };
                ui.checkbox(&mut self.hist_model_only, egui::RichText::new(format!("Nur {label}")).size(12.0)).on_hover_text("Nur Gespräche mit diesem Modell");
            });
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.add(egui::TextEdit::singleline(&mut self.hist_query).hint_text("Suchen …").desired_width(w - 20.0));
            });
            ui.add_space(4.0);
            let q = self.hist_query.to_lowercase();
            let mut delete: Option<String> = None;
            let mut renamed: Option<(String, String)> = None;
            egui::ScrollArea::vertical().id_salt("ai_hist_list").max_height(340.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_width(w - 8.0);
                let mut any = false;
                for c in self.hist.iter().filter(|c| c.mode() == mode && (!self.hist_model_only || same_model(c)) && (q.is_empty() || c.title.to_lowercase().contains(&q))) {
                    any = true;
                    ui.horizontal(|ui| {
                        if let Some((id, t)) = &mut self.hist_rename {
                            if *id == c.id {
                                let te = ui.add(egui::TextEdit::singleline(t).desired_width(w - 80.0));
                                te.request_focus();
                                if ui.small_button("OK").clicked() || (te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                                    renamed = Some((id.clone(), t.clone()));
                                }
                                return;
                            }
                        }
                        let current = c.id == self.chat_id;
                        let rw = (w - 8.0 - 64.0).max(120.0);
                        let (rect, resp) = ui.allocate_exact_size(vec2(rw, 36.0), Sense::click());
                        if resp.hovered() || current {
                            ui.painter().rect_filled(rect, 6.0, if current { th.sel } else { th.hover });
                        }
                        // Titel auf die Zeilenbreite kürzen
                        let p = ui.painter_at(rect.shrink2(vec2(6.0, 0.0)));
                        let title = ui.fonts(|f| {
                            let mut t = c.title.clone();
                            while t.chars().count() > 4 && f.layout_no_wrap(format!("{t}…"), FontId::proportional(13.0), th.text).size().x > rw - 16.0 {
                                t.pop();
                            }
                            if t.len() < c.title.len() { format!("{}…", t.trim_end()) } else { t }
                        });
                        p.text(pos2(rect.left() + 8.0, rect.top() + 4.0), egui::Align2::LEFT_TOP, title, FontId::proportional(13.0), th.text);
                        p.text(pos2(rect.left() + 8.0, rect.bottom() - 4.0), egui::Align2::LEFT_BOTTOM, format!("{} · {} Nachricht(en)", fmt_time(c.updated), c.messages), FontId::proportional(11.0), th.sub);
                        if resp.on_hover_text(&c.title).clicked() && !current {
                            open = Some(c.id.clone());
                        }
                        if ui::tool_btn(ui, th, Ic::Rename, "Umbenennen", true, false).clicked() {
                            self.hist_rename = Some((c.id.clone(), c.title.clone()));
                        }
                        if ui::tool_btn(ui, th, Ic::Trash, "Löschen", true, false).clicked() {
                            delete = Some(c.id.clone());
                        }
                    });
                }
                if !any {
                    ui::menu_label(ui, "   noch keine Gespräche in diesem Bereich");
                }
            });
            if let Some(id) = delete {
                delete_chat(&id);
                self.hist.retain(|c| c.id != id);
                if id == self.chat_id {
                    // laufendes Gespräch bleibt offen, wird aber nicht erneut gespeichert, bis sich etwas ändert
                    self.saved.set((self.items.len(), self.history.len()));
                }
            }
            if let Some((id, t)) = renamed {
                if !t.trim().is_empty() {
                    rename_chat(&id, &t);
                    if id == self.chat_id {
                        self.title = t.trim().to_string();
                        self.title_custom = true;
                    }
                    if let Some(c) = self.hist.iter_mut().find(|c| c.id == id) {
                        c.title = t.trim().to_string();
                    }
                }
                self.hist_rename = None;
            }
        });
        if let Some(id) = open {
            ui.memory_mut(|m| m.close_popup());
            self.open_saved(&id);
        }
        if with_modes {
            self.mode_row(ui, th, None);
        }
    }

    /// Umschalter Agent / Assistent / Chat / Bild; `width` = über die ganze Breite
    fn mode_row(&mut self, ui: &mut egui::Ui, th: &Theme, width: Option<f32>) {
        let cur = self.mode();
        let mut m = cur;
        mode_switch(ui, th, &mut m, !self.running() && !self.pending(), width);
        if m != cur {
            self.set_image(m == Mode::Image);
            if m != Mode::Image {
                self.set_mode(m == Mode::Agent, m == Mode::Chat);
            }
            self.resume_last();
        }
    }

    /// Kopfzeile im Modus „Bild“: nur Bildmodelle zur Auswahl
    fn image_head(&mut self, ui: &mut egui::Ui, th: &Theme, out: &mut Vec<AiOut>) {
        let choices = crate::localllm::image_choices();
        if self.img_model.as_ref().is_none_or(|m| !choices.iter().any(|c| c.1 == *m)) {
            self.img_model = choices.first().map(|c| c.1.clone());
        }
        let shown = self.img_model.as_ref().and_then(|m| choices.iter().find(|c| c.1 == *m)).map_or_else(|| "Bildmodell wählen".to_string(), |c| c.0.clone());
        let r = ui::cmd_btn(ui, th, None, &shown, !self.running(), true).on_hover_text("Welches lokale Bildmodell malt (Einstellungen → Lokale Modelle → Bildmodelle)");
        let pid = ui.id().with("img_backend");
        if r.clicked() {
            ui.memory_mut(|m| m.toggle_popup(pid));
        }
        egui::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(300.0);
            ui::menu_label(ui, "Bildmodelle");
            if choices.is_empty() {
                ui.label(egui::RichText::new("Noch keins installiert").size(12.0).color(th.sub));
            }
            for (name, c, kind, _) in &choices {
                if ui::menu_entry(ui, Some(Ic::Spark), name, kind, true, self.img_model.as_ref() == Some(c)) {
                    self.img_model = Some(c.clone());
                }
            }
            ui::menu_sep(ui);
            ui::menu_label(ui, "Vorlage bearbeiten (Bild ins Feld ziehen)");
            let cur_edit = self.edit_choice().map(|m| m.id);
            for m in crate::localllm::EDIT {
                if m.installed() {
                    if ui::menu_entry(ui, Some(Ic::Rename), m.name, m.desc, true, cur_edit == Some(m.id)) {
                        self.edit_model = Some(m.id);
                    }
                } else if let Some((f, _)) = m.progress() {
                    ui::menu_entry(ui, Some(Ic::Download), m.name, &if f >= 0.0 { format!("lädt … {:.0} %", f * 100.0) } else { "wartet …".into() }, false, false);
                } else if ui::menu_entry(ui, Some(Ic::Download), &format!("{} laden", m.name), &crate::localui::gb(m.missing_mb() as u64), true, false) {
                    crate::localllm::install_edit(m);
                }
            }
            ui::menu_sep(ui);
            if ui::menu_entry(ui, Some(Ic::Gear), "Bildmodelle laden …", "", true, false) {
                out.push(AiOut::OpenSettings);
            }
        });
        if let Some(m) = self.edit_choice().filter(|_| !self.attach.is_empty()) {
            ui.label(egui::RichText::new(format!("bearbeitet mit {}", m.name)).size(12.0).color(th.sub));
        }
    }

    /// Bild erzeugen (im Hintergrund), gespeichert im aktuellen Ordner
    fn image_send(&mut self, env: &Env, ctx: &egui::Context) {
        let prompt = self.input.trim().to_string();
        if prompt.is_empty() || self.running() {
            return;
        }
        // „bearbeite das Bild …“ ohne Anhang: das zuletzt gezeigte bzw. markierte Bild weiterbearbeiten
        let reference = self.attach.first().cloned().or_else(|| if wants_edit(&prompt) { self.last_image(env) } else { None });
        if reference.is_none() && self.img_model.is_none() {
            return;
        }
        self.input.clear();
        self.attach.clear();
        self.items.push(Item::User(prompt.clone()));
        match reference {
            Some(r) => {
                self.items.push(Item::Ref(vec![r.clone()]));
                self.image_edit(prompt, r, env, ctx);
            }
            None => self.image_generate(prompt, 1, env, ctx),
        }
    }

    /// Bild aus Text erzeugen (Hintergrund), gespeichert im aktuellen Ordner
    fn image_generate(&mut self, prompt: String, count: u32, env: &Env, ctx: &egui::Context) {
        let Some(model) = self.img_model.clone() else { return };
        let (size, steps, cfg, neg_ok) = crate::localllm::choice_settings(&model);
        // Stil-Vorlage: Prompt ergänzen, passender negativer Prompt (nur bei Modellen, die ihn nutzen)
        let (prompt, negative) = match self.img_style.and_then(|i| STYLES.get(i)) {
            Some((_, add, neg)) => (format!("{prompt}, {add}"), if neg_ok { neg.to_string() } else { String::new() }),
            None => (prompt, String::new()),
        };
        let req = crate::localllm::GenReq { model, prompt, negative, w: size, h: size, steps, cfg, seed: (now() as i64 * 7919 + rand_u16() as i64) % 2_147_483_647, count };
        let dir = env.cwd.to_path_buf();
        self.start_img_job(ctx, move |p| crate::localllm::generate(&req, &dir, p));
    }

    /// Vorlage nach Anweisung ändern – mit einem lokalen Bearbeitungsmodell
    fn image_edit(&mut self, prompt: String, reference: PathBuf, env: &Env, ctx: &egui::Context) {
        let Some(m) = self.edit_choice() else {
            self.items.push(Item::Note("Zum Bearbeiten einer Vorlage fehlt noch ein Bearbeitungsmodell – oben in der Modellwahl laden (z. B. FLUX.2 klein, 5 GB).".into(), true));
            return;
        };
        let dir = env.cwd.to_path_buf();
        let seed = (now() as i64 * 7919 + rand_u16() as i64) % 2_147_483_647;
        let prompt = match self.img_style.and_then(|i| STYLES.get(i)) {
            Some((_, add, _)) => format!("{prompt}, {add}"),
            None => prompt,
        };
        self.start_img_job(ctx, move |p| {
            let img = image::open(&reference).map_err(|e| format!("Vorlage: {e}"))?.to_rgba8();
            let out = crate::localllm::edit_image(m, &img, &prompt, seed, p)?;
            let stem = reference.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Bild".into());
            let file = unique_png(&dir, &format!("{stem}_KI"));
            out.save(&file).map_err(|e| e.to_string())?;
            Ok(vec![file])
        });
    }

    /// erstes Bild vergrößern (Real-ESRGAN, 2×)
    fn image_upscale(&mut self, file: PathBuf, ctx: &egui::Context, out: &mut Vec<AiOut>) {
        if !crate::localai::missing(&[crate::localai::Need::Upscale]).is_empty() {
            self.items.push(Item::Note("Zum Vergrößern fehlt Real-ESRGAN (45 MB) – Einstellungen → Bildeditor-KI.".into(), true));
            out.push(AiOut::OpenSettings);
            return;
        }
        self.start_img_job(ctx, move |p| {
            let img = image::open(&file).map_err(|e| e.to_string())?.to_rgba8();
            let big = crate::localai::upscale(&img, 2, false, p)?;
            let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Bild".into());
            let dest = unique_png(file.parent().unwrap_or(Path::new(".")), &format!("{stem}_2x"));
            big.save(&dest).map_err(|e| e.to_string())?;
            Ok(vec![dest])
        });
    }

    fn start_img_job(&mut self, ctx: &egui::Context, f: impl FnOnce(&crate::localai::Progress) -> Result<Vec<PathBuf>, String> + Send + 'static) {
        let prog = crate::localai::Progress::default();
        let (tx, rx) = mpsc::channel();
        let (p, ctx) = (prog.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(f(&p));
            ctx.request_repaint();
        });
        self.img_job = Some((prog, rx));
    }

    /// Bild, auf das sich „das Bild“ bezieht: das zuletzt im Gespräch gezeigte, sonst ein markiertes im Ordner
    fn last_image(&self, env: &Env) -> Option<PathBuf> {
        let from_chat = self.items.iter().rev().find_map(|it| match it {
            Item::Images(v) | Item::Ref(v) => v.iter().rev().find(|p| is_image(p) && p.is_file()).cloned(),
            Item::Calls(v) => v.iter().rev().find_map(|c| c.files.last().filter(|p| p.is_file()).cloned()),
            _ => None,
        });
        from_chat.or_else(|| env.selection.iter().map(|n| env.cwd.join(n)).find(|p| is_image(p) && p.is_file()))
    }

    /// gewähltes bzw. erstes installiertes Bearbeitungsmodell
    fn edit_choice(&self) -> Option<&'static crate::localllm::EditModel> {
        let ed = crate::localllm::EDIT;
        self.edit_model.and_then(|id| ed.iter().find(|m| m.id == id && m.installed())).or_else(|| ed.iter().find(|m| m.installed()))
    }

    /// Verlauf und Eingabe im Modus „Bild“
    fn image_ui(&mut self, ui: &mut egui::Ui, th: &Theme, env: &Env, ctx: &egui::Context, out: &mut Vec<AiOut>) {
        if let Some((_, rx)) = &self.img_job {
            match rx.try_recv() {
                Ok(r) => {
                    self.img_job = None;
                    self.items.push(match r {
                        Ok(files) => Item::Images(files),
                        Err(e) => Item::Note(e, true),
                    });
                    out.push(AiOut::Changed);
                    self.persist();
                }
                Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
            }
        }
        if crate::localllm::image_choices().is_empty() {
            centered_card(ui, th, |ui| {
                ui.label(egui::RichText::new("Noch kein Bildmodell").size(16.0).strong());
                ui.label(egui::RichText::new("Bilder entstehen lokal auf diesem PC. Erst ein Bildmodell laden – z. B. SD-Turbo (2 GB, sehr schnell) oder FLUX/Z-Image für beste Qualität.").color(th.sub));
                ui.add_space(6.0);
                if ui.button("Einstellungen → Lokale Modelle").clicked() {
                    out.push(AiOut::OpenSettings);
                }
            });
            return;
        }
        let input_h = if self.attach.is_empty() { 140.0 } else { 186.0 };
        let list_h = (ui.available_height() - input_h).max(80.0);
        let mut suggestion: Option<String> = None;
        let mut open: Option<PathBuf> = None;
        let mut action: Option<(usize, ImgAction)> = None;
        let busy_now = self.running();
        let (items, thumbs) = (&self.items, &self.thumbs);
        egui::ScrollArea::vertical().id_salt("ai_img_chat").max_height(list_h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            let w = ui.available_width();
            let col_w = w.min(820.0);
            let pad = ((w - col_w) / 2.0).max(0.0);
            ui.add_space(12.0);
            if items.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    avatar(ui, th, 44.0);
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Was soll ich malen?").size(20.0).strong());
                    ui.add(egui::Label::new(egui::RichText::new(format!("Beschreibe das Bild – es entsteht lokal und wird in {} gespeichert. Englisch klappt meist am besten.", env.cwd.display())).size(13.0).color(th.sub)).wrap());
                    ui.add_space(14.0);
                    for c in ["a cozy cabin in snowy mountains at dusk, warm light, photo", "cute robot reading a book in a library, pixar style", "minimalist logo of a fox, flat vector, orange"] {
                        if chip(ui, th, c).clicked() {
                            suggestion = Some(c.to_string());
                        }
                    }
                });
            }
            for (n, item) in items.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(col_w - 24.0);
                        match item {
                            Item::User(t) => user_bubble(ui, th, t),
                            Item::Ref(v) => ref_strip(ui, th, v, thumbs),
                            Item::Images(v) => {
                                if let Some(f) = images_grid(ui, th, v, thumbs, col_w) {
                                    open = Some(f);
                                }
                                // Weiterarbeiten: Nochmal, Varianten, Größer, Editor
                                if !busy_now {
                                    if let Some(a) = image_actions(ui, th, n) {
                                        action = Some((n, a));
                                    }
                                }
                            }
                            Item::Note(t, err) => {
                                ui.add(egui::Label::new(egui::RichText::new(t).size(12.5).color(if *err { th.danger } else { th.sub })).wrap());
                                ui.add_space(4.0);
                            }
                            _ => {}
                        }
                    });
                });
            }
            if let Some((p, _)) = &self.img_job {
                let (f, t) = p.get();
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.spinner();
                    if f >= 0.0 {
                        ui.add(egui::ProgressBar::new(f.clamp(0.0, 1.0)).desired_width(160.0).desired_height(6.0));
                    }
                    ui.label(egui::RichText::new(if t.is_empty() { "malt …".to_string() } else { t }).size(12.5).color(th.sub));
                });
            }
            ui.add_space(10.0);
        });
        if let Some(s) = suggestion {
            self.input = s;
        }
        if let Some(f) = open {
            out.push(AiOut::Preview(f));
        }
        if let Some((n, a)) = action {
            self.image_action(n, a, env, ctx, out);
        }
        // Eingabe
        let can = !self.running();
        let area = ui.available_rect_before_wrap();
        let w = area.width().min(844.0);
        let x0 = area.center().x - w / 2.0;
        let strip = !self.attach.is_empty();
        // Stil-Vorlagen (optional): ein Klick wählt, ein zweiter hebt die Wahl auf
        let chips = Rect::from_min_size(pos2(x0 + 12.0, area.top() + 4.0), vec2(w - 24.0, 26.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(chips), |ui| {
            egui::ScrollArea::horizontal().id_salt("img_styles").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(egui::RichText::new("Stil").size(12.0).color(th.sub));
                for (i, (name, add, _)) in STYLES.iter().enumerate() {
                    let on = self.img_style == Some(i);
                    let b = egui::Button::new(egui::RichText::new(*name).size(12.0).color(if on { th.on_accent() } else { th.text })).fill(if on { th.accent } else { th.field }).stroke(Stroke::new(1.0, if on { th.accent } else { th.line })).rounding(Rounding::same(11.0));
                    if ui.add(b).on_hover_text(format!("Ergänzt den Prompt um: {add}")).clicked() {
                        self.img_style = if on { None } else { Some(i) };
                    }
                }
            }));
        });
        let frame_rect = Rect::from_min_size(pos2(x0 + 12.0, area.top() + 36.0), vec2(w - 24.0, if strip { 122.0 } else { 76.0 }));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(frame_rect), |ui| {
            egui::Frame::none()
                .fill(th.field)
                .stroke(Stroke::new(1.0, th.line))
                .rounding(Rounding::same(12.0))
                .inner_margin(Margin { left: 12.0, right: 8.0, top: 8.0, bottom: 8.0 })
                .show(ui, |ui| {
                    attach_chips(ui, th, &mut self.attach, &self.thumbs);
                    ui.horizontal(|ui| {
                        attach_button(ui, th, &mut self.attach, true);
                        let tw = ui.available_width() - 90.0;
                        let hint = if !can { "Bild entsteht …" } else if self.attach.is_empty() { "Bild beschreiben … oder ein Bild hierher ziehen und sagen, was daraus werden soll" } else { "Was soll aus dem Bild werden? z. B. „mach daraus ein Aquarell“" };
                        let te = egui::ScrollArea::vertical().id_salt("ai_img_input").max_height(54.0).max_width(tw).stick_to_bottom(true).show(ui, |ui| {
                            ui.add_enabled(can, egui::TextEdit::multiline(&mut self.input).frame(false).hint_text(hint).desired_rows(2).desired_width(tw - 10.0))
                        }).inner;
                        let enter = te.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                        if enter {
                            while self.input.ends_with('\n') {
                                self.input.pop();
                            }
                        }
                        self.mic.button(ui, th, &mut self.input, can);
                        let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                        if let Some((p, _)) = &self.img_job {
                            ui.painter().circle_filled(r.center(), 18.0, if resp.hovered() { th.danger } else { th.danger.gamma_multiply(0.85) });
                            ui.painter().rect_filled(Rect::from_center_size(r.center(), vec2(11.0, 11.0)), 2.0, Color32::WHITE);
                            if resp.on_hover_text("Abbrechen").clicked() {
                                p.cancel();
                            }
                        } else {
                            let on = can && !self.input.trim().is_empty();
                            ui.painter().circle_filled(r.center(), 18.0, if on { th.accent } else { th.line });
                            ui::draw(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Ic::Up, if on { th.on_accent() } else { Color32::WHITE });
                            if (resp.on_hover_text(if self.attach.is_empty() { "Erzeugen" } else { "Vorlage bearbeiten" }).clicked() || enter) && on {
                                self.image_send(env, ctx);
                            }
                        }
                    });
                });
            ui.add(egui::Label::new(egui::RichText::new(format!("Bild: entsteht lokal auf diesem PC und wird in {} gespeichert.", env.cwd.display())).size(11.5).color(th.sub)).truncate());
        });
    }

    /// Zeichnet den Chat.
    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme, env: &Env) -> Vec<AiOut> {
        let ctx = ui.ctx().clone();
        let mut out = Vec::new();
        self.cwd = env.cwd.to_path_buf();
        let backend = self.resolve(env);
        self.label = match (&backend, self.editor) {
            (_, true) => if self.plain { "Chat".into() } else if self.agent_mode { "Explorer Agent".into() } else { "Explorer Assistent".into() },
            (Some(Backend::Agent { name, .. }), _) => name.clone(),
            (Some(Backend::Model(p)), _) => p.name.clone(),
            (None, _) => "KI-Chat".into(),
        };
        self.poll(backend.as_ref(), env, &ctx, &mut out);
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, th.bg);

        // ── Kopfzeile (schmal: zwei Zeilen) ──
        // schmal (Seitenleiste): Titel + Knöpfe · Modus über die ganze Breite · Modellwahl; breit: eine Zeile
        let narrow = rect.width() < 640.0;
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), if narrow { 128.0 } else { 54.0 }));
        ui.painter().rect_filled(head, 0.0, th.panel);
        ui.painter().line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, th.line));
        let inner = head.shrink2(vec2(12.0, 0.0));
        if narrow {
            let row = |top: f32, h: f32| Rect::from_min_size(pos2(inner.left(), inner.top() + top), vec2(inner.width(), h));
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(row(4.0, 40.0)), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    self.head_title(ui, th);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.head_right(ui, th, &mut out, false));
                });
            });
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(row(46.0, 34.0)), |ui| {
                ui.horizontal_centered(|ui| self.mode_row(ui, th, Some(inner.width())));
            });
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(row(86.0, 38.0)), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    self.head_left(ui, th, env, backend.as_ref(), &mut out);
                });
            });
        } else {
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    self.head_title(ui, th);
                    ui.add_space(6.0);
                    self.head_left(ui, th, env, backend.as_ref(), &mut out);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.head_right(ui, th, &mut out, true));
                });
            });
        }
        ui.advance_cursor_after_rect(head);
        // Modus „Bild“: eigener Verlauf mit lokalem Bildmodell, braucht kein Sprachmodell
        if self.image {
            self.image_ui(ui, th, env, &ctx, &mut out);
            return out;
        }

        // ── Kein Backend / fehlende Einrichtung ──
        let Some(backend) = backend else {
            centered_card(ui, th, |ui| {
                if self.editor {
                    ui.label(egui::RichText::new("Kein Modell für die Explorer-KI").size(16.0).strong());
                    ui.label(egui::RichText::new("Die Explorer-KI braucht ein eigenes Modell (Anthropic-API, LM Studio, OpenAI-kompatibel …) – oben auswählen oder in den Einstellungen anlegen.").color(th.sub));
                } else {
                    ui.label(egui::RichText::new("Diese KI ist nicht verfügbar").size(16.0).strong());
                    ui.label(egui::RichText::new("Der Agent ist ausgeschaltet oder nicht installiert bzw. das Modell wurde gelöscht. In den Einstellungen → KI lässt sich das ändern.").color(th.sub));
                }
                ui.add_space(6.0);
                if ui.button("Einstellungen → KI").clicked() {
                    out.push(AiOut::OpenSettings);
                }
            });
            return out;
        };
        if let Backend::Model(p) = &backend {
            let has_key = env.keys.get(&p.id).is_some_and(|k| k.is_some());
            if p.needs_key() && !has_key {
                centered_card(ui, th, |ui| {
                    ui.label(egui::RichText::new(format!("{} verbinden", p.name)).size(17.0).strong());
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new("Füge deinen API-Schlüssel ein. Er wird verschlüsselt und an dein Benutzerkonto gebunden gespeichert und nie exportiert.").size(12.5).color(th.sub));
                    ui.add_space(12.0);
                    let w = ui.available_width();
                    let field = ui.add(egui::TextEdit::singleline(&mut self.key_input).password(true).hint_text("Schlüssel einfügen (z. B. sk-…)").desired_width(w).margin(egui::Margin::symmetric(10.0, 8.0)));
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(10.0);
                    let ready = !self.key_input.trim().is_empty();
                    let mut save = enter && ready;
                    ui.horizontal(|ui| {
                        let txt = th.on_accent();
                        let b = egui::Button::new(egui::RichText::new("Speichern").strong().color(if ready { txt } else { th.sub })).fill(if ready { th.accent } else { th.field }).min_size(egui::vec2(110.0, 32.0)).rounding(8.0);
                        if ui.add_enabled(ready, b).clicked() {
                            save = true;
                        }
                        // wo es den Schlüssel gibt (bekannte Anbieter)
                        let u = p.url.to_lowercase();
                        let page = if u.contains("anthropic") {
                            Some("https://console.anthropic.com/settings/keys")
                        } else if u.contains("openai.com") {
                            Some("https://platform.openai.com/api-keys")
                        } else if u.contains("openrouter") {
                            Some("https://openrouter.ai/keys")
                        } else if u.contains("googleapis") || u.contains("gemini") {
                            Some("https://aistudio.google.com/apikey")
                        } else {
                            None
                        };
                        if let Some(page) = page {
                            if ui.add(egui::Button::new("Schlüssel erstellen …").min_size(egui::vec2(0.0, 32.0)).rounding(8.0)).on_hover_text(page).clicked() {
                                let _ = open::that_detached(page);
                            }
                        }
                    });
                    if save {
                        match ai::save_key(&p.id, &self.key_input) {
                            Ok(()) => {
                                self.key_input.clear();
                                out.push(AiOut::KeySaved(p.id.clone()));
                            }
                            Err(e) => self.items.push(Item::Note(e, true)),
                        }
                    }
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("Ohne Schlüssel und ohne Internet:").size(12.5).color(th.sub));
                        if ui.link(egui::RichText::new("lokales Modell verwenden").size(12.5)).clicked() {
                            out.push(AiOut::OpenSettings);
                        }
                    });
                });
                return out;
            }
            if p.model.trim().is_empty() && self.model.trim().is_empty() {
                centered_card(ui, th, |ui| {
                    ui.label(egui::RichText::new(format!("Modell für „{}“ wählen", p.name)).size(16.0).strong());
                    ui.label(
                        egui::RichText::new(if p.is_local() {
                            "Im lokalen Server (z. B. LM Studio: Developer → Start Server) ein Modell laden, dann hier abrufen."
                        } else {
                            "Verfügbare Modelle beim Anbieter abrufen oder die Modell-ID in den Einstellungen eintragen."
                        })
                        .size(12.5)
                        .color(th.sub),
                    );
                    ui.add_space(6.0);
                    if ui.add_enabled(self.models_rx.is_none(), egui::Button::new("Verfügbare Modelle abrufen")).clicked() {
                        self.fetch_models(p, env, &ctx);
                    }
                    if self.models_rx.is_some() {
                        ui.spinner();
                    }
                    match &self.models {
                        Some(Ok(v)) if v.is_empty() => {
                            ui.label("Keine Modelle geladen.");
                        }
                        Some(Ok(v)) => {
                            for m in v {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(m).monospace());
                                    if ui.small_button("Verwenden").clicked() {
                                        out.push(AiOut::SetProfileModel(p.id.clone(), m.clone()));
                                    }
                                });
                            }
                        }
                        Some(Err(e)) => {
                            ui.label(egui::RichText::new(e).color(th.danger));
                        }
                        None => {}
                    }
                });
                return out;
            }
        }

        // ── Verlauf ──
        let input_h = if self.attach.is_empty() { 112.0 } else { 158.0 };
        let list_h = (ui.available_height() - input_h).max(80.0);
        let mut decide: Option<(usize, bool)> = None;
        let mut suggestion: Option<String> = None;
        let mut preview: Option<PathBuf> = None;
        egui::ScrollArea::vertical().id_salt("ai_chat").max_height(list_h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            let w = ui.available_width();
            let col_w = w.min(820.0);
            let pad = ((w - col_w) / 2.0).max(0.0);
            ui.add_space(12.0);
            if self.items.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    avatar(ui, th, 44.0);
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Wie kann ich helfen?").size(20.0).strong());
                    let sub = match (&backend, self.agent_mode) {
                        (Backend::Model(p), _) if self.plain => format!("{} beantwortet einfach deine Nachrichten – ohne Zugriff auf Explorer, Ordner oder Dateien.", p.name),
                        (Backend::Agent { name, .. }, _) if self.plain => format!("{name} beantwortet einfach deine Nachrichten – ohne Zugriff auf deine Ordner oder Dateien."),
                        (Backend::Model(p), true) if self.editor => format!("{} bedient den Explorer: Tabs öffnen und wechseln, Ansichten umschalten, Dateien bearbeiten – Änderungen nach deiner Bestätigung.", p.name),
                        (Backend::Model(p), false) if self.editor => format!("{} sieht deine Tabs und Dateien und beantwortet Fragen – Tabs wechseln geht, Änderungen nicht.", p.name),
                        (Backend::Agent { name, .. }, true) => format!("{name} arbeitet im Ordner {} und darf dort Dateien bearbeiten.", env.cwd.display()),
                        (Backend::Agent { name, .. }, false) => format!("{name} schaut sich {} an und beantwortet Fragen – ohne etwas zu ändern.", env.cwd.display()),
                        (Backend::Model(p), true) => format!("{} kann Dateien ansehen, bearbeiten und verwalten – Änderungen {}.", p.name, if env.auto.is_empty() { "nach deiner Bestätigung" } else { "teils selbstständig (Einstellungen)" }),
                        (Backend::Model(p), false) => format!("{} sieht sich Dateien an und beantwortet Fragen.", p.name),
                    };
                    ui.add(egui::Label::new(egui::RichText::new(sub).size(13.0).color(th.sub)).wrap());
                    ui.add_space(14.0);
                    let chips: &[&str] = match (self.editor, self.agent_mode) {
                        _ if self.plain => &["Erkläre mir den Unterschied zwischen TCP und UDP", "Schreib mir eine kurze, höfliche Absage-E-Mail", "Wie funktioniert ein Hash-Map intern?"],
                        (true, true) => &["Öffne meinen Downloads-Ordner in einem neuen Tab", "Zeige diesen Ordner als Raster mit Vorschau", "Öffne hier ein Terminal und die Git-Ansicht"],
                        (true, false) => &["Welche Tabs sind offen?", "Was liegt in diesem Ordner?", "Welche Dateien sind am größten?"],
                        (false, true) => &["Sortiere die Dateien hier nach Typ in Unterordner", "Zeig mir git status und committe die Änderungen", "Erstelle eine README für dieses Projekt"],
                        (false, false) => &["Was liegt in diesem Ordner?", "Welche Dateien sind am größten?", "Erkläre mir dieses Projekt"],
                    };
                    for c in chips {
                        if chip(ui, th, c).clicked() {
                            suggestion = Some(c.to_string());
                        }
                    }
                });
            }
            let last = self.items.len().saturating_sub(1);
            for (n, item) in self.items.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(col_w - 24.0);
                        match item {
                            Item::User(t) => user_bubble(ui, th, t),
                            Item::Assistant(t) => assistant_msg(ui, th, t),
                            Item::Step(tool, detail, res) => step_row(ui, th, tool, detail, res.as_ref()),
                            Item::Note(t, err) => {
                                ui.add(egui::Label::new(egui::RichText::new(t).size(12.5).color(if *err { th.danger } else { th.sub })).wrap());
                                ui.add_space(4.0);
                            }
                            Item::Mark => {}
                            Item::Ref(v) => ref_strip(ui, th, v, &self.thumbs),
                            Item::Images(v) => {
                                if let Some(f) = images_grid(ui, th, v, &self.thumbs, col_w) {
                                    preview = Some(f);
                                }
                            }
                            Item::Calls(v) => {
                                for (i, c) in v.iter().enumerate() {
                                    if let Some(d) = call_card(ui, th, c, n == last) {
                                        decide = Some((i, d));
                                    }
                                    if !c.files.is_empty() {
                                        if let Some(f) = images_grid(ui, th, &c.files, &self.thumbs, col_w) {
                                            preview = Some(f);
                                        }
                                    }
                                }
                            }
                        }
                    });
                });
            }
            if self.running() {
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.spinner();
                    let t = match &backend {
                        Backend::Agent { name, .. } => format!("{name} arbeitet …"),
                        Backend::Model(_) => "denkt nach …".into(),
                    };
                    ui.label(egui::RichText::new(t).size(12.5).color(th.sub));
                });
            }
            if self.cost > 0.0 {
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.label(egui::RichText::new(format!("Kosten dieses Gesprächs laut Agent: {:.4} $", self.cost)).size(11.5).color(th.sub));
                });
            }
            ui.add_space(10.0);
        });
        if let Some(s) = suggestion {
            self.input = s;
        }
        if let Some(f) = preview {
            out.push(AiOut::Preview(f));
        }
        if let Some((i, accept)) = decide {
            if let Some(Item::Calls(v)) = self.items.last_mut() {
                if let Some(c) = v.get_mut(i) {
                    if accept {
                        c.state = CallState::Running;
                    } else {
                        c.state = CallState::Rejected;
                        c.result = Some(("Der Benutzer hat diese Aktion abgelehnt.".into(), true));
                    }
                }
            }
            self.run_ready(env, &ctx);
            out.extend(self.effects.drain(..).map(AiOut::Effect));
        }

        // ── Eingabe ──
        let can = !self.running() && !self.pending();
        let area = ui.available_rect_before_wrap();
        let w = area.width().min(844.0);
        let x0 = area.center().x - w / 2.0;
        let strip = !self.attach.is_empty();
        let frame_rect = Rect::from_min_size(pos2(x0 + 12.0, area.top() + 8.0), vec2(w - 24.0, if strip { 122.0 } else { 76.0 }));
        // Kurzbefehle: „/“ tippen zeigt die passenden; ↑/↓ wählen, Enter/Tab übernimmt
        let skills = if can { crate::skills::matching(&self.input) } else { Vec::new() };
        let mut take_skill: Option<String> = None;
        if !skills.is_empty() {
            self.skill_sel = self.skill_sel.min(skills.len() - 1);
            let (down, up, tab) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)));
            if down {
                self.skill_sel = (self.skill_sel + 1) % skills.len();
            }
            if up {
                self.skill_sel = (self.skill_sel + skills.len() - 1) % skills.len();
            }
            if tab {
                take_skill = Some(skills[self.skill_sel].name.clone());
            }
            let rows = skills.len().min(8);
            let h = rows as f32 * 34.0 + 34.0;
            egui::Area::new(egui::Id::new(("ai_skills", &self.chat_id))).order(egui::Order::Foreground).fixed_pos(pos2(frame_rect.left(), frame_rect.top() - h - 6.0)).show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).fill(th.panel).show(ui, |ui| {
                    ui.set_width(frame_rect.width() - 16.0);
                    egui::ScrollArea::vertical().max_height(rows as f32 * 34.0).show(ui, |ui| {
                        for (i, s) in skills.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                            let sel = i == self.skill_sel;
                            if sel || resp.hovered() {
                                ui.painter().rect_filled(r, 6.0, if sel { th.sel } else { th.hover });
                            }
                            if sel && (down || up) {
                                ui.scroll_to_rect(r, None);
                            }
                            let g = ui.painter().layout_no_wrap(format!("/{}", s.name), egui::FontId::monospace(13.0), th.accent);
                            let nw = g.size().x;
                            ui.painter().galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, th.accent);
                            ui::paint_text(ui, r.left() + 24.0 + nw.max(110.0), r.center().y, &s.desc, 12.5, th.sub, r.width() - nw.max(110.0) - 110.0, false);
                            ui.painter().text(pos2(r.right() - 10.0, r.center().y), egui::Align2::RIGHT_CENTER, &s.group, egui::FontId::proportional(11.0), th.sub);
                            if resp.clicked() {
                                take_skill = Some(s.name.clone());
                            }
                        }
                    });
                    if ui.add(egui::Label::new(egui::RichText::new("Eigene Kurzbefehle bearbeiten …").size(12.0).color(th.accent)).sense(Sense::click())).on_hover_text("JSON-Datei mit Name, Beschreibung und Prompt; {text} steht für das, was hinter dem Befehl getippt wird").clicked() {
                        out.push(AiOut::Preview(crate::skills::ensure_custom_file()));
                        self.input.clear();
                    }
                });
            });
        }
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(frame_rect), |ui| {
            egui::Frame::none()
                .fill(th.field)
                .stroke(Stroke::new(1.0, th.line))
                .rounding(Rounding::same(12.0))
                .inner_margin(Margin { left: 12.0, right: 8.0, top: 8.0, bottom: 8.0 })
                .show(ui, |ui| {
                    attach_chips(ui, th, &mut self.attach, &self.thumbs);
                    ui.horizontal(|ui| {
                        attach_button(ui, th, &mut self.attach, false);
                        let hint = if can {
                            "Nachricht … (Enter senden, Umschalt+Enter neue Zeile)"
                        } else if self.pending() {
                            "Bitte zuerst die vorgeschlagenen Änderungen übernehmen oder ablehnen"
                        } else {
                            "Antwort läuft …"
                        };
                        // lange Eingaben scrollen im Feld statt aus dem Rahmen zu laufen
                        let tw = ui.available_width() - 90.0;
                        let te = egui::ScrollArea::vertical().id_salt("ai_input").max_height(54.0).max_width(tw).stick_to_bottom(true).show(ui, |ui| {
                            ui.add_enabled(can, egui::TextEdit::multiline(&mut self.input).frame(false).hint_text(hint).desired_rows(2).desired_width(tw - 10.0))
                        }).inner;
                        let mut enter = te.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                        if enter {
                            while self.input.ends_with('\n') {
                                self.input.pop();
                            }
                        }
                        // Enter bei offener Liste übernimmt den Kurzbefehl (statt zu senden)
                        if enter && !skills.is_empty() && take_skill.is_none() {
                            take_skill = Some(skills[self.skill_sel].name.clone());
                        }
                        if let Some(n) = take_skill.take() {
                            enter = false;
                            self.input = format!("/{n} ");
                            self.skill_sel = 0;
                            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), te.id) {
                                let end = egui::text::CCursor::new(self.input.chars().count());
                                st.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                                st.store(ui.ctx(), te.id);
                            }
                            te.request_focus();
                        }
                        self.mic.button(ui, th, &mut self.input, can);
                        let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                        if self.running() {
                            ui.painter().circle_filled(r.center(), 18.0, if resp.hovered() { th.danger } else { th.danger.gamma_multiply(0.85) });
                            ui.painter().rect_filled(Rect::from_center_size(r.center(), vec2(11.0, 11.0)), 2.0, Color32::WHITE);
                            if resp.on_hover_text("Anhalten").clicked() {
                                self.cancel();
                            }
                        } else {
                            let on = can && !self.input.trim().is_empty();
                            ui.painter().circle_filled(r.center(), 18.0, if on { th.accent } else { th.line });
                            ui::draw(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Ic::Up, if on { th.on_accent() } else { Color32::WHITE });
                            if (resp.on_hover_text("Senden").clicked() || enter) && on {
                                self.send(&backend, env, &ctx);
                            }
                        }
                    });
                });
            let note = match (&backend, self.agent_mode) {
                _ if self.plain => "Chat: antwortet nur auf deine Nachrichten – kein Zugriff auf Explorer, Ordner oder Dateien.".to_string(),
                (_, false) if self.editor => "Explorer Assistent: sieht Tabs und Dateien an, ändert nichts.".to_string(),
                (_, false) => "Assistent: sieht sich Dateien an, ändert nichts.".to_string(),
                (Backend::Agent { .. }, true) if env.cli_full => "Agent: darf Dateien bearbeiten und Befehle ausführen (Einstellungen → KI).".into(),
                (Backend::Agent { .. }, true) => "Agent: darf Dateien in diesem Ordner bearbeiten. Befehle nur nach Freigabe in den Einstellungen.".into(),
                (Backend::Model(_), true) if env.auto.is_empty() => "Agent: jede Änderung und jeder Befehl erst nach „Übernehmen“. Dateiänderungen lassen sich mit Strg+Z rückgängig machen.".into(),
                (Backend::Model(_), true) => "Agent: freigegebene Änderungen laufen selbstständig, der Rest nach „Übernehmen“. Strg+Z macht rückgängig.".into(),
            };
            ui.add(egui::Label::new(egui::RichText::new(note).size(11.5).color(th.sub)).truncate());
        });
        out
    }
}

impl Drop for ChatView {
    fn drop(&mut self) {
        if let Some(h) = &self.cli {
            h.cancel();
        }
        self.persist();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ImgAction {
    Again,
    Variants,
    Bigger,
    Editor,
}

impl ChatView {
    /// Knopf unter einem Bild: Prompt und Vorlage stehen in den Einträgen davor
    fn image_action(&mut self, n: usize, a: ImgAction, env: &Env, ctx: &egui::Context, out: &mut Vec<AiOut>) {
        let Some(Item::Images(files)) = self.items.get(n) else { return };
        let first = files.first().cloned();
        let before = &self.items[..n];
        let prompt = before.iter().rev().find_map(|i| if let Item::User(t) = i { Some(t.clone()) } else { None });
        // Vorlage: Ref direkt nach der zugehörigen Nachricht
        let reference = before.iter().rev().take_while(|i| !matches!(i, Item::User(_))).find_map(|i| if let Item::Ref(v) = i { v.first().cloned() } else { None });
        match a {
            ImgAction::Editor => {
                if let Some(f) = first {
                    out.push(AiOut::Edit(f));
                }
            }
            ImgAction::Bigger => {
                if let Some(f) = first {
                    self.items.push(Item::User("Größer (KI-Hochskalieren 2×)".into()));
                    self.image_upscale(f, ctx, out);
                }
            }
            ImgAction::Again | ImgAction::Variants => {
                let Some(prompt) = prompt else { return };
                self.items.push(Item::User(prompt.clone()));
                match reference {
                    Some(r) => {
                        self.items.push(Item::Ref(vec![r.clone()]));
                        self.image_edit(prompt, r, env, ctx);
                    }
                    None => self.image_generate(prompt, if a == ImgAction::Variants { 4 } else { 1 }, env, ctx),
                }
            }
        }
    }
}

/// Knöpfe unter einem erzeugten Bild
fn image_actions(ui: &mut egui::Ui, th: &Theme, n: usize) -> Option<ImgAction> {
    let mut a = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (act, ic, label, tip) in [
            (ImgAction::Again, Ic::Refresh, "Nochmal", "Gleiche Beschreibung, neues Bild"),
            (ImgAction::Variants, Ic::Grid, "4 Varianten", "Vier Bilder auf einmal"),
            (ImgAction::Bigger, Ic::Plus, "Größer", "KI-Hochskalieren (2×, Real-ESRGAN)"),
            (ImgAction::Editor, Ic::Rename, "Im Editor", "Im Bildeditor öffnen"),
        ] {
            if ui::cmd_btn(ui, th, Some(ic), label, true, false).on_hover_text(tip).clicked() {
                a = Some(act);
            }
        }
        let _ = n;
    });
    ui.add_space(8.0);
    a
}

fn is_image(p: &Path) -> bool {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).is_some_and(|e| ["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "tga", "qoi"].contains(&e.as_str()))
}

fn unique_png(dir: &Path, stem: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}.png"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}).png"));
        n += 1;
    }
    p
}

/// Büroklammer: Dateien bzw. (Bild-Modus) ein Bild als Vorlage wählen
fn attach_button(ui: &mut egui::Ui, th: &Theme, attach: &mut Vec<PathBuf>, image_only: bool) {
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 36.0), Sense::click());
    if resp.hovered() {
        ui.painter().circle_filled(r.center(), 15.0, th.hover);
    }
    ui::draw(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Ic::Plus, if resp.hovered() { th.text } else { th.sub });
    let tip = if image_only { "Bild als Vorlage anhängen (oder hierher ziehen)" } else { "Dateien oder Bilder anhängen (oder hierher ziehen)" };
    if resp.on_hover_text(tip).clicked() {
        let d = rfd::FileDialog::new();
        let d = if image_only { d.add_filter("Bild", &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"]) } else { d };
        let picked: Vec<PathBuf> = if image_only { d.pick_file().into_iter().collect() } else { d.pick_files().unwrap_or_default() };
        for p in picked {
            if image_only {
                attach.clear();
            }
            if !attach.contains(&p) {
                attach.push(p);
            }
        }
    }
}

/// Anhänge über dem Eingabefeld (Bilder als kleine Vorschau, Klick auf × entfernt)
fn attach_chips(ui: &mut egui::Ui, th: &Theme, attach: &mut Vec<PathBuf>, thumbs: &std::cell::RefCell<HashMap<PathBuf, Option<egui::TextureHandle>>>) {
    if attach.is_empty() {
        return;
    }
    let mut remove = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (i, p) in attach.iter().enumerate() {
            let name = p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let short: String = if name.chars().count() > 20 { format!("{}…", name.chars().take(19).collect::<String>()) } else { name };
            let size = std::fs::metadata(p).map(|m| crate::summary::human(m.len())).unwrap_or_default();
            let g = ui.fonts(|f| f.layout_no_wrap(short.clone(), FontId::proportional(12.5), th.text));
            let w = 36.0 + 10.0 + g.size().x.max(48.0) + 34.0;
            let (r, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
            let pnt = ui.painter();
            pnt.rect(r, 9.0, if resp.hovered() { th.hover } else { th.panel }, Stroke::new(1.0, th.line));
            // Vorschau bzw. Datei-Symbol
            let ir = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(32.0, 32.0));
            match is_image(p).then(|| thumb(ui, p, thumbs)).flatten() {
                Some(t) => {
                    pnt.image(t.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    pnt.rect_filled(ir, 6.0, th.field);
                    ui::draw(pnt, Rect::from_center_size(ir.center(), vec2(16.0, 16.0)), Ic::Docs, th.sub);
                }
            }
            pnt.galley(pos2(ir.right() + 10.0, r.top() + 5.0), g, th.text);
            pnt.text(pos2(ir.right() + 10.0, r.bottom() - 5.0), egui::Align2::LEFT_BOTTOM, size, FontId::proportional(10.5), th.sub);
            // runder Entfernen-Knopf
            let xr = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(20.0, 20.0));
            let xresp = ui.interact(xr, ui.id().with(("attach_x", i)), Sense::click());
            let hot = xresp.hovered();
            pnt.circle_filled(xr.center(), 10.0, if hot { th.danger } else { th.field });
            let c = if hot { Color32::WHITE } else { th.sub };
            let d = 3.5;
            pnt.line_segment([xr.center() + vec2(-d, -d), xr.center() + vec2(d, d)], Stroke::new(1.6, c));
            pnt.line_segment([xr.center() + vec2(-d, d), xr.center() + vec2(d, -d)], Stroke::new(1.6, c));
            if hot {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if xresp.on_hover_text("Entfernen").clicked() {
                remove = Some(i);
            }
            resp.on_hover_text(p.display().to_string());
        }
    });
    if let Some(i) = remove {
        attach.remove(i);
    }
    ui.add_space(4.0);
}

/// angehängte Dateien im Verlauf (rechts, klein)
fn ref_strip(ui: &mut egui::Ui, th: &Theme, files: &[PathBuf], thumbs: &std::cell::RefCell<HashMap<PathBuf, Option<egui::TextureHandle>>>) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        for p in files {
            if is_image(p) {
                if let Some(t) = thumb(ui, p, thumbs) {
                    let k = 96.0 / t.size_vec2().max_elem();
                    ui.add(egui::Image::new((t.id(), t.size_vec2() * k)).rounding(8.0)).on_hover_text(p.display().to_string());
                    continue;
                }
            }
            egui::Frame::none().fill(th.panel).stroke(Stroke::new(1.0, th.line)).rounding(8.0).inner_margin(Margin::symmetric(8.0, 5.0)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    ui::draw(ui.painter(), ir, Ic::Docs, th.sub);
                    ui.label(egui::RichText::new(p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())).size(12.0));
                });
            })
            .response
            .on_hover_text(p.display().to_string());
        }
    });
    ui.add_space(8.0);
}

fn thumb(ui: &egui::Ui, p: &Path, thumbs: &std::cell::RefCell<HashMap<PathBuf, Option<egui::TextureHandle>>>) -> Option<egui::TextureHandle> {
    thumbs
        .borrow_mut()
        .entry(p.to_path_buf())
        .or_insert_with(|| {
            image::open(p).ok().map(|i| {
                let t = i.thumbnail(512, 512).to_rgba8();
                ui.ctx().load_texture(p.display().to_string(), egui::ColorImage::from_rgba_unmultiplied([t.width() as usize, t.height() as usize], t.as_raw()), egui::TextureOptions::LINEAR)
            })
        })
        .clone()
}

/// Klingt die Eingabe nach „ändere das vorhandene Bild“ statt „male ein neues“?
fn wants_edit(prompt: &str) -> bool {
    let t = format!(" {} ", prompt.to_lowercase());
    const NEW: &[&str] = &["neues bild", "neue bild", "ein bild von", "erzeuge ein", "erstelle ein", "generiere ein", "male ein", "zeichne ein", "new image", "generate a", "create a"];
    if NEW.iter().any(|w| t.contains(w)) {
        return false;
    }
    const EDIT: &[&str] = &[
        "bearbeit", "beararbeit", "änder", "aender", "verändere", "mach das bild", "mache das bild", "mach es", "mache es", "mach ihn", "mach sie", " mach den ", " mache den ", " mach die ", " mache die ", " mach das ", " mache das ",
        "füge", "fuege", "hinzu", "entfern", "lösch", "ersetz", "tausch", "färbe", "faerbe", "anstatt", "stattdessen",
        "dieses bild", "das bild", "dem bild", "im bild", "auf dem bild", "darauf", "umwandeln", "verwandle",
        "edit", "change", "modify", "make it", "make the", "add ", "remove", "replace", "turn it", "turn the", "instead",
    ];
    EDIT.iter().any(|w| t.contains(w))
}

/// Pfade aus dem Ergebnis von generate_image („Erzeugt mit …: a.png, b.png“)
fn image_paths(text: &str) -> Vec<PathBuf> {
    let list = text.split_once(": ").map_or(text, |x| x.1);
    list.split(", ").map(|p| PathBuf::from(p.trim())).filter(|p| p.is_file()).collect()
}

/// Bilder als Vorschau (Klick → in neuem Tab ansehen); gibt das angeklickte zurück
fn images_grid(ui: &mut egui::Ui, th: &Theme, files: &[PathBuf], thumbs: &std::cell::RefCell<HashMap<PathBuf, Option<egui::TextureHandle>>>, col_w: f32) -> Option<PathBuf> {
    let mut open = None;
    ui.horizontal_wrapped(|ui| {
        for f in files {
            let mut cache = thumbs.borrow_mut();
            let tex = cache.entry(f.clone()).or_insert_with(|| {
                image::open(f).ok().map(|i| {
                    let t = i.thumbnail(512, 512).to_rgba8();
                    ui.ctx().load_texture(f.display().to_string(), egui::ColorImage::from_rgba_unmultiplied([t.width() as usize, t.height() as usize], t.as_raw()), egui::TextureOptions::LINEAR)
                })
            });
            match tex {
                Some(t) => {
                    let size = t.size_vec2() * (320.0f32.min(col_w - 40.0) / t.size_vec2().max_elem());
                    let r = ui.add(egui::Image::new((t.id(), size)).rounding(8.0).sense(Sense::click())).on_hover_text(format!("{}\nKlick: in neuem Tab ansehen", f.display()));
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if r.clicked() {
                        open = Some(f.clone());
                    }
                }
                None => {
                    ui.label(egui::RichText::new(format!("{} (nicht mehr da)", f.display())).size(12.0).color(th.sub));
                }
            }
        }
    });
    ui.add_space(6.0);
    open
}

/// gespeicherter Schlüssel eines Bildmodells
fn img_key(c: &crate::localllm::ImgChoice) -> String {
    match c {
        crate::localllm::ImgChoice::Catalog(id) => id.to_string(),
        crate::localllm::ImgChoice::File(p) => p.display().to_string(),
    }
}

fn rand_u16() -> u16 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(now());
    h.finish() as u16
}

/// Anzeigename einer Agent-Modellstufe.
fn agent_model_label(m: &str) -> String {
    match m {
        "" => "Standardmodell".into(),
        "opus" => "Opus".into(),
        "sonnet" => "Sonnet".into(),
        "haiku" => "Haiku".into(),
        "fable" => "Fable".into(),
        other => other.to_string(),
    }
}

fn agent_model_hint(m: &str) -> &'static str {
    match m {
        "" => "wie im Agenten eingestellt",
        "opus" => "am stärksten",
        "sonnet" => "schnell und stark",
        "haiku" => "am schnellsten, günstig",
        _ => "",
    }
}

// ───────────────────────────── Bausteine ─────────────────────────────

/// KI-Kennzeichen: nur ein schlichtes ✦ in Textfarbe – kein Kreis, kein Farbfeld.
fn avatar(ui: &mut egui::Ui, th: &Theme, size: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui::draw(ui.painter(), Rect::from_center_size(r.center(), vec2(size * 0.62, size * 0.62)), Ic::Spark, th.text);
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Agent,
    Assistant,
    Chat,
    Image,
}

/// Umschalter Agent / Assistent / Chat / Bild.
fn mode_switch(ui: &mut egui::Ui, th: &Theme, cur: &mut Mode, enabled: bool, width: Option<f32>) {
    let font = FontId::proportional(13.0);
    let opts = [(Mode::Agent, "Agent"), (Mode::Assistant, "Assistent"), (Mode::Chat, "Chat"), (Mode::Image, "Bild")];
    let ws: Vec<f32> = match width {
        Some(w) => vec![(w - 6.0) / opts.len() as f32; opts.len()],
        None => opts.iter().map(|(_, l)| ui.fonts(|f| f.layout_no_wrap(l.to_string(), font.clone(), th.text)).size().x + 24.0).collect(),
    };
    let (rect, _) = ui.allocate_exact_size(vec2(ws.iter().sum::<f32>() + 6.0, 30.0), Sense::hover());
    ui.painter().rect(rect, 15.0, th.field, Stroke::new(1.0, th.line));
    let mut x = rect.left() + 3.0;
    for (i, (v, l)) in opts.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(ws[i], 24.0));
        let resp = ui.interact(r, ui.id().with(("aimode", i)), if enabled { Sense::click() } else { Sense::hover() });
        let sel = *cur == *v;
        if sel {
            ui.painter().rect_filled(r, 12.0, th.accent);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 12.0, th.hover);
        }
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *l, font.clone(), if sel { th.on_accent() } else { th.text });
        let tip = match v {
            Mode::Agent => "Darf Dateien bearbeiten, erstellen, verschieben und Befehle im Hintergrund ausführen",
            Mode::Assistant => "Sieht sich Ordner und Dateien an und antwortet – ändert nichts",
            Mode::Chat => "Normaler KI-Chat: antwortet nur auf deine Nachrichten, ohne Zugriff auf Explorer und Dateien",
            Mode::Image => "Bilder erzeugen mit einem lokalen Bildmodell – oben nur Bildmodelle",
        };
        if resp.on_hover_text(tip).clicked() {
            *cur = *v;
        }
        x += ws[i];
    }
}

fn chip(ui: &mut egui::Ui, th: &Theme, text: &str) -> egui::Response {
    let g = ui.fonts(|f| f.layout_no_wrap(text.to_string(), FontId::proportional(13.0), th.text));
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 28.0, 32.0), Sense::click());
    ui.painter().rect(r, 16.0, if resp.hovered() { th.hover } else { th.panel }, Stroke::new(1.0, th.line));
    ui.painter().galley(pos2(r.left() + 14.0, r.center().y - g.size().y / 2.0), g, th.text);
    ui.add_space(4.0);
    resp
}

fn centered_card(ui: &mut egui::Ui, th: &Theme, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(24.0);
    let w = (ui.available_width() - 32.0).clamp(200.0, 520.0);
    ui.vertical_centered(|ui| {
        egui::Frame::none().fill(th.panel).stroke(Stroke::new(1.0, th.line)).rounding(Rounding::same(12.0)).inner_margin(Margin::same(18.0)).show(ui, |ui| {
            ui.set_width(w - 36.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), add);
        });
    });
}

fn user_bubble(ui: &mut egui::Ui, th: &Theme, text: &str) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        egui::Frame::none().fill(th.accent.gamma_multiply(0.16)).rounding(Rounding { nw: 14.0, ne: 14.0, sw: 14.0, se: 4.0 }).inner_margin(Margin::symmetric(14.0, 9.0)).show(ui, |ui| {
            ui.set_max_width(ui.available_width() * 0.78);
            ui.add(egui::Label::new(egui::RichText::new(text).size(14.0)).wrap());
        });
    });
    ui.add_space(10.0);
}

/// Antwort mit einfacher Formatierung: Codeblöcke (```), Überschriften (#), Aufzählungen.
fn assistant_msg(ui: &mut egui::Ui, th: &Theme, text: &str) {
    ui.horizontal_top(|ui| {
        avatar(ui, th, 24.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (i, part) in text.split("```").enumerate() {
                if i % 2 == 1 {
                    let (lang, code) = match part.split_once('\n') {
                        Some((first, rest)) if first.trim().chars().all(|c| c.is_alphanumeric() || c == '+' || c == '#') => (first.trim(), rest),
                        _ => ("", part),
                    };
                    code_block(ui, th, lang, code.trim_end(), i);
                } else {
                    for line in part.trim_matches('\n').lines() {
                        let t = line.trim_end();
                        if let Some(h) = t.strip_prefix("### ").or_else(|| t.strip_prefix("## ")).or_else(|| t.strip_prefix("# ")) {
                            ui.label(egui::RichText::new(h).size(15.0).strong());
                        } else if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
                            ui.horizontal_top(|ui| {
                                ui.label(egui::RichText::new("•").color(th.accent));
                                ui.add(egui::Label::new(egui::RichText::new(b.replace("**", "")).size(14.0)).wrap());
                            });
                        } else if t.is_empty() {
                            ui.add_space(4.0);
                        } else {
                            ui.add(egui::Label::new(egui::RichText::new(t.replace("**", "")).size(14.0)).wrap());
                        }
                    }
                }
            }
        });
    });
    ui.add_space(10.0);
}

/// Codeblock mit Kopfzeile (Sprache, „Kopieren“).
fn code_block(ui: &mut egui::Ui, th: &Theme, lang: &str, code: &str, n: usize) {
    let id = ui.id().with(("code", n, code.len()));
    egui::Frame::none().fill(th.ed_bg).stroke(Stroke::new(1.0, th.line)).rounding(Rounding::same(8.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Frame::none().fill(th.panel).rounding(Rounding { nw: 8.0, ne: 8.0, sw: 0.0, se: 0.0 }).inner_margin(Margin::symmetric(10.0, 3.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(if lang.is_empty() { "Code" } else { lang }).size(11.5).color(th.sub));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let copied = ui.data(|d| d.get_temp::<f64>(id)).is_some_and(|t| ui.input(|i| i.time) - t < 1.5);
                    let (ic, label) = if copied { (Ic::Check, "Kopiert") } else { (Ic::Copy, "Kopieren") };
                    let g = ui.fonts(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(11.5), th.sub));
                    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 26.0, 20.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 4.0, th.hover);
                    }
                    ui::draw(ui.painter(), Rect::from_center_size(pos2(r.left() + 10.0, r.center().y), vec2(12.0, 12.0)), ic, th.sub);
                    ui.painter().galley(pos2(r.left() + 20.0, r.center().y - g.size().y / 2.0), g, th.sub);
                    if resp.on_hover_text("Code in die Zwischenablage kopieren").clicked() {
                        ui.ctx().copy_text(code.to_string());
                        let now = ui.input(|i| i.time);
                        ui.data_mut(|d| d.insert_temp(id, now));
                    }
                    if copied {
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                    }
                });
            });
        });
        egui::Frame::none().inner_margin(Margin::same(10.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(egui::RichText::new(code).monospace().size(12.5).color(th.ed_text)).wrap().selectable(true));
        });
    });
}

/// Schritt eines Agenten: kompakte Zeile mit Werkzeug, Detail und Ergebnis.
fn step_row(ui: &mut egui::Ui, th: &Theme, tool: &str, detail: &str, res: Option<&(String, bool)>) {
    let ic = match tool.to_lowercase().as_str() {
        t if t.contains("read") || t.contains("view") => Ic::Docs,
        t if t.contains("edit") || t.contains("write") || t.contains("datei") => Ic::Rename,
        t if t.contains("bash") || t.contains("befehl") || t.contains("shell") => Ic::Terminal,
        t if t.contains("grep") || t.contains("glob") || t.contains("search") => Ic::Search,
        _ => Ic::Tools,
    };
    let col = match res {
        Some((_, true)) => th.danger,
        Some(_) => th.sub,
        None => th.accent,
    };
    egui::Frame::none().fill(th.panel).stroke(Stroke::new(1.0, th.line)).rounding(Rounding::same(8.0)).inner_margin(Margin::symmetric(10.0, 5.0)).show(ui, |ui| {
        ui.set_width(ui.available_width() - 30.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
            ui::draw(ui.painter(), r, ic, col);
            ui.label(egui::RichText::new(tool).size(12.5).strong());
            ui.add(egui::Label::new(egui::RichText::new(detail).size(12.0).color(th.sub).monospace()).truncate());
        });
        if let Some((t, err)) = res {
            if !t.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(t).size(11.5).color(if *err { th.danger } else { th.sub })).truncate());
            }
        }
    });
    ui.add_space(4.0);
}

/// Karte für eine vorgeschlagene Änderung. Rückgabe: Entscheidung (true = Übernehmen).
fn call_card(ui: &mut egui::Ui, th: &Theme, c: &CallView, active: bool) -> Option<bool> {
    let mut decision = None;
    let ok_col = crate::gitui::status_color(crate::git::St::Added, th);
    let (col, status) = match c.state {
        CallState::Ask => (th.accent, "wartet auf deine Bestätigung"),
        CallState::Running if c.auto => (th.sub, "läuft automatisch …"),
        CallState::Running => (th.sub, "läuft …"),
        CallState::Done if c.auto => (ok_col, "erledigt (automatisch)"),
        CallState::Done => (ok_col, "erledigt"),
        CallState::Failed => (th.danger, "fehlgeschlagen"),
        CallState::Rejected => (th.sub, "abgelehnt"),
    };
    let ic = match c.call.name.as_str() {
        "move" => Ic::Cut,
        "copy" => Ic::Copy,
        "rename" | "edit_text_file" | "write_text_file" => Ic::Rename,
        "move_to_trash" => Ic::Trash,
        "create_folder" => Ic::NewFolder,
        "create_text_file" => Ic::Docs,
        "open_file" => Ic::Open,
        "find_files" => Ic::Search,
        "run_shell" | "run_in_terminal" => Ic::Terminal,
        _ => Ic::List,
    };
    if !c.needs_ok {
        // Lesen/Navigieren: nur eine kompakte Zeile
        let res = c.result.as_ref().map(|(t, e)| (if *e { t.clone() } else { String::new() }, *e));
        step_row(ui, th, &c.title, &c.lines.join("  "), res.as_ref());
        return None;
    }
    egui::Frame::none()
        .fill(th.panel)
        .stroke(Stroke::new(if c.state == CallState::Ask { 1.5 } else { 1.0 }, if c.state == CallState::Ask { th.accent } else { th.line }))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(14.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width() - 30.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                ui::draw(ui.painter(), r, ic, col);
                ui.label(egui::RichText::new(&c.title).strong());
                ui.label(egui::RichText::new(status).size(12.0).color(col));
            });
            for l in c.lines.iter().take(14) {
                let color = if l.starts_with("+ ") {
                    ok_col
                } else if l.starts_with("− ") {
                    th.danger
                } else {
                    th.sub
                };
                ui.add(egui::Label::new(egui::RichText::new(l).size(12.5).color(color).monospace()).truncate());
            }
            if c.lines.len() > 14 {
                ui.label(egui::RichText::new(format!("… und {} weitere", c.lines.len() - 14)).size(12.0).color(th.sub));
            }
            if c.call.name == "run_shell" {
                // Ausgabe des Befehls: live während er läuft, danach das Ergebnis
                let live = if c.state == CallState::Running { ai::live_output(&c.call.id) } else { None };
                let shown = c.result.as_ref().map(|r| r.0.clone()).or(live);
                if let Some(t) = shown {
                    shell_output(ui, th, &t, c.result.as_ref().is_some_and(|r| r.1), c.call.id.as_str());
                } else if c.state == CallState::Running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("wartet auf Ausgabe …").size(12.0).color(th.sub));
                    });
                }
            } else if let Some((t, err)) = &c.result {
                let short: String = t.chars().take(300).collect();
                ui.label(egui::RichText::new(short).size(12.0).color(if *err { th.danger } else { th.sub }));
            }
            if c.state == CallState::Ask && active {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("Übernehmen").color(th.on_accent()).strong()).fill(th.accent).rounding(Rounding::same(8.0))).clicked() {
                        decision = Some(true);
                    }
                    if ui.add(egui::Button::new("Ablehnen").rounding(Rounding::same(8.0))).clicked() {
                        decision = Some(false);
                    }
                });
            }
        });
    ui.add_space(6.0);
    decision
}

/// Ausgabe eines Hintergrund-Befehls: Status in der ersten Zeile, darunter die Ausgabe (scrollbar, Ende sichtbar).
fn shell_output(ui: &mut egui::Ui, th: &Theme, text: &str, err: bool, id: &str) {
    let (head, body) = match text.split_once('\n') {
        Some((h, b)) if h.starts_with("Exit-Code") || h.starts_with("Nach ") => (Some(h), b),
        _ => (None, text),
    };
    if let Some(h) = head {
        let ok_col = crate::gitui::status_color(crate::git::St::Added, th);
        ui.label(egui::RichText::new(h).size(12.0).strong().color(if err { th.danger } else { ok_col }));
    }
    egui::Frame::none().fill(th.ed_bg).stroke(Stroke::new(1.0, th.line)).rounding(Rounding::same(6.0)).inner_margin(Margin::same(8.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt(("shell_out", id)).max_height(220.0).stick_to_bottom(true).auto_shrink([false, true]).show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(body.trim_end()).monospace().size(12.0).color(th.ed_text)).wrap().selectable(true));
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_or_new() {
        for p in ["beararbeite das bild so dass der himmel rot ist", "mach den Hintergrund blau", "füge einen Hut hinzu", "entferne die Person", "ändere die Farbe des Autos", "make it a watercolor", "Ersetze den Hund durch eine Katze"] {
            assert!(wants_edit(p), "{p}");
        }
        for p in ["ein Hund als Astronaut", "Sonnenuntergang am Meer, Ölgemälde", "erstelle ein neues Bild von einer Burg", "a cat in space"] {
            assert!(!wants_edit(p), "{p}");
        }
    }

    #[test]
    fn history_sanitize_and_roundtrip() {
        // offene Werkzeugaufrufe und unbeantwortete Nachrichten werden abgeschnitten
        let mut h = vec![
            json!({ "role": "user", "content": "a" }),
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "ok" }] }),
            json!({ "role": "user", "content": "b" }),
            json!({ "role": "assistant", "content": [{ "type": "tool_use", "id": "1", "name": "list_folder", "input": {} }] }),
        ];
        sanitize(&mut h);
        assert_eq!(h.len(), 2);
        let mut c = ChatView::new(PathBuf::from("."), "model:x".into(), true);
        c.items = vec![Item::User("Hallo Welt".into()), Item::Assistant("Hi".into()), Item::Step("Read".into(), "a.txt".into(), Some(("ok".into(), false)))];
        c.history = h;
        c.history_kind = Some(Kind::Anthropic);
        let j = c.to_json();
        let mut d = ChatView::new(PathBuf::from("."), String::new(), false);
        d.load(&j);
        assert_eq!(d.items.len(), 3);
        assert_eq!(d.history.len(), 2);
        assert_eq!(d.auto_title(), "Hallo Welt");
        assert!(d.history_kind == Some(Kind::Anthropic));
        // Testobjekte nicht auf die Platte schreiben
        c.items.clear();
        d.items.clear();
    }
}
