//! Git über die installierte `git`-Kommandozeile (keine zusätzliche Bibliothek): Repository erkennen,
//! Dateistatus, Diff, Stage/Unstage, Commit, Pull/Push/Fetch und Branches. Alle Aufrufe laufen in
//! Hintergrund-Threads; Passwortabfragen werden unterdrückt, damit nichts hängen bleibt.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum St {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflict,
    Ignored,
}

impl St {
    pub fn letter(self) -> &'static str {
        match self {
            St::Modified => "M",
            St::Added => "A",
            St::Deleted => "D",
            St::Renamed => "R",
            St::Untracked => "U",
            St::Conflict => "!",
            St::Ignored => "I",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            St::Modified => "geändert",
            St::Added => "neu",
            St::Deleted => "gelöscht",
            St::Renamed => "umbenannt",
            St::Untracked => "nicht verfolgt",
            St::Conflict => "Konflikt",
            St::Ignored => "ignoriert",
        }
    }
}

/// Ein Eintrag aus `git status`: Pfad relativ zum Repository (mit „/“), Index- und Arbeitsbaum-Status.
#[derive(Clone, Debug)]
pub struct Change {
    pub path: String,
    pub staged: Option<St>,
    pub unstaged: Option<St>,
    /// bei Umbenennungen: der alte Pfad
    pub orig: Option<String>,
}

#[derive(Clone, Default, Debug)]
pub struct Status {
    pub root: PathBuf,
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<Change>,
    /// Pfad → wichtigster Status (für die Markierung in der Dateiliste, inkl. übergeordneter Ordner)
    pub marks: HashMap<String, St>,
    pub merging: bool,
}

impl Status {
    pub fn dirty(&self) -> bool {
        self.changes.iter().any(|c| c.unstaged != Some(St::Untracked) || c.staged.is_some())
    }
    pub fn conflicts(&self) -> usize {
        self.changes.iter().filter(|c| c.staged == Some(St::Conflict)).count()
    }
    /// Status eines Elements der Dateiliste (Datei direkt, Ordner = irgendetwas darunter).
    pub fn mark_for(&self, abs: &Path) -> Option<St> {
        let rel = abs.strip_prefix(&self.root).ok()?;
        let key = rel.to_string_lossy().replace('\\', "/");
        self.marks.get(&key).copied()
    }
}

pub fn git_available() -> bool {
    crate::editors::which("git").is_some()
}

/// Repository-Wurzel zu einem Ordner (sucht `.git` in den übergeordneten Ordnern).
pub fn find_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|a| a.join(".git").exists()).map(Path::to_path_buf)
}

fn cmd(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(root);
    c.env("GIT_TERMINAL_PROMPT", "0").env("GCM_INTERACTIVE", "never").env("LC_ALL", "C");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Führt git aus; Fehlermeldungen werden in verständliches Deutsch übersetzt.
pub fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = cmd(root).args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Git ist nicht installiert oder nicht im PATH.".to_string()
        } else {
            format!("git konnte nicht gestartet werden: {e}")
        }
    })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let err = if err.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { err };
        Err(explain(&err))
    }
}

fn explain(err: &str) -> String {
    let l = err.to_lowercase();
    let hint = if l.contains("could not read username") || l.contains("authentication failed") || l.contains("terminal prompts disabled") {
        "Anmeldung beim Server fehlgeschlagen – Zugangsdaten im Git Credential Manager hinterlegen oder einmal im Terminal anmelden."
    } else if l.contains("conflict") {
        "Es gibt Merge-Konflikte. Konfliktdateien bearbeiten, dann stagen und committen."
    } else if l.contains("would be overwritten") || l.contains("local changes") {
        "Ungesicherte lokale Änderungen würden überschrieben – erst committen oder verwerfen."
    } else if l.contains("non-fast-forward") || l.contains("rejected") || l.contains("fetch first") {
        "Der Server hat neuere Commits – erst „Pull“, dann erneut „Push“."
    } else if l.contains("no upstream") || l.contains("has no upstream") {
        "Der Branch hat noch keinen Upstream – „Push“ legt ihn auf origin an."
    } else if l.contains("nothing to commit") {
        "Nichts zu committen."
    } else if l.contains("not a git repository") {
        "Kein Git-Repository."
    } else if l.contains("please tell me who you are") {
        "Git kennt deinen Namen/E-Mail nicht: git config --global user.name / user.email setzen."
    } else {
        ""
    };
    if hint.is_empty() {
        err.to_string()
    } else {
        format!("{hint}\n\n{err}")
    }
}

fn code(c: u8) -> Option<St> {
    match c {
        b'M' | b'T' => Some(St::Modified),
        b'A' => Some(St::Added),
        b'D' => Some(St::Deleted),
        b'R' | b'C' => Some(St::Renamed),
        b'?' => Some(St::Untracked),
        b'!' => Some(St::Ignored),
        b'U' => Some(St::Conflict),
        _ => None,
    }
}

fn rank(s: St) -> u8 {
    match s {
        St::Conflict => 6,
        St::Modified | St::Renamed => 5,
        St::Deleted => 4,
        St::Added => 3,
        St::Untracked => 2,
        St::Ignored => 1,
    }
}

/// `git status --porcelain=v1 -z --branch` auswerten.
pub fn parse_status(root: &Path, out: &str) -> Status {
    let mut s = Status { root: root.to_path_buf(), ..Default::default() };
    let mut parts = out.split('\0').peekable();
    while let Some(e) = parts.next() {
        if e.is_empty() {
            continue;
        }
        if let Some(b) = e.strip_prefix("## ") {
            // „main...origin/main [ahead 1, behind 2]“ bzw. „No commits yet on main“
            let b = b.trim_start_matches("No commits yet on ").trim_start_matches("Initial commit on ");
            let (head, rest) = b.split_once(' ').unwrap_or((b, ""));
            let (br, up) = head.split_once("...").map_or((head, None), |(a, u)| (a, Some(u.to_string())));
            s.branch = br.to_string();
            s.upstream = up;
            for t in rest.trim_matches(['[', ']']).split(", ") {
                if let Some(n) = t.strip_prefix("ahead ") {
                    s.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = t.strip_prefix("behind ") {
                    s.behind = n.parse().unwrap_or(0);
                }
            }
            continue;
        }
        let b = e.as_bytes();
        if b.len() < 4 {
            continue;
        }
        let (x, y) = (b[0], b[1]);
        let path = e[3..].to_string();
        let orig = if x == b'R' || x == b'C' { parts.next().map(str::to_string) } else { None };
        let conflict = x == b'U' || y == b'U' || (x == b'A' && y == b'A') || (x == b'D' && y == b'D');
        let (staged, unstaged) = if conflict {
            (Some(St::Conflict), Some(St::Conflict))
        } else if x == b'?' {
            (None, Some(St::Untracked))
        } else {
            (code(x), code(y))
        };
        s.changes.push(Change { path, staged, unstaged, orig });
    }
    for c in &s.changes {
        let st = [c.staged, c.unstaged].into_iter().flatten().max_by_key(|s| rank(*s)).unwrap_or(St::Modified);
        let trimmed = c.path.trim_end_matches('/');
        let mut key = String::new();
        for (i, comp) in trimmed.split('/').enumerate() {
            if i > 0 {
                key.push('/');
            }
            key.push_str(comp);
            let e = s.marks.entry(key.clone()).or_insert(st);
            if rank(st) > rank(*e) {
                *e = st;
            }
        }
    }
    s.merging = root.join(".git").join("MERGE_HEAD").exists();
    s
}

pub fn status(root: &Path) -> Result<Status, String> {
    let out = run(root, &["status", "--porcelain=v1", "-z", "--branch", "--untracked-files=all"])?;
    Ok(parse_status(root, &out))
}

pub fn branches(root: &Path) -> Vec<(String, bool)> {
    run(root, &["branch", "--format=%(HEAD)%(refname:short)"])
        .map(|o| o.lines().filter(|l| !l.is_empty()).map(|l| (l[1..].to_string(), l.starts_with('*'))).collect())
        .unwrap_or_default()
}

/// Diff einer Datei: gestaged → `--cached`; nicht verfolgt → Inhalt als neu.
pub fn diff(root: &Path, c: &Change, staged: bool) -> String {
    if !staged && c.unstaged == Some(St::Untracked) {
        let p = root.join(&c.path);
        return match std::fs::read(&p) {
            Ok(b) if b.len() > 512 * 1024 => "(Datei größer als 512 KB – keine Vorschau)".into(),
            Ok(b) if b.contains(&0) => "(Binärdatei)".into(),
            Ok(b) => String::from_utf8_lossy(&b).lines().map(|l| format!("+{l}\n")).collect(),
            Err(e) => format!("(nicht lesbar: {e})"),
        };
    }
    let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
    if staged {
        args.push("--cached");
    }
    args.push("--");
    args.push(&c.path);
    match run(root, &args) {
        Ok(s) if s.trim().is_empty() => "(keine Textänderungen)".into(),
        Ok(s) if s.len() > 2_000_000 => s[..2_000_000].to_string() + "\n… (gekürzt)",
        Ok(s) => s,
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_overview_and_plain_message() {
        let c = |p: &str, s: Option<St>, u: Option<St>, o: Option<&str>| Change { path: p.into(), staged: s, unstaged: u, orig: o.map(str::to_string) };
        // nichts gestaged → alles zählt, nicht verfolgt = neu
        let v = vec![c("src/a.rs", None, Some(St::Modified), None), c("src/new.rs", None, Some(St::Untracked), None), c("src/old.rs", None, Some(St::Deleted), None)];
        let (staged, set) = commit_set(&v);
        assert!(!staged);
        assert_eq!(counts(&set), (1, 1, 1, 0));
        let m = plain_message(&set);
        assert!(m.starts_with("src: Update 3 files (1 added, 1 modified, 1 deleted)"), "{m}");
        assert!(m.contains("- Add src/new.rs") && m.contains("- Delete src/old.rs") && m.contains("- Update src/a.rs"), "{m}");
        // gestaged → nur das Gestagte; Umbenennung mit altem Namen
        let v = vec![c("b.txt", Some(St::Renamed), None, Some("a.txt")), c("x.txt", None, Some(St::Modified), None)];
        let (staged, set) = commit_set(&v);
        assert!(staged);
        assert_eq!(plain_message(&set), "Rename a.txt to b.txt");
        assert_eq!(clean_message("```\nFix bug\n```"), "Fix bug");
        assert_eq!(clean_message("\"Add tests\""), "Add tests");
    }

    #[test]
    fn parse() {
        let out = "## main...origin/main [ahead 2, behind 1]\0 M src/a.rs\0A  new.txt\0?? docs/x.md\0UU c.txt\0R  b.txt\0old.txt\0";
        let s = parse_status(Path::new("/r"), out);
        assert_eq!(s.branch, "main");
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (2, 1));
        assert_eq!(s.changes.len(), 5);
        assert_eq!(s.marks.get("src"), Some(&St::Modified));
        assert_eq!(s.marks.get("docs/x.md"), Some(&St::Untracked));
        assert_eq!(s.conflicts(), 1);
        assert_eq!(s.changes[4].path, "b.txt");
        assert_eq!(s.changes[4].staged, Some(St::Renamed));
    }
}

#[cfg(test)]
mod repo_tests {
    use super::*;

    /// Echtes Repository: init, neue Datei, stagen, committen, ändern.
    #[test]
    fn real_repo() {
        if !git_available() {
            return;
        }
        let d = std::env::temp_dir().join(format!("fc-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        run(&d, &["init", "-q", "-b", "main"]).unwrap();
        run(&d, &["config", "user.email", "t@example.com"]).unwrap();
        run(&d, &["config", "user.name", "Test"]).unwrap();
        std::fs::write(d.join("src/a.txt"), "1\n").unwrap();
        let s = status(&d).unwrap();
        assert_eq!(s.branch, "main");
        assert_eq!(s.mark_for(&d.join("src")), Some(St::Untracked));
        run(&d, &["add", "-A", "--", "src/a.txt"]).unwrap();
        run(&d, &["commit", "-q", "-m", "x"]).unwrap();
        std::fs::write(d.join("src/a.txt"), "2\n").unwrap();
        let s = status(&d).unwrap();
        assert_eq!(s.changes.len(), 1);
        assert_eq!(s.changes[0].unstaged, Some(St::Modified));
        assert!(diff(&d, &s.changes[0], false).contains("+2"));
        assert_eq!(find_root(&d.join("src")), Some(d.clone()));
        assert!(run(&d, &["switch", "gibt-es-nicht"]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Ein Eintrag aus `git log`.
#[derive(Clone, Debug)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

/// Die letzten `n` Commits des aktuellen Branches.
pub fn log(root: &Path, n: usize) -> Result<Vec<Commit>, String> {
    let out = run(root, &["log", &format!("-n{n}"), "--pretty=format:%H%x1f%h%x1f%an%x1f%ar%x1f%s"])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\u{1f}').collect();
            (f.len() == 5).then(|| Commit { hash: f[0].into(), short: f[1].into(), author: f[2].into(), date: f[3].into(), subject: f[4].into() })
        })
        .collect())
}

/// Konfigurierte Remotes: (Name, URL).
pub fn remotes(root: &Path) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = run(root, &["remote", "-v"])
        .unwrap_or_default()
        .lines()
        .filter(|l| l.ends_with("(fetch)"))
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), it.next()?.to_string()))
        })
        .collect();
    v.dedup();
    v
}

/// Inhalt eines Commits (Statistik + Änderungen).
pub fn show(root: &Path, hash: &str) -> String {
    match run(root, &["show", "--stat", "--patch", "--no-color", "--format=commit %H%nAutor: %an <%ae>%nDatum: %ad%n%n%B", hash]) {
        Ok(s) if s.len() > 2_000_000 => s[..2_000_000].to_string() + "\n… (gekürzt)",
        Ok(s) => s,
        Err(e) => e,
    }
}

// ───────────────────────────── Commit-Übersicht und -Nachricht ─────────────────────────────

/// Was ein Commit enthielte: bei gestagten Änderungen nur diese, sonst alle (dann wird vorher alles gestagt).
/// Je Datei der wirksame Status (nicht verfolgt zählt als neu), Pfad und ggf. alter Pfad.
pub fn commit_set(changes: &[Change]) -> (bool, Vec<(St, String, Option<String>)>) {
    let staged = changes.iter().any(|c| c.staged.is_some());
    let v = changes
        .iter()
        .filter_map(|c| {
            let st = if staged { c.staged? } else { c.unstaged.or(c.staged)? };
            let st = if st == St::Untracked { St::Added } else { st };
            Some((st, c.path.clone(), c.orig.clone()))
        })
        .collect();
    (staged, v)
}

/// Anzahl je Art: (geändert, neu, gelöscht, umbenannt).
pub fn counts(set: &[(St, String, Option<String>)]) -> (usize, usize, usize, usize) {
    let n = |s: St| set.iter().filter(|c| c.0 == s).count();
    (n(St::Modified) + n(St::Conflict), n(St::Added), n(St::Deleted), n(St::Renamed))
}

/// Commit-Nachricht ohne KI aus der Liste der Änderungen (Englisch, wie bei Git üblich).
pub fn plain_message(set: &[(St, String, Option<String>)]) -> String {
    let line = |(st, p, o): &(St, String, Option<String>)| match st {
        St::Added => format!("Add {p}"),
        St::Deleted => format!("Delete {p}"),
        St::Renamed => format!("Rename {} to {p}", o.as_deref().unwrap_or("?")),
        _ => format!("Update {p}"),
    };
    match set {
        [] => String::new(),
        [one] => line(one),
        _ => {
            let (m, a, d, r) = counts(set);
            let mut parts = Vec::new();
            for (n, w) in [(a, "added"), (m, "modified"), (d, "deleted"), (r, "renamed")] {
                if n > 0 {
                    parts.push(format!("{n} {w}"));
                }
            }
            // gemeinsamer Ordner als Bereich, z. B. „src: …“
            let first_dir = |p: &str| p.split_once('/').map(|(d, _)| d.to_string());
            let scope = first_dir(&set[0].1).filter(|d| set.iter().all(|c| first_dir(&c.1).as_deref() == Some(d)));
            let head = format!("{}Update {} files ({})", scope.map(|s| format!("{s}: ")).unwrap_or_default(), set.len(), parts.join(", "));
            let body: Vec<String> = set.iter().take(40).map(|c| format!("- {}", line(c))).collect();
            let more = if set.len() > 40 { format!("\n- … and {} more", set.len() - 40) } else { String::new() };
            format!("{head}\n\n{}{more}", body.join("\n"))
        }
    }
}

/// Kontext für eine KI-Commit-Nachricht: Liste der Änderungen, Statistik, Diff (gekürzt) und die letzten
/// Commit-Betreffs (für Sprache und Stil). `max` = Höchstlänge des Diffs in Zeichen.
pub fn ai_context(root: &Path, changes: &[Change], max: usize) -> String {
    let (staged, set) = commit_set(changes);
    let mut s = String::from("Changed files:\n");
    for (st, p, o) in &set {
        match o {
            Some(o) => s.push_str(&format!("{} {o} -> {p}\n", st.letter())),
            None => s.push_str(&format!("{} {p}\n", st.letter())),
        }
    }
    let cached: &[&str] = if staged { &["--cached"] } else { &[] };
    let mut stat_args = vec!["diff", "--stat", "--no-color"];
    stat_args.extend_from_slice(cached);
    if let Ok(st) = run(root, &stat_args) {
        s.push_str(&format!("\nStatistics:\n{st}"));
    }
    let mut diff_args = vec!["diff", "--no-color", "--no-ext-diff", "-U2"];
    diff_args.extend_from_slice(cached);
    let mut diff = run(root, &diff_args).unwrap_or_default();
    // neue, noch nicht verfolgte Dateien: Anfang des Inhalts
    if !staged {
        for c in changes.iter().filter(|c| c.unstaged == Some(St::Untracked)).take(20) {
            if let Ok(b) = std::fs::read(root.join(&c.path)) {
                if !b.contains(&0) {
                    let t: String = String::from_utf8_lossy(&b).lines().take(40).map(|l| format!("+{l}\n")).collect();
                    diff.push_str(&format!("\nnew file {}\n{t}", c.path));
                }
            }
        }
    }
    if diff.chars().count() > max {
        diff = diff.chars().take(max).collect::<String>() + "\n… (diff truncated)";
    }
    s.push_str(&format!("\nDiff:\n{diff}"));
    if let Ok(l) = run(root, &["log", "-8", "--format=%s"]) {
        if !l.trim().is_empty() {
            s.push_str(&format!("\nRecent commit subjects (match their language and style):\n{l}"));
        }
    }
    s
}

/// Systemanweisung für die KI-Commit-Nachricht.
pub const COMMIT_SYSTEM: &str = "You write git commit messages. Reply with the commit message only – no explanations, no quotes, no code fences. \
Format: a summary line in the imperative mood (at most 72 characters), then a blank line and a short body with bullet points (\"- \") for the important changes and why, if it helps. \
Small changes need no body. Use the language and style of the recent commit subjects if given (e.g. conventional commits), otherwise English.";

/// KI-Antwort säubern: Codezäune und Anführungszeichen entfernen.
pub fn clean_message(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix("```").map(|r| r.split_once('\n').map_or(r, |(_, b)| b)).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    t.trim_matches('"').trim().to_string()
}

/// Vorlagen für .gitignore beim Erstellen eines Repositorys: (Name, Inhalt).
pub const IGNORE_TEMPLATES: &[(&str, &str)] = &[
    ("Keine", ""),
    ("Allgemein", "# Betriebssystem\nThumbs.db\ndesktop.ini\n.DS_Store\n\n# Editoren\n.vscode/\n.idea/\n\n# Geheimnisse und Logs\n.env\n*.log\n"),
    ("Rust", "/target/\n**/*.rs.bk\n.env\n.idea/\n.vscode/\n"),
    ("Node / JavaScript", "node_modules/\ndist/\nbuild/\n.env\n.env.*\nnpm-debug.log*\n.DS_Store\n"),
    ("Python", "__pycache__/\n*.py[cod]\n.venv/\nvenv/\n.env\ndist/\nbuild/\n*.egg-info/\n"),
];
