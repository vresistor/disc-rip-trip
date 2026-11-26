//! MakeMKV, which video discs are read with. It doesn't come with the app
//! (it decrypts copy protection and is proprietary: THIRD-PARTY.md), so
//! it's run only from where you said it may
//! be: a setting of the app's, like its appearance. Until you've said,
//! the usual places are looked at (do the files exist?) but nothing is
//! run. Approving runs it once, for its version.

use crate::model::Model;
use std::path::PathBuf;
use std::sync::Arc;
use trip_steps::real::video::makemkv::{self as mk, Location};

/// The app's id, which flatpak knows it by.
pub const APP_ID: &str = "io.github.vresistor.trip";

/// Where to get it.
pub const GET_URL: &str = "https://www.makemkv.com/download/";

/// What the sidebar says first, when it isn't set up.
pub const WHY: &str = "Video discs are read by MakeMKV, which doesn't come with this app: it isn't free software, and it gets round copy protection. Trip only runs it if you say so.";

/// What the sidebar says when the flatpak's permission to start programs
/// outside it has been taken away. It comes with it; choosing MakeMKV is
/// all it should take.
pub const TAKEN_AWAY: &str = "Trip has been stopped from starting programs outside its flatpak, so it can't start MakeMKV. This is usually turned off in Flatseal (\"Talk to org.freedesktop.Flatpak\"). Turn it back on there, then close Trip and open it again.";

pub type Candidates = Arc<dyn Fn() -> Vec<Location> + Send + Sync>;
pub type Version = Arc<dyn Fn(Location) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>> + Send + Sync>;
pub type PickFile = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;
pub type HostAllowed = Arc<dyn Fn() -> bool + Send + Sync>;

/// How the app finds and runs MakeMKV: the real ones, or a test's.
#[derive(Clone)]
pub struct Hooks {
    /// Whether a video recipe needs it chosen before it starts. Off for
    /// the simulation, whose video steps don't run it.
    pub required: bool,
    /// Whether the app is inside a flatpak, so MakeMKV is on the host.
    pub sandboxed: bool,
    /// Whether the flatpak may start programs on the host, as it was
    /// started (true outside a flatpak).
    pub host_allowed: HostAllowed,
    /// `TRIP_MAKEMKVCON` is set: you said where, outside the app.
    pub from_env: bool,
    /// The MakeMKVs that seem to be installed. Checks files only.
    pub candidates: Candidates,
    /// Runs it once and gives its version: only for one you approved.
    pub version: Version,
    /// A file picker, for *Choose…*.
    pub pick_file: PickFile,
}

impl Hooks {
    /// This computer's.
    pub fn real() -> Hooks {
        let sandboxed = mk::sandboxed();
        Hooks {
            required: true,
            sandboxed,
            host_allowed: Arc::new(mk::host_allowed_here),
            from_env: mk::from_env().is_some(),
            candidates: Arc::new(|| mk::candidates(&mk::Places::here())),
            version: Arc::new(move |loc| {
                Box::pin(async move {
                    let dir = std::env::temp_dir().join(format!("trip-makemkv-{}", std::process::id()));
                    let r = mk::version(&mk::Launcher::of(&loc, mk::sandboxed()), &dir).await;
                    let _ = std::fs::remove_dir_all(&dir);
                    r
                })
            }),
            pick_file: Arc::new(|| rfd::FileDialog::new().set_title("Choose makemkvcon").pick_file()),
        }
    }

    /// Not needed, and never run: the simulation, and tests that aren't
    /// about it.
    pub fn not_needed() -> Hooks {
        Hooks {
            required: false,
            sandboxed: false,
            host_allowed: Arc::new(|| true),
            from_env: false,
            candidates: Arc::new(Vec::new),
            version: Arc::new(|_| Box::pin(async { anyhow::bail!("not run here") })),
            pick_file: Arc::new(|| None),
        }
    }
}

/// What's saved: where, and the version it said when approved.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Saved {
    pub location: Location,
    #[serde(default)]
    pub version: Option<String>,
}

/// Whether a video recipe can start, as far as MakeMKV goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    /// Nobody has said where it is.
    NotChosen,
    /// Approved, but on the host, and the flatpak may not start it.
    NeedsPermission,
    /// Approved just now; its version is being asked.
    Checking,
}

impl Readiness {
    /// The card's line and detail.
    pub fn card(self) -> (String, String) {
        match self {
            Readiness::Ready => (String::new(), String::new()),
            Readiness::NotChosen => ("MakeMKV not set up".into(), "Choose where MakeMKV is".into()),
            Readiness::NeedsPermission => ("MakeMKV can't be started".into(), "Starting programs outside the flatpak was turned off".into()),
            Readiness::Checking => ("Checking MakeMKV".into(), String::new()),
        }
    }

    /// Why Start can't, as a sentence.
    pub fn sentence(self) -> String {
        match self {
            Readiness::Ready => String::new(),
            Readiness::NotChosen => "Video discs need MakeMKV: choose where it is first".into(),
            Readiness::NeedsPermission => "MakeMKV is outside the flatpak, and starting programs outside it was turned off".into(),
            Readiness::Checking => "Still checking MakeMKV".into(),
        }
    }
}

/// The app's MakeMKV setting and what's known about it.
pub struct MakeMkv {
    pub hooks: Hooks,
    pub saved: Option<Saved>,
    /// What was found in the usual places when last looked.
    pub candidates: Vec<Location>,
    /// The one being run for its version just now.
    pub checking: Option<Location>,
    /// Why the last one chosen couldn't be used.
    pub error: Option<String>,
    /// Whether the free beta key is kept up to date. Off until turned on.
    pub key_updates: bool,
    /// Whether the flatpak may start host programs, as last read.
    pub host_ok: bool,
    /// The choices are open again, to change it.
    pub changing: bool,
    tx: std::sync::mpsc::Sender<(Location, Result<String, String>)>,
    rx: std::sync::mpsc::Receiver<(Location, Result<String, String>)>,
}

impl MakeMkv {
    pub fn new(hooks: Hooks, saved: Option<Saved>, key_updates: bool) -> MakeMkv {
        let (tx, rx) = std::sync::mpsc::channel();
        let host_ok = (hooks.host_allowed)();
        let candidates = if hooks.required && saved.is_none() && !hooks.from_env { (hooks.candidates)() } else { Vec::new() };
        MakeMkv { hooks, saved, candidates, checking: None, error: None, key_updates, host_ok, changing: false, tx, rx }
    }

    pub fn readiness(&self) -> Readiness {
        if self.hooks.from_env {
            return Readiness::Ready;
        }
        if self.checking.is_some() {
            return Readiness::Checking;
        }
        match &self.saved {
            None => Readiness::NotChosen,
            Some(s) if s.location.needs_host(self.hooks.sandboxed) && !self.host_ok => Readiness::NeedsPermission,
            Some(_) => Readiness::Ready,
        }
    }

    /// Whether the approved one is waiting on the flatpak's permission.
    pub fn wants_permission(&self) -> bool {
        self.readiness() == Readiness::NeedsPermission
    }

    /// The line the settings area shows.
    pub fn summary(&self) -> String {
        if self.hooks.from_env {
            return "Set by TRIP_MAKEMKVCON".into();
        }
        match (self.readiness(), &self.saved) {
            (Readiness::Checking, _) => "Checking…".into(),
            (Readiness::NeedsPermission, _) => "Can't be started".into(),
            // The version, without the name the row already says.
            (_, Some(s)) => s.version.as_deref().map_or("Chosen".into(), |v| v.trim_start_matches("MakeMKV ").to_owned()),
            (_, None) => "Not set up".into(),
        }
    }
}

/// Step kinds that run MakeMKV.
pub fn runs_makemkv(kind: &str) -> bool {
    matches!(kind, "video-disc" | "rip-video")
}

impl Model {
    /// Whether the recipe has a step that runs MakeMKV, and so needs it
    /// chosen.
    pub fn needs_makemkv(&self, i: usize) -> bool {
        self.makemkv.hooks.required && self.recipes.get(i).is_some_and(|r| r.root.steps().into_iter().any(|n| matches!(n, trip_engine::Node::Step { step, .. } if runs_makemkv(step))))
    }

    /// What keeps the recipe from starting, as far as MakeMKV goes.
    pub fn makemkv_blocks(&self, i: usize) -> Option<Readiness> {
        let r = self.makemkv.readiness();
        (self.needs_makemkv(i) && r != Readiness::Ready).then_some(r)
    }

    /// Looks in the usual places again (files only).
    pub fn look_for_makemkv(&mut self) {
        self.makemkv.candidates = (self.makemkv.hooks.candidates)();
        self.makemkv.host_ok = (self.makemkv.hooks.host_allowed)();
    }

    /// You said MakeMKV may be run from here. Where it can be run now, it's
    /// run once for its version, off the window's thread; it's saved once
    /// it answers. Where it's on the host and the flatpak may not start it,
    /// it's saved as it is, to run once you've let it.
    pub fn approve_makemkv(&mut self, loc: Location) {
        self.makemkv.error = None;
        self.makemkv.host_ok = (self.makemkv.hooks.host_allowed)();
        if loc.needs_host(self.makemkv.hooks.sandboxed) && !self.makemkv.host_ok {
            self.makemkv.changing = false;
            self.save_makemkv(Some(Saved { location: loc, version: None }));
            return;
        }
        self.check_makemkv(loc);
    }

    /// Runs it for its version (it's approved).
    fn check_makemkv(&mut self, loc: Location) {
        self.makemkv.checking = Some(loc.clone());
        let (version, tx, ctx) = (self.makemkv.hooks.version.clone(), self.makemkv.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let r = version(loc.clone()).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send((loc, r));
            if let Some(c) = ctx {
                c.request_repaint();
            }
        });
    }

    /// One saved from before whose version isn't known (it was waiting on
    /// the permission): asked now that it can be.
    pub fn check_saved_makemkv(&mut self) {
        let Some(s) = self.makemkv.saved.clone() else { return };
        if s.version.is_none() && self.makemkv.readiness() == Readiness::Ready && !self.makemkv.hooks.from_env && self.makemkv.hooks.required {
            self.check_makemkv(s.location);
        }
    }

    /// Takes in a version check's answer. True if something changed.
    pub fn poll_makemkv(&mut self) -> bool {
        let mut changed = false;
        while let Ok((loc, r)) = self.makemkv.rx.try_recv() {
            if self.makemkv.checking.as_ref() != Some(&loc) {
                continue;
            }
            self.makemkv.checking = None;
            changed = true;
            match r {
                Ok(v) => {
                    self.makemkv.changing = false;
                    self.save_makemkv(Some(Saved { location: loc, version: Some(v) }));
                }
                Err(e) => self.makemkv.error = Some(format!("Couldn't use {}: {e}", loc.label())),
            }
        }
        changed
    }

    /// MakeMKV may not be run any more.
    pub fn revoke_makemkv(&mut self) {
        self.makemkv.changing = false;
        self.makemkv.error = None;
        self.save_makemkv(None);
        self.look_for_makemkv();
    }

    fn save_makemkv(&mut self, s: Option<Saved>) {
        if let Err(e) = self.store.save_makemkv(s.as_ref()) {
            eprintln!("couldn't save the MakeMKV setting: {e:#}");
        }
        mk::approve(s.as_ref().map(|s| s.location.clone()));
        self.makemkv.saved = s;
    }

    /// Whether the free beta key is fetched and saved in MakeMKV's settings
    /// when it runs out. Remembered.
    pub fn set_makemkv_key_updates(&mut self, on: bool) {
        self.makemkv.key_updates = on;
        mk::set_key_updates(on);
        if let Err(e) = self.store.save_makemkv_key_updates(on) {
            eprintln!("couldn't save the MakeMKV key setting: {e:#}");
        }
    }

    /// A file chosen with *Choose…*, off the window's thread.
    pub fn choose_makemkv_file(&mut self, path: PathBuf) {
        match mk::chosen_file(&path) {
            Ok(loc) => self.approve_makemkv(loc),
            Err(e) => self.makemkv.error = Some(cap(&e.to_string())),
        }
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
