//! Data moves only along wires: a step knows what its items carry, and a
//! Choose chooses among the candidates the lookups wired into it offered.
//! The real CD-Text and Choose, run by the engine with no drive and no
//! network: a test source stands in for Drive and Audio CD, putting on the
//! disc what they would.

use anyhow::Result;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use trip_engine::recipe::{branches, chain};
use trip_engine::snapshot::QuestionShape;
use trip_engine::{Answer, Candidate, Command, Cx, Engine, Fact, Group, Inlet, Item, ItemKind, Node, Outlet, Recipe, Snapshot, Step, StepInfo, Subject};

const TITLES: [&str; 3] = ["JIMMY THE EXPLODER", "STOP BREAKING DOWN", "THE BIG THREE KILLED MY BABY"];

/// The disc as Drive and Audio CD leave it: the drive's offset, the table
/// of contents, and the disc's own CD-Text kept aside. In Audio CD's place,
/// so the lookups may follow it. Without `offset`, a drive whose offset
/// isn't known.
struct Disc {
    offset: bool,
}

#[async_trait]
impl Step for Disc {
    fn info(&self) -> StepInfo {
        StepInfo::new("audio-cd", "Audio CD", Group::Kind, ItemKind::Nothing, ItemKind::AudioDisc).gives(&["toc"])
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        inlet.recv().await;
        let mut disc = Item::disc("disc").becomes(ItemKind::AudioDisc, None);
        let c = disc.context_mut();
        c.add(Subject::Disc, "device", "/dev/sr-none", "the drive");
        if self.offset {
            c.add(Subject::Disc, "offset", "103", "the drive");
        }
        c.add(Subject::Disc, "parts", TITLES.len().to_string(), "the disc");
        c.add(Subject::Disc, "cdtext.album", "THE WHITE STRIPES", "the disc");
        c.add(Subject::Disc, "cdtext.artist", "THE WHITE STRIPES", "the disc");
        for (i, t) in TITLES.iter().enumerate() {
            c.add(Subject::Part(i as u32 + 1), "length", "150", "the disc");
            c.add(Subject::Part(i as u32 + 1), "cdtext.title", *t, "the disc");
        }
        outlet.send(disc).await;
        Ok(())
    }
}

/// A database lookup that always offers two releases, the first ahead.
struct TwoReleases;

#[async_trait]
impl Step for TwoReleases {
    fn info(&self) -> StepInfo {
        StepInfo::new("test-db", "Database", Group::LookUp, ItemKind::AudioDisc, ItemKind::AudioDisc).gives(&["candidates"]).unless(&["release"]).right_after(&["audio-cd"])
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let release = |id: &str, year: &str, score: f32| Candidate {
                id: id.into(),
                source: "Database".into(),
                title: "The White Stripes".into(),
                detail: year.into(),
                score,
                facts: vec![
                    Fact { subject: Subject::Disc, key: "album".into(), value: "The White Stripes".into(), source: "Database".into(), step: String::new() },
                    Fact { subject: Subject::Part(1), key: "title".into(), value: "Jimmy the Exploder".into(), source: "Database".into(), step: String::new() },
                ],
                step: String::new(),
            };
            disc.context_mut().offer([release("db-1999", "1999", 0.9), release("db-2002", "2002", 0.8)]);
            outlet.send(disc).await;
        }
        Ok(())
    }
}

/// Keeps every disc that reaches it, by its node id.
#[derive(Clone, Default)]
struct Probe(Arc<Mutex<BTreeMap<String, Item>>>);

#[async_trait]
impl Step for Probe {
    fn info(&self) -> StepInfo {
        StepInfo::new("test-probe", "Probe", Group::Tell, ItemKind::AudioDisc, ItemKind::Nothing)
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(disc) = inlet.recv().await {
            self.0.lock().unwrap().insert(cx.id().to_owned(), disc);
        }
        Ok(())
    }
}

/// Runs a recipe on the real steps and these, answering each question by
/// `answer` (given the asking step's id).
async fn run(root: Node, answer: impl Fn(&str, &QuestionShape) -> Answer) -> (Snapshot, BTreeMap<String, Item>) {
    run_on(Disc { offset: true }, root, answer).await
}

async fn run_on(disc: Disc, root: Node, answer: impl Fn(&str, &QuestionShape) -> Answer) -> (Snapshot, BTreeMap<String, Item>) {
    let probe = Probe::default();
    let mut reg = trip_steps::real();
    reg.add(disc);
    reg.add(TwoReleases);
    reg.add(probe.clone());
    let mut e = Engine::new(reg);
    e.time_scale = 0.0;
    e.work_root = std::env::temp_dir().join(format!("trip-wires-{}", std::process::id()));
    let recipe = Recipe { name: "Wires".into(), root };
    trip_engine::check(&recipe.root, &e.registry).unwrap();
    let run = e.start(&recipe).unwrap();
    let mut rx = run.snapshot.clone();
    let mut answered = vec![];
    loop {
        let s = rx.borrow_and_update().clone();
        if let Some((step, q)) = &s.question {
            if !answered.contains(step) {
                answered.push(step.clone());
                run.send(Command::Answer { step: step.clone(), answer: answer(step, &q.shape) }).await;
            }
        }
        if s.finished || rx.changed().await.is_err() {
            break;
        }
    }
    let snap = run.finished().await;
    let got = probe.0.lock().unwrap().clone();
    (snap, got)
}

#[tokio::test]
async fn cd_text_is_an_option_in_choose_and_choosing_it_names_the_tracks() {
    let root = chain([Node::step("src", "audio-cd"), Node::step("cdtext", "cd-text"), Node::step("choose", "choose"), Node::step("probe", "test-probe")]);
    let (s, got) = run(root, |_, shape| {
        let QuestionShape::PickOne { options, details, .. } = shape else { panic!("{shape:?}") };
        // The disc's own text alone: offered, never taken unasked.
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].label, "From the disc (CD-Text)");
        assert_eq!(options[0].detail, "THE WHITE STRIPES — THE WHITE STRIPES, 3 track names");
        // The form starts from the candidate, nothing else.
        assert_eq!(details.as_ref().unwrap()["title.2"], "STOP BREAKING DOWN");
        Answer::Pick(0)
    })
    .await;
    assert_eq!(s.steps["choose"].record.options.iter().flatten().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["cd-text"]);
    let d = s.steps["choose"].record.decided.clone().unwrap();
    assert_eq!((d.chosen.as_deref(), d.why.as_str()), (Some("cd-text"), "you picked"));
    // What leaves Choose: the chosen identity, the rest dropped, and what
    // the drive put on the disc still there for Rip.
    let disc = &got["probe"];
    let c = disc.context();
    assert_eq!(c.chosen().map(|x| x.id.as_str()), Some("cd-text"));
    assert!(c.candidates().is_empty());
    assert_eq!(c.get(Subject::Disc, "offset"), Some("103"));
    // Tracks are tagged and named from it.
    let tags = trip_steps::real::encode::tags(c, 1);
    assert!(tags.contains(&("title".into(), "JIMMY THE EXPLODER".into())), "{tags:?}");
    assert!(tags.contains(&("album".into(), "THE WHITE STRIPES".into())), "{tags:?}");
    let f = trip_steps::real::fields_in(c, 3, "Track 3");
    assert_eq!((f.title.as_str(), f.albumartist.as_str()), ("THE BIG THREE KILLED MY BABY", "THE WHITE STRIPES"));
}

#[tokio::test]
async fn a_choose_sees_only_the_lookups_wired_into_it() {
    // Two Chooses on branches: one fed by CD-Text alone (it asks), one by
    // a database alone (it needn't). Neither sees what the other branch's
    // lookup offered.
    let root = chain([
        Node::step("src", "audio-cd"),
        branches([
            chain([Node::step("cdtext", "cd-text"), Node::step("c1", "choose"), Node::step("p1", "test-probe")]),
            chain([Node::step("db", "test-db"), Node::step("c2", "choose"), Node::step("p2", "test-probe")]),
        ]),
    ]);
    let (s, got) = run(root, |step, shape| {
        let QuestionShape::PickOne { options, .. } = shape else { panic!() };
        assert_eq!(step, "c1");
        assert_eq!(options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["From the disc (CD-Text)"]);
        Answer::Pick(0)
    })
    .await;
    let ids = |step: &str| s.steps[step].record.options.iter().flatten().map(|c| c.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids("c1"), ["cd-text"]);
    assert_eq!(ids("c2"), ["db-1999", "db-2002"]);
    assert_eq!(got["p1"].context().chosen().unwrap().id, "cd-text");
    assert_eq!(got["p2"].context().chosen().unwrap().id, "db-1999");
    assert_eq!(s.steps["c2"].record.decided.as_ref().unwrap().why, "best match 0.90");
    assert_eq!(got["p2"].context().get(Subject::Part(1), "title"), Some("Jimmy the Exploder"));
}

#[tokio::test]
async fn where_branches_join_the_item_knows_what_each_learned() {
    // CD-Text and the database side by side, joined at Choose: both offers
    // reach it, each once, in the order of the wires; the database's
    // match is clearly ahead of the disc's own text.
    let root = chain([Node::step("src", "audio-cd"), branches([Node::step("db", "test-db"), Node::step("cdtext", "cd-text")]), Node::step("choose", "choose"), Node::step("probe", "test-probe")]);
    let (s, got) = run(root, |_, q| panic!("asked {q:?}")).await;
    let ids: Vec<&str> = s.steps["choose"].record.options.iter().flatten().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["db-1999", "db-2002", "cd-text"]);
    let c = got["probe"].context();
    assert_eq!(c.chosen().unwrap().id, "db-1999");
    assert_eq!(c.all(Subject::Disc, "offset").count(), 1, "what both branches carried isn't doubled");
}

#[tokio::test]
async fn rip_takes_the_offset_from_the_disc_and_never_reads_without_it() {
    // The drive put no offset on the disc: Rip refuses before touching it,
    // whatever any other part of the run might know.
    let root = chain([Node::step("src", "audio-cd"), Node::step("rip", "rip")]);
    let (s, _) = run_on(Disc { offset: false }, root, |_, q| panic!("asked {q:?}")).await;
    let rip = &s.steps["rip"];
    assert_eq!(rip.state, trip_engine::StepState::Bad);
    assert!(rip.detail.contains("the drive's read offset isn't known"), "{}", rip.detail);
}
