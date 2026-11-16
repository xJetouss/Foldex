//! Kachel-Layout (Tiling): ein Baum aus Teilungen mit beliebig vielen Teilfenstern.
//! Blätter verweisen auf Pane-Nummern; jede Teilung hat Richtung und Teilungsverhältnis.

use eframe::egui::{pos2, vec2, Pos2, Rect};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// nebeneinander: `a` links, `b` rechts
    H,
    /// übereinander: `a` oben, `b` unten
    V,
}

/// Wohin ein Fenster an einem Kachelrand angedockt wird.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zone {
    Left,
    Right,
    Top,
    Bottom,
}

impl Zone {
    pub fn label(self) -> &'static str {
        match self {
            Zone::Left => "Links andocken",
            Zone::Right => "Rechts andocken",
            Zone::Top => "Oben andocken",
            Zone::Bottom => "Unten andocken",
        }
    }

    /// Der Teil von `r`, den das angedockte Fenster belegt.
    pub fn rect(self, r: Rect) -> Rect {
        let c = r.center();
        match self {
            Zone::Left => Rect::from_min_max(r.min, pos2(c.x, r.max.y)),
            Zone::Right => Rect::from_min_max(pos2(c.x, r.min.y), r.max),
            Zone::Top => Rect::from_min_max(r.min, pos2(r.max.x, c.y)),
            Zone::Bottom => Rect::from_min_max(pos2(r.min.x, c.y), r.max),
        }
    }

    /// Zone anhand der Zeigerposition in `r` (Randbereich = Andocken, Mitte = nichts).
    pub fn at(pos: Pos2, r: Rect) -> Option<Zone> {
        if !r.contains(pos) || r.width() < 1.0 || r.height() < 1.0 {
            return None;
        }
        let (fx, fy) = ((pos.x - r.left()) / r.width(), (pos.y - r.top()) / r.height());
        let (dl, dr, dt, db) = (fx, 1.0 - fx, fy, 1.0 - fy);
        let m = dl.min(dr).min(dt).min(db);
        if m > 0.3 {
            return None;
        }
        Some(if m == dl {
            Zone::Left
        } else if m == dr {
            Zone::Right
        } else if m == dt {
            Zone::Top
        } else {
            Zone::Bottom
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(usize),
    Split {
        dir: Dir,
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

/// Ziehbarer Trenner zwischen zwei Teilen.
pub struct Handle {
    pub path: Vec<bool>,
    pub dir: Dir,
    pub hit: Rect,
    pub parent: Rect,
}

pub const MIN_SIZE: f32 = 180.0;

impl Node {
    pub fn leaves(&self) -> Vec<usize> {
        let mut v = Vec::new();
        self.collect(&mut v);
        v
    }

    fn collect(&self, v: &mut Vec<usize>) {
        match self {
            Node::Leaf(i) => v.push(*i),
            Node::Split { a, b, .. } => {
                a.collect(v);
                b.collect(v);
            }
        }
    }

    pub fn count(&self) -> usize {
        self.leaves().len()
    }

    pub fn contains(&self, leaf: usize) -> bool {
        self.leaves().contains(&leaf)
    }

    /// Verteilt `r` auf die Blätter. `gap` = Abstand zwischen Kacheln.
    pub fn layout(&self, r: Rect, gap: f32, leaves: &mut Vec<(usize, Rect)>, handles: &mut Vec<Handle>) {
        self.layout_in(r, gap, &mut Vec::new(), leaves, handles);
    }

    fn layout_in(&self, r: Rect, gap: f32, path: &mut Vec<bool>, leaves: &mut Vec<(usize, Rect)>, handles: &mut Vec<Handle>) {
        match self {
            Node::Leaf(i) => leaves.push((*i, r)),
            Node::Split { dir, ratio, a, b } => {
                let (ra, rb, hit) = match dir {
                    Dir::H => {
                        let x = r.left() + r.width() * ratio;
                        (
                            Rect::from_min_max(r.min, pos2(x - gap / 2.0, r.max.y)),
                            Rect::from_min_max(pos2(x + gap / 2.0, r.min.y), r.max),
                            Rect::from_center_size(pos2(x, r.center().y), vec2(10.0, r.height())),
                        )
                    }
                    Dir::V => {
                        let y = r.top() + r.height() * ratio;
                        (
                            Rect::from_min_max(r.min, pos2(r.max.x, y - gap / 2.0)),
                            Rect::from_min_max(pos2(r.min.x, y + gap / 2.0), r.max),
                            Rect::from_center_size(pos2(r.center().x, y), vec2(r.width(), 10.0)),
                        )
                    }
                };
                handles.push(Handle { path: path.clone(), dir: *dir, hit, parent: r });
                path.push(false);
                a.layout_in(ra, gap, path, leaves, handles);
                path.pop();
                path.push(true);
                b.layout_in(rb, gap, path, leaves, handles);
                path.pop();
            }
        }
    }

    /// Ersetzt das Blatt `target` durch eine Teilung aus `sub` und dem Blatt – `sub` kommt auf die Seite `z`.
    pub fn split_leaf(&mut self, target: usize, sub: Node, z: Zone) -> bool {
        match self {
            Node::Leaf(i) if *i == target => {
                let leaf = Node::Leaf(target);
                let (dir, first) = match z {
                    Zone::Left => (Dir::H, true),
                    Zone::Right => (Dir::H, false),
                    Zone::Top => (Dir::V, true),
                    Zone::Bottom => (Dir::V, false),
                };
                let (a, b) = if first { (sub, leaf) } else { (leaf, sub) };
                *self = Node::Split { dir, ratio: 0.5, a: Box::new(a), b: Box::new(b) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => {
                if a.contains(target) {
                    a.split_leaf(target, sub, z)
                } else {
                    b.split_leaf(target, sub, z)
                }
            }
        }
    }

    /// Entfernt ein Blatt; sein Nachbar nimmt den Platz ein. `false`, wenn es das einzige Blatt ist.
    pub fn remove_leaf(&mut self, leaf: usize) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => {
                if matches!(**a, Node::Leaf(i) if i == leaf) {
                    *self = (**b).clone();
                    true
                } else if matches!(**b, Node::Leaf(i) if i == leaf) {
                    *self = (**a).clone();
                    true
                } else {
                    a.remove_leaf(leaf) || b.remove_leaf(leaf)
                }
            }
        }
    }

    /// Nach dem Entfernen von Pane `removed` aus der Liste: größere Nummern rücken nach.
    pub fn shift_after(&mut self, removed: usize) {
        match self {
            Node::Leaf(i) => {
                if *i > removed {
                    *i -= 1;
                }
            }
            Node::Split { a, b, .. } => {
                a.shift_after(removed);
                b.shift_after(removed);
            }
        }
    }

    pub fn offset(&mut self, k: usize) {
        match self {
            Node::Leaf(i) => *i += k,
            Node::Split { a, b, .. } => {
                a.offset(k);
                b.offset(k);
            }
        }
    }

    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) {
        if let Node::Split { ratio: r, a, b, .. } = self {
            match path.split_first() {
                None => *r = ratio,
                Some((false, rest)) => a.set_ratio(rest, ratio),
                Some((true, rest)) => b.set_ratio(rest, ratio),
            }
        }
    }

    /// Dreht die Richtung der Teilung, die das Blatt direkt enthält (nebeneinander ↔ übereinander).
    pub fn toggle_parent(&mut self, leaf: usize) -> bool {
        if let Node::Split { dir, a, b, .. } = self {
            let direct = matches!(**a, Node::Leaf(i) if i == leaf) || matches!(**b, Node::Leaf(i) if i == leaf);
            if direct {
                *dir = if *dir == Dir::H { Dir::V } else { Dir::H };
                return true;
            }
            return a.toggle_parent(leaf) || b.toggle_parent(leaf);
        }
        false
    }

    /// Gleichmäßig aufteilen: alle Verhältnisse zurücksetzen.
    pub fn equalize(&mut self) {
        if let Node::Split { ratio, a, b, .. } = self {
            let (na, nb) = (a.count() as f32, b.count() as f32);
            *ratio = na / (na + nb);
            a.equalize();
            b.equalize();
        }
    }

    /// Eine Reihe aus `n` Blättern (für Fenster ohne gespeichertes Layout).
    pub fn chain(n: usize, dir: Dir) -> Node {
        let mut node = Node::Leaf(n.saturating_sub(1));
        for i in (0..n.saturating_sub(1)).rev() {
            node = Node::Split { dir, ratio: 1.0 / (n - i) as f32, a: Box::new(Node::Leaf(i)), b: Box::new(node) };
        }
        node
    }

    // ── Text-Form (für Fensterwechsel/Startargumente), z. B. H0.50(0,V0.50(1,2)) ──
    pub fn serialize(&self) -> String {
        match self {
            Node::Leaf(i) => i.to_string(),
            Node::Split { dir, ratio, a, b } => format!(
                "{}{:.3}({},{})",
                if *dir == Dir::H { 'H' } else { 'V' },
                ratio,
                a.serialize(),
                b.serialize()
            ),
        }
    }

    pub fn parse(s: &str) -> Option<Node> {
        let (n, rest) = Self::parse_in(s.trim())?;
        rest.is_empty().then_some(n)
    }

    fn parse_in(s: &str) -> Option<(Node, &str)> {
        let c = s.chars().next()?;
        if c.is_ascii_digit() {
            let end = s.find(|ch: char| !ch.is_ascii_digit()).unwrap_or(s.len());
            return Some((Node::Leaf(s[..end].parse().ok()?), &s[end..]));
        }
        let dir = match c {
            'H' => Dir::H,
            'V' => Dir::V,
            _ => return None,
        };
        let s = &s[1..];
        let end = s.find('(')?;
        let ratio: f32 = s[..end].parse().ok()?;
        let (a, s) = Self::parse_in(&s[end + 1..])?;
        let s = s.strip_prefix(',')?;
        let (b, s) = Self::parse_in(s)?;
        let s = s.strip_prefix(')')?;
        Some((Node::Split { dir, ratio: ratio.clamp(0.05, 0.95), a: Box::new(a), b: Box::new(b) }, s))
    }

    /// Prüft, ob das Layout genau die Blätter 0..n enthält.
    pub fn valid_for(&self, n: usize) -> bool {
        let mut l = self.leaves();
        l.sort_unstable();
        l == (0..n).collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0))
    }

    #[test]
    fn split_remove_roundtrip() {
        let mut n = Node::Leaf(0);
        assert!(n.split_leaf(0, Node::Leaf(1), Zone::Right));
        assert!(n.split_leaf(1, Node::Leaf(2), Zone::Bottom));
        assert!(n.split_leaf(0, Node::Leaf(3), Zone::Left));
        assert_eq!(n.count(), 4);
        assert!(n.valid_for(4));
        let s = n.serialize();
        assert_eq!(Node::parse(&s).unwrap(), n);
        // Pane 1 entfernen → Nachbar übernimmt, Nummern rücken nach
        assert!(n.remove_leaf(1));
        n.shift_after(1);
        assert!(n.valid_for(3), "{:?}", n);
        // letztes Blatt bleibt
        let mut single = Node::Leaf(0);
        assert!(!single.remove_leaf(0));
    }

    #[test]
    fn layout_covers_and_handles() {
        let mut n = Node::Leaf(0);
        n.split_leaf(0, Node::Leaf(1), Zone::Right);
        n.split_leaf(1, Node::Leaf(2), Zone::Bottom);
        let (mut leaves, mut handles) = (vec![], vec![]);
        n.layout(rect(), 6.0, &mut leaves, &mut handles);
        assert_eq!(leaves.len(), 3);
        assert_eq!(handles.len(), 2);
        let area: f32 = leaves.iter().map(|(_, r)| r.width() * r.height()).sum();
        assert!(area > 1000.0 * 600.0 * 0.95);
        // Pane 0 links, Panes 1 und 2 rechts übereinander
        let r0 = leaves.iter().find(|l| l.0 == 0).unwrap().1;
        let r1 = leaves.iter().find(|l| l.0 == 1).unwrap().1;
        let r2 = leaves.iter().find(|l| l.0 == 2).unwrap().1;
        assert!(r0.right() < r1.left() && r1.bottom() < r2.top());
        // Verhältnis ändern über den Pfad
        n.set_ratio(&handles[0].path.clone(), 0.25);
        let (mut l2, mut h2) = (vec![], vec![]);
        n.layout(rect(), 6.0, &mut l2, &mut h2);
        assert!((l2.iter().find(|l| l.0 == 0).unwrap().1.width() - 247.0).abs() < 1.0);
    }

    #[test]
    fn nested_dock_subtree_and_toggle() {
        let mut host = Node::Leaf(0);
        let mut sub = Node::Leaf(0);
        sub.split_leaf(0, Node::Leaf(1), Zone::Right); // zwei Panes aus einem anderen Tab
        sub.offset(1); // → Blätter 1 und 2
        assert!(host.split_leaf(0, sub, Zone::Bottom));
        assert!(host.valid_for(3));
        assert!(host.toggle_parent(0));
        assert!(matches!(host, Node::Split { dir: Dir::H, .. }));
        assert!(Node::chain(4, Dir::H).valid_for(4));
        let mut e = Node::chain(3, Dir::H);
        e.equalize();
        let (mut l, mut h) = (vec![], vec![]);
        e.layout(rect(), 0.0, &mut l, &mut h);
        for (_, r) in l {
            assert!((r.width() - 1000.0 / 3.0).abs() < 1.0);
        }
    }

    #[test]
    fn zones_and_parse_errors() {
        let r = rect();
        assert_eq!(Zone::at(pos2(10.0, 300.0), r), Some(Zone::Left));
        assert_eq!(Zone::at(pos2(500.0, 300.0), r), None);
        assert_eq!(Zone::at(pos2(500.0, 590.0), r), Some(Zone::Bottom));
        assert!(Node::parse("H0.5(0").is_none());
        assert!(Node::parse("x").is_none());
        assert_eq!(Node::parse("3"), Some(Node::Leaf(3)));
    }
}
