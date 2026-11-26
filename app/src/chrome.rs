//! The window's own frame: no title bar from the system. The top rows of
//! the list and the canvas move the window, a double-click there maximises
//! it, three quiet buttons sit at the top right, and every edge resizes.
//! Rounded corners with a hairline outline, square when maximised.

use crate::theme::Pal;
use egui::{pos2, vec2, Color32, CursorIcon, Id, Rect, ResizeDirection, Sense, Shape, Stroke, Ui, ViewportCommand};

/// The window's corner radius when it isn't maximised.
pub const ROUND: f32 = 10.0;
/// How far in from an edge resizing starts.
pub const EDGE: f32 = 6.0;
/// The smallest the window goes.
pub const MIN_SIZE: [f32; 2] = [960.0, 600.0];

/// Whether the window fills the screen (maximised or fullscreen): square
/// corners, no resizing edges.
pub fn filled(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false))
}

/// The corner radius for now.
pub fn round(ctx: &egui::Context) -> f32 {
    if filled(ctx) {
        0.0
    } else {
        ROUND
    }
}

/// The part of the window's top band under `rect`: a double-click there
/// maximises or restores. (Dragging anywhere on the band is `band`'s.)
/// Call before laying out what's on it, so its buttons and fields still
/// get their clicks; labels on it mustn't be selectable.
pub fn drag_area(ui: &Ui, rect: Rect, id: Id) {
    let r = ui.interact(rect, id, Sense::click_and_drag());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TitleBar, true, "Move window"));
    if r.double_clicked() {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
    }
}

/// Moves the window when a press anywhere on the top band (`rects`) moves
/// past egui's drag threshold before it's let go, as GNOME's title bars
/// do: over a label, a chip, a gap, even a button (a press that moves
/// isn't a click, so buttons still click). Not from `except` (a text
/// field, where dragging selects), nor from the resizing edges. Call once
/// a frame, after the band is drawn.
pub fn band(ctx: &egui::Context, rects: &[Rect], except: &[Rect]) {
    let id = Id::NULL.with("band-drag-sent");
    let (origin, dragging, down) = ctx.input(|i| (i.pointer.press_origin(), i.pointer.is_decidedly_dragging(), i.pointer.primary_down()));
    if !down {
        ctx.data_mut(|d| d.remove::<bool>(id));
        return;
    }
    let sent = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let Some(o) = origin else { return };
    let screen = ctx.content_rect();
    let on_edge = !filled(ctx) && (o.x - screen.left() < EDGE || screen.right() - o.x < EDGE || o.y - screen.top() < EDGE);
    if dragging && !sent && !on_edge && rects.iter().any(|r| r.contains(o)) && !except.iter().any(|r| r.contains(o)) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
}

/// Minimise, maximise (or restore) and close, right to left in `ui`.
pub fn buttons(ui: &mut Ui, p: &Pal) {
    let ctx = ui.ctx().clone();
    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ui.spacing_mut().item_spacing.x = 2.0;
    if button(ui, p, Glyph::Close, "Close window").clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    let (g, name) = if max { (Glyph::Restore, "Restore") } else { (Glyph::Maximise, "Maximise") };
    if button(ui, p, g, name).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
    }
    if button(ui, p, Glyph::Minimise, "Minimise").clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    Minimise,
    Maximise,
    Restore,
    Close,
}

fn button(ui: &mut Ui, p: &Pal, g: Glyph, name: &str) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(34.0, 30.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, name));
    let hot = resp.hovered() || resp.is_pointer_button_down_on();
    let close = g == Glyph::Close;
    let (fill, ink) = match (hot, close) {
        (true, true) => (p.bad, Color32::WHITE),
        (true, false) => (crate::theme::mix(p.ink, 0.07, p.surface), p.ink),
        (false, _) => (Color32::TRANSPARENT, p.muted),
    };
    let pt = ui.painter();
    pt.rect_filled(r, 6.0, fill);
    let c = r.center();
    let s = Stroke::new(1.3, ink);
    match g {
        Glyph::Minimise => {
            pt.hline(c.x - 5.0..=c.x + 5.0, c.y + 0.5, s);
        }
        Glyph::Maximise => {
            pt.rect_stroke(Rect::from_center_size(c, vec2(10.0, 10.0)), 1.5, s, egui::StrokeKind::Middle);
        }
        Glyph::Restore => {
            pt.rect_stroke(Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(8.0, 8.0)), 1.5, s, egui::StrokeKind::Middle);
            pt.add(Shape::line(vec![c + vec2(-2.5, -3.5), c + vec2(-2.5, -5.5), c + vec2(5.5, -5.5), c + vec2(5.5, 2.5), c + vec2(3.5, 2.5)], s));
        }
        Glyph::Close => {
            pt.line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], s);
            pt.line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], s);
        }
    }
    resp
}

/// The edges and corners that resize the window, over everything else,
/// and the outline round it. Call last.
pub fn frame(ctx: &egui::Context, p: &Pal) {
    let screen = ctx.content_rect();
    let r = round(ctx);
    let outline = egui::LayerId::new(egui::Order::Foreground, Id::NULL.with("window-outline"));
    ctx.layer_painter(outline).rect_stroke(screen, r, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
    if filled(ctx) {
        return;
    }
    let (e, c) = (EDGE, ROUND.max(EDGE * 2.0));
    let (l, t, rt, b) = (screen.left(), screen.top(), screen.right(), screen.bottom());
    use ResizeDirection as D;
    let zones = [
        (D::NorthWest, Rect::from_min_max(pos2(l, t), pos2(l + c, t + c)), CursorIcon::ResizeNorthWest, "nw"),
        (D::NorthEast, Rect::from_min_max(pos2(rt - c, t), pos2(rt, t + c)), CursorIcon::ResizeNorthEast, "ne"),
        (D::SouthWest, Rect::from_min_max(pos2(l, b - c), pos2(l + c, b)), CursorIcon::ResizeSouthWest, "sw"),
        (D::SouthEast, Rect::from_min_max(pos2(rt - c, b - c), pos2(rt, b)), CursorIcon::ResizeSouthEast, "se"),
        (D::North, Rect::from_min_max(pos2(l + c, t), pos2(rt - c, t + e)), CursorIcon::ResizeNorth, "n"),
        (D::South, Rect::from_min_max(pos2(l + c, b - e), pos2(rt - c, b)), CursorIcon::ResizeSouth, "s"),
        (D::West, Rect::from_min_max(pos2(l, t + c), pos2(l + e, b - c)), CursorIcon::ResizeWest, "w"),
        (D::East, Rect::from_min_max(pos2(rt - e, t + c), pos2(rt, b - c)), CursorIcon::ResizeEast, "e"),
    ];
    for (dir, rect, cursor, name) in zones {
        // A corner is a small square; the cursor and the press only count
        // within EDGE of the window's edge.
        egui::Area::new(Id::NULL.with(("resize", name))).order(egui::Order::Foreground).fixed_pos(rect.min).movable(false).show(ctx, |ui| {
            let (_, resp) = ui.allocate_exact_size(rect.size(), Sense::drag());
            let label = format!("Resize {name}");
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Splitter, true, &label));
            let on_edge = resp.hover_pos().is_some_and(|q| q.x - l < e || rt - q.x < e || q.y - t < e || b - q.y < e);
            if on_edge || resp.dragged() {
                ctx.set_cursor_icon(cursor);
            }
            if resp.drag_started_by(egui::PointerButton::Primary) && resp.interact_pointer_pos().is_some_and(|q| q.x - l < e || rt - q.x < e || q.y - t < e || b - q.y < e) {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            }
        });
    }
}

/// The window's title, for the taskbar and alt-tab: the app, then the
/// recipe you're on.
pub fn title(recipe: &str) -> String {
    if recipe.is_empty() {
        "Trip".into()
    } else {
        format!("Trip — {recipe}")
    }
}
