//! Repository-Ansicht (eigener Tab): Status, Diff, Stage/Unstage, Commit, Pull/Push/Fetch, Branches.
//! Jeder git-Aufruf läuft im Hintergrund; destruktive Aktionen (Verwerfen, Branch löschen, Wechsel
//! mit ungesicherten Änderungen, Merge abbrechen) müssen bestätigt werden. Nicht verfolgte Dateien
//! werden beim Verwerfen in den Papierkorb gelegt statt gelöscht.
//!
//! Commit: Übersicht der enthaltenen Dateien (geändert, neu, gelöscht, umbenannt) mit Diff-Vorschau, eine
//! Commit-Nachricht ohne KI per Knopfdruck oder eine KI-Zusammenfassung mit frei wählbarem Anbieter (eigene Modelle
//! oder installierte Agenten). Ist nichts gestaged, committet „Alles committen“ sämtliche Änderungen.

use crate::git::{self, Change, St, Status};
use crate::themes::Theme;
use crate::ui::{self, Ic};
use eframe::egui::{self, pos2, vec2, Color32, FontId, Rect, Rounding, Sense};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

enum Confirm {
    Discard(Vec<Change>),
    Checkout(String),
    DeleteBranch(String),
    AbortMerge,
}

pub enum GitOut {
    OpenFile(PathBuf),
    Reveal(PathBuf),
}

/// KI-Anbieter für Commit-Nachrichten.
#[derive(Clone)]
pub enum CommitAi {
    /// eigenes Modell (Anthropic-API, LM Studio, OpenAI-kompatibel …)
    Model { id: String, name: String, cfg: crate::ai::Cfg },
    /// installierter Agent (Claude Code, Codex, Gemini CLI …)
    Agent { key: String, name: String, program: String },
}

impl CommitAi {
    pub fn id(&self) -> String {
        match self {
            CommitAi::Model { id, .. } => format!("model:{id}"),
            CommitAi::Agent { key, .. } => format!("agent:{key}"),
        }
    }
    pub fn name(&self) -> &str {
        match self {
            CommitAi::Model { name, .. } | CommitAi::Agent { name, .. } => name,
        }
    }
}

/// Zuletzt gewählter Anbieter (gilt für alle Git-Ansichten dieser Sitzung).
static LAST_AI: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub struct GitView {
    pub root: PathBuf,
    status: Option<Status>,
    err: Option<String>,
    status_rx: Option<Receiver<(Result<Status, String>, Vec<(String, bool)>, Vec<(String, String)>)>>,
    /// Ordner ist (noch) kein Repository → Ansicht „Erstellen / Klonen“
    no_repo: bool,
    /// 0 = Änderungen, 1 = Verlauf
    tab: u8,
    commits: Vec<git::Commit>,
    commits_rx: Option<Receiver<Result<Vec<git::Commit>, String>>>,
    commit_sel: Option<String>,
    commit_rx: Option<Receiver<Vec<String>>>,
    commit_diff: Vec<String>,
    remotes: Vec<(String, String)>,
    remote_url: String,
    // Repository erstellen / klonen
    ignore_tpl: usize,
    first_commit: bool,
    clone_url: String,
    /// nach erfolgreichem Klonen: neues Repository anzeigen
    after_root: Option<PathBuf>,
    branches: Vec<(String, bool)>,
    sel: Option<(String, bool)>,
    diff: Vec<String>,
    diff_rx: Option<Receiver<Vec<String>>>,
    msg: String,
    busy: Option<(String, Receiver<Result<String, String>>)>,
    log: Option<(bool, String)>,
    confirm: Option<Confirm>,
    new_branch: String,
    /// KI-Commit-Nachricht in Arbeit
    ai_rx: Option<Receiver<Result<String, String>>>,
    /// Übersicht: Gesamt-Diff aller enthaltenen Änderungen
    all_diff: Vec<String>,
    all_rx: Option<Receiver<Vec<String>>>,
}

impl GitView {
    pub fn new(root: PathBuf, ctx: &egui::Context) -> GitView {
        let mut v = GitView {
            root,
            status: None,
            err: None,
            status_rx: None,
            branches: Vec::new(),
            sel: None,
            diff: Vec::new(),
            diff_rx: None,
            msg: String::new(),
            busy: None,
            log: None,
            confirm: None,
            new_branch: String::new(),
            no_repo: false,
            tab: 0,
            commits: Vec::new(),
            commits_rx: None,
            commit_sel: None,
            commit_rx: None,
            commit_diff: Vec::new(),
            remotes: Vec::new(),
            remote_url: String::new(),
            ignore_tpl: 1,
            first_commit: false,
            clone_url: String::new(),
            after_root: None,
            ai_rx: None,
            all_diff: Vec::new(),
            all_rx: None,
        };
        v.refresh(ctx);
        v
    }

    pub fn title(&self) -> String {
        let name = self.root.file_name().map_or_else(|| self.root.display().to_string(), |n| n.to_string_lossy().into_owned());
        match &self.status {
            Some(s) if !s.branch.is_empty() => format!("Git: {name} ({})", s.branch),
            _ => format!("Git: {name}"),
        }
    }

    /// Status und Branches im Hintergrund neu laden (git-Aufrufe nie im UI-Thread).
    pub fn refresh(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let (root, c2) = (self.root.clone(), ctx.clone());
        std::thread::spawn(move || {
            let st = git::status(&root);
            let (b, r) = if st.is_ok() { (git::branches(&root), git::remotes(&root)) } else { (Vec::new(), Vec::new()) };
            let _ = tx.send((st, b, r));
            c2.request_repaint();
        });
        self.status_rx = Some(rx);
        if self.tab == 1 {
            self.load_log(ctx);
        }
    }

    fn load_log(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let (root, ctx) = (self.root.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(git::log(&root, 300));
            ctx.request_repaint();
        });
        self.commits_rx = Some(rx);
    }

    fn load_commit(&mut self, hash: String, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let (root, ctx) = (self.root.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(git::show(&root, &hash).lines().map(str::to_string).collect());
            ctx.request_repaint();
        });
        self.commit_rx = Some(rx);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.status_rx {
            if let Ok((r, branches, remotes)) = rx.try_recv() {
                self.status_rx = None;
                self.branches = branches;
                self.remotes = remotes;
                self.no_repo = matches!(&r, Err(e) if e.contains("Kein Git-Repository") || e.to_lowercase().contains("not a git repository"));
                if self.no_repo {
                    self.status = None;
                    self.err = None;
                    return;
                }
                match r {
                    Ok(s) => {
                        // Auswahl beibehalten, wenn die Datei noch Änderungen hat
                        if let Some((p, staged)) = self.sel.clone() {
                            let still = s.changes.iter().any(|c| c.path == p && if staged { c.staged.is_some() } else { c.unstaged.is_some() });
                            if still {
                                self.load_diff(&s, &p, staged, ctx);
                            } else {
                                self.sel = None;
                                self.diff.clear();
                            }
                        }
                        self.status = Some(s);
                        self.err = None;
                    }
                    Err(e) => self.err = Some(e),
                }
            }
        }
        if let Some(rx) = &self.diff_rx {
            if let Ok(d) = rx.try_recv() {
                self.diff = d;
                self.diff_rx = None;
            }
        }
        if let Some(rx) = &self.commits_rx {
            if let Ok(r) = rx.try_recv() {
                self.commits_rx = None;
                match r {
                    Ok(v) => self.commits = v,
                    // leeres Repository: noch keine Commits
                    Err(_) => self.commits.clear(),
                }
            }
        }
        if let Some(rx) = &self.commit_rx {
            if let Ok(d) = rx.try_recv() {
                self.commit_diff = d;
                self.commit_rx = None;
            }
        }
        if let Some(rx) = &self.ai_rx {
            match rx.try_recv() {
                Ok(Ok(m)) => {
                    self.msg = m;
                    self.ai_rx = None;
                }
                Ok(Err(e)) => {
                    self.log = Some((true, format!("KI-Commit-Nachricht fehlgeschlagen:\n{e}")));
                    self.ai_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.ai_rx = None,
                Err(_) => {}
            }
        }
        if let Some(rx) = &self.all_rx {
            if let Ok(d) = rx.try_recv() {
                self.all_diff = d;
                self.all_rx = None;
            }
        }
        if let Some((label, rx)) = &self.busy {
            if let Ok(r) = rx.try_recv() {
                let label = label.clone();
                self.busy = None;
                self.log = Some(match r {
                    Ok(out) => {
                        let out = out.trim();
                        (false, if out.is_empty() { format!("{label}: erledigt") } else { format!("{label}: {out}") })
                    }
                    Err(e) => {
                        self.after_root = None;
                        (true, format!("{label} fehlgeschlagen:\n{e}"))
                    }
                });
                if let Some(r) = self.after_root.take() {
                    self.root = r;
                }
                self.refresh(ctx);
            }
        }
    }

    fn load_diff(&mut self, s: &Status, path: &str, staged: bool, ctx: &egui::Context) {
        let Some(c) = s.changes.iter().find(|c| c.path == path).cloned() else { return };
        let (tx, rx) = mpsc::channel();
        let (root, ctx) = (self.root.clone(), ctx.clone());
        std::thread::spawn(move || {
            let d = git::diff(&root, &c, staged);
            let _ = tx.send(d.lines().map(str::to_string).collect());
            ctx.request_repaint();
        });
        self.diff_rx = Some(rx);
    }

    /// Führt eine oder mehrere git-Befehlsfolgen im Hintergrund aus.
    fn exec(&mut self, label: &str, cmds: Vec<Vec<String>>, ctx: &egui::Context) {
        let root = self.root.clone();
        self.exec_in(root, label, cmds, ctx);
    }

    fn exec_in(&mut self, root: PathBuf, label: &str, cmds: Vec<Vec<String>>, ctx: &egui::Context) {
        if self.busy.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut out = String::new();
            let mut res = Ok(String::new());
            for c in cmds {
                let args: Vec<&str> = c.iter().map(String::as_str).collect();
                match git::run(&root, &args) {
                    Ok(o) => out.push_str(&o),
                    Err(e) => {
                        res = Err(e);
                        break;
                    }
                }
            }
            let _ = tx.send(res.map(|_| out));
            ctx.request_repaint();
        });
        self.busy = Some((label.to_string(), rx));
    }

    /// KI schreibt die Commit-Nachricht aus Diff und Dateiliste (im Hintergrund).
    fn gen_message(&mut self, ai: CommitAi, s: &Status, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let (root, changes, ctx) = (self.root.clone(), s.changes.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = (|| {
                // Agenten, die den Text als Argument bekommen, vertragen nur kürzere Eingaben
                let max = match &ai {
                    CommitAi::Agent { key, .. } if !matches!(key.as_str(), "claude" | "codex") => 12_000,
                    _ => 60_000,
                };
                let context = git::ai_context(&root, &changes, max);
                let reply = match &ai {
                    CommitAi::Model { cfg, .. } => crate::ai::quick(cfg, git::COMMIT_SYSTEM, &context)?,
                    CommitAi::Agent { key, program, .. } => crate::agents::oneshot(key, program, &format!("{}\n\n{context}", git::COMMIT_SYSTEM), &ctx)?,
                };
                let m = git::clean_message(&reply);
                if m.is_empty() {
                    Err("Die KI hat keine Nachricht geliefert.".to_string())
                } else {
                    Ok(m)
                }
            })();
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.ai_rx = Some(rx);
    }

    /// Gesamt-Diff dessen, was committet würde (gestaged, sonst alles inkl. neuer Dateien).
    fn load_all_diff(&mut self, s: &Status, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let (root, changes, ctx) = (self.root.clone(), s.changes.clone(), ctx.clone());
        std::thread::spawn(move || {
            let (staged, _) = git::commit_set(&changes);
            let mut out = String::new();
            for c in changes.iter().filter(|c| if staged { c.staged.is_some() } else { c.unstaged.is_some() || c.staged.is_some() }) {
                let d = git::diff(&root, c, staged && c.staged.is_some());
                if c.unstaged == Some(St::Untracked) && !staged {
                    out.push_str(&format!("diff --git a/{0} b/{0}\nnew file\n", c.path));
                }
                out.push_str(&d);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                if out.len() > 2_000_000 {
                    out.push_str("… (gekürzt)\n");
                    break;
                }
            }
            let _ = tx.send(out.lines().map(str::to_string).collect());
            ctx.request_repaint();
        });
        self.all_rx = Some(rx);
    }

    fn args(a: &[&str], paths: &[String]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).chain(std::iter::once("--".to_string())).chain(paths.iter().cloned()).collect()
    }

    pub fn pull(&mut self, ctx: &egui::Context) {
        self.exec("Pull", vec![vec!["pull".into(), "--no-edit".into()]], ctx);
    }

    pub fn push(&mut self, ctx: &egui::Context) {
        let cmd = match &self.status {
            Some(s) if s.upstream.is_none() && !s.branch.is_empty() => {
                vec!["push".into(), "-u".into(), "origin".into(), s.branch.clone()]
            }
            _ => vec!["push".into()],
        };
        self.exec("Push", vec![cmd], ctx);
    }

    pub fn fetch(&mut self, ctx: &egui::Context) {
        self.exec("Fetch", vec![vec!["fetch".into(), "--prune".into()]], ctx);
    }

    fn discard(&mut self, changes: Vec<Change>, ctx: &egui::Context) {
        // Nicht verfolgte Dateien: in den Papierkorb (wiederherstellbar), verfolgte: git restore
        let mut tracked = Vec::new();
        let mut trashed = 0;
        let mut errs = Vec::new();
        for c in changes {
            if c.unstaged == Some(St::Untracked) {
                match trash::delete(self.root.join(&c.path)) {
                    Ok(()) => trashed += 1,
                    Err(e) => errs.push(format!("{}: {e}", c.path)),
                }
            } else {
                tracked.push(c.path);
            }
        }
        if !errs.is_empty() {
            self.log = Some((true, format!("Nicht alles verworfen:\n{}", errs.join("\n"))));
        } else if trashed > 0 {
            self.log = Some((false, format!("{trashed} nicht verfolgte Datei(en) in den Papierkorb gelegt")));
        }
        if tracked.is_empty() {
            self.refresh(ctx);
        } else {
            self.exec("Änderungen verwerfen", vec![Self::args(&["restore", "--worktree"], &tracked)], ctx);
        }
    }

    /// Ordner ohne Repository: neues Repository erstellen oder eines hierher klonen.
    fn ui_no_repo(&mut self, ui: &mut egui::Ui, th: &Theme, ctx: &egui::Context) {
        let busy = self.busy.is_some();
        let name = self.root.file_name().map_or_else(|| self.root.display().to_string(), |n| n.to_string_lossy().into_owned());
        egui::ScrollArea::vertical().id_salt("git_init").auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::none().inner_margin(egui::Margin::symmetric(28.0, 22.0)).show(ui, |ui| {
                ui.set_max_width(620.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), egui::Sense::hover());
                    ui::draw(ui.painter(), r, Ic::Git, th.accent);
                    ui.label(egui::RichText::new(format!("„{name}“ ist noch kein Git-Repository")).size(18.0).strong());
                });
                ui.label(egui::RichText::new(self.root.display().to_string()).size(12.5).color(th.sub));
                ui.add_space(14.0);
                card(ui, th, |ui| {
                    ui.label(egui::RichText::new("Neues Repository erstellen").size(15.0).strong());
                    ui.label(egui::RichText::new("Legt die Versionsverwaltung in diesem Ordner an (Branch „main“).").size(12.5).color(th.sub));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(".gitignore:");
                        egui::ComboBox::from_id_salt("ignore_tpl").selected_text(git::IGNORE_TEMPLATES[self.ignore_tpl].0).show_ui(ui, |ui| {
                            for (i, (n, _)) in git::IGNORE_TEMPLATES.iter().enumerate() {
                                ui.selectable_value(&mut self.ignore_tpl, i, *n);
                            }
                        });
                        if self.root.join(".gitignore").exists() {
                            ui.label(egui::RichText::new("(vorhandene .gitignore bleibt unverändert)").size(12.0).color(th.sub));
                        }
                    });
                    ui.checkbox(&mut self.first_commit, "Ersten Commit mit allen Dateien des Ordners erstellen");
                    ui.add_space(4.0);
                    if ui.add_enabled(!busy, egui::Button::new(egui::RichText::new("Repository erstellen").strong())).clicked() {
                        let tpl = git::IGNORE_TEMPLATES[self.ignore_tpl].1;
                        let gi = self.root.join(".gitignore");
                        if !tpl.is_empty() && !gi.exists() {
                            if let Err(e) = std::fs::write(&gi, tpl) {
                                self.log = Some((true, format!(".gitignore konnte nicht geschrieben werden: {e}")));
                            }
                        }
                        let mut cmds = vec![vec!["init".into(), "-b".into(), "main".into()]];
                        if self.first_commit {
                            cmds.push(vec!["add".into(), "-A".into()]);
                            cmds.push(vec!["commit".into(), "-m".into(), "Erster Commit".into()]);
                        }
                        self.exec("Repository erstellen", cmds, ctx);
                    }
                });
                ui.add_space(10.0);
                card(ui, th, |ui| {
                    ui.label(egui::RichText::new("Repository klonen").size(15.0).strong());
                    ui.label(egui::RichText::new("Lädt ein vorhandenes Repository in einen neuen Unterordner von diesem Ordner.").size(12.5).color(th.sub));
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(&mut self.clone_url).hint_text("https://github.com/name/projekt.git").desired_width(f32::INFINITY));
                    let target = clone_target(&self.clone_url);
                    if let Some(t) = &target {
                        let dest = self.root.join(t);
                        let exists = dest.exists();
                        ui.label(
                            egui::RichText::new(format!("Ziel: {}{}", dest.display(), if exists { " – existiert bereits" } else { "" }))
                                .size(12.0)
                                .color(if exists { th.danger } else { th.sub }),
                        );
                    }
                    let can = !busy && target.as_ref().is_some_and(|t| !self.root.join(t).exists());
                    if ui.add_enabled(can, egui::Button::new(egui::RichText::new("Klonen").strong())).clicked() {
                        let t = target.unwrap();
                        self.after_root = Some(self.root.join(&t));
                        let root = self.root.clone();
                        self.exec_in(root, "Klonen", vec![vec!["clone".into(), self.clone_url.trim().to_string(), t]], ctx);
                    }
                });
                if let Some((label, _)) = &self.busy {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("{label} …"));
                    });
                }
                if let Some((err, t)) = self.log.clone() {
                    banner(ui, th, err, &t);
                }
                if !git::git_available() {
                    banner(ui, th, true, "Git ist nicht installiert – bitte von git-scm.com installieren.");
                }
            });
        });
    }

    /// `ais`: verfügbare KI-Anbieter für Commit-Nachrichten (leer = KI aus).
    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme, ais: &[CommitAi]) -> Option<GitOut> {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        let mut out = None;
        let busy = self.busy.is_some();
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, th.bg);
        if self.no_repo {
            self.ui_no_repo(ui, th, &ctx);
            return out;
        }

        // Kopfzeile
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(8.0);
            ui::draw(ui.painter(), Rect::from_center_size(ui.cursor().min + vec2(10.0, 16.0), vec2(18.0, 18.0)), Ic::Git, th.accent);
            ui.add_space(26.0);
            let name = self.root.file_name().map_or_else(|| self.root.display().to_string(), |n| n.to_string_lossy().into_owned());
            ui.label(egui::RichText::new(name).size(15.0).strong());
            ui.add_space(8.0);
            let branch = self.status.as_ref().map(|s| s.branch.clone()).unwrap_or_default();
            let r = ui::cmd_btn(ui, th, Some(Ic::Branch), if branch.is_empty() { "Branch" } else { &branch }, !busy, true);
            let pid = ui.id().with("branches");
            if r.clicked() {
                ui.memory_mut(|m| m.toggle_popup(pid));
            }
            egui::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                ui.set_min_width(280.0);
                ui::menu_label(ui, "Wechseln zu");
                for (b, cur) in self.branches.clone() {
                    ui.horizontal(|ui| {
                        if ui::menu_entry(ui, None, &b, "", !cur, cur) {
                            self.confirm_checkout(b.clone(), &ctx);
                            ui.memory_mut(|m| m.close_popup());
                        }
                    });
                }
                ui::menu_sep(ui);
                ui::menu_label(ui, "Neuer Branch (vom aktuellen Stand)");
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let te = ui.add(egui::TextEdit::singleline(&mut self.new_branch).hint_text("Name").desired_width(170.0));
                    let ok = valid_branch(&self.new_branch);
                    if ui.add_enabled(ok, egui::Button::new("Erstellen")).clicked() || (ok && te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        let n = self.new_branch.trim().to_string();
                        self.exec("Branch erstellen", vec![vec!["switch".into(), "-c".into(), n]], &ctx);
                        self.new_branch.clear();
                        ui.memory_mut(|m| m.close_popup());
                    }
                });
                let others: Vec<String> = self.branches.iter().filter(|b| !b.1).map(|b| b.0.clone()).collect();
                if !others.is_empty() {
                    ui::menu_sep(ui);
                    ui::menu_label(ui, "Branch löschen");
                    for b in others {
                        if ui::menu_entry(ui, Some(Ic::Trash), &b, "", true, false) {
                            self.confirm = Some(Confirm::DeleteBranch(b));
                            ui.memory_mut(|m| m.close_popup());
                        }
                    }
                }
            });
            if let Some(s) = &self.status {
                let (sync, col) = match (&s.upstream, s.ahead, s.behind) {
                    (None, _, _) if self.remotes.is_empty() => ("kein Remote".to_string(), th.sub),
                    (None, _, _) => ("nicht veröffentlicht".to_string(), th.sub),
                    (Some(_), 0, 0) => ("aktuell".to_string(), status_color(St::Added, th)),
                    (Some(_), a, b) => (format!("↑{a}  ↓{b}"), th.accent),
                };
                badge(ui, &sync, col);
            }
            ui.add_space(10.0);
            // Umschalter Änderungen / Verlauf
            let n = self.status.as_ref().map_or(0, |s| s.changes.len());
            for (i, label) in [(0u8, format!("Änderungen ({n})")), (1, "Verlauf".to_string())] {
                let r = ui::cmd_btn(ui, th, Some(if i == 0 { Ic::Docs } else { Ic::Clock }), &label, true, false);
                if self.tab == i {
                    ui.painter().rect_filled(Rect::from_min_size(r.rect.left_bottom() - vec2(-10.0, 2.0), vec2(r.rect.width() - 20.0, 2.0)), 1.0, th.accent);
                }
                if r.clicked() && self.tab != i {
                    self.tab = i;
                    if i == 1 {
                        self.load_log(&ctx);
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if ui::tool_btn(ui, th, Ic::Refresh, "Status aktualisieren", !busy, false).clicked() {
                    self.refresh(&ctx);
                }
                // Weitere Aktionen
                let r = ui::cmd_btn(ui, th, None, "Mehr", !busy, true);
                let pid = ui.id().with("git_more");
                if r.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(pid));
                }
                egui::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                    ui.set_min_width(300.0);
                    ui::menu_label(ui, "Zwischenspeichern (Stash)");
                    if ui::menu_entry(ui, Some(Ic::Download), "Änderungen beiseitelegen", "", self.status.as_ref().is_some_and(|s| !s.changes.is_empty()), false) {
                        self.exec("Stash", vec![vec!["stash".into(), "push".into(), "-u".into(), "-m".into(), "Foldex".into()]], &ctx);
                        ui.memory_mut(|m| m.close_popup());
                    }
                    if ui::menu_entry(ui, Some(Ic::Push), "Beiseitegelegte Änderungen zurückholen", "", true, false) {
                        self.exec("Stash zurückholen", vec![vec!["stash".into(), "pop".into()]], &ctx);
                        ui.memory_mut(|m| m.close_popup());
                    }
                    ui::menu_sep(ui);
                    ui::menu_label(ui, if self.remotes.is_empty() { "Remote hinzufügen (origin)" } else { "Remote-Adresse ändern (origin)" });
                    for (n, u) in &self.remotes {
                        ui::menu_label(ui, &format!("{n}: {u}"));
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.add(egui::TextEdit::singleline(&mut self.remote_url).hint_text("https://github.com/name/projekt.git").desired_width(220.0));
                        let ok = !self.remote_url.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("Speichern")).clicked() {
                            let url = self.remote_url.trim().to_string();
                            let has = self.remotes.iter().any(|r| r.0 == "origin");
                            let cmd = if has {
                                vec!["remote".into(), "set-url".into(), "origin".into(), url]
                            } else {
                                vec!["remote".into(), "add".into(), "origin".into(), url]
                            };
                            self.exec("Remote speichern", vec![cmd], &ctx);
                            self.remote_url.clear();
                            ui.memory_mut(|m| m.close_popup());
                        }
                    });
                    ui::menu_sep(ui);
                    if ui::menu_entry(ui, Some(Ic::Docs), ".gitignore bearbeiten", "", true, false) {
                        let gi = self.root.join(".gitignore");
                        if !gi.exists() {
                            let _ = std::fs::write(&gi, git::IGNORE_TEMPLATES[1].1);
                        }
                        out = Some(GitOut::OpenFile(gi));
                        ui.memory_mut(|m| m.close_popup());
                    }
                });
                if ui::cmd_btn(ui, th, Some(Ic::Push), "Push", !busy, false).on_hover_text("Commits zum Server senden").clicked() {
                    self.push(&ctx);
                }
                if ui::cmd_btn(ui, th, Some(Ic::Download), "Pull", !busy, false).on_hover_text("Änderungen vom Server holen und zusammenführen").clicked() {
                    self.pull(&ctx);
                }
                if ui::cmd_btn(ui, th, Some(Ic::Refresh), "Fetch", !busy, false).clicked() {
                    self.fetch(&ctx);
                }
                if ui::cmd_btn(ui, th, Some(Ic::Open), "Ordner", true, false).on_hover_text("Repository im Explorer öffnen").clicked() {
                    out = Some(GitOut::Reveal(self.root.clone()));
                }
                if let Some((label, _)) = &self.busy {
                    ui.label(egui::RichText::new(format!("{label} …")).size(12.5).color(th.sub));
                    ui.spinner();
                }
            });
        });
        ui.add_space(4.0);
        let sep = ui.cursor().min.y;
        ui.painter().line_segment([pos2(rect.left(), sep), pos2(rect.right(), sep)], egui::Stroke::new(1.0, th.line));

        // Hinweise: Fehler, Merge, letzte Meldung
        if let Some(e) = &self.err {
            banner(ui, th, true, e);
        }
        let merging = self.status.as_ref().is_some_and(|s| s.merging);
        let conflicts = self.status.as_ref().map_or(0, |s| s.conflicts());
        if merging || conflicts > 0 {
            let t = if conflicts > 0 {
                format!("Merge-Konflikt in {conflicts} Datei(en): Dateien bearbeiten, Konfliktmarker (<<<<<<< ======= >>>>>>>) auflösen, stagen und committen.")
            } else {
                "Merge läuft – alle Konflikte gelöst, jetzt committen.".to_string()
            };
            ui.horizontal(|ui| {
                banner(ui, th, conflicts > 0, &t);
                if merging && ui.button("Merge abbrechen …").clicked() {
                    self.confirm = Some(Confirm::AbortMerge);
                }
            });
        }
        if let Some((err, t)) = self.log.clone() {
            ui.horizontal(|ui| {
                banner(ui, th, err, &t);
                if ui::tool_btn(ui, th, Ic::Close, "Meldung schließen", true, false).clicked() {
                    self.log = None;
                }
            });
        }

        // Inhalt: links Änderungen + Commit, rechts Diff
        let body = ui.available_rect_before_wrap();
        let left_w = (body.width() * 0.38).clamp(260.0, 460.0);
        let left = Rect::from_min_max(body.min, pos2(body.left() + left_w, body.bottom()));
        let right = Rect::from_min_max(pos2(left.right() + 1.0, body.top()), body.max);
        ui.painter().line_segment([pos2(left.right(), body.top()), pos2(left.right(), body.bottom())], egui::Stroke::new(1.0, th.line));
        let Some(s) = self.status.clone() else {
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body.shrink(16.0)), |ui| {
                if self.err.is_none() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Status wird geladen …");
                    });
                }
            });
            return out;
        };
        if self.tab == 1 {
            self.ui_history(ui, th, left, right, &ctx);
            self.confirm_window(&ctx, th, &s);
            return out;
        }
        let staged: Vec<Change> = s.changes.iter().filter(|c| c.staged.is_some()).cloned().collect();
        let unstaged: Vec<Change> = s.changes.iter().filter(|c| c.unstaged.is_some()).cloned().collect();
        let (any_staged, set) = git::commit_set(&s.changes);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(left.shrink2(vec2(6.0, 6.0))), |ui| {
            let commit_h = 222.0;
            let list_h = (ui.available_height() - commit_h).max(80.0);
            egui::ScrollArea::vertical().id_salt("git_changes").max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.section(ui, th, &format!("Gestagte Änderungen ({})", staged.len()), true, &staged, busy, &ctx, &mut out);
                ui.add_space(6.0);
                self.section(ui, th, &format!("Änderungen ({})", unstaged.len()), false, &unstaged, busy, &ctx, &mut out);
                if s.changes.is_empty() {
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("Keine Änderungen – der Arbeitsbereich ist sauber.").color(th.sub));
                }
            });
            ui.add_space(6.0);
            // Übersicht: was der Commit enthält
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (m, a, d, r) = git::counts(&set);
                ui.label(egui::RichText::new(if any_staged { "Commit enthält:" } else { "Nichts gestaged – Commit enthält alles:" }).size(12.0).color(th.sub));
                for (n, st) in [(m, St::Modified), (a, St::Added), (d, St::Deleted), (r, St::Renamed)] {
                    if n > 0 {
                        badge(ui, &format!("{n} {}", st.label()), status_color(st, th));
                    }
                }
                if set.is_empty() {
                    ui.label(egui::RichText::new("keine Änderungen").size(12.0).color(th.sub));
                }
            });
            // lange Nachrichten scrollen im Feld (senkrecht und für lange Zeilen waagerecht), statt das Layout zu sprengen
            egui::Frame::none().stroke(egui::Stroke::new(1.0, th.line)).rounding(Rounding::same(6.0)).fill(th.field).show(ui, |ui| {
                egui::ScrollArea::both().id_salt("commit_msg").max_height(84.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.msg)
                            .hint_text("Commit-Nachricht (erste Zeile = Zusammenfassung)")
                            .frame(false)
                            .desired_rows(3)
                            .desired_width(ui.available_width().max(240.0)),
                    );
                });
            });
            if let Some(first) = self.msg.lines().next() {
                let n = first.chars().count();
                if n > 72 {
                    ui.label(egui::RichText::new(format!("Erste Zeile hat {n} Zeichen – üblich sind höchstens 72")).size(11.5).color(th.sub));
                }
            }
            // Nachricht: per KI (Anbieter wählbar) oder ohne KI aus der Dateiliste
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let generating = self.ai_rx.is_some();
                if ais.is_empty() {
                    ui.label(egui::RichText::new("KI: kein Modell/Agent verfügbar").size(12.0).color(th.sub)).on_hover_text("Einstellungen → KI: ein Modell anlegen oder einen Agenten installieren");
                } else {
                    let want = LAST_AI.lock().map(|g| g.clone()).unwrap_or_default();
                    let pick = ais.iter().find(|a| a.id() == want).unwrap_or(&ais[0]).clone();
                    let r = ui::cmd_btn(ui, th, Some(Ic::Spark), if generating { "KI schreibt …" } else { "KI-Nachricht" }, !set.is_empty() && !generating && !busy, false)
                        .on_hover_text(format!("{} fasst die Änderungen als Commit-Nachricht zusammen", pick.name()));
                    if r.clicked() {
                        self.gen_message(pick.clone(), &s, &ctx);
                    }
                    if generating {
                        ui.spinner();
                    }
                    let r = ui::cmd_btn(ui, th, None, pick.name(), !generating, true).on_hover_text("KI-Anbieter für die Commit-Nachricht");
                    let pid = ui.id().with("commit_ai");
                    if r.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(pid));
                    }
                    egui::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        ui.set_min_width(240.0);
                        ui::menu_label(ui, "KI für Commit-Nachrichten");
                        for a in ais {
                            let hint = if matches!(a, CommitAi::Agent { .. }) { "Agent" } else { "eigenes Modell" };
                            if ui::menu_entry(ui, Some(Ic::Spark), a.name(), hint, true, a.id() == pick.id()) {
                                if let Ok(mut g) = LAST_AI.lock() {
                                    *g = a.id();
                                }
                            }
                        }
                    });
                }
                if ui::cmd_btn(ui, th, Some(Ic::Docs), "Ohne KI", !set.is_empty(), false).on_hover_text("Nachricht aus der Liste der geänderten Dateien erzeugen").clicked() {
                    self.msg = git::plain_message(&set);
                }
            });
            ui.horizontal(|ui| {
                let n = set.len();
                let can = !busy && n > 0 && !self.msg.trim().is_empty() && conflicts == 0;
                let label = if any_staged { format!("Commit ({})", staged.len()) } else { format!("Alles committen ({n})") };
                let r = ui.add_enabled(can, egui::Button::new(egui::RichText::new(label).strong()));
                let tip = if n == 0 {
                    "Keine Änderungen"
                } else if self.msg.trim().is_empty() {
                    "Commit-Nachricht eingeben (oder per KI / ohne KI erzeugen)"
                } else if conflicts > 0 {
                    "Erst alle Konflikte lösen"
                } else if any_staged {
                    "Gestagte Änderungen committen"
                } else {
                    "Alle Änderungen stagen und committen"
                };
                if r.on_hover_text(tip).clicked() || (can && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))) {
                    let m = self.msg.trim().to_string();
                    let mut cmds = Vec::new();
                    if !any_staged {
                        cmds.push(vec!["add".into(), "-A".into()]);
                    }
                    cmds.push(vec!["commit".into(), "-m".into(), m]);
                    self.exec("Commit", cmds, &ctx);
                    self.msg.clear();
                }
                ui.label(egui::RichText::new("Strg+Enter").size(12.0).color(th.sub));
            });
        });
        let mut out_back = false;
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(right.shrink2(vec2(8.0, 6.0))), |ui| {
            match &self.sel.clone() {
                None if set.is_empty() => {
                    ui.add_space(20.0);
                    ui.label(egui::RichText::new("Keine Änderungen.").color(th.sub));
                }
                None => {
                    // Übersicht: Dateien nach Art, darunter auf Wunsch der gesamte Diff
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Übersicht des Commits").strong());
                        ui.label(egui::RichText::new(if any_staged { "gestagte Änderungen" } else { "alle Änderungen" }).size(12.0).color(th.sub));
                        let r = ui.add_enabled(self.all_rx.is_none(), egui::Button::new(if self.all_diff.is_empty() { "Gesamten Diff anzeigen" } else { "Diff neu laden" }).small());
                        if r.clicked() {
                            self.load_all_diff(&s, &ctx);
                        }
                        if self.all_rx.is_some() {
                            ui.spinner();
                        }
                    });
                    let list_h = if self.all_diff.is_empty() { ui.available_height() } else { (ui.available_height() * 0.35).max(90.0) };
                    egui::ScrollArea::vertical().id_salt("git_overview").max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
                        for st in [St::Added, St::Modified, St::Renamed, St::Deleted, St::Conflict] {
                            let group: Vec<&(St, String, Option<String>)> = set.iter().filter(|c| c.0 == st).collect();
                            if group.is_empty() {
                                continue;
                            }
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("{} ({})", st.label(), group.len())).size(12.5).strong().color(status_color(st, th)));
                            for (_, p, o) in group {
                                let text = match o {
                                    Some(o) => format!("{o}  →  {p}"),
                                    None => p.clone(),
                                };
                                let r = ui.add(egui::Label::new(egui::RichText::new(text).size(12.5).monospace()).sense(Sense::click()).truncate());
                                if r.on_hover_text("Klicken für den Diff dieser Datei").clicked() {
                                    let staged = any_staged;
                                    self.sel = Some((p.clone(), staged));
                                    self.diff.clear();
                                    self.load_diff(&s, p, staged, &ctx);
                                }
                            }
                        }
                    });
                    if !self.all_diff.is_empty() {
                        ui.add_space(6.0);
                        diff_view(ui, th, &self.all_diff);
                    }
                }
                Some((p, staged)) => {
                    ui.horizontal(|ui| {
                        if ui::tool_btn(ui, th, Ic::Back, "Zur Übersicht", true, false).clicked() {
                            out_back = true;
                        }
                        ui.label(egui::RichText::new(p).strong());
                        ui.label(egui::RichText::new(if *staged { "gestaged" } else { "Arbeitsbereich" }).size(12.0).color(th.sub));
                        let full = self.root.join(p);
                        if full.is_file() && ui.small_button("Öffnen").clicked() {
                            out = Some(GitOut::OpenFile(full));
                        }
                    });
                    if self.diff_rx.is_some() {
                        ui.spinner();
                    }
                    diff_view(ui, th, &self.diff);
                }
            }
        });
        if out_back {
            self.sel = None;
            self.diff.clear();
        }
        self.confirm_window(&ctx, th, &s);
        out
    }

    /// Verlauf: Commits links, Inhalt des gewählten Commits rechts.
    fn ui_history(&mut self, ui: &mut egui::Ui, th: &Theme, left: Rect, right: Rect, ctx: &egui::Context) {
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(left.shrink2(vec2(6.0, 6.0))), |ui| {
            if self.commits_rx.is_some() && self.commits.is_empty() {
                ui.spinner();
            } else if self.commits.is_empty() {
                ui.add_space(12.0);
                ui.label(egui::RichText::new("Noch keine Commits.").color(th.sub));
            }
            let row_h = 44.0;
            let commits = self.commits.clone();
            egui::ScrollArea::vertical().id_salt("git_log").auto_shrink([false, false]).show_rows(ui, row_h, commits.len(), |ui, range| {
                for c in &commits[range] {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), egui::Sense::click());
                    let sel = self.commit_sel.as_deref() == Some(c.hash.as_str());
                    if sel {
                        ui.painter().rect_filled(r, Rounding::same(6.0), th.sel);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, Rounding::same(6.0), th.hover);
                    }
                    // Zeitstrahl
                    let x = r.left() + 12.0;
                    ui.painter().line_segment([pos2(x, r.top()), pos2(x, r.bottom())], egui::Stroke::new(1.5, th.line));
                    ui.painter().circle(pos2(x, r.center().y), 4.0, th.bg, egui::Stroke::new(2.0, th.accent));
                    ui::paint_text(ui, x + 14.0, r.top() + 14.0, &c.subject, 13.5, th.text, r.width() - 40.0, false);
                    ui::paint_text(ui, x + 14.0, r.bottom() - 13.0, &format!("{}  ·  {}  ·  {}", c.short, c.author, c.date), 11.5, th.sub, r.width() - 40.0, false);
                    if resp.clicked() {
                        self.commit_sel = Some(c.hash.clone());
                        self.commit_diff.clear();
                        self.load_commit(c.hash.clone(), ctx);
                    }
                }
            });
        });
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(right.shrink2(vec2(8.0, 6.0))), |ui| {
            if self.commit_sel.is_none() {
                ui.add_space(20.0);
                ui.label(egui::RichText::new("Commit links auswählen, um seine Änderungen zu sehen.").color(th.sub));
            } else {
                if self.commit_rx.is_some() {
                    ui.spinner();
                }
                diff_view(ui, th, &self.commit_diff);
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn section(&mut self, ui: &mut egui::Ui, th: &Theme, title: &str, staged: bool, list: &[Change], busy: bool, ctx: &egui::Context, out: &mut Option<GitOut>) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(title).size(12.5).strong().color(th.sub));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if list.is_empty() {
                    return;
                }
                let paths: Vec<String> = list.iter().map(|c| c.path.clone()).collect();
                if staged {
                    if ui.add_enabled(!busy, egui::Button::new("Alle unstagen").small()).clicked() {
                        self.exec("Unstage", vec![Self::args(&["restore", "--staged"], &paths)], ctx);
                    }
                } else {
                    if ui.add_enabled(!busy, egui::Button::new("Alle stagen").small()).clicked() {
                        self.exec("Stage", vec![Self::args(&["add", "-A"], &paths)], ctx);
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Alle verwerfen …").small()).clicked() {
                        self.confirm = Some(Confirm::Discard(list.to_vec()));
                    }
                }
            });
        });
        let row_h = 26.0;
        for c in list {
            let st = if staged { c.staged } else { c.unstaged }.unwrap_or(St::Modified);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            let selected = self.sel.as_ref().is_some_and(|(p, s)| *p == c.path && *s == staged);
            if selected {
                ui.painter().rect_filled(rect, Rounding::same(5.0), th.sel);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, Rounding::same(5.0), th.hover);
            }
            let col = status_color(st, th);
            ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), egui::Align2::CENTER_CENTER, st.letter(), FontId::monospace(12.5), col);
            let (dir, file) = c.path.trim_end_matches('/').rsplit_once('/').map_or(("", c.path.as_str()), |(d, f)| (d, f));
            let btn_w = if resp.hovered() { 56.0 } else { 0.0 };
            let avail = rect.width() - 30.0 - btn_w;
            ui::paint_text(ui, rect.left() + 26.0, rect.center().y, file, 13.5, th.text, avail * 0.65, false);
            let fw = ui.fonts(|f| f.layout_no_wrap(file.to_string(), FontId::proportional(13.5), th.text).size().x).min(avail * 0.65);
            if !dir.is_empty() {
                ui::paint_text(ui, rect.left() + 32.0 + fw, rect.center().y, dir, 12.0, th.sub, avail - fw - 8.0, false);
            }
            let resp = resp.on_hover_text(format!("{} – {}", c.path, st.label()));
            if resp.hovered() && !busy {
                // Schnellknöpfe rechts
                let r1 = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(24.0, 22.0));
                let r2 = Rect::from_center_size(pos2(rect.right() - 40.0, rect.center().y), vec2(24.0, 22.0));
                let (ic1, tip1) = if staged { (Ic::Undo, "Unstagen") } else { (Ic::Plus, "Stagen") };
                if ui::tool_btn_at(ui, th, r1, resp.id.with("a"), ic1, tip1, false).clicked() {
                    if staged {
                        self.exec("Unstage", vec![Self::args(&["restore", "--staged"], &[c.path.clone()])], ctx);
                    } else {
                        self.exec("Stage", vec![Self::args(&["add", "-A"], &[c.path.clone()])], ctx);
                    }
                }
                if !staged && st != St::Conflict && ui::tool_btn_at(ui, th, r2, resp.id.with("b"), Ic::Trash, "Änderungen verwerfen …", false).clicked() {
                    self.confirm = Some(Confirm::Discard(vec![c.clone()]));
                }
            }
            if resp.clicked() {
                self.sel = Some((c.path.clone(), staged));
                self.diff.clear();
                if let Some(s) = self.status.clone() {
                    self.load_diff(&s, &c.path, staged, ctx);
                }
            }
            if resp.double_clicked() {
                let full = self.root.join(&c.path);
                if full.is_file() {
                    *out = Some(GitOut::OpenFile(full));
                }
            }
        }
    }

    fn confirm_checkout(&mut self, b: String, ctx: &egui::Context) {
        if self.status.as_ref().is_some_and(|s| s.dirty()) {
            self.confirm = Some(Confirm::Checkout(b));
        } else {
            self.exec("Branch wechseln", vec![vec!["switch".into(), b]], ctx);
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context, th: &Theme, s: &Status) {
        let Some(c) = &self.confirm else { return };
        let (title, text, action): (&str, String, &str) = match c {
            Confirm::Discard(v) => (
                "Änderungen verwerfen",
                format!(
                    "{} Datei(en) auf den letzten Commit zurücksetzen?\nNicht verfolgte Dateien kommen in den Papierkorb, geänderte Dateien verlieren ihre ungesicherten Änderungen endgültig.",
                    v.len()
                ),
                "Verwerfen",
            ),
            Confirm::Checkout(b) => (
                "Branch wechseln",
                format!(
                    "Es gibt ungesicherte Änderungen ({} Datei(en)). Git nimmt sie nach „{b}“ mit, sofern sie nicht mit dem Ziel kollidieren – sonst wird der Wechsel abgelehnt und nichts geht verloren.",
                    s.changes.len()
                ),
                "Wechseln",
            ),
            Confirm::DeleteBranch(b) => (
                "Branch löschen",
                format!("Branch „{b}“ löschen? Nicht zusammengeführte Branches lehnt Git ab – so gehen keine Commits verloren."),
                "Löschen",
            ),
            Confirm::AbortMerge => ("Merge abbrechen", "Merge abbrechen und den Stand vor dem Merge wiederherstellen? Konfliktlösungen gehen verloren.".into(), "Merge abbrechen"),
        };
        let mut decision = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.label(text);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(egui::RichText::new(action).color(th.danger))).clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Abbrechen").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => match self.confirm.take().unwrap() {
                Confirm::Discard(v) => self.discard(v, ctx),
                Confirm::Checkout(b) => self.exec("Branch wechseln", vec![vec!["switch".into(), b]], ctx),
                Confirm::DeleteBranch(b) => self.exec("Branch löschen", vec![vec!["branch".into(), "-d".into(), b]], ctx),
                Confirm::AbortMerge => self.exec("Merge abbrechen", vec![vec!["merge".into(), "--abort".into()]], ctx),
            },
            Some(false) => self.confirm = None,
            None => {}
        }
    }
}

/// Zielordner beim Klonen aus der URL („…/projekt.git“ → „projekt“).
fn clone_target(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches('/');
    if u.is_empty() || !(u.contains("://") || u.contains('@') || std::path::Path::new(u).is_dir()) {
        return None;
    }
    let last = u.rsplit(['/', ':', '\\']).next()?.trim_end_matches(".git");
    (!last.is_empty() && !last.contains(['<', '>', '"', '|', '?', '*'])).then(|| last.to_string())
}

fn card(ui: &mut egui::Ui, th: &Theme, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(th.panel)
        .stroke(egui::Stroke::new(1.0, th.line))
        .rounding(Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(16.0, 12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        });
}

fn badge(ui: &mut egui::Ui, text: &str, col: Color32) {
    egui::Frame::none().fill(col.gamma_multiply(0.14)).rounding(Rounding::same(9.0)).inner_margin(egui::Margin::symmetric(8.0, 2.0)).show(ui, |ui| {
        ui.label(egui::RichText::new(text).size(12.0).color(col));
    });
}

fn valid_branch(n: &str) -> bool {
    let n = n.trim();
    !n.is_empty()
        && !n.starts_with('-')
        && !n.contains("..")
        && !n.ends_with('/')
        && !n.ends_with(".lock")
        && n.chars().all(|c| !c.is_whitespace() && !c.is_control() && !"~^:?*[\\".contains(c))
}

pub fn status_color(st: St, th: &Theme) -> Color32 {
    let (d, l) = match st {
        St::Modified | St::Renamed => (0xe2c08d, 0x9a6700),
        St::Added | St::Untracked => (0x73c991, 0x1a7f37),
        St::Deleted => (0xf48771, 0xc42b1c),
        St::Conflict => (0xff7b72, 0xcf222e),
        St::Ignored => (0x8c8c8c, 0x6e7781),
    };
    let h = if th.dark { d } else { l };
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

fn banner(ui: &mut egui::Ui, th: &Theme, err: bool, text: &str) {
    egui::Frame::none()
        .fill(th.field)
        .stroke(egui::Stroke::new(1.0, if err { th.danger } else { th.line }))
        .rounding(Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
        .outer_margin(egui::Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width() - 60.0);
            ui.label(egui::RichText::new(text).size(12.5).color(if err { th.danger } else { th.text }));
        });
}

/// Diff mit farbigen Zeilen; nur sichtbare Zeilen werden gezeichnet (auch bei sehr großen Diffs flüssig).
fn diff_view(ui: &mut egui::Ui, th: &Theme, lines: &[String]) {
    let font = FontId::monospace(12.5);
    let row_h = ui.fonts(|f| f.row_height(&font)) + 2.0;
    let add = status_color(St::Added, th);
    let del = status_color(St::Deleted, th);
    egui::Frame::none().fill(th.ed_bg).rounding(Rounding::same(6.0)).stroke(egui::Stroke::new(1.0, th.line)).show(ui, |ui| {
        egui::ScrollArea::both().id_salt("git_diff").auto_shrink([false, false]).show_rows(ui, row_h, lines.len(), |ui, range| {
            for l in &lines[range] {
                let (col, bg) = if l.starts_with("+++") || l.starts_with("---") || l.starts_with("diff ") || l.starts_with("index ") {
                    (th.sub, Color32::TRANSPARENT)
                } else if l.starts_with('+') {
                    (add, add.gamma_multiply(0.12))
                } else if l.starts_with('-') {
                    (del, del.gamma_multiply(0.12))
                } else if l.starts_with("@@") {
                    (th.accent, Color32::TRANSPARENT)
                } else {
                    (th.ed_text, Color32::TRANSPARENT)
                };
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().max(800.0), row_h), Sense::hover());
                if bg != Color32::TRANSPARENT {
                    ui.painter().rect_filled(r, 0.0, bg);
                }
                ui.painter().text(pos2(r.left() + 6.0, r.center().y), egui::Align2::LEFT_CENTER, l.replace('\t', "    "), font.clone(), col);
            }
        });
    });
}
