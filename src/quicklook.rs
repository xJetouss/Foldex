//! Schnellansicht für weitere Formate: Office-Dokumente (als lesbarer Text mit Überschriften und Tabellen),
//! Schriften (Schriftmuster) und 3D-Modelle (.obj, .stl, .glb, .gltf – kleiner Software-Renderer).

use std::path::Path;

use eframe::egui::ColorImage;

/// Office-Dokument → Markdown (Word, LibreOffice, RTF, EPUB, Excel, PowerPoint, Impress)
pub fn office_markdown(p: &Path, ext: &str) -> Option<String> {
    let md = match ext {
        "docx" | "odt" | "rtf" | "epub" | "xlsx" => crate::docconv::to_markdown(&crate::docconv::read(p, ext).ok()?),
        "pptx" => slides(p, "ppt/slides/slide", "a:t")?,
        "odp" => slides(p, "content.xml", "text:p")?,
        _ => return None,
    };
    (!md.trim().is_empty()).then_some(md)
}

/// Text der Folien (PowerPoint: eine XML-Datei je Folie; Impress: alles in content.xml)
fn slides(p: &Path, prefix: &str, tag: &str) -> Option<String> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(p).ok()?).ok()?;
    let mut names: Vec<String> = z.file_names().filter(|n| n.starts_with(prefix) && n.ends_with(".xml")).map(String::from).collect();
    // slide2.xml vor slide10.xml
    names.sort_by_key(|n| n.trim_start_matches(prefix).trim_end_matches(".xml").parse::<u32>().unwrap_or(0));
    let mut out = String::new();
    for (i, n) in names.iter().enumerate() {
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut z.by_name(n).ok()?, &mut xml).ok()?;
        let texts = xml_texts(&xml, tag);
        if names.len() > 1 || prefix.starts_with("ppt") {
            out.push_str(&format!("## Folie {}\n\n", i + 1));
        }
        for t in texts {
            out.push_str(&t);
            out.push_str("\n\n");
        }
    }
    Some(out)
}

/// Inhalt aller <tag …>…</tag> (ohne innere Tags), XML-Zeichen entschlüsselt
fn xml_texts(xml: &str, tag: &str) -> Vec<String> {
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut v = Vec::new();
    let mut rest = xml;
    while let Some(a) = rest.find(&open) {
        let after = &rest[a + open.len()..];
        // <a:tab> o. ä. nicht mit <a:t> verwechseln
        if !after.starts_with(['>', ' ', '/']) {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            rest = &after[gt..];
            continue;
        }
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        let mut t = String::new();
        let mut in_tag = false;
        for c in body[..end].chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => t.push(c),
                _ => {}
            }
        }
        let t = t.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&");
        if !t.trim().is_empty() {
            v.push(t.trim().to_string());
        }
        rest = &body[end + close.len()..];
    }
    v
}

// ───────────────────────── Schriften ─────────────────────────

/// Name aus der „name“-Tabelle (vollständiger Name, sonst Familie)
fn font_name(b: &[u8]) -> Option<String> {
    let u16_at = |o: usize| b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize);
    let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize);
    // Schriftsammlung (.ttc): erste Schrift
    let base = if b.get(0..4) == Some(b"ttcf") { u32_at(12)? } else { 0 };
    let n = u16_at(base + 4)?;
    let name = (0..n).find_map(|i| {
        let r = base + 12 + i * 16;
        (b.get(r..r + 4) == Some(b"name")).then(|| u32_at(r + 8)).flatten()
    })?;
    let (count, strings) = (u16_at(name + 2)?, name + u16_at(name + 4)?);
    let mut best: Option<(usize, String)> = None;
    for i in 0..count {
        let r = name + 6 + i * 12;
        let (plat, id, len, off) = (u16_at(r)?, u16_at(r + 6)?, u16_at(r + 8)?, u16_at(r + 10)?);
        let rank = match id {
            4 => 0,
            1 => 1,
            _ => continue,
        };
        let raw = b.get(strings + off..strings + off + len)?;
        let s = if plat == 3 || plat == 0 {
            String::from_utf16_lossy(&raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>())
        } else {
            raw.iter().map(|&c| c as char).collect()
        };
        if !s.trim().is_empty() && best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, s.trim().to_string()));
        }
    }
    best.map(|b| b.1)
}

/// Schriftmuster: Name, Alphabet, Ziffern und ein Satz in mehreren Größen
pub fn font_specimen(p: &Path) -> Result<(ColorImage, Vec<(String, String)>), String> {
    use ab_glyph::{Font, FontVec};
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
    let name = font_name(&bytes).unwrap_or_else(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let font = FontVec::try_from_vec_and_index(bytes, 0).map_err(|_| "Keine lesbare Schrift (TrueType/OpenType)".to_string())?;
    let glyphs = font.glyph_count();
    let lines: Vec<(String, f32, [u8; 3])> = vec![
        (name.clone(), 46.0, [20, 20, 24]),
        ("ABCDEFGHIJKLMNOPQRSTUVWXYZ ÄÖÜ".into(), 30.0, [40, 40, 46]),
        ("abcdefghijklmnopqrstuvwxyz äöüß".into(), 30.0, [40, 40, 46]),
        ("0123456789 !?&@€$%()[]{}+-=/".into(), 30.0, [40, 40, 46]),
        ("Franz jagt im komplett verwahrlosten Taxi quer durch Bayern.".into(), 14.0, [70, 70, 78]),
        ("Franz jagt im komplett verwahrlosten Taxi quer durch Bayern.".into(), 20.0, [70, 70, 78]),
        ("Franz jagt im komplett verwahrlosten Taxi quer durch Bayern.".into(), 28.0, [60, 60, 68]),
        ("Victor jagt zwölf Boxkämpfer quer über den großen Sylter Deich.".into(), 40.0, [50, 50, 58]),
        ("Sphinx of black quartz, judge my vow.".into(), 56.0, [30, 30, 36]),
    ];
    let (w, pad) = (1400u32, 36.0f32);
    let h: f32 = pad * 2.0 + lines.iter().map(|l| l.1 * 1.45).sum::<f32>() + 16.0;
    let mut img = image::RgbaImage::from_pixel(w, h.ceil() as u32, image::Rgba([250, 250, 252, 255]));
    let mut y = pad;
    for (i, (t, px, c)) in lines.iter().enumerate() {
        y += px * 1.15;
        draw_line(&mut img, &font, t, pad, y, *px, *c);
        y += px * 0.30;
        if i == 0 || i == 3 {
            // Trennlinie
            for x in pad as u32..w - pad as u32 {
                img.put_pixel(x, (y + 4.0) as u32, image::Rgba([225, 225, 230, 255]));
            }
            y += 8.0;
        }
    }
    let ci = ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    Ok((ci, vec![("Schrift".into(), name), ("Zeichen".into(), glyphs.to_string())]))
}

fn draw_line(img: &mut image::RgbaImage, font: &ab_glyph::FontVec, text: &str, x0: f32, base: f32, px: f32, c: [u8; 3]) {
    use ab_glyph::{Font, ScaleFont};
    let sf = font.as_scaled(px);
    let mut x = x0;
    let mut prev: Option<ab_glyph::GlyphId> = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        let g = id.with_scale_and_position(px, ab_glyph::point(x, base));
        x += sf.h_advance(id);
        prev = Some(id);
        if x > img.width() as f32 - 8.0 {
            break;
        }
        if let Some(o) = font.outline_glyph(g) {
            let bb = o.px_bounds();
            o.draw(|gx, gy, cov| {
                let (px_, py_) = (bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32);
                if px_ >= 0 && py_ >= 0 && (px_ as u32) < img.width() && (py_ as u32) < img.height() {
                    let d = img.get_pixel_mut(px_ as u32, py_ as u32);
                    let a = cov.clamp(0.0, 1.0);
                    for k in 0..3 {
                        d.0[k] = (d.0[k] as f32 * (1.0 - a) + c[k] as f32 * a) as u8;
                    }
                }
            });
        }
    }
}

// ───────────────────────── 3D-Modelle ─────────────────────────

type V3 = [f32; 3];
type Mat = [[f32; 4]; 4];

/// Dreiecke eines Modells (Y zeigt nach oben)
pub fn load_model(p: &Path, ext: &str) -> Result<Vec<[V3; 3]>, String> {
    let tris = match ext {
        "obj" => obj(&std::fs::read_to_string(p).map_err(|e| e.to_string())?),
        "stl" => stl(&std::fs::read(p).map_err(|e| e.to_string())?),
        "glb" => {
            let b = std::fs::read(p).map_err(|e| e.to_string())?;
            let (json, bin) = glb_chunks(&b).ok_or("Keine gültige GLB-Datei")?;
            gltf(&json, |i| if i == 0 { bin.clone() } else { None }, p)?
        }
        "gltf" => {
            let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            gltf(&json, |_| None, p)?
        }
        _ => return Err("Unbekanntes 3D-Format".into()),
    };
    if tris.is_empty() {
        return Err("Das Modell enthält keine Dreiecke".into());
    }
    Ok(tris)
}

fn obj(t: &str) -> Vec<[V3; 3]> {
    let mut v: Vec<V3> = Vec::new();
    let mut tris = Vec::new();
    for l in t.lines() {
        let mut it = l.split_whitespace();
        match it.next() {
            Some("v") => {
                let c: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if c.len() == 3 {
                    v.push([c[0], c[1], c[2]]);
                }
            }
            Some("f") => {
                let idx: Vec<usize> = it
                    .filter_map(|x| x.split('/').next()?.parse::<i64>().ok())
                    .filter_map(|i| if i < 0 { (v.len() as i64 + i).try_into().ok() } else { (i - 1).try_into().ok() })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    if let (Some(a), Some(b), Some(c)) = (v.get(idx[0]), v.get(idx[k]), v.get(idx[k + 1])) {
                        tris.push([*a, *b, *c]);
                    }
                }
            }
            _ => {}
        }
    }
    tris
}

fn stl(b: &[u8]) -> Vec<[V3; 3]> {
    // STL zeigt meist mit Z nach oben
    let up = |p: V3| [p[0], p[2], -p[1]];
    let n = b.get(80..84).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize);
    if b.len() == 84 + n * 50 && n > 0 {
        let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        return (0..n)
            .map(|i| {
                let o = 84 + i * 50 + 12;
                let v = |k: usize| up([f(o + k * 12), f(o + k * 12 + 4), f(o + k * 12 + 8)]);
                [v(0), v(1), v(2)]
            })
            .collect();
    }
    let t = String::from_utf8_lossy(b);
    let vs: Vec<V3> = t
        .lines()
        .filter_map(|l| {
            let l = l.trim().strip_prefix("vertex")?;
            let c: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            (c.len() == 3).then(|| up([c[0], c[1], c[2]]))
        })
        .collect();
    vs.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
}

fn glb_chunks(b: &[u8]) -> Option<(serde_json::Value, Option<Vec<u8>>)> {
    let u = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize);
    if b.get(0..4)? != b"glTF" {
        return None;
    }
    let (jl, mut o) = (u(12)?, 20);
    let json = serde_json::from_slice(b.get(o..o + jl)?).ok()?;
    o += jl;
    let bin = u(o).and_then(|bl| b.get(o + 8..o + 8 + bl)).map(<[u8]>::to_vec);
    Some((json, bin))
}

fn mat_mul(a: &Mat, b: &Mat) -> Mat {
    let mut m = [[0.0; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, c) in row.iter_mut().enumerate() {
            *c = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

const IDENT: Mat = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// Knoten-Matrix (glTF: Spalten-Reihenfolge bzw. Verschiebung · Drehung · Skalierung); hier zeilenweise m[Zeile][Spalte]
fn node_mat(n: &serde_json::Value) -> Mat {
    let f = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32;
    if let Some(m) = n["matrix"].as_array().filter(|m| m.len() == 16) {
        let mut r = [[0.0; 4]; 4];
        for c in 0..4 {
            for row in 0..4 {
                r[row][c] = f(&m[c * 4 + row]);
            }
        }
        return r;
    }
    let get = |k: &str, d: &[f32]| n[k].as_array().map_or(d.to_vec(), |a| a.iter().map(f).collect());
    let (t, q, s) = (get("translation", &[0.0; 3]), get("rotation", &[0.0, 0.0, 0.0, 1.0]), get("scale", &[1.0; 3]));
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let r = [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ];
    let mut m = IDENT;
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = r[i][j] * s[j];
        }
        m[i][3] = t[i];
    }
    m
}

fn apply(m: &Mat, p: V3) -> V3 {
    [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3])
}

fn gltf(j: &serde_json::Value, embedded: impl Fn(usize) -> Option<Vec<u8>>, path: &Path) -> Result<Vec<[V3; 3]>, String> {
    // Puffer laden (eingebettet oder Datei daneben)
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for (i, b) in j["buffers"].as_array().cloned().unwrap_or_default().iter().enumerate() {
        let data = match b["uri"].as_str() {
            Some(u) if u.starts_with("data:") => return Err("Eingebettete Base64-Puffer werden in der Schnellansicht nicht gelesen".into()),
            Some(u) => std::fs::read(path.with_file_name(u.replace("%20", " "))).map_err(|e| format!("Puffer {u}: {e}"))?,
            None => embedded(i).unwrap_or_default(),
        };
        buffers.push(data);
    }
    let accessor = |i: usize| -> Option<(Vec<u8>, usize, usize, usize, usize, usize)> {
        // (Bytes, Start, Abstand, Anzahl, Komponententyp, Komponenten)
        let a = &j["accessors"][i];
        let bv = &j["bufferViews"][a["bufferView"].as_u64()? as usize];
        let buf = buffers.get(bv["buffer"].as_u64()? as usize)?;
        let ct = a["componentType"].as_u64()? as usize;
        let comps = match a["type"].as_str()? {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => return None,
        };
        let size = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            _ => 4,
        };
        let start = bv["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = bv["byteStride"].as_u64().map_or(size * comps, |s| s as usize);
        Some((buf.clone(), start, stride, a["count"].as_u64()? as usize, ct, comps))
    };
    let mut tris = Vec::new();
    let prim = |mesh: usize, m: &Mat, tris: &mut Vec<[V3; 3]>| {
        for p in j["meshes"][mesh]["primitives"].as_array().cloned().unwrap_or_default() {
            if p["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            let Some((buf, st, sd, n, 5126, 3)) = p["attributes"]["POSITION"].as_u64().and_then(|i| accessor(i as usize)) else { continue };
            let pos: Vec<V3> = (0..n)
                .filter_map(|k| {
                    let o = st + k * sd;
                    let f = |q: usize| buf.get(o + q * 4..o + q * 4 + 4).map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]));
                    Some(apply(m, [f(0)?, f(1)?, f(2)?]))
                })
                .collect();
            let idx: Vec<usize> = match p["indices"].as_u64().and_then(|i| accessor(i as usize)) {
                Some((b, st, sd, n, ct, _)) => (0..n)
                    .filter_map(|k| {
                        let o = st + k * sd;
                        Some(match ct {
                            5121 => *b.get(o)? as usize,
                            5123 => u16::from_le_bytes([*b.get(o)?, *b.get(o + 1)?]) as usize,
                            _ => u32::from_le_bytes([*b.get(o)?, *b.get(o + 1)?, *b.get(o + 2)?, *b.get(o + 3)?]) as usize,
                        })
                    })
                    .collect(),
                None => (0..pos.len()).collect(),
            };
            for c in idx.chunks_exact(3) {
                if let (Some(a), Some(b), Some(cc)) = (pos.get(c[0]), pos.get(c[1]), pos.get(c[2])) {
                    tris.push([*a, *b, *cc]);
                }
            }
            if tris.len() > 3_000_000 {
                return;
            }
        }
    };
    // Szene durchlaufen (mit Knoten-Transformationen); ohne Szene alle Netze direkt
    let roots: Vec<usize> = {
        let s = j["scene"].as_u64().unwrap_or(0) as usize;
        j["scenes"][s]["nodes"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|v| v as usize)).collect()).unwrap_or_default()
    };
    if roots.is_empty() {
        for m in 0..j["meshes"].as_array().map_or(0, Vec::len) {
            prim(m, &IDENT, &mut tris);
        }
    } else {
        let mut stack: Vec<(usize, Mat, usize)> = roots.into_iter().map(|r| (r, IDENT, 0)).collect();
        while let Some((n, parent, depth)) = stack.pop() {
            let node = &j["nodes"][n];
            let m = mat_mul(&parent, &node_mat(node));
            if let Some(mesh) = node["mesh"].as_u64() {
                prim(mesh as usize, &m, &mut tris);
            }
            if depth < 64 {
                for c in node["children"].as_array().cloned().unwrap_or_default() {
                    if let Some(c) = c.as_u64() {
                        stack.push((c as usize, m, depth + 1));
                    }
                }
            }
        }
    }
    Ok(tris)
}

/// Modell von schräg oben zeichnen (Tiefenpuffer, weiche Beleuchtung, geglättete Kanten); `yaw`/`pitch` in Grad
pub fn render_model(tris: &[[V3; 3]], w: usize, h: usize, yaw: f32, pitch: f32, accent: [u8; 3]) -> ColorImage {
    let ss = 2; // Kantenglättung: doppelt so groß zeichnen, dann verkleinern
    let (cw, ch) = (w * ss, h * ss);
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for t in tris {
        for p in t {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    let c = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
    let (sy, cy, sp, cp) = (yaw.to_radians().sin(), yaw.to_radians().cos(), pitch.to_radians().sin(), pitch.to_radians().cos());
    let view = |p: V3| -> V3 {
        let (x, y, z) = (p[0] - c[0], p[1] - c[1], p[2] - c[2]);
        let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
        let (y, z) = (y * cp - z * sp, y * sp + z * cp);
        [x, y, z]
    };
    // Größe aus der gedrehten Hülle
    let mut ext = 1e-6f32;
    for t in tris {
        for p in t {
            let v = view(*p);
            ext = ext.max(v[0].abs() / cw as f32).max(v[1].abs() / ch as f32);
        }
    }
    let scale = 0.44 / ext;
    let mut z = vec![f32::MIN; cw * ch];
    let mut col = vec![[0u8; 4]; cw * ch];
    let light = {
        let l = [-0.45f32, 0.75, 0.5];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        l.map(|x| x / n)
    };
    for t in tris {
        let v = t.map(view);
        let e1 = [v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]];
        let e2 = [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]];
        let mut n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len <= 0.0 {
            continue;
        }
        n = n.map(|x| x / len);
        let d = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).abs();
        let rim = 1.0 - n[2].abs();
        let shade = (0.28 + 0.62 * d + 0.18 * rim).min(1.0);
        let rgb = [0, 1, 2].map(|k| ((accent[k] as f32 * 0.55 + 200.0 * 0.45) * shade).min(255.0) as u8);
        let s = v.map(|p| [cw as f32 / 2.0 + p[0] * scale, ch as f32 / 2.0 - p[1] * scale, p[2]]);
        let (x0, x1) = (s.iter().map(|p| p[0]).fold(f32::MAX, f32::min).max(0.0) as usize, (s.iter().map(|p| p[0]).fold(f32::MIN, f32::max).ceil() as usize).min(cw - 1));
        let (y0, y1) = (s.iter().map(|p| p[1]).fold(f32::MAX, f32::min).max(0.0) as usize, (s.iter().map(|p| p[1]).fold(f32::MIN, f32::max).ceil() as usize).min(ch - 1));
        let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        if area.abs() < 1e-9 || x0 > x1 || y0 > y1 {
            continue;
        }
        for py in y0..=y1 {
            for px in x0..=x1 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let w0 = ((s[1][0] - fx) * (s[2][1] - fy) - (s[2][0] - fx) * (s[1][1] - fy)) / area;
                let w1 = ((s[2][0] - fx) * (s[0][1] - fy) - (s[0][0] - fx) * (s[2][1] - fy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let depth = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
                let i = py * cw + px;
                if depth > z[i] {
                    z[i] = depth;
                    col[i] = [rgb[0], rgb[1], rgb[2], 255];
                }
            }
        }
    }
    // verkleinern (Mittelwert je 2×2)
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            for dy in 0..ss {
                for dx in 0..ss {
                    let p = col[(y * ss + dy) * cw + x * ss + dx];
                    let a = p[3] as u32;
                    for k in 0..3 {
                        acc[k] += p[k] as u32 * a / 255;
                    }
                    acc[3] += a;
                }
            }
            let n = (ss * ss) as u32;
            let o = (y * w + x) * 4;
            for k in 0..4 {
                out[o + k] = (acc[k] / n) as u8;
            }
        }
    }
    ColorImage::from_rgba_premultiplied([w, h], &out)
}

/// Eckdaten zum Modell
pub fn model_info(tris: &[[V3; 3]]) -> Vec<(String, String)> {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for t in tris {
        for p in t {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    vec![
        ("Dreiecke".into(), tris.len().to_string()),
        ("Abmessungen".into(), format!("{:.2} × {:.2} × {:.2}", hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2])),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obj_cube_renders() {
        let src = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\nf 1 2 3 4\nf 5 6 7 8\nf 1 2 6 5\nf 4 3 7 8\nf 1 4 8 5\nf 2 3 7 6\n";
        let t = obj(src);
        assert_eq!(t.len(), 12);
        let img = render_model(&t, 64, 48, 35.0, 25.0, [80, 140, 255]);
        assert!(img.pixels.iter().any(|p| p.a() > 0));
    }

    #[test]
    fn pptx_text() {
        let x = r#"<p:sp><a:t>Hallo</a:t><a:tab/><a:t>Welt &amp; mehr</a:t></p:sp>"#;
        assert_eq!(xml_texts(x, "a:t"), vec!["Hallo", "Welt & mehr"]);
    }
}
