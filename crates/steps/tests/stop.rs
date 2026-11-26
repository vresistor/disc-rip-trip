//! Stop while a game's Rip reads the disc (a data disc's is the same code): the run ends, Stopped, within
//! a bounded time, with no redumper left running and nothing on the window
//! still saying it's reading. A stand-in redumper (`TRIP_REDUMPER`, set once
//! for this test binary) is slow at the stage the test asks for, and shrugs
//! off SIGTERM, SIGINT and SIGHUP; the "drive" is a plain file, so no drive
//! is touched.

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::Duration;
use trip_engine::recipe::chain;
use trip_engine::{Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, ParamKind, ParamSpec, Recipe, Run, Snapshot, Step, StepInfo, StepState, Subject};

const DRIVE: &str = "ACME XY123";

/// Stands in for Drive: a CD in a "drive" that is a plain file.
struct Disc {
    device: PathBuf,
}

#[async_trait]
impl Step for Disc {
    fn info(&self) -> StepInfo {
        StepInfo::new("drive", "Drive", Group::Source, ItemKind::Nothing, ItemKind::GameDisc).param(ParamSpec::new("drive", "Drive", ParamKind::Text))
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        inlet.recv().await;
        cx.drive_says(StepState::Ok, "CD in", "");
        let mut d = Item::new("disc", ItemKind::GameDisc, "STANDIN", None, None);
        let c = d.context_mut();
        c.add(Subject::Disc, "device", self.device.display().to_string(), "the disc");
        c.add(Subject::Disc, "label", "STANDIN", "the disc");
        c.add(Subject::Disc, "media", "CD", "the disc");
        outlet.send(d).await;
        Ok(())
    }
}

/// redumper's stand-in, slow at the stage named in `mode` beside it
/// (`probe`, `dump`, `refine`, `split`): there it prints a progress line
/// a second, on and on, and ignores SIGTERM, SIGINT and SIGHUP. Every
/// other stage goes quickly: the drive is unknown to it, every order reads
/// the test sectors cleanly, and the read leaves errors to re-read. Each
/// start and end is written to `calls`, with its pid.
const STAND_IN: &str = r#"#!/bin/sh
trap '' TERM INT HUP
here=$(dirname "$0")
cmd=$1; shift
dir=.; name=x; short=no
for a in "$@"; do case $a in --image-path=*) dir=${a#*=};; --image-name=*) name=${a#*=};; --lba-end=*) short=yes;; esac; done
mode=$(cat "$here/mode")
echo "start $$ $cmd $short" >> "$here/calls"
slow() {
  i=0
  while :; do
    i=$((i + 1))
    echo "[ 73%] LBA: $((198000 + i))/269159, errors: { SCSIs: $((588 * i)), C2s: 35994, Q: 348 }$1"
    sleep 1
  done
}
case "$cmd" in
dump)
  echo "warning: drive not found in the database"
  echo "  configuration: GENERIC (read offset: +0, C2 shift: 0, pre-gap start: +0, read method: BE, sector order: DATA_C2_SUB)"
  echo "*** DUMP (time check: 0s)"
  if [ $short = yes ]; then
    [ "$mode" = probe ] && slow ""
    echo "media errors: "
    echo "  SCSI: 0 samples"
    echo "  C2: 0 samples"
    echo "  Q: 0"
    exit 0
  fi
  [ "$mode" = dump ] && slow ""
  : > "$dir/$name.scram"
  echo "media errors: "
  echo "  SCSI: 588 samples"
  echo "  C2: 0 samples"
  echo "  Q: 0"
  ;;
refine)
  echo "*** REFINE (time check: 0s)"
  [ "$mode" = refine ] && slow ", retry: 3"
  ;;
split)
  echo "*** SPLIT (time check: 0s)"
  [ "$mode" = split ] && slow ""
  ;;
esac
echo "end $$" >> "$here/calls"
"#;

fn base() -> PathBuf {
    let b = std::env::current_dir().unwrap().join("../../target/trip-test/stop");
    let _ = std::fs::remove_dir_all(&b);
    std::fs::create_dir_all(&b).unwrap();
    b
}

fn start(b: &Path, mode: &str) -> Run {
    std::fs::write(b.join("mode"), mode).unwrap();
    let _ = std::fs::remove_file(b.join("calls"));
    let mut reg = trip_steps::real();
    reg.add(Disc { device: b.join("drive") });
    let mut e = Engine::new(reg);
    e.work_root = b.join(format!("runs-{mode}"));
    let recipe = Recipe { name: "Stand-in".into(), root: chain([Node::step("drive", "drive").with("drive", DRIVE), Node::step("rip", "game-rip")]) };
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
    tokio::time::timeout(Duration::from_secs(20), wait).await.unwrap_or_else(|_| panic!("stuck waiting for {what}: {:#?}", run.snapshot.borrow()))
}

/// The stand-in's runs that started and never ended, and are still alive.
fn still_running(b: &Path) -> Vec<String> {
    let calls = std::fs::read_to_string(b.join("calls")).unwrap_or_default();
    let ended: Vec<&str> = calls.lines().filter_map(|l| l.strip_prefix("end ")).collect();
    calls
        .lines()
        .filter_map(|l| l.strip_prefix("start "))
        .filter_map(|l| l.split(' ').next())
        .filter(|pid| !ended.contains(pid) && Path::new(&format!("/proc/{pid}")).exists())
        .map(str::to_owned)
        .collect()
}

/// Rip slow at `mode`'s stage: once it's there, Stop, and the run ends
/// Stopped with nothing of it running or shown as busy.
async fn stop_at(b: &Path, mode: &str, there: impl Fn(&Snapshot) -> bool) {
    let run = start(b, mode);
    // Looked at every tenth of a second too: a test read shows no progress.
    let wait = async {
        while !there(&run.snapshot.borrow()) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait).await.unwrap_or_else(|_| panic!("{mode}: never got there: {:#?}", run.snapshot.borrow().steps["rip"]));
    // Well into the slow stage.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    run.send(Command::Stop).await;
    let s = until(&run, &format!("the run stopped at {mode} to end"), |s| s.finished).await;
    assert!(s.stopped);
    assert!(still_running(b).is_empty(), "{mode}: redumper left running: {:?}", still_running(b));
    let rip = &s.steps["rip"];
    assert_eq!((rip.state, rip.status.as_str()), (StepState::Idle, "Stopped"), "{mode}: {rip:#?}");
    assert_eq!(rip.progress, None, "{mode}");
    assert_eq!(rip.now, None, "{mode}");
    assert!(rip.items.iter().all(|i| i.state == ItemState::Skipped && i.note == trip_engine::snapshot::STOPPED), "{mode}: an item not stopped: {:#?}", rip.items);
    let busy: Vec<_> = s.steps.iter().filter(|(_, v)| v.state == StepState::Busy).map(|(k, v)| format!("{k}: {} {}", v.status, v.detail)).collect();
    assert!(busy.is_empty(), "{mode}: still busy: {busy:?}");
}

fn reading(s: &Snapshot, how: &str) -> bool {
    s.steps["rip"].items.first().is_some_and(|i| i.state == ItemState::Busy) && s.steps["rip"].record.log.iter().any(|l| l.line.contains(how))
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_ends_rip_at_every_stage_of_its_read() {
    let b = base();
    std::fs::write(b.join("drive"), "").unwrap();
    let script = b.join("redumper");
    std::fs::write(&script, STAND_IN).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // SAFETY: set before any thread of this binary runs a tool, and this
    // binary's only test.
    unsafe { std::env::set_var("TRIP_REDUMPER", &script) };

    let slow = |s: &Snapshot| s.steps["rip"].progress.is_some();
    stop_at(&b, "probe", |_| std::fs::read_to_string(b.join("calls")).unwrap_or_default().contains(" dump yes")).await;
    stop_at(&b, "dump", |s| reading(s, "redumper dump") && !s.steps["rip"].record.log.iter().any(|l| l.line.contains("--lba-end")) && slow(s)).await;
    stop_at(&b, "refine", |s| reading(s, "redumper refine") && slow(s)).await;
    stop_at(&b, "split", |s| reading(s, "redumper split") && slow(s)).await;
    std::fs::remove_dir_all(b).unwrap();
}
