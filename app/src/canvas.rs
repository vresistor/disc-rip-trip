//! The recipe on its canvas: drawn from drt-layout and the live snapshot,
//! with one transform (pan, zoom) applied to every coordinate and every
//! size, text included, so zooming scales the whole picture uniformly.

use crate::theme::{font, kind_icon, mix, Pal};
use drt_engine::{Snapshot, StepState};
use drt_layout::Layout;
use egui::epaint::{CubicBezierShape, PathShape, RectShape};
use egui::{pos2, vec2, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};

/// Where the board's (0, 0) sits from the canvas's top left at first, as
/// the mockup's `.board { margin: 28px 32px }`.
pub const MARGIN: Vec2 = vec2(32.0, 28.0);

/// How much one point of scroll zooms: a 50-point wheel notch is ×1.28.
const ZOOM_PER_POINT: f32 = 1.0 / 200.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f32,
    /// Screen offset of the board's origin from the canvas's top left.
    pub pan: Vec2,
}

impl View {
    pub fn to_screen(&self, origin: Pos2, x: f32, y: f32) -> Pos2 {
        origin + self.pan + vec2(x, y) * self.zoom
    }

    /// Zooms about a point on screen, keeping what's under it still.
    pub fn zoom_about(&mut self, origin: Pos2, at: Pos2, new_zoom: f32) {
        let new_zoom = new_zoom.clamp(0.2, 3.0);
        let rel = at - origin;
        self.pan = rel - (rel - self.pan) * (new_zoom / self.zoom);
        self.zoom = new_zoom;
    }

    /// Pans just enough that the layout rect `r` is inside `area` (from
    /// the canvas's top left), with a margin.
    pub fn reveal(&mut self, r: drt_layout::Rect, area: Vec2) {
        let m = 24.0;
        let min = self.pan + vec2(r.x, r.y) * self.zoom;
        let max = min + vec2(r.w, r.h) * self.zoom;
        for (lo, hi, lim, d) in [(min.x, max.x, area.x, 0), (min.y, max.y, area.y, 1)] {
            let shift = if hi > lim - m {
                (lim - m - hi).max(m - lo)
            } else if lo < m {
                m - lo
            } else {
                0.0
            };
            if d == 0 {
                self.pan.x += shift
            } else {
                self.pan.y += shift
            }
        }
    }

    /// Fits the whole layout into `area` (relative to the canvas).
    pub fn fit(&mut self, lay: &Layout, area: Vec2) {
        let z = ((area.x - 2.0 * MARGIN.x) / lay.width).min((area.y - 2.0 * MARGIN.y) / lay.height).min(1.0);
        self.zoom = z.max(0.2);
        self.pan = (area - vec2(lay.width, lay.height) * self.zoom) / 2.0;
    }
}

/// What was clicked on the canvas.
pub enum Click {
    Tile(String),
    /// The + after the strip ending in this step.
    Plus(String),
    /// The canvas itself, away from any tile.
    Empty,
}

/// What the canvas draws besides the recipe.
pub struct Marks<'a> {
    /// The tile whose sidebar is open, outlined.
    pub open: Option<&'a str>,
    /// Show a + after every strip, and which one's picker is open.
    pub editing: bool,
    pub plus_open: Option<&'a str>,
    /// Tiles a Remove would take with it, marked in red.
    pub doomed: &'a [String],
    /// The window's corner radius, for the canvas's bottom-right corner.
    pub round: f32,
    /// One-shot cues playing: a tile that just finished, failed or asked,
    /// with the state it turned to and when.
    pub cues: &'a std::collections::HashMap<String, (StepState, f64)>,
    /// Steps after which nothing could be added: no + there.
    pub dead: &'a std::collections::HashSet<String>,
    /// What each card shows besides its step's view: its settings, its
    /// cover.
    pub cards: &'a std::collections::HashMap<String, CardExtra>,
    /// The clock the animations run on, in seconds.
    pub t: f64,
}

/// What a card shows that isn't in its step's view.
pub struct CardExtra {
    pub settings: Vec<crate::card::CardSetting>,
    pub locked: bool,
    pub retries: bool,
    pub asking: bool,
    pub picture: crate::card::Picture,
}

/// Draws the canvas into `rect` and handles panning and zooming over it.
pub fn show(ui: &mut Ui, rect: Rect, view: &mut View, lay: &Layout, snap: &Snapshot, p: &Pal, marks: Marks) -> Option<Click> {
    let t = marks.t;
    let resp = ui.interact(rect, ui.scope_id().with("canvas"), Sense::click_and_drag());
    if resp.dragged() {
        view.pan += resp.drag_delta();
    }
    if resp.contains_pointer() {
        let (scroll, zd, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
        // Any scroll over the canvas zooms about the pointer (egui has
        // already smoothed it); Ctrl+wheel and pinch arrive as `zd`.
        // Dragging pans.
        let factor = zd * (scroll.y * ZOOM_PER_POINT).exp();
        if factor != 1.0 {
            view.zoom_about(rect.min, hover.unwrap_or(rect.center()), view.zoom * factor);
        }
    }

    let painter = ui.painter_at(rect);
    let corner = egui::CornerRadius { se: marks.round as u8, ..Default::default() };
    painter.rect_filled(rect, corner, p.surface);
    // Dots, 20 px apart on screen, moving with the board.
    let step = 20.0;
    let off = vec2(view.pan.x.rem_euclid(step), view.pan.y.rem_euclid(step));
    let mut dots = Vec::new();
    let mut y = rect.top() + off.y - step / 2.0;
    while y < rect.bottom() {
        let mut x = rect.left() + off.x - step / 2.0;
        while x < rect.right() {
            // None in the window's rounded corner.
            if !(x > rect.right() - marks.round && y > rect.bottom() - marks.round) {
                dots.push(Shape::circle_filled(pos2(x, y), 1.0, p.line));
            }
            x += step;
        }
        y += step;
    }
    painter.extend(dots);

    let z = view.zoom;
    let o = rect.min;
    let at = |x: f32, y: f32| view.to_screen(o, x, y);

    // Groups: a dashed rounded box with its name.
    for g in &lay.groups {
        let r = Rect::from_min_max(at(g.rect.x, g.rect.y), at(g.rect.right(), g.rect.y + g.rect.h));
        let tint = Color32::from_rgba_unmultiplied(p.c_make.r(), p.c_make.g(), p.c_make.b(), 10);
        painter.rect_filled(r, 14.0 * z, tint);
        let col = mix(p.c_make, 0.55, p.line);
        painter.extend(Shape::dashed_line(&rounded_outline(r, 14.0 * z), Stroke::new(1.5 * z, col), 5.0 * z, 3.5 * z));
        painter.text(r.min + vec2(16.0, 8.0) * z, Align2::LEFT_TOP, &g.name, font(13.0 * z, 500), p.muted);
    }

    // Wires: straight, then a cubic. Into a busy tile, the busy colour,
    // its dashes moving toward it (still, without motion); between two
    // done tiles, a hint of green.
    let ws = Stroke::new(2.0 * z, p.faint);
    let state = |id: &str| snap.steps.get(id).map(|v| v.state);
    let mut marching = false;
    for w in &lay.wires {
        // Into a tile working on what reached it, even while the one before
        // is still busy (Rip streams tracks into the encoders); not into one
        // only waiting for the end (Next disc).
        let into_busy = snap.steps.get(&w.to_step).is_some_and(|v| v.state == StepState::Busy && (!v.items.is_empty() || v.progress.is_some()));
        if into_busy {
            let col = Color32::from_rgba_unmultiplied(p.busy.r(), p.busy.g(), p.busy.b(), 180);
            let pts: Vec<Pos2> = w.points(24).into_iter().map(|(x, y)| at(x, y)).collect();
            let (dash, gap) = (6.0 * z, 6.0 * z);
            // Dashes move toward the tile at 24 px/s: egui starts the first
            // dash `offset` along the line, so a growing offset moves them on.
            let shift = if crate::motion::on() { ((t * 24.0) as f32 * z) % (dash + gap) } else { 0.0 };
            painter.extend(Shape::dashed_line_with_offset(&pts, Stroke::new(2.0 * z, col), &[dash], &[gap], shift));
            marching = true;
            continue;
        }
        // Into a step that hasn't run: part of the plan, dashed and faint.
        let plan = snap.steps.get(&w.to_step).is_some_and(|v| v.record.started.is_none() && v.step != "drive" && matches!(v.state, StepState::Idle | StepState::Wait));
        if plan {
            let col = mix(p.faint, 0.6, p.surface);
            let pts: Vec<Pos2> = w.points(24).into_iter().map(|(x, y)| at(x, y)).collect();
            painter.extend(Shape::dashed_line(&pts, Stroke::new(2.0 * z, col), 6.0 * z, 6.0 * z));
            continue;
        }
        let done = state(&w.from_step) == Some(StepState::Ok) && state(&w.to_step) == Some(StepState::Ok);
        let ws = if done { Stroke::new(2.0 * z, mix(p.ok, 0.35, p.faint)) } else { ws };
        let (a, b) = (at(w.from.0, w.from.1), at(w.bend.0, w.bend.1));
        if (b - a).length() > 0.1 {
            painter.line_segment([a, b], ws);
        }
        painter.add(CubicBezierShape::from_points_stroke([b, at(w.c1.0, w.c1.1), at(w.c2.0, w.c2.1), at(w.to.0, w.to.1)], false, Color32::TRANSPARENT, ws));
    }
    let _ = marching;

    // Next disc's line back into the start: rounded corners, an arrow in.
    for path in &lay.loops {
        let pts: Vec<Pos2> = path.iter().map(|&(x, y)| at(x, y)).collect();
        if pts.len() < 2 {
            continue;
        }
        painter.add(Shape::line(rounded(&pts, 16.0 * z), ws));
        let tip = pts[pts.len() - 1];
        let dir = (tip - pts[pts.len() - 2]).normalized();
        let side = vec2(-dir.y, dir.x);
        let (len, half) = (9.0 * z, 5.0 * z);
        let base = tip - dir * len;
        painter.add(PathShape::convex_polygon(vec![tip, base + side * half, base - side * half], p.faint, Stroke::NONE));
    }

    // Tiles, then the notches that point into the next tile over them.
    let mut click = None;
    let mut notches = Vec::new();
    let measure = crate::card::measurer(&painter);
    let none = CardExtra { settings: vec![], locked: false, retries: false, asking: false, picture: crate::card::Picture::None };
    for tp in &lay.tiles {
        let Some(v) = snap.steps.get(&tp.id) else { continue };
        let extra = marks.cards.get(&tp.id).unwrap_or(&none);
        let card = crate::card::build(
            &crate::card::Input { v, settings: &extra.settings, locked: extra.locked, cover: !matches!(extra.picture, crate::card::Picture::None), asking: extra.asking, retries: extra.retries },
            &measure,
        );
        let r = Rect::from_min_max(at(tp.rect.x, tp.rect.y), at(tp.rect.right(), tp.rect.y + tp.rect.h));
        // Each tile is a widget too, for clicks and the accessibility tree;
        // off-screen ones stay in the tree so a screen reader can reach them.
        let resp = ui.interact(r, ui.scope_id().with(("tile", &tp.id)), Sense::click());
        let label = card.said();
        resp.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::Button, true, marks.open == Some(tp.id.as_str()), &label));
        if resp.clicked() {
            click = Some(Click::Tile(tp.id.clone()));
        }
        let hovered = resp.hovered();
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // A setting's help over its label; else what had to be shortened, in full.
        let help = resp.hover_pos().and_then(|pos| card.help_at((pos - r.min) / z));
        let resp = match help {
            Some(h) => resp.on_hover_text(h),
            None if !card.cut.is_empty() => resp.on_hover_text(card.cut.join("\n")),
            None => resp,
        };
        if !r.intersects(rect) {
            continue;
        }
        // A cue just played: a wash of its colour comes in and fades.
        let cue = marks.cues.get(&tp.id).filter(|(st, _)| *st == card.shown.cue_state()).map(|(st, t0)| (*st, *t0));
        let mut fill = p.raised;
        if let Some((st, t0)) = cue {
            let k = crate::motion::since(ui.ctx(), t0, 0.6);
            let col = match st {
                StepState::Ok => p.ok,
                StepState::Bad => p.bad,
                _ => p.warn,
            };
            if st != StepState::Needs {
                let a = 0.18 * (1.0 - crate::motion::cubic_out(k));
                fill = mix(col, a, p.raised);
            }
        }
        if hovered {
            fill = mix(p.ink, 0.025, fill);
        }
        // Pressed, the tile settles in a pixel.
        let r = if resp.is_pointer_button_down_on() { r.shrink(z) } else { r };
        let rad = (10.0 * z).round().min(255.0) as u8;
        let cr = CornerRadius {
            nw: if tp.in_strip.round_left() { rad } else { 0 },
            sw: if tp.in_strip.round_left() { rad } else { 0 },
            ne: if tp.in_strip.round_right() { rad } else { 0 },
            se: if tp.in_strip.round_right() { rad } else { 0 },
        };
        if card.plan {
            // A blueprint: a see-through ground and a dashed edge.
            let fill = Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 140);
            painter.add(RectShape::new(r, cr, fill, Stroke::NONE, StrokeKind::Inside));
            let k = |c: u8| c as f32;
            let edge = outline(r.shrink(0.75 * z), [k(cr.nw), k(cr.ne), k(cr.se), k(cr.sw)]);
            painter.extend(Shape::dashed_line(&edge, Stroke::new(1.5 * z, p.faint), 6.0 * z, 4.0 * z));
        } else {
            painter.add(RectShape::new(r, cr, fill, Stroke::new(z, p.line), StrokeKind::Inside));
        }
        // A tick (or a cross) that just came pops in.
        let pop = match cue.map(|(st, t0)| (st, crate::motion::since(ui.ctx(), t0, 0.24))) {
            Some((StepState::Ok, k)) => 0.6 + 0.4 * crate::motion::back_out(k),
            Some((StepState::Bad, k)) => 0.6 + 0.4 * crate::motion::cubic_out(k),
            _ => 1.0,
        };
        crate::card::paint(&painter, r, z, &card, v, p, t, pop, &extra.picture);
        // What's working, needing you or failed is outlined in its colour;
        // else the tile whose sidebar is open, in the action colour.
        if marks.doomed.contains(&tp.id) {
            painter.add(RectShape::new(r, cr, Color32::from_rgba_unmultiplied(p.bad.r(), p.bad.g(), p.bad.b(), 28), Stroke::new(2.0 * z, p.bad), StrokeKind::Inside));
        } else if let Some(c) = card.shown.outline(p) {
            painter.add(RectShape::stroke(r, cr, Stroke::new(2.0 * z, c), StrokeKind::Inside));
        } else if marks.open == Some(tp.id.as_str()) {
            painter.add(RectShape::stroke(r, cr, Stroke::new(2.0 * z, p.action), StrokeKind::Inside));
        }
        // A question rings once: a ring grows out and fades.
        if let Some((StepState::Needs, t0)) = cue {
            let k = crate::motion::since(ui.ctx(), t0, 0.7);
            if k < 1.0 {
                let e = crate::motion::cubic_out(k);
                let a = ((1.0 - k) * 255.0) as u8;
                let col = Color32::from_rgba_unmultiplied(p.warn.r(), p.warn.g(), p.warn.b(), a);
                painter.add(RectShape::stroke(r.expand(8.0 * z * e), cr, Stroke::new(2.0 * z, col), StrokeKind::Outside));
            }
        }
        // The keyboard's focus.
        if resp.has_focus() && crate::widgets::kb_focus(ui.ctx()) {
            painter.add(RectShape::stroke(r.expand(2.0), CornerRadius::same(rad.saturating_add(2)), Stroke::new(2.0, p.action), StrokeKind::Outside));
        }
        if tp.in_strip.notch() {
            notches.push((pos2(r.right(), r.center().y), fill));
        }
    }
    let h = 7.07 * z;
    for (c, fill) in notches {
        let pts = vec![pos2(c.x, c.y - h), pos2(c.x + h, c.y), pos2(c.x, c.y + h), pos2(c.x - h, c.y)];
        painter.add(PathShape::convex_polygon(pts, fill, Stroke::NONE));
        painter.add(PathShape::line(vec![pos2(c.x, c.y - h), pos2(c.x + h, c.y), pos2(c.x, c.y + h)], Stroke::new(z, p.line)));
    }
    // The + after each strip, while editing.
    if marks.editing {
        for (id, (x, y)) in lay.pluses.iter().filter(|(id, _)| !marks.dead.contains(id)) {
            let c = at(*x, *y);
            let name = snap.steps.get(id).map_or(id.clone(), |v| {
                // Two strips ending in a Library: say which, by its folder.
                let twin = lay.pluses.iter().any(|(o, _)| o != id && snap.steps.get(o).is_some_and(|w| w.name == v.name));
                if twin && !v.from.is_empty() {
                    format!("{}, {}", v.name, v.from)
                } else {
                    v.name.clone()
                }
            });
            let hit = Rect::from_center_size(c, vec2(26.0, 26.0) * z);
            let resp = ui.interact(hit, ui.scope_id().with(("plus", id)), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, format!("Add after {name}")));
            let on = marks.plus_open == Some(id.as_str());
            let (fill, fg, line) = if on { (p.action, p.action_ink, p.action) } else { (p.raised, p.muted, p.line) };
            let fill = if resp.hovered() && !on { mix(p.ink, 0.05, fill) } else { fill };
            painter.circle(c, 13.0 * z, fill, Stroke::new(1.5 * z, line));
            painter.extend(kind_icon("plus", c - vec2(8.0, 8.0) * z, 16.0 * z, fg));
            if resp.clicked() {
                click = Some(Click::Plus(id.clone()));
            }
        }
        // A smaller, quieter + on each seam inside a strip, to put a step
        // between two: it adds in line after the tile on its left.
        for (id, (x, y)) in lay.seams.iter().filter(|(id, _)| !marks.dead.contains(id)) {
            let c = at(*x, *y);
            let name_of = |id: &str| snap.steps.get(id).map_or(id.to_owned(), |v| v.name.clone());
            let right = lay.tiles.iter().position(|t| t.id == *id).and_then(|k| lay.tiles.get(k + 1)).map(|t| name_of(&t.id)).unwrap_or_default();
            let hit = Rect::from_center_size(c, vec2(20.0, 20.0) * z);
            let resp = ui.interact(hit, ui.scope_id().with(("seam", id)), Sense::click());
            let label = format!("Add between {} and {right}", name_of(id));
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &label));
            let on = marks.plus_open == Some(id.as_str());
            let (fill, fg, line) = if on {
                (p.action, p.action_ink, p.action)
            } else if resp.hovered() {
                (mix(p.ink, 0.05, p.raised), p.ink, p.faint)
            } else {
                (p.raised, p.faint, p.line)
            };
            painter.circle(c, 8.5 * z, fill, Stroke::new(1.0 * z, line));
            painter.extend(kind_icon("plus", c - vec2(5.5, 5.5) * z, 11.0 * z, fg));
            if resp.clicked() {
                click = Some(Click::Plus(id.clone()));
            }
        }
    }
    // The canvas itself, where no tile or + took the click.
    if click.is_none() && resp.clicked() {
        click = Some(Click::Empty);
    }
    click
}

/// A polyline with its corners rounded to `r` (less where a leg is short),
/// as points; the last leg stops short so the arrowhead sits on it.
fn rounded(pts: &[Pos2], r: f32) -> Vec<Pos2> {
    let mut out = vec![pts[0]];
    for w in pts.windows(3) {
        let (a, b, c) = (w[0], w[1], w[2]);
        let r = r.min((b - a).length() / 2.0).min((c - b).length() / 2.0);
        let (p0, p1) = (b - (b - a).normalized() * r, b + (c - b).normalized() * r);
        // A quadratic curve through the corner, as a few points.
        for k in 0..=6 {
            let t = k as f32 / 6.0;
            let u = 1.0 - t;
            out.push(pos2(u * u * p0.x + 2.0 * u * t * b.x + t * t * p1.x, u * u * p0.y + 2.0 * u * t * b.y + t * t * p1.y));
        }
    }
    let (a, b) = (pts[pts.len() - 2], pts[pts.len() - 1]);
    out.push(b - (b - a).normalized() * (9.0 * r / 16.0));
    out
}

/// A rectangle's outline with its own radius at each corner (nw, ne, se,
/// sw), as a closed polyline, for dashing.
fn outline(r: Rect, rad: [f32; 4]) -> Vec<Pos2> {
    let [nw, ne, se, sw] = rad;
    let corners = [
        (pos2(r.right() - ne, r.top() + ne), -90.0f32, ne),
        (pos2(r.right() - se, r.bottom() - se), 0.0, se),
        (pos2(r.left() + sw, r.bottom() - sw), 90.0, sw),
        (pos2(r.left() + nw, r.top() + nw), 180.0, nw),
    ];
    let mut pts = Vec::new();
    for (c, a0, rad) in corners {
        for i in 0..=6 {
            pts.push(c + Vec2::angled((a0 + i as f32 * 15.0).to_radians()) * rad);
        }
    }
    pts.push(pts[0]);
    pts
}

/// A rounded rectangle's outline as a closed polyline, for dashing.
fn rounded_outline(r: Rect, rad: f32) -> Vec<Pos2> {
    let mut pts = Vec::new();
    let corners = [
        (pos2(r.right() - rad, r.top() + rad), -90.0f32),
        (pos2(r.right() - rad, r.bottom() - rad), 0.0),
        (pos2(r.left() + rad, r.bottom() - rad), 90.0),
        (pos2(r.left() + rad, r.top() + rad), 180.0),
    ];
    for (c, a0) in corners {
        for i in 0..=6 {
            let a = (a0 + i as f32 * 15.0).to_radians();
            pts.push(c + Vec2::angled(a) * rad);
        }
    }
    pts.push(pts[0]);
    pts
}
