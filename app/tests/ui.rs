//! The window, headless: the real engine on simulated steps, the real UI
//! in egui_kittest, driven through its accessibility tree. Each test keeps
//! its config in a temporary folder, and leaves screenshots of the states
//! it passes through in `app/shots/`, light and dark.

use drt_app::{App, Deps, Model, Store, Tray, Window};
use drt_engine::{Engine, ItemState, Node, Recipe, Setting, Settings, StepState};
use drt_steps::{recipes, simulated};
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Env {
    _rt: tokio::runtime::Runtime,
    dir: tempfile::TempDir,
    opened: Arc<Mutex<Vec<PathBuf>>>,
    /// The drives plugged in: a test can unplug one.
    plugged: Arc<Mutex<Vec<String>>>,
    /// Where each folder picker started.
    picked_from: Arc<Mutex<Vec<Option<PathBuf>>>>,
    /// Each Notify test asked for: to this computer, the server, the topic.
    tests_sent: Arc<Mutex<Vec<(bool, String, String)>>>,
    /// Each drive whose tray was opened.
    trays: Arc<Mutex<Vec<String>>>,
    /// Folders are really looked at (else every one can take a disc).
    folders: bool,
    /// How MakeMKV is found and run: not needed, unless a test says.
    makemkv: drt_app::makemkv::Hooks,
}

fn env() -> Env {
    Env {
        _rt: tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap(),
        dir: tempfile::tempdir().unwrap(),
        opened: Arc::default(),
        plugged: Arc::new(Mutex::new(vec![recipes::BLU_RAY.into(), recipes::DVD.into()])),
        picked_from: Arc::default(),
        tests_sent: Arc::default(),
        trays: Arc::default(),
        folders: false,
        makemkv: drt_app::makemkv::Hooks::not_needed(),
    }
}

/// The window on simulated steps, at 1280×720. `offsets` gives both drives
/// their offsets, as if measured.
fn window<'a>(e: &Env, bad: Option<u32>, offsets: bool, time_scale: f64) -> Harness<'a, Window> {
    window_at(e, bad, offsets, time_scale, egui::vec2(1280.0, 720.0), 1.0)
}

fn window_at<'a>(e: &Env, bad: Option<u32>, offsets: bool, time_scale: f64, size: egui::Vec2, ppp: f32) -> Harness<'a, Window> {
    window_on(e, simulated(bad), offsets, time_scale, size, ppp)
}

fn window_on<'a>(e: &Env, reg: drt_engine::Registry, offsets: bool, time_scale: f64, size: egui::Vec2, ppp: f32) -> Harness<'a, Window> {
    let mut engine = Engine::new(reg);
    engine.time_scale = time_scale;
    engine.work_root = e.dir.path().join("work");
    if offsets {
        let mut s = engine.settings.lock().unwrap();
        s.set_drive(recipes::BLU_RAY, "offset", Setting::detected(Value::from(103), "AccurateRip"));
        s.set_drive(recipes::DVD, "offset", Setting::detected(Value::from(6), "AccurateRip"));
    }
    let opened = e.opened.clone();
    let picked = e.dir.path().join("picked");
    let deps = Deps {
        makemkv: e.makemkv.clone(),
        // Every folder can take a disc, unless the test looks.
        // Only the test's own folders: never this computer's.
        reach: if e.folders {
            let mine = e.dir.path().to_owned();
            Arc::new(move |p, need| if p.starts_with(&mine) { drt_steps::real::reach::look(p, need, drt_steps::real::reach::statvfs_free) } else { Ok(()) })
        } else {
            Arc::new(|_, _| Ok(()))
        },
        engine,
        store: Store::new(e.dir.path().join("config")),
        // The drives known: those plugged in (a test can start with one).
        drives: e.plugged.lock().unwrap().clone(),
        rt: e._rt.handle().clone(),
        open_folder: Arc::new(move |p| {
            opened.lock().unwrap().push(p.to_owned());
            Ok(())
        }),
        pick_folder: {
            let from = e.picked_from.clone();
            Arc::new(move |start| {
                from.lock().unwrap().push(start);
                Some(picked.clone())
            })
        },
        open_tray: {
            let trays = e.trays.clone();
            Arc::new(move |d| {
                trays.lock().unwrap().push(d.to_owned());
                Ok(())
            })
        },
        connected: {
            let plugged = e.plugged.clone();
            Arc::new(move || plugged.lock().unwrap().clone())
        },
        // Nothing goes out: a server named *down* can't be reached.
        send_test: {
            let sent = e.tests_sent.clone();
            Arc::new(move |desktop, server, topic| {
                let down = server.contains("down");
                sent.lock().unwrap().push((desktop, server, topic));
                Box::pin(async move {
                    if down {
                        anyhow::bail!("Sent to this computer, but ntfy: couldn't reach down.example")
                    }
                    Ok(vec!["this computer", "ntfy"])
                })
            })
        },
    };
    let mut model = Model::new(deps);
    // No disc after the first unless a test asks: these tests look at one.
    model.repeat_limit = Some(0);
    let mut app = App::new(model, false);
    // Still, so what a test sees doesn't hang on timing; one test turns
    // motion on to see it move.
    app.motion = false;
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(ppp)
        .with_render_cursor(false)
        .with_context_setup(|ctx| ctx.set_fonts(drt_app::theme::fonts()))
        .wgpu_setup(renderer())
        .build_eframe(|_| Window(app));
    h.run_steps(3);
    h
}

/// kittest's wgpu setup, without debug object names: tests run in parallel,
/// and the Vulkan loader's vkSetDebugUtilsObjectNameEXT isn't safe while
/// other threads create and drop devices (it crashed one run in five).
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

/// Steps the window until `cond` holds, as the engine runs beside it.
fn until(h: &mut Harness<Window>, what: &str, cond: impl Fn(&App) -> bool) {
    let start = Instant::now();
    while !cond(app(h)) {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}: {:?}",
            app(h).m.snapshot(app(h).sel).steps.values().map(|v| (v.step.clone(), v.status.clone(), v.state)).collect::<Vec<_>>()
        );
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run_steps(3);
}

fn snap(h: &Harness<Window>) -> drt_engine::Snapshot {
    let a = app(h);
    a.m.snapshot(a.sel)
}

/// Renders the window in light and dark to `app/shots/<name>-<theme>.png`.
fn shot(h: &mut Harness<Window>, name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("shots");
    std::fs::create_dir_all(&dir).unwrap();
    let was = app(h).dark;
    app_mut(h).clock = Some(0.3);
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        app_mut(h).dark = dark;
        h.run_steps(3);
        h.render().unwrap().save(dir.join(format!("{name}-{theme}.png"))).unwrap();
    }
    app_mut(h).dark = was;
    app_mut(h).clock = None;
    h.run_steps(2);
}

fn click(h: &mut Harness<Window>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(2);
}

/// Clicks a tile by its name and status (its accessible label starts so).
fn click_tile(h: &mut Harness<Window>, starts: &str) {
    let n = h
        .get_all_by_role(Role::Button)
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with(starts) || format!("{l},") == starts))
        .unwrap_or_else(|| panic!("no tile labelled {starts}…"));
    n.click_accesskit();
    h.run_steps(2);
}

fn saved_recipes(e: &Env) -> Vec<Recipe> {
    serde_json::from_str(&std::fs::read_to_string(e.dir.path().join("config/recipes.json")).unwrap()).unwrap()
}

fn param(r: &Recipe, id: &str, key: &str) -> Option<Setting<Value>> {
    let mut root = r.root.clone();
    match root.find_mut(id) {
        Some(Node::Step { params, .. }) => params.get(key).cloned(),
        _ => None,
    }
}

#[test]
fn the_built_in_recipes_are_seeded_and_listed() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    assert_eq!(saved_recipes(&e).len(), recipes::all().len(), "seeded into recipes.json");
    h.get_by_label_contains("CDs, Ready");
    h.get_by_label_contains("Archive everything, Ready");
    shot(&mut h, "idle");
    // The most complex recipe, fitted.
    h.get_by_label_contains("Archive everything, Ready").click();
    h.run_steps(2);
    click(&mut h, "Fit to window");
    assert!((app(&h).m.recipes[1].name == "Archive everything") && app(&h).sel == 1);
    shot(&mut h, "complex-fit");
}

#[test]
fn a_question_opens_the_sidebar_and_is_answered_there() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    h.get_by_label("Use this one");
    shot(&mut h, "ask");
    // The second release, then Use this one.
    h.get_all_by_role(Role::RadioButton).filter(|n| !n.accesskit_node().label().is_some_and(|l| l.starts_with("Appearance"))).nth(1).unwrap().click();
    h.run_steps(2);
    click(&mut h, "Use this one");
    until(&mut h, "Choose to finish", |a| a.m.snapshot(0).steps["choose"].state == StepState::Ok);
    assert_eq!(snap(&h).steps["choose"].qualifier, "2002", "the second release was used");
    assert!(app(&h).tray.is_none(), "the sidebar went with the question");
}

/// Puts two of the disc's tracks in a folder of their own for the CDs
/// recipe's Review, so it finds their names taken. Gives the album's folder.
fn review_has_two(h: &mut Harness<Window>, e: &Env) -> PathBuf {
    let lib = e.dir.path().join("review");
    let album = lib.join("The White Stripes/The White Stripes");
    std::fs::create_dir_all(&album).unwrap();
    for name in ["01 - Jimmy the Exploder.flac", "02 - Stop Breaking Down.flac"] {
        std::fs::write(album.join(name), "already here").unwrap();
    }
    app_mut(h).m.set_param(0, "review", "folder", Value::from(lib.display().to_string())).unwrap();
    album
}

#[test]
fn files_already_there_are_asked_about_with_a_button_for_each_answer() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let album = review_has_two(&mut h, &e);
    click(&mut h, "Start");
    to_the_hold(&mut h);
    until(&mut h, "Review's question", |a| matches!(&a.tray, Some(Tray::Question { step }) if step == "review"));
    assert!(h.query_all_by_label_contains("2 files already in The White Stripes / The White Stripes").next().is_some());
    // Each answer is a button that says what it does, with what that means.
    for (button, means) in [("Replace them", "The files there now are overwritten"), ("Keep both", "The new files get \" (2)\" after their names"), ("Skip", "The new files aren't filed")] {
        h.get_by_label(button);
        h.get_by_label(means);
    }
    assert!(h.query_by_label("Use this one").is_none(), "no button that only takes the default");
    let side = app(&h).side_rect().unwrap();
    assert!(side.width() < 700.0, "not the wide sidebar Choose compares lists in: {side:?}");
    shot(&mut h, "folder-names-taken");
    // Replace, not the default: the two are filed under their own names.
    click(&mut h, "Replace them");
    until(&mut h, "Review to file them all", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    let review = &snap(&h).steps["review"];
    let files: Vec<&str> = review.record.files.iter().map(|f| f.path.as_str()).collect();
    assert!(files.contains(&album.join("01 - Jimmy the Exploder.flac").display().to_string().as_str()), "{files:?}");
    assert!(files.iter().all(|f| !f.contains(" (2)")), "{files:?}");
    assert_eq!(review.count(ItemState::Ok), 17);
}

#[test]
fn keep_both_and_skip_from_the_buttons() {
    for (button, ok, named) in [("Keep both", 17, Some("01 - Jimmy the Exploder (2).flac")), ("Skip", 15, None)] {
        let e = env();
        let mut h = window(&e, None, true, 0.0);
        let album = review_has_two(&mut h, &e);
        click(&mut h, "Start");
        to_the_hold(&mut h);
        until(&mut h, "Review's question", |a| matches!(&a.tray, Some(Tray::Question { step }) if step == "review"));
        click(&mut h, button);
        until(&mut h, "Review to finish", |a| a.m.snapshot(0).steps["review"].state == StepState::Ok);
        let review = &snap(&h).steps["review"];
        assert_eq!(review.count(ItemState::Ok), ok, "{button}");
        let files: Vec<&str> = review.record.files.iter().map(|f| f.path.as_str()).collect();
        match named {
            Some(n) => assert!(files.contains(&album.join(n).display().to_string().as_str()), "{files:?}"),
            None => {
                assert_eq!(review.count(ItemState::Skipped), 2);
                assert!(!files.iter().any(|f| f.contains("Jimmy the Exploder")), "{files:?}");
            }
        }
    }
}

#[test]
fn a_hold_is_let_through_from_its_sidebar() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    h.get_by_label_contains("CDs, Holding");
    click_tile(&mut h, "Hold,");
    shot(&mut h, "hold");
    h.get_by_label("Let 17 through").click();
    h.run_steps(2);
    until(&mut h, "the end", |a| a.m.snapshot(0).finished);
    assert_eq!(snap(&h).steps["lib1"].count(ItemState::Ok), 17);
    h.get_by_label_contains("CDs, Done");
}

#[test]
fn a_bad_track_is_retried_from_its_row() {
    let e = env();
    let mut h = window(&e, Some(9), true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "a bad track", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 1 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 16);
    click_tile(&mut h, "Rip,");
    shot(&mut h, "inside");
    // Its title in full, its retries in the row.
    assert!(!app(&h).cut_titles.contains(&"Broken Bricks".to_owned()), "cut: {:?}", app(&h).cut_titles);
    // Its choices once, all on its row and none under a More; the
    // header keeps to what's wrong.
    for b in ["Read again", "Best of 3 reads", "Try the other drive", "Read again slowly", "Keep it anyway", "Skip this track"] {
        h.get_by_label(&format!("{b}, Broken Bricks"));
        assert!(h.query_by_label(b).is_none(), "{b} said twice");
    }
    assert!(h.query_by_label("More, Broken Bricks").is_none());
    h.get_by_label("Skip this track, Broken Bricks").click();
    h.run_steps(2);
    until(&mut h, "the track left out", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Skipped) == 1);
    let rip = &snap(&h).steps["rip"];
    assert_eq!(rip.count(ItemState::Bad), 0);
    assert_eq!(rip.items.iter().find(|i| i.id == "track-9").unwrap().state, ItemState::Skipped);
    // Done, its strips say so: the track left out outlined on Rip's; the
    // encoder's 16, none still waiting for a track that never comes.
    until(&mut h, "FLAC done", |a| a.m.snapshot(0).steps["flac"].state == StepState::Ok);
    let all = cards(&mut h);
    let get = |id: &str| &all.iter().find(|(x, _)| x == id).unwrap().1;
    use drt_app::card::Cell;
    assert_eq!(get("rip").cells[8], Cell::Out);
    assert_eq!(get("flac").cells, vec![Cell::Done; 16]);
}

#[test]
fn a_parameter_is_edited_in_its_sidebar_and_saved() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click_tile(&mut h, "MP3,");
    assert!(matches!(&app(&h).tray, Some(Tray::Step { id, .. }) if id == "mp3"));
    // A default says nothing; one you chose says so.
    assert!(h.query_by_label("default").is_none());
    click(&mut h, "Stereo: Full");
    assert_eq!(param(&saved_recipes(&e)[0], "mp3", "stereo").unwrap().resolve(None), Some(Value::from("Full")));
    h.get_by_label_contains("yours");
    // A folder: typed, then picked in the (stand-in) portal.
    click_tile(&mut h, "Library,");
    shot(&mut h, "params");
    h.get_by_label("Choose folder").click();
    let want = e.dir.path().join("picked").display().to_string();
    until(&mut h, "the picked folder", |a| {
        a.m.recipes[0].root.clone().find_mut("lib1").is_some_and(|n| matches!(n, Node::Step { params, .. } if params.get("folder").and_then(|s| s.resolve(None)) == Some(Value::from(want.clone()))))
    });
    assert_eq!(param(&saved_recipes(&e)[0], "lib1", "folder").unwrap().resolve(None), Some(Value::from(want.clone())));
    h.run_steps(2);
    assert!(text_boxes(&h).contains(&want), "the box shows the picked folder: {:?}", text_boxes(&h));
    click(&mut h, "Open folder");
    assert_eq!(e.opened.lock().unwrap().len(), 1, "the folder link opened it");
}

#[test]
fn a_new_recipe_asks_which_drive() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // From a recipe whose drive step isn't called `drive`.
    h.get_by_label_contains("Archive everything, Ready").click();
    h.run_steps(2);
    click(&mut h, "New recipe");
    h.event(egui::Event::Text("Box set".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let n = recipes::all().len();
    assert_eq!(app(&h).m.recipes.len(), n + 1);
    assert_eq!(app(&h).sel, n);
    assert!(app(&h).editing, "a new recipe opens for editing");
    h.get_by_label_contains("Drive, Drive?");
    shot(&mut h, "new");
    pick_drive(&mut h, recipes::DVD);
    let saved = saved_recipes(&e);
    assert_eq!(saved[n].name, "Box set");
    assert_eq!(param(&saved[n], "drive", "drive").unwrap().resolve(None), Some(Value::from(recipes::DVD)));
    // The offset follows the drive picked (AccurateRip's, so as text), and
    // switching never writes one drive's offset onto the other.
    h.get_by_label("+6 · AccurateRip");
    pick_drive(&mut h, recipes::BLU_RAY);
    h.get_by_label("+103 · AccurateRip");
    let s = app(&h).m.engine.settings.lock().unwrap().clone();
    assert_eq!(s.drive(recipes::DVD, "offset").provenance(), "AccurateRip");
    assert_eq!(s.drive(recipes::BLU_RAY, "offset").resolve(None), Some(Value::from(103)));
    click(&mut h, "Save");
    h.get_by_label_contains("Box set, Ready");
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
    // And it's all there after a restart.
    drop(h);
    let h = window(&e, None, false, 0.0);
    assert_eq!(app(&h).m.recipes[n].name, "Box set");
    assert_eq!(app(&h).m.drive(n), recipes::BLU_RAY);
}

fn text_boxes(h: &Harness<Window>) -> Vec<String> {
    h.get_all_by_role(Role::TextInput).filter_map(|n| n.accesskit_node().value().map(|v| v.to_string())).collect()
}

/// The CD recipe with no folder for its MP3 library, as recipes.json.
fn seed_without_folder(e: &Env) {
    let mut r = recipes::cd();
    if let Some(Node::Step { params, .. }) = r.root.find_mut("lib2") {
        params.remove("folder");
    }
    let dir = e.dir.path().join("config");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("recipes.json"), serde_json::to_string(&vec![r]).unwrap()).unwrap();
}

#[test]
fn a_missing_folder_is_set_on_its_tile_before_the_start() {
    let e = env();
    seed_without_folder(&e);
    let mut h = window(&e, None, true, 0.0);
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    h.get_by_label_contains("CDs, Folder?");
    h.get_by_label("1 to set");
    click_tile(&mut h, "Library, Folder?");
    assert!(matches!(&app(&h).tray, Some(Tray::Step { id, focus: Some(f) }) if id == "lib2" && f == "folder"));
    h.run_steps(2);
    shot(&mut h, "setup");
    h.event(egui::Event::Text("~/Music/MP3".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
    assert_eq!(param(&saved_recipes(&e)[0], "lib2", "folder").unwrap().resolve(None), Some(Value::from("~/Music/MP3")));
    // Set: no longer asked for. (What the built-in recipe has, so not
    // marked yours.)
    assert!(h.query_by_label_contains("not set").is_none());
    // Saved, so a restart keeps it.
    drop(h);
    let h = window(&e, None, true, 0.0);
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
    h.get_by_label_contains("CDs, Ready");
}

#[test]
fn an_unknown_offset_is_looked_up_at_once() {
    let e = env();
    let mut h = window(&e, None, false, 0.0);
    until(&mut h, "the offset", |a| a.m.engine.settings.lock().unwrap().drive(recipes::BLU_RAY, "offset").resolve(None).is_some());
    // Saved, with where it came from; nothing to set, Start open.
    let s: Settings = serde_json::from_str(&std::fs::read_to_string(e.dir.path().join("config/settings.json")).unwrap()).unwrap();
    assert_eq!(s.drive(recipes::BLU_RAY, "offset").resolve(None), Some(Value::from(103)));
    assert_eq!(s.drive(recipes::DVD, "offset").resolve(None), Some(Value::from(6)), "every recipe's drive");
    h.get_by_label_contains("CDs, Ready");
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
    click_tile(&mut h, "Drive,");
    h.get_by_label_contains("AccurateRip");
}

/// The sim Drive, with a lookup that takes a while, or that fails unless
/// it may measure.
struct TestDrive {
    inner: std::sync::Arc<dyn drt_engine::Step>,
    /// The lookup waits until this is set.
    release: Arc<std::sync::atomic::AtomicBool>,
    only_by_measuring: bool,
}

#[async_trait::async_trait]
impl drt_engine::Step for TestDrive {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        self.inner.run(inlet, outlet, cx).await
    }
    async fn find(&self, key: &str, drive: &str, measure: bool) -> Option<(Value, String)> {
        if self.only_by_measuring && !measure {
            return None;
        }
        while !self.release.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.inner.find(key, drive, measure).await.map(|(v, _)| (v, if measure { "measured".into() } else { "AccurateRip".into() }))
    }
}

fn registry_with(drive: TestDrive) -> drt_engine::Registry {
    let mut reg = simulated(None);
    reg.add(drive);
    reg
}

#[test]
fn while_the_offset_is_looked_up_the_drive_tile_is_busy() {
    let e = env();
    let inner = simulated(None).get("drive").unwrap().0;
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reg = registry_with(TestDrive { inner, release: release.clone(), only_by_measuring: false });
    let mut h = window_on(&e, reg, false, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    h.get_by_label_contains("Drive, Looking up");
    h.get_by_label_contains("CDs, Looking up");
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    shot(&mut h, "offset-lookup");
    release.store(true, std::sync::atomic::Ordering::SeqCst);
    until(&mut h, "the lookup", |a| a.m.finding.is_empty());
    h.get_by_label_contains("CDs, Ready");
}

#[test]
fn an_offset_not_in_any_list_is_asked_for_and_measured() {
    let e = env();
    let inner = simulated(None).get("drive").unwrap().0;
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reg = registry_with(TestDrive { inner, release: release.clone(), only_by_measuring: true });
    let mut h = window_on(&e, reg, false, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    until(&mut h, "the lookup to fail", |a| a.m.finding.is_empty());
    // Amber, asking, with Measure.
    h.get_by_label_contains("Drive, Read offset?");
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    click_tile(&mut h, "Drive, Read offset?");
    h.get_by_label("not set");
    shot(&mut h, "offset-measure");
    click(&mut h, "Measure offset");
    assert_eq!(app(&h).m.finding.get(&0), Some(&true));
    h.get_by_label("Measuring…");
    release.store(true, std::sync::atomic::Ordering::SeqCst);
    until(&mut h, "the measure", |a| a.m.finding.is_empty());
    h.get_by_label_contains("measured");
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
    let s: Settings = serde_json::from_str(&std::fs::read_to_string(e.dir.path().join("config/settings.json")).unwrap()).unwrap();
    assert_eq!(s.drive(recipes::BLU_RAY, "offset").provenance(), "measured");
}

#[test]
fn next_disc_leads_back_to_the_drive_on_the_canvas() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let lay = app(&h).layout(0);
    assert_eq!(lay.loops.len(), 1, "one line back, from Next disc");
    let next = lay.tile("next").unwrap().rect;
    assert_eq!(lay.loops[0][0], (next.x + next.w / 2.0, next.y + next.h));
    click(&mut h, "Fit to window");
    shot(&mut h, "loop");
}

/// The sim Hold, but it takes nothing in until `open` is set: so a test
/// can have the next disc start before the first disc's Hold waits for you.
struct LatchedHold {
    inner: std::sync::Arc<dyn drt_engine::Step>,
    open: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl drt_engine::Step for LatchedHold {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        while !self.open.load(std::sync::atomic::Ordering::SeqCst) {
            cx.sleep(5).await?;
        }
        self.inner.run(inlet, outlet, cx).await
    }
}

/// The sim Folder, but the one filing into `folder` takes nothing in until
/// `open` is set.
struct LatchedFolder {
    inner: std::sync::Arc<dyn drt_engine::Step>,
    folder: &'static str,
    open: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl drt_engine::Step for LatchedFolder {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        while cx.text("folder") == self.folder && !self.open.load(std::sync::atomic::Ordering::SeqCst) {
            cx.sleep(5).await?;
        }
        self.inner.run(inlet, outlet, cx).await
    }
}

/// The sim steps with the Hold latched shut, and the latch.
fn latched_hold(mut reg: drt_engine::Registry) -> (drt_engine::Registry, Arc<std::sync::atomic::AtomicBool>) {
    let open = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let inner = simulated(None).get("hold").unwrap().0;
    reg.add(LatchedHold { inner, open: open.clone() });
    (reg, open)
}

fn open_latch(open: &std::sync::atomic::AtomicBool) {
    open.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Runs to the hold of the newest CDs disc and answers its question.
fn to_the_hold(h: &mut Harness<Window>) {
    until(h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(h, "Use this one");
}

#[test]
fn next_disc_starts_the_next_and_the_earlier_waits_in_the_list() {
    let e = env();
    // The first disc's Hold fills only once the second has started: one
    // full before would hold the second back.
    let (reg, open) = latched_hold(simulated(None));
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    app_mut(&mut h).m.repeat_limit = Some(1);
    click(&mut h, "Start");
    to_the_hold(&mut h);
    until(&mut h, "the second disc", |a| a.m.runs.get(&0).is_some_and(|v| v.len() == 2));
    open_latch(&open);
    // The canvas follows the new disc; the first holds at its Library.
    let first = app(&h).m.runs[&0][0].id;
    until(&mut h, "the first to hold", |a| a.m.runs[&0][0].snap.steps["hold"].detail == "All here");
    until(&mut h, "the second's question", |a| a.m.snapshot(0).question.is_some());
    assert_ne!(app(&h).m.shown_run(0).unwrap().id, first);
    h.get_by_label_contains("The White Stripes, Holding");
    shot(&mut h, "next-disc");
    // Its canvas, its held tile, let through: it finishes and leaves.
    h.get_by_label_contains("The White Stripes, Holding").click();
    h.run_steps(2);
    assert_eq!(app(&h).m.shown_run(0).unwrap().id, first);
    click_tile(&mut h, "Hold,");
    h.get_by_label("Let 17 through").click();
    h.run_steps(2);
    until(&mut h, "the first disc to finish", |a| a.m.earlier(0).is_empty());
    assert!(h.query_by_label_contains("The White Stripes, Holding").is_none());
    assert_ne!(app(&h).m.shown_run(0).unwrap().id, first, "back to the newest");
}

/// The sim CDs recipe whose Review finds two names taken, and whose next
/// disc waits for `disc`. Runs to Review's question.
fn to_the_review_question(e: &Env) -> (Harness<'static, Window>, Arc<std::sync::atomic::AtomicBool>) {
    let inner = simulated(None).get("drive").unwrap().0;
    let disc = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut reg = simulated(None);
    reg.add(WaitingDrive { inner, runs: Default::default(), disc: disc.clone() });
    let mut h = window_on(e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    app_mut(&mut h).m.repeat_limit = Some(1);
    review_has_two(&mut h, e);
    click(&mut h, "Start");
    to_the_hold(&mut h);
    until(&mut h, "Review's question", |a| matches!(&a.tray, Some(Tray::Question { step }) if step == "review"));
    until(&mut h, "Next disc", |a| a.m.runs[&0][0].snap.steps["next"].state == StepState::Ok);
    h.run_steps(5);
    (h, disc)
}

#[test]
fn no_next_disc_while_the_last_one_asks() {
    let e = env();
    let (mut h, disc) = to_the_review_question(&e);
    // Next disc opened the tray, but the next disc doesn't start (or, if
    // it had, it's put back) while the first asks.
    assert_eq!(app(&h).m.runs[&0].len(), 1);
    let why = "Answer the question on the last disc first";
    assert_eq!(app(&h).m.held_back(0).as_deref(), Some(why));
    h.get_by_label_contains(&format!("CDs, {why}"));
    assert!(h.query_all_by_label(why).count() >= 1, "the header says so too");
    shot(&mut h, "next-disc-held-back");
    // Answered from the sidebar it's in: the next disc follows once the
    // files are let through too.
    click(&mut h, "Replace them");
    until(&mut h, "the Hold to wait for you", |a| a.m.held_back(0).as_deref() == Some("Let the files on the last disc through first"));
    assert_eq!(app(&h).m.runs[&0].len(), 1);
    assert_eq!(snap(&h).steps["review"].count(ItemState::Ok), 17, "the first disc's answer went to it");
    click_tile(&mut h, "Hold,");
    h.get_by_label("Let 17 through").click();
    h.run_steps(2);
    until(&mut h, "the second disc", |a| a.m.runs[&0].len() == 2);
    assert!(app(&h).m.held_back(0).is_none());
    disc.store(true, std::sync::atomic::Ordering::SeqCst);
    until(&mut h, "the second disc in", |a| a.m.newest(0).is_some_and(|r| r.snap.steps["drive"].state == StepState::Ok));
}

#[test]
fn an_earlier_disc_holding_holds_the_third_back_until_let_through_from_its_row() {
    let e = env();
    let (reg, open) = latched_hold(simulated(None));
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    app_mut(&mut h).m.repeat_limit = Some(2);
    click(&mut h, "Start");
    to_the_hold(&mut h);
    until(&mut h, "the second disc", |a| a.m.runs.get(&0).is_some_and(|v| v.len() == 2));
    // The second asks which release: while it does, the first's Hold fills.
    until(&mut h, "the second's question", |a| a.m.newest(0).is_some_and(|r| r.snap.question.is_some()));
    let second = app(&h).m.newest(0).unwrap().id;
    open_latch(&open);
    until(&mut h, "the first to hold", |a| a.m.runs[&0][0].snap.steps["hold"].detail == "All here");
    to_the_hold(&mut h);
    until(&mut h, "both to hold, the second through Next disc", |a| {
        let v = &a.m.runs[&0];
        v.len() == 2 && v.iter().all(|r| r.snap.steps["hold"].detail == "All here") && v[1].snap.steps["next"].state == StepState::Ok
    });
    h.run_steps(5);
    assert_eq!(app(&h).m.newest(0).unwrap().id, second, "no third disc");
    assert_eq!(app(&h).m.held_back(0).as_deref(), Some("Let the files on the last disc through first"));
    // The second let through: the first still holds.
    click_tile(&mut h, "Hold,");
    h.get_by_label("Let 17 through").click();
    h.run_steps(2);
    until(&mut h, "the earlier disc to be what waits", |a| a.m.held_back(0).as_deref() == Some("Let the files on an earlier disc through first"));
    assert_eq!(app(&h).m.newest(0).unwrap().id, second, "still no third disc");
    h.get_by_label_contains("CDs, Let the files on an earlier disc through first");
    // From its row in the list, its Hold, let through: the third starts.
    h.get_by_label_contains("The White Stripes, Holding").click();
    h.run_steps(2);
    assert_ne!(app(&h).m.shown_run(0).unwrap().id, second);
    click_tile(&mut h, "Hold,");
    h.get_by_label("Let 17 through").click();
    h.run_steps(2);
    until(&mut h, "the third disc", |a| a.m.newest(0).is_some_and(|r| r.id > second));
}

#[test]
fn stop_while_held_back_calls_only_the_next_disc_off() {
    let e = env();
    let (mut h, _disc) = to_the_review_question(&e);
    assert!(app(&h).m.next_waits(0));
    click(&mut h, "Stop");
    assert!(!app(&h).m.next_waits(0));
    // The first disc goes on: answered, let through, filed; no second.
    click(&mut h, "Keep both");
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    click_tile(&mut h, "Hold,");
    h.get_by_label("Let 17 through").click();
    h.run_steps(2);
    until(&mut h, "the first to finish", |a| a.m.snapshot(0).finished);
    h.run_steps(5);
    assert_eq!(app(&h).m.runs[&0].len(), 1);
    let s = snap(&h);
    assert!(!s.stopped);
    assert_eq!(s.steps["lib1"].count(ItemState::Ok), 17);
    h.get_by_label_contains("CDs, Done");
}

#[test]
fn stop_means_no_next_disc() {
    let e = env();
    let mut h = window(&e, None, true, 0.03);
    app_mut(&mut h).m.repeat_limit = Some(1);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Stop");
    until(&mut h, "the stop", |a| a.m.snapshot(0).finished);
    h.run_steps(5);
    assert_eq!(app(&h).m.runs[&0].len(), 1);
    h.get_by_label_contains("CDs, Stopped");
}

#[test]
fn a_recipe_without_next_disc_does_one_disc() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    app_mut(&mut h).m.repeat_limit = None;
    h.get_by_label_contains("Archive everything").click();
    h.run_steps(2);
    assert!(app(&h).layout(1).loops.is_empty());
    click(&mut h, "Start");
    // Answer and approve as it goes, until it's done.
    let start = Instant::now();
    while !app(&h).m.snapshot(1).finished {
        assert!(start.elapsed() < Duration::from_secs(30), "the archive never finished");
        let s = app(&h).m.snapshot(1);
        if let Some((step, _)) = &s.question {
            app(&h).m.send(1, drt_engine::Command::Answer { step: step.clone(), answer: drt_engine::Answer::Pick(0) });
        }
        for (id, v) in &s.steps {
            if v.state == StepState::Hold && v.detail == "All here" {
                app(&h).m.send(1, drt_engine::Command::Approve { step: id.clone() });
            }
        }
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run_steps(5);
    assert_eq!(app(&h).m.runs[&1].len(), 1, "no second disc");
    h.get_by_label_contains("Archive everything, Done");
}

#[test]
fn each_recipe_pans_and_zooms_on_its_own() {
    use egui::{vec2, Modifiers};
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let canvas = egui::pos2(700.0, 450.0); // empty canvas, below the CD recipe
    let view = |h: &Harness<Window>| {
        let a = app(h);
        let v = a.views_of(a.sel);
        (v.zoom, v.pan)
    };
    let (z0, p0) = view(&h);
    // Drag the canvas.
    h.hover_at(canvas);
    h.step();
    h.drag_at(canvas);
    h.step();
    for k in 1..=5 {
        h.hover_at(canvas + vec2(12.0, 5.0) * k as f32);
        h.step();
    }
    h.drop_at(canvas + vec2(60.0, 25.0));
    h.run_steps(2);
    assert!(((view(&h).1 - p0) - vec2(60.0, 25.0)).length() < 1.5, "drag: {:?}", view(&h).1 - p0);
    // Ctrl+wheel zooms about the pointer, text and all.
    let wheel = |h: &mut Harness<Window>, delta: egui::Vec2, m: Modifiers| {
        h.hover_at(canvas);
        h.step();
        let ev = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Unknown, modifiers: m };
        if m.is_none() {
            h.event(ev)
        } else {
            h.event_modifiers(ev, m)
        }
        h.run_steps(40);
    };
    // A plain scroll zooms about the pointer: what's under it stays put.
    let before = view(&h);
    wheel(&mut h, vec2(0.0, -40.0), Modifiers::NONE);
    let after = view(&h);
    assert!(after.0 < before.0 * 0.95, "scrolling down zooms out: {} -> {}", before.0, after.0);
    let canvas_top = app(&h).canvas_rect().top();
    let under = |v: (f32, egui::Vec2)| (canvas - egui::pos2(232.0, canvas_top) - v.1) / v.0;
    assert!((under(before) - under(after)).length() < 0.5, "about the pointer");
    wheel(&mut h, vec2(0.0, 80.0), Modifiers::NONE);
    assert!(view(&h).0 > after.0 * 1.2, "scrolling up zooms in");
    wheel(&mut h, vec2(0.0, 60.0), Modifiers::CTRL);
    let cds = view(&h);
    assert!(cds.0 > z0 * 1.05, "ctrl+wheel zooms too: {}", cds.0);
    // The limits hold.
    for _ in 0..30 {
        wheel(&mut h, vec2(0.0, -200.0), Modifiers::NONE);
    }
    assert!((view(&h).0 - 0.2).abs() < 1e-3, "no further out than 20%: {}", view(&h).0);
    for _ in 0..30 {
        wheel(&mut h, vec2(0.0, 200.0), Modifiers::NONE);
    }
    assert!((view(&h).0 - 3.0).abs() < 1e-3, "no further in than 300%: {}", view(&h).0);
    app_mut(&mut h).set_view(0, cds);
    h.run_steps(2);
    // Another recipe has its own view; coming back keeps this one.
    h.get_by_label_contains("Archive everything, Ready").click();
    h.run_steps(2);
    assert_eq!(view(&h).0, 1.0);
    click(&mut h, "Fit to window");
    let arch = view(&h).0;
    assert!(arch < 0.6, "the archive fits at under 60%: {arch}");
    h.get_by_label_contains("CDs, Ready").click();
    h.run_steps(2);
    assert_eq!(view(&h), cds);
}

#[test]
fn two_recipes_run_at_once_but_never_two_on_one_drive() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the CDs question", |a| a.m.snapshot(0).question.is_some());
    h.get_by_label_contains("Archive everything").click();
    h.run_steps(2);
    click(&mut h, "Start");
    until(&mut h, "the archive question", |a| a.m.snapshot(1).question.is_some());
    assert!(app(&h).m.running(0) && app(&h).m.running(1));
    h.get_by_label_contains("CDs, Which album is this?");
    h.get_by_label_contains("Archive everything, Which album is this?");
    // A new recipe on the CDs drive can't start while CDs runs.
    let i = app_mut(&mut h).m.new_recipe("Same drive");
    app_mut(&mut h).m.set_param(i, "drive", "drive", recipes::BLU_RAY.into()).unwrap();
    assert_eq!(app(&h).m.cant_start(i).as_deref(), Some("The drive is busy with CDs"));
}

#[test]
fn a_running_recipe_updates_live() {
    let e = env();
    let mut h = window(&e, None, true, 0.03);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "ripping", |a| {
        let s = a.m.snapshot(0);
        s.steps["rip"].count(ItemState::Ok) >= 6 && s.steps["flac"].state == StepState::Busy
    });
    h.get_by_label_contains("CDs, ");
    assert!(h.get_by_label("Edit").accesskit_node().is_disabled(), "no editing while it runs");
    shot(&mut h, "running");
    click(&mut h, "Stop");
    until(&mut h, "the stop", |a| a.m.snapshot(0).finished);
    h.get_by_label_contains("CDs, Stopped");
}

/// The sidebar, every kind, at several window sizes and display scales:
/// on the very first frame it's drawn it must be fully open, from under
/// the header to the window's bottom at its right edge, with its close
/// button inside it; the canvas keeps its full height.
#[test]
fn the_sidebar_opens_fully_on_its_first_frame_at_any_size_and_scale() {
    let sizes = [egui::vec2(1280.0, 720.0), egui::vec2(1920.0, 1080.0), egui::vec2(1100.0, 800.0), egui::vec2(1024.0, 576.0), egui::vec2(800.0, 600.0)];
    let mut failures = Vec::new();
    for size in sizes {
        for ppp in [1.0, 1.5, 2.0] {
            for kind in ["question", "step", "setup", "switch"] {
                let e = env();
                if kind == "setup" {
                    seed_without_folder(&e);
                }
                let mut h = window_at(&e, if kind == "switch" { Some(9) } else { None }, true, 0.0, size, ppp);
                // Open it, and stop at the first frame that draws it.
                match kind {
                    "question" => {
                        h.get_by_label("Start").click();
                        let start = Instant::now();
                        while app(&h).tray.is_none() {
                            assert!(start.elapsed() < Duration::from_secs(20));
                            h.step();
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                    "step" => {
                        h.get_all_by_role(Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("MP3,"))).unwrap().click_accesskit();
                        h.step();
                    }
                    "setup" => {
                        h.get_by_label_contains("Library, Folder?").click_accesskit();
                        h.step();
                    }
                    _ => {
                        // From one step straight to another.
                        h.get_all_by_role(Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("MP3,"))).unwrap().click_accesskit();
                        h.run_steps(3);
                        h.get_by_label_contains("Drive,").click_accesskit();
                        h.step();
                    }
                }
                let img = h.render().unwrap();
                let a = app(&h);
                let d = a.side_rect().expect("open");
                let c = a.canvas_rect();
                let px = |x: f32, y: f32| *img.get_pixel(((x * ppp) as u32).min(img.width() - 1), ((y * ppp) as u32).min(img.height() - 1));
                let mut why = vec![];
                if (d.bottom() - size.y).abs() > 1.0 || (d.right() - size.x).abs() > 1.0 {
                    why.push(format!("ends at {:?}, not the window's corner {size:?}", d.max));
                }
                if (c.bottom() - size.y).abs() > 1.0 || (c.right() - d.left()).abs() > 1.0 {
                    why.push(format!("canvas {c:?} beside {d:?}"));
                }
                let want = (size.x * 0.35).clamp(440.0, 680.0).min(size.x - 232.0 - 320.0).max(360.0);
                if kind != "question" && (d.width() - want).abs() > 1.0 {
                    why.push(format!("{} wide, not {want}", d.width()));
                }
                // Drawn where it says, this frame: its fill near its foot is
                // the list's (both are the surface).
                if px(d.left() + 8.0, d.top() + 6.0) != px(100.0, size.y - 4.0) {
                    why.push(format!("not drawn at {d:?}: {:?}", px(d.left() + 8.0, d.top() + 6.0)));
                }
                let close = h.get_by_label("Close").rect();
                if !d.contains_rect(close) {
                    why.push(format!("close at {close:?} outside {d:?}"));
                }
                if !why.is_empty() {
                    let name = format!("side-{kind}-{}x{}@{ppp}", size.x, size.y);
                    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("shots/failed");
                    std::fs::create_dir_all(&dir).unwrap();
                    img.save(dir.join(format!("{name}.png"))).unwrap();
                    failures.push(format!("{name}: {}", why.join("; ")));
                } else if ppp == 2.0 && size.x == 1920.0 {
                    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("shots");
                    img.save(dir.join(format!("scale-{kind}-1920x1080@2.png"))).unwrap();
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_scroll_over_the_sidebar_scrolls_it_not_the_canvas() {
    let e = env();
    // Short enough that a 17-track list has to scroll.
    let mut h = window_at(&e, None, true, 0.0, egui::vec2(1280.0, 560.0), 1.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    click_tile(&mut h, "Hold,");
    let view = app(&h).views_of(0);
    let y = |h: &Harness<Window>| h.get_by_label_contains("1, Jimmy the Exploder").rect().top();
    let before = y(&h);
    let over = app(&h).side_rect().unwrap().center() + egui::vec2(0.0, 40.0);
    h.hover_at(over);
    h.step();
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -60.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(40);
    assert!(y(&h) < before - 20.0, "the list scrolled: {before} -> {}", y(&h));
    assert_eq!(app(&h).views_of(0), view, "the canvas didn't zoom or move");
    // The last row can be reached.
    for _ in 0..8 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -200.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(20);
    }
    let last = h.get_by_label_contains("9, Broken Bricks").rect();
    assert!(last.bottom() <= app(&h).side_rect().unwrap().bottom(), "track 9 scrolled into the sidebar: {last:?}");
}

/// Under the real steps (never started here), a recipe using steps they
/// don't have shows those as muted tiles, can't start, and is mended by
/// removing them.
#[test]
fn steps_that_arent_available_are_shown_muted_and_can_be_removed() {
    let e = env();
    let size = egui::vec2(1280.0, 720.0);
    let h = window_on(&e, drt_steps::real(), true, 1.0, size, 1.0);
    let names: Vec<String> = saved_recipes(&e).into_iter().map(|r| r.name).collect();
    let real = drt_steps::real();
    let runnable: Vec<String> = recipes::all().into_iter().filter(|r| drt_engine::check(&r.root, &real).is_ok()).map(|r| r.name).collect();
    assert_eq!(names, runnable, "only the recipes these steps can run are seeded");
    drop(h);
    // A recipes.json with a step no build has (here GnuDB, renamed) is
    // used as it is.
    let mut all = recipes::all();
    for r in &mut all {
        if let Some(Node::Step { step, .. }) = r.root.find_mut("a_gn") {
            *step = "gnudb-gone".into();
        }
    }
    std::fs::write(e.dir.path().join("config/recipes.json"), serde_json::to_string(&all).unwrap()).unwrap();
    let mut h = window_on(&e, simulated(None), true, 1.0, size, 1.0);
    h.get_by_label_contains("Archive everything, Not available").click();
    h.run_steps(2);
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    h.get_by_label("Not available");
    assert!(h.query_by_label_contains("no step called").is_none(), "no raw check errors");
    click_tile(&mut h, "Gnudb-gone, Not available");
    shot(&mut h, "step-unavailable");
    // Remove, without going into Edit: it says what goes, then goes.
    click(&mut h, "Remove");
    h.get_by_label("Remove 1?");
    click(&mut h, "Remove");
    let arch = saved_recipes(&e).into_iter().find(|r| r.name == "Archive everything").unwrap();
    assert!(!arch.root.ids().contains(&"a_gn"));
    assert!(arch.root.ids().contains(&"a_dg"), "only that one went");
}

fn ids(r: &Recipe) -> Vec<String> {
    r.root.ids().into_iter().map(str::to_owned).collect()
}

#[test]
fn a_step_added_with_a_plus_is_a_draft_until_saved() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Edit");
    assert!(h.get_by_label("Undo").accesskit_node().is_disabled());
    click(&mut h, "Add after Audio CD");
    // A lookup reads the disc, so nothing goes after one: no + there.
    assert!(h.query_by_label("Add after MusicBrainz").is_none());
    // After the lookups' fork, a new lookup is a branch beside them.
    assert!(app(&h).popover.as_ref().is_some_and(|p| p.branch));
    // Only what fits: lookups, no encoders.
    h.get_by_label("Discogs");
    assert!(h.query_by_label("FLAC").is_none());
    click(&mut h, "Discogs");
    // Nothing to set on it: no sidebar, just the new tile, selected.
    assert!(app(&h).popover.is_none() && app(&h).tray.is_none());
    assert_eq!(app(&h).selected.as_deref(), Some("discogs"));
    let draft = ids(&app(&h).m.recipes[0]);
    assert!(draft.contains(&"discogs".to_owned()));
    drt_engine::check(&app(&h).m.recipes[0].root, &app(&h).m.engine.registry).unwrap();
    assert!(!ids(&saved_recipes(&e)[0]).contains(&"discogs".into()), "not saved yet");
    assert_eq!(app(&h).m.cant_start(0).as_deref(), Some("Save or cancel your edits first"));
    shot(&mut h, "edit-header");
    click(&mut h, "Save");
    assert!(ids(&saved_recipes(&e)[0]).contains(&"discogs".into()));
    assert!(!app(&h).editing);
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
}

#[test]
fn undo_redo_and_cancel_while_editing() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let original = ids(&saved_recipes(&e)[0]);
    click(&mut h, "Edit");
    click(&mut h, "Add after Audio CD");
    click(&mut h, "Discogs");
    assert!(ids(&app(&h).m.recipes[0]).contains(&"discogs".into()));
    click(&mut h, "Undo");
    assert_eq!(ids(&app(&h).m.recipes[0]), original);
    click(&mut h, "Redo");
    assert!(ids(&app(&h).m.recipes[0]).contains(&"discogs".into()));
    // The keys too.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(ids(&app(&h).m.recipes[0]), original);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z);
    h.run_steps(2);
    assert!(ids(&app(&h).m.recipes[0]).contains(&"discogs".into()));
    // Cancel drops the draft; nothing reached the file.
    click(&mut h, "Cancel");
    assert_eq!(ids(&app(&h).m.recipes[0]), original);
    assert_eq!(ids(&saved_recipes(&e)[0]), original);
    assert!(!app(&h).editing);
}

#[test]
fn leaving_unsaved_edits_asks_once_in_the_header() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Edit");
    click(&mut h, "Add after Audio CD");
    click(&mut h, "Discogs");
    h.get_by_label_contains("Archive everything").click();
    h.run_steps(2);
    assert_eq!(app(&h).sel, 0, "stays until answered");
    h.get_by_label("Save changes to CDs?");
    click(&mut h, "Save");
    assert_eq!(app(&h).sel, 1, "then goes");
    assert!(ids(&saved_recipes(&e)[0]).contains(&"discogs".into()));
}

#[test]
fn removing_a_step_takes_what_depends_on_it_after_a_confirm() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Edit");
    click(&mut h, "Fit to window");
    click_tile(&mut h, "Rip,");
    shot(&mut h, "step-edit");
    click(&mut h, "Remove");
    // Says what goes, and marks it.
    h.get_by_label("Remove 8?");
    h.get_by_label("Rip and the 7 after it.");
    let doomed = app(&h).confirm_remove.clone().unwrap().1;
    for id in ["rip", "flac", "review", "hold", "lib1", "mp3", "lib2", "next"] {
        assert!(doomed.contains(&id.to_owned()), "{id}");
    }
    shot(&mut h, "remove-confirm");
    click(&mut h, "Remove");
    let left = ids(&app(&h).m.recipes[0]);
    assert_eq!(left, ["drive", "cd", "mb", "cdtext", "choose", "art"]);
    drt_engine::check(&app(&h).m.recipes[0].root, &app(&h).m.engine.registry).expect("never left invalid");
    click(&mut h, "Save");
    assert_eq!(ids(&saved_recipes(&e)[0]), left);
    // Only the last step is kept.
    let i = app_mut(&mut h).m.new_recipe("One");
    assert!(app_mut(&mut h).m.remove_step(i, "drive").is_err());
    assert_eq!(app(&h).m.recipes[i].root.ids(), ["drive"]);
}

#[test]
fn a_new_branch_from_the_plus() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // Without Next disc, to add it back as a branch.
    app_mut(&mut h).m.begin_edit(0);
    app_mut(&mut h).m.remove_step(0, "next").unwrap();
    app_mut(&mut h).m.save_edit(0);
    click(&mut h, "Edit");
    h.get_by_label("Add after Rip").click_accesskit();
    h.run_steps(2);
    // The line splits after Rip: this + adds a branch; everything listed fits.
    assert!(app(&h).popover.as_ref().is_some_and(|p| p.branch));
    h.get_by_label("Next disc");
    h.get_by_label("FLAC");
    // The line splits after Rip, so this + adds a branch; no mode to pick.
    assert!(h.query_by_label("Adds: In line").is_none() && h.query_by_label("adds a branch").is_none());
    shot(&mut h, "picker-popover-branch");
    h.get_by_label("Next disc").click_accesskit();
    h.run_steps(2);
    let r = app(&h).m.recipes[0].clone();
    drt_engine::check(&r.root, &app(&h).m.engine.registry).unwrap();
    let Node::Chain { nodes } = &r.root else { panic!() };
    let Node::Branches { branches } = nodes.last().unwrap() else { panic!("{:?}", nodes.last()) };
    assert_eq!(branches.len(), 3, "beside FLAC and MP3");
    assert!(matches!(branches.last().unwrap(), Node::Step { step, .. } if step == "next-disc"));
    // Every option, either way, keeps the recipe valid.
    let m = &app(&h).m;
    for (info, inline, br) in m.placements_after(0, "rip") {
        for (ok, as_branch) in [(inline, false), (br, true)] {
            if ok {
                let mut t = r.root.clone();
                let n = Node::step("t", info.kind);
                assert!(if as_branch { t.add_branch_after("rip", n) } else { t.insert_after("rip", n) });
                drt_engine::check(&t, &m.engine.registry).unwrap_or_else(|e| panic!("{} offered but {e}", info.name));
            }
        }
    }
}

#[test]
fn a_busy_drive_shows_on_its_cards_and_tiles() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    // A new recipe: the CDs drive is busy, the other free.
    click(&mut h, "New recipe");
    h.event(egui::Event::Text("Box set".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let opts = drive_options(&mut h);
    let busy = |d: &str| opts.iter().find(|l| l.starts_with(d)).cloned().unwrap_or_else(|| panic!("no {d} in {opts:?}"));
    assert!(busy(recipes::BLU_RAY).ends_with(" · Busy"), "{}", busy(recipes::BLU_RAY));
    assert!(!busy(recipes::DVD).contains("Busy"));
    pick_drive(&mut h, recipes::BLU_RAY);
    shot(&mut h, "drive-busy");
    click(&mut h, "Save");
    // Its Drive tile waits, the header says why, Start is off.
    h.get_by_label_contains("Drive, Busy, CDs");
    h.get_by_label("Drive busy");
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    h.get_by_label_contains("Box set, Drive busy");
}

/// Cover art that finds a real image: a small PNG it writes, so the sidebar
/// can show it.
struct TestCover {
    inner: std::sync::Arc<dyn drt_engine::Step>,
    png: PathBuf,
}

#[async_trait::async_trait]
impl drt_engine::Step for TestCover {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let png = self.png.display().to_string();
            disc.context_mut().add(drt_engine::Subject::Disc, "cover", png, "Cover Art Archive");
            // Its own output, which its card shows.
            cx.made(&self.png);
            cx.status("240 × 240", "");
            outlet.send(disc).await;
        }
        Ok(())
    }
}

/// A cover like the album's: red, cream and black bands.
fn write_cover(path: &Path) {
    let img = image::RgbaImage::from_fn(240, 240, |x, y| {
        let f = (x + y) as f32 / 480.0;
        if f < 0.45 {
            image::Rgba([0xb8, 0x39, 0x2f, 255])
        } else if f < 0.7 {
            image::Rgba([0xf2, 0xef, 0xe9, 255])
        } else {
            image::Rgba([0x18, 0x20, 0x2b, 255])
        }
    });
    img.save(path).unwrap();
}

fn open_tile(h: &mut Harness<Window>, starts: &str) {
    app_mut(h).close_tray();
    h.run_steps(2);
    click_tile(h, starts);
    h.run_steps(3);
}

/// Every kind of sidebar, light and dark, as `shots/step-*.png`.
#[test]
fn every_kind_of_step_sidebar() {
    let e = env();
    let png = e.dir.path().join("cover.png");
    write_cover(&png);
    let mut reg = simulated(Some(9));
    let inner = reg.get("cover-art").unwrap().0;
    reg.add(TestCover { inner, png });
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    // Before the start.
    open_tile(&mut h, "Drive,");
    shot(&mut h, "step-drive");
    open_tile(&mut h, "Library,");
    shot(&mut h, "step-library-idle");
    open_tile(&mut h, "MP3,");
    shot(&mut h, "step-encoder-idle");
    open_tile(&mut h, "Next disc,");
    shot(&mut h, "step-next-disc");
    app_mut(&mut h).close_tray();
    click(&mut h, "Edit");
    open_tile(&mut h, "Rip,");
    click(&mut h, "Remove");
    shot(&mut h, "step-remove");
    click(&mut h, "Don't remove");
    click(&mut h, "Cancel");
    // Running: the question, then the lookups that fed it.
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    shot(&mut h, "step-choose");
    open_tile(&mut h, "Audio CD,");
    shot(&mut h, "step-audio-cd");
    open_tile(&mut h, "MusicBrainz,");
    shot(&mut h, "step-lookup");
    open_tile(&mut h, "CD-Text,");
    shot(&mut h, "step-cd-text");
    app_mut(&mut h).close_tray();
    let step = snap(&h).question.unwrap().0;
    app(&h).m.send(0, drt_engine::Command::Answer { step, answer: drt_engine::Answer::Pick(0) });
    until(&mut h, "a damaged track", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 1 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 16);
    open_tile(&mut h, "Rip,");
    shot(&mut h, "step-rip");
    open_tile(&mut h, "Choose,");
    shot(&mut h, "step-chosen");
    open_tile(&mut h, "Cover art,");
    until(&mut h, "the cover", |a| a.covers.values().any(|c| matches!(c, drt_app::tray::Cover::Ready(..))));
    shot(&mut h, "step-cover");
    open_tile(&mut h, "FLAC,");
    shot(&mut h, "step-encoder");
    // Keep the bad track, then the hold.
    app(&h).m.send(0, drt_engine::Command::Retry { step: "rip".into(), item: "track-9".into(), option: "keep".into() });
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    open_tile(&mut h, "Hold,");
    shot(&mut h, "step-hold");
    open_tile(&mut h, "Review,");
    shot(&mut h, "step-folder");
}

/// Cover art whose front came from another edition of the album, as the
/// real step says it.
struct TestEditionCover {
    inner: std::sync::Arc<dyn drt_engine::Step>,
    png: PathBuf,
}

#[async_trait::async_trait]
impl drt_engine::Step for TestEditionCover {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        use drt_steps::real::artwork;
        while let Some(mut disc) = inlet.recv().await {
            let e = artwork::Elsewhere {
                edition: artwork::Edition { id: "887fdba3-6599-4c00-80a5-ee30bcdde469".into(), ..Default::default() },
                size: (2813, 2805),
                words: "Polydor, Europe 1995".into(),
                measured: 4,
            };
            let c = disc.context_mut();
            c.add(drt_engine::Subject::Disc, "cover", self.png.display().to_string(), "Cover Art Archive");
            c.add(drt_engine::Subject::Disc, artwork::COVER_RELEASE, e.edition.id.clone(), "Cover Art Archive");
            c.add(drt_engine::Subject::Disc, artwork::COVER_FROM, e.words.clone(), "Cover Art Archive");
            cx.made(&self.png);
            let (status, detail) = drt_steps::real::cover_says(Some((1200, 1197)), Some(&e));
            cx.status(status, detail);
            outlet.send(disc).await;
        }
        Ok(())
    }
}

/// A front from another edition, said on Cover art's card; and every scan
/// saved beside the tracks, in Artwork files' sidebar.
#[test]
fn a_cover_from_another_edition_and_every_scan() {
    let e = env();
    let png = e.dir.path().join("cover.png");
    write_cover(&png);
    let mut reg = simulated(None);
    let inner = reg.get("cover-art").unwrap().0;
    reg.add(TestEditionCover { inner, png: png.clone() });
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    h.get_by_label_contains("Archive everything").click();
    h.run_steps(2);
    click(&mut h, "Start");
    let start = Instant::now();
    while !app(&h).m.snapshot(1).finished {
        assert!(start.elapsed() < Duration::from_secs(30), "the archive never finished");
        let s = app(&h).m.snapshot(1);
        if let Some((step, _)) = &s.question {
            app(&h).m.send(1, drt_engine::Command::Answer { step: step.clone(), answer: drt_engine::Answer::Pick(0) });
        }
        for (id, v) in &s.steps {
            if v.state == StepState::Hold && v.detail == "All here" {
                app(&h).m.send(1, drt_engine::Command::Approve { step: id.clone() });
            }
        }
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run_steps(5);
    let s = snap(&h);
    assert_eq!((s.steps["a_art"].status.as_str(), s.steps["a_art"].detail.as_str()), ("1200 × 1197", "From another edition"));
    assert_eq!((s.steps["a_scans"].status.as_str(), s.steps["a_scans"].detail.as_str()), ("6 pictures", "From this release"));
    let all = cards(&mut h);
    let art = &all.iter().find(|(x, _)| x == "a_art").unwrap().1;
    assert_eq!(art.detail, "From another edition", "whole, not cut short");
    open_tile(&mut h, "Cover art,");
    until(&mut h, "the cover", |a| a.covers.values().any(|c| matches!(c, drt_app::tray::Cover::Ready(..))));
    shot(&mut h, "step-cover-edition");
    // It opens at full size, from its link or the picture itself.
    click(&mut h, "Open full size");
    assert_eq!(e.opened.lock().unwrap().last(), Some(&png), "the cover opened");
    h.get_by_label("The cover").click();
    h.run_steps(2);
    assert_eq!(e.opened.lock().unwrap().len(), 2, "a click on the picture opens it too");
    // So does the cover on its card, rather than its sidebar.
    let (lay, v, c) = (app(&h).layout(1), app(&h).views_of(1), app(&h).canvas_rect());
    let t = lay.tile("a_art").unwrap().rect;
    let at = c.min + v.pan + egui::vec2(t.x + 64.0, t.y + 128.0) * v.zoom;
    assert!(c.contains(at), "the card's cover is in view");
    h.event(egui::Event::PointerMoved(at));
    h.step();
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(e.opened.lock().unwrap().len(), 3, "a click on the card's cover opens it");
    assert!(matches!(&app(&h).tray, Some(Tray::Step { id, .. }) if id == "a_art"), "and leaves the sidebar as it was");
    click(&mut h, "Settings");
    assert!(h.query_all_by_label_contains("Which cover").count() > 0);
    shot(&mut h, "step-cover-settings");
    open_tile(&mut h, "Artwork files,");
    shot(&mut h, "step-artwork-files");
    // Each scan opens from its row.
    click(&mut h, "Open, Booklet 01.jpg");
    let opened = e.opened.lock().unwrap().last().cloned().unwrap();
    assert!(opened.is_absolute() && opened.ends_with("Artwork/Booklet 01.jpg"), "the scan opened: {opened:?}");
}

/// Cover art set to let you pick: three fronts to choose from, as the real
/// step puts them.
struct TestPickCover {
    inner: std::sync::Arc<dyn drt_engine::Step>,
    pngs: Vec<PathBuf>,
}

#[async_trait::async_trait]
impl drt_engine::Step for TestPickCover {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        use drt_steps::real::artwork::{cover_question, Edition, Front};
        while let Some(mut disc) = inlet.recv().await {
            let front = |words: &str, size, own| Front { edition: Edition::default(), size, words: words.into(), own };
            let fronts = [front("This release", (491, 500), true), front("Polydor, Europe 1995", (2813, 2805), false), front("A&M Records 31452 3787 2, US 1995", (1902, 1858), false)];
            let thumbs: Vec<Option<PathBuf>> = self.pngs.iter().cloned().map(Some).collect();
            let answer = cx.ask(cover_question(&fronts, &thumbs, 1)).await?;
            let k = match answer {
                drt_engine::Answer::Pick(k) => k,
                _ => 1,
            };
            disc.context_mut().add(drt_engine::Subject::Disc, "cover", self.pngs[k].display().to_string(), "Cover Art Archive");
            cx.made(&self.pngs[k]);
            cx.status(format!("picked {}", fronts[k].words), "");
            outlet.send(disc).await;
        }
        Ok(())
    }
}

/// A cover in other colours, for a second and third front.
fn write_cover_in(path: &Path, a: [u8; 3], b: [u8; 3]) {
    let img = image::RgbaImage::from_fn(240, 240, |x, y| if (x / 40 + y / 40) % 2 == 0 { image::Rgba([a[0], a[1], a[2], 255]) } else { image::Rgba([b[0], b[1], b[2], 255]) });
    img.save(path).unwrap();
}

/// Let me pick the cover: the fronts as pictures side by side, the best
/// picked to start with; another is picked by clicking it.
#[test]
fn a_cover_is_picked_from_pictures() {
    let e = env();
    let pngs: Vec<PathBuf> = (0..3).map(|k| e.dir.path().join(format!("front-{k}.png"))).collect();
    write_cover(&pngs[0]);
    write_cover_in(&pngs[1], [0x2f, 0x5d, 0xb8], [0xf2, 0xef, 0xe9]);
    write_cover_in(&pngs[2], [0x18, 0x20, 0x2b], [0xe0, 0xa8, 0x30]);
    let mut reg = simulated(None);
    let inner = reg.get("cover-art").unwrap().0;
    reg.add(TestPickCover { inner, pngs: pngs.clone() });
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    click(&mut h, "Start");
    until(&mut h, "Which album?", |a| a.m.snapshot(0).question.as_ref().is_some_and(|(s, _)| s == "choose"));
    app(&h).m.send(0, drt_engine::Command::Answer { step: "choose".into(), answer: drt_engine::Answer::Pick(0) });
    until(&mut h, "Which cover?", |a| a.m.snapshot(0).question.as_ref().is_some_and(|(_, q)| q.prompt == "Which cover?"));
    let step = snap(&h).question.unwrap().0;
    app_mut(&mut h).open(Tray::Question { step: step.clone() });
    h.run_steps(3);
    until(&mut h, "the fronts", |a| a.covers.values().filter(|c| matches!(c, drt_app::tray::Cover::Ready(..))).count() >= 3);
    shot(&mut h, "question-cover");
    // The biggest is picked to start with.
    assert!(h.get_by_label("Polydor, Europe 1995, 2813 × 2805").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True));
    // Opening one at full size doesn't pick it.
    click(&mut h, "Open full size: A&M Records 31452 3787 2, US 1995");
    assert!(pngs.contains(e.opened.lock().unwrap().last().unwrap()), "a front opened");
    assert!(h.get_by_label("Polydor, Europe 1995, 2813 × 2805").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True));
    click(&mut h, "A&M Records 31452 3787 2, US 1995, 1902 × 1858");
    assert!(h.get_by_label("A&M Records 31452 3787 2, US 1995, 1902 × 1858").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True));
    h.get_by_label("Use this cover").click_accesskit();
    until(&mut h, "the cover picked", |a| a.m.snapshot(0).steps[&step].status == "picked A&M Records 31452 3787 2, US 1995");
}

/// In a narrow window nothing in the sidebar runs off its sides.
#[test]
fn the_sidebar_keeps_inside_a_narrow_window() {
    let e = env();
    let mut h = window_at(&e, Some(9), true, 0.0, egui::vec2(800.0, 600.0), 1.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    app(&h).m.send(0, drt_engine::Command::Answer { step: "choose".into(), answer: drt_engine::Answer::Pick(0) });
    until(&mut h, "a damaged track", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 1 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 16);
    let inside = |h: &Harness<Window>, what: &str| {
        let d = app(h).side_rect().unwrap();
        let mut bad = vec![];
        for n in h.root().children_recursive() {
            let Some(r) = n.accesskit_node().bounding_box() else { continue };
            // The window's resizing edges run along its sides, and the
            // canvas's tiles go on behind the sidebar.
            let tiles: Vec<String> = app(h).m.snapshot(app(h).sel).steps.values().map(|v| format!("{},", v.name)).collect();
            if n.accesskit_node().label().is_some_and(|l| l.starts_with("Resize ") || tiles.iter().any(|t| l.starts_with(t.as_str()))) {
                continue;
            }
            let (top, left, right) = (r.y0 as f32, r.x0 as f32, r.x1 as f32);
            if top >= d.top() && right > left && right > d.left() + 0.5 && (left < d.left() - 0.5 || right > d.right() + 0.5) {
                bad.push(format!("{:?} {:?}", n.accesskit_node().label(), r));
            }
        }
        assert!(bad.is_empty(), "{what}: past the sidebar's sides: {bad:?}");
    };
    open_tile(&mut h, "Rip,");
    assert!(!app(&h).cut_titles.contains(&"Broken Bricks".to_owned()), "cut at 800: {:?}", app(&h).cut_titles);
    inside(&h, "Rip");
    app(&h).m.send(0, drt_engine::Command::Retry { step: "rip".into(), item: "track-9".into(), option: "keep".into() });
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    open_tile(&mut h, "Hold,");
    inside(&h, "the held Library");
}

/// FLAC that reports progress as the real encoder does, and holds two
/// tracks part done until let go.
struct TestEncoder {
    inner: std::sync::Arc<dyn drt_engine::Step>,
    release: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl drt_engine::Step for TestEncoder {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        let mut items = vec![];
        while let Some(it) = inlet.recv().await {
            items.push(it);
        }
        let n = items.len();
        for (k, it) in items.iter().enumerate() {
            match k {
                0 => cx.item(it, ItemState::Busy, "encoding 43%, 12s left"),
                1 => cx.item(it, ItemState::Busy, "encoding 78%, 4s left"),
                _ => cx.item(it, ItemState::Ok, ""),
            }
        }
        cx.status(format!("{} of {n}", n - 2), "2 encoding, 61%");
        cx.progress(Some(0.61));
        while !self.release.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        for it in items {
            cx.item(&it, ItemState::Ok, "");
            outlet.send(it.becomes(drt_engine::ItemKind::AudioFile, None)).await;
        }
        Ok(())
    }
}

#[test]
fn encoding_shows_how_far_and_how_long() {
    let e = env();
    let mut reg = simulated(None);
    let inner = reg.get("flac").unwrap().0;
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    reg.add(TestEncoder { inner, release: release.clone() });
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "encoding", |a| a.m.snapshot(0).steps["flac"].detail == "2 encoding, 61%");
    open_tile(&mut h, "FLAC,");
    // Each track in progress: its time left and percentage, and a bar.
    h.get_by_label_contains("12s left 43%");
    h.get_by_label_contains("4s left 78%");
    h.get_by_label("61% overall");
    assert!(h.get_all_by_label("Encoding").next().is_some(), "the pill says it is");
    // The tile keeps the percentage when its detail is cut.
    shot(&mut h, "step-encoder-progress");
    // The Library's names, with this disc's example path.
    open_tile(&mut h, "Library,");
    // (The simulated lookups give no album artist, so the real naming
    // says so, as it would on a disc without one.)
    h.get_by_label_contains("Unknown artist/The White Stripes/01 - Jimmy the Exploder.flac");
    release.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn a_folder_names_its_files_by_a_template() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    open_tile(&mut h, "Library,");
    // A preset from the dropdown.
    h.get_by_label("Names").click();
    h.run_steps(2);
    click(&mut h, "Sorted artist / Album / 01 Artist - Title");
    assert_eq!(param(&saved_recipes(&e)[0], "lib1", "names").unwrap().resolve(None), Some(Value::from("Sorted artist / Album / 01 Artist - Title")));
    // With no disc yet, an example from a sample track.
    h.get_by_label_contains("Cinematic Orchestra, The/Every Day/01 The Cinematic Orchestra - All That You Give.flac");
    shot(&mut h, "step-folder-template");
    // Typed: it becomes Custom.
    h.get_by_label("Template").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("{year} {album}/{track}. {title}".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(param(&saved_recipes(&e)[0], "lib1", "names").unwrap().resolve(None), Some(Value::from("{year} {album}/{track}. {title}")));
    h.get_by_value("Custom");
    h.get_by_label_contains("2002 Every Day/01. All That You Give.flac");
    // A token goes in where the cursor was: here, the end.
    h.get_by_label("Insert {label}").click_accesskit();
    h.run_steps(3);
    assert_eq!(param(&saved_recipes(&e)[0], "lib1", "names").unwrap().resolve(None), Some(Value::from("{year} {album}/{track}. {title}{label}")));
}

#[test]
fn encoder_settings_follow_their_mode_and_fold_away() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    open_tile(&mut h, "MP3,");
    // VBR: Quality, not Bitrate.
    assert!(h.query_by_label("Quality").is_some() && h.query_by_label("Bitrate").is_none());
    assert!(h.query_by_label_contains("MP3 as V0").is_none(), "not the tile again");
    shot(&mut h, "step-mp3-vbr");
    // CBR swaps them at once.
    click(&mut h, "Bitrate type: Constant");
    assert!(h.query_by_label("Bitrate").is_some() && h.query_by_label("Quality").is_none());
    shot(&mut h, "step-mp3-cbr");
    // FLAC's compression is a slider; moving it saves it.
    open_tile(&mut h, "FLAC,");
    let slider = h.get_by_role(Role::Slider);
    assert_eq!(slider.accesskit_node().numeric_value(), Some(8.0));
    slider.focus();
    h.run_steps(1);
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(3);
    assert_eq!(param(&saved_recipes(&e)[0], "flac", "level").unwrap().resolve(None), Some(Value::from(7)));
    // Opus: Advanced is shown, under its heading; nothing folds.
    app_mut(&mut h).begin_edit();
    h.run_steps(1);
    let i = app(&h).sel;
    let id = app_mut(&mut h).m.insert_after(i, "rip", "opus", true).unwrap();
    app_mut(&mut h).m.save_edit(i);
    app_mut(&mut h).editing = false;
    app_mut(&mut h).open(Tray::Step { id, focus: None });
    h.run_steps(3);
    h.get_by_label("Advanced");
    assert!(h.get_all_by_label("Effort (higher is slower, sounds a little better)").count() > 0, "shown, not folded");
    shot(&mut h, "step-opus");
}

/// Where a strip's + is on the screen, and how big.
fn plus_at(h: &Harness<Window>, after: &str) -> (egui::Pos2, f32) {
    let a = app(h);
    let lay = a.layout(a.sel);
    let (_, (x, y)) = lay.pluses.iter().find(|(id, _)| id == after).unwrap().clone();
    let v = a.views_of(a.sel);
    (a.canvas_rect().min + v.pan + egui::vec2(x, y) * v.zoom, drt_layout::PLUS_R * v.zoom)
}

#[test]
fn the_plus_opens_a_popover_beside_it_with_a_search() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Edit");
    click(&mut h, "Add after Audio CD");
    h.run_steps(2);
    // Anchored beside its +, and on the canvas.
    let (plus, r) = plus_at(&h, "cd");
    let pr = app(&h).popover_rect;
    assert!((pr.left() - (plus.x + r + 10.0)).abs() < 1.0 || (pr.right() - (plus.x - r - 10.0)).abs() < 1.0, "{pr:?} by {plus:?}");
    assert!(app(&h).canvas_rect().contains_rect(pr));
    assert!((pr.width() - drt_app::popover::WIDTH).abs() < 2.0 && pr.height() <= drt_app::popover::MAX_HEIGHT + 1.0, "{pr:?} {}x{}", pr.width(), pr.height());
    assert!(app(&h).tray.is_none(), "not the sidebar");
    // The search has the keys at once, and filters by name.
    h.event(egui::Event::Text("gnu".into()));
    h.run_steps(2);
    h.get_by_label("GnuDB");
    assert!(h.query_by_label("Discogs").is_none());
    shot(&mut h, "picker-popover");
    // And by what a step does.
    for _ in 0..3 {
        h.key_press(egui::Key::Backspace);
    }
    // ("which album" would find Choose, but this recipe already has one
    // further on, so it isn't offered; "database" finds the other lookups.)
    h.event(egui::Event::Text("database".into()));
    h.run_steps(2);
    h.get_by_label("Discogs");
    assert!(h.query_by_label("Choose").is_none());
    // Esc closes it.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(app(&h).popover.is_none());
    // So does a click anywhere else.
    click(&mut h, "Add after Audio CD");
    assert!(app(&h).popover.is_some());
    let empty = app(&h).canvas_rect().left_bottom() + egui::vec2(300.0, -40.0);
    h.hover_at(empty);
    h.step();
    h.drag_at(empty);
    h.drop_at(empty);
    h.run_steps(2);
    assert!(app(&h).popover.is_none());
    // And panning the canvas.
    click(&mut h, "Add after Audio CD");
    app_mut(&mut h).set_view(0, (1.0, egui::vec2(10.0, 10.0)));
    h.run_steps(2);
    assert!(app(&h).popover.is_none(), "closes when the canvas moves");
}

#[test]
fn the_keyboard_picks_from_the_popover() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Edit");
    click(&mut h, "Add after Audio CD");
    h.run_steps(2);
    // The rows as listed: by group, then name.
    let mut opts: Vec<(u8, &'static str, &'static str)> = app(&h).m.placements_after(0, "cd").into_iter().map(|(i, _, _)| (i.group as u8, i.name, i.kind)).collect();
    opts.sort();
    // Every row listed fits; nothing else is.
    let rows: Vec<String> = h.get_all_by_role(Role::ListItem).filter_map(|n| n.accesskit_node().label().map(|l| l.to_string())).filter(|l| opts.iter().any(|o| l.starts_with(o.1))).collect();
    assert_eq!(rows.len(), opts.len(), "{rows:?}");
    h.key_press(egui::Key::ArrowDown);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(app(&h).popover.is_none());
    let want = opts[2].2;
    let draft = app(&h).m.recipes[0].clone();
    let mut root = draft.root.clone();
    let added = root.ids().into_iter().map(str::to_owned).find(|id| !recipes::cd().root.ids().contains(&id.as_str())).unwrap();
    assert!(matches!(root.find_mut(&added), Some(Node::Step { step, .. }) if step == want), "added {added}, wanted a {want}");
}

#[test]
fn the_popover_stays_on_screen_at_the_edges() {
    for ppp in [1.0, 2.0] {
        let e = env();
        let mut h = window_at(&e, None, true, 0.0, egui::vec2(1280.0, 720.0), ppp);
        click(&mut h, "Edit");
        // Move the canvas so a + sits at the right edge, then the bottom.
        for (dx, dy) in [(1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            let (plus, _) = plus_at(&h, "lib2");
            let c = app(&h).canvas_rect();
            let want = egui::pos2(if dx > 0.0 { c.right() - 20.0 } else { plus.x }, if dy > 0.0 { c.bottom() - 20.0 } else { plus.y });
            let v = app(&h).views_of(0);
            app_mut(&mut h).set_view(0, (v.zoom, v.pan + (want - plus)));
            h.run_steps(2);
            h.get_by_label("Add after Library, ~/Music/MP3").click_accesskit();
            h.run_steps(4);
            assert!(app(&h).popover.is_some());
            let pr = app(&h).popover_rect;
            assert!(c.contains_rect(pr), "at {ppp}x, ({dx}, {dy}): {pr:?} outside {c:?}");
            app_mut(&mut h).popover = None;
            h.run_steps(2);
        }
    }
}

fn names(h: &Harness<Window>) -> Vec<String> {
    app(h).m.recipes.iter().map(|r| r.name.clone()).collect()
}

#[test]
fn in_edit_mode_the_header_renames_duplicates_and_deletes() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // No ··· in the list any more.
    assert!(h.query_by_label("Options for CDs").is_none());
    click(&mut h, "Edit");
    // The title is the name, edited in place, as part of the draft.
    h.get_by_label("Recipe name").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("Music".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(app(&h).m.recipes[0].name, "Music");
    assert_eq!(saved_recipes(&e)[0].name, "CDs", "not until Save");
    shot(&mut h, "edit-header-rename");
    // With that unsaved, Duplicate waits.
    assert!(h.get_by_label("Duplicate").accesskit_node().is_disabled());
    click(&mut h, "Cancel");
    assert_eq!(app(&h).m.recipes[0].name, "CDs", "Cancel puts it back");
    click(&mut h, "Edit");
    h.get_by_label("Recipe name").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("Music".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    click(&mut h, "Save");
    assert_eq!(saved_recipes(&e)[0].name, "Music");
    // Duplicate: the saved recipe, below it, in edit mode.
    click(&mut h, "Edit");
    click(&mut h, "Duplicate");
    assert_eq!(names(&h)[1], "Music copy");
    assert_eq!(app(&h).sel, 1);
    assert!(app(&h).editing && app(&h).m.editing(1) && !app(&h).m.editing(0));
    assert_eq!(saved_recipes(&e)[1].name, "Music copy");
    // Delete asks in the header, then goes; the next one along is shown.
    // The title field holds the whole name.
    let field = h.get_by_label("Recipe name").rect();
    let text = h.ctx.fonts_mut(|f| f.layout_no_wrap("Music copy".into(), drt_app::theme::font(17.0, 700), egui::Color32::WHITE).size().x);
    assert!(field.width() >= text + 16.0, "field {} for text {text}", field.width());
    click(&mut h, "Delete");
    h.get_by_label("Delete Music copy?");
    shot(&mut h, "edit-header-delete");
    click(&mut h, "Don't delete");
    assert_eq!(names(&h)[1], "Music copy", "kept");
    click(&mut h, "Delete");
    click(&mut h, "Delete");
    assert!(!names(&h).contains(&"Music copy".to_owned()));
    assert!(!saved_recipes(&e).iter().any(|r| r.name == "Music copy"));
    assert_eq!(app(&h).sel, 1, "the next one along");
    assert!(!app(&h).editing);
}

#[test]
fn a_recipe_holding_a_disc_cannot_be_deleted() {
    let e = env();
    let (reg, open) = latched_hold(simulated(None));
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    app_mut(&mut h).m.repeat_limit = Some(1);
    click(&mut h, "Start");
    to_the_hold(&mut h);
    until(&mut h, "the second disc", |a| a.m.runs.get(&0).is_some_and(|v| v.len() == 2));
    open_latch(&open);
    until(&mut h, "the first to hold", |a| a.m.runs[&0][0].snap.steps["hold"].detail == "All here");
    // While running, it can't even be edited, and Delete would be refused.
    assert!(h.get_by_label("Edit").accesskit_node().is_disabled());
    assert_eq!(app(&h).m.cant_delete(0), Some("running"));
    // Stop the second disc: the first still holds.
    click(&mut h, "Stop");
    until(&mut h, "the stop", |a| a.m.newest(0).is_some_and(|r| r.snap.finished));
    assert!(!app(&h).m.running(0) && !app(&h).m.earlier(0).is_empty());
    // No new disc while the first waits for you: Start says why.
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    assert_eq!(app(&h).m.cant_start(0).as_deref(), Some("Let the files on an earlier disc through first"));
    h.get_by_label_contains("CDs, Let the files on an earlier disc through first");
    click(&mut h, "Edit");
    click(&mut h, "Delete");
    assert!(app(&h).confirm_delete.is_none(), "not even asked");
    h.get_by_label_contains("Can't delete: holding a disc");
    assert_eq!(names(&h)[0], "CDs");
    assert!(app_mut(&mut h).m.delete_recipe(0).is_err());
}

#[test]
fn a_folder_of_disc_images_names_its_folder_from_the_dump() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    h.get_by_label_contains("Data discs").click();
    h.run_steps(2);
    open_tile(&mut h, "Library,");
    // The image presets and tokens, and an example folder, not a track.
    h.get_by_value("Name");
    h.get_by_label("Insert {system}");
    assert!(h.query_by_label("Insert {album}").is_none());
    h.get_by_label_contains("Encarta 98 Encyclopedia (USA) (Disc 1)/");
    h.get_by_label("Names").click();
    h.run_steps(2);
    click(&mut h, "System / Name");
    h.get_by_label_contains("IBM PC/Encarta 98 Encyclopedia (USA) (Disc 1)/");
    assert!(h.query_by_label("Sorted artist / Album / 01 Artist - Title").is_none(), "no music layouts here");
    shot(&mut h, "step-folder-images");
}

/// What a Folder's card says of it, as `Label value` rows.
fn card_says(h: &mut Harness<Window>, id: &str) -> Vec<String> {
    let ctx = h.ctx.clone();
    let s = snap(h);
    app_mut(h).cards(&ctx, &s)[id].settings.iter().map(|x| format!("{} {}", x.label, x.value)).collect()
}

/// What a Folder's template box holds.
fn template_box(h: &Harness<Window>) -> String {
    h.get_all_by_role(Role::TextInput).find(|n| n.accesskit_node().label().as_deref() == Some("Template")).and_then(|n| n.accesskit_node().value()).unwrap_or_default()
}

/// The presets a Folder's Names offers, with its dropdown open.
fn presets_offered(h: &mut Harness<Window>) -> Vec<String> {
    h.get_by_label("Names").click();
    h.run_steps(2);
    let all = [
        "Artist / Album / 01 - Title",
        "Sorted artist / Album / 01 Artist - Title",
        "Album / 01 - Title",
        "01 - Title",
        "Name",
        "System / Name",
        "System / Game",
        "Date and disc name",
        "Jellyfin",
        "Jellyfin film",
        "Jellyfin episode",
    ];
    let offered = all.iter().filter(|l| h.query_all_by_label(l).any(|n| n.accesskit_node().role() != Role::ComboBox)).map(|l| (*l).to_owned()).collect();
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    offered
}

#[test]
fn a_folder_s_names_follow_what_reaches_it() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // After Rip and FLAC: tracks, the music layouts.
    open_tile(&mut h, "Library,");
    h.get_by_value("Artist / Album / 01 - Title");
    shot(&mut h, "step-folder-music");
    assert_eq!(presets_offered(&mut h), ["Artist / Album / 01 - Title", "Sorted artist / Album / 01 Artist - Title", "Album / 01 - Title", "01 - Title"]);
    assert_eq!(card_says(&mut h, "lib1")[1], "Names Artist / Album / 01 - Title");
    // A game's CHD: a folder for the game under its system, and only the
    // image layouts and tokens.
    h.get_by_label_contains("Games, ").click();
    h.run_steps(2);
    open_tile(&mut h, "Library,");
    h.get_by_value("System / Game");
    assert_eq!(template_box(&h), "{system}/{title}");
    h.get_by_label_contains("PlayStation/Colony Wars (Europe)/");
    assert!(h.query_by_label_contains("(Disc 1)/").is_none(), "the game's folder, not the disc's");
    h.get_by_label("Insert {serial}");
    assert!(h.query_by_label("Insert {album}").is_none());
    shot(&mut h, "step-folder-game");
    assert_eq!(presets_offered(&mut h), ["Name", "System / Name", "System / Game", "Date and disc name"]);
    assert_eq!(card_says(&mut h, "g_lib"), ["Folder ~/Games", "Names System / Game"]);
    // Films and episodes, made smaller: Jellyfin's.
    h.get_by_label_contains("Films and TV").click();
    h.run_steps(2);
    open_tile(&mut h, "Library,");
    h.get_by_value("Jellyfin");
    h.get_by_label("Insert {tmdbid}");
    assert!(h.query_by_label("Insert {album}").is_none() && h.query_by_label("Insert {system}").is_none());
    shot(&mut h, "step-folder-video");
    assert_eq!(presets_offered(&mut h), ["Jellyfin", "Jellyfin film", "Jellyfin episode"]);
    assert_eq!(card_says(&mut h, "v_lib")[1], "Names Jellyfin");
}

#[test]
fn a_folder_added_after_a_chd_names_its_folder_from_the_dump() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    h.get_by_label_contains("Data discs").click();
    h.run_steps(2);
    let i = app(&h).sel;
    let id = app_mut(&mut h).m.insert_after(i, "d_chd", "folder", true).unwrap();
    h.run_steps(2);
    // Nothing is stored: its names are the default for what reaches it.
    assert_eq!(param(&saved_recipes(&e)[i], &id, "names"), None);
    assert_eq!(card_says(&mut h, &id)[1], "Names Name");
    open_id(&mut h, &id);
    h.get_by_value("Name");
    assert_eq!(template_box(&h), "{name}");
    assert!(h.query_by_label("Insert {album}").is_none() && h.query_by_label("Insert {serial}").is_none(), "no music or game tokens for another disc");
    shot(&mut h, "step-folder-added-after-chd");
    assert_eq!(presets_offered(&mut h), ["Name", "System / Name", "Date and disc name"]);
    // One saved on the music layout before names followed the files reads as the image's.
    app_mut(&mut h).m.set_param(i, &id, "names", Value::from("{albumartist}/{album}/{track} - {title}")).unwrap();
    h.run_steps(2);
    assert_eq!(card_says(&mut h, &id)[1], "Names Name");
    open_id(&mut h, &id);
    assert_eq!(template_box(&h), "{name}");
    h.get_by_value("Name");
    // The + after the CHD offers a Folder.
    app_mut(&mut h).close_tray();
    click(&mut h, "Edit");
    click(&mut h, "Add after CHD");
    h.event(egui::Event::Text("fol".into()));
    h.run_steps(2);
    shot(&mut h, "picker-folder-after-chd");
}

/// A setting's label (not the control it names).
fn first(h: &Harness<Window>, label: &str) -> egui::Rect {
    h.get_all_by_role(Role::Label)
        .find(|n| n.accesskit_node().value().is_some_and(|v| v == label) || n.accesskit_node().label().is_some_and(|l| l == label))
        .unwrap_or_else(|| panic!("no {label}"))
        .rect()
}

#[test]
fn settings_stack_one_per_row_and_wide_editors_take_the_sidebar() {
    let e = env();
    let mut h = window_at(&e, Some(9), true, 0.0, egui::vec2(1920.0, 1080.0), 1.0);
    open_tile(&mut h, "Library,");
    let d = app(&h).side_rect().unwrap();
    assert!((d.width() - 672.0).abs() < 1.0, "wider at 1920: {d:?}");
    // Label then control, one under the other, never in columns.
    let (folder, names) = (first(&h, "Folder"), first(&h, "Names"));
    assert!(names.top() > folder.bottom() + 20.0 && (names.left() - folder.left()).abs() < 1.0, "stacked: {folder:?} {names:?}");
    // The template's field runs the sidebar's width.
    let tpl = h.get_by_label("Template").rect();
    assert!(tpl.width() > d.width() - 60.0, "full width: {tpl:?} in {d:?}");
    // Rip mid-run: its rows from the sidebar's edge; its settings locked,
    // folded under the rows.
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "a damaged track", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 1 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 16);
    open_tile(&mut h, "Rip,");
    let row = h.get_by_label_contains("9, Broken Bricks").rect();
    let d = app(&h).side_rect().unwrap();
    assert!(row.left() < d.left() + 30.0 && row.right() > d.right() - 40.0, "the rows take the sidebar: {row:?} in {d:?}");
    h.get_by_label("Settings");
    assert!(h.query_all_by_role(Role::Label).all(|n| n.accesskit_node().value().as_deref() != Some("Ripping speed")), "folded while ripping");
}

#[test]
fn an_encoder_s_settings_are_one_column() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    open_tile(&mut h, "MP3,");
    let (mode, stereo) = (first(&h, "Bitrate type"), first(&h, "Stereo"));
    assert!(stereo.top() > mode.bottom() && (mode.left() - stereo.left()).abs() < 1.0, "stacked: {mode:?} {stereo:?}");
    assert!((app(&h).side_rect().unwrap().width() - 448.0).abs() < 1.0, "448 at 1280");
}

#[test]
fn old_library_steps_load_as_folders() {
    let e = env();
    // A recipes.json from before: two `library` steps, one with a label.
    let old = r#"[{"name":"Old","root":{"t":"chain","nodes":[
        {"t":"step","id":"drive","step":"drive","params":{"drive":{"how":"chosen","value":"Blu-ray drive","at":0}}},
        {"t":"step","id":"cd","step":"audio-cd"},
        {"t":"step","id":"rip","step":"rip"},
        {"t":"step","id":"flac","step":"flac"},
        {"t":"step","id":"lib","step":"library","params":{"folder":{"how":"chosen","value":"~/Music","at":0}}},
        {"t":"step","id":"nas","step":"library","label":"NAS","params":{"folder":{"how":"chosen","value":"/mnt/nas","at":0}}}]}}]"#;
    let dir = e.dir.path().join("config");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("recipes.json"), old).unwrap();
    let h = window(&e, None, true, 0.0);
    // Folders now, keeping what they were called; saved so.
    for r in [app(&h).m.recipes[0].clone(), saved_recipes(&e)[0].clone()] {
        let mut root = r.root.clone();
        assert!(matches!(root.find_mut("lib"), Some(Node::Step { step, label: Some(l), .. }) if step == "folder" && l == "Library"));
        assert!(matches!(root.find_mut("nas"), Some(Node::Step { step, label: Some(l), .. }) if step == "folder" && l == "NAS"));
    }
    let tiles: Vec<String> = h.get_all_by_role(Role::Button).filter_map(|n| n.accesskit_node().label()).collect();
    assert!(tiles.iter().any(|l| l.starts_with("Library,")) && tiles.iter().any(|l| l.starts_with("NAS,")), "{tiles:?}");
}

#[test]
fn one_folder_step_and_a_tile_can_be_named() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Edit");
    assert!(!app(&h).m.recipes[0].root.ids().contains(&"folder"));
    h.get_by_label("Add after Library, ~/Music/MP3").click_accesskit();
    h.run_steps(3);
    // One Folder, no Library.
    h.get_by_label("Folder");
    assert!(h.query_by_label("Library").is_none());
    h.get_by_label("Folder").click_accesskit();
    h.run_steps(3);
    // It's called Folder.
    let id = "folder".to_owned();
    assert!(app(&h).m.recipes[0].root.ids().contains(&id.as_str()));
    open_tile(&mut h, "Folder,");
    shot(&mut h, "tile-name");
    // Named in its sidebar's header, as part of the draft.
    h.get_by_label("Step name").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("Car".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    click(&mut h, "Save");
    let mut root = saved_recipes(&e)[0].root.clone();
    assert!(matches!(root.find_mut(&id), Some(Node::Step { step, label: Some(l), .. }) if step == "folder" && l == "Car"));
}

#[test]
fn settings_are_locked_while_ripping() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    // A folder's names: the preset and template as text, no box, no chips.
    open_id(&mut h, "lib1");
    h.get_by_label("Locked while ripping");
    h.get_by_label("Artist / Album / 01 - Title");
    h.get_by_label("{albumartist}/{album}/{track} - {title}");
    assert!(h.query_by_label("Template").is_none() && h.query_all_by_role(Role::TextInput).next().is_none());
    assert!(h.query_by_label("Insert {label}").is_none());
    shot(&mut h, "locked-folder");
    // An encoder's segments and slider: values, not controls.
    open_id(&mut h, "mp3");
    assert!(h.query_by_label("Bitrate type: Constant").is_none() && h.query_by_label("Bitrate type: Variable, best for the size").is_none());
    open_id(&mut h, "flac");
    assert!(h.query_by_role(Role::Slider).is_none());
    h.get_by_label("8");
    shot(&mut h, "locked-flac");
    // The drive's settings can't change underneath it either.
    let i = app(&h).sel;
    assert!(app_mut(&mut h).m.set_param(i, "flac", "level", Value::from(5)).is_err());
    assert!(app_mut(&mut h).m.set_param(i, "drive", "offset", Value::from(6)).is_err());
    // Answering still works; when the run is done the controls come back.
    app_mut(&mut h).open(Tray::Question { step: "choose".into() });
    h.run_steps(3);
    click(&mut h, "Use this one");
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    click_tile(&mut h, "Hold,");
    h.get_by_label("Let 17 through").click();
    h.run_steps(2);
    until(&mut h, "the end", |a| a.m.snapshot(0).finished);
    open_id(&mut h, "flac");
    tab(&mut h, "Settings");
    assert!(h.query_by_label("Locked while ripping").is_none());
    h.get_by_role(Role::Slider);
    open_id(&mut h, "lib1");
    tab(&mut h, "Settings");
    h.get_by_label("Template");
    h.get_by_label("Insert {label}");
}

/// Opens a tile's sidebar by its id, whatever its tile says just now.
fn open_id(h: &mut Harness<Window>, id: &str) {
    app_mut(h).open(Tray::Step { id: id.into(), focus: None });
    h.run_steps(3);
}

/// Picks one of the sidebar's tabs.
fn tab(h: &mut Harness<Window>, name: &str) {
    h.get_all_by_role(Role::Tab).find(|n| n.accesskit_node().label().as_deref() == Some(name)).unwrap_or_else(|| panic!("no {name} tab")).click_accesskit();
    h.run_steps(2);
}

/// The sidebar's tab shown, if it has a tab bar.
fn tab_on(h: &Harness<Window>) -> Option<String> {
    h.query_all_by_role(Role::Tab)
        .find(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True) && ["Progress", "Settings", "Log"].contains(&n.accesskit_node().label().as_deref().unwrap_or("")))
        .and_then(|n| n.accesskit_node().label())
}

/// The drive dropdown's choices, opened: each one's words.
fn drive_options(h: &mut Harness<Window>) -> Vec<String> {
    h.get_all_by_role(Role::ComboBox).next().expect("the drive dropdown, first").click_accesskit();
    h.run_steps(2);
    let all: Vec<String> = h
        .get_all_by_role(Role::Button)
        .filter_map(|n| n.accesskit_node().label())
        .filter(|l| l.contains(recipes::BLU_RAY) || l.contains(recipes::DVD))
        .filter(|l| !l.contains(", Ready") && !l.contains(", Drive"))
        .collect();
    all
}

/// Picks a drive in the dropdown.
fn pick_drive(h: &mut Harness<Window>, d: &str) {
    if h.query_all_by_role(Role::Button).all(|n| !n.accesskit_node().label().is_some_and(|l| l.starts_with(d))) {
        h.get_all_by_role(Role::ComboBox).next().expect("the drive dropdown, first").click_accesskit();
        h.run_steps(2);
    }
    h.get_all_by_role(Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with(d))).unwrap_or_else(|| panic!("no {d} to pick")).click();
    h.run_steps(3);
}

/// What the window asked of the system over the next few frames, a
/// twentieth of a second apart (quick enough for a double-click).
fn sent(h: &mut Harness<Window>, events: Vec<Vec<egui::Event>>) -> Vec<String> {
    let mut out = vec![];
    for frame in events.into_iter().chain([vec![], vec![]]) {
        for e in frame {
            h.event(e);
        }
        let now = h.ctx.input(|i| i.time);
        h.input_mut().time = Some(now + 0.05);
        h.step();
        h.input_mut().time = None;
        if let Some(v) = h.output().viewport_output.get(&egui::ViewportId::ROOT) {
            out.extend(v.commands.iter().map(|c| format!("{c:?}")));
        }
    }
    out
}

fn press(at: egui::Pos2, down: bool) -> egui::Event {
    egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: down, modifiers: egui::Modifiers::NONE }
}

#[test]
fn the_window_draws_its_own_frame() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // No system title bar: the title names the recipe.
    assert_eq!(app(&h).title, "Disc Rip Trip — CDs");
    // A double-click on an empty part of the header maximises.
    let at = egui::pos2(820.0, 25.0);
    let s = sent(&mut h, vec![vec![egui::Event::PointerMoved(at)], vec![press(at, true)], vec![press(at, false)], vec![press(at, true)], vec![press(at, false)]]);
    assert!(s.iter().any(|c| c == "Maximized(true)"), "{s:?}");
    // An empty part of the header drags the window; of the list's top too.
    for at in [egui::pos2(820.0, 25.0), egui::pos2(120.0, 25.0)] {
        let s = sent(
            &mut h,
            vec![vec![egui::Event::PointerMoved(at)], vec![press(at, true)], vec![egui::Event::PointerMoved(at + egui::vec2(30.0, 10.0))], vec![press(at + egui::vec2(30.0, 10.0), false)]],
        );
        assert!(s.iter().any(|c| c == "StartDrag"), "{at:?}: {s:?}");
    }
    // Anywhere on the band: the recipe's name, the status chip, the gap
    // between the header's buttons, the window buttons themselves.
    let drag_from = |h: &mut Harness<Window>, at: egui::Pos2| {
        let to = at + egui::vec2(40.0, 12.0);
        sent(
            h,
            vec![vec![egui::Event::PointerMoved(at)], vec![press(at, true)], vec![egui::Event::PointerMoved(at + egui::vec2(3.0, 1.0))], vec![egui::Event::PointerMoved(to)], vec![press(to, false)]],
        )
    };
    let title = h.get_all_by_label("CDs").find(|n| n.accesskit_node().role() == Role::Label && n.rect().top() < 50.0).expect("the title label").rect().center();
    let (edit, start) = (h.get_by_label("Edit").rect(), h.get_by_label("Start").rect());
    let gap = egui::pos2((edit.right() + start.left()) / 2.0, edit.center().y);
    for (what, at) in [("the title", title), ("the gap between buttons", gap), ("Edit", edit.center()), ("Minimise", h.get_by_label("Minimise").rect().center())] {
        let s = drag_from(&mut h, at);
        assert_eq!(s.iter().filter(|c| *c == "StartDrag").count(), 1, "{what} at {at:?}: {s:?}");
        assert!(!app(&h).editing, "a drag from Edit isn't a click on it");
    }
    // A double-click on the name maximises, as on any of the band.
    let s = sent(&mut h, vec![vec![egui::Event::PointerMoved(title)], vec![press(title, true)], vec![press(title, false)], vec![press(title, true)], vec![press(title, false)]]);
    assert!(s.iter().any(|c| c.starts_with("Maximized(")), "{s:?}");
    // A press that doesn't move is still nothing but a click.
    let s = sent(&mut h, vec![vec![egui::Event::PointerMoved(title)], vec![press(title, true)], vec![press(title, false)]]);
    assert!(!s.iter().any(|c| c == "StartDrag"), "{s:?}");
    // But a button in the header is still a button.
    let at = h.get_by_label("Edit").rect().center();
    let s = sent(&mut h, vec![vec![egui::Event::PointerMoved(at)], vec![press(at, true)], vec![press(at, false)]]);
    assert!(s.is_empty() && app(&h).editing, "{s:?}");
    app_mut(&mut h).end_edit(false);
    h.run_steps(2);
    // The three buttons.
    for (name, cmd) in [("Minimise", "Minimized(true)"), ("Maximise", "Maximized(true)"), ("Close window", "Close")] {
        h.get_by_label(name).click_accesskit();
        let s = sent(&mut h, vec![]);
        assert!(s.iter().any(|c| c == cmd), "{name}: {s:?}");
    }
    // Every edge and corner resizes.
    for (at, dir) in [
        (egui::pos2(640.0, 718.0), "South"),
        (egui::pos2(640.0, 1.0), "North"),
        (egui::pos2(1.0, 360.0), "West"),
        (egui::pos2(1278.0, 360.0), "East"),
        (egui::pos2(1278.0, 718.0), "SouthEast"),
        (egui::pos2(1.0, 1.0), "NorthWest"),
    ] {
        let s = sent(&mut h, vec![vec![egui::Event::PointerMoved(at)], vec![press(at, true)], vec![press(at, false)]]);
        assert!(s.iter().any(|c| *c == format!("BeginResize({dir})")), "{dir}: {s:?}");
    }
    // The status chip, while it runs.
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    let chip = h.get_all_by_label("Needs you").find(|n| n.rect().top() < 50.0).expect("the chip").rect().center();
    let s = drag_from(&mut h, chip);
    assert!(s.iter().any(|c| c == "StartDrag"), "the chip: {s:?}");
    click(&mut h, "Stop");
    until(&mut h, "the stop", |a| a.m.snapshot(0).finished);
    // A step's sidebar still closes with its own Close.
    open_tile(&mut h, "Rip,");
    h.get_by_label("Close");
    app_mut(&mut h).close_tray();
    h.run_steps(2);
    shot(&mut h, "chrome");
}

#[test]
fn a_step_goes_between_two_tiles_from_the_plus_on_their_seam() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // Only while editing.
    assert!(h.query_by_label("Add between FLAC and Review").is_none());
    click(&mut h, "Edit");
    // FLAC's strip is off to the right at this size, so press it by its
    // accessible name rather than where it is drawn.
    h.get_by_label("Add between FLAC and Review").click_accesskit();
    h.run_steps(2);
    assert!(app(&h).popover.as_ref().is_some_and(|p| p.at == drt_engine::Spot::After("flac".into()) && !p.branch));
    // Offered: exactly what, put there in line, leaves the recipe checking.
    let i = app(&h).sel;
    let want: Vec<String> = app(&h).m.placements_after(i, "flac").into_iter().filter(|(_, inline, _)| *inline).map(|(info, _, _)| info.name.to_owned()).collect();
    let not: Vec<String> = app(&h).m.placements_after(i, "flac").into_iter().filter(|(_, inline, _)| !*inline).map(|(info, _, _)| info.name.to_owned()).collect();
    assert!(!want.is_empty());
    // (A group's heading can share its one step's name: Notify.)
    for name in &want {
        assert!(h.get_all_by_label(name).next().is_some(), "{name}");
    }
    // What fits only beside what follows is offered too, and goes there.
    for name in &not {
        assert!(h.get_all_by_label(name).next().is_some(), "{name}");
    }
    shot(&mut h, "picker-between");
    // Picked: in line between the two.
    h.get_by_label(&want[0]).click_accesskit();
    h.run_steps(3);
    let ids: Vec<String> = app(&h).m.recipes[i].root.ids().into_iter().map(str::to_owned).collect();
    let k = ids.iter().position(|x| x == "flac").unwrap();
    assert_eq!(ids[k + 2], "review", "{ids:?}");
    assert!(drt_engine::check(&app(&h).m.recipes[i].root, &app(&h).m.engine.registry).is_ok());
    // Between Choose and Cover art nothing fits in line (Cover art goes
    // straight after Choose), but Lyrics does beside it, on a branch.
    h.get_by_label("Add between Choose and Cover art").click_accesskit();
    h.run_steps(2);
    h.get_by_label("Lyrics").click_accesskit();
    h.run_steps(3);
    let root = &app(&h).m.recipes[i].root;
    assert!(root.ids().contains(&"lyrics"), "{:?}", root.ids());
    assert!(drt_engine::check(root, &app(&h).m.engine.registry).is_ok());
}

#[test]
fn choose_takes_typed_details() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    // The candidates, and the form as one more choice (below CD-Text's,
    // so further down the list).
    assert!(h.query_by_label("Album").is_none());
    h.get_by_label_contains("Enter details, ").click_accesskit();
    h.run_steps(2);
    // Pre-filled from the likeliest release.
    assert_eq!(input(&h, "Album").value().as_deref(), Some("The White Stripes"));
    assert_eq!(input(&h, "Year").value().as_deref(), Some("1999"));
    assert_eq!(input(&h, "Title 1").value().as_deref(), Some("Jimmy the Exploder"));
    input(&h, "Artist 17");
    shot(&mut h, "step-choose-details");
    // Typed over, and used.
    input(&h, "Album").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("Elephant".into()));
    h.run_steps(2);
    click(&mut h, "Use this one");
    until(&mut h, "Choose to take it", |a| a.m.snapshot(0).steps["choose"].state == StepState::Ok);
    assert_eq!(snap(&h).steps["choose"].status, "Elephant");
}

#[test]
fn choose_can_be_skipped() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Skip");
    until(&mut h, "Choose to go on", |a| a.m.snapshot(0).steps["choose"].state == StepState::Ok);
    assert_eq!(snap(&h).steps["choose"].status, "Skipped");
}

/// MusicBrainz that finds nothing.
struct NoMatch(std::sync::Arc<dyn drt_engine::Step>);

#[async_trait::async_trait]
impl drt_engine::Step for NoMatch {
    fn info(&self) -> drt_engine::StepInfo {
        self.0.info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        while let Some(disc) = inlet.recv().await {
            cx.status("Nothing", "by disc ID");
            outlet.send(disc).await;
        }
        Ok(())
    }
}

#[test]
fn with_nothing_found_the_disc_s_own_cd_text_is_offered() {
    let e = env();
    let mut reg = simulated(None);
    let mb = reg.get("musicbrainz").unwrap().0;
    reg.add(NoMatch(mb));
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    // The disc's own text is one of the options, never taken unasked.
    assert!(h.get_all_by_label_contains("What is this disc?").next().is_some());
    h.get_all_by_label_contains("From the disc (CD-Text)").next().unwrap();
    // The form starts from what the options say: CD-Text's titles.
    h.get_by_label_contains("Enter details, ").click_accesskit();
    h.run_steps(2);
    assert_eq!(input(&h, "Title 1").value().as_deref(), Some("JIMMY THE EXPLODER"));
    assert_eq!(input(&h, "Album").value().as_deref(), Some("THE WHITE STRIPES"));
    input(&h, "Album").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("De Stijl".into()));
    h.run_steps(2);
    click(&mut h, "Use this one");
    until(&mut h, "Choose to take it", |a| a.m.snapshot(0).steps["choose"].state == StepState::Ok);
    assert_eq!(snap(&h).steps["choose"].status, "THE WHITE STRIPES — De Stijl", "the artist as CD-Text gave it");
}

#[test]
fn choosing_cd_text_names_the_files_from_its_titles() {
    let e = env();
    let mut reg = simulated(None);
    let mb = reg.get("musicbrainz").unwrap().0;
    reg.add(NoMatch(mb));
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "Choose to take it", |a| a.m.snapshot(0).steps["choose"].state == StepState::Ok);
    assert_eq!(snap(&h).steps["choose"].status, "From the disc (CD-Text)");
    until(&mut h, "the MP3s filed", |a| a.m.snapshot(0).steps["lib2"].state == StepState::Ok);
    let filed = &snap(&h).steps["lib2"].record.files;
    assert_eq!(filed.len(), 17);
    assert!(filed.iter().any(|f| f.path.ends_with("01 - JIMMY THE EXPLODER.mp3")), "{:?}", filed.iter().map(|f| &f.path).collect::<Vec<_>>());
}

/// The text box named `name` (not its label).
fn input<'a>(h: &'a Harness<Window>, name: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(name).find(|n| n.accesskit_node().role() == Role::TextInput).unwrap_or_else(|| panic!("no box {name}"))
}

#[test]
fn a_missing_tool_is_named_in_the_header() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    assert!(h.query_by_label_contains("installed").is_none());
    app_mut(&mut h).m.missing_tools = vec!["cyanrip"];
    h.run_steps(2);
    h.get_by_label("cyanrip isn't installed");
    app_mut(&mut h).m.missing_tools = vec!["cyanrip", "redumper", "ffmpeg"];
    h.run_steps(2);
    h.get_by_label("cyanrip, redumper and ffmpeg aren't installed");
}

#[test]
fn a_settings_file_that_doesnt_read_is_never_overwritten() {
    let e = env();
    let dir = e.dir.path().join("config");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
    let mut h = window(&e, None, true, 0.0);
    // Said in the header, with the folder to look in.
    h.get_by_label("Not saving: settings.json can't be read");
    h.get_by_label("Open folder");
    shot(&mut h, "not-saving");
    // Changing a drive's setting leaves the file as it was.
    let i = app(&h).sel;
    let _ = app_mut(&mut h).m.set_param(i, "drive", "offset", Value::from(6));
    h.run_steps(2);
    assert_eq!(std::fs::read_to_string(dir.join("settings.json")).unwrap(), "{ not json");
}

#[test]
fn closing_mid_rip_asks_first() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // Nothing going through: closing just closes.
    h.get_by_label("Close window").click_accesskit();
    h.run_steps(1);
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    let s = sent(&mut h, vec![]);
    assert!(!s.iter().any(|c| c == "CancelClose") && !app(&h).confirm_quit, "{s:?}");
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    // Mid-rip: cancelled, and asked in the header.
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    let s = sent(&mut h, vec![]);
    assert!(s.iter().any(|c| c == "CancelClose"), "{s:?}");
    h.get_by_label("Quit? CDs is ripping");
    shot(&mut h, "quit-confirm");
    click(&mut h, "Keep ripping");
    assert!(!app(&h).confirm_quit);
    // Asked again, and Quit closes.
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    sent(&mut h, vec![]);
    h.get_by_label("Quit").click_accesskit();
    let s = sent(&mut h, vec![]);
    assert!(s.iter().any(|c| c == "Close"), "{s:?}");
}

#[test]
fn text_that_doesnt_read_stays_with_why() {
    let e = env();
    let inner = simulated(None).get("drive").unwrap().0;
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reg = registry_with(TestDrive { inner, release, only_by_measuring: true });
    let mut h = window_on(&e, reg, false, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    until(&mut h, "the lookup to fail", |a| a.m.finding.is_empty());
    click_tile(&mut h, "Drive, Read offset?");
    input(&h, "Read offset").click();
    h.run_steps(1);
    h.event(egui::Event::Text("abc".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    // Kept as typed, saying why; nothing saved, nothing flashed.
    assert_eq!(input(&h, "Read offset").value().as_deref(), Some("abc"));
    h.get_by_label("A whole number, like +103");
    assert!(app(&h).flash_text().is_none());
    assert!(app(&h).m.engine.settings.lock().unwrap().drive(recipes::BLU_RAY, "offset").resolve(None).is_none());
    shot(&mut h, "offset-invalid");
    // Mended: saved, and the reason goes.
    input(&h, "Read offset").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("+7".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.query_by_label("A whole number, like +103").is_none());
    assert_eq!(app(&h).m.engine.settings.lock().unwrap().drive(recipes::BLU_RAY, "offset").resolve(None), Some(Value::from(7)));
}

#[test]
fn leaving_unsaved_edits_can_stay_and_esc_undoes_the_innermost_thing() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Edit");
    let i = app(&h).sel;
    app_mut(&mut h).m.insert_after(i, "rip", "opus", true).unwrap();
    h.run_steps(2);
    // Another recipe, mid-draft: asked, with a way to stay.
    h.get_by_label_contains("Archive everything").click();
    h.run_steps(2);
    h.get_by_label("Save changes to CDs?");
    shot(&mut h, "leave-unsaved");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(app(&h).leave_to.is_none() && app(&h).editing && app(&h).sel == i, "Esc stays");
    h.get_by_label_contains("Archive everything").click();
    h.run_steps(2);
    click(&mut h, "Discard");
    assert!(app(&h).sel != i && !app(&h).m.recipes[i].root.ids().contains(&"opus"));
    // Typing then Esc puts the field back, and leaves the sidebar open.
    open_tile(&mut h, "Library,");
    input(&h, "Template").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("{title}".into()));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app(&h).tray.is_some(), "the sidebar stays");
    let j = app(&h).sel;
    let lib = app(&h).m.recipes[j].root.ids().into_iter().find(|x| x.contains("lib")).unwrap().to_owned();
    assert_ne!(param(&saved_recipes(&e)[j], &lib, "names").and_then(|s| s.resolve(None)), Some(Value::from("{title}")));
    // Then Esc closes it.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(app(&h).tray.is_none());
}

#[test]
fn a_setting_of_yours_resets_and_a_seeded_recipe_goes_back_to_built_in() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    open_tile(&mut h, "FLAC,");
    assert!(h.query_by_label("Reset Compression").is_none(), "default: nothing to reset");
    let i = app(&h).sel;
    app_mut(&mut h).m.set_param(i, "flac", "level", Value::from(5)).unwrap();
    h.run_steps(2);
    shot(&mut h, "setting-reset");
    h.get_by_label("Reset Compression").click_accesskit();
    h.run_steps(2);
    assert!(param(&saved_recipes(&e)[i], "flac", "level").is_none_or(|s| s.resolve(None).is_none()));
    // Editing a built-in recipe: back to how it came.
    app_mut(&mut h).close_tray();
    click(&mut h, "Edit");
    assert!(h.query_by_label("Reset to original").is_none(), "as built in");
    app_mut(&mut h).m.insert_after(i, "rip", "opus", true).unwrap();
    h.run_steps(2);
    click(&mut h, "Reset to original");
    assert!(!app(&h).m.recipes[i].root.ids().contains(&"opus"));
    // Undoable, like any edit.
    click(&mut h, "Undo");
    assert!(app(&h).m.recipes[i].root.ids().contains(&"opus"));
}

#[test]
fn a_game_redump_doesnt_know_is_named_by_typing_it() {
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::TOMB_RAIDER_UNKNOWN]));
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    h.get_by_label_contains("Games, Ready").click();
    h.run_steps(2);
    click(&mut h, "Start");
    // A dump that doesn't match Redump's waits at Check: kept as is.
    let asked = |a: &App| matches!(&a.m.snapshot(a.sel).question, Some((s, _)) if s == "g_choose");
    until(&mut h, "the check or the question", |a| asked(a) || a.m.snapshot(a.sel).steps["g_check"].state == StepState::Bad);
    if !asked(app(&h)) {
        // From its sidebar, which offers what Check does about it.
        open_id(&mut h, "g_check");
        shot(&mut h, "step-game-check");
        h.get_all_by_label_contains("Keep it anyway, ").next().expect("Keep it anyway on the dump's row").click_accesskit();
        h.run_steps(2);
        until(&mut h, "the question", asked);
    }
    app_mut(&mut h).open(Tray::Question { step: "g_choose".into() });
    h.run_steps(3);
    h.get_by_label_contains("Enter details,").click();
    h.run_steps(2);
    // A game's words and fields, not an album's.
    assert!(h.query_by_label_contains("release").is_none() && h.query_by_label_contains("Tracks").is_none());
    assert!(h.query_all_by_label_contains("Which game?").next().is_some());
    assert!(h.query_by_label("Album").is_none());
    input(&h, "Name").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("Tomb Raider (USA) (Prototype)".into()));
    h.run_steps(2);
    shot(&mut h, "step-choose-game");
    click(&mut h, "Use this one");
    until(&mut h, "Choose to take it", |a| a.m.snapshot(a.sel).steps["g_choose"].status == "Tomb Raider (USA) (Prototype)");
    h.get_by_label_contains("Choose, Tomb Raider (USA) (Prototype)");
}

/// A game Redump has under its serial but read badly: Check holds it and
/// says why, Redump beside it says it waits for Check, then what it found;
/// a game Check names leaves Redump nothing to do, and it says so.
#[test]
fn check_says_why_a_game_doesnt_match_and_redump_says_what_it_did() {
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::LANTERN_KEEP_DAMAGED]));
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 1040.0), 1.0);
    h.get_by_label_contains("Games, Ready").click();
    h.run_steps(2);
    // Before a run, each says what it's for.
    open_id(&mut h, "g_redump");
    h.get_by_label_contains("Looks the game up by its serial when Check and fix can't name it");
    open_id(&mut h, "g_check");
    h.get_by_label_contains("Compares the copy's checksums with Redump's list");
    app_mut(&mut h).close_tray();
    let i = app(&h).sel;
    app_mut(&mut h).m.set_param(i, "g_check", "if_no_match", Value::from("ask")).unwrap();
    // No rips again by itself: asked, it's still offered, once at least.
    app_mut(&mut h).m.set_param(i, "g_check", "extra_rips", Value::from(0)).unwrap();
    h.run_steps(2);
    click(&mut h, "Start");
    until(&mut h, "Check to hold the dump", |a| a.m.snapshot(a.sel).steps["g_check"].items.first().is_some_and(|i| i.state == ItemState::Bad));
    // Close enough to read the cards: Check holding, Redump waiting for it.
    let i = app(&h).sel;
    app_mut(&mut h).set_view(i, (0.9, egui::vec2(-560.0, 40.0)));
    h.run_steps(3);
    h.get_by_label_contains("Redump, Waits for Check and fix, Check and fix needs you");
    h.get_by_label_contains("Check and fix, Doesn't match Redump, same size, 140 bad sectors");
    shot(&mut h, "games-check-holds");
    open_id(&mut h, "g_check");
    // The headline, what's wrong in a line, and why; no sentence under
    // the header, and no retelling of Rip's read.
    h.get_by_label("Doesn't match Redump");
    h.get_by_label("140 sectors read wrong");
    h.get_by_label("The drive didn't report them as bad, so only reading each sector again can find them.");
    assert!(h.query_by_label_contains("Redump has 2 copies of SLES-04711").is_none(), "no header sentence");
    assert!(h.query_by_label_contains("What the first rip reported").is_none() && h.query_by_label_contains("What it likely means").is_none());
    assert!(row_says(&h, &["From Rip: the image"]) && row_says(&h, &["1, Compare with Redump", "Doesn't match"]));
    // The proof, last.
    h.get_by_label("Redump's copy and this one");
    h.get_by_label("Size: Redump 200,000 sectors, this copy 200,000 sectors, the same");
    h.get_by_label_contains("SHA-1: Redump 43c1fe26…, this copy 9b07a5e1…, different");
    // The drive's and redumper's own counts, and what it likely means, are the log's.
    let log: Vec<String> = snap(&h).steps["g_check"].record.log.iter().map(|l| l.line.clone()).collect();
    assert!(log.iter().any(|l| l == "the rip's log: SCSI 0 and C2 0 samples; sectors MSF 140, ECC 15, EDC 12"), "{log:?}");
    assert!(log.iter().any(|l| l.contains("redumper doesn't know this drive")), "{log:?}");
    assert!(h.query_by_label_contains("ECC 15").is_none() && h.query_by_label_contains("SCSI").is_none(), "not on Progress");
    assert!(h.query_all_by_label_contains("Fix bad sectors, ").next().is_some());
    assert!(h.query_all_by_label_contains("Re-read bad parts, ").next().is_none(), "no help when the drive reported nothing");
    assert!(h.query_all_by_label_contains("Rip again and combine, ").next().is_some());
    shot(&mut h, "step-check-mismatch");
    open_id(&mut h, "g_redump");
    h.get_by_label_contains("Waits for Check and fix, which needs you to decide about the dump.");
    shot(&mut h, "step-redump-waiting");
    // Kept: Redump looks the serial up, and Choose asks.
    open_id(&mut h, "g_check");
    h.get_all_by_label_contains("Keep it anyway, ").next().expect("Keep it anyway on the dump's row").click_accesskit();
    h.run_steps(2);
    until(&mut h, "Choose to ask", |a| matches!(&a.m.snapshot(a.sel).question, Some((s, _)) if s == "g_choose"));
    open_id(&mut h, "g_redump");
    h.get_by_label_contains("Found 2 games with serial SLES-04711. None has this copy's checksums, so Choose asks you which it is.");
    assert!(h.query_all_by_label_contains("Lantern Keep (Europe) (Disc 1) (Rev 1)").next().is_some(), "the serial's games are listed");
    shot(&mut h, "step-redump-found");
    h.get_by_label_contains("Redump, Found 2 games, with serial SLES-04711");

    // A game Check names: nothing for Redump to look up.
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::APE_ESCAPE]));
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 800.0), 1.0);
    h.get_by_label_contains("Games, Ready").click();
    h.run_steps(2);
    click(&mut h, "Start");
    until(&mut h, "Redump", |a| a.m.snapshot(a.sel).steps["g_redump"].state == StepState::Ok);
    open_id(&mut h, "g_redump");
    h.get_by_label_contains("Check and fix found the game by its checksums, so there was nothing to look up.");
    h.get_by_label_contains("Redump, Not needed, Check and fix found the game");
    shot(&mut h, "step-redump-not-needed");
    open_id(&mut h, "g_check");
    h.get_by_label("Verified");
    h.get_by_label("Matches Redump's Ape Escape (Europe)");
    assert!(row_says(&h, &["1, Compare with Redump", "Matches"]));
}

/// A misread the drive kept quiet about: Rip again and combine, twice, and
/// Check's sidebar shows each rip and what combining came to; the second
/// combination matches Redump, and the dump goes on verified.
#[test]
fn check_rips_again_and_combines_until_it_matches() {
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::LANTERN_KEEP_DAMAGED]));
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 1100.0), 1.0);
    h.get_by_label_contains("Games, Ready").click();
    h.run_steps(2);
    let i = app(&h).sel;
    app_mut(&mut h).m.set_param(i, "g_check", "if_no_match", Value::from("ask")).unwrap();
    click(&mut h, "Start");
    until(&mut h, "Check to hold the dump", |a| a.m.snapshot(a.sel).steps["g_check"].items.first().is_some_and(|i| i.state == ItemState::Bad));
    open_id(&mut h, "g_check");
    h.get_all_by_label_contains("Rip again and combine, ").next().expect("on the dump's row").click_accesskit();
    h.run_steps(2);
    until(&mut h, "two rips combined", |a| a.m.snapshot(a.sel).steps["g_check"].detail.starts_with("combined from 2 rips"));
    h.get_by_label("4 sectors still not put right, after 2 rips");
    assert!(row_says(&h, &["2, Rip again and combine", "131 taken from rip 2", "rip 2"]));
    assert!(row_says(&h, &["3, Compare with Redump", "Doesn't match"]));
    assert!(h.query_by_label_contains("What the first rip reported").is_none());
    assert!(h.query_all_by_label_contains("Keep the best combination (not verified), ").next().is_some());
    shot(&mut h, "step-check-combined");
    h.get_all_by_label_contains("Rip again and combine again, ").next().expect("again").click_accesskit();
    h.run_steps(2);
    until(&mut h, "the third rip to verify it", |a| a.m.snapshot(a.sel).steps["g_check"].status == "Verified");
    open_id(&mut h, "g_check");
    h.get_by_label("Matches Redump's Lantern Keep (Europe) (Disc 1)");
    assert!(row_says(&h, &["2, Rip again and combine", "140 taken from the other rips", "rips 2 and 3"]));
    assert!(row_says(&h, &["3, Compare with Redump", "Matches"]));
    shot(&mut h, "step-check-combined-verified");
}

/// The disc's error correction, last of the ways to put a copy right:
/// asked first, it repairs what it can and the copy still isn't Redump's,
/// so it says so, not verified; after Fix bad sectors it repairs the one
/// sector left, and the copy is Redump's. Rip's map of the disc marks each
/// sector it repaired with a shape of its own.
#[test]
fn check_repairs_by_the_disc_s_error_correction_last() {
    let held = |a: &App| a.m.snapshot(a.sel).steps["g_check"].items.first().is_some_and(|i| i.state == ItemState::Bad);
    let start = |e: &Env| {
        let mut reg = simulated(None);
        reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::LANTERN_KEEP_DAMAGED]));
        let mut h = window_on(e, reg, true, 0.0, egui::vec2(1280.0, 1100.0), 1.0);
        h.get_by_label_contains("Games, Ready").click();
        h.run_steps(2);
        let i = app(&h).sel;
        app_mut(&mut h).m.set_param(i, "g_check", "if_no_match", Value::from("ask")).unwrap();
        click(&mut h, "Start");
        until(&mut h, "Check to hold the dump", held);
        open_id(&mut h, "g_check");
        h
    };
    let e = env();
    let mut h = start(&e);
    let options: Vec<String> = snap(&h).steps["g_check"].items[0].options.iter().map(|o| o.1.clone()).collect();
    assert_eq!(options, ["Fix bad sectors", "Rip again and combine", "Repair with the disc's error correction", "Keep it anyway", "Skip this disc"], "last of the ways to put it right");
    h.get_all_by_label_contains("Repair with the disc's error correction, ").next().expect("on the dump's row").click_accesskit();
    h.run_steps(2);
    until(&mut h, "the repair", |a| held(a) && a.m.snapshot(a.sel).steps["g_check"].detail.contains("repaired by error correction"));
    h.run_steps(3);
    assert_eq!(snap(&h).steps["g_check"].items[0].note, "Not verified: 137 sectors repaired by the disc's error correction");
    h.get_by_label("Doesn't match Redump");
    h.get_by_label("3 sectors still not put right");
    assert!(row_says(&h, &["2, Repair with error correction", "137 repaired"]));
    // Check's own map: what it took in, and what came of each.
    h.get_by_label("Map of the disc: 199,860 as Rip made them · 137 repaired by error correction · 3 read wrong");
    shot(&mut h, "step-check-repaired-not-verified");
    // Rip's sidebar shows only Rip's own work.
    open_id(&mut h, "g_rip");
    h.run_steps(3);
    assert!(h.query_by_label_contains("Map of the disc: ").is_none(), "Rip made no map here, and Check's isn't drawn as Rip's");
    shot(&mut h, "games-rip-map-repaired");

    let e = env();
    let mut h = start(&e);
    h.get_all_by_label_contains("Fix bad sectors, ").next().expect("on the dump's row").click_accesskit();
    h.run_steps(2);
    until(&mut h, "the fix", |a| held(a) && a.m.snapshot(a.sel).steps["g_check"].detail.contains("re-read"));
    h.get_all_by_label_contains("Repair with the disc's error correction, ").next().expect("still offered, last").click_accesskit();
    h.run_steps(2);
    until(&mut h, "it to verify", |a| a.m.snapshot(a.sel).steps["g_check"].status == "Verified");
    open_id(&mut h, "g_check");
    h.run_steps(3);
    assert_eq!(snap(&h).steps["g_check"].items[0].note, "Verified: Lantern Keep (Europe) (Disc 1), 139 sectors re-read, 1 sector repaired by the disc's error correction");
    h.get_by_label("Matches Redump's Lantern Keep (Europe) (Disc 1)");
    assert!(row_says(&h, &["Read bad sectors again", "139 put right"]) && row_says(&h, &["Repair with error correction", "1 repaired"]));
    shot(&mut h, "step-check-repaired-verified");
    open_id(&mut h, "g_rip");
    h.run_steps(3);
    shot(&mut h, "games-rip-map-repaired-verified");
}

/// A Notify on Rip's path for when it needs you says so the moment Rip
/// can't read the disc and holds it for you, once, in Rip's own words;
/// and the canvas draws that path dashed.
#[test]
fn a_notify_on_rips_needs_you_path_says_at_once() {
    use drt_engine::Side;
    let e = env();
    let mut all = recipes::all();
    for r in all.iter_mut().filter(|r| r.name == "Games") {
        let Some(rip) = r.root.find_mut("g_rip") else { panic!() };
        *rip = rip.clone().on(Side::NeedsYou, Node::step("g_tell_needs", "tell"));
    }
    Store::new(e.dir.path().join("config")).save_recipes(&all).unwrap();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::LANTERN_KEEP_SCRATCHED]));
    let mut h = window_on(&e, reg, true, 1.0, egui::vec2(1280.0, 720.0), 1.0);
    h.get_by_label_contains("Games, Ready").click();
    h.run_steps(2);
    let lay = app(&h).layout(app(&h).sel);
    assert_eq!(lay.side_wires.len(), 1);
    assert_eq!((lay.side_wires[0].from_step.as_str(), lay.side_wires[0].to_step.as_str()), ("g_rip", "g_tell_needs"));
    assert_eq!(app(&h).m.snapshot(app(&h).sel).steps["g_tell_needs"].status, "If rip needs you");
    click(&mut h, "Start");
    until(&mut h, "Rip's Notify to send", |a| a.m.snapshot(a.sel).steps["g_tell_needs"].status == "Sent");
    h.run_steps(5);
    let v = app(&h).m.snapshot(app(&h).sel).steps["g_tell_needs"].clone();
    assert_eq!(v.detail, "Rip needs you: Couldn't read 771 sectors");
    let said: Vec<&str> = v.record.log.iter().map(|l| l.line.as_str()).filter(|l| l.contains("sectors")).collect();
    assert_eq!(said, ["SLES_147.11: Rip needs you: Couldn't read 771 sectors"], "once");
    // On a path, When means nothing: what reaches it is sent as it comes.
    assert!(h.query_by_label_contains("always").is_none(), "no When on a Notify on a path");
    shot(&mut h, "notify-if-rip-needs-you");
}

/// In edit mode each card has a button for each path it can take: one
/// for if it fails, one for if it needs you. Each offers only what takes
/// a message, Notify, and puts it on that path.
#[test]
fn the_buttons_on_a_card_put_a_notify_on_its_fail_and_needs_paths() {
    use drt_engine::Side;
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    assert!(h.query_by_label("If Rip fails").is_none(), "only while editing");
    click(&mut h, "Edit");
    h.get_by_label("If Rip fails").click_accesskit();
    h.run_steps(2);
    assert_eq!(h.get_all_by_label("If Rip fails").count(), 2, "the button, and the list's title");
    let listed: Vec<String> = h.get_all_by_role(Role::ListItem).filter_map(|n| n.accesskit_node().label()).filter(|l| !l.contains(", ")).collect();
    assert_eq!(listed, ["Notify"], "only what takes a message");
    h.get_all_by_role(Role::ListItem).find(|n| n.accesskit_node().label().as_deref() == Some("Notify")).unwrap().click_accesskit();
    h.run_steps(2);
    h.get_by_label("If Rip needs you").click_accesskit();
    h.run_steps(2);
    h.get_all_by_role(Role::ListItem).find(|n| n.accesskit_node().label().as_deref() == Some("Notify")).unwrap().click_accesskit();
    h.run_steps(2);
    let root = app(&h).m.recipes[0].root.clone();
    drt_engine::check(&root, &app(&h).m.engine.registry).unwrap();
    let rip = root.steps().into_iter().find(|n| n.ids()[0] == "rip").unwrap().clone();
    for s in Side::BOTH {
        assert!(matches!(rip.side(s), Some(Node::Step { step, .. }) if step == "tell"), "{s:?}");
    }
    assert!(h.query_by_label("If Rip fails").is_none(), "one path each");
    app_mut(&mut h).popover = None;
    app_mut(&mut h).selected = None;
    h.run_steps(2);
    shot(&mut h, "edit-notify-if-rip-fails-or-needs-you");
    // Removing Rip takes its paths with it.
    let gone = app(&h).m.removal(0, "rip");
    for s in Side::BOTH {
        let id = rip.side(s).unwrap().ids()[0].to_owned();
        assert!(gone.contains(&id), "{id} in {gone:?}");
    }
}

/// A disc the drive can't read in places: the read crawling, the re-read
/// going through the bad sectors, then how many it couldn't read.
#[test]
fn a_scratched_disc_says_how_its_read_and_re_read_go() {
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::LANTERN_KEEP_SCRATCHED]));
    let mut h = window_on(&e, reg, true, 1.0, egui::vec2(1280.0, 900.0), 1.0);
    h.get_by_label_contains("Games, Ready").click();
    h.run_steps(2);
    let live = |a: &App| a.m.snapshot(a.sel).steps["g_rip"].now.clone().and_then(|n| serde_json::from_value::<drt_steps::real::redumper::Live>(n).ok());
    click(&mut h, "Start");
    // The read crawls over the scratch: said on the card and in the
    // sidebar, with Stop.
    until(&mut h, "the read to crawl", |a| live(a).is_some_and(|l| l.crawling && !l.rereading));
    open_id(&mut h, "g_rip");
    h.get_by_label(drt_steps::real::redumper::CRAWLING);
    // Its speed still said, however slow: on the card, and in the sidebar
    // with why.
    let card = snap(&h).steps["g_rip"].detail.clone();
    assert_eq!(card, "The drive can't read parts of this disc · below 0.1×");
    // The card short of room keeps the speed whole and the crawl's start.
    h.get_by_label("Rip, The drive can't read parts of… · below 0.1×, Retries 20");
    h.get_by_label("Slowed by read errors");
    h.get_by_label("Time left unknown · below 0.1×");
    // One Stop: the window's.
    assert_eq!(h.get_all_by_label("Stop").count(), 1, "the crawl warning has no Stop of its own");
    shot(&mut h, "games-rip-crawling");
    // The re-read: how far through the bad sectors, where, which try,
    // what's come back.
    until(&mut h, "the re-read to be some way in", |a| live(a).is_some_and(|l| l.rereading && l.done >= 30 && l.retry >= 19));
    h.run_steps(3);
    let l = live(app(&h)).unwrap();
    assert_eq!((l.total, l.recovered, l.retries), (Some(774), 2, 20));
    h.get_by_label("Re-reading bad parts");
    h.get_by_label_contains("of 774 bad sectors · try 19 of 20");
    h.get_by_label_contains("2 put right so far");
    let card = {
        let a = app(&h);
        a.m.snapshot(a.sel).steps["g_rip"].detail.clone()
    };
    assert!(card.starts_with("Re-reading bad parts: sector 148,") && card.ends_with("2 put right so far · try 19 of 20 · The drive can't read parts of this disc"), "{card}");
    h.get_by_label(drt_steps::real::redumper::CRAWLING);
    shot(&mut h, "games-rip-rereading");
    // It couldn't: how many, and what to try.
    until(&mut h, "Rip to need you", |a| a.m.snapshot(a.sel).steps["g_rip"].items.first().is_some_and(|i| i.state == ItemState::Bad));
    h.run_steps(3);
    h.get_by_label_contains("Couldn't read 771 sectors");
    h.get_by_label("771 sectors couldn't be read");
    // What needs you: why, in a line, and what can still help.
    h.get_by_label("Clean the disc and re-read them, or make the image with them left blank, for Check and fix to try again.");
    h.get_by_label("Re-read bad parts, SLES_147.11");
    h.get_by_label("Make the image anyway, SLES_147.11");
    shot(&mut h, "games-rip-unreadable");
}

/// By itself, as Check is set at first: the card and the sidebar say it's
/// re-reading the bad sectors, then which rip it's on, then that it's
/// verified and how.
#[test]
fn check_fixes_bad_sectors_by_itself_then_rips_again() {
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::LANTERN_KEEP_DAMAGED]));
    // Twice as slow as real time: a slow machine still sees each stage.
    let mut h = window_on(&e, reg, true, 2.0, egui::vec2(1280.0, 1100.0), 1.0);
    h.get_by_label_contains("Games, Ready").click();
    h.run_steps(2);
    open_id(&mut h, "g_check");
    h.get_by_label("If it doesn't match Redump");
    h.get_by_label_contains("Try to fix it by itself");
    app_mut(&mut h).close_tray();
    h.run_steps(2);
    click(&mut h, "Start");
    until(&mut h, "Check to re-read the bad sectors", |a| a.m.snapshot(a.sel).steps["g_check"].status == drt_steps::real::game::REREADING);
    // The sidebar first, while it's at it, then the card.
    open_id(&mut h, "g_check");
    h.get_by_label(drt_steps::real::game::REREADING);
    assert!(row_says(&h, &["2, Read bad sectors again", "Re-reading"]));
    shot(&mut h, "step-check-fixing");
    app_mut(&mut h).close_tray();
    let i = app(&h).sel;
    app_mut(&mut h).set_view(i, (0.9, egui::vec2(-560.0, 40.0)));
    h.run_steps(3);
    h.get_by_label_contains("Check and fix, Reading bad sectors again");
    shot(&mut h, "games-check-fixing");
    until(&mut h, "the second rip", |a| a.m.snapshot(a.sel).steps["g_check"].status == "Rip 2 of 4");
    open_id(&mut h, "g_check");
    h.run_steps(3);
    h.get_by_label("Rip 2 of 4");
    assert!(row_says(&h, &["2, Read bad sectors again", "139 put right", "tries each"]));
    assert!(row_says(&h, &["3, Rip again and combine", "Ripping"]));
    shot(&mut h, "step-check-ripping-again");
    until(&mut h, "it to verify", |a| a.m.snapshot(a.sel).steps["g_check"].status == "Verified");
    open_id(&mut h, "g_check");
    h.get_by_label("Matches Redump's Lantern Keep (Europe) (Disc 1)");
    assert!(row_says(&h, &["3, Rip again and combine", "1 taken from rip 2"]) && row_says(&h, &["4, Compare with Redump", "Matches"]));
    shot(&mut h, "step-check-fixed-verified");
}

/// Check's sector being read again now.
fn rereading(a: &App) -> Option<drt_steps::real::fix::Rereading> {
    a.m.snapshot(a.sel).steps["g_check"].now.clone().and_then(|n| serde_json::from_value(n).ok())
}

fn check_map(a: &App) -> drt_steps::map::SectorMap {
    serde_json::from_value(a.m.snapshot(a.sel).steps["g_check"].record.map.clone().expect("a map")).unwrap()
}

/// Reading bad sectors again, the sidebar and the card say which sector,
/// which try, how far through and what's come back so far, and the map
/// turns each sector put right as it comes back, the one being read
/// marked.
#[test]
fn reading_bad_sectors_again_says_how_it_goes_and_the_map_changes_as_it_goes() {
    use drt_steps::map::Mark;
    let e = env();
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::LANTERN_KEEP_DAMAGED]));
    // Twice as slow as real time: the pass is still going while it's
    // looked at.
    let mut h = window_on(&e, reg, true, 2.0, egui::vec2(1280.0, 1100.0), 1.0);
    h.get_by_label_contains("Games, Ready").click();
    h.run_steps(2);
    click(&mut h, "Start");
    until(&mut h, "a few sectors put right", |a| rereading(a).is_some_and(|r| r.fixed >= 12));
    let first = check_map(app(&h));
    let back = first.total(Mark::Recovered);
    assert!(back > 0 && back < 139, "{back}");
    assert!(first.at.is_some(), "the sector being read is marked");
    assert_eq!(first.total(Mark::Wrong) + back, 140, "each found bad is on the map, bad or put right");
    open_id(&mut h, "g_check");
    h.run_steps(3);
    h.get_by_label(drt_steps::real::game::REREADING);
    h.get_by_label_contains(" of 140 · try ");
    h.get_by_label_contains("put right so far · 1 still bad");
    assert!(h.query_by_label("Time left unknown").is_some() || h.query_all_by_label_contains(" left").any(|n| n.accesskit_node().value().is_some_and(|l| l.starts_with("About "))));
    assert!(row_says(&h, &["2, Read bad sectors again", "Re-reading"]));
    let shown = |h: &Harness<Window>| h.query_by_label_contains("Map of the disc: ").and_then(|n| n.accesskit_node().label()).unwrap_or_default();
    assert!(shown(&h).contains(" put right"), "{}", shown(&h));
    shot(&mut h, "step-check-rereading");
    // Later in the same pass: more put right, on the map too.
    let was = shown(&h);
    until(&mut h, "more put right", |a| rereading(a).is_some_and(|r| r.fixed >= back + 20));
    assert!(check_map(app(&h)).total(Mark::Recovered) >= back + 20);
    assert_ne!(shown(&h), was, "the map in the sidebar changed");
    // The card.
    app_mut(&mut h).close_tray();
    let i = app(&h).sel;
    app_mut(&mut h).set_view(i, (0.9, egui::vec2(-560.0, 40.0)));
    h.run_steps(3);
    h.get_by_label_contains("Check and fix, Reading bad sectors again");
    shot(&mut h, "games-check-rereading");
}

#[test]
fn every_step_has_its_own_icon_and_colour() {
    let p = drt_app::theme::Pal::get(false);
    for (which, reg) in [("real", drt_steps::real()), ("simulated", simulated(None))] {
        for info in reg.infos() {
            assert!(drt_app::theme::has_icon(info.kind), "{which}: {} has no icon of its own", info.kind);
            assert!(p.kind_known(info.kind).is_some(), "{which}: {} has no colour group", info.kind);
        }
    }
    // One not drawn yet still gets something sensible.
    assert!(!drt_app::theme::kind_icon("choose-something", egui::Pos2::ZERO, 24.0, egui::Color32::WHITE).is_empty());
}

fn video_window(e: &Env, disc: drt_steps::real::video::sim::SimDisc) -> Harness<'static, Window> {
    let mut reg = simulated(None);
    drt_steps::real::video::sim::register(&mut reg, disc);
    let mut h = window_on(e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    h.get_by_label_contains("Films and TV,").click();
    h.run_steps(2);
    h
}

#[test]
fn a_season_disc_is_sorted_title_by_title_in_its_sidebar() {
    let e = env();
    let mut h = video_window(&e, drt_steps::real::video::sim::SimDisc::Season);
    let i = app(&h).sel;
    app(&h).m.engine.settings.lock().unwrap().set_app("tmdb", "api_key", Setting::chosen(Value::from("test-key")));
    h.run_steps(2);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    assert!(h.get_all_by_label_contains("Which episode is each video?").next().is_some());
    // The ones it couldn't place first, each with its guess.
    let order: Vec<String> = h.get_all_by_role(Role::ComboBox).filter_map(|n| n.accesskit_node().label().map(|l| l.to_owned())).collect();
    assert_eq!(order.first().map(String::as_str), Some("Video 5: choice"), "{order:?}");
    h.get_by_label_contains("duplicate of video 2");
    shot(&mut h, "step-per-item");
    // The trailer kept, as an extra; the rest as guessed.
    h.get_by_label("Video 6: choice").click_accesskit();
    h.run_steps(2);
    h.get_all_by_label_contains("Extra").next().unwrap().click_accesskit();
    h.run_steps(2);
    assert!(matches!(app(&h).per_item_for_tests().get("title-6"), Some(Some(_))));
    h.get_by_label("Use these").click_accesskit();
    h.run_steps(2);
    until(&mut h, "the end", |a| a.m.snapshot(i).finished);
    let s = app(&h).m.snapshot(i);
    assert_eq!(s.steps["v_choose"].status, "The Sopranos");
    let ripped: Vec<&str> = s.steps["v_rip"].items.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(ripped, ["title-1", "title-2", "title-3", "title-4", "title-6"]);
}

#[test]
fn an_api_key_is_set_once_for_the_app_and_never_shown_whole() {
    let e = env();
    let mut h = video_window(&e, drt_steps::real::video::sim::SimDisc::Film);
    h.get_by_label_contains("TMDb, API key?");
    click_tile(&mut h, "TMDb, API key?");
    h.get_by_label_contains("not set, app-wide");
    input(&h, "API key").click();
    h.run_steps(1);
    h.event(egui::Event::Text("abcd1234efgh5678".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    // Saved for the app, shown by its last four.
    assert_eq!(app(&h).m.engine.settings.lock().unwrap().app("tmdb", "api_key").resolve(None), Some(Value::from("abcd1234efgh5678")));
    assert_eq!(input(&h, "API key").value().as_deref(), Some("••••5678"));
    assert!(h.query_all_by_label_contains("yours").count() > 0 && h.query_all_by_label_contains("app-wide").count() > 0);
    assert!(h.query_by_label_contains("abcd1234").is_none());
    let saved = std::fs::read_to_string(e.dir.path().join("config/settings.json")).unwrap();
    assert!(saved.contains("tmdb.api_key"));
    shot(&mut h, "step-api-key");
    // Its Library names titles the Jellyfin way, a preset of its own.
    open_id(&mut h, "v_lib");
    h.get_by_value("Jellyfin");
    h.get_by_label_contains("Movies/Paprika (2006) [tmdbid-4977]/Paprika (2006)");
    h.get_by_label("Insert {episode}");
    shot(&mut h, "step-video-folder");
    open_id(&mut h, "v_tmdb");
    // Typing replaces it; leaving it empty keeps it.
    input(&h, "API key").click();
    h.run_steps(1);
    assert_eq!(input(&h, "API key").value().as_deref(), Some(""));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(app(&h).m.engine.settings.lock().unwrap().app("tmdb", "api_key").resolve(None), Some(Value::from("abcd1234efgh5678")));
}

#[test]
fn every_pill_is_twenty_pixels_tall() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    // The header's and the sidebar's: one design, one height.
    let side = app(&h).side_rect().unwrap();
    let header = h.get_all_by_label("Needs you").find(|n| n.rect().top() < 50.0).expect("the header's pill").rect();
    let sidebar = h.get_all_by_label("Needs you").find(|n| n.rect().left() > side.left()).expect("the sidebar's pill").rect();
    for r in [header, sidebar] {
        assert!((r.height() - drt_app::widgets::PILL_H).abs() < 0.5, "{r:?}");
    }
}

#[test]
fn the_keyboard_focus_shows_a_ring() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let action = drt_app::theme::Pal::get(false).action;
    let ring = |h: &mut Harness<Window>| {
        let r = h.get_by_label("Edit").rect();
        let img = h.render().unwrap();
        let px = *img.get_pixel((r.left() - 3.0) as u32, r.center().y as u32);
        let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
        d(px[0], action.r()) + d(px[1], action.g()) + d(px[2], action.b()) < 40
    };
    // From the keyboard: Tab, then Edit has the focus, ringed.
    h.key_press(egui::Key::Tab);
    h.get_by_label("Edit").focus();
    h.run_steps(2);
    assert!(ring(&mut h), "a ring round Edit");
    shot(&mut h, "focus-ring");
    // A pointer press: no ring, wherever the focus is.
    let at = app(&h).canvas_rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.get_by_label("Edit").focus();
    h.run_steps(2);
    assert!(!ring(&mut h), "no ring for the pointer");
}

#[test]
fn reduced_motion_turns_animation_off() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // The system's say: DRT_REDUCED_MOTION.
    std::env::set_var("DRT_REDUCED_MOTION", "1");
    assert!(drt_app::motion::reduced_by_system());
    std::env::set_var("DRT_REDUCED_MOTION", "0");
    assert!(!drt_app::motion::reduced_by_system());
    std::env::remove_var("DRT_REDUCED_MOTION");
    // The sidebar opening from shut, a twentieth of a second in.
    let top_after = |h: &mut Harness<Window>, motion: bool| {
        app_mut(h).motion = motion;
        app_mut(h).close_tray();
        h.run_steps(2);
        app_mut(h).open(Tray::Step { id: "flac".into(), focus: None });
        let now = h.ctx.input(|i| i.time);
        h.input_mut().time = Some(now + 0.05);
        h.step();
        // From here, time runs on by itself again.
        h.input_mut().time = None;
        let style = h.ctx.global_style().animation_time;
        let close = h.get_all_by_label("Close").find(|n| n.rect().top() > 50.0).unwrap().rect().left();
        h.run_steps(30);
        let settled = h.get_all_by_label("Close").find(|n| n.rect().top() > 50.0).unwrap().rect().left();
        (close - settled, style)
    };
    let (moving, style) = top_after(&mut h, true);
    assert!(moving > 20.0 && style > 0.0, "with motion it's still sliding in: {moving}");
    let (still, style) = top_after(&mut h, false);
    assert!(still.abs() < 0.5 && style == 0.0, "reduced, it's there at once: {still}");
}

#[test]
fn an_idle_window_asks_for_no_frames_even_with_motion_on() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    app_mut(&mut h).motion = true;
    // The pointer resting on a button, its hover faded in.
    let at = h.get_by_label("Edit").rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(6);
    let delay = h.output().viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
    assert_eq!(delay, Some(std::time::Duration::MAX), "nothing moving, no more frames");
}

#[test]
fn a_drive_unplugged_says_so_calmly_and_comes_back() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // The CDs drive goes away.
    e.plugged.lock().unwrap().retain(|d| d != recipes::BLU_RAY);
    app_mut(&mut h).m.refresh_drives();
    h.run_steps(2);
    h.get_by_label_contains("CDs, Not connected");
    h.get_by_label_contains("Drive, Not connected");
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    assert_eq!(app(&h).m.cant_start(0).as_deref(), Some("Not connected"));
    assert!(h.get_all_by_label("Not connected").any(|n| n.rect().top() < 50.0), "the header says so");
    // Grey, not orange: nothing to do here.
    assert_eq!(snap(&h).steps["drive"].state, StepState::Idle);
    // Its sidebar says so too; the other drive is free.
    click_tile(&mut h, "Drive, Not connected");
    let opts = drive_options(&mut h);
    assert!(opts.iter().any(|l| l == &format!("{} · Not connected", recipes::BLU_RAY)), "{opts:?}");
    assert!(opts.iter().any(|l| l == recipes::DVD), "{opts:?}");
    shot(&mut h, "drive-not-connected");
    // Plugged back in: as before, by itself.
    e.plugged.lock().unwrap().push(recipes::BLU_RAY.into());
    app_mut(&mut h).m.refresh_drives();
    app_mut(&mut h).close_tray();
    h.run_steps(2);
    h.get_by_label_contains("CDs, Ready");
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
    assert!(app(&h).m.cant_start(0).is_none());
}

/// A Folder whose folder can't be made (it's under one nothing can be
/// written in, as a drive's mount point is while it's away) says so on its
/// card and in the list, and Start says why it can't; once the folder is
/// there, Start can.
#[test]
fn a_folder_that_isnt_there_stops_start_until_it_is() {
    use std::os::unix::fs::PermissionsExt;
    let mut e = env();
    e.folders = true;
    let base = e.dir.path().join("lib");
    let away = base.join("away");
    std::fs::create_dir_all(&away).unwrap();
    std::fs::set_permissions(&away, std::fs::Permissions::from_mode(0o555)).unwrap();
    // As root, nothing is read-only: nothing to see.
    if std::fs::write(away.join("x"), "").is_ok() {
        return;
    }
    let mut h = window(&e, None, true, 0.0);
    // Every folder in a folder of the test's; the Library's can't be made.
    for (id, sub) in [("review", "Review"), ("lib2", "MP3")] {
        app_mut(&mut h).m.set_param(0, id, "folder", Value::from(base.join(sub).display().to_string())).unwrap();
    }
    let flac = away.join("FLAC");
    app_mut(&mut h).m.set_param(0, "lib1", "folder", Value::from(flac.display().to_string())).unwrap();
    until(&mut h, "the look", |a| !a.m.unreachable(0).is_empty());
    let why = app(&h).m.cant_start(0).unwrap();
    assert!(why.starts_with("Can't make the folder ") && why.ends_with("FLAC"), "{why}");
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    let v = &snap(&h).steps["lib1"];
    assert_eq!((v.state, v.status.as_str()), (StepState::Needs, "Can't make the folder"));
    h.get_by_label_contains("CDs, Library folder unavailable");
    app_mut(&mut h).select_tile("lib1".into());
    h.run_steps(4);
    shot(&mut h, "folder-unavailable");
    // Start, pressed anyway, looks again and says the same.
    assert_eq!(app_mut(&mut h).m.start(0).unwrap_err().to_string(), why);

    // The folder made: Start can, once the watcher has looked again.
    std::fs::set_permissions(&away, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::create_dir(&flac).unwrap();
    std::fs::set_permissions(&away, std::fs::Permissions::from_mode(0o555)).unwrap();
    app(&h).m.look_again();
    until(&mut h, "the look", |a| a.m.unreachable(0).is_empty());
    assert!(app(&h).m.cant_start(0).is_none(), "{:?}", app(&h).m.cant_start(0));
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
    h.get_by_label_contains("CDs, Ready");
    std::fs::set_permissions(&away, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Not enough room for the rip, in the working folder or a Folder's: said
/// with how much is needed and free, in the list too after a disc.
#[test]
fn not_enough_room_is_said_with_the_sizes() {
    let mut e = env();
    e.folders = true;
    let mut h = window(&e, None, true, 0.0);
    for id in ["review", "lib1", "lib2"] {
        app_mut(&mut h).m.set_param(0, id, "folder", Value::from(e.dir.path().join(id).display().to_string())).unwrap();
    }
    app_mut(&mut h).m.set_param(2, "d_lib", "folder", Value::from(e.dir.path().join("images").display().to_string())).unwrap();
    // A disc of data through first, while there's room.
    app_mut(&mut h).select(2);
    h.run_steps(2);
    app_mut(&mut h).m.start(2).unwrap();
    until(&mut h, "the disc", |a| a.m.newest(2).is_some_and(|r| r.snap.finished));
    // Then the drive with 0.4 GB free.
    let full = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mine = e.dir.path().to_owned();
    let f = full.clone();
    app_mut(&mut h).m.set_reach(Arc::new(move |p: &Path, need| {
        let free: drt_steps::real::reach::FreeFn = if f.load(std::sync::atomic::Ordering::Relaxed) { |_| Ok(Some(400_000_000)) } else { |_| Ok(None) };
        if p.starts_with(&mine) {
            drt_steps::real::reach::look(p, need, free)
        } else {
            Ok(())
        }
    }));
    app_mut(&mut h).m.look_at_folders();
    // Both FLAC folders and the working folder; the MP3s fit.
    until(&mut h, "the look", |a| a.m.unreachable(0).len() == 3 && !a.m.unreachable(2).is_empty());
    let why = app(&h).m.cant_start(0).unwrap();
    assert!(why.starts_with("Not enough room in ") && why.contains(" GB needed, 0.4 GB free"), "{why}");
    // After a disc, the list says so as well as the card.
    h.get_by_label_contains("Data discs, No room in Library");
    assert!(app(&h).m.cant_start(2).unwrap().starts_with("Not enough room"));
    assert_eq!(snap(&h).steps["d_lib"].status, "Not enough room");
    app_mut(&mut h).select(0);
    h.run_steps(2);
    let v = &snap(&h).steps["review"];
    assert_eq!((v.status.as_str(), v.state), ("Not enough room", StepState::Needs));
    assert!(v.detail.ends_with("needed, 0.4 GB free"), "{}", v.detail);
    h.get_by_label_contains("CDs, No room in Review");
    app_mut(&mut h).select_tile("lib1".into());
    h.run_steps(4);
    shot(&mut h, "folder-full");

    // Room again: the watcher's next look clears every recipe, the working
    // folder too though each recipe needs a different amount there.
    full.store(false, std::sync::atomic::Ordering::Relaxed);
    app(&h).m.look_again();
    until(&mut h, "room", |a| a.m.unreachable(0).is_empty() && a.m.unreachable(1).is_empty() && a.m.unreachable(2).is_empty());
    assert!(app(&h).m.cant_start(0).is_none());
}

#[test]
fn the_watcher_notices_a_drive_go_by_itself() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    e.plugged.lock().unwrap().clear();
    // Within a couple of seconds, with no one asking.
    until(&mut h, "the watcher", |a| a.m.unplugged(0));
    h.get_by_label_contains("CDs, Not connected");
}

/// Lookups saved chained one after another each go on a branch from what
/// they look up, when the recipe loads.
#[test]
fn chained_lookups_are_branched_off_what_they_look_up() {
    use drt_engine::recipe::{branches, chain};
    let reg = simulated(None);
    let mut root = chain([
        Node::step("d", "drive"),
        Node::step("cd", "audio-cd"),
        Node::step("mb", "musicbrainz"),
        Node::step("t", "cd-text"),
        Node::step("ch", "choose"),
        Node::step("ly", "lyrics"),
        Node::step("art", "cover-art"),
        Node::step("r", "rip"),
        Node::step("f", "flac"),
    ]);
    assert!(drt_engine::check(&root, &reg).is_err());
    assert!(drt_app::model::migrate(&mut root, &reg));
    let want = chain([
        Node::step("d", "drive"),
        Node::step("cd", "audio-cd"),
        branches([Node::step("mb", "musicbrainz"), Node::step("t", "cd-text")]),
        Node::step("ch", "choose"),
        branches([Node::step("ly", "lyrics"), Node::step("art", "cover-art")]),
        Node::step("r", "rip"),
        Node::step("f", "flac"),
    ]);
    assert_eq!(root, want);
    drt_engine::check(&root, &reg).unwrap();
    // Once is enough.
    assert!(!drt_app::model::migrate(&mut root, &reg));
}

/// A Cue sheet saved where it took the tracks from Rip moves after the
/// Library of the format it named, with the Folder that filed its sheet;
/// Lyrics' old *.lrc files too* becomes a Lyrics file after each Library.
#[test]
fn cue_and_lrc_from_before_move_after_the_libraries() {
    use drt_engine::recipe::{branches, chain};
    let reg = simulated(None);
    let old = |cue: Node| {
        chain([
            Node::step("d", "drive"),
            Node::step("cd", "audio-cd"),
            Node::step("mb", "musicbrainz"),
            Node::step("ch", "choose"),
            branches([Node::step("ly", "lyrics").with("lrc", true), Node::step("art", "cover-art")]),
            Node::step("r", "rip"),
            branches([
                chain([
                    Node::step("f", "flac"),
                    Node::step("rev", "folder").labelled("Review"),
                    // As a Hold upgrade leaves it: a chain of its own.
                    chain([Node::step("h", "hold"), Node::step("lib1", "library")]),
                    branches([Node::step("nas", "folder").labelled("NAS")]),
                ]),
                chain([Node::step("m", "mp3"), Node::step("lib2", "folder").labelled("Library")]),
                chain([cue, Node::step("arc", "folder").labelled("Archive")]),
            ]),
            Node::step("t", "tell"),
        ])
    };
    let mut root = old(Node::step("c", "cue").with("ext", "mp3").with("names", "{track}"));
    assert!(drt_engine::check(&root, &reg).is_err(), "Cue no longer takes tracks");
    assert!(drt_app::model::migrate(&mut root, &reg));
    let want = chain([
        Node::step("d", "drive"),
        Node::step("cd", "audio-cd"),
        Node::step("mb", "musicbrainz"),
        Node::step("ch", "choose"),
        branches([Node::step("ly", "lyrics"), Node::step("art", "cover-art")]),
        Node::step("r", "rip"),
        branches([
            chain([
                Node::step("f", "flac"),
                Node::step("rev", "folder").labelled("Review"),
                Node::step("h", "hold"),
                Node::step("lib1", "folder").labelled("Library"),
                branches([Node::step("nas", "folder").labelled("NAS"), Node::step("lyricsfile", "lyrics-file")]),
            ]),
            chain([
                Node::step("m", "mp3"),
                Node::step("lib2", "folder").labelled("Library"),
                branches([chain([Node::step("c", "cue"), Node::step("arc", "folder").labelled("Archive")]), Node::step("lyricsfile2", "lyrics-file")]),
            ]),
        ]),
        Node::step("t", "tell"),
    ]);
    // The old Cue's settings went with it; only *.lrc files too* had to go
    // from Lyrics, so it isn't migrated again.
    let Some(Node::Step { params, .. }) = root.find_mut("ly") else { panic!() };
    assert!(params.is_empty(), "{params:?}");
    assert_eq!(root, want);
    drt_engine::check(&root, &reg).unwrap();
    drt_engine::check(&root, &drt_steps::real()).unwrap();
    assert!(!drt_app::model::migrate(&mut root, &reg), "once is enough");

    // No format named: FLAC's, as the old default was. *.lrc files too*
    // off adds nothing, and its old value is just ignored.
    let mut root = old(Node::step("c", "cue"));
    if let Some(Node::Step { params, .. }) = root.find_mut("ly") {
        params.insert("lrc".into(), Setting::chosen(Value::from(false)));
    }
    assert!(drt_app::model::migrate(&mut root, &reg));
    drt_engine::check(&root, &reg).unwrap();
    assert!(!root.ids().iter().any(|i| i.starts_with("lyricsfile")));
    let json = serde_json::to_string(&root).unwrap();
    assert_eq!(json.matches("\"cue\"").count(), 1, "one Cue: {json}");
    assert!(json.find("\"lib1\"").unwrap() < json.find("\"c\"").unwrap() && json.find("\"c\"").unwrap() < json.find("\"m\"").unwrap(), "after the FLAC library: {json}");

    // One encoder beside the old Cue: once the Cue's line goes, its
    // branches are one line, and the Cue still goes beside what follows
    // the Library, never in line before it.
    let mut root = chain([
        Node::step("d", "drive"),
        Node::step("cd", "audio-cd"),
        Node::step("mb", "musicbrainz"),
        Node::step("ch", "choose"),
        Node::step("r", "rip"),
        branches([chain([Node::step("f", "flac"), Node::step("lib", "folder")]), chain([Node::step("c", "cue"), Node::step("arc", "folder")])]),
        Node::step("t", "tell"),
    ]);
    assert!(drt_app::model::migrate(&mut root, &reg));
    let want = chain([
        Node::step("d", "drive"),
        Node::step("cd", "audio-cd"),
        Node::step("mb", "musicbrainz"),
        Node::step("ch", "choose"),
        Node::step("r", "rip"),
        Node::step("f", "flac"),
        Node::step("lib", "folder"),
        branches([Node::step("t", "tell"), chain([Node::step("c", "cue"), Node::step("arc", "folder")])]),
    ]);
    assert_eq!(root, want);
    drt_engine::check(&root, &reg).unwrap();
}

/// Runs the CDs recipe to its end: the second release, the hold let go.
fn run_cd_to_the_end(h: &mut Harness<Window>) {
    click(h, "Start");
    until(h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(h, "Use this one");
    until(h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    click_tile(h, "Hold,");
    h.get_by_label("Let 17 through").click();
    h.run_steps(2);
    until(h, "the end", |a| a.m.snapshot(0).finished);
}

/// Whether the old record tabs are gone (Items, Found).
fn no_tabs(h: &Harness<Window>) -> bool {
    h.query_all_by_role(Role::Tab).all(|n| !["Items", "Found"].contains(&n.accesskit_node().label().as_deref().unwrap_or("")))
}

/// CD-Text's sidebar says nothing of MusicBrainz, nor of how a later step
/// used what it gave: MusicBrainz is named only on its own tile.
fn says_nothing_of_others(h: &Harness<Window>) {
    for n in h.query_all_by_label_contains("") {
        let l = n.accesskit_node().label().unwrap_or_default();
        assert!(!l.contains("not used") && !l.contains(" won"), "{l}");
        if l.contains("MusicBrainz") {
            assert!(n.accesskit_node().role() == Role::Button && l.starts_with("MusicBrainz, "), "{l}");
        }
    }
}

#[test]
fn a_steps_sidebar_shows_what_it_found_and_what_it_offered() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // Before a run there's nothing to tell: no Progress, no Log.
    open_id(&mut h, "cdtext");
    assert!(tab_on(&h).is_none() && h.query_by_label("Log").is_none());
    app_mut(&mut h).close_tray();
    run_cd_to_the_end(&mut h);
    // CD-Text: what it offered: the disc's own text, as one candidate.
    // Nothing of what Choose later did with it.
    open_id(&mut h, "cdtext");
    assert!(no_tabs(&h));
    h.get_by_label_contains("From the disc (CD-Text), THE WHITE STRIPES — THE WHITE STRIPES, 17 track names, CD-Text");
    says_nothing_of_others(&h);
    shot(&mut h, "record-found");
    // Choose: what reached it, what it chose, and why.
    open_id(&mut h, "choose");
    h.get_by_label_contains("you picked");
    h.get_by_label_contains("From the disc (CD-Text), ");
    h.get_by_label_contains("MusicBrainz · 1999, US");
    shot(&mut h, "record-choose");
    // FLAC: its rows, and the files it wrote.
    open_id(&mut h, "flac");
    h.get_by_label_contains("01.flac");
    shot(&mut h, "record-files");
    // Rip: its log on Log, the tool's own output folded beneath.
    open_id(&mut h, "rip");
    assert_eq!(tab_on(&h).as_deref(), Some("Progress"));
    tab(&mut h, "Log");
    h.get_by_label("Tool output").click_accesskit();
    h.run_steps(2);
    h.get_by_label_contains("Tool output lines");
    shot(&mut h, "record-log");
    // Kept only while the app is open: a new window starts without them.
    drop(h);
    let mut h = window(&e, None, true, 0.0);
    open_id(&mut h, "cdtext");
    assert!(h.query_by_label("Log").is_none(), "no record before a run");
    assert!(!e.dir.path().join("runs").exists(), "nothing written for a run");
}

/// A step that fails: logs why, then stops with an error.
struct Failing {
    inner: std::sync::Arc<dyn drt_engine::Step>,
}

#[async_trait::async_trait]
impl drt_engine::Step for Failing {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, _outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        inlet.recv().await;
        cx.log("GnuDB did not answer within 10 s");
        anyhow::bail!("no answer")
    }
}

#[test]
fn a_failed_step_opens_on_its_log() {
    let e = env();
    // GnuDB with no answer is skipped, not failed: this one fails.
    let mut reg = simulated(None);
    let inner = reg.get("gnudb").unwrap().0;
    reg.add(Failing { inner });
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    h.get_by_label_contains("Archive everything,").click();
    h.run_steps(2);
    click(&mut h, "Start");
    until(&mut h, "GnuDB to fail", |a| a.m.snapshot(a.sel).steps.get("a_gn").is_some_and(|v| v.state == StepState::Bad));
    open_id(&mut h, "a_gn");
    // It opens on its Log.
    assert_eq!(tab_on(&h).as_deref(), Some("Log"));
    h.get_by_label_contains("did not answer");
}

#[test]
fn the_table_of_contents_shows_no_titles_before_choose() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    // Audio CD read the disc's tracks (and its CD-Text, as the disc says
    // it, for the CD-Text step to offer); titles are the lookups', and
    // they meet at Choose.
    // It opens on its tracks, each as long as it plays, the disc's shape
    // above them.
    open_id(&mut h, "cd");
    assert!(no_tabs(&h));
    h.get_by_label("1, Track 1");
    h.get_by_label("The disc's tracks, by length");
    assert!(h.query_by_label_contains("Jimmy").is_none() && h.query_by_label_contains(", title ").is_none());
}

// ---- cards -------------------------------------------------------------------------------------

/// Every card in the window as it's built: its step's view and what it
/// shows besides, fitted at 100 % with the window's own fonts.
fn cards(h: &mut Harness<Window>) -> Vec<(String, drt_app::card::Card)> {
    let ctx = h.ctx.clone();
    let s = snap(h);
    let extras = app_mut(h).cards(&ctx, &s);
    let painter = egui::Painter::new(ctx, egui::LayerId::background(), egui::Rect::EVERYTHING);
    let measure = drt_app::card::measurer(&painter);
    s.steps
        .iter()
        .map(|(id, v)| {
            let x = &extras[id];
            let inp = drt_app::card::Input { v, settings: &x.settings, locked: x.locked, cover: !matches!(x.picture, drt_app::card::Picture::None), asking: x.asking, retries: x.retries };
            (id.clone(), drt_app::card::build(&inp, &measure))
        })
        .collect()
}

#[test]
fn every_card_has_its_five_slots_in_the_same_place() {
    use drt_app::card::{self, Cell};
    let e = env();
    let mut h = window(&e, None, true, 0.03);
    // Every tile is one card size, at 100 %.
    let lay = app(&h).layout(0);
    assert!(lay.tiles.iter().all(|t| (t.rect.w, t.rect.h) == (card::W, card::H)));
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "ripping", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Ok) >= 4);
    for (id, c) in cards(&mut h) {
        // 1: one name line; 2: at most two settings rows; 3: an outcome of
        // at most two lines, ending on the lower third line; 4: one detail
        // line; 5: the strip on the bottom margin, the slot's whole width.
        let rows: usize = c.rows.iter().map(|r| r.value.len()).sum();
        assert!(rows <= 2, "{id}: {rows} settings rows");
        assert!(c.outcome.len() <= 2, "{id}: {:?}", c.outcome);
        assert!(!c.detail.contains('\n'), "{id}");
        let strip = c.strip();
        // A step the disc hasn't reached is a blueprint: no strip yet.
        if c.plan {
            assert!(strip.is_empty() && c.outcome.is_empty() && c.detail.is_empty(), "{id}: a blueprint is its name and settings");
            continue;
        }
        assert!(!strip.is_empty(), "{id}: every card that has run has a strip");
        let x0 = if c.cover { card::BESIDE_COVER } else { card::M };
        assert!((strip[0].0 - x0).abs() < 0.01 && (strip.last().unwrap().1 - (card::W - card::M)).abs() < 0.01, "{id}: {strip:?}");
        assert_eq!(c.text_x(), x0, "{id}");
        // Only Cover art moves slots 3 to 5, beside its picture.
        assert_eq!(c.cover, snap(&h).steps[&id].step == "cover-art", "{id}");
        // The state is in the glyph and the outline: no cell colour is a tint.
        assert!(c.cells.iter().all(|x| *x != Cell::Out || snap(&h).steps[&id].count(ItemState::Skipped) > 0 || c.shown == card::Shown::Skipped), "{id}");
    }
    // The slots are where the spec puts them, on the card's 16 px grid.
    assert_eq!((card::NAME, card::SETTINGS, card::OUT_BOTTOM, card::DETAIL, card::STRIP), ((16.0, 32.0), [(32.0, 48.0), (48.0, 64.0)], 128.0, (136.0, 152.0), (168.0, 176.0)));
}

#[test]
fn the_item_strip_has_one_cell_per_item() {
    use drt_app::card::Cell;
    let e = env();
    let mut h = window(&e, Some(9), true, 0.0);
    // Before a run: blueprints, with no strip; only the Drive speaks.
    for (id, c) in cards(&mut h) {
        if id == "drive" {
            assert_eq!(c.cells, vec![Cell::Wait], "{id}");
        } else {
            assert!(c.plan && c.cells.is_empty(), "{id}");
        }
    }
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "a damaged track", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 1 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 16);
    let s = snap(&h);
    let all = cards(&mut h);
    let get = |id: &str| &all.iter().find(|(x, _)| x == id).unwrap().1;
    // Rip: a cell per track, verified green, the one to decide orange.
    let rip = get("rip");
    assert_eq!(rip.cells.len(), s.steps["rip"].items.len());
    assert_eq!(rip.cells.iter().filter(|c| **c == Cell::Verified).count(), 16);
    assert_eq!(rip.cells[8], Cell::Needs, "track 9 needs you");
    assert_eq!(rip.shown, drt_app::card::Shown::Needs);
    // Audio CD: the disc's tracks; a lookup, the disc as a whole.
    assert_eq!(get("cd").cells, vec![Cell::Done; 17]);
    assert_eq!(get("mb").cells, vec![Cell::Done]);
    // An encoder: its tracks, those still to come waiting.
    let flac = get("flac");
    assert_eq!(flac.cells.len(), 17);
    assert_eq!(flac.cells.iter().filter(|c| **c == Cell::Done).count(), s.steps["flac"].count(ItemState::Ok));
}

#[test]
fn short_settings_and_outcomes_are_never_cut_and_long_ones_shorten_smartly() {
    use drt_app::card::{self, CardSetting};
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    // Every built-in step's real contents, whole; where a detail of
    // several parts can't fit, its first part (the catalogue number) gives
    // way before any word is cut, and the hover has it all.
    for (id, c) in cards(&mut h) {
        let label = |x: &String| id == "choose" && x.starts_with("SFTRI 577 · ");
        assert!(c.cut.iter().all(label), "{id}: cut {:?}", c.cut);
        if id == "choose" {
            assert_eq!(c.detail, "Sympathy for the Record Industry");
        }
    }
    // The spec's examples, at the default size: never cut.
    let ctx = h.ctx.clone();
    let painter = egui::Painter::new(ctx, egui::LayerId::background(), egui::Rect::EVERYTHING);
    let measure = card::measurer(&painter);
    let mut v = snap(&h).steps["flac"].clone();
    let build = |v: &drt_engine::StepView, sets: &[CardSetting], cover: bool| card::build(&card::Input { v, settings: sets, locked: true, cover, asking: false, retries: false }, &measure);
    let set = |l: &str, val: &str, path: bool| CardSetting { label: l.into(), value: val.into(), yours: true, path, help: None };
    for (status, q, detail) in [
        ("The White Stripes", "1999", "Sympathy · SFTRI 577"),
        ("Colony Wars (Europe) (Disc 1)", "", "Track 6 · Wasting My Time"),
        ("16/17 verified", "1 kept", "Track 9 doesn't match AccurateRip"),
        ("294 MB", "58 %", "Encarta 98 Encyclopedia (USA) (Disc 1)"),
        ("Colony Wars.m3u", "2 discs", "The White Stripes / The White Stripes"),
    ] {
        (v.status, v.qualifier, v.detail) = (status.into(), q.into(), detail.into());
        let sets = [set("Folder", "~/Music/FLAC", true), set("Names", "Artist / Album / 01 - Title", false)];
        let c = build(&v, &sets, false);
        assert!(c.cut.is_empty(), "{status}: cut {:?}", c.cut);
        let c = build(&v, &[set("Quality", "V0 · joint stereo", false), set("Video", "H.265 CRF 22", false)], false);
        assert!(c.cut.is_empty(), "{status}: cut {:?}", c.cut);
    }
    // Long ones: a deep path keeps both ends; a long title wraps to two
    // lines, then keeps its start; the full text goes in the tooltip.
    v.status = "The Rise and Fall of Ziggy Stardust and the Spiders from Mars".into();
    v.qualifier = "1972".into();
    v.detail = "David Bowie / The Rise and Fall of Ziggy Stardust and the Spiders from Mars".into();
    let c = build(&v, &[set("Folder", "/mnt/nas/media/music/lossless/FLAC", true)], false);
    assert_eq!(c.outcome.len(), 2, "{:?}", c.outcome);
    assert!(c.outcome[0].starts_with("The Rise and Fall"), "{:?}", c.outcome);
    assert!(c.outcome[1].ends_with('…'), "{:?}", c.outcome);
    assert_eq!(c.qualifier, "1972", "the qualifier is never cut");
    assert!(c.rows[0].value[0].starts_with("/mnt/") && c.rows[0].value[0].ends_with("/FLAC") && c.rows[0].value[0].contains('…'), "{:?}", c.rows[0]);
    assert!(c.detail.starts_with("David Bowie / ") && c.detail.ends_with('…'), "{}", c.detail);
    for full in ["/mnt/nas/media/music/lossless/FLAC", "The Rise and Fall of Ziggy Stardust and the Spiders from Mars", "David Bowie / The Rise and Fall of Ziggy Stardust and the Spiders from Mars"] {
        assert!(c.cut.iter().any(|x| x == full), "{full} in the tooltip");
    }
    // A time left keeps its place when the title before it gives way.
    v.detail = "The Rise and Fall of Ziggy Stardust and the Spiders from Mars · 12 min left".into();
    let c = build(&v, &[], false);
    assert!(c.detail.ends_with(" · 12 min left") && c.detail.contains('…'), "{}", c.detail);
    // Beside a cover, the same rules in less room.
    v.status = "1200 × 1200".into();
    v.qualifier.clear();
    v.detail.clear();
    assert!(build(&v, &[set("Cover size", "Large", false)], true).cut.is_empty());
}

#[test]
fn a_setting_you_changed_has_its_dot_and_all_are_locked_while_ripping() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let ctx = h.ctx.clone();
    let level = |h: &mut Harness<Window>| {
        let s = snap(h);
        let c = app_mut(h).cards(&ctx, &s);
        (c["flac"].settings[0].clone(), c["flac"].locked)
    };
    // The default: no dot, no lock.
    let (s, locked) = level(&mut h);
    assert_eq!((s.label.as_str(), s.value.as_str(), s.yours, locked), ("Level", "8", false, false));
    // Yours: a dot.
    let i = app(&h).sel;
    app_mut(&mut h).m.set_param(i, "flac", "level", Value::from(5)).unwrap();
    h.run_steps(2);
    let (s, _) = level(&mut h);
    assert_eq!((s.value.as_str(), s.yours), ("5", true));
    // Back to the default value: no dot, though you set it.
    app_mut(&mut h).m.set_param(i, "flac", "level", Value::from(8)).unwrap();
    h.run_steps(2);
    assert!(!level(&mut h).0.yours);
    // While ripping, every card's settings are locked.
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    let s = snap(&h);
    let all = app_mut(&mut h).cards(&ctx, &s);
    assert!(all.values().all(|c| c.locked));
    // The card shows the drive and its offset, the Library its folder and names.
    let said = |k: &str| all[k].settings.iter().map(|x| format!("{} {}", x.label, x.value)).collect::<Vec<_>>();
    assert_eq!(said("drive"), ["Drive Blu-ray drive", "Offset +103"]);
    assert_eq!(said("lib1"), ["Folder ~/Music/FLAC", "Names Artist / Album / 01 - Title"]);
    assert_eq!(said("mp3"), ["Quality V0", "Stereo joint"]);
    click(&mut h, "Stop");
    until(&mut h, "the stop", |a| a.m.snapshot(0).finished);
    let s = snap(&h);
    assert!(app_mut(&mut h).cards(&ctx, &s).values().all(|c| !c.locked), "unlocked once it's done");
    // A folder the recipe came with isn't yours; one you change is.
    let folder = |h: &mut Harness<Window>| {
        let s = snap(h);
        app_mut(h).cards(&ctx, &s)["lib1"].settings[0].clone()
    };
    assert!(!folder(&mut h).yours, "as built in");
    app_mut(&mut h).m.set_param(i, "lib1", "folder", Value::from("~/Music/Lossless")).unwrap();
    h.run_steps(2);
    let f = folder(&mut h);
    assert_eq!((f.value.as_str(), f.yours), ("~/Music/Lossless", true));
    // The sidebar agrees: yours, with a Reset.
    open_id(&mut h, "lib1");
    h.get_by_label("Reset Folder");
}

// ---- the sidebar --------------------------------------------------------------------------------

/// The card's settings as the canvas shows them, for one step.
fn card_settings(h: &mut Harness<Window>, id: &str) -> Vec<drt_app::card::CardSetting> {
    let ctx = h.ctx.clone();
    let s = snap(h);
    app_mut(h).cards(&ctx, &s).remove(id).unwrap().settings
}

#[test]
fn a_click_before_a_run_opens_the_steps_settings_in_the_sidebar() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let canvas = app(&h).canvas_rect();
    click_tile(&mut h, "FLAC,");
    assert!(matches!(&app(&h).tray, Some(Tray::Step { id, .. }) if id == "flac"));
    let d = app(&h).side_rect().unwrap();
    // On the right, under the header, the canvas narrowed beside it and
    // as tall as before.
    assert!((d.right() - 1280.0).abs() < 1.0 && (d.bottom() - 720.0).abs() < 1.0 && d.top() < 60.0, "{d:?}");
    let c = app(&h).canvas_rect();
    assert!((c.right() - d.left()).abs() < 1.0 && (c.height() - canvas.height()).abs() < 1.0, "{c:?} beside {d:?}");
    h.get_by_role(Role::Slider);
    // The card it opened from stays whole beside it.
    let card = h.get_all_by_role(Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("FLAC,"))).unwrap().rect();
    assert!(card.right() <= d.left(), "the card glided out from under it: {card:?}");
    // A changed value: applied at once, its dot on the card, Reset beside.
    assert!(!card_settings(&mut h, "flac")[0].yours);
    app_mut(&mut h).m.set_param(0, "flac", "level", Value::from(5)).unwrap();
    h.run_steps(2);
    let set = card_settings(&mut h, "flac");
    assert!(set[0].yours && set[0].value == "5", "{:?}", set[0].value);
    h.get_by_label_contains("Reset");
    h.get_by_label_contains("yours");
    click(&mut h, "Reset Compression");
    assert!(!card_settings(&mut h, "flac")[0].yours);
    assert!(h.query_by_label_contains("Reset").is_none(), "Reset only on a changed value");
}

#[test]
fn esc_and_a_click_on_the_empty_canvas_close_the_sidebar() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click_tile(&mut h, "FLAC,");
    assert!(app(&h).tray.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app(&h).tray.is_none(), "Esc closed it");
    click_tile(&mut h, "MP3,");
    assert!(app(&h).tray.is_some());
    // Below the cards, where nothing is.
    let c = app(&h).canvas_rect();
    let at = egui::pos2(c.left() + 40.0, c.bottom() - 80.0);
    h.hover_at(at);
    h.step();
    h.event(press(at, true));
    h.step();
    h.event(press(at, false));
    h.run_steps(3);
    assert!(app(&h).tray.is_none(), "a click on the canvas closed it: {:?} at {at:?} in {c:?}", app(&h).tray);
}

#[test]
fn start_closes_the_sidebar_and_while_running_it_shows_the_run_locked() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click_tile(&mut h, "FLAC,");
    click(&mut h, "Start");
    assert!(!matches!(app(&h).tray, Some(Tray::Step { .. })), "Start closed it");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    // The question opens it by itself, in the sidebar.
    h.get_by_label("Use this one");
    assert!(app(&h).side_rect().is_some());
    // Mid-run a card opens on its run; its settings can't change.
    click_tile(&mut h, "FLAC,");
    if tab_on(&h).is_some() {
        tab(&mut h, "Settings");
    }
    assert!(h.query_by_role(Role::Slider).is_none());
    h.get_by_label_contains("Compression");
    h.get_by_label("Locked while ripping");
}

#[test]
fn the_drive_is_always_a_dropdown_with_one_drive_or_two() {
    let e = env();
    e.plugged.lock().unwrap().retain(|d| d == recipes::BLU_RAY);
    let mut h = window(&e, None, true, 0.0);
    click_tile(&mut h, "Drive,");
    // One drive: still a dropdown, so it can be changed later.
    h.get_all_by_role(Role::ComboBox).next().expect("the drive dropdown, first");
    let opts = drive_options(&mut h);
    assert_eq!(opts, [recipes::BLU_RAY.to_owned()], "{opts:?}");
    h.get_all_by_role(Role::ComboBox).next().expect("the drive dropdown, first").click_accesskit();
    h.run_steps(2);
    // Just its name: nothing said about there being only one.
    assert!(h.query_all_by_label_contains("Blu-ray drive").next().is_some());
    assert!(h.query_by_label_contains("only drive").is_none());
    // The offset AccurateRip knows: as text, with where it's from; no box.
    h.get_by_label("+103 · AccurateRip");
    assert!(h.query_all_by_role(Role::TextInput).next().is_none());
    assert!(h.query_by_label("Measure offset").is_none());
    drop(h);
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click_tile(&mut h, "Drive,");
    h.get_all_by_role(Role::ComboBox).next().expect("the drive dropdown, first");
    let opts = drive_options(&mut h);
    assert_eq!(opts.len(), 2, "{opts:?}");
}

#[test]
fn a_missing_folder_opens_the_sidebar_at_the_folder() {
    let e = env();
    seed_without_folder(&e);
    let mut h = window(&e, None, true, 0.0);
    click_tile(&mut h, "Library, Folder?");
    assert!(matches!(&app(&h).tray, Some(Tray::Step { id, focus: Some(f) }) if id == "lib2" && f == "folder"));
    h.run_steps(2);
    // The box has the keys: typing goes straight in.
    let focused = h.get_all_by_role(Role::TextInput).filter(|n| n.accesskit_node().is_focused()).count();
    assert_eq!(focused, 1, "the folder's box is focused");
    h.get_by_label_contains("not set");
    let d = app(&h).side_rect().unwrap();
    assert!(d.contains_rect(first(&h, "Folder")));
}

#[test]
fn a_track_that_needs_you_is_answered_in_the_sidebar() {
    let e = env();
    let mut h = window(&e, Some(9), true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "a damaged track", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 1 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 16);
    click_tile(&mut h, "Rip,");
    // Its outcome once, then the rows, the one needing you first.
    h.get_by_label("The disc, 16/17 verified");
    let rows: Vec<String> = h.get_all_by_role(Role::ListItem).filter_map(|n| n.accesskit_node().label()).filter(|l| l.starts_with(|c: char| c.is_ascii_digit())).collect();
    assert!(rows[0].starts_with("9, Broken Bricks"), "{rows:?}");
    assert!(rows[1].starts_with("1, ") && rows.iter().any(|r| r.ends_with("Verified 104")), "each row its proof, in track order: {rows:?}");
    h.get_by_label("AccurateRip");
    click(&mut h, "Keep it anyway, Broken Bricks");
    until(&mut h, "kept", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 0);
}

#[test]
fn which_release_widens_the_sidebar_for_the_question_only() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    h.run_steps(3);
    let wide = app(&h).side_rect().unwrap().width();
    assert!(wide > 600.0, "{wide}");
    click(&mut h, "Use this one");
    h.run_steps(3);
    click_tile(&mut h, "FLAC,");
    assert!((app(&h).side_rect().unwrap().width() - 448.0).abs() < 1.0);
}

/// The sidebar's shots, at one window size: `sidebar-<what>[-1920]`.
fn sidebar_shots(size: egui::Vec2, tag: &str) {
    let name = |n: &str| format!("sidebar-{n}{tag}");
    // One drive, then two.
    let e = env();
    e.plugged.lock().unwrap().retain(|d| d == recipes::BLU_RAY);
    let mut h = window_at(&e, None, true, 0.0, size, 1.0);
    open_tile(&mut h, "Drive,");
    shot(&mut h, &name("drive-one"));
    drop(h);
    let e = env();
    let mut h = window_at(&e, Some(9), true, 0.02, size, 1.0);
    open_tile(&mut h, "Drive,");
    shot(&mut h, &name("drive-two"));
    // FLAC changed from its default; Library's names.
    app_mut(&mut h).m.set_param(0, "flac", "level", Value::from(5)).unwrap();
    open_tile(&mut h, "FLAC,");
    shot(&mut h, &name("flac"));
    open_tile(&mut h, "Library,");
    shot(&mut h, &name("library"));
    open_tile(&mut h, "MP3,");
    shot(&mut h, &name("mp3"));
    app_mut(&mut h).close_tray();
    // Asking which release.
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    h.run_steps(3);
    shot(&mut h, &name("choose-asking"));
    click(&mut h, "Use this one");
    until(&mut h, "ripping", |a| {
        let r = &a.m.snapshot(0).steps["rip"];
        r.count(ItemState::Ok) >= 5 && r.count(ItemState::Busy) == 1
    });
    open_id(&mut h, "rip");
    shot(&mut h, &name("rip-running"));
    until(&mut h, "a damaged track", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 1 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 16);
    open_id(&mut h, "rip");
    shot(&mut h, &name("rip-retry"));
    click(&mut h, "Keep it anyway, Broken Bricks");
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    open_id(&mut h, "hold");
    h.get_all_by_role(Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("Let "))).unwrap().click_accesskit();
    until(&mut h, "the end", |a| a.m.snapshot(0).finished);
    open_id(&mut h, "flac");
    shot(&mut h, &name("flac-done"));
    open_id(&mut h, "rip");
    shot(&mut h, &name("rip-done"));
    open_id(&mut h, "lib1");
    shot(&mut h, &name("library-done"));
    drop(h);
    // A missing key, asked for.
    let e = env();
    let mut h = video_window(&e, drt_steps::real::video::sim::SimDisc::Film);
    h.set_size(size);
    h.run_steps(3);
    click_tile(&mut h, "TMDb, API key?");
    shot(&mut h, &name("api-key"));
}

#[test]
fn every_kind_of_sidebar() {
    sidebar_shots(egui::vec2(1280.0, 720.0), "");
    sidebar_shots(egui::vec2(1920.0, 1080.0), "-1920");
}

#[test]
fn the_sidebar_opens_on_the_tab_that_fits() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // Before a run: settings only, no tab bar.
    open_id(&mut h, "flac");
    assert!(tab_on(&h).is_none());
    h.get_by_role(Role::Slider);
    run_cd_to_the_end(&mut h);
    // After: Progress first, with Settings and Log beside it; settings
    // flat, nothing folded.
    open_id(&mut h, "flac");
    assert_eq!(tab_on(&h).as_deref(), Some("Progress"));
    assert!(h.query_by_role(Role::Slider).is_none());
    tab(&mut h, "Settings");
    h.get_by_role(Role::Slider);
    // Asked for a value: Settings, whatever was picked before.
    app_mut(&mut h).open(Tray::Step { id: "flac".into(), focus: Some("level".into()) });
    h.run_steps(3);
    assert_eq!(tab_on(&h).as_deref(), Some("Settings"));
}

#[test]
fn open_folder_opens_where_the_files_went_and_the_picker_starts_at_the_folder() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    open_id(&mut h, "lib1");
    // Before anything is filed: the base folder; the picker starts there
    // (or the nearest parent that's there).
    let base = drt_steps::real::expand("~/Music/FLAC");
    click(&mut h, "Choose folder");
    until(&mut h, "the picker", |_| !e.picked_from.lock().unwrap().is_empty());
    let from = e.picked_from.lock().unwrap()[0].clone().unwrap();
    assert!(base.starts_with(&from) && from.is_dir(), "{from:?} for {base:?}");
    app_mut(&mut h).m.set_param(0, "lib1", "folder", Value::from(e.dir.path().join("lib").display().to_string())).unwrap();
    run_cd_to_the_end(&mut h);
    open_id(&mut h, "lib1");
    e.opened.lock().unwrap().clear();
    click(&mut h, "Open folder");
    let opened = e.opened.lock().unwrap().clone();
    let files = snap(&h).steps["lib1"].record.files.clone();
    assert!(!files.is_empty());
    // The album's folder, which holds every file it filed.
    assert!(opened[0] != e.dir.path().join("lib") && opened[0].starts_with(e.dir.path().join("lib")), "{opened:?}");
    assert!(files.iter().all(|f| drt_steps::real::expand(&f.path).parent() == Some(opened[0].as_path())), "{opened:?} {:?}", files[0]);
}

#[test]
fn short_text_in_the_sidebar_is_never_cut_at_1280() {
    let e = env();
    let mut h = window(&e, Some(9), true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "a damaged track", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 1 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 16);
    let mut cut = vec![];
    let mut look = |h: &mut Harness<Window>, id: &str| {
        open_id(h, id);
        cut.extend(app(h).cut_titles.iter().filter(|t| t.chars().count() < 40).map(|t| format!("{id}: {t}")));
    };
    for id in ["cd", "cdtext", "mb", "choose", "rip", "flac", "review", "mp3"] {
        look(&mut h, id);
    }
    app(&h).m.send(0, drt_engine::Command::Retry { step: "rip".into(), item: "track-9".into(), option: "keep".into() });
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    for id in ["hold", "review"] {
        look(&mut h, id);
    }
    assert!(cut.is_empty(), "cut with room to spare: {cut:?}");
}

// ---- links, lyrics, and Rip's pace ---------------------------------------------------------

const MBIDS: [&str; 2] = ["9b4f7c1e-2d3a-4e5f-8a6b-1c2d3e4f5a6b", "3f2e1d0c-b9a8-4765-8432-10fedcba9876"];

/// MusicBrainz as the real one offers: releases by their MBIDs.
struct TestMb {
    inner: std::sync::Arc<dyn drt_engine::Step>,
}

#[async_trait::async_trait]
impl drt_engine::Step for TestMb {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        use drt_engine::{Candidate, Fact, Subject};
        while let Some(mut disc) = inlet.recv().await {
            let release = |id: &str, title: &str, detail: &str, year: &str, score: f32| {
                let fact = |subject, key: &str, value: &str| Fact { step: String::new(), subject, key: key.into(), value: value.into(), source: "MusicBrainz".into() };
                let mut facts = vec![
                    fact(Subject::Disc, "album", "The White Stripes"),
                    fact(Subject::Disc, "album_artist", "The White Stripes"),
                    fact(Subject::Disc, "year", year),
                    fact(Subject::Disc, "musicbrainz_albumid", id),
                ];
                for (k, (t, len)) in drt_steps::sim::TRACKS.iter().enumerate() {
                    facts.push(fact(Subject::Part(k as u32 + 1), "title", t));
                    facts.push(fact(Subject::Part(k as u32 + 1), "length", &len.to_string()));
                }
                Candidate { step: String::new(), id: id.into(), source: "MusicBrainz".into(), title: title.into(), detail: detail.into(), score, facts }
            };
            disc.context_mut().offer([
                release(MBIDS[0], "The White Stripes — The White Stripes", "1999, US, Sympathy for the Record Industry, SFTRI 577", "1999", 0.9),
                release(MBIDS[1], "The White Stripes — The White Stripes (reissue)", "2002, UK, XL Recordings, XLCD 151", "2002", 0.9),
            ]);
            cx.status("2 releases", "by disc ID");
            outlet.send(disc).await;
        }
        Ok(())
    }
}

/// Lyrics as LRCLIB gives them: synced, plain, or none.
struct TestLyrics {
    inner: std::sync::Arc<dyn drt_engine::Step>,
}

const SYNCED: &str = "[ar:The White Stripes]\n[00:12.40]Well, I was walking down the road\n[00:16.10]With my head held to the sky\n[00:20.85]And a long and winding verse that goes on well past the edge of any sidebar you could draw\n[00:25.00]\n[00:27.30]Jimmy, Jimmy, Jimmy";

#[async_trait::async_trait]
impl drt_engine::Step for TestLyrics {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        use drt_engine::Subject;
        while let Some(mut disc) = inlet.recv().await {
            for t in 1..=disc.context().parts() {
                let title = disc.context().get(Subject::Part(t), "title").unwrap_or("").to_owned();
                let track = drt_engine::Item::track(t, title);
                match t {
                    3 => cx.item(&track, ItemState::Skipped, "none"),
                    2 => {
                        disc.context_mut().add(Subject::Part(t), "lyrics", "Stop breaking down\nPlease stop breaking down\n\nThe stuff I got'll bust your brains out", "LRCLIB");
                        disc.context_mut().add(Subject::Part(t), "lyrics.synced", "no", "LRCLIB");
                        cx.item(&track, ItemState::Ok, "plain");
                    }
                    _ => {
                        disc.context_mut().add(Subject::Part(t), "lyrics", SYNCED, "LRCLIB");
                        disc.context_mut().add(Subject::Part(t), "lyrics.synced", "yes", "LRCLIB");
                        cx.item(&track, ItemState::Ok, "synced");
                    }
                }
            }
            cx.status("16/17 synced", "none for 3");
            outlet.send(disc).await;
        }
        Ok(())
    }
}

/// CDs with Lyrics beside Cover art, on the test steps.
fn links_window(e: &Env, size: egui::Vec2) -> Harness<'static, Window> {
    use drt_engine::recipe::{branches, chain};
    let mut reg = simulated(None);
    let png = e.dir.path().join("cover.png");
    write_cover(&png);
    let mb = reg.get("musicbrainz").unwrap().0;
    reg.add(TestMb { inner: mb });
    let ly = reg.get("lyrics").unwrap().0;
    reg.add(TestLyrics { inner: ly });
    let art = reg.get("cover-art").unwrap().0;
    reg.add(TestCover { inner: art, png });
    let mut r = recipes::cd();
    r.root = chain([
        Node::step("drive", "drive").with("drive", recipes::BLU_RAY),
        Node::step("cd", "audio-cd"),
        branches([Node::step("mb", "musicbrainz"), Node::step("cdtext", "cd-text")]),
        Node::step("choose", "choose"),
        branches([Node::step("art", "cover-art"), Node::step("lyrics", "lyrics")]),
        Node::step("rip", "rip"),
        Node::step("flac", "flac"),
    ]);
    drt_engine::check(&r.root, &reg).unwrap();
    std::fs::create_dir_all(e.dir.path().join("config")).unwrap();
    std::fs::write(e.dir.path().join("config/recipes.json"), serde_json::to_string(&vec![r]).unwrap()).unwrap();
    window_on(e, reg, true, 0.0, size, 1.0)
}

fn opened_last(e: &Env) -> String {
    e.opened.lock().unwrap().last().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

fn links_and_lyrics(size: egui::Vec2, tag: &str) {
    let e = env();
    let mut h = links_window(&e, size);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    h.run_steps(3);
    // Each option links to its release on MusicBrainz; following one
    // doesn't pick it.
    let picked = app(&h).pick;
    click(&mut h, "Open on MusicBrainz, The White Stripes — The White Stripes (reissue)");
    assert_eq!(opened_last(&e), format!("https://musicbrainz.org/release/{}", MBIDS[1]));
    assert_eq!(app(&h).pick, picked, "the link doesn't pick it");
    shot(&mut h, &format!("links-choose{tag}"));
    click(&mut h, "Use this one");
    until(&mut h, "the lyrics", |a| a.m.snapshot(0).steps["lyrics"].state == StepState::Ok && a.m.snapshot(0).steps["art"].state == StepState::Ok);
    // The lookup's own list, and the decision.
    open_id(&mut h, "mb");
    click(&mut h, "Open on MusicBrainz, The White Stripes — The White Stripes");
    assert_eq!(opened_last(&e), format!("https://musicbrainz.org/release/{}", MBIDS[0]));
    open_id(&mut h, "choose");
    h.get_by_label("Open on MusicBrainz, The White Stripes — The White Stripes");
    h.get_by_label("Open on MusicBrainz, The White Stripes — The White Stripes (reissue)");
    shot(&mut h, &format!("links-chosen{tag}"));
    // The cover: the release's images in the archive.
    open_id(&mut h, "art");
    click(&mut h, "Open on Cover Art Archive");
    assert_eq!(opened_last(&e), format!("https://musicbrainz.org/release/{}/cover-art", MBIDS[0]));
    // Lyrics: every row whole; one with none says so; one opens in place.
    open_id(&mut h, "lyrics");
    h.run_steps(2);
    assert!(app(&h).cut_titles.is_empty(), "nothing cut: {:?}", app(&h).cut_titles);
    h.get_by_label_contains("3, The Big Three Killed My Baby").accesskit_node().label().unwrap().contains("No lyrics found").then_some(()).expect("says it has none");
    assert!(h.query_by_label_contains("Well, I was walking").is_none(), "closed at first");
    let row =
        |h: &Harness<Window>| h.get_all_by_role(Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("1, Jimmy the Exploder"))).unwrap().accesskit_node().label().unwrap();
    assert!(row(&h).ends_with("Synced, closed"), "{}", row(&h));
    h.get_all_by_role(Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("1, Jimmy the Exploder"))).unwrap().click();
    h.run_steps(3);
    assert!(row(&h).ends_with(", open"), "{}", row(&h));
    // Whole, with its times; the tag that isn't a time left out.
    h.get_by_label("Well, I was walking down the road");
    h.get_by_label("And a long and winding verse that goes on well past the edge of any sidebar you could draw");
    assert!(h.query_by_label_contains("ar:The White").is_none());
    shot(&mut h, &format!("step-lyrics-open{tag}"));
    // And closes again.
    h.get_all_by_role(Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("1, Jimmy the Exploder"))).unwrap().click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("Well, I was walking").is_none());
}

#[test]
fn lookups_link_to_their_entries_and_lyrics_open_in_place() {
    links_and_lyrics(egui::vec2(1280.0, 720.0), "");
}

#[test]
fn lookups_link_to_their_entries_and_lyrics_open_in_place_at_1920() {
    links_and_lyrics(egui::vec2(1920.0, 1080.0), "-1920");
}

/// Rip as the real one reports: each track's percentage as it reads.
/// Track 2 moves on by 8.2× over three seconds, then waits.
struct TestRip {
    inner: std::sync::Arc<dyn drt_engine::Step>,
    release: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl drt_engine::Step for TestRip {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, _outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        use drt_engine::Subject;
        let Some(disc) = inlet.recv().await else { return Ok(()) };
        let n = disc.context().parts();
        let title = |t: u32| disc.context().get(Subject::Part(t), "title").unwrap_or("").to_owned();
        let mut items: Vec<drt_engine::Item> = (1..=n).map(|t| drt_engine::Item::track(t, title(t)).knowing(&disc)).collect();
        for it in &items {
            cx.item(it, ItemState::Waiting, "");
        }
        drt_steps::real::rip_verdict(&cx, &mut items[0], ItemState::Ok, "✓ 104");
        let len: f32 = disc.context().get(Subject::Part(2), "length").and_then(|l| l.parse().ok()).unwrap();
        drt_steps::real::rip_says(&cx, format!("Track 2 · {}", items[1].label));
        cx.item(&items[1], ItemState::Busy, "reading 30%");
        tokio::time::sleep(Duration::from_millis(3200)).await;
        let p = 30.0 + 100.0 * 8.2 * 3.2 / len;
        cx.item(&items[1], ItemState::Busy, format!("reading {p:.0}%"));
        // A percentage changing beside it: 9, then 10.
        cx.item(&items[2], ItemState::Busy, "re-reading 9%");
        while !self.release.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        for it in items.iter_mut().skip(1) {
            drt_steps::real::rip_verdict(&cx, it, ItemState::Ok, "✓ 104");
        }
        drt_steps::real::rip_done(&cx);
        Ok(())
    }
}

fn rip_pace(size: egui::Vec2, tag: &str) {
    let e = env();
    let mut reg = simulated(None);
    let inner = reg.get("rip").unwrap().0;
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    reg.add(TestRip { inner, release: release.clone() });
    let mut h = window_on(&e, reg, true, 0.0, size, 1.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "track 2 under way", |a| a.m.snapshot(0).steps["rip"].items.iter().any(|i| i.note == "re-reading 9%"));
    h.run_steps(3);
    // The card: the track, its time left, the disc's, the speed.
    let card = h.get_by_label_contains("Rip, ").accesskit_node().label().unwrap();
    let line = card.split(", ").find(|s| s.starts_with("Track 2 · ")).unwrap_or_else(|| panic!("{card}")).to_owned();
    let parts: Vec<&str> = line.split(" · ").collect();
    assert_eq!(parts.len(), 4, "{line}");
    assert!(parts[1].ends_with(" left") && parts[1].contains(':'), "the track's time: {line}");
    assert!(parts[2].starts_with("disc ") && parts[2].ends_with(" min left"), "the disc's: {line}");
    let speed: f32 = parts[3].strip_suffix('×').unwrap().parse().unwrap();
    assert!((7.6..=8.8).contains(&speed), "about 8.2×: {line}");
    // The disc: what's left of track 2 and all of 4 to 17 at that speed.
    let lens: Vec<f32> = drt_steps::sim::TRACKS.iter().map(|t| t.1 as f32).collect();
    let want = (lens[1] * 0.5 + lens[3..].iter().sum::<f32>()) / speed / 60.0;
    let mins: f32 = parts[2].trim_start_matches("disc ").trim_end_matches(" min left").parse().unwrap();
    assert!((mins - want).abs() <= 2.0, "{mins} min, want about {want}");
    // The sidebar says the same, under the track.
    open_id(&mut h, "rip");
    h.run_steps(2);
    let times = format!("{} · {} · {}", parts[1], parts[2], parts[3]);
    h.get_by_label(&times);
    shot(&mut h, &format!("step-rip-reading{tag}"));
    // Once read: how long it took, and at what speed.
    release.store(true, std::sync::atomic::Ordering::SeqCst);
    until(&mut h, "the read", |a| a.m.snapshot(0).steps["rip"].state == StepState::Ok);
    h.run_steps(3);
    open_id(&mut h, "rip");
    h.run_steps(2);
    let said: Vec<String> = h.query_all_by_label_contains("Read in ").filter_map(|n| n.accesskit_node().label()).collect();
    assert!(said.iter().any(|l| l.starts_with("Rip, 17/17 verified, Read in ") && l.contains(" at ")), "the card: {said:?}");
    let read = said[0].split(", ").find(|s| s.starts_with("Read in ")).unwrap().to_owned();
    assert!(read.ends_with('×'), "{read}");
    h.get_by_label(&read);
    shot(&mut h, &format!("step-rip-read{tag}"));
}

#[test]
fn rip_shows_the_time_left_for_the_track_and_the_disc_and_its_speed() {
    rip_pace(egui::vec2(1280.0, 720.0), "");
}

#[test]
fn rip_shows_the_time_left_for_the_track_and_the_disc_and_its_speed_at_1920() {
    rip_pace(egui::vec2(1920.0, 1080.0), "-1920");
}

// ---- Rip for game, data and video discs ------------------------------------------------------

/// Where a held Rip stops, to be looked at.
#[derive(Clone, Copy, Debug, PartialEq)]
enum At {
    /// redumper's first pass under way, with errors.
    Reading,
    /// Read, its bad parts re-read.
    Done,
    /// Sectors left unreadable: it needs you.
    Failed,
    /// Read cleanly at the first go: no errors anywhere.
    Clean,
    /// Read with ~200 errors spread from 10% to 70%, re-read, and 25
    /// sectors found wrong between 47% and 68%: a disc like Koudelka's.
    Wrong,
    /// As [`At::Wrong`], its read errors coming back once per turn at two
    /// single spots about 4.4 and 4.9 cm out, as Koudelka's do.
    Spots,
    /// As [`At::Reading`], the read slowed by errors to a crawl.
    Crawling,
    /// Read, re-read, and made with 34 sectors it couldn't read left
    /// blank, handed on for Check and fix to try: as its settings say.
    HandedOn,
}

/// Where [`slips`] puts its `n` places with errors, spread from `from` to
/// `to` (shares of the disc), bunched towards the middle; in order. Every
/// third the drive couldn't read; the others had C2 errors.
fn slip_places(n: u64, from: f64, to: f64) -> Vec<u64> {
    let mut seed = 0x2545_f491u64;
    let mut places: Vec<u64> = (0..n)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            // Bunched towards the middle of the stretch: two draws averaged.
            let a = (seed >> 33) as f64 / (1u64 << 31) as f64;
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let b = (seed >> 33) as f64 / (1u64 << 31) as f64;
            ((from + (to - from) * (a * 0.4 + b * 0.6)) * CD_SECTORS as f64) as u64
        })
        .collect();
    places.sort_unstable();
    places
}

/// A held Rip's map of the disc, as Image would make it from redumper's
/// `.state`: the places [`slips`] logged, and a spot met once per turn
/// (as a scratch is, 2% off the nominal turn, as on a real disc), read up
/// to `upto`; re-read and put right once `reread`, all but `left` of the
/// spot's sectors.
fn held_map(n: u64, from: f64, to: f64, upto: u64, reread: bool, left: usize) -> drt_steps::map::SectorMap {
    use drt_steps::map::{Geometry, Mark, Sectors};
    let mut s = Sectors::new(CD_SECTORS);
    for k in 0..upto.min(CD_SECTORS) {
        s.set(k, Mark::Good);
    }
    let fixed = |m: Mark| if reread { Mark::Recovered } else { m };
    for (k, at) in slip_places(n, from, to).into_iter().enumerate().filter(|(_, at)| *at < upto) {
        s.set(at, fixed(if k % 3 == 0 { Mark::Unreadable } else { Mark::C2 }));
    }
    if n > 0 {
        let start = (0.3 * CD_SECTORS as f64) as u64;
        let turn = Geometry::CD.per_turn(start as f64) * 1.02;
        let spot: Vec<u64> = (0..30).filter(|k| k % 7 != 3).map(|k| start + (k as f64 * turn).round() as u64).collect();
        for (j, at) in spot.iter().enumerate().filter(|(_, at)| **at < upto) {
            for x in [*at, at + 1] {
                s.set(x, if j >= spot.len() - left && x == *at { Mark::Unreadable } else { fixed(Mark::Unreadable) });
            }
        }
    }
    s.map((upto < CD_SECTORS).then_some(upto), true)
}

/// A Rip's map of a disc like Koudelka's: the places [`slips`] logged, and
/// two spots met once per turn (2% off the nominal turn), about 4.4 and
/// 4.9 cm out; all re-read and put right.
fn spots_map() -> drt_steps::map::SectorMap {
    use drt_steps::map::{Geometry, Mark, Sectors};
    let mut s = Sectors::new(CD_SECTORS);
    for k in 0..CD_SECTORS {
        s.set(k, Mark::Good);
    }
    for at in slip_places(200, 0.10, 0.70) {
        s.set(at, Mark::Recovered);
    }
    for start in [148_500u64, 201_200] {
        let turn = Geometry::CD.per_turn(start as f64) * 1.02;
        for k in (0..30).filter(|k| k % 7 != 3) {
            let at = start + (k as f64 * turn).round() as u64;
            s.set(at, Mark::Recovered);
            s.set(at + 1, Mark::Recovered);
        }
    }
    s.map(None, true)
}

/// The sectors of the disc [`At::HandedOn`] reads: as many as Koudelka
/// (Europe) (Disc 4) has.
const DISC_4: u64 = 269_159;
/// The sectors it can't read, left blank for Check and fix.
const HANDED_ON: u64 = 34;

/// Rip's map of a disc with two bad patches, each a spot met once per
/// turn (2% off the nominal turn) about 4.4 and 4.9 cm out, as Koudelka
/// (Europe) (Disc 4)'s: re-read, and all but [`HANDED_ON`] put right. With
/// the sectors left unreadable.
fn handed_map() -> (drt_steps::map::SectorMap, Vec<u64>) {
    use drt_steps::map::{Geometry, Mark, Sectors};
    let mut s = Sectors::new(DISC_4);
    for k in 0..DISC_4 {
        s.set(k, Mark::Good);
    }
    let mut blank = vec![];
    for start in [148_500u64, 201_200] {
        let turn = Geometry::CD.per_turn(start as f64) * 1.02;
        let bad: Vec<u64> = (0..160u64).filter(|k| k % 7 != 3).flat_map(|k| (0..5).map(move |x| x + start + (k as f64 * turn).round() as u64)).collect();
        for at in &bad {
            s.set(*at, Mark::Recovered);
        }
        blank.extend(bad.iter().rev().step_by(9).take(HANDED_ON as usize / 2));
    }
    for at in &blank {
        s.set(*at, Mark::Unreadable);
    }
    blank.sort_unstable();
    (s.map(None, true), blank)
}

/// How a held Check and fix ends: verified, or holding the copy for you.
#[derive(Clone, Copy, PartialEq)]
enum CheckAt {
    Verified,
    Holding,
    /// Read again, stopped as nothing came back, and held.
    Stopped,
}

/// Check and fix as the real one reports a copy Rip handed on with
/// [`HANDED_ON`] sectors blank, held where it ends: read again, then
/// ripped again and combined, until it matches Redump's copy, or it
/// can't. With the times each took, its map of those sectors, and
/// Redump's copy beside this one.
struct HeldCheck {
    at: CheckAt,
    held: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl drt_engine::Step for HeldCheck {
    fn info(&self) -> drt_engine::StepInfo {
        drt_steps::real::data::check_info()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        use drt_steps::map::{Mark, SectorMap};
        use drt_steps::real::mismatch::{Combination, File, Fixing, Mismatch, Pair, Read, RipRead};
        let Some(dump) = inlet.recv().await else { return Ok(()) };
        let done = self.at == CheckAt::Verified;
        let stopped = self.at == CheckAt::Stopped;
        let blank = handed_map().1;
        let file = |sha1: &str| File { name: "SLES_147.11.bin".into(), size: DISC_4 * 2352, sha1: sha1.into() };
        let left = if done { 0 } else { 3 };
        let rips = if done { 2 } else { 4 };
        let m = Mismatch {
            serial: "SLES-14711".into(),
            entry: "Lantern Keep (Europe) (Disc 2)".into(),
            files: vec![Pair {
                expected: Some(file("3a64731400000000000000000000000000000000")),
                got: Some(file(if done { "3a64731400000000000000000000000000000000" } else { "91c0d2e500000000000000000000000000000000" })),
            }],
            read: Some(Read { drive: "ACME DVD-RW XY123".into(), generic: true, read_offset: Some(0), scsi: 19_992, disc_sectors: DISC_4, ..Read::default() }),
            can_combine: true,
            fixed: Some(if stopped {
                Fixing {
                    bad: 1_226,
                    fixed: 9,
                    left: 1_217,
                    left_at: (0..10).map(|k| 150_000 + k * 7).collect(),
                    tries: 10,
                    secs: Some(3_130),
                    stalled: 3,
                    given_up: true,
                    not_tried: 1_100,
                    ..Fixing::default()
                }
            } else {
                Fixing { bad: HANDED_ON, fixed: 31, left: 3, left_at: blank[31..].iter().map(|l| *l as i64).collect(), tries: 10, secs: Some(250), ..Fixing::default() }
            }),
            combined: (!stopped).then_some(Combination {
                rips: (1..=rips).map(|n| RipRead { n, bad: if n == 1 || !done { 3 } else { 0 } }).collect(),
                sectors: DISC_4,
                differing: 3,
                taken: if done { 3 } else { 0 },
                no_good: left,
                still_bad: left,
                left: blank[34 - left as usize..].iter().map(|l| *l as i64).collect(),
                verified: done,
                secs: Some(if done { 2_462 } else { 7_304 }),
                ..Combination::default()
            }),
            ..Mismatch::default()
        };
        let found: Vec<(u64, Mark)> = if stopped {
            // A long scratch: the first few put right, the rest still bad.
            (0..1_226u64).map(|k| (150_000 + k * 7, if k < 9 { Mark::Recovered } else { Mark::Wrong })).collect()
        } else {
            blank.iter().enumerate().map(|(k, l)| (*l, if done || k < 31 { Mark::Recovered } else { Mark::Unreadable })).collect()
        };
        cx.item(&dump, ItemState::Busy, "checking");
        cx.map(serde_json::to_value(SectorMap::checked(DISC_4, &found, true)).ok());
        cx.report(serde_json::to_value(&m).ok());
        if done {
            cx.verified(&dump, "Verified: Lantern Keep (Europe) (Disc 2), 31 sectors re-read, 3 from a second rip");
            cx.status("Verified", "31 sectors re-read, 3 from a second rip");
            self.held.store(true, std::sync::atomic::Ordering::SeqCst);
            outlet.send(dump).await;
            return Ok(());
        }
        cx.status("Doesn't match Redump", m.short());
        cx.state(StepState::Bad);
        cx.item_options(&dump, m.options());
        cx.item(&dump, ItemState::Bad, "Tried by itself (31 sectors re-read, 3 more rips); still doesn't match Redump");
        self.held.store(true, std::sync::atomic::Ordering::SeqCst);
        cx.cancelled().await;
        Ok(())
    }
}

/// The window with Games started on a scratched disc, Rip held at `at`
/// and, once Rip hands it on, Check and fix held at `check`; the sidebar
/// of `open` open.
fn held_game<'a>(e: &Env, at: At, check: Option<CheckAt>, open: &str, size: egui::Vec2) -> Harness<'a, Window> {
    let mut reg = simulated(None);
    reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::LANTERN_KEEP_SCRATCHED]));
    let rip = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let checked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    reg.add(HeldRip { info: drt_steps::real::game::rip_info(), at, held: rip.clone() });
    reg.add(HeldCheck { at: check.unwrap_or(CheckAt::Verified), held: checked.clone() });
    let mut h = window_on(e, reg, true, 0.0, size, 1.0);
    h.get_by_label_contains("Games,").click();
    h.run_steps(2);
    click(&mut h, "Start");
    until(&mut h, "Rip held", |_| rip.load(std::sync::atomic::Ordering::SeqCst));
    if at == At::HandedOn {
        until(&mut h, "Rip to finish", |a| a.m.snapshot(a.sel).steps["g_rip"].state == StepState::Ok);
    }
    if check.is_some() {
        until(&mut h, "Check held", |_| checked.load(std::sync::atomic::Ordering::SeqCst));
    }
    h.run_steps(3);
    open_id(&mut h, open);
    h.run_steps(3);
    h
}

/// redumper's first read of a CD as Image logs it, its progress lines fed
/// through the real trail: `n` places with errors, spread from `from` to
/// `to` (shares of the disc), stopping at `upto`. Gives the counts at the end.
fn slips(cx: &drt_engine::Cx, n: u64, from: f64, to: f64, upto: u64) -> (u64, u64) {
    use drt_steps::real::redumper;
    let mut trail = redumper::ErrorTrail::new();
    trail.line("*** DUMP (time check: 0s)");
    cx.log("*** DUMP (time check: 0s)");
    let places = slip_places(n, from, to);
    let (mut scsi, mut c2, mut next) = (0u64, 0u64, 0usize);
    let mut at = 0u64;
    while at <= upto {
        while next < places.len() && places[next] <= at {
            if next % 3 == 0 {
                scsi += 588;
            } else {
                c2 += 40 + (next as u64 * 7) % 200;
            }
            next += 1;
        }
        if let Some(l) = trail.line(&redumper::progress_line(at, CD_SECTORS, true, scsi, c2)) {
            cx.log(l);
        }
        at += 150;
    }
    if let Some(l) = trail.finish() {
        cx.log(l);
    }
    (scsi, c2)
}

/// The sectors `info` lists wrong on a disc like Koudelka's: 25, from 47%
/// to 68%, some side by side.
fn koudelka_wrong() -> String {
    let at = |pc: f64| (pc / 100.0 * CD_SECTORS as f64) as u64;
    let mut list: Vec<String> = vec![];
    for (k, pc) in [47.2, 49.0, 49.1, 52.5, 55.0, 55.05, 57.3, 58.0, 60.2, 61.0, 61.02, 62.4, 63.3, 64.0, 65.1, 66.0, 66.4, 67.2, 67.9].iter().enumerate() {
        let s = at(*pc);
        list.push(if k % 3 == 1 { format!("{s}-{}", s + 1) } else { s.to_string() });
    }
    format!("EDC errors: 25 (LBA: {})", list.join(", "))
}

/// Image, a game's Rip or a video Rip as the real ones report (their notes,
/// redumper's and MakeMKV's lines), held at one moment.
struct HeldRip {
    info: drt_engine::StepInfo,
    at: At,
    /// Set once it's there.
    held: Arc<std::sync::atomic::AtomicBool>,
}

const CD_SECTORS: u64 = 332_999;

impl HeldRip {
    /// The first pass, from sector `from` for `secs` at 8× (600 sectors a
    /// second), as redumper prints it and Image reports it; its heading
    /// already logged.
    async fn read(cx: &drt_engine::Cx, item: &drt_engine::Item, from: u64, secs: f32, scsi: u64, c2: u64) {
        use drt_steps::real::redumper;
        cx.status("", "Reading");
        let mut at = from;
        for k in 0..=(secs * 4.0) as u32 {
            if k > 0 {
                tokio::time::sleep(Duration::from_millis(250)).await;
                at += 150;
            }
            let line = redumper::progress_line(at, CD_SECTORS, true, scsi, c2);
            cx.raw(&line);
            let p = redumper::parse_progress(&line).unwrap();
            cx.item(item, ItemState::Busy, redumper::progress_note(&p));
            cx.progress(Some(p.fraction));
        }
    }

    async fn dump(&self, disc: &drt_engine::Item, outlet: &drt_engine::Outlet, cx: &drt_engine::Cx) -> anyhow::Result<()> {
        use drt_engine::{Item, ItemKind};
        let item = Item::new(drt_steps::real::data::DUMP, ItemKind::Dump, disc.label.clone(), None, None).knowing(disc);
        cx.log(format!("redumper dump --drive=/dev/sr0 --retries=20 --image-name={}", disc.label));
        match self.at {
            At::Reading | At::Crawling => {
                // Errors met so far, up to where it is, then reading on.
                slips(cx, 160, 0.10, 0.70, 141_500);
                cx.map(serde_json::to_value(held_map(160, 0.10, 0.70, 141_500 + 150 * 18, false, 0)).ok());
                Self::read(cx, &item, 141_500, 4.5, 2, 37).await;
                if self.at == At::Crawling {
                    let live = drt_steps::real::redumper::Live { at: 144_200, of: CD_SECTORS, crawling: true, ..Default::default() };
                    cx.now(serde_json::to_value(live).ok());
                }
            }
            At::HandedOn => {
                cx.map(serde_json::to_value(handed_map().0).ok());
                for l in [
                    "*** DUMP (time check: 0s)",
                    "media errors:",
                    "SCSI: 790272 samples",
                    "C2: 0 samples",
                    "Q: 0",
                    "*** REFINE (time check: 0s)",
                    "correction statistics:",
                    "SCSI: 770280 samples",
                    "C2: 0 samples",
                    "Q: 0 sectors",
                    "*** END (time check: 1s)",
                    // Stopped on the sectors it couldn't read; then, as
                    // Rip's settings say, made anyway with them blank.
                    "*** SPLIT (time check: 0s)",
                    "errors detected, track: 1, sectors: {SKIP: 34, C2: 0}, samples: {SKIP: 19992, C2: 0}",
                    "error: data errors detected, unable to continue",
                    "image: 34 unreadable sectors; making it anyway, as Rip's settings say, with them left blank for Check and fix to read again",
                    "*** SPLIT (time check: 0s)",
                    "errors detected, track: 1, sectors: {SKIP: 34, C2: 0}, samples: {SKIP: 19992, C2: 0}",
                    "*** END (time check: 1s)",
                    "*** INFO (time check: 0s)",
                    "*** END (time check: 1s)",
                ] {
                    cx.log(l);
                }
                let note = drt_steps::real::data::made_anyway_note(HANDED_ON);
                cx.item(&item, ItemState::Ok, note.clone());
                cx.status("633 MB", note);
                let mut made = item.becomes(ItemKind::Dump, Some(PathBuf::from(format!("{}.cue", disc.label))));
                made.context_mut().add(drt_engine::Subject::Disc, drt_steps::real::data::UNREADABLE, HANDED_ON.to_string(), "Rip");
                outlet.send(made).await;
            }
            At::Clean => {
                slips(cx, 0, 0.0, 1.0, CD_SECTORS);
                cx.map(serde_json::to_value(held_map(0, 0.0, 1.0, CD_SECTORS, false, 0)).ok());
                for l in ["media errors:", "SCSI: 0 samples", "C2: 0 samples", "Q: 0", "*** SPLIT (time check: 0s)", "*** INFO (time check: 0s)", "*** END (time check: 1s)"] {
                    cx.log(l);
                }
                cx.item(&item, ItemState::Ok, "✓ 682 MB, read cleanly");
                cx.status("682 MB", "");
                outlet.send(item.becomes(ItemKind::Dump, Some(PathBuf::from(format!("{}.cue", disc.label))))).await;
            }
            At::Wrong | At::Spots => {
                let (scsi, c2) = slips(cx, 200, 0.10, 0.70, CD_SECTORS);
                let map = if self.at == At::Spots { spots_map() } else { held_map(200, 0.10, 0.70, CD_SECTORS, true, 0) };
                cx.map(serde_json::to_value(map).ok());
                for l in [
                    "media errors:".to_owned(),
                    format!("SCSI: {scsi} samples"),
                    format!("C2: {c2} samples"),
                    "Q: 0".into(),
                    "*** REFINE (time check: 52s)".into(),
                    "correction statistics:".into(),
                    format!("SCSI: {scsi} samples"),
                    format!("C2: {} samples", c2 - 300),
                    "Q: 0 sectors".into(),
                    "*** SPLIT (time check: 0s)".into(),
                    "*** INFO (time check: 0s)".into(),
                    koudelka_wrong(),
                    koudelka_wrong().replace("EDC", "ECC"),
                    "*** END (time check: 1s)".into(),
                ] {
                    cx.log(l);
                }
                cx.item(&item, ItemState::Ok, "read, but 25 sectors read wrong");
                cx.status("682 MB", "");
                outlet.send(item.becomes(ItemKind::Dump, Some(PathBuf::from(format!("{}.cue", disc.label))))).await;
            }
            At::Done | At::Failed => {
                slips(cx, 120, 0.15, 0.55, CD_SECTORS);
                cx.map(serde_json::to_value(held_map(120, 0.15, 0.55, CD_SECTORS, false, 0)).ok());
                for l in ["media errors:", "SCSI: 105840 samples", "C2: 211731 samples", "Q: 2641", "*** END (time check: 348s)"] {
                    cx.log(l);
                }
                cx.status("", "Re-reading bad parts");
                cx.log("*** REFINE (time check: 52s)");
                tokio::time::sleep(Duration::from_millis(1100)).await;
                cx.map(serde_json::to_value(held_map(120, 0.15, 0.55, CD_SECTORS, true, if self.at == At::Done { 0 } else { 3 })).ok());
                if self.at == At::Done {
                    for l in [
                        "correction statistics:",
                        "SCSI: 105840 samples",
                        "C2: 211731 samples",
                        "Q: 0 sectors",
                        "*** END (time check: 52s)",
                        "*** SPLIT (time check: 0s)",
                        "*** END (time check: 12s)",
                        "*** INFO (time check: 0s)",
                        "*** END (time check: 1s)",
                    ] {
                        cx.log(l);
                    }
                    cx.progress(None);
                    cx.item(&item, ItemState::Ok, "✓ 682 MB, bad parts re-read");
                    cx.status("682 MB", "bad parts re-read");
                    outlet.send(item.becomes(ItemKind::Dump, Some(PathBuf::from(format!("{}.cue", disc.label))))).await;
                } else {
                    for l in [
                        "correction statistics:",
                        "SCSI: 104076 samples",
                        "C2: 211731 samples",
                        "Q: 0 sectors",
                        "*** END (time check: 52s)",
                        "*** SPLIT (time check: 0s)",
                        "errors detected, track: 1, sectors: {SKIP: 3, C2: 0}, samples: {SKIP: 1764, C2: 0}",
                        "error: data errors detected, unable to continue",
                    ] {
                        cx.log(l);
                    }
                    cx.progress(None);
                    cx.log("image: 3 unreadable sectors");
                    cx.item(&item, ItemState::Bad, "3 unreadable sectors");
                    cx.state(StepState::Bad);
                    cx.status("", "3 unreadable sectors");
                }
            }
        }
        Ok(())
    }

    async fn titles(&self, disc: &drt_engine::Item, outlet: &drt_engine::Outlet, cx: &drt_engine::Cx) -> anyhow::Result<()> {
        use drt_engine::{ItemKind, Subject};
        use drt_steps::real::video;
        let items = video::rip_items(disc);
        for i in &items {
            cx.item(i, ItemState::Waiting, "");
        }
        let len = |i: &drt_engine::Item| i.context().get(Subject::Part(i.index.unwrap_or(0)), video::facts::LENGTH).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let busy = if self.at == At::Done { items.len() } else { 1.min(items.len().saturating_sub(1)) };
        for (k, item) in items.iter().enumerate() {
            if k < busy || (self.at == At::Failed && k != 1) {
                if self.at == At::Failed && k > 1 && k < busy {
                    continue;
                }
                cx.item(item, ItemState::Ok, format!("✓ {}, {}", video::hms(len(item)), drt_steps::real::media::size(len(item) as u64 * 4_200_000)));
                outlet.send(item.becomes(ItemKind::Title, Some(PathBuf::from(format!("title-{k:02}.mkv"))))).await;
            }
        }
        match self.at {
            At::Done | At::Clean | At::Wrong | At::Spots | At::HandedOn => cx.status(video::titles_word(items.len()), ""),
            At::Reading | At::Crawling => {
                let it = &items[busy];
                cx.status(format!("{busy} of {}", video::titles_word(items.len())), it.label.clone());
                cx.item(it, ItemState::Busy, "saving to mkv file 20%");
                tokio::time::sleep(Duration::from_millis(3200)).await;
                cx.item(it, ItemState::Busy, "saving to mkv file 23%");
                cx.progress(Some(0.4));
            }
            At::Failed => {
                if let Some(it) = items.get(1) {
                    cx.item(it, ItemState::Bad, "Couldn't copy the video: read error at 61%");
                }
                cx.state(StepState::Bad);
                cx.status(video::titles_word(items.len() - 1), format!("{} couldn't be read", items.get(1).map_or("", |i| i.label.as_str())));
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl drt_engine::Step for HeldRip {
    fn info(&self) -> drt_engine::StepInfo {
        self.info.clone()
    }
    async fn run(&self, mut inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        let Some(disc) = inlet.recv().await else { return Ok(()) };
        if self.info.kind == "rip-video" {
            self.titles(&disc, &outlet, &cx).await?;
        } else {
            self.dump(&disc, &outlet, &cx).await?;
        }
        self.held.store(true, std::sync::atomic::Ordering::SeqCst);
        // Done ends; the others stay where they are.
        if !matches!(self.at, At::Done | At::Clean | At::Wrong | At::Spots | At::HandedOn) {
            cx.cancelled().await;
        }
        Ok(())
    }
}

/// The window with a held Rip for a kind of disc (`data`, `game`,
/// `video` for a season's disc, `film` for one title), started and held at
/// `at`, its sidebar open.
fn held_rip<'a>(e: &Env, kind: &str, at: At, size: egui::Vec2) -> (Harness<'a, Window>, &'static str) {
    use drt_steps::real::{data, game, video};
    let mut reg = simulated(None);
    let (info, recipe, id) = match kind {
        "data" => (data::image_info(), "Data discs", "d_image"),
        "game" => {
            reg.add(drt_steps::game_sim::GameDisc::new(vec![drt_steps::game_sim::COLONY_WARS_1]));
            (game::rip_info(), "Games", "g_rip")
        }
        _ => {
            video::sim::register(&mut reg, if kind == "film" { video::sim::SimDisc::Film } else { video::sim::SimDisc::Season });
            (video::rip_info(), "Films and TV", "v_rip")
        }
    };
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    reg.add(HeldRip { info, at, held: ready.clone() });
    let mut h = window_on(e, reg, true, 0.0, size, 1.0);
    app(&h).m.engine.settings.lock().unwrap().set_app("tmdb", "api_key", Setting::chosen(Value::from("test-key")));
    h.get_by_label_contains(&format!("{recipe},")).click();
    h.run_steps(2);
    click(&mut h, "Start");
    if kind == "video" {
        until(&mut h, "which episode", |a| matches!(a.tray, Some(Tray::Question { .. })));
        h.get_by_label("Use these").click_accesskit();
        h.run_steps(2);
    }
    until(&mut h, &format!("{kind} held"), |_| ready.load(std::sync::atomic::Ordering::SeqCst));
    if matches!(at, At::Done | At::Clean | At::Wrong | At::Spots) {
        until(&mut h, "Rip to finish", |a| a.m.snapshot(a.sel).steps[id].state == StepState::Ok);
    }
    h.run_steps(3);
    open_id(&mut h, id);
    (h, id)
}

/// The times on Rip's card (*Reading · 6 min left · 8.0×*, *Video 2 ·
/// 1:25 left · disc 5 min left · 16.2×*), checked to be the sidebar's too,
/// under what's being read.
fn times_said(h: &Harness<Window>) -> String {
    let card = h.get_by_label_contains("Rip, ").accesskit_node().label().unwrap();
    let line = card.split(", ").find(|s| s.contains(" left")).unwrap_or_else(|| panic!("no times on the card: {card}"));
    let times = line.split_once(" · ").map(|(_, t)| t).unwrap_or(line).to_owned();
    h.get_by_label(&times);
    times
}

/// The whole of the text that says `part`.
fn text_of(h: &Harness<Window>, part: &str) -> String {
    let n = h.get_by_label_contains(part);
    let a = n.accesskit_node();
    a.label().or_else(|| a.value()).map(|s| s.to_string()).unwrap_or_default()
}

/// Whether a list row says all of `parts`.
fn row_says(h: &Harness<Window>, parts: &[&str]) -> bool {
    h.get_all_by_role(Role::ListItem).filter_map(|n| n.accesskit_node().label()).any(|l| parts.iter().all(|p| l.contains(p)))
}

/// The Rip sidebar of each kind of disc in each state, as
/// `rip-<kind>-<state>[tag]`, checked for what it should say.
fn disc_rip_shots(size: egui::Vec2, tag: &str) {
    for kind in ["data", "game", "video"] {
        for (at, word) in [(At::Reading, "reading"), (At::Done, "done"), (At::Failed, "failed")] {
            let e = env();
            let (mut h, _) = held_rip(&e, kind, at, size);
            let label = if kind == "data" { "ENCARTA98" } else { "SLES_008.60" };
            match (kind, at) {
                // The disc as a ring, how far; beside it the pass, the time
                // left and the speed; the errors met so far on its row.
                ("data" | "game", At::Reading) => {
                    let ring = h.get_by_label_contains("The disc, 4").accesskit_node().label().unwrap();
                    assert!(ring.ends_with("% read"), "{ring}");
                    assert!(h.query_by_label("Pass 1 · Reading").is_none(), "the row says the pass");
                    let times = times_said(&h);
                    assert!(times.ends_with('×'), "{times}");
                    h.get_by_label_contains("Sector 14");
                    // The errors so far, in sectors: the drive's counts are the log's.
                    assert!(row_says(&h, &["1, Read the disc", "140 bad so far"]), "the errors so far");
                    assert!(h.query_by_label_contains("SCSI").is_none());
                    // Every sector as a map: how far it's read, what's
                    // wrong where, and how far from the centre, in a line.
                    h.get_by_label("Map of the disc: 144,060 good · 58 read with errors · 82 unreadable · 188,799 not read yet");
                    let line = text_of(&h, "So far ");
                    assert!(line.contains(" cm") && line.contains("once per turn") && line.ends_with("Look there for a scratch."), "{line}");
                    assert!(h.query_by_label_contains("Most read errors").is_none(), "said in centimetres, under the map");
                }
                ("data" | "game", At::Done) => {
                    h.get_by_label("The disc, 682 MB read");
                    h.get_by_label("Bad parts re-read");
                    // Every bad part put right, and where they were.
                    h.get_by_label("Map of the disc: 332,827 good · 172 put right on a re-read");
                    let line = text_of(&h, " patch");
                    assert!(line.contains("single spot, once per turn"), "{line}");
                    assert!(row_says(&h, &["1, Read the disc", "172 bad"]));
                    assert!(row_says(&h, &["2, Re-read bad parts", "172 put right", "20 tries each"]));
                    assert!(row_says(&h, &["Test every sector", "None read wrong"]));
                    // Only what Rip knows: nothing of Redump's here (Check, on
                    // the canvas, says whether it matches).
                    assert!(!row_says(&h, &["Redump"]) && !row_says(&h, &["erified"]));
                }
                ("data" | "game", At::Failed) => {
                    h.get_by_label("The disc, — not made");
                    h.get_by_label("3 sectors couldn't be read");
                    let map = h.get_by_label_contains("Map of the disc: ").accesskit_node().label().unwrap();
                    assert!(map.contains(" put right on a re-read · 3 unreadable"), "{map}");
                    assert!(row_says(&h, &["2, Re-read bad parts", "put right"]));
                    assert!(row_says(&h, &["3, Write the tracks", "3 unreadable"]));
                    h.get_by_label(&format!("Re-read bad parts, {label}"));
                    h.get_by_label(&format!("Make the image anyway, {label}"));
                    // A game's disc can't be left out; a data disc's can.
                    assert_eq!(h.query_by_label(&format!("Skip this disc, {label}")).is_some(), kind == "data");
                }
                // The titles as tracks: a ring, the one being read with its
                // time left and the disc's, each title a row with its length.
                (_, At::Reading) => {
                    h.get_by_label("The disc, 1/4 read");
                    h.get_by_label("Video 2 of 4");
                    let times = times_said(&h);
                    assert!(times.contains(" min left"), "{times}");
                    assert!(row_says(&h, &["1, S03E01", "Read 12.7 GB"]));
                    assert!(row_says(&h, &["2, S03E02", "Reading 23%"]));
                    assert!(row_says(&h, &["3, S03E03", "Waiting"]));
                }
                (_, At::Done) => {
                    h.get_by_label("The disc, 4/4 read");
                    h.get_by_label("All 4 read");
                }
                (_, At::Failed) => {
                    h.get_by_label("3 of 4 read");
                    assert!(row_says(&h, &["S03E02", "Not read", "Read error at 61%"]));
                    h.get_by_label("Read again, S03E02 Proshai, Livushka");
                    h.get_by_label("Skip this video, S03E02 Proshai, Livushka");
                }
                (_, At::Clean | At::Wrong | At::Spots | At::Crawling | At::HandedOn) => unreachable!(),
            }
            shot(&mut h, &format!("rip-{kind}-{word}{tag}"));
            // The same, the disc drawn to scale.
            if kind == "game" && at == At::Reading {
                click(&mut h, "Map: Disc");
                h.get_by_label_contains("Map of the disc: 144,060 good");
                shot(&mut h, &format!("rip-{kind}-{word}-disc{tag}"));
            }
        }
    }
    // A game read cleanly, and one like Koudelka's: errors as it read,
    // and sectors found wrong.
    for (at, word) in [(At::Clean, "clean"), (At::Wrong, "wrong"), (At::Spots, "spots")] {
        let e = env();
        let (mut h, _) = held_rip(&e, "game", at, size);
        // Every sector in rows first, a small switch to the disc drawn to
        // scale under the heading.
        assert_eq!(app(&h).map_view, drt_app::sectormap::MapView::Sectors);
        h.get_by_label("Where on the disc");
        h.get_by_label("Map: Sectors");
        if at == At::Spots {
            let line = text_of(&h, " patch");
            assert!(line.contains("4.4 cm and 4.9 cm") && line.contains("single spots, once per turn"), "{line}");
            assert!(h.query_by_label("a single spot somewhere on this ring").is_none(), "only on the disc");
            // Only Rip's own work: nothing Check and fix repaired.
            let map = h.get_by_label_contains("Map of the disc: ").accesskit_node().label().unwrap();
            assert!(map.contains(" put right on a re-read · 25 read wrong") && !map.contains("repaired"), "{map}");
        } else if at == At::Clean {
            h.get_by_label("The disc, 682 MB read");
            h.get_by_label("Read cleanly");
            assert!(h.query_by_label_contains("bad sector").is_none() && h.query_by_label_contains("read errors").is_none());
            h.get_by_label("Map of the disc: 332,999 good");
            assert!(h.query_by_label_contains("patch").is_none() && h.query_by_label_contains("once per turn").is_none());
        } else {
            h.get_by_label("The disc, 682 MB not clean");
            h.get_by_label("25 sectors read wrong");
            // The sectors found wrong from Rip's log, over its map.
            h.get_by_label("Map of the disc: 332,722 good · 252 put right on a re-read · 25 read wrong");
            assert!(row_says(&h, &["Test every sector", "25 read wrong"]));
            assert!(h.query_by_label_contains("25 bad sectors").is_none(), "the count is in the headline once");
        }
        shot(&mut h, &format!("rip-game-{word}{tag}"));
        if at != At::Wrong {
            click(&mut h, "Map: Disc");
            if at == At::Spots {
                // Each spot's ring dashed, and what that means.
                h.get_by_label("a single spot somewhere on this ring");
            }
            shot(&mut h, &format!("rip-game-{word}-disc{tag}"));
        }
    }
    // One title: the emptiest list.
    let e = env();
    let (mut h, _) = held_rip(&e, "film", At::Done, size);
    h.get_by_label("The disc, 1/1 read");
    h.get_by_label("Read");
    shot(&mut h, &format!("rip-film-done{tag}"));
}

/// The map's view picked stays picked, the next time too.
#[test]
fn the_map_s_view_is_remembered() {
    use drt_app::sectormap::MapView;
    let e = env();
    let (mut h, _) = held_rip(&e, "game", At::Clean, egui::vec2(1280.0, 720.0));
    assert_eq!(app(&h).map_view, MapView::Sectors, "every sector first");
    click(&mut h, "Map: Disc");
    assert_eq!(app(&h).map_view, MapView::Disc);
    drop(h);
    let h = window(&e, None, true, 0.0);
    assert_eq!(app(&h).map_view, MapView::Disc);
}

/// A map with no distances (not a CD) has only its sectors: no switch.
#[test]
fn a_map_without_distances_has_no_disc_view() {
    use drt_steps::map::{Mark, Sectors};
    let mut s = Sectors::new(2_000_000);
    for k in 0..2_000_000 {
        s.set(k, Mark::Good);
    }
    let m = s.map(None, false);
    let h = Harness::builder().with_context_setup(|ctx| ctx.set_fonts(drt_app::theme::fonts())).build_ui(move |ui| {
        drt_app::sectormap::show(ui, &drt_app::theme::Pal::light(), &m, 0.0, drt_app::sectormap::MapView::Disc, drt_app::sectormap::Whose::Rip);
    });
    h.get_by_label("Map of the disc: 2,000,000 good");
    assert!(h.query_by_label("Map: Disc").is_none());
}

/// A data disc with sectors it couldn't read, settled each way from the
/// image's row: re-read, kept, skipped.
#[test]
fn an_image_that_needs_you_is_settled_from_its_row() {
    for (option, word) in [("Re-read bad parts", "reread"), ("Make the image anyway", "kept"), ("Skip this disc", "skipped")] {
        let e = env();
        let mut h = window_on(&e, simulated(Some(1)), true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
        h.get_by_label_contains("Data discs,").click();
        h.run_steps(2);
        click(&mut h, "Start");
        until(&mut h, "the bad sectors", |a| a.m.snapshot(a.sel).steps["d_image"].state == StepState::Bad);
        open_id(&mut h, "d_image");
        h.get_by_label("The disc, — not made");
        h.get_by_label("3 sectors couldn't be read");
        assert!(row_says(&h, &["1, Read the disc", "Errors"]));
        click(&mut h, &format!("{option}, ENCARTA98"));
        until(&mut h, "the image settled", |a| !matches!(a.m.snapshot(a.sel).steps["d_image"].items[0].state, ItemState::Bad | ItemState::Busy));
        h.run_steps(3);
        open_id(&mut h, "d_image");
        match word {
            "reread" => {
                h.get_by_label("Bad parts re-read");
                assert!(row_says(&h, &["Re-read bad parts", "All put right"]));
                assert!(row_says(&h, &["Test every sector", "None read wrong"]));
            }
            "kept" => {
                h.get_by_label("The disc, 682 MB kept");
                h.get_by_label("Kept with 3 unreadable sectors left blank");
            }
            _ => {
                h.get_by_label("The disc, Skipped ");
                h.get_by_label("No image kept");
            }
        }
        // Nothing left to do on its row.
        assert!(h.query_by_label(&format!("{option}, ENCARTA98")).is_none());
        shot(&mut h, &format!("rip-data-{word}"));
    }
}

#[test]
fn a_disc_s_rip_shows_its_progress_and_verdicts() {
    disc_rip_shots(egui::vec2(1280.0, 720.0), "");
}

#[test]
fn a_disc_s_rip_shows_its_progress_and_verdicts_at_1920() {
    disc_rip_shots(egui::vec2(1920.0, 1080.0), "-1920");
}

#[test]
fn a_folder_s_progress_lists_each_filed_file_once() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "the review folder", |a| a.m.snapshot(0).steps["review"].count(ItemState::Ok) == 17);
    open_id(&mut h, "review");
    h.run_steps(2);
    // One list: each track, then where it was filed; no second list of
    // the same files under it.
    assert!(h.query_by_label("Files").is_none(), "no Files list as well");
    let rows: Vec<String> = h.get_all_by_role(Role::ListItem).filter_map(|n| n.accesskit_node().label()).filter(|l| l.contains("Jimmy the Exploder")).collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].ends_with(".flac"), "{rows:?}");
}

#[test]
fn a_setting_says_its_choices_in_words_and_its_help_under_its_name() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    // Choices by their labels; what's saved is still the value.
    open_tile(&mut h, "Rip,");
    click(&mut h, "Ripping speed: Slow and careful");
    assert_eq!(param(&saved_recipes(&e)[0], "rip", "first_read").unwrap().resolve(None), Some(Value::from("careful")));
    // Help, where a setting has it, is under its name.
    open_tile(&mut h, "Drive,");
    h.get_by_label("How early or late this drive reads audio. Found for you, and needed to check each track.");
    // On the card, a choice shows the words before its explanation, and
    // the label's help is there on hover.
    let ctx = h.ctx.clone();
    let s = snap(&h);
    let cards = app_mut(&mut h).cards(&ctx, &s);
    let rip = &cards["rip"].settings[0];
    assert_eq!((rip.label.as_str(), rip.value.as_str()), ("Speed", "slow and careful"));
    let drive = cards["drive"].settings.iter().find(|x| x.label == "Offset").unwrap();
    assert!(drive.help.is_some_and(|h| h.starts_with("How early or late")));
    let v = &s.steps["drive"];
    let painter = egui::Painter::new(ctx, egui::LayerId::background(), egui::Rect::EVERYTHING);
    let measure = drt_app::card::measurer(&painter);
    let c = drt_app::card::build(&drt_app::card::Input { v, settings: &cards["drive"].settings, locked: false, cover: false, asking: false, retries: false }, &measure);
    let row = drt_app::card::SETTINGS[1];
    assert_eq!(c.help_at(egui::vec2(30.0, (row.0 + row.1) / 2.0)), drive.help, "over the offset's label");
    assert_eq!(c.help_at(egui::vec2(150.0, (row.0 + row.1) / 2.0)), None, "not over its value");
    // Every card label fits its column whole.
    use drt_app::card::{M, SMALL, THIRD};
    let cut: Vec<String> = drt_steps::real()
        .infos()
        .flat_map(|info| info.params.iter().filter_map(|p| p.on_card).filter(|on| measure(on.label, SMALL, 400) > THIRD - M - 4.0).map(|on| format!("{}: {}", info.kind, on.label)).collect::<Vec<_>>())
        .collect();
    assert!(cut.is_empty(), "cut on the card: {cut:?}");
}

/// Steps the window until something labelled `label` shows.
fn until_label(h: &mut Harness<Window>, label: &str) {
    let start = Instant::now();
    while h.query_by_label(label).is_none() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {label}");
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run_steps(2);
}

/// Notify makes up a topic for you, its own, and sends a test through
/// each way that's on.
#[test]
fn notify_makes_up_a_topic_and_sends_a_test() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let i = app(&h).m.recipes.iter().position(|r| r.name == "Archive everything").unwrap();
    app_mut(&mut h).select(i);
    h.run_steps(3);
    open_id(&mut h, "a_tell");
    if h.query_all_by_role(Role::Tab).count() > 0 {
        tab(&mut h, "Settings");
    }
    let topic_of = |h: &Harness<'_, Window>| -> Option<String> {
        app(h).m.recipes[i].root.steps().into_iter().find_map(|n| match n {
            Node::Step { id, params, .. } if id == "a_tell" => params.get("ntfy_topic").and_then(|s| s.resolve(None)).and_then(|v| v.as_str().map(str::to_owned)),
            _ => None,
        })
    };
    click(&mut h, "Make one up");
    let topic = topic_of(&h).unwrap();
    let parts: Vec<&str> = topic.split('-').collect();
    assert!(parts.len() == 4 && parts[0] == "rip" && parts[3].len() == 4, "{topic}");
    assert!(text_boxes(&h).contains(&topic), "the box shows it: {:?}", text_boxes(&h));
    // Saved with this Notify, in this recipe only.
    let saved = std::fs::read_to_string(e.dir.path().join("config/recipes.json")).unwrap();
    assert_eq!(saved.matches(&topic).count(), 1, "{saved}");
    assert!(app(&h).m.engine.settings.lock().unwrap().app("tell", "ntfy_topic") == Setting::Default);
    assert!(h.query_all_by_label_contains("yours").count() > 0 && h.query_all_by_label_contains("app-wide").count() == 0);
    // Another one, different.
    click(&mut h, "Make one up");
    let again = Value::from(topic_of(&h).unwrap());
    assert_ne!(again, Value::from(topic.clone()));

    click(&mut h, "Send a test");
    let topic = again.as_str().unwrap().to_owned();
    until_label(&mut h, &format!("Sent to this computer and to ntfy topic {topic}"));
    assert_eq!(*e.tests_sent.lock().unwrap(), vec![(true, "https://ntfy.sh".to_owned(), topic.clone())]);
    shot(&mut h, "step-notify");
    // A server that can't be reached says so.
    app_mut(&mut h).m.set_param(i, "a_tell", "ntfy_server", Value::from("https://down.example")).unwrap();
    h.run_steps(2);
    click(&mut h, "Send a test");
    until_label(&mut h, "Sent to this computer, but ntfy: couldn't reach down.example");
    shot(&mut h, "step-notify-failed");
}

/// The sim Drive, but every disc after the first waits until `disc` is set.
struct WaitingDrive {
    inner: std::sync::Arc<dyn drt_engine::Step>,
    runs: std::sync::atomic::AtomicUsize,
    disc: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl drt_engine::Step for WaitingDrive {
    fn info(&self) -> drt_engine::StepInfo {
        self.inner.info()
    }
    async fn run(&self, inlet: drt_engine::Inlet, outlet: drt_engine::Outlet, cx: drt_engine::Cx) -> anyhow::Result<()> {
        if self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
            while !self.disc.load(std::sync::atomic::Ordering::SeqCst) {
                cx.status("Empty", "Insert a disc");
                cx.sleep(5).await?;
            }
        }
        self.inner.run(inlet, outlet, cx).await
    }
}

#[test]
fn the_disc_before_stays_on_the_canvas_until_the_next_goes_in() {
    let e = env();
    let inner = simulated(None).get("drive").unwrap().0;
    let disc = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut reg = simulated(None);
    reg.add(WaitingDrive { inner, runs: Default::default(), disc: disc.clone() });
    let (mut reg, open) = latched_hold(reg);
    // The FLAC Library files only when the test says, so the canvas can be
    // seen to follow it.
    let filing = Arc::new(std::sync::atomic::AtomicBool::new(false));
    reg.add(LatchedFolder { inner: simulated(None).get("folder").unwrap().0, folder: "~/Music/FLAC", open: filing.clone() });
    let mut h = window_on(&e, reg, true, 0.0, egui::vec2(1280.0, 720.0), 1.0);
    app_mut(&mut h).m.repeat_limit = Some(1);
    click(&mut h, "Start");
    to_the_hold(&mut h);
    until(&mut h, "the second disc", |a| a.m.runs.get(&0).is_some_and(|v| v.len() == 2));
    until(&mut h, "the drive to wait", |a| a.m.newest(0).is_some_and(|r| r.snap.steps["drive"].status == "Empty"));
    open_latch(&open);
    // The first disc's tracks are still there; the Drive says it's empty.
    let s = app(&h).m.snapshot(0);
    assert_eq!(s.steps["rip"].items.len(), 17, "the disc before, as it ended");
    assert_eq!(s.steps["drive"].status, "Empty");
    // The first disc's Hold fills: it waits for you, so the next disc,
    // not in yet, is put back until you've let the files through.
    until(&mut h, "the first to hold", |a| a.m.runs[&0][0].snap.steps["hold"].detail == "All here");
    until(&mut h, "the next disc put back", |a| a.m.runs[&0].len() == 1);
    assert_eq!(app(&h).m.snapshot(0).steps["hold"].detail, "All here");
    assert_eq!(app(&h).m.held_back(0).as_deref(), Some("Let the files on the last disc through first"));
    click_tile(&mut h, "Hold,");
    h.get_by_label("Let 17 through").click();
    h.run_steps(2);
    until(&mut h, "the second disc again", |a| a.m.runs[&0].len() == 2);
    until(&mut h, "the drive to wait again", |a| a.m.newest(0).is_some_and(|r| r.snap.steps["drive"].status == "Empty"));
    // Followed as it finishes, not frozen as it was when the tray opened.
    assert_ne!(app(&h).m.snapshot(0).steps["lib1"].status, "17 filed");
    open_latch(&filing);
    until(&mut h, "the first to file", |a| a.m.snapshot(0).steps["lib1"].status == "17 filed");
    assert!(app(&h).m.newest(0).is_some_and(|r| r.snap.steps["drive"].state != StepState::Ok), "still waiting for the disc");
    disc.store(true, std::sync::atomic::Ordering::SeqCst);
    // In: the canvas is the new disc's from here.
    until(&mut h, "the new disc", |a| a.m.newest(0).is_some_and(|r| r.snap.steps["drive"].state == StepState::Ok));
    let a = app(&h);
    assert_eq!(a.m.snapshot(0).steps["rip"].items.len(), a.m.newest(0).unwrap().snap.steps["rip"].items.len());
}

/// System, Light or Dark, picked at the bottom of the recipes and kept.
#[test]
fn the_appearance_is_picked_and_remembered() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    h.get_by_label("Appearance: Dark").click();
    h.run_steps(2);
    assert!(app(&h).dark);
    assert_eq!(std::fs::read_to_string(e.dir.path().join("config/appearance")).unwrap(), "dark");
    h.get_by_label("Appearance: System").click();
    h.run_steps(2);
    assert_eq!(app(&h).appearance, drt_app::Appearance::System);
    assert_eq!(std::fs::read_to_string(e.dir.path().join("config/appearance")).unwrap(), "system");
}

/// Several tracks to decide, at the sidebar's narrowest and at 1920:
/// each row's choices on lines of their own, wrapping, nothing cut.
#[test]
fn several_tracks_to_decide_each_have_their_choices_on_their_row() {
    for (size, tag) in [(egui::vec2(1280.0, 720.0), ""), (egui::vec2(1920.0, 1080.0), "-1920")] {
        let e = env();
        let mut h = window_on(&e, drt_steps::simulated_bad(vec![4, 9]), true, 0.0, size, 1.0);
        click(&mut h, "Start");
        until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
        click(&mut h, "Use this one");
        until(&mut h, "two to decide", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 2 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 15);
        open_id(&mut h, "rip");
        let side = app(&h).side_rect().unwrap();
        if tag.is_empty() {
            assert!(side.width() <= 450.0, "the narrowest sidebar: {side:?}");
        }
        for title in ["Suzy Lee", "Broken Bricks"] {
            for b in ["Read again", "Best of 3 reads", "Read again slowly", "Keep it anyway", "Skip this track"] {
                let r = h.get_by_label(&format!("{b}, {title}")).rect();
                assert!(side.contains_rect(r), "{b} for {title} inside the sidebar: {r:?} in {side:?}");
            }
        }
        h.get_by_label("Read them all again");
        h.get_by_label("Open tray");
        shot(&mut h, &format!("sidebar-rip-retry-two{tag}"));
    }
}

/// *Open tray* on the drive's sidebar, idle; and on Rip's mid-read, which
/// stops the read: that track and the ones after wait for you, saying why,
/// and go on once read again.
#[test]
fn open_tray_from_the_drive_and_from_rip_mid_read() {
    let e = env();
    let mut h = window(&e, None, true, 0.2);
    open_tile(&mut h, "Drive,");
    click(&mut h, "Open tray");
    until(&mut h, "the tray to open", |a| !a.m.opening_tray(0));
    assert_eq!(*e.trays.lock().unwrap(), [recipes::BLU_RAY]);
    app_mut(&mut h).close_tray();
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "reading", |a| {
        let r = &a.m.snapshot(0).steps["rip"];
        (3..=12).contains(&r.count(ItemState::Ok)) && r.count(ItemState::Busy) == 1
    });
    open_id(&mut h, "rip");
    click(&mut h, "Open tray");
    until(&mut h, "the read to stop", |a| {
        let r = &a.m.snapshot(0).steps["rip"];
        r.count(ItemState::Busy) == 0 && r.count(ItemState::Waiting) == 0 && r.count(ItemState::Bad) > 0
    });
    until(&mut h, "the tray to open", |a| !a.m.opening_tray(0));
    assert_eq!(e.trays.lock().unwrap().len(), 2);
    let s = snap(&h);
    let rip = &s.steps["rip"];
    assert!(rip.items.iter().filter(|i| i.state == ItemState::Bad).all(|i| i.note == drt_steps::real::TRAY_OPENED), "{:?}", rip.items);
    assert!(!s.finished && !s.stopped);
    // The one being read and every one after it.
    assert!(rip.count(ItemState::Bad) >= 5, "{:?}", rip.items);
    h.run_steps(3);
    shot(&mut h, "sidebar-rip-tray-opened");
    click(&mut h, "Read them all again");
    until(&mut h, "all read", |a| a.m.snapshot(0).steps["rip"].count(ItemState::Bad) == 0 && a.m.snapshot(0).steps["rip"].count(ItemState::Ok) == 17);
}

/// Puts the view at `zoom` with the + labelled `plus` at the canvas's
/// middle, a little left so its menu has room on the right.
fn look_at_join(h: &mut Harness<Window>, plus: &str, zoom: f32) {
    let a = app(h);
    let (i, lay, c) = (a.sel, a.layout(a.sel), a.canvas_rect());
    let root = a.m.recipes[i].root.clone();
    let snap = a.m.snapshot(i);
    let j = lay.joins.iter().find(|j| drt_app::popover::join_title(&root, &j.path, &snap) == plus).unwrap_or_else(|| panic!("no + {plus}"));
    let want = egui::vec2(c.width() * 0.4, c.height() * 0.45);
    app_mut(h).set_view(i, (zoom, want - egui::vec2(j.at.0, j.at.1) * zoom));
    h.run_steps(2);
}

/// The name the registry gives a step kind, as its tile and the + list show it.
fn step_name(h: &Harness<Window>, kind: &str) -> &'static str {
    app(h).m.engine.registry.info(kind).unwrap_or_else(|| panic!("no step {kind}")).name
}

/// Opens the + with this label and picks `kind` from its list.
fn add_with_plus(h: &mut Harness<Window>, plus: &str, kind: &str) {
    h.get_by_label(plus).click_accesskit();
    h.run_steps(2);
    let name = step_name(h, kind);
    h.get_all_by_role(Role::ListItem).find(|n| n.accesskit_node().label().as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} isn't offered by {plus}")).click_accesskit();
    h.run_steps(3);
}

#[test]
fn branches_meet_at_a_plus_and_what_goes_there_gets_all_of_them() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Edit");
    // CDs: the lookups meet at Choose, with a + where their wires arrive.
    look_at_join(&mut h, "Add after MusicBrainz and CD-Text, before Choose", 0.7);
    // A real click: it sits on Choose's edge, and must take the click.
    h.get_by_label("Add after MusicBrainz and CD-Text, before Choose").click();
    h.run_steps(2);
    let offered: Vec<String> = h.get_all_by_role(Role::ListItem).filter_map(|n| n.accesskit_node().label()).collect();
    // Beside Choose's line, nothing chosen yet: another Choose, a Rip, a Notify.
    for name in ["Choose", "Rip", "Notify"] {
        assert!(offered.iter().any(|o| o == name), "{name} in {offered:?}");
    }
    shot(&mut h, "join-plus-lookups");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // FLAC's, MP3's and Next disc's lines end with nothing after them: a +
    // where lines from each meet, for something after all three.
    look_at_join(&mut h, "Add after FLAC, MP3 and Next disc", 0.6);
    shot(&mut h, "join-plus-cd-end");
    h.get_by_label("Add after FLAC, MP3 and Next disc").click_accesskit();
    h.run_steps(2);
    assert!(app(&h).popover.as_ref().is_some_and(|p| matches!(p.at, drt_engine::Spot::Join(_))));
    let offered: Vec<String> = h.get_all_by_role(Role::ListItem).filter_map(|n| n.accesskit_node().label()).filter(|l| !l.contains(", ")).collect();
    assert_eq!(offered, ["Notify"], "only what takes what all three give");
    shot(&mut h, "join-plus-cd");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // The archive: an inner set (NAS, Backup…) and the outer, at Notify.
    click(&mut h, "Cancel");
    h.get_by_label_contains("Archive everything, Ready").click();
    h.run_steps(2);
    click(&mut h, "Edit");
    // The copies after the FLAC library meet Notify with MP3's and the
    // phone's lines; their own + is in the gap, and shows which it follows.
    let inner = "Add after NAS, Backup and 3 more";
    look_at_join(&mut h, inner, 0.6);
    shot(&mut h, "join-plus-archive-closed");
    h.get_by_label(inner).click();
    h.run_steps(2);
    shot(&mut h, "join-plus-archive");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.get_by_label("Add after FLAC, MP3 and Opus, before Notify");
}

/// A PS1 recipe made by hand, one + at a time: checked against Redump,
/// filed, its LibCrypt protection file and its playlist side by side, then
/// one Notify once both are done; and the tray opened for the next disc.
#[test]
fn a_ps1_recipe_is_built_by_hand_with_its_side_by_side_steps_joined() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "New recipe");
    h.event(egui::Event::Text("PlayStation".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    pick_drive(&mut h, recipes::BLU_RAY);
    let n = |h: &Harness<Window>, k: &str| step_name(h, k);
    for (after, kind) in [("drive", "game-disc"), ("game-disc", "game-rip"), ("game-rip", "check"), ("check", "redump"), ("redump", "game-choose"), ("game-choose", "chd"), ("chd", "folder")] {
        let plus = format!("Add after {}", n(&h, after));
        add_with_plus(&mut h, &plus, kind);
    }
    // Where to file it: the folder picker is the system's, so set directly.
    let i = app(&h).sel;
    app_mut(&mut h).m.set_param(i, "folder", "folder", Value::from("~/Games")).unwrap();
    let plus = format!("Add after {}", n(&h, "folder"));
    add_with_plus(&mut h, &plus, "libcrypt");
    // The playlist beside the protection file, not after it: both fit in
    // line, so its row has a button for the other way.
    let (folder, sbi, m3u) = (n(&h, "folder"), n(&h, "libcrypt"), n(&h, "m3u"));
    click(&mut h, "Fit to window");
    h.get_by_label(&format!("Add between {folder} and {sbi}")).click_accesskit();
    h.run_steps(2);
    shot(&mut h, "ps1-beside");
    // A real click: the button sits on the row, and must take the click.
    h.get_by_label(&format!("Add {m3u} beside {sbi}")).click();
    h.run_steps(3);
    // Where they meet, one Notify after both.
    let join = format!("Add after {sbi} and {m3u}");
    look_at_join(&mut h, &join, 0.6);
    h.get_by_label(&join).click();
    h.run_steps(2);
    shot(&mut h, "ps1-join-plus");
    h.get_all_by_role(Role::ListItem).find(|x| x.accesskit_node().label().as_deref() == Some(n(&h, "tell"))).unwrap().click_accesskit();
    h.run_steps(3);
    // And the tray opened once it's chosen: beside CHD's line.
    let plus = format!("Add between {} and {}", n(&h, "game-choose"), n(&h, "chd"));
    add_with_plus(&mut h, &plus, "next-disc");
    let r = app(&h).m.recipes[i].clone();
    use drt_engine::recipe::{branches, chain};
    let want = chain([
        Node::step("drive", "drive"),
        Node::step("gamedisc", "game-disc"),
        Node::step("gamerip", "game-rip"),
        Node::step("check", "check"),
        Node::step("redump", "redump"),
        Node::step("gamechoose", "game-choose"),
        branches([
            chain([Node::step("chd", "chd"), Node::step("folder", "folder"), branches([Node::step("libcrypt", "libcrypt"), Node::step("m3u", "m3u")]), Node::step("tell", "tell")]),
            Node::step("nextdisc", "next-disc"),
        ]),
    ]);
    fn shape(n: &Node) -> Node {
        match n {
            Node::Step { id, step, .. } => Node::step(id, step),
            Node::Chain { nodes } => {
                let mut v = vec![];
                for n in nodes.iter().map(shape) {
                    match n {
                        Node::Chain { nodes } => v.extend(nodes),
                        n => v.push(n),
                    }
                }
                if v.len() == 1 {
                    v.pop().unwrap()
                } else {
                    Node::Chain { nodes: v }
                }
            }
            Node::Branches { branches } => Node::Branches { branches: branches.iter().map(shape).collect() },
            Node::Group { name, body } => Node::Group { name: name.clone(), body: Box::new(shape(body)) },
        }
    }
    assert_eq!(shape(&r.root), shape(&want), "{:#?}", r.root);
    drt_engine::check(&r.root, &app(&h).m.engine.registry).unwrap();
    click(&mut h, "Fit to window");
    shot(&mut h, "ps1-canvas");
    let met = format!("{join}, before {}", n(&h, "tell"));
    look_at_join(&mut h, &met, 0.45);
    shot(&mut h, "ps1-canvas-near");
    click(&mut h, "Save");
    assert!(app(&h).m.cant_start(i).is_none(), "{:?}", app(&h).m.cant_start(i));
    shot(&mut h, "ps1-saved");
}

/// Follow, at the foot of the canvas: on, it's kept for next time; a drag
/// turns it off.
#[test]
fn follow_is_switched_on_remembered_and_dragged_off() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    assert!(!app(&h).follow, "off to start with");
    assert_eq!(h.get_by_label("Follow").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    click(&mut h, "Follow");
    assert!(app(&h).follow);
    assert_eq!(h.get_by_label("Follow").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    assert_eq!(std::fs::read_to_string(e.dir.path().join("config/follow")).unwrap(), "on");
    // Another window on the same settings starts with it on.
    assert!(window(&e, None, true, 0.0).state().0.follow);
    // Dragging the canvas by hand stops it.
    let canvas = egui::pos2(700.0, 450.0);
    h.hover_at(canvas);
    h.step();
    h.drag_at(canvas);
    h.step();
    h.hover_at(canvas + egui::vec2(30.0, 10.0));
    h.step();
    h.drop_at(canvas + egui::vec2(40.0, 10.0));
    h.run_steps(2);
    assert!(!app(&h).follow, "a drag turns Follow off");
    // So does zooming with the buttons.
    click(&mut h, "Follow");
    click(&mut h, "Zoom in");
    assert!(!app(&h).follow, "zooming by hand turns Follow off");
}

/// In a window too small for a card at 100%, Follow zooms in only as far
/// as the card still fits, with room round it.
#[test]
fn follow_in_a_small_window_zooms_in_as_far_as_the_card_fits() {
    let e = env();
    let mut h = window_on(&e, simulated(None), true, 0.0, egui::vec2(1280.0, 270.0), 1.0);
    app_mut(&mut h).follow = true;
    app_mut(&mut h).set_view(0, (0.2, egui::vec2(20.0, 20.0)));
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    h.run_steps(60);
    let a = app(&h);
    let c = a.canvas_rect();
    let card = a.layout(0).tile(&snap(&h).question.unwrap().0).unwrap().rect;
    let v = a.views_of(0);
    assert!(v.zoom > 0.5 && v.zoom < 1.0, "as far in as it fits: {}", v.zoom);
    let (min, max) = (v.pan + egui::vec2(card.x, card.y) * v.zoom, v.pan + egui::vec2(card.right(), card.y + card.h) * v.zoom);
    assert!(min.x >= 0.0 && min.y >= 0.0 && max.x <= c.width() && max.y <= c.height(), "and it's all in view");
}

/// With Follow on, the canvas goes to where things are happening, even
/// from far off and zoomed in, and zoomed out it comes in to the cards'
/// own size, where they read; with it off, it stays where you left it.
#[test]
fn follow_brings_the_latest_step_into_view() {
    fn inside(a: &App, step: &str) -> bool {
        let r = a.layout(0).tile(step).unwrap().rect;
        let v = a.views_of(0);
        let (min, max) = (v.pan + egui::vec2(r.x, r.y) * v.zoom, v.pan + egui::vec2(r.right(), r.y + r.h) * v.zoom);
        let c = a.canvas_rect();
        min.x >= 0.0 && min.y >= 0.0 && max.x <= c.width() && max.y <= c.height()
    }
    for on in [true, false] {
        let e = env();
        let mut h = window(&e, None, true, 0.0);
        app_mut(&mut h).follow = on;
        click(&mut h, "Start");
        until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
        h.run_steps(10);
        let step = snap(&h).question.unwrap().0;
        assert!(inside(app(&h), &step), "the step asking is in view");
        click(&mut h, "Use this one");
        // Zoomed in on the drive, the rest of the recipe off to the right.
        app_mut(&mut h).set_view(0, (2.0, egui::vec2(40.0, 40.0)));
        until(&mut h, "Hold to wait for you", |a| a.m.snapshot(0).steps["hold"].state == StepState::Hold && a.m.snapshot(0).steps["rip"].state == StepState::Ok);
        h.run_steps(120);
        let a = app(&h);
        let last = a.followed().expect("something happened").to_owned();
        assert_eq!(last, "hold", "the step waiting for you comes first");
        let v = a.views_of(0);
        if on {
            assert!(inside(a, &last), "{last} is in view with Follow on");
            assert_eq!(v.zoom, 2.0, "zoomed in by hand, it fits, so it stays");
            // Zoomed far out, it comes in to 100%, and no further.
            app_mut(&mut h).set_view(0, (0.4, egui::vec2(40.0, 40.0)));
            h.run_steps(120);
            let a = app(&h);
            assert!((a.views_of(0).zoom - 1.0).abs() < 0.001, "readable: {}", a.views_of(0).zoom);
            assert!(inside(a, &last));
        } else {
            assert!(!inside(a, &last), "{last} is left alone with Follow off");
            assert_eq!((v.zoom, v.pan), (2.0, egui::vec2(40.0, 40.0)), "without Follow, the view stays");
        }
    }
}

/// In edit mode each card's right edge has its +, big enough to hit, and
/// just above and below it a button for each path the step can take: if
/// it fails, if it needs you. A path's line leaves from there. On a games
/// recipe that ends a disc beside other branches, what hears of Rip sits
/// just under Rip, with the line back to the drive under that.
#[test]
fn the_plus_on_each_cards_edge_has_its_path_buttons_above_and_below() {
    use drt_engine::recipe::{branches, chain};
    use drt_engine::Side;
    let e = env();
    let mut all = recipes::all();
    for r in all.iter_mut().filter(|r| r.name == "Games") {
        let s = Node::step;
        r.root = chain([
            s("g_drive", "drive"),
            s("g_disc", "game-disc"),
            s("g_rip", "game-rip").on(Side::Fails, s("g_tell_fails", "tell")).on(Side::NeedsYou, s("g_tell_needs", "tell")),
            s("g_check", "check"),
            s("g_redump", "redump"),
            s("g_choose", "game-choose"),
            branches([
                chain([s("g_chd", "chd"), s("g_lib", "folder").labelled("Library").with("folder", "~/Games/Compressed"), branches([s("g_sbi", "libcrypt"), s("g_m3u", "m3u")])]),
                s("g_tell", "tell").with("when", "always"),
                s("g_next", "next-disc"),
                chain([s("g_keep", "keep"), s("g_full", "folder").with("folder", "~/Games/Full")]),
            ]),
        ]);
    }
    Store::new(e.dir.path().join("config")).save_recipes(&all).unwrap();
    let mut h = window_at(&e, None, true, 0.0, egui::vec2(1280.0, 800.0), 1.0);
    click(&mut h, "Edit");
    // CDs: the + on Rip's edge, and Rip's buttons right above and below it.
    let lay = app(&h).layout(app(&h).sel);
    let plus = h.get_by_label("Add after Rip").rect();
    let (fails, needs) = (h.get_by_label("If Rip fails").rect(), h.get_by_label("If Rip needs you").rect());
    assert!(plus.width() >= 32.0 && plus.height() >= 32.0, "a + big enough to hit: {plus:?}");
    assert!(fails.width() >= 22.0 && fails.width() < plus.width(), "a path button smaller than its +: {fails:?}");
    let rip = lay.tile("rip").unwrap().rect;
    let v = app(&h).views_of(app(&h).sel);
    let edge = app(&h).canvas_rect().left() + v.pan.x + rip.right() * v.zoom;
    for b in [fails, needs] {
        assert!((b.center().x - edge).abs() < 1.0, "on Rip's right edge: {b:?} vs {edge}");
        assert!((b.center().x - plus.center().x).abs() < 1.0);
    }
    assert!(fails.bottom() <= plus.top() && plus.bottom() <= needs.top(), "above and below: {fails:?} {plus:?} {needs:?}");
    shot(&mut h, "edit-pluses-cds");
    let pan = v.pan;
    let i = app(&h).sel;
    app_mut(&mut h).set_view(i, (0.5, pan));
    h.run_steps(2);
    shot(&mut h, "edit-pluses-cds-50");
    // Games, with Notify on Rip's paths.
    h.get_by_label_contains("Games, ").click();
    h.run_steps(2);
    let i = app(&h).sel;
    let lay = app(&h).layout(i);
    let rip = lay.tile("g_rip").unwrap().rect;
    for id in ["g_tell_fails", "g_tell_needs"] {
        let t = lay.tile(id).unwrap().rect;
        assert!(t.y - (rip.y + rip.h) < 128.0, "{id} just under Rip");
    }
    let low = lay.tiles.iter().map(|t| t.rect.y + t.rect.h).fold(0.0, f32::max);
    assert!(lay.loops[0].iter().any(|p| p.1 > low), "the line back under everything");
    click(&mut h, "Edit");
    click(&mut h, "Fit to window");
    shot(&mut h, "edit-pluses-games-fit");
    app_mut(&mut h).set_view(i, (1.0, egui::vec2(-400.0, -300.0)));
    h.run_steps(2);
    shot(&mut h, "edit-pluses-games");
}

/// The Drive's tile is busy whenever anything reads the drive: Follow
/// doesn't keep going back to it.
#[test]
fn follow_doesnt_go_back_to_the_drive() {
    let e = env();
    // At a pace, not all at once: on a slow machine an instant run can
    // reach its question before Follow has seen a step change.
    let mut h = window(&e, None, true, 0.03);
    app_mut(&mut h).set_follow(true);
    click(&mut h, "Start");
    let mut seen = vec![];
    for _ in 0..5000 {
        h.step();
        std::thread::sleep(Duration::from_millis(2));
        if let Some(f) = app(&h).followed() {
            seen.push(f.to_owned());
        }
        if matches!(app(&h).tray, Some(Tray::Question { .. })) {
            break;
        }
    }
    assert!(!seen.is_empty(), "it followed something");
    assert!(seen.iter().all(|f| f != "drive"), "{seen:?}");
}

/// A topic typed and left without Enter is saved when the sidebar closes.
#[test]
fn typing_is_saved_when_the_sidebar_closes() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let i = app(&h).m.recipes.iter().position(|r| r.name == "Archive everything").unwrap();
    app_mut(&mut h).select(i);
    h.run_steps(3);
    open_id(&mut h, "a_tell");
    if h.query_all_by_role(Role::Tab).count() > 0 {
        tab(&mut h, "Settings");
    }
    let field = h.get_by_label_contains("ntfy topic");
    field.focus();
    h.run_steps(2);
    h.get_by_label_contains("ntfy topic").type_text("typed-not-entered");
    h.run_steps(2);
    app_mut(&mut h).close_tray();
    h.run_steps(2);
    let saved = app(&h).m.recipes[i].root.steps().into_iter().find_map(|n| match n {
        Node::Step { id, params, .. } if id == "a_tell" => params.get("ntfy_topic").and_then(|s| s.resolve(None)).and_then(|v| v.as_str().map(str::to_owned)),
        _ => None,
    });
    assert_eq!(saved.as_deref(), Some("typed-not-entered"));
}

#[test]
fn a_sector_order_found_by_testing_shows_on_the_drive_and_resets_to_be_found_again() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    let i = app(&h).sel;
    let drive = app(&h).m.drive(i);
    app(&h).m.engine.settings.lock().unwrap().set_drive(&drive, "sector_order", Setting::detected(Value::from("DATA_SUB_C2"), "found by testing"));
    click_tile(&mut h, "Drive,");
    h.run_steps(2);
    assert!(h.query_all_by_label_contains("found by testing").next().is_some());
    h.get_by_label("Reset How this drive hands over each sector").click_accesskit();
    h.run_steps(2);
    assert_eq!(app(&h).m.engine.settings.lock().unwrap().drive(&drive, "sector_order"), Setting::Default);
}

/// Stop is said at once: the recipe says Stopping… until the run has
/// actually stopped, however long the drive takes to answer.
#[test]
fn stop_says_stopping_at_once() {
    let e = env();
    let mut h = window(&e, None, true, 0.03);
    click(&mut h, "Start");
    h.run_steps(3);
    app_mut(&mut h).m.stop(0);
    assert!(app(&h).m.stopping(0) || app(&h).m.snapshot(0).stopped, "said before the run answers");
    if app(&h).m.stopping(0) {
        assert_eq!(app(&h).m.status_line(0).0, "Stopping…");
    }
    until(&mut h, "the stop", |a| a.m.snapshot(0).finished || a.m.snapshot(0).stopped);
    assert!(!app(&h).m.stopping(0), "and not once it has");
}

/// Rip and Check and fix for a game, in one shape (docs/ux/game-rip-sidebar.md),
/// in the four states it was drawn in: A, Rip reading through errors; B,
/// Rip done, its unreadable sectors handed on; C, Check and fix verified;
/// D, Check and fix holding the copy for you. Each at 1280 × 720 and
/// 1920 × 1080.
#[test]
fn the_game_rip_sidebars_in_their_four_states() {
    for (size, tag) in [(egui::vec2(1280.0, 720.0), ""), (egui::vec2(1920.0, 1080.0), "-1920")] {
        // A: the read slowed to a crawl; the window's Stop, no other.
        let e = env();
        let mut h = held_game(&e, At::Crawling, None, "g_rip", size);
        h.get_by_label("Slowed by read errors");
        h.get_by_label(drt_steps::real::redumper::CRAWLING);
        assert_eq!(h.get_all_by_label("Stop").count(), 1);
        h.get_by_label("Open tray");
        assert!(row_says(&h, &["1, Read the disc", "140 bad so far"]));
        h.get_by_label("Where on the disc");
        shot(&mut h, &format!("ripside-a-reading{tag}"));

        // B: made with 34 sectors blank, for Check and fix.
        let e = env();
        let mut h = held_game(&e, At::HandedOn, Some(CheckAt::Verified), "g_rip", size);
        h.get_by_label("The disc, 633 MB made");
        h.get_by_label("34 sectors couldn't be read");
        h.get_by_label("Left blank, for Check and fix to try again");
        assert!(row_says(&h, &["1, Read the disc", "1,370 bad"]));
        assert!(row_says(&h, &["2, Re-read bad parts", "1,336 put right", "20 tries each"]));
        assert!(row_says(&h, &["3, Write the tracks", "34 left blank"]));
        assert!(row_says(&h, &["4, Test every sector", "No others wrong"]));
        assert!(!row_says(&h, &["Write the tracks", "34 unreadable"]), "the try it stopped on is one row with the one that made it");
        h.get_by_label("Map of the disc: 267,789 good · 1,336 put right on a re-read · 34 unreadable");
        h.get_by_label("Two patches, about 4.4 cm and 4.9 cm out: each a single spot, once per turn.");
        assert!(h.query_by_label_contains("SCSI").is_none(), "the drive's counts are the log's");
        // Nothing needs you: it went on to Check and fix, as set.
        assert!(h.query_by_label_contains("Make the image anyway").is_none());
        shot(&mut h, &format!("ripside-b-handed-on{tag}"));

        // C: Check and fix put the 34 right, and it's Redump's.
        open_id(&mut h, "g_check");
        h.run_steps(3);
        h.get_by_label("The disc, 633 MB verified");
        h.get_by_label("Verified");
        h.get_by_label("Matches Redump's Lantern Keep (Europe) (Disc 2)");
        assert!(row_says(&h, &["From Rip: the image, 34 sectors blank"]));
        assert!(row_says(&h, &["1, Compare with Redump", "Doesn't match"]));
        assert!(row_says(&h, &["2, Read bad sectors again", "31 put right", "up to 10 tries each"]));
        assert!(row_says(&h, &["3, Rip again and combine", "3 taken from rip 2", "rip 2"]));
        assert!(row_says(&h, &["4, Compare with Redump", "Matches"]));
        h.get_by_label("Map of the disc: 269,125 as Rip made them · 34 put right");
        h.get_by_label("Size: Redump 269,159 sectors, this copy 269,159 sectors, the same");
        assert!(h.query_by_label_contains("Compares the copy's checksums").is_none(), "no sentence under the header");
        shot(&mut h, &format!("ripside-c-verified{tag}"));

        // Read again, and stopped as nothing came back: said plainly.
        let e = env();
        let mut h = held_game(&e, At::HandedOn, Some(CheckAt::Stopped), "g_check", size);
        shot(&mut h, &format!("ripside-stopped{tag}"));
        assert!(row_says(&h, &["2, Read bad sectors again", "9 put right", "3 skipped, the drive stalled"]));
        h.get_by_label(drt_steps::real::fix::STOPPED);
        h.get_by_label("Rip again and combine, SLES_147.11");

        // D: still 3 it can't read; only what can still help.
        let e = env();
        let mut h = held_game(&e, At::HandedOn, Some(CheckAt::Holding), "g_check", size);
        h.get_by_label("The disc, 633 MB not verified");
        h.get_by_label("Doesn't match Redump");
        h.get_by_label("3 sectors still not put right, after 4 rips");
        h.get_by_label("The drive was read with general settings: redumper doesn't know it. Another drive may read these 3.");
        h.get_by_label("Repair with the disc's error correction, SLES_147.11");
        h.get_by_label("Keep the best combination (not verified), SLES_147.11");
        h.get_by_label("Skip this disc, SLES_147.11");
        assert!(h.query_by_label_contains("Fix bad sectors").is_none() && h.query_by_label_contains("Rip again and combine again").is_none(), "what's been done isn't offered again");
        assert!(row_says(&h, &["3, Rip again and combine", "Nothing taken", "rips 2, 3 and 4"]));
        assert!(row_says(&h, &["4, Compare with Redump", "Doesn't match"]));
        h.get_by_label("Map of the disc: 269,125 as Rip made them · 31 put right · 3 unreadable");
        h.get_by_label_contains("SHA-1: Redump 3a647314…, this copy 91c0d2e5…, different");
        shot(&mut h, &format!("ripside-d-holding{tag}"));
    }
}

// ---- MakeMKV, only with your OK -------------------------------------------------------------

/// MakeMKV's hooks for a test: one found at a made-up path, and each time
/// it's run (for its version) noted.
fn makemkv_hooks(sandboxed: bool, host_allowed: bool, ran: Arc<Mutex<Vec<drt_steps::real::video::makemkv::Location>>>) -> drt_app::makemkv::Hooks {
    use drt_steps::real::video::makemkv::Location;
    drt_app::makemkv::Hooks {
        required: true,
        sandboxed,
        host_allowed: Arc::new(move || host_allowed),
        from_env: false,
        candidates: Arc::new(|| vec![Location::Program { path: "/usr/bin/makemkvcon".into() }, Location::Flatpak]),
        version: Arc::new(move |loc| {
            ran.lock().unwrap().push(loc);
            Box::pin(async { Ok("MakeMKV v1.18.2".to_owned()) })
        }),
        pick_file: Arc::new(|| None),
    }
}

/// A video recipe, with TMDb's key given so MakeMKV is all that's left.
fn makemkv_window(e: &Env) -> Harness<'static, Window> {
    let mut h = video_window(e, drt_steps::real::video::sim::SimDisc::Film);
    app(&h).m.engine.settings.lock().unwrap().set_app("tmdb", "api_key", Setting::chosen(Value::from("test-key")));
    h.run_steps(3);
    h
}

#[test]
fn a_video_recipe_waits_for_your_ok_to_run_makemkv_and_runs_it_once_when_given() {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let mut e = env();
    e.makemkv = makemkv_hooks(false, true, ran.clone());
    let mut h = makemkv_window(&e);
    let i = app(&h).sel;
    // Both steps that run it are in Needs, the list says so, and Start says why.
    let s = snap(&h);
    for id in ["v_disc", "v_rip"] {
        assert_eq!((s.steps[id].state, s.steps[id].status.as_str()), (StepState::Needs, "MakeMKV not set up"), "{id}");
    }
    assert_eq!(app(&h).m.status_line(i), ("MakeMKV not set up".to_owned(), StepState::Needs));
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    assert_eq!(app(&h).m.cant_start(i).as_deref(), Some("Video discs need MakeMKV: choose where it is first"));
    assert!(app_mut(&mut h).m.start(i).is_err(), "Start refuses too");
    // The step's sidebar says why, and offers what was found.
    click_tile(&mut h, "Video disc, MakeMKV not set up");
    h.get_by_label_contains("Disc Rip Trip only runs it if you say so.");
    h.get_by_label("Use this: /usr/bin/makemkvcon");
    h.get_by_label("Use this: MakeMKV's flatpak (com.makemkv.MakeMKV)");
    h.get_by_label("Choose makemkvcon");
    h.get_by_label("Get MakeMKV");
    shot(&mut h, "makemkv-not-set-up");
    assert!(ran.lock().unwrap().is_empty(), "nothing is run before you say so");
    assert!(!e.dir.path().join("config/makemkv").exists());
    // Use this: run once, for its version, then saved.
    h.get_by_label("Use this: /usr/bin/makemkvcon").click_accesskit();
    h.run_steps(2);
    until(&mut h, "its version", |a| a.m.makemkv.readiness() == drt_app::makemkv::Readiness::Ready);
    assert_eq!(ran.lock().unwrap().len(), 1, "run once, for its version");
    let saved = std::fs::read_to_string(e.dir.path().join("config/makemkv")).unwrap();
    assert!(saved.contains("/usr/bin/makemkvcon") && saved.contains("MakeMKV v1.18.2"), "{saved}");
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
    assert_eq!(snap(&h).steps["v_disc"].state, StepState::Idle);
    // Its page at the foot of the list: what's used, and its version;
    // the key switch is off until turned on.
    app_mut(&mut h).open_page(drt_app::Page::MakeMkv);
    h.run_steps(3);
    h.get_by_label("MakeMKV v1.18.2");
    h.get_by_label("MakeMKV, v1.18.2");
    let switch = h.get_by_label("Keep MakeMKV's beta key up to date");
    assert_eq!(switch.accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    assert!(!app(&h).m.makemkv.key_updates && !drt_steps::real::video::makemkv::key_updates());
    shot(&mut h, "makemkv-page");
    h.get_by_label("Keep MakeMKV's beta key up to date").click_accesskit();
    h.run_steps(2);
    assert_eq!(std::fs::read_to_string(e.dir.path().join("config/makemkv-key-updates")).unwrap(), "on");
    h.get_by_label("Keep MakeMKV's beta key up to date").click_accesskit();
    h.run_steps(2);
    // Revoked: back to asking, and nothing more was run.
    h.get_by_label("Don't use MakeMKV").click_accesskit();
    h.run_steps(3);
    assert!(!e.dir.path().join("config/makemkv").exists());
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    assert_eq!(ran.lock().unwrap().len(), 1);
}

#[test]
fn approved_makemkv_is_remembered_and_not_run_again_at_the_next_start() {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let mut e = env();
    e.makemkv = makemkv_hooks(false, true, ran.clone());
    std::fs::create_dir_all(e.dir.path().join("config")).unwrap();
    std::fs::write(e.dir.path().join("config/makemkv"), r#"{"location":{"kind":"flatpak"},"version":"MakeMKV v1.18.2"}"#).unwrap();
    let h = makemkv_window(&e);
    assert_eq!(app(&h).m.makemkv.readiness(), drt_app::makemkv::Readiness::Ready);
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled());
    assert!(ran.lock().unwrap().is_empty(), "its version is known: not run");
}

#[test]
fn in_a_flatpak_whose_permission_was_taken_away_makemkv_isnt_run_and_it_says_why() {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let mut e = env();
    e.makemkv = makemkv_hooks(true, false, ran.clone());
    let mut h = makemkv_window(&e);
    let i = app(&h).sel;
    click_tile(&mut h, "Video disc, MakeMKV not set up");
    h.get_by_label("Use this: MakeMKV's flatpak (com.makemkv.MakeMKV)").click_accesskit();
    h.run_steps(3);
    // Saved, but it can't be started from here, so it isn't run.
    assert_eq!(app(&h).m.makemkv.readiness(), drt_app::makemkv::Readiness::NeedsPermission);
    assert!(ran.lock().unwrap().is_empty());
    assert_eq!(snap(&h).steps["v_disc"].status, "MakeMKV can't be started");
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    assert!(app(&h).m.cant_start(i).unwrap().contains("turned off"));
    assert!(h.query_by_label_contains("flatpak override").is_none(), "no command to type");
    h.get_by_label_contains("Flatseal");
    h.get_by_label_contains("close Disc Rip Trip and open it again");
    shot(&mut h, "makemkv-permission");
    // Checking again can't see a change made after the app started.
    h.get_by_label("Check again").click_accesskit();
    h.run_steps(2);
    assert!(ran.lock().unwrap().is_empty());
}

// ---- Working files ----------------------------------------------------------------------------

/// A working folder under the test's `work/`, with these files, marked as
/// a settled run would mark it (`None`: as a run cut off would leave it).
fn work_folder(e: &Env, name: &str, files: &[&str], mark: Option<drt_app::workfiles::Mark>) -> PathBuf {
    let d = e.dir.path().join("work").join(name);
    for f in files {
        let p = d.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; 1000]).unwrap();
    }
    if let Some(m) = mark {
        drt_app::workfiles::write_mark(&d, &m);
    }
    d
}

fn backdate(p: &Path, by: Duration) {
    if p.is_dir() {
        for e in std::fs::read_dir(p).unwrap().flatten() {
            backdate(&e.path(), by);
        }
    }
    std::fs::File::open(p).unwrap().set_modified(std::time::SystemTime::now() - by).unwrap();
}

#[test]
fn working_files_never_filed_are_kept_listed_and_deleted_only_when_you_say() {
    use drt_app::workfiles::{Class, Mark};
    let e = env();
    let unfiled = work_folder(&e, "cds-1", &["flac/01.flac", "flac/02.flac", "flac/03.flac", "rip/01.wav"], Some(Mark { class: Class::Unfiled, title: "Tortoise – TNT".into() }));
    // Cut off mid-run long ago, with encodes in it: kept all the same.
    let cut_off = work_folder(&e, "cds-4", &["mp3/01.mp3", "rip/01.wav"], None);
    backdate(&cut_off, Duration::from_secs(40 * 24 * 3600));
    let filed = work_folder(&e, "cds-2", &["flac/01.flac"], Some(Mark { class: Class::Filed, title: String::new() }));
    let raw = work_folder(&e, "cds-3", &["rip/01.wav"], None);
    backdate(&raw, Duration::from_secs(25 * 3600));
    let fresh_raw = work_folder(&e, "cds-5", &["rip/01.wav"], None);
    let mut h = window(&e, None, true, 0.0);
    app(&h).m.sweep_work();
    until(&mut h, "the start-up sweep", |a| a.m.work.kept.len() == 2 && !a.m.work.kept.is_empty());
    let start = Instant::now();
    while filed.exists() || raw.exists() {
        assert!(start.elapsed() < Duration::from_secs(10));
        h.step();
    }
    assert!(!filed.exists() && !raw.exists(), "filed, and raw a day old: gone");
    assert!(unfiled.join("flac/01.flac").exists() && cut_off.join("mp3/01.mp3").exists() && fresh_raw.exists());
    h.get_by_label("Working files, 2 never filed");
    app_mut(&mut h).open_page(drt_app::Page::WorkingFiles);
    h.run_steps(3);
    h.get_by_label("Tortoise – TNT: 3 tracks, never filed");
    assert!(app(&h).m.work.kept.iter().any(|k| k.line.starts_with("A disc from ") && k.line.ends_with(": 1 track, never filed")), "{:?}", app(&h).m.work.kept);
    shot(&mut h, "working-files");
    h.get_by_label("Open folder: Tortoise – TNT: 3 tracks, never filed").click_accesskit();
    h.run_steps(2);
    assert_eq!(e.opened.lock().unwrap().last(), Some(&unfiled));
    // Delete asks first; Keep leaves it.
    h.get_by_label("Delete: Tortoise – TNT: 3 tracks, never filed").click_accesskit();
    h.run_steps(2);
    h.get_by_label_contains("for good?");
    h.get_by_label("Keep").click_accesskit();
    h.run_steps(2);
    assert!(unfiled.exists());
    h.get_by_label("Delete: Tortoise – TNT: 3 tracks, never filed").click_accesskit();
    h.run_steps(2);
    h.get_by_label("Delete").click_accesskit();
    h.run_steps(2);
    until(&mut h, "it's deleted", |a| a.m.work.kept.len() == 1);
    assert!(!unfiled.exists());
}

#[test]
fn a_held_runs_working_folder_is_never_swept() {
    let e = env();
    let mut h = window(&e, None, true, 0.0);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "the hold", |a| a.m.snapshot(0).steps["hold"].detail == "All here");
    // Its folder holds only what was read, and looks a day old: what a
    // sweep deletes, but not while a run uses it.
    let mine = app(&h).m.newest(0).unwrap().run.work().to_owned();
    std::fs::create_dir_all(mine.join("rip")).unwrap();
    std::fs::write(mine.join("rip/01.wav"), b"x").unwrap();
    backdate(&mine, Duration::from_secs(30 * 3600));
    let other = work_folder(&e, "cds-old", &["rip/01.wav"], None);
    backdate(&other, Duration::from_secs(30 * 3600));
    app(&h).m.sweep_work();
    let start = Instant::now();
    while other.exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "the sweep didn't run");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(mine.join("rip/01.wav").exists(), "a held run's folder stays");
    assert!(app(&h).m.ripping(0));
}

/// Puts a file in the newest run's working folder, as a step would.
fn in_work(h: &Harness<Window>, rel: &str) -> PathBuf {
    let w = app(h).m.newest(0).unwrap().run.work().to_owned();
    let p = w.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, b"x").unwrap();
    w
}

fn mark_of(dir: &Path, h: &mut Harness<Window>) -> drt_app::workfiles::Class {
    let start = Instant::now();
    loop {
        if let Some(m) = drt_app::workfiles::read_mark(dir) {
            return m.class;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "never marked");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_run_stopped_before_anything_was_encoded_goes_when_the_next_disc_replaces_it() {
    let e = env();
    let mut h = window(&e, None, true, 0.03);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    let first = in_work(&h, "rip/01.wav");
    click(&mut h, "Stop");
    until(&mut h, "the stop", |a| a.m.snapshot(0).finished);
    assert_eq!(mark_of(&first, &mut h), drt_app::workfiles::Class::Raw);
    assert!(first.exists(), "kept while it's shown");
    // The next disc: it's replaced, and its folder goes.
    std::thread::sleep(Duration::from_millis(1100));
    click(&mut h, "Start");
    until(&mut h, "the next disc", |a| a.m.runs[&0].len() == 1 && !a.m.snapshot(0).finished);
    let start = Instant::now();
    while first.exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "it wasn't deleted");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_run_stopped_mid_encode_keeps_its_encodes_and_lists_them() {
    let e = env();
    let mut h = window(&e, None, true, 0.03);
    click(&mut h, "Start");
    until(&mut h, "the question", |a| matches!(a.tray, Some(Tray::Question { .. })));
    click(&mut h, "Use this one");
    until(&mut h, "an encode", |a| a.m.snapshot(0).steps.values().any(|v| drt_app::tray::is_encoder(&v.step) && v.count(ItemState::Ok) > 0));
    let w = in_work(&h, "flac/01 - Track.flac");
    click(&mut h, "Stop");
    until(&mut h, "the stop", |a| a.m.snapshot(0).finished);
    assert_eq!(mark_of(&w, &mut h), drt_app::workfiles::Class::Unfiled);
    until(&mut h, "it's listed", |a| a.m.work.kept.iter().any(|k| k.path == w));
    // Replaced, it stays: it may be the only copy.
    std::thread::sleep(Duration::from_millis(1100));
    click(&mut h, "Start");
    until(&mut h, "the next disc", |a| a.m.runs[&0].len() == 1 && !a.m.snapshot(0).finished);
    for _ in 0..50 {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(w.join("flac/01 - Track.flac").exists());
}
