//! Measuring again, when a drive's offset is already known.

use serde_json::Value;
use trip_engine::{Engine, Setting};
use trip_steps::{recipes, simulated};

#[tokio::test]
async fn measuring_again_replaces_a_known_offset() {
    let e = Engine::new(simulated(None));
    let r = recipes::cd();
    e.settings.lock().unwrap().set_drive(recipes::BLU_RAY, "offset", Setting::chosen(Value::from(99)));
    // A plain lookup leaves a known value alone.
    assert!(e.find_missing(&r, false).await.is_empty());
    assert_eq!(e.settings.lock().unwrap().drive(recipes::BLU_RAY, "offset").resolve(None), Some(Value::from(99)));
    // Measuring finds it again and records where it came from.
    assert_eq!(e.find_again(&r, true).await, 1);
    let s = e.settings.lock().unwrap().drive(recipes::BLU_RAY, "offset");
    assert_eq!(s.resolve(None), Some(Value::from(103)));
    assert_ne!(s.provenance(), "yours");
}

#[test]
fn a_data_disc_recipe_doesnt_need_the_read_offset() {
    let e = Engine::new(simulated(None));
    let data = recipes::all().into_iter().find(|r| r.name == "Data discs").unwrap();
    assert!(e.missing(&data).is_empty(), "{:?}", e.missing(&data));
    assert_eq!(e.missing(&recipes::cd()).len(), 1, "the CD recipe still does");
}

#[test]
fn a_recipe_saved_with_library_steps_still_checks() {
    use trip_engine::recipe::chain;
    use trip_engine::Node;
    let reg = simulated(None);
    let old = chain([Node::step("d", "drive"), Node::step("cd", "audio-cd"), Node::step("rip", "rip"), Node::step("f", "flac"), Node::step("l", "library")]);
    trip_engine::check(&old, &reg).unwrap();
    assert_eq!(reg.info("library").unwrap().kind, "folder");
    assert!(!trip_engine::fits(trip_engine::ItemKind::AudioFile, &reg).iter().any(|i| i.kind == "library"), "not offered as a second step");
}

#[tokio::test]
async fn a_clean_run_leaves_nothing_in_its_working_folder() {
    use trip_engine::{Answer, Command};
    let mut e = Engine::new(simulated(None));
    e.time_scale = 0.0;
    let dir = std::env::temp_dir().join(format!("trip-tidy-{}", std::process::id()));
    e.work_root = dir.clone();
    e.settings.lock().unwrap().set_drive(recipes::BLU_RAY, "offset", Setting::chosen(Value::from(103)));
    // A recipe with no question or hold, that writes into its working folder.
    let r = recipes::all().into_iter().find(|r| r.name == "Data discs").unwrap();
    let run = e.start(&r).unwrap();
    let mut rx = run.snapshot.clone();
    loop {
        let s = rx.borrow_and_update().clone();
        if let Some((step, _)) = &s.question {
            run.send(Command::Answer { step: step.clone(), answer: Answer::Pick(0) }).await;
        }
        if s.finished || rx.changed().await.is_err() {
            break;
        }
    }
    let s = run.finished().await;
    assert!(s.finished && !s.stopped);
    assert_eq!(std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0), 0, "nothing left behind");
    let _ = std::fs::remove_dir_all(dir);
}
