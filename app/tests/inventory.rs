//! An inventory of every built-in step's card and sidebar, in its main
//! states, for the cards audit (docs/ux/cards-audit.md). Ignored; run by
//! hand:
//!
//! ```sh
//! cargo test -p drt-app --test inventory -- --ignored --test-threads 1
//! ```
//!
//! Writes `target/inventory/<recipe>-<moment>-<step>[-<tab>].png` (light,
//! 1280 × 720, the recipe list cropped off) and `target/inventory/index.tsv`:
//! each shot with its step's name, subtitle, status and detail as the
//! snapshot has them.

use drt_app::{App, Deps, Model, Store, Tray, Window};
use drt_engine::{ItemState, Node, Setting, StepState};
use drt_steps::{recipes, simulated};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use serde_json::Value;
use std::io::Write as _;
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

/// The window on simulated steps, both drives measured, TMDb's key set.
/// `plugged` lists the drives connected.
fn window<'a>(e: &Env, reg: drt_engine::Registry, time_scale: f64, plugged: Vec<String>) -> Harness<'a, Window> {
    let mut engine = drt_engine::Engine::new(reg);
    engine.time_scale = time_scale;
    engine.work_root = e.dir.path().join("work");
    {
        let mut s = engine.settings.lock().unwrap();
        s.set_drive(recipes::BLU_RAY, "offset", Setting::detected(Value::from(103), "AccurateRip"));
        s.set_drive(recipes::DVD, "offset", Setting::detected(Value::from(6), "AccurateRip"));
        s.set_app("tmdb", "api_key", Setting::chosen(Value::from("demo-key")));
    }
    let deps = Deps {
        engine,
        store: Store::new(e.dir.path().join("config")),
        drives: vec![recipes::BLU_RAY.to_owned(), recipes::DVD.to_owned()],
        rt: e.rt.handle().clone(),
        open_folder: Arc::new(|_| Ok(())),
        pick_folder: Arc::new(|_| None),
        send_test: Arc::new(|_, _, _| Box::pin(async { Ok(vec![]) })),
        alert: Arc::new(|_, _, _| Box::pin(async { Ok(()) })),
        open_tray: Arc::new(|_| Ok(())),
        connected: Arc::new(move || plugged.clone()),
    };
    let mut model = Model::new(deps);
    model.repeat_limit = Some(0);
    let mut app = App::new(model, false);
    app.motion = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .with_pixels_per_point(1.0)
        .with_render_cursor(false)
        .with_context_setup(|ctx| ctx.set_fonts(drt_app::theme::fonts()))
        .wgpu_setup(renderer())
        .build_eframe(|_| Window(app));
    h.run_steps(3);
    h
}

fn both() -> Vec<String> {
    vec![recipes::BLU_RAY.to_owned(), recipes::DVD.to_owned()]
}

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

fn out() -> PathBuf {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/inventory");
    std::fs::create_dir_all(&out).unwrap();
    out
}

fn until(h: &mut Harness<Window>, what: &str, cond: impl Fn(&App) -> bool) {
    let start = Instant::now();
    while !cond(app(h)) {
        assert!(start.elapsed() < Duration::from_secs(120), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run_steps(3);
}

fn snap(h: &Harness<Window>) -> drt_engine::Snapshot {
    app(h).m.snapshot(app(h).sel)
}

/// Picks a recipe by name, at 100%.
fn pick(h: &mut Harness<Window>, name: &str) {
    let i = app(h).m.recipes.iter().position(|r| r.name == name).unwrap();
    app_mut(h).select(i);
    h.run_steps(3);
}

/// The index is begun afresh once per run of this file.
static INDEX: std::sync::Once = std::sync::Once::new();

/// Renders the window, light, without the recipe list.
fn render(h: &mut Harness<Window>) -> image::RgbaImage {
    app_mut(h).clock = Some(0.3);
    app_mut(h).dark = false;
    h.run_steps(4);
    let img = h.render().unwrap();
    app_mut(h).clock = None;
    let (w, ht) = (img.width(), img.height());
    image::imageops::crop_imm(&img, 232, 0, w - 232, ht).to_image()
}

/// Saves a render, and notes what the snapshot says of `id` (every step,
/// for a canvas shot).
fn keep_shot(h: &Harness<Window>, img: &image::RgbaImage, name: &str, id: Option<&str>) {
    img.save(out().join(format!("{name}.png"))).unwrap();
    INDEX.call_once(|| {
        let _ = std::fs::remove_file(out().join("index.tsv"));
    });
    let s = snap(h);
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(out().join("index.tsv")).unwrap();
    for (sid, v) in &s.steps {
        if id.is_some_and(|x| x != sid) {
            continue;
        }
        writeln!(f, "{name}\t{sid}\t{:?}\t{}\t{}\t{}\t{}", v.state, v.name, v.from, v.status, v.detail).unwrap();
    }
}

fn save(h: &mut Harness<Window>, name: &str, id: Option<&str>) {
    let img = render(h);
    keep_shot(h, &img, name, id);
}

/// The canvas with no sidebar: each tile's own outline shows.
fn canvas(h: &mut Harness<Window>, name: &str, fit: bool) {
    app_mut(h).close_tray();
    h.run_steps(3);
    if fit {
        h.get_by_label("Fit to window").click();
        h.run_steps(3);
    }
    save(h, name, None);
    if fit {
        let i = app(h).sel;
        app_mut(h).view(i).zoom = 1.0;
        h.run_steps(2);
    }
}

/// A step's sidebar: one stack, no tabs.
fn sidebar(h: &mut Harness<Window>, prefix: &str, id: &str) {
    app_mut(h).open(Tray::Step { id: id.into(), focus: None });
    h.run_steps(4);
    let base = render(h);
    keep_shot(h, &base, &format!("{prefix}-{id}"), Some(id));
}

fn sidebars(h: &mut Harness<Window>, prefix: &str, ids: &[&str]) {
    for id in ids {
        sidebar(h, prefix, id);
    }
}

/// The question's sidebar, as it opens by itself.
fn question(h: &mut Harness<Window>, name: &str) {
    let Some((step, _)) = snap(h).question else { panic!("no question") };
    app_mut(h).open(Tray::Question { step: step.clone() });
    h.run_steps(4);
    save(h, name, Some(&step));
}

/// Answers with what it suggests, and lets holds through, until the disc
/// is done.
fn to_the_end(h: &mut Harness<Window>) {
    let start = Instant::now();
    loop {
        let s = snap(h);
        if s.finished {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(120), "timed out");
        if let Some((step, _)) = &s.question {
            app_mut(h).open(Tray::Question { step: step.clone() });
            h.run_steps(3);
            if let Some(b) = h.query_by_label("Use this one").or_else(|| h.query_by_label("Use these")) {
                b.click_accesskit();
            }
        } else if let Some((id, _)) = s.steps.iter().find(|(_, v)| v.step == "hold" && v.detail == "All here") {
            app_mut(h).open(Tray::Step { id: id.clone(), focus: None });
            h.run_steps(3);
            if let Some(b) = h.get_all_by_role(egui::accesskit::Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("Let "))) {
                b.click_accesskit();
            }
        }
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    app_mut(h).close_tray();
    h.run_steps(3);
}

fn answer(h: &mut Harness<Window>) {
    let Some((step, _)) = snap(h).question else { return };
    app_mut(h).open(Tray::Question { step });
    h.run_steps(3);
    if let Some(b) = h.query_by_label("Use this one").or_else(|| h.query_by_label("Use these")) {
        b.click_accesskit();
    }
    h.run_steps(3);
}

fn start(h: &mut Harness<Window>) {
    h.get_by_label("Start").click();
    h.run_steps(2);
}

fn state_of(a: &App, id: &str) -> StepState {
    a.m.snapshot(a.sel).steps[id].state
}

// ---- CDs -------------------------------------------------------------------------------------

const CD: [&str; 13] = ["drive", "cd", "mb", "cdtext", "choose", "art", "rip", "flac", "review", "hold", "lib1", "mp3", "lib2"];

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn cd_waiting_asking_holding_done() {
    let e = env();
    let mut h = window(&e, simulated(None), 0.0, both());
    pick(&mut h, "CDs");
    // Waiting: before Start.
    canvas(&mut h, "cd-0idle-canvas", true);
    let mut idle: Vec<&str> = CD.to_vec();
    idle.push("next");
    sidebars(&mut h, "cd-0idle", &idle);
    // Asking: Choose's question.
    start(&mut h);
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    canvas(&mut h, "cd-1ask-canvas", true);
    question(&mut h, "cd-1ask-choose-question");
    sidebars(&mut h, "cd-1ask", &["drive", "cd", "mb", "cdtext", "art", "rip"]);
    // Holding: the FLACs in Review.
    answer(&mut h);
    until(&mut h, "the hold", |a| a.m.snapshot(a.sel).steps["hold"].detail == "All here");
    canvas(&mut h, "cd-2hold-canvas", true);
    sidebars(&mut h, "cd-2hold", &["hold", "lib1", "next"]);
    // Done.
    to_the_end(&mut h);
    canvas(&mut h, "cd-3done-canvas", true);
    for id in CD {
        sidebar(&mut h, "cd-3done", id);
    }
    sidebar(&mut h, "cd-3done", "next");
}

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn cd_busy() {
    let e = env();
    let mut h = window(&e, simulated(None), 2.0, both());
    pick(&mut h, "CDs");
    start(&mut h);
    until(&mut h, "the lookups", |a| state_of(a, "mb") == StepState::Busy);
    canvas(&mut h, "cd-busy0-canvas", true);
    sidebars(&mut h, "cd-busy0", &["mb"]);
    until(&mut h, "the question", |a| a.m.snapshot(a.sel).question.is_some());
    answer(&mut h);
    until(&mut h, "the cover", |a| state_of(a, "art") == StepState::Busy || state_of(a, "rip") == StepState::Busy);
    sidebars(&mut h, "cd-busy1", &["art"]);
    until(&mut h, "five tracks", |a| a.m.snapshot(a.sel).steps["rip"].count(ItemState::Ok) >= 5);
    canvas(&mut h, "cd-busy2-canvas", true);
    canvas(&mut h, "cd-busy2-canvas100", false);
    sidebars(&mut h, "cd-busy2", &["drive", "rip", "flac", "review", "hold", "mp3", "lib2", "next"]);
    sidebar(&mut h, "cd-busy2", "rip");
}

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn cd_bad_track() {
    let e = env();
    let mut h = window(&e, simulated(Some(9)), 0.0, both());
    pick(&mut h, "CDs");
    start(&mut h);
    until(&mut h, "the question", |a| a.m.snapshot(a.sel).question.is_some());
    answer(&mut h);
    until(&mut h, "the bad track", |a| state_of(a, "rip") == StepState::Bad);
    canvas(&mut h, "cd-bad-canvas", true);
    sidebar(&mut h, "cd-bad", "rip");
}

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn cd_setup() {
    // A folder not set, and the drive not connected.
    let e = env();
    let mut r = recipes::cd();
    if let Some(Node::Step { params, .. }) = r.root.find_mut("lib2") {
        params.remove("folder");
    }
    let dir = e.dir.path().join("config");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("recipes.json"), serde_json::to_string(&vec![r]).unwrap()).unwrap();
    let mut h = window(&e, simulated(None), 0.0, vec![recipes::DVD.to_owned()]);
    pick(&mut h, "CDs");
    canvas(&mut h, "cd-setup-canvas", true);
    sidebars(&mut h, "cd-setup", &["drive", "lib2"]);
}

// ---- Archive everything ------------------------------------------------------------------------

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn archive() {
    let e = env();
    let mut h = window(&e, simulated(None), 0.0, both());
    pick(&mut h, "Archive everything");
    canvas(&mut h, "ar-0idle-canvas", true);
    sidebars(&mut h, "ar-0idle", &["a_dg", "a_gn", "a_lyr", "a_opus", "a_rg", "a_cue", "a_tell"]);
    start(&mut h);
    until(&mut h, "the question", |a| a.m.snapshot(a.sel).question.is_some());
    canvas(&mut h, "ar-1ask-canvas", true);
    question(&mut h, "ar-1ask-a_ch-question");
    sidebar(&mut h, "ar-1ask", "a_dg");
    sidebar(&mut h, "ar-1ask", "a_gn");
    to_the_end(&mut h);
    canvas(&mut h, "ar-3done-canvas", true);
    for id in ["a_ch", "a_lyr", "a_opus", "a_rg", "a_cue", "a_lrc", "a_nas", "a_tell"] {
        sidebar(&mut h, "ar-3done", id);
    }
}

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn archive_busy() {
    let e = env();
    let mut h = window(&e, simulated(None), 2.0, both());
    pick(&mut h, "Archive everything");
    start(&mut h);
    until(&mut h, "the question", |a| a.m.snapshot(a.sel).question.is_some());
    answer(&mut h);
    until(&mut h, "the lyrics", |a| state_of(a, "a_lyr") == StepState::Busy);
    sidebars(&mut h, "ar-busy1", &["a_lyr"]);
    until(&mut h, "five tracks", |a| a.m.snapshot(a.sel).steps["a_rip"].count(ItemState::Ok) >= 5);
    sidebars(&mut h, "ar-busy2", &["a_opus", "a_rg", "a_cue", "a_ph", "a_tell"]);
}

// ---- Data discs ----------------------------------------------------------------------------

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn data() {
    let e = env();
    let mut h = window(&e, simulated(None), 0.0, both());
    pick(&mut h, "Data discs");
    canvas(&mut h, "da-0idle-canvas", true);
    sidebars(&mut h, "da-0idle", &["d_disc", "d_image", "d_check", "d_chd", "d_lib"]);
    start(&mut h);
    to_the_end(&mut h);
    canvas(&mut h, "da-3done-canvas", true);
    for id in ["d_drive", "d_disc", "d_image", "d_check", "d_chd", "d_lib"] {
        sidebar(&mut h, "da-3done", id);
    }
}

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn data_busy_and_bad() {
    let e = env();
    let mut h = window(&e, simulated(None), 2.0, both());
    pick(&mut h, "Data discs");
    start(&mut h);
    until(&mut h, "the image", |a| state_of(a, "d_image") == StepState::Busy && !a.m.snapshot(a.sel).steps["d_image"].items.is_empty());
    sidebars(&mut h, "da-busy", &["d_image"]);
    until(&mut h, "the CHD", |a| state_of(a, "d_chd") == StepState::Busy);
    sidebars(&mut h, "da-busy", &["d_chd"]);
    // Bad sectors.
    let e = env();
    let mut h = window(&e, simulated(Some(1)), 0.0, both());
    pick(&mut h, "Data discs");
    start(&mut h);
    until(&mut h, "the bad sectors", |a| state_of(a, "d_image") == StepState::Bad);
    canvas(&mut h, "da-bad-canvas", true);
    sidebar(&mut h, "da-bad", "d_image");
}

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn keep() {
    // Plain image is in no built-in recipe: the data recipe with it for CHD.
    let e = env();
    let mut r = recipes::data();
    if let Some(Node::Step { step, .. }) = r.root.find_mut("d_chd") {
        *step = "keep".into();
    }
    let dir = e.dir.path().join("config");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("recipes.json"), serde_json::to_string(&vec![r]).unwrap()).unwrap();
    let mut h = window(&e, simulated(None), 0.0, both());
    pick(&mut h, "Data discs");
    sidebars(&mut h, "keep-0idle", &["d_chd"]);
    start(&mut h);
    to_the_end(&mut h);
    sidebar(&mut h, "keep-3done", "d_chd");
}

// ---- Games -----------------------------------------------------------------------------------

const GAME: [&str; 10] = ["g_drive", "g_disc", "g_rip", "g_check", "g_redump", "g_choose", "g_chd", "g_lib", "g_sbi", "g_m3u"];

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn games() {
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::COLONY_WARS_1]));
    let mut h = window(&e, reg, 0.0, both());
    pick(&mut h, "Games");
    canvas(&mut h, "ga-0idle-canvas", true);
    sidebars(&mut h, "ga-0idle", &GAME[1..]);
    start(&mut h);
    to_the_end(&mut h);
    canvas(&mut h, "ga-3done-canvas", true);
    for id in GAME {
        sidebar(&mut h, "ga-3done", id);
    }
}

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn games_busy_and_unknown() {
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::APE_ESCAPE]));
    let mut h = window(&e, reg, 2.0, both());
    pick(&mut h, "Games");
    start(&mut h);
    until(&mut h, "the rip", |a| state_of(a, "g_rip") == StepState::Busy && !a.m.snapshot(a.sel).steps["g_rip"].items.is_empty());
    sidebars(&mut h, "ga-busy", &["g_rip"]);
    until(&mut h, "the CHD", |a| state_of(a, "g_chd") == StepState::Busy);
    canvas(&mut h, "ga-busy-canvas", true);
    sidebars(&mut h, "ga-busy", &["g_chd"]);
    to_the_end(&mut h);
    sidebars(&mut h, "ga-ape", &["g_sbi", "g_m3u"]);
    // A dump Redump doesn't know.
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::TOMB_RAIDER_UNKNOWN]));
    let mut h = window(&e, reg, 0.0, both());
    pick(&mut h, "Games");
    start(&mut h);
    let asked = |a: &App| matches!(&a.m.snapshot(a.sel).question, Some((s, _)) if s == "g_choose");
    until(&mut h, "the check or the question", |a| asked(a) || state_of(a, "g_check") == StepState::Bad);
    if !asked(app(&h)) {
        canvas(&mut h, "ga-unknown-canvas", true);
        sidebar(&mut h, "ga-unknown", "g_check");
        app_mut(&mut h).open(Tray::Step { id: "g_check".into(), focus: None });
        h.run_steps(3);
        h.get_all_by_label_contains("Keep, ").next().expect("Keep on the dump's row").click_accesskit();
        h.run_steps(2);
        until(&mut h, "the question", asked);
    }
    question(&mut h, "ga-unknown-g_choose-question");
    sidebar(&mut h, "ga-unknown", "g_redump");
}

// ---- Films and TV ----------------------------------------------------------------------------

const VIDEO: [&str; 8] = ["v_drive", "v_disc", "v_tmdb", "v_choose", "v_rip", "v_make", "v_lib", "v_next"];

fn video_reg(disc: drt_steps::real::video::sim::SimDisc) -> drt_engine::Registry {
    let mut reg = simulated(None);
    drt_steps::real::video::sim::register(&mut reg, disc);
    reg
}

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn video() {
    use drt_steps::real::video::sim::SimDisc;
    let e = env();
    let mut h = window(&e, video_reg(SimDisc::Film), 0.0, both());
    pick(&mut h, "Films and TV");
    canvas(&mut h, "vi-0idle-canvas", true);
    sidebars(&mut h, "vi-0idle", &VIDEO[1..]);
    start(&mut h);
    to_the_end(&mut h);
    canvas(&mut h, "vi-3done-canvas", true);
    for id in VIDEO {
        sidebar(&mut h, "vi-3done", id);
    }
    // A season disc asks which episode each title is.
    let e = env();
    let mut h = window(&e, video_reg(SimDisc::Season), 0.0, both());
    pick(&mut h, "Films and TV");
    start(&mut h);
    until(&mut h, "the question", |a| a.m.snapshot(a.sel).question.is_some());
    canvas(&mut h, "vi-season-canvas", true);
    question(&mut h, "vi-season-v_choose-question");
}

#[test]
#[ignore = "renders the cards inventory; run by hand"]
fn video_busy() {
    use drt_steps::real::video::sim::SimDisc;
    let e = env();
    let mut h = window(&e, video_reg(SimDisc::Film), 2.0, both());
    pick(&mut h, "Films and TV");
    start(&mut h);
    until(&mut h, "TMDb", |a| state_of(a, "v_tmdb") == StepState::Busy || state_of(a, "v_tmdb") == StepState::Ok);
    sidebars(&mut h, "vi-busy0", &["v_tmdb"]);
    until(&mut h, "the rip", |a| state_of(a, "v_rip") == StepState::Busy && !a.m.snapshot(a.sel).steps["v_rip"].items.is_empty());
    sidebars(&mut h, "vi-busy", &["v_rip"]);
    until(&mut h, "the transcode", |a| state_of(a, "v_make") == StepState::Busy);
    canvas(&mut h, "vi-busy-canvas", true);
    sidebars(&mut h, "vi-busy", &["v_make"]);
}
