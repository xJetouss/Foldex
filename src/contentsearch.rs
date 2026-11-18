//! Inhaltssuche: „finde die Rechnung von Media Markt“ durchsucht PDFs, Bilder (Texterkennung) und Dokumente im
//! Ordner und erklärt je Treffer, warum er passt (gefundene Begriffe, Stelle im Text, Dateiname, Datum).
//! Erkannter Text wird zwischengespeichert (Pfad + Größe + Änderungszeit), die zweite Suche ist sofort fertig.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

/// Füllwörter, die in der Anfrage nichts bedeuten
const STOP: &[&str] = &[
    "finde", "finden", "find", "such", "suche", "suchen", "zeig", "zeige", "zeigen", "wo", "ist", "sind", "die", "der", "das", "den", "dem", "des", "ein", "eine", "einen", "einer", "von", "vom", "mit",
    "für", "fuer", "im", "in", "am", "an", "auf", "aus", "bei", "zu", "zum", "zur", "und", "oder", "mein", "meine", "meinen", "meiner", "alle", "alles", "bitte", "mir", "ich", "mal", "the", "of", "my", "a",
    "datei", "dateien", "dokument", "dokumente", "irgendwo", "hier", "welche", "welcher", "wann",
];

/// verwandte Begriffe (Dokumentarten): ein Treffer auf eines davon zählt halb
const SYNONYMS: &[(&str, &[&str])] = &[
    ("rechnung", &["invoice", "rechnungsnummer", "rechnungsbetrag", "beleg", "quittung", "kassenbon", "bon", "receipt"]),
    ("vertrag", &["contract", "vereinbarung", "laufzeit", "kündigung"]),
    ("kündigung", &["kuendigung", "gekündigt", "vertragsende"]),
    ("lieferschein", &["lieferung", "delivery"]),
    ("garantie", &["gewährleistung", "warranty"]),
    ("versicherung", &["police", "versicherungsschein", "insurance"]),
    ("lohn", &["gehalt", "entgelt", "lohnabrechnung", "verdienstabrechnung", "payslip"]),
    ("steuer", &["finanzamt", "steuerbescheid", "einkommensteuer", "tax"]),
    ("ausweis", &["personalausweis", "reisepass", "passport"]),
    ("zeugnis", &["arbeitszeugnis", "zertifikat", "certificate"]),
];

pub struct Hit {
    pub path: PathBuf,
    pub score: f32,
    /// Begründung in einem Satz
    pub why: String,
    /// Textstelle mit dem besten Begriff
    pub snippet: String,
}

#[derive(Default)]
pub struct Search {
    pub hits: Arc<Mutex<Vec<Hit>>>,
    pub scanned: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    pub done: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
}

fn norm(s: &str) -> String {
    s.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss")
}

/// Suchbegriffe: Wörter ohne Füllwörter; zusätzlich zusammengeschriebene Paare („media markt“ → „mediamarkt“)
pub fn terms(q: &str) -> Vec<String> {
    let words: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '.')
        .map(|w| w.trim_matches(['.', '-']).to_lowercase())
        .filter(|w| w.chars().count() >= 2 && !STOP.contains(&w.as_str()))
        .collect();
    words.iter().map(|w| norm(w)).collect()
}

fn cache_file() -> PathBuf {
    dirs::cache_dir().or_else(dirs::config_dir).unwrap_or_else(std::env::temp_dir).join("foldex").join("inhalt-cache.json")
}

type Cache = HashMap<String, (u64, i64, String)>;

fn load_cache() -> Cache {
    std::fs::read(cache_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_cache(c: &Cache) {
    let f = cache_file();
    let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
    // höchstens ~20 000 Einträge, Text je Datei schon gekürzt
    if let Ok(b) = serde_json::to_vec(c) {
        let _ = std::fs::write(f, b);
    }
}

/// Text einer Datei (Dokumente direkt, PDFs mit Textebene oder per Texterkennung, Bilder per Texterkennung)
fn extract(p: &Path) -> Option<String> {
    let ext = crate::convert::ext_of(p);
    let t = match ext.as_str() {
        "pdf" => pdf_text(p),
        "docx" | "odt" | "rtf" | "epub" | "xlsx" | "pptx" | "odp" => crate::quicklook::office_markdown(p, &ext),
        "txt" | "md" | "csv" | "json" | "html" | "htm" | "xml" | "eml" | "log" => std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b[..b.len().min(2 << 20)]).into_owned()),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "jfif" => {
            let img = crate::convert::load_image(p, 0).ok()?;
            // Texterkennung braucht keine riesigen Bilder
            let img = if img.width().max(img.height()) > 2600 { img.thumbnail(2600, 2600) } else { img };
            crate::ocr::recognize(&img).ok().map(|w| crate::ocr::all_text(&w))
        }
        _ => None,
    }?;
    Some(t.chars().take(60_000).collect())
}

fn pdf_text(p: &Path) -> Option<String> {
    if let Some(pt) = crate::docconv::pdftotext() {
        let mut c = std::process::Command::new(pt);
        c.args(["-l", "8", "-enc", "UTF-8"]).arg(p).arg("-");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        if let Ok(o) = c.output() {
            let t = String::from_utf8_lossy(&o.stdout).into_owned();
            if t.trim().len() > 40 {
                return Some(t);
            }
        }
    }
    // gescanntes PDF: die ersten Seiten erkennen (Windows rendert die Seiten selbst)
    #[cfg(not(windows))]
    return None;
    #[cfg(windows)]
    let mut out = String::new();
    #[cfg(windows)]
    let _ = crate::pdfview::pdf_pages(p, 1600, |i| i < 3, |_, _, img| {
        if let Ok(w) = crate::ocr::recognize(&image::DynamicImage::ImageRgba8(img)) {
            out.push_str(&crate::ocr::all_text(&w));
            out.push('\n');
        }
        true
    });
    #[cfg(windows)]
    return (!out.trim().is_empty()).then_some(out);
}

fn searchable(p: &Path) -> bool {
    matches!(
        crate::convert::ext_of(p).as_str(),
        "pdf" | "docx" | "odt" | "rtf" | "epub" | "xlsx" | "pptx" | "odp" | "txt" | "md" | "csv" | "json" | "html" | "htm" | "xml" | "eml" | "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "jfif"
    )
}

/// Treffer bewerten und begründen
fn rate(p: &Path, text: &str, terms: &[String]) -> Option<Hit> {
    let name = norm(&p.file_name()?.to_string_lossy());
    let body = norm(text);
    let compact: String = body.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    let mut score = 0.0f32;
    let mut found: Vec<String> = Vec::new();
    let mut in_name: Vec<String> = Vec::new();
    let mut best_at: Option<usize> = None;
    // benachbarte Begriffe zusammen („media“ + „markt“ → „mediamarkt“) zählen als ein Begriff
    let mut list: Vec<(String, bool)> = terms.iter().map(|t| (t.clone(), false)).collect();
    for w in terms.windows(2) {
        list.push((format!("{}{}", w[0], w[1]), true));
    }
    let mut matched_pair = Vec::new();
    for (t, pair) in &list {
        let n = if *pair { compact.matches(t.as_str()).count() } else { body.matches(t.as_str()).count() };
        let syn = SYNONYMS.iter().find(|(k, _)| norm(k) == *t).map_or(0, |(_, s)| s.iter().map(|x| body.matches(norm(x).as_str()).count()).sum());
        if n > 0 || syn > 0 {
            score += if *pair { 3.0 } else { 2.0 } + (n.min(10) as f32).ln_1p() + if n == 0 { 0.0 } else { 0.5 } + if syn > 0 && n == 0 { -1.0 } else { 0.0 };
            if *pair {
                matched_pair.push(t.clone());
            }
            found.push(if n > 0 { format!("„{t}“ ({n}×)") } else { format!("„{t}“ (verwandt)") });
            if best_at.is_none() || *pair {
                best_at = body.find(t.as_str()).or(best_at);
            }
        }
        if name.contains(t.as_str()) {
            score += 2.5;
            in_name.push(t.clone());
        }
    }
    if score <= 0.0 {
        return None;
    }
    // mehr gefundene Begriffe = deutlich besser
    let covered = terms.iter().filter(|t| body.contains(t.as_str()) || name.contains(t.as_str()) || matched_pair.iter().any(|p| p.contains(t.as_str()))).count();
    score *= 0.5 + covered as f32 / terms.len().max(1) as f32;
    let mut why = Vec::new();
    if !found.is_empty() {
        why.push(format!("enthält {}", found.join(", ")));
    }
    if !in_name.is_empty() {
        why.push(format!("Dateiname passt ({})", in_name.join(", ")));
    }
    if let Some(d) = date_in(text) {
        why.push(format!("Datum im Text: {d}"));
    }
    if let Some(a) = amount_in(text) {
        why.push(format!("Betrag: {a}"));
    }
    let ext = crate::convert::ext_of(p);
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "heic" | "bmp" | "tif" | "tiff") {
        why.push("Text im Bild erkannt".into());
    }
    let snippet = best_at.map(|i| snippet(text, &body, i)).unwrap_or_default();
    Some(Hit { path: p.to_path_buf(), score, why: why.join(" · "), snippet })
}

/// Textstelle um eine Fundstelle (Position im normalisierten Text, ungefähr übertragen)
fn snippet(text: &str, body: &str, at: usize) -> String {
    let frac = at as f64 / body.len().max(1) as f64;
    let chars: Vec<char> = text.chars().collect();
    let c = (frac * chars.len() as f64) as usize;
    let (a, b) = (c.saturating_sub(70), (c + 110).min(chars.len()));
    let s: String = chars[a..b].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{}{s}{}", if a > 0 { "… " } else { "" }, if b < chars.len() { " …" } else { "" })
}

fn date_in(t: &str) -> Option<String> {
    let re = regex::Regex::new(r"\b([0-3]?\d)\.([01]?\d)\.((?:19|20)?\d\d)\b").ok()?;
    re.find(t).map(|m| m.as_str().to_string())
}

fn amount_in(t: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?i)(?:summe|gesamt|total|betrag|zu zahlen)[^\d\n]{0,25}(\d{1,6}[.,]\d{2})\s*(?:€|eur)?").ok()?;
    re.captures(t).and_then(|c| c.get(1)).map(|m| format!("{} €", m.as_str()))
}

impl Search {
    /// im Hintergrund suchen (Unterordner bis Tiefe 6, höchstens 4000 Dateien)
    pub fn start(query: &str, root: &Path, ctx: &eframe::egui::Context) -> Search {
        let s = Search::default();
        let terms = terms(query);
        let (hits, scanned, total, done, cancel) = (s.hits.clone(), s.scanned.clone(), s.total.clone(), s.done.clone(), s.cancel.clone());
        let (root, ctx) = (root.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let mut files = Vec::new();
            let mut stack = vec![(root, 0)];
            while let Some((d, depth)) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    let n = e.file_name().to_string_lossy().to_lowercase();
                    if e.file_type().is_ok_and(|t| t.is_dir()) {
                        if depth < 6 && !n.starts_with('.') && !matches!(n.as_str(), "node_modules" | "target" | "$recycle.bin" | "appdata" | "windows") {
                            stack.push((p, depth + 1));
                        }
                    } else if searchable(&p) && e.metadata().is_ok_and(|m| m.len() < 60 << 20) {
                        files.push(p);
                    }
                }
                if files.len() >= 4000 || cancel.load(Relaxed) {
                    break;
                }
            }
            total.store(files.len() as u64, Relaxed);
            if terms.is_empty() {
                done.store(true, Relaxed);
                return;
            }
            let cache = Mutex::new(load_cache());
            use rayon::prelude::*;
            files.par_iter().with_max_len(1).for_each(|p| {
                if cancel.load(Relaxed) {
                    return;
                }
                let md = std::fs::metadata(p).ok();
                let (len, mt) = (md.as_ref().map_or(0, |m| m.len()), md.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64));
                let key = p.to_string_lossy().into_owned();
                let cached = cache.lock().ok().and_then(|c| c.get(&key).filter(|e| e.0 == len && e.1 == mt).map(|e| e.2.clone()));
                let text = match cached {
                    Some(t) => t,
                    None => {
                        let t = extract(p).unwrap_or_default();
                        if let Ok(mut c) = cache.lock() {
                            c.insert(key, (len, mt, t.clone()));
                        }
                        t
                    }
                };
                if let Some(h) = rate(p, &text, &terms) {
                    if let Ok(mut v) = hits.lock() {
                        v.push(h);
                        v.sort_by(|a, b| b.score.total_cmp(&a.score));
                        v.truncate(60);
                    }
                }
                scanned.fetch_add(1, Relaxed);
                ctx.request_repaint();
            });
            if let Ok(c) = cache.into_inner() {
                save_cache(&c);
            }
            done.store(true, Relaxed);
            ctx.request_repaint();
        });
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_terms() {
        assert_eq!(terms("finde die Rechnung von Media Markt"), vec!["rechnung", "media", "markt"]);
    }

    #[test]
    fn rates_invoice() {
        let t = "MediaMarkt Berlin\nRechnung Nr. 4711\nDatum 12.03.2024\nGesamtbetrag 349,99 EUR";
        let h = rate(Path::new("scan.pdf"), t, &terms("finde die Rechnung von Media Markt")).unwrap();
        assert!(h.why.contains("rechnung") && h.why.contains("mediamarkt"), "{}", h.why);
        assert!(h.why.contains("12.03.2024"));
        assert!(rate(Path::new("x.txt"), "Einkaufsliste Milch", &terms("Rechnung Media Markt")).is_none());
    }
}
