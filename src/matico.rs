//! Standard-Icons: das mitgelieferte „Material Icon Theme“ (Ordner `assets/icons`, SVG).
//! Die SVGs sind ins Programm eingebettet, werden bei Bedarf in der passenden Größe zu Texturen gerendert
//! und danach zwischengespeichert. Zuordnung: Dateiname → Dateiendung → Standard-Dokument-Icon;
//! Ordner nach Ordnername (z. B. `src`, `node_modules`, `Downloads`) → neutraler Ordner.
//!
//! Weitere Icon-Themes (beide MIT-Lizenz, Lizenztexte liegen daneben in `assets/icon-themes`):
//! - **Catppuccin Icons** (github.com/catppuccin/vscode-icons, Varianten Mocha/Latte)
//! - **vscode-icons** (github.com/vscode-icons/vscode-icons)
//!
//! Die Zuordnung erfolgt immer über die Material-Namen; für die anderen Themes werden diese übersetzt.

use eframe::egui;
use include_dir::{include_dir, Dir};
use std::{collections::HashMap, sync::OnceLock};

static DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/assets/icons");
static CAT_DARK: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/assets/icon-themes/catppuccin/mocha");
static CAT_LIGHT: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/assets/icon-themes/catppuccin/latte");
static VSI: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/assets/icon-themes/vscode-icons/icons");

/// Mitgelieferte Icon-Themes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Set {
    Material,
    Catppuccin,
    VsCode,
}

impl Set {
    pub const ALL: [Set; 3] = [Set::Material, Set::Catppuccin, Set::VsCode];

    pub fn key(self) -> &'static str {
        match self {
            Set::Material => "material",
            Set::Catppuccin => "catppuccin",
            Set::VsCode => "vscode-icons",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Set::Material => "Material",
            Set::Catppuccin => "Catppuccin",
            Set::VsCode => "vscode-icons",
        }
    }

    pub fn from_key(k: &str) -> Set {
        Set::ALL.into_iter().find(|s| s.key() == k).unwrap_or(Set::Material)
    }
}

fn dir_table(d: &'static Dir<'static>) -> HashMap<&'static str, &'static [u8]> {
    d.files().filter_map(|f| Some((f.path().file_stem()?.to_str()?, f.contents()))).collect()
}

fn set_table(set: Set, dark: bool) -> &'static HashMap<&'static str, &'static [u8]> {
    static CD: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    static CL: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    static V: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    match (set, dark) {
        (Set::Material, _) => table(),
        (Set::Catppuccin, true) => CD.get_or_init(|| dir_table(&CAT_DARK)),
        (Set::Catppuccin, false) => CL.get_or_init(|| dir_table(&CAT_LIGHT)),
        (Set::VsCode, _) => V.get_or_init(|| dir_table(&VSI)),
    }
}

/// Material-Name → Name im Catppuccin-Theme (nur wo er abweicht).
fn cat_alias(m: &str) -> &str {
    match m {
        "document" => "text",
        "word" => "ms-word",
        "table" => "ms-excel",
        "powerpoint" => "ms-powerpoint",
        "adobe-photoshop" => "adobe-ps",
        "adobe-illustrator" => "adobe-ai",
        "jar" => "java-jar",
        "disc" | "dll" => "binary",
        "console" => "bash",
        "python-misc" => "python-compiled",
        "react" => "javascript-react",
        "react_ts" => "typescript-react",
        "settings" | "hosts" => "config",
        "h" => "c-header",
        "hpp" => "cpp-header",
        "email" | "subtitles" => "text",
        "epub" => "pdf",
        "tex" => "latex",
        "webassembly" => "web-assembly",
        "nodejs" => "package-json",
        "npm" => "npm-lock",
        "yarn" => "yarn-lock",
        "pnpm" => "pnpm-lock",
        "tsconfig" => "typescript-config",
        "jsconfig" => "javascript-config",
        "tune" => "env",
        "gemfile" => "ruby-gem",
        "jenkins" => "workflow",
        "folder-base" => "_folder",
        "folder-resource" => "folder_assets",
        "folder-test" => "folder_tests",
        "folder-font" => "folder_fonts",
        "folder-plugin" => "folder_plugins",
        "folder-mock" => "folder_mocks",
        "folder-secure" => "folder_security",
        "folder-i18n" => "folder_locales",
        "folder-css" => "folder_styles",
        "folder-template" => "folder_templates",
        "folder-layout" => "folder_layouts",
        "folder-controller" => "folder_controllers",
        "folder-theme" => "folder_themes",
        "folder-github" => "folder_github",
        "folder-scripts" => "folder_scripts",
        "folder-archive" => "folder_packages",
        "folder-backup" => "folder_packages",
        other => other,
    }
}

/// Material-Name → Name bei vscode-icons (ohne „file_type_“ bzw. „folder_type_“).
fn vsi_alias(m: &str) -> &str {
    match m {
        "document" => "text",
        "table" => "excel",
        "adobe-photoshop" => "photoshop",
        "adobe-illustrator" => "ai",
        "disc" => "iso",
        "exe" | "dll" => "binary",
        "console" => "shell",
        "python-misc" => "python",
        "react" => "reactjs",
        "react_ts" => "reactts",
        "javascript" => "js",
        "typescript" => "typescript",
        "settings" | "hosts" => "config",
        "h" => "cheader",
        "hpp" => "cppheader",
        "database" => "db",
        "certificate" => "cert",
        "subtitles" => "text",
        "lock" => "lock",
        "webassembly" => "wasm",
        "proto" => "protobuf",
        "nodejs" => "node",
        "tune" => "dotenv",
        "go-mod" => "go_package",
        "gemfile" => "bundler",
        "favicon" => "favicon",
        "folder-resource" => "asset",
        "folder-node" => "node",
        "folder-images" => "images",
        "folder-test" => "test",
        "folder-temp" => "temp",
        "folder-scripts" => "script",
        "folder-database" => "db",
        "folder-utils" => "helper",
        "folder-lib" => "library",
        "folder-plugin" => "plugin",
        "folder-mock" => "mock",
        "folder-i18n" => "locale",
        "folder-css" => "style",
        "folder-views" => "view",
        "folder-controller" => "controller",
        "folder-store" => "redux",
        "folder-secure" => "private",
        other => other,
    }
}

/// SVG-Daten eines Icons im gewählten Theme: (Cache-Schlüssel, Daten). `m` ist der Material-Name
/// (aus `file_icon`/`folder_icon`), `ext` die Dateiendung für Themes mit eigenen Endungs-Icons.
pub fn icon_data(set: Set, dark: bool, m: &str, ext: &str, dir: bool) -> Option<(String, &'static [u8])> {
    let t = set_table(set, dark);
    let hit = |n: &str| t.get_key_value(n).map(|(k, v)| (format!("{}:{}", set.key(), k), *v));
    match set {
        Set::Material => hit(themed(known(m)?, dark)),
        Set::Catppuccin => {
            if dir {
                let a = cat_alias(m);
                hit(a).or_else(|| hit(&a.replacen("folder-", "folder_", 1))).or_else(|| hit("_folder"))
            } else {
                hit(cat_alias(m)).or_else(|| (!ext.is_empty()).then(|| hit(ext)).flatten()).or_else(|| hit("_file"))
            }
        }
        Set::VsCode => {
            let (pre, base) = if dir { ("folder_type_", vsi_alias(m).trim_start_matches("folder-")) } else { ("file_type_", vsi_alias(m)) };
            let named = |b: &str| {
                let light = (!dark).then(|| hit(&format!("{pre}light_{b}"))).flatten();
                light.or_else(|| hit(&format!("{pre}{b}")))
            };
            named(base).or_else(|| (!dir && !ext.is_empty()).then(|| named(ext)).flatten()).or_else(|| hit(if dir { "default_folder" } else { "default_file" }))
        }
    }
}

fn table() -> &'static HashMap<&'static str, &'static [u8]> {
    static T: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    T.get_or_init(|| {
        DIR.files()
            .filter_map(|f| Some((f.path().file_stem()?.to_str()?, f.contents())))
            .collect()
    })
}

fn known(n: &str) -> Option<&'static str> {
    table().get_key_value(n).map(|(k, _)| *k)
}

/// Helle Variante (`<name>_light`), falls es eine gibt und das Farbschema hell ist.
pub fn themed(name: &'static str, dark: bool) -> &'static str {
    if dark {
        return name;
    }
    known(&format!("{name}_light")).unwrap_or(name)
}

const SPECIAL: &[(&str, &str)] = &[
    ("cargo.toml", "toml"),
    ("cargo.lock", "lock"),
    ("dockerfile", "docker"),
    ("docker-compose.yml", "docker"),
    ("docker-compose.yaml", "docker"),
    (".dockerignore", "docker"),
    ("makefile", "makefile"),
    ("cmakelists.txt", "cmake"),
    ("readme", "readme"),
    ("readme.md", "readme"),
    ("readme.txt", "readme"),
    ("license", "license"),
    ("license.txt", "license"),
    ("license.md", "license"),
    ("changelog", "changelog"),
    ("changelog.md", "changelog"),
    ("contributing.md", "contributing"),
    ("authors", "authors"),
    (".gitignore", "git"),
    (".gitattributes", "git"),
    (".gitmodules", "git"),
    ("package.json", "nodejs"),
    ("package-lock.json", "npm"),
    ("yarn.lock", "yarn"),
    ("pnpm-lock.yaml", "pnpm"),
    ("tsconfig.json", "tsconfig"),
    ("jsconfig.json", "jsconfig"),
    (".env", "tune"),
    (".editorconfig", "editorconfig"),
    (".prettierrc", "prettier"),
    (".eslintrc", "eslint"),
    ("go.mod", "go-mod"),
    ("gemfile", "gemfile"),
    ("jenkinsfile", "jenkins"),
    ("robots.txt", "robots"),
    ("hosts", "hosts"),
];

/// Icon-Name für eine Datei (Endung in Kleinbuchstaben).
pub fn file_icon(name: &str, ext: &str) -> &'static str {
    for (n, i) in SPECIAL {
        if name.eq_ignore_ascii_case(n) {
            if let Some(k) = known(i) {
                return k;
            }
        }
    }
    let i = match ext {
        "txt" | "text" | "nfo" => "document",
        "md" | "markdown" => "markdown",
        "pdf" => "pdf",
        "doc" | "docx" | "dot" | "dotx" | "odt" | "rtf" => "word",
        "xls" | "xlsx" | "xlsm" | "ods" | "csv" | "tsv" => "table",
        "ppt" | "pptx" | "pps" | "ppsx" | "odp" => "powerpoint",
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" | "avif" | "raw" | "jfif" => "image",
        "ico" => "favicon",
        "psd" => "adobe-photoshop",
        "ai" => "adobe-illustrator",
        "svg" => "svg",
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" | "mpg" | "mpeg" => "video",
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" | "mid" | "midi" => "audio",
        "zip" | "rar" | "7z" | "tar" | "gz" | "xz" | "bz2" | "zst" | "tgz" | "cab" | "tbz2" | "txz" | "cbz" | "cbr" => "zip",
        "jar" | "war" => "jar",
        "iso" | "img" | "dmg" => "disc",
        "exe" | "msi" | "com" | "scr" | "appimage" | "apk" => "exe",
        "bat" | "cmd" | "sh" | "bash" | "zsh" | "fish" => "console",
        "ps1" | "psm1" => "powershell",
        "dll" | "so" | "dylib" | "sys" => "dll",
        "lib" | "a" => "lib",
        "rs" => "rust",
        "py" | "pyw" => "python",
        "pyc" | "pyd" => "python-misc",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "react",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "react_ts",
        "html" | "htm" | "xhtml" => "html",
        "css" => "css",
        "scss" | "sass" => "sass",
        "less" => "less",
        "xml" | "xsd" | "xsl" | "plist" => "xml",
        "json" | "jsonc" | "json5" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "ini" | "cfg" | "conf" | "config" | "properties" => "settings",
        "c" => "c",
        "h" => "h",
        "cpp" | "cc" | "cxx" | "c++" => "cpp",
        "hpp" | "hh" | "hxx" => "hpp",
        "cs" => "csharp",
        "java" | "class" => "java",
        "go" => "go",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "php" => "php",
        "rb" => "ruby",
        "sql" | "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" => "database",
        "ttf" | "otf" | "woff" | "woff2" | "eot" => "font",
        "log" => "log",
        "lnk" | "url" | "webloc" => "url",
        "vue" => "vue",
        "svelte" => "svelte",
        "lua" => "lua",
        "dart" => "dart",
        "pl" | "pm" => "perl",
        "ex" | "exs" => "elixir",
        "hs" => "haskell",
        "scala" => "scala",
        "zig" => "zig",
        "key" | "pem" | "ppk" | "pub" | "gpg" => "key",
        "crt" | "cer" | "der" | "p12" | "pfx" => "certificate",
        "eml" | "msg" | "mbox" => "email",
        "epub" | "mobi" | "azw3" => "epub",
        "srt" | "vtt" | "ass" | "sub" => "subtitles",
        "lock" => "lock",
        "diff" | "patch" => "diff",
        "tex" | "latex" => "tex",
        "ipynb" => "jupyter",
        "wasm" => "webassembly",
        "tf" | "tfvars" => "terraform",
        "proto" => "proto",
        "graphql" | "gql" => "graphql",
        "gd" => "godot",
        "r" => "r",
        "jl" => "julia",
        "m" => "matlab",
        "tmp" | "bak" | "old" => "document",
        _ => "",
    };
    if !i.is_empty() {
        if let Some(k) = known(i) {
            return k;
        }
    }
    known(ext).filter(|_| !ext.is_empty()).unwrap_or("document")
}

/// Icon-Name für einen Ordner (nach dessen Namen).
pub fn folder_icon(name: &str) -> &'static str {
    let l = name.to_ascii_lowercase();
    let alias = match l.as_str() {
        "node_modules" => "folder-node",
        "docs" | "doc" | "documentation" | "dokumente" | "documents" => "folder-docs",
        "images" | "img" | "imgs" | "pictures" | "bilder" | "photos" | "fotos" | "screenshots" => "folder-images",
        "videos" | "video" | "movies" | "filme" => "folder-video",
        "music" | "musik" | "audio" | "sounds" => "folder-audio",
        "downloads" | "download" => "folder-download",
        "desktop" => "folder-desktop",
        "src" | "source" | "sources" => "folder-src",
        "dist" | "build" | "out" | "output" | "bin" | "release" => "folder-dist",
        ".git" => "folder-git",
        ".github" => "folder-github",
        ".gitlab" => "folder-gitlab",
        ".vscode" => "folder-vscode",
        "test" | "tests" | "__tests__" | "spec" | "specs" => "folder-test",
        "assets" | "resources" | "res" => "folder-resource",
        "config" | "configs" | "conf" | ".config" | "settings" => "folder-config",
        "lib" | "libs" | "library" => "folder-lib",
        "temp" | "tmp" | ".tmp" | "cache" | ".cache" => "folder-temp",
        "public" | "www" | "static" => "folder-public",
        "scripts" | "script" => "folder-scripts",
        "backup" | "backups" | "bak" => "folder-backup",
        "logs" | "log" => "folder-log",
        "fonts" | "font" => "folder-font",
        "archive" | "archives" | "archiv" => "folder-archive",
        "database" | "db" | "data" | "sql" => "folder-database",
        "server" | "backend" => "folder-server",
        "client" | "frontend" => "folder-client",
        "utils" | "util" | "helpers" | "helper" => "folder-utils",
        "plugins" | "plugin" | "extensions" | "addons" => "folder-plugin",
        "tools" | "tool" => "folder-tools",
        "examples" | "example" | "samples" | "demo" | "demos" => "folder-examples",
        "mock" | "mocks" => "folder-mock",
        "shared" | "common" => "folder-shared",
        "security" | "secure" => "folder-secure",
        "trash" | "papierkorb" | "$recycle.bin" | ".trash" => "folder-trash",
        "i18n" | "locales" | "lang" | "languages" => "folder-i18n",
        "css" | "styles" | "style" => "folder-css",
        "scss" => "folder-sass",
        "md" => "folder-markdown",
        "views" => "folder-views",
        "templates" => "folder-template",
        "layouts" => "folder-layout",
        "controllers" => "folder-controller",
        "store" | "stores" => "folder-store",
        "themes" => "folder-theme",
        "includes" => "folder-include",
        "app" | "apps" => "folder-app",
        "home" => "folder-home",
        _ => "",
    };
    if let Some(k) = known(alias) {
        return k;
    }
    known(&format!("folder-{l}")).unwrap_or("folder-base")
}

/// Rendert ein SVG quadratisch in `px` × `px` Pixel (vormultipliziertes RGBA).
pub fn raster(name: &str, px: u32) -> Option<egui::ColorImage> {
    raster_data(table().get(name)?, px)
}

/// Gruppen von Vorlagen für eigene Icons: (Schlüssel, Beschriftung)
pub const PRESET_GROUPS: &[(&str, &str)] = &[
    ("material-ordner", "Ordner"),
    ("material-symbole", "Symbole"),
    ("catppuccin-ordner", "Ordner · Catppuccin"),
    ("catppuccin-symbole", "Symbole · Catppuccin"),
    ("vscode-ordner", "Ordner · VS Code"),
    ("vscode-symbole", "Symbole · VS Code"),
];

/// Vorlagen einer Gruppe als „Theme:Name“ (ohne helle, offene oder doppelte Varianten)
pub fn presets(group: &str) -> Vec<String> {
    let (set, prefix, folders): (Set, &str, bool) = match group {
        "material-ordner" => (Set::Material, "", true),
        "material-symbole" => (Set::Material, "", false),
        "catppuccin-ordner" => (Set::Catppuccin, "catppuccin:", true),
        "catppuccin-symbole" => (Set::Catppuccin, "catppuccin:", false),
        "vscode-ordner" => (Set::VsCode, "vscode-icons:", true),
        _ => (Set::VsCode, "vscode-icons:", false),
    };
    let t = set_table(set, true);
    let mut v: Vec<String> = t
        .keys()
        .copied()
        .filter(|k| {
            let is_folder = k.starts_with("folder") || k.starts_with("_folder") || k.starts_with("default_folder");
            is_folder == folders
                && !k.ends_with("_light")
                && !k.contains("_light_")
                && !k.starts_with("file_type_light")
                && !k.ends_with("-open")
                && !k.ends_with("_open")
                && !k.ends_with("_opened")
                && !k.contains("-open_")
        })
        .map(|k| format!("{prefix}{k}"))
        .collect();
    v.sort_unstable();
    v
}

/// SVG einer Vorlage (passend zum Farbschema); ohne Präfix = Material
pub fn preset_svg(name: &str, dark: bool) -> Option<&'static [u8]> {
    if let Some(n) = name.strip_prefix("catppuccin:") {
        return set_table(Set::Catppuccin, dark).get(n).copied();
    }
    if let Some(n) = name.strip_prefix("vscode-icons:") {
        let t = set_table(Set::VsCode, true);
        let light = (!dark).then(|| n.replacen("folder_type_", "folder_type_light_", 1).replacen("file_type_", "file_type_light_", 1));
        return light.and_then(|l| t.get(l.as_str()).copied()).or_else(|| t.get(n).copied());
    }
    table().get(themed(known(name)?, dark)).copied()
}

pub fn raster_data(data: &[u8], px: u32) -> Option<egui::ColorImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let sz = tree.size();
    let scale = px as f32 / sz.width().max(sz.height()).max(1.0);
    let tx = (px as f32 - sz.width() * scale) / 2.0;
    let ty = (px as f32 - sz.height() * scale) / 2.0;
    let mut pm = tiny_skia::Pixmap::new(px, px)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty),
        &mut pm.as_mut(),
    );
    Some(egui::ColorImage::from_rgba_premultiplied(
        [px as usize, px as usize],
        pm.data(),
    ))
}

/// Gängige Texturgrößen, damit nicht für jede Zoomstufe neu gerendert wird.
pub fn bucket(px: f32) -> u32 {
    const B: [u32; 10] = [16, 24, 32, 48, 64, 96, 128, 192, 256, 384];
    let n = px.ceil() as u32;
    B.iter().copied().find(|&b| b >= n).unwrap_or(384)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_and_raster() {
        assert!(table().len() > 800);
        assert_eq!(file_icon("main.rs", "rs"), "rust");
        assert_eq!(file_icon("Cargo.toml", "toml"), "toml");
        assert_eq!(file_icon("Dockerfile", ""), "docker");
        assert_eq!(file_icon("foo.unbekannt", "unbekannt"), "document");
        assert_eq!(file_icon("a.png", "png"), "image");
        assert_eq!(file_icon("a.php", "php"), "php");
        assert_eq!(folder_icon("src"), "folder-src");
        assert_eq!(folder_icon("Downloads"), "folder-download");
        assert_eq!(folder_icon("Irgendwas"), "folder-base");
        assert_eq!(folder_icon("rust"), "folder-rust");
        let img = raster("folder-base", 48).unwrap();
        assert!(img.pixels.iter().any(|c| c.a() > 0));
        // alle mitgelieferten Icons müssen renderbar sein
        for n in table().keys() {
            assert!(raster(n, 16).is_some(), "{n}");
        }
        // weitere Themes: Übersetzung der Namen und Rendern
        for set in [Set::Catppuccin, Set::VsCode] {
            for dark in [true, false] {
                for (m, ext, dir) in [("rust", "rs", false), ("document", "xyz", false), ("folder-src", "", true), ("folder-base", "", true), ("image", "png", false)] {
                    let (k, d) = icon_data(set, dark, m, ext, dir).unwrap_or_else(|| panic!("{set:?} {m}"));
                    assert!(raster_data(d, 24).is_some_and(|i| i.pixels.iter().any(|c| c.a() > 0)), "{k}");
                }
            }
            for d in set_table(set, true).values().chain(set_table(set, false).values()) {
                assert!(raster_data(d, 16).is_some());
            }
        }
        assert_eq!(icon_data(Set::Catppuccin, true, "rust", "rs", false).unwrap().0, "catppuccin:rust");
        assert_eq!(icon_data(Set::VsCode, true, "folder-src", "", true).unwrap().0, "vscode-icons:folder_type_src");
    }
}
