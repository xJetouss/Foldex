//! Lokale Modelle direkt in Foldex – statt LM Studio. Text- und Bildmodelle lassen sich hier aussuchen,
//! herunterladen (mit Fortsetzen nach Abbruch) und ausführen, alles auf diesem PC:
//!
//! * Text: llama.cpp (`llama-server`, Vulkan = Grafikkarte) mit GGUF-Modellen von Hugging Face. Der Server
//!   spricht die OpenAI-Schnittstelle; ein lokales Modell ist daher ein ganz normales Modellprofil für
//!   Chat und Explorer-KI und wird beim ersten Gebrauch automatisch gestartet.
//! * Bilder erzeugen: stable-diffusion.cpp (`sd-cli`, Vulkan) mit SD-Turbo, SDXL-Turbo und Z-Image-Turbo.
//!
//! Neben den empfohlenen Modellen lässt sich Hugging Face nach beliebigen GGUF-Dateien durchsuchen.

use crate::localai::Progress;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ───────────────────────── Programme ─────────────────────────

pub struct Runtime {
    pub name: &'static str,
    url: &'static str,
    /// zweites Paket in denselben Ordner (CUDA-Laufzeit)
    extra: Option<&'static str>,
    folder: &'static str,
    /// Programmname(n), der erste gefundene gilt
    exe: &'static [&'static str],
    pub mb: u32,
}

pub static LLAMA: Runtime = Runtime {
    name: "llama.cpp (Textmodelle)",
    url: if cfg!(windows) { "https://github.com/ggml-org/llama.cpp/releases/download/b12110/llama-b12110-bin-win-vulkan-x64.zip" } else { "https://github.com/ggml-org/llama.cpp/releases/download/b12110/llama-b12110-bin-ubuntu-vulkan-x64.tar.gz" },
    extra: None,
    folder: "llama.cpp",
    exe: if cfg!(windows) { &["llama-server.exe"] } else { &["llama-server"] },
    mb: 37,
};

/// für NVIDIA-Karten: CUDA statt Vulkan – deutlich schneller und sieht die Karte auch, wenn Vulkan nur die
/// Prozessorgrafik findet
pub static LLAMA_CUDA: Runtime = Runtime {
    name: "llama.cpp CUDA (NVIDIA)",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b12110/llama-b12110-bin-win-cuda-13.4-x64.zip",
    extra: Some("https://github.com/ggml-org/llama.cpp/releases/download/b12110/cudart-llama-bin-win-cuda-13.4-x64.zip"),
    folder: "llama.cpp-cuda",
    exe: &["llama-server.exe"],
    mb: 322,
};

/// Bilder auf NVIDIA-Karten: CUDA statt Vulkan (Vulkan findet dort teils nur die Prozessorgrafik)
pub static SD_CUDA: Runtime = Runtime {
    name: "stable-diffusion.cpp CUDA (NVIDIA)",
    url: "https://github.com/leejet/stable-diffusion.cpp/releases/download/master-1047-984eb06/sd-master-984eb06-bin-win-cuda12-x64.zip",
    extra: Some("https://github.com/leejet/stable-diffusion.cpp/releases/download/master-1047-984eb06/cudart-sd-bin-win-cu12-x64.zip"),
    folder: "sd.cpp-cuda",
    exe: &["sd-cli.exe", "sd.exe"],
    mb: 925,
};

/// Programm für Bilder: CUDA, wenn installiert, sonst Vulkan
pub fn sd_exe() -> Option<PathBuf> {
    exe(&SD_CUDA).or_else(|| exe(&SD))
}

/// das passende Bild-Paket für diesen PC
pub fn sd_runtime() -> &'static Runtime {
    if cfg!(windows) && has_nvidia() {
        &SD_CUDA
    } else {
        &SD
    }
}

/// NVIDIA-Grafikkarte vorhanden? (nvidia-smi kommt mit dem Treiber)
pub fn has_nvidia() -> bool {
    static N: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        let mut c = Command::new("nvidia-smi");
        c.arg("-L").stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        c.output().is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("GPU"))
    })
}

/// Programm für Textmodelle: CUDA, wenn installiert, sonst Vulkan
pub fn llama_exe() -> Option<PathBuf> {
    exe(&LLAMA_CUDA).or_else(|| exe(&LLAMA))
}

/// das passende Paket für diesen PC
pub fn llama_runtime() -> &'static Runtime {
    if cfg!(windows) && has_nvidia() {
        &LLAMA_CUDA
    } else {
        &LLAMA
    }
}

pub static SD: Runtime = Runtime {
    name: "stable-diffusion.cpp (Bilder erzeugen)",
    url: if cfg!(windows) { "https://github.com/leejet/stable-diffusion.cpp/releases/download/master-1047-984eb06/sd-master-984eb06-bin-win-vulkan-x64.zip" } else { "https://github.com/leejet/stable-diffusion.cpp/releases/download/master-1047-984eb06/sd-master-984eb06-bin-Linux-Ubuntu-26.04-x86_64-vulkan.zip" },
    extra: None,
    folder: "sd.cpp",
    exe: if cfg!(windows) { &["sd-cli.exe", "sd.exe"] } else { &["sd-cli", "sd"] },
    mb: 30,
};

/// Cloud-Konten ohne Programm des Anbieters (Anmelden und als Laufwerk einbinden)
pub static RCLONE: Runtime = Runtime {
    name: "rclone (Cloud-Speicher)",
    url: if cfg!(windows) { "https://downloads.rclone.org/rclone-current-windows-amd64.zip" } else { "https://downloads.rclone.org/rclone-current-linux-amd64.zip" },
    extra: None,
    folder: "rclone",
    exe: if cfg!(windows) { &["rclone.exe"] } else { &["rclone"] },
    mb: 25,
};

/// Ziel des Downloads eines Programmpakets (für den Fortschritt)
pub fn runtime_zip(r: &Runtime) -> PathBuf {
    root().join(format!("{}.{}", r.folder, if r.url.ends_with(".tar.gz") || r.url.ends_with(".tgz") { "tar.gz" } else { "zip" }))
}

pub fn root() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("models")
}
fn text_dir() -> PathBuf {
    root().join("text")
}
fn image_dir() -> PathBuf {
    root().join("image")
}

/// Programm finden (auch in einem Unterordner des Pakets)
pub fn exe(r: &Runtime) -> Option<PathBuf> {
    let e = raw_exe(r)?;
    // CUDA-Pakete bestehen aus zwei Teilen: ohne Laufzeit (cudart*.dll) startet das Programm nicht
    if r.extra.is_some() && !has_cudart(e.parent()?) {
        return None;
    }
    Some(e)
}

/// Programmdatei, auch wenn das Paket noch unvollständig ist
fn raw_exe(r: &Runtime) -> Option<PathBuf> {
    // bis zu drei Ebenen tief (Linux-Pakete liegen z. B. in build/bin/)
    let mut dirs = vec![root().join(r.folder)];
    let mut i = 0;
    while i < dirs.len() && dirs.len() < 200 {
        let depth = dirs[i].strip_prefix(root().join(r.folder)).map_or(0, |p| p.components().count());
        if depth < 3 {
            if let Ok(rd) = std::fs::read_dir(&dirs[i]) {
                let subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
                dirs.extend(subs);
            }
        }
        i += 1;
    }
    dirs.iter().flat_map(|d| r.exe.iter().map(move |e| d.join(e))).find(|p| p.is_file())
}

/// Linux: mitgelieferte Bibliotheken (.so) liegen neben dem Programm – dort suchen lassen
pub fn lib_path(c: &mut Command, exe: &Path) {
    #[cfg(unix)]
    if let Some(dir) = exe.parent() {
        let old = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        c.env("LD_LIBRARY_PATH", if old.is_empty() { dir.display().to_string() } else { format!("{}:{old}", dir.display()) });
    }
    #[cfg(not(unix))]
    let _ = (c, exe);
}

fn has_cudart(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.starts_with("cudart") && n.ends_with(".dll")
        })
    })
}

/// angefangen, aber nicht fertig (z. B. Laufzeit-Download abgebrochen)?
pub fn incomplete(r: &Runtime) -> bool {
    raw_exe(r).is_some() && exe(r).is_none()
}

pub fn supported() -> bool {
    cfg!(all(windows, target_arch = "x86_64"))
}

// ───────────────────────── Katalog ─────────────────────────

/// eine Datei auf Hugging Face
#[derive(Clone, Copy)]
pub struct HfFile {
    pub repo: &'static str,
    pub file: &'static str,
    pub mb: u32,
}

impl HfFile {
    fn url(&self) -> String {
        hf_url(self.repo, self.file)
    }
}

fn hf_url(repo: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/main/{file}")
}

pub struct TextModel {
    pub name: &'static str,
    pub desc: &'static str,
    pub file: HfFile,
    /// Bild-Projektor (mmproj) in MB – 0 = kann keine Bilder ansehen
    pub vision_mb: u32,
}

/// empfohlene Textmodelle (Q4_K_M: guter Kompromiss aus Größe und Qualität)
pub static TEXT: &[TextModel] = &[
    TextModel { name: "Qwen3.5 0.8B", desc: "Winzig – läuft auf jedem PC, für einfache Aufgaben", file: HfFile { repo: "unsloth/Qwen3.5-0.8B-GGUF", file: "Qwen3.5-0.8B-Q4_K_M.gguf", mb: 532 }, vision_mb: 207 },
    TextModel { name: "Qwen3.5 2B", desc: "Klein und schnell, auch ohne Grafikkarte brauchbar", file: HfFile { repo: "unsloth/Qwen3.5-2B-GGUF", file: "Qwen3.5-2B-Q4_K_M.gguf", mb: 1280 }, vision_mb: 671 },
    TextModel { name: "Gemma 4 E2B", desc: "Google, klein, viele Sprachen", file: HfFile { repo: "unsloth/gemma-4-E2B-it-GGUF", file: "gemma-4-E2B-it-Q4_K_M.gguf", mb: 3106 }, vision_mb: 986 },
    TextModel { name: "Qwen3.5 4B", desc: "Empfohlen: klein, stark, gut auf Deutsch, kann Werkzeuge", file: HfFile { repo: "unsloth/Qwen3.5-4B-GGUF", file: "Qwen3.5-4B-Q4_K_M.gguf", mb: 2740 }, vision_mb: 675 },
    TextModel { name: "Gemma 4 E4B", desc: "Google, ausgewogen, sehr gut beim Schreiben", file: HfFile { repo: "unsloth/gemma-4-E4B-it-GGUF", file: "gemma-4-E4B-it-Q4_K_M.gguf", mb: 4977 }, vision_mb: 991 },
    TextModel { name: "Qwen3.5 9B", desc: "Deutlich klüger, ca. 8 GB Grafikspeicher", file: HfFile { repo: "unsloth/Qwen3.5-9B-GGUF", file: "Qwen3.5-9B-Q4_K_M.gguf", mb: 5680 }, vision_mb: 921 },
    TextModel { name: "Gemma 4 12B", desc: "Stark, ca. 10 GB Grafikspeicher", file: HfFile { repo: "unsloth/gemma-4-12b-it-GGUF", file: "gemma-4-12b-it-Q4_K_M.gguf", mb: 7121 }, vision_mb: 175 },
    TextModel { name: "gpt-oss 20B", desc: "OpenAI, denkt mit, 16 GB Grafikspeicher oder viel RAM", file: HfFile { repo: "unsloth/gpt-oss-20b-GGUF", file: "gpt-oss-20b-Q4_K_M.gguf", mb: 11624 }, vision_mb: 0 },
    TextModel { name: "Gemma 4 26B-A4B", desc: "Sehr gut und trotz Größe flott (MoE), 20 GB+", file: HfFile { repo: "unsloth/gemma-4-26B-A4B-it-GGUF", file: "gemma-4-26B-A4B-it-UD-Q4_K_M.gguf", mb: 16947 }, vision_mb: 1194 },
    TextModel { name: "Qwen3.6 27B", desc: "Spitzenklasse für starke Grafikkarten (24 GB)", file: HfFile { repo: "unsloth/Qwen3.6-27B-GGUF", file: "Qwen3.6-27B-Q4_K_M.gguf", mb: 16817 }, vision_mb: 931 },
    TextModel { name: "Qwen3.6 35B-A3B", desc: "Sehr stark, schnell durch MoE – viel RAM/Grafikspeicher", file: HfFile { repo: "unsloth/Qwen3.6-35B-A3B-GGUF", file: "Qwen3.6-35B-A3B-UD-Q4_K_M.gguf", mb: 22134 }, vision_mb: 902 },
    TextModel { name: "Qwen3 Coder 30B-A3B", desc: "Programmieren; schnell trotz Größe (MoE), 24 GB+", file: HfFile { repo: "unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF", file: "Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf", mb: 18556 }, vision_mb: 0 },
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    /// vollständiges Modell (`-m`)
    Model,
    Diffusion,
    Vae,
    /// Text-Kodierer als Sprachmodell (`--llm`)
    Llm,
    ClipL,
    ClipG,
    T5xxl,
}

impl Role {
    fn flag(self) -> &'static str {
        match self {
            Role::Model => "-m",
            Role::Diffusion => "--diffusion-model",
            Role::Vae => "--vae",
            Role::Llm => "--llm",
            Role::ClipL => "--clip_l",
            Role::ClipG => "--clip_g",
            Role::T5xxl => "--t5xxl",
        }
    }
}

pub struct ImageModel {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    files: &'static [(Role, HfFile)],
    pub size: u32,
    pub steps: u32,
    pub cfg: f32,
    /// negativer Prompt sinnvoll?
    pub negative: bool,
    extra: &'static [&'static str],
}

impl ImageModel {
    /// Größe aller Dateien (MB)
    pub fn files_mb(&self) -> u32 {
        self.files.iter().map(|f| f.1.mb).sum()
    }
    /// größte Einzeldatei (bestimmt, ob es in den Grafikspeicher passt)
    pub fn largest_mb(&self) -> u32 {
        self.files.iter().map(|f| f.1.mb).max().unwrap_or(0)
    }
}

const QWEN3_4B: HfFile = HfFile { repo: "unsloth/Qwen3-4B-Instruct-2507-GGUF", file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf", mb: 2497 };
/// Bild-Dekodierer von FLUX/Z-Image (dieselbe Datei für beide)
const FLUX_AE: HfFile = HfFile { repo: "Comfy-Org/z_image_turbo", file: "split_files/vae/ae.safetensors", mb: 335 };
/// SDXL-Dekodierer ohne schwarze Bilder bei halber Genauigkeit
const SDXL_VAE: HfFile = HfFile { repo: "madebyollin/sdxl-vae-fp16-fix", file: "sdxl_vae.safetensors", mb: 334 };

/// empfohlene Bildmodelle, nach Größe
pub static IMAGE: &[ImageModel] = &[
    ImageModel {
        id: "sd-turbo",
        name: "SD-Turbo",
        desc: "Sehr schnell (1–4 Schritte, 512 px) – auch ohne starke Grafikkarte",
        files: &[(Role::Model, HfFile { repo: "Green-Sky/SD-Turbo-GGUF", file: "sd_turbo-f16-q8_0.gguf", mb: 2023 })],
        size: 512,
        steps: 4,
        cfg: 1.0,
        negative: false,
        extra: &[],
    },
    ImageModel {
        id: "sd15",
        name: "Stable Diffusion 1.5",
        desc: "Der Klassiker (512 px) – vielseitig, schnell",
        files: &[(Role::Model, HfFile { repo: "Comfy-Org/stable-diffusion-v1-5-archive", file: "v1-5-pruned-emaonly-fp16.safetensors", mb: 2132 })],
        size: 512,
        steps: 25,
        cfg: 7.0,
        negative: true,
        extra: &[],
    },
    ImageModel {
        id: "dreamshaper8",
        name: "DreamShaper 8",
        desc: "Illustrationen, Fantasy und Fotos (512 px) – sehr beliebt",
        files: &[(Role::Model, HfFile { repo: "Lykon/DreamShaper", file: "DreamShaper_8_pruned.safetensors", mb: 2132 })],
        size: 512,
        steps: 25,
        cfg: 7.0,
        negative: true,
        extra: &[],
    },
    ImageModel {
        id: "realvis6",
        name: "Realistic Vision 6",
        desc: "Fotorealistische Menschen und Szenen (512 px)",
        files: &[(Role::Model, HfFile { repo: "SG161222/Realistic_Vision_V6.0_B1_noVAE", file: "Realistic_Vision_V6.0_NV_B1_fp16.safetensors", mb: 2132 })],
        size: 512,
        steps: 25,
        cfg: 6.0,
        negative: true,
        extra: &[],
    },
    ImageModel {
        id: "sdxl-turbo",
        name: "SDXL-Turbo",
        desc: "Schnell, schönere Bilder als SD-Turbo (512 px)",
        files: &[(Role::Model, HfFile { repo: "OlegSkutte/sdxl-turbo-GGUF", file: "sd_xl_turbo_1.0.q8_0.gguf", mb: 4098 })],
        size: 512,
        steps: 4,
        cfg: 1.0,
        negative: false,
        extra: &[],
    },
    ImageModel {
        id: "flux2-klein",
        name: "FLUX.2 klein 4B",
        desc: "Klein und sehr schnell (4 Schritte, 1024 px), kann auch Bilder bearbeiten",
        files: &[(Role::Diffusion, HfFile { repo: "leejet/FLUX.2-klein-4B-GGUF", file: "flux-2-klein-4b-Q4_0.gguf", mb: 2460 }), (Role::Vae, FLUX2_VAE), (Role::Llm, QWEN3_4B_BASE)],
        size: 1024,
        steps: 4,
        cfg: 1.0,
        negative: false,
        extra: &["--offload-to-cpu", "--diffusion-fa", "--sampling-method", "euler"],
    },
    ImageModel {
        id: "z-image-turbo",
        name: "Z-Image-Turbo",
        desc: "Beste Qualität, kann Text im Bild (1024 px, 8 Schritte) – nutzt Qwen3 4B mit",
        files: &[(Role::Diffusion, HfFile { repo: "leejet/Z-Image-Turbo-GGUF", file: "z_image_turbo-Q4_K.gguf", mb: 3864 }), (Role::Vae, FLUX_AE), (Role::Llm, QWEN3_4B)],
        size: 1024,
        steps: 8,
        cfg: 1.0,
        negative: false,
        extra: &["--offload-to-cpu", "--diffusion-fa"],
    },
    ImageModel {
        id: "sdxl-lightning",
        name: "SDXL-Lightning (4 Schritte)",
        desc: "SDXL-Qualität in 4 Schritten (1024 px)",
        files: &[(Role::Model, HfFile { repo: "ByteDance/SDXL-Lightning", file: "sdxl_lightning_4step.safetensors", mb: 6938 }), (Role::Vae, SDXL_VAE)],
        size: 1024,
        steps: 4,
        cfg: 1.0,
        negative: false,
        extra: &["--sampling-method", "euler"],
    },
    ImageModel {
        id: "dreamshaper-xl",
        name: "DreamShaper XL Lightning",
        desc: "Vielseitig und schnell in hoher Auflösung (1024 px, 6 Schritte)",
        files: &[(Role::Model, HfFile { repo: "Lykon/dreamshaper-xl-lightning", file: "DreamShaperXL_Lightning.safetensors", mb: 6939 }), (Role::Vae, SDXL_VAE)],
        size: 1024,
        steps: 6,
        cfg: 2.0,
        negative: true,
        extra: &["--sampling-method", "dpm++2m"],
    },
    ImageModel {
        id: "sdxl-base",
        name: "SDXL 1.0",
        desc: "Original von Stability AI (1024 px) – Grundlage vieler Stile",
        files: &[(Role::Model, HfFile { repo: "stabilityai/stable-diffusion-xl-base-1.0", file: "sd_xl_base_1.0.safetensors", mb: 6938 }), (Role::Vae, SDXL_VAE)],
        size: 1024,
        steps: 30,
        cfg: 7.0,
        negative: true,
        extra: &[],
    },
    ImageModel {
        id: "juggernaut-xl",
        name: "Juggernaut XL 9",
        desc: "Sehr gute Fotos und Porträts (1024 px)",
        files: &[(Role::Model, HfFile { repo: "RunDiffusion/Juggernaut-XL-v9", file: "Juggernaut-XL_v9_RunDiffusionPhoto_v2.safetensors", mb: 7105 }), (Role::Vae, SDXL_VAE)],
        size: 1024,
        steps: 30,
        cfg: 5.0,
        negative: true,
        extra: &[],
    },
    ImageModel {
        id: "qwen-image-turbo",
        name: "Qwen-Image 2.1 Turbo",
        desc: "Alibaba: sehr gute Bilder und Schrift im Bild, auch Chinesisch (1024 px, 8 Schritte) – groß",
        files: &[
            (Role::Diffusion, HfFile { repo: "unsloth/Qwen-Image-2.1-Turbo-GGUF", file: "qwen-image-2.1-turbo-Q4_K_M.gguf", mb: 4199 }),
            (Role::Vae, HfFile { repo: "Comfy-Org/Qwen-Image-2.1", file: "vae/qwen_image_2.1_vae_bf16.safetensors", mb: 675 }),
            (Role::Llm, HfFile { repo: "Qwen/Qwen3-VL-8B-Instruct-GGUF", file: "Qwen3VL-8B-Instruct-Q4_K_M.gguf", mb: 5027 }),
        ],
        size: 1024,
        steps: 8,
        cfg: 1.0,
        negative: false,
        extra: &["--offload-to-cpu", "--diffusion-fa", "--sigmas", "qwen-image-2.1-turbo", "--sampling-method", "euler"],
    },
    ImageModel {
        id: "flux-schnell",
        name: "FLUX.1 schnell",
        desc: "Spitzenqualität und gutes Textverständnis (1024 px, 4 Schritte)",
        files: &[
            (Role::Diffusion, HfFile { repo: "city96/FLUX.1-schnell-gguf", file: "flux1-schnell-Q4_K_S.gguf", mb: 6783 }),
            (Role::Vae, FLUX_AE),
            (Role::ClipL, HfFile { repo: "comfyanonymous/flux_text_encoders", file: "clip_l.safetensors", mb: 246 }),
            (Role::T5xxl, HfFile { repo: "city96/t5-v1_1-xxl-encoder-gguf", file: "t5-v1_1-xxl-encoder-Q5_K_M.gguf", mb: 3386 }),
        ],
        size: 1024,
        steps: 4,
        cfg: 1.0,
        negative: false,
        extra: &["--offload-to-cpu", "--diffusion-fa"],
    },
];

/// Ziel einer Hugging-Face-Datei: Sprachmodelle bei den Textmodellen (auch der Kodierer von Z-Image –
/// so wird Qwen3 4B nur einmal geladen), alles andere bei den Bildmodellen
fn local_path(repo_file: &str, text: bool) -> PathBuf {
    let name = repo_file.rsplit('/').next().unwrap_or(repo_file);
    if text {
        text_dir().join(name)
    } else {
        image_dir().join(name)
    }
}

fn image_file_path(role: Role, f: &HfFile) -> PathBuf {
    local_path(f.file, role == Role::Llm)
}

pub fn image_missing(m: &ImageModel) -> Vec<(String, PathBuf, String, u32)> {
    m.files
        .iter()
        .filter(|(r, f)| !image_file_path(*r, f).is_file())
        .map(|(r, f)| (format!("{} – {}", m.name, f.file.rsplit('/').next().unwrap_or(f.file)), image_file_path(*r, f), f.url(), f.mb))
        .collect()
}

pub fn image_installed(m: &ImageModel) -> bool {
    image_missing(m).is_empty()
}

pub fn text_path(m: &TextModel) -> PathBuf {
    local_path(m.file.file, true)
}

/// installierte Textmodelle: (Name ohne Endung, Pfad, Größe). Teile 2… geteilter Dateien zählen nicht extra.
pub fn installed_text() -> Vec<(String, PathBuf, u64)> {
    fn ggufs(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth > 0 {
                    ggufs(&p, depth - 1, out);
                }
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gguf")) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    ggufs(&text_dir(), 0, &mut files);
    // schon in LM Studio geladene Modelle direkt mitbenutzen (models/<Anbieter>/<Repo>/*.gguf)
    for d in lmstudio_dirs() {
        ggufs(&d, 2, &mut files);
    }
    let mut v: Vec<(String, PathBuf, u64)> = Vec::new();
    for p in files {
        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if n.contains("mmproj") || (n.contains("-of-0") && !n.contains("-00001-of-")) {
            continue;
        }
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        // gleiche Datei in beiden Ordnern: die eigene zählt
        if v.iter().any(|t| t.0 == stem) {
            continue;
        }
        let size = std::fs::metadata(&p).map_or(0, |m| m.len());
        v.push((stem, p, size));
    }
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    v
}

/// Modellordner von LM Studio (neu: ~/.lmstudio/models, alt: ~/.cache/lm-studio/models)
fn lmstudio_dirs() -> Vec<PathBuf> {
    let Some(h) = dirs::home_dir() else { return Vec::new() };
    [h.join(".lmstudio").join("models"), h.join(".cache").join("lm-studio").join("models")].into_iter().filter(|d| d.is_dir()).collect()
}

/// gehört die Datei Foldex (darf gelöscht werden) oder z. B. LM Studio?
pub fn is_own(p: &Path) -> bool {
    p.starts_with(text_dir())
}

pub fn delete_file(p: &Path) -> Result<(), String> {
    // ein laufender Server hält die Datei offen
    if server_model().is_some_and(|m| p.file_stem().is_some_and(|s| s.to_string_lossy() == m)) {
        stop_server();
    }
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// Bildmodell löschen – ohne das von Textmodellen mitbenutzte Sprachmodell
pub fn delete_image(m: &ImageModel) -> Result<(), String> {
    for (r, f) in m.files {
        let p = image_file_path(*r, f);
        // von anderen installierten Bildmodellen mitbenutzte Dateien (z. B. der SDXL-Dekodierer) bleiben
        let shared = IMAGE.iter().filter(|o| !std::ptr::eq(*o, m) && image_installed(o)).any(|o| o.files.iter().any(|(r2, f2)| image_file_path(*r2, f2) == p));
        if *r != Role::Llm && !shared {
            delete_file(&p)?;
        }
    }
    Ok(())
}

// ───────────────────────── Downloads (Warteschlange) ─────────────────────────

#[derive(Clone)]
struct Job {
    label: String,
    url: String,
    dest: PathBuf,
    /// Programmpaket: nach dem Laden in diesen Ordner entpacken
    unzip_to: Option<PathBuf>,
    /// Ordner vorher leeren (nicht beim Zusatzpaket)
    clean: bool,
    /// Zugangsschlüssel (Civitai)
    auth: Option<String>,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    cur: Option<(Job, Progress)>,
    log: Vec<(bool, String)>,
    worker: bool,
}

fn queue() -> &'static Mutex<Queue> {
    static Q: Mutex<Queue> = Mutex::new(Queue { jobs: VecDeque::new(), cur: None, log: Vec::new(), worker: false });
    &Q
}

fn enqueue(job: Job) {
    let Ok(mut q) = queue().lock() else { return };
    if q.jobs.iter().any(|j| j.dest == job.dest) || q.cur.as_ref().is_some_and(|c| c.0.dest == job.dest) {
        return;
    }
    q.jobs.push_back(job);
    if !q.worker {
        q.worker = true;
        std::thread::spawn(worker);
    }
}

fn worker() {
    loop {
        let job = {
            let Ok(mut q) = queue().lock() else { return };
            match q.jobs.pop_front() {
                Some(j) => {
                    let p = Progress::default();
                    q.cur = Some((j.clone(), p.clone()));
                    (j, p)
                }
                None => {
                    q.cur = None;
                    q.worker = false;
                    return;
                }
            }
        };
        let (j, p) = job;
        let r = run_job(&j, &p);
        if let Ok(mut q) = queue().lock() {
            q.cur = None;
            q.log.push(match r {
                Ok(()) => (false, format!("{}: fertig", j.label)),
                Err(e) => (true, format!("{}: {e}", j.label)),
            });
        }
    }
}

fn run_job(j: &Job, p: &Progress) -> Result<(), String> {
    let mut tries = 0;
    loop {
        match download(&j.url, &j.dest, &j.label, j.auth.as_deref(), p) {
            Ok(()) => break,
            Err(e) if p.check().is_ok() && tries < 4 && (e.starts_with("Verbindung") || e.starts_with("unvollständig") || e.contains("nicht erreichbar") || e.contains("fehlgeschlagen:")) => {
                tries += 1;
                p.set(-1.0, format!("{} – Verbindung unterbrochen, neuer Versuch {tries}/4 …", j.label));
                std::thread::sleep(Duration::from_secs(3 * tries));
            }
            Err(e) => return Err(e),
        }
    }
    if let Some(dir) = &j.unzip_to {
        p.set(-1.0, format!("{} entpacken …", j.label));
        if j.clean {
            let _ = std::fs::remove_dir_all(dir);
        }
        let d = dir.clone();
        crate::localai::unzip(&j.dest, |name| {
            let n = name.replace('\\', "/");
            (!n.split('/').any(|c| c == "..") && !n.starts_with('/')).then(|| d.join(&n))
        })?;
        let _ = std::fs::remove_file(&j.dest);
    }
    Ok(())
}

/// große Datei laden; eine angefangene `.part`-Datei wird fortgesetzt
fn download(url: &str, dest: &Path, label: &str, auth: Option<&str>, p: &Progress) -> Result<(), String> {
    if let Some(par) = dest.parent() {
        std::fs::create_dir_all(par).map_err(|e| format!("{}: {e}", par.display()))?;
    }
    let part = dest.with_extension(format!("{}.part", dest.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
    let have = std::fs::metadata(&part).map_or(0, |m| m.len());
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).user_agent("Foldex").build();
    let mut req = agent.get(url);
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
    }
    if let Some(k) = auth {
        req = req.set("Authorization", &format!("Bearer {k}"));
    }
    let resp = match req.call() {
        Ok(r) => r,
        // 416: angefangene Datei ist schon vollständig (z. B. Umbenennen beim letzten Mal gescheitert)
        Err(ureq::Error::Status(416, _)) if have > 0 => {
            return finish(&part, dest);
        }
        Err(ureq::Error::Status(401 | 403, _)) if auth.is_some() => return Err("Zugriff verweigert – Civitai-Schlüssel prüfen (oder das Modell verlangt eine Anmeldung auf civitai.com)".into()),
        Err(ureq::Error::Status(code, _)) => return Err(format!("Laden fehlgeschlagen (HTTP {code})")),
        Err(ureq::Error::Transport(t)) => return Err(format!("Laden fehlgeschlagen: {}", t.kind())),
    };
    let resumed = resp.status() == 206;
    let rest = resp.header("Content-Length").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let start = if resumed { have } else { 0 };
    let total = start + rest;
    let mut f = std::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(&part).map_err(|e| e.to_string())?;
    let mut rd = resp.into_reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = start;
    let t0 = Instant::now();
    loop {
        if p.check().is_err() {
            // .part bleibt liegen: beim nächsten Mal geht es dort weiter
            return Err("Abgebrochen (wird beim nächsten Mal fortgesetzt)".into());
        }
        let n = rd.read(&mut buf).map_err(|e| format!("Verbindung unterbrochen: {e}"))?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        let secs = t0.elapsed().as_secs_f64().max(0.1);
        let speed = (done - start) as f64 / secs;
        let eta = if speed > 0.0 && total > done { format!(" · noch {}", human_secs((total - done) as f64 / speed)) } else { String::new() };
        p.set(if total > 0 { done as f32 / total as f32 } else { -1.0 }, format!("{label} · {} / {} · {:.1} MB/s{eta}", mb(done), mb(total), speed / 1e6));
    }
    f.flush().map_err(|e| e.to_string())?;
    drop(f);
    if total > 0 && done < total {
        return Err("unvollständig – erneut versuchen setzt fort".into());
    }
    finish(&part, dest)
}

/// fertige `.part`-Datei an ihren Platz – Umbenennen kann scheitern (z. B. umgeleitete Ordner auf einem
/// anderen Laufwerk), dann kopieren
fn finish(part: &Path, dest: &Path) -> Result<(), String> {
    if std::fs::rename(part, dest).is_err() {
        std::fs::copy(part, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        let _ = std::fs::remove_file(part);
    }
    Ok(())
}

fn mb(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2} GB", n as f64 / 1e9)
    } else {
        format!("{} MB", n / 1_000_000)
    }
}

fn human_secs(s: f64) -> String {
    let s = s as u64;
    if s >= 3600 {
        format!("{} h {} min", s / 3600, s % 3600 / 60)
    } else if s >= 60 {
        format!("{} min", s / 60)
    } else {
        format!("{s} s")
    }
}

pub fn install_runtime(r: &'static Runtime) {
    let dir = root().join(r.folder);
    if raw_exe(r).is_none() {
        enqueue(Job { label: r.name.into(), url: r.url.into(), dest: runtime_zip(r), unzip_to: Some(dir.clone()), clean: true, auth: None });
    }
    if let Some(x) = r.extra {
        enqueue(Job { label: format!("{} – Laufzeit", r.name), url: x.into(), dest: root().join(format!("{}-rt.zip", r.folder)), unzip_to: Some(dir), clean: false, auth: None });
    }
}

pub fn install_text(m: &TextModel) {
    if llama_exe().is_none() {
        install_runtime(llama_runtime());
    }
    enqueue(Job { label: m.name.into(), url: m.file.url(), dest: text_path(m), unzip_to: None, clean: false, auth: None });
    // Bild-Projektor gleich mit – dann kann das Modell auch Bilder ansehen
    if m.vision_mb > 0 {
        enqueue(Job { label: format!("{} – Bilder verstehen", m.name), url: hf_url(m.file.repo, "mmproj-F16.gguf"), dest: own_mmproj(&text_path(m)), unzip_to: None, clean: false, auth: None });
    }
}

pub fn install_image(m: &ImageModel) {
    if sd_exe().is_none() {
        install_runtime(sd_runtime());
    }
    for (label, dest, url, _) in image_missing(m) {
        enqueue(Job { label, url, dest, unzip_to: None, clean: false, auth: None });
    }
}

/// beliebige GGUF-Datei aus einem Hugging-Face-Repo (geteilte Dateien: alle Teile)
pub fn install_hf(repo: &str, file: &str, all_files: &[String]) {
    if llama_exe().is_none() {
        install_runtime(llama_runtime());
    }
    let parts: Vec<String> = match split_parts(file) {
        Some(prefix) => all_files.iter().filter(|f| f.starts_with(&prefix) && f.contains("-of-")).cloned().collect(),
        None => vec![file.to_string()],
    };
    let first = parts.first().map(|f| local_path(f, true));
    for f in parts {
        let name = f.rsplit('/').next().unwrap_or(&f).to_string();
        enqueue(Job { label: name, url: hf_url(repo, &f), dest: local_path(&f, true), unzip_to: None, clean: false, auth: None });
    }
    // Bild-Projektor des Repos (F16 bevorzugt) – sonst sieht das Modell keine Bilder
    let mm: Vec<&String> = all_files.iter().filter(|f| f.to_lowercase().contains("mmproj")).collect();
    let pick = mm.iter().find(|f| f.ends_with("mmproj-F16.gguf")).or_else(|| mm.iter().find(|f| f.to_lowercase().contains("f16"))).or(mm.first());
    if let (Some(mf), Some(first)) = (pick, first) {
        enqueue(Job { label: format!("{} – Bilder verstehen", first.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()), url: hf_url(repo, mf), dest: own_mmproj(&first), unzip_to: None, clean: false, auth: None });
    }
}

/// Ablageort des Projektors zu einem eigenen Modell („Name-mmproj.gguf“ daneben)
fn own_mmproj(model: &Path) -> PathBuf {
    let stem = model.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    model.with_file_name(format!("{stem}-mmproj.gguf"))
}

/// Projektor eines installierten Modells: eigener („Name-mmproj.gguf“) oder bei LM-Studio-Modellen der im
/// selben Ordner
pub fn mmproj_for(model: &Path) -> Option<PathBuf> {
    let own = own_mmproj(model);
    if own.is_file() {
        return Some(own);
    }
    if is_own(model) {
        return None;
    }
    let dir = model.parent()?;
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains("mmproj"))).collect();
    v.sort();
    v.into_iter().next()
}

/// kann das installierte Modell Bilder ansehen?
pub fn has_vision(stem: &str) -> bool {
    installed_text().into_iter().find(|t| t.0 == stem).is_some_and(|t| mmproj_for(&t.1).is_some())
}

/// „…-00001-of-00003.gguf“ → „…-“ (gemeinsamer Anfang aller Teile)
fn split_parts(file: &str) -> Option<String> {
    let i = file.rfind("-00001-of-")?;
    Some(file[..=i].to_string())
}

/// läuft oder wartet ein Download für diese Datei?
pub fn queued(dest: &Path) -> Option<(f32, String)> {
    let q = queue().lock().ok()?;
    if let Some((j, p)) = &q.cur {
        if j.dest == dest {
            return Some(p.get());
        }
    }
    q.jobs.iter().any(|j| j.dest == dest).then(|| (-2.0, "Wartet …".to_string()))
}

/// alle Downloads: (Name, Anteil, Text); Anteil −2 = wartet
pub fn downloads() -> Vec<(String, f32, String)> {
    let Ok(q) = queue().lock() else { return Vec::new() };
    let mut v: Vec<(String, f32, String)> = q.cur.iter().map(|(j, p)| {
        let (f, t) = p.get();
        (j.label.clone(), f, t)
    }).collect();
    v.extend(q.jobs.iter().map(|j| (j.label.clone(), -2.0, "Wartet …".into())));
    v
}

pub fn cancel_downloads() {
    if let Ok(mut q) = queue().lock() {
        q.jobs.clear();
        if let Some((_, p)) = &q.cur {
            p.cancel();
        }
    }
}

/// Meldungen fertiger Downloads abholen
pub fn take_log() -> Vec<(bool, String)> {
    queue().lock().map(|mut q| std::mem::take(&mut q.log)).unwrap_or_default()
}

// ───────────────────────── Passt das Modell zu diesem PC? ─────────────────────────

/// Grafikkarte mit dem meisten eigenen Speicher und der Arbeitsspeicher
pub struct Pc {
    pub gpu: String,
    pub vram_mb: u64,
    pub ram_mb: u64,
}

pub fn pc() -> &'static Pc {
    static P: std::sync::OnceLock<Pc> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let (gpu, vram_mb) = gpus().into_iter().max_by_key(|g| g.1).unwrap_or_default();
        Pc { gpu, vram_mb, ram_mb: ram_mb() }
    })
}

/// Grafikkarten aus der Registry (Treiber trägt den echten Speicher als 64-Bit-Wert ein; WMI kappt bei 4 GB)
fn gpus() -> Vec<(String, u64)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::*;
        let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let mut out = Vec::new();
        for i in 0..16 {
            let key = w(&format!("SYSTEM\\CurrentControlSet\\Control\\Class\\{{4d36e968-e325-11ce-bfc1-08002be10318}}\\{i:04}"));
            let read = |name: &str, buf: &mut [u8]| -> Option<usize> {
                let mut len = buf.len() as u32;
                let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), w(name).as_ptr(), RRF_RT_ANY, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut len) };
                (r == 0).then_some(len as usize)
            };
            let mut b = [0u8; 512];
            let Some(n) = read("DriverDesc", &mut b) else { continue };
            let name = String::from_utf16_lossy(&b[..n].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect::<Vec<_>>());
            let mut m = [0u8; 8];
            let mem = match read("HardwareInformation.qwMemorySize", &mut m) {
                Some(8) => u64::from_le_bytes(m),
                _ => match read("HardwareInformation.MemorySize", &mut m) {
                    Some(4) => u32::from_le_bytes([m[0], m[1], m[2], m[3]]) as u64,
                    Some(8) => u64::from_le_bytes(m),
                    _ => 0,
                },
            };
            out.push((name, mem / 1_048_576));
        }
        out
    }
    #[cfg(not(windows))]
    Vec::new()
}

fn ram_mb() -> u64 {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::SystemInformation::*;
        let mut m: MEMORYSTATUSEX = std::mem::zeroed();
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) != 0 {
            return m.ullTotalPhys / 1_048_576;
        }
    }
    0
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// passt ganz in den Grafikspeicher – schnell
    Gpu,
    /// läuft, aber teils im Arbeitsspeicher – langsamer
    Slow,
    TooBig,
}

impl Fit {
    pub fn label(self) -> &'static str {
        match self {
            Fit::Gpu => "passt in die Grafikkarte",
            Fit::Slow => "läuft, aber langsamer",
            Fit::TooBig => "zu groß für diesen PC",
        }
    }
}

fn fit_with(need_gpu: u64, total: u64, pc: &Pc) -> Fit {
    if pc.vram_mb > 0 && need_gpu <= pc.vram_mb && total <= pc.vram_mb + pc.ram_mb / 2 {
        Fit::Gpu
    } else if total <= pc.vram_mb + pc.ram_mb * 6 / 10 {
        Fit::Slow
    } else {
        Fit::TooBig
    }
}

/// Textmodell: Datei plus etwa 1 GB für den Kontext
pub fn fit_text(mb: u64) -> Fit {
    fit_with(mb + 1000, mb + 1000, pc())
}

/// Bildmodell: die größte Datei muss in die Grafikkarte, der Rest darf ausgelagert werden
pub fn fit_image(largest_mb: u64, total_mb: u64) -> Fit {
    fit_with(largest_mb + 1500, total_mb + 1500, pc())
}

// ───────────────────────── Hugging Face: seitenweise ─────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HfKind {
    Text,
    Image,
}

#[derive(Clone, Debug, Default)]
pub struct HfRepo {
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
    /// Dateinamen im Repo
    pub files: Vec<String>,
    pub tags: Vec<String>,
    /// Aufgabe laut Hugging Face (text-generation, text-to-image …)
    pub pipeline: String,
}

/// eine Seite Suchergebnisse und die Adresse der nächsten
pub fn hf_page(kind: HfKind, query: &str, next: Option<&str>) -> Result<(Vec<HfRepo>, Option<String>), String> {
    let url = match next {
        Some(n) => n.to_string(),
        None => {
            let filter = match kind {
                HfKind::Text => "filter=gguf",
                HfKind::Image => "pipeline_tag=text-to-image",
            };
            let q = if query.trim().is_empty() { String::new() } else { format!("&search={}", urlenc(query.trim())) };
            format!("https://huggingface.co/api/models?{filter}{q}&sort=downloads&direction=-1&limit=30&expand[]=siblings&expand[]=downloads&expand[]=likes&expand[]=tags&expand[]=pipeline_tag")
        }
    };
    let resp = ureq::get(&url).timeout(Duration::from_secs(20)).call().map_err(|e| match e {
        ureq::Error::Status(c, _) => format!("Hugging Face: HTTP {c}"),
        ureq::Error::Transport(t) => format!("Hugging Face nicht erreichbar: {}", t.kind()),
    })?;
    // Link: <…>; rel="next"
    let next = resp.header("Link").and_then(|l| l.split(',').find(|p| p.contains("rel=\"next\"")).and_then(|p| Some(p[p.find('<')? + 1..p.find('>')?].to_string())));
    let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let repos = v
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(HfRepo {
                id: m["id"].as_str()?.to_string(),
                downloads: m["downloads"].as_u64().unwrap_or(0),
                likes: m["likes"].as_u64().unwrap_or(0),
                files: m["siblings"].as_array().into_iter().flatten().filter_map(|s| s["rfilename"].as_str().map(str::to_string)).collect(),
                tags: m["tags"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect(),
                pipeline: m["pipeline_tag"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect();
    Ok((repos, next))
}

/// Bauart und mögliche Modelldateien eines Hugging-Face-Repos (None = läuft hier nicht). Reine
/// Diffusions-Teile (FLUX, Z-Image, Qwen-Image …) sind dabei – ihre Zusatzdateien lädt Foldex mit.
pub fn hf_image(r: &HfRepo) -> Option<(Family, Vec<String>)> {
    // Zusätze, Videomodelle, Bearbeitungsmodelle und Erwachseneninhalte nicht
    const NO: [&str; 16] = ["nsfw", "not-for-all-audiences", "inpaint", "lora", "controlnet", "base_model:adapter", "ip-adapter", "wan2", "hunyuan", "video", "ltx", "audio", "upscal", "-vae", "-edit", "kontext"];
    let id = r.id.to_lowercase();
    let tags: Vec<String> = r.tags.iter().map(|t| t.to_lowercase()).collect();
    if NO.iter().any(|o| id.contains(o) || tags.iter().any(|t| t.contains(o))) {
        return None;
    }
    let fam = family_from_text(&r.id).or_else(|| tags.iter().find_map(|t| family_from_text(t)))?;
    let files: Vec<String> = r.files.iter().filter(|f| is_model_file(f, fam)).cloned().collect();
    (!files.is_empty()).then_some((fam, files))
}

/// ganzes Modell (SD) bzw. Diffusions-Teil – keine Kodierer, Dekodierer, Teilstücke oder Zusätze
fn is_model_file(f: &str, fam: Family) -> bool {
    if fam == Family::Sd {
        return is_ckpt_name(f);
    }
    let l = f.to_lowercase();
    let name = l.rsplit('/').next().unwrap_or(&l);
    (l.ends_with(".gguf") || l.ends_with(".safetensors"))
        && !["text_encoder", "vae", "clip", "t5", "mmproj", "lora", "tokenizer", "controlnet", "qwen2.5-vl", "qwen3vl", "qwen3-vl", "transformer/", "unet/"].iter().any(|x| l.contains(x))
        && !name.starts_with("model-0")
        && !name.starts_with("diffusion_pytorch_model")
}

/// beste Datei: GGUF mit 4 Bit, sonst 8 Bit (fp8/int8), sonst die kleinste; SD: halbe Genauigkeit/„pruned“
pub fn pick_model_file(files: &[(String, u64)], fam: Family) -> Option<String> {
    if fam == Family::Sd {
        return files
            .iter()
            .find(|f| {
                let l = f.0.to_lowercase();
                l.contains("fp16") || l.contains("pruned")
            })
            .or_else(|| files.iter().min_by_key(|f| f.1))
            .map(|f| f.0.clone());
    }
    for pref in ["q4_k_m", "q4_k", "q4_0", "q5_k_m", "q5_0", "q3_k", "fp8", "int8", "q8_0"] {
        if let Some(f) = files.iter().find(|f| f.0.to_lowercase().contains(pref)) {
            return Some(f.0.clone());
        }
    }
    files.iter().min_by_key(|f| f.1).map(|f| f.0.clone())
}

/// sieht wie ein ganzes Bildmodell im Hauptordner aus (kein LoRA, Dekodierer, ControlNet …)
fn is_ckpt_name(f: &str) -> bool {
    let l = f.to_lowercase();
    !f.contains('/')
        && (l.ends_with(".safetensors") || l.ends_with(".gguf") || l.ends_with(".ckpt"))
        && !["lora", "vae", "controlnet", "inpaint", "refiner", "ip-adapter", "ip_adapter", "embedding", "adapter", "text_encoder", "clip", "t5"].iter().any(|x| l.contains(x))
}

/// beste GGUF-Datei eines Text-Repos (Q4_K_M, sonst die nächstbeste mittlere Stufe)
pub fn best_gguf(r: &HfRepo) -> Option<String> {
    // nur Sprachmodelle (auch solche, die zusätzlich Bilder verstehen) – keine Sprachausgabe, Musik, Einbettungen
    if !r.pipeline.is_empty() && !["text-generation", "image-text-to-text", "text2text-generation", "conversational"].contains(&r.pipeline.as_str()) {
        return None;
    }
    let id = r.id.to_lowercase();
    if ["tts", "audio", "music", "speech", "whisper", "embed", "rerank", "vocoder", "yue", "-asr", "diffusion", "image-gen"].iter().any(|x| id.contains(x) || r.tags.iter().any(|t| t.to_lowercase().contains(x))) {
        return None;
    }
    let g: Vec<&String> = r.files.iter().filter(|f| {
        let l = f.to_lowercase();
        l.ends_with(".gguf") && !l.contains("mmproj") && !(l.contains("-of-0") && !l.contains("-00001-of-"))
    }).collect();
    for pref in ["q4_k_m", "q4_k_s", "q4_0", "iq4_xs", "q5_k_m", "q3_k_m", "q8_0"] {
        if let Some(f) = g.iter().find(|f| f.to_lowercase().contains(pref)) {
            return Some((*f).clone());
        }
    }
    g.first().map(|f| (*f).clone())
}

/// Bild-Einzeldatei laden
pub fn install_hf_image(repo: &str, file: &str, fam: Family) {
    if sd_exe().is_none() {
        install_runtime(sd_runtime());
    }
    let dest = local_path(file, false);
    remember_family(&dest, fam);
    enqueue(Job { label: file.rsplit('/').next().unwrap_or(file).to_string(), url: hf_url(repo, file), dest, unzip_to: None, clean: false, auth: None });
    fam.enqueue_missing();
}

/// Ziel einer Datei aus der Suche (für „schon geladen?“)
pub fn hf_dest(file: &str, text: bool) -> PathBuf {
    local_path(file, text)
}

/// selbst geladene Bildmodelle (Einzeldateien, die zu keinem Katalogmodell gehören)
pub fn installed_image_files() -> Vec<(String, PathBuf, u64)> {
    // Katalogdateien und Zusatzdateien der Bauarten sind keine eigenen Modelle
    let mut catalog: Vec<PathBuf> = IMAGE.iter().flat_map(|m| m.files.iter().map(|(r, f)| image_file_path(*r, f))).collect();
    catalog.extend(Family::ALL.iter().flat_map(|f| f.companions().iter().map(|(r, h)| image_file_path(*r, h))));
    // Bearbeitungsmodelle erzeugen keine Bilder aus Text
    catalog.extend(EDIT.iter().map(|m| m.path()));
    let mut v: Vec<(String, PathBuf, u64)> = std::fs::read_dir(image_dir())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| ["safetensors", "gguf", "ckpt"].iter().any(|x| e.eq_ignore_ascii_case(x))))
                .filter(|p| !catalog.contains(p))
                .filter_map(|p| {
                    let n = std::fs::metadata(&p).ok()?.len();
                    Some((p.file_stem()?.to_string_lossy().into_owned(), p, n))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    v
}

/// sinnvolle Grundeinstellungen für eine eigene Bilddatei: (Größe, Schritte, CFG)
pub fn file_defaults(p: &Path) -> (u32, u32, f32) {
    let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let fam = family_of(p);
    if fam != Family::Sd {
        let (s, st, c, _, _) = fam.settings(is_fast(&n));
        return (s, st, c);
    }
    let size = std::fs::metadata(p).map_or(0, |m| m.len());
    let xl = n.contains("xl") || size > 5_000_000_000;
    let fast = n.contains("turbo") || n.contains("lightning") || n.contains("hyper") || n.contains("lcm");
    (if xl { 1024 } else { 512 }, if fast { 6 } else { 25 }, if fast { 1.5 } else if xl { 6.0 } else { 7.0 })
}

// ───────────────────────── Bauarten von Bildmodellen ─────────────────────────
//
// Viele Bildmodelle (vor allem auf Civitai) sind nur der Diffusions-Teil; Textkodierer und Bild-Dekodierer
// gehören zur Bauart und sind bei allen Modellen dieser Bauart gleich. Foldex lädt sie einmal mit.

const CLIP_L: HfFile = HfFile { repo: "comfyanonymous/flux_text_encoders", file: "clip_l.safetensors", mb: 246 };
const CLIP_G: HfFile = HfFile { repo: "Comfy-Org/stable-diffusion-3.5-fp8", file: "text_encoders/clip_g.safetensors", mb: 1389 };
const T5XXL: HfFile = HfFile { repo: "city96/t5-v1_1-xxl-encoder-gguf", file: "t5-v1_1-xxl-encoder-Q5_K_M.gguf", mb: 3386 };
const QWEN_VAE: HfFile = HfFile { repo: "QuantStack/Qwen-Image-GGUF", file: "VAE/Qwen_Image-VAE.safetensors", mb: 253 };
const QWEN25_VL: HfFile = HfFile { repo: "mradermacher/Qwen2.5-VL-7B-Instruct-GGUF", file: "Qwen2.5-VL-7B-Instruct.Q4_K_M.gguf", mb: 4683 };
const QWEN21_VAE: HfFile = HfFile { repo: "Comfy-Org/Qwen-Image-2.1", file: "vae/qwen_image_2.1_vae_bf16.safetensors", mb: 675 };
const QWEN3_VL8: HfFile = HfFile { repo: "Qwen/Qwen3-VL-8B-Instruct-GGUF", file: "Qwen3VL-8B-Instruct-Q4_K_M.gguf", mb: 5027 };
const WAN_VAE: HfFile = HfFile { repo: "Comfy-Org/Wan_2.1_ComfyUI_repackaged", file: "split_files/vae/wan_2.1_vae.safetensors", mb: 253 };
const QWEN3_VL4: HfFile = HfFile { repo: "Qwen/Qwen3-VL-4B-Instruct-GGUF", file: "Qwen3VL-4B-Instruct-Q4_K_M.gguf", mb: 2497 };
const ANIMA_VAE: HfFile = HfFile { repo: "circlestone-labs/Anima", file: "split_files/vae/qwen_image_vae.safetensors", mb: 253 };
const FLUX2_VAE: HfFile = HfFile { repo: "Comfy-Org/Flux-2-Klein-4B", file: "split_files/vae/flux2-vae.safetensors", mb: 336 };
const QWEN3_4B_BASE: HfFile = HfFile { repo: "unsloth/Qwen3-4B-GGUF", file: "Qwen3-4B-Q4_K_M.gguf", mb: 2497 };
const QWEN3_06B: HfFile = HfFile { repo: "mradermacher/Qwen3-0.6B-Base-GGUF", file: "Qwen3-0.6B-Base.Q8_0.gguf", mb: 639 };

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    /// SD 1.x/2.x, SDXL, Pony, Illustrious, NoobAI – eine Datei genügt
    Sd,
    Sd3,
    Flux,
    ZImage,
    Qwen,
    Qwen21,
    Chroma,
    Krea2,
    Anima,
    /// FLUX.2 klein (4B): klein und schnell, erzeugt und bearbeitet
    Flux2,
    /// FLUX.3 Image Klein
    Flux3,
}

impl Family {
    pub const ALL: [Family; 11] = [Family::Sd, Family::Sd3, Family::Flux, Family::ZImage, Family::Qwen, Family::Qwen21, Family::Chroma, Family::Krea2, Family::Anima, Family::Flux2, Family::Flux3];

    pub fn key(self) -> &'static str {
        match self {
            Family::Sd => "sd",
            Family::Sd3 => "sd3",
            Family::Flux => "flux",
            Family::ZImage => "zimage",
            Family::Qwen => "qwen",
            Family::Qwen21 => "qwen21",
            Family::Chroma => "chroma",
            Family::Krea2 => "krea2",
            Family::Anima => "anima",
            Family::Flux2 => "flux2",
            Family::Flux3 => "flux3",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Family::Sd => "SD/SDXL",
            Family::Sd3 => "SD 3.5",
            Family::Flux => "FLUX",
            Family::ZImage => "Z-Image",
            Family::Qwen => "Qwen-Image",
            Family::Qwen21 => "Qwen-Image 2.1",
            Family::Chroma => "Chroma",
            Family::Krea2 => "Krea 2",
            Family::Anima => "Anima",
            Family::Flux2 => "FLUX.2 klein",
            Family::Flux3 => "FLUX.3 Klein",
        }
    }

    /// ganze Modelle laden mit `-m`, reine Diffusions-Teile mit `--diffusion-model`
    fn main_flag(self) -> &'static str {
        match self {
            Family::Sd | Family::Sd3 => "-m",
            _ => "--diffusion-model",
        }
    }

    fn companions(self) -> &'static [(Role, HfFile)] {
        match self {
            Family::Sd => &[],
            Family::Sd3 => &[(Role::ClipL, CLIP_L), (Role::ClipG, CLIP_G), (Role::T5xxl, T5XXL)],
            Family::Flux => &[(Role::Vae, FLUX_AE), (Role::ClipL, CLIP_L), (Role::T5xxl, T5XXL)],
            Family::ZImage => &[(Role::Vae, FLUX_AE), (Role::Llm, QWEN3_4B)],
            Family::Qwen => &[(Role::Vae, QWEN_VAE), (Role::Llm, QWEN25_VL)],
            Family::Qwen21 => &[(Role::Vae, QWEN21_VAE), (Role::Llm, QWEN3_VL8)],
            Family::Chroma => &[(Role::Vae, FLUX_AE), (Role::T5xxl, T5XXL)],
            Family::Krea2 => &[(Role::Vae, WAN_VAE), (Role::Llm, QWEN3_VL4)],
            Family::Anima => &[(Role::Vae, ANIMA_VAE), (Role::Llm, QWEN3_06B)],
            Family::Flux2 => &[(Role::Vae, FLUX2_VAE), (Role::Llm, QWEN3_4B_BASE)],
            Family::Flux3 => &[(Role::Vae, FLUX2_VAE), (Role::Llm, QWEN3_VL4)],
        }
    }

    /// (Größe, Schritte, CFG, negativer Prompt sinnvoll, Zusatzschalter); `fast` = Turbo/Lightning/Schnell
    pub fn settings(self, fast: bool) -> (u32, u32, f32, bool, &'static [&'static str]) {
        const BIG: &[&str] = &["--offload-to-cpu", "--diffusion-fa"];
        match self {
            Family::Sd => (512, if fast { 6 } else { 25 }, if fast { 1.5 } else { 7.0 }, true, &[]),
            Family::Sd3 => (1024, if fast { 8 } else { 28 }, if fast { 1.0 } else { 4.5 }, true, &["--offload-to-cpu", "--clip-on-cpu", "--sampling-method", "euler"]),
            Family::Flux => (1024, if fast { 4 } else { 20 }, 1.0, false, &["--offload-to-cpu", "--diffusion-fa", "--clip-on-cpu", "--sampling-method", "euler"]),
            Family::ZImage => (1024, if fast { 8 } else { 28 }, if fast { 1.0 } else { 4.0 }, !fast, BIG),
            Family::Qwen => (1024, if fast { 8 } else { 20 }, if fast { 1.0 } else { 2.5 }, false, &["--offload-to-cpu", "--diffusion-fa", "--sampling-method", "euler", "--flow-shift", "3"]),
            Family::Qwen21 if fast => (1024, 8, 1.0, false, &["--offload-to-cpu", "--diffusion-fa", "--sampling-method", "euler", "--sigmas", "qwen-image-2.1-turbo"]),
            Family::Qwen21 => (1024, 30, 6.0, true, &["--offload-to-cpu", "--diffusion-fa", "--sampling-method", "euler"]),
            Family::Chroma => (1024, 26, 4.0, true, &["--offload-to-cpu", "--clip-on-cpu", "--sampling-method", "euler", "--model-args", "chroma_use_dit_mask=false"]),
            Family::Krea2 => (1024, if fast { 8 } else { 28 }, if fast { 1.0 } else { 4.0 }, false, BIG),
            Family::Anima => (1024, 30, 6.0, true, &["--offload-to-cpu", "--sampling-method", "euler"]),
            Family::Flux2 => (1024, 4, 1.0, false, &["--offload-to-cpu", "--diffusion-fa", "--sampling-method", "euler"]),
            Family::Flux3 => (1024, 16, 1.0, false, &["--offload-to-cpu", "--diffusion-fa", "--sampling-method", "euler"]),
        }
    }

    /// Zusatzdateien, die noch fehlen: (Ziel, Adresse, Name, MB)
    pub fn missing(self) -> Vec<(PathBuf, String, String, u32)> {
        self.companions().iter().filter(|(r, f)| !image_file_path(*r, f).is_file()).map(|(r, f)| (image_file_path(*r, f), f.url(), f.file.rsplit('/').next().unwrap_or(f.file).to_string(), f.mb)).collect()
    }

    pub fn companions_mb(self) -> u32 {
        self.companions().iter().map(|c| c.1.mb).sum()
    }

    fn enqueue_missing(self) {
        for (dest, url, name, _) in self.missing() {
            enqueue(Job { label: format!("{} – {name}", self.label()), url, dest, unzip_to: None, clean: false, auth: None });
        }
    }
}

/// schnelle Variante (wenige Schritte) am Namen erkennen
pub fn is_fast(name: &str) -> bool {
    let n = name.to_lowercase();
    ["turbo", "lightning", "hyper", "lcm", "schnell", "distill", "fast", "flash", "dmd"].iter().any(|x| n.contains(x))
}

/// Civitai-Grundmodell → Bauart (None = läuft hier nicht, z. B. Videomodelle)
pub fn family_from_base(base: &str) -> Option<Family> {
    let b = base.to_lowercase();
    Some(if ["sd 1", "sd 2", "sdxl", "pony", "illustrious", "noobai"].iter().any(|x| b.starts_with(x)) {
        Family::Sd
    } else if b.starts_with("sd 3") {
        Family::Sd3
    } else if b.starts_with("flux.2") {
        Family::Flux2
    } else if b.starts_with("flux.3") {
        Family::Flux3
    } else if b.starts_with("flux.1") {
        Family::Flux
    } else if b.starts_with("zimage") || b.starts_with("z-image") {
        Family::ZImage
    } else if b.starts_with("qwen") {
        if b.contains("2.1") {
            Family::Qwen21
        } else {
            Family::Qwen
        }
    } else if b.starts_with("chroma") {
        Family::Chroma
    } else if b.starts_with("krea 2") || b.starts_with("krea2") {
        Family::Krea2
    } else if b.starts_with("anima") {
        Family::Anima
    } else {
        return None;
    })
}

/// Bauart aus Repo-Name und Kennzeichnungen (Hugging Face) bzw. Dateiname
pub fn family_from_text(s: &str) -> Option<Family> {
    let s = s.to_lowercase();
    let has = |x: &str| s.contains(x);
    Some(if has("qwen-image") || has("qwen_image") {
        if has("2.1") || has("2_1") {
            Family::Qwen21
        } else {
            Family::Qwen
        }
    } else if has("z-image") || has("z_image") || has("zimage") {
        Family::ZImage
    } else if has("krea-2") || has("krea_2") || has("krea2") {
        Family::Krea2
    } else if has("chroma") {
        Family::Chroma
    } else if has("anima") && !has("animagine") && !has("animal") && !has("animation") {
        Family::Anima
    } else if has("flux.3") || has("flux-3") || has("flux3") || has("flux_3") {
        Family::Flux3
    } else if has("flux.2") || has("flux-2") || has("flux2") || has("flux_2") {
        Family::Flux2
    } else if has("flux.1") || has("flux1") || has("flux-1") || has("flux_1") {
        Family::Flux
    } else if has("sd3") || has("stable-diffusion-3") || has("sd3.5") {
        Family::Sd3
    } else if has("stable-diffusion") || has("stablediffusion") || has("sdxl") || has("sd-1") || has("sd1") || has("sd15") || has("illustrious") || has("pony") || has("noob") {
        Family::Sd
    } else {
        return None;
    })
}

/// gemerkte Bauart selbst geladener Dateien (Datei → Bauart)
fn families_file() -> PathBuf {
    image_dir().join("bauarten.txt")
}

fn remember_family(file: &Path, f: Family) {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut lines: Vec<String> = std::fs::read_to_string(families_file()).unwrap_or_default().lines().filter(|l| !l.starts_with(&format!("{name}|"))).map(str::to_string).collect();
    lines.push(format!("{name}|{}", f.key()));
    let _ = std::fs::create_dir_all(image_dir());
    let _ = std::fs::write(families_file(), lines.join("\n"));
}

pub fn family_of(file: &Path) -> Family {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    std::fs::read_to_string(families_file())
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name}|")).and_then(|k| Family::ALL.into_iter().find(|f| f.key() == k)))
        .or_else(|| family_from_text(&name))
        .unwrap_or(Family::Sd)
}

// ───────────────────────── Civitai (Bildmodelle) ─────────────────────────

/// Schlüssel-ID im verschlüsselten Schlüsselspeicher
pub const CIVITAI_KEY: &str = "civitai";

#[derive(Clone, Debug, Default)]
pub struct CivModel {
    pub id: u64,
    pub name: String,
    pub creator: String,
    pub downloads: u64,
    /// Grundmodell laut Civitai („SD 1.5“, „SDXL 1.0“, „Pony“, „Flux.1 D“ …)
    pub base: String,
    pub file: String,
    pub mb: u64,
    pub url: String,
    /// Bauart (None = läuft hier nicht, z. B. Videomodelle)
    pub family: Option<Family>,
}

impl CivModel {
    /// läuft mit stable-diffusion.cpp (Zusatzdateien der Bauart lädt Foldex mit)
    pub fn usable(&self) -> bool {
        // Inpainting-Modelle erzeugen ohne Maske keine brauchbaren Bilder
        let inpaint = self.name.to_lowercase().contains("inpaint") || self.file.to_lowercase().contains("inpaint");
        !self.url.is_empty() && !inpaint && self.family.is_some()
    }
}

/// eine Seite Checkpoints von Civitai (ohne Erwachseneninhalte), nach Downloads sortiert
pub fn civitai_page(query: &str, next: Option<&str>) -> Result<(Vec<CivModel>, Option<String>), String> {
    let url = match next {
        Some(n) => n.to_string(),
        None => {
            let q = if query.trim().is_empty() { String::new() } else { format!("&query={}", urlenc(query.trim())) };
            format!("https://civitai.com/api/v1/models?types=Checkpoint&sort=Most%20Downloaded&nsfw=false&limit=20{q}")
        }
    };
    let resp = ureq::get(&url).timeout(Duration::from_secs(25)).call().map_err(|e| match e {
        ureq::Error::Status(c, _) => format!("Civitai: HTTP {c}"),
        ureq::Error::Transport(t) => format!("Civitai nicht erreichbar: {}", t.kind()),
    })?;
    let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let next = v.pointer("/metadata/nextPage").and_then(Value::as_str).map(str::to_string);
    let items = v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| !m["nsfw"].as_bool().unwrap_or(false))
        .filter_map(|m| {
            let ver = m["modelVersions"].as_array()?.first()?;
            // die eigentliche Modelldatei: SafeTensor, bevorzugt halbe Genauigkeit und „pruned“ (kleiner)
            let files: Vec<&Value> = ver["files"].as_array()?.iter().filter(|f| f["type"] == "Model" && f.pointer("/metadata/format").and_then(Value::as_str).is_none_or(|x| x == "SafeTensor")).collect();
            let f = files
                .iter()
                .find(|f| f.pointer("/metadata/fp").and_then(Value::as_str) == Some("fp16") && f.pointer("/metadata/size").and_then(Value::as_str) == Some("pruned"))
                .or_else(|| files.iter().min_by(|a, b| a["sizeKB"].as_f64().unwrap_or(0.0).total_cmp(&b["sizeKB"].as_f64().unwrap_or(0.0))))?;
            Some(CivModel {
                id: m["id"].as_u64().unwrap_or(0),
                name: m["name"].as_str().unwrap_or_default().to_string(),
                creator: m.pointer("/creator/username").and_then(Value::as_str).unwrap_or_default().to_string(),
                downloads: m.pointer("/stats/downloadCount").and_then(Value::as_u64).unwrap_or(0),
                base: ver["baseModel"].as_str().unwrap_or_default().to_string(),
                family: family_from_base(ver["baseModel"].as_str().unwrap_or_default()),
                file: f["name"].as_str().unwrap_or_default().to_string(),
                mb: (f["sizeKB"].as_f64().unwrap_or(0.0) / 1000.0) as u64,
                url: f["downloadUrl"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect();
    Ok((items, next))
}

pub fn civitai_dest(m: &CivModel) -> PathBuf {
    image_dir().join(&m.file)
}

/// Civitai-Modell laden – Civitai verlangt dafür einen (kostenlosen) API-Schlüssel
pub fn install_civitai(m: &CivModel, key: &str) {
    if sd_exe().is_none() {
        install_runtime(sd_runtime());
    }
    if let Some(f) = m.family {
        remember_family(&civitai_dest(m), f);
        f.enqueue_missing();
    }
    enqueue(Job { label: m.file.clone(), url: m.url.clone(), dest: civitai_dest(m), unzip_to: None, clean: false, auth: Some(key.to_string()) });
}

/// GGUF-Dateien eines Repos mit Größe (Projektor-Dateien für Bilder und spätere Teile ausgeblendet)
pub fn hf_files(repo: &str, kind: HfKind) -> Result<Vec<(String, u64)>, String> {
    let url = format!("https://huggingface.co/api/models/{repo}/tree/main?recursive=1");
    let v: Value = serde_json::from_str(&ureq::get(&url).timeout(Duration::from_secs(20)).call().map_err(|e| format!("Hugging Face: {e}"))?.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let all: Vec<(String, u64)> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["type"] == "file")
        .filter_map(|e| Some((e["path"].as_str()?.to_string(), e["lfs"]["size"].as_u64().or(e["size"].as_u64()).unwrap_or(0))))
        .filter(|(p, _)| if kind == HfKind::Text { p.to_lowercase().ends_with(".gguf") } else { is_ckpt_name(p) || is_model_file(p, family_from_text(repo).unwrap_or(Family::Flux)) })
        .collect();
    if kind == HfKind::Image {
        return Ok(all);
    }
    // geteilte Dateien: Größe aller Teile beim ersten Teil
    let mut out = Vec::new();
    for (p, s) in &all {
        let low = p.to_lowercase();
        if low.contains("mmproj") || (low.contains("-of-0") && !low.contains("-00001-of-")) {
            continue;
        }
        let size = match split_parts(p) {
            Some(prefix) => all.iter().filter(|(q, _)| q.starts_with(&prefix)).map(|(_, s)| s).sum(),
            None => *s,
        };
        out.push((p.clone(), size));
    }
    Ok(out)
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ───────────────────────── Textmodell-Server ─────────────────────────

pub const PORT: u16 = 39281;
/// Adresse für Modellprofile – daran erkennt der Chat ein eingebautes lokales Modell
pub const BASE: &str = "http://127.0.0.1:39281/v1";

struct Server {
    child: Child,
    model: String,
}

fn server() -> &'static Mutex<Option<Server>> {
    static S: Mutex<Option<Server>> = Mutex::new(None);
    &S
}

pub fn is_builtin(base: &str) -> bool {
    base.trim_end_matches('/') == BASE
}

/// Modell, das gerade läuft
pub fn server_model() -> Option<String> {
    let mut g = server().lock().ok()?;
    let s = g.as_mut()?;
    if s.child.try_wait().ok().flatten().is_some() {
        *g = None;
        return None;
    }
    Some(s.model.clone())
}

pub fn stop_server() {
    if let Ok(mut g) = server().lock() {
        if let Some(mut s) = g.take() {
            let _ = s.child.kill();
            let _ = s.child.wait();
        }
    }
}

/// gestartetes Programm an Foldex binden: endet Foldex (auch durch Absturz), endet es mit
pub fn tie_to_app(child: &Child) {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        static JOB: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        let job = *JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !h.is_null() {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(h, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
            }
            h as usize
        });
        if job != 0 {
            unsafe {
                AssignProcessToJobObject(job as _, child.as_raw_handle() as _);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = child;
}

fn log_path() -> PathBuf {
    root().join("llama-server.log")
}

/// vor jeder Anfrage an ein eingebautes Modell: Server mit diesem Modell bereitstellen (blockiert, bis er
/// antwortet – das erste Laden eines großen Modells dauert etwas)
pub fn prepare(base: &str, model: &str) -> Result<(), String> {
    if !is_builtin(base) {
        return Ok(());
    }
    let mut g = server().lock().map_err(|_| "Server gesperrt".to_string())?;
    if let Some(s) = g.as_mut() {
        if s.model == model && s.child.try_wait().ok().flatten().is_none() {
            return Ok(());
        }
        let _ = s.child.kill();
        let _ = s.child.wait();
        *g = None;
    }
    let exe = llama_exe().ok_or("llama.cpp fehlt – in Einstellungen → Lokale Modelle installieren")?;
    let path = installed_text().into_iter().find(|(stem, _, _)| stem == model).map(|t| t.1).ok_or_else(|| format!("Lokales Modell „{model}“ ist nicht installiert"))?;
    // erst mit Grafikkarte, wenn das scheitert (zu wenig Grafikspeicher) auf dem Prozessor
    let mut last = String::new();
    for gpu in [true, false] {
        let log = std::fs::File::create(log_path()).map_err(|e| e.to_string())?;
        let mut c = Command::new(&exe);
        lib_path(&mut c, &exe);
        c.current_dir(exe.parent().unwrap_or(Path::new(".")))
            .arg("-m")
            .arg(&path)
            .args(["--host", "127.0.0.1", "--port", &PORT.to_string(), "-c", "16384", "--jinja", "--alias", model])
            .args(["-ngl", if gpu { "999" } else { "0" }])
            .args(mmproj_for(&path).map(|m| vec!["--mmproj".to_string(), m.display().to_string()]).unwrap_or_default())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // kein Konsolenfenster
        }
        let mut child = c.spawn().map_err(|e| format!("llama-server starten: {e}"))?;
        tie_to_app(&child);
        let t0 = Instant::now();
        let health = format!("http://127.0.0.1:{PORT}/health");
        let ok = loop {
            if child.try_wait().ok().flatten().is_some() {
                break false;
            }
            if ureq::get(&health).timeout(Duration::from_secs(2)).call().is_ok() {
                break true;
            }
            if t0.elapsed() > Duration::from_secs(300) {
                let _ = child.kill();
                break false;
            }
            std::thread::sleep(Duration::from_millis(250));
        };
        if ok {
            *g = Some(Server { child, model: model.to_string() });
            return Ok(());
        }
        let code = child.wait().ok().and_then(|s| s.code());
        if let Some(e) = missing_dll(code) {
            return Err(e);
        }
        last = std::fs::read_to_string(log_path()).unwrap_or_default().lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().to_string();
    }
    Err(format!("Lokales Modell ließ sich nicht starten{}", if last.is_empty() { String::new() } else { format!(": {last}") }))
}

// ───────────────────────── Bilder erzeugen ─────────────────────────

/// gewähltes Bildmodell: aus dem Katalog oder eine selbst geladene Einzeldatei
#[derive(Clone, Debug, PartialEq)]
pub enum ImgChoice {
    Catalog(&'static str),
    File(PathBuf),
}

#[derive(Clone, Debug)]
pub struct GenReq {
    pub model: ImgChoice,
    pub prompt: String,
    pub negative: String,
    pub w: u32,
    pub h: u32,
    pub steps: u32,
    pub cfg: f32,
    pub seed: i64,
    pub count: u32,
}

// ───────────────────────── Spracheingabe (whisper.cpp) ─────────────────────────

pub static WHISPER: Runtime = Runtime {
    name: "whisper.cpp (Spracheingabe)",
    url: if cfg!(windows) { "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip" } else { "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-ubuntu-x64.tar.gz" },
    extra: None,
    folder: "whisper.cpp",
    exe: if cfg!(windows) { &["whisper-cli.exe", "main.exe"] } else { &["whisper-cli", "main"] },
    mb: 8,
};

pub struct SttModel {
    pub name: &'static str,
    pub desc: &'static str,
    file: HfFile,
}

impl SttModel {
    pub fn mb(&self) -> u32 {
        self.file.mb
    }
}

/// Spracherkennungs-Modelle (mehrsprachig, auch Deutsch); das genauere wird bevorzugt, wenn beide da sind
pub static STT: &[SttModel] = &[
    SttModel { name: "Whisper large-v3-turbo", desc: "Sehr genau, auch bei Fachbegriffen", file: HfFile { repo: "ggerganov/whisper.cpp", file: "ggml-large-v3-turbo-q5_0.bin", mb: 574 } },
    SttModel { name: "Whisper small", desc: "Schnell und klein, gut für Alltagssprache", file: HfFile { repo: "ggerganov/whisper.cpp", file: "ggml-small-q5_1.bin", mb: 190 } },
];

pub fn stt_path(m: &SttModel) -> PathBuf {
    root().join("speech").join(m.file.file)
}

/// installiertes Modell (und Programm) – None = Spracheingabe noch nicht eingerichtet
pub fn stt_ready() -> Option<PathBuf> {
    exe(&WHISPER)?;
    let active = stt_active().map(stt_path).filter(|p| p.is_file());
    active.or_else(|| STT.iter().map(stt_path).find(|p| p.is_file()))
}

fn stt_choice_file() -> PathBuf {
    root().join("speech").join("aktiv.txt")
}

/// gewähltes Spracheingabe-Modell (None = das erste installierte)
pub fn stt_active() -> Option<&'static SttModel> {
    let f = std::fs::read_to_string(stt_choice_file()).ok()?;
    STT.iter().find(|m| m.file.file == f.trim())
}

/// installiertes Modell, das gerade benutzt wird
pub fn stt_in_use() -> Option<&'static SttModel> {
    let p = stt_ready()?;
    STT.iter().find(|m| stt_path(m) == p)
}

pub fn set_stt_active(m: &SttModel) {
    let _ = std::fs::create_dir_all(root().join("speech"));
    let _ = std::fs::write(stt_choice_file(), m.file.file);
}

pub fn delete_stt(m: &SttModel) -> Result<(), String> {
    delete_file(&stt_path(m))
}

pub fn install_stt(m: &SttModel) {
    if exe(&WHISPER).is_none() {
        install_runtime(&WHISPER);
    }
    enqueue(Job { label: m.name.into(), url: m.file.url(), dest: stt_path(m), unzip_to: None, clean: false, auth: None });
}

/// whisper.cpp-Programm (für Dateien umwandeln)
pub fn whisper_exe() -> Option<PathBuf> {
    exe(&WHISPER)
}

/// Bibliothekspfad für whisper.cpp setzen
pub fn whisper_env(c: &mut Command, exe: &Path) {
    lib_path(c, exe);
}

/// Aufnahme (WAV, 16 kHz mono) in Text umwandeln; Sprache wird erkannt
pub fn transcribe(wav: &Path) -> Result<String, String> {
    let model = stt_ready().ok_or("Spracheingabe ist nicht eingerichtet")?;
    let exe = exe(&WHISPER).ok_or("whisper.cpp fehlt")?;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let mut c = Command::new(&exe);
        lib_path(&mut c, &exe);
    c.current_dir(exe.parent().unwrap_or(Path::new(".")))
        .arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(wav)
        .args(["-l", "auto", "-nt", "-np", "-t", &threads.to_string()])
        // Eigennamen richtig schreiben (sonst wird aus „Foldex“ gern „Volldex“)
        .args(["--prompt", "Foldex"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let out = c.output().map_err(|e| format!("whisper.cpp starten: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("Spracherkennung fehlgeschlagen: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("")));
    }
    // Zeilen zusammenfügen; Platzhalter wie „[BLANK_AUDIO]“ fallen weg
    let text: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|l| !l.is_empty() && !(l.starts_with('[') && l.ends_with(']'))).map(str::to_string).collect();
    Ok(text.join(" "))
}

/// alle nutzbaren Bildmodelle, bestes zuerst: (Name, Wahl, Bauart, Standardgröße)
pub fn image_choices() -> Vec<(String, ImgChoice, String, u32)> {
    // Katalog von hinten = größere, bessere Modelle zuerst
    let mut v: Vec<(String, ImgChoice, String, u32)> = IMAGE.iter().rev().filter(|m| image_installed(m)).map(|m| (m.name.to_string(), ImgChoice::Catalog(m.id), m.desc.to_string(), m.size)).collect();
    for (stem, p, _) in installed_image_files() {
        let f = family_of(&p);
        let c = ImgChoice::File(p);
        let size = choice_settings(&c).0;
        v.push((stem, c, format!("Eigenes Modell ({})", f.label()), size));
    }
    v
}

/// Grundeinstellungen eines Bildmodells: (Größe, Schritte, CFG, negativer Prompt sinnvoll)
pub fn choice_settings(c: &ImgChoice) -> (u32, u32, f32, bool) {
    match c {
        ImgChoice::Catalog(id) => image_model(id).map_or((512, 20, 7.0, true), |m| (m.size, m.steps, m.cfg, m.negative)),
        ImgChoice::File(p) => {
            let (s, st, cfg) = file_defaults(p);
            let neg = family_of(p).settings(is_fast(&p.to_string_lossy())).3;
            (s, st, cfg, neg)
        }
    }
}

pub fn image_model(id: &str) -> Option<&'static ImageModel> {
    IMAGE.iter().find(|m| m.id == id)
}

/// Bilder erzeugen und in `out_dir` speichern; liefert die Dateien
pub fn generate(req: &GenReq, out_dir: &Path, p: &Progress) -> Result<Vec<PathBuf>, String> {
    // (Schalter, Datei) und Zusatzschalter je nach Modell
    let (files, extra, negative): (Vec<(&str, PathBuf)>, &[&str], bool) = match &req.model {
        ImgChoice::Catalog(id) => {
            let m = image_model(id).ok_or("Unbekanntes Bildmodell")?;
            if !image_installed(m) {
                return Err(format!("{} ist nicht vollständig installiert", m.name));
            }
            (m.files.iter().map(|(r, f)| (r.flag(), image_file_path(*r, f))).collect(), m.extra, m.negative)
        }
        ImgChoice::File(p) => {
            let fam = family_of(p);
            let miss = fam.missing();
            if !miss.is_empty() {
                // nachladen und Bescheid geben
                fam.enqueue_missing();
                return Err(format!("Zusatzdateien für {} werden noch geladen ({}) – Fortschritt unter Einstellungen → Lokale Modelle", fam.label(), miss.iter().map(|m| m.2.as_str()).collect::<Vec<_>>().join(", ")));
            }
            let (_, _, _, neg, extra) = fam.settings(is_fast(&p.to_string_lossy()));
            let mut v = vec![(fam.main_flag(), p.clone())];
            v.extend(fam.companions().iter().map(|(r, f)| (r.flag(), image_file_path(*r, f))));
            (v, extra, neg)
        }
    };
    let exe = sd_exe().ok_or("stable-diffusion.cpp fehlt – in Einstellungen → Lokale Modelle installieren")?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let mut out = Vec::new();
    for i in 0..req.count.max(1) {
        p.check()?;
        let file = unique(out_dir, &format!("KI-Bild_{stamp}{}", if req.count > 1 { format!("_{}", i + 1) } else { String::new() }));
        let mut c = Command::new(&exe);
        lib_path(&mut c, &exe);
        c.current_dir(exe.parent().unwrap_or(Path::new(".")));
        for (flag, path) in &files {
            c.arg(flag).arg(path);
        }
        c.args(["-p", &req.prompt])
            .args(["-W", &req.w.to_string(), "-H", &req.h.to_string()])
            .args(["--steps", &req.steps.to_string(), "--cfg-scale", &format!("{:.1}", req.cfg)])
            .args(["-s", &(req.seed + i as i64).to_string()])
            .args(extra)
            .arg("-o")
            .arg(&file)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if negative && !req.negative.trim().is_empty() {
            c.args(["-n", &req.negative]);
        }
        run_sd(c, p, (i as usize, req.count.max(1) as usize), "Bild")?;
        if !file.is_file() {
            return Err("Bild erzeugen fehlgeschlagen: keine Ausgabedatei".into());
        }
        out.push(file);
    }
    Ok(out)
}

/// stable-diffusion.cpp ausführen: Fortschritt („ 3/8 “ auf stdout oder stderr, mit \r statt \n) melden,
/// Abbruch beachten. `part` = (Nummer, Anzahl) bei mehreren Bildern.
fn run_sd(mut c: Command, p: &Progress, part: (usize, usize), what: &str) -> Result<(), String> {
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let mut child = c.spawn().map_err(|e| format!("stable-diffusion.cpp starten: {e}"))?;
    tie_to_app(&child);
    let tail = Arc::new(Mutex::new(String::new()));
    let mut readers = Vec::new();
    for pipe in [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)].into_iter().flatten() {
        let (p, tail, what) = (p.clone(), tail.clone(), what.to_string());
        let (n, count) = part;
        readers.push(std::thread::spawn(move || {
            let mut rd = std::io::BufReader::new(pipe);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                let mut byte = [0u8; 1];
                loop {
                    match rd.read(&mut byte) {
                        Ok(1) if byte[0] != b'\r' && byte[0] != b'\n' => buf.push(byte[0]),
                        Ok(1) => break,
                        _ => return,
                    }
                }
                let line = String::from_utf8_lossy(&buf).to_string();
                if let Some((a, b)) = step_of(&line) {
                    let f = (n as f32 + a as f32 / b.max(1) as f32) / count.max(1) as f32;
                    let nr = if count > 1 { format!(" {} von {count}", n + 1) } else { String::new() };
                    p.set(f, format!("{what}{nr} · Schritt {a}/{b}"));
                } else if line.contains("loading") || line.contains("load ") {
                    p.set(-1.0, "Modell laden …");
                }
                if !line.trim().is_empty() {
                    if let Ok(mut t) = tail.lock() {
                        *t = line;
                    }
                }
            }
        }));
    }
    let st = loop {
        if p.check().is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Abgebrochen".into());
        }
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    for r in readers {
        let _ = r.join();
    }
    if let Some(e) = missing_dll(st.code()) {
        return Err(e);
    }
    if !st.success() {
        let t = tail.lock().map(|t| t.clone()).unwrap_or_default();
        return Err(format!("{what} fehlgeschlagen{}", if t.is_empty() { String::new() } else { format!(": {t}") }));
    }
    Ok(())
}

/// Windows-Exit-Code „DLL nicht gefunden“ (0xC0000135) bzw. „Einsprungpunkt fehlt“ (0xC0000139)
fn missing_dll(code: Option<i32>) -> Option<String> {
    match code.map(|c| c as u32) {
        Some(0xC000_0135) | Some(0xC000_0139) => Some("Eine Programmdatei (DLL) fehlt – das Programmpaket ist unvollständig. Unter Einstellungen → Lokale Modelle erneut auf „Installieren“ klicken (setzt den Download fort).".into()),
        _ => None,
    }
}

// ───────────────────────── Bild per Text bearbeiten ─────────────────────────

/// Modelle, die ein vorhandenes Bild nach einer Anweisung ändern („mach den Himmel rot“)
pub struct EditModel {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    main: HfFile,
    fam: Family,
    pub steps: u32,
    cfg: f32,
    extra: &'static [&'static str],
}

pub static EDIT: &[EditModel] = &[
    EditModel {
        id: "flux2-klein",
        name: "FLUX.2 klein 4B",
        desc: "Klein und schnell (4 Schritte) – gut für Alltags-Änderungen, läuft schon mit 6 GB",
        main: HfFile { repo: "leejet/FLUX.2-klein-4B-GGUF", file: "flux-2-klein-4b-Q4_0.gguf", mb: 2460 },
        fam: Family::Flux2,
        steps: 4,
        cfg: 1.0,
        extra: &["--sampling-method", "euler", "--offload-to-cpu", "--diffusion-fa"],
    },
    EditModel {
        id: "flux3-klein",
        name: "FLUX.3 Image Klein",
        desc: "Neuer und genauer, noch kompakt (16 Schritte)",
        main: HfFile { repo: "leejet/flux-3-image-klein-GGUF", file: "flux3-image-klein-Q4_K.gguf", mb: 3514 },
        fam: Family::Flux3,
        steps: 16,
        cfg: 1.0,
        extra: &["--sampling-method", "euler", "--offload-to-cpu", "--diffusion-fa"],
    },
    EditModel {
        id: "flux-kontext",
        name: "FLUX.1 Kontext",
        desc: "Ändert Bilder nach Anweisung, behält Personen und Stil gut bei – läuft mit 6–8 GB Grafikspeicher",
        main: HfFile { repo: "QuantStack/FLUX.1-Kontext-dev-GGUF", file: "flux1-kontext-dev-Q4_K_S.gguf", mb: 6797 },
        fam: Family::Flux,
        steps: 20,
        cfg: 1.0,
        extra: &["--sampling-method", "euler", "--clip-on-cpu", "--offload-to-cpu", "--diffusion-fa"],
    },
    EditModel {
        id: "qwen-edit",
        name: "Qwen-Image-Edit 2511",
        desc: "Sehr genaue Änderungen, auch Text im Bild – groß, braucht viel Speicher",
        main: HfFile { repo: "unsloth/Qwen-Image-Edit-2511-GGUF", file: "qwen-image-edit-2511-Q4_K_M.gguf", mb: 12842 },
        fam: Family::Qwen,
        steps: 20,
        cfg: 2.5,
        extra: &["--sampling-method", "euler", "--offload-to-cpu", "--diffusion-fa", "--flow-shift", "3", "--model-args", "qwen_image_zero_cond_t=true"],
    },
];

impl EditModel {
    fn path(&self) -> PathBuf {
        local_path(self.main.file, false)
    }
    /// noch fehlend: (Ziel, MB) – Hauptdatei und Zusatzdateien der Bauart
    pub fn missing_mb(&self) -> u32 {
        (if self.path().is_file() { 0 } else { self.main.mb }) + self.fam.missing().iter().map(|m| m.3).sum::<u32>()
    }
    pub fn total_mb(&self) -> u32 {
        self.main.mb + self.fam.companions_mb()
    }
    pub fn installed(&self) -> bool {
        self.path().is_file() && self.fam.missing().is_empty()
    }
    /// läuft der Download gerade? (Anteil, Text)
    pub fn progress(&self) -> Option<(f32, String)> {
        queued(&self.path()).or_else(|| self.fam.missing().iter().find_map(|m| queued(&m.0)))
    }
    pub fn fit(&self) -> Fit {
        fit_image(self.main.mb as u64, self.total_mb() as u64)
    }
}

pub fn install_edit(m: &EditModel) {
    if sd_exe().is_none() {
        install_runtime(sd_runtime());
    }
    if !m.path().is_file() {
        enqueue(Job { label: m.name.into(), url: m.main.url(), dest: m.path(), unzip_to: None, clean: false, auth: None });
    }
    m.fam.enqueue_missing();
}

pub fn delete_edit(m: &EditModel) -> Result<(), String> {
    delete_file(&m.path())
}

/// Bild nach Anweisung ändern. Gerechnet wird mit höchstens 1024 px (Seiten durch 16 teilbar), das Ergebnis
/// kommt in der Originalgröße zurück.
pub fn edit_image(m: &EditModel, img: &image::RgbaImage, prompt: &str, seed: i64, p: &Progress) -> Result<image::RgbaImage, String> {
    if !m.installed() {
        return Err(format!("{} ist noch nicht vollständig geladen", m.name));
    }
    let exe = sd_exe().ok_or("stable-diffusion.cpp fehlt – in Einstellungen → Lokale Modelle installieren")?;
    let (w, h) = img.dimensions();
    let k = (1024.0 / w.max(h) as f32).min(1.0);
    let (sw, sh) = ((((w as f32 * k) as u32) / 16 * 16).max(256), (((h as f32 * k) as u32) / 16 * 16).max(256));
    let tmp = std::env::temp_dir().join(format!("foldex_edit_{}_{}", std::process::id(), seed));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let (inp, outp) = (tmp.join("in.png"), tmp.join("out.png"));
    let res = (|| {
        // durchsichtige Stellen auf Weiß, sonst rät das Modell
        let mut flat = image::RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
        for (x, y, px) in img.enumerate_pixels() {
            let a = px[3] as u32;
            let o = flat.get_pixel_mut(x, y);
            for c in 0..3 {
                o[c] = ((px[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
            }
        }
        image::imageops::resize(&flat, sw, sh, image::imageops::FilterType::Lanczos3).save(&inp).map_err(|e| e.to_string())?;
        let mut c = Command::new(&exe);
        lib_path(&mut c, &exe);
        c.current_dir(exe.parent().unwrap_or(Path::new(".")));
        c.arg(m.fam.main_flag()).arg(m.path());
        for (r, f) in m.fam.companions() {
            c.arg(r.flag()).arg(image_file_path(*r, f));
        }
        c.arg("-r")
            .arg(&inp)
            .args(["-p", prompt])
            .args(["-W", &sw.to_string(), "-H", &sh.to_string()])
            .args(["--steps", &m.steps.to_string(), "--cfg-scale", &format!("{:.1}", m.cfg), "-s", &seed.to_string()])
            .args(m.extra)
            .arg("-o")
            .arg(&outp);
        run_sd(c, p, (0, 1), "Bearbeiten")?;
        let out = image::open(&outp).map_err(|e| format!("Ergebnis lesen: {e}"))?.to_rgba8();
        let mut out = image::imageops::resize(&out, w, h, image::imageops::FilterType::Lanczos3);
        // Durchsichtigkeit des Originals behalten
        for (o, i) in out.pixels_mut().zip(img.pixels()) {
            o[3] = i[3];
        }
        Ok(out)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

/// „|=====>   | 3/8 - 1.20s/it“ → (3, 8)
fn step_of(line: &str) -> Option<(u32, u32)> {
    let l = line.trim();
    if !l.contains('|') {
        return None;
    }
    let after = l.rsplit('|').next()?.trim();
    let tok = after.split_whitespace().next()?;
    let (a, b) = tok.split_once('/')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn unique(dir: &Path, stem: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}.png"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}).png"));
        n += 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// echter Durchlauf: llama.cpp laden (falls nötig), kleinstes installiertes Modell starten, eine Frage stellen.
    /// cargo test real_llama -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_llama() {
        if llama_exe().is_none() {
            install_runtime(&LLAMA);
            let t = Instant::now();
            while llama_exe().is_none() && t.elapsed() < Duration::from_secs(600) {
                for d in downloads() {
                    eprintln!("{} {:.0} % {}", d.0, d.1 * 100.0, d.2);
                }
                for (err, m) in take_log() {
                    eprintln!("Log: {m}");
                    assert!(!err, "{m}");
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
        let exe = llama_exe().expect("llama-server fehlt");
        eprintln!("NVIDIA: {}", has_nvidia());
        eprintln!("llama-server: {}", exe.display());
        let (stem, path, _) = installed_text().into_iter().min_by_key(|t| t.2).expect("kein Textmodell");
        eprintln!("Modell: {stem} ({})", path.display());
        let t = Instant::now();
        prepare(BASE, &stem).unwrap();
        eprintln!("Server bereit nach {:?}", t.elapsed());
        let body = serde_json::json!({ "model": stem, "messages": [{ "role": "user", "content": "Antworte nur mit einem Wort: Was ist die Hauptstadt von Frankreich?" }], "max_tokens": 300 });
        let t = Instant::now();
        let r = ureq::post(&format!("{BASE}/chat/completions")).set("content-type", "application/json").send_string(&body.to_string()).unwrap().into_string().unwrap();
        let v: Value = serde_json::from_str(&r).unwrap();
        eprintln!("Antwort nach {:?}: {:?} / Denken: {:?} / {}", t.elapsed(), v.pointer("/choices/0/message/content"), v.pointer("/choices/0/message/reasoning_content").and_then(Value::as_str).map(|s| s.chars().take(120).collect::<String>()), v["timings"]);
        eprintln!("Log: {}", std::fs::read_to_string(log_path()).unwrap_or_default().lines().filter(|l| l.contains("Vulkan") || l.contains("offload") || l.contains("GPU")).take(6).collect::<Vec<_>>().join("\n     "));
        stop_server();
    }

    /// Hugging Face live: zwei Seiten je Art, Kompatibilität, PC-Daten
    #[test]
    #[ignore]
    fn real_hf() {
        let pc = pc();
        eprintln!("PC: {} · {} MB VRAM · {} MB RAM", pc.gpu, pc.vram_mb, pc.ram_mb);
        for kind in [HfKind::Text, HfKind::Image] {
            let (a, next) = hf_page(kind, "", None).unwrap();
            let (b, _) = hf_page(kind, "", next.as_deref()).unwrap();
            assert!(!a.is_empty() && !b.is_empty() && a[0].id != b[0].id, "Seite 2 anders als Seite 1");
            let usable: Vec<_> = a.iter().chain(&b).filter(|r| if kind == HfKind::Text { best_gguf(r).is_some() } else { hf_image(r).is_some() }).collect();
            eprintln!("{kind:?}: {} + {} Repos, nutzbar {}: {:?}", a.len(), b.len(), usable.len(), usable.iter().take(5).map(|r| (r.id.as_str(), best_gguf(r).or_else(|| hf_image(r).and_then(|x| x.1.first().cloned())))).collect::<Vec<_>>());
            if let Some(r) = usable.first() {
                let f = hf_files(&r.id, kind).unwrap();
                eprintln!("  Dateien {}: {:?}", r.id, f.iter().take(3).map(|(n, s)| (n.as_str(), s / 1_000_000, fit_text(s / 1_000_000))).collect::<Vec<_>>());
            }
        }
        let (c, cn) = civitai_page("", None).unwrap();
        let (c2, _) = civitai_page("", cn.as_deref()).unwrap();
        let _ = (c, c2);
        let (cq, _) = civitai_page("juggernaut", None).unwrap();
        eprintln!("Civitai „juggernaut“: {:?}", cq.iter().take(3).map(|m| (m.name.as_str(), m.base.as_str(), m.file.as_str())).collect::<Vec<_>>());
        // Bauarten: wie viele laufen jetzt?
        let mut civ = Vec::new();
        let mut next = None;
        for _ in 0..5 {
            let (v, n) = civitai_page("", next.as_deref()).unwrap();
            civ.extend(v);
            next = n;
        }
        let bad: Vec<_> = civ.iter().filter(|m| !m.usable()).map(|m| m.base.as_str()).collect();
        eprintln!("Civitai 100: nutzbar {} – nicht: {:?}", civ.iter().filter(|m| m.usable()).count(), bad);
        for q in ["qwen", "flux", "z-image", "chroma", "krea"] {
            let (v, _) = hf_page(HfKind::Image, q, None).unwrap();
            let ok: Vec<_> = v.iter().filter_map(|r| hf_image(r).map(|(f, files)| (r.id.clone(), f.label(), files.len()))).take(3).collect();
            eprintln!("HF Bild „{q}“: {} von {} nutzbar, z. B. {:?}", v.iter().filter(|r| hf_image(r).is_some()).count(), v.len(), ok);
        }
        let (q, _) = hf_page(HfKind::Image, "anime", None).unwrap();
        eprintln!("Bild „anime“: {:?}", q.iter().filter(|r| hf_image(r).is_some()).take(4).map(|r| &r.id).collect::<Vec<_>>());
    }

    /// Bild verstehen mit einem lokalen Vision-Modell: FOLDEX_VISION_IMG=<bild> cargo test real_vision -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_vision() {
        let Ok(img) = std::env::var("FOLDEX_VISION_IMG") else { return };
        let (stem, path, _) = installed_text().into_iter().find(|t| mmproj_for(&t.1).is_some()).expect("kein Vision-Modell installiert");
        eprintln!("Modell: {stem} · Projektor: {:?}", mmproj_for(&path));
        let t = Instant::now();
        prepare(BASE, &stem).unwrap();
        eprintln!("Server bereit nach {:?}", t.elapsed());
        let b = crate::ai::image_b64(Path::new(&img)).unwrap();
        let entry = crate::ai::with_images(serde_json::json!({ "role": "user", "content": "Beschreibe dieses Bild in einem Satz auf Deutsch." }), crate::ai::Kind::OpenAi, &[b]);
        let body = serde_json::json!({ "model": stem, "messages": [entry], "max_tokens": 400 });
        let r = ureq::post(&format!("{BASE}/chat/completions")).set("content-type", "application/json").send_string(&body.to_string());
        match r {
            Ok(r) => {
                let v: Value = serde_json::from_str(&r.into_string().unwrap()).unwrap();
                eprintln!("Antwort nach {:?}: {:?}", t.elapsed(), v.pointer("/choices/0/message/content"));
            }
            Err(ureq::Error::Status(c, r)) => eprintln!("HTTP {c}: {}", r.into_string().unwrap_or_default()),
            Err(e) => eprintln!("Fehler: {e}"),
        }
        stop_server();
    }

    /// angefangenes CUDA-Paket für Bilder fertig laden: cargo test finish_sd_cuda -- --ignored --nocapture
    #[test]
    #[ignore]
    fn finish_sd_cuda() {
        install_runtime(&SD_CUDA);
        let t = Instant::now();
        while exe(&SD_CUDA).is_none() && t.elapsed() < Duration::from_secs(1500) {
            if let Some(d) = downloads().first() {
                eprintln!("{} {:.0} % {}", d.0, d.1 * 100.0, d.2);
            }
            for (err, m) in take_log() {
                eprintln!("Log: {m}");
                assert!(!err, "{m}");
            }
            std::thread::sleep(Duration::from_secs(5));
        }
        assert!(exe(&SD_CUDA).is_some(), "CUDA-Paket nicht vollständig");
        eprintln!("fertig: {:?}", sd_exe());
    }

    /// echtes Bild mit dem ersten installierten Bildmodell: cargo test real_image -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_image() {
        let c = image_choices();
        let (name, choice, _, size) = c.first().expect("kein Bildmodell installiert").clone();
        let (_, steps, cfg, _) = choice_settings(&choice);
        let req = GenReq { model: choice, prompt: "a cozy cabin in snowy mountains at dusk, warm light, photo".into(), negative: String::new(), w: size, h: size, steps, cfg, seed: 42, count: 1 };
        let dir = std::env::temp_dir().join("foldex_real_image");
        let t = Instant::now();
        let files = generate(&req, &dir, &Progress::default()).unwrap();
        let img = image::open(&files[0]).unwrap();
        eprintln!("{name}: {}x{} in {:?} → {}", img.width(), img.height(), t.elapsed(), files[0].display());
        assert_eq!(img.width(), size);
    }

    #[test]
    fn families() {
        assert_eq!(family_from_base("SDXL 1.0"), Some(Family::Sd));
        assert_eq!(family_from_base("Illustrious"), Some(Family::Sd));
        assert_eq!(family_from_base("Flux.1 D"), Some(Family::Flux));
        assert_eq!(family_from_base("ZImageTurbo"), Some(Family::ZImage));
        assert_eq!(family_from_base("Krea 2"), Some(Family::Krea2));
        assert_eq!(family_from_base("Hunyuan Video"), None);
        assert_eq!(family_from_text("unsloth/Qwen-Image-2.1-Turbo-GGUF"), Some(Family::Qwen21));
        assert_eq!(family_from_text("cagliostrolab/animagine-xl-4.0"), None);
        assert!(is_model_file("diffusion_models/flux1-dev-fp8.safetensors", Family::Flux));
        assert!(!is_model_file("text_encoders/t5xxl_fp16.safetensors", Family::Flux));
        assert!(!is_model_file("vae/ae.safetensors", Family::Flux));
        assert!(is_fast("qwen-image-2.1-turbo-Q4_K_M.gguf") && !is_fast("juggernautXL_v9.safetensors"));
        let files = vec![("x-F16.gguf".to_string(), 16_000), ("x-Q4_K_M.gguf".to_string(), 5_000), ("x-Q8_0.gguf".to_string(), 9_000)];
        assert_eq!(pick_model_file(&files, Family::Qwen21).as_deref(), Some("x-Q4_K_M.gguf"));
        // Bauart-Zusatzdateien zählen nicht als eigene Modelle; alle Schalter vorhanden
        for f in Family::ALL {
            assert!(f.companions().iter().all(|(r, _)| r.flag().starts_with('-')));
        }
    }

    #[test]
    fn progress_and_parts() {
        assert_eq!(step_of("  |==========>         | 3/8 - 1.20s/it"), Some((3, 8)));
        assert_eq!(step_of("loading model"), None);
        assert_eq!(split_parts("Q4/model-Q4_K_M-00001-of-00003.gguf").as_deref(), Some("Q4/model-Q4_K_M-"));
        assert_eq!(split_parts("model.gguf"), None);
        assert_eq!(urlenc("qwen 3 coder"), "qwen%203%20coder");
        assert!(is_builtin("http://127.0.0.1:39281/v1/"));
        // der Text-Kodierer von Z-Image liegt bei den Textmodellen (auch im Chat nutzbar)
        let z = image_model("z-image-turbo").unwrap();
        let llm = z.files.iter().find(|f| f.0 == Role::Llm).unwrap();
        assert!(image_file_path(Role::Llm, &llm.1).starts_with(text_dir()));
    }
}
