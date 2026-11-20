//! Rip and a hidden track, with a stand-in for cyanrip (`DRT_CYANRIP`)
//! that prints what cyanrip 0.9.3 prints and writes the WAV it would. No
//! drive is touched. It checks what Rip does with what it's given: the
//! split-off pregap is asked for only on track 0's read, silence isn't
//! kept, a read of the wrong length is never filed, and pre-emphasis is
//! recorded, not undone.
//!
//! Whether a real drive gives back a real disc's hidden track this way is
//! for a disc that has one; see docs/audio-accuracy.md.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::chain;
use drt_engine::{Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Snapshot, Step, StepInfo, Subject};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Track 1 starts 10 sectors late: 5880 sample frames of pregap.
const TOC: &str = "1 2 400 160 260";

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

#[derive(Clone, Default)]
struct Probe(Arc<Mutex<Vec<Item>>>);

#[async_trait]
impl Step for Probe {
    fn info(&self) -> StepInfo {
        StepInfo::new("test-probe", "Probe", Group::Tell, ItemKind::Track, ItemKind::Nothing)
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, _cx: Cx) -> Result<()> {
        while let Some(t) = inlet.recv().await {
            self.0.lock().unwrap().push(t);
        }
        Ok(())
    }
}

/// The stand-in: `-l N` writes `NN.wav` of N's length, printing cyanrip's
/// lines; track 0 only with `-p 1=track`. `HIDDEN` says what the pregap
/// holds: `music`, `silence`, or `short` (a read one sector short).
const FAKE: &str = r#"
import os, sys, struct, zlib
a = sys.argv[1:]
open(os.environ["FAKE_LOG"], "a").write(" ".join(a) + "\n")
t = int(a[a.index("-l") + 1])
if t == 0 and "1=track" not in a:
    print("Invalid rip index 0, list has 2 tracks!"); sys.exit(1)
if "-W" not in a:
    print("de-emphasis left on"); sys.exit(1)
hidden = os.environ["HIDDEN"]
frames = {0: 10 * 588, 1: 100 * 588, 2: 140 * 588}[t]
if t == 0 and hidden == "short":
    frames -= 588
if t == 0 and hidden == "silence":
    pcm = bytes(frames * 4)
else:
    pcm = bytes((i * 7 + t) % 251 for i in range(frames * 4))
hdr = b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 2, 44100, 176400, 4, 16) + b"data" + struct.pack("<I", len(pcm))
open("%02d.wav" % t, "wb").write(hdr + pcm)
samples = 10 * 588 if t == 0 else frames
print("Track %d ripped and encoded successfully!" % t)
print("  Preemphasis:   %s" % ("present (TOC)" if t == 1 else "none detected"))
print("  Properties:")
print("    Samples:     %d" % samples)
print("    EAC CRC32:   %08X" % (zlib.crc32(pcm) & 0xffffffff))
if t > 0:
    print("    Accurip v2:  12345678 (accurately ripped, confidence 9)")
else:
    print("    Accurip v2:  12345678 (not found, either a new pressing, or bad rip)")
"#;

async fn run(base: &Path, hidden: &str) -> (Snapshot, Vec<Item>, String) {
    let _ = std::fs::remove_dir_all(base);
    // The stand-in lives in a folder of its own, apart from the runs.
    let tools = base.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let script = tools.join("cyanrip.py");
    std::fs::write(&script, FAKE).unwrap();
    let log = base.join("calls.log");
    std::env::set_var("DRT_CYANRIP", format!("python3 -I {}", script.display()));
    std::env::set_var("FAKE_LOG", &log);
    std::env::set_var("HIDDEN", hidden);
    let probe = Probe::default();
    let mut reg = drt_steps::real();
    reg.add(Disc);
    reg.add(probe.clone());
    let mut e = Engine::new(reg);
    e.time_scale = 0.0;
    e.work_root = base.join("runs");
    let recipe = Recipe { name: "Hidden".into(), root: chain([Node::step("cd", "audio-cd"), Node::step("rip", "rip"), Node::step("probe", "test-probe")]) };
    drt_engine::check(&recipe.root, &e.registry).unwrap();
    let run = e.start(&recipe).unwrap();
    let mut rx = run.snapshot.clone();
    // A track that needs you would wait for ever; stop once Rip settles.
    loop {
        let s = rx.borrow_and_update().clone();
        let rip = &s.steps["rip"];
        let settled = rip.state == drt_engine::StepState::Bad && rip.items.iter().all(|i| i.state != ItemState::Busy && i.state != ItemState::Waiting);
        // And every track it settled has reached the probe.
        let sent = rip.items.iter().filter(|i| i.state == ItemState::Ok).count();
        if s.finished || (settled && probe.0.lock().unwrap().len() >= sent) {
            break;
        }
        if settled {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            continue;
        }
        if rx.changed().await.is_err() {
            break;
        }
    }
    let snap = run.snapshot.borrow().clone();
    run.send(drt_engine::Command::Stop).await;
    let _ = run.finished().await;
    let got = probe.0.lock().unwrap().clone();
    (snap, got, std::fs::read_to_string(log).unwrap_or_default())
}

fn base(n: &str) -> PathBuf {
    std::env::current_dir().unwrap().join("../../target/drt-test").join(n)
}

// One test, so the stand-in's environment is never shared between two at once.
#[tokio::test(flavor = "multi_thread")]
async fn a_hidden_track_is_track_0_when_it_holds_audio_and_only_then() {
    if std::process::Command::new("python3").arg("-V").output().is_err() {
        eprintln!("no python3 for the stand-in: skipped");
        return;
    }

    // Audio in the pregap: track 0, read twice, settled by agreeing reads.
    let b = base("hidden-music");
    let (s, got, calls) = run(&b, "music").await;
    let rows: Vec<(String, ItemState, String)> = s.steps["rip"].items.iter().map(|i| (i.id.clone(), i.state, i.note.clone())).collect();
    assert_eq!(rows[0], ("track-0".to_owned(), ItemState::Ok, "✓ two reads agree".to_owned()), "{rows:?}");
    assert!(rows[1..].iter().all(|(_, st, n)| *st == ItemState::Ok && n == "✓ 9"), "{rows:?}");
    assert_eq!(got.iter().map(|i| i.index.unwrap()).collect::<Vec<_>>(), [0, 1, 2]);
    let c = got[0].context();
    assert_eq!(c.get(Subject::Disc, "selected"), Some("3"));
    assert_eq!(got[0].label, "Hidden track");
    assert_eq!(got[1].context().get(Subject::Part(1), "preemphasis"), Some("1"), "the flag is kept");
    assert_eq!(got[2].context().get(Subject::Part(2), "preemphasis"), None);
    let lines: Vec<&str> = calls.lines().collect();
    assert_eq!(lines.iter().filter(|l| l.contains("-p 1=track")).count(), 2, "{calls}");
    assert!(lines.iter().all(|l| l.contains("-p 1=track") == l.contains("-l 0")), "only track 0 splits the pregap off: {calls}");
    std::fs::remove_dir_all(&b).unwrap();

    // Digital silence: not kept, and the disc is read as if it weren't there.
    let b = base("hidden-silence");
    let (s, got, calls) = run(&b, "silence").await;
    assert_eq!(got.iter().map(|i| i.index.unwrap()).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(got[0].context().get(Subject::Disc, "selected"), Some("2"));
    let zero = s.steps["rip"].items.iter().find(|i| i.id == "track-0").unwrap();
    assert_eq!((zero.state, zero.note.as_str()), (ItemState::Skipped, "digital silence, not kept"));
    assert_eq!(calls.lines().filter(|l| l.contains("-l 0")).count(), 1, "read once to look: {calls}");
    std::fs::remove_dir_all(&b).unwrap();

    // A read shorter than the TOC says: never filed; it waits for you.
    let b = base("hidden-short");
    let (s, got, _) = run(&b, "short").await;
    assert_eq!(got.iter().map(|i| i.index.unwrap()).collect::<Vec<_>>(), [1, 2]);
    let zero = s.steps["rip"].items.iter().find(|i| i.id == "track-0").unwrap();
    assert_eq!(zero.state, ItemState::Bad, "{}", zero.note);
    std::fs::remove_dir_all(&b).unwrap();
}
