//! Two steps asking at once: the run has one place for a question, so the
//! second waits until the first is answered, and both get their answers.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::{branches, chain};
use drt_engine::snapshot::{Choice, Question, QuestionShape};
use drt_engine::{Answer, Command, Cx, Engine, Group, Inlet, ItemKind, Node, Outlet, Recipe, Registry, Step, StepInfo};

struct Start;

#[async_trait]
impl Step for Start {
    fn info(&self) -> StepInfo {
        StepInfo::new("start", "Start", Group::Source, ItemKind::Nothing, ItemKind::Disc)
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        inlet.recv().await;
        outlet.send(drt_engine::Item::disc("d")).await;
        Ok(())
    }
}

struct Asker;

#[async_trait]
impl Step for Asker {
    fn info(&self) -> StepInfo {
        StepInfo::new("asker", "Asker", Group::Tell, ItemKind::Disc, ItemKind::Nothing)
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, cx: Cx) -> Result<()> {
        inlet.recv().await;
        let q = Question {
            prompt: format!("{}?", cx.id()),
            shape: QuestionShape::PickOne {
                options: vec![Choice { label: "a".into(), detail: String::new() }, Choice { label: "b".into(), detail: String::new() }],
                preselected: 0,
                details: None,
                skip: false,
            },
        };
        let Answer::Pick(n) = cx.ask(q).await? else { anyhow::bail!("not a pick") };
        // Notify reads this: the run waited on you.
        cx.status(format!("picked {n}"), if cx.needed_you() { "needed you" } else { "" });
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_steps_asking_at_once_are_asked_in_turn() {
    let mut reg = Registry::default();
    reg.add(Start);
    reg.add(Asker);
    let mut e = Engine::new(reg);
    e.time_scale = 0.0;
    e.work_root = std::env::temp_dir().join(format!("drt-questions-{}", std::process::id()));
    let recipe = Recipe { name: "q".into(), root: chain([Node::step("s", "start"), branches([Node::step("a", "asker"), Node::step("b", "asker")])]) };
    let run = e.start(&recipe).unwrap();
    let mut rx = run.snapshot.clone();
    let mut asked = vec![];
    let done = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let s = rx.borrow_and_update().clone();
            if let Some((step, _)) = &s.question {
                if !asked.contains(step) {
                    // Long enough for the other step to ask as well.
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    let s = rx.borrow_and_update().clone();
                    let Some((step, _)) = &s.question else { continue };
                    asked.push(step.clone());
                    run.send(Command::Answer { step: step.clone(), answer: Answer::Pick(1) }).await;
                }
            }
            if s.finished || rx.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    assert!(done.is_ok(), "a question was hidden and never answered");
    let s = run.finished().await;
    asked.sort();
    assert_eq!(asked, ["a", "b"]);
    assert_eq!((s.steps["a"].status.as_str(), s.steps["b"].status.as_str()), ("picked 1", "picked 1"));
    assert_eq!(s.steps["a"].detail, "needed you", "a question counts as needing you");
}
