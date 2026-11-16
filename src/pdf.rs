//! Kleiner PDF-Schreiber ohne Abhängigkeiten: Text (Standardschriften Helvetica/Courier, A4, Zeilenumbruch)
//! und Bilder (JPEG unverändert eingebettet, eine Seite je Bild).

use crate::docconv::{to_cp1252, Block};

const W: f32 = 595.0;
const H: f32 = 842.0;
const MARGIN: f32 = 56.0;

/// PDF aus Objekten zusammensetzen (Objekt 1 = Katalog, 2 = Seitenbaum).
fn assemble(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    v.extend_from_slice(data);
    v.extend_from_slice(b"\nendstream");
    v
}

/// Text als PDF-String (WinAnsi), Klammern und Backslash maskiert.
fn pdf_str(t: &str) -> Vec<u8> {
    let mut v = vec![b'('];
    for c in t.chars() {
        let b = to_cp1252(c);
        if matches!(b, b'(' | b')' | b'\\') {
            v.push(b'\\');
        }
        v.push(b);
    }
    v.push(b')');
    v
}

/// Zeilen umbrechen; Breite grob geschätzt (Helvetica ≈ 0,5 em je Zeichen, Courier 0,6 em).
fn wrap(t: &str, size: f32, mono: bool) -> Vec<String> {
    let max = ((W - 2.0 * MARGIN) / (size * if mono { 0.6 } else { 0.5 })) as usize;
    let mut out = Vec::new();
    for para in t.split('\n') {
        if mono {
            let chars: Vec<char> = para.chars().collect();
            if chars.is_empty() {
                out.push(String::new());
            }
            for c in chars.chunks(max.max(1)) {
                out.push(c.iter().collect());
            }
            continue;
        }
        let mut line = String::new();
        for w in para.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + w.chars().count() > max {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(w);
        }
        out.push(line);
    }
    out
}

/// Text-Dokument als PDF (A4, Überschriften fett, Code in Courier).
pub fn text_pdf(blocks: &[Block]) -> Vec<u8> {
    // (Schrift, Größe, Einzug, Text) je Zeile; None = Abstand
    let mut lines: Vec<Option<(&str, f32, f32, String)>> = Vec::new();
    for b in blocks {
        match b {
            Block::H(l, t) => {
                let size = [20.0, 16.0, 14.0, 12.5, 12.0, 11.5][(*l as usize).clamp(1, 6) - 1];
                lines.push(None);
                lines.extend(wrap(t, size, false).into_iter().map(|s| Some(("F2", size, 0.0, s))));
                lines.push(None);
            }
            Block::P(t) => {
                lines.extend(wrap(t, 11.0, false).into_iter().map(|s| Some(("F1", 11.0, 0.0, s))));
                lines.push(None);
            }
            Block::Li(t) => {
                for (i, s) in wrap(t, 11.0, false).into_iter().enumerate() {
                    lines.push(Some(("F1", 11.0, 14.0, if i == 0 { format!("• {s}") } else { format!("  {s}") })));
                }
            }
            Block::Code(t) => {
                lines.extend(wrap(t, 9.5, true).into_iter().map(|s| Some(("F3", 9.5, 0.0, s))));
                lines.push(None);
            }
            Block::Table(rows) => {
                for r in rows {
                    lines.extend(wrap(&r.join("   |   "), 10.0, false).into_iter().map(|s| Some(("F1", 10.0, 0.0, s))));
                }
                lines.push(None);
            }
        }
    }
    // auf Seiten verteilen
    let mut pages: Vec<Vec<u8>> = vec![Vec::new()];
    let mut y = H - MARGIN;
    for l in lines {
        let lead = l.as_ref().map_or(6.0, |x| x.1 * 1.35);
        if y - lead < MARGIN {
            pages.push(Vec::new());
            y = H - MARGIN;
            if l.is_none() {
                continue;
            }
        }
        y -= lead;
        if let Some((font, size, indent, text)) = l {
            let p = pages.last_mut().unwrap();
            p.extend_from_slice(format!("BT /{font} {size} Tf {:.1} {y:.1} Td ", MARGIN + indent).as_bytes());
            p.extend_from_slice(&pdf_str(&text));
            p.extend_from_slice(b" Tj ET\n");
        }
    }
    let n = pages.len();
    // Objekte: 1 Katalog, 2 Seiten, 3–5 Schriften, dann je Seite: Seite + Inhalt
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Count {n} /Kids [{}] >>", (0..n).map(|i| format!("{} 0 R", 6 + 2 * i)).collect::<Vec<_>>().join(" ")).into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    for (i, content) in pages.iter().enumerate() {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {W} {H}] /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >> >> /Contents {} 0 R >>", 7 + 2 * i).into_bytes());
        objs.push(stream("", content));
    }
    assemble(objs)
}

/// Bilder (JPEG, Breite, Höhe) als PDF – jede Seite so groß wie das Bild (auf A4 begrenzt).
pub fn image_pdf(images: &[(Vec<u8>, u32, u32)]) -> Vec<u8> {
    let n = images.len();
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Count {n} /Kids [{}] >>", (0..n).map(|i| format!("{} 0 R", 3 + 3 * i)).collect::<Vec<_>>().join(" ")).into_bytes(),
    ];
    for (i, (jpg, w, h)) in images.iter().enumerate() {
        // 96 dpi → Punkte, höchstens A4
        let s = (0.75f32).min(W / *w as f32).min(H / *h as f32).max(0.01);
        let (pw, ph) = (*w as f32 * s, *h as f32 * s);
        let base = 3 + 3 * i;
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>", base + 1, base + 2).into_bytes());
        objs.push(stream(&format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"), jpg));
        objs.push(stream("", format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q").as_bytes()));
    }
    assemble(objs)
}
