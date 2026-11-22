//! A drawing of the disc, to scale: the hole in the middle, where the
//! drive holds it, and the data area as rings from the first sector out,
//! each the worst of its sectors. Drawn from any [`SectorMap`] of a CD,
//! whose sectors' distances from the centre are known.
//!
//! Rings, not spots: where round the disc a sector lies can't be worked
//! out (see [`drt_steps::map`]), so a ring is drawn the whole way round,
//! and where errors come back once per turn to a single spot its ring is
//! dashed, never a dot at a made-up place.

use crate::sectormap::{colour, range_text};
use crate::theme::{font, mix, tnum, Pal};
use drt_steps::map::{cm, Mark, Ring, SectorMap};
use egui::{pos2, vec2, Color32, Pos2, Sense, Shape, Stroke, Ui};

/// A CD's edge, its hole, and where a drive holds it (IEC 60908: a 15 mm
/// hole, clamped from 26 to 33 mm across), in metres from the centre.
pub const EDGE: f64 = 0.060;
const HOLE: f64 = 0.0075;
const CLAMP: (f64, f64) = (0.013, 0.0165);
/// Rings at least this wide, in metres, and a pixel at least.
pub const RING: f64 = 0.000_25;
/// The drawing's widest, in pixels.
const MAX_D: f32 = 420.0;
/// Room under the disc for its distances.
const RULER_H: f32 = 30.0;
/// A spot's dashed ring, wide enough to see.
const SPOT_W: f32 = 2.5;
/// The distances marked under it, in millimetres.
const TICKS: [u32; 7] = [25, 30, 35, 40, 45, 50, 55];

/// The widest a ring is drawn when there's something in it: thinner and
/// it wouldn't be seen. Bad wider than put right, so the two differ by
/// more than their colour.
fn least_width(m: Mark) -> f32 {
    match m {
        Mark::NotRead | Mark::Good => 0.0,
        Mark::Recovered | Mark::Repaired => 1.5,
        Mark::C2 | Mark::Wrong | Mark::Unreadable => 2.0,
    }
}

/// Points round a circle, for a dashed ring.
fn circle(c: Pos2, r: f32) -> Vec<Pos2> {
    let n = ((r * 0.8) as usize).clamp(48, 400);
    (0..=n).map(|k| k as f32 / n as f32 * std::f32::consts::TAU).map(|a| c + vec2(a.cos(), a.sin()) * r).collect()
}

/// A dashed ring, for errors that come back to a single spot on it.
fn dashed(pt: &egui::Painter, c: Pos2, r: f32, s: Stroke) {
    pt.extend(Shape::dashed_line(&circle(c, r), s, 4.0, 3.0));
}

/// The rings in stretches with the same mark side by side, so the quiet
/// most of a disc is drawn in a few wide strokes: inner, outer, mark.
fn stretches(rings: &[Ring]) -> Vec<(f64, f64, Mark)> {
    let mut out: Vec<(f64, f64, Mark)> = vec![];
    for r in rings {
        // A spot's ring is drawn dashed over the quiet colour.
        let m = if r.spot { Mark::Good } else { r.mark };
        match out.last_mut() {
            Some(s) if s.2 == m => s.1 = r.outer,
            _ => out.push((r.inner, r.outer, m)),
        }
    }
    out
}

/// What hovering ring `r` says: how far out, its sectors, what's in them.
pub fn hover_text(m: &SectorMap, r: &Ring) -> String {
    let (a, _) = m.range(r.first);
    let (_, z) = m.range(r.last);
    let mut t = range_text(m, a, z, &r.counts);
    if r.spot {
        t.push_str("\nErrors come back to a single spot somewhere on this ring");
    }
    t
}

/// Draws the disc with its rings, the distances under it, and what a
/// dashed ring means when there is one.
pub fn show(ui: &mut Ui, p: &Pal, m: &SectorMap, t: f64) {
    let Some(g) = m.geometry() else { return };
    let w = ui.available_width();
    let d = w.min(MAX_D).floor();
    let big = d / 2.0;
    let scale = big as f64 / EDGE;
    let rings = m.rings(RING.max(1.0 / scale));
    let (rect, resp) = ui.allocate_exact_size(vec2(w, d + RULER_H), Sense::hover());
    let label = format!("Map of the disc: {}", crate::sectormap::counts_line(&m.totals));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Image, true, label.clone()));
    let pt = ui.painter();
    let c = pos2(rect.left() + big, rect.top() + big);
    let px = |r: f64| (r.min(EDGE) * scale) as f32;
    // A ring from `a` to `b` pixels out, at least `least` wide (about its
    // middle). egui strokes a circle outside its radius.
    let band = |pt: &egui::Painter, a: f32, b: f32, col: Color32, least: f32| {
        let w = (b - a).max(least);
        pt.circle_stroke(c, (a + b) / 2.0 - w / 2.0, Stroke::new(w, col));
    };
    let ring = |pt: &egui::Painter, inner: f64, outer: f64, col: Color32, least: f32| band(pt, px(inner), px(outer), col, least);
    // The disc: blank where nothing's recorded, the hole, where it's held.
    let body = if p.dark { mix(p.ink, 0.05, p.raised) } else { mix(p.ink, 0.03, p.raised) };
    pt.circle_filled(c, big, body);
    ring(pt, CLAMP.0, CLAMP.1, mix(p.line, 0.6, body), 0.0);
    pt.circle_filled(c, px(HOLE), p.raised);
    for r in [px(HOLE), big - 1.0] {
        pt.circle_stroke(c, r, Stroke::new(1.0, p.line));
    }
    // The data area: the quiet rings in wide strokes (a hair wider, so no
    // seams show between them), then what wasn't read cleanly over them,
    // the worst last.
    let spans = stretches(&rings);
    for (a, b, mk) in spans.iter().filter(|s| least_width(s.2) == 0.0) {
        let col = if *mk == Mark::NotRead { mix(p.line, 0.7, body) } else { colour(*mk, p) };
        band(pt, px(*a) - 0.3, px(*b) + 0.3, col, 0.0);
    }
    let mut loud: Vec<&(f64, f64, Mark)> = spans.iter().filter(|s| least_width(s.2) > 0.0).collect();
    loud.sort_by_key(|s| Mark::ALL.iter().position(|m| *m == s.2));
    for (a, b, mk) in loud {
        ring(pt, *a, *b, colour(*mk, p), least_width(*mk));
    }
    // A spot's ring: dashes in a channel of blank disc, so they show
    // against rings of their own colour too.
    for r in rings.iter().filter(|r| r.spot) {
        let mid = (px(r.inner) + px(r.outer)) / 2.0;
        band(pt, mid, mid, body, SPOT_W + 3.0);
        dashed(pt, c, mid, Stroke::new(SPOT_W, colour(r.mark, p)));
    }
    // Round the ring being read: a hairline either side, breathing while
    // things may move.
    let bracket = |pt: &egui::Painter, r: &Ring, s: Stroke| {
        let least = least_width(r.mark).max(if r.spot { SPOT_W + 3.0 } else { 0.0 });
        let (a, b) = (px(r.inner), px(r.outer));
        let half = (b - a).max(least) / 2.0 + 1.5;
        for x in [(a + b) / 2.0 - half, (a + b) / 2.0 + half] {
            pt.circle_stroke(c, x - s.width / 2.0, s);
        }
    };
    if let Some(b) = m.reading() {
        let pulse = if crate::motion::on() {
            crate::app::tick(ui.ctx());
            0.55 + 0.45 * (t as f32 * 3.0).sin().abs()
        } else {
            1.0
        };
        if let Some(r) = rings.iter().find(|r| (r.first..=r.last).contains(&b)) {
            bracket(pt, r, Stroke::new(1.0, p.ink.gamma_multiply(pulse)));
        }
    }
    // How far from the centre, under the disc, as a ruler laid from the
    // centre out would read.
    let y = rect.top() + d + 6.0;
    let faint = Stroke::new(1.0, p.line);
    pt.line_segment([pos2(c.x, y), pos2(c.x + big, y)], faint);
    pt.line_segment([pos2(c.x, y - 3.0), pos2(c.x, y + 3.0)], faint);
    // Every tick; a number on the whole centimetres, and on the halves
    // between where there's room.
    let mut shown: Vec<(f32, f32)> = vec![];
    for half in [false, true] {
        for mm in TICKS.into_iter().filter(|mm| (mm % 10 != 0) == half) {
            let x = c.x + px(mm as f64 / 1000.0);
            pt.line_segment([pos2(x, y), pos2(x, y + if half { 4.0 } else { 6.0 })], Stroke::new(1.0, p.muted));
            let n = cm(mm as f64 / 1000.0);
            let gal = pt.layout_no_wrap(n.trim_end_matches(" cm").trim_end_matches(".0").to_owned(), tnum(11.0, 400), p.muted);
            let (l, r) = (x - gal.size().x / 2.0, x + gal.size().x / 2.0);
            if shown.iter().all(|(a, b)| l >= b + 5.0 || r + 5.0 <= *a) {
                shown.push((l, r));
                pt.galley(pos2(l, y + 8.0), gal, p.muted);
            }
        }
    }
    pt.text(pos2(c.x + big + 4.0, y + 8.0), egui::Align2::LEFT_TOP, "cm", tnum(11.0, 400), p.muted);
    // Hovering: the ring under the pointer, how far out, what's in it.
    if let Some(pos) = resp.hover_pos() {
        let r = (pos - c).length() as f64 / scale;
        if let Some(rg) = rings.iter().find(|x| (x.inner..x.outer).contains(&r)) {
            bracket(ui.painter(), rg, Stroke::new(1.0, p.ink));
            resp.on_hover_text_at_pointer(hover_text(m, rg));
        } else if (HOLE..=EDGE).contains(&r) && pos.y <= rect.top() + d {
            let say = if (CLAMP.0..=CLAMP.1).contains(&r) {
                "Where the drive holds the disc"
            } else if r < g.r0 {
                "Nothing is recorded this near the centre"
            } else {
                "Nothing is recorded this far out"
            };
            resp.on_hover_text_at_pointer(say);
        }
    }
    // What a dashed ring means, when there's one.
    if rings.iter().any(|r| r.spot) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 14.0), Sense::hover());
            let mark = rings.iter().find(|r| r.spot).map_or(Mark::Unreadable, |r| r.mark);
            ui.painter().extend(Shape::dashed_line(&[pos2(r.left(), r.center().y), pos2(r.right(), r.center().y)], Stroke::new(SPOT_W, colour(mark, p)), 4.0, 3.0));
            ui.label(egui::RichText::new("a single spot somewhere on this ring").font(font(13.0, 400)).color(p.ink));
        });
    }
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_rings_are_drawn_together_and_a_spot_s_ring_over_them() {
        let r = |k: u32, m: Mark, spot: bool| Ring { first: 0, last: 0, inner: k as f64, outer: k as f64 + 1.0, mark: m, counts: [0; drt_steps::map::KINDS], spot };
        let rings = [r(25, Mark::Good, false), r(26, Mark::Good, false), r(27, Mark::Recovered, true), r(28, Mark::Good, false), r(29, Mark::C2, false), r(30, Mark::NotRead, false)];
        assert_eq!(stretches(&rings), [(25.0, 29.0, Mark::Good), (29.0, 30.0, Mark::C2), (30.0, 31.0, Mark::NotRead)]);
    }

    #[test]
    fn hovering_a_ring_says_how_far_out_and_what_s_in_it() {
        let mut s = drt_steps::map::Sectors::new(333_000);
        for k in 0..333_000 {
            s.set(k, Mark::Good);
        }
        s.set(148_500, Mark::Unreadable);
        let m = s.map(None, true);
        let rings = m.rings(RING);
        let rg = rings.iter().find(|r| r.mark == Mark::Unreadable).unwrap();
        let t = hover_text(&m, rg);
        assert!(t.starts_with("Sectors 14") && t.contains("\nAbout 4.4 cm from the centre\n") && t.ends_with(" good · 1 unreadable"), "{t}");
    }
}
