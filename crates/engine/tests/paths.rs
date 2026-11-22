//! A step's own paths, besides its usual one: what's on its *If it fails*
//! path hears of it once when it fails, and what's on its *If it needs
//! you* path once when it asks something. Neither holds the run up.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::chain;
use drt_engine::snapshot::{Choice, Question, QuestionShape};
use drt_engine::{check, Answer, CheckError, Command, Cx, Engine, Group, Inlet, Item, ItemKind, Node, Outlet, Recipe, Registry, Side, Snapshot, Step, StepInfo, StepState, Subject};

struct Start;

#[async_trait]
impl Step for Start {
    fn info(&self) -> StepInfo {
        StepInfo::new("start", "Start", Group::Source, ItemKind::Nothing, ItemKind::Disc)
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        inlet.recv().await;
        let mut d = Item::disc("d");
        d.context_mut().add(Subject::Disc, "name", "Koudelka (Europe) (Disc 4)", "a catalogue");
        outlet.send(d).await;
        Ok(())
    }
}

/// Fails with a reason, after a moment.
struct Breaks;

#[async_trait]
impl Step for Breaks {
    fn info(&self) -> StepInfo {
        StepInfo::new("breaks", "Rip", Group::Read, ItemKind::Disc, ItemKind::Disc)
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, cx: Cx) -> Result<()> {
        inlet.recv().await;
        cx.state(StepState::Busy);
        cx.status("Reading", "");
        anyhow::bail!("couldn't read 774 sectors")
    }
}

/// Asks which game it is, then passes the disc on.
struct Asks;

#[async_trait]
impl Step for Asks {
    fn info(&self) -> StepInfo {
        StepInfo::new("asks", "Choose", Group::Decide, ItemKind::Disc, ItemKind::Disc)
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let d = inlet.recv().await;
        let q = Question {
            prompt: "Which game is this?".into(),
            shape: QuestionShape::PickOne { options: vec![Choice { label: "a".into(), detail: String::new(), destructive: false, picture: None }], preselected: 0, details: None, skip: false },
        };
        cx.ask(q).await?;
        if let Some(d) = d {
            outlet.send(d).await;
        }
        Ok(())
    }
}

/// Takes the disc, or a message: says each message it's given.
struct Hears;

#[async_trait]
impl Step for Hears {
    fn info(&self) -> StepInfo {
        StepInfo::new("hears", "Notify", Group::Tell, ItemKind::Disc, ItemKind::Nothing).takes_messages()
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, cx: Cx) -> Result<()> {
        let mut said = vec![];
        while let Some(i) = inlet.recv().await {
            if let Some((step, line)) = i.as_message() {
                let name = i.context().get(Subject::Disc, "name").unwrap_or("");
                said.push(format!("{name} / {step} / {line}"));
            }
        }
        cx.status(said.len().to_string(), said.join("\n"));
        Ok(())
    }
}

fn engine() -> Engine {
    let mut reg = Registry::default();
    reg.add(Start).add(Breaks).add(Asks).add(Hears);
    let mut e = Engine::new(reg);
    e.time_scale = 0.0;
    e.work_root = std::env::temp_dir().join(format!("drt-paths-{}", std::process::id()));
    e
}

async fn run(e: &Engine, root: Node, answer: bool) -> Snapshot {
    let run = e.start(&Recipe { name: "p".into(), root }).unwrap();
    let mut rx = run.snapshot.clone();
    let wait = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let s = rx.borrow_and_update().clone();
            if let (true, Some((step, _))) = (answer, &s.question) {
                run.send(Command::Answer { step: step.clone(), answer: Answer::Pick(0) }).await;
            }
            if s.finished || rx.changed().await.is_err() {
                break;
            }
        }
    });
    assert!(wait.await.is_ok(), "the run never finished");
    run.finished().await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_step_that_fails_says_so_once_down_its_fail_path() {
    let e = engine();
    let root = chain([Node::step("s", "start"), Node::step("rip", "breaks").on(Side::Fails, Node::step("t", "hears")).on(Side::NeedsYou, Node::step("n", "hears"))]);
    check(&root, &e.registry).unwrap();
    let idle = e.idle(&Recipe { name: "p".into(), root: root.clone() });
    assert_eq!(idle.steps["t"].status, "If rip fails");
    let s = run(&e, root, false).await;
    assert_eq!(s.steps["rip"].state, StepState::Bad);
    assert_eq!(s.steps["t"].status, "1", "once");
    assert_eq!(s.steps["t"].detail, "Koudelka (Europe) (Disc 4) / Rip / Rip: couldn't read 774 sectors");
    assert_eq!(s.steps["n"].status, "0", "it didn't need you");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_goes_down_the_needs_you_path_and_the_run_goes_on() {
    let e = engine();
    let root = chain([Node::step("s", "start"), Node::step("c", "asks").on(Side::NeedsYou, Node::step("n", "hears")).on(Side::Fails, Node::step("t", "hears")), Node::step("end", "hears")]);
    let s = run(&e, root, true).await;
    assert_eq!(s.steps["n"].status, "1");
    assert_eq!(s.steps["n"].detail, "Koudelka (Europe) (Disc 4) / Choose / Choose needs you: Which game is this?");
    assert_eq!(s.steps["t"].status, "0");
    assert_eq!(s.steps["end"].status, "0", "the disc went on as usual, with no message");
    assert_eq!(s.steps["c"].state, StepState::Ok);
}

#[test]
fn only_a_step_that_takes_a_message_goes_on_a_path() {
    let e = engine();
    let reg = &e.registry;
    // Notify does; Choose, which takes a disc, doesn't.
    let ok = chain([Node::step("s", "start"), Node::step("rip", "breaks").on(Side::Fails, Node::step("t", "hears"))]);
    assert!(check(&ok, reg).is_ok());
    let wrong = chain([Node::step("s", "start"), Node::step("rip", "breaks").on(Side::Fails, Node::step("c", "asks"))]);
    assert!(matches!(check(&wrong, reg), Err(CheckError::Mismatch { id, .. }) if id == "c"));
    // Nothing gets a message on the main line.
    assert!(!ItemKind::Any.accepts(ItemKind::Message));
    // A path on a path is too much.
    let nested = Node::step("t", "hears").on(Side::Fails, Node::step("t2", "hears"));
    let deep = chain([Node::step("s", "start"), Node::step("rip", "breaks").on(Side::Fails, nested)]);
    assert!(matches!(check(&deep, reg), Err(CheckError::PathOnPath { id }) if id == "t"));
    // The + on a path offers only what takes a message.
    let offers = drt_engine::offers(&chain([Node::step("s", "start"), Node::step("rip", "breaks")]), &drt_engine::Spot::On("rip".into(), Side::NeedsYou), reg);
    assert_eq!(offers.iter().map(|o| o.info.kind).collect::<Vec<_>>(), ["hears"]);
    // Saved, a path is there only when there is one; removing the step
    // takes its paths, and removing what's on one leaves the step.
    let json = serde_json::to_string(&chain([Node::step("s", "start")])).unwrap();
    assert!(!json.contains("on_fail") && !json.contains("on_needs"));
    let back: Node = serde_json::from_str(&serde_json::to_string(&ok).unwrap()).unwrap();
    assert_eq!(back, ok);
    let mut r = ok.clone();
    assert!(r.remove("t"));
    assert_eq!(r, chain([Node::step("s", "start"), Node::step("rip", "breaks")]));
    let mut r = ok.clone();
    assert_eq!(r.remove_with_downstream("rip", reg), ["rip", "t"]);
}
