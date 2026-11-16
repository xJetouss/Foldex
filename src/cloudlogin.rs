//! Cloud-Konten ohne Programm des Anbieters: Anmelden direkt aus Foldex, der Speicher erscheint als Laufwerk.
//!
//! Dahinter steckt rclone (wird beim ersten Mal geladen, ~25 MB): Für Google Drive, OneDrive, Dropbox, Box und
//! pCloud öffnet sich die Anmeldeseite des Anbieters im Browser; iCloud, MEGA, Proton Drive und Nextcloud melden
//! sich mit Benutzer und Passwort an (iCloud fragt danach den Bestätigungscode). Eingebunden wird mit
//! `rclone mount` – dafür braucht Windows einmalig den kleinen Dateisystem-Treiber WinFsp. Die Laufwerke leben so
//! lange wie Foldex und werden beim nächsten Start wieder eingebunden.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

/// Anmeldeart eines Anbieters
pub enum Auth {
    /// Anmeldeseite des Anbieters im Browser (rclone-Typ, feste Parameter)
    Browser(&'static str, &'static [&'static str]),
    /// Eingabefelder: (Beschriftung, geheim)
    Form(&'static [(&'static str, bool)]),
}

pub fn auth(key: &str) -> Option<Auth> {
    Some(match key {
        "gdrive" => Auth::Browser("drive", &["scope=drive"]),
        "onedrive" => Auth::Browser("onedrive", &[]),
        "dropbox" => Auth::Browser("dropbox", &[]),
        "box" => Auth::Browser("box", &[]),
        "pcloud" => Auth::Browser("pcloud", &[]),
        "icloud" => Auth::Form(&[("Apple-ID (E-Mail)", false), ("Passwort", true)]),
        "mega" => Auth::Form(&[("E-Mail", false), ("Passwort", true)]),
        "proton" => Auth::Form(&[("E-Mail bzw. Benutzername", false), ("Passwort", true), ("2FA-Code (nur falls aktiv)", false)]),
        "nextcloud" => Auth::Form(&[("Server (z. B. https://cloud.example.de)", false), ("Benutzername", false), ("App-Passwort", true)]),
        _ => return None,
    })
}

/// Hinweis unter dem Anmeldeformular
pub fn form_hint(key: &str) -> &'static str {
    match key {
        "icloud" => "Danach fragt Foldex den Bestätigungscode ab – ohne Apple-Gerät kommt er per SMS an deine Telefonnummer. In den iCloud-Einstellungen muss „Auf iCloud-Daten im Web zugreifen“ an und „Erweiterter Datenschutz“ aus sein.",
        "mega" => "Bei aktiver Zwei-Faktor-Anmeldung fragt Foldex nach dem Code.",
        "proton" => "Das Passwort bleibt auf diesem PC (verschleiert in Foldex' Einstellungen).",
        "nextcloud" => "App-Passwort: in Nextcloud unter Einstellungen → Sicherheit → „Neues App-Passwort erstellen“.",
        _ => "",
    }
}

// ───────────────────────── Konten ─────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Account {
    /// Name in der rclone-Konfiguration
    pub remote: String,
    /// Anbieter (cloud::PROVIDERS)
    pub key: String,
    pub name: String,
    /// Laufwerksbuchstabe
    pub letter: char,
    /// Laufwerk getrennt (angemeldet bleiben, nicht einbinden)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub off: bool,
}

impl Account {
    /// Windows: Laufwerk (Z:\); Linux: Ordner ~/Cloud/<Name>
    pub fn path(&self) -> PathBuf {
        #[cfg(windows)]
        return PathBuf::from(format!("{}:\\", self.letter));
        #[cfg(not(windows))]
        {
            let safe: String = self.name.chars().map(|c| if c == '/' || c == '\0' { '-' } else { c }).collect();
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("Cloud").join(safe.trim())
        }
    }
}

/// Name des rclone-Prozesses
const RCLONE_PROC: &str = if cfg!(windows) { "rclone.exe" } else { "rclone" };

/// Linux: ist dort gerade etwas eingebunden?
#[cfg(not(windows))]
fn is_mountpoint(p: &Path) -> bool {
    let want = p.to_string_lossy().replace(' ', "\\040");
    std::fs::read_to_string("/proc/mounts").is_ok_and(|m| m.lines().any(|l| l.split(' ').nth(1) == Some(want.as_str())))
}

/// Linux: FUSE-Einbindung lösen (auch wenn rclone abgestürzt ist und „Transport endpoint …“ meldet)
#[cfg(not(windows))]
fn fuse_unmount(p: &Path) {
    for prog in ["fusermount3", "fusermount", "umount"] {
        let mut c = Command::new(prog);
        if prog == "umount" {
            c.arg("-l");
        } else {
            c.arg("-uz");
        }
        if c.arg(p).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
            return;
        }
    }
}

fn dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("cloud")
}
fn conf() -> PathBuf {
    dir().join("rclone.conf")
}
fn accounts_file() -> PathBuf {
    dir().join("accounts.json")
}

static ACC: Mutex<Option<(Instant, Vec<Account>)>> = Mutex::new(None);

pub fn accounts() -> Vec<Account> {
    let mut g = ACC.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, v)) = g.as_ref() {
        if t.elapsed() < Duration::from_secs(2) {
            return v.clone();
        }
    }
    let v: Vec<Account> = std::fs::read_to_string(accounts_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    *g = Some((Instant::now(), v.clone()));
    v
}

fn save_accounts(v: &[Account]) -> Result<(), String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    std::fs::write(accounts_file(), serde_json::to_string_pretty(v).unwrap_or_default()).map_err(|e| e.to_string())?;
    *ACC.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// eingebundene Konten (von diesem oder einem anderen Foldex-Fenster)
pub fn mounted() -> Vec<Account> {
    let list = accounts();
    if list.is_empty() {
        return list;
    }
    // getrennte Konten zählen nicht, auch wenn rclone noch die letzten Änderungen hochlädt; das Laufwerk selbst
    // wird nicht angefragt (kann hängen) – es zählt, ob sein rclone läuft (auch von einem früheren Foldex)
    list.into_iter().filter(|a| !a.off && (is_mounted(&a.remote) || running(&a.remote))).collect()
}

pub fn is_mounted(remote: &str) -> bool {
    let mut m = mounts().lock().unwrap_or_else(|e| e.into_inner());
    match m.get_mut(remote) {
        Some(c) => matches!(c.try_wait(), Ok(None)),
        None => false,
    }
}

fn mounts() -> &'static Mutex<HashMap<String, Child>> {
    static M: std::sync::OnceLock<Mutex<HashMap<String, Child>>> = std::sync::OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

// ───────────────────────── Programme: rclone und WinFsp ─────────────────────────

pub fn rclone() -> Option<PathBuf> {
    crate::localllm::exe(&crate::localllm::RCLONE)
}

fn rclone_cmd(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    c.arg("--config").arg(conf());
    hide(&mut c);
    c
}

fn hide(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = c;
}

/// WinFsp (Dateisystem-Treiber für `rclone mount`) installiert?
pub fn winfsp() -> bool {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
        base.join("WinFsp").join("bin").join("winfsp-x64.dll").is_file() || PathBuf::from(r"C:\Program Files\WinFsp\bin\winfsp-x64.dll").is_file()
    }
    // Linux: FUSE (fusermount3 bzw. fusermount)
    #[cfg(not(windows))]
    ["/usr/bin/fusermount3", "/bin/fusermount3", "/usr/bin/fusermount", "/bin/fusermount", "/usr/local/bin/fusermount3"].iter().any(|p| Path::new(p).exists())
}

/// Linux: FUSE lässt sich nicht ohne Administratorrechte installieren – Hinweis mit dem passenden Befehl
#[cfg(not(windows))]
fn install_winfsp(_status: &dyn Fn(String)) -> Result<(), String> {
    let cmd = if Path::new("/usr/bin/apt").exists() {
        "sudo apt install fuse3"
    } else if Path::new("/usr/bin/dnf").exists() {
        "sudo dnf install fuse3"
    } else if Path::new("/usr/bin/pacman").exists() {
        "sudo pacman -S fuse3"
    } else if Path::new("/usr/bin/zypper").exists() {
        "sudo zypper install fuse3"
    } else {
        "das Paket „fuse3“ installieren"
    };
    Err(format!("Für Cloud-Laufwerke fehlt FUSE. Bitte im Terminal „{cmd}“ ausführen und dann erneut anmelden."))
}

/// neueste WinFsp-Version von GitHub laden und installieren (Windows fragt nach Administratorrechten)
#[cfg(windows)]
fn install_winfsp(status: &dyn Fn(String)) -> Result<(), String> {
    status("WinFsp suchen …".into());
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).user_agent("Foldex").build();
    let rel: Value = agent.get("https://api.github.com/repos/winfsp/winfsp/releases/latest").call().map_err(|e| format!("WinFsp nicht erreichbar: {e}"))?.into_string().ok().and_then(|t| serde_json::from_str(&t).ok()).ok_or("WinFsp: Antwort unlesbar")?;
    let url = rel["assets"].as_array().into_iter().flatten().filter_map(|a| a["browser_download_url"].as_str()).find(|u| u.ends_with(".msi")).ok_or("WinFsp: kein Installationspaket gefunden")?.to_string();
    status("WinFsp laden (2 MB) …".into());
    let mut buf = Vec::new();
    agent.get(&url).call().map_err(|e| format!("WinFsp laden: {e}"))?.into_reader().take(64 << 20).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let msi = std::env::temp_dir().join("foldex-winfsp.msi");
    std::fs::write(&msi, &buf).map_err(|e| e.to_string())?;
    status("WinFsp installieren – bitte die Windows-Abfrage bestätigen …".into());
    let st = Command::new("msiexec").arg("/i").arg(&msi).args(["/passive", "/norestart"]).status().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&msi);
    if !winfsp() {
        return Err(if st.code() == Some(1602) { "WinFsp-Installation abgebrochen".into() } else { format!("WinFsp wurde nicht installiert (Code {})", st.code().unwrap_or(-1)) });
    }
    Ok(())
}

/// rclone laden (über die Download-Warteschlange der lokalen Modelle) und warten
fn ensure_rclone(status: &dyn Fn(String), cancel: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    if let Some(e) = rclone() {
        return Ok(e);
    }
    crate::localllm::install_runtime(&crate::localllm::RCLONE);
    let dest = crate::localllm::runtime_zip(&crate::localllm::RCLONE);
    let mut seen = false;
    loop {
        if cancel() {
            return Err("Abgebrochen".into());
        }
        if let Some(e) = rclone() {
            return Ok(e);
        }
        match crate::localllm::queued(&dest) {
            Some((f, t)) => {
                seen = true;
                status(if f >= 0.0 { format!("rclone laden … {:.0} %", f * 100.0) } else { t });
            }
            None if seen => {
                std::thread::sleep(Duration::from_millis(300));
                return rclone().ok_or_else(|| "rclone konnte nicht geladen werden".to_string());
            }
            None => {}
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// ───────────────────────── Anmelden ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Busy(String),
    /// Rückfrage (z. B. Bestätigungscode): Text, geheim, Knöpfe (Antwort, Beschriftung), nur Auswahl
    Ask(String, bool, Vec<(String, String)>, bool),
    Done(String),
    Failed(String),
}

pub struct Login {
    pub key: String,
    pub step: Step,
    answer: Option<mpsc::Sender<String>>,
    cancel: bool,
    child: Option<u32>,
}

fn login_state() -> &'static Mutex<Option<Login>> {
    static L: Mutex<Option<Login>> = Mutex::new(None);
    &L
}

/// aktueller Stand der Anmeldung: (Anbieter, Schritt)
pub fn login_step() -> Option<(String, Step)> {
    login_state().lock().ok()?.as_ref().map(|l| (l.key.clone(), l.step.clone()))
}

pub fn login_busy() -> bool {
    matches!(login_step(), Some((_, Step::Busy(_) | Step::Ask(..))))
}

pub fn answer(text: String) {
    if let Ok(mut g) = login_state().lock() {
        if let Some(l) = g.as_mut() {
            if let Some(tx) = l.answer.take() {
                let _ = tx.send(text);
                l.step = Step::Busy("Prüfen …".into());
            }
        }
    }
}

pub fn cancel_login() {
    if let Ok(mut g) = login_state().lock() {
        if let Some(l) = g.as_mut() {
            l.cancel = true;
            l.answer = None;
            if let Some(pid) = l.child.take() {
                kill_pid(pid);
            }
        }
        if matches!(g.as_ref().map(|l| &l.step), Some(Step::Done(_) | Step::Failed(_))) {
            *g = None;
        }
    }
}

pub fn dismiss() {
    if let Ok(mut g) = login_state().lock() {
        if !matches!(g.as_ref().map(|l| &l.step), Some(Step::Busy(_) | Step::Ask(..))) {
            *g = None;
        }
    }
}

fn kill_pid(pid: u32) {
    #[cfg(windows)]
    {
        let mut c = Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null());
        hide(&mut c);
        let _ = c.status();
    }
    // Linux: erst freundlich (rclone löst dabei die Einbindung sauber), nach 3 s erzwingen
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        let t0 = Instant::now();
        while Path::new(&format!("/proc/{pid}")).exists() && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(100));
        }
        if Path::new(&format!("/proc/{pid}")).exists() {
            let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }
}

fn set_step(s: Step) {
    if let Ok(mut g) = login_state().lock() {
        if let Some(l) = g.as_mut() {
            l.step = s;
        }
    }
}

fn cancelled() -> bool {
    login_state().lock().map(|g| g.as_ref().is_none_or(|l| l.cancel)).unwrap_or(true)
}

fn ask(text: String, secret: bool, choices: Vec<(String, String)>, pick_only: bool) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    if let Ok(mut g) = login_state().lock() {
        if let Some(l) = g.as_mut() {
            l.answer = Some(tx);
            l.step = Step::Ask(text, secret, choices, pick_only);
        }
    }
    loop {
        if cancelled() {
            return Err("Abgebrochen".into());
        }
        match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(a) => return Ok(a),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("Abgebrochen".into()),
        }
    }
}

/// Anmeldung im Hintergrund starten; `fields` = Werte der Formularfelder (bei Browser-Anmeldung leer)
pub fn start_login(key: &str, fields: Vec<String>, ctx: &eframe::egui::Context) {
    {
        let mut g = login_state().lock().unwrap_or_else(|e| e.into_inner());
        if matches!(g.as_ref().map(|l| &l.step), Some(Step::Busy(_) | Step::Ask(..))) {
            return;
        }
        *g = Some(Login { key: key.to_string(), step: Step::Busy("Vorbereiten …".into()), answer: None, cancel: false, child: None });
    }
    let key = key.to_string();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let tick = {
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                while login_busy() {
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_millis(250));
                }
            })
        };
        let r = login(&key, &fields);
        set_step(match r {
            Ok(a) => Step::Done(format!("{} ist als Laufwerk {}: eingebunden", a.name, a.letter)),
            Err(e) if cancelled() => Step::Failed(if e.is_empty() { "Abgebrochen".into() } else { e }),
            Err(e) => Step::Failed(e),
        });
        ctx.request_repaint();
        let _ = tick.join();
    });
}

fn login(key: &str, fields: &[String]) -> Result<Account, String> {
    let status = |s: String| set_step(Step::Busy(s));
    let exe = ensure_rclone(&status, &cancelled)?;
    if !winfsp() {
        install_winfsp(&status)?;
    }
    let pr = crate::cloud::provider(key).ok_or("Unbekannter Anbieter")?;
    let auth = auth(key).ok_or("Dieser Anbieter lässt sich nur über sein Programm einrichten")?;
    let existing = accounts();
    let mut remote = key.to_string();
    let mut n = 2;
    while existing.iter().any(|a| a.remote == remote) {
        remote = format!("{key}{n}");
        n += 1;
    }
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let (typ, params): (&str, Vec<String>) = match auth {
        Auth::Browser(typ, fixed) => {
            status(format!("Browser geöffnet – bei {} anmelden und Zugriff erlauben …", pr.name));
            let token = authorize(&exe, typ)?;
            let mut p: Vec<String> = fixed.iter().map(|s| s.to_string()).collect();
            p.push(format!("token={token}"));
            (typ, p)
        }
        Auth::Form(_) => {
            let f = |i: usize| fields.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
            if f(0).is_empty() || fields.get(1).is_none_or(|s| s.is_empty()) {
                return Err("Bitte alle Felder ausfüllen".into());
            }
            status("Anmelden …".into());
            match key {
                "icloud" => ("iclouddrive", vec![format!("apple_id={}", f(0)), format!("password={}", obscure(&exe, &fields[1])?)]),
                "mega" => ("mega", vec![format!("user={}", f(0)), format!("pass={}", obscure(&exe, &fields[1])?)]),
                "proton" => {
                    let mut p = vec![format!("username={}", f(0)), format!("password={}", obscure(&exe, &fields[1])?)];
                    if !f(2).is_empty() {
                        p.push(format!("2fa={}", f(2)));
                    }
                    ("protondrive", p)
                }
                "nextcloud" => {
                    let (server, user) = (f(0), f(1));
                    if fields.get(2).is_none_or(|s| s.is_empty()) {
                        return Err("Bitte alle Felder ausfüllen".into());
                    }
                    let mut s = server.trim_end_matches('/').to_string();
                    if !s.starts_with("http") {
                        s = format!("https://{s}");
                    }
                    let url = if s.contains("/remote.php/") { s } else { format!("{s}/remote.php/dav/files/{user}") };
                    ("webdav", vec![format!("url={url}"), "vendor=nextcloud".into(), format!("user={user}"), format!("pass={}", obscure(&exe, &fields[2])?)])
                }
                _ => return Err("Unbekannter Anbieter".into()),
            }
        }
    };
    // Rest eines abgebrochenen Versuchs unter diesem Namen entfernen
    delete_remote(&exe, &remote);
    configure(&exe, &remote, typ, &params)?;
    // Verbindung prüfen, bevor ein Laufwerk entsteht
    status("Verbindung prüfen …".into());
    let mut c = rclone_cmd(&exe);
    c.args(["lsjson", "--max-depth", "1", "--dirs-only"]).arg(format!("{remote}:")).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let out = c.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        delete_remote(&exe, &remote);
        return Err(format!("Anmeldung fehlgeschlagen: {}", last_line(&String::from_utf8_lossy(&out.stderr))));
    }
    let count = existing.iter().filter(|a| a.key == key).count();
    let name = if count == 0 { pr.name.to_string() } else { format!("{} ({})", pr.name, count + 1) };
    let letter = free_letter(&existing).ok_or("Kein Laufwerksbuchstabe mehr frei")?;
    let acc = Account { remote, key: key.to_string(), name, letter, off: false };
    let mut all = existing;
    all.push(acc.clone());
    save_accounts(&all)?;
    status(format!("Als Laufwerk {}: einbinden …", acc.letter));
    mount(&acc)?;
    crate::cloud::refresh();
    Ok(acc)
}

/// Browser-Anmeldung (`rclone authorize`): gibt das Token (JSON) zurück
fn authorize(exe: &Path, typ: &str) -> Result<String, String> {
    // rclone öffnet die Anmeldeseite selbst im Browser und wartet auf die Rückmeldung (localhost:53682)
    let mut c = rclone_cmd(exe);
    c.args(["authorize", typ]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = c.spawn().map_err(|e| format!("rclone: {e}"))?;
    let pid = child.id();
    if let Ok(mut g) = login_state().lock() {
        if let Some(l) = g.as_mut() {
            l.child = Some(pid);
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if let Ok(mut g) = login_state().lock() {
        if let Some(l) = g.as_mut() {
            l.child = None;
        }
    }
    if cancelled() {
        return Err("Abgebrochen".into());
    }
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    token_from(&text).ok_or_else(|| format!("Anmeldung nicht abgeschlossen: {}", last_line(&String::from_utf8_lossy(&out.stderr))))
}

/// Token aus der Ausgabe von `rclone authorize` („--->“ … „<---End paste“)
fn token_from(text: &str) -> Option<String> {
    let a = text.find("--->")? + 4;
    let b = text[a..].find("<---")? + a;
    let t = text[a..b].trim();
    (t.starts_with('{') && serde_json::from_str::<Value>(t).is_ok()).then(|| t.to_string())
}

/// Passwort verschleiern, wie es in der rclone-Konfiguration steht (über stdin – nie in der Befehlszeile)
fn obscure(exe: &Path, secret: &str) -> Result<String, String> {
    use std::io::Write;
    let mut c = rclone_cmd(exe);
    c.args(["obscure", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut ch = c.spawn().map_err(|e| e.to_string())?;
    ch.stdin.take().ok_or("rclone")?.write_all(secret.as_bytes()).map_err(|e| e.to_string())?;
    let out = ch.wait_with_output().map_err(|e| e.to_string())?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return Err("Passwort konnte nicht verarbeitet werden".into());
    }
    Ok(s)
}

/// Konto in der rclone-Konfiguration anlegen; Rückfragen von rclone (z. B. iCloud-Code, OneDrive-Auswahl)
/// werden mit dem Vorschlag beantwortet bzw. dem Benutzer gestellt
fn configure(exe: &Path, remote: &str, typ: &str, params: &[String]) -> Result<(), String> {
    let mut c = rclone_cmd(exe);
    c.args(["config", "create", remote, typ]).args(params).args(["--non-interactive", "--no-obscure"]);
    let mut reply = run_json(c)?;
    for _ in 0..24 {
        if cancelled() {
            delete_remote(exe, remote);
            return Err("Abgebrochen".into());
        }
        let err = reply["Error"].as_str().unwrap_or_default().to_string();
        let state = reply["State"].as_str().unwrap_or_default().to_string();
        if state.is_empty() {
            if !err.is_empty() {
                delete_remote(exe, remote);
                return Err(format!("Anmeldung fehlgeschlagen: {err}"));
            }
            return Ok(());
        }
        let opt = &reply["Option"];
        let name = opt["Name"].as_str().unwrap_or_default();
        let secret = opt["IsPassword"].as_bool().unwrap_or(false) || opt["Sensitive"].as_bool().unwrap_or(false);
        let default = match &opt["Default"] {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
        let first_example = opt["Examples"].as_array().and_then(|a| a.first()).and_then(|e| e["Value"].as_str()).map(str::to_string);
        let help = opt["Help"].as_str().unwrap_or("Eingabe").lines().next().unwrap_or_default().to_string();
        let examples: Vec<(String, String)> = opt["Examples"].as_array().into_iter().flatten().filter_map(|e| Some((e["Value"].as_str()?.to_string(), e["Help"].as_str().unwrap_or_default().to_string()))).collect();
        let why = if err.is_empty() { String::new() } else { format!(" ({err})") };
        let sl = state.to_lowercase();
        // iCloud: Code vom Apple-Gerät oder per SMS („sms“), Telefonnummer wählen, SMS-Code
        let question: Option<(String, Vec<(String, String)>, bool)> = if sl.contains("2fa_sms_select") {
            Some((format!("An welche Nummer soll die SMS gehen?{why}"), examples.iter().map(|(v, h)| (v.clone(), if h.is_empty() { v.clone() } else { h.clone() })).collect(), true))
        } else if sl.contains("2fa_sms") {
            let to = help.split_once("sent to ").map(|x| x.1.trim_end_matches('.').to_string()).unwrap_or_default();
            Some((if to.is_empty() { format!("Code aus der SMS eingeben{why}") } else { format!("Code aus der SMS an {to} eingeben{why}") }, Vec::new(), false))
        } else if sl.contains("2fa") || name.contains("2fa") || name.contains("code") {
            let sms = help.to_lowercase().contains("sms");
            let text = if sms { format!("Bestätigungscode vom Apple-Gerät eingeben – kein Apple-Gerät? Code per SMS anfordern{why}") } else { format!("Bestätigungscode eingeben{why}") };
            Some((text, if sms { vec![("sms".to_string(), "Code per SMS senden".to_string())] } else { Vec::new() }, false))
        } else if default.is_empty() && first_example.is_none() && opt["Required"].as_bool().unwrap_or(false) {
            Some((help.clone(), Vec::new(), false))
        } else {
            None
        };
        // feste Antworten: Googles gemeinsame Client-ID von rclone weiter nutzen (sonst verlangt rclone eine
        // eigene Google-Client-ID), ein frisch geholtes Token nicht erneut anfordern
        let fixed = match name {
            "config_shared_client_id" => Some("true"),
            "config_refresh_token" => Some("false"),
            _ => None,
        };
        let result = if let Some(f) = fixed {
            f.to_string()
        } else if let Some((text, choices, pick_only)) = question {
            ask(text, secret, choices, pick_only)?
        } else if !default.is_empty() {
            default
        } else {
            first_example.unwrap_or_default()
        };
        set_step(Step::Busy("Anmelden …".into()));
        let mut c = rclone_cmd(exe);
        c.args(["config", "update", remote, "--continue", "--state", &state, "--result", &result, "--non-interactive", "--no-obscure"]);
        reply = run_json(c)?;
    }
    delete_remote(exe, remote);
    Err("Anmeldung: zu viele Rückfragen".into())
}

fn run_json(mut c: Command) -> Result<Value, String> {
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = c.output().map_err(|e| format!("rclone: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    // die Antwort ist ein JSON-Objekt; davor können Hinweise stehen
    let json = text.find('{').and_then(|i| serde_json::from_str::<Value>(text[i..].trim()).ok());
    match json {
        Some(v) => Ok(v),
        None if out.status.success() => Ok(Value::Null),
        None => Err(format!("Anmeldung fehlgeschlagen: {}", last_line(&String::from_utf8_lossy(&out.stderr)))),
    }
}

fn last_line(s: &str) -> String {
    let l = s.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("unbekannter Fehler");
    // rclone-Zeitstempel und „ERROR :“ abschneiden
    let l = l.split_once("ERROR : ").map_or(l, |x| x.1);
    let l = l.split_once("CRITICAL: ").map_or(l, |x| x.1);
    l.chars().take(240).collect()
}

fn delete_remote(exe: &Path, remote: &str) {
    let mut c = rclone_cmd(exe);
    c.args(["config", "delete", remote]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let _ = c.status();
}

/// freier Laufwerksbuchstabe von Z abwärts (nicht belegt und nicht für ein anderes Konto vorgemerkt)
fn free_letter(taken: &[Account]) -> Option<char> {
    let used = used_letters();
    ('G'..='Z').rev().find(|l| used & (1 << (*l as u32 - 'A' as u32)) == 0 && !taken.iter().any(|a| a.letter == *l))
}

fn used_letters() -> u32 {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::Storage::FileSystem::GetLogicalDrives()
    }
    #[cfg(not(windows))]
    0
}

// ───────────────────────── Einbinden ─────────────────────────

/// Konto als Laufwerk einbinden (läuft, solange Foldex läuft)
pub fn mount(a: &Account) -> Result<(), String> {
    if is_mounted(&a.remote) {
        return Ok(());
    }
    let exe = rclone().ok_or("rclone fehlt")?;
    if !winfsp() {
        return Err(if cfg!(windows) { "WinFsp fehlt – im Fenster „Cloud-Speicher“ neu anmelden".into() } else { "FUSE fehlt – bitte „fuse3“ installieren".into() });
    }
    let mut letter = a.letter;
    // schon eingebunden (anderes Foldex-Fenster oder früherer Start)? Mit alten Optionen: einmal neu einbinden
    if running(&a.remote) {
        if read_info(&a.remote).is_some_and(|i| i.ver >= MOUNT_VER) {
            return Ok(());
        }
        unmount(&a.remote);
    }
    #[cfg(not(windows))]
    let _ = &mut letter;
    #[cfg(not(windows))]
    {
        let root = a.path();
        if is_mountpoint(&root) {
            fuse_unmount(&root); // Rest eines abgestürzten rclone
        }
        std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    }
    // gerade getrennt? Windows gibt den Buchstaben erst nach einem Moment frei – kurz warten statt umzuziehen
    #[cfg(windows)]
    let bit = 1u32 << (letter as u32 - 'A' as u32);
    #[cfg(windows)]
    {
        let t0 = Instant::now();
        while used_letters() & bit != 0 && t0.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    #[cfg(windows)]
    if used_letters() & bit != 0 {
        // Buchstabe inzwischen belegt (z. B. USB-Stick) → neuen wählen und merken
        let mut all = accounts();
        let others: Vec<Account> = all.iter().filter(|x| x.remote != a.remote).cloned().collect();
        letter = free_letter(&others).ok_or("Kein Laufwerksbuchstabe frei")?;
        if let Some(x) = all.iter_mut().find(|x| x.remote == a.remote) {
            x.letter = letter;
        }
        save_accounts(&all)?;
    }
    let cache = dir().join("cache");
    let log = dir().join(format!("mount-{}.log", a.remote));
    // Fernsteuerung nur für Foldex (localhost, mit Passwort): fragt vor dem Trennen, ob noch hochgeladen wird
    let port = std::net::TcpListener::bind("127.0.0.1:0").ok().and_then(|l| l.local_addr().ok()).map_or(0, |a| a.port());
    let pass = secret();
    let mut c = rclone_cmd(&exe);
    if port != 0 {
        c.args(["--rc", "--rc-addr", &format!("127.0.0.1:{port}"), "--rc-user", "foldex", "--rc-pass", &pass]);
    }
    #[cfg(windows)]
    let target = format!("{letter}:");
    #[cfg(not(windows))]
    let target = a.path().display().to_string();
    c.arg("mount")
        .arg(format!("{}:", a.remote))
        .arg(&target)
        .args(["--vfs-cache-mode", "full", "--vfs-cache-max-age", "72h", "--vfs-write-back", "5s"])
        // Zwischenspeicher begrenzen: nie die Systemplatte volllaufen lassen
        .args(["--vfs-cache-max-size", "8G", "--vfs-cache-min-free-space", "3G"])
        // Anbieter, die Änderungen melden: Ordner lange merken (kaum Abfragen, keine Wartezeiten), Änderungen
        // kommen per Abfrage alle 30 s; die übrigen Ordner nach 1 min neu lesen
        .args(if polls(&a.key) { ["--dir-cache-time", "1000h", "--poll-interval", "10s"] } else { ["--dir-cache-time", "1m", "--poll-interval", "1m"] })
        .arg("--volname")
        .arg(&a.name);
    // iCloud meldet rclone keinen Speicher (Windows sähe 1 PB) → freien Speicher von Apple holen und vorgeben
    if a.key == "icloud" {
        if let Some((free, _)) = icloud_space(&a.remote) {
            c.arg("--vfs-disk-space-total-size").arg(format!("{free}B"));
        }
    }
    c.arg("--cache-dir")
        .arg(&cache)
        .arg("--log-file")
        .arg(&log)
        .args(["--log-level", "NOTICE"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // nicht an Foldex gebunden: das Laufwerk bleibt nach dem Schließen da und lädt Änderungen fertig hoch
    let mut child = c.spawn().map_err(|e| format!("rclone: {e}"))?;
    let info = MountInfo { pid: child.id(), port, pass, ver: MOUNT_VER };
    let _ = std::fs::write(info_file(&a.remote), serde_json::to_string(&info).unwrap_or_default());
    // warten, bis das Laufwerk da ist
    #[cfg(windows)]
    let root = PathBuf::from(format!("{letter}:\\"));
    #[cfg(windows)]
    let ready = || root.is_dir();
    #[cfg(not(windows))]
    let root = a.path();
    #[cfg(not(windows))]
    let ready = || is_mountpoint(&root);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(45) {
        if let Ok(Some(_)) = child.try_wait() {
            let msg = std::fs::read_to_string(&log).map(|s| last_line(&s)).unwrap_or_default();
            return Err(format!("Einbinden fehlgeschlagen: {msg}"));
        }
        if ready() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    mounts().lock().unwrap_or_else(|e| e.into_inner()).insert(a.remote.clone(), child);
    Ok(())
}

/// beim Start: alle Konten wieder einbinden (im Hintergrund)
pub fn mount_all() {
    let list: Vec<Account> = accounts().into_iter().filter(|a| !a.off).collect();
    if list.is_empty() || rclone().is_none() || !winfsp() {
        return;
    }
    // alle gleichzeitig: jedes Konto wartet sonst auf das vorige
    for a in list {
        std::thread::spawn(move || {
            let _ = mount(&a);
            crate::cloud::refresh();
            space(&a.remote);
        });
    }
}

/// Ordnergröße online zählen: (Bytes, Dateien) – None, wenn der Pfad zu keinem Konto gehört
pub fn size_of(p: &Path) -> Option<(u64, u64)> {
    let (a, sub) = locate(p)?;
    let exe = rclone()?;
    let out = rclone_cmd(&exe).args(["size", "--json", "--fast-list", &format!("{}:{sub}", a.remote)]).stdin(Stdio::null()).output().ok()?;
    let j: Value = serde_json::from_slice(&out.stdout).ok()?;
    Some((j.get("bytes")?.as_u64()?, j.get("count")?.as_u64()?))
}

/// Laufwerksbuchstaben aller angemeldeten Konten (auch getrennter) – sofort, ohne Laufwerke anzufassen
pub fn letters() -> Vec<char> {
    let mut v: Vec<char> = accounts().into_iter().map(|a| a.letter).collect();
    // Entwicklung: weitere Laufwerke wie Cloud behandeln (z. B. ein künstlich gebremstes Testlaufwerk)
    if let Ok(x) = std::env::var("FOLDEX_CLOUD_LETTERS") {
        v.extend(x.chars().filter(|c| c.is_ascii_uppercase()));
    }
    v.extend(LEAVING.lock().unwrap_or_else(|e| e.into_inner()).iter().copied());
    v
}

/// Buchstaben abgemeldeter Konten, deren Laufwerk noch fertig hochlädt
static LEAVING: Mutex<Vec<char>> = Mutex::new(Vec::new());

/// frei/gesamt, wie das Laufwerk es Windows meldet (GetDiskFreeSpaceExW direkt – fs4 scheitert an WinFsp-Laufwerken)
pub fn disk_space(root: &Path) -> Option<(u64, u64)> {
    #[cfg(windows)]
    {
        let w: Vec<u16> = root.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        let (mut free, mut total) = (0u64, 0u64);
        let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(w.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
        (ok != 0 && total > 0).then_some((free, total))
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        None
    }
}

/// Wert eines Eintrags aus rclone.conf (z. B. die iCloud-Sitzung)
fn conf_value(remote: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(conf()).ok()?;
    let mut inside = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            inside = l == format!("[{remote}]");
        } else if inside {
            if let Some((k, v)) = l.split_once(" = ") {
                if k.trim() == key {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// gültiger Zugangsschlüssel für die Google-Drive-Schnittstelle. Läuft er bald ab, frischt rclone ihn mit einer
/// kleinen Abfrage auf und schreibt ihn zurück in die Konfiguration.
pub fn drive_token(remote: &str) -> Option<String> {
    let read = || -> Option<(String, Option<chrono::DateTime<chrono::FixedOffset>>)> {
        let t: Value = serde_json::from_str(&conf_value(remote, "token")?).ok()?;
        let exp = t.get("expiry").and_then(|e| e.as_str()).and_then(|e| chrono::DateTime::parse_from_rfc3339(e).ok());
        Some((t.get("access_token")?.as_str()?.to_string(), exp))
    };
    let (tok, exp) = read()?;
    let fresh = exp.is_some_and(|e| e.signed_duration_since(chrono::Local::now()).num_seconds() > 120);
    if fresh {
        return Some(tok);
    }
    let exe = rclone()?;
    let _ = rclone_cmd(&exe).args(["about", &format!("{remote}:")]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    read().map(|(t, _)| t)
}

/// iCloud-Speicher (frei, gesamt) über setup.icloud.com – mit den Sitzungs-Cookies, die rclone gespeichert hat
fn icloud_space(remote: &str) -> Option<(u64, u64)> {
    let cookies = conf_value(remote, "cookies").filter(|c| !c.is_empty())?;
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    let req = |r: ureq::Request| r.set("Cookie", &cookies).set("Origin", "https://www.icloud.com").set("Referer", "https://www.icloud.com/").set("User-Agent", "Mozilla/5.0");
    let v: Value = serde_json::from_str(&req(agent.post("https://setup.icloud.com/setup/ws/1/validate")).send_string("null").ok()?.into_string().ok()?).ok()?;
    let dsid = v.pointer("/dsInfo/dsid").and_then(|d| d.as_str().map(str::to_string).or_else(|| d.as_u64().map(|n| n.to_string())))?;
    let u: Value = serde_json::from_str(&req(agent.get("https://setup.icloud.com/setup/ws/1/storageUsageInfo").query("dsid", &dsid)).call().ok()?.into_string().ok()?).ok()?;
    let total = u.pointer("/storageUsageInfo/totalStorageInBytes")?.as_u64()?;
    let used = u.pointer("/storageUsageInfo/usedStorageInBytes")?.as_u64()?;
    (total > 0).then(|| (total.saturating_sub(used), total))
}

/// freier und gesamter Speicher eines Kontos (`rclone about`), im Hintergrund ermittelt, 5 min gemerkt
#[derive(Default)]
struct SpaceEntry {
    at: Option<Instant>,
    busy: bool,
    val: Option<(u64, u64)>,
}
type E = SpaceEntry;
static SPACE: std::sync::OnceLock<Mutex<HashMap<String, SpaceEntry>>> = std::sync::OnceLock::new();
/// so lange gilt ein ermittelter Wert
const SPACE_TTL: Duration = Duration::from_secs(60);

/// nach Dateiaktionen: Speicher bald neu abfragen (das Hochladen läuft im Hintergrund, daher mit kurzer Verzögerung)
pub fn space_soon() {
    if let Some(m) = SPACE.get() {
        let soon = Instant::now().checked_sub(SPACE_TTL - Duration::from_secs(8));
        for e in m.lock().unwrap_or_else(|e| e.into_inner()).values_mut() {
            if e.at.is_some() {
                e.at = soon;
            }
        }
    }
}

pub fn space(remote: &str) -> Option<(u64, u64)> {
    let m = SPACE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    let e = g.entry(remote.to_string()).or_default();
    if !e.busy && e.at.is_none_or(|t| t.elapsed() > SPACE_TTL) {
        e.busy = true;
        let r = remote.to_string();
        let icloud = accounts().iter().any(|a| a.remote == remote && a.key == "icloud");
        std::thread::spawn(move || {
            // iCloud kennt in rclone kein „about“ → Apples Web-Schnittstelle mit der gespeicherten Sitzung
            let v = if icloud { icloud_space(&r) } else { rclone().and_then(|exe| {
                let out = rclone_cmd(&exe).args(["about", &format!("{r}:"), "--json"]).stdin(Stdio::null()).output().ok()?;
                let j: Value = serde_json::from_slice(&out.stdout).ok()?;
                let total = j.get("total").and_then(|v| v.as_u64())?;
                let free = match j.get("free").and_then(|v| v.as_u64()) {
                    Some(f) => f,
                    None => total.saturating_sub(j.get("used").and_then(|v| v.as_u64())?),
                };
                Some((free, total))
            }) };
            let mut g = SPACE.get().unwrap().lock().unwrap_or_else(|e| e.into_inner());
            let e = g.entry(r).or_default();
            e.busy = false;
            e.at = Some(Instant::now());
            if v.is_some() {
                e.val = v;
            }
        });
    }
    e.val
}

/// Deinstallieren: alle Cloud-Laufwerke sofort trennen (ohne auf Uploads zu warten)
pub fn stop_all() {
    for a in accounts() {
        if let Some(mut c) = mounts().lock().unwrap_or_else(|e| e.into_inner()).remove(&a.remote) {
            let _ = c.kill();
            let _ = c.wait();
        } else if let Some(i) = read_info(&a.remote) {
            if crate::cloud::process_name(i.pid).as_deref() == Some(RCLONE_PROC) {
                kill_pid(i.pid);
            }
        }
        #[cfg(not(windows))]
        if is_mountpoint(&a.path()) {
            fuse_unmount(&a.path());
        }
    }
}

/// Prozess, Port und Passwort eines eingebundenen Kontos (bleibt über Foldex-Neustarts gültig)
#[derive(Serialize, Deserialize)]
struct MountInfo {
    pid: u32,
    port: u16,
    pass: String,
    /// Stand der Einbinde-Optionen: ältere Einbindungen werden beim Start einmal neu eingebunden
    #[serde(default)]
    ver: u32,
}

/// aktueller Stand der Einbinde-Optionen (bei Änderungen erhöhen)
const MOUNT_VER: u32 = 2;

fn info_file(remote: &str) -> PathBuf {
    dir().join(format!("mount-{remote}.json"))
}

fn read_info(remote: &str) -> Option<MountInfo> {
    serde_json::from_str(&std::fs::read_to_string(info_file(remote)).ok()?).ok()
}

/// zufälliges Passwort (Betriebssystem-Zufall über die Hash-Schlüssel der Standardbibliothek)
fn secret() -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..2)
        .map(|i| {
            let mut h = std::hash::RandomState::new().build_hasher();
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) + i);
            format!("{:016x}", h.finish())
        })
        .collect()
}

/// Dateien, die noch hochgeladen werden bzw. warten (None = unbekannt)
fn uploads_pending(i: &MountInfo) -> Option<u64> {
    if i.port == 0 {
        return None;
    }
    let auth = format!("Basic {}", base64_encode(format!("foldex:{}", i.pass).as_bytes()));
    let r = ureq::post(&format!("http://127.0.0.1:{}/vfs/stats", i.port)).set("Authorization", &auth).timeout(Duration::from_secs(5)).send_string("{}").ok()?;
    let j: Value = serde_json::from_str(&r.into_string().ok()?).ok()?;
    let n = |k: &str| j.pointer(&format!("/diskCache/{k}")).and_then(|v| v.as_u64()).unwrap_or(0);
    Some(n("uploadsInProgress") + n("uploadsQueued"))
}

/// läuft das rclone dieses Kontos noch?
fn running(remote: &str) -> bool {
    read_info(remote).is_some_and(|i| crate::cloud::process_name(i.pid).as_deref() == Some(RCLONE_PROC))
}

/// meldet der Anbieter Änderungen von selbst (rclone fragt sie regelmäßig ab)?
pub fn polls(key: &str) -> bool {
    matches!(key, "gdrive" | "onedrive" | "dropbox" | "box")
}

/// Konto und Pfad im Konto (mit „/“) zu einem Pfad auf einem Cloud-Laufwerk
pub fn locate(p: &Path) -> Option<(Account, String)> {
    #[cfg(not(windows))]
    {
        let a = accounts().into_iter().find(|a| p.starts_with(a.path()))?;
        let sub = p.strip_prefix(a.path()).ok()?.to_string_lossy().trim_matches('/').to_string();
        return Some((a, sub));
    }
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        let mut ch = s.chars();
        let (l, colon) = (ch.next()?.to_ascii_uppercase(), ch.next()?);
        if colon != ':' {
            return None;
        }
        let a = accounts().into_iter().find(|a| a.letter == l)?;
        let sub = s.get(2..).unwrap_or("").replace('\\', "/").trim_matches('/').to_string();
        Some((a, sub))
    }
}

/// Befehl an das rclone des eingebundenen Kontos (Fernsteuerung)
pub fn rc(remote: &str, method: &str, body: Value) -> Result<Value, String> {
    let i = read_info(remote).filter(|i| i.port != 0).ok_or("nicht eingebunden")?;
    let auth = format!("Basic {}", base64_encode(format!("foldex:{}", i.pass).as_bytes()));
    let r = ureq::post(&format!("http://127.0.0.1:{}/{method}", i.port))
        .set("Authorization", &auth)
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(60))
        .send_string(&body.to_string());
    match r {
        Ok(r) => serde_json::from_str(&r.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string()),
        Err(ureq::Error::Status(_, r)) => {
            let t = r.into_string().unwrap_or_default();
            Err(serde_json::from_str::<Value>(&t).ok().and_then(|j| j.get("error").and_then(|e| e.as_str()).map(str::to_string)).unwrap_or(t))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Ordner neu vom Anbieter holen (eine kleine Abfrage). Auch bei Google Drive: dessen Änderungsmeldungen
/// kommen oft erst nach bis zu einer Minute – für den gerade angezeigten Ordner zu spät.
pub fn refresh_dir(p: &Path) {
    if let Some((a, sub)) = locate(p) {
        let body = if sub.is_empty() { serde_json::json!({}) } else { serde_json::json!({ "dir": sub }) };
        let _ = rc(&a.remote, "vfs/refresh", body);
    }
}

/// Laufwerk vergisst den Ordner, in dem `p` lag → die nächste Anzeige liest ihn neu
pub fn forget_parent(p: &Path) {
    if let Some((a, sub)) = locate(p) {
        let parent = sub.rsplit_once('/').map_or("", |(d, _)| d).to_string();
        let body = if parent.is_empty() { serde_json::json!({}) } else { serde_json::json!({ "dir": parent }) };
        let _ = rc(&a.remote, "vfs/forget", body);
    }
}

/// Element auf einem Cloud-Laufwerk löschen – beim Anbieter direkt statt Datei für Datei über das Laufwerk.
/// Google Drive: in den Drive-Papierkorb (Rückgabe: Kennung für „Rückgängig“). Sonst löscht rclone; der
/// Anbieter legt es meist in seinen eigenen Papierkorb. Ohne Fernsteuerung: über das Laufwerk.
pub fn cloud_delete(p: &Path) -> Result<Option<(String, String)>, String> {
    let fallback = || if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }.map(|_| None).map_err(|e| e.to_string());
    let Some((a, sub)) = locate(p) else { return fallback() };
    if sub.is_empty() {
        return Err("Das ganze Laufwerk lässt sich nicht löschen".into());
    }
    let fs = format!("{}:", a.remote);
    // noch nicht hochgeladen (nur im Zwischenspeicher) oder keine Fernsteuerung → über das Laufwerk
    let Some(stat) = rc(&a.remote, "operations/stat", serde_json::json!({ "fs": fs, "remote": sub })).ok() else { return fallback() };
    let Some(item) = stat.get("item").filter(|i| !i.is_null()) else { return fallback() };
    let dir = item.get("IsDir").and_then(|d| d.as_bool()).unwrap_or(false);
    let r = if a.key == "gdrive" {
        let id = item.get("ID").and_then(|i| i.as_str()).ok_or("keine Kennung")?.to_string();
        crate::cloudtrash::trash(&a.remote, std::slice::from_ref(&id))?;
        Some((a.remote.clone(), id))
    } else {
        let method = if dir { "operations/purge" } else { "operations/deletefile" };
        rc(&a.remote, method, serde_json::json!({ "fs": fs, "remote": sub }))?;
        None
    };
    forget_parent(p);
    Ok(r)
}

/// Uploads eines Kontos (für die Anzeige): None = nicht eingebunden oder unbekannt
pub fn pending(remote: &str) -> Option<u64> {
    read_info(remote).and_then(|i| uploads_pending(&i))
}

fn base64_encode(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                s.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Laufwerk trennen: erst warten, bis alles hochgeladen ist (höchstens 10 min), dann rclone beenden.
/// Läuft lange – nur im Hintergrund aufrufen.
fn unmount(remote: &str) {
    let info = read_info(remote);
    if let Some(i) = &info {
        // gerade geschlossene Dateien landen erst kurz danach in der Warteschlange → zweimal hintereinander „0“ abwarten
        let t0 = Instant::now();
        let mut zero = 0;
        while t0.elapsed() < Duration::from_secs(600) && zero < 2 {
            std::thread::sleep(Duration::from_millis(1500));
            match uploads_pending(i) {
                Some(0) => zero += 1,
                Some(_) => zero = 0,
                None => break,
            }
        }
    }
    if let Some(mut c) = mounts().lock().unwrap_or_else(|e| e.into_inner()).remove(remote) {
        let _ = c.kill();
        let _ = c.wait();
    } else if let Some(i) = &info {
        // von einem früheren Foldex gestartet: nur beenden, wenn es wirklich noch rclone ist
        if crate::cloud::process_name(i.pid).as_deref() == Some(RCLONE_PROC) {
            kill_pid(i.pid);
        }
    }
    let _ = std::fs::remove_file(info_file(remote));
    // Linux: hängengebliebene Einbindung lösen
    #[cfg(not(windows))]
    if let Some(a) = accounts().into_iter().find(|a| a.remote == remote) {
        if is_mountpoint(&a.path()) {
            fuse_unmount(&a.path());
        }
    }
    // warten, bis Windows den Buchstaben freigegeben hat (sonst zieht ein gleich folgendes Einbinden um)
    #[cfg(windows)]
    if let Some(a) = accounts().into_iter().find(|a| a.remote == remote) {
        let bit = 1u32 << (a.letter as u32 - 'A' as u32);
        let t0 = Instant::now();
        while used_letters() & bit != 0 && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    crate::cloud::refresh();
}

/// Laufwerk trennen (`false`) bzw. wieder einbinden (`true`); die Anmeldung bleibt erhalten
pub fn set_connected(remote: &str, on: bool) {
    let mut all = accounts();
    let Some(a) = all.iter_mut().find(|a| a.remote == remote) else { return };
    a.off = !on;
    let acc = a.clone();
    let _ = save_accounts(&all);
    let r = remote.to_string();
    std::thread::spawn(move || {
        if on {
            let _ = mount(&acc);
            crate::cloud::refresh();
        } else {
            unmount(&r);
        }
    });
}

/// abmelden: Laufwerk trennen, Konto aus der Konfiguration entfernen
/// (sofort aus der Liste, getrennt und gelöscht wird im Hintergrund – nach dem letzten Upload)
pub fn logout(remote: &str) -> Result<(), String> {
    let letter = accounts().into_iter().find(|a| a.remote == remote).map(|a| a.letter);
    if let Some(l) = letter {
        LEAVING.lock().unwrap_or_else(|e| e.into_inner()).push(l);
    }
    let all: Vec<Account> = accounts().into_iter().filter(|a| a.remote != remote).collect();
    save_accounts(&all)?;
    let r = remote.to_string();
    std::thread::spawn(move || {
        unmount(&r);
        if let Some(l) = letter {
            LEAVING.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| *x != l);
        }
        if let Some(exe) = rclone() {
            delete_remote(&exe, &r);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Löschen über den Anbieter (+ Rückgängig bei Google Drive) und Änderungen von außen sehen
    #[test]
    #[ignore]
    fn real_delete_live() {
        let key = std::env::var("FOLDEX_SPACE_KEY").unwrap_or("gdrive".into());
        let a = accounts().into_iter().find(|a| a.key == key).expect("kein Konto");
        mount(&a).expect("mount");
        let a = accounts().into_iter().find(|a| a.key == key).unwrap();
        let exe = rclone().unwrap();
        let base = a.path().join("foldex-live");
        // 1. von außen anlegen (eigener rclone-Aufruf = wie Handy/Web) und warten, bis das Laufwerk es zeigt
        let mut c = rclone_cmd(&exe);
        c.args(["rcat", &format!("{}:foldex-live/von-aussen.txt", a.remote)]).stdin(Stdio::piped());
        let mut ch = c.spawn().unwrap();
        use std::io::Write;
        ch.stdin.take().unwrap().write_all(b"hallo").unwrap();
        ch.wait().unwrap();
        let _ = std::fs::read_dir(a.path()).map(|r| r.count()); // Wurzel einmal gelesen (wie in Foldex)
        let t0 = Instant::now();
        let seen = loop {
            refresh_dir(&a.path());
            refresh_dir(&base);
            if base.join("von-aussen.txt").exists() {
                break Some(t0.elapsed());
            }
            if t0.elapsed() > Duration::from_secs(90) {
                break None;
            }
            std::thread::sleep(Duration::from_secs(2));
        };
        eprintln!("Änderung von außen sichtbar nach {seen:?}");
        // 2. löschen über Foldex
        let f = base.join("von-aussen.txt");
        let t1 = Instant::now();
        let r = cloud_delete(&f);
        eprintln!("gelöscht in {:?}: {r:?}, noch sichtbar: {}", t1.elapsed(), f.exists());
        let gone = !f.exists();
        // 3. Rückgängig (Google Drive)
        if let Ok(Some((remote, id))) = &r {
            crate::cloudtrash::restore(remote, std::slice::from_ref(id)).unwrap();
            forget_parent(&f);
            eprintln!("zurückgeholt, sichtbar: {}", f.exists());
            assert!(f.exists(), "nicht zurückgeholt");
        }
        // aufräumen: Testordner endgültig weg
        let _ = rclone_cmd(&exe).args(["purge", &format!("{}:foldex-live", a.remote), "--drive-use-trash=false"]).output();
        forget_parent(&base);
        assert!(seen.is_some(), "Änderung von außen nicht gesehen");
        assert!(gone && r.is_ok(), "nicht gelöscht");
    }

    /// Laufwerksbuchstaben wiederherstellen: FOLDEX_LETTERS="gdrive=Y,icloud=Z"
    #[test]
    #[ignore]
    fn set_letters() {
        let want = std::env::var("FOLDEX_LETTERS").expect("FOLDEX_LETTERS fehlt");
        for a in accounts() {
            unmount(&a.remote);
        }
        let mut all = accounts();
        for pair in want.split(',') {
            if let Some((r, l)) = pair.split_once('=') {
                if let Some(a) = all.iter_mut().find(|a| a.remote == r) {
                    a.letter = l.chars().next().unwrap();
                }
            }
        }
        save_accounts(&all).unwrap();
        for a in accounts() {
            mount(&a).expect("mount");
            eprintln!("{} → {}:", a.name, a.letter);
        }
        let after = accounts();
        for pair in want.split(',') {
            let (r, l) = pair.split_once('=').unwrap();
            assert_eq!(after.iter().find(|a| a.remote == r).unwrap().letter.to_string(), l);
        }
    }

    /// echtes Konto einbinden und wieder trennen: cargo test real_mount -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_mount() {
        let a = accounts().into_iter().next().expect("kein Konto");
        let bit = 1u32 << (a.letter as u32 - 'A' as u32);
        mount(&a).expect("mount");
        assert!(used_letters() & bit != 0 && a.path().is_dir(), "Laufwerk fehlt");
        let n = std::fs::read_dir(a.path()).map(|r| r.count()).unwrap_or(0);
        eprintln!("{} als {}: eingebunden, {n} Einträge oben", a.name, a.letter);
        assert!(mounted().iter().any(|x| x.remote == a.remote));
        unmount(&a.remote);
        let t0 = Instant::now();
        while used_letters() & bit != 0 && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("getrennt nach {:?}", t0.elapsed());
        assert!(used_letters() & bit == 0, "Laufwerk noch da");
        assert!(!mounted().iter().any(|x| x.remote == a.remote));
    }

    /// Dateien auf dem Cloud-Laufwerk anlegen, ändern, kopieren, umbenennen, löschen (Papierkorb) – wie Foldex es tut
    #[test]
    #[ignore]
    fn real_ops() {
        let key = std::env::var("FOLDEX_SPACE_KEY").unwrap_or("gdrive".into());
        let a = accounts().into_iter().find(|a| a.key == key).expect("kein Konto");
        mount(&a).expect("mount");
        let base = a.path().join("foldex-test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir(&base).expect("Ordner anlegen");
        let f = base.join("a.txt");
        std::fs::write(&f, "eins").expect("schreiben");
        std::fs::write(&f, "eins zwei").expect("ändern");
        std::fs::copy(&f, base.join("b.txt")).expect("kopieren");
        std::fs::rename(base.join("b.txt"), base.join("c.txt")).expect("umbenennen");
        assert_eq!(std::fs::read_to_string(base.join("c.txt")).unwrap(), "eins zwei");
        // wie „Neu“ in Foldex: leere Datei anlegen, gleich umbenennen, dann im Editor speichern (write_atomic)
        let t0 = Instant::now();
        std::fs::File::create_new(base.join("Neue Textdatei.txt")).expect("neu");
        let t_new = t0.elapsed();
        std::fs::rename(base.join("Neue Textdatei.txt"), base.join("notiz.txt")).expect("umbenennen (neu)");
        let t_ren = t0.elapsed();
        crate::themes::write_atomic(&base.join("notiz.txt"), b"bearbeitet").expect("speichern");
        let t_save = t0.elapsed();
        assert_eq!(std::fs::read_to_string(base.join("notiz.txt")).unwrap(), "bearbeitet");
        eprintln!("neu {t_new:?}, umbenannt {t_ren:?}, gespeichert {t_save:?}");
        let r = crate::trashx::delete(&[base.join("c.txt")]);
        eprintln!("Papierkorb: {r:?}, noch da: {}", base.join("c.txt").exists());
        let cap = disk_space(&a.path());
        eprintln!("canonicalize: {:?}", std::fs::canonicalize(&f).is_ok());
        eprintln!("Laufwerk meldet frei/gesamt: {cap:?}");
        let _ = std::fs::remove_dir_all(&base);
        assert!(r.is_ok(), "Löschen fehlgeschlagen");
    }

    /// iCloud mit vorgegebener Größe auf einem freien Buchstaben einbinden: meldet Windows den richtigen freien Speicher?
    #[test]
    #[ignore]
    fn real_icloud_size() {
        let mut a = accounts().into_iter().find(|a| a.key == "icloud").expect("kein iCloud");
        let (free, _) = icloud_space(&a.remote).expect("Apple liefert keinen Speicher");
        a.letter = free_letter(&accounts()).expect("kein Buchstabe");
        a.name = "Foldex Test".into();
        mount(&a).expect("mount");
        let got = disk_space(&a.path());
        unmount(&a.remote);
        eprintln!("Apple frei: {free}, Laufwerk meldet: {got:?}");
        let (f, _) = got.expect("keine Angabe");
        assert!(f.abs_diff(free) < 1 << 30, "freier Speicher falsch");
    }

    /// Datei schreiben und sofort trennen: wartet das Trennen den Upload ab? Danach direkt beim Anbieter nachsehen.
    #[test]
    #[ignore]
    fn real_upload() {
        let key = std::env::var("FOLDEX_SPACE_KEY").unwrap_or("gdrive".into());
        let a = accounts().into_iter().find(|a| a.key == key).expect("kein Konto");
        mount(&a).expect("mount");
        let name = format!("foldex-upload-{}.txt", std::process::id());
        let f = a.path().join(&name);
        std::fs::write(&f, vec![b'x'; 3 << 20]).expect("schreiben");
        std::fs::write(&f, b"bearbeitet").expect("bearbeiten");
        eprintln!("ausstehend direkt danach: {:?}", pending(&a.remote));
        let t0 = Instant::now();
        unmount(&a.remote);
        eprintln!("getrennt nach {:?}", t0.elapsed());
        let exe = rclone().unwrap();
        let out = rclone_cmd(&exe).args(["cat", &format!("{}:{name}", a.remote)]).output().unwrap();
        let online = String::from_utf8_lossy(&out.stdout).to_string();
        let _ = rclone_cmd(&exe).args(["deletefile", &format!("{}:{name}", a.remote)]).output();
        eprintln!("online: {online:?}");
        assert_eq!(online, "bearbeitet", "nicht (richtig) hochgeladen");
    }

    /// Lässt sich ein hängender Zugriff auf ein (künstlich gebremstes) Cloud-Laufwerk mit CancelSynchronousIo abbrechen?
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn real_cancel() {
        use std::os::windows::io::AsRawHandle;
        let a = accounts().into_iter().find(|a| a.key == "gdrive").expect("kein Google");
        let letter = free_letter(&accounts()).unwrap();
        let exe = rclone().unwrap();
        let cache = std::env::temp_dir().join("foldex-cancel-cache");
        let mut c = rclone_cmd(&exe);
        c.args(["mount", &format!("{}:", a.remote), &format!("{letter}:"), "--tpslimit", "0.1", "--tpslimit-burst", "1", "--cache-dir"]).arg(&cache).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = c.spawn().unwrap();
        let root = PathBuf::from(format!("{letter}:\\"));
        let t0 = Instant::now();
        while !root.is_dir() && t0.elapsed() < Duration::from_secs(200) {
            std::thread::sleep(Duration::from_millis(200));
        }
        eprintln!("eingebunden nach {:?}", t0.elapsed());
        // erster Zugriff braucht eine Abfrage → wartet auf die Bremse (20 s)
        let f = root.join("config.json");
        let h = std::thread::spawn(move || {
            let t = Instant::now();
            let r = std::fs::metadata(&f).map(|m| m.len());
            (t.elapsed(), r.map_err(|e| e.to_string()))
        });
        std::thread::sleep(Duration::from_millis(800));
        let th = h.as_raw_handle();
        let ok = unsafe { windows_sys::Win32::System::IO::CancelSynchronousIo(th as _) };
        let (took, r) = h.join().unwrap();
        eprintln!("CancelSynchronousIo={ok}, Zugriff dauerte {took:?}: {r:?}");
        let _ = child.kill();
        let _ = std::fs::remove_dir_all(&cache);
        assert!(took < Duration::from_secs(5), "nicht abgebrochen");
    }

    #[test]
    fn token_parse() {
        let out = "Paste the following into your remote machine --->\n{\"access_token\":\"x\",\"token_type\":\"Bearer\"}\n<---End paste\n";
        assert_eq!(token_from(out).as_deref(), Some("{\"access_token\":\"x\",\"token_type\":\"Bearer\"}"));
        assert!(token_from("nothing").is_none());
        assert_eq!(last_line("2026/11/07 10:00:00 ERROR : x: bad password\n"), "x: bad password");
        assert!(auth("gdrive").is_some() && auth("icloud").is_some() && auth("nextcloud").is_some());
    }
}

#[cfg(test)]
mod space_test {
    #[test]
    #[ignore]
    fn real_space() {
        let r = super::accounts().into_iter().find(|a| a.key == std::env::var("FOLDEX_SPACE_KEY").as_deref().unwrap_or("gdrive")).expect("kein Konto").remote;
        let _ = super::space(&r);
        for _ in 0..100 {
            if let Some((f, t)) = super::space(&r) {
                assert!(t > 0 && f <= t);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        panic!("kein Speicher ermittelt");
    }
}
