//! Where everything in a recipe goes on the canvas, at 100% zoom.
//!
//! The same tree as the engine runs: a chain lays its parts left to right,
//! consecutive steps snapping into one strip; branches stack top to bottom;
//! a group is a dashed box with its name above what it holds. Wires run
//! from whatever ends one part to whatever starts the next, straight along
//! and bending only in the last gap, so a short branch's wire into a join
//! doesn't cut across the others.
//!
//! A window only draws what this gives it, scaled by its zoom.

use drt_engine::{Node, Side};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub tile_w: f32,
    pub tile_h: f32,
    /// Between the parts of a chain, where wires run.
    pub gap: f32,
    /// Between branches.
    pub vgap: f32,
    pub group_pad: f32,
    /// Room for a group's name above its contents.
    pub group_label: f32,
}

/// One rhythm from the card (docs/ux/cards.md, *Composition*): a 288 × 192
/// card on a 16 px module; strips a third of its height apart, stacked
/// branches a sixth.
impl Default for Metrics {
    fn default() -> Self {
        Metrics { tile_w: 288.0, tile_h: 192.0, gap: 64.0, vgap: 32.0, group_pad: 16.0, group_label: 32.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn mid_y(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

/// Where a tile sits in its strip, which decides its rounded corners and
/// the arrow notch on its right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InStrip {
    Alone,
    First,
    Middle,
    Last,
}

impl InStrip {
    pub fn round_left(self) -> bool {
        matches!(self, InStrip::Alone | InStrip::First)
    }
    pub fn round_right(self) -> bool {
        matches!(self, InStrip::Alone | InStrip::Last)
    }
    /// The notch pointing into the next tile of the strip.
    pub fn notch(self) -> bool {
        matches!(self, InStrip::First | InStrip::Middle)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TilePlace {
    /// The step's id in the recipe and the snapshot.
    pub id: String,
    pub rect: Rect,
    pub in_strip: InStrip,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupPlace {
    pub name: String,
    pub rect: Rect,
}

/// A wire: straight from `from` to `bend`, then a curve to `to` with these
/// two control points. `from_step` is the step it leaves, `to_step` the
/// one it feeds (for a wire to or from a group's edge, the step inside).
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub from: (f32, f32),
    pub bend: (f32, f32),
    pub c1: (f32, f32),
    pub c2: (f32, f32),
    pub to: (f32, f32),
    pub from_step: String,
    pub to_step: String,
}

impl Wire {
    /// The wire as points, for toolkits that draw polylines.
    pub fn points(&self, segments: usize) -> Vec<(f32, f32)> {
        let mut pts = vec![self.from, self.bend];
        let (p0, p1, p2, p3) = (self.bend, self.c1, self.c2, self.to);
        for i in 1..=segments {
            let t = i as f32 / segments as f32;
            let u = 1.0 - t;
            let x = u * u * u * p0.0 + 3.0 * u * u * t * p1.0 + 3.0 * u * t * t * p2.0 + t * t * t * p3.0;
            let y = u * u * u * p0.1 + 3.0 * u * u * t * p1.1 + 3.0 * u * t * t * p2.1 + t * t * t * p3.1;
            pts.push((x, y));
        }
        pts
    }
}

/// Where a + goes where a set of branches comes together, to add a step
/// that gets everything from all of them.
#[derive(Clone, Debug, PartialEq)]
pub struct JoinPlace {
    /// The branches' path in the recipe (`Spot::Join`).
    pub path: Vec<usize>,
    /// Where the + goes: where the wires out of the branches meet, at the
    /// left edge of what follows them; with nothing after them, in the gap
    /// after them, level with their middle.
    pub at: (f32, f32),
    /// Where each branch ends, and its wire out leaves.
    pub from: Vec<(f32, f32)>,
    /// Whether wires already leave the branches' ends. If not, the + is
    /// shown with lines from each end into it.
    pub met: bool,
}

impl JoinPlace {
    /// The lines from each branch's end into the +: straight along, then
    /// a curve over the last half gap, as the wires into a step run.
    pub fn lines_in(&self, gap: f32) -> Vec<Wire> {
        let mut out = Layout::default();
        let from: Vec<_> = self.from.iter().map(|p| (*p, String::new())).collect();
        wire_all(&from, &[(self.at, String::new())], gap / 2.0, &mut out);
        out.wires
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub tiles: Vec<TilePlace>,
    pub groups: Vec<GroupPlace>,
    pub wires: Vec<Wire>,
    /// Where each strip's + goes when editing, by the id of the strip's last
    /// tile: centred in the gap after it.
    pub pluses: Vec<(String, (f32, f32))>,
    /// Where a + goes between two tiles of a strip when editing, by the id
    /// of the tile on its left: on the seam between them, half way down.
    pub seams: Vec<(String, (f32, f32))>,
    /// Where a + goes after each set of branches, outermost first where
    /// two end together.
    pub joins: Vec<JoinPlace>,
    /// Lines from a step that ends the disc back into the first tile, for
    /// the next disc: down from the step, along under everything, and up
    /// into the start. Corner points, in order; the last is the arrow's tip.
    pub loops: Vec<Vec<(f32, f32)>>,
    /// Where each step's buttons for a path of its own go when editing:
    /// on its card's foot, by its right corner. Only for a path it hasn't.
    pub side_buttons: Vec<(String, Side, (f32, f32))>,
    /// The paths a step takes when it fails or needs you: down from its
    /// card's foot, round what's below, along under everything and down
    /// into what hears of it, in a row of its own at the bottom.
    pub side_wires: Vec<SideWire>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SideWire {
    pub from_step: String,
    pub to_step: String,
    pub side: Side,
    /// Corner points, in order; the last is the arrow's tip.
    pub path: Vec<(f32, f32)>,
}

/// Where on a card's foot a path for `side` leaves, from its left.
pub fn side_x(r: Rect, side: Side) -> f32 {
    match side {
        Side::Fails => r.right() - 48.0,
        Side::NeedsYou => r.right() - 24.0,
    }
}

impl Layout {
    pub fn tile(&self, id: &str) -> Option<&TilePlace> {
        self.tiles.iter().find(|t| t.id == id)
    }

    /// Draws `from` leading back to the first tile, and makes room for the
    /// line under everything. Each further loop runs a little lower.
    pub fn loop_back(&mut self, from: &str) {
        let (Some(f), Some(start)) = (self.tile(from).map(|t| t.rect), self.tiles.first().map(|t| t.rect)) else { return };
        let below = self.tiles.iter().map(|t| t.rect.y + t.rect.h).fold(0.0, f32::max) + 32.0 + 16.0 * self.loops.len() as f32;
        let (fx, sx) = (f.x + f.w / 2.0, start.x + start.w / 2.0);
        // Does anything stand in a vertical line's way, from `y` down?
        let blocked = |x: f32, y: f32| self.tiles.iter().any(|t| t.rect != f && t.rect.x < x && x < t.rect.x + t.rect.w && t.rect.y + t.rect.h > y);
        let path = if !blocked(fx, f.y + f.h) {
            // Straight down from under the step.
            vec![(fx, f.y + f.h), (fx, below), (sx, below), (sx, start.y + start.h)]
        } else {
            // Something is below it: leave from its side and drop through
            // the gap beside it (the right if it's clear, else the left).
            let mid = f.y + f.h / 2.0;
            let gap = 16.0;
            let (edge, x) = if !blocked(f.x + f.w + gap, mid) { (f.x + f.w, f.x + f.w + gap) } else { (f.x, f.x - gap) };
            vec![(edge, mid), (x, mid), (x, below), (sx, below), (sx, start.y + start.h)]
        };
        self.loops.push(path);
        self.height = self.height.max(below + 16.0);
    }
}

/// The layout, with every step that ends a disc leading back to the start.
pub fn layout_with_loops(root: &Node, m: Metrics, loops_back: impl Fn(&str) -> bool) -> Layout {
    // A branch that leads back to the start is drawn last of its siblings,
    // so its line back runs under the others rather than through them.
    let leads_back = |n: &Node| n.ids().into_iter().any(&loops_back);
    let mut l = layout_ordered(root, m, &leads_back);
    let ids: Vec<String> = l.tiles.iter().map(|t| t.id.clone()).filter(|id| loops_back(id)).collect();
    for id in ids {
        l.loop_back(&id);
    }
    add_sides(&mut l, root, m);
    l
}

pub fn layout(root: &Node, m: Metrics) -> Layout {
    let mut l = layout_ordered(root, m, &|_| false);
    add_sides(&mut l, root, m);
    l
}

/// The paths for when a step fails or needs you, after everything else
/// (the start stays the first tile): each what hears of it in a row at
/// the bottom, under the step where there's room, its line on a lane of
/// its own above that row.
fn add_sides(l: &mut Layout, root: &Node, m: Metrics) {
    let main: Vec<TilePlace> = l.tiles.clone();
    let mut paths: Vec<(Rect, &str, Side, &Node)> = vec![];
    for n in root.steps() {
        let Node::Step { id, .. } = n else { continue };
        let Some(t) = main.iter().find(|t| t.id == *id) else { continue };
        for s in Side::BOTH {
            match n.side(s) {
                Some(path) => paths.push((t.rect, id, s, path)),
                None => l.side_buttons.push((id.clone(), s, (side_x(t.rect, s), t.rect.y + t.rect.h))),
            }
        }
    }
    if paths.is_empty() {
        return;
    }
    paths.sort_by(|a, b| a.0.x.total_cmp(&b.0.x).then(a.0.y.total_cmp(&b.0.y)));
    let below = main.iter().map(|t| t.rect.y + t.rect.h).chain(l.loops.iter().flatten().map(|p| p.1)).fold(0.0, f32::max) + 32.0;
    let lane = |k: usize| below + 16.0 * k as f32;
    let row = lane(paths.len() - 1) + 32.0;
    // Is the way down at `x`, from `y0` to `y1`, clear of every card but
    // the step's own?
    let blocked = |x: f32, y0: f32, y1: f32| main.iter().any(|t| t.rect.x < x && x < t.rect.right() && t.rect.y < y1 && t.rect.y + t.rect.h > y0);
    let across = |x0: f32, x1: f32, y: f32| main.iter().any(|t| t.rect.x < x0.max(x1) && x0.min(x1) < t.rect.right() && t.rect.y < y && y < t.rect.y + t.rect.h);
    let mut cursor = f32::MIN;
    for (k, (r, from, side, path)) in paths.into_iter().enumerate() {
        let sub = layout(path, m);
        let x = r.x.max(cursor);
        cursor = x + sub.width;
        let first = l.tiles.len();
        for t in &sub.tiles {
            l.tiles.push(TilePlace { id: t.id.clone(), rect: Rect { x: t.rect.x + x, y: t.rect.y + row, ..t.rect }, in_strip: t.in_strip });
        }
        let shift = |p: (f32, f32)| (p.0 + x, p.1 + row);
        for w in &sub.wires {
            l.wires.push(Wire { from: shift(w.from), bend: shift(w.bend), c1: shift(w.c1), c2: shift(w.c2), to: shift(w.to), ..w.clone() });
        }
        let into = l.tiles[first].clone();
        let (sx, foot, y) = (side_x(r, side), r.y + r.h, lane(k));
        // Straight down; else down into the gap below the card, along to
        // the nearest clear way down, and down that.
        let mut pts = vec![(sx, foot)];
        if blocked(sx, foot, y) {
            let mid = foot + m.vgap / 2.0;
            let off = if side == Side::Fails { 12.0 } else { 24.0 };
            let mut ways: Vec<f32> = main.iter().flat_map(|t| [t.rect.x - off, t.rect.right() + off]).collect();
            ways.sort_by(|a, b| (a - sx).abs().total_cmp(&(b - sx).abs()));
            if let Some(gx) = ways.into_iter().find(|&gx| !across(sx, gx, mid) && !blocked(gx, mid, y)) {
                pts.extend([(sx, mid), (gx, mid)]);
            }
        }
        let last = pts.last().unwrap().0;
        let cx = into.rect.x + into.rect.w / 2.0;
        pts.extend([(last, y), (cx, y), (cx, into.rect.y)]);
        pts.dedup();
        l.side_wires.push(SideWire { from_step: from.to_owned(), to_step: into.id.clone(), side, path: pts });
        l.width = l.width.max(cursor + m.gap);
    }
    l.height = l.height.max(row + m.tile_h);
}

/// The layout, with the branches `last` picks drawn after their siblings.
fn layout_ordered(root: &Node, m: Metrics, last: &dyn Fn(&Node) -> bool) -> Layout {
    let unit = Unit::of(root, vec![], last);
    let (w, h) = unit.size(&m);
    let mut out = Layout { width: w + m.gap, height: h, ..Default::default() };
    unit.place(0.0, 0.0, &m, &mut out);
    // Outermost first, as the recipe has them.
    out.joins.sort_by(|a, b| a.path.len().cmp(&b.path.len()).then(a.path.cmp(&b.path)));
    out
}

/// The tree regrouped for layout: runs of consecutive steps become strips.
enum Unit<'a> {
    Strip(Vec<&'a str>),
    Seq(Vec<Unit<'a>>),
    /// Branches, and their path in the recipe.
    Par(Vec<Unit<'a>>, Vec<usize>),
    Group(&'a str, Box<Unit<'a>>),
}

/// Where a unit's wires come in and go out, with the step at each end.
struct Ends {
    ins: Vec<((f32, f32), String)>,
    outs: Vec<((f32, f32), String)>,
    /// Sets of branches ending here, by index in `joins`, outermost last:
    /// their wires meet at whatever follows.
    open: Vec<usize>,
}

impl<'a> Unit<'a> {
    fn of(n: &'a Node, path: Vec<usize>, last: &dyn Fn(&Node) -> bool) -> Unit<'a> {
        let at = |k: usize| {
            let mut p = path.clone();
            p.push(k);
            p
        };
        match n {
            Node::Step { id, .. } => Unit::Strip(vec![id]),
            Node::Chain { nodes } => {
                let mut units: Vec<Unit> = Vec::new();
                for (k, n) in nodes.iter().enumerate() {
                    match (Unit::of(n, at(k), last), units.last_mut()) {
                        (Unit::Strip(ids), Some(Unit::Strip(prev))) => prev.extend(ids),
                        (u, _) => units.push(u),
                    }
                }
                if units.len() == 1 {
                    units.pop().unwrap()
                } else {
                    Unit::Seq(units)
                }
            }
            Node::Branches { branches } => {
                let mut bs: Vec<(usize, &Node)> = branches.iter().enumerate().collect();
                bs.sort_by_key(|(_, b)| last(b));
                Unit::Par(bs.into_iter().map(|(k, b)| Unit::of(b, at(k), last)).collect(), path)
            }
            Node::Group { name, body } => Unit::Group(name, Box::new(Unit::of(body, at(0), last))),
        }
    }

    fn size(&self, m: &Metrics) -> (f32, f32) {
        match self {
            Unit::Strip(ids) => (ids.len() as f32 * m.tile_w, m.tile_h),
            Unit::Seq(us) => {
                let s: Vec<_> = us.iter().map(|u| u.size(m)).collect();
                (s.iter().map(|x| x.0).sum::<f32>() + m.gap * (s.len() as f32 - 1.0), s.iter().map(|x| x.1).fold(0.0, f32::max))
            }
            Unit::Par(us, _) => {
                let s: Vec<_> = us.iter().map(|u| u.size(m)).collect();
                (s.iter().map(|x| x.0).fold(0.0, f32::max), s.iter().map(|x| x.1).sum::<f32>() + m.vgap * (s.len() as f32 - 1.0))
            }
            Unit::Group(_, u) => {
                let (w, h) = u.size(m);
                (w + 2.0 * m.group_pad, h + m.group_label + m.group_pad)
            }
        }
    }

    fn place(&self, x: f32, y: f32, m: &Metrics, out: &mut Layout) -> Ends {
        match self {
            Unit::Strip(ids) => {
                let n = ids.len();
                for (i, id) in ids.iter().enumerate() {
                    let in_strip = match (n, i) {
                        (1, _) => InStrip::Alone,
                        (_, 0) => InStrip::First,
                        (_, i) if i == n - 1 => InStrip::Last,
                        _ => InStrip::Middle,
                    };
                    out.tiles.push(TilePlace { id: (*id).to_owned(), rect: Rect { x: x + i as f32 * m.tile_w, y, w: m.tile_w, h: m.tile_h }, in_strip });
                }
                let w = n as f32 * m.tile_w;
                let mid = y + m.tile_h / 2.0;
                for (i, id) in ids.iter().enumerate().take(n - 1) {
                    out.seams.push(((*id).to_owned(), (x + (i + 1) as f32 * m.tile_w, mid)));
                }
                // On the strip's right edge, like the seams between tiles.
                out.pluses.push((ids[n - 1].to_owned(), (x + w, mid)));
                Ends { ins: vec![((x, mid), ids[0].to_owned())], outs: vec![((x + w, mid), ids[n - 1].to_owned())], open: vec![] }
            }
            Unit::Seq(us) => {
                let (_, h) = self.size(m);
                let mut cx = x;
                let mut first: Option<Vec<((f32, f32), String)>> = None;
                let mut last: Option<Vec<((f32, f32), String)>> = None;
                let mut open: Vec<usize> = vec![];
                for u in us {
                    let (uw, uh) = u.size(m);
                    let e = u.place(cx, y + (h - uh) / 2.0, m, out);
                    if let Some(outs) = &last {
                        wire_all(outs, &e.ins, m.gap, out);
                        // The wires out of the branches before meet here:
                        // the outermost's + goes where they arrive.
                        for &j in &open {
                            out.joins[j].met = true;
                        }
                        if let (Some(&j), [(to, _)]) = (open.last(), e.ins.as_slice()) {
                            out.joins[j].at = *to;
                        }
                    }
                    first.get_or_insert(e.ins);
                    last = Some(e.outs);
                    open = e.open;
                    cx += uw + m.gap;
                }
                Ends { ins: first.unwrap_or_default(), outs: last.unwrap_or_default(), open }
            }
            Unit::Par(us, path) => {
                let mut cy = y;
                let mut e = Ends { ins: vec![], outs: vec![], open: vec![] };
                for u in us {
                    let (_, uh) = u.size(m);
                    let r = u.place(x, cy, m, out);
                    e.ins.extend(r.ins);
                    e.outs.extend(r.outs);
                    e.open.extend(r.open);
                    cy += uh + m.vgap;
                }
                let (w, h) = self.size(m);
                let from: Vec<(f32, f32)> = e.outs.iter().map(|o| o.0).collect();
                // In the gap after them, level with their middle; but not on
                // top of the + at the end of a branch reaching that far: then
                // half way between that branch and the next.
                let mut at = (x + w + m.gap / 2.0, y + h / 2.0);
                if let Some(k) = from.iter().position(|f| (f.1 - at.1).abs() < 1.0 && at.0 - f.0 < m.gap) {
                    let ys: Vec<f32> = from.iter().map(|f| f.1).collect();
                    let other = ys.iter().copied().filter(|&v| v > ys[k]).fold(None, |b: Option<f32>, v| Some(b.map_or(v, |b| b.min(v))));
                    let other = other.or_else(|| ys.iter().copied().filter(|&v| v < ys[k]).fold(None, |b: Option<f32>, v| Some(b.map_or(v, |b| b.max(v)))));
                    if let Some(o) = other {
                        at.1 = (ys[k] + o) / 2.0;
                    }
                }
                // One branch left alone is just a line: its own + will do.
                if us.len() > 1 {
                    out.joins.push(JoinPlace { path: path.clone(), at, from, met: false });
                    e.open.push(out.joins.len() - 1);
                }
                e
            }
            Unit::Group(name, u) => {
                let (w, h) = self.size(m);
                out.groups.push(GroupPlace { name: (*name).to_owned(), rect: Rect { x, y, w, h } });
                let r = u.place(x + m.group_pad, y + m.group_label, m, out);
                // Wires meet the group's edge, level with what's inside.
                let iy = r.ins.first().map_or(y + h / 2.0, |p| p.0 .1);
                let oy = r.outs.last().map_or(y + h / 2.0, |p| p.0 .1);
                let first = r.ins.first().map(|p| p.1.clone()).unwrap_or_default();
                let last = r.outs.last().map(|p| p.1.clone()).unwrap_or_default();
                if let Some(p) = r.ins.first() {
                    wire_all(&[((x, iy), first.clone())], std::slice::from_ref(p), 0.0, out);
                }
                if let Some(p) = r.outs.last() {
                    wire_all(std::slice::from_ref(p), &[((x + w, oy), last.clone())], 0.0, out);
                }
                // Branches ending inside the box: their + stays inside it.
                for &j in &r.open {
                    let (jx, _) = out.joins[j].at;
                    out.joins[j].at.0 = jx.min(x + w - m.group_pad / 2.0);
                }
                Ends { ins: vec![((x, iy), first)], outs: vec![((x + w, oy), last)], open: vec![] }
            }
        }
    }
}

fn wire_all(outs: &[((f32, f32), String)], ins: &[((f32, f32), String)], gap: f32, out: &mut Layout) {
    for (from, from_step) in outs {
        for (to, to_step) in ins {
            let (from, to) = (*from, *to);
            if (to.0 - from.0).abs() < 0.5 && (to.1 - from.1).abs() < 0.5 {
                continue;
            }
            let bx = from.0.max(to.0 - gap);
            // The control points half the gap out.
            let d = (to.0 - bx) * 0.5;
            out.wires.push(Wire { from, bend: (bx, from.1), c1: (bx + d, from.1), c2: (to.0 - d, to.1), to, from_step: from_step.clone(), to_step: to_step.clone() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drt_steps::recipes;

    /// No vertical or horizontal leg of a line back crosses any card.
    fn crosses(l: &Layout, path: &[(f32, f32)], own: &str) -> Option<String> {
        for w in path.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            for t in l.tiles.iter().filter(|t| t.id != own) {
                let r = t.rect;
                let (lx, hx, ly, hy) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
                if lx < r.x + r.w - 0.5 && hx > r.x + 0.5 && ly < r.y + r.h - 0.5 && hy > r.y + 0.5 {
                    return Some(t.id.clone());
                }
            }
        }
        None
    }

    #[test]
    fn a_steps_own_paths_go_round_every_card_into_a_row_below() {
        let m = Metrics::default();
        for r in recipes::all() {
            let mut root = r.root.clone();
            let ids: Vec<String> = root.ids().into_iter().map(str::to_owned).collect();
            for (k, id) in ids.iter().enumerate() {
                let n = root.find_mut(id).unwrap();
                *n = n.clone().on(Side::Fails, Node::step(&format!("f{k}"), "tell")).on(Side::NeedsYou, Node::step(&format!("n{k}"), "tell"));
            }
            let l = layout_with_loops(&root, m, |id| id.ends_with("next"));
            assert_eq!(l.side_wires.len(), 2 * ids.len(), "{}", r.name);
            let main_bottom = l.tiles.iter().filter(|t| ids.contains(&t.id)).map(|t| t.rect.y + t.rect.h).fold(0.0, f32::max);
            for w in &l.side_wires {
                let into = l.tile(&w.to_step).unwrap().rect;
                assert!(into.y > main_bottom, "{} sits below everything", w.to_step);
                assert_eq!(*w.path.last().unwrap(), (into.x + into.w / 2.0, into.y), "into the top of {}", w.to_step);
                // Leaves its own card's foot and crosses no other.
                let from = l.tile(&w.from_step).unwrap().rect;
                assert_eq!(w.path[0], (side_x(from, w.side), from.y + from.h));
                let others = Layout { tiles: l.tiles.iter().filter(|t| t.id != w.to_step).cloned().collect(), ..Layout::default() };
                assert_eq!(crosses(&others, &w.path, &w.from_step), None, "{}: {} → {}", r.name, w.from_step, w.to_step);
            }
            // No two cards overlap.
            for (a, ta) in l.tiles.iter().enumerate() {
                for tb in &l.tiles[a + 1..] {
                    let (x, y) = (ta.rect, tb.rect);
                    assert!(x.right() <= y.x + 0.5 || y.right() <= x.x + 0.5 || x.y + x.h <= y.y + 0.5 || y.y + y.h <= x.y + 0.5, "{} on {}", ta.id, tb.id);
                }
            }
            assert!(l.width >= l.tiles.iter().map(|t| t.rect.right()).fold(0.0, f32::max));
        }
        // A step with both paths has no buttons left; one with none, two.
        let root = drt_engine::recipe::chain([Node::step("d", "drive"), Node::step("r", "rip").on(Side::Fails, Node::step("t", "tell"))]);
        let l = layout(&root, m);
        let buttons: Vec<(&str, Side)> = l.side_buttons.iter().map(|b| (b.0.as_str(), b.1)).collect();
        assert_eq!(buttons, [("d", Side::Fails), ("d", Side::NeedsYou), ("r", Side::NeedsYou)]);
    }

    #[test]
    fn the_line_back_from_next_disc_goes_round_what_shares_its_branch() {
        use drt_engine::recipe::{branches, chain};
        let m = Metrics::default();
        // Next disc listed before Notify: it's still drawn last, under it.
        let root = chain([Node::step("d", "drive"), Node::step("r", "rip"), branches([Node::step("f", "flac"), Node::step("next", "next-disc"), Node::step("tell", "tell")])]);
        let l = layout_with_loops(&root, m, |id| id == "next");
        let (next, tell) = (l.tile("next").unwrap().rect, l.tile("tell").unwrap().rect);
        assert!(next.y > tell.y, "Next disc is drawn below Notify");
        assert_eq!(crosses(&l, &l.loops[0], "next"), None);
        // Something after the join, below Next disc's column: the line
        // leaves from the side instead of going through it.
        let root = chain([Node::step("d", "drive"), Node::step("r", "rip"), branches([Node::step("f", "flac"), chain([Node::step("next", "next-disc")])])]);
        let l = layout_with_loops(&root, m, |id| id == "next");
        assert_eq!(crosses(&l, &l.loops[0], "next"), None);
        // The CD recipe, as built in.
        let l = layout_with_loops(&recipes::cd().root, m, |id| id == "next");
        assert_eq!(crosses(&l, &l.loops[0], "next"), None);
    }

    #[test]
    fn the_cd_recipe_is_ten_tiles_long_and_three_rows_tall() {
        let m = Metrics::default();
        // Cards 3:2 on the 16 px module; strips a third of a card apart,
        // stacked branches a sixth.
        assert_eq!((m.tile_w, m.tile_h, m.gap, m.vgap), (288.0, 192.0, 64.0, 32.0));
        let l = layout(&recipes::cd().root, m);
        assert_eq!(l.tiles.len(), 14);
        // Drive, Audio CD | a lookup | Choose, Cover art, Rip | FLAC, Review, Hold, Library
        assert_eq!(l.width, 10.0 * m.tile_w + 4.0 * m.gap);
        // FLAC, MP3 and Next disc stacked after Rip.
        assert_eq!(l.height, 3.0 * m.tile_h + 2.0 * m.vgap);
        assert_eq!(l.tile("drive").unwrap().in_strip, InStrip::First);
        assert_eq!(l.tile("rip").unwrap().in_strip, InStrip::Last);
        // Two lookups fan out from Audio CD and join at Choose; three lines fan out from Rip.
        assert_eq!(l.wires.len(), 2 + 2 + 3);
    }

    #[test]
    fn every_wire_knows_the_steps_at_its_ends() {
        let l = layout(&recipes::cd().root, Metrics::default());
        let ends: Vec<(&str, &str)> = l.wires.iter().map(|w| (w.from_step.as_str(), w.to_step.as_str())).collect();
        // Audio CD fans out to the lookups, they join at Choose, Rip fans out.
        for pair in [("cd", "mb"), ("cd", "cdtext"), ("mb", "choose"), ("cdtext", "choose"), ("rip", "flac"), ("rip", "mp3"), ("rip", "next")] {
            assert!(ends.contains(&pair), "{pair:?} in {ends:?}");
        }
        // And each wire ends where its step's tile is.
        for w in &l.wires {
            let to = l.tile(&w.to_step).unwrap().rect;
            assert_eq!(w.to, (to.x, to.mid_y()), "{}", w.to_step);
        }
        // A group's edge wires name the step inside.
        let a = layout(&grouped(), Metrics::default());
        assert!(a.wires.iter().all(|w| a.tile(&w.from_step).is_some() && a.tile(&w.to_step).is_some()));
    }

    #[test]
    fn every_seam_inside_a_strip_has_a_plus_on_it() {
        let m = Metrics::default();
        let l = layout(&recipes::cd().root, m);
        // Choose | Cover art | Rip: two seams, on the tiles' shared edge.
        let at = |id: &str| l.seams.iter().find(|(x, _)| x == id).map(|(_, p)| *p);
        let (choose, cover) = (l.tile("choose").unwrap().rect, l.tile("art").unwrap().rect);
        assert_eq!(at("choose"), Some((choose.right(), choose.mid_y())));
        assert_eq!(at("choose").unwrap().0, cover.x);
        assert!(at("art").is_some());
        // None after a strip's last tile (that's its + at the end), nor for
        // a tile alone.
        assert!(at("rip").is_none() && at("mb").is_none());
        let pairs: usize = l.tiles.iter().filter(|t| t.in_strip.notch()).count();
        assert_eq!(l.seams.len(), pairs);
    }

    #[test]
    fn next_disc_leads_back_into_the_drive_under_everything() {
        let m = Metrics::default();
        let plain = layout(&recipes::cd().root, m);
        let l = layout_with_loops(&recipes::cd().root, m, |id| id == "next");
        assert_eq!(l.loops.len(), 1);
        let (next, drive) = (l.tile("next").unwrap().rect, l.tile("drive").unwrap().rect);
        let path = &l.loops[0];
        assert_eq!(path[0], (next.x + next.w / 2.0, next.y + next.h), "leaves from the bottom of Next disc");
        assert_eq!(*path.last().unwrap(), (drive.x + drive.w / 2.0, drive.y + drive.h), "arrives at the bottom of Drive");
        assert!(path[1].1 > plain.height, "runs under every tile");
        assert!(l.height > plain.height);
    }

    /// The archive recipe with its phone line in a named group.
    fn grouped() -> Node {
        use drt_engine::recipe::group;
        let mut root = recipes::archive().root;
        let Node::Chain { nodes } = &mut root else { panic!() };
        let Some(Node::Branches { branches }) = nodes.iter_mut().rev().find(|n| matches!(n, Node::Branches { .. })) else { panic!() };
        let phone = branches.pop().unwrap();
        branches.push(group("For the phone", phone));
        root
    }

    #[test]
    fn the_most_complex_recipe_has_its_join_and_a_group_draws_round_its_steps() {
        let l = layout(&recipes::archive().root, Metrics::default());
        let tell = l.tile("a_tell").unwrap().rect;
        let into_tell = l.wires.iter().filter(|w| w.to == (tell.x, tell.mid_y())).count();
        assert_eq!(into_tell, 7, "NAS, Backup, the cue sheet, the .lrc files, the scans, MP3's library and the phone");
        let l = layout(&grouped(), Metrics::default());
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].name, "For the phone");
        let tell = l.tile("a_tell").unwrap().rect;
        assert_eq!(l.wires.iter().filter(|w| w.to == (tell.x, tell.mid_y())).count(), 7, "the group's wire for the phone");
        let phone = l.tile("a_ph").unwrap().rect;
        assert!(l.groups[0].rect.right() > phone.right());
    }

    #[test]
    fn each_set_of_branches_has_a_plus_where_its_wires_meet() {
        let m = Metrics::default();
        let l = layout(&recipes::cd().root, m);
        // The lookups meet at Choose: the + sits on Choose's left edge,
        // where their wires arrive.
        let choose = l.tile("choose").unwrap().rect;
        let lookups = l.joins.iter().find(|j| j.path == [2]).unwrap();
        assert_eq!(lookups.at, (choose.x, choose.mid_y()));
        assert!(lookups.met);
        assert_eq!(lookups.from.len(), 2);
        // Nothing after FLAC's, MP3's and Next disc's lines yet: a + in the
        // gap after them, level with their middle, with lines drawn in.
        let end = l.joins.iter().find(|j| j.path == [6]).unwrap();
        assert!(!end.met);
        assert_eq!(end.from.len(), 3);
        let right = l.tiles.iter().map(|t| t.rect.right()).fold(0.0, f32::max);
        assert_eq!(end.at.0, right + m.gap / 2.0);
        assert!(end.at.0 < l.width);
        assert_eq!(end.lines_in(m.gap).len(), 3);
        assert!(end.lines_in(m.gap).iter().all(|w| w.to == end.at));
        // In the archive, two sets of branches end in the same gap: the
        // outer one's + is at Notify, the inner one's in the gap.
        let a = layout(&recipes::archive().root, m);
        let tell = a.tile("a_tell").unwrap().rect;
        let (outer, inner) = (&a.joins.iter().find(|j| j.path == [6]).unwrap(), &a.joins.iter().find(|j| j.path.len() > 2).unwrap());
        assert_eq!(outer.at, (tell.x, tell.mid_y()));
        assert!(inner.met && inner.at != outer.at && inner.at.0 < tell.x);
        // Not on the + at the end of the cue sheet's line, level with it:
        // half way down to the next.
        let cue = a.tile("a_cue").unwrap().rect;
        let lrc = a.tile("a_lrc").unwrap().rect;
        assert_eq!(inner.at.1, (cue.mid_y() + lrc.mid_y()) / 2.0);
    }

    #[test]
    fn a_plus_after_branches_knows_them_by_their_place_in_the_recipe_however_they_are_drawn() {
        use drt_engine::recipe::{branches, chain};
        let m = Metrics::default();
        // Next disc listed first is drawn last; the inner branches are still
        // the recipe's second branch's second part.
        let root = chain([
            Node::step("d", "drive"),
            Node::step("r", "rip"),
            branches([Node::step("next", "next-disc"), chain([Node::step("f", "flac"), branches([Node::step("a", "folder"), Node::step("b", "folder")])])]),
        ]);
        let l = layout_with_loops(&root, m, |id| id == "next");
        let paths: Vec<&[usize]> = l.joins.iter().map(|j| j.path.as_slice()).collect();
        assert_eq!(paths, [&[2][..], &[2, 1, 1][..]]);
        for j in &l.joins {
            assert!(matches!(root.at(&j.path), Some(Node::Branches { .. })));
        }
        // One branch left, after the others were removed: just a line, no +.
        let root = chain([Node::step("d", "drive"), branches([chain([Node::step("f", "flac"), Node::step("g", "folder")])])]);
        assert!(layout(&root, m).joins.is_empty());
        assert!(root.joins().is_empty());
    }
}
