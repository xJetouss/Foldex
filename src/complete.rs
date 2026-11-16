//! Eigene Vorschläge im Terminal: Befehle (PATH + Shell-Befehle), Verlauf, Unterbefehle (git, cargo, npm …),
//! Git-Branches, npm-Skripte sowie Ordner und Dateien relativ zum aktuellen Ordner der Shell.
//! Reine Logik ohne Oberfläche – das Terminal liest die Eingabezeile vom Bildschirm und fragt hier nach.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shell {
    Pwsh,
    Cmd,
    Unix,
}

pub fn shell_of(program: &str) -> Shell {
    let p = program.to_lowercase();
    if p.contains("pwsh") || p.contains("powershell") {
        Shell::Pwsh
    } else if p.ends_with("cmd.exe") || p == "cmd" {
        Shell::Cmd
    } else {
        Shell::Unix
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    History,
    Command,
    Sub,
    Branch,
    Script,
    Dir,
    File,
    Ai,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::History => "Verlauf",
            Kind::Command => "Befehl",
            Kind::Sub => "Unterbefehl",
            Kind::Branch => "Branch",
            Kind::Script => "Skript",
            Kind::Dir => "Ordner",
            Kind::File => "Datei",
            Kind::Ai => "✦ KI",
        }
    }
}

/// Ein Vorschlag: `replace` Zeichen vor dem Cursor werden durch `insert` ersetzt.
#[derive(Clone, Debug, PartialEq)]
pub struct Cand {
    pub label: String,
    pub kind: Kind,
    pub replace: usize,
    pub insert: String,
}

impl Cand {
    /// Was beim Übernehmen gesendet werden muss: (Rückschritte, Text). Passt der Anfang schon, nur der Rest.
    pub fn edit(&self, typed_token: &str) -> (usize, String) {
        let tok: String = typed_token.chars().rev().take(self.replace).collect::<Vec<_>>().into_iter().rev().collect();
        if let Some(rest) = self.insert.strip_prefix(&tok) {
            (0, rest.to_string())
        } else {
            (self.replace, self.insert.clone())
        }
    }

    /// Grauer Resttext hinter dem Cursor (nur wenn der getippte Teil exakt passt).
    pub fn ghost(&self, input: &str) -> Option<String> {
        let (back, text) = self.edit(input);
        (back == 0 && !text.is_empty()).then_some(text)
    }
}

// ───────────────────────────── Prompt ─────────────────────────────

/// Zeile bis zum Cursor in Prompt und Eingabe zerlegen: (Ordner aus dem Prompt, Eingabe).
pub fn split_prompt(line: &str, shell: Shell) -> Option<(Option<PathBuf>, String)> {
    match shell {
        Shell::Pwsh => {
            let i = line.find("PS ")?;
            let rest = &line[i + 3..];
            let end = rest.find("> ")?;
            Some((Some(PathBuf::from(&rest[..end])), rest[end + 2..].to_string()))
        }
        Shell::Cmd => {
            let b = line.as_bytes();
            if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
                return None;
            }
            let end = line.find('>')?;
            Some((Some(PathBuf::from(&line[..end])), line[end + 1..].to_string()))
        }
        Shell::Unix => {
            // „user@host:~/ordner$ “, „# “, „❯ “, „% “
            let idx = ["$ ", "# ", "❯ ", "% ", "> "].iter().filter_map(|m| line.find(m).map(|i| i + m.len())).min()?;
            Some((None, line[idx..].to_string()))
        }
    }
}

// ───────────────────────────── Befehle und Verlauf ─────────────────────────────

const PWSH: &[&str] = &[
    "cd", "ls", "dir", "cls", "clear", "echo", "cat", "cp", "mv", "rm", "mkdir", "pwd", "exit", "Get-ChildItem", "Set-Location", "Get-Content", "Set-Content", "Add-Content",
    "Copy-Item", "Move-Item", "Remove-Item", "New-Item", "Rename-Item", "Get-Item", "Get-Location", "Push-Location", "Pop-Location", "Get-Process", "Stop-Process", "Start-Process",
    "Get-Service", "Select-String", "Where-Object", "ForEach-Object", "Sort-Object", "Measure-Object", "Select-Object", "Get-Command", "Get-Help", "Invoke-WebRequest",
    "Invoke-RestMethod", "Test-Path", "Write-Host", "Write-Output", "Out-File", "Get-History", "Clear-Host", "Expand-Archive", "Compress-Archive",
];
const CMD: &[&str] = &["cd", "dir", "cls", "copy", "move", "del", "ren", "mkdir", "rmdir", "type", "echo", "set", "exit", "start", "where", "tree", "pushd", "popd", "xcopy", "robocopy"];
const UNIX: &[&str] = &["cd", "ls", "pwd", "cat", "cp", "mv", "rm", "mkdir", "rmdir", "touch", "echo", "export", "grep", "find", "less", "head", "tail", "chmod", "exit", "clear", "which", "source"];

fn builtins(s: Shell) -> &'static [&'static str] {
    match s {
        Shell::Pwsh => PWSH,
        Shell::Cmd => CMD,
        Shell::Unix => UNIX,
    }
}

const SUBS: &[(&str, &[&str])] = &[
    ("git", &["add", "blame", "branch", "checkout", "cherry-pick", "clean", "clone", "commit", "config", "diff", "fetch", "init", "log", "merge", "pull", "push", "rebase", "remote", "reset", "restore", "revert", "show", "stash", "status", "switch", "tag"]),
    ("cargo", &["add", "bench", "build", "check", "clean", "clippy", "doc", "fmt", "init", "install", "new", "publish", "remove", "run", "test", "tree", "update"]),
    ("npm", &["audit", "ci", "exec", "init", "install", "list", "outdated", "publish", "run", "start", "test", "uninstall", "update"]),
    ("pnpm", &["add", "dlx", "exec", "install", "remove", "run", "start", "test", "update"]),
    ("yarn", &["add", "install", "remove", "run", "start", "test", "upgrade"]),
    ("docker", &["build", "compose", "exec", "images", "logs", "ps", "pull", "push", "rm", "rmi", "run", "stop"]),
    ("winget", &["install", "list", "search", "show", "uninstall", "upgrade"]),
    ("dotnet", &["add", "build", "new", "publish", "restore", "run", "test"]),
    ("rustup", &["component", "default", "show", "target", "toolchain", "update"]),
    ("pip", &["install", "list", "show", "uninstall", "freeze"]),
];

/// cd-artige Befehle: nur Ordner vorschlagen.
fn wants_dirs(cmd: &str) -> bool {
    matches!(cmd.to_lowercase().as_str(), "cd" | "chdir" | "pushd" | "set-location" | "sl" | "push-location" | "rmdir" | "rd" | "mkdir" | "md")
}

/// Programme aus dem PATH (einmal je Programmstart im Hintergrund gesammelt).
fn path_cmds() -> &'static Mutex<Option<Vec<String>>> {
    static C: OnceLock<Mutex<Option<Vec<String>>>> = OnceLock::new();
    C.get_or_init(|| {
        std::thread::spawn(|| {
            let v = scan_path();
            if let Some(m) = C.get() {
                if let Ok(mut g) = m.lock() {
                    *g = Some(v);
                }
            }
        });
        Mutex::new(None)
    })
}

fn scan_path() -> Vec<String> {
    let mut set = HashSet::new();
    let exts = ["exe", "cmd", "bat", "com", "ps1"];
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            let Some(stem) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            let ok = if cfg!(windows) {
                p.extension().is_some_and(|x| exts.contains(&x.to_string_lossy().to_lowercase().as_str()))
            } else {
                p.is_file()
            };
            if ok && !stem.is_empty() {
                set.insert(stem);
            }
        }
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

/// PATH-Suche anstoßen (früh aufrufen, damit die Liste beim ersten Tippen bereitsteht).
pub fn warm_up() {
    let _ = path_cmds();
    let _ = history();
}

fn history_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("term_history.txt")
}

fn history() -> &'static Mutex<Vec<String>> {
    static H: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(fs::read_to_string(history_file()).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect()))
}

/// Ausgeführte Eingabe merken (gemeinsamer Verlauf aller Terminals, neueste zuletzt).
pub fn remember(line: &str) {
    let line = line.trim();
    if line.is_empty() || line.len() > 500 {
        return;
    }
    if let Ok(mut h) = history().lock() {
        h.retain(|x| x != line);
        h.push(line.to_string());
        let n = h.len();
        if n > 1000 {
            h.drain(..n - 1000);
        }
        let _ = crate::themes::write_atomic(&history_file(), h.join("\n").as_bytes());
    }
}

// ───────────────────────────── Vorschläge ─────────────────────────────

/// Letztes Segment (nach |, ;, &&) in Wörter zerlegen; Anführungszeichen halten Leerzeichen zusammen.
/// Rückgabe: (fertige Wörter, angefangenes Wort).
fn tokens(input: &str) -> (Vec<String>, String) {
    let mut seg_start = 0;
    let (mut q, b) = (None::<char>, input.as_bytes());
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        match (q, c) {
            (None, '"' | '\'') => q = Some(c),
            (Some(x), _) if x == c => q = None,
            (None, '|' | ';' | '&') => seg_start = i + 1,
            _ => {}
        }
        i += 1;
    }
    let seg = &input[seg_start..];
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut q = None::<char>;
    for c in seg.chars() {
        match (q, c) {
            (None, '"' | '\'') => {
                q = Some(c);
                cur.push(c);
            }
            (Some(x), _) if x == c => {
                q = None;
                cur.push(c);
            }
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    (words, cur)
}

/// Beginnt `s` mit `prefix`, ohne Groß-/Kleinschreibung (auch Umlaute)?
fn starts_ci(s: &str, prefix: &str) -> bool {
    let mut a = s.chars().flat_map(char::to_lowercase);
    prefix.chars().flat_map(char::to_lowercase).all(|c| a.next() == Some(c))
}

/// Die letzten Einträge des Verlaufs (neueste zuletzt).
pub fn recent(n: usize) -> Vec<String> {
    history().lock().map(|h| h.iter().rev().take(n).rev().cloned().collect()).unwrap_or_default()
}

/// Sieht das Wort wie ein Pfad aus (.\x, ..\, C:\, ~/, /x)?
fn looks_like_path(t: &str) -> bool {
    let b = t.as_bytes();
    t.starts_with('.') || t.starts_with('~') || t.starts_with('\\') || t.starts_with('/') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// Vorschläge für die Eingabe (Cursor am Ende). Höchstens 8, bester zuerst.
pub fn suggest(input: &str, cwd: &Path, shell: Shell) -> Vec<Cand> {
    let cmds = path_cmds().lock().ok().and_then(|g| g.clone()).unwrap_or_default();
    let hist = history().lock().map(|h| h.clone()).unwrap_or_default();
    suggest_with(input, cwd, shell, &cmds, &hist)
}

pub fn suggest_with(input: &str, cwd: &Path, shell: Shell, cmds: &[String], hist: &[String]) -> Vec<Cand> {
    let mut out: Vec<Cand> = Vec::new();
    if input.trim().is_empty() {
        return out;
    }
    let n_in = input.chars().count();
    let (words, tok) = tokens(input);
    let cd = words.first().is_some_and(|w| wants_dirs(w));
    // 1. ganze Zeile aus dem Verlauf (bei cd nur, wenn das Ziel von hier aus existiert)
    for h in hist.iter().rev() {
        let ok_here = !cd || h.split_once(char::is_whitespace).is_some_and(|(_, t)| {
            let t = t.trim().trim_matches(['"', '\'']);
            let p = Path::new(t);
            if p.is_absolute() { p.is_dir() } else { cwd.join(p).is_dir() }
        });
        if ok_here && h.len() > input.len() && starts_ci(h, input) && !out.iter().any(|c| c.label == *h) {
            out.push(Cand { label: h.clone(), kind: Kind::History, replace: n_in, insert: h.clone() });
            if out.len() >= 3 {
                break;
            }
        }
    }
    let n_tok = tok.chars().count();
    let mut add = |out: &mut Vec<Cand>, label: String, kind: Kind, insert: String| {
        if insert != tok && !out.iter().any(|c| c.insert == insert && c.replace == n_tok) {
            out.push(Cand { label, kind, replace: n_tok, insert });
        }
    };
    match words.len() {
        // 2. Befehl (oder ein Skript/Programm per Pfad, z. B. .\build.ps1)
        0 if looks_like_path(&tok) => {}
        0 if !tok.is_empty() => {
            let mut seen = HashSet::new();
            for c in builtins(shell).iter().map(|s| s.to_string()).chain(cmds.iter().cloned()) {
                if starts_ci(&c, &tok) && seen.insert(c.to_lowercase()) {
                    add(&mut out, c.clone(), Kind::Command, c);
                }
                if out.len() >= 8 {
                    break;
                }
            }
        }
        0 => {}
        // 3. Unterbefehle
        1 if SUBS.iter().any(|(c, _)| words[0].eq_ignore_ascii_case(c)) => {
            let subs = SUBS.iter().find(|(c, _)| words[0].eq_ignore_ascii_case(c)).map(|s| s.1).unwrap_or(&[]);
            for s in subs.iter().filter(|s| starts_ci(s, &tok)) {
                add(&mut out, s.to_string(), Kind::Sub, s.to_string());
            }
        }
        _ => {
            let cmd = words[0].to_lowercase();
            let sub = words.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
            // 4. Git-Branches
            if cmd == "git" && matches!(sub.as_str(), "checkout" | "switch" | "merge" | "rebase" | "branch" | "diff" | "log") && words.len() >= 2 {
                for b in git_branches(cwd).into_iter().filter(|b| starts_ci(b, &tok)) {
                    add(&mut out, b.clone(), Kind::Branch, b);
                }
            }
            // 5. npm-Skripte
            if matches!(cmd.as_str(), "npm" | "pnpm" | "yarn") && sub == "run" && words.len() == 2 {
                for s in npm_scripts(cwd).into_iter().filter(|s| starts_ci(s, &tok)) {
                    add(&mut out, s.clone(), Kind::Script, s);
                }
            }
        }
    }
    // 6. Pfade (ab dem zweiten Wort, bei cd auch ohne angefangenes Wort; vorne nur bei Pfad-Schreibweise)
    let path_pos = if words.is_empty() { looks_like_path(&tok) } else { !tok.is_empty() || cd };
    if path_pos {
        // Laufwerk ohne Schrägstrich („D:“) → „D:\“ vorschlagen
        let b = tok.as_bytes();
        if shell != Shell::Unix && b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            let d = format!("{}\\", tok.to_ascii_uppercase());
            if Path::new(&d).is_dir() {
                add(&mut out, d.clone(), Kind::Dir, d);
            }
        }
        // „..“ bei cd ohne Eingabe bzw. mit „.“
        if cd && (tok.is_empty() || tok == ".") {
            let sep = if shell == Shell::Unix { "../" } else { "..\\" };
            add(&mut out, sep.to_string(), Kind::Dir, sep.to_string());
        }
        for (label, kind, insert) in paths(&tok, cwd, shell, cd) {
            add(&mut out, label, kind, insert);
            if out.len() >= 8 {
                break;
            }
        }
    }
    out.truncate(8);
    out
}

fn paths(tok: &str, cwd: &Path, shell: Shell, dirs_only: bool) -> Vec<(String, Kind, String)> {
    let quoted = tok.starts_with('"') || tok.starts_with('\'');
    // angefangenes oder schon geschlossenes Zitat: „"Projekte Alt\"“
    let raw = tok.trim_start_matches(['"', '\'']).trim_end_matches(['"', '\'']);
    let cut = raw.rfind(['\\', '/']).map_or(0, |i| i + 1);
    let (dir_part, prefix) = raw.split_at(cut);
    let base = if dir_part.is_empty() {
        cwd.to_path_buf()
    } else if let Some(rest) = dir_part.strip_prefix('~') {
        dirs::home_dir().unwrap_or_default().join(rest.trim_start_matches(['\\', '/']))
    } else {
        let p = PathBuf::from(dir_part);
        if p.is_absolute() || dir_part.starts_with('\\') || dir_part.starts_with('/') {
            p
        } else {
            cwd.join(p)
        }
    };
    let sep = if shell == Shell::Unix { '/' } else { '\\' };
    let mut found: Vec<(bool, String)> = fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .take(5000)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !starts_ci(&name, prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            let dir = e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir();
            (dir || !dirs_only).then_some((dir, name))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    found
        .into_iter()
        .take(12)
        .map(|(dir, name)| {
            let mut full = format!("{dir_part}{name}");
            if dir {
                full.push(sep);
            }
            let needs_quotes = full.contains(' ') || full.contains('(') || full.contains('&');
            let insert = if quoted {
                let q = &tok[..1];
                format!("{q}{full}{q}")
            } else if needs_quotes && shell == Shell::Unix {
                full.replace(' ', "\\ ")
            } else if needs_quotes {
                format!("\"{full}\"")
            } else {
                full
            };
            let label = if dir { format!("{name}{sep}") } else { name };
            (label, if dir { Kind::Dir } else { Kind::File }, insert)
        })
        .collect()
}

fn git_branches(cwd: &Path) -> Vec<String> {
    let Some(root) = cwd.ancestors().find(|a| a.join(".git").exists()) else { return Vec::new() };
    let git = root.join(".git");
    let mut v = Vec::new();
    let heads = git.join("refs").join("heads");
    let mut stack = vec![heads.clone()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(r) = p.strip_prefix(&heads) {
                v.push(r.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    for l in fs::read_to_string(git.join("packed-refs")).unwrap_or_default().lines() {
        if let Some(b) = l.split_once(" refs/heads/").map(|x| x.1) {
            v.push(b.to_string());
        }
    }
    v.sort();
    v.dedup();
    v
}

fn npm_scripts(cwd: &Path) -> Vec<String> {
    let Some(j) = fs::read(cwd.join("package.json")).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { return Vec::new() };
    j.get("scripts").and_then(|s| s.as_object()).map(|o| o.keys().cloned().collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts() {
        assert_eq!(split_prompt(r"PS C:\Users\me> git st", Shell::Pwsh), Some((Some(PathBuf::from(r"C:\Users\me")), "git st".into())));
        assert_eq!(split_prompt(r"C:\Temp>dir", Shell::Cmd), Some((Some(PathBuf::from(r"C:\Temp")), "dir".into())));
        assert_eq!(split_prompt("me@box:~/x$ ls -", Shell::Unix), Some((None, "ls -".into())));
        assert_eq!(split_prompt("Kein Prompt", Shell::Pwsh), None);
    }

    #[test]
    fn suggestions() {
        let dir = std::env::temp_dir().join(format!("foldex_compl_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Projekte Alt")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("readme.md"), "x").unwrap();
        let cmds = vec!["cargo".to_string(), "code".to_string()];
        let hist = vec!["cargo build --release".to_string()];
        // Befehl + Verlauf
        let v = suggest_with("car", &dir, Shell::Pwsh, &cmds, &hist);
        assert_eq!(v[0].kind, Kind::History);
        assert!(v.iter().any(|c| c.kind == Kind::Command && c.insert == "cargo"));
        assert_eq!(v[1].ghost("car").as_deref(), Some("go"));
        // Unterbefehl
        let v = suggest_with("git sta", &dir, Shell::Pwsh, &cmds, &[]);
        assert!(v.iter().any(|c| c.insert == "stash") && v.iter().any(|c| c.insert == "status"));
        // cd: nur Ordner, mit Anführungszeichen bei Leerzeichen; Verlauf nur, wenn das Ziel hier existiert
        let v = suggest_with("cd pro", &dir, Shell::Pwsh, &cmds, &["cd projekte_gibts_nicht".to_string()]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].insert, "\"Projekte Alt\\\"");
        assert_eq!(v[0].edit("cd pro"), (3, "\"Projekte Alt\\\"".to_string()));
        // Dateien
        let v = suggest_with("cat rea", &dir, Shell::Unix, &cmds, &[]);
        assert_eq!(v[0].insert, "readme.md");
        assert_eq!(v[0].kind, Kind::File);
        // weiter in den zitierten Ordner hinein
        fs::create_dir_all(dir.join("Projekte Alt").join("Neu")).unwrap();
        let v = suggest_with("cd \"Projekte Alt\\\"", &dir, Shell::Pwsh, &cmds, &[]);
        assert_eq!(v[0].insert, "\"Projekte Alt\\Neu\\\"");
        // cd ohne Eingabe: .. und Ordner
        let v = suggest_with("cd ", &dir, Shell::Pwsh, &cmds, &[]);
        assert_eq!(v[0].insert, "..\\");
        assert!(starts_ci("Übersicht", "üb"));
        // Skript per Pfad an erster Stelle
        fs::write(dir.join("build.ps1"), "x").unwrap();
        let v = suggest_with(".\\bu", &dir, Shell::Pwsh, &cmds, &[]);
        assert_eq!(v[0].insert, ".\\build.ps1");
        // Pipe: neues Segment
        let v = suggest_with("ls | co", &dir, Shell::Pwsh, &cmds, &[]);
        assert!(v.iter().any(|c| c.insert == "code"));
        let _ = fs::remove_dir_all(&dir);
    }
}
