//! Buttons with painted icons, and chips. egui 0.37 has no symbol font,
//! so icons are drawn (`theme::kind_icon`).

use crate::theme::{kind_icon, mix, Pal};
use egui::{pos2, vec2, Color32, FontId, Response, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, PartialEq)]
pub enum Look {
    /// No frame until hovered.
    Quiet,
    /// Raised, with a line.
    Plain,
    /// Filled with the action colour.
    Primary,
    /// A tinted link: the working folder.
    Link,
}

pub struct Btn<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    /// The accessible name, when the button has no label.
    pub name: Option<&'a str>,
    pub font: FontId,
    pub fg: Option<Color32>,
    pub look: Look,
    pub pad: Vec2,
    pub enabled: bool,
}

impl<'a> Btn<'a> {
    pub fn new(icon: &'a str, label: &'a str) -> Self {
        Btn { icon, label, name: None, font: crate::theme::font(14.0, 500), fg: None, look: Look::Plain, pad: vec2(14.0, 7.0), enabled: true }
    }
    pub fn look(mut self, l: Look) -> Self {
        self.look = l;
        if l == Look::Quiet {
            self.pad = vec2(6.0, 4.0);
        }
        if l == Look::Link {
            self.pad = vec2(8.0, 3.0);
            self.font = crate::theme::font(13.0, 400);
        }
        self
    }
    pub fn fg(mut self, c: Color32) -> Self {
        self.fg = Some(c);
        self
    }
    pub fn font(mut self, f: FontId) -> Self {
        self.font = f;
        self
    }
    pub fn pad(mut self, p: Vec2) -> Self {
        self.pad = p;
        self
    }
    pub fn name(mut self, n: &'a str) -> Self {
        self.name = Some(n);
        self
    }
    pub fn enabled(mut self, e: bool) -> Self {
        self.enabled = e;
        self
    }

    pub fn show(self, ui: &mut Ui, p: &Pal) -> Response {
        let fg = self.fg.unwrap_or(match self.look {
            Look::Primary => p.action_ink,
            Look::Link => p.action,
            Look::Quiet => p.muted,
            Look::Plain => p.ink,
        });
        let fg = if self.enabled { fg } else { fg.gamma_multiply(0.55) };
        let isz = (self.font.size + 1.0).round();
        let g = ui.painter().layout_no_wrap(self.label.to_owned(), self.font, fg);
        let gap = if self.icon.is_empty() || self.label.is_empty() { 0.0 } else { 5.0 };
        let iw = if self.icon.is_empty() { 0.0 } else { isz };
        let size = vec2(iw + gap + g.size().x, g.size().y.max(isz)) + self.pad * 2.0;
        let (r, resp) = ui.allocate_exact_size(size, if self.enabled { Sense::click() } else { Sense::hover() });
        let name = self.name.unwrap_or(if self.label.is_empty() { self.icon } else { self.label });
        let enabled = self.enabled;
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, enabled, name));
        // Hover fades in; a press darkens and the label settles a pixel.
        let hov = crate::motion::tween(ui.ctx(), resp.id.with("hover"), resp.hovered() && enabled, 0.09, crate::motion::cubic_out);
        let down = resp.is_pointer_button_down_on() && enabled;
        let pt = ui.painter();
        match self.look {
            Look::Primary => {
                let fill = if down { mix(p.ink, 0.22, p.action) } else { mix(p.ink, 0.12 * hov, p.action) };
                let fill = if enabled { fill } else { fill.gamma_multiply(0.5) };
                pt.rect_filled(r, 6.0, fill);
            }
            Look::Plain => {
                let fill = if down { mix(p.ink, 0.10, p.raised) } else { mix(p.ink, 0.04 * hov, p.raised) };
                pt.rect(r, 6.0, fill, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
            }
            Look::Link => {
                let a = if down { 52.0 } else { 20.0 + 16.0 * hov };
                pt.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(p.action.r(), p.action.g(), p.action.b(), a as u8));
            }
            Look::Quiet => {
                if down {
                    pt.rect_filled(r, 6.0, mix(p.ink, 0.10, p.raised));
                } else if hov > 0.0 {
                    pt.rect_filled(r, 6.0, mix(p.bg, hov, p.bg.gamma_multiply(0.0)));
                }
            }
        }
        let nudge = if down { 1.0 } else { 0.0 };
        let x = r.left() + self.pad.x;
        if !self.icon.is_empty() {
            pt.extend(kind_icon(self.icon, pos2(x, r.center().y - isz / 2.0 + nudge), isz, fg));
        }
        pt.galley(pos2(x + iw + gap, r.center().y - g.size().y / 2.0 + nudge), g, fg);
        focus_ring(ui, &resp, r, 6.0, p);
        if resp.hovered() && enabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp
    }
}

/// Whether focus came from the keyboard (Tab or an arrow since the last
/// pointer press): then the focused thing shows a ring.
pub fn kb_focus(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::NULL.with("kb-focus"))).unwrap_or(false)
}

pub fn set_kb_focus(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL.with("kb-focus"), on));
}

/// The keyboard focus ring: 2 px in the action colour, 2 px outside the
/// widget, following its corners.
pub fn focus_ring(ui: &Ui, resp: &Response, r: egui::Rect, radius: f32, p: &Pal) {
    if resp.has_focus() && kb_focus(ui.ctx()) {
        ui.painter().rect_stroke(r.expand(2.0), radius + 2.0, Stroke::new(2.0, p.action), egui::StrokeKind::Outside);
    }
}

/// The app's one chevron, pointing down (or turned by `turn` radians):
/// for dropdowns, menus and disclosures.
pub fn chevron(painter: &egui::Painter, c: egui::Pos2, size: f32, turn: f32, col: Color32) {
    let k = size / 24.0;
    let rot = egui::emath::Rot2::from_angle(turn);
    let pts: Vec<egui::Pos2> = [(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)].iter().map(|&(x, y)| c + rot * (vec2(x - 12.0, y - 12.0) * k)).collect();
    let w = 2.0 * k.max(0.75);
    painter.add(egui::Shape::line(pts.clone(), Stroke::new(w, col)));
    // Round caps and the joint.
    for q in pts {
        painter.circle_filled(q, w / 2.0, col);
    }
}

/// A ComboBox in the app's style: its chevron, not egui's solid triangle.
pub fn combo(id: impl std::hash::Hash + std::fmt::Debug, p: &Pal) -> egui::ComboBox<'static> {
    let col = p.muted;
    egui::ComboBox::from_id_salt(id).icon(move |ui, r, _v, open| {
        chevron(ui.painter(), r.center(), 14.0, if open { std::f32::consts::PI } else { 0.0 }, col);
    })
}

/// The height of every pill.
pub const PILL_H: f32 = 20.0;

/// A rounded tag without a state: a provenance word, a refusal. Its exact
/// size, so a tall row can't stretch it.
pub fn chip(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    let g = ui.painter().layout_no_wrap(text.to_owned(), crate::theme::font(12.5, 600), fg);
    let size = vec2(g.size().x + 20.0, PILL_H);
    let (r, resp) = ui.allocate_exact_size(size, Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Label, true, text));
    ui.painter().rect_filled(r, PILL_H / 2.0, bg);
    ui.painter().galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, fg);
    resp
}

/// The tone of a state: its pill, its text.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tone {
    Ok,
    Bad,
    Warn,
    Busy,
    Plain,
}

impl Tone {
    pub fn colours(self, p: &Pal) -> (Color32, Color32) {
        match self {
            Tone::Ok => (p.ok, p.ok_soft),
            Tone::Bad => (p.bad, p.bad_soft),
            Tone::Warn => (p.warn, p.warn_soft),
            Tone::Busy => (p.busy, p.busy_soft),
            Tone::Plain => (p.muted, p.bg),
        }
    }

    pub fn of(state: trip_engine::StepState) -> Tone {
        use trip_engine::StepState::*;
        match state {
            Ok => Tone::Ok,
            Bad => Tone::Bad,
            Needs | Hold | Wait => Tone::Warn,
            Busy => Tone::Busy,
            Idle => Tone::Plain,
        }
    }
}

/// A state pill: the tile's state icon, small, and a few words.
pub fn pill(ui: &mut Ui, p: &Pal, text: &str, tone: Tone, state: Option<trip_engine::StepState>, t: f64) -> Response {
    let (fg, bg) = tone.colours(p);
    let f = crate::theme::font(12.0, 500);
    let g = ui.painter().layout_no_wrap(text.to_owned(), f, fg);
    let icon = if state.is_some() { 12.0 + 4.0 } else { 0.0 };
    // One pill everywhere: exactly PILL_H tall, whatever row it's in; 8 at
    // the sides, the glyph 4 before the text.
    let size = vec2(8.0 + icon + g.size().x + 8.0, PILL_H);
    let (r, resp) = ui.allocate_exact_size(size, Sense::hover());
    let r = egui::Rect::from_center_size(r.center(), size);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Label, true, text));
    let pt = ui.painter();
    pt.rect_filled(r, PILL_H / 2.0, bg);
    if let Some(st) = state {
        // The tile's own state icon, in the pill's colour where it's a line.
        let c = pos2(r.left() + 8.0 + 6.0, r.center().y);
        let shapes = crate::theme::state_icon(p, st, c, 12.0 / 16.0, t);
        pt.extend(shapes);
    }
    pt.galley(pos2(r.left() + 8.0 + icon, r.center().y - g.size().y / 2.0), g, fg);
    resp
}

/// A segmented control: one of a few options, the chosen one filled.
/// `name` prefixes each option's accessible name ("AccurateRip: On"), so
/// two controls never share a name. Gives the option clicked.
pub fn segmented(ui: &mut Ui, p: &Pal, name: &str, options: &[&str], chosen: Option<usize>, enabled: bool) -> Option<usize> {
    segmented_sized(ui, p, name, options, chosen, enabled, (13.0, 26.0, 10.0))
}

/// [`segmented`], smaller: a quiet choice that isn't the point of what
/// it's beside (how a map is drawn).
pub fn segmented_small(ui: &mut Ui, p: &Pal, name: &str, options: &[&str], chosen: Option<usize>) -> Option<usize> {
    segmented_sized(ui, p, name, options, chosen, true, (12.0, 22.0, 8.0))
}

/// A segmented control in a size: its font, height and the room either
/// side of each option's words.
fn segmented_sized(ui: &mut Ui, p: &Pal, name: &str, options: &[&str], chosen: Option<usize>, enabled: bool, (size, h, pad): (f32, f32, f32)) -> Option<usize> {
    let f = crate::theme::font(size, 400);
    let mut clicked = None;
    let mut rects = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (k, o) in options.iter().enumerate() {
            let on = chosen == Some(k);
            // The chosen cell: ink in light; in dark a lifted cell, not a
            // near-white slab. (A galley keeps the colour it's laid out in.)
            let (on_fill, on_text) = if p.dark { (mix(p.ink, 0.2, p.raised), p.ink) } else { (p.ink, p.raised) };
            let g = ui.painter().layout_no_wrap((*o).to_owned(), f.clone(), if on { on_text } else { p.muted });
            let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + pad * 2.0, h), if enabled { Sense::click() } else { Sense::hover() });
            let label = format!("{name}: {o}");
            resp.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::RadioButton, enabled, on, &label));
            // Round only the control's outer corners.
            let last = k + 1 == options.len();
            let cr = egui::CornerRadius { nw: if k == 0 { 6 } else { 0 }, sw: if k == 0 { 6 } else { 0 }, ne: if last { 6 } else { 0 }, se: if last { 6 } else { 0 } };
            if on {
                ui.painter().rect_filled(r, cr, on_fill);
            } else if resp.is_pointer_button_down_on() && enabled {
                ui.painter().rect_filled(r, cr, mix(p.ink, 0.10, p.raised));
            } else if resp.hovered() && enabled {
                ui.painter().rect_filled(r, cr, p.bg);
            }
            if resp.hovered() && enabled {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            ui.painter().galley(pos2(r.left() + pad, r.center().y - g.size().y / 2.0), g, p.ink);
            focus_ring(ui, &resp, r, 6.0, p);
            if resp.clicked() {
                clicked = Some(k);
            }
            rects.push(r);
        }
    });
    if let (Some(a), Some(b)) = (rects.first(), rects.last()) {
        let all = a.union(*b);
        // Round the outer corners by painting the frame over the cells.
        ui.painter().rect_stroke(all, 6.0, Stroke::new(1.0, if p.dark { p.faint } else { p.line }), egui::StrokeKind::Inside);
        for r in &rects[1..] {
            ui.painter().vline(r.left(), r.y_range(), Stroke::new(1.0, p.line));
        }
    }
    clicked
}

/// A small on/off switch with its words after it (none where there's no
/// room; `name` is still its name), as quiet as the buttons beside it.
pub fn switch(ui: &mut Ui, p: &Pal, name: &str, words: &str, on: bool, f: FontId) -> Response {
    let fg = if on { p.ink } else { p.muted };
    let g = ui.painter().layout_no_wrap(words.to_owned(), f, fg);
    let track = vec2(24.0, 14.0);
    let pad = vec2(6.0, 4.0);
    let gap = if words.is_empty() { 0.0 } else { 6.0 };
    let size = vec2(track.x + gap + g.size().x, g.size().y.max(track.y)) + pad * 2.0;
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::Switch, true, on, name));
    let hov = crate::motion::tween(ui.ctx(), resp.id.with("hover"), resp.hovered(), 0.09, crate::motion::cubic_out);
    let k = crate::motion::tween(ui.ctx(), resp.id.with("on"), on, 0.14, crate::motion::cubic_out);
    let pt = ui.painter();
    if resp.is_pointer_button_down_on() {
        pt.rect_filled(r, 6.0, mix(p.ink, 0.10, p.raised));
    } else if hov > 0.0 {
        pt.rect_filled(r, 6.0, mix(p.bg, hov, p.bg.gamma_multiply(0.0)));
    }
    let tr = egui::Rect::from_min_size(pos2(r.left() + pad.x, r.center().y - track.y / 2.0), track);
    pt.rect_filled(tr, track.y / 2.0, mix(p.action, k, p.faint));
    let knob = track.y / 2.0 - 2.0;
    let cx = tr.left() + track.y / 2.0 + k * (track.x - track.y);
    pt.circle_filled(pos2(cx, tr.center().y), knob, if on { p.action_ink } else { p.raised });
    pt.galley(pos2(tr.right() + gap, r.center().y - g.size().y / 2.0), g, fg);
    focus_ring(ui, &resp, r, 6.0, p);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Whether Esc was what just took a field's focus: then what was typed
/// is put back, not kept.
pub fn escaped(ui: &egui::Ui) -> bool {
    ui.input(|i| i.key_pressed(egui::Key::Escape))
}

/// The size a link's text is set in.
const LINK_SIZE: f32 = 12.5;

/// A quiet link to a web page, in the action colour, underlined under the
/// pointer: *MusicBrainz ↗*. Placed by its right edge and middle, over
/// whatever it sits on (it takes the click). Gives its rect and whether
/// it was clicked.
pub fn link_at(ui: &mut Ui, p: &Pal, right: f32, cy: f32, label: &str, name: &str, tip: &str) -> (egui::Rect, bool) {
    let g = ui.painter().layout_no_wrap(label.to_owned(), crate::theme::font(LINK_SIZE, 500), p.action);
    let r = egui::Rect::from_min_size(pos2(right - g.size().x - 4.0, cy - g.size().y / 2.0 - 2.0), g.size() + vec2(4.0, 4.0));
    let resp = ui.interact(r, ui.scope_id().with(("link", name)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Link, true, name));
    let at = pos2(r.left() + 2.0, r.top() + 2.0);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().hline(at.x..=at.x + g.size().x, at.y + g.size().y - 1.0, Stroke::new(1.0, p.action));
    }
    ui.painter().galley(at, g, p.action);
    focus_ring(ui, &resp, r, 4.0, p);
    let resp = resp.on_hover_text(tip.to_owned());
    (r, resp.clicked())
}

/// The width a [`link_at`] takes.
pub fn link_width(ui: &Ui, label: &str) -> f32 {
    ui.painter().layout_no_wrap(label.to_owned(), crate::theme::font(LINK_SIZE, 500), Color32::WHITE).size().x + 4.0
}
