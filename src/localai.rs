//! Lokale KI für den Bildeditor – gerechnet wird auf diesem PC, kein Bild verlässt ihn.
//!
//! * Hauptmotiv erkennen (Hintergrund entfernen / unscharf): ISNet über ONNX Runtime
//! * Objekte auswählen: Segment Anything (ViT-B, quantisiert) – das Bild wird einmal kodiert, danach braucht
//!   jeder Klick nur noch Millisekunden
//! * Schwarz-Weiß kolorieren: DeOldify; die Helligkeit bleibt vom Original, nur die Farbe kommt vom Modell
//! * Hochskalieren: Real-ESRGAN (eigenes Programm über Vulkan, nutzt die Grafikkarte)
//!
//! Modelle und ONNX Runtime lädt der Editor erst nach Rückfrage einmalig von GitHub in den
//! Konfigurationsordner (`…/foldex/ai`); danach läuft alles ohne Netz.

use image::{GrayImage, Luma, Rgba, RgbaImage};
use ort::session::Session;
use ort::value::Tensor;
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(PartialEq, Eq)]
enum Kind {
    /// eine Datei, so wie sie ist
    File,
    /// ONNX-Runtime-Paket: nur die DLLs aus `lib/`
    OrtZip,
    /// Real-ESRGAN-Paket: alles in einen eigenen Ordner
    ExeZip,
}

pub struct Asset {
    pub name: &'static str,
    url: &'static str,
    file: &'static str,
    pub mb: u32,
    kind: Kind,
}

pub static ORT: Asset = Asset {
    name: "ONNX Runtime 1.22",
    url: if cfg!(windows) { "https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-win-x64-1.22.0.zip" } else { "https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-linux-x64-1.22.0.tgz" },
    file: if cfg!(windows) { "onnxruntime.dll" } else { "libonnxruntime.so" },
    mb: 72,
    kind: Kind::OrtZip,
};
pub static ISNET: Asset = Asset {
    name: "ISNet (Motiv-Erkennung)",
    url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx",
    file: "isnet-general-use.onnx",
    mb: 178,
    kind: Kind::File,
};
// ViT-L statt ViT-B: deutlich genauere Kanten und weniger „Löcher“ in Objekten
pub static SAM_ENC: Asset = Asset {
    name: "Segment Anything ViT-L – Kodierer",
    url: "https://github.com/vietanhdev/anylabeling-assets/releases/download/v0.2.0/segment_anything_vit_l_encoder_quant.onnx",
    file: "sam_vit_l_encoder_quant.onnx",
    mb: 323,
    kind: Kind::File,
};
pub static SAM_DEC: Asset = Asset {
    name: "Segment Anything ViT-L – Dekodierer",
    url: "https://github.com/vietanhdev/anylabeling-assets/releases/download/v0.2.0/segment_anything_vit_l_decoder_quant.onnx",
    file: "sam_vit_l_decoder_quant.onnx",
    mb: 9,
    kind: Kind::File,
};
pub static DEOLDIFY: Asset = Asset {
    name: "DeOldify (Kolorieren)",
    url: "https://github.com/facefusion/facefusion-assets/releases/download/models-3.0.0/deoldify_artistic.onnx",
    file: "deoldify_artistic.onnx",
    mb: 255,
    kind: Kind::File,
};
pub static LAMA: Asset = Asset {
    // 1024er-Export von Big-LaMa: doppelte Auflösung gegenüber dem üblichen 512er – deutlich schärfer
    name: "LaMa HD (Objekt entfernen)",
    url: "https://huggingface.co/Vincent615/lama-2048-onnx/resolve/main/lama_1024.onnx",
    file: "lama_1024.onnx",
    mb: 208,
    kind: Kind::File,
};
pub static ESRGAN: Asset = Asset {
    name: "Real-ESRGAN (Hochskalieren)",
    url: if cfg!(windows) { "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-windows.zip" } else { "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-ubuntu.zip" },
    file: "realesrgan",
    mb: 45,
    kind: Kind::ExeZip,
};

/// Wofür ein Schritt Modelle braucht
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Need {
    Subject,
    Objects,
    Colorize,
    Upscale,
    Inpaint,
}

fn assets(n: Need) -> &'static [&'static Asset] {
    static SUBJECT: [&Asset; 2] = [&ORT, &ISNET];
    static OBJECTS: [&Asset; 3] = [&ORT, &SAM_ENC, &SAM_DEC];
    static COLORIZE: [&Asset; 2] = [&ORT, &DEOLDIFY];
    static UPSCALE: [&Asset; 1] = [&ESRGAN];
    static INPAINT: [&Asset; 2] = [&ORT, &LAMA];
    match n {
        Need::Subject => &SUBJECT,
        Need::Objects => &OBJECTS,
        Need::Colorize => &COLORIZE,
        Need::Upscale => &UPSCALE,
        Need::Inpaint => &INPAINT,
    }
}

pub fn dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("ai")
}

fn target(a: &Asset) -> PathBuf {
    dir().join(a.file)
}

fn have(a: &Asset) -> bool {
    match a.kind {
        Kind::ExeZip => esrgan_exe().is_some(),
        _ => target(a).is_file(),
    }
}

/// noch fehlende Dateien für diese Schritte (jede nur einmal)
pub fn missing(needs: &[Need]) -> Vec<&'static Asset> {
    let mut out: Vec<&'static Asset> = Vec::new();
    for n in needs {
        for a in assets(*n) {
            if !have(a) && !out.iter().any(|o| std::ptr::eq(*o, *a)) {
                out.push(a);
            }
        }
    }
    out
}

/// alle Dateien für diese Schritte (auch schon vorhandene; jede nur einmal) – für Größenangaben
pub fn needed(needs: &[Need]) -> Vec<&'static Asset> {
    let mut out: Vec<&'static Asset> = Vec::new();
    for n in needs {
        for a in assets(*n) {
            if !out.iter().any(|o| std::ptr::eq(*o, *a)) {
                out.push(a);
            }
        }
    }
    out
}

/// Nur unter Windows (x64) gibt es die fertigen Pakete
pub fn supported() -> bool {
    cfg!(all(windows, target_arch = "x86_64"))
}

// ───────────────────────── Fortschritt ─────────────────────────

/// Fortschritt einer laufenden Aufgabe (Anteil < 0 = unbestimmt) und Abbruch-Wunsch
#[derive(Clone, Default)]
pub struct Progress(Arc<Mutex<(f32, String)>>, Arc<AtomicBool>);

impl Progress {
    pub fn set(&self, frac: f32, text: impl Into<String>) {
        if let Ok(mut g) = self.0.lock() {
            *g = (frac, text.into());
        }
    }
    pub fn get(&self) -> (f32, String) {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }
    pub fn cancel(&self) {
        self.1.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> Result<(), String> {
        if self.1.load(Ordering::Relaxed) {
            Err("Abgebrochen".into())
        } else {
            Ok(())
        }
    }
}

// ───────────────────────── Herunterladen ─────────────────────────

/// Dateien früherer Versionen (ViT-B → ViT-L, LaMa 512 → 1024) – gehören der App, also weg damit
pub fn remove_obsolete() {
    for f in ["vit_b-encoder-quant.onnx", "vit_b-decoder-quant.onnx", "lama_fp32.onnx"] {
        let _ = std::fs::remove_file(dir().join(f));
    }
}

pub fn ensure(needs: &[Need], p: &Progress) -> Result<(), String> {
    remove_obsolete();
    if !supported() {
        return Err("Lokale KI gibt es bisher nur für Windows (x64)".into());
    }
    for a in missing(needs) {
        download(a, p)?;
    }
    Ok(())
}

/// alle Modelle mit ihrem Zweck (Einstellungen → KI-Modelle)
pub fn catalog() -> [(&'static Asset, &'static str); 7] {
    [
        (&ORT, "Laufzeit für alle KI-Modelle außer Hochskalieren"),
        (&SAM_ENC, "Objekte auswählen"),
        (&SAM_DEC, "Objekte auswählen"),
        (&ISNET, "Hauptmotiv, Hintergrund entfernen / unscharf"),
        (&LAMA, "Objekt entfernen (Stelle neu erzeugen)"),
        (&DEOLDIFY, "Schwarz-Weiß kolorieren"),
        (&ESRGAN, "KI-Hochskalieren"),
    ]
}

pub fn installed(a: &Asset) -> bool {
    have(a)
}

/// belegter Platz auf der Platte
pub fn disk_size(a: &Asset) -> u64 {
    fn walk(p: &std::path::Path) -> u64 {
        match std::fs::metadata(p) {
            Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| walk(&e.path())).sum()).unwrap_or(0),
            Ok(m) => m.len(),
            Err(_) => 0,
        }
    }
    let mut n = walk(&target(a));
    if a.kind == Kind::OrtZip {
        // weitere DLLs aus dem Paket
        n += std::fs::read_dir(dir()).map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("onnxruntime_")).map(|e| walk(&e.path())).sum()).unwrap_or(0);
    }
    n
}

/// Modell löschen. Die ONNX-Runtime-DLL ist nach Benutzung bis zum Neustart gesperrt.
pub fn uninstall(a: &Asset) -> Result<(), String> {
    unload();
    let t = target(a);
    let r = if a.kind == Kind::ExeZip { std::fs::remove_dir_all(&t) } else { std::fs::remove_file(&t) };
    if a.kind == Kind::OrtZip {
        if let Ok(rd) = std::fs::read_dir(dir()) {
            for e in rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("onnxruntime_")) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    match r {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) if a.kind == Kind::OrtZip => Err("ONNX Runtime ist geladen – nach einem Neustart von Foldex entfernen".into()),
        Err(e) => Err(format!("{}: {e}", a.name)),
    }
}

/// Installation im Hintergrund (Einstellungen): Fortschritt, Ergebnis
struct Install {
    prog: Progress,
    done: Arc<Mutex<Option<Result<(), String>>>>,
}

fn install_slot() -> &'static Mutex<Option<Install>> {
    static S: Mutex<Option<Install>> = Mutex::new(None);
    &S
}

pub fn install(list: Vec<&'static Asset>) {
    let Ok(mut g) = install_slot().lock() else { return };
    if g.as_ref().is_some_and(|i| i.done.lock().is_ok_and(|d| d.is_none())) {
        return;
    }
    let (prog, done) = (Progress::default(), Arc::new(Mutex::new(None)));
    let (p, d) = (prog.clone(), done.clone());
    std::thread::spawn(move || {
        let r = if supported() { list.into_iter().filter(|a| !have(a)).try_for_each(|a| download(a, &p)) } else { Err("Lokale KI gibt es bisher nur für Windows (x64)".into()) };
        if let Ok(mut d) = d.lock() {
            *d = Some(r);
        }
    });
    *g = Some(Install { prog, done });
}

/// läuft gerade eine Installation? (Anteil, Text)
pub fn install_status() -> Option<(f32, String)> {
    let g = install_slot().lock().ok()?;
    let i = g.as_ref()?;
    let running = i.done.lock().ok()?.is_none();
    running.then(|| i.prog.get())
}

/// Ergebnis der letzten Installation (einmal abholen)
pub fn install_result() -> Option<Result<(), String>> {
    let mut g = install_slot().lock().ok()?;
    let r = g.as_ref()?.done.lock().ok()?.take();
    if r.is_some() {
        *g = None;
    }
    r
}

pub fn cancel_install() {
    if let Ok(g) = install_slot().lock() {
        if let Some(i) = g.as_ref() {
            i.prog.cancel();
        }
    }
}

fn download(a: &Asset, p: &Progress) -> Result<(), String> {
    // nie zwei Downloads derselben Datei gleichzeitig (Editor und Einstellungen)
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().map_err(|_| "Download gesperrt".to_string())?;
    if have(a) {
        return Ok(());
    }
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| format!("Ordner {}: {e}", d.display()))?;
    let tmp = d.join(format!("{}.part", a.url.rsplit('/').next().unwrap_or(a.file)));
    let agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(15)).build();
    let resp = agent.get(a.url).call().map_err(|e| format!("{} laden: {e}", a.name))?;
    let total = resp.header("Content-Length").and_then(|s| s.parse::<u64>().ok()).unwrap_or(a.mb as u64 * 1_000_000).max(1);
    let mut rd = resp.into_reader();
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let res = (|| -> Result<(), String> {
        loop {
            p.check()?;
            let n = rd.read(&mut buf).map_err(|e| format!("{} laden: {e}", a.name))?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            p.set(done as f32 / total as f32, format!("{} laden … {} / {} MB", a.name, done / 1_000_000, total / 1_000_000));
        }
        f.flush().map_err(|e| e.to_string())
    })();
    drop(f);
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if done < total / 2 {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: Download unvollständig", a.name));
    }
    p.set(-1.0, format!("{} einrichten …", a.name));
    let r = match a.kind {
        Kind::File => std::fs::rename(&tmp, target(a)).map_err(|e| e.to_string()),
        Kind::OrtZip => unzip(&tmp, |name| {
            // onnxruntime-win-x64-1.22.0/lib/onnxruntime.dll → ai/onnxruntime.dll
            // onnxruntime-linux-x64-1.22.0/lib/libonnxruntime.so.1.22.0 → ai/libonnxruntime.so
            let n = name.replace('\\', "/");
            let file = n.rsplit('/').next().unwrap_or_default();
            if !n.contains("/lib/") {
                None
            } else if file.ends_with(".dll") {
                Some(d.join(file))
            } else if file.starts_with("libonnxruntime.so") {
                Some(d.join("libonnxruntime.so"))
            } else {
                None
            }
        }),
        Kind::ExeZip => {
            let root = target(a);
            unzip(&tmp, |name| {
                let n = name.replace('\\', "/");
                // nichts außerhalb des Zielordners
                (!n.split('/').any(|c| c == "..") && !n.starts_with('/')).then(|| root.join(&n))
            })
        }
    };
    let _ = std::fs::remove_file(&tmp);
    r?;
    if !have(a) {
        return Err(format!("{}: Paket enthält nicht das Erwartete", a.name));
    }
    Ok(())
}

/// Einträge eines Pakets entpacken – ZIP (Windows-Pakete) oder tar.gz (Linux-Pakete), erkannt am Inhalt;
/// `to` liefert das Ziel (None = überspringen). Unter Linux bleiben Ausführrechte erhalten.
pub(crate) fn unzip(zip: &std::path::Path, to: impl Fn(&str) -> Option<PathBuf>) -> Result<(), String> {
    use std::io::Read;
    let mut magic = [0u8; 2];
    std::fs::File::open(zip).and_then(|mut f| f.read_exact(&mut magic)).map_err(|e| e.to_string())?;
    let write = |dst: &std::path::Path, rd: &mut dyn Read, mode: Option<u32>| -> Result<(), String> {
        if let Some(par) = dst.parent() {
            std::fs::create_dir_all(par).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        std::io::copy(rd, &mut out).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Programme und Bibliotheken ausführbar machen (ZIPs aus Windows-Werkzeugen haben oft keine Rechte)
            let m = mode.filter(|m| m & 0o777 != 0).unwrap_or(0o755) | 0o600;
            let _ = std::fs::set_permissions(dst, std::fs::Permissions::from_mode(m & 0o7777));
        }
        #[cfg(not(unix))]
        let _ = mode;
        Ok(())
    };
    if magic == [0x1f, 0x8b] {
        let f = std::fs::File::open(zip).map_err(|e| e.to_string())?;
        let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
        for e in ar.entries().map_err(|e| e.to_string())? {
            let mut e = e.map_err(|e| e.to_string())?;
            if !e.header().entry_type().is_file() {
                continue;
            }
            let name = e.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
            let Some(dst) = to(&name) else { continue };
            let mode = e.header().mode().ok();
            write(&dst, &mut e, mode)?;
        }
        return Ok(());
    }
    let f = std::fs::File::open(zip).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        if e.is_dir() {
            continue;
        }
        let Some(dst) = to(e.name()) else { continue };
        let mode = e.unix_mode();
        write(&dst, &mut e, mode)?;
    }
    Ok(())
}

fn esrgan_exe() -> Option<PathBuf> {
    let root = target(&ESRGAN);
    let name = if cfg!(windows) { "realesrgan-ncnn-vulkan.exe" } else { "realesrgan-ncnn-vulkan" };
    let direct = root.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    // manche Pakete haben noch einen Unterordner
    std::fs::read_dir(&root).ok()?.flatten().map(|e| e.path().join(name)).find(|p| p.is_file())
}

// ───────────────────────── ONNX Runtime ─────────────────────────

fn oe(e: ort::Error) -> String {
    format!("KI: {e}")
}

fn ort_ready() -> Result<(), String> {
    static OK: Mutex<bool> = Mutex::new(false);
    let mut ok = OK.lock().map_err(|_| "KI gesperrt".to_string())?;
    if *ok {
        return Ok(());
    }
    let dll = target(&ORT);
    if !dll.is_file() {
        return Err("ONNX Runtime fehlt".into());
    }
    // ein kaputtes Laden der DLL darf nicht den Editor mitreißen
    let r = std::panic::catch_unwind(|| ort::init_from(dll.to_string_lossy()).with_name("foldex").commit());
    match r {
        Ok(Ok(_)) => {
            *ok = true;
            Ok(())
        }
        Ok(Err(e)) => Err(oe(e)),
        Err(_) => Err("ONNX Runtime ließ sich nicht laden".into()),
    }
}

fn sessions() -> &'static Mutex<HashMap<&'static str, Session>> {
    static S: OnceLock<Mutex<HashMap<&'static str, Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Modell (einmal geladen, dann im Speicher) benutzen
fn with_session<R>(a: &'static Asset, f: impl FnOnce(&mut Session) -> Result<R, String>) -> Result<R, String> {
    ort_ready()?;
    let mut m = sessions().lock().map_err(|_| "KI gesperrt".to_string())?;
    if !m.contains_key(a.file) {
        let s = Session::builder()
            .and_then(|b| b.with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3))
            .and_then(|b| b.commit_from_file(target(a)))
            .map_err(|e| format!("{}: {e}", a.name))?;
        m.insert(a.file, s);
    }
    f(m.get_mut(a.file).expect("eben geladen"))
}

/// Modelle aus dem Speicher werfen (Editor geschlossen)
pub fn unload() {
    if let Ok(mut m) = sessions().lock() {
        m.clear();
    }
}

fn in_shape(s: &Session, i: usize) -> Vec<i64> {
    s.inputs.get(i).and_then(|x| x.input_type.tensor_shape()).map(|s| s.to_vec()).unwrap_or_default()
}

// ───────────────────────── Hauptmotiv (ISNet) ─────────────────────────

/// Maske des Hauptmotivs in Bildgröße (255 = Motiv)
pub fn subject_mask(img: &RgbaImage, p: &Progress) -> Result<GrayImage, String> {
    ensure(&[Need::Subject], p)?;
    p.set(-1.0, "Hauptmotiv erkennen …");
    const S: u32 = 1024;
    let (w, h) = img.dimensions();
    let small = image::imageops::resize(img, S, S, image::imageops::FilterType::Triangle);
    let n = (S * S) as usize;
    // wie rembg: durch den hellsten Wert teilen, dann −0,5
    let max = small.pixels().flat_map(|p| [p[0], p[1], p[2]]).max().unwrap_or(255).max(1) as f32;
    let mut x = vec![0f32; 3 * n];
    for (i, px) in small.pixels().enumerate() {
        for c in 0..3 {
            x[c * n + i] = px[c] as f32 / max - 0.5;
        }
    }
    let out = with_session(&ISNET, |s| {
        let name = s.inputs[0].name.clone();
        let t = Tensor::from_array(([1usize, 3, S as usize, S as usize], x)).map_err(oe)?;
        let o = s.run(ort::inputs![name => t]).map_err(oe)?;
        let (_, d) = o[0].try_extract_tensor::<f32>().map_err(oe)?;
        Ok(d.get(..n).ok_or("KI: unerwartete Ausgabe")?.to_vec())
    })?;
    p.check()?;
    let (mi, ma) = out.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let k = 255.0 / (ma - mi).max(1e-6);
    let m = GrayImage::from_fn(S, S, |x, y| Luma([((out[(y * S + x) as usize] - mi) * k).round().clamp(0.0, 255.0) as u8]));
    Ok(image::imageops::resize(&m, w, h, image::imageops::FilterType::Triangle))
}

// ───────────────────────── Objekte auswählen (Segment Anything) ─────────────────────────

/// kodiertes Bild – gilt, bis sich das Bild ändert
pub struct SamEmbed {
    data: Vec<f32>,
    shape: Vec<i64>,
    w: u32,
    h: u32,
}

/// Maßstab vom Bild auf die 1024er-Eingabe des Kodierers (längere Seite = 1024)
fn sam_scale(w: u32, h: u32) -> f32 {
    1024.0 / w.max(h) as f32
}

pub fn sam_embed(img: &RgbaImage, p: &Progress) -> Result<SamEmbed, String> {
    ensure(&[Need::Objects], p)?;
    p.set(-1.0, "Bild für die Objektauswahl analysieren …");
    let (w, h) = img.dimensions();
    let k = sam_scale(w, h);
    let (nw, nh) = (((w as f32 * k).round() as u32).clamp(1, 1024), ((h as f32 * k).round() as u32).clamp(1, 1024));
    let small = image::imageops::resize(img, nw, nh, image::imageops::FilterType::Triangle);
    with_session(&SAM_ENC, |s| {
        let shape = in_shape(s, 0);
        let name = s.inputs[0].name.clone();
        const N: usize = 1024 * 1024;
        // rechts/unten mit Schwarz auf 1024 × 1024 aufgefüllt
        let t = if shape.len() == 4 && shape[1] == 3 {
            // NCHW mit SAM-Normierung
            let (mean, std) = ([123.675f32, 116.28, 103.53], [58.395f32, 57.12, 57.375]);
            let mut x = vec![0f32; 3 * N];
            for (px, py, c) in small.enumerate_pixels() {
                let i = py as usize * 1024 + px as usize;
                for ch in 0..3 {
                    x[ch * N + i] = (c[ch] as f32 - mean[ch]) / std[ch];
                }
            }
            Tensor::from_array(([1usize, 3, 1024, 1024], x))
        } else {
            // HWC 0–255, Vorverarbeitung steckt im Modell (rembg-Variante)
            let mut x = vec![0f32; 3 * N];
            for (px, py, c) in small.enumerate_pixels() {
                let i = (py as usize * 1024 + px as usize) * 3;
                for ch in 0..3 {
                    x[i + ch] = c[ch] as f32;
                }
            }
            if shape.len() == 4 {
                Tensor::from_array((vec![1i64, 1024, 1024, 3], x))
            } else {
                Tensor::from_array((vec![1024i64, 1024, 3], x))
            }
        }
        .map_err(oe)?;
        let o = s.run(ort::inputs![name => t]).map_err(oe)?;
        let (sh, d) = o[0].try_extract_tensor::<f32>().map_err(oe)?;
        Ok(SamEmbed { data: d.to_vec(), shape: sh.to_vec(), w, h })
    })
}

/// Maske aus Klickpunkten (x, y in Bildpunkten; true = gehört dazu). `prev` ist die grobe Maske (256 × 256
/// Logits) des letzten Klicks – damit verfeinert jeder weitere Klick die Auswahl, statt neu anzufangen.
/// Liefert die Maske in Bildgröße und die neue grobe Maske.
pub fn sam_mask(e: &SamEmbed, pts: &[(f32, f32, bool)], prev: Option<&[f32]>) -> Result<(GrayImage, Vec<f32>), String> {
    let k = sam_scale(e.w, e.h);
    // Maske höchstens 2048 px groß rechnen, dann auf Bildgröße ziehen
    let q = (2048.0 / e.w.max(e.h) as f32).min(1.0);
    let (ow, oh) = (((e.w as f32 * q).round() as u32).max(1), ((e.h as f32 * q).round() as u32).max(1));
    let mut coords: Vec<f32> = pts.iter().flat_map(|&(x, y, _)| [x * k, y * k]).collect();
    let mut labels: Vec<f32> = pts.iter().map(|&(_, _, pos)| if pos { 1.0 } else { 0.0 }).collect();
    // Füllpunkt, wie ihn das exportierte Modell ohne Rahmen erwartet
    coords.extend([0.0, 0.0]);
    labels.push(-1.0);
    let n = labels.len();
    let prev = prev.filter(|p| p.len() == 256 * 256);
    let (masks, ious, mshape, low) = with_session(&SAM_DEC, |s| {
        let mut ins: Vec<(std::borrow::Cow<'static, str>, ort::session::SessionInputValue<'static>)> = Vec::new();
        for i in &s.inputs {
            let v: ort::value::DynValue = match i.name.as_str() {
                "image_embeddings" => Tensor::from_array((e.shape.clone(), e.data.clone())).map_err(oe)?.into_dyn(),
                "point_coords" => Tensor::from_array(([1usize, n, 2], coords.clone())).map_err(oe)?.into_dyn(),
                "point_labels" => Tensor::from_array(([1usize, n], labels.clone())).map_err(oe)?.into_dyn(),
                "mask_input" => Tensor::from_array(([1usize, 1, 256, 256], prev.map_or_else(|| vec![0f32; 256 * 256], |p| p.to_vec()))).map_err(oe)?.into_dyn(),
                "has_mask_input" => Tensor::from_array(([1usize], vec![if prev.is_some() { 1f32 } else { 0.0 }])).map_err(oe)?.into_dyn(),
                "orig_im_size" => Tensor::from_array(([2usize], vec![oh as f32, ow as f32])).map_err(oe)?.into_dyn(),
                other => return Err(format!("KI: unbekannte Modell-Eingabe „{other}“")),
            };
            ins.push((i.name.clone().into(), v.into()));
        }
        let o = s.run(ins).map_err(oe)?;
        let m = o.get("masks").unwrap_or(&o[0]);
        let (sh, d) = m.try_extract_tensor::<f32>().map_err(oe)?;
        let ious = o.get("iou_predictions").and_then(|v| v.try_extract_tensor::<f32>().ok()).map(|(_, d)| d.to_vec()).unwrap_or_default();
        let low = o.get("low_res_masks").and_then(|v| v.try_extract_tensor::<f32>().ok()).map(|(_, d)| d.to_vec()).unwrap_or_default();
        Ok((d.to_vec(), ious, sh.to_vec(), low))
    })?;
    // [1, N, H, W]: die Maske mit der besten geschätzten Güte
    let (mh, mw) = (mshape.get(2).copied().unwrap_or(oh as i64) as u32, mshape.get(3).copied().unwrap_or(ow as i64) as u32);
    let plane = (mh * mw) as usize;
    let count = (masks.len() / plane.max(1)).max(1);
    let best = (0..count).max_by(|a, b| ious.get(*a).unwrap_or(&0.0).total_cmp(ious.get(*b).unwrap_or(&0.0))).unwrap_or(0);
    let m = &masks[best * plane..(best + 1) * plane];
    let low = low.get(best * 256 * 256..(best + 1) * 256 * 256).map(|s| s.to_vec()).unwrap_or_default();
    // Logits → weiche Kante
    let mut g = GrayImage::from_fn(mw, mh, |x, y| Luma([(255.0 / (1.0 + (-m[(y * mw + x) as usize] * 2.0).exp())).round() as u8]));
    let seeds: Vec<(u32, u32)> = pts.iter().filter(|p| p.2).map(|&(x, y, _)| (((x * mw as f32 / e.w as f32) as u32).min(mw - 1), ((y * mh as f32 / e.h as f32) as u32).min(mh - 1))).collect();
    clean_mask(&mut g, &seeds);
    let g = if (mw, mh) == (e.w, e.h) { g } else { image::imageops::resize(&g, e.w, e.h, image::imageops::FilterType::Triangle) };
    Ok((g, low))
}

/// Sprenkel und Löcher entfernen: kleine abgetrennte Inseln ohne Klickpunkt fallen weg, kleine Löcher im
/// Objekt werden gefüllt (jeweils unter 1 % der Objektfläche).
fn clean_mask(m: &mut GrayImage, seeds: &[(u32, u32)]) {
    let (w, h) = (m.width() as usize, m.height() as usize);
    let on: Vec<bool> = m.as_raw().iter().map(|v| *v > 127).collect();
    let area = on.iter().filter(|v| **v).count();
    if area == 0 {
        return;
    }
    let small = (area / 100).max(16);
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut comp = Vec::new();
    for start in 0..w * h {
        if seen[start] {
            continue;
        }
        let val = on[start];
        comp.clear();
        stack.push(start);
        seen[start] = true;
        let mut border = false;
        while let Some(i) = stack.pop() {
            comp.push(i);
            let (x, y) = (i % w, i / w);
            border |= x == 0 || y == 0 || x == w - 1 || y == h - 1;
            let mut go = |j: usize| {
                if !seen[j] && on[j] == val {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                go(i - 1);
            }
            if x + 1 < w {
                go(i + 1);
            }
            if y > 0 {
                go(i - w);
            }
            if y + 1 < h {
                go(i + w);
            }
        }
        if comp.len() >= small {
            continue;
        }
        let raw = m.as_mut();
        if val {
            // Insel: bleibt nur, wenn hier geklickt wurde
            if !seeds.iter().any(|&(sx, sy)| comp.contains(&(sy as usize * w + sx as usize))) {
                comp.iter().for_each(|&i| raw[i] = 0);
            }
        } else if !border {
            // Loch mitten im Objekt
            comp.iter().for_each(|&i| raw[i] = 255);
        }
    }
}

// ───────────────────────── Kolorieren (DeOldify) ─────────────────────────

pub fn colorize(img: &RgbaImage, p: &Progress) -> Result<RgbaImage, String> {
    ensure(&[Need::Colorize], p)?;
    p.set(-1.0, "Kolorieren …");
    let (w, h) = img.dimensions();
    let (color, mw, mh) = with_session(&DEOLDIFY, |s| {
        let sh = in_shape(s, 0);
        let (mh, mw) = if sh.len() == 4 && sh[2] > 0 && sh[3] > 0 { (sh[2] as u32, sh[3] as u32) } else { (512, 512) };
        let small = image::imageops::resize(img, mw, mh, image::imageops::FilterType::Triangle);
        let n = (mw * mh) as usize;
        let mut x = vec![0f32; 3 * n];
        for (i, px) in small.pixels().enumerate() {
            let l = 0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32;
            for c in 0..3 {
                x[c * n + i] = l;
            }
        }
        let name = s.inputs[0].name.clone();
        let t = Tensor::from_array(([1usize, 3, mh as usize, mw as usize], x)).map_err(oe)?;
        let o = s.run(ort::inputs![name => t]).map_err(oe)?;
        let (_, d) = o[0].try_extract_tensor::<f32>().map_err(oe)?;
        if d.len() < 3 * n {
            return Err("KI: unerwartete Ausgabe".into());
        }
        Ok((d[..3 * n].to_vec(), mw, mh))
    })?;
    p.check()?;
    let n = (mw * mh) as usize;
    let small = RgbaImage::from_fn(mw, mh, |x, y| {
        let i = (y * mw + x) as usize;
        Rgba([color[i].clamp(0.0, 255.0) as u8, color[n + i].clamp(0.0, 255.0) as u8, color[2 * n + i].clamp(0.0, 255.0) as u8, 255])
    });
    let big = image::imageops::resize(&small, w, h, image::imageops::FilterType::CatmullRom);
    Ok(merge_color(img, &big))
}

/// Helligkeit (L) vom Original, Farbe (a, b) vom Modell – so bleibt das Bild so scharf wie vorher
fn merge_color(orig: &RgbaImage, color: &RgbaImage) -> RgbaImage {
    let mut out = orig.clone();
    out.as_mut().par_chunks_mut(4).zip(color.as_raw().par_chunks(4)).for_each(|(o, c)| {
        let gray = 0.299 * o[0] as f32 + 0.587 * o[1] as f32 + 0.114 * o[2] as f32;
        let l = rgb_to_lab([gray / 255.0; 3])[0];
        let lab = rgb_to_lab([c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]);
        let rgb = lab_to_rgb([l, lab[1], lab[2]]);
        for i in 0..3 {
            o[i] = (rgb[i] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    });
    out
}

fn to_lin(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB (0–1) → CIE-Lab (D65)
pub fn rgb_to_lab(c: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = c.map(to_lin);
    let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
    let z = (0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b) / 1.088_83;
    let f = |t: f32| if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

pub fn lab_to_rgb(l: [f32; 3]) -> [f32; 3] {
    let fy = (l[0] + 16.0) / 116.0;
    let (fx, fz) = (fy + l[1] / 500.0, fy - l[2] / 200.0);
    let inv = |t: f32| if t > 0.206_893 { t * t * t } else { (t - 16.0 / 116.0) / 7.787 };
    let (x, y, z) = (inv(fx) * 0.950_47, inv(fy), inv(fz) * 1.088_83);
    let r = 3.240_454_2 * x - 1.537_138_5 * y - 0.498_531_4 * z;
    let g = -0.969_266 * x + 1.876_010_8 * y + 0.041_556 * z;
    let b = 0.055_643_4 * x - 0.204_025_9 * y + 1.057_225_2 * z;
    [to_srgb(r), to_srgb(g), to_srgb(b)]
}

// ───────────────────────── Hochskalieren (Real-ESRGAN) ─────────────────────────

pub fn upscale(img: &RgbaImage, factor: u32, drawing: bool, p: &Progress) -> Result<RgbaImage, String> {
    ensure(&[Need::Upscale], p)?;
    let exe = esrgan_exe().ok_or("Real-ESRGAN fehlt")?;
    let (w, h) = img.dimensions();
    let tmp = std::env::temp_dir().join(format!("foldex_upscale_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let (inp, outp) = (tmp.join("in.png"), tmp.join("out.png"));
    let res = (|| -> Result<RgbaImage, String> {
        img.save(&inp).map_err(|e| e.to_string())?;
        p.set(0.0, "Hochskalieren …");
        let mut c = std::process::Command::new(&exe);
        crate::localllm::lib_path(&mut c, &exe);
        c.current_dir(exe.parent().unwrap_or(&tmp))
            .arg("-i")
            .arg(&inp)
            .arg("-o")
            .arg(&outp)
            .args(["-n", if drawing { "realesrgan-x4plus-anime" } else { "realesrgan-x4plus" }, "-s", "4", "-f", "png"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // kein Konsolenfenster
        }
        let mut child = c.spawn().map_err(|e| format!("Real-ESRGAN starten: {e}"))?;
        let mut last = String::new();
        if let Some(err) = child.stderr.take() {
            // Fortschritt kommt als „12.34%“ je Zeile
            for line in std::io::BufReader::new(err).split(b'\n').map_while(Result::ok) {
                let s = String::from_utf8_lossy(&line).trim().to_string();
                if let Some(v) = s.strip_suffix('%').and_then(|v| v.trim().replace(',', ".").parse::<f32>().ok()) {
                    p.set(v / 100.0, format!("Hochskalieren … {v:.0} %"));
                } else if !s.is_empty() {
                    last = s;
                }
                if p.check().is_err() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Abgebrochen".into());
                }
            }
        }
        let st = child.wait().map_err(|e| e.to_string())?;
        if !st.success() || !outp.is_file() {
            return Err(format!("Hochskalieren fehlgeschlagen (braucht eine Vulkan-fähige Grafikkarte){}", if last.is_empty() { String::new() } else { format!(": {last}") }));
        }
        let mut big = image::open(&outp).map_err(|e| e.to_string())?.to_rgba8();
        if factor != 4 {
            big = image::imageops::resize(&big, w * factor, h * factor, image::imageops::FilterType::Lanczos3);
        }
        Ok(big)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

// ───────────────────────── Bildoperationen mit Masken ─────────────────────────

/// Hintergrund durchsichtig: Deckkraft × Maske
pub fn remove_bg(img: &mut RgbaImage, mask: &GrayImage) {
    img.as_mut().par_chunks_mut(4).zip(mask.as_raw().par_iter()).for_each(|(p, m)| {
        p[3] = ((p[3] as u32 * *m as u32 + 127) / 255) as u8;
    });
}

/// Hintergrund weichzeichnen, Motiv bleibt scharf. Der Hintergrund wird ohne das Motiv gerechnet
/// (Farben mit 1 − Maske gewichtet), sonst entsteht ein Lichthof um die Kante.
pub fn blur_background(img: &mut RgbaImage, mask: &GrayImage, radius: u32) {
    let mut bg = img.clone();
    bg.as_mut().par_chunks_mut(4).zip(mask.as_raw().par_iter()).for_each(|(p, m)| {
        let k = 255 - *m as u32;
        for c in 0..3 {
            p[c] = ((p[c] as u32 * k + 127) / 255) as u8;
        }
        p[3] = k as u8;
    });
    crate::imgedit::box_blur(&mut bg, radius.max(1));
    // Maskenkante etwas weicher, damit der Übergang nicht ausgeschnitten wirkt
    let mut soft = image::DynamicImage::ImageLuma8(mask.clone()).to_rgba8();
    crate::imgedit::box_blur(&mut soft, (radius / 8).max(1));
    img.as_mut().par_chunks_mut(4).zip(bg.as_raw().par_chunks(4)).zip(soft.as_raw().par_chunks(4)).for_each(|((p, b), s)| {
        let m = s[0] as f32 / 255.0;
        let wgt = b[3] as f32 / 255.0;
        if wgt < 0.02 {
            return;
        }
        for c in 0..3 {
            let bgc = (b[c] as f32 / wgt).min(255.0);
            p[c] = (p[c] as f32 * m + bgc * (1.0 - m)).round() as u8;
        }
    });
}

/// Objekt entfernen: LaMa (1024er-Export) erfindet passenden Inhalt an der Stelle – Hintergrund, Struktur,
/// Licht. Gerechnet wird auf einem Ausschnitt um das Objekt mit genug Umgebung: passt er in die Modellgröße,
/// in voller Auflösung (nicht verkleinert, damit nichts verwischt), sonst gleichmäßig verkleinert. Eingesetzt
/// wird nur unter der erweiterten, weich auslaufenden Maske.
pub fn inpaint(img: &RgbaImage, mask: &GrayImage, p: &Progress) -> Result<RgbaImage, String> {
    ensure(&[Need::Inpaint], p)?;
    p.set(-1.0, "Objekt entfernen – Stelle neu erzeugen …");
    let (w, h) = img.dimensions();
    let Some((bx0, by0, bx1, by1)) = mask_bounds(mask) else { return Err("Nichts ausgewählt".into()) };
    let obj = (bx1 - bx0).max(by1 - by0);
    // Maske erweitern, damit Schatten, Kantenreste und Lichthöfe mit verschwinden
    let grow = (obj / 25).max(w.max(h) / 300).clamp(4, 48);
    let mut big = image::DynamicImage::ImageLuma8(mask.clone()).to_rgba8();
    crate::imgedit::box_blur(&mut big, grow);
    let hole = GrayImage::from_fn(w, h, |x, y| Luma([if big.get_pixel(x, y)[0] > 4 || mask.get_pixel(x, y)[0] > 100 { 255 } else { 0 }]));
    let (x0, y0, x1, y1) = mask_bounds(&hole).unwrap_or((bx0, by0, bx1, by1));
    let s = with_session(&LAMA, |s| Ok(in_shape(s, 0).get(2).copied().filter(|v| *v > 0).unwrap_or(512) as u32))?;
    // Ausschnitt: Objekt plus rundherum etwa eine Objektbreite Umgebung, mindestens die Modellgröße
    let side = ((x1 - x0).max(y1 - y0) * 2 + 2 * grow).max(s);
    let (cw, ch) = (side.min(w), side.min(h));
    let (cx, cy) = ((x0 + x1) / 2, (y0 + y1) / 2);
    let sx = cx.saturating_sub(cw / 2).min(w - cw);
    let sy = cy.saturating_sub(ch / 2).min(h - ch);
    let crop = image::imageops::crop_imm(img, sx, sy, cw, ch).to_image();
    let cmask = image::imageops::crop_imm(&hole, sx, sy, cw, ch).to_image();
    // gleichmäßig auf die Modellgröße (nie vergrößern), Rest gespiegelt auffüllen – nichts wird verzerrt
    let k = (s as f32 / cw.max(ch) as f32).min(1.0);
    let (iw, ih) = (((cw as f32 * k).round() as u32).clamp(1, s), ((ch as f32 * k).round() as u32).clamp(1, s));
    let small = if k < 1.0 { image::imageops::resize(&crop, iw, ih, image::imageops::FilterType::Lanczos3) } else { crop.clone() };
    let smask = if k < 1.0 { image::imageops::resize(&cmask, iw, ih, image::imageops::FilterType::Triangle) } else { cmask.clone() };
    let n = (s * s) as usize;
    let mut xi = vec![0f32; 3 * n];
    let mut xm = vec![0f32; n];
    let mirror = |v: u32, len: u32| -> u32 {
        let period = 2 * len.max(1);
        let m = v % period;
        if m < len { m } else { period - 1 - m }
    };
    for y in 0..s {
        for x in 0..s {
            let (mx, my) = (mirror(x, iw), mirror(y, ih));
            let i = (y * s + x) as usize;
            let hole = smask.get_pixel(mx, my)[0] > 0;
            // der gespiegelte Rand gilt als bekannt, das Loch (auch gespiegelt) ist leer
            xm[i] = if hole { 1.0 } else { 0.0 };
            let px = small.get_pixel(mx, my);
            for c in 0..3 {
                xi[c * n + i] = if hole { 0.0 } else { px[c] as f32 / 255.0 };
            }
        }
    }
    let out = with_session(&LAMA, |ses| {
        let mut ins: Vec<(std::borrow::Cow<'static, str>, ort::session::SessionInputValue<'static>)> = Vec::new();
        for (idx, i) in ses.inputs.iter().enumerate() {
            let is_mask = i.name.contains("mask") || (idx == 1 && !i.name.contains("image"));
            let v: ort::value::DynValue = if is_mask {
                Tensor::from_array(([1usize, 1, s as usize, s as usize], xm.clone())).map_err(oe)?.into_dyn()
            } else {
                Tensor::from_array(([1usize, 3, s as usize, s as usize], xi.clone())).map_err(oe)?.into_dyn()
            };
            ins.push((i.name.clone().into(), v.into()));
        }
        let o = ses.run(ins).map_err(oe)?;
        let (_, d) = o[0].try_extract_tensor::<f32>().map_err(oe)?;
        if d.len() < 3 * n {
            return Err("KI: unerwartete Ausgabe".into());
        }
        Ok(d[..3 * n].to_vec())
    })?;
    p.check()?;
    // manche Exporte liefern 0–1, andere 0–255
    let scale = if out.iter().fold(0f32, |a, v| a.max(*v)) <= 1.5 { 255.0 } else { 1.0 };
    let gen = RgbaImage::from_fn(iw, ih, |x, y| {
        let i = (y * s + x) as usize;
        Rgba([(out[i] * scale).clamp(0.0, 255.0) as u8, (out[n + i] * scale).clamp(0.0, 255.0) as u8, (out[2 * n + i] * scale).clamp(0.0, 255.0) as u8, 255])
    });
    let gen = if k < 1.0 { image::imageops::resize(&gen, cw, ch, image::imageops::FilterType::Lanczos3) } else { gen };
    // weicher Übergang am Rand des Lochs: innen voll, nach außen auslaufend
    let mut soft = image::DynamicImage::ImageLuma8(cmask).to_rgba8();
    crate::imgedit::box_blur(&mut soft, (grow / 3).max(1));
    let mut res = img.clone();
    for y in 0..ch {
        for x in 0..cw {
            if hole.get_pixel(sx + x, sy + y)[0] == 0 {
                continue;
            }
            let a = (soft.get_pixel(x, y)[0] as f32 / 255.0 * 2.0).min(1.0);
            let (g, o) = (gen.get_pixel(x, y), res.get_pixel_mut(sx + x, sy + y));
            for c in 0..3 {
                o[c] = (g[c] as f32 * a + o[c] as f32 * (1.0 - a)).round() as u8;
            }
            o[3] = 255;
        }
    }
    Ok(res)
}

/// Begrenzungsrechteck der Maske (Werte > 127): x0, y0, x1, y1 (exklusiv)
pub fn mask_bounds(mask: &GrayImage) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in mask.enumerate_pixels() {
        if p[0] > 127 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

// ───────────────────────── Kette ─────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bg {
    Keep,
    Remove,
    Blur,
}

/// mehrere KI-Schritte nacheinander, als ein Rückgängig-Schritt
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Chain {
    pub colorize: bool,
    pub bg: Bg,
    /// Unschärfe des Hintergrunds, 1–40
    pub blur: f32,
    /// 1 = aus, 2 oder 4
    pub upscale: u32,
    /// Hochskalieren für Zeichnungen/Anime statt Fotos
    pub drawing: bool,
}

impl Default for Chain {
    fn default() -> Self {
        Chain { colorize: false, bg: Bg::Keep, blur: 12.0, upscale: 1, drawing: false }
    }
}

impl Chain {
    pub fn needs(&self, has_mask: bool) -> Vec<Need> {
        let mut v = Vec::new();
        if self.colorize {
            v.push(Need::Colorize);
        }
        if self.bg != Bg::Keep && !has_mask {
            v.push(Need::Subject);
        }
        if self.upscale > 1 {
            v.push(Need::Upscale);
        }
        v
    }
    pub fn is_empty(&self) -> bool {
        !self.colorize && self.bg == Bg::Keep && self.upscale <= 1
    }
    pub fn label(&self) -> String {
        let mut v = Vec::new();
        if self.colorize {
            v.push("Kolorieren".to_string());
        }
        match self.bg {
            Bg::Remove => v.push("Hintergrund entfernen".into()),
            Bg::Blur => v.push("Hintergrund unscharf".into()),
            Bg::Keep => {}
        }
        if self.upscale > 1 {
            v.push(format!("{}× hochskalieren", self.upscale));
        }
        v.join(" + ")
    }
    pub fn blur_radius(&self, w: u32, h: u32) -> u32 {
        ((self.blur * w.max(h) as f32 / 1000.0).round() as u32).max(1)
    }
}

/// Kette ausführen: Kolorieren → Hintergrund (mit vorhandener Objektmaske oder automatisch) → Hochskalieren
pub fn run_chain(mut img: RgbaImage, mask: Option<GrayImage>, c: &Chain, p: &Progress) -> Result<RgbaImage, String> {
    ensure(&c.needs(mask.is_some()), p)?;
    if c.colorize {
        img = colorize(&img, p)?;
        p.check()?;
    }
    if c.bg != Bg::Keep {
        let m = match mask {
            Some(m) if m.dimensions() == img.dimensions() => m,
            _ => subject_mask(&img, p)?,
        };
        p.check()?;
        p.set(-1.0, "Hintergrund bearbeiten …");
        match c.bg {
            Bg::Remove => remove_bg(&mut img, &m),
            Bg::Blur => {
                let r = c.blur_radius(img.width(), img.height());
                blur_background(&mut img, &m, r);
            }
            Bg::Keep => {}
        }
    }
    if c.upscale > 1 {
        img = upscale(&img, c.upscale, c.drawing, p)?;
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lab_roundtrip() {
        for c in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.8, 0.2, 0.1], [0.1, 0.5, 0.9], [0.5, 0.5, 0.5]] {
            let back = lab_to_rgb(rgb_to_lab(c));
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 0.01, "{c:?} → {back:?}");
            }
        }
        // Grau hat keine Farbe
        let g = rgb_to_lab([0.5, 0.5, 0.5]);
        assert!(g[1].abs() < 0.5 && g[2].abs() < 0.5);
    }

    #[test]
    fn merge_keeps_brightness() {
        let orig = RgbaImage::from_pixel(4, 4, Rgba([100, 100, 100, 255]));
        let col = RgbaImage::from_pixel(4, 4, Rgba([200, 60, 40, 255]));
        let m = merge_color(&orig, &col);
        let p = m.get_pixel(1, 1);
        assert!(p[0] > p[2], "rötlich: {p:?}");
        let l0 = rgb_to_lab([100.0 / 255.0; 3])[0];
        let l1 = rgb_to_lab([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0])[0];
        assert!((l0 - l1).abs() < 3.0, "{l0} vs {l1}");
    }

    #[test]
    fn clean_mask_removes_specks_and_holes() {
        // großes Objekt mit Loch, dazu ein Sprenkel ohne Klick
        let mut m = GrayImage::from_fn(100, 100, |x, y| Luma([if (20..80).contains(&x) && (20..80).contains(&y) && !((48..51).contains(&x) && (48..51).contains(&y)) { 255 } else { 0 }]));
        m.put_pixel(5, 5, Luma([255]));
        m.put_pixel(6, 5, Luma([255]));
        clean_mask(&mut m, &[(30, 30)]);
        assert_eq!(m.get_pixel(5, 5)[0], 0, "Sprenkel weg");
        assert_eq!(m.get_pixel(49, 49)[0], 255, "Loch gefüllt");
        assert_eq!(m.get_pixel(30, 30)[0], 255);
        assert_eq!(m.get_pixel(90, 90)[0], 0, "Hintergrund bleibt");
    }

    #[test]
    fn remove_and_blur_respect_mask() {
        let mut img = RgbaImage::from_fn(40, 40, |x, _| if x < 20 { Rgba([255, 0, 0, 255]) } else { Rgba([0, 0, 255, 255]) });
        let mask = GrayImage::from_fn(40, 40, |x, _| Luma([if x < 20 { 255 } else { 0 }]));
        let mut b = img.clone();
        blur_background(&mut b, &mask, 4);
        // Motiv (links) unverändert, Hintergrund ohne Rot aus dem Motiv
        assert_eq!(b.get_pixel(5, 20)[0], 255);
        assert!(b.get_pixel(30, 20)[0] < 10, "{:?}", b.get_pixel(30, 20));
        remove_bg(&mut img, &mask);
        assert_eq!(img.get_pixel(5, 5)[3], 255);
        assert_eq!(img.get_pixel(35, 5)[3], 0);
        assert_eq!(mask_bounds(&mask), Some((0, 0, 20, 40)));
    }

    /// mit echten, installierten Modellen: `FOLDEX_AI_TEST=<bild>` cargo test real_models -- --ignored
    /// (Ergebnisse landen neben dem Bild als *_ki_*.png)
    #[test]
    #[ignore]
    fn real_models() {
        let Ok(src) = std::env::var("FOLDEX_AI_TEST") else { return };
        let img = image::open(&src).unwrap().to_rgba8();
        let out = |n: &str, i: &RgbaImage| i.save(format!("{src}_ki_{n}.png")).unwrap();
        let p = Progress::default();
        if have(&ISNET) {
            let t = std::time::Instant::now();
            let m = subject_mask(&img, &p).unwrap();
            eprintln!("ISNet {:?}, Motiv {:?}", t.elapsed(), mask_bounds(&m));
            let mut b = img.clone();
            blur_background(&mut b, &m, 12);
            out("unscharf", &b);
        }
        if have(&DEOLDIFY) {
            let t = std::time::Instant::now();
            let c = colorize(&img, &p).unwrap();
            eprintln!("DeOldify {:?}", t.elapsed());
            assert_eq!(c.dimensions(), img.dimensions());
            out("koloriert", &c);
        }
        if have(&SAM_ENC) && have(&SAM_DEC) {
            let t = std::time::Instant::now();
            let e = sam_embed(&img, &p).unwrap();
            eprintln!("SAM kodieren {:?}", t.elapsed());
            let (w, h) = img.dimensions();
            let t = std::time::Instant::now();
            let (m1, low) = sam_mask(&e, &[(w as f32 * 0.5, h as f32 * 0.5, true)], None).unwrap();
            let (m2, _) = sam_mask(&e, &[(w as f32 * 0.5, h as f32 * 0.5, true), (w as f32 * 0.36, h as f32 * 0.5, true)], Some(&low)).unwrap();
            eprintln!("SAM 2 Klicks {:?}, {:?} → {:?}", t.elapsed(), mask_bounds(&m1), mask_bounds(&m2));
            out("objekt", &image::DynamicImage::ImageLuma8(m2).to_rgba8());
        }
        if have(&LAMA) && have(&ISNET) {
            let m = subject_mask(&img, &p).unwrap();
            let t = std::time::Instant::now();
            out("entfernt", &inpaint(&img, &m, &p).unwrap());
            eprintln!("LaMa {:?}", t.elapsed());
        }
        if have(&ESRGAN) {
            let small = image::imageops::resize(&img, 200, 200 * img.height() / img.width(), image::imageops::FilterType::Triangle);
            match upscale(&small, 2, false, &p) {
                Ok(u) => {
                    eprintln!("Real-ESRGAN {:?}", u.dimensions());
                    out("hoch", &u);
                }
                Err(e) => eprintln!("Real-ESRGAN: {e}"),
            }
        }
    }

    #[test]
    fn chain_needs() {
        let c = Chain { colorize: true, bg: Bg::Blur, upscale: 2, ..Default::default() };
        assert_eq!(c.needs(false), vec![Need::Colorize, Need::Subject, Need::Upscale]);
        assert_eq!(c.needs(true), vec![Need::Colorize, Need::Upscale]);
        assert!(Chain::default().is_empty());
        // ORT nur einmal, auch wenn mehrere Schritte es brauchen
        let m = missing(&[Need::Subject, Need::Colorize]);
        assert!(m.iter().filter(|a| a.name == ORT.name).count() <= 1);
    }
}
