//! The window: the recipes down the left; the chosen one's header, its
//! canvas, and the sidebar beside it for the selected step.

use crate::canvas::{self, CardExtra, Click, Marks, View};
use crate::chrome;
use crate::model::Model;
use crate::theme::{self, font, kind_icon, mix, state_icon, Pal};
use crate::widgets::{chip, pill, segmented, Btn, Look, Tone};
use drt_engine::{Scope, Setting, StepState};
use egui::{pos2, vec2, Align, Align2, Key, Layout as L, Margin, Rect, RichText, Sense, Stroke, Ui};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

/// Both ends of a channel from a worker thread to the window.
pub(crate) type Channel<T> = (mpsc::Sender<T>, mpsc::Receiver<T>);

/// A folder picked in the portal: the recipe, the step, the setting's key,
/// and the path.
pub(crate) type Picked = (usize, String, String, String);

/// What the sidebar shows. One at a time.
#[derive(Clone, Debug, PartialEq)]
pub enum Tray {
    /// The step asking, and its question.
    Question { step: String },
    /// A tile's parameters and items. `focus` is a parameter to put the
    /// cursor in: the one setup needs.
    Step { id: String, focus: Option<String> },
}

/// The sidebar's least width, and its most: about a third of the window
/// between (448 at 1280, 672 at 1920).
pub const SIDE_W: f32 = 440.0;
pub const SIDE_MAX: f32 = 680.0;
/// How wide a question that compares two lists (Choose's candidates
/// beside the disc's tracks) may make it.
pub const SIDE_WIDE: f32 = 800.0;

pub struct App {
    pub m: Model,
    /// What the sidebar showed last, drawn while it slides out.
    pub(crate) side_last: Option<Tray>,
    /// When it last opened or closed, for its slide.
    pub(crate) side_t0: Option<(f64, bool)>,
    /// How wide it is this frame (0 when shut), and when open.
    pub(crate) side_w: f32,
    pub(crate) side_full: f32,
    /// The width it's going to.
    side_target: f32,
    pub sel: usize,
    views: HashMap<usize, View>,
    pub tray: Option<Tray>,
    /// Questions you closed, so they don't open again by themselves.
    dismissed: HashSet<(usize, u64, String)>,
    /// Questions that have opened the sidebar by themselves once.
    asked: HashSet<(usize, u64, String)>,
    pub editing: bool,
    /// The name being typed for a new recipe.
    pub naming: Option<String>,
    pub dark: bool,
    /// Light, dark or following the system; remembered.
    pub appearance: Appearance,
    applied: Option<bool>,
    /// Text being typed into a parameter, by `step/key`.
    pub(crate) buffers: HashMap<String, String>,
    pub(crate) focus_done: bool,
    /// Folders picked in the portal, arriving off the UI thread.
    pub(crate) picked: Channel<Picked>,
    /// A refusal to show for a moment: *doesn't fit here*.
    pub(crate) flash: Option<(String, f64)>,
    /// Fields holding text that doesn't read as their value, and why:
    /// the text stays, outlined, until it does.
    pub(crate) invalid: HashMap<String, String>,
    /// A per-item question's choices so far, by item id.
    pub(crate) per_item: HashMap<String, Option<usize>>,
    /// The candidate picked in a Which release? question.
    pub pick: Option<usize>,
    /// The window's corner radius just now (none when maximised).
    pub round: f32,
    /// The window title last sent.
    pub title: String,
    /// Whether things may move: off under GNOME's Reduce animations (or
    /// DRT_REDUCED_MOTION=1), and while a screenshot's clock is pinned.
    pub motion: bool,
    motion_applied: Option<bool>,
    /// This frame's time, for cues started outside the UI pass.
    now: f64,
    /// A camera move under way: recipe, from, to, start, length.
    glide: Option<(usize, View, View, f64, f32)>,
    /// One-shot cues: the states last seen for the shown recipe and run,
    /// and the cues playing.
    cue_key: Option<(usize, u64)>,
    cue_seen: HashMap<String, StepState>,
    pub cues: HashMap<String, (StepState, f64)>,
    /// Follow: the canvas keeps the step where something last happened in
    /// view. Remembered; a drag or a zoom by hand turns it off.
    pub follow: bool,
    /// The step that last began, finished or began to ask, and when.
    follow_last: Option<(String, f64)>,
    /// When Follow last moved the camera.
    follow_moved: f64,
    /// The +s that would offer nothing, for the tree they were worked out
    /// on: they aren't drawn.
    dead_pluses: Option<(drt_engine::Node, std::collections::HashSet<drt_engine::Spot>)>,
    /// The top band this frame (the list's top row, the header), and the
    /// title field in it when editing, for `chrome::band`.
    band: Vec<Rect>,
    title_field: Option<Rect>,
    /// Closing was asked for while a disc was going through: the header
    /// asks first.
    pub confirm_quit: bool,
    /// Quit was confirmed: the next close goes ahead.
    quit_ok: bool,
    /// Whether a field had the keys at the end of last frame: its Esc is
    /// its own (egui lets go of the focus before the frame sees the key).
    was_typing: bool,
    /// Bring this tile into view on the next frame.
    reveal: Option<String>,
    /// The time spinners turn by; fixed in screenshots.
    pub clock: Option<f64>,
    /// A Remove waiting to be confirmed: the step, and every id that goes.
    pub confirm_remove: Option<(String, Vec<String>)>,
    /// A recipe you picked while this one had unsaved edits.
    pub leave_to: Option<usize>,
    /// A recipe whose header asks "Delete …?".
    pub confirm_delete: Option<usize>,
    /// A Delete refused, and why: said in the header.
    pub delete_refused: Option<(usize, String)>,
    /// The + popover, when one is open.
    pub popover: Option<crate::popover::Popover>,
    pub popover_rect: Rect,
    /// A tile shown selected without its sidebar: one just added.
    pub selected: Option<String>,
    /// Whether the Log's tool output is open (else it opens for a failure).
    pub(crate) raw_open: Option<bool>,
    /// Titles the sidebar's rows had to cut short last frame, for tests.
    pub cut_titles: Vec<String>,
    /// The tracks whose lyrics are open under their row in Lyrics.
    pub lyrics_open: HashSet<u32>,
    /// The sidebar's tab you picked; else it opens on the one that fits.
    pub rtab: Option<crate::tray::Tab>,
    /// Where the cursor was in each template box, by `step/key`, so a
    /// token chip inserts there.
    pub(crate) cursors: HashMap<String, usize>,
    /// Cover images, decoded off the window's thread, by path.
    pub covers: HashMap<std::path::PathBuf, crate::tray::Cover>,
    pub(crate) cover_rx: Channel<(std::path::PathBuf, Option<egui::ColorImage>)>,
    pub canvas: Rect,
    /// Where the sidebar was drawn last frame.
    pub(crate) side_rect: Rect,
    /// Notify's last *Send a test*, filled in off the UI thread.
    pub(crate) test_send: std::sync::Arc<std::sync::Mutex<Option<crate::tray::TestSend>>>,
}

/// How the window looks: as the system is, or always light or dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    System,
    Light,
    Dark,
}

impl Appearance {
    pub fn parse(s: &str) -> Option<Appearance> {
        match s {
            "system" => Some(Appearance::System),
            "light" => Some(Appearance::Light),
            "dark" => Some(Appearance::Dark),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }
}

impl App {
    /// Changes how the window looks, and remembers it.
    pub fn set_appearance(&mut self, a: Appearance) {
        self.appearance = a;
        match a {
            Appearance::Light => self.dark = false,
            Appearance::Dark => self.dark = true,
            Appearance::System => {}
        }
        if let Err(e) = self.m.store.save_appearance(a.as_str()) {
            eprintln!("couldn't save the appearance: {e:#}");
        }
    }

    pub fn new(m: Model, dark: bool) -> App {
        let follow = m.store.load_follow().unwrap_or(false);
        App {
            m,
            side_last: None,
            side_t0: None,
            side_w: 0.0,
            side_full: SIDE_W,
            side_target: SIDE_W,
            sel: 0,
            views: HashMap::new(),
            tray: None,
            dismissed: HashSet::new(),
            asked: HashSet::new(),
            editing: false,
            naming: None,
            dark,
            appearance: if dark { Appearance::Dark } else { Appearance::Light },
            applied: None,
            buffers: HashMap::new(),
            focus_done: false,
            picked: mpsc::channel(),
            flash: None,
            invalid: HashMap::new(),
            per_item: HashMap::new(),
            pick: None,
            round: 0.0,
            title: String::new(),
            motion: !crate::motion::reduced_by_system(),
            motion_applied: None,
            now: 0.0,
            glide: None,
            cue_key: None,
            cue_seen: HashMap::new(),
            cues: HashMap::new(),
            follow,
            follow_last: None,
            follow_moved: f64::NEG_INFINITY,
            dead_pluses: None,
            band: Vec::new(),
            title_field: None,
            confirm_quit: false,
            quit_ok: false,
            was_typing: false,
            reveal: None,
            clock: None,
            confirm_remove: None,
            leave_to: None,
            confirm_delete: None,
            delete_refused: None,
            popover: None,
            popover_rect: Rect::NOTHING,
            selected: None,
            raw_open: None,
            cut_titles: Vec::new(),
            lyrics_open: HashSet::new(),
            rtab: None,
            cursors: HashMap::new(),
            covers: HashMap::new(),
            cover_rx: mpsc::channel(),
            canvas: Rect::NOTHING,
            side_rect: Rect::NOTHING,
            test_send: Default::default(),
        }
    }

    /// The recipe's layout, with Next disc's line back to the start.
    pub fn layout(&self, i: usize) -> drt_layout::Layout {
        let root = &self.m.recipes[i].root;
        let reg = &self.m.engine.registry;
        drt_layout::layout_with_loops(root, drt_layout::Metrics::default(), |id| crate::model::step_kind(root, id).and_then(|k| reg.info(&k)).is_some_and(|inf| inf.loops_back))
    }

    /// Moves the camera to `to`: a glide (zoom geometric, pan straight,
    /// eased) unless nothing may move or the + popover is open (it's pinned
    /// to the view).
    pub fn glide_to(&mut self, i: usize, to: View, secs: f32) {
        let from = self.views_of(i);
        if !crate::motion::on() || self.popover.is_some() || from == to {
            *self.view(i) = to;
            self.glide = None;
            return;
        }
        self.glide = Some((i, from, to, self.now, secs));
    }

    fn step_glide(&mut self, i: usize) {
        let Some((gi, from, to, t0, secs)) = self.glide else { return };
        if gi != i {
            self.glide = None;
            *self.view(gi) = to;
            return;
        }
        let ctx = self.m.ctx.clone();
        let k = ctx.as_ref().map_or(1.0, |c| crate::motion::since(c, t0, secs));
        let e = crate::motion::cubic_in_out(k);
        let zoom = from.zoom * (to.zoom / from.zoom).powf(e);
        // The pan follows so the point at the centre moves straight.
        let pan = from.pan + (to.pan - from.pan) * e;
        *self.view(i) = View { zoom, pan };
        if k >= 1.0 {
            *self.view(i) = to;
            self.glide = None;
        }
    }

    /// Notes each tile's state for the shown recipe and run; a tile that
    /// has just finished, failed or begun to ask plays its cue once.
    /// Switching recipe or run only notes, so nothing replays.
    fn watch_cues(&mut self, i: usize, snap: &drt_engine::Snapshot) {
        let key = (i, self.m.shown_run(i).map_or(u64::MAX, |r| r.id));
        let fresh = self.cue_key != Some(key);
        if fresh {
            self.cue_key = Some(key);
            self.cue_seen.clear();
            self.cues.clear();
            self.follow_last = None;
        }
        // By the state the card shows: a track to decide rings as a
        // question, as does a Hold once it's full.
        for (id, v) in &snap.steps {
            let retries = self.m.info(i, id).is_some_and(|inf| inf.retries.iter().any(|r| !matches!(r.id, "keep" | "leave_out")));
            let st = crate::card::Shown::of(v, retries).cue_state();
            let was = self.cue_seen.insert(id.clone(), st);
            if !fresh && was.is_some_and(|w| w != st) && matches!(st, StepState::Ok | StepState::Bad | StepState::Needs) {
                self.cues.insert(id.clone(), (st, self.now));
            }
            // Follow goes to the latest change, a step that asks first.
            if !fresh && was.is_some_and(|w| w != st) && matches!(st, StepState::Busy | StepState::Ok | StepState::Bad | StepState::Needs) {
                let asking = self.follow_last.as_ref().is_some_and(|(f, _)| f != id && self.cue_seen.get(f) == Some(&StepState::Needs));
                if st == StepState::Needs || !asking {
                    self.follow_last = Some((id.clone(), self.now));
                }
            }
        }
        let now = self.now;
        self.cues.retain(|_, (_, t0)| now - *t0 < 1.0);
    }

    /// Turns Follow on or off, and remembers it.
    pub fn set_follow(&mut self, on: bool) {
        self.follow = on;
        self.follow_moved = f64::NEG_INFINITY;
        if let Err(e) = self.m.store.save_follow(on) {
            eprintln!("couldn't save Follow: {e:#}");
        }
    }

    /// The step where something last happened, which Follow goes to.
    pub fn followed(&self) -> Option<&str> {
        self.follow_last.as_ref().map(|(id, _)| id.as_str())
    }

    /// Follow: once the latest change has settled a moment, glides just
    /// far enough to bring its step into view, zooming out only if it
    /// wouldn't fit. A step that asks you goes first; otherwise it moves
    /// at most every second or so, so several busy steps don't pull it
    /// about.
    fn follow_step(&mut self, i: usize, lay: &drt_layout::Layout, area: egui::Vec2) {
        const SETTLE: f64 = 0.35;
        const GAP: f64 = 1.2;
        if !self.follow || self.editing || self.popover.is_some() {
            return;
        }
        // A step asking now wins over the latest change.
        let asking = self.cue_seen.iter().filter(|(_, s)| **s == StepState::Needs).map(|(id, _)| id.clone()).min_by_key(|id| lay.tile(id).map(|t| t.rect.x as i64));
        let (id, settled) = match (asking, &self.follow_last) {
            (Some(a), Some((l, at))) if *l == a => (a, self.now - at >= SETTLE),
            (Some(a), _) => (a, true),
            (None, Some((l, at))) => (l.clone(), self.now - at >= SETTLE && self.now - self.follow_moved >= GAP),
            (None, None) => return,
        };
        let Some(tp) = lay.tile(&id) else { return };
        if !settled {
            return;
        }
        let from = self.glide.filter(|g| g.0 == i).map_or(self.views_of(i), |g| g.2);
        let mut to = from;
        // Readable: zoomed out further than the cards' own size, in to
        // it (or as near as the canvas has room for); too big for the
        // canvas, out until it fits. Zoomed in by hand, left so.
        let room = (area.x - 48.0) / tp.rect.w;
        let room = room.min((area.y - 48.0) / tp.rect.h);
        let want = if to.zoom > room {
            room
        } else if to.zoom < 1.0 {
            room.min(1.0)
        } else {
            to.zoom
        };
        if (want - to.zoom).abs() > 0.001 {
            let c = egui::Pos2::ZERO + area / 2.0;
            to.zoom_about(egui::Pos2::ZERO, c, want);
        }
        to.reveal(tp.rect, area);
        if (to.pan - from.pan).length() > 0.5 || (to.zoom - from.zoom).abs() > 0.001 {
            self.glide_to(i, to, 0.6);
            self.follow_moved = self.now;
        }
    }

    pub fn view(&mut self, i: usize) -> &mut View {
        self.views.entry(i).or_insert(View { zoom: 1.0, pan: canvas::MARGIN })
    }

    /// A recipe's pan and zoom.
    pub fn views_of(&self, i: usize) -> View {
        self.views.get(&i).copied().unwrap_or(View { zoom: 1.0, pan: canvas::MARGIN })
    }

    pub fn set_view(&mut self, i: usize, v: (f32, egui::Vec2)) {
        *self.view(i) = View { zoom: v.0, pan: v.1 };
    }

    /// Where the sidebar was drawn last frame, if it's open.
    pub fn side_rect(&self) -> Option<Rect> {
        self.tray.is_some().then_some(self.side_rect)
    }

    /// Where the canvas was drawn last frame.
    pub fn canvas_rect(&self) -> Rect {
        self.canvas
    }

    /// Shows one of a recipe's runs: an earlier disc, or (`None`) the
    /// newest.
    pub fn show_run(&mut self, i: usize, run: Option<u64>) {
        let before = self.m.shown_run(i).map(|r| r.id);
        match run {
            Some(id) => self.m.shown.insert(i, id),
            None => self.m.shown.remove(&i),
        };
        if self.m.shown_run(i).map(|r| r.id) != before {
            self.close_tray();
        }
    }

    pub fn select(&mut self, i: usize) {
        // Leaving unsaved edits asks first, in the header.
        if i != self.sel && self.m.dirty(self.sel) {
            self.leave_to = Some(i);
            return;
        }
        if i != self.sel && self.m.editing(self.sel) {
            self.m.cancel_edit(self.sel);
        }
        if i != self.sel {
            self.sel = i;
            self.close_tray();
            self.editing = false;
        }
    }

    /// Shows a tile selected, without opening its sidebar: one just added.
    pub fn select_tile(&mut self, id: String) {
        self.close_tray();
        self.reveal = Some(id.clone());
        self.selected = Some(id);
    }

    pub fn open(&mut self, t: Tray) {
        self.selected = None;
        self.buffers.clear();
        self.invalid.clear();
        self.per_item.clear();
        self.raw_open = None;
        self.confirm_remove = None;
        self.focus_done = false;
        self.pick = None;
        self.rtab = None;
        let (Tray::Question { step } | Tray::Step { id: step, .. }) = &t;
        self.reveal = Some(step.clone());
        // It slides in from shut; switching tiles with it open doesn't.
        if self.tray.is_none() {
            self.side_t0 = Some((self.now, true));
        }
        self.side_last = Some(t.clone());
        self.tray = Some(t);
    }

    /// Starts editing the shown recipe's structure, on a draft.
    pub fn begin_edit(&mut self) {
        self.m.begin_edit(self.sel);
        self.editing = true;
    }

    /// Ends editing, saving or dropping the draft, and goes where you were
    /// going if you'd picked another recipe.
    pub fn end_edit(&mut self, save: bool) {
        if save {
            self.m.save_edit(self.sel);
        } else {
            self.m.cancel_edit(self.sel);
        }
        self.editing = false;
        self.confirm_remove = None;
        self.confirm_delete = None;
        self.delete_refused = None;
        self.popover = None;
        if let Some(j) = self.leave_to.take() {
            self.select(j);
        }
    }

    pub fn close_tray(&mut self) {
        self.confirm_remove = None;
        if let Some(Tray::Question { step }) = &self.tray {
            let run = self.m.shown_run(self.sel).map_or(0, |r| r.id);
            self.dismissed.insert((self.sel, run, step.clone()));
        }
        if self.tray.is_some() {
            self.side_t0 = Some((self.now, false));
        }
        self.tray = None;
        self.buffers.clear();
    }

    /// What's being refused just now, if anything.
    pub fn per_item_for_tests(&self) -> &HashMap<String, Option<usize>> {
        &self.per_item
    }

    pub fn flash_text(&self) -> Option<&str> {
        self.flash.as_ref().map(|f| f.0.as_str())
    }

    /// Says why something was refused, as a sentence, by its cause: in
    /// the open sidebar's sentence, else in the header. It stays until
    /// the next click.
    pub(crate) fn refuse(&mut self, ctx: &egui::Context, why: impl Into<String>) {
        self.flash = Some((sentence(&why.into()), ctx.input(|i| i.time)));
    }

    /// A tile was clicked: the sidebar opens on it, whatever its state
    /// (its settings before a run, its run during and after, the question
    /// it's asking); shut if it was already open.
    fn tile_clicked(&mut self, id: String) {
        let open_here = matches!(&self.tray, Some(Tray::Step { id: x, .. } | Tray::Question { step: x }) if *x == id);
        if open_here {
            self.close_tray();
            return;
        }
        let snap = self.m.snapshot(self.sel);
        if snap.question.as_ref().is_some_and(|(s, _)| *s == id) {
            let run = self.m.shown_run(self.sel).map_or(0, |r| r.id);
            self.dismissed.remove(&(self.sel, run, id.clone()));
            self.open(Tray::Question { step: id });
            return;
        }
        let focus = self.m.engine.missing(&self.m.recipes[self.sel]).into_iter().find(|m| m.step == id).map(|m| m.key.to_owned());
        self.open(Tray::Step { id, focus });
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if self.m.ctx.is_none() {
            self.m.ctx = Some(ctx.clone());
            // A sidebar opening can take a pass more to lay out.
            ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(3).unwrap());
        }
        if self.appearance == Appearance::System {
            if let Some(t) = ctx.system_theme() {
                self.dark = t == egui::Theme::Dark;
            }
        }
        if self.applied != Some(self.dark) {
            theme::apply(&ctx, &Pal::get(self.dark));
            self.applied = Some(self.dark);
            self.motion_applied = None;
        }
        // Motion: on unless reduced, and never while a screenshot's clock
        // is pinned. Off, egui's own fades go too.
        let moving = self.motion && self.clock.is_none();
        crate::motion::set(moving);
        if self.motion_applied != Some(moving) {
            ctx.all_styles_mut(|s| s.animation_time = if moving { 1.0 / 12.0 } else { 0.0 });
            self.motion_applied = Some(moving);
        }
        self.now = ctx.input(|i| i.time);

        // The focus ring shows for the keyboard: Tab or an arrow turns it
        // on, a pointer press off.
        let (keys, press) = ctx.input(|i| {
            let keys = [Key::Tab, Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight].iter().any(|k| i.key_pressed(*k));
            (keys, i.pointer.any_pressed())
        });
        if press {
            crate::widgets::set_kb_focus(&ctx, false);
        } else if keys {
            crate::widgets::set_kb_focus(&ctx, true);
        }
        self.m.poll();
        // Closing mid-rip or with a disc held would lose it: ask first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_ok && self.ripping().is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_quit = true;
        }
        if self.confirm_quit && self.ripping().is_none() {
            self.confirm_quit = false;
        }
        while let Ok((i, id, key, path)) = self.picked.1.try_recv() {
            if let Err(e) = self.m.set_param(i, &id, &key, path.into()) {
                self.refuse(&ctx, e.to_string());
            }
        }
        if self.sel >= self.m.recipes.len() {
            self.sel = 0;
        }
        // A question opens the sidebar by itself when it's asked, even over
        // a step left open (its settings are locked now); once. Another
        // tile you open after stays open. It goes when the question does.
        let snap = self.m.snapshot(self.sel);
        match (&snap.question, &self.tray) {
            (Some((step, _)), t) if !matches!(t, Some(Tray::Question { step: s }) if s == step) => {
                let run = self.m.shown_run(self.sel).map_or(0, |r| r.id);
                let key = (self.sel, run, step.clone());
                if !self.dismissed.contains(&key) && (self.asked.insert(key) || t.is_none()) {
                    self.open(Tray::Question { step: step.clone() });
                }
            }
            (None, Some(Tray::Question { .. })) => {
                self.tray = None;
                self.side_t0 = Some((self.now, false));
            }
            _ => {}
        }
        // Esc undoes the innermost thing: a field being typed in reverts
        // (the field itself sees to that), the popover closes (its own),
        // a confirm keeps, then the sidebar closes.
        let typing = self.was_typing || ctx.text_edit_focused();
        if self.popover.is_none() && !typing && ctx.input(|i| i.key_pressed(Key::Escape)) && self.naming.is_none() {
            if self.confirm_quit {
                self.confirm_quit = false;
            } else if self.leave_to.is_some() {
                self.leave_to = None;
            } else if self.confirm_delete.is_some() {
                self.confirm_delete = None;
            } else if self.confirm_remove.is_some() {
                self.confirm_remove = None;
            } else {
                self.close_tray();
                self.selected = None;
            }
        }
        // Undo and redo while editing, unless a text box has the keys.
        if self.editing && !ctx.text_edit_focused() {
            let redo = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Z);
            let undo = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::Z);
            if ctx.input_mut(|i| i.consume_shortcut(&redo)) {
                self.m.redo(self.sel);
                self.close_tray();
            } else if ctx.input_mut(|i| i.consume_shortcut(&undo)) {
                self.m.undo(self.sel);
                self.close_tray();
            }
        }
        let p = Pal::get(self.dark);
        let t = self.clock.unwrap_or_else(|| ctx.input(|i| i.time));

        // The window's own frame: rounded unless maximised; its title for
        // the taskbar names the recipe.
        self.round = chrome::round(&ctx);
        let r = self.round as u8;
        let title = chrome::title(&self.m.recipes[self.sel].name);
        if self.title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        egui::Panel::left("recipes")
            .exact_size(232.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.surface).stroke(Stroke::new(1.0, p.line)).corner_radius(egui::CornerRadius { nw: r, sw: r, ne: 0, se: 0 }))
            .show(ui, |ui| self.sidebar(ui, &p, t));
        self.title_field = None;
        let header = egui::Panel::top("header")
            .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 18, right: 8, top: 8, bottom: 8 }).corner_radius(egui::CornerRadius { ne: r, ..Default::default() }))
            .show_separator_line(true)
            .show(ui, |ui| {
                // Its words are part of the bar you grab, not text to select.
                ui.style_mut().interaction.selectable_labels = false;
                // Its empty parts move the window; the window's buttons
                // sit at its right.
                chrome::drag_area(ui, ui.max_rect().expand2(vec2(18.0, 8.0)), ui.scope_id().with("drag-header"));
                let full = ui.max_rect();
                let split = full.right() - 3.0 * 36.0 - 10.0;
                let right = Rect::from_min_max(pos2(split, full.top()), full.max);
                let left = Rect::from_min_max(full.min, pos2(split, full.bottom()));
                ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(L::right_to_left(Align::Center)), |ui| {
                    ui.set_min_height(34.0);
                    chrome::buttons(ui, &p);
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| self.header(ui, &p));
            });
        self.band.push(header.response.rect);
        // The sidebar: shut when its step has gone (undo, Remove).
        if let Some(Tray::Step { id, .. }) = &self.tray {
            if !self.m.recipes[self.sel].root.ids().contains(&id.as_str()) {
                self.close_tray();
            }
        }
        // About a third of the window; a question
        // that compares two lists wider.
        let win = ctx.content_rect().width();
        // A small window keeps 320 of canvas, the sidebar at least 360.
        let mut full = (win * 0.35).clamp(SIDE_W, SIDE_MAX).min(win - 232.0 - 320.0).max(360.0);
        if crate::tray::wants_wide(self) {
            full = full.max(SIDE_WIDE.min(win - 232.0 - 420.0));
        }
        self.side_target = full;
        let glide = crate::motion::on() && self.side_w >= 1.0;
        self.side_full = ctx.animate_value_with_time(ui.scope_id().with("side-width"), full, if glide { 0.18 } else { 0.0 });
        // It slides in from the right, 180 ms; at once without motion.
        let k = match self.side_t0 {
            Some((t0, opening)) => {
                let k = crate::motion::cubic_out(crate::motion::since(&ctx, t0, 0.18));
                if k >= 1.0 {
                    self.side_t0 = None;
                }
                if opening {
                    k
                } else {
                    1.0 - k
                }
            }
            None => self.tray.is_some() as u8 as f32,
        };
        self.side_w = (self.side_full * k).round();
        if self.side_w >= 1.0 {
            egui::Panel::right("sidebar")
                .exact_size(self.side_w)
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(p.surface).corner_radius(egui::CornerRadius { se: r, ..Default::default() }))
                .show(ui, |ui| {
                    let snap = self.m.snapshot(self.sel);
                    crate::tray::show(self, ui, &p, &snap)
                });
        } else {
            self.side_last = None;
        }
        // The snapshot again: the list may have just picked another recipe.
        let had = self.tray.is_some();
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let snap = self.m.snapshot(self.sel);
            self.stage(ui, &p, t, &snap)
        });
        // A click on the canvas opened or shut the sidebar: lay the frame
        // out again with it, so it's never a frame late.
        if self.tray.is_some() != had {
            ctx.request_discard("the sidebar opened or shut");
        }

        // Spinners turn while anything is busy; otherwise egui sleeps until
        // a run changes (the watcher in Model::start) or you do something.
        chrome::frame(&ctx, &p);
        chrome::band(&ctx, &std::mem::take(&mut self.band), &self.title_field.into_iter().collect::<Vec<_>>());
        // A text field has the keys (not a tile or a button the keyboard
        // has reached).
        self.was_typing = ctx.text_edit_focused();

        if !self.m.finding.is_empty() || self.m.runs.values().flatten().any(|r| r.snap.steps.values().any(|s| s.state == StepState::Busy)) {
            tick(&ctx);
        }
        // A refusal goes with the next click after it.
        if let Some((_, at)) = &self.flash {
            if ctx.input(|i| i.pointer.any_pressed() && i.time > *at) {
                self.flash = None;
            }
        }
    }

    /// A recipe with a disc going through or held, if any.
    pub fn ripping(&self) -> Option<usize> {
        (0..self.m.recipes.len()).find(|&j| self.m.ripping(j))
    }

    fn sidebar(&mut self, ui: &mut Ui, p: &Pal, t: f64) {
        // Its top row, beside the canvas header, moves the window too.
        let top = Rect::from_min_size(ui.max_rect().min, vec2(ui.max_rect().width(), 50.0));
        chrome::drag_area(ui, top, ui.scope_id().with("drag-sidebar"));
        self.band.push(top);
        // "Recipes" is part of the bar you grab, not text to select.
        ui.style_mut().interaction.selectable_labels = false;
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(RichText::new("Recipes").font(font(15.0, 700)));
            ui.with_layout(L::right_to_left(Align::Center), |ui| {
                ui.add_space(10.0);
                if Btn::new("plus", "").name("New recipe").pad(vec2(6.0, 6.0)).show(ui, p).clicked() && self.naming.is_none() {
                    self.naming = Some(String::new());
                }
            });
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical().id_salt("recipe-list").show(ui, |ui| {
            // Checked each time round: a row can delete or add a recipe.
            let mut i = 0;
            while i < self.m.recipes.len() {
                self.recipe_row(ui, p, t, i);
                i += 1;
            }
            if self.naming.is_some() {
                self.naming_row(ui, p);
            }
        });
        ui.with_layout(L::bottom_up(Align::Min), |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                let all = [Appearance::System, Appearance::Light, Appearance::Dark];
                let at = Some(match self.appearance {
                    Appearance::System => 0,
                    _ if self.dark => 2,
                    _ => 1,
                });
                if let Some(k) = segmented(ui, p, "Appearance", &["System", "Light", "Dark"], at, true) {
                    self.set_appearance(all[k]);
                }
            });
        });
    }

    fn recipe_row(&mut self, ui: &mut Ui, p: &Pal, t: f64, i: usize) {
        let on = i == self.sel;
        let r = &self.m.recipes[i];
        let name = r.name.clone();
        let drive = self.m.drive(i);
        let (status, state) = if on && self.editing && !self.m.running(i) { ("Editing".to_owned(), StepState::Idle) } else { self.m.status_line(i) };
        // Orange is for you: waiting (a drive in use) is grey.
        let col = match state {
            StepState::Busy => p.busy,
            StepState::Needs | StepState::Hold => p.warn,
            StepState::Wait => p.muted,
            StepState::Bad => p.bad,
            StepState::Ok => p.ok,
            StepState::Idle => p.muted,
        };
        let icon = recipe_icon(r);
        // As the mockup's `.ri`: 8 px above and below the three lines, 10 px
        // in from the sides; the lines stacked by their real heights.
        const PAD: egui::Vec2 = vec2(10.0, 8.0);
        let w = 216.0 - PAD.x - 26.0 - 26.0;
        let drive_text = if drive.is_empty() { "no drive".to_owned() } else { drive.clone() };
        let lines = [(name.clone(), font(14.5, 600), p.ink), (status.clone(), font(13.0, 400), col), (drive_text, font(12.0, 400), p.subtle)];
        // What it waits for you to do may take a second line.
        let held = self.m.held_back(i).is_some_and(|why| why == status);
        let galleys: Vec<_> = lines
            .into_iter()
            .enumerate()
            .map(|(k, (t, f, c))| {
                let mut job = egui::text::LayoutJob::single_section(t, egui::TextFormat::simple(f, c));
                job.wrap = egui::text::TextWrapping { max_rows: if k == 1 && held { 2 } else { 1 }, break_anywhere: !(k == 1 && held), ..egui::text::TextWrapping::truncate_at_width(w) };
                ui.painter().layout_job(job)
            })
            .collect();
        // Each line in a box of the mockup's line-height (1.45).
        let boxes = [14.5 * 1.45, 13.0 * 1.45 * galleys[1].rows.len().max(1) as f32, 12.0 * 1.45];
        let height = boxes.iter().sum::<f32>() + 2.0 * PAD.y;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(216.0, height), Sense::click());
            let label = [name.as_str(), status.as_str(), drive.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
            resp.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::ListItem, true, on, &label));
            let pt = ui.painter();
            if on {
                pt.rect(rect, 8.0, p.raised, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
            } else if resp.hovered() {
                pt.rect_filled(rect, 8.0, p.bg);
            }
            let x = rect.left() + PAD.x;
            pt.extend(kind_icon(icon, pos2(x, rect.top() + PAD.y + (boxes[0] - 17.0) / 2.0), 17.0, if on { p.ink } else { p.muted }));
            let tx = x + 26.0;
            let mut y = rect.top() + PAD.y;
            for (g, b) in galleys.into_iter().zip(boxes) {
                let h = g.size().y;
                pt.galley(pos2(tx, y + (b - h) / 2.0), g, p.ink);
                y += b;
            }
            pt.extend(state_icon(p, state, pos2(rect.right() - 18.0, rect.center().y), 1.0, t));
            if resp.clicked() {
                self.delete_refused = None;
                self.select(i);
                self.show_run(i, None);
            }
        });
        ui.add_space(2.0);
        self.earlier_rows(ui, p, t, i);
    }

    /// Earlier discs of a recipe still encoding or holding: one line each
    /// under its row, the album and what it's doing.
    fn earlier_rows(&mut self, ui: &mut Ui, p: &Pal, t: f64, i: usize) {
        let rows: Vec<(u64, String, String, StepState)> = self
            .m
            .earlier(i)
            .into_iter()
            .map(|r| {
                let (line, state) = crate::model::run_line(&r.snap, true);
                (r.id, r.album(), line, state)
            })
            .collect();
        let shown = (self.sel == i && !self.m.shows_newest(i)).then(|| self.m.shown_run(i).map(|r| r.id)).flatten();
        for (id, album, line, state) in rows {
            let col = match state {
                StepState::Busy => p.busy,
                StepState::Needs | StepState::Hold => p.warn,
                StepState::Bad => p.bad,
                _ => p.muted,
            };
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(216.0, 24.0), Sense::click());
                let on = shown == Some(id);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::ListItem, true, on, format!("{album}, {line}")));
                let pt = ui.painter();
                if on {
                    pt.rect(rect, 6.0, p.raised, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
                } else if resp.hovered() {
                    pt.rect_filled(rect, 6.0, p.bg);
                }
                // Under the recipe's name, with a small state icon.
                pt.extend(state_icon(p, state, pos2(rect.left() + 18.0, rect.center().y), 0.75, t));
                // The status in full on the right; the album cut short
                // before it if need be.
                let x = rect.left() + 36.0;
                let st = pt.layout_no_wrap(line.clone(), font(12.5, 400), col);
                let sx = rect.right() - 8.0 - st.size().x;
                let mut job = egui::text::LayoutJob::single_section(album.clone(), egui::TextFormat::simple(font(12.5, 500), p.ink));
                job.wrap = egui::text::TextWrapping::truncate_at_width((sx - 8.0 - x).max(20.0));
                let g = pt.layout_job(job);
                pt.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, p.ink);
                pt.galley(pos2(sx, rect.center().y - st.size().y / 2.0), st, col);
                if resp.clicked() {
                    self.select(i);
                    self.show_run(i, Some(id));
                }
            });
            ui.add_space(2.0);
        }
    }

    /// Deletes a recipe; the next one along is shown.
    fn delete_recipe(&mut self, i: usize) {
        if let Err(e) = self.m.delete_recipe(i) {
            self.delete_refused = Some((i, e.to_string()));
            return;
        }
        self.delete_refused = None;
        self.reindex(|k| match k.cmp(&i) {
            std::cmp::Ordering::Less => Some(k),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(k - 1),
        });
        self.tray = None;
        self.popover = None;
        self.editing = false;
        self.buffers.clear();
        self.sel = i.min(self.m.recipes.len() - 1);
    }

    /// A copy of the recipe as saved, below it, shown in edit mode; the
    /// original leaves edit mode.
    fn duplicate(&mut self, i: usize) {
        self.m.cancel_edit(i);
        let j = self.m.duplicate_recipe(i);
        self.reindex(|k| Some(if k > i { k + 1 } else { k }));
        self.tray = None;
        self.popover = None;
        self.buffers.clear();
        self.sel = j;
        self.begin_edit();
    }

    /// Moves what the window keeps by recipe index; `None` drops it.
    fn reindex(&mut self, f: impl Fn(usize) -> Option<usize>) {
        self.views = std::mem::take(&mut self.views).into_iter().filter_map(|(k, v)| f(k).map(|k| (k, v))).collect();
        self.dismissed = std::mem::take(&mut self.dismissed).into_iter().filter_map(|(k, r, s)| f(k).map(|k| (k, r, s))).collect();
        if let Some(s) = f(self.sel) {
            self.sel = s;
        }
        self.leave_to = self.leave_to.and_then(&f);
    }

    fn naming_row(&mut self, ui: &mut Ui, p: &Pal) {
        let mut done = None;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            egui::Frame::new().fill(p.raised).stroke(Stroke::new(1.0, p.line)).corner_radius(8).inner_margin(Margin::symmetric(10, 10)).show(ui, |ui| {
                ui.set_width(196.0);
                let text = self.naming.as_mut().unwrap();
                let r = ui.add(egui::TextEdit::singleline(text).hint_text("Name").desired_width(190.0).font(font(14.5, 600)));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, "New recipe name"));
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                if r.lost_focus() {
                    done = Some(ui.input(|i| i.key_pressed(Key::Enter)) && !text.trim().is_empty());
                }
            });
        });
        match done {
            Some(true) => {
                let name = self.naming.take().unwrap();
                let i = self.m.new_recipe(&name);
                self.select(i);
                self.begin_edit();
                self.open(Tray::Step { id: "drive".into(), focus: Some("drive".into()) });
            }
            Some(false) => self.naming = None,
            None => {}
        }
    }

    fn header(&mut self, ui: &mut Ui, p: &Pal) {
        let i = self.sel;
        let running = self.m.running(i);
        let t = self.clock.unwrap_or_else(|| ui.input(|i| i.time));

        ui.horizontal(|ui| {
            ui.set_min_height(34.0);
            ui.spacing_mut().item_spacing.x = 12.0;
            if self.editing && !running {
                // The name, edited in place: part of the draft, so Save and
                // Cancel cover it.
                let name = self.m.recipes[i].name.clone();
                let buf = self.buffers.entry("title".into()).or_insert_with(|| name.clone());
                // As wide as the name, plus its padding and room for the
                // cursor, within sensible bounds: never clipped.
                let w = title_width(ui, buf);
                let r = ui.add(egui::TextEdit::singleline(buf).font(font(17.0, 700)).desired_width(w).margin(vec2(8.0, 3.0)));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, "Recipe name"));
                // Dragging in it selects its text, not the window.
                self.title_field = Some(r.rect);
                if r.lost_focus() {
                    let text = self.buffers.remove("title").unwrap_or_default();
                    if !crate::widgets::escaped(ui) {
                        self.m.rename_draft(i, &text);
                    }
                } else if !r.has_focus() {
                    self.buffers.remove("title");
                }
            } else {
                ui.label(RichText::new(&self.m.recipes[i].name).font(font(17.0, 700)));
            }
            let drive = self.m.drive(i);
            let album = self.m.shown_run(i).and_then(|r| r.run.context().get(drt_engine::Subject::Disc, "album").map(str::to_owned));
            let sub = match (album, drive.is_empty()) {
                (Some(a), false) => format!("{a}, in {drive}"),
                (Some(a), true) => a,
                (None, false) => drive.clone(),
                (None, true) => "new".into(),
            };
            ui.label(RichText::new(sub).font(font(13.0, 400)).color(p.muted));
            let missing = self.m.engine.missing(&self.m.recipes[i]).len();
            if self.m.unplugged(i) {
                // Grey: nothing to do here but plug it in.
                pill(ui, p, "Not connected", Tone::Plain, None, t);
            } else if let Some(why) = self.m.held_back(i).filter(|_| self.m.shows_newest(i)) {
                // What it waits for before the next disc, in so many words.
                pill(ui, p, &why, Tone::Warn, Some(StepState::Needs), t);
            } else if let Some(r) = self.m.shown_run(i) {
                let (line, state) = crate::model::run_line(&r.snap, !self.m.shows_newest(i));
                // The one pill, as in the sidebar: its state icon and words.
                let text = match state {
                    StepState::Needs => "Needs you".to_owned(),
                    StepState::Hold => "Holding".to_owned(),
                    StepState::Wait => "Waiting".to_owned(),
                    StepState::Ok => "Done".to_owned(),
                    StepState::Busy => "Running".to_owned(),
                    StepState::Bad | StepState::Idle => line,
                };
                let tone = if state == StepState::Wait { Tone::Plain } else { Tone::of(state) };
                pill(ui, p, &text, tone, Some(state), t);
            } else if self.m.finding.contains_key(&i) {
                pill(ui, p, "Looking up", Tone::Busy, Some(StepState::Busy), t);
            } else if missing == 0 && self.m.drive_busy(i) {
                pill(ui, p, "Drive busy", Tone::Plain, Some(StepState::Wait), t);
            } else if missing > 0 {
                pill(ui, p, &format!("{missing} to set"), Tone::Warn, Some(StepState::Needs), t);
            } else if let Some(why) = self.m.check_word(i) {
                pill(ui, p, &crate::tray::cap_pub(why), Tone::Plain, None, t);
            }
            if let Some((why, _)) = self.flash.as_ref().filter(|_| self.tray.is_none()) {
                chip(ui, why, p.bad, p.bad_soft);
            }
            if let Some(why) = self.m.tools_missing() {
                chip(ui, &why, p.bad, p.bad_soft);
            }
            // Nothing being kept: says so until it's fixed, with the folder.
            if let Some(why) = self.m.not_saving() {
                chip(ui, &why, p.bad, p.bad_soft);
                if Btn::new("folder-link", "Open folder").look(Look::Quiet).font(font(13.0, 500)).show(ui, p).clicked() {
                    let dir = self.m.store.dir.clone();
                    self.m.open(&dir);
                }
            }
            ui.with_layout(L::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if self.confirm_quit {
                    // Quit asks here, as Delete does.
                    let what = self.ripping().map(|j| self.m.recipes[j].name.clone()).unwrap_or_default();
                    if crate::tray::danger_button(ui, p, "Quit") {
                        self.confirm_quit = false;
                        self.quit_ok = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if Btn::new("", "Keep ripping").show(ui, p).clicked() {
                        self.confirm_quit = false;
                    }
                    ui.label(RichText::new(format!("Quit? {what} is ripping")).font(font(14.0, 600)).color(p.bad));
                    return;
                }
                if self.editing && !running && self.leave_to.is_some() {
                    // Leaving with unsaved edits: stay, or go with them saved
                    // or not.
                    let name = self.m.recipes[i].name.clone();
                    if Btn::new("done", "Save").look(Look::Primary).show(ui, p).clicked() {
                        self.end_edit(true);
                        return;
                    }
                    if Btn::new("", "Discard").show(ui, p).clicked() {
                        self.end_edit(false);
                        return;
                    }
                    if Btn::new("", "Stay").look(Look::Quiet).show(ui, p).clicked() {
                        self.leave_to = None;
                    }
                    ui.label(RichText::new(format!("Save changes to {name}?")).font(font(14.0, 600)).color(p.warn));
                    return;
                }
                if self.editing && !running && self.confirm_delete == Some(i) {
                    // Delete asks here, once.
                    let name = self.m.recipes[i].name.clone();
                    let g = ui.painter().layout_no_wrap("Delete".into(), font(14.0, 500), p.raised);
                    let (r, resp) = ui.allocate_exact_size(g.size() + vec2(28.0, 14.0), Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, "Delete"));
                    ui.painter().rect_filled(r, 6.0, if resp.hovered() { mix(p.ink, 0.12, p.bad) } else { p.bad });
                    let ink = if p.dark { p.bg } else { egui::Color32::WHITE };
                    ui.painter().galley(r.center() - g.size() / 2.0, g, ink);
                    if Btn::new("", "Don't delete").show(ui, p).clicked() {
                        self.confirm_delete = None;
                    }
                    ui.label(RichText::new(format!("Delete {name}?")).font(font(14.0, 600)).color(p.bad));
                    if resp.clicked() {
                        self.confirm_delete = None;
                        self.delete_recipe(i);
                    }
                    return;
                }
                if self.editing && !running {
                    // Editing a draft: Save or Cancel; the recipe itself;
                    // back and forward.
                    if Btn::new("done", "Save").look(Look::Primary).show(ui, p).clicked() {
                        self.end_edit(true);
                        return;
                    }
                    if Btn::new("", "Cancel").show(ui, p).clicked() {
                        self.end_edit(false);
                        return;
                    }
                    ui.add_space(6.0);
                    if Btn::new("trash", "Delete").look(Look::Quiet).fg(p.bad).show(ui, p).clicked() {
                        match self.m.cant_delete(i) {
                            Some(why) => self.delete_refused = Some((i, why.to_owned())),
                            None => self.confirm_delete = Some(i),
                        }
                    }
                    // A seeded recipe, back as it was built in (undoable).
                    if self.m.differs_from_builtin(i) && Btn::new("", "Reset to original").look(Look::Quiet).show(ui, p).clicked() {
                        self.m.reset_to_builtin(i);
                        self.close_tray();
                    }
                    // A copy of the recipe as saved; with unsaved changes,
                    // save or cancel them first.
                    let dirty = self.m.dirty(i);
                    let d = Btn::new("", "Duplicate").look(Look::Quiet).enabled(!dirty).show(ui, p);
                    if dirty {
                        d.clone().on_disabled_hover_text("Save or cancel first");
                    }
                    if d.clicked() {
                        self.duplicate(i);
                        return;
                    }
                    if let Some((_, why)) = self.delete_refused.as_ref().filter(|d| d.0 == i) {
                        ui.label(RichText::new(format!("Can't delete: {why}")).font(font(13.0, 400)).color(p.muted));
                    }
                    ui.add_space(6.0);
                    if Btn::new("redo", "").name("Redo").look(Look::Quiet).enabled(self.m.can_redo(i)).show(ui, p).on_hover_text("Ctrl+Shift+Z").clicked() {
                        self.m.redo(i);
                        self.close_tray();
                    }
                    if Btn::new("undo", "").name("Undo").look(Look::Quiet).enabled(self.m.can_undo(i)).show(ui, p).on_hover_text("Ctrl+Z").clicked() {
                        self.m.undo(i);
                        self.close_tray();
                    }
                } else {
                    if !self.m.shows_newest(i) {
                        // An earlier disc: back to the one in the drive.
                        if Btn::new("", "Back to current disc").look(Look::Quiet).show(ui, p).clicked() {
                            self.show_run(i, None);
                        }
                    } else if running {
                        let b = Btn::new("stop", "Stop").fg(p.bad).show(ui, p);
                        // Held back, Stop only calls the next disc off.
                        let b = if self.m.next_waits(i) { b.on_hover_text("No next disc; this one finishes") } else { b };
                        if b.clicked() {
                            self.m.stop(i);
                        }
                    } else {
                        let why = self.m.cant_start(i);
                        let b = Btn::new("play", "Start").look(Look::Primary).enabled(why.is_none()).show(ui, p);
                        if let Some(why) = &why {
                            b.clone().on_disabled_hover_text(why.as_str());
                        }
                        if b.clicked() {
                            self.close_tray();
                            self.dismissed.retain(|(r, _, _)| *r != i);
                            if let Err(e) = self.m.start(i) {
                                self.refuse(ui.ctx(), e.to_string());
                            }
                        }
                    }
                    if Btn::new("pencil", "Edit").enabled(!running).show(ui, p).clicked() {
                        self.begin_edit();
                    }
                }
                // The run's working folder, once it has one.
                if let Some(work) = self.m.shown_run(i).map(|r| r.run.work().to_owned()).filter(|w| w.exists()) {
                    if Btn::new("folder-link", "Working files").look(Look::Link).show(ui, p).on_hover_text(work.display().to_string()).clicked() {
                        self.m.open(&work);
                    }
                }
            });
        });
    }

    fn stage(&mut self, ui: &mut Ui, p: &Pal, t: f64, snap: &drt_engine::Snapshot) {
        let rect = ui.max_rect();
        self.canvas = rect;
        let i = self.sel;
        let lay = self.layout(i);
        // The canvas's width once the sidebar has finished sliding: a tile
        // it would cover glides out from under it.
        let final_w = rect.width() + self.side_w - if self.tray.is_some() { self.side_target } else { 0.0 };
        if let Some(id) = self.reveal.take() {
            if let Some(tp) = lay.tile(&id) {
                let mut to = self.glide.filter(|g| g.0 == i).map_or(self.views_of(i), |g| g.2);
                to.reveal(tp.rect, vec2(final_w, rect.height()));
                self.glide_to(i, to, 0.18);
            }
        }
        self.watch_cues(i, snap);
        // Above the zoom bar, which sits over the canvas's foot.
        self.follow_step(i, &lay, vec2(final_w, rect.height() - 40.0));
        self.step_glide(i);
        let open = match &self.tray {
            Some(Tray::Step { id, .. }) => Some(id.clone()),
            Some(Tray::Question { step }) => Some(step.clone()),
            None => self.selected.clone(),
        };
        let plus_open = self.popover.as_ref().map(|p| p.at.clone());
        let editing = self.editing && !self.m.running(i);
        crate::tray::take_covers(self, ui.ctx());
        let cards = self.cards(ui.ctx(), snap);
        // Red: what a Remove would take, or the tile that keeps the recipe
        // from starting because it doesn't fit.
        let doomed: Vec<String> = match &self.confirm_remove {
            Some(c) => c.1.clone(),
            None => self.m.misfit(i).into_iter().collect(),
        };
        if editing && self.dead_pluses.as_ref().is_none_or(|(root, _)| *root != self.m.recipes[i].root) {
            let root = self.m.recipes[i].root.clone();
            let dead = self.m.spots(i).into_iter().filter(|at| self.m.offers(i, at).is_empty()).collect();
            self.dead_pluses = Some((root, dead));
        }
        let none = std::collections::HashSet::new();
        let dead = self.dead_pluses.as_ref().map_or(&none, |(_, d)| d);
        let view = self.views.entry(i).or_insert(View { zoom: 1.0, pan: canvas::MARGIN });
        // The window's bottom-right corner is the sidebar's while it's out.
        let round = if self.side_w >= 1.0 { 0.0 } else { self.round };
        let root = self.m.recipes[i].root.clone();
        let marks = Marks { open: open.as_deref(), editing, plus_open: plus_open.as_ref(), doomed: &doomed, round, cues: &self.cues, dead, cards: &cards, t, root: &root };
        let before = *view;
        let clicked = canvas::show(ui, rect, view, &lay, snap, p, marks);
        // A drag or a scroll takes the camera back from a glide, and from
        // Follow.
        if *view != before {
            self.glide = None;
            self.follow = false;
        }
        match clicked {
            Some(Click::Tile(id)) => self.tile_clicked(id),
            Some(Click::Cover(id)) => {
                if let Some(path) = snap.steps.get(&id).and_then(|v| v.record.files.iter().map(|f| drt_steps::real::expand(&f.path)).find(|f| f.is_absolute())) {
                    self.m.open(&path);
                }
            }
            // Empty canvas: the sidebar goes, and any tile only selected.
            Some(Click::Empty) => {
                self.close_tray();
                self.selected = None;
            }
            Some(Click::Plus(at)) => {
                if plus_open.as_ref() == Some(&at) {
                    self.popover = None;
                } else {
                    // After a fork, a new branch is what you'd want.
                    let branch = self.m.forks_at(self.sel, &at);
                    let v = self.views_of(i);
                    self.popover = Some(crate::popover::Popover::new(at, branch, (v.zoom, v.pan)));
                    self.selected = None;
                }
            }
            None => {}
        }
        self.zoom_controls(ui, p, rect, &lay, pos2(rect.left() + 12.0, rect.bottom() - 12.0));
        if self.popover.is_some() {
            crate::popover::show(self, ui, p, rect, &lay);
        } else {
            self.popover_rect = Rect::NOTHING;
        }
    }

    /// What each card shows besides its step's view: the settings marked
    /// for it, as label and value (yours with a dot; locked while
    /// ripping), and Cover art's picture.
    pub fn cards(&mut self, ctx: &egui::Context, snap: &drt_engine::Snapshot) -> std::collections::HashMap<String, CardExtra> {
        let i = self.sel;
        let locked = self.m.locked(i, Scope::Recipe) || !self.m.shows_newest(i);
        let builtin = self.m.builtin(i);
        let mut out = std::collections::HashMap::new();
        for (id, v) in &snap.steps {
            let Some(info) = self.m.info(i, id) else { continue };
            let sets: Vec<(&drt_engine::ParamSpec, Setting<serde_json::Value>)> = info.params.iter().map(|s| (s, self.m.shown_param(i, id, s))).collect();
            let flow = self.m.shown_flow(i, id);
            let get = |k: &str| sets.iter().find(|(s, _)| s.key == k).and_then(|(s, set)| s.resolve_at(set, flow.as_ref()));
            // Changed by you, from what the recipe came with.
            let yours = |k: &str| sets.iter().find(|(s, _)| s.key == k).is_some_and(|(s, _)| self.m.is_yours(i, id, s, builtin.as_ref()));
            let settings = sets
                .iter()
                .filter(|(s, _)| s.on_card.is_some() && s.applies(get))
                .filter(|(s, _)| !(s.main_path_only && self.m.on_a_path(i, id)))
                .filter(|(s, _)| !(s.scope == Scope::Drive && self.m.drive(i).is_empty()))
                .take(2)
                .map(|(s, _)| {
                    let on = s.on_card.unwrap();
                    let value =
                        match on.value {
                            Some(f) => f(&get),
                            // A choice's word reads in lower case (*joint*); a name or a path as it is.
                            None => get(s.key)
                                .map(|v| {
                                    if matches!(s.kind, drt_engine::ParamKind::Choice(_)) {
                                        card_value(label_head(s.choice_label(&crate::model::value_text(&v))))
                                    } else {
                                        crate::model::value_text(&v)
                                    }
                                })
                                .unwrap_or_default(),
                        };
                    let value = if value.is_empty() { "—".to_owned() } else { value };
                    crate::card::CardSetting { label: on.label.to_owned(), value, yours: yours(s.key) || on.with.iter().any(|k| yours(k)), path: s.kind == drt_engine::ParamKind::Folder, help: s.help }
                })
                .collect();
            // Cover art: the picture it made.
            let picture = if v.step == "cover-art" {
                match v.record.files.iter().map(|f| drt_steps::real::expand(&f.path)).find(|f| f.is_absolute()) {
                    Some(path) => {
                        crate::tray::load_cover(self, ctx, &path);
                        match self.covers.get(&path) {
                            Some(crate::tray::Cover::Ready(t, size)) => crate::card::Picture::Ready(t.id(), *size),
                            _ => crate::card::Picture::Blank,
                        }
                    }
                    None => crate::card::Picture::Blank,
                }
            } else {
                crate::card::Picture::None
            };
            let asking = snap.question.as_ref().is_some_and(|(s, _)| s == id);
            out.insert(id.clone(), CardExtra { settings, locked, retries: info.retries.iter().any(|r| !matches!(r.id, "keep" | "leave_out")), asking, picture });
        }
        out
    }

    fn zoom_controls(&mut self, ui: &mut Ui, p: &Pal, canvas: Rect, lay: &drt_layout::Layout, bottom_left: egui::Pos2) {
        let cover = 0.0;
        let i = self.sel;
        egui::Area::new(ui.scope_id().with("zoom")).order(egui::Order::Middle).pivot(Align2::LEFT_BOTTOM).fixed_pos(bottom_left).show(ui.ctx(), |ui| {
            egui::Frame::new().fill(p.raised).stroke(Stroke::new(1.0, p.line)).corner_radius(8).inner_margin(Margin::symmetric(2, 1)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal(|ui| {
                    let (o, centre) = (canvas.min, canvas.center());
                    let f = font(12.5, 400);
                    // Each glides there (at once without motion).
                    let mut to = self.glide.filter(|g| g.0 == i).map_or(self.views_of(i), |g| g.2);
                    let z = to.zoom;
                    // Zooming by hand stops Follow.
                    if Btn::new("minus", "").name("Zoom out").look(Look::Quiet).font(f.clone()).show(ui, p).clicked() {
                        to.zoom_about(o, centre, z / 1.25);
                        self.glide_to(i, to, 0.14);
                        self.follow = false;
                    }
                    let pct = format!("{:.0}%", self.views_of(i).zoom * 100.0);
                    if Btn::new("", &pct).name("Actual size").look(Look::Quiet).font(f.clone()).show(ui, p).clicked() {
                        to.zoom_about(o, centre, 1.0);
                        self.glide_to(i, to, 0.24);
                        self.follow = false;
                    }
                    if Btn::new("plus", "").name("Zoom in").look(Look::Quiet).font(f.clone()).show(ui, p).clicked() {
                        to.zoom_about(o, centre, z * 1.25);
                        self.glide_to(i, to, 0.14);
                        self.follow = false;
                    }
                    // Its words where there's room; on a narrow canvas, the icon.
                    let words = if canvas.width() >= 360.0 { "Fit to window" } else { "" };
                    if Btn::new("fit", words).name("Fit to window").look(Look::Quiet).font(f.clone()).show(ui, p).clicked() {
                        to.fit(lay, vec2(canvas.width(), canvas.height() - cover));
                        self.glide_to(i, to, 0.24);
                    }
                    ui.add_space(4.0);
                    let tip = "Keep the step that's working, or waiting for you, in view";
                    let words = if canvas.width() >= 360.0 { "Follow" } else { "" };
                    if crate::widgets::switch(ui, p, "Follow", words, self.follow, f).on_hover_text(tip).clicked() {
                        self.set_follow(!self.follow);
                    }
                });
            });
        });
    }
}

/// The recipe's icon in the list: what its kind step reads.
fn recipe_icon(r: &drt_engine::Recipe) -> &'static str {
    let kinds: Vec<&str> = r
        .root
        .steps()
        .into_iter()
        .filter_map(|n| match n {
            drt_engine::Node::Step { step, .. } => Some(step.as_str()),
            _ => None,
        })
        .collect();
    if kinds.contains(&"audio-cd") {
        "disc-row"
    } else if kinds.contains(&"dvd") {
        "tv"
    } else if kinds.contains(&"game-disc") {
        "game"
    } else {
        "drive"
    }
}

/// The title field's width for `name`: its text, both sides' padding and a
/// cursor's room, at least 100 and at most 420.
/// A choice's word as a card shows it: *Joint* as *joint*, but *V0*,
/// *H.265* and *PlayStation* as they are.
/// A choice's label as a card has room for: the words before any
/// explanation (*Joint* of *Joint, best for the size*; *Large* of
/// *Large (1200 px)*).
pub fn label_head(label: &str) -> &str {
    label.split([',', '(']).next().map(str::trim).filter(|h| !h.is_empty()).unwrap_or(label)
}

pub fn card_value(v: &str) -> String {
    let mut c = v.chars();
    match c.next() {
        Some(f) if f.is_uppercase() && c.clone().all(|x| !x.is_uppercase() && !x.is_ascii_digit()) => f.to_lowercase().chain(c).collect(),
        _ => v.to_owned(),
    }
}

pub fn title_width(ui: &Ui, name: &str) -> f32 {
    let text = ui.painter().layout_no_wrap(name.to_owned(), font(17.0, 700), egui::Color32::WHITE).size().x;
    (text + 2.0 * 8.0 + 6.0).clamp(100.0, 420.0)
}

/// The engine's and the model's short refusals, as sentences.
pub fn sentence(why: &str) -> String {
    match why {
        "last step" => "A recipe needs at least its drive.".into(),
        "running" => "Not while a disc is going through.".into(),
        "no drive yet" => "Choose a drive first.".into(),
        "Locked while ripping" => "Settings are locked while ripping.".into(),
        w if w.ends_with('.') || w.ends_with('?') => w.into(),
        w => {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>() + ".").unwrap_or_default()
        }
    }
}

/// Another frame soon, for something that turns (a spinner, wires
/// marching): every 33 ms with the window focused, every 250 ms without;
/// none at all when nothing may move (a run's changes still wake the
/// window).
pub fn tick(ctx: &egui::Context) {
    if !crate::motion::on() {
        return;
    }
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    ctx.request_repaint_after(std::time::Duration::from_millis(if focused { 33 } else { 250 }));
}
