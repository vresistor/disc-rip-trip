//! The + popover: what can go after a strip, found by typing, added with a
//! click or Enter. Anchored to the + it came from; it closes if the canvas
//! moves under it, on Esc, or on a click anywhere else.

use crate::app::{App, Tray};
use crate::theme::{font, kind_icon, mix, Pal};
use drt_engine::{Group, StepInfo};
use egui::{pos2, vec2, Color32, Key, Rect, RichText, Sense, Stroke, Ui};

pub const WIDTH: f32 = 320.0;
pub const MAX_HEIGHT: f32 = 420.0;

/// An open popover.
#[derive(Clone, Debug, PartialEq)]
pub struct Popover {
    /// The step whose strip the + follows.
    pub after: String,
    pub search: String,
    /// Whether it adds a branch: a + where the line splits does; any other
    /// continues the line.
    pub branch: bool,
    /// The row the keyboard is on.
    pub sel: usize,
    /// The canvas's zoom and pan when it opened; if they change, it closes.
    pub(crate) view: (f32, egui::Vec2),
    pub(crate) focused: bool,
}

impl Popover {
    pub fn new(after: String, branch: bool, view: (f32, egui::Vec2)) -> Popover {
        Popover { after, search: String::new(), branch, sel: 0, view, focused: false }
    }
}

/// One option in the list.
struct Opt<'a> {
    info: &'a StepInfo,
    /// Where it goes: the way this + places things if it fits so, else
    /// the other way (a second encoder after Rip goes beside the first).
    branch: bool,
}

/// What it'll do: a step's own words, else its settings summed up.
fn describe(info: &StepInfo) -> String {
    if !info.from.is_empty() {
        return info.from.to_owned();
    }
    info.summary.map(|f| f(&|k| info.params.iter().find(|p| p.key == k).and_then(|p| p.default.clone()))).unwrap_or_default()
}

/// Draws the popover, if one is open, next to its + on `canvas`.
pub fn show(app: &mut App, ui: &mut Ui, p: &Pal, canvas: Rect, lay: &drt_layout::Layout) {
    let Some(mut pop) = app.popover.clone() else { return };
    let i = app.sel;
    let ctx = ui.ctx().clone();
    // Anchored to its +, in screen space; the canvas moving closes it.
    let v = app.views_of(i);
    let Some(&(_, (px, py))) = lay.pluses.iter().chain(&lay.seams).find(|(id, _)| *id == pop.after) else {
        app.popover = None;
        return;
    };
    if (v.zoom, v.pan) != pop.view {
        app.popover = None;
        return;
    }
    let plus = canvas.min + v.pan + vec2(px, py) * v.zoom;
    // On a seam: the tile it goes in front of.
    let between = lay.seams.iter().any(|(id, _)| *id == pop.after).then(|| {
        let next = lay.tiles.iter().position(|t| t.id == pop.after).and_then(|k| lay.tiles.get(k + 1)).map(|t| t.id.clone()).unwrap_or_default();
        app.m.snapshot(i).steps.get(&next).map_or(next, |v| v.name.clone())
    });
    let plus_r = 13.0 * v.zoom;
    // A click anywhere else closes it (the + itself toggles it).
    let outside = ctx.input(|inp| inp.pointer.any_pressed() && inp.pointer.interact_pos().is_some_and(|at| !app.popover_rect.contains(at) && at.distance(plus) > plus_r + 2.0));
    if outside {
        app.popover = None;
        return;
    }
    // Keys first, before the search box sees them.
    let (down, up, enter, esc) = ctx.input_mut(|inp| {
        (
            inp.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            inp.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            inp.consume_key(egui::Modifiers::NONE, Key::Enter),
            inp.consume_key(egui::Modifiers::NONE, Key::Escape),
        )
    });
    if esc {
        app.popover = None;
        return;
    }

    // The options: everything that checks here, placed as this + places
    // things (a branch where the line splits, in line anywhere else) if it
    // fits so, else the other way; filtered by the search.
    let placements = app.m.placements_after(i, &pop.after);
    let needle = pop.search.to_lowercase();
    let mut opts: Vec<Opt> = placements
        .iter()
        .filter(|(info, _, _)| needle.is_empty() || info.name.to_lowercase().contains(&needle) || describe(info).to_lowercase().contains(&needle))
        .map(|&(info, inline, br)| Opt { info, branch: if pop.branch { br } else { !inline } })
        .collect();
    opts.sort_by_key(|o| (o.info.group as u8, o.info.name));
    if down && !opts.is_empty() {
        pop.sel = (pop.sel + 1).min(opts.len() - 1);
    }
    if up {
        pop.sel = pop.sel.saturating_sub(1);
    }
    pop.sel = pop.sel.min(opts.len().saturating_sub(1));
    let mut chosen = if enter { opts.get(pop.sel).map(|o| (o.info.kind, o.branch)) } else { None };

    // Where it goes: beside the + (right, else left), level with it, kept
    // on the canvas.
    // Placed as if at its tallest, so however much it holds it stays on
    // the canvas; it's only as tall as what it holds.
    let h = MAX_HEIGHT.min(canvas.height() - 16.0);
    let mut x = plus.x + plus_r + 10.0;
    if x + WIDTH > canvas.right() - 8.0 {
        x = plus.x - plus_r - 10.0 - WIDTH;
    }
    let x = x.clamp(canvas.left() + 8.0, (canvas.right() - 8.0 - WIDTH).max(canvas.left() + 8.0));
    let y = (plus.y - 44.0).clamp(canvas.top() + 8.0, (canvas.bottom() - 8.0 - h).max(canvas.top() + 8.0));
    let name = app.m.snapshot(i).steps.get(&pop.after).map(|v| v.name.clone()).unwrap_or_default();
    let moved = down || up;

    let area = egui::Area::new(ui.scope_id().with("plus-popover")).order(egui::Order::Foreground).fixed_pos(pos2(x, y)).constrain(false).show(&ctx, |ui| {
        egui::Frame::new()
            .fill(p.raised)
            .stroke(Stroke::new(1.0, p.line))
            .corner_radius(10)
            .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if p.dark { 110 } else { 40 }) })
            .show(ui, |ui| {
                // Inside its 1 px border.
                ui.set_width(WIDTH - 2.0);
                ui.set_max_height(h - 2.0);
                // Header: what this adds after, and how.
                egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 10, top: 10, bottom: 8 }).show(ui, |ui| {
                    ui.set_width(WIDTH - 24.0);
                    let title = match &between {
                        Some(next) => format!("Add between {name} and {next}"),
                        None => format!("Add after {name}"),
                    };
                    ui.label(RichText::new(title).font(font(13.5, 600)));
                    ui.add_space(8.0);
                    let r = ui.add(egui::TextEdit::singleline(&mut pop.search).hint_text("Search steps").desired_width(f32::INFINITY).font(font(13.5, 400)).margin(vec2(8.0, 5.0)));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::SearchInput, true, "Search steps"));
                    if !pop.focused {
                        r.request_focus();
                        pop.focused = true;
                    }
                    if r.changed() {
                        pop.sel = 0;
                    }
                });
                ui.painter().hline(ui.min_rect().x_range(), ui.min_rect().bottom(), Stroke::new(1.0, p.line_soft));
                // The list, grouped.
                egui::ScrollArea::vertical().id_salt("plus-list").max_height((h - ui.min_rect().height() - 8.0).max(60.0)).show(ui, |ui| {
                    ui.set_width(WIDTH - 2.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add_space(4.0);
                    if opts.is_empty() {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(RichText::new(if needle.is_empty() { "Nothing fits here" } else { "No step by that name" }).font(font(13.0, 400)).color(p.subtle));
                        });
                        ui.add_space(10.0);
                    }
                    let mut last: Option<Group> = None;
                    for (k, o) in opts.iter().enumerate() {
                        if last != Some(o.info.group) {
                            ui.add_space(if last.is_some() { 6.0 } else { 2.0 });
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                ui.label(RichText::new(group_name(o.info.group)).font(font(11.5, 600)).color(p.subtle));
                            });
                            ui.add_space(2.0);
                            last = Some(o.info.group);
                        }
                        let (r, resp) = ui.allocate_exact_size(vec2(WIDTH - 2.0, 42.0), Sense::click());
                        let label = o.info.name.to_owned();
                        let on = k == pop.sel;
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::ListItem, true, on, &label));
                        let pt = ui.painter();
                        let row = r.shrink2(vec2(6.0, 1.0));
                        if on {
                            pt.rect_filled(row, 7.0, Color32::from_rgba_unmultiplied(p.action.r(), p.action.g(), p.action.b(), if p.dark { 40 } else { 22 }));
                        } else if resp.hovered() {
                            pt.rect_filled(row, 7.0, mix(p.ink, 0.04, p.raised));
                        }
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        pt.extend(kind_icon(o.info.kind, pos2(r.left() + 14.0, r.top() + 6.0), 16.0, p.kind(o.info.kind)));
                        pt.text(pos2(r.left() + 40.0, r.top() + 4.0), egui::Align2::LEFT_TOP, o.info.name, font(13.5, 500), p.ink);
                        let mut job = egui::text::LayoutJob::single_section(describe(o.info), egui::TextFormat::simple(font(12.0, 400), p.muted));
                        job.wrap = egui::text::TextWrapping::truncate_at_width(WIDTH - 56.0);
                        pt.galley(pos2(r.left() + 40.0, r.top() + 22.0), pt.layout_job(job), p.muted);
                        if on && moved {
                            ui.scroll_to_rect(r, None);
                        }
                        if resp.clicked() {
                            chosen = Some((o.info.kind, o.branch));
                        }
                    }
                    ui.add_space(6.0);
                });
            });
    });
    app.popover_rect = area.response.rect;
    if app.popover.is_some() {
        app.popover = Some(pop.clone());
    }
    // Adding: the sidebar opens only if the new step needs something set.
    if let Some((kind, as_branch)) = chosen {
        app.popover = None;
        match app.m.insert_after(i, &pop.after, kind, as_branch) {
            Ok(new) => {
                let focus = app.m.engine.missing(&app.m.recipes[i]).into_iter().find(|m| m.step == new).map(|m| m.key.to_owned());
                match focus {
                    Some(f) => app.open(Tray::Step { id: new, focus: Some(f) }),
                    None => app.select_tile(new),
                }
            }
            Err(e) => app.refuse(&ctx, e.to_string()),
        }
    }
}

pub fn group_name(g: Group) -> &'static str {
    match g {
        Group::Source => "Drive",
        Group::Kind => "Disc",
        Group::LookUp => "Look up",
        Group::Decide => "Decide",
        Group::Read => "Rip",
        Group::Make => "Make",
        Group::Place => "Save to",
        Group::Check => "Check",
        Group::Tell => "Notify",
    }
}
