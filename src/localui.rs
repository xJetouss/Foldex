//! Oberfläche für lokale Modelle: Einstellungsseite (Text- und Bildmodelle – oben das Installierte, darunter
//! Empfehlungen, beim Weiterscrollen lädt die Hugging-Face-Suche automatisch nach) und das Fenster
//! „Bild erzeugen“.

use crate::ai::{Kind, ModelProfile};
use crate::localai::Progress;
use crate::localllm::{self as ll, Fit, HfKind, HfRepo, ImgChoice};
use crate::themes::Theme;
use eframe::egui::{self, vec2, Color32, Stroke, TextureHandle};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Text,
    Image,
}

type Page1 = Result<(Vec<HfRepo>, Option<String>), String>;
type Files = Result<Vec<(String, u64)>, String>;

/// Hugging-Face-Liste eines Reiters: lädt seitenweise nach, Dateigrößen je Repo bei Bedarf
#[derive(Default)]
struct Hf {
    query: String,
    /// Suche, zu der `repos` gehört (None = noch nichts geladen)
    loaded_for: Option<String>,
    repos: Vec<HfRepo>,
    next: Option<String>,
    end: bool,
    loading: Option<mpsc::Receiver<Page1>>,
    sizes: HashMap<String, Vec<(String, u64)>>,
    size_rx: Vec<(String, mpsc::Receiver<Files>)>,
    open: Option<String>,
}

/// Zustand der Einstellungsseite (über Frames hinweg)
struct Page {
    tab: Tab,
    /// nur Modelle zeigen, die auf diesem PC laufen
    only_fit: bool,
    text: Hf,
    image: Hf,
    /// Quelle für weitere Bildmodelle
    civitai: bool,
    civ: Civ,
    msg: Option<(bool, String)>,
}

fn page() -> &'static Mutex<Option<Page>> {
    static P: Mutex<Option<Page>> = Mutex::new(None);
    &P
}

pub fn gb(mb: u64) -> String {
    if mb >= 1000 {
        format!("{:.1} GB", mb as f64 / 1000.0)
    } else {
        format!("{mb} MB")
    }
}

fn fit_color(f: Fit, th: &Theme) -> Color32 {
    match f {
        Fit::Gpu => th.accent,
        Fit::Slow => Color32::from_rgb(220, 160, 40),
        Fit::TooBig => th.danger,
    }
}

fn card(ui: &mut egui::Ui, th: &Theme, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none().fill(th.bg).stroke(Stroke::new(1.0, th.line)).rounding(8.0).inner_margin(egui::Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

/// Name + Beschreibung (+ Passt-Hinweis) links, Knöpfe rechts
fn row(ui: &mut egui::Ui, th: &Theme, title: &str, sub: &str, on: bool, fit: Option<Fit>, right: impl FnOnce(&mut egui::Ui)) {
    // erst die Knöpfe rechts, dann der Text im verbleibenden Platz (gekürzt) – sonst schieben lange Namen
    // die Zeile und damit das ganze Einstellungsfenster in die Breite
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        right(ui);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
        crate::ui::draw(ui.painter(), r, crate::ui::Ic::Spark, if on { th.accent } else { th.sub });
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add(egui::Label::new(egui::RichText::new(title).strong()).truncate());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some(f) = fit {
                    ui.label(egui::RichText::new(format!("● {}", f.label())).size(11.5).color(fit_color(f, th)));
                }
                ui.add(egui::Label::new(egui::RichText::new(sub).size(12.0).color(if on { th.accent } else { th.sub })).truncate());
            });
        });
        });
        });
    });
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text).size(13.5).strong());
    ui.add_space(2.0);
}

fn progress(ui: &mut egui::Ui, th: &Theme, frac: f32, text: &str) {
    if frac >= 0.0 {
        ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_width(140.0).desired_height(6.0));
    } else if frac > -1.5 {
        ui.spinner();
    }
    ui.label(egui::RichText::new(text).size(11.0).color(th.sub));
}

fn primary(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(egui::Button::new(egui::RichText::new(text).strong())).clicked()
}

fn profile_for(stem: &str) -> ModelProfile {
    ModelProfile { id: format!("local-{stem}"), name: format!("{stem} (lokal)"), kind: Kind::OpenAi, url: ll::BASE.into(), model: stem.into() }
}

/// Einstellungen → Lokale Modelle. Gibt true zurück, wenn „Bild erzeugen“ geöffnet werden soll.
pub fn settings_ui(ui: &mut egui::Ui, th: &Theme, profiles: &mut Vec<ModelProfile>) -> bool {
    let mut g = page().lock().unwrap_or_else(|e| e.into_inner());
    // FOLDEX_SHOT_TAB=image: für Bildschirmfotos gleich den Bild-Reiter
    let start = if std::env::var("FOLDEX_SHOT_TAB").as_deref() == Ok("image") { Tab::Image } else { Tab::Text };
    let pg = g.get_or_insert_with(|| Page { tab: start, only_fit: true, text: Hf::default(), image: Hf::default(), civitai: false, civ: Civ::default(), msg: None });
    for (err, m) in ll::take_log() {
        pg.msg = Some((err, m));
    }
    let mut open_gen = false;
    ui.label(egui::RichText::new("Lokale Modelle – statt LM Studio").size(14.5).strong());
    ui.label(egui::RichText::new(format!("Text- und Bildmodelle laden und direkt hier ausführen. Nichts verlässt den PC. Gespeichert in {}.", ll::root().display())).size(12.0).color(th.sub));
    if !ll::supported() {
        ui.label(egui::RichText::new("Bisher nur für Windows (x64)").color(th.danger));
        return false;
    }
    let pc = ll::pc();
    ui.horizontal(|ui| {
        let gpu = if pc.vram_mb > 0 { format!("{} · {} Grafikspeicher", pc.gpu, gb(pc.vram_mb * 1048 / 1000)) } else { "keine eigene Grafikkarte erkannt".into() };
        ui.label(egui::RichText::new(format!("Dieser PC: {gpu} · {} RAM", gb(pc.ram_mb * 1048 / 1000))).size(12.0).color(th.sub));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut pg.only_fit, "Nur passende").on_hover_text("Modelle ausblenden, die für diesen PC zu groß sind oder nicht laufen");
        });
    });
    ui.add_space(4.0);
    if let Some(m) = ll::server_model() {
        card(ui, th, |ui| {
            row(ui, th, &format!("Läuft: {m}"), &format!("Lokaler Server auf {}", ll::BASE), true, None, |ui| {
                if ui.button("Stoppen").on_hover_text("Modell aus dem Speicher nehmen").clicked() {
                    ll::stop_server();
                }
            });
        });
        ui.add_space(4.0);
    }
    downloads_ui(ui, th);
    ui.horizontal(|ui| {
        for (t, l) in [(Tab::Text, "Textmodelle"), (Tab::Image, "Bildmodelle")] {
            if ui.selectable_label(pg.tab == t, egui::RichText::new(l).size(13.5)).clicked() {
                pg.tab = t;
            }
        }
    });
    if let Some((err, m)) = &pg.msg {
        ui.label(egui::RichText::new(m).size(12.0).color(if *err { th.danger } else { th.accent }));
    }
    let only_fit = pg.only_fit;
    match pg.tab {
        Tab::Text => {
            text_tab(ui, th, profiles, only_fit);
            hf_ui(ui, th, &mut pg.text, HfKind::Text, &mut pg.only_fit);
        }
        Tab::Image => {
            open_gen = image_tab(ui, th, only_fit);
            // weitere Bildmodelle: Hugging Face oder Civitai
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Mehr Bildmodelle von").size(13.5).strong());
                for (c, l) in [(false, "Hugging Face"), (true, "Civitai")] {
                    if ui.selectable_label(pg.civitai == c, l).clicked() {
                        pg.civitai = c;
                    }
                }
            });
            if pg.civitai {
                civ_ui(ui, th, &mut pg.civ, &mut pg.only_fit);
            } else {
                hf_ui(ui, th, &mut pg.image, HfKind::Image, &mut pg.only_fit);
            }
        }
    }
    open_gen
}

fn downloads_ui(ui: &mut egui::Ui, th: &Theme) {
    let dls = ll::downloads();
    if dls.is_empty() {
        return;
    }
    card(ui, th, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("Downloads ({})", dls.len())).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Alle abbrechen").on_hover_text("Angefangene Dateien werden beim nächsten Mal fortgesetzt").clicked() {
                    ll::cancel_downloads();
                }
            });
        });
        for (name, f, t) in dls.iter().take(6) {
            ui.add(egui::Label::new(egui::RichText::new(name).size(12.0)).truncate());
            ui.horizontal(|ui| progress(ui, th, *f, t));
        }
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    ui.add_space(4.0);
}

/// Knöpfe für ein installiertes Textmodell: im Chat verfügbar machen, löschen
fn text_buttons(ui: &mut egui::Ui, th: &Theme, stem: &str, path: &Path, profiles: &mut Vec<ModelProfile>) {
    // Dateien von LM Studio bleiben dort – nur eigene lassen sich löschen
    if ll::is_own(path) && ui.button("Löschen").clicked() && ll::delete_file(path).is_ok() {
        profiles.retain(|p| !(ll::is_builtin(&p.url) && p.model == stem));
    }
    let used = profiles.iter().any(|p| ll::is_builtin(&p.url) && p.model == stem);
    if used {
        ui.label(egui::RichText::new("✓ in Chats").color(th.accent)).on_hover_text("Steht unter KI-Chats und als Modell der Explorer-KI zur Auswahl");
    } else if ui.add(egui::Button::new(egui::RichText::new("In Chats nutzen").strong())).on_hover_text("Als Modell für Chat und Explorer-KI hinzufügen").clicked() {
        profiles.push(profile_for(stem));
    }
}

fn text_tab(ui: &mut egui::Ui, th: &Theme, profiles: &mut Vec<ModelProfile>, only_fit: bool) {
    // Programm: bei NVIDIA die CUDA-Version (schneller), sonst Vulkan
    let rt = ll::llama_runtime();
    let rt_busy = ll::downloads().iter().any(|d| d.0.starts_with(rt.name));
    if ll::exe(rt).is_none() && !rt_busy {
        let (title, sub) = if ll::incomplete(rt) {
            (format!("{} – nicht fertig geladen", rt.name), "Die CUDA-Laufzeit fehlt noch (Download wurde unterbrochen). „Installieren“ setzt dort fort; bis dahin läuft die Vulkan-Version.".to_string())
        } else if ll::llama_exe().is_none() {
            (rt.name.to_string(), format!("Programm zum Ausführen – wird mit dem ersten Modell geladen ({} MB)", rt.mb))
        } else {
            ("NVIDIA-Grafikkarte erkannt".to_string(), format!("{} nutzt sie direkt und ist deutlich schneller als die Vulkan-Version ({} MB)", rt.name, rt.mb))
        };
        ui.add_space(4.0);
        card(ui, th, |ui| {
            row(ui, th, &title, &sub, false, None, |ui| {
                if primary(ui, "Installieren") {
                    ll::install_runtime(rt);
                }
            });
        });
    }
    let installed = ll::installed_text();
    heading(ui, &format!("Installiert ({})", installed.len()));
    if installed.is_empty() {
        ui.label(egui::RichText::new("Noch keins – unten eins auswählen.").size(12.0).color(th.sub));
    }
    for (stem, path, size) in &installed {
        let name = ll::TEXT.iter().find(|m| ll::text_path(m).file_stem().is_some_and(|s| s.to_string_lossy() == *stem)).map_or(stem.as_str(), |m| m.name);
        let sub = format!("{}{}", gb(size / 1_000_000), if ll::is_own(path) { "" } else { " · aus LM Studio" });
        card(ui, th, |ui| row(ui, th, name, &sub, true, Some(ll::fit_text(size / 1_000_000)), |ui| text_buttons(ui, th, stem, path, profiles)));
    }
    // Spracheingabe (Mikrofon neben „Senden“ in den Chats)
    heading(ui, "Spracheingabe (Mikrofon im Chat)");
    card(ui, th, |ui| crate::stt::model_rows(ui, th));
    heading(ui, "Empfohlen");
    for m in ll::TEXT {
        let stem = ll::text_path(m).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if installed.iter().any(|t| t.0 == stem) {
            continue;
        }
        let fit = ll::fit_text(m.file.mb as u64);
        if only_fit && fit == Fit::TooBig {
            continue;
        }
        let path = ll::text_path(m);
        card(ui, th, |ui| {
            row(ui, th, m.name, &format!("{} · {}", m.desc, gb(m.file.mb as u64)), false, Some(fit), |ui| {
                if let Some((f, t)) = ll::queued(&path) {
                    progress(ui, th, f, &t);
                } else if primary(ui, &format!("Laden ({})", gb(m.file.mb as u64))) {
                    ll::install_text(m);
                }
            });
        });
    }
}

fn image_tab(ui: &mut egui::Ui, th: &Theme, only_fit: bool) -> bool {
    let mut open = false;
    let own = ll::installed_image_files();
    let cat_on: Vec<&ll::ImageModel> = ll::IMAGE.iter().filter(|m| ll::image_installed(m)).collect();
    // Programm: bei NVIDIA die CUDA-Version (viel schneller), sonst Vulkan
    let rt = ll::sd_runtime();
    let busy = ll::downloads().iter().any(|d| d.0.starts_with(rt.name));
    if ll::exe(rt).is_none() && !busy {
        let (title, sub) = if ll::incomplete(rt) {
            (format!("{} – nicht fertig geladen", rt.name), "Die CUDA-Laufzeit fehlt noch (Download wurde unterbrochen). „Installieren“ setzt dort fort; bis dahin läuft die Vulkan-Version.".to_string())
        } else if ll::sd_exe().is_none() {
            (rt.name.to_string(), format!("Programm zum Erzeugen – wird mit dem ersten Modell geladen ({})", gb(rt.mb as u64)))
        } else {
            ("NVIDIA-Grafikkarte erkannt".to_string(), format!("{} rechnet Bilder auf ihr – ein Vielfaches schneller als die Vulkan-Version ({})", rt.name, gb(rt.mb as u64)))
        };
        ui.add_space(4.0);
        card(ui, th, |ui| {
            row(ui, th, &title, &sub, false, None, |ui| {
                if primary(ui, "Installieren") {
                    ll::install_runtime(rt);
                }
            });
        });
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("Installiert ({})", cat_on.len() + own.len())).size(13.5).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(!cat_on.is_empty() || !own.is_empty(), egui::Button::new(egui::RichText::new("Bild erzeugen …").strong())).on_disabled_hover_text("Erst ein Bildmodell laden").clicked() {
                open = true;
            }
        });
    });
    if cat_on.is_empty() && own.is_empty() {
        ui.label(egui::RichText::new("Noch keins – unten eins auswählen. Bilder erzeugen geht dann auch über KI → „Bild erzeugen“.").size(12.0).color(th.sub));
    }
    for m in &cat_on {
        card(ui, th, |ui| {
            row(ui, th, m.name, &format!("{} · {}", m.desc, gb(m.files_mb() as u64)), true, Some(ll::fit_image(m.largest_mb() as u64, m.files_mb() as u64)), |ui| {
                if ui.button("Löschen").on_hover_text("Bildmodell löschen (mitbenutzte Dateien bleiben)").clicked() {
                    let _ = ll::delete_image(m);
                }
            });
        });
    }
    for (stem, path, size) in &own {
        let mb = size / 1_000_000;
        card(ui, th, |ui| {
            row(ui, th, stem, &format!("Eigenes Modell · {} · {}{}", ll::family_of(path).label(), gb(mb), extra_note(ll::family_of(path))), true, Some(ll::fit_image(mb, mb)), |ui| {
                if ui.button("Löschen").clicked() {
                    let _ = ll::delete_file(path);
                }
            });
        });
    }
    // Bearbeitungsmodelle für den Bildeditor („schreib, was sich ändern soll“)
    heading(ui, "Bilder per Text bearbeiten (Bildeditor)");
    for m in ll::EDIT {
        let fit = m.fit();
        card(ui, th, |ui| {
            row(ui, th, m.name, &format!("{} · {}", m.desc, gb(m.total_mb() as u64)), m.installed(), Some(fit), |ui| {
                if m.installed() {
                    if ui.button("Löschen").on_hover_text("Bearbeitungsmodell löschen (mitbenutzte Zusatzdateien bleiben)").clicked() {
                        let _ = ll::delete_edit(m);
                    }
                } else if let Some((f, t)) = m.progress() {
                    progress(ui, th, f, &t);
                } else if primary(ui, &format!("Laden ({})", gb(m.missing_mb() as u64))) {
                    ll::install_edit(m);
                }
            });
        });
    }
    heading(ui, "Empfohlen");
    for m in ll::IMAGE.iter().filter(|m| !ll::image_installed(m)) {
        let fit = ll::fit_image(m.largest_mb() as u64, m.files_mb() as u64);
        if only_fit && fit == Fit::TooBig {
            continue;
        }
        let miss = ll::image_missing(m);
        let need: u32 = miss.iter().map(|x| x.3).sum();
        card(ui, th, |ui| {
            row(ui, th, m.name, &format!("{} · {}", m.desc, gb(m.files_mb() as u64)), false, Some(fit), |ui| {
                if let Some((f, t)) = miss.iter().find_map(|x| ll::queued(&x.1)) {
                    progress(ui, th, f, &t);
                } else if primary(ui, &format!("Laden ({})", gb(need as u64))) {
                    ll::install_image(m);
                }
            });
        });
    }
    open
}

/// „Mehr auf Hugging Face“: Suchfeld und eine Liste, die beim Herunterscrollen von selbst nachlädt
fn hf_ui(ui: &mut egui::Ui, th: &Theme, hf: &mut Hf, kind: HfKind, only_fit_flag: &mut bool) {
    let only_fit = *only_fit_flag;
    if kind == HfKind::Text {
        heading(ui, "Mehr auf Hugging Face");
    }
    ui.horizontal(|ui| {
        let hint = if kind == HfKind::Text { "Filtern, z. B. qwen, llama, mistral, coder …" } else { "Filtern, z. B. anime, photo, xl, pixel …" };
        let r = ui.add(egui::TextEdit::singleline(&mut hf.query).hint_text(hint).desired_width((ui.available_width() - 90.0).max(120.0)));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Suchen").clicked() || enter {
            hf.loaded_for = None;
        }
    });
    ui.label(
        egui::RichText::new(if kind == HfKind::Text {
            "GGUF-Modelle, nach Downloads sortiert. „Laden“ nimmt die empfohlene Stufe (Q4_K_M); andere unter „Dateien“."
        } else {
            "Bildmodelle als Einzeldatei (Stable Diffusion 1.5/2.x/SDXL), nach Downloads sortiert. Modelle aus mehreren Teilen (z. B. Diffusers-Ordner) laufen nicht und werden mit „Nur passende“ ausgeblendet."
        })
        .size(11.5)
        .color(th.sub),
    );
    // neue Suche: Liste leeren und von vorn laden
    if hf.loaded_for.as_deref() != Some(hf.query.as_str()) && hf.loading.is_none() {
        hf.loaded_for = Some(hf.query.clone());
        hf.repos.clear();
        hf.next = None;
        hf.end = false;
        hf.open = None;
        start_page(ui.ctx(), hf, kind, None);
    }
    // fertige Seiten und Dateigrößen abholen
    if let Some(rx) = &hf.loading {
        if let Ok(r) = rx.try_recv() {
            hf.loading = None;
            match r {
                Ok((v, next)) => {
                    hf.end = next.is_none() || v.is_empty();
                    hf.next = next;
                    hf.repos.extend(v);
                }
                Err(_) => hf.end = true,
            }
        }
    }
    hf.size_rx.retain(|(id, rx)| match rx.try_recv() {
        Ok(r) => {
            hf.sizes.insert(id.clone(), r.unwrap_or_default());
            false
        }
        Err(mpsc::TryRecvError::Empty) => true,
        Err(_) => false,
    });
    let mut toggle: Option<String> = None;
    let mut want_sizes: Vec<String> = Vec::new();
    let (mut shown, mut hidden) = (0, 0);
    for r in &hf.repos {
        // Bildmodelle: Bauart bestimmt, welche Dateien das Modell sind und was mitgeladen wird
        let (fam, files): (Option<ll::Family>, Vec<String>) = match kind {
            HfKind::Text => (None, ll::best_gguf(r).into_iter().collect()),
            HfKind::Image => ll::hf_image(r).map_or((None, Vec::new()), |(f, v)| (Some(f), v)),
        };
        let usable = !files.is_empty();
        let sizes = hf.sizes.get(&r.id);
        // vorgeschlagene Datei und ihre Größe
        let pick = match (kind, fam) {
            (HfKind::Image, Some(f)) => sizes
                .and_then(|s| ll::pick_model_file(&s.iter().filter(|x| files.contains(&x.0)).cloned().collect::<Vec<_>>(), f))
                .or_else(|| ll::pick_model_file(&files.iter().map(|x| (x.clone(), 0)).collect::<Vec<_>>(), f)),
            _ => files.first().cloned(),
        };
        let mb = pick.as_ref().and_then(|p| sizes.and_then(|s| s.iter().find(|f| f.0 == *p)).map(|f| f.1 / 1_000_000));
        let extra = fam.map_or(0, |f| f.companions_mb() as u64);
        let fit = mb.map(|mb| if kind == HfKind::Text { ll::fit_text(mb) } else { ll::fit_image(mb, mb + extra) });
        if only_fit && (!usable || fit == Some(Fit::TooBig)) {
            hidden += 1;
            continue;
        }
        shown += 1;
        let is_open = hf.open.as_deref() == Some(r.id.as_str());
        let resp = egui::Frame::none().fill(th.bg).stroke(Stroke::new(1.0, th.line)).rounding(8.0).inner_margin(egui::Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut sub = format!("{} Downloads · {} Likes", short(r.downloads), short(r.likes));
            if let (Some(p), Some(mb)) = (&pick, mb) {
                sub = format!("{} · {} · {sub}", p.rsplit('/').next().unwrap_or(p), gb(mb));
            } else if !usable {
                sub = format!("läuft hier nicht (z. B. Video-, Bearbeitungs- oder Zusatzmodell) · {sub}");
            }
            if let Some(f) = fam {
                sub = format!("{}{} · {sub}", f.label(), extra_note(f));
            }
            row(ui, th, &r.id, &sub, false, fit, |ui| {
                if usable && ui.button(if is_open { "Zuklappen" } else { "Dateien" }).clicked() {
                    toggle = Some(r.id.clone());
                }
                if let Some(p) = &pick {
                    let dest = ll::hf_dest(p, kind == HfKind::Text);
                    if dest.is_file() {
                        ui.label(egui::RichText::new("✓ geladen").color(th.accent));
                    } else if let Some((f, t)) = ll::queued(&dest) {
                        progress(ui, th, f, &t);
                    } else if primary(ui, &mb.map_or_else(|| "Laden".to_string(), |mb| format!("Laden ({})", gb(mb)))) {
                        install(kind, &r.id, p, &r.files, fam);
                    }
                }
            });
            if is_open {
                match sizes {
                    None => {
                        ui.spinner();
                    }
                    Some(list) => {
                        for (f, size) in list {
                            let mb = size / 1_000_000;
                            let fit = if kind == HfKind::Text { ll::fit_text(mb) } else { ll::fit_image(mb, mb) };
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let dest = ll::hf_dest(f, kind == HfKind::Text);
                                    if dest.is_file() {
                                        ui.label(egui::RichText::new("✓ geladen").color(th.accent));
                                    } else if let Some((fr, t)) = ll::queued(&dest) {
                                        progress(ui, th, fr, &t);
                                    } else if ui.small_button("Laden").clicked() {
                                        install(kind, &r.id, f, &r.files, fam);
                                    }
                                    // Name gekürzt im Rest der Zeile
                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                        ui.add_space(26.0);
                                        ui.label(egui::RichText::new(format!("{} · {}", gb(mb), fit.label())).size(11.5).color(fit_color(fit, th)));
                                        ui.add(egui::Label::new(egui::RichText::new(f.rsplit('/').next().unwrap_or(f)).size(12.5)).truncate());
                                    });
                                });
                            });
                        }
                    }
                }
            }
        });
        // Größen nur für sichtbare Einträge holen
        if usable && sizes.is_none() && ui.is_rect_visible(resp.response.rect) {
            want_sizes.push(r.id.clone());
        }
    }
    for id in want_sizes {
        if hf.size_rx.len() >= 6 || hf.size_rx.iter().any(|(i, _)| *i == id) {
            continue;
        }
        let (tx, rx) = mpsc::channel();
        let (rid, ctx) = (id.clone(), ui.ctx().clone());
        std::thread::spawn(move || {
            let _ = tx.send(ll::hf_files(&rid, kind));
            ctx.request_repaint();
        });
        hf.size_rx.push((id, rx));
    }
    if !hf.size_rx.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    if let Some(id) = toggle {
        hf.open = if hf.open.as_deref() == Some(id.as_str()) { None } else { Some(id) };
    }
    hidden_hint(ui, th, hidden, only_fit_flag);
    // Ende der Liste sichtbar → nächste Seite (automatisch beim Scrollen)
    let (end_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), egui::Sense::hover());
    if hf.loading.is_some() {
        ui.put(end_rect, egui::Spinner::new());
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
    } else if hf.end {
        ui.put(end_rect, egui::Label::new(egui::RichText::new(if shown == 0 { "Keine passenden Modelle gefunden" } else { "Ende der Liste" }).size(11.5).color(th.sub)));
    } else if ui.is_rect_visible(end_rect) {
        let next = hf.next.clone();
        start_page(ui.ctx(), hf, kind, next);
    }
}

fn start_page(ctx: &egui::Context, hf: &mut Hf, kind: HfKind, next: Option<String>) {
    let (tx, rx) = mpsc::channel();
    let (q, ctx) = (hf.query.clone(), ctx.clone());
    std::thread::spawn(move || {
        let _ = tx.send(ll::hf_page(kind, &q, next.as_deref()));
        ctx.request_repaint();
    });
    hf.loading = Some(rx);
}

fn install(kind: HfKind, repo: &str, file: &str, all: &[String], fam: Option<ll::Family>) {
    match kind {
        HfKind::Text => ll::install_hf(repo, file, all),
        HfKind::Image => ll::install_hf_image(repo, file, fam.unwrap_or(ll::Family::Sd)),
    }
}

/// „ + 3.9 GB Zusatz“: was für diese Bauart noch mitgeladen wird
fn extra_note(f: ll::Family) -> String {
    let mb: u32 = f.missing().iter().map(|m| m.3).sum();
    if mb == 0 {
        String::new()
    } else {
        format!(" (+ {} Zusatzdateien)", gb(mb as u64))
    }
}

fn short(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} Mio.", n as f64 / 1e6)
    } else if n >= 1000 {
        format!("{:.0} Tsd.", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

/// „N Treffer ausgeblendet“ mit Knopf zum Anzeigen – sonst wirkt eine gefilterte Suche einfach leer
fn hidden_hint(ui: &mut egui::Ui, th: &Theme, hidden: usize, only_fit: &mut bool) {
    if hidden == 0 {
        return;
    }
    ui.horizontal(|ui| {
        if ui.small_button("Alle zeigen").clicked() {
            *only_fit = false;
        }
        ui.add(egui::Label::new(egui::RichText::new(format!("{hidden} Treffer ausgeblendet – zu groß für diesen PC oder hier nicht lauffähig")).size(12.0).color(th.sub)).truncate());
    });
}

type CivPage = Result<(Vec<ll::CivModel>, Option<String>), String>;

/// Civitai-Liste (lädt beim Scrollen nach) und der Schlüssel zum Herunterladen
#[derive(Default)]
struct Civ {
    query: String,
    loaded_for: Option<String>,
    items: Vec<ll::CivModel>,
    next: Option<String>,
    end: bool,
    err: Option<String>,
    loading: Option<mpsc::Receiver<CivPage>>,
    key_input: String,
    /// Schlüssel einmal aus dem Speicher gelesen
    key: Option<Option<String>>,
}

fn civ_ui(ui: &mut egui::Ui, th: &Theme, cv: &mut Civ, only_fit_flag: &mut bool) {
    let only_fit = *only_fit_flag;
    let key = cv.key.get_or_insert_with(|| crate::ai::load_key(ll::CIVITAI_KEY)).clone();
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut cv.query).hint_text("Filtern, z. B. realistic, anime, pony, juggernaut …").desired_width((ui.available_width() - 90.0).max(120.0)));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Suchen").clicked() || enter {
            cv.loaded_for = None;
        }
    });
    ui.label(egui::RichText::new("Die größte Sammlung von Stable-Diffusion-Modellen (SD 1.5, SDXL, Pony, Illustrious), ohne Erwachseneninhalte, nach Downloads sortiert.").size(11.5).color(th.sub));
    // Civitai lässt nur mit (kostenlosem) Schlüssel herunterladen
    card(ui, th, |ui| match &key {
        Some(_) => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("✓ Civitai-Schlüssel hinterlegt (verschlüsselt gespeichert)").size(12.0).color(th.accent));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Entfernen").clicked() {
                        crate::ai::delete_key(ll::CIVITAI_KEY);
                        cv.key = None;
                    }
                });
            });
        }
        None => {
            ui.label(egui::RichText::new("Zum Herunterladen verlangt Civitai einen kostenlosen API-Schlüssel: civitai.com → Konto → Einstellungen → API Keys.").size(12.0));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut cv.key_input).password(true).hint_text("API-Schlüssel einfügen").desired_width((ui.available_width() - 190.0).max(120.0)));
                if ui.add_enabled(!cv.key_input.trim().is_empty(), egui::Button::new("Speichern")).clicked() && crate::ai::save_key(ll::CIVITAI_KEY, cv.key_input.trim()).is_ok() {
                    cv.key_input.clear();
                    cv.key = None;
                }
                if ui.small_button("Seite öffnen").clicked() {
                    let _ = open::that("https://civitai.com/user/account");
                }
            });
        }
    });
    if cv.loaded_for.as_deref() != Some(cv.query.as_str()) && cv.loading.is_none() {
        cv.loaded_for = Some(cv.query.clone());
        cv.items.clear();
        cv.next = None;
        cv.end = false;
        cv.err = None;
        civ_page(ui.ctx(), cv, None);
    }
    if let Some(rx) = &cv.loading {
        if let Ok(r) = rx.try_recv() {
            cv.loading = None;
            match r {
                Ok((v, next)) => {
                    cv.end = next.is_none() || v.is_empty();
                    cv.next = next;
                    cv.items.extend(v);
                }
                Err(e) => {
                    cv.err = Some(e);
                    cv.end = true;
                }
            }
        }
    }
    let (mut shown, mut hidden) = (0, 0);
    for m in &cv.items {
        let fit = ll::fit_image(m.mb, m.mb + m.family.map_or(0, |f| f.companions_mb() as u64));
        if only_fit && (!m.usable() || fit == Fit::TooBig) {
            hidden += 1;
            continue;
        }
        shown += 1;
        let sub = match (m.usable(), m.family) {
            (true, Some(f)) => format!("{} · {}{} · von {} · {} Downloads", m.base, gb(m.mb), extra_note(f), m.creator, short(m.downloads)),
            _ => format!("{} – läuft hier nicht (Video- oder Inpainting-Modell) · von {}", m.base, m.creator),
        };
        card(ui, th, |ui| {
            row(ui, th, &m.name, &sub, false, m.usable().then_some(fit), |ui| {
                if ui.small_button("Seite").on_hover_text("Auf civitai.com ansehen").clicked() {
                    let _ = open::that(format!("https://civitai.com/models/{}", m.id));
                }
                if !m.usable() {
                    return;
                }
                let dest = ll::civitai_dest(m);
                if dest.is_file() {
                    ui.label(egui::RichText::new("✓ geladen").color(th.accent));
                } else if let Some((f, t)) = ll::queued(&dest) {
                    progress(ui, th, f, &t);
                } else if let Some(k) = &key {
                    if primary(ui, &format!("Laden ({})", gb(m.mb))) {
                        ll::install_civitai(m, k);
                    }
                } else {
                    ui.add_enabled(false, egui::Button::new(format!("Laden ({})", gb(m.mb)))).on_disabled_hover_text("Erst oben den Civitai-Schlüssel eintragen");
                }
            });
        });
    }
    hidden_hint(ui, th, hidden, only_fit_flag);
    if let Some(e) = &cv.err {
        ui.label(egui::RichText::new(e).size(12.0).color(th.danger));
    }
    let (end_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), egui::Sense::hover());
    if cv.loading.is_some() {
        ui.put(end_rect, egui::Spinner::new());
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
    } else if cv.end {
        ui.put(end_rect, egui::Label::new(egui::RichText::new(if shown == 0 { "Keine passenden Modelle gefunden" } else { "Ende der Liste" }).size(11.5).color(th.sub)));
    } else if ui.is_rect_visible(end_rect) {
        let next = cv.next.clone();
        civ_page(ui.ctx(), cv, next);
    }
}

fn civ_page(ctx: &egui::Context, cv: &mut Civ, next: Option<String>) {
    let (tx, rx) = mpsc::channel();
    let (q, ctx) = (cv.query.clone(), ctx.clone());
    std::thread::spawn(move || {
        let _ = tx.send(ll::civitai_page(&q, next.as_deref()));
        ctx.request_repaint();
    });
    cv.loading = Some(rx);
}

// ───────────────────────── Fenster „Bild erzeugen“ ─────────────────────────

pub struct ImageGen {
    /// Zielordner (aktueller Ordner beim Öffnen)
    dir: PathBuf,
    model: ImgChoice,
    prompt: String,
    negative: String,
    w: u32,
    h: u32,
    steps: u32,
    cfg: f32,
    seed: i64,
    random_seed: bool,
    count: u32,
    job: Option<(Progress, mpsc::Receiver<Result<Vec<PathBuf>, String>>)>,
    results: Vec<(PathBuf, Option<TextureHandle>)>,
    msg: Option<(bool, String)>,
}

impl ImageGen {
    pub fn new(dir: PathBuf) -> ImageGen {
        let model = match ll::IMAGE.iter().find(|m| ll::image_installed(m)) {
            Some(m) => ImgChoice::Catalog(m.id),
            None => ll::installed_image_files().into_iter().next().map_or(ImgChoice::Catalog(ll::IMAGE[0].id), |f| ImgChoice::File(f.1)),
        };
        let mut g = ImageGen { dir, model, prompt: String::new(), negative: String::new(), w: 0, h: 0, steps: 0, cfg: 1.0, seed: 42, random_seed: true, count: 1, job: None, results: Vec::new(), msg: None };
        g.defaults();
        g
    }

    /// (Größe, Schritte, CFG, negativer Prompt sinnvoll)
    fn settings(&self) -> (u32, u32, f32, bool) {
        match &self.model {
            ImgChoice::Catalog(id) => ll::image_model(id).map_or((512, 20, 7.0, true), |m| (m.size, m.steps, m.cfg, m.negative)),
            ImgChoice::File(p) => {
                let (s, st, c) = ll::file_defaults(p);
                (s, st, c, true)
            }
        }
    }

    fn defaults(&mut self) {
        let (s, st, c, _) = self.settings();
        self.w = s;
        self.h = s;
        self.steps = st;
        self.cfg = c;
    }

    fn start(&mut self, ctx: &egui::Context) {
        if self.random_seed {
            self.seed = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) % 2_147_483_647) as i64;
        }
        let req = ll::GenReq { model: self.model.clone(), prompt: self.prompt.clone(), negative: self.negative.clone(), w: self.w, h: self.h, steps: self.steps, cfg: self.cfg, seed: self.seed, count: self.count };
        let (p, (tx, rx)) = (Progress::default(), mpsc::channel());
        let (pp, dir, ctx) = (p.clone(), self.dir.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(ll::generate(&req, &dir, &pp));
            ctx.request_repaint();
        });
        self.job = Some((p, rx));
        self.msg = None;
    }

    /// Fenster zeichnen; false = geschlossen
    pub fn ui(&mut self, ctx: &egui::Context, th: &Theme) -> bool {
        let mut open = true;
        if let Some((_, rx)) = &self.job {
            if let Ok(r) = rx.try_recv() {
                self.job = None;
                match r {
                    Ok(files) => {
                        self.msg = Some((false, format!("{} Bild(er) gespeichert in {}", files.len(), self.dir.display())));
                        for f in files.into_iter().rev() {
                            self.results.insert(0, (f, None));
                        }
                        self.results.truncate(12);
                    }
                    Err(e) => self.msg = Some((true, e)),
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
        egui::Window::new("Bild erzeugen (lokal)").open(&mut open).default_size(vec2(760.0, 560.0)).resizable(true).collapsible(false).show(ctx, |ui| {
            // Auswahl: Katalogmodelle und eigene Dateien (Name, Wahl, Hinweis)
            let mut installed: Vec<(String, ImgChoice, String)> = ll::IMAGE.iter().filter(|m| ll::image_installed(m)).map(|m| (m.name.to_string(), ImgChoice::Catalog(m.id), m.desc.to_string())).collect();
            installed.extend(ll::installed_image_files().into_iter().map(|(stem, p, _)| (stem, ImgChoice::File(p), "Eigenes Modell".to_string())));
            if installed.is_empty() {
                ui.label("Noch kein Bildmodell installiert.");
                let m = &ll::IMAGE[0];
                let miss = ll::image_missing(m);
                if let Some((f, t)) = miss.iter().find_map(|x| ll::queued(&x.1)) {
                    ui.horizontal(|ui| progress(ui, th, f, &t));
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                } else if ui.add(egui::Button::new(egui::RichText::new(format!("{} laden ({})", m.name, gb(m.files_mb() as u64))).strong())).on_hover_text(m.desc).clicked() {
                    ll::install_image(m);
                }
                ui.label(egui::RichText::new("Weitere Modelle (FLUX, SDXL, DreamShaper …): Einstellungen → Lokale Modelle → Bildmodelle").size(11.5).color(th.sub));
                return;
            }
            let busy = self.job.is_some();
            ui.horizontal(|ui| {
                ui.label("Modell");
                let cur = installed.iter().find(|x| x.1 == self.model).map_or("", |x| x.0.as_str()).to_string();
                egui::ComboBox::from_id_salt("imggen_model").selected_text(cur).width(220.0).show_ui(ui, |ui| {
                    for (name, choice, tip) in &installed {
                        if ui.selectable_value(&mut self.model, choice.clone(), name).on_hover_text(tip).clicked() {
                            self.defaults();
                        }
                    }
                });
                ui.label(egui::RichText::new(format!("Speichert in {}", self.dir.display())).size(11.5).color(th.sub));
            });
            ui.add(egui::TextEdit::multiline(&mut self.prompt).desired_rows(3).desired_width(f32::INFINITY).hint_text("Beschreibung (Englisch klappt meist am besten), z. B. „a cozy cabin in snowy mountains at dusk, warm light, photo“"));
            if self.settings().3 {
                ui.add(egui::TextEdit::singleline(&mut self.negative).desired_width(f32::INFINITY).hint_text("Nicht im Bild (optional)"));
            }
            ui.horizontal_wrapped(|ui| {
                let base = self.settings().0;
                let b2 = base * 3 / 4 / 64 * 64;
                for (l, w, h) in [("Quadrat", base, base), ("Quer", base, b2), ("Hoch", b2, base)] {
                    if ui.selectable_label(self.w == w && self.h == h, format!("{l} {w}×{h}")).clicked() {
                        self.w = w;
                        self.h = h;
                    }
                }
                ui.separator();
                ui.add(egui::DragValue::new(&mut self.steps).range(1..=50).prefix("Schritte "));
                ui.add(egui::DragValue::new(&mut self.cfg).range(1.0..=12.0).speed(0.1).prefix("CFG "));
                ui.add(egui::DragValue::new(&mut self.count).range(1..=8).prefix("Anzahl "));
                ui.checkbox(&mut self.random_seed, "Zufällig");
                if !self.random_seed {
                    ui.add(egui::DragValue::new(&mut self.seed).prefix("Seed "));
                }
            });
            ui.horizontal(|ui| {
                let can = !busy && !self.prompt.trim().is_empty();
                let go = ui.add_enabled(can, egui::Button::new(egui::RichText::new("Erzeugen").strong()).min_size(vec2(110.0, 30.0))).clicked()
                    || (can && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)));
                if go {
                    self.start(ui.ctx());
                }
                if let Some((p, _)) = &self.job {
                    let (f, t) = p.get();
                    progress(ui, th, f, if t.is_empty() { "Startet …" } else { &t });
                    if ui.button("Abbrechen").clicked() {
                        p.cancel();
                    }
                } else if let Some((err, m)) = &self.msg {
                    ui.label(egui::RichText::new(m).size(12.0).color(if *err { th.danger } else { th.accent }));
                }
            });
            ui.separator();
            // Ergebnisse als Vorschaubilder; Klick öffnet das Bild
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (path, tex) in &mut self.results {
                        if tex.is_none() {
                            if let Ok(i) = image::open(&*path) {
                                let t = i.thumbnail(320, 320).to_rgba8();
                                *tex = Some(ui.ctx().load_texture(path.display().to_string(), egui::ColorImage::from_rgba_unmultiplied([t.width() as usize, t.height() as usize], t.as_raw()), egui::TextureOptions::LINEAR));
                            }
                        }
                        if let Some(t) = tex {
                            let size = t.size_vec2() * (220.0 / t.size_vec2().max_elem());
                            let r = ui.add(egui::Image::new((t.id(), size)).sense(egui::Sense::click())).on_hover_text(format!("{}\nKlick: öffnen", path.display()));
                            if r.clicked() {
                                let _ = open::that(&*path);
                            }
                        }
                    }
                });
            });
        });
        open
    }
}
