//! The real Check ripping a held dump again and combining the rips, with a
//! stand-in for the rip (no drive): each rip is a made-up one-track
//! PlayStation image whose sectors carry real addresses and EDCs, wrong
//! in different places. A match with Redump after three rips goes on
//! verified; rips that never match are kept as the best combination, not
//! verified; and a disc Redump doesn't have is never called verified.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::chain;
use drt_engine::{Answer, Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Run, Snapshot, Step, StepInfo, Subject};
use drt_steps::real::combine::{edc, msf, SECTOR};
use drt_steps::real::data::{self, DUMP};
use drt_steps::real::game::RipFn;
use drt_steps::real::mismatch::Mismatch;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SECTORS: i64 = 12;

/// A form 2 sector at `lba`, with its EDC, as the disc holds it.
fn sector(lba: i64) -> Vec<u8> {
    let mut s = vec![0u8; SECTOR];
    s[1..11].fill(0xFF);
    s[12..15].copy_from_slice(&msf(lba));
    s[15] = 2;
    s[16..24].copy_from_slice(&[0, 0, 0x20, 0, 0, 0, 0x20, 0]);
    for (i, b) in s[24..2348].iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(5) ^ lba as u8;
    }
    let e = edc(&s[16..2348]);
    s[2348..].copy_from_slice(&e.to_le_bytes());
    s
}

/// The disc as it is, then a rip of it with `slips` (a sector read as the
/// one before it) and `flips` (a byte wrong).
fn rip(slips: &[i64], flips: &[i64]) -> Vec<u8> {
    (0..SECTORS)
        .flat_map(|l| {
            let mut s = if slips.contains(&l) { sector(l - 1) } else { sector(l) };
            if flips.contains(&l) {
                s[600] ^= 0x22;
            }
            s
        })
        .collect()
}

fn write_rip(dir: &Path, bin: &[u8]) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("G.bin"), bin).unwrap();
    std::fs::write(dir.join("G.log"), drt_steps::game_sim::LANTERN_KEEP_LOG).unwrap();
    let cue = dir.join("G.cue");
    std::fs::write(&cue, "FILE \"G.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n").unwrap();
    cue
}

fn md5(bytes: &[u8]) -> String {
    use md5::Digest;
    drt_steps::real::hash::hex(&md5::Md5::digest(bytes))
}

fn sha1(bytes: &[u8]) -> String {
    use sha1::Digest;
    drt_steps::real::hash::hex(&sha1::Sha1::digest(bytes))
}

/// Gives the first rip, as Game disc and Rip would.
struct Dumped {
    main: PathBuf,
}

#[async_trait]
impl Step for Dumped {
    fn info(&self) -> StepInfo {
        StepInfo::new("dumped", "Dumped", Group::Source, ItemKind::Nothing, ItemKind::Dump).gives(&["serial", "label", "system", "read"])
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        inlet.recv().await;
        let mut dump = Item::new(DUMP, ItemKind::Dump, "SLES_047.11", None, Some(self.main.clone()));
        for (k, v) in [("label", "SLES_047.11"), ("media", "CD"), ("system", "PlayStation"), ("catalogue", "psx"), ("serial", "SLES-04711"), (data::RIPPED_WITH, "redumper")] {
            dump.context_mut().add(Subject::Disc, k, v, "the disc");
        }
        outlet.send(dump).await;
        Ok(())
    }
}

/// A rip stand-in: rip N (from its folder's name) is `rips[N - 2]`.
fn stand_in(rips: Vec<Vec<u8>>) -> RipFn {
    let rips = Arc::new(rips);
    Arc::new(move |_cx, _dump, dir: PathBuf| {
        let rips = rips.clone();
        Box::pin(async move {
            let n: usize = dir.file_name().unwrap().to_string_lossy().trim_start_matches("rip-").parse()?;
            Ok(write_rip(&dir, &rips[(n - 2).min(rips.len() - 1)]))
        })
    })
}

/// A run of the real Check, Redump and Choose on the first rip, Redump's
/// list holding the disc when `listed`.
fn start(base: &Path, first: &[u8], later: Vec<Vec<u8>>, listed: bool) -> (Run, Engine) {
    let _ = std::fs::remove_dir_all(base);
    let main = write_rip(&base.join("dump"), first);
    let good = rip(&[], &[]);
    let dat = if listed {
        format!(
            r#"<?xml version="1.0"?><datafile><header><version>test</version></header>
<game name="Lantern Keep (Europe) (Disc 1)"><serial>SLES-04711</serial>
<rom name="Lantern Keep (Europe) (Disc 1).cue" size="1" crc="00" md5="00" sha1="ffff"/>
<rom name="Lantern Keep (Europe) (Disc 1).bin" size="{}" crc="{:08x}" md5="{}" sha1="{}"/></game>
</datafile>"#,
            good.len(),
            crc32fast::hash(&good),
            md5(&good),
            sha1(&good)
        )
    } else {
        r#"<?xml version="1.0"?><datafile><header><version>test</version></header>
<game name="Something Else (USA)"><serial>SLUS-00001</serial><rom name="x.bin" size="1" crc="00" md5="00" sha1="0000"/></game></datafile>"#
            .to_owned()
    };
    std::fs::create_dir_all(base.join("cache/redump")).unwrap();
    std::fs::write(base.join("cache/redump/psx.dat"), dat).unwrap();
    let mut reg = drt_steps::real();
    reg.add(Dumped { main });
    reg.add(data::Check { rip: stand_in(later) });
    let mut e = Engine::new(reg);
    e.work_root = base.join("runs");
    let recipe =
        Recipe { name: "Fixture combine".into(), root: chain([Node::step("src", "dumped"), Node::step("check", "check"), Node::step("redump", "redump"), Node::step("choose", "game-choose")]) };
    drt_engine::check(&recipe.root, &e.registry).unwrap();
    (e.start(&recipe).unwrap(), e)
}

/// Waits till Check holds the dump with `rips` combined (0: none yet).
async fn held(run: &Run, rips: usize) -> Snapshot {
    let mut rx = run.snapshot.clone();
    loop {
        let s = rx.borrow_and_update().clone();
        let c = &s.steps["check"];
        let combined = serde_json::from_value::<Mismatch>(c.record.report.clone().unwrap_or_default()).ok().and_then(|m| m.combined).map_or(0, |c| c.rips.len());
        if c.items.first().is_some_and(|i| i.state == ItemState::Bad) && combined == rips {
            return s;
        }
        assert!(!s.finished, "finished without holding: {:?}", c.items);
        rx.changed().await.unwrap();
    }
}

async fn retry(run: &Run, option: &str) {
    run.send(Command::Retry { step: "check".into(), item: DUMP.into(), option: option.into() }).await;
}

async fn finish(run: Run) -> Snapshot {
    let mut rx = run.snapshot.clone();
    loop {
        let s = rx.borrow_and_update().clone();
        if s.finished {
            break;
        }
        if let Some((step, _)) = &s.question {
            run.send(Command::Answer { step: step.clone(), answer: Answer::Skip }).await;
        }
        if rx.changed().await.is_err() {
            break;
        }
    }
    run.finished().await
}

fn labels(s: &Snapshot) -> Vec<String> {
    s.steps["check"].items[0].options.iter().map(|o| o.1.clone()).collect()
}

fn base(name: &str) -> PathBuf {
    std::env::current_dir().unwrap().join("../../target/drt-test").join(name)
}

#[tokio::test(flavor = "multi_thread")]
async fn three_rips_combined_match_redump_and_go_on_verified() {
    let base = base("combine-match");
    // Rip 1 slips at 3 and 7; rip 2 is wrong at 3 another way; rip 3 slips at 9.
    let first = rip(&[3, 7], &[]);
    let (run, _e) = start(&base, &first, vec![rip(&[], &[3]), rip(&[9], &[])], true);
    let s = held(&run, 0).await;
    assert_eq!(labels(&s), ["Rip again and combine", "Keep it anyway", "Skip this disc"]);
    retry(&run, "combine").await;
    let s = held(&run, 2).await;
    let m: Mismatch = serde_json::from_value(s.steps["check"].record.report.clone().unwrap()).unwrap();
    let c = m.combined.clone().unwrap();
    assert_eq!((c.differing, c.no_good, c.still_bad, c.left.clone(), c.verified), (2, 1, 1, vec![3], false), "sector 3 has no good copy yet");
    assert_eq!(c.rips.iter().map(|r| r.bad).collect::<Vec<_>>(), [2, 1]);
    assert_eq!(labels(&s), ["Rip again and combine again", "Keep the best combination (not verified)", "Skip this disc"]);
    assert_eq!(std::fs::read(base.join("dump/G.bin")).unwrap(), first, "the dump's own files wait");
    // Every rip kept as it was read, in the run's working folder (tidied
    // away with it once the run ends clean).
    assert_eq!(std::fs::read(run.work().join("rip-1/G.bin")).unwrap(), first);
    assert_eq!(std::fs::read(run.work().join("rip-2/G.bin")).unwrap(), rip(&[], &[3]));
    retry(&run, "combine").await;
    let s = finish(run).await;
    let check = &s.steps["check"];
    assert_eq!(check.items[0].note, "Verified: Lantern Keep (Europe) (Disc 1), combined from 3 rips, 2 sectors taken from the later rips");
    assert!(check.items[0].verified);
    // The combination is the dump now, for its cue sheet and all made from it.
    assert_eq!(std::fs::read(base.join("dump/G.bin")).unwrap(), rip(&[], &[]));
    // Redump has nothing to look up: Check named it.
    assert_eq!(s.steps["redump"].status, "Not needed");
    assert_eq!(s.steps["choose"].detail, "verified");
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn rips_that_never_match_are_kept_as_the_best_combination_not_verified() {
    let base = base("combine-no-match");
    // Every rip is wrong at sector 5 the same way: nothing to take it from.
    let bad = rip(&[], &[5]);
    let (run, _e) = start(&base, &bad, vec![rip(&[2], &[5])], true);
    held(&run, 0).await;
    retry(&run, "combine").await;
    let s = held(&run, 2).await;
    assert_eq!(s.steps["check"].detail, "combined from 2 rips, 1 sector still bad");
    retry(&run, "keep").await;
    let s = finish(run).await;
    let item = &s.steps["check"].items[0];
    assert_eq!(item.note, "kept the best combination, not verified; doesn't match Redump's copies of SLES-04711");
    assert!(item.state == ItemState::Ok && !item.verified);
    assert_eq!(std::fs::read(base.join("dump/G.bin")).unwrap(), bad, "sector 5 as every rip read it, never rewritten");
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_redump_doesnt_have_is_never_called_verified() {
    let base = base("combine-unlisted");
    let (run, _e) = start(&base, &rip(&[4], &[]), vec![rip(&[], &[])], false);
    let s = held(&run, 0).await;
    assert_eq!(s.steps["check"].status, "Not verified");
    assert_eq!(labels(&s), ["Rip again and combine", "Keep it anyway", "Skip this disc"]);
    let m: Mismatch = serde_json::from_value(s.steps["check"].record.report.clone().unwrap()).unwrap();
    assert!(m.reading[0].starts_with("Redump has no copy of this disc to compare with"), "{:?}", m.reading);
    retry(&run, "combine").await;
    // Every sector passes now, and still: nothing to compare with.
    let s = held(&run, 2).await;
    let m: Mismatch = serde_json::from_value(s.steps["check"].record.report.clone().unwrap()).unwrap();
    assert!(m.reading[0].contains("Redump has no copy to compare with, so it can't be called verified"), "{:?}", m.reading);
    assert_eq!(labels(&s)[1], "Keep the best combination (not verified)");
    retry(&run, "keep").await;
    let s = finish(run).await;
    let item = &s.steps["check"].items[0];
    assert_eq!(item.note, "kept the best combination, not verified");
    assert!(!item.verified);
    assert_eq!(std::fs::read(base.join("dump/G.bin")).unwrap(), rip(&[], &[]));
    std::fs::remove_dir_all(base).unwrap();
}
