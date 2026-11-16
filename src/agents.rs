//! KI-Agenten für die Kommandozeile (Claude Code, Codex, Gemini CLI, Aider, OpenCode): Erkennung, Version,
//! Installation und – vor allem – der Betrieb als Chat ohne Terminalfenster. Jede Nachricht startet den
//! Agenten im aktuellen Ordner im Nicht-interaktiv-Modus; seine Ausgabe (bei Claude Code und Codex ein
//! JSON-Ereignisstrom) wird in Text, Werkzeugschritte und Ergebnis zerlegt. Gespräche werden über die
//! Sitzungs-ID des Agenten fortgesetzt.

use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
};

pub struct AgentDef {
    pub key: &'static str,
    pub name: &'static str,
    /// Befehl im PATH
    pub cmd: &'static str,
    /// Installationsbefehl (Windows: PowerShell, sonst sh)
    pub install_win: &'static str,
    pub install_unix: &'static str,
    /// Vorschläge für die Modellauswahl („“ = Standard des Agenten)
    pub models: &'static [&'static str],
    pub url: &'static str,
}

pub const AGENTS: &[AgentDef] = &[
    AgentDef {
        key: "claude",
        name: "Claude Code",
        cmd: "claude",
        install_win: "irm https://claude.ai/install.ps1 | iex",
        install_unix: "curl -fsSL https://claude.ai/install.sh | bash",
        models: &["", "opus", "sonnet", "haiku", "fable"],
        url: "https://claude.com/claude-code",
    },
    AgentDef {
        key: "codex",
        name: "OpenAI Codex",
        cmd: "codex",
        install_win: "npm install -g @openai/codex",
        install_unix: "npm install -g @openai/codex",
        models: &[""],
        url: "https://github.com/openai/codex",
    },
    AgentDef {
        key: "gemini",
        name: "Gemini CLI",
        cmd: "gemini",
        install_win: "npm install -g @google/gemini-cli",
        install_unix: "npm install -g @google/gemini-cli",
        models: &["", "gemini-2.5-pro", "gemini-2.5-flash"],
        url: "https://github.com/google-gemini/gemini-cli",
    },
    AgentDef {
        key: "aider",
        name: "Aider",
        cmd: "aider",
        install_win: "python -m pip install aider-install; aider-install",
        install_unix: "python3 -m pip install aider-install && aider-install",
        models: &[""],
        url: "https://aider.chat",
    },
    AgentDef {
        key: "opencode",
        name: "OpenCode",
        cmd: "opencode",
        install_win: "npm install -g opencode-ai",
        install_unix: "npm install -g opencode-ai",
        models: &[""],
        url: "https://opencode.ai",
    },
];

pub fn def(key: &str) -> Option<&'static AgentDef> {
    AGENTS.iter().find(|a| a.key == key)
}

/// Programmpfad eines Agenten (PATH, bei Claude Code auch der Ordner des offiziellen Installers).
pub fn find(a: &AgentDef) -> Option<PathBuf> {
    crate::editors::which(a.cmd).or_else(|| {
        (a.key == "claude")
            .then(|| dirs::home_dir().map(|h| h.join(".local").join("bin").join(if cfg!(windows) { "claude.exe" } else { "claude" })))
            .flatten()
            .filter(|p| p.is_file())
    })
}

/// Installierte Agenten: (Schlüssel, Name, Programm).
pub fn detect() -> Vec<(String, String, String)> {
    AGENTS.iter().filter_map(|a| find(a).map(|p| (a.key.to_string(), a.name.to_string(), p.to_string_lossy().into_owned()))).collect()
}

/// Ist dieses Programm ein bekannter Agent? (Symbol und Titel im Terminal-Tab)
pub fn agent_name(program: &str) -> Option<&'static str> {
    let stem = Path::new(program).file_stem()?.to_string_lossy().to_lowercase();
    AGENTS.iter().find(|a| a.cmd == stem).map(|a| a.name)
}

fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // kein Konsolenfenster
    }
    c
}

/// Version (erste Zeile von `--version`) – blockiert kurz, nur im Hintergrund aufrufen.
pub fn version(program: &str) -> Option<String> {
    let out = command(program).arg("--version").stdin(Stdio::null()).output().ok()?;
    let t = String::from_utf8_lossy(&out.stdout);
    t.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| l.chars().take(40).collect())
}

/// Shell + Argumente für die Installation (läuft sichtbar in einem Terminal-Tab).
pub fn install_command(a: &AgentDef) -> (String, Vec<String>) {
    if cfg!(windows) {
        ("powershell.exe".into(), vec!["-NoProfile".into(), "-ExecutionPolicy".into(), "RemoteSigned".into(), "-Command".into(), a.install_win.into()])
    } else {
        ("sh".into(), vec!["-c".into(), a.install_unix.into()])
    }
}

// ───────────────────────────── Chat-Betrieb ─────────────────────────────

/// Was ein Agent im Chat darf.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    /// reiner Chat: keine Werkzeuge (läuft in einem leeren Ordner, siehe `chat_dir`)
    Chat,
    /// nur lesen und planen
    Read,
    /// Dateien bearbeiten
    Edit,
    /// zusätzlich Befehle ausführen, ohne zu fragen
    Full,
}

/// Ereignisse eines Laufs.
#[derive(Clone, Debug)]
pub enum Ev {
    /// Sitzungs-ID zum Fortsetzen
    Session(String),
    Text(String),
    /// Werkzeugschritt: (Werkzeug, Detail)
    Tool(String, String),
    /// Ergebnis eines Werkzeugschritts (Kurzfassung, Fehler?)
    ToolResult(String, bool),
    /// Ende: Kosten in USD (falls bekannt), Fehlermeldung
    Done(Option<f64>, Option<String>),
}

/// Laufender Agent – zum Abbrechen.
#[derive(Clone, Default)]
pub struct Handle(Arc<Mutex<Option<Child>>>);

impl Handle {
    pub fn cancel(&self) {
        if let Ok(mut g) = self.0.lock() {
            if let Some(c) = g.as_mut() {
                let _ = c.kill();
            }
        }
    }
}

/// Einzelne Frage an einen Agenten ohne Werkzeuge (z. B. Commit-Nachricht); blockiert bis zur Antwort –
/// nur im Hintergrund aufrufen. Gibt den gesamten Antworttext zurück.
pub fn oneshot(key: &str, program: &str, prompt: &str, ctx: &eframe::egui::Context) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    let _h = run(key, program, prompt, "", &chat_dir(), "", "", Access::Chat, None, tx, ctx)?;
    let mut text = String::new();
    for ev in rx {
        match ev {
            Ev::Text(t) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t);
            }
            Ev::Done(_, Some(e)) => return Err(e),
            Ev::Done(..) => break,
            _ => {}
        }
    }
    if text.trim().is_empty() {
        Err("Keine Antwort vom Agenten".into())
    } else {
        Ok(text)
    }
}

/// Leerer Arbeitsordner für den reinen Chat – so sieht ein Agent keine Dateien des Benutzers.
pub fn chat_dir() -> PathBuf {
    let d = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("chat");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Startet eine Chat-Runde. `resume` = Sitzungs-ID der vorigen Runde (Claude Code, Codex).
/// Agenten ohne Fortsetzung bekommen `transcript` (die letzten Nachrichten) vorangestellt.
#[allow(clippy::too_many_arguments)]
pub fn run(
    key: &str,
    program: &str,
    prompt: &str,
    transcript: &str,
    cwd: &Path,
    model: &str,
    effort: &str,
    access: Access,
    resume: Option<&str>,
    tx: mpsc::Sender<Ev>,
    ctx: &eframe::egui::Context,
) -> Result<Handle, String> {
    let mut c = command(program);
    c.current_dir(cwd).stdout(Stdio::piped()).stderr(Stdio::piped());
    let model = model.trim();
    // Prompt über die Standardeingabe (Claude Code, Codex) oder als Argument
    let mut stdin_prompt: Option<String> = None;
    let one_line = |s: &str| s.replace(['\r', '\n'], " ");
    let full = if transcript.is_empty() { prompt.to_string() } else { format!("{transcript}\n\nUser: {prompt}") };
    match key {
        "claude" => {
            c.args(["-p", "--output-format", "stream-json", "--verbose", "--permission-mode"]);
            c.arg(match access {
                Access::Chat | Access::Read => "plan",
                Access::Edit => "acceptEdits",
                Access::Full => "bypassPermissions",
            });
            if access == Access::Chat {
                // keine Werkzeuge: nur antworten
                c.args(["--disallowedTools", "Bash,Edit,Write,Read,Glob,Grep,NotebookEdit,WebFetch,WebSearch,Task,TodoWrite"]);
            }
            if !model.is_empty() {
                c.args(["--model", model]);
            }
            // Denkaufwand: low, medium, high, xhigh, max
            if !effort.is_empty() {
                c.args(["--effort", effort]);
            }
            if let Some(id) = resume {
                c.args(["--resume", id]);
            }
            stdin_prompt = Some(prompt.to_string());
        }
        "codex" => {
            c.arg("exec");
            if let Some(id) = resume {
                c.args(["resume", id]);
            }
            c.args(["--json", "--skip-git-repo-check", "--sandbox"]);
            c.arg(match access {
                Access::Chat | Access::Read => "read-only",
                Access::Edit => "workspace-write",
                Access::Full => "danger-full-access",
            });
            if !model.is_empty() {
                c.args(["-m", model]);
            }
            if !effort.is_empty() {
                // Codex kennt kein „max“ – die höchste Stufe heißt dort „xhigh“
                let e = if effort == "max" { "xhigh" } else { effort };
                c.args(["-c", &format!("model_reasoning_effort=\"{e}\"")]);
            }
            c.arg("-");
            stdin_prompt = Some(prompt.to_string());
        }
        "gemini" => {
            c.args(["--output-format", "json", "--approval-mode"]);
            c.arg(match access {
                Access::Chat | Access::Read => "default",
                Access::Edit => "auto_edit",
                Access::Full => "yolo",
            });
            if !model.is_empty() {
                c.args(["-m", model]);
            }
            c.args(["-p", &one_line(&full)]);
        }
        "aider" => {
            c.args(["--message", &one_line(&full), "--yes-always", "--no-pretty", "--no-stream", "--no-check-update"]);
            if matches!(access, Access::Read | Access::Chat) {
                c.args(["--chat-mode", "ask"]);
            }
            if access == Access::Chat {
                c.args(["--no-git", "--no-auto-commits"]);
            }
            if !model.is_empty() {
                c.args(["--model", model]);
            }
        }
        _ => {
            c.args(["run", &one_line(&full)]);
            if !model.is_empty() {
                c.args(["-m", model]);
            }
        }
    }
    c.stdin(if stdin_prompt.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = c.spawn().map_err(|e| format!("{program} konnte nicht gestartet werden: {e}"))?;
    if let (Some(p), Some(mut si)) = (stdin_prompt, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = si.write_all(p.as_bytes());
        });
    }
    let stdout = child.stdout.take().ok_or("keine Ausgabe")?;
    let mut stderr = child.stderr.take().ok_or("keine Fehlerausgabe")?;
    let handle = Handle(Arc::new(Mutex::new(Some(child))));
    let (h2, key, ctx) = (handle.clone(), key.to_string(), ctx.clone());
    std::thread::spawn(move || {
        let err_text = Arc::new(Mutex::new(String::new()));
        let et = err_text.clone();
        let errt = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            *et.lock().unwrap() = s;
        });
        let mut done = false;
        let mut plain = String::new();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let parsed = serde_json::from_str::<Value>(&line).ok();
            let evs = match (&parsed, key.as_str()) {
                (Some(v), "claude") => parse_claude(v),
                (Some(v), "codex") => parse_codex(v),
                (Some(v), "gemini") => v.get("response").and_then(Value::as_str).map(|t| vec![Ev::Text(t.to_string())]).unwrap_or_default(),
                _ => {
                    // Klartext-Agenten: Ausgabe sammeln
                    plain.push_str(&line);
                    plain.push('\n');
                    Vec::new()
                }
            };
            for e in evs {
                done |= matches!(e, Ev::Done(..));
                let _ = tx.send(e);
            }
            ctx.request_repaint();
        }
        // Prozessende abwarten, ohne die Sperre zu halten (sonst blockiert „Abbrechen“)
        let mut status = None;
        for _ in 0..250 {
            let s = h2.0.lock().ok().and_then(|mut g| g.as_mut().and_then(|c| c.try_wait().ok().flatten()));
            if s.is_some() {
                status = s;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = errt.join();
        if !plain.trim().is_empty() {
            let _ = tx.send(Ev::Text(strip_ansi(plain.trim())));
        }
        if !done {
            let err = err_text.lock().map(|s| s.trim().to_string()).unwrap_or_default();
            let failed = status.is_some_and(|s| !s.success());
            let msg = if failed {
                Some(if err.is_empty() { "Der Agent wurde mit einem Fehler beendet (oder abgebrochen).".into() } else { tail(&strip_ansi(&err), 1200) })
            } else {
                None
            };
            let _ = tx.send(Ev::Done(None, msg));
        }
        ctx.request_repaint();
    });
    Ok(handle)
}

fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}

/// Farbcodes (ANSI) aus Klartext-Ausgaben entfernen.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Kurzbeschreibung eines Werkzeugaufrufs aus seinen Parametern.
fn tool_detail(input: &Value) -> String {
    for k in ["file_path", "path", "command", "pattern", "url", "query", "description", "prompt"] {
        if let Some(s) = input.get(k).and_then(Value::as_str) {
            return s.lines().next().unwrap_or("").chars().take(160).collect();
        }
    }
    String::new()
}

/// Claude Code `--output-format stream-json`.
pub fn parse_claude(v: &Value) -> Vec<Ev> {
    let mut out = Vec::new();
    match v.get("type").and_then(Value::as_str) {
        Some("system") => {
            if let Some(id) = v.get("session_id").and_then(Value::as_str) {
                out.push(Ev::Session(id.to_string()));
            }
        }
        Some("assistant") => {
            for b in v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                            out.push(Ev::Text(t.to_string()));
                        }
                    }
                    Some("tool_use") => out.push(Ev::Tool(
                        b.get("name").and_then(Value::as_str).unwrap_or("Werkzeug").to_string(),
                        b.get("input").map(tool_detail).unwrap_or_default(),
                    )),
                    _ => {}
                }
            }
        }
        Some("user") => {
            for b in v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                if b.get("type").and_then(Value::as_str) == Some("tool_result") {
                    let text = match b.get("content") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Array(a)) => a.iter().filter_map(|x| x.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
                        _ => String::new(),
                    };
                    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(160).collect();
                    out.push(Ev::ToolResult(first, b.get("is_error").and_then(Value::as_bool).unwrap_or(false)));
                }
            }
        }
        Some("result") => {
            if let Some(id) = v.get("session_id").and_then(Value::as_str) {
                out.push(Ev::Session(id.to_string()));
            }
            let err = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let msg = err.then(|| v.get("result").and_then(Value::as_str).unwrap_or("Fehler").to_string());
            out.push(Ev::Done(v.get("total_cost_usd").and_then(Value::as_f64), msg));
        }
        _ => {}
    }
    out
}

/// Codex `exec --json` (JSONL-Ereignisse).
pub fn parse_codex(v: &Value) -> Vec<Ev> {
    let mut out = Vec::new();
    match v.get("type").and_then(Value::as_str) {
        Some("thread.started") | Some("session.created") => {
            if let Some(id) = v.get("thread_id").or_else(|| v.get("session_id")).and_then(Value::as_str) {
                out.push(Ev::Session(id.to_string()));
            }
        }
        Some("item.completed") => {
            let item = v.get("item").cloned().unwrap_or(Value::Null);
            match item.get("type").or_else(|| item.get("item_type")).and_then(Value::as_str) {
                Some("agent_message") | Some("assistant_message") => {
                    if let Some(t) = item.get("text").and_then(Value::as_str) {
                        out.push(Ev::Text(t.to_string()));
                    }
                }
                Some("command_execution") => {
                    out.push(Ev::Tool("Befehl".into(), item.get("command").and_then(Value::as_str).unwrap_or("").to_string()));
                    let code = item.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
                    let o = item.get("aggregated_output").and_then(Value::as_str).unwrap_or("");
                    out.push(Ev::ToolResult(o.lines().next().unwrap_or("").chars().take(160).collect(), code != 0));
                }
                Some("file_change") => {
                    let paths: Vec<String> = item
                        .get("changes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|c| c.get("path").and_then(Value::as_str).map(str::to_string))
                        .collect();
                    out.push(Ev::Tool("Dateien geändert".into(), paths.join(", ")));
                }
                _ => {}
            }
        }
        Some("error") | Some("turn.failed") => {
            let m = v.get("message").or_else(|| v.pointer("/error/message")).and_then(Value::as_str).unwrap_or("Fehler");
            out.push(Ev::Done(None, Some(m.to_string())));
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_stream() {
        let evs: Vec<Ev> = [
            json!({"type":"system","subtype":"init","session_id":"s1","model":"claude-opus-5-5"}),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"Ich schaue nach."},{"type":"tool_use","name":"Read","input":{"file_path":"C:\\a.txt"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","content":"zeile1\nzeile2","is_error":false}]}}),
            json!({"type":"result","subtype":"success","result":"fertig","session_id":"s1","total_cost_usd":0.012,"is_error":false}),
        ]
        .iter()
        .flat_map(parse_claude)
        .collect();
        assert!(matches!(&evs[0], Ev::Session(s) if s == "s1"));
        assert!(matches!(&evs[1], Ev::Text(t) if t == "Ich schaue nach."));
        assert!(matches!(&evs[2], Ev::Tool(n, d) if n == "Read" && d == "C:\\a.txt"));
        assert!(matches!(&evs[3], Ev::ToolResult(t, false) if t == "zeile1"));
        assert!(matches!(evs.last(), Some(Ev::Done(Some(c), None)) if (*c - 0.012).abs() < 1e-9));
    }

    #[test]
    fn codex_stream() {
        let evs: Vec<Ev> = [
            json!({"type":"thread.started","thread_id":"t9"}),
            json!({"type":"item.completed","item":{"type":"command_execution","command":"ls","aggregated_output":"a\nb","exit_code":0}}),
            json!({"type":"item.completed","item":{"type":"agent_message","text":"Erledigt."}}),
        ]
        .iter()
        .flat_map(parse_codex)
        .collect();
        assert!(matches!(&evs[0], Ev::Session(s) if s == "t9"));
        assert!(matches!(&evs[1], Ev::Tool(_, c) if c == "ls"));
        assert!(matches!(evs.last(), Some(Ev::Text(t)) if t == "Erledigt."));
        assert_eq!(strip_ansi("\x1b[32mgrün\x1b[0m"), "grün");
    }
}
