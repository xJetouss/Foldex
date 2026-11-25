//! Teilen – eigene Oberfläche (nicht das Windows-Teilen-Menü, auch nicht kurz im Hintergrund). Angezeigt wird nur,
//! was auf diesem PC wirklich geht; Bereiche ohne verfügbare Ziele entfallen. Nichts braucht API-Schlüssel:
//! - **LocalSend**: Foldex sendet selbst – mit Gerätename und Zertifikat der installierten LocalSend-App – über eine
//!   eigene Geräteauswahl. Die LocalSend-App wird dafür nicht geöffnet (siehe localsend.rs).
//! - **Messenger** über ihre offiziellen Wege: Telegram Desktop bekommt die Datei direkt (`-sendpath`, Chat-Auswahl
//!   von Telegram). WhatsApp nimmt Dateien von anderen Programmen nur über das Windows-Teilen an: „Ohne Link“
//!   öffnet es mit der Datei als Anhang (WhatsApp wählen → Chat-Auswahl), „Mit Link“ lädt hoch und öffnet WhatsApps
//!   eigene Chat-Auswahl mit dem Download-Link (`whatsapp://send?text=`).
//!   Signal und Discord haben weder Datei- noch Text-Schnittstelle: Link wird kopiert, die App geöffnet – das Popup
//!   sagt es deutlich.
//! - **E-Mail**: Thunderbird bzw. Outlook (klassisch) mit Anhang; sonst Standard-Mailprogramm per Simple MAPI, beim
//!   neuen Outlook (kann keine fremden Anhänge) eine Mail mit Download-Link (`mailto:`).
//! - **Cloud-Apps**: OneDrive, Google Drive, Dropbox, iCloud Drive, Nextcloud – Kopie in den synchronisierten Ordner.
//! - **Link ohne Anmeldung**: catbox, Gofile, temp.sh, litterbox, uguu, tmpfiles. Lehnt ein Dienst ab (z. B. die
//!   Litterbox-Firewall mit „412 No file!“), versucht Foldex es einmal erneut und weicht dann auf einen anderen Dienst aus.
//! Fortschritt und Ergebnis erscheinen als Popup in der Fenstermitte (Kopieren / Öffnen); nach dem Schließen ist alles
//! weg – es gibt keine angesammelte Liste.

use crate::convert;
use crate::localsend::{self, Device, Discovery};
use crate::themes::Theme;
use crate::ui::{self, Ic};
use eframe::egui::{self, pos2, vec2, Color32, FontId, Margin, Rect, Rounding, Sense, Stroke};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};

const MB: u64 = 1024 * 1024;
const UA: &str = concat!("Foldex/", env!("CARGO_PKG_VERSION"));

// ───────────────────────────── Installierte Apps ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum AppId {
    LocalSend,
    WhatsApp,
    Signal,
    Discord,
    Telegram,
    Mail,
    OneDrive,
    GoogleDrive,
    Dropbox,
    ICloud,
    Nextcloud,
}

/// Wie die Dateien zur App kommen.
#[derive(Clone, Debug)]
enum Via {
    /// LocalSend mit der Identität der installierten App: eigene Geräteauswahl
    LocalSend(localsend::Me),
    /// Telegram Desktop: je Datei „-sendpath“ (öffnet Telegrams Chat-Auswahl mit der Datei)
    Telegram(PathBuf),
    /// Datei hochladen, dann die App per Deep Link mit vorbereitetem Text öffnen (`{text}` wird ersetzt)
    LinkDeep(&'static str),
    /// WhatsApp: eigenes Auswahlfenster – mit Link (Upload + Chat-Auswahl) oder ohne Link (App direkt öffnen)
    WhatsApp(String),
    /// Datei hochladen, Link kopieren und die App starten (Programm + Argumente) – kein direktes Teilen möglich
    LinkCopy(PathBuf, Vec<String>),
    /// Thunderbird: Mail mit Anhängen verfassen
    Thunderbird(PathBuf),
    /// Outlook (klassisch): /a je Anhang
    Outlook(PathBuf),
    /// Standard-Mailprogramm über Simple MAPI
    Mapi,
    /// in einen synchronisierten Ordner kopieren
    Cloud(PathBuf),
}

#[derive(Clone, Debug)]
pub struct App {
    pub id: AppId,
    pub name: &'static str,
    via: Via,
    /// Name für das Programmsymbol (EXE-Pfad oder shell:AppsFolder\…)
    icon: Option<String>,
    color: Color32,
}

fn rgb(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

fn env_dir(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_dir())
}

fn first_file(cands: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    cands.into_iter().find(|p| p.is_file())
}

fn program_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"].iter().filter_map(|k| env_dir(k)).collect();
    v.dedup();
    v
}

/// Store-App installiert? (Paketordner im Benutzerprofil) → App-ID für shell:AppsFolder.
fn store_app(package: &str, app: &str) -> Option<String> {
    let dir = dirs::data_local_dir()?.join("Packages").join(package);
    dir.is_dir().then(|| format!("{package}!{app}"))
}

fn apps_folder(aumid: &str) -> String {
    format!("shell:AppsFolder\\{aumid}")
}

/// Installierte Apps, über die geteilt werden kann (nur gefundene). Blockiert kurz – im Hintergrund aufrufen.
pub fn detect() -> Vec<App> {
    let local = dirs::data_local_dir().unwrap_or_default();
    let roaming = dirs::config_dir().unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    let mut v = Vec::new();
    let mut push = |id, name, via: Option<Via>, icon: Option<String>, color| {
        if let Some(via) = via {
            v.push(App { id, name, via, icon, color: rgb(color) });
        }
    };

    // LocalSend: installiert und schon einmal gestartet (dann gibt es Name und Zertifikat)
    let ls = first_file(program_files().into_iter().map(|d| d.join("LocalSend").join("localsend_app.exe")).chain([local.join("Programs").join("LocalSend").join("localsend_app.exe")]));
    push(AppId::LocalSend, "LocalSend", ls.as_ref().and(localsend::identity()).map(Via::LocalSend), ls.map(|p| p.display().to_string()), 0x2bb38a);

    // WhatsApp: eigenes Auswahlfenster (mit/ohne Link). Mit Desktop-App deren App-ID, sonst leer → WhatsApp Web.
    let wa = store_app("5319275A.WhatsAppDesktop_cv1g1gvanyjgm", "App");
    push(AppId::WhatsApp, "WhatsApp", Some(Via::WhatsApp(wa.clone().unwrap_or_default())), wa.as_ref().map(|a| apps_folder(a)), 0x25d366);

    let tg = first_file([roaming.join("Telegram Desktop").join("Telegram.exe")].into_iter().chain(program_files().into_iter().map(|d| d.join("Telegram Desktop").join("Telegram.exe"))));
    push(AppId::Telegram, "Telegram", tg.clone().map(Via::Telegram), tg.map(|p| p.display().to_string()), 0x29a9eb);

    let sig = first_file([local.join("Programs").join("signal-desktop").join("Signal.exe")]);
    push(AppId::Signal, "Signal", sig.clone().map(|p| Via::LinkCopy(p, vec![])), sig.map(|p| p.display().to_string()), 0x3a76f0);

    let upd = Some(local.join("Discord").join("Update.exe")).filter(|p| p.is_file());
    let dexe = std::fs::read_dir(local.join("Discord")).into_iter().flatten().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("app-")).map(|e| e.path().join("Discord.exe")).filter(|p| p.is_file()).max();
    push(AppId::Discord, "Discord", upd.clone().map(|u| Via::LinkCopy(u, vec!["--processStart".into(), "Discord.exe".into()])), dexe.or(upd).map(|p| p.display().to_string()), 0x5865f2);

    // E-Mail: Thunderbird, Outlook (klassisch) mit Anhang; neues Outlook: Mail mit Link; sonst Simple MAPI
    let tb = first_file(program_files().into_iter().map(|d| d.join("Mozilla Thunderbird").join("thunderbird.exe")));
    let ol = first_file(program_files().into_iter().flat_map(|d| ["Office16", "Office15"].map(|o| d.join("Microsoft Office").join("root").join(o).join("OUTLOOK.EXE"))));
    let (via, icon) = match (tb, ol) {
        (Some(t), _) => (Via::Thunderbird(t.clone()), Some(t.display().to_string())),
        (None, Some(o)) => (Via::Outlook(o.clone()), Some(o.display().to_string())),
        _ => match store_app("Microsoft.OutlookForWindows_8wekyb3d8bbwe", "Microsoft.OutlookforWindows") {
            Some(a) => (Via::LinkDeep("mailto:?subject={subject}&body={text}"), Some(apps_folder(&a))),
            None => (Via::Mapi, None),
        },
    };
    push(AppId::Mail, "E-Mail", Some(via), icon, 0x6b7a8f);

    // Cloud-Apps: synchronisierte Ordner
    let onedrive = env_dir("OneDriveConsumer").or_else(|| env_dir("OneDrive")).or_else(|| env_dir("OneDriveCommercial"));
    let od_exe = first_file([local.join("Microsoft").join("OneDrive").join("OneDrive.exe")].into_iter().chain(program_files().into_iter().map(|d| d.join("Microsoft OneDrive").join("OneDrive.exe"))));
    push(AppId::OneDrive, "OneDrive", onedrive.map(Via::Cloud), od_exe.map(|p| p.display().to_string()), 0x0f6cbd);

    let gdrive = ('C'..='Z').map(|l| PathBuf::from(format!("{l}:\\"))).flat_map(|r| ["Meine Ablage", "My Drive"].map(|n| r.join(n))).find(|p| p.is_dir());
    let gd_exe = program_files().into_iter().flat_map(|d| std::fs::read_dir(d.join("Google").join("Drive File Stream")).into_iter().flatten().flatten().map(|e| e.path().join("GoogleDriveFS.exe"))).find(|p| p.is_file());
    push(AppId::GoogleDrive, "Google Drive", gdrive.map(Via::Cloud), gd_exe.map(|p| p.display().to_string()), 0x1fa463);

    let dropbox = [roaming.join("Dropbox").join("info.json"), local.join("Dropbox").join("info.json")]
        .iter()
        .find_map(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|j| ["personal", "business"].iter().find_map(|k| j[k]["path"].as_str().map(PathBuf::from)))
        .filter(|p| p.is_dir());
    let db_exe = first_file(program_files().into_iter().map(|d| d.join("Dropbox").join("Client").join("Dropbox.exe")));
    push(AppId::Dropbox, "Dropbox", dropbox.map(Via::Cloud), db_exe.map(|p| p.display().to_string()), 0x0061fe);

    let icloud = Some(home.join("iCloudDrive")).filter(|p| p.is_dir());
    push(AppId::ICloud, "iCloud Drive", icloud.map(Via::Cloud), store_app("AppleInc.iCloud_nzyj5cx40ttqa", "iCloud").map(|a| apps_folder(&a)), 0x3693f3);

    let nextcloud = std::fs::read_to_string(roaming.join("Nextcloud").join("nextcloud.cfg"))
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.split_once("\\localPath=").map(|(_, p)| PathBuf::from(p.trim().replace('/', "\\")))))
        .filter(|p| p.is_dir());
    let nc_exe = first_file(program_files().into_iter().map(|d| d.join("Nextcloud").join("nextcloud.exe")));
    push(AppId::Nextcloud, "Nextcloud", nextcloud.map(Via::Cloud), nc_exe.map(|p| p.display().to_string()), 0x0082c9);
    v
}

impl App {
    /// Kurzer Hinweis unter dem Namen (ehrlich: Datei oder nur Link).
    pub fn hint(&self) -> String {
        match &self.via {
            Via::LocalSend(me) => format!("als „{}“ an ein Gerät senden", me.alias),
            Via::Telegram(_) => "Datei direkt – Chat wählen".into(),
            Via::LinkDeep(_) if self.id == AppId::Mail => "Outlook: Mail mit Download-Link".into(),
            Via::LinkDeep(_) => "Download-Link – Chat wählen".into(),
            Via::WhatsApp(_) => "mit oder ohne Link".into(),
            Via::LinkCopy(..) => "nur Link (kein direktes Teilen)".into(),
            Via::Thunderbird(_) => "Thunderbird, als Anhang".into(),
            Via::Outlook(_) => "Outlook, als Anhang".into(),
            Via::Mapi => "Standard-Mailprogramm, als Anhang".into(),
            Via::Cloud(_) => "mit deinem Konto hochladen".into(),
        }
    }
    fn is_cloud_id(id: AppId) -> bool {
        matches!(id, AppId::OneDrive | AppId::GoogleDrive | AppId::Dropbox | AppId::ICloud | AppId::Nextcloud)
    }
    /// Teilt diese App nur einen Link (Datei wird vorher hochgeladen)?
    fn needs_link(&self) -> bool {
        matches!(self.via, Via::LinkDeep(_) | Via::LinkCopy(..))
    }
}

fn quiet(prog: &Path) -> std::process::Command {
    let mut c = std::process::Command::new(prog);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Ergebnis eines Vorgangs für das Popup.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub title: String,
    pub text: String,
    /// Link zum Kopieren/Öffnen
    pub link: Option<String>,
}

fn outcome(title: impl Into<String>, text: impl Into<String>, link: Option<String>) -> Outcome {
    Outcome { title: title.into(), text: text.into(), link }
}

/// Dateien an eine App übergeben. Link-Apps laden vorher hoch (`host`). Läuft im Hintergrund.
fn send_to_app(app: &App, files: &[PathBuf], host: Option<Host>, prog: &Arc<AtomicU64>, cancel: &Arc<AtomicBool>) -> Result<Outcome, String> {
    // Link-Apps: erst hochladen
    let links = if app.needs_link() {
        let h = host.ok_or("Kein Upload-Dienst nimmt diese Datei(en) an – für Messenger vorher verkleinern oder komprimieren")?;
        let mut v = Vec::new();
        for f in files {
            let (l, _) = upload_with_fallback(h, f, prog.clone(), cancel.clone())?;
            v.push(l);
        }
        v
    } else {
        Vec::new()
    };
    let names = names_str(files);
    let text = if links.len() == 1 { format!("{names}: {}", links[0]) } else { links.join("\n") };
    match app.via.clone() {
        Via::LocalSend(_) => Err("LocalSend: bitte ein Gerät auswählen".into()),
        Via::WhatsApp(_) => Err("WhatsApp: bitte im Auswahlfenster wählen".into()),
        Via::Telegram(exe) => {
            for f in files {
                std::process::Command::new(&exe).arg("-sendpath").arg(f).spawn().map_err(|e| format!("Telegram startet nicht: {e}"))?;
                std::thread::sleep(Duration::from_millis(400));
            }
            Ok(outcome("An Telegram übergeben", "Telegram zeigt seine Chat-Auswahl – die Datei ist angehängt.", None))
        }
        Via::LinkDeep(tpl) => {
            let url = tpl.replace("{text}", &enc(&text)).replace("{subject}", &enc(&names));
            open::that_detached(&url).map_err(|e| format!("{} lässt sich nicht öffnen: {e}", app.name))?;
            let note = if app.id == AppId::Mail {
                "Das neue Outlook nimmt keine Anhänge von anderen Programmen an – die Mail enthält den Download-Link.".to_string()
            } else {
                format!("{} nimmt Dateien von anderen Programmen nicht direkt an. Die Datei wurde hochgeladen; {} zeigt seine Chat-Auswahl mit dem Download-Link.", app.name, app.name)
            };
            Ok(outcome(format!("{} geöffnet", app.name), note, links.first().cloned()))
        }
        Via::LinkCopy(exe, args) => {
            std::process::Command::new(&exe).args(&args).spawn().map_err(|e| format!("{} startet nicht: {e}", app.name))?;
            Ok(outcome(
                format!("{} geöffnet – Link kopiert", app.name),
                format!("{} bietet anderen Programmen weder Datei- noch Text-Teilen an. Die Datei wurde hochgeladen und der Link kopiert – im Chat mit Strg+V einfügen.", app.name),
                links.first().cloned(),
            ))
        }
        Via::Thunderbird(exe) => {
            let list: Vec<String> = files.iter().map(|f| format!("file:///{}", f.display().to_string().replace('\\', "/"))).collect();
            std::process::Command::new(&exe).arg("-compose").arg(format!("attachment='{}'", list.join(","))).spawn().map_err(|e| format!("Thunderbird startet nicht: {e}"))?;
            Ok(outcome("Thunderbird geöffnet", "Neue Mail mit Anhang – Empfänger eintragen und senden.", None))
        }
        Via::Outlook(exe) => {
            let mut c = std::process::Command::new(&exe);
            for f in files {
                c.arg("/a").arg(f);
            }
            c.spawn().map_err(|e| format!("Outlook startet nicht: {e}"))?;
            Ok(outcome("Outlook geöffnet", "Neue Mail mit Anhang – Empfänger eintragen und senden.", None))
        }
        Via::Mapi => mapi_send(files).map(|t| outcome("E-Mail", t, None)),
        Via::Cloud(root) => {
            let dir = root.join("Foldex Freigaben");
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let mut last = dir.clone();
            for f in files {
                if cancel.load(Relaxed) {
                    return Err("abgebrochen".into());
                }
                let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let stem = Path::new(&name).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = Path::new(&name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                let dst = crate::ops::unique_with_ext(&dir, &stem, &ext);
                copy_counted(f, &dst, prog, cancel)?;
                last = dst;
            }
            if !cfg!(test) {
                // im Explorer zeigen: dort „Freigeben“ bzw. „Link kopieren“ der Cloud-App
                let _ = quiet(Path::new("explorer.exe")).arg(format!("/select,{}", last.display())).spawn();
            }
            Ok(outcome(format!("In {} kopiert", app.name), format!("Die {}-App lädt mit deinem Konto hoch. Einen Link erstellst du im Explorer per Rechtsklick → Freigeben.", app.name), None))
        }
    }
}

fn copy_counted(src: &Path, dst: &Path, prog: &AtomicU64, cancel: &AtomicBool) -> Result<(), String> {
    use std::io::Write;
    let mut r = BufReader::with_capacity(1 << 20, std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?);
    let mut w = std::fs::File::create(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Relaxed) {
            drop(w);
            let _ = std::fs::remove_file(dst);
            return Err("abgebrochen".into());
        }
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        prog.fetch_add(n as u64, Relaxed);
    }
    Ok(())
}

/// Standard-Mailprogramm über Simple MAPI (mapi32.dll): neue Mail mit Anhängen zum Bearbeiten.
#[cfg(windows)]
fn mapi_send(files: &[PathBuf]) -> Result<String, String> {
    use std::ffi::c_void;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    #[repr(C)]
    struct FileDesc {
        reserved: u32,
        flags: u32,
        position: u32,
        path: *const u16,
        name: *const u16,
        file_type: *const c_void,
    }
    #[repr(C)]
    struct Message {
        reserved: u32,
        subject: *const u16,
        note: *const u16,
        message_type: *const u16,
        date: *const u16,
        conversation: *const u16,
        flags: u32,
        originator: *const c_void,
        recip_count: u32,
        recips: *const c_void,
        file_count: u32,
        files: *const FileDesc,
    }
    type SendMail = unsafe extern "system" fn(usize, usize, *const Message, u32, u32) -> u32;
    let w = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let paths: Vec<Vec<u16>> = files.iter().map(|f| w(&f.display().to_string())).collect();
    let names: Vec<Vec<u16>> = files.iter().map(|f| w(&f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())).collect();
    let descs: Vec<FileDesc> = paths.iter().zip(&names).map(|(p, n)| FileDesc { reserved: 0, flags: 0, position: u32::MAX, path: p.as_ptr(), name: n.as_ptr(), file_type: std::ptr::null() }).collect();
    let subject = w(&if files.len() == 1 { names_str(files) } else { format!("{} Dateien", files.len()) });
    let msg = Message {
        reserved: 0,
        subject: subject.as_ptr(),
        note: std::ptr::null(),
        message_type: std::ptr::null(),
        date: std::ptr::null(),
        conversation: std::ptr::null(),
        flags: 0,
        originator: std::ptr::null(),
        recip_count: 0,
        recips: std::ptr::null(),
        file_count: descs.len() as u32,
        files: descs.as_ptr(),
    };
    unsafe {
        let lib = LoadLibraryW(w("mapi32.dll").as_ptr());
        if lib.is_null() {
            return Err("Kein Mailprogramm mit Mail-Schnittstelle (MAPI) gefunden – Thunderbird oder Outlook installieren".into());
        }
        let Some(f) = GetProcAddress(lib, c"MAPISendMailW".as_ptr().cast()) else {
            return Err("Das Standard-Mailprogramm nimmt keine Anhänge von anderen Programmen an".into());
        };
        let send: SendMail = std::mem::transmute(f);
        // MAPI_LOGON_UI | MAPI_DIALOG: Mailfenster zum Bearbeiten öffnen
        match send(0, 0, &msg, 0x1 | 0x8, 0) {
            0 => Ok("Neue Mail mit Anhang geöffnet".into()),
            1 => Ok("Mail verworfen".into()),
            code => Err(format!("Standard-Mailprogramm meldet Fehler {code} – es nimmt evtl. keine Anhänge von anderen Programmen an; Thunderbird oder klassisches Outlook helfen")),
        }
    }
}

#[cfg(not(windows))]
fn mapi_send(_files: &[PathBuf]) -> Result<String, String> {
    Err("E-Mail mit Anhang ist nur unter Windows eingebaut".into())
}

/// Windows-Teilen (wie im Explorer: Rechtsklick → Teilen) mit den Dateien als Anhang öffnen. Dort wählt man
/// WhatsApp; WhatsApp zeigt dann seine eigene Chat-Auswahl mit der Datei. (Eine Ziel-App vorzuwählen erlaubt
/// Windows nicht.)
#[cfg(windows)]
fn share_sheet(hwnd: isize, files: &[PathBuf], title: &str) -> Result<(), String> {
    use windows::core::{Interface, Ref, HSTRING};
    use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::{IStorageItem, StorageFile};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::IDataTransferManagerInterop;
    // Dateien in einem eigenen Thread holen: kein blockierendes Warten auf WinRT im Fenster-Thread
    let paths = files.to_vec();
    let items = std::thread::spawn(move || -> windows::core::Result<Vec<windows::core::AgileReference<StorageFile>>> {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        paths
            .iter()
            .map(|p| {
                let full = std::path::absolute(p).unwrap_or(p.clone()).to_string_lossy().replace('/', "\\");
                windows::core::AgileReference::new(&StorageFile::GetFileFromPathAsync(&HSTRING::from(full))?.join()?)
            })
            .collect()
    })
    .join()
    .map_err(|_| "Dateien konnten nicht vorbereitet werden".to_string())?
    .map_err(|e| e.message().to_string())?;
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    let res = (|| -> windows::core::Result<()> {
        let interop = windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()?;
        let dtm: DataTransferManager = unsafe { interop.GetForWindow(hwnd)? };
        // nur ein Handler je Fenster: den vom letzten Teilen entfernen
        static TOKEN: std::sync::Mutex<Option<i64>> = std::sync::Mutex::new(None);
        let mut tok = TOKEN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = tok.take() {
            let _ = dtm.RemoveDataRequested(t);
        }
        let title = HSTRING::from(title);
        let handler = TypedEventHandler::new(move |_: Ref<DataTransferManager>, args: Ref<DataRequestedEventArgs>| {
            if let Some(a) = args.as_ref() {
                let data = a.Request()?.Data()?;
                data.Properties()?.SetTitle(&title)?;
                let list: Vec<Option<IStorageItem>> = items.iter().map(|f| f.resolve().and_then(|f| f.cast::<IStorageItem>()).ok()).collect();
                data.SetStorageItemsReadOnly(&windows_collections::IIterable::<IStorageItem>::from(list))?;
            }
            Ok(())
        });
        *tok = Some(dtm.DataRequested(&handler)?);
        unsafe { interop.ShowShareUIForWindow(hwnd) }
    })();
    res.map_err(|e| e.message().to_string())
}

#[cfg(not(windows))]
fn share_sheet(_hwnd: isize, _files: &[PathBuf], _title: &str) -> Result<(), String> {
    Err("nur unter Windows".into())
}

/// Datei direkt an WhatsApp: Windows-Teilen öffnen und darin sofort WhatsApp auswählen (UI-Automation) – es
/// erscheint WhatsApps Chat-Auswahl mit der Datei als Anhang. WhatsApp nimmt Dateien anderer Programme nur so
/// an (laut seinem App-Manifest kein Protokoll, keine Befehlszeile für Dateien). Das Ergebnis (WhatsApp
/// gefunden?) kommt über den Kanal; sonst bleibt das Teilen offen und man wählt WhatsApp selbst.
fn share_to_whatsapp(hwnd: isize, files: &[PathBuf], ctx: &egui::Context) -> Result<Receiver<Pick>, String> {
    let title = names_str(files);
    share_sheet(hwnd, files, &title)?;
    let (tx, rx) = mpsc::channel();
    let c = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(pick_whatsapp());
        c.request_repaint();
    });
    Ok(rx)
}

/// Das Teilen-Fenster von Windows (ein „ApplicationFrameWindow“ des Explorers) finden – nur lesen: Fenster des
/// Explorers werden nie verändert (Verstecken/Verschieben brachte im Test den Explorer zum Absturz).
#[cfg(windows)]
mod sheet {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    fn explorer_pid() -> u32 {
        let mut pid = 0;
        unsafe {
            let tray = FindWindowW(windows_sys::w!("Shell_TrayWnd"), std::ptr::null());
            GetWindowThreadProcessId(tray, &mut pid);
        }
        pid
    }

    /// Wirklich zu sehen? Windows behält geschlossene Teilen-Fenster „sichtbar“, aber verborgen (cloaked).
    fn shown(h: HWND) -> bool {
        let mut cloaked: u32 = 0;
        unsafe {
            IsWindowVisible(h) != 0
                && (windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(h, windows_sys::Win32::Graphics::Dwm::DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as *mut _, 4) != 0
                    || cloaked == 0)
        }
    }

    /// offene Rahmenfenster des Explorers (als Zahl, damit sie zwischen Threads wandern dürfen)
    pub fn frames() -> Vec<isize> {
        struct S {
            pid: u32,
            out: Vec<isize>,
        }
        unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
            let s = &mut *(l as *mut S);
            let mut cls = [0u16; 64];
            let n = GetClassNameW(h, cls.as_mut_ptr(), 64).max(0) as usize;
            if shown(h) && String::from_utf16_lossy(&cls[..n]) == "ApplicationFrameWindow" {
                let mut pid = 0;
                GetWindowThreadProcessId(h, &mut pid);
                if pid == s.pid {
                    s.out.push(h as isize);
                }
            }
            1
        }
        let mut s = S { pid: explorer_pid(), out: Vec::new() };
        if s.pid != 0 {
            unsafe { EnumWindows(Some(each), &mut s as *mut S as LPARAM) };
        }
        s.out
    }
}

/// Im Windows-Teilen den App-Eintrag „WhatsApp“ (Liste „Teilen mit“) auslösen – so schnell wie möglich, damit das
/// Teilen nur kurz zu sehen ist: Sein Fenster wird alle 5 ms gesucht, WhatsApp dann alle 10 ms gezielt in genau
/// diesem Fenster (statt im ganzen Bildschirm). Nur im Teilen-Fenster (Element „SharePicker“); Kontakte
/// („… über WhatsApp“) bleiben unberührt. Klappt es nicht (6 s), bleibt das Teilen offen und man wählt selbst.
/// Ergebnis der automatischen WhatsApp-Auswahl im Windows-Teilen
#[derive(Clone, Copy, PartialEq, Debug)]
enum Pick {
    Done,
    /// Teilen-Fenster war da, WhatsApp nicht darin
    NoWhatsApp,
    /// Windows hat kein Teilen-Fenster gezeigt
    NoSheet,
}

#[cfg(windows)]
fn pick_whatsapp() -> Pick {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::*;
    let deadline = Instant::now() + Duration::from_secs(6);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let mut seen = false;
        (|| -> Option<Pick> {
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let app = uia.CreatePropertyCondition(UIA_NamePropertyId, &VARIANT::from("WhatsApp")).ok()?;
            let picker = uia.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from("SharePicker")).ok()?;
            while Instant::now() < deadline {
                for h in sheet::frames() {
                    let Ok(top) = uia.ElementFromHandle(HWND(h as *mut core::ffi::c_void)) else { continue };
                    if top.FindFirst(TreeScope_Descendants, &picker).is_err() {
                        continue;
                    }
                    seen = true;
                    let Ok(hits) = top.FindAll(TreeScope_Descendants, &app) else { continue };
                    for j in 0..hits.Length().unwrap_or(0) {
                        let Ok(e) = hits.GetElement(j) else { continue };
                        if e.CurrentControlType().ok() != Some(UIA_ListItemControlTypeId) {
                            continue;
                        }
                        if let Ok(inv) = e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
                            if inv.Invoke().is_ok() {
                                return Some(Pick::Done);
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Some(if seen { Pick::NoWhatsApp } else { Pick::NoSheet })
        })()
        .unwrap_or(Pick::NoSheet)
    }
}

#[cfg(not(windows))]
fn pick_whatsapp() -> Pick {
    Pick::NoSheet
}

fn names_str(files: &[PathBuf]) -> String {
    files.iter().filter_map(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
}

/// Prozent-Kodierung für Adressen.
fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

// ───────────────────────────── Link ohne Anmeldung ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Host {
    Catbox,
    Litterbox,
    Uguu,
    Tmpfiles,
    TempSh,
    Gofile,
}

impl Host {
    pub const ALL: [Host; 6] = [Host::Catbox, Host::Gofile, Host::TempSh, Host::Litterbox, Host::Uguu, Host::Tmpfiles];
    pub fn name(self) -> &'static str {
        match self {
            Host::Catbox => "catbox.moe",
            Host::Litterbox => "litterbox",
            Host::Uguu => "uguu.se",
            Host::Tmpfiles => "tmpfiles.org",
            Host::TempSh => "temp.sh",
            Host::Gofile => "Gofile",
        }
    }
    /// Haltbarkeit und Grenze.
    pub fn note(self) -> &'static str {
        match self {
            Host::Catbox => "dauerhaft · bis 200 MB",
            Host::Litterbox => "72 Stunden · bis 1 GB",
            Host::Uguu => "3 Stunden · bis 128 MB",
            Host::Tmpfiles => "48 Stunden · bis 100 MB",
            Host::TempSh => "3 Tage · bis 4 GB",
            Host::Gofile => "ohne feste Grenze · solange abgerufen",
        }
    }
    fn max(self) -> u64 {
        match self {
            Host::Catbox => 200 * MB,
            Host::Litterbox => 1024 * MB,
            Host::Uguu => 128 * MB,
            Host::Tmpfiles => 100 * MB,
            Host::TempSh => 4096 * MB,
            Host::Gofile => 10 * 1024 * MB,
        }
    }
    fn color(self) -> Color32 {
        rgb(match self {
            Host::Catbox => 0x8e5bd6,
            Host::Litterbox => 0x6c8ebf,
            Host::Uguu => 0xe0559b,
            Host::Tmpfiles => 0x2f9e8f,
            Host::TempSh => 0xd98c2b,
            Host::Gofile => 0x4a90e2,
        })
    }
    /// Nimmt der Dienst diese Datei an?
    pub fn accepts(self, p: &Path) -> bool {
        let ext = convert::ext_of(p);
        let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX);
        // catbox/litterbox sperren ausführbare Dateien und Word-Makroformate
        size > 0 && size <= self.max() && !(matches!(self, Host::Catbox | Host::Litterbox) && matches!(ext.as_str(), "exe" | "scr" | "cpl" | "doc" | "docx" | "jar"))
    }
    /// Ausweichdienste mit ähnlicher Haltbarkeit, falls dieser ablehnt.
    fn fallbacks(self) -> &'static [Host] {
        match self {
            Host::Catbox => &[Host::Gofile, Host::TempSh],
            Host::Litterbox => &[Host::TempSh, Host::Uguu, Host::Gofile],
            Host::Uguu => &[Host::Tmpfiles, Host::TempSh],
            Host::Tmpfiles => &[Host::TempSh, Host::Uguu],
            Host::TempSh => &[Host::Gofile, Host::Litterbox],
            Host::Gofile => &[Host::TempSh, Host::Catbox],
        }
    }
    /// Dienst für Messenger-Links: dauerhaft bevorzugt.
    fn for_links(files: &[PathBuf]) -> Option<Host> {
        [Host::Catbox, Host::TempSh, Host::Gofile].into_iter().find(|h| files.iter().all(|p| h.accepts(p)))
    }
}

fn mime(p: &Path) -> &'static str {
    match convert::ext_of(p).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "txt" | "md" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Lesestrom, der den Fortschritt zählt und beim Abbrechen mit einem Fehler endet.
struct Counted<R> {
    inner: R,
    prog: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Relaxed) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "abgebrochen"));
        }
        let n = self.inner.read(buf)?;
        self.prog.fetch_add(n as u64, Relaxed);
        Ok(n)
    }
}

/// multipart/form-data als Strom: Textfelder, dann die Datei. Rückgabe: (Content-Type, Länge, Strom).
fn multipart(fields: &[(&str, &str)], file_field: &str, p: &Path) -> Result<(String, u64, Box<dyn Read + Send>), String> {
    let boundary = format!("----foldex{:x}", crate::session::now_ms() ^ 0x5eed_f01d);
    let mut head = String::new();
    for (k, v) in fields {
        head.push_str(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"));
    }
    let name = p.file_name().map(|n| n.to_string_lossy().replace('"', "'")).unwrap_or_default();
    head.push_str(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{name}\"\r\nContent-Type: {}\r\n\r\n", mime(p)));
    let tail = format!("\r\n--{boundary}--\r\n");
    let f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let size = f.metadata().map_err(|e| e.to_string())?.len();
    let len = head.len() as u64 + size + tail.len() as u64;
    let body = std::io::Cursor::new(head.into_bytes()).chain(BufReader::with_capacity(1 << 16, f)).chain(std::io::Cursor::new(tail.into_bytes()));
    Ok((format!("multipart/form-data; boundary={boundary}"), len, Box::new(body)))
}

/// Verständliche Meldung zu einer Fehlerantwort (HTML-Fehlerseiten werden nicht wörtlich gezeigt).
fn http_err(e: ureq::Error, who: &str) -> String {
    match e {
        ureq::Error::Status(code, r) => {
            let body = r.into_string().unwrap_or_default();
            let html = body.trim_start().starts_with('<');
            let detail: String = if html { String::new() } else { body.trim().chars().take(160).collect() };
            let why = match code {
                412 => "hat die Datei abgelehnt (der Dienst meldet „keine Datei“ – das passiert, wenn seine Schutz-Firewall echte Dateien blockiert)",
                413 => "Datei zu groß",
                429 => "zu viele Uploads – später erneut versuchen",
                403 => "hat die Anfrage blockiert",
                500..=599 if html => "ist gestört bzw. seine Schutz-Firewall blockiert gerade",
                500..=599 => "ist gerade gestört",
                _ => "hat einen Fehler gemeldet",
            };
            format!("{who} {why} (Fehler {code}){}", if detail.is_empty() { String::new() } else { format!(": {detail}") })
        }
        ureq::Error::Transport(t) => format!("{who} ist nicht erreichbar: {t}"),
    }
}

/// Datei als multipart/form-data senden (feste Länge, keine Weiterleitungen) und die Antwort als Text liefern.
fn send_multipart(url: &str, fields: &[(&str, &str)], field: &str, p: &Path, prog: Arc<AtomicU64>, cancel: Arc<AtomicBool>, who: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout(Duration::from_secs(3600)).user_agent(UA).redirects(0).build();
    let (ct, len, body) = multipart(fields, field, p)?;
    let r = agent.post(url).set("Content-Type", &ct).set("Content-Length", &len.to_string()).send(Counted { inner: body, prog, cancel: cancel.clone() });
    if cancel.load(Relaxed) {
        return Err("abgebrochen".into());
    }
    r.map_err(|e| http_err(e, who))?.into_string().map_err(|e| e.to_string())
}

/// Lädt `p` anonym zu `host` hoch und gibt den Link zurück. Blockiert – im Hintergrund aufrufen.
pub fn upload(host: Host, p: &Path, prog: Arc<AtomicU64>, cancel: Arc<AtomicBool>) -> Result<String, String> {
    if !host.accepts(p) {
        return Err(format!("{} nimmt diese Datei nicht an (Art oder Größe)", host.name()));
    }
    let who = host.name();
    let send = |url: &str, fields: &[(&str, &str)], field: &str| send_multipart(url, fields, field, p, prog.clone(), cancel.clone(), who);
    let json = |t: &str| serde_json::from_str::<Value>(t).map_err(|_| format!("{who}: unerwartete Antwort: {}", t.chars().take(160).collect::<String>()));
    let link = match host {
        Host::Catbox => send("https://catbox.moe/user/api.php", &[("reqtype", "fileupload")], "fileToUpload")?,
        Host::Litterbox => send("https://litterbox.catbox.moe/resources/internals/api.php", &[("reqtype", "fileupload"), ("time", "72h")], "fileToUpload")?,
        Host::TempSh => send("https://temp.sh/upload", &[], "file")?,
        Host::Tmpfiles => {
            let v = json(&send("https://tmpfiles.org/api/v1/upload", &[("expire", "172800")], "file")?)?;
            let u = v.pointer("/data/url").and_then(Value::as_str).ok_or_else(|| format!("{who}: {v}"))?;
            // direkter Download statt Vorschauseite
            u.replacen("tmpfiles.org/", "tmpfiles.org/dl/", 1)
        }
        Host::Uguu => {
            let v = json(&send("https://uguu.se/upload.php", &[], "files[]")?)?;
            v.pointer("/files/0/url").and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("{who}: {}", v.get("description").unwrap_or(&v)))?
        }
        Host::Gofile => {
            let v = json(&send("https://upload.gofile.io/uploadfile", &[], "file")?)?;
            v.pointer("/data/downloadPage").and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("{who}: {v}"))?
        }
    };
    let link = link.trim().to_string();
    if link.starts_with("https://") || link.starts_with("http://") {
        Ok(link)
    } else {
        Err(format!("{who}: {}", link.chars().take(200).collect::<String>()))
    }
}

/// Hochladen mit Wiederholung und Ausweichdienst: Lehnt der Dienst ab, wird es einmal erneut versucht, danach bei
/// einem Ausweichdienst. Rückgabe: (Link, tatsächlich genutzter Dienst).
pub fn upload_with_fallback(host: Host, p: &Path, prog: Arc<AtomicU64>, cancel: Arc<AtomicBool>) -> Result<(String, Host), String> {
    let mut first_err = String::new();
    for (k, h) in std::iter::once(host).chain(std::iter::once(host)).chain(host.fallbacks().iter().copied()).enumerate() {
        if cancel.load(Relaxed) {
            return Err("abgebrochen".into());
        }
        if !h.accepts(p) {
            continue;
        }
        prog.store(0, Relaxed);
        match upload(h, p, prog.clone(), cancel.clone()) {
            Ok(l) => return Ok((l, h)),
            Err(e) if e == "abgebrochen" => return Err(e),
            Err(e) => {
                if k == 0 {
                    first_err = e;
                    std::thread::sleep(Duration::from_millis(800));
                }
            }
        }
    }
    Err(if first_err.is_empty() { "Kein Dienst hat die Datei angenommen".into() } else { format!("{first_err}. Auch die Ausweichdienste haben nicht geantwortet.") })
}

// ───────────────────────────── Fenster ─────────────────────────────

#[derive(Clone)]
enum Target {
    App(App),
    Upload(Host),
    Device(Device, localsend::Me),
    /// Upload für das WhatsApp-Fenster (Link landet dort, kein Ergebnis-Popup)
    WhatsAppLink(Host),
}

/// Auswahlfenster für WhatsApp.
struct WaDialog {
    /// App-ID von WhatsApp (zum direkten Öffnen)
    aumid: String,
    /// Nachricht für WhatsApp – bearbeitbar, mit Link nach dem Upload
    message: String,
    link: Option<String>,
    note: Option<(bool, String)>,
    copied: bool,
    /// läuft: im Windows-Teilen automatisch WhatsApp wählen (Ergebnis: gefunden?)
    auto: Option<Receiver<Pick>>,
}

/// Der laufende Vorgang (es gibt immer nur einen).
struct Job {
    title: String,
    file: String,
    prog: Arc<AtomicU64>,
    total: u64,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<Outcome, String>>,
    /// Ergebnis gehört ins WhatsApp-Fenster
    for_wa: bool,
}

/// Geräteauswahl für LocalSend.
struct Picker {
    disc: Discovery,
}

pub struct ShareWin {
    files: Vec<PathBuf>,
    total: u64,
    kind: String,
    apps: Vec<App>,
    apps_rx: Option<Receiver<(Vec<App>, Vec<(String, egui::ColorImage)>)>>,
    icons: HashMap<String, egui::TextureHandle>,
    thumb: Option<egui::TextureHandle>,
    thumb_rx: Option<Receiver<Option<egui::ColorImage>>>,
    picker: Option<Picker>,
    job: Option<Job>,
    /// Ergebnis bzw. Fehler – als Popup, bis es geschlossen wird
    result: Option<Result<Outcome, String>>,
    copied: bool,
    wa: Option<WaDialog>,
    /// Foldex-Fenster (Windows-Teilen)
    hwnd: isize,
}

impl ShareWin {
    /// `hwnd`: Foldex-Fenster (für das Windows-Teilen, über das WhatsApp Dateien annimmt).
    pub fn new(files: Vec<PathBuf>, ctx: &egui::Context, hwnd: isize) -> ShareWin {
        let files: Vec<PathBuf> = files.into_iter().filter(|p| p.is_file()).collect();
        let total = files.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        let mut kinds: Vec<&str> = files.iter().map(|p| convert::class_name(convert::class_of(p))).collect();
        kinds.sort();
        kinds.dedup();
        // Apps erkennen und ihre Symbole laden – im Hintergrund, das Fenster erscheint sofort
        let (tx, rx) = mpsc::channel();
        let c2 = ctx.clone();
        std::thread::spawn(move || {
            let apps = detect();
            let icons = apps.iter().filter_map(|a| a.icon.clone()).filter_map(|n| crate::preview::shell_icon(&n, 64).map(|i| (n, i))).collect();
            let _ = tx.send((apps, icons));
            c2.request_repaint();
        });
        let (ttx, trx) = mpsc::channel();
        let (first, c3) = (files.first().cloned(), ctx.clone());
        std::thread::spawn(move || {
            let img = first.and_then(|f| crate::preview::shell_image(&f, 96).or_else(|| crate::preview::shell_icon(&f.to_string_lossy(), 64)));
            let _ = ttx.send(img);
            c3.request_repaint();
        });
        ShareWin { files, total, kind: kinds.join(", "), apps: Vec::new(), apps_rx: Some(rx), icons: HashMap::new(), thumb: None, thumb_rx: Some(trx), picker: None, job: None, result: None, copied: false, wa: None, hwnd }
    }

    /// Nur für Bildschirmfotos (FOLDEX_SHOT): Fortschritt bzw. Ergebnis-Popup in einem festen Zustand zeigen.
    pub fn dev_state(&mut self, what: &str, ctx: &egui::Context) {
        match what {
            "progress" => {
                let (_tx, rx) = mpsc::channel();
                std::mem::forget(_tx);
                let prog = Arc::new(AtomicU64::new(self.total * 62 / 100));
                self.job = Some(Job { title: "Lade zu catbox.moe hoch".into(), file: names_str(&self.files), prog, total: self.total.max(1), cancel: Arc::new(AtomicBool::new(false)), rx, for_wa: false });
            }
            "wa-desktop" | "wa-share" => {
                let mut w = WaDialog { aumid: "5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App".into(), message: names_str(&self.files), link: None, note: None, copied: false, auto: None };
                if what == "wa-share" {
                    match share_to_whatsapp(self.hwnd, &self.files, ctx) {
                        Ok(rx) => w.auto = Some(rx),
                        Err(e) => w.note = Some((true, format!("WhatsApp lässt sich nicht öffnen: {e}"))),
                    }
                }
                self.wa = Some(w);
            }
            "wa" | "wa-link" => {
                let mut w = WaDialog { aumid: String::new(), message: names_str(&self.files), link: None, note: None, copied: false, auto: None };
                if what == "wa-link" {
                    let l = "https://files.catbox.moe/abc123.png".to_string();
                    w.message = format!("{}: {l}", names_str(&self.files));
                    w.link = Some(l);
                    w.note = Some((false, "Link erstellt – „In WhatsApp senden“ öffnet die Chat-Auswahl von WhatsApp mit dieser Nachricht.".into()));
                }
                self.wa = Some(w);
            }
            "error" => self.result = Some(Err("litterbox hat die Datei abgelehnt (der Dienst meldet „keine Datei“ – das passiert, wenn seine Schutz-Firewall echte Dateien blockiert) (Fehler 412). Auch die Ausweichdienste haben nicht geantwortet.".into())),
            _ => self.result = Some(Ok(outcome("Link erstellt", "Jeder mit dem Link kann die Datei laden.", Some("https://files.catbox.moe/abc123.png".into())))),
        }
    }

    /// LocalSend-Geräteauswahl öffnen (falls LocalSend eingerichtet ist).
    pub fn open_localsend(&mut self, ctx: &egui::Context) {
        if let Some(me) = localsend::identity() {
            self.open_picker(me, ctx);
        }
    }

    fn open_picker(&mut self, me: localsend::Me, ctx: &egui::Context) {
        let c = ctx.clone();
        self.picker = Some(Picker { disc: localsend::discover(me, Arc::new(move || c.request_repaint())) });
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    /// Neuen Vorgang starten – alte Ergebnisse verschwinden, angezeigt wird nur der aktuelle.
    fn start(&mut self, t: Target, ctx: &egui::Context) {
        if self.job.is_some() {
            return;
        }
        self.result = None;
        self.copied = false;
        let prog = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let files = self.files.clone();
        let (p2, c2, ctx2) = (prog.clone(), cancel.clone(), ctx.clone());
        let title = match &t {
            Target::App(a) if App::is_cloud_id(a.id) => format!("Kopiere nach {}", a.name),
            Target::App(a) if a.needs_link() => format!("Lade für {} hoch", a.name),
            Target::App(a) => format!("Übergebe an {}", a.name),
            Target::Upload(h) => format!("Lade zu {} hoch", h.name()),
            Target::Device(d, _) => format!("Sende an {} – auf dem Gerät bestätigen", d.alias),
            Target::WhatsAppLink(h) => format!("Lade für WhatsApp zu {} hoch", h.name()),
        };
        let for_wa = matches!(t, Target::WhatsAppLink(_));
        let link_host = Host::for_links(&files);
        std::thread::spawn(move || {
            let r = match t {
                Target::App(a) => send_to_app(&a, &files, link_host, &p2, &c2),
                Target::Device(d, me) => localsend::send(&d, &me, &files, p2, c2).map(|t| outcome("Gesendet", t, None)),
                Target::WhatsAppLink(h) => {
                    let mut links = Vec::new();
                    let mut res = Ok(());
                    for f in &files {
                        match upload_with_fallback(h, f, p2.clone(), c2.clone()) {
                            Ok((l, _)) => links.push(l),
                            Err(e) => {
                                res = Err(e);
                                break;
                            }
                        }
                    }
                    res.map(|_| outcome("", "", Some(links.join("\n"))))
                }
                Target::Upload(h) => {
                    let mut links = Vec::new();
                    let mut used = h;
                    let mut res = Ok(());
                    for f in &files {
                        match upload_with_fallback(h, f, p2.clone(), c2.clone()) {
                            Ok((l, u)) => {
                                links.push(l);
                                used = u;
                            }
                            Err(e) => {
                                res = Err(e);
                                break;
                            }
                        }
                    }
                    res.map(|_| {
                        let via = if used == h { String::new() } else { format!(" – {} hat abgelehnt, deshalb über {} ({})", h.name(), used.name(), used.note()) };
                        let text = format!("Jeder mit dem Link kann die Datei laden{via}.");
                        outcome(if links.len() == 1 { "Link erstellt".to_string() } else { format!("{} Links erstellt", links.len()) }, text, Some(links.join("\n")))
                    })
                }
            };
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        let file = if self.files.len() == 1 { names_str(&self.files) } else { format!("{} Dateien", self.files.len()) };
        self.job = Some(Job { title, file, prog, total: self.total.max(1), cancel, rx, for_wa });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(Ok((apps, icons))) = self.apps_rx.as_ref().map(|r| r.try_recv()) {
            self.apps = apps;
            for (n, img) in icons {
                let t = ctx.load_texture(format!("share-{n}"), img, egui::TextureOptions::LINEAR);
                self.icons.insert(n, t);
            }
            self.apps_rx = None;
        }
        if let Some(Ok(img)) = self.thumb_rx.as_ref().map(|r| r.try_recv()) {
            self.thumb = img.map(|i| ctx.load_texture("share-thumb", i, egui::TextureOptions::LINEAR));
            self.thumb_rx = None;
        }
        if let Some(j) = &self.job {
            match j.rx.try_recv() {
                Ok(r) if j.for_wa && self.wa.is_some() => {
                    // Link ins WhatsApp-Fenster: Nachricht vorbereiten, nichts automatisch senden
                    if let Some(w) = &mut self.wa {
                        match r {
                            Ok(Outcome { link: Some(l), .. }) => {
                                let name = names_str(&self.files);
                                w.message = if l.lines().count() == 1 { format!("{name}: {l}") } else { l.clone() };
                                w.link = Some(l);
                                w.copied = false;
                                w.note = Some((false, "Link erstellt – „In WhatsApp senden“ öffnet die Chat-Auswahl von WhatsApp mit dieser Nachricht.".into()));
                            }
                            Ok(_) => w.note = Some((true, "Kein Link erhalten".into())),
                            Err(e) => w.note = Some((true, if e == "abgebrochen" { "Hochladen abgebrochen.".into() } else { e })),
                        }
                    }
                    self.job = None;
                }
                Ok(r) => {
                    // Link gleich kopieren (Signal/Discord brauchen ihn in der Zwischenablage)
                    if let Ok(Outcome { link: Some(l), .. }) = &r {
                        ctx.copy_text(l.clone());
                        self.copied = true;
                    }
                    self.result = Some(r.map_err(|e| if e == "abgebrochen" { "Abgebrochen.".into() } else { e }));
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.job = None,
                Err(_) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        if self.apps_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    /// Zeichnet das Fenster, Geräteauswahl, Fortschritt und Ergebnis. `false` = geschlossen.
    pub fn ui(&mut self, ctx: &egui::Context, th: &Theme) -> bool {
        self.poll(ctx);
        let mut open = true;
        let frame = egui::Frame::window(&ctx.style()).fill(th.panel).rounding(Rounding::same(12.0)).stroke(Stroke::new(1.0, th.line)).inner_margin(Margin::same(18.0));
        let max_h = (ctx.screen_rect().height() - 60.0).clamp(320.0, 760.0);
        let busy = self.job.is_some() || self.result.is_some() || self.wa.is_some() || self.picker.is_some();
        // Esc schließt (außer während etwas läuft)
        if !busy && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
        egui::Window::new("Teilen")
            .title_bar(false)
            .frame(frame)
            .collapsible(false)
            .resizable(true)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .default_width(660.0)
            .default_height(max_h.min(700.0))
            .min_width(460.0)
            .max_height(max_h)
            .show(ctx, |ui| {
                // Kopf wie bei den anderen Foldex-Fenstern: Titel links, Schließen rechts
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Teilen").size(17.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled_ui(!busy, |ui| close_button(ui, th)).inner.clicked() {
                            open = false;
                        }
                    });
                });
                ui.add_space(6.0);
                // während ein Popup offen ist, ist das Fenster gesperrt
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(max_h - 60.0).show(ui, |ui| self.body(ui, th, ctx));
                });
            });
        self.picker_ui(ctx, th);
        self.wa_ui(ctx, th);
        self.progress_ui(ctx, th);
        self.result_ui(ctx, th);
        if !open {
            if let Some(j) = &self.job {
                j.cancel.store(true, Relaxed);
            }
        }
        open
    }

    fn popup_frame(ctx: &egui::Context, th: &Theme, accent: Color32) -> egui::Frame {
        egui::Frame::window(&ctx.style()).fill(th.panel).rounding(Rounding::same(12.0)).stroke(Stroke::new(1.0, accent)).inner_margin(Margin::same(20.0))
    }

    /// Fortschritt mittig im Fenster: Titel, Dateiname, Balken mit Prozent, Abbrechen.
    fn progress_ui(&mut self, ctx: &egui::Context, th: &Theme) {
        let Some(j) = &self.job else { return };
        let done = j.prog.load(Relaxed);
        let frac = (done as f32 / j.total as f32).clamp(0.0, 1.0);
        let mut cancel = false;
        egui::Window::new("share_progress")
            .title_bar(false)
            .frame(Self::popup_frame(ctx, th, th.line))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([380.0, 0.0])
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(&j.title).size(15.0).strong());
                ui.add(egui::Label::new(egui::RichText::new(&j.file).size(12.0).color(th.sub)).truncate());
                ui.add_space(10.0);
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
                ui.painter().rect_filled(r, 4.0, th.field);
                let fill = Rect::from_min_size(r.min, vec2((r.width() * frac).max(8.0), r.height()));
                ui.painter().rect_filled(fill, 4.0, th.accent);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let pct = if frac >= 1.0 { "Server antwortet …".to_string() } else { format!("{} %", (frac * 100.0) as u32) };
                    ui.label(egui::RichText::new(format!("{pct}   ·   {} von {}", crate::fsops::human(done.min(j.total)), crate::fsops::human(j.total))).size(12.0).color(th.sub));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Abbrechen").clicked() {
                            cancel = true;
                        }
                    });
                });
            });
        if cancel {
            j.cancel.store(true, Relaxed);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// Ergebnis bzw. Fehler mittig; nach dem Schließen ist alles weg.
    fn result_ui(&mut self, ctx: &egui::Context, th: &Theme) {
        let Some(r) = self.result.clone() else { return };
        let mut close = false;
        let ok_col = crate::gitui::status_color(crate::git::St::Added, th);
        let accent = if r.is_ok() { ok_col.gamma_multiply(0.7) } else { th.danger.gamma_multiply(0.8) };
        egui::Window::new("share_result")
            .title_bar(false)
            .frame(Self::popup_frame(ctx, th, accent))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([440.0, 0.0])
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                match &r {
                    Ok(o) => {
                        ui.horizontal(|ui| {
                            let (ir, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                            ui::draw(ui.painter(), ir, Ic::Check, ok_col);
                            ui.label(egui::RichText::new(&o.title).size(16.0).strong());
                        });
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(egui::RichText::new(&o.text).size(12.5).color(th.sub)).wrap());
                        if let Some(l) = &o.link {
                            ui.add_space(10.0);
                            egui::Frame::none().fill(th.field).rounding(Rounding::same(8.0)).inner_margin(Margin::symmetric(10.0, 8.0)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(egui::Label::new(egui::RichText::new(l).size(13.0).monospace().color(ok_col)).wrap().selectable(true));
                            });
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                let label = if self.copied { "Kopiert ✓" } else { "Kopieren" };
                                if ui.add(egui::Button::new(egui::RichText::new(label).strong().color(if th.dark { th.bg } else { Color32::WHITE })).fill(th.accent).min_size(vec2(110.0, 30.0))).clicked() {
                                    ctx.copy_text(l.clone());
                                    self.copied = true;
                                }
                                if ui.add(egui::Button::new("Öffnen").min_size(vec2(90.0, 30.0))).clicked() {
                                    for u in l.lines() {
                                        let _ = open::that_detached(u);
                                    }
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::Button::new("Schließen").min_size(vec2(90.0, 30.0))).clicked() {
                                        close = true;
                                    }
                                });
                            });
                        } else {
                            ui.add_space(10.0);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new("Schließen").min_size(vec2(90.0, 30.0))).clicked() {
                                    close = true;
                                }
                            });
                        }
                    }
                    Err(e) => {
                        ui.horizontal(|ui| {
                            let (ir, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                            ui::draw(ui.painter(), ir, Ic::Info, th.danger);
                            ui.label(egui::RichText::new("Teilen fehlgeschlagen").size(16.0).strong());
                        });
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(egui::RichText::new(e).size(12.5)).wrap());
                        ui.add_space(10.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("Schließen").min_size(vec2(90.0, 30.0))).clicked() {
                                close = true;
                            }
                        });
                    }
                }
            });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            // alles weg: kein Verlauf, keine alten Links oder Meldungen
            self.result = None;
            self.copied = false;
        }
    }

    /// WhatsApp: „Mit Link“ (hochladen, Nachricht mit Link vorbereiten) oder „Ohne Link“ (App direkt öffnen),
    /// Nachricht bearbeiten, kopieren oder per offiziellem Link (`whatsapp://send`) an WhatsApps Chat-Auswahl geben.
    fn wa_ui(&mut self, ctx: &egui::Context, th: &Theme) {
        if self.wa.is_none() {
            return;
        }
        let host = Host::for_links(&self.files);
        let hwnd = self.hwnd;
        let uploading = self.job.as_ref().is_some_and(|j| j.for_wa);
        let files = self.files.clone();
        let mut open = true;
        let mut upload = false;
        let green = rgb(0x25d366);
        let Some(w) = &mut self.wa else { return };
        egui::Window::new("An WhatsApp senden")
            .open(&mut open)
            .frame(Self::popup_frame(ctx, th, green.gamma_multiply(0.6)))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([470.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(names_str(&files)).size(12.5).color(th.sub)).truncate());
                ui.add_space(8.0);
                let card = |ui: &mut egui::Ui, title: &str, text: &str, button: &str, enabled: bool| -> bool {
                    let mut clicked = false;
                    egui::Frame::none().fill(th.field).stroke(Stroke::new(1.0, th.line)).rounding(Rounding::same(8.0)).inner_margin(Margin::symmetric(12.0, 10.0)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(ui.available_width() - 150.0);
                                ui.label(egui::RichText::new(title).size(14.0).strong());
                                ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(th.sub)).wrap());
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                clicked = ui.add_enabled(enabled, egui::Button::new(button).min_size(vec2(130.0, 30.0))).clicked();
                            });
                        });
                    });
                    ui.add_space(6.0);
                    clicked
                };
                let with_text = match host {
                    Some(h) => format!("Lädt die Datei zu {} hoch ({}) und bereitet die Nachricht mit dem Download-Link vor.", h.name(), h.note()),
                    None => "Kein Upload-Dienst nimmt diese Datei an (Art oder Größe).".to_string(),
                };
                if card(ui, "Mit Link", &with_text, if w.link.is_some() { "Erneut hochladen" } else { "Hochladen" }, host.is_some() && !uploading) {
                    upload = true;
                }
                let desktop = !w.aumid.is_empty();
                let ohne_text = if desktop {
                    "Schickt die Datei selbst, ohne Upload: WhatsApp öffnet seine Chat-Auswahl mit der Datei als Anhang – Chats wählen und senden."
                } else {
                    "Öffnet WhatsApp Web direkt, ohne Upload. WhatsApp Web nimmt Dateien anderer Programme nicht direkt an – die Datei liegt zum Einfügen bereit: im Chat Strg+V drücken."
                };
                // Ergebnis der automatischen Auswahl
                if let Some(r) = w.auto.as_ref().and_then(|rx| rx.try_recv().ok()) {
                    w.auto = None;
                    w.note = Some(match r {
                        Pick::Done => (false, "WhatsApp geöffnet – Chats auswählen und senden. Die Datei ist angehängt.".into()),
                        Pick::NoWhatsApp => (false, "WhatsApp war im Teilen-Fenster nicht zu finden – dort bitte selbst WhatsApp wählen.".into()),
                        Pick::NoSheet => (
                            true,
                            "Windows hat das Teilen-Fenster nicht angezeigt. Bitte prüfen, ob im Windows-Explorer Rechtsklick → „Teilen“ funktioniert – sonst hilft meist ab- und wieder anmelden. Alternativ „Mit Link“ nutzen.".into(),
                        ),
                    });
                }
                let busy = w.auto.is_some();
                if desktop && card(ui, "Ohne Link", ohne_text, if busy { "Öffnet …" } else { "In WhatsApp teilen" }, !uploading && !busy) {
                    // Windows reicht die Datei an WhatsApps Teilen-Ziel weiter; WhatsApp wird darin gleich ausgewählt
                    match share_to_whatsapp(hwnd, &files, ctx) {
                        Ok(rx) => {
                            w.auto = Some(rx);
                            w.note = Some((false, "WhatsApp wird geöffnet …".into()));
                        }
                        Err(e) => w.note = Some((true, format!("WhatsApp lässt sich nicht öffnen: {e}"))),
                    }
                } else if !desktop && card(ui, "Ohne Link", ohne_text, "WhatsApp Web öffnen", !uploading) {
                    // Datei in die Zwischenablage, damit sie sich in WhatsApp mit Strg+V einfügen lässt
                    let ok = crate::clip::set(&files, false) != 0 || !cfg!(windows);
                    // Desktop-App über ihre App-ID, sonst WhatsApp Web im Browser
                    let r = if desktop {
                        std::process::Command::new("explorer.exe").arg(apps_folder(&w.aumid)).spawn().map(|_| ())
                    } else {
                        open::that_detached("https://web.whatsapp.com").map_err(std::io::Error::other)
                    };
                    let where_ = if desktop { "WhatsApp" } else { "WhatsApp Web" };
                    w.note = Some(match (r, ok) {
                        (Ok(_), true) => (false, format!("{where_} geöffnet – Chat wählen und Strg+V drücken, dann senden.")),
                        (Ok(_), false) => (true, format!("{where_} geöffnet, aber die Zwischenablage war belegt – Datei aus dem Explorer in den Chat ziehen.")),
                        (Err(e), _) => (true, format!("{where_} lässt sich nicht öffnen: {e}")),
                    });
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Nachricht").size(12.5).color(th.sub));
                ui.add(egui::TextEdit::multiline(&mut w.message).desired_rows(2).desired_width(f32::INFINITY));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let label = if w.copied { "Kopiert ✓" } else { "Kopieren" };
                    if ui.add_enabled(!w.message.trim().is_empty(), egui::Button::new(label).min_size(vec2(100.0, 30.0))).on_hover_text("Nachricht (mit Link) in die Zwischenablage").clicked() {
                        ctx.copy_text(w.message.clone());
                        w.copied = true;
                    }
                    let send = egui::Button::new(egui::RichText::new("In WhatsApp senden").strong().color(Color32::WHITE)).fill(green).min_size(vec2(160.0, 30.0));
                    if ui.add_enabled(!w.message.trim().is_empty() && !uploading, send).on_hover_text("Öffnet WhatsApps Chat-Auswahl mit dieser Nachricht").clicked() {
                        // Desktop-App über das offizielle Schema, sonst der universelle wa.me-Link (öffnet Web bzw. App)
                        let url = if w.aumid.is_empty() { format!("https://wa.me/?text={}", enc(&w.message)) } else { format!("whatsapp://send?text={}", enc(&w.message)) };
                        match open::that_detached(url) {
                            Ok(()) => w.note = Some((false, "WhatsApp geöffnet – Chat wählen und senden.".into())),
                            Err(e) => w.note = Some((true, format!("WhatsApp lässt sich nicht öffnen: {e}"))),
                        }
                    }
                });
                if let Some((err, t)) = &w.note {
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(egui::RichText::new(t).size(12.0).color(if *err { th.danger } else { th.sub })).wrap());
                }
            });
        if upload {
            if let Some(h) = host {
                self.start(Target::WhatsAppLink(h), ctx);
            }
        }
        if !open {
            if let Some(j) = self.job.as_ref().filter(|j| j.for_wa) {
                j.cancel.store(true, Relaxed);
            }
            self.wa = None;
        }
    }

    /// LocalSend: Geräte im Netz, Klick sendet. Die LocalSend-App bleibt geschlossen.
    fn picker_ui(&mut self, ctx: &egui::Context, th: &Theme) {
        let Some(p) = &self.picker else { return };
        let devices = p.disc.devices.lock().map(|v| v.clone()).unwrap_or_default();
        let searching = p.disc.searching.load(Relaxed);
        let (done, total) = (p.disc.scanned.load(Relaxed), p.disc.total);
        let nets = p.disc.nets.join(", ");
        let me = p.disc.me.clone();
        let green = rgb(0x2bb38a);
        let mut close = false;
        let mut pick: Option<Device> = None;
        let mut rescan = false;
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        egui::Window::new("ls_picker")
            .title_bar(false)
            .frame(Self::popup_frame(ctx, th, th.line))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([500.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                // Kopf: LocalSend-Zeichen, Titel, als wer gesendet wird, Schließen
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 20.0, green.gamma_multiply(0.18));
                    ui.painter().circle_stroke(r.center(), 11.0, Stroke::new(2.2, green));
                    ui.painter().circle_filled(r.center(), 5.0, green);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(egui::RichText::new("An ein Gerät senden").size(16.5).strong());
                        ui.label(egui::RichText::new(format!("über LocalSend · als „{}“", me.alias)).size(12.0).color(th.sub));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if close_button(ui, th).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(10.0);
                ui.add(egui::Label::new(egui::RichText::new(names_str(&self.files)).size(12.0).color(th.sub)).truncate());
                ui.add_space(10.0);

                // Suche: Fortschritt über alle Adressen im Netz
                if searching {
                    let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, th.field);
                    ui.painter().rect_filled(Rect::from_min_size(r.min, vec2((r.width() * frac).max(6.0), r.height())), 2.0, green);
                    ui.add_space(4.0);
                    let txt = if nets.is_empty() { "Suche Geräte …".to_string() } else { format!("Suche Geräte in {nets} …  {done} von {total}") };
                    ui.label(egui::RichText::new(txt).size(11.5).color(th.sub));
                    ui.add_space(6.0);
                }

                // Geräte als Karten
                egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                    for d in &devices {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
                        let hov = resp.hovered();
                        ui.painter().rect(r, 10.0, if hov { th.hover } else { th.field }, Stroke::new(1.0, if hov { green.gamma_multiply(0.7) } else { th.line }));
                        let ic = Rect::from_center_size(pos2(r.left() + 30.0, r.center().y), vec2(36.0, 36.0));
                        ui.painter().circle_filled(ic.center(), 18.0, green.gamma_multiply(0.16));
                        device_icon(ui.painter(), ic.shrink(9.0), &d.kind, green);
                        let tx = ic.right() + 14.0;
                        let tw = r.right() - tx - 100.0;
                        ui::paint_text(ui, tx, r.center().y - 9.0, &d.alias, 14.5, th.text, tw, false);
                        ui::paint_text(ui, tx, r.center().y + 10.0, &d.detail(), 11.5, th.sub, tw, false);
                        // „Senden“ rechts – deutlich beim Darüberfahren
                        let b = Rect::from_center_size(pos2(r.right() - 52.0, r.center().y), vec2(78.0, 28.0));
                        ui.painter().rect_filled(b, 14.0, if hov { green } else { th.panel });
                        ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, "Senden", FontId::proportional(12.5), if hov { Color32::WHITE } else { th.sub });
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            pick = Some(d.clone());
                        }
                        ui.add_space(6.0);
                    }
                });
                if devices.is_empty() {
                    {
                        egui::Frame::none().fill(th.field).rounding(Rounding::same(10.0)).inner_margin(Margin::same(14.0)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(egui::RichText::new(if searching { "Noch kein Gerät gefunden" } else { "Kein Gerät gefunden" }).size(14.0).strong());
                            ui.add_space(4.0);
                            for t in [
                                "LocalSend auf dem anderen Gerät öffnen und im Vordergrund lassen",
                                "Beide Geräte im selben WLAN – nicht im Gast-WLAN",
                                "Auf dem Handy in LocalSend „Sichtbar“ bzw. „Schnelles Speichern“ prüfen",
                                "VPN auf diesem PC kurz ausschalten, falls es lokale Geräte blockiert",
                            ] {
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                    ui.painter().circle_filled(r.center(), 2.5, green);
                                    ui.add(egui::Label::new(egui::RichText::new(t).size(12.0).color(th.sub)).wrap());
                                });
                            }
                        });
                    }
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let label = if searching { "Sucht …" } else { "Neu suchen" };
                    if ui.add_enabled(!searching, egui::Button::new(label).min_size(vec2(110.0, 30.0))).clicked() {
                        rescan = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Abbrechen").min_size(vec2(100.0, 30.0))).clicked() {
                            close = true;
                        }
                        if !devices.is_empty() {
                            ui.label(egui::RichText::new(format!("{} Gerät(e) gefunden", devices.len())).size(12.0).color(th.sub));
                        }
                    });
                });
            });
        if searching {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if rescan {
            self.open_picker(me.clone(), ctx);
        }
        if let Some(d) = pick {
            self.picker = None;
            self.start(Target::Device(d, me), ctx);
        } else if close {
            self.picker = None;
        }
    }

    fn body(&mut self, ui: &mut egui::Ui, th: &Theme, ctx: &egui::Context) {
        if self.files.is_empty() {
            ui.label(egui::RichText::new("Keine Dateien ausgewählt (Ordner vorher komprimieren).").color(th.danger));
            return;
        }
        // Kopf: Vorschau, Name(n), Größe
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
            ui.painter().rect_filled(r, 10.0, th.field);
            match &self.thumb {
                Some(t) => {
                    let s = t.size_vec2();
                    let k = (48.0 / s.x).min(48.0 / s.y);
                    ui.painter().image(t.id(), Rect::from_center_size(r.center(), s * k), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => ui::draw(ui.painter(), Rect::from_center_size(r.center(), vec2(26.0, 26.0)), Ic::Share, th.accent),
            }
            ui.vertical(|ui| {
                ui.add_space(6.0);
                let shown: Vec<String> = self.files.iter().take(3).filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
                let more = if self.files.len() > 3 { format!(" und {} weitere", self.files.len() - 3) } else { String::new() };
                ui.add(egui::Label::new(egui::RichText::new(shown.join(", ") + &more).size(15.0).strong()).truncate());
                ui.label(egui::RichText::new(format!("{} Datei(en) · {} · {}", self.files.len(), crate::fsops::human(self.total), self.kind)).size(12.0).color(th.sub));
            });
        });
        ui.add_space(4.0);
        if self.apps_rx.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new("suche installierte Apps …").size(12.0).color(th.sub));
            });
        }

        let mut pick: Option<Target> = None;
        let group = |ui: &mut egui::Ui, title: &str, sub: &str| {
            ui.add_space(12.0);
            ui.label(egui::RichText::new(title).size(13.5).strong());
            ui.label(egui::RichText::new(sub).size(11.5).color(th.sub));
            ui.add_space(4.0);
        };
        let cols = ((ui.available_width() + 8.0) / 200.0).floor().clamp(1.0, 4.0) as usize;
        let tile_w = (ui.available_width() - 8.0 * (cols as f32 - 1.0)) / cols as f32;
        let link_ok = Host::for_links(&self.files).is_some();

        // Nur installierte Apps; Link-Apps nur, wenn ein Dienst die Datei annimmt; Bereiche ohne App entfallen
        let direct: Vec<App> = self.apps.iter().filter(|a| !App::is_cloud_id(a.id) && (!a.needs_link() || link_ok)).cloned().collect();
        let cloud: Vec<App> = self.apps.iter().filter(|a| App::is_cloud_id(a.id)).cloned().collect();
        for (title, sub, list) in [("Senden", "Über deine installierten Apps und ihre Konten", &direct), ("In die Cloud", "Die Cloud-App lädt mit deinem Konto hoch – Link danach über „Freigeben“ im Explorer", &cloud)] {
            if list.is_empty() {
                continue;
            }
            group(ui, title, sub);
            grid(ui, cols, list.len(), |ui, k| {
                let a = &list[k];
                let icon = a.icon.as_ref().and_then(|n| self.icons.get(n));
                if tile(ui, th, tile_w, a.name, &a.hint(), icon, a.color).clicked() {
                    pick = Some(Target::App(a.clone()));
                }
            });
        }

        // Link ohne Anmeldung: nur Dienste, die alle gewählten Dateien annehmen
        let hosts: Vec<Host> = Host::ALL.into_iter().filter(|h| self.files.iter().all(|p| h.accepts(p))).collect();
        if !hosts.is_empty() {
            group(ui, "Link erstellen", "Ohne Konto und Anmeldung – der Link wird gleich kopiert. Jeder mit dem Link kann die Datei laden.");
            grid(ui, cols, hosts.len(), |ui, k| {
                let h = hosts[k];
                if tile(ui, th, tile_w, h.name(), h.note(), None, h.color()).clicked() {
                    pick = Some(Target::Upload(h));
                }
            });
        }

        match pick {
            Some(Target::App(App { via: Via::LocalSend(me), .. })) => self.open_picker(me, ctx),
            Some(Target::App(App { via: Via::WhatsApp(aumid), .. })) => {
                self.result = None;
                self.wa = Some(WaDialog { aumid, message: names_str(&self.files), link: None, note: None, copied: false, auto: None });
            }
            Some(t) => self.start(t, ctx),
            None => {}
        }
    }
}

/// Runder Schließen-Knopf (×) für Popups ohne Titelleiste.
fn close_button(ui: &mut egui::Ui, th: &Theme) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().circle_filled(r.center(), 14.0, th.hover);
    }
    let c = r.center();
    let st = Stroke::new(1.5, if resp.hovered() { th.text } else { th.sub });
    ui.painter().line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], st);
    ui.painter().line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], st);
    resp.on_hover_text("Schließen (Esc)")
}

/// Symbol je Geräteart: Handy, Browser oder PC.
fn device_icon(p: &egui::Painter, r: Rect, kind: &str, col: Color32) {
    let st = Stroke::new(1.6, col);
    match kind {
        "mobile" => {
            let b = Rect::from_center_size(r.center(), vec2(r.width() * 0.62, r.height()));
            p.rect_stroke(b, 3.0, st);
            p.line_segment([pos2(b.center().x - 2.5, b.bottom() - 3.0), pos2(b.center().x + 2.5, b.bottom() - 3.0)], st);
        }
        "web" => {
            p.circle_stroke(r.center(), r.width() * 0.5, st);
            p.line_segment([pos2(r.left(), r.center().y), pos2(r.right(), r.center().y)], st);
            p.line_segment([pos2(r.center().x, r.top()), pos2(r.center().x, r.bottom())], st);
        }
        _ => ui::draw(p, r, Ic::Pc, col),
    }
}

/// Raster mit `n` Kacheln in `cols` Spalten.
fn grid(ui: &mut egui::Ui, cols: usize, n: usize, mut cell: impl FnMut(&mut egui::Ui, usize)) {
    for row in 0..n.div_ceil(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for k in row * cols..(row * cols + cols).min(n) {
                cell(ui, k);
            }
        });
        ui.add_space(4.0);
    }
}

/// Kachel: Programmsymbol (bzw. farbiges Monogramm), Name, Hinweis.
fn tile(ui: &mut egui::Ui, th: &Theme, w: f32, name: &str, hint: &str, icon: Option<&egui::TextureHandle>, color: Color32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(w, 54.0), Sense::click());
    let hot = resp.hovered();
    ui.painter().rect(r, 10.0, if hot { th.hover } else { th.field }, Stroke::new(1.0, if hot { color.gamma_multiply(0.8) } else { th.line }));
    let ir = Rect::from_center_size(pos2(r.left() + 27.0, r.center().y), vec2(30.0, 30.0));
    match icon {
        Some(t) => {
            ui.painter().image(t.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(ir, 8.0, color);
            let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            ui.painter().text(ir.center(), egui::Align2::CENTER_CENTER, letter, FontId::proportional(16.0), Color32::WHITE);
        }
    }
    let x = ir.right() + 10.0;
    let tw = r.right() - x - 8.0;
    ui::paint_text(ui, x, r.center().y - 9.0, name, 13.5, th.text, tw, false);
    ui::paint_text(ui, x, r.center().y + 10.0, hint, 11.0, th.sub, tw, false);
    resp.on_hover_text(hint).on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rohe Anfrage eines Uploads an einen lokalen Server: Kopfzeilen und Multipart-Inhalt wie ein Server prüfen.
    #[test]
    fn multipart_request_on_the_wire() {
        use std::io::{Read as _, Write as _};
        let d = std::env::temp_dir().join(format!("fx-wire-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("Bild äöü.png");
        let data: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&f, &data).unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api.php", l.local_addr().unwrap());
        let srv = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = s.read(&mut buf).unwrap_or(0);
                raw.extend_from_slice(&buf[..n]);
                if let Some(h) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&raw[..h]).to_lowercase();
                    let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).map(|v| v.trim().parse().unwrap()).unwrap_or(0);
                    if raw.len() >= h + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let reply = "https://x.example/abc";
            s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).unwrap();
            raw
        });
        let link = send_multipart(&url, &[("reqtype", "fileupload"), ("time", "72h")], "fileToUpload", &f, Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)), "Test");
        assert_eq!(link.unwrap(), "https://x.example/abc");
        let raw = srv.join().unwrap();
        let h = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&raw[..h]).to_string();
        let body = &raw[h + 4..];
        let lower = head.to_lowercase();
        assert!(!lower.contains("transfer-encoding"), "{head}");
        let len: usize = lower.lines().find_map(|l| l.strip_prefix("content-length:")).unwrap().trim().parse().unwrap();
        assert_eq!(len, body.len(), "Content-Length stimmt nicht");
        let boundary = head.lines().find_map(|l| l.split_once("boundary=").map(|(_, b)| b.trim().to_string())).unwrap();
        let text = String::from_utf8_lossy(body);
        assert!(text.starts_with(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"reqtype\"\r\n\r\nfileupload\r\n")), "{}", &text[..200.min(text.len())]);
        assert!(text.contains("name=\"time\"\r\n\r\n72h\r\n"));
        assert!(text.ends_with(&format!("\r\n--{boundary}--\r\n")));
        // Dateiteil: Inhalt unverändert zwischen Kopf und Abschluss
        let at = text.find("name=\"fileToUpload\"; filename=").unwrap();
        let start = body[at..].windows(4).position(|w| w == b"\r\n\r\n").unwrap() + at + 4;
        let end = body.len() - format!("\r\n--{boundary}--\r\n").len();
        assert_eq!(&body[start..end], &data[..]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn apps_hosts_and_cloud_copy() {
        // nur gefundene Apps, jede höchstens einmal
        let apps = detect();
        for id in [AppId::LocalSend, AppId::WhatsApp, AppId::Signal, AppId::Discord, AppId::Telegram, AppId::Mail, AppId::OneDrive] {
            assert!(apps.iter().filter(|a| a.id == id).count() <= 1, "{id:?}");
        }
        assert!(apps.iter().any(|a| a.id == AppId::Mail), "E-Mail ist immer da");
        let d = std::env::temp_dir().join(format!("fx-share-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let exe = d.join("b.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        assert!(!Host::Catbox.accepts(&exe));
        assert!(Host::TempSh.accepts(&exe));
        assert_eq!(Host::for_links(std::slice::from_ref(&exe)), Some(Host::TempSh));
        let empty = d.join("leer.txt");
        std::fs::write(&empty, b"").unwrap();
        assert!(!Host::TempSh.accepts(&empty), "leere Dateien werden nicht hochgeladen");
        assert_eq!(enc("a b&c/ä"), "a%20b%26c%2F%C3%A4");
        // Cloud: Kopie in den „synchronisierten“ Ordner mit Fortschritt
        let cloud = App { id: AppId::OneDrive, name: "Test-Cloud", via: Via::Cloud(d.join("cloud")), icon: None, color: Color32::WHITE };
        std::fs::create_dir_all(d.join("cloud")).unwrap();
        let prog = Arc::new(AtomicU64::new(0));
        let r = send_to_app(&cloud, std::slice::from_ref(&exe), None, &prog, &Arc::new(AtomicBool::new(false)));
        assert!(r.is_ok(), "{r:?}");
        assert!(d.join("cloud").join("Foldex Freigaben").join("b.exe").is_file());
        assert_eq!(prog.load(Relaxed), 2);
        // Link-App ohne passenden Dienst: verständlicher Fehler statt Absturz
        let wa = App { id: AppId::WhatsApp, name: "WhatsApp", via: Via::LinkDeep("whatsapp://send?text={text}"), icon: None, color: Color32::WHITE };
        let e = send_to_app(&wa, std::slice::from_ref(&exe), None, &prog, &Arc::new(AtomicBool::new(false))).unwrap_err();
        assert!(e.contains("Kein Upload-Dienst"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn error_messages_are_readable() {
        // HTML-Fehlerseiten (Firewall) werden nicht wörtlich gezeigt
        let srv = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", srv.local_addr().unwrap());
        let t = std::thread::spawn(move || {
            use std::io::{Read as _, Write as _};
            for body in ["No file!", "<html><body>BunkerWeb</body></html>"] {
                let (mut s, _) = srv.accept().unwrap();
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b);
                let code = if body.starts_with('<') { "500 Internal Server Error" } else { "412 Precondition Failed" };
                let _ = s.write_all(format!("HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            }
        });
        let e412 = ureq::get(&url).call().map(|_| ()).map_err(|e| http_err(e, "litterbox")).unwrap_err();
        assert!(e412.contains("abgelehnt") && e412.contains("412"), "{e412}");
        let e500 = ureq::get(&url).call().map(|_| ()).map_err(|e| http_err(e, "litterbox")).unwrap_err();
        assert!(e500.contains("Firewall") && !e500.contains("<html"), "{e500}");
        t.join().unwrap();
    }
}
