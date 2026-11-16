//! Vektor-Icons für Bild- und Audioeditor (gezeichnet wie die übrigen Icons in ui.rs: Einheitskoordinaten
//! 0..1, Strichstärke ≈ 8,5 % der Größe) und die passenden Knöpfe: Werkzeug (quadratisch, aktiv hervorgehoben),
//! Icon-Knopf und Icon mit Beschriftung. Jeder Knopf hat einen Tooltip.

use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, Painter, Pos2, Rect, Response, Rounding, Sense, Shape, Stroke, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum E {
    Move,
    Select,
    Crop,
    Brush,
    Eraser,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Fill,
    Picker,
    Blur,
    Pixelate,
    Undo,
    Redo,
    ZoomIn,
    ZoomOut,
    Fit,
    Actual,
    Save,
    SaveAs,
    Close,
    RotL,
    RotR,
    FlipH,
    FlipV,
    Swap,
    Copy,
    Trash,
    Check,
    Play,
    Pause,
    ToStart,
    Keep,
    Cut,
    SelectAll,
    ZoomSel,
    ZoomAll,
    Resize,
    Sliders,
    Sparkle,
    // lokale KI
    Wand,
    Subject,
    Palette,
    Focus,
    Upscale,
    Invert,
}

pub fn draw(p: &Painter, r: Rect, e: E, col: Color32) {
    let s = r.width().min(r.height());
    let o = r.center() - vec2(s * 0.5, s * 0.5);
    let pt = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
    let w = (s * 0.085).max(1.3);
    let st = Stroke::new(w, col);
    let thin = Stroke::new((w * 0.7).max(1.0), col);
    let line = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), st));
    };
    let closed = |pts: &[(f32, f32)]| {
        p.add(Shape::closed_line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), st));
    };
    let fill = |pts: &[(f32, f32)]| {
        p.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| pt(x, y)).collect(), col, Stroke::NONE));
    };
    let seg = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([pt(a.0, a.1), pt(b.0, b.1)], st);
    };
    let rr = |x0: f32, y0: f32, x1: f32, y1: f32, rad: f32| {
        p.rect_stroke(Rect::from_min_max(pt(x0, y0), pt(x1, y1)), Rounding::same(rad * s), st);
    };
    let rf = |x0: f32, y0: f32, x1: f32, y1: f32| {
        p.rect_filled(Rect::from_min_max(pt(x0, y0), pt(x1, y1)), Rounding::same(0.03 * s), col);
    };
    let circ = |x: f32, y: f32, rad: f32| {
        p.circle_stroke(pt(x, y), rad * s, st);
    };
    let dot = |x: f32, y: f32, rad: f32| {
        p.circle_filled(pt(x, y), rad * s, col);
    };
    // Bogen um (cx, cy) von a0 bis a1 (Grad), optional mit Pfeilspitze am Ende
    let arc = |cx: f32, cy: f32, rad: f32, a0: f32, a1: f32, head: bool| {
        let n = 20;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                pt(cx + rad * a.cos(), cy + rad * a.sin())
            })
            .collect();
        p.add(Shape::line(pts, st));
        if head {
            let a = a1.to_radians();
            let dir = if a1 > a0 { 1.0 } else { -1.0 };
            let tip = (cx + rad * a.cos(), cy + rad * a.sin());
            // Tangente in Laufrichtung
            let t = (-a.sin() * dir, a.cos() * dir);
            let n = (a.cos(), a.sin());
            let k = 0.13;
            fill(&[(tip.0 + t.0 * k, tip.1 + t.1 * k), (tip.0 - n.0 * k * 0.8, tip.1 - n.1 * k * 0.8), (tip.0 + n.0 * k * 0.8, tip.1 + n.1 * k * 0.8)]);
        }
    };
    let dashed_rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let edges = [((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))];
        for (a, b) in edges {
            let n = 4;
            for k in 0..n {
                let (t0, t1) = (k as f32 / n as f32, (k as f32 + 0.55) / n as f32);
                p.line_segment([pt(a.0 + (b.0 - a.0) * t0, a.1 + (b.1 - a.1) * t0), pt(a.0 + (b.0 - a.0) * t1, a.1 + (b.1 - a.1) * t1)], thin);
            }
        }
    };
    let magnifier = |p2: &dyn Fn()| {
        circ(0.42, 0.42, 0.25);
        seg((0.6, 0.6), (0.86, 0.86));
        p2();
    };

    match e {
        E::Move => {
            seg((0.5, 0.1), (0.5, 0.9));
            seg((0.1, 0.5), (0.9, 0.5));
            for (a, b, c) in [((0.36, 0.24), (0.5, 0.1), (0.64, 0.24)), ((0.36, 0.76), (0.5, 0.9), (0.64, 0.76)), ((0.24, 0.36), (0.1, 0.5), (0.24, 0.64)), ((0.76, 0.36), (0.9, 0.5), (0.76, 0.64))] {
                line(&[a, b, c]);
            }
        }
        E::Select => dashed_rect(0.14, 0.2, 0.86, 0.8),
        E::Crop => {
            line(&[(0.28, 0.08), (0.28, 0.72), (0.92, 0.72)]);
            line(&[(0.08, 0.28), (0.72, 0.28), (0.72, 0.92)]);
        }
        E::Brush => {
            // Stiel und Borsten
            seg((0.86, 0.12), (0.48, 0.52));
            fill(&[(0.5, 0.44), (0.58, 0.52), (0.38, 0.74), (0.28, 0.66)]);
            p.add(Shape::convex_polygon(vec![pt(0.3, 0.64), pt(0.4, 0.74), pt(0.22, 0.9), pt(0.1, 0.9), pt(0.12, 0.8)], col, Stroke::NONE));
        }
        E::Eraser => {
            closed(&[(0.52, 0.14), (0.88, 0.5), (0.5, 0.88), (0.14, 0.52)]);
            fill(&[(0.33, 0.33), (0.69, 0.69), (0.5, 0.88), (0.14, 0.52)]);
            seg((0.42, 0.88), (0.9, 0.88));
        }
        E::Line => {
            seg((0.18, 0.82), (0.82, 0.18));
            dot(0.18, 0.82, 0.08);
            dot(0.82, 0.18, 0.08);
        }
        E::Arrow => {
            seg((0.16, 0.84), (0.78, 0.22));
            fill(&[(0.86, 0.14), (0.52, 0.24), (0.76, 0.48)]);
        }
        E::Rect => rr(0.14, 0.24, 0.86, 0.76, 0.04),
        E::Ellipse => {
            let pts: Vec<Pos2> = (0..40).map(|i| {
                let a = i as f32 / 40.0 * std::f32::consts::TAU;
                pt(0.5 + 0.38 * a.cos(), 0.5 + 0.28 * a.sin())
            }).collect();
            p.add(Shape::closed_line(pts, st));
        }
        E::Text => {
            seg((0.18, 0.18), (0.82, 0.18));
            seg((0.5, 0.18), (0.5, 0.86));
            seg((0.18, 0.18), (0.18, 0.3));
            seg((0.82, 0.18), (0.82, 0.3));
            seg((0.38, 0.86), (0.62, 0.86));
        }
        E::Fill => {
            // Eimer, gekippt, mit Tropfen
            closed(&[(0.2, 0.46), (0.48, 0.18), (0.8, 0.5), (0.52, 0.78)]);
            seg((0.2, 0.46), (0.8, 0.5));
            p.add(Shape::convex_polygon(vec![pt(0.86, 0.6), pt(0.92, 0.78), pt(0.86, 0.86), pt(0.8, 0.78)], col, Stroke::NONE));
        }
        E::Picker => {
            seg((0.2, 0.8), (0.58, 0.42));
            p.add(Shape::convex_polygon(vec![pt(0.52, 0.36), pt(0.64, 0.48), pt(0.84, 0.28), pt(0.72, 0.16)], col, Stroke::NONE));
            dot(0.18, 0.82, 0.06);
        }
        E::Blur => {
            // Tropfen: Spitze oben, runder Bauch
            let full: Vec<Pos2> = std::iter::once(pt(0.5, 0.1))
                .chain((0..=32).map(|i| {
                    let a = (-30.0 + 240.0 * i as f32 / 32.0).to_radians();
                    pt(0.5 + 0.27 * a.cos(), 0.6 + 0.27 * a.sin())
                }))
                .collect();
            p.add(Shape::closed_line(full, st));
            arc(0.5, 0.6, 0.13, 100.0, 170.0, false);
        }
        E::Pixelate => {
            for y in 0..3 {
                for x in 0..3 {
                    let (x0, y0) = (0.14 + x as f32 * 0.24, 0.14 + y as f32 * 0.24);
                    if (x + y) % 2 == 0 {
                        rf(x0, y0, x0 + 0.22, y0 + 0.22);
                    } else {
                        rr(x0 + 0.02, y0 + 0.02, x0 + 0.2, y0 + 0.2, 0.02);
                    }
                }
            }
        }
        E::Undo => arc(0.52, 0.56, 0.28, 10.0, 200.0, true),
        E::Redo => arc(0.48, 0.56, 0.28, 170.0, -20.0, true),
        E::ZoomIn => magnifier(&|| {
            seg((0.3, 0.42), (0.54, 0.42));
            seg((0.42, 0.3), (0.42, 0.54));
        }),
        E::ZoomOut => magnifier(&|| seg((0.3, 0.42), (0.54, 0.42))),
        E::Fit => {
            for (a, b, c) in [((0.12, 0.34), (0.12, 0.12), (0.34, 0.12)), ((0.66, 0.12), (0.88, 0.12), (0.88, 0.34)), ((0.88, 0.66), (0.88, 0.88), (0.66, 0.88)), ((0.34, 0.88), (0.12, 0.88), (0.12, 0.66))] {
                line(&[a, b, c]);
            }
            rr(0.34, 0.34, 0.66, 0.66, 0.03);
        }
        E::Actual => {
            // „1:1“
            seg((0.22, 0.32), (0.3, 0.24));
            seg((0.3, 0.24), (0.3, 0.78));
            dot(0.5, 0.4, 0.045);
            dot(0.5, 0.64, 0.045);
            seg((0.7, 0.32), (0.78, 0.24));
            seg((0.78, 0.24), (0.78, 0.78));
        }
        E::Save | E::SaveAs => {
            closed(&[(0.14, 0.14), (0.72, 0.14), (0.86, 0.28), (0.86, 0.86), (0.14, 0.86)]);
            rr(0.3, 0.14, 0.62, 0.36, 0.0);
            rr(0.28, 0.56, 0.72, 0.86, 0.0);
            if e == E::SaveAs {
                // kleines Plus unten rechts
                seg((0.72, 0.72), (0.98, 0.72));
                seg((0.85, 0.59), (0.85, 0.85));
            }
        }
        E::Close => {
            seg((0.24, 0.24), (0.76, 0.76));
            seg((0.76, 0.24), (0.24, 0.76));
        }
        E::RotL => {
            rr(0.34, 0.42, 0.74, 0.86, 0.04);
            arc(0.38, 0.4, 0.26, 0.0, -150.0, true);
        }
        E::RotR => {
            rr(0.26, 0.42, 0.66, 0.86, 0.04);
            arc(0.62, 0.4, 0.26, 180.0, 330.0, true);
        }
        E::FlipH => {
            closed(&[(0.42, 0.22), (0.42, 0.78), (0.1, 0.78)]);
            fill(&[(0.58, 0.22), (0.58, 0.78), (0.9, 0.78)]);
            for k in 0..4 {
                let y = 0.1 + k as f32 * 0.22;
                p.line_segment([pt(0.5, y), pt(0.5, y + 0.12)], thin);
            }
        }
        E::FlipV => {
            closed(&[(0.22, 0.42), (0.78, 0.42), (0.78, 0.1)]);
            fill(&[(0.22, 0.58), (0.78, 0.58), (0.78, 0.9)]);
            for k in 0..4 {
                let x = 0.1 + k as f32 * 0.22;
                p.line_segment([pt(x, 0.5), pt(x + 0.12, 0.5)], thin);
            }
        }
        E::Swap => {
            seg((0.16, 0.34), (0.8, 0.34));
            line(&[(0.66, 0.2), (0.82, 0.34), (0.66, 0.48)]);
            seg((0.84, 0.66), (0.2, 0.66));
            line(&[(0.34, 0.52), (0.18, 0.66), (0.34, 0.8)]);
        }
        E::Copy => {
            rr(0.3, 0.3, 0.86, 0.86, 0.06);
            line(&[(0.14, 0.66), (0.14, 0.14), (0.66, 0.14)]);
        }
        E::Trash => {
            seg((0.14, 0.24), (0.86, 0.24));
            line(&[(0.38, 0.24), (0.4, 0.12), (0.6, 0.12), (0.62, 0.24)]);
            line(&[(0.22, 0.24), (0.28, 0.88), (0.72, 0.88), (0.78, 0.24)]);
            seg((0.42, 0.38), (0.43, 0.74));
            seg((0.58, 0.38), (0.57, 0.74));
        }
        E::Check => line(&[(0.16, 0.52), (0.4, 0.76), (0.86, 0.26)]),
        E::Play => fill(&[(0.26, 0.14), (0.86, 0.5), (0.26, 0.86)]),
        E::Pause => {
            rf(0.24, 0.16, 0.42, 0.84);
            rf(0.58, 0.16, 0.76, 0.84);
        }
        E::ToStart => {
            rf(0.18, 0.18, 0.3, 0.82);
            fill(&[(0.84, 0.18), (0.84, 0.82), (0.32, 0.5)]);
        }
        E::Keep => {
            // Klammern um einen gefüllten Bereich (Zuschneiden auf die Auswahl)
            line(&[(0.3, 0.14), (0.14, 0.14), (0.14, 0.86), (0.3, 0.86)]);
            line(&[(0.7, 0.14), (0.86, 0.14), (0.86, 0.86), (0.7, 0.86)]);
            rf(0.3, 0.32, 0.7, 0.68);
        }
        E::Cut => {
            circ(0.28, 0.74, 0.13);
            circ(0.72, 0.74, 0.13);
            seg((0.36, 0.63), (0.74, 0.12));
            seg((0.64, 0.63), (0.26, 0.12));
        }
        E::SelectAll => {
            dashed_rect(0.12, 0.18, 0.88, 0.82);
            rf(0.3, 0.36, 0.7, 0.64);
        }
        E::ZoomSel => magnifier(&|| {
            line(&[(0.32, 0.36), (0.32, 0.3), (0.38, 0.3)]);
            line(&[(0.52, 0.48), (0.52, 0.54), (0.46, 0.54)]);
        }),
        E::ZoomAll => {
            seg((0.12, 0.5), (0.88, 0.5));
            line(&[(0.28, 0.34), (0.12, 0.5), (0.28, 0.66)]);
            line(&[(0.72, 0.34), (0.88, 0.5), (0.72, 0.66)]);
            seg((0.12, 0.2), (0.12, 0.8));
            seg((0.88, 0.2), (0.88, 0.8));
        }
        E::Resize => {
            rr(0.12, 0.42, 0.58, 0.88, 0.03);
            seg((0.5, 0.5), (0.86, 0.14));
            line(&[(0.62, 0.14), (0.86, 0.14), (0.86, 0.38)]);
        }
        E::Sliders => {
            for (y, x) in [(0.24, 0.36), (0.5, 0.66), (0.76, 0.3)] {
                seg((0.12, y), (0.88, y));
                p.circle_filled(pt(x, y), 0.09 * s, col);
            }
        }
        E::Sparkle => {
            fill(&[(0.5, 0.1), (0.6, 0.4), (0.9, 0.5), (0.6, 0.6), (0.5, 0.9), (0.4, 0.6), (0.1, 0.5), (0.4, 0.4)]);
        }
        E::Wand => {
            // Zauberstab mit Funken
            seg((0.14, 0.86), (0.6, 0.4));
            p.line_segment([pt(0.6, 0.4), pt(0.7, 0.3)], Stroke::new(w * 1.8, col));
            for (x, y, r) in [(0.78, 0.14, 0.05), (0.88, 0.4, 0.035), (0.5, 0.16, 0.035)] {
                dot(x, y, r);
            }
        }
        E::Subject => {
            // Person vor gestricheltem Rahmen
            dashed_rect(0.1, 0.1, 0.9, 0.9);
            dot(0.5, 0.36, 0.11);
            arc(0.5, 0.82, 0.24, 180.0, 360.0, false);
        }
        E::Palette => {
            circ(0.5, 0.5, 0.38);
            for (x, y) in [(0.34, 0.36), (0.52, 0.27), (0.68, 0.38), (0.36, 0.58)] {
                dot(x, y, 0.06);
            }
            circ(0.62, 0.66, 0.08);
        }
        E::Focus => {
            // scharfer Kern, unscharfe Ringe
            dot(0.5, 0.5, 0.14);
            p.circle_stroke(pt(0.5, 0.5), 0.27 * s, Stroke::new(w, col.gamma_multiply(0.55)));
            p.circle_stroke(pt(0.5, 0.5), 0.39 * s, Stroke::new(w, col.gamma_multiply(0.25)));
        }
        E::Upscale => {
            rr(0.1, 0.5, 0.5, 0.9, 0.03);
            dashed_rect(0.1, 0.1, 0.9, 0.9);
            seg((0.42, 0.58), (0.8, 0.2));
            line(&[(0.58, 0.2), (0.8, 0.2), (0.8, 0.42)]);
        }
        E::Invert => {
            circ(0.5, 0.5, 0.36);
            let half: Vec<Pos2> = (0..=20).map(|i| {
                let a = (-90.0 + 180.0 * i as f32 / 20.0f32).to_radians();
                pt(0.5 + 0.36 * a.cos(), 0.5 + 0.36 * a.sin())
            }).collect();
            p.add(Shape::convex_polygon(half, col, Stroke::NONE));
        }
    }
}

/// Werkzeug-Knopf (quadratisch): aktiv mit Akzentfläche
pub fn tool(ui: &mut Ui, th: &Theme, e: E, active: bool, tip: &str) -> Response {
    let (r, resp) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::click());
    let bg = if active {
        th.accent.gamma_multiply(0.22)
    } else if resp.hovered() {
        th.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(r, 7.0, bg);
    if active {
        ui.painter().rect_stroke(r.shrink(0.5), 7.0, Stroke::new(1.0, th.accent));
    }
    draw(ui.painter(), Rect::from_center_size(r.center(), vec2(20.0, 20.0)), e, if active { th.accent } else { th.text });
    resp.on_hover_text(tip)
}

/// Icon-Knopf (30 × 30) mit Tooltip
pub fn btn(ui: &mut Ui, th: &Theme, e: E, tip: &str, enabled: bool) -> Response {
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(r, 6.0, if resp.is_pointer_button_down_on() { th.press } else { th.hover });
    }
    draw(ui.painter(), Rect::from_center_size(r.center(), vec2(17.0, 17.0)), e, if enabled { th.text } else { th.sub.gamma_multiply(0.45) });
    resp.on_hover_text(tip)
}

/// Icon mit Beschriftung; `primary` = gefüllter Akzent-Knopf (z. B. Speichern)
pub fn labeled(ui: &mut Ui, th: &Theme, e: E, label: &str, tip: &str, enabled: bool, primary: bool) -> Response {
    let font = egui::FontId::proportional(13.5);
    let tw = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font.clone(), th.text)).size().x;
    let (r, resp) = ui.allocate_exact_size(vec2(tw + 44.0, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    let (bg, fg) = if primary {
        (if enabled { if resp.hovered() { th.accent.gamma_multiply(0.85) } else { th.accent } } else { th.accent.gamma_multiply(0.35) }, Color32::WHITE)
    } else {
        (if enabled && resp.hovered() { th.hover } else { th.field }, if enabled { th.text } else { th.sub.gamma_multiply(0.5) })
    };
    ui.painter().rect_filled(r, 6.0, bg);
    if !primary {
        ui.painter().rect_stroke(r, 6.0, Stroke::new(1.0, th.line));
    }
    draw(ui.painter(), Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), vec2(16.0, 16.0)), e, fg);
    ui.painter().text(pos2(r.left() + 32.0, r.center().y), egui::Align2::LEFT_CENTER, label, font, fg);
    resp.on_hover_text(tip)
}

/// Abschnitts-Überschrift mit Icon
pub fn heading(ui: &mut Ui, th: &Theme, e: E, text: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        draw(ui.painter(), r, e, th.sub);
        ui.label(egui::RichText::new(text).strong());
    });
}
