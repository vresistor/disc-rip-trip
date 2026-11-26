//! The real Folder when its folder goes away mid-run (a network share
//! dropped, a drive pulled): it holds, says what it waits for, and files by
//! itself once the folder is back. Told to skip, it keeps the files in the
//! working folder. Nothing ripped is lost.

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Notify;
use trip_engine::recipe::chain;
use trip_engine::snapshot::QuestionShape;
use trip_engine::{Answer, Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Snapshot, Step, StepInfo, StepState, Subject};

/// Two encoded tracks, made in the run's working folder as an encoder
/// makes them; the second only once `gate` is let go.
struct Gated {
    gate: Arc<Notify>,
}

#[async_trait]
impl Step for Gated {
    fn info(&self) -> StepInfo {
        StepInfo::new("gated", "Gated", Group::Source, ItemKind::Nothing, ItemKind::AudioFile)
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        inlet.recv().await;
        let work = cx.work()?;
        for (i, title) in ["Total Football", "Violence"].iter().enumerate() {
            let t = i as u32 + 1;
            if t == 2 {
                self.gate.notified().await;
            }
            let p = work.join(format!("{t:02}.flac"));
            std::fs::write(&p, format!("track {t}"))?;
            let mut item = Item::new(format!("track-{t}"), ItemKind::AudioFile, *title, Some(t), Some(p));
            let c = item.context_mut();
            c.add(Subject::Disc, "parts", "2", "the disc");
            c.add(Subject::Disc, "album", "Wide Awake!", "the disc");
            c.add(Subject::Disc, "album_artist", "Parquet Courts", "the disc");
            c.add(Subject::Part(t), "title", *title, "the disc");
            outlet.send(item).await;
        }
        Ok(())
    }
}

fn album(lib: &Path) -> PathBuf {
    lib.join("Parquet Courts/Wide Awake!")
}

/// The folder gone: a file where it was, so nothing can be made there.
fn take_away(lib: &Path) {
    std::fs::remove_dir_all(lib).unwrap();
    std::fs::write(lib, "").unwrap();
}

fn bring_back(lib: &Path) {
    std::fs::remove_file(lib).unwrap();
    std::fs::create_dir_all(lib).unwrap();
}

/// Starts Gated → Folder on `lib`, looking again every few milliseconds.
fn start(base: &Path, gate: Arc<Notify>, lib: &Path) -> (Engine, trip_engine::Run) {
    let mut reg = trip_steps::real();
    reg.add(Gated { gate });
    let mut e = Engine::new(reg);
    // Small, not 0: at 0 a wait only yields, and the Folder's looks would spin.
    e.time_scale = 0.001;
    e.work_root = base.join("runs");
    let recipe = Recipe { name: "Away".into(), root: chain([Node::step("src", "gated"), Node::step("lib", "folder").with("folder", lib.display().to_string().as_str())]) };
    trip_engine::check(&recipe.root, &e.registry).unwrap();
    let run = e.start(&recipe).unwrap();
    (e, run)
}

/// Waits for the run to come to a snapshot `f` likes.
async fn until(run: &trip_engine::Run, what: &str, f: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let mut rx = run.snapshot.clone();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let s = rx.borrow_and_update().clone();
        if f(&s) {
            return s;
        }
        assert!(!s.finished, "finished before {what}: {:?}", s.steps["lib"]);
        if tokio::time::timeout_at(deadline, rx.changed()).await.is_err() {
            panic!("no {what}: {:?}", s.steps["lib"]);
        }
    }
}

fn track(s: &Snapshot, n: u32) -> ItemState {
    s.steps["lib"].items.iter().find(|i| i.id == format!("track-{n}")).map_or(ItemState::Waiting, |i| i.state)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_goes_away_holds_then_files_once_it_is_back() {
    let base = tempfile::tempdir().unwrap();
    let lib = base.path().join("Music");
    std::fs::create_dir_all(&lib).unwrap();
    let gate = Arc::new(Notify::new());
    let (_e, run) = start(base.path(), gate.clone(), &lib);

    // The first track filed; then the folder goes, and the second comes.
    until(&run, "the first track filed", |s| track(s, 1) == ItemState::Ok).await;
    take_away(&lib);
    gate.notify_one();
    let s = until(&run, "a hold", |s| s.question.is_some()).await;
    let (step, q) = s.question.clone().unwrap();
    assert_eq!(step, "lib");
    assert!(q.prompt.starts_with("Waiting for ") && q.prompt.ends_with(" to come back"), "{}", q.prompt);
    let QuestionShape::PickOne { options, .. } = &q.shape else { panic!("{q:?}") };
    assert_eq!(options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["Choose another folder", "Skip"]);
    assert_eq!(s.steps["lib"].state, StepState::Needs);
    assert_eq!(track(&s, 2), ItemState::Waiting);
    // Its file is still where the encoder left it.
    let work: Vec<PathBuf> = std::fs::read_dir(base.path().join("runs")).unwrap().map(|e| e.unwrap().path()).collect();
    assert!(work[0].join("02.flac").exists());

    // Back: filed by itself, the question gone, without an answer.
    bring_back(&lib);
    let s = run.finished().await;
    assert!(s.question.is_none());
    assert_eq!(track(&s, 2), ItemState::Ok, "{:?}", s.steps["lib"]);
    assert_eq!(std::fs::read_to_string(album(&lib).join("02 - Violence.flac")).unwrap(), "track 2");
    assert_eq!(s.steps["lib"].state, StepState::Ok, "{:?}", s.steps["lib"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn skipped_while_away_the_files_stay_in_the_working_folder() {
    let base = tempfile::tempdir().unwrap();
    let lib = base.path().join("Music");
    std::fs::write(&lib, "").unwrap();
    let gate = Arc::new(Notify::new());
    gate.notify_one();
    let (_e, run) = start(base.path(), gate, &lib);
    let s = until(&run, "a hold", |s| s.question.is_some()).await;
    run.send(Command::Answer { step: "lib".into(), answer: Answer::Pick(1) }).await;
    let s2 = run.finished().await;
    // Asked once for the disc, not once a track.
    assert!(s.question.is_some() && s2.question.is_none());
    assert!(s2.steps["lib"].items.iter().all(|i| i.state == ItemState::Skipped && i.note.starts_with("not filed: kept in ")), "{:?}", s2.steps["lib"].items);
    let work: Vec<PathBuf> = std::fs::read_dir(base.path().join("runs")).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(work.len(), 1, "the working folder is kept");
    assert!(work[0].join("01.flac").exists() && work[0].join("02.flac").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn another_folder_can_be_chosen_for_the_disc() {
    let base = tempfile::tempdir().unwrap();
    let lib = base.path().join("Music");
    std::fs::write(&lib, "").unwrap();
    let other = base.path().join("Elsewhere");
    let gate = Arc::new(Notify::new());
    gate.notify_one();
    let (_e, run) = start(base.path(), gate, &lib);
    until(&run, "a hold", |s| s.question.is_some()).await;
    run.send(Command::Answer { step: "lib".into(), answer: Answer::Pick(0) }).await;
    let s = until(&run, "which folder", |s| s.question.as_ref().is_some_and(|(_, q)| matches!(q.shape, QuestionShape::Value { folder: true, .. }))).await;
    assert_eq!(s.question.unwrap().1.prompt, "Which folder should this disc go in?");
    run.send(Command::Answer { step: "lib".into(), answer: Answer::Value(other.display().to_string()) }).await;
    let s = run.finished().await;
    assert!(s.steps["lib"].items.iter().all(|i| i.state == ItemState::Ok), "{:?}", s.steps["lib"].items);
    assert!(album(&other).join("01 - Total Football.flac").exists() && album(&other).join("02 - Violence.flac").exists());
}
