//! The pictures in the docs, rendered headless from the real window on
//! simulated steps. Both tests are ignored; run them by hand:
//!
//! ```sh
//! cargo test -p drt-app --test demo -- --ignored --test-threads 1
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
//! `demo_video` writes `target/demo/frame-00000.png`…, 30 frames a second
//! of a simulated CD going through the CD recipe, with motion on. Then:
//!
//! ```sh
//! ffmpeg -framerate 30 -i target/demo/frame-%05d.png -c:v libvpx-vp9 \
//!   -crf 30 -b:v 0 -row-mt 1 -pix_fmt yuv420p docs/demo.webm
//! ffmpeg -framerate 30 -i target/demo/frame-%05d.png -vf fps=15 \
//!   -c:v libwebp_anim -q:v 75 -compression_level 6 -loop 0 docs/demo.webp
//! rm -r target/demo
//! ```

use drt_app::{App, Deps, Model, Store, Tray, Window};
use drt_engine::{Engine, ItemState, Setting};
use drt_steps::{recipes, simulated};
use egui_kittest::kittest::Queryable;
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
        engine,
        store: Store::new(e.dir.path().join("config")),
        drives: drives.clone(),
        rt: e.rt.handle().clone(),
        open_folder: Arc::new(|_| Ok(())),
        pick_folder: Arc::new(|_| None),
        send_test: Arc::new(|_, _, _| Box::pin(async { Ok(vec![]) })),
        connected: Arc::new(move || drives.clone()),
    };
    let mut model = Model::new(deps);
    model.repeat_limit = Some(0);
    let mut app = App::new(model, false);
    app.motion = motion;
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

#[test]
#[ignore = "renders the demo video's frames; run by hand"]
fn demo_video() {
    const FPS: f64 = 30.0;
    // Wall seconds per second of video: rendering and saving a frame takes
    // longer than a thirtieth of a second, so the engine runs this much
    // slower and every frame lands on its own moment.
    const SLOW: f64 = 9.0;
    let dir = target("demo");
    for f in std::fs::read_dir(&dir).unwrap().flatten() {
        let _ = std::fs::remove_file(f.path());
    }
    let e = env();
    let mut h = window(&e, simulated(None), 0.4 * SLOW, egui::vec2(1280.0, 720.0), 1.0, FPS as f32, true);
    app_mut(&mut h).m.repeat_limit = Some(1);
    pick(&mut h, "CDs");
    // Fit glides; let it settle before the first frame.
    h.run_steps(30);
    let t0 = Instant::now();
    let mut started = None::<f64>;
    let mut asked_at = None::<f64>;
    let mut answered = false;
    let mut late = 0.0f64;
    let mut second_disc = None::<f64>;
    for k in 0..(50.0 * FPS) as usize {
        let t = k as f64 / FPS;
        // Keep the engine's time and the video's in step.
        let due = t0 + Duration::from_secs_f64(t * SLOW);
        match due.checked_duration_since(Instant::now()) {
            Some(wait) => std::thread::sleep(wait),
            None => late = late.max(Instant::now().duration_since(due).as_secs_f64()),
        }
        let i = app(&h).sel;
        if started.is_none() && t >= 1.5 {
            h.get_by_label("Start").click_accesskit();
            started = Some(t);
        }
        if asked_at.is_none() && matches!(app(&h).tray, Some(Tray::Question { .. })) {
            asked_at = Some(t);
        }
        if let (Some(a), false) = (asked_at, answered) {
            if t >= a + 1.6 && t < a + 1.7 {
                // The second release first, to show the choice moving.
                if let Some(n) = h.get_all_by_role(egui::accesskit::Role::RadioButton).nth(1) {
                    n.click_accesskit();
                }
            }
            if t >= a + 2.6 && t < a + 2.7 {
                if let Some(n) = h.get_all_by_role(egui::accesskit::Role::RadioButton).next() {
                    n.click_accesskit();
                }
            }
            if t >= a + 3.4 {
                h.get_by_label("Use this one").click_accesskit();
                answered = true;
            }
        }
        // The next disc is in: show it for a few seconds, then stop.
        if second_disc.is_none() && answered && app(&h).m.runs.get(&i).is_some_and(|v| v.len() > 1) {
            second_disc = Some(t);
        }
        if second_disc.is_some_and(|s| t >= s + 5.0) {
            break;
        }
        h.step();
        h.render().unwrap().save(dir.join(format!("frame-{k:05}.png"))).unwrap();
    }
    eprintln!("{} frames, {:.1} s behind at most", std::fs::read_dir(&dir).unwrap().count(), late);
    assert!(answered && second_disc.is_some(), "the video didn't get to the next disc");
}
