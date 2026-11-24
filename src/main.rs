#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod extract;
mod fsx;
mod startpage;
mod shortcut;
mod print;
mod uninstall;
mod update;
mod quicklook;
mod transcribe;
mod batch;
mod contentsearch;
mod skills;
mod aisetup;
mod cloudtrash;
mod matico;
mod foldericons;
mod summary;
mod cloud;
mod cloudlogin;
mod ops;
mod trashx;
mod fsops;
mod icons;
mod ipc;
mod tiling;
mod index;
mod live;
mod pane;
mod settings;
mod sidebar;
mod themes;
mod agents;
mod ai;
mod aiui;
mod quickbar;
mod clip;
mod docconv;
mod pdf;
mod pdfview;
mod codeedit;
mod ocr;
mod langs;
mod imgedit;
mod localai;
mod localllm;
mod localui;
mod stt;
mod audioedit;
mod edicons;
mod videoedit;
mod complete;
mod preview;
mod syntax;
mod session;
mod share;
mod localsend;
mod media;
mod convert;
mod locks;
mod dirsize;
mod editors;
mod git;
mod gitui;
mod keymap;
mod terminal;
mod tools;
mod ui;

use eframe::egui::{
    self, Pos2, pos2, vec2, Align2, Color32, FontId, Id, Key, Margin, Rect, Rounding, Sense, Stroke,
};
use fsops::*;
use icons::{type_name, IconKind, Icons};
use index::{Index, Query};
use ops::{Format, Hist, History, Op, Policy};
use pane::*;
use tiling::{Dir, Node, Zone};
use settings::Settings;
use sidebar::{probe_drives, probe_git, Entry, GitRepo, SidebarCfg};
use themes::Theme;
use keymap::{Keymap, Shortcut};
use ui::Ic;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

const ROW_H: f32 = 30.0;
const HEAD_H: f32 = 32.0;
const SIDE_ROW_H: f32 = 30.0;
const ICON: f32 = 22.0;

// ───────────────────────────── Theme (Windows-11-Look) ─────────────────────────────

fn apply_theme(ctx: &egui::Context, th: &Theme) {
    ctx.style_mut(|s| {
        let v = &mut s.visuals;
        v.panel_fill = th.bg;
        v.window_fill = th.panel;
        v.extreme_bg_color = th.field;
        v.faint_bg_color = th.panel;
        v.override_text_color = Some(th.text);
        v.hyperlink_color = th.accent;
        v.window_stroke = Stroke::new(1.0, th.line);
        v.window_rounding = Rounding::same(10.0);
        v.menu_rounding = Rounding::same(8.0);
        v.popup_shadow = egui::epaint::Shadow {
            offset: vec2(0.0, 6.0),
            blur: 22.0,
            spread: 0.0,
            color: Color32::from_black_alpha(if th.dark { 120 } else { 50 }),
        };
        v.window_shadow = v.popup_shadow;
        v.selection.bg_fill = th.accent.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, th.accent);
        v.text_cursor.stroke = Stroke::new(1.5, th.cursor);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.rounding = Rounding::same(6.0);
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, th.line);
        v.widgets.inactive.weak_bg_fill = th.field;
        v.widgets.inactive.bg_fill = th.field;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, th.line);
        v.widgets.hovered.weak_bg_fill = th.hover;
        v.widgets.hovered.bg_fill = th.hover;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, th.line);
        v.widgets.active.weak_bg_fill = th.press;
        v.widgets.active.bg_fill = th.press;
        v.widgets.open.weak_bg_fill = th.hover;
        v.widgets.open.bg_fill = th.hover;
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
            w.fg_stroke = Stroke::new(1.0, th.text);
        }
        s.spacing.item_spacing = vec2(8.0, 4.0);
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.menu_margin = Margin::same(6.0);
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating_width = 4.0;
        s.text_styles.insert(egui::TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Button, FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Small, FontId::proportional(12.0));
        s.interaction.selectable_labels = false;
    });
}

fn setup_fonts(ctx: &egui::Context) {
    #[cfg(windows)]
    if let Ok(bytes) = fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        let mut f = egui::FontDefinitions::default();
        f.font_data
            .insert("segoe".into(), egui::FontData::from_owned(bytes));
        if let Some(fam) = f.families.get_mut(&egui::FontFamily::Proportional) {
            fam.insert(0, "segoe".into());
        }
        ctx.set_fonts(f);
    }
    // Linux: die übliche Systemschrift der Distribution (sonst egui-Standardschrift)
    #[cfg(not(windows))]
    {
        const FONTS: [&str; 8] = [
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
            "/usr/share/fonts/ubuntu/Ubuntu-R.ttf",
            "/usr/share/fonts/cantarell/Cantarell-Regular.otf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ];
        if let Some(bytes) = FONTS.iter().find_map(|p| fs::read(p).ok()) {
            let mut f = egui::FontDefinitions::default();
            f.font_data.insert("system".into(), egui::FontData::from_owned(bytes));
            if let Some(fam) = f.families.get_mut(&egui::FontFamily::Proportional) {
                fam.insert(0, "system".into());
            }
            ctx.set_fonts(f);
        }
    }
}

/// Farbthema für den aktuellen Hell-/Dunkel-Modus.
fn themes_for(store: &themes::Store, st: &Settings, dark: bool) -> Theme {
    store.resolve(if dark { &st.theme_dark } else { &st.theme_light }, dark)
}

/// Eingeschaltete und gefundene Editoren: (Schlüssel, Name, Programm). Eigene Editoren: „custom:N“.
fn find_editors(st: &Settings) -> Vec<(String, String, PathBuf)> {
    let mut v: Vec<(String, String, PathBuf)> = editors::EDITORS
        .iter()
        .filter(|d| st.editor_on.get(d.key).copied().unwrap_or(d.default_on))
        .filter_map(|d| editors::resolve(d.key, st.editor_paths.get(d.key)).map(|p| (d.key.to_string(), d.name.to_string(), p)))
        .collect();
    for (i, (name, path)) in st.custom_editors.iter().enumerate() {
        let p = PathBuf::from(path);
        if p.is_file() {
            v.push((format!("custom:{i}"), name.clone(), p));
        }
    }
    v
}

/// Agenten suchen und ihre Versionen ermitteln (startet Prozesse → Hintergrund).
fn scan_agents(ctx: &egui::Context) -> mpsc::Receiver<(Vec<(String, String, String)>, HashMap<String, String>)> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let found = agents::detect();
        let versions = found.iter().filter_map(|(k, _, p)| agents::version(p).map(|v| (k.clone(), v))).collect();
        let _ = tx.send((found, versions));
        ctx.request_repaint();
    });
    rx
}

/// Ausgewählte Elemente einer Kachel – auch in Suchergebnissen.
fn pane_targets(pn: &Pane) -> Vec<PathBuf> {
    match &pn.search {
        Some(sv) if !sv.sel.is_empty() => (0..sv.hits.len()).filter_map(|i| hit_path(sv, i)).filter(|p| sv.sel.contains(p)).collect(),
        Some(_) => Vec::new(),
        None => pn.sel_paths(),
    }
}

/// Pfad des Treffers an Position `i` (None, wenn der Index inzwischen ungültig ist).
fn hit_path(sv: &SearchView, i: usize) -> Option<PathBuf> {
    match &sv.hits.get(i)?.1 {
        Hit::Idx(k, id) => sv.idxs.get(*k as usize).filter(|ix| (*id as usize) < ix.len()).map(|ix| ix.path(*id as usize)),
        Hit::Live(h) => Some(h.path.clone()),
    }
}

/// Wo nach Git-Repositories gesucht wird: Home + angeheftete Ordner.
fn git_roots(side: &SidebarCfg) -> Vec<PathBuf> {
    dirs::home_dir().into_iter().chain(side.pins.iter().map(|e| e.path.clone())).collect()
}

fn theme_pref(ctx: &egui::Context, t: u8) {
    let p = match t {
        1 => egui::ThemePreference::Light,
        2 => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    };
    ctx.set_theme(p);
}

// ───────────────────────────── Datenmodell ─────────────────────────────

#[derive(Clone, PartialEq)]
enum Dest {
    Dir(PathBuf),
    Pin,
    Trash,
    /// KI-Seitenleiste: Dateien an die nächste Nachricht hängen
    Ai,
}

enum Act {
    /// Audio → .txt (false) bzw. Video → .srt (true) mit Whisper
    Transcribe(Vec<PathBuf>, bool),
    /// mehrere Bilder bearbeiten (Fenster)
    BatchImages(Vec<PathBuf>),
    /// Inhaltssuche (PDFs, Bilder, Dokumente) im aktuellen Ordner, optional mit Anfrage
    ContentSearch(Option<String>),
    Go(PathBuf),
    OpenFile(PathBuf),
    Open(PathBuf),
    OpenSel,
    /// Schnellansicht (Leertaste): Datei in einem eigenen Vorschau-Tab öffnen
    Preview(PathBuf),
    Reveal(PathBuf),
    SetClip(Vec<PathBuf>, bool),
    PasteInto(PathBuf),
    TransferOther(Vec<PathBuf>, bool),
    SelectAll,
    Trash(Vec<PathBuf>),
    Rename,
    CommitRename,
    CancelRename,
    Refresh,
    Back,
    Forward,
    Up,
    NewFolder,
    NewFile,
    FocusAddr,
    FocusSearch,
    CopyText(String),
    Pin(PathBuf),
    Props(PathBuf),
    SortKey(SortBy),
    SortDir(bool),
    Search,
    ClearSearch,
    PcSearch,
    Extract(Vec<PathBuf>, bool),
    ReindexAll,
    Reindex(PathBuf),
    Unpin(usize),
    PinMove(usize, usize),
    SideRename(usize),
    TogglePc,
    RescanGit,
    ResetSidebar,
    TabNew,
    /// Mittelklick: Ordner in neuem Tab bzw. Datei – ihr Ordner in neuem Tab, Datei ausgewählt und in der Vorschau
    TabOpen(PathBuf),
    TabClose(usize),
    /// zuletzt geschlossenen Tab wieder öffnen
    TabReopen,
    TabSelect(usize),
    TabDup(usize),
    TabCloseOthers(usize),
    TabNext(bool),
    ToggleDual,
    SwitchPane,
    ShowSettings,
    Undo,
    Redo,
    Compress(Vec<PathBuf>, Format),
    DeletePerm(Vec<PathBuf>),
    SetView(ViewMode),
    SetGroup(GroupBy),
    SetFilter(Filter),
    SetFoldersFirst(bool),
    ShowTrash,
    ToggleSidebar,
    TabMove(usize, usize),
    TabDetach(usize, Pos2),
    PaneToTab(usize),
    ToggleOrient,
    Equalize,
    /// Befehl aus der Palette / Tastenkürzel (ID aus keymap::CMDS)
    Cmd(&'static str),
    /// neues Fenster, optional auf einem bestimmten Monitor (Arbeitsbereich in physischen Pixeln)
    NewWindow(Option<[i32; 4]>),
    /// Terminal-Tab: (Shell-Programm, None = Standard), Arbeitsordner
    Terminal(Option<String>, PathBuf),
    GitView(PathBuf),
    /// In Editor öffnen (Schlüssel aus find_editors)
    OpenIn(String, PathBuf),
    /// KI-Chat als neuer Tab: Ordner, Backend („agent:…“/„model:…“, None = Standard), Agent-Modus
    AiChat(PathBuf, Option<String>, bool),
    /// Explorer-KI-Seitenleiste ein/aus: Agent-Modus (true) oder Chat
    EditorAi(bool),
    /// reiner KI-Chat in der Seitenleiste (ohne Explorer und Dateien)
    EditorChat,
    /// Fenster „Bild erzeugen“ (lokales Bildmodell), speichert in diesen Ordner
    ImageGen(PathBuf),
    /// Fenster „Ordner-Icon ändern“
    FolderIcon(PathBuf),
    /// Zusammenfassung eines Ordners bzw. einer Textdatei
    Summary(PathBuf),
    /// Cloud: immer auf diesem Gerät behalten (true) bzw. Speicherplatz freigeben (false)
    CloudKeep(Vec<PathBuf>, bool),
    /// Fenster „Cloud hinzufügen“
    CloudSetup,
    /// Cloud-Fenster öffnen und gleich die Anmeldung dieses Anbieters starten
    CloudLogin(&'static str),
    /// Startseite: Kachel öffnen (Startseite verlassen)
    StartGo(PathBuf),
    /// Startseite: ganzen PC durchsuchen
    StartSearch(String),
    /// Startseite in neuem Tab
    StartOpen,
    /// Dateien drucken
    Print(Vec<PathBuf>),
    /// Verknüpfungen zu Programmen anlegen: (Ziele, auf dem Desktop statt im selben Ordner)
    Shortcut(Vec<PathBuf>, bool),
    /// Startseite im aktuellen Tab
    StartHere,
    /// Papierkorb eines Cloud-Dienstes: (Anbieter, Konto, Name)
    CloudTrash(&'static str, Option<String>, String),
    /// Cloud-Eintrag in der Seitenleiste verschieben (von, nach – Stellen der sichtbaren Einträge)
    CloudMove(usize, usize),
    /// Laufwerk bzw. Cloud-Eintrag aus der Seitenleiste entfernen (ausblenden)
    SideHide(PathBuf),
    /// ausgeblendete Einträge wieder zeigen
    SideUnhide,
    /// in Foldex angemeldetes Konto abmelden (rclone-Name)
    CloudLogout(String),
    /// Laufwerk trennen bzw. wieder verbinden, Anmeldung bleibt (rclone-Name)
    CloudConnect(String, bool),
    /// ffmpeg per winget installieren (sichtbar in einem Terminal-Tab)
    InstallFfmpeg,
    /// eigenes Teilen-Fenster (LocalSend, Uploads, Messenger)
    Share(Vec<PathBuf>),
    /// Datei in ein anderes Format umwandeln (neue Datei neben dem Original)
    Convert(PathBuf, String),
    /// mehrere Dateien in dasselbe Zielformat
    ConvertMany(Vec<PathBuf>, String),
    /// Agent installieren (Installationsbefehl läuft sichtbar in einem Terminal-Tab)
    InstallAgent(String),
    OpenWith(PathBuf),
    Tools(ToolTab),
    SelectRules,
    ClearCut,
    /// Tabs in einer Kachel: neuer Tab (Beschreibung wie pane_spec), wechseln, schließen
    SubNew(usize, PathBuf),
    SubSwitch(usize, usize),
    SubClose(usize, usize),
    /// Tab einer Kachel als eigenen Tab in der Tab-Leiste lösen
    SubToTab(usize, usize),
}

/// Besondere Kacheln ohne Dateiansicht (in Split-View, eigenem Tab oder anderem Fenster nutzbar).
enum PaneView {
    Term(terminal::Term),
    Git(gitui::GitView),
    Ai(aiui::ChatView),
    /// Schnellansicht (Leertaste): reine Vorschau einer Datei in einem eigenen Tab
    View(Box<ViewTab>),
}

/// Vorschau-Tab: zeigt genau eine Datei (Bild, Video, Audio, Dokument, Text mit Syntaxfarben) zum Ansehen.
struct ViewTab {
    path: PathBuf,
    pv: preview::Preview,
    /// Tab, der vor dem Öffnen aktiv war – beim Schließen kehrt Foldex dorthin zurück
    back: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolTab {
    Compare,
    Dups,
    Space,
    Large,
}

/// Fenster „Werkzeuge“: Eingaben, laufender Auftrag, Ergebnisse.
struct ToolsWin {
    tab: ToolTab,
    a: String,
    b: String,
    content: bool,
    min_size: String,
    task: Option<tools::Task>,
    result: Option<tools::Output>,
    /// Duplikate: zum Löschen markierte Dateien
    marked: HashSet<PathBuf>,
}

/// Dialog „Auswahl nach Regeln“.
struct RulesWin {
    kind: tools::RuleKind,
    value: String,
    mode: tools::SelMode,
    err: Option<String>,
}

/// Command Palette: Suchtext, markierte Zeile.
struct PaletteState {
    query: String,
    sel: usize,
    first: bool,
}

struct Props {
    path: PathBuf,
    name: String,
    kind: String,
    modified: String,
    created: String,
    is_dir: bool,
    size: Arc<AtomicU64>,
    files: Arc<AtomicU64>,
    done: Arc<AtomicBool>,
}

/// Geschlossener Tab (für Strg+Umschalt+T): alte Position und Zustand bzw. Datei der Schnellansicht.
enum ClosedTab {
    State(usize, session::TabState),
    View(usize, PathBuf),
}

struct Tab {
    id: u64,
    panes: Vec<Pane>,
    active: usize,
    /// Kachel-Layout: beliebig viele Teilfenster
    root: Node,
}

impl Tab {
    fn new(id: u64, pane: Pane) -> Tab {
        Tab { id, panes: vec![pane], active: 0, root: Node::Leaf(0) }
    }
    fn dual(&self) -> bool {
        self.panes.len() > 1
    }
    fn title(&self) -> String {
        pane_title(&self.panes[self.active.min(self.panes.len() - 1)])
    }
}

// ───────────── Tabs innerhalb einer Kachel ─────────────
// Die sichtbare Pane einer Kachel besitzt die übrigen Tabs (`stack`); dadurch bleibt das Kachel-Layout
// unverändert und Andocken/Verschieben nimmt die ganze Gruppe mit.

/// Alle Tabs einer Kachel in Anzeigereihenfolge als (Titel, Symbol, sichtbar?).
fn sub_tabs(p: &Pane) -> Vec<(String, Option<Ic>, bool)> {
    let pos = p.stack_pos.min(p.stack.len());
    let mut v: Vec<(String, Option<Ic>, bool)> = p.stack.iter().map(|s| (pane_title(s), pane_icon(s), false)).collect();
    v.insert(pos, (pane_title(p), pane_icon(p), true));
    v
}

/// Gruppe auflösen: alle Tabs in Anzeigereihenfolge (jeweils ohne eigenen Stack).
fn sub_flatten(slot: &mut Pane) -> Vec<Pane> {
    let mut d = std::mem::take(&mut slot.stack);
    let pos = slot.stack_pos.min(d.len());
    let mut vis = std::mem::take(slot);
    vis.stack_pos = 0;
    d.insert(pos, vis);
    d
}

/// Gruppe aus Tabs bilden; `vis` = sichtbarer Tab.
fn sub_group(mut d: Vec<Pane>, vis: usize) -> Pane {
    let vis = vis.min(d.len().saturating_sub(1));
    let mut p = d.remove(vis);
    p.stack = d;
    p.stack_pos = vis;
    p
}

fn sub_switch(slot: &mut Pane, k: usize) {
    if k == slot.stack_pos || k > slot.stack.len() {
        return;
    }
    let d = sub_flatten(slot);
    *slot = sub_group(d, k);
}

/// Tab `k` aus der Kachel herauslösen (nur wenn die Kachel mehr als einen Tab hat).
fn sub_take(slot: &mut Pane, k: usize) -> Option<Pane> {
    if slot.stack.is_empty() || k > slot.stack.len() {
        return None;
    }
    let pos = slot.stack_pos.min(slot.stack.len());
    let mut d = sub_flatten(slot);
    let out = d.remove(k);
    let vis = if k == pos { k.min(d.len() - 1) } else if k < pos { pos - 1 } else { pos };
    *slot = sub_group(d, vis);
    Some(out)
}

/// Tab (samt eventueller eigener Gruppe) in die Kachel aufnehmen; der neue Tab wird sichtbar.
fn sub_add(slot: &mut Pane, mut p: Pane) {
    let first_new_vis = p.stack_pos.min(p.stack.len());
    let incoming = sub_flatten(&mut p);
    let mut d = sub_flatten(slot);
    let vis = d.len() + first_new_vis;
    d.extend(incoming);
    *slot = sub_group(d, vis);
}

/// Tab `from` an Position `to` der Kachel verschieben; der verschobene Tab wird sichtbar.
fn sub_move(slot: &mut Pane, from: usize, to: usize) {
    let n = slot.stack.len() + 1;
    if from >= n {
        return;
    }
    let mut d = sub_flatten(slot);
    let p = d.remove(from);
    let to = to.min(d.len());
    d.insert(to, p);
    *slot = sub_group(d, to);
}

/// Jede Pane aller Tabs einschließlich der unsichtbaren Tabs in Kacheln.
fn each_pane(tabs: &mut [Tab], mut f: impl FnMut(&mut Pane)) {
    for t in tabs {
        for p in &mut t.panes {
            f(p);
            for s in &mut p.stack {
                f(s);
            }
        }
    }
}

fn pane_title(p: &Pane) -> String {
    match &p.extra {
        Some(PaneView::Term(t)) => t.title(),
        Some(PaneView::Git(g)) => g.title(),
        Some(PaneView::Ai(a)) => a.title(),
        Some(PaneView::View(v)) => v.path.file_name().map_or_else(|| "Vorschau".into(), |n| n.to_string_lossy().into_owned()),
        None => p.title(),
    }
}

fn pane_icon(p: &Pane) -> Option<Ic> {
    match &p.extra {
        Some(PaneView::Term(t)) if agents::agent_name(&t.program).is_some() => Some(Ic::Spark),
        Some(PaneView::Term(_)) => Some(Ic::Terminal),
        Some(PaneView::Git(_)) => Some(Ic::Git),
        Some(PaneView::Ai(_)) => Some(Ic::Spark),
        Some(PaneView::View(_)) => Some(Ic::Eye),
        None => None,
    }
}

/// KI-Kachel (Assistent oder Agent im Terminal)? – wird beim Ausschalten der KI geschlossen.
fn is_ai_pane(p: &Pane) -> bool {
    match &p.extra {
        Some(PaneView::Ai(_)) => true,
        Some(PaneView::Term(t)) => agents::agent_name(&t.program).is_some(),
        _ => false,
    }
}

// Kacheln über Fenstergrenzen hinweg: Ordner = Pfad; Terminal/Git = Beschreibung zum Neustarten.
const SPEC_TERM: &str = "fc-term:";
const SPEC_GIT: &str = "fc-git:";
/// Kachel mit mehreren Tabs: „fc-group:<sichtbar>|<Beschreibung>|<Beschreibung>…“
const SPEC_GROUP: &str = "fc-group:";
const SPEC_AI: &str = "fc-ai:";

fn pane_spec(p: &Pane) -> PathBuf {
    if !p.stack.is_empty() {
        let pos = p.stack_pos.min(p.stack.len());
        let mut parts: Vec<String> = p.stack.iter().map(|s| single_spec(s).to_string_lossy().into_owned()).collect();
        parts.insert(pos, single_spec(p).to_string_lossy().into_owned());
        return PathBuf::from(format!("{SPEC_GROUP}{pos}|{}", parts.join("|")));
    }
    single_spec(p)
}

fn single_spec(p: &Pane) -> PathBuf {
    match &p.extra {
        Some(PaneView::Term(t)) => PathBuf::from(format!("{SPEC_TERM}{}*{}", t.program, t.cwd.display())),
        Some(PaneView::Git(g)) => PathBuf::from(format!("{SPEC_GIT}{}", g.root.display())),
        Some(PaneView::Ai(a)) => {
            // Gespräch sichern, damit es im Zielfenster weiterläuft
            a.persist();
            PathBuf::from(format!("{SPEC_AI}{}*{}*{}*{}", if a.agent_mode { "agent" } else { "chat" }, a.backend, a.chat_id, a.cwd.display()))
        }
        // Schnellansicht ist flüchtig: in einem anderen Fenster erscheint der Ordner der Datei
        Some(PaneView::View(v)) => v.path.parent().map_or_else(|| p.cwd.clone(), Path::to_path_buf),
        None => p.cwd.clone(),
    }
}

/// Zustand eines Tabs wie in der Sitzung: Kacheln (als Beschreibung), Aufteilung, Vorschau je Kachel.
fn tab_state(t: &Tab) -> session::TabState {
    session::TabState {
        layout: t.root.serialize(),
        active: t.active,
        panes: t
            .panes
            .iter()
            .map(|p| session::PaneState {
                spec: pane_spec(p).to_string_lossy().into_owned(),
                preview: p.preview_on,
                preview_side: p.preview_side.key().into(),
                preview_frac: p.preview_frac,
            })
            .collect(),
    }
}

/// Tab aus einem gespeicherten Zustand neu aufbauen (Sitzung, wiederhergestellter Tab).
fn tab_from_state(t: &session::TabState, id: u64, ctx: &egui::Context, st: &Settings, shells: &[(String, String)]) -> Tab {
    let mut panes: Vec<Pane> = t
        .panes
        .iter()
        .map(|ps| {
            let mut p = pane_from_spec(Path::new(&ps.spec), ctx, st, shells);
            p.preview_on = ps.preview;
            p.preview_side = PvSide::from_key(&ps.preview_side);
            p.preview_frac = ps.preview_frac;
            p
        })
        .collect();
    if panes.is_empty() {
        panes.push(Pane::new(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")), ctx, st));
    }
    let root = Node::parse(&t.layout).filter(|n| n.valid_for(panes.len())).unwrap_or_else(|| Node::chain(panes.len(), Dir::H));
    Tab { id, active: t.active.min(panes.len() - 1), root, panes }
}

fn is_spec(s: &str) -> bool {
    s.starts_with(SPEC_TERM) || s.starts_with(SPEC_GIT) || s.starts_with(SPEC_GROUP) || s.starts_with(SPEC_AI)
}

fn shell_name(shells: &[(String, String)], prog: &str) -> String {
    if let Some(n) = agents::agent_name(prog) {
        return n.to_string();
    }
    shells
        .iter()
        .find(|s| s.1 == prog)
        .map(|s| s.0.clone())
        .unwrap_or_else(|| Path::new(prog).file_stem().map_or(prog.to_string(), |n| n.to_string_lossy().into_owned()))
}

/// Kachel aus einer Beschreibung (`pane_spec`) erzeugen – Terminals starten im gleichen Ordner neu.
fn pane_from_spec(spec: &Path, ctx: &egui::Context, st: &Settings, shells: &[(String, String)]) -> Pane {
    let s = spec.to_string_lossy();
    if let Some((pos, rest)) = s.strip_prefix(SPEC_GROUP).and_then(|r| r.split_once('|')) {
        let d: Vec<Pane> = rest.split('|').filter(|x| !x.is_empty()).map(|x| pane_from_spec(Path::new(x), ctx, st, shells)).collect();
        if !d.is_empty() {
            return sub_group(d, pos.parse().unwrap_or(0));
        }
    }
    if let Some((prog, dir)) = s.strip_prefix(SPEC_TERM).and_then(|r| r.split_once('*')) {
        let dir = PathBuf::from(dir);
        let mut p = Pane::new(dir.clone(), ctx, st);
        if agents::agent_name(prog).is_some() && !st.ai_enabled {
            return p;
        }
        if let Ok(t) = terminal::Term::spawn(&shell_name(shells, prog), prog, &dir, ctx) {
            p.extra = Some(PaneView::Term(t));
        }
        return p;
    }
    if let Some(rest) = s.strip_prefix(SPEC_AI) {
        // „agent|chat*<backend>*<gespräch>*<ordner>“ (ältere Formen: ohne Gespräch bzw. nur der Ordner)
        let f: Vec<&str> = rest.splitn(4, '*').collect();
        let (agent, backend, chat, dir) = match f.len() {
            4 => (f[0] == "agent", f[1].to_string(), Some(f[2]), f[3]),
            3 => (f[0] == "agent", f[1].to_string(), None, f[2]),
            _ => (false, st.ai_assist.clone(), None, rest),
        };
        let dir = PathBuf::from(dir);
        let mut p = Pane::new(dir.clone(), ctx, st);
        if st.ai_enabled {
            let c = chat.and_then(|id| aiui::ChatView::restore(id, dir.clone())).unwrap_or_else(|| aiui::ChatView::new(dir, backend, agent));
            p.extra = Some(PaneView::Ai(c));
        }
        return p;
    }
    if let Some(root) = s.strip_prefix(SPEC_GIT) {
        let root = PathBuf::from(root);
        let mut p = Pane::new(root.clone(), ctx, st);
        p.extra = Some(PaneView::Git(gitui::GitView::new(root, ctx)));
        return p;
    }
    Pane::new(spec.to_path_buf(), ctx, st)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum DragKind {
    /// ganzer Tab (Tab-ID)
    Tab(u64),
    /// einzelnes Teilfenster (Tab-ID, Pane-Nummer)
    Pane(u64, usize),
    /// ein Tab innerhalb einer Kachel (Tab-ID, Pane-Nummer, Position in der Kachel)
    Sub(u64, usize, usize),
}

/// Laufendes Ziehen eines Tabs oder Teilfensters.
struct TabDrag {
    kind: DragKind,
    title: String,
    grab_dx: f32,
    /// Zeigerposition im Fenster (kann außerhalb liegen)
    pos: Pos2,
    /// Position auf dem Bildschirm (Punkte dieses Fensters)
    screen: Option<Pos2>,
    /// Position auf dem Bildschirm in physischen Pixeln – gilt über alle Monitore und Skalierungen
    px: Option<Pos2>,
    /// Zeiger außerhalb des Fensters → Ablegen erzeugt/ergänzt ein anderes Fenster
    outside: bool,
    payload: ipc::Payload,
}

/// Übergabe an ein anderes Fenster, das noch bestätigen muss.
struct Handoff {
    seq: u64,
    kind: DragKind,
    deadline: Instant,
}

/// Startparameter (für Fenster, die durch Herausziehen eines Tabs entstehen).
#[derive(Default)]
struct Launch {
    paths: Vec<PathBuf>,
    pos: Option<[f32; 2]>,
    layout: Option<String>,
    /// Fensterrechteck in physischen Pixeln (Neues Fenster auf einem bestimmten Monitor)
    place: Option<[i32; 4]>,
    /// wiederhergestellte Sitzung (Tabs, Kacheln, Vorschau, Fensterlage)
    session: Option<session::WinState>,
    /// Fenster beim Start maximieren (gespeicherte Fensterlage)
    maximized: bool,
}

fn parse_args(args: &[String]) -> Launch {
    let mut l = Launch::default();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--open" => {
                if let Some(p) = it.next() {
                    let p = arg_path(p);
                    if is_spec(&p) || Path::new(&p).is_dir() {
                        l.paths.push(PathBuf::from(p));
                    }
                }
            }
            "--pos" => {
                if let Some((x, y)) = it.next().and_then(|s| s.split_once(',')) {
                    if let (Ok(x), Ok(y)) = (x.parse::<f32>(), y.parse::<f32>()) {
                        l.pos = Some([x, y]);
                    }
                }
            }
            "--layout" => l.layout = it.next().cloned(),
            "--session" => {
                if let Some(p) = it.next() {
                    l.session = session::load(Path::new(p));
                    let _ = fs::remove_file(p);
                }
            }
            "--place" => {
                let v: Vec<i32> = it.next().map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect()).unwrap_or_default();
                if v.len() == 4 && v[2] > 100 && v[3] > 100 {
                    l.place = Some([v[0], v[1], v[2], v[3]]);
                }
            }
            // Ordner ohne Schalter (z. B. „Öffnen mit“ oder per Drag & Drop auf foldex.exe)
            p if !p.starts_with("--") => {
                let p = arg_path(p);
                if Path::new(&p).is_dir() {
                    l.paths.push(PathBuf::from(p));
                }
            }
            _ => {}
        }
    }
    l.paths.truncate(12);
    l
}

/// Pfad aus der Befehlszeile reparieren: Explorer übergibt Laufwerke als `"C:\"` – nach den Windows-Regeln
/// wird der Backslash vor dem Anführungszeichen zu einem maskierten `"` und es kommt `C:"` an.
fn arg_path(p: &str) -> String {
    let mut s = p.trim().to_string();
    if s.ends_with('"') {
        s = s.trim_end_matches('"').to_string();
        if !s.ends_with('\\') && !s.ends_with('/') {
            s.push(if cfg!(windows) { '\\' } else { '/' });
        }
    }
    // „C:“ allein meint das Laufwerk, nicht den aktuellen Ordner darauf
    if cfg!(windows) && s.len() == 2 && s.ends_with(':') {
        s.push('\\');
    }
    s
}

struct DriveIndex {
    root: PathBuf,
    index: Option<Arc<Index>>,
}

struct BuildJob {
    root: PathBuf,
    rx: mpsc::Receiver<io::Result<Index>>,
    progress: Arc<AtomicUsize>,
}

/// Ergebnis eines Hintergrundauftrags: Statusmeldung + Eintrag für den Rückgängig-Verlauf.
struct JobDone {
    msg: String,
    hist: Hist,
    /// Löschen scheiterte, weil Programme die Dateien geöffnet haben → nachfragen
    locked: Option<Locked>,
}

/// Entwicklung: langsame Bilder protokollieren (FOLDEX_FRAMELOG=<datei>), mit Zeiten der Abschnitte
mod prof {
    use std::cell::RefCell;
    use std::time::Instant;
    thread_local! {
        static MARKS: RefCell<Vec<(&'static str, Instant)>> = const { RefCell::new(Vec::new()) };
    }
    pub fn mark(name: &'static str) {
        MARKS.with(|m| m.borrow_mut().push((name, Instant::now())));
    }
    pub struct Frame(Instant);
    impl Frame {
        pub fn start() -> Frame {
            MARKS.with(|m| m.borrow_mut().clear());
            Frame(Instant::now())
        }
    }
    impl Drop for Frame {
        fn drop(&mut self) {
            let total = self.0.elapsed();
            // Schwelle: FOLDEX_FRAMELOG_MS (Standard 80 ms)
            static MIN: std::sync::OnceLock<u128> = std::sync::OnceLock::new();
            let min = *MIN.get_or_init(|| std::env::var("FOLDEX_FRAMELOG_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(80));
            if total.as_millis() < min {
                return;
            }
            let Some(f) = std::env::var_os("FOLDEX_FRAMELOG") else { return };
            let mut last = self.0;
            let mut line = format!("{} ms:", total.as_millis());
            MARKS.with(|m| {
                for (n, t) in m.borrow().iter() {
                    line.push_str(&format!(" {n}={}", t.duration_since(last).as_millis()));
                    last = *t;
                }
            });
            line.push_str(&format!(" rest={}
", last.elapsed().as_millis()));
            use std::io::Write;
            if let Ok(mut o) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
                let _ = o.write_all(line.as_bytes());
            }
        }
    }
}

/// Datei mit dem Standardprogramm öffnen. Cloud: im Hintergrund – Windows liest dabei die Datei an,
/// und das kann dauern, solange sie erst geladen wird.
fn open_detached(p: PathBuf) {
    if fsx::slow(&p) {
        std::thread::spawn(move || {
            let _ = open::that_detached(&p);
        });
    } else {
        let _ = open::that_detached(&p);
    }
}

/// Zeilen für die Rückfrage bei Namenskonflikten: (Name, Quelle, Ziel)
fn conflict_lines(conflicts: &[ops::Conflict]) -> Vec<(String, String, String)> {
    let info = |p: &Path| -> String {
        match fs::symlink_metadata(p) {
            Ok(md) if md.is_dir() => format!("Ordner, {}", fmt_time(mtime_ns(md.modified().ok()))),
            Ok(md) => format!("{}, {}", human(md.len()), fmt_time(mtime_ns(md.modified().ok()))),
            Err(_) => "?".into(),
        }
    };
    conflicts
        .iter()
        .take(40)
        .map(|c| (c.dst.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), info(&c.src), info(&c.dst)))
        .collect()
}

/// Fingerabdruck eines Ordnerinhalts (Name, Ordner?, Größe, Datum) – unabhängig von der Sortierung
fn listing_sig<'a>(it: impl Iterator<Item = (&'a str, bool, u64, i64)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut v: Vec<(&str, bool, u64, i64)> = it.collect();
    v.sort_unstable();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// Ordner frisch lesen und den Fingerabdruck bilden (None = nicht lesbar)
fn read_sig(dir: &Path) -> Option<u64> {
    let rows: Vec<(String, bool, u64, i64)> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            let d = md.is_dir();
            Some((e.file_name().to_string_lossy().into_owned(), d, if d { 0 } else { md.len() }, mtime_ns(md.modified().ok())))
        })
        .collect();
    Some(listing_sig(rows.iter().map(|(n, d, s, m)| (n.as_str(), *d, *s, *m))))
}

/// Ergebnis einer Hintergrund-Aktion, das die Oberfläche anwendet
type Deferred = Box<dyn FnOnce(&mut App) + Send>;
/// Aktion für die Cloud-Warteschlange
type CloudTask = Box<dyn FnOnce() -> Deferred + Send>;

/// Ein Arbeits-Thread für Cloud-Aktionen: der Reihe nach (Neu vor Umbenennen), die Oberfläche wartet nie.
fn cloud_worker(ctx: &egui::Context) -> (mpsc::Sender<CloudTask>, mpsc::Receiver<Deferred>, mpsc::Sender<Deferred>) {
    let (tx, rx) = mpsc::channel::<CloudTask>();
    let (dtx, drx) = mpsc::channel::<Deferred>();
    let dtx2 = dtx.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        for t in rx {
            let _ = dtx.send(t());
            ctx.request_repaint();
        }
    });
    (tx, drx, dtx2)
}

/// Kachel der Startseite
struct Tile {
    path: PathBuf,
    name: String,
    sub: Option<String>,
    space: Option<(u64, u64)>,
    icon: TileIcon,
    /// Cloud: Anbieter und Konto (für „Papierkorb anzeigen“)
    cloud: Option<(&'static str, Option<String>)>,
    /// eigener Link: Stelle in der Liste
    link: Option<usize>,
}

enum TileIcon {
    Quick(Ic),
    Folder,
    Drive,
    Cloud(&'static str),
}

/// Gesperrte Elemente beim Löschen: welche, von wem, Papierkorb oder endgültig, schon freundlich versucht?
#[derive(Clone)]
struct Locked {
    paths: Vec<PathBuf>,
    procs: Vec<locks::Locker>,
    perm: bool,
    tried: bool,
}

/// Löschen ausführen (Papierkorb oder endgültig) und gesperrte Reste erkennen.
fn delete_job(v: Vec<PathBuf>, perm: bool) -> JobDone {
    let (msg, hist) = if perm {
        let (ok, fail) = ops::delete_permanent(&v);
        (if fail == 0 { format!("{ok} Element(e) endgültig gelöscht") } else { format!("{ok} endgültig gelöscht, {fail} fehlgeschlagen") }, Hist::None)
    } else {
        let (res, ctrashed) = trashx::delete_ex(&v);
        let done: Vec<PathBuf> = v.iter().filter(|p| !ops::exists(p) || ctrashed.iter().any(|c| c.paths.contains(p))).cloned().collect();
        for p in &done {
            fsx::note(p, None);
        }
        let cloud = done.iter().filter(|p| trashx::on_cloud(p)).count();
        let drive = ctrashed.iter().map(|c| c.ids.len()).sum::<usize>();
        let msg = match (&res, done.len()) {
            (Ok(_), n) if n == v.len() && drive == n => format!("{n} Element(e) in den Papierkorb von Google Drive gelegt – Strg+Z holt sie zurück"),
            (Ok(_), n) if n == v.len() && cloud == n => format!("{n} Element(e) gelöscht (liegen im Papierkorb des Cloud-Anbieters)"),
            (Ok(_), n) if n == v.len() => format!("{n} Element(e) in den Papierkorb verschoben"),
            (Ok(_), n) => format!("{n} von {} Element(en) in den Papierkorb verschoben", v.len()),
            (Err(e), 0) => format!("Löschen fehlgeschlagen: {e}"),
            (Err(e), n) => format!("{n} von {} Element(en) gelöscht – Fehler: {e}", v.len()),
        };
        // aus der Cloud Gelöschtes lässt sich nicht aus dem Windows-Papierkorb zurückholen – Google Drive aber
        // aus seinem eigenen Papierkorb
        let done: Vec<PathBuf> = done.into_iter().filter(|p| !trashx::on_cloud(p)).collect();
        let hist = if !done.is_empty() {
            Hist::Record(Op::Trash { paths: done })
        } else if let Some(c) = ctrashed.into_iter().next() {
            Hist::Record(Op::CloudTrash { remote: c.remote, ids: c.ids, paths: c.paths })
        } else {
            Hist::None
        };
        (msg, hist)
    };
    // Was noch existiert, ist gescheitert – liegt das an einem Programm, das die Datei offen hat?
    let failed: Vec<PathBuf> = v.into_iter().filter(|p| ops::exists(p)).collect();
    let procs = if failed.is_empty() { Vec::new() } else { locks::lockers(&failed) };
    let locked = (!procs.is_empty()).then(|| Locked { paths: failed, procs, perm, tried: false });
    JobDone { msg, hist, locked }
}

struct Job {
    rx: mpsc::Receiver<JobDone>,
    label: String,
    prog: Option<Arc<AtomicU64>>,
    /// Gesamtzahl für die Fortschrittsanzeige (0 = unbekannt)
    total: u64,
    /// Abbrechen (wirkt zwischen Elementen – nie mitten in einer Datei)
    cancel: Option<Arc<AtomicBool>>,
    /// Fortschritt als Prozent anzeigen (`prog` in Promille, `total` = 1000)
    pct: bool,
}

/// Kopieren/Verschieben, das auf eine Entscheidung bei Namenskonflikten wartet.
struct Pending {
    srcs: Vec<PathBuf>,
    dest: PathBuf,
    copy: bool,
    /// (Name, Beschreibung neu, Beschreibung vorhanden)
    lines: Vec<(String, String, String)>,
    total: usize,
}

enum TrashAsk {
    DeleteSel,
    Empty,
}

struct TrashWin {
    rows: Vec<trashx::Row>,
    rx: Option<mpsc::Receiver<Result<Vec<trashx::Row>, String>>>,
    sel: HashSet<usize>,
    anchor: Option<usize>,
    err: Option<String>,
    ask: Option<TrashAsk>,
}

struct App {
    ctx: egui::Context,
    th: Theme,
    st: Settings,
    tabs: Vec<Tab>,
    cur: usize,
    next_tab: u64,
    clip: Option<clip::Clip>,
    props: Option<Props>,
    show_settings: bool,
    /// Fenster „Bild erzeugen“
    imggen: Option<localui::ImageGen>,
    /// Fenster „Ordner-Icon ändern“: Ordner und Suchtext
    folder_icon: Option<(PathBuf, String)>,
    /// gewählte Gruppe im Fenster „Icon ändern“
    folder_icon_group: &'static str,
    /// offene Zusammenfassungen
    summaries: Vec<summary::Summary>,
    /// liegt der Ordner in einem Cloud-Ordner? (zuletzt geprüfter Ordner)
    cloud_memo: std::cell::RefCell<Option<(PathBuf, bool)>>,
    /// Fenster „Cloud hinzufügen“
    cloud_setup: bool,
    /// offenes Anmeldeformular: Anbieter, Feldwerte
    cloud_form: Option<(&'static str, Vec<String>)>,
    /// Antwort auf eine Rückfrage (z. B. iCloud-Bestätigungscode)
    cloud_answer: String,
    // Ordnergrößen, Verlauf, Tastenkürzel, Werkzeuge
    dsize: dirsize::DirSizes,
    recent: sidebar::Recent,
    keymap: Keymap,
    palette: Option<PaletteState>,
    tools: Option<ToolsWin>,
    rules: Option<RulesWin>,
    /// Einstellungen: Befehl, dessen Tastenkürzel gerade aufgenommen wird; ggf. Konflikt
    key_rec: Option<&'static str>,
    key_conflict: Option<(&'static str, Shortcut, &'static str)>,
    /// Git-Status des aktuellen Ordners (für Markierungen in der Liste)
    git_st: Option<git::Status>,
    git_st_rx: Option<mpsc::Receiver<(PathBuf, Option<git::Status>)>>,
    git_ok: bool,
    /// gefundene Editoren (VS Code, Zed) mit Programmpfad
    ed_paths: Vec<(String, String, PathBuf)>,
    /// automatisch erkannte Editoren (ohne eigene Pfade) – für die Einstellungen
    ed_detected: Vec<(String, Option<PathBuf>)>,
    /// installierte KI-Agenten für die Kommandozeile: (Schlüssel, Name, Programm)
    agents: Vec<(String, String, String)>,
    /// zuletzt in der aktiven Ordneransicht ausgewählte Namen (Kontext für den KI-Assistenten)
    seen_sel: Vec<String>,
    /// vom KI-Assistenten angeforderte Navigation / Auswahl (nach dem Zeichnen umgesetzt)
    ai_go: Option<PathBuf>,
    /// Explorer Assistent / Explorer Agent (Seitenleiste rechts, bleibt beim Tabwechsel sichtbar)
    editor_ai: Option<aiui::ChatView>,
    /// Tab- und Befehlswünsche der Explorer-KI (nach dem Zeichnen ausgeführt)
    ai_fx: Vec<ai::Effect>,
    /// Eingaben der Explorer-KI (Maus, Tastatur): je Bild ein Paket
    inject: std::collections::VecDeque<Vec<egui::Event>>,
    /// wartendes Bildschirmfoto (Aufruf-ID)
    shot_wait: Option<String>,
    /// Bildpixel → Punkte des letzten Bildschirmfotos
    shot_scale: f32,
    /// zuletzt von der KI bewegter Mauszeiger (für die Anzeige)
    ai_cursor: Option<(Pos2, std::time::Instant)>,
    /// Terminal-Tab, den die KI zuletzt geöffnet hat (Tab-ID)
    ai_term: Option<u64>,
    ai_select: Option<Vec<String>>,
    /// API-Schlüssel je Modell-Profil (einmal aus dem verschlüsselten Speicher gelesen)
    ai_keys: HashMap<String, Option<String>>,
    /// Version je installiertem Agenten; Suche nach Agenten im Hintergrund
    agent_versions: HashMap<String, String>,
    agent_scan: Option<mpsc::Receiver<(Vec<(String, String, String)>, HashMap<String, String>)>>,
    /// Verbindungstest je Modell-Profil
    model_test: HashMap<String, Result<String, String>>,
    model_test_rx: Option<mpsc::Receiver<(String, Result<String, String>, Vec<String>)>>,
    /// in den Einstellungen abgerufene Modelle je Profil
    model_list: HashMap<String, Vec<String>>,
    /// in den Einstellungen gerade aufgeklapptes eigenes Modell
    model_open: Option<String>,
    /// Schnellleiste in den Einstellungen: gezogener Eintrag
    qb_drag: Option<usize>,
    /// Vorlage für „Modell hinzufügen“
    new_model: usize,
    /// verfügbare Shells (Anzeigename, Programm) – beim Start ermittelt
    shells: Vec<(String, String)>,
    /// zuletzt gesehener Ordner der aktiven Ansicht (für Verlauf, Größen und Git-Status)
    seen_cwd: PathBuf,
    /// Löschen scheiterte an geöffneten Dateien → Nachfrage
    locked: Option<Locked>,
    /// Vorschau-Panel (eines für die aktive Kachel)
    /// Umbenennen in eine fremde Dateiart → Nachfrage (von, nach, Begründung)
    rename_ask: Option<(PathBuf, PathBuf, String)>,
    /// Strg+V war im letzten Frame gedrückt (Flankenerkennung)
    paste_down: bool,
    /// Dateien, die gerade per Windows-Drag&Drop aus diesem Fenster gezogen werden
    /// (läuft modal; der Drop auf dieses Fenster kommt kurz danach als Datei-Ereignis an)
    os_drag_src: Option<(Vec<PathBuf>, Instant)>,
    /// Ziehen: ist das erste Element ein Ordner? (einmal ermittelt)
    drag_first_dir: Option<(PathBuf, bool)>,
    /// Fensterposition für „Neues Fenster auf Monitor …“ (wird im ersten Frame gesetzt)
    place: Option<[i32; 4]>,
    // Themes
    themes: themes::Store,
    /// in den Einstellungen markiertes Theme (Vorschau, noch nicht angewendet)
    theme_sel: Option<String>,
    /// gewählter Bereich der Einstellungen
    settings_page: String,
    /// Bilder seit dem Start (nur für FOLDEX_SHOT)
    dev_frames: u32,
    dev_shot_sent: bool,
    /// Fenster-Handle (für das Teilen über das Teilen-Menü einer App)
    hwnd: isize,
    /// Meldung im Theme-/Übertragen-Bereich: (Fehler?, Zeilen)
    cfg_msg: Option<(bool, Vec<String>)>,
    /// geprüfter Einstellungs-Import, wartet auf Bestätigung
    import: Option<settings::Import>,
    /// Eingabefeld für einen API-Schlüssel in den Einstellungen (für das Profil `key_edit`)
    ai_key_input: String,
    key_edit: Option<String>,
    // Verlauf / Dialoge
    side_visible: bool,
    tab_drag: Option<TabDrag>,
    /// zuletzt geschlossene Tabs dieser Sitzung (neueste am Ende)
    closed_tabs: Vec<ClosedTab>,
    /// Video im schwebenden Bild-in-Bild-Fenster und dessen Startposition
    pip: Option<media::Player>,
    pip_pos: Option<[f32; 2]>,
    /// Kacheln des aktuellen Tabs im letzten Frame: (Pane-Nummer, Bereich)
    leaf_rects: Vec<(usize, Rect)>,
    /// Kopfzeilen (Tab-Leisten) der Kacheln im letzten Frame – Ablageziel „als Tab aufnehmen“
    head_rects: Vec<(usize, Rect)>,
    merge_target_last: Option<usize>,
    /// Umsortieren in einer Kachel: (Kachel, neue Position) – im letzten Frame berechnet
    sub_reorder: Option<(usize, usize)>,
    tabbar_rect: Rect,
    /// Ein anderes Foldex-Fenster zieht gerade etwas über dieses Fenster
    foreign: Option<ipc::DragInfo>,
    handoff: Option<Handoff>,
    ipc_rx: mpsc::Receiver<ipc::Event>,
    win_rect: Arc<std::sync::Mutex<Option<[f32; 4]>>>,
    #[cfg(target_os = "macos")]
    device: Option<device_query::DeviceState>,
    /// gezeichnete (animierte) x-Position je Tab
    tab_x: HashMap<u64, f32>,
    /// offenes Teilen-Fenster
    share: Option<share::ShareWin>,
    /// Sitzung: zuletzt gespeichert (Zeitpunkt, Inhalt) und Fensterlage (Position, Größe, maximiert)
    session_at: Instant,
    session_last: Option<session::WinState>,
    session_geo: (Option<[f32; 2]>, Option<[f32; 2]>, bool),
    /// zuletzt gespeicherte Fensterlage (window.json) und wann
    geo_saved: Option<session::WinGeo>,
    geo_at: Instant,
    /// waagerechtes Scrollen der Tab-Leiste (wenn nicht alle Tabs passen) und zuletzt sichtbar gemachter Tab
    tab_scroll: f32,
    tab_scroll_cur: usize,
    /// Inhaltsbereich des letzten Frames (Ziel für das Andocken)
    central_rect: Rect,
    /// Laufwerk → (frei, gesamt) in Bytes; Papierkorb-Größe
    drive_space: HashMap<PathBuf, (u64, u64)>,
    trash_bytes: Option<u64>,
    space_rx: Option<mpsc::Receiver<(Vec<(PathBuf, u64, u64)>, u64)>>,
    space_last: Option<Instant>,
    history: History,
    pending: Option<Pending>,
    confirm_del: Option<Vec<PathBuf>>,
    trash: Option<TrashWin>,
    // Index
    slots: Vec<DriveIndex>,
    build: Option<BuildJob>,
    queue: VecDeque<PathBuf>,
    redo_pc: bool,
    // Seitenleiste
    side: SidebarCfg,
    drives: Vec<Entry>,
    drives_rx: mpsc::Receiver<Entry>,
    drives_tx: mpsc::Sender<Entry>,
    /// zuletzt gesehene Laufwerksbuchstaben (Bitmaske) und wann geprüft
    drive_mask: (u32, Instant),
    /// Linux: Fingerabdruck der eingehängten Datenträger
    #[cfg_attr(windows, allow(dead_code))]
    mount_sig: u64,
    side_rename: Option<(usize, String)>,
    git: Vec<GitRepo>,
    git_rx: Option<mpsc::Receiver<Vec<GitRepo>>>,
    side_drag: Option<usize>,
    pin_rects: Vec<Rect>,
    /// Cloud-Aktionen der Oberfläche (Neu, Umbenennen, Prüfen …) laufen der Reihe nach im Hintergrund;
    /// ihr Ergebnis kommt als Auftrag an die Oberfläche zurück
    cloud_q: mpsc::Sender<CloudTask>,
    defer_rx: mpsc::Receiver<Deferred>,
    /// für eigene Hintergrund-Threads: Ergebnis an die Oberfläche
    defer_tx: mpsc::Sender<Deferred>,
    /// Cloud-Ordner live abgleichen: letzter Abgleich, läuft gerade, wann welcher Ordner neu geholt wurde
    cloud_watch: Instant,
    cloud_watch_busy: Arc<AtomicBool>,
    cloud_refresh_at: HashMap<PathBuf, Instant>,
    /// Druckfenster
    printwin: Option<print::PrintWin>,
    /// Rückfrage „Deinstallieren“: Some(Programmdatei auch löschen?)
    uninstall_ask: Option<bool>,
    /// Updates von GitHub; `update_quit`: Installer läuft, Foldex beendet sich
    updater: update::Updater,
    /// Fenster „Bilder bearbeiten“
    batch: Option<batch::Opts>,
    /// Inhaltssuche: Anfrage, Ordner, laufende/fertige Suche
    csearch: Option<(String, PathBuf, Option<contentsearch::Search>)>,
    update_quit: Arc<AtomicBool>,
    update_checked: bool,
    /// Startseite: Einstellungen, Suchfeld, Hintergrundbild
    start_cfg: startpage::StartCfg,
    start_query: String,
    start_bg: Option<(PathBuf, egui::TextureHandle)>,
    /// erstes Bild: „Foldex startet mit der Startseite“
    boot: bool,
    /// Cloud-Eintrag, der gerade verschoben wird, und die Zeilen der Cloud-Einträge
    cloud_drag: Option<usize>,
    cloud_rects: Vec<Rect>,
    pc_query: String,
    // Drag & Drop
    drag: Option<Vec<PathBuf>>,
    zones: Vec<(Rect, Dest)>,
    zones_prev: Vec<(Rect, Dest)>,
    drop_target: Option<Dest>,
    // Sonstiges
    icons: Icons,
    jobs: Vec<Job>,
    status: String,
    col_w: [f32; 3],
    dual_now: bool,
    last_title: String,
}

// ───────────────────────────── Logik ─────────────────────────────

impl App {
    fn new(ctx: egui::Context, launch: Launch) -> Self {
        boot("App::new");
        setup_fonts(&ctx);
        boot("Schriften");
        // maximiert gespeichert: nach dem ersten Bild maximieren (beim Erzeugen allein greift es nicht)
        if launch.maximized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        let st = Settings::load();
        ctx.style_mut(|s| s.animation_time = if st.animations { 0.12 } else { 0.0 });
        theme_pref(&ctx, st.theme);
        let themes = themes::Store::load();
        let th = themes_for(&themes, &st, ctx.style().visuals.dark_mode);
        apply_theme(&ctx, &th);
        let mut icons = Icons::new();
        icons.set_system(st.sys_icons, &ctx);
        icons.set_theme(matico::Set::from_key(&st.icon_set));
        boot("Einstellungen, Themes, Symbole");
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut starts = launch.paths.clone();
        if starts.is_empty() {
            starts.push(home);
        }
        let shells = editors::shells();
        boot("Shells gesucht");
        let panes: Vec<Pane> = starts.iter().map(|p| pane_from_spec(p, &ctx, &st, &shells)).collect();
        let root = launch
            .layout
            .as_deref()
            .and_then(Node::parse)
            .filter(|n| n.valid_for(panes.len()))
            .unwrap_or_else(|| Node::chain(panes.len(), Dir::H));
        // Sitzung wiederherstellen: Tabs in Reihenfolge, Kacheln mit Aufteilung, Vorschau je Kachel
        let restored: Vec<Tab> = launch
            .session
            .as_ref()
            .map(|ws| {
                ws.tabs
                    .iter()
                    .filter(|t| !t.panes.is_empty())
                    .enumerate()
                    .map(|(k, t)| tab_from_state(t, k as u64 + 1, &ctx, &st, &shells))
                    .collect()
            })
            .unwrap_or_default();
        boot("Tabs/Sitzung");
        let restored_cur = launch.session.as_ref().map_or(0, |s| s.cur);
        let win_rect = Arc::new(std::sync::Mutex::new(None));
        let ipc_rx = ipc::spawn_watcher(ctx.clone(), win_rect.clone());
        let (drives_tx, drives_rx) = probe_drives(&ctx);
        fsx::init(&ctx);
        let (cloud_q, defer_rx, defer_tx) = cloud_worker(&ctx);
        // Cloud-Suche gleich beim Start anstoßen
        let _ = cloud::places();
        let side = SidebarCfg::load();
        let git_rx = st.show_git.then(|| probe_git(&ctx, git_roots(&side)));
        boot("Hintergrund gestartet");
        App {
            th,
            cur: if restored.is_empty() { 0 } else { restored_cur.min(restored.len() - 1) },
            next_tab: restored.len().max(1) as u64 + 1,
            tabs: if restored.is_empty() {
                vec![Tab {
                    id: 1,
                    active: 0,
                    root,
                    panes,
                }]
            } else {
                restored
            },
            session_at: Instant::now(),
            share: None,
            session_last: None,
            session_geo: (None, None, false),
            geo_saved: None,
            geo_at: Instant::now(),
            clip: None,
            props: None,
            show_settings: false,
            imggen: None,
            folder_icon: None,
            folder_icon_group: "material-ordner",
            summaries: Vec::new(),
            cloud_memo: Default::default(),
            cloud_setup: false,
            cloud_form: None,
            cloud_answer: String::new(),
            dsize: dirsize::DirSizes::new(&ctx),
            recent: sidebar::Recent::load(),
            keymap: Keymap::load(),
            palette: None,
            tools: None,
            rules: None,
            key_rec: None,
            key_conflict: None,
            git_st: None,
            git_st_rx: None,
            git_ok: git::git_available(),
            ed_paths: find_editors(&st),
            ed_detected: editors::EDITORS.iter().map(|d| (d.key.to_string(), editors::detect(d.key))).collect(),
            agents: agents::detect(),
            seen_sel: Vec::new(),
            ai_go: None,
            editor_ai: None,
            ai_fx: Vec::new(),
            inject: Default::default(),
            shot_wait: None,
            shot_scale: 1.0,
            ai_cursor: None,
            ai_term: None,
            ai_select: None,
            ai_keys: st.ai_models.iter().map(|p| (p.id.clone(), ai::key_for(p))).collect(),
            agent_versions: HashMap::new(),
            agent_scan: Some(scan_agents(&ctx)),
            model_test: HashMap::new(),
            model_test_rx: None,
            model_list: HashMap::new(),
            model_open: None,
            qb_drag: None,
            new_model: 0,
            shells,
            seen_cwd: PathBuf::new(),
            place: launch.place,
            paste_down: false,
            locked: None,
            rename_ask: None,
            os_drag_src: None,
            drag_first_dir: None,
            themes,
            theme_sel: None,
            settings_page: String::new(),
            dev_frames: 0,
            dev_shot_sent: false,
            hwnd: 0,
            cfg_msg: None,
            import: None,
            ai_key_input: String::new(),
            key_edit: None,
            side_visible: st.show_sidebar,
            tab_drag: None,
            closed_tabs: Vec::new(),
            pip: None,
            pip_pos: None,
            leaf_rects: Vec::new(),
            head_rects: Vec::new(),
            merge_target_last: None,
            sub_reorder: None,
            tabbar_rect: Rect::NOTHING,
            foreign: None,
            handoff: None,
            ipc_rx,
            win_rect,
            #[cfg(target_os = "macos")]
            device: device_query::DeviceState::checked_new(),
            tab_x: HashMap::new(),
            tab_scroll: 0.0,
            tab_scroll_cur: usize::MAX,
            central_rect: Rect::NOTHING,
            drive_space: HashMap::new(),
            trash_bytes: None,
            space_rx: None,
            space_last: None,
            history: History::default(),
            pending: None,
            confirm_del: None,
            trash: None,
            slots: vec![],
            build: None,
            queue: VecDeque::new(),
            redo_pc: false,
            side,
            drives: vec![],
            drives_rx,
            drives_tx,
            drive_mask: (sidebar::drive_mask(), Instant::now()),
            mount_sig: 0,
            side_rename: None,
            git: vec![],
            git_rx,
            side_drag: None,
            pin_rects: vec![],
            cloud_q,
            defer_rx,
            defer_tx,
            cloud_watch: Instant::now(),
            cloud_watch_busy: Arc::new(AtomicBool::new(false)),
            cloud_refresh_at: HashMap::new(),
            printwin: None,
            uninstall_ask: None,
            updater: update::Updater::default(),
            batch: None,
            csearch: None,
            update_quit: Arc::new(AtomicBool::new(false)),
            update_checked: false,
            start_cfg: startpage::StartCfg::load(),
            start_query: String::new(),
            start_bg: None,
            boot: true,
            cloud_drag: None,
            cloud_rects: vec![],
            pc_query: String::new(),
            drag: None,
            zones: vec![],
            zones_prev: vec![],
            drop_target: None,
            icons,
            jobs: vec![],
            status: String::new(),
            col_w: [150.0, 170.0, 90.0],
            dual_now: false,
            last_title: String::new(),
            st,
            ctx,
        }
    }

    fn pane(&self) -> &Pane {
        let t = &self.tabs[self.cur];
        &t.panes[t.active]
    }
    fn pane_mut(&mut self) -> &mut Pane {
        let t = &mut self.tabs[self.cur];
        let a = t.active;
        &mut t.panes[a]
    }

    fn nav(&mut self, p: PathBuf) {
        let pc = matches!(
            self.pane().search,
            Some(SearchView {
                kind: SearchKind::Pc,
                ..
            })
        );
        if pc {
            self.pc_query.clear();
        }
        if self.pane().extra.is_some() {
            self.new_tab(p);
            return;
        }
        let t = &mut self.tabs[self.cur];
        let a = t.active;
        t.panes[a].go(p, true, &self.ctx, &self.st);
    }

    fn refresh_all(&mut self) {
        let ctx = self.ctx.clone();
        each_pane(&mut self.tabs, |p| p.start_load(&ctx));
    }

    fn refresh_views(&mut self) {
        let st = self.st.clone();
        each_pane(&mut self.tabs, |p| p.refresh_view(&st));
    }

    // ── Hintergrund-Aufträge ──
    fn poll(&mut self) {
        while let Ok(d) = self.defer_rx.try_recv() {
            d(self);
        }
        self.cloud_live();
        // einmal am Tag nach einer neuen Version sehen (nicht bei Bildschirmfotos für die Doku)
        if !self.update_checked {
            self.update_checked = true;
            if std::env::var_os("FOLDEX_SHOT").is_none() && std::env::var_os("FOLDEX_NO_UPDATE").is_none() {
                self.updater.check_daily(&self.ctx);
            }
        }
        if self.update_quit.swap(false, Relaxed) {
            // der Installer ist gestartet: normal beenden (Sitzung speichern), er startet danach die neue Version
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.icons.poll(&self.ctx);
        if let Some(Ok(v)) = self.git_rx.as_ref().map(|rx| rx.try_recv()) {
            self.git = v;
            self.git_rx = None;
        }
        // Laufwerks-Belegung und Papierkorb-Größe (im Hintergrund, alle 20 s und nach jedem Auftrag)
        if let Some(rx) = &self.space_rx {
            if let Ok((v, t)) = rx.try_recv() {
                for (p, free, total) in v {
                    self.drive_space.insert(p, (free, total));
                }
                self.trash_bytes = Some(t);
                self.space_rx = None;
            }
        }
        let due = self.space_last.map_or(true, |t| t.elapsed() > Duration::from_secs(20));
        if due && self.space_rx.is_none() && !self.drives.is_empty() {
            self.space_last = Some(Instant::now());
            // Cloud-Laufwerke nicht: deren Speicher kommt vom Anbieter, und die Abfrage kann hängen
            let paths: Vec<PathBuf> = self.drives.iter().map(|d| d.path.clone()).filter(|p| !trashx::on_cloud(p)).collect();
            let (tx, rx) = mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let v = paths
                    .into_iter()
                    .filter_map(|p| {
                        let free = fs4::available_space(&p).ok()?;
                        let total = fs4::total_space(&p).ok()?;
                        Some((p, free, total))
                    })
                    .collect();
                let _ = tx.send((v, trashx::total_size()));
                ctx.request_repaint();
            });
            self.space_rx = Some(rx);
        }
        self.ctx.request_repaint_after(Duration::from_secs(5));
        // Laufwerke kommen und gehen (USB-Stick, Cloud-Laufwerk getrennt …): alle 2 s die Buchstaben prüfen
        #[cfg(windows)]
        if self.drive_mask.1.elapsed() > Duration::from_secs(2) {
            let now = sidebar::drive_mask();
            let old = self.drive_mask.0;
            self.drive_mask = (now, Instant::now());
            if now != old {
                let bit = |p: &Path| p.to_string_lossy().chars().next().filter(|c| c.is_ascii_alphabetic()).map(|c| 1u32 << (c.to_ascii_uppercase() as u32 - 'A' as u32));
                self.drives.retain(|d| bit(&d.path).is_none_or(|b| now & b != 0));
                self.slots.retain(|s| bit(&s.root).is_none_or(|b| now & b != 0));
                for i in 0..26u32 {
                    if now & (1 << i) != 0 && old & (1 << i) == 0 {
                        sidebar::probe_letter(&self.drives_tx, &self.ctx, (b'A' + i as u8) as char);
                    }
                }
                cloud::refresh();
            }
        }
        // Linux: eingehängte Datenträger alle 2 s vergleichen (USB-Stick ein/aus)
        #[cfg(not(windows))]
        if self.drive_mask.1.elapsed() > Duration::from_secs(2) {
            self.drive_mask.1 = Instant::now();
            let sig = sidebar::mounts_sig();
            if sig != self.mount_sig {
                self.mount_sig = sig;
                let now = sidebar::linux_drives();
                self.drives.retain(|d| now.iter().any(|e| e.path == d.path));
                self.slots.retain(|s| now.iter().any(|e| e.path == s.root));
                for e in now {
                    if !self.drives.iter().any(|d| d.path == e.path) {
                        let _ = self.drives_tx.send(e);
                    }
                }
                cloud::refresh();
            }
        }
        // neue Laufwerke
        while let Ok(e) = self.drives_rx.try_recv() {
            if self.drives.iter().any(|d| d.path == e.path) {
                continue;
            }
            let mut idx = None;
            // Cloud-Laufwerke nicht indexieren: das würde den ganzen Online-Speicher durchlaufen
            let cloud_drive = trashx::on_cloud(&e.path);
            if cloud_drive {
            } else if let Some(f) = index::latest_cache(&e.path) {
                if let Ok(ix) = Index::open(&f) {
                    let stale = ix
                        .built
                        .elapsed()
                        .map(|d| d > Duration::from_secs(12 * 3600))
                        .unwrap_or(true);
                    idx = Some(Arc::new(ix));
                    if stale && self.st.auto_index {
                        self.queue.push_back(e.path.clone());
                    }
                }
            } else if self.st.auto_index {
                self.queue.push_back(e.path.clone());
            }
            self.slots.push(DriveIndex {
                root: e.path.clone(),
                index: idx,
            });
            self.drives.push(e);
            self.drives.sort_by(|a, b| a.path.cmp(&b.path));
        }
        // Indexbau
        let r = self.build.as_ref().map(|b| b.rx.try_recv());
        match r {
            Some(Ok(res)) => {
                let root = self.build.take().unwrap().root;
                match res {
                    Ok(ix) => {
                        let keep = ix.file.clone();
                        if let Some(s) = self.slots.iter_mut().find(|s| s.root == root) {
                            s.index = Some(Arc::new(ix));
                        }
                        index::cleanup(&root, &keep);
                        self.redo_pc = true;
                    }
                    Err(e) => self.status = format!("Index-Fehler ({}): {e}", root.display()),
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => self.build = None,
            Some(Err(_)) => self.ctx.request_repaint_after(Duration::from_millis(150)),
            None => {}
        }
        if self.build.is_none() {
            if let Some(root) = self.queue.pop_front() {
                self.start_build(root);
            }
        }
        if self.redo_pc {
            self.redo_pc = false;
            if !self.pc_query.trim().is_empty() {
                self.run_pc_search();
            }
        }
        // Jobs (Kopieren, Entpacken, Löschen …)
        let mut finished = false;
        let mut msg = None;
        let mut hists = Vec::new();
        let mut locked = None;
        self.jobs.retain(|j| match j.rx.try_recv() {
            Ok(d) => {
                if d.locked.is_some() {
                    locked = d.locked;
                }
                msg = Some(d.msg);
                hists.push(d.hist);
                finished = true;
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for h in hists {
            self.history.apply(h);
        }
        if locked.is_some() {
            self.locked = locked;
        }
        if finished {
            let c = self.seen_cwd.clone();
            self.dsize.invalidate(&c);
            self.refresh_git();
        }
        if let Some(m) = msg {
            self.status = m;
        }
        if !self.jobs.is_empty() {
            self.ctx.request_repaint_after(Duration::from_millis(150));
        }
        // Papierkorb-Fenster: Listing abholen
        if let Some(tw) = &mut self.trash {
            if let Some(rx) = &tw.rx {
                match rx.try_recv() {
                    Ok(r) => {
                        tw.rx = None;
                        match r {
                            Ok(rows) => {
                                tw.rows = rows;
                                tw.err = None;
                            }
                            Err(e) => tw.err = Some(e),
                        }
                        tw.sel.clear();
                        tw.anchor = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => self.ctx.request_repaint_after(Duration::from_millis(100)),
                    Err(_) => tw.rx = None,
                }
            }
        }
        // Panes
        let (st, ctx) = (&self.st, &self.ctx);
        each_pane(&mut self.tabs, |p| {
            p.poll(st, ctx);
        });
        if finished {
            self.space_last = None; // Belegung/Papierkorb neu berechnen
            cloudlogin::space_soon();
            self.refresh_all();
            if self.tabs.iter().any(|t| t.panes.iter().any(|p| p.trash)) {
                self.load_trash();
            }
        }
    }

    fn start_build(&mut self, root: PathBuf) {
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(AtomicUsize::new(0));
        let (prog, ctx, r2) = (progress.clone(), self.ctx.clone(), root.clone());
        std::thread::spawn(move || {
            let dest = index::new_cache_file(&r2);
            let res = index::build(&r2, &dest, &prog, &AtomicBool::new(false));
            let _ = tx.send(res);
            ctx.request_repaint();
        });
        self.build = Some(BuildJob { root, rx, progress });
    }

    fn queue_index(&mut self, root: PathBuf) {
        let busy = self.build.as_ref().is_some_and(|b| b.root == root);
        if !busy && !self.queue.contains(&root) {
            self.queue.push_back(root);
        }
    }

    // ── Suche ──
    fn run_folder_search(&mut self) {
        let t = &mut self.tabs[self.cur];
        let a = t.active;
        let pn = &mut t.panes[a];
        pn.search = None; // beendet eine laufende Live-Suche
        let qs = pn.query.trim().to_string();
        if qs.is_empty() {
            pn.refresh_view(&self.st);
            return;
        }
        let mut q = Query::parse(&qs);
        if q.is_empty() && !q.has_path_terms() {
            return;
        }
        let started = Instant::now();
        let cwd = pn.cwd.clone();
        let title = pn.title();
        let label = if q.has_content() { format!("Inhaltssuche in »{title}«") } else { format!("Suche in »{title}«") };
        // Gibt es einen Index, der diesen Ordner abdeckt? → blitzschnelle Suche nur in diesem Teilbaum.
        // Nicht innerhalb von System-/Cache-Ordnern (dort hat der Index keine Einträge) und nicht für Inhalte.
        let found = if q.has_content() || index::inside_excluded(&cwd) {
            None
        } else {
            self.slots.iter().find_map(|s| s.index.as_ref().and_then(|ix| ix.find_dir(&cwd).map(|d| (ix.clone(), d))))
        };
        pn.search = Some(match found {
            Some((ix, d)) => {
                q.scope = Some(d as u32);
                let r = ix.search(&q, 5000);
                let hits: Vec<(i32, Hit)> = r.ids.iter().zip(&r.scores).map(|(&id, &sc)| (sc, Hit::Idx(0, id))).collect();
                // Unterhalb eines Laufwerks zusätzlich live nachsehen: findet, was seit dem Indexbau neu
                // hinzukam (auch in versteckten Ordnern) – bereits angezeigte Treffer werden übersprungen.
                let live = (cwd.parent().is_some()).then(|| {
                    let skip: HashSet<PathBuf> = r.ids.iter().map(|&id| ix.path(id as usize)).collect();
                    live::start(cwd.clone(), Query::parse(&qs), skip, self.ctx.clone())
                });
                SearchView {
                    kind: SearchKind::Folder,
                    hits,
                    idxs: vec![ix],
                    total: r.total,
                    capped: r.capped,
                    ms: started.elapsed().as_secs_f64() * 1000.0,
                    live,
                    sel: HashSet::new(),
                    anchor: None,
                    label,
                }
            }
            None => SearchView {
                kind: SearchKind::Folder,
                idxs: vec![],
                hits: vec![],
                total: 0,
                capped: false,
                ms: 0.0,
                live: Some(live::start(cwd, q, HashSet::new(), self.ctx.clone())),
                sel: HashSet::new(),
                anchor: None,
                label,
            },
        });
    }

    fn run_pc_search(&mut self) {
        let qs = self.pc_query.trim().to_string();
        if qs.is_empty() {
            let pn = self.pane_mut();
            if matches!(
                pn.search,
                Some(SearchView {
                    kind: SearchKind::Pc,
                    ..
                })
            ) {
                pn.search = None;
            }
            return;
        }
        // Fehlende Indizes im Hintergrund aufbauen (einmalig; danach liegt er als Datei im Cache).
        let missing: Vec<PathBuf> = self
            .slots
            .iter()
            .filter(|s| s.index.is_none())
            .map(|s| s.root.clone())
            .collect();
        for r in missing {
            self.queue_index(r);
        }
        let q = Query::parse(&qs);
        if q.is_empty() && !q.has_path_terms() {
            return;
        }
        if q.has_content() && q.terms.is_empty() && q.exts.is_empty() && !q.has_path_terms() {
            self.status = "PC-Inhaltssuche: zusätzlich einen Namen oder z. B. ext:txt angeben – oder im Ordner suchen".into();
            return;
        }
        let started = Instant::now();
        let (mut idxs, mut hits, mut total, mut capped) = (vec![], Vec::new(), 0, false);
        for s in &self.slots {
            if let Some(ix) = &s.index {
                let r = ix.search(&q, 5000);
                let pos = idxs.len() as u16;
                idxs.push(ix.clone());
                hits.extend(
                    r.ids
                        .iter()
                        .zip(&r.scores)
                        .map(|(&id, &sc)| (sc, Hit::Idx(pos, id))),
                );
                total += r.total;
                capped |= r.capped;
            }
        }
        hits.sort_by(|a: &(i32, Hit), b: &(i32, Hit)| b.0.cmp(&a.0));
        hits.truncate(5000);
        // Inhaltssuche am ganzen PC: die Namenstreffer aus dem Index werden im Hintergrund gelesen
        let (live, label) = if q.has_content() {
            let files: Vec<PathBuf> = hits
                .iter()
                .filter_map(|(_, h)| match h {
                    Hit::Idx(k, id) => {
                        let ix = &idxs[*k as usize];
                        (!ix.is_dir(*id as usize)).then(|| ix.path(*id as usize))
                    }
                    Hit::Live(_) => None,
                })
                .collect();
            hits.clear();
            total = 0;
            (Some(live::start_content(files, q, self.ctx.clone())), "PC-Inhaltssuche (Namenstreffer aus dem Index)".to_string())
        } else {
            (None, "PC-Suche (alle Laufwerke)".to_string())
        };
        let pn = self.pane_mut();
        pn.search = Some(SearchView {
            kind: SearchKind::Pc,
            idxs,
            hits,
            total,
            capped,
            ms: started.elapsed().as_secs_f64() * 1000.0,
            live,
            sel: HashSet::new(),
            anchor: None,
            label,
        });
    }

    // ── Dateioperationen (alles im Hintergrund) ──
    fn run_job(&mut self, label: &str, prog: Option<Arc<AtomicU64>>, f: impl FnOnce() -> JobDone + Send + 'static) {
        self.run_job_ex(label, prog, 0, None, f);
    }

    fn run_job_ex(
        &mut self,
        label: &str,
        prog: Option<Arc<AtomicU64>>,
        total: u64,
        cancel: Option<Arc<AtomicBool>>,
        f: impl FnOnce() -> JobDone + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.jobs.push(Job {
            rx,
            label: label.into(),
            prog,
            total,
            cancel,
            pct: false,
        });
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }

    /// Kopieren/Verschieben. Gibt es im Ziel Namenskonflikte, fragt ein Dialog erst nach (Ersetzen / Beide
    /// behalten / Überspringen) – nichts wird stillschweigend überschrieben.
    fn spawn_transfer(&mut self, srcs: Vec<PathBuf>, dest: PathBuf, copy: bool) {
        if srcs.is_empty() {
            return;
        }
        // Cloud beteiligt: Prüfen kann dauern → im Hintergrund, danach geht es hier weiter
        if fsx::slow(&dest) || srcs.iter().any(|s| fsx::slow(s)) {
            self.status = "Prüfe Ziel …".into();
            let _ = self.cloud_q.send(Box::new(move || {
                let conflicts = ops::find_conflicts(&srcs, &dest, copy);
                let lines = conflict_lines(&conflicts);
                Box::new(move |app: &mut App| app.transfer_checked(srcs, dest, copy, conflicts.len(), lines))
            }));
            return;
        }
        let conflicts = ops::find_conflicts(&srcs, &dest, copy);
        let lines = conflict_lines(&conflicts);
        self.transfer_checked(srcs, dest, copy, conflicts.len(), lines);
    }

    /// nach der Konfliktprüfung: gleich los oder erst fragen
    fn transfer_checked(&mut self, srcs: Vec<PathBuf>, dest: PathBuf, copy: bool, total: usize, lines: Vec<(String, String, String)>) {
        if total == 0 {
            self.start_transfer(srcs, dest, copy, Policy::KeepBoth);
            return;
        }
        self.pending = Some(Pending { srcs, dest, copy, lines, total });
    }

    fn start_transfer(&mut self, srcs: Vec<PathBuf>, dest: PathBuf, copy: bool, policy: Policy) {
        let prog = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p2, c2) = (prog.clone(), cancel.clone());
        self.dsize.invalidate(&dest);
        for s in &srcs {
            if let Some(par) = s.parent() {
                self.dsize.invalidate(par);
            }
        }
        let total = srcs.len() as u64;
        self.run_job_ex(if copy { "Kopiere" } else { "Verschiebe" }, Some(prog), total, Some(cancel), move || {
            let t = ops::transfer(&srcs, &dest, copy, policy, &p2, &c2);
            let verb = if copy { "kopiert" } else { "verschoben" };
            let n = t.pairs.len();
            let mut msg = format!("{n} Element(e) {verb}");
            if c2.load(Relaxed) {
                msg += &format!(" – abgebrochen, {} nicht bearbeitet (bereits erledigte lassen sich mit Strg+Z rückgängig machen)", total as usize - n - t.skipped - t.errors.len());
            }
            if !t.replaced.is_empty() {
                msg += &format!(", {} ersetzt (Originale im Papierkorb)", t.replaced.len());
            }
            if t.skipped > 0 {
                msg += &format!(", {} übersprungen", t.skipped);
            }
            if !t.errors.is_empty() {
                msg += &format!(", {} fehlgeschlagen – {}", t.errors.len(), t.errors.join("; "));
            }
            let hist = if n == 0 {
                Hist::None
            } else if copy {
                Hist::Record(Op::Copy { pairs: t.pairs, replaced: t.replaced })
            } else {
                Hist::Record(Op::Move { pairs: t.pairs, replaced: t.replaced })
            };
            JobDone { locked: None,  msg, hist }
        });
    }

    fn spawn_extract(&mut self, archives: Vec<PathBuf>, into_folder: bool) {
        let archives: Vec<PathBuf> = archives.into_iter().filter(|p| fsx::is_file(p)).collect();
        if archives.is_empty() {
            return;
        }
        let prog = Arc::new(AtomicU64::new(0));
        let p2 = prog.clone();
        self.run_job("Entpacke …", Some(prog), move || {
            let mut total = 0;
            let mut errors = Vec::new();
            let mut created: Vec<PathBuf> = Vec::new();
            let cancel = AtomicBool::new(false);
            let names = |d: &Path| -> HashSet<std::ffi::OsString> {
                fs::read_dir(d)
                    .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
                    .unwrap_or_default()
            };
            for a in &archives {
                let name = a
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let parent = a.parent().map(Path::to_path_buf).unwrap_or_default();
                let before = if into_folder { HashSet::new() } else { names(&parent) };
                let dest = if into_folder {
                    unique(parent.join(extract::folder_name(&name)))
                } else {
                    parent.clone()
                };
                match extract::extract(a, &dest, &p2, &cancel) {
                    Ok(n) => total = n,
                    Err(e) => errors.push(format!("{name}: {e}")),
                }
                // Was ist neu hinzugekommen? (für „Rückgängig“)
                if into_folder {
                    if dest.exists() {
                        created.push(dest);
                    }
                } else {
                    for n in names(&parent) {
                        if !before.contains(&n) {
                            created.push(parent.join(n));
                        }
                    }
                }
            }
            let msg = if errors.is_empty() {
                format!("{total} Datei(en) entpackt")
            } else {
                format!("Entpacken fehlgeschlagen – {}", errors.join("; "))
            };
            let hist = if created.is_empty() { Hist::None } else { Hist::Record(Op::Extract { created }) };
            JobDone { locked: None,  msg, hist }
        });
    }

    fn spawn_compress(&mut self, srcs: Vec<PathBuf>, fmt: Format) {
        let Some(out) = ops::archive_path(&srcs, fmt) else { return };
        let prog = Arc::new(AtomicU64::new(0));
        let p2 = prog.clone();
        self.run_job("Komprimiere …", Some(prog), move || {
            let name = out.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            match ops::compress(&srcs, &out, fmt, &p2, &AtomicBool::new(false)) {
                Ok(bad) => JobDone { locked: None, 
                    msg: if bad == 0 {
                        format!("»{name}« erstellt")
                    } else {
                        format!("»{name}« erstellt – {bad} nicht lesbare Elemente übersprungen")
                    },
                    hist: Hist::Record(Op::Compress { out, srcs, fmt }),
                },
                Err(e) => JobDone { locked: None, 
                    msg: format!("Komprimieren fehlgeschlagen: {e}"),
                    hist: Hist::None,
                },
            }
        });
    }

    /// „Löschen“ = in den Papierkorb (rückgängig machbar).
    fn spawn_trash(&mut self, v: Vec<PathBuf>) {
        if v.is_empty() {
            return;
        }
        self.run_job("Lösche …", None, move || delete_job(v, false));
    }

    /// Endgültiges Löschen (nur nach Bestätigung, nicht rückgängig zu machen).
    fn spawn_delete_perm(&mut self, v: Vec<PathBuf>) {
        if v.is_empty() {
            return;
        }
        self.run_job("Lösche endgültig …", None, move || delete_job(v, true));
    }

    /// Sperrende Programme schließen (freundlich bzw. erzwungen) und das Löschen wiederholen.
    fn close_lockers_and_retry(&mut self, l: Locked, force: bool) {
        self.run_job(if force { "Beende Programm …" } else { "Schließe Programm …" }, None, move || {
            if let Err(e) = locks::close(&l.paths, force) {
                // freundlich abgelehnt → Erzwingen anbieten
                let procs = locks::lockers(&l.paths);
                let locked = (!procs.is_empty()).then(|| Locked { paths: l.paths.clone(), procs, perm: l.perm, tried: true });
                return JobDone { locked, msg: e, hist: Hist::None };
            }
            std::thread::sleep(Duration::from_millis(600));
            let mut d = delete_job(l.paths, l.perm);
            if let Some(x) = &mut d.locked {
                x.tried = true;
            }
            d
        });
    }

    /// Nachfrage, wenn Löschen an geöffneten Dateien scheitert.
    fn ui_locked(&mut self, ctx: &egui::Context) {
        let Some(l) = self.locked.clone() else { return };
        let th = self.th;
        let mut decision: Option<Option<bool>> = None; // Some(Some(force)) = schließen, Some(None) = abbrechen
        egui::Window::new("Datei ist geöffnet")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                let first = l.paths.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let more = if l.paths.len() > 1 { format!(" (und {} weitere)", l.paths.len() - 1) } else { String::new() };
                ui.label(format!("„{first}“{more} kann nicht {} werden, weil es geöffnet ist in:", if l.perm { "endgültig gelöscht" } else { "in den Papierkorb verschoben" }));
                ui.add_space(4.0);
                for (pid, name) in &l.procs {
                    ui.label(egui::RichText::new(format!("•  {name}   (Prozess {pid})")).strong());
                }
                ui.add_space(6.0);
                if l.tried {
                    ui.label(egui::RichText::new("Das Programm hat sich nicht schließen lassen – vielleicht wartet es auf eine Antwort (z. B. „Speichern?“). Erzwingen beendet es sofort; ungespeicherte Änderungen gehen verloren.").size(12.5).color(th.danger));
                } else {
                    ui.label("Soll ich das Programm schließen und es nochmal versuchen?");
                    ui.label(egui::RichText::new("Das Programm wird gebeten, sich zu beenden – wie beim Herunterfahren. Es kann vorher nachfragen, ob gespeichert werden soll.").size(12.0).color(th.sub));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if !l.tried && ui.button(egui::RichText::new("Programm schließen und erneut versuchen").strong()).clicked() {
                        decision = Some(Some(false));
                    }
                    if l.tried && ui.button(egui::RichText::new("Programm beenden (erzwingen)").color(th.danger)).clicked() {
                        decision = Some(Some(true));
                    }
                    if l.tried && ui.button("Nochmal freundlich versuchen").clicked() {
                        decision = Some(Some(false));
                    }
                    if ui.button("Abbrechen").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        decision = Some(None);
                    }
                });
            });
        match decision {
            Some(Some(force)) => {
                self.locked = None;
                self.close_lockers_and_retry(l, force);
            }
            Some(None) => {
                self.locked = None;
                self.status = "Löschen abgebrochen".into();
            }
            None => {}
        }
    }

    fn spawn_hist(&mut self, undo: bool) {
        if !self.jobs.is_empty() {
            self.status = "Bitte warten, bis der laufende Vorgang beendet ist".into();
            return;
        }
        let op = if undo { self.history.undo.pop() } else { self.history.redo.pop() };
        let Some(op) = op else {
            self.status = if undo { "Nichts zum Rückgängigmachen".into() } else { "Nichts zum Wiederholen".into() };
            return;
        };
        self.run_job(if undo { "Mache rückgängig …" } else { "Wiederhole …" }, None, move || {
            let r = if undo { ops::undo(&op) } else { ops::redo(&op) };
            match r {
                Ok(msg) => JobDone { locked: None, 
                    msg,
                    hist: if undo { Hist::Undone(op) } else { Hist::Redone(op) },
                },
                Err(e) => JobDone { locked: None, 
                    msg: format!("{} nicht möglich: {e}", if undo { "Rückgängig" } else { "Wiederholen" }),
                    hist: if undo { Hist::UndoFailed(op) } else { Hist::RedoFailed(op) },
                },
            }
        });
    }

    // ── Papierkorb-Fenster ──
    fn load_trash(&mut self) {
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(trashx::list());
            ctx.request_repaint();
        });
        match &mut self.trash {
            Some(tw) => tw.rx = Some(rx),
            None => {
                self.trash = Some(TrashWin {
                    rows: Vec::new(),
                    rx: Some(rx),
                    sel: HashSet::new(),
                    anchor: None,
                    err: None,
                    ask: None,
                })
            }
        }
    }

    fn open_trash(&mut self) {
        if trashx::CAN_LIST {
            let pn = self.pane_mut();
            pn.trash = true;
            pn.search = None;
            pn.rename = None;
            self.load_trash();
        } else if let Some(h) = dirs::home_dir() {
            let _ = open::that_detached(h.join(".Trash")); // macOS: Papierkorb im Finder öffnen
        }
    }

    fn create(&mut self, dir: bool) {
        let base = if dir {
            "Neuer Ordner"
        } else {
            "Neue Textdatei.txt"
        };
        let ctx = self.ctx.clone();
        let st = self.st.clone();
        let cloud_q = self.cloud_q.clone();
        let pn = self.pane_mut();
        if pn.search.is_some() {
            return;
        }
        let cloud = fsx::slow(&pn.cwd);
        let p = if cloud {
            // freier Name nach der angezeigten Liste – das Laufwerk selbst kann gerade hängen
            let (stem, ext) = match base.rsplit_once('.') {
                Some((s, e)) if !dir => (s.to_string(), format!(".{e}")),
                _ => (base.to_string(), String::new()),
            };
            let taken = |n: &str| pn.items.iter().any(|i| i.name.eq_ignore_ascii_case(n));
            let mut n = 1;
            let mut name = base.to_string();
            while taken(&name) {
                n += 1;
                name = format!("{stem} ({n}){ext}");
            }
            pn.cwd.join(name)
        } else {
            unique(pn.cwd.join(base))
        };
        let r = if cloud {
            let q = p.clone();
            let _ = cloud_q.send(Box::new(move || {
                let make = |p: &Path| if dir { fs::create_dir(p) } else { fs::File::create_new(p).map(|_| ()) };
                // Name doch schon vergeben (Liste war noch nicht fertig geladen) → nächsten freien nehmen
                let (q, r, moved) = match make(&q) {
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        let q2 = unique(q.clone());
                        let r = make(&q2);
                        (q2, r, true)
                    }
                    r => (q, r, false),
                };
                Box::new(move |app: &mut App| match r {
                    Ok(()) => {
                        fsx::note(&q, Some(fsx::Stat { dir, len: 0 }));
                        if moved {
                            // die Liste zeigt noch den alten Namen: neu laden (Umbenennen beginnt dann von vorn)
                            let ctx = app.ctx.clone();
                            let pn = app.pane_mut();
                            pn.rename = None;
                            pn.start_load(&ctx);
                            app.status = format!("Angelegt als „{}“", q.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                        }
                        app.history.apply(Hist::Record(Op::Create { path: q, dir }));
                    }
                    Err(e) => {
                        app.status = format!("Erstellen fehlgeschlagen: {e}");
                        let ctx = app.ctx.clone();
                        app.pane_mut().start_load(&ctx);
                    }
                })
            }));
            Ok(())
        } else if dir {
            fs::create_dir(&p)
        } else {
            fs::File::create_new(&p).map(|_| ()) // überschreibt nie eine vorhandene Datei
        };
        let mut created = None;
        match r {
            Ok(_) => {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                // Eintrag sofort lokal einfügen, damit er ohne Wartezeit umbenannt werden kann
                pn.items.push(Item {
                    name: name.clone().into_boxed_str(),
                    size: 0,
                    mtime: mtime_ns(Some(std::time::SystemTime::now())),
                    dir,
                    flags: 0,
                    grp: 0,
                });
                sort_items(&mut pn.items, pn.sort, pn.asc, pn.folders_first, pn.group);
                pn.refresh_view(&st);
                pn.select_name(&name);
                pn.layout_now();
                if let Some(c) = pn.cursor {
                    pn.ensure_visible(c);
                }
                pn.rename = Some(RenameState {
                    orig: name.clone(),
                    buf: name,
                    first: true,
                });
                if !cloud {
                    pn.start_load(&ctx); // danach mit Filterung/Sortierung neu laden
                    created = Some(p);
                }
            }
            Err(e) => self.status = format!("Erstellen fehlgeschlagen: {e}"),
        }
        if let Some(path) = created {
            self.history.apply(Hist::Record(Op::Create { path, dir }));
        }
    }

    fn commit_rename(&mut self) {
        let ctx = self.ctx.clone();
        let pn = self.pane_mut();
        let Some(rs) = pn.rename.take() else { return };
        let new = rs.buf.trim().to_string();
        if new.is_empty() || new == rs.orig {
            return;
        }
        if let Err(e) = ops::check_name(&new) {
            self.status = format!("Umbenennen nicht möglich: {e}");
            return;
        }
        let (from, to) = (pn.cwd.join(&rs.orig), pn.cwd.join(&new));
        let _ = ctx;
        let is_dir = match pn.items.iter().find(|i| *i.name == *rs.orig) {
            Some(i) => i.dir,
            None => fsx::is_dir(&from),
        };
        match convert::rename_kind(&rs.orig, &new, is_dir) {
            convert::RenameKind::Plain => self.do_rename(from, to),
            // z. B. icon.ico → icon.svg: ohne Nachfrage umwandeln (Original kommt in den Papierkorb)
            convert::RenameKind::Convert => self.spawn_convert(from, to, true),
            convert::RenameKind::Warn(why) => self.rename_ask = Some((from, to, why)),
        }
    }

    /// Umbenennen ohne Formatwechsel; überschreibt nie ein vorhandenes Element.
    fn do_rename(&mut self, from: PathBuf, to: PathBuf) {
        let ctx = self.ctx.clone();
        if fsx::slow(&from) {
            // sofort in der Liste umbenennen, auf dem Laufwerk im Hintergrund (nach einem evtl. „Neu“)
            let (old, new) = (from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            let st = self.st.clone();
            let pn = self.pane_mut();
            if let Some(i) = pn.items.iter_mut().find(|i| *i.name == *old) {
                i.name = new.clone().into_boxed_str();
            }
            sort_items(&mut pn.items, pn.sort, pn.asc, pn.folders_first, pn.group);
            pn.row_cache.clear();
            pn.refresh_view(&st);
            pn.sel.clear();
            pn.sel.insert(new);
            pn.sel_dirty = true;
            let _ = self.cloud_q.send(Box::new(move || {
                let r = ops::rename_safe(&from, &to).map(|_| ()).map_err(|e| e.to_string());
                Box::new(move |app: &mut App| {
                    let ctx = app.ctx.clone();
                    match r {
                        Ok(()) => {
                            fsx::note(&from, None);
                            app.history.apply(Hist::Record(Op::Rename { from, to }));
                        }
                        Err(e) => {
                            app.status = format!("Umbenennen fehlgeschlagen: {e}");
                            app.pane_mut().start_load(&ctx);
                        }
                    }
                })
            }));
            return;
        }
        match ops::rename_safe(&from, &to) {
            Ok(_) => {
                let pn = self.pane_mut();
                pn.start_load(&ctx);
                pn.sel.clear();
                if let Some(n) = to.file_name() {
                    pn.sel.insert(n.to_string_lossy().into_owned());
                }
                pn.sel_dirty = true;
                self.history.apply(Hist::Record(Op::Rename { from, to }));
            }
            Err(e) => self.status = format!("Umbenennen fehlgeschlagen: {e}"),
        }
    }

    /// Konvertieren im Hintergrund. `replace`: Umbenennen mit Formatwechsel – das Original kommt danach in
    /// den Papierkorb (rückgängig machbar); sonst entsteht eine zusätzliche Datei.
    fn spawn_convert(&mut self, src: PathBuf, dst: PathBuf, replace: bool) {
        self.spawn_convert_many(vec![(src, dst)], replace);
    }

    /// Mehrere Konvertierungen nacheinander in einem Auftrag – mit Fortschrittsbalken (Prozent) und „Abbrechen“.
    fn spawn_convert_many(&mut self, pairs: Vec<(PathBuf, PathBuf)>, replace: bool) {
        if pairs.is_empty() {
            return;
        }
        let name_of = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let label = if pairs.len() == 1 { format!("Konvertiere {} …", name_of(&pairs[0].1)) } else { format!("Konvertiere {} Dateien …", pairs.len()) };
        let prog = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p2, c2) = (prog.clone(), cancel.clone());
        self.run_job_ex(&label, Some(prog), 1000, Some(cancel), move || {
            let n = pairs.len() as u64;
            let (mut made, mut errors): (Vec<(PathBuf, PathBuf)>, Vec<String>) = (Vec::new(), Vec::new());
            for (k, (src, dst)) in pairs.into_iter().enumerate() {
                if c2.load(Relaxed) {
                    break;
                }
                // Gesamtfortschritt = erledigte Dateien + Anteil der aktuellen
                let k = k as u64;
                let (cur, done) = (AtomicU64::new(0), AtomicBool::new(false));
                let r = std::thread::scope(|sc| {
                    sc.spawn(|| {
                        while !done.load(Relaxed) {
                            p2.store((k * 1000 + cur.load(Relaxed)) / n, Relaxed);
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    });
                    let r = convert::convert_progress(&src, &dst, &cur, &c2);
                    done.store(true, Relaxed);
                    r
                });
                match r {
                    Ok(()) => made.push((src, dst)),
                    Err(_) if c2.load(Relaxed) => break,
                    Err(e) => errors.push(format!("{}: {e}", name_of(&src))),
                }
                p2.store((k + 1) * 1000 / n, Relaxed);
            }
            let tail = if c2.load(Relaxed) { " – abgebrochen" } else { "" };
            if made.len() == 1 && errors.is_empty() {
                let (src, dst) = made.pop().unwrap();
                let name = name_of(&dst);
                return if replace {
                    match trashx::delete(std::slice::from_ref(&src)) {
                        Ok(()) => JobDone { locked: None, msg: format!("In „{name}“ umgewandelt (Original im Papierkorb)"), hist: Hist::Record(Op::Convert { src, out: dst }) },
                        Err(e) => JobDone { locked: None, msg: format!("„{name}“ erstellt, Original blieb erhalten: {e}"), hist: Hist::Record(Op::Create { path: dst, dir: false }) },
                    }
                } else {
                    JobDone { locked: None, msg: format!("„{name}“ erstellt{tail}"), hist: Hist::Record(Op::Create { path: dst, dir: false }) }
                };
            }
            if made.is_empty() {
                let msg = if tail.is_empty() { format!("Konvertieren fehlgeschlagen: {}", errors.join("; ")) } else { "Konvertieren abgebrochen".to_string() };
                return JobDone { locked: None, msg, hist: Hist::None };
            }
            let errs = if errors.is_empty() { String::new() } else { format!(" – {} fehlgeschlagen: {}", errors.len(), errors.join("; ")) };
            let created: Vec<PathBuf> = made.into_iter().map(|(_, d)| d).collect();
            // mehrere neue Dateien: „Rückgängig“ legt sie in den Papierkorb (wie nach dem Entpacken)
            JobDone { locked: None, msg: format!("{} Datei(en) konvertiert{errs}{tail}", created.len()), hist: Hist::Record(Op::Extract { created }) }
        });
        if let Some(j) = self.jobs.last_mut() {
            j.pct = true;
        }
    }

    /// Nachfrage beim Wechsel in eine fremde Dateiart (z. B. .ico → .mp3).
    fn ui_rename_ask(&mut self, ctx: &egui::Context) {
        let Some((from, to, why)) = self.rename_ask.clone() else { return };
        let mut decision = None;
        egui::Window::new("Dateiendung ändern?")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                let n = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(egui::RichText::new(format!("{}  →  {}", n(&from), n(&to))).strong());
                ui.add_space(4.0);
                ui.label(&why);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Trotzdem umbenennen").clicked() {
                        decision = Some(true);
                    }
                    if ui.button(egui::RichText::new("Abbrechen").strong()).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.rename_ask = None;
                self.do_rename(from, to);
            }
            Some(false) => self.rename_ask = None,
            None => {}
        }
    }

    fn open_props(&mut self, p: PathBuf) {
        if fsx::slow(&p) {
            let _ = self.cloud_q.send(Box::new(move || {
                let md = fs::metadata(&p);
                Box::new(move |app: &mut App| match md {
                    Ok(md) => app.open_props_md(p, md),
                    Err(e) => app.status = format!("Eigenschaften: {e}"),
                })
            }));
            return;
        }
        let Ok(md) = fs::metadata(&p) else { return };
        self.open_props_md(p, md);
    }

    fn open_props_md(&mut self, p: PathBuf, md: fs::Metadata) {
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string());
        let pr = Props {
            kind: type_name(ext_buf(&name).as_str(), md.is_dir()),
            modified: fmt_time(mtime_ns(md.modified().ok())),
            created: fmt_time(mtime_ns(md.created().ok())),
            is_dir: md.is_dir(),
            size: Arc::new(AtomicU64::new(if md.is_dir() { 0 } else { md.len() })),
            files: Arc::new(AtomicU64::new(0)),
            done: Arc::new(AtomicBool::new(!md.is_dir())),
            name,
            path: p.clone(),
        };
        if md.is_dir() {
            let (s, f, d, ctx) = (
                pr.size.clone(),
                pr.files.clone(),
                pr.done.clone(),
                self.ctx.clone(),
            );
            std::thread::spawn(move || {
                // Cloud: rclone zählt online (schneller und schonender als das Laufwerk zu durchlaufen)
                match cloudlogin::size_of(&p) {
                    Some((b, n)) => {
                        s.store(b, Relaxed);
                        f.store(n, Relaxed);
                    }
                    None => folder_size(&p, &s, &f),
                }
                d.store(true, Relaxed);
                ctx.request_repaint();
            });
        }
        self.props = Some(pr);
    }

    fn new_tab(&mut self, p: PathBuf) {
        let pane = Pane::new(p, &self.ctx, &self.st);
        self.tabs.push(Tab::new(self.next_tab, pane));
        self.next_tab += 1;
        self.cur = self.tabs.len() - 1;
    }

    /// Schnellansicht (Leertaste): Datei in einem eigenen Tab öffnen. Ist die aktive Datei schon als
    /// Vorschau offen, wird dieser Tab nur aktiviert (kein zweiter für dieselbe Datei).
    fn open_view(&mut self, p: PathBuf) {
        if fsx::is_dir(&p) || !fsx::exists(&p) {
            return;
        }
        if let Some(i) = self.tabs.iter().position(|t| t.panes.iter().any(|pn| matches!(&pn.extra, Some(PaneView::View(v)) if v.path == p))) {
            self.cur = i;
            return;
        }
        let back = self.tabs.get(self.cur).map(|t| t.id);
        let mut pane = Pane::new(p.parent().map_or_else(|| p.clone(), Path::to_path_buf), &self.ctx, &self.st);
        pane.extra = Some(PaneView::View(Box::new(ViewTab { path: p, pv: preview::Preview::default(), back })));
        self.tabs.push(Tab::new(self.next_tab, pane));
        self.next_tab += 1;
        self.cur = self.tabs.len() - 1;
    }

    /// Ergebnis einer Vorschau verarbeiten (ffmpeg installieren, Bild-in-Bild starten/zurückholen).
    fn preview_out(&mut self, pv: &mut preview::Preview, out: Option<preview::PreviewOut>, acts: &mut Vec<Act>) {
        match out {
            Some(preview::PreviewOut::InstallFfmpeg) => acts.push(Act::InstallFfmpeg),
            Some(preview::PreviewOut::Pip) => {
                if let Some(pl) = pv.take_player() {
                    self.end_pip();
                    self.pip = Some(pl);
                    self.pip_pos = None;
                }
            }
            Some(preview::PreviewOut::PipBack) => {
                if let Some(pl) = self.pip.take() {
                    if let Err(pl) = pv.adopt(pl) {
                        self.pip = Some(pl);
                    }
                }
            }
            None => {}
        }
    }

    /// Bild-in-Bild beenden: Zeigt eine Vorschau noch dieselbe Datei, läuft die Wiedergabe dort ohne
    /// Unterbrechung weiter. Sonst: `keep` (Knopf „zurück“) öffnet die Datei in einer Schnellansicht und
    /// spielt dort weiter; ohne `keep` (Fenster geschlossen) endet die Wiedergabe.
    fn end_pip_to(&mut self, keep: bool) {
        let mut slot = self.pip.take();
        let mut give = |p: &mut Pane| {
            if let Some(pl) = slot.take() {
                slot = p.preview.adopt(pl).err();
            }
            if let (Some(pl), Some(PaneView::View(v))) = (slot.take(), &mut p.extra) {
                slot = v.pv.adopt(pl).err();
            }
        };
        // zuerst der sichtbare Tab (dort läuft es sichtbar weiter), dann alle anderen
        let cur = self.cur.min(self.tabs.len().saturating_sub(1));
        each_pane(&mut self.tabs[cur..=cur], &mut give);
        each_pane(&mut self.tabs, &mut give);
        if let Some(pl) = slot.filter(|_| keep) {
            let path = pl.path.clone();
            let back = self.tabs.get(self.cur).map(|t| t.id);
            let mut pane = Pane::new(path.parent().map_or_else(|| path.clone(), Path::to_path_buf), &self.ctx, &self.st);
            pane.extra = Some(PaneView::View(Box::new(ViewTab { path, pv: preview::Preview::with_player(pl, &self.ctx), back })));
            self.tabs.push(Tab::new(self.next_tab, pane));
            self.next_tab += 1;
            self.cur = self.tabs.len() - 1;
        }
    }

    fn end_pip(&mut self) {
        self.end_pip_to(false);
    }

    /// Schwebendes Bild-in-Bild-Fenster (eigenes Fenster, immer im Vordergrund): Video mit Bedienleiste –
    /// läuft weiter, während im Explorer Tabs und Ordner wechseln.
    fn pip_ui(&mut self, ctx: &egui::Context) {
        let Some(pl) = self.pip.as_mut() else { return };
        let th = self.th;
        const BAR: f32 = 48.0;
        let w = 440.0;
        let size = [w, (w / pl.aspect().clamp(0.4, 3.0)).round() + BAR];
        // Startposition: unten rechts im Hauptfenster
        let pos = *self.pip_pos.get_or_insert_with(|| {
            let main = ctx.input(|i| i.viewport().outer_rect).unwrap_or(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0)));
            [main.right() - size[0] - 32.0, main.bottom() - size[1] - 48.0]
        });
        let name = pl.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("{name} – Bild-in-Bild"))
            .with_inner_size(size)
            .with_min_inner_size([260.0, 190.0])
            .with_position(pos)
            .with_always_on_top();
        let mut close = false;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("foldex_pip"), builder, |ctx, class| {
            let mut body = |ui: &mut egui::Ui| {
                let full = ui.max_rect();
                let bar = Rect::from_min_max(pos2(full.left(), full.bottom() - BAR), full.max);
                pl.video(ui, Rect::from_min_max(full.min, pos2(full.right(), bar.top())));
                ui.painter().rect_filled(bar, 0.0, Color32::from_gray(16));
                pl.controls(ui, &th, bar.shrink2(vec2(10.0, 6.0)), true);
                pl.keys(ui);
            };
            if class == egui::ViewportClass::Embedded {
                // ohne eigene Fenster (Backend kann es nicht): schwebendes Fenster im Explorer
                egui::Window::new("Bild-in-Bild").default_size(size).show(ctx, |ui| {
                    ui.set_min_size(vec2(size[0], size[1]));
                    body(ui);
                });
            } else {
                egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::BLACK)).show(ctx, |ui| body(ui));
                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
            }
        });
        if pl.pip_request {
            self.end_pip_to(true);
        } else if close {
            self.end_pip();
        }
    }

    /// Geschlossenen Tab für Strg+Umschalt+T merken (Schnellansicht: die Datei, sonst Kacheln und Ordner).
    fn remember_closed(&mut self, i: usize) {
        let Some(t) = self.tabs.get(i) else { return };
        let view = match t.panes.as_slice() {
            [p] => match &p.extra {
                Some(PaneView::View(v)) => Some(v.path.clone()),
                _ => None,
            },
            _ => None,
        };
        let c = match view {
            Some(p) => ClosedTab::View(i, p),
            None => ClosedTab::State(i, tab_state(t)),
        };
        self.closed_tabs.push(c);
        if self.closed_tabs.len() > 30 {
            self.closed_tabs.remove(0);
        }
    }

    /// Zuletzt geschlossenen Tab an seiner alten Stelle wieder öffnen.
    fn reopen_tab(&mut self) {
        let Some(c) = self.closed_tabs.pop() else {
            self.status = "Keine geschlossenen Tabs".into();
            return;
        };
        let (at, tab) = match c {
            ClosedTab::View(at, p) if fsx::exists(&p) => {
                let back = self.tabs.get(self.cur).map(|t| t.id);
                let mut pane = Pane::new(p.parent().map_or_else(|| p.clone(), Path::to_path_buf), &self.ctx, &self.st);
                pane.extra = Some(PaneView::View(Box::new(ViewTab { path: p, pv: preview::Preview::default(), back })));
                (at, Tab::new(self.next_tab, pane))
            }
            ClosedTab::View(at, p) => (at, Tab::new(self.next_tab, Pane::new(p.parent().map_or(p.clone(), Path::to_path_buf), &self.ctx, &self.st))),
            ClosedTab::State(at, s) => (at, tab_from_state(&s, self.next_tab, &self.ctx, &self.st, &self.shells)),
        };
        self.next_tab += 1;
        let at = at.min(self.tabs.len());
        self.tabs.insert(at, tab);
        self.cur = at;
    }

    fn close_tab(&mut self, i: usize) {
        // bearbeitete Datei mit ungespeicherten Änderungen: nicht stillschweigend verwerfen
        let unsaved = self.tabs.get(i).is_some_and(|t| t.panes.iter().any(|pn| matches!(&pn.extra, Some(PaneView::View(v)) if v.pv.unsaved())));
        if unsaved {
            self.cur = i;
            self.status = "Ungespeicherte Änderungen – erst speichern (Strg+S) oder „Verwerfen“".into();
            return;
        }
        self.remember_closed(i);
        // Vorschau-Tab geschlossen: zum zuvor aktiven Tab zurückkehren
        let back = self.tabs.get(i).and_then(|t| t.panes.iter().find_map(|pn| if let Some(PaneView::View(v)) = &pn.extra { Some(v.back) } else { None })).flatten();
        if self.tabs.len() == 1 {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            let mut pn = Pane::new(home, &self.ctx, &self.st);
            pn.start = self.start_cfg.close_last == 0; // alles zu → Startseite (einstellbar)
            self.tabs[0].panes = vec![pn];
            self.tabs[0].active = 0;
            self.tabs[0].root = Node::Leaf(0);
            return;
        }
        self.tabs.remove(i);
        // Nach dem Schließen eines Vorschau-Tabs zum vorher aktiven Tab springen, sonst die übliche Anpassung
        if let Some(bi) = back.and_then(|id| self.tabs.iter().position(|t| t.id == id)) {
            self.cur = bi;
        } else if self.cur >= self.tabs.len() {
            self.cur = self.tabs.len() - 1;
        } else if i < self.cur {
            self.cur -= 1;
        }
    }

    fn apply(&mut self, a: Act) {
        match a {
            Act::Go(p) => self.nav(p),
            Act::OpenFile(p) => open_detached(p),
            Act::Open(p) => {
                if fsx::is_dir(&p) {
                    self.nav(p);
                } else if fsx::exists(&p) {
                    open_detached(p);
                } else {
                    self.status = format!("Pfad nicht gefunden: {}", p.display());
                }
            }
            Act::OpenSel => {
                let pn = self.pane();
                if pn.search.is_none() && pn.sel.len() == 1 {
                    let p = pn.sel_paths().remove(0);
                    self.apply(Act::Open(p));
                }
            }
            Act::Preview(p) => self.open_view(p),
            Act::Reveal(p) => {
                if let Some(par) = p.parent() {
                    self.nav(par.to_path_buf());
                    if let Some(n) = p.file_name() {
                        let n = n.to_string_lossy().into_owned();
                        let pn = self.pane_mut();
                        pn.sel.insert(n);
                        pn.sel_dirty = true;
                    }
                }
            }
            Act::SetClip(v, cut) => {
                if !v.is_empty() {
                    // in die Windows-Zwischenablage (auch für Explorer, VS Code …) und intern merken
                    let mut v = v;
                    v.sort();
                    let seq = clip::set(&v, cut);
                    self.status = format!(
                        "{} Element(e) {}{}",
                        v.len(),
                        if cut { "ausgeschnitten" } else { "kopiert" },
                        if cut { " – Esc hebt das Ausschneiden auf" } else { "" }
                    );
                    self.clip = Some(clip::Clip { paths: v, cut, seq });
                }
            }
            Act::PasteInto(dest) => {
                // Hat ein anderes Programm seitdem etwas in die Zwischenablage gelegt, gilt dessen Inhalt
                let ours = self.clip.as_ref().is_some_and(|c| c.seq == 0 || c.seq == clip::seq());
                let src = if ours { self.clip.clone().map(|c| (c.paths, c.cut)) } else { clip::get() };
                match src {
                    Some((srcs, cut)) => {
                        let srcs: Vec<PathBuf> = srcs.into_iter().filter(|p| fsx::exists(&p)).collect();
                        if srcs.is_empty() {
                            self.status = "Die Elemente in der Zwischenablage gibt es nicht mehr".into();
                        } else {
                            if cut {
                                // Ausschneiden gilt nur einmal (wie im Explorer)
                                if let Some(c) = self.clip.take() {
                                    clip::clear_if_ours(c.seq);
                                } else {
                                    clip::clear_if_ours(clip::seq());
                                }
                            }
                            self.spawn_transfer(srcs, dest, !cut);
                        }
                    }
                    None => self.status = "Keine Dateien in der Zwischenablage".into(),
                }
            }
            Act::ClearCut => {
                if let Some(c) = self.clip.take() {
                    if c.cut {
                        clip::clear_if_ours(c.seq);
                        self.status = "Ausschneiden aufgehoben".into();
                    } else {
                        self.clip = Some(c);
                    }
                }
            }
            Act::TransferOther(v, copy) => {
                let t = &self.tabs[self.cur];
                if t.panes.len() > 1 {
                    let order = t.root.leaves();
                    let at = order.iter().position(|&p| p == t.active).unwrap_or(0);
                    let other = order[(at + 1) % order.len()];
                    let dest = t.panes[other].cwd.clone();
                    self.spawn_transfer(v, dest, copy);
                }
            }
            Act::SelectAll if self.pane().search.is_some() => {
                if let Some(sv) = self.pane_mut().search.as_mut() {
                    sv.sel = (0..sv.hits.len()).filter_map(|i| hit_path(sv, i)).collect();
                }
            }
            Act::SelectAll => {
                let pn = self.pane_mut();
                pn.sel = pn
                    .view
                    .iter()
                    .map(|&i| pn.items[i].name.to_string())
                    .collect();
                pn.sel_dirty = true;
            }
            Act::Trash(v) => {
                self.pane_mut().sel.clear();
                self.spawn_trash(v);
            }
            Act::Rename => {
                let pn = self.pane_mut();
                if pn.search.is_none() && pn.sel.len() == 1 {
                    let n = pn.sel.iter().next().unwrap().clone();
                    if let Some(pos) = pn.view.iter().position(|&i| *pn.items[i].name == *n) {
                        pn.ensure_visible(pos);
                    }
                    pn.rename = Some(RenameState {
                        orig: n.clone(),
                        buf: n,
                        first: true,
                    });
                }
            }
            Act::CommitRename => self.commit_rename(),
            Act::CancelRename => self.pane_mut().rename = None,
            Act::Refresh => self.refresh_all(),
            Act::Back => {
                let (ctx, st) = (self.ctx.clone(), self.st.clone());
                let pn = self.pane_mut();
                if let Some(p) = pn.back.pop() {
                    pn.fwd.push(pn.cwd.clone());
                    pn.go(p, false, &ctx, &st);
                }
            }
            Act::Forward => {
                let (ctx, st) = (self.ctx.clone(), self.st.clone());
                let pn = self.pane_mut();
                if let Some(p) = pn.fwd.pop() {
                    pn.back.push(pn.cwd.clone());
                    pn.go(p, false, &ctx, &st);
                }
            }
            Act::Up => {
                let cwd = self.pane().cwd.clone();
                if let Some(p) = cwd.parent() {
                    self.nav(p.to_path_buf());
                    if let Some(n) = cwd.file_name() {
                        let pn = self.pane_mut();
                        pn.sel.insert(n.to_string_lossy().into_owned());
                        pn.sel_dirty = true;
                    }
                }
            }
            Act::NewFolder => self.create(true),
            Act::NewFile => self.create(false),
            Act::FocusAddr => {
                let pn = self.pane_mut();
                pn.crumb_edit = Some(pn.cwd.display().to_string());
                pn.crumb_focus = true;
            }
            Act::FocusSearch => self.pane_mut().focus_search = true,
            Act::CopyText(t) => self.ctx.copy_text(t),
            Act::Pin(p) => {
                if self.side.pin(&p) {
                    self.status = "An Schnellzugriff angeheftet".into();
                }
            }
            Act::Props(p) => self.open_props(p),
            Act::SortKey(s) => {
                let st = self.st.clone();
                let pn = self.pane_mut();
                if pn.sort == s {
                    pn.asc = !pn.asc;
                } else {
                    pn.sort = s;
                    pn.asc = true;
                }
                pn.resort(&st);
            }
            Act::SortDir(asc) => {
                let st = self.st.clone();
                let pn = self.pane_mut();
                pn.asc = asc;
                pn.resort(&st);
            }
            Act::Search => self.run_folder_search(),
            Act::ClearSearch => {
                let st = self.st.clone();
                let pc = matches!(
                    self.pane().search,
                    Some(SearchView {
                        kind: SearchKind::Pc,
                        ..
                    })
                );
                if pc {
                    self.pc_query.clear();
                }
                let pn = self.pane_mut();
                pn.query.clear();
                pn.search = None;
                pn.refresh_view(&st);
            }
            Act::PcSearch => self.run_pc_search(),
            Act::Extract(v, into) => self.spawn_extract(v, into),
            Act::ReindexAll => {
                let roots: Vec<PathBuf> = self.slots.iter().map(|s| s.root.clone()).collect();
                for r in roots {
                    self.queue_index(r);
                }
            }
            Act::Reindex(root) => {
                // gehört der Pfad zu einem bekannten Laufwerk, wird dieses Laufwerk neu indexiert
                let r = self
                    .slots
                    .iter()
                    .filter(|s| root.starts_with(&s.root))
                    .map(|s| s.root.clone())
                    .max_by_key(|p| p.as_os_str().len());
                if let Some(r) = r {
                    self.queue_index(r);
                }
            }
            Act::Unpin(i) => {
                if i < self.side.pins.len() {
                    self.side.pins.remove(i);
                    self.side.save();
                }
            }
            Act::PinMove(from, to) => self.side.move_pin(from, to),
            Act::SideRename(i) => {
                if let Some(e) = self.side.pins.get(i) {
                    self.side_rename = Some((i, e.label.clone()));
                }
            }
            Act::TogglePc => {
                self.side.show_pc = !self.side.show_pc;
                self.side.save();
            }
            Act::RescanGit => {
                self.git_rx = Some(probe_git(&self.ctx, git_roots(&self.side)));
            }
            Act::ResetSidebar => {
                self.side = SidebarCfg::defaults();
                self.side.save();
            }
            Act::TabNew => {
                let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                match self.start_cfg.new_tab {
                    1 => {
                        self.new_tab(home);
                        self.pane_mut().start = true;
                    }
                    2 => {
                        let here = Some(self.pane()).filter(|p| !p.start && p.ctrash.is_none()).map(|p| p.cwd.clone()).unwrap_or(home);
                        self.new_tab(here);
                    }
                    _ => self.new_tab(home),
                }
            }
            Act::Transcribe(paths, srt) => {
                if localllm::stt_ready().is_none() {
                    self.status = "Whisper ist noch nicht eingerichtet – Einstellungen → Lokale Modelle → Spracheingabe".into();
                    self.settings_page = "Lokale Modelle".into();
                    self.show_settings = true;
                } else {
                    let prog = Arc::new(AtomicU64::new(0));
                    let cancel = Arc::new(AtomicBool::new(false));
                    let (p2, c2) = (prog.clone(), cancel.clone());
                    let n = paths.len();
                    let label = if srt { "Untertitel erzeugen" } else { "In Text umwandeln" };
                    self.run_job_ex(label, Some(prog), 1000, Some(cancel), move || {
                        let mut made = Vec::new();
                        let mut err = None;
                        let one = AtomicU64::new(0);
                        for (i, p) in paths.iter().enumerate() {
                            if c2.load(Relaxed) {
                                break;
                            }
                            one.store(0, Relaxed);
                            // Gesamtfortschritt: fertige Dateien + Anteil der laufenden
                            let r = std::thread::scope(|s| {
                                let h = s.spawn(|| transcribe::run(p, srt, &one, &c2));
                                while !h.is_finished() {
                                    p2.store((i as u64 * 1000 + one.load(Relaxed)) / n as u64, Relaxed);
                                    std::thread::sleep(Duration::from_millis(200));
                                }
                                h.join().unwrap_or_else(|_| Err("abgestürzt".into()))
                            });
                            match r {
                                Ok(f) => made.push(f),
                                Err(e) => err = Some(e),
                            }
                        }
                        let what = if srt { "Untertitel" } else { "Transkript" };
                        let msg = match (made.len(), err) {
                            (1, None) => format!("{what} gespeichert: {}", made[0].file_name().unwrap_or_default().to_string_lossy()),
                            (k, None) => format!("{k} × {what} gespeichert"),
                            (_, Some(e)) => e,
                        };
                        JobDone { msg, hist: Hist::None, locked: None }
                    });
                    if let Some(j) = self.jobs.last_mut() {
                        j.pct = true;
                    }
                }
            }
            Act::BatchImages(paths) => {
                self.batch = Some(batch::Opts::new(paths));
            }
            Act::ContentSearch(q) => {
                let dir = self.pane().cwd.clone();
                let s = q.as_deref().map(|q| contentsearch::Search::start(q, &dir, &self.ctx));
                self.csearch = Some((q.unwrap_or_default(), dir, s));
            }
            Act::Print(paths) => {
                let files: Vec<PathBuf> = paths.into_iter().filter(|p| !fsx::is_dir(p) && print::supported(p)).take(30).collect();
                if files.is_empty() {
                    self.status = "Drucken: erst eine Datei in einem druckbaren Format auswählen (z. B. PDF, Text, Bild, Word)".into();
                } else {
                    // eigenes Druckfenster (Vorschau, Drucker, Kopien, Seiten …) – kein fremdes Programm
                    self.printwin = Some(print::PrintWin::new(files, &self.ctx));
                }
            }
            Act::Shortcut(targets, desktop) => {
                let tx = self.defer_tx.clone();
                let ctx = self.ctx.clone();
                std::thread::spawn(move || {
                    let mut made: Vec<PathBuf> = Vec::new();
                    let mut err: Option<String> = None;
                    for t in &targets {
                        let dir = if desktop { dirs::desktop_dir() } else { t.parent().map(Path::to_path_buf) };
                        match dir.ok_or_else(|| "kein Zielordner".to_string()).and_then(|d| shortcut::create(t, &d)) {
                            Ok(p) => made.push(p),
                            Err(e) => err = Some(e),
                        }
                    }
                    let _ = tx.send(Box::new(move |app: &mut App| {
                        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let place = if desktop { " auf dem Desktop" } else { "" };
                        app.status = match (&err, made.len()) {
                            (None, 1) => format!("Verknüpfung „{}“{place} erstellt", name(&made[0])),
                            (None, n) => format!("{n} Verknüpfungen{place} erstellt"),
                            (Some(e), 0) => format!("Verknüpfung fehlgeschlagen: {e}"),
                            (Some(e), n) => format!("{n} Verknüpfungen erstellt – Fehler: {e}"),
                        };
                        if made.len() == 1 {
                            app.history.apply(Hist::Record(Op::Create { path: made[0].clone(), dir: false }));
                        }
                        // Ordner, in denen eine Verknüpfung entstanden ist, neu laden (und die neue markieren)
                        let ctx = app.ctx.clone();
                        each_pane(&mut app.tabs, |p| {
                            if let Some(m) = made.iter().find(|m| m.parent() == Some(p.cwd.as_path())) {
                                p.start_load(&ctx);
                                p.sel.clear();
                                p.sel.insert(name(m));
                                p.sel_dirty = true;
                            }
                        });
                    }));
                    ctx.request_repaint();
                });
            }
            Act::StartHere => {
                let pn = self.pane_mut();
                pn.start = true;
                pn.start_settings = false;
                pn.ctrash = None;
                pn.search = None;
            }
            Act::StartOpen => {
                let p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                self.new_tab(p);
                self.pane_mut().start = true;
            }
            Act::StartGo(p) => {
                self.pane_mut().start = false;
                self.nav(p);
            }
            Act::StartSearch(q) => {
                self.pane_mut().start = false;
                self.pc_query = q;
                self.run_pc_search();
            }
            Act::CloudTrash(key, remote, name) => match remote {
                Some(r) if cloudtrash::in_app(key) => {
                    // schon offen → dorthin wechseln, sonst eigener Tab
                    if let Some(i) = self.tabs.iter().position(|t| t.panes.iter().any(|p| p.ctrash.as_ref().is_some_and(|v| v.remote == r))) {
                        self.cur = i;
                    } else {
                        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                        self.new_tab(home);
                        self.pane_mut().ctrash = Some(Box::new(cloudtrash::TrashView::new(r.clone(), name)));
                    }
                    self.ctrash_load(&r);
                }
                _ => match cloudtrash::web_url(key) {
                    Some(u) => {
                        let _ = open::that_detached(u);
                        self.status = format!("Papierkorb von {name} im Browser geöffnet");
                    }
                    None => self.status = format!("Für {name} gibt es keinen Papierkorb-Link"),
                },
            },
            Act::TabOpen(p) => {
                if fsx::is_dir(&p) {
                    self.new_tab(p);
                } else if let (Some(dir), Some(name)) = (p.parent(), p.file_name()) {
                    self.new_tab(dir.to_path_buf());
                    let pn = self.pane_mut();
                    pn.sel.insert(name.to_string_lossy().into_owned());
                    pn.sel_dirty = true;
                    pn.preview_on = true;
                }
            }
            Act::TabClose(i) => self.close_tab(i),
            Act::TabReopen => self.reopen_tab(),
            Act::TabSelect(i) => {
                if i < self.tabs.len() {
                    self.cur = i;
                }
            }
            Act::TabDup(i) => {
                let t = &self.tabs[i];
                let spec = pane_spec(&t.panes[t.active]);
                let pane = pane_from_spec(&spec, &self.ctx, &self.st, &self.shells);
                self.tabs.push(Tab::new(self.next_tab, pane));
                self.next_tab += 1;
                self.cur = self.tabs.len() - 1;
            }
            Act::TabCloseOthers(i) if self.tabs.iter().enumerate().any(|(k, t)| k != i && t.panes.iter().any(|pn| matches!(&pn.extra, Some(PaneView::View(v)) if v.pv.unsaved()))) => {
                self.status = "Ein anderer Tab hat ungespeicherte Änderungen – erst speichern oder verwerfen".into();
            }
            Act::TabCloseOthers(i) => {
                // von hinten merken: Strg+Umschalt+T bringt sie in umgekehrter Reihenfolge zurück
                for k in (0..self.tabs.len()).rev().filter(|k| *k != i) {
                    self.remember_closed(k);
                }
                let keep = self.tabs.remove(i);
                self.tabs = vec![keep];
                self.cur = 0;
            }
            Act::TabNext(fwd) => {
                let n = self.tabs.len();
                self.cur = if fwd {
                    (self.cur + 1) % n
                } else {
                    (self.cur + n - 1) % n
                };
            }
            Act::ToggleDual => {
                let (ctx, st) = (self.ctx.clone(), self.st.clone());
                let t = &mut self.tabs[self.cur];
                if t.dual() {
                    // alle Teilfenster außer dem aktiven schließen
                    let keep = t.panes.swap_remove(t.active.min(t.panes.len() - 1));
                    t.panes = vec![keep];
                    t.root = Node::Leaf(0);
                    t.active = 0;
                } else {
                    let second = t.panes[0].duplicate(&ctx, &st);
                    t.panes.push(second);
                    t.root.split_leaf(0, Node::Leaf(1), Zone::Right);
                    t.active = 1;
                }
            }
            Act::SwitchPane => {
                let t = &mut self.tabs[self.cur];
                if t.dual() {
                    let order = t.root.leaves();
                    let at = order.iter().position(|&p| p == t.active).unwrap_or(0);
                    t.active = order[(at + 1) % order.len()];
                }
            }
            Act::ShowSettings => self.show_settings = true,
            Act::Cmd(id) => self.run_cmd(id),
            Act::SubNew(pi, spec) => {
                let pane = pane_from_spec(&spec, &self.ctx, &self.st, &self.shells);
                let t = &mut self.tabs[self.cur];
                if let Some(slot) = t.panes.get_mut(pi) {
                    sub_add(slot, pane);
                    t.active = pi;
                }
            }
            Act::SubToTab(pi, k) => {
                let taken = self.tabs[self.cur].panes.get_mut(pi).and_then(|slot| sub_take(slot, k));
                if let Some(p) = taken {
                    let id = self.next_tab;
                    self.next_tab += 1;
                    self.tabs.push(Tab::new(id, p));
                    self.cur = self.tabs.len() - 1;
                }
            }
            Act::SubSwitch(pi, k) => {
                let t = &mut self.tabs[self.cur];
                if let Some(slot) = t.panes.get_mut(pi) {
                    sub_switch(slot, k);
                    t.active = pi;
                }
            }
            Act::SubClose(pi, k) => {
                let t = &mut self.tabs[self.cur];
                let single = t.panes.get(pi).is_some_and(|p| p.stack.is_empty());
                if !single {
                    if let Some(slot) = t.panes.get_mut(pi) {
                        sub_take(slot, k);
                    }
                } else if t.panes.len() > 1 {
                    self.close_pane(pi);
                } else {
                    self.close_tab(self.cur);
                }
            }
            Act::NewWindow(place) => self.new_window(place),
            Act::Terminal(shell, dir) => self.open_terminal(shell, dir),
            Act::GitView(dir) => match git::find_root(&dir) {
                Some(root) => {
                    let found = self.tabs.iter().enumerate().find_map(|(ti, t)| {
                        t.panes.iter().position(|p| matches!(&p.extra, Some(PaneView::Git(g)) if g.root == root)).map(|pi| (ti, pi))
                    });
                    if let Some((ti, pi)) = found {
                        self.cur = ti;
                        self.tabs[ti].active = pi;
                    } else {
                        let view = gitui::GitView::new(root, &self.ctx);
                        self.new_tab(dir);
                        self.pane_mut().extra = Some(PaneView::Git(view));
                    }
                }
                None if self.git_ok => {
                    let view = gitui::GitView::new(dir.clone(), &self.ctx);
                    self.new_tab(dir);
                    self.pane_mut().extra = Some(PaneView::Git(view));
                }
                None => self.status = "Git ist nicht installiert (git-scm.com)".into(),
            },
            Act::OpenIn(key, p) => match self.ed_paths.iter().find(|x| x.0 == key).map(|x| x.2.clone()) {
                Some(exe) => {
                    if let Err(err) = editors::open(&exe, &p) {
                        self.status = err;
                    }
                }
                None => {
                    let name = editors::def(&key).map_or(key.clone(), |d| d.name.to_string());
                    self.status = format!("{name} ist ausgeschaltet oder nicht installiert – Einstellungen → Editoren");
                }
            },
            Act::AiChat(dir, backend, agent) => {
                if !self.st.ai_enabled {
                    self.status = "KI-Funktionen sind in den Einstellungen ausgeschaltet".into();
                } else {
                    let b = self.pick_backend(backend, agent);
                    self.new_tab(dir.clone());
                    self.pane_mut().extra = Some(PaneView::Ai(aiui::ChatView::new(dir, b, agent)));
                }
            }
            Act::EditorAi(agent) => {
                if !self.st.ai_enabled {
                    self.status = "KI-Funktionen sind in den Einstellungen ausgeschaltet".into();
                } else {
                    match &mut self.editor_ai {
                        Some(e) if e.agent_mode == agent && !e.plain => self.editor_ai = None,
                        Some(e) => e.set_mode(agent, false),
                        None => {
                            let b = self.editor_backend();
                            let dir = if self.seen_cwd.as_os_str().is_empty() { self.pane().cwd.clone() } else { self.seen_cwd.clone() };
                            self.editor_ai = Some(aiui::ChatView::editor(dir, b, agent));
                        }
                    }
                }
            }
            // Bild erzeugen: Explorer-KI-Seitenleiste im Modus „Bild“ (letztes Bild-Gespräch wird fortgesetzt)
            Act::FolderIcon(p) => self.folder_icon = Some((p, String::new())),
            Act::CloudSetup => self.cloud_setup = true,
            Act::CloudLogin(key) => {
                self.cloud_setup = true;
                match cloudlogin::auth(key) {
                    Some(cloudlogin::Auth::Form(f)) => self.cloud_form = Some((key, vec![String::new(); f.len()])),
                    Some(_) if !cloudlogin::login_busy() => cloudlogin::start_login(key, Vec::new(), &self.ctx),
                    _ => {}
                }
            }
            Act::CloudMove(from, to) => {
                let vis: Vec<PathBuf> = cloud::places().into_iter().map(|c| c.path).filter(|p| !self.side.hidden.contains(p)).collect();
                cloud::move_place(vis, from, to);
            }
            Act::SideHide(p) => {
                if !self.side.hidden.contains(&p) {
                    self.side.hidden.push(p.clone());
                    self.side.save();
                }
                self.status = format!("„{}“ aus der Seitenleiste entfernt – Rechtsklick auf „Cloud“ bzw. „Dieser PC“ holt es zurück", p.display());
            }
            Act::SideUnhide => {
                self.side.hidden.clear();
                self.side.save();
            }
            Act::CloudLogout(remote) => {
                let name = cloudlogin::accounts().into_iter().find(|a| a.remote == remote).map(|a| a.name).unwrap_or_default();
                if self.pane().cwd.starts_with(cloudlogin::accounts().iter().find(|a| a.remote == remote).map(|a| a.path()).unwrap_or_default()) {
                    if let Some(h) = dirs::home_dir() {
                        self.apply(Act::Go(h));
                    }
                }
                match cloudlogin::logout(&remote) {
                    Ok(()) => self.status = format!("{name}: abgemeldet – Laufwerk getrennt"),
                    Err(e) => self.status = format!("Abmelden: {e}"),
                }
                cloud::refresh();
            }
            Act::CloudConnect(remote, on) => {
                let acc = cloudlogin::accounts().into_iter().find(|a| a.remote == remote);
                if let Some(a) = acc {
                    if !on && self.pane().cwd.starts_with(a.path()) {
                        if let Some(h) = dirs::home_dir() {
                            self.apply(Act::Go(h));
                        }
                    }
                    self.status = if on { format!("{} wird verbunden …", a.name) } else { format!("{}: Laufwerk {}: getrennt", a.name, a.letter) };
                    cloudlogin::set_connected(&remote, on);
                }
                cloud::refresh();
            }
            Act::CloudKeep(paths, keep) => {
                self.status = match cloud::set_keep(&paths, keep) {
                    Ok(n) if keep => format!("{n} Element(e) werden auf diesem Gerät behalten – der Cloud-Dienst lädt sie herunter"),
                    Ok(n) => format!("Speicherplatz für {n} Element(e) wird freigegeben – sie bleiben in der Cloud"),
                    Err(e) => e,
                };
                self.refresh_all();
            }
            Act::Summary(p) => {
                if !self.summaries.iter().any(|x| x.path == p) {
                    let cfg = self.summary_ai_cfg();
                    self.summaries.push(summary::Summary::new(p, cfg, &self.ctx));
                }
            }
            Act::ImageGen(dir) => match &mut self.editor_ai {
                Some(e) if e.image => {}
                Some(e) => e.switch_to_image(),
                None => {
                    let mut c = aiui::ChatView::editor(dir, self.editor_backend(), false);
                    c.switch_to_image();
                    self.editor_ai = Some(c);
                }
            },
            Act::EditorChat => {
                if !self.st.ai_enabled {
                    self.status = "KI-Funktionen sind in den Einstellungen ausgeschaltet".into();
                } else {
                    match &mut self.editor_ai {
                        Some(e) if e.plain => self.editor_ai = None,
                        Some(e) => e.set_mode(false, true),
                        None => {
                            let b = self.editor_backend();
                            let dir = if self.seen_cwd.as_os_str().is_empty() { self.pane().cwd.clone() } else { self.seen_cwd.clone() };
                            let mut c = aiui::ChatView::editor(dir, b, false);
                            c.set_mode(false, true);
                            self.editor_ai = Some(c);
                        }
                    }
                }
            }
            Act::Convert(src, ext) => {
                let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
                let dst = ops::unique_with_ext(&dir, &convert::base_name(&src), &format!(".{ext}"));
                self.spawn_convert(src, dst, false);
            }
            Act::ConvertMany(srcs, ext) => {
                let pairs = srcs
                    .into_iter()
                    .map(|src| {
                        let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
                        let dst = ops::unique_with_ext(&dir, &convert::base_name(&src), &format!(".{ext}"));
                        (src, dst)
                    })
                    .collect();
                self.spawn_convert_many(pairs, false);
            }
            Act::InstallAgent(key) => {
                if let Some(d) = agents::def(&key) {
                    let (prog, args) = agents::install_command(d);
                    let dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
                    match terminal::Term::spawn_args(&format!("{} installieren", d.name), &prog, &args, &dir, &self.ctx) {
                        Ok(t) => {
                            self.new_tab(dir);
                            self.pane_mut().extra = Some(PaneView::Term(t));
                            self.status = format!("{} wird installiert – danach in den Einstellungen „Neu prüfen“", d.name);
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            Act::Share(paths) => {
                let files: Vec<PathBuf> = paths.into_iter().filter(|p| fsx::is_file(&p)).collect();
                if files.is_empty() {
                    self.status = "Teilen: erst eine oder mehrere Dateien auswählen (Ordner vorher komprimieren)".into();
                } else {
                    self.share = Some(share::ShareWin::new(files, &self.ctx, self.hwnd));
                }
            }
            Act::InstallFfmpeg => {
                let dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
                // Windows: winget; Linux: Paketverwaltung der Distribution (fragt im Terminal nach dem Passwort)
                let (prog, args): (&str, Vec<String>) = if cfg!(windows) {
                    ("powershell.exe", ["-NoProfile", "-Command", "winget install --id Gyan.FFmpeg -e"].map(String::from).to_vec())
                } else {
                    let cmd = "if command -v apt >/dev/null; then sudo apt install -y ffmpeg; elif command -v dnf >/dev/null; then sudo dnf install -y ffmpeg; elif command -v pacman >/dev/null; then sudo pacman -S --noconfirm ffmpeg; elif command -v zypper >/dev/null; then sudo zypper install -y ffmpeg; else echo 'Bitte ffmpeg mit der Paketverwaltung installieren'; fi";
                    ("sh", vec!["-c".into(), cmd.into()])
                };
                match terminal::Term::spawn_args("ffmpeg installieren", prog, &args, &dir, &self.ctx) {
                    Ok(t) => {
                        self.new_tab(dir);
                        self.pane_mut().extra = Some(PaneView::Term(t));
                        self.status = "ffmpeg wird installiert – danach lassen sich Videos abspielen und umwandeln".into();
                    }
                    Err(e) => self.status = e,
                }
            }
            Act::OpenWith(p) => {
                if let Err(e) = editors::open_with_dialog(&p) {
                    self.status = format!("„Öffnen mit“ fehlgeschlagen: {e}");
                }
            }
            Act::Tools(tab) => {
                let cwd = self.pane().cwd.clone();
                let other = {
                    let t = &self.tabs[self.cur];
                    let order = t.root.leaves();
                    order.iter().find(|&&i| i != t.active).map(|&i| t.panes[i].cwd.clone())
                };
                match &mut self.tools {
                    Some(w) => {
                        w.tab = tab;
                        if w.task.is_none() {
                            w.a = cwd.display().to_string();
                        }
                    }
                    None => {
                        self.tools = Some(ToolsWin {
                            tab,
                            a: cwd.display().to_string(),
                            b: other.map(|p| p.display().to_string()).unwrap_or_default(),
                            content: false,
                            min_size: "1 MB".into(),
                            task: None,
                            result: None,
                            marked: HashSet::new(),
                        })
                    }
                }
            }
            Act::SelectRules => {
                if self.rules.is_none() {
                    self.rules = Some(RulesWin { kind: tools::RuleKind::Ext, value: String::new(), mode: tools::SelMode::Replace, err: None });
                }
            }
            Act::Undo => self.spawn_hist(true),
            Act::Redo => self.spawn_hist(false),
            Act::Compress(v, f) => {
                if !v.is_empty() {
                    self.spawn_compress(v, f);
                }
            }
            Act::DeletePerm(v) => {
                if !v.is_empty() {
                    self.confirm_del = Some(v);
                }
            }
            Act::SetView(mode) => {
                let pn = self.pane_mut();
                pn.mode = mode;
                pn.pending_scroll = Some(0.0);
                pn.pending_scroll_x = Some(0.0);
                self.st.view_mode = mode.to_u8();
                self.st.save();
            }
            Act::SetGroup(g) => {
                let st = self.st.clone();
                let pn = self.pane_mut();
                pn.group = g;
                pn.pending_scroll = Some(0.0);
                pn.resort(&st);
            }
            Act::SetFilter(f) => {
                let st = self.st.clone();
                let pn = self.pane_mut();
                pn.filter = f;
                pn.pending_scroll = Some(0.0);
                pn.refresh_view(&st);
            }
            Act::SetFoldersFirst(b) => {
                self.st.folders_first = b;
                self.st.save();
                let st = self.st.clone();
                for t in &mut self.tabs {
                    for p in &mut t.panes {
                        p.resort(&st);
                    }
                }
            }
            Act::ShowTrash => self.open_trash(),
            Act::TabMove(from, to) => {
                let n = self.tabs.len();
                if from < n && to < n && from != to {
                    let t = self.tabs.remove(from);
                    self.tabs.insert(to, t);
                    if self.cur == from {
                        self.cur = to;
                    } else if from < self.cur && to >= self.cur {
                        self.cur -= 1;
                    } else if from > self.cur && to <= self.cur {
                        self.cur += 1;
                    }
                }
            }
            Act::TabDetach(i, p) => {
                let ppp = self.ctx.pixels_per_point();
                self.detach_tab(i, pos2(p.x * ppp, p.y * ppp));
            }
            Act::ToggleOrient => {
                let t = &mut self.tabs[self.cur];
                let a = t.active;
                t.root.toggle_parent(a);
            }
            Act::PaneToTab(pi) => self.pane_to_tab(pi),
            Act::Equalize => self.tabs[self.cur].root.equalize(),
            Act::ToggleSidebar => {
                self.side_visible = !self.side_visible;
                self.st.show_sidebar = self.side_visible;
                self.st.save();
            }
        }
    }
}

// ───────────────────────────── UI-Helfer ─────────────────────────────

fn menu_item(ui: &mut egui::Ui, label: &str, hint: &str, enabled: bool) -> bool {
    // mark() setzt „●  “ vor gewählte Optionen → Häkchen
    let (checked, text) = match label.strip_prefix("●  ") {
        Some(t) => (true, t),
        None => (false, label.trim_start()),
    };
    ui::menu_entry(ui, icon_for(text), text, hint, enabled, checked)
}

/// Passendes Symbol zu einem Menüeintrag – einheitlich in allen Menüs.
fn icon_for(label: &str) -> Option<Ic> {
    let l = label.to_lowercase();
    let has = |w: &str| l.contains(w);
    Some(if l.starts_with("öffnen") || has("in neuem tab") || has("öffnen mit") {
        Ic::Open
    } else if has("ausschneiden") {
        Ic::Cut
    } else if has("pfad kopieren") || l.starts_with("kopieren") || has("duplizieren") || has("in anderes panel kopieren") {
        Ic::Copy
    } else if has("einfügen") {
        Ic::Paste
    } else if has("umbenennen") {
        Ic::Rename
    } else if has("löschen") || has("papierkorb") {
        Ic::Trash
    } else if has("eigenschaften") {
        Ic::Info
    } else if has("anheften") || has("lösen von") || has("schnellzugriff") {
        Ic::Pin
    } else if has("einstellungen") {
        Ic::Gear
    } else if has("aktualisieren") || has("neu suchen") || has("neu indexieren") || has("index") {
        Ic::Refresh
    } else if l == "ordner" || has("neuer ordner") {
        Ic::NewFolder
    } else if has("textdokument") || has("textdatei") {
        Ic::Docs
    } else if has("rückgängig") {
        Ic::Undo
    } else if has("wiederholen") {
        Ic::Redo
    } else if has("terminal") {
        Ic::Terminal
    } else if has("git") {
        Ic::Git
    } else if has("fenster") || has("teilfenster") || has("dual-panel") || has("teilung") {
        Ic::Window
    } else if has("entpacken") || has("komprimieren") || has("zip") || has("7z") || has("tar") {
        Ic::Docs
    } else if has("schließen") {
        Ic::Close
    } else if has("seitenleiste") || has("dieser pc") {
        Ic::Sidebar
    } else {
        return None;
    })
}

/// Karte mit Überschrift (Einstellungen).
/// Bereiche der Einstellungen: (Titel = Titel der Karte, Symbol, Kurzbeschreibung).
const SETTINGS_PAGES: &[(&str, Ic, &str)] = &[
    ("Darstellung", Ic::Eye, "Farbschema, Ansicht, Seitenleiste"),
    ("Themes", Ic::Star, "Farben auswählen, anpassen, importieren"),
    ("Icons", Ic::Grid, "Symbole für Dateien und Ordner"),
    ("Dateien", Ic::Docs, "Dateiliste, Vorschau, Fenster wiederherstellen"),
    ("Suche", Ic::Search, "Index und Suche"),
    ("Terminal", Ic::Terminal, "Shell und Vervollständigung"),
    ("Editoren", Ic::Code, "Programme zum Öffnen von Dateien und Ordnern"),
    ("KI", Ic::Spark, "Agenten, eigene Modelle, Rechte"),
    ("Lokale Modelle", Ic::Download, "Text- und Bildmodelle laden und ausführen – statt LM Studio"),
    ("Bildeditor-KI", Ic::Spark, "Modelle für Freistellen, Objekte, Kolorieren, Hochskalieren"),
    ("Schnellleiste", Ic::Tools, "Knöpfe über der Dateiliste"),
    ("Tastenkürzel", Ic::Keyboard, "Befehle und ihre Tasten"),
    ("Import / Export", Ic::Download, "Einstellungen als JSON-Datei übertragen"),
    ("Updates", Ic::Download, "Neue Versionen von GitHub laden und installieren"),
    ("Deinstallieren", Ic::Trash, "Foldex mit allen Daten und Modellen entfernen"),
];

/// Update-Anzeige: kompakt (Statuszeile, nur wenn es etwas gibt) oder ausführlich (Einstellungen)
fn update_ui(ui: &mut egui::Ui, th: Theme, up: &update::Updater, quit: &Arc<AtomicBool>, compact: bool) {
    let install = |ui: &mut egui::Ui| {
        let q = quit.clone();
        up.install(ui.ctx(), move |r| {
            if r.is_ok() {
                q.store(true, Relaxed);
            }
        });
    };
    match up.state() {
        update::State::Available(v) => {
            let txt = if compact { format!("Foldex {v} verfügbar – aktualisieren") } else { format!("Jetzt auf {v} aktualisieren") };
            if ui.add(egui::Button::new(egui::RichText::new(txt).size(12.5).color(th.accent))).on_hover_text("Lädt den neuesten Installer von GitHub, installiert ihn und startet Foldex neu").clicked() {
                install(ui);
            }
            if !compact {
                ui.label(egui::RichText::new(format!("Installiert ist {}. Einstellungen, Verlauf und Modelle bleiben erhalten.", update::current())).color(th.sub));
            }
        }
        update::State::Downloading => {
            let (g, t) = (up.got.load(Relaxed), up.total.load(Relaxed));
            ui.spinner();
            if t > 0 {
                ui.add(egui::ProgressBar::new(g as f32 / t as f32).desired_width(if compact { 120.0 } else { 260.0 }).desired_height(6.0));
            }
            ui.label(egui::RichText::new(format!("Update wird geladen … {}", human(g))).size(12.5).color(th.sub));
        }
        _ if compact => {}
        s => {
            let msg = match &s {
                update::State::Checking => "Suche nach Updates …".to_string(),
                update::State::UpToDate => format!("Foldex {} ist die neueste Version.", update::current()),
                update::State::Failed(e) => e.clone(),
                _ => format!("Installiert: Foldex {}", update::current()),
            };
            ui.label(egui::RichText::new(msg).color(th.sub));
            if ui.add_enabled(!matches!(s, update::State::Checking), egui::Button::new("Nach Updates suchen")).clicked() {
                up.check(ui.ctx());
            }
            if matches!(s, update::State::Failed(_)) && ui.button("Installer trotzdem laden und ausführen").clicked() {
                install(ui);
            }
        }
    }
    if !compact {
        ui.hyperlink_to("Foldex auf GitHub", update::PAGE_URL);
    }
}

fn settings_card(ui: &mut egui::Ui, th: Theme, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    // nur der gewählte Bereich wird gezeigt (sein Titel steht schon als Seitenüberschrift)
    if ui.ctx().data(|d| d.get_temp::<String>(Id::new("fc_settings_page"))).is_some_and(|p| p != title) {
        return;
    }
    let _ = th;
    egui::Frame::none()
        .fill(th.field)
        .rounding(Rounding::same(4.0))
        .stroke(Stroke::new(1.0, th.line))
        .inner_margin(Margin::symmetric(14.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            add(ui);
        });
}

/// Zeile: links Titel + Beschreibung, rechts das Bedienelement.
fn setting_row(ui: &mut egui::Ui, th: Theme, title: &str, desc: &str, controls: impl FnOnce(&mut egui::Ui)) {
    let full = ui.available_width();
    // Text so breit wie möglich, rechts bleibt Platz für Schalter, Auswahl oder Knöpfe
    let text_w = (full - 270.0).clamp(full * 0.45, full * 0.66);
    ui.horizontal(|ui| {
        ui.allocate_ui(vec2(text_w, 0.0), |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(egui::RichText::new(title).size(14.5));
                ui.label(egui::RichText::new(desc).size(12.0).color(th.sub));
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), controls);
    });
}

/// Schalter mit gleitendem Knopf.
fn toggle_switch(ui: &mut egui::Ui, th: Theme, t: f32, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(42.0, 23.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let k = ui.ctx().animate_bool_with_time(resp.id, *on, t);
    let r = rect.height() / 2.0;
    let off = Color32::from_rgb(110, 110, 118);
    let col = Color32::from_rgb(
        (off.r() as f32 + (th.accent.r() as f32 - off.r() as f32) * k) as u8,
        (off.g() as f32 + (th.accent.g() as f32 - off.g() as f32) * k) as u8,
        (off.b() as f32 + (th.accent.b() as f32 - off.b() as f32) * k) as u8,
    );
    ui.painter().rect_filled(rect, r, col);
    let cx = rect.left() + r + (rect.width() - 2.0 * r) * k;
    let kr = r - 3.0 + if resp.hovered() { 0.8 } else { 0.0 };
    ui.painter().circle_filled(pos2(cx, rect.center().y), kr, Color32::WHITE);
    resp
}

/// Segmentierte Auswahl (wie bei Windows 11): die gewählte Option ist farbig hinterlegt.
fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, th: Theme, t: f32, cur: &mut T, opts: &[(T, &str)]) {
    let font = FontId::proportional(13.0);
    let widths: Vec<f32> = opts
        .iter()
        .map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), th.text).size().x + 22.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 30.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, th.bg);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, th.line));
    let mut x = rect.left() + 3.0;
    for (i, (v, label)) in opts.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(widths[i], 24.0));
        let resp = ui.interact(r, ui.id().with(("seg", i, rect.min.x as i32, rect.min.y as i32)), Sense::click());
        let sel = *cur == *v;
        let k = ui.ctx().animate_bool_with_time(resp.id, sel, t);
        if k > 0.0 {
            ui.painter().rect_filled(r, 3.0, th.accent.gamma_multiply(k));
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 3.0, th.hover);
        }
        let tc = if sel { Color32::WHITE } else { th.text };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *label, font.clone(), tc);
        if resp.clicked() {
            *cur = *v;
        }
        x += widths[i];
    }
}

/// Einzeiliger Text, links an `pos` (vertikal zentriert), bei Bedarf mit „…“ auf `max_w` gekürzt.
fn text_trunc(p: &egui::Painter, pos: Pos2, text: &str, font: FontId, color: Color32, max_w: f32) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(20.0));
    let g = p.layout_job(job);
    p.galley(pos2(pos.x, pos.y - g.size().y / 2.0), g, color);
}

enum ThemeAct {
    Select(String),
    Apply(String),
    Export(String),
    Delete(String),
    Import,
    Reset,
}

/// Zeile in der Theme-Liste: Farbstreifen, Name, Typ und ob das Theme gerade verwendet wird.
fn theme_row(ui: &mut egui::Ui, th: Theme, p: &themes::Palette, selected: bool, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 3.0, th.sel);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, th.hover);
    }
    let sw = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 7.0), vec2(48.0, 14.0));
    let t = &p.t;
    for (i, c) in [t.bg, t.panel, t.text, t.accent].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(sw.left() + 12.0 * i as f32, sw.top()), vec2(12.0, sw.height()));
        ui.painter().rect_filled(r, 0.0, c);
    }
    ui.painter().rect_stroke(sw, 0.0, Stroke::new(1.0, th.line));
    let right = rect.right() - 10.0;
    let tag = format!("{}{}", if active { "aktiv · " } else { "" }, if t.dark { "Dunkel" } else { "Hell" });
    let g = ui.painter().layout_no_wrap(tag, FontId::proportional(12.0), if active { th.accent } else { th.sub });
    let tag_w = g.size().x;
    ui.painter().galley(pos2(right - tag_w, rect.center().y - g.size().y / 2.0), g, th.sub);
    let x = sw.right() + 10.0;
    text_trunc(ui.painter(), pos2(x, rect.center().y), &p.name, FontId::proportional(14.0), th.text, right - tag_w - 12.0 - x);
    resp
}

/// Vorschau eines Themes, ohne es anzuwenden: Seitenleiste, Dateiliste und ein Code-Ausschnitt
/// mit Zeilennummern, Zeilenhervorhebung, Auswahl, Suchtreffer und Cursor.
fn paint_theme_preview(ui: &mut egui::Ui, t: &Theme, name: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 168.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, t.bg);
    let f = |s: f32| FontId::proportional(s);
    // Seitenleiste
    let side = Rect::from_min_max(rect.min, pos2(rect.left() + 130.0, rect.bottom()));
    p.rect_filled(side, 0.0, t.panel);
    p.text(pos2(side.left() + 12.0, side.top() + 14.0), Align2::LEFT_CENTER, "Schnellzugriff", f(11.0), t.sub);
    for (i, l) in ["Desktop", "Projekte", "Downloads"].iter().enumerate() {
        let y = side.top() + 36.0 + i as f32 * 24.0;
        let r = Rect::from_min_size(pos2(side.left() + 6.0, y - 11.0), vec2(side.width() - 12.0, 22.0));
        if i == 1 {
            p.rect_filled(r, 3.0, t.hover);
            p.rect_filled(Rect::from_min_size(pos2(r.left(), y - 7.0), vec2(3.0, 14.0)), 1.0, t.accent);
        }
        p.text(pos2(r.left() + 12.0, y), Align2::LEFT_CENTER, *l, f(13.0), t.text);
    }
    p.text(pos2(side.left() + 12.0, side.bottom() - 14.0), Align2::LEFT_CENTER, name, f(11.0), t.sub);
    // Dateiliste
    let list = Rect::from_min_max(pos2(side.right(), rect.top()), pos2(rect.right(), rect.top() + 64.0));
    for (i, (n, d)) in [("main.rs", "Heute 09:12"), ("Cargo.toml", "Gestern 18:40")].iter().enumerate() {
        let r = Rect::from_min_size(pos2(list.left() + 8.0, list.top() + 8.0 + i as f32 * 26.0), vec2(list.width() - 16.0, 24.0));
        if i == 0 {
            p.rect_filled(r, 3.0, t.sel);
        }
        p.text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, *n, f(13.0), t.text);
        p.text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, *d, f(12.0), t.sub);
    }
    p.line_segment([pos2(list.left(), list.bottom()), pos2(list.right(), list.bottom())], Stroke::new(1.0, t.line));
    // Code
    let code = Rect::from_min_max(pos2(side.right(), list.bottom() + 1.0), rect.max);
    p.rect_filled(code, 0.0, t.ed_bg);
    let mono = FontId::monospace(12.5);
    let lines: [&[(&str, Color32)]; 4] = [
        &[("// Beispiel", t.comment)],
        &[("fn ", t.keyword), ("main", t.function), ("() {", t.ed_text)],
        &[("    let ", t.keyword), ("n", t.ed_text), (": ", t.ed_text), ("u32", t.ty), (" = ", t.ed_text), ("42", t.number), (";", t.ed_text)],
        &[("    println!", t.function), ("(", t.ed_text), ("\"Hallo\"", t.string), (");", t.ed_text)],
    ];
    let lh = 22.0;
    let cw = p.layout_no_wrap("0".into(), mono.clone(), t.ed_text).size().x;
    for (i, toks) in lines.iter().enumerate() {
        let y = code.top() + 8.0 + i as f32 * lh;
        let row = Rect::from_min_size(pos2(code.left(), y), vec2(code.width(), lh));
        if i == 2 {
            p.rect_filled(row, 0.0, t.line_hl);
        }
        p.text(pos2(code.left() + 26.0, row.center().y), Align2::RIGHT_CENTER, (i + 1).to_string(), mono.clone(), t.line_no);
        let x0 = code.left() + 40.0;
        if i == 3 {
            // Suchtreffer „Hallo“
            p.rect_filled(Rect::from_min_size(pos2(x0 + cw * 13.0, row.top() + 3.0), vec2(cw * 5.0, lh - 6.0)), 2.0, t.find_hl);
        }
        if i == 1 {
            // Auswahl „main“ + Cursor dahinter
            p.rect_filled(Rect::from_min_size(pos2(x0 + cw * 3.0, row.top() + 2.0), vec2(cw * 4.0, lh - 4.0)), 0.0, t.ed_sel);
            p.rect_filled(Rect::from_min_size(pos2(x0 + cw * 7.0, row.top() + 3.0), vec2(1.5, lh - 6.0)), 0.0, t.cursor);
        }
        let mut job = egui::text::LayoutJob::default();
        for (s, c) in toks.iter() {
            job.append(s, 0.0, egui::TextFormat::simple(mono.clone(), *c));
        }
        let g = p.layout_job(job);
        p.galley(pos2(x0, row.center().y - g.size().y / 2.0), g, t.ed_text);
    }
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.line));
}

/// Hinweis- bzw. Fehlerkasten mit mehreren Zeilen.
fn message_box(ui: &mut egui::Ui, th: Theme, error: bool, lines: &[String]) {
    let red = if th.dark { Color32::from_rgb(255, 120, 110) } else { Color32::from_rgb(180, 30, 20) };
    egui::Frame::none()
        .fill(th.field)
        .stroke(Stroke::new(1.0, if error { red } else { th.line }))
        .rounding(Rounding::same(3.0))
        .inner_margin(Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            for (i, l) in lines.iter().enumerate() {
                let t = egui::RichText::new(l).size(12.5);
                ui.label(if i == 0 && error { t.color(red).strong() } else { t });
            }
        });
}

fn same_pins(a: &[Entry], b: &[Entry]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.label == y.label && x.path == y.path)
}

/// Lesbare Liste der Unterschiede zweier Einstellungen (Import-Vorschau).
fn settings_diff(a: &Settings, b: &Settings, theme_name: &dyn Fn(&str) -> String) -> Vec<String> {
    let mut v = Vec::new();
    let mode = |m: u8| ["System", "Hell", "Dunkel"][m.min(2) as usize];
    let view = |m: u8| ["Details", "Liste", "Raster", "Symbole"][m.min(3) as usize];
    let onoff = |x: bool| if x { "an" } else { "aus" };
    if a.theme != b.theme {
        v.push(format!("Farbschema: {} → {}", mode(a.theme), mode(b.theme)));
    }
    if a.theme_light != b.theme_light {
        v.push(format!("Helles Theme: {} → {}", theme_name(&a.theme_light), theme_name(&b.theme_light)));
    }
    if a.theme_dark != b.theme_dark {
        v.push(format!("Dunkles Theme: {} → {}", theme_name(&a.theme_dark), theme_name(&b.theme_dark)));
    }
    if a.view_mode != b.view_mode {
        v.push(format!("Standardansicht: {} → {}", view(a.view_mode), view(b.view_mode)));
    }
    for (label, x, y) in [
        ("Terminal-Shell", &a.term_shell, &b.term_shell),
        ("Modell der Explorer-KI", &a.ai_agent, &b.ai_agent),
    ] {
        if x != y {
            let show = |s: &str| if s.is_empty() { "automatisch".to_string() } else { s.to_string() };
            v.push(format!("{label}: {} → {}", show(x), show(y)));
        }
    }
    if a.editor_paths != b.editor_paths || a.editor_on != b.editor_on || a.custom_editors != b.custom_editors {
        v.push("Editoren: andere Auswahl bzw. Programmpfade".into());
    }
    if a.ai_models != b.ai_models || a.agent_on != b.agent_on || a.agent_model != b.agent_model {
        v.push(format!("KI: {} eigene Modelle, Agenten-Einstellungen", b.ai_models.len()));
    }
    if a.ai_auto != b.ai_auto || a.ai_cli_full != b.ai_cli_full {
        v.push("KI: andere Rechte für den Agenten".into());
    }
    if a.quickbar != b.quickbar {
        v.push("Schnellleiste: andere Knöpfe bzw. Reihenfolge".into());
    }
    if a.sys_icons != b.sys_icons || a.icon_set != b.icon_set {
        v.push(format!("Icon-Satz: {}", if b.sys_icons { "System" } else { matico::Set::from_key(&b.icon_set).label() }));
    }
    for (label, x, y) in [
        ("Animationen", a.animations, b.animations),
        ("Seitenleiste", a.show_sidebar, b.show_sidebar),
        ("Git-Repositories in der Seitenleiste", a.show_git, b.show_git),
        ("Versteckte Dateien", a.show_hidden, b.show_hidden),
        ("Geschützte Systemdateien", a.show_system, b.show_system),
        ("Ordner zuerst", a.folders_first, b.folders_first),
        ("Index beim Start", a.auto_index, b.auto_index),
        ("Zuletzt verwendete Ordner", a.show_recent, b.show_recent),
        ("Ordnergrößen", a.folder_sizes, b.folder_sizes),
        ("Git-Status markieren", a.git_marks, b.git_marks),
        ("KI-Funktionen", a.ai_enabled, b.ai_enabled),
    ] {
        if x != y {
            v.push(format!("{label}: {} → {}", onoff(x), onoff(y)));
        }
    }
    v
}

const GROUP_H: f32 = 34.0;

/// Anzeigename wie im Windows-Explorer: Dort heißen einige Unterordner von „Bilder“ lokalisiert
/// (z. B. „Screenshots“ → „Bildschirmfotos“). Der echte Ordnername auf der Platte bleibt unverändert.
fn display_name<'a>(name: &'a str, dir: bool, cwd: &Path) -> std::borrow::Cow<'a, str> {
    if cfg!(windows) && dir {
        let in_pictures = cwd
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("Pictures") || n.eq_ignore_ascii_case("Bilder"));
        if in_pictures {
            let l = match name.to_ascii_lowercase().as_str() {
                "screenshots" => "Bildschirmfotos",
                "camera roll" => "Eigene Aufnahmen",
                "saved pictures" => "Gespeicherte Bilder",
                _ => return name.into(),
            };
            return l.into();
        }
    }
    name.into()
}

fn mark(on: bool) -> &'static str {
    if on {
        "●  "
    } else {
        "     "
    }
}

fn menu_view(ui: &mut egui::Ui, pn: &Pane, acts: &mut Vec<Act>) {
    for m in ViewMode::ALL {
        if menu_item(ui, &format!("{}{}", mark(pn.mode == m), m.label()), "", true) && pn.mode != m {
            acts.push(Act::SetView(m));
        }
    }
}

fn menu_sort(ui: &mut egui::Ui, pn: &Pane, st: &Settings, acts: &mut Vec<Act>) {
    for (s, l) in [
        (SortBy::Name, "Name"),
        (SortBy::Date, "Änderungsdatum"),
        (SortBy::Type, "Typ"),
        (SortBy::Size, "Größe"),
    ] {
        if menu_item(ui, &format!("{}{l}", mark(pn.sort == s)), "", true) && pn.sort != s {
            acts.push(Act::SortKey(s));
        }
    }
    ui.separator();
    if menu_item(ui, &format!("{}Aufsteigend", mark(pn.asc)), "", true) {
        acts.push(Act::SortDir(true));
    }
    if menu_item(ui, &format!("{}Absteigend", mark(!pn.asc)), "", true) {
        acts.push(Act::SortDir(false));
    }
    ui.separator();
    if menu_item(ui, &format!("{}Ordner zuerst (bei Name/Typ)", mark(st.folders_first)), "", true) {
        acts.push(Act::SetFoldersFirst(!st.folders_first));
    }
}

fn menu_group(ui: &mut egui::Ui, pn: &Pane, acts: &mut Vec<Act>) {
    for g in GroupBy::ALL {
        if menu_item(ui, &format!("{}{}", mark(pn.group == g), g.label()), "", true) && pn.group != g {
            acts.push(Act::SetGroup(g));
        }
    }
}

fn menu_filter(ui: &mut egui::Ui, pn: &Pane, acts: &mut Vec<Act>) {
    let mut f = pn.filter;
    ui.label(egui::RichText::new("Typ").strong().size(12.0));
    for c in Cat::ALL {
        if ui.radio(f.cat == c, c.label()).clicked() {
            f.cat = c;
        }
    }
    ui.separator();
    ui.label(egui::RichText::new("Änderungsdatum").strong().size(12.0));
    for d in DateF::ALL {
        if ui.radio(f.date == d, d.label()).clicked() {
            f.date = d;
        }
    }
    ui.separator();
    ui.label(egui::RichText::new("Größe (Dateien)").strong().size(12.0));
    for s in SizeF::ALL {
        if ui.radio(f.size == s, s.label()).clicked() {
            f.size = s;
        }
    }
    ui.separator();
    if ui.add_enabled(f.active(), egui::Button::new("Filter zurücksetzen")).clicked() {
        f = Filter::default();
    }
    if f != pn.filter {
        acts.push(Act::SetFilter(f));
    }
}

/// Vektor-Symbol für bekannte Benutzerordner (wie im FoldexViewer).
/// Farbe der Startseiten-Plättchen je Ordnerart (wie man sie von Desktop-Symbolen kennt)
fn quick_color(ic: Ic, accent: Color32) -> Color32 {
    match ic {
        Ic::Desktop => Color32::from_rgb(79, 140, 247),
        Ic::Download => Color32::from_rgb(46, 182, 125),
        Ic::Docs => Color32::from_rgb(242, 153, 74),
        Ic::Pictures => Color32::from_rgb(155, 108, 246),
        Ic::Music => Color32::from_rgb(236, 72, 153),
        Ic::Video => Color32::from_rgb(239, 68, 68),
        _ => accent,
    }
}

/// zwei Farben mischen (t = 0 → a, 1 → b)
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn quick_icon(p: &Path) -> Option<Ic> {
    let is = |d: Option<PathBuf>| d.as_deref() == Some(p);
    if is(dirs::desktop_dir()) {
        Some(Ic::Desktop)
    } else if is(dirs::download_dir()) {
        Some(Ic::Download)
    } else if is(dirs::document_dir()) {
        Some(Ic::Docs)
    } else if is(dirs::picture_dir()) {
        Some(Ic::Pictures)
    } else if is(dirs::audio_dir()) {
        Some(Ic::Music)
    } else if is(dirs::video_dir()) {
        Some(Ic::Video)
    } else if is(dirs::home_dir()) {
        Some(Ic::Home)
    } else {
        None
    }
}

/// Andock-Kompass: vier Zielfelder um die Mitte einer Kachel.
fn compass(r: Rect) -> [(Zone, Rect); 4] {
    let c = r.center();
    let d = (r.width().min(r.height()) * 0.18).clamp(34.0, 48.0);
    let sz = vec2(34.0, 34.0);
    [
        (Zone::Left, Rect::from_center_size(c - vec2(d, 0.0), sz)),
        (Zone::Right, Rect::from_center_size(c + vec2(d, 0.0), sz)),
        (Zone::Top, Rect::from_center_size(c - vec2(0.0, d), sz)),
        (Zone::Bottom, Rect::from_center_size(c + vec2(0.0, d), sz)),
    ]
}

/// Mitte des Kompasses: als Tab in diese Kachel aufnehmen.
fn compass_center(r: Rect) -> Rect {
    Rect::from_center_size(r.center(), vec2(34.0, 34.0))
}

fn paint_compass_center(p: &egui::Painter, th: &Theme, b: Rect, hot: bool, alpha: f32) {
    p.rect_filled(b.expand(1.0).translate(vec2(0.0, 2.0)), 8.0, Color32::from_black_alpha((50.0 * alpha) as u8));
    p.rect(b, 8.0, if hot { th.accent.gamma_multiply(alpha) } else { th.panel.gamma_multiply(alpha) }, Stroke::new(1.0, th.line.gamma_multiply(alpha)));
    let inner = b.shrink(8.0);
    let col = if hot { Color32::WHITE } else { th.text };
    p.rect_stroke(inner, 2.0, Stroke::new(1.2, col.gamma_multiply(alpha)));
    // kleine Tab-Leiste oben
    p.rect_filled(Rect::from_min_size(inner.min, vec2(inner.width() * 0.45, 4.0)), 1.0, col.gamma_multiply(alpha));
    p.line_segment([pos2(inner.left(), inner.top() + 4.0), pos2(inner.right(), inner.top() + 4.0)], Stroke::new(1.0, col.gamma_multiply(alpha)));
}

/// Ein Kompass-Feld: Rahmen mit dem Teil, der belegt wird (gefüllt).
fn paint_compass_button(p: &egui::Painter, th: &Theme, z: Zone, b: Rect, hot: bool, alpha: f32) {
    p.rect_filled(b.expand(1.0).translate(vec2(0.0, 2.0)), 8.0, Color32::from_black_alpha((50.0 * alpha) as u8));
    p.rect(b, 8.0, if hot { th.accent.gamma_multiply(alpha) } else { th.panel.gamma_multiply(alpha) }, Stroke::new(1.0, th.line.gamma_multiply(alpha)));
    let inner = b.shrink(8.0);
    let col = if hot { Color32::WHITE } else { th.text };
    p.rect_stroke(inner, 2.0, Stroke::new(1.2, col.gamma_multiply(alpha)));
    p.rect_filled(z.rect(inner), 2.0, col.gamma_multiply(0.8 * alpha));
}

/// Symbol eines Schnellleisten-Knopfs (Einstellungen).
fn qb_icon(id: &str) -> Ic {
    match id {
        "new" => Ic::Plus,
        "newfolder" => Ic::NewFolder,
        "cut" => Ic::Cut,
        "copy" | "copypath" => Ic::Copy,
        "paste" => Ic::Paste,
        "rename" => Ic::Rename,
        "trash" => Ic::Trash,
        "undo" => Ic::Undo,
        "redo" => Ic::Redo,
        "sort" => Ic::Sort,
        "view" => Ic::List,
        "filter" => Ic::Search,
        "select" => Ic::Select,
        "tools" => Ic::Tools,
        "terminal" => Ic::Terminal,
        "git" => Ic::Git,
        "ai" => Ic::Spark,
        "split" => Ic::Split,
        "props" => Ic::Info,
        "refresh" => Ic::Refresh,
        "compress" => Ic::Docs,
        "openin" => Ic::Code,
        "hidden" => Ic::Eye,
        "preview" => Ic::Docs,
        "share" => Ic::Share,
        _ => Ic::Command,
    }
}

/// Aufklappmenü unter einem Knopf (wie im FoldexViewer).
fn dropdown(ui: &mut egui::Ui, resp: &egui::Response, id: Id, content: impl FnOnce(&mut egui::Ui)) {
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup_below_widget(ui, id, resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(250.0);
        content(ui);
    });
}

fn tools_items(ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    for (ic, label, t) in [
        (Ic::Compare, "Ordner vergleichen", ToolTab::Compare),
        (Ic::Copy, "Duplikate finden", ToolTab::Dups),
        (Ic::Chart, "Speicherplatz analysieren", ToolTab::Space),
        (Ic::Chart, "Große Dateien", ToolTab::Large),
    ] {
        if ui::menu_entry(ui, Some(ic), label, "", true, false) {
            acts.push(Act::Tools(t));
        }
    }
}

#[derive(Clone, Copy)]
enum Arrow {
    Left,
    Right,
    Up,
}

fn nav_button(ui: &mut egui::Ui, th: &Theme, a: Arrow, enabled: bool) -> bool {
    let (ic, tip) = match a {
        Arrow::Left => (Ic::Back, "Zurück (Alt+←)"),
        Arrow::Right => (Ic::Forward, "Vorwärts (Alt+→)"),
        Arrow::Up => (Ic::Up, "Übergeordneter Ordner (Alt+↑)"),
    };
    enabled && ui::tool_btn(ui, th, ic, tip, enabled, false).clicked()
}

fn paint_chevron_right(p: &egui::Painter, c: egui::Pos2, col: Color32) {
    let s = Stroke::new(1.3, col);
    p.line_segment([c + vec2(-2.0, -4.0), c + vec2(2.0, 0.0)], s);
    p.line_segment([c + vec2(2.0, 0.0), c + vec2(-2.0, 4.0)], s);
}

// ───────────────────────────── Oberfläche ─────────────────────────────

impl App {
    fn frame_begin(&mut self, ctx: &egui::Context) {
        self.zones_prev = std::mem::take(&mut self.zones);
        let pos = ctx.input(|i| i.pointer.latest_pos());
        self.drop_target = pos.and_then(|p| {
            self.zones_prev
                .iter()
                .rev()
                .find(|(r, _)| r.contains(p))
                .map(|(_, d)| d.clone())
        });
    }

    fn keys(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        // Vollbild-Video bzw. Maus über einem Player: Leertaste, Pfeile usw. steuern die Wiedergabe
        let mut player_keys = false;
        each_pane(&mut self.tabs, |p| {
            player_keys |= p.preview.wants_keys();
            if let Some(PaneView::View(v)) = &p.extra {
                player_keys |= v.pv.wants_keys();
            }
        });
        if player_keys {
            return;
        }
        // Palette und Tastenkürzel-Aufnahme verarbeiten ihre Tasten selbst
        if self.palette.is_some() || self.key_rec.is_some() {
            return;
        }
        let typing = ctx.wants_keyboard_input();
        let presses: Vec<(Shortcut, egui::Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => {
                        Some((Shortcut { ctrl: modifiers.command, shift: modifiers.shift, alt: modifiers.alt, key: *key }, *modifiers))
                    }
                    _ => None,
                })
                .collect()
        });
        let view_tab = self.pane().extra.is_some();
        for (sc, mods) in presses {
            let Some(id) = self.keymap.lookup(sc) else { continue };
            // beim Tippen (Adressleiste, Terminal …) nur Kürzel mit Strg/Alt, die nichts an Dateien ändern
            let global = (sc.ctrl || sc.alt) && !id.starts_with("file.") && !id.starts_with("go.") && id != "tab.close";
            if typing && !global {
                continue;
            }
            if view_tab && (id.starts_with("file.") || id.starts_with("view.")) {
                continue;
            }
            if typing {
                ctx.input_mut(|i| i.consume_key(mods, sc.key));
            }
            acts.push(Act::Cmd(id));
        }
        // Vorschau-Tab (Leertaste): Esc oder nochmals Leertaste schließt ihn und kehrt zum vorigen Tab zurück
        // (liegt die Maus über einem Player, hat dieser die Tasten schon oben abgefangen)
        if view_tab && !typing && matches!(self.pane().extra, Some(PaneView::View(_))) && ctx.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Space)) {
            acts.push(Act::TabClose(self.cur));
        }
        if typing || view_tab {
            return;
        }
        struct K {
            copy: bool,
            cut: bool,
            paste: bool,
            text: String,
            shift: bool,
            ctrl: bool,
            alt: bool,
            keys: Vec<Key>,
        }
        let all = [
            Key::ArrowDown,
            Key::ArrowUp,
            Key::Home,
            Key::End,
            Key::PageDown,
            Key::PageUp,
            Key::Enter,
            Key::Space,
            Key::Backspace,
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::V,
            Key::Escape,
        ];
        let k = ctx.input(|i| K {
            copy: i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
            cut: i.events.iter().any(|e| matches!(e, egui::Event::Cut)),
            paste: i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
            text: i
                .events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Text(t) = e {
                        Some(t.as_str())
                    } else {
                        None
                    }
                })
                .collect(),
            shift: i.modifiers.shift,
            ctrl: i.modifiers.command,
            alt: i.modifiers.alt,
            keys: all
                .iter()
                .copied()
                .filter(|&key| i.key_pressed(key))
                .collect(),
        });
        let has = |key: Key| k.keys.contains(&key);
        let searching = self.pane().search.is_some();
        let paths = pane_targets(self.pane());
        let cwd = self.pane().cwd.clone();
        // Kopieren/Ausschneiden/Einfügen sind fest belegt (Ereignisse des Betriebssystems)
        if k.copy {
            acts.push(Act::SetClip(paths.clone(), false));
        }
        if k.cut {
            acts.push(Act::SetClip(paths.clone(), true));
        }
        // egui liefert für Strg+V nur ein Ereignis, wenn Text in der Zwischenablage liegt → Taste selbst abfragen
        let v_now = ctx.input(|i| i.focused) && clip::paste_keys_down();
        let v_edge = v_now && !self.paste_down;
        self.paste_down = v_now;
        if k.paste || v_edge || (k.ctrl && !k.shift && has(Key::V)) {
            acts.push(Act::PasteInto(cwd));
        }
        if !k.alt && !k.ctrl && has(Key::Enter) {
            acts.push(Act::OpenSel);
        }
        // Leertaste: genau eine Datei ausgewählt → Schnellansicht in einem eigenen Tab
        if !k.alt && !k.ctrl && !k.shift && has(Key::Space) && !searching && paths.len() == 1 && fsx::is_file(&paths[0]) {
            acts.push(Act::Preview(paths[0].clone()));
        }
        if has(Key::Backspace) && !k.ctrl {
            acts.push(Act::Back);
        }
        if has(Key::Escape) {
            if searching {
                acts.push(Act::ClearSearch);
            } else if self.clip.as_ref().is_some_and(|c| c.cut) {
                acts.push(Act::ClearCut);
            } else {
                let pn = self.pane_mut();
                pn.sel.clear();
                pn.sel_dirty = true;
            }
        }
        // Pfeiltasten-Navigation (nur im Ordnermodus)
        let pn = self.pane_mut();
        if pn.search.is_none() && !pn.view.is_empty() && !k.alt {
            let n = pn.view.len() as isize;
            let cur = pn.cursor.map(|c| c as isize);
            // Schrittweite je Ansicht: Details = 1 Zeile; Raster = 1 Zeile = `cols` Elemente; Liste = Spalte = `rows_per_col`
            let (sv, sh) = match pn.mode {
                ViewMode::Details => (1isize, 0isize),
                ViewMode::List => (1, pn.rows_per_col.max(1) as isize),
                ViewMode::Grid | ViewMode::Large => (pn.cols.max(1) as isize, 1),
            };
            let target = if has(Key::ArrowDown) {
                Some(cur.map_or(0, |c| c + sv))
            } else if has(Key::ArrowUp) {
                Some(cur.map_or(0, |c| c - sv))
            } else if has(Key::ArrowRight) && sh > 0 {
                Some(cur.map_or(0, |c| c + sh))
            } else if has(Key::ArrowLeft) && sh > 0 {
                Some(cur.map_or(0, |c| c - sh))
            } else if has(Key::PageDown) {
                Some(cur.unwrap_or(0) + 8 * sv)
            } else if has(Key::PageUp) {
                Some(cur.unwrap_or(0) - 8 * sv)
            } else if has(Key::Home) {
                Some(0)
            } else if has(Key::End) {
                Some(n - 1)
            } else {
                None
            };
            if let Some(t) = target {
                let t = t.clamp(0, n - 1) as usize;
                pn.select_pos(t, k.shift, false);
                pn.ensure_visible(t);
            }
            // Tippen springt zum passenden Eintrag
            if !k.text.is_empty() && !k.ctrl && !k.alt {
                if pn
                    .typeahead
                    .1
                    .is_none_or(|t| t.elapsed() > Duration::from_millis(1000))
                {
                    pn.typeahead.0.clear();
                }
                pn.typeahead = (
                    format!("{}{}", pn.typeahead.0, k.text.to_lowercase()),
                    Some(Instant::now()),
                );
                let q = pn.typeahead.0.clone();
                if let Some(pos) = pn
                    .view
                    .iter()
                    .position(|&i| pn.items[i].name.to_lowercase().starts_with(&q))
                {
                    pn.select_pos(pos, false, false);
                    pn.ensure_visible(pos);
                }
            }
        }
    }

    /// Führt einen Befehl der Palette bzw. eines Tastenkürzels aus.
    fn run_cmd(&mut self, id: &str) {
        let cwd = self.pane().cwd.clone();
        let sel = pane_targets(self.pane());
        let target = if sel.len() == 1 { sel[0].clone() } else { cwd.clone() };
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let origin = self.ctx.input(|i| i.viewport().inner_rect).map_or(Pos2::ZERO, |r| r.min);
        let a = match id {
            "go.back" => Act::Back,
            "go.forward" => Act::Forward,
            "go.up" => Act::Up,
            "go.home" => Act::Go(home),
            "go.address" => Act::FocusAddr,
            "go.search" => Act::FocusSearch,
            "go.refresh" => Act::Refresh,
            "tab.new" => Act::TabNew,
            "tab.close" => Act::TabClose(self.cur),
            "view.start" => Act::StartOpen,
            "tab.reopen" => Act::TabReopen,
            "tab.next" => Act::TabNext(true),
            "tab.prev" => Act::TabNext(false),
            "tab.dup" => Act::TabDup(self.cur),
            "tab.detach" => Act::TabDetach(self.cur, origin + vec2(120.0, 90.0)),
            "tile.newtab" => {
                let a = self.tabs[self.cur].active;
                Act::SubNew(a, cwd)
            }
            "tile.next" | "tile.prev" => {
                let t = &self.tabs[self.cur];
                let p = &t.panes[t.active];
                let n = p.stack.len() + 1;
                if n < 2 {
                    return;
                }
                let k = if id == "tile.next" { (p.stack_pos + 1) % n } else { (p.stack_pos + n - 1) % n };
                Act::SubSwitch(t.active, k)
            }
            "tile.close" => {
                let t = &self.tabs[self.cur];
                Act::SubClose(t.active, t.panes[t.active].stack_pos)
            }
            "win.new" => Act::NewWindow(None),
            "win.split" => Act::ToggleDual,
            "win.pane" => Act::SwitchPane,
            "win.sidebar" => Act::ToggleSidebar,
            "file.copy" => Act::SetClip(sel, false),
            "file.cut" => Act::SetClip(sel, true),
            "file.paste" => Act::PasteInto(cwd),
            "file.newfolder" => Act::NewFolder,
            "file.newfile" => Act::NewFile,
            "file.rename" => Act::Rename,
            "file.trash" => Act::Trash(sel),
            "file.delete" => Act::DeletePerm(sel),
            "file.selectall" => Act::SelectAll,
            "file.selectrules" => Act::SelectRules,
            "file.invert" => {
                let pn = self.pane_mut();
                let all: HashSet<String> = pn.view.iter().map(|&i| pn.items[i].name.to_string()).collect();
                pn.sel = all.difference(&pn.sel).cloned().collect();
                pn.sel_dirty = true;
                return;
            }
            "file.undo" => Act::Undo,
            "file.redo" => Act::Redo,
            "file.copypath" => Act::CopyText(if sel.is_empty() { cwd.display().to_string() } else { sel.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n") }),
            "file.props" => Act::Props(target),
            "file.share" => Act::Share(sel),
            // Vorschau-Tab: die angezeigte Datei; sonst die Auswahl
            "file.print" => match &self.pane().extra {
                Some(PaneView::View(v)) => Act::Print(vec![v.path.clone()]),
                _ => Act::Print(sel),
            },
            "file.vscode" => Act::OpenIn("vscode".into(), target),
            "file.zed" => Act::OpenIn("zed".into(), target),
            "ai.assistant" => Act::EditorAi(false),
            "ai.agent" => Act::EditorAi(true),
            "ai.chat" => Act::EditorChat,
            "file.openwith" => match sel.first() {
                Some(p) if fsx::is_file(&p) => Act::OpenWith(p.clone()),
                _ => {
                    self.status = "„Öffnen mit“: bitte genau eine Datei auswählen".into();
                    return;
                }
            },
            "view.details" => Act::SetView(ViewMode::Details),
            "view.list" => Act::SetView(ViewMode::List),
            "view.grid" => Act::SetView(ViewMode::Grid),
            "view.large" => Act::SetView(ViewMode::Large),
            "view.preview" => {
                // Vorschau der aktiven Kachel; die Einstellung gilt als Vorgabe für neue Kacheln
                let on = !self.pane().preview_on;
                self.pane_mut().preview_on = on;
                let mut st = self.st.clone();
                st.show_preview = on;
                self.set_settings(st);
                return;
            }
            "view.hidden" => {
                let mut st = self.st.clone();
                st.show_hidden = !st.show_hidden;
                self.status = if st.show_hidden { "Versteckte Dateien werden angezeigt" } else { "Versteckte Dateien ausgeblendet" }.into();
                self.set_settings(st);
                return;
            }
            "tools.compare" => Act::Tools(ToolTab::Compare),
            "tools.dups" => Act::Tools(ToolTab::Dups),
            "tools.space" => Act::Tools(ToolTab::Space),
            "tools.large" => Act::Tools(ToolTab::Large),
            "term.new" => Act::Terminal(None, cwd),
            "git.view" => Act::GitView(cwd),
            "git.refresh" | "git.pull" | "git.push" | "git.fetch" => {
                // in der Git-Ansicht des aktuellen Tabs ausführen; sonst erst öffnen
                if !matches!(self.pane().extra, Some(PaneView::Git(_))) {
                    self.apply(Act::GitView(cwd));
                }
                let ctx = self.ctx.clone();
                if let Some(PaneView::Git(g)) = &mut self.pane_mut().extra {
                    match id {
                        "git.pull" => g.pull(&ctx),
                        "git.push" => g.push(&ctx),
                        "git.fetch" => g.fetch(&ctx),
                        _ => g.refresh(&ctx),
                    }
                }
                self.refresh_git();
                return;
            }
            "app.palette" => {
                self.palette = if self.palette.is_some() { None } else { Some(PaletteState { query: String::new(), sel: 0, first: true }) };
                return;
            }
            "app.settings" | "app.keys" => Act::ShowSettings,
            "app.clearrecent" => {
                self.recent.clear();
                self.status = "Verlauf gelöscht".into();
                return;
            }
            _ => return,
        };
        self.apply(a);
    }

    /// Neues Explorer-Fenster (eigener Prozess), optional auf einem bestimmten Monitor.
    fn new_window(&mut self, work: Option<[i32; 4]>) {
        let Ok(exe) = std::env::current_exe() else {
            self.status = "Neues Fenster konnte nicht gestartet werden".into();
            return;
        };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--open").arg(&self.pane().cwd);
        if let Some(w) = work {
            let r = editors::window_on(w);
            cmd.arg("--place").arg(format!("{},{},{},{}", r[0], r[1], r[2], r[3]));
        }
        if let Err(e) = cmd.spawn() {
            self.status = format!("Neues Fenster konnte nicht gestartet werden: {e}");
        }
    }

    fn open_terminal(&mut self, shell: Option<String>, dir: PathBuf) {
        let shells = self.shells.clone();
        let prog = shell
            .or_else(|| (!self.st.term_shell.is_empty()).then(|| self.st.term_shell.clone()))
            .or_else(|| shells.first().map(|s| s.1.clone()));
        let Some(prog) = prog else {
            self.status = "Keine Shell gefunden".into();
            return;
        };
        let name = shell_name(&shells, &prog);
        let dir = if dir.is_dir() { dir } else { dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")) };
        match terminal::Term::spawn(&name, &prog, &dir, &self.ctx) {
            Ok(t) => {
                self.new_tab(dir);
                self.pane_mut().extra = Some(PaneView::Term(t));
            }
            Err(e) => self.status = e,
        }
    }

    /// Wählbare KI-Backends: (ID, Name) – eingeschaltete Agenten, dann eigene Modelle.
    fn backends(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self.active_agents().into_iter().map(|a| (format!("agent:{}", a.0), a.1)).collect();
        v.extend(self.st.ai_models.iter().map(|p| (format!("model:{}", p.id), p.name.clone())));
        v
    }

    /// Backend für einen neuen Chat: gewünschtes bzw. Standard – falls nicht verfügbar das erste verfügbare.
    fn pick_backend(&self, want: Option<String>, agent: bool) -> String {
        let all = self.backends();
        let want = want.unwrap_or_else(|| if agent { self.st.ai_agent.clone() } else { self.st.ai_assist.clone() });
        if all.iter().any(|b| b.0 == want) {
            want
        } else {
            all.first().map(|b| b.0.clone()).unwrap_or(want)
        }
    }

    /// Modell der Explorer-KI: Standard aus den Einstellungen, sonst das erste eigene Modell.
    fn editor_backend(&self) -> String {
        for b in [&self.st.ai_agent, &self.st.ai_assist] {
            if let Some(id) = b.strip_prefix("model:") {
                if self.st.ai_models.iter().any(|p| p.id == id) {
                    return b.clone();
                }
            }
        }
        self.st.ai_models.first().map(|p| format!("model:{}", p.id)).unwrap_or_default()
    }

    /// Modell für Zusammenfassungen: das der Explorer-KI, sonst ein installiertes lokales Modell
    fn summary_ai_cfg(&self) -> Option<ai::Cfg> {
        if self.st.ai_enabled {
            let b = self.editor_backend();
            if let Some(p) = self.st.ai_models.iter().find(|p| Some(p.id.as_str()) == b.strip_prefix("model:")) {
                let key = self.ai_keys.get(&p.id).cloned().flatten();
                if !p.needs_key() || key.is_some() {
                    return Some(ai::Cfg::from_profile(p, key));
                }
            }
        }
        // kleinstes lokales Modell (schnell genug für ein paar Sätze)
        let (stem, _, _) = localllm::installed_text().into_iter().filter(|t| t.2 > 300_000_000).min_by_key(|t| t.2)?;
        let p = ai::ModelProfile { id: format!("local-{stem}"), name: stem.clone(), kind: ai::Kind::OpenAi, url: localllm::BASE.into(), model: stem };
        Some(ai::Cfg::from_profile(&p, None))
    }

    /// Modell für KI-Vorschläge im Terminal (das Modell der Explorer-KI), sonst `None`.
    fn term_ai_cfg(&self) -> Option<ai::Cfg> {
        if !self.st.ai_enabled || !self.st.term_complete {
            return None;
        }
        let b = self.editor_backend();
        let p = self.st.ai_models.iter().find(|p| Some(p.id.as_str()) == b.strip_prefix("model:"))?;
        let key = self.ai_keys.get(&p.id).cloned().flatten();
        (!p.needs_key() || key.is_some()).then(|| ai::Cfg::from_profile(p, key))
    }

    /// Für Entwicklung und Tests: Mit FOLDEX_SHOT=<datei.png> öffnet Foldex die Ansicht aus FOLDEX_SHOT_VIEW
    /// („settings:<Bereich>“ oder „share:<Datei>“), speichert ein Bildschirmfoto seines Fensters und beendet sich.
    fn dev_shot(&mut self, ctx: &egui::Context) {
        let Ok(out) = std::env::var("FOLDEX_SHOT") else { return };
        self.dev_frames += 1;
        if self.dev_frames == 1 {
            // Anzeigeskalierung nachstellen (z. B. 1.25 für 125 %)
            if let Some(z) = std::env::var("FOLDEX_SHOT_ZOOM").ok().and_then(|v| v.parse::<f32>().ok()) {
                ctx.set_zoom_factor(z);
            }
        }
        if self.dev_frames == 3 {
            let view = std::env::var("FOLDEX_SHOT_VIEW").unwrap_or_default();
            // FOLDEX_SHOT_DEMO=<Ordner>: Seitenleiste nur mit dessen Unterordnern (für Website-Bilder ohne
            // persönliche Einträge) – nur im Speicher, nichts wird gespeichert
            if let Ok(d) = std::env::var("FOLDEX_SHOT_DEMO") {
                let mut pins: Vec<sidebar::Entry> = std::fs::read_dir(&d)
                    .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).map(|e| sidebar::Entry { label: e.file_name().to_string_lossy().into_owned(), path: e.path() }).collect())
                    .unwrap_or_default();
                pins.sort_by(|a, b| a.label.cmp(&b.label));
                self.side.pins = pins;
                self.st.show_recent = false;
                self.side.hidden.extend(cloud::places().into_iter().map(|c| c.path));
            }
            // FOLDEX_SHOT_SORT=size: nach Größe absteigend (Test der Ordnergrößen)
            if std::env::var("FOLDEX_SHOT_SORT").is_ok_and(|v| v == "size") {
                self.apply(Act::SortKey(SortBy::Size));
                self.apply(Act::SortDir(false));
            }
            // FOLDEX_SHOT_VIEW=csearch:<Ordner>|<Anfrage>: Inhaltssuche
            if let Some(rest) = view.strip_prefix("csearch:") {
                let (d, q) = rest.split_once('|').unwrap_or((rest, ""));
                self.apply(Act::Go(PathBuf::from(d)));
                let s = (!q.is_empty()).then(|| contentsearch::Search::start(q, Path::new(d), ctx));
                self.csearch = Some((q.to_string(), PathBuf::from(d), s));
            }
            // FOLDEX_SHOT_VIEW=batch:<Bild>;<Bild>: Fenster „Bilder bearbeiten“
            if let Some(list) = view.strip_prefix("batch:") {
                let mut o = batch::Opts::new(list.split(';').map(PathBuf::from).collect());
                o.resize = batch::Resize::MaxSide(1920);
                o.format = "webp".into();
                o.remove_bg = true;
                self.batch = Some(o);
            }
            // FOLDEX_SHOT_VIEW=foldericon:<Ordner>: Fenster „Ordner-Icon ändern“
            if let Some(d) = view.strip_prefix("foldericon:") {
                self.folder_icon = Some((PathBuf::from(d), String::new()));
                // FOLDEX_SHOT_GROUP=material-symbole|…|farben
                if let Ok(g) = std::env::var("FOLDEX_SHOT_GROUP") {
                    if let Some(k) = matico::PRESET_GROUPS.iter().map(|x| x.0).chain(["farben"]).find(|k| *k == g) {
                        self.folder_icon_group = k;
                    }
                }
            }
            // FOLDEX_SHOT_VIEW=imgchat: Explorer-KI-Seitenleiste im Modus „Bild“
            if view == "imgchat" {
                let b = self.editor_backend();
                let mut c = aiui::ChatView::editor(self.pane().cwd.clone(), b, false);
                c.set_image(true);
                // FOLDEX_SHOT_STYLE=<n>: Stil-Vorlage gewählt; FOLDEX_SHOT_INPUT=<Text>: Eingabe
                c.dev_set(std::env::var("FOLDEX_SHOT_INPUT").ok(), std::env::var("FOLDEX_SHOT_STYLE").ok().and_then(|v| v.parse().ok()));
                // FOLDEX_SHOT_ATTACH=<Bild>: als Vorlage angehängt
                if let Ok(f) = std::env::var("FOLDEX_SHOT_ATTACH") {
                    c.attach(vec![PathBuf::from(f)]);
                }
                self.editor_ai = Some(c);
            }
            // FOLDEX_SHOT_VIEW=cloud: Fenster „Cloud-Speicher“; cloud:<Ordner>: dorthin wechseln
            // FOLDEX_SHOT_VIEW=start | startedit | ctrash: Startseite (mit Anpassen) bzw. Papierkorb von Google Drive
            if view == "start" || view == "startedit" {
                let pn = self.pane_mut();
                pn.start = true;
                pn.start_settings = view == "startedit";
            }
            // FOLDEX_SHOT_VIEW=print:<Datei>: Druckfenster
            if let Some(f) = view.strip_prefix("print:") {
                self.apply(Act::Print(vec![PathBuf::from(f)]));
            }
            if let Some(f) = view.strip_prefix("printsw:") {
                self.apply(Act::Print(vec![PathBuf::from(f)]));
                if let Some(w) = &mut self.printwin {
                    w.dev_gray();
                }
            }
            if view == "ctrash" {
                if let Some(a) = cloudlogin::accounts().into_iter().find(|a| a.key == "gdrive") {
                    self.apply(Act::CloudTrash("gdrive", Some(a.remote), a.name));
                }
            }
            if view == "cloud" {
                self.cloud_setup = true;
            } else if let Some(k) = view.strip_prefix("cloudform:") {
                // FOLDEX_SHOT_VIEW=cloudform:icloud: Anmeldeformular offen
                self.cloud_setup = true;
                if let Some(pr) = cloud::provider(k) {
                    if let Some(cloudlogin::Auth::Form(f)) = cloudlogin::auth(k) {
                        self.cloud_form = Some((pr.key, vec![String::new(); f.len()]));
                    }
                }
            } else if let Some(p) = view.strip_prefix("cloud:") {
                self.apply(Act::Go(PathBuf::from(p)));
            }
            // FOLDEX_SHOT_VIEW=tabs:<Pfad>|<Pfad>|…: je Pfad ein Tab (zum Messen des Tab-Wechsels)
            if let Some(list) = view.strip_prefix("tabs:") {
                for (k, p) in list.split('|').enumerate() {
                    // „view:<Datei>“: als Schnellansicht-Tab (großer Text, Video, PDF …)
                    if let Some(f) = p.strip_prefix("view:") {
                        self.open_view(PathBuf::from(f));
                        continue;
                    }
                    if k > 0 {
                        self.apply(Act::TabNew);
                    }
                    self.apply(Act::Go(PathBuf::from(p)));
                }
            }
            // FOLDEX_SHOT_VIEW=summary:<Pfad>: Zusammenfassung
            if let Some(p) = view.strip_prefix("summary:") {
                self.apply(Act::Summary(PathBuf::from(p)));
            }
            // FOLDEX_SHOT_VIEW=aichat:agent|assistant|chat: Explorer-KI-Seitenleiste im gewählten Modus
            if let Some(m) = view.strip_prefix("aichat:") {
                let b = self.editor_backend();
                let mut c = aiui::ChatView::editor(self.pane().cwd.clone(), b, m == "agent");
                c.set_mode(m == "agent", m == "chat");
                c.dev_set(std::env::var("FOLDEX_SHOT_INPUT").ok(), None);
                // FOLDEX_SHOT_ATTACH=<Datei>;<Datei>: angehängt
                if let Ok(f) = std::env::var("FOLDEX_SHOT_ATTACH") {
                    c.attach(f.split(';').map(PathBuf::from).collect());
                }
                self.editor_ai = Some(c);
            }
            if view == "imggen" {
                self.imggen = Some(localui::ImageGen::new(std::env::temp_dir()));
            }
            if let Some(page) = view.strip_prefix("settings:") {
                self.show_settings = true;
                self.settings_page = page.to_string();
                // FOLDEX_SHOT_MODELS=n: n Testprofile mit langen Namen/Adressen (nur im Speicher), das erste aufgeklappt
                if let Some(n) = std::env::var("FOLDEX_SHOT_MODELS").ok().and_then(|v| v.parse::<usize>().ok()) {
                    for k in 0..n {
                        let id = format!("shot{k}");
                        self.st.ai_models.push(ai::ModelProfile {
                            id: id.clone(),
                            name: format!("Sehr langer Modellname Nummer {k} für den Test der Darstellung im Einstellungsfenster"),
                            kind: if k % 2 == 0 { ai::Kind::OpenAi } else { ai::Kind::Anthropic },
                            url: format!("https://sehr-lange-adresse-{k}.beispiel.example.com/v1/chat/completions/mit/noch/mehr/pfad"),
                            model: if k % 3 == 0 { String::new() } else { "meta-llama/llama-3.3-70b-instruct-turbo-mit-langem-namen".into() },
                        });
                        if k == 0 {
                            self.model_open = Some(id);
                        }
                    }
                }
            } else if let Some(f) = view.strip_prefix("share:") {
                self.share = Some(share::ShareWin::new(vec![PathBuf::from(f)], ctx, self.hwnd));
            } else if let Some(rest) = view.strip_prefix("sharestate:") {
                // „sharestate:progress|C:\datei“, „…:result|…“, „…:error|…“
                let (state, f) = rest.split_once('|').unwrap_or(("result", rest));
                let mut w = share::ShareWin::new(vec![PathBuf::from(f)], ctx, self.hwnd);
                w.dev_state(state, ctx);
                self.share = Some(w);
            } else if let Some(f) = view.strip_prefix("localsend:") {
                let mut w = share::ShareWin::new(vec![PathBuf::from(f)], ctx, self.hwnd);
                w.open_localsend(ctx);
                self.share = Some(w);
            } else if let Some(f) = view.strip_prefix("preview:") {
                // Schnellansicht-Tab (Leertaste) für ein Bildschirmfoto
                self.open_view(PathBuf::from(f));
            } else if let Some(f) = view.strip_prefix("reveal:") {
                // Datei im Ordner auswählen und die Vorschau neben der Liste zeigen (FOLDEX_SHOT_PVFRAC = Anteil)
                self.apply(Act::Reveal(PathBuf::from(f)));
                let pn = self.pane_mut();
                pn.preview_on = true;
                if let Some(fr) = std::env::var("FOLDEX_SHOT_PVFRAC").ok().and_then(|v| v.parse::<f32>().ok()) {
                    pn.preview_frac = fr;
                }
            }
        }
        // FOLDEX_SHOT_CMDS="term.new;tab.close;…": ab Bild 10 alle 10 Bilder einen Befehl ausführen (wie per Tastenkürzel)
        if self.dev_frames >= 10 && self.dev_frames % 10 == 0 {
            if let Ok(cmds) = std::env::var("FOLDEX_SHOT_CMDS") {
                let k = (self.dev_frames / 10 - 1) as usize;
                if let Some(id) = cmds.split(';').nth(k).and_then(|c| keymap::cmd(c.trim())).map(|c| c.id) {
                    self.apply(Act::Cmd(id));
                }
            }
        }
        let mut at = std::env::var("FOLDEX_SHOT_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(120u32);
        // FOLDEX_SHOT_SECS=<n>: nach n Sekunden fotografieren (statt nach Bildern)
        if let Some(secs) = std::env::var("FOLDEX_SHOT_SECS").ok().and_then(|v| v.parse::<u64>().ok()) {
            static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let t0 = *T0.get_or_init(Instant::now);
            at = if t0.elapsed() >= Duration::from_secs(secs) { 0 } else { u32::MAX };
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        // Diagnose des Teilen-Menüs: Struktur als Text speichern statt Bildschirmfoto
        // erst fotografieren, wenn das Fenster eine Größe hat (minimiert = 0 × 0 → eframe stürzte ab)
        let has_size = ctx.input(|i| i.viewport().inner_rect.is_some_and(|r| r.width() >= 1.0 && r.height() >= 1.0) && i.viewport().minimized != Some(true));
        if self.dev_frames >= at && has_size && !self.dev_shot_sent {
            self.dev_shot_sent = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        let shot = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
        if let Some(img) = shot.filter(|_| self.dev_frames >= at) {
            let [w, h] = img.size;
            let px: Vec<u8> = img.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b(), 255]).collect();
            if let Some(im) = image::RgbaImage::from_raw(w as u32, h as u32, px) {
                let _ = im.save(&out);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }

    /// Zustand dieses Fensters für die Sitzung.
    fn win_state(&self) -> session::WinState {
        session::WinState {
            pos: self.session_geo.0,
            size: self.session_geo.1,
            maximized: self.session_geo.2,
            cur: self.cur,
            tabs: self.tabs.iter().map(tab_state).collect(),
            closed: 0,
        }
    }

    /// Alle 2 s: Fensterlage merken und die Sitzung speichern, wenn sich etwas geändert hat.
    fn session_tick(&mut self, ctx: &egui::Context) {
        let (geo, special, ppp) = ctx.input(|i| {
            let v = i.viewport();
            (
                (v.outer_rect.map(|r| [r.min.x, r.min.y]), v.inner_rect.map(|r| [r.width(), r.height()]), v.maximized.unwrap_or(false)),
                // Vollbild (Video) und minimiert sind keine Fensterlage, die man sich merken will
                v.fullscreen == Some(true) || v.minimized == Some(true),
                v.native_pixels_per_point.unwrap_or(1.0),
            )
        });
        // maximiert: die normale Lage behalten, damit das Fenster nach dem Wiederherstellen sinnvoll verkleinert
        if !special {
            if geo.2 {
                self.session_geo.2 = true;
            } else {
                self.session_geo = (geo.0.or(self.session_geo.0), geo.1.or(self.session_geo.1), false);
            }
        }
        // nur für Bildschirmfotos gestartet: weder Fensterlage noch Sitzung überschreiben
        if std::env::var_os("FOLDEX_SHOT").is_some() {
            return;
        }
        // Fensterlage sofort nach dem Verschieben/Vergrößern sichern (höchstens alle 0,5 s)
        if let (Some(pos), Some(size)) = (self.session_geo.0, self.session_geo.1) {
            let g = session::WinGeo { pos, size, maximized: self.session_geo.2, ppp };
            if self.geo_saved != Some(g) {
                if self.geo_at.elapsed() >= Duration::from_millis(500) {
                    session::save_geo(&g);
                    self.geo_saved = Some(g);
                    self.geo_at = Instant::now();
                } else {
                    ctx.request_repaint_after(Duration::from_millis(550));
                }
            }
        }
        if !self.st.restore_session || self.session_at.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.session_at = Instant::now();
        let s = self.win_state();
        if self.session_last.as_ref() != Some(&s) {
            session::save(&s);
            self.session_last = Some(s);
        }
        ctx.request_repaint_after(Duration::from_secs(3));
    }

    /// KI-Anbieter für Commit-Nachrichten: eigene Modelle (das der Explorer-KI zuerst), dann die Agenten.
    fn commit_ais(&self) -> Vec<gitui::CommitAi> {
        if !self.st.ai_enabled {
            return Vec::new();
        }
        let pref = self.editor_backend();
        let mut v = Vec::new();
        for p in &self.st.ai_models {
            let key = self.ai_keys.get(&p.id).cloned().flatten();
            if (p.needs_key() && key.is_none()) || p.model.trim().is_empty() {
                continue;
            }
            let item = gitui::CommitAi::Model { id: p.id.clone(), name: p.name.clone(), cfg: ai::Cfg::from_profile(p, key) };
            if pref == format!("model:{}", p.id) {
                v.insert(0, item);
            } else {
                v.push(item);
            }
        }
        v.extend(self.active_agents().into_iter().map(|(key, name, program)| gitui::CommitAi::Agent { key, name, program }));
        v
    }

    /// Offene Tabs, wie die Explorer-KI sie sieht.
    fn tab_infos(&self) -> Vec<ai::TabInfo> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let p = &t.panes[t.active.min(t.panes.len().saturating_sub(1))];
                let kind = match &p.extra {
                    Some(PaneView::Term(_)) => "terminal",
                    Some(PaneView::Git(_)) => "git",
                    Some(PaneView::Ai(_)) => "chat",
                    Some(PaneView::View(_)) => "preview",
                    None => "folder",
                };
                ai::TabInfo { title: t.title(), kind, folder: p.cwd.clone(), active: i == self.cur }
            })
            .collect()
    }

    /// Chat zeichnen (Tab oder Seitenleiste) – baut die Umgebung aus dem Explorer-Zustand.
    fn chat_ui(&self, a: &mut aiui::ChatView, ui: &mut egui::Ui) -> Vec<aiui::AiOut> {
        let th = self.th;
        let folder = if self.seen_cwd.as_os_str().is_empty() { a.cwd.clone() } else { self.seen_cwd.clone() };
        let (sel, agents, tabs) = (self.seen_sel.clone(), self.active_agents(), self.tab_infos());
        let env = aiui::Env {
            profiles: &self.st.ai_models,
            keys: &self.ai_keys,
            agents: &agents,
            agent_model: &self.st.agent_model,
            auto: &self.st.ai_auto,
            cli_full: self.st.ai_cli_full,
            cwd: &folder,
            selection: &sel,
            tabs: &tabs,
        };
        a.ui(ui, &th, &env)
    }

    /// Ergebnisse eines Chats umsetzen.
    fn handle_ai_outs(&mut self, outs: Vec<aiui::AiOut>, acts: &mut Vec<Act>) {
        for o in outs {
            match o {
                aiui::AiOut::Done(op) => {
                    self.history.apply(Hist::Record(op));
                    let c = self.seen_cwd.clone();
                    self.dsize.invalidate(&c);
                    self.refresh_all();
                    self.refresh_git();
                }
                aiui::AiOut::Effect(ai::Effect::Go(p)) => self.ai_go = Some(p),
                aiui::AiOut::Effect(ai::Effect::Select(names)) => self.ai_select = Some(names),
                aiui::AiOut::Effect(ai::Effect::OpenFile(p)) => acts.push(Act::OpenFile(p)),
                aiui::AiOut::Effect(e) => self.ai_fx.push(e),
                aiui::AiOut::OpenSettings => acts.push(Act::ShowSettings),
                aiui::AiOut::KeySaved(id) => self.reload_key(&id),
                aiui::AiOut::Changed => {
                    let c = self.seen_cwd.clone();
                    self.dsize.invalidate(&c);
                    self.refresh_all();
                    self.refresh_git();
                }
                aiui::AiOut::SetAgentModel(k, m) => {
                    let mut st = self.st.clone();
                    st.agent_model.insert(k, m);
                    self.set_settings(st);
                }
                aiui::AiOut::SetProfileModel(id, m) => {
                    let mut st = self.st.clone();
                    if let Some(pr) = st.ai_models.iter_mut().find(|x| x.id == id) {
                        pr.model = m;
                    }
                    self.set_settings(st);
                }
                aiui::AiOut::Close => self.editor_ai = None,
                aiui::AiOut::Preview(p) => self.open_view(p),
                aiui::AiOut::Edit(p) => {
                    // Vorschau-Tab öffnen und dort gleich den Bildeditor
                    self.open_view(p.clone());
                    for t in &mut self.tabs {
                        for pn in &mut t.panes {
                            if let Some(PaneView::View(v)) = &mut pn.extra {
                                if v.path == p {
                                    v.pv.want_editor = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Taste drücken und im nächsten Bild loslassen.
    fn inject_key(&mut self, key: Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            self.inject.push_back(vec![egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }]);
        }
    }

    /// Mausaktion der Explorer-KI: Ereignisse für egui und echten Mauszeiger an dieselbe Stelle setzen.
    fn ai_mouse(&mut self, action: &str, pos: Pos2, amount: f32) {
        use egui::{Event, PointerButton};
        let none = egui::Modifiers::NONE;
        self.inject.push_back(vec![Event::PointerMoved(pos)]);
        let click = |b: PointerButton| [true, false].map(|pressed| vec![Event::PointerButton { pos, button: b, pressed, modifiers: none }]);
        match action {
            "click" => self.inject.extend(click(PointerButton::Primary)),
            "right_click" => self.inject.extend(click(PointerButton::Secondary)),
            "double_click" => {
                self.inject.extend(click(PointerButton::Primary));
                self.inject.extend(click(PointerButton::Primary));
            }
            "scroll" => self.inject.push_back(vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, -amount), modifiers: none }]),
            _ => {}
        }
        self.ai_cursor = Some((pos, std::time::Instant::now()));
        #[cfg(windows)]
        {
            let (inner, ppp) = self.ctx.input(|i| (i.viewport().inner_rect, i.viewport().native_pixels_per_point.unwrap_or(1.0)));
            if let Some(r) = inner {
                let p = (r.min + pos.to_vec2()) * ppp;
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos(p.x as i32, p.y as i32) };
            }
        }
        self.ctx.request_repaint();
    }

    /// Bildschirmfoto für die Explorer-KI: auf höchstens 1280 Punkte Breite verkleinern, als JPEG senden.
    fn take_screenshots(&mut self, ctx: &egui::Context) {
        let shot = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
        let (Some(img), Some(id)) = (shot, self.shot_wait.clone()) else { return };
        self.shot_wait = None;
        let ppp = ctx.pixels_per_point();
        let [w, h] = img.size;
        let pts_w = w as f32 / ppp;
        let tw = pts_w.min(1280.0).max(1.0);
        let th = (h as f32 * tw / w as f32).max(1.0);
        self.shot_scale = pts_w / tw;
        std::thread::spawn(move || {
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b(), 255]).collect();
            let reply = image::RgbaImage::from_raw(w as u32, h as u32, rgba)
                .map(|im| image::imageops::resize(&im, tw as u32, th as u32, image::imageops::FilterType::Triangle))
                .and_then(|im| {
                    let rgb = image::DynamicImage::ImageRgba8(im).to_rgb8();
                    let mut buf = Vec::new();
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80).encode_image(&rgb).ok()?;
                    Some(buf)
                });
            match reply {
                Some(b) => ai::deliver(&id, (format!("Bildschirmfoto des Foldex-Fensters, {} × {} Pixel.", tw as u32, th as u32), false, Some(convert::base64(&b)))),
                None => ai::deliver(&id, ("Bildschirmfoto konnte nicht erstellt werden".into(), true, None)),
            }
        });
    }

    /// Zeiger der KI kurz sichtbar machen.
    fn ui_ai_cursor(&mut self, ctx: &egui::Context) {
        let Some((p, t0)) = self.ai_cursor else { return };
        let age = t0.elapsed().as_secs_f32();
        if age > 2.0 {
            self.ai_cursor = None;
            return;
        }
        let a = (1.0 - (age - 1.4).max(0.0) / 0.6).clamp(0.0, 1.0);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("ai_cursor")));
        let col = self.th.accent.gamma_multiply(a);
        painter.circle_stroke(p, 10.0 + age * 6.0, Stroke::new(2.0, col.gamma_multiply(0.6)));
        let pts = vec![p, p + vec2(0.0, 18.0), p + vec2(5.0, 13.5), p + vec2(12.5, 13.0)];
        painter.add(egui::Shape::convex_polygon(pts, col, Stroke::new(1.2, Color32::WHITE.gamma_multiply(a))));
        painter.text(p + vec2(14.0, 16.0), Align2::LEFT_TOP, "✦ KI", FontId::proportional(11.5), col);
        ctx.request_repaint();
    }

    /// Explorer-KI als Seitenleiste rechts.
    fn ui_editor_ai(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        if !self.st.ai_enabled {
            self.editor_ai = None;
        }
        let Some(mut a) = self.editor_ai.take() else { return };
        let th = self.th;
        let mut outs = Vec::new();
        egui::SidePanel::right("editor_ai")
            .frame(egui::Frame::none().fill(th.bg))
            .resizable(true)
            .default_width(420.0)
            .width_range(340.0..=820.0)
            .show(ctx, |ui| {
                ui.painter().line_segment([ui.max_rect().left_top(), ui.max_rect().left_bottom()], Stroke::new(1.0, th.line));
                let panel = ui.max_rect();
                outs = self.chat_ui(&mut a, ui);
                // Dateien hierher ziehen = anhängen; beim Ziehen deutlich markieren
                self.zones.push((panel, Dest::Ai));
                let hovering_os = ctx.input(|i| !i.raw.hovered_files.is_empty());
                if (self.drag.is_some() || hovering_os) && ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| panel.contains(p)) {
                    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("ai_drop")));
                    p.rect(panel.shrink(6.0), 10.0, th.accent.gamma_multiply(0.12), Stroke::new(2.0, th.accent));
                    p.text(panel.center(), Align2::CENTER_CENTER, if a.image { "Bild als Vorlage ablegen" } else { "Dateien anhängen" }, FontId::proportional(16.0), th.accent);
                }
            });
        self.editor_ai = Some(a);
        self.handle_ai_outs(outs, acts);
    }

    fn reload_key(&mut self, id: &str) {
        if let Some(p) = self.st.ai_models.iter().find(|p| p.id == id) {
            self.ai_keys.insert(id.to_string(), ai::key_for(p));
        }
    }

    /// Ergebnisse der Hintergrund-Prüfungen (Agenten, Verbindungstests) übernehmen.
    fn poll_ai(&mut self) {
        if let Some(Ok((found, versions))) = self.agent_scan.as_ref().map(|r| r.try_recv()) {
            self.agents = found;
            self.agent_versions = versions;
            self.agent_scan = None;
        }
        if let Some(Ok((id, r, list))) = self.model_test_rx.as_ref().map(|r| r.try_recv()) {
            if !list.is_empty() {
                self.model_list.insert(id.clone(), list);
            }
            self.model_test.insert(id, r);
            self.model_test_rx = None;
        }
    }

    /// Eingeschaltete und installierte KI-Agenten (Kommandozeile).
    fn active_agents(&self) -> Vec<(String, String, String)> {
        if !self.st.ai_enabled {
            return Vec::new();
        }
        self.agents.iter().filter(|a| self.st.agent_on.get(&a.0).copied().unwrap_or(true)).cloned().collect()
    }

    /// Vom KI-Assistenten angefordert: Ordner anzeigen bzw. Elemente auswählen – in einer Ordner-Kachel
    /// des aktuellen Tabs, sonst in einem neuen Tab.
    fn apply_ai_effects(&mut self) {
        for e in std::mem::take(&mut self.ai_fx) {
            match e {
                ai::Effect::OpenTab(kind, p) => match kind.as_str() {
                    "terminal" => self.apply(Act::Terminal(None, p)),
                    "git" => self.apply(Act::GitView(p)),
                    _ => self.new_tab(p),
                },
                ai::Effect::SwitchTab(i) if i < self.tabs.len() => self.cur = i,
                ai::Effect::CloseTab(i) if i < self.tabs.len() => self.close_tab(i),
                ai::Effect::Command(id) => self.run_cmd(&id),
                ai::Effect::Screenshot(id) => {
                    self.shot_wait = Some(id);
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                }
                ai::Effect::Mouse(action, x, y, amount) => self.ai_mouse(&action, pos2(x, y) * self.shot_scale, amount),
                ai::Effect::TypeText(t) => {
                    // Zeilenumbrüche als Enter
                    for (i, line) in t.split('\n').enumerate() {
                        if i > 0 {
                            self.inject_key(Key::Enter, egui::Modifiers::NONE);
                        }
                        if !line.is_empty() {
                            self.inject.push_back(vec![egui::Event::Text(line.to_string())]);
                        }
                    }
                }
                ai::Effect::Key(spec) => {
                    if let Some((ctrl, shift, alt, k)) = ai::parse_key(&spec) {
                        let m = egui::Modifiers { ctrl, shift, alt, command: ctrl, mac_cmd: false };
                        match (ctrl && !shift && !alt, k.as_str()) {
                            (true, "C") => self.inject.push_back(vec![egui::Event::Copy]),
                            (true, "X") => self.inject.push_back(vec![egui::Event::Cut]),
                            _ => {
                                if let Some(key) = Key::from_name(&k) {
                                    self.inject_key(key, m);
                                }
                            }
                        }
                    }
                }
                ai::Effect::Shell(dir, cmd) => {
                    self.open_terminal(None, dir);
                    self.ai_term = Some(self.tabs[self.cur].id);
                    if let Some(PaneView::Term(t)) = &mut self.pane_mut().extra {
                        t.type_line(&cmd);
                    }
                }
                ai::Effect::ReadTerminal(id, idx) => {
                    let term_of = |t: &Tab| t.panes.iter().find_map(|p| if let Some(PaneView::Term(x)) = &p.extra { Some(x.screen_text()) } else { None });
                    let text = match idx {
                        Some(i) => self.tabs.get(i).and_then(term_of),
                        None => self.ai_term.and_then(|tid| self.tabs.iter().find(|t| t.id == tid)).and_then(term_of).or_else(|| self.tabs.iter().find_map(term_of)),
                    };
                    let reply = match text {
                        Some(t) => (t.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_end().to_string(), false, None),
                        None => ("Kein Terminal-Tab gefunden".to_string(), true, None),
                    };
                    ai::deliver(&id, reply);
                }
                _ => {}
            }
        }
        if let Some(p) = self.ai_go.take() {
            let t = &mut self.tabs[self.cur];
            match t.root.leaves().into_iter().find(|&i| t.panes[i].extra.is_none()) {
                Some(i) => {
                    t.active = i;
                    let (ctx, st) = (self.ctx.clone(), self.st.clone());
                    t.panes[i].go(p, true, &ctx, &st);
                }
                None => self.new_tab(p),
            }
        }
        if let Some(names) = self.ai_select.take() {
            let target = self.seen_cwd.clone();
            for t in &mut self.tabs {
                for pn in &mut t.panes {
                    if pn.extra.is_none() && pn.cwd == target {
                        pn.sel = names.iter().cloned().collect();
                        pn.sel_dirty = true;
                    }
                }
            }
        }
    }

    /// Git-Status des aktuellen Ordners im Hintergrund neu bestimmen.
    fn refresh_git(&mut self) {
        if !self.st.git_marks || !self.git_ok {
            self.git_st = None;
            return;
        }
        let cwd = self.seen_cwd.clone();
        if cwd.as_os_str().is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let st = git::find_root(&cwd).and_then(|r| git::status(&r).ok());
            let _ = tx.send((cwd, st));
            ctx.request_repaint();
        });
        self.git_st_rx = Some(rx);
    }

    /// Sortieren nach Größe: Ordner zählen mit ihrer (im Hintergrund berechneten) Gesamtgröße. Alle
    /// Unterordner der sichtbaren Kacheln werden angefragt; neue Werte sortieren höchstens alle 0,4 s neu.
    fn size_sort(&mut self) {
        if !self.st.folder_sizes {
            return;
        }
        let ver = self.dsize.ver;
        let st = self.st.clone();
        let (dsize, tabs, cur) = (&mut self.dsize, &mut self.tabs, self.cur);
        let Some(t) = tabs.get_mut(cur) else { return };
        for pn in &mut t.panes {
            if pn.sort != pane::SortBy::Size || pn.search.is_some() || pn.trash || pn.extra.is_some() {
                pn.size_sorted = None;
                continue;
            }
            let key = (pn.cwd.clone(), pn.items.len(), ver);
            if pn.size_sorted.as_ref().is_some_and(|(k, at)| (k.0 == key.0 && k.1 == key.1 && k.2 == key.2) || (k.0 == key.0 && k.1 == key.1 && at.elapsed() < Duration::from_millis(400))) {
                continue;
            }
            let mut changed = false;
            for it in pn.items.iter_mut().filter(|i| i.dir) {
                let b = dsize.get(&pn.cwd.join(&*it.name)).map_or(0, |s| s.bytes);
                if it.size != b {
                    it.size = b;
                    changed = true;
                }
            }
            if changed {
                pn.resort(&st);
            }
            pn.size_sorted = Some((key, Instant::now()));
        }
    }

    /// Jedes Frame: Ordnerwechsel erkennen (Verlauf, Größen, Git) und Zwischenablage beobachten.
    fn track(&mut self) {
        self.dsize.poll();
        self.size_sort();
        if let Some(Ok((dir, st))) = self.git_st_rx.as_ref().map(|r| r.try_recv()) {
            self.git_st_rx = None;
            if dir == self.seen_cwd {
                self.git_st = st;
            }
        }
        if self.pane().extra.is_none() {
            if self.pane().sel_dirty || self.pane().sel.len() != self.seen_sel.len() {
                self.seen_sel = self.pane().sel.iter().cloned().collect();
            }
            let cwd = self.pane().cwd.clone();
            if cwd != self.seen_cwd && !self.pane().trash {
                self.seen_cwd = cwd.clone();
                self.dsize.new_generation();
                if self.st.show_recent && cwd.is_absolute() {
                    self.recent.push(&cwd);
                }
                if self.git_st.as_ref().is_some_and(|g| !cwd.starts_with(&g.root)) {
                    self.git_st = None;
                }
                self.refresh_git();
            }
        }
        // Ausgeschnittene Elemente: hat ein anderes Programm die Zwischenablage überschrieben, Markierung aufheben
        if let Some(c) = &self.clip {
            if c.cut && c.seq != 0 && clip::seq() != c.seq {
                self.clip = None;
            }
        }
    }

    // ── Tabs ──
    fn at(&self, secs: f32) -> f32 {
        if self.st.animations {
            secs
        } else {
            0.0
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_tab(&mut self, ui: &mut egui::Ui, r: Rect, title: &str, selected: bool, hover_t: f32, target: bool, lifted: bool, _panes: usize, ic: Option<Ic>) {
        let th = self.th;
        let font = FontId::proportional(13.5);
        let top = Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 };
        if lifted {
            // schwebend: Schatten + Rahmen
            ui.painter().rect_filled(r.translate(vec2(0.0, 3.0)).expand(1.0), 6.0, Color32::from_black_alpha(60));
            ui.painter().rect(r, 6.0, th.bg, Stroke::new(1.0, th.accent));
        } else if selected {
            ui.painter().rect_filled(r, top, th.bg);
        } else if hover_t > 0.0 || target {
            let c = if target { th.sel } else { th.hover.gamma_multiply(hover_t) };
            ui.painter().rect_filled(r.shrink2(vec2(0.0, 3.0)), 6.0, c);
        }
        let ir = Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 9.0), vec2(18.0, 18.0));
        match ic {
            Some(ic) => ui::draw(ui.painter(), ir.shrink(1.0), ic, if selected || lifted { th.accent } else { th.sub }),
            None => self.icons.paint(ui.painter(), ir, IconKind::Dir, "", "", false, PathBuf::new),
        }
        // lange Titel mit „…“ kürzen statt hart abschneiden
        text_trunc(
            ui.painter(),
            pos2(ir.right() + 8.0, r.center().y),
            title,
            font,
            if selected || lifted { th.text } else { th.sub },
            r.right() - 26.0 - (ir.right() + 8.0),
        );
    }

    // ───────────── Kacheln, Ziehen, Andocken, Fensterwechsel ─────────────
    fn zone_at(&self, pos: Pos2) -> Option<(usize, Zone)> {
        let (l, r) = self.leaf_rects.iter().find(|(_, r)| r.contains(pos))?;
        compass(*r)
            .into_iter()
            .find(|(_, b)| b.expand(4.0).contains(pos))
            .map(|(z, _)| z)
            .or_else(|| Zone::at(pos, *r))
            .map(|z| (*l, z))
    }

    fn tab_payload(&self, i: usize) -> ipc::Payload {
        let t = &self.tabs[i];
        ipc::Payload {
            title: t.title(),
            layout: t.root.serialize(),
            paths: t.panes.iter().map(pane_spec).collect(),
        }
    }

    /// Entfernt Pane `pi` samt Kachel aus Tab `ti` (mind. eine Pane bleibt).
    fn take_pane(&mut self, ti: usize, pi: usize) -> Option<Pane> {
        let t = self.tabs.get_mut(ti)?;
        if t.panes.len() < 2 || pi >= t.panes.len() || !t.root.remove_leaf(pi) {
            return None;
        }
        t.root.shift_after(pi);
        let pane = t.panes.remove(pi);
        if t.active > pi {
            t.active -= 1;
        } else if t.active == pi {
            t.active = t.root.leaves().first().copied().unwrap_or(0);
        }
        t.active = t.active.min(t.panes.len() - 1);
        Some(pane)
    }

    fn close_pane(&mut self, pi: usize) {
        let cur = self.cur;
        self.take_pane(cur, pi);
    }

    /// Teilfenster in einen eigenen Tab lösen.
    fn pane_to_tab(&mut self, pi: usize) {
        let cur = self.cur;
        if let Some(pane) = self.take_pane(cur, pi) {
            let id = self.next_tab;
            self.next_tab += 1;
            self.tabs.push(Tab::new(id, pane));
            self.cur = self.tabs.len() - 1;
            self.status = "Teilfenster in eigenen Tab gelöst".into();
        }
    }

    /// Ganzen Tab `src` an Kachel `leaf` des aktuellen Tabs andocken (behält alle seine Teilfenster).
    fn dock_tab(&mut self, src: usize, leaf: usize, z: Zone) {
        if src == self.cur || src >= self.tabs.len() || !self.tabs[self.cur].root.contains(leaf) {
            return;
        }
        let Tab { panes, mut root, .. } = self.tabs.remove(src);
        let cur = if src < self.cur { self.cur - 1 } else { self.cur };
        let t = &mut self.tabs[cur];
        let k = t.panes.len();
        root.offset(k);
        t.panes.extend(panes);
        t.root.split_leaf(leaf, root, z);
        t.active = k;
        self.cur = cur;
        self.status = format!("Angedockt: {}", z.label());
    }

    /// Teilfenster `pi` an eine andere Kachel desselben Tabs verschieben.
    fn dock_pane(&mut self, pi: usize, leaf: usize, z: Zone) {
        let t = &mut self.tabs[self.cur];
        if pi == leaf || !t.root.contains(pi) || !t.root.contains(leaf) {
            return;
        }
        if t.root.remove_leaf(pi) {
            t.root.split_leaf(leaf, Node::Leaf(pi), z);
            t.active = pi;
        }
    }

    /// Kachel, in die beim Loslassen als Tab aufgenommen wird: über ihrem Kopf oder der Kompass-Mitte.
    fn merge_target(&self, pos: Pos2) -> Option<usize> {
        if let Some((leaf, _)) = self.head_rects.iter().find(|(_, r)| r.contains(pos)) {
            return Some(*leaf);
        }
        self.leaf_rects.iter().find(|(_, r)| r.contains(pos) && compass_center(*r).expand(4.0).contains(pos)).map(|(l, _)| *l)
    }

    /// Gezogenes Element als Tab in Kachel `leaf` des aktuellen Tabs aufnehmen.
    fn merge_into(&mut self, kind: DragKind, leaf: usize) {
        let cur = self.cur;
        let pane = match kind {
            DragKind::Tab(id) => {
                let Some(src) = self.tabs.iter().position(|t| t.id == id) else { return };
                if src == cur {
                    return;
                }
                let tab = self.tabs.remove(src);
                if src < self.cur {
                    self.cur -= 1;
                }
                // alle Kacheln des Tabs werden Tabs der Zielkachel
                let mut all: Vec<Pane> = Vec::new();
                for mut p in tab.panes {
                    all.extend(sub_flatten(&mut p));
                }
                sub_group(all, 0)
            }
            DragKind::Pane(_, pi) => {
                if pi == leaf {
                    return;
                }
                let Some(p) = self.take_pane(cur, pi) else { return };
                // Kachel-Nummern hinter der entfernten rücken nach vorn
                let leaf = if leaf > pi { leaf - 1 } else { leaf };
                let t = &mut self.tabs[cur];
                if let Some(slot) = t.panes.get_mut(leaf) {
                    sub_add(slot, p);
                    t.active = leaf;
                }
                self.status = "Als Tab in die Kachel aufgenommen".into();
                return;
            }
            DragKind::Sub(_, pi, k) => {
                if pi == leaf {
                    return;
                }
                match self.tabs[cur].panes.get_mut(pi).and_then(|slot| sub_take(slot, k)) {
                    Some(p) => p,
                    None => return,
                }
            }
        };
        let t = &mut self.tabs[self.cur];
        if let Some(slot) = t.panes.get_mut(leaf) {
            sub_add(slot, pane);
            t.active = leaf;
            self.status = "Als Tab in die Kachel aufgenommen".into();
        }
    }

    /// Inhalt eines anderen Fensters hier ablegen: an eine Kachel andocken oder als neuer Tab.
    fn dock_payload(&mut self, p: ipc::Payload, local: Pos2) {
        let panes: Vec<Pane> = p.paths.iter().map(|x| pane_from_spec(x, &self.ctx, &self.st, &self.shells)).collect();
        if panes.is_empty() {
            return;
        }
        if !self.tabbar_rect.contains(local) {
            if let Some(leaf) = self.merge_target(local) {
                let mut all: Vec<Pane> = Vec::new();
                for mut p in panes {
                    all.extend(sub_flatten(&mut p));
                }
                let t = &mut self.tabs[self.cur];
                if let Some(slot) = t.panes.get_mut(leaf) {
                    sub_add(slot, sub_group(all, 0));
                    t.active = leaf;
                }
                self.status = "Von anderem Fenster als Tab in die Kachel aufgenommen".into();
                return;
            }
        }
        let mut root = Node::parse(&p.layout)
            .filter(|n| n.valid_for(panes.len()))
            .unwrap_or_else(|| Node::chain(panes.len(), Dir::H));
        let target = if self.tabbar_rect.contains(local) { None } else { self.zone_at(local) };
        match target {
            Some((leaf, z)) => {
                let t = &mut self.tabs[self.cur];
                let k = t.panes.len();
                root.offset(k);
                t.panes.extend(panes);
                t.root.split_leaf(leaf, root, z);
                t.active = k;
                self.status = format!("Von anderem Fenster angedockt: {}", z.label());
            }
            None => {
                let id = self.next_tab;
                self.next_tab += 1;
                self.tabs.push(Tab { id, panes, active: 0, root });
                self.cur = self.tabs.len() - 1;
                self.status = "Von anderem Fenster als neuer Tab eingefügt".into();
            }
        }
    }

    /// Quelle nach erfolgreichem Verschieben in ein anderes Fenster entfernen.
    fn close_source(&mut self, kind: DragKind) {
        match kind {
            DragKind::Tab(id) => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                    if self.tabs.len() > 1 {
                        self.close_tab(i);
                    } else {
                        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close); // leeres Fenster schließen
                    }
                }
            }
            DragKind::Pane(tid, pi) => {
                if let Some(ti) = self.tabs.iter().position(|t| t.id == tid) {
                    self.take_pane(ti, pi);
                }
            }
            DragKind::Sub(tid, pi, k) => {
                if let Some(slot) = self.tabs.iter_mut().find(|t| t.id == tid).and_then(|t| t.panes.get_mut(pi)) {
                    sub_take(slot, k);
                }
            }
        }
    }

    /// Neues Foldex-Fenster mit diesem Inhalt an der Bildschirmposition starten.
    /// Tab bzw. Teilfenster in ein neues Fenster lösen. `at` = Bildschirmpunkt in physischen Pixeln:
    /// das Fenster erscheint auf genau dem Monitor, über dem losgelassen wurde, in dessen Skalierung.
    fn detach_payload(&mut self, kind: DragKind, payload: ipc::Payload, at: Pos2) {
        let only_one = self.tabs.len() == 1 && self.tabs[0].panes.len() == 1 && self.tabs[0].panes[0].stack.is_empty();
        if only_one {
            self.status = "Das ist das einzige Fenster – es kann nicht herausgelöst werden".into();
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            self.status = "Neues Fenster konnte nicht gestartet werden".into();
            return;
        };
        let mut cmd = std::process::Command::new(exe);
        match editors::window_at_point([at.x as i32, at.y as i32], [120.0, 18.0]) {
            Some(r) => {
                cmd.arg("--place").arg(format!("{},{},{},{}", r[0], r[1], r[2], r[3]));
            }
            None => {
                let ppp = self.ctx.pixels_per_point();
                cmd.arg("--pos").arg(format!("{:.0},{:.0}", (at.x / ppp - 120.0).max(0.0), (at.y / ppp - 18.0).max(0.0)));
            }
        }
        cmd.arg("--layout").arg(&payload.layout);
        for p in &payload.paths {
            cmd.arg("--open").arg(p);
        }
        match cmd.spawn() {
            Ok(_) => self.close_source(kind),
            Err(e) => self.status = format!("Neues Fenster konnte nicht gestartet werden: {e}"),
        }
    }

    /// Kontextmenü „In neuem Fenster öffnen“.
    fn detach_tab(&mut self, i: usize, at: Pos2) {
        if i >= self.tabs.len() {
            return;
        }
        let (payload, id) = (self.tab_payload(i), self.tabs[i].id);
        self.detach_payload(DragKind::Tab(id), payload, at);
    }

    /// `sp` = Ablagepunkt in physischen Pixeln (das Zielfenster rechnet in seine Skalierung um).
    fn handoff_to(&mut self, pid: u32, sp: Pos2, kind: DragKind, payload: &ipc::Payload) {
        let seq = ipc::send_drop(pid, [sp.x, sp.y], payload);
        self.handoff = Some(Handoff { seq, kind, deadline: Instant::now() + Duration::from_millis(2500) });
        self.status = "Wird an das andere Fenster übergeben …".into();
    }

    /// Nachrichten anderer Fenster verarbeiten; eigenen Fensterbereich veröffentlichen.
    fn ipc_poll(&mut self, ctx: &egui::Context) {
        let inner = ctx.input(|i| i.viewport().inner_rect);
        let ppp = ctx.pixels_per_point();
        if let Some(r) = inner {
            *self.win_rect.lock().unwrap() = Some([r.min.x * ppp, r.min.y * ppp, r.width() * ppp, r.height() * ppp]);
        }
        while let Ok(ev) = self.ipc_rx.try_recv() {
            match ev {
                ipc::Event::Drag(info) => self.foreign = info,
                ipc::Event::Drop(msg) => {
                    let origin = inner.map_or(Pos2::ZERO, |r| r.min);
                    let local = pos2(msg.pos[0] / ppp, msg.pos[1] / ppp) - origin.to_vec2();
                    self.foreign = None;
                    self.dock_payload(msg.payload, local);
                    ipc::send_ack(msg.from, msg.seq);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                ipc::Event::Ack(seq) => {
                    if self.handoff.as_ref().is_some_and(|h| h.seq == seq) {
                        if let Some(h) = self.handoff.take() {
                            self.close_source(h.kind);
                            self.status = "In das andere Fenster verschoben".into();
                        }
                    }
                }
            }
        }
        if self.handoff.as_ref().is_some_and(|h| Instant::now() > h.deadline) {
            self.handoff = None;
            self.status = "Das andere Fenster hat nicht geantwortet – nichts wurde verschoben".into();
        }
    }

    /// Mausposition auf dem gesamten Bildschirm in physischen Pixeln (Windows/macOS), damit ein Tab auch
    /// außerhalb des Fensters verfolgt wird. Rückgabe: (Position, linke Taste gedrückt).
    fn global_cursor(&self) -> Option<(Pos2, bool)> {
        // Windows: Cursorposition direkt abfragen; die Taste aus dem Zustand dieses Threads (beim Ziehen hält
        // das Fenster die Maus fest und bekommt auch das Loslassen außerhalb mit)
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::POINT;
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_LBUTTON};
            use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
            let mut pt = POINT { x: 0, y: 0 };
            if unsafe { GetCursorPos(&mut pt) } == 0 {
                return None;
            }
            let down = unsafe { GetKeyState(VK_LBUTTON as i32) } < 0;
            return Some((pos2(pt.x as f32, pt.y as f32), down));
        }
        #[cfg(target_os = "macos")]
        {
            use device_query::DeviceQuery;
            let m = self.device.as_ref()?.get_mouse();
            return Some((pos2(m.coords.0 as f32, m.coords.1 as f32), m.button_pressed.get(1).copied().unwrap_or(false)));
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        None
    }

    /// Einmal pro Frame nach dem Zeichnen: Ziehen fortschreiben, Vorschau zeigen, beim Loslassen ablegen.
    fn drive_drag(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.tab_drag.take() else { return };
        let ppp = ctx.pixels_per_point();
        let (ptr, down, inner) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.viewport().inner_rect));
        let origin = inner.map_or(Pos2::ZERO, |r| r.min);
        let global = self.global_cursor();
        // physisch (für andere Fenster/Monitore) und in Punkten dieses Fensters (für die eigene Fläche)
        let px = global.map(|g| g.0).or_else(|| ptr.map(|p| pos2((origin.x + p.x) * ppp, (origin.y + p.y) * ppp)));
        let screen = px.map(|p| pos2(p.x / ppp, p.y / ppp));
        d.outside = match (screen, inner) {
            (Some(g), Some(r)) if global.is_some() => !r.contains(g),
            _ => ptr.map_or(true, |p| !ctx.screen_rect().contains(p)),
        };
        d.px = px.or(d.px);
        d.screen = screen.or(d.screen);
        if let Some(sp) = d.screen {
            if d.outside || ptr.is_none() {
                d.pos = sp - origin.to_vec2();
            } else if let Some(p) = ptr {
                d.pos = p;
            }
        }
        let released = !down || global.is_some_and(|(_, pressed)| !pressed);
        if released {
            ipc::clear_drag();
            self.close_ghost(ctx);
            self.finish_drag(d);
            return;
        }
        // Während des Ziehens außerhalb: anderen Fenstern melden + Vorschau-Fenster am Zeiger zeigen
        if d.outside {
            if let Some(p) = d.px {
                ipc::write_drag([p.x, p.y], &d.payload);
                let target = ipc::window_at([p.x, p.y]).is_some();
                self.show_ghost(ctx, &d, p, target);
            }
        } else {
            ipc::clear_drag();
            self.close_ghost(ctx);
        }
        ctx.request_repaint();
        self.tab_drag = Some(d);
    }

    fn finish_drag(&mut self, d: TabDrag) {
        if d.outside {
            if let Some(p) = d.px {
                if let Some(pid) = ipc::window_at([p.x, p.y]) {
                    self.handoff_to(pid, p, d.kind, &d.payload);
                } else {
                    self.detach_payload(d.kind, d.payload, p);
                }
            }
            return;
        }
        let in_bar = d.pos.y < self.tabbar_rect.bottom();
        // Tab in der Kopfzeile der eigenen Kachel losgelassen → Reihenfolge ändern
        if let (DragKind::Sub(tid, pi, k), Some((rp, to))) = (d.kind, self.sub_reorder) {
            if rp == pi && self.tabs[self.cur].id == tid {
                let t = &mut self.tabs[self.cur];
                if let Some(slot) = t.panes.get_mut(pi) {
                    sub_move(slot, k, to);
                    t.active = pi;
                }
                return;
            }
        }
        // In eine Kachel als Tab aufnehmen (Kachel-Kopf oder Mitte des Kompasses)
        if !in_bar {
            if let Some(leaf) = self.merge_target(d.pos) {
                self.merge_into(d.kind, leaf);
                return;
            }
        }
        if let DragKind::Sub(tid, pi, k) = d.kind {
            let cur = self.cur;
            if self.tabs[cur].id != tid {
                return;
            }
            let zone = if in_bar { None } else { self.zone_at(d.pos) };
            if !in_bar && zone.is_none() {
                return;
            }
            let Some(pane) = self.tabs[cur].panes.get_mut(pi).and_then(|slot| sub_take(slot, k)) else { return };
            match zone {
                Some((leaf, z)) => {
                    let t = &mut self.tabs[cur];
                    let n = t.panes.len();
                    t.panes.push(pane);
                    t.root.split_leaf(leaf, Node::Leaf(n), z);
                    t.active = n;
                    self.status = format!("Angedockt: {}", z.label());
                }
                None => {
                    let id = self.next_tab;
                    self.next_tab += 1;
                    self.tabs.push(Tab::new(id, pane));
                    self.cur = self.tabs.len() - 1;
                }
            }
            return;
        }
        match d.kind {
            DragKind::Tab(id) => {
                if !in_bar {
                    if let (Some(i), Some((leaf, z))) = (self.tabs.iter().position(|t| t.id == id), self.zone_at(d.pos)) {
                        self.dock_tab(i, leaf, z);
                    }
                }
            }
            DragKind::Pane(_, pi) => {
                if in_bar {
                    self.pane_to_tab(pi);
                } else if let Some((leaf, z)) = self.zone_at(d.pos) {
                    self.dock_pane(pi, leaf, z);
                }
            }
            DragKind::Sub(..) => {}
        }
    }

    /// Fenster-Vorschau am Mauszeiger, solange ein Tab außerhalb des Hauptfensters gezogen wird:
    /// Miniatur eines Explorer-Fensters (Titelleiste, Ordner) und darunter, was beim Loslassen passiert.
    /// `px` = Zeiger in physischen Pixeln; die Position wird in die Skalierung des Monitors unter dem Zeiger umgerechnet.
    fn show_ghost(&mut self, ctx: &egui::Context, d: &TabDrag, px: Pos2, into_window: bool) {
        let th = self.th;
        let id = egui::ViewportId::from_hash_of("foldex_ghost");
        let (w, h) = (300.0, 132.0);
        let (scale, mon) = match editors::monitor_at([px.x as i32, px.y as i32]) {
            Some((_, s)) => (s, editors::monitors().iter().position(|m| m.1[0] <= px.x as i32 && px.x as i32 <= m.1[0] + m.1[2] && m.1[1] <= px.y as i32 && px.y as i32 <= m.1[1] + m.1[3])),
            None => (ctx.pixels_per_point(), None),
        };
        let multi_monitor = editors::monitors().len() > 1;
        let grab = d.grab_dx.clamp(0.0, 140.0);
        let pos = pos2(px.x / scale - grab, px.y / scale - 16.0);
        let builder = egui::ViewportBuilder::default()
            .with_title("Foldex")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_resizable(false)
            .with_inner_size([w, h])
            .with_position(pos);
        let title = d.title.clone();
        let paths = d.payload.paths.clone();
        let icons = &mut self.icons;
        ctx.show_viewport_immediate(id, builder, |ctx, _| {
            egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
                let full = ui.max_rect();
                let p = ui.painter();
                // weicher Schatten
                for k in 0..6 {
                    let e = 6.0 - k as f32;
                    p.rect_filled(full.shrink(6.0).expand(e).translate(vec2(0.0, 3.0)), 10.0 + e, Color32::from_black_alpha(10 + k * 4));
                }
                let r = full.shrink(8.0);
                p.rect(r, 10.0, th.bg, Stroke::new(1.0, th.line));
                // Titelleiste wie ein Mini-Fenster
                let bar = Rect::from_min_size(r.min, vec2(r.width(), 34.0));
                p.rect_filled(bar, Rounding { nw: 10.0, ne: 10.0, sw: 0.0, se: 0.0 }, th.panel);
                let tab = Rect::from_min_size(bar.min + vec2(8.0, 6.0), vec2((r.width() * 0.62).min(190.0), 28.0));
                p.rect_filled(tab, Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 }, th.bg);
                let ir = Rect::from_min_size(pos2(tab.left() + 8.0, tab.center().y - 8.0), vec2(16.0, 16.0));
                icons.paint(p, ir, IconKind::Dir, "", "", false, PathBuf::new);
                text_trunc(p, pos2(ir.right() + 7.0, tab.center().y), &title, FontId::proportional(13.0), th.text, tab.right() - 8.0 - (ir.right() + 7.0));
                for (k, c) in [th.sub, th.sub, th.sub].into_iter().enumerate() {
                    p.circle_filled(pos2(bar.right() - 14.0 - k as f32 * 14.0, bar.center().y), 3.0, c.gamma_multiply(0.5));
                }
                // Inhalt: Ordner des Tabs (bei geteilten Tabs mehrere)
                let mut y = bar.bottom() + 8.0;
                for path in paths.iter().take(2) {
                    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                    let ir = Rect::from_min_size(pos2(r.left() + 12.0, y), vec2(16.0, 16.0));
                    icons.paint(p, ir, IconKind::Dir, &name, "", false, PathBuf::new);
                    text_trunc(p, pos2(ir.right() + 8.0, ir.center().y), &path.display().to_string(), FontId::proportional(12.0), th.sub, r.right() - 12.0 - (ir.right() + 8.0));
                    y += 21.0;
                }
                if paths.len() > 2 {
                    p.text(pos2(r.left() + 36.0, y + 6.0), Align2::LEFT_CENTER, format!("+ {} weitere Ansichten", paths.len() - 2), FontId::proportional(12.0), th.sub);
                }
                // Was beim Loslassen passiert
                let pill = Rect::from_min_max(pos2(r.left() + 8.0, r.bottom() - 32.0), pos2(r.right() - 8.0, r.bottom() - 8.0));
                p.rect_filled(pill, 6.0, th.accent.gamma_multiply(0.16));
                let (ic, text) = if into_window {
                    (Ic::Plus, "Loslassen: in dieses Fenster einfügen".to_string())
                } else if multi_monitor {
                    (Ic::Window, format!("Loslassen: neues Fenster{}", mon.map_or(String::new(), |m| format!(" auf Monitor {}", m + 1))))
                } else {
                    (Ic::Window, "Loslassen: neues Fenster".to_string())
                };
                ui::draw(p, Rect::from_center_size(pos2(pill.left() + 16.0, pill.center().y), vec2(14.0, 14.0)), ic, th.accent);
                p.text(pos2(pill.left() + 32.0, pill.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.5), th.accent);
            });
        });
        ctx.send_viewport_cmd_to(id, egui::ViewportCommand::OuterPosition(pos));
    }

    /// Vorschau-Fenster schließen (beim Zurückkehren ins Fenster oder Loslassen).
    fn close_ghost(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of("foldex_ghost"), egui::ViewportCommand::Close);
    }

    /// Vorschau beim Ziehen: Andock-Zonen auf der Kachel unter dem Zeiger (auch für Tabs aus anderen Fenstern).
    fn ui_snap_overlay(&mut self, ctx: &egui::Context) {
        let th = self.th;
        let t = self.at(0.14);
        let screen_rect = ctx.screen_rect();
        let (pos, outside, kind, title, n) = if let Some(d) = &self.tab_drag {
            (d.pos, d.outside, Some(d.kind), d.title.clone(), d.payload.paths.len())
        } else if let Some(f) = &self.foreign {
            let origin = ctx.input(|i| i.viewport().inner_rect).map_or(Pos2::ZERO, |r| r.min);
            let ppp = ctx.pixels_per_point();
            let p = pos2(f.pos[0] / ppp, f.pos[1] / ppp) - origin.to_vec2();
            (p, !screen_rect.contains(p), None, f.payload.title.clone(), f.payload.paths.len())
        } else {
            return;
        };
        if outside {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("snap_overlay")));
        let in_bar = self.tabbar_rect.contains(pos);
        let over_content = !in_bar && self.central_rect.contains(pos);
        let cur_id = self.tabs[self.cur].id;
        let show = ctx.animate_bool_with_time(Id::new("snap_show"), over_content, t);
        if show > 0.01 {
            if let Some((leaf, lr)) = self.leaf_rects.iter().find(|(_, r)| r.contains(pos)).copied() {
                let allowed = match kind {
                    Some(DragKind::Tab(id)) => id != cur_id,
                    Some(DragKind::Pane(_, pi)) => pi != leaf,
                    Some(DragKind::Sub(..)) => true,
                    None => true,
                };
                if allowed {
                    // Kachel leicht abdunkeln, Zielbereich als Vorschau, Kompass in der Mitte
                    painter.rect_filled(lr.shrink(2.0), 8.0, Color32::from_black_alpha((40.0 * show) as u8));
                    if self.sub_reorder.is_some_and(|(p, _)| p == leaf) {
                        ctx.request_repaint();
                        return;
                    }
                    let merge = self.merge_target(pos) == Some(leaf);
                    let zone = if merge { None } else { self.zone_at(pos).filter(|(l, _)| *l == leaf).map(|(_, z)| z) };
                    let mut pill: Option<(Pos2, &str, Ic)> = None;
                    if merge {
                        painter.rect(lr.shrink(6.0), 8.0, th.accent.gamma_multiply(0.18 * show), Stroke::new(2.0, th.accent.gamma_multiply(show)));
                        pill = Some((pos2(lr.center().x, lr.top() + 52.0), "Als Tab in diese Kachel", Ic::Plus));
                    }
                    if let Some(z) = zone {
                        let tr = z.rect(lr).shrink(6.0);
                        let rr = Rect::from_min_max(
                            pos2(ctx.animate_value_with_time(Id::new("sn_a"), tr.min.x, t), ctx.animate_value_with_time(Id::new("sn_b"), tr.min.y, t)),
                            pos2(ctx.animate_value_with_time(Id::new("sn_c"), tr.max.x, t), ctx.animate_value_with_time(Id::new("sn_d"), tr.max.y, t)),
                        );
                        painter.rect(rr, 8.0, th.accent.gamma_multiply(0.22 * show), Stroke::new(2.0, th.accent.gamma_multiply(show)));
                        // Beschriftung am äußeren Rand des Zielbereichs – nie unter dem Kompass
                        let y = if z == Zone::Bottom { rr.bottom() - 26.0 } else { rr.top() + 26.0 };
                        pill = Some((pos2(rr.center().x, y), z.label(), Ic::Split));
                    }
                    for (z, b) in compass(lr) {
                        paint_compass_button(&painter, &th, z, b, zone == Some(z), show);
                    }
                    paint_compass_center(&painter, &th, compass_center(lr), merge, show);
                    // zuletzt: Beschriftung liegt über allem anderen
                    if let Some((at, label, ic)) = pill {
                        let g = painter.layout_no_wrap(label.to_string(), FontId::proportional(13.0), Color32::WHITE);
                        let pr = Rect::from_center_size(at, vec2(g.size().x + 40.0, 26.0));
                        painter.rect_filled(pr.translate(vec2(0.0, 2.0)), 13.0, Color32::from_black_alpha((60.0 * show) as u8));
                        painter.rect_filled(pr, 13.0, th.accent.gamma_multiply(show));
                        ui::draw(&painter, Rect::from_center_size(pos2(pr.left() + 16.0, pr.center().y), vec2(13.0, 13.0)), ic, Color32::WHITE);
                        painter.galley(pos2(pr.left() + 28.0, pr.center().y - g.size().y / 2.0), g, Color32::WHITE);
                    }
                } else {
                    painter.rect_filled(lr.shrink(2.0), 8.0, Color32::from_black_alpha((60.0 * show) as u8));
                    let g = painter.layout_no_wrap("Auf eine andere Kachel ziehen".to_string(), FontId::proportional(13.0), Color32::WHITE);
                    let pill = Rect::from_center_size(lr.center(), vec2(g.size().x + 28.0, 28.0));
                    painter.rect_filled(pill, 14.0, Color32::from_black_alpha((160.0 * show) as u8));
                    painter.galley(pill.center() - g.size() / 2.0, g, Color32::WHITE);
                }
            }
        }
        // Tab-Leiste als Ziel: Teilfenster / fremder Tab wird zu einem neuen Tab
        if in_bar && !matches!(kind, Some(DragKind::Tab(_))) {
            let r = self.tabbar_rect.shrink2(vec2(4.0, 3.0));
            painter.rect(r, 8.0, th.accent.gamma_multiply(0.12), Stroke::new(1.5, th.accent));
            ui::draw(&painter, Rect::from_center_size(r.center() - vec2(70.0, 0.0), vec2(14.0, 14.0)), Ic::Plus, th.accent);
            painter.text(r.center() + vec2(8.0, 0.0), Align2::CENTER_CENTER, "Als eigenen Tab ablegen", FontId::proportional(13.5), th.accent);
        }
        // schwebende Karte am Zeiger (Teilfenster, fremde Tabs und Tabs über dem Inhalt)
        if !in_bar || !matches!(kind, Some(DragKind::Tab(_))) {
            let label = if n > 1 { format!("{title}  +{}", n - 1) } else { title };
            let g = painter.layout_no_wrap(label.clone(), FontId::proportional(13.0), th.text);
            let w = (g.size().x + 52.0).clamp(140.0, 280.0);
            let r = Rect::from_min_size(pos + vec2(16.0, 12.0), vec2(w, 34.0));
            painter.rect_filled(r.translate(vec2(0.0, 3.0)).expand(1.0), 9.0, Color32::from_black_alpha(55));
            painter.rect(r, 8.0, th.bg, Stroke::new(1.0, th.line));
            painter.rect_filled(Rect::from_min_size(r.min + vec2(0.0, 6.0), vec2(3.0, r.height() - 12.0)), 2.0, th.accent);
            ui::draw(&painter, Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(15.0, 15.0)), Ic::Window, th.accent);
            text_trunc(&painter, pos2(r.left() + 36.0, r.center().y), &label, FontId::proportional(13.0), th.text, r.width() - 46.0);
        }
        ctx.request_repaint();
    }

    fn ui_tabs(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let th = self.th;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
        self.tabbar_rect = rect;
        ui.painter().rect_filled(rect, 0.0, th.panel);
        let font = FontId::proportional(13.5);
        // Schalter: Seitenleiste ein-/ausblenden (Strg+B)
        let sr = Rect::from_min_size(pos2(rect.left() + 6.0, rect.top() + 5.0), vec2(34.0, 30.0));
        let tip = format!("Seitenleiste ein-/ausblenden ({})", self.keymap.label("win.sidebar"));
        let sresp = ui::tool_btn_at(ui, &th, sr, Id::new("sidebtn"), Ic::Sidebar, &tip, self.side_visible);
        if sresp.clicked() {
            acts.push(Act::ToggleSidebar);
        }
        let n = self.tabs.len();
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let screen = ui.ctx().screen_rect();
        let anim_t = self.at(0.12);
        // Titel, Breiten und Zielpositionen
        let mut infos: Vec<(String, PathBuf, u64, usize)> = Vec::with_capacity(n);
        let mut widths = Vec::with_capacity(n);
        let mut kinds: Vec<Option<Ic>> = Vec::with_capacity(n);
        let mut tips: Vec<String> = Vec::with_capacity(n);
        for t in &self.tabs {
            // geteilte Tabs zeigen nur das Split-Symbol; die Kacheln stehen im Tooltip
            let s = if t.dual() { String::new() } else { t.title() };
            tips.push(t.panes.iter().map(pane_title).collect::<Vec<_>>().join("  |  "));
            let tw = ui.painter().layout_no_wrap(s.clone(), font.clone(), th.text).size().x;
            widths.push(if t.dual() { 66.0 } else { (tw + 52.0).clamp(110.0, 240.0) });
            // Terminal-/Git-Kacheln sind kein Ablageziel für Dateien
            let ap = &t.panes[t.active.min(t.panes.len() - 1)];
            let cwd = if ap.extra.is_some() { PathBuf::new() } else { ap.cwd.clone() };
            infos.push((s, cwd, t.id, t.panes.len()));
            kinds.push(if t.dual() { Some(Ic::Split) } else { pane_icon(ap) });
        }
        // Viele Tabs: erst gleichmäßig schmaler (bis 110 px), reicht das nicht, wird die Leiste waagerecht
        // scrollbar (Mausrad, Ziehen an der dezenten Leiste) – kein Tab wird abgeschnitten oder unlesbar.
        let strip = Rect::from_min_max(pos2(rect.left() + 46.0, rect.top()), pos2(rect.right() - 96.0 - 40.0, rect.bottom()));
        let sum: f32 = widths.iter().map(|w| w + 2.0).sum();
        if sum > strip.width() && n > 0 {
            let k = (strip.width() / sum).max(0.0);
            for w in &mut widths {
                *w = (*w * k).max(110.0_f32.min(*w));
            }
        }
        let total: f32 = widths.iter().map(|w| w + 2.0).sum();
        let overflow = total > strip.width() + 0.5;
        let max_scroll = (total - strip.width()).max(0.0);
        if overflow {
            if ui.rect_contains_pointer(rect) {
                let d = ui.input(|i| i.smooth_scroll_delta);
                self.tab_scroll -= if d.x.abs() > d.y.abs() { d.x } else { d.y };
            }
            // aktiven Tab sichtbar halten, wenn er gewechselt hat
            if self.tab_scroll_cur != self.cur && self.cur < n {
                let left: f32 = widths[..self.cur].iter().map(|w| w + 2.0).sum();
                let right = left + widths[self.cur];
                if left < self.tab_scroll {
                    self.tab_scroll = left;
                } else if right > self.tab_scroll + strip.width() {
                    self.tab_scroll = right - strip.width();
                }
            }
        }
        self.tab_scroll_cur = self.cur;
        self.tab_scroll = if overflow { self.tab_scroll.clamp(0.0, max_scroll) } else { 0.0 };
        let mut targets = Vec::with_capacity(n);
        let mut x = strip.left() - self.tab_scroll;
        for w in &widths {
            targets.push(x);
            x += w + 2.0;
        }
        let outer_clip = ui.clip_rect();
        if overflow {
            ui.set_clip_rect(strip.intersect(outer_clip));
        }
        // Umsortieren beim Ziehen: Position anhand der Mitte des gezogenen Tabs bestimmen
        if let (Some(d), Some(p)) = (&self.tab_drag, pointer) {
            let tid = if let DragKind::Tab(id) = d.kind { Some(id) } else { None };
            if let Some(di) = tid.and_then(|id| infos.iter().position(|t| t.2 == id)) {
                if p.y < rect.bottom() && !d.outside {
                    let center = p.x - d.grab_dx + widths[di] / 2.0;
                    let to = (0..n).filter(|&k| k != di && targets[k] + widths[k] / 2.0 < center).count();
                    if to != di {
                        acts.push(Act::TabMove(di, to));
                    }
                }
            }
        }
        let mut lifted: Option<(Rect, String, usize, Option<Ic>)> = None;
        for i in 0..n {
            let (title, tab_cwd, tab_id, npanes) = infos[i].clone();
            let (w, tx) = (widths[i], targets[i]);
            let dragging = self.tab_drag.as_ref().is_some_and(|d| d.kind == DragKind::Tab(tab_id));
            // sanft zur Zielposition gleiten
            let ax = {
                let e = self.tab_x.entry(tab_id).or_insert(tx);
                if anim_t == 0.0 {
                    *e = tx;
                } else {
                    *e += (tx - *e) * (1.0 - (-dt * 22.0).exp());
                    if (tx - *e).abs() < 0.3 {
                        *e = tx;
                    } else {
                        ui.ctx().request_repaint();
                    }
                }
                *e
            };
            let hit = Rect::from_min_size(pos2(tx, rect.top() + 5.0), vec2(w, rect.height() - 5.0));
            // halb sichtbare Tabs nur im sichtbaren Teil anklickbar
            let hit = if overflow { hit.intersect(strip) } else { hit };
            let mut resp = ui.interact(hit, Id::new(("tab", tab_id)), Sense::click_and_drag());
            if title.is_empty() {
                resp = resp.on_hover_text(format!("Geteilte Ansicht: {}", tips[i]));
            }
            if resp.drag_started() && self.tab_drag.is_none() {
                if let Some(p) = pointer {
                    let payload = self.tab_payload(i);
                    self.tab_drag = Some(TabDrag {
                        kind: DragKind::Tab(tab_id),
                        title: title.clone(),
                        grab_dx: p.x - tx,
                        pos: p,
                        screen: None,
                        px: None,
                        outside: false,
                        payload,
                    });
                }
            }
            let selected = i == self.cur;
            let target = self.drag.is_some() && !tab_cwd.as_os_str().is_empty() && self.drop_target == Some(Dest::Dir(tab_cwd.clone()));
            let hover_t = ui.ctx().animate_bool_with_time(Id::new(("tabh", tab_id)), (resp.hovered() || target) && !dragging, anim_t);
            let r = Rect::from_min_size(pos2(ax, rect.top() + 5.0), vec2(w, rect.height() - 5.0));
            if dragging {
                let px = pointer.map_or(ax, |p| p.x - self.tab_drag.as_ref().map_or(0.0, |d| d.grab_dx));
                let lr = Rect::from_min_size(pos2(px.clamp(rect.left(), rect.right() - w), rect.top() + 3.0), r.size());
                lifted = Some((lr, title.clone(), npanes, kinds[i]));
            } else {
                self.paint_tab(ui, r, &title, selected, hover_t, target, false, npanes, kinds[i]);
                if !tab_cwd.as_os_str().is_empty() {
                    self.zones.push((r, Dest::Dir(tab_cwd)));
                }
                // Schließen-Knopf
                let cr = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(20.0, 20.0));
                let cr_hit = if overflow { cr.intersect(strip) } else { cr };
                let cresp = ui.interact(cr_hit, Id::new(("tabx", tab_id)), Sense::click());
                if resp.hovered() || selected {
                    if cresp.hovered() {
                        ui.painter().rect_filled(cr, 4.0, th.hover);
                    }
                    let c = cr.center();
                    let s = Stroke::new(1.3, th.sub);
                    ui.painter().line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], s);
                    ui.painter().line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], s);
                }
                if cresp.clicked() || resp.middle_clicked() {
                    acts.push(Act::TabClose(i));
                } else if resp.clicked() {
                    acts.push(Act::TabSelect(i));
                }
            }
            resp.context_menu(|ui| {
                if menu_item(ui, "Tab schließen", "Strg+W", true) {
                    acts.push(Act::TabClose(i));
                }
                if menu_item(ui, "Andere Tabs schließen", "", n > 1) {
                    acts.push(Act::TabCloseOthers(i));
                }
                if menu_item(ui, "Tab duplizieren", "", true) {
                    acts.push(Act::TabDup(i));
                }
                if menu_item(ui, "In neuem Fenster öffnen", "", n > 1) {
                    let origin = ui.ctx().input(|i| i.viewport().inner_rect).map_or(Pos2::ZERO, |r| r.min);
                    acts.push(Act::TabDetach(i, origin + vec2(120.0, 90.0)));
                }
            });
        }
        ui.set_clip_rect(outer_clip);
        if overflow {
            // dezente Scrollleiste unter den Tabs (ziehbar)
            let track = Rect::from_min_max(pos2(strip.left(), rect.bottom() - 4.0), pos2(strip.right(), rect.bottom()));
            let len = (strip.width() * strip.width() / total).max(30.0);
            let pos = if max_scroll > 0.0 { self.tab_scroll / max_scroll * (strip.width() - len) } else { 0.0 };
            let thumb = Rect::from_min_size(pos2(strip.left() + pos, track.top()), vec2(len, track.height()));
            let tresp = ui.interact(track.expand2(vec2(0.0, 2.0)), Id::new("tabscroll"), Sense::click_and_drag());
            if tresp.dragged() {
                self.tab_scroll = (self.tab_scroll + tresp.drag_delta().x * max_scroll / (strip.width() - len).max(1.0)).clamp(0.0, max_scroll);
            }
            let hot = tresp.hovered() || tresp.dragged() || ui.rect_contains_pointer(rect);
            ui.painter().rect_filled(thumb, 2.0, th.sub.gamma_multiply(if hot { 0.55 } else { 0.25 }));
            // Kanten zeigen, dass es links/rechts weitergeht
            for (edge, dir) in [(strip.left(), 1.0), (strip.right(), -1.0)] {
                let more = if dir > 0.0 { self.tab_scroll > 0.5 } else { self.tab_scroll < max_scroll - 0.5 };
                if more {
                    for k in 0..10 {
                        let a = (10 - k) as f32 / 10.0;
                        let xk = edge + dir * k as f32 * 1.6;
                        ui.painter().line_segment([pos2(xk, rect.top() + 5.0), pos2(xk, rect.bottom() - 5.0)], Stroke::new(1.6, th.panel.gamma_multiply(a)));
                    }
                }
            }
        }
        if let Some((lr, title, np, ic)) = lifted {
            // nur solange der Zeiger über der Leiste ist; darunter übernimmt die schwebende Karte
            if self.tab_drag.as_ref().is_some_and(|d| !d.outside && d.pos.y < rect.bottom()) {
                self.paint_tab(ui, lr, &title, true, 1.0, false, true, np, ic);
            }
        }
        self.tab_x.retain(|id, _| infos.iter().any(|t| t.2 == *id));
        let _ = screen;
        if self.tab_drag.is_some() {
            ui.ctx().request_repaint();
        }
        // „+“: Klick = neuer Tab; Rechtsklick = Terminal, Git, neues Fenster (auch auf anderem Monitor)
        let pr = Rect::from_min_size(pos2(if overflow { strip.right() + 4.0 } else { x + 2.0 }, rect.top() + 6.0), vec2(32.0, 28.0));
        let tip = format!("Neuer Tab ({})  ·  Rechtsklick: Terminal, Fenster, Monitor", self.keymap.label("tab.new"));
        let presp = ui::tool_btn_at(ui, &th, pr, Id::new("tabplus"), Ic::Plus, &tip, false);
        if presp.clicked() {
            acts.push(Act::TabNew);
        }
        let cwd = self.pane().cwd.clone();
        let keys = self.keymap.clone();
        let git_here = self.git_st.is_some();
        let shells = self.shells.clone();
        let backends = self.backends();
        let ai = self.st.ai_enabled;
        presp.context_menu(|ui| new_menu(ui, &cwd, &keys, &shells, &backends, ai, git_here, acts));
        // rechts: Command Palette und Einstellungen
        let gr = Rect::from_min_size(pos2(rect.right() - 42.0, rect.top() + 5.0), vec2(34.0, 30.0));
        let tip = format!("Einstellungen ({})", self.keymap.label("app.settings"));
        if ui::tool_btn_at(ui, &th, gr, Id::new("setbtn"), Ic::Gear, &tip, self.show_settings).clicked() {
            acts.push(Act::ShowSettings);
        }
        let cr = gr.translate(vec2(-36.0, 0.0));
        let tip = format!("Command Palette ({})", self.keymap.label("app.palette"));
        if ui::tool_btn_at(ui, &th, cr, Id::new("palbtn"), Ic::Command, &tip, self.palette.is_some()).clicked() {
            acts.push(Act::Cmd("app.palette"));
        }
    }

}

/// Kachel in Liste, Vorschau und Griff dazwischen aufteilen.
fn preview_split(full: Rect, side: PvSide, share: f32) -> (Rect, Rect, Rect) {
    const G: f32 = 6.0;
    if side.horizontal() {
        let w = (full.width() * share).clamp(200.0, (full.width() - 240.0).max(200.0));
        match side {
            PvSide::Left => (
                Rect::from_min_max(pos2(full.left() + w + G, full.top()), full.max),
                Rect::from_min_max(full.min, pos2(full.left() + w, full.bottom())),
                Rect::from_min_max(pos2(full.left() + w, full.top()), pos2(full.left() + w + G, full.bottom())),
            ),
            _ => (
                Rect::from_min_max(full.min, pos2(full.right() - w - G, full.bottom())),
                Rect::from_min_max(pos2(full.right() - w, full.top()), full.max),
                Rect::from_min_max(pos2(full.right() - w - G, full.top()), pos2(full.right() - w, full.bottom())),
            ),
        }
    } else {
        let h = (full.height() * share).clamp(140.0, (full.height() - 140.0).max(140.0));
        match side {
            PvSide::Top => (
                Rect::from_min_max(pos2(full.left(), full.top() + h + G), full.max),
                Rect::from_min_max(full.min, pos2(full.right(), full.top() + h)),
                Rect::from_min_max(pos2(full.left(), full.top() + h), pos2(full.right(), full.top() + h + G)),
            ),
            _ => (
                Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - h - G)),
                Rect::from_min_max(pos2(full.left(), full.bottom() - h), full.max),
                Rect::from_min_max(pos2(full.left(), full.bottom() - h - G), pos2(full.right(), full.bottom() - h)),
            ),
        }
    }
}

/// Symbol „Vorschau links/rechts/oben/unten“: Fensterumriss mit gefülltem Streifen auf der jeweiligen Seite.
fn paint_side_icon(p: &egui::Painter, c: Pos2, s: PvSide, col: Color32) {
    let r = Rect::from_center_size(c, vec2(14.0, 11.0));
    p.rect_stroke(r, 2.0, Stroke::new(1.2, col));
    let band = match s {
        PvSide::Left => Rect::from_min_max(r.min, pos2(r.left() + 5.0, r.bottom())),
        PvSide::Right => Rect::from_min_max(pos2(r.right() - 5.0, r.top()), r.max),
        PvSide::Top => Rect::from_min_max(r.min, pos2(r.right(), r.top() + 4.5)),
        PvSide::Bottom => Rect::from_min_max(pos2(r.left(), r.bottom() - 4.5), r.max),
    };
    p.rect_filled(band, 2.0, col);
}

/// Menü „Neu“ der Tab-Leiste: Tab, Terminal (je Shell), Git, Fenster (je Monitor).
#[allow(clippy::too_many_arguments)]
/// KI-Einträge für Menüs: Explorer Assistent/Agent (Seitenleiste) und je ein Eintrag pro Agent und eigenem Modell.
fn ai_menu(ui: &mut egui::Ui, cwd: &Path, keys: &Keymap, backends: &[(String, String)], acts: &mut Vec<Act>) {
    ui::menu_label(ui, "Explorer-KI");
    if ui::menu_entry(ui, Some(Ic::Spark), "Explorer Assistent", &keys.label("ai.assistant"), true, false) {
        acts.push(Act::EditorAi(false));
    }
    if ui::menu_entry(ui, Some(Ic::Spark), "Explorer Agent", &keys.label("ai.agent"), true, false) {
        acts.push(Act::EditorAi(true));
    }
    if ui::menu_entry(ui, Some(Ic::Spark), "Chat (ohne Explorer)", &keys.label("ai.chat"), true, false) {
        acts.push(Act::EditorChat);
    }
    if ui::menu_entry(ui, Some(Ic::Spark), "Bild erzeugen (lokal)", "", true, false) {
        acts.push(Act::ImageGen(cwd.to_path_buf()));
    }
    if !backends.is_empty() {
        ui::menu_sep(ui);
        ui::menu_label(ui, "KI-Chats");
        for (id, name) in backends {
            let hint = if id.starts_with("agent:") { "Agent" } else { "eigenes Modell" };
            if ui::menu_entry(ui, Some(Ic::Spark), name, hint, true, false) {
                acts.push(Act::AiChat(cwd.to_path_buf(), Some(id.clone()), id.starts_with("agent:")));
            }
        }
    }
}

fn new_menu(ui: &mut egui::Ui, cwd: &Path, keys: &Keymap, shells: &[(String, String)], backends: &[(String, String)], ai: bool, git_here: bool, acts: &mut Vec<Act>) {
    if ui::menu_entry(ui, Some(Ic::Plus), "Neuer Tab", &keys.label("tab.new"), true, false) {
        acts.push(Act::TabNew);
    }
    ui::menu_sep(ui);
    ui::menu_label(ui, "Terminal in diesem Ordner");
    for (i, (name, prog)) in shells.iter().cloned().enumerate() {
        let hint = if i == 0 { keys.label("term.new") } else { String::new() };
        if ui::menu_entry(ui, Some(Ic::Terminal), &name, &hint, true, false) {
            acts.push(Act::Terminal(Some(prog), cwd.to_path_buf()));
        }
    }
    if ai {
        ui::menu_sep(ui);
        ai_menu(ui, cwd, keys, backends, acts);
    }
    ui::menu_sep(ui);
    if ui::menu_entry(ui, Some(Ic::Git), if git_here { "Git-Repository" } else { "Git-Repository erstellen / klonen" }, &keys.label("git.view"), true, false) {
        acts.push(Act::GitView(cwd.to_path_buf()));
    }
    ui::menu_sep(ui);
    if ui::menu_entry(ui, Some(Ic::Window), "Neues Fenster", &keys.label("win.new"), true, false) {
        acts.push(Act::NewWindow(None));
    }
    let mons = editors::monitors();
    if mons.len() > 1 {
        ui::menu_label(ui, "Neues Fenster auf Monitor");
        for (k, (name, work, primary)) in mons.into_iter().enumerate() {
            let label = format!("Monitor {}{} – {}×{}", k + 1, if primary { " (Haupt)" } else { "" }, work[2], work[3]);
            if ui::menu_entry(ui, Some(Ic::Pc), &label, &name, true, false) {
                acts.push(Act::NewWindow(Some(work)));
            }
        }
    }
}

impl App {
    // ── Navigationsleiste eines Panes ──
    fn pane_nav(&mut self, ui: &mut egui::Ui, pn: &mut Pane, acts: &mut Vec<Act>) {
        let th = self.th;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(6.0);
            if nav_button(ui, &th, Arrow::Left, !pn.back.is_empty()) {
                acts.push(Act::Back);
            }
            if nav_button(ui, &th, Arrow::Right, !pn.fwd.is_empty()) {
                acts.push(Act::Forward);
            }
            if nav_button(ui, &th, Arrow::Up, pn.cwd.parent().is_some()) {
                acts.push(Act::Up);
            }
            ui.add_space(4.0);
            let total = ui.available_width() - 8.0;
            let search_w = (total * 0.34).clamp(130.0, 280.0);
            self.ui_crumbs(ui, pn, total - search_w - 8.0, acts);
            ui.add_space(6.0);
            self.ui_search(ui, pn, search_w, acts);
        });
        ui.add_space(4.0);
    }

    fn ui_crumbs(&mut self, ui: &mut egui::Ui, pn: &mut Pane, w: f32, acts: &mut Vec<Act>) {
        let th = self.th;
        let (rect, resp) = ui.allocate_exact_size(vec2(w.max(60.0), 34.0), Sense::click());
        ui.painter()
            .rect(rect, 6.0, th.field, Stroke::new(1.0, th.line));
        if pn.crumb_edit.is_some() {
            let mut buf = pn.crumb_edit.take().unwrap();
            let te = egui::TextEdit::singleline(&mut buf)
                .frame(false)
                .margin(Margin::symmetric(10.0, 8.0));
            let r = ui.put(rect, te);
            if std::mem::take(&mut pn.crumb_focus) {
                r.request_focus();
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(buf.chars().count()),
                    )));
                    st.store(ui.ctx(), r.id);
                }
            }
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            let esc = ui.input(|i| i.key_pressed(Key::Escape));
            if r.lost_focus() && enter {
                acts.push(Act::Open(PathBuf::from(buf.trim())));
            } else if !(r.lost_focus() || esc) {
                pn.crumb_edit = Some(buf);
            }
            return;
        }
        let mut segs: Vec<(String, PathBuf)> = Vec::new();
        for a in pn.cwd.ancestors() {
            let label = match a.file_name() {
                Some(n) => n.to_string_lossy().into_owned(),
                None => a.display().to_string().trim_end_matches('\\').to_string(),
            };
            segs.push((
                if label.is_empty() { "/".into() } else { label },
                a.to_path_buf(),
            ));
        }
        segs.reverse();
        let font = FontId::proportional(14.0);
        let widths: Vec<f32> = segs
            .iter()
            .map(|(l, _)| {
                ui.painter()
                    .layout_no_wrap(l.clone(), font.clone(), th.text)
                    .size()
                    .x
                    + 16.0
            })
            .collect();
        let avail = rect.width() - 16.0;
        let mut start = segs.len();
        let mut used = 0.0;
        while start > 0 {
            let need = widths[start - 1] + 14.0;
            if used + need > avail && start < segs.len() {
                break;
            }
            used += need;
            start -= 1;
        }
        let mut x = rect.left() + 8.0;
        if start > 0 {
            ui.painter().text(
                pos2(x + 6.0, rect.center().y),
                Align2::CENTER_CENTER,
                "…",
                font.clone(),
                th.sub,
            );
            x += 16.0;
        }
        for i in start..segs.len() {
            let r = Rect::from_min_size(
                pos2(x, rect.top() + 4.0),
                vec2(widths[i], rect.height() - 8.0),
            );
            let sr = ui.interact(r, resp.id.with(("crumb", i)), Sense::click());
            let is_target =
                self.drag.is_some() && self.drop_target == Some(Dest::Dir(segs[i].1.clone()));
            if sr.hovered() || is_target {
                ui.painter()
                    .rect_filled(r, 4.0, if is_target { th.sel } else { th.hover });
            }
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                &segs[i].0,
                font.clone(),
                th.text,
            );
            self.zones.push((r, Dest::Dir(segs[i].1.clone())));
            if sr.clicked() {
                acts.push(Act::Go(segs[i].1.clone()));
            }
            x += widths[i];
            if i + 1 < segs.len() {
                paint_chevron_right(ui.painter(), pos2(x + 7.0, rect.center().y), th.sub);
                x += 14.0;
            }
        }
        if resp.clicked() {
            acts.push(Act::FocusAddr);
        }
    }

    fn ui_search(&mut self, ui: &mut egui::Ui, pn: &mut Pane, w: f32, acts: &mut Vec<Act>) {
        let th = self.th;
        let (rect, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
        ui.painter()
            .rect(rect, 6.0, th.field, Stroke::new(1.0, th.line));
        let c = pos2(rect.left() + 17.0, rect.center().y - 1.0);
        ui.painter().circle_stroke(c, 5.0, Stroke::new(1.5, th.sub));
        ui.painter().line_segment(
            [c + vec2(3.6, 3.6), c + vec2(8.0, 8.0)],
            Stroke::new(1.5, th.sub),
        );
        let hint = format!("»{}« durchsuchen  (inhalt:wort für Dateiinhalte)", pn.title());
        let te = egui::TextEdit::singleline(&mut pn.query)
            .frame(false)
            .hint_text(hint)
            .margin(Margin {
                left: 34.0,
                right: 8.0,
                top: 8.0,
                bottom: 8.0,
            });
        let r = ui.put(rect, te);
        if std::mem::take(&mut pn.focus_search) {
            r.request_focus();
        }
        if r.changed() {
            acts.push(Act::Search);
        }
        if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
            acts.push(Act::ClearSearch);
        }
    }

    // ── Seitenleiste ──
    /// liegt der Ordner in einem Cloud-Ordner? (gemerkt, damit es nicht für jede Zeile neu geprüft wird)
    fn in_cloud(&self, dir: &Path) -> bool {
        let mut m = self.cloud_memo.borrow_mut();
        match &*m {
            Some((d, v)) if d == dir => *v,
            _ => {
                // nur Ordner der Anbieter-Programme haben Sync-Status (rclone-Laufwerke nicht)
                let v = cloud::place_of(dir).is_some_and(|p| p.remote.is_none());
                *m = Some((dir.to_path_buf(), v));
                v
            }
        }
    }

    /// Cloud-Dienst in der Seitenleiste: Logo, Name; Klick öffnet den Ordner
    fn side_cloud_row(&mut self, ui: &mut egui::Ui, idx: usize, c: &cloud::Place, cwd: &Path, acts: &mut Vec<Act>) {
        let th = self.th;
        // freier Speicher des Kontos (nur bei in Foldex angemeldeten Konten bekannt)
        // eigenes Konto → rclone/Anbieter; App-Ordner → Konto desselben Anbieters in Foldex oder das Laufwerk des
        // Anbieters (Google Drive, pCloud melden dort ihren Online-Speicher). Ordner auf der Systemplatte zählen nicht.
        let space = match c.remote.as_deref() {
            Some(r) => cloudlogin::space(r),
            None => cloudlogin::accounts().into_iter().find(|a| a.key == c.key && !a.off).and_then(|a| cloudlogin::space(&a.remote)).or_else(|| {
                let root = PathBuf::from(format!("{}\\", c.path.to_string_lossy().get(..2)?));
                let home = dirs::home_dir().map(|h| h.to_string_lossy().get(..2).unwrap_or_default().to_lowercase()).unwrap_or_default();
                (root.to_string_lossy()[..2].to_lowercase() != home).then(|| self.drive_space.get(&root).copied()).flatten()
            }),
        };
        // ohne Zahl: Hinweis, wie man sie bekommt
        let hint = space.is_none() && c.remote.is_none();
        // Anbieter-Programm installiert, läuft aber nicht → der Ordner wird nicht abgeglichen
        let app_off = c.remote.is_none() && !cloud::app_running(c.key);
        let h = if space.is_some() { SIDE_ROW_H + 34.0 } else if hint { SIDE_ROW_H + 14.0 } else { SIDE_ROW_H };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
        let inner = rect.shrink2(vec2(6.0, 1.0));
        let current = cwd.starts_with(&c.path);
        // Reihenfolge per Ziehen ändern
        self.cloud_rects.push(rect);
        if resp.drag_started() && self.drag.is_none() {
            self.cloud_drag = Some(idx);
        }
        if self.cloud_drag == Some(idx) {
            ui.painter().rect_filled(inner, 5.0, th.sel);
        }
        let is_target = self.drag.is_some() && self.drop_target == Some(Dest::Dir(c.path.clone()));
        if current || is_target {
            ui.painter().rect_filled(inner, 6.0, th.sel);
        } else if resp.hovered() && self.drag.is_none() {
            ui.painter().rect_filled(inner, 6.0, th.hover);
        }
        let cy = if space.is_some() || hint { rect.top() + SIDE_ROW_H / 2.0 } else { rect.center().y };
        let ir = Rect::from_min_size(pos2(inner.left() + 12.0, cy - 9.0), vec2(18.0, 18.0));
        match foldericons::get(&c.path) {
            Some(spec) if self.icons.paint_spec(ui.painter(), ir, &spec) => {}
            _ => cloud::logo(ui.painter(), ir, c.key),
        }
        let tx = ir.right() + 10.0;
        text_trunc(ui.painter(), pos2(tx, cy), &c.name, FontId::proportional(14.0), th.text, inner.right() - 8.0 - tx);
        // Belegung wie bei den Laufwerken: Balken + „x GB frei von y GB“ (rot ab 90 % belegt)
        if let Some((free, total)) = space {
            let frac = if total > 0 { total.saturating_sub(free) as f32 / total as f32 } else { 0.0 };
            let bx = pos2(ir.left(), rect.top() + SIDE_ROW_H + 1.0);
            let bar = Rect::from_min_size(bx, vec2((inner.right() - 14.0 - bx.x).max(20.0), 8.0));
            ui.painter().rect_filled(bar, 3.0, th.line);
            let fill = Rect::from_min_size(bar.min, vec2(bar.width() * frac.clamp(0.0, 1.0), bar.height()));
            ui.painter().rect_filled(fill, 3.0, if frac > 0.9 { Color32::from_rgb(210, 70, 60) } else { th.accent });
            ui.painter().text(pos2(bar.left(), bar.bottom() + 5.0), Align2::LEFT_TOP, format!("{} frei von {}", fsops::human(free), fsops::human(total)), FontId::proportional(11.5), th.sub);
        }
        if hint {
            let t = if app_off { "App aus · Rechtsklick: anmelden" } else { "Rechtsklick: Speicher anzeigen" };
            text_trunc(ui.painter(), pos2(tx, cy + 15.0), t, FontId::proportional(11.0), th.sub, inner.right() - 8.0 - tx);
        }
        self.zones.push((rect, Dest::Dir(c.path.clone())));
        let resp = resp.on_hover_text(c.path.display().to_string());
        if resp.middle_clicked() {
            acts.push(Act::TabOpen(c.path.clone()));
        }
        if resp.clicked() {
            acts.push(Act::Go(c.path.clone()));
        }
        let p = c.path.clone();
        let remote = c.remote.clone();
        let key = c.key;
        let name = c.name.clone();
        resp.context_menu(|ui| {
            if menu_item(ui, "Öffnen", "", true) {
                acts.push(Act::Go(p.clone()));
            }
            if menu_item(ui, "In neuem Tab öffnen", "", true) {
                acts.push(Act::TabNew);
                acts.push(Act::Go(p.clone()));
            }
            if menu_item(ui, "An Schnellzugriff anheften", "", true) {
                acts.push(Act::Pin(p.clone()));
            }
            if menu_item(ui, "Icon ändern …", "", true) {
                acts.push(Act::FolderIcon(p.clone()));
            }
            ui::menu_sep(ui);
            ui::menu_sep(ui);
            if menu_item(ui, "Nach oben", "", idx > 0) {
                acts.push(Act::CloudMove(idx, idx.saturating_sub(1)));
            }
            if menu_item(ui, "Nach unten", "", true) {
                acts.push(Act::CloudMove(idx, idx + 1));
            }
            if hint && cloudlogin::auth(key).is_some() && menu_item(ui, "In Foldex anmelden (ohne App, mit freiem Speicher) …", "", true) {
                acts.push(Act::CloudLogin(key));
            }
            if menu_item(ui, "Weitere Cloud-Dienste …", "", true) {
                acts.push(Act::CloudSetup);
            }
            if (remote.is_some() && cloudtrash::in_app(key)) || cloudtrash::web_url(key).is_some() {
                if menu_item(ui, "Papierkorb anzeigen", "", true) {
                    acts.push(Act::CloudTrash(key, remote.clone(), name.clone()));
                }
            }
            if menu_item(ui, "Aus der Seitenleiste entfernen", "", true) {
                acts.push(Act::SideHide(p.clone()));
            }
            if menu_item(ui, "Eigenschaften", "", true) {
                acts.push(Act::Props(p.clone()));
            }
            if let Some(r) = &remote {
                ui::menu_sep(ui);
                if menu_item(ui, "Laufwerk trennen", "", true) {
                    acts.push(Act::CloudConnect(r.clone(), false));
                }
                if menu_item(ui, "Abmelden und entfernen", "", true) {
                    acts.push(Act::CloudLogout(r.clone()));
                }
            }
        });
    }

    /// Fenster „Inhaltssuche“: Anfrage, Fortschritt, Treffer mit Begründung
    fn ui_content_search(&mut self, ctx: &egui::Context) {
        let Some((q, dir, search)) = self.csearch.as_mut() else { return };
        let th = self.th;
        let mut open = true;
        let mut go = false;
        let mut acts = Vec::new();
        let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
        egui::Window::new("Inhaltssuche").open(&mut open).collapsible(false).resizable(true).default_width(640.0).max_width(720.0).default_height(520.0).show(ctx, |ui| {
            ui.label(egui::RichText::new(format!("Sucht in PDFs, Bildern (Texterkennung) und Dokumenten unter „{name}“ – alles lokal.")).size(12.0).color(th.sub));
            ui.horizontal(|ui| {
                let te = ui.add(egui::TextEdit::singleline(q).hint_text("z. B. finde die Rechnung von Media Markt").desired_width(ui.available_width() - 80.0));
                if search.is_none() && !te.has_focus() && q.is_empty() {
                    te.request_focus();
                }
                if (te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) || ui.button("Suchen").clicked() {
                    go = true;
                }
            });
            let Some(s) = search.as_ref() else { return };
            let (n, t, done) = (s.scanned.load(Relaxed), s.total.load(Relaxed), s.done.load(Relaxed));
            ui.horizontal(|ui| {
                if !done {
                    ui.spinner();
                    if t > 0 {
                        ui.add(egui::ProgressBar::new(n as f32 / t as f32).desired_width(160.0).desired_height(6.0));
                    }
                    ui.label(egui::RichText::new(format!("{n} von {t} Dateien gelesen …")).size(12.0).color(th.sub));
                    if ui.small_button("Stopp").clicked() {
                        s.cancel.store(true, Relaxed);
                    }
                } else {
                    let k = s.hits.lock().map_or(0, |h| h.len());
                    ui.label(egui::RichText::new(format!("{t} Dateien durchsucht – {k} Treffer")).size(12.0).color(th.sub));
                }
            });
            ui.separator();
            let Ok(hits) = s.hits.lock() else { return };
            if hits.is_empty() && done {
                ui.label("Nichts gefunden. Andere Begriffe versuchen oder einen übergeordneten Ordner öffnen.");
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for h in hits.iter() {
                    let fname = h.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let fr = egui::Frame::none().fill(th.field).rounding(6.0).inner_margin(Margin::same(8.0)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new(&fname).strong());
                        ui.label(egui::RichText::new(format!("Passt, weil: {}", h.why)).size(12.0).color(th.accent));
                        if !h.snippet.is_empty() {
                            ui.label(egui::RichText::new(&h.snippet).size(12.0).color(th.sub).italics());
                        }
                        ui.add(egui::Label::new(egui::RichText::new(h.path.parent().map(|p| p.display().to_string()).unwrap_or_default()).size(11.0).color(th.sub)).truncate());
                    });
                    let r = ui.interact(fr.response.rect, Id::new(("cs_hit", &h.path)), Sense::click());
                    if r.double_clicked() {
                        acts.push(Act::OpenFile(h.path.clone()));
                    } else if r.clicked() {
                        acts.push(Act::Preview(h.path.clone()));
                    }
                    if r.secondary_clicked() {
                        acts.push(Act::Reveal(h.path.clone()));
                    }
                    r.on_hover_text("Klick: Schnellansicht · Doppelklick: öffnen · Rechtsklick: im Ordner zeigen");
                    ui.add_space(4.0);
                }
            });
        });
        if go && !q.trim().is_empty() {
            if let Some(s) = search.as_ref() {
                s.cancel.store(true, Relaxed);
            }
            *search = Some(contentsearch::Search::start(q, dir, ctx));
        }
        if !open {
            if let Some((_, _, Some(s))) = &self.csearch {
                s.cancel.store(true, Relaxed);
            }
            self.csearch = None;
        }
        for a in acts {
            self.apply(a);
        }
    }

    /// Fenster „Bilder bearbeiten“ und Start im Hintergrund
    fn ui_batch(&mut self, ctx: &egui::Context) {
        let Some(o) = self.batch.as_mut() else { return };
        match batch::window(ctx, &self.th, o) {
            Some(true) => {
                let o = self.batch.take().unwrap();
                let (n, label) = (o.paths.len() as u64, format!("Bilder bearbeiten ({})", o.label()));
                let done = Arc::new(AtomicU64::new(0));
                let cancel = Arc::new(AtomicBool::new(false));
                let (d2, c2) = (done.clone(), cancel.clone());
                let refresh = o.paths.first().and_then(|p| p.parent()).map(Path::to_path_buf);
                self.run_job_ex(&label, Some(done), n, Some(cancel), move || {
                    let msg = batch::run(o, &d2, &c2);
                    let _ = refresh;
                    JobDone { msg, hist: Hist::None, locked: None }
                });
            }
            Some(false) => self.batch = None,
            None => {}
        }
    }

    /// Rückfrage vor dem Deinstallieren
    fn ui_uninstall(&mut self, ctx: &egui::Context) {
        let Some(mut del_exe) = self.uninstall_ask else { return };
        let th = self.th;
        let mut go = false;
        let mut cancel = ctx.input(|i| i.key_pressed(Key::Escape));
        egui::Window::new("Foldex deinstallieren?")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label("Foldex wird beendet und entfernt – mit allen Einstellungen, KI-Modellen, Cloud-Anmeldungen, dem Suchindex und den Verknüpfungen.");
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Das lässt sich nicht rückgängig machen. Deine eigenen Dateien und deine Cloud-Daten bleiben erhalten.").color(th.danger));
                ui.add_space(8.0);
                if uninstall::installed() {
                    ui.label(egui::RichText::new("Der Programmordner wird ebenfalls gelöscht.").color(th.sub));
                } else {
                    ui.checkbox(&mut del_exe, format!("Auch die Programmdatei löschen ({})", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("Ja, alles entfernen").color(Color32::WHITE).strong()).fill(th.danger).min_size(vec2(0.0, 32.0))).clicked() {
                        go = true;
                    }
                    if ui.add(egui::Button::new("Abbrechen").min_size(vec2(0.0, 32.0))).clicked() {
                        cancel = true;
                    }
                });
            });
        self.uninstall_ask = Some(del_exe);
        if cancel {
            self.uninstall_ask = None;
        } else if go {
            match uninstall::run(del_exe) {
                // sofort beenden: nichts mehr speichern (sonst entstünden die Ordner neu)
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    self.status = format!("Deinstallieren fehlgeschlagen: {e}");
                    self.uninstall_ask = None;
                }
            }
        }
    }

    // ───────────────────────── Cloud live ─────────────────────────

    /// Angezeigte Cloud-Ordner alle 3 s im Hintergrund abgleichen: Änderungen von außen (Handy, Web, anderer
    /// PC) erscheinen von selbst. Google Drive & Co. melden Änderungen an rclone (alle 10 s); bei iCloud holt
    /// Foldex den sichtbaren Ordner alle 10 s selbst neu.
    fn cloud_live(&mut self) {
        if self.cloud_watch.elapsed() < Duration::from_secs(3) || self.cloud_watch_busy.load(Relaxed) {
            return;
        }
        self.cloud_watch = Instant::now();
        let mut dirs: Vec<PathBuf> = Vec::new();
        for t in &self.tabs {
            for p in &t.panes {
                let calm = p.extra.is_none() && !p.start && p.ctrash.is_none() && !p.trash && p.search.is_none() && p.rename.is_none() && !p.is_loading();
                if calm && fsx::slow(&p.cwd) && !dirs.contains(&p.cwd) {
                    dirs.push(p.cwd.clone());
                }
            }
        }
        if dirs.is_empty() {
            return;
        }
        self.ctx.request_repaint_after(Duration::from_secs(3));
        let refresh: Vec<PathBuf> = dirs.iter().filter(|d| self.cloud_refresh_at.get(*d).is_none_or(|t| t.elapsed() > Duration::from_secs(10))).cloned().collect();
        for d in &refresh {
            self.cloud_refresh_at.insert(d.clone(), Instant::now());
        }
        let busy = self.cloud_watch_busy.clone();
        busy.store(true, Relaxed);
        let tx = self.defer_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            for d in &refresh {
                cloudlogin::refresh_dir(d);
            }
            let sigs: Vec<(PathBuf, Option<u64>)> = dirs.into_iter().map(|d| {
                let s = read_sig(&d);
                (d, s)
            }).collect();
            busy.store(false, Relaxed);
            let _ = tx.send(Box::new(move |app: &mut App| {
                let ctx = app.ctx.clone();
                let mut changed = false;
                for t in &mut app.tabs {
                    for p in &mut t.panes {
                        let Some((_, Some(sig))) = sigs.iter().find(|(d, _)| *d == p.cwd) else { continue };
                        if p.is_loading() || p.rename.is_some() || p.search.is_some() || p.start || p.ctrash.is_some() {
                            continue;
                        }
                        let now = listing_sig(p.items.iter().map(|i| (&*i.name, i.dir, i.size, i.mtime)));
                        if now != *sig {
                            p.start_load(&ctx);
                            p.preview.invalidate();
                            changed = true;
                        }
                    }
                }
                if changed {
                    cloudlogin::space_soon();
                }
            }));
            ctx.request_repaint();
        });
    }

    // ───────────────────────── Startseite ─────────────────────────

    /// „Start“ oben in der Seitenleiste
    fn side_start_row(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let th = self.th;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SIDE_ROW_H), Sense::click());
        let inner = rect.shrink2(vec2(6.0, 1.0));
        let cur = self.pane().start;
        if cur {
            ui.painter().rect_filled(inner, 6.0, th.sel);
        } else if resp.hovered() {
            ui.painter().rect_filled(inner, 6.0, th.hover);
        }
        let ir = Rect::from_min_size(pos2(inner.left() + 12.0, rect.center().y - 9.0), vec2(18.0, 18.0));
        ui::draw(ui.painter(), ir.shrink(1.0), Ic::Home, if cur { th.accent } else { th.text });
        ui.painter().text(pos2(ir.right() + 10.0, rect.center().y), Align2::LEFT_CENTER, "Start", FontId::proportional(14.0), th.text);
        if resp.clicked() {
            acts.push(Act::StartHere);
        }
        if resp.middle_clicked() {
            acts.push(Act::StartOpen);
        }
        let mut off = false;
        resp.on_hover_text("Startseite").context_menu(|ui| {
            if menu_item(ui, "Öffnen", "", true) {
                acts.push(Act::StartHere);
            }
            if menu_item(ui, "In neuem Tab öffnen", "", true) {
                acts.push(Act::StartOpen);
            }
            ui::menu_sep(ui);
            if menu_item(ui, "Aus der Seitenleiste entfernen", "", true) {
                off = true;
            }
        });
        if off {
            self.start_cfg.sidebar_entry = false;
            self.start_cfg.save();
            self.status = "„Start“ ausgeblendet – zurückholen: Startseite → Zahnrad → Verhalten".into();
        }
    }

    /// Hintergrund der Startseite (schlicht, Verlauf, Bild); true = Bild liegt dahinter
    fn paint_start_bg(&mut self, ui: &egui::Ui, r: Rect) -> bool {
        let th = self.th;
        ui.painter().rect_filled(r, 0.0, th.bg);
        match self.start_cfg.background {
            1 => {
                let mut m = egui::Mesh::default();
                let a = th.accent.gamma_multiply(0.28);
                let b = th.accent.gamma_multiply(0.06);
                m.colored_vertex(r.left_top(), a);
                m.colored_vertex(r.right_top(), b);
                m.colored_vertex(r.right_bottom(), Color32::TRANSPARENT);
                m.colored_vertex(r.left_bottom(), b);
                m.add_triangle(0, 1, 2);
                m.add_triangle(0, 2, 3);
                ui.painter().add(egui::Shape::mesh(m));
                false
            }
            2 => {
                let Some(want) = self.start_cfg.bg_image.clone() else { return false };
                if self.start_bg.as_ref().map(|(p, _)| p) != Some(&want) {
                    match image::open(&want) {
                        Ok(img) => {
                            let img = img.thumbnail(2560, 2560).to_rgba8();
                            let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                            self.start_bg = Some((want, ui.ctx().load_texture("start-bg", ci, egui::TextureOptions::LINEAR)));
                        }
                        Err(e) => {
                            self.status = format!("Hintergrundbild lässt sich nicht laden: {e}");
                            self.start_cfg.background = 0;
                            self.start_cfg.save();
                            return false;
                        }
                    }
                }
                let Some((_, tex)) = &self.start_bg else { return false };
                // „cover“: Fläche füllen, Seitenverhältnis bleibt, Überstand wird abgeschnitten
                let [w, h] = tex.size().map(|v| v as f32);
                let s = (r.width() / w).max(r.height() / h);
                let (uw, uh) = (r.width() / (w * s), r.height() / (h * s));
                let uv = Rect::from_min_size(pos2((1.0 - uw) / 2.0, (1.0 - uh) / 2.0), vec2(uw, uh));
                ui.painter().image(tex.id(), r, uv, Color32::WHITE);
                ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha((self.start_cfg.bg_dim * 255.0) as u8));
                true
            }
            _ => false,
        }
    }

    /// Startseite: Begrüßung, Suche, Kachel-Bereiche – Zahnrad öffnet die Einstellungen im selben Tab
    fn ui_start(&mut self, ui: &mut egui::Ui, pn: &mut Pane, acts: &mut Vec<Act>) {
        let th = self.th;
        let full = ui.available_rect_before_wrap();
        let on_img = self.paint_start_bg(ui, full);
        let tc = if on_img { Color32::WHITE } else { th.text };
        let sc = if on_img { Color32::from_white_alpha(210) } else { th.sub };
        let mut changed = false;
        egui::ScrollArea::vertical().id_salt("startseite").auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::none().inner_margin(Margin { left: 32.0, right: 32.0, top: 24.0, bottom: 32.0 }).show(ui, |ui| {
                // Kopf als Karte: Begrüßung, Datum, große Uhr, Suche – leicht in der Akzentfarbe getönt
                let card_fill = if on_img { Color32::from_black_alpha(120) } else { mix(th.panel, th.accent, if th.dark { 0.10 } else { 0.07 }) };
                egui::Frame::none()
                    .fill(card_fill)
                    .rounding(18.0)
                    .stroke(Stroke::new(1.0, if on_img { Color32::from_white_alpha(28) } else { mix(th.line, th.accent, 0.25) }))
                    .inner_margin(Margin { left: 26.0, right: 22.0, top: 22.0, bottom: 22.0 })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let now = chrono::Local::now();
                        ui.horizontal(|ui| {
                            let (g, mut line) = startpage::header(&self.start_cfg, &now);
                            // große Uhr rechts → Uhrzeit nicht noch einmal in der Datumszeile
                            let big_clock = self.start_cfg.clock && ui.available_width() > 560.0;
                            if big_clock && self.start_cfg.date {
                                if let Some((d, _)) = line.rsplit_once(" · ") {
                                    line = d.to_string();
                                }
                            } else if big_clock {
                                line.clear();
                            }
                            ui.vertical(|ui| {
                                let title = if self.start_cfg.greeting { g } else { "Start".to_string() };
                                ui.label(egui::RichText::new(title).size(28.0).strong().color(tc));
                                if !line.is_empty() {
                                    ui.add_space(2.0);
                                    ui.label(egui::RichText::new(line).size(13.5).color(sc));
                                }
                            });
                            if self.start_cfg.clock {
                                ui.ctx().request_repaint_after(Duration::from_secs(20));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if Self::icon_btn(ui, th, Ic::Gear, on_img).on_hover_text("Startseite anpassen").clicked() {
                                    pn.start_settings = true;
                                }
                                if big_clock {
                                    use chrono::Timelike;
                                    ui.add_space(14.0);
                                    ui.label(egui::RichText::new(format!("{:02}:{:02}", now.hour(), now.minute())).size(38.0).color(if on_img { Color32::WHITE } else { mix(th.text, th.accent, 0.35) }));
                                }
                            });
                        });
                        if self.start_cfg.search {
                            ui.add_space(16.0);
                            // Suchfeld mit Lupe
                            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::hover());
                            ui.painter().rect(rect, 12.0, th.field, Stroke::new(1.0, th.line));
                            let c = pos2(rect.left() + 20.0, rect.center().y - 1.0);
                            ui.painter().circle_stroke(c, 6.0, Stroke::new(1.6, th.sub));
                            ui.painter().line_segment([c + vec2(4.3, 4.3), c + vec2(9.0, 9.0)], Stroke::new(1.6, th.sub));
                            let te = egui::TextEdit::singleline(&mut self.start_query)
                                .hint_text("Ganzen PC durchsuchen …")
                                .frame(false)
                                .font(FontId::proportional(15.0))
                                .margin(Margin { left: 40.0, right: 12.0, top: 11.0, bottom: 11.0 });
                            let r = ui.put(rect, te);
                            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !self.start_query.trim().is_empty() {
                                acts.push(Act::StartSearch(self.start_query.trim().to_string()));
                            }
                        }
                    });
                let secs = self.start_cfg.sections.clone();
                let mut first = true;
                for sec in secs.iter().filter(|s| s.on) {
                    let tiles = self.start_tiles(&sec.key);
                    if tiles.is_empty() && sec.key != "links" {
                        continue;
                    }
                    ui.add_space(if first { 24.0 } else { 20.0 });
                    first = false;
                    if self.start_cfg.headings {
                        ui.label(egui::RichText::new(startpage::title(&sec.key)).size(13.0).strong().color(sc));
                        ui.add_space(8.0);
                    }
                    changed |= self.start_grid(ui, &sec.key, &tiles, on_img, acts);
                }
            });
        });
        if changed {
            self.start_cfg.save();
        }
    }

    /// runder Knopf mit Symbol (Zahnrad, Zurück)
    fn icon_btn(ui: &mut egui::Ui, th: Theme, ic: Ic, on_img: bool) -> egui::Response {
        let (r, resp) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::click());
        let fill = if resp.hovered() { th.hover } else if on_img { Color32::from_black_alpha(90) } else { th.panel };
        ui.painter().circle(r.center(), 18.0, fill, Stroke::new(1.0, if resp.hovered() { th.accent } else { th.line }));
        ui::draw(ui.painter(), r.shrink(10.0), ic, if on_img { Color32::WHITE } else { th.text });
        resp
    }

    /// Kacheln eines Bereichs
    fn start_tiles(&self, key: &str) -> Vec<Tile> {
        let cfg = &self.start_cfg;
        let shown = |p: &Path| !cfg.hidden.iter().any(|h| h == p);
        let mut v = Vec::new();
        match key {
            "pins" => {
                for e in self.side.pins.iter().filter(|e| shown(&e.path)) {
                    v.push(Tile { path: e.path.clone(), name: e.label.clone(), sub: None, space: None, icon: quick_icon(&e.path).map_or(TileIcon::Folder, TileIcon::Quick), cloud: None, link: None });
                }
            }
            "cloud" => {
                for c in cloud::places().into_iter().filter(|c| shown(&c.path)) {
                    let space = match c.remote.as_deref() {
                        Some(r) => cloudlogin::space(r),
                        None => cloudlogin::accounts().into_iter().find(|a| a.key == c.key && !a.off).and_then(|a| cloudlogin::space(&a.remote)),
                    };
                    v.push(Tile { sub: if space.is_none() { Some(c.path.display().to_string()) } else { None }, path: c.path, name: c.name, space, icon: TileIcon::Cloud(c.key), cloud: Some((c.key, c.remote)), link: None });
                }
            }
            "pc" => {
                for d in self.drives.iter().filter(|d| shown(&d.path) && !trashx::on_cloud(&d.path)) {
                    // „Lokaler Datenträger (C:)“ passt nicht auf eine Kachel → „Laufwerk C:“
                    let name = match d.label.rsplit_once('(') {
                        Some((_, l)) if d.label.starts_with("Lokaler Datenträger") => format!("Laufwerk {}", l.trim_end_matches(')')),
                        _ => d.label.clone(),
                    };
                    v.push(Tile { path: d.path.clone(), name, sub: None, space: self.drive_space.get(&d.path).copied(), icon: TileIcon::Drive, cloud: None, link: None });
                }
            }
            "recent" => {
                for p in self.recent.dirs.iter().filter(|p| shown(p)).take(cfg.recent_max) {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                    v.push(Tile { sub: p.parent().map(|x| x.display().to_string()), path: p.clone(), name, space: None, icon: quick_icon(p).map_or(TileIcon::Folder, TileIcon::Quick), cloud: None, link: None });
                }
            }
            "links" => {
                for (i, l) in cfg.links.iter().enumerate() {
                    v.push(Tile { path: l.path.clone(), name: l.label.clone(), sub: Some(l.path.display().to_string()), space: None, icon: TileIcon::Folder, cloud: None, link: Some(i) });
                }
            }
            _ => {}
        }
        v
    }

    /// Kacheln eines Bereichs als Raster bzw. Liste; true = Einstellungen geändert
    fn start_grid(&mut self, ui: &mut egui::Ui, key: &str, tiles: &[Tile], on_img: bool, acts: &mut Vec<Act>) -> bool {
        let th = self.th;
        let cfg = &self.start_cfg;
        let (tw, h) = cfg.tile_size();
        let big = cfg.tile as usize;
        let list = cfg.style == 1;
        let gap = if list { 8.0 } else { 10.0 };
        let avail = ui.available_width();
        let cols = if cfg.columns > 0 { cfg.columns as usize } else { ((avail + gap) / (tw + gap)).floor().max(1.0) as usize };
        let w = (avail - gap * (cols - 1) as f32) / cols as f32;
        let add = key == "links";
        let n = tiles.len() + add as usize;
        let rows = n.div_ceil(cols).max(1);
        let (area, _) = ui.allocate_exact_size(vec2(avail, rows as f32 * (h + gap) - gap), Sense::hover());
        let cell = |i: usize| Rect::from_min_size(area.min + vec2((i % cols) as f32 * (w + gap), (i / cols) as f32 * (h + gap)), vec2(w, h));
        let (show_space, colorful) = (cfg.space, cfg.colorful);
        let isz = if list { [18.0, 22.0, 28.0][big] } else { [20.0, 26.0, 34.0][big] };
        let round = if list { 8.0 } else { 12.0 };
        let base = if on_img { Color32::from_rgba_unmultiplied(th.panel.r(), th.panel.g(), th.panel.b(), 225) } else { th.panel };
        let mut changed = false;
        for (i, t) in tiles.iter().enumerate() {
            let r = cell(i);
            let resp = ui.interact(r, ui.id().with(("kachel", key, i)), Sense::click());
            let hov = resp.hovered();
            // beim Darüberfahren leicht „anheben“: weicher Schatten unter der Kachel
            if hov {
                let sh = egui::epaint::Shadow { offset: vec2(0.0, 4.0), blur: 16.0, spread: 0.0, color: Color32::from_black_alpha(if th.dark { 110 } else { 40 }) };
                ui.painter().add(sh.as_shape(r, round));
            }
            ui.painter().rect_filled(r, round, if hov { th.hover } else { base });
            if colorful {
                ui.painter().rect_filled(r, round, th.accent.gamma_multiply(if hov { 0.2 } else { 0.1 }));
            }
            ui.painter().rect_stroke(r, round, Stroke::new(1.0, if hov { th.accent } else { th.line }));
            // Symbol auf farbigem Plättchen
            let bsz = isz + if list { 10.0 } else { 16.0 };
            let badge = Rect::from_center_size(pos2(r.left() + 12.0 + bsz / 2.0, r.center().y), vec2(bsz, bsz));
            let tint = match &t.icon {
                TileIcon::Quick(ic) => quick_color(*ic, th.accent),
                TileIcon::Cloud(_) => Color32::from_rgb(120, 160, 230),
                TileIcon::Drive => th.sub,
                TileIcon::Folder => Color32::from_rgb(240, 190, 70),
            };
            ui.painter().rect_filled(badge, if list { 7.0 } else { 10.0 }, tint.gamma_multiply(if th.dark { 0.18 } else { 0.14 }));
            let ir = Rect::from_center_size(badge.center(), vec2(isz, isz));
            match (&t.icon, foldericons::get(&t.path)) {
                (_, Some(spec)) if self.icons.paint_spec(ui.painter(), ir, &spec) => {}
                (TileIcon::Quick(ic), _) => ui::draw(ui.painter(), ir.shrink(isz * 0.08), *ic, tint),
                (TileIcon::Cloud(k), _) => cloud::logo(ui.painter(), ir, k),
                (TileIcon::Drive, _) => self.icons.paint(ui.painter(), ir, IconKind::Drive, &t.name, "", true, || t.path.clone()),
                (TileIcon::Folder, _) => self.icons.paint(ui.painter(), ir, IconKind::Dir, &t.name, "", false, || t.path.clone()),
            }
            let tx = badge.right() + 12.0;
            let fs = [13.5, 14.5, 16.0][big];
            let space = t.space.filter(|_| show_space);
            // Belegung in Prozent rechts neben dem Namen
            let pct = space.filter(|(_, tot)| *tot > 0).map(|(f, tot)| format!("{} %", ((tot.saturating_sub(f)) as f64 / tot as f64 * 100.0).round() as u32));
            let pw = pct.as_ref().map_or(0.0, |p| ui.painter().layout_no_wrap(p.clone(), FontId::proportional(11.5), th.sub).size().x + 8.0);
            let tmax = r.right() - 14.0 - tx;
            if list {
                // Liste: Name links, Zusatz (Speicher bzw. Ort) rechts daneben
                let extra = space.map(|(f, tot)| format!("{} frei von {}", human(f), human(tot))).or_else(|| t.sub.clone().filter(|_| big > 0));
                let ew = if extra.is_some() { (tmax * 0.45).min(220.0) } else { 0.0 };
                text_trunc(ui.painter(), pos2(tx, r.center().y), &t.name, FontId::proportional(fs), th.text, tmax - ew - 8.0);
                if let Some(e) = extra {
                    let galley = ui.painter().layout_no_wrap(e.clone(), FontId::proportional(11.5), th.sub);
                    let x = (r.right() - 12.0 - galley.size().x).max(r.right() - 12.0 - ew);
                    text_trunc(ui.painter(), pos2(x, r.center().y), &e, FontId::proportional(11.5), th.sub, ew);
                }
            } else {
                let has_sub = space.is_some() || (t.sub.is_some() && big > 0);
                let ny = if has_sub { r.center().y - if big == 2 { 14.0 } else { 9.0 } } else { r.center().y };
                text_trunc(ui.painter(), pos2(tx, ny), &t.name, FontId::proportional(fs), th.text, tmax - pw);
                if let Some(p) = &pct {
                    ui.painter().text(pos2(r.right() - 14.0, ny), Align2::RIGHT_CENTER, p, FontId::proportional(11.5), th.sub);
                }
                if let Some((free, total)) = space {
                    let frac = if total > 0 { total.saturating_sub(free) as f32 / total as f32 } else { 0.0 };
                    let by = ny + if big == 2 { 16.0 } else { 11.0 };
                    let bar = Rect::from_min_size(pos2(tx, by), vec2(tmax.max(20.0), 6.0));
                    ui.painter().rect_filled(bar, 3.0, th.line);
                    ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2((bar.width() * frac.clamp(0.0, 1.0)).max(6.0), 6.0)), 3.0, if frac > 0.9 { Color32::from_rgb(220, 72, 62) } else { th.accent });
                    if big > 0 {
                        text_trunc(ui.painter(), pos2(tx, bar.bottom() + 8.0), &format!("{} frei von {}", human(free), human(total)), FontId::proportional(11.0), th.sub, tmax);
                    }
                } else if let (Some(sub), true) = (&t.sub, big > 0) {
                    text_trunc(ui.painter(), pos2(tx, ny + if big == 2 { 20.0 } else { 17.0 }), sub, FontId::proportional(11.0), th.sub, tmax);
                }
            }
            let resp = resp.on_hover_text(t.path.display().to_string());
            if resp.clicked() {
                acts.push(Act::StartGo(t.path.clone()));
            }
            if resp.middle_clicked() {
                acts.push(Act::TabOpen(t.path.clone()));
            }
            let mut hide = false;
            resp.context_menu(|ui| {
                if menu_item(ui, "Öffnen", "", true) {
                    acts.push(Act::StartGo(t.path.clone()));
                }
                if menu_item(ui, "In neuem Tab öffnen", "", true) {
                    acts.push(Act::TabOpen(t.path.clone()));
                }
                if let Some((k, rem)) = &t.cloud {
                    if (rem.is_some() && cloudtrash::in_app(k)) || cloudtrash::web_url(k).is_some() {
                        if menu_item(ui, "Papierkorb anzeigen", "", true) {
                            acts.push(Act::CloudTrash(k, rem.clone(), t.name.clone()));
                        }
                    }
                }
                ui::menu_sep(ui);
                if menu_item(ui, if t.link.is_some() { "Link entfernen" } else { "Auf der Startseite ausblenden" }, "", true) {
                    hide = true;
                }
            });
            if hide {
                match t.link {
                    Some(li) if li < self.start_cfg.links.len() => {
                        self.start_cfg.links.remove(li);
                    }
                    _ => self.start_cfg.hidden.push(t.path.clone()),
                }
                changed = true;
            }
        }
        if add {
            let r = cell(tiles.len());
            let resp = ui.interact(r, ui.id().with(("kachel-neu", key)), Sense::click());
            let col = if resp.hovered() { th.accent } else if on_img { Color32::from_white_alpha(200) } else { th.sub };
            ui.painter().rect_stroke(r, round, Stroke::new(1.0, col));
            let ir = Rect::from_center_size(pos2(r.left() + 14.0 + isz / 2.0, r.center().y), vec2(isz * 0.7, isz * 0.7));
            ui::draw(ui.painter(), ir, Ic::Plus, col);
            ui.painter().text(pos2(ir.right() + 14.0, r.center().y), Align2::LEFT_CENTER, "Ordner hinzufügen …", FontId::proportional(13.5), col);
            if resp.on_hover_text("Beliebigen Ordner auf die Startseite legen").clicked() {
                changed |= self.start_add_link();
            }
        }
        changed
    }

    /// Ordner wählen und als eigenen Link merken
    fn start_add_link(&mut self) -> bool {
        let Some(d) = rfd::FileDialog::new().set_title("Ordner für die Startseite").pick_folder() else { return false };
        let label = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| d.display().to_string());
        self.start_cfg.links.push(startpage::Link { label, path: d });
        true
    }

    // ── Bausteine der Einstellungsseite ──

    /// Ein/Aus-Schalter; true = umgeschaltet
    fn switch(ui: &mut egui::Ui, th: Theme, on: &mut bool) -> bool {
        let (r, resp) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
        if resp.clicked() {
            *on = !*on;
        }
        let t = ui.ctx().animate_bool(resp.id, *on);
        let bg = if *on { th.accent } else if resp.hovered() { th.sub } else { th.line };
        ui.painter().rect_filled(r, 11.0, bg);
        let x = egui::lerp(r.left() + 11.0..=r.right() - 11.0, t);
        ui.painter().circle_filled(pos2(x, r.center().y), 8.0, Color32::WHITE);
        resp.clicked()
    }

    /// Auswahl aus mehreren Möglichkeiten (Segmente); true = geändert
    fn segmented(ui: &mut egui::Ui, th: Theme, id: &str, sel: &mut u8, opts: &[&str]) -> bool {
        let font = FontId::proportional(13.0);
        let widths: Vec<f32> = opts.iter().map(|o| ui.painter().layout_no_wrap(o.to_string(), font.clone(), th.text).size().x + 26.0).collect();
        let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 6.0, 32.0), Sense::hover());
        ui.painter().rect(rect, 9.0, th.field, Stroke::new(1.0, th.line));
        let mut x = rect.left() + 3.0;
        let mut changed = false;
        for (i, (o, w)) in opts.iter().zip(&widths).enumerate() {
            let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(*w, 26.0));
            let resp = ui.interact(r, ui.id().with(("segment", id, i)), Sense::click());
            let on = *sel as usize == i;
            if on {
                ui.painter().rect_filled(r, 7.0, th.accent);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 7.0, th.hover);
            }
            ui.painter().text(r.center(), Align2::CENTER_CENTER, *o, font.clone(), if on { th.on_accent() } else { th.text });
            if resp.clicked() && !on {
                *sel = i as u8;
                changed = true;
            }
            x += w;
        }
        changed
    }

    /// kleiner Pfeil-Knopf (nach oben / unten); true = geklickt
    fn chevron(ui: &mut egui::Ui, th: Theme, up: bool, enabled: bool) -> bool {
        let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), if enabled { Sense::click() } else { Sense::hover() });
        if enabled && resp.hovered() {
            ui.painter().rect_filled(r, 7.0, th.hover);
        }
        let c = r.center();
        let d = if up { -1.0 } else { 1.0 };
        let col = if enabled { th.text } else { th.line };
        ui.painter().add(egui::Shape::line(vec![c + vec2(-5.0, -2.5 * d), c + vec2(0.0, 2.5 * d), c + vec2(5.0, -2.5 * d)], Stroke::new(1.8, col)));
        enabled && resp.on_hover_text(if up { "Nach oben" } else { "Nach unten" }).clicked()
    }

    /// Karte mit Überschrift
    fn card(ui: &mut egui::Ui, th: Theme, title: &str, add: impl FnOnce(&mut egui::Ui)) {
        ui.add_space(22.0);
        ui.label(egui::RichText::new(title).size(13.0).strong().color(th.sub));
        ui.add_space(8.0);
        egui::Frame::none().fill(th.panel).rounding(14.0).stroke(Stroke::new(1.0, th.line)).inner_margin(Margin::symmetric(18.0, 6.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    }

    /// Zeile einer Karte: Titel (+ Erklärung) links, Bedienelement rechts
    fn row(ui: &mut egui::Ui, th: Theme, first: bool, title: &str, desc: &str, control: impl FnOnce(&mut egui::Ui)) {
        if !first {
            let r = ui.available_rect_before_wrap();
            ui.painter().hline(r.left()..=r.right(), r.top(), Stroke::new(1.0, th.line));
        }
        ui.horizontal(|ui| {
            ui.set_min_height(if desc.is_empty() { 46.0 } else { 56.0 });
            ui.vertical(|ui| {
                ui.add_space(if desc.is_empty() { 13.0 } else { 9.0 });
                ui.label(egui::RichText::new(title).size(14.0).color(th.text));
                if !desc.is_empty() {
                    ui.label(egui::RichText::new(desc).size(11.5).color(th.sub));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
        });
    }

    /// Einstellungen der Startseite – im selben Tab, statt der Kacheln
    fn ui_start_settings(&mut self, ui: &mut egui::Ui, pn: &mut Pane) {
        let th = self.th;
        let full = ui.available_rect_before_wrap();
        ui.painter().rect_filled(full, 0.0, th.bg);
        let mut ch = false;
        let mut pick_bg = false;
        let mut add_link = false;
        egui::ScrollArea::vertical().id_salt("start-einstellungen").auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::none().inner_margin(Margin { left: 32.0, right: 32.0, top: 26.0, bottom: 40.0 }).show(ui, |ui| {
                ui.set_max_width(780.0);
                ui.horizontal(|ui| {
                    if Self::icon_btn(ui, th, Ic::Back, false).on_hover_text("Zurück zur Startseite").clicked() {
                        pn.start_settings = false;
                    }
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Startseite anpassen").size(24.0).strong());
                        ui.label(egui::RichText::new("Änderungen gelten sofort").size(12.0).color(th.sub));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Alles zurücksetzen").min_size(vec2(0.0, 32.0))).clicked() {
                            self.start_cfg = startpage::StartCfg::default();
                            ch = true;
                        }
                    });
                });
                let c = &mut self.start_cfg;

                Self::card(ui, th, "Verhalten", |ui| {
                    Self::row(ui, th, true, "Neuer Tab öffnet", "Strg+T und das +", |ui| ch |= Self::segmented(ui, th, "newtab", &mut c.new_tab, &["Benutzerordner", "Startseite", "Aktuellen Ordner"]));
                    Self::row(ui, th, false, "Letzten Tab schließen", "Was danach angezeigt wird", |ui| ch |= Self::segmented(ui, th, "closelast", &mut c.close_last, &["Startseite", "Benutzerordner"]));
                    Self::row(ui, th, false, "Mit der Startseite beginnen", "Beim Öffnen von Foldex", |ui| ch |= Self::switch(ui, th, &mut c.on_launch));
                    Self::row(ui, th, false, "„Start“ in der Seitenleiste", "Ganz oben, über dem Schnellzugriff", |ui| ch |= Self::switch(ui, th, &mut c.sidebar_entry));
                });

                Self::card(ui, th, "Kopfbereich", |ui| {
                    Self::row(ui, th, true, "Begrüßung", "„Guten Morgen“, „Guten Tag“ …", |ui| ch |= Self::switch(ui, th, &mut c.greeting));
                    Self::row(ui, th, false, "Dein Name", "Erscheint in der Begrüßung", |ui| {
                        ch |= ui.add(egui::TextEdit::singleline(&mut c.name).hint_text("z. B. Jesper").desired_width(200.0)).changed();
                    });
                    Self::row(ui, th, false, "Datum", "", |ui| ch |= Self::switch(ui, th, &mut c.date));
                    Self::row(ui, th, false, "Uhrzeit", "", |ui| ch |= Self::switch(ui, th, &mut c.clock));
                    Self::row(ui, th, false, "Suchfeld", "Durchsucht den ganzen PC", |ui| ch |= Self::switch(ui, th, &mut c.search));
                });

                Self::card(ui, th, "Aussehen", |ui| {
                    Self::row(ui, th, true, "Darstellung", "", |ui| ch |= Self::segmented(ui, th, "style", &mut c.style, &["Kacheln", "Liste"]));
                    Self::row(ui, th, false, "Größe", "", |ui| ch |= Self::segmented(ui, th, "size", &mut c.tile, &["Klein", "Mittel", "Groß"]));
                    Self::row(ui, th, false, "Spalten", "Automatisch passt sich der Fensterbreite an", |ui| ch |= Self::segmented(ui, th, "cols", &mut c.columns, &["Auto", "1", "2", "3", "4", "5", "6"]));
                    Self::row(ui, th, false, "Farbige Kacheln", "Leicht in der Akzentfarbe", |ui| ch |= Self::switch(ui, th, &mut c.colorful));
                    Self::row(ui, th, false, "Speicherbalken", "Bei Laufwerken und Cloud", |ui| ch |= Self::switch(ui, th, &mut c.space));
                    Self::row(ui, th, false, "Überschriften", "Über den Bereichen", |ui| ch |= Self::switch(ui, th, &mut c.headings));
                    Self::row(ui, th, false, "Hintergrund", "", |ui| ch |= Self::segmented(ui, th, "bg", &mut c.background, &["Schlicht", "Verlauf", "Bild"]));
                    if c.background == 2 {
                        let name = c.bg_image.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Kein Bild gewählt".into());
                        Self::row(ui, th, false, "Bild", &name, |ui| {
                            if ui.add(egui::Button::new("Bild wählen …").min_size(vec2(0.0, 30.0))).clicked() {
                                pick_bg = true;
                            }
                        });
                        Self::row(ui, th, false, "Abdunkeln", "Damit Schrift gut lesbar bleibt", |ui| {
                            ch |= ui.add(egui::Slider::new(&mut c.bg_dim, 0.0..=0.9).show_value(false)).changed();
                        });
                    }
                });

                Self::card(ui, th, "Bereiche", |ui| {
                    let n = c.sections.len();
                    let mut mv: Option<(usize, usize)> = None;
                    for i in 0..n {
                        let key = c.sections[i].key.clone();
                        let desc = if key == "recent" { "Anzahl rechts einstellbar" } else { "" };
                        Self::row(ui, th, i == 0, startpage::title(&key), desc, |ui| {
                            if Self::chevron(ui, th, false, i + 1 < n) {
                                mv = Some((i, i + 1));
                            }
                            if Self::chevron(ui, th, true, i > 0) {
                                mv = Some((i, i - 1));
                            }
                            ui.add_space(6.0);
                            ch |= Self::switch(ui, th, &mut c.sections[i].on);
                            if key == "recent" {
                                ui.add_space(8.0);
                                ch |= ui.add(egui::DragValue::new(&mut c.recent_max).range(1..=40).suffix(" Ordner")).changed();
                            }
                        });
                    }
                    if let Some((a, b)) = mv {
                        c.sections.swap(a, b);
                        ch = true;
                    }
                });

                Self::card(ui, th, "Eigene Links", |ui| {
                    let mut remove = None;
                    for (i, l) in c.links.iter_mut().enumerate() {
                        let path = l.path.display().to_string();
                        Self::row(ui, th, i == 0, "", &path, |ui| {
                            if ui.button("Entfernen").clicked() {
                                remove = Some(i);
                            }
                            ch |= ui.add(egui::TextEdit::singleline(&mut l.label).desired_width(220.0)).changed();
                        });
                    }
                    if let Some(i) = remove {
                        c.links.remove(i);
                        ch = true;
                    }
                    if c.links.is_empty() {
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new("Noch keine eigenen Links – beliebige Ordner lassen sich als Kachel ablegen.").color(th.sub));
                    }
                    ui.add_space(8.0);
                    if ui.add(egui::Button::new("+  Ordner hinzufügen …").min_size(vec2(0.0, 30.0))).clicked() {
                        add_link = true;
                    }
                    ui.add_space(8.0);
                });

                if !c.hidden.is_empty() {
                    Self::card(ui, th, "Ausgeblendete Kacheln", |ui| {
                        let mut show = None;
                        for (i, p) in c.hidden.iter().enumerate() {
                            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                            Self::row(ui, th, i == 0, &name, &p.display().to_string(), |ui| {
                                if ui.button("Einblenden").clicked() {
                                    show = Some(i);
                                }
                            });
                        }
                        if let Some(i) = show {
                            c.hidden.remove(i);
                            ch = true;
                        }
                        ui.add_space(8.0);
                        if ui.button("Alle einblenden").clicked() {
                            c.hidden.clear();
                            ch = true;
                        }
                        ui.add_space(8.0);
                    });
                }
            });
        });
        if pick_bg {
            if let Some(f) = rfd::FileDialog::new().set_title("Hintergrundbild").add_filter("Bilder", &["png", "jpg", "jpeg", "webp", "bmp", "gif"]).pick_file() {
                self.start_cfg.bg_image = Some(f);
                self.start_bg = None;
                ch = true;
            }
        }
        if add_link {
            ch |= self.start_add_link();
        }
        if ch {
            self.start_cfg.save();
        }
    }

    // ───────────────────────── Papierkorb eines Cloud-Kontos (eigener Tab) ─────────────────────────

    /// Ergebnis an den Papierkorb-Tab dieses Kontos geben
    fn with_trash(app: &mut App, remote: &str, f: impl Fn(&mut cloudtrash::TrashView)) {
        each_pane(&mut app.tabs, |p| {
            if let Some(v) = p.ctrash.as_deref_mut().filter(|v| v.remote == remote) {
                f(v);
            }
        });
    }

    /// Papierkorb neu laden (im Hintergrund)
    fn ctrash_load(&mut self, remote: &str) {
        Self::with_trash(self, remote, |v| v.busy = true);
        let r = remote.to_string();
        let tx = self.defer_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let res = cloudtrash::list(&r);
            let _ = tx.send(Box::new(move |app: &mut App| {
                Self::with_trash(app, &r, |v| {
                    v.items = Some(res.clone());
                    v.busy = false;
                    v.sel.clear();
                });
            }));
            ctx.request_repaint();
        });
    }

    /// Wiederherstellen (0), endgültig löschen (1) oder leeren (2), danach neu laden
    fn ctrash_run(&mut self, remote: String, what: u8, ids: Vec<String>) {
        Self::with_trash(self, &remote, |v| {
            v.busy = true;
            v.confirm = 0;
        });
        let tx = self.defer_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let msg = match what {
                0 => cloudtrash::restore(&remote, &ids).map(|n| format!("{n} Element(e) wiederhergestellt")),
                1 => cloudtrash::delete_forever(&remote, &ids).map(|n| format!("{n} Element(e) endgültig gelöscht")),
                _ => cloudtrash::empty(&remote).map(|()| "Papierkorb geleert".to_string()),
            }
            .unwrap_or_else(|e| e);
            let _ = tx.send(Box::new(move |app: &mut App| {
                Self::with_trash(app, &remote, |v| v.msg = msg.clone());
                app.ctrash_load(&remote);
                cloudlogin::space_soon();
            }));
            ctx.request_repaint();
        });
    }

    /// Papierkorb-Tab: Kopf, Aktionen, Liste
    fn ui_trash_page(&mut self, ui: &mut egui::Ui, v: &mut cloudtrash::TrashView) {
        let th = self.th;
        let full = ui.available_rect_before_wrap();
        ui.painter().rect_filled(full, 0.0, th.bg);
        let mut run: Option<u8> = None;
        let mut reload = false;
        let n = v.sel.len();
        let items: Vec<cloudtrash::Item> = match &v.items {
            Some(Ok(x)) => x.clone(),
            _ => Vec::new(),
        };
        egui::Frame::none().inner_margin(Margin { left: 28.0, right: 28.0, top: 22.0, bottom: 12.0 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                cloud::logo(ui.painter(), ir, "gdrive");
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Papierkorb").size(24.0).strong());
                    let sub = match &v.items {
                        Some(Ok(x)) => format!("{} · {} Element(e)", v.name, x.len()),
                        _ => v.name.clone(),
                    };
                    ui.label(egui::RichText::new(sub).size(12.5).color(th.sub));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!items.is_empty() && !v.busy, egui::Button::new(egui::RichText::new("Papierkorb leeren").color(th.danger)).min_size(vec2(0.0, 32.0))).clicked() {
                        v.confirm = 2;
                    }
                    if ui.add(egui::Button::new("Im Browser").min_size(vec2(0.0, 32.0))).on_hover_text("drive.google.com öffnen").clicked() {
                        let _ = open::that_detached("https://drive.google.com/drive/trash");
                    }
                    if ui.add_enabled(!v.busy, egui::Button::new("Aktualisieren").min_size(vec2(0.0, 32.0))).clicked() {
                        reload = true;
                    }
                    if v.busy {
                        ui.spinner();
                    }
                });
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let restore = egui::Button::new(egui::RichText::new(format!("Wiederherstellen ({n})")).color(if n > 0 { Color32::WHITE } else { th.sub })).fill(if n > 0 { th.accent } else { th.field }).min_size(vec2(0.0, 32.0));
                if ui.add_enabled(n > 0 && !v.busy, restore).clicked() {
                    run = Some(0);
                }
                if ui.add_enabled(n > 0 && !v.busy, egui::Button::new(egui::RichText::new(format!("Endgültig löschen ({n})")).color(th.danger)).min_size(vec2(0.0, 32.0))).clicked() {
                    v.confirm = 1;
                }
                if !v.msg.is_empty() {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(&v.msg).size(12.5).color(th.sub));
                }
            });
            if v.confirm > 0 {
                ui.add_space(10.0);
                egui::Frame::none().fill(th.panel).rounding(10.0).stroke(Stroke::new(1.0, th.danger)).inner_margin(Margin::same(12.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let t = if v.confirm == 1 { format!("{n} Element(e) endgültig löschen?") } else { "Den ganzen Papierkorb leeren?".into() };
                    ui.label(egui::RichText::new(format!("{t} Das lässt sich nicht rückgängig machen.")).color(th.danger));
                    ui.horizontal(|ui| {
                        if ui.button(egui::RichText::new("Ja, endgültig löschen").color(th.danger)).clicked() {
                            run = Some(v.confirm);
                        }
                        if ui.button("Abbrechen").clicked() {
                            v.confirm = 0;
                        }
                    });
                });
            }
        });
        // Liste
        egui::Frame::none().inner_margin(Margin { left: 28.0, right: 28.0, top: 4.0, bottom: 16.0 }).show(ui, |ui| match &v.items {
            None => {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new("Lade Papierkorb …").color(th.sub));
                });
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e).color(th.danger));
            }
            Some(Ok(_)) if items.is_empty() => {
                ui.add_space(50.0);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                    ui::draw(ui.painter(), r, Ic::Trash, th.sub);
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Der Papierkorb ist leer").size(16.0));
                    ui.label(egui::RichText::new("Gelöschte Dateien von Google Drive erscheinen hier.").color(th.sub));
                });
            }
            Some(Ok(_)) => {
                let w = ui.available_width();
                let (hr, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
                let all = v.sel.len() == items.len();
                let cb = Rect::from_center_size(pos2(hr.left() + 14.0, hr.center().y), vec2(16.0, 16.0));
                let cresp = ui.interact(cb.expand(4.0), ui.id().with("ct-all"), Sense::click());
                Self::check(ui, th, cb, all);
                if cresp.on_hover_text(if all { "Auswahl aufheben" } else { "Alle auswählen" }).clicked() {
                    v.sel = if all { HashSet::new() } else { items.iter().map(|i| i.id.clone()).collect() };
                }
                let p = ui.painter();
                p.text(pos2(hr.left() + 66.0, hr.center().y), Align2::LEFT_CENTER, "Name", FontId::proportional(12.5), th.sub);
                p.text(pos2(hr.right() - 170.0, hr.center().y), Align2::RIGHT_CENTER, "Größe", FontId::proportional(12.5), th.sub);
                p.text(pos2(hr.right() - 10.0, hr.center().y), Align2::RIGHT_CENTER, "Gelöscht am", FontId::proportional(12.5), th.sub);
                p.hline(hr.left()..=hr.right(), hr.bottom(), Stroke::new(1.0, th.line));
                egui::ScrollArea::vertical().id_salt("ctrash-liste").auto_shrink([false, false]).show(ui, |ui| {
                    for it in &items {
                        let (r, resp) = ui.allocate_exact_size(vec2(w, 38.0), Sense::click());
                        let on = v.sel.contains(&it.id);
                        if on {
                            ui.painter().rect_filled(r, 6.0, th.sel);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, 6.0, th.hover);
                        }
                        Self::check(ui, th, Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(16.0, 16.0)), on);
                        let ir = Rect::from_center_size(pos2(r.left() + 48.0, r.center().y), vec2(20.0, 20.0));
                        self.icons.paint(ui.painter(), ir, if it.dir { IconKind::Dir } else { IconKind::File }, &it.name, ext_buf(&it.name).as_str(), false, PathBuf::new);
                        text_trunc(ui.painter(), pos2(r.left() + 66.0, r.center().y), &it.name, FontId::proportional(14.0), th.text, (r.width() - 66.0 - 300.0).max(80.0));
                        if !it.dir {
                            ui.painter().text(pos2(r.right() - 170.0, r.center().y), Align2::RIGHT_CENTER, human(it.size), FontId::proportional(12.5), th.sub);
                        }
                        ui.painter().text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, cloudtrash::fmt_time(&it.trashed), FontId::proportional(12.5), th.sub);
                        if resp.clicked() {
                            if on {
                                v.sel.remove(&it.id);
                            } else {
                                v.sel.insert(it.id.clone());
                            }
                        }
                    }
                });
            }
        });
        let remote = v.remote.clone();
        if reload {
            self.ctrash_load(&remote);
        }
        if let Some(x) = run {
            let ids = v.sel.iter().cloned().collect();
            self.ctrash_run(remote, x, ids);
        }
    }

    /// Kästchen zum Auswählen
    fn check(ui: &egui::Ui, th: Theme, r: Rect, on: bool) {
        if on {
            ui.painter().rect_filled(r, 4.0, th.accent);
            ui.painter().add(egui::Shape::line(vec![r.left_center() + vec2(3.5, 0.0), r.center_bottom() + vec2(-1.0, -4.0), r.right_top() + vec2(-3.5, 4.0)], Stroke::new(1.8, Color32::WHITE)));
        } else {
            ui.painter().rect_stroke(r, 4.0, Stroke::new(1.2, th.sub));
        }
    }

    /// „Cloud hinzufügen“ (öffnet die Liste der Anbieter)
    fn side_cloud_add(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let th = self.th;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SIDE_ROW_H), Sense::click());
        let inner = rect.shrink2(vec2(6.0, 1.0));
        if resp.hovered() {
            ui.painter().rect_filled(inner, 6.0, th.hover);
        }
        let ir = Rect::from_min_size(pos2(inner.left() + 13.0, rect.center().y - 8.0), vec2(16.0, 16.0));
        ui::draw(ui.painter(), ir, Ic::Plus, th.sub);
        ui.painter().text(pos2(ir.right() + 11.0, rect.center().y), Align2::LEFT_CENTER, "Cloud hinzufügen", FontId::proportional(14.0), th.sub);
        if resp.on_hover_text("Google Drive, iCloud, Dropbox, MEGA, Proton Drive …").clicked() {
            acts.push(Act::CloudSetup);
        }
    }

    /// Fenster: alle Anbieter, eingerichtete mit Ordner, die übrigen mit „Einrichten“
    fn ui_cloud_setup(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        if !self.cloud_setup {
            return;
        }
        let th = self.th;
        let mut open = true;
        let places = cloud::places();
        let login = cloudlogin::login_step();
        let busy = cloudlogin::login_busy();
        egui::Window::new("Cloud-Speicher").open(&mut open).collapsible(false).resizable(false).default_width(480.0).show(ctx, |ui| {
            ui.label(egui::RichText::new("„Anmelden“ genügt – kein Programm des Anbieters nötig: Foldex bindet den Speicher als Laufwerk ein (beim ersten Mal werden rclone, 25 MB, und der Windows-Treiber WinFsp, 2 MB, geladen). Wer das Programm des Anbieters schon nutzt, findet dessen Ordner hier ebenfalls.").size(12.0).color(th.sub));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height((ctx.screen_rect().height() - 180.0).max(240.0)).auto_shrink([false, true]).show(ui, |ui| {
                for pr in cloud::PROVIDERS {
                    let mine: Vec<&cloud::Place> = places.iter().filter(|p| p.key == pr.key).collect();
                    let off: Vec<cloudlogin::Account> = cloudlogin::accounts().into_iter().filter(|a| a.key == pr.key && !mine.iter().any(|p| p.remote.as_deref() == Some(a.remote.as_str()))).collect();
                    let auth = cloudlogin::auth(pr.key);
                    let form_open = self.cloud_form.as_ref().is_some_and(|f| f.0 == pr.key);
                    let this_login = login.as_ref().filter(|l| l.0 == pr.key).map(|l| l.1.clone());
                    egui::Frame::none().fill(th.bg).stroke(Stroke::new(1.0, if form_open || this_login.is_some() { th.accent } else { th.line })).rounding(8.0).inner_margin(Margin::symmetric(10.0, 7.0)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if let Some(a) = off.first().filter(|_| mine.is_empty()) {
                                    if ui.button(egui::RichText::new("Verbinden").strong()).on_hover_text(format!("Wieder als Laufwerk {}: einbinden", a.letter)).clicked() {
                                        acts.push(Act::CloudConnect(a.remote.clone(), true));
                                    }
                                    if ui.small_button("Entfernen").on_hover_text("Abmelden und aus Foldex entfernen").clicked() {
                                        acts.push(Act::CloudLogout(a.remote.clone()));
                                    }
                                } else if let Some(p) = mine.first() {
                                    if ui.button("Öffnen").clicked() {
                                        acts.push(Act::Go(p.path.clone()));
                                    }
                                    if let Some(r) = &p.remote {
                                        if ui.small_button("Entfernen").on_hover_text("Abmelden und Laufwerk entfernen").clicked() {
                                            acts.push(Act::CloudLogout(r.clone()));
                                        }
                                        if ui.small_button("Trennen").on_hover_text("Laufwerk trennen – angemeldet bleiben, später wieder verbinden").clicked() {
                                            acts.push(Act::CloudConnect(r.clone(), false));
                                        }
                                    }
                                    if auth.is_some() && !busy && ui.small_button("+ Konto").on_hover_text("Weiteres Konto dieses Anbieters anmelden").clicked() {
                                        match &auth {
                                            Some(cloudlogin::Auth::Form(f)) => self.cloud_form = Some((pr.key, vec![String::new(); f.len()])),
                                            _ => cloudlogin::start_login(pr.key, Vec::new(), ctx),
                                        }
                                    }
                                } else {
                                    if auth.is_some() {
                                        let b = ui.add_enabled(!busy, egui::Button::new(egui::RichText::new("Anmelden").strong()));
                                        let hover = match &auth {
                                            Some(cloudlogin::Auth::Browser(..)) => format!("Anmeldeseite von {} im Browser öffnen", pr.name),
                                            _ => "Mit Benutzer und Passwort anmelden".to_string(),
                                        };
                                        if b.on_hover_text(hover).clicked() {
                                            match &auth {
                                                Some(cloudlogin::Auth::Form(f)) => {
                                                    self.cloud_form = if form_open { None } else { Some((pr.key, vec![String::new(); f.len()])) };
                                                }
                                                _ => {
                                                    self.cloud_form = None;
                                                    cloudlogin::start_login(pr.key, Vec::new(), ctx);
                                                }
                                            }
                                        }
                                    }
                                }
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    let (ir, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                                    cloud::logo(ui.painter(), ir, pr.key);
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(pr.name).strong());
                                            if !mine.is_empty() {
                                                ui.label(egui::RichText::new(if mine.iter().any(|p| p.remote.is_some()) { "angemeldet" } else { "eingerichtet" }).size(11.5).color(th.accent));
                                            } else if !off.is_empty() {
                                                ui.label(egui::RichText::new("getrennt").size(11.5).color(th.sub));
                                            }
                                        });
                                        let sub = if let Some(a) = off.first().filter(|_| mine.is_empty()) {
                                            format!("Laufwerk {}: ({})", a.letter, a.name)
                                        } else if mine.is_empty() {
                                            pr.desc.to_string()
                                        } else {
                                            mine.iter().map(|p| if p.remote.is_some() { format!("{} ({})", p.path.display(), p.name) } else { p.path.display().to_string() }).collect::<Vec<_>>().join(" · ")
                                        };
                                        ui.add(egui::Label::new(egui::RichText::new(sub).size(11.5).color(th.sub)).truncate());
                                    });
                                });
                            });
                        });
                        // Anmeldeformular (iCloud, MEGA, Proton, Nextcloud)
                        if form_open && this_login.is_none() {
                            if let Some(cloudlogin::Auth::Form(fields)) = &auth {
                                ui.add_space(4.0);
                                let mut go = false;
                                if let Some((_, vals)) = self.cloud_form.as_mut() {
                                    for (i, (label, secret)) in fields.iter().enumerate() {
                                        if let Some(v) = vals.get_mut(i) {
                                            let r = ui.add(egui::TextEdit::singleline(v).password(*secret).hint_text(*label).desired_width(f32::INFINITY));
                                            if i == 0 && ctx.memory(|m| m.focused().is_none()) {
                                                r.request_focus();
                                            }
                                            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                                go = true;
                                            }
                                        }
                                    }
                                }
                                let hint = cloudlogin::form_hint(pr.key);
                                if !hint.is_empty() {
                                    ui.label(egui::RichText::new(hint).size(11.5).color(th.sub));
                                }
                                ui.horizontal(|ui| {
                                    if ui.add_enabled(!busy, egui::Button::new(egui::RichText::new("Anmelden").strong())).clicked() {
                                        go = true;
                                    }
                                    if ui.button("Abbrechen").clicked() {
                                        self.cloud_form = None;
                                    }
                                });
                                if go && !busy {
                                    if let Some((k, vals)) = self.cloud_form.take() {
                                        cloudlogin::start_login(k, vals, ctx);
                                    }
                                }
                            }
                        }
                        // Fortschritt bzw. Rückfrage der laufenden Anmeldung
                        if let Some(step) = &this_login {
                            ui.add_space(4.0);
                            match step {
                                cloudlogin::Step::Busy(t) => {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(egui::RichText::new(t).size(12.0));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.small_button("Abbrechen").clicked() {
                                                cloudlogin::cancel_login();
                                            }
                                        });
                                    });
                                }
                                cloudlogin::Step::Ask(t, secret, choices, pick_only) => {
                                    ui.add(egui::Label::new(egui::RichText::new(t).size(12.0).strong()).wrap());
                                    let mut go = false;
                                    let mut picked = None;
                                    if *pick_only {
                                        for (v, label) in choices {
                                            if ui.button(label).clicked() {
                                                picked = Some(v.clone());
                                            }
                                        }
                                        if ui.small_button("Abbrechen").clicked() {
                                            cloudlogin::cancel_login();
                                        }
                                    } else {
                                    ui.horizontal(|ui| {
                                        let r = ui.add(egui::TextEdit::singleline(&mut self.cloud_answer).password(*secret).desired_width(160.0));
                                        if ctx.memory(|m| m.focused().is_none()) {
                                            r.request_focus();
                                        }
                                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                            go = true;
                                        }
                                        if ui.button(egui::RichText::new("Weiter").strong()).clicked() {
                                            go = true;
                                        }
                                        if ui.button("Abbrechen").clicked() {
                                            cloudlogin::cancel_login();
                                        }
                                    });
                                    for (v, label) in choices {
                                        if ui.button(label).clicked() {
                                            picked = Some(v.clone());
                                        }
                                    }
                                    }
                                    if let Some(v) = picked {
                                        self.cloud_answer.clear();
                                        cloudlogin::answer(v);
                                    } else if go && !self.cloud_answer.trim().is_empty() {
                                        cloudlogin::answer(std::mem::take(&mut self.cloud_answer).trim().to_string());
                                    }
                                }
                                cloudlogin::Step::Done(t) => {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(t.as_str()).size(12.0).color(th.accent));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.small_button("OK").clicked() {
                                                cloudlogin::dismiss();
                                            }
                                        });
                                    });
                                }
                                cloudlogin::Step::Failed(t) => {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(egui::RichText::new(t).size(12.0).color(Color32::from_rgb(0xE5, 0x53, 0x4B)));
                                        if ui.small_button("OK").clicked() {
                                            cloudlogin::dismiss();
                                        }
                                    });
                                }
                            }
                        }
                    });
                    ui.add_space(3.0);
                }
            });
        });
        if !open {
            self.cloud_setup = false;
            self.cloud_form = None;
            cloudlogin::dismiss();
        }
    }

    fn side_header(&mut self, ui: &mut egui::Ui, title: &str, pin_zone: bool) -> egui::Response {
        let th = self.th;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
        let drop_here = pin_zone && self.drag.is_some() && self.drop_target == Some(Dest::Pin);
        if drop_here {
            ui.painter().rect_filled(rect, 4.0, th.sel);
        }
        let label = if drop_here {
            format!("{title}  –  zum Anheften loslassen")
        } else {
            title.to_string()
        };
        ui.painter().text(
            pos2(rect.left() + 14.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.5),
            th.sub,
        );
        if pin_zone {
            self.zones.push((rect, Dest::Pin));
        }
        resp
    }

    /// Rechtsklick auf „Cloud“ bzw. „Dieser PC“: ausgeblendete Einträge zurückholen
    fn side_unhide_menu(&self, resp: egui::Response, acts: &mut Vec<Act>) {
        let n = self.side.hidden.len();
        resp.context_menu(|ui| {
            if menu_item(ui, &format!("Ausgeblendete wieder anzeigen ({n})"), "", n > 0) {
                acts.push(Act::SideUnhide);
            }
        });
    }

    fn side_row(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        path: &Path,
        pin_idx: Option<usize>,
        cwd: &Path,
        acts: &mut Vec<Act>,
    ) {
        let th = self.th;
        let space = if pin_idx.is_none() { self.drive_space.get(path).copied() } else { None };
        let row_h = if space.is_some() { SIDE_ROW_H + 34.0 } else { SIDE_ROW_H };
        let (rect, resp) = ui.allocate_exact_size(
            vec2(ui.available_width(), row_h),
            Sense::click_and_drag(),
        );
        let current = cwd == path;
        let is_target =
            self.drag.is_some() && self.drop_target == Some(Dest::Dir(path.to_path_buf()));
        let inner = rect.shrink2(vec2(6.0, 1.0));
        if current || is_target {
            ui.painter().rect_filled(inner, 6.0, th.sel);
            if is_target {
                ui.painter().rect_stroke(inner.shrink(0.5), 6.0, Stroke::new(1.5, th.accent));
            }
        } else if resp.hovered() && self.drag.is_none() {
            ui.painter().rect_filled(inner, 6.0, th.hover);
        }
        if current {
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(inner.left(), inner.center().y - 8.0), vec2(3.0, 16.0)),
                2.0,
                th.accent,
            );
        }
        let kind = if pin_idx.is_some() {
            IconKind::Dir
        } else {
            IconKind::Drive
        };
        let top_y = rect.top() + SIDE_ROW_H / 2.0;
        let ir = Rect::from_min_size(
            pos2(inner.left() + 12.0, top_y - 9.0),
            vec2(18.0, 18.0),
        );
        let custom = foldericons::get(path);
        match pin_idx.and_then(|_| quick_icon(path)).filter(|_| custom.is_none()) {
            Some(ic) => ui::draw(ui.painter(), ir.shrink(1.0), ic, if current { th.accent } else { th.text }),
            None => self.icons.paint(
                ui.painter(),
                ir,
                kind,
                &path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                "",
                true,
                || path.to_path_buf(),
            ),
        }
        text_trunc(ui.painter(), pos2(ir.right() + 10.0, top_y), label, FontId::proportional(14.0), th.text, inner.right() - 8.0 - (ir.right() + 10.0));
        if resp.hovered() && label != path.to_string_lossy() {
            resp.clone().on_hover_text(path.display().to_string());
        }
        // Belegung wie im Explorer: Balken + „x GB frei von y GB“ (rot ab 90 % belegt)
        if let Some((free, total)) = space {
            let used = total.saturating_sub(free);
            let frac = if total > 0 { used as f32 / total as f32 } else { 0.0 };
            let bx = pos2(ir.left(), rect.top() + SIDE_ROW_H + 1.0);
            let bar = Rect::from_min_size(bx, vec2((inner.right() - 14.0 - bx.x).max(20.0), 8.0));
            ui.painter().rect_filled(bar, 3.0, th.line);
            let fill = Rect::from_min_size(bar.min, vec2(bar.width() * frac.clamp(0.0, 1.0), bar.height()));
            ui.painter().rect_filled(fill, 3.0, if frac > 0.9 { Color32::from_rgb(210, 70, 60) } else { th.accent });
            ui.painter().text(
                pos2(bar.left(), bar.bottom() + 5.0),
                Align2::LEFT_TOP,
                format!("{} frei von {}", human(free), human(total)),
                FontId::proportional(11.5),
                th.sub,
            );
        }
        self.zones.push((rect, Dest::Dir(path.to_path_buf())));
        if let Some(i) = pin_idx {
            self.pin_rects.push(rect);
            if resp.drag_started() && self.drag.is_none() {
                self.side_drag = Some(i);
            }
            if self.side_drag == Some(i) {
                ui.painter().rect_filled(inner, 5.0, th.sel);
            }
        }
        if resp.clicked() {
            acts.push(Act::Go(path.to_path_buf()));
        }
        if resp.middle_clicked() {
            acts.push(Act::TabOpen(path.to_path_buf()));
        }
        let (p2, n) = (path.to_path_buf(), self.side.pins.len());
        resp.context_menu(|ui| {
            if menu_item(ui, "Öffnen", "", true) {
                acts.push(Act::Go(p2.clone()));
            }
            if menu_item(ui, "In neuem Tab öffnen", "", true) {
                acts.push(Act::TabNew);
                acts.push(Act::Go(p2.clone()));
            }
            if let Some(i) = pin_idx {
                ui.separator();
                if menu_item(ui, "Umbenennen", "", true) {
                    acts.push(Act::SideRename(i));
                }
                if menu_item(ui, "Nach oben", "", i > 0) {
                    acts.push(Act::PinMove(i, i - 1));
                }
                if menu_item(ui, "Nach unten", "", i + 1 < n) {
                    acts.push(Act::PinMove(i, i + 1));
                }
                if menu_item(ui, "Von Schnellzugriff lösen", "", true) {
                    acts.push(Act::Unpin(i));
                }
            } else {
                ui.separator();
                if menu_item(ui, "An Schnellzugriff anheften", "", true) {
                    acts.push(Act::Pin(p2.clone()));
                }
                if menu_item(ui, "Dieses Laufwerk neu indexieren", "", true) {
                    acts.push(Act::Reindex(p2.clone()));
                }
                if menu_item(ui, "Aus der Seitenleiste entfernen", "", true) {
                    acts.push(Act::SideHide(p2.clone()));
                }
            }
            ui::menu_sep(ui);
            if menu_item(ui, "Icon ändern …", "", true) {
                acts.push(Act::FolderIcon(p2.clone()));
            }
            if menu_item(ui, "Eigenschaften", "", true) {
                acts.push(Act::Props(p2.clone()));
            }
        });
    }

    /// Git-Repository: Name links, Branch rechts (gedämpft); Klick öffnet den Ordner.
    fn side_git_row(&mut self, ui: &mut egui::Ui, i: usize, cwd: &Path, acts: &mut Vec<Act>) {
        let th = self.th;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SIDE_ROW_H), Sense::click());
        let inner = rect.shrink2(vec2(6.0, 1.0));
        let repo = &self.git[i];
        let current = cwd.starts_with(&repo.path);
        if current || (resp.hovered() && self.drag.is_none()) {
            ui.painter().rect_filled(inner, 3.0, th.hover);
        }
        let ir = Rect::from_min_size(pos2(inner.left() + 12.0, rect.center().y - 9.0), vec2(18.0, 18.0));
        self.icons.paint(ui.painter(), ir, IconKind::Dir, ".git", "", false, PathBuf::new);
        let right = inner.right() - 8.0;
        let mut name_max = right - (ir.right() + 10.0);
        if let Some(b) = &repo.branch {
            let g = ui.painter().layout_no_wrap(b.clone(), FontId::proportional(12.0), th.sub);
            let w = g.size().x.min(name_max * 0.45);
            ui.painter().with_clip_rect(Rect::from_x_y_ranges(right - w..=right, rect.y_range())).galley(
                pos2(right - w, rect.center().y - g.size().y / 2.0),
                g,
                th.sub,
            );
            name_max -= w + 8.0;
        }
        text_trunc(ui.painter(), pos2(ir.right() + 10.0, rect.center().y), &repo.name, FontId::proportional(14.0), th.text, name_max);
        let p = repo.path.clone();
        self.zones.push((rect, Dest::Dir(p.clone())));
        let resp = resp.on_hover_text(p.display().to_string());
        if resp.middle_clicked() {
            acts.push(Act::TabOpen(p.clone()));
        }
        if resp.clicked() {
            acts.push(Act::Go(p.clone()));
        }
        resp.context_menu(|ui| {
            if menu_item(ui, "Öffnen", "", true) {
                acts.push(Act::Go(p.clone()));
            }
            if menu_item(ui, "In neuem Tab öffnen", "", true) {
                acts.push(Act::TabNew);
                acts.push(Act::Go(p.clone()));
            }
            if menu_item(ui, "An Schnellzugriff anheften", "", true) {
                acts.push(Act::Pin(p.clone()));
            }
            if menu_item(ui, "Git-Repository anzeigen", "", true) {
                acts.push(Act::GitView(p.clone()));
            }
            if menu_item(ui, "Icon ändern …", "", true) {
                acts.push(Act::FolderIcon(p.clone()));
            }
            ui::menu_sep(ui);
            if menu_item(ui, "Repositories neu suchen", "", true) {
                acts.push(Act::RescanGit);
            }
            if menu_item(ui, "Eigenschaften", "", true) {
                acts.push(Act::Props(p.clone()));
            }
        });
    }

    /// Zuletzt verwendeter Ordner: Uhr-Symbol, Ordnername, voller Pfad als Tooltip.
    fn side_recent_row(&mut self, ui: &mut egui::Ui, p: &Path, acts: &mut Vec<Act>) {
        let th = self.th;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SIDE_ROW_H), Sense::click());
        let inner = rect.shrink2(vec2(6.0, 1.0));
        if resp.hovered() && self.drag.is_none() {
            ui.painter().rect_filled(inner, 6.0, th.hover);
        }
        let ir = Rect::from_center_size(pos2(inner.left() + 21.0, rect.center().y), vec2(16.0, 16.0));
        ui::draw(ui.painter(), ir, Ic::Clock, th.sub);
        let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
        text_trunc(ui.painter(), pos2(ir.right() + 11.0, rect.center().y), &name, FontId::proportional(14.0), th.text, inner.right() - 8.0 - (ir.right() + 11.0));
        self.zones.push((rect, Dest::Dir(p.to_path_buf())));
        let resp = resp.on_hover_text(p.display().to_string());
        if resp.middle_clicked() {
            acts.push(Act::TabOpen(p.to_path_buf()));
        }
        if resp.clicked() {
            acts.push(Act::Go(p.to_path_buf()));
        }
        let p2 = p.to_path_buf();
        resp.context_menu(|ui| {
            if menu_item(ui, "Öffnen", "", true) {
                acts.push(Act::Go(p2.clone()));
            }
            if menu_item(ui, "In neuem Tab öffnen", "", true) {
                acts.push(Act::TabNew);
                acts.push(Act::Go(p2.clone()));
            }
            if menu_item(ui, "An Schnellzugriff anheften", "", true) {
                acts.push(Act::Pin(p2.clone()));
            }
            ui::menu_sep(ui);
            if ui::menu_entry(ui, Some(Ic::Clock), "Verlauf löschen", "", true, false) {
                acts.push(Act::Cmd("app.clearrecent"));
            }
            if menu_item(ui, "Eigenschaften", "", true) {
                acts.push(Act::Props(p2.clone()));
            }
        });
    }

    /// Papierkorb: eigene Zeile wie die Ordner; Klick zeigt den Papierkorb im Fenster, Ziehen darauf löscht.
    fn side_trash_row(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let th = self.th;
        let current = self.pane().trash;
                ui.add_space(8.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SIDE_ROW_H), Sense::click());
                let inner = rect.shrink2(vec2(6.0, 1.0));
                let drop_here = self.drag.is_some() && self.drop_target == Some(Dest::Trash);
                if drop_here || current {
                    ui.painter().rect_filled(inner, 5.0, if drop_here { th.sel } else { th.hover });
                } else if resp.hovered() && self.drag.is_none() {
                    ui.painter().rect_filled(inner, 5.0, th.hover);
                }
                let ir = Rect::from_min_size(pos2(inner.left() + 12.0, rect.center().y - 9.0), vec2(18.0, 18.0));
                self.icons.paint_trash(ui.painter(), ir, self.trash_bytes.is_some_and(|b| b > 0));
                ui.painter().text(pos2(ir.right() + 10.0, rect.center().y), Align2::LEFT_CENTER, "Papierkorb", FontId::proportional(14.0), th.text);
                if let Some(b) = self.trash_bytes {
                    ui.painter().text(
                        pos2(inner.right() - 10.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        if b == 0 { "Leer".to_string() } else { human(b) },
                        FontId::proportional(12.5),
                        th.sub,
                    );
                }
                self.zones.push((rect, Dest::Trash));
                if resp.clicked() {
                    acts.push(Act::ShowTrash);
                }
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        self.pin_rects.clear();
        self.cloud_rects.clear();
        let th = self.th;
        let cwd = self.pane().cwd.clone();
        let area = ui.available_rect_before_wrap();
        let bg = ui.interact(area, ui.id().with("sidebg"), Sense::click());
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(6.0);
                if self.start_cfg.sidebar_entry {
                    self.side_start_row(ui, acts);
                    ui.add_space(2.0);
                }
                self.side_header(ui, "Schnellzugriff", true);
                for i in 0..self.side.pins.len() {
                    let (l, p) = (
                        self.side.pins[i].label.clone(),
                        self.side.pins[i].path.clone(),
                    );
                    self.side_row(ui, &l, &p, Some(i), &cwd, acts);
                }
                if self.st.show_git && (!self.git.is_empty() || self.git_rx.is_some()) {
                    ui.add_space(8.0);
                    self.side_header(ui, "Git-Repositories", false);
                    if self.git.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            ui.spinner();
                            ui.label(egui::RichText::new("Suche …").size(12.0).color(th.sub));
                        });
                    }
                    for i in 0..self.git.len() {
                        self.side_git_row(ui, i, &cwd, acts);
                    }
                }
                if self.st.show_recent && !self.recent.dirs.is_empty() {
                    ui.add_space(8.0);
                    self.side_header(ui, "Zuletzt verwendet", false);
                    let recent: Vec<PathBuf> = self.recent.dirs.iter().filter(|d| **d != cwd).take(8).cloned().collect();
                    for p in recent {
                        self.side_recent_row(ui, &p, acts);
                    }
                }
                // Cloud: eingerichtete Dienste und „Cloud hinzufügen“
                ui.add_space(8.0);
                let h = self.side_header(ui, "Cloud", false);
                self.side_unhide_menu(h, acts);
                let vis: Vec<cloud::Place> = cloud::places().into_iter().filter(|c| !self.side.hidden.contains(&c.path)).collect();
                for (i, c) in vis.iter().enumerate() {
                    self.side_cloud_row(ui, i, c, &cwd, acts);
                }
                self.side_cloud_add(ui, acts);
                if self.side.show_pc {
                    ui.add_space(8.0);
                    let h = self.side_header(ui, "Dieser PC", false);
                    self.side_unhide_menu(h, acts);
                    let own: Vec<PathBuf> = cloudlogin::letters().into_iter().map(|l| PathBuf::from(format!("{l}:\\"))).collect();
                    for i in 0..self.drives.len() {
                        let (l, p) = (self.drives[i].label.clone(), self.drives[i].path.clone());
                        // in Foldex angemeldete Cloud-Laufwerke stehen schon unter „Cloud“
                        if own.contains(&p) || self.side.hidden.contains(&p) {
                            continue;
                        }
                        self.side_row(ui, &l, &p, None, &cwd, acts);
                    }
                    // Papierkorb direkt unter den Laufwerken, über der PC-Suche
                    ui.add_space(4.0);
                    self.side_trash_row(ui, acts);
                    // PC-Suche: genau EINE Leiste – durchsucht alle Datenträger.
                    ui.add_space(8.0);
                    let (rect, _) = ui.allocate_exact_size(
                        vec2(ui.available_width() - 8.0, 34.0),
                        Sense::hover(),
                    );
                    ui.painter()
                        .rect(rect, 6.0, th.field, Stroke::new(1.0, th.line));
                    let c = pos2(rect.left() + 17.0, rect.center().y - 1.0);
                    ui.painter().circle_stroke(c, 5.0, Stroke::new(1.5, th.sub));
                    ui.painter().line_segment(
                        [c + vec2(3.6, 3.6), c + vec2(8.0, 8.0)],
                        Stroke::new(1.5, th.sub),
                    );
                    let te = egui::TextEdit::singleline(&mut self.pc_query)
                        .frame(false)
                        .hint_text("PC durchsuchen (alle Laufwerke)")
                        .margin(Margin {
                            left: 34.0,
                            right: 8.0,
                            top: 8.0,
                            bottom: 8.0,
                        });
                    let r = ui.put(rect, te);
                    if r.changed() {
                        acts.push(Act::PcSearch);
                    }
                    if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.pc_query.clear();
                        acts.push(Act::PcSearch);
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        if ui.add(egui::Label::new(egui::RichText::new("Inhalt durchsuchen (PDFs, Bilder, Dokumente) …").size(12.0).color(th.accent)).sense(Sense::click())).on_hover_text("z. B. „finde die Rechnung von Media Markt“ – liest Text in PDFs, Bildern (Texterkennung) und Dokumenten im aktuellen Ordner").clicked() {
                            acts.push(Act::ContentSearch(None));
                        }
                    });
                    if let Some(b) = &self.build {
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.spinner();
                            ui.label(
                                egui::RichText::new(format!(
                                    "Index {} … {}",
                                    b.root.display(),
                                    b.progress.load(Relaxed)
                                ))
                                .size(12.0)
                                .color(th.sub),
                            );
                        });
                    }
                }
                if !self.side.show_pc {
                    self.side_trash_row(ui, acts);
                }
            });
        let show_pc = self.side.show_pc;
        let show_git = self.st.show_git;
        let cwd2 = cwd.clone();
        bg.context_menu(|ui| {
            if menu_item(ui, "Aktuellen Ordner anheften", "", true) {
                acts.push(Act::Pin(cwd2.clone()));
            }
            if menu_item(
                ui,
                if show_pc {
                    "„Dieser PC“ ausblenden"
                } else {
                    "„Dieser PC“ anzeigen"
                },
                "",
                true,
            ) {
                acts.push(Act::TogglePc);
            }
            if show_git && menu_item(ui, "Git-Repositories neu suchen", "", true) {
                acts.push(Act::RescanGit);
            }
            ui.separator();
            if menu_item(ui, "Seitenleiste zurücksetzen", "", true) {
                acts.push(Act::ResetSidebar);
            }
            if menu_item(ui, "Einstellungen", "", true) {
                acts.push(Act::ShowSettings);
            }
        });
    }

    // ── Kontextmenüs ──
    fn bg_menu(&mut self, ui: &mut egui::Ui, pn: &Pane, acts: &mut Vec<Act>) {
        let st = self.st.clone();
        ui.menu_button("Ansicht", |ui| menu_view(ui, pn, acts));
        ui.menu_button("Sortieren nach", |ui| menu_sort(ui, pn, &st, acts));
        ui.menu_button("Gruppieren nach", |ui| menu_group(ui, pn, acts));
        ui.menu_button("Filtern", |ui| menu_filter(ui, pn, acts));
        if menu_item(ui, "Aktualisieren", "F5", true) {
            acts.push(Act::Refresh);
        }
        ui.separator();
        let undo = self.history.undo.last().map(|o| o.label());
        let redo = self.history.redo.last().map(|o| o.label());
        if menu_item(
            ui,
            &undo.as_ref().map_or("Rückgängig".to_string(), |l| format!("Rückgängig: {l}")),
            "Strg+Z",
            undo.is_some(),
        ) {
            acts.push(Act::Undo);
        }
        if menu_item(
            ui,
            &redo.as_ref().map_or("Wiederholen".to_string(), |l| format!("Wiederholen: {l}")),
            "Strg+Y",
            redo.is_some(),
        ) {
            acts.push(Act::Redo);
        }
        ui.separator();
        if menu_item(ui, "Einfügen", "Strg+V", self.clip.is_some() || clip::has_files()) {
            acts.push(Act::PasteInto(pn.cwd.clone()));
        }
        ui.menu_button("Neu", |ui| {
            if menu_item(ui, "Ordner", "Strg+Umschalt+N", true) {
                acts.push(Act::NewFolder);
            }
            if menu_item(ui, "Textdokument", "", true) {
                acts.push(Act::NewFile);
            }
        });
        ui.separator();
        if menu_item(ui, "In neuem Tab öffnen", "", true) {
            acts.push(Act::TabDup(self.cur));
        }
        if ui::menu_entry(ui, Some(Ic::Terminal), "Terminal hier öffnen", &self.keymap.label("term.new"), true, false) {
            acts.push(Act::Terminal(None, pn.cwd.clone()));
        }
        if !self.ed_paths.is_empty() {
            let eds = self.ed_paths.clone();
            let cwd = pn.cwd.clone();
            ui.menu_button("Ordner öffnen in", |ui| {
                ui.set_min_width(250.0);
                for (k, n, _) in eds {
                    if ui::menu_entry(ui, Some(Ic::Code), &n, "", true, false) {
                        acts.push(Act::OpenIn(k, cwd.clone()));
                    }
                }
            });
        }
        if self.st.ai_enabled {
            let (backends, keys) = (self.backends(), self.keymap.clone());
            ui.menu_button("KI", |ui| {
                ui.set_min_width(240.0);
                ai_menu(ui, &pn.cwd, &keys, &backends, acts);
            });
        }
        if self.git_st.is_none() && self.git_ok && ui::menu_entry(ui, Some(Ic::Git), "Git-Repository erstellen / klonen …", "", true, false) {
            acts.push(Act::GitView(pn.cwd.clone()));
        }
        if self.git_st.is_some() && ui::menu_entry(ui, Some(Ic::Git), "Git-Repository anzeigen", &self.keymap.label("git.view"), true, false) {
            acts.push(Act::GitView(pn.cwd.clone()));
        }
        ui.menu_button("Werkzeuge", |ui| {
            ui.set_min_width(250.0);
            tools_items(ui, acts);
        });
        if menu_item(
            ui,
            if self.dual_now {
                "Dual-Panel schließen"
            } else {
                "Dual-Panel öffnen"
            },
            "Strg+Umschalt+D",
            true,
        ) {
            acts.push(Act::ToggleDual);
        }
        if self.dual_now {
            if menu_item(ui, "Teilung: Ausrichtung wechseln (nebeneinander/übereinander)", "", true) {
                acts.push(Act::ToggleOrient);
            }
            if menu_item(ui, "Dieses Teilfenster in eigenen Tab lösen", "", true) {
                acts.push(Act::PaneToTab(self.tabs[self.cur].active));
            }
        }
        if menu_item(ui, "Papierkorb anzeigen", "", true) {
            acts.push(Act::ShowTrash);
        }
        if menu_item(ui, "Suchindex aktualisieren", "", true) {
            acts.push(Act::Reindex(pn.cwd.clone()));
        }
        if menu_item(ui, "Zusammenfassung dieses Ordners", "", true) {
            acts.push(Act::Summary(pn.cwd.clone()));
        }
        if menu_item(ui, "Eigenschaften", "", true) {
            acts.push(Act::Props(pn.cwd.clone()));
        }
    }

    fn item_menu(&mut self, ui: &mut egui::Ui, pn: &Pane, acts: &mut Vec<Act>) {
        let paths = pn.sel_paths();
        if paths.is_empty() {
            return;
        }
        let single = paths.len() == 1;
        let single_dir = single && fsx::is_dir(&paths[0]);
        if menu_item(ui, "Öffnen", "Enter", single) {
            acts.push(Act::Open(paths[0].clone()));
        }
        if menu_item(ui, "In neuem Tab öffnen", "", single_dir) {
            acts.push(Act::TabNew);
            acts.push(Act::Go(paths[0].clone()));
        }
        if single {
            let p0 = paths[0].clone();
            let name = p0.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let editors: Vec<(String, String)> = self.ed_paths.iter().map(|e| (e.0.clone(), e.1.clone())).collect();
            if single_dir {
                if !editors.is_empty() {
                    ui.menu_button("Ordner öffnen in", |ui| {
                        ui.set_min_width(250.0);
                        for (k, n) in &editors {
                            if ui::menu_entry(ui, Some(Ic::Code), n, "", true, false) {
                                acts.push(Act::OpenIn(k.clone(), p0.clone()));
                            }
                        }
                    });
                }
                if ui::menu_entry(ui, Some(Ic::Terminal), "Terminal hier öffnen", "", true, false) {
                    acts.push(Act::Terminal(None, p0.clone()));
                }
            } else {
                ui.menu_button("Öffnen mit", |ui| {
                    ui.set_min_width(250.0);
                    if editors::is_text(&name) || !editors.is_empty() {
                        for (k, n) in &editors {
                            if ui::menu_entry(ui, Some(Ic::Code), n, "", true, false) {
                                acts.push(Act::OpenIn(k.clone(), p0.clone()));
                            }
                        }
                        if editors.is_empty() {
                            ui::menu_label(ui, "Kein Editor gefunden (Einstellungen → Editoren)");
                        }
                        ui::menu_sep(ui);
                    }
                    if ui::menu_entry(ui, Some(Ic::Open), "Standard-App", "Enter", true, false) {
                        acts.push(Act::OpenFile(p0.clone()));
                    }
                    if ui::menu_entry(ui, Some(Ic::Open), "Andere App auswählen …", "", true, false) {
                        acts.push(Act::OpenWith(p0.clone()));
                    }
                });
            }
        }
        // Programme, Skripte, Installer: Verknüpfung anlegen (wie im Explorer)
        let exes: Vec<PathBuf> = paths.iter().filter(|p| !fsx::is_dir(p) && (cfg!(windows) || !fsx::slow(p)) && shortcut::is_executable(p)).cloned().collect();
        if !exes.is_empty() {
            ui::menu_sep(ui);
            if ui::menu_entry(ui, Some(Ic::Plus), "Verknüpfung erstellen", "", true, false) {
                acts.push(Act::Shortcut(exes.clone(), false));
            }
            if ui::menu_entry(ui, Some(Ic::Desktop), "Verknüpfung auf dem Desktop", "", true, false) {
                acts.push(Act::Shortcut(exes.clone(), true));
            }
        }
        // Sprachnotizen und Untertitel (lokal mit Whisper)
        let audio: Vec<PathBuf> = paths.iter().filter(|p| fsx::is_file(p) && convert::class_of(p) == convert::Class::Audio).cloned().collect();
        let video: Vec<PathBuf> = paths.iter().filter(|p| fsx::is_file(p) && convert::class_of(p) == convert::Class::Video).cloned().collect();
        let images: Vec<PathBuf> = paths.iter().filter(|p| fsx::is_file(p) && batch::is_image(p)).cloned().collect();
        if !audio.is_empty() || !video.is_empty() || !images.is_empty() {
            ui::menu_sep(ui);
        }
        if !audio.is_empty() && ui::menu_entry(ui, Some(Ic::Docs), "In Text umwandeln (Whisper)", "", true, false) {
            acts.push(Act::Transcribe(audio, false));
        }
        if !video.is_empty() && ui::menu_entry(ui, Some(Ic::Docs), "Untertitel erzeugen (.srt)", "", true, false) {
            acts.push(Act::Transcribe(video, true));
        }
        if !images.is_empty() && ui::menu_entry(ui, Some(Ic::Tools), &if images.len() > 1 { format!("{} Bilder bearbeiten …", images.len()) } else { "Bild stapelweise bearbeiten …".to_string() }, "", true, false) {
            acts.push(Act::BatchImages(images));
        }
        if paths.iter().all(|p| fsx::is_file(&p)) {
            // Zielformate, die für alle ausgewählten Dateien gehen (bei einer Datei: alle für diese)
            let mut targets = convert::targets(&paths[0]);
            for p in &paths[1..] {
                let t2 = convert::targets(p);
                targets.retain(|t| t2.contains(t));
            }
            if !targets.is_empty() {
                let srcs = paths.clone();
                let title = if single { "Konvertieren in …".to_string() } else { format!("{} Dateien konvertieren in …", paths.len()) };
                ui.menu_button(title, |ui| {
                    ui.set_min_width(240.0);
                    // nach Art gruppiert: Bild, Audio, Video, Dokument, Archiv
                    let mut last = None;
                    for t in targets {
                        let k = if convert::ARCHIVE_OUT.contains(&t) { convert::Class::Archive } else { convert::class(t) };
                        if last != Some(k) {
                            if last.is_some() {
                                ui::menu_sep(ui);
                            }
                            ui::menu_label(ui, convert::class_name(k));
                        }
                        last = Some(k);
                        if ui::menu_entry(ui, Some(Ic::Refresh), &format!(".{t}"), "", true, false) {
                            acts.push(if srcs.len() == 1 { Act::Convert(srcs[0].clone(), t.to_string()) } else { Act::ConvertMany(srcs.clone(), t.to_string()) });
                        }
                    }
                });
            }
        }
        let archives: Vec<PathBuf> = paths
            .iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| extract::is_archive(&n.to_string_lossy()))
            })
            .cloned()
            .collect();
        ui.separator();
        ui.menu_button("Komprimieren in …", |ui| {
            for f in Format::ALL {
                if menu_item(ui, f.label(), "", true) {
                    acts.push(Act::Compress(paths.clone(), f));
                }
            }
        });
        if !archives.is_empty() {
            if menu_item(ui, "Hier entpacken", "", true) {
                acts.push(Act::Extract(archives.clone(), false));
            }
            let label = if archives.len() == 1 {
                format!(
                    "In »{}\\« entpacken",
                    extract::folder_name(&archives[0].file_name().unwrap().to_string_lossy())
                )
            } else {
                "Jedes in eigenen Ordner entpacken".to_string()
            };
            if menu_item(ui, &label, "", true) {
                acts.push(Act::Extract(archives, true));
            }
        }
        ui.separator();
        if menu_item(ui, "Ausschneiden", "Strg+X", true) {
            acts.push(Act::SetClip(paths.clone(), true));
        }
        if menu_item(ui, "Kopieren", "Strg+C", true) {
            acts.push(Act::SetClip(paths.clone(), false));
        }
        if menu_item(ui, "Einfügen", "Strg+V", (self.clip.is_some() || clip::has_files()) && single_dir) {
            acts.push(Act::PasteInto(paths[0].clone()));
        }
        if self.dual_now {
            if menu_item(ui, "In anderes Panel kopieren", "", true) {
                acts.push(Act::TransferOther(paths.clone(), true));
            }
            if menu_item(ui, "In anderes Panel verschieben", "", true) {
                acts.push(Act::TransferOther(paths.clone(), false));
            }
        }
        if menu_item(ui, "Teilen …", &self.keymap.label("file.share"), paths.iter().any(|p| fsx::is_file(&p))) {
            acts.push(Act::Share(paths.clone()));
        }
        // nur bei Formaten, die sich drucken lassen
        if paths.iter().any(|p| !fsx::is_dir(p) && print::supported(p)) && menu_item(ui, "Drucken", &self.keymap.label("file.print"), true) {
            acts.push(Act::Print(paths.clone()));
        }
        if menu_item(ui, "Pfad kopieren", "", true) {
            acts.push(Act::CopyText(
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }
        ui.separator();
        if menu_item(ui, "Umbenennen", "F2", single) {
            acts.push(Act::Rename);
        }
        if menu_item(ui, "Löschen (Papierkorb)", "Entf", true) {
            acts.push(Act::Trash(paths.clone()));
        }
        if menu_item(ui, "Endgültig löschen …", "Umschalt+Entf", true) {
            acts.push(Act::DeletePerm(paths.clone()));
        }
        ui.separator();
        if menu_item(ui, "An Schnellzugriff anheften", "", single_dir) {
            acts.push(Act::Pin(paths[0].clone()));
        }
        if menu_item(ui, "Ordner-Icon ändern …", "", single_dir) {
            acts.push(Act::FolderIcon(paths[0].clone()));
        }
        if !paths.is_empty() && self.in_cloud(&pn.cwd) {
            ui.separator();
            if ui::menu_entry(ui, Some(Ic::Check), "Immer auf diesem Gerät behalten", "", true, false) {
                acts.push(Act::CloudKeep(paths.clone(), true));
            }
            if ui::menu_entry(ui, Some(Ic::Download), "Speicherplatz freigeben", "nur online", true, false) {
                acts.push(Act::CloudKeep(paths.clone(), false));
            }
        }
        if single && summary::supported(&paths[0]) && menu_item(ui, "Zusammenfassung", "", true) {
            acts.push(Act::Summary(paths[0].clone()));
        }
        if menu_item(ui, "Eigenschaften", "Alt+Enter", single) {
            acts.push(Act::Props(paths[0].clone()));
        }
    }

    // ── Pane ──
    fn ui_pane(&mut self, ui: &mut egui::Ui, ti: usize, pi: usize, acts: &mut Vec<Act>) {
        let mut pn = std::mem::take(&mut self.tabs[ti].panes[pi]);
        let tid = self.tabs[ti].id;
        let multi = self.tabs[ti].dual();
        let rect = ui.max_rect();
        if ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| rect.contains(p))
        {
            self.tabs[ti].active = pi;
        }
        let active = self.tabs[ti].active == pi;
        self.dual_now = multi;
        ui.push_id((tid, pi), |ui| {
            if multi || !pn.stack.is_empty() {
                self.pane_header(ui, &pn, tid, pi, active, acts);
            }
            // KI ausgeschaltet: KI-Kacheln schließen (Agenten werden beendet), die Kachel zeigt wieder Dateien
            if !self.st.ai_enabled && is_ai_pane(&pn) {
                pn.extra = None;
                self.status = "KI-Funktionen sind ausgeschaltet – KI-Kacheln wurden geschlossen".into();
            }
            if let Some(PaneView::Ai(a)) = &mut pn.extra {
                let rect = ui.available_rect_before_wrap();
                let outs = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| self.chat_ui(a, ui)).inner;
                self.handle_ai_outs(outs, acts);
                return;
            }
            if let Some(PaneView::View(v)) = &mut pn.extra {
                let rect = ui.available_rect_before_wrap();
                let th = self.th;
                let path = v.path.clone();
                // großes Preview: dort lassen sich Textdateien bearbeiten
                v.pv.big = true;
                let out = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| v.pv.ui(ui, &th, &mut self.icons, Some(&path), None)).inner;
                self.preview_out(&mut v.pv, out, acts);
                return;
            }
            if let Some(v) = &mut pn.extra {
                let th = self.th;
                let term_ai = self.term_ai_cfg();
                let ais = if matches!(v, PaneView::Git(_)) { self.commit_ais() } else { Vec::new() };
                let rect = ui.available_rect_before_wrap();
                let out = ui
                    .allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| match v {
                        PaneView::Term(t) => {
                            t.complete = self.st.term_complete;
                            t.ai = term_ai.clone();
                            t.ui(ui, &th);
                            None
                        }
                        PaneView::Git(g) => g.ui(ui, &th, &ais),
                        PaneView::Ai(_) | PaneView::View(_) => None,
                    })
                    .inner;
                match out {
                    Some(gitui::GitOut::OpenFile(p)) => acts.push(Act::OpenFile(p)),
                    Some(gitui::GitOut::Reveal(p)) => acts.push(Act::Go(p)),
                    None => {}
                }
                return;
            }
            if let Some(v) = pn.ctrash.as_deref_mut() {
                self.ui_trash_page(ui, v);
                return;
            }
            if pn.start {
                if pn.start_settings {
                    self.ui_start_settings(ui, &mut pn);
                } else {
                    self.ui_start(ui, &mut pn, acts);
                }
                return;
            }
            prof::mark("pane-start");
            self.pane_nav(ui, &mut pn, acts);
            prof::mark("pane_nav");
            if pn.trash {
                self.ui_trash(ui);
            } else {
                self.ui_cmdbar(ui, &pn, acts);
                prof::mark("cmdbar");
                // Liste und – in der aktiven Kachel – Vorschau nebeneinander
                let full = ui.available_rect_before_wrap();
                // Jede Kachel hat ihre eigene Vorschau: an/aus, links/rechts/oben/unten, Größe per Ziehen.
                // Sie teilt sich den Platz der Kachel mit der Liste – nichts außerhalb wird verschoben oder verdeckt.
                let side = pn.preview_side;
                let room = if side.horizontal() { full.width() > 460.0 } else { full.height() > 320.0 };
                let show = pn.preview_on && room;
                let (list, pr, grip) = if show { preview_split(full, side, pn.preview_share()) } else { (full, Rect::NOTHING, Rect::NOTHING) };
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(list), |ui| {
                    if pn.search.is_some() {
                        self.ui_results(ui, &mut pn, acts);
                    } else {
                        self.ui_list(ui, &mut pn, acts);
                        prof::mark("list");
                    }
                });
                if show {
                    let targets = pane_targets(&pn);
                    let multi = (targets.len() > 1).then(|| {
                        pn.refresh_sel_size();
                        (targets.len(), pn.sel_size)
                    });
                    let target = match targets.len() {
                        1 => Some(targets[0].clone()),
                        0 if pn.search.is_none() => Some(pn.cwd.clone()),
                        _ => None,
                    };
                    let th = self.th;
                    // Griff zum Vergrößern/Verkleinern (Doppelklick = Standardgröße)
                    let gresp = ui.interact(grip, Id::new(("pvgrip", tid, pi)), Sense::click_and_drag());
                    if gresp.hovered() || gresp.dragged() {
                        ui.ctx().set_cursor_icon(if side.horizontal() { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
                    }
                    if gresp.dragged() {
                        if let Some(p) = gresp.interact_pointer_pos() {
                            let f = match side {
                                PvSide::Right => (full.right() - p.x) / full.width(),
                                PvSide::Left => (p.x - full.left()) / full.width(),
                                PvSide::Bottom => (full.bottom() - p.y) / full.height(),
                                PvSide::Top => (p.y - full.top()) / full.height(),
                            };
                            pn.preview_frac = f.clamp(0.12, 0.85);
                        }
                    }
                    if gresp.double_clicked() {
                        pn.preview_frac = 0.0;
                    }
                    let hot = gresp.hovered() || gresp.dragged();
                    let (a, b) = if side.horizontal() { (pos2(grip.center().x, grip.top()), pos2(grip.center().x, grip.bottom())) } else { (pos2(grip.left(), grip.center().y), pos2(grip.right(), grip.center().y)) };
                    ui.painter().line_segment([a, b], Stroke::new(if hot { 2.0 } else { 1.0 }, if hot { th.accent } else { th.line }));
                    // Kopfzeile: Lage wählen, schließen
                    let head = Rect::from_min_size(pr.min, vec2(pr.width(), 30.0));
                    ui.painter().rect_filled(head, 0.0, th.panel);
                    ui.painter().line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, th.line));
                    ui.painter().text(pos2(head.left() + 12.0, head.center().y), Align2::LEFT_CENTER, "Vorschau", FontId::proportional(12.5), th.sub);
                    let close = Rect::from_center_size(pos2(head.right() - 16.0, head.center().y), vec2(24.0, 22.0));
                    if ui::tool_btn_at(ui, &th, close, Id::new(("pvclose", tid, pi)), Ic::Close, &format!("Vorschau dieser Kachel schließen ({})", self.keymap.label("view.preview")), false).clicked() {
                        pn.preview_on = false;
                    }
                    for (k, s) in PvSide::ALL.into_iter().rev().enumerate() {
                        let r = Rect::from_center_size(pos2(close.left() - 18.0 - k as f32 * 26.0, head.center().y), vec2(24.0, 22.0));
                        let resp = ui.interact(r, Id::new(("pvside", tid, pi, k)), Sense::click());
                        let on = s == side;
                        if on || resp.hovered() {
                            ui.painter().rect_filled(r, 5.0, if on { th.sel } else { th.hover });
                        }
                        paint_side_icon(ui.painter(), r.center(), s, if on { th.accent } else { th.sub });
                        if resp.on_hover_text(format!("Vorschau {}", s.label())).clicked() {
                            pn.preview_side = s;
                        }
                    }
                    let body = Rect::from_min_max(pos2(pr.left(), head.bottom()), pr.max);
                    prof::mark("pre-preview");
                    let out = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body), |ui| pn.preview.ui(ui, &th, &mut self.icons, target.as_deref(), multi)).inner;
                    prof::mark("preview");
                    self.preview_out(&mut pn.preview, out, acts);
                }
                ui.advance_cursor_after_rect(full);
            }
        });
        self.tabs[ti].panes[pi] = pn;
    }

    /// Kopfzeile einer Kachel: ihre eigenen Tabs (wechseln, schließen, ziehen = andocken / in andere Kachel /
    /// als Tab lösen / anderes Fenster), „+“ für einen neuen Tab, rechts „Als eigenen Tab lösen“.
    fn pane_header(&mut self, ui: &mut egui::Ui, pn: &Pane, tid: u64, pi: usize, active: bool, acts: &mut Vec<Act>) {
        let th = self.th;
        let multi = self.tabs.iter().find(|t| t.id == tid).is_some_and(|t| t.panes.len() > 1);
        let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
        self.head_rects.push((pi, strip));
        ui.painter().rect_filled(strip, Rounding { nw: 8.0, ne: 8.0, sw: 0.0, se: 0.0 }, th.panel);
        ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, th.line));
        let tabs = sub_tabs(pn);
        let font = FontId::proportional(13.0);
        let right_w = if multi { 40.0 } else { 4.0 };
        let avail = strip.width() - 12.0 - 34.0 - right_w;
        let mut widths: Vec<f32> = tabs
            .iter()
            .map(|(t, _, _)| (ui.painter().layout_no_wrap(t.clone(), font.clone(), th.text).size().x + 62.0).clamp(96.0, 220.0))
            .collect();
        let sum: f32 = widths.iter().map(|w| w + 2.0).sum();
        if sum > avail {
            let k = (avail / sum).max(0.0);
            widths.iter_mut().for_each(|w| *w = (*w * k).max(96.0));
        }
        // passen die Tabs nicht hinein: waagerecht scrollen (Mausrad über der Kopfzeile)
        let tabs_area = Rect::from_min_max(pos2(strip.left() + 6.0, strip.top()), pos2(strip.left() + 6.0 + avail.max(40.0), strip.bottom()));
        let total: f32 = widths.iter().map(|w| w + 2.0).sum();
        let sid = Id::new(("subscroll", tid, pi));
        let mut scroll = ui.data(|d| d.get_temp::<f32>(sid)).unwrap_or(0.0);
        let overflow = total > tabs_area.width() + 0.5;
        let max_scroll = (total - tabs_area.width()).max(0.0);
        if overflow && ui.rect_contains_pointer(strip) {
            let d = ui.input(|i| i.smooth_scroll_delta);
            scroll -= if d.x.abs() > d.y.abs() { d.x } else { d.y };
        }
        scroll = if overflow { scroll.clamp(0.0, max_scroll) } else { 0.0 };
        ui.data_mut(|d| d.insert_temp(sid, scroll));
        // eigener Tab dieser Kachel wird über ihre Kopfzeile gezogen → umsortieren statt aufnehmen
        let own_drag = match self.tab_drag.as_ref().map(|d| (d.kind, d.pos)) {
            Some((DragKind::Sub(t, p, k), pos)) if t == tid && p == pi && strip.contains(pos) => Some((k, pos)),
            _ => None,
        };
        let drag_target = own_drag.is_none() && self.tab_drag.is_some() && self.merge_target_last == Some(pi);
        if drag_target {
            ui.painter().rect_stroke(strip.shrink(1.0), 8.0, Stroke::new(1.5, th.accent));
        }
        if let Some((from, pos)) = own_drag {
            // Zielposition: Anzahl der übrigen Tabs, deren Mitte links vom Zeiger liegt
            let mut x0 = strip.left() + 6.0 - scroll;
            let mut to = 0;
            let mut mark = x0;
            for (k, w) in widths.iter().enumerate() {
                if k != from && x0 + w / 2.0 < pos.x {
                    to += 1;
                    mark = x0 + w + 1.0;
                }
                x0 += w + 2.0;
            }
            if to == 0 {
                mark = strip.left() + 5.0;
            }
            self.sub_reorder = Some((pi, to));
            let m = Rect::from_center_size(pos2(mark, strip.center().y + 2.0), vec2(3.0, strip.height() - 12.0));
            ui.painter().rect_filled(m, 1.5, th.accent);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        let mut x = strip.left() + 6.0 - scroll;
        let n = tabs.len();
        let outer_clip = ui.clip_rect();
        if overflow {
            ui.set_clip_rect(tabs_area.intersect(outer_clip));
        }
        let clipped = |r: Rect| if overflow { r.intersect(tabs_area) } else { r };
        for (k, (title, ic, vis)) in tabs.into_iter().enumerate() {
            let w = widths[k];
            let tab = Rect::from_min_size(pos2(x, strip.top() + 5.0), vec2(w, strip.height() - 5.0));
            x += w + 2.0;
            let resp = ui.interact(clipped(tab), ui.id().with(("subtab", pi, k)), Sense::click_and_drag());
            let on = vis && active;
            let dragged = matches!(self.tab_drag.as_ref().map(|d| d.kind), Some(DragKind::Sub(t, p, kk)) if t == tid && p == pi && kk == k);
            let fill = if dragged {
                th.hover
            } else if vis {
                th.bg
            } else if resp.hovered() {
                th.hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(tab, Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 }, fill);
            if dragged {
                ui.painter().rect_stroke(tab.shrink(0.5), Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 }, Stroke::new(1.0, th.accent.gamma_multiply(0.6)));
            }
            if on {
                ui.painter().rect_filled(Rect::from_min_size(tab.min + vec2(8.0, 0.0), vec2(tab.width() - 16.0, 2.0)), 1.0, th.accent);
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            let ir = Rect::from_min_size(pos2(tab.left() + 9.0, tab.center().y - 8.0), vec2(16.0, 16.0));
            match ic {
                Some(ic) => ui::draw(ui.painter(), ir, ic, if on { th.accent } else { th.sub }),
                None => self.icons.paint(ui.painter(), ir, IconKind::Dir, "", "", false, PathBuf::new),
            }
            let show_x = vis || resp.hovered();
            text_trunc(
                ui.painter(),
                pos2(ir.right() + 7.0, tab.center().y),
                &title,
                font.clone(),
                if vis { th.text } else { th.sub },
                tab.right() - if show_x { 28.0 } else { 8.0 } - (ir.right() + 7.0),
            );
            let cr = Rect::from_center_size(pos2(tab.right() - 13.0, tab.center().y), vec2(20.0, 20.0));
            let cresp = ui.interact(clipped(cr), resp.id.with("x"), Sense::click()).on_hover_text("Tab schließen");
            if show_x {
                if cresp.hovered() {
                    ui.painter().rect_filled(cr, 4.0, th.hover);
                }
                ui::draw(ui.painter(), cr.shrink(5.0), Ic::Close, if cresp.hovered() { th.text } else { th.sub });
            }
            if cresp.clicked() || resp.middle_clicked() {
                acts.push(Act::SubClose(pi, k));
            } else if resp.clicked() {
                acts.push(Act::SubSwitch(pi, k));
            }
            if resp.drag_started() && self.tab_drag.is_none() {
                if let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
                    // einzelner Tab einer Gruppe – oder die ganze Kachel, wenn sie nur einen Tab hat
                    let (kind, spec) = if n > 1 {
                        let pos = pn.stack_pos.min(pn.stack.len());
                        let sp = if k == pos { single_spec(pn) } else { single_spec(&pn.stack[if k < pos { k } else { k - 1 }]) };
                        (DragKind::Sub(tid, pi, k), sp)
                    } else {
                        (DragKind::Pane(tid, pi), pane_spec(pn))
                    };
                    self.tab_drag = Some(TabDrag {
                        kind,
                        title: title.clone(),
                        grab_dx: (p.x - tab.left()).clamp(0.0, 160.0),
                        pos: p,
                        screen: None,
                        px: None,
                        outside: false,
                        payload: ipc::Payload { title: title.clone(), layout: "0".into(), paths: vec![spec] },
                    });
                }
            }
            resp.context_menu(|ui| {
                if ui::menu_entry(ui, Some(Ic::Close), "Tab schließen", &self.keymap.label("tile.close"), true, false) {
                    acts.push(Act::SubClose(pi, k));
                }
                if n > 1 && ui::menu_entry(ui, Some(Ic::Window), "Als eigenen Tab lösen", "", true, false) {
                    acts.push(Act::SubToTab(pi, k));
                }
                if n == 1 && multi && ui::menu_entry(ui, Some(Ic::Window), "Als eigenen Tab lösen", "", true, false) {
                    acts.push(Act::PaneToTab(pi));
                }
                ui::menu_sep(ui);
                if ui::menu_entry(ui, Some(Ic::Split), "Ausrichtung wechseln", "", multi, false) {
                    self.tabs.iter_mut().filter(|t| t.id == tid).for_each(|t| t.active = pi);
                    acts.push(Act::ToggleOrient);
                }
                if ui::menu_entry(ui, Some(Ic::Grid), "Alle Kacheln gleich groß", "", multi, false) {
                    acts.push(Act::Equalize);
                }
            });
        }
        ui.set_clip_rect(outer_clip);
        if overflow {
            let len = (tabs_area.width() * tabs_area.width() / total).max(24.0);
            let pos = if max_scroll > 0.0 { scroll / max_scroll * (tabs_area.width() - len) } else { 0.0 };
            ui.painter().rect_filled(Rect::from_min_size(pos2(tabs_area.left() + pos, strip.bottom() - 3.0), vec2(len, 3.0)), 1.5, th.sub.gamma_multiply(if ui.rect_contains_pointer(strip) { 0.55 } else { 0.25 }));
        }
        // „+“: neuer Tab in dieser Kachel; Rechtsklick: Terminal / Git
        let pr = Rect::from_min_size(pos2((x + 2.0).min(tabs_area.right() + 2.0), strip.top() + 4.0), vec2(30.0, 28.0));
        let tip = format!("Neuer Tab in dieser Kachel ({})  ·  Rechtsklick: Terminal, Git", self.keymap.label("tile.newtab"));
        let presp = ui::tool_btn_at(ui, &th, pr, ui.id().with(("subplus", pi)), Ic::Plus, &tip, false);
        let cwd = pn.cwd.clone();
        if presp.clicked() {
            acts.push(Act::SubNew(pi, cwd.clone()));
        }
        let shells = self.shells.clone();
        presp.context_menu(|ui| {
            if ui::menu_entry(ui, Some(Ic::NewFolder), "Ordner-Tab", &self.keymap.label("tile.newtab"), true, false) {
                acts.push(Act::SubNew(pi, cwd.clone()));
            }
            ui::menu_sep(ui);
            ui::menu_label(ui, "Terminal");
            for (name, prog) in &shells {
                if ui::menu_entry(ui, Some(Ic::Terminal), name, "", true, false) {
                    acts.push(Act::SubNew(pi, PathBuf::from(format!("{SPEC_TERM}{prog}*{}", cwd.display()))));
                }
            }
            if self.st.ai_enabled {
                ui::menu_sep(ui);
                ui::menu_label(ui, "KI");
                for (b, name) in self.backends() {
                    if ui::menu_entry(ui, Some(Ic::Spark), &name, "", true, false) {
                        let mode = if b.starts_with("agent:") { "agent" } else { "chat" };
                        acts.push(Act::SubNew(pi, PathBuf::from(format!("{SPEC_AI}{mode}*{b}*{}", cwd.display()))));
                    }
                }
            }
            ui::menu_sep(ui);
            if ui::menu_entry(ui, Some(Ic::Git), "Git-Repository", "", true, false) {
                let root = git::find_root(&cwd).unwrap_or_else(|| cwd.clone());
                acts.push(Act::SubNew(pi, PathBuf::from(format!("{SPEC_GIT}{}", root.display()))));
            }
        });
        // rechts: Kachel als eigenen Tab lösen
        if multi {
            let br = Rect::from_min_size(pos2(strip.right() - 36.0, strip.top() + 3.0), vec2(30.0, 28.0));
            if ui::tool_btn_at(ui, &th, br, ui.id().with(("totab", pi)), Ic::Window, "Kachel als eigenen Tab lösen", false).clicked() {
                acts.push(Act::PaneToTab(pi));
            }
        }
    }

    /// Alle Kacheln des aktuellen Tabs samt ziehbaren Trennern.
    fn ui_tiles(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let th = self.th;
        let ti = self.cur;
        let rect = ui.max_rect();
        self.central_rect = rect;
        let multi = self.tabs[ti].dual();
        let gap = if multi { 10.0 } else { 0.0 };
        let area = if multi { rect.shrink(4.0) } else { rect };
        let (mut leaves, mut handles) = (Vec::new(), Vec::new());
        self.tabs[ti].root.layout(area, gap, &mut leaves, &mut handles);
        self.merge_target_last = self.tab_drag.as_ref().and_then(|d| self.merge_target(d.pos));
        self.leaf_rects = leaves.clone();
        self.head_rects.clear();
        self.sub_reorder = None;
        let active = self.tabs[ti].active;
        for (pi, r) in &leaves {
            if *pi >= self.tabs[ti].panes.len() {
                continue;
            }
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(*r), |ui| {
                ui.set_clip_rect(*r);
                self.ui_pane(ui, ti, *pi, acts);
            });
            if multi {
                let on = *pi == active;
                ui.painter().rect_stroke(*r, 8.0, Stroke::new(1.0, if on { th.accent.gamma_multiply(0.7) } else { th.line }));
            }
        }
        let tid = self.tabs[ti].id;
        for h in handles {
            let resp = ui.interact(h.hit, Id::new(("split", tid, h.path.clone())), Sense::drag());
            let horiz = h.dir == Dir::H;
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(if horiz { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
            }
            let k = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() || resp.dragged(), self.at(0.12));
            if k > 0.0 {
                let line = if horiz { Rect::from_center_size(h.hit.center(), vec2(3.0, h.hit.height() - 24.0)) } else { Rect::from_center_size(h.hit.center(), vec2(h.hit.width() - 24.0, 3.0)) };
                ui.painter().rect_filled(line, 2.0, th.accent.gamma_multiply(k));
            }
            if resp.dragged() {
                if let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
                    let (pos, start, len) = if horiz { (p.x, h.parent.left(), h.parent.width()) } else { (p.y, h.parent.top(), h.parent.height()) };
                    let lo = (tiling::MIN_SIZE / len).min(0.45);
                    let ratio = ((pos - start) / len).clamp(lo, 1.0 - lo);
                    self.tabs[ti].root.set_ratio(&h.path, ratio);
                }
            }
        }
    }

    /// Schnellleiste: Knöpfe in der Reihenfolge und Auswahl aus den Einstellungen. Alles wirkt im Ordner
    /// dieser Kachel bzw. auf ihre Auswahl – auch in Suchergebnissen.
    fn ui_cmdbar(&mut self, ui: &mut egui::Ui, pn: &Pane, acts: &mut Vec<Act>) {
        let th = self.th;
        let st = self.st.clone();
        let undo = self.history.undo.last().map(|o| o.label());
        let redo = self.history.redo.last().map(|o| o.label());
        let paths = pane_targets(pn);
        let has_sel = !paths.is_empty();
        let searching = pn.search.is_some();
        let can_paste = (self.clip.is_some() || clip::has_files()) && !pn.trash;
        let keys = self.keymap.clone();
        let git_here = self.git_st.is_some();
        let branch = self.git_st.as_ref().map(|g| g.branch.clone()).unwrap_or_default();
        let eds = self.ed_paths.clone();
        let backends = if st.ai_enabled { self.backends() } else { Vec::new() };
        let dual = self.dual_now;
        let tip = |label: &str, id: &str| {
            let k = keys.label(id);
            if k.is_empty() {
                label.to_string()
            } else {
                format!("{label} ({k})")
            }
        };
        let visible: Vec<String> = st.quickbar.iter().filter(|q| q.1).map(|q| q.0.clone()).collect();
        egui::ScrollArea::horizontal()
            .id_salt("cmdbar")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                    ui.add_space(6.0);
                    // Trenner nur zwischen Knöpfen, nie doppelt oder am Rand
                    let mut sep_pending = false;
                    let mut any = false;
                    for id in &visible {
                        if id.starts_with("sep") {
                            sep_pending = any;
                            continue;
                        }
                        // ausgeblendete Knöpfe (z. B. Git ohne Repository, KI aus) erzeugen keinen Trenner
                        let shown = match id.as_str() {
                            "git" => git_here,
                            "ai" => st.ai_enabled,
                            "openin" => !eds.is_empty(),
                            _ => true,
                        };
                        if !shown {
                            continue;
                        }
                        if sep_pending {
                            ui::sep_v(ui, &th);
                            sep_pending = false;
                        }
                        any = true;
                        match id.as_str() {
                            "new" => {
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Plus), "Neu", !searching && !pn.trash, true);
                                dropdown(ui, &r, ui.id().with("dd_new"), |ui| {
                                    if menu_item(ui, "Ordner", &keys.label("file.newfolder"), true) {
                                        acts.push(Act::NewFolder);
                                    }
                                    if menu_item(ui, "Textdokument", "", true) {
                                        acts.push(Act::NewFile);
                                    }
                                });
                            }
                            "newfolder" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::NewFolder), "", !searching && !pn.trash, false).on_hover_text(tip("Neuer Ordner", "file.newfolder")).clicked() {
                                    acts.push(Act::NewFolder);
                                }
                            }
                            "cut" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::Cut), "", has_sel, false).on_hover_text("Ausschneiden (Strg+X)").clicked() {
                                    acts.push(Act::SetClip(paths.clone(), true));
                                }
                            }
                            "copy" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::Copy), "", has_sel, false).on_hover_text("Kopieren (Strg+C)").clicked() {
                                    acts.push(Act::SetClip(paths.clone(), false));
                                }
                            }
                            "paste" => {
                                let t = format!("In „{}“ einfügen (Strg+V)", pn.title());
                                if ui::cmd_btn(ui, &th, Some(Ic::Paste), "", can_paste, false).on_hover_text(t).clicked() {
                                    acts.push(Act::PasteInto(pn.cwd.clone()));
                                }
                            }
                            "rename" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::Rename), "", paths.len() == 1 && !searching, false).on_hover_text(tip("Umbenennen", "file.rename")).clicked() {
                                    acts.push(Act::Rename);
                                }
                            }
                            "trash" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::Trash), "", has_sel, false).on_hover_text(tip("In den Papierkorb", "file.trash")).clicked() {
                                    acts.push(Act::Trash(paths.clone()));
                                }
                            }
                            "undo" => {
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Undo), "", undo.is_some(), false);
                                if r.clicked() {
                                    acts.push(Act::Undo);
                                }
                                r.on_hover_text(undo.as_ref().map_or("Rückgängig".into(), |l| format!("Rückgängig: {l} ({})", keys.label("file.undo"))));
                            }
                            "redo" => {
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Redo), "", redo.is_some(), false);
                                if r.clicked() {
                                    acts.push(Act::Redo);
                                }
                                r.on_hover_text(redo.as_ref().map_or("Wiederholen".into(), |l| format!("Wiederholen: {l} ({})", keys.label("file.redo"))));
                            }
                            "sort" => {
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Sort), "Sortieren", !searching, true);
                                dropdown(ui, &r, ui.id().with("dd_sort"), |ui| menu_sort(ui, pn, &st, acts));
                            }
                            "view" => {
                                let vic = if matches!(pn.mode, ViewMode::Details | ViewMode::List) { Ic::List } else { Ic::Grid };
                                let r = ui::cmd_btn(ui, &th, Some(vic), "Ansicht", !searching, true);
                                dropdown(ui, &r, ui.id().with("dd_view"), |ui| {
                                    ui::menu_label(ui, "Ansicht");
                                    menu_view(ui, pn, acts);
                                    ui::menu_sep(ui);
                                    ui::menu_label(ui, "Gruppieren nach");
                                    menu_group(ui, pn, acts);
                                    ui::menu_sep(ui);
                                    if ui::menu_entry(ui, Some(Ic::Eye), "Versteckte Dateien", &keys.label("view.hidden"), true, st.show_hidden) {
                                        acts.push(Act::Cmd("view.hidden"));
                                    }
                                });
                            }
                            "filter" => {
                                let flabel = if pn.filter.active() { "Filter aktiv" } else { "Filter" };
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Search), flabel, !searching, true);
                                dropdown(ui, &r, ui.id().with("dd_filter"), |ui| menu_filter(ui, pn, acts));
                            }
                            "select" => {
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Select), "Auswahl", !searching, true);
                                dropdown(ui, &r, ui.id().with("dd_select"), |ui| {
                                    if menu_item(ui, "Alles auswählen", &keys.label("file.selectall"), true) {
                                        acts.push(Act::SelectAll);
                                    }
                                    if ui::menu_entry(ui, Some(Ic::Select), "Auswahl umkehren", &keys.label("file.invert"), true, false) {
                                        acts.push(Act::Cmd("file.invert"));
                                    }
                                    if ui::menu_entry(ui, Some(Ic::Select), "Nach Regeln auswählen …", &keys.label("file.selectrules"), true, false) {
                                        acts.push(Act::SelectRules);
                                    }
                                });
                            }
                            "tools" => {
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Tools), "Werkzeuge", true, true);
                                dropdown(ui, &r, ui.id().with("dd_tools"), |ui| tools_items(ui, acts));
                            }
                            "terminal" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::Terminal), "", true, false).on_hover_text(tip("Terminal hier öffnen", "term.new")).clicked() {
                                    acts.push(Act::Terminal(None, pn.cwd.clone()));
                                }
                            }
                            "git" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::Git), &branch, true, false).on_hover_text(tip("Git-Repository anzeigen", "git.view")).clicked() {
                                    acts.push(Act::GitView(pn.cwd.clone()));
                                }
                            }
                            "ai" => {
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Spark), "KI", true, true);
                                dropdown(ui, &r, ui.id().with("dd_ai"), |ui| ai_menu(ui, &pn.cwd, &keys, &backends, acts));
                            }
                            "split" => {
                                if ui::tool_btn(ui, &th, Ic::Split, &tip(if dual { "Teilung aufheben" } else { "Ansicht teilen" }, "win.split"), true, dual).clicked() {
                                    acts.push(Act::ToggleDual);
                                }
                            }
                            "share" => {
                                let any_file = paths.iter().any(|p| fsx::is_file(&p));
                                if ui::cmd_btn(ui, &th, Some(Ic::Share), "", any_file, false).on_hover_text(tip("Teilen – LocalSend, Upload-Links, Messenger", "file.share")).clicked() {
                                    acts.push(Act::Share(paths.clone()));
                                }
                            }
                            "props" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::Info), "", true, false).on_hover_text("Eigenschaften").clicked() {
                                    acts.push(Act::Props(if paths.len() == 1 { paths[0].clone() } else { pn.cwd.clone() }));
                                }
                            }
                            "copypath" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::Copy), "Pfad", true, false).on_hover_text("Pfad der Auswahl bzw. des Ordners kopieren").clicked() {
                                    let t = if paths.is_empty() { pn.cwd.display().to_string() } else { paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n") };
                                    acts.push(Act::CopyText(t));
                                }
                            }
                            "refresh" => {
                                if ui::cmd_btn(ui, &th, Some(Ic::Refresh), "", true, false).on_hover_text(tip("Aktualisieren", "go.refresh")).clicked() {
                                    acts.push(Act::Refresh);
                                }
                            }
                            "compress" => {
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Docs), "Komprimieren", has_sel, true);
                                dropdown(ui, &r, ui.id().with("dd_zip"), |ui| {
                                    for f in Format::ALL {
                                        if menu_item(ui, f.label(), "", true) {
                                            acts.push(Act::Compress(paths.clone(), f));
                                        }
                                    }
                                });
                            }
                            "openin" => {
                                let target = if paths.len() == 1 { paths[0].clone() } else { pn.cwd.clone() };
                                let r = ui::cmd_btn(ui, &th, Some(Ic::Code), "Öffnen in", true, true);
                                dropdown(ui, &r, ui.id().with("dd_openin"), |ui| {
                                    for (k, n, _) in &eds {
                                        if ui::menu_entry(ui, Some(Ic::Code), n, "", true, false) {
                                            acts.push(Act::OpenIn(k.clone(), target.clone()));
                                        }
                                    }
                                });
                            }
                            "hidden" => {
                                if ui::tool_btn(ui, &th, Ic::Eye, &tip("Versteckte Dateien", "view.hidden"), true, st.show_hidden).clicked() {
                                    acts.push(Act::Cmd("view.hidden"));
                                }
                            }
                            "preview" => {
                                if ui::tool_btn(ui, &th, Ic::Docs, &tip("Vorschau", "view.preview"), true, pn.preview_on).clicked() {
                                    acts.push(Act::Cmd("view.preview"));
                                }
                            }
                            _ => {}
                        }
                    }
                });
            });
        ui.add_space(2.0);
    }

    /// linke Kanten der Spalten Datum, Typ, Größe. Ist es zu schmal, behält der Name mindestens 180 px und
    /// es fallen nacheinander Typ, Datum und Größe weg (Breite 0)
    fn col_lefts(&self, left: f32, right: f32) -> [f32; 3] {
        let mut w = self.col_w;
        let room = right - left - 40.0 - 180.0;
        for k in [1, 0, 2] {
            if w.iter().sum::<f32>() <= room {
                break;
            }
            w[k] = 0.0;
        }
        let size_l = right - w[2];
        let type_l = size_l - w[1];
        [type_l - w[0], type_l, size_l]
    }

    fn list_header(&mut self, ui: &mut egui::Ui, pn: &Pane, acts: &mut Vec<Act>) {
        let th = self.th;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEAD_H), Sense::hover());
        let right = rect.right() - 12.0;
        let [dl, tl, sl] = self.col_lefts(rect.left(), right);
        let cells = [
            (SortBy::Name, "Name", rect.left() + 12.0, dl - 8.0),
            (SortBy::Date, "Änderungsdatum", dl, tl),
            (SortBy::Type, "Typ", tl, sl),
            (SortBy::Size, "Größe", sl, right),
        ];
        for (k, (s, title, l, r)) in cells.iter().enumerate() {
            if k > 0 && *r - *l < 1.0 {
                continue;
            }
            let cell = Rect::from_min_max(
                pos2(*l, rect.top()),
                pos2((*r).max(*l + 10.0), rect.bottom()),
            );
            let resp = ui.interact(cell, ui.id().with(("hdr", k)), Sense::click());
            if resp.hovered() {
                ui.painter()
                    .rect_filled(cell.shrink2(vec2(0.0, 3.0)), 4.0, th.hover);
            }
            let (anchor, x) = if k == 3 {
                (Align2::RIGHT_CENTER, *r - 6.0)
            } else {
                (Align2::LEFT_CENTER, *l + 6.0)
            };
            ui.painter().with_clip_rect(cell).text(
                pos2(x, rect.center().y + 3.0),
                anchor,
                title,
                FontId::proportional(13.0),
                th.sub,
            );
            if pn.sort == *s {
                let cx = if k == 3 { *r - 20.0 } else { *l + 30.0 };
                let (y0, y1) = if pn.asc { (3.0, 0.0) } else { (0.0, 3.0) };
                let c = pos2(cx, rect.top() + 7.0);
                let st = Stroke::new(1.3, th.sub);
                ui.painter()
                    .line_segment([c + vec2(-4.0, y0), c + vec2(0.0, y1)], st);
                ui.painter()
                    .line_segment([c + vec2(0.0, y1), c + vec2(4.0, y0)], st);
            }
            if resp.clicked() {
                acts.push(Act::SortKey(*s));
            }
            if k >= 1 {
                let h = Rect::from_min_max(
                    pos2(*l - 4.0, rect.top() + 4.0),
                    pos2(*l + 4.0, rect.bottom() - 4.0),
                );
                let hr = ui.interact(h, ui.id().with(("hdrh", k)), Sense::drag());
                if hr.hovered() || hr.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if hr.dragged() {
                    self.col_w[k - 1] = (self.col_w[k - 1] - hr.drag_delta().x).clamp(60.0, 420.0);
                }
                ui.painter().line_segment(
                    [
                        pos2(*l - 0.5, rect.top() + 8.0),
                        pos2(*l - 0.5, rect.bottom() - 8.0),
                    ],
                    Stroke::new(1.0, th.line),
                );
            }
        }
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, th.line),
        );
    }

    fn ui_list(&mut self, ui: &mut egui::Ui, pn: &mut Pane, acts: &mut Vec<Act>) {
        let th = self.th;
        ui.spacing_mut().item_spacing.y = 0.0;
        if pn.mode == ViewMode::Details {
            self.list_header(ui, pn, acts);
        }
        let area = ui.available_rect_before_wrap();
        let bg = ui.interact(area, ui.id().with("listbg"), Sense::click());
        if bg.clicked() {
            pn.sel.clear();
            pn.sel_dirty = true;
            if pn.rename.is_some() {
                acts.push(Act::CommitRename);
            }
        }
        bg.context_menu(|ui| self.bg_menu(ui, pn, acts));
        self.zones.push((area, Dest::Dir(pn.cwd.clone())));
        // Statusmeldungen im leeren Bereich
        let msg = if let Some(e) = &pn.err {
            Some(e.clone())
        } else if pn
            .loading_since
            .is_some_and(|t| t.elapsed() > Duration::from_millis(150))
        {
            Some("Wird geladen …".to_string())
        } else if pn.view.is_empty() && !pn.is_loading() {
            Some(if pn.filter.active() {
                "Keine Elemente entsprechen dem Filter.".to_string()
            } else if pn.query.trim().is_empty() {
                "Dieser Ordner ist leer.".to_string()
            } else {
                "Keine Treffer in diesem Ordner.".to_string()
            })
        } else {
            None
        };
        if let Some(m) = msg {
            ui.painter().text(
                pos2(area.center().x, area.top() + 50.0),
                Align2::CENTER_CENTER,
                m,
                FontId::proportional(14.0),
                th.sub,
            );
        }
        if pn.mode == ViewMode::List {
            self.ui_hlist(ui, pn, acts);
        } else {
            self.ui_vlist(ui, pn, acts);
        }
        pn.refresh_sel_size();
    }

    /// Details und Raster: senkrecht scrollende, virtuelle Liste mit optionalen Gruppenköpfen.
    fn ui_vlist(&mut self, ui: &mut egui::Ui, pn: &mut Pane, acts: &mut Vec<Act>) {
        let th = self.th;
        let mode = pn.mode;
        let width = ui.available_width();
        let grid = mode.cell();
        let (cols, cell_h) = match grid {
            Some((cw, ch, _)) => ((((width - 12.0) / cw).floor() as usize).max(1), ch),
            None => (1, ROW_H),
        };
        pn.cols = cols;
        pn.build_layout(cols, cell_h, GROUP_H);
        let total = pn.layout.total;
        let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(y) = pn.pending_scroll.take() {
            scroll = scroll.vertical_scroll_offset(y);
        }
        let out = scroll.show_viewport(ui, |ui, vp| {
            let (base, _) = ui.allocate_exact_size(vec2(ui.available_width(), total.max(1.0)), Sense::hover());
            let first = pn.layout.rows.partition_point(|r| r.y + r.h < vp.min.y);
            let last = pn.layout.rows.partition_point(|r| r.y <= vp.max.y);
            let cw = if grid.is_some() { (base.width() - 12.0) / cols as f32 } else { base.width() };
            for ri in first..last {
                let (y, h, from, to, head) = {
                    let r = &pn.layout.rows[ri];
                    (r.y, r.h, r.from, r.to, r.head.clone())
                };
                if let Some((label, a, b)) = head {
                    let hr = Rect::from_min_size(base.min + vec2(0.0, y), vec2(base.width(), h));
                    let hresp = ui.interact(hr, ui.id().with(("grp", a)), Sense::click());
                    if hresp.hovered() {
                        ui.painter().rect_filled(hr.shrink2(vec2(4.0, 2.0)), 5.0, th.hover);
                    }
                    let cy = hr.center().y + 3.0;
                    let lr = ui.painter().text(
                        pos2(hr.left() + 16.0, cy),
                        Align2::LEFT_CENTER,
                        &*label,
                        FontId::proportional(14.5),
                        th.text,
                    );
                    let cr = ui.painter().text(
                        pos2(lr.right() + 8.0, cy),
                        Align2::LEFT_CENTER,
                        format!("({})", b - a),
                        FontId::proportional(12.5),
                        th.sub,
                    );
                    ui.painter().line_segment(
                        [pos2(cr.right() + 12.0, cy), pos2(hr.right() - 16.0, cy)],
                        Stroke::new(1.0, th.line),
                    );
                    if hresp.clicked() {
                        let ctrl = ui.input(|i| i.modifiers.command);
                        pn.select_range(a, b, ctrl);
                    }
                } else {
                    for pos in from..to.min(pn.view.len()) {
                        let k = (pos - from) as f32;
                        let rect = if grid.is_some() {
                            Rect::from_min_size(base.min + vec2(6.0 + k * cw, y), vec2(cw, h))
                        } else {
                            Rect::from_min_size(base.min + vec2(0.0, y), vec2(base.width(), h))
                        };
                        self.draw_item(ui, pn, pos, rect, acts);
                    }
                }
            }
        });
        pn.scroll_y = out.state.offset.y;
        pn.view_h = out.inner_rect.height();
    }

    /// Liste: kompakte Zeilen, die spaltenweise von oben nach unten laufen; scrollt waagerecht.
    fn ui_hlist(&mut self, ui: &mut egui::Ui, pn: &mut Pane, acts: &mut Vec<Act>) {
        let n = pn.view.len();
        let avail_h = ui.available_height().max(LIST_ROW_H);
        let rpc = (((avail_h - 16.0) / LIST_ROW_H).floor() as usize).max(1);
        pn.rows_per_col = rpc;
        let ncols = n.div_ceil(rpc).max(1);
        let total_w = ncols as f32 * LIST_COL_W;
        let mut scroll = egui::ScrollArea::horizontal().auto_shrink([false, false]);
        if let Some(x) = pn.pending_scroll_x.take() {
            scroll = scroll.horizontal_scroll_offset(x);
        }
        let out = scroll.show_viewport(ui, |ui, vp| {
            let (base, _) = ui.allocate_exact_size(vec2(total_w, avail_h - 16.0), Sense::hover());
            let c0 = (vp.min.x / LIST_COL_W).floor().max(0.0) as usize;
            let c1 = ((vp.max.x / LIST_COL_W).ceil() as usize).min(ncols);
            for c in c0..c1 {
                for r in 0..rpc {
                    let pos = c * rpc + r;
                    if pos >= n {
                        break;
                    }
                    let rect = Rect::from_min_size(
                        base.min + vec2(c as f32 * LIST_COL_W, r as f32 * LIST_ROW_H),
                        vec2(LIST_COL_W - 4.0, LIST_ROW_H),
                    );
                    self.draw_item(ui, pn, pos, rect, acts);
                }
            }
        });
        pn.scroll_x = out.state.offset.x;
        pn.view_w = out.inner_rect.width();
        pn.view_h = out.inner_rect.height();
    }

    /// Ein Element in allen Ansichten (Details, Liste, Raster, Große Symbole).
    fn draw_item(&mut self, ui: &mut egui::Ui, pn: &mut Pane, pos: usize, rect: Rect, acts: &mut Vec<Act>) {
        let th = self.th;
        let mode = pn.mode;
        let Some(&idx) = pn.view.get(pos) else { return };
        if idx >= pn.items.len() {
            return;
        }
        let resp = ui.interact(rect, ui.id().with(("item", pos)), Sense::click_and_drag());
        let (name, dir, size, flags) = {
            let it = &pn.items[idx];
            (it.name.to_string(), it.dir, it.size, it.flags)
        };
        let selected = pn.sel.contains(name.as_str());
        let is_target = self.drag.is_some()
            && dir
            && !selected
            && matches!(&self.drop_target, Some(Dest::Dir(d)) if d.parent() == Some(pn.cwd.as_path()) && d.file_name().is_some_and(|f| f == name.as_str()));
        let hk = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() && self.drag.is_none() && !selected, self.at(0.1));
        let big = mode.cell().is_some();
        let inner = if big { rect.shrink(3.0) } else { rect.shrink2(vec2(4.0, 1.0)) };
        let rr = if big { 8.0 } else { 5.0 };
        if is_target {
            ui.painter().rect(inner, rr, th.sel, Stroke::new(1.0, th.accent));
        } else if selected {
            ui.painter().rect_filled(inner, rr, if resp.hovered() { th.sel.gamma_multiply(1.4) } else { th.sel });
        } else if hk > 0.0 {
            ui.painter().rect_filled(inner, rr, th.hover.gamma_multiply(hk));
        }
        let shown = display_name(&name, dir, &pn.cwd);
        let right = rect.right() - 12.0;
        let [dl, tl, sl] = self.col_lefts(rect.left(), right);
        let (ir, name_rect) = match mode {
            ViewMode::Details => {
                let ir = Rect::from_min_size(pos2(rect.left() + 14.0, rect.center().y - ICON / 2.0), vec2(ICON, ICON));
                let nr = Rect::from_min_max(pos2(ir.right() + 8.0, rect.top()), pos2((dl - 8.0).max(ir.right() + 20.0), rect.bottom()));
                (ir, nr)
            }
            ViewMode::List => {
                let ir = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 9.0), vec2(18.0, 18.0));
                let nr = Rect::from_min_max(pos2(ir.right() + 6.0, rect.top()), pos2(rect.right() - 6.0, rect.bottom()));
                (ir, nr)
            }
            _ => {
                let isz = mode.cell().map_or(48.0, |c| c.2);
                let ir = Rect::from_center_size(pos2(rect.center().x, rect.top() + 8.0 + isz / 2.0), vec2(isz, isz));
                let nr = Rect::from_min_max(pos2(rect.left() + 4.0, ir.bottom() + 4.0), pos2(rect.right() - 4.0, rect.bottom() - 3.0));
                (ir, nr)
            }
        };
        let full = pn.cwd.join(&name);
        let is_cut = self.clip.as_ref().is_some_and(|c| c.cut && c.paths.binary_search(&full).is_ok());
        {
            let eb = ext_buf(&name);
            let cwd = &pn.cwd;
            let kind = if dir { IconKind::Dir } else { IconKind::File };
            self.icons.paint(ui.painter(), ir, kind, &name, eb.as_str(), false, || cwd.join(&name));
            if is_cut {
                // ausgeschnitten: Symbol abgeschwächt (wie im Windows-Explorer)
                ui.painter().rect_filled(ir.expand(1.0), 0.0, th.bg.gamma_multiply(0.55));
            }
        }
        // Cloud: Sync-Status unten links am Symbol (nur online / auf dem Gerät / immer behalten)
        if self.in_cloud(&pn.cwd) {
            let st = if flags & pane::F_PINNED != 0 {
                cloud::Sync::Pinned
            } else if flags & pane::F_ONLINE != 0 {
                cloud::Sync::Online
            } else {
                cloud::Sync::Local
            };
            cloud::badge(ui.painter(), ir, st, th.bg);
        }
        // Git-Status: farbiger Buchstabe an der Symbol-Ecke
        if let Some(gs) = self.git_st.as_ref().and_then(|g| g.mark_for(&full)) {
            let c = gitui::status_color(gs, &th);
            let b = Rect::from_center_size(ir.right_bottom() - vec2(1.0, 2.0), vec2(11.0, 11.0));
            ui.painter().rect_filled(b, 3.0, th.bg);
            ui.painter().text(b.center(), Align2::CENTER_CENTER, gs.letter(), FontId::proportional(9.5), c);
        }
        let editing = pn.rename.as_ref().is_some_and(|r| r.orig == name);
        let color = if is_cut || flags & (pane::F_HIDDEN | pane::F_SYSTEM) != 0 { th.sub } else { th.text };
        if editing {
            let mut rs = pn.rename.take().unwrap();
            let id = Id::new("fc_rename_edit");
            let er = if big {
                Rect::from_min_size(name_rect.min, vec2(name_rect.width(), 24.0))
            } else {
                name_rect.shrink2(vec2(0.0, 2.0))
            };
            let r = ui.put(er, egui::TextEdit::singleline(&mut rs.buf).id(id).margin(Margin::symmetric(4.0, 3.0)));
            if rs.first {
                r.request_focus();
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
                    let all = rs.buf.chars().count();
                    let stem = if dir {
                        all
                    } else {
                        rs.buf.rfind('.').filter(|&p| p > 0).map_or(all, |p| rs.buf[..p].chars().count())
                    };
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(stem),
                    )));
                    st.store(ui.ctx(), id);
                }
                rs.first = false;
            } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                acts.push(Act::CancelRename);
            } else if ui.input(|i| i.key_pressed(Key::Enter)) || r.lost_focus() {
                acts.push(Act::CommitRename);
            }
            pn.rename = Some(rs);
        } else if big {
            // bis zu zwei Zeilen, zentriert, mit „…“
            let mut job = egui::text::LayoutJob::single_section(
                shown.to_string(),
                egui::TextFormat::simple(FontId::proportional(13.0), color),
            );
            job.wrap = egui::text::TextWrapping {
                max_width: name_rect.width(),
                max_rows: 2,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            job.halign = egui::Align::Center;
            let galley = ui.fonts(|f| f.layout_job(job));
            ui.painter().with_clip_rect(name_rect.expand2(vec2(0.0, 3.0))).galley(pos2(name_rect.center().x, name_rect.top()), galley, color);
        } else {
            ui.painter().with_clip_rect(name_rect).text(
                pos2(name_rect.left(), rect.center().y),
                Align2::LEFT_CENTER,
                &*shown,
                FontId::proportional(14.0),
                color,
            );
        }
        if mode == ViewMode::Details {
            let sub = FontId::proportional(13.0);
            {
                let s = pn.row_strings(idx);
                let p = ui.painter();
                p.with_clip_rect(Rect::from_min_max(pos2(dl, rect.top()), pos2(tl, rect.bottom())))
                    .text(pos2(dl + 6.0, rect.center().y), Align2::LEFT_CENTER, &*s.0, sub.clone(), th.sub);
                p.with_clip_rect(Rect::from_min_max(pos2(tl, rect.top()), pos2(sl, rect.bottom())))
                    .text(pos2(tl + 6.0, rect.center().y), Align2::LEFT_CENTER, &*s.1, sub.clone(), th.sub);
            }
            if right - sl < 1.0 {
                // Spalte ausgeblendet (schmales Fenster)
            } else if !dir {
                ui.painter().text(pos2(right - 6.0, rect.center().y), Align2::RIGHT_CENTER, size_col(size), sub, th.sub);
            } else if self.st.folder_sizes {
                // sofort Cache-/Zwischenwert, im Hintergrund genauer (≈ = Berechnung läuft), im Hintergrund genauer (≈ = Berechnung läuft)
                let txt = match self.dsize.get(&full) {
                    Some(sz) if sz.exact && !sz.partial => size_col(sz.bytes),
                    Some(sz) if sz.exact => format!("≥ {}", size_col(sz.bytes)),
                    Some(sz) => format!("≈ {}", size_col(sz.bytes)),
                    None => "…".into(),
                };
                ui.painter().text(pos2(right - 6.0, rect.center().y), Align2::RIGHT_CENTER, txt, sub, th.sub);
            }
        }
        if dir {
            self.zones.push((rect, Dest::Dir(pn.cwd.join(&name))));
        }
        // Interaktion
        if resp.drag_started() && pn.rename.is_none() {
            if !selected {
                pn.select_pos(pos, false, false);
            }
            self.drag = Some(pn.sel_paths());
        }
        if resp.clicked() {
            let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
            pn.select_pos(pos, shift, ctrl);
            if pn.rename.as_ref().is_some_and(|r| r.orig != name) {
                acts.push(Act::CommitRename);
            }
        }
        if resp.double_clicked() {
            let p = pn.cwd.join(&name);
            acts.push(if dir { Act::Go(p) } else { Act::OpenFile(p) });
        }
        if resp.middle_clicked() {
            acts.push(Act::TabOpen(pn.cwd.join(&name)));
        }
        if resp.secondary_clicked() && !selected {
            pn.select_pos(pos, false, false);
        }
        resp.context_menu(|ui| self.item_menu(ui, pn, acts));
    }

    // ── Suchergebnisse (Ordnersuche und PC-Suche) ──
    fn ui_results(&mut self, ui: &mut egui::Ui, pn: &mut Pane, acts: &mut Vec<Act>) {
        let th = self.th;
        ui.spacing_mut().item_spacing.y = 0.0;
        let building = self.build.is_some() || !self.queue.is_empty();
        let sv = pn.search.as_mut().unwrap();
        // Kopfzeile
        let (brect, bresp) =
            ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        ui.painter().rect_filled(brect, 0.0, th.panel);
        let mut text = format!(
            "{}  –  {}{} Treffer",
            sv.label,
            sv.total,
            if sv.capped { "+" } else { "" }
        );
        if sv.ms > 0.0 {
            text.push_str(&format!("  ({:.1} ms)", sv.ms));
        }
        if sv.live.is_some() {
            text.push_str("  –  durchsuche …");
        }
        if sv.kind == SearchKind::Pc && building {
            text.push_str("  –  Index wird aufgebaut, weitere Treffer folgen automatisch");
        }
        ui.painter()
            .with_clip_rect(brect.shrink2(vec2(40.0, 0.0)))
            .text(
                pos2(brect.left() + 14.0, brect.center().y),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional(13.0),
                th.sub,
            );
        let cx = pos2(brect.right() - 20.0, brect.center().y);
        let st = Stroke::new(1.4, th.sub);
        ui.painter()
            .line_segment([cx + vec2(-4.0, -4.0), cx + vec2(4.0, 4.0)], st);
        ui.painter()
            .line_segment([cx + vec2(-4.0, 4.0), cx + vec2(4.0, -4.0)], st);
        if bresp.clicked() {
            acts.push(Act::ClearSearch);
        }
        let (hrect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEAD_H), Sense::hover());
        let right = hrect.right() - 12.0;
        let folder_l = hrect.left() + (hrect.width() * 0.42).max(220.0);
        let type_l = (right - 190.0).max(folder_l + 60.0);
        for (t, x) in [
            ("Name", hrect.left() + 12.0),
            ("Ordner", folder_l),
            ("Typ", type_l),
        ] {
            ui.painter().text(
                pos2(x + 6.0, hrect.center().y),
                Align2::LEFT_CENTER,
                t,
                FontId::proportional(13.0),
                th.sub,
            );
        }
        ui.painter().line_segment(
            [hrect.left_bottom(), hrect.right_bottom()],
            Stroke::new(1.0, th.line),
        );
        let n = sv.hits.len();
        if n == 0 {
            let r = ui.available_rect_before_wrap();
            let m = if sv.live.is_some() {
                "Suche läuft …"
            } else {
                "Keine Treffer."
            };
            ui.painter().text(
                pos2(r.center().x, r.top() + 50.0),
                Align2::CENTER_CENTER,
                m,
                FontId::proportional(14.0),
                th.sub,
            );
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, n, |ui, range| {
                for pos in range {
                    let (path, name, dir, snippet) = match &sv.hits[pos].1 {
                        Hit::Idx(k, id) => {
                            let ix = &sv.idxs[*k as usize];
                            let id = *id as usize;
                            if id >= ix.len() {
                                // Zeile trotzdem belegen, sonst verrutschen alle folgenden
                                ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                                continue;
                            }
                            (ix.path(id), ix.name(id), ix.is_dir(id), None)
                        }
                        Hit::Live(h) => (
                            h.path.clone(),
                            h.path
                                .file_name()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            h.dir,
                            h.snippet.clone(),
                        ),
                    };
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
                    let inner = rect.shrink2(vec2(4.0, 1.0));
                    let selected = sv.sel.contains(&path);
                    if selected {
                        ui.painter().rect_filled(inner, 5.0, th.sel);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(inner, 5.0, th.hover);
                    }
                    let ir = Rect::from_min_size(
                        pos2(rect.left() + 14.0, rect.center().y - ICON / 2.0),
                        vec2(ICON, ICON),
                    );
                    let eb = ext_buf(&name);
                    self.icons.paint(
                        ui.painter(),
                        ir,
                        if dir { IconKind::Dir } else { IconKind::File },
                        &name,
                        eb.as_str(),
                        false,
                        || path.clone(),
                    );
                    let nr = Rect::from_min_max(
                        pos2(ir.right() + 8.0, rect.top()),
                        pos2(folder_l - 4.0, rect.bottom()),
                    );
                    ui.painter().with_clip_rect(nr).text(
                        pos2(nr.left(), rect.center().y),
                        Align2::LEFT_CENTER,
                        &name,
                        FontId::proportional(14.0),
                        th.text,
                    );
                    let parent = path
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    let fr = Rect::from_min_max(
                        pos2(folder_l + 6.0, rect.top()),
                        pos2(type_l - 4.0, rect.bottom()),
                    );
                    match &snippet {
                        // Inhaltstreffer: Zeile und Ausschnitt statt des Ordners (Ordner im Tooltip)
                        Some((line, text)) => {
                            ui.painter().with_clip_rect(fr).text(
                                pos2(fr.left(), rect.center().y),
                                Align2::LEFT_CENTER,
                                format!("Z. {line}: {text}"),
                                FontId::monospace(12.0),
                                th.accent,
                            );
                        }
                        None => {
                            ui.painter().with_clip_rect(fr).text(
                                pos2(fr.left(), rect.center().y),
                                Align2::LEFT_CENTER,
                                &parent,
                                FontId::proportional(13.0),
                                th.sub,
                            );
                        }
                    }
                    let resp = if snippet.is_some() { resp.on_hover_text(&parent) } else { resp };
                    ui.painter().text(
                        pos2(type_l + 6.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        type_name(eb.as_str(), dir),
                        FontId::proportional(13.0),
                        th.sub,
                    );
                    // Auswahl: Klick = nur diesen, Strg = hinzufügen/entfernen, Umschalt = Bereich ab Anker
                    if resp.clicked() {
                        let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
                        if shift {
                            let a = sv.anchor.as_ref().and_then(|ap| (0..sv.hits.len()).find(|&i| hit_path(sv, i).as_ref() == Some(ap))).unwrap_or(pos);
                            let (lo, hi) = (a.min(pos), a.max(pos));
                            if !ctrl {
                                sv.sel.clear();
                            }
                            let range: Vec<PathBuf> = (lo..=hi).filter_map(|i| hit_path(sv, i)).collect();
                            sv.sel.extend(range);
                        } else if ctrl {
                            if !sv.sel.remove(&path) {
                                sv.sel.insert(path.clone());
                            }
                            sv.anchor = Some(path.clone());
                        } else {
                            sv.sel.clear();
                            sv.sel.insert(path.clone());
                            sv.anchor = Some(path.clone());
                        }
                    }
                    if resp.secondary_clicked() && !selected {
                        sv.sel.clear();
                        sv.sel.insert(path.clone());
                        sv.anchor = Some(path.clone());
                    }
                    // Ziehen: alle ausgewählten Treffer (wie in der Dateiliste)
                    if resp.drag_started() {
                        if !selected {
                            sv.sel.clear();
                            sv.sel.insert(path.clone());
                        }
                        self.drag = Some((0..sv.hits.len()).filter_map(|i| hit_path(sv, i)).filter(|p| sv.sel.contains(p)).collect());
                    }
                    let targets: Vec<PathBuf> = if sv.sel.contains(&path) {
                        (0..sv.hits.len()).filter_map(|i| hit_path(sv, i)).filter(|p| sv.sel.contains(p)).collect()
                    } else {
                        vec![path.clone()]
                    };
                    let many = targets.len() > 1;
                    if resp.double_clicked() {
                        acts.push(if dir {
                            Act::Go(path.clone())
                        } else {
                            Act::OpenFile(path.clone())
                        });
                    }
                    if resp.middle_clicked() {
                        acts.push(Act::TabOpen(path.clone()));
                    }
                    resp.context_menu(|ui| {
                        if many {
                            ui::menu_label(ui, &format!("{} Treffer ausgewählt", targets.len()));
                        }
                        if menu_item(ui, "Öffnen", "", !many) {
                            acts.push(Act::Open(path.clone()));
                        }
                        if menu_item(ui, "Ordner anzeigen", "", !many) {
                            acts.push(Act::Reveal(path.clone()));
                        }
                        ui.separator();
                        if menu_item(ui, "Kopieren", "Strg+C", true) {
                            acts.push(Act::SetClip(targets.clone(), false));
                        }
                        if menu_item(ui, "Ausschneiden", "Strg+X", true) {
                            acts.push(Act::SetClip(targets.clone(), true));
                        }
                        if menu_item(ui, "Pfad kopieren", "", true) {
                            acts.push(Act::CopyText(targets.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n")));
                        }
                        if menu_item(ui, "Teilen …", "", targets.iter().any(|p| fsx::is_file(&p))) {
                            acts.push(Act::Share(targets.clone()));
                        }
                        if extract::is_archive(&name) {
                            ui.separator();
                            if menu_item(ui, "Hier entpacken", "", true) {
                                acts.push(Act::Extract(vec![path.clone()], false));
                            }
                        }
                        ui.separator();
                        if menu_item(ui, "Löschen", "Entf", true) {
                            acts.push(Act::Trash(targets.clone()));
                        }
                        if menu_item(ui, "Endgültig löschen …", "Umschalt+Entf", true) {
                            acts.push(Act::DeletePerm(targets.clone()));
                        }
                        if menu_item(ui, "Eigenschaften", "", !many) {
                            acts.push(Act::Props(path.clone()));
                        }
                    });
                }
            });
    }

    fn ui_status(&mut self, ui: &mut egui::Ui) {
        let th = self.th;
        let txt = {
            let t = &mut self.tabs[self.cur];
            let a = t.active;
            let pn = &mut t.panes[a];
            pn.refresh_sel_size();
            if let Some(t) = &pn.ctrash {
                format!("Papierkorb – {}", t.name)
            } else if pn.start {
                "Startseite".to_string()
            } else if let Some(sv) = &pn.search {
                if sv.sel.is_empty() {
                    format!("{} Treffer", sv.hits.len())
                } else {
                    format!("{} Treffer   |   {} ausgewählt", sv.hits.len(), sv.sel.len())
                }
            } else if pn.sel.is_empty() {
                format!("{} Elemente{}", pn.view.len(), if pn.filter.active() { " (gefiltert)" } else { "" })
            } else if pn.sel_size > 0 {
                format!(
                    "{} Elemente   |   {} ausgewählt   {}",
                    pn.view.len(),
                    pn.sel.len(),
                    human(pn.sel_size)
                )
            } else {
                format!(
                    "{} Elemente   |   {} ausgewählt",
                    pn.view.len(),
                    pn.sel.len()
                )
            }
        };
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(txt).size(12.5).color(th.sub));
            if !self.status.is_empty() {
                ui.label(
                    egui::RichText::new(format!("   •   {}", self.status))
                        .size(12.5)
                        .color(th.sub),
                );
            }
            update_ui(ui, th, &self.updater, &self.update_quit, true);
            for j in &self.jobs {
                ui.spinner();
                let done = j.prog.as_ref().map(|p| p.load(Relaxed));
                let n = match (done, j.total) {
                    (Some(d), t) if t > 0 && j.pct => {
                        let frac = (d as f32 / t as f32).clamp(0.0, 1.0);
                        ui.add(egui::ProgressBar::new(frac).desired_width(140.0).desired_height(6.0));
                        format!(" {} %", (frac * 100.0).floor() as u32)
                    }
                    (Some(d), t) if t > 0 => {
                        let frac = (d as f32 / t as f32).clamp(0.0, 1.0);
                        ui.add(egui::ProgressBar::new(frac).desired_width(90.0).desired_height(6.0));
                        format!(" {d}/{t}")
                    }
                    (Some(d), _) => format!(" {d}"),
                    _ => String::new(),
                };
                let cancelled = j.cancel.as_ref().is_some_and(|c| c.load(Relaxed));
                ui.label(
                    egui::RichText::new(format!("{}{}{}", j.label, n, if cancelled { " – wird abgebrochen …" } else { "" }))
                        .size(12.5)
                        .color(th.sub),
                );
                if let Some(c) = &j.cancel {
                    if !cancelled && ui.small_button("Abbrechen").on_hover_text("Nach dem aktuellen Element anhalten – nichts geht verloren").clicked() {
                        c.store(true, Relaxed);
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                if let Some(b) = &self.build {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(format!(
                            "Index {} … {}",
                            b.root.display(),
                            b.progress.load(Relaxed)
                        ))
                        .size(12.5)
                        .color(th.sub),
                    );
                } else {
                    let total: usize = self
                        .slots
                        .iter()
                        .filter_map(|s| s.index.as_ref())
                        .map(|i| i.len())
                        .sum();
                    if total > 0 {
                        ui.label(
                            egui::RichText::new(format!("Index: {total} Einträge"))
                                .size(12.5)
                                .color(th.sub),
                        );
                    }
                }
            });
        });
    }

    fn ui_windows(&mut self, ctx: &egui::Context) {
        // Namenskonflikt beim Kopieren/Verschieben
        if let Some(pd) = self.pending.take() {
            let mut decision: Option<Option<Policy>> = None;
            let sub = self.th.sub;
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                decision = Some(None);
            }
            egui::Window::new("Namenskonflikt")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(520.0);
                    let dn = pd.dest.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| pd.dest.display().to_string());
                    ui.label(format!(
                        "Im Zielordner »{dn}« gibt es bereits {} Element(e) mit gleichem Namen:",
                        pd.total
                    ));
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                        for (n, new, old) in &pd.lines {
                            ui.label(egui::RichText::new(n).strong());
                            ui.label(egui::RichText::new(format!("   Neu: {new}")).size(12.5).color(sub));
                            ui.label(egui::RichText::new(format!("   Vorhanden: {old}")).size(12.5).color(sub));
                            ui.add_space(4.0);
                        }
                        if pd.total > pd.lines.len() {
                            ui.label(format!("… und {} weitere", pd.total - pd.lines.len()));
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("„Ersetzen“ legt die vorhandenen Elemente in den Papierkorb – mit Strg+Z lässt sich das rückgängig machen.")
                            .size(12.0)
                            .color(sub),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Ersetzen").clicked() {
                            decision = Some(Some(Policy::Replace));
                        }
                        if ui.button("Beide behalten").clicked() {
                            decision = Some(Some(Policy::KeepBoth));
                        }
                        if ui.button("Überspringen").clicked() {
                            decision = Some(Some(Policy::Skip));
                        }
                        if ui.button("Abbrechen").clicked() {
                            decision = Some(None);
                        }
                    });
                });
            match decision {
                Some(Some(p)) => self.start_transfer(pd.srcs, pd.dest, pd.copy, p),
                Some(None) => self.status = "Vorgang abgebrochen".into(),
                None => self.pending = Some(pd),
            }
        }
        // Endgültig löschen: immer bestätigen
        if let Some(v) = self.confirm_del.take() {
            let mut decision: Option<bool> = None;
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                decision = Some(false);
            }
            egui::Window::new("Endgültig löschen?")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!(
                        "{} Element(e) werden endgültig gelöscht. Das lässt sich NICHT rückgängig machen und sie landen nicht im Papierkorb.",
                        v.len()
                    ));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Endgültig löschen").clicked() {
                            decision = Some(true);
                        }
                        if ui.button("Abbrechen").clicked() {
                            decision = Some(false);
                        }
                    });
                });
            match decision {
                Some(true) => self.spawn_delete_perm(v),
                Some(false) => {}
                None => self.confirm_del = Some(v),
            }
        }
        // Seitenleiste umbenennen
        if let Some((i, mut buf)) = self.side_rename.take() {
            let mut keep = true;
            egui::Window::new("Umbenennen")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let r = ui.text_edit_singleline(&mut buf);
                    r.request_focus();
                    let enter = ui.input(|i| i.key_pressed(Key::Enter));
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() || enter {
                            if let Some(e) = self.side.pins.get_mut(i) {
                                if !buf.trim().is_empty() {
                                    e.label = buf.trim().to_string();
                                }
                            }
                            self.side.save();
                            keep = false;
                        }
                        if ui.button("Abbrechen").clicked()
                            || ui.input(|i| i.key_pressed(Key::Escape))
                        {
                            keep = false;
                        }
                    });
                });
            if keep {
                self.side_rename = Some((i, buf));
            }
        }
        // Eigenschaften
        let mut close = false;
        if let Some(p) = &self.props {
            let sub = self.th.sub;
            let mut open = true;
            egui::Window::new("Eigenschaften")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    egui::Grid::new("props")
                        .num_columns(2)
                        .spacing([16.0, 6.0])
                        .show(ui, |ui| {
                            let row = |ui: &mut egui::Ui, k: &str, v: String| {
                                ui.label(egui::RichText::new(k).color(sub));
                                ui.label(v);
                                ui.end_row();
                            };
                            row(ui, "Name", p.name.clone());
                            row(ui, "Typ", p.kind.clone());
                            row(
                                ui,
                                "Ort",
                                p.path
                                    .parent()
                                    .map(|x| x.display().to_string())
                                    .unwrap_or_default(),
                            );
                            let size = p.size.load(Relaxed);
                            let sz = if p.is_dir {
                                format!(
                                    "{} ({} Dateien){}",
                                    human(size),
                                    p.files.load(Relaxed),
                                    if p.done.load(Relaxed) {
                                        ""
                                    } else {
                                        "  – wird berechnet …"
                                    }
                                )
                            } else {
                                format!("{} ({} Bytes)", human(size), size)
                            };
                            row(ui, "Größe", sz);
                            row(ui, "Erstellt", p.created.clone());
                            row(ui, "Geändert", p.modified.clone());
                        });
                });
            if !p.done.load(Relaxed) {
                ctx.request_repaint_after(Duration::from_millis(150));
            }
            close = !open;
        }
        if close {
            self.props = None;
        }
        self.ui_settings(ctx);
        self.ui_folder_icon(ctx);
        {
            let mut acts2 = Vec::new();
            self.ui_cloud_setup(ctx, &mut acts2);
            for a in acts2 {
                self.apply(a);
            }
        }
        // Zusammenfassungen; Klick auf eine Datei zeigt sie im Explorer
        let th = self.th;
        let mut reveal: Option<PathBuf> = None;
        self.summaries.retain_mut(|s| s.ui(ctx, &th, &mut reveal));
        if let Some(p) = reveal {
            if let Some(d) = p.parent() {
                self.ai_go = Some(d.to_path_buf());
                self.ai_select = p.file_name().map(|n| vec![n.to_string_lossy().into_owned()]);
            }
        }
        if let Some(g) = &mut self.imggen {
            let th = self.th;
            if !g.ui(ctx, &th) {
                self.imggen = None;
            }
        }
    }

    /// Papierkorb als eigener „Ordner“ im Hauptbereich (mit Leeren/Wiederherstellen-Schaltflächen).
    fn ui_trash(&mut self, ui: &mut egui::Ui) {
        let Some(mut tw) = self.trash.take() else {
            self.load_trash();
            return;
        };
        let th = self.th;
        self.zones.push((ui.max_rect(), Dest::Trash));
        let mut reload = false;
        let mut restore = false;
        let mut purge_sel = false;
        let mut purge_all = false;
        {
            {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    let b = egui::Button::new(egui::RichText::new("Papierkorb leeren").color(Color32::WHITE))
                        .fill(Color32::from_rgb(196, 62, 54));
                    if ui.add_enabled(!tw.rows.is_empty(), b).clicked() {
                        tw.ask = Some(TrashAsk::Empty);
                    }
                    if ui.add_enabled(!tw.sel.is_empty(), egui::Button::new("Wiederherstellen")).clicked() {
                        restore = true;
                    }
                    if ui.add_enabled(!tw.sel.is_empty(), egui::Button::new("Endgültig löschen")).clicked() {
                        tw.ask = Some(TrashAsk::DeleteSel);
                    }
                    if ui.button("Aktualisieren").clicked() {
                        reload = true;
                    }
                    let total: u64 = tw.rows.iter().filter_map(|r| r.size).sum();
                    ui.label(
                        egui::RichText::new(format!("{} Elemente  •  {}", tw.rows.len(), human(total)))
                            .size(12.5)
                            .color(th.sub),
                    );
                });
                match tw.ask {
                    Some(TrashAsk::DeleteSel) | Some(TrashAsk::Empty) => {
                        let all = matches!(tw.ask, Some(TrashAsk::Empty));
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                Color32::from_rgb(220, 90, 80),
                                if all {
                                    "Alle Elemente endgültig löschen? Das ist nicht rückgängig zu machen."
                                } else {
                                    "Ausgewählte Elemente endgültig löschen? Das ist nicht rückgängig zu machen."
                                },
                            );
                            if ui.button("Ja, löschen").clicked() {
                                if all {
                                    purge_all = true;
                                } else {
                                    purge_sel = true;
                                }
                                tw.ask = None;
                            }
                            if ui.button("Abbrechen").clicked() {
                                tw.ask = None;
                            }
                        });
                    }
                    None => {}
                }
                ui.separator();
                if let Some(e) = &tw.err {
                    ui.colored_label(Color32::from_rgb(220, 90, 80), e);
                } else if tw.rx.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Wird geladen …");
                    });
                } else if tw.rows.is_empty() {
                    ui.label("Der Papierkorb ist leer.");
                }
                let n = tw.rows.len();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 24.0, n, |ui, range| {
                    for i in range {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                        let sel = tw.sel.contains(&i);
                        if sel {
                            ui.painter().rect_filled(rect, 4.0, th.sel);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 4.0, th.hover);
                        }
                        let r = &tw.rows[i];
                        let p = ui.painter().with_clip_rect(rect);
                        let f = FontId::proportional(13.5);
                        let w = rect.width();
                        p.with_clip_rect(Rect::from_min_max(rect.min, pos2(rect.left() + w * 0.34, rect.bottom())))
                            .text(pos2(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, &r.name, f.clone(), th.text);
                        p.with_clip_rect(Rect::from_min_max(pos2(rect.left() + w * 0.35, rect.top()), pos2(rect.left() + w * 0.72, rect.bottom())))
                            .text(pos2(rect.left() + w * 0.35, rect.center().y), Align2::LEFT_CENTER, r.parent.display().to_string(), f.clone(), th.sub);
                        p.text(
                            pos2(rect.left() + w * 0.73, rect.center().y),
                            Align2::LEFT_CENTER,
                            fmt_time(r.deleted.saturating_mul(1_000_000_000)),
                            f.clone(),
                            th.sub,
                        );
                        if let (Some(s), false) = (r.size, r.dir) {
                            p.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, human(s), f, th.sub);
                        }
                        if resp.clicked() {
                            let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
                            if shift && tw.anchor.is_some() {
                                let a = tw.anchor.unwrap();
                                tw.sel.clear();
                                for k in a.min(i)..=a.max(i) {
                                    tw.sel.insert(k);
                                }
                            } else if ctrl {
                                if !tw.sel.remove(&i) {
                                    tw.sel.insert(i);
                                }
                                tw.anchor = Some(i);
                            } else {
                                tw.sel.clear();
                                tw.sel.insert(i);
                                tw.anchor = Some(i);
                            }
                        }
                    }
                });
            }
        }
        if restore || purge_sel || purge_all {
            let rows = std::mem::take(&mut tw.rows);
            let sel = std::mem::take(&mut tw.sel);
            let picked: Vec<trashx::Row> = rows
                .into_iter()
                .enumerate()
                .filter(|(i, _)| purge_all || sel.contains(i))
                .map(|(_, r)| r)
                .collect();
            let label = if restore { "Stelle wieder her …" } else { "Lösche endgültig …" };
            self.run_job(label, None, move || {
                let refs: Vec<&trashx::Row> = picked.iter().collect();
                let msg = if restore {
                    let (ok, errs) = trashx::restore(&refs);
                    if errs.is_empty() {
                        format!("{ok} Element(e) wiederhergestellt")
                    } else {
                        format!("{ok} wiederhergestellt – {}", errs.join("; "))
                    }
                } else {
                    match trashx::purge(&refs) {
                        Ok(_) => format!("{} Element(e) endgültig gelöscht", refs.len()),
                        Err(e) => format!("Löschen fehlgeschlagen: {e}"),
                    }
                };
                JobDone { locked: None,  msg, hist: Hist::None }
            });
        }
        self.trash = Some(tw);
        if reload {
            self.load_trash();
        }
    }

    /// Übernimmt geänderte Einstellungen, speichert sie und wendet die Auswirkungen sofort an.
    fn set_settings(&mut self, st: Settings) {
        if st == self.st {
            return;
        }
        let ctx = self.ctx.clone();
        if st.icon_set != self.st.icon_set {
            self.icons.set_theme(matico::Set::from_key(&st.icon_set));
        }
        if st.sys_icons != self.st.sys_icons {
            self.icons.set_system(st.sys_icons, &ctx);
        }
        if st.theme != self.st.theme {
            theme_pref(&ctx, st.theme);
        }
        if st.animations != self.st.animations {
            ctx.style_mut(|s| s.animation_time = if st.animations { 0.12 } else { 0.0 });
        }
        if st.show_sidebar != self.st.show_sidebar {
            self.side_visible = st.show_sidebar;
        }
        if st.show_git && !self.st.show_git && self.git.is_empty() {
            self.git_rx = Some(probe_git(&ctx, git_roots(&self.side)));
        }
        let view_changed = st.show_hidden != self.st.show_hidden || st.show_system != self.st.show_system;
        let ff_changed = st.folders_first != self.st.folders_first;
        let editors_changed = st.editor_paths != self.st.editor_paths || st.editor_on != self.st.editor_on || st.custom_editors != self.st.custom_editors;
        let git_changed = st.git_marks != self.st.git_marks;
        if !st.show_recent && self.st.show_recent {
            // Verlauf ausgeschaltet: gespeicherten Verlauf entfernen (Datenschutz)
            self.recent.clear();
        }
        self.st = st;
        self.st.save();
        if editors_changed {
            self.ed_paths = find_editors(&self.st);
        }
        if git_changed {
            self.refresh_git();
        }
        if view_changed {
            self.refresh_views();
        }
        if ff_changed {
            let st = self.st.clone();
            each_pane(&mut self.tabs, |p| p.resort(&st));
        }
    }

    /// Ordner-Icon ändern: Vorlagen (durchsuchbar), eigenes Bild, zurücksetzen
    fn ui_folder_icon(&mut self, ctx: &egui::Context) {
        let Some((dir, query)) = &mut self.folder_icon else { return };
        let th = self.th;
        let mut open = true;
        let mut pick: Option<Option<foldericons::Spec>> = None;
        let cur = foldericons::get(dir);
        let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
        let icons = &mut self.icons;
        let group = &mut self.folder_icon_group;
        egui::Window::new(format!("Icon für „{name}“"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(vec2(560.0, 480.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Eigenes Bild …").on_hover_text("PNG, JPEG, ICO, SVG, WebP …").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("Bild", &["png", "jpg", "jpeg", "ico", "svg", "webp", "bmp", "gif"]).pick_file() {
                            pick = Some(Some(foldericons::Spec::Image(f)));
                        }
                    }
                    if ui.add_enabled(cur.is_some(), egui::Button::new("Zurücksetzen")).on_hover_text("Wieder das normale Ordnersymbol").clicked() {
                        pick = Some(None);
                    }
                    ui.add(egui::TextEdit::singleline(query).hint_text("Vorlagen suchen, z. B. images, music, git …").desired_width(ui.available_width()));
                });
                ui.add_space(4.0);
                // Vorschläge passend zum Inhalt (einmal je Ordner ermittelt)
                static SUGGEST: std::sync::Mutex<Option<(PathBuf, Vec<String>)>> = std::sync::Mutex::new(None);
                let sug = {
                    let mut g = SUGGEST.lock().unwrap_or_else(|e| e.into_inner());
                    if g.as_ref().is_none_or(|(p, _)| p != dir) {
                        let have = matico::presets("material-ordner");
                        let v = foldericons::suggest(dir).into_iter().filter(|n| have.contains(n)).collect();
                        *g = Some((dir.clone(), v));
                    }
                    g.as_ref().map(|x| x.1.clone()).unwrap_or_default()
                };
                if !sug.is_empty() && query.is_empty() {
                    ui.label(egui::RichText::new("Vorschläge für diesen Ordner").size(12.0).strong().color(th.sub));
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for n in &sug {
                            let spec = foldericons::Spec::Preset(n.clone());
                            let (r, resp) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::click());
                            let sel = cur.as_ref() == Some(&spec);
                            ui.painter().rect_filled(r, 6.0, if sel { th.sel } else if resp.hovered() { th.hover } else { th.field });
                            let ir = Rect::from_center_size(r.center() - vec2(0.0, 7.0), vec2(34.0, 34.0));
                            icons.paint_spec(ui.painter(), ir, &spec);
                            let label = foldericons::preset_label(n);
                            let short: String = label.chars().take(10).collect();
                            ui.painter().text(pos2(r.center().x, r.bottom() - 6.0), Align2::CENTER_BOTTOM, short, FontId::proportional(10.5), th.sub);
                            if resp.on_hover_text(format!("{label} – passt zum Inhalt")).clicked() {
                                pick = Some(Some(spec));
                            }
                        }
                    });
                    ui.add_space(2.0);
                }
                // Gruppen: Ordner/Symbole aus drei Icon-Themes und farbige Ordner
                ui.horizontal_wrapped(|ui| {
                    for (k, l) in matico::PRESET_GROUPS.iter().copied().chain([("farben", "Farben")]) {
                        if ui.selectable_label(*group == k, l).clicked() {
                            *group = k;
                        }
                    }
                });
                ui.separator();
                let q = query.to_lowercase();
                if *group == "farben" {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for c in foldericons::COLORS {
                            let spec = foldericons::Spec::Color(*c);
                            let (r, resp) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::click());
                            let sel = cur.as_ref() == Some(&spec);
                            if sel || resp.hovered() {
                                ui.painter().rect_filled(r, 6.0, if sel { th.sel } else { th.hover });
                            }
                            icons::paint_folder_color(ui.painter(), Rect::from_center_size(r.center(), vec2(40.0, 40.0)), *c);
                            if resp.on_hover_text(format!("#{c:06X}")).clicked() {
                                pick = Some(Some(spec));
                            }
                        }
                    });
                    return;
                }
                let presets: Vec<String> = matico::presets(group).into_iter().filter(|n| q.is_empty() || foldericons::preset_label(n).contains(&q)).collect();
                ui.label(egui::RichText::new(format!("{} Vorlagen", presets.len())).size(12.0).color(th.sub));
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for n in presets.iter().map(String::as_str) {
                            let spec = foldericons::Spec::Preset(n.to_string());
                            let (r, resp) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::click());
                            let sel = cur.as_ref() == Some(&spec);
                            if sel || resp.hovered() {
                                ui.painter().rect_filled(r, 6.0, if sel { th.sel } else { th.hover });
                            }
                            if ui.is_rect_visible(r) {
                                let ir = Rect::from_center_size(r.center() - vec2(0.0, 7.0), vec2(34.0, 34.0));
                                icons.paint_spec(ui.painter(), ir, &spec);
                                let label = foldericons::preset_label(n);
                                let short: String = label.chars().take(10).collect();
                                ui.painter().text(pos2(r.center().x, r.bottom() - 6.0), Align2::CENTER_BOTTOM, short, FontId::proportional(10.5), th.sub);
                            }
                            if resp.on_hover_text(foldericons::preset_label(n)).clicked() {
                                pick = Some(Some(spec));
                            }
                        }
                    });
                });
            });
        if let Some(p) = pick {
            if let Some(foldericons::Spec::Image(_)) = &p {
                if let Some(s) = &p {
                    icons.forget_spec(s);
                }
            }
            foldericons::set(dir, p);
            ctx.request_repaint();
        }
        if !open {
            self.folder_icon = None;
        }
    }

    fn ui_settings(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let th = self.th;
        let at = self.at(0.14);
        let mut st = self.st.clone();
        let mut reindex = false;
        self.record_key(ctx);
        let mut close = self.import.is_none() && self.key_rec.is_none() && self.key_conflict.is_none() && ctx.input(|i| i.key_pressed(Key::Escape));
        let mut theme_act: Option<ThemeAct> = None;
        let mut transfer: Option<bool> = None; // Some(true) = exportieren, Some(false) = importieren
        let mut uninstall_ask = false;
        let mut ai_act: Vec<Act> = Vec::new();
        // wirksame Theme-IDs (unbekannte oder unpassende IDs aus alten Einstellungen → Standard), sonst fehlen
        // Vorschau und Knöpfe, weil keine Auswahl gefunden wird
        let eff = |id: &str, dark: bool| -> String {
            match self.themes.get(id) {
                Some(p) if p.t.dark == dark => id.to_string(),
                _ => (if dark { themes::DEFAULT_DARK } else { themes::DEFAULT_LIGHT }).to_string(),
            }
        };
        let (light_id, dark_id) = (eff(&self.st.theme_light, false), eff(&self.st.theme_dark, true));
        let active_id = if th.dark { dark_id.clone() } else { light_id.clone() };
        let sel_id = self.theme_sel.clone().unwrap_or_else(|| active_id.clone());
        let frame = egui::Frame::window(&ctx.style())
            .fill(th.panel)
            .rounding(Rounding::same(6.0))
            .stroke(Stroke::new(1.0, th.line))
            .inner_margin(Margin::same(18.0))
            .shadow(egui::epaint::Shadow {
                offset: vec2(0.0, 6.0),
                blur: 18.0,
                spread: 0.0,
                color: Color32::from_black_alpha(70),
            });
        // Größe an kleine Fenster / hohe Windows-Skalierung anpassen
        let max_h = (ctx.screen_rect().height() - 60.0).clamp(300.0, 720.0);
        let win_w = (ctx.screen_rect().width() - 40.0).clamp(560.0, 900.0);
        if !SETTINGS_PAGES.iter().any(|p| p.0 == self.settings_page) {
            self.settings_page = SETTINGS_PAGES[0].0.to_string();
        }
        egui::Window::new("Einstellungen")
            .title_bar(false)
            .frame(frame)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([win_w, max_h])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Einstellungen").size(19.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, 3.0, th.hover);
                        }
                        let c = r.center();
                        let s = Stroke::new(1.4, th.text);
                        ui.painter().line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], s);
                        ui.painter().line_segment([c + vec2(-5.0, 5.0), c + vec2(5.0, -5.0)], s);
                        if resp.on_hover_text("Schließen (Esc)").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(8.0);
                // Links die Bereiche, rechts nur der gewählte Bereich – statt einer langen Liste
                let body_h = max_h - 60.0;
                let nav_w = if win_w < 680.0 { 150.0 } else { 186.0 };
                let mut page = self.settings_page.clone();
                ui.horizontal_top(|ui| {
                    let (nav, _) = ui.allocate_exact_size(vec2(nav_w, body_h), Sense::hover());
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(nav).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (name, ic, _) in SETTINGS_PAGES {
                            let (r, resp) = ui.allocate_exact_size(vec2(nav_w - 8.0, 32.0), Sense::click());
                            let on = page == *name;
                            if on || resp.hovered() {
                                ui.painter().rect_filled(r, 6.0, if on { th.sel } else { th.hover });
                            }
                            if on {
                                ui.painter().rect_filled(Rect::from_min_size(r.min + vec2(0.0, 8.0), vec2(3.0, 16.0)), 1.5, th.accent);
                            }
                            ui::draw(ui.painter(), Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(16.0, 16.0)), *ic, if on { th.accent } else { th.sub });
                            ui::paint_text(ui, r.left() + 38.0, r.center().y, name, 13.5, if on { th.text } else { th.sub }, r.width() - 44.0, false);
                            if resp.clicked() {
                                page = name.to_string();
                            }
                        }
                    });
                    let (sep, _) = ui.allocate_exact_size(vec2(1.0, body_h), Sense::hover());
                    ui.painter().rect_filled(sep, 0.0, th.line);
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    let desc = SETTINGS_PAGES.iter().find(|p| p.0 == page).map_or("", |p| p.2);
                    ui.label(egui::RichText::new(&page).size(17.0).strong());
                    ui.label(egui::RichText::new(desc).size(12.0).color(th.sub));
                    ui.add_space(6.0);
                    ui.ctx().data_mut(|d| d.insert_temp(Id::new("fc_settings_page"), page.clone()));
                let mut sa = egui::ScrollArea::vertical().id_salt(("settings_page", page.clone())).max_height(body_h - 52.0).auto_shrink([false, false]);
                // nur für Bildschirmfotos (FOLDEX_SHOT): an eine Stelle scrollen
                if let Some(y) = std::env::var("FOLDEX_SHOT_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
                    sa = sa.vertical_scroll_offset(y);
                }
                sa.show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    // Platz für die schwebende Scrollleiste, damit sie keine Karte überdeckt
                    ui.set_max_width(ui.available_width() - 14.0);
                    settings_card(ui, th, "Darstellung", |ui| {
                        setting_row(ui, th, "Farbschema", "Hell, dunkel oder wie dein System", |ui| {
                            segmented(ui, th, at, &mut st.theme, &[(0u8, "System"), (1, "Hell"), (2, "Dunkel")]);
                        });
                        setting_row(ui, th, "Animationen", "Gleitende Übergänge bei Tabs, Seitenleiste und Andocken", |ui| {
                            toggle_switch(ui, th, at, &mut st.animations);
                        });
                        setting_row(ui, th, "Standardansicht", "Gilt für neue Fenster und Tabs", |ui| {
                            segmented(
                                ui,
                                th,
                                at,
                                &mut st.view_mode,
                                &[(0u8, "Details"), (1, "Liste"), (2, "Raster"), (3, "Symbole")],
                            );
                        });
                        setting_row(ui, th, "Seitenleiste", "Schnellzugriff, Laufwerke und Papierkorb (Strg+B)", |ui| {
                            toggle_switch(ui, th, at, &mut st.show_sidebar);
                        });
                        setting_row(ui, th, "Git-Repositories", "In der Seitenleiste: Repositories unter Home und angehefteten Ordnern", |ui| {
                            toggle_switch(ui, th, at, &mut st.show_git);
                        });
                        setting_row(ui, th, "Zuletzt verwendete Ordner", "Verlauf in der Seitenleiste – ausschalten löscht den gespeicherten Verlauf", |ui| {
                            toggle_switch(ui, th, at, &mut st.show_recent);
                        });
                    });
                    settings_card(ui, th, "Themes", |ui| {
                        let line = |ui: &mut egui::Ui, t: &str| {
                            ui.label(egui::RichText::new(t).size(12.0).color(th.sub));
                        };
                        line(ui, &format!(
                            "Hell: {}   ·   Dunkel: {}",
                            self.themes.get(&self.st.theme_light).map_or("Foldex Hell", |p| p.name.as_str()),
                            self.themes.get(&self.st.theme_dark).map_or("Foldex Dunkel", |p| p.name.as_str()),
                        ));
                        ui.spacing_mut().item_spacing.y = 0.0;
                        line(ui, "Standard");
                        ui.add_space(4.0);
                        for p in &self.themes.builtin {
                            let active = p.id == light_id || p.id == dark_id;
                            let r = theme_row(ui, th, p, p.id == sel_id, active);
                            if r.clicked() {
                                theme_act = Some(ThemeAct::Select(p.id.clone()));
                            }
                            if r.double_clicked() {
                                theme_act = Some(ThemeAct::Apply(p.id.clone()));
                            }
                        }
                        ui.add_space(8.0);
                        line(ui, "Eigene");
                        ui.add_space(4.0);
                        if self.themes.custom.is_empty() {
                            line(ui, "Noch keine eigenen Themes – über „Importieren …“ eine JSON-Datei laden.");
                        }
                        for p in &self.themes.custom {
                            let active = p.id == light_id || p.id == dark_id;
                            let r = theme_row(ui, th, p, p.id == sel_id, active);
                            if r.clicked() {
                                theme_act = Some(ThemeAct::Select(p.id.clone()));
                            }
                            if r.double_clicked() {
                                theme_act = Some(ThemeAct::Apply(p.id.clone()));
                            }
                        }
                        for (file, err) in &self.themes.broken {
                            line(ui, &format!("⚠ {file}.json wird ignoriert: {err}"));
                        }
                        ui.spacing_mut().item_spacing.y = 10.0;
                        ui.add_space(10.0);
                        if let Some(p) = self.themes.get(&sel_id) {
                            paint_theme_preview(ui, &p.t, &p.name);
                            let (builtin, is_active) = (p.builtin, p.id == active_id);
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!is_active, egui::Button::new("Anwenden")).clicked() {
                                    theme_act = Some(ThemeAct::Apply(sel_id.clone()));
                                }
                                if ui.button("Exportieren …").on_hover_text("Als JSON speichern, z. B. als Vorlage für ein eigenes Theme").clicked() {
                                    theme_act = Some(ThemeAct::Export(sel_id.clone()));
                                }
                                if !builtin && ui.button("Löschen").clicked() {
                                    theme_act = Some(ThemeAct::Delete(sel_id.clone()));
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("Zurücksetzen").on_hover_text("Foldex Hell / Dunkel verwenden").clicked() {
                                        theme_act = Some(ThemeAct::Reset);
                                    }
                                    if ui.button("Importieren …").clicked() {
                                        theme_act = Some(ThemeAct::Import);
                                    }
                                });
                            });
                        }
                    });
                    if let Some((err, lines)) = &self.cfg_msg {
                        message_box(ui, th, *err, lines);
                    }
                    settings_card(ui, th, "Icons", |ui| {
                        let sys_ok = cfg!(any(windows, target_os = "macos"));
                        setting_row(
                            ui,
                            th,
                            "Icon-Satz",
                            "Mitgelieferte Themes (Material, Catppuccin, vscode-icons – MIT-Lizenz) oder die Icons deines Systems",
                            |ui| {
                                // None = System-Icons
                                let mut cur = if st.sys_icons && sys_ok { None } else { Some(matico::Set::from_key(&st.icon_set)) };
                                let mut opts: Vec<(Option<matico::Set>, &str)> = matico::Set::ALL.iter().map(|s| (Some(*s), s.label())).collect();
                                if sys_ok {
                                    opts.push((None, "System"));
                                }
                                segmented(ui, th, at, &mut cur, &opts);
                                st.sys_icons = cur.is_none();
                                if let Some(s) = cur {
                                    st.icon_set = s.key().to_string();
                                }
                            },
                        );
                        // Vorschau
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
                        ui.painter().rect_filled(rect, 3.0, th.bg);
                        let samples: [(&str, &str, bool); 8] = [
                            ("Ordner", "", true),
                            ("main.rs", "rs", false),
                            ("foto.png", "png", false),
                            ("buch.pdf", "pdf", false),
                            ("daten.zip", "zip", false),
                            ("lied.mp3", "mp3", false),
                            ("text.docx", "docx", false),
                            ("film.mp4", "mp4", false),
                        ];
                        let step = rect.width() / samples.len() as f32;
                        for (k, (name, ext, dir)) in samples.iter().enumerate() {
                            let ir = Rect::from_center_size(pos2(rect.left() + step * (k as f32 + 0.5), rect.center().y), vec2(28.0, 28.0));
                            let kind = if *dir { IconKind::Dir } else { IconKind::File };
                            self.icons.paint(ui.painter(), ir, kind, name, ext, false, PathBuf::new);
                        }
                    });
                    settings_card(ui, th, "Dateien", |ui| {
                        setting_row(ui, th, "Versteckte Dateien", "Versteckte Dateien und Ordner anzeigen", |ui| {
                            toggle_switch(ui, th, at, &mut st.show_hidden);
                        });
                        if cfg!(windows) {
                            setting_row(ui, th, "Geschützte Systemdateien", "Nur für Fortgeschrittene", |ui| {
                                toggle_switch(ui, th, at, &mut st.show_system);
                            });
                        }
                        setting_row(ui, th, "Ordner zuerst", "Bei Sortierung nach Name oder Typ", |ui| {
                            toggle_switch(ui, th, at, &mut st.folders_first);
                        });
                        setting_row(ui, th, "Ordnergrößen", "In der Detailansicht im Hintergrund berechnen (zwischengespeichert)", |ui| {
                            toggle_switch(ui, th, at, &mut st.folder_sizes);
                        });
                        setting_row(ui, th, "Git-Status markieren", "Geänderte, neue und nicht verfolgte Dateien kennzeichnen", |ui| {
                            toggle_switch(ui, th, at, &mut st.git_marks);
                        });
                        setting_row(ui, th, "Fenster wiederherstellen", "Beim Start die zuletzt offenen Fenster mit Tabs, Kachel-Aufteilung und Vorschau wieder öffnen", |ui| {
                            toggle_switch(ui, th, at, &mut st.restore_session);
                        });
                        setting_row(ui, th, "Vorschau", "Vorschau für neue Kacheln einschalten – jede Kachel hat ihre eigene (Alt+P, Lage und Größe in der Kopfzeile der Vorschau)", |ui| {
                            toggle_switch(ui, th, at, &mut st.show_preview);
                        });
                    });
                    settings_card(ui, th, "Suche", |ui| {
                        setting_row(ui, th, "Index beim Start", "Suchindex aller Laufwerke automatisch aktualisieren", |ui| {
                            toggle_switch(ui, th, at, &mut st.auto_index);
                        });
                        setting_row(ui, th, "Index neu aufbauen", &format!("Cache: {}", index::cache_dir().display()), |ui| {
                            if ui.button("Jetzt neu aufbauen").clicked() {
                                reindex = true;
                            }
                        });
                    });
                    settings_card(ui, th, "Terminal", |ui| {
                        let shells = self.shells.clone();
                        setting_row(ui, th, "Shell", "Für neue Terminal-Tabs (Alt+T)", |ui| {
                            let cur = if st.term_shell.is_empty() {
                                shells.first().map_or("Standard".to_string(), |s| format!("{} (Standard)", s.0))
                            } else {
                                shells.iter().find(|s| s.1 == st.term_shell).map_or(st.term_shell.clone(), |s| s.0.clone())
                            };
                            egui::ComboBox::from_id_salt("shell").selected_text(cur).width(220.0).show_ui(ui, |ui| {
                                ui.selectable_value(&mut st.term_shell, String::new(), "Standard (erste gefundene)");
                                for (name, prog) in &shells {
                                    ui.selectable_value(&mut st.term_shell, prog.clone(), name);
                                }
                            });
                        });
                        setting_row(ui, th, "Vorschläge beim Tippen", "Befehle, Verlauf, Unterbefehle, Branches, Ordner und Dateien – Tab übernimmt, ↑↓ wählt, Esc blendet aus", |ui| {
                            toggle_switch(ui, th, at, &mut st.term_complete);
                        });
                        setting_row(ui, th, "Eigene Shell", "Programmpfad, z. B. C:\\msys64\\usr\\bin\\bash.exe", |ui| {
                            if ui.button("Durchsuchen …").clicked() {
                                if let Some(f) = rfd::FileDialog::new().add_filter("Programm", &["exe", "cmd", "bat"]).pick_file() {
                                    st.term_shell = f.display().to_string();
                                }
                            }
                        });
                    });
                    settings_card(ui, th, "Editoren", |ui| {
                        ui.label(egui::RichText::new("Eingeschaltete Editoren erscheinen unter „Öffnen mit“ und „Ordner öffnen in“.").size(12.0).color(th.sub));
                        for (key, detected) in self.ed_detected.clone() {
                            let Some(d) = editors::def(&key) else { continue };
                            let mut on = st.editor_on.get(&key).copied().unwrap_or(d.default_on);
                            let custom = st.editor_paths.get(&key).cloned().unwrap_or_default();
                            let desc = if !custom.is_empty() {
                                let ok = Path::new(&custom).is_file();
                                format!("Eigener Pfad: {custom}{}", if ok { "" } else { "  – existiert nicht" })
                            } else {
                                match &detected {
                                    Some(p) => format!("Gefunden: {}", p.display()),
                                    None => "Nicht installiert – Programm über „Pfad …“ wählen".into(),
                                }
                            };
                            let mut pick: Option<Option<String>> = None;
                            setting_row(ui, th, d.name, &desc, |ui| {
                                toggle_switch(ui, th, at, &mut on);
                                if ui.small_button("Pfad …").on_hover_text("Programm selbst auswählen").clicked() {
                                    if let Some(f) = rfd::FileDialog::new().add_filter("Programm", &["exe", "cmd", "bat"]).pick_file() {
                                        pick = Some(Some(f.display().to_string()));
                                    }
                                }
                                if !custom.is_empty() && ui.small_button("Automatisch").on_hover_text("Eigenen Pfad entfernen").clicked() {
                                    pick = Some(None);
                                }
                            });
                            if on != d.default_on || st.editor_on.contains_key(&key) {
                                st.editor_on.insert(key.clone(), on);
                            }
                            match pick {
                                Some(Some(p)) => {
                                    st.editor_paths.insert(key.clone(), p);
                                }
                                Some(None) => {
                                    st.editor_paths.remove(&key);
                                }
                                None => {}
                            }
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Eigene Editoren").size(12.5).strong().color(th.sub));
                        let mut remove = None;
                        for (i, (name, path)) in st.custom_editors.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(name);
                                ui.label(egui::RichText::new(path).size(12.0).color(if Path::new(path).is_file() { th.sub } else { th.danger }));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("Entfernen").clicked() {
                                        remove = Some(i);
                                    }
                                });
                            });
                        }
                        if let Some(i) = remove {
                            st.custom_editors.remove(i);
                        }
                        if ui.button("Editor hinzufügen …").clicked() {
                            if let Some(f) = rfd::FileDialog::new().add_filter("Programm", &["exe", "cmd", "bat"]).pick_file() {
                                let name = f.file_stem().map_or("Editor".into(), |n| n.to_string_lossy().into_owned());
                                st.custom_editors.push((name, f.display().to_string()));
                            }
                        }
                    });
                    settings_card(ui, th, "KI", |ui| {
                        setting_row(ui, th, "KI-Funktionen", "Hauptschalter für Chat, Agenten und Modelle. Aus = alle KI-Tabs werden geschlossen, nichts wird gesendet.", |ui| {
                            toggle_switch(ui, th, at, &mut st.ai_enabled);
                        });
                        if !st.ai_enabled {
                            return;
                        }
                        // Explorer Assistent / Explorer Agent: nur eigene Modelle können Tabs und Befehle bedienen
                        let backs: Vec<(String, String)> = st.ai_models.iter().map(|p| (format!("model:{}", p.id), p.name.clone())).collect();
                        let cur = backs.iter().find(|b| b.0 == st.ai_agent).map_or_else(|| "erstes eigenes Modell".to_string(), |b| b.1.clone());
                        setting_row(ui, th, "Modell der Explorer-KI", "Explorer Assistent (Strg+Umschalt+I) und Explorer Agent (Strg+Umschalt+K) in der Seitenleiste", |ui| {
                            egui::ComboBox::from_id_salt("ed_model").selected_text(cur).width(220.0).show_ui(ui, |ui| {
                                for (id, n) in &backs {
                                    if ui.selectable_value(&mut st.ai_agent, id.clone(), n).clicked() {
                                        st.ai_assist = id.clone();
                                    }
                                }
                            });
                        });

                    });
                    // eigene Karten je Thema statt einer überfüllten Karte
                    if st.ai_enabled {
                    settings_card(ui, th, "KI", |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("KI-Agenten").size(14.5).strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if self.agent_scan.is_some() {
                                    ui.spinner();
                                } else if ui.small_button("Neu prüfen").on_hover_text("Nach einer Installation erneut suchen").clicked() {
                                    self.agent_scan = Some(scan_agents(ctx));
                                }
                            });
                        });
                        for d in agents::AGENTS {
                            let found = self.agents.iter().find(|a| a.0 == d.key).cloned();
                            egui::Frame::none().fill(th.bg).stroke(Stroke::new(1.0, th.line)).rounding(Rounding::same(8.0)).inner_margin(Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                                    ui::draw(ui.painter(), r, Ic::Spark, if found.is_some() { th.accent } else { th.sub });
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.label(egui::RichText::new(d.name).strong());
                                        let (txt, col) = match (&found, self.agent_versions.get(d.key)) {
                                            (Some(_), Some(v)) => (format!("Installiert · {v}"), crate::gitui::status_color(git::St::Added, &th)),
                                            (Some(f), None) => (format!("Installiert · {}", f.2), crate::gitui::status_color(git::St::Added, &th)),
                                            (None, _) => ("Nicht installiert".to_string(), th.sub),
                                        };
                                        ui.label(egui::RichText::new(txt).size(12.0).color(col));
                                    });
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        match &found {
                                            Some(f) => {
                                                let mut on = st.agent_on.get(d.key).copied().unwrap_or(true);
                                                toggle_switch(ui, th, at, &mut on);
                                                if !on || st.agent_on.contains_key(d.key) {
                                                    st.agent_on.insert(d.key.to_string(), on);
                                                }
                                                if ui.button("Terminal").on_hover_text("Im Terminal starten (klassische Oberfläche)").clicked() {
                                                    ai_act.push(Act::Terminal(Some(f.2.clone()), self.seen_cwd.clone()));
                                                }
                                                if ui.add_enabled(on, egui::Button::new(egui::RichText::new("Chat starten").strong())).clicked() {
                                                    ai_act.push(Act::AiChat(self.seen_cwd.clone(), Some(format!("agent:{}", d.key)), false));
                                                }
                                            }
                                            None => {
                                                if ui.button("Website").clicked() {
                                                    let _ = open::that_detached(d.url);
                                                }
                                                let cmd = if cfg!(windows) { d.install_win } else { d.install_unix };
                                                if ui.add(egui::Button::new(egui::RichText::new("Installieren").strong())).on_hover_text(format!("Führt in einem Terminal-Tab aus:\n{cmd}")).clicked() {
                                                    ai_act.push(Act::InstallAgent(d.key.to_string()));
                                                }
                                            }
                                        }
                                    });
                                });
                            });
                        }

                    });
                    settings_card(ui, th, "KI", |ui| {
                        ui.label(egui::RichText::new("Eigene Modelle").size(14.5).strong());
                        ui.label(egui::RichText::new("Anthropic-API oder OpenAI-kompatibel (LM Studio, Ollama, OpenAI, OpenRouter …). Schlüssel werden verschlüsselt gespeichert und nie exportiert.").size(12.0).color(th.sub));
                        // Liste: je Modell eine kompakte Zeile (alles gekürzt mit „…“); bearbeitet wird immer nur eines
                        let mut remove: Option<usize> = None;
                        let ok_col = crate::gitui::status_color(git::St::Added, &th);
                        let warn_col = crate::gitui::status_color(git::St::Modified, &th);
                        ui.add_space(4.0);
                        for i in 0..st.ai_models.len() {
                            let id = st.ai_models[i].id.clone();
                            let open = self.model_open.as_deref() == Some(id.as_str());
                            let has_key = self.ai_keys.get(&id).is_some_and(|k| k.is_some());
                            let p = &mut st.ai_models[i];
                            // Zustand: was fehlt noch?
                            let mut missing = Vec::new();
                            if p.name.trim().is_empty() {
                                missing.push("Name");
                            }
                            if !(p.url.starts_with("http://") || p.url.starts_with("https://")) {
                                missing.push("Adresse");
                            }
                            if p.model.trim().is_empty() {
                                missing.push("Modell");
                            }
                            if p.needs_key() && !has_key {
                                missing.push("API-Schlüssel");
                            }
                            let model_name = ai::CLAUDE_MODELS.iter().find(|m| m.0 == p.model).map_or_else(|| p.model.clone(), |m| m.1.to_string());
                            egui::Frame::none()
                                .fill(th.bg)
                                .stroke(Stroke::new(1.0, if open { th.accent.gamma_multiply(0.6) } else { th.line }))
                                .rounding(Rounding::same(8.0))
                                .inner_margin(Margin::symmetric(12.0, 8.0))
                                .outer_margin(Margin::symmetric(0.0, 3.0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    // ── Kopfzeile (klickbar): Symbol, Name, Schnittstelle · Adresse · Modell, Zustand ──
                                    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
                                    if resp.hovered() {
                                        ui.painter().rect_filled(row.expand2(vec2(6.0, 2.0)), 6.0, th.hover);
                                    }
                                    ui::draw(ui.painter(), Rect::from_center_size(pos2(row.left() + 10.0, row.center().y), vec2(18.0, 18.0)), Ic::Spark, th.accent);
                                    let status_w = 150.0;
                                    let text_x = row.left() + 30.0;
                                    let text_w = (row.width() - 30.0 - status_w - 24.0).max(60.0);
                                    let shown_name = if p.name.trim().is_empty() { "(ohne Namen)" } else { p.name.as_str() };
                                    ui::paint_text(ui, text_x, row.top() + 11.0, shown_name, 14.0, th.text, text_w, false);
                                    let detail = format!("{}  ·  {}  ·  {}", p.kind.label(), if p.url.is_empty() { "keine Adresse" } else { p.url.as_str() }, if model_name.is_empty() { "kein Modell" } else { model_name.as_str() });
                                    ui::paint_text(ui, text_x, row.bottom() - 10.0, &detail, 11.5, th.sub, text_w, false);
                                    let (status, col) = if missing.is_empty() { ("bereit".to_string(), ok_col) } else { (format!("fehlt: {}", missing.join(", ")), warn_col) };
                                    ui::paint_text(ui, row.right() - 22.0, row.center().y, &status, 12.0, col, status_w, true);
                                    ui::draw(ui.painter(), Rect::from_center_size(pos2(row.right() - 8.0, row.center().y), vec2(12.0, 12.0)), if open { Ic::ChevD } else { Ic::ChevR }, th.sub);
                                    if resp.on_hover_text(if open { "Einklappen" } else { "Bearbeiten" }).clicked() {
                                        self.model_open = if open { None } else { Some(id.clone()) };
                                    }
                                    if !open {
                                        return;
                                    }
                                    // ── Bearbeiten: feste Spalten, gleiche Höhe (28 px) für alle Felder ──
                                    ui.add_space(6.0);
                                    let label_w = 110.0;
                                    let field_w = (ui.available_width() - label_w - ui.spacing().item_spacing.x).max(160.0);
                                    let row_ui = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
                                        ui.horizontal(|ui| {
                                            // feste Beschriftungsspalte: alle Felder beginnen an derselben Stelle
                                            let (r, _) = ui.allocate_exact_size(vec2(label_w, 28.0), Sense::hover());
                                            ui::paint_text(ui, r.left(), r.center().y, label, 12.5, th.sub, label_w - 8.0, false);
                                            ui.allocate_ui_with_layout(vec2(field_w, 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| add(ui));
                                        });
                                    };
                                    row_ui(ui, "Name", &mut |ui| {
                                        ui.add(egui::TextEdit::singleline(&mut p.name).hint_text("Name des Modells").desired_width(field_w).min_size(vec2(field_w, 28.0)).vertical_align(egui::Align::Center));
                                    });
                                    let before = p.kind;
                                    row_ui(ui, "Schnittstelle", &mut |ui| {
                                        let items: Vec<(ai::Kind, String, String)> = ai::Kind::ALL.iter().map(|k| (*k, k.label().to_string(), String::new())).collect();
                                        if let Some(k) = ui::select_field(ui, &th, ("kind", &id), field_w, p.kind.label(), &items, Some(&p.kind)) {
                                            p.kind = k;
                                        }
                                    });
                                    if p.kind != before {
                                        // passende Standardadresse, wenn die alte zur anderen Schnittstelle gehörte
                                        if let Some(pr) = ai::PRESETS.iter().find(|x| x.1 == p.kind) {
                                            if p.url.trim().is_empty() || ai::PRESETS.iter().any(|x| x.1 == before && x.2 == p.url) {
                                                p.url = pr.2.to_string();
                                            }
                                        }
                                        p.model.clear();
                                        self.model_list.remove(&id);
                                    }
                                    row_ui(ui, "Adresse (URL)", &mut |ui| {
                                        ui.add(egui::TextEdit::singleline(&mut p.url).hint_text("https://… bzw. http://localhost:1234/v1").desired_width(field_w).min_size(vec2(field_w, 28.0)).vertical_align(egui::Align::Center));
                                    });
                                    row_ui(ui, "Modell", &mut |ui| {
                                        let list_w = 150.0;
                                        let tw = (field_w - list_w - ui.spacing().item_spacing.x).max(100.0);
                                        ui.add(egui::TextEdit::singleline(&mut p.model).hint_text("Modell-ID, z. B. qwen2.5-7b-instruct").desired_width(tw).min_size(vec2(tw, 28.0)).vertical_align(egui::Align::Center));
                                        let mut items: Vec<(String, String, String)> = Vec::new();
                                        if p.kind == ai::Kind::Anthropic {
                                            items.extend(ai::CLAUDE_MODELS.iter().map(|m| (m.0.to_string(), m.1.to_string(), String::new())));
                                        }
                                        for m in self.model_list.get(&id).into_iter().flatten() {
                                            if !items.iter().any(|x| &x.0 == m) {
                                                items.push((m.clone(), m.clone(), String::new()));
                                            }
                                        }
                                        let shown = if items.is_empty() { "„Testen“ füllt die Liste".to_string() } else { format!("{} verfügbar", items.len()) };
                                        if let Some(m) = ui::select_field(ui, &th, ("models", &id), list_w, &shown, &items, Some(&p.model)) {
                                            p.model = m;
                                        }
                                    });
                                    row_ui(ui, "API-Schlüssel", &mut |ui| {
                                        if self.key_edit.as_deref() == Some(id.as_str()) {
                                            let kw = (field_w - 190.0).max(120.0);
                                            let r = ui.add(egui::TextEdit::singleline(&mut self.ai_key_input).password(true).hint_text("Schlüssel einfügen").desired_width(kw).min_size(vec2(kw, 28.0)).vertical_align(egui::Align::Center));
                                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                                            if (ui.button("Speichern").clicked() || enter) && !self.ai_key_input.trim().is_empty() {
                                                match ai::save_key(&id, &self.ai_key_input) {
                                                    Ok(()) => {
                                                        self.ai_keys.insert(id.clone(), Some(self.ai_key_input.trim().to_string()));
                                                        self.ai_key_input.clear();
                                                        self.key_edit = None;
                                                    }
                                                    Err(e) => self.cfg_msg = Some((true, vec![e])),
                                                }
                                            }
                                            if ui.button("Abbrechen").clicked() {
                                                self.ai_key_input.clear();
                                                self.key_edit = None;
                                            }
                                        } else {
                                            let (t, c) = if has_key {
                                                ("gespeichert (verschlüsselt)", ok_col)
                                            } else if p.is_local() {
                                                ("nicht nötig (läuft auf diesem PC)", th.sub)
                                            } else {
                                                ("fehlt", th.danger)
                                            };
                                            ui.label(egui::RichText::new(t).size(12.5).color(c));
                                            if ui.button(if has_key { "Ändern" } else { "Eingeben" }).clicked() {
                                                self.key_edit = Some(id.clone());
                                                self.ai_key_input.clear();
                                            }
                                            if has_key && ui.button("Entfernen").clicked() {
                                                ai::delete_key(&id);
                                                self.ai_keys.insert(id.clone(), ai::key_for(p));
                                            }
                                        }
                                    });
                                    // ── Aktionen ──
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        let busy = self.model_test_rx.is_some();
                                        if ui.add_enabled(!busy, egui::Button::new("Verbindung testen")).on_hover_text("Verbindung prüfen und verfügbare Modelle abrufen").clicked() {
                                            let cfg = ai::Cfg::from_profile(p, self.ai_keys.get(&id).cloned().flatten());
                                            let (tx, rx) = mpsc::channel();
                                            let (id2, c2) = (id.clone(), ctx.clone());
                                            std::thread::spawn(move || {
                                                let t = ai::test_connection(&cfg);
                                                let list = ai::list_models(&cfg).unwrap_or_default();
                                                let _ = tx.send((id2, t, list));
                                                c2.request_repaint();
                                            });
                                            self.model_test_rx = Some(rx);
                                        }
                                        if busy {
                                            ui.spinner();
                                        }
                                        if ui.add_enabled(missing.is_empty(), egui::Button::new(egui::RichText::new("Chat starten").strong())).on_disabled_hover_text(format!("Es fehlt: {}", missing.join(", "))).clicked() {
                                            ai_act.push(Act::AiChat(self.seen_cwd.clone(), Some(format!("model:{id}")), false));
                                        }
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.button(egui::RichText::new("Löschen").color(th.danger)).on_hover_text("Profil und gespeicherten Schlüssel entfernen").clicked() {
                                                remove = Some(i);
                                            }
                                        });
                                    });
                                    if let Some(r) = self.model_test.get(&id) {
                                        let (t, c) = match r {
                                            Ok(t) => (t.clone(), ok_col),
                                            Err(e) => (e.clone(), th.danger),
                                        };
                                        ui.add(egui::Label::new(egui::RichText::new(t).size(12.0).color(c)).wrap());
                                    }
                                });
                        }
                        if let Some(i) = remove {
                            let p = st.ai_models.remove(i);
                            ai::delete_key(&p.id);
                            self.ai_keys.remove(&p.id);
                            self.model_test.remove(&p.id);
                            self.model_list.remove(&p.id);
                            if self.model_open.as_deref() == Some(p.id.as_str()) {
                                self.model_open = None;
                            }
                        }
                        // ── Neues Modell ──
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(110.0, 28.0), Sense::hover());
                            ui::paint_text(ui, r.left() + 12.0, r.center().y, "Neues Modell", 12.5, th.sub, 98.0, false);
                            let items: Vec<(usize, String, String)> = ai::PRESETS.iter().enumerate().map(|(k, pr)| (k, pr.0.to_string(), String::new())).collect();
                            let cur = ai::PRESETS[self.new_model.min(ai::PRESETS.len() - 1)].0;
                            if let Some(k) = ui::select_field(ui, &th, "preset", 220.0, cur, &items, Some(&self.new_model)) {
                                self.new_model = k;
                            }
                            if ui.add(egui::Button::new("+ Hinzufügen").min_size(vec2(0.0, 28.0))).clicked() {
                                let pr = ai::PRESETS[self.new_model.min(ai::PRESETS.len() - 1)];
                                let id = ai::ModelProfile::new_id(&st.ai_models);
                                // gleicher Name wie ein vorhandenes Profil → Nummer anhängen
                                let mut name = pr.0.to_string();
                                let mut n = 2;
                                while st.ai_models.iter().any(|m| m.name == name) {
                                    name = format!("{} {n}", pr.0);
                                    n += 1;
                                }
                                let p = ai::ModelProfile { id: id.clone(), name, kind: pr.1, url: pr.2.to_string(), model: pr.3.to_string() };
                                self.ai_keys.insert(id.clone(), ai::key_for(&p));
                                st.ai_models.push(p);
                                // das neue Modell gleich zum Bearbeiten öffnen
                                self.model_open = Some(id);
                            }
                        });

                    });
                    settings_card(ui, th, "KI", |ui| {
                        ui.label(egui::RichText::new("KI-Agent darf ohne Nachfrage (eigene Modelle)").size(14.5).strong());
                        ui.label(egui::RichText::new("Nicht angehakte Aktionen erscheinen als Karte mit „Übernehmen“. Alles bleibt mit Strg+Z rückgängig zu machen.").size(12.0).color(th.sub));
                        for (k, label) in ai::CATEGORIES {
                            let mut on = st.ai_auto.contains(*k);
                            let before = on;
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(*label).size(13.5));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    toggle_switch(ui, th, at, &mut on);
                                });
                            });
                            if on != before {
                                if on {
                                    st.ai_auto.insert(k.to_string());
                                } else {
                                    st.ai_auto.remove(*k);
                                }
                            }
                        }
                        setting_row(
                            ui,
                            th,
                            "Agenten dürfen Befehle ausführen",
                            "Claude Code, Codex … im Agent-Modus ohne Nachfrage auch Programme/Befehle starten lassen. Nur für vertrauenswürdige Ordner!",
                            |ui| {
                                toggle_switch(ui, th, at, &mut st.ai_cli_full);
                            },
                        );
                        if st.ai_cli_full {
                            ui.label(egui::RichText::new("Achtung: Agenten können damit beliebige Befehle ausführen.").size(12.0).color(th.danger));
                        }
                    });
                    }
                    settings_card(ui, th, "Lokale Modelle", |ui| {
                        if crate::localui::settings_ui(ui, &th, &mut st.ai_models) {
                            ai_act.push(Act::ImageGen(self.seen_cwd.clone()));
                        }
                    });
                    settings_card(ui, th, "Bildeditor-KI", |ui| crate::imgedit::models_settings_ui(ui, &th));
                    settings_card(ui, th, "Schnellleiste", |ui| {
                        ui.label(egui::RichText::new("Am Griff ziehen zum Sortieren, Schalter zum Ein-/Ausblenden. Änderungen gelten sofort.").size(12.0).color(th.sub));
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let mut rects: Vec<Rect> = Vec::with_capacity(st.quickbar.len());
                        for i in 0..st.quickbar.len() {
                            let (id, mut on) = st.quickbar[i].clone();
                            let sep = id.starts_with("sep");
                            let full = ui.available_width();
                            let (row, _) = ui.allocate_exact_size(vec2(full, 30.0), Sense::hover());
                            rects.push(row);
                            let grab = Rect::from_min_max(row.min, pos2(row.right() - 60.0, row.bottom()));
                            let resp = ui.interact(grab, ui.id().with(("qb", i)), Sense::drag());
                            let dragging = self.qb_drag == Some(i);
                            if dragging {
                                ui.painter().rect_filled(row, 6.0, th.sel);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(row, 6.0, th.hover);
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            // Griff
                            for r in 0..3 {
                                for c in 0..2 {
                                    ui.painter().circle_filled(pos2(row.left() + 10.0 + c as f32 * 5.0, row.center().y - 5.0 + r as f32 * 5.0), 1.2, th.sub);
                                }
                            }
                            let col = if on { th.text } else { th.sub };
                            if sep {
                                ui.painter().line_segment([pos2(row.left() + 34.0, row.center().y), pos2(row.left() + 120.0, row.center().y)], Stroke::new(1.0, th.line));
                                ui.painter().text(pos2(row.left() + 128.0, row.center().y), Align2::LEFT_CENTER, "Trenner", FontId::proportional(13.0), th.sub);
                            } else {
                                ui::draw(ui.painter(), Rect::from_center_size(pos2(row.left() + 42.0, row.center().y), vec2(16.0, 16.0)), qb_icon(&id), col);
                                ui.painter().text(pos2(row.left() + 60.0, row.center().y), Align2::LEFT_CENTER, quickbar::label(&id), FontId::proportional(14.0), col);
                            }
                            let tr = Rect::from_min_size(pos2(row.right() - 50.0, row.center().y - 11.5), vec2(42.0, 23.0));
                            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(tr), |ui| {
                                toggle_switch(ui, th, at, &mut on);
                            });
                            st.quickbar[i].1 = on;
                            if resp.drag_started() {
                                self.qb_drag = Some(i);
                            }
                        }
                        // Ziehen: Einfügelinie und Verschieben beim Loslassen
                        if let Some(from) = self.qb_drag {
                            if let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
                                let to = rects.iter().filter(|r| r.center().y < p.y).count();
                                let y = if to < rects.len() { rects[to].top() } else { rects.last().map_or(0.0, |r| r.bottom()) };
                                if let Some(r0) = rects.first() {
                                    ui.painter().line_segment([pos2(r0.left() + 4.0, y), pos2(r0.right() - 4.0, y)], Stroke::new(2.0, th.accent));
                                }
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                if ui.input(|i| i.pointer.any_released()) {
                                    let to = if to > from { to - 1 } else { to };
                                    let item = st.quickbar.remove(from);
                                    st.quickbar.insert(to.min(st.quickbar.len()), item);
                                    self.qb_drag = None;
                                }
                            }
                            if !ui.input(|i| i.pointer.any_down()) {
                                self.qb_drag = None;
                            }
                        }
                        ui.add_space(6.0);
                        if ui.button("Standard wiederherstellen").clicked() {
                            st.quickbar = quickbar::defaults();
                        }
                    });
                    settings_card(ui, th, "Tastenkürzel", |ui| {
                        ui.label(egui::RichText::new("Auf ein Kürzel klicken und die neue Tastenkombination drücken. Esc bricht ab, Entf entfernt das Kürzel.").size(12.0).color(th.sub));
                        let mut cat = "";
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for c in keymap::CMDS {
                            if c.cat != cat {
                                cat = c.cat;
                                ui.add_space(6.0);
                                ui.label(egui::RichText::new(cat).size(12.5).strong().color(th.sub));
                            }
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                ui.label(c.label);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let custom = self.keymap.custom.contains_key(c.id);
                                    if custom && !c.fixed && ui.small_button("Standard").clicked() {
                                        let def = Shortcut::parse(c.default);
                                        match def.and_then(|d| self.keymap.conflict(d, c.id)) {
                                            Some(other) => self.key_conflict = Some((c.id, def.unwrap(), other.id)),
                                            None => {
                                                self.keymap.set(c.id, def);
                                                self.keymap.save();
                                            }
                                        }
                                    }
                                    let recording = self.key_rec == Some(c.id);
                                    let label = if recording {
                                        "Taste drücken …".to_string()
                                    } else {
                                        let l = self.keymap.label(c.id);
                                        if l.is_empty() { "—".into() } else { l }
                                    };
                                    let b = egui::Button::new(egui::RichText::new(label).size(12.5).color(if recording { th.accent } else { th.text }))
                                        .min_size(vec2(150.0, 22.0));
                                    let r = ui.add_enabled(!c.fixed, b);
                                    if c.fixed {
                                        r.on_disabled_hover_text("Fest belegt (Windows-Standard)");
                                    } else if r.clicked() {
                                        self.key_rec = if recording { None } else { Some(c.id) };
                                    }
                                });
                            });
                        }
                        ui.add_space(6.0);
                        if !self.keymap.custom.is_empty() && ui.button("Alle Tastenkürzel zurücksetzen").clicked() {
                            self.keymap = Keymap::default();
                            self.keymap.save();
                        }
                    });
                    settings_card(ui, th, "Import / Export", |ui| {
                        setting_row(
                            ui,
                            th,
                            "Export und Import",
                            "Darstellung, Themes, Tastenkürzel, Editoren, Terminal, Seitenleiste und Dateiansicht als JSON-Datei",
                            |ui| {
                                if ui.button("Importieren …").clicked() {
                                    transfer = Some(false);
                                }
                                if ui.button("Exportieren …").clicked() {
                                    transfer = Some(true);
                                }
                            },
                        );
                    });
                    settings_card(ui, th, "Updates", |ui| {
                        update_ui(ui, th, &self.updater, &self.update_quit, false);
                    });
                    settings_card(ui, th, "Deinstallieren", |ui| {
                        // Größen einmal im Hintergrund ermitteln (Modelle können viele GB groß sein)
                        static SIZES: std::sync::Mutex<Option<Vec<u64>>> = std::sync::Mutex::new(None);
                        static STARTED: AtomicBool = AtomicBool::new(false);
                        if !STARTED.swap(true, Relaxed) {
                            let ctx = ui.ctx().clone();
                            std::thread::spawn(move || {
                                let v: Vec<u64> = uninstall::items().iter().map(|i| uninstall::size_of(&i.path)).collect();
                                *SIZES.lock().unwrap() = Some(v);
                                ctx.request_repaint();
                            });
                        }
                        let sizes = SIZES.lock().unwrap().clone();
                        ui.label(egui::RichText::new("Entfernt Foldex vollständig von diesem PC – mit allem, was Foldex angelegt hat:").color(th.text));
                        ui.add_space(6.0);
                        let items = uninstall::items();
                        // „Einstellungen“ enthält die übrigen Ordner – deren Größe nicht doppelt zählen
                        let rest = sizes.as_ref().map(|s| s[3].saturating_sub(s[0] + s[1] + s[2]));
                        for (k, it) in items.iter().enumerate() {
                            let sz = sizes.as_ref().map(|s| if k == 3 { rest.unwrap_or(0) } else { s[k] });
                            setting_row(ui, th, it.label, &it.path.display().to_string(), |ui| {
                                ui.label(egui::RichText::new(sz.map_or("…".to_string(), human)).color(th.sub));
                            });
                        }
                        let prog = if uninstall::installed() { "Programm, Startmenü- und Desktop-Verknüpfung, Eintrag unter „Apps“" } else { "Startmenü- und Desktop-Verknüpfung" };
                        setting_row(ui, th, "Programm und Verknüpfungen", prog, |_| {});
                        if let Some(s) = &sizes {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("Zusammen etwa {}", human(s[3] + s.get(4).copied().unwrap_or(0)))).strong());
                        }
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Deine eigenen Dateien und deine Daten in der Cloud bleiben unangetastet.").size(12.0).color(th.sub));
                        ui.add_space(10.0);
                        if ui.add(egui::Button::new(egui::RichText::new("Foldex deinstallieren …").color(Color32::WHITE).strong()).fill(th.danger).min_size(vec2(0.0, 34.0))).clicked() {
                            uninstall_ask = true;
                        }
                    });
                });
                    });
                });
                self.settings_page = page;
            });
        self.set_settings(st);
        for a in ai_act {
            self.apply(a);
        }
        if let Some(a) = theme_act {
            self.theme_action(a);
        }
        if uninstall_ask {
            self.uninstall_ask = Some(!uninstall::installed());
        }
        match transfer {
            Some(true) => self.export_settings(),
            Some(false) => self.pick_import(),
            None => {}
        }
        if reindex {
            self.apply(Act::ReindexAll);
        }
        if close {
            self.show_settings = false;
            self.theme_sel = None;
            self.cfg_msg = None;
        }
        self.ui_import(ctx);
        self.ui_key_conflict(ctx);
    }

    /// Nimmt das nächste Tastenkürzel für den markierten Befehl auf; prüft Reservierungen und Konflikte.
    fn record_key(&mut self, ctx: &egui::Context) {
        let Some(id) = self.key_rec else { return };
        let ev = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, m)) = ev else { return };
        ctx.input_mut(|i| i.consume_key(m, key));
        if key == Key::Escape && !m.any() {
            self.key_rec = None;
            return;
        }
        if (key == Key::Delete || key == Key::Backspace) && !m.any() {
            self.keymap.set(id, None);
            self.keymap.save();
            self.key_rec = None;
            return;
        }
        let sc = Shortcut { ctrl: m.command, shift: m.shift, alt: m.alt, key };
        self.key_rec = None;
        if sc.reserved() {
            self.cfg_msg = Some((true, vec![format!("„{}“ ist reserviert (Navigation, Tippen oder Kopieren/Einfügen) und kann nicht belegt werden.", sc.label())]));
            return;
        }
        match self.keymap.conflict(sc, id) {
            Some(other) if other.fixed => {
                self.cfg_msg = Some((true, vec![format!("„{}“ ist fest mit „{}“ belegt.", sc.label(), other.label)]));
            }
            Some(other) => self.key_conflict = Some((id, sc, other.id)),
            None => {
                self.keymap.set(id, Some(sc));
                self.keymap.save();
                self.cfg_msg = None;
            }
        }
    }

    /// Konflikt bei Tastenkürzeln: ersetzen (der andere Befehl verliert sein Kürzel) oder abbrechen.
    fn ui_key_conflict(&mut self, ctx: &egui::Context) {
        let Some((id, sc, other)) = self.key_conflict else { return };
        let label = |id: &str| keymap::cmd(id).map_or(id.to_string(), |c| c.label.to_string());
        let mut decision = None;
        egui::Window::new("Tastenkürzel bereits belegt")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(format!("„{}“ ist bereits „{}“ zugewiesen.", sc.label(), label(other)));
                ui.label(format!("Ersetzen: „{}“ bekommt das Kürzel, „{}“ hat danach keines mehr.", label(id), label(other)));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Ersetzen").clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Abbrechen").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.keymap.set(other, None);
                self.keymap.set(id, Some(sc));
                self.keymap.save();
                self.key_conflict = None;
            }
            Some(false) => self.key_conflict = None,
            None => {}
        }
    }

    // ── Command Palette (Strg+Leertaste) ──
    fn ui_palette(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(mut pal) = self.palette.take() else { return };
        let th = self.th;
        // Einträge: alle Befehle + dynamische (Monitore, Shells, zuletzt verwendete Ordner)
        enum Item {
            Cmd(&'static str),
            Act(Act),
        }
        let mut items: Vec<(String, String, String, Ic, Item)> = Vec::new();
        for c in keymap::CMDS {
            if c.id == "app.palette" {
                continue;
            }
            items.push((c.label.to_string(), c.cat.to_string(), self.keymap.label(c.id), c.ic.unwrap_or(Ic::Command), Item::Cmd(c.id)));
        }
        let cwd = self.pane().cwd.clone();
        for (name, prog) in self.shells.clone() {
            items.push((format!("Terminal: {name}"), "Terminal".into(), String::new(), Ic::Terminal, Item::Act(Act::Terminal(Some(prog), cwd.clone()))));
        }
        if self.st.ai_enabled {
            for (id, name) in self.backends() {
                let agent = id.starts_with("agent:");
                items.push((format!("KI-Chat: {name}"), "KI".into(), String::new(), Ic::Spark, Item::Act(Act::AiChat(cwd.clone(), Some(id), agent))));
            }
        }
        let mons = editors::monitors();
        if mons.len() > 1 {
            for (k, (_, work, primary)) in mons.into_iter().enumerate() {
                let l = format!("Neues Fenster auf Monitor {}{}", k + 1, if primary { " (Haupt)" } else { "" });
                items.push((l, "Fenster".into(), String::new(), Ic::Window, Item::Act(Act::NewWindow(Some(work)))));
            }
        }
        for d in self.recent.dirs.iter().take(10) {
            let n = d.file_name().map_or_else(|| d.display().to_string(), |n| n.to_string_lossy().into_owned());
            items.push((format!("Gehe zu: {n}"), d.display().to_string(), String::new(), Ic::Clock, Item::Act(Act::Go(d.clone()))));
        }
        for p in &self.side.pins {
            items.push((format!("Gehe zu: {}", p.label), p.path.display().to_string(), String::new(), Ic::Star, Item::Act(Act::Go(p.path.clone()))));
        }
        // Filtern und sortieren
        let mut hits: Vec<(i32, usize)> = items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                let a = keymap::fuzzy(&it.0, &pal.query);
                let b = keymap::fuzzy(&format!("{} {}", it.1, it.0), &pal.query).map(|s| s - 5);
                a.max(b).map(|sc| (sc, i))
            })
            .collect();
        if !pal.query.is_empty() {
            hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        }
        hits.truncate(200);
        let (down, up, enter, esc) = ctx.input(|i| {
            (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape))
        });
        if down && !hits.is_empty() {
            pal.sel = (pal.sel + 1).min(hits.len() - 1);
        }
        if up {
            pal.sel = pal.sel.saturating_sub(1);
        }
        pal.sel = pal.sel.min(hits.len().saturating_sub(1));
        let mut run: Option<usize> = if enter { hits.get(pal.sel).map(|h| h.1) } else { None };
        let mut close = esc;
        let screen = ctx.screen_rect();
        let w = 560.0f32.min(screen.width() - 40.0);
        let area = egui::Area::new(Id::new("palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(screen.center().x - w / 2.0, screen.top() + 60.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(th.panel).rounding(Rounding::same(10.0)).inner_margin(Margin::same(8.0)).show(ui, |ui| {
                    ui.set_width(w);
                    let te = ui.add(
                        egui::TextEdit::singleline(&mut pal.query)
                            .hint_text("Befehl oder Ordner suchen …")
                            .desired_width(f32::INFINITY)
                            .margin(Margin::symmetric(10.0, 8.0))
                            .font(FontId::proportional(15.0)),
                    );
                    if pal.first {
                        te.request_focus();
                        pal.first = false;
                    }
                    if te.changed() {
                        pal.sel = 0;
                    }
                    ui.add_space(4.0);
                    let row_h = 32.0;
                    let max_rows = ((screen.height() * 0.6) / row_h).floor().max(4.0) as usize;
                    egui::ScrollArea::vertical().max_height(row_h * max_rows as f32).auto_shrink([false, true]).show_rows(ui, row_h, hits.len(), |ui, range| {
                        for k in range {
                            let it = &items[hits[k].1];
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                            let selected = k == pal.sel;
                            if selected || resp.hovered() {
                                ui.painter().rect_filled(r, Rounding::same(6.0), if selected { th.sel } else { th.hover });
                            }
                            ui::draw(ui.painter(), Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), vec2(16.0, 16.0)), it.3, if selected { th.accent } else { th.sub });
                            let kw = if it.2.is_empty() { 0.0 } else { 130.0 };
                            ui::paint_text(ui, r.left() + 38.0, r.center().y, &it.0, 14.0, th.text, r.width() * 0.62 - 38.0, false);
                            ui::paint_text(ui, r.right() - 12.0 - kw, r.center().y, &it.1, 12.0, th.sub, r.width() * 0.38 - kw - 16.0, true);
                            if kw > 0.0 {
                                ui::paint_text(ui, r.right() - 12.0, r.center().y, &it.2, 12.0, th.sub, kw - 8.0, true);
                            }
                            if selected && (down || up) {
                                ui.scroll_to_rect(r, None);
                            }
                            if resp.clicked() {
                                run = Some(hits[k].1);
                            }
                        }
                    });
                    if hits.is_empty() {
                        ui.label(egui::RichText::new("Keine Treffer").color(th.sub));
                    }
                });
            });
        // Klick außerhalb schließt
        if ctx.input(|i| i.pointer.any_pressed()) && !ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| area.response.rect.contains(p)) {
            close = true;
        }
        if let Some(i) = run {
            close = true;
            match items.swap_remove(i).4 {
                Item::Cmd(id) => acts.push(Act::Cmd(id)),
                Item::Act(a) => acts.push(a),
            }
        }
        if !close {
            self.palette = Some(pal);
        }
    }

    // ── Auswahl nach Regeln ──
    fn ui_rules(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.rules.take() else { return };
        let mut open = true;
        let mut close = false;
        let mut apply = false;
        egui::Window::new("Auswahl nach Regeln")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(420.0);
                egui::ComboBox::from_label("Regel").selected_text(w.kind.label()).width(300.0).show_ui(ui, |ui| {
                    for k in tools::RuleKind::ALL {
                        ui.selectable_value(&mut w.kind, k, k.label());
                    }
                });
                let te = ui.add(egui::TextEdit::singleline(&mut w.value).desired_width(f32::INFINITY).hint_text(match w.kind {
                    tools::RuleKind::Name => "z. B. bericht oder *.tmp",
                    tools::RuleKind::Ext => "z. B. jpg, png, gif",
                    tools::RuleKind::SizeMin | tools::RuleKind::SizeMax => "z. B. 10 MB",
                    tools::RuleKind::NewerDays | tools::RuleKind::OlderDays => "Anzahl Tage, z. B. 30",
                    tools::RuleKind::Regex => r"z. B. ^IMG_\d{4}\.jpe?g$",
                }));
                te.request_focus();
                ui.horizontal(|ui| {
                    ui.radio_value(&mut w.mode, tools::SelMode::Replace, "Neue Auswahl");
                    ui.radio_value(&mut w.mode, tools::SelMode::Add, "Hinzufügen");
                    ui.radio_value(&mut w.mode, tools::SelMode::Remove, "Entfernen");
                });
                if let Some(e) = &w.err {
                    ui.label(egui::RichText::new(e).color(self.th.danger));
                }
                ui.horizontal(|ui| {
                    if ui.button("Auswählen").clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                        apply = true;
                    }
                    if ui.button("Schließen").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                });
            });
        if apply {
            match tools::rule_matcher(w.kind, &w.value) {
                Err(e) => w.err = Some(e),
                Ok(m) => {
                    let pn = self.pane_mut();
                    let hits: Vec<String> = pn
                        .view
                        .iter()
                        .map(|&i| &pn.items[i])
                        .filter(|it| m(&it.name, it.dir, it.size, it.mtime / 1_000_000_000))
                        .map(|it| it.name.to_string())
                        .collect();
                    let n = hits.len();
                    match w.mode {
                        tools::SelMode::Replace => pn.sel = hits.into_iter().collect(),
                        tools::SelMode::Add => pn.sel.extend(hits),
                        tools::SelMode::Remove => {
                            for h in hits {
                                pn.sel.remove(&h);
                            }
                        }
                    }
                    pn.sel_dirty = true;
                    let total = pn.sel.len();
                    w.err = None;
                    self.status = format!("{n} Element(e) passen zur Regel – {total} ausgewählt");
                }
            }
        }
        if open && !close {
            self.rules = Some(w);
        }
    }

    // ── Werkzeuge: Vergleichen, Duplikate, Speicherplatz, große Dateien ──
    fn ui_tools(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(mut w) = self.tools.take() else { return };
        let th = self.th;
        if let Some(t) = &w.task {
            if let Ok(out) = t.rx.try_recv() {
                w.result = Some(out);
                w.task = None;
                w.marked.clear();
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
        let mut open = true;
        let mut start = false;
        let mut trash: Option<Vec<PathBuf>> = None;
        egui::Window::new("Werkzeuge")
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .default_size([720.0, 520.0])
            .min_size([480.0, 320.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (t, ic, l) in [
                        (ToolTab::Compare, Ic::Compare, "Vergleichen"),
                        (ToolTab::Dups, Ic::Copy, "Duplikate"),
                        (ToolTab::Space, Ic::Chart, "Speicherplatz"),
                        (ToolTab::Large, Ic::Chart, "Große Dateien"),
                    ] {
                        let r = ui::cmd_btn(ui, &th, Some(ic), l, w.task.is_none(), false);
                        if w.tab == t {
                            ui.painter().rect_filled(Rect::from_min_size(r.rect.left_bottom() - vec2(-10.0, 2.0), vec2(r.rect.width() - 20.0, 2.0)), 1.0, th.accent);
                        }
                        if r.clicked() && w.tab != t {
                            w.tab = t;
                            w.result = None;
                        }
                    }
                });
                ui.separator();
                let path_row = |ui: &mut egui::Ui, label: &str, v: &mut String| {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.add(egui::TextEdit::singleline(v).desired_width(ui.available_width() - 110.0));
                        if ui.button("Wählen …").clicked() {
                            if let Some(d) = rfd::FileDialog::new().set_directory(v.as_str()).pick_folder() {
                                *v = d.display().to_string();
                            }
                        }
                    });
                };
                match w.tab {
                    ToolTab::Compare => {
                        path_row(ui, "Links: ", &mut w.a);
                        path_row(ui, "Rechts:", &mut w.b);
                        ui.checkbox(&mut w.content, "Inhalt vergleichen (langsamer, statt Größe + Änderungszeit)");
                    }
                    ToolTab::Dups => {
                        path_row(ui, "Ordner:", &mut w.a);
                        ui.horizontal(|ui| {
                            ui.label("Nur Dateien ab");
                            ui.add(egui::TextEdit::singleline(&mut w.min_size).desired_width(80.0));
                        });
                    }
                    ToolTab::Space | ToolTab::Large => path_row(ui, "Ordner:", &mut w.a),
                }
                ui.horizontal(|ui| {
                    match &w.task {
                        Some(t) => {
                            ui.spinner();
                            ui.label(format!("{} … {} Dateien geprüft", t.label, t.progress.load(Relaxed)));
                            if ui.button("Abbrechen").clicked() {
                                t.cancel.store(true, Relaxed);
                            }
                        }
                        None => {
                            if ui.button(egui::RichText::new("Starten").strong()).clicked() {
                                start = true;
                            }
                        }
                    }
                });
                ui.separator();
                match &w.result {
                    None => {}
                    Some(tools::Output::Error(e)) => {
                        ui.label(egui::RichText::new(e).color(th.danger));
                    }
                    Some(tools::Output::Compare(v)) => {
                        let (l, r, d) = v.iter().fold((0, 0, 0), |a, x| match x.kind {
                            tools::DiffKind::OnlyLeft => (a.0 + 1, a.1, a.2),
                            tools::DiffKind::OnlyRight => (a.0, a.1 + 1, a.2),
                            tools::DiffKind::Different => (a.0, a.1, a.2 + 1),
                        });
                        ui.label(format!("{l} nur links · {r} nur rechts · {d} unterschiedlich"));
                        if v.is_empty() {
                            ui.label(egui::RichText::new("Beide Ordner sind gleich.").color(th.sub));
                        }
                        let a = PathBuf::from(&w.a);
                        let b = PathBuf::from(&w.b);
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 24.0, v.len(), |ui, range| {
                            for x in &v[range] {
                                let (tag, col) = match x.kind {
                                    tools::DiffKind::OnlyLeft => ("nur links", th.accent),
                                    tools::DiffKind::OnlyRight => ("nur rechts", gitui::status_color(git::St::Added, &th)),
                                    tools::DiffKind::Different => ("anders", gitui::status_color(git::St::Modified, &th)),
                                };
                                ui.horizontal(|ui| {
                                    ui.add_sized([80.0, 22.0], egui::Label::new(egui::RichText::new(tag).size(12.0).color(col)));
                                    let sz = |s: Option<(u64, i64)>| s.map_or("–".into(), |s| human(s.0));
                                    let r = ui.add(egui::Label::new(x.rel.display().to_string()).truncate().sense(Sense::click()));
                                    r.clone().on_hover_text(format!("links: {}   rechts: {}", sz(x.left), sz(x.right)));
                                    if r.double_clicked() {
                                        let base = if x.kind == tools::DiffKind::OnlyRight { &b } else { &a };
                                        acts.push(Act::Reveal(base.join(&x.rel)));
                                    }
                                });
                            }
                        });
                    }
                    Some(tools::Output::Dups(groups)) => {
                        let waste: u64 = groups.iter().map(|g| g.size * (g.files.len() as u64 - 1)).sum();
                        ui.horizontal(|ui| {
                            ui.label(format!("{} Gruppen · {} belegt durch Kopien", groups.len(), human(waste)));
                            if ui.button("Alle Kopien markieren").on_hover_text("Je Gruppe bleibt die erste Datei erhalten").clicked() {
                                w.marked = groups.iter().flat_map(|g| g.files.iter().skip(1).cloned()).collect();
                            }
                            let n = w.marked.len();
                            if n > 0 && ui.button(egui::RichText::new(format!("{n} markierte in den Papierkorb …")).color(th.danger)).clicked() {
                                trash = Some(w.marked.iter().cloned().collect());
                            }
                        });
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            for g in groups.iter().take(500) {
                                ui.label(egui::RichText::new(format!("{} × {}", g.files.len(), human(g.size))).strong().size(12.5));
                                for f in &g.files {
                                    ui.horizontal(|ui| {
                                        let mut on = w.marked.contains(f);
                                        if ui.checkbox(&mut on, "").changed() {
                                            if on {
                                                w.marked.insert(f.clone());
                                            } else {
                                                w.marked.remove(f);
                                            }
                                        }
                                        let r = ui.add(egui::Label::new(f.display().to_string()).truncate().sense(Sense::click()));
                                        if r.double_clicked() {
                                            acts.push(Act::Reveal(f.clone()));
                                        }
                                    });
                                }
                                ui.add_space(4.0);
                            }
                        });
                    }
                    Some(tools::Output::Space(sp)) => {
                        let large = w.tab == ToolTab::Large;
                        ui.label(format!("{}: {} insgesamt", sp.root.display(), human(sp.total)));
                        let rows: Vec<(PathBuf, u64, bool)> = if large {
                            sp.largest.iter().map(|(p, s)| (p.clone(), *s, false)).collect()
                        } else {
                            sp.children.clone()
                        };
                        let max = rows.first().map_or(1, |r| r.1.max(1));
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 26.0, rows.len(), |ui, range| {
                            for (p, sz, dir) in &rows[range] {
                                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                                if resp.hovered() {
                                    ui.painter().rect_filled(r, 4.0, th.hover);
                                }
                                let bar_w = 140.0;
                                let bar = Rect::from_min_size(pos2(r.right() - bar_w - 90.0, r.center().y - 4.0), vec2(bar_w, 8.0));
                                ui.painter().rect_filled(bar, 3.0, th.line);
                                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar_w * (*sz as f32 / max as f32), 8.0)), 3.0, th.accent);
                                let name = if large { p.display().to_string() } else { p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()) };
                                ui::draw(ui.painter(), Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(14.0, 14.0)), if *dir { Ic::NewFolder } else { Ic::Docs }, th.sub);
                                ui::paint_text(ui, r.left() + 26.0, r.center().y, &name, 13.5, th.text, bar.left() - r.left() - 34.0, false);
                                ui::paint_text(ui, r.right() - 8.0, r.center().y, &human(*sz), 12.5, th.sub, 80.0, true);
                                let resp = resp.on_hover_text(p.display().to_string());
                                if resp.double_clicked() {
                                    if *dir {
                                        acts.push(Act::Go(p.clone()));
                                    } else {
                                        acts.push(Act::Reveal(p.clone()));
                                    }
                                }
                            }
                        });
                    }
                }
            });
        if start {
            let a = PathBuf::from(w.a.trim());
            let b = PathBuf::from(w.b.trim());
            let ctx2 = ctx.clone();
            if !a.is_dir() {
                w.result = Some(tools::Output::Error(format!("Ordner nicht gefunden: {}", a.display())));
            } else {
                w.result = None;
                w.task = Some(match w.tab {
                    ToolTab::Compare if !b.is_dir() => {
                        w.result = Some(tools::Output::Error(format!("Ordner nicht gefunden: {}", b.display())));
                        None
                    }
                    ToolTab::Compare => {
                        let content = w.content;
                        Some(tools::Task::spawn("Vergleiche", &ctx2, move |p, c| tools::Output::Compare(tools::compare(&a, &b, content, p, c))))
                    }
                    ToolTab::Dups => {
                        let min = tools::parse_size(&w.min_size).unwrap_or(1);
                        Some(tools::Task::spawn("Suche Duplikate", &ctx2, move |p, c| tools::Output::Dups(tools::duplicates(&a, min, p, c))))
                    }
                    ToolTab::Space | ToolTab::Large => Some(tools::Task::spawn("Analysiere", &ctx2, move |p, c| tools::Output::Space(tools::space(&a, p, c)))),
                })
                .flatten();
            }
        }
        if let Some(v) = trash {
            w.marked.clear();
            if let Some(tools::Output::Dups(groups)) = &mut w.result {
                for g in groups.iter_mut() {
                    g.files.retain(|f| !v.contains(f));
                }
                groups.retain(|g| g.files.len() > 1);
            }
            acts.push(Act::Trash(v));
        }
        if open {
            self.tools = Some(w);
        } else if let Some(t) = &w.task {
            t.cancel.store(true, Relaxed);
        }
    }

    fn theme_action(&mut self, a: ThemeAct) {
        match a {
            ThemeAct::Select(id) => {
                self.theme_sel = Some(id);
                self.cfg_msg = None;
            }
            ThemeAct::Apply(id) => {
                let Some(p) = self.themes.get(&id) else { return };
                let mut st = self.st.clone();
                let dark = p.t.dark;
                if dark {
                    st.theme_dark = id.clone();
                } else {
                    st.theme_light = id.clone();
                }
                // Theme gilt für seinen Modus; ist gerade der andere aktiv, dorthin wechseln
                if dark != self.ctx.style().visuals.dark_mode {
                    st.theme = if dark { 2 } else { 1 };
                }
                self.set_settings(st);
                self.theme_sel = Some(id);
                self.cfg_msg = None;
            }
            ThemeAct::Reset => {
                let mut st = self.st.clone();
                st.theme_light = themes::DEFAULT_LIGHT.into();
                st.theme_dark = themes::DEFAULT_DARK.into();
                self.set_settings(st);
                self.theme_sel = None;
                self.cfg_msg = Some((false, vec!["Standard-Themes wiederhergestellt. Eigene Themes bleiben erhalten.".into()]));
            }
            ThemeAct::Delete(id) => {
                let name = self.themes.get(&id).map(|p| p.name.clone()).unwrap_or_default();
                match self.themes.remove_custom(&id) {
                    Ok(()) => {
                        let mut st = self.st.clone();
                        if st.theme_dark == id {
                            st.theme_dark = themes::DEFAULT_DARK.into();
                        }
                        if st.theme_light == id {
                            st.theme_light = themes::DEFAULT_LIGHT.into();
                        }
                        self.set_settings(st);
                        self.theme_sel = None;
                        self.cfg_msg = Some((false, vec![format!("Theme „{name}“ gelöscht.")]));
                    }
                    Err(e) => self.cfg_msg = Some((true, vec![e])),
                }
            }
            ThemeAct::Export(id) => {
                let Some(p) = self.themes.get(&id) else { return };
                let file = format!("{}.json", themes::custom_id(&p.name).trim_start_matches("custom:"));
                let Some(path) = rfd::FileDialog::new().add_filter("Foldex-Theme", &["json"]).set_file_name(file).save_file() else { return };
                let data = serde_json::to_vec_pretty(&themes::to_json(p)).unwrap_or_default();
                self.cfg_msg = Some(match themes::write_atomic(&path, &data) {
                    Ok(()) => (false, vec![format!("Theme gespeichert: {}", path.display())]),
                    Err(e) => (true, vec![format!("Theme konnte nicht gespeichert werden: {e}")]),
                });
            }
            ThemeAct::Import => {
                let Some(path) = rfd::FileDialog::new().add_filter("Foldex-Theme", &["json"]).pick_file() else { return };
                let c = themes::check_file(&path);
                match c.palette {
                    Err(errs) => {
                        let mut lines = vec![format!("„{}“ wurde nicht importiert:", path.file_name().unwrap_or_default().to_string_lossy())];
                        lines.extend(errs.into_iter().map(|e| format!("• {e}")));
                        self.cfg_msg = Some((true, lines));
                    }
                    Ok(p) => {
                        let name = p.name.clone();
                        let replaced = self.themes.get(&themes::custom_id(&name)).is_some();
                        match self.themes.add_custom(p) {
                            Ok(id) => {
                                let mut lines = vec![format!(
                                    "Theme „{name}“ {} – zur Vorschau ausgewählt, mit „Anwenden“ übernehmen.",
                                    if replaced { "aktualisiert" } else { "importiert" }
                                )];
                                lines.extend(c.warnings.into_iter().map(|w| format!("• {w}")));
                                self.theme_sel = Some(id);
                                self.cfg_msg = Some((false, lines));
                            }
                            Err(e) => self.cfg_msg = Some((true, vec![e])),
                        }
                    }
                }
            }
        }
    }

    fn export_settings(&mut self) {
        let name = format!("foldex-einstellungen-{}.json", chrono::Local::now().format("%Y-%m-%d"));
        let Some(path) = rfd::FileDialog::new().add_filter("Foldex-Einstellungen", &["json"]).set_file_name(name).save_file() else { return };
        let data = serde_json::to_vec_pretty(&settings::export(&self.st, &self.side, &self.themes, &self.keymap)).unwrap_or_default();
        self.cfg_msg = Some(match themes::write_atomic(&path, &data) {
            Ok(()) => (false, vec![format!("Einstellungen exportiert: {}", path.display())]),
            Err(e) => (true, vec![format!("Export fehlgeschlagen: {e}")]),
        });
    }

    fn pick_import(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Foldex-Einstellungen", &["json"]).pick_file() else { return };
        match settings::parse_import(path.clone(), &self.st, &self.side, &self.keymap) {
            Ok(imp) => {
                self.import = Some(imp);
                self.cfg_msg = None;
            }
            Err(errs) => {
                let mut lines = vec![format!(
                    "„{}“ wurde nicht importiert – deine Einstellungen sind unverändert:",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )];
                lines.extend(errs.into_iter().map(|e| format!("• {e}")));
                self.cfg_msg = Some((true, lines));
            }
        }
    }

    /// Vorschau eines geprüften Imports: zeigt alle Änderungen; erst „Importieren“ schreibt etwas.
    fn ui_import(&mut self, ctx: &egui::Context) {
        let Some(imp) = &self.import else { return };
        let th = self.th;
        let name_of = |id: &str| -> String {
            self.themes
                .get(id)
                .map(|p| p.name.clone())
                .or_else(|| imp.themes.iter().find(|p| themes::custom_id(&p.name) == id).map(|p| p.name.clone()))
                .unwrap_or_else(|| format!("{id} (unbekannt – Standard wird verwendet)"))
        };
        let mut changes = settings_diff(&self.st, &imp.st, &name_of);
        if imp.side.show_pc != self.side.show_pc {
            changes.push(format!("„Dieser PC“ in der Seitenleiste: {}", if imp.side.show_pc { "anzeigen" } else { "ausblenden" }));
        }
        if !same_pins(&imp.side.pins, &self.side.pins) {
            changes.push(format!("Schnellzugriff: {} → {} angeheftete Ordner", self.side.pins.len(), imp.side.pins.len()));
        }
        let key_changes = keymap::CMDS.iter().filter(|c| imp.keys.get(c.id) != self.keymap.get(c.id)).count();
        if key_changes > 0 {
            changes.push(format!("Tastenkürzel: {key_changes} Befehl(e) anders belegt"));
        }
        for p in &imp.themes {
            let id = themes::custom_id(&p.name);
            changes.push(format!(
                "Eigenes Theme „{}“ {}",
                p.name,
                if self.themes.get(&id).is_some() { "ersetzen" } else { "hinzufügen" }
            ));
        }
        let mut decision: Option<bool> = None;
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            decision = Some(false);
        }
        egui::Window::new("Einstellungen importieren")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(520.0);
                ui.label(egui::RichText::new(imp.path.display().to_string()).size(12.0).color(th.sub));
                ui.label(
                    egui::RichText::new(format!("Exportiert {} mit Foldex {}", imp.exported, imp.app_version))
                        .size(12.0)
                        .color(th.sub),
                );
                ui.add_space(8.0);
                if changes.is_empty() {
                    ui.label("Die Datei enthält keine Änderungen gegenüber deinen aktuellen Einstellungen.");
                } else {
                    ui.label(egui::RichText::new(format!("{} Änderung(en):", changes.len())).strong());
                    egui::ScrollArea::vertical().id_salt("imp_changes").max_height(220.0).show(ui, |ui| {
                        for c in &changes {
                            ui.label(format!("• {c}"));
                        }
                    });
                }
                if !imp.warnings.is_empty() {
                    ui.add_space(6.0);
                    message_box(ui, th, false, &imp.warnings);
                }
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Die aktuelle Konfiguration wird vorher gesichert: {}",
                        settings::backup_file().display()
                    ))
                    .size(12.0)
                    .color(th.sub),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!changes.is_empty(), egui::Button::new("Importieren")).clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Abbrechen").clicked() {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => self.apply_import(),
            Some(false) => {
                self.import = None;
                self.cfg_msg = Some((false, vec!["Import abgebrochen – nichts wurde geändert.".into()]));
            }
            None => {}
        }
    }

    fn apply_import(&mut self) {
        let Some(imp) = self.import.take() else { return };
        // 1. Sicherung – schlägt sie fehl, wird nichts verändert
        let backup = serde_json::to_vec_pretty(&settings::export(&self.st, &self.side, &self.themes, &self.keymap)).unwrap_or_default();
        if let Err(e) = themes::write_atomic(&settings::backup_file(), &backup) {
            self.cfg_msg = Some((true, vec![format!("Import abgebrochen: Sicherung konnte nicht geschrieben werden ({e}).")]));
            return;
        }
        // 2. Themes (bereits geprüft), 3. Seitenleiste, 4. Einstellungen – jeweils atomar geschrieben
        let mut errs = Vec::new();
        for p in imp.themes {
            if let Err(e) = self.themes.add_custom(p) {
                errs.push(e);
            }
        }
        if !errs.is_empty() {
            errs.insert(0, "Import abgebrochen – Einstellungen und Seitenleiste sind unverändert:".into());
            self.cfg_msg = Some((true, errs));
            return;
        }
        if imp.side.show_pc != self.side.show_pc || !same_pins(&imp.side.pins, &self.side.pins) {
            self.side = imp.side;
            self.side.save();
            if self.st.show_git {
                self.git_rx = Some(probe_git(&self.ctx, git_roots(&self.side)));
            }
        }
        if imp.keys != self.keymap {
            self.keymap = imp.keys;
            self.keymap.save();
        }
        self.set_settings(imp.st);
        self.theme_sel = None;
        self.cfg_msg = Some((false, vec!["Einstellungen importiert.".into()]));
    }

    // ── Drag & Drop ──
    fn frame_end(&mut self, ctx: &egui::Context, frame: &eframe::Frame, acts: &mut Vec<Act>) {
        let th = self.th;
        // Dateien von außen (Desktop, anderer Explorer) → in diesen Ordner kopieren
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            // Ziel am tatsächlichen Ablagepunkt (Ordner, Tab, Seitenleiste, Papierkorb)
            let ppp = ctx.pixels_per_point();
            let origin = ctx.input(|i| i.viewport().inner_rect).map_or(Pos2::ZERO, |r| r.min);
            let at = self.global_cursor().map(|(p, _)| pos2(p.x / ppp, p.y / ppp) - origin.to_vec2());
            let zone = at.and_then(|p| self.zones.iter().rev().find(|(r, _)| r.contains(p)).map(|(_, d)| d.clone()));
            let target = zone.or_else(|| self.drop_target.clone());
            // aus diesem Fenster herausgezogen und wieder zurück → wie internes Ziehen (verschieben auf
            // dem gleichen Laufwerk, sonst kopieren; im gleichen Ordner passiert nichts)
            let own = self.os_drag_src.take().is_some_and(|(src, t)| {
                if t.elapsed() > Duration::from_secs(3) {
                    return false;
                }
                let mut a = src.clone();
                let mut b = dropped.clone();
                a.sort();
                b.sort();
                a == b
            });
            match target {
                Some(Dest::Trash) => acts.push(Act::Trash(dropped)),
                Some(Dest::Ai) => {
                    if let Some(a) = &mut self.editor_ai {
                        a.attach(dropped);
                    }
                }
                Some(Dest::Pin) => {
                    for p in dropped.into_iter().filter(|p| p.is_dir()) {
                        acts.push(Act::Pin(p));
                    }
                }
                t => {
                    let dest = match t {
                        Some(Dest::Dir(p)) => p,
                        _ => self.pane().cwd.clone(),
                    };
                    if own && dropped.iter().all(|s| s.parent() == Some(dest.as_path())) {
                        self.status = "Liegt bereits in diesem Ordner".into();
                    } else if dropped.iter().any(|s| dest.starts_with(s)) {
                        self.status = "Ein Ordner kann nicht in sich selbst abgelegt werden".into();
                    } else if own {
                        let copy = dropped.first().is_some_and(|s| !same_volume(s, &dest));
                        self.spawn_transfer(dropped, dest, copy);
                    } else {
                        self.spawn_transfer(dropped, dest, true);
                    }
                }
            }
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let r = ctx.screen_rect().shrink(3.0);
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                Id::new("dropovl"),
            ));
            painter.rect_stroke(r, 6.0, Stroke::new(2.5, th.accent));
            painter.text(
                r.center_bottom() - vec2(0.0, 40.0),
                Align2::CENTER_CENTER,
                "Zum Kopieren hier ablegen",
                FontId::proportional(16.0),
                th.accent,
            );
        }
        // Reihenfolge der Schnellzugriff-Einträge per Drag ändern
        if let Some(from) = self.side_drag {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            let released = ctx.input(|i| i.pointer.any_released());
            if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
                let to = self.pin_rects.iter().filter(|r| r.center().y < p.y).count();
                if let Some(r0) = self.pin_rects.first() {
                    let ly = if to < self.pin_rects.len() {
                        self.pin_rects[to].top()
                    } else {
                        self.pin_rects.last().map_or(0.0, |r| r.bottom())
                    };
                    let painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        Id::new("pinline"),
                    ));
                    painter.line_segment(
                        [pos2(r0.left() + 8.0, ly), pos2(r0.right() - 8.0, ly)],
                        Stroke::new(2.0, th.accent),
                    );
                }
                if released {
                    let to = if to > from { to - 1 } else { to };
                    if to != from {
                        acts.push(Act::PinMove(from, to));
                    }
                }
            }
            if released {
                self.side_drag = None;
            }
        }
        // Reihenfolge der Cloud-Einträge per Drag ändern
        if let Some(from) = self.cloud_drag {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            let released = ctx.input(|i| i.pointer.any_released());
            if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
                let to = self.cloud_rects.iter().filter(|r| r.center().y < p.y).count();
                if let Some(r0) = self.cloud_rects.first() {
                    let ly = if to < self.cloud_rects.len() { self.cloud_rects[to].top() } else { self.cloud_rects.last().map_or(0.0, |r| r.bottom()) };
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("cloudline")));
                    painter.line_segment([pos2(r0.left() + 8.0, ly), pos2(r0.right() - 8.0, ly)], Stroke::new(2.0, th.accent));
                }
                if released {
                    let to = if to > from { to - 1 } else { to };
                    if to != from {
                        acts.push(Act::CloudMove(from, to));
                    }
                }
            }
            if released {
                self.cloud_drag = None;
            }
        }
        // Dateien im Fenster ziehen (auch zwischen den Panels und auf Tabs)
        let Some(paths) = self.drag.clone() else {
            return;
        };
        let (pos, released, down, esc, shift, ctrl) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.any_released(),
                i.pointer.primary_down(),
                i.key_pressed(Key::Escape),
                i.modifiers.shift,
                i.modifiers.command,
            )
        });
        if esc {
            self.drag = None;
            self.drag_first_dir = None;
            self.status = "Ziehen abgebrochen".into();
            return;
        }
        let outside = pos.is_none_or(|p| !ctx.screen_rect().contains(p));
        if outside && down {
            // Fenster verlassen → echtes OS-Drag&Drop (z. B. auf den Desktop) mit Datei-Symbol als Vorschau
            let image = drag_image(&paths, self.th.dark);
            let src = paths.clone();
            os_drag(frame, paths, image);
            self.os_drag_src = Some((src, Instant::now()));
            self.drag = None;
            self.drag_first_dir = None;
            return;
        }
        let copy_mode = |dest: &Path| -> bool {
            if ctrl {
                true
            } else if shift {
                false
            } else {
                paths.first().is_some_and(|s| !same_volume(s, dest))
            }
        };
        // Ziel prüfen: schon dort / Ordner in sich selbst
        let verdict = |d: &Path| -> Result<(), &'static str> {
            if paths.iter().any(|s| d.starts_with(s)) {
                Err("Ein Ordner kann nicht in sich selbst abgelegt werden")
            } else if paths.iter().all(|s| s.parent() == Some(d)) {
                Err("Liegt bereits in diesem Ordner")
            } else {
                Ok(())
            }
        };
        if let Some(p) = pos {
            let first = paths.first().cloned().unwrap_or_default();
            let first_dir = match &self.drag_first_dir {
                Some((fp, d)) if *fp == first => *d,
                _ => {
                    let d = fsx::is_dir(&first);
                    self.drag_first_dir = Some((first.clone(), d));
                    d
                }
            };
            let (ic, text, col) = match &self.drop_target {
                Some(Dest::Dir(d)) => {
                    let n = d.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| d.display().to_string());
                    match verdict(d) {
                        Err(why) => (Ic::Close, why.to_string(), th.danger),
                        Ok(()) if copy_mode(d) => (Ic::Copy, format!("Nach „{n}“ kopieren"), th.accent),
                        Ok(()) => (Ic::Cut, format!("Nach „{n}“ verschieben"), th.accent),
                    }
                }
                Some(Dest::Pin) => (Ic::Pin, "An Schnellzugriff anheften".to_string(), th.accent),
                Some(Dest::Ai) => (Ic::Spark, if self.editor_ai.as_ref().is_some_and(|a| a.image) { "Als Vorlage für die KI".to_string() } else { "An die KI-Nachricht anhängen".to_string() }, th.accent),
                Some(Dest::Trash) => (Ic::Trash, "In den Papierkorb".to_string(), th.danger),
                None => (Ic::Open, "Auf einen Ordner, Tab oder die Seitenleiste ziehen".to_string(), th.sub),
            };
            let name = first.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = ext_buf(&name);
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("dragghost")));
            let w = 280.0;
            let r = Rect::from_min_size(p + vec2(18.0, 14.0), vec2(w, 62.0));
            // bei mehreren Elementen: gestapelte Karten
            for k in (1..paths.len().min(3)).rev() {
                let o = vec2(4.0, 4.0) * k as f32;
                painter.rect(r.translate(o), 8.0, th.panel, Stroke::new(1.0, th.line));
            }
            painter.rect_filled(r.translate(vec2(0.0, 3.0)).expand(1.0), 9.0, Color32::from_black_alpha(45));
            painter.rect(r, 8.0, th.bg, Stroke::new(1.0, th.line));
            // Symbol links, Name rechts daneben
            let ir = Rect::from_min_size(pos2(r.left() + 10.0, r.top() + 8.0), vec2(26.0, 26.0));
            self.icons.paint(&painter, ir, if first_dir { IconKind::Dir } else { IconKind::File }, &name, ext.as_str(), false, || first.clone());
            let label = if paths.len() > 1 { format!("{name}  und {} weitere", paths.len() - 1) } else { name.clone() };
            text_trunc(&painter, pos2(ir.right() + 10.0, ir.center().y), &label, FontId::proportional(14.0), th.text, r.right() - 12.0 - (ir.right() + 10.0));
            if paths.len() > 1 {
                let b = Rect::from_center_size(ir.right_top() + vec2(-2.0, 2.0), vec2(20.0, 16.0));
                painter.rect_filled(b, 8.0, th.accent);
                painter.text(b.center(), Align2::CENTER_CENTER, paths.len().min(999).to_string(), FontId::proportional(10.5), Color32::WHITE);
            }
            // Aktion
            let line = Rect::from_min_max(pos2(r.left() + 6.0, r.bottom() - 24.0), pos2(r.right() - 6.0, r.bottom() - 4.0));
            painter.rect_filled(line, 5.0, col.gamma_multiply(0.12));
            ui::draw(&painter, Rect::from_center_size(pos2(line.left() + 13.0, line.center().y), vec2(13.0, 13.0)), ic, col);
            text_trunc(&painter, pos2(line.left() + 26.0, line.center().y), &text, FontId::proportional(12.5), col, line.width() - 32.0);
            if matches!(self.drop_target, Some(Dest::Dir(_))) {
                painter.text(
                    pos2(r.left() + 2.0, r.bottom() + 12.0),
                    Align2::LEFT_CENTER,
                    "Strg: kopieren  ·  Umschalt: verschieben  ·  Esc: abbrechen",
                    FontId::proportional(11.0),
                    th.sub,
                );
            }
            ctx.set_cursor_icon(if col == th.danger && !matches!(self.drop_target, Some(Dest::Trash)) {
                egui::CursorIcon::NotAllowed
            } else if matches!(self.drop_target, Some(Dest::Dir(_))) && copy_mode(match &self.drop_target {
                Some(Dest::Dir(d)) => d,
                _ => Path::new(""),
            }) {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::Grabbing
            });
            ctx.request_repaint();
        }
        if released {
            self.drag = None;
            self.drag_first_dir = None;
            match self.drop_target.clone() {
                Some(Dest::Dir(d)) => match verdict(&d) {
                    Ok(()) => {
                        let copy = copy_mode(&d);
                        self.spawn_transfer(paths, d, copy);
                    }
                    Err(why) => self.status = why.into(),
                },
                Some(Dest::Pin) => {
                    for p in paths {
                        acts.push(Act::Pin(p));
                    }
                }
                Some(Dest::Trash) => acts.push(Act::Trash(paths)),
                Some(Dest::Ai) => {
                    if let Some(a) = &mut self.editor_ai {
                        a.attach(paths);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(windows)]
fn os_drag(frame: &eframe::Frame, paths: Vec<PathBuf>, image: Vec<u8>) {
    let _ = drag::start_drag(
        frame,
        drag::DragItem::Files(paths),
        drag::Image::Raw(image),
        |_, _| {},
        drag::Options::default(),
    );
}
#[cfg(not(windows))]
fn os_drag(_frame: &eframe::Frame, _paths: Vec<PathBuf>, _image: Vec<u8>) {}

/// PNG des Datei- bzw. Ordnersymbols als Vorschau beim Ziehen aus dem Fenster heraus.
fn drag_image(paths: &[PathBuf], dark: bool) -> Vec<u8> {
    use resvg::tiny_skia;
    let Some(first) = paths.first() else { return Vec::new() };
    let name = first.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let icon = if fsx::is_dir(&first) {
        matico::folder_icon(&name)
    } else {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        matico::file_icon(&name, &ext)
    };
    let Some(img) = matico::raster(matico::themed(icon, dark), 64) else { return Vec::new() };
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    tiny_skia::IntSize::from_wh(64, 64)
        .and_then(|sz| tiny_skia::Pixmap::from_vec(bytes, sz))
        .and_then(|pm| pm.encode_png().ok())
        .unwrap_or_default()
}

impl Drop for App {
    fn drop(&mut self) {
        // Sitzung: Schließzeitpunkt vermerken – beim nächsten Start kommen die zuletzt offenen Fenster wieder
        if std::env::var_os("FOLDEX_SHOT").is_some() {
            // Bildschirmfoto-Lauf: nichts speichern
        } else if self.st.restore_session {
            session::close(self.win_state());
        } else {
            session::remove();
        }
        // letzte Fensterlage sichern (auch, wenn die Entprellung sie noch nicht geschrieben hat)
        if let (Some(pos), Some(size), None) = (self.session_geo.0, self.session_geo.1, std::env::var_os("FOLDEX_SHOT")) {
            let g = session::WinGeo { pos, size, maximized: self.session_geo.2, ppp: self.geo_saved.map_or(1.0, |g| g.ppp) };
            if self.geo_saved != Some(g) {
                session::save_geo(&g);
            }
        }
        self.dsize.save();
        ipc::remove_window();
        ipc::clear_drag();
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // lokalen Modell-Server nicht weiterlaufen lassen
        localllm::stop_server();
    }

    /// Eingaben der Explorer-KI einspeisen (ein Paket je Bild, damit Klicks als Drücken + Loslassen ankommen).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(batch) = self.inject.pop_front() {
            raw.events.extend(batch);
            ctx.request_repaint();
        }
    }

    /// Transparent: das kleine Vorschau-Fenster beim Herausziehen eines Tabs hat abgerundete Ecken.
    /// (Das Hauptfenster füllt alles selbst aus.)
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let _prof = prof::Frame::start();
        if self.boot {
            boot("erstes Bild beginnt");
        }
        if std::mem::take(&mut self.boot) && self.start_cfg.on_launch && std::env::var_os("FOLDEX_SHOT").is_none() {
            self.pane_mut().start = true;
        }
        let want = themes_for(&self.themes, &self.st, ctx.style().visuals.dark_mode);
        if want != self.th {
            self.th = want;
            apply_theme(ctx, &self.th);
        }
        let th = self.th;
        ui::set_theme(ctx, th);
        if let Some(r) = self.place.take() {
            editors::place_window(frame, r);
        }
        if self.hwnd == 0 {
            self.hwnd = editors::hwnd(frame);
        }
        self.poll();
        prof::mark("poll");
        self.poll_ai();
        prof::mark("poll_ai");
        self.track();
        prof::mark("track");
        self.ipc_poll(ctx);
        prof::mark("ipc");
        self.frame_begin(ctx);
        prof::mark("frame_begin");
        let mut acts: Vec<Act> = Vec::new();
        self.keys(ctx, &mut acts);
        prof::mark("keys");

        egui::TopBottomPanel::top("tabs")
            .frame(egui::Frame::none().fill(th.panel))
            .show_separator_line(false)
            .show(ctx, |ui| self.ui_tabs(ui, &mut acts));
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::none()
                    .fill(th.bg)
                    .inner_margin(Margin::symmetric(0.0, 5.0))
                    .stroke(Stroke::new(1.0, th.line)),
            )
            .show(ctx, |ui| self.ui_status(ui));
        prof::mark("tabs+status");
        egui::SidePanel::left("side")
            .frame(
                egui::Frame::none()
                    .fill(th.panel)
                    .inner_margin(Margin::symmetric(4.0, 0.0)),
            )
            .resizable(true)
            .default_width(250.0)
            .width_range(160.0..=420.0)
            .show_animated(ctx, self.side_visible, |ui| self.ui_sidebar(ui, &mut acts));
        prof::mark("sidebar");
        self.take_screenshots(ctx);
        self.ui_editor_ai(ctx, &mut acts);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(th.bg))
            .show(ctx, |ui| {
                self.ui_tiles(ui, &mut acts);
            });
        prof::mark("central");
        if std::env::var_os("FOLDEX_FRAMELOG").is_some() {
            // Entwicklung: welcher Tab war zu sehen (für die Messung des Tab-Wechsels)
            let t = format!("tab{}:{}", self.cur, self.tabs[self.cur].title().chars().take(12).collect::<String>().replace(' ', "_"));
            prof::mark(Box::leak(t.into_boxed_str()));
        }
        self.drive_drag(ctx);
        self.ui_snap_overlay(ctx);
        self.ui_windows(ctx);
        self.ui_locked(ctx);
        self.ui_rename_ask(ctx);
        self.ui_ai_cursor(ctx);
        self.ui_palette(ctx, &mut acts);
        self.ui_tools(ctx, &mut acts);
        self.ui_uninstall(ctx);
        self.ui_batch(ctx);
        self.ui_content_search(ctx);
        if let Some(w) = &mut self.printwin {
            if !w.ui(ctx, &th) {
                if let Some(m) = w.result.take() {
                    self.status = m;
                }
                self.printwin = None;
            }
        }
        self.ui_rules(ctx);
        self.pip_ui(ctx);
        prof::mark("windows");
        if let Some(w) = &mut self.share {
            if !w.ui(ctx, &th) {
                self.share = None;
            }
        }
        each_pane(&mut self.tabs, |p| {
            p.preview.tick(ctx, &th);
            // Vorschau-Tab: Wiedergabe anhalten, sobald der Tab nicht mehr sichtbar ist
            if let Some(PaneView::View(v)) = &mut p.extra {
                v.pv.tick(ctx, &th);
            }
        });
        prof::mark("share+tick");
        self.session_tick(ctx);
        self.dev_shot(ctx);
        prof::mark("session+shot");
        self.frame_end(ctx, frame, &mut acts);
        prof::mark("frame_end");
        for a in acts {
            if std::env::var_os("FOLDEX_FRAMELOG").is_some() {
                prof::mark(Box::leak(format!("act:{:?}", std::mem::discriminant(&a)).into_boxed_str()));
            }
            self.apply(a);
            prof::mark("apply");
        }
        self.apply_ai_effects();
        {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = N.fetch_add(1, Relaxed);
            if n < 4 {
                boot(&format!("Bild {} fertig", n + 1));
            }
            static LISTED: AtomicBool = AtomicBool::new(false);
            if !LISTED.load(Relaxed) && !self.pane().is_loading() && !self.pane().items.is_empty() {
                LISTED.store(true, Relaxed);
                boot(&format!("Ordnerliste sichtbar (Bild {})", n + 1));
            }
        }
        // Fenstertitel = aktueller Ordner
        let title = format!("{} – Foldex", self.tabs[self.cur].title());
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }
}

// ───────────────────────────── Benchmark (foldex --bench <Ordner>) ─────────────────────────────

fn rss_mb() -> String {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("VmRSS")).map(|l| {
                l.split_whitespace()
                    .nth(1)
                    .unwrap_or("0")
                    .parse::<f64>()
                    .unwrap_or(0.0)
                    / 1024.0
            })
        })
        .map_or("n/a".into(), |v| format!("{v:.0} MB"))
}

fn bench(root: &str) {
    let root = PathBuf::from(root);
    let dest = std::env::temp_dir().join("foldex-bench.fci");
    let t = Instant::now();
    let ix = index::build(&root, &dest, &AtomicUsize::new(0), &AtomicBool::new(false))
        .expect("Index-Bau fehlgeschlagen");
    let size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    println!(
        "Index: {} Einträge in {:.2} s | Datei {} | RSS danach {}",
        ix.len(),
        t.elapsed().as_secs_f64(),
        human(size),
        rss_mb()
    );
    for q in [
        "a",
        "lib",
        "readme",
        "ext:so",
        "config !conf",
        "*.h",
        "is:dir share",
        "rdme",
    ] {
        let t = Instant::now();
        let r = ix.search(&Query::parse(q), 5000);
        println!(
            "  {:<16} {:>7} Treffer in {:>7.2} ms{}",
            q,
            r.total,
            t.elapsed().as_secs_f64() * 1000.0,
            if r.capped { " (gekappt)" } else { "" }
        );
    }
    let _ = fs::remove_file(dest);
}

/// Fenster-/Taskleisten-Icon: von build.rs aus assets/app.ico (bzw. app.svg) vorbereitet.
/// Linux/BSD: .desktop-Datei und Symbol unter ~/.local/share (Ordner öffnen „mit Foldex“ inklusive)
fn install_desktop() -> Result<PathBuf, String> {
    let data = dirs::data_dir().ok_or("kein Datenordner")?;
    let icon = app_icon().ok_or("Symbol fehlt")?;
    let icon_path = data.join("icons").join("hicolor").join("256x256").join("apps").join("foldex.png");
    fs::create_dir_all(icon_path.parent().unwrap()).map_err(|e| e.to_string())?;
    let img = image::RgbaImage::from_raw(icon.width, icon.height, icon.rgba).ok_or("Symbol unlesbar")?;
    image::imageops::resize(&img, 256, 256, image::imageops::FilterType::Lanczos3).save(&icon_path).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let desk = data.join("applications").join("foldex.desktop");
    fs::create_dir_all(desk.parent().unwrap()).map_err(|e| e.to_string())?;
    let text = format!(
        "[Desktop Entry]
Type=Application
Name=Foldex
GenericName=Dateimanager
Comment=Dateien, Cloud und KI an einem Ort
Exec=\"{}\" %F
Icon={}
Terminal=false
Categories=System;FileTools;FileManager;Utility;
MimeType=inode/directory;
StartupWMClass=foldex
",
        exe.display(),
        icon_path.display()
    );
    fs::write(&desk, text).map_err(|e| e.to_string())?;
    Ok(desk)
}

fn app_icon() -> Option<egui::IconData> {
    let blob: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon.rgba"));
    let w = u32::from_le_bytes(blob.get(0..4)?.try_into().ok()?);
    let h = u32::from_le_bytes(blob.get(4..8)?.try_into().ok()?);
    let rgba = blob.get(8..)?.to_vec();
    (rgba.len() == (w * h * 4) as usize).then_some(egui::IconData { rgba, width: w, height: h })
}

/// Das Programm hieß früher „FileCore“: Einstellungen, Themes, Schlüssel, Verlauf und Caches einmalig in die
/// Foldex-Ordner übernehmen (umbenennen; scheitert das, kopieren). Vorhandene Foldex-Daten bleiben unangetastet.
fn migrate_old_name() {
    for base in [dirs::config_dir(), dirs::cache_dir()].into_iter().flatten() {
        let (old, new) = (base.join("filecore"), base.join("foldex"));
        if !old.is_dir() {
            continue;
        }
        let _ = fs::create_dir_all(&new);
        // Eintrag für Eintrag: nur übernehmen, was es unter Foldex noch nicht gibt
        for e in fs::read_dir(&old).into_iter().flatten().flatten() {
            let t = new.join(e.file_name());
            if t.exists() {
                // Laufzeitdaten der Fenster-Kommunikation werden nicht gebraucht
                if e.file_name() == "ipc" {
                    let _ = fs::remove_dir_all(e.path());
                }
                continue;
            }
            if fs::rename(e.path(), &t).is_err() {
                // Verschieben nicht möglich (z. B. gesperrt) → kopieren; Original nur nach erfolgreicher Kopie entfernen
                let copied = if e.path().is_dir() { copy_dir(&e.path(), &t) } else { fs::copy(e.path(), &t).map(|_| ()) };
                if copied.is_ok() {
                    let _ = if e.path().is_dir() { fs::remove_dir_all(e.path()) } else { fs::remove_file(e.path()) };
                }
            }
        }
        // alten Ordner entfernen, wenn alles übernommen wurde
        let _ = fs::remove_dir(&old);
    }
}

/// Gespeicherte Fensterlage (Punkte, Skalierung `ppp`) prüfen: Liegt die Titelleiste auf einem Bildschirm,
/// kommt die Lage zurück – Größe und Position notfalls in dessen Arbeitsbereich eingepasst; sonst `None`.
#[cfg(windows)]
fn fit_to_monitors(pos: [f32; 2], size: [f32; 2], ppp: f32) -> Option<([f32; 2], [f32; 2])> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONULL};
    let k = if ppp > 0.1 { ppp } else { 1.0 };
    // mittlerer Teil der Titelleiste (dort greift man das Fenster)
    let bar = RECT {
        left: ((pos[0] + 40.0) * k) as i32,
        top: (pos[1] * k) as i32,
        right: ((pos[0] + (size[0] - 40.0).max(80.0)) * k) as i32,
        bottom: ((pos[1] + 30.0) * k) as i32,
    };
    unsafe {
        let m = MonitorFromRect(&bar, MONITOR_DEFAULTTONULL);
        if m.is_null() {
            return None;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(m, &mut mi) == 0 {
            return Some((pos, size));
        }
        let w = mi.rcWork;
        let (wl, wt, wr, wb) = (w.left as f32 / k, w.top as f32 / k, w.right as f32 / k, w.bottom as f32 / k);
        // Rahmen und Titelleiste grob mit einrechnen
        let size = [size[0].min(wr - wl - 16.0).max(700.0), size[1].min(wb - wt - 40.0).max(420.0)];
        let pos = [pos[0].clamp(wl - 8.0, (wr - size[0] - 8.0).max(wl)), pos[1].clamp(wt, (wb - size[1] - 32.0).max(wt))];
        Some((pos, size))
    }
}

#[cfg(not(windows))]
fn fit_to_monitors(pos: [f32; 2], size: [f32; 2], _ppp: f32) -> Option<([f32; 2], [f32; 2])> {
    (pos[0] > -2000.0 && pos[1] > -200.0 && pos[0] < 16000.0 && pos[1] < 9000.0).then_some((pos, size))
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)?.flatten() {
        let t = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &t)?;
        } else {
            fs::copy(e.path(), t)?;
        }
    }
    Ok(())
}

/// Entwicklung: Startzeiten messen (FOLDEX_STARTLOG=<datei>) – Millisekunden seit Programmstart je Schritt
fn boot(step: &str) {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let t0 = *T0.get_or_init(Instant::now);
    if let Some(f) = std::env::var_os("FOLDEX_STARTLOG") {
        use std::io::Write;
        if let Ok(mut o) = fs::OpenOptions::new().create(true).append(true).open(f) {
            let _ = writeln!(o, "{:>6} ms  {step}", t0.elapsed().as_millis());
        }
    }
}

fn main() -> eframe::Result<()> {
    boot("main");
    migrate_old_name();
    // gestartete Programme (Modelle, Werkzeuge) erben das: fehlt eine DLL, gibt es kein Windows-Fehlerfenster,
    // sondern einen Exit-Code, den Foldex verständlich meldet
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Diagnostics::Debug::{SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX};
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
    }
    let args: Vec<String> = std::env::args().collect();
    // für den Installer: KI-Komponenten auflisten bzw. installieren (ohne Fenster)
    if let Some(code) = aisetup::cli(&args) {
        std::process::exit(code);
    }
    // Linux: Eintrag im Startmenü (Anwendungen) mit Symbol anlegen – ruft build.sh nach dem Bauen auf
    if args.get(1).map(String::as_str) == Some("--install-desktop") {
        match install_desktop() {
            Ok(p) => println!("Startmenü-Eintrag: {}", p.display()),
            Err(e) => eprintln!("Startmenü-Eintrag fehlgeschlagen: {e}"),
        }
        return Ok(());
    }
    if args.len() >= 3 && args[1] == "--bench" {
        bench(&args[2]);
        return Ok(());
    }
    let mut launch = parse_args(&args);
    // Normaler Start ohne andere offene Fenster: zuletzt offene Fenster wiederherstellen
    let alone = ipc::other_windows().is_empty();
    // in Foldex angemeldete Cloud-Konten wieder als Laufwerk einbinden (nur das erste Fenster)
    if alone && std::env::var_os("FOLDEX_SHOT").is_none() {
        cloudlogin::mount_all();
    }
    // Entwicklung: FOLDEX_SHOT_MOUNT=1 bindet die Konten vor dem Bildschirmfoto-Lauf ein (und wartet darauf)
    if std::env::var_os("FOLDEX_SHOT_MOUNT").is_some() {
        for a in cloudlogin::accounts() {
            let _ = cloudlogin::mount(&a);
        }
    }
    // (nicht bei Bildschirmfoto-Läufen: das Einlesen entfernt die gespeicherten Fenster)
    if launch.paths.is_empty() && launch.session.is_none() && Settings::load().restore_session && alone && std::env::var_os("FOLDEX_SHOT").is_none() {
        let mut all = session::take_all();
        if !all.is_empty() {
            launch.session = Some(all.remove(0));
            if let Ok(exe) = std::env::current_exe() {
                for (k, s) in all.iter().enumerate() {
                    if let Some(f) = session::handoff(s, k) {
                        let _ = std::process::Command::new(&exe).arg("--session").arg(f).spawn();
                    }
                }
            }
        }
    }
    let mut vp = egui::ViewportBuilder::default()
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([700.0, 420.0])
        .with_title("Foldex");
    if let Some(icon) = app_icon() {
        vp = vp.with_icon(Arc::new(icon));
    }
    // nur für Bildschirmfotos (FOLDEX_SHOT): Fenstergröße „Breite,Höhe“
    let shot_size = std::env::var("FOLDEX_SHOT_SIZE").ok().and_then(|v| v.split_once(',').and_then(|(w, h)| Some([w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?])));
    if let Some(sz) = shot_size {
        vp = vp.with_inner_size(sz);
    } else if let Some(p) = launch.pos {
        vp = vp.with_position(p); // neues Fenster erscheint dort, wo der Tab losgelassen wurde
    } else {
        // Fensterlage: aus der Sitzung dieses Fensters, sonst die zuletzt benutzte (nur als einziges Fenster,
        // sonst lägen neue Fenster genau über dem offenen)
        let saved = session::load_geo();
        let geo = launch
            .session
            .as_ref()
            .and_then(|s| Some(session::WinGeo { pos: s.pos?, size: s.size?, maximized: s.maximized, ppp: saved.map_or(1.0, |g| g.ppp) }))
            .or(saved.filter(|_| alone));
        match geo {
            Some(g) => {
                let size = [g.size[0].max(700.0), g.size[1].max(420.0)];
                // Bildschirme geändert? Lage nur übernehmen, wenn die Titelleiste auf einem Bildschirm liegt
                match fit_to_monitors(g.pos, size, g.ppp) {
                    Some((pos, size)) => vp = vp.with_position(pos).with_inner_size(size),
                    None => vp = vp.with_inner_size([1280.0, 760.0]),
                }
                vp = vp.with_maximized(g.maximized);
                launch.maximized = g.maximized;
            }
            None => vp = vp.with_inner_size([1280.0, 760.0]),
        }
    }
    let opts = eframe::NativeOptions {
        viewport: vp,
        ..Default::default()
    };
    boot("Fenster wird erzeugt");
    eframe::run_native(
        "Foldex",
        opts,
        Box::new(move |cc| Ok(Box::new(App::new(cc.egui_ctx.clone(), launch)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_zones() {
        let r = Rect::from_min_size(pos2(100.0, 50.0), vec2(1000.0, 600.0));
        assert_eq!(Zone::at(pos2(120.0, 350.0), r), Some(Zone::Left));
        assert_eq!(Zone::at(pos2(1090.0, 350.0), r), Some(Zone::Right));
        assert_eq!(Zone::at(pos2(600.0, 60.0), r), Some(Zone::Top));
        assert_eq!(Zone::at(pos2(600.0, 640.0), r), Some(Zone::Bottom));
        assert_eq!(Zone::at(pos2(600.0, 350.0), r), None); // Mitte: kein Andocken
        assert_eq!(Zone::at(pos2(5.0, 5.0), r), None); // außerhalb
        let l = Zone::Left.rect(r);
        assert_eq!(l.width(), 500.0);
        assert_eq!(Zone::Bottom.rect(r).top(), r.center().y);
    }

    #[test]
    fn pane_specs_cross_windows() {
        // Terminal-/Git-Kacheln überstehen den Weg über die Befehlszeile in ein neues Fenster
        let a: Vec<String> = ["x", "--open", r"fc-term:cmd.exe*C:\Temp", "--open", r"fc-git:C:\Repo", "--open", "kein-ordner"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let l = parse_args(&a);
        assert_eq!(l.paths.len(), 2);
        assert!(is_spec(&l.paths[0].to_string_lossy()) && is_spec(&l.paths[1].to_string_lossy()));
        assert_eq!(shell_name(&[("CMD".into(), "cmd.exe".into())], "cmd.exe"), "CMD");
        assert_eq!(shell_name(&[], r"C:\x\pwsh.exe"), "pwsh");
    }

    #[test]
    fn tabs_in_tile() {
        let mk = |n: &str| {
            let mut p = Pane::default();
            p.cwd = PathBuf::from(n);
            p
        };
        let order = |p: &Pane| -> Vec<String> {
            let pos = p.stack_pos.min(p.stack.len());
            let mut v: Vec<String> = p.stack.iter().map(|s| s.cwd.display().to_string()).collect();
            v.insert(pos, format!("[{}]", p.cwd.display()));
            v
        };
        let mut slot = mk("a");
        sub_add(&mut slot, mk("b"));
        sub_add(&mut slot, mk("c"));
        assert_eq!(order(&slot), ["a", "b", "[c]"]);
        sub_switch(&mut slot, 0);
        assert_eq!(order(&slot), ["[a]", "b", "c"]);
        // sichtbaren Tab herauslösen → Nachbar wird sichtbar, Reihenfolge bleibt
        let a = sub_take(&mut slot, 0).unwrap();
        assert_eq!(a.cwd, PathBuf::from("a"));
        assert_eq!(order(&slot), ["[b]", "c"]);
        // ganze Gruppe in eine andere Kachel aufnehmen
        let mut other = mk("x");
        sub_add(&mut other, slot);
        assert_eq!(order(&other), ["x", "[b]", "c"]);
        // über Fenstergrenzen: Gruppe als Beschreibung
        let spec = pane_spec(&other).to_string_lossy().into_owned();
        assert_eq!(spec, "fc-group:1|x|b|c");
        assert!(is_spec(&spec));
        // Reihenfolge ändern: verschobener Tab wird sichtbar
        sub_move(&mut other, 0, 2);
        assert_eq!(order(&other), ["b", "c", "[x]"]);
        sub_move(&mut other, 1, 0);
        assert_eq!(order(&other), ["[c]", "b", "x"]);
        // letzter Tab lässt sich nicht herauslösen (Kachel bliebe leer)
        let mut single = mk("s");
        assert!(sub_take(&mut single, 0).is_none());
    }

    #[test]
    fn compass_targets() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let c = compass(r);
        for (i, (z, b)) in c.iter().enumerate() {
            assert!(r.contains_rect(*b));
            assert!(!b.contains(r.center()), "Mitte bleibt frei");
            assert!(c.iter().enumerate().all(|(k, (_, o))| k == i || !o.intersects(*b)), "{z:?} überlappt");
        }
        assert_eq!(c[0].0, Zone::Left);
    }

    #[test]
    fn launch_args() {
        // vorhandene Ordner auf jedem System; nicht vorhandene werden ignoriert
        let tmp = std::env::temp_dir();
        let dirs = [tmp.clone(), tmp.parent().unwrap_or(&tmp).to_path_buf(), std::env::current_dir().unwrap()];
        let mut a: Vec<String> = ["x", "--pos", "10,20", "--layout", "H0.5(0,1)", "--place", "100,50,1200,800", "--open", "/gibt/es/nicht"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        for d in &dirs {
            a.push("--open".into());
            a.push(d.display().to_string());
        }
        let l = parse_args(&a);
        assert_eq!(l.pos, Some([10.0, 20.0]));
        assert_eq!(l.layout.as_deref(), Some("H0.5(0,1)"));
        assert_eq!(l.place, Some([100, 50, 1200, 800]));
        assert_eq!(l.paths.len(), 3);
    }
}
