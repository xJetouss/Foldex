//! Plugins: eigene Befehle im Rechtsklick-Menü und in der Befehlspalette – ohne Foldex neu zu bauen.
//!
//! Jedes Plugin ist ein Ordner unter `…/foldex/plugins/<id>/` mit einer `plugin.json` (Aufbau siehe
//! `docs/plugins.md`). Ein Befehl startet ein Programm oder Skript mit Platzhaltern für die markierten
//! Dateien und den aktuellen Ordner; die Ausgabe kann in der Statusleiste oder in einem Fenster erscheinen.
//!
//! Plugins laufen mit den Rechten des Benutzers wie jedes andere Programm – nur Plugins aus vertrauenswürdiger
//! Quelle installieren.

use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

/// Version der Plugin-Schnittstelle (Feld `"api"` in plugin.json; fehlt = 1)
pub const API_VERSION: u32 = 1;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    name: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    homepage: String,
    #[serde(default = "one")]
    api: u32,
    #[serde(default)]
    commands: Vec<CommandDef>,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct CommandDef {
    id: String,
    title: String,
    #[serde(default)]
    description: String,
    /// wo der Befehl erscheint: "file", "folder", "background" (leere Stelle im Ordner)
    #[serde(default = "default_when")]
    when: Vec<String>,
    /// nur für diese Endungen (ohne Punkt, klein); leer = alle
    #[serde(default)]
    extensions: Vec<String>,
    /// mit mehreren markierten Einträgen erlaubt (sonst nur bei genau einem)
    #[serde(default = "yes")]
    multiple: bool,
    /// bei mehreren Einträgen das Programm einmal je Eintrag starten (statt einmal mit allen)
    #[serde(default)]
    each: bool,
    /// Programm oder Skript (relativ zum Plugin-Ordner oder im PATH)
    run: String,
    #[serde(default)]
    args: Vec<String>,
    /// Arbeitsordner (Standard: der aktuelle Ordner)
    #[serde(default)]
    cwd: String,
    /// "none" = nur starten, "status" = letzte Zeile in der Statusleiste, "window" = Ausgabe im Fenster
    #[serde(default = "default_output")]
    output: String,
    /// Ordner danach neu einlesen
    #[serde(default)]
    refresh: bool,
    /// vorher nachfragen (Text der Frage)
    #[serde(default)]
    confirm: String,
    /// Symbol im Menü: file, folder, image, code, terminal, tools, star, spark, share, refresh
    #[serde(default)]
    icon: String,
}

fn default_when() -> Vec<String> {
    vec!["file".into()]
}
fn yes() -> bool {
    true
}
fn default_output() -> String {
    "status".into()
}

/// Wie die Ausgabe eines Befehls gezeigt wird.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Output {
    None,
    Status,
    Window,
}

#[derive(Clone, Debug)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub description: String,
    pub on_file: bool,
    pub on_folder: bool,
    pub on_background: bool,
    pub extensions: Vec<String>,
    pub multiple: bool,
    pub each: bool,
    pub run: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub output: Output,
    pub refresh: bool,
    pub confirm: String,
    pub icon: String,
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub homepage: String,
    pub dir: PathBuf,
    pub commands: Vec<Command>,
    /// Fehler beim Lesen der plugin.json (Plugin wird angezeigt, aber nicht ausgeführt)
    pub error: Option<String>,
}

impl Plugin {
    /// Schlüssel eines Befehls: „plugin-id/befehl-id“
    pub fn key(&self, c: &Command) -> String {
        format!("{}/{}", self.id, c.id)
    }
}

/// Ordner für Plugins
pub fn dir() -> PathBuf {
    // FOLDEX_PLUGINS_DIR: anderer Plugin-Ordner (z. B. zum Entwickeln oder für eine portable Installation)
    if let Some(d) = std::env::var_os("FOLDEX_PLUGINS_DIR") {
        return PathBuf::from(d);
    }
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("plugins")
}

/// Alle Plugins im Plugin-Ordner (alphabetisch). Kaputte plugin.json erscheinen mit Fehlertext.
pub fn load_all() -> Vec<Plugin> {
    load_from(&dir())
}

pub fn load_from(root: &Path) -> Vec<Plugin> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else { return v };
    for e in rd.flatten() {
        let d = e.path();
        let f = d.join("plugin.json");
        if !f.is_file() {
            continue;
        }
        let folder = e.file_name().to_string_lossy().into_owned();
        v.push(match std::fs::read_to_string(&f).map_err(|e| e.to_string()).and_then(|t| parse(&t, &folder, &d)) {
            Ok(p) => p,
            Err(err) => Plugin {
                id: folder.clone(),
                name: folder,
                version: String::new(),
                author: String::new(),
                description: String::new(),
                homepage: String::new(),
                dir: d,
                commands: Vec::new(),
                error: Some(err),
            },
        });
    }
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

/// plugin.json lesen und prüfen
pub fn parse(text: &str, folder: &str, dir: &Path) -> Result<Plugin, String> {
    let m: Manifest = serde_json::from_str(text).map_err(|e| format!("plugin.json: {e}"))?;
    if m.api > API_VERSION {
        return Err(format!("braucht Plugin-API {} – diese Foldex-Version kann {API_VERSION}; bitte Foldex aktualisieren", m.api));
    }
    if m.name.trim().is_empty() {
        return Err("plugin.json: „name“ fehlt".into());
    }
    let mut commands = Vec::new();
    for c in m.commands {
        if c.id.trim().is_empty() || c.title.trim().is_empty() || c.run.trim().is_empty() {
            return Err(format!("Befehl „{}“: „id“, „title“ und „run“ sind Pflicht", c.title));
        }
        let output = match c.output.as_str() {
            "none" => Output::None,
            "window" => Output::Window,
            "status" | "" => Output::Status,
            o => return Err(format!("Befehl „{}“: unbekannte Ausgabe „{o}“ (none, status, window)", c.id)),
        };
        for w in &c.when {
            if !["file", "folder", "background"].contains(&w.as_str()) {
                return Err(format!("Befehl „{}“: unbekanntes „when“: „{w}“ (file, folder, background)", c.id));
            }
        }
        commands.push(Command {
            on_file: c.when.iter().any(|w| w == "file"),
            on_folder: c.when.iter().any(|w| w == "folder"),
            on_background: c.when.iter().any(|w| w == "background"),
            extensions: c.extensions.iter().map(|e| e.trim_start_matches('.').to_lowercase()).collect(),
            id: c.id,
            title: c.title,
            description: c.description,
            multiple: c.multiple,
            each: c.each,
            run: c.run,
            args: c.args,
            cwd: c.cwd,
            output,
            refresh: c.refresh,
            confirm: c.confirm,
            icon: c.icon,
        });
    }
    let id = if m.id.trim().is_empty() { folder.to_string() } else { m.id.trim().to_string() };
    Ok(Plugin { id, name: m.name, version: m.version, author: m.author, description: m.description, homepage: m.homepage, dir: dir.to_path_buf(), commands, error: None })
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

impl Command {
    /// Passt der Befehl zur Auswahl? `sel` leer = Rechtsklick auf eine leere Stelle im Ordner.
    pub fn applies(&self, sel: &[PathBuf]) -> bool {
        if sel.is_empty() {
            return self.on_background;
        }
        if sel.len() > 1 && !self.multiple {
            return false;
        }
        sel.iter().all(|p| {
            if p.is_dir() {
                self.on_folder
            } else {
                self.on_file && (self.extensions.is_empty() || self.extensions.contains(&ext_of(p)))
            }
        })
    }
}

/// Platzhalter in einem Argument ersetzen. `{files}` als ganzes Argument wird zu mehreren Argumenten.
fn expand(arg: &str, files: &[PathBuf], cwd: &Path, plugin: &Path) -> Vec<String> {
    if arg == "{files}" {
        return files.iter().map(|p| p.display().to_string()).collect();
    }
    let first = files.first();
    let s = |o: Option<&std::ffi::OsStr>| o.map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
    let joined = files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" ");
    vec![arg
        .replace("{files}", &joined)
        .replace("{file}", &first.map(|p| p.display().to_string()).unwrap_or_default())
        .replace("{name}", &s(first.and_then(|p| p.file_name())))
        .replace("{stem}", &s(first.and_then(|p| p.file_stem())))
        .replace("{ext}", &s(first.and_then(|p| p.extension())))
        .replace("{parent}", &first.and_then(|p| p.parent()).map(|p| p.display().to_string()).unwrap_or_else(|| cwd.display().to_string()))
        .replace("{dir}", &cwd.display().to_string())
        .replace("{plugin}", &plugin.display().to_string())
        .replace("{count}", &files.len().to_string())]
}

/// Programm auflösen: im Plugin-Ordner, sonst im PATH. Skripte über ihren Interpreter starten.
fn program(run: &str, plugin: &Path) -> (PathBuf, Vec<String>) {
    let local = plugin.join(run);
    let exe = if local.is_file() { local } else { PathBuf::from(run) };
    let lower = run.to_lowercase();
    let pre = |p: &str, a: &[&str]| -> (PathBuf, Vec<String>) {
        let mut v: Vec<String> = a.iter().map(|s| s.to_string()).collect();
        v.push(exe.display().to_string());
        (PathBuf::from(p), v)
    };
    if lower.ends_with(".ps1") {
        pre("powershell", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
    } else if lower.ends_with(".bat") || lower.ends_with(".cmd") {
        pre("cmd", &["/C"])
    } else if lower.ends_with(".py") {
        pre(if cfg!(windows) { "python" } else { "python3" }, &[])
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
        pre("node", &[])
    } else if lower.ends_with(".sh") {
        pre(if cfg!(windows) { "bash" } else { "sh" }, &[])
    } else {
        (exe, Vec::new())
    }
}

/// Die Programmaufrufe für eine Auswahl (bei `each` einer je Eintrag).
pub fn invocations(p: &Plugin, c: &Command, sel: &[PathBuf], cwd: &Path) -> Vec<(PathBuf, Vec<String>, PathBuf)> {
    let groups: Vec<Vec<PathBuf>> = if c.each && sel.len() > 1 { sel.iter().map(|x| vec![x.clone()]).collect() } else { vec![sel.to_vec()] };
    groups
        .into_iter()
        .map(|files| {
            let (exe, mut args) = program(&c.run, &p.dir);
            for a in &c.args {
                args.extend(expand(a, &files, cwd, &p.dir));
            }
            let wd = if c.cwd.is_empty() { cwd.to_path_buf() } else { PathBuf::from(expand(&c.cwd, &files, cwd, &p.dir).concat()) };
            (exe, args, wd)
        })
        .collect()
}

/// Ergebnis eines ausgeführten Befehls
pub struct Done {
    pub title: String,
    pub ok: bool,
    /// gesammelte Ausgabe (stdout und stderr)
    pub text: String,
    pub output: Output,
    pub refresh: bool,
    pub secs: f32,
}

/// Befehl im Hintergrund ausführen. Ausgabe wird nur bei `status`/`window` gesammelt (höchstens 10 Minuten).
pub fn run(p: &Plugin, c: &Command, sel: &[PathBuf], cwd: &Path, repaint: impl Fn() + Send + 'static) -> Receiver<Done> {
    let (tx, rx) = mpsc::channel();
    let calls = invocations(p, c, sel, cwd);
    let (title, output, refresh) = (c.title.clone(), c.output, c.refresh);
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut text = String::new();
        let mut ok = true;
        for (exe, args, wd) in calls {
            let mut cmd = std::process::Command::new(&exe);
            cmd.args(&args).current_dir(&wd);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                // eigene Konsolenfenster nur, wenn die Ausgabe nicht gebraucht wird
                if output != Output::None {
                    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
                }
            }
            if output == Output::None {
                if let Err(e) = cmd.spawn() {
                    ok = false;
                    text.push_str(&format!("{} lässt sich nicht starten: {e}\n", exe.display()));
                }
                continue;
            }
            cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            match cmd.spawn() {
                Err(e) => {
                    ok = false;
                    text.push_str(&format!("{} lässt sich nicht starten: {e}\n", exe.display()));
                }
                Ok(mut child) => {
                    // Ausgabe parallel lesen, damit volle Puffer das Programm nicht blockieren
                    let (mut so, mut se) = (child.stdout.take(), child.stderr.take());
                    let ro = std::thread::spawn(move || read_all(so.as_mut()));
                    let re = std::thread::spawn(move || read_all(se.as_mut()));
                    let deadline = Instant::now() + Duration::from_secs(600);
                    let status = loop {
                        match child.try_wait() {
                            Ok(Some(s)) => break Some(s),
                            Ok(None) if Instant::now() > deadline => {
                                let _ = child.kill();
                                break None;
                            }
                            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                            Err(_) => break None,
                        }
                    };
                    text.push_str(&ro.join().unwrap_or_default());
                    text.push_str(&re.join().unwrap_or_default());
                    match status {
                        Some(s) if s.success() => {}
                        Some(s) => {
                            ok = false;
                            text.push_str(&format!("\n(Exit-Code {})\n", s.code().unwrap_or(-1)));
                        }
                        None => {
                            ok = false;
                            text.push_str("\n(nach 10 Minuten abgebrochen)\n");
                        }
                    }
                }
            }
        }
        let _ = tx.send(Done { title, ok, text: text.trim_end().to_string(), output, refresh, secs: t0.elapsed().as_secs_f32() });
        repaint();
    });
    rx
}

fn read_all(r: Option<&mut impl std::io::Read>) -> String {
    use std::io::Read;
    let mut b = Vec::new();
    if let Some(r) = r {
        let _ = r.take(4 << 20).read_to_end(&mut b);
    }
    String::from_utf8_lossy(&b).into_owned()
}

/// Beispiel-Plugin anlegen (falls noch keins da ist) und dessen Ordner zurückgeben.
pub fn create_example() -> std::io::Result<PathBuf> {
    let d = dir().join("beispiel");
    std::fs::create_dir_all(&d)?;
    let f = d.join("plugin.json");
    if !f.exists() {
        std::fs::write(&f, EXAMPLE)?;
        std::fs::write(d.join("kopieren.ps1"), EXAMPLE_SCRIPT)?;
    }
    Ok(d)
}

/// Inhalt des Beispiel-Plugins (auch in docs/plugins.md beschrieben)
pub const EXAMPLE: &str = r#"{
  "name": "Beispiel",
  "version": "1.0",
  "author": "Du",
  "description": "Zeigt, wie Plugin-Befehle funktionieren. Bearbeite diese Datei und klicke in Foldex auf „Neu laden“.",
  "api": 1,
  "commands": [
    {
      "id": "pfad-kopieren",
      "title": "Pfade als Liste kopieren",
      "description": "Schreibt die markierten Pfade untereinander in die Zwischenablage",
      "when": ["file", "folder"],
      "run": "kopieren.ps1",
      "args": ["{files}"],
      "output": "status",
      "icon": "file"
    },
    {
      "id": "ordner-info",
      "title": "Ordner-Info",
      "description": "Anzahl und Größe der Dateien im aktuellen Ordner",
      "when": ["background"],
      "run": "powershell",
      "args": ["-NoProfile", "-Command", "Get-ChildItem -File -Recurse | Measure-Object Length -Sum | Format-List Count, Sum"],
      "output": "window",
      "icon": "tools"
    }
  ]
}
"#;

/// Skript zum ersten Beispiel-Befehl: bekommt die Pfade als Argumente
pub const EXAMPLE_SCRIPT: &str = "$args | Set-Clipboard
\"$($args.Count) Pfad(e) kopiert\"
";

#[cfg(test)]
mod tests {
    use super::*;

    fn plug(json: &str) -> Plugin {
        parse(json, "test", Path::new("/plug")).unwrap()
    }

    #[test]
    fn manifest_and_matching() {
        let p = plug(r#"{"name":"T","commands":[
            {"id":"a","title":"Bilder","run":"x","extensions":[".PNG","jpg"],"multiple":false},
            {"id":"b","title":"Hintergrund","run":"x","when":["background"],"output":"window"}]}"#);
        assert_eq!(p.id, "test");
        let a = &p.commands[0];
        assert_eq!(a.extensions, ["png", "jpg"]);
        assert!(a.applies(&[PathBuf::from("bild.png")]));
        assert!(!a.applies(&[PathBuf::from("text.txt")]));
        assert!(!a.applies(&[PathBuf::from("a.png"), PathBuf::from("b.png")]), "multiple: false");
        assert!(!a.applies(&[]));
        assert!(p.commands[1].applies(&[]));
        assert_eq!(p.commands[1].output, Output::Window);
        assert!(parse(r#"{"name":"T","commands":[{"id":"a","title":"x","run":"y","output":"laut"}]}"#, "t", Path::new(".")).is_err());
        assert!(parse(r#"{"name":"T","api":99}"#, "t", Path::new(".")).is_err());
        // Beispiel-Plugin ist gültig
        assert_eq!(plug(EXAMPLE).commands.len(), 2);
    }

    /// echter Aufruf: Ausgabe kommt zurück, Fehler werden gemeldet
    #[test]
    #[cfg(windows)]
    fn runs_and_collects_output() {
        let p = plug(r#"{"name":"T","commands":[
            {"id":"ok","title":"Echo","run":"cmd","args":["/C","echo","hallo {name}"],"output":"window"},
            {"id":"bad","title":"Fehler","run":"cmd","args":["/C","exit","3"]}]}"#);
        let sel = [PathBuf::from(r"C:\x\datei.txt")];
        let d = run(&p, &p.commands[0], &sel, &std::env::temp_dir(), || {}).recv().unwrap();
        assert!(d.ok && d.text.contains("hallo datei.txt"), "{}", d.text);
        let d = run(&p, &p.commands[1], &sel, &std::env::temp_dir(), || {}).recv().unwrap();
        assert!(!d.ok && d.text.contains("Exit-Code 3"), "{}", d.text);
    }

    #[test]
    fn placeholders() {
        let p = plug(r#"{"name":"T","commands":[{"id":"a","title":"t","run":"tool.exe","args":["-o","{dir}/{stem}.out","{files}","n={count}"],"each":false}]}"#);
        let sel = [PathBuf::from("/d/a.txt"), PathBuf::from("/d/b.txt")];
        let calls = invocations(&p, &p.commands[0], &sel, Path::new("/d"));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, ["-o", "/d/a.out", "/d/a.txt", "/d/b.txt", "n=2"]);
        let mut c = p.commands[0].clone();
        c.each = true;
        assert_eq!(invocations(&p, &c, &sel, Path::new("/d")).len(), 2);
        // Skripte über ihren Interpreter
        let (exe, pre) = program("tool.ps1", Path::new("/plug"));
        assert_eq!(exe, PathBuf::from("powershell"));
        assert!(pre.last().unwrap().ends_with("tool.ps1"));
    }
}
