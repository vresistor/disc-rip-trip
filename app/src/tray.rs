//! The sidebar on the right, for the selected step. One anatomy for
//! every step: a header with the state, one sentence and the one thing to
//! do; then up to three tabs, only those with something in them:
//! Progress (what it did item by item, each with its proof), Settings,
//! and Log (its lines and its tool's output). Questions open here too.
//! Built from real widgets, so a screen reader sees all of it.

use crate::app::{App, Page, Tray};
use crate::makemkv::Readiness;
use crate::model::{text_value, value_text};
use crate::theme::{font, kind_icon, mix, state_icon, Pal};
use crate::widgets::{pill, segmented, Btn, Look, Tone};
use drt_engine::{Answer, Command, Context, Group, ItemState, ItemView, ParamKind, ParamSpec, QuestionShape, Scope, Setting, Snapshot, StepInfo, StepState, StepView, Subject};
use drt_steps::real::{expand, mmss};
use drt_steps::template::{self, Files};
use egui::{pos2, vec2, Align, Color32, Layout as L, Rect, RichText, Sense, Stroke, Ui};
use std::path::PathBuf;

/// A cover image, as far as it's got.
pub enum Cover {
    Loading,
    Ready(egui::TextureHandle, [usize; 2]),
    Failed,
}

/// Starts loading a cover picture, once: decoded off the window's thread.
pub fn load_cover(app: &mut App, ctx: &egui::Context, path: &std::path::Path) {
    app.covers.entry(path.to_owned()).or_insert_with(|| {
        let tx = app.cover_rx.0.clone();
        let ctx = ctx.clone();
        let p2 = path.to_owned();
        std::thread::spawn(move || {
            let img = image::open(&p2).ok().map(|i| {
                let i = i.into_rgba8();
                egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw())
            });
            let _ = tx.send((p2, img));
            ctx.request_repaint();
        });
        Cover::Loading
    });
}

/// Takes in the covers that have finished loading.
pub fn take_covers(app: &mut App, ctx: &egui::Context) {
    while let Ok((path, img)) = app.cover_rx.1.try_recv() {
        let c = match img {
            Some(img) => {
                let size = img.size;
                Cover::Ready(ctx.load_texture(path.display().to_string(), img, egui::TextureOptions::LINEAR), size)
            }
            None => Cover::Failed,
        };
        app.covers.insert(path, c);
    }
}

/// Draws the sidebar into its panel: the step's header (name, state, one
/// sentence, what to do), then one stack that scrolls. Its contents are
/// laid out at the width it opens to, so it slides rather than reflows.
pub fn show(app: &mut App, ui: &mut Ui, p: &Pal, snap: &Snapshot) {
    let panel = ui.max_rect();
    app.side_rect = panel;
    ui.painter().vline(panel.left(), panel.y_range(), Stroke::new(1.0, p.line));
    let Some(tray) = app.tray.clone().or_else(|| app.side_last.clone()) else { return };
    let t = app.clock.unwrap_or_else(|| ui.input(|i| i.time));
    let Some(plan) = plan(app, snap, &tray) else {
        if app.tray.is_some() {
            app.tray = None;
        }
        return;
    };
    let w = app.side_full.max(panel.width());
    let clip = panel.intersect(ui.clip_rect());
    let head = Rect::from_min_max(pos2(panel.left() + 20.0, panel.top() + 12.0), pos2(panel.left() + w - 12.0, panel.bottom()));
    let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(head).layout(L::top_down(Align::Min)));
    hu.set_clip_rect(clip);
    header(app, &mut hu, p, &plan, t);
    let mut body_top = hu.min_rect().bottom() + 12.0;
    // A step's three jobs, one tab each: Progress, Settings, Log; only
    // those with something in them, and no bar for just one.
    let tab = match (&tray, &plan.body) {
        (Tray::Step { id, focus }, Body::Step | Body::Choose) => {
            let tabs = side_tabs(app, id);
            let tab = app.rtab.filter(|t| tabs.contains(t)).unwrap_or_else(|| first_tab(app, id, focus.is_some(), &tabs));
            if tabs.len() > 1 {
                let row = Rect::from_min_max(pos2(panel.left() + 20.0, body_top - 4.0), pos2(panel.left() + w - 20.0, body_top + 32.0));
                let mut tu = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(L::left_to_right(Align::Center)));
                tu.set_clip_rect(clip);
                if let Some(t) = tab_row(&mut tu, p, &tabs, tab) {
                    app.rtab = Some(t);
                }
                body_top = row.bottom();
            }
            Some(tab)
        }
        _ => None,
    };
    ui.painter().hline(pos2(panel.left() + 1.0, 0.0).x..=panel.left() + w, body_top, Stroke::new(1.0, p.line_soft));
    if plan.body == Body::None {
        return;
    }
    let body = Rect::from_min_max(pos2(panel.left() + 20.0, body_top + 1.0), pos2(panel.left() + w - 6.0, panel.bottom()));
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(L::top_down(Align::Min)));
    // Its left margin is drawn in too (an asked-for field's outline).
    bu.set_clip_rect(Rect::from_min_max(pos2(panel.left() + 1.0, body.top()), body.max).intersect(clip));
    let key = match &tray {
        Tray::Step { id, .. } => id.clone(),
        Tray::Question { step } => format!("q/{step}"),
        Tray::App(page) => format!("app/{page:?}"),
    };
    egui::ScrollArea::vertical().id_salt(("side", key)).auto_shrink([false, false]).show(&mut bu, |ui| {
        // Room for the scroll bar, so it never sits on a field.
        ui.set_width(w - 20.0 - 20.0);
        ui.add_space(12.0);
        match (&tray, &plan.body) {
            (Tray::Question { step }, Body::Value) => value_question(app, ui, p, snap, step),
            (Tray::Question { .. }, Body::PerItem) => per_item(app, ui, p, snap),
            (Tray::Question { step }, Body::Choose) => choose(app, ui, p, snap, step, t),
            (Tray::Question { step }, Body::Answers) => answers(app, ui, p, snap, step),
            (Tray::Question { step }, Body::Pictures) => pictures(app, ui, p, snap, step),
            (Tray::App(Page::MakeMkv), _) => makemkv_page(app, ui, p, true),
            (Tray::App(Page::WorkingFiles), _) => working_files(app, ui, p),
            (Tray::Step { id, focus }, body) => {
                // A step that runs MakeMKV, while it isn't set up: that first.
                if snap.steps.get(id).is_some_and(|v| crate::makemkv::runs_makemkv(&v.step)) && app.m.makemkv_blocks(app.sel).is_some() && !app.m.ripping(app.sel) {
                    makemkv_page(app, ui, p, false);
                    ui.add_space(8.0);
                    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, p.line_soft));
                    ui.add_space(8.0);
                }
                match tab {
                    Some(Tab::Progress) if *body == Body::Choose => choose(app, ui, p, snap, id, t),
                    Some(Tab::Progress) => result(app, ui, p, snap, id, t),
                    Some(Tab::Settings) => {
                        if let Some(info) = app.m.info(app.sel, id) {
                            settings(app, ui, p, id, &info, focus.as_deref());
                            if info.kind == "tell" {
                                send_test(app, ui, p, id, &info);
                            }
                        }
                    }
                    Some(Tab::Log) => {
                        if let Some(v) = snap.steps.get(id).cloned() {
                            log_view(app, ui, p, id, &v);
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
        ui.add_space(20.0);
    });
}

/// Whether the sidebar is asking a question that compares two lists:
/// Which release?, its candidates beside the disc's tracks.
pub fn wants_wide(app: &App) -> bool {
    let Some(Tray::Question { step }) = &app.tray else { return false };
    let snap = app.m.snapshot(app.sel);
    let offered = snap.steps.get(step).is_some_and(|v| v.record.options.is_some());
    offered && matches!(&snap.question, Some((s, q)) if s == step && matches!(q.shape, QuestionShape::PickOne { .. }))
}

/// Whether a step's question is one of its own few answers (a Folder's
/// *Replace them*, *Keep both*, *Skip*) rather than candidates it was
/// offered: it never put candidates to you.
fn own_answers(v: &StepView, options: &[drt_engine::Choice]) -> bool {
    !options.is_empty() && v.record.options.is_none()
}

/// Whether a step has a run to show: it has started, or has a record.
pub fn has_record(app: &App, i: usize, id: &str) -> bool {
    if app.m.shown_run(i).is_none() {
        return false;
    }
    let snap = app.m.snapshot(i);
    let (Some(v), Some(info)) = (snap.steps.get(id), app.m.info(i, id)) else { return false };
    !matches!(v.state, StepState::Idle | StepState::Wait) || !v.items.is_empty() || !record_tabs(app, i, id, v, &info).is_empty()
}

/// Whether a step has settings to show.
pub fn has_settings(app: &App, i: usize, id: &str, info: &StepInfo) -> bool {
    info.kind == "drive" || !rail_specs_beside(app, i, id, info, false).is_empty()
}

/// The sidebar's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// The working state and the result.
    Progress,
    Settings,
    /// The log and the tool's own output.
    Log,
}

impl Tab {
    fn word(self) -> &'static str {
        match self {
            Tab::Progress => "Progress",
            Tab::Settings => "Settings",
            Tab::Log => "Log",
        }
    }
}

/// The tabs a step has something for, in order.
pub fn side_tabs(app: &App, id: &str) -> Vec<Tab> {
    let i = app.sel;
    let mut tabs = vec![];
    let snap = app.m.snapshot(i);
    let (Some(v), Some(info)) = (snap.steps.get(id), app.m.info(i, id)) else { return tabs };
    if has_record(app, i, id) || (v.step == "choose" && snap.question.as_ref().is_some_and(|(s, _)| s == id)) {
        tabs.push(Tab::Progress);
    }
    if has_settings(app, i, id, &info) {
        tabs.push(Tab::Settings);
    }
    if !v.record.log.is_empty() || !app.m.shown_raw(i, id).is_empty() {
        tabs.push(Tab::Log);
    }
    tabs
}

/// Which tab a step opens on: Settings when a value is asked for, or
/// before it has run; Log when it failed; else Progress.
fn first_tab(app: &App, id: &str, asked: bool, tabs: &[Tab]) -> Tab {
    let has = |t: Tab| tabs.contains(&t);
    let snap = app.m.snapshot(app.sel);
    let failed = snap.steps.get(id).is_some_and(|v| failed(app, id, v));
    if asked && has(Tab::Settings) {
        Tab::Settings
    } else if failed && has(Tab::Log) {
        Tab::Log
    } else if has(Tab::Progress) {
        Tab::Progress
    } else {
        tabs.first().copied().unwrap_or(Tab::Settings)
    }
}

/// The tab row: each tab's word, the chosen one underlined. Gives the
/// one clicked.
fn tab_row(ui: &mut Ui, p: &Pal, tabs: &[Tab], on: Tab) -> Option<Tab> {
    let mut clicked = None;
    ui.spacing_mut().item_spacing.x = 20.0;
    for &t in tabs {
        let sel = t == on;
        let g = ui.painter().layout_no_wrap(t.word().into(), font(13.0, if sel { 600 } else { 500 }), if sel { p.ink } else { p.muted });
        let (r, resp) = ui.allocate_exact_size(vec2(g.size().x, 36.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::Tab, true, sel, t.word()));
        let col = if sel || resp.hovered() { p.ink } else { p.muted };
        ui.painter().galley(pos2(r.left(), r.center().y - g.size().y / 2.0 - 1.0), g, col);
        if sel {
            ui.painter().line_segment([pos2(r.left(), r.bottom() - 1.0), pos2(r.right(), r.bottom() - 1.0)], Stroke::new(2.0, p.action));
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        crate::widgets::focus_ring(ui, &resp, r, 4.0, p);
        if resp.clicked() {
            clicked = Some(t);
        }
    }
    clicked
}

/// A small heading between the parts of the stack.
fn heading(ui: &mut Ui, p: &Pal, text: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(text).font(font(12.5, 600)).color(p.muted));
    ui.add_space(4.0);
}

/// What a step did, as a step of its kind shows it.
fn result(app: &mut App, ui: &mut Ui, p: &Pal, snap: &Snapshot, id: &str, t: f64) {
    step_body(app, ui, p, snap, id, t);
    let i = app.sel;
    let Some(v) = snap.steps.get(id).cloned() else { return };
    let Some(info) = app.m.info(i, id) else { return };
    let ctx = app.m.shown_context(i);
    // A choice: the one chosen, and why; the others below it.
    if let Some(d) = &v.record.decided {
        let options = options_of(&v, ctx.as_ref());
        heading(ui, p, "Chose");
        if let Some(url) = decision_card(ui, p, d, options.iter().find(|x| Some(&x.id) == d.chosen.as_ref())) {
            app.m.open_url(&url);
        }
        let others: Vec<&drt_engine::Candidate> = options.iter().filter(|x| Some(&x.id) != d.chosen.as_ref()).collect();
        if !others.is_empty() {
            heading(ui, p, "The other options");
            for x in others {
                if let Some(url) = candidate_row(ui, p, x, false) {
                    app.m.open_url(&url);
                }
            }
        }
    }
    // A lookup (or the disc's own text): what it offered, best first.
    if let Some(c) = &ctx {
        let mut offered: Vec<&drt_engine::Candidate> = c.offered_by(id).collect();
        offered.sort_by(|a, b| b.score.total_cmp(&a.score));
        if !offered.is_empty() && v.record.decided.is_none() {
            heading(ui, p, if offered.len() == 1 { "Offered" } else { "Offered, best first" });
            for x in offered {
                if let Some(url) = candidate_row(ui, p, x, false) {
                    app.m.open_url(&url);
                }
            }
        }
    }
    // The files it wrote, unless its rows already name them.
    // (An encoder's rows and a folder's each name their file; any other
    // step's rows stand for its files when there's one for each, or name
    // them.)
    let in_rows = |f: &drt_engine::Made| {
        let name = file_name(&f.path);
        v.items.iter().any(|it| it.label.contains(&name) || it.note.contains(&name))
    };
    let named = (is_encoder(&v.step) && v.items.iter().enumerate().any(|(k, it)| file_for(&v.record.files, number(it, k), &it.label).is_some()))
        || (is_place(&v.step) && !v.items.is_empty())
        || (!v.items.is_empty() && (v.record.files.iter().all(in_rows) || v.record.files.len() <= v.items.len()));
    if !v.record.files.is_empty() && !named {
        heading(ui, p, "Files");
        files(app, ui, p, &v.record.files);
    }
    // With nothing else to show, the facts it found.
    if let Some(c) = &ctx {
        let facts: Vec<&drt_engine::Fact> = c.found_by(id).filter(|f| !matches!(f.key.as_str(), "cover" | "cover_release" | "cover_from")).collect();
        if !facts.is_empty() && !has_items(id, &v, &info, Some(c)) && v.record.decided.is_none() && v.record.files.is_empty() {
            heading(ui, p, "Found");
            fact_table(ui, p, &facts);
        }
    }
}

/// Whether the step failed, as its card says: a track to decide isn't.
fn failed(app: &App, id: &str, v: &StepView) -> bool {
    let retries = app.m.info(app.sel, id).is_some_and(|i| i.retries.iter().any(|r| !matches!(r.id, "keep" | "leave_out")));
    crate::card::Shown::of(v, retries) == crate::card::Shown::Bad
}

/// What the sidebar is about, worked out once per frame.
struct Plan {
    icon: String,
    colour: Color32,
    name: String,
    pill: Option<(String, Tone, Option<StepState>)>,
    say: String,
    acts: Vec<Act>,
    body: Body,
    /// The tile whose name can be edited here (in edit mode).
    rename: Option<String>,
    /// Remove, in the header's row beside the close button (in edit mode).
    remove: bool,
    /// A quiet action on the header's own row, beside the close button: a
    /// game or data disc's Rip opens the tray from there.
    corner: Option<Act>,
}

/// A header action. The last `Primary` one is drawn filled.
#[derive(Clone)]
enum Act {
    OpenFolder {
        label: String,
        path: PathBuf,
    },
    LetThrough(usize),
    UseThis(usize),
    Measure {
        busy: bool,
    },
    /// Stop any read on the recipe's drive and open its tray.
    OpenTray {
        busy: bool,
        /// The offset is being measured: the tray waits till that's done.
        measuring: bool,
    },
    AskRemove,
    Remove,
    Keep,
    Answer(String),
    /// Go on without an answer.
    Skip,
    /// Send each item's choice.
    UseThese,
    /// The first retry, for every item that needs you.
    RetryAll {
        items: Vec<String>,
        option: String,
    },
}

/// What goes under the header.
#[derive(Clone, PartialEq)]
enum Body {
    /// The step: its settings before a run; its result, then its
    /// settings and details, once it has one.
    Step,
    Choose,
    /// The question's own answers, a button for each.
    Answers,
    /// Answers that are pictures (covers), side by side.
    Pictures,
    Value,
    /// A choice for each item.
    PerItem,
    /// One of the app's own settings.
    App,
    /// Header only.
    None,
}

/// What's left for a scrolling list: as much as there is (the sidebar
/// scrolls as a whole).
fn room(ui: &Ui) -> f32 {
    (ui.available_height() - 16.0).max(40.0)
}

// ---- the plan for each sidebar ------------------------------------------------------------

fn plan(app: &App, snap: &Snapshot, tray: &Tray) -> Option<Plan> {
    let i = app.sel;
    match tray {
        Tray::App(Page::WorkingFiles) => {
            let p = Pal::get(app.dark);
            let w = &app.m.work;
            Some(Plan {
                icon: "folder".into(),
                colour: p.kind("folder"),
                name: "Working files".into(),
                pill: Some((drt_steps::real::media::size(w.total), if w.kept.is_empty() { Tone::Plain } else { Tone::Warn }, None)),
                say: "Where each disc's rip and encodes wait until they're filed. They're deleted once everything is filed; a folder holding files that never were is kept, since it may be the only copy.".into(),
                acts: vec![],
                body: Body::App,
                rename: None,
                remove: false,
                corner: None,
            })
        }
        Tray::App(Page::MakeMkv) => {
            let p = Pal::get(app.dark);
            let r = app.m.makemkv.readiness();
            Some(Plan {
                icon: "rip-video".into(),
                colour: p.kind("rip-video"),
                name: "MakeMKV".into(),
                pill: Some(match r {
                    Readiness::Ready => ("Ready".into(), Tone::Ok, Some(StepState::Ok)),
                    Readiness::Checking => ("Checking".into(), Tone::Busy, Some(StepState::Busy)),
                    Readiness::NotChosen => ("Not set up".into(), Tone::Warn, Some(StepState::Needs)),
                    Readiness::NeedsPermission => ("Can't be started".into(), Tone::Warn, Some(StepState::Needs)),
                }),
                say: "Reads video discs, for Video disc and Rip.".into(),
                acts: vec![],
                body: Body::App,
                rename: None,
                remove: false,
                corner: None,
            })
        }
        Tray::Question { step } => {
            let (qstep, q) = snap.question.clone()?;
            if qstep != *step {
                return None;
            }
            let v = snap.steps.get(step)?;
            let p = Pal::get(app.dark);
            match &q.shape {
                QuestionShape::PickOne { options, skip, .. } if own_answers(v, options) => Some(Plan {
                    icon: v.step.clone(),
                    colour: p.kind(&v.step),
                    name: v.name.clone(),
                    pill: Some(("Needs you".into(), Tone::Warn, Some(StepState::Needs))),
                    say: q.prompt.clone(),
                    // A button for each answer, in the body with what it does.
                    acts: if *skip { vec![Act::Skip] } else { vec![] },
                    body: if options.iter().any(|o| o.picture.is_some()) { Body::Pictures } else { Body::Answers },
                    rename: None,
                    remove: false,
                    corner: None,
                }),
                QuestionShape::PickOne { options, preselected, details, skip } => {
                    let n = options.len();
                    // With nothing found, the form is the only choice.
                    let pick = if n == 0 { 0 } else { app.pick.unwrap_or(*preselected) };
                    let mut acts = vec![];
                    if *skip {
                        acts.push(Act::Skip);
                    }
                    if n > 0 || details.is_some() {
                        acts.push(Act::UseThis(pick));
                    }
                    // What tells them apart, from the options' own facts.
                    let ctx = app.m.shown_run(i).map(|r| r.run.context());
                    let differ = differences(&options_of(v, ctx.as_ref()));
                    Some(Plan {
                        icon: v.step.clone(),
                        colour: p.kind(&v.step),
                        name: v.name.clone(),
                        pill: Some(("Needs you".into(), Tone::Warn, Some(StepState::Needs))),
                        // The question, then what tells the options apart.
                        say: match (n, v.step.as_str()) {
                            (0, _) => format!("{} Nothing found.", q.prompt),
                            (_, _) if n > 1 && !differ.is_empty() => format!("{} They differ by {differ}.", q.prompt),
                            _ => q.prompt.clone(),
                        },
                        acts,
                        body: Body::Choose,
                        rename: None,
                        remove: false,
                        corner: None,
                    })
                }
                QuestionShape::PerItem { .. } => Some(Plan {
                    icon: v.step.clone(),
                    colour: p.kind(&v.step),
                    name: v.name.clone(),
                    pill: Some(("Needs you".into(), Tone::Warn, Some(StepState::Needs))),
                    say: q.prompt.clone(),
                    acts: vec![Act::UseThese],
                    body: Body::PerItem,
                    // The rows take the whole width.
                    rename: None,
                    remove: false,
                    corner: None,
                }),
                QuestionShape::Value { label, .. } => Some(Plan {
                    icon: v.step.clone(),
                    colour: p.kind(&v.step),
                    name: v.name.clone(),
                    pill: Some(("Needs you".into(), Tone::Warn, Some(StepState::Needs))),
                    say: format!("{label}?"),
                    acts: vec![Act::Answer(step.clone())],
                    body: Body::Value,
                    rename: None,
                    remove: false,
                    corner: None,
                }),
            }
        }
        Tray::Step { id, .. } => {
            let v = snap.steps.get(id)?.clone();
            let p = Pal::get(app.dark);
            let info = app.m.info(i, id);
            let mut plan = Plan {
                icon: v.step.clone(),
                colour: p.kind(&v.step),
                name: v.name.clone(),
                pill: Some(state_pill(&v, &app.m.info(i, id))),
                say: String::new(),
                acts: vec![],
                body: Body::Step,
                rename: None,
                remove: false,
                corner: None,
            };
            let editing = app.editing && !app.m.ripping(i);
            let Some(info) = info else {
                // A step this build doesn't have.
                plan.pill = Some(("Not available".into(), Tone::Plain, None));
                plan.say = "Not in this version of the app.".into();
                plan.body = Body::None;
                if !app.m.ripping(i) && app.m.recipes[i].root.ids().len() > 1 {
                    plan.acts.push(Act::AskRemove);
                }
                return Some(confirming(app, plan, id, &v));
            };
            let ctx = app.m.shown_run(i).map(|r| r.run.context());
            match v.step.as_str() {
                "drive" => {
                    let drive = app.m.drive(i);
                    plan.say = if drive.is_empty() {
                        "Choose a drive.".into()
                    } else if !app.m.is_connected(&drive) {
                        // Not there: say so, and struck through.
                        "Not connected.".into()
                    } else {
                        String::new()
                    };
                    if !app.m.is_connected(&drive) {
                        plan.icon = "drive-off".into();
                        plan.colour = Pal::get(app.dark).subtle;
                    }
                    if app.m.can_open_tray(i) {
                        plan.acts.push(Act::OpenTray { busy: app.m.opening_tray(i), measuring: app.m.measuring_on_drive(i) });
                    }
                    let missing = app.m.findable_missing(i).iter().any(|m| m.step == *id);
                    // Not while the drive is reading: its settings are locked.
                    if (missing || app.m.finding.contains_key(&i)) && !app.m.locked(i, Scope::Drive) {
                        plan.acts.push(Act::Measure { busy: app.m.finding.get(&i) == Some(&true) });
                    }
                }
                "audio-cd" => {
                    // The card says what was read.
                    plan.say = String::new();
                }
                "choose" => {
                    // Asking, or not yet decided: the candidates; decided,
                    // its record (the choice and why, on Found).
                    let asking = snap.question.as_ref().is_some_and(|(s, _)| s == id);
                    if asking || v.record.decided.is_none() {
                        plan.body = Body::Choose;
                    }
                    // Its own choice, from its record.
                    // Why it chose: the card says what.
                    plan.say = match v.record.decided.as_ref() {
                        Some(d) => format!("{}.", cap(&d.why)),
                        None => String::new(),
                    };
                }
                "rip" | "game-rip" | "image" | "rip-video" => {
                    // What's needed, and the retries as actions: the card
                    // already says how far it's got. A disc's image says
                    // what's wrong with it beside its ring.
                    let bad: Vec<&ItemView> = v.items.iter().filter(|x| x.state == ItemState::Bad).collect();
                    let what = if v.step == "rip-video" { "videos" } else { "tracks" };
                    plan.say = match bad.as_slice() {
                        [] => String::new(),
                        [_] if matches!(v.step.as_str(), "game-rip" | "image") => String::new(),
                        [_] => format!("{}.", v.detail),
                        b => format!("{} {what} need you.", b.len()),
                    };
                    // While it has the drive: stop and take the disc out. A
                    // disc's image has it quietly on the header's row.
                    if app.m.ripping(i) && app.m.shows_newest(i) && app.m.can_open_tray(i) {
                        let act = Act::OpenTray { busy: app.m.opening_tray(i), measuring: app.m.measuring_on_drive(i) };
                        if matches!(v.step.as_str(), "game-rip" | "image") {
                            plan.corner = Some(act);
                        } else {
                            plan.acts.push(act);
                        }
                    }
                    // One track's choices are on its row, not here too;
                    // several get one button for them all.
                    if bad.len() > 1 {
                        if let Some(o) = info.retries.iter().find(|o| !matches!(o.id, "keep" | "leave_out")) {
                            plan.acts.push(Act::RetryAll { items: bad.iter().map(|x| x.id.clone()).collect(), option: o.id.to_owned() });
                        }
                    }
                    // (The working folder is the window header's link.)
                }
                "next-disc" => {
                    plan.pill = Some(if v.state == StepState::Ok { ("Ejected".into(), Tone::Ok, Some(StepState::Ok)) } else { ("Waiting".into(), Tone::Plain, Some(StepState::Idle)) });
                    plan.say = String::new();
                    plan.body = Body::None;
                }
                "hold" => {
                    let n = v.count(ItemState::Waiting);
                    let holding = v.state == StepState::Hold && app.m.shown_run(i).is_some();
                    let before = v.preds.first().and_then(|pr| snap.steps.get(pr)).filter(|pv| is_place(&pv.step));
                    let all_in = v.detail == crate::card::WAITS_FOR_YOU;
                    plan.say = match (holding, before) {
                        (true, Some(pv)) if all_in => format!("Listen in {}, then let them through.", pv.name),
                        (true, None) if all_in => "Let them through when you're ready.".into(),
                        _ => String::new(),
                    };
                    // The folder before it, to listen in; then let them on.
                    if let Some(pv) = before {
                        if let Some(f) = folder_of(app, i, &v.preds[0]) {
                            plan.acts.push(Act::OpenFolder { label: format!("Open {}", pv.name), path: filed_dir(&pv.record.files, &f) });
                        }
                    }
                    if holding {
                        plan.acts.push(Act::LetThrough(n));
                    }
                }
                k if is_place(k) => {
                    let folder = folder_of(app, i, id);
                    plan.say = if folder.is_none() { "Choose a folder.".into() } else { String::new() };
                    if let Some(f) = &folder {
                        plan.acts.push(Act::OpenFolder { label: "Open folder".into(), path: filed_dir(&v.record.files, f) });
                    }
                }
                k if is_encoder(k) => {
                    // Not the tile again: its settings are right below, and
                    // the counts are in the list. Working, the pill just
                    // says so, with how far overall beside it.
                    plan.say = match (v.items.is_empty(), percent(&v.detail)) {
                        (true, _) => String::new(),
                        (false, Some(pc)) if v.state == StepState::Busy => format!("{pc} overall"),
                        (false, _) if v.state == StepState::Busy => String::new(),
                        _ => String::new(),
                    };
                    if v.state == StepState::Busy {
                        plan.pill = Some(("Encoding".into(), Tone::Busy, Some(StepState::Busy)));
                    }
                }
                "cover-art" => {
                    plan.say = String::new();
                }
                // What each does, and what came of it.
                "check" => plan.say = crate::compare::check_say(&v),
                "redump" => plan.say = crate::compare::redump_say(&v),
                "cd-text" => {
                    plan.say = match ctx.as_ref().map(|c| cdtext_rows(c, id).len()) {
                        Some(0) if v.state == StepState::Ok => "No CD-Text on this disc.".into(),
                        _ => String::new(),
                    }
                }
                _ if info.group == Group::LookUp => {
                    plan.say = match ctx.as_ref().map(|c| c.offered_by(id).count()) {
                        Some(n) if (v.state == StepState::Ok || v.state == StepState::Bad) && n == 0 && v.items.is_empty() => "Nothing found.".into(),
                        _ => String::new(),
                    }
                }
                _ => {}
            }
            plan.remove = editing && app.m.recipes[i].root.ids().len() > 1 && v.step != "drive";
            if editing {
                plan.rename = Some(id.clone());
            }
            Some(confirming(app, plan, id, &v))
        }
    }
}

// ---- Working files ----------------------------------------------------------------------------

/// The working folders kept because they hold files never filed: each
/// with Open folder and Delete, which asks first.
fn working_files(app: &mut App, ui: &mut Ui, p: &Pal) {
    let kept = app.m.work.kept.clone();
    if kept.is_empty() {
        empty(ui, p, "Nothing waiting to be filed.");
        return;
    }
    heading(ui, p, "Never filed");
    for k in kept {
        let size = drt_steps::real::media::size(k.bytes);
        ui.add(egui::Label::new(RichText::new(&k.line).font(font(13.5, 500)).color(p.ink)).wrap());
        ui.label(RichText::new(format!("{size} · {}", drt_steps::real::reach::shown(&k.path))).font(font(12.0, 400)).color(p.muted));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if app.confirm_work_delete.as_ref() == Some(&k.path) {
                ui.label(RichText::new(format!("Delete {size} for good?")).font(font(13.0, 500)).color(p.bad));
                if danger(ui, p, "Delete") {
                    app.m.delete_work(&k.path);
                    app.confirm_work_delete = None;
                }
                if Btn::new("", "Keep").look(Look::Quiet).font(font(13.0, 500)).show(ui, p).clicked() {
                    app.confirm_work_delete = None;
                }
            } else {
                let open = format!("Open folder: {}", k.line);
                if Btn::new("folder-link", "Open folder").look(Look::Plain).font(font(13.0, 500)).name(&open).show(ui, p).clicked() {
                    app.m.open(&k.path);
                }
                let del = format!("Delete: {}", k.line);
                if Btn::new("trash", "Delete").look(Look::Quiet).fg(p.bad).font(font(13.0, 500)).name(&del).show(ui, p).clicked() {
                    app.confirm_work_delete = Some(k.path.clone());
                }
            }
        });
        ui.add_space(14.0);
    }
}

// ---- MakeMKV --------------------------------------------------------------------------------

/// Where MakeMKV is, and your OK to run it: what's found, *Choose…* and
/// *Get MakeMKV* until you've said; then what you chose, and the flatpak's
/// permission if it needs one. `full` is the app's own page, with the key
/// setting; else it's the part a step that needs it shows.
fn makemkv_page(app: &mut App, ui: &mut Ui, p: &Pal, full: bool) {
    let wrap = |ui: &mut Ui, text: &str, col: Color32| {
        ui.add(egui::Label::new(RichText::new(text).font(font(13.0, 400)).color(col)).wrap());
    };
    let mk = &app.m.makemkv;
    if mk.hooks.from_env {
        wrap(ui, "MakeMKV is run as DRT_MAKEMKVCON says, which you set yourself.", p.ink);
        return;
    }
    let r = mk.readiness();
    let choosing = r == Readiness::NotChosen || mk.changing;
    if !full || r == Readiness::NotChosen {
        wrap(ui, crate::makemkv::WHY, p.ink);
        ui.add_space(8.0);
    }
    if let (Some(s), false) = (&mk.saved, mk.changing) {
        let (label, version) = (s.location.label(), s.version.clone());
        field(
            ui,
            p,
            "Runs MakeMKV from",
            |ui, l| {
                ui.add(egui::Label::new(RichText::new(&label).font(font(13.5, 500)).color(p.ink)).wrap()).labelled_by(l);
                let v = match (r, version) {
                    (Readiness::NeedsPermission, _) => "Not run yet: it can't be started from here".to_owned(),
                    (_, Some(v)) => v,
                    (_, None) => "Version not known yet".to_owned(),
                };
                ui.label(RichText::new(v).font(font(12.5, 400)).color(p.muted));
            },
            None,
        );
        if r == Readiness::NeedsPermission {
            // The app comes allowed to start MakeMKV; this is only seen
            // when someone has taken that away (Flatseal, or an override).
            wrap(ui, crate::makemkv::TAKEN_AWAY, p.ink);
            ui.add_space(6.0);
            if Btn::new("", "Check again").look(Look::Quiet).font(font(13.0, 500)).show(ui, p).clicked() {
                app.m.look_for_makemkv();
                app.m.check_saved_makemkv();
            }
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            if Btn::new("", "Change…").look(Look::Plain).font(font(13.0, 500)).show(ui, p).clicked() {
                app.m.look_for_makemkv();
                app.m.makemkv.changing = true;
            }
            if Btn::new("", "Don't use MakeMKV").look(Look::Quiet).font(font(13.0, 500)).show(ui, p).clicked() {
                app.m.revoke_makemkv();
            }
        });
    }
    if let Some(loc) = &app.m.makemkv.checking {
        ui.add_space(6.0);
        wrap(ui, &format!("Checking {}…", loc.label()), p.muted);
    } else if choosing {
        let found = app.m.makemkv.candidates.clone();
        heading(ui, p, if found.is_empty() { "Not found in the usual places" } else { "Found on this computer" });
        let mut use_this = None;
        for (k, loc) in found.iter().enumerate() {
            ui.horizontal(|ui| {
                let label = loc.label();
                let bw = 86.0;
                ui.scope(|ui| {
                    ui.set_width((ui.available_width() - bw - 8.0).max(80.0));
                    ui.add(egui::Label::new(RichText::new(&label).font(font(13.0, 500)).color(p.ink)).wrap());
                });
                ui.with_layout(L::right_to_left(Align::Center), |ui| {
                    let name = format!("Use this: {label}");
                    if Btn::new("", "Use this").look(if k == 0 { Look::Primary } else { Look::Plain }).font(font(13.0, 500)).name(&name).show(ui, p).clicked() {
                        use_this = Some(loc.clone());
                    }
                });
            });
            ui.add_space(4.0);
        }
        if let Some(loc) = use_this {
            app.m.approve_makemkv(loc);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if Btn::new("folder-link", "Choose…").look(Look::Plain).font(font(13.0, 500)).name("Choose makemkvcon").show(ui, p).clicked() {
                let (hook, tx, ctx) = (app.m.makemkv.hooks.pick_file.clone(), app.picked_file.0.clone(), ui.ctx().clone());
                std::thread::spawn(move || {
                    if let Some(path) = hook() {
                        let _ = tx.send(path);
                        ctx.request_repaint();
                    }
                });
            }
            if Btn::new("", "Get MakeMKV").look(Look::Link).show(ui, p).on_hover_text(crate::makemkv::GET_URL).clicked() {
                app.m.open_url(crate::makemkv::GET_URL);
            }
            if app.m.makemkv.changing && Btn::new("", "Cancel").look(Look::Quiet).font(font(13.0, 500)).show(ui, p).clicked() {
                app.m.makemkv.changing = false;
                app.m.makemkv.error = None;
            }
        });
        ui.add_space(4.0);
        wrap(ui, "Get MakeMKV opens makemkv.com. On Linux it's also on Flathub, as com.makemkv.MakeMKV.", p.muted);
    }
    if let Some(e) = &app.m.makemkv.error {
        ui.add_space(6.0);
        wrap(ui, e, p.bad);
    }
    if full {
        heading(ui, p, "MakeMKV's key");
        let on = app.m.makemkv.key_updates;
        let name = "Keep MakeMKV's beta key up to date";
        if crate::widgets::switch(ui, p, name, name, on, font(13.0, 500)).clicked() {
            app.m.set_makemkv_key_updates(!on);
        }
        ui.add_space(2.0);
        wrap(ui, "MakeMKV needs a key, and its free beta key runs out every couple of months. When MakeMKV says it has, Disc Rip Trip fetches the current one from the MakeMKV forum and saves it in MakeMKV's settings. A key you bought is never changed. Off until you turn it on.", p.muted);
    }
}

/// What tells candidates apart, from their own facts: *year, country and
/// label*. Empty when nothing does, or there's only one.
fn differences(cands: &[drt_engine::Candidate]) -> String {
    if cands.len() < 2 {
        return String::new();
    }
    let words: Vec<&str> = [("year", "year"), ("country", "country"), ("label", "label"), ("catalognumber", "catalogue number")]
        .into_iter()
        .filter(|(k, _)| {
            let mut vals: Vec<&str> = cands.iter().filter_map(|c| c.facts.iter().find(|f| f.subject == Subject::Disc && f.key == *k).map(|f| f.value.as_str())).collect();
            vals.sort();
            vals.dedup();
            vals.len() > 1
        })
        .map(|(_, w)| w)
        .collect();
    match words.as_slice() {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Whether a step has something to list beside its settings yet.
fn has_items(id: &str, v: &StepView, info: &StepInfo, ctx: Option<&Context>) -> bool {
    match v.step.as_str() {
        "drive" | "next-disc" => false,
        "audio-cd" => ctx.is_some_and(|c| c.parts() > 0),
        "cd-text" => ctx.is_some_and(|c| !cdtext_rows(c, id).is_empty()),
        "choose" => !options_of(v, ctx).is_empty(),
        "cover-art" => ctx.is_some_and(|c| c.found_by(id).any(|f| f.key == "cover")),
        "lyrics" => !v.items.is_empty(),
        _ if info.group == Group::LookUp => ctx.is_some_and(|c| c.offered_by(id).next().is_some()),
        _ => !v.items.is_empty(),
    }
}

/// The candidates that reached a Choose, best first (ties in the order
/// they came), as it put them to you. None before any reached it; a run
/// kept from before Choose recorded them has every candidate offered in
/// the run instead.
fn options_of(v: &StepView, ctx: Option<&Context>) -> Vec<drt_engine::Candidate> {
    let mut cands = match (&v.record.options, &v.record.decided) {
        (Some(o), _) => o.clone(),
        (None, Some(_)) => ctx.map(|c| c.candidates().to_vec()).unwrap_or_default(),
        (None, None) => vec![],
    };
    cands.sort_by(|a, b| b.score.total_cmp(&a.score));
    cands
}

/// Swaps the header for the Remove confirm, if one is pending for `id`.
fn confirming(app: &App, mut plan: Plan, id: &str, v: &StepView) -> Plan {
    if let Some((x, gone)) = &app.confirm_remove {
        if x == id {
            let n = gone.len();
            plan.pill = Some((format!("Remove {n}?"), Tone::Bad, None));
            plan.say = if n == 1 { format!("Just {}.", v.name) } else { format!("{} and the {} after it.", v.name, n - 1) };
            plan.acts = vec![Act::Keep, Act::Remove];
            plan.remove = false;
        }
    }
    plan
}

/// The tile's state, as the header's pill.
/// The pill: the state in one word, as the card shows it: *Ready*,
/// *Waiting*, the working verb, *Needs you*, *Done*, *Skipped*, *Failed*.
/// Orange only for *Needs you*.
fn state_pill(v: &StepView, info: &Option<std::sync::Arc<StepInfo>>) -> (String, Tone, Option<StepState>) {
    use crate::card::Shown;
    let retries = info.as_ref().is_some_and(|i| i.retries.iter().any(|r| !matches!(r.id, "keep" | "leave_out")));
    let shown = Shown::of(v, retries);
    let verb = match info.as_ref().map(|i| i.group) {
        Some(Group::LookUp) => "Asking",
        Some(Group::Read | Group::Kind) => "Reading",
        Some(Group::Make) => "Making",
        Some(Group::Place) => "Filing",
        Some(Group::Check) => "Checking",
        _ => "Working",
    };
    let verb = if crate::tray::is_encoder(&v.step) { "Encoding" } else { verb };
    match shown {
        Shown::Idle => ("Ready".into(), Tone::Plain, Some(StepState::Idle)),
        Shown::Wait => ("Waiting".into(), Tone::Plain, Some(StepState::Wait)),
        Shown::Busy => (verb.into(), Tone::Busy, Some(StepState::Busy)),
        Shown::Ok => ("Done".into(), Tone::Ok, Some(StepState::Ok)),
        Shown::Needs => ("Needs you".into(), Tone::Warn, Some(StepState::Needs)),
        Shown::Bad => ("Failed".into(), Tone::Bad, Some(StepState::Bad)),
        Shown::Skipped => ("Skipped".into(), Tone::Plain, None),
    }
}

/// The settings shown: beside a list (`beside`), the ones the card shows
/// locked go; with nothing to list, every one shows, as text.
fn rail_specs_beside(app: &App, i: usize, id: &str, info: &StepInfo, beside: bool) -> Vec<ParamSpec> {
    let values: Vec<(&str, Option<serde_json::Value>)> = info.params.iter().map(|s| (s.key, app.m.shown_value(i, id, s))).collect();
    let get = |k: &str| values.iter().find(|(x, _)| *x == k).and_then(|(_, v)| v.clone());
    let locked = read_only(app, i, Scope::Recipe);
    info.params
        .iter()
        .filter(|spec| !(info.kind == "drive" && spec.key == "drive") && !(spec.scope == Scope::Drive && app.m.drive(i).is_empty()) && spec.applies(get))
        .filter(|spec| !(beside && locked && spec.on_card.is_some()))
        .filter(|spec| !(spec.main_path_only && app.m.on_a_path(i, id)))
        .cloned()
        .collect()
}

/// A Folder step. `library` is its old name (`Registry::canonical`); old
/// recipes are migrated on load, so this is only belt and braces.
fn is_place(kind: &str) -> bool {
    matches!(kind, "folder" | "library")
}

pub fn is_encoder(kind: &str) -> bool {
    matches!(kind, "flac" | "mp3" | "opus" | "aac" | "vorbis" | "alac" | "wavpack" | "wav")
}

fn length(c: &Context, n: u32) -> Option<u32> {
    c.get(Subject::Part(n), "length").and_then(|l| l.parse().ok())
}

/// The folder a step's files actually went into: the deepest one that
/// holds them all (*~/Music/FLAC/The White Stripes/The White Stripes*);
/// before anything is filed, its base folder.
pub fn filed_dir(files: &[drt_engine::Made], base: &str) -> PathBuf {
    let dirs: Vec<PathBuf> = files.iter().filter_map(|f| expand(&f.path).parent().map(|d| d.to_owned())).collect();
    let Some(first) = dirs.first() else { return expand(base) };
    let mut common: Vec<std::path::Component> = first.components().collect();
    for d in &dirs[1..] {
        let n = common.iter().zip(d.components()).take_while(|(a, b)| **a == *b).count();
        common.truncate(n);
    }
    let path: PathBuf = common.iter().collect();
    if path.as_os_str().is_empty() {
        expand(base)
    } else {
        path
    }
}

fn folder_of(app: &App, i: usize, id: &str) -> Option<String> {
    let info = app.m.info(i, id)?;
    let spec = info.params.iter().find(|s| s.kind == ParamKind::Folder)?;
    app.m.shown_value(i, id, spec).map(|v| value_text(&v))
}

/// The titles CD-Text offered: its candidate's, by track. (A run kept
/// from before it offered one has them as its own facts.)
fn cdtext_rows(c: &Context, id: &str) -> Vec<(u32, String)> {
    let offered = c.offered_by(id).next().map(|x| x.facts.iter().collect::<Vec<_>>());
    let facts = offered.unwrap_or_else(|| c.found_by(id).collect());
    let mut rows: Vec<(u32, String)> = facts
        .into_iter()
        .filter(|f| f.key == "title")
        .filter_map(|f| match f.subject {
            Subject::Part(n) => Some((n, f.value.clone())),
            Subject::Disc => None,
        })
        .collect();
    rows.sort();
    rows.dedup();
    rows
}

// ---- the header -------------------------------------------------------------------------------

/// Top to bottom: the step's icon and name, with ✕ at the right; its
/// state and one sentence; then what to do, the primary action last.
fn header(app: &mut App, ui: &mut Ui, p: &Pal, plan: &Plan, t: f64) {
    let i = app.sel;
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        ui.painter().extend(kind_icon(&plan.icon, r.min, 18.0, plan.colour));
        // The close button's room, kept for it on the right, and Remove's.
        let remove_w = if plan.remove { ui.painter().layout_no_wrap("Remove".into(), font(13.0, 500), p.muted).size().x + 16.0 + 8.0 } else { 0.0 };
        // The state's pill, after the name on its row.
        let pill_w =
            plan.pill.as_ref().map_or(0.0, |(text, _, state)| ui.painter().layout_no_wrap(text.clone(), font(12.0, 500), p.ink).size().x + 16.0 + if state.is_some() { 16.0 } else { 0.0 } + 8.0);
        let corner_w = if plan.corner.is_some() { ui.painter().layout_no_wrap("Open tray".into(), font(13.0, 500), p.muted).size().x + 16.0 + 26.0 } else { 0.0 };
        let room = ui.available_width() - 40.0 - remove_w - pill_w - corner_w;
        match &plan.rename {
            // In edit mode, the tile's name is its label, edited here.
            Some(id) => {
                let key = format!("label/{id}");
                let buf = app.buffers.entry(key.clone()).or_insert_with(|| plan.name.clone());
                let w = (ui.painter().layout_no_wrap(buf.clone(), font(16.0, 600), p.ink).size().x + 22.0).clamp(80.0, room);
                let r = ui.add(egui::TextEdit::singleline(buf).font(font(16.0, 600)).desired_width(w).margin(vec2(8.0, 2.0)));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, "Step name"));
                if r.lost_focus() {
                    let text = app.buffers.remove(&key).unwrap_or_default();
                    if text.trim() != plan.name && !crate::widgets::escaped(ui) {
                        app.m.set_label(app.sel, id, &text);
                    }
                } else if !r.has_focus() {
                    app.buffers.remove(&key);
                }
            }
            None => {
                ui.scope(|ui| {
                    ui.set_max_width(room);
                    ui.add(egui::Label::new(RichText::new(&plan.name).font(font(16.0, 600))).truncate());
                });
            }
        }
        if let Some((text, tone, state)) = &plan.pill {
            pill(ui, p, text, *tone, *state, t);
        }
        ui.with_layout(L::right_to_left(Align::Center), |ui| {
            if Btn::new("x", "").name("Close").look(Look::Quiet).fg(p.subtle).show(ui, p).clicked() {
                app.close_tray();
            }
            if plan.remove && quiet_bad(ui, p, "Remove") {
                act_on(app, ui, Act::AskRemove);
            }
            if let Some(Act::OpenTray { busy, measuring }) = &plan.corner {
                let says = if *measuring { "Measuring the offset: the tray can open once that's done" } else { "Stops any read on this drive" };
                let b = Btn::new("next-disc", if *busy { "Opening…" } else { "Open tray" }).look(Look::Quiet).font(font(13.0, 500)).enabled(!busy && !measuring).show(ui, p);
                if b.on_hover_text(says).on_disabled_hover_text(if *measuring { says } else { "Opening the tray" }).clicked() {
                    act_on(app, ui, Act::OpenTray { busy: *busy, measuring: *measuring });
                }
            }
        });
    });
    // The sentence, on a row of its own, only when there is one; a
    // refusal of something done here in its place.
    if let Some((why, _)) = &app.flash {
        ui.add_space(2.0);
        ui.add(egui::Label::new(RichText::new(why).font(font(13.0, 500)).color(p.bad)).wrap());
    } else if !plan.say.is_empty() {
        ui.add_space(2.0);
        ui.add(egui::Label::new(RichText::new(&plan.say).font(font(13.0, 400)).color(p.ink)).wrap());
    }
    if plan.acts.is_empty() {
        return;
    }
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for (k, act) in plan.acts.iter().enumerate() {
            let primary = k + 1 == plan.acts.len() && !matches!(act, Act::OpenFolder { .. } | Act::AskRemove | Act::Keep | Act::Skip | Act::OpenTray { .. });
            let clicked = match act {
                Act::OpenFolder { label, path } => Btn::new("folder-link", label).look(Look::Quiet).font(font(13.0, 500)).show(ui, p).on_hover_text(path.display().to_string()).clicked(),
                Act::LetThrough(n) => Btn::new("play", &if *n > 0 { format!("Let {n} through") } else { "Let them through".into() }).look(Look::Primary).show(ui, p).clicked(),
                Act::UseThis(_) => Btn::new("", "Use this one").look(Look::Primary).show(ui, p).clicked(),
                Act::Answer(_) => Btn::new("", "Use").look(Look::Primary).show(ui, p).clicked(),
                Act::Measure { busy } => {
                    let label = if *busy { "Measuring…" } else { "Measure offset" };
                    Btn::new("", label).look(if primary { Look::Primary } else { Look::Plain }).enabled(!busy && !app.m.finding.contains_key(&i)).show(ui, p).clicked()
                }
                Act::OpenTray { busy, measuring } => Btn::new("next-disc", if *busy { "Opening…" } else { "Open tray" })
                    .enabled(!busy && !measuring)
                    .show(ui, p)
                    .on_hover_text(if *measuring { "Measuring the offset: the tray can open once that's done" } else { "Stops any read on this drive; a track being read waits for you" })
                    .on_disabled_hover_text(if *measuring { "Measuring the offset: the tray can open once that's done" } else { "Opening the tray" })
                    .clicked(),
                // The one thing to do with a step this build doesn't have.
                Act::AskRemove if plan.body == Body::None && plan.pill.as_ref().is_some_and(|x| x.0 == "Not available") => danger(ui, p, "Remove"),
                Act::AskRemove => Btn::new("trash", "Remove").look(Look::Quiet).fg(p.bad).font(font(13.0, 500)).show(ui, p).clicked(),
                Act::Remove => danger(ui, p, "Remove"),
                Act::Keep => Btn::new("", "Don't remove").look(Look::Quiet).font(font(13.0, 500)).show(ui, p).clicked(),
                Act::Skip => Btn::new("", "Skip").look(Look::Quiet).font(font(13.0, 500)).show(ui, p).clicked(),
                Act::UseThese => Btn::new("", "Use these").look(Look::Primary).show(ui, p).clicked(),
                Act::RetryAll { .. } => Btn::new("", "Read them all again").look(Look::Primary).show(ui, p).clicked(),
            };
            if clicked {
                act_on(app, ui, act.clone());
            }
        }
    });
}

/// A quiet button that turns the bad colour under the pointer: Remove in
/// the header.
fn quiet_bad(ui: &mut Ui, p: &Pal, label: &str) -> bool {
    let g = ui.painter().layout_no_wrap(label.to_owned(), font(13.0, 500), p.muted);
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 26.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, label));
    let hot = resp.hovered() || resp.has_focus();
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(r, 6.0, mix(p.bad, 0.18, p.raised));
    } else if hot {
        ui.painter().rect_filled(r, 6.0, mix(p.bad, 0.10, p.raised));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let col = if hot { p.bad } else { p.muted };
    let g = ui.painter().layout_no_wrap(label.to_owned(), font(13.0, 500), col);
    ui.painter().galley(pos2(r.left() + 8.0, r.center().y - g.size().y / 2.0), g, col);
    crate::widgets::focus_ring(ui, &resp, r, 6.0, p);
    resp.clicked()
}

/// The confirm of a destructive action: filled in the bad colour.
/// A red button, for the one thing a confirm does.
pub fn danger_button(ui: &mut Ui, p: &Pal, label: &str) -> bool {
    red(ui, p, label, None)
}

fn danger(ui: &mut Ui, p: &Pal, label: &str) -> bool {
    red(ui, p, label, Some("trash"))
}

fn red(ui: &mut Ui, p: &Pal, label: &str, icon: Option<&str>) -> bool {
    let g = ui.painter().layout_no_wrap(label.to_owned(), font(14.0, 500), p.raised);
    let lead = if icon.is_some() { 31.0 } else { 14.0 };
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + lead + 14.0, 32.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, label));
    let fill = if resp.hovered() { mix(p.ink, 0.12, p.bad) } else { p.bad };
    ui.painter().rect_filled(r, 7.0, fill);
    let ink = if p.dark { p.bg } else { Color32::WHITE };
    if let Some(icon) = icon {
        ui.painter().extend(kind_icon(icon, pos2(r.left() + 12.0, r.center().y - 7.5), 15.0, ink));
    }
    ui.painter().galley(pos2(r.left() + lead, r.center().y - g.size().y / 2.0), g, ink);
    resp.clicked()
}

fn act_on(app: &mut App, ui: &Ui, act: Act) {
    let i = app.sel;
    let id = match &app.tray {
        Some(Tray::Step { id, .. } | Tray::Question { step: id }) => id.clone(),
        _ => String::new(),
    };
    match act {
        Act::OpenFolder { path, .. } => app.m.open(&path),
        Act::LetThrough(_) => app.m.send(i, Command::Approve { step: id }),
        // Past the candidates is the details form: what you typed.
        Act::UseThis(pick) => {
            let answer = match details_of(app) {
                Some(keys) if pick >= candidates(app) => Answer::Details(keys.into_iter().map(|(k, v)| (k.clone(), app.buffers.get(&format!("details/{k}")).cloned().unwrap_or(v))).collect()),
                _ => Answer::Pick(pick),
            };
            app.m.send(i, Command::Answer { step: id, answer });
        }
        Act::Skip => app.m.send(i, Command::Answer { step: id, answer: Answer::Skip }),
        Act::UseThese => {
            let picks = std::mem::take(&mut app.per_item);
            app.m.send(i, Command::Answer { step: id, answer: Answer::PerItem(picks.into_iter().collect()) });
        }
        Act::Answer(step) => {
            let text = app.buffers.get(&format!("q/{step}")).cloned().unwrap_or_default();
            app.m.send(i, Command::Answer { step, answer: Answer::Value(text) });
        }
        Act::Measure { .. } if !app.m.locked(i, Scope::Drive) => app.m.look_up(i, true),
        Act::Measure { .. } => {}
        Act::OpenTray { .. } => app.m.open_tray(i),
        Act::AskRemove => {
            let gone = app.m.removal(i, &id);
            if gone.is_empty() {
                app.refuse(ui.ctx(), "last step");
            } else {
                app.confirm_remove = Some((id, gone));
            }
        }
        Act::Remove => {
            app.confirm_remove = None;
            match app.m.remove_step(i, &id) {
                Ok(_) => app.close_tray(),
                Err(e) => app.refuse(ui.ctx(), e.to_string()),
            }
        }
        Act::Keep => app.confirm_remove = None,
        Act::RetryAll { items, option } => {
            for item in items {
                app.m.send(i, Command::Retry { step: id.clone(), item, option: option.clone() });
            }
        }
    }
}

// ---- the settings column ---------------------------------------------------------------------

/// A setting: its label, its control, where its value came from.
fn field(ui: &mut Ui, p: &Pal, label: &str, control: impl FnOnce(&mut Ui, egui::Id), from: Option<egui::text::LayoutJob>) {
    ui.add_space(2.0);
    let l = ui.label(RichText::new(label).font(font(12.5, 400)).color(p.muted));
    ui.add_space(2.0);
    control(ui, l.id);
    if let Some(job) = from.filter(|j| !j.text.is_empty()) {
        ui.add_space(1.0);
        ui.label(job);
    }
    ui.add_space(12.0);
}

/// A setting with a Reset after its provenance, when it's yours. Gives
/// whether Reset was clicked.
fn field_reset(ui: &mut Ui, p: &Pal, label: &str, control: impl FnOnce(&mut Ui, egui::Id), from: egui::text::LayoutJob, reset: bool) -> bool {
    ui.add_space(2.0);
    let l = ui.label(RichText::new(label).font(font(12.5, 400)).color(p.muted));
    ui.add_space(2.0);
    control(ui, l.id);
    let mut clicked = false;
    if from.text.is_empty() && !reset {
        ui.add_space(12.0);
        return false;
    }
    ui.add_space(1.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(from);
        if reset {
            let g = ui.painter().layout_no_wrap("Reset".into(), font(12.0, 500), p.action);
            let (r, resp) = ui.allocate_exact_size(g.size(), Sense::click());
            let name = format!("Reset {label}");
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &name));
            let col = if resp.hovered() { p.ink } else { p.action };
            ui.painter().galley(r.min, g, col);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            clicked = resp.clicked();
        }
    });
    ui.add_space(12.0);
    clicked
}

/// "**yours**, 18 Nov", "**AccurateRip**, 528 agree", "default".
fn provenance(p: &Pal, set: &Setting<serde_json::Value>, unset_word: Option<(&str, Color32)>) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let faint = egui::TextFormat::simple(font(12.0, 400), p.subtle);
    let strong = egui::TextFormat::simple(font(12.0, 500), p.muted);
    match set {
        _ if unset_word.is_some() => {
            let (w, c) = unset_word.unwrap();
            job.append(w, 0.0, egui::TextFormat::simple(font(12.0, 500), c));
        }
        Setting::Chosen { at, .. } => {
            job.append("yours", 0.0, strong);
            job.append(&format!(", {}", day_month(*at)), 0.0, faint);
        }
        Setting::Detected { source, .. } => match source.split_once(", ") {
            Some((head, rest)) => {
                job.append(head, 0.0, strong);
                job.append(&format!(", {rest}"), 0.0, faint);
            }
            None => job.append(source, 0.0, strong),
        },
        // A default says nothing: no caption.
        Setting::Default => {}
        _ => job.append(set.provenance(), 0.0, faint),
    }
    job
}

/// "18 Nov", from seconds since the epoch (UTC).
pub fn day_month(secs: u64) -> String {
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {}", M[(m - 1) as usize])
}

/// A step's settings, stacked one per row, label then control: its own
/// first, untitled; then each named section; Advanced last, folded away
/// until asked for. A setting that only applies in another mode is hidden.
/// The one being asked for (`focus`) is outlined in amber while it's still
/// missing, and has the keys.
fn settings(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, info: &StepInfo, focus: Option<&str>) -> usize {
    let i = app.sel;
    let specs: Vec<ParamSpec> = rail_specs_beside(app, i, id, info, false);
    let mut sections: Vec<&str> = vec![""];
    for s in &specs {
        if !sections.contains(&s.section) {
            sections.push(s.section);
        }
    }
    // Advanced goes last, whatever order the step lists it in.
    sections.sort_by_key(|s| (*s == "Advanced") as u8);
    let mut shown = 0;
    // While a disc is going through, nothing here changes: the values, as
    // text, with where they came from.
    if (!specs.is_empty() || info.kind == "drive") && read_only(app, i, Scope::Recipe) {
        let why = if app.m.ripping(i) { "Locked while ripping" } else { "As this disc was ripped" };
        ui.label(RichText::new(why).font(font(13.0, 500)).color(p.subtle));
        ui.add_space(4.0);
    }
    let missing: Vec<String> = app.m.engine.missing(&app.m.recipes[i]).into_iter().filter(|m| m.step == id).map(|m| m.key.to_owned()).collect();
    for section in sections {
        let mut blocks: Vec<Block> = specs.iter().filter(|s| s.section == section).map(|s| Block::Param(Box::new(s.clone()))).collect();
        if section.is_empty() && info.kind == "drive" {
            blocks.insert(0, Block::Drives);
        }
        if blocks.is_empty() {
            continue;
        }
        if !section.is_empty() {
            ui.add_space(4.0);
            // A quiet heading; nothing folds away.
            ui.add_space(6.0);
            ui.label(RichText::new(section).font(font(12.5, 600)).color(p.muted));
            ui.add_space(6.0);
        }
        for b in blocks {
            let key = match &b {
                Block::Param(spec) => spec.key,
                Block::Drives => "drive",
            };
            let asked = focus == Some(key);
            let top = ui.cursor().top();
            match b {
                Block::Param(spec) => setting(app, ui, p, id, &spec, asked),
                Block::Drives => {
                    ui.add_space(2.0);
                    let l = ui.label(RichText::new("Drive").font(font(12.5, 400)).color(p.muted));
                    ui.add_space(2.0);
                    let open = !read_only(app, i, Scope::Recipe);
                    drive_choice(app, ui, p, id, open, l.id);
                    ui.add_space(12.0);
                }
            }
            // Asked for and still missing: outlined in amber, and in view.
            if asked && missing.iter().any(|k| k == key) {
                let r = Rect::from_min_max(pos2(ui.max_rect().left() - 8.0, top - 4.0), pos2(ui.max_rect().right() + 8.0, ui.cursor().top() - 6.0));
                ui.painter().rect_stroke(r, 8.0, Stroke::new(1.5, p.warn), egui::StrokeKind::Inside);
                if !app.focus_done {
                    ui.scroll_to_rect(r, None);
                    app.focus_done = true;
                }
            }
            shown += 1;
        }
    }
    shown
}

/// One thing in the settings.
enum Block {
    Param(Box<ParamSpec>),
    /// Which drive the recipe reads from.
    Drives,
}

fn setting(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, spec: &ParamSpec, focus: bool) {
    let i = app.sel;
    let set = app.m.shown_param(i, id, spec);
    let value = app.m.shown_value(i, id, spec);
    let unset = value.is_none();
    let key = format!("{id}/{}", spec.key);
    let finding = app.m.finding.get(&i).copied();
    let unset_word = if !unset {
        None
    } else if finding.is_some() {
        Some(("looking up", p.busy))
    } else if app.m.measure_failed.contains(&i) {
        Some(("Not measured yet: put in a well-known CD, then Measure offset", p.warn))
    } else {
        Some(("not set", p.warn))
    };
    // Text that didn't read: why, where the provenance goes.
    let from = match app.invalid.get(&key) {
        Some(why) => egui::text::LayoutJob::single_section(why.clone(), egui::TextFormat::simple(font(12.0, 500), p.bad)),
        None => {
            // What the recipe came with isn't yours: nothing to say.
            let shown = if matches!(set, Setting::Chosen { .. }) && !app.m.is_yours(i, id, spec, app.m.builtin(i).as_ref()) { Setting::Default } else { set.clone() };
            let mut job = provenance(p, &shown, unset_word);
            // One for every recipe: says so.
            if spec.scope == Scope::App {
                let sep = if job.text.is_empty() { "" } else { ", " };
                job.append(&format!("{sep}app-wide"), 0.0, egui::TextFormat::simple(font(12.0, 400), p.subtle));
            }
            job
        }
    };
    let label = spec.label;
    // A key is never shown whole once it's set: its last four.
    let secret = is_secret(spec);
    // An offset AccurateRip lists is the drive's, and no one disc can
    // change it: the value with where it came from, as text.
    if spec.key == "offset" && !unset && set.provenance().starts_with("AccurateRip") {
        let n = value.as_ref().map(|v| if v.as_i64().is_some_and(|n| n > 0) { format!("+{}", value_text(v)) } else { value_text(v) }).unwrap_or_default();
        let mut job = egui::text::LayoutJob::default();
        job.append(&n, 0.0, egui::TextFormat::simple(font(13.5, 500), p.ink));
        job.append(&format!(" · {}", set.provenance()), 0.0, egui::TextFormat::simple(font(13.5, 400), p.muted));
        field(
            ui,
            p,
            label,
            |ui, l| {
                help_line(ui, p, spec);
                ui.add_space(5.0);
                let r = ui.add(egui::Label::new(job).wrap());
                let _ = r.labelled_by(l);
                ui.add_space(5.0);
            },
            None,
        );
        return;
    }
    if read_only(app, i, spec.scope) {
        // The value as text, in the same place; nothing to change.
        let text = match (&spec.kind, value.as_ref()) {
            (_, None) => "—".to_owned(),
            (ParamKind::Toggle, Some(v)) => if v.as_bool() == Some(true) { "On" } else { "Off" }.to_owned(),
            (ParamKind::Choice(_), Some(v)) => cap(spec.choice_label(&value_text(v))),
            (_, Some(v)) if spec.key == "offset" && v.as_i64().is_some_and(|n| n > 0) => format!("+{}", value_text(v)),
            (_, Some(v)) if secret => masked(&value_text(v)),
            (_, Some(v)) => value_text(v),
        };
        match &spec.kind {
            ParamKind::Template(_) => field(
                ui,
                p,
                label,
                |ui, l| {
                    help_line(ui, p, spec);
                    template_text(app, ui, p, id, spec, l)
                },
                Some(from),
            ),
            _ => field(
                ui,
                p,
                label,
                |ui, l| {
                    help_line(ui, p, spec);
                    plain(ui, p, &text, l)
                },
                Some(from),
            ),
        }
        return;
    }
    let mut commit: Option<serde_json::Value> = None;
    // A drive setting found by testing (its sector order) can be reset to be found again.
    let found = spec.scope == Scope::Drive && !spec.findable && matches!(set, Setting::Detected { .. });
    let yours = (app.m.is_yours(i, id, spec, app.m.builtin(i).as_ref()) || found) && !app.invalid.contains_key(&key);
    let reset = field_reset(
        ui,
        p,
        label,
        |ui, label_id| {
            help_line(ui, p, spec);
            match &spec.kind {
                ParamKind::Toggle => {
                    let on = value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false);
                    if let Some(k) = segmented(ui, p, spec.label, &["On", "Off"], Some(if on { 0 } else { 1 }), true) {
                        commit = Some((k == 0).into());
                    }
                }
                ParamKind::Choice(options) => {
                    let cur = value.as_ref().map(value_text).unwrap_or_default();
                    let chosen = options.iter().position(|o| *o == cur);
                    let shown: Vec<String> = options.iter().map(|o| cap(spec.choice_label(o))).collect();
                    let shown_ref: Vec<&str> = shown.iter().map(String::as_str).collect();
                    // Segmented when the options fit across the column.
                    let width: f32 = shown.iter().map(|o| ui.painter().layout_no_wrap(o.clone(), font(13.0, 400), p.ink).size().x + 20.0).sum();
                    if width <= ui.available_width() {
                        if let Some(k) = segmented(ui, p, spec.label, &shown_ref, chosen, true) {
                            commit = Some(options[k].into());
                        }
                    } else {
                        let r = crate::widgets::combo(&key, p)
                            .selected_text(if cur.is_empty() { "—".to_owned() } else { cap(spec.choice_label(&cur)) })
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for o in options.iter() {
                                    if ui.selectable_label(cur == *o, cap(spec.choice_label(o))).clicked() {
                                        commit = Some((*o).into());
                                    }
                                }
                            })
                            .response;
                        let _ = r.labelled_by(label_id);
                    }
                }
                ParamKind::Template(presets) => template_field(app, ui, p, id, spec, presets, &key, label_id, &mut commit),
                ParamKind::Range { min, max } => {
                    // A slider and its value; saved when you let go.
                    let cur = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(*min);
                    let mut v: i64 = app.buffers.get(&key).and_then(|b| b.parse().ok()).unwrap_or(cur);
                    // As wide as the column, less the value's box.
                    ui.spacing_mut().slider_width = (ui.available_width() - 56.0).max(80.0);
                    let r = ui.add(egui::Slider::new(&mut v, *min..=*max).show_value(true).integer()).labelled_by(label_id);
                    if r.dragged() {
                        app.buffers.insert(key.clone(), v.to_string());
                    } else {
                        app.buffers.remove(&key);
                        if v != cur {
                            commit = Some(v.into());
                        }
                    }
                }
                kind => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let shown = value.as_ref().map(|v| {
                            let t = value_text(v);
                            if spec.key == "offset" && v.as_i64().is_some_and(|n| n > 0) {
                                format!("+{t}")
                            } else if secret {
                                masked(&t)
                            } else {
                                t
                            }
                        });
                        // Typed text is kept only while the box has focus;
                        // otherwise it shows the value as it stands.
                        let extra = match kind {
                            ParamKind::Folder => 36.0,
                            _ if spec.key == "offset" && !unset => 90.0,
                            _ if spec.suggest.is_some() => ui.painter().layout_no_wrap(SUGGEST.into(), font(13.0, 500), p.muted).size().x + 16.0 + 6.0,
                            _ => 0.0,
                        };
                        let buf = app.buffers.entry(key.clone()).or_insert_with(|| shown.clone().unwrap_or_default());
                        let r = ui.add(egui::TextEdit::singleline(buf).desired_width(ui.available_width() - extra).font(font(13.5, 400)).margin(vec2(8.0, 5.0))).labelled_by(label_id);
                        if focus && !app.focus_done {
                            r.request_focus();
                            app.focus_done = true;
                        }
                        // A key: typing replaces it, never edits the dots.
                        if secret && r.gained_focus() && shown.is_some() {
                            app.buffers.insert(key.clone(), String::new());
                        }
                        if app.invalid.contains_key(&key) {
                            ui.painter().rect_stroke(r.rect, 6.0, Stroke::new(1.5, p.bad), egui::StrokeKind::Inside);
                        }
                        if r.lost_focus() && crate::widgets::escaped(ui) {
                            // Esc: back to the value as it stands.
                            app.buffers.remove(&key);
                            app.invalid.remove(&key);
                        } else if r.lost_focus() {
                            let text = app.buffers.get(&key).cloned().unwrap_or_default();
                            match text_value(kind, &text) {
                                _ if text.trim() == shown.as_deref().unwrap_or("").trim() => {
                                    app.buffers.remove(&key);
                                    app.invalid.remove(&key);
                                }
                                Some(v) => {
                                    commit = Some(v);
                                    app.invalid.remove(&key);
                                }
                                None if text.trim().is_empty() => {
                                    app.buffers.remove(&key);
                                    app.invalid.remove(&key);
                                }
                                // Kept as typed, with why; nothing saved.
                                None => {
                                    let why = if spec.key == "offset" { "A whole number, like +103" } else { "A number, like 8" };
                                    app.invalid.insert(key.clone(), why.into());
                                }
                            }
                        } else if !r.has_focus() && !app.invalid.contains_key(&key) {
                            app.buffers.remove(&key);
                        }
                        // A value made up for you (an ntfy topic), saved at once.
                        if let Some(make) = spec.suggest {
                            if Btn::new("", SUGGEST).look(Look::Quiet).font(font(13.0, 500)).pad(vec2(8.0, 6.0)).show(ui, p).clicked() {
                                commit = Some(make().into());
                                app.invalid.remove(&key);
                            }
                        }
                        if *kind == ParamKind::Folder && Btn::new("folder-link", "").name("Choose folder").pad(vec2(7.0, 6.0)).show(ui, p).clicked() {
                            let hook = app.m.pick_folder.clone();
                            let tx = app.picked.0.clone();
                            let ctx = ui.ctx().clone();
                            let (step, k) = (id.to_owned(), spec.key.to_owned());
                            // It starts at the folder set, or the nearest one of
                            // its parents that's there.
                            let start = value.as_ref().map(|v| expand(&value_text(v))).and_then(|p| p.ancestors().find(|a| a.is_dir()).map(|a| a.to_owned()));
                            std::thread::spawn(move || {
                                if let Some(path) = hook(start) {
                                    let _ = tx.send((i, step, k, path.display().to_string()));
                                    ctx.request_repaint();
                                }
                            });
                        }
                        // Measure again, beside a known offset; not for a drive
                        // AccurateRip lists, whose offset no one disc can change.
                        if spec.key == "offset" && spec.findable && !unset && !set.provenance().starts_with("AccurateRip") {
                            let busy = finding == Some(true);
                            if Btn::new("", if busy { "Measuring…" } else { "Measure offset" }).font(font(13.0, 500)).pad(vec2(10.0, 5.0)).enabled(finding.is_none()).show(ui, p).clicked() {
                                app.m.look_up(i, true);
                            }
                        }
                    });
                }
            }
        },
        from,
        yours,
    );
    if reset {
        if let Err(e) = app.m.reset_param(i, id, spec.key) {
            app.refuse(ui.ctx(), e.to_string());
        }
        app.buffers.remove(&key);
    }
    if let Some(v) = commit {
        if let Err(e) = app.m.set_param(i, id, spec.key, v) {
            app.refuse(ui.ctx(), e.to_string());
        }
        app.buffers.remove(&key);
    }
}

/// The button beside a text setting that can make up its value.
const SUGGEST: &str = "Make one up";

/// Notify's *Send a test*: for which recipe and step, the topic it was
/// sent to, and how it went (`None` while it's sending).
pub struct TestSend {
    pub at: (usize, String),
    pub topic: String,
    pub result: Option<Result<Vec<&'static str>, String>>,
}

/// *Send a test*, under Notify's settings: a test through each way that's
/// on, sent on the app's runtime, and how it went under the button. Not
/// while the settings are locked.
fn send_test(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, info: &StepInfo) {
    let i = app.sel;
    let get = |k: &str| info.params.iter().find(|s| s.key == k).and_then(|s| app.m.value(i, id, s));
    let desktop = get("desktop").and_then(|v| v.as_bool()).unwrap_or(true);
    let server = get("ntfy_server").map(|v| value_text(&v)).unwrap_or_default();
    let topic = get("ntfy_topic").map(|v| value_text(&v).trim().to_owned()).unwrap_or_default();
    let at = (i, id.to_owned());
    let mine = app.test_send.lock().unwrap().as_ref().filter(|t| t.at == at).map(|t| (t.topic.clone(), t.result.clone()));
    let sending = matches!(mine, Some((_, None)));
    ui.add_space(6.0);
    let open = !sending && !read_only(app, i, Scope::Recipe);
    if Btn::new("", if sending { "Sending…" } else { "Send a test" }).font(font(13.0, 500)).pad(vec2(10.0, 5.0)).enabled(open).show(ui, p).clicked() {
        *app.test_send.lock().unwrap() = Some(TestSend { at: at.clone(), topic: topic.clone(), result: None });
        let sent = (app.m.send_test)(desktop, server, topic);
        let (slot, ctx) = (app.test_send.clone(), ui.ctx().clone());
        app.m.rt.spawn(async move {
            let r = sent.await.map_err(|e| e.to_string());
            if let Some(t) = slot.lock().unwrap().as_mut().filter(|t| t.at == at) {
                t.result = Some(r);
            }
            ctx.request_repaint();
        });
    }
    // The topic is said here, to you, and nowhere else.
    if let Some((topic, Some(r))) = mine {
        let (text, col) = match r {
            Ok(to) if to.is_empty() => ("Nowhere to send it: turn on On this computer, or set a topic".to_owned(), p.muted),
            Ok(to) => (format!("Sent to {}", to.iter().map(|w| if *w == "ntfy" { format!("ntfy topic {topic}") } else { (*w).to_owned() }).collect::<Vec<_>>().join(" and to ")), p.ok),
            Err(e) => (e, p.bad),
        };
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(text).font(font(12.5, 400)).color(col)).wrap());
    }
}

/// A setting's help, if it has any: one short line under its name.
fn help_line(ui: &mut Ui, p: &Pal, spec: &ParamSpec) {
    if let Some(h) = spec.help {
        ui.add(egui::Label::new(RichText::new(h).font(font(12.0, 400)).color(p.subtle)).wrap());
        ui.add_space(4.0);
    }
}

/// Whether a setting of this scope is shown read-only: the recipe is
/// ripping (or another recipe reads from its drive, for the drive's own
/// settings), or it's an earlier disc's, as that run used it.
fn read_only(app: &App, i: usize, scope: Scope) -> bool {
    app.m.locked(i, scope) || !app.m.shows_newest(i)
}

/// Whether a setting is a secret (an API key): shown only as its last four.
fn is_secret(spec: &ParamSpec) -> bool {
    spec.key.contains("key") || spec.key.contains("token") || spec.key.contains("password")
}

/// "••••1234".
fn masked(v: &str) -> String {
    let tail: String = v.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("••••{tail}")
}

/// A setting's value as text where its control would be, as tall as one.
fn plain(ui: &mut Ui, p: &Pal, text: &str, label_id: egui::Id) {
    ui.add_space(5.0);
    let r = ui.add(egui::Label::new(RichText::new(text).font(font(13.5, 400)).color(p.ink)).wrap());
    let _ = r.labelled_by(label_id);
    ui.add_space(5.0);
}

/// A path template, read-only: its preset (or Custom), the template, and
/// where a file goes.
fn template_text(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, spec: &ParamSpec, label_id: egui::Id) {
    let i = app.sel;
    let snap = app.m.snapshot(i);
    let stored = app.m.shown_value(i, id, spec).map(|v| value_text(&v)).unwrap_or_else(|| template::DEFAULT.to_owned());
    let n = names(app, id, spec, &stored);
    plain(ui, p, n.preset.unwrap_or("Custom"), label_id);
    ui.label(RichText::new(&n.tpl).font(font(13.0, 400)).color(p.muted));
    ui.add_space(3.0);
    ui.add(egui::Label::new(RichText::new(example(app, &snap, id, n.files, &n.tpl, n.preset)).font(font(12.0, 400)).color(p.subtle)).wrap());
}

/// Where a file would go: this disc's first track (or the disc's image),
/// or a sample one.
fn example(app: &App, snap: &Snapshot, id: &str, files: Files, shown: &str, preset: Option<&str>) -> String {
    let ctx = app.m.shown_run(app.sel).map(|r| r.run.context());
    if files == Files::Video {
        // A film; and for the Jellyfin layout, an episode too.
        let (film, ep) = video_samples();
        let one = template::render_video(shown, &film).display().to_string();
        return if preset == Some("Jellyfin") { format!("{one}\n{}", template::render_video(template::EPISODE_DEFAULT, &ep).display()) } else { one };
    }
    if matches!(files, Files::Game | Files::Data) {
        let f = match ctx.as_ref().filter(|c| c.get(Subject::Disc, "label").is_some() || c.get(Subject::Disc, "name").is_some()) {
            Some(c) => dump_fields(c),
            None => dump_sample(files),
        };
        format!("{}/", template::render_dump(shown, &f).display())
    } else {
        let ext = snap.steps.get(id).and_then(|v| encoder_before(snap, v)).map(|e| format!(".{}", ext_of(e))).unwrap_or_default();
        let f = match ctx.as_ref().filter(|c| c.get(Subject::Disc, "album").is_some()) {
            Some(c) => fields(Some(c), 1, "Track 1"),
            None => sample(),
        };
        format!("{}{ext}", template::render(shown, &f).display())
    }
}

/// A path template: a preset or Custom, the template itself to edit, an
/// example of where a file goes, and the tokens to put in it.
#[allow(clippy::too_many_arguments)]
fn template_field(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, spec: &ParamSpec, _all: &[(&str, &str)], key: &str, label_id: egui::Id, commit: &mut Option<serde_json::Value>) {
    let i = app.sel;
    // What it files decides its presets, tokens and example: disc images
    // name a folder from the dump's facts; tracks a path from the music's.
    let snap = app.m.snapshot(i);
    let stored = app.m.value(i, id, spec).map(|v| value_text(&v)).unwrap_or_else(|| template::DEFAULT.to_owned());
    // As the step reads it: its own, or the default for what reaches it.
    let Names { files, presets, tokens, preset, tpl } = names(app, id, spec, &stored);
    // The preset, or Custom.
    let combo = crate::widgets::combo((key, "preset"), p)
        .selected_text(preset.unwrap_or("Custom"))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (l, _) in presets {
                if ui.selectable_label(preset == Some(*l), *l).clicked() {
                    *commit = Some((*l).into());
                }
            }
            if ui.selectable_label(preset.is_none(), "Custom").clicked() && preset.is_some() {
                // The same layout, now yours to change.
                *commit = Some(tpl.clone().into());
            }
        })
        .response;
    let _ = combo.labelled_by(label_id);
    ui.add_space(4.0);
    // The template itself; typing makes it Custom.
    let buf = app.buffers.entry(key.to_owned()).or_insert_with(|| tpl.clone());
    let out = egui::TextEdit::singleline(buf).desired_width(ui.available_width()).font(font(13.0, 400)).margin(vec2(8.0, 5.0)).show(ui);
    out.response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, "Template"));
    let r = &out.response;
    if let Some(c) = out.cursor_range {
        app.cursors.insert(key.to_owned(), c.primary.index.0);
    }
    if r.lost_focus() {
        let text = app.buffers.remove(key).unwrap_or_default();
        if !text.trim().is_empty() && text.trim() != tpl && !crate::widgets::escaped(ui) {
            *commit = Some(text.trim().into());
        }
    } else if !r.has_focus() {
        app.buffers.remove(key);
    }
    let shown = app.buffers.get(key).cloned().unwrap_or_else(|| tpl.clone());
    let example = example(app, &snap, id, files, &shown, preset);
    ui.add_space(3.0);
    ui.add(egui::Label::new(RichText::new(example).font(font(12.0, 400)).color(p.muted)).wrap());
    ui.add_space(5.0);
    // The tokens, to put in at the cursor.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (tok, what) in tokens {
            let label = format!("{{{tok}}}");
            let g = ui.painter().layout_no_wrap(label.clone(), font(11.5, 500), p.muted);
            let (r, resp) = ui.allocate_exact_size(g.size() + vec2(12.0, 6.0), Sense::click());
            let name = format!("Insert {label}");
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &name));
            let fill = if resp.hovered() { p.bg } else { p.raised };
            ui.painter().rect(r, 5.0, fill, Stroke::new(1.0, p.line_soft), egui::StrokeKind::Inside);
            ui.painter().galley(r.min + vec2(6.0, 3.0), g, p.muted);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let resp = resp.on_hover_text(*what);
            if resp.clicked() {
                let text = app.buffers.get(key).cloned().unwrap_or_else(|| tpl.clone());
                let chars: Vec<char> = text.chars().collect();
                let at = app.cursors.get(key).copied().unwrap_or(chars.len()).min(chars.len());
                let new: String = chars[..at].iter().chain(label.chars().collect::<Vec<_>>().iter()).chain(chars[at..].iter()).collect();
                app.cursors.insert(key.to_owned(), at + label.chars().count());
                app.buffers.remove(key);
                *commit = Some(new.into());
            }
        }
    });
}

// ---- the items column ----------------------------------------------------------------------

/// A row of the items list.
#[derive(Clone)]
struct Row {
    n: Option<u32>,
    title: String,
    /// Why it needs you, on a second line in the bad colour.
    why: Option<String>,
    len: Option<String>,
    /// The verdict: its words, tone, and a faint note after it.
    verdict: Option<(String, Tone, String)>,
    /// A muted second column of text (a file name); `dim` is its faint tail.
    dest: Option<(String, String)>,
    /// Buttons in the row: (label, retry id, quiet).
    acts: Vec<(String, String, bool)>,
    item: Option<String>,
    attn: bool,
    /// How far along an item in progress is: a thin bar under its title.
    progress: Option<f32>,
    /// More proof, on hovering the row: each read and its CRC.
    hover: Option<String>,
    /// The row opens something under it (a track's lyrics): open or not.
    toggle: Option<bool>,
    /// Its file opens from a quiet *Open* at its end (a picture).
    open: bool,
    /// The file's size, in a column of its own before the verdict, and
    /// that column's width (the list's widest).
    size: Option<String>,
    size_w: f32,
    /// Its file always on a second line, under the title.
    two_line: bool,
    /// What its verdict can grow to while it changes (a percentage to
    /// 100 %): its column is that wide from the start, so it never moves.
    widest: Vec<(String, String)>,
    /// Its title muted: what came in, not what the step did.
    faint: bool,
}

impl Row {
    fn new(n: Option<u32>, title: impl Into<String>) -> Row {
        Row {
            n,
            title: title.into(),
            why: None,
            len: None,
            verdict: None,
            dest: None,
            acts: vec![],
            item: None,
            attn: false,
            progress: None,
            hover: None,
            toggle: None,
            open: false,
            widest: vec![],
            size: None,
            size_w: 0.0,
            two_line: false,
            faint: false,
        }
    }
}

/// The filter row: tabs and one muted fact on the right. Gives the tab
/// chosen: the one you picked, or Needs you if anything does, else All.
/// Filters only when two or more groups (besides All) have something in
/// them: on the tab row's right, or a row of their own before a run.
fn filter_row(app: &mut App, ui: &mut Ui, p: &Pal, options: &[(&str, usize)], default: usize, fact: &str) -> usize {
    // One stack, no filters: everything, the rows needing you first.
    let _ = (app, ui, p, fact);
    options.iter().position(|o| o.0 == "All").unwrap_or(default)
}

/// The file an item became, by its name.
/// The encoders name each file by its track alone (`07.flac`), so the
/// number is the match; never a title, which another track's may contain.
fn file_for<'a>(files: &'a [drt_engine::Made], n: u32, _label: &str) -> Option<&'a drt_engine::Made> {
    let want = format!("{n:02}");
    files.iter().find(|f| std::path::Path::new(&f.path).file_stem().is_some_and(|s| s.to_string_lossy() == want))
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_owned())
}

/// The rows, the ones needing you first, all of them.
fn rows(app: &mut App, ui: &mut Ui, p: &Pal, key: &str, mut list: Vec<Row>, _more_word: &str, t: f64) -> Vec<(String, String)> {
    app.cut_titles.clear();
    list.sort_by_key(|r| (!r.attn, r.n));
    // Every row, always: the sidebar scrolls, so nothing is hidden behind
    // an "N more".
    let mut out = Vec::new();
    let w = ui.available_width();
    let _ = key;
    let vw = verdict_column(ui, p, &list, w);
    // Sizes right-aligned in one column, as wide as the widest.
    let size_w = list.iter().filter_map(|r| r.size.as_ref()).map(|s| ui.painter().layout_no_wrap(s.clone(), crate::theme::tnum(13.0, 400), p.subtle).size().x).fold(0.0f32, f32::max);
    for r in &mut list {
        r.size_w = size_w;
    }
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (k, r) in list.iter().enumerate() {
            let (a, cut_short, _) = row(ui, p, w, vw, r, k == 0, t);
            if let Some(a) = a {
                out.push(a);
            }
            if cut_short {
                app.cut_titles.push(r.title.clone());
            }
        }
    });
    out
}

/// Whether rows are in the narrow sidebar (at 1280): a verdict with a
/// note drops its word (*✓ 104* under *AccurateRip*), a file goes under
/// its title, and a row's buttons get a line of their own.
fn narrow(w: f32) -> bool {
    w < 460.0
}

/// What a row's verdict says, as drawn: its word, unless narrow and it
/// has a note to say instead.
fn verdict_word(r: &Row, w: f32) -> Option<&str> {
    let (word, tone, note) = r.verdict.as_ref()?;
    Some(if narrow(w) && *tone == Tone::Ok && !note.is_empty() { "" } else { word.as_str() })
}

/// The verdict column's width for a list: its widest verdict's, up to a
/// third of the row, so every row's verdict starts at one x.
/// A row's verdict as wide as it can get while it changes: each number
/// with as many figures as it can grow to (a percentage, 100), the figures
/// tabular. Works on how many figures, not which.
fn widen(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            run.push(c);
            if !chars.peek().is_some_and(|n| n.is_ascii_digit()) {
                let pct = chars.peek().is_some_and(|n| *n == '%' || *n == ' ' && s.contains('%'));
                out.push_str(&"0".repeat(if pct { 3 } else { run.len().max(2) }));
                run.clear();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The figures in a list's verdicts: tabular, so a number that changes
/// keeps its width.
fn vword_font() -> egui::FontId {
    crate::theme::tnum(13.0, 500)
}

fn vnote_font() -> egui::FontId {
    crate::theme::tnum(13.0, 400)
}

/// The verdict column's width for a list: its widest verdict's, up to a
/// third of the row, so every row's verdict starts at one x. A verdict
/// that's changing (a busy row's) counts as wide as it can get, and so
/// does any a row says it may grow to, so the column never moves.
fn verdict_column(ui: &Ui, p: &Pal, list: &[Row], w: f32) -> f32 {
    let pt = ui.painter();
    let width = |word: &str, note: &str, icon: bool| {
        let ww = if word.is_empty() { 0.0 } else { pt.layout_no_wrap(word.to_owned(), vword_font(), p.muted).size().x + 6.0 };
        let nw = if note.is_empty() { 0.0 } else { pt.layout_no_wrap(note.to_owned(), vnote_font(), p.subtle).size().x };
        (if icon { 19.0 } else { 0.0 }) + ww + nw + 4.0
    };
    let need = list
        .iter()
        .filter_map(|r| {
            let (_, tone, note) = r.verdict.as_ref()?;
            let word = verdict_word(r, w).unwrap_or("");
            let icon = *tone != Tone::Plain;
            let mut n = width(word, note, icon);
            if *tone == Tone::Busy {
                n = n.max(width(&widen(word), &widen(note), icon));
            }
            for (a, b) in &r.widest {
                n = n.max(width(a, b, true));
            }
            Some(n)
        })
        .fold(0.0f32, f32::max);
    need.min(220.0).min(w * 0.36)
}

/// A header over the rows' right-hand column (the verdict's), so a bare
/// number in it reads: *AccurateRip* over *✓ 104*.
fn columns(ui: &mut Ui, p: &Pal, heads: &[(&str, f32)], vw: f32) {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
    let names = heads.iter().map(|h| h.0).collect::<Vec<_>>().join(", ");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Label, true, &names));
    for (text, dx) in heads {
        ui.painter().text(pos2(r.right() - 6.0 - vw + dx, r.center().y), egui::Align2::LEFT_CENTER, *text, font(11.5, 500), p.subtle);
    }
}

/// One row. Gives (item, retry) for a button clicked. A row that needs
/// you takes two lines: what it is and its verdict, then why, with what
/// you can do about it on the right.
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, p: &Pal, w: f32, vw: f32, r: &Row, first: bool, t: f64) -> (Option<(String, String)>, bool, bool) {
    let f = font(13.0, 400);
    let small = font(13.0, 400);
    let two = r.why.is_some();
    // Measured, not guessed: what each part needs. The title gets all the
    // room the others don't; a file name that won't fit beside it goes
    // under it instead of crushing it.
    let tf = if r.attn { font(13.0, 600) } else { f.clone() };
    let painter = ui.painter().clone();
    let measure = |text: &str, fnt: egui::FontId| painter.layout_no_wrap(text.to_owned(), fnt, p.ink).size().x;
    let title_need = measure(&r.title, tf.clone());
    let len_need = r.len.as_ref().map_or(0.0, |l| measure(l, crate::theme::tnum(13.0, 400)));
    let dest_need = r.dest.as_ref().map_or(0.0, |(a, b)| measure(&format!("{a}{b}"), crate::theme::tnum(13.0, 400)));
    let x0 = if r.attn { 8.0 } else { 0.0 } + if r.n.is_some() { 30.0 } else { 4.0 };
    let room = w - 6.0 - x0 - if r.verdict.is_some() { vw + 12.0 } else { 0.0 } - if r.len.is_some() { len_need + 12.0 } else { 0.0 } - if r.size.is_some() { r.size_w + 12.0 } else { 0.0 } - 8.0;
    let under = !two && r.dest.is_some() && (r.two_line || title_need + 14.0 + dest_need > room);
    // One line, 24; a row that needs you is 48, with its reason; one with
    // its file under it, 40.
    let h = if two {
        48.0
    } else if under {
        40.0
    } else {
        24.0
    };
    // Its background goes under its buttons too, drawn once they're placed.
    let bg = ui.painter().add(egui::Shape::Noop);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), if r.toggle.is_some() { Sense::click() } else { Sense::hover() });
    let toggled = resp.clicked();
    let name = [
        r.n.map(|n| n.to_string()).unwrap_or_default(),
        r.title.clone(),
        r.why.clone().unwrap_or_default(),
        r.verdict.as_ref().map(|v| format!("{} {}", v.0, v.2).trim().to_owned()).unwrap_or_default(),
        r.dest.as_ref().map(|d| format!("{}{}", d.0, d.1)).unwrap_or_default(),
    ]
    .into_iter()
    .filter(|s| !s.trim().is_empty())
    .collect::<Vec<_>>()
    .join(", ");
    match r.toggle {
        Some(open) => {
            let name = format!("{}, {}", name.trim(), if open { "open" } else { "closed" });
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &name));
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        None => resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ListItem, true, name.trim())),
    }
    // What's cut is shown whole on hovering the row, with any proof.
    let mut cut: Vec<String> = vec![];
    let pt = ui.painter().clone();
    if !first {
        pt.hline(rect.x_range(), rect.top(), Stroke::new(1.0, p.line_soft));
    }
    if r.toggle.is_some() && resp.hovered() {
        pt.rect_filled(rect.shrink2(vec2(0.0, 1.0)), 4.0, mix(p.ink, 0.03, p.surface));
    }
    // The first line's middle, and the second's.
    let cy = if two {
        rect.top() + 16.0
    } else if under {
        rect.top() + 13.0
    } else {
        rect.center().y
    };
    let cy2 = rect.top() + 32.0;
    let mut x = rect.left() + if r.attn { 8.0 } else { 0.0 };
    if let Some(n) = r.n {
        pt.text(pos2(x + 18.0, cy), egui::Align2::RIGHT_CENTER, n.to_string(), crate::theme::tnum(13.0, 400), p.subtle);
        x += 30.0;
    } else {
        x += 4.0;
    }
    let mut right = rect.right() - 6.0;
    // A file it stands for, opened from a quiet link at its end.
    let mut open_file = false;
    if r.open {
        let (lr, c) = crate::widgets::link_at(ui, p, right, cy, "Open", &format!("Open, {}", r.title), "Open full size");
        open_file = c;
        right = lr.left() - 10.0;
    }
    // A row that opens: its chevron at the end, turning as it opens.
    if let Some(open) = r.toggle {
        crate::widgets::chevron(&pt, pos2(right - 7.0, cy), 14.0, if open { 0.0 } else { -std::f32::consts::FRAC_PI_2 }, p.muted);
        right -= 22.0;
    }
    let vw = if r.verdict.is_some() { vw } else { 0.0 };
    if let Some((_, tone, note)) = &r.verdict {
        let word = verdict_word(r, w).unwrap_or("").to_owned();
        let vx = right - vw;
        let (fg, _) = tone.colours(p);
        let mut ix = vx;
        match tone {
            Tone::Ok => {
                pt.extend(kind_icon("tick", pos2(ix, cy - 7.0), 14.0, fg));
                ix += 19.0;
            }
            Tone::Bad => {
                pt.extend(kind_icon("x", pos2(ix, cy - 7.0), 14.0, fg));
                ix += 19.0;
            }
            Tone::Busy => {
                pt.extend(state_icon(p, StepState::Busy, pos2(ix + 7.0, cy), 0.8, t));
                ix += 19.0;
            }
            Tone::Warn => {
                pt.extend(state_icon(p, StepState::Wait, pos2(ix + 7.0, cy), 0.8, t));
                ix += 19.0;
            }
            Tone::Plain => {}
        }
        // The glyph in its state's colour; the word in secondary.
        let g = pt.layout_no_wrap(word.clone(), vword_font(), p.muted);
        let gw = if word.is_empty() { -6.0 } else { g.size().x };
        pt.galley(pos2(ix, cy - g.size().y / 2.0), g, p.muted);
        if !note.is_empty() {
            let nw = (vx + vw - (ix + gw + 6.0)).max(10.0);
            if measure(note, vnote_font()) > nw + 0.5 {
                cut.push(format!("{word} {note}").trim().to_owned());
            }
            let mut job = egui::text::LayoutJob::single_section(note.clone(), egui::TextFormat::simple(vnote_font(), p.subtle));
            job.wrap = egui::text::TextWrapping::truncate_at_width(nw);
            let g = pt.layout_job(job);
            // A changing number (a busy row's) sits at the column's right,
            // so its last figure stays put as it grows.
            let nx = if *tone == Tone::Busy { (vx + vw - 4.0 - g.size().x).max(ix + gw + 6.0) } else { ix + gw + 6.0 };
            pt.galley(pos2(nx, cy - g.size().y / 2.0 + 1.0), g, p.subtle);
        }
        right = vx - 12.0;
    }
    let dest_font = if under { crate::theme::tnum(12.5, 400) } else { crate::theme::tnum(13.0, 400) };
    let dest_job = |w: f32| {
        let (name, tail) = r.dest.clone().unwrap_or_default();
        let mut job = egui::text::LayoutJob::default();
        job.append(&name, 0.0, egui::TextFormat::simple(dest_font.clone(), p.muted));
        job.append(&tail, 0.0, egui::TextFormat::simple(dest_font.clone(), p.subtle));
        job.wrap = egui::text::TextWrapping::truncate_at_width(w);
        job
    };
    let full_dest = r.dest.as_ref().map(|(a, b)| format!("{a}{b}")).unwrap_or_default();
    // The size, right-aligned in its column.
    if let Some(sz) = &r.size {
        pt.text(pos2(right, cy), egui::Align2::RIGHT_CENTER, sz, crate::theme::tnum(13.0, 400), p.subtle);
        right -= r.size_w + 12.0;
    }
    // The length, as wide as it is.
    if let Some(len) = &r.len {
        pt.text(pos2(right, cy), egui::Align2::RIGHT_CENTER, len, crate::theme::tnum(13.0, 400), p.subtle);
        right -= len_need + 12.0;
    }
    // A file name beside the title when both fit (measured above), else
    // on a second line of its own, the row's whole width.
    if r.dest.is_some() && under {
        // Its whole width; a name too long for it keeps its extension.
        let w2 = rect.right() - 6.0 - x;
        let fits = measure(&full_dest, dest_font.clone()) <= w2 + 0.5;
        if !fits {
            cut.push(full_dest.clone());
            let short = keep_extension(&full_dest, w2, |t| measure(t, dest_font.clone()));
            let g = pt.layout_no_wrap(short, dest_font.clone(), p.muted);
            pt.galley(pos2(x, rect.top() + 30.0 - g.size().y / 2.0), g, p.muted);
        } else {
            let g = pt.layout_job(dest_job(w2 + 1.0));
            pt.galley(pos2(x, rect.top() + 30.0 - g.size().y / 2.0), g, p.muted);
        }
    } else if r.dest.is_some() {
        let dest_w = dest_need.min(right - x - 8.0 - 14.0 - title_need.min((right - x) * 0.5)).max(40.0);
        if dest_need > dest_w + 0.5 {
            cut.push(full_dest.clone());
        }
        let g = pt.layout_job(dest_job(dest_w));
        let gw = g.size().x;
        pt.galley(pos2(right - gw, cy - g.size().y / 2.0), g, p.muted);
        right -= gw + 14.0;
    }
    let tw = (right - x - 8.0).max(30.0);
    let ink = if r.faint { p.muted } else { p.ink };
    let mut job = egui::text::LayoutJob::single_section(r.title.clone(), egui::TextFormat::simple(tf.clone(), ink));
    job.wrap = egui::text::TextWrapping::truncate_at_width(tw);
    let g = pt.layout_job(job);
    let lift = if r.progress.is_some() && !two { 4.0 } else { 0.0 };
    pt.galley(pos2(x, cy - g.size().y / 2.0 - lift), g, ink);
    if let Some(f) = r.progress {
        let bar = Rect::from_min_size(pos2(x, cy + 8.0), vec2(tw.min(180.0), 3.0));
        pt.rect_filled(bar, 2.0, p.busy_soft);
        pt.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f.clamp(0.0, 1.0), 3.0)), 2.0, p.busy);
    }
    let cut_short = title_need > tw + 0.5;
    if cut_short {
        cut.insert(0, r.title.clone());
    }
    if let Some(why) = &r.why {
        // The whole width: the buttons are on a line of their own.
        let ww = (rect.right() - 6.0 - x).max(40.0);
        if measure(why, small.clone()) > ww + 0.5 {
            cut.push(why.clone());
        }
        let mut job = egui::text::LayoutJob::single_section(why.clone(), egui::TextFormat::simple(small, p.bad));
        job.wrap = egui::text::TextWrapping::truncate_at_width(ww);
        let g = pt.layout_job(job);
        pt.galley(pos2(x, cy2 - g.size().y / 2.0), g, p.bad);
    }
    // Shown whole on hover: anything cut, then any proof.
    let tip: Vec<String> = cut.iter().cloned().chain(r.hover.clone()).collect();
    if !tip.is_empty() {
        resp.on_hover_text(tip.join("\n"));
    }
    // Its choices, on a line of their own under its reason, lined up with
    // its title: the first filled, the others outlined, settling it without
    // a good read (Keep it anyway, Skip) quiet. What doesn't fit wraps.
    let mut clicked = None;
    let mut bottom = rect.bottom();
    if !r.acts.is_empty() {
        let indent = x - rect.left();
        let width = (w - indent - 8.0).max(80.0);
        let placed = ui
            .allocate_ui_with_layout(vec2(w, 0.0), L::left_to_right(Align::Min), |ui| {
                ui.add_space(indent);
                ui.allocate_ui_with_layout(vec2(width, 0.0), L::left_to_right(Align::Center).with_main_wrap(true), |ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    let first = r.acts.iter().position(|a| !a.2);
                    for (k, (label, id, quiet)) in r.acts.iter().enumerate() {
                        let acc = format!("{label}, {}", r.title);
                        let look = if *quiet {
                            Look::Quiet
                        } else if Some(k) == first {
                            Look::Primary
                        } else {
                            Look::Plain
                        };
                        let b = Btn::new("", label).look(look).font(font(13.0, 500)).pad(if *quiet { vec2(8.0, 5.0) } else { vec2(11.0, 5.0) }).name(&acc).show(ui, p);
                        if b.clicked() {
                            clicked = r.item.clone().map(|i| (i, id.clone()));
                        }
                    }
                })
                .response
                .rect
            })
            .inner;
        ui.add_space(10.0);
        bottom = placed.bottom() + 10.0;
    }
    if open_file {
        clicked = r.item.clone().map(|i| (i, OPEN.to_owned()));
    }
    if r.attn {
        let all = Rect::from_min_max(rect.min, pos2(rect.right(), bottom));
        ui.painter().set(bg, egui::Shape::rect_filled(all, 6.0, mix(p.bad_soft, if p.dark { 0.8 } else { 0.6 }, p.raised)));
    }
    (clicked, cut_short || !cut.is_empty(), toggled)
}

/// A file name cut to fit `w`, keeping its extension: *Sugar Never
/// Tasted….flac*.
fn keep_extension(name: &str, w: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(name) <= w {
        return name.to_owned();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(k) if k > 0 => (&name[..k], &name[k..]),
        _ => (name, ""),
    };
    let chars: Vec<char> = stem.chars().collect();
    for n in (1..chars.len()).rev() {
        let s = format!("{}…{ext}", chars[..n].iter().collect::<String>().trim_end());
        if measure(&s) <= w {
            return s;
        }
    }
    format!("…{ext}")
}

/// What a Rip note says, as a verdict: words, tone, a faint note, and why
/// it needs you. Unknown notes are shown as they are.
pub fn rip_verdict(state: ItemState, note: &str) -> (String, Tone, String, Option<String>) {
    let n = note.trim();
    match state {
        ItemState::Ok => {
            if let Some(c) = n.strip_prefix("✓ ").filter(|c| c.chars().all(|ch| ch.is_ascii_digit()) && !c.is_empty()) {
                ("Verified".into(), Tone::Ok, matches_others(c), None)
            } else if n.contains("two reads agree") || n.contains("consistent") {
                ("Reads agree".into(), Tone::Ok, "no AccurateRip match".into(), None)
            } else if let Some(reads) = best_of(n) {
                // *✓ best of 3 reads*, or *combined from 3 reads* as earlier
                // runs said; unsure samples left, it isn't clean.
                let rest = n.split_once(", ").map(|(_, r)| r.replace("unsettled", "uncertain")).unwrap_or_default();
                (format!("Best of {reads}"), if n.starts_with('✓') { Tone::Ok } else { Tone::Warn }, rest, None)
            } else if drt_steps::real::is_kept(n) {
                ("Kept anyway".into(), Tone::Warn, "not verified".into(), None)
            } else {
                ("Done".into(), Tone::Ok, n.trim_start_matches("✓").trim().to_owned(), None)
            }
        }
        // The glossary's words: only reads that disagree are damage; no
        // match after a fast read isn't evidence of it.
        ItemState::Bad => {
            let (word, why) = if n.contains("samples differ") {
                ("Damaged", reads_differ(n))
            } else if n == "no match" {
                ("Doesn't match", "Other people's rips of this disc differ".into())
            } else if n.is_empty() {
                ("Couldn't be read", "Nothing came back from the drive".into())
            } else if n == drt_steps::real::TRAY_OPENED {
                ("Stopped", n.to_owned())
            } else if n == drt_steps::real::NO_DISC {
                ("Not read", n.to_owned())
            } else {
                let mut c = n.chars();
                ("Unverified", c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default())
            };
            (word.into(), Tone::Bad, String::new(), Some(why))
        }
        ItemState::Skipped => (left_word(n).into(), Tone::Plain, String::new(), None),
        ItemState::Busy => {
            let mut c = if n.is_empty() { "reading".chars() } else { n.chars() };
            let word = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
            (word, Tone::Busy, String::new(), None)
        }
        ItemState::Waiting => ("Waiting".into(), Tone::Plain, String::new(), None),
    }
}

/// AccurateRip's confidence in words: how many other people's rips match.
fn matches_others(c: &str) -> String {
    format!("matches {c} other rip{}", if c == "1" { "" } else { "s" })
}

/// How many reads a best-of-several kept, from its note: *✓ best of 3
/// reads*, or *combined from 3 reads* as earlier runs said.
fn best_of(n: &str) -> Option<String> {
    let n = n.trim_start_matches('✓').trim();
    let rest = n.strip_prefix("best of ").or_else(|| n.strip_prefix("combined from "))?;
    rest.split_whitespace().next().filter(|d| d.chars().all(|c| c.is_ascii_digit())).map(str::to_owned)
}

/// Where two reads differ, then by how much: *439 samples differ,
/// 0:17–0:43* reads *Reads differ at 0:17–0:43 (439 samples)*.
fn reads_differ(n: &str) -> String {
    match n.split_once(" samples differ, ") {
        Some((count, at)) => format!("Reads differ at {at} ({count} samples)"),
        None => format!("Reads differ: {n}"),
    }
}

/// The step's own retries on a row that needs you: each a button, then
/// Keep it anyway and Skip as quiet words.
fn retries(r: &mut Row, info: &StepInfo) {
    let all: Vec<(String, String)> = info.retries.iter().map(|o| (o.id.to_owned(), o.label.to_owned())).collect();
    offered(r, &all);
}

/// What one item offers, `(id, label)`: each a button, then Keep and Skip
/// as quiet words.
fn offered(r: &mut Row, options: &[(String, String)]) {
    let (settle, tries): (Vec<_>, Vec<_>) = options.iter().partition(|o| matches!(o.0.as_str(), "keep" | "leave_out"));
    r.acts = tries.iter().map(|o| (o.1.clone(), o.0.clone(), false)).collect();
    r.acts.extend(settle.iter().map(|o| (o.1.clone(), o.0.clone(), true)));
}

/// Any other step's item, in plain words: done (with its note), failed
/// (and why), left out, or what it's doing. No AccurateRip words: those
/// are Rip's.
fn item_verdict(state: ItemState, note: &str) -> (String, Tone, String, Option<String>) {
    let n = note.trim().trim_start_matches('✓').trim();
    match state {
        ItemState::Ok => ("Done".into(), Tone::Ok, n.to_owned(), None),
        ItemState::Bad => ("Failed".into(), Tone::Bad, String::new(), Some(if n.is_empty() { "Something went wrong".into() } else { cap(n) })),
        ItemState::Skipped => (left_word(n).into(), Tone::Plain, String::new(), None),
        ItemState::Busy => (if n.is_empty() { "Working".into() } else { cap(n) }, Tone::Busy, String::new(), None),
        ItemState::Waiting => ("Waiting".into(), Tone::Plain, String::new(), None),
    }
}

/// The track number of an item: `track-9` is 9.
fn number(it: &ItemView, k: usize) -> u32 {
    it.id.rsplit('-').next().and_then(|n| n.parse().ok()).unwrap_or(k as u32 + 1)
}

// ---- each step's body ------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn step_body(app: &mut App, ru: &mut Ui, p: &Pal, snap: &Snapshot, id: &str, t: f64) {
    let i = app.sel;
    let Some(v) = snap.steps.get(id).cloned() else { return };
    let Some(info) = app.m.info(i, id) else { return };
    let ctx = app.m.shown_run(i).map(|r| r.run.context());
    let key = id.to_owned();
    let clicked: Vec<(String, String)> = match v.step.as_str() {
        // The drives are among its settings.
        "drive" => vec![],
        "audio-cd" => {
            let Some(c) = ctx.as_ref().filter(|c| c.parts() > 0) else { return empty(ru, p, "No disc read yet") };
            let n = c.parts();
            filter_row(app, ru, p, &[("Tracks", n as usize)], 0, &c.get(Subject::Disc, "disc_id").map(|d| format!("disc ID {d}")).unwrap_or_default());
            let list = (1..=n)
                .map(|k| {
                    // What the disc itself gave: its tracks, their lengths and
                    // ISRCs. Never a title: those are the lookups' (which may
                    // disagree), and they meet at Choose.
                    let own = |key: &str| c.found_by(id).find(|f| f.subject == Subject::Part(k) && f.key == key).map(|f| f.value.clone());
                    let mut r = Row::new(Some(k), format!("Track {k}"));
                    r.len = own("length").and_then(|l| l.parse::<u32>().ok()).map(mmss);
                    r.verdict = own("isrc").map(|isrc| (isrc, Tone::Plain, String::new()));
                    r
                })
                .collect();
            // The disc's shape: each track as long as it plays.
            let lens: Vec<f32> = (1..=n).map(|k| length(c, k).unwrap_or(0) as f32).collect();
            disc_shape(ru, p, &lens);
            rows(app, ru, p, &key, list, "", t)
        }
        "cd-text" => {
            let list: Vec<Row> = ctx.as_ref().map(|c| cdtext_rows(c, id)).unwrap_or_default().into_iter().map(|(n, title)| Row::new(Some(n), title)).collect();
            if list.is_empty() {
                return empty(ru, p, if v.state == StepState::Ok { "None on the disc" } else { "Nothing read yet" });
            }
            filter_row(app, ru, p, &[("Titles", list.len())], 0, "");
            rows(app, ru, p, &key, list, "", t)
        }
        "cover-art" => {
            cover(app, ru, p, &v, ctx.as_ref());
            vec![]
        }
        "rip" => rip(app, ru, p, id, &v, &info, ctx.as_ref(), t),
        "image" | "game-rip" => {
            let map = sector_map(&v);
            dump_rip(app, ru, p, id, &v, &info, map.as_ref(), t)
        }
        "rip-video" => video_rip(app, ru, p, id, &v, &info, ctx.as_ref(), t),
        "check" => check_fix(app, ru, p, id, &v, &info, ctx.as_ref(), t),
        "lyrics" => {
            lyrics(app, ru, p, id, &v, ctx.as_ref(), t);
            vec![]
        }
        "hold" => hold(app, ru, p, snap, &v, ctx.as_ref(), t),
        k if is_place(k) => place(app, ru, p, snap, &v, ctx.as_ref(), t),
        k if is_encoder(k) => {
            let list: Vec<Row> = v
                .items
                .iter()
                .enumerate()
                .map(|(k, it)| {
                    let mut r = Row::new(Some(number(it, k)), it.label.clone());
                    r.len = ctx.as_ref().and_then(|c| length(c, number(it, k))).map(mmss);
                    // Its proof: the file it made, under its title, and how big.
                    if let Some(f) = file_for(&v.record.files, number(it, k), &it.label) {
                        let name = file_name(&f.path);
                        if name != it.label {
                            r.dest = Some((name, String::new()));
                            r.two_line = true;
                        }
                        r.size = Some(drt_steps::real::media::size(f.size));
                    }
                    let (pct, left) = progress(&it.note);
                    if it.state == ItemState::Busy {
                        r.progress = pct;
                    }
                    r.verdict = Some(match it.state {
                        ItemState::Ok => ("Encoded".into(), Tone::Ok, String::new()),
                        // The time left as the verdict, its percentage beside.
                        ItemState::Busy => match (left, pct) {
                            (Some(l), Some(f)) => (cap(&l), Tone::Busy, format!("{}%", (f * 100.0).round())),
                            (None, Some(f)) => ("Encoding".into(), Tone::Busy, format!("{}%", (f * 100.0).round())),
                            _ => ("Encoding".into(), Tone::Busy, String::new()),
                        },
                        ItemState::Bad => ("Failed".into(), Tone::Bad, it.note.clone()),
                        ItemState::Skipped => (left_word(&it.note).into(), Tone::Plain, String::new()),
                        ItemState::Waiting => ("Waiting".into(), Tone::Plain, String::new()),
                    });
                    r.attn = it.state == ItemState::Bad;
                    r
                })
                .collect();
            if list.is_empty() {
                return empty(ru, p, "Nothing to encode yet");
            }
            let (done, busy) = (v.count(ItemState::Ok), v.count(ItemState::Busy));
            let tab = filter_row(app, ru, p, &[("Encoding", busy), ("Done", done), ("All", list.len())], if busy > 0 { 0 } else { 2 }, "");
            let list = list.into_iter().filter(|r| match tab {
                0 => r.verdict.as_ref().is_some_and(|v| v.1 == Tone::Busy),
                1 => r.verdict.as_ref().is_some_and(|v| v.1 == Tone::Ok),
                _ => true,
            });
            rows(app, ru, p, &key, list.collect(), "", t)
        }
        // A lookup's result is what it offered, below.
        _ if info.group == Group::LookUp && ctx.as_ref().is_some_and(|c| c.offered_by(id).next().is_some()) => vec![],
        _ if info.group == Group::LookUp => {
            let found: Vec<Row> = ctx
                .as_ref()
                // Only its own candidates.
                .map(|c| c.offered_by(id).map(|x| Row { dest: Some((x.detail.clone(), String::new())), ..Row::new(None, x.title.clone()) }).collect())
                .unwrap_or_default();
            if found.is_empty() {
                return empty(ru, p, if matches!(v.state, StepState::Ok | StepState::Bad) { "Nothing found" } else { "Nothing looked up yet" });
            }
            filter_row(app, ru, p, &[("Found", found.len())], 0, "");
            rows(app, ru, p, &key, found, "", t)
        }
        _ => {
            let list: Vec<Row> = v
                .items
                .iter()
                .enumerate()
                .map(|(k, it)| {
                    let (word, tone, note, why) = item_verdict(it.state, &it.note);
                    let mut r = Row { verdict: Some((word, tone, note)), attn: why.is_some(), why, item: Some(it.id.clone()), ..Row::new(Some(number(it, k)), it.label.clone()) };
                    // What the step offers for one that failed, as Rip's:
                    // the item's own choices, when it has them.
                    if r.attn {
                        if it.options.is_empty() {
                            retries(&mut r, &info);
                        } else {
                            offered(&mut r, &it.options);
                        }
                    }
                    // A picture it saved opens at full size.
                    if info.output == drt_engine::ItemKind::PictureFile && it.state == ItemState::Ok && picture_file(&v.record.files, &it.label).is_some() {
                        r.open = true;
                    }
                    r
                })
                .collect();
            if list.is_empty() {
                // Only a step that handles tracks, titles or files has a list
                // to be empty; one that handles the disc as a whole (Choose,
                // Notify, Next disc) says what it did above, or nothing.
                use drt_engine::ItemKind;
                if matches!(info.input, ItemKind::Track | ItemKind::Title | ItemKind::Dump) || info.input.is_file() {
                    empty(ru, p, "Nothing yet");
                }
                return;
            }
            filter_row(app, ru, p, &[("All", list.len())], 0, "");
            // The picture that would crowd a card: ReplayGain's spread of
            // gains.
            if v.step == "replaygain" {
                gain_spread(ru, p, &v.items);
            }
            rows(app, ru, p, &key, list, "", t)
        }
    };
    for (item, option) in clicked {
        if option == OPEN {
            let label = v.items.iter().find(|it| it.id == item).map(|it| it.label.as_str()).unwrap_or_default();
            if let Some(f) = picture_file(&v.record.files, label) {
                app.m.open(&expand(&f.path));
            }
            continue;
        }
        app.m.send(i, Command::Retry { step: id.into(), item, option });
    }
}

/// A row's *Open*, which opens its file rather than retrying anything.
const OPEN: &str = "open-file";

/// The picture file a row stands for, by its name.
fn picture_file<'a>(files: &'a [drt_engine::Made], label: &str) -> Option<&'a drt_engine::Made> {
    files.iter().find(|f| file_name(&f.path) == label && expand(&f.path).is_absolute())
}

/// Lyrics: each track and what it found; a row with lyrics opens them
/// under it, whole, synced ones with their times.
fn lyrics(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, v: &StepView, ctx: Option<&Context>, t: f64) {
    if v.items.is_empty() {
        return empty(ui, p, "Nothing looked up yet");
    }
    // Only what this step found.
    let text = |n: u32| ctx.and_then(|c| c.found_by(id).find(|f| f.subject == Subject::Part(n) && f.key == "lyrics").map(|f| f.value.clone()));
    let synced = |n: u32| ctx.is_some_and(|c| c.found_by(id).any(|f| f.subject == Subject::Part(n) && f.key == "lyrics.synced" && f.value == "yes"));
    let mut list: Vec<(Row, Option<String>, bool)> = v
        .items
        .iter()
        .enumerate()
        .map(|(k, it)| {
            let n = number(it, k);
            let body = text(n).filter(|l| !l.trim().is_empty());
            let sync = synced(n);
            let note = it.note.trim();
            let verdict = match it.state {
                ItemState::Ok if body.is_some() => (if sync { "Synced" } else { "Plain" }.to_owned(), Tone::Ok, String::new()),
                ItemState::Ok => ("Found".to_owned(), Tone::Ok, String::new()),
                ItemState::Busy => ("Asking".to_owned(), Tone::Busy, String::new()),
                ItemState::Waiting => ("Waiting".to_owned(), Tone::Plain, String::new()),
                ItemState::Bad => ("Failed".to_owned(), Tone::Bad, cap(note)),
                ItemState::Skipped => match note {
                    "no answer" => ("No answer".to_owned(), Tone::Plain, String::new()),
                    "no title to ask with" => ("Not looked up".to_owned(), Tone::Plain, "no title".to_owned()),
                    _ => ("No lyrics found".to_owned(), Tone::Plain, String::new()),
                },
            };
            let mut r = Row::new(Some(n), it.label.clone());
            r.len = ctx.and_then(|c| length(c, n)).map(mmss);
            r.verdict = Some(verdict);
            if body.is_some() {
                r.toggle = Some(app.lyrics_open.contains(&n));
            }
            (r, body, sync)
        })
        .collect();
    list.sort_by_key(|(r, _, _)| r.n);
    let w = ui.available_width();
    let vw = verdict_column(ui, p, &list.iter().map(|(r, _, _)| r.clone()).collect::<Vec<_>>(), w - if list.iter().any(|x| x.0.toggle.is_some()) { 22.0 } else { 0.0 });
    app.cut_titles.clear();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (k, (r, body, sync)) in list.iter().enumerate() {
            let (_, cut, toggled) = row(ui, p, w, vw, r, k == 0, t);
            if cut {
                app.cut_titles.push(r.title.clone());
            }
            let n = r.n.unwrap_or(0);
            if toggled && !app.lyrics_open.remove(&n) {
                app.lyrics_open.insert(n);
            }
            if let (Some(body), Some(true)) = (body, r.toggle) {
                lyrics_text(ui, p, n, body, *sync);
            }
        }
    });
}

/// A lyric's lines: each with its time (synced), the LRC tags that aren't
/// times (artist, album) left out.
fn lrc_lines(text: &str) -> Vec<(Option<String>, String)> {
    let mut out = vec![];
    for line in text.lines() {
        let mut rest = line.trim();
        let mut times = vec![];
        while let Some(tag) = rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
            let (inside, after) = tag;
            // [mm:ss.xx] is a time; [ar:Someone] is a tag.
            let (m, s) = match inside.split_once(':') {
                Some((m, s)) if m.chars().all(|c| c.is_ascii_digit()) && s.chars().next().is_some_and(|c| c.is_ascii_digit()) => (m, s),
                _ => {
                    rest = "";
                    break;
                }
            };
            let secs = s.split('.').next().unwrap_or("0");
            times.push(format!("{}:{secs:0>2}", m.parse::<u32>().unwrap_or(0)));
            rest = after.trim_start();
        }
        if rest.is_empty() && times.is_empty() {
            // A blank line in plain lyrics is a gap between verses.
            if !line.trim().is_empty() || out.is_empty() {
                continue;
            }
        }
        if times.is_empty() {
            out.push((None, rest.to_owned()));
        }
        for t in times {
            out.push((Some(t), rest.to_owned()));
        }
    }
    // Repeated times (a chorus tagged twice) in time order.
    if out.iter().all(|(t, _)| t.is_some()) {
        out.sort_by_key(|(t, _)| t.as_deref().map(|t| t.split_once(':').map(|(m, s)| m.parse::<u32>().unwrap_or(0) * 60 + s.parse::<u32>().unwrap_or(0)).unwrap_or(0)));
    }
    while out.last().is_some_and(|(_, l)| l.trim().is_empty()) {
        out.pop();
    }
    out
}

/// One track's lyrics, under its row: every line whole (wrapping), the
/// times in a quiet column; long ones scroll.
fn lyrics_text(ui: &mut Ui, p: &Pal, n: u32, body: &str, synced: bool) {
    let lines = lrc_lines(body);
    let tf = crate::theme::tnum(12.5, 400);
    let time_w = if synced { ui.painter().layout_no_wrap("00:00".into(), tf.clone(), p.subtle).size().x + 14.0 } else { 0.0 };
    ui.add_space(2.0);
    egui::Frame::new().fill(mix(p.ink, 0.025, p.surface)).corner_radius(6).inner_margin(egui::Margin { left: 30, right: 12, top: 10, bottom: 10 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt(("lyrics", n)).max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            let w = ui.available_width();
            for (time, line) in &lines {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if synced {
                        let (r, _) = ui.allocate_exact_size(vec2(time_w, 18.0), Sense::hover());
                        if let Some(t) = time {
                            ui.painter().text(pos2(r.left(), r.top() + 9.0), egui::Align2::LEFT_CENTER, t, tf.clone(), p.subtle);
                        }
                    }
                    let text = if line.trim().is_empty() { " " } else { line.as_str() };
                    ui.vertical(|ui| {
                        ui.set_width(w - time_w);
                        ui.add(egui::Label::new(RichText::new(text).font(font(13.0, 400)).color(p.ink)).wrap());
                    });
                });
            }
        });
    });
    ui.add_space(6.0);
}

/// Audio CD's picture: the disc's tracks side by side, each as wide as it
/// plays, above its list.
fn disc_shape(ui: &mut Ui, p: &Pal, lens: &[f32]) {
    let total: f32 = lens.iter().sum();
    if total <= 0.0 {
        return;
    }
    ui.add_space(4.0);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Image, true, "The disc's tracks, by length"));
    let gap = 2.0;
    let w = r.width() - gap * (lens.len() as f32 - 1.0);
    let mut x = r.left();
    for l in lens {
        let cw = (w * l / total).max(1.0);
        ui.painter().rect_filled(Rect::from_min_size(pos2(x, r.top()), vec2(cw, r.height())), 2.0, p.c_read);
        x += cw + gap;
    }
    ui.add_space(8.0);
}

/// ReplayGain's picture: each track's gain as a bar from the middle,
/// above its list.
fn gain_spread(ui: &mut Ui, p: &Pal, items: &[ItemView]) {
    let gains: Vec<f32> = items.iter().filter_map(|i| i.note.trim_end_matches(" dB").replace('\u{2212}', "-").parse::<f32>().ok()).collect();
    if gains.is_empty() {
        return;
    }
    ui.add_space(4.0);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let lo = gains.iter().copied().fold(f32::INFINITY, f32::min).min(0.0);
    let hi = gains.iter().copied().fold(f32::NEG_INFINITY, f32::max).max(0.0);
    let span = (hi - lo).max(1.0);
    let said = format!("Gains from {lo:+.1} to {hi:+.1} dB");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Image, true, &said));
    let zero = r.top() + r.height() * (hi / span);
    let gap = 2.0;
    let cw = (r.width() - gap * (gains.len() as f32 - 1.0)) / gains.len() as f32;
    for (k, g) in gains.iter().enumerate() {
        let x = r.left() + k as f32 * (cw + gap);
        let y = zero - r.height() * (g / span);
        ui.painter().rect_filled(Rect::from_two_pos(pos2(x, zero), pos2(x + cw, y)), 1.0, p.c_make);
    }
    ui.painter().hline(r.x_range(), zero, Stroke::new(1.0, p.line));
    ui.add_space(8.0);
}

/// When the right column has nothing to list.
fn empty(ui: &mut Ui, p: &Pal, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).font(font(13.5, 400)).color(p.subtle));
    ui.add_space(6.0);
}

pub fn cap_pub(s: &str) -> String {
    cap(s)
}

/// What an item left undone says: *Stopped* when the run was stopped
/// while it was being worked on, else *Skipped*.
fn left_word(note: &str) -> &'static str {
    if note.trim() == drt_engine::snapshot::STOPPED {
        "Stopped"
    } else {
        "Skipped"
    }
}

/// "fast" as "Fast", for the window; the value stays as it is.
fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn rip(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, v: &StepView, info: &StepInfo, ctx: Option<&Context>, t: f64) -> Vec<(String, String)> {
    if v.items.is_empty() {
        empty(ui, p, "Nothing read yet");
        return vec![];
    }
    let i = app.sel;
    let drive = app.m.drive(i);
    let offset = app.m.engine.settings.lock().unwrap().drive(&drive, "offset").resolve(None);
    let fact = match offset {
        Some(o) => format!("{}{}, {drive}", if o.as_i64().is_some_and(|n| n > 0) { "+" } else { "" }, value_text(&o)),
        None => drive.clone(),
    };
    rip_head(app, ui, p, id, v, info, ctx, &fact, t);
    let list: Vec<Row> = v
        .items
        .iter()
        .enumerate()
        .map(|(k, it)| {
            let n = number(it, k);
            let (word, tone, note, why) = rip_verdict(it.state, &it.note);
            let mut r = Row::new(Some(n), it.label.clone());
            r.len = ctx.and_then(|c| length(c, n)).map(mmss);
            r.attn = why.is_some();
            r.why = why;
            r.item = Some(it.id.clone());
            // Room from the start for what a read comes to (its confidence
            // and CRC), so the column doesn't move as tracks finish.
            r.widest = vec![if narrow(ui.available_width()) { (String::new(), "000".into()) } else { ("Verified".into(), "000  DDDDDDDD".into()) }];
            if r.attn {
                retries(&mut r, info);
            }
            if it.state == ItemState::Busy {
                let (pct, left) = progress(&it.note);
                r.progress = pct;
                if let Some(f) = pct {
                    // "re-reading 40%": the words, then how far.
                    let words: String = it.note.split(|c: char| c.is_ascii_digit()).next().unwrap_or("").trim().trim_end_matches(',').to_owned();
                    r.verdict = Some((cap(if words.is_empty() { "reading" } else { &words }), Tone::Busy, left.unwrap_or_else(|| format!("{}%", (f * 100.0).round()))));
                    return r;
                }
            }
            // Verified's note is AccurateRip's confidence, its column
            // headed once above the rows; more than one read says how many.
            // Each read and its CRC show on hovering the row.
            let note = note.strip_prefix("matches ").and_then(|c| c.split_whitespace().next()).filter(|c| c.chars().all(|ch| ch.is_ascii_digit())).map(str::to_owned).unwrap_or(note);
            let reads: Vec<String> = ctx.map(|c| c.all(Subject::Part(n), "read").map(|f| f.value.clone()).collect()).unwrap_or_default();
            let note = if reads.len() > 1 { format!("{note} ×{}", reads.len()).trim().to_owned() } else { note };
            // The last read's CRC, where there's room for it.
            let crc = reads.last().and_then(|r| r.rsplit_once("CRC ").map(|(_, c)| c.trim().to_owned()));
            let note = match crc {
                Some(c) if !narrow(ui.available_width()) => format!("{note}  {c}").trim().to_owned(),
                _ => note,
            };
            if !reads.is_empty() {
                r.hover = Some(reads.join("\n"));
            }
            r.verdict = Some((word, tone, note));
            r
        })
        .collect();
    let vw = verdict_column(ui, p, &list, ui.available_width());
    columns(ui, p, &[("AccurateRip", 0.0)], vw);
    let needs = v.count(ItemState::Bad);
    // Verified means AccurateRip agreed, nothing less.
    let ok = v.items.iter().filter(|it| rip_verdict(it.state, &it.note).0 == "Verified").count();
    let tab = filter_row(app, ui, p, &[("Needs you", needs), ("Verified", ok), ("All", list.len())], if needs > 0 { 0 } else { 2 }, &fact);
    let list: Vec<Row> = list
        .into_iter()
        .filter(|r| match tab {
            0 => r.attn,
            1 => r.verdict.as_ref().is_some_and(|v| v.0 == "Verified"),
            _ => true,
        })
        .collect();
    let word = if tab == 1 { "verified" } else { "" };
    let key = format!("{}/{tab}", v.step);
    rows(app, ui, p, &key, list, word, t)
}

/// One arc of a ring: how it's drawn, its share of the ring, and how far
/// a busy one has got.
struct Arc {
    cell: crate::card::Cell,
    share: f32,
    fill: f32,
}

/// What's being read now, beside a ring: a muted line (*Track 7 of 17*),
/// its title, a bar for how far, and the times and speed under it.
fn now_reading(ui: &mut Ui, p: &Pal, line: &str, title: &str, pct: Option<f32>, times: Option<String>, t: f64) {
    ui.label(RichText::new(line).font(crate::theme::tnum(13.0, 400)).color(p.muted));
    ui.add(egui::Label::new(RichText::new(title).font(font(16.0, 600)).color(p.ink)).wrap());
    ui.add_space(6.0);
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width().min(260.0), 4.0), Sense::hover());
    ui.painter().rect_filled(bar, 2.0, p.busy_soft);
    match pct {
        Some(f) => {
            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f.clamp(0.0, 1.0), 4.0)), 2.0, p.busy);
        }
        // How far isn't said: a short stretch, moving only if things may
        // move.
        None => {
            let k = if crate::motion::on() { ((t * 0.6).fract()) as f32 } else { 0.0 };
            let x = bar.left() + (bar.width() - 40.0) * k;
            ui.painter().rect_filled(Rect::from_min_size(pos2(x, bar.top()), vec2(40.0, 4.0)), 2.0, p.busy);
            crate::app::tick(ui.ctx());
        }
    }
    ui.add_space(6.0);
    if let Some(l) = times.filter(|t| !t.is_empty()) {
        ui.add(egui::Label::new(RichText::new(l).font(crate::theme::tnum(13.0, 400)).color(p.muted)).wrap());
    }
}

/// A line under a ring's headline, muted, wrapping.
fn ring_note(ui: &mut Ui, p: &Pal, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).font(font(13.0, 400)).color(p.muted)).wrap());
}

/// A ring's headline, beside it once read: *All 17 read*, *Read cleanly*;
/// its figures tabular when it's a time (*Read in 24:51 at 8.1×*).
fn ring_headline(ui: &mut Ui, p: &Pal, text: &str) {
    let f = if text.starts_with("Read in ") { crate::theme::tnum(16.0, 600) } else { font(16.0, 600) };
    ui.add(egui::Label::new(RichText::new(text).font(f).color(p.ink)).wrap());
}

/// Every Rip's head (docs/ux/rip-view.md): the disc as a ring, one arc per
/// track or title as long as it plays (one for a whole disc's image), in
/// the strip's colours, the outcome in its middle (`big` over `word`);
/// beside it, what `beside` draws: what's happening now, or once done, the
/// proof.
#[allow(clippy::too_many_arguments)]
fn ring_head(ui: &mut Ui, p: &Pal, arcs: &[Arc], ticks: Option<&[Tick]>, here: Option<Here>, big: String, word: String, beside: impl FnOnce(&mut Ui, bool)) {
    let wide = ui.available_width() >= 560.0;
    let size = if wide { 144.0 } else { 120.0 };
    let total: f32 = arcs.iter().map(|a| a.share).sum::<f32>().max(f32::EPSILON);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Image, true, format!("The disc, {big} {word}")));
        let pt = ui.painter();
        let c = r.center();
        // A map of the disc keeps room round the ring for its marks.
        let rad = size / 2.0 - if ticks.is_some() { 12.0 } else { 8.0 };
        let thick = if wide { 12.0 } else { 10.0 };
        // A gap between arcs; none for one arc, which is the whole ring.
        let gap = if arcs.len() > 40 || arcs.len() < 2 { 0.0 } else { 0.035 };
        let mut a0 = -std::f32::consts::FRAC_PI_2;
        let arc = |from: f32, to: f32, col: Color32| {
            let steps = (((to - from) / 0.05).ceil() as usize).max(2);
            let pts: Vec<egui::Pos2> = (0..=steps).map(|k| from + (to - from) * k as f32 / steps as f32).map(|a| c + vec2(a.cos(), a.sin()) * rad).collect();
            egui::Shape::line(pts, Stroke::new(thick, col))
        };
        for a in arcs {
            let a1 = a0 + a.share / total * std::f32::consts::TAU;
            let (from, to) = (a0 + gap / 2.0, (a1 - gap / 2.0).max(a0 + gap / 2.0 + 0.01));
            match a.cell {
                // Reading: its arc filling as it goes.
                crate::card::Cell::Busy => {
                    pt.add(arc(from, to, p.line));
                    let f = a.fill.clamp(0.0, 1.0);
                    pt.add(arc(from, from + (to - from) * if f > 0.0 { f } else { 0.08 }, p.busy));
                }
                crate::card::Cell::Out => {
                    pt.add(arc(from, to, p.line_soft));
                }
                cell => {
                    pt.add(arc(from, to, crate::card::cell_colour(cell, p)));
                }
            }
            a0 = a1;
        }
        // Marks where something went wrong, gathered into places round
        // the ring: where the drive reported errors as it read, as bars
        // outside it, longer where there were more; sectors found wrong,
        // across it.
        let top = -std::f32::consts::FRAC_PI_2;
        let place = std::f32::consts::TAU / PLACES as f32;
        for (k, col, strength, across) in gathered(ticks.unwrap_or(&[])) {
            let mid = top + (k as f32 + 0.5) * place;
            let dir = vec2(mid.cos(), mid.sin());
            if across {
                let w = (3.0 / rad).max(place * 0.6) / 2.0;
                let pts: Vec<egui::Pos2> = [mid - w, mid, mid + w].iter().map(|a| c + vec2(a.cos(), a.sin()) * rad).collect();
                pt.add(egui::Shape::line(pts, Stroke::new(thick + 4.0, col)));
            } else {
                let from = rad + thick / 2.0 + 1.0;
                pt.line_segment([c + dir * from, c + dir * (from + 2.0 + 3.0 * strength)], Stroke::new(2.0, col));
            }
        }
        // Where the drive is on the disc, as a dot on the ring: it breathes
        // while the drive works, and grows for a moment each new try, so a
        // quiet drive doesn't look stopped. Still, without motion.
        if let Some(h) = here {
            let a = top + h.at.clamp(0.0, 1.0) * std::f32::consts::TAU;
            let at = c + vec2(a.cos(), a.sin()) * rad;
            let (grow, glow) = if crate::motion::on() {
                let since = (ui.input(|i| i.time) - h.since).max(0.0) as f32;
                let fresh = (1.0 - since / 0.6).max(0.0);
                crate::app::tick(ui.ctx());
                (fresh * 3.0, 0.35 + 0.25 * (h.t as f32 * 3.0).sin().abs())
            } else {
                (0.0, 0.45)
            };
            pt.circle_filled(at, thick / 2.0 + 5.0 + grow, p.busy.gamma_multiply(glow));
            pt.circle_filled(at, thick / 2.0 + 1.0, p.busy);
            pt.circle_filled(at, 2.5, p.raised);
        }
        let g = pt.layout_no_wrap(big.clone(), crate::theme::tnum(if wide { 22.0 } else { 20.0 }, 600), p.ink);
        let gw = pt.layout(word.clone(), font(11.5, 500), p.muted, rad * 1.4);
        let hh = g.size().y + gw.size().y;
        pt.galley(pos2(c.x - g.size().x / 2.0, c.y - hh / 2.0), g.clone(), p.ink);
        pt.galley(pos2(c.x - gw.size().x / 2.0, c.y - hh / 2.0 + g.size().y), gw, p.muted);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.add_space(if wide { 24.0 } else { 14.0 });
            beside(ui, wide);
        });
    });
    ui.add_space(16.0);
}

/// Where the drive is on a ring's disc (0 to 1), now (`t`), and when its
/// latest try began, for the mark that shows it working.
#[derive(Clone, Copy, Debug)]
struct Here {
    at: f32,
    t: f64,
    since: f64,
}

/// A mark on a ring: from and to as shares of the disc (0 to 1), how
/// much went wrong there, its colour, and whether it lies across the ring
/// (sectors found wrong) or outside it (errors the drive reported).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tick {
    from: f32,
    to: f32,
    weight: f32,
    col: Color32,
    across: bool,
}

/// How many places round a ring marks are gathered into: 4° each, a few
/// pixels, so 200 errors make a pattern, not a smear, and one shows.
const PLACES: usize = 90;

/// Marks as drawn: each colour's gathered into [`PLACES`], each place with
/// anything in it one mark, with how much went wrong there beside the
/// worst place of that colour (0 to 1). `(place, colour, strength, across)`, the
/// later colours over the earlier.
fn gathered(ticks: &[Tick]) -> Vec<(usize, Color32, f32, bool)> {
    let mut kinds: Vec<(Color32, bool)> = vec![];
    for t in ticks {
        if !kinds.contains(&(t.col, t.across)) {
            kinds.push((t.col, t.across));
        }
    }
    let mut out = vec![];
    for (col, across) in kinds {
        let mut at = [0f32; PLACES];
        for t in ticks.iter().filter(|t| t.col == col && t.across == across) {
            let (a, b) = (t.from.clamp(0.0, 1.0), t.to.clamp(0.0, 1.0).max(t.from));
            let (pa, pb) = (((a * PLACES as f32) as usize).min(PLACES - 1), ((b * PLACES as f32 - 1e-4).max(0.0) as usize).clamp(0, PLACES - 1));
            let pb = pb.max(pa);
            let share = t.weight / (pb - pa + 1) as f32;
            for x in &mut at[pa..=pb] {
                *x += share;
            }
        }
        let most = at.iter().copied().fold(0.0, f32::max);
        for (k, w) in at.iter().enumerate().filter(|(_, w)| **w > 0.0) {
            out.push((k, col, (w / most).sqrt(), across));
        }
    }
    out
}

/// The arcs for a step's items: each as long as it plays when every
/// length is known, else all alike; a busy one filled as far as it says.
fn item_arcs(v: &StepView, info: &StepInfo, ctx: Option<&Context>) -> Vec<Arc> {
    let retries = info.retries.iter().any(|r| !matches!(r.id, "keep" | "leave_out"));
    let cells = crate::card::cells(v, crate::card::Shown::of(v, retries));
    let lens: Vec<f32> = (0..cells.len()).map(|k| v.items.get(k).map(|it| number(it, k)).and_then(|n| ctx.and_then(|c| length(c, n))).unwrap_or(0) as f32).collect();
    let known = lens.iter().all(|l| *l > 0.0);
    cells.iter().enumerate().map(|(k, cell)| Arc { cell: *cell, share: if known { lens[k] } else { 1.0 }, fill: v.items.get(k).and_then(|it| progress(&it.note).0).unwrap_or(0.0) }).collect()
}

/// Rip's head for a CD: the tracks as a ring; beside it the track being
/// read, or once done, the proof.
#[allow(clippy::too_many_arguments)]
fn rip_head(app: &App, ui: &mut Ui, p: &Pal, id: &str, v: &StepView, info: &StepInfo, ctx: Option<&Context>, offset: &str, t: f64) {
    let arcs = item_arcs(v, info, ctx);
    let busy = v.items.iter().position(|i| i.state == ItemState::Busy);
    let read = v.items.iter().filter(|i| !matches!(i.state, ItemState::Waiting | ItemState::Busy)).count();
    let verified = v.items.iter().filter(|i| i.verified).count();
    let kept = v.items.iter().filter(|i| i.state == ItemState::Ok && drt_steps::real::is_kept(&i.note)).count();
    let counted = v.items.iter().filter(|i| i.state != ItemState::Skipped).count();
    let reading = busy.is_some() || read < v.items.len();
    let (big, word) = if reading {
        (format!("{read}/{}", v.items.len()), "read".to_owned())
    } else if kept > 0 {
        (format!("{verified}/{counted}"), format!("verified · {kept} kept"))
    } else {
        (format!("{verified}/{counted}"), "verified".to_owned())
    };
    ring_head(ui, p, &arcs, None, None, big, word, |ui, _| {
        if let Some(k) = busy {
            let it = &v.items[k];
            let n = number(it, k);
            // The track's time left, the disc's, and the speed.
            let times = app.m.reading(app.sel, id).map(|r| r.times());
            now_reading(ui, p, &format!("Track {n} of {}", v.items.len()), &it.label, progress(&it.note).0, times, t);
        } else if reading {
            ring_note(ui, p, &v.detail);
        } else {
            let took = crate::model::read_in(v, |n| ctx.and_then(|c| length(c, n)).map(|l| l as f32));
            let confs: Vec<u32> = v.items.iter().filter(|i| i.verified).filter_map(|i| i.note.trim().strip_prefix("✓ ").and_then(|c| c.parse().ok())).collect();
            let conf = match (confs.iter().min(), confs.iter().max()) {
                (Some(a), Some(b)) if a == b => Some(format!("Each matches {a} other rip{} in AccurateRip", if *a == 1 { "" } else { "s" })),
                (Some(a), Some(b)) => Some(format!("Each matches {a}–{b} other rips in AccurateRip")),
                _ => None,
            };
            let needs = v.count(ItemState::Bad);
            // Tracks the tray stopped, or that found no disc, weren't read.
            let unread = v.items.iter().filter(|i| i.state == ItemState::Bad && matches!(i.note.as_str(), drt_steps::real::TRAY_OPENED | drt_steps::real::NO_DISC)).count();
            if needs > 0 {
                let head = if unread > 0 { format!("{} of {} read", v.items.len() - unread, v.items.len()) } else { format!("All {} read", v.items.len()) };
                ring_headline(ui, p, &head);
            } else if let Some(tk) = took {
                ring_headline(ui, p, &tk);
            }
            ui.add_space(4.0);
            for line in [conf, (!offset.is_empty()).then(|| format!("Offset {offset}"))].into_iter().flatten() {
                ring_note(ui, p, &line);
            }
        }
    });
}

// ---- Rip for a game or data disc: redumper's image of the whole disc ---------------------------

/// What an image came to, from its note: the headline, the ring's word,
/// and its tone. *✓ 682 MB, read cleanly*; *kept anyway, 3 unreadable
/// sectors left blank*; *read, but 2 sectors read wrong*; made by itself
/// with sectors it couldn't read, for Check and fix: *34 sectors couldn't
/// be read*.
pub fn dump_verdict(state: ItemState, note: &str) -> (String, String, Tone) {
    let n = note.trim();
    let rest = n.trim_start_matches('✓').trim();
    let after_size = rest.split_once(", ").map_or("", |(_, r)| r);
    match state {
        ItemState::Ok if n.starts_with('✓') && after_size.contains("re-read") => ("Bad parts re-read".into(), "read".into(), Tone::Ok),
        ItemState::Ok if n.starts_with('✓') => ("Read cleanly".into(), "read".into(), Tone::Ok),
        // Not clean: kept with sectors blank, or sectors that came back
        // wrong. Never a tick.
        // Made by itself with sectors it couldn't read, for Check and fix.
        ItemState::Ok if n.starts_with("made the image with") => {
            let count = n.trim_start_matches("made the image with").split_whitespace().next().and_then(|c| c.parse::<u64>().ok());
            (count.map_or_else(|| "Some sectors couldn't be read".to_owned(), |c| format!("{} couldn't be read", sectors_text(c))), "made".into(), Tone::Warn)
        }
        ItemState::Ok if n.starts_with("kept anyway") => {
            let why = n.trim_start_matches("kept anyway").trim_start_matches(", ");
            (if why.is_empty() || why.ends_with(" MB") || why.ends_with(" GB") { "Kept anyway".into() } else { format!("Kept with {why}") }, "kept".into(), Tone::Warn)
        }
        // (An older run's note says *came back wrong*.)
        ItemState::Ok if n.starts_with("read, but ") => (cap(&n.trim_start_matches("read, but ").replace("came back wrong", "read wrong")), "not clean".into(), Tone::Warn),
        ItemState::Ok => ("Read".into(), "read".into(), Tone::Ok),
        ItemState::Bad if n == drt_steps::real::TRAY_OPENED || n == drt_steps::real::NO_DISC => (cap(n), "not read".into(), Tone::Bad),
        ItemState::Bad => {
            let unreadable = n.strip_suffix(" unreadable sectors").or_else(|| n.strip_suffix(" unreadable sector")).filter(|c| c.chars().all(|ch| ch.is_ascii_digit()));
            let head = match unreadable {
                Some("1") => "1 sector couldn't be read".to_owned(),
                Some(c) => format!("{c} sectors couldn't be read"),
                None if n.is_empty() => "Couldn't be read".to_owned(),
                None => cap(n),
            };
            (head, "unreadable".into(), Tone::Bad)
        }
        ItemState::Skipped => (left_word(n).into(), left_word(n).to_lowercase(), Tone::Plain),
        ItemState::Busy | ItemState::Waiting => ("Reading".into(), "read".into(), Tone::Busy),
    }
}

/// A number with its thousands apart: 143,300.
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (k, c) in s.chars().enumerate() {
        if k > 0 && (s.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A count of sectors in words: *1 sector*, *1,310 sectors*.
fn sectors_text(n: u64) -> String {
    if n == 1 {
        "1 sector".into()
    } else {
        format!("{} sectors", thousands(n))
    }
}

/// How long something took, in words a clock would use: *41 s*, *47 min*,
/// *2 h 31 min*.
fn span(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min", s / 60),
        s => format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

/// How long a pass took, as a clock shows it: *0:41*, *38:12*, *1:52:10*.
fn clock(secs: u64) -> String {
    if secs < 3600 {
        mmss(secs)
    } else {
        format!("{}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
    }
}

/// How long a step took from start to finish: *Read in 2 h 31 min*.
fn took(v: &StepView, verb: &str) -> Option<String> {
    let (a, b) = (v.record.started?, v.record.finished?);
    b.checked_sub(a).filter(|t| *t > 0).map(|t| format!("{verb} in {}", span(t)))
}

/// Working, beside a ring: what's going on as a headline, a bar for how
/// far (moving along when that isn't known), and a line or two of how
/// it's going.
fn working(ui: &mut Ui, p: &Pal, head: &str, pct: Option<f32>, lines: &[String], t: f64) {
    ring_headline(ui, p, head);
    ui.add_space(6.0);
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width().min(260.0), 4.0), Sense::hover());
    ui.painter().rect_filled(bar, 2.0, p.busy_soft);
    match pct {
        Some(f) => {
            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f.clamp(0.0, 1.0), 4.0)), 2.0, p.busy);
        }
        None => {
            let k = if crate::motion::on() { ((t * 0.6).fract()) as f32 } else { 0.0 };
            let x = bar.left() + (bar.width() - 40.0) * k;
            ui.painter().rect_filled(Rect::from_min_size(pos2(x, bar.top()), vec2(40.0, 4.0)), 2.0, p.busy);
            crate::app::tick(ui.ctx());
        }
    }
    ui.add_space(6.0);
    for l in lines.iter().filter(|l| !l.is_empty()) {
        ui.add(egui::Label::new(RichText::new(l).font(crate::theme::tnum(13.0, 400)).color(p.muted)).wrap());
    }
}

/// What needs you, in one box under the ring: why, in a line, when
/// there's something to say; then only the buttons that can still help,
/// the first filled, settling it without a good copy (Keep it anyway,
/// Skip) quiet. Each button is named with what it's for (`name`). Gives
/// the option clicked.
fn needs_box(ui: &mut Ui, p: &Pal, reason: Option<&str>, acts: &[(String, String, bool)], name: &str) -> Option<String> {
    let mut clicked = None;
    egui::Frame::new().fill(mix(p.bad_soft, if p.dark { 0.8 } else { 0.6 }, p.raised)).corner_radius(6.0).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let Some(r) = reason.filter(|r| !r.is_empty()) {
            ui.add(egui::Label::new(RichText::new(r).font(font(13.0, 400)).color(p.ink)).wrap());
            ui.add_space(8.0);
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            let first = acts.iter().position(|a| !a.2);
            for (k, (label, id, quiet)) in acts.iter().enumerate() {
                let look = if *quiet {
                    Look::Quiet
                } else if Some(k) == first {
                    Look::Primary
                } else {
                    Look::Plain
                };
                let b = Btn::new("", label).look(look).font(font(13.0, 500)).pad(if *quiet { vec2(8.0, 5.0) } else { vec2(11.0, 5.0) }).name(&format!("{label}, {name}")).show(ui, p);
                if b.clicked() {
                    clicked = Some(id.clone());
                }
            }
        });
    });
    ui.add_space(4.0);
    clicked
}

/// How many tries redumper gives each bad sector, from its command line.
fn retries_of(log: &[&str]) -> Option<u32> {
    log.iter().find_map(|l| l.split_whitespace().find_map(|w| w.strip_prefix("--retries=")).and_then(|n| n.parse().ok()))
}

/// Rip of a game or data disc, in the CD Rip's shape: the disc as one
/// ring, filling as redumper reads it, then what it made; beside it what's
/// going on, or what it came to, in a line or two. Under it, what needs
/// you, if anything does; each pass as a row in sectors; and where on the
/// disc things went wrong, from Rip's own map. Only what Rip itself knows:
/// whether the image matches Redump, and what was put right later, are
/// Check and fix's to say.
#[allow(clippy::too_many_arguments)]
fn dump_rip(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, v: &StepView, info: &StepInfo, sectors_map: Option<&drt_steps::map::SectorMap>, t: f64) -> Vec<(String, String)> {
    use drt_steps::map::Mark;
    let Some(it) = v.items.first().cloned() else {
        empty(ui, p, "Nothing read yet");
        return vec![];
    };
    let i = app.sel;
    let log: Vec<&str> = v.record.log.iter().map(|l| l.line.as_str()).collect();
    let passes = drt_steps::real::redumper::passes(log.iter().copied());
    let raw = app.m.shown_raw(i, id);
    let live = raw.iter().rev().find_map(|l| drt_steps::real::redumper::parse_progress(l));
    let busy = it.state == ItemState::Busy;
    let (headline, word, tone) = dump_verdict(it.state, &it.note);
    let pct = progress(&it.note).0;
    let cell = match (it.state, tone) {
        (ItemState::Busy, _) => crate::card::Cell::Busy,
        (ItemState::Waiting, _) => crate::card::Cell::Wait,
        (ItemState::Skipped, _) => crate::card::Cell::Out,
        (_, Tone::Warn) => crate::card::Cell::Needs,
        (_, Tone::Bad) if !info.retries.is_empty() => crate::card::Cell::Needs,
        (_, Tone::Bad) => crate::card::Cell::Bad,
        _ if it.verified => crate::card::Cell::Verified,
        _ => crate::card::Cell::Done,
    };
    let size = (!v.status.is_empty() && v.status != "Skipped").then(|| v.status.clone());
    // What redumper is doing just now: where, which try, what's come back.
    let now_live: Option<drt_steps::real::redumper::Live> = v.now.clone().and_then(|n| serde_json::from_value(n).ok()).filter(|_| busy);
    let rereading = now_live.as_ref().filter(|l| l.rereading && l.of > 0);
    let crawling = now_live.as_ref().is_some_and(|l| l.crawling);
    // A re-read's own progress, not the disc's: in a bad patch it moves a
    // sector a minute, and the disc's share would sit still for hours.
    let pct = rereading.map(|l| l.fraction()).or(pct);
    let here = now_live.as_ref().filter(|l| l.of > 0).map(|l| {
        // When the latest try began: the sector or the try changing.
        let key = egui::Id::NULL.with(("drive-here", id));
        let now = ui.input(|i| i.time);
        let mark = (l.at, l.retry);
        let (seen, since) = ui.ctx().data(|d| d.get_temp::<((u64, u32), f64)>(key)).unwrap_or((mark, now));
        let since = if seen == mark { since } else { now };
        ui.ctx().data_mut(|d| d.insert_temp(key, (mark, since)));
        Here { at: l.position(), t, since }
    });
    let (big, small) = match it.state {
        ItemState::Skipped => (left_word(&it.note).to_owned(), String::new()),
        ItemState::Busy if rereading.is_some() => (pct.map_or("…".into(), |f| format!("{}%", (f * 100.0).floor())), "re-read".to_owned()),
        ItemState::Busy => (pct.map_or("…".into(), |f| format!("{}%", (f * 100.0).floor())), word),
        // Nothing made yet: the headline says how many it couldn't read.
        ItemState::Bad => ("—".to_owned(), "not made".to_owned()),
        _ => (size.clone().unwrap_or_else(|| cap(&word)), word),
    };
    // The disc from its start to its end round the ring, with where the
    // drive reported errors as it read, and where the test of every
    // sector found some wrong: as Rip logged them.
    let map = drt_steps::real::redumper::disc_map(log.iter().copied());
    let sectors = if map.sectors > 0 { map.sectors } else { crate::pace::sectors(&it.note).map_or(0, |s| s.1) };
    let put_right = passes.iter().any(|p| p.kind == drt_steps::real::redumper::PassKind::Reread) && drt_steps::real::redumper::left_after(&passes, passes.len().saturating_sub(1)) == 0;
    // With a map of every sector under it, the ring keeps to how far and
    // what it came to: where things went wrong is the map's to show.
    let ticks = if sectors_map.is_some() { vec![] } else { disc_ticks(p, &map, sectors, put_right && !busy) };
    // The ring in the verdict's colour; but sectors found wrong, red
    // across it, are the verdict, and stand out only on a plain ring.
    let cell = if matches!(cell, crate::card::Cell::Bad | crate::card::Cell::Needs) && !map.wrong.is_empty() && sectors_map.is_none() { crate::card::Cell::Done } else { cell };
    // Where things went wrong, as shares of the disc, only with no map to
    // say it in centimetres.
    let where_line = if sectors_map.is_some() { None } else { map_line(&map, sectors, busy) };
    let arcs = [Arc { cell, share: 1.0, fill: pct.unwrap_or(0.0) }];
    let made_anyway = it.state == ItemState::Ok && it.note.starts_with("made the image with");
    ring_head(ui, p, &arcs, Some(&ticks), here, big, small, |ui, _| {
        if let Some(l) = rereading {
            // How far through the bad sectors, which try, what's come back.
            let mut gone = match l.total {
                Some(n) => format!("{} of {} bad sectors", thousands(l.done), thousands(n)),
                None => format!("{} bad sectors gone through", thousands(l.done)),
            };
            if l.retry > 0 {
                gone.push_str(&format!(" · try {} of {}", l.retry, l.retries));
            }
            let mut back = l.recovered_text();
            if let Some(s) = l.left_secs {
                back.push_str(&format!(" · {}", drt_steps::real::redumper::left_words(s)));
            }
            working(ui, p, "Re-reading bad parts", pct, &[gone, cap(&back)], t);
        } else if busy {
            // What it's doing, how far, then the time left and speed: the
            // read slowed by errors says so as its headline.
            let doing = if crawling {
                "Slowed by read errors".to_owned()
            } else if v.detail.is_empty() {
                "Reading".to_owned()
            } else {
                cap(v.detail.split(" · ").next().unwrap_or("Reading"))
            };
            let times = app.m.reading(i, id).map(|r| r.times()).map(|s| cap(s.trim_end_matches(" · slowed by read errors"))).unwrap_or_default();
            let at = crate::pace::sectors(&it.note).map(|(at, of)| format!("Sector {} of {}", thousands(at), thousands(of))).unwrap_or_default();
            working(ui, p, &doing, pct, &[at, times], t);
        } else {
            let head = match it.state {
                ItemState::Skipped if it.note == drt_engine::snapshot::STOPPED => "Stopped".to_owned(),
                ItemState::Skipped => "No image kept".to_owned(),
                _ => headline.clone(),
            };
            ring_headline(ui, p, &head);
            ui.add_space(4.0);
            if made_anyway {
                ring_note(ui, p, "Left blank, for Check and fix to try again");
            }
            if let Some(l) = took(v, "Read") {
                ring_note(ui, p, &l);
            }
            if let Some(l) = &where_line {
                ring_note(ui, p, l);
            }
        }
        // Crawling: said once, under the rest. Stop is the window's.
        if crawling {
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(drt_steps::real::redumper::CRAWLING).font(font(13.5, 500)).color(p.warn)).wrap());
        }
    });
    let mut clicked = vec![];
    // What needs you: the sectors it couldn't read, and what can still help.
    if it.state == ItemState::Bad {
        let mut r = Row::new(None, it.label.clone());
        retries(&mut r, info);
        let reason = drt_steps::real::data::failure_detail(&it.note) != cap(&it.note);
        let why = reason.then_some("Clean the disc and re-read them, or make the image with them left blank, for Check and fix to try again.");
        if let Some(o) = needs_box(ui, p, why, &r.acts, &it.label) {
            clicked.push((it.id.clone(), o));
        }
    }
    // What it's done: each pass as a row, in order, in sectors from Rip's
    // own map; the drive's own counts are the log's.
    let count = |k: Mark| sectors_map.map(|m| m.total(k));
    let read_bad = sectors_map.map(|m| m.total(Mark::Recovered) + m.total(Mark::Unreadable) + m.total(Mark::C2));
    let left = sectors_map.map(|m| m.total(Mark::Unreadable) + m.total(Mark::C2));
    let mut list: Vec<Row> = vec![];
    let last = passes.len().saturating_sub(1);
    let later = |k: usize, kind| passes[k + 1..].iter().any(|q| q.kind == kind);
    let mut n = 0u32;
    for (k, ps) in passes.iter().enumerate() {
        use drt_steps::real::redumper::{left_after, PassKind};
        // Writing the tracks again, made anyway: the first try, stopped on
        // what it couldn't read, is one row with the second.
        if ps.kind == PassKind::Tracks && later(k, PassKind::Tracks) {
            continue;
        }
        n += 1;
        let mut r = Row::new(Some(n), ps.kind.name());
        let now = busy && k == last;
        // Its time, from its heading to the next one's (or now).
        let from = v.record.log.get(ps.at).map(|l| l.at);
        let to = passes.get(k + 1).and_then(|n| v.record.log.get(n.at)).map(|l| l.at).or(if now { None } else { v.record.finished });
        r.len = from.zip(to).and_then(|(a, b)| b.checked_sub(a)).filter(|d| *d > 0).map(clock);
        if ps.kind == PassKind::Reread {
            if let Some(n) = retries_of(&log) {
                r.dest = Some((format!("{n} tries each"), String::new()));
                r.two_line = true;
            }
        }
        r.verdict = Some(if now {
            let word = match (ps.kind, read_bad) {
                (PassKind::Read, Some(0)) => "No errors so far".to_owned(),
                (PassKind::Read, Some(n)) => format!("{} bad so far", thousands(n)),
                (PassKind::Read, None) if live.as_ref().is_some_and(|l| l.counts.iter().any(|c| c.0 != "Q" && c.1 > 0)) => "Errors so far".into(),
                (PassKind::Read, None) => "No errors so far".into(),
                (PassKind::Reread, _) => match count(Mark::Recovered).filter(|n| *n > 0) {
                    Some(n) => format!("{} put right so far", thousands(n)),
                    None => "Re-reading".into(),
                },
                _ => cap(&v.detail.split(" · ").next().unwrap_or("").to_lowercase()),
            };
            (word, Tone::Busy, String::new())
        } else {
            match (ps.kind, read_bad) {
                (PassKind::Read, Some(0)) => ("No errors".into(), Tone::Ok, String::new()),
                (PassKind::Read, Some(n)) => (format!("{} bad", thousands(n)), Tone::Plain, String::new()),
                (PassKind::Read, None) if left_after(&passes, k) == 0 => ("No errors".into(), Tone::Ok, String::new()),
                (PassKind::Read, None) => ("Errors".into(), Tone::Plain, String::new()),
                // A re-read before another: the map counts them all on the last.
                (PassKind::Reread, Some(_)) if later(k, PassKind::Reread) => (if left_after(&passes, k) == 0 { "All put right" } else { "Some left" }.into(), Tone::Plain, String::new()),
                (PassKind::Reread, Some(_)) => {
                    let back = count(Mark::Recovered).unwrap_or(0);
                    let tone = if left == Some(0) { Tone::Ok } else { Tone::Plain };
                    (if back == 0 { "None put right".into() } else { format!("{} put right", thousands(back)) }, tone, String::new())
                }
                (PassKind::Reread, None) if left_after(&passes, k) == 0 => ("All put right".into(), Tone::Ok, String::new()),
                (PassKind::Reread, None) => ("Some left".into(), Tone::Plain, String::new()),
                // Stopped on them; or, made anyway, written with them blank.
                (PassKind::Tracks, _) if ps.skipped > 0 && ps.error.is_some() => (format!("{} unreadable", thousands(ps.skipped)), Tone::Bad, String::new()),
                (PassKind::Tracks, _) if ps.skipped > 0 => (format!("{} left blank", thousands(ps.skipped)), Tone::Plain, String::new()),
                (PassKind::Tracks, _) if ps.error.is_none() => ("Done".into(), Tone::Ok, String::new()),
                (PassKind::Check, _) if ps.wrong.iter().any(|w| w.1 > 0) => (format!("{} read wrong", thousands(ps.wrong.iter().map(|w| w.1).max().unwrap_or(0))), Tone::Plain, String::new()),
                (PassKind::Check, _) if left.unwrap_or(0) > 0 || made_anyway => ("No others wrong".into(), Tone::Ok, String::new()),
                (PassKind::Check, _) => ("None read wrong".into(), Tone::Ok, String::new()),
                _ if ps.error.is_some() => ("Stopped".into(), Tone::Bad, String::new()),
                _ => ("Done".into(), Tone::Ok, String::new()),
            }
        });
        list.push(r);
    }
    if passes.is_empty() && !busy && it.state != ItemState::Bad {
        // A run from before passes were logged: the image's row alone.
        let mut r = Row::new(None, it.label.clone());
        r.verdict = Some((headline, tone, String::new()));
        list.push(r);
    }
    if !list.is_empty() {
        heading(ui, p, "What it's done");
        let key = format!("{}/dump", v.step);
        clicked.extend(rows(app, ui, p, &key, list, "", t));
    }
    // Where on the disc, as Rip read it.
    if let Some(m) = sectors_map {
        ui.add_space(8.0);
        heading(ui, p, "Where on the disc");
        if let Some(v) = crate::sectormap::show(ui, p, m, t, app.map_view, crate::sectormap::Whose::Rip) {
            app.set_map_view(v);
        }
    }
    clicked
}

/// The map of a game or data disc's sectors for Rip's sidebar: as Rip
/// read it, with the sectors its test of every sector found wrong over it
/// (from its log). Only Rip's own: what Check and fix did after is on its
/// own map. None when Rip made none (a run from before maps, a simulated
/// step).
fn sector_map(v: &StepView) -> Option<drt_steps::map::SectorMap> {
    use drt_steps::map::SectorMap;
    let log: Vec<&str> = v.record.log.iter().map(|l| l.line.as_str()).collect();
    let wrong: Vec<u64> = drt_steps::real::redumper::disc_map(log.iter().copied()).wrong.iter().flat_map(|w| w.from..=w.to).take(200_000).collect();
    let rip = v.record.map.clone().and_then(|j| serde_json::from_value::<SectorMap>(j).ok());
    drt_steps::map::combined(rip, &wrong, None)
}

/// The marks on a game or data disc's ring: where the drive reported
/// errors as it read (orange; green once re-reading put them all right),
/// then over them where the check of every sector found some wrong (red).
fn disc_ticks(p: &Pal, map: &drt_steps::real::redumper::DiscMap, sectors: u64, put_right: bool) -> Vec<Tick> {
    if sectors == 0 {
        return vec![];
    }
    let at = |s: u64| s as f32 / sectors as f32;
    let read = if put_right { p.ok } else { p.warn };
    let mark = |s: &drt_steps::real::redumper::Span, col, across| Tick { from: at(s.from), to: at(s.to + 1), weight: s.weight as f32, col, across };
    map.read.iter().map(|s| mark(s, read, false)).chain(map.wrong.iter().map(|s| mark(s, p.bad, true))).collect()
}

/// Where on the disc things went wrong, in a line: *25 bad sectors, most
/// between 49% and 68%*; or, when the check found none, where the drive
/// reported errors as it read.
fn map_line(map: &drt_steps::real::redumper::DiscMap, sectors: u64, count: bool) -> Option<String> {
    use drt_steps::real::redumper::most_between;
    if sectors == 0 {
        return None;
    }
    let pc = |s: u64| (s * 100 / sectors).min(100);
    let place = |spans| {
        let (a, b, whole) = most_between(spans)?;
        Some(match (pc(a), pc(b)) {
            (a, b) if a == b => (format!("at {a}%"), whole),
            (a, b) => (format!("between {a}% and {b}%"), whole),
        })
    };
    if map.wrong_count > 0 {
        let n = map.wrong_count;
        let head = format!("{} bad sector{}", thousands(n), if n == 1 { "" } else { "s" });
        // Where the headline already says how many, only where they are.
        return match (place(&map.wrong), count) {
            (Some((w, true)), true) => Some(format!("{head}, {w}")),
            (Some((w, false)), true) => Some(format!("{head}, most {w}")),
            (None, true) => Some(head),
            (Some((w, true)), false) => Some(cap(&format!("{w} of the way through"))),
            (Some((w, false)), false) => Some(format!("Most {w} of the way through")),
            (None, false) => None,
        };
    }
    place(&map.read).map(|(w, whole)| if whole { format!("Read errors {w}") } else { format!("Most read errors {w}") })
}

// ---- Check and fix: a game or data disc's image, against Redump ------------------------------

/// What Check and fix is busy with, from its status, as its row says it:
/// the row's name, and the word beside its spinner.
fn check_doing(status: &str) -> Option<(&'static str, &'static str)> {
    let s = status;
    if s == "Looking for bad sectors" {
        Some(("Read bad sectors again", "Looking"))
    } else if s == drt_steps::real::game::REREADING || (s.starts_with("Re-reading ") && (s.ends_with(" bad sectors") || s.ends_with(" bad sector"))) {
        Some(("Read bad sectors again", "Re-reading"))
    } else if (s.starts_with("Rip ") && s.contains(" of ")) || s.starts_with("Ripping again") {
        Some(("Rip again and combine", "Ripping"))
    } else if s.starts_with("Combining ") {
        Some(("Rip again and combine", "Combining"))
    } else if s.starts_with("Repairing") {
        Some(("Repair with error correction", "Repairing"))
    } else if s == "Re-reading bad parts" {
        Some(("Re-read bad parts", "Re-reading"))
    } else {
        None
    }
}

/// Check and fix, in the CD Rip's shape: the copy as a ring, how it
/// stands beside it in a line or two; what needs you, if anything does;
/// each thing it did, in order, from what came in from Rip; where on the
/// disc the sectors it took in as bad are, and what came of each; then
/// Redump's copy beside this one, the proof.
#[allow(clippy::too_many_arguments)]
fn check_fix(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, v: &StepView, info: &StepInfo, ctx: Option<&Context>, t: f64) -> Vec<(String, String)> {
    let Some(it) = v.items.first().cloned() else {
        empty(ui, p, "Nothing checked yet");
        return vec![];
    };
    let i = app.sel;
    let m = crate::compare::mismatch(v);
    let busy = it.state == ItemState::Busy;
    let verified = it.verified || matches!(v.status.as_str(), "Verified" | "Matches Redump");
    let held = it.state == ItemState::Bad;
    // Reading bad sectors again: where, which try, what's come back.
    let rereading: Option<drt_steps::real::fix::Rereading> = v.now.clone().and_then(|n| serde_json::from_value(n).ok()).filter(|_| busy);
    let pct = rereading.as_ref().map(|r| r.fraction()).or(progress(&it.note).0);
    let size = m.as_ref().and_then(crate::compare::copy_bytes).map(drt_steps::real::media::size);
    let stopped = it.state == ItemState::Skipped && it.note == drt_engine::snapshot::STOPPED;
    let (cell, word) = match it.state {
        ItemState::Busy => (crate::card::Cell::Busy, "checking"),
        ItemState::Waiting => (crate::card::Cell::Wait, "waiting"),
        ItemState::Skipped if stopped => (crate::card::Cell::Out, "stopped"),
        ItemState::Skipped => (crate::card::Cell::Out, "skipped"),
        ItemState::Bad => (crate::card::Cell::Needs, "not verified"),
        _ if verified => (crate::card::Cell::Verified, "verified"),
        _ => (crate::card::Cell::Done, "not verified"),
    };
    let big = match (it.state, &size) {
        (ItemState::Skipped, _) => "—".to_owned(),
        (_, Some(s)) => s.clone(),
        (ItemState::Busy, None) => pct.map_or("…".into(), |f| format!("{}%", (f * 100.0).floor())),
        (_, None) => "1/1".to_owned(),
    };
    // Reading again, how far through the bad sectors, not the copy's size.
    let (big, word) = match (&rereading, pct) {
        (Some(_), Some(f)) => (format!("{}%", (f * 100.0).floor()), "re-read"),
        _ => (big, word),
    };
    let entry = m.as_ref().map(|m| m.entry.clone()).filter(|e| !e.is_empty()).or_else(|| ctx.and_then(|c| c.get(Subject::Disc, "name")).map(str::to_owned));
    let arcs = [Arc { cell, share: 1.0, fill: pct.unwrap_or(0.0) }];
    // A mark at the ring's end that grows each new try, as Rip's does, so
    // a drive that takes a minute a try doesn't look stopped.
    let here = rereading.as_ref().map(|r| {
        let key = egui::Id::NULL.with(("check-here", id));
        let now = ui.input(|i| i.time);
        let mark = (r.at, r.attempt);
        let (seen, since) = ui.ctx().data(|d| d.get_temp::<((i64, u32), f64)>(key)).unwrap_or((mark, now));
        let since = if seen == mark { since } else { now };
        ui.ctx().data_mut(|d| d.insert_temp(key, (mark, since)));
        Here { at: r.fraction(), t, since }
    });
    ring_head(ui, p, &arcs, None, here, big, word.to_owned(), |ui, _| {
        if let Some(r) = &rereading {
            let mut lines = vec![cap(&r.place()), r.so_far(), cap(&r.left_text())];
            lines.extend(r.last_text());
            working(ui, p, drt_steps::real::game::REREADING, pct, &lines, t);
            return;
        }
        if busy {
            let head = if v.status.is_empty() { "Checking".to_owned() } else { v.status.clone() };
            let line = if v.status.is_empty() { cap(&v.detail) } else { String::new() };
            working(ui, p, &head, pct, &[line], t);
            return;
        }
        let (head, lines): (String, Vec<String>) = match v.status.as_str() {
            // Stopped part way, whatever it was at.
            _ if stopped => ("Stopped".into(), vec![]),
            _ if it.state == ItemState::Skipped => ("Skipped".into(), vec![]),
            _ if verified => ("Verified".into(), vec![entry.clone().map(|e| format!("Matches Redump's {e}")).unwrap_or_default()]),
            _ if held => (v.status.clone(), vec![m.as_ref().map(crate::compare::still_line).unwrap_or_default()]),
            "" => (cap(&it.note), vec![]),
            s => (s.to_owned(), vec![cap(&v.detail)]),
        };
        ring_headline(ui, p, &head);
        ui.add_space(4.0);
        for l in lines.iter().filter(|l| !l.is_empty()) {
            ring_note(ui, p, l);
        }
        // Reading bad sectors again stopped, getting nowhere: said plainly.
        if !verified && m.as_ref().and_then(|m| m.fixed.as_ref()).is_some_and(|f| f.given_up) {
            ring_note(ui, p, drt_steps::real::fix::STOPPED);
        }
        if let Some(l) = took(v, "Done").filter(|_| !held) {
            ring_note(ui, p, &l);
        }
    });
    let mut clicked = vec![];
    // What needs you: why, and only what can still help. Ripping again is
    // offered again until as many rips as Check's settings allow are made.
    if held {
        // (Never hidden before it has run: asked, you can always rip again once.)
        let rips = m.as_ref().and_then(|m| m.combined.as_ref()).map(|c| c.rips.len());
        let most = info.params.iter().find(|s| s.key == "extra_rips").and_then(|s| app.m.param(i, id, s).resolve(None)).and_then(|v| v.as_u64()).unwrap_or(drt_steps::real::game::EXTRA_RIPS as u64)
            as usize
            + 1;
        let options: Vec<(String, String)> = it.options.iter().filter(|o| !(o.0 == "combine" && rips.is_some_and(|r| r >= most))).cloned().collect();
        let mut r = Row::new(None, it.label.clone());
        offered(&mut r, &options);
        let reason = m.as_ref().and_then(|m| crate::compare::reason(m, &it.note));
        if let Some(o) = needs_box(ui, p, reason.as_deref(), &r.acts, &it.label) {
            clicked.push((it.id.clone(), o));
        }
    }
    // What it's done, in order: what came in from Rip, the first
    // comparison, each way it tried to put the copy right, the last.
    let mut list: Vec<Row> = vec![];
    let blank = ctx.and_then(|c| c.get(Subject::Disc, drt_steps::real::data::UNREADABLE)).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
    let mut from = Row::new(None, if blank > 0 { format!("From Rip: the image, {} blank", sectors_text(blank)) } else { "From Rip: the image".to_owned() });
    from.faint = true;
    list.push(from);
    let mut n = 0u32;
    let mut next = |title: &str| {
        n += 1;
        Row::new(Some(n), title)
    };
    let secs = |s: Option<u64>| s.filter(|s| *s > 0).map(clock);
    let first_word = match (&m, v.status.as_str()) {
        (Some(m), _) if m.in_redump() => ("Doesn't match".to_owned(), Tone::Plain),
        (Some(_), _) => ("Not in Redump".to_owned(), Tone::Plain),
        (None, _) if busy => ("Checking".to_owned(), Tone::Busy),
        (None, _) if verified => ("Matches".to_owned(), Tone::Ok),
        (None, "Couldn't check") => ("Couldn't reach Redump".to_owned(), Tone::Bad),
        (None, "Not in Redump") => ("Not in Redump".to_owned(), Tone::Plain),
        (None, _) => (cap(&v.status), Tone::Plain),
    };
    let mut r = next("Compare with Redump");
    r.verdict = Some((first_word.0, first_word.1, String::new()));
    list.push(r);
    let doing = if busy { check_doing(&v.status) } else { None };
    let mut tried = false;
    if let Some(m) = &m {
        if let Some(f) = &m.fixed {
            tried = true;
            let mut r = next("Read bad sectors again");
            let mut dest = format!("up to {} tries each", f.tries);
            if f.stalled > 0 {
                dest.push_str(&format!(" · {} skipped, the drive stalled", thousands(f.stalled)));
            }
            r.dest = Some((dest, String::new()));
            r.two_line = true;
            r.len = secs(f.secs);
            r.verdict = Some(match (f.other_disc, f.bad, f.fixed) {
                (true, _, _) => ("Nothing changed".into(), Tone::Bad, String::new()),
                (false, 0, _) => ("None bad".into(), Tone::Ok, String::new()),
                (false, _, 0) => ("None put right".into(), Tone::Plain, String::new()),
                (false, _, k) => (format!("{} put right", thousands(k)), if f.left == 0 { Tone::Ok } else { Tone::Plain }, String::new()),
            });
            list.push(r);
        }
        if let Some(c) = &m.combined {
            tried = true;
            let mut r = next("Rip again and combine");
            let later: Vec<String> = c.rips.iter().skip(1).map(|r| r.n.to_string()).collect();
            let named = match later.as_slice() {
                [] => String::new(),
                [one] => format!("rip {one}"),
                [rest @ .., last] => format!("rips {} and {last}", rest.join(", ")),
            };
            if !named.is_empty() {
                r.dest = Some((named, String::new()));
                r.two_line = true;
            }
            r.len = secs(c.secs);
            let from = if later.len() == 1 { format!("rip {}", later[0]) } else { "the other rips".to_owned() };
            r.verdict = Some(match c.taken {
                0 => ("Nothing taken".into(), Tone::Plain, String::new()),
                k => (format!("{} taken from {from}", thousands(k)), if c.still_bad == 0 { Tone::Ok } else { Tone::Plain }, String::new()),
            });
            if doing.is_some_and(|d| d.0 == "Rip again and combine") {
                r.verdict = Some((doing.map_or("", |d| d.1).to_owned(), Tone::Busy, String::new()));
            }
            list.push(r);
        }
        if let Some(rp) = &m.repaired {
            tried = true;
            let mut r = next("Repair with error correction");
            r.len = secs(rp.secs);
            r.verdict = Some(match rp.repaired {
                0 => ("Nothing repaired".into(), Tone::Plain, String::new()),
                k => (format!("{} repaired", thousands(k)), if rp.left() == 0 { Tone::Ok } else { Tone::Plain }, String::new()),
            });
            list.push(r);
        }
    }
    // What it's at now, its spinner on its row, unless that row's above.
    if let Some((title, w)) = doing {
        if !list.iter().any(|r| r.title == title && r.verdict.as_ref().is_some_and(|v| v.1 == Tone::Busy)) {
            let mut r = next(title);
            r.verdict = Some((w.to_owned(), Tone::Busy, String::new()));
            list.push(r);
        }
    } else if tried {
        // How the last comparison came out.
        let mut r = next("Compare with Redump");
        r.verdict = Some(match (&m, verified) {
            (_, true) => ("Matches".into(), Tone::Ok, String::new()),
            (Some(m), false) if m.in_redump() => ("Doesn't match".into(), Tone::Bad, String::new()),
            _ => ("Not in Redump".into(), Tone::Plain, String::new()),
        });
        list.push(r);
    }
    heading(ui, p, "What it's done");
    clicked.extend(rows(app, ui, p, &format!("{}/check", v.step), list, "", t));
    // Where on the disc: the sectors it took in as bad, and what came of each.
    if let Some(map) = v.record.map.clone().and_then(|j| serde_json::from_value::<drt_steps::map::SectorMap>(j).ok()).filter(|m| !m.is_empty()) {
        ui.add_space(8.0);
        heading(ui, p, "Where on the disc");
        if let Some(v) = crate::sectormap::show(ui, p, &map, t, app.map_view, crate::sectormap::Whose::Check) {
            app.set_map_view(v);
        }
    }
    // The proof, last.
    if let Some(m) = m.as_ref().filter(|m| m.in_redump() && !m.files.is_empty()) {
        ui.add_space(8.0);
        heading(ui, p, "Redump's copy and this one");
        crate::compare::proof(ui, p, m);
    }
    clicked
}

// ---- Rip for a video disc: MakeMKV's copy of each title ---------------------------------------

/// What a title came to, from its note: *✓ 50:12, 12.7 GB* is read and
/// that size.
fn title_verdict(state: ItemState, note: &str) -> (String, Tone, String, Option<String>) {
    let n = note.trim();
    match state {
        ItemState::Ok => ("Read".into(), Tone::Ok, n.trim_start_matches('✓').trim().rsplit_once(", ").map_or(String::new(), |(_, size)| size.to_owned()), None),
        ItemState::Bad if n == drt_steps::real::TRAY_OPENED || n == drt_steps::real::NO_DISC => ("Not read".into(), Tone::Bad, String::new(), Some(cap(n))),
        ItemState::Bad => ("Not read".into(), Tone::Bad, String::new(), Some(cap(n.trim_start_matches("Couldn't copy the video: ")))),
        ItemState::Skipped => (left_word(n).into(), Tone::Plain, String::new(), None),
        ItemState::Busy => {
            let pc = progress(n).0.map(|f| format!("{}%", (f * 100.0).floor())).unwrap_or_default();
            // Checking that the file is whole comes after reading: no
            // figure to show.
            if n.starts_with("checking") {
                ("Checking".into(), Tone::Busy, String::new(), None)
            } else {
                ("Reading".into(), Tone::Busy, pc, None)
            }
        }
        ItemState::Waiting => ("Waiting".into(), Tone::Plain, String::new(), None),
    }
}

/// Rip of a video disc: the titles as a ring, each as long as it plays;
/// beside it the one being read, its time left and the disc's; then each
/// title as a row, like a CD's tracks.
#[allow(clippy::too_many_arguments)]
fn video_rip(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, v: &StepView, info: &StepInfo, ctx: Option<&Context>, t: f64) -> Vec<(String, String)> {
    if v.items.is_empty() {
        empty(ui, p, "Nothing read yet");
        return vec![];
    }
    let i = app.sel;
    let drive = app.m.drive(i);
    let arcs = item_arcs(v, info, ctx);
    let busy = v.items.iter().position(|i| i.state == ItemState::Busy);
    let read = v.items.iter().filter(|i| i.state == ItemState::Ok).count();
    let counted = v.items.iter().filter(|i| i.state != ItemState::Skipped).count();
    let needs = v.count(ItemState::Bad);
    let (big, word) = (format!("{read}/{counted}"), "read".to_owned());
    ring_head(ui, p, &arcs, None, None, big, word, |ui, _| {
        if let Some(k) = busy {
            let it = &v.items[k];
            let at = v.items.iter().take(k + 1).filter(|i| i.state != ItemState::Skipped).count();
            let times = app.m.reading(i, id).map(|r| r.times());
            now_reading(ui, p, &format!("Video {at} of {counted}"), &it.label, progress(&it.note).0, times, t);
        } else {
            // What needs you is said above and on its row.
            let head = match (needs, read) {
                (0, 1) if counted == 1 => "Read".to_owned(),
                (0, r) if r == counted => format!("All {counted} read"),
                (_, r) => format!("{r} of {counted} read"),
            };
            ring_headline(ui, p, &head);
            ui.add_space(4.0);
            if let Some(tk) = crate::model::read_in(v, |_| None) {
                ring_note(ui, p, &tk);
            }
            if !drive.is_empty() {
                ring_note(ui, p, &drive);
            }
        }
    });
    let list: Vec<Row> = v
        .items
        .iter()
        .enumerate()
        .map(|(k, it)| {
            let n = number(it, k);
            let (word, tone, note, why) = title_verdict(it.state, &it.note);
            let mut r = Row::new(Some(n), it.label.clone());
            r.len = ctx.and_then(|c| length(c, n)).map(drt_steps::real::video::hms);
            r.attn = why.is_some();
            r.why = why;
            r.item = Some(it.id.clone());
            // Room from the start, while it reads, for what it comes to.
            if v.state == StepState::Busy {
                r.widest = vec![("Read".into(), "00.0 GB".into()), ("Reading".into(), "100%".into())];
            }
            if r.attn {
                retries(&mut r, info);
            }
            if it.state == ItemState::Busy {
                r.progress = progress(&it.note).0;
            }
            r.verdict = Some((word, tone, note));
            r
        })
        .collect();
    let vw = verdict_column(ui, p, &list, ui.available_width());
    columns(ui, p, &[("Size", 0.0)], vw);
    let key = format!("{}/titles", v.step);
    rows(app, ui, p, &key, list, "", t)
}

fn place(app: &mut App, ui: &mut Ui, p: &Pal, snap: &Snapshot, v: &StepView, ctx: Option<&Context>, t: f64) -> Vec<(String, String)> {
    let i = app.sel;
    let id = snap.steps.iter().find(|(_, x)| *x == v).map(|(k, _)| k.clone()).unwrap_or_default();
    if v.items.is_empty() {
        empty(ui, p, if folder_of(app, i, &id).is_some() { "Nothing filed yet" } else { "Choose a folder" });
        return vec![];
    }
    // A video title: where it went, as the step says.
    if files_of(app, &id) == Files::Video {
        let list: Vec<Row> = v
            .items
            .iter()
            .map(|it| {
                let (word, tone, note, why) = item_verdict(it.state, &it.note);
                Row {
                    verdict: Some((word, tone, String::new())),
                    dest: (!note.is_empty()).then(|| (note, String::new())),
                    attn: why.is_some(),
                    why,
                    item: Some(it.id.clone()),
                    ..Row::new(None, it.label.clone())
                }
            })
            .collect();
        filter_row(app, ui, p, &[("Titles", list.len())], 0, "");
        return rows(app, ui, p, &id, list, "", t);
    }
    // A disc image goes into a folder its template names, its files as
    // they were made.
    if matches!(files_of(app, &id), Files::Game | Files::Data) {
        let names = names_of(app, i, &id);
        let folder = ctx.map(dump_fields).map(|f| template::render_dump(&names, &f).display().to_string()).unwrap_or_default();
        let list: Vec<Row> = v
            .items
            .iter()
            .map(|it| {
                let mut r = Row::new(None, it.label.clone());
                r.dest = Some((format!("{folder}/"), String::new()));
                r.attn = it.state == ItemState::Bad;
                if r.attn {
                    r.why = Some(it.note.clone());
                }
                r
            })
            .collect();
        filter_row(app, ui, p, &[("Images", list.len())], 0, "");
        return rows(app, ui, p, &id, list, "", t);
    }
    // Names as the real step makes them: its path template, rendered.
    let names = names_of(app, i, &id);
    let ext = encoder_before(snap, v).map(|e| format!(".{}", ext_of(e))).unwrap_or_default();
    let path_of = |n: u32, label: &str| template::render(&names, &fields(ctx, n, label));
    // The folders, shown once above the rows (from the first track).
    let prefix: String = v
        .items
        .first()
        .map(|it| path_of(number(it, 0), &it.label))
        .and_then(|p| p.parent().map(|d| d.components().map(|c| format!("{} / ", c.as_os_str().to_string_lossy())).collect()))
        .unwrap_or_default();
    let list: Vec<Row> = v
        .items
        .iter()
        .enumerate()
        .map(|(k, it)| {
            let n = number(it, k);
            let mut r = Row::new(Some(n), it.label.clone());
            r.len = ctx.and_then(|c| length(c, n)).map(mmss);
            let file = path_of(n, &it.label).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            r.dest = Some((file, ext.clone()));
            // Where it's filed, under its title, as an encoder's file is.
            r.two_line = true;
            r.attn = it.state == ItemState::Bad;
            if r.attn {
                r.why = Some(it.note.clone());
            }
            r
        })
        .collect();
    let waiting = v.count(ItemState::Waiting) + v.count(ItemState::Busy);
    let filed = v.count(ItemState::Ok);
    let held = v.state == StepState::Hold;
    let tab = filter_row(app, ui, p, &[(if held { "Waiting" } else { "To file" }, waiting), ("Filed", filed), ("All", list.len())], if waiting > 0 { 0 } else { 2 }, &prefix);
    let states: Vec<ItemState> = v.items.iter().map(|x| x.state).collect();
    let list: Vec<Row> = list
        .into_iter()
        .zip(states)
        .filter(|(_, s)| match tab {
            0 => matches!(s, ItemState::Waiting | ItemState::Busy),
            1 => *s == ItemState::Ok,
            _ => true,
        })
        .map(|(r, _)| r)
        .collect();
    rows(app, ui, p, &format!("{id}/{tab}"), list, "", t)
}

/// What a Hold keeps: each item, and where it's waiting (the file in the
/// folder before it, when there is one).
fn hold(app: &mut App, ui: &mut Ui, p: &Pal, snap: &Snapshot, v: &StepView, ctx: Option<&Context>, t: f64) -> Vec<(String, String)> {
    let i = app.sel;
    let id = snap.steps.iter().find(|(_, x)| *x == v).map(|(k, _)| k.clone()).unwrap_or_default();
    if v.items.is_empty() {
        empty(ui, p, "Nothing held yet");
        return vec![];
    }
    let pred = v.preds.first().cloned().unwrap_or_default();
    let before = snap.steps.get(&pred).filter(|pv| is_place(&pv.step));
    let (names, ext, folder) = match before {
        Some(pv) => (Some(names_of(app, i, &pred)), encoder_before(snap, pv).map(|e| format!(".{}", ext_of(e))).unwrap_or_default(), folder_of(app, i, &pred)),
        None => (None, String::new(), None),
    };
    // Disc images wait in a folder their template names, not as tracks.
    let dumps = before.is_some() && matches!(files_of(app, &pred), Files::Game | Files::Data);
    let dump_dir = if dumps { names.as_ref().zip(ctx).map(|(t, c)| template::render_dump(t, &dump_fields(c)).display().to_string()) } else { None };
    let names = if dumps { None } else { names };
    let path_of = |n: u32, label: &str| names.as_ref().map(|t| template::render(t, &fields(ctx, n, label)));
    // Where they wait, once above the rows: the folder before, and the
    // folders its names make.
    let dirs: String = v
        .items
        .first()
        .and_then(|it| path_of(number(it, 0), &it.label))
        .and_then(|p| p.parent().map(|d| d.components().map(|c| format!("{} / ", c.as_os_str().to_string_lossy())).collect()))
        .unwrap_or_default();
    let prefix = folder.map(|f| format!("{f} / {dirs}")).unwrap_or(dirs);
    let mut list: Vec<(u32, Row, ItemState)> = v
        .items
        .iter()
        .enumerate()
        .map(|(k, it)| {
            let n = number(it, k);
            let mut r = Row::new((!dumps).then_some(n), it.label.clone());
            if !dumps {
                r.len = ctx.and_then(|c| length(c, n)).map(mmss);
            }
            if let Some(d) = &dump_dir {
                r.dest = Some((format!("{d}/"), String::new()));
            } else if let Some(f) = path_of(n, &it.label).and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned())) {
                r.dest = Some((f, ext.clone()));
            }
            (n, r, it.state)
        })
        .collect();
    // In track order, whatever order they arrived in.
    list.sort_by_key(|(n, _, _)| *n);
    let waiting = v.count(ItemState::Waiting);
    let through = v.count(ItemState::Ok);
    let tab = filter_row(app, ui, p, &[("Waiting", waiting), ("Let through", through), ("All", list.len())], if waiting > 0 { 0 } else { 2 }, &prefix);
    let list: Vec<Row> = list
        .into_iter()
        .filter(|(_, _, s)| match tab {
            0 => *s == ItemState::Waiting,
            1 => *s == ItemState::Ok,
            _ => true,
        })
        .map(|(_, r, _)| r)
        .collect();
    rows(app, ui, p, &format!("{id}/{tab}"), list, "", t)
}

/// The nearest encoder upstream of a step, for its files' extension.
fn encoder_before<'a>(snap: &'a Snapshot, v: &'a StepView) -> Option<&'a str> {
    let mut cur = v;
    for _ in 0..snap.steps.len() {
        let pred = snap.steps.get(cur.preds.first()?)?;
        if is_encoder(&pred.step) {
            return Some(pred.step.as_str());
        }
        cur = pred;
    }
    None
}

/// The extension an encoder's files get: AAC and ALAC are both `.m4a`.
fn ext_of(kind: &str) -> &'static str {
    drt_steps::real::encode::ALL.iter().find(|c| c.kind() == kind).map_or("", |c| c.ext())
}

/// A folder step's path template: a preset's name or one typed.
fn names_of(app: &App, i: usize, id: &str) -> String {
    app.m
        .info(i, id)
        .and_then(|inf| inf.params.iter().find(|s| s.key == "names").cloned())
        .and_then(|s| app.m.shown_value(i, id, &s))
        .map(|v| value_text(&v))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| template::DEFAULT.to_owned())
}

/// What a template can refer to for track `n`, from the run's context, as
/// the real step builds it (`drt_steps::real::fields`).
fn fields(ctx: Option<&Context>, n: u32, fallback_title: &str) -> template::Fields {
    let Some(c) = ctx else { return template::Fields { title: fallback_title.to_owned(), track: n, ..Default::default() } };
    let get = |s, k| c.get(s, k).unwrap_or("").to_owned();
    let albumartist = c.get(Subject::Disc, "album_artist").unwrap_or("Unknown artist").to_owned();
    template::Fields {
        artist: c.get(Subject::Part(n), "artist").map(str::to_owned).unwrap_or_else(|| albumartist.clone()),
        albumartist,
        album: c.get(Subject::Disc, "album").unwrap_or("Unknown album").to_owned(),
        title: c.get(Subject::Part(n), "title").map(str::to_owned).unwrap_or_else(|| fallback_title.to_owned()),
        track: n,
        tracktotal: c.parts(),
        year: get(Subject::Disc, "year"),
        disc: get(Subject::Disc, "disc"),
        label: get(Subject::Disc, "label"),
    }
}

/// What a Folder's names are for: what reaches it, as its step says
/// ([`template::Files::of`]); tracks where that could be anything.
fn files_of(app: &App, id: &str) -> Files {
    app.m.shown_flow(app.sel, id).as_ref().and_then(Files::of).unwrap_or(Files::Music)
}

/// A Folder's names: its presets and tokens, the preset it's on (or
/// Custom), and the template as the step reads it.
struct Names {
    files: Files,
    presets: &'static [(&'static str, &'static str)],
    tokens: &'static [(&'static str, &'static str)],
    preset: Option<&'static str>,
    tpl: String,
}

/// `value`: the names in effect, as [`Model::value`] reads them, so a
/// Folder still on another kind's old default shows its own.
fn names(app: &App, id: &str, spec: &ParamSpec, value: &str) -> Names {
    let files = files_of(app, id);
    let fitted = spec.fitted(app.m.shown_flow(app.sel, id).as_ref()).unwrap_or_else(|| files.fitted());
    let (presets, tokens) = (fitted.presets, fitted.tokens);
    let stored = template::renamed(value);
    let tpl = if files == Files::Video {
        // "Jellyfin" is a layout of its own: a film's or an episode's, by
        // what the title is. Shown as the film's.
        let raw = presets.iter().find(|(l, _)| *l == stored).map_or(stored, |(_, t)| *t);
        template::video_template(raw, false).to_owned()
    } else {
        template::resolve(stored).to_owned()
    };
    let preset = presets.iter().find(|(l, t)| *l == stored || *t == stored || (files != Files::Video && *t == tpl)).map(|(l, _)| *l);
    Names { files, presets, tokens, preset, tpl }
}

/// A sample film, and a sample episode, for a video template's example.
fn video_samples() -> (template::VideoFields, template::VideoFields) {
    let film = template::VideoFields { title: "Paprika".into(), year: "2006".into(), tmdbid: "4977".into(), ..Default::default() };
    let ep = template::VideoFields {
        title: "Twin Peaks".into(),
        year: "1990".into(),
        tmdbid: "1920".into(),
        season: Some(1),
        episode: Some(3),
        episode_title: "Zen, or the Skill to Catch a Killer".into(),
        ..Default::default()
    };
    (film, ep)
}

/// What a dump's template can use, from the run's context, as the real
/// step builds it (`drt_steps::real::data::dump_fields`).
fn dump_fields(c: &Context) -> template::DumpFields {
    let get = |k| c.get(Subject::Disc, k).unwrap_or("").to_owned();
    let label = get("label");
    let name = c.get(Subject::Disc, "name").map(str::to_owned).filter(|n| !n.is_empty()).unwrap_or_else(|| if label.is_empty() { "Disc".into() } else { label.clone() });
    let bytes: u64 = get("bytes").parse().unwrap_or(0);
    template::DumpFields { name, label, system: get("system"), date: template::today(), size: if bytes > 0 { drt_steps::real::media::size(bytes) } else { String::new() } }
}

/// A disc image to show an example folder with when there's none yet: a
/// game's for a game.
fn dump_sample(files: Files) -> template::DumpFields {
    if files == Files::Game {
        return template::DumpFields { name: "Colony Wars (Europe) (Disc 1)".into(), label: "SLES_008.60".into(), system: "PlayStation".into(), date: template::today(), size: "680 MB".into() };
    }
    template::DumpFields { name: "Encarta 98 Encyclopedia (USA) (Disc 1)".into(), label: "ENCARTA98".into(), system: "IBM PC".into(), date: template::today(), size: "650 MB".into() }
}

/// A track to show an example path with when there's no disc yet.
fn sample() -> template::Fields {
    template::Fields {
        albumartist: "The Cinematic Orchestra".into(),
        album: "Every Day".into(),
        artist: "The Cinematic Orchestra".into(),
        title: "All That You Give".into(),
        track: 1,
        tracktotal: 7,
        year: "2002".into(),
        disc: "1".into(),
        label: "Ninja Tune".into(),
    }
}

/// Progress in a note: "encoding 43%, 12s left" is 43% and "12s left".
pub fn progress(note: &str) -> (Option<f32>, Option<String>) {
    let pct = crate::pace::percent(note);
    let left = note.split(", ").find(|part| part.ends_with(" left")).map(|l| l.trim().to_owned());
    (pct, left)
}

/// The overall percentage in an encoder's detail: "2 encoding, 61%".
fn percent(detail: &str) -> Option<&str> {
    detail.split(", ").find(|p| p.ends_with('%'))
}

/// Which drive the recipe reads from: a dropdown when there's a choice
/// (more than one drive known, or none chosen yet); with one drive, its
/// name as text.
fn drive_choice(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, open: bool, label_id: egui::Id) {
    let i = app.sel;
    let current = app.m.drive(i);
    // Every drive known, and the one the recipe reads even when it isn't
    // plugged in, so it can always be changed.
    let mut list = app.m.drives.clone();
    if !current.is_empty() && !list.contains(&current) {
        list.push(current.clone());
    }
    if list.is_empty() && current.is_empty() {
        return empty(ui, p, "No drives found");
    }
    // What a drive is doing, after its name: busy, not connected.
    let state = |app: &App, d: &str| -> Option<&'static str> {
        if !app.m.is_connected(d) {
            Some("Not connected")
        } else if app.m.drive_user(d).is_some_and(|j| j != i) {
            Some("Busy")
        } else {
            None
        }
    };
    let words = |app: &App, d: &str| match state(app, d) {
        Some(s) => format!("{d} · {s}"),
        None => d.to_owned(),
    };
    if !open {
        let mut job = egui::text::LayoutJob::default();
        job.append(&current, 0.0, egui::TextFormat::simple(font(13.5, 400), p.ink));
        if let Some(s) = state(app, &current) {
            job.append(&format!(" · {s}"), 0.0, egui::TextFormat::simple(font(13.5, 400), p.subtle));
        }
        ui.add_space(5.0);
        let r = ui.add(egui::Label::new(job).wrap());
        let _ = r.labelled_by(label_id);
        ui.add_space(5.0);
        return;
    }
    let shown = if current.is_empty() { "Choose a drive".to_owned() } else { words(app, &current) };
    let mut pick = None;
    let r = crate::widgets::combo(("drive", id), p)
        .selected_text(shown)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for d in &list {
                if ui.selectable_label(*d == current, words(app, d)).clicked() {
                    pick = Some(d.clone());
                }
            }
        })
        .response;
    let _ = r.labelled_by(label_id);
    if let Some(d) = pick.filter(|d| *d != current) {
        if let Err(e) = app.m.set_param(i, id, "drive", d.as_str().into()) {
            app.refuse(ui.ctx(), e.to_string());
        }
    }
}

/// The cover, large, with its size and source.
fn cover(app: &mut App, ui: &mut Ui, p: &Pal, v: &StepView, ctx: Option<&Context>) {
    let fact = ctx.and_then(|c| c.all(Subject::Disc, "cover").last().map(|f| (f.value.clone(), f.source.clone())));
    let Some((path, source)) = fact else {
        return empty(ui, p, if v.state == StepState::Ok { "No cover found" } else { "Nothing looked up yet" });
    };
    let path = expand(&path);
    load_cover(app, ui.ctx(), &path);
    let side = room(ui).min(220.0);
    let state = &app.covers[&path];
    let mut open = false;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        match state {
            Cover::Ready(tex, [w, h]) => {
                let (w, h) = (*w as f32, *h as f32);
                let s = side / w.max(h);
                // A click opens it at full size, in the system's viewer.
                let resp = ui.add(egui::Image::new((tex.id(), vec2(w * s, h * s))).corner_radius(6).alt_text("The cover").sense(Sense::click()));
                open = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Open full size").clicked();
            }
            Cover::Loading => {
                let (r, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                ui.painter().rect_filled(r, 6.0, p.surface);
            }
            Cover::Failed => {
                let (r, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                ui.painter().rect_filled(r, 6.0, p.surface);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Can't show this image", font(12.5, 400), p.subtle);
            }
        }
    });
    if matches!(state, Cover::Ready(..)) {
        ui.add_space(6.0);
        let label = "Open full size";
        let (r, _) = ui.allocate_exact_size(vec2(crate::widgets::link_width(ui, label), 20.0), Sense::hover());
        open |= crate::widgets::link_at(ui, p, r.right(), r.center().y, label, label, &path.display().to_string()).1;
    }
    if open {
        app.m.open(&path);
    }
    // Where it came from, when that isn't the chosen release.
    if let Some(from) = ctx.and_then(|c| c.get(Subject::Disc, "cover_from")) {
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(format!("From another edition: {from}")).font(font(13.0, 400)).color(p.muted)).wrap());
    }
    // The images on the archive's page for the release, when it came from
    // there.
    if let Some(url) = ctx.filter(|_| source == "Cover Art Archive").and_then(crate::links::cover) {
        ui.add_space(6.0);
        let label = "Cover Art Archive ↗";
        let (r, _) = ui.allocate_exact_size(vec2(crate::widgets::link_width(ui, label), 20.0), Sense::hover());
        if crate::widgets::link_at(ui, p, r.right(), r.center().y, label, "Open on Cover Art Archive", &url).1 {
            app.m.open_url(&url);
        }
    }
}

// ---- a question's own answers -----------------------------------------------------------------

/// Each answer as a button named for what it does, with what that means
/// beside it: the one it would pick filled, one that loses what's there red.
fn answers(app: &mut App, ui: &mut Ui, p: &Pal, snap: &Snapshot, step: &str) {
    let Some((_, q)) = &snap.question else { return };
    let QuestionShape::PickOne { options, preselected, .. } = &q.shape else { return };
    let w = ui.available_width();
    for (k, o) in options.iter().enumerate() {
        let clicked = ui
            .horizontal(|ui| {
                ui.set_min_height(36.0);
                ui.spacing_mut().item_spacing.x = 12.0;
                // The buttons line up, the widest setting the column.
                let bw = options.iter().map(|o| ui.painter().layout_no_wrap(o.label.clone(), font(14.0, 500), p.ink).size().x).fold(0.0, f32::max) + 28.0;
                let x0 = ui.cursor().left();
                let clicked = match (o.destructive, k == *preselected) {
                    (true, _) => danger_button(ui, p, &o.label),
                    (false, true) => Btn::new("", &o.label).look(Look::Primary).show(ui, p).clicked(),
                    (false, false) => Btn::new("", &o.label).look(Look::Plain).show(ui, p).clicked(),
                };
                ui.add_space((x0 + bw - ui.cursor().left()).max(0.0));
                if !o.detail.is_empty() {
                    ui.allocate_ui_with_layout(vec2((w - bw - 12.0).max(80.0), 32.0), L::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(&o.detail).font(font(13.0, 400)).color(p.muted)).wrap());
                    });
                }
                clicked
            })
            .inner;
        if clicked {
            app.m.send(app.sel, Command::Answer { step: step.to_owned(), answer: Answer::Pick(k) });
        }
        ui.add_space(6.0);
    }
}

/// Answers that are pictures: each shown as it is, side by side, with
/// what it is and its size under it. One is picked to start with; a click
/// picks another, and *Use this cover* answers.
fn pictures(app: &mut App, ui: &mut Ui, p: &Pal, snap: &Snapshot, step: &str) {
    let Some((_, q)) = &snap.question else { return };
    let QuestionShape::PickOne { options, preselected, .. } = &q.shape else { return };
    // A pick left from another question doesn't carry over.
    if app.pick.is_some_and(|k| k >= options.len()) {
        app.pick = None;
    }
    let pick = *app.pick.get_or_insert(*preselected);
    let w = ui.available_width();
    // As many across as fit, at least two, never wider than 200.
    let gap = 12.0;
    let across = ((w + gap) / (150.0 + gap)).floor().max(2.0);
    let side = ((w - gap * (across - 1.0)) / across).min(200.0);
    let per_row = across as usize;
    for row in options.chunks(per_row).enumerate() {
        let (r, chunk) = row;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (j, o) in chunk.iter().enumerate() {
                let k = r * per_row + j;
                ui.vertical(|ui| {
                    ui.set_width(side);
                    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, k == pick, format!("{}, {}", o.label, o.detail)));
                    ui.painter().rect_filled(rect, 6.0, p.surface);
                    if let Some(path) = &o.picture {
                        let path = expand(path);
                        load_cover(app, ui.ctx(), &path);
                        match &app.covers[&path] {
                            Cover::Ready(tex, [iw, ih]) => {
                                let (iw, ih) = (*iw as f32, *ih as f32);
                                let s = side / iw.max(ih);
                                let img = Rect::from_center_size(rect.center(), vec2(iw * s, ih * s));
                                egui::Image::new((tex.id(), img.size())).corner_radius(6).paint_at(ui, img);
                            }
                            Cover::Loading => {}
                            Cover::Failed => {
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Can't show this image", font(12.5, 400), p.subtle);
                            }
                        }
                    }
                    if k == pick {
                        ui.painter().rect_stroke(rect.expand(3.0), 8.0, Stroke::new(2.5, p.action), egui::StrokeKind::Outside);
                    } else if resp.hovered() {
                        ui.painter().rect_stroke(rect.expand(2.0), 7.0, Stroke::new(1.0, p.line), egui::StrokeKind::Outside);
                    }
                    if resp.clicked() {
                        app.pick = Some(k);
                    }
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(&o.label).font(font(13.0, if k == pick { 600 } else { 500 })).color(p.ink)).wrap());
                    ui.label(RichText::new(&o.detail).font(font(12.5, 400)).color(p.muted));
                    // Seeing one at full size doesn't pick it.
                    if let Some(path) = o.picture.as_deref().map(expand).filter(|f| f.is_absolute()) {
                        let label = "Open full size";
                        let (r, _) = ui.allocate_exact_size(vec2(crate::widgets::link_width(ui, label), 20.0), Sense::hover());
                        if crate::widgets::link_at(ui, p, r.right(), r.center().y, label, &format!("Open full size: {}", o.label), &path.display().to_string()).1 {
                            app.m.open(&path);
                        }
                    }
                });
            }
        });
        ui.add_space(14.0);
    }
    if Btn::new("", "Use this cover").look(Look::Primary).show(ui, p).clicked() {
        app.m.send(app.sel, Command::Answer { step: step.to_owned(), answer: Answer::Pick(pick) });
        app.pick = None;
    }
}

// ---- Choose ----------------------------------------------------------------------------------

fn choose(app: &mut App, ru: &mut Ui, p: &Pal, snap: &Snapshot, step: &str, t: f64) {
    let i = app.sel;
    let Some(v) = snap.steps.get(step).cloned() else { return };
    let ctx = app.m.shown_run(i).map(|r| r.run.context());
    // What reached it, in the order it asks about them.
    let cands = options_of(&v, ctx.as_ref());
    let asking = matches!(&snap.question, Some((s, q)) if s == step && matches!(q.shape, QuestionShape::PickOne { .. }));
    let chosen = v.record.decided.as_ref().and_then(|d| d.chosen.clone());
    let pick = if asking {
        let pre = match &snap.question {
            Some((_, q)) => match q.shape {
                QuestionShape::PickOne { preselected, .. } => preselected,
                _ => 0,
            },
            None => 0,
        };
        *app.pick.get_or_insert(pre)
    } else {
        chosen.as_ref().and_then(|c| cands.iter().position(|x| &x.id == c)).unwrap_or(0)
    };
    let details = if asking { details_of(app) } else { None };
    if cands.is_empty() {
        // Nothing found: the form at once.
        if let Some(d) = &details {
            return details_form(app, ru, p, d);
        }
        return empty(ru, p, if v.state == StepState::Ok { "Nothing to choose from" } else { "Waits for the lookups" });
    }
    ru.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        let cw = (ui.available_width() * 0.42).clamp(220.0, 320.0);
        ui.vertical(|ui| {
            ui.set_width(cw);
            egui::ScrollArea::vertical().id_salt("cands").max_height(room(ui)).show(ui, |ui| {
                for (n, c) in cands.iter().enumerate() {
                    let link = crate::links::entry(c);
                    let (picked, open) = card(ui, p, n == pick, &c.title, &c.detail, ("cand", n), asking, link.as_ref().map(|(s, u)| (*s, u.as_str())));
                    if picked && asking {
                        app.pick = Some(n);
                    }
                    if let (true, Some((_, url))) = (open, &link) {
                        app.m.open_url(url);
                    }
                    ui.add_space(8.0);
                }
                // None of them: type it yourself.
                if details.is_some() {
                    let n = cands.len();
                    let what = if details.as_ref().is_some_and(|d| d.contains_key("album")) { "type the album and tracks" } else { "type what it is" };
                    if card(ui, p, pick == n, "Enter details", what, ("cand", n), true, None).0 {
                        app.pick = Some(n);
                    }
                }
            });
        });
        // The details form, if that's the choice.
        if let (Some(d), true) = (&details, pick == cands.len()) {
            ui.vertical(|ui| details_form(app, ui, p, d));
        }
        // The picked one's tracks, against the disc (a release has them;
        // a game or a film doesn't).
        let has_tracks = |c: &drt_engine::Candidate| c.facts.iter().any(|f| f.key == "title" && matches!(f.subject, Subject::Part(_)));
        if let Some(c) = cands.get(pick).filter(|c| has_tracks(c)) {
            ui.vertical(|ui| {
                let year = c.facts.iter().find(|f| f.key == "year").map(|f| f.value.clone()).unwrap_or_default();
                ui.horizontal(|ui| {
                    let head = if year.is_empty() { "Tracks, against the disc".to_owned() } else { format!("Tracks, {year} edition, against the disc") };
                    ui.label(RichText::new(head).font(font(12.5, 400)).color(p.muted));
                });
                ui.add_space(4.0);
                // Each track against the disc: on it, and as long, if the
                // release says how long.
                let parts = ctx.as_ref().map_or(0, |x| x.parts());
                let fact = |n: u32, key: &str| c.facts.iter().find(|f| f.key == key && f.subject == Subject::Part(n)).map(|f| f.value.clone());
                let mut nums: Vec<u32> = c
                    .facts
                    .iter()
                    .filter(|f| f.key == "title")
                    .filter_map(|f| match f.subject {
                        Subject::Part(n) => Some(n),
                        Subject::Disc => None,
                    })
                    .chain(1..=parts)
                    .collect();
                nums.sort();
                nums.dedup();
                let rows_: Vec<(u32, String, String, Result<(), String>)> = nums
                    .into_iter()
                    .map(|n| {
                        let disc_len = ctx.as_ref().and_then(|x| length(x, n));
                        let their_len = fact(n, "length").and_then(|l| l.parse::<f32>().ok()).map(|l| l.round() as i64);
                        let fit = match (fact(n, "title"), disc_len, their_len) {
                            (None, _, _) => Err("not on this edition".to_owned()),
                            (_, None, _) if parts > 0 => Err("not on the disc".to_owned()),
                            (_, Some(d), Some(t)) if (t - d as i64).abs() > 2 => Err(format!("{:+}s", t - d as i64)),
                            _ => Ok(()),
                        };
                        (n, fact(n, "title").unwrap_or_else(|| "—".into()), disc_len.map(mmss).unwrap_or_default(), fit)
                    })
                    .collect();
                let mut row = |ui: &mut Ui, k: usize, w: f32| {
                    let (n, title, len, fit) = &rows_[k];
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
                    let says = match fit {
                        Ok(()) => "fits".to_owned(),
                        Err(e) => e.clone(),
                    };
                    let name = format!("{n}, {title}, {len}, {says}");
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ListItem, true, &name));
                    let pt = ui.painter();
                    pt.text(pos2(rect.left() + 18.0, rect.center().y), egui::Align2::RIGHT_CENTER, n.to_string(), font(12.5, 400), p.subtle);
                    // The check at the right; the length before it.
                    let (mark, mark_col) = match fit {
                        Ok(()) => ("✓".to_owned(), p.ok),
                        Err(e) => (e.clone(), p.warn),
                    };
                    let mg = pt.layout_no_wrap(mark, font(12.5, 500), mark_col);
                    let mw = mg.size().x;
                    pt.galley(pos2(rect.right() - mw, rect.center().y - mg.size().y / 2.0), mg, mark_col);
                    pt.text(pos2(rect.right() - mw - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, len, font(13.0, 400), p.subtle);
                    let mut job = egui::text::LayoutJob::single_section(title.clone(), egui::TextFormat::simple(font(13.0, 400), p.ink));
                    job.wrap = egui::text::TextWrapping::truncate_at_width(w - 26.0 - 52.0 - mw);
                    let g = pt.layout_job(job);
                    pt.galley(pos2(rect.left() + 26.0, rect.center().y - g.size().y / 2.0), g, p.ink);
                    pt.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.line_soft));
                };
                // Two columns only when each still has room for a title.
                let wide = ui.available_width() >= 2.0 * 300.0 + 22.0;
                egui::ScrollArea::vertical().id_salt("tracks").max_height(room(ui)).show(ui, |ui| {
                    if wide {
                        two_columns(ui, p, rows_.len(), &mut row);
                    } else {
                        let w = ui.available_width();
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for k in 0..rows_.len() {
                            row(ui, k, w);
                        }
                    }
                });
            });
        }
    });
    let _ = t;
}

/// A choice for each item (a DVD's titles: which episode, or skip), the
/// likeliest already picked. Ones it couldn't place (to be skipped unless
/// you say) come first.
fn per_item(app: &mut App, ui: &mut Ui, p: &Pal, snap: &Snapshot) {
    let Some((_, q)) = &snap.question else { return };
    let QuestionShape::PerItem { items, options } = &q.shape else { return };
    let mut order: Vec<&drt_engine::ItemChoice> = items.iter().collect();
    order.sort_by_key(|it| it.preselected.is_some());
    let w = ui.available_width();
    let unplaced = items.iter().filter(|it| it.preselected.is_none()).count();
    let placed = items.len() - unplaced;
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.label(RichText::new(format!("{placed} placed, {unplaced} skipped unless you pick")).font(font(12.5, 400)).color(p.subtle));
    });
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("per-item").max_height(room(ui)).show(ui, |ui| {
        ui.set_width(w - 12.0);
        let cw = ((w - 12.0) * 0.42).clamp(180.0, 300.0);
        for it in order {
            let cur = *app.per_item.get(&it.id).unwrap_or(&it.preselected);
            let (row, resp) = ui.allocate_exact_size(vec2(w - 12.0, 44.0), Sense::hover());
            let name = format!("{}, {}", it.label, it.detail);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ListItem, true, &name));
            let pt = ui.painter();
            if it.preselected.is_none() && !app.per_item.contains_key(&it.id) {
                pt.rect_filled(row, 0.0, mix(p.warn_soft, 0.5, p.raised));
            }
            pt.text(pos2(row.left() + 8.0, row.top() + 13.0), egui::Align2::LEFT_CENTER, &it.label, font(13.5, 500), p.ink);
            let mut job = egui::text::LayoutJob::single_section(it.detail.clone(), egui::TextFormat::simple(font(12.0, 400), p.muted));
            job.wrap = egui::text::TextWrapping::truncate_at_width(row.width() - cw - 28.0);
            let g = pt.layout_job(job);
            pt.galley(pos2(row.left() + 8.0, row.top() + 22.0), g, p.muted);
            pt.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, p.line_soft));
            // The choice, at the right.
            let shown = cur.and_then(|k| options.get(k)).map_or_else(|| "Skip".to_owned(), |o| o.label.clone());
            let cr = Rect::from_min_size(pos2(row.right() - cw - 6.0, row.center().y - 14.0), vec2(cw, 28.0));
            let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(cr).layout(L::left_to_right(Align::Center)));
            let r = crate::widgets::combo(("per-item", &it.id), p).selected_text(shown).width(cw).show_ui(&mut cu, |ui| {
                for (k, o) in options.iter().enumerate() {
                    let text = if o.detail.is_empty() { o.label.clone() } else { format!("{}  {}", o.label, o.detail) };
                    if ui.selectable_label(cur == Some(k), text).clicked() {
                        app.per_item.insert(it.id.clone(), Some(k));
                    }
                }
                if ui.selectable_label(cur.is_none(), "Skip").clicked() {
                    app.per_item.insert(it.id.clone(), None);
                }
            });
            let name = format!("{}: choice", it.label);
            r.response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, &name));
        }
    });
}

/// A details key as a field's label: "Album artist", "Name"; any other
/// key with its first letter capitalised.
fn detail_label(key: &str) -> String {
    match key {
        "album" => "Album".into(),
        "album_artist" => "Album artist".into(),
        "year" => "Year".into(),
        "name" => "Name".into(),
        "title" => "Title".into(),
        "kind" => "Film or series".into(),
        "season" => "Season".into(),
        "tmdbid" => "TMDb id".into(),
        k => cap(&k.replace('_', " ")),
    }
}

/// The details the question offers to type, pre-filled, if it does.
fn details_of(app: &App) -> Option<std::collections::BTreeMap<String, String>> {
    let snap = app.m.snapshot(app.sel);
    match snap.question.map(|(_, q)| q.shape) {
        Some(QuestionShape::PickOne { details, .. }) => details,
        _ => None,
    }
}

/// How many candidates the question offers.
fn candidates(app: &App) -> usize {
    match app.m.snapshot(app.sel).question.map(|(_, q)| q.shape) {
        Some(QuestionShape::PickOne { options, .. }) => options.len(),
        _ => 0,
    }
}

/// The details to type: album, album artist and year, then each track's
/// title and artist. Starts from what the question offers; what you type
/// is kept until the answer goes.
fn details_form(app: &mut App, ui: &mut Ui, p: &Pal, start: &std::collections::BTreeMap<String, String>) {
    let field = |app: &mut App, ui: &mut Ui, key: &str, name: &str, w: f32| {
        let buf = app.buffers.entry(format!("details/{key}")).or_insert_with(|| start.get(key).cloned().unwrap_or_default());
        let r = ui.add(egui::TextEdit::singleline(buf).desired_width(w).font(font(13.5, 400)).margin(vec2(8.0, 5.0)));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, name));
    };
    let w = ui.available_width();
    // The disc's own fields, whatever the question offers (an album's, a
    // game's name): the known ones first, then the rest in order, up to
    // three to a row.
    const KNOWN: [&str; 8] = ["album", "album_artist", "name", "title", "year", "kind", "season", "tmdbid"];
    let mut keys: Vec<&str> = KNOWN.iter().copied().filter(|k| start.contains_key(*k)).collect();
    keys.extend(start.keys().map(String::as_str).filter(|k| !k.contains('.') && !KNOWN.contains(k)));
    let per_row = keys.len().clamp(1, 3);
    for row in keys.chunks(per_row) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let narrow = |k: &str| matches!(k, "year" | "season" | "kind");
            let short = row.iter().map(|k| if narrow(k) { 0.5 } else { 1.0 }).sum::<f32>();
            for key in row {
                let share = if narrow(key) { 0.5 } else { 1.0 } / short;
                let fw = (w - 12.0 * (row.len() as f32 - 1.0)) * share;
                let name = detail_label(key);
                ui.vertical(|ui| {
                    ui.set_width(fw);
                    ui.label(RichText::new(&name).font(font(12.5, 400)).color(p.muted));
                    ui.add_space(2.0);
                    if *key == "kind" {
                        // A film or a series: two choices, not a word to type.
                        let k = format!("details/{key}");
                        let cur = app.buffers.entry(k.clone()).or_insert_with(|| start.get(*key).cloned().unwrap_or_default()).clone();
                        let at = match cur.as_str() {
                            "film" | "movie" => Some(0),
                            "series" | "tv" => Some(1),
                            _ => None,
                        };
                        if let Some(n) = segmented(ui, p, &name, &["Film", "Series"], at, true) {
                            app.buffers.insert(k, if n == 0 { "film" } else { "series" }.into());
                        }
                    } else {
                        field(app, ui, key, &name, fw);
                    }
                });
            }
        });
        ui.add_space(6.0);
    }
    ui.add_space(10.0);
    // The tracks: number, title, artist.
    let tracks = start.keys().filter_map(|k| k.strip_prefix("title.")).filter_map(|n| n.parse::<u32>().ok()).max().unwrap_or(0);
    if tracks == 0 {
        return;
    }
    let (num, tw) = (26.0, (w - 26.0 - 12.0 - 12.0) * 0.6);
    let aw = w - num - tw - 24.0 - 12.0;
    // Over their boxes, left-aligned like every other label.
    let (head, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
    for (x, text) in [(num, "Title"), (num + tw + 12.0, "Artist")] {
        ui.painter().text(pos2(head.left() + x, head.center().y), egui::Align2::LEFT_CENTER, text, font(12.5, 400), p.muted);
    }
    ui.add_space(2.0);
    egui::ScrollArea::vertical().id_salt("details").max_height(room(ui)).show(ui, |ui| {
        ui.set_width(w - 12.0);
        for n in 1..=tracks {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (r, _) = ui.allocate_exact_size(vec2(num - 12.0, 26.0), Sense::hover());
                ui.painter().text(pos2(r.right(), r.center().y), egui::Align2::RIGHT_CENTER, n.to_string(), font(12.5, 400), p.subtle);
                field(app, ui, &format!("title.{n}"), &format!("Title {n}"), tw);
                field(app, ui, &format!("artist.{n}"), &format!("Artist {n}"), aw);
            });
            ui.add_space(2.0);
        }
    });
}

/// One option, on one 36 px line: the radio, its title, and what tells it
/// apart (its detail, in tertiary).
/// Gives whether it was picked, and whether its link (its entry's page:
/// site, url) was clicked.
#[allow(clippy::too_many_arguments)]
fn card(ui: &mut Ui, p: &Pal, on: bool, title: &str, detail: &str, salt: impl std::hash::Hash + std::fmt::Debug, enabled: bool, link: Option<(&str, &str)>) -> (bool, bool) {
    let w = ui.available_width();
    let link_label = link.map(|(site, _)| format!("{site} ↗"));
    let lw = link_label.as_ref().map_or(0.0, |l| crate::widgets::link_width(ui, l) + 10.0);
    // Title, detail and link on one line when they fit (measured); else
    // the title has the line, and the detail and the link the next.
    let full = w - 12.0 - 34.0;
    let room = full - lw;
    let need = |t: &str, f: egui::FontId| ui.painter().layout_no_wrap(t.to_owned(), f, p.ink).size().x;
    let (tn, dn) = (need(title, font(13.0, 600)), need(detail, font(13.0, 400)));
    let two = !detail.is_empty() && tn + 8.0 + dn > room;
    // Under the title, the detail wraps to two lines before it's cut.
    let detail_g = two.then(|| {
        let mut d = egui::text::LayoutJob::single_section(detail.to_owned(), egui::TextFormat::simple(font(13.0, 400), p.subtle));
        d.wrap = egui::text::TextWrapping { max_width: room, max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
        ui.painter().layout_job(d)
    });
    let dh = detail_g.as_ref().map_or(0.0, |g| g.size().y);
    let (rect, _) = ui.allocate_exact_size(vec2(w, if two { 36.0 + dh } else { 36.0 }), Sense::hover());
    let pt = ui.painter();
    pt.rect(rect, 6.0, p.raised, Stroke::new(if on { 2.0 } else { 1.0 }, if on { p.action } else { p.line }), egui::StrokeKind::Inside);
    let c = pos2(rect.left() + 12.0 + 7.0, if two { rect.top() + 17.0 } else { rect.center().y });
    if on {
        pt.circle_filled(c, 7.0, p.action);
        pt.circle_filled(c, 3.0, p.raised);
    } else {
        pt.circle(c, 6.5, p.raised, Stroke::new(1.5, p.faint));
    }
    let tx = c.x + 15.0;
    let tw = rect.right() - 12.0 - if two { 0.0 } else { lw } - tx;
    let mut job = egui::text::LayoutJob::default();
    job.append(title, 0.0, egui::TextFormat::simple(font(13.0, 600), p.ink));
    if !detail.is_empty() && !two {
        job.append(detail, 8.0, egui::TextFormat::simple(font(13.0, 400), p.subtle));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(tw);
    let g = pt.layout_job(job);
    pt.galley(pos2(tx, c.y - g.size().y / 2.0), g, p.ink);
    let mut cut = tn > tw + 0.5;
    if let Some(dg) = detail_g {
        cut |= dg.elided;
        pt.galley(pos2(tx, rect.top() + 27.0), dg, p.subtle);
    }
    let r = egui::InnerResponse { inner: (), response: ui.interact(rect, ui.scope_id().with(("card-rect", &salt)), Sense::hover()) };
    let resp = ui.interact(r.response.rect, ui.scope_id().with(salt), if enabled { Sense::click() } else { Sense::hover() });
    let name = [title, detail].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
    resp.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::RadioButton, enabled, on, &name));
    let resp = if cut { resp.on_hover_text(format!("{title}\n{detail}")) } else { resp };
    if resp.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // The link, over the card: it takes its own click.
    let opened = match (link, &link_label) {
        (Some((site, url)), Some(label)) => crate::widgets::link_at(ui, p, rect.right() - 12.0, if two { rect.top() + 36.0 } else { c.y }, label, &format!("Open on {site}, {title}"), url).1,
        _ => false,
    };
    (resp.clicked() && !opened, opened)
}

/// Rows split into two columns, as the mockup's track lists.
fn two_columns(ui: &mut Ui, _p: &Pal, n: usize, mut each: impl FnMut(&mut Ui, usize, f32)) {
    let per = n.div_ceil(2).max(1);
    let w = ((ui.available_width() - 22.0) / 2.0).max(100.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 22.0;
        for c in 0..2 {
            ui.vertical(|ui| {
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = 0.0;
                for k in (c * per)..((c + 1) * per).min(n) {
                    each(ui, k, w);
                }
            });
        }
    });
}

/// A question that wants a value: the box on the left; Use in the header.
fn value_question(app: &mut App, lu: &mut Ui, p: &Pal, snap: &Snapshot, step: &str) {
    let Some((_, q)) = snap.question.clone() else { return };
    let QuestionShape::Value { label, suggested, folder } = &q.shape else { return };
    let key = format!("q/{step}");
    let text = app.buffers.entry(key).or_insert_with(|| suggested.clone());
    let mut pick = false;
    field(
        lu,
        p,
        label,
        |ui, l| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - if *folder { 40.0 } else { 0.0 };
                ui.add(egui::TextEdit::singleline(text).desired_width(w).font(font(13.5, 400)).margin(vec2(8.0, 5.0))).labelled_by(l);
                pick = *folder && Btn::new("folder-link", "").name("Choose folder").pad(vec2(7.0, 6.0)).show(ui, p).clicked();
            });
        },
        None,
    );
    // A folder picked in the portal fills the box; Use sends it.
    if pick {
        let hook = app.m.pick_folder.clone();
        let tx = app.picked_answer.0.clone();
        let ctx = lu.ctx().clone();
        let (step, start) = (step.to_owned(), expand(text).ancestors().find(|a| a.is_dir()).map(|a| a.to_owned()));
        std::thread::spawn(move || {
            if let Some(path) = hook(start) {
                let _ = tx.send((step, path.display().to_string()));
                ctx.request_repaint();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` in a frame with the app's fonts.
    fn in_frame(f: impl Fn(&mut Ui)) {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::fonts());
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ui| f(ui));
            out.textures_delta.clear();
        }
    }

    #[test]
    fn tabular_figures_are_all_one_width_and_letters_still_draw() {
        in_frame(|ui| {
            let w = |t: &str| ui.painter().layout_no_wrap(t.to_owned(), crate::theme::tnum(13.0, 400), Color32::WHITE).size().x;
            let one = w("1");
            for d in "023456789".chars() {
                assert!((w(&d.to_string()) - one).abs() < 0.01, "{d}");
            }
            assert!((w("11:11") - w("00:00")).abs() < 0.01);
            // Letters from the face itself, as wide as in it.
            let plain = ui.painter().layout_no_wrap("Verified".into(), font(13.0, 400), Color32::WHITE).size().x;
            assert!((w("Verified") - plain).abs() < 0.01 && plain > 20.0);
        });
    }

    #[test]
    fn a_changing_percentage_doesnt_move_its_column() {
        in_frame(|ui| {
            let list = |pct: &str| {
                let mut r = Row::new(Some(2), "Stop Breaking Down");
                r.verdict = Some(("Reading".into(), Tone::Busy, pct.into()));
                vec![Row::new(Some(1), "Jimmy the Exploder"), r]
            };
            let widths: Vec<f32> = ["0%", "9%", "10%", "55%", "100%"].iter().map(|p| verdict_column(ui, &Pal::get(false), &list(p), 448.0)).collect();
            assert!(widths.iter().all(|w| (w - widths[0]).abs() < 0.01), "{widths:?}");
        });
    }

    #[test]
    fn long_file_names_keep_their_extension() {
        let m = |t: &str| t.chars().count() as f32;
        assert_eq!(keep_extension("01 - Sugar Never Tasted So Good.flac", 20.0, m), "01 - Sugar Nev….flac");
        assert_eq!(keep_extension("short.flac", 20.0, m), "short.flac");
    }

    #[test]
    fn lyrics_lines_keep_their_times_and_drop_tags() {
        let l = lrc_lines("[ar:Someone]\n[00:12.40]One\n[01:02.00][00:30.00]Two");
        assert_eq!(l, vec![(Some("0:12".into()), "One".into()), (Some("0:30".into()), "Two".into()), (Some("1:02".into()), "Two".into())]);
        let plain = lrc_lines("A\n\nB");
        assert_eq!(plain, vec![(None, "A".into()), (None, "".into()), (None, "B".into())]);
    }

    #[test]
    fn rip_notes_read_as_verdicts() {
        assert_eq!(rip_verdict(ItemState::Ok, "✓ 104").0, "Verified");
        assert_eq!(rip_verdict(ItemState::Ok, "✓ 104").2, "matches 104 other rips");
        assert_eq!(rip_verdict(ItemState::Ok, "✓ 1").2, "matches 1 other rip");
        assert_eq!(rip_verdict(ItemState::Ok, "✓ two reads agree").0, "Reads agree");
        assert_eq!(rip_verdict(ItemState::Ok, "✓ consistent").0, "Reads agree", "as earlier runs said it");
        assert_eq!(rip_verdict(ItemState::Ok, "✓ best of 3 reads").0, "Best of 3");
        assert_eq!(rip_verdict(ItemState::Ok, "✓ combined from 3 reads").0, "Best of 3", "as earlier runs said it");
        let (w, t, d, _) = rip_verdict(ItemState::Ok, "best of 5 reads, 2 samples uncertain");
        assert_eq!((w.as_str(), t, d.as_str()), ("Best of 5", Tone::Warn, "2 samples uncertain"));
        assert_eq!(rip_verdict(ItemState::Ok, "combined from 3 reads, 4 samples unsettled").2, "4 samples uncertain");
        assert_eq!(rip_verdict(ItemState::Ok, drt_steps::real::KEPT).0, "Kept anyway");
        assert_eq!(rip_verdict(ItemState::Ok, "kept, unverified").1, Tone::Warn);
        let (w, t, _, why) = rip_verdict(ItemState::Bad, "439 samples differ, 0:17–0:43");
        assert_eq!((w.as_str(), t), ("Damaged", Tone::Bad));
        assert_eq!(why.unwrap(), "Reads differ at 0:17–0:43 (439 samples)");
        assert_eq!(rip_verdict(ItemState::Bad, "no match").0, "Doesn't match");
        assert_eq!(rip_verdict(ItemState::Bad, "no match").3.unwrap(), "Other people's rips of this disc differ");
        assert_eq!(rip_verdict(ItemState::Bad, "").0, "Couldn't be read");
        assert_eq!(rip_verdict(ItemState::Bad, "something new").3.unwrap(), "Something new", "unknown notes shown as they are");
        assert_eq!(rip_verdict(ItemState::Busy, "re-reading 40%").0, "Re-reading 40%");
    }

    /// An image Rip made by itself with sectors it couldn't read says so,
    /// and that Check and fix will try them; never a tick.
    #[test]
    fn an_image_made_anyway_says_check_and_fix_will_try() {
        let (head, _, tone) = dump_verdict(ItemState::Ok, &drt_steps::real::data::made_anyway_note(21));
        assert_eq!((head.as_str(), tone), ("21 sectors couldn't be read", Tone::Warn));
    }

    /// An item the run was stopped in the middle of says so, and nothing
    /// of it looks busy; one left out is skipped.
    #[test]
    fn an_item_stopped_part_way_says_stopped_not_reading() {
        let stopped = drt_engine::snapshot::STOPPED;
        assert_eq!(dump_verdict(ItemState::Skipped, stopped), ("Stopped".into(), "stopped".into(), Tone::Plain));
        assert_eq!(dump_verdict(ItemState::Skipped, "skipped").0, "Skipped");
        for (w, t, _, _) in [rip_verdict(ItemState::Skipped, stopped), item_verdict(ItemState::Skipped, stopped), title_verdict(ItemState::Skipped, stopped)] {
            assert_eq!((w.as_str(), t), ("Stopped", Tone::Plain));
        }
    }

    #[test]
    fn what_an_image_came_to_from_each_note_image_makes() {
        use drt_steps::real::data::image_note;
        let v = dump_verdict;
        assert_eq!(v(ItemState::Ok, &image_note("682 MB", false, 0, false, &[])), ("Read cleanly".into(), "read".into(), Tone::Ok));
        assert_eq!(v(ItemState::Ok, &image_note("682 MB", false, 0, true, &[])), ("Bad parts re-read".into(), "read".into(), Tone::Ok));
        // Kept, or sectors read wrong: done, but never a tick.
        assert_eq!(v(ItemState::Ok, &image_note("682 MB", true, 0, false, &[])).0, "Kept anyway");
        assert_eq!(v(ItemState::Ok, &image_note("682 MB", true, 3, false, &[])), ("Kept with 3 unreadable sectors left blank".into(), "kept".into(), Tone::Warn));
        assert_eq!(v(ItemState::Ok, &image_note("682 MB", false, 0, false, &[("EDC", 2)])), ("2 sectors read wrong".into(), "not clean".into(), Tone::Warn));
        assert_eq!(v(ItemState::Ok, "read, but 2 sectors came back wrong").0, "2 sectors read wrong", "an older run's note");
        assert_eq!(v(ItemState::Bad, "3 unreadable sectors").0, "3 sectors couldn't be read");
        assert_eq!(v(ItemState::Bad, "1 unreadable sector").0, "1 sector couldn't be read");
        assert_eq!(v(ItemState::Bad, "drive not ready").0, "Drive not ready");
        assert_eq!(thousands(2_298_496), "2,298,496");
    }

    #[test]
    fn times_read_as_a_clock_and_in_words() {
        assert_eq!((clock(41), clock(2_292), clock(6_730)), ("0:41".into(), "38:12".into(), "1:52:10".into()));
        assert_eq!((span(41), span(2_820), span(9_060)), ("41 s".into(), "47 min".into(), "2 h 31 min".into()));
    }

    #[test]
    fn refusals_read_as_sentences() {
        assert_eq!(crate::app::sentence("last step"), "A recipe needs at least its drive.");
        assert_eq!(crate::app::sentence("no step called x"), "No step called x.");
        assert_eq!(crate::app::sentence("Rip?"), "Rip?");
    }

    #[test]
    fn progress_is_read_from_notes_and_details() {
        assert_eq!(progress("encoding 43%, 12s left"), (Some(0.43), Some("12s left".to_owned())));
        assert_eq!(progress("re-reading 40%"), (Some(0.4), None));
        assert_eq!(progress("encoding"), (None, None));
        assert_eq!(percent("2 encoding, 61%"), Some("61%"));
        assert_eq!(percent("2 encoding"), None);
    }

    #[test]
    fn dates_read_as_day_and_month() {
        assert_eq!(day_month(1_795_039_822), "18 Nov");
        assert_eq!(day_month(0), "1 Jan");
    }

    #[test]
    fn file_names_come_from_the_template() {
        let f = sample();
        assert_eq!(
            template::render(template::resolve("Sorted artist / Album / 01 Artist - Title"), &f),
            std::path::PathBuf::from("Cinematic Orchestra, The/Every Day/01 The Cinematic Orchestra - All That You Give")
        );
        assert_eq!(fields(None, 3, "Track 3").title, "Track 3");
    }
}

// ---- the step's record (docs/ux/step-record.md) -----------------------------------------------

/// What a step's record holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Items,
    Found,
    Log,
}

/// The tabs a step has something for, in order. None before it has run.
fn record_tabs(app: &App, i: usize, id: &str, v: &StepView, info: &StepInfo) -> Vec<Held> {
    let r = &v.record;
    let ctx = app.m.shown_context(i);
    let mut tabs = vec![];
    // Items: a list of the things it worked on (not a lookup's disc);
    // Audio CD's, the disc's tracks.
    let disc_tracks = v.step == "audio-cd" && ctx.as_ref().is_some_and(|c| c.parts() > 0);
    if (!v.items.is_empty() && info.group != Group::LookUp) || disc_tracks {
        tabs.push(Held::Items);
    }
    let found = ctx.as_ref().is_some_and(|c| c.found_by(id).next().is_some() || c.offered_by(id).next().is_some());
    if found || r.options.as_ref().is_some_and(|o| !o.is_empty()) || r.decided.is_some() || !r.files.is_empty() {
        tabs.push(Held::Found);
    }
    if !r.log.is_empty() || !app.m.shown_raw(i, id).is_empty() {
        tabs.push(Held::Log);
    }
    tabs
}

/// The decision: what was chosen and why, where from; a link to its
/// entry, if it has a page. Gives the link if clicked.
fn decision_card(ui: &mut Ui, p: &Pal, d: &drt_engine::Decision, c: Option<&drt_engine::Candidate>) -> Option<String> {
    let mut open = None;
    egui::Frame::new().fill(p.raised).stroke(Stroke::new(2.0, p.action)).corner_radius(8).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let title = if d.title.is_empty() { "Nothing" } else { d.title.as_str() };
            ui.label(RichText::new(title).font(font(14.5, 600)).color(p.ink));
            ui.add_space(4.0);
            crate::widgets::chip(ui, &d.why, p.action, Color32::from_rgba_unmultiplied(p.action.r(), p.action.g(), p.action.b(), 26));
        });
        if let Some(c) = c {
            ui.add_space(2.0);
            let link = crate::links::entry(c);
            let lw = link.as_ref().map_or(0.0, |(site, _)| crate::widgets::link_width(ui, &format!("{site} ↗")) + 12.0);
            let r = ui
                .scope(|ui| {
                    ui.set_max_width(ui.available_width() - lw);
                    ui.add(egui::Label::new(RichText::new(format!("{} · {}", c.source, c.detail)).font(font(12.5, 400)).color(p.subtle)).wrap()).rect
                })
                .inner;
            if let Some((site, url)) = link {
                let right = ui.max_rect().right();
                if crate::widgets::link_at(ui, p, right, r.top() + 9.0, &format!("{site} ↗"), &format!("Open on {site}, {}", c.title), &url).1 {
                    open = Some(url);
                }
            }
        }
    });
    ui.add_space(10.0);
    open
}

/// A candidate: title, detail, where from, how well it fits.
/// Gives its entry's page if its link was clicked.
fn candidate_row(ui: &mut Ui, p: &Pal, c: &drt_engine::Candidate, chosen: bool) -> Option<String> {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    let name = format!("{}, {}, {}, {:.0}%{}", c.title, c.detail, c.source, c.score * 100.0, if chosen { ", chosen" } else { "" });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ListItem, true, &name));
    let pt = ui.painter();
    let x = r.left() + 22.0;
    if chosen {
        crate::widgets::chevron(pt, pos2(r.left() + 8.0, r.top() + 12.0), 0.0, 0.0, p.ok);
        pt.text(pos2(r.left() + 2.0, r.top() + 12.0), egui::Align2::LEFT_CENTER, "✓", font(13.0, 600), p.ok);
    }
    let score = format!("{:.0}%", c.score * 100.0);
    let sg = pt.layout_no_wrap(score, crate::theme::tnum(12.5, 500), p.muted);
    let sw = sg.size().x;
    pt.galley(pos2(r.right() - sw, r.top() + 12.0 - sg.size().y / 2.0), sg, p.muted);
    let mut t = egui::text::LayoutJob::single_section(c.title.clone(), egui::TextFormat::simple(font(13.5, 600), p.ink));
    t.wrap = egui::text::TextWrapping::truncate_at_width(r.right() - sw - 12.0 - x);
    let tg = pt.layout_job(t);
    pt.galley(pos2(x, r.top() + 12.0 - tg.size().y / 2.0), tg, p.ink);
    let line = format!("{} · {}", c.source, c.detail);
    // Its entry's page, on the second line's right.
    let link = crate::links::entry(c);
    let lw = link.as_ref().map_or(0.0, |(site, _)| crate::widgets::link_width(ui, &format!("{site} ↗")) + 12.0);
    let cut = pt.layout_no_wrap(c.title.clone(), font(13.5, 600), p.ink).size().x > r.right() - sw - 12.0 - x + 0.5
        || pt.layout_no_wrap(line.clone(), font(12.5, 400), p.ink).size().x > r.right() - x - lw + 0.5;
    if cut {
        resp.clone().on_hover_text(format!("{}\n{line}", c.title));
    }
    let mut d = egui::text::LayoutJob::single_section(line, egui::TextFormat::simple(font(12.5, 400), p.subtle));
    d.wrap = egui::text::TextWrapping::truncate_at_width(r.right() - x - lw);
    let dg = pt.layout_job(d);
    pt.galley(pos2(x, r.top() + 29.0 - dg.size().y / 2.0), dg, p.subtle);
    pt.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.line_soft));
    let (site, url) = link?;
    crate::widgets::link_at(ui, p, r.right(), r.top() + 29.0, &format!("{site} ↗"), &format!("Open on {site}, {}", c.title), &url).1.then_some(url)
}

/// The facts a step found as a table: the disc's row first, then one per
/// track; a column per key.
fn fact_table(ui: &mut Ui, p: &Pal, facts: &[&drt_engine::Fact]) {
    let mut keys: Vec<&str> = vec![];
    for f in facts {
        if !keys.contains(&f.key.as_str()) {
            keys.push(&f.key);
        }
    }
    let mut subjects: Vec<Subject> = facts.iter().map(|f| f.subject).collect();
    subjects.sort();
    subjects.dedup();
    let w = ui.available_width();
    let first = 52.0;
    // Columns as wide as they share, but not so wide a row is hard to
    // follow.
    let col = ((w - first) / keys.len().max(1) as f32).clamp(90.0, 340.0);
    // Head: the keys, as words.
    let (head, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    for (k, key) in keys.iter().enumerate() {
        let x = head.left() + first + k as f32 * col;
        ui.painter().text(pos2(x, head.center().y), egui::Align2::LEFT_CENTER, key_word(key), font(12.5, 500), p.muted);
    }
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.line));
    for s in subjects {
        let (row, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
        let label = match s {
            Subject::Disc => "Disc".to_owned(),
            Subject::Part(n) => n.to_string(),
        };
        let mut said = vec![label.clone()];
        ui.painter().text(pos2(row.left(), row.center().y), egui::Align2::LEFT_CENTER, &label, font(12.5, 500), p.subtle);
        for (k, key) in keys.iter().enumerate() {
            let Some(f) = facts.iter().find(|f| f.subject == s && f.key == *key) else { continue };
            let x = row.left() + first + k as f32 * col;
            let right = x + col - 12.0;
            let value = f.value.replace('\n', " ");
            let mut job = egui::text::LayoutJob::single_section(value.clone(), egui::TextFormat::simple(font(13.0, 400), p.ink));
            job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(x, row.center().y - g.size().y / 2.0), g, p.ink);
            said.push(format!("{key} {value}"));
        }
        let name = said.join(", ");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Row, true, &name));
        ui.painter().hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, p.line_soft));
    }
    ui.add_space(8.0);
}

/// A fact's key as a column's head: "album_artist" as "Album artist".
fn key_word(k: &str) -> String {
    match k {
        "parts" => "Tracks".into(),
        "length" => "Length (s)".into(),
        "disc_id" => "Disc ID".into(),
        "isrc" => "ISRC".into(),
        "cdtext.title" => "CD-Text title".into(),
        "cdtext.album" => "CD-Text album".into(),
        "cdtext.artist" => "CD-Text artist".into(),
        k => cap(&k.replace(['_', '.'], " ")),
    }
}

/// The files a step wrote, by folder: name and size; a folder can be
/// opened.
fn files(app: &mut App, ui: &mut Ui, p: &Pal, list: &[drt_engine::Made]) {
    let mut folders: Vec<(String, Vec<&drt_engine::Made>)> = vec![];
    let mut list: Vec<&drt_engine::Made> = list.iter().collect();
    list.sort_by(|a, b| a.path.cmp(&b.path));
    for f in list {
        let dir = std::path::Path::new(&f.path).parent().map(|d| d.display().to_string()).unwrap_or_default();
        match folders.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, v)) => v.push(f),
            None => folders.push((dir, vec![f])),
        }
    }
    let w = ui.available_width();
    for (dir, fs) in folders {
        if !dir.is_empty() {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&dir).font(font(12.5, 500)).color(p.muted)).truncate());
                ui.with_layout(L::right_to_left(Align::Center), |ui| {
                    let path = expand(&dir);
                    if path.is_absolute() && Btn::new("folder-link", "Open").look(Look::Quiet).font(font(12.5, 500)).show(ui, p).on_hover_text(path.display().to_string()).clicked() {
                        app.m.open(&path);
                    }
                });
            });
        }
        for f in fs {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
            let name = std::path::Path::new(&f.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.path.clone());
            let size = drt_steps::real::media::size(f.size);
            let acc = format!("{name}, {size}");
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ListItem, true, &acc));
            let pt = ui.painter();
            let sg = pt.layout_no_wrap(size, crate::theme::mono(12.0), p.muted);
            let sw = sg.size().x;
            pt.galley(pos2(r.right() - sw, r.center().y - sg.size().y / 2.0), sg, p.muted);
            let mut job = egui::text::LayoutJob::single_section(name, egui::TextFormat::simple(font(13.0, 400), p.ink));
            job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - sw - 16.0);
            let g = pt.layout_job(job);
            pt.galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, p.ink);
            pt.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.line_soft));
        }
        ui.add_space(8.0);
    }
}

/// The step's log, newest at the bottom and following it; its tool's
/// output folded beneath, open by itself when the step failed.
fn log_view(app: &mut App, ui: &mut Ui, p: &Pal, id: &str, v: &StepView) {
    let i = app.sel;
    let raw = app.m.shown_raw(i, id);
    let w = ui.available_width();
    let start = v.record.started.or(v.record.log.first().map(|l| l.at)).unwrap_or(0);
    // Inside the sidebar's own scrolling: a window onto each.
    let total: f32 = 360.0;
    let open = app.raw_open.unwrap_or(failed(app, id, v)) && !raw.is_empty();
    let log_h = if raw.is_empty() {
        total
    } else if open {
        (total * 0.4).max(60.0)
    } else {
        total - 34.0
    };
    egui::ScrollArea::vertical().id_salt(("log", id)).max_height(log_h).stick_to_bottom(true).show(ui, |ui| {
        ui.set_width(w - 12.0);
        if v.record.log.is_empty() {
            ui.label(RichText::new("Nothing logged").font(font(13.0, 400)).color(p.subtle));
        }
        for l in &v.record.log {
            ui.horizontal_top(|ui| {
                let t = l.at.saturating_sub(start);
                ui.add_sized(vec2(44.0, 18.0), egui::Label::new(RichText::new(mmss(t)).font(crate::theme::mono(12.0)).color(p.subtle)));
                ui.add(egui::Label::new(RichText::new(&l.line).font(font(13.0, 400)).color(if l.line.starts_with("failed") { p.bad } else { p.ink })).wrap());
            });
        }
    });
    if raw.is_empty() {
        return;
    }
    ui.add_space(6.0);
    // The fold: "Tool output", with how many lines.
    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::Button, true, open, "Tool output"));
    let col = if resp.hovered() { p.ink } else { p.muted };
    crate::widgets::chevron(ui.painter(), pos2(r.left() + 6.0, r.center().y), 13.0, if open { 0.0 } else { -std::f32::consts::FRAC_PI_2 }, col);
    ui.painter().text(pos2(r.left() + 17.0, r.center().y), egui::Align2::LEFT_CENTER, "Tool output", font(12.5, 600), col);
    let n = format!("{} lines", raw.len());
    ui.painter().text(pos2(r.right(), r.center().y), egui::Align2::RIGHT_CENTER, n, font(12.0, 400), p.subtle);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    crate::widgets::focus_ring(ui, &resp, r, 6.0, p);
    if resp.clicked() {
        app.raw_open = Some(!open);
    }
    if open {
        egui::Frame::new().fill(if p.dark { p.bg } else { mix(p.ink, 0.03, p.surface) }).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(w - 20.0);
            egui::ScrollArea::vertical().id_salt(("raw", id)).max_height(320.0).stick_to_bottom(true).show(ui, |ui| {
                ui.set_width(w - 32.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                let text = raw.join("\n");
                let r = ui.add(egui::Label::new(RichText::new(text).font(crate::theme::mono(12.0)).color(p.muted)).wrap());
                r.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Label, true, "Tool output lines"));
            });
        });
    }
}
