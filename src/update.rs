//! Updates: neueste Version aus dem GitHub-Repository (Cargo.toml auf `main`) und Installer aus
//! `dist/Foldex-Installer.exe`. Foldex prüft einmal am Tag im Hintergrund; „Aktualisieren“ lädt den Installer,
//! startet ihn still (`/silent /launch`) und beendet sich – der Installer startet danach die neue Version.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const VERSION_URL: &str = "https://raw.githubusercontent.com/xJetouss/Foldex/main/Cargo.toml";
pub const INSTALLER_URL: &str = "https://github.com/xJetouss/Foldex/raw/main/dist/Foldex-Installer.exe";
pub const PAGE_URL: &str = "https://github.com/xJetouss/Foldex";
/// Android: neueste APK (aus dem GitHub-Release) – Android installiert sie nach dem Download selbst
pub const APK_URL: &str = "https://github.com/xJetouss/Foldex/releases/latest/download/Foldex-Android.apk";

/// laufende Version
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// „0.9.10“ → (0, 9, 10)
fn parse(v: &str) -> (u32, u32, u32) {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|x| x.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn newer(a: &str, than: &str) -> bool {
    parse(a) > parse(than)
}

/// neueste Version auf GitHub (aus Cargo.toml)
pub fn latest() -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).user_agent("Foldex").build();
    let text = agent.get(VERSION_URL).call().map_err(|e| format!("Keine Verbindung zu GitHub: {e}"))?.into_string().map_err(|e| e.to_string())?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix("version").map(|r| r.trim_start_matches([' ', '=']).trim().trim_matches('"').to_string()))
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "Versionsangabe nicht gefunden".into())
}

/// Zustand der Update-Prüfung (für Statuszeile und Einstellungen)
#[derive(Clone, Default)]
pub enum State {
    #[default]
    Unknown,
    Checking,
    UpToDate,
    Available(String),
    Downloading,
    Failed(String),
}

#[derive(Clone, Default)]
pub struct Updater {
    pub state: Arc<Mutex<State>>,
    /// geladene Bytes / Gesamtgröße (Download)
    pub got: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
}

impl Updater {
    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// im Hintergrund nach einer neuen Version sehen
    pub fn check(&self, ctx: &eframe::egui::Context) {
        if matches!(self.state(), State::Checking | State::Downloading) {
            return;
        }
        *self.state.lock().unwrap() = State::Checking;
        let (st, ctx) = (self.state.clone(), ctx.clone());
        std::thread::spawn(move || {
            let s = match latest() {
                Ok(v) if newer(&v, current()) => State::Available(v),
                Ok(_) => State::UpToDate,
                Err(e) => State::Failed(e),
            };
            *st.lock().unwrap() = s;
            let _ = std::fs::write(stamp(), chrono::Local::now().timestamp().to_string());
            ctx.request_repaint();
        });
    }

    /// beim Start: höchstens einmal am Tag prüfen
    pub fn check_daily(&self, ctx: &eframe::egui::Context) {
        let last: i64 = std::fs::read_to_string(stamp()).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        if chrono::Local::now().timestamp() - last > 20 * 3600 {
            self.check(ctx);
        }
    }

    /// Installer laden und still ausführen; danach muss Foldex sich beenden (Rückgabe Ok)
    pub fn install(&self, ctx: &eframe::egui::Context, done: impl FnOnce(Result<(), String>) + Send + 'static) {
        // Android: die neue APK im Browser laden – Android fragt dann, ob die App aktualisiert werden soll
        #[cfg(target_os = "android")]
        {
            let _ = (ctx, done);
            if let Err(e) = crate::android::open_url(APK_URL) {
                *self.state.lock().unwrap() = State::Failed(format!("Download lässt sich nicht öffnen: {e}"));
            }
        }
        #[cfg(not(target_os = "android"))]
        self.install_desktop(ctx, done);
    }

    #[cfg(not(target_os = "android"))]
    fn install_desktop(&self, ctx: &eframe::egui::Context, done: impl FnOnce(Result<(), String>) + Send + 'static) {
        *self.state.lock().unwrap() = State::Downloading;
        let (st, got, total, ctx) = (self.state.clone(), self.got.clone(), self.total.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = download_and_run(&got, &total, &ctx);
            if let Err(e) = &r {
                *st.lock().unwrap() = State::Failed(e.clone());
            }
            ctx.request_repaint();
            done(r);
        });
    }
}

fn stamp() -> std::path::PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("update-check.txt")
}

fn download_and_run(got: &AtomicU64, total: &AtomicU64, ctx: &eframe::egui::Context) -> Result<(), String> {
    use std::io::{Read, Write};
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).user_agent("Foldex").build();
    let resp = agent.get(INSTALLER_URL).call().map_err(|e| format!("Download fehlgeschlagen: {e}"))?;
    let len: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    total.store(len, Relaxed);
    got.store(0, Relaxed);
    let dest = std::env::temp_dir().join(format!("Foldex-Installer-{}.exe", std::process::id()));
    let mut f = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
    let mut rd = resp.into_reader().take(512 << 20);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = rd.read(&mut buf).map_err(|e| format!("Download unterbrochen: {e}"))?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got.fetch_add(n as u64, Relaxed);
        ctx.request_repaint();
    }
    drop(f);
    let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    if size < 1 << 20 || (len > 0 && size != len) {
        return Err("Der geladene Installer ist unvollständig".into());
    }
    #[cfg(windows)]
    {
        // still installieren und danach die neue Version starten
        std::process::Command::new(&dest).args(["/silent", "/launch"]).spawn().map_err(|e| format!("Installer startet nicht: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = std::fs::remove_file(&dest);
        Err(format!("Unter Linux bitte die neue Version von {PAGE_URL} laden und mit build.sh bauen"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("0.9.1", "0.9.0"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(newer("0.10.0", "0.9.0"));
        assert!(!newer("0.9.0", "0.9.0"));
        assert!(!newer("0.8.5", "0.9.0"));
        assert!(newer("v1.2", "1.1.9"));
    }
}
