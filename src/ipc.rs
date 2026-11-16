//! Verbindung zwischen mehreren Foldex-Fenstern (jedes Fenster ist ein eigener Prozess).
//! Ohne Sockets, nur über kleine Dateien im Konfigurationsordner:
//!
//! * `win-<pid>.txt`   – Bildschirmbereich des Fensters (wird regelmäßig aufgefrischt → „lebt noch“)
//! * `drag.txt`        – läuft gerade ein Ziehen eines Tabs/Teilfensters? (Quelle schreibt Position + Inhalt)
//! * `inbox-<an>-<von>-<n>.msg` – „hier wurde etwas auf dein Fenster abgelegt“
//! * `ack-<an>-<n>`    – Bestätigung des Empfängers (erst dann schließt die Quelle ihren Tab)

use eframe::egui;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering::Relaxed},
        mpsc, Arc, Mutex,
    },
    time::{Duration, SystemTime},
};

pub fn pid() -> u32 {
    std::process::id()
}

fn dir() -> PathBuf {
    let d = match std::env::var_os("FILECORE_IPC_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("ipc"),
    };
    let _ = fs::create_dir_all(&d);
    d
}

/// Was beim Ziehen mitwandert: Titel, Kachel-Layout (Text) und die Ordner der Teilfenster.
#[derive(Clone, PartialEq, Debug)]
pub struct Payload {
    pub title: String,
    pub layout: String,
    pub paths: Vec<PathBuf>,
}

impl Payload {
    pub fn encode(&self) -> String {
        let mut s = format!("{}\n{}\n", self.title.replace('\n', " "), self.layout);
        for p in &self.paths {
            s.push_str(&p.to_string_lossy().replace('\n', " "));
            s.push('\n');
        }
        s
    }

    pub fn decode(s: &str) -> Option<Payload> {
        let mut l = s.lines();
        let title = l.next()?.to_string();
        let layout = l.next()?.to_string();
        let paths: Vec<PathBuf> = l.filter(|x| !x.is_empty()).map(PathBuf::from).collect();
        (!paths.is_empty()).then_some(Payload { title, layout, paths })
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct DragInfo {
    pub pid: u32,
    pub pos: [f32; 2],
    pub payload: Payload,
}

fn fresh(p: &Path, max: Duration) -> bool {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|d| d <= max)
}

fn write_atomic(path: &Path, text: &str) {
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

// ── Fenster-Verzeichnis ──
pub fn write_window(r: [f32; 4]) {
    write_atomic(&dir().join(format!("win-{}.txt", pid())), &format!("{} {} {} {}", r[0], r[1], r[2], r[3]));
}

pub fn remove_window() {
    let _ = fs::remove_file(dir().join(format!("win-{}.txt", pid())));
}

/// Andere lebende Fenster und ihr Bildschirmbereich (x, y, Breite, Höhe).
pub fn other_windows() -> Vec<(u32, [f32; 4])> {
    let mut v = Vec::new();
    let Ok(rd) = fs::read_dir(dir()) else { return v };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix("win-").and_then(|s| s.strip_suffix(".txt")).and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if id == pid() {
            continue;
        }
        if !fresh(&e.path(), Duration::from_secs(3)) {
            if !fresh(&e.path(), Duration::from_secs(60)) {
                let _ = fs::remove_file(e.path());
            }
            continue;
        }
        let Ok(t) = fs::read_to_string(e.path()) else { continue };
        let n: Vec<f32> = t.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if n.len() == 4 {
            v.push((id, [n[0], n[1], n[2], n[3]]));
        }
    }
    v
}

/// Welches andere Fenster liegt unter dem Bildschirmpunkt?
pub fn window_at(p: [f32; 2]) -> Option<u32> {
    other_windows()
        .into_iter()
        .find(|(_, r)| p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3])
        .map(|(id, _)| id)
}

// ── laufendes Ziehen ──
pub fn write_drag(pos: [f32; 2], payload: &Payload) {
    write_atomic(&dir().join("drag.txt"), &format!("{}\n{} {}\n{}", pid(), pos[0], pos[1], payload.encode()));
}

pub fn clear_drag() {
    let p = dir().join("drag.txt");
    if let Ok(t) = fs::read_to_string(&p) {
        if t.lines().next().and_then(|l| l.parse::<u32>().ok()) == Some(pid()) {
            let _ = fs::remove_file(p);
        }
    }
}

fn read_drag() -> Option<DragInfo> {
    let p = dir().join("drag.txt");
    if !fresh(&p, Duration::from_millis(1500)) {
        return None;
    }
    let t = fs::read_to_string(&p).ok()?;
    let mut l = t.splitn(3, '\n');
    let from: u32 = l.next()?.parse().ok()?;
    let pos: Vec<f32> = l.next()?.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    let payload = Payload::decode(l.next()?)?;
    (pos.len() == 2).then_some(DragInfo { pid: from, pos: [pos[0], pos[1]], payload })
}

// ── Ablegen auf ein anderes Fenster ──
static SEQ: AtomicU64 = AtomicU64::new(1);

pub fn send_drop(to: u32, pos: [f32; 2], payload: &Payload) -> u64 {
    let n = SEQ.fetch_add(1, Relaxed);
    write_atomic(
        &dir().join(format!("inbox-{to}-{}-{n}.msg", pid())),
        &format!("{} {}\n{}", pos[0], pos[1], payload.encode()),
    );
    n
}

pub struct DropMsg {
    pub from: u32,
    pub seq: u64,
    pub pos: [f32; 2],
    pub payload: Payload,
}

fn take_drops() -> Vec<DropMsg> {
    let mut v = Vec::new();
    let prefix = format!("inbox-{}-", pid());
    let Ok(rd) = fs::read_dir(dir()) else { return v };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(&prefix).and_then(|s| s.strip_suffix(".msg")) else { continue };
        let Some((from, seq)) = rest.split_once('-') else { continue };
        let (Ok(from), Ok(seq)) = (from.parse::<u32>(), seq.parse::<u64>()) else { continue };
        let text = fs::read_to_string(e.path());
        let _ = fs::remove_file(e.path());
        let Ok(text) = text else { continue };
        let Some((head, body)) = text.split_once('\n') else { continue };
        let pos: Vec<f32> = head.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if let (2, Some(payload)) = (pos.len(), Payload::decode(body)) {
            v.push(DropMsg { from, seq, pos: [pos[0], pos[1]], payload });
        }
    }
    v
}

pub fn send_ack(to: u32, seq: u64) {
    let _ = fs::write(dir().join(format!("ack-{to}-{seq}")), "");
}

fn take_acks() -> Vec<u64> {
    let mut v = Vec::new();
    let prefix = format!("ack-{}-", pid());
    let Ok(rd) = fs::read_dir(dir()) else { return v };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(n) = name.strip_prefix(&prefix).and_then(|s| s.parse::<u64>().ok()) {
            let _ = fs::remove_file(e.path());
            v.push(n);
        }
    }
    v
}

pub enum Event {
    /// Ein anderes Fenster zieht gerade etwas (`None` = Ziehen beendet)
    Drag(Option<DragInfo>),
    Drop(DropMsg),
    Ack(u64),
}

/// Hintergrund-Thread: hält den eigenen Fenstereintrag aktuell und meldet Ereignisse anderer Fenster.
pub fn spawn_watcher(ctx: egui::Context, rect: Arc<Mutex<Option<[f32; 4]>>>) -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut last: Option<DragInfo> = None;
        let mut tick = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(60));
            tick = tick.wrapping_add(1);
            if tick % 8 == 1 {
                if let Some(r) = *rect.lock().unwrap() {
                    write_window(r);
                }
            }
            let cur = read_drag().filter(|d| d.pid != pid());
            if cur != last {
                last = cur.clone();
                let _ = tx.send(Event::Drag(cur));
                ctx.request_repaint();
            }
            for d in take_drops() {
                let _ = tx.send(Event::Drop(d));
                ctx.request_repaint();
            }
            for a in take_acks() {
                let _ = tx.send(Event::Ack(a));
                ctx.request_repaint();
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip() {
        let p = Payload { title: "Bilder".into(), layout: "H0.500(0,1)".into(), paths: vec!["/a".into(), "/b c".into()] };
        assert_eq!(Payload::decode(&p.encode()), Some(p));
        assert!(Payload::decode("x\ny\n").is_none());
    }

    #[test]
    fn drop_and_window_registry() {
        let d = std::env::temp_dir().join(format!("fc-ipc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        std::env::set_var("FILECORE_IPC_DIR", &d);
        let p = Payload { title: "t".into(), layout: "0".into(), paths: vec!["/x".into()] };
        // an uns selbst senden (pid ist gleich) → take_drops findet es
        let n = send_drop(pid(), [10.0, 20.0], &p);
        let got = take_drops();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].seq, got[0].pos, got[0].from), (n, [10.0, 20.0], pid()));
        assert_eq!(got[0].payload, p);
        assert!(take_drops().is_empty());
        send_ack(pid(), 7);
        assert_eq!(take_acks(), vec![7]);
        // Fensterverzeichnis: fremdes Fenster eintragen
        fs::write(d.join("win-999999.txt"), "100 100 800 600").unwrap();
        assert_eq!(window_at([150.0, 150.0]), Some(999999));
        assert_eq!(window_at([50.0, 50.0]), None);
        // Ziehen
        write_drag([5.0, 6.0], &p);
        assert_eq!(read_drag().unwrap().pos, [5.0, 6.0]);
        clear_drag();
        assert!(read_drag().is_none());
        let _ = fs::remove_dir_all(&d);
    }
}
