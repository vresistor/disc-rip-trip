//! Rip when redumper is killed from outside part way through a stage: it
//! holds with what happened, for you to try again or skip, and never sits
//! there. A stand-in redumper (`DRT_REDUMPER`, set once for this test
//! binary) kills itself at the stage the test asks for; the "drive" is a
//! plain file, so no drive is touched.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::chain;
use drt_engine::{Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Run, Snapshot, Step, StepInfo, StepState, Subject};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Gives a data disc in a "drive" that is a plain file.
struct Disc {
    device: PathBuf,
}

#[async_trait]
impl Step for Disc {
    fn info(&self) -> StepInfo {
        StepInfo::new("given-disc", "Disc", Group::Source, ItemKind::Nothing, ItemKind::DataDisc)
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        inlet.recv().await;
        let mut d = Item::new("disc", ItemKind::DataDisc, "STANDIN", None, None);
        let c = d.context_mut();
        c.add(Subject::Disc, "device", self.device.display().to_string(), "the disc");
        c.add(Subject::Disc, "label", "STANDIN", "the disc");
        c.add(Subject::Disc, "media", "CD", "the disc");
        outlet.send(d).await;
        Ok(())
    }
}

/// redumper's stand-in: prints what the stage would, then kills itself
/// where `mode` (a file beside it) says: in the read (`dump`), or in the
/// re-read and the writing of the tracks (`refine`).
const STAND_IN: &str = r#"#!/bin/sh
cmd=$1; shift
dir=.; name=x
for a in "$@"; do case $a in --image-path=*) dir=${a#*=};; --image-name=*) name=${a#*=};; esac; done
mode=$(cat "$(dirname "$0")/mode")
case "$cmd" in
dump)
  echo "*** DUMP (time check: 0s)"
  echo "[ 10%] LBA:   1000/10000, errors: { SCSIs: 0, C2s: 0, Q: 0 }"
  [ "$mode" = dump ] && kill -KILL $$
  : > "$dir/$name.scram"
  echo "media errors: "
  echo "  SCSI: 588 samples"
  echo "  C2: 0 samples"
  echo "  Q: 0"
  ;;
refine)
  echo "*** REFINE (time check: 0s)"
  echo "[ 50%] LBA:   5000/10000, errors: { SCSIs: 588, C2s: 0, Q: 0 }, retry: 3"
  kill -INT $$
  sleep 5
  ;;
split)
  echo "*** SPLIT (time check: 0s)"
  kill -KILL $$
  ;;
esac
"#;

fn base() -> PathBuf {
    let b = std::env::current_dir().unwrap().join("../../target/drt-test/redumper-dies");
    let _ = std::fs::remove_dir_all(&b);
    std::fs::create_dir_all(&b).unwrap();
    b
}

fn start(b: &Path, mode: &str) -> Run {
    std::fs::write(b.join("mode"), mode).unwrap();
    let mut reg = drt_steps::real();
    reg.add(Disc { device: b.join("drive") });
    let mut e = Engine::new(reg);
    e.work_root = b.join(format!("runs-{mode}"));
    let recipe = Recipe { name: "Stand-in".into(), root: chain([Node::step("disc", "given-disc"), Node::step("rip", "image")]) };
    e.start(&recipe).unwrap()
}

/// Waits for the run to come to `cond`, for at most 20 s: a step that
/// hangs fails here.
async fn until(run: &Run, what: &str, cond: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let mut rx = run.snapshot.clone();
    let wait = async {
        loop {
            let s = rx.borrow_and_update().clone();
            if cond(&s) {
                return s;
            }
            rx.changed().await.unwrap();
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait).await.unwrap_or_else(|_| panic!("stuck waiting for {what}: {:#?}", run.snapshot.borrow().steps["rip"]))
}

/// Held on `note`, the card saying it too.
fn held(s: &Snapshot, note: &str) -> bool {
    let v = &s.steps["rip"];
    v.state == StepState::Bad && v.detail == note && v.items.first().is_some_and(|i| i.state == ItemState::Bad && i.note == note)
}

async fn retry(run: &Run, option: &str) {
    run.send(Command::Retry { step: "rip".into(), item: "dump".into(), option: option.into() }).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_redumper_killed_mid_stage_holds_rip_with_what_happened() {
    let b = base();
    std::fs::write(b.join("drive"), "").unwrap();
    let script = b.join("redumper");
    std::fs::write(&script, STAND_IN).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // SAFETY: set before any thread of this binary runs a tool, and this
    // binary's only test.
    unsafe { std::env::set_var("DRT_REDUMPER", &script) };

    // Killed in the read: held, and skipping it ends the run.
    let run = start(&b, "dump");
    let s = until(&run, "Rip to hold", |s| held(s, "redumper stopped unexpectedly: killed by signal 9 (SIGKILL)")).await;
    assert!(!s.finished);
    retry(&run, "leave_out").await;
    let s = until(&run, "the run to end", |s| s.finished).await;
    assert_eq!(s.steps["rip"].items[0].state, ItemState::Skipped);

    // Read through, then the re-read stopped by SIGINT: held. Re-read again,
    // the same. Made anyway, and the writing of the tracks killed: held.
    let run = start(&b, "refine");
    until(&run, "Rip to hold after the re-read", |s| held(s, "redumper stopped unexpectedly: killed by signal 2 (SIGINT)")).await;
    retry(&run, "refine").await;
    until(&run, "the re-read again", |s| s.steps["rip"].items[0].state == ItemState::Busy).await;
    until(&run, "Rip to hold again", |s| held(s, "redumper stopped unexpectedly: killed by signal 2 (SIGINT)")).await;
    retry(&run, "keep").await;
    until(&run, "Make the image anyway", |s| s.steps["rip"].items[0].state == ItemState::Busy).await;
    until(&run, "Rip to hold after the tracks", |s| held(s, "redumper stopped unexpectedly: killed by signal 9 (SIGKILL)")).await;
    retry(&run, "leave_out").await;
    until(&run, "the run to end", |s| s.finished).await;
    std::fs::remove_dir_all(b).unwrap();
}
