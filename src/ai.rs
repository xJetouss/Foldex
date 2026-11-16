//! KI für den Explorer über eigene Modell-Profile (Anthropic-API oder OpenAI-kompatibel: LM Studio, Ollama,
//! Cloud-Dienste): Werkzeuge, mit denen das Modell Dateien ansieht, bearbeitet und verwaltet, und deren Ausführung.
//!
//! Sicherheit: Verändernde Werkzeuge laufen nur nach „Übernehmen“ – außer der Benutzer hat die Kategorie in den
//! Einstellungen ausdrücklich für den Agenten freigegeben. Im Assistent-Modus werden sie immer abgelehnt.
//! Gelöscht wird nur in den Papierkorb, nichts wird ungefragt überschrieben (Bearbeiten legt Sicherungskopien an),
//! und jede Änderung landet im Rückgängig-Verlauf. API-Schlüssel liegen verschlüsselt (Windows DPAPI, an das
//! Benutzerkonto gebunden) im Konfigurationsordner und werden nie exportiert.

use crate::ops::{self, Op, Policy};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64},
    time::Duration,
};

// ───────────────────────────── Modell-Profile ─────────────────────────────

/// Schnittstelle eines Modells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Anthropic Messages API (Claude)
    Anthropic,
    /// OpenAI-kompatible Chat-Completions (LM Studio, Ollama, OpenAI, OpenRouter …)
    OpenAi,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Anthropic, Kind::OpenAi];
    pub fn key(self) -> &'static str {
        match self {
            Kind::Anthropic => "anthropic",
            Kind::OpenAi => "openai",
        }
    }
    pub fn from_key(k: &str) -> Kind {
        if k == "anthropic" {
            Kind::Anthropic
        } else {
            Kind::OpenAi
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Anthropic => "Anthropic (Claude)",
            Kind::OpenAi => "OpenAI-kompatibel",
        }
    }
}

/// Ein selbst verwaltetes Modell. Der API-Schlüssel liegt getrennt und verschlüsselt im Schlüsselspeicher
/// (unter der `id`) und ist nie Teil der Einstellungen oder eines Exports.
#[derive(Clone, PartialEq, Debug)]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub url: String,
    pub model: String,
}

impl ModelProfile {
    /// Läuft auf diesem PC (LM Studio, Ollama …) → kein Schlüssel nötig.
    pub fn is_local(&self) -> bool {
        let u = self.url.to_lowercase();
        u.contains("://localhost") || u.contains("://127.0.0.1") || u.contains("://[::1]")
    }
    pub fn needs_key(&self) -> bool {
        !self.is_local()
    }
    /// Speicherformat einer Zeile: id|name|schnittstelle|url|modell
    pub fn encode(&self) -> String {
        let c = |s: &str| s.replace(['|', '\n', '\r'], " ");
        format!("{}|{}|{}|{}|{}", c(&self.id), c(&self.name), self.kind.key(), c(&self.url), c(&self.model))
    }
    pub fn decode(s: &str) -> Option<ModelProfile> {
        let f: Vec<&str> = s.splitn(5, '|').collect();
        (f.len() == 5 && !f[0].trim().is_empty()).then(|| ModelProfile {
            id: f[0].trim().into(),
            name: f[1].trim().into(),
            kind: Kind::from_key(f[2].trim()),
            url: f[3].trim().into(),
            model: f[4].trim().into(),
        })
    }
    /// Neue, eindeutige ID für ein hinzugefügtes Modell.
    pub fn new_id(existing: &[ModelProfile]) -> String {
        (1..).map(|n| format!("m{n}")).find(|id| !existing.iter().any(|p| &p.id == id)).unwrap()
    }
}

/// Vorlagen für „Modell hinzufügen“: (Name, Schnittstelle, Adresse, Modell-ID).
pub const PRESETS: &[(&str, Kind, &str, &str)] = &[
    ("Claude (Anthropic)", Kind::Anthropic, "https://api.anthropic.com", "claude-opus-5-5"),
    ("LM Studio (lokal)", Kind::OpenAi, "http://localhost:1234/v1", ""),
    ("Ollama (lokal)", Kind::OpenAi, "http://localhost:11434/v1", ""),
    ("OpenAI", Kind::OpenAi, "https://api.openai.com/v1", ""),
    ("OpenRouter", Kind::OpenAi, "https://openrouter.ai/api/v1", ""),
];

/// Startausstattung: Claude über die Anthropic-API und LM Studio lokal.
/// (IDs „anthropic“/„lmstudio“ entsprechen den Schlüsseldateien älterer Versionen.)
pub fn default_profiles() -> Vec<ModelProfile> {
    vec![
        ModelProfile { id: "anthropic".into(), name: "Claude".into(), kind: Kind::Anthropic, url: PRESETS[0].2.into(), model: "claude-opus-5-5".into() },
        ModelProfile { id: "lmstudio".into(), name: "LM Studio".into(), kind: Kind::OpenAi, url: PRESETS[1].2.into(), model: String::new() },
    ]
}

/// Claude-Modelle zur Auswahl (Opus 5.5 ist der Standard).
pub const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("claude-opus-5-5", "Claude Opus 5.5"),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
    ("claude-haiku-5-5", "Claude Haiku 5.5"),
    ("claude-fable-5-1", "Claude Fable 5.1"),
];

/// Alles, was eine Anfrage braucht (Profil + Schlüssel).
#[derive(Clone)]
pub struct Cfg {
    pub kind: Kind,
    pub model: String,
    pub base: String,
    pub key: Option<String>,
    pub local: bool,
    /// Explorer-KI: zusätzlich Werkzeuge für Tabs und Explorer-Befehle
    pub editor: bool,
    /// Denkaufwand (nur Claude): „low“, „medium“, „high“, „xhigh“, „max“
    pub effort: String,
    /// Reiner Chat: keine Werkzeuge, kein Explorer-Kontext – das Modell antwortet nur auf den Text
    pub plain: bool,
}

impl Cfg {
    pub fn from_profile(p: &ModelProfile, key: Option<String>) -> Cfg {
        Cfg { kind: p.kind, model: p.model.clone(), base: p.url.clone(), key, local: p.is_local(), editor: false, effort: "medium".into(), plain: false }
    }
}

/// Denkaufwand-Stufen für Claude: (Wert, Beschriftung, Erklärung).
pub const EFFORTS: &[(&str, &str, &str)] = &[
    ("low", "Niedrig", "schnell, kurze Antworten – günstig"),
    ("medium", "Normal", "ausgewogen – Standard"),
    ("high", "Hoch", "denkt länger nach – für knifflige Aufgaben"),
    ("xhigh", "Extra hoch", "sehr gründlich – gut für Programmieren und Agenten"),
    ("max", "Maximal", "so gründlich wie möglich – langsam und teuer"),
];

// ───────────────────────────── Schlüsselspeicher ─────────────────────────────

fn key_file(id: &str) -> PathBuf {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("keys").join(format!("{safe}.bin"))
}

#[cfg(windows)]
fn protect(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let ok = unsafe {
        if encrypt {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        }
    };
    if ok == 0 || out.pbData.is_null() {
        return None;
    }
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe { LocalFree(out.pbData as _) };
    Some(v)
}

/// Andere Systeme: keine Systemverschlüsselung verfügbar.
#[cfg(not(windows))]
fn protect(data: &[u8], _encrypt: bool) -> Option<Vec<u8>> {
    Some(data.to_vec())
}

pub fn save_key(id: &str, key: &str) -> Result<(), String> {
    let enc = protect(key.trim().as_bytes(), true).ok_or("Schlüssel konnte nicht verschlüsselt werden")?;
    crate::themes::write_atomic(&key_file(id), &enc).map_err(|e| e.to_string())
}

pub fn load_key(id: &str) -> Option<String> {
    let enc = fs::read(key_file(id)).ok()?;
    let dec = protect(&enc, false)?;
    String::from_utf8(dec).ok().filter(|k| !k.is_empty())
}

pub fn delete_key(id: &str) {
    let _ = fs::remove_file(key_file(id));
}

/// Schlüssel eines Profils: aus dem Speicher, sonst aus der Umgebung (ANTHROPIC_API_KEY / OPENAI_API_KEY).
pub fn key_for(p: &ModelProfile) -> Option<String> {
    load_key(&p.id).or_else(|| match p.kind {
        Kind::Anthropic => std::env::var("ANTHROPIC_API_KEY").ok(),
        Kind::OpenAi if p.url.contains("api.openai.com") => std::env::var("OPENAI_API_KEY").ok(),
        Kind::OpenAi => None,
    })
}

/// Zu welcher Freigabe-Kategorie ein verändernder Werkzeugaufruf gehört (für „selbstständig erlauben“).
pub fn tool_category(name: &str) -> Option<&'static str> {
    Some(match name {
        "create_folder" | "create_text_file" | "generate_image" | "edit_image" => "create",
        "write_text_file" | "edit_text_file" => "edit",
        "rename" | "move" | "copy" => "move",
        "move_to_trash" => "trash",
        "open_file" => "open",
        "close_tab" => "tabs",
        "run_command" => "commands",
        "mouse" | "type_text" | "press_key" => "control",
        "run_in_terminal" | "run_shell" => "shell",
        _ => return None,
    })
}

/// Muss dieser Aufruf bestätigt werden? (Explorer-Befehle nur, wenn sie etwas verändern können)
pub fn needs_ok(call: &Call) -> bool {
    if call.name == "run_command" {
        let id = call.input.get("command").and_then(Value::as_str).unwrap_or("");
        return command_policy(id) != Some(false);
    }
    tool(&call.name).is_none_or(|t| t.needs_ok)
}

/// Ist ein Shell-Befehl riskant (löscht, überschreibt, veröffentlicht, ändert das System)? Dann wird immer
/// nachgefragt – auch wenn Befehle in den Einstellungen selbstständig erlaubt sind. Rückgabe: Grund.
pub fn risky_command(cmd: &str) -> Option<&'static str> {
    let c = format!(" {} ", cmd.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "));
    let has = |pats: &[&str]| pats.iter().any(|p| c.contains(p));
    Some(if has(&[" rm ", " rm -", " rmdir ", " rd ", " del ", " erase ", "remove-item", " ri ", "clear-content", " shred "]) {
        "löscht Dateien oder Ordner"
    } else if has(&[" format ", "format-volume", "diskpart", " mkfs", " dd ", "clear-disk", "initialize-disk"]) {
        "formatiert oder überschreibt Laufwerke"
    } else if has(&["git push", "git push -f", "--force", "npm publish", "cargo publish", "gh release", "gh pr merge", "gh repo delete", "gh repo create", "twine upload", "docker push"]) {
        "veröffentlicht oder überschreibt Daten auf einem Server"
    } else if has(&["reset --hard", "git clean", "git checkout -- ", "git restore ", "branch -d", "branch -D", "stash drop", "stash clear", "rebase", "filter-branch", "reflog expire"]) {
        "verwirft Git-Änderungen oder schreibt den Verlauf um"
    } else if has(&["shutdown", "restart-computer", "stop-computer", "reboot", "logoff"]) {
        "fährt den PC herunter oder startet ihn neu"
    } else if has(&["reg delete", "reg add", "set-itemproperty", "new-itemproperty", "bcdedit", "set-executionpolicy", "netsh ", "sc delete", "sc config", "schtasks", "takeown", "icacls", "cacls", "net user", "net localgroup", "setx "]) {
        "ändert System-, Registry- oder Sicherheitseinstellungen"
    } else if has(&["stop-process", "taskkill", " kill ", "pkill", "killall"]) {
        "beendet Programme"
    } else if has(&["sudo ", "runas", "-verb runas", "start-process"]) {
        "startet Programme mit Adminrechten bzw. außerhalb von Foldex"
    } else if has(&["| iex", "|iex", "invoke-expression", "| sh", "|sh", "| bash", "|bash", "irm ", "iwr ", "invoke-webrequest", "curl ", "wget "]) {
        "lädt etwas aus dem Internet oder führt geladenen Code aus"
    } else if has(&[" > ", " >> ", "set-content", "out-file", "move-item", " mv ", " move ", "rename-item", " ren ", "copy-item", " cp ", " copy ", " xcopy ", "robocopy"]) {
        "überschreibt, verschiebt oder kopiert Dateien"
    } else if has(&["install", "uninstall", "winget ", "choco ", "scoop ", "pip ", "npm i ", "npm add", "cargo add"]) {
        "installiert oder entfernt Software"
    } else {
        return None;
    })
}

/// Explorer-Befehle für die Explorer-KI: `Some(false)` = ohne Rückfrage, `Some(true)` = nach Bestätigung,
/// `None` = gesperrt (endgültig löschen, Einstellungen, KI selbst …).
pub fn command_policy(id: &str) -> Option<bool> {
    if !crate::keymap::CMDS.iter().any(|c| c.id == id) {
        return None;
    }
    match id {
        // würde die Explorer-KI selbst schließen
        "ai.assistant" | "ai.agent" | "ai.chat" => None,
        "file.delete" | "file.trash" | "app.clearrecent" | "file.cut" | "file.paste" | "tab.close" | "tile.close" | "tab.detach" | "win.new" | "file.undo" | "file.redo" | "file.newfolder" | "file.newfile"
        | "file.rename" | "file.vscode" | "file.zed" | "file.openwith" | "git.pull" | "git.push" | "git.fetch" | "term.new" => Some(true),
        _ => Some(false),
    }
}

/// Freigabe-Kategorien: (Schlüssel, Beschriftung).
pub const CATEGORIES: &[(&str, &str)] = &[
    ("create", "Ordner und Dateien erstellen"),
    ("edit", "Dateien bearbeiten"),
    ("move", "Umbenennen, verschieben, kopieren"),
    ("trash", "In den Papierkorb legen"),
    ("open", "Dateien mit ihrem Programm öffnen"),
    ("tabs", "Tabs schließen (Explorer-KI)"),
    ("commands", "Explorer-Befehle ausführen (Explorer-KI)"),
    ("control", "Maus und Tastatur im Explorer bedienen (Explorer-KI)"),
    ("shell", "Befehle im Hintergrund ausführen (Shell, Git, GitHub CLI) – riskante fragen immer"),
];

// ───────────────────────────── Werkzeuge ─────────────────────────────

pub struct ToolDef {
    pub name: &'static str,
    pub desc: &'static str,
    /// Verändert Dateien oder startet etwas → nur nach „Übernehmen“
    pub needs_ok: bool,
    schema: fn() -> Value,
}

fn path_schema(desc: &str) -> Value {
    json!({ "type": "object", "properties": { "path": { "type": "string", "description": desc } }, "required": ["path"] })
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "view_image",
        desc: "Looks at an image file (photo, screenshot, scan …) and returns it to you so you can see its content. Use it to describe, sort, rename or compare pictures by what they show. Only works if your model understands images.",
        needs_ok: false,
        schema: || path_schema("Image file (absolute or relative to the current folder)"),
    },
    ToolDef {
        name: "list_image_models",
        desc: "Lists the local image generation models installed in Foldex (name, type, default size). Use before generate_image if the user wants a specific model.",
        needs_ok: false,
        schema: || json!({ "type": "object", "properties": {} }),
    },
    ToolDef {
        name: "generate_image",
        desc: "Generates NEW image(s) locally on this PC from a text prompt with an installed image model and saves them as PNG files. To change an existing image (e.g. one generated before, attached or selected) use edit_image instead. English prompts work best; describe subject, style, lighting and composition. Returns the saved file paths. Takes from a few seconds to a few minutes. The user may have to approve.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "prompt": { "type": "string", "description": "What the image should show" },
                "negative_prompt": { "type": "string", "description": "What should not be in the image (optional)" },
                "model": { "type": "string", "description": "Model name from list_image_models (default: the best installed one)" },
                "folder": { "type": "string", "description": "Folder to save into (default: current folder)" },
                "width": { "type": "integer" },
                "height": { "type": "integer" },
                "count": { "type": "integer", "description": "Number of images, 1-4 (default 1)" }
            }, "required": ["prompt"] })
        },
    },
    ToolDef {
        name: "edit_image",
        desc: "Edits an existing image file locally according to an instruction (e.g. 'make the sky a sunset', 'add a hat', 'remove the person', 'turn it into a watercolor') with an installed image edit model, keeping the rest of the picture. Saves the result as a new PNG next to the original (the original stays unchanged) and returns its path. Use this whenever the user wants to change, edit or modify an image instead of generating a new one; for an image you generated earlier pass its path.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "path": { "type": "string", "description": "Image file to edit" },
                "instruction": { "type": "string", "description": "What to change (English works best)" }
            }, "required": ["path", "instruction"] })
        },
    },
    ToolDef {
        name: "get_current_folder",
        desc: "Returns the folder currently open in the file explorer and the names of the selected items.",
        needs_ok: false,
        schema: || json!({ "type": "object", "properties": {} }),
    },
    ToolDef {
        name: "list_folder",
        desc: "Lists the entries of a folder (name, type, size, modified date). Use before acting on files you have not seen.",
        needs_ok: false,
        schema: || path_schema("Absolute folder path, or relative to the current folder."),
    },
    ToolDef {
        name: "read_text_file",
        desc: "Reads a text file (first 64 KB).",
        needs_ok: false,
        schema: || path_schema("Absolute file path, or relative to the current folder."),
    },
    ToolDef {
        name: "find_files",
        desc: "Searches a folder recursively for files and folders whose name matches a pattern (* and ? wildcards, case-insensitive). Returns at most 200 paths.",
        needs_ok: false,
        schema: || {
            json!({ "type": "object", "properties": {
                "folder": { "type": "string", "description": "Folder to search in." },
                "pattern": { "type": "string", "description": "Name pattern, e.g. *.pdf or report*" }
            }, "required": ["folder", "pattern"] })
        },
    },
    ToolDef {
        name: "open_folder",
        desc: "Shows a folder in the file explorer (navigation only, changes nothing).",
        needs_ok: false,
        schema: || path_schema("Folder to open."),
    },
    ToolDef {
        name: "select_items",
        desc: "Selects items by name in the folder currently shown in the explorer (changes nothing).",
        needs_ok: false,
        schema: || json!({ "type": "object", "properties": { "names": { "type": "array", "items": { "type": "string" } } }, "required": ["names"] }),
    },
    ToolDef {
        name: "create_folder",
        desc: "Creates a new folder. The user must approve.",
        needs_ok: true,
        schema: || path_schema("Path of the new folder."),
    },
    ToolDef {
        name: "create_text_file",
        desc: "Creates a new text file with the given content. Never overwrites an existing file. The user must approve.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            }, "required": ["path", "content"] })
        },
    },
    ToolDef {
        name: "rename",
        desc: "Renames a file or folder within its folder. Never overwrites. The user must approve.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "path": { "type": "string" },
                "new_name": { "type": "string", "description": "New name only, without a folder." }
            }, "required": ["path", "new_name"] })
        },
    },
    ToolDef {
        name: "edit_text_file",
        desc: "Edits a text file by replacing one exact occurrence of old_text with new_text. old_text must match exactly once (include enough context). Read the file first. The user may have to approve.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "path": { "type": "string" },
                "old_text": { "type": "string", "description": "Exact text to replace (must occur exactly once)." },
                "new_text": { "type": "string" }
            }, "required": ["path", "old_text", "new_text"] })
        },
    },
    ToolDef {
        name: "write_text_file",
        desc: "Writes the full content of a text file, creating it or replacing its content (a backup is kept for undo). Prefer edit_text_file for small changes. The user may have to approve.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            }, "required": ["path", "content"] })
        },
    },
    ToolDef {
        name: "move",
        desc: "Moves files or folders into a destination folder. Name clashes keep both (a number is added). The user must approve.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "paths": { "type": "array", "items": { "type": "string" } },
                "destination": { "type": "string", "description": "Destination folder (created if missing)." }
            }, "required": ["paths", "destination"] })
        },
    },
    ToolDef {
        name: "copy",
        desc: "Copies files or folders into a destination folder. Name clashes keep both. The user must approve.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "paths": { "type": "array", "items": { "type": "string" } },
                "destination": { "type": "string" }
            }, "required": ["paths", "destination"] })
        },
    },
    ToolDef {
        name: "move_to_trash",
        desc: "Moves files or folders to the recycle bin (recoverable). The user must approve.",
        needs_ok: true,
        schema: || json!({ "type": "object", "properties": { "paths": { "type": "array", "items": { "type": "string" } } }, "required": ["paths"] }),
    },
    ToolDef {
        name: "open_file",
        desc: "Opens a file with its default program. The user must approve.",
        needs_ok: true,
        schema: || path_schema("File to open."),
    },
    ToolDef {
        name: "run_shell",
        desc: "Runs a shell command in the background (PowerShell on Windows, sh elsewhere) without opening a terminal window and returns exit code and output (stdout + stderr). \
Use it for git (status, diff, add, commit, log, branch, push, pull), the GitHub CLI gh (pr, issue, repo, release), builds, tests, package managers and other command line tools. \
Commands run non-interactively (no input possible, no pagers); prefer flags that avoid prompts. Default timeout 120 s (max 900). The user may have to approve; risky commands (delete, push, install, system changes) always need approval.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "command": { "type": "string", "description": "Command line to run." },
                "folder": { "type": "string", "description": "Working folder (default: current folder)." },
                "timeout_seconds": { "type": "integer", "description": "Abort after this many seconds (default 120, max 900)." }
            }, "required": ["command"] })
        },
    },
];

/// Zusätzliche Werkzeuge der Explorer-KI: Tabs ansehen, öffnen, wechseln, schließen und Explorer-Befehle.
/// Laufen im Oberflächen-Thread (siehe `execute_editor`).
pub const EDITOR_TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "list_tabs",
        desc: "Lists the open tabs of the Foldex window: index, title, kind (folder, terminal, git, chat), folder and which one is active.",
        needs_ok: false,
        schema: || json!({ "type": "object", "properties": {} }),
    },
    ToolDef {
        name: "open_tab",
        desc: "Opens a new tab and makes it active. kind: \"folder\" (file list, default), \"terminal\" (shell in that folder) or \"git\" (repository view).",
        needs_ok: false,
        schema: || {
            json!({ "type": "object", "properties": {
                "path": { "type": "string", "description": "Folder for the tab (absolute or relative to the current folder)." },
                "kind": { "type": "string", "enum": ["folder", "terminal", "git"] }
            }, "required": ["path"] })
        },
    },
    ToolDef {
        name: "switch_tab",
        desc: "Activates the tab with the given index (from list_tabs).",
        needs_ok: false,
        schema: || json!({ "type": "object", "properties": { "index": { "type": "integer" } }, "required": ["index"] }),
    },
    ToolDef {
        name: "close_tab",
        desc: "Closes the tab with the given index (from list_tabs). Terminals in it are ended. The user may have to approve.",
        needs_ok: true,
        schema: || json!({ "type": "object", "properties": { "index": { "type": "integer" } }, "required": ["index"] }),
    },
    ToolDef {
        name: "list_commands",
        desc: "Lists the explorer commands you may run with run_command (id, description, whether the user must approve).",
        needs_ok: false,
        schema: || json!({ "type": "object", "properties": {} }),
    },
    ToolDef {
        name: "run_command",
        desc: "Runs an explorer command by id in the active tab (e.g. view.grid, view.hidden, view.preview, win.split, go.back, tab.next, tools.space, git.view). Commands that change things need the user's approval.",
        needs_ok: true,
        schema: || json!({ "type": "object", "properties": { "command": { "type": "string" } }, "required": ["command"] }),
    },
    ToolDef {
        name: "screenshot",
        desc: "Takes a screenshot of the whole Foldex window (including this chat panel). Use it to see the interface before using mouse, and again afterwards to check the result. Coordinates for mouse refer to this image.",
        needs_ok: false,
        schema: || json!({ "type": "object", "properties": {} }),
    },
    ToolDef {
        name: "mouse",
        desc: "Moves the mouse pointer inside the Foldex window and clicks or scrolls. x/y are pixel coordinates in the latest screenshot. action: move, click, double_click, right_click, scroll (amount: positive = down). The user may have to approve.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "action": { "type": "string", "enum": ["move", "click", "double_click", "right_click", "scroll"] },
                "x": { "type": "number" }, "y": { "type": "number" },
                "amount": { "type": "number", "description": "Scroll lines for action scroll" }
            }, "required": ["action", "x", "y"] })
        },
    },
    ToolDef {
        name: "type_text",
        desc: "Types text into the focused field of Foldex (click into the field first). In a terminal tab the text goes to the shell; add \\n to press Enter. The user may have to approve.",
        needs_ok: true,
        schema: || json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }),
    },
    ToolDef {
        name: "press_key",
        desc: "Presses a key or shortcut in Foldex, e.g. Enter, Escape, Tab, ArrowDown, F2, Delete, Ctrl+A, Ctrl+Shift+T. The user may have to approve.",
        needs_ok: true,
        schema: || json!({ "type": "object", "properties": { "key": { "type": "string" } }, "required": ["key"] }),
    },
    ToolDef {
        name: "run_in_terminal",
        desc: "Opens a visible terminal tab in the given folder and runs a shell command there (PowerShell on Windows). Only use this when the user explicitly wants to see or interact with a terminal (e.g. a long-running server); for everything else use run_shell, which runs in the background and returns the output directly. The user may have to approve.",
        needs_ok: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "command": { "type": "string" },
                "folder": { "type": "string", "description": "Working folder (default: current folder)" }
            }, "required": ["command"] })
        },
    },
    ToolDef {
        name: "read_terminal",
        desc: "Returns the visible text of a terminal tab (index from list_tabs; default: the terminal opened last by run_in_terminal, otherwise the first terminal tab).",
        needs_ok: false,
        schema: || json!({ "type": "object", "properties": { "index": { "type": "integer" } } }),
    },
];

/// Werkzeuge, deren Ergebnis erst einige Bilder später von der Oberfläche kommt (Bildschirmfoto, Terminaltext).
pub fn is_async_tool(name: &str) -> bool {
    matches!(name, "screenshot" | "read_terminal")
}

/// Antwort der Oberfläche auf ein asynchrones Werkzeug: Text und optional ein JPEG (Base64).
pub type Reply = (String, bool, Option<String>);

fn mailbox() -> &'static std::sync::Mutex<HashMap<String, Reply>> {
    static M: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Reply>>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

pub fn deliver(id: &str, reply: Reply) {
    if let Ok(mut m) = mailbox().lock() {
        m.insert(id.to_string(), reply);
    }
}

pub fn take_reply(id: &str) -> Option<Reply> {
    mailbox().lock().ok()?.remove(id)
}

pub fn tool(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().chain(EDITOR_TOOLS).find(|t| t.name == name)
}

pub fn is_editor_tool(name: &str) -> bool {
    EDITOR_TOOLS.iter().any(|t| t.name == name)
}

fn tools_for(cfg: &Cfg) -> impl Iterator<Item = &'static ToolDef> {
    let (ed, plain) = (cfg.editor, cfg.plain);
    TOOLS.iter().filter(move |_| !plain).chain(EDITOR_TOOLS.iter().filter(move |_| ed && !plain))
}

fn system_for(cfg: &Cfg) -> String {
    if cfg.plain {
        PLAIN_SYSTEM.to_string()
    } else if cfg.editor {
        format!("{SYSTEM} {EDITOR_SYSTEM}")
    } else {
        SYSTEM.to_string()
    }
}

/// Reiner Chat: keine Werkzeuge, kein Zugriff auf Explorer oder Dateien.
pub const PLAIN_SYSTEM: &str = "You are a helpful, knowledgeable assistant in a chat window. Answer the user's messages directly and clearly. \
You have no tools and no access to the user's files, folders or the file explorer; if a question needs them, say so and suggest the user switch to assistant or agent mode. \
Use Markdown for structure and fenced code blocks for code. Answer in the user's language.";

pub const EDITOR_SYSTEM: &str = "You are the explorer assistant of Foldex and live in a side panel that stays visible while tabs change. \
Besides the file tools you can see and control the explorer itself: list_tabs, open_tab, switch_tab, close_tab, and run_command for explorer commands (call list_commands to see them). \
Use them when the user asks you to open, show, arrange or switch something; the current folder in each message is the folder of the active tab. Tab indices change when tabs are opened or closed, so list the tabs again before closing one. \
You can also operate the whole program like a person: take a screenshot, then move and click the mouse, type text and press keys inside the Foldex window. \
Run command line tools with run_shell: it works in the background without opening a terminal and returns the output to you – the user only sees the command and its result in this chat. Only open a visible terminal tab (run_in_terminal) when the user asks for one. \
Prefer the direct tools (open_tab, run_command, file tools) when they can do the job; use mouse and keyboard for everything else, and check the result with a new screenshot.";

pub const SYSTEM: &str = "You are the AI built into Foldex, a file explorer for Windows. You help the user find, inspect, organize, edit, rename, move, copy and clean up files and folders by calling the provided tools. \
Look before you act: list or search folders first instead of guessing paths, and read a file before editing it. Paths may be absolute or relative to the current folder, which is given at the start of each user message together with the mode. \
In assistant mode you only look and answer; tools that change something are refused, so explain what the user could do or suggest switching to agent mode. In agent mode, changes are either applied directly or shown to the user for approval, depending on their settings; if a change is rejected, do not repeat it, ask what they want instead. \
Prefer few, well-grouped actions (one move call with several paths rather than many single moves). Deleting always goes to the recycle bin. \
For command line work, Git and GitHub use run_shell (git, gh): check `git status` and `git diff` before committing, stage deliberately, write a concise commit message (summary line, optional body), and only push when the user wants it. \
Answer in the user's language, briefly. After changes, summarize what was done.";

// ───────────────────────────── Anfragen ─────────────────────────────

/// Ein Werkzeugaufruf des Modells.
#[derive(Clone, Debug)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// Antwort einer Runde: Text, Werkzeugaufrufe und der Eintrag für den Gesprächsverlauf.
pub struct Turn {
    pub history_entry: Value,
    pub text: String,
    pub calls: Vec<Call>,
    /// „end_turn“, „tool_use“, „max_tokens“, „refusal“ …
    pub stop: String,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout(Duration::from_secs(600)).build()
}

fn http_error(e: ureq::Error, who: &str) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or(body);
            let hint = match code {
                401 | 403 => " – API-Schlüssel prüfen.",
                404 => " – Modellname oder Adresse prüfen.",
                429 => " – zu viele Anfragen, kurz warten.",
                500..=599 => " – Dienst gerade gestört, später erneut versuchen.",
                _ => "",
            };
            format!("{who}: Fehler {code}: {msg}{hint}")
        }
        ureq::Error::Transport(t) => format!("{who} nicht erreichbar: {t}"),
    }
}

/// Eine Runde an das Modell schicken (blockierend – nur im Hintergrund-Thread aufrufen).
pub fn request(cfg: &Cfg, history: &[Value]) -> Result<Turn, String> {
    match cfg.kind {
        Kind::Anthropic => request_anthropic(cfg, history),
        Kind::OpenAi => request_openai(cfg, history),
    }
}

fn request_anthropic(cfg: &Cfg, history: &[Value]) -> Result<Turn, String> {
    let key = cfg.key.as_deref().ok_or("Kein API-Schlüssel hinterlegt (Einstellungen → KI → Modelle).")?;
    let tools: Vec<Value> = tools_for(cfg).map(|t| json!({ "name": t.name, "description": t.desc, "input_schema": (t.schema)() })).collect();
    let effort = if EFFORTS.iter().any(|e| e.0 == cfg.effort) { cfg.effort.as_str() } else { "medium" };
    let mut body = json!({
        "model": cfg.model,
        "max_tokens": 16000,
        "system": system_for(cfg),
        "messages": history,
        "output_config": { "effort": effort },
        "cache_control": { "type": "ephemeral" }
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    let mut req = agent()
        .post(&format!("{}/v1/messages", cfg.base.trim_end_matches('/')))
        .set("content-type", "application/json")
        .set("x-api-key", key)
        .set("anthropic-version", "2026-10-21");
    // Server-seitige Ersatzmodelle bei Ablehnungen der Sicherheitsfilter – nur bei der offiziellen API (nicht Haiku)
    if cfg.base.contains("api.anthropic.com") && !cfg.model.contains("haiku") {
        body["fallbacks"] = json!("default");
        req = req.set("anthropic-beta", "server-side-fallback-2026-07-01");
    }
    let resp = req.send_string(&body.to_string()).map_err(|e| http_error(e, "Claude"))?;
    let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let content = v.get("content").cloned().unwrap_or(json!([]));
    let mut text = String::new();
    let mut calls = Vec::new();
    for b in content.as_array().into_iter().flatten() {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(b.get("text").and_then(Value::as_str).unwrap_or(""));
            }
            Some("tool_use") => calls.push(Call {
                id: b.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                name: b.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                input: b.get("input").cloned().unwrap_or(json!({})),
            }),
            _ => {}
        }
    }
    // Antwort unverändert in den Verlauf (inkl. Denk-Blöcken) – der Verlauf wird nur angehängt, nie bearbeitet
    Ok(Turn {
        history_entry: json!({ "role": "assistant", "content": content }),
        text,
        calls,
        stop: v.get("stop_reason").and_then(Value::as_str).unwrap_or("end_turn").to_string(),
    })
}

fn request_openai(cfg: &Cfg, history: &[Value]) -> Result<Turn, String> {
    let who = if cfg.local { "Lokaler KI-Server" } else { "KI-Dienst" };
    if cfg.model.trim().is_empty() {
        return Err(format!("{who}: kein Modell ausgewählt."));
    }
    // eingebautes lokales Modell: Server bei Bedarf starten
    crate::localllm::prepare(&cfg.base, &cfg.model)?;
    let tools: Vec<Value> = tools_for(cfg).map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.desc, "parameters": (t.schema)() } })).collect();
    let mut messages = vec![json!({ "role": "system", "content": system_for(cfg) })];
    messages.extend(history.iter().cloned());
    let mut body = json!({ "model": cfg.model, "messages": messages });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    let mut req = agent().post(&format!("{}/chat/completions", cfg.base.trim_end_matches('/'))).set("content-type", "application/json");
    if let Some(k) = cfg.key.as_deref().filter(|k| !k.is_empty()) {
        req = req.set("authorization", &format!("Bearer {k}"));
    }
    let resp = req.send_string(&body.to_string()).map_err(|e| http_error(e, who))?;
    let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let choice = v.pointer("/choices/0").ok_or_else(|| format!("{who}: unerwartete Antwort"))?;
    let msg = choice.get("message").cloned().unwrap_or(json!({ "role": "assistant", "content": "" }));
    let text = msg.get("content").and_then(Value::as_str).unwrap_or("").to_string();
    let calls = msg
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| Call {
            id: c.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
            name: c.pointer("/function/name").and_then(Value::as_str).unwrap_or_default().to_string(),
            input: c
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .and_then(|a| serde_json::from_str(a).ok())
                .unwrap_or(json!({})),
        })
        .collect::<Vec<_>>();
    let stop = match choice.get("finish_reason").and_then(Value::as_str).unwrap_or("stop") {
        "tool_calls" => "tool_use",
        "length" => "max_tokens",
        _ if !calls.is_empty() => "tool_use",
        _ => "end_turn",
    };
    Ok(Turn { history_entry: msg, text, calls, stop: stop.to_string() })
}

/// Kurze Einzelanfrage ohne Werkzeuge (z. B. Terminal-Vorschläge). Blockierend – nur im Hintergrund aufrufen.
pub fn quick(cfg: &Cfg, system: &str, user: &str) -> Result<String, String> {
    match cfg.kind {
        Kind::Anthropic => {
            let key = cfg.key.as_deref().ok_or("Kein API-Schlüssel für die Explorer-KI")?;
            let body = json!({
                "model": cfg.model, "max_tokens": 1024, "system": system,
                "messages": [{ "role": "user", "content": user }],
                "output_config": { "effort": "low" }
            });
            let resp = agent()
                .post(&format!("{}/v1/messages", cfg.base.trim_end_matches('/')))
                .set("content-type", "application/json")
                .set("x-api-key", key)
                .set("anthropic-version", "2026-10-21")
                .send_string(&body.to_string())
                .map_err(|e| http_error(e, "Claude"))?;
            let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            if v["stop_reason"] == "refusal" {
                return Err("abgelehnt".into());
            }
            Ok(v["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("
"))
        }
        Kind::OpenAi => {
            if cfg.model.trim().is_empty() {
                return Err("kein Modell gewählt".into());
            }
            crate::localllm::prepare(&cfg.base, &cfg.model)?;
            let body = json!({ "model": cfg.model, "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }] });
            let mut req = agent().post(&format!("{}/chat/completions", cfg.base.trim_end_matches('/'))).set("content-type", "application/json");
            if let Some(k) = cfg.key.as_deref().filter(|k| !k.is_empty()) {
                req = req.set("authorization", &format!("Bearer {k}"));
            }
            let resp = req.send_string(&body.to_string()).map_err(|e| http_error(e, "KI"))?;
            let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            Ok(v.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or("").to_string())
        }
    }
}

/// Bild als JPEG (höchstens 1024 px, Base64) für ein Modell, das Bilder ansieht
pub fn image_b64(p: &Path) -> Option<String> {
    let img = image::open(p).ok()?;
    let img = if img.width().max(img.height()) > 1024 { img.thumbnail(1024, 1024) } else { img };
    let rgb = img.to_rgb8();
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85).encode_image(&rgb).ok()?;
    Some(crate::convert::base64(&buf))
}

/// Bilder an eine Benutzernachricht hängen (Claude: image-Blöcke, OpenAI-Format: image_url)
pub fn with_images(entry: Value, kind: Kind, imgs: &[String]) -> Value {
    if imgs.is_empty() {
        return entry;
    }
    let text = entry["content"].as_str().unwrap_or_default().to_string();
    let mut parts = vec![json!({ "type": "text", "text": text })];
    for b in imgs {
        parts.push(match kind {
            Kind::Anthropic => json!({ "type": "image", "source": { "type": "base64", "media_type": "image/jpeg", "data": b } }),
            Kind::OpenAi => json!({ "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{b}") } }),
        });
    }
    json!({ "role": "user", "content": parts })
}

/// Verlaufseintrag im reinen Chat: nur der Text, ohne Ordner oder Auswahl.
pub fn plain_entry(text: &str) -> Value {
    json!({ "role": "user", "content": text })
}

/// Verlaufseintrag für eine Benutzernachricht (inkl. Modus und aktuellem Ordner als Kontext).
pub fn user_entry(text: &str, folder: &Path, selection: &[String], agent: bool) -> Value {
    let mut ctx = format!("[Mode: {}]\n[Current folder: {}]", if agent { "agent" } else { "assistant" }, folder.display());
    if !selection.is_empty() {
        ctx.push_str(&format!("\n[Selected: {}]", selection.join(", ")));
    }
    json!({ "role": "user", "content": format!("{ctx}\n\n{text}") })
}

/// Verlaufseinträge mit den Ergebnissen aller Werkzeugaufrufe einer Runde.
/// Claude: alle Ergebnisse in EINER Benutzernachricht; OpenAI-Format: je eine „tool“-Nachricht.
/// Ein Ergebnis kann ein Bild (JPEG, Base64) enthalten – Claude bekommt es im tool_result, das OpenAI-Format
/// als zusätzliche Benutzernachricht (Werkzeugnachrichten dürfen dort nur Text enthalten).
pub fn results_entries(k: Kind, results: &[(String, String, bool, Option<String>)]) -> Vec<Value> {
    match k {
        Kind::Anthropic => vec![json!({
            "role": "user",
            "content": results.iter().map(|(id, text, err, img)| match img {
                Some(b) => json!({ "type": "tool_result", "tool_use_id": id, "is_error": err, "content": [
                    { "type": "text", "text": if text.is_empty() { "Bild" } else { text } },
                    { "type": "image", "source": { "type": "base64", "media_type": "image/jpeg", "data": b } }
                ] }),
                None => json!({ "type": "tool_result", "tool_use_id": id, "content": text, "is_error": err }),
            }).collect::<Vec<_>>()
        })],
        _ => {
            let mut v: Vec<Value> = results.iter().map(|(id, text, _, _)| json!({ "role": "tool", "tool_call_id": id, "content": text })).collect();
            let imgs: Vec<Value> = results.iter().filter_map(|r| r.3.as_ref()).map(|b| json!({ "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{b}") } })).collect();
            if !imgs.is_empty() {
                let mut parts = vec![json!({ "type": "text", "text": "Screenshot from the tool call above." })];
                parts.extend(imgs);
                v.push(json!({ "role": "user", "content": parts }));
            }
            v
        }
    }
}

/// Verbindung prüfen (ohne Kosten): Modellliste abrufen.
pub fn test_connection(cfg: &Cfg) -> Result<String, String> {
    match cfg.kind {
        Kind::Anthropic => {
            let key = cfg.key.as_deref().ok_or("Kein API-Schlüssel hinterlegt.")?;
            let resp = agent()
                .get(&format!("{}/v1/models", cfg.base.trim_end_matches('/')))
                .set("x-api-key", key)
                .set("anthropic-version", "2026-10-21")
                .call()
                .map_err(|e| http_error(e, "Claude"))?;
            let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            let n = v.get("data").and_then(Value::as_array).map_or(0, |a| a.len());
            Ok(format!("Verbindung ok – {n} Modelle verfügbar"))
        }
        Kind::OpenAi => {
            let ids = list_models(cfg)?;
            let has = cfg.model.is_empty() || ids.iter().any(|m| *m == cfg.model);
            Ok(format!(
                "Verbindung ok – {} Modell(e){}",
                ids.len(),
                if has { String::new() } else { format!("; „{}“ ist nicht dabei (verfügbar: {})", cfg.model, ids.join(", ")) }
            ))
        }
    }
}

/// Verfügbare Modelle beim Anbieter (Anthropic, LM Studio, OpenAI-kompatibel).
pub fn list_models(cfg: &Cfg) -> Result<Vec<String>, String> {
    if crate::localllm::is_builtin(&cfg.base) {
        return Ok(crate::localllm::installed_text().into_iter().map(|t| t.0).collect());
    }
    let base = cfg.base.trim_end_matches('/');
    let mut req = match cfg.kind {
        // Anthropic: /v1/models mit x-api-key
        Kind::Anthropic => agent().get(&format!("{base}/v1/models")).set("x-api-key", cfg.key.as_deref().ok_or("Kein API-Schlüssel hinterlegt.")?).set("anthropic-version", "2026-10-21"),
        Kind::OpenAi => agent().get(&format!("{base}/models")),
    };
    if let Some(k) = cfg.key.as_deref().filter(|k| !k.is_empty() && cfg.kind == Kind::OpenAi) {
        req = req.set("authorization", &format!("Bearer {k}"));
    }
    let resp = req.call().map_err(|e| http_error(e, "Modellliste"))?;
    let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut ids: Vec<String> = v.get("data").and_then(Value::as_array).into_iter().flatten().filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string)).collect();
    ids.sort();
    Ok(ids)
}

// ───────────────────────────── Ausführung ─────────────────────────────

/// Wirkung auf die Oberfläche (wird vom Explorer umgesetzt).
#[derive(Clone, Debug)]
pub enum Effect {
    Go(PathBuf),
    Select(Vec<String>),
    OpenFile(PathBuf),
    /// neuer Tab: Art („folder“, „terminal“, „git“) und Ordner
    OpenTab(String, PathBuf),
    SwitchTab(usize),
    CloseTab(usize),
    /// Explorer-Befehl (ID aus keymap::CMDS)
    Command(String),
    /// Bildschirmfoto für den Werkzeugaufruf mit dieser ID
    Screenshot(String),
    /// Maus: Aktion, x, y (Bildkoordinaten), Menge (Scrollen)
    Mouse(String, f32, f32, f32),
    TypeText(String),
    Key(String),
    /// Befehl in einem neuen Terminal-Tab: Ordner, Befehl
    Shell(PathBuf, String),
    /// Text eines Terminals für die Aufruf-ID (Tab-Index optional)
    ReadTerminal(String, Option<usize>),
}

/// „Ctrl+Shift+T“ → (Strg, Umschalt, Alt, Taste). `None` bei unbekannter Taste.
pub fn parse_key(spec: &str) -> Option<(bool, bool, bool, String)> {
    let parts: Vec<&str> = spec.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let (key, mods) = parts.split_last()?;
    let mut m = (false, false, false);
    for x in mods {
        match x.to_ascii_lowercase().as_str() {
            "ctrl" | "strg" | "control" | "cmd" => m.0 = true,
            "shift" | "umschalt" => m.1 = true,
            "alt" => m.2 = true,
            _ => return None,
        }
    }
    let k = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => "Enter".to_string(),
        "esc" | "escape" => "Escape".to_string(),
        "del" | "delete" | "entf" => "Delete".to_string(),
        "backspace" => "Backspace".to_string(),
        "tab" => "Tab".to_string(),
        "space" | "leertaste" => "Space".to_string(),
        "up" => "ArrowUp".to_string(),
        "down" => "ArrowDown".to_string(),
        "left" => "ArrowLeft".to_string(),
        "right" => "ArrowRight".to_string(),
        k if k.len() == 1 => k.to_ascii_uppercase(),
        _ => key.to_string(),
    };
    Some((m.0, m.1, m.2, k))
}

/// Ein offener Tab, wie ihn die Explorer-KI sieht.
#[derive(Clone, Debug)]
pub struct TabInfo {
    pub title: String,
    /// „folder“, „terminal“, „git“, „chat“
    pub kind: &'static str,
    pub folder: PathBuf,
    pub active: bool,
}

/// Werkzeuge der Explorer-KI ausführen (im Oberflächen-Thread, braucht die Tab-Liste).
pub fn execute_editor(call: &Call, cwd: &Path, tabs: &[TabInfo]) -> Outcome {
    let i = &call.input;
    let index = || i.get("index").and_then(Value::as_u64).map(|n| n as usize).filter(|&n| n < tabs.len()).ok_or_else(|| format!("Kein Tab mit diesem Index (0 bis {})", tabs.len().saturating_sub(1)));
    let effect = |text: String, e: Effect| Outcome { text, is_error: false, op: None, effect: Some(e) };
    let r = || -> Result<Outcome, String> {
        Ok(match call.name.as_str() {
            "list_tabs" => ok(json!(tabs
                .iter()
                .enumerate()
                .map(|(n, t)| json!({ "index": n, "title": t.title, "kind": t.kind, "folder": t.folder.display().to_string(), "active": t.active }))
                .collect::<Vec<_>>())
            .to_string()),
            "open_tab" => {
                let p = resolve(cwd, str_arg(i, "path")?);
                if !p.is_dir() {
                    return Err(format!("{} ist kein Ordner", p.display()));
                }
                let kind = match i.get("kind").and_then(Value::as_str).unwrap_or("folder") {
                    k @ ("terminal" | "git") => k,
                    _ => "folder",
                };
                effect(format!("Tab geöffnet ({kind}): {}", p.display()), Effect::OpenTab(kind.into(), p))
            }
            "switch_tab" => {
                let n = index()?;
                effect(format!("Tab {n} „{}“ ist jetzt aktiv", tabs[n].title), Effect::SwitchTab(n))
            }
            "close_tab" => {
                let n = index()?;
                effect(format!("Tab {n} „{}“ geschlossen", tabs[n].title), Effect::CloseTab(n))
            }
            "list_commands" => ok(json!(crate::keymap::CMDS
                .iter()
                .filter_map(|c| command_policy(c.id).map(|ask| json!({ "id": c.id, "description": c.label, "needs_approval": ask })))
                .collect::<Vec<_>>())
            .to_string()),
            "run_command" => {
                let id = str_arg(i, "command")?;
                if command_policy(id).is_none() {
                    return Err(format!("Befehl „{id}“ ist nicht erlaubt oder unbekannt – list_commands zeigt die verfügbaren"));
                }
                effect(format!("Befehl „{id}“ ausgeführt"), Effect::Command(id.to_string()))
            }
            "screenshot" => effect(String::new(), Effect::Screenshot(call.id.clone())),
            "read_terminal" => effect(String::new(), Effect::ReadTerminal(call.id.clone(), i.get("index").and_then(Value::as_u64).map(|n| n as usize))),
            "mouse" => {
                let action = i.get("action").and_then(Value::as_str).unwrap_or("click");
                if !matches!(action, "move" | "click" | "double_click" | "right_click" | "scroll") {
                    return Err(format!("Unbekannte Mausaktion „{action}“"));
                }
                let n = |k: &str| i.get(k).and_then(Value::as_f64).map(|v| v as f32);
                let (x, y) = (n("x").ok_or("Parameter „x“ fehlt")?, n("y").ok_or("Parameter „y“ fehlt")?);
                effect(format!("Maus: {action} bei ({x:.0}, {y:.0}) – mit screenshot das Ergebnis prüfen"), Effect::Mouse(action.into(), x, y, n("amount").unwrap_or(3.0)))
            }
            "type_text" => {
                let t = i.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or("Parameter „text“ fehlt")?;
                effect(format!("{} Zeichen getippt", t.chars().count()), Effect::TypeText(t.to_string()))
            }
            "press_key" => {
                let k = str_arg(i, "key")?;
                parse_key(k).ok_or_else(|| format!("Unbekannte Taste „{k}“"))?;
                effect(format!("Taste {k} gedrückt"), Effect::Key(k.to_string()))
            }
            "run_in_terminal" => {
                let cmd = str_arg(i, "command")?;
                let dir = resolve(cwd, i.get("folder").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).unwrap_or("."));
                if !dir.is_dir() {
                    return Err(format!("{} ist kein Ordner", dir.display()));
                }
                effect(format!("Terminal-Tab geöffnet, Befehl läuft in {} – Ausgabe mit read_terminal lesen", dir.display()), Effect::Shell(dir, cmd.to_string()))
            }
            other => return Err(format!("Unbekanntes Werkzeug „{other}“")),
        })
    };
    r().unwrap_or_else(err)
}

pub struct Outcome {
    pub text: String,
    pub is_error: bool,
    pub op: Option<Op>,
    pub effect: Option<Effect>,
}

fn ok(text: impl Into<String>) -> Outcome {
    Outcome { text: text.into(), is_error: false, op: None, effect: None }
}

fn err(text: impl Into<String>) -> Outcome {
    Outcome { text: text.into(), is_error: true, op: None, effect: None }
}

/// Liest eine Textdatei vollständig (bis 5 MB); Binärdateien werden abgelehnt.
fn read_text(p: &Path) -> Result<String, String> {
    let b = fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if b.len() > 5 * 1024 * 1024 {
        return Err("Datei größer als 5 MB – zu groß zum Bearbeiten".into());
    }
    if b.iter().take(8192).any(|&c| c == 0) {
        return Err("Binärdatei – kann nicht als Text bearbeitet werden".into());
    }
    String::from_utf8(b).map_err(|_| "Datei ist kein UTF-8-Text".to_string())
}

fn resolve(cwd: &Path, p: &str) -> PathBuf {
    let pb = PathBuf::from(p.trim());
    if pb.is_absolute() {
        pb
    } else {
        cwd.join(pb)
    }
}

fn str_arg<'a>(input: &'a Value, k: &str) -> Result<&'a str, String> {
    input.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("Parameter „{k}“ fehlt"))
}

fn paths_arg(input: &Value, cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let v: Vec<PathBuf> = input.get("paths").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(|s| resolve(cwd, s)).collect();
    if v.is_empty() {
        Err("Parameter „paths“ fehlt oder ist leer".into())
    } else {
        Ok(v)
    }
}

/// Kurzbeschreibung für die Bestätigungskarte: (Titel, Zeilen).
pub fn describe(call: &Call, cwd: &Path) -> (String, Vec<String>) {
    let i = &call.input;
    let p = |k: &str| i.get(k).and_then(Value::as_str).map(|s| resolve(cwd, s).display().to_string()).unwrap_or_default();
    let list = || paths_arg(i, cwd).map(|v| v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()).unwrap_or_default();
    match call.name.as_str() {
        "create_folder" => ("Ordner erstellen".into(), vec![p("path")]),
        "view_image" => ("Bild ansehen".into(), vec![p("path")]),
        "edit_image" => ("Bild bearbeiten (lokal)".into(), vec![p("path"), i.get("instruction").and_then(Value::as_str).unwrap_or_default().to_string()]),
        "generate_image" => {
            let prompt = i.get("prompt").and_then(Value::as_str).unwrap_or_default().to_string();
            let dir = i.get("folder").and_then(Value::as_str).map(|s| resolve(cwd, s)).unwrap_or_else(|| cwd.to_path_buf());
            ("Bild erzeugen (lokal)".into(), vec![prompt, format!("→ {}", dir.display())])
        }
        "create_text_file" => {
            let n = i.get("content").and_then(Value::as_str).map_or(0, |c| c.lines().count());
            ("Textdatei erstellen".into(), vec![p("path"), format!("{n} Zeile(n) Inhalt")])
        }
        "rename" => ("Umbenennen".into(), vec![p("path"), format!("→ {}", i.get("new_name").and_then(Value::as_str).unwrap_or("?"))]),
        "edit_text_file" => {
            let s = |k: &str| i.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let mut v = vec![p("path")];
            v.extend(s("old_text").lines().take(6).map(|l| format!("− {l}")));
            v.extend(s("new_text").lines().take(6).map(|l| format!("+ {l}")));
            ("Datei bearbeiten".into(), v)
        }
        "write_text_file" => {
            let n = i.get("content").and_then(Value::as_str).map_or(0, |c| c.lines().count());
            let path = i.get("path").and_then(Value::as_str).map(|s| resolve(cwd, s));
            let verb = if path.as_ref().is_some_and(|p| p.exists()) { "Datei überschreiben" } else { "Datei erstellen" };
            (verb.into(), vec![p("path"), format!("{n} Zeile(n) Inhalt (Sicherungskopie für Rückgängig)")])
        }
        "move" => {
            let mut v = list();
            v.push(format!("→ {}", p("destination")));
            (format!("Verschieben ({})", v.len() - 1), v)
        }
        "copy" => {
            let mut v = list();
            v.push(format!("→ {}", p("destination")));
            (format!("Kopieren ({})", v.len() - 1), v)
        }
        "move_to_trash" => {
            let v = list();
            (format!("In den Papierkorb ({})", v.len()), v)
        }
        "open_file" => ("Datei öffnen".into(), vec![p("path")]),
        "list_folder" => ("Ordner ansehen".into(), vec![p("path")]),
        "read_text_file" => ("Datei lesen".into(), vec![p("path")]),
        "find_files" => ("Suchen".into(), vec![format!("{} in {}", i.get("pattern").and_then(Value::as_str).unwrap_or("?"), p("folder"))]),
        "open_folder" => ("Ordner anzeigen".into(), vec![p("path")]),
        "select_items" => ("Auswählen".into(), vec![i.get("names").map(|v| v.to_string()).unwrap_or_default()]),
        "get_current_folder" => ("Aktueller Ordner".into(), vec![]),
        "list_tabs" => ("Tabs ansehen".into(), vec![]),
        "list_commands" => ("Befehle ansehen".into(), vec![]),
        "open_tab" => (format!("Tab öffnen ({})", i.get("kind").and_then(Value::as_str).unwrap_or("folder")), vec![p("path")]),
        "switch_tab" => ("Tab wechseln".into(), vec![format!("Tab {}", i.get("index").map(|v| v.to_string()).unwrap_or_default())]),
        "close_tab" => ("Tab schließen".into(), vec![format!("Tab {}", i.get("index").map(|v| v.to_string()).unwrap_or_default())]),
        "screenshot" => ("Bildschirmfoto".into(), vec![]),
        "read_terminal" => ("Terminal lesen".into(), vec![]),
        "mouse" => {
            let a = i.get("action").and_then(Value::as_str).unwrap_or("click");
            let label = match a {
                "move" => "Maus bewegen",
                "double_click" => "Doppelklick",
                "right_click" => "Rechtsklick",
                "scroll" => "Scrollen",
                _ => "Klicken",
            };
            (label.into(), vec![format!("bei ({}, {})", i.get("x").map(|v| v.to_string()).unwrap_or_default(), i.get("y").map(|v| v.to_string()).unwrap_or_default())])
        }
        "type_text" => ("Text tippen".into(), i.get("text").and_then(Value::as_str).unwrap_or("").lines().take(6).map(|l| format!("+ {l}")).collect()),
        "press_key" => ("Taste drücken".into(), vec![i.get("key").and_then(Value::as_str).unwrap_or("?").to_string()]),
        "run_in_terminal" => ("Befehl im Terminal".into(), vec![format!("$ {}", i.get("command").and_then(Value::as_str).unwrap_or("")), p("folder")]),
        "run_shell" => {
            let cmd = i.get("command").and_then(Value::as_str).unwrap_or("");
            let dir = i.get("folder").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map_or_else(|| cwd.display().to_string(), |s| resolve(cwd, s).display().to_string());
            let mut v: Vec<String> = cmd.lines().map(|l| format!("$ {l}")).collect();
            v.push(format!("in {dir}"));
            if let Some(why) = risky_command(cmd) {
                v.push(format!("− Riskant: {why} – bitte genau prüfen"));
            }
            ("Befehl ausführen".into(), v)
        }
        "run_command" => {
            let id = i.get("command").and_then(Value::as_str).unwrap_or("?");
            let label = crate::keymap::CMDS.iter().find(|c| c.id == id).map_or(id.to_string(), |c| format!("{} ({id})", c.label));
            ("Explorer-Befehl".into(), vec![label])
        }
        other => (other.to_string(), vec![i.to_string()]),
    }
}

/// Führt einen Werkzeugaufruf aus (im Hintergrund-Thread). Verändernde Werkzeuge nur nach Bestätigung aufrufen!
pub fn execute(call: &Call, cwd: &Path, selection: &[String]) -> Outcome {
    let i = &call.input;
    let run = || -> Result<Outcome, String> {
        Ok(match call.name.as_str() {
            "get_current_folder" => ok(json!({ "folder": cwd.display().to_string(), "selected": selection }).to_string()),
            "list_folder" => {
                let dir = resolve(cwd, str_arg(i, "path")?);
                let rd = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let mut out = Vec::new();
                for e in rd.flatten().take(500) {
                    let md = e.metadata().ok();
                    let is_dir = md.as_ref().is_some_and(|m| m.is_dir());
                    let modified = md
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default();
                    out.push(json!({
                        "name": e.file_name().to_string_lossy(),
                        "type": if is_dir { "folder" } else { "file" },
                        "size": if is_dir { None } else { md.as_ref().map(|m| m.len()) },
                        "modified": modified
                    }));
                }
                ok(json!({ "folder": dir.display().to_string(), "entries": out }).to_string())
            }
            "read_text_file" => {
                let f = resolve(cwd, str_arg(i, "path")?);
                let mut b = fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
                let cut = b.len() > 64 * 1024;
                b.truncate(64 * 1024);
                if b.iter().take(4096).any(|&c| c == 0) {
                    return Err("Binärdatei – kein Text".into());
                }
                let mut t = String::from_utf8_lossy(&b).into_owned();
                if cut {
                    t.push_str("\n… (gekürzt auf 64 KB)");
                }
                ok(t)
            }
            "find_files" => {
                let dir = resolve(cwd, str_arg(i, "folder")?);
                let re = crate::tools::rule_matcher(crate::tools::RuleKind::Name, str_arg(i, "pattern")?)?;
                let (mut hits, mut stack, mut budget) = (Vec::new(), vec![dir], 20_000usize);
                while let Some(d) = stack.pop() {
                    let Ok(rd) = fs::read_dir(&d) else { continue };
                    for e in rd.flatten() {
                        budget = budget.saturating_sub(1);
                        let Ok(ft) = e.file_type() else { continue };
                        let name = e.file_name().to_string_lossy().into_owned();
                        if re(&name, ft.is_dir(), 0, 0) {
                            hits.push(e.path().display().to_string());
                        }
                        if ft.is_dir() && !ft.is_symlink() {
                            stack.push(e.path());
                        }
                    }
                    if hits.len() >= 200 || budget == 0 {
                        break;
                    }
                }
                ok(json!({ "matches": hits, "complete": budget > 0 && hits.len() < 200 }).to_string())
            }
            "open_folder" => {
                let d = resolve(cwd, str_arg(i, "path")?);
                if !d.is_dir() {
                    return Err(format!("Kein Ordner: {}", d.display()));
                }
                Outcome { text: "angezeigt".into(), is_error: false, op: None, effect: Some(Effect::Go(d)) }
            }
            "select_items" => {
                let names: Vec<String> = i.get("names").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
                Outcome { text: format!("{} ausgewählt", names.len()), is_error: false, op: None, effect: Some(Effect::Select(names)) }
            }
            "view_image" => {
                let p = resolve(cwd, str_arg(i, "path")?);
                let b = image_b64(&p).ok_or_else(|| format!("Kein lesbares Bild: {}", p.display()))?;
                let text = format!("Bild {} – siehe angehängtes Bild.", p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
                // das Bild selbst geht über den Briefkasten mit (wie beim Bildschirmfoto)
                deliver(&call.id, (text.clone(), false, Some(b)));
                ok(text)
            }
            "list_image_models" => {
                let v: Vec<Value> = crate::localllm::image_choices().into_iter().map(|(name, _, kind, size)| json!({ "name": name, "type": kind, "size": size })).collect();
                if v.is_empty() {
                    ok("No image model installed. The user can download one in Settings → Lokale Modelle → Bildmodelle.")
                } else {
                    ok(json!({ "models": v }).to_string())
                }
            }
            "generate_image" => {
                let prompt = str_arg(i, "prompt")?.to_string();
                let choices = crate::localllm::image_choices();
                let want = i.get("model").and_then(Value::as_str).unwrap_or_default().to_lowercase();
                let pick = choices.iter().find(|c| !want.is_empty() && c.0.to_lowercase().contains(&want)).or(choices.first()).ok_or("Kein Bildmodell installiert – Einstellungen → Lokale Modelle → Bildmodelle")?;
                let (size, steps, cfg, _) = crate::localllm::choice_settings(&pick.1);
                let dim = |k: &str| i.get(k).and_then(Value::as_u64).map(|v| (v as u32).clamp(256, 2048) / 16 * 16);
                let req = crate::localllm::GenReq {
                    model: pick.1.clone(),
                    prompt,
                    negative: i.get("negative_prompt").and_then(Value::as_str).unwrap_or_default().to_string(),
                    w: dim("width").unwrap_or(size),
                    h: dim("height").unwrap_or(size),
                    steps,
                    cfg,
                    seed: (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) % 2_147_483_647) as i64,
                    count: i.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, 4) as u32,
                };
                let dir = resolve(cwd, i.get("folder").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).unwrap_or("."));
                let files = crate::localllm::generate(&req, &dir, &crate::localai::Progress::default())?;
                let list: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
                // im aktuellen Ordner die neuen Bilder auswählen, sonst den Zielordner zeigen
                let effect = if dir == cwd { Effect::Select(files.iter().filter_map(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).collect()) } else { Effect::Go(dir) };
                Outcome { text: format!("Erzeugt mit {}: {}", pick.0, list.join(", ")), is_error: false, op: None, effect: Some(effect) }
            }
            "edit_image" => {
                let src = resolve(cwd, str_arg(i, "path")?);
                let instruction = str_arg(i, "instruction")?.to_string();
                let m = crate::localllm::EDIT.iter().find(|m| m.installed()).ok_or("Kein Bildbearbeitungsmodell installiert – im KI-Modus „Bild“ oben in der Modellwahl unter „Vorlage bearbeiten“ laden (z. B. FLUX.2 klein)")?;
                let img = image::open(&src).map_err(|e| format!("{}: {e}", src.display()))?.to_rgba8();
                let seed = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) % 2_147_483_647) as i64;
                let out = crate::localllm::edit_image(m, &img, &instruction, seed, &crate::localai::Progress::default())?;
                let dir = src.parent().map(Path::to_path_buf).unwrap_or_else(|| cwd.to_path_buf());
                let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Bild".into());
                let mut file = dir.join(format!("{stem}_KI.png"));
                let mut n = 2;
                while file.exists() {
                    file = dir.join(format!("{stem}_KI_{n}.png"));
                    n += 1;
                }
                out.save(&file).map_err(|e| e.to_string())?;
                let effect = if dir == cwd { Effect::Select(file.file_name().map(|n| vec![n.to_string_lossy().into_owned()]).unwrap_or_default()) } else { Effect::Go(dir) };
                Outcome { text: format!("Bearbeitet mit {}: {}", m.name, file.display()), is_error: false, op: None, effect: Some(effect) }
            }
            "create_folder" => {
                let p = resolve(cwd, str_arg(i, "path")?);
                if p.exists() {
                    return Err(format!("Existiert bereits: {}", p.display()));
                }
                fs::create_dir_all(&p).map_err(|e| e.to_string())?;
                Outcome { text: format!("erstellt: {}", p.display()), is_error: false, op: Some(Op::Create { path: p, dir: true }), effect: None }
            }
            "create_text_file" => {
                let p = resolve(cwd, str_arg(i, "path")?);
                let content = i.get("content").and_then(Value::as_str).unwrap_or("");
                use std::io::Write;
                let mut f = fs::File::create_new(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
                Outcome { text: format!("erstellt: {}", p.display()), is_error: false, op: Some(Op::Create { path: p, dir: false }), effect: None }
            }
            "edit_text_file" | "write_text_file" => {
                let p = resolve(cwd, str_arg(i, "path")?);
                let new_content = if call.name == "edit_text_file" {
                    let old = i.get("old_text").and_then(Value::as_str).ok_or("Parameter „old_text“ fehlt")?;
                    let new = i.get("new_text").and_then(Value::as_str).unwrap_or("");
                    let text = read_text(&p)?;
                    match text.matches(old).count() {
                        0 => return Err("old_text kommt in der Datei nicht vor – Datei erneut lesen".into()),
                        1 => text.replacen(old, new, 1),
                        n => return Err(format!("old_text kommt {n}-mal vor – mehr Kontext angeben")),
                    }
                } else {
                    i.get("content").and_then(Value::as_str).ok_or("Parameter „content“ fehlt")?.to_string()
                };
                if p.exists() {
                    read_text(&p)?; // nur Textdateien ersetzen
                    let before = ops::snapshot(&p)?;
                    fs::write(&p, &new_content).map_err(|e| format!("{}: {e}", p.display()))?;
                    let after = ops::snapshot(&p)?;
                    Outcome { text: format!("gespeichert: {}", p.display()), is_error: false, op: Some(Op::Edit { path: p, before, after }), effect: None }
                } else {
                    if let Some(par) = p.parent() {
                        fs::create_dir_all(par).map_err(|e| e.to_string())?;
                    }
                    fs::write(&p, &new_content).map_err(|e| format!("{}: {e}", p.display()))?;
                    Outcome { text: format!("erstellt: {}", p.display()), is_error: false, op: Some(Op::Create { path: p, dir: false }), effect: None }
                }
            }
            "rename" => {
                let from = resolve(cwd, str_arg(i, "path")?);
                let name = str_arg(i, "new_name")?;
                ops::check_name(name)?;
                let to = from.parent().ok_or("Ungültiger Pfad")?.join(name);
                ops::rename_safe(&from, &to)?;
                Outcome { text: format!("umbenannt in {name}"), is_error: false, op: Some(Op::Rename { from, to }), effect: None }
            }
            "move" | "copy" => {
                let copy = call.name == "copy";
                let srcs = paths_arg(i, cwd)?;
                let dest = resolve(cwd, str_arg(i, "destination")?);
                fs::create_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
                let t = ops::transfer(&srcs, &dest, copy, Policy::KeepBoth, &AtomicU64::new(0), &AtomicBool::new(false));
                let mut text = format!("{} Element(e) {}", t.pairs.len(), if copy { "kopiert" } else { "verschoben" });
                if !t.errors.is_empty() {
                    text.push_str(&format!("; Fehler: {}", t.errors.join("; ")));
                }
                let op = (!t.pairs.is_empty()).then(|| if copy { Op::Copy { pairs: t.pairs, replaced: t.replaced } } else { Op::Move { pairs: t.pairs, replaced: t.replaced } });
                Outcome { is_error: op.is_none(), text, op, effect: None }
            }
            "move_to_trash" => {
                let srcs = paths_arg(i, cwd)?;
                crate::trashx::delete(&srcs)?;
                let done: Vec<PathBuf> = srcs.into_iter().filter(|p| !ops::exists(p)).collect();
                Outcome { text: format!("{} Element(e) im Papierkorb", done.len()), is_error: done.is_empty(), op: Some(Op::Trash { paths: done }), effect: None }
            }
            "open_file" => {
                let p = resolve(cwd, str_arg(i, "path")?);
                if !p.is_file() {
                    return Err(format!("Keine Datei: {}", p.display()));
                }
                Outcome { text: "geöffnet".into(), is_error: false, op: None, effect: Some(Effect::OpenFile(p)) }
            }
            "run_shell" => {
                let cmd = str_arg(i, "command")?;
                let dir = resolve(cwd, i.get("folder").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).unwrap_or("."));
                if !dir.is_dir() {
                    return Err(format!("{} ist kein Ordner", dir.display()));
                }
                let secs = i.get("timeout_seconds").and_then(Value::as_u64).unwrap_or(120).clamp(1, 900);
                let (code, out) = shell(&call.id, cmd, &dir, Duration::from_secs(secs))?;
                let text = match code {
                    Some(c) => format!("Exit-Code {c}\n{out}"),
                    None => format!("Nach {secs} s abgebrochen (Zeitlimit)\n{out}"),
                };
                Outcome { is_error: code != Some(0), text, op: None, effect: None }
            }
            other => return Err(format!("Unbekanntes Werkzeug „{other}“")),
        })
    };
    run().unwrap_or_else(err)
}

// ───────────────────────────── Shell im Hintergrund ─────────────────────────────

/// Laufende Ausgaben von run_shell (Aufruf-ID → bisheriger Text), damit der Chat sie live zeigen kann.
fn live_map() -> &'static std::sync::Mutex<HashMap<String, String>> {
    static M: std::sync::OnceLock<std::sync::Mutex<HashMap<String, String>>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

/// Bisherige Ausgabe eines laufenden Befehls.
pub fn live_output(id: &str) -> Option<String> {
    live_map().lock().ok()?.get(id).cloned()
}

/// Höchstlänge der Ausgabe, die an das Modell geht (Anfang und Ende bleiben erhalten).
const SHELL_MAX: usize = 30_000;

/// Führt `cmd` ohne Fenster aus (Windows: PowerShell mit UTF-8-Ausgabe) und sammelt stdout + stderr.
/// Rückgabe: (Exit-Code, `None` bei Zeitüberschreitung; Ausgabe).
pub fn shell(id: &str, cmd: &str, dir: &Path, timeout: Duration) -> Result<(Option<i32>, String), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut c = if cfg!(windows) {
        let mut c = Command::new("powershell.exe");
        // RemoteSigned: lokale Skripte (z. B. npm.ps1) laufen, heruntergeladene nur signiert – kein „Bypass“
        c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "RemoteSigned", "-Command"]);
        c.arg(format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; $OutputEncoding=[Text.Encoding]::UTF8; $ProgressPreference='SilentlyContinue'; {cmd}"));
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    c.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // nie auf Eingaben warten: keine Pager, keine Passwortabfragen
    c.env("GIT_TERMINAL_PROMPT", "0").env("GCM_INTERACTIVE", "never").env("GIT_PAGER", "cat").env("PAGER", "cat").env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let mut child = c.spawn().map_err(|e| format!("Shell konnte nicht gestartet werden: {e}"))?;
    let buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let pump = |r: Box<dyn Read + Send>| {
        let (buf, id) = (buf.clone(), id.to_string());
        std::thread::spawn(move || {
            let mut r = r;
            let mut chunk = [0u8; 4096];
            while let Ok(n) = r.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&chunk[..n]).replace("\r\n", "\n");
                if let Ok(mut b) = buf.lock() {
                    b.push_str(&text);
                    if let Ok(mut m) = live_map().lock() {
                        m.insert(id.clone(), b.clone());
                    }
                }
            }
        })
    };
    let t1 = child.stdout.take().map(|o| pump(Box::new(o)));
    let t2 = child.stderr.take().map(|e| pump(Box::new(e)));
    let start = std::time::Instant::now();
    let code = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st.code().or(Some(-1));
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    for t in [t1, t2].into_iter().flatten() {
        let _ = t.join();
    }
    if let Ok(mut m) = live_map().lock() {
        m.remove(id);
    }
    let mut out = buf.lock().map(|b| b.trim_end().to_string()).unwrap_or_default();
    if out.chars().count() > SHELL_MAX {
        let v: Vec<char> = out.chars().collect();
        out = format!("{}\n… ({} Zeichen ausgelassen) …\n{}", v[..SHELL_MAX / 3].iter().collect::<String>(), v.len() - SHELL_MAX, v[v.len() - SHELL_MAX * 2 / 3..].iter().collect::<String>());
    }
    if out.is_empty() {
        out = "(keine Ausgabe)".into();
    }
    Ok((code, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Mini-HTTP-Server: nimmt eine Anfrage an, gibt sie zurück und antwortet mit `reply`.
    fn mock(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 65536];
            loop {
                let n = s.read(&mut tmp).unwrap();
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(i) = text.find("\r\n\r\n") {
                    let len = text.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                    if buf.len() >= i + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len());
            s.write_all(resp.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf).to_string()
        });
        (addr, h)
    }

    #[test]
    fn anthropic_wire_format() {
        let (addr, h) = mock(r#"{"content":[{"type":"thinking","thinking":"","signature":"x"},{"type":"text","text":"Ich sehe nach."},{"type":"tool_use","id":"toolu_1","name":"list_folder","input":{"path":"."}}],"stop_reason":"tool_use"}"#);
        // Fallbacks nur gegen die offizielle API – hier lokaler Testserver
        let cfg = Cfg { kind: Kind::Anthropic, model: "claude-opus-5-5".into(), base: addr, key: Some("sk-test".into()), local: true, editor: true, effort: "high".into(), plain: false };
        let hist = vec![user_entry("Was liegt hier?", Path::new("C:\\Daten"), &[], true)];
        let t = request(&cfg, &hist).unwrap();
        let req = h.join().unwrap();
        let lower = req.to_lowercase();
        assert!(lower.starts_with("post /v1/messages"));
        assert!(lower.contains("anthropic-version: 2026-10-21"));
        assert!(lower.contains("x-api-key: sk-test"));
        assert!(!lower.contains("anthropic-beta"));
        let body: Value = serde_json::from_str(&req[req.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(body["model"], "claude-opus-5-5");
        assert!(body.get("fallbacks").is_none());
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("[Mode: agent]"));
        // Explorer-KI: Datei- und Tab-/Befehlswerkzeuge, Denkaufwand wie gewählt
        assert_eq!(body["tools"].as_array().unwrap().len(), TOOLS.len() + EDITOR_TOOLS.len());
        assert_eq!(body["output_config"]["effort"], "high");
        assert!(body["system"].as_str().unwrap().contains("list_tabs"));
        assert!(body["tools"][0]["input_schema"].is_object());
        assert!(body.get("temperature").is_none());
        // Antwort: Text, Werkzeugaufruf, Inhalt (inkl. Denk-Block) unverändert für den Verlauf
        assert_eq!(t.stop, "tool_use");
        assert_eq!(t.text, "Ich sehe nach.");
        assert_eq!(t.calls[0].name, "list_folder");
        assert_eq!(t.history_entry["content"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn openai_compatible_wire_format() {
        let (addr, h) = mock(
            r#"{"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"move","arguments":"{\"paths\":[\"a.txt\"],\"destination\":\"x\"}"}}]}}]}"#,
        );
        let cfg = Cfg { kind: Kind::OpenAi, model: "qwen".into(), base: format!("{addr}/v1"), key: None, local: true, editor: false, effort: "medium".into(), plain: false };
        let t = request(&cfg, &[json!({ "role": "user", "content": "hi" })]).unwrap();
        let req = h.join().unwrap();
        assert!(req.to_lowercase().starts_with("post /v1/chat/completions"));
        assert!(!req.to_lowercase().contains("authorization:"));
        let body: Value = serde_json::from_str(&req[req.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(t.stop, "tool_use");
        assert_eq!(t.calls[0].input["destination"], "x");
    }

    #[test]
    fn profiles_roundtrip() {
        let p = ModelProfile { id: "m1".into(), name: "Mein|LM".into(), kind: Kind::OpenAi, url: "http://localhost:1234/v1".into(), model: "qwen".into() };
        let back = ModelProfile::decode(&p.encode()).unwrap();
        assert_eq!(back.name, "Mein LM");
        assert!(back.is_local() && !back.needs_key());
        assert_eq!(ModelProfile::new_id(&[p]), "m2");
        assert!(default_profiles()[0].needs_key());
    }

    #[test]
    fn shell_and_risky_commands() {
        assert!(risky_command("Remove-Item -Recurse build").is_some());
        assert!(risky_command("git push origin main").is_some());
        assert!(risky_command("git reset --hard HEAD~1").is_some());
        assert!(risky_command("winget install foo").is_some());
        assert!(risky_command("git status").is_none());
        assert!(risky_command("git diff --stat").is_none());
        assert!(risky_command("cargo build").is_none());
        assert!(risky_command("git log -5 --oneline").is_none());
        // Hintergrund-Befehl: Ausgabe und Exit-Code, ohne Fenster
        let d = std::env::temp_dir();
        let call = Call { id: "t1".into(), name: "run_shell".into(), input: json!({ "command": "echo foldex-ok", "folder": d.display().to_string() }) };
        let o = execute(&call, &d, &[]);
        assert!(!o.is_error, "{}", o.text);
        assert!(o.text.contains("foldex-ok") && o.text.starts_with("Exit-Code 0"), "{}", o.text);
        let bad = Call { id: "t2".into(), name: "run_shell".into(), input: json!({ "command": "exit 3" }) };
        let o = execute(&bad, &d, &[]);
        assert!(o.is_error && o.text.starts_with("Exit-Code 3"), "{}", o.text);
        // reiner Chat: keine Werkzeuge
        let mut cfg = Cfg::from_profile(&default_profiles()[0], None);
        cfg.plain = true;
        assert_eq!(tools_for(&cfg).count(), 0);
        assert!(system_for(&cfg).contains("no tools"));
        cfg.plain = false;
        assert!(tools_for(&cfg).any(|t| t.name == "run_shell"));
        assert!(needs_ok(&call));
    }

    #[test]
    fn tools_and_execution() {
        // jedes verändernde Werkzeug ist als bestätigungspflichtig markiert
        for t in TOOLS {
            let mutates = matches!(t.name, "create_folder" | "create_text_file" | "generate_image" | "edit_image" | "write_text_file" | "edit_text_file" | "rename" | "move" | "copy" | "move_to_trash" | "open_file" | "run_shell");
            assert_eq!(t.needs_ok, mutates, "{}", t.name);
            assert_eq!(tool_category(t.name).is_some(), mutates, "{}", t.name);
            assert!((t.schema)().get("type").is_some());
        }
        let base = std::env::temp_dir().join(format!("fc-ai-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let call = |name: &str, input: Value| Call { id: "1".into(), name: name.into(), input };
        let r = execute(&call("create_text_file", json!({ "path": "a.txt", "content": "hallo" })), &base, &[]);
        assert!(!r.is_error, "{}", r.text);
        // nie überschreiben
        assert!(execute(&call("create_text_file", json!({ "path": "a.txt", "content": "x" })), &base, &[]).is_error);
        assert_eq!(fs::read_to_string(base.join("a.txt")).unwrap(), "hallo");
        assert!(!execute(&call("rename", json!({ "path": "a.txt", "new_name": "b.txt" })), &base, &[]).is_error);
        assert!(execute(&call("rename", json!({ "path": "b.txt", "new_name": "../x.txt" })), &base, &[]).is_error);
        let m = execute(&call("move", json!({ "paths": ["b.txt"], "destination": "sub" })), &base, &[]);
        assert!(!m.is_error && matches!(m.op, Some(Op::Move { .. })), "{}", m.text);
        assert!(base.join("sub/b.txt").is_file());
        let l = execute(&call("list_folder", json!({ "path": "sub" })), &base, &[]);
        assert!(l.text.contains("b.txt"));
        let f = execute(&call("find_files", json!({ "folder": ".", "pattern": "*.TXT" })), &base, &[]);
        assert!(f.text.contains("b.txt"));
        assert!(execute(&call("read_text_file", json!({ "path": "sub/b.txt" })), &base, &[]).text.contains("hallo"));
        // Bearbeiten: genau ein Treffer, Sicherungskopien für Rückgängig
        let e = execute(&call("edit_text_file", json!({ "path": "sub/b.txt", "old_text": "hallo", "new_text": "servus" })), &base, &[]);
        assert!(!e.is_error, "{}", e.text);
        assert_eq!(fs::read_to_string(base.join("sub/b.txt")).unwrap(), "servus");
        match e.op {
            Some(Op::Edit { ref before, .. }) => assert_eq!(fs::read_to_string(before).unwrap(), "hallo"),
            _ => panic!("kein Edit"),
        }
        ops::undo(e.op.as_ref().unwrap()).unwrap();
        assert_eq!(fs::read_to_string(base.join("sub/b.txt")).unwrap(), "hallo");
        assert!(execute(&call("edit_text_file", json!({ "path": "sub/b.txt", "old_text": "fehlt", "new_text": "x" })), &base, &[]).is_error);
        let w = execute(&call("write_text_file", json!({ "path": "neu/c.md", "content": "# Titel" })), &base, &[]);
        assert!(!w.is_error && matches!(w.op, Some(Op::Create { .. })), "{}", w.text);
        let _ = fs::remove_dir_all(&base);
        // Ergebnisformate je Anbieter
        let r = results_entries(Kind::Anthropic, &[("t1".into(), "ok".into(), false, None), ("t2".into(), "nein".into(), true, Some("QUJD".into()))]);
        assert_eq!(r[0]["content"][1]["content"][1]["source"]["media_type"], "image/jpeg");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0]["content"][1]["is_error"], json!(true));
        assert_eq!(results_entries(Kind::OpenAi, &[("t1".into(), "ok".into(), false, None)])[0]["role"], "tool");
        let o = results_entries(Kind::OpenAi, &[("t1".into(), "".into(), false, Some("QUJD".into()))]);
        assert_eq!(o[1]["role"], "user");
        assert_eq!(parse_key("Ctrl+Shift+t"), Some((true, true, false, "T".into())));
        assert!(parse_key("Hyper+X").is_none());
    }
}
