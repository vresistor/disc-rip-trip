//! The real Folder and Cue sheet, run by the engine on files made here: a
//! Folder never files over a different file without asking, asks once per
//! disc, and what comes after it sees the names the files were given.

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use trip_engine::recipe::chain;
use trip_engine::snapshot::QuestionShape;
use trip_engine::{Answer, Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Snapshot, Step, StepInfo, Subject};

/// Two encoded tracks, as an encoder would send them, with what the disc
/// knew. `bytes[i]` is track i+1's content.
struct Encoded {
    dir: PathBuf,
    bytes: [&'static str; 2],
}

#[async_trait]
impl Step for Encoded {
    fn info(&self) -> StepInfo {
        StepInfo::new("encoded", "Encoded", Group::Source, ItemKind::Nothing, ItemKind::AudioFile)
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        inlet.recv().await;
        std::fs::create_dir_all(&self.dir)?;
        for (i, title) in ["Total Football", "Violence"].iter().enumerate() {
            let t = i as u32 + 1;
            let p = self.dir.join(format!("{t:02}.flac"));
            std::fs::write(&p, self.bytes[i])?;
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

/// Runs Encoded → Folder → Cue sheet, answering any question with `pick`.
/// Gives the run and the questions it asked.
async fn run(base: &Path, bytes: [&'static str; 2], pick: usize) -> (Snapshot, Vec<String>) {
    let mut reg = trip_steps::real();
    reg.add(Encoded { dir: base.join(format!("src-{}", bytes.join("-"))), bytes });
    let mut e = Engine::new(reg);
    e.time_scale = 0.0;
    e.work_root = base.join("runs");
    let lib = base.join("Music").display().to_string();
    let recipe = Recipe { name: "Folder".into(), root: chain([Node::step("src", "encoded"), Node::step("lib", "folder").with("folder", lib.as_str()), Node::step("cue", "cue")]) };
    trip_engine::check(&recipe.root, &e.registry).unwrap();
    let run = e.start(&recipe).unwrap();
    let mut rx = run.snapshot.clone();
    let mut asked = vec![];
    loop {
        let s = rx.borrow_and_update().clone();
        if let Some((step, q)) = &s.question {
            if asked.len() < 5 && !asked.contains(&q.prompt) {
                asked.push(q.prompt.clone());
                assert!(
                    matches!(&q.shape, QuestionShape::PickOne { options, preselected: 1, .. } if options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Replace them", "Keep both", "Skip"])
                );
                run.send(Command::Answer { step: step.clone(), answer: Answer::Pick(pick) }).await;
            }
        }
        if s.finished || rx.changed().await.is_err() {
            break;
        }
    }
    (run.finished().await, asked)
}

fn album(base: &Path) -> PathBuf {
    base.join("Music/Parquet Courts/Wide Awake!")
}

fn cue(base: &Path) -> String {
    std::fs::read_to_string(album(base).join("Wide Awake!.cue")).unwrap()
}

fn names(base: &Path) -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(album(base)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".flac")).collect();
    n.sort();
    n
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_asks_once_and_the_cue_sheet_gets_the_names_it_chose() {
    let base = std::env::current_dir().unwrap().join("../../target/trip-test/folder-names");
    let _ = std::fs::remove_dir_all(&base);

    // First time: nothing there, nothing asked.
    let (s, asked) = run(&base, ["a1", "a2"], 1).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(s.steps["lib"].status, "2 filed");
    assert_eq!(names(&base), ["01 - Total Football.flac", "02 - Violence.flac"]);

    // The same files again: they're filed already; no question.
    let (s, asked) = run(&base, ["a1", "a2"], 1).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(s.steps["lib"].status, "2 filed");
    assert!(s.steps["lib"].items.iter().all(|i| i.state == ItemState::Ok && i.note.contains("already there")));

    // Both different: one question for the disc, and Keep both.
    let (s, asked) = run(&base, ["b1", "b2"], 1).await;
    assert_eq!(asked, ["2 files already in Parquet Courts / Wide Awake!"]);
    assert_eq!(names(&base), ["01 - Total Football (2).flac", "01 - Total Football.flac", "02 - Violence (2).flac", "02 - Violence.flac"]);
    assert_eq!(std::fs::read_to_string(album(&base).join("02 - Violence.flac")).unwrap(), "a2", "the old file is untouched");
    let c = cue(&base);
    assert!(c.contains("FILE \"01 - Total Football (2).flac\" WAVE") && c.contains("FILE \"02 - Violence (2).flac\" WAVE"), "{c}");
    assert!(s.steps["cue"].state == trip_engine::StepState::Ok, "{:?}", s.steps["cue"]);

    // Keep both again with the same new files: " (2)" holds them already,
    // so nothing more is made.
    let (_, asked) = run(&base, ["b1", "b2"], 1).await;
    assert_eq!(asked.len(), 1);
    assert_eq!(names(&base).len(), 4, "{:?}", names(&base));

    // One new, one the same: only the new one is asked about. Skip.
    let (s, asked) = run(&base, ["a1", "c2"], 2).await;
    assert_eq!(asked, ["1 file already in Parquet Courts / Wide Awake!"]);
    let skipped = s.steps["lib"].items.iter().find(|i| i.id == "track-2").unwrap();
    assert_eq!(skipped.state, ItemState::Skipped);
    assert_eq!(std::fs::read_to_string(album(&base).join("02 - Violence.flac")).unwrap(), "a2");

    // Replace.
    let (_, asked) = run(&base, ["a1", "c2"], 0).await;
    assert_eq!(asked.len(), 1);
    assert_eq!(std::fs::read_to_string(album(&base).join("02 - Violence.flac")).unwrap(), "c2");
    assert_eq!(names(&base).len(), 4);
    std::fs::remove_dir_all(base).unwrap();
}

/// Each answer to *2 files already in …*, from a folder holding only the
/// first copies: Replace overwrites them, Keep both adds " (2)", Skip
/// files nothing and sends nothing on.
#[tokio::test(flavor = "multi_thread")]
async fn each_answer_does_what_it_says() {
    for (pick, what) in [(0, "replace"), (1, "keep-both"), (2, "skip")] {
        let base = std::env::current_dir().unwrap().join(format!("../../target/trip-test/folder-answer-{what}"));
        let _ = std::fs::remove_dir_all(&base);
        run(&base, ["a1", "a2"], pick).await;
        let before = cue(&base);
        let (s, asked) = run(&base, ["b1", "b2"], pick).await;
        assert_eq!(asked, ["2 files already in Parquet Courts / Wide Awake!"], "{what}");
        let read = |name: &str| std::fs::read_to_string(album(&base).join(name)).unwrap();
        let lib = &s.steps["lib"];
        match what {
            "replace" => {
                assert_eq!(names(&base), ["01 - Total Football.flac", "02 - Violence.flac"], "no \" (2)\"");
                assert_eq!((read("01 - Total Football.flac"), read("02 - Violence.flac")), ("b1".into(), "b2".into()), "overwritten");
                assert!(lib.items.iter().all(|i| i.state == ItemState::Ok), "{:?}", lib.items);
                assert_eq!(lib.status, "2 filed");
                assert!(cue(&base).contains("FILE \"02 - Violence.flac\" WAVE"));
            }
            "keep-both" => {
                assert_eq!(names(&base), ["01 - Total Football (2).flac", "01 - Total Football.flac", "02 - Violence (2).flac", "02 - Violence.flac"]);
                assert_eq!((read("01 - Total Football.flac"), read("02 - Violence.flac")), ("a1".into(), "a2".into()), "the old ones untouched");
                assert_eq!((read("01 - Total Football (2).flac"), read("02 - Violence (2).flac")), ("b1".into(), "b2".into()));
                assert!(cue(&base).contains("FILE \"02 - Violence (2).flac\" WAVE"));
            }
            _ => {
                assert_eq!(names(&base), ["01 - Total Football.flac", "02 - Violence.flac"], "nothing new");
                assert_eq!((read("01 - Total Football.flac"), read("02 - Violence.flac")), ("a1".into(), "a2".into()), "untouched");
                assert!(lib.items.iter().all(|i| i.state == ItemState::Skipped), "{:?}", lib.items);
                assert!(s.steps["cue"].items.is_empty(), "nothing went on: {:?}", s.steps["cue"].items);
                assert_eq!(cue(&base), before, "the cue sheet is the first one's");
            }
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
