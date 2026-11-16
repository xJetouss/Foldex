//! Ein Fenster-Bereich (Pane): Ordnerinhalt, Auswahl, Verlauf, Suchansicht.
//! Ordner werden IMMER in einem Hintergrund-Thread geladen und sortiert – die Oberfläche blockiert nie,
//! auch nicht beim schnellen Wechseln zwischen Ordnern. Veraltete Ladevorgänge brechen sich selbst ab.

use crate::fsops::*;
use crate::index::Index;
use crate::live::{Live, LiveHit};
use crate::settings::Settings;
use eframe::egui;
use std::{
    cmp::Ordering,
    collections::HashMap,
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering::Relaxed},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

pub const F_HIDDEN: u8 = 1;
pub const F_SYSTEM: u8 = 2;
/// Cloud: nur online verfügbar (wird beim Öffnen geladen)
pub const F_ONLINE: u8 = 4;
/// Cloud: immer auf diesem Gerät behalten
pub const F_PINNED: u8 = 8;

pub struct Item {
    pub name: Box<str>,
    pub size: u64,
    pub mtime: i64,
    pub dir: bool,
    pub flags: u8,
    /// Gruppen-Rang (wird beim Sortieren berechnet, wenn nach Name/Datum/Größe gruppiert wird)
    pub grp: u8,
}

#[derive(PartialEq, Clone, Copy, Default)]
pub enum SortBy {
    #[default]
    Name,
    Date,
    Type,
    Size,
}

/// Lage der Vorschau innerhalb ihrer Kachel.
#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub enum PvSide {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

impl PvSide {
    pub const ALL: [PvSide; 4] = [PvSide::Left, PvSide::Right, PvSide::Top, PvSide::Bottom];
    pub fn key(self) -> &'static str {
        match self {
            PvSide::Left => "l",
            PvSide::Right => "r",
            PvSide::Top => "t",
            PvSide::Bottom => "b",
        }
    }
    pub fn from_key(k: &str) -> PvSide {
        PvSide::ALL.into_iter().find(|s| s.key() == k).unwrap_or_default()
    }
    pub fn label(self) -> &'static str {
        match self {
            PvSide::Left => "links",
            PvSide::Right => "rechts",
            PvSide::Top => "oben",
            PvSide::Bottom => "unten",
        }
    }
    /// Teilt die Vorschau die Kachel nebeneinander (links/rechts)?
    pub fn horizontal(self) -> bool {
        matches!(self, PvSide::Left | PvSide::Right)
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub enum ViewMode {
    #[default]
    Details,
    List,
    Grid,
    Large,
}

impl ViewMode {
    pub const ALL: [ViewMode; 4] = [ViewMode::Details, ViewMode::List, ViewMode::Grid, ViewMode::Large];
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Details => "Details",
            ViewMode::List => "Liste",
            ViewMode::Grid => "Raster",
            ViewMode::Large => "Große Symbole",
        }
    }
    pub fn from_u8(n: u8) -> Self {
        Self::ALL.get(n as usize).copied().unwrap_or_default()
    }
    pub fn to_u8(self) -> u8 {
        Self::ALL.iter().position(|&m| m == self).unwrap_or(0) as u8
    }
    /// Zellgröße (Breite, Höhe) und Icongröße im Raster; `None` bei Details/Liste.
    pub fn cell(self) -> Option<(f32, f32, f32)> {
        match self {
            ViewMode::Grid => Some((112.0, 104.0, 48.0)),
            ViewMode::Large => Some((156.0, 160.0, 96.0)),
            _ => None,
        }
    }
}

pub const LIST_ROW_H: f32 = 26.0;
pub const LIST_COL_W: f32 = 250.0;

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub enum GroupBy {
    #[default]
    None,
    Name,
    Date,
    Type,
    Size,
}

impl GroupBy {
    pub const ALL: [GroupBy; 5] = [GroupBy::None, GroupBy::Name, GroupBy::Date, GroupBy::Type, GroupBy::Size];
    pub fn label(self) -> &'static str {
        match self {
            GroupBy::None => "Keine",
            GroupBy::Name => "Name",
            GroupBy::Date => "Änderungsdatum",
            GroupBy::Type => "Typ",
            GroupBy::Size => "Größe",
        }
    }
}

// ───────────── Filter ─────────────

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub enum Cat {
    #[default]
    All,
    Folders,
    Documents,
    Images,
    Audio,
    Video,
    Archives,
    Code,
    Apps,
}

impl Cat {
    pub const ALL: [Cat; 9] = [
        Cat::All,
        Cat::Folders,
        Cat::Documents,
        Cat::Images,
        Cat::Audio,
        Cat::Video,
        Cat::Archives,
        Cat::Code,
        Cat::Apps,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Cat::All => "Alle Elemente",
            Cat::Folders => "Ordner",
            Cat::Documents => "Dokumente",
            Cat::Images => "Bilder",
            Cat::Audio => "Audio",
            Cat::Video => "Videos",
            Cat::Archives => "Archive",
            Cat::Code => "Code & Konfiguration",
            Cat::Apps => "Programme",
        }
    }
}

pub fn cat_of(ext: &str) -> Cat {
    match ext {
        "txt" | "md" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx" | "ods" | "csv" | "ppt" | "pptx" | "odp" | "epub" | "log" => Cat::Documents,
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tif" | "tiff" | "heic" | "avif" | "raw" | "psd" => Cat::Images,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" => Cat::Audio,
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" => Cat::Video,
        "zip" | "rar" | "7z" | "tar" | "gz" | "xz" | "bz2" | "zst" | "tgz" | "iso" | "cab" => Cat::Archives,
        "rs" | "py" | "js" | "mjs" | "jsx" | "ts" | "tsx" | "html" | "htm" | "css" | "scss" | "xml" | "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "c" | "h" | "cpp" | "hpp" | "cs" | "java" | "go" | "kt" | "swift" | "php" | "rb" | "sql" | "sh" | "ps1" | "lua" => Cat::Code,
        "exe" | "msi" | "bat" | "cmd" | "com" | "scr" | "appimage" | "dll" | "so" => Cat::Apps,
        _ => Cat::All,
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub enum DateF {
    #[default]
    All,
    Today,
    Week,
    Month,
    Year,
}

impl DateF {
    pub const ALL: [DateF; 5] = [DateF::All, DateF::Today, DateF::Week, DateF::Month, DateF::Year];
    pub fn label(self) -> &'static str {
        match self {
            DateF::All => "Beliebig",
            DateF::Today => "Heute",
            DateF::Week => "Letzte 7 Tage",
            DateF::Month => "Letzte 30 Tage",
            DateF::Year => "Letzte 12 Monate",
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub enum SizeF {
    #[default]
    All,
    Small,
    Medium,
    Large,
}

impl SizeF {
    pub const ALL: [SizeF; 4] = [SizeF::All, SizeF::Small, SizeF::Medium, SizeF::Large];
    pub fn label(self) -> &'static str {
        match self {
            SizeF::All => "Beliebig",
            SizeF::Small => "Unter 1 MB",
            SizeF::Medium => "1 – 100 MB",
            SizeF::Large => "Über 100 MB",
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub struct Filter {
    pub cat: Cat,
    pub date: DateF,
    pub size: SizeF,
}

impl Filter {
    pub fn active(&self) -> bool {
        *self != Filter::default()
    }

    fn pass(&self, it: &Item, now: &Now) -> bool {
        match self.cat {
            Cat::All => {}
            Cat::Folders => {
                if !it.dir {
                    return false;
                }
            }
            c => {
                if it.dir || cat_of(ext_buf(&it.name).as_str()) != c {
                    return false;
                }
            }
        }
        if self.date != DateF::All {
            if it.mtime == NO_TIME {
                return false;
            }
            let secs = it.mtime.div_euclid(1_000_000_000);
            let ok = match self.date {
                DateF::Today => day_num(secs) == Some(now.days),
                DateF::Week => now.secs - secs <= 7 * 86_400,
                DateF::Month => now.secs - secs <= 30 * 86_400,
                _ => now.secs - secs <= 365 * 86_400,
            };
            if !ok {
                return false;
            }
        }
        if !it.dir {
            const MB: u64 = 1 << 20;
            let ok = match self.size {
                SizeF::All => true,
                SizeF::Small => it.size < MB,
                SizeF::Medium => (MB..=100 * MB).contains(&it.size),
                SizeF::Large => it.size > 100 * MB,
            };
            if !ok {
                return false;
            }
        }
        true
    }
}

// ───────────── Datum/Gruppen ─────────────

struct Now {
    secs: i64,
    days: i32,
    dow: i32,
    year: i32,
    month: u32,
}

fn day_num(secs: i64) -> Option<i32> {
    use chrono::Datelike;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).map(|d| d.with_timezone(&chrono::Local).date_naive().num_days_from_ce())
}

fn now_info() -> Now {
    use chrono::{Datelike, Local};
    let n = Local::now();
    Now {
        secs: n.timestamp(),
        days: n.date_naive().num_days_from_ce(),
        dow: n.weekday().num_days_from_monday() as i32,
        year: n.year(),
        month: n.month(),
    }
}

/// Rang einer Zeit: größer = neuer.
fn date_rank(ns: i64, now: &Now) -> u8 {
    use chrono::Datelike;
    if ns == NO_TIME {
        return 0;
    }
    let secs = ns.div_euclid(1_000_000_000);
    let Some(d) = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).map(|d| d.with_timezone(&chrono::Local)) else {
        return 0;
    };
    let age = now.days - d.date_naive().num_days_from_ce();
    let (y, m) = (d.year(), d.month());
    if age < 0 {
        10
    } else if age == 0 {
        9
    } else if age == 1 {
        8
    } else if age <= now.dow {
        7
    } else if age <= now.dow + 7 {
        6
    } else if (y, m) == (now.year, now.month) {
        5
    } else if (y, m) == if now.month == 1 { (now.year - 1, 12) } else { (now.year, now.month - 1) } {
        4
    } else if y == now.year {
        3
    } else if y == now.year - 1 {
        2
    } else {
        1
    }
}

pub fn date_label(rank: u8) -> &'static str {
    match rank {
        10 => "In der Zukunft",
        9 => "Heute",
        8 => "Gestern",
        7 => "Diese Woche",
        6 => "Letzte Woche",
        5 => "Diesen Monat",
        4 => "Letzten Monat",
        3 => "Früher in diesem Jahr",
        2 => "Letztes Jahr",
        1 => "Vor langer Zeit",
        _ => "Unbekannt",
    }
}

fn size_rank(it: &Item) -> u8 {
    const KB: u64 = 1024;
    const MB: u64 = KB * KB;
    if it.dir {
        0
    } else if it.size == 0 {
        1
    } else if it.size <= 10 * KB {
        2
    } else if it.size <= 100 * KB {
        3
    } else if it.size <= MB {
        4
    } else if it.size <= 16 * MB {
        5
    } else if it.size <= 128 * MB {
        6
    } else {
        7
    }
}

pub fn size_label(rank: u8) -> &'static str {
    match rank {
        0 => "Ordner",
        1 => "Leer (0 KB)",
        2 => "Winzig (bis 10 KB)",
        3 => "Klein (10 – 100 KB)",
        4 => "Mittel (100 KB – 1 MB)",
        5 => "Groß (1 – 16 MB)",
        6 => "Sehr groß (16 – 128 MB)",
        _ => "Riesig (über 128 MB)",
    }
}

fn name_rank(name: &str) -> u8 {
    let Some(c) = name.chars().next() else { return 0 };
    if c.is_ascii_digit() {
        return 1;
    }
    let c = c.to_lowercase().next().unwrap_or(c);
    let c = match c {
        'ä' | 'à' | 'á' | 'â' => 'a',
        'ö' | 'ò' | 'ó' | 'ô' => 'o',
        'ü' | 'ù' | 'ú' | 'û' => 'u',
        'é' | 'è' | 'ê' => 'e',
        'ß' => 's',
        c => c,
    };
    if c.is_ascii_lowercase() {
        2 + (c as u8 - b'a')
    } else {
        0
    }
}

fn name_label(rank: u8) -> String {
    match rank {
        0 => "Sonstige".into(),
        1 => "0 – 9".into(),
        r => ((b'A' + r - 2) as char).to_string(),
    }
}

fn assign_groups(items: &mut [Item], g: GroupBy) {
    let now = now_info();
    for it in items {
        it.grp = match g {
            GroupBy::Name => name_rank(&it.name),
            GroupBy::Date => date_rank(it.mtime, &now),
            GroupBy::Size => size_rank(it),
            _ => 0,
        };
    }
}

fn trim_zeros(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|&&c| c == b'0').count();
    &b[n..]
}

/// „Natürliche“ Sortierung wie im Windows-Explorer: Groß-/Kleinschreibung egal, Zahlen nach Wert
/// (datei2 < datei10). Ohne Speicherzuweisung, weil sie bei 100 000 Dateien Millionen Mal läuft.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < x.len() && j < y.len() {
        if x[i].is_ascii_digit() && y[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < x.len() && x[i].is_ascii_digit() {
                i += 1;
            }
            while j < y.len() && y[j].is_ascii_digit() {
                j += 1;
            }
            let (na, nb) = (trim_zeros(&x[si..i]), trim_zeros(&y[sj..j]));
            let o = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
            if o != Ordering::Equal {
                return o;
            }
        } else {
            let (c, d) = (x[i].to_ascii_lowercase(), y[j].to_ascii_lowercase());
            if c != d {
                return c.cmp(&d);
            }
            i += 1;
            j += 1;
        }
    }
    (x.len() - i).cmp(&(y.len() - j)).then_with(|| a.cmp(b))
}

fn ext_ref(name: &str) -> &str {
    match name.rfind('.') {
        Some(p) if p > 0 && p + 1 < name.len() => &name[p + 1..],
        _ => "",
    }
}

/// Typ-Vergleich: Ordner zuerst, dann nach Endung (Ordnernamen mit Punkt zählen nicht als Endung).
fn type_cmp(a: &Item, b: &Item) -> Ordering {
    b.dir.cmp(&a.dir).then_with(|| {
        if a.dir {
            Ordering::Equal
        } else {
            ext_ref(&a.name)
                .bytes()
                .map(|c| c.to_ascii_lowercase())
                .cmp(ext_ref(&b.name).bytes().map(|c| c.to_ascii_lowercase()))
        }
    })
}

fn grp_cmp(a: &Item, b: &Item, g: GroupBy) -> Ordering {
    match g {
        GroupBy::None => Ordering::Equal,
        GroupBy::Type => type_cmp(a, b),
        _ => a.grp.cmp(&b.grp),
    }
}

/// Totale, reproduzierbare Ordnung:
/// 1. (optional) Gruppe, 2. bei Name/Typ optional Ordner zuerst, 3. gewählte Spalte, 4. Name.
///
/// Bei Datum und Größe werden Ordner und Dateien NICHT getrennt – sonst stünde z. B. ein Ordner von vor
/// zehn Tagen über einer Datei von heute und die Datumsspalte wirkte unsortiert.
/// Die Zeit wird auf die Nanosekunde genau verglichen.
pub fn sort_items(items: &mut [Item], sort: SortBy, asc: bool, folders_first: bool, group: GroupBy) {
    if matches!(group, GroupBy::Name | GroupBy::Date | GroupBy::Size) {
        assign_groups(items, group);
    }
    let ff = folders_first && matches!(sort, SortBy::Name | SortBy::Type);
    items.sort_unstable_by(|a, b| {
        if group != GroupBy::None {
            let o = grp_cmp(a, b, group);
            let o = if asc { o } else { o.reverse() };
            if o != Ordering::Equal {
                return o;
            }
        }
        if ff && a.dir != b.dir {
            return b.dir.cmp(&a.dir);
        }
        let o = match sort {
            SortBy::Name => natural_cmp(&a.name, &b.name),
            SortBy::Date => a.mtime.cmp(&b.mtime),
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Type => type_cmp(a, b),
        };
        (if asc { o } else { o.reverse() }).then_with(|| natural_cmp(&a.name, &b.name))
    });
}

fn group_label(it: &Item, g: GroupBy) -> String {
    match g {
        GroupBy::None => String::new(),
        GroupBy::Name => name_label(it.grp),
        GroupBy::Date => date_label(it.grp).into(),
        GroupBy::Size => size_label(it.grp).into(),
        GroupBy::Type => crate::icons::type_name(ext_buf(&it.name).as_str(), it.dir),
    }
}

#[cfg(windows)]
fn flags_of(_name: &str, md: &fs::Metadata) -> u8 {
    use std::os::windows::fs::MetadataExt;
    let a = md.file_attributes();
    let cloud = match crate::cloud::sync_of(a) {
        crate::cloud::Sync::Online => F_ONLINE,
        crate::cloud::Sync::Pinned => F_PINNED,
        crate::cloud::Sync::Local => 0,
    };
    (if a & 0x2 != 0 { F_HIDDEN } else { 0 }) | (if a & 0x4 != 0 { F_SYSTEM } else { 0 }) | cloud
}
#[cfg(not(windows))]
fn flags_of(name: &str, _md: &fs::Metadata) -> u8 {
    if name.starts_with('.') {
        F_HIDDEN
    } else {
        0
    }
}

pub struct RenameState {
    pub orig: String,
    pub buf: String,
    pub first: bool,
}

pub enum Hit {
    /// (Position in `SearchView::idxs`, Eintrags-ID)
    Idx(u16, u32),
    Live(Box<LiveHit>),
}

#[derive(PartialEq, Clone, Copy)]
pub enum SearchKind {
    Folder,
    Pc,
}

pub struct SearchView {
    pub kind: SearchKind,
    /// Schnappschuss der benutzten Indizes – bleibt gültig, auch wenn im Hintergrund ein neuer Index gebaut wird.
    pub idxs: Vec<Arc<Index>>,
    pub hits: Vec<(i32, Hit)>,
    pub total: usize,
    pub capped: bool,
    pub ms: f64,
    pub live: Option<Live>,
    /// Ausgewählte Treffer als Pfade – bleibt richtig, auch wenn neue Treffer einsortiert werden
    pub sel: HashSet<PathBuf>,
    /// Ankerzeile für Umschalt+Klick
    pub anchor: Option<PathBuf>,
    pub label: String,
}

type Loaded = (usize, Vec<Item>, Option<String>);

#[derive(Default)]
pub struct Pane {
    pub cwd: PathBuf,
    /// Terminal oder Git-Ansicht statt Dateien (Kachel bleibt andock- und verschiebbar)
    pub extra: Option<crate::PaneView>,
    /// Weitere Tabs in derselben Kachel (nicht sichtbar). Reihenfolge der Tab-Leiste der Kachel:
    /// stack[..stack_pos], diese Pane, stack[stack_pos..]
    pub stack: Vec<Pane>,
    pub stack_pos: usize,
    pub items: Vec<Item>,
    pub view: Vec<usize>,
    pub back: Vec<PathBuf>,
    pub fwd: Vec<PathBuf>,
    pub sort: SortBy,
    pub asc: bool,
    pub folders_first: bool,
    pub mode: ViewMode,
    /// Zeigt statt des Ordnerinhalts den Papierkorb
    pub trash: bool,
    /// Zeigt die Startseite (Kacheln) statt des Ordners
    pub start: bool,
    /// Startseite: Einstellungen statt Kacheln
    pub start_settings: bool,
    /// Papierkorb eines Cloud-Kontos statt des Ordners
    pub ctrash: Option<Box<crate::cloudtrash::TrashView>>,
    pub group: GroupBy,
    pub filter: Filter,
    pub err: Option<String>,
    // Auswahl
    pub sel: HashSet<String>,
    pub cursor: Option<usize>,
    pub anchor: Option<usize>,
    pub sel_size: u64,
    pub sel_dirty: bool,
    pub rename: Option<RenameState>,
    pub typeahead: (String, Option<Instant>),
    // Adressleiste / Suche
    pub crumb_edit: Option<String>,
    pub crumb_focus: bool,
    pub query: String,
    pub focus_search: bool,
    pub search: Option<SearchView>,
    // Laden
    gen: Arc<AtomicUsize>,
    load_rx: Option<mpsc::Receiver<Loaded>>,
    pub loading_since: Option<Instant>,
    // Anzeige
    pub row_cache: HashMap<usize, (Box<str>, Box<str>)>,
    pub pending_scroll: Option<f32>,
    pub pending_scroll_x: Option<f32>,
    pub scroll_y: f32,
    pub scroll_x: f32,
    pub view_h: f32,
    pub view_w: f32,
    /// Zähler, der bei jeder Änderung von `view` steigt (Cache-Schlüssel für das Layout)
    pub view_ver: u64,
    pub layout: Layout,
    /// Spalten im Raster / Zeilen pro Spalte in der Liste (für die Pfeiltasten)
    pub cols: usize,
    pub rows_per_col: usize,
    // Vorschau: jede Kachel hat ihre eigene – an/aus, Lage und Größe unabhängig von den anderen
    pub preview: crate::preview::Preview,
    pub preview_on: bool,
    pub preview_side: PvSide,
    /// Anteil der Kachel (Breite bzw. Höhe), 0 = Standard
    pub preview_frac: f32,
}

/// Eine Zeile der virtuellen Liste: Gruppenkopf oder Elemente `from..to` (Positionen in `Pane::view`).
pub struct LRow {
    pub y: f32,
    pub h: f32,
    pub from: usize,
    pub to: usize,
    /// Gruppenkopf: (Titel, erste Position, Position hinter der letzten)
    pub head: Option<(Box<str>, usize, usize)>,
}

#[derive(Default)]
pub struct Layout {
    key: Option<(u64, usize, GroupBy, u32, u32)>,
    pub rows: Vec<LRow>,
    pub total: f32,
}

impl Pane {
    pub fn new(cwd: PathBuf, ctx: &egui::Context, st: &Settings) -> Self {
        let mut p = Pane {
            asc: true,
            folders_first: st.folders_first,
            mode: ViewMode::from_u8(st.view_mode),
            cols: 1,
            rows_per_col: 1,
            preview_on: st.show_preview,
            ..Default::default()
        };
        p.go(cwd, false, ctx, st);
        p
    }

    /// Anteil der Vorschau an der Kachel (Standard 36 %).
    pub fn preview_share(&self) -> f32 {
        if self.preview_frac <= 0.0 {
            0.36
        } else {
            self.preview_frac.clamp(0.12, 0.85)
        }
    }

    pub fn duplicate(&self, ctx: &egui::Context, st: &Settings) -> Self {
        let mut p = Pane {
            asc: self.asc,
            sort: self.sort,
            folders_first: self.folders_first,
            mode: self.mode,
            trash: false,
            start: false,
            start_settings: false,
            ctrash: None,
            group: self.group,
            filter: self.filter,
            cols: 1,
            rows_per_col: 1,
            preview_on: self.preview_on,
            preview_side: self.preview_side,
            preview_frac: self.preview_frac,
            ..Default::default()
        };
        p.go(self.cwd.clone(), false, ctx, st);
        p
    }

    pub fn title(&self) -> String {
        if self.trash {
            return "Papierkorb".into();
        }
        if let Some(t) = &self.ctrash {
            return format!("Papierkorb – {}", t.name);
        }
        if self.start {
            return if self.start_settings { "Startseite anpassen".into() } else { "Start".into() };
        }
        self.cwd
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.cwd.display().to_string())
    }

    pub fn go(&mut self, p: PathBuf, hist: bool, ctx: &egui::Context, st: &Settings) {
        if hist && p != self.cwd && !self.cwd.as_os_str().is_empty() {
            let old = std::mem::replace(&mut self.cwd, p);
            self.back.push(old);
            self.fwd.clear();
        } else {
            self.cwd = p;
        }
        self.query.clear();
        self.search = None;
        self.sel.clear();
        self.sel_dirty = true;
        self.cursor = None;
        self.anchor = None;
        self.rename = None;
        self.crumb_edit = None;
        self.err = None;
        self.items.clear();
        self.view.clear();
        // Wichtig: Layout verwerfen, sonst zeigen die zwischengespeicherten Zeilen noch auf Positionen
        // des vorherigen Ordners und der Zugriff auf `view[pos]` stürzt ab.
        self.view_ver += 1;
        self.layout = Layout::default();
        self.trash = false;
        self.start = false;
        self.start_settings = false;
        self.ctrash = None;
        self.row_cache.clear();
        self.pending_scroll = Some(0.0);
        self.start_load(ctx);
        let _ = st;
    }

    /// Startet das Laden im Hintergrund. Ein neuer Aufruf macht alle laufenden Ladevorgänge ungültig.
    pub fn start_load(&mut self, ctx: &egui::Context) {
        let g = self.gen.fetch_add(1, Relaxed) + 1;
        let (tx, rx) = mpsc::channel();
        self.load_rx = Some(rx);
        self.loading_since = Some(Instant::now());
        let (gen, path, sort, asc, ff, group, ctx) = (
            self.gen.clone(),
            self.cwd.clone(),
            self.sort,
            self.asc,
            self.folders_first,
            self.group,
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let mut items = Vec::new();
            let mut err = None;
            match fs::read_dir(&path) {
                Ok(rd) => {
                    for (n, e) in rd.flatten().enumerate() {
                        if n % 128 == 0 && gen.load(Relaxed) != g {
                            return; // veraltet → sofort aufgeben
                        }
                        let Ok(md) = e.metadata() else { continue };
                        let name = e.file_name().to_string_lossy().into_owned();
                        let dir = md.is_dir() || (md.file_type().is_symlink() && e.path().is_dir());
                        items.push(Item {
                            flags: flags_of(&name, &md),
                            name: name.into_boxed_str(),
                            size: if dir { 0 } else { md.len() },
                            mtime: mtime_ns(md.modified().ok()),
                            dir,
                            grp: 0,
                        });
                    }
                }
                Err(e) => {
                    err = Some(if e.kind() == std::io::ErrorKind::NotFound {
                        "Pfad nicht gefunden".to_string()
                    } else {
                        e.to_string()
                    })
                }
            }
            crate::fsx::note_listing(&path, items.iter().map(|i| (&*i.name, i.dir, i.size)));
            sort_items(&mut items, sort, asc, ff, group);
            items.shrink_to_fit();
            if gen.load(Relaxed) == g {
                let _ = tx.send((g, items, err));
                ctx.request_repaint();
            }
        });
    }

    /// Nimmt fertig geladene Daten entgegen und holt Live-Suchtreffer ab. `true` = Anzeige hat sich geändert.
    pub fn poll(&mut self, st: &Settings, ctx: &egui::Context) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.load_rx {
            match rx.try_recv() {
                Ok((g, items, err)) => {
                    if g == self.gen.load(Relaxed) {
                        self.items = items;
                        self.err = err;
                        self.loading_since = None;
                        self.row_cache.clear();
                        self.load_rx = None;
                        self.refresh_view(st);
                        changed = true;
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.load_rx = None;
                    self.loading_since = None;
                }
                Err(_) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        if let Some(sv) = &mut self.search {
            if let Some(live) = &sv.live {
                let mut got = false;
                while let Ok(batch) = live.rx.try_recv() {
                    sv.total += batch.len();
                    sv.hits
                        .extend(batch.into_iter().map(|(s, h)| (s, Hit::Live(Box::new(h)))));
                    got = true;
                }
                if got {
                    sv.hits.sort_by(|a, b| b.0.cmp(&a.0));
                    sv.hits.truncate(5000);
                    changed = true;
                }
                if !live.done.load(Relaxed) {
                    ctx.request_repaint_after(Duration::from_millis(120));
                } else if !got {
                    sv.live = None;
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn is_loading(&self) -> bool {
        self.load_rx.is_some()
    }

    pub fn resort(&mut self, st: &Settings) {
        self.folders_first = st.folders_first;
        sort_items(&mut self.items, self.sort, self.asc, self.folders_first, self.group);
        self.row_cache.clear();
        self.refresh_view(st);
    }

    pub fn refresh_view(&mut self, st: &Settings) {
        let q = if self.search.is_none() {
            self.query.trim().to_lowercase()
        } else {
            String::new()
        };
        let filter = self.filter;
        let now = now_info();
        self.view = (0..self.items.len())
            .filter(|&i| {
                let it = &self.items[i];
                (st.show_hidden || it.flags & F_HIDDEN == 0)
                    && (st.show_system || it.flags & F_SYSTEM == 0)
                    && (q.is_empty() || it.name.to_lowercase().contains(&q))
                    && (!filter.active() || filter.pass(it, &now))
            })
            .collect();
        self.view_ver += 1;
        self.row_cache.clear();
        // Auswahl, die nicht mehr sichtbar ist (z. B. durch Filter), wird verworfen
        if self.filter.active() && !self.sel.is_empty() {
            let vis: HashSet<&str> = self.view.iter().map(|&i| &*self.items[i].name).collect();
            self.sel.retain(|n| vis.contains(n.as_str()));
            self.sel_dirty = true;
        }
        self.cursor = None;
        self.anchor = None;
    }

    /// (Neu-)berechnet die Zeilen der virtuellen Liste. `cols` = Elemente pro Zeile, `cell_h` = Zeilenhöhe.
    pub fn build_layout(&mut self, cols: usize, cell_h: f32, head_h: f32) {
        let cols = cols.max(1);
        let key = (self.view_ver, cols, self.group, cell_h.to_bits(), head_h.to_bits());
        if self.layout.key == Some(key) {
            return;
        }
        let mut rows = Vec::new();
        let mut y = 0.0f32;
        let n = self.view.len();
        let g = self.group;
        let mut i = 0;
        while i < n {
            let mut end = n;
            if g != GroupBy::None {
                let first = &self.items[self.view[i]];
                end = i + 1;
                while end < n && grp_cmp(first, &self.items[self.view[end]], g) == Ordering::Equal {
                    end += 1;
                }
                rows.push(LRow {
                    y,
                    h: head_h,
                    from: i,
                    to: i,
                    head: Some((group_label(first, g).into_boxed_str(), i, end)),
                });
                y += head_h;
            }
            let mut k = i;
            while k < end {
                let to = (k + cols).min(end);
                rows.push(LRow { y, h: cell_h, from: k, to, head: None });
                y += cell_h;
                k = to;
            }
            i = end;
        }
        self.layout = Layout { key: Some(key), rows, total: y };
    }

    /// Layout sofort mit den zuletzt benutzten Maßen neu aufbauen (damit `ensure_visible` direkt greift).
    pub fn layout_now(&mut self) {
        let (cols, h) = match self.mode.cell() {
            Some((_, h, _)) => (self.cols.max(1), h),
            None => (1, 30.0),
        };
        self.build_layout(cols, h, 34.0);
    }

    pub fn select_range(&mut self, from: usize, to: usize, add: bool) {
        if !add {
            self.sel.clear();
        }
        for p in from..to.min(self.view.len()) {
            self.sel.insert(self.items[self.view[p]].name.to_string());
        }
        self.cursor = Some(from);
        self.anchor = Some(from);
        self.sel_dirty = true;
    }

    /// Datum- und Typ-Text einer sichtbaren Zeile – wird nur einmal pro Zeile berechnet.
    pub fn row_strings(&mut self, idx: usize) -> &(Box<str>, Box<str>) {
        let items = &self.items;
        self.row_cache.entry(idx).or_insert_with(|| {
            let it = &items[idx];
            (
                fmt_time(it.mtime).into_boxed_str(),
                crate::icons::type_name(&ext_of(&it.name), it.dir).into_boxed_str(),
            )
        })
    }

    pub fn sel_paths(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.sel.iter().map(|n| self.cwd.join(n)).collect();
        v.sort();
        v
    }

    pub fn select_pos(&mut self, pos: usize, shift: bool, ctrl: bool) {
        if pos >= self.view.len() {
            return;
        }
        let name = self.items[self.view[pos]].name.to_string();
        if shift && self.anchor.is_some() {
            let a = self.anchor.unwrap().min(self.view.len() - 1);
            let (lo, hi) = (a.min(pos), a.max(pos));
            if !ctrl {
                self.sel.clear();
            }
            for p in lo..=hi {
                self.sel.insert(self.items[self.view[p]].name.to_string());
            }
        } else if ctrl {
            if !self.sel.remove(&name) {
                self.sel.insert(name);
            }
            self.anchor = Some(pos);
        } else {
            self.sel.clear();
            self.sel.insert(name);
            self.anchor = Some(pos);
        }
        self.cursor = Some(pos);
        self.sel_dirty = true;
    }

    pub fn select_name(&mut self, name: &str) {
        self.sel.clear();
        self.sel.insert(name.to_string());
        self.sel_dirty = true;
        if let Some(pos) = self.view.iter().position(|&i| &*self.items[i].name == name) {
            self.cursor = Some(pos);
            self.anchor = Some(pos);
        }
    }

    /// Scrollt so, dass die Position `pos` sichtbar ist (wirkt beim nächsten Zeichnen).
    pub fn ensure_visible(&mut self, pos: usize) {
        if self.mode == ViewMode::List {
            let col = pos / self.rows_per_col.max(1);
            let x = col as f32 * LIST_COL_W;
            if x < self.scroll_x {
                self.pending_scroll_x = Some(x);
            } else if x + LIST_COL_W > self.scroll_x + self.view_w {
                self.pending_scroll_x = Some(x + LIST_COL_W - self.view_w);
            }
            return;
        }
        let rows = &self.layout.rows;
        let i = rows.partition_point(|r| r.to <= pos);
        if let Some(r) = rows.get(i) {
            let (top, bottom) = (r.y, r.y + r.h);
            if top < self.scroll_y {
                self.pending_scroll = Some(top);
            } else if bottom > self.scroll_y + self.view_h {
                self.pending_scroll = Some(bottom - self.view_h);
            }
        }
    }

    pub fn refresh_sel_size(&mut self) {
        if self.sel_dirty {
            self.sel_dirty = false;
            self.sel_size = if self.sel.len() > 5000 {
                0
            } else {
                self.items
                    .iter()
                    .filter(|i| !i.dir && self.sel.contains(&*i.name))
                    .map(|i| i.size)
                    .sum()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(name: &str, dir: bool, size: u64, mtime: i64) -> Item {
        Item {
            name: name.into(),
            size,
            mtime,
            dir,
            flags: 0,
            grp: 0,
        }
    }
    fn names(v: &[Item]) -> Vec<&str> {
        v.iter().map(|i| &*i.name).collect()
    }
    const NG: GroupBy = GroupBy::None;

    #[test]
    fn natural_and_date_sort() {
        let mut v = vec![
            it("datei10.txt", false, 1, 0),
            it("Datei2.txt", false, 1, 0),
            it("datei1.txt", false, 1, 0),
            it("Ordner", true, 0, 0),
            it("a", false, 1, 0),
        ];
        sort_items(&mut v, SortBy::Name, true, true, NG);
        assert_eq!(names(&v), ["Ordner", "a", "datei1.txt", "Datei2.txt", "datei10.txt"]);
        sort_items(&mut v, SortBy::Name, false, true, NG);
        assert_eq!(names(&v), ["Ordner", "datei10.txt", "Datei2.txt", "datei1.txt", "a"]);
        sort_items(&mut v, SortBy::Name, true, false, NG);
        assert_eq!(names(&v), ["a", "datei1.txt", "Datei2.txt", "datei10.txt", "Ordner"]);

        // Dateien aus derselben Sekunde (Nanosekunden-Unterschied) müssen exakt sortiert werden
        let s = 1_700_000_000_000_000_000i64;
        let mut d = vec![
            it("c", false, 1, s + 900),
            it("a", false, 1, s + 100),
            it("b", false, 1, s + 500),
            it("neu", false, 1, s + 2_000_000_000),
            it("alt", false, 1, -5),
            it("none", false, 1, NO_TIME),
        ];
        sort_items(&mut d, SortBy::Date, true, true, NG);
        assert_eq!(names(&d), ["none", "alt", "a", "b", "c", "neu"]);
        sort_items(&mut d, SortBy::Date, false, true, NG);
        assert_eq!(names(&d), ["neu", "c", "b", "a", "alt", "none"]);

        // gleiche Zeit → Name als Tie-Break (stabil und reproduzierbar)
        let mut e = vec![it("b", false, 1, s), it("a", false, 1, s)];
        sort_items(&mut e, SortBy::Date, false, true, NG);
        assert_eq!(names(&e), ["a", "b"]);
    }

    /// Der gemeldete Fehler: Ordner standen bei der Datumssortierung immer oben, obwohl sie ÄLTER waren
    /// als die Dateien (16.10., 11.10., dann 26.10. …). Datum und Größe sortieren jetzt rein nach Wert.
    #[test]
    fn date_sort_mixes_folders_and_files() {
        let day = 86_400_000_000_000i64;
        let t = 1_790_000_000_000_000_000i64;
        let mut v = vec![
            it("ordner_alt", true, 0, t - 10 * day),
            it("datei_neu", false, 5, t),
            it("ordner_mittel", true, 0, t - 15 * day),
            it("datei_mittel", false, 5, t - day),
        ];
        sort_items(&mut v, SortBy::Date, false, true, NG);
        assert_eq!(names(&v), ["datei_neu", "datei_mittel", "ordner_alt", "ordner_mittel"]);
        sort_items(&mut v, SortBy::Date, true, true, NG);
        assert_eq!(names(&v), ["ordner_mittel", "ordner_alt", "datei_mittel", "datei_neu"]);
        sort_items(&mut v, SortBy::Size, false, true, NG);
        assert_eq!(&*v[0].name, "datei_mittel"); // Größe 5 vor Ordnern (0)
    }

    #[test]
    fn type_sort_ignores_dots_in_folder_names() {
        let mut v = vec![
            it("b.txt", false, 1, 0),
            it("mein.ordner", true, 0, 0),
            it("a.rs", false, 1, 0),
            it("z", true, 0, 0),
        ];
        sort_items(&mut v, SortBy::Type, true, true, NG);
        assert_eq!(names(&v), ["mein.ordner", "z", "a.rs", "b.txt"]);
    }

    #[test]
    fn grouping_orders_groups_then_column() {
        let mut v = vec![
            it("zebra", false, 1, 0),
            it("Apfel", false, 1, 0),
            it("Ärger", false, 1, 0),
            it("1abc", false, 1, 0),
            it("birne", false, 1, 0),
        ];
        sort_items(&mut v, SortBy::Name, true, true, GroupBy::Name);
        assert_eq!(names(&v), ["1abc", "Apfel", "Ärger", "birne", "zebra"]);
        assert_eq!(group_label(&v[1], GroupBy::Name), "A");
        assert_eq!(group_label(&v[2], GroupBy::Name), "A");
        assert_eq!(group_label(&v[0], GroupBy::Name), "0 – 9");
        // Größe
        let mut s = vec![it("gross", false, 50 << 20, 0), it("klein", false, 50 << 10, 0), it("dir", true, 0, 0)];
        sort_items(&mut s, SortBy::Name, true, true, GroupBy::Size);
        assert_eq!(names(&s), ["dir", "klein", "gross"]);
        sort_items(&mut s, SortBy::Name, false, true, GroupBy::Size);
        assert_eq!(names(&s), ["gross", "klein", "dir"]);
    }

    #[test]
    fn date_groups() {
        let now = now_info();
        let ns = |secs: i64| secs * 1_000_000_000;
        assert_eq!(date_rank(ns(now.secs - 60), &now), 9);
        assert_eq!(date_rank(NO_TIME, &now), 0);
        assert_eq!(date_rank(ns(now.secs - 400 * 86_400 * 3), &now), 1);
        assert!(date_rank(ns(now.secs - 86_400 * 2), &now) < 9);
    }

    #[test]
    fn filters() {
        let now = now_info();
        let img = it("a.PNG", false, 10, ns_now(&now));
        let doc = it("b.pdf", false, 200 << 20, 1);
        let dir = it("ordner", true, 0, ns_now(&now));
        let f = Filter { cat: Cat::Images, ..Default::default() };
        assert!(f.pass(&img, &now) && !f.pass(&doc, &now) && !f.pass(&dir, &now));
        let f = Filter { cat: Cat::Folders, ..Default::default() };
        assert!(f.pass(&dir, &now) && !f.pass(&img, &now));
        let f = Filter { size: SizeF::Large, ..Default::default() };
        assert!(f.pass(&doc, &now) && !f.pass(&img, &now));
        let f = Filter { date: DateF::Today, ..Default::default() };
        assert!(f.pass(&img, &now) && !f.pass(&doc, &now));
        assert!(!Filter::default().active());
    }

    /// Regression: Beim Ordnerwechsel blieb das alte Layout stehen und zeigte auf Positionen, die es nicht
    /// mehr gab (→ Absturz beim Öffnen eines Ordners).
    #[test]
    fn go_resets_layout() {
        let st = Settings::default();
        let ctx = egui::Context::default();
        let mut p = Pane::new(std::env::temp_dir(), &ctx, &st);
        p.items = (0..50).map(|i| it(&format!("f{i}"), false, 1, 0)).collect();
        p.refresh_view(&st);
        p.build_layout(1, 30.0, 34.0);
        assert!(!p.layout.rows.is_empty());
        p.go(std::env::temp_dir().join("."), true, &ctx, &st);
        assert!(p.layout.rows.is_empty() && p.view.is_empty());
        p.build_layout(1, 30.0, 34.0);
        assert!(p.layout.rows.is_empty());
    }

    fn ns_now(n: &Now) -> i64 {
        n.secs * 1_000_000_000
    }
}
