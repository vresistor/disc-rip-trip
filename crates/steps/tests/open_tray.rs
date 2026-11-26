//! *Open tray* while Rip reads, with a stand-in for cyanrip (`TRIP_CYANRIP`)
//! and for the drive's "is there a disc?", so no drive is touched: the read
//! is stopped by killing its cyanrip, the track waits for you saying why,
//! nothing of the partial read goes on, and no read starts on an empty
//! drive.

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use trip_engine::recipe::chain;
use trip_engine::{Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Run, Snapshot, Step, StepInfo, StepState, Subject};
use trip_steps::real::{NO_DISC, TRAY_OPENED};

const TOC: &str = "1 2 10000 150 1150";

struct Disc;

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
        c.add(Subject::Disc, "offset", "103", "the drive");
        c.add(Subject::Disc, "parts", "2", "the disc");
        c.add(Subject::Disc, "toc", TOC, "the disc");
        for t in [1, 2] {
            c.add(Subject::Part(t), "length", "1", "the disc");
        }
        outlet.send(disc).await;
        Ok(())
    }
}

/// Notes the id of every track that reaches it.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<String>>>);

#[async_trait]
impl Step for Sink {
    fn info(&self) -> StepInfo {
        StepInfo::new("test-sink", "Sink", Group::Tell, ItemKind::Track, ItemKind::Nothing)
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, _cx: Cx) -> Result<()> {
        while let Some(i) = inlet.recv().await {
            self.0.lock().unwrap().push(i.id);
        }
        Ok(())
    }
}

/// The stand-in: `-l N` writes `NN.wav`, the same samples every read.
/// The track `SLOW` names writes half a file, says where it got to, notes
/// its process id and then reads for ever, as a bad last track on a
/// careful read seems to.
const FAKE: &str = r#"
import os, sys, struct, zlib, time
a = sys.argv[1:]
open(os.environ["FAKE_LOG"], "a").write("cyanrip " + " ".join(a) + "\n")
t = int(a[a.index("-l") + 1])
frames = 50 * 588
pcm = bytes((i * 7 + t) % 251 for i in range(frames * 4))
hdr = b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 2, 44100, 176400, 4, 16) + b"data" + struct.pack("<I", len(pcm))
if os.environ.get("SLOW") == str(t):
    open("%02d.wav" % t, "wb").write(hdr + pcm[: len(pcm) // 2])
    open(os.environ["FAKE_PID"], "w").write(str(os.getpid()))
    print("Ripping track %d, progress - 40.00%%" % t, flush=True)
    time.sleep(600)
open("%02d.wav" % t, "wb").write(hdr + pcm)
print("Track %d ripped and encoded successfully!" % t)
print("  Preemphasis:   none detected")
print("  Properties:")
print("    Samples:     %d" % frames)
print("    EAC CRC32:   %08X" % (zlib.crc32(pcm) & 0xffffffff))
print("    Accurip:     disc not found in database")
"#;

struct Setup {
    base: PathBuf,
    log: PathBuf,
    pid: PathBuf,
    disc_in: Arc<AtomicBool>,
    sunk: Sink,
}

fn start(name: &str, slow: &str, disc_in: bool) -> (Setup, Run) {
    let base = std::env::current_dir().unwrap().join("../../target/trip-test").join(name);
    let _ = std::fs::remove_dir_all(&base);
    let tools = base.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let script = tools.join("cyanrip.py");
    std::fs::write(&script, FAKE).unwrap();
    let (log, pid) = (base.join("calls.log"), base.join("cyanrip.pid"));
    std::env::set_var("TRIP_CYANRIP", format!("python3 -I {}", script.display()));
    std::env::set_var("FAKE_LOG", &log);
    std::env::set_var("FAKE_PID", &pid);
    std::env::set_var("SLOW", slow);
    let s = Setup { base, log, pid, disc_in: Arc::new(AtomicBool::new(disc_in)), sunk: Sink::default() };
    let mut reg = trip_steps::real();
    reg.add(Disc);
    reg.add(s.sunk.clone());
    let has = s.disc_in.clone();
    reg.add(trip_steps::real::Rip::with_flush(Arc::new(|_, _, _| Ok(()))).with_disc_check(Arc::new(move |_| has.load(Ordering::SeqCst))));
    let mut e = Engine::new(reg);
    e.time_scale = 0.0;
    e.work_root = s.base.join("runs");
    let recipe = Recipe { name: "Tray".into(), root: chain([Node::step("cd", "audio-cd"), Node::step("rip", "rip"), Node::step("sink", "test-sink")]) };
    trip_engine::check(&recipe.root, &e.registry).unwrap();
    let run = e.start(&recipe).unwrap();
    (s, run)
}

fn calls(log: &Path) -> usize {
    std::fs::read_to_string(log).unwrap_or_default().lines().count()
}

fn note(s: &Snapshot, item: &str) -> (ItemState, String) {
    s.steps["rip"].items.iter().find(|i| i.id == item).map(|i| (i.state, i.note.clone())).unwrap_or((ItemState::Waiting, "no row".into()))
}

async fn until(run: &Run, what: &str, cond: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let start = Instant::now();
    loop {
        let s = run.snapshot.borrow().clone();
        if cond(&s) {
            return s;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}: {:?}", s.steps["rip"].items);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn retry(run: &Run, item: &str) {
    run.send(Command::Retry { step: "rip".into(), item: item.into(), option: "two_reads".into() }).await;
}

async fn end(run: Run, s: Setup) {
    run.send(Command::Stop).await;
    let _ = run.finished().await;
    std::fs::remove_dir_all(&s.base).unwrap();
}

// One test, so the stand-in's environment is never shared between two at once.
#[tokio::test(flavor = "multi_thread")]
async fn open_tray_stops_the_read_and_the_track_waits_for_you() {
    if std::process::Command::new("python3").arg("-V").output().is_err() {
        eprintln!("no python3 for the stand-in: skipped");
        return;
    }

    // Track 2 reads for ever; the tray is asked to open in the middle.
    let (s, run) = start("open-tray", "2", true);
    until(&run, "track 2's read", |_| s.pid.exists()).await;
    let pid = std::fs::read_to_string(&s.pid).unwrap();
    let asked = Instant::now();
    assert!(run.release_drive(Duration::from_secs(10)).await, "the drive was let go of");
    assert!(asked.elapsed() < Duration::from_secs(5), "at once, not after the read: {:?}", asked.elapsed());
    // Its cyanrip has exited, and been waited for, before the tray may open.
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists(), "cyanrip {pid} still there");
    // The tray opens: the drive is empty.
    s.disc_in.store(false, Ordering::SeqCst);
    let made = calls(&s.log);
    let snap = until(&run, "track 2 to wait", |s| note(s, "track-2").0 == ItemState::Bad).await;
    assert_eq!(note(&snap, "track-2"), (ItemState::Bad, TRAY_OPENED.into()));
    assert_eq!(note(&snap, "track-1").0, ItemState::Ok);
    assert_eq!(snap.steps["rip"].detail, "Track 2 stopped: the tray was opened");
    // Nothing of the partial read goes on, or is kept to be counted later.
    assert_eq!(*s.sunk.0.lock().unwrap(), ["track-1"]);
    let rip = std::fs::read_dir(s.base.join("runs")).unwrap().flatten().next().unwrap().path().join("rip");
    assert!(!rip.join("02-1").exists(), "the partial read's folder is gone");
    // The run hasn't failed: Rip waits for you.
    assert!(!snap.finished && snap.steps["rip"].state == StepState::Bad, "{:?}", snap.steps["rip"]);
    assert!(snap.steps.values().all(|v| v.step == "rip" || v.state != StepState::Bad));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(calls(&s.log), made, "no read started after the tray opened");

    // Read again with the drive still empty: no read, and it says why.
    retry(&run, "track-2").await;
    until(&run, "no disc", |s| note(s, "track-2").1 == NO_DISC).await;
    assert_eq!(calls(&s.log), made, "no read on an empty drive");

    // The disc back in: Read again reads it, and it goes on.
    s.disc_in.store(true, Ordering::SeqCst);
    std::env::set_var("SLOW", "");
    retry(&run, "track-2").await;
    let snap = until(&run, "track 2 read", |s| note(s, "track-2").0 == ItemState::Ok).await;
    assert_eq!(note(&snap, "track-2").1, "✓ two reads agree");
    until(&run, "track 2 sent on", |_| s.sunk.0.lock().unwrap().len() == 2).await;
    end(run, s).await;

    // No disc from the start: cyanrip is never started.
    let (s, run) = start("open-tray-empty", "", false);
    let snap = until(&run, "both to wait", |s| s.steps["rip"].items.len() == 2 && s.steps["rip"].items.iter().all(|i| i.state == ItemState::Bad)).await;
    assert!(snap.steps["rip"].items.iter().all(|i| i.note == NO_DISC), "{:?}", snap.steps["rip"].items);
    assert_eq!(calls(&s.log), 0);
    assert!(s.sunk.0.lock().unwrap().is_empty());
    end(run, s).await;
}
