//! `trip [--simulate]`: the window. `--simulate` runs simulated
//! steps (no drive, no network, no files) and keeps its recipes and
//! settings in a `simulate` folder of their own, so a made-up offset never
//! reaches a real rip.

use serde_json::Value;
use std::sync::Arc;
use trip_app::{App, Deps, Model, Store, Window};
use trip_engine::{Engine, Setting};
use trip_steps::recipes;

fn main() -> eframe::Result<()> {
    let simulate = std::env::args().any(|a| a == "--simulate");
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("tokio");
    let mut dir = Store::default_dir();
    let (mut engine, drives) = if simulate {
        dir = dir.join("simulate");
        let mut e = Engine::new(trip_steps::simulated(None));
        e.time_scale = 0.3;
        (e, vec![recipes::BLU_RAY.to_owned(), recipes::DVD.to_owned()])
    } else {
        (Engine::new(trip_steps::real()), trip_steps::real::drives().into_iter().map(|(name, _)| name).collect())
    };
    let store = Store::new(dir);
    // The flatpak keeps its own config folder; the first time, it takes the
    // recipes a build from source made.
    if !simulate && std::path::Path::new("/.flatpak-info").exists() {
        if let Some(home) = std::env::var_os("HOME") {
            for f in store.adopt(&std::path::PathBuf::from(home).join(".config/trip")) {
                eprintln!("copied {} into {}", f.display(), store.dir.display());
            }
        }
    }
    // Simulated drives start out measured; Model::new replaces this with
    // the saved settings once there are any.
    if simulate {
        let mut s = engine.settings.lock().unwrap();
        s.set_drive(recipes::BLU_RAY, "offset", Setting::detected(Value::from(103), "AccurateRip"));
        s.set_drive(recipes::DVD, "offset", Setting::detected(Value::from(6), "AccurateRip"));
    }
    if simulate {
        engine.work_root = std::env::temp_dir().join("trip-simulate");
    }
    let deps = Deps {
        engine,
        store,
        drives,
        rt: rt.handle().clone(),
        open_folder: Arc::new(|p| opener::open(p).map_err(|e| e.to_string())),
        pick_folder: Arc::new(|start| {
            let d = rfd::FileDialog::new();
            match start {
                Some(dir) => d.set_directory(dir),
                None => d,
            }
            .pick_folder()
        }),
        // Simulated drives are always there; real ones as sysfs says.
        send_test: Arc::new(|desktop, server, topic| Box::pin(async move { trip_steps::real::tell::send_test(desktop, &server, &topic).await })),
        // A simulated drive has no tray to open.
        open_tray: if simulate { Arc::new(|_| Ok(())) } else { Arc::new(trip_steps::real::open_tray) },
        // Simulated steps write nothing, so their folders aren't looked at.
        reach: if simulate { Arc::new(|_, _| Ok(())) } else { Arc::new(|p, need| trip_steps::real::reach::look(p, need, trip_steps::real::reach::statvfs_free)) },
        // The simulation's video steps don't run MakeMKV.
        makemkv: if simulate { trip_app::makemkv::Hooks::not_needed() } else { trip_app::makemkv::Hooks::real() },
        connected: if simulate { Arc::new(|| vec![recipes::BLU_RAY.to_owned(), recipes::DVD.to_owned()]) } else { Arc::new(|| trip_steps::real::drives().into_iter().map(|(name, _)| name).collect()) },
    };
    let mut model = Model::new(deps);
    // Working folders filed, or raw and a day old, from earlier sessions.
    model.sweep_work();
    if !simulate {
        model.missing_tools = trip_steps::real::tool::missing(&trip_steps::real::tool::BUNDLED);
    }
    if simulate {
        // A simulated drive never waits for a disc, so Next disc would go
        // round for ever: three more discs after each Start.
        model.repeat_limit = Some(3);
    }
    if let Some(why) = &model.store.recipes_unreadable {
        eprintln!("{why}\nUsing the built-in recipes; your file is left as it is.");
    }
    let opts = eframe::NativeOptions {
        // No title bar from the system: the window draws its own frame.
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size(trip_app::chrome::MIN_SIZE)
            .with_decorations(false)
            .with_transparent(true)
            .with_title("Trip")
            .with_app_id(trip_app::makemkv::APP_ID),
        ..Default::default()
    };
    eframe::run_native(
        "Trip",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_fonts(trip_app::theme::fonts());
            let dark = cc.egui_ctx.system_theme() == Some(egui::Theme::Dark);
            let saved = model.store.load_appearance().and_then(|s| trip_app::Appearance::parse(&s)).unwrap_or(trip_app::Appearance::System);
            let mut app = App::new(model, dark);
            app.appearance = saved;
            match saved {
                trip_app::Appearance::Light => app.dark = false,
                trip_app::Appearance::Dark => app.dark = true,
                trip_app::Appearance::System => {}
            }
            Ok(Box::new(Keep { window: Window(app), _rt: rt }))
        }),
    )
}

/// The window, and the runtime its runs live on.
struct Keep {
    window: Window,
    _rt: tokio::runtime::Runtime,
}

impl eframe::App for Keep {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.window, ui, frame);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        eframe::App::clear_color(&self.window, visuals)
    }
}
