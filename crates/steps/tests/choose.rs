//! Choose: picking, typing the details, or skipping.

use drt_engine::snapshot::QuestionShape;
use drt_engine::{Answer, Command, Engine, Setting, Snapshot};
use drt_steps::{recipes, simulated};
use serde_json::Value;
use std::collections::BTreeMap;

async fn run_answering(answer: impl Fn(&QuestionShape) -> Answer) -> Snapshot {
    let mut e = Engine::new(simulated(None));
    e.time_scale = 0.0;
    e.work_root = std::env::temp_dir().join(format!("drt-choose-{}", std::process::id()));
    e.settings.lock().unwrap().set_drive(recipes::BLU_RAY, "offset", Setting::chosen(Value::from(103)));
    let run = e.start(&recipes::cd()).unwrap();
    let mut rx = run.snapshot.clone();
    loop {
        let s = rx.borrow_and_update().clone();
        if let Some((step, q)) = &s.question {
            run.send(Command::Answer { step: step.clone(), answer: answer(&q.shape) }).await;
        }
        for (id, v) in &s.steps {
            if v.detail == "All here" {
                run.send(Command::Approve { step: id.clone() }).await;
            }
        }
        if s.finished || rx.changed().await.is_err() {
            break;
        }
    }
    run.finished().await
}

#[tokio::test]
async fn the_question_offers_typing_the_details_and_skipping() {
    let s = run_answering(|shape| {
        let QuestionShape::PickOne { details, skip, .. } = shape else { panic!() };
        assert!(*skip);
        let d = details.as_ref().expect("details offered");
        assert_eq!(d["title.1"], "Jimmy the Exploder", "starts from the best guess");
        let mut d: BTreeMap<String, String> = d.clone();
        d.insert("album".into(), "My Own Name".into());
        d.insert("album_artist".into(), "Someone".into());
        Answer::Details(d)
    })
    .await;
    assert_eq!(s.steps["choose"].status, "Someone — My Own Name");
}

#[tokio::test]
async fn skipping_goes_on_without_a_release() {
    let s = run_answering(|_| Answer::Skip).await;
    assert_eq!(s.steps["choose"].status, "Skipped");
    assert!(s.finished && !s.stopped);
}
