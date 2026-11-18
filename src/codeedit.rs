//! Text- und Quelltextansicht der Vorschau auf Basis von egui `TextEdit`: Syntaxfarben, Zeilennummern,
//! Zeilenumbruch an der Breite, markieren und kopieren (Strg+C, Strg+A). Im Bearbeitungsmodus zusätzlich
//! Vorschläge beim Schreiben (Funktionen, Variablen, Typen, Schlüsselwörter und Wörter der Datei) und eine
//! Liste der Definitionen zum Springen.
//!
//! Erkennung ohne Sprachserver, sprachübergreifend über Muster: Definitionen (fn/def/function/func …,
//! let/var/const …, struct/class/enum …), Aufrufe (Name vor „(“) und alle übrigen Bezeichner.

use crate::syntax::{Highlight, Spans};
use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, FontId, Galley, Key, Modifiers, Rect, Stroke};
use regex::Regex;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Kind {
    Function,
    Type,
    Variable,
    /// Methode/Eigenschaft (nach „.“)
    Member,
    /// Standardfunktion, -typ oder Makro der Sprache
    Builtin,
    Keyword,
    Word,
}

impl Kind {
    fn badge(self) -> (&'static str, &'static str) {
        match self {
            Kind::Function => ("f", "Funktion"),
            Kind::Type => ("T", "Typ"),
            Kind::Variable => ("v", "Variable"),
            Kind::Member => ("m", "Methode/Feld"),
            Kind::Builtin => ("s", "Standard"),
            Kind::Keyword => ("k", "Schlüsselwort"),
            Kind::Word => ("w", "Wort"),
        }
    }
    fn color(self, th: &Theme) -> Color32 {
        match self {
            Kind::Function => th.function,
            Kind::Type => th.ty,
            Kind::Variable => th.number,
            Kind::Member => th.string,
            Kind::Builtin => th.ty,
            Kind::Keyword => th.keyword,
            Kind::Word => th.sub,
        }
    }
}

/// Erkannter Bezeichner; `line` = Zeile der Definition (falls es eine gibt), `sig` = Zeile der Definition als
/// Text (Signatur), `count` = wie oft er vorkommt.
#[derive(Clone, Debug)]
pub struct Sym {
    pub name: String,
    pub kind: Kind,
    pub line: Option<usize>,
    pub sig: Option<String>,
    pub count: u32,
}

impl Sym {
    fn plain(name: &str, kind: Kind) -> Sym {
        Sym { name: name.to_string(), kind, line: None, sig: None, count: 0 }
    }
}

struct Popup {
    /// Zeichenposition, an der das getippte Wortstück beginnt
    start: usize,
    items: Vec<Sym>,
    sel: usize,
}

pub struct CodeEdit {
    id: egui::Id,
    /// (Schlüssel, Galley) – neu aufgebaut nur, wenn sich Text, Breite oder Farben ändern
    /// fertiges Layout: (Inhalt, Stand der Hervorhebung, wann gebaut, Layout)
    cache: Option<(u64, usize, Instant, Arc<Galley>)>,
    syms: Vec<Sym>,
    syms_key: u64,
    syms_at: Instant,
    popup: Option<Popup>,
    /// Sprung zu einer Zeile (aus der Symbolliste), wird beim nächsten Zeichnen ausgeführt
    jump: Option<usize>,
    scroll_to: Option<f32>,
    /// Vorschlag per Mausklick gewählt: im nächsten Bild übernehmen
    click_accept: bool,
    /// nur für Bildschirmfotos: (Zeile, Text) tippen
    pub dev_type: Option<(usize, String)>,
    /// Zeile und Spalte des Cursors (ab 1)
    pub caret: Option<(usize, usize)>,
    /// Sprache (nach Dateiendung) und zuletzt übernommene Vorschläge (kommen künftig weiter nach oben)
    lang: &'static crate::langs::Lang,
    recent: Vec<String>,
    /// große Dateien nur ansehen: Anfang jeder Zeile (Inhalt-Schlüssel, Byte-Positionen)
    line_starts: Option<(u64, Vec<usize>)>,
}

/// ab so vielen Zeilen zeigt die reine Ansicht nur die sichtbaren Zeilen (statt den ganzen Text zu setzen)
const VIRTUAL_LINES: usize = 1500;

impl CodeEdit {
    /// Ansicht für große Dateien: nur die sichtbaren Zeilen werden gesetzt (schneller Tab-Wechsel und
    /// flüssiges Scrollen auch bei zehntausenden Zeilen). Auswählen und Bearbeiten im Bearbeiten-Modus.
    #[allow(clippy::too_many_arguments)]
    fn show_large(&mut self, ui: &mut egui::Ui, th: &Theme, text: &str, ext: &str, hl: Option<&Highlight>, inner: egui::Rect, gutter: f32, font: &FontId) {
        let key = quick_hash(text);
        if self.line_starts.as_ref().is_none_or(|(k, _)| *k != key) {
            let mut v = vec![0usize];
            v.extend(text.match_indices('\n').map(|(i, _)| i + 1));
            self.line_starts = Some((key, v));
        }
        let starts = &self.line_starts.as_ref().unwrap().1;
        let row_h = ui.fonts(|f| f.row_height(font)) + 2.0;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner.shrink2(vec2(0.0, 4.0))));
        let mut sa = egui::ScrollArea::both().id_salt(self.id.with("gross")).auto_shrink([false, false]);
        if let Some(y) = self.scroll_to.take() {
            sa = sa.vertical_scroll_offset(y.max(0.0));
        }
        if let Some(line) = self.jump.take() {
            sa = sa.vertical_scroll_offset(line.saturating_sub(4) as f32 * row_h);
        }
        sa.show_rows(&mut child, row_h, starts.len(), |ui, range| {
            for i in range {
                let a = starts[i];
                let b = starts.get(i + 1).map_or(text.len(), |e| e.saturating_sub(1));
                let line = text.get(a..b.max(a)).unwrap_or("").trim_end_matches('\r');
                let mut job = egui::text::LayoutJob::default();
                line_job(&mut job, line, ext, th, font, hl.and_then(|h| h.line(i)));
                job.wrap = egui::text::TextWrapping::no_max_width();
                let g = ui.fonts(|f| f.layout_job(job));
                let (r, _) = ui.allocate_exact_size(vec2(gutter + 8.0 + g.size().x + 16.0, row_h), egui::Sense::hover());
                ui.painter().text(pos2(r.left() + gutter - 2.0, r.center().y), egui::Align2::RIGHT_CENTER, (i + 1).to_string(), font.clone(), th.line_no);
                ui.painter().galley(pos2(r.left() + gutter + 8.0, r.center().y - g.size().y / 2.0), g, th.ed_text);
            }
        });
        if hl.is_some_and(|h| !h.done()) {
            ui.ctx().request_repaint_after(Duration::from_millis(300));
        }
    }

    pub fn new(salt: &str) -> CodeEdit {
        CodeEdit {
            id: egui::Id::new(("codeedit", salt.to_string())),
            cache: None,
            line_starts: None,
            syms: Vec::new(),
            syms_key: 0,
            syms_at: Instant::now() - Duration::from_secs(10),
            popup: None,
            jump: None,
            scroll_to: None,
            click_accept: false,
            dev_type: None,
            caret: None,
            lang: &crate::langs::GENERIC,
            recent: Vec::new(),
        }
    }

    /// Definitionen (Funktionen, Typen, Variablen) für die Sprungliste, nach Zeile sortiert.
    pub fn definitions(&self) -> Vec<Sym> {
        let mut v: Vec<Sym> = self.syms.iter().filter(|s| s.line.is_some() && matches!(s.kind, Kind::Function | Kind::Type | Kind::Variable)).cloned().collect();
        v.sort_by_key(|s| s.line);
        v
    }

    pub fn jump_to(&mut self, line: usize) {
        self.jump = Some(line);
    }

    /// Text in genau `size` zeigen. `editable` = bearbeiten (mit Vorschlägen), sonst nur markieren/kopieren.
    /// `hl` nur übergeben, wenn die Hervorhebung zu genau diesem Text gehört. Gibt `true` zurück, wenn sich
    /// der Text geändert hat.
    #[allow(clippy::too_many_arguments)]
    pub fn show(&mut self, ui: &mut egui::Ui, th: &Theme, text: &mut dyn egui::TextBuffer, editable: bool, ext: &str, hl: Option<&Highlight>, size: egui::Vec2) -> bool {
        let font = FontId::monospace(12.5);
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter().rect_filled(rect, 6.0, th.ed_bg);
        ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, th.line));
        let inner = rect.shrink(1.0);
        let lines = text.as_str().bytes().filter(|b| *b == b'\n').count() + 1;
        let gutter = format!("{lines}").len() as f32 * 8.0 + 14.0;
        // große Dateien nur ansehen: Zeile für Zeile nur das Sichtbare setzen und zeichnen
        if !editable && lines > VIRTUAL_LINES {
            self.show_large(ui, th, text.as_str(), ext, hl, inner, gutter, &font);
            return false;
        }

        // Bezeichner neu erkennen (gedrosselt beim Tippen; große Dateien seltener)
        if editable {
            self.lang = crate::langs::for_ext(ext);
            let key = quick_hash(text.as_str());
            let every = if text.as_str().len() > 150_000 { 900 } else { 400 };
            if key != self.syms_key && (self.syms_at.elapsed() > Duration::from_millis(every) || self.syms.is_empty()) {
                self.syms = scan(text.as_str(), self.lang);
                self.syms_key = key;
                self.syms_at = Instant::now();
            } else if key != self.syms_key {
                ui.ctx().request_repaint_after(Duration::from_millis(420));
            }
        }

        // Tasten für das Vorschlagsfenster abfangen, bevor das Textfeld sie sieht
        let mut accept = std::mem::take(&mut self.click_accept);
        if let Some(p) = &mut self.popup {
            let n = p.items.len().max(1);
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    p.sel = (p.sel + 1) % n;
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    p.sel = (p.sel + n - 1) % n;
                }
                if i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab) {
                    accept = true;
                }
            });
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.popup = None;
            }
        }

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let mut sa = egui::ScrollArea::vertical().id_salt(self.id.with("scroll")).auto_shrink([false, false]);
        if let Some(y) = self.scroll_to.take() {
            sa = sa.vertical_scroll_offset(y.max(0.0));
        }
        let cache = &mut self.cache;
        let dark = th.dark;
        let mut layouter = |ui: &egui::Ui, s: &str, wrap: f32| -> Arc<Galley> {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut h);
            wrap.to_bits().hash(&mut h);
            dark.hash(&mut h);
            th.text.hash(&mut h);
            let key = h.finish();
            let progress = hl.map(Highlight::progress).unwrap_or(0);
            if let Some((k, p, at, g)) = cache.as_ref() {
                // gleicher Text: altes Layout behalten, solange die Hervorhebung nur weiterläuft – große Dateien
                // sonst bei jedem Bild komplett neu gesetzt (spürbar beim Tab-Wechsel und Scrollen)
                if *k == key && (*p == progress || (at.elapsed() < Duration::from_millis(350) && hl.is_some_and(|h| !h.done()))) {
                    if *p != progress {
                        ui.ctx().request_repaint_after(Duration::from_millis(360));
                    }
                    return g.clone();
                }
            }
            let mut job = egui::text::LayoutJob::default();
            for (i, line) in s.split('\n').enumerate() {
                if i > 0 {
                    job.append("\n", 0.0, egui::TextFormat::simple(font.clone(), th.ed_text));
                }
                line_job(&mut job, line, ext, th, &font, hl.and_then(|h| h.line(i)));
            }
            job.wrap = egui::text::TextWrapping { max_width: wrap, break_anywhere: true, ..Default::default() };
            let g = ui.fonts(|f| f.layout_job(job));
            *cache = Some((key, progress, Instant::now(), g.clone()));
            g
        };
        let mut changed = false;
        let mut out_state = None;
        sa.show(&mut child, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (g, _) = ui.allocate_exact_size(vec2(gutter + 4.0, 1.0), egui::Sense::hover());
                let te = egui::TextEdit::multiline(text)
                    .id(self.id)
                    .font(font.clone())
                    .code_editor()
                    .frame(false)
                    .margin(egui::Margin::symmetric(0.0, 4.0))
                    .desired_width(ui.available_width() - 12.0)
                    .lock_focus(editable)
                    .interactive(true)
                    .layouter(&mut layouter);
                let out = te.show(ui);
                changed = out.response.changed();
                // Zeilennummern an jeder ersten Bildschirmzeile einer Textzeile (nur was sichtbar ist)
                let clip = ui.clip_rect();
                let mut line_no = 1;
                let mut starts = true;
                for row in &out.galley.rows {
                    let y = out.galley_pos.y + row.rect.top();
                    if starts && y + row.rect.height() >= clip.top() && y <= clip.bottom() {
                        ui.painter().text(pos2(g.left() + gutter - 6.0, y + row.rect.height() / 2.0), egui::Align2::RIGHT_CENTER, line_no.to_string(), font.clone(), th.line_no);
                    }
                    if y > clip.bottom() {
                        break;
                    }
                    starts = row.ends_with_newline;
                    if row.ends_with_newline {
                        line_no += 1;
                    }
                }
                out_state = Some((out, ui.min_rect().top()));
            });
        });
        let Some((out, content_top)) = out_state else { return changed };

        // Cursor: Zeile/Spalte, Sprung aus der Symbolliste
        let cursor = out.cursor_range.map(|c| c.primary.ccursor.index);
        self.caret = cursor.map(|c| line_col(text.as_str(), c));
        // nur für Bildschirmfotos: „Zeile:Text“ am Ende der Zeile tippen (öffnet die Vorschläge)
        let mut dev_fired = false;
        if let Some((line, typed)) = self.dev_type.take() {
            match cursor {
                Some(c) if editable => {
                    text.insert_text(&typed, c);
                    let at = c + typed.chars().count();
                    let mut st = out.state.clone();
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
                    st.store(ui.ctx(), self.id);
                    let (start, prefix) = word_before(text.as_str(), at);
                    let member = member_ctx(text.as_str(), start);
                    let line = line_col(text.as_str(), at).0 - 1;
                    let items = self.candidates(&prefix, member, line);
                    self.popup = (!items.is_empty()).then_some(Popup { start, items, sel: 0 });
                    dev_fired = true;
                }
                _ => {
                    let idx = char_of_line(text.as_str(), line + 1).saturating_sub(1);
                    let mut st = out.state.clone();
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(idx))));
                    st.store(ui.ctx(), self.id);
                    ui.ctx().memory_mut(|m| m.request_focus(self.id));
                    self.dev_type = Some((line, typed));
                }
            }
            ui.ctx().request_repaint();
        }
        if let Some(line) = self.jump.take() {
            let idx = char_of_line(text.as_str(), line);
            let mut st = out.state.clone();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(idx))));
            st.store(ui.ctx(), self.id);
            ui.ctx().memory_mut(|m| m.request_focus(self.id));
            let y = out.galley.pos_from_ccursor(egui::text::CCursor::new(idx)).top();
            self.scroll_to = Some(out.galley_pos.y - content_top + y - inner.height() / 3.0);
            ui.ctx().request_repaint();
        }
        if !editable {
            self.popup = None;
            return false;
        }

        // Vorschläge: nach dem Tippen eines Wortstücks öffnen bzw. nachführen
        if let Some(c) = cursor.filter(|_| !dev_fired) {
            if accept {
                if let Some(p) = self.popup.take() {
                    if let Some(s) = p.items.get(p.sel) {
                        let c = c.max(p.start);
                        let typed: String = text.as_str().chars().skip(p.start).take(c - p.start).collect();
                        text.delete_char_range(p.start..c);
                        // Schlüsselwörter in der Schreibweise des Getippten (SQL: „sel“ → „select“)
                        let name = if s.kind == Kind::Keyword && !typed.is_empty() && typed.chars().all(|c| !c.is_uppercase()) { s.name.to_lowercase() } else { s.name.clone() };
                        // Funktionen und Makros mit Klammern, Cursor dazwischen
                        let callable = s.kind == Kind::Function || (s.kind == Kind::Member && s.sig.as_deref() == Some("()")) || name.ends_with('!');
                        let call = callable && next_char(text.as_str(), p.start) != Some('(');
                        let ins = if call { format!("{name}()") } else { name.clone() };
                        text.insert_text(&ins, p.start);
                        let at = p.start + name.chars().count() + usize::from(call);
                        self.recent.retain(|r| *r != s.name);
                        self.recent.insert(0, s.name.clone());
                        self.recent.truncate(30);
                        let mut st = out.state.clone();
                        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
                        st.store(ui.ctx(), self.id);
                        changed = true;
                        ui.ctx().request_repaint();
                    }
                }
            } else if changed {
                let (start, prefix) = word_before(text.as_str(), c);
                let member = member_ctx(text.as_str(), start);
                // nach „.“ sofort Methoden/Felder anbieten, sonst ab zwei Zeichen
                if prefix.chars().count() >= 2 || (member && self.popup.is_none() && prefix.is_empty()) || (member && !prefix.is_empty()) {
                    let line = line_col(text.as_str(), c).0 - 1;
                    let items = self.candidates(&prefix, member, line);
                    let sel = self.popup.as_ref().map_or(0, |p| p.sel).min(items.len().saturating_sub(1));
                    self.popup = (!items.is_empty()).then_some(Popup { start, items, sel });
                } else {
                    self.popup = None;
                }
            } else if out.response.clicked() || self.popup.as_ref().is_some_and(|p| c < p.start) {
                self.popup = None;
            }
            if !out.response.has_focus() {
                self.popup = None;
            }
        }

        // Vorschlagsfenster unter dem Cursor
        if let (Some(p), Some(cr)) = (&mut self.popup, out.cursor_range) {
            let caret = out.galley.pos_from_cursor(&cr.primary).translate(out.galley_pos.to_vec2());
            let mut clicked = None;
            egui::Area::new(self.id.with("popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(caret.left_bottom() + vec2(0.0, 2.0))
                .constrain(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4.0)).show(ui, |ui| {
                        ui.set_min_width(340.0);
                        for (k, s) in p.items.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(340.0), 22.0), egui::Sense::click());
                            if k == p.sel {
                                ui.painter().rect_filled(r, 4.0, th.sel);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(r, 4.0, th.hover);
                            }
                            let (letter, what) = s.kind.badge();
                            let b = Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(15.0, 15.0));
                            ui.painter().rect_filled(b, 3.0, s.kind.color(th).gamma_multiply(0.25));
                            ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, letter, FontId::monospace(10.5), s.kind.color(th));
                            ui.painter().text(pos2(b.right() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, &s.name, FontId::monospace(12.5), th.text);
                            // Signatur der Definition (gekürzt) bzw. Art und Zeile
                            let detail = match (&s.sig, s.line) {
                                (Some(sig), Some(l)) if s.kind == Kind::Function && sig.len() > 2 => {
                                    let sig: String = sig.chars().take(34).collect();
                                    format!("{sig}{} · Z. {}", if s.sig.as_ref().is_some_and(|x| x.chars().count() > 34) { "…" } else { "" }, l + 1)
                                }
                                (_, Some(l)) => format!("{what} · Z. {}", l + 1),
                                _ if s.kind == Kind::Builtin || (s.kind == Kind::Keyword && self.lang.name != "Text") => format!("{what} · {}", self.lang.name),
                                _ => what.to_string(),
                            };
                            ui.painter().text(pos2(r.right() - 6.0, r.center().y), egui::Align2::RIGHT_CENTER, detail, FontId::proportional(11.0), th.sub);
                            if resp.clicked() {
                                clicked = Some(k);
                            }
                        }
                        ui.label(egui::RichText::new("↑↓ wählen · Tab/Enter übernehmen · Esc schließen").size(10.5).color(th.sub));
                    });
                });
            if let Some(k) = clicked {
                // Übernehmen im nächsten Bild (wie Tab), damit der Cursor stimmt
                p.sel = k;
                self.click_accept = true;
                ui.ctx().memory_mut(|m| m.request_focus(self.id));
                ui.ctx().request_repaint();
            }
        }
        changed
    }

    /// Vorschläge zum Wortstück, nach Güte sortiert: wie gut der Name passt (Anfang, Wortteile wie „bs“ für
    /// „berechne_summe“, enthalten, unscharf), was er ist (Definition, Standard, Schlüsselwort, Wort), wie nah
    /// seine Definition am Cursor liegt, wie oft er vorkommt und ob er zuletzt gewählt wurde.
    /// `member` = nach „.“, „->“ oder „::“: Methoden und Felder zuerst.
    fn candidates(&self, prefix: &str, member: bool, cursor_line: usize) -> Vec<Sym> {
        let lang = self.lang;
        let mut pool: HashMap<String, Sym> = HashMap::new();
        let mut add = |s: Sym| {
            if s.name == prefix {
                return;
            }
            match pool.get_mut(&s.name) {
                // die aussagekräftigere Art behalten (Definition vor Wort), Zählung zusammenführen
                Some(e) if (s.kind, s.line.is_none()) < (e.kind, e.line.is_none()) => {
                    let n = e.count;
                    *e = s;
                    e.count += n;
                }
                Some(e) => e.count += s.count,
                None => {
                    pool.insert(s.name.clone(), s);
                }
            }
        };
        for s in &self.syms {
            add(s.clone());
        }
        if member {
            for m in lang.members {
                add(Sym::plain(m, Kind::Member));
            }
        } else {
            for k in lang.keywords.iter().chain(if lang.name == "Text" { crate::preview::KEYWORDS } else { &[] }) {
                add(Sym::plain(k, Kind::Keyword));
            }
            for b in lang.builtins {
                add(Sym::plain(b, Kind::Builtin));
            }
        }
        let mut v: Vec<(i32, Sym)> = pool
            .into_values()
            .filter_map(|s| {
                let m = if prefix.is_empty() { 50 } else { match_score(&s.name, prefix)? };
                let kind = match s.kind {
                    Kind::Member if member => 30,
                    Kind::Function | Kind::Type | Kind::Variable if s.line.is_some() => 22,
                    Kind::Function if member => 18,
                    Kind::Function => 14,
                    Kind::Builtin => 10,
                    Kind::Keyword => 8,
                    Kind::Member => 4,
                    _ => 0,
                };
                if member && matches!(s.kind, Kind::Keyword | Kind::Builtin | Kind::Type) {
                    return None;
                }
                // Nähe der Definition zum Cursor (lokale Variablen, Parameter)
                let near = s.line.map_or(0, |l| {
                    let d = l.abs_diff(cursor_line);
                    if d < 40 {
                        10
                    } else if d < 200 {
                        5
                    } else {
                        0
                    }
                });
                let freq = (s.count.min(12) as i32) / 2;
                let recent = self.recent.iter().position(|r| *r == s.name).map_or(0, |i| 25 - i as i32 / 2);
                let len = -((s.name.chars().count() as i32 - prefix.chars().count() as i32).max(0) / 4);
                Some((m + kind + near + freq + recent + len, s))
            })
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.len().cmp(&b.1.name.len())).then(a.1.name.cmp(&b.1.name)));
        v.into_iter().take(10).map(|(_, s)| s).collect()
    }
}

/// Wie gut passt `name` zum Getippten? Exakter Anfang > Anfang ohne Groß/Klein > Wortteile („bs“ →
/// „berechne_summe“, „gEl“ → „getElement“) > enthalten > unscharf (Buchstaben in Reihenfolge).
fn match_score(name: &str, typed: &str) -> Option<i32> {
    if name.starts_with(typed) {
        return Some(100);
    }
    let (n, t) = (name.to_lowercase(), typed.to_lowercase());
    if n.starts_with(&t) {
        return Some(90);
    }
    // Anfangsbuchstaben der Wortteile (snake_case, camelCase, kebab-case)
    let mut initials = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        let start = match prev {
            None => true,
            Some(p) => (p == '_' || p == '-') && c != '_' || (p.is_lowercase() && c.is_uppercase()),
        };
        if start && c.is_alphanumeric() {
            initials.extend(c.to_lowercase());
        }
        prev = Some(c);
    }
    if t.chars().count() >= 2 && initials.starts_with(&t) {
        return Some(75);
    }
    if n.contains(&t) {
        return Some(60);
    }
    // unscharf: alle Buchstaben in Reihenfolge, erster passt
    let mut it = n.chars();
    let all = t.chars().all(|c| it.any(|x| x == c));
    if all && t.chars().count() >= 2 {
        return Some(if n.chars().next() == t.chars().next() { 45 } else { 30 });
    }
    None
}

/// Steht vor `start` ein „.“, „->“ oder „::“ (Methode/Feld eines Objekts)? Zahlen wie „3.“ zählen nicht.
fn member_ctx(text: &str, start: usize) -> bool {
    if start == 0 {
        return false;
    }
    let before: Vec<char> = text.chars().skip(start.saturating_sub(3)).take(start.min(3)).collect();
    match before.as_slice() {
        [.., a, '.'] => a.is_alphanumeric() && !a.is_ascii_digit() || matches!(a, ')' | ']' | '_' | '>' | '"' | '\''),
        [.., '-', '>'] | [.., ':', ':'] => true,
        _ => false,
    }
}

/// Eine Zeile als Farbabschnitte anhängen – Text bleibt Zeichen für Zeichen gleich (wichtig für den Cursor).
fn line_job(job: &mut egui::text::LayoutJob, line: &str, ext: &str, th: &Theme, font: &FontId, spans: Option<Spans>) {
    let fmt = |c: Color32| egui::TextFormat::simple(font.clone(), c);
    let Some(spans) = spans.filter(|s| !s.is_empty()) else {
        let j = crate::preview::highlight(line, ext, th, font);
        for s in &j.sections {
            job.append(&j.text[s.byte_range.clone()], 0.0, s.format.clone());
        }
        return;
    };
    let mut pos = 0usize;
    for (from, to, cls) in spans {
        let (from, to) = (from as usize, to as usize);
        if to > line.len() || from < pos || !line.is_char_boundary(from) || !line.is_char_boundary(to) {
            break;
        }
        if from > pos {
            job.append(&line[pos..from], 0.0, fmt(th.ed_text));
        }
        job.append(&line[from..to], 0.0, fmt(cls.color(th)));
        pos = to;
    }
    if pos < line.len() {
        job.append(&line[pos..], 0.0, fmt(th.ed_text));
    }
}

fn quick_hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Wortstück vor dem Cursor: (Zeichenposition des Anfangs, Text).
fn word_before(text: &str, cursor: usize) -> (usize, String) {
    let chars: Vec<char> = text.chars().take(cursor).collect();
    let mut i = chars.len();
    while i > 0 && is_ident(chars[i - 1]) {
        i -= 1;
    }
    // Zahlen sind keine Bezeichner
    if chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
        return (cursor, String::new());
    }
    (i, chars[i..].iter().collect())
}

fn next_char(text: &str, at: usize) -> Option<char> {
    text.chars().nth(at)
}

fn line_col(text: &str, cursor: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for c in text.chars().take(cursor) {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Zeichenposition des Anfangs von Zeile `line` (ab 0).
fn char_of_line(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut n = 0;
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            n += 1;
            if n == line {
                return i + 1;
            }
        }
    }
    text.chars().count()
}

/// Bezeichner der Datei erkennen – Kommentare und Zeichenketten zählen nicht mit:
/// - Definitionen mit Zeile und Signatur: Funktionen (fn/def/function/func …, auch Go-Methoden und
///   C-artige „Typ name(…)“), Typen (struct/class/enum …), Variablen (let/var/const …, Zuweisungen am
///   Zeilenanfang, „:=“, Schleifenvariablen, C-artige „Typ name =“) und die Parameter jeder Funktion
/// - Methoden und Felder (nach „.“, „->“, „::“), mit Klammern, wenn sie aufgerufen werden
/// - alle übrigen Bezeichner als Wörter (Aufrufe „name(“ als Funktion), mit Anzahl
pub fn scan(text: &str, lang: &crate::langs::Lang) -> Vec<Sym> {
    struct Re {
        func: Regex,
        func_bare: Regex,
        ty: Regex,
        var: Regex,
        assign: Regex,
        walrus: Regex,
        for_var: Regex,
        c_decl: Regex,
        member: Regex,
        word: Regex,
    }
    static RE: OnceLock<Re> = OnceLock::new();
    let re = RE.get_or_init(|| Re {
        // Name und Parameterliste; Go-Empfänger „func (r *T) Name(“, Generika „<T>“
        func: Regex::new(r"\b(?:fn|def|function|func|fun|sub|proc|procedure|method)\s+(?:\([^)]*\)\s*)?([A-Za-z_$][\w$]*)\s*(?:<[^>(]*>)?\s*\(([^)]*)\)").unwrap(),
        func_bare: Regex::new(r"\b(?:def|function|sub)\s+([A-Za-z_][\w]*)").unwrap(),
        ty: Regex::new(r"\b(?:struct|enum|class|trait|interface|type|union|record|impl|mod|namespace|module)\s+([A-Za-z_]\w*)").unwrap(),
        var: Regex::new(r"\b(?:let|var|const|static|auto|my|local|val|global|readonly)\s+(?:mut\s+|ref\s+)?([A-Za-z_$][\w$]*)").unwrap(),
        assign: Regex::new(r"(?m)^[ \t]*(?:export\s+)?\$?([A-Za-z_]\w*)[ \t]*(?::[^=\n]+)?=[^=]").unwrap(),
        walrus: Regex::new(r"\b([A-Za-z_]\w*)\s*:=").unwrap(),
        for_var: Regex::new(r"\bfor\s*\(?\s*(?:let\s+|var\s+|const\s+|auto&?\s+|int\s+|\(\s*)?(?:mut\s+)?([A-Za-z_]\w*)(?:\s*,\s*([A-Za-z_]\w*))?\)?\s+(?:in|of|:)\s").unwrap(),
        // „Typ name(“ / „Typ name =“ (C, C++, C#, Java)
        c_decl: Regex::new(r"(?m)^[ \t]*(?:(?:public|private|protected|internal|static|final|const|virtual|override|inline|extern|unsigned|signed|async|readonly)\s+)*([A-Za-z_][\w:<>,\[\]]*[\*&]*)\s+[\*&]*([A-Za-z_]\w*)\s*(\(|=|;|\[)").unwrap(),
        member: Regex::new(r"(?:\.|->|::)([A-Za-z_]\w*)(\s*\()?").unwrap(),
        word: Regex::new(r"[A-Za-z_$][A-Za-z0-9_$]*!?").unwrap(),
    });
    let clean = crate::langs::blank_comments_strings(text, lang);
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line_of = |byte: usize| starts.partition_point(|&s| s <= byte).saturating_sub(1);
    let sig_of = |line: usize| -> String {
        let a = starts[line];
        let b = starts.get(line + 1).map_or(text.len(), |e| e - 1);
        text[a..b.max(a)].trim().trim_end_matches('{').trim().to_string()
    };
    let is_kw = |n: &str| lang.keywords.contains(&n) || crate::preview::KEYWORDS.contains(&n);
    let mut out: HashMap<String, Sym> = HashMap::new();
    let define = |out: &mut HashMap<String, Sym>, name: &str, kind: Kind, at: usize| {
        if name.is_empty() || is_kw(name) {
            return;
        }
        let line = line_of(at);
        out.entry(name.to_string()).or_insert(Sym { name: name.to_string(), kind, line: Some(line), sig: Some(sig_of(line)), count: 0 });
    };
    // Parameter: „name: Typ“, „Typ name“ (C), „name Typ“ (Go), „name=Vorgabe“, „&self“ …
    let params_of = |out: &mut HashMap<String, Sym>, list: &str, at: usize| {
        for part in list.split(',') {
            let part = part.split('=').next().unwrap_or("").trim();
            let decl = part.split(':').next().unwrap_or("").trim();
            let idents: Vec<&str> = decl.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).filter(|s| !s.is_empty() && !is_kw(s)).collect();
            let name = if lang.name_first && !part.contains(':') { idents.first() } else { idents.last() };
            if let Some(n) = name {
                if !matches!(*n, "self" | "this" | "cls") {
                    define(out, n.trim_start_matches('$'), Kind::Variable, at);
                }
            }
        }
    };
    for c in re.func.captures_iter(&clean) {
        let (Some(m), params) = (c.get(1), c.get(2)) else { continue };
        define(&mut out, m.as_str(), Kind::Function, m.start());
        if let Some(ps) = params {
            params_of(&mut out, ps.as_str(), ps.start());
        }
    }
    for c in re.func_bare.captures_iter(&clean) {
        if let Some(m) = c.get(1) {
            define(&mut out, m.as_str(), Kind::Function, m.start());
        }
    }
    for c in re.ty.captures_iter(&clean) {
        if let Some(m) = c.get(1) {
            define(&mut out, m.as_str(), Kind::Type, m.start());
        }
    }
    for regex in [&re.var, &re.assign, &re.walrus] {
        for c in regex.captures_iter(&clean) {
            let Some(m) = c.get(1) else { continue };
            // „x = (…) =>“, „x = function“, „x = lambda“ sind Funktionen
            let rest = &clean[m.end()..clean[m.end()..].find('\n').map_or(clean.len(), |e| m.end() + e)];
            let kind = if rest.contains("=>") || rest.contains("function") || rest.contains("lambda") { Kind::Function } else { Kind::Variable };
            define(&mut out, m.as_str(), kind, m.start());
        }
    }
    for c in re.for_var.captures_iter(&clean) {
        for g in [c.get(1), c.get(2)].into_iter().flatten() {
            define(&mut out, g.as_str(), Kind::Variable, g.start());
        }
    }
    if matches!(lang.name, "C/C++" | "C#/Java/Kotlin") {
        for c in re.c_decl.captures_iter(&clean) {
            let (Some(ty), Some(m), Some(after)) = (c.get(1), c.get(2), c.get(3)) else { continue };
            // „return foo(“, „else x =“ usw. sind keine Deklarationen
            if matches!(ty.as_str(), "return" | "else" | "new" | "delete" | "throw" | "case" | "goto" | "await" | "yield" | "using" | "import" | "package") {
                continue;
            }
            let func = after.as_str() == "(";
            define(&mut out, m.as_str(), if func { Kind::Function } else { Kind::Variable }, m.start());
            if func {
                if let Some(close) = clean[after.end()..].find(')') {
                    params_of(&mut out, &clean[after.end()..after.end() + close], after.end());
                }
            }
        }
    }
    // Methoden und Felder
    for c in re.member.captures_iter(&clean) {
        let Some(m) = c.get(1) else { continue };
        let call = c.get(2).is_some();
        let e = out.entry(m.as_str().to_string()).or_insert(Sym { name: m.as_str().to_string(), kind: Kind::Member, line: None, sig: None, count: 0 });
        e.count += 1;
        if call && e.kind == Kind::Member {
            e.sig = Some("()".into());
        }
    }
    // alle übrigen Bezeichner; Name vor „(“ = Aufruf einer Funktion
    let bytes = clean.as_bytes();
    for m in re.word.find_iter(&clean) {
        let name = m.as_str();
        if name.len() < 3 || is_kw(name) {
            continue;
        }
        // „.name“ wurde oben als Methode/Feld gezählt
        if m.start() > 0 && bytes[m.start() - 1] == b'.' {
            continue;
        }
        let mut j = m.end();
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        let call = bytes.get(j) == Some(&b'(');
        let e = out.entry(name.to_string()).or_insert(Sym { name: name.to_string(), kind: if call { Kind::Function } else { Kind::Word }, line: None, sig: None, count: 0 });
        e.count += 1;
        if call && matches!(e.kind, Kind::Word | Kind::Member) && e.line.is_none() {
            e.kind = Kind::Function;
        }
    }
    out.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langs;

    fn kind_of(syms: &[Sym], name: &str) -> Option<Kind> {
        syms.iter().find(|s| s.name == name).map(|s| s.kind)
    }

    fn top(text: &str, ext: &str, typed: &str, member: bool, line: usize) -> Vec<String> {
        let mut ce = CodeEdit::new("t");
        ce.lang = langs::for_ext(ext);
        ce.syms = scan(text, ce.lang);
        ce.candidates(typed, member, line).into_iter().map(|s| s.name).collect()
    }

    #[test]
    fn rust() {
        let t = "struct Konto { stand: f64 }\n// fn kommentar_fn() {}\nfn einzahlen(konto: &mut Konto, betrag: f64) -> f64 {\n    let neu = konto.stand + betrag;\n    konto.stand = neu;\n    let s = \"text_in_string\";\n    neu\n}\n";
        let s = scan(t, &langs::RUST);
        assert_eq!(kind_of(&s, "einzahlen"), Some(Kind::Function));
        assert_eq!(kind_of(&s, "Konto"), Some(Kind::Type));
        assert_eq!(kind_of(&s, "konto"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "betrag"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "neu"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "stand"), Some(Kind::Member));
        // Kommentare und Zeichenketten zählen nicht
        assert_eq!(kind_of(&s, "kommentar_fn"), None);
        assert_eq!(kind_of(&s, "text_in_string"), None);
        let sig = s.iter().find(|x| x.name == "einzahlen").and_then(|x| x.sig.clone()).unwrap();
        assert!(sig.starts_with("fn einzahlen(konto"), "{sig}");
    }

    #[test]
    fn python() {
        let t = "class Lager:\n    def hinzufuegen(self, artikel, menge=1):\n        self.bestand[artikel] = menge\n\nfor zeile, wert in daten:\n    gesamt = wert * 2\n";
        let s = scan(t, &langs::PYTHON);
        assert_eq!(kind_of(&s, "Lager"), Some(Kind::Type));
        assert_eq!(kind_of(&s, "hinzufuegen"), Some(Kind::Function));
        assert_eq!(kind_of(&s, "artikel"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "menge"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "zeile"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "wert"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "gesamt"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "self"), None);
    }

    #[test]
    fn javascript() {
        let t = "const holeDaten = async (url) => fetch(url);\nfunction zeigeListe(eintraege, filter) {\n  let anzahl = eintraege.length;\n  eintraege.forEach(e => console.log(e));\n}\nclass Benutzer {}\n";
        let s = scan(t, &langs::JS);
        assert_eq!(kind_of(&s, "holeDaten"), Some(Kind::Function));
        assert_eq!(kind_of(&s, "zeigeListe"), Some(Kind::Function));
        assert_eq!(kind_of(&s, "eintraege"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "anzahl"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "Benutzer"), Some(Kind::Type));
        assert_eq!(kind_of(&s, "forEach"), Some(Kind::Member));
    }

    #[test]
    fn c_and_go() {
        let c = "static int zaehle_zeilen(const char *pfad, int max) {\n    int anzahl = 0;\n    return anzahl;\n}\n";
        let s = scan(c, &langs::C);
        assert_eq!(kind_of(&s, "zaehle_zeilen"), Some(Kind::Function));
        assert_eq!(kind_of(&s, "anzahl"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "pfad"), Some(Kind::Variable));
        let go = "func (s *Server) Starte(port int, name string) error {\n\tadresse := fmt.Sprintf(\":%d\", port)\n\treturn nil\n}\n";
        let s = scan(go, &langs::GO);
        assert_eq!(kind_of(&s, "Starte"), Some(Kind::Function));
        assert_eq!(kind_of(&s, "port"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "name"), Some(Kind::Variable));
        assert_eq!(kind_of(&s, "adresse"), Some(Kind::Variable));
    }

    #[test]
    fn ranking() {
        let t = "fn berechne_summe() {}\nfn berechne_steuer() {}\nfn main() {\n    let steuersatz = 1;\n    let gesamt_betrag = 2;\n}\n";
        // Anfang
        let b = top(t, "rs", "bere", false, 4);
        assert!(b[..2].contains(&"berechne_summe".to_string()) && b[..2].contains(&"berechne_steuer".to_string()), "{b:?}");
        // Wortteile: „gb“ → gesamt_betrag
        assert_eq!(top(t, "rs", "gb", false, 4)[0], "gesamt_betrag");
        // Standard der Sprache: „printl“ → println!
        assert!(top(t, "rs", "printl", false, 4).contains(&"println!".to_string()));
        // nach „.“: Methoden der Sprache
        assert!(top(t, "rs", "unw", true, 4).contains(&"unwrap".to_string()));
        // Python-Standard
        assert!(top("x = 1\n", "py", "enum", false, 0).contains(&"enumerate".to_string()));
        // SQL-Schlüsselwort
        assert!(top("", "sql", "sel", false, 0).contains(&"SELECT".to_string()));
    }

    #[test]
    fn member_context() {
        assert!(member_ctx("konto.", 6));
        assert!(member_ctx("a->", 3));
        assert!(member_ctx("Vec::", 5));
        assert!(!member_ctx("3.", 2));
        assert!(!member_ctx("x", 0));
    }

    #[test]
    fn fuzzy() {
        assert_eq!(match_score("berechne_summe", "bere"), Some(100));
        assert_eq!(match_score("getElementById", "gebi"), Some(75));
        assert!(match_score("berechne_summe", "summe").is_some());
        assert!(match_score("berechne_summe", "xyz").is_none());
    }
}
