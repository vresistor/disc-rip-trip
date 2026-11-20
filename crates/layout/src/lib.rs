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

use drt_engine::Node;

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
    /// Lines from a step that ends the disc back into the first tile, for
    /// the next disc: down from the step, along under everything, and up
    /// into the start. Corner points, in order; the last is the arrow's tip.
    pub loops: Vec<Vec<(f32, f32)>>,
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
        self.loops.push(vec![(fx, f.y + f.h), (fx, below), (sx, below), (sx, start.y + start.h)]);
        self.height = self.height.max(below + 16.0);
    }
}

/// The layout, with every step that ends a disc leading back to the start.
pub fn layout_with_loops(root: &Node, m: Metrics, loops_back: impl Fn(&str) -> bool) -> Layout {
    let mut l = layout(root, m);
    let ids: Vec<String> = l.tiles.iter().map(|t| t.id.clone()).filter(|id| loops_back(id)).collect();
    for id in ids {
        l.loop_back(&id);
    }
    l
}

pub fn layout(root: &Node, m: Metrics) -> Layout {
    let unit = Unit::of(root);
    let (w, h) = unit.size(&m);
    let mut out = Layout { width: w + m.gap, height: h, ..Default::default() };
    unit.place(0.0, 0.0, &m, &mut out);
    out
}

/// The tree regrouped for layout: runs of consecutive steps become strips.
enum Unit<'a> {
    Strip(Vec<&'a str>),
    Seq(Vec<Unit<'a>>),
    Par(Vec<Unit<'a>>),
    Group(&'a str, Box<Unit<'a>>),
}

/// Where a unit's wires come in and go out, with the step at each end.
struct Ends {
    ins: Vec<((f32, f32), String)>,
    outs: Vec<((f32, f32), String)>,
}

impl<'a> Unit<'a> {
    fn of(n: &'a Node) -> Unit<'a> {
        match n {
            Node::Step { id, .. } => Unit::Strip(vec![id]),
            Node::Chain { nodes } => {
                let mut units: Vec<Unit> = Vec::new();
                for n in nodes {
                    match (Unit::of(n), units.last_mut()) {
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
            Node::Branches { branches } => Unit::Par(branches.iter().map(Unit::of).collect()),
            Node::Group { name, body } => Unit::Group(name, Box::new(Unit::of(body))),
        }
    }

    fn size(&self, m: &Metrics) -> (f32, f32) {
        match self {
            Unit::Strip(ids) => (ids.len() as f32 * m.tile_w, m.tile_h),
            Unit::Seq(us) => {
                let s: Vec<_> = us.iter().map(|u| u.size(m)).collect();
                (s.iter().map(|x| x.0).sum::<f32>() + m.gap * (s.len() as f32 - 1.0), s.iter().map(|x| x.1).fold(0.0, f32::max))
            }
            Unit::Par(us) => {
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
                Ends { ins: vec![((x, mid), ids[0].to_owned())], outs: vec![((x + w, mid), ids[n - 1].to_owned())] }
            }
            Unit::Seq(us) => {
                let (_, h) = self.size(m);
                let mut cx = x;
                let mut first: Option<Vec<((f32, f32), String)>> = None;
                let mut last: Option<Vec<((f32, f32), String)>> = None;
                for u in us {
                    let (uw, uh) = u.size(m);
                    let e = u.place(cx, y + (h - uh) / 2.0, m, out);
                    if let Some(outs) = &last {
                        wire_all(outs, &e.ins, m.gap, out);
                    }
                    first.get_or_insert(e.ins);
                    last = Some(e.outs);
                    cx += uw + m.gap;
                }
                Ends { ins: first.unwrap_or_default(), outs: last.unwrap_or_default() }
            }
            Unit::Par(us) => {
                let mut cy = y;
                let mut e = Ends { ins: vec![], outs: vec![] };
                for u in us {
                    let (_, uh) = u.size(m);
                    let r = u.place(x, cy, m, out);
                    e.ins.extend(r.ins);
                    e.outs.extend(r.outs);
                    cy += uh + m.vgap;
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
                Ends { ins: vec![((x, iy), first)], outs: vec![((x + w, oy), last)] }
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
        let a = layout(&recipes::archive().root, Metrics::default());
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

    #[test]
    fn the_most_complex_recipe_has_its_group_and_its_join() {
        let l = layout(&recipes::archive().root, Metrics::default());
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].name, "For the phone");
        let tell = l.tile("a_tell").unwrap().rect;
        let into_tell = l.wires.iter().filter(|w| w.to == (tell.x, tell.mid_y())).count();
        assert_eq!(into_tell, 6, "NAS, Backup, the cue sheet, the .lrc files, MP3's library and the phone group");
        let phone = l.tile("a_ph").unwrap().rect;
        assert!(l.groups[0].rect.right() > phone.right());
    }
}
