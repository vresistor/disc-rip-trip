//! Running a recipe.
//!
//! The tree is compiled to a graph: one Tokio task per step, a bounded
//! channel per wire. A step with several wires in gets a join that lets an
//! item through once it has arrived along every one of them, so what
//! follows branches sees each item once, after all the branches are done
//! with it. A held step gets a gate that keeps its items until you approve.

use crate::check::check;
use crate::context::Context;
use crate::item::{Item, ItemKind};
use crate::lanes::Lanes;
use crate::recipe::{Node, Recipe, Side};
use crate::setting::{Scope, Setting, Settings};
use crate::snapshot::{idle_status, Answer, Command, Snapshot, StepState, StepView};
use crate::step::{Cx, Inlet, Outlet, Registry, RetryRequest};
use anyhow::{bail, Result};
use futures::stream::{self, StreamExt};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot, watch, Notify};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const WIRE: usize = 64;

/// What every run shares: the steps, the lanes, the settings that belong to
/// drives. One per app.
pub struct Engine {
    pub registry: Registry,
    pub lanes: Arc<Lanes>,
    pub settings: Arc<Mutex<Settings>>,
    /// Multiplies every simulated delay. 1 is real time; 0 makes the
    /// simulation instant, for tests.
    pub time_scale: f64,
    /// Where each run gets a working folder of its own.
    pub work_root: std::path::PathBuf,
}

/// A missing setting: which tile, which parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Missing {
    pub step: String,
    pub key: &'static str,
    pub label: &'static str,
}

/// Where the app keeps what it can fetch or make again:
/// `$XDG_CACHE_HOME/disc-rip-trip`, or `~/.cache/disc-rip-trip`.
pub fn cache_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?;
    Some(base.join("disc-rip-trip"))
}

impl Engine {
    pub fn new(registry: Registry) -> Self {
        let work_root = cache_dir().unwrap_or_else(|| std::env::temp_dir().join("disc-rip-trip")).join("runs");
        Engine { registry, lanes: Arc::new(Lanes::default()), settings: Arc::default(), time_scale: 1.0, work_root }
    }

    /// The drive a recipe reads from: its drive step's `drive` parameter.
    pub fn drive_of(recipe: &Recipe) -> String {
        recipe
            .root
            .steps()
            .into_iter()
            .find_map(|n| match n {
                Node::Step { step, params, .. } if step == "drive" => params.get("drive").and_then(|s| match s {
                    Setting::Chosen { value, .. } | Setting::Detected { value, .. } => value.as_str().map(str::to_owned),
                    _ => None,
                }),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Required settings nobody has given yet. A recipe with any can't start;
    /// each makes its tile ask.
    pub fn missing(&self, recipe: &Recipe) -> Vec<Missing> {
        let upgraded = {
            let mut r = recipe.clone();
            r.upgrade();
            r
        };
        let recipe = &upgraded;
        let drive = Self::drive_of(recipe);
        let settings = self.settings.lock().unwrap();
        let mut out = Vec::new();
        for node in recipe.root.steps() {
            let Node::Step { id, step, params, .. } = node else { continue };
            let Some(info) = self.registry.info(step) else { continue };
            let kinds: Vec<&str> = recipe
                .root
                .steps()
                .into_iter()
                .filter_map(|n| match n {
                    Node::Step { step, .. } => Some(step.as_str()),
                    _ => None,
                })
                .collect();
            for p in info.params.iter().filter(|p| p.required() && (p.only_with.is_empty() || p.only_with.iter().any(|k| kinds.contains(k)))) {
                let set = settings.lookup(p, info.kind, params, &drive);
                if set.resolve(None).is_none() {
                    out.push(Missing { step: id.clone(), key: p.key, label: p.label });
                }
            }
        }
        out
    }

    /// Looks up every missing findable setting the recipe has, as soon as
    /// it has what the lookup needs (a drive's offset once there's a drive),
    /// and records each found one as detected. Gives what's still missing,
    /// which is what has to be asked for.
    pub async fn find_missing(&self, recipe: &Recipe, measure: bool) -> Vec<Missing> {
        let drive = Self::drive_of(recipe);
        for m in self.missing(recipe) {
            let Some(Node::Step { step, .. }) = recipe.root.steps().into_iter().find(|n| matches!(n, Node::Step { id, .. } if *id == m.step)) else { continue };
            let Some((s, info)) = self.registry.get(step) else { continue };
            let Some(spec) = info.params.iter().find(|p| p.key == m.key) else { continue };
            // Only what lives on the drive can be recorded without editing the recipe.
            if !spec.findable || spec.scope != Scope::Drive || drive.is_empty() {
                continue;
            }
            if let Some((value, source)) = s.find(m.key, &drive, measure).await {
                self.settings.lock().unwrap().set_drive(&drive, m.key, Setting::detected(value, source));
            }
        }
        self.missing(recipe)
    }

    /// Finds every findable drive setting the recipe's steps have again,
    /// even ones already known, and records what's found: *Measure* on a
    /// drive whose offset came from a list. Gives how many were found.
    pub async fn find_again(&self, recipe: &Recipe, measure: bool) -> usize {
        let drive = Self::drive_of(recipe);
        if drive.is_empty() {
            return 0;
        }
        let mut found = 0;
        for node in recipe.root.steps() {
            let Node::Step { step, .. } = node else { continue };
            let Some((s, info)) = self.registry.get(step) else { continue };
            for spec in info.params.iter().filter(|p| p.findable && p.scope == Scope::Drive) {
                if let Some((value, source)) = s.find(spec.key, &drive, measure).await {
                    self.settings.lock().unwrap().set_drive(&drive, spec.key, Setting::detected(value, source));
                    found += 1;
                }
            }
        }
        found
    }

    /// The recipe as it looks before it starts: every tile idle, and the ones
    /// missing a setting asking for it.
    pub fn idle(&self, recipe: &Recipe) -> Snapshot {
        let upgraded = {
            let mut r = recipe.clone();
            r.upgrade();
            r
        };
        let recipe = &upgraded;
        let mut g = Vec::new();
        compile(&recipe.root, &[], &mut g);
        let mut snap = self.views(recipe, &g);
        for m in self.missing(recipe) {
            if let Some(v) = snap.steps.get_mut(&m.step) {
                v.state = StepState::Needs;
                v.status = format!("{}?", m.label);
            }
        }
        snap
    }

    fn views(&self, recipe: &Recipe, g: &[GNode]) -> Snapshot {
        let drive = Self::drive_of(recipe);
        let settings = self.settings.lock().unwrap();
        let flows = crate::check::flows_in(&recipe.root, &self.registry);
        let mut steps = BTreeMap::new();
        for n in g {
            let flow = flows.get(&n.id);
            let info = self.registry.info(&n.step);
            let name = n.label.clone().or_else(|| info.as_ref().map(|i| i.name.to_owned())).unwrap_or_else(|| n.step.clone());
            let from = info
                .as_ref()
                .and_then(|i| {
                    let f = i.summary?;
                    Some(f(&|key: &str| i.resolve_in(key, &n.params, &settings, &drive, flow)))
                })
                .or_else(|| {
                    info.as_ref().and_then(|i| {
                        let key = i.from_param?;
                        let spec = i.params.iter().find(|p| p.key == key)?;
                        let set = settings.lookup(spec, i.kind, &n.params, &drive);
                        spec.resolve_at(&set, flow).map(|v| v.as_str().map(str::to_owned).unwrap_or(v.to_string()))
                    })
                })
                .or_else(|| info.as_ref().map(|i| i.from.to_owned()))
                .unwrap_or_default();
            let preds = n.preds.iter().map(|&p| g[p].id.clone()).collect();
            steps.insert(
                n.id.clone(),
                StepView {
                    step: n.step.clone(),
                    name,
                    from,
                    state: if n.hold { StepState::Hold } else { StepState::Idle },
                    status: String::new(),
                    qualifier: String::new(),
                    detail: String::new(),
                    progress: None,
                    now: None,
                    items: vec![],
                    total: None,
                    preds,
                    hold: n.hold,
                    record: Default::default(),
                },
            );
        }
        for n in g {
            let status = match &n.side {
                Some((from, side)) => {
                    let name = steps.get(from).map_or(from.clone(), |v| v.name.to_lowercase());
                    match side {
                        Side::Fails => format!("If {name} fails"),
                        Side::NeedsYou => format!("If {name} needs you"),
                    }
                }
                None if n.hold => "Holding".to_owned(),
                None => idle_status(&steps[&n.id].preds, &steps),
            };
            steps.get_mut(&n.id).unwrap().status = status;
        }
        let mut snap = Snapshot { recipe: recipe.name.clone(), order: g.iter().map(|n| n.id.clone()).collect(), steps, ..Default::default() };
        snap.derive_waits();
        snap
    }

    /// Starts a recipe. Fails if it doesn't type-check or a setting is missing.
    pub fn start(&self, recipe: &Recipe) -> Result<Run> {
        let upgraded = {
            let mut r = recipe.clone();
            r.upgrade();
            r
        };
        let recipe = &upgraded;
        check(&recipe.root, &self.registry)?;
        let missing = self.missing(recipe);
        if !missing.is_empty() {
            // By the names the window shows: the setting's, then its step's.
            let name = |id: &str| {
                recipe.root.steps().into_iter().find_map(|n| match n {
                    Node::Step { id: i, label: Some(l), .. } if i == id => Some(l.clone()),
                    Node::Step { id: i, step, .. } if i == id => Some(self.registry.info(step).map_or(step.clone(), |inf| inf.name.to_owned())),
                    _ => None,
                })
            };
            let what: Vec<_> = missing.iter().map(|m| format!("{}, on {}", m.label, name(&m.step).unwrap_or_else(|| m.step.clone()))).collect();
            bail!("Missing: {}", what.join("; "));
        }

        let mut g = Vec::new();
        compile(&recipe.root, &[], &mut g);
        let snap = self.views(recipe, &g);
        let (tx, rx) = watch::channel(snap.clone());
        let slug: String = recipe.name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let work = self.work_root.join(format!("{slug}-{}", crate::setting::now()));
        let shared = Arc::new(Shared {
            snap: Mutex::new(snap),
            tx,
            context: Mutex::default(),
            settings: self.settings.clone(),
            lanes: self.lanes.clone(),
            cancel: CancellationToken::new(),
            answers: Mutex::default(),
            asking: tokio::sync::Mutex::default(),
            needed_you: Default::default(),
            retries: Mutex::default(),
            approvals: Mutex::default(),
            drive: Self::drive_of(recipe),
            time_scale: self.time_scale,
            log: Mutex::default(),
            raw: Mutex::default(),
            raw_file: Mutex::default(),
            tray: watch::channel(0).0,
            holding: watch::channel(0).0,
            work,
        });

        // One channel per wire.
        let mut ins: Vec<Vec<mpsc::Receiver<Item>>> = (0..g.len()).map(|_| vec![]).collect();
        let mut outs: Vec<Vec<mpsc::Sender<Item>>> = (0..g.len()).map(|_| vec![]).collect();
        let mut starts = Vec::new();
        // Into each step's paths for when it fails or needs you: the
        // watcher sends down them.
        let mut to_sides: Vec<((String, Side), mpsc::Sender<Item>)> = Vec::new();
        for (i, n) in g.iter().enumerate() {
            if let (true, Some(from)) = (n.preds.is_empty(), &n.side) {
                let (t, r) = mpsc::channel(WIRE);
                to_sides.push((from.clone(), t));
                ins[i].push(r);
            } else if n.preds.is_empty() {
                let (t, r) = mpsc::channel(1);
                starts.push(t);
                ins[i].push(r);
            }
            for &p in &n.preds {
                let (t, r) = mpsc::channel(WIRE);
                outs[p].push(t);
                ins[i].push(r);
            }
        }

        let mut flows = crate::check::flows_in(&recipe.root, &self.registry);
        let mut tasks: Vec<JoinHandle<()>> = Vec::new();
        // The steps on those paths: done once the rest is, and what's
        // been sent them is sent.
        let mut side_tasks: Vec<JoinHandle<()>> = Vec::new();
        for (i, n) in g.iter().enumerate().rev() {
            let (step, info) = self.registry.get(&n.step).expect("checked above");
            let (retry_tx, retry_rx) = mpsc::channel(16);
            shared.retries.lock().unwrap().insert(n.id.clone(), retry_tx);
            let cx = Cx {
                id: n.id.clone(),
                shared: shared.clone(),
                info,
                params: Arc::new(n.params.clone()),
                flow: flows.remove(&n.id).map(Arc::new),
                retries: Arc::new(tokio::sync::Mutex::new(retry_rx)),
                arrived: Arc::default(),
                tray_seen: Arc::default(),
                tray_cut: Arc::default(),
            };
            let list = if n.on_side { &mut side_tasks } else { &mut tasks };
            let rx = join(std::mem::take(&mut ins[i]), list);
            let inlet = Inlet { rx, cx: cx.clone(), started: false };
            let outlet = Outlet { to: std::mem::take(&mut outs[i]), cx: Some(cx.clone()) };
            list.push(tokio::spawn(async move {
                let result = crate::step::CURRENT.scope(cx.clone(), step.run(inlet, outlet, cx.clone())).await;
                finish(&cx, result);
            }));
        }
        for t in starts {
            let _ = t.try_send(Item::new("start", ItemKind::Nothing, "", None, None));
        }

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(32);
        let s = shared.clone();
        let commands = tokio::spawn(async move {
            while let Some(c) = cmd_rx.recv().await {
                s.command(c);
            }
        });
        let (end, ended) = oneshot::channel();
        let watcher = (!to_sides.is_empty()).then(|| tokio::spawn(watch_sides(shared.clone(), to_sides, ended)));
        let s = shared.clone();
        let done = tokio::spawn(async move {
            for t in tasks {
                let _ = t.await;
            }
            // The rest is done: what's still to say is said, and the
            // paths for it close.
            let _ = end.send(());
            if let Some(w) = watcher {
                let _ = w.await;
            }
            for t in side_tasks {
                let _ = t.await;
            }
            commands.abort();
            if let Some(mut w) = s.raw_file.lock().unwrap().take() {
                use std::io::Write;
                let _ = w.flush();
            }
            s.update(|snap| {
                snap.finished = true;
                snap.question = None;
                let stopped = snap.stopped;
                for v in snap.steps.values_mut().filter(|v| v.step == "drive" && !stopped && matches!(v.state, StepState::Busy | StepState::Idle | StepState::Ok)) {
                    v.state = StepState::Ok;
                    if v.status.is_empty() {
                        v.status = "Done".into();
                    }
                    v.detail.clear();
                }
            });
            // A run that finished cleanly has filed everything it made, so
            // its working folder (a CD's WAVs, a disc's image) goes. One
            // that was stopped or has an item that went wrong keeps it, for
            // a look or another try.
            let clean = {
                let snap = s.snap.lock().unwrap();
                !snap.stopped && snap.steps.values().all(|v| v.state != StepState::Bad && v.count(crate::snapshot::ItemState::Bad) == 0)
            };
            if clean && s.work.exists() {
                if let Err(e) = std::fs::remove_dir_all(&s.work) {
                    s.log.lock().unwrap().push(format!("couldn't tidy {}: {e}", s.work.display()));
                }
            }
        });
        Ok(Run { snapshot: rx, commands: cmd_tx, shared, done })
    }
}

/// A recipe running.
pub struct Run {
    pub snapshot: watch::Receiver<Snapshot>,
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
    done: JoinHandle<()>,
}

impl Run {
    pub fn commands(&self) -> mpsc::Sender<Command> {
        self.commands.clone()
    }

    pub async fn send(&self, c: Command) {
        let _ = self.commands.send(c).await;
    }

    /// Everything the steps sent on, merged, each fact and candidate marked
    /// with its step: what the window shows. Steps never see it; each
    /// knows only what its items carry.
    pub fn context(&self) -> Context {
        self.shared.context.lock().unwrap().clone()
    }

    /// This run's working folder; it exists once a step has used it.
    pub fn work(&self) -> &std::path::Path {
        &self.shared.work
    }

    /// The log, for when you want it.
    pub fn log(&self) -> Vec<String> {
        self.shared.log.lock().unwrap().clone()
    }

    /// A step's tool output, the last RAW_KEPT lines.
    pub fn raw(&self, step: &str) -> Vec<String> {
        self.shared.raw.lock().unwrap().get(step).map(|v| v.iter().cloned().collect()).unwrap_or_default()
    }

    /// Every step's kept tool output, by step.
    pub fn raw_all(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        self.shared.raw.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.iter().cloned().collect())).collect()
    }

    /// Lets go of the drive so its tray can open: every read on it stops
    /// (its tool is killed and waited for), and no step starts another
    /// until it's asked to again. Done once nothing holds the drive, or
    /// after `wait`; gives whether it was let go of in time.
    pub fn release_drive(&self, wait: std::time::Duration) -> impl std::future::Future<Output = bool> + Send + 'static {
        let shared = self.shared.clone();
        shared.tray.send_modify(|g| *g += 1);
        let mut holding = shared.holding.subscribe();
        async move { tokio::time::timeout(wait, holding.wait_for(|n| *n == 0)).await.is_ok_and(|r| r.is_ok()) }
    }

    /// Waits for every step to finish, and gives the last snapshot.
    pub async fn finished(self) -> Snapshot {
        let _ = self.done.await;
        self.shared.snap.lock().unwrap().clone()
    }
}

pub(crate) struct Shared {
    pub snap: Mutex<Snapshot>,
    pub tx: watch::Sender<Snapshot>,
    /// What the steps sent on, for the window. Written only as items leave
    /// a step; no step reads it.
    pub context: Mutex<Context>,
    pub settings: Arc<Mutex<Settings>>,
    pub lanes: Arc<Lanes>,
    pub cancel: CancellationToken,
    pub answers: Mutex<HashMap<String, oneshot::Sender<Answer>>>,
    /// Held while a question is up: there's one place to show it, so a
    /// second step asking waits its turn instead of hiding the first.
    pub asking: tokio::sync::Mutex<()>,
    /// Set once any step has waited on you: a question, or a *Retry*.
    pub needed_you: std::sync::atomic::AtomicBool,
    pub retries: Mutex<HashMap<String, mpsc::Sender<RetryRequest>>>,
    pub approvals: Mutex<HashMap<String, Arc<Notify>>>,
    pub drive: String,
    pub time_scale: f64,
    pub log: Mutex<Vec<String>>,
    /// Each step's tool output, the last RAW_KEPT lines.
    pub raw: Mutex<HashMap<String, std::collections::VecDeque<String>>>,
    /// All of it, as it comes, in the working folder.
    pub raw_file: Mutex<Option<std::io::BufWriter<std::fs::File>>>,
    /// How many times the drive has been let go of for its tray to open.
    pub tray: watch::Sender<u64>,
    /// How many reads hold the drive just now (see `Cx::drive_read`).
    pub holding: watch::Sender<usize>,
    /// This run's working folder, made when first asked for.
    pub work: std::path::PathBuf,
}

/// How many lines of a tool's output are kept in memory, per step.
pub const RAW_KEPT: usize = 2000;

impl Shared {
    pub(crate) fn raw_line(&self, step: &str, line: &str) {
        {
            let mut raw = self.raw.lock().unwrap();
            let q = raw.entry(step.to_owned()).or_default();
            q.push_back(line.to_owned());
            while q.len() > RAW_KEPT {
                q.pop_front();
            }
        }
        use std::io::Write;
        let mut f = self.raw_file.lock().unwrap();
        if f.is_none() && std::fs::create_dir_all(&self.work).is_ok() {
            *f = std::fs::OpenOptions::new().create(true).append(true).open(self.work.join("tool-output.txt")).ok().map(std::io::BufWriter::new);
        }
        if let Some(w) = f.as_mut() {
            let _ = writeln!(w, "{step}: {line}");
        }
    }

    pub fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        let mut s = self.snap.lock().unwrap();
        f(&mut s);
        s.derive_waits();
        self.tx.send_replace(s.clone());
    }

    fn command(&self, c: Command) {
        match c {
            Command::Answer { step, answer } => {
                if let Some(tx) = self.answers.lock().unwrap().remove(&step) {
                    let _ = tx.send(answer);
                }
            }
            Command::Retry { step, item, option } => {
                if let Some(tx) = self.retries.lock().unwrap().get(&step) {
                    let _ = tx.try_send(RetryRequest { item, option });
                }
            }
            Command::Approve { step } => {
                if let Some(n) = self.approvals.lock().unwrap().get(&step) {
                    n.notify_one();
                }
            }
            Command::Stop => {
                self.cancel.cancel();
                self.update(|s| s.stopped = true);
            }
        }
    }
}

fn finish(cx: &Cx, result: Result<()>) {
    let stopped = cx.stopped();
    cx.view(|v| {
        if v.record.started.is_some() {
            v.record.finished = Some(crate::setting::now());
        }
    });
    cx.view(|v| match result {
        // Stopped: a step that hadn't started isn't done, it just never ran;
        // one that was working stopped. Neither turns green.
        Ok(()) if stopped => {
            match v.state {
                StepState::Busy => {
                    v.state = StepState::Idle;
                    v.status = "Stopped".into();
                    v.qualifier.clear();
                }
                StepState::Idle | StepState::Wait => {
                    v.state = StepState::Idle;
                    v.status = "Didn't run".into();
                    v.qualifier.clear();
                    v.detail.clear();
                }
                _ => {}
            }
            v.progress = None;
        }
        Ok(()) => {
            if matches!(v.state, StepState::Busy | StepState::Idle | StepState::Wait) {
                v.state = StepState::Ok;
                if v.status.is_empty() {
                    v.status = "Done".into();
                }
            }
            v.progress = None;
        }
        Err(_) if stopped => {
            if v.state == StepState::Busy {
                v.status = "Stopped".into();
                v.qualifier.clear();
                v.state = StepState::Idle;
            }
            v.progress = None;
        }
        Err(_) if cx.tray_cut() => {
            v.state = StepState::Bad;
            v.status = "Read stopped".into();
            v.qualifier.clear();
            v.detail = crate::snapshot::TRAY_DETAIL.into();
            v.progress = None;
            v.record.log.push(crate::snapshot::LogLine { at: crate::setting::now(), line: "read stopped: tray opened".into() });
        }
        Err(e) => {
            v.state = StepState::Bad;
            // What it had got to stays the outcome; why it stopped is the detail.
            if v.status.is_empty() {
                v.status = "Failed".into();
            }
            v.detail = e.to_string();
            v.progress = None;
            // Why, in its log too, where the sidebar opens for a failure.
            v.record.log.push(crate::snapshot::LogLine { at: crate::setting::now(), line: format!("failed: {e:#}") });
        }
    });
}

/// One step in the compiled graph.
struct GNode {
    id: String,
    step: String,
    label: Option<String>,
    params: BTreeMap<String, Setting<Value>>,
    hold: bool,
    preds: Vec<usize>,
    /// Starts the path for when this step fails or needs you.
    side: Option<(String, Side)>,
    /// On such a path.
    on_side: bool,
}

/// Adds `node` to the graph after `preds`; gives the steps it ends with.
fn compile(node: &Node, preds: &[usize], g: &mut Vec<GNode>) -> Vec<usize> {
    match node {
        Node::Step { id, step, label, params, .. } => {
            // A step marked to hold in an older recipe is upgraded before
            // it's compiled; here hold means it is a Hold step.
            g.push(GNode { id: id.clone(), step: step.clone(), label: label.clone(), params: params.clone(), hold: step == "hold", preds: preds.to_vec(), side: None, on_side: false });
            let me = g.len() - 1;
            for s in Side::BOTH {
                let Some(path) = node.side(s) else { continue };
                let first = g.len();
                compile(path, &[], g);
                for n in &mut g[first..] {
                    if n.preds.is_empty() && n.side.is_none() {
                        n.side = Some((id.clone(), s));
                    }
                    n.on_side = true;
                }
            }
            vec![me]
        }
        Node::Chain { nodes } => nodes.iter().fold(preds.to_vec(), |p, n| compile(n, &p, g)),
        Node::Branches { branches } => branches.iter().flat_map(|b| compile(b, preds, g)).collect(),
        Node::Group { body, .. } => compile(body, preds, g),
    }
}

/// Watches the run for a step with a path of its own beginning to fail
/// or need you, and sends one message down that path each time it does:
/// in the step's own words, knowing what the run knows of the disc. Ends
/// once told the rest of the run is done, after a last look, closing the
/// paths.
async fn watch_sides(s: Arc<Shared>, to: Vec<((String, Side), mpsc::Sender<Item>)>, mut end: oneshot::Receiver<()>) {
    let mut rx = s.tx.subscribe();
    let mut before = rx.borrow_and_update().clone();
    let mut n = 0;
    loop {
        let last = tokio::select! {
            r = rx.changed() => r.is_err(),
            _ = &mut end => true,
        };
        let now = rx.borrow_and_update().clone();
        let begun: Vec<_> = to.iter().filter(|((id, side), _)| !now.stopped && now.steps.get(id).is_some_and(|v| begins(*side, v, before.steps.get(id)))).collect();
        // A step says why just after it turns: let that land first.
        let latest = if begun.is_empty() || last {
            now.clone()
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            s.tx.borrow().clone()
        };
        for ((id, side), tx) in begun {
            // As it is now, if it still waits so; else as it turned.
            let Some(v) = latest.steps.get(id).filter(|l| begins(*side, l, None)).or_else(|| now.steps.get(id)) else { continue };
            n += 1;
            let context = s.context.lock().unwrap().clone();
            let _ = tx.send(Item::message(n, &v.name, &message_line(*side, v), context)).await;
        }
        before = now;
        if last {
            break;
        }
    }
}

/// Whether a step has just begun to fail (`Fails`) or to need you, from
/// how it was. Holding a track or an image for you to retry or settle is
/// needing you, as is a question, or a hold once all is in. A tray opened
/// by hand isn't news: whoever opened it is there.
fn begins(side: Side, v: &StepView, was: Option<&StepView>) -> bool {
    let holding = |v: &StepView| v.state == StepState::Bad && v.record.finished.is_none() && v.items.iter().any(|i| i.state == crate::snapshot::ItemState::Bad);
    let waits = |v: &StepView| match side {
        Side::Fails => v.state == StepState::Bad && !holding(v),
        Side::NeedsYou => v.state == StepState::Needs || (v.state == StepState::Hold && !v.detail.is_empty()) || holding(v),
    };
    let bad: Vec<_> = v.items.iter().filter(|i| i.state == crate::snapshot::ItemState::Bad).collect();
    let tray = v.detail == crate::snapshot::TRAY_DETAIL || (!bad.is_empty() && bad.iter().all(|i| i.note == crate::snapshot::TRAY_OPENED));
    waits(v) && !was.is_some_and(waits) && !tray
}

/// What a message says: the step by name, then in its own words why it
/// failed (*CHD: chdman isn't there*) or what it waits on you for (*Choose
/// needs you: Which game is this?*).
pub fn message_line(side: Side, v: &StepView) -> String {
    // A question is its status; why it failed or holds, its detail.
    let said = if v.detail.is_empty() || (v.state == StepState::Needs && !v.status.is_empty()) { &v.status } else { &v.detail };
    let name = &v.name;
    match side {
        Side::Fails if said.is_empty() => format!("{name} failed"),
        Side::Fails => format!("{name}: {said}"),
        Side::NeedsYou if said.is_empty() => format!("{name} needs you"),
        Side::NeedsYou => format!("{name} needs you: {said}"),
    }
}

/// Merges a step's wires in. With more than one, an item goes through once
/// it has arrived along all of them, knowing what every branch's copy of it
/// learned.
fn join(mut wires: Vec<mpsc::Receiver<Item>>, tasks: &mut Vec<JoinHandle<()>>) -> mpsc::Receiver<Item> {
    if wires.len() == 1 {
        return wires.pop().unwrap();
    }
    let n = wires.len();
    let (tx, rx) = mpsc::channel(WIRE);
    tasks.push(tokio::spawn(async move {
        let mut merged = stream::select_all(wires.into_iter().enumerate().map(|(w, r)| Box::pin(stream::unfold(r, move |mut r| async move { r.recv().await.map(|i| ((w, i), r)) }))));
        // Each item's copies so far, by the wire they came on. Items that
        // some branch didn't pass on (left out, or gathered into something
        // new, like a cue sheet) go through when the wires close.
        let mut partial: Vec<(String, Vec<(usize, Item)>)> = Vec::new();
        while let Some((w, item)) = merged.next().await {
            let at = match partial.iter().position(|(id, _)| *id == item.id) {
                Some(at) => at,
                None => {
                    partial.push((item.id.clone(), Vec::new()));
                    partial.len() - 1
                }
            };
            partial[at].1.push((w, item));
            if partial[at].1.len() == n {
                let (_, copies) = partial.remove(at);
                if tx.send(joined(copies, true)).await.is_err() {
                    return;
                }
            }
        }
        for (_, copies) in partial {
            if tx.send(joined(copies, false)).await.is_err() {
                return;
            }
        }
    }));
    rx
}

/// One item from its copies: the last to arrive when every wire brought
/// it, the first when only some did (the kind and file that go on),
/// knowing what each copy knew, merged in the order of the wires so the
/// same recipe always gives the same order.
fn joined(mut copies: Vec<(usize, Item)>, all: bool) -> Item {
    let pick = if all { copies.last() } else { copies.first() };
    let mut item = pick.map(|(_, i)| i.clone()).expect("a copy");
    copies.sort_by_key(|(w, _)| *w);
    if copies.iter().any(|(_, c)| !std::sync::Arc::ptr_eq(&c.context, &copies[0].1.context)) {
        let mut cx = Context::default();
        for (_, c) in &copies {
            cx.merge(&c.context);
        }
        item.context = std::sync::Arc::new(cx);
    }
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Subject;

    #[tokio::test]
    async fn a_join_lets_an_item_through_knowing_what_each_branch_learned() {
        let mut disc = Item::disc("d");
        disc.context_mut().add(Subject::Disc, "parts", "17", "the disc");
        let (ta, ra) = mpsc::channel(4);
        let (tb, rb) = mpsc::channel(4);
        let mut tasks = Vec::new();
        let mut out = join(vec![ra, rb], &mut tasks);
        let (mut a, mut b) = (disc.clone(), disc.clone());
        b.context_mut().add(Subject::Disc, "cover", "front.jpg", "Cover Art Archive");
        a.context_mut().add(Subject::Disc, "lyrics.lrc", "yes", "Lyrics");
        // The second wire's copy arrives first; the order is still the wires'.
        tb.send(b).await.unwrap();
        ta.send(a).await.unwrap();
        drop((ta, tb));
        let got = out.recv().await.unwrap();
        let keys: Vec<&str> = got.context().facts().iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["parts", "lyrics.lrc", "cover"], "each once, in the wires' order");
        assert!(out.recv().await.is_none());
    }
}
