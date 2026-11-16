use crate::sidebar::{Entry, SidebarCfg};
use crate::keymap::Keymap;
use crate::themes::{self, Palette, Store};
use serde_json::{json, Value};
use crate::ai::{self, ModelProfile};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

#[derive(Clone, PartialEq)]
pub struct Settings {
    /// Windows/macOS: echte System-Icons statt der mitgelieferten Standard-Icons (Material Icon Theme) nutzen.
    pub sys_icons: bool,
    /// Icon-Theme: „material“, „catppuccin“, „vscode-icons“
    pub icon_set: String,
    /// Bei Sortierung nach Name/Typ Ordner vor Dateien anzeigen (bei Datum/Größe wird nie gruppiert).
    pub folders_first: bool,
    /// Standardansicht neuer Fenster: 0 = Details, 1 = Liste, 2 = Raster, 3 = Große Symbole
    pub view_mode: u8,
    pub show_hidden: bool,
    /// Animationen (Tabs, Seitenleiste, Schalter, Andock-Vorschau)
    pub animations: bool,
    /// Seitenleiste sichtbar
    pub show_sidebar: bool,
    pub show_system: bool,
    /// 0 = System, 1 = Hell, 2 = Dunkel
    pub theme: u8,
    /// Farbthemes für den hellen bzw. dunklen Modus (IDs aus `themes::Store`)
    pub theme_light: String,
    pub theme_dark: String,
    /// Seitenleiste: gefundene Git-Repositories anzeigen
    pub show_git: bool,
    /// Suchindex beim Start für alle Laufwerke aufbauen/aktualisieren
    pub auto_index: bool,
    /// Seitenleiste: zuletzt verwendete Ordner (Verlauf wird nur bei eingeschalteter Option gespeichert)
    pub show_recent: bool,
    /// Ordnergrößen in der Detailansicht berechnen
    pub folder_sizes: bool,
    /// Git-Status in der Dateiliste markieren
    pub git_marks: bool,
    /// Terminal: Programm der Shell (leer = erste gefundene)
    pub term_shell: String,
    /// Editoren: eigener Programmpfad je Editor (fehlt = automatisch erkennen)
    pub editor_paths: BTreeMap<String, String>,
    /// Editoren ein-/ausgeschaltet (fehlt = Standard des Editors)
    pub editor_on: BTreeMap<String, bool>,
    /// selbst hinzugefügte Editoren: (Name, Programm)
    pub custom_editors: Vec<(String, String)>,
    /// Hauptschalter für alle KI-Funktionen (Agent-Tabs, KI-Assistent)
    pub ai_enabled: bool,
    /// KI-Agenten für die Kommandozeile (Claude Code …) ein-/ausgeschaltet (fehlt = an)
    pub agent_on: BTreeMap<String, bool>,
    /// Eigene Modelle (Anthropic-API oder OpenAI-kompatibel); Schlüssel liegen getrennt verschlüsselt
    pub ai_models: Vec<ModelProfile>,
    /// Standard für „KI-Assistent“ bzw. „KI-Agent“: „agent:<schlüssel>“ oder „model:<id>“
    pub ai_assist: String,
    pub ai_agent: String,
    /// Agent darf diese Kategorien ohne Nachfrage ausführen (ai::CATEGORIES); leer = immer fragen
    pub ai_auto: BTreeSet<String>,
    /// Kommandozeilen-Agenten im Agent-Modus zusätzlich Befehle ohne Nachfrage erlauben
    pub ai_cli_full: bool,
    /// gewähltes Modell je Kommandozeilen-Agent („“ = Standard des Agenten)
    pub agent_model: BTreeMap<String, String>,
    /// Schnellleiste: Knöpfe in Reihenfolge, sichtbar ja/nein
    pub quickbar: Vec<(String, bool)>,
    /// Vorschau-Panel neben der Dateiliste
    pub show_preview: bool,
    /// Terminal: eigene Vorschläge (Befehle, Pfade, Dateinamen) beim Tippen
    pub term_complete: bool,
    /// Fenster, Tabs, Kacheln und Vorschau beim Start wiederherstellen
    pub restore_session: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sys_icons: false,
            icon_set: "material".into(),
            folders_first: true,
            view_mode: 0,
            show_hidden: false,
            animations: true,
            show_sidebar: true,
            show_system: false,
            theme: 0,
            theme_light: themes::DEFAULT_LIGHT.into(),
            theme_dark: themes::DEFAULT_DARK.into(),
            show_git: true,
            auto_index: false,
            show_recent: true,
            folder_sizes: true,
            git_marks: true,
            term_shell: String::new(),
            editor_paths: BTreeMap::new(),
            editor_on: BTreeMap::new(),
            custom_editors: Vec::new(),
            ai_enabled: true,
            agent_on: BTreeMap::new(),
            ai_models: ai::default_profiles(),
            ai_assist: "agent:claude".into(),
            ai_agent: "agent:claude".into(),
            ai_auto: BTreeSet::new(),
            ai_cli_full: false,
            agent_model: BTreeMap::new(),
            quickbar: crate::quickbar::defaults(),
            show_preview: false,
            term_complete: true,
            restore_session: true,
        }
    }
}

fn file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("foldex")
        .join("settings.txt")
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Settings::default();
        // Werte älterer Versionen (fester Anbieter statt Modell-Profilen) – werden unten übernommen
        let (mut old_provider, mut old_model, mut old_base) = (None::<String>, BTreeMap::<String, String>::new(), BTreeMap::<String, String>::new());
        let mut profiles: Vec<ModelProfile> = Vec::new();
        if let Ok(text) = fs::read_to_string(file()) {
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else {
                    continue;
                };
                let b = v.trim() == "1";
                match k.trim() {
                    // neuer Schlüsselname: alte Einstellungen („sys_icons“) werden bewusst ignoriert,
                    // damit überall die mitgelieferten Standard-Icons aktiv sind
                    "system_icons" => s.sys_icons = b,
                    "folders_first" => s.folders_first = b,
                    "view_mode" => s.view_mode = v.trim().parse::<u8>().unwrap_or(0).min(3),
                    "show_hidden" => s.show_hidden = b,
                    "animations" => s.animations = b,
                    "show_sidebar" => s.show_sidebar = b,
                    "show_system" => s.show_system = b,
                    "show_git" => s.show_git = b,
                    "auto_index" => s.auto_index = b,
                    "theme" => s.theme = v.trim().parse::<u8>().unwrap_or(0).min(2),
                    "show_recent" => s.show_recent = b,
                    "folder_sizes" => s.folder_sizes = b,
                    "git_marks" => s.git_marks = b,
                    "term_shell" => s.term_shell = v.trim().to_string(),
                    "icon_set" if !v.trim().is_empty() => s.icon_set = v.trim().to_string(),
                    // ältere Versionen: nur VS Code und Zed
                    "editor_vscode" | "editor_zed" if !v.trim().is_empty() => {
                        s.editor_paths.insert(k.trim()["editor_".len()..].to_string(), v.trim().to_string());
                    }
                    "ai_enabled" => s.ai_enabled = b,
                    "ai_provider" if !v.trim().is_empty() => old_provider = Some(v.trim().to_string()),
                    "ai_profile" => profiles.extend(ModelProfile::decode(v)),
                    "ai_assist" if !v.trim().is_empty() => s.ai_assist = v.trim().to_string(),
                    "ai_agent" if !v.trim().is_empty() => s.ai_agent = v.trim().to_string(),
                    "ai_auto" => s.ai_auto = v.split(',').map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect(),
                    "ai_cli_full" => s.ai_cli_full = b,
                    "quickbar" => s.quickbar = crate::quickbar::decode(v),
                    "show_preview" => s.show_preview = b,
                    "term_complete" => s.term_complete = b,
                    "restore_session" => s.restore_session = b,
                    k if k.starts_with("agent_model.") => {
                        s.agent_model.insert(k["agent_model.".len()..].to_string(), v.trim().to_string());
                    }
                    "custom_editor" => {
                        if let Some((n, p)) = v.split_once('|') {
                            s.custom_editors.push((n.trim().to_string(), p.trim().to_string()));
                        }
                    }
                    k if k.starts_with("editor.") => {
                        s.editor_on.insert(k["editor.".len()..].to_string(), b);
                    }
                    k if k.starts_with("editor_path.") && !v.trim().is_empty() => {
                        s.editor_paths.insert(k["editor_path.".len()..].to_string(), v.trim().to_string());
                    }
                    k if k.starts_with("agent.") => {
                        s.agent_on.insert(k["agent.".len()..].to_string(), b);
                    }
                    k if k.starts_with("ai_model.") && !v.trim().is_empty() => {
                        old_model.insert(k["ai_model.".len()..].to_string(), v.trim().to_string());
                    }
                    k if k.starts_with("ai_base.") && !v.trim().is_empty() => {
                        old_base.insert(k["ai_base.".len()..].to_string(), v.trim().to_string());
                    }
                    "theme_light" if !v.trim().is_empty() => s.theme_light = v.trim().to_string(),
                    "theme_dark" if !v.trim().is_empty() => s.theme_dark = v.trim().to_string(),
                    _ => {}
                }
            }
        }
        if !profiles.is_empty() {
            s.ai_models = profiles;
        } else {
            // ältere Version: Modell/Adresse je Anbieter in die Standard-Profile übernehmen
            for p in &mut s.ai_models {
                if let Some(m) = old_model.get(&p.id) {
                    p.model = m.clone();
                }
                if let Some(u) = old_base.get(&p.id) {
                    p.url = u.clone();
                }
            }
            if old_provider.as_deref() == Some("openai") || old_model.contains_key("openai") {
                s.ai_models.push(ModelProfile {
                    id: "openai".into(),
                    name: "OpenAI-kompatibel".into(),
                    kind: ai::Kind::OpenAi,
                    url: old_base.get("openai").cloned().unwrap_or_else(|| "https://api.openai.com/v1".into()),
                    model: old_model.get("openai").cloned().unwrap_or_default(),
                });
            }
            if let Some(p) = old_provider {
                s.ai_assist = format!("model:{p}");
                s.ai_agent = format!("model:{p}");
            }
        }
        s
    }

    pub fn save(&self) {
        let n = |b: bool| if b { 1 } else { 0 };
        let mut extra = String::new();
        for (k, v) in &self.editor_on {
            extra.push_str(&format!("editor.{k}={}\n", n(*v)));
        }
        for (k, v) in &self.editor_paths {
            extra.push_str(&format!("editor_path.{k}={}\n", one_line(v)));
        }
        for (name, path) in &self.custom_editors {
            extra.push_str(&format!("custom_editor={}|{}\n", one_line(&name.replace('|', " ")), one_line(path)));
        }
        for (k, v) in &self.agent_on {
            extra.push_str(&format!("agent.{k}={}\n", n(*v)));
        }
        for p in &self.ai_models {
            extra.push_str(&format!("ai_profile={}\n", p.encode()));
        }
        for (k, v) in &self.agent_model {
            extra.push_str(&format!("agent_model.{k}={}\n", one_line(v)));
        }
        extra.push_str(&format!(
            "ai_assist={}\nai_agent={}\nai_auto={}\nai_cli_full={}\nquickbar={}\nshow_preview={}\nterm_complete={}\nrestore_session={}\nicon_set={}\n",
            one_line(&self.ai_assist),
            one_line(&self.ai_agent),
            self.ai_auto.iter().cloned().collect::<Vec<_>>().join(","),
            n(self.ai_cli_full),
            crate::quickbar::encode(&self.quickbar),
            n(self.show_preview),
            n(self.term_complete),
            n(self.restore_session),
            one_line(&self.icon_set)
        ));
        let _ = themes::write_atomic(
            &file(),
            format!(
                "system_icons={}\nfolders_first={}\nview_mode={}\nshow_hidden={}\nanimations={}\nshow_sidebar={}\nshow_system={}\nshow_git={}\ntheme={}\ntheme_light={}\ntheme_dark={}\nauto_index={}\nshow_recent={}\nfolder_sizes={}\ngit_marks={}\nterm_shell={}\nai_enabled={}\n{}",
                n(self.sys_icons),
                n(self.folders_first),
                self.view_mode,
                n(self.show_hidden),
                n(self.animations),
                n(self.show_sidebar),
                n(self.show_system),
                n(self.show_git),
                self.theme,
                self.theme_light,
                self.theme_dark,
                n(self.auto_index),
                n(self.show_recent),
                n(self.folder_sizes),
                n(self.git_marks),
                one_line(&self.term_shell),
                n(self.ai_enabled),
                extra
            )
            .as_bytes(),
        );
    }
}

fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], "")
}

// ───────────────────────────── Export / Import ─────────────────────────────
//
// Versioniertes JSON: { "format": "foldex-settings", "version": 1, "appVersion", "exported",
//   "settings": { … }, "sidebar": { "showDrives", "pins": [ { "label", "path" } ] }, "customThemes": [ Theme-JSON ] }
// Unbekannte Einträge werden ignoriert (mit Hinweis), damit Exporte neuerer Versionen soweit wie möglich lesbar bleiben.

pub const EXPORT_FORMAT: &str = "foldex-settings";
pub const EXPORT_VERSION: u64 = 1;

const MODES: [&str; 3] = ["system", "light", "dark"];
const VIEWS: [&str; 4] = ["details", "list", "grid", "icons"];

pub fn export(st: &Settings, side: &SidebarCfg, store: &Store, keys: &Keymap) -> Value {
    json!({
        "format": EXPORT_FORMAT,
        "version": EXPORT_VERSION,
        "appVersion": env!("CARGO_PKG_VERSION"),
        "exported": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        "settings": {
            "colorMode": MODES[st.theme.min(2) as usize],
            "lightTheme": st.theme_light,
            "darkTheme": st.theme_dark,
            "animations": st.animations,
            "defaultView": VIEWS[st.view_mode.min(3) as usize],
            "foldersFirst": st.folders_first,
            "showHidden": st.show_hidden,
            "showSystemFiles": st.show_system,
            "systemIcons": st.sys_icons,
            "iconTheme": st.icon_set,
            "autoIndex": st.auto_index,
            "showSidebar": st.show_sidebar,
            "showGitRepositories": st.show_git,
            "showRecentFolders": st.show_recent,
            "folderSizes": st.folder_sizes,
            "gitStatusMarks": st.git_marks,
            "terminalShell": st.term_shell,
            "aiEnabled": st.ai_enabled,
            "showPreview": st.show_preview,
            "terminalCompletion": st.term_complete,
            "restoreSession": st.restore_session
        },
        // Editoren: alle bekannten mit Zustand und eigenem Pfad
        "editors": crate::editors::EDITORS.iter().map(|d| (d.key.to_string(), json!({
            "enabled": st.editor_on.get(d.key).copied().unwrap_or(d.default_on),
            "path": st.editor_paths.get(d.key).cloned().unwrap_or_default()
        }))).collect::<serde_json::Map<String, Value>>(),
        "customEditors": st.custom_editors.iter().map(|(n, p)| json!({ "name": n, "path": p })).collect::<Vec<_>>(),
        "agents": st.agent_on.iter().map(|(k, v)| (k.clone(), Value::Bool(*v))).collect::<serde_json::Map<String, Value>>(),
        // KI ohne API-Schlüssel (die werden nie exportiert)
        "ai": {
            "models": st.ai_models.iter().map(|p| json!({ "id": p.id, "name": p.name, "api": p.kind.key(), "url": p.url, "model": p.model })).collect::<Vec<_>>(),
            "assistant": st.ai_assist,
            "agent": st.ai_agent,
            "autoApprove": st.ai_auto.iter().cloned().collect::<Vec<_>>(),
            "cliFullAccess": st.ai_cli_full,
            "agentModels": st.agent_model
        },
        "quickbar": st.quickbar.iter().map(|(id, on)| json!({ "id": id, "visible": on })).collect::<Vec<_>>(),
        "shortcuts": keys.to_json(),
        "sidebar": {
            "showDrives": side.show_pc,
            "pins": side.pins.iter().map(|e| json!({ "label": e.label, "path": e.path.to_string_lossy() })).collect::<Vec<_>>()
        },
        "customThemes": store.custom.iter().map(themes::to_json).collect::<Vec<_>>()
    })
}

/// Vollständig geprüfter Import – wird erst nach Bestätigung angewendet.
pub struct Import {
    pub path: PathBuf,
    pub app_version: String,
    pub exported: String,
    pub st: Settings,
    pub side: SidebarCfg,
    pub themes: Vec<Palette>,
    pub keys: Keymap,
    pub warnings: Vec<String>,
}

/// Liest eine Export-Datei. Jeder Fehler bricht ab, bevor irgendetwas geschrieben wird.
/// Fehlende Einstellungen behalten den aktuellen Wert (`cur`).
pub fn parse_import(path: PathBuf, cur: &Settings, cur_side: &SidebarCfg, cur_keys: &Keymap) -> Result<Import, Vec<String>> {
    let bytes = fs::read(&path).map_err(|e| vec![format!("Datei kann nicht gelesen werden: {e}")])?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(vec!["Die Datei ist größer als 8 MB – das ist kein Foldex-Export.".into()]);
    }
    let text = String::from_utf8(bytes).map_err(|_| vec!["Die Datei ist kein UTF-8-Text.".to_string()])?;
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| vec![format!("Kein gültiges JSON (Zeile {}, Spalte {}): {e}", e.line(), e.column())])?;
    let obj = v.as_object().ok_or_else(|| vec!["Die Datei enthält kein JSON-Objekt.".to_string()])?;

    let mut errs = Vec::new();
    let mut warns = Vec::new();
    match obj.get("format").and_then(Value::as_str) {
        // „filecore-settings“: Exporte aus der Zeit vor der Umbenennung in Foldex
        Some(EXPORT_FORMAT) | Some("filecore-settings") => {}
        Some(themes::FORMAT) => errs.push("Das ist eine Theme-Datei – bitte unter „Themes → Importieren“ laden.".into()),
        _ => errs.push(format!("Keine Foldex-Einstellungsdatei („format“ muss „{EXPORT_FORMAT}“ sein).")),
    }
    match obj.get("version").and_then(Value::as_u64) {
        Some(n) if n > EXPORT_VERSION => errs.push(format!(
            "Die Datei stammt aus einer neueren Foldex-Version (Format {n}); diese Version unterstützt bis Format {EXPORT_VERSION}."
        )),
        Some(0) | None => errs.push("„version“ fehlt oder ist ungültig.".into()),
        Some(_) => {}
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    // Einstellungen: nur bekannte Schlüssel, jeweils typgeprüft
    let mut st = cur.clone();
    let empty = serde_json::Map::new();
    let s = match obj.get("settings") {
        Some(Value::Object(m)) => m,
        Some(_) => return Err(vec!["„settings“ muss ein Objekt sein.".into()]),
        None => &empty,
    };
    for (k, v) in s {
        let flag = |errs: &mut Vec<String>, f: &mut bool| match v.as_bool() {
            Some(b) => *f = b,
            None => errs.push(format!("„settings.{k}“ muss true oder false sein.")),
        };
        let choice = |errs: &mut Vec<String>, opts: &[&str]| -> Option<u8> {
            let i = v.as_str().and_then(|s| opts.iter().position(|o| *o == s));
            if i.is_none() {
                errs.push(format!("„settings.{k}“: erlaubt sind {}.", opts.join(", ")));
            }
            i.map(|i| i as u8)
        };
        match k.as_str() {
            "colorMode" => st.theme = choice(&mut errs, &MODES).unwrap_or(st.theme),
            "defaultView" => st.view_mode = choice(&mut errs, &VIEWS).unwrap_or(st.view_mode),
            "lightTheme" | "darkTheme" => match v.as_str() {
                Some(id) if !id.trim().is_empty() => {
                    if k == "lightTheme" {
                        st.theme_light = id.trim().into()
                    } else {
                        st.theme_dark = id.trim().into()
                    }
                }
                _ => errs.push(format!("„settings.{k}“ muss eine Theme-ID sein.")),
            },
            "animations" => flag(&mut errs, &mut st.animations),
            "foldersFirst" => flag(&mut errs, &mut st.folders_first),
            "showHidden" => flag(&mut errs, &mut st.show_hidden),
            "showSystemFiles" => flag(&mut errs, &mut st.show_system),
            "systemIcons" => flag(&mut errs, &mut st.sys_icons),
            "autoIndex" => flag(&mut errs, &mut st.auto_index),
            "showSidebar" => flag(&mut errs, &mut st.show_sidebar),
            "showGitRepositories" => flag(&mut errs, &mut st.show_git),
            "showRecentFolders" => flag(&mut errs, &mut st.show_recent),
            "folderSizes" => flag(&mut errs, &mut st.folder_sizes),
            "gitStatusMarks" => flag(&mut errs, &mut st.git_marks),
            "aiEnabled" => flag(&mut errs, &mut st.ai_enabled),
            "showPreview" => flag(&mut errs, &mut st.show_preview),
            "terminalCompletion" => flag(&mut errs, &mut st.term_complete),
            "restoreSession" => flag(&mut errs, &mut st.restore_session),
            "iconTheme" => {
                if let Some(t) = v.as_str() {
                    st.icon_set = crate::matico::Set::from_key(t).key().to_string();
                }
            }
            "terminalShell" | "editorVSCode" | "editorZed" => match v.as_str() {
                Some(t) if !t.contains('\n') => {
                    let t = t.trim().to_string();
                    if !t.is_empty() && k != "terminalShell" && !std::path::Path::new(&t).is_file() {
                        warns.push(format!("Programm für „{k}“ gibt es auf diesem PC nicht: {t} – es wird automatisch gesucht."));
                    }
                    match k.as_str() {
                        "terminalShell" => st.term_shell = t,
                        // Exporte älterer Versionen
                        "editorVSCode" if !t.is_empty() => {
                            st.editor_paths.insert("vscode".into(), t);
                        }
                        "editorZed" if !t.is_empty() => {
                            st.editor_paths.insert("zed".into(), t);
                        }
                        _ => {}
                    }
                }
                _ => errs.push(format!("„settings.{k}“ muss ein Programmpfad (Text) sein.")),
            },
            _ => warns.push(format!("Unbekannte Einstellung „{k}“ wird ignoriert.")),
        }
    }

    // Seitenleiste
    let mut side = SidebarCfg { pins: cur_side.pins.clone(), show_pc: cur_side.show_pc, hidden: cur_side.hidden.clone() };
    match obj.get("sidebar") {
        None => {}
        Some(Value::Object(m)) => {
            if let Some(b) = m.get("showDrives") {
                match b.as_bool() {
                    Some(b) => side.show_pc = b,
                    None => errs.push("„sidebar.showDrives“ muss true oder false sein.".into()),
                }
            }
            if let Some(p) = m.get("pins") {
                match p.as_array() {
                    Some(arr) => {
                        side.pins.clear();
                        for (i, e) in arr.iter().enumerate() {
                            let label = e.get("label").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
                            let path = e.get("path").and_then(Value::as_str).filter(|s| !s.is_empty());
                            match (label, path) {
                                (Some(l), Some(p)) => {
                                    let p = PathBuf::from(p);
                                    if !p.is_dir() {
                                        warns.push(format!("Angehefteter Ordner „{l}“ existiert auf diesem PC nicht: {}", p.display()));
                                    }
                                    side.pins.push(Entry { label: l.replace(['\t', '\n'], " "), path: p });
                                }
                                _ => errs.push(format!("„sidebar.pins[{i}]“ braucht „label“ und „path“.")),
                            }
                        }
                    }
                    None => errs.push("„sidebar.pins“ muss eine Liste sein.".into()),
                }
            }
        }
        Some(_) => errs.push("„sidebar“ muss ein Objekt sein.".into()),
    }

    // Eigene Themes: jedes einzeln prüfen; ein fehlerhaftes Theme verhindert den ganzen Import
    let mut pals = Vec::new();
    match obj.get("customThemes") {
        None => {}
        Some(Value::Array(arr)) => {
            for (i, t) in arr.iter().enumerate() {
                let c = themes::from_json(t);
                let name = t.get("name").and_then(Value::as_str).unwrap_or("?");
                warns.extend(c.warnings.into_iter().map(|w| format!("Theme „{name}“: {w}")));
                match c.palette {
                    Ok(p) => pals.push(p),
                    Err(e) => errs.extend(e.into_iter().map(|m| format!("customThemes[{i}] „{name}“: {m}"))),
                }
            }
        }
        Some(_) => errs.push("„customThemes“ muss eine Liste sein.".into()),
    }
    // Editoren
    if let Some(v) = obj.get("editors") {
        match v.as_object() {
            Some(m) => {
                for (k, e) in m {
                    if crate::editors::def(k).is_none() {
                        warns.push(format!("Unbekannter Editor „{k}“ wird ignoriert."));
                        continue;
                    }
                    if let Some(b) = e.get("enabled").and_then(Value::as_bool) {
                        st.editor_on.insert(k.clone(), b);
                    }
                    match e.get("path").and_then(Value::as_str).map(str::trim) {
                        Some("") | None => {
                            st.editor_paths.remove(k);
                        }
                        Some(p) => {
                            if !std::path::Path::new(p).is_file() {
                                warns.push(format!("Editor „{k}“: {p} gibt es auf diesem PC nicht – es wird automatisch gesucht."));
                            }
                            st.editor_paths.insert(k.clone(), p.to_string());
                        }
                    }
                }
            }
            None => errs.push("„editors“ muss ein Objekt sein.".into()),
        }
    }
    if let Some(v) = obj.get("customEditors") {
        match v.as_array() {
            Some(arr) => {
                st.custom_editors.clear();
                for (i, e) in arr.iter().enumerate() {
                    match (e.get("name").and_then(Value::as_str), e.get("path").and_then(Value::as_str)) {
                        (Some(n), Some(p)) if !n.trim().is_empty() && !p.trim().is_empty() => {
                            st.custom_editors.push((n.trim().to_string(), p.trim().to_string()))
                        }
                        _ => errs.push(format!("„customEditors[{i}]“ braucht „name“ und „path“.")),
                    }
                }
            }
            None => errs.push("„customEditors“ muss eine Liste sein.".into()),
        }
    }
    if let Some(m) = obj.get("agents").and_then(Value::as_object) {
        for (k, b) in m {
            if let Some(b) = b.as_bool() {
                st.agent_on.insert(k.clone(), b);
            }
        }
    }
    if let Some(ai) = obj.get("ai").and_then(Value::as_object) {
        if let Some(arr) = ai.get("models").and_then(Value::as_array) {
            let mut v = Vec::new();
            for (i, m) in arr.iter().enumerate() {
                let f = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
                if f("id").is_empty() || f("url").is_empty() {
                    errs.push(format!("„ai.models[{i}]“ braucht „id“ und „url“."));
                    continue;
                }
                v.push(ModelProfile { id: f("id"), name: f("name"), kind: ai::Kind::from_key(&f("api")), url: f("url"), model: f("model") });
            }
            if !v.is_empty() {
                st.ai_models = v;
            }
        }
        for (k, field) in [("assistant", &mut st.ai_assist), ("agent", &mut st.ai_agent)] {
            if let Some(x) = ai.get(k).and_then(Value::as_str) {
                *field = x.to_string();
            }
        }
        if let Some(a) = ai.get("autoApprove").and_then(Value::as_array) {
            st.ai_auto = a.iter().filter_map(Value::as_str).filter(|c| ai::CATEGORIES.iter().any(|k| k.0 == *c)).map(str::to_string).collect();
        }
        if let Some(b) = ai.get("cliFullAccess").and_then(Value::as_bool) {
            st.ai_cli_full = b;
        }
        if let Some(m) = ai.get("agentModels").and_then(Value::as_object) {
            for (k, v) in m {
                if let Some(v) = v.as_str() {
                    st.agent_model.insert(k.clone(), v.to_string());
                }
            }
        }
    }
    if let Some(arr) = obj.get("quickbar").and_then(Value::as_array) {
        st.quickbar = crate::quickbar::normalize(
            arr.iter()
                .filter_map(|e| Some((e.get("id")?.as_str()?.to_string(), e.get("visible").and_then(Value::as_bool).unwrap_or(true))))
                .collect(),
        );
    }
    // Tastenkürzel
    let mut keys = cur_keys.clone();
    if let Some(v) = obj.get("shortcuts") {
        match Keymap::from_json(v) {
            Ok((k, w)) => {
                keys = k;
                warns.extend(w);
            }
            Err(e) => errs.extend(e.into_iter().map(|m| format!("Tastenkürzel: {m}"))),
        }
    }
    for k in obj.keys() {
        if !["format", "version", "appVersion", "exported", "settings", "sidebar", "customThemes", "shortcuts", "editors", "customEditors", "agents", "ai", "quickbar"].contains(&k.as_str()) {
            warns.push(format!("Unbekannter Abschnitt „{k}“ wird ignoriert."));
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    let str_of = |k: &str| obj.get(k).and_then(Value::as_str).unwrap_or("unbekannt").to_string();
    Ok(Import { path, app_version: str_of("appVersion"), exported: str_of("exported"), st, side, themes: pals, keys, warnings: warns })
}

/// Sicherung der aktuellen Konfiguration vor einem Import.
pub fn backup_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("backup-vor-import.json")
}
