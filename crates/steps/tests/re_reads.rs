//! Rip's re-reads, with a stand-in for cyanrip (`DRT_CYANRIP`) and for
//! clearing the drive's cache, so no drive is touched: every read after a
//! track's first is preceded by clearing the cache, a re-read without it
//! can't settle a track by agreeing, and *Read again slowly* on a drive
//! that won't change speed reads at the drive's own.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::chain;
use drt_engine::{Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Run, Snapshot, Step, StepInfo, Subject};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Two tracks of a disc that runs on long enough to clear the cache from
/// well away: track 1 from LBA 0 to 1000, track 2 from 1000 to 9850.
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
        if std::env::var("FIXED").as_deref() == Ok("1") {
            c.add(Subject::Disc, "fixed_speed", "1", "the drive");
        }
        for t in [1, 2] {
            c.add(Subject::Part(t), "length", "1", "the disc");
        }
        outlet.send(disc).await;
        Ok(())
    }
}

struct Sink;

#[async_trait]
impl Step for Sink {
    fn info(&self) -> StepInfo {
        StepInfo::new("test-sink", "Sink", Group::Tell, ItemKind::Track, ItemKind::Nothing)
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, _cx: Cx) -> Result<()> {
        while inlet.recv().await.is_some() {}
        Ok(())
    }
}

/// The stand-in: `-l N` writes `NN.wav`. AccurateRip never matches.
/// `READS=same` gives the same samples every read, `differ` different ones
/// each time. `REFUSE_SPEED=1` answers `-S` as cyanrip 0.9.3 does on a
/// drive that won't change speed.
const FAKE: &str = r#"
import os, sys, struct, zlib
a = sys.argv[1:]
log = os.environ["FAKE_LOG"]
open(log, "a").write("cyanrip " + " ".join(a) + "\n")
if "-S" in a and os.environ.get("REFUSE_SPEED") == "1":
    print("Opening drive...")
    print("Device does not support changing speeds!")
    sys.exit(1)
t = int(a[a.index("-l") + 1])
n = sum(1 for l in open(log) if l.startswith("cyanrip ") and ("-l %d " % t) in l)
seed = n if os.environ["READS"] == "differ" else 0
frames = 50 * 588
pcm = bytearray((i * 7 + t) % 251 for i in range(frames * 4))
pcm[1000] = seed % 256
pcm = bytes(pcm)
hdr = b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 2, 44100, 176400, 4, 16) + b"data" + struct.pack("<I", len(pcm))
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
}

fn setup(name: &str, reads: &str, refuse_speed: bool, fixed: bool) -> Setup {
    let base = std::env::current_dir().unwrap().join("../../target/drt-test").join(name);
    let _ = std::fs::remove_dir_all(&base);
    let tools = base.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let script = tools.join("cyanrip.py");
    std::fs::write(&script, FAKE).unwrap();
    let log = base.join("calls.log");
    std::env::set_var("DRT_CYANRIP", format!("python3 -I {}", script.display()));
    std::env::set_var("FAKE_LOG", &log);
    std::env::set_var("READS", reads);
    std::env::set_var("REFUSE_SPEED", if refuse_speed { "1" } else { "0" });
    std::env::set_var("FIXED", if fixed { "1" } else { "0" });
    Setup { base, log }
}

/// Starts Rip with a cache-clearing stand-in that notes each call where
/// cyanrip's are, and succeeds while `ok` holds.
fn start(s: &Setup, ok: Arc<AtomicBool>) -> Run {
    let mut reg = drt_steps::real();
    reg.add(Disc);
    reg.add(Sink);
    let log = s.log.clone();
    reg.add(drt_steps::real::Rip::with_flush(Arc::new(move |dev, lba, count| {
        use std::io::Write;
        let good = ok.load(Ordering::SeqCst);
        writeln!(std::fs::OpenOptions::new().append(true).create(true).open(&log)?, "flush {dev} {lba} {count}{}", if good { "" } else { " failed" })?;
        if good {
            Ok(())
        } else {
            Err(std::io::Error::other("the drive refused the command"))
        }
    })));
    let mut e = Engine::new(reg);
    e.time_scale = 0.0;
    e.work_root = s.base.join("runs");
    let recipe = Recipe { name: "Cache".into(), root: chain([Node::step("cd", "audio-cd"), Node::step("rip", "rip"), Node::step("sink", "test-sink")]) };
    drt_engine::check(&recipe.root, &e.registry).unwrap();
    e.start(&recipe).unwrap()
}

/// Waits until Rip has nothing left to read, and has made more than
/// `calls` calls to cyanrip (so a retry asked for has run).
async fn settled(run: &Run, log: &Path, calls: usize) -> Snapshot {
    for _ in 0..1500 {
        let s = run.snapshot.borrow().clone();
        let rip = &s.steps["rip"];
        let idle = rip.items.len() == 2 && rip.items.iter().all(|i| i.state != ItemState::Busy && i.state != ItemState::Waiting);
        let made = std::fs::read_to_string(log).unwrap_or_default().lines().filter(|l| l.starts_with("cyanrip")).count();
        if (s.finished || (idle && rip.state == drt_engine::StepState::Bad)) && made > calls {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Rip never settled: {:?}", run.snapshot.borrow().steps["rip"].items);
}

/// The calls in order, shortened to what matters: `read 1 P0`, `flush`.
fn order(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            if w[0] == "flush" {
                return if l.ends_with("failed") { "flush failed".into() } else { "flush".into() };
            }
            let arg = |k: &str| w.iter().position(|x| *x == k).map(|i| w[i + 1]).unwrap_or("");
            let speed = if w.contains(&"-S") { format!(" S{}", arg("-S")) } else { String::new() };
            format!("read {} P{}{speed}", arg("-l"), arg("-P"))
        })
        .collect()
}

fn calls(log: &Path) -> usize {
    order(log).iter().filter(|c| c.starts_with("read")).count()
}

fn notes(s: &Snapshot) -> Vec<(ItemState, String)> {
    s.steps["rip"].items.iter().map(|i| (i.state, i.note.clone())).collect()
}

fn logged(s: &Snapshot) -> String {
    s.steps["rip"].record.log.iter().map(|l| l.line.clone()).collect::<Vec<_>>().join("\n")
}

async fn retry(run: &Run, item: &str, option: &str) {
    run.send(Command::Retry { step: "rip".into(), item: item.into(), option: option.into() }).await;
}

async fn end(run: Run, s: Setup) {
    run.send(Command::Stop).await;
    let _ = run.finished().await;
    std::fs::remove_dir_all(&s.base).unwrap();
}

// One test, so the stand-in's environment is never shared between two at once.
#[tokio::test(flavor = "multi_thread")]
async fn re_reads_clear_the_drives_cache_first_and_slow_ones_fall_back_to_its_speed() {
    if std::process::Command::new("python3").arg("-V").output().is_err() {
        eprintln!("no python3 for the stand-in: skipped");
        return;
    }

    // The cache is cleared before each careful re-read, not before a first
    // read; then two reads that agree settle the track.
    let s = setup("cache-cleared", "same", false, false);
    let run = start(&s, Arc::new(AtomicBool::new(true)));
    let snap = settled(&run, &s.log, 0).await;
    assert_eq!(order(&s.log), ["read 1 P0", "flush", "read 1 P3", "read 2 P0", "flush", "read 2 P3"]);
    assert!(notes(&snap).iter().all(|(st, n)| *st == ItemState::Ok && n == "✓ two reads agree"), "{:?}", notes(&snap));
    // Away from both ends of the track: track 1 from after it; track 2,
    // with no room before or after, from its own middle.
    let flushes: Vec<String> = std::fs::read_to_string(&s.log).unwrap().lines().filter(|l| l.starts_with("flush")).map(str::to_owned).collect();
    assert_eq!(flushes, ["flush /dev/sr-none 3400 2400", "flush /dev/sr-none 4225 2400"]);
    let log = logged(&snap);
    assert!(log.contains("track 1: cleared the drive's cache: read 2,400 sectors from track 2"), "{log}");
    assert!(log.contains("track 2: cleared the drive's cache: read 2,400 sectors from track 2"), "{log}");
    end(run, s).await;

    // Clearing fails: the re-read still happens, but agreeing reads don't
    // settle the track; it waits for you, saying why.
    let s = setup("cache-failed", "same", false, false);
    let ok = Arc::new(AtomicBool::new(false));
    let run = start(&s, ok.clone());
    let snap = settled(&run, &s.log, 0).await;
    assert_eq!(order(&s.log), ["read 1 P0", "flush failed", "read 1 P3", "read 2 P0", "flush failed", "read 2 P3"]);
    assert!(notes(&snap).iter().all(|(st, n)| *st == ItemState::Bad && n == drt_steps::real::NOT_CLEARED), "{:?}", notes(&snap));
    assert!(logged(&snap).contains("track 1: couldn't clear the drive's cache (the drive refused the command)"), "{}", logged(&snap));
    assert_eq!(snap.steps["rip"].detail, "2 tracks to decide");
    // Read again once clearing works: that read agrees, and settles it.
    ok.store(true, Ordering::SeqCst);
    retry(&run, "track-1", "two_reads").await;
    let snap = settled(&run, &s.log, 4).await;
    assert_eq!(order(&s.log)[6..], ["flush", "read 1 P3"]);
    assert_eq!(notes(&snap)[0], (ItemState::Ok, "✓ two reads agree".into()));
    // Best of 3 reads votes only with reads that surely came off the disc.
    ok.store(false, Ordering::SeqCst);
    retry(&run, "track-2", "combine").await;
    let snap = settled(&run, &s.log, 5).await;
    assert_eq!(notes(&snap)[1], (ItemState::Bad, drt_steps::real::NOT_CLEARED.into()), "{:?}", order(&s.log));
    end(run, s).await;

    // Read again slowly, on a drive that refuses: read at its own speed,
    // said so, and not asked again for the second read.
    let s = setup("slowly-refused", "differ", true, false);
    let run = start(&s, Arc::new(AtomicBool::new(true)));
    settled(&run, &s.log, 0).await;
    assert_eq!(calls(&s.log), 4);
    retry(&run, "track-1", "slowly").await;
    let snap = settled(&run, &s.log, 4).await;
    assert_eq!(order(&s.log)[6..], ["flush", "read 1 P3 S4", "read 1 P3", "flush", "read 1 P3"]);
    let log = logged(&snap);
    assert_eq!(log.matches("track 1: this drive won't read slower; read at its usual speed").count(), 1, "{log}");
    assert!(notes(&snap)[0].1.contains("samples differ"), "a refused speed isn't a failed read: {:?}", notes(&snap));
    // The next slow read knows already.
    retry(&run, "track-2", "slowly").await;
    let snap = settled(&run, &s.log, 7).await;
    assert!(!order(&s.log)[11..].iter().any(|c| c.contains("S4")), "{:?}", order(&s.log));
    assert!(logged(&snap).contains("track 2: this drive won't read slower"), "{}", logged(&snap));
    end(run, s).await;

    // A drive cyanrip already said reads at one speed: never asked.
    let s = setup("slowly-fixed", "differ", true, true);
    let run = start(&s, Arc::new(AtomicBool::new(true)));
    settled(&run, &s.log, 0).await;
    retry(&run, "track-1", "slowly").await;
    let snap = settled(&run, &s.log, 4).await;
    assert!(!order(&s.log).iter().any(|c| c.contains("S4")), "{:?}", order(&s.log));
    assert!(logged(&snap).contains("track 1: this drive won't read slower; read at its usual speed"), "{}", logged(&snap));
    end(run, s).await;
}
