//! The pictures in the docs, rendered headless from the real window on
//! simulated steps. The tests here are ignored; run them by hand:
//!
//! ```sh
//! cargo test --release -p drt-app --test demo -- --ignored --test-threads 1
//! ```
//!
//! `docs_screenshots` writes `target/docs-shots/<name>-<light|dark>.png`,
//! from which `docs/screenshots/` is made (`data`, `games`, `video`), with
//! four of the window tests' shots from `app/shots/`:
//! `sidebar-rip-running-1920` (`rip-running`, also `docs/screenshot.png`),
//! `sidebar-choose-asking` (`choose`), `sidebar-rip-retry` (`rip-sidebar`)
//! and `picker-popover` (`edit-picker`). `social-light` is
//! `docs/social-preview.png`.
//!
//! `demo_video` and `demo_video_retry` write `target/demo-cd/` and
//! `target/demo-retry/`: `frame-00000.png`…, 30 frames a second of a
//! simulated CD in the CDs recipe, with motion and Follow on and a caption
//! at each step that matters. The first goes from Start to the next disc
//! and the files let through; the second is a track that doesn't match.
//! Then, for each (`cd` → `demo`, `retry` → `demo-retry`):
//!
//! ```sh
//! ffmpeg -framerate 30 -i target/demo-cd/frame-%05d.png -c:v libvpx-vp9 \
//!   -crf 32 -b:v 0 -row-mt 1 -pix_fmt yuv420p docs/demo.webm
//! ffmpeg -framerate 30 -i target/demo-cd/frame-%05d.png \
//!   -vf fps=12,scale=1120:-1:flags=lanczos -c:v libwebp_anim -q:v 70 \
//!   -compression_level 6 -loop 0 docs/demo.webp
//! rm -r target/demo-cd
//! ```

use drt_app::{App, Deps, Model, Store, Tray, Window};
use drt_engine::{Engine, ItemState, Setting};
use drt_steps::{recipes, simulated};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Env {
    rt: tokio::runtime::Runtime,
    dir: tempfile::TempDir,
}

fn env() -> Env {
    Env { rt: tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap(), dir: tempfile::tempdir().unwrap() }
}

/// The window on simulated steps, both drives measured, every recipe on
/// the first; `fps` sets how far egui's clock moves each step.
fn window<'a>(e: &Env, reg: drt_engine::Registry, time_scale: f64, size: egui::Vec2, ppp: f32, fps: f32, motion: bool) -> Harness<'a, Window> {
    let app = make_app(e, reg, time_scale, motion);
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(ppp)
        .with_step_dt(1.0 / fps)
        .with_render_cursor(false)
        .with_context_setup(|ctx| ctx.set_fonts(drt_app::theme::fonts()))
        .wgpu_setup(renderer())
        .build_eframe(|_| Window(app));
    h.run_steps(3);
    h
}

fn make_app(e: &Env, reg: drt_engine::Registry, time_scale: f64, motion: bool) -> App {
    let mut engine = Engine::new(reg);
    engine.time_scale = time_scale;
    engine.work_root = e.dir.path().join("work");
    {
        let mut s = engine.settings.lock().unwrap();
        s.set_drive(recipes::BLU_RAY, "offset", Setting::detected(Value::from(103), "AccurateRip"));
        s.set_drive(recipes::DVD, "offset", Setting::detected(Value::from(6), "AccurateRip"));
        s.set_app("tmdb", "api_key", Setting::chosen(Value::from("demo-key")));
    }
    let drives = vec![recipes::BLU_RAY.to_owned(), recipes::DVD.to_owned()];
    let deps = Deps {
        // Every folder can take a disc; a test that looks gives its own.
        reach: Arc::new(|_, _| Ok(())),
        engine,
        store: Store::new(e.dir.path().join("config")),
        drives: drives.clone(),
        rt: e.rt.handle().clone(),
        open_folder: Arc::new(|_| Ok(())),
        pick_folder: Arc::new(|_| None),
        send_test: Arc::new(|_, _, _| Box::pin(async { Ok(vec![]) })),
        open_tray: Arc::new(|_| Ok(())),
        connected: Arc::new(move || drives.clone()),
    };
    let mut model = Model::new(deps);
    model.repeat_limit = Some(0);
    let mut app = App::new(model, false);
    app.motion = motion;
    app
}

/// As in `ui.rs`: no debug object names, which the Vulkan loader can't
/// take from several threads at once.
fn renderer() -> eframe::egui_wgpu::WgpuSetup {
    let mut setup = egui_kittest::wgpu::default_wgpu_setup();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(new) = &mut setup {
        new.instance_descriptor.flags = eframe::wgpu::InstanceFlags::empty();
    }
    setup
}

fn app<'h>(h: &'h Harness<Window>) -> &'h App {
    &h.state().0
}

fn app_mut<'h>(h: &'h mut Harness<'_, Window>) -> &'h mut App {
    &mut h.state_mut().0
}

fn target(dir: &str) -> PathBuf {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target").join(dir);
    std::fs::create_dir_all(&out).unwrap();
    out
}

fn until(h: &mut Harness<Window>, what: &str, cond: impl Fn(&App) -> bool) {
    let start = Instant::now();
    while !cond(app(h)) {
        assert!(start.elapsed() < Duration::from_secs(60), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run_steps(3);
}

/// Picks the recipe by its name in the list, and fits it to the window.
fn pick(h: &mut Harness<Window>, name: &str) {
    let i = app(h).m.recipes.iter().position(|r| r.name == name).unwrap();
    app_mut(h).select(i);
    h.run_steps(3);
    h.get_by_label("Fit to window").click();
    h.run_steps(3);
}

/// Answers with what it suggests, and lets a hold through, until the
/// selected recipe's disc is done.
fn to_the_end(h: &mut Harness<Window>) {
    let start = Instant::now();
    loop {
        let snap = app(h).m.snapshot(app(h).sel);
        if snap.finished {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "timed out");
        if let Some((step, _)) = &snap.question {
            app_mut(h).open(Tray::Question { step: step.clone() });
            h.run_steps(3);
            if let Some(b) = h.query_by_label("Use this one").or_else(|| h.query_by_label("Use these")) {
                b.click_accesskit();
            }
        } else if let Some((id, _)) = snap.steps.iter().find(|(_, v)| v.step == "hold" && v.detail == "All here") {
            app_mut(h).open(Tray::Step { id: id.clone(), focus: None });
            h.run_steps(3);
            if let Some(b) = h.query_by_label_contains("through") {
                b.click_accesskit();
            }
        }
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    app_mut(h).close_tray();
    h.run_steps(3);
}

/// Renders the window still, in light and dark.
fn shot(h: &mut Harness<Window>, name: &str) {
    let dir = target("docs-shots");
    app_mut(h).clock = Some(0.3);
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        app_mut(h).dark = dark;
        h.run_steps(3);
        h.render().unwrap().save(dir.join(format!("{name}-{theme}.png"))).unwrap();
    }
    app_mut(h).dark = false;
    app_mut(h).clock = None;
    h.run_steps(2);
}

#[test]
#[ignore = "renders the docs' screenshots; run by hand"]
fn docs_screenshots() {
    // The CD recipe mid-rip, the whole of it in view; also the social
    // preview's 1280×640.
    for (name, size, ppp) in [("cd-running", egui::vec2(1280.0, 720.0), 1.5), ("social", egui::vec2(1280.0, 640.0), 1.0)] {
        let e = env();
        let mut h = window(&e, simulated(None), 0.15, size, ppp, 4.0, false);
        pick(&mut h, "CDs");
        h.get_by_label("Start").click();
        h.run_steps(2);
        until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
        h.get_by_label("Use this one").click();
        h.run_steps(2);
        until(&mut h, "half the disc", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Ok) >= 9);
        shot(&mut h, name);
    }
    // The other kinds of disc, each run to the end.
    let e = env();
    let mut reg = simulated(None);
    drt_steps::real::video::sim::register(&mut reg, drt_steps::real::video::sim::SimDisc::Film);
    let mut h = window(&e, reg, 0.0, egui::vec2(1280.0, 720.0), 1.0, 4.0, false);
    for (recipe, name) in [("Data discs", "data-done"), ("Games", "games-done"), ("Films and TV", "video-done")] {
        pick(&mut h, recipe);
        h.get_by_label("Start").click();
        h.run_steps(2);
        to_the_end(&mut h);
        h.get_by_label("Fit to window").click();
        h.run_steps(3);
        shot(&mut h, name);
    }
}

/// The window with a caption over its canvas, for the demo videos.
struct Demo {
    w: Window,
    caption: Option<String>,
    /// The last caption, kept while it fades out.
    last: String,
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.w, ui, frame);
        if let Some(c) = &self.caption {
            self.last = c.clone();
        }
        let ctx = ui.ctx().clone();
        let k = ctx.animate_bool_with_time(ui.scope_id().with("caption"), self.caption.is_some(), 0.3);
        if k <= 0.0 {
            return;
        }
        // Over the canvas, above its zoom bar; dark in light and dark.
        let canvas = self.w.0.canvas_rect();
        let a = |x: f32| (x * k) as u8;
        egui::Area::new(ui.scope_id().with("caption-area"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(egui::pos2(canvas.center().x, canvas.bottom() - 56.0 + 8.0 * (1.0 - k)))
            .show(&ctx, |ui| {
                egui::Frame::new().fill(egui::Color32::from_rgba_unmultiplied(22, 26, 34, a(235.0))).corner_radius(10).inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
                    ui.set_max_width((canvas.width() - 48.0).min(560.0));
                    ui.add(egui::Label::new(egui::RichText::new(&self.last).font(drt_app::theme::font(16.0, 500)).color(egui::Color32::from_white_alpha(a(255.0)))).wrap());
                });
            });
    }

    fn clear_color(&self, v: &egui::Visuals) -> [f32; 4] {
        self.w.clear_color(v)
    }
}

/// The demo's window: 1120×700 at twice the pixels, so its words read
/// when the README shrinks it; Follow on, the canvas a little zoomed out.
fn demo_window<'a>(e: &Env, reg: drt_engine::Registry, time_scale: f64) -> Harness<'a, Demo> {
    let w = Window(make_app(e, reg, time_scale, true));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1120.0, 700.0))
        .with_pixels_per_point(2.0)
        .with_step_dt(1.0 / FPS as f32)
        .with_render_cursor(false)
        .with_context_setup(|ctx| ctx.set_fonts(drt_app::theme::fonts()))
        .wgpu_setup(renderer())
        .build_eframe(|_| Demo { w, caption: None, last: String::new() });
    h.run_steps(3);
    let a = &mut h.state_mut().w.0;
    a.follow = true;
    a.m.repeat_limit = Some(1);
    // The CDs recipe, the whole of it.
    let i = a.m.recipes.iter().position(|r| r.name == "CDs").unwrap();
    a.select(i);
    h.run_steps(3);
    h.get_by_label("Fit to window").click_accesskit();
    h.run_steps(30);
    h
}

/// Its start, close enough to read: 80%, the drive at the left.
fn close_up(a: &App) -> drt_app::canvas::View {
    let lay = a.layout(a.sel);
    let z = 0.8;
    drt_app::canvas::View { zoom: z, pan: egui::vec2(24.0, ((700.0 - 52.0 - 40.0 - lay.height * z) / 2.0).max(16.0)) }
}

const FPS: f64 = 30.0;

/// Records frames: the engine and egui run in step with the video, a
/// frame saved for each `speed` steps (2 is twice as fast).
struct Rec {
    dir: PathBuf,
    t0: Instant,
    /// Seconds of engine and egui time so far.
    t: f64,
    acc: f64,
    saved: usize,
    late: f64,
    slow: f64,
}

impl Rec {
    fn new(name: &str, slow: f64) -> Rec {
        let dir = target(&format!("demo-{name}"));
        for f in std::fs::read_dir(&dir).unwrap().flatten() {
            let _ = std::fs::remove_file(f.path());
        }
        Rec { dir, t0: Instant::now(), t: 0.0, acc: 0.0, saved: 0, late: 0.0, slow }
    }

    fn tick(&mut self, h: &mut Harness<Demo>, speed: f64, caption: Option<&str>) {
        self.tick_while(h, speed, caption, |_| true);
    }

    /// As `tick`, but the frame is kept only while `keep` holds after
    /// the step; says whether it did.
    fn tick_while(&mut self, h: &mut Harness<Demo>, speed: f64, caption: Option<&str>, keep: impl Fn(&App) -> bool) -> bool {
        h.state_mut().caption = caption.map(str::to_owned);
        self.t += 1.0 / FPS;
        // Keep the engine's time and the video's in step.
        let due = self.t0 + Duration::from_secs_f64(self.t * self.slow);
        match due.checked_duration_since(Instant::now()) {
            Some(wait) => std::thread::sleep(wait),
            None => self.late = self.late.max(Instant::now().duration_since(due).as_secs_f64()),
        }
        h.step();
        if !keep(&h.state().w.0) {
            return false;
        }
        self.acc += 1.0 / speed;
        if self.acc >= 1.0 - 1e-9 {
            self.acc -= 1.0;
            h.render().unwrap().save(self.dir.join(format!("frame-{:05}.png", self.saved))).unwrap();
            self.saved += 1;
        }
        true
    }

    fn done(&self) {
        eprintln!("{}: {} frames ({:.1} s), {:.1} s behind at most", self.dir.display(), self.saved, self.saved as f64 / FPS, self.late);
    }
}

fn demo<'h>(h: &'h Harness<Demo>) -> &'h App {
    &h.state().w.0
}

fn demo_mut<'h>(h: &'h mut Harness<'_, Demo>) -> &'h mut App {
    &mut h.state_mut().w.0
}

/// A CD from start to finish: the lookups, Choose asking, the rip, the
/// next disc, then the first's files let through into the library.
#[test]
#[ignore = "renders the demo video's frames; run by hand"]
fn demo_video() {
    // Wall seconds per second of engine time: rendering and saving a
    // frame takes longer than a thirtieth of a second.
    const SLOW: f64 = 12.0;
    let e = env();
    let mut h = demo_window(&e, simulated(None), 0.4 * SLOW);
    let i = demo(&h).sel;
    let mut rec = Rec::new("cd", SLOW);
    #[derive(PartialEq, Debug)]
    enum P {
        Ready,
        LookUp,
        Asking,
        Rip,
        Fast,
        NextDisc,
        ToHold,
        Hold,
        Filing,
    }
    let mut p = P::Ready;
    let mut at = 0.0;
    let mut first = None::<u64>;
    let mut end = None::<f64>;
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(1200), "the demo never finished, at {p:?}");
        let t = rec.t;
        let since = t - at;
        let a = demo(&h);
        let mut next = None;
        let (speed, caption): (f64, Option<&str>) = match p {
            P::Ready => {
                if (3.0..3.0 + 1.0 / FPS).contains(&since) {
                    let to = close_up(a);
                    demo_mut(&mut h).glide_to(i, to, 0.9);
                }
                if since >= 4.2 {
                    h.get_by_label("Start").click_accesskit();
                    next = Some(P::LookUp);
                }
                (1.0, Some("The CDs recipe: each step a CD goes through, left to right"))
            }
            P::LookUp => {
                if matches!(a.tray, Some(Tray::Question { .. })) {
                    next = Some(P::Asking);
                }
                (1.5, Some("Start: it reads the disc and looks it up"))
            }
            P::Asking => {
                let radios = || h.get_all_by_role(egui::accesskit::Role::RadioButton).filter(|n| !n.accesskit_node().label().is_some_and(|l| l.starts_with("Appearance")));
                if (3.0..3.0 + 1.0 / FPS).contains(&since) {
                    if let Some(n) = radios().nth(1) {
                        n.click_accesskit();
                    }
                }
                if (4.2..4.2 + 1.0 / FPS).contains(&since) {
                    if let Some(n) = radios().next() {
                        n.click_accesskit();
                    }
                }
                if let Some(b) = h.query_by_label("Use this one").filter(|_| since >= 5.5) {
                    b.click_accesskit();
                    next = Some(P::Rip);
                }
                (1.0, Some("Not sure which album it is, so it asks. The best match is already picked."))
            }
            P::Rip => {
                if since >= 5.0 {
                    next = Some(P::Fast);
                }
                (1.0, Some("Each track is read, and checked against AccurateRip"))
            }
            P::Fast => {
                if a.m.runs.get(&i).is_some_and(|v| v.len() > 1) {
                    // The first disc stays on the canvas; the next shows
                    // in the list.
                    first = a.m.runs[&i].first().map(|r| r.id);
                    demo_mut(&mut h).show_run(i, first);
                    next = Some(P::NextDisc);
                }
                (4.0, if since < 1.0 { Some("Each track is read, and checked against AccurateRip") } else { None })
            }
            P::NextDisc => {
                if since >= 5.0 {
                    next = Some(P::ToHold);
                }
                (1.0, Some("Next disc: out comes this CD and in goes the next, under CDs in the list. This one carries on."))
            }
            P::ToHold => {
                let held = a.m.runs[&i].iter().find(|r| Some(r.id) == first).is_some_and(|r| r.snap.steps["hold"].detail == "All here");
                if held {
                    demo_mut(&mut h).open(Tray::Step { id: "hold".into(), focus: None });
                    next = Some(P::Hold);
                }
                (3.0, None)
            }
            P::Hold => {
                if since >= 4.0 {
                    if let Some(b) = h.get_all_by_role(egui::accesskit::Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("Let ") && l.ends_with(" through"))) {
                        b.click_accesskit();
                    }
                    next = Some(P::Filing);
                }
                (1.0, Some("Hold keeps the files back until you've had a look"))
            }
            P::Filing => {
                // Done once the library has them, before the canvas goes
                // back to the next disc.
                let filed = a.m.runs[&i].iter().find(|r| Some(r.id) == first).is_none_or(|r| r.snap.finished || r.snap.steps["lib1"].state == drt_engine::StepState::Ok);
                if filed && end.is_none() {
                    end = Some(t);
                }
                // The canvas whole again, to see the library fill.
                if since >= 0.4 && demo(&h).tray.is_some() {
                    demo_mut(&mut h).close_tray();
                }
                (1.0, Some("Let through: the tracks go into your library, named and tagged"))
            }
        };
        if end.is_some_and(|e| t >= e + 2.0) {
            break;
        }
        // Not a frame of the canvas going back to the next disc.
        if !rec.tick_while(&mut h, speed, caption, |a| first.is_none() || !a.m.shows_newest(i)) {
            break;
        }
        if let Some(n) = next {
            eprintln!("{:.1} s: {n:?}", rec.t);
            p = n;
            at = rec.t;
        }
    }
    rec.done();
}

/// When a track doesn't match: through to Rip asking about track 9, then
/// read again.
#[test]
#[ignore = "renders the demo video's frames; run by hand"]
fn demo_video_retry() {
    const SLOW: f64 = 12.0;
    let e = env();
    let mut h = demo_window(&e, simulated(Some(9)), 0.4 * SLOW);
    demo_mut(&mut h).m.repeat_limit = Some(0);
    let i = demo(&h).sel;
    let to = close_up(demo(&h));
    demo_mut(&mut h).set_view(i, (to.zoom, to.pan));
    h.run_steps(2);
    let mut rec = Rec::new("retry", SLOW);
    #[derive(PartialEq, Debug)]
    enum P {
        Ready,
        LookUp,
        Fast,
        NoMatch,
        Faster,
        Asks,
        Again,
        Agrees,
    }
    let mut p = P::Ready;
    let mut at = 0.0;
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(1200), "the demo never finished, at {p:?}");
        let since = rec.t - at;
        let a = demo(&h);
        let snap = a.m.snapshot(i);
        let rip = &snap.steps["rip"];
        let nine = rip.items.iter().find(|it| it.id == "track-9").map(|it| it.state);
        let mut next = None;
        let (speed, caption): (f64, Option<&str>) = match p {
            P::Ready => {
                if since >= 2.5 {
                    h.get_by_label("Start").click_accesskit();
                    next = Some(P::LookUp);
                }
                (1.0, Some("This CD has a scratch on track 9"))
            }
            P::LookUp => {
                if let Some(b) = h.query_by_label("Use this one").filter(|_| since > 0.5) {
                    b.click_accesskit();
                    next = Some(P::Fast);
                }
                (3.0, Some("This CD has a scratch on track 9"))
            }
            P::Fast => {
                if nine == Some(ItemState::Bad) {
                    next = Some(P::NoMatch);
                }
                (4.0, None)
            }
            P::NoMatch => {
                if since >= 3.5 {
                    next = Some(P::Faster);
                }
                (1.0, Some("Track 9 doesn't match AccurateRip. The rip goes on with the rest."))
            }
            P::Faster => {
                if rip.state == drt_engine::StepState::Bad {
                    demo_mut(&mut h).open(Tray::Step { id: "rip".into(), focus: None });
                    next = Some(P::Asks);
                }
                (4.0, None)
            }
            P::Asks => {
                if since >= 5.0 {
                    h.get_by_label("Read again, Broken Bricks").click_accesskit();
                    next = Some(P::Again);
                }
                (1.0, Some("Then it waits for you: read it again, keep it, or skip it"))
            }
            P::Again => {
                if nine == Some(ItemState::Ok) {
                    next = Some(P::Agrees);
                }
                (1.0, Some("Read again, carefully"))
            }
            P::Agrees => {
                if since >= 4.0 {
                    break;
                }
                (1.0, Some("Two reads agree: the track is good, and goes on with the rest"))
            }
        };
        rec.tick(&mut h, speed, caption);
        if let Some(n) = next {
            eprintln!("{:.1} s: {n:?}", rec.t);
            p = n;
            at = rec.t;
        }
    }
    for _ in 0..20 {
        rec.tick(&mut h, 1.0, None);
    }
    rec.done();
}
