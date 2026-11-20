//! Steps, and what a step gets while it runs.

use crate::context::Candidate;
use crate::item::{Item, ItemKind};
use crate::lanes::LaneKind;
use crate::run::Shared;
use crate::setting::{ParamSpec, Setting, Settings};
use crate::snapshot::{Answer, ItemState, Question, StepState, StepView};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit};

/// The heading a step is listed under in the + picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Source,
    /// The kind step: establishes what the disc is and lists its items.
    Kind,
    LookUp,
    Decide,
    Read,
    Make,
    Place,
    Check,
    Tell,
}

/// Another way to try an item, offered on its row inside the tile.
#[derive(Clone, Debug, PartialEq)]
pub struct RetryOption {
    pub id: &'static str,
    pub label: &'static str,
    /// What it costs: *~1 min*, *move disc*.
    pub cost: &'static str,
}

#[derive(Clone, Debug)]
pub struct StepInfo {
    pub kind: &'static str,
    pub name: &'static str,
    /// The tile's second line when no parameter gives it.
    pub from: &'static str,
    /// The parameter whose value is the tile's second line (a folder).
    pub from_param: Option<&'static str>,
    pub group: Group,
    pub input: ItemKind,
    pub output: ItemKind,
    pub params: Vec<ParamSpec>,
    pub retries: Vec<RetryOption>,
    /// Ends a disc and leads back to the start of the recipe, for the next
    /// disc. Drawn as a line back into the first tile.
    pub loops_back: bool,
    /// What this step needs to have been found before it: every one of
    /// these, each met by any one of its facts. Choose needs candidates or
    /// titles from a lookup; Cover art needs the release Choose settled;
    /// LibCrypt needs the image filed, and Redump's name for it.
    pub needs: Vec<Vec<&'static str>>,
    /// What it finds, for the steps after it.
    pub gives: Vec<&'static str>,
    /// Gives whatever it's given, unchanged in kind (Hold): takes `Any`
    /// and hands the same kind on.
    pub passes_through: bool,
    /// May appear more than once on a path (Folder: Review, then Library).
    pub repeatable: bool,
    /// Only makes sense before any of these is found: a lookup that
    /// identifies the disc isn't offered once Choose has settled it.
    pub unless: Vec<&'static str>,
    /// Must come straight after one of these steps, whose item it looks
    /// up: a lookup reads the disc itself, not what another lookup did.
    pub right_after: Vec<&'static str>,
    /// The tile's second line worked out from the settings in effect
    /// ("V0, joint stereo"), given how to read one.
    pub summary: Option<crate::setting::Words>,
}

impl StepInfo {
    pub fn new(kind: &'static str, name: &'static str, group: Group, input: ItemKind, output: ItemKind) -> Self {
        StepInfo {
            kind,
            name,
            from: "",
            from_param: None,
            group,
            input,
            output,
            params: vec![],
            retries: vec![],
            loops_back: false,
            summary: None,
            needs: vec![],
            gives: vec![],
            passes_through: false,
            repeatable: false,
            unless: vec![],
            right_after: vec![],
        }
    }

    pub fn from(mut self, from: &'static str) -> Self {
        self.from = from;
        self
    }

    pub fn from_param(mut self, key: &'static str) -> Self {
        self.from_param = Some(key);
        self
    }

    pub fn param(mut self, p: ParamSpec) -> Self {
        self.params.push(p);
        self
    }

    /// Needs any one of these to have been found before it. Called again,
    /// needs that too.
    pub fn needs(mut self, any_of: &[&'static str]) -> Self {
        self.needs.push(any_of.to_vec());
        self
    }

    pub fn repeatable(mut self) -> Self {
        self.repeatable = true;
        self
    }

    /// Not offered once any of these is found.
    pub fn unless(mut self, facts: &[&'static str]) -> Self {
        self.unless = facts.to_vec();
        self
    }

    /// Only straight after one of these steps (or on a branch from it).
    pub fn right_after(mut self, kinds: &[&'static str]) -> Self {
        self.right_after = kinds.to_vec();
        self
    }

    pub fn passes_through(mut self) -> Self {
        self.passes_through = true;
        self
    }

    pub fn gives(mut self, facts: &[&'static str]) -> Self {
        self.gives = facts.to_vec();
        self
    }

    pub fn summary(mut self, f: crate::setting::Words) -> Self {
        self.summary = Some(f);
        self
    }

    /// The value in effect for one of this step's settings, wherever it
    /// lives: the node's, the recipe's drive's, or the app's.
    /// `flow`: what arrives at the step, for a setting that follows it.
    pub fn resolve_in(&self, key: &str, params: &std::collections::BTreeMap<String, Setting<Value>>, settings: &Settings, drive: &str, flow: Option<&crate::check::Flow>) -> Option<Value> {
        let spec = self.params.iter().find(|p| p.key == key)?;
        spec.resolve_at(&settings.lookup(spec, self.kind, params, drive), flow)
    }

    pub fn loops_back(mut self) -> Self {
        self.loops_back = true;
        self
    }

    pub fn retry(mut self, id: &'static str, label: &'static str, cost: &'static str) -> Self {
        self.retries.push(RetryOption { id, label, cost });
        self
    }
}

/// One stage of a recipe. A step reads items from its inlet and sends
/// items to its outlet; when it returns, its outlet closes and the steps
/// after it see the end of their input.
#[async_trait]
pub trait Step: Send + Sync + 'static {
    fn info(&self) -> StepInfo;
    async fn run(&self, inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()>;

    /// Looks up a findable parameter without running: `drive` is the
    /// recipe's drive. `measure` allows slower ways that use the disc in
    /// the drive. Gives the value and where it came from.
    async fn find(&self, _key: &str, _drive: &str, _measure: bool) -> Option<(Value, String)> {
        None
    }
}

/// Every step there is, by kind.
#[derive(Clone, Default)]
pub struct Registry {
    steps: HashMap<String, (Arc<dyn Step>, Arc<StepInfo>)>,
    /// Old names for steps, so recipes saved with them still load.
    aliases: HashMap<String, String>,
}

impl Registry {
    pub fn add(&mut self, step: impl Step) -> &mut Self {
        let info = step.info();
        self.steps.insert(info.kind.to_owned(), (Arc::new(step), Arc::new(info)));
        self
    }

    /// `old` is another name for the step `new`.
    pub fn alias(&mut self, old: &str, new: &str) -> &mut Self {
        self.aliases.insert(old.into(), new.into());
        self
    }

    /// The step's current name, for one saved under an old one.
    pub fn canonical<'a>(&'a self, kind: &'a str) -> &'a str {
        self.aliases.get(kind).map_or(kind, String::as_str)
    }

    pub fn get(&self, kind: &str) -> Option<(Arc<dyn Step>, Arc<StepInfo>)> {
        self.steps.get(self.canonical(kind)).cloned()
    }

    pub fn info(&self, kind: &str) -> Option<Arc<StepInfo>> {
        self.steps.get(self.canonical(kind)).map(|(_, i)| i.clone())
    }

    pub fn infos(&self) -> impl Iterator<Item = &StepInfo> {
        self.steps.values().map(|(_, i)| i.as_ref())
    }
}

/// A request to try an item again, from the item's *Retry* menu.
#[derive(Clone, Debug, PartialEq)]
pub struct RetryRequest {
    pub item: String,
    pub option: String,
}

/// Where a step's items come from. The step's tile turns busy when the
/// first one arrives.
pub struct Inlet {
    pub(crate) rx: mpsc::Receiver<Item>,
    pub(crate) cx: Cx,
    pub(crate) started: bool,
}

impl Inlet {
    /// The next item, or `None` when the steps before have finished or the
    /// run was stopped.
    pub async fn recv(&mut self) -> Option<Item> {
        let item = tokio::select! {
            i = self.rx.recv() => i,
            _ = self.cx.shared.cancel.cancelled() => None,
        }?;
        let first = !self.started;
        self.started = true;
        // A tray opened before this item came is no concern of its.
        self.cx.drive_back();
        let kind = item.kind;
        let parts = if item.index.is_some() { item.context().parts() as usize } else { 0 };
        self.cx.arrived.lock().unwrap().insert(item.id.clone(), item.path.clone());
        self.cx.view(|v| {
            if first {
                if matches!(v.state, StepState::Idle | StepState::Wait) {
                    // What it waited for is behind it.
                    if v.state == StepState::Wait {
                        v.detail.clear();
                    }
                    v.state = StepState::Busy;
                    v.status.clear();
                }
                v.record.started.get_or_insert(crate::setting::now());
            }
            // What came in; the start of a recipe brings nothing worth saying.
            if kind != ItemKind::Nothing {
                v.record.came_in(kind);
            }
            // A disc's track says how many there are: the rest are to come.
            if parts > 0 && matches!(kind, ItemKind::Track | ItemKind::AudioFile) {
                v.total = Some(v.total.unwrap_or(0).max(parts));
            }
        });
        Some(item)
    }

    /// Every item, once the steps before have finished.
    pub async fn all(&mut self) -> Vec<Item> {
        let mut out = Vec::new();
        while let Some(i) = self.recv().await {
            out.push(i);
        }
        out
    }
}

/// Where a step's items go: every step after it.
#[derive(Clone)]
pub struct Outlet {
    pub(crate) to: Vec<mpsc::Sender<Item>>,
    pub(crate) cx: Option<Cx>,
}

impl Outlet {
    pub async fn send(&self, mut item: Item) {
        if let Some(cx) = &self.cx {
            // What this step added to the item is marked as its own, and
            // goes into the run's record for the window (which no step reads).
            if item.context.untagged() {
                item.context_mut().tag(&cx.id);
            }
            cx.shared.context.lock().unwrap().merge(&item.context);
            if !self.to.is_empty() && item.kind != ItemKind::Nothing {
                let kind = item.kind;
                cx.view(|v| v.record.went_out(kind));
            }
            // A file at a path it didn't come in with: this step made it.
            if let Some(path) = item.path.as_ref().filter(|_| item.kind.is_file()) {
                let came = cx.arrived.lock().unwrap().get(&item.id).cloned().flatten();
                if came.as_ref() != Some(path) {
                    cx.made(path);
                }
            }
        }
        for tx in &self.to {
            // A step after this one may have stopped early; that's its business.
            let _ = tx.send(item.clone()).await;
        }
    }
}

tokio::task_local! {
    /// The step whose task this is, for output that arrives far from its
    /// `Cx` (a tool's lines).
    pub(crate) static CURRENT: Cx;
}

/// Runs `f` as part of a step: for work a step spawns off on its own
/// (an encoder's tracks), so a tool's output there is still the step's.
pub async fn within<F: std::future::Future>(cx: Cx, f: F) -> F::Output {
    CURRENT.scope(cx, f).await
}

/// A line a tool printed, for the step running it (nothing outside a
/// step). See [`Cx::raw`].
pub fn raw_here(line: &str) {
    let _ = CURRENT.try_with(|cx| cx.raw(line));
}

/// What a running step can see and do.
#[derive(Clone)]
pub struct Cx {
    pub(crate) id: String,
    pub(crate) shared: Arc<Shared>,
    pub(crate) info: Arc<StepInfo>,
    pub(crate) params: Arc<BTreeMap<String, Setting<Value>>>,
    /// What arrives at the step, by the recipe: what its settings that
    /// follow it go by.
    pub(crate) flow: Option<Arc<crate::check::Flow>>,
    pub(crate) retries: Arc<tokio::sync::Mutex<mpsc::Receiver<RetryRequest>>>,
    /// The path each item came in with, so a file sent on at a new path is
    /// one this step made (a step passing files through made none).
    pub(crate) arrived: Arc<std::sync::Mutex<HashMap<String, Option<std::path::PathBuf>>>>,
    /// The tray openings this step knows of (see [`Cx::drive_released`]).
    pub(crate) tray_seen: Arc<std::sync::atomic::AtomicU64>,
    /// A read of this step's was stopped because the tray opened.
    pub(crate) tray_cut: Arc<std::sync::atomic::AtomicBool>,
}

/// A read holding the drive: the tray waits for it to be dropped before
/// opening. See [`Cx::drive_read`].
pub struct DriveRead {
    shared: Arc<Shared>,
    seen: u64,
    cut: Arc<std::sync::atomic::AtomicBool>,
}

impl DriveRead {
    /// Resolves when the read should stop: the run stopped, or the tray
    /// was asked to open. Give it to the tool as its `stop`.
    pub fn stop(&self) -> impl std::future::Future<Output = ()> + Send + '_ {
        let mut tray = self.shared.tray.subscribe();
        let seen = self.seen;
        async move {
            tokio::select! {
                _ = tray.wait_for(|g| *g != seen) => self.cut.store(true, std::sync::atomic::Ordering::SeqCst),
                _ = self.shared.cancel.cancelled() => {}
            }
        }
    }

    /// Whether the tray was asked to open since this step last went on.
    pub fn tray_opened(&self) -> bool {
        *self.shared.tray.borrow() != self.seen
    }
}

impl Drop for DriveRead {
    fn drop(&mut self) {
        self.shared.holding.send_modify(|n| *n = n.saturating_sub(1));
    }
}

impl Cx {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Changes this step's tile.
    pub fn view(&self, f: impl FnOnce(&mut StepView)) {
        let id = self.id.clone();
        self.shared.update(|s| {
            if let Some(v) = s.steps.get_mut(&id) {
                f(v)
            }
        });
    }

    pub fn status(&self, status: impl Into<String>, detail: impl Into<String>) {
        self.outcome(status, "", detail);
    }

    /// The outcome with one qualifier beside it (*17 tracks* · *43:45*),
    /// and the detail line.
    pub fn outcome(&self, status: impl Into<String>, qualifier: impl Into<String>, detail: impl Into<String>) {
        let (status, qualifier, detail) = (status.into(), qualifier.into(), detail.into());
        self.view(|v| {
            v.status = status;
            v.qualifier = qualifier;
            v.detail = detail;
        });
    }

    /// How many items it will handle in all, once it knows.
    pub fn expect(&self, n: usize) {
        self.view(|v| v.total = Some(n));
    }

    /// An item done and verified against something independent of the
    /// read (AccurateRip, a catalogue's hashes).
    pub fn verified(&self, item: &Item, note: impl Into<String>) {
        let note = note.into();
        self.view(|v| {
            let row = v.item_mut(&item.id, &item.label);
            row.state = ItemState::Ok;
            row.note = note;
            row.verified = true;
        });
    }

    pub fn state(&self, state: StepState) {
        self.view(|v| v.state = state);
    }

    pub fn progress(&self, p: Option<f32>) {
        self.view(|v| v.progress = p);
    }

    pub fn item(&self, item: &Item, state: ItemState, note: impl Into<String>) {
        let note = note.into();
        self.view(|v| {
            let row = v.item_mut(&item.id, &item.label);
            row.state = state;
            row.note = note;
            row.verified = false;
        });
    }

    /// The value in effect for a parameter, wherever it lives.
    pub fn param(&self, key: &str) -> Option<Value> {
        let spec = self.info.params.iter().find(|p| p.key == key)?;
        let setting = self.shared.settings.lock().unwrap().lookup(spec, self.info.kind, &self.params, &self.shared.drive);
        spec.resolve_at(&setting, self.flow.as_deref())
    }

    pub fn text(&self, key: &str) -> String {
        match self.param(key) {
            Some(Value::String(s)) => s,
            Some(v) => v.to_string(),
            None => String::new(),
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        self.param(key).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// Asks you, and waits for the answer. The tile turns amber and the
    /// question opens in the sidebar.
    pub async fn ask(&self, q: Question) -> Result<Answer> {
        self.shared.needed_you.store(true, std::sync::atomic::Ordering::Relaxed);
        // One question at a time: another step's stays up until answered.
        let _turn = tokio::select! {
            t = self.shared.asking.lock() => t,
            _ = self.shared.cancel.cancelled() => bail!("stopped"),
        };
        let (tx, rx) = oneshot::channel();
        self.shared.answers.lock().unwrap().insert(self.id.clone(), tx);
        let (id, prompt) = (self.id.clone(), q.prompt.clone());
        self.shared.update(|s| {
            s.question = Some((id.clone(), q));
            if let Some(v) = s.steps.get_mut(&id) {
                v.state = StepState::Needs;
                v.status = prompt;
            }
        });
        let answer = tokio::select! {
            a = rx => a.ok(),
            _ = self.shared.cancel.cancelled() => None,
        };
        let id = self.id.clone();
        self.shared.update(|s| {
            if s.question.as_ref().is_some_and(|(q, _)| *q == id) {
                s.question = None;
            }
            if let Some(v) = s.steps.get_mut(&id) {
                v.state = StepState::Busy;
            }
        });
        match answer {
            Some(a) => Ok(a),
            None => bail!("stopped"),
        }
    }

    /// Waits for room in a lane.
    pub async fn lane(&self, lane: LaneKind) -> OwnedSemaphorePermit {
        self.shared.lanes.enter(&lane, &self.shared.drive).await
    }

    pub fn stopped(&self) -> bool {
        self.shared.cancel.is_cancelled()
    }

    /// Whether any step in this run has waited on you so far: asked a
    /// question, or held something for a *Retry*.
    pub fn needed_you(&self) -> bool {
        self.shared.needed_you.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The next *Retry* asked of this step, or `None` if the run stops first.
    pub async fn next_retry(&self) -> Option<RetryRequest> {
        self.shared.needed_you.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut rx = self.retries.lock().await;
        let r = tokio::select! {
            r = rx.recv() => r,
            _ = self.shared.cancel.cancelled() => None,
        };
        // Asked to go on: a tray opened before is behind it now.
        if r.is_some() {
            self.drive_back();
        }
        r
    }

    /// Holds the drive for a read: while it's held, opening the tray waits,
    /// having stopped the read through [`DriveRead::stop`]. Drop it once
    /// the tool has exited.
    pub fn drive_read(&self) -> DriveRead {
        self.shared.holding.send_modify(|n| *n += 1);
        DriveRead { shared: self.shared.clone(), seen: self.tray_seen.load(std::sync::atomic::Ordering::SeqCst), cut: self.tray_cut.clone() }
    }

    /// Whether the tray was asked to open since this step was last given
    /// something to do (an item, or a *Retry*): it starts no read till then.
    pub fn drive_released(&self) -> bool {
        *self.shared.tray.borrow() != self.tray_seen.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The tray openings so far are behind this step: it may read again.
    pub fn drive_back(&self) {
        self.tray_seen.store(*self.shared.tray.borrow(), std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether a read of this step's was stopped by the tray opening.
    pub fn tray_cut(&self) -> bool {
        self.tray_cut.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Shows what the drive is doing on the recipe's drive tile. Its
    /// outcome (*CD in*) stays; what it's doing (*Reading at 8×*) is its
    /// detail while it works. Done reading (`Idle`, *Waiting*), the disc is
    /// still in: the tile is done until the next read.
    pub fn drive_says(&self, state: StepState, status: &str, detail: &str) {
        self.shared.update(|s| {
            for v in s.steps.values_mut().filter(|v| v.step == "drive") {
                match state {
                    StepState::Busy => {
                        v.state = state;
                        v.detail = if detail.is_empty() { status.to_owned() } else { format!("{status} {detail}") };
                    }
                    StepState::Idle if status == "Waiting" => {
                        v.state = if v.status.is_empty() { StepState::Idle } else { StepState::Ok };
                        v.detail.clear();
                    }
                    _ => {
                        v.state = state;
                        v.status = status.into();
                        v.qualifier.clear();
                        v.detail = detail.into();
                    }
                }
            }
        });
    }

    /// This run's working folder, where rips and encodes land before
    /// they're filed. Made on first use; kept after the run.
    pub fn work(&self) -> Result<std::path::PathBuf> {
        std::fs::create_dir_all(&self.shared.work)?;
        Ok(self.shared.work.clone())
    }

    /// What an *Approve* for this step wakes: a step that waits for you
    /// to let things through (Hold) waits on this.
    pub fn approval(&self) -> Arc<tokio::sync::Notify> {
        self.shared.approvals.lock().unwrap().entry(self.id.clone()).or_insert_with(|| Arc::new(tokio::sync::Notify::new())).clone()
    }

    /// The run's stop signal, for steps that wait on things of their own.
    pub fn cancelled(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
        self.shared.cancel.cancelled()
    }

    /// A line for the run's log and this step's record.
    pub fn log(&self, line: impl Into<String>) {
        let line = line.into();
        self.shared.log.lock().unwrap().push(format!("{}: {}", self.info.name, line));
        self.view(|v| v.record.log.push(crate::snapshot::LogLine { at: crate::setting::now(), line }));
    }

    /// A line its tool printed: kept (the last RAW_KEPT) for its sidebar,
    /// and all of them in `tool-output.txt` in the run's working folder.
    /// Doesn't touch the snapshot, so a chatty tool costs little.
    pub fn raw(&self, line: impl Into<String>) {
        let line = line.into();
        self.shared.raw_line(&self.id, &line);
    }

    /// The candidates that reached Choose, for its record: what it chose
    /// among.
    pub fn options(&self, options: &[Candidate]) {
        let options = options.to_vec();
        self.view(|v| v.record.options = Some(options));
    }

    /// What Choose settled on (`None`: nothing), and why in a few words.
    pub fn decided(&self, chosen: Option<&Candidate>, why: impl Into<String>) {
        let d = crate::snapshot::Decision { chosen: chosen.map(|c| c.id.clone()), title: chosen.map(|c| c.title.clone()).unwrap_or_default(), why: why.into() };
        self.view(|v| v.record.decided = Some(d));
    }

    /// What this step found, in its own shape, for its sidebar (`None`
    /// takes it away).
    pub fn report(&self, report: Option<Value>) {
        self.view(|v| v.record.report = report);
    }

    /// A file this step wrote; its size is read now (one already noted
    /// with a size keeps it if the file can't be read).
    pub fn made(&self, path: &std::path::Path) {
        let size = std::fs::metadata(path).map(|m| m.len()).ok();
        self.note_file(path.display().to_string(), size);
    }

    /// A file this step wrote, and its size, said rather than read (the
    /// simulated steps, whose files aren't there).
    pub fn made_sized(&self, path: &std::path::Path, size: u64) {
        self.note_file(path.display().to_string(), Some(size));
    }

    fn note_file(&self, path: String, size: Option<u64>) {
        self.view(|v| match (v.record.files.iter_mut().find(|f| f.path == path), size) {
            (Some(f), Some(s)) => f.size = s,
            (Some(_), None) => {}
            (None, s) => v.record.files.push(crate::snapshot::Made { path, size: s.unwrap_or(0) }),
        });
    }

    /// Sleeps, scaled by the engine's time scale (0 in tests). Fails if the
    /// run stops meanwhile.
    pub async fn sleep(&self, ms: u64) -> Result<()> {
        let d = Duration::from_secs_f64(ms as f64 * self.shared.time_scale / 1000.0);
        if d.is_zero() {
            tokio::task::yield_now().await;
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(d) => Ok(()),
            _ = self.shared.cancel.cancelled() => bail!("stopped"),
        }
    }
}
