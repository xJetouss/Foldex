//! Text in Bildern und PDF-Seiten: Erkennung (Windows-Texterkennung bzw. Wortlagen von pdftotext) und eine
//! markierbare Textebene über dem Bild – Wörter mit der Maus markieren (ziehen, Doppelklick, Strg+A) und mit
//! Strg+C oder Rechtsklick kopieren, wie in einem Text.

use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Pos2, Rect};

/// Erkanntes Wort; `rect` relativ zum Bild (0..1), `line` = Zeile in Lesereihenfolge.
#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    pub rect: Rect,
    pub line: usize,
}

/// Wörter in Lesereihenfolge bringen: Wörter, deren Mitte auf gleicher Höhe liegt, bilden eine Zeile
/// (oben nach unten, in der Zeile links nach rechts).
pub fn order(mut words: Vec<Word>) -> Vec<Word> {
    words.sort_by(|a, b| a.rect.center().y.total_cmp(&b.rect.center().y));
    let mut rows: Vec<(f32, f32, Vec<Word>)> = Vec::new();
    for w in words {
        let (y, h) = (w.rect.center().y, w.rect.height());
        match rows.last_mut() {
            Some((ry, rh, ws)) if (y - *ry).abs() < rh.max(h) * 0.55 => {
                // Zeilenhöhe mitteln, damit schiefe Zeilen zusammenbleiben
                *ry = (*ry * ws.len() as f32 + y) / (ws.len() + 1) as f32;
                ws.push(w);
            }
            _ => rows.push((y, h, vec![w])),
        }
    }
    let mut out = Vec::new();
    for (k, (_, _, mut ws)) in rows.into_iter().enumerate() {
        ws.sort_by(|a, b| a.rect.left().total_cmp(&b.rect.left()));
        out.extend(ws.into_iter().map(|mut w| {
            w.line = k;
            w
        }));
    }
    out
}

/// Text der Wörter `a..=b`: Leerzeichen in der Zeile, Zeilenumbruch zwischen Zeilen.
pub fn text_of(words: &[Word], a: usize, b: usize) -> String {
    let mut s = String::new();
    let mut last: Option<usize> = None;
    for w in words.iter().take(b + 1).skip(a) {
        match last {
            Some(l) if l == w.line => s.push(' '),
            Some(_) => s.push('\n'),
            None => {}
        }
        s.push_str(&w.text);
        last = Some(w.line);
    }
    s
}

pub fn all_text(words: &[Word]) -> String {
    if words.is_empty() {
        String::new()
    } else {
        text_of(words, 0, words.len() - 1)
    }
}

/// Text eines Bildes erkennen (Windows-Texterkennung). Kleine Bilder werden vorher vergrößert (die Erkennung
/// arbeitet am besten mit Schrift von gut 20 px Höhe), zu große verkleinert.
#[cfg(windows)]
pub fn recognize(img: &image::DynamicImage) -> Result<Vec<Word>, String> {
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let max_dim = OcrEngine::MaxImageDimension().unwrap_or(2600).max(1000);
    let (w0, h0) = (img.width().max(1), img.height().max(1));
    let side = w0.max(h0) as f32;
    let k = if side < 1800.0 { (1800.0 / side).min(3.0) } else { 1.0 }.min(max_dim as f32 / side);
    let img = if (k - 1.0).abs() > 0.01 {
        img.resize(((w0 as f32 * k) as u32).max(1), ((h0 as f32 * k) as u32).max(1), image::imageops::FilterType::CatmullRom)
    } else {
        img.clone()
    };
    // Transparenz auf Weiß legen (sonst wird transparenter Hintergrund schwarz und dunkle Schrift unlesbar)
    let rgba = img.to_rgba8();
    let flat = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let a = p[3] as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        image::Rgb([mix(p[0]), mix(p[1]), mix(p[2])])
    });
    let (w, h) = (flat.width() as f32, flat.height() as f32);
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(flat).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| "Keine Texterkennung für die Systemsprache installiert".to_string())?;
    let words = (|| -> windows::core::Result<Vec<Word>> {
        let stream = InMemoryRandomAccessStream::new()?;
        let dw = DataWriter::CreateDataWriter(&stream)?;
        dw.WriteBytes(&png)?;
        dw.StoreAsync()?.join()?;
        dw.FlushAsync()?.join()?;
        let _ = dw.DetachStream();
        stream.Seek(0)?;
        let bmp = BitmapDecoder::CreateAsync(&stream)?.join()?.GetSoftwareBitmapAsync()?.join()?;
        let res = engine.RecognizeAsync(&bmp)?.join()?;
        let lines = res.Lines()?;
        let mut out = Vec::new();
        for k in 0..lines.Size()? {
            let ws = lines.GetAt(k)?.Words()?;
            for j in 0..ws.Size()? {
                let wd = ws.GetAt(j)?;
                let r = wd.BoundingRect()?;
                out.push(Word { text: wd.Text()?.to_string(), rect: Rect::from_min_size(pos2(r.X / w, r.Y / h), vec2(r.Width / w, r.Height / h)), line: 0 });
            }
        }
        Ok(out)
    })()
    .map_err(|e| format!("Texterkennung fehlgeschlagen: {}", e.message()))?;
    Ok(order(words))
}

#[cfg(not(windows))]
pub fn recognize(_img: &image::DynamicImage) -> Result<Vec<Word>, String> {
    Err("Texterkennung gibt es nur unter Windows".into())
}

/// Wörter aller Seiten aus „pdftotext -bbox“ (genau, ohne Erkennung). Leere Seiten = kein Text (Scan).
pub fn pdf_words(pdftotext: &std::path::Path, pdf: &std::path::Path) -> Result<Vec<Vec<Word>>, String> {
    let out = crate::convert::quiet(pdftotext).args(["-bbox", "-enc", "UTF-8"]).arg(pdf).arg("-").output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("pdftotext -bbox fehlgeschlagen".into());
    }
    let html = String::from_utf8_lossy(&out.stdout);
    let num = |s: &str, key: &str| -> f32 {
        s.find(&format!("{key}=\"")).and_then(|i| s[i + key.len() + 2..].split('"').next()).and_then(|v| v.parse().ok()).unwrap_or(0.0)
    };
    let mut pages: Vec<Vec<Word>> = Vec::new();
    for page in html.split("<page ").skip(1) {
        let (pw, ph) = (num(page, "width").max(1.0), num(page, "height").max(1.0));
        let mut words = Vec::new();
        for part in page.split("<word ").skip(1) {
            let Some((attrs, rest)) = part.split_once('>') else { continue };
            let Some(text) = rest.split("</word>").next() else { continue };
            let r = Rect::from_min_max(pos2(num(attrs, "xMin") / pw, num(attrs, "yMin") / ph), pos2(num(attrs, "xMax") / pw, num(attrs, "yMax") / ph));
            words.push(Word { text: unescape(text), rect: r, line: 0 });
        }
        pages.push(order(words));
    }
    Ok(pages)
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent.strip_prefix('#').and_then(|n| n.strip_prefix('x').map_or_else(|| n.parse().ok(), |h| u32::from_str_radix(h, 16).ok())).and_then(char::from_u32),
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Markierung in einer Textebene: Anker und Ende (Wortindizes).
#[derive(Default, Clone, Copy)]
pub struct Selection {
    anchor: Option<usize>,
    end: Option<usize>,
}

impl Selection {
    pub fn range(&self) -> Option<(usize, usize)> {
        Some((self.anchor?.min(self.end?), self.anchor?.max(self.end?)))
    }
    pub fn clear(&mut self) {
        *self = Selection::default();
    }
    /// alle `n` Wörter markieren
    pub fn all(n: usize) -> Selection {
        if n == 0 {
            Selection::default()
        } else {
            Selection { anchor: Some(0), end: Some(n - 1) }
        }
    }
}

/// Wort unter bzw. am nächsten zu `p` (Bildschirm); `exact` = nur, wenn der Zeiger auf dem Wort liegt.
fn hit(words: &[Word], r: Rect, p: Pos2, exact: bool) -> Option<usize> {
    let to_screen = |w: &Word| Rect::from_min_max(r.lerp_inside(w.rect.min.to_vec2()), r.lerp_inside(w.rect.max.to_vec2()));
    if let Some(i) = words.iter().position(|w| to_screen(w).expand(2.0).contains(p)) {
        return Some(i);
    }
    if exact {
        return None;
    }
    // sonst: in der Zeile auf dieser Höhe das nächste Wort, notfalls das insgesamt nächste
    let mut line = words.iter().filter(|w| {
        let s = to_screen(w);
        p.y >= s.top() - 2.0 && p.y <= s.bottom() + 2.0
    });
    let near = |it: &mut dyn Iterator<Item = &Word>| it.min_by(|a, b| to_screen(a).distance_sq_to_pos(p).total_cmp(&to_screen(b).distance_sq_to_pos(p))).and_then(|w| words.iter().position(|x| std::ptr::eq(x, w)));
    near(&mut line).or_else(|| near(&mut words.iter()))
}

/// Textebene über dem Bild `r`: markieren mit der Maus (ziehen; Doppelklick = Wort; Strg+A = alles),
/// kopieren mit Strg+C oder Rechtsklick. Gibt eine Meldung zurück, wenn kopiert wurde.
pub fn text_layer(ui: &mut egui::Ui, th: &Theme, id: egui::Id, r: Rect, words: &[Word], sel: &mut Selection) -> Option<String> {
    if words.is_empty() {
        return None;
    }
    let resp = ui.interact(r, id, egui::Sense::click_and_drag());
    let mut msg = None;
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let on_word = pointer.and_then(|p| hit(words, r, p, true));
    if on_word.is_some() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            let i = hit(words, r, p, false);
            *sel = Selection { anchor: i, end: i };
            resp.request_focus();
        }
    } else if resp.dragged() {
        if let Some(p) = resp.interact_pointer_pos() {
            sel.end = hit(words, r, p, false).or(sel.end);
        }
    }
    if resp.double_clicked() {
        if let Some(i) = on_word {
            *sel = Selection { anchor: Some(i), end: Some(i) };
            resp.request_focus();
        }
    } else if resp.clicked() && !resp.dragged() {
        // Klick ohne Ziehen: Markierung aufheben
        sel.clear();
        resp.surrender_focus();
    }
    let focused = resp.has_focus();
    if focused && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
        *sel = Selection { anchor: Some(0), end: Some(words.len() - 1) };
    }
    let copy_now = focused && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
    if let (true, Some((a, b))) = (copy_now, sel.range()) {
        ui.ctx().copy_text(text_of(words, a, b));
        msg = Some("Text kopiert".to_string());
    }
    resp.context_menu(|ui| {
        if ui.add_enabled(sel.range().is_some(), egui::Button::new("Markierten Text kopieren")).clicked() {
            if let Some((a, b)) = sel.range() {
                ui.ctx().copy_text(text_of(words, a, b));
                msg = Some("Text kopiert".to_string());
            }
            ui.close_menu();
        }
        if ui.button("Gesamten Text kopieren").clicked() {
            ui.ctx().copy_text(all_text(words));
            msg = Some("Gesamter Text kopiert".to_string());
            ui.close_menu();
        }
        if ui.button("Alles markieren").clicked() {
            *sel = Selection { anchor: Some(0), end: Some(words.len() - 1) };
            ui.close_menu();
        }
    });
    // Markierung: je Zeile ein durchgehendes Band über die markierten Wörter
    let p = ui.painter_at(r);
    let to_screen = |w: &Word| Rect::from_min_max(r.lerp_inside(w.rect.min.to_vec2()), r.lerp_inside(w.rect.max.to_vec2()));
    if let Some((a, b)) = sel.range() {
        let fill = th.accent.gamma_multiply(0.35);
        let mut band: Option<(usize, Rect)> = None;
        for w in &words[a..=b.min(words.len() - 1)] {
            let s = to_screen(w);
            band = match band {
                Some((l, rr)) if l == w.line => Some((l, rr.union(s))),
                Some((_, rr)) => {
                    p.rect_filled(rr.expand(1.5), 2.0, fill);
                    Some((w.line, s))
                }
                None => Some((w.line, s)),
            };
        }
        if let Some((_, rr)) = band {
            p.rect_filled(rr.expand(1.5), 2.0, fill);
        }
    } else if let Some(i) = on_word {
        // Hinweis: hier ist erkannter Text
        p.rect_stroke(to_screen(&words[i]).expand(1.5), 2.0, egui::Stroke::new(1.0, th.accent.gamma_multiply(0.6)));
    }
    msg
}
