//! Eigene Ordner-Icons: je Ordner ein Bild vom PC oder eine Vorlage aus den eingebauten Ordnersymbolen.
//! Gespeichert in `…/foldex/ordner-icons.txt` (eine Zeile je Ordner: Pfad|preset:Name bzw. Pfad|bild:Datei).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spec {
    /// eingebaute Vorlage (Name eines Material-Ordnersymbols, z. B. „folder-images“)
    Preset(String),
    /// eigenes Bild (PNG, JPEG, ICO, SVG …)
    Image(PathBuf),
    /// klassischer Ordner in einer Farbe (0xRRGGBB)
    Color(u32),
}

impl Spec {
    fn encode(&self) -> String {
        match self {
            Spec::Preset(n) => format!("preset:{n}"),
            Spec::Image(p) => format!("bild:{}", p.display()),
            Spec::Color(c) => format!("farbe:#{c:06x}"),
        }
    }
    fn decode(s: &str) -> Option<Spec> {
        if let Some(n) = s.strip_prefix("preset:") {
            Some(Spec::Preset(n.to_string()))
        } else if let Some(c) = s.strip_prefix("farbe:#") {
            u32::from_str_radix(c, 16).ok().map(Spec::Color)
        } else {
            s.strip_prefix("bild:").map(|p| Spec::Image(PathBuf::from(p)))
        }
    }
}

fn file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("ordner-icons.txt")
}

/// Schlüssel: Pfad klein geschrieben, ohne Schrägstrich am Ende (Windows unterscheidet nicht)
fn key(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

fn map() -> &'static Mutex<Option<HashMap<String, (PathBuf, Spec)>>> {
    static M: Mutex<Option<HashMap<String, (PathBuf, Spec)>>> = Mutex::new(None);
    &M
}

fn with<R>(f: impl FnOnce(&mut HashMap<String, (PathBuf, Spec)>) -> R) -> R {
    let mut g = map().lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(|| {
        std::fs::read_to_string(file())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let (p, s) = l.split_once('|')?;
                let p = PathBuf::from(p);
                Some((key(&p), (p, Spec::decode(s)?)))
            })
            .collect()
    });
    f(m)
}

/// gar keine eigenen Icons? (dann muss beim Zeichnen nichts nachgeschlagen werden)
pub fn is_empty() -> bool {
    with(|m| m.is_empty())
}

pub fn get(p: &Path) -> Option<Spec> {
    with(|m| m.get(&key(p)).map(|e| e.1.clone()))
}

/// Icon setzen (None = wieder das normale)
pub fn set(p: &Path, spec: Option<Spec>) {
    with(|m| {
        match spec {
            Some(s) => {
                m.insert(key(p), (p.to_path_buf(), s));
            }
            None => {
                m.remove(&key(p));
            }
        }
        let text: Vec<String> = m.values().map(|(p, s)| format!("{}|{}", p.display(), s.encode())).collect();
        if let Some(d) = file().parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(file(), text.join("\n"));
    });
}

/// Name einer Vorlage für die Anzeige: „vscode-icons:folder_type_images“ → „images“
pub fn preset_label(name: &str) -> String {
    let n = name.rsplit(':').next().unwrap_or(name);
    let n = ["folder_type_", "file_type_", "folder-", "folder_", "_folder", "default_"].iter().fold(n, |n, p| n.strip_prefix(p).unwrap_or(n));
    n.trim_start_matches(['-', '_']).replace(['-', '_'], " ")
}

/// Farben für farbige Ordner
pub const COLORS: &[u32] = &[
    0xF5B935, 0xF08A24, 0xE5484D, 0xE93D82, 0xA855F7, 0x6E56CF, 0x3E63DD, 0x0091FF, 0x05A2C2, 0x12A594, 0x30A46C, 0x84CC16, 0x8D6E63, 0x6B7280, 0x1F2937, 0xE2E8F0,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_roundtrip() {
        for s in [Spec::Preset("folder-images".into()), Spec::Image(PathBuf::from(r"C:\Bilder\icon.png")), Spec::Color(0x3e63dd)] {
            assert_eq!(Spec::decode(&s.encode()), Some(s));
        }
        assert_eq!(key(Path::new(r"C:\Users\A\")), key(Path::new("c:/users/a")));
        assert_eq!(preset_label("folder-node_modules"), "node modules");
        assert_eq!(preset_label("vscode-icons:folder_type_images"), "images");
    }
}

/// Vorschläge passend zum Inhalt: Dateitypen, typische Unterordner und Projektdateien zählen, dazu der Name.
/// Ergebnis: Namen von Material-Ordnersymbolen, bester zuerst (der Aufrufer filtert auf vorhandene).
pub fn suggest(dir: &Path) -> Vec<String> {
    let mut score: HashMap<&'static str, f32> = HashMap::new();
    let mut add = |k: &'static str, v: f32| *score.entry(k).or_default() += v;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let by_name = crate::matico::folder_icon(&name);
    if by_name != "folder" && !by_name.is_empty() {
        add(by_name, 6.0);
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files = 0f32;
    let mut cnt: HashMap<&'static str, f32> = HashMap::new();
    for e in rd.flatten().take(600) {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            match n.as_str() {
                ".git" => add("folder-git", 8.0),
                ".github" => add("folder-github", 3.0),
                "node_modules" => add("folder-node", 5.0),
                "src" => add("folder-src", 2.0),
                "test" | "tests" => add("folder-test", 1.0),
                "target" => add("folder-rust", 2.0),
                "venv" | ".venv" | "__pycache__" => add("folder-python", 4.0),
                ".vscode" => add("folder-vscode", 1.0),
                _ => {}
            }
            continue;
        }
        files += 1.0;
        match n.as_str() {
            "cargo.toml" => add("folder-rust", 8.0),
            "package.json" => add("folder-javascript", 6.0),
            "tsconfig.json" => add("folder-typescript", 6.0),
            "pyproject.toml" | "requirements.txt" | "setup.py" => add("folder-python", 7.0),
            "go.mod" => add("folder-go", 7.0),
            "dockerfile" | "docker-compose.yml" | "compose.yaml" => add("folder-docker", 4.0),
            "pom.xml" | "build.gradle" => add("folder-java", 6.0),
            "pubspec.yaml" => add("folder-flutter", 7.0),
            _ => {}
        }
        let ext = n.rsplit_once('.').map_or("", |x| x.1);
        let k = match ext {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "bmp" | "tif" | "tiff" | "raw" | "cr2" | "nef" | "arw" | "dng" | "avif" => "folder-images",
            "svg" | "ico" => "folder-images",
            "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" => "folder-video",
            "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "wma" => "folder-audio",
            "pdf" => "folder-pdf",
            "doc" | "docx" | "odt" | "rtf" | "txt" | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "epub" => "folder-docs",
            "md" | "markdown" => "folder-markdown",
            "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "zst" => "folder-archive",
            "ttf" | "otf" | "woff" | "woff2" => "folder-font",
            "rs" => "folder-rust",
            "py" | "ipynb" => "folder-python",
            "js" | "mjs" | "cjs" | "jsx" => "folder-javascript",
            "ts" | "tsx" => "folder-typescript",
            "go" => "folder-go",
            "java" | "kt" => "folder-java",
            "c" | "cpp" | "h" | "hpp" | "cs" => "folder-src",
            "html" | "css" | "scss" => "folder-public",
            "sql" | "db" | "sqlite" => "folder-database",
            "log" => "folder-log",
            "exe" | "msi" | "dll" => "folder-dist",
            "sh" | "bat" | "ps1" | "cmd" => "folder-scripts",
            "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" => "folder-config",
            "bak" => "folder-backup",
            "obj" | "glb" | "gltf" | "fbx" | "blend" | "stl" => "folder-blender",
            "eml" | "msg" => "folder-mail",
            _ => continue,
        };
        *cnt.entry(k).or_default() += 1.0;
    }
    // Dateitypen zählen relativ zur Menge (ein Bild unter 300 Dateien ist kein Bilderordner)
    let files = files.max(1.0);
    for (k, c) in cnt {
        *score.entry(k).or_default() += c / files * 10.0;
    }
    let mut v: Vec<(&str, f32)> = score.into_iter().filter(|(_, s)| *s >= 0.8).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v.into_iter().take(10).map(|(k, _)| k.to_string()).collect()
}
