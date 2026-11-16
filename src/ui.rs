//! Bedienoberfläche im Stil des FoldexViewer: Vektor-Symbole (keine Schrift-Glyphen, daher nie „Kästchen“)
//! und die zugehörigen Knöpfe, Menüeinträge und Trenner. Farben kommen aus dem aktiven Theme.

use crate::themes::Theme;
use eframe::egui::{self, pos2, vec2, Color32, FontId, Painter, Pos2, Rect, Response, Rounding, Sense, Shape, Stroke, Ui};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ic {
    Back,
    Forward,
    Up,
    Refresh,
    Search,
    ChevR,
    ChevD,
    Close,
    Pc,
    Desktop,
    Download,
    Docs,
    Pictures,
    Music,
    Video,
    Home,
    Grid,
    List,
    Cut,
    Copy,
    Paste,
    Trash,
    Rename,
    NewFolder,
    Plus,
    Check,
    Pin,
    Info,
    Gear,
    Open,
    Sort,
    Eye,
    Terminal,
    Git,
    Branch,
    Window,
    Tools,
    Command,
    Clock,
    Star,
    Undo,
    Redo,
    Select,
    Compare,
    Chart,
    Code,
    Keyboard,
    Push,
    Sidebar,
    Share,
    Split,
    Spark,
}

pub fn draw(p: &Painter, r: Rect, ic: Ic, col: Color32) {
    let s = r.width().min(r.height());
    let o = r.center() - vec2(s * 0.5, s * 0.5);
    let pt = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
    let w = (s * 0.085).max(1.3);
    let st = Stroke::new(w, col);

    let line = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), st));
    };
    let closed = |pts: &[(f32, f32)]| {
        p.add(Shape::closed_line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), st));
    };
    let seg = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([pt(a.0, a.1), pt(b.0, b.1)], st);
    };
    let rr = |x0: f32, y0: f32, x1: f32, y1: f32, rad: f32| {
        p.rect_stroke(
            Rect::from_min_max(pt(x0, y0), pt(x1, y1)),
            Rounding::same(rad * s),
            st,
        );
    };
    let circ = |x: f32, y: f32, rad: f32| {
        p.circle_stroke(pt(x, y), rad * s, st);
    };
    let dot = |x: f32, y: f32, rad: f32| {
        p.circle_filled(pt(x, y), rad * s, col);
    };

    match ic {
        Ic::Back => {
            seg((0.84, 0.5), (0.18, 0.5));
            line(&[(0.46, 0.22), (0.18, 0.5), (0.46, 0.78)]);
        }
        Ic::Forward => {
            seg((0.16, 0.5), (0.82, 0.5));
            line(&[(0.54, 0.22), (0.82, 0.5), (0.54, 0.78)]);
        }
        Ic::Up => {
            seg((0.5, 0.84), (0.5, 0.18));
            line(&[(0.22, 0.46), (0.5, 0.18), (0.78, 0.46)]);
        }
        Ic::Refresh => {
            let rad = 0.3f32;
            let n = 24;
            let a0 = 35f32.to_radians();
            let a1 = 325f32.to_radians();
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = a0 + (a1 - a0) * i as f32 / n as f32;
                    pt(0.5 + rad * a.cos(), 0.5 + rad * a.sin())
                })
                .collect();
            p.add(Shape::line(pts, st));
            let a = a1;
            let base = (0.5 + rad * a.cos(), 0.5 + rad * a.sin());
            let tip = (base.0 - a.sin() * 0.16, base.1 + a.cos() * 0.16);
            let perp = (a.cos(), a.sin());
            p.add(Shape::convex_polygon(
                vec![
                    pt(tip.0, tip.1),
                    pt(base.0 + perp.0 * 0.13, base.1 + perp.1 * 0.13),
                    pt(base.0 - perp.0 * 0.13, base.1 - perp.1 * 0.13),
                ],
                col,
                Stroke::NONE,
            ));
        }
        Ic::Search => {
            circ(0.43, 0.43, 0.27);
            seg((0.63, 0.63), (0.88, 0.88));
        }
        Ic::ChevR => line(&[(0.36, 0.2), (0.64, 0.5), (0.36, 0.8)]),
        Ic::ChevD => line(&[(0.2, 0.36), (0.5, 0.64), (0.8, 0.36)]),
        Ic::Close => {
            seg((0.24, 0.24), (0.76, 0.76));
            seg((0.76, 0.24), (0.24, 0.76));
        }
        Ic::Pc => {
            rr(0.1, 0.16, 0.9, 0.66, 0.06);
            seg((0.5, 0.66), (0.5, 0.8));
            seg((0.3, 0.84), (0.7, 0.84));
        }
        Ic::Desktop => {
            rr(0.1, 0.16, 0.9, 0.7, 0.06);
            seg((0.1, 0.3), (0.9, 0.3));
            seg((0.5, 0.7), (0.5, 0.82));
            seg((0.3, 0.84), (0.7, 0.84));
        }
        Ic::Download => {
            seg((0.5, 0.14), (0.5, 0.6));
            line(&[(0.28, 0.4), (0.5, 0.62), (0.72, 0.4)]);
            line(&[(0.18, 0.66), (0.18, 0.86), (0.82, 0.86), (0.82, 0.66)]);
        }
        Ic::Docs => {
            closed(&[(0.22, 0.1), (0.6, 0.1), (0.8, 0.3), (0.8, 0.9), (0.22, 0.9)]);
            seg((0.34, 0.52), (0.68, 0.52));
            seg((0.34, 0.68), (0.68, 0.68));
        }
        Ic::Pictures => {
            rr(0.1, 0.2, 0.9, 0.8, 0.07);
            line(&[(0.16, 0.72), (0.4, 0.46), (0.55, 0.62), (0.68, 0.5), (0.84, 0.72)]);
            circ(0.7, 0.36, 0.06);
        }
        Ic::Music => {
            dot(0.34, 0.74, 0.12);
            seg((0.44, 0.74), (0.44, 0.2));
            line(&[(0.44, 0.2), (0.78, 0.3), (0.78, 0.5)]);
        }
        Ic::Video => {
            rr(0.1, 0.22, 0.9, 0.78, 0.07);
            p.add(Shape::convex_polygon(
                vec![pt(0.42, 0.36), pt(0.42, 0.64), pt(0.66, 0.5)],
                col,
                Stroke::NONE,
            ));
        }
        Ic::Home => {
            line(&[(0.1, 0.5), (0.5, 0.14), (0.9, 0.5)]);
            line(&[(0.2, 0.42), (0.2, 0.86), (0.8, 0.86), (0.8, 0.42)]);
            line(&[(0.42, 0.86), (0.42, 0.6), (0.58, 0.6), (0.58, 0.86)]);
        }
        Ic::Grid => {
            rr(0.14, 0.14, 0.44, 0.44, 0.04);
            rr(0.56, 0.14, 0.86, 0.44, 0.04);
            rr(0.14, 0.56, 0.44, 0.86, 0.04);
            rr(0.56, 0.56, 0.86, 0.86, 0.04);
        }
        Ic::List => {
            for y in [0.24f32, 0.5, 0.76] {
                dot(0.17, y, 0.04);
                seg((0.32, y), (0.86, y));
            }
        }
        Ic::Cut => {
            circ(0.3, 0.76, 0.12);
            circ(0.7, 0.76, 0.12);
            seg((0.36, 0.66), (0.7, 0.12));
            seg((0.64, 0.66), (0.3, 0.12));
        }
        Ic::Copy => {
            rr(0.32, 0.3, 0.84, 0.9, 0.06);
            line(&[(0.2, 0.7), (0.16, 0.7), (0.16, 0.1), (0.56, 0.1), (0.56, 0.3)]);
        }
        Ic::Paste => {
            rr(0.18, 0.18, 0.82, 0.9, 0.07);
            p.rect_filled(
                Rect::from_min_max(pt(0.36, 0.08), pt(0.64, 0.28)),
                Rounding::same(0.05 * s),
                col,
            );
            seg((0.32, 0.54), (0.68, 0.54));
            seg((0.32, 0.7), (0.6, 0.7));
        }
        Ic::Trash => {
            seg((0.16, 0.28), (0.84, 0.28));
            line(&[(0.4, 0.28), (0.4, 0.14), (0.6, 0.14), (0.6, 0.28)]);
            line(&[(0.24, 0.28), (0.3, 0.88), (0.7, 0.88), (0.76, 0.28)]);
            seg((0.45, 0.42), (0.45, 0.74));
            seg((0.55, 0.42), (0.55, 0.74));
        }
        Ic::Rename => {
            seg((0.5, 0.14), (0.5, 0.86));
            seg((0.34, 0.14), (0.66, 0.14));
            seg((0.34, 0.86), (0.66, 0.86));
        }
        Ic::NewFolder => {
            closed(&[(0.08, 0.2), (0.38, 0.2), (0.46, 0.32), (0.92, 0.32), (0.92, 0.82), (0.08, 0.82)]);
            seg((0.5, 0.46), (0.5, 0.7));
            seg((0.38, 0.58), (0.62, 0.58));
        }
        Ic::Plus => {
            seg((0.5, 0.18), (0.5, 0.82));
            seg((0.18, 0.5), (0.82, 0.5));
        }
        Ic::Check => line(&[(0.2, 0.54), (0.42, 0.76), (0.82, 0.28)]),
        Ic::Pin => {
            dot(0.5, 0.34, 0.17);
            seg((0.5, 0.5), (0.5, 0.9));
        }
        Ic::Info => {
            circ(0.5, 0.5, 0.38);
            dot(0.5, 0.3, 0.04);
            seg((0.5, 0.44), (0.5, 0.72));
        }
        Ic::Gear => {
            circ(0.5, 0.5, 0.17);
            for k in 0..8 {
                let a = (k as f32) * std::f32::consts::FRAC_PI_4;
                seg(
                    (0.5 + 0.27 * a.cos(), 0.5 + 0.27 * a.sin()),
                    (0.5 + 0.38 * a.cos(), 0.5 + 0.38 * a.sin()),
                );
            }
        }
        Ic::Open => {
            line(&[(0.46, 0.2), (0.16, 0.2), (0.16, 0.84), (0.8, 0.84), (0.8, 0.54)]);
            seg((0.5, 0.5), (0.86, 0.14));
            line(&[(0.6, 0.14), (0.86, 0.14), (0.86, 0.4)]);
        }
        Ic::Sort => {
            seg((0.3, 0.16), (0.3, 0.84));
            line(&[(0.14, 0.68), (0.3, 0.84), (0.46, 0.68)]);
            seg((0.62, 0.3), (0.88, 0.3));
            seg((0.62, 0.5), (0.8, 0.5));
            seg((0.62, 0.7), (0.72, 0.7));
        }
        Ic::Eye => {
            line(&[(0.08, 0.5), (0.3, 0.3), (0.5, 0.24), (0.7, 0.3), (0.92, 0.5)]);
            line(&[(0.08, 0.5), (0.3, 0.7), (0.5, 0.76), (0.7, 0.7), (0.92, 0.5)]);
            circ(0.5, 0.5, 0.11);
        }
        Ic::Terminal => {
            rr(0.08, 0.16, 0.92, 0.84, 0.07);
            line(&[(0.24, 0.36), (0.38, 0.5), (0.24, 0.64)]);
            seg((0.46, 0.66), (0.7, 0.66));
        }
        Ic::Git => {
            dot(0.3, 0.22, 0.07);
            dot(0.3, 0.78, 0.07);
            dot(0.72, 0.4, 0.07);
            seg((0.3, 0.3), (0.3, 0.7));
            line(&[(0.72, 0.48), (0.72, 0.56), (0.36, 0.72)]);
        }
        Ic::Branch => {
            circ(0.3, 0.2, 0.08);
            circ(0.3, 0.8, 0.08);
            circ(0.72, 0.3, 0.08);
            seg((0.3, 0.28), (0.3, 0.72));
            line(&[(0.72, 0.38), (0.72, 0.5), (0.36, 0.7)]);
        }
        Ic::Window => {
            rr(0.1, 0.16, 0.9, 0.84, 0.06);
            seg((0.1, 0.32), (0.9, 0.32));
        }
        Ic::Tools => {
            seg((0.2, 0.8), (0.58, 0.42));
            circ(0.68, 0.32, 0.16);
        }
        Ic::Command => {
            rr(0.12, 0.2, 0.88, 0.8, 0.08);
            line(&[(0.3, 0.4), (0.42, 0.5), (0.3, 0.6)]);
            seg((0.5, 0.6), (0.68, 0.6));
        }
        Ic::Clock => {
            circ(0.5, 0.5, 0.36);
            line(&[(0.5, 0.28), (0.5, 0.5), (0.66, 0.6)]);
        }
        Ic::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                    let r = if i % 2 == 0 { 0.4 } else { 0.17 };
                    (0.5 + r * a.cos(), 0.54 + r * a.sin())
                })
                .collect();
            closed(&pts);
        }
        Ic::Undo => {
            line(&[(0.34, 0.2), (0.16, 0.38), (0.34, 0.56)]);
            line(&[(0.16, 0.38), (0.62, 0.38), (0.76, 0.46), (0.84, 0.6), (0.76, 0.74), (0.6, 0.82), (0.36, 0.82)]);
        }
        Ic::Redo => {
            line(&[(0.66, 0.2), (0.84, 0.38), (0.66, 0.56)]);
            line(&[(0.84, 0.38), (0.38, 0.38), (0.24, 0.46), (0.16, 0.6), (0.24, 0.74), (0.4, 0.82), (0.64, 0.82)]);
        }
        Ic::Select => {
            rr(0.14, 0.14, 0.86, 0.86, 0.08);
            line(&[(0.3, 0.52), (0.44, 0.66), (0.7, 0.36)]);
        }
        Ic::Compare => {
            rr(0.08, 0.18, 0.44, 0.82, 0.05);
            rr(0.56, 0.18, 0.92, 0.82, 0.05);
            seg((0.16, 0.38), (0.36, 0.38));
            seg((0.64, 0.38), (0.84, 0.38));
            seg((0.16, 0.56), (0.32, 0.56));
            seg((0.64, 0.56), (0.8, 0.56));
        }
        Ic::Chart => {
            rr(0.14, 0.52, 0.32, 0.86, 0.03);
            rr(0.41, 0.3, 0.59, 0.86, 0.03);
            rr(0.68, 0.14, 0.86, 0.86, 0.03);
        }
        Ic::Code => {
            line(&[(0.34, 0.26), (0.12, 0.5), (0.34, 0.74)]);
            line(&[(0.66, 0.26), (0.88, 0.5), (0.66, 0.74)]);
            seg((0.56, 0.2), (0.44, 0.8));
        }
        Ic::Keyboard => {
            rr(0.06, 0.24, 0.94, 0.76, 0.07);
            for (x, y) in [(0.22, 0.4), (0.38, 0.4), (0.54, 0.4), (0.7, 0.4), (0.22, 0.54), (0.78, 0.54)] {
                dot(x, y, 0.035);
            }
            seg((0.34, 0.62), (0.66, 0.62));
        }
        Ic::Push => {
            seg((0.5, 0.84), (0.5, 0.3));
            line(&[(0.28, 0.5), (0.5, 0.28), (0.72, 0.5)]);
            seg((0.18, 0.14), (0.82, 0.14));
        }
        Ic::Spark => spark(p, Rect::from_min_size(o, vec2(s, s)), col),
        Ic::Share => {
            // drei verbundene Punkte
            seg((0.3, 0.5), (0.7, 0.24));
            seg((0.3, 0.5), (0.7, 0.76));
            dot(0.28, 0.5, 0.12);
            dot(0.72, 0.22, 0.12);
            dot(0.72, 0.78, 0.12);
        }
        Ic::Split => {
            rr(0.1, 0.2, 0.9, 0.8, 0.07);
            seg((0.5, 0.2), (0.5, 0.8));
        }
        Ic::Sidebar => {
            rr(0.1, 0.2, 0.9, 0.8, 0.07);
            p.rect_filled(Rect::from_min_max(pt(0.1, 0.2), pt(0.36, 0.8)), Rounding::same(0.07 * s), col);
        }
    }
}

/// KI-Stern ✦: ein einziger, symmetrischer SVG-Pfad (vier Spitzen, nach innen gewölbte Seiten). Er wird in
/// genau der Pixelgröße gerendert und eingefärbt – ohne Nähte zwischen Teilflächen und ohne Treppenkanten.
const SPARK_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><path fill="#fff" d="M32 2C33.6 19.4 44.6 30.4 62 32C44.6 33.6 33.6 44.6 32 62C30.4 44.6 19.4 33.6 2 32C19.4 30.4 30.4 19.4 32 2Z"/></svg>"##;

fn spark(p: &Painter, r: Rect, col: Color32) {
    let ctx = p.ctx();
    let ppp = ctx.pixels_per_point();
    // auf das Pixelraster legen, damit nichts verschwimmt
    let px = (r.width() * ppp).round().max(4.0) as u32;
    let min = pos2((r.center().x * ppp - px as f32 / 2.0).round() / ppp, (r.center().y * ppp - px as f32 / 2.0).round() / ppp);
    let rect = Rect::from_min_size(min, vec2(px as f32 / ppp, px as f32 / ppp));
    let id = egui::Id::new(("fc_spark", px));
    let tex = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)).or_else(|| {
        use resvg::{tiny_skia, usvg};
        let tree = usvg::Tree::from_data(SPARK_SVG.as_bytes(), &usvg::Options::default()).ok()?;
        let mut pm = tiny_skia::Pixmap::new(px, px)?;
        let k = px as f32 / tree.size().width();
        resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
        let rgba: Vec<u8> = pm.pixels().iter().flat_map(|c| {
            let c = c.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        }).collect();
        let img = egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], &rgba);
        let t = ctx.load_texture(format!("spark{px}"), img, egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| d.insert_temp(id, t.clone()));
        Some(t)
    });
    if let Some(t) = tex {
        p.image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), col);
    }
}

// ───────────────────────────── Bedienelemente ─────────────────────────────

/// Aktives Theme für Hilfsfunktionen ohne direkten Zugriff (Menüs) – wird jedes Frame gesetzt.
pub fn set_theme(ctx: &egui::Context, th: Theme) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("fc_theme"), th));
}

pub fn theme(ui: &Ui) -> Theme {
    ui.ctx().data(|d| d.get_temp::<Theme>(egui::Id::new("fc_theme"))).expect("Theme nicht gesetzt")
}

/// Einzeiliger Text, vertikal zentriert; mit right rechtsbündig an x; zu lange Texte enden mit „…“.
#[allow(clippy::too_many_arguments)]
pub fn paint_text(ui: &Ui, x: f32, cy: f32, text: &str, size: f32, color: Color32, width: f32, right: bool) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(8.0));
    let g = ui.fonts(|f| f.layout_job(job));
    let pos = if right { pos2(x - g.size().x, cy - g.size().y / 2.0) } else { pos2(x, cy - g.size().y / 2.0) };
    ui.painter().galley(pos, g, color);
}

/// Quadratischer Symbolknopf (Navigation, Tab-Leiste).
pub fn tool_btn(ui: &mut Ui, th: &Theme, ic: Ic, tip: &str, enabled: bool, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    paint_tool(ui, th, rect, &resp, ic, enabled, active);
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}

/// Wie `tool_btn`, aber an fester Position (Tab-Leiste).
pub fn tool_btn_at(ui: &mut Ui, th: &Theme, rect: Rect, id: egui::Id, ic: Ic, tip: &str, active: bool) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    paint_tool(ui, th, rect, &resp, ic, true, active);
    resp.on_hover_text(tip)
}

fn paint_tool(ui: &Ui, th: &Theme, rect: Rect, resp: &Response, ic: Ic, enabled: bool, active: bool) {
    if active {
        ui.painter().rect_filled(rect, Rounding::same(6.0), th.sel);
    } else if enabled && resp.hovered() {
        let c = if resp.is_pointer_button_down_on() { th.press } else { th.hover };
        ui.painter().rect_filled(rect, Rounding::same(6.0), c);
    }
    let col = if enabled { th.text } else { th.sub.gamma_multiply(0.5) };
    draw(ui.painter(), Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), ic, col);
}

/// Befehlsknopf mit optionalem Symbol, Beschriftung und Aufklapp-Pfeil.
pub fn cmd_btn(ui: &mut Ui, th: &Theme, ic: Option<Ic>, label: &str, enabled: bool, chevron: bool) -> Response {
    let font = FontId::proportional(14.0);
    let tw = if label.is_empty() { 0.0 } else { ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font.clone(), th.text)).size().x };
    let mut w = 20.0 + tw;
    if ic.is_some() {
        w += 26.0;
    }
    if chevron {
        w += 18.0;
    }
    if label.is_empty() && !chevron {
        w = 40.0;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        let c = if resp.is_pointer_button_down_on() { th.press } else { th.hover };
        ui.painter().rect_filled(rect, Rounding::same(6.0), c);
    }
    let col = if enabled { th.text } else { th.sub.gamma_multiply(0.5) };
    let mut x = rect.min.x + 10.0;
    if label.is_empty() && !chevron {
        x = rect.center().x - 9.0;
    }
    if let Some(ic) = ic {
        let icol = if enabled && ic == Ic::Plus { th.accent } else { col };
        draw(ui.painter(), Rect::from_center_size(pos2(x + 9.0, rect.center().y), vec2(18.0, 18.0)), ic, icol);
        x += 26.0;
    }
    if !label.is_empty() {
        ui.painter().text(pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, label, font, col);
        x += tw;
    }
    if chevron {
        draw(ui.painter(), Rect::from_center_size(pos2(x + 12.0, rect.center().y), vec2(10.0, 10.0)), Ic::ChevD, th.sub);
    }
    resp
}

/// Senkrechter Trenner zwischen Knopfgruppen.
pub fn sep_v(ui: &mut Ui, th: &Theme) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 32.0), Sense::hover());
    ui.painter().line_segment(
        [pos2(rect.center().x, rect.min.y + 7.0), pos2(rect.center().x, rect.max.y - 7.0)],
        Stroke::new(1.0, th.line),
    );
}

/// Menüeintrag: Symbol bzw. Häkchen links, Text, Tastenkürzel rechts. Schließt das Menü beim Klick.
pub fn menu_entry(ui: &mut Ui, ic: Option<Ic>, label: &str, shortcut: &str, enabled: bool, checked: bool) -> bool {
    let th = theme(ui);
    let w = ui.available_width().max(250.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(2.0, 1.0)), Rounding::same(5.0), th.hover);
    }
    let col = if enabled { th.text } else { th.sub.gamma_multiply(0.6) };
    let ir = Rect::from_center_size(pos2(rect.min.x + 18.0, rect.center().y), vec2(16.0, 16.0));
    if checked {
        draw(ui.painter(), ir.shrink(1.0), Ic::Check, th.accent);
    } else if let Some(ic) = ic {
        draw(ui.painter(), ir, ic, col);
    }
    let sw = if shortcut.is_empty() { 0.0 } else { 120.0 };
    paint_text(ui, rect.min.x + 38.0, rect.center().y, label, 14.0, col, w - 50.0 - sw, false);
    if !shortcut.is_empty() {
        paint_text(ui, rect.max.x - 12.0, rect.center().y, shortcut, 12.5, th.sub, sw - 10.0, true);
    }
    let c = resp.clicked();
    if c {
        ui.close_menu();
    }
    c
}

/// Auswahlfeld im Stil der übrigen Menüs: feste Breite, langer Text endet mit „…“, die Liste hat dieselbe Breite
/// (lange Einträge werden gekürzt statt das Layout zu vergrößern) und scrollt bei vielen Einträgen.
/// `items`: (Wert, Beschriftung, Hinweis rechts). Rückgabe: gewählter Wert.
pub fn select_field<T: Clone + PartialEq>(ui: &mut Ui, th: &Theme, id: impl std::hash::Hash, width: f32, current: &str, items: &[(T, String, String)], selected: Option<&T>) -> Option<T> {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    let hot = resp.hovered();
    ui.painter().rect(rect, Rounding::same(6.0), if hot { th.hover } else { th.field }, Stroke::new(1.0, th.line));
    paint_text(ui, rect.min.x + 10.0, rect.center().y, current, 13.5, th.text, rect.width() - 36.0, false);
    draw(ui.painter(), Rect::from_center_size(pos2(rect.max.x - 15.0, rect.center().y), vec2(12.0, 12.0)), Ic::ChevD, th.sub);
    let pid = ui.id().with(("select_field", id));
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(pid));
    }
    let mut out = None;
    egui::popup_below_widget(ui, pid, &resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        let w = width.max(250.0);
        ui.set_min_width(w);
        ui.set_max_width(w);
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            if items.is_empty() {
                menu_label(ui, "keine Einträge");
            }
            for (v, label, hint) in items {
                if menu_entry(ui, None, label, hint, true, selected == Some(v)) {
                    out = Some(v.clone());
                }
            }
        });
    });
    if out.is_some() {
        ui.memory_mut(|m| m.close_popup());
    }
    out
}

pub fn menu_sep(ui: &mut Ui) {
    let th = theme(ui);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().max(250.0), 9.0), Sense::hover());
    ui.painter().line_segment([pos2(r.min.x + 8.0, r.center().y), pos2(r.max.x - 8.0, r.center().y)], Stroke::new(1.0, th.line));
}

pub fn menu_label(ui: &mut Ui, text: &str) {
    let th = theme(ui);
    let w = ui.available_width().max(250.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    paint_text(ui, r.min.x + 12.0, r.center().y, text, 12.0, th.sub, w, false);
}
