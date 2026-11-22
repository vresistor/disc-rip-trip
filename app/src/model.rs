//! The window's state without the window: the recipes, their runs, and
//! everything you can do to them. The UI calls these; tests can too.

use crate::store::Store;
use anyhow::{anyhow, bail, Result};
use drt_engine::{check, flows_in, Command, Engine, Flow, ItemState, Node, ParamKind, ParamSpec, Recipe, Run, Scope, Setting, Settings, Snapshot, Spot, StepInfo, StepState};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;

pub use drt_steps::real::redumper::CRAWLING_CARD;

/// A Rip's pace is kept by recipe, run and step.
type PaceKey = (usize, u64, String);

/// Opens a folder in the file manager; says why if it can't.
pub type OpenFolder = Arc<dyn Fn(&Path) -> std::result::Result<(), String> + Send + Sync>;
/// Asks for a folder, starting at the one given if any.
pub type PickFolder = Arc<dyn Fn(Option<PathBuf>) -> Option<PathBuf> + Send + Sync>;
/// Sends Notify's test: to this computer or not, the ntfy server, the
/// topic. Gives where it went, as [`drt_steps::real::tell::send_test`].
pub type SendTest = Arc<dyn Fn(bool, String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<&'static str>>> + Send>> + Send + Sync>;
/// Opens a drive's tray, by the drive's name; says why if it can't.
/// Blocks: called off the window's thread.
pub type OpenTray = Arc<dyn Fn(&str) -> std::result::Result<(), String> + Send + Sync>;
/// The optical drives connected just now, by name (cheap: sysfs).
pub type ListDrives = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// Everything the window is given rather than makes, so a test can give
/// it a simulated engine, a temporary folder and harmless hooks.
pub struct Deps {
    pub engine: Engine,
    pub store: Store,
    /// Drives to offer a new recipe.
    pub drives: Vec<String>,
    pub rt: tokio::runtime::Handle,
    pub open_folder: OpenFolder,
    pub pick_folder: PickFolder,
    /// Which drives are connected; asked every couple of seconds.
    pub connected: ListDrives,
    pub send_test: SendTest,
    /// Opens a drive's tray: [`drt_steps::real::open_tray`], or a stand-in.
    pub open_tray: OpenTray,
}

/// One disc going through a recipe.
pub struct Running {
    /// Unique across the window's life, so an earlier disc can be picked.
    pub id: u64,
    pub run: Run,
    rx: watch::Receiver<Snapshot>,
    pub snap: Snapshot,
    /// Whether this run's Next disc has already started the next one.
    repeated: bool,
    /// Whether it has been kept, once it ended.
    kept: bool,
    /// The recipe and the drive settings as the run started with them:
    /// what an earlier disc's sidebars show.
    pub recipe: Recipe,
    pub settings: Settings,
}

impl Running {
    /// The album, from the Choose tile once it has chosen.
    pub fn album(&self) -> String {
        self.snap
            .steps
            .values()
            .find(|v| v.step == "choose" && v.state == StepState::Ok)
            .map(|v| v.status.clone())
            .or_else(|| self.run.context().get(drt_engine::Subject::Disc, "album").map(str::to_owned))
            .unwrap_or_else(|| "A disc".into())
    }
}

pub struct Model {
    pub engine: Arc<Engine>,
    pub store: Store,
    pub recipes: Vec<Recipe>,
    /// Each recipe's runs, oldest first: the newest is the disc in the
    /// drive, the others earlier discs still encoding or holding.
    pub runs: HashMap<usize, Vec<Running>>,
    /// The run a recipe shows, when it isn't the newest.
    pub shown: HashMap<usize, u64>,
    next_id: u64,
    /// Recipes stopped by hand: they don't go on to the next disc.
    no_repeat: std::collections::HashSet<usize>,
    /// How many further discs a recipe may start by itself after a Start;
    /// `None` is no limit. The simulated window caps it, since a simulated
    /// drive never waits for a disc.
    pub repeat_limit: Option<usize>,
    /// The bundled tools that couldn't be found at start-up; said in the
    /// header. Empty in the flatpak, which has them all.
    pub missing_tools: Vec<&'static str>,
    auto_left: HashMap<usize, Option<usize>>,
    /// The disc before, as it ended: shown until the next disc goes in, so
    /// the graph doesn't empty the moment the tray opens.
    /// Which run that was, and how it last looked: followed while it
    /// finishes encoding and filing.
    before_disc: HashMap<usize, (u64, Snapshot)>,
    /// Recipes whose findable settings (a drive's offset) are being looked
    /// up, and whether by measuring.
    pub finding: HashMap<usize, bool>,
    /// Each recipe's last run, as kept (by name): what it showed and found,
    /// for after a restart, until a new run starts.
    pub last: HashMap<String, crate::store::Kept>,
    /// The drives connected, as last seen (`None` before the first look),
    /// and the watcher's news of a change.
    pub connected: Option<std::collections::HashSet<String>>,
    list_drives: ListDrives,
    /// The watcher's readings, each with when it looked.
    drives_rx: std::sync::mpsc::Receiver<(std::time::Instant, Vec<String>)>,
    /// When the drives applied last were looked at: an older reading,
    /// still queued, is stale.
    drives_at: Option<std::time::Instant>,
    /// The window, for the watcher to wake when a drive comes or goes.
    wake: Arc<std::sync::OnceLock<egui::Context>>,
    found_tx: std::sync::mpsc::Sender<(usize, bool, usize)>,
    found_rx: std::sync::mpsc::Receiver<(usize, bool, usize)>,
    /// Measures that found nothing, so the tile can say so.
    pub measure_failed: std::collections::HashSet<usize>,
    /// Recipes being edited: each as saved, while its draft is in
    /// `recipes`.
    drafts: HashMap<usize, Recipe>,
    undo: HashMap<usize, Vec<Node>>,
    redo: HashMap<usize, Vec<Node>>,
    pub drives: Vec<String>,
    pub rt: tokio::runtime::Handle,
    pub open_folder: OpenFolder,
    pub pick_folder: PickFolder,
    pub send_test: SendTest,
    /// The last save, folder-open or tray that failed, until one works.
    failed: Arc<std::sync::Mutex<Option<String>>>,
    open_tray_hook: OpenTray,
    /// Drives whose tray is being opened just now.
    opening: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// Set once the window exists, so runs can wake it.
    pub ctx: Option<egui::Context>,
    /// The settings as last saved, so ones a run finds (a drive's offset)
    /// are saved as soon as they appear.
    saved: drt_engine::Settings,
    /// Each Rip's pace as it reads (by recipe, run and step), and what it
    /// last came to: for its card and its sidebar.
    pace: std::sync::Mutex<HashMap<PaceKey, (crate::pace::Pace, Option<crate::pace::Reading>)>>,
}

impl Model {
    pub fn new(mut d: Deps) -> Model {
        // Seed only the built-in recipes these steps can run. An existing
        // file is left as it is.
        let reg = &d.engine.registry;
        let mut recipes = d.store.load_recipes(|| drt_steps::recipes::all().into_iter().filter(|r| check(&r.root, reg).is_ok()).collect());
        // Brought up to date, and saved once if anything changed: Library
        // is Folder now (old nodes keep what they were called), and a step
        // that held is a Hold step in front of it.
        // Every recipe, so no `any`: it would stop at the first.
        let mut changed = false;
        for r in recipes.iter_mut() {
            changed |= migrate(&mut r.root, reg) | r.upgrade();
        }
        if changed {
            if let Err(e) = d.store.save_recipes(&recipes) {
                eprintln!("couldn't save the recipes: {e:#}");
            }
        }
        if let Some(mut s) = d.store.load_settings() {
            // The ntfy topic and server were once one for the whole app;
            // each Notify has its own now, starting from that one.
            if ntfy_to_each_notify(&mut recipes, &mut s) {
                if let Err(e) = d.store.save_recipes(&recipes).and_then(|_| d.store.save_settings(&s)) {
                    eprintln!("couldn't save the recipes: {e:#}");
                }
            }
            *d.engine.settings.lock().unwrap() = s;
        }
        // Drives under the names recipes and settings already use.
        let mut known: Vec<String> = recipes.iter().map(Engine::drive_of).collect();
        known.extend(d.engine.settings.lock().unwrap().drives.keys().cloned());
        known.extend([drt_steps::recipes::BLU_RAY.to_owned(), drt_steps::recipes::DVD.to_owned()]);
        let drives = d.drives.iter().map(|n| known_name(n, &known)).collect();
        let (found_tx, found_rx) = std::sync::mpsc::channel();
        // Watch the drives on the runtime, every 2 s; only a change is
        // news, and only news wakes the window.
        let (drives_tx, drives_rx) = std::sync::mpsc::channel();
        let wake: Arc<std::sync::OnceLock<egui::Context>> = Arc::default();
        {
            let (list, wake) = (d.connected.clone(), wake.clone());
            d.rt.spawn(async move {
                let mut last: Option<Vec<String>> = None;
                loop {
                    let list = list.clone();
                    let at = std::time::Instant::now();
                    let Ok(mut now) = tokio::task::spawn_blocking(move || list()).await else { break };
                    now.sort();
                    if last.as_ref() != Some(&now) {
                        if drives_tx.send((at, now.clone())).is_err() {
                            break;
                        }
                        if let Some(ctx) = wake.get() {
                            ctx.request_repaint();
                        }
                        last = Some(now);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            });
        }
        let mut m = Model {
            engine: Arc::new(d.engine),
            store: d.store,
            recipes,
            runs: HashMap::new(),
            shown: HashMap::new(),
            next_id: 1,
            no_repeat: Default::default(),
            repeat_limit: None,
            missing_tools: Vec::new(),
            auto_left: HashMap::new(),
            before_disc: HashMap::new(),
            finding: HashMap::new(),
            connected: None,
            last: HashMap::new(),
            list_drives: d.connected,
            drives_rx,
            drives_at: None,
            wake,
            found_tx,
            found_rx,
            measure_failed: Default::default(),
            drafts: HashMap::new(),
            undo: HashMap::new(),
            redo: HashMap::new(),
            drives,
            rt: d.rt,
            open_folder: d.open_folder,
            failed: Default::default(),
            pick_folder: d.pick_folder,
            send_test: d.send_test,
            open_tray_hook: d.open_tray,
            opening: Default::default(),
            ctx: None,
            saved: Default::default(),
            pace: Default::default(),
        };
        // Which drives are here, before anything looks them up.
        m.refresh_drives();
        // Whatever can be looked up is looked up at once.
        for i in 0..m.recipes.len() {
            m.look_up(i, false);
        }
        m
    }

    /// Looks at which drives are connected now, at once (the watcher does
    /// this every 2 s by itself).
    pub fn refresh_drives(&mut self) {
        let at = std::time::Instant::now();
        let now = (self.list_drives)();
        self.drives_at = Some(at);
        self.apply_drives(now);
    }

    fn apply_drives(&mut self, now: Vec<String>) {
        // Under the names recipes and settings already use.
        let mut known: Vec<String> = self.recipes.iter().map(Engine::drive_of).collect();
        known.extend(self.engine.settings.lock().unwrap().drives.keys().cloned());
        known.extend(self.drives.iter().cloned());
        let now: std::collections::HashSet<String> = now.iter().map(|n| known_name(n, &known)).collect();
        let back: Vec<String> = match &self.connected {
            Some(was) => now.difference(was).cloned().collect(),
            None => vec![],
        };
        for d in &now {
            if !self.drives.contains(d) {
                self.drives.push(d.clone());
            }
        }
        self.connected = Some(now);
        // One that's come back: what was waiting on it is looked up now.
        for i in 0..self.recipes.len() {
            if back.contains(&self.drive(i)) {
                self.look_up(i, false);
            }
        }
    }

    /// What the shown run (or, after a restart, the last) found.
    pub fn shown_context(&self, i: usize) -> Option<drt_engine::Context> {
        match self.shown_run(i) {
            Some(r) => Some(r.run.context()),
            None => self.last.get(&self.recipes[i].name).map(|k| k.context.clone()),
        }
    }

    /// What a step's tool printed in the shown run (or the last).
    pub fn shown_raw(&self, i: usize, step: &str) -> Vec<String> {
        match self.shown_run(i) {
            Some(r) => r.run.raw(step),
            None => self.last.get(&self.recipes[i].name).and_then(|k| k.raw.get(step).cloned()).unwrap_or_default(),
        }
    }

    /// Whether a drive is connected (an unnamed one, or before the first
    /// look, counts as yes).
    pub fn is_connected(&self, drive: &str) -> bool {
        drive.is_empty() || self.connected.as_ref().is_none_or(|s| s.contains(drive))
    }

    /// Whether the recipe's drive isn't there.
    pub fn unplugged(&self, i: usize) -> bool {
        !self.is_connected(&self.drive(i))
    }

    /// Settings the recipe is missing that its steps can look up: a
    /// drive's offset, once it has a drive.
    pub fn findable_missing(&self, i: usize) -> Vec<drt_engine::run::Missing> {
        if self.drive(i).is_empty() {
            return vec![];
        }
        self.engine
            .missing(&self.recipes[i])
            .into_iter()
            .filter(|m| self.info(i, &m.step).and_then(|inf| inf.params.iter().find(|p| p.key == m.key).map(|p| p.findable && p.scope == Scope::Drive)).unwrap_or(false))
            .collect()
    }

    /// Looks up what the recipe is missing and its steps can find, off the
    /// window's thread. `measure` allows the slow ways that use the disc.
    pub fn look_up(&mut self, i: usize, measure: bool) {
        // Measuring is asked for, so it runs even when a value is known
        // (from a list, or an older measurement); a plain lookup only fills
        // in what's missing.
        // Nothing found or measured while the drive is in use: its settings
        // are read live by the run.
        if self.finding.contains_key(&i) || self.drive(i).is_empty() || self.drive_busy(i) || self.unplugged(i) || (!measure && self.findable_missing(i).is_empty()) {
            return;
        }
        self.finding.insert(i, measure);
        self.measure_failed.remove(&i);
        let (engine, recipe, tx, ctx) = (self.engine.clone(), self.recipes[i].clone(), self.found_tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let still = if measure {
                let found = engine.find_again(&recipe, true).await;
                // Nothing measured counts as a failure, whatever was known before.
                if found == 0 {
                    usize::MAX
                } else {
                    engine.missing(&recipe).len()
                }
            } else {
                engine.find_missing(&recipe, false).await.len()
            };
            let _ = tx.send((i, measure, still));
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }

    /// Takes in whatever the runs have published. True if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(ctx) = &self.ctx {
            let _ = self.wake.set(ctx.clone());
        }
        while let Ok((at, now)) = self.drives_rx.try_recv() {
            if self.drives_at.is_some_and(|seen| at < seen) {
                continue;
            }
            self.drives_at = Some(at);
            self.apply_drives(now);
            changed = true;
        }
        while let Ok((i, measured, still)) = self.found_rx.try_recv() {
            if i >= self.recipes.len() {
                continue;
            }
            self.finding.remove(&i);
            if measured && (still == usize::MAX || (self.findable_missing(i).len() == still && still > 0)) {
                self.measure_failed.insert(i);
            }
            changed = true;
        }
        let mut ended = Vec::new();
        for (&i, v) in self.runs.iter_mut() {
            for r in v.iter_mut() {
                if r.rx.has_changed().unwrap_or(false) {
                    r.snap = r.rx.borrow_and_update().clone();
                    changed = true;
                    if let Some((_, snap)) = self.before_disc.get_mut(&i).filter(|(id, _)| *id == r.id) {
                        *snap = r.snap.clone();
                    }
                }
                // Ended: kept, replacing the recipe's last.
                if r.snap.finished && !r.kept {
                    r.kept = true;
                    ended.push((i, crate::store::Kept { snap: r.snap.clone(), context: r.run.context(), raw: r.run.raw_all().into_iter().collect() }));
                }
            }
        }
        for (i, k) in ended {
            // Kept until the app closes, not on disk.
            let name = self.recipes.get(i).map(|r| r.name.clone()).unwrap_or_default();
            self.last.insert(name, k);
        }
        // The next disc: when the newest run's Next disc is done, and no
        // disc of the recipe waits for you. One that starts waiting for you
        // before the next disc has gone in puts the next disc back.
        let ids: Vec<usize> = self.runs.keys().copied().collect();
        for i in ids {
            let waits = self.waits_for_you(i).is_some();
            if waits && self.next_not_in(i) {
                self.put_back(i);
                changed = true;
            }
            let loops = self.loops_back_done(self.newest(i).map(|r| &r.snap));
            let left = self.auto_left.get(&i).copied().unwrap_or(self.repeat_limit);
            let no_more = self.no_repeat.contains(&i) || left == Some(0);
            let Some(newest) = self.runs.get_mut(&i).and_then(|v| v.last_mut()) else { continue };
            if newest.repeated || !loops {
                continue;
            }
            let no_more = no_more || newest.snap.stopped;
            // Looked at again each time, until nothing waits for you.
            if waits && !no_more {
                continue;
            }
            newest.repeated = true;
            if no_more || self.drive_busy(i) {
                continue;
            }
            self.auto_left.insert(i, left.map(|n| n - 1));
            if let Err(e) = self.start_run(i) {
                eprintln!("couldn't start the next disc: {e:#}");
            }
            changed = true;
        }
        // Earlier discs leave once they're done.
        for (i, v) in self.runs.iter_mut() {
            let n = v.len();
            let mut k = 0;
            v.retain(|r| {
                k += 1;
                k == n || !r.snap.finished
            });
            if let Some(id) = self.shown.get(i) {
                if !v.iter().any(|r| r.id == *id) || v.last().is_some_and(|r| r.id == *id) {
                    self.shown.remove(i);
                }
            }
        }
        if changed {
            let now = self.engine.settings.lock().unwrap().clone();
            if now != self.saved {
                self.saved = now;
                self.save_settings();
            }
        }
        changed
    }

    /// Whether the recipe's newest run is still going.
    pub fn running(&self, i: usize) -> bool {
        self.newest(i).is_some_and(|r| !r.snap.finished)
    }

    /// Whether any of the recipe's runs hasn't finished, an earlier disc
    /// still holding included. Its settings are locked meanwhile.
    pub fn ripping(&self, i: usize) -> bool {
        self.runs.get(&i).is_some_and(|v| v.iter().any(|r| !r.snap.finished))
    }

    /// Whether a setting of this scope can't be changed just now: the
    /// recipe is ripping, or (for the drive's own settings) another recipe
    /// is reading from the same drive.
    pub fn locked(&self, i: usize, scope: Scope) -> bool {
        self.ripping(i) || (scope == Scope::Drive && self.drive_busy(i))
    }

    pub fn newest(&self, i: usize) -> Option<&Running> {
        self.runs.get(&i).and_then(|v| v.last())
    }

    /// The run the recipe shows: an earlier disc you picked, or the newest.
    pub fn shown_run(&self, i: usize) -> Option<&Running> {
        let v = self.runs.get(&i)?;
        self.shown.get(&i).and_then(|id| v.iter().find(|r| r.id == *id)).or(v.last())
    }

    /// Whether the step `id` hangs off another's *If it fails* or *If it
    /// needs you* path, in the recipe as shown (a draft while editing).
    pub fn on_a_path(&self, i: usize, id: &str) -> bool {
        self.drafts.get(&i).or_else(|| self.recipes.get(i)).is_some_and(|r| r.root.on_a_path(id))
    }

    pub fn shows_newest(&self, i: usize) -> bool {
        self.shown_run(i).map(|r| r.id) == self.newest(i).map(|r| r.id)
    }

    /// Earlier discs still encoding or holding, oldest first.
    pub fn earlier(&self, i: usize) -> Vec<&Running> {
        let Some(v) = self.runs.get(&i) else { return vec![] };
        v[..v.len().saturating_sub(1)].iter().filter(|r| !r.snap.finished).collect()
    }

    /// What a disc of the recipe waits on you for, as what to do first: a
    /// question, files held for you to let through, a step that needs you
    /// (a track to decide). Only discs still going count.
    pub fn waits_for_you(&self, i: usize) -> Option<String> {
        let v = self.runs.get(&i)?;
        // The disc out of the drive last: the newest, or the one before a
        // next disc that hasn't gone in.
        let last = if self.next_not_in(i) { v.len().checked_sub(2) } else { v.len().checked_sub(1) };
        v.iter().enumerate().rev().filter(|(_, r)| !r.snap.finished && !r.snap.stopped).find_map(|(k, r)| {
            let disc = if Some(k) == last { "the last disc" } else { "an earlier disc" };
            if r.snap.question.is_some() {
                return Some(format!("Answer the question on {disc} first"));
            }
            let steps = r.snap.order.iter().filter_map(|id| r.snap.steps.get(id));
            for v in steps {
                if v.state == StepState::Hold && v.detail == crate::card::WAITS_FOR_YOU && v.count(ItemState::Waiting) > 0 {
                    return Some(format!("Let the files on {disc} through first"));
                }
                let retries = self.engine.registry.info(&v.step).is_some_and(|inf| inf.retries.iter().any(|r| !matches!(r.id, "keep" | "leave_out")));
                if crate::card::Shown::of(v, retries) == crate::card::Shown::Needs {
                    return Some(format!("{} on {disc} needs you first", v.name));
                }
            }
            None
        })
    }

    /// Why the recipe is held back, if it is: the next disc won't start, or
    /// Start can't, until you've done what a disc waits for.
    pub fn held_back(&self, i: usize) -> Option<String> {
        if self.next_waits(i) || !self.running(i) {
            return self.waits_for_you(i);
        }
        None
    }

    /// Whether the newest run is through its Next disc and the next disc
    /// will start once nothing waits for you.
    pub fn next_waits(&self, i: usize) -> bool {
        let Some(r) = self.newest(i) else { return false };
        let left = self.auto_left.get(&i).copied().unwrap_or(self.repeat_limit);
        !r.repeated && !r.snap.stopped && !r.snap.finished && self.loops_back_done(Some(&r.snap)) && !self.no_repeat.contains(&i) && left != Some(0)
    }

    /// Whether the newest run is a next disc that hasn't gone in: started by
    /// the one before's Next disc, its Drive still waiting, nothing else begun.
    fn next_not_in(&self, i: usize) -> bool {
        let Some(v) = self.runs.get(&i) else { return false };
        let (Some(r), Some(before)) = (v.last(), v.len().checked_sub(2).map(|k| &v[k])) else { return false };
        before.repeated && waiting_for_disc(&r.snap)
    }

    /// Puts a next disc that hasn't gone in back: stopped and gone, and
    /// the disc before it starts it again once nothing waits for you.
    fn put_back(&mut self, i: usize) {
        let Some(v) = self.runs.get_mut(&i) else { return };
        let Some(r) = v.pop() else { return };
        let _ = r.run.commands().try_send(Command::Stop);
        if let Some(before) = v.last_mut() {
            before.repeated = false;
        }
        if let Some(Some(n)) = self.auto_left.get_mut(&i) {
            *n += 1;
        }
        self.before_disc.remove(&i);
        if self.shown.get(&i) == Some(&r.id) {
            self.shown.remove(&i);
        }
    }

    /// Whether a run's step that leads back to the start (Next disc) is done.
    fn loops_back_done(&self, s: Option<&Snapshot>) -> bool {
        s.is_some_and(|s| s.steps.values().any(|v| v.state == StepState::Ok && self.engine.registry.info(&v.step).is_some_and(|i| i.loops_back)))
    }

    /// Whether any run (of any recipe) has the recipe's drive: one that
    /// hasn't got through its Next disc and hasn't finished.
    pub fn drive_busy(&self, i: usize) -> bool {
        self.drive_user(&self.drive(i)).is_some()
    }

    /// The recipe whose run has the drive just now, if any: a run that
    /// hasn't got through its Next disc and hasn't finished. A drive's
    /// state is the same in every recipe and on every card.
    pub fn drive_user(&self, drive: &str) -> Option<usize> {
        if drive.is_empty() {
            return None;
        }
        self.runs.iter().find(|(&j, v)| self.drive(j) == drive && v.iter().any(|r| !r.snap.finished && !self.loops_back_done(Some(&r.snap)))).map(|(&j, _)| j)
    }

    /// Whether the line splits straight after a spot: a fork.
    pub fn forks_at(&self, i: usize, at: &Spot) -> bool {
        self.recipes[i].root.forks_at(at)
    }

    /// The recipe as it is now: the shown run's snapshot, or how it looks
    /// idle.
    pub fn snapshot(&self, i: usize) -> Snapshot {
        let mut snap = self.snapshot_inner(i);
        let idle = self.shown_run(i).is_none();
        // A tile not started says what it comes after, by name as on its
        // tile ("After Audio CD"); before a run nothing waits in amber.
        let names: std::collections::HashMap<String, String> = snap.steps.iter().map(|(k, v)| (k.clone(), v.name.clone())).collect();
        for v in snap.steps.values_mut() {
            let after = match v.preds.as_slice() {
                [] => "Ready".to_owned(),
                [p] => format!("After {}", names.get(p).cloned().unwrap_or_default()),
                ps => format!("After all {}", ps.len()),
            };
            if idle && v.state == StepState::Wait {
                v.state = StepState::Idle;
                v.status = after;
                v.detail.clear();
            } else if idle && v.state == StepState::Hold {
                // A Hold before any disc: it says what it will do, in grey.
                v.state = StepState::Idle;
            } else if v.state == StepState::Idle && v.status.starts_with("After ") {
                v.status = after;
            }
        }
        // An offset reads with its sign, as in its field: "offset +103".
        for v in snap.steps.values_mut() {
            if let Some(n) = v.detail.strip_prefix("offset ").filter(|n| n.parse::<i64>().is_ok_and(|n| n > 0) && !n.starts_with('+')) {
                v.detail = format!("offset +{n}");
            }
        }
        self.rip_pace(i, &mut snap);
        // A drive that isn't there: its tile says so, calmly, running or not.
        if self.unplugged(i) {
            for v in snap.steps.values_mut().filter(|v| v.step == "drive") {
                v.state = StepState::Idle;
                v.status = "Not connected".into();
                v.detail.clear();
            }
        }
        // Not running, and another run has the drive: its Drive tile waits.
        if !self.running(i) && !self.unplugged(i) {
            if let Some(j) = self.drive_user(&self.drive(i)).filter(|&j| j != i) {
                for v in snap.steps.values_mut().filter(|v| v.step == "drive") {
                    v.state = StepState::Wait;
                    v.status = "Busy".into();
                    v.detail = self.recipes[j].name.clone();
                }
            }
        }
        snap
    }

    /// Rip's card while it reads: the track, its time left, the disc's,
    /// and the speed; once read, how long it took and at what speed, if it
    /// has nothing else to say. A disc's image (Image, a game's Rip) the
    /// same while a pass reads the drive, its speed from the sectors; a
    /// video disc's titles from their sizes, when the scan gave them.
    fn rip_pace(&self, i: usize, snap: &mut Snapshot) {
        let Some(run) = self.shown_run(i) else { return };
        let rips: Vec<String> = snap.steps.iter().filter(|(_, v)| matches!(v.step.as_str(), "rip" | "image" | "game-rip" | "rip-video")).map(|(k, _)| k.clone()).collect();
        if rips.is_empty() {
            return;
        }
        let c = run.run.context();
        let media = c.get(drt_engine::Subject::Disc, "media").unwrap_or("").to_owned();
        let part = |n: u32, key: &str| c.get(drt_engine::Subject::Part(n), key).and_then(|l| l.parse::<f32>().ok());
        let mut paces = self.pace.lock().unwrap();
        for id in rips {
            let Some(v) = snap.steps.get_mut(&id) else { continue };
            let key = (i, run.id, id.clone());
            let kind = v.step.clone();
            let dump = matches!(kind.as_str(), "image" | "game-rip");
            // A disc's image: only while a pass reads the drive, and by the
            // sectors its note says.
            let phase = v.detail.clone();
            let reads = !dump || matches!(phase.as_str(), "Reading" | "Re-reading bad parts");
            let busy = v.items.iter().find(|it| it.state == ItemState::Busy);
            let disc_1x = busy.and_then(|it| crate::pace::sectors(&it.note)).and_then(|(_, of)| crate::pace::seconds_at_1x(&media, of));
            // A title's length for its speed: its size at 1× if known (a
            // drive's speed), else none (a speed against playing time
            // isn't the drive's).
            let one_x = crate::pace::one_x(&media);
            let length = |n: u32| match kind.as_str() {
                "image" | "game-rip" => disc_1x,
                "rip-video" => part(n, "bytes").zip(one_x).map(|(b, x)| b / x),
                _ => part(n, "length"),
            };
            // What redumper is on just now: a re-read says where and how
            // it's going itself (its pace isn't the disc's); a read, its
            // speed now, from its own lines.
            let live = v.now.clone().and_then(|n| serde_json::from_value::<drt_steps::real::redumper::Live>(n).ok()).filter(|_| dump && busy.is_some());
            if let Some(l) = live.as_ref().filter(|l| l.rereading) {
                paces.remove(&key);
                v.detail = l.card();
                continue;
            }
            let crawling = live.as_ref().is_some_and(|l| l.crawling);
            if busy.is_some() && reads {
                let (pace, last) = paces.entry(key).or_default();
                let now = std::time::Instant::now();
                *last = match &live {
                    Some(l) => Some(pace.drive(l, now)),
                    None => pace.read(&v.items, &v.detail, length, now),
                };
                if let Some(r) = last {
                    if kind == "rip-video" {
                        r.unit = "Video";
                    }
                    v.detail = if dump {
                        let times = r.times();
                        if times.is_empty() {
                            phase
                        } else {
                            format!("{phase} · {times}")
                        }
                    } else {
                        r.line()
                    };
                }
                // The times count down between Rip's own news.
                if let Some(ctx) = &self.ctx {
                    ctx.request_repaint_after(std::time::Duration::from_secs(1));
                }
                // A crawl says so, with the speed: a card short of room
                // keeps the speed whole and cuts the words before it.
                if crawling {
                    v.detail = match last.as_ref().and_then(|r| r.speed.as_ref()) {
                        Some(s) => format!("{CRAWLING_CARD} · {s}"),
                        None => CRAWLING_CARD.to_owned(),
                    };
                }
            } else if crawling {
                paces.remove(&key);
                v.detail = CRAWLING_CARD.to_owned();
            } else {
                paces.remove(&key);
                if v.detail.is_empty() && v.state == StepState::Ok && kind == "rip" {
                    if let Some(said) = read_in(v, length) {
                        v.detail = said;
                    }
                }
            }
        }
    }

    /// What Rip came to last, while it reads.
    pub fn reading(&self, i: usize, step: &str) -> Option<crate::pace::Reading> {
        let run = self.shown_run(i)?;
        self.pace.lock().unwrap().get(&(i, run.id, step.to_owned())).and_then(|(_, r)| r.clone())
    }

    fn snapshot_inner(&self, i: usize) -> Snapshot {
        if let Some(r) = self.shown_run(i) {
            // Waiting for the next disc: the one before as it ended, with
            // the Drive as it is now.
            if let Some((_, before)) = self.before_disc.get(&i).filter(|_| self.shows_newest(i) && waiting_for_disc(&r.snap)) {
                let mut snap = before.clone();
                for (id, v) in r.snap.steps.iter().filter(|(_, v)| v.step == "drive") {
                    snap.steps.insert(id.clone(), v.clone());
                }
                return snap;
            }
            return r.snap.clone();
        }
        let recipe = &self.recipes[i];
        let mut snap = self.engine.idle(recipe);
        // After a restart, the last run as it was left, while the recipe
        // still has the same steps.
        if let Some(k) = self.last.get(&recipe.name).filter(|k| k.snap.order == snap.order) {
            return k.snap.clone();
        }
        // A step these steps don't have: a muted tile that says so.
        for id in self.unavailable(i) {
            if let Some(v) = snap.steps.get_mut(&id) {
                v.name = cap(&v.step);
                v.from.clear();
                v.state = StepState::Idle;
                v.status = "Not available".into();
                v.detail.clear();
            }
        }
        // A tile missing several things asks for the first.
        let mut seen = Vec::new();
        for m in self.engine.missing(recipe) {
            if seen.contains(&m.step) {
                continue;
            }
            if let Some(v) = snap.steps.get_mut(&m.step) {
                v.status = format!("{}?", m.label);
                // Being looked up: busy, not amber.
                if self.finding.contains_key(&i) && self.findable_missing(i).iter().any(|f| f.step == m.step && f.key == m.key) {
                    v.state = StepState::Busy;
                    v.status = "Looking up".into();
                    v.detail = m.label.to_lowercase();
                }
            }
            seen.push(m.step);
        }
        snap
    }

    pub fn drive(&self, i: usize) -> String {
        Engine::drive_of(&self.recipes[i])
    }

    /// Why the recipe can't start, if it can't.
    /// As a sentence, for Start's tooltip.
    pub fn cant_start(&self, i: usize) -> Option<String> {
        let r = &self.recipes[i];
        if self.running(i) {
            return Some("Already running".into());
        }
        if self.drafts.contains_key(&i) {
            return Some("Save or cancel your edits first".into());
        }
        if let Some(why) = self.waits_for_you(i) {
            return Some(why);
        }
        if self.unplugged(i) {
            return Some("Not connected".into());
        }
        let missing = self.engine.missing(r);
        if let Some(m) = missing.first() {
            let more = if missing.len() > 1 { format!(", and {} more", missing.len() - 1) } else { String::new() };
            return Some(format!("{} needs {}{more}", self.tile_name(i, &m.step), m.label.to_lowercase()));
        }
        if let Some(id) = self.unavailable(i).first() {
            return Some(format!("{} isn't in this version of the app", self.tile_name(i, id)));
        }
        if let Some(id) = self.misfit(i) {
            return Some(format!("{} doesn't fit where it is", self.tile_name(i, &id)));
        }
        if self.check_word(i).is_some() {
            return Some("The steps don't fit together".into());
        }
        if let Some(j) = self.drive_user(&self.drive(i)).filter(|&j| j != i) {
            return Some(format!("The drive is busy with {}", self.recipes[j].name));
        }
        if self.drive_busy(i) {
            return Some("The drive is busy".into());
        }
        None
    }

    /// The first tile that doesn't fit after what comes before it.
    pub fn misfit(&self, i: usize) -> Option<String> {
        drt_engine::first_misfit(&self.recipes[i].root, &self.engine.registry)
    }

    /// A tile's name: its label, or its step's.
    pub fn tile_name(&self, i: usize, id: &str) -> String {
        let mut root = self.recipes[i].root.clone();
        match root.find_mut(id) {
            Some(Node::Step { label: Some(l), .. }) => l.clone(),
            Some(Node::Step { step, .. }) => self.engine.registry.info(step).map_or(step.clone(), |inf| inf.name.to_owned()),
            _ => id.to_owned(),
        }
    }

    /// Steps in the recipe this registry doesn't have, by id.
    pub fn unavailable(&self, i: usize) -> Vec<String> {
        self.recipes[i]
            .root
            .steps()
            .into_iter()
            .filter_map(|n| match n {
                Node::Step { id, step, .. } if self.engine.registry.info(step).is_none() => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    /// Why the recipe doesn't check, in a word or two.
    pub fn check_word(&self, i: usize) -> Option<&'static str> {
        if !self.unavailable(i).is_empty() {
            return Some("not available");
        }
        check(&self.recipes[i].root, &self.engine.registry).err().map(|_| "doesn't fit")
    }

    /// Start, by hand: this disc, and the ones after it through Next disc.
    pub fn start(&mut self, i: usize) -> Result<()> {
        if let Some(why) = self.cant_start(i) {
            bail!("{why}");
        }
        self.no_repeat.remove(&i);
        self.auto_left.insert(i, self.repeat_limit);
        self.shown.remove(&i);
        self.start_run(i)
    }

    /// Stop, by hand: the newest disc, and no more after it.
    pub fn stop(&mut self, i: usize) {
        // The next disc held back: only it is called off; the disc out of
        // the drive goes on finishing.
        let waiting = self.next_waits(i);
        self.no_repeat.insert(i);
        if waiting {
            return;
        }
        if let Some(r) = self.newest(i) {
            let _ = r.run.commands().try_send(Command::Stop);
        }
    }

    fn start_run(&mut self, i: usize) -> Result<()> {
        match self.newest(i).filter(|r| r.snap.finished || self.loops_back_done(Some(&r.snap))) {
            Some(r) => {
                let before = (r.id, r.snap.clone());
                self.before_disc.insert(i, before);
            }
            None => {
                self.before_disc.remove(&i);
            }
        }
        let run = {
            let _g = self.rt.enter();
            self.engine.start(&self.recipes[i])?
        };
        if let Some(ctx) = &self.ctx {
            // Wake the window whenever the run changes; nothing polls.
            let ctx = ctx.clone();
            let mut w = run.snapshot.clone();
            self.rt.spawn(async move {
                while w.changed().await.is_ok() {
                    ctx.request_repaint();
                }
            });
        }
        let rx = run.snapshot.clone();
        let snap = rx.borrow().clone();
        let id = self.next_id;
        self.next_id += 1;
        let (recipe, settings) = (self.recipes[i].clone(), self.engine.settings.lock().unwrap().clone());
        self.runs.entry(i).or_default().push(Running { id, run, rx, snap, repeated: false, kept: false, recipe, settings });
        Ok(())
    }

    /// Says something to the run the recipe shows.
    pub fn send(&self, i: usize, c: Command) {
        // Waiting for the next disc, the canvas is the disc before's: what
        // you answer there is its.
        let before = self.before_disc.get(&i).filter(|_| self.shows_newest(i) && self.shown_run(i).is_some_and(|r| waiting_for_disc(&r.snap)));
        let to_before = matches!(c, Command::Answer { .. } | Command::Approve { .. } | Command::Retry { .. });
        let run = match before {
            Some((id, _)) if to_before => self.runs.get(&i).and_then(|v| v.iter().find(|r| r.id == *id)),
            _ => self.shown_run(i),
        };
        if let Some(r) = run {
            let _ = r.run.commands().try_send(c);
        }
    }

    /// One line for the recipe list, and the state its icon shows.
    pub fn status_line(&self, i: usize) -> (String, StepState) {
        // Calm, not a fault: nothing to do here till it's plugged in.
        if self.unplugged(i) {
            return ("Not connected".into(), StepState::Idle);
        }
        if let Some(why) = self.held_back(i) {
            return (why, StepState::Needs);
        }
        if let Some(r) = self.newest(i) {
            return run_line(&r.snap, false);
        }
        if self.finding.contains_key(&i) {
            return ("Looking up".into(), StepState::Busy);
        }
        if self.drive_busy(i) && self.engine.missing(&self.recipes[i]).is_empty() {
            return ("Drive busy".into(), StepState::Wait);
        }
        let missing = self.engine.missing(&self.recipes[i]);
        match missing.len() {
            0 if self.check_word(i).is_some() => (cap(self.check_word(i).unwrap()), StepState::Idle),
            0 => ("Ready".into(), StepState::Idle),
            1 => (format!("{}?", missing[0].label), StepState::Needs),
            n => (format!("{n} to set"), StepState::Needs),
        }
    }

    /// Writes recipes.json: each recipe as saved, so a draft being edited
    /// stays out of it until Save.
    fn save(&self) {
        let out: Vec<Recipe> = self.recipes.iter().enumerate().map(|(i, r)| self.drafts.get(&i).unwrap_or(r).clone()).collect();
        let r = self.store.save_recipes(&out);
        self.note(r.map_err(|e| format!("Not saving: {e:#}")));
    }

    pub fn save_settings(&self) {
        let s = self.engine.settings.lock().unwrap().clone();
        let r = self.store.save_settings(&s);
        self.note(r.map_err(|e| format!("Not saving: {e:#}")));
    }

    fn note(&self, r: std::result::Result<(), String>) {
        let mut f = self.failed.lock().unwrap();
        match r {
            Ok(()) => *f = None,
            Err(e) => {
                eprintln!("{e}");
                *f = Some(e);
            }
        }
    }

    /// Whether *Open tray* is offered for the recipe's drive: one is
    /// named and connected.
    pub fn can_open_tray(&self, i: usize) -> bool {
        let drive = self.drive(i);
        !drive.is_empty() && self.is_connected(&drive)
    }

    /// Whether the offset is being measured on the recipe's drive: that
    /// reads the disc with no run to stop, so the tray waits till it's done.
    pub fn measuring_on_drive(&self, i: usize) -> bool {
        let drive = self.drive(i);
        self.finding.iter().any(|(&j, &measure)| measure && self.drive(j) == drive)
    }

    /// Whether the recipe's drive's tray is being opened just now.
    pub fn opening_tray(&self, i: usize) -> bool {
        self.opening.lock().unwrap().contains(&self.drive(i))
    }

    /// *Open tray* on the recipe's drive, at any time: every run on that
    /// drive stops what it's reading (its tool exits; the track waits for
    /// you) and starts nothing more till asked; then the door is unlocked
    /// and the tray opens. Off the window's thread.
    pub fn open_tray(&mut self, i: usize) {
        let drive = self.drive(i);
        if !self.can_open_tray(i) || self.measuring_on_drive(i) || !self.opening.lock().unwrap().insert(drive.clone()) {
            return;
        }
        let released: Vec<_> = self
            .runs
            .iter()
            .filter(|(&j, _)| self.drive(j) == drive)
            .flat_map(|(_, v)| v.iter().filter(|r| !r.snap.finished))
            .map(|r| r.run.release_drive(std::time::Duration::from_secs(30)))
            .collect();
        let (hook, failed, opening, ctx) = (self.open_tray_hook.clone(), self.failed.clone(), self.opening.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            for r in released {
                if !r.await {
                    eprintln!("{drive}: a read hadn't stopped after 30 s; opening the tray anyway");
                }
            }
            let d = drive.clone();
            let r = tokio::task::spawn_blocking(move || hook(&d)).await.unwrap_or_else(|e| Err(e.to_string()));
            *failed.lock().unwrap() = r.err().inspect(|e| eprintln!("{e}"));
            opening.lock().unwrap().remove(&drive);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }

    /// Opens a folder; if that fails, says so where saving failures go.
    pub fn open(&self, path: &Path) {
        let r = (self.open_folder)(path);
        self.note(r.map_err(|e| format!("Can't open {}: {e}", path.display())));
    }

    /// Opens a web page in the browser, by the same hook as a folder.
    pub fn open_url(&self, url: &str) {
        self.open(Path::new(url));
    }

    /// Why nothing is being kept, or what last failed, until it's fixed:
    /// shown in the header.
    pub fn not_saving(&self) -> Option<String> {
        if self.store.recipes_unreadable.is_some() {
            return Some("Not saving: recipes.json can't be read".into());
        }
        if self.store.settings_unreadable.is_some() {
            return Some("Not saving: settings.json can't be read".into());
        }
        self.failed.lock().unwrap().clone()
    }

    /// "cyanrip isn't installed", "cyanrip and ffmpeg aren't installed".
    pub fn tools_missing(&self) -> Option<String> {
        let names = &self.missing_tools;
        let (last, rest) = names.split_last()?;
        Some(match rest {
            [] => format!("{last} isn't installed"),
            _ => format!("{} and {last} aren't installed", rest.join(", ")),
        })
    }

    pub fn info(&self, i: usize, id: &str) -> Option<Arc<StepInfo>> {
        let kind = self.recipes[i].root.ids().contains(&id).then(|| step_kind(&self.recipes[i].root, id)).flatten()?;
        self.engine.registry.info(&kind)
    }

    /// A parameter's setting as it stands, for its tile.
    /// A setting as the recipe shows it: for an earlier disc, what that
    /// run started with; otherwise as it is now.
    pub fn shown_param(&self, i: usize, id: &str, spec: &ParamSpec) -> Setting<Value> {
        match self.shown_run(i).filter(|_| !self.shows_newest(i)) {
            Some(r) => match spec.scope {
                Scope::Drive => r.settings.drive(&Engine::drive_of(&r.recipe), spec.key),
                Scope::App => r.settings.app(&step_kind(&r.recipe.root, id).unwrap_or_default(), spec.key),
                Scope::Recipe => match r.recipe.root.clone().find_mut(id) {
                    Some(Node::Step { params, .. }) => params.get(spec.key).cloned().unwrap_or(Setting::Default),
                    _ => Setting::Default,
                },
            },
            None => self.param(i, id, spec),
        }
    }

    /// What arrives at a step in the recipe as shown (an earlier disc's
    /// as that run had it): what a setting that follows it goes by.
    pub fn shown_flow(&self, i: usize, id: &str) -> Option<Flow> {
        match self.shown_run(i).filter(|_| !self.shows_newest(i)) {
            Some(r) => flows_in(&r.recipe.root, &self.engine.registry).remove(id),
            None => self.flow(i, id),
        }
    }

    /// What arrives at a step in the recipe as it is now.
    pub fn flow(&self, i: usize, id: &str) -> Option<Flow> {
        flows_in(&self.recipes[i].root, &self.engine.registry).remove(id)
    }

    /// The value in effect for a setting as shown: its own, or the default
    /// where its step stands.
    pub fn shown_value(&self, i: usize, id: &str, spec: &ParamSpec) -> Option<Value> {
        spec.resolve_at(&self.shown_param(i, id, spec), self.shown_flow(i, id).as_ref())
    }

    /// The value in effect for a setting as it is now.
    pub fn value(&self, i: usize, id: &str, spec: &ParamSpec) -> Option<Value> {
        spec.resolve_at(&self.param(i, id, spec), self.flow(i, id).as_ref())
    }

    pub fn param(&self, i: usize, id: &str, spec: &ParamSpec) -> Setting<Value> {
        match spec.scope {
            Scope::Drive => self.engine.settings.lock().unwrap().drive(&self.drive(i), spec.key),
            // The app's own (an API key): one for every recipe.
            Scope::App => self.engine.settings.lock().unwrap().app(&step_kind(&self.recipes[i].root, id).unwrap_or_default(), spec.key),
            Scope::Recipe => {
                let mut root = self.recipes[i].root.clone();
                match root.find_mut(id) {
                    Some(Node::Step { params, .. }) => params.get(spec.key).cloned().unwrap_or(Setting::Default),
                    _ => Setting::Default,
                }
            }
        }
    }

    /// Back to the default: a setting you chose goes, and one that can be
    /// looked up (a drive's offset) is looked up again.
    pub fn reset_param(&mut self, i: usize, id: &str, key: &str) -> Result<()> {
        let info = self.info(i, id).ok_or_else(|| anyhow!("no step {id}"))?;
        let spec = info.params.iter().find(|p| p.key == key).ok_or_else(|| anyhow!("{id} has no {key}"))?;
        if self.locked(i, spec.scope) {
            bail!("Locked while ripping");
        }
        match spec.scope {
            Scope::Drive => {
                self.engine.settings.lock().unwrap().set_drive(&self.drive(i), key, Setting::Default);
                self.save_settings();
                if spec.findable {
                    self.look_up(i, false);
                }
            }
            Scope::App => {
                self.engine.settings.lock().unwrap().app.remove(&format!("{}.{key}", info.kind));
                self.save_settings();
            }
            Scope::Recipe => {
                for root in [Some(&mut self.recipes[i].root), self.drafts.get_mut(&i).map(|d| &mut d.root)].into_iter().flatten() {
                    if let Some(Node::Step { params, .. }) = root.find_mut(id) {
                        params.remove(key);
                    }
                }
                self.save();
            }
        }
        Ok(())
    }

    /// Whether you changed a setting: chosen, and not what the recipe came
    /// with (its built-in's value) nor the step's default. Its card shows
    /// a dot; its sidebar says *yours*. `builtin`: [`Model::builtin`].
    pub fn is_yours(&self, i: usize, id: &str, spec: &ParamSpec, builtin: Option<&Recipe>) -> bool {
        let set = self.shown_param(i, id, spec);
        if !matches!(set, Setting::Chosen { .. }) {
            return false;
        }
        // Both as they read where the step stands: a value that only ever
        // meant the default is the default.
        let flow = self.shown_flow(i, id);
        let seeded = builtin.and_then(|b| {
            let mut root = b.root.clone();
            match root.find_mut(id) {
                Some(Node::Step { params, .. }) => params.get(spec.key).cloned(),
                _ => None,
            }
        });
        spec.resolve_at(&seeded.unwrap_or(Setting::Default), flow.as_ref()) != spec.resolve_at(&set, flow.as_ref())
    }

    /// The built-in recipe this one was seeded from, by name, if any.
    pub fn builtin(&self, i: usize) -> Option<Recipe> {
        drt_steps::recipes::all().into_iter().find(|r| r.name == self.recipes[i].name && check(&r.root, &self.engine.registry).is_ok())
    }

    /// Whether the recipe differs from the one it was built in as (when
    /// a setting was chosen doesn't count).
    pub fn differs_from_builtin(&self, i: usize) -> bool {
        fn bare(n: &Node) -> Value {
            fn strip(v: &mut Value) {
                match v {
                    Value::Object(m) => {
                        m.remove("at");
                        m.values_mut().for_each(strip);
                    }
                    Value::Array(a) => a.iter_mut().for_each(strip),
                    _ => {}
                }
            }
            let mut v = serde_json::to_value(n).unwrap_or_default();
            strip(&mut v);
            v
        }
        self.builtin(i).is_some_and(|b| bare(&b.root) != bare(&self.recipes[i].root))
    }

    /// While editing: the recipe as it was built in, as one undoable edit.
    pub fn reset_to_builtin(&mut self, i: usize) {
        if let Some(b) = self.builtin(i).filter(|_| self.differs_from_builtin(i)) {
            let before = self.recipes[i].root.clone();
            self.edit(i, before, b.root);
        }
    }

    /// Sets a parameter, as chosen: into the recipe, or onto the drive for
    /// every recipe using it. Saved at once.
    pub fn set_param(&mut self, i: usize, id: &str, key: &str, value: Value) -> Result<()> {
        let info = self.info(i, id).ok_or_else(|| anyhow!("no step {id}"))?;
        let spec = info.params.iter().find(|p| p.key == key).ok_or_else(|| anyhow!("{id} has no {key}"))?;
        if self.locked(i, spec.scope) {
            bail!("Locked while ripping");
        }
        match spec.scope {
            Scope::Drive => {
                let drive = self.drive(i);
                if drive.is_empty() {
                    bail!("no drive yet");
                }
                self.engine.settings.lock().unwrap().set_drive(&drive, key, Setting::chosen(value));
                self.save_settings();
            }
            Scope::App => {
                self.engine.settings.lock().unwrap().set_app(info.kind, key, Setting::chosen(value));
                self.save_settings();
            }
            Scope::Recipe => {
                let set = Setting::chosen(value);
                match self.recipes[i].root.find_mut(id) {
                    Some(Node::Step { params, .. }) => {
                        params.insert(key.to_owned(), set.clone());
                    }
                    _ => bail!("no step {id}"),
                }
                // Only structure is drafted: a parameter is saved at once,
                // into the saved recipe too while a draft is open.
                if let Some(Node::Step { params, .. }) = self.drafts.get_mut(&i).and_then(|d| d.root.find_mut(id)) {
                    params.insert(key.to_owned(), set);
                }
                self.save();
            }
        }
        // A new drive: look up what's known about it at once.
        if key == "drive" {
            self.look_up(i, false);
        }
        Ok(())
    }

    /// Every step that can follow `after`, with whether it checks in line
    /// and as a new branch. A step that fits neither way isn't listed.
    pub fn placements_after(&self, i: usize, after: &str) -> Vec<(&StepInfo, bool, bool)> {
        drt_engine::placements(&self.recipes[i].root, &Spot::After(after.to_owned()), &self.engine.registry)
    }

    /// What the + at a spot offers, and how a click places each.
    pub fn offers(&self, i: usize, at: &Spot) -> Vec<drt_engine::Offer<'_>> {
        drt_engine::offers(&self.recipes[i].root, at, &self.engine.registry)
    }

    /// Every spot a + can be: after each step, and after each set of
    /// branches.
    pub fn spots(&self, i: usize) -> Vec<Spot> {
        let root = &self.recipes[i].root;
        let sides = root.line_ids().into_iter().flat_map(|id| drt_engine::Side::BOTH.map(|s| Spot::On(id.to_owned(), s)));
        root.ids().into_iter().map(|id| Spot::After(id.to_owned())).chain(root.joins().into_iter().map(Spot::Join)).chain(sides).collect()
    }

    /// Puts a new step of `kind` after `after`, if the recipe still checks.
    /// Gives the new step's id.
    pub fn insert_after(&mut self, i: usize, after: &str, kind: &str, branch: bool) -> Result<String> {
        self.add(i, &Spot::After(after.to_owned()), kind, branch)
    }

    /// Puts a new step of `kind` at a spot, in line or `beside` what
    /// follows, if the recipe still checks. Gives the new step's id.
    pub fn add(&mut self, i: usize, at: &Spot, kind: &str, beside: bool) -> Result<String> {
        if self.ripping(i) {
            bail!("running");
        }
        let root = &self.recipes[i].root;
        let id = fresh_id(root, kind);
        let mut new = root.clone();
        if !new.add_at(at, Node::step(&id, kind), beside) {
            bail!("nowhere to add it");
        }
        check(&new, &self.engine.registry).map_err(|e| anyhow!(plain_misfit(&self.engine.registry, &new, &e)))?;
        let before = self.recipes[i].root.clone();
        self.edit(i, before, new);
        Ok(id)
    }

    /// What removing `id` takes with it: the step, and whatever
    /// downstream would no longer fit without it. Empty if nothing would
    /// be left.
    pub fn removal(&self, i: usize, id: &str) -> Vec<String> {
        self.recipes[i].root.clone().remove_with_downstream(id, &self.engine.registry)
    }

    /// Removes `id` and what depends on it, so the recipe still checks.
    /// Never refused for not fitting: removing a step that doesn't fit, or
    /// one these steps don't have, is how a recipe is mended. Only the last
    /// step stays. Gives the ids removed.
    pub fn remove_step(&mut self, i: usize, id: &str) -> Result<Vec<String>> {
        if self.ripping(i) {
            bail!("running");
        }
        let before = self.recipes[i].root.clone();
        let mut new = before.clone();
        let gone = new.remove_with_downstream(id, &self.engine.registry);
        if gone.is_empty() {
            bail!("last step");
        }
        self.edit(i, before, new);
        Ok(gone)
    }

    /// A structural change: into the draft (undoable) while editing, or
    /// saved at once otherwise.
    fn edit(&mut self, i: usize, before: Node, after: Node) {
        self.recipes[i].root = after;
        self.runs.remove(&i);
        if self.drafts.contains_key(&i) {
            self.undo.entry(i).or_default().push(before);
            self.redo.remove(&i);
        } else {
            self.save();
        }
    }

    /// Starts editing the recipe's structure, on a draft.
    pub fn begin_edit(&mut self, i: usize) {
        self.drafts.entry(i).or_insert_with(|| self.recipes[i].clone());
    }

    pub fn editing(&self, i: usize) -> bool {
        self.drafts.contains_key(&i)
    }

    /// Whether the draft differs from what's saved.
    pub fn dirty(&self, i: usize) -> bool {
        self.drafts.get(&i).is_some_and(|d| *d != self.recipes[i])
    }

    pub fn save_edit(&mut self, i: usize) {
        self.drafts.remove(&i);
        self.undo.remove(&i);
        self.redo.remove(&i);
        self.save();
    }

    pub fn cancel_edit(&mut self, i: usize) {
        if let Some(r) = self.drafts.remove(&i) {
            self.recipes[i] = r;
        }
        self.undo.remove(&i);
        self.redo.remove(&i);
        self.runs.remove(&i);
    }

    pub fn can_undo(&self, i: usize) -> bool {
        self.undo.get(&i).is_some_and(|u| !u.is_empty())
    }

    pub fn can_redo(&self, i: usize) -> bool {
        self.redo.get(&i).is_some_and(|u| !u.is_empty())
    }

    pub fn undo(&mut self, i: usize) {
        if let Some(prev) = self.undo.get_mut(&i).and_then(Vec::pop) {
            let now = std::mem::replace(&mut self.recipes[i].root, prev);
            self.redo.entry(i).or_default().push(now);
        }
    }

    pub fn redo(&mut self, i: usize) {
        if let Some(next) = self.redo.get_mut(&i).and_then(Vec::pop) {
            let now = std::mem::replace(&mut self.recipes[i].root, next);
            self.undo.entry(i).or_default().push(now);
        }
    }

    /// Why a recipe can't be deleted just now, if it can't.
    pub fn cant_delete(&self, i: usize) -> Option<&'static str> {
        if self.running(i) {
            Some("running")
        } else if !self.earlier(i).is_empty() {
            Some("holding a disc")
        } else if self.recipes.len() <= 1 {
            Some("the only recipe")
        } else {
            None
        }
    }

    /// Deletes a recipe, and saves.
    pub fn delete_recipe(&mut self, i: usize) -> Result<()> {
        if let Some(why) = self.cant_delete(i) {
            bail!("{why}");
        }
        self.recipes.remove(i);
        self.reindex(|k| match k.cmp(&i) {
            std::cmp::Ordering::Less => Some(k),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(k - 1),
        });
        self.save();
        Ok(())
    }

    /// Renames a recipe being edited: into its draft, so Save keeps it
    /// and Cancel puts the old name back. Kept unique.
    pub fn rename_draft(&mut self, i: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() || name == self.recipes[i].name {
            return;
        }
        let name = self.unique_name(name);
        self.recipes[i].name = name;
        if !self.drafts.contains_key(&i) {
            self.save();
        }
    }

    /// Names a tile: its label, as part of the draft while editing. Empty
    /// goes back to the step's own name.
    pub fn set_label(&mut self, i: usize, id: &str, text: &str) {
        if self.ripping(i) {
            return;
        }
        let text = text.trim();
        if let Some(Node::Step { label, .. }) = self.recipes[i].root.find_mut(id) {
            *label = (!text.is_empty()).then(|| text.to_owned());
        }
        if !self.drafts.contains_key(&i) {
            self.save();
        }
    }

    /// A copy of a recipe, "<name> copy", right below it. Gives its index.
    pub fn duplicate_recipe(&mut self, i: usize) -> usize {
        let mut r = self.drafts.get(&i).unwrap_or(&self.recipes[i]).clone();
        r.name = self.unique_name(&format!("{} copy", r.name));
        self.reindex(|k| Some(if k > i { k + 1 } else { k }));
        self.recipes.insert(i + 1, r);
        self.save();
        i + 1
    }

    fn unique_name(&self, name: &str) -> String {
        (1..).map(|n| if n == 1 { name.to_owned() } else { format!("{name} {n}") }).find(|c| !self.recipes.iter().any(|r| &r.name == c)).unwrap()
    }

    /// Moves everything kept by recipe index; `None` drops it.
    fn reindex(&mut self, f: impl Fn(usize) -> Option<usize>) {
        fn map<V>(m: &mut HashMap<usize, V>, f: &impl Fn(usize) -> Option<usize>) {
            *m = std::mem::take(m).into_iter().filter_map(|(k, v)| f(k).map(|k| (k, v))).collect();
        }
        fn set(m: &mut std::collections::HashSet<usize>, f: &impl Fn(usize) -> Option<usize>) {
            *m = std::mem::take(m).into_iter().filter_map(f).collect();
        }
        map(&mut self.runs, &f);
        map(&mut self.shown, &f);
        map(&mut self.auto_left, &f);
        map(&mut self.finding, &f);
        map(&mut self.drafts, &f);
        map(&mut self.undo, &f);
        map(&mut self.redo, &f);
        set(&mut self.no_repeat, &f);
        set(&mut self.measure_failed, &f);
    }

    /// A new recipe: one Drive tile, asking which drive.
    pub fn new_recipe(&mut self, name: &str) -> usize {
        let name = name.trim();
        let name = (1..).map(|n| if n == 1 { name.to_owned() } else { format!("{name} {n}") }).find(|c| !self.recipes.iter().any(|r| &r.name == c)).unwrap();
        self.recipes.push(Recipe { name, root: Node::step("drive", "drive") });
        self.save();
        self.recipes.len() - 1
    }
}

/// The name a drive already goes by, if one's every word is in what the
/// system calls it: sysfs says *MATSHITA BD-MLT UJ240AS* for the drive
/// recipes call *MATSHITA UJ240AS*. Otherwise the system's name.
pub fn known_name(system: &str, known: &[String]) -> String {
    let words: Vec<String> = system.split_whitespace().map(str::to_lowercase).collect();
    known.iter().filter(|k| !k.is_empty() && k.split_whitespace().all(|w| words.contains(&w.to_lowercase()))).max_by_key(|k| k.split_whitespace().count()).cloned().unwrap_or_else(|| system.to_owned())
}

/// A run's line: *Which release?*, *Holding*, *6 of 17*. For an
/// earlier disc, what it's doing now (*2 encoding*) rather than the rip.
/// A run whose Drive hasn't a disc yet: nothing of this disc to show.
fn waiting_for_disc(s: &Snapshot) -> bool {
    // Nothing else begun: a run through its own Next disc says its drive
    // is empty too, but it has a disc's worth to show.
    !s.finished
        && !s.stopped
        && s.question.is_none()
        && s.steps.values().any(|v| v.step == "drive" && matches!(v.state, StepState::Busy | StepState::Idle | StepState::Wait))
        && s.steps.values().all(|v| v.step == "drive" || matches!(v.state, StepState::Idle | StepState::Wait) || (v.state == StepState::Hold && v.items.is_empty()))
}

pub fn run_line(s: &Snapshot, earlier: bool) -> (String, StepState) {
    if s.stopped {
        return ("Stopped".into(), StepState::Idle);
    }
    if let Some((_, q)) = &s.question {
        return (q.prompt.clone(), StepState::Needs);
    }
    if s.steps.values().any(|v| v.state == StepState::Hold && v.count(ItemState::Waiting) > 0 && v.detail == crate::card::WAITS_FOR_YOU) {
        return ("Holding".into(), StepState::Hold);
    }
    let bad: usize = s.steps.values().map(|v| v.count(ItemState::Bad)).sum();
    if bad > 0 {
        // A track to decide is a question for you, not a failure.
        let _ = bad;
        return ("Needs you".into(), StepState::Needs);
    }
    if s.finished {
        return ("Done".into(), StepState::Ok);
    }
    if !earlier {
        if let Some(rip) = s.steps.values().find(|v| v.step == "rip" && !v.items.is_empty() && v.state != StepState::Ok) {
            return (format!("{} of {}", rip.count(ItemState::Ok), rip.items.len()), StepState::Busy);
        }
    }
    let busy = s.order.iter().filter_map(|id| s.steps.get(id)).find(|v| v.state == StepState::Busy);
    let line = busy.map(|v| if earlier && !v.detail.is_empty() { v.detail.clone() } else { v.status.clone() });
    (line.filter(|t| !t.is_empty()).unwrap_or_else(|| "Running".into()), StepState::Busy)
}

/// "gnudb" as "Gnudb": a name for a step the registry doesn't know.
fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Brings a recipe saved by an older version up to date. True if
/// anything changed.
///
/// - Step nodes saved under an old kind (`library`) get the current one
///   (`folder`), labelled with their old name if they had none.
/// - Lookups chained one after another go on branches.
/// - A Cue sheet that took the tracks from Rip goes after the Library of
///   each encoder whose files it named (its old *Tracks are*; FLAC if
///   unset; every one if none matches), and what filed its sheet goes.
/// - Lyrics' old *.lrc files too*, when on, becomes a Lyrics file step
///   after each Library.
pub fn migrate(root: &mut Node, reg: &drt_engine::Registry) -> bool {
    let renamed = migrate_steps(root, reg);
    let cues = move_old_cues(root);
    let lrc = lrc_to_lyrics_file(root);
    renamed | cues | lrc | as_soon_is_after_every_disc(root)
}

/// Notify's *As soon as something needs you* went: a path from the step
/// for when it needs you says so at once now. A Notify set so says
/// *After every disc*, as it also did.
fn as_soon_is_after_every_disc(root: &mut Node) -> bool {
    use drt_steps::real::tell;
    let ids: Vec<String> = root
        .steps()
        .into_iter()
        .filter_map(|n| match n {
            Node::Step { id, step, params, .. } if step == "tell" && params.get("when").and_then(|s| s.resolve(None)).is_some_and(|v| v.as_str() == Some(tell::OLD_AS_SOON)) => Some(id.clone()),
            _ => None,
        })
        .collect();
    for id in &ids {
        if let Some(Node::Step { params, .. }) = root.find_mut(id) {
            params.insert("when".into(), Setting::chosen(Value::from(tell::WHEN[0])));
            eprintln!("Notify ({id}): \"As soon as something needs you\" is gone; it now says \"After every disc\"");
        }
    }
    !ids.is_empty()
}

/// Every node, so no `any`: it would stop at the first that changed.
fn migrate_all(nodes: &mut [Node], reg: &drt_engine::Registry) -> bool {
    let mut changed = false;
    for n in nodes {
        changed |= migrate_steps(n, reg);
    }
    changed
}

fn migrate_steps(node: &mut Node, reg: &drt_engine::Registry) -> bool {
    match node {
        Node::Step { step, label, .. } => {
            let now = reg.canonical(step).to_owned();
            if now == *step {
                return false;
            }
            if label.is_none() {
                let mut c = step.chars();
                *label = c.next().map(|f| f.to_uppercase().chain(c).collect());
            }
            *step = now;
            true
        }
        Node::Chain { nodes } => {
            let any = migrate_all(nodes, reg);
            branch_chained_lookups(nodes, reg) || any
        }
        Node::Branches { branches: nodes } => migrate_all(nodes, reg),
        Node::Group { body, .. } => migrate_steps(body, reg),
    }
}

/// Each encoder's kind and the id of its Library: the last Folder on the
/// encoder's own line, before that line splits into branches or turns
/// into text files (a Folder after a Cue files the sheet, not the tracks).
fn libraries(node: &Node) -> Vec<(String, String)> {
    fn line(nodes: &[Node], last: &mut Option<String>) -> bool {
        for n in nodes {
            match n {
                Node::Step { step, .. } if step == "cue" || step == "lyrics-file" || step == "artwork-files" => return false,
                Node::Step { id, step, .. } if step == "folder" || step == "library" => *last = Some(id.clone()),
                Node::Step { .. } => {}
                Node::Chain { nodes } if !line(nodes, last) => return false,
                Node::Chain { .. } => {}
                Node::Group { body, .. } if !line(std::slice::from_ref(&**body), last) => return false,
                Node::Group { .. } => {}
                Node::Branches { .. } => return false,
            }
        }
        true
    }
    fn go(node: &Node, out: &mut Vec<(String, String)>) {
        match node {
            Node::Chain { nodes } => {
                for (k, n) in nodes.iter().enumerate() {
                    if let Node::Step { step, .. } = n {
                        if drt_steps::real::encode::ALL.iter().any(|c| c.kind() == step) {
                            let mut last = None;
                            line(&nodes[k + 1..], &mut last);
                            out.extend(last.map(|l| (step.clone(), l)));
                        }
                    }
                    go(n, out);
                }
            }
            Node::Branches { branches } => branches.iter().for_each(|b| go(b, out)),
            Node::Group { body, .. } => go(body, out),
            Node::Step { .. } => {}
        }
    }
    let mut out = vec![];
    go(node, &mut out);
    out
}

/// The step kinds before the step `id` on its path.
fn upstream(root: &Node, id: &str) -> Vec<String> {
    fn go(node: &Node, mut before: Vec<String>, id: &str) -> Result<Vec<String>, Vec<String>> {
        match node {
            Node::Step { id: x, .. } if x == id => Err(before),
            Node::Step { step, .. } => {
                before.push(step.clone());
                Ok(before)
            }
            Node::Chain { nodes } => nodes.iter().try_fold(before, |b, n| go(n, b, id)),
            Node::Branches { branches } => {
                let mut all = before.clone();
                for b in branches {
                    let inner = go(b, before.clone(), id)?;
                    all.extend(inner.into_iter().skip(before.len()));
                }
                Ok(all)
            }
            Node::Group { body, .. } => go(body, before, id),
        }
    }
    go(root, vec![], id).err().unwrap_or_default()
}

/// Splices chains inside chains into them: the same recipe, but what
/// follows a step on its line is then beside it, where
/// `add_branch_after` looks. (A Hold upgrade leaves `[Hold, Library]` as
/// a chain of its own, with the Library's branches after it.)
fn flatten_chains(node: &mut Node) {
    match node {
        Node::Chain { nodes } => {
            nodes.iter_mut().for_each(flatten_chains);
            *nodes = std::mem::take(nodes)
                .into_iter()
                .flat_map(|n| match n {
                    Node::Chain { nodes } => nodes,
                    n => vec![n],
                })
                .collect();
        }
        Node::Branches { branches } => branches.iter_mut().for_each(flatten_chains),
        Node::Group { body, .. } => flatten_chains(body),
        Node::Step { .. } => {}
    }
}

/// An id for a new step of `kind`, unused in `root`.
fn fresh_id(root: &Node, kind: &str) -> String {
    let ids = root.ids();
    let base: String = kind.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}{n}") }).find(|c| !ids.contains(&c.as_str())).unwrap()
}

/// Takes out the step `id` and everything after it on its line, and gives
/// them; branches or chains left empty go too, and branches left with one
/// line become that line.
fn cut_from(node: &mut Node, id: &str) -> Option<Vec<Node>> {
    fn empty(n: &Node) -> bool {
        match n {
            Node::Step { .. } => false,
            Node::Chain { nodes: v } | Node::Branches { branches: v } => v.iter().all(empty),
            Node::Group { body, .. } => empty(body),
        }
    }
    let found = match node {
        Node::Step { .. } => None,
        Node::Chain { nodes } => match nodes.iter().position(|n| matches!(n, Node::Step { id: x, .. } if x == id)) {
            Some(k) => Some(nodes.split_off(k)),
            None => nodes.iter_mut().find_map(|n| cut_from(n, id)),
        },
        Node::Branches { branches } => match branches.iter().position(|n| matches!(n, Node::Step { id: x, .. } if x == id)) {
            Some(k) => Some(vec![branches.remove(k)]),
            None => branches.iter_mut().find_map(|n| cut_from(n, id)),
        },
        Node::Group { body, .. } => cut_from(body, id),
    };
    if found.is_some() {
        match node {
            Node::Chain { nodes: v } => v.retain(|n| !empty(n)),
            Node::Branches { branches: v } => {
                v.retain(|n| !empty(n));
                if v.len() == 1 {
                    let only = v.pop().unwrap();
                    *node = match only {
                        Node::Chain { .. } => only,
                        n => Node::Chain { nodes: vec![n] },
                    };
                }
            }
            _ => {}
        }
    }
    found
}

/// A Cue sheet from before Cue took filed files: it sat on a line from
/// Rip, with no Folder before it. It moves after the Library of the format
/// it named, taking what followed it (the Folder that filed the sheet)
/// with it to the first.
fn move_old_cues(root: &mut Node) -> bool {
    let old: Vec<(String, Option<String>, String)> = root
        .steps()
        .into_iter()
        .filter_map(|n| match n {
            Node::Step { id, step, label, params, .. } if step == "cue" => {
                Some((id.clone(), label.clone(), params.get("ext").and_then(|s| s.resolve(None)).and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "flac".into())))
            }
            _ => None,
        })
        .filter(|(id, ..)| !upstream(root, id).iter().any(|s| s == "folder" || s == "library"))
        .collect();
    for (id, label, ext) in &old {
        let Some(mut tail) = cut_from(root, id) else { continue };
        // The Cue itself, without the settings it no longer has.
        if let Some(Node::Step { params, .. }) = tail.first_mut() {
            params.clear();
        }
        // So what follows each Library is beside it.
        flatten_chains(root);
        let libs = libraries(root);
        let ext_of = |kind: &str| drt_steps::real::encode::ALL.iter().find(|c| c.kind() == kind).map(|c| c.ext());
        let matching: Vec<&(String, String)> = libs.iter().filter(|(k, _)| ext_of(k) == Some(ext.as_str())).collect();
        let targets: Vec<&(String, String)> = if matching.is_empty() { libs.iter().collect() } else { matching };
        let targets: Vec<String> = targets.into_iter().map(|(_, l)| l.clone()).collect();
        for (n, lib) in targets.iter().enumerate() {
            let line = if n == 0 {
                std::mem::take(&mut tail)
            } else {
                let mut cue = Node::step(&fresh_id(root, "cue"), "cue");
                if let Node::Step { label: l, .. } = &mut cue {
                    l.clone_from(label);
                }
                vec![cue]
            };
            let line = if line.len() == 1 { line.into_iter().next().unwrap() } else { Node::Chain { nodes: line } };
            root.add_branch_after(lib, line);
            flatten_chains(root);
        }
    }
    !old.is_empty()
}

/// Lyrics' old *.lrc files too*, on: the setting goes, and a Lyrics file
/// step follows each Library instead.
/// Gives every Notify the ntfy topic and server that were set for the
/// whole app, unless it has its own, and forgets the app-wide ones.
pub fn ntfy_to_each_notify(recipes: &mut [Recipe], s: &mut drt_engine::Settings) -> bool {
    let keys = ["ntfy_topic", "ntfy_server"];
    let old: Vec<(&str, Setting<serde_json::Value>)> = keys.iter().filter_map(|k| s.app.remove(&format!("tell.{k}")).map(|v| (*k, v))).collect();
    if old.is_empty() {
        return false;
    }
    for r in recipes.iter_mut() {
        let ids: Vec<String> = r.root.steps().into_iter().filter(|n| matches!(n, Node::Step { step, .. } if step == "tell")).map(|n| n.ids()[0].to_owned()).collect();
        for id in ids {
            if let Some(Node::Step { params, .. }) = r.root.find_mut(&id) {
                for (k, v) in &old {
                    params.entry((*k).to_owned()).or_insert_with(|| v.clone());
                }
            }
        }
    }
    true
}

fn lrc_to_lyrics_file(root: &mut Node) -> bool {
    let ids: Vec<String> = root
        .steps()
        .into_iter()
        .filter_map(|n| match n {
            Node::Step { id, step, params, .. } if step == "lyrics" && params.get("lrc").and_then(|s| s.resolve(None)).and_then(|v| v.as_bool()) == Some(true) => Some(id.clone()),
            _ => None,
        })
        .collect();
    if ids.is_empty() {
        return false;
    }
    flatten_chains(root);
    for id in &ids {
        if let Some(Node::Step { params, .. }) = root.find_mut(id) {
            params.remove("lrc");
        }
    }
    for (_, lib) in libraries(root) {
        let new_id = fresh_id(root, "lyrics-file");
        root.add_branch_after(&lib, Node::step(&new_id, "lyrics-file"));
        flatten_chains(root);
    }
    true
}

/// Lookups that were chained one after another (Choose, then Lyrics, then
/// Cover art) each go straight after what they look up, so they become
/// branches from it: Choose, then Lyrics beside Cover art.
fn branch_chained_lookups(nodes: &mut Vec<Node>, reg: &drt_engine::Registry) -> bool {
    let kind = |n: &Node| match n {
        Node::Step { step, .. } => Some(step.clone()),
        _ => None,
    };
    let mut changed = false;
    let mut i = 0;
    while i < nodes.len() {
        let Some(anchor) = kind(&nodes[i]) else {
            i += 1;
            continue;
        };
        let looks_up = |n: &Node| kind(n).and_then(|k| reg.info(&k)).is_some_and(|info| info.input == info.output && info.right_after.contains(&anchor.as_str()));
        let end = (i + 1..nodes.len()).find(|&j| !looks_up(&nodes[j])).unwrap_or(nodes.len());
        if end - (i + 1) >= 2 {
            let run: Vec<Node> = nodes.drain(i + 1..end).collect();
            nodes.insert(i + 1, Node::Branches { branches: run });
            changed = true;
        }
        i += 1;
    }
    changed
}

pub fn step_kind(root: &Node, id: &str) -> Option<String> {
    root.steps().into_iter().find_map(|n| match n {
        Node::Step { id: x, step, .. } if x == id => Some(step.clone()),
        _ => None,
    })
}

/// A value as the text in its box.
pub fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

/// Text from a box as a value of the parameter's kind.
pub fn text_value(kind: &ParamKind, text: &str) -> Option<Value> {
    let t = text.trim();
    match kind {
        ParamKind::Number => {
            let t = t.trim_start_matches('+');
            t.parse::<i64>().map(Value::from).ok().or_else(|| t.parse::<f64>().ok().map(Value::from))
        }
        ParamKind::Toggle => Some(Value::from(matches!(t, "true" | "on" | "yes"))),
        _ if t.is_empty() => None,
        _ => Some(Value::from(t)),
    }
}

/// How long a read took and its average speed: *Read in 24:51 at 8.1×*.
pub fn read_in(v: &drt_engine::StepView, length: impl Fn(u32) -> Option<f32>) -> Option<String> {
    let (a, b) = (v.record.started?, v.record.finished?);
    let took = b.checked_sub(a).filter(|t| *t > 0)?;
    let mmss = drt_steps::real::mmss(took);
    Some(match crate::pace::average(&v.items, length, took) {
        Some(x) => format!("Read in {mmss} at {}", crate::pace::speed_text(x)),
        None => format!("Read in {mmss}"),
    })
}

/// Why an edit doesn't fit, in the window's words: the step by its name,
/// and what it needs as everyday things, not ids and types.
pub fn plain_misfit(reg: &drt_engine::Registry, root: &Node, e: &drt_engine::CheckError) -> String {
    use drt_engine::{CheckError as E, ItemKind as K};
    let mut root = root.clone();
    let mut name = |id: &str| match root.find_mut(id) {
        Some(Node::Step { label: Some(l), .. }) => l.clone(),
        Some(Node::Step { step, .. }) => reg.info(step).map_or(step.clone(), |inf| inf.name.to_owned()),
        _ => id.to_owned(),
    };
    let kind = |k: &K| match k {
        K::Nothing => "nothing",
        K::Disc => "a disc",
        K::AudioDisc => "an audio CD",
        K::DataDisc => "a data disc",
        K::GameDisc => "a game disc",
        K::VideoDisc => "a video disc",
        K::Track => "tracks",
        K::Title => "videos",
        K::Dump => "an image",
        K::AudioFile => "audio files",
        K::ImageFile => "an image file",
        K::VideoFile => "video files",
        K::TextFile => "a text file",
        K::PictureFile => "pictures",
        K::AnyFile => "files",
        K::Any => "something",
        K::Message => "a message",
    };
    let fact = |f: &str| match f {
        "release" => "an album or game chosen".to_owned(),
        "candidates" => "a lookup".to_owned(),
        "filed" => "files saved".to_owned(),
        "toc" => "the track list".to_owned(),
        "read" => "a rip".to_owned(),
        "system" => "the game's system".to_owned(),
        "serial" => "a serial number".to_owned(),
        "label" => "the disc's name".to_owned(),
        "hashes" => "a check".to_owned(),
        "titles" => "the videos listed".to_owned(),
        "cover" => "cover art".to_owned(),
        "lyrics" => "lyrics".to_owned(),
        other => other.to_owned(),
    };
    let step_name = |k: &str| reg.info(k).map_or(k.to_owned(), |inf| inf.name.to_owned());
    match e {
        E::Mismatch { id, takes, .. } => format!("{} can't go here: it needs {} before it", name(id), kind(takes)),
        E::Needs { id, any_of } => format!("{} can't go here: it needs {} before it", name(id), any_of.iter().map(|f| fact(f)).collect::<Vec<_>>().join(" or ")),
        E::Repeated { id, .. } => format!("{} is already earlier on this path", name(id)),
        E::Settled { id, fact: f } => format!("{} can't go here: it has nothing to do once there's {}", name(id), fact(f)),
        E::NotRightAfter { id, after } => format!("{} goes straight after {}", name(id), after.iter().map(|k| step_name(k)).collect::<Vec<_>>().join(" or ")),
        E::UnknownStep { step, .. } => format!("{} isn't in this version of the app", step),
        E::Empty => "There's nothing to run".to_owned(),
        E::PathOnPath { id } => format!("{} already says when a step fails or needs you", name(id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_wide_ntfy_topic_goes_to_each_notify_that_has_none() {
        let mut s = Settings::default();
        s.set_app("tell", "ntfy_topic", Setting::Chosen { value: "rip-old-topic-1234".into(), at: 1 });
        let mine = Node::step("t2", "tell").with("ntfy_topic", "its-own");
        let mut recipes = vec![
            Recipe { name: "A".into(), root: drt_engine::recipe::chain([Node::step("d", "drive"), Node::step("t1", "tell")]) },
            Recipe { name: "B".into(), root: drt_engine::recipe::chain([Node::step("d", "drive"), mine]) },
        ];
        assert!(ntfy_to_each_notify(&mut recipes, &mut s));
        let topic = |r: &Recipe, id: &str| {
            r.root.steps().into_iter().find_map(|n| match n {
                Node::Step { id: i, params, .. } if i == id => params.get("ntfy_topic").and_then(|v| v.resolve(None)),
                _ => None,
            })
        };
        assert_eq!(topic(&recipes[0], "t1"), Some(serde_json::Value::from("rip-old-topic-1234")));
        assert_eq!(topic(&recipes[1], "t2"), Some(serde_json::Value::from("its-own")), "its own is kept");
        assert_eq!(s.app("tell", "ntfy_topic"), Setting::Default, "no longer app-wide");
        assert!(!ntfy_to_each_notify(&mut recipes, &mut s), "once");
    }

    #[test]
    fn notify_set_to_say_at_once_says_after_every_disc() {
        let reg = drt_steps::simulated(None);
        let mut root = drt_engine::recipe::chain([Node::step("d", "drive"), Node::step("t", "tell").with("when", drt_steps::real::tell::OLD_AS_SOON)]);
        assert!(migrate(&mut root, &reg));
        let Some(Node::Step { params, .. }) = root.find_mut("t") else { panic!() };
        assert_eq!(params["when"].resolve(None), Some(Value::from("Always")));
        assert!(!migrate(&mut root, &reg), "once");
    }

    #[test]
    fn a_drive_keeps_the_name_it_already_has() {
        let known = vec!["MATSHITA UJ240AS".to_owned(), "ATAPI eHAU324".to_owned(), String::new()];
        assert_eq!(known_name("MATSHITA BD-MLT UJ240AS", &known), "MATSHITA UJ240AS");
        assert_eq!(known_name("ATAPI   eHAU324", &known), "ATAPI eHAU324");
        assert_eq!(known_name("HL-DT-ST DVDRAM GH24NSD1", &known), "HL-DT-ST DVDRAM GH24NSD1");
    }

    #[test]
    fn typed_values_take_the_parameters_kind() {
        assert_eq!(text_value(&ParamKind::Number, "+103"), Some(Value::from(103)));
        assert_eq!(text_value(&ParamKind::Number, "-6"), Some(Value::from(-6)));
        assert_eq!(text_value(&ParamKind::Number, "x"), None);
        assert_eq!(text_value(&ParamKind::Text, "  "), None);
        assert_eq!(text_value(&ParamKind::Folder, " ~/Music "), Some(Value::from("~/Music")));
    }
}
