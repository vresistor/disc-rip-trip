//! The map of a disc's sectors in a sidebar: a legend with counts and
//! where the bad sectors are, then blocks in rows, each the worst of its
//! sectors, with how far from the centre each row lies beside it. Draws any
//! [`SectorMap`], wherever it came from (Rip's read, Check and fix).
//!
//! Rows and blocks, never a drawing of the disc: only the distance from
//! the centre can be worked out, not where round it a sector lies (see
//! [`drt_steps::map`]).

use crate::theme::{font, mix, tnum, Pal};
use drt_steps::map::{cm, turn_line, where_line, Mark, SectorMap};
use egui::{pos2, vec2, Color32, Rect, RichText, Sense, Stroke, Ui};

/// Blocks to a row, at most and at least.
const COLS: usize = 36;
const MIN_COLS: usize = 20;
/// A block's side and the gap between blocks, at most.
const BLOCK: f32 = 13.0;
const GAP: f32 = 2.0;
/// Room for the distances beside the rows.
const SCALE_W: f32 = 64.0;

/// A mark's colour on the map. Each kind has a shape of its own too (see
/// [`draw`]), so nothing rests on colour alone; and the colours are from
/// Okabe and Ito's set, told apart with every common kind of colour
/// blindness: blue for put right, orange for C2, vermilion for bad.
pub fn colour(m: Mark, p: &Pal) -> Color32 {
    match m {
        Mark::NotRead => p.line,
        // Most of a disc reads cleanly: quiet, so what didn't stands out.
        Mark::Good => mix(p.muted, if p.dark { 0.45 } else { 0.32 }, p.raised),
        Mark::Recovered => Color32::from_rgb(0x00, 0x72, 0xB2),
        Mark::C2 => Color32::from_rgb(0xE6, 0x9F, 0x00),
        Mark::Wrong | Mark::Unreadable => Color32::from_rgb(0xD5, 0x5E, 0x00),
    }
}

/// Draws one block (or a legend swatch) in `r`: a shape for each kind, so
/// they're told apart without colour. Not read: an empty outline. Good: a
/// quiet fill. Put right: filled, a hole in the middle. C2: half filled.
/// Bad: filled, a cross through it.
pub fn draw(pt: &egui::Painter, r: Rect, m: Mark, p: &Pal) {
    let c = colour(m, p);
    let round = 2.0;
    match m {
        Mark::NotRead => {
            pt.rect_stroke(r.shrink(0.5), round, Stroke::new(1.0, c), egui::StrokeKind::Inside);
        }
        Mark::Good => {
            pt.rect_filled(r, round, c);
        }
        Mark::Recovered => {
            pt.rect_filled(r, round, c);
            pt.rect_filled(Rect::from_center_size(r.center(), r.size() * 0.42), 1.0, p.surface);
        }
        Mark::C2 => {
            pt.rect_stroke(r.shrink(0.5), round, Stroke::new(1.0, c), egui::StrokeKind::Inside);
            pt.rect_filled(Rect::from_min_max(pos2(r.left(), r.center().y), r.max), round, c);
        }
        Mark::Wrong | Mark::Unreadable => {
            pt.rect_filled(r, round, c);
            let k = r.shrink(r.width() * 0.24);
            let s = Stroke::new((r.width() * 0.13).max(1.2), Color32::WHITE);
            pt.line_segment([k.left_top(), k.right_bottom()], s);
            pt.line_segment([k.right_top(), k.left_bottom()], s);
        }
    }
}

/// What a mark's sectors are called, in the legend and on hover.
fn word(m: Mark) -> &'static str {
    match m {
        Mark::NotRead => "not read yet",
        Mark::Good => "good",
        Mark::Recovered => "recovered on a re-read",
        Mark::C2 => "with C2 errors",
        Mark::Wrong => "came back wrong",
        Mark::Unreadable => "unreadable",
    }
}

/// The legend's order: what's fine first, then what isn't.
const ORDER: [Mark; 6] = [Mark::Good, Mark::Recovered, Mark::C2, Mark::Wrong, Mark::Unreadable, Mark::NotRead];

fn thousands(n: u64) -> String {
    drt_steps::real::redumper::thousands(n)
}

/// The counts in words: *268,850 good · 140 unreadable · 11 with C2
/// errors*; only those there are.
pub fn counts_line(c: &[u64; 6]) -> String {
    ORDER.iter().filter(|m| c[**m as usize] > 0).map(|m| format!("{} {}", thousands(c[*m as usize]), word(*m))).collect::<Vec<_>>().join(" · ")
}

/// What hovering block `b` says: its sectors, how far out, and what's in it.
pub fn hover_text(m: &SectorMap, b: usize) -> String {
    let (a, z) = m.range(b);
    let mut lines = vec![if a == z { format!("Sector {}", thousands(a)) } else { format!("Sectors {}–{}", thousands(a), thousands(z)) }];
    if let Some(g) = m.geometry() {
        let (ra, rz) = (cm(g.radius(a as f64)), cm(g.radius(z as f64 + 1.0)));
        lines.push(if ra == rz { format!("About {ra} from the centre") } else { format!("About {}–{rz} from the centre", ra.trim_end_matches(" cm")) });
    }
    let c = m.counts(b);
    lines.push(counts_line(&c.map(|x| x as u64)));
    lines.join("\n")
}

/// Draws the map: the blocks, the distances beside them, the legend, and
/// where the bad sectors are.
pub fn show(ui: &mut Ui, p: &Pal, m: &SectorMap, t: f64) {
    if m.is_empty() {
        return;
    }
    // What it comes to first, then the blocks.
    ui.scope(|ui| legend(ui, p, m));
    for line in [where_line(m), turn_line(m)].into_iter().flatten() {
        ui.add(egui::Label::new(RichText::new(line).font(font(13.0, 400)).color(p.muted)).wrap());
    }
    ui.add_space(8.0);
    let geo = m.geometry();
    let w = ui.available_width();
    let room = w - if geo.is_some() { SCALE_W } else { 0.0 };
    let cols = COLS.min(((room + GAP) / (8.0 + GAP)).floor() as usize).max(MIN_COLS);
    let step = ((room + GAP) / cols as f32).min(BLOCK + GAP).floor();
    let side = step - GAP;
    let n = m.blocks();
    let rows = n.div_ceil(cols);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, rows as f32 * step - GAP), Sense::hover());
    let label = format!("Map of the disc: {}", counts_line(&m.totals));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Image, true, label.clone()));
    let pt = ui.painter();
    let at = |b: usize| pos2(rect.left() + (b % cols) as f32 * step, rect.top() + (b / cols) as f32 * step);
    let reading = m.reading();
    // The block being read breathes while things may move.
    let pulse = if crate::motion::on() && reading.is_some() {
        crate::app::tick(ui.ctx());
        0.55 + 0.45 * (t as f32 * 3.0).sin().abs()
    } else {
        1.0
    };
    for b in 0..n {
        let r = Rect::from_min_size(at(b), vec2(side, side));
        if Some(b) == reading {
            // Being read: a thick ring, breathing, not a colour of its own.
            draw(pt, r, m.mark(b), p);
            pt.rect_stroke(r.expand(1.0), 2.0, Stroke::new(2.0, p.ink.gamma_multiply(pulse)), egui::StrokeKind::Outside);
        } else {
            draw(pt, r, m.mark(b), p);
        }
    }
    // How far from the centre, beside the rows: each whole and half
    // centimetre at the row it's reached in.
    if let Some(g) = geo {
        let x = rect.left() + cols as f32 * step + 6.0;
        let lba_at = |r: f64| (std::f64::consts::PI * (r * r - g.r0 * g.r0) / (g.sector_len() * g.pitch)).max(0.0);
        // Whole centimetres first, then the halves between where there's room.
        let end = g.radius(m.sectors as f64);
        let y_of = |r: f64| rect.top() + (lba_at(r) / m.per_block as f64 / cols as f64).floor() as f32 * step + side / 2.0;
        let mut shown: Vec<(f32, f64)> = vec![];
        for half in [false, true] {
            for mm in (25..=(end * 1000.0) as u32).step_by(5) {
                let r = mm as f64 / 1000.0;
                let y = y_of(r);
                if (mm % 10 == 5) == half && y <= rect.bottom() && shown.iter().all(|(sy, _)| (sy - y).abs() >= 22.0) {
                    shown.push((y, r));
                }
            }
        }
        for (y, r) in shown {
            pt.text(pos2(x, y), egui::Align2::LEFT_CENTER, cm(r), tnum(11.5, 400), p.muted);
        }
    }
    // Hovering a block: its sectors, how far out, and what's in them.
    if let Some(pos) = resp.hover_pos() {
        let (cx, cy) = (((pos.x - rect.left()) / step).floor(), ((pos.y - rect.top()) / step).floor());
        let b = cy as usize * cols + cx as usize;
        if cx >= 0.0 && (cx as usize) < cols && cy >= 0.0 && b < n {
            let r = Rect::from_min_size(at(b), vec2(side, side));
            ui.painter().rect_stroke(r.expand(1.0), 2.0, Stroke::new(1.5, p.ink), egui::StrokeKind::Outside);
            resp.on_hover_text_at_pointer(hover_text(m, b));
        }
    }
    ui.add_space(8.0);
}

/// A swatch for each kind of sector there is, with how many.
fn legend(ui: &mut Ui, p: &Pal, m: &SectorMap) {
    // Rows only as tall as a line of text.
    ui.spacing_mut().interact_size.y = 16.0;
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        // Each swatch with its count, kept together on a line.
        for k in ORDER.iter().copied().filter(|k| m.total(*k) > 0) {
            let g = ui.painter().layout_no_wrap(format!("{} {}", thousands(m.total(k)), word(k)), tnum(13.0, 400), p.ink);
            let (r, _) = ui.allocate_exact_size(vec2(16.0 + g.size().x, g.size().y.max(16.0)), Sense::hover());
            draw(ui.painter(), Rect::from_min_size(pos2(r.left(), r.center().y - 6.0), vec2(12.0, 12.0)), k, p);
            ui.painter().galley(pos2(r.left() + 16.0, r.center().y - g.size().y / 2.0), g, p.ink);
        }
    });
    ui.add_space(2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legend_and_hover_say_only_what_there_is() {
        let mut c = [0u64; 6];
        c[Mark::Good as usize] = 268_850;
        c[Mark::Unreadable as usize] = 140;
        c[Mark::C2 as usize] = 11;
        assert_eq!(counts_line(&c), "268,850 good · 11 with C2 errors · 140 unreadable");
        let mut s = drt_steps::map::Sectors::new(300_000);
        for k in 0..300_000 {
            s.set(k, Mark::Good);
        }
        s.set(147_108, Mark::Unreadable);
        let m = s.map(None, true);
        let b = (147_108 / m.per_block) as usize;
        assert_eq!(hover_text(&m, b), "Sectors 147,000–147,299\nAbout 4.4 cm from the centre\n299 good · 1 unreadable");
    }
}
