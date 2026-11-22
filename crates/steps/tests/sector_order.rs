//! Rip on a drive redumper doesn't know: the order the drive hands over
//! each sector's data, subchannel and C2 in is found by short test reads,
//! kept on the drive, and given to redumper then and on every later rip;
//! a drive redumper knows isn't tested. A stand-in redumper
//! (`DRT_REDUMPER`, set once for this test binary) answers as the Lite-On
//! eHAU324 does; the "drive" is a plain file, so no drive is touched.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::chain;
use drt_engine::setting::Setting;
use drt_engine::{Command, Cx, Engine, Group, Inlet, Item, ItemKind, Node, Outlet, ParamKind, ParamSpec, Recipe, Run, Snapshot, Step, StepInfo, Subject};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DRIVE: &str = "ACME XY123";

/// Stands in for Drive: a data disc in a "drive" that is a plain file,
/// in a recipe whose drive is [`DRIVE`].
struct Disc {
    device: PathBuf,
}

#[async_trait]
impl Step for Disc {
    fn info(&self) -> StepInfo {
        StepInfo::new("drive", "Drive", Group::Source, ItemKind::Nothing, ItemKind::DataDisc).param(ParamSpec::new("drive", "Drive", ParamKind::Text))
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

/// redumper's stand-in: writes each command it's given to `calls`; a
/// short read (`--lba-end`) answers as the eHAU324 reads a good disc in
/// the order asked for (redumper's own guess, DATA_C2_SUB, wrong), or as
/// a drive redumper knows when `known` is beside it. The whole read is
/// stopped part way, which is all these tests need of it.
const STAND_IN: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "$*" >> "$here/calls"
order=DATA_C2_SUB; short=no
for a in "$@"; do case $a in --drive-sector-order=*) order=${a#*=};; --lba-end=*) short=yes;; esac; done
echo "redumper (build: b744)"
if [ -e "$here/known" ]; then
  echo "  configuration: PLEXTOR (read offset: +30, C2 shift: 294, pre-gap start: -75, read method: D8, sector order: DATA_SUB_C2)"
else
  echo "warning: drive not found in the database"
  echo "  configuration: GENERIC (read offset: +0, C2 shift: 0, pre-gap start: +0, read method: BE, sector order: $order)"
fi
echo "*** DUMP (time check: 0s)"
if [ $short = yes ]; then
  case $order in DATA_C2_SUB) c2=6140; q=299;; DATA_C2) c2=0; q=299;; *) c2=0; q=0;; esac
  [ -e "$here/known" ] && { c2=0; q=0; }
  echo "media errors: "
  echo "  SCSI: 0 samples"
  echo "  C2: $c2 samples"
  echo "  Q: $q"
  exit 0
fi
kill -KILL $$
"#;

fn base() -> PathBuf {
    let b = std::env::current_dir().unwrap().join("../../target/drt-test/sector-order");
    let _ = std::fs::remove_dir_all(&b);
    std::fs::create_dir_all(&b).unwrap();
    b
}

fn engine(b: &Path) -> Engine {
    let mut reg = drt_steps::real();
    reg.add(Disc { device: b.join("drive") });
    let mut e = Engine::new(reg);
    e.work_root = b.join("runs");
    e
}

fn start(e: &Engine) -> Run {
    let recipe = Recipe { name: "Stand-in".into(), root: chain([Node::step("drive", "drive").with("drive", DRIVE), Node::step("rip", "image")]) };
    e.start(&recipe).unwrap()
}

/// Waits for the run to come to `cond`, for at most 20 s.
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

/// One rip, up to the whole read stopping: the commands redumper was
/// given, and what Rip logged.
async fn rip(b: &Path, e: &Engine) -> (Vec<String>, Vec<String>) {
    let _ = std::fs::remove_file(b.join("calls"));
    let run = start(e);
    let s = until(&run, "the read to stop", |s| s.steps["rip"].items.first().is_some_and(|i| i.note.starts_with("redumper stopped unexpectedly"))).await;
    run.send(Command::Stop).await;
    until(&run, "the run to end", |s| s.finished).await;
    let calls = std::fs::read_to_string(b.join("calls")).unwrap().lines().map(str::to_owned).collect();
    (calls, s.steps["rip"].record.log.iter().map(|l| l.line.clone()).collect())
}

fn short(c: &str) -> bool {
    c.contains("--lba-end=")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_drive_s_order_is_found_once_and_kept() {
    let b = base();
    std::fs::write(b.join("drive"), "").unwrap();
    let script = b.join("redumper");
    std::fs::write(&script, STAND_IN).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // SAFETY: set before any thread of this binary runs a tool, and this
    // binary's only test.
    unsafe { std::env::set_var("DRT_REDUMPER", &script) };

    // First rip: redumper's guess, then DATA_SUB_C2, which reads cleanly;
    // kept on the drive, and the whole read told it.
    let e = engine(&b);
    let (calls, log) = rip(&b, &e).await;
    assert_eq!(calls.len(), 3, "{calls:#?}");
    assert!(short(&calls[0]) && calls[0].contains("--auto-detect") && !calls[0].contains("--drive-sector-order"));
    assert!(short(&calls[1]) && calls[1].contains("--drive-sector-order=DATA_SUB_C2") && calls[1].contains("--retries=0"));
    assert!(!short(&calls[2]) && calls[2].starts_with("dump ") && calls[2].contains("--drive-sector-order=DATA_SUB_C2") && !calls[2].contains("--auto-detect"));
    assert!(log.iter().any(|l| l == "This drive hands over data, then subchannel, then C2 (found by testing)"), "{log:#?}");
    assert!(log.iter().any(|l| l.contains("as redumper guessed (DATA_C2_SUB)") && l.contains("C2 6140, Q 299")), "{log:#?}");
    let kept = e.settings.lock().unwrap().drive(DRIVE, "sector_order");
    assert!(matches!(&kept, Setting::Detected { value, source, .. } if value == "DATA_SUB_C2" && source == "found by testing"), "{kept:?}");

    // Next rip on the drive: no testing, the order kept given.
    let (calls, log) = rip(&b, &e).await;
    assert_eq!(calls.len(), 1, "{calls:#?}");
    assert!(calls[0].contains("--drive-sector-order=DATA_SUB_C2"));
    assert!(log.iter().any(|l| l == "This drive hands over data, then subchannel, then C2 (found by testing)"));

    // One chosen: given as it is, untested.
    e.settings.lock().unwrap().set_drive(DRIVE, "sector_order", Setting::chosen(serde_json::Value::from("DATA_C2_SUB")));
    let (calls, log) = rip(&b, &e).await;
    assert_eq!(calls.len(), 1, "{calls:#?}");
    assert!(calls[0].contains("--drive-sector-order=DATA_C2_SUB"));
    assert!(log.iter().any(|l| l == "This drive hands over data, then C2, then subchannel (your choice)"));

    // A drive redumper knows: one short read says so, and nothing's kept
    // or given.
    std::fs::write(b.join("known"), "").unwrap();
    let e = engine(&b);
    let (calls, log) = rip(&b, &e).await;
    assert_eq!(calls.len(), 2, "{calls:#?}");
    assert!(short(&calls[0]) && !short(&calls[1]) && !calls[1].contains("--drive-sector-order") && calls[1].contains("--auto-detect"));
    assert!(!log.iter().any(|l| l.starts_with("This drive hands over")));
    assert_eq!(e.settings.lock().unwrap().drive(DRIVE, "sector_order"), Setting::Default);
    std::fs::remove_dir_all(b).unwrap();
}
