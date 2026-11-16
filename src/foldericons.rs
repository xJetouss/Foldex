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
