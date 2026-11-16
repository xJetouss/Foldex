//! Integriertes Terminal als Tab: echte Pseudo-Konsole (ConPTY unter Windows) über `portable-pty`,
//! Escape-Sequenzen per `vt100`, Darstellung direkt mit egui. Das Lesen läuft in einem eigenen Thread;
//! die Oberfläche wird nur bei neuer Ausgabe neu gezeichnet.

use crate::complete::{self, Cand, Shell};
use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, FontId, Key, Rect, Sense};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, Mutex,
    },
};

const SCROLLBACK: usize = 5000;

pub struct Term {
    pub shell_name: String,
    /// Shell-Programm und Startordner – damit lässt sich das Terminal in einem anderen Fenster neu starten
    pub program: String,
    pub cwd: PathBuf,
    parser: Arc<Mutex<vt100::Parser>>,
    /// geteilt mit dem Lese-Thread, der Statusabfragen der Konsole beantwortet
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    exited: Arc<AtomicBool>,
    size: (u16, u16),
    scroll: usize,
    /// Auswahl in Zellen (Zeile, Spalte) Start/Ende
    sel: Option<((u16, u16), (u16, u16))>,
    /// eigene Vorschläge beim Tippen (Einstellung „Terminal-Vorschläge“)
    pub complete: bool,
    comp: Comp,
    /// Modell für KI-Vorschläge (`None` = aus); wird vom Explorer jedes Bild gesetzt
    pub ai: Option<crate::ai::Cfg>,
}

/// Zustand der Vorschläge: gelesene Eingabe, Kandidaten, Auswahl.
#[derive(Default)]
struct Comp {
    input: String,
    cands: Vec<Cand>,
    sel: usize,
    /// mit Esc ausgeblendet für genau diese Eingabe
    dismissed: Option<String>,
    /// Rest der Zeile hinter dem Cursor ist leer (dann grauer Resttext möglich)
    at_end: bool,
    /// Zeitpunkt der letzten Änderung der Eingabe (für „# …“-Anfragen an die KI)
    changed: Option<std::time::Instant>,
    /// KI-Anfrage läuft für diese Eingabe
    ai_for: String,
    ai_rx: Option<std::sync::mpsc::Receiver<(String, Result<String, String>)>>,
    ai_err: Option<String>,
}

impl Term {
    pub fn spawn(shell_name: &str, program: &str, cwd: &Path, ctx: &egui::Context) -> Result<Term, String> {
        Term::spawn_args(shell_name, program, &[], cwd, ctx)
    }

    /// Wie `spawn`, mit Argumenten (z. B. ein Installationsbefehl, dessen Ausgabe sichtbar sein soll).
    pub fn spawn_args(shell_name: &str, program: &str, args: &[String], cwd: &Path, ctx: &egui::Context) -> Result<Term, String> {
        complete::warm_up();
        let size = (24u16, 100u16);
        let pair = native_pty_system()
            .openpty(PtySize { rows: size.0, cols: size.1, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("Pseudo-Konsole konnte nicht erstellt werden: {e}"))?;
        let mut cmd = CommandBuilder::new(program);
        for a in args {
            cmd.arg(a);
        }
        let dir = if cwd.is_dir() { cwd.to_path_buf() } else { dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")) };
        cmd.cwd(&dir);
        cmd.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(cmd).map_err(|e| format!("„{program}“ konnte nicht gestartet werden: {e}"))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| e.to_string())?));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(size.0, size.1, SCROLLBACK)));
        let exited = Arc::new(AtomicBool::new(false));
        let (p, ex, w, ctx) = (parser.clone(), exited.clone(), writer.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("terminal".into())
            .spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let chunk = &buf[..n];
                            let mut reply = None;
                            if let Ok(mut p) = p.lock() {
                                p.process(chunk);
                                // ConPTY fragt beim Start die Cursorposition ab (ESC[6n) und wartet auf die Antwort
                                if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                                    let (r, c) = p.screen().cursor_position();
                                    reply = Some(format!("\x1b[{};{}R", r + 1, c + 1));
                                }
                            }
                            if let (Some(r), Ok(mut w)) = (reply, w.lock()) {
                                let _ = w.write_all(r.as_bytes());
                                let _ = w.flush();
                            }
                            ctx.request_repaint();
                        }
                    }
                }
                ex.store(true, Relaxed);
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        Ok(Term {
            shell_name: shell_name.to_string(),
            program: program.to_string(),
            cwd: dir,
            parser,
            writer,
            master: pair.master,
            child,
            exited,
            size,
            scroll: 0,
            sel: None,
            complete: true,
            comp: Comp::default(),
            ai: None,
        })
    }

    /// Prozess beendet? (Die Pseudo-Konsole meldet das Ende nicht immer über den Lese-Kanal.)
    pub fn exited(&mut self) -> bool {
        if !self.exited.load(Relaxed) && matches!(self.child.try_wait(), Ok(Some(_))) {
            self.exited.store(true, Relaxed);
        }
        self.exited.load(Relaxed)
    }

    /// Titel, den die Shell gesetzt hat (z. B. PowerShell: aktueller Pfad), sonst Shell-Name.
    pub fn title(&self) -> String {
        let t = self.parser.lock().map(|p| p.screen().title().to_string()).unwrap_or_default();
        if t.trim().is_empty() {
            self.shell_name.clone()
        } else {
            t
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
        if self.scroll != 0 {
            self.scroll = 0;
            if let Ok(mut p) = self.parser.lock() {
                p.set_scrollback(0);
            }
        }
    }

    /// Eine Zeile so eingeben, als hätte man sie getippt, und mit Enter abschicken.
    pub fn type_line(&mut self, line: &str) {
        let mut b = line.replace('\n', "\r").into_bytes();
        b.push(b'\r');
        self.send(&b);
    }

    /// Sichtbarer Text (für die Explorer-KI).
    pub fn screen_text(&self) -> String {
        self.parser.lock().map(|p| p.screen().contents()).unwrap_or_default()
    }

    pub fn paste(&mut self, text: &str) {
        let bracketed = self.parser.lock().map(|p| p.screen().bracketed_paste()).unwrap_or(false);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            self.send(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.send(text.as_bytes());
        }
    }

    fn selected_text(&self) -> Option<String> {
        let ((r0, c0), (r1, c1)) = self.sel?;
        let ((r0, c0), (r1, c1)) = if (r0, c0) <= (r1, c1) { ((r0, c0), (r1, c1)) } else { ((r1, c1), (r0, c0)) };
        let p = self.parser.lock().ok()?;
        let t = p.screen().contents_between(r0, c0, r1, c1 + 1);
        (!t.is_empty()).then_some(t)
    }

    /// Zeichnet das Terminal in den verfügbaren Bereich und verarbeitet Eingaben.
    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        let full = ui.available_rect_before_wrap();
        let head = Rect::from_min_size(full.min, vec2(full.width(), 30.0));
        self.header(ui, th, head);
        let rect = Rect::from_min_max(pos2(full.left(), head.bottom()), full.max);
        let resp = ui.interact(rect, ui.id().with("term"), Sense::click_and_drag());
        if resp.clicked() || resp.drag_started() {
            resp.request_focus();
        }
        let font = FontId::monospace(13.5);
        let (cw, ch) = ui.fonts(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let pad = 10.0;
        let cols = (((rect.width() - 2.0 * pad) / cw).floor() as u16).max(10);
        let rows = (((rect.height() - 2.0 * pad) / ch).floor() as u16).max(3);
        if (rows, cols) != self.size {
            self.size = (rows, cols);
            let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
            if let Ok(mut p) = self.parser.lock() {
                p.set_size(rows, cols);
            }
        }
        let origin = rect.min + vec2(pad, pad);
        let cell_at = |p: egui::Pos2| -> (u16, u16) {
            let c = ((p.x - origin.x) / cw).floor().clamp(0.0, cols as f32 - 1.0) as u16;
            let r = ((p.y - origin.y) / ch).floor().clamp(0.0, rows as f32 - 1.0) as u16;
            (r, c)
        };
        // Auswahl mit der Maus
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                let c = cell_at(p);
                self.sel = Some((c, c));
            }
        } else if resp.dragged() {
            if let (Some(p), Some((a, _))) = (resp.interact_pointer_pos(), self.sel) {
                self.sel = Some((a, cell_at(p)));
            }
        } else if resp.clicked() {
            self.sel = None;
        }
        // Mausrad: im Verlauf blättern (vt100 erlaubt höchstens eine Bildschirmhöhe je Schritt)
        if resp.hovered() {
            let dy = ui.input(|i| i.raw_scroll_delta.y);
            if dy != 0.0 {
                let lines = (dy / ch).round() as isize;
                let max = rows as usize;
                self.scroll = (self.scroll as isize + lines).clamp(0, max as isize) as usize;
                if let Ok(mut p) = self.parser.lock() {
                    p.set_scrollback(self.scroll);
                    self.scroll = p.screen().scrollback();
                }
            }
        }
        if resp.has_focus() {
            self.read_input(cols);
            self.input(ui);
            // Tabulator, Pfeiltasten usw. nicht für die Fokus-Navigation von egui verwenden
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    resp.id,
                    egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
                )
            });
        }
        resp.context_menu(|ui| {
            let has_sel = self.sel.is_some();
            if crate::ui::menu_entry(ui, Some(crate::ui::Ic::Copy), "Kopieren", "Strg+Umschalt+C", has_sel, false) {
                if let Some(t) = self.selected_text() {
                    ui.ctx().copy_text(t);
                }
            }
            if crate::ui::menu_entry(ui, Some(crate::ui::Ic::Paste), "Einfügen", "Strg+Umschalt+V", true, false) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                resp.request_focus();
            }
        });

        // Zeichnen
        let p = ui.painter_at(rect);
        let bg = th.ed_bg;
        p.rect_filled(rect, 0.0, bg);
        let Ok(parser) = self.parser.lock() else { return };
        let screen = parser.screen();
        let sel = self.sel.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        for r in 0..rows {
            let y = origin.y + r as f32 * ch;
            // Jedes Zeichen exakt an seine Spalte setzen: ein durchgehender Textlauf würde durch gerundete
            // Zeichenbreiten und Ersatzschriften (Prompt-Symbole) gegenüber dem Cursor-Raster verrutschen.
            for c in 0..cols {
                let Some(cell) = screen.cell(r, c) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut fg = color(cell.fgcolor(), th, true);
                let mut cbg = color(cell.bgcolor(), th, false);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut cbg);
                    if cbg == Color32::TRANSPARENT {
                        cbg = th.ed_text;
                    }
                    if fg == Color32::TRANSPARENT {
                        fg = bg;
                    }
                }
                let in_sel = sel.is_some_and(|(a, b)| (r, c) >= a && (r, c) <= b);
                let x = origin.x + c as f32 * cw;
                let w = if cell.is_wide() { cw * 2.0 } else { cw };
                let cell_rect = Rect::from_min_size(pos2(x, y), vec2(w, ch));
                if in_sel {
                    p.rect_filled(cell_rect, 0.0, th.ed_sel);
                } else if cbg != Color32::TRANSPARENT {
                    p.rect_filled(cell_rect, 0.0, cbg);
                }
                let s = cell.contents();
                if s.is_empty() || s == " " {
                    continue;
                }
                let col = if cell.bold() && fg == th.ed_text { th.text } else { fg };
                // breite Zeichen (CJK, Emoji) mittig in ihre zwei Zellen
                if cell.is_wide() {
                    p.text(cell_rect.center(), egui::Align2::CENTER_CENTER, &s, font.clone(), col);
                } else {
                    p.text(pos2(x, y), egui::Align2::LEFT_TOP, &s, font.clone(), col);
                }
                if cell.underline() {
                    p.line_segment([pos2(x, y + ch - 1.5), pos2(x + w, y + ch - 1.5)], egui::Stroke::new(1.0, col));
                }
            }
        }
        // Cursor: blinkender Balken bei Fokus, sonst Umriss
        if !screen.hide_cursor() && self.scroll == 0 {
            let (cr, cc) = screen.cursor_position();
            let r = Rect::from_min_size(pos2(origin.x + cc as f32 * cw, origin.y + cr as f32 * ch), vec2(cw, ch));
            if resp.has_focus() {
                let t = ui.input(|i| i.time);
                if (t * 1.6).fract() < 0.62 {
                    p.rect_filled(Rect::from_min_size(r.min, vec2(2.0, ch)), 1.0, th.cursor);
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(320));
            } else {
                p.rect_stroke(r.shrink(0.5), 2.0, egui::Stroke::new(1.0, th.cursor.gamma_multiply(0.6)));
            }
        }
        // Vorschläge: grauer Rest hinter dem Cursor und Liste darunter (bzw. darüber)
        let ai_busy = self.comp.ai_rx.is_some();
        if resp.has_focus() && (!self.comp.cands.is_empty() || ai_busy || self.comp.ai_err.is_some()) && self.scroll == 0 {
            let (cr, cc) = screen.cursor_position();
            let cur = Rect::from_min_size(pos2(origin.x + cc as f32 * cw, origin.y + cr as f32 * ch), vec2(cw, ch));
            let sel = self.comp.sel.min(self.comp.cands.len().saturating_sub(1));
            if self.comp.at_end && !self.comp.cands.is_empty() {
                if let Some(g) = self.comp.cands[sel].ghost(&self.comp.input) {
                    p.text(pos2(cur.left() + cw, cur.top()), egui::Align2::LEFT_TOP, g, font.clone(), th.sub.gamma_multiply(0.8));
                }
            }
            let small = FontId::proportional(11.5);
            let row_h = 22.0;
            let w = self
                .comp
                .cands
                .iter()
                .map(|c| ui.fonts(|f| f.layout_no_wrap(c.label.clone(), font.clone(), th.text).size().x))
                .fold(160.0f32, f32::max)
                .min(rect.width() - 40.0)
                + 96.0;
            let extra = usize::from(ai_busy || self.comp.ai_err.is_some());
            let h = row_h * (self.comp.cands.len() + extra) as f32 + 26.0;
            let below = cur.bottom() + h + 4.0 < rect.bottom();
            let x = cur.left().min(rect.right() - w - 8.0).max(rect.left() + 4.0);
            let top = if below { cur.bottom() + 4.0 } else { cur.top() - h - 4.0 };
            let pr = Rect::from_min_size(pos2(x, top), vec2(w, h));
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.id().with("term_comp")));
            painter.rect(pr.expand(1.0), 7.0, th.panel, egui::Stroke::new(1.0, th.line));
            for (i, c) in self.comp.cands.iter().enumerate() {
                let r = Rect::from_min_size(pos2(pr.left() + 4.0, pr.top() + 4.0 + i as f32 * row_h), vec2(w - 8.0, row_h));
                if i == sel {
                    painter.rect_filled(r, 5.0, th.sel);
                }
                let label: String = if c.label.chars().count() > 70 { format!("{} …", c.label.chars().take(68).collect::<String>()) } else { c.label.clone() };
                painter.text(pos2(r.left() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, label, font.clone(), th.text);
                let kc = if c.kind == complete::Kind::Ai { th.accent } else { th.sub };
                painter.text(pos2(r.right() - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, c.kind.label(), small.clone(), kc);
            }
            if extra > 0 {
                let r = Rect::from_min_size(pos2(pr.left() + 4.0, pr.top() + 4.0 + self.comp.cands.len() as f32 * row_h), vec2(w - 8.0, row_h));
                let (t, col) = match &self.comp.ai_err {
                    Some(e) if !ai_busy => (format!("✦ {e}"), th.danger),
                    _ => ("✦ KI denkt nach …".to_string(), th.accent),
                };
                painter.text(pos2(r.left() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, t, small.clone(), col);
            }
            painter.text(
                pos2(pr.left() + 12.0, pr.bottom() - 11.0),
                egui::Align2::LEFT_CENTER,
                if self.ai.is_some() { "Tab übernehmen · ↑↓ wählen · Esc ausblenden · Strg+Leertaste KI · # Frage an KI" } else { "Tab übernehmen · ↑↓ wählen · Esc ausblenden" },
                small,
                th.sub,
            );
        }
        if self.scroll > 0 {
            let t = format!("Verlauf −{} Zeilen", self.scroll);
            p.text(rect.right_top() + vec2(-12.0, 8.0), egui::Align2::RIGHT_TOP, t, FontId::proportional(12.0), th.sub);
        }
        drop(parser);
        if self.exited() {
            p.text(
                pos2(rect.center().x, rect.bottom() - 18.0),
                egui::Align2::CENTER_CENTER,
                "Prozess beendet – Tab schließen mit Strg+W",
                FontId::proportional(13.0),
                th.sub,
            );
        }
    }

    /// Eingabezeile vom Bildschirm lesen und Vorschläge neu bestimmen, wenn sie sich geändert hat.
    fn read_input(&mut self, cols: u16) {
        let shell = complete::shell_of(&self.program);
        let read = self.parser.lock().ok().and_then(|p| {
            let s = p.screen();
            if !self.complete || s.alternate_screen() || self.scroll != 0 {
                return None;
            }
            let (r, c) = s.cursor_position();
            let line = s.contents_between(r, 0, r, c);
            let rest = s.contents_between(r, c, r, cols);
            let (dir, input) = complete::split_prompt(&line, shell)?;
            Some((dir, input, rest.trim().is_empty()))
        });
        let Some((dir, input, at_end)) = read else {
            self.comp = Comp::default();
            return;
        };
        self.comp.at_end = at_end;
        let cwd = dir.clone().filter(|d| d.is_dir()).unwrap_or_else(|| self.cwd.clone());
        // KI-Antwort übernehmen, wenn die Eingabe noch dieselbe ist
        if let Some(rx) = &self.comp.ai_rx {
            if let Ok((for_input, r)) = rx.try_recv() {
                self.comp.ai_rx = None;
                match r {
                    Ok(cmd) if for_input == input && !cmd.is_empty() && cmd != input => {
                        let n = input.chars().count();
                        self.comp.cands.retain(|c| c.kind != complete::Kind::Ai);
                        self.comp.cands.insert(0, Cand { label: cmd.clone(), kind: complete::Kind::Ai, replace: n, insert: cmd });
                        self.comp.sel = 0;
                        self.comp.dismissed = None;
                    }
                    Ok(_) => {}
                    Err(e) => self.comp.ai_err = Some(e),
                }
            }
        }
        // „# was ich tun will“: nach kurzer Pause die KI nach dem passenden Befehl fragen
        if input == self.comp.input {
            let calm = self.comp.changed.is_some_and(|t| t.elapsed() > std::time::Duration::from_millis(900));
            if calm && input.trim_start().starts_with('#') && input.trim().len() > 4 && self.comp.ai_for != input {
                self.ask_ai(&input, &cwd);
            }
            return;
        }
        self.comp.input = input.clone();
        self.comp.changed = Some(std::time::Instant::now());
        self.comp.ai_err = None;
        self.comp.sel = 0;
        if self.comp.dismissed.as_deref().is_some_and(|d| d != input) {
            self.comp.dismissed = None;
        }
        self.comp.cands = if self.comp.dismissed.is_some() || shell == Shell::Unix && self.program.to_lowercase().contains("wsl") {
            Vec::new()
        } else {
            complete::suggest(&input, &cwd, shell)
        };
    }

    /// KI nach einer Vervollständigung bzw. dem Befehl zu einer Beschreibung („# …“) fragen.
    fn ask_ai(&mut self, input: &str, cwd: &Path) {
        let Some(cfg) = self.ai.clone() else { return };
        self.comp.ai_for = input.to_string();
        self.comp.ai_err = None;
        let shell = match complete::shell_of(&self.program) {
            Shell::Pwsh => "PowerShell",
            Shell::Cmd => "cmd.exe",
            Shell::Unix => "bash/sh",
        };
        let recent = complete::recent(8).join("\n");
        let entries: Vec<String> = std::fs::read_dir(cwd).into_iter().flatten().flatten().take(40).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        let prompt = format!(
            "Shell: {shell}\nFolder: {}\nFiles here: {}\nRecent commands:\n{recent}\n\nCurrent input: {input}",
            cwd.display(),
            entries.join(", ")
        );
        let system = "You complete shell command lines in a terminal. Reply with exactly one complete command line and nothing else: no explanation, no quotes, no code fences. \
If the input starts with '#', it describes in words what the user wants: reply with the command that does it. Otherwise reply with the most likely full command line that starts with the current input.";
        let (tx, rx) = std::sync::mpsc::channel();
        let key = input.to_string();
        std::thread::spawn(move || {
            let r = crate::ai::quick(&cfg, system, &prompt).map(|t| {
                let line = t.trim().trim_matches('`').lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("```")).unwrap_or("").to_string();
                line.trim_start_matches("$ ").trim_start_matches("PS> ").to_string()
            });
            let _ = tx.send((key, r));
        });
        self.comp.ai_rx = Some(rx);
    }

    /// Kopfzeile: Shell, Ordner, Leeren / Kopieren / Einfügen.
    fn header(&mut self, ui: &mut egui::Ui, th: &Theme, head: Rect) {
        let p = ui.painter_at(head);
        p.rect_filled(head, 0.0, th.panel);
        p.line_segment([head.left_bottom(), head.right_bottom()], egui::Stroke::new(1.0, th.line));
        let ic = Rect::from_center_size(pos2(head.left() + 18.0, head.center().y), vec2(14.0, 14.0));
        crate::ui::draw(&p, ic, crate::ui::Ic::Terminal, th.accent);
        let title = self.title();
        let t = if title == self.shell_name { title } else { format!("{}  ·  {title}", self.shell_name) };
        p.text(pos2(head.left() + 32.0, head.center().y), egui::Align2::LEFT_CENTER, t, FontId::proportional(12.5), th.text);
        let mut x = head.right() - 6.0;
        let mut btn = |ui: &mut egui::Ui, ic: crate::ui::Ic, tip: &str, id: &str| -> bool {
            x -= 26.0;
            crate::ui::tool_btn_at(ui, th, Rect::from_min_size(pos2(x, head.top() + 3.0), vec2(24.0, 24.0)), ui.id().with(("termh", id)), ic, tip, false).clicked()
        };
        if btn(ui, crate::ui::Ic::Paste, "Einfügen (Strg+Umschalt+V)", "paste") {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        if btn(ui, crate::ui::Ic::Copy, "Auswahl bzw. ganzen Bildschirm kopieren", "copy") {
            let t = self.selected_text().unwrap_or_else(|| self.screen_text());
            ui.ctx().copy_text(t.trim_end().to_string());
        }
        if btn(ui, crate::ui::Ic::Trash, "Bildschirm leeren", "clear") {
            let cmd = match complete::shell_of(&self.program) {
                Shell::Cmd => "cls",
                Shell::Pwsh => "Clear-Host",
                Shell::Unix => "clear",
            };
            self.type_line(cmd);
        }
        if self.exited() {
            p.text(pos2(x - 8.0, head.center().y), egui::Align2::RIGHT_CENTER, "beendet", FontId::proportional(12.0), th.danger);
        }
    }

    /// Vorschlag übernehmen: Rückschritte für den ersetzten Teil, dann der neue Text.
    fn accept(&mut self) {
        let Some(c) = self.comp.cands.get(self.comp.sel).cloned() else { return };
        let (back, text) = c.edit(&self.comp.input);
        let mut b = vec![0x7f; back];
        b.extend_from_slice(text.as_bytes());
        self.send(&b);
        self.comp.cands.clear();
    }

    fn input(&mut self, ui: &egui::Ui) {
        let events = ui.input(|i| i.events.clone());
        let app_cursor = self.parser.lock().map(|p| p.screen().application_cursor()).unwrap_or(false);
        for e in events {
            // Vorschlagsliste offen: Tab/→ übernehmen, ↑↓ wählen, Esc ausblenden
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = &e {
                let plain = modifiers.is_none();
                if !self.comp.cands.is_empty() && plain {
                    let n = self.comp.cands.len();
                    match key {
                        Key::Tab => {
                            self.accept();
                            continue;
                        }
                        Key::ArrowRight if self.comp.at_end && self.comp.cands.get(self.comp.sel).is_some_and(|c| c.ghost(&self.comp.input).is_some()) => {
                            self.accept();
                            continue;
                        }
                        Key::ArrowDown => {
                            self.comp.sel = (self.comp.sel + 1) % n;
                            continue;
                        }
                        Key::ArrowUp => {
                            self.comp.sel = (self.comp.sel + n - 1) % n;
                            continue;
                        }
                        Key::Escape => {
                            self.comp.dismissed = Some(self.comp.input.clone());
                            self.comp.cands.clear();
                            continue;
                        }
                        _ => {}
                    }
                }
                if *key == Key::Space && modifiers.ctrl && !modifiers.shift && !modifiers.alt && self.ai.is_some() && self.complete {
                    let (input, cwd) = (self.comp.input.clone(), self.cwd.clone());
                    if !input.trim().is_empty() {
                        self.ask_ai(&input, &cwd);
                    }
                    continue;
                }
                if *key == Key::Enter && plain && self.complete {
                    complete::remember(&self.comp.input);
                    self.comp.cands.clear();
                }
            }
            match e {
                egui::Event::Text(t) => self.send(t.as_bytes()),
                egui::Event::Paste(t) => self.paste(&t),
                egui::Event::Copy => {
                    // Strg+C ohne Auswahl = Abbruchsignal; mit Auswahl = Kopieren
                    match self.selected_text() {
                        Some(t) => {
                            ui.ctx().copy_text(t);
                            self.sel = None;
                        }
                        None => self.send(&[0x03]),
                    }
                }
                egui::Event::Cut => self.send(&[0x18]),
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if modifiers.ctrl && modifiers.shift && key == Key::C {
                        if let Some(t) = self.selected_text() {
                            ui.ctx().copy_text(t);
                        }
                        continue;
                    }
                    if modifiers.ctrl && modifiers.shift && key == Key::V {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                        continue;
                    }
                    if let Some(b) = key_bytes(key, modifiers, app_cursor) {
                        self.send(&b);
                    }
                }
                _ => {}
            }
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn key_bytes(key: Key, m: egui::Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let arrow = |c: u8| -> Vec<u8> {
        if m.ctrl {
            format!("\x1b[1;5{}", c as char).into_bytes()
        } else if app_cursor {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        }
    };
    Some(match key {
        Key::Enter => b"\r".to_vec(),
        Key::Backspace => if m.ctrl { vec![0x17] } else { vec![0x7f] },
        Key::Tab => if m.shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() },
        Key::Escape => vec![0x1b],
        Key::ArrowUp => arrow(b'A'),
        Key::ArrowDown => arrow(b'B'),
        Key::ArrowRight => arrow(b'C'),
        Key::ArrowLeft => arrow(b'D'),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::Insert => b"\x1b[2~".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::F1 => b"\x1bOP".to_vec(),
        Key::F2 => b"\x1bOQ".to_vec(),
        Key::F3 => b"\x1bOR".to_vec(),
        Key::F4 => b"\x1bOS".to_vec(),
        Key::F5 => b"\x1b[15~".to_vec(),
        Key::F6 => b"\x1b[17~".to_vec(),
        Key::F7 => b"\x1b[18~".to_vec(),
        Key::F8 => b"\x1b[19~".to_vec(),
        Key::F9 => b"\x1b[20~".to_vec(),
        Key::F10 => b"\x1b[21~".to_vec(),
        Key::F11 => b"\x1b[23~".to_vec(),
        Key::F12 => b"\x1b[24~".to_vec(),
        // Strg+Buchstabe → Steuerzeichen (Strg+C/X kommen als Copy/Cut-Ereignis)
        k if m.ctrl && !m.shift => {
            let n = k.name();
            let b = n.as_bytes();
            if b.len() == 1 && b[0].is_ascii_alphabetic() {
                vec![b[0].to_ascii_uppercase() - b'@']
            } else {
                return None;
            }
        }
        _ => return None,
    })
}

/// xterm-Farben, an das Theme angepasst (Standardfarben = Editorfarben).
fn color(c: vt100::Color, th: &Theme, fg: bool) -> Color32 {
    const BASE: [u32; 16] = [
        0x1e1e1e, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666, 0xf14c4c, 0x23d18b, 0xf5f543,
        0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
    ];
    let rgb = |h: u32| Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8);
    match c {
        vt100::Color::Default => {
            if fg {
                th.ed_text
            } else {
                Color32::TRANSPARENT
            }
        }
        vt100::Color::Idx(i) if i < 16 => {
            // im hellen Theme sind „weiß“/„hellgelb“ unlesbar → dunkler darstellen
            let mut v = BASE[i as usize];
            if !th.dark && fg && matches!(i, 3 | 7 | 11 | 15) {
                v = if i == 3 || i == 11 { 0x949400 } else { 0x444444 };
            }
            rgb(v)
        }
        vt100::Color::Idx(i) if i < 232 => {
            let i = i - 16;
            let s = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(s(i / 36), s((i / 6) % 6), s(i % 6))
        }
        vt100::Color::Idx(i) => {
            let g = 8 + (i - 232) * 10;
            Color32::from_rgb(g, g, g)
        }
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Echte Pseudo-Konsole: Befehl senden und Ausgabe im Bildschirmpuffer wiederfinden.
    #[test]
    fn conpty_roundtrip() {
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir();
        let mut t = Term::spawn("CMD", "cmd.exe", &dir, &ctx).expect("cmd.exe startet");
        t.send(b"echo FC_TERM_OK\r");
        // Befehlszeile + Ausgabe = zweimal sichtbar
        let ok = (0..50).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            t.parser.lock().unwrap().screen().contents().matches("FC_TERM_OK").count() >= 2
        });
        assert!(ok, "Ausgabe nicht gefunden: {:?}", t.parser.lock().unwrap().screen().contents());
        t.send(b"exit\r");
        assert!((0..50).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            t.exited()
        }));
    }
}
