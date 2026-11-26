//! The real Check fixing a held dump's bad sectors and ripping it again
//! and combining the rips, with stand-ins for the rip and the drive: each
//! rip is a made-up one-track PlayStation image whose sectors carry real
//! addresses and EDCs, wrong in different places, and the drive misreads
//! a sector a few times before it reads it right, or never does.
//!
//! Asked first: a match with Redump after three rips goes on verified;
//! rips that never match are kept as the best combination, not verified;
//! and a disc Redump doesn't have is never called verified. By itself:
//! fixed by re-reads alone; fixed, then a second rip; never a match, so
//! it asks; and a disc Redump doesn't have, fixed and still held.

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use trip_engine::recipe::chain;
use trip_engine::{Answer, Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Run, Snapshot, Step, StepInfo, Subject};
use trip_steps::map::{Mark, Run as Stretch};
use trip_steps::real::combine::{edc, msf, SECTOR};
use trip_steps::real::data::{self, DUMP};
use trip_steps::real::ecc;
use trip_steps::real::fix::Source;
use trip_steps::real::game::{RipFn, SourceFn};
use trip_steps::real::mismatch::Mismatch;

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
    std::fs::write(dir.join("G.log"), trip_steps::game_sim::LANTERN_KEEP_LOG).unwrap();
    let cue = dir.join("G.cue");
    std::fs::write(&cue, "FILE \"G.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n").unwrap();
    cue
}

fn md5(bytes: &[u8]) -> String {
    use md5::Digest;
    trip_steps::real::hash::hex(&md5::Md5::digest(bytes))
}

fn sha1(bytes: &[u8]) -> String {
    use sha1::Digest;
    trip_steps::real::hash::hex(&sha1::Sha1::digest(bytes))
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

/// A stand-in drive with the disc in: sector `lba` reads wrong (slipped or
/// a byte off, as a seeded random draw decides) the first `misreads` of
/// its reads, then right; `never` sectors always read a byte off.
struct Flaky {
    misreads: u32,
    never: Vec<i64>,
    reads: std::collections::HashMap<i64, u32>,
    seed: u64,
}

impl Source for Flaky {
    fn read(&mut self, lba: i64) -> std::io::Result<Vec<u8>> {
        let n = self.reads.entry(lba).or_insert(0);
        *n += 1;
        if self.never.contains(&lba) {
            return Ok(rip(&[], &[lba])[lba as usize * SECTOR..][..SECTOR].to_vec());
        }
        if *n <= self.misreads {
            self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            return Ok(match self.seed >> 62 {
                0 => sector(lba - 1),
                1 => return Err(std::io::Error::other("the drive refused the command (sense 3/11/05)")),
                _ => {
                    let mut s = sector(lba);
                    s[100 + (self.seed >> 40) as usize % 2000] ^= 0x08;
                    s
                }
            });
        }
        Ok(sector(lba))
    }

    fn clear_cache(&mut self, _near: i64) -> std::io::Result<()> {
        Ok(())
    }
}

fn drive(misreads: u32, never: &[i64]) -> SourceFn {
    let never = never.to_vec();
    Arc::new(move |_dump: &Item| Ok(Box::new(Flaky { misreads, never: never.clone(), reads: Default::default(), seed: 7 }) as Box<dyn Source>))
}

/// A run of the real Check, Redump and Choose on the first rip, Redump's
/// list holding the disc when `listed`, Check asking first.
fn start(base: &Path, first: &[u8], later: Vec<Vec<u8>>, listed: bool) -> (Run, Engine) {
    start_with(base, first, later, listed, "ask", drive(0, &[]))
}

/// The same, Check doing `if_no_match` with a stand-in drive.
fn start_with(base: &Path, first: &[u8], later: Vec<Vec<u8>>, listed: bool, if_no_match: &str, source: SourceFn) -> (Run, Engine) {
    start_disc(base, &rip(&[], &[]), first, later, listed, if_no_match, source)
}

/// The same, Redump's copy of the disc being `good`.
fn start_disc(base: &Path, good: &[u8], first: &[u8], later: Vec<Vec<u8>>, listed: bool, if_no_match: &str, source: SourceFn) -> (Run, Engine) {
    let _ = std::fs::remove_dir_all(base);
    let main = write_rip(&base.join("dump"), first);
    let dat = if listed {
        format!(
            r#"<?xml version="1.0"?><datafile><header><version>test</version></header>
<game name="Lantern Keep (Europe) (Disc 1)"><serial>SLES-04711</serial>
<rom name="Lantern Keep (Europe) (Disc 1).cue" size="1" crc="00" md5="00" sha1="ffff"/>
<rom name="Lantern Keep (Europe) (Disc 1).bin" size="{}" crc="{:08x}" md5="{}" sha1="{}"/></game>
</datafile>"#,
            good.len(),
            crc32fast::hash(good),
            md5(good),
            sha1(good)
        )
    } else {
        r#"<?xml version="1.0"?><datafile><header><version>test</version></header>
<game name="Something Else (USA)"><serial>SLUS-00001</serial><rom name="x.bin" size="1" crc="00" md5="00" sha1="0000"/></game></datafile>"#
            .to_owned()
    };
    std::fs::create_dir_all(base.join("cache/redump")).unwrap();
    std::fs::write(base.join("cache/redump/psx.dat"), dat).unwrap();
    let mut reg = trip_steps::real();
    reg.add(Dumped { main });
    reg.add(data::Check { rip: stand_in(later), source });
    let mut e = Engine::new(reg);
    e.work_root = base.join("runs");
    let recipe = Recipe {
        name: "Fixture combine".into(),
        root: chain([Node::step("src", "dumped"), Node::step("check", "check").with("if_no_match", if_no_match), Node::step("redump", "redump"), Node::step("choose", "game-choose")]),
    };
    trip_engine::check(&recipe.root, &e.registry).unwrap();
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
    std::env::current_dir().unwrap().join("../../target/trip-test").join(name)
}

#[tokio::test(flavor = "multi_thread")]
async fn three_rips_combined_match_redump_and_go_on_verified() {
    let base = base("combine-match");
    // Rip 1 slips at 3 and 7; rip 2 is wrong at 3 another way; rip 3 slips at 9.
    let first = rip(&[3, 7], &[]);
    let (run, _e) = start(&base, &first, vec![rip(&[], &[3]), rip(&[9], &[])], true);
    let s = held(&run, 0).await;
    assert_eq!(labels(&s), ["Fix bad sectors", "Rip again and combine", "Repair with the disc's error correction", "Keep it anyway", "Skip this disc"]);
    retry(&run, "combine").await;
    let s = held(&run, 2).await;
    let m: Mismatch = serde_json::from_value(s.steps["check"].record.report.clone().unwrap()).unwrap();
    let c = m.combined.clone().unwrap();
    assert_eq!((c.differing, c.no_good, c.still_bad, c.left.clone(), c.verified), (2, 1, 1, vec![3], false), "sector 3 has no good copy yet");
    assert_eq!(c.rips.iter().map(|r| r.bad).collect::<Vec<_>>(), [2, 1]);
    assert_eq!(labels(&s), ["Rip again and combine again", "Repair with the disc's error correction", "Keep the best combination (not verified)", "Skip this disc"]);
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
    assert_eq!(labels(&s), ["Fix bad sectors", "Rip again and combine", "Repair with the disc's error correction", "Keep it anyway", "Skip this disc"]);
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

fn report(s: &Snapshot) -> Mismatch {
    serde_json::from_value(s.steps["check"].record.report.clone().unwrap()).unwrap()
}

/// Waits till Check holds the dump, whatever it's done.
async fn asks(run: &Run) -> Snapshot {
    let mut rx = run.snapshot.clone();
    loop {
        let s = rx.borrow_and_update().clone();
        if s.steps["check"].items.first().is_some_and(|i| i.state == ItemState::Bad) {
            return s;
        }
        assert!(!s.finished, "finished without holding: {:?}", s.steps["check"].items);
        rx.changed().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn by_itself_bad_sectors_read_again_make_it_match() {
    let base = base("fix-by-itself");
    // Rip 1 slipped at 3 and 7, a byte off at 9; the drive misreads each
    // a few times, at random, before it reads it right.
    let first = rip(&[3, 7], &[9]);
    let (run, _e) = start_with(&base, &first, vec![], true, "fix", drive(3, &[]));
    let s = finish(run).await;
    let check = &s.steps["check"];
    assert_eq!(check.items[0].note, "Verified: Lantern Keep (Europe) (Disc 1), 3 sectors re-read");
    assert!(check.items[0].verified);
    assert_eq!(check.status, "Verified");
    assert_eq!(std::fs::read(base.join("dump/G.bin")).unwrap(), rip(&[], &[]), "fixed in place");
    let m = report(&s);
    let f = m.fixed.clone().unwrap();
    assert_eq!((f.bad, f.fixed, f.left, f.verified), (3, 3, 0, true));
    assert!(m.combined.is_none(), "no rip needed");
    assert!(
        m.reading[0].starts_with("Fix bad sectors read 3 sectors again, up to 10 times each, and put the 3 copies that passed their own check in place. It now matches Redump's copy."),
        "{:?}",
        m.reading
    );
    let log = check.record.log.iter().map(|l| l.line.as_str()).collect::<Vec<_>>().join("\n");
    assert!(log.contains("fix bad sectors: 3 of 12 data sectors fail their own check: 3 (wrong address), 7 (wrong address), 9 (fails its checksum)"), "{log}");
    assert!(log.contains("read right on a later try, with the drive's cache cleared first"), "{log}");
    assert!(log.contains(": all four the same"), "{log}");
    assert_eq!(s.steps["redump"].status, "Not needed");
    std::fs::remove_dir_all(base).unwrap();
}

/// A drive that reads every sector right, but holds on to sector `wait`
/// until it's let go.
struct Held {
    wait: i64,
    go: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Source for Held {
    fn read(&mut self, lba: i64) -> std::io::Result<Vec<u8>> {
        if lba == self.wait {
            let _ = self.go.lock().unwrap().recv();
        }
        Ok(sector(lba))
    }

    fn clear_cache(&mut self, _near: i64) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reading_bad_sectors_again_says_how_it_goes_as_it_goes() {
    use trip_steps::real::fix::Rereading;
    let base = base("fix-live");
    let first = rip(&[3, 7], &[9]);
    let (tx, rx) = std::sync::mpsc::channel();
    let rx = Arc::new(std::sync::Mutex::new(Some(rx)));
    let source: SourceFn = Arc::new(move |_dump: &Item| Ok(Box::new(Held { wait: 7, go: std::sync::Mutex::new(rx.lock().unwrap().take().unwrap()) }) as Box<dyn Source>));
    let (run, _e) = start_with(&base, &first, vec![], true, "fix", source);
    // Held on sector 7, its first try: 3 is put right already.
    let mut snap = run.snapshot.clone();
    let s = loop {
        let s = snap.borrow_and_update().clone();
        let now = s.steps["check"].now.clone().and_then(|n| serde_json::from_value::<Rereading>(n).ok());
        if now.as_ref().is_some_and(|n| n.at == 7 && n.attempt == 1) && s.steps["check"].record.map.as_ref().is_some_and(|m| m["at"] == 7) {
            break s;
        }
        assert!(!s.finished);
        snap.changed().await.unwrap();
    };
    let check = &s.steps["check"];
    let now: Rereading = serde_json::from_value(check.now.clone().unwrap()).unwrap();
    assert_eq!((now.done, now.total, now.fixed, now.left), (1, 3, 1, 0));
    assert_eq!(now.line(), "Reading bad sectors again: sector 7 · 2 of 3 · try 1 of 10");
    assert_eq!(check.status, trip_steps::real::game::REREADING);
    assert_eq!(check.items[0].note, "re-reading 33%, 1 of 3 bad sectors");
    let map = map_of(&s);
    assert_eq!((map.total(Mark::Recovered), map.total(Mark::Wrong), map.at), (1, 2, Some(7)), "3 put right, 7 and 9 still to go");
    assert!(log_of(&s).contains("fix bad sectors: sector 3: read right on try 1"), "{}", log_of(&s));
    assert!(!log_of(&s).contains("sector 7:"));
    tx.send(()).unwrap();
    let s = finish(run).await;
    let check = &s.steps["check"];
    assert!(check.items[0].verified);
    assert!(check.now.is_none(), "nothing going on now");
    let map = map_of(&s);
    assert_eq!((map.total(Mark::Recovered), map.total(Mark::Wrong), map.at), (3, 0, None));
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn by_itself_a_sector_the_drive_never_reads_right_comes_from_a_second_rip() {
    let base = base("fix-then-rip");
    // The drive reads sector 5 a byte off every time; rip 2 has it right.
    let first = rip(&[3], &[5]);
    let (run, _e) = start_with(&base, &first, vec![rip(&[8], &[])], true, "fix", drive(1, &[5]));
    let s = finish(run).await;
    let check = &s.steps["check"];
    assert_eq!(check.items[0].note, "Verified: Lantern Keep (Europe) (Disc 1), 1 sector re-read, 1 from a second rip");
    assert!(check.items[0].verified);
    assert_eq!(std::fs::read(base.join("dump/G.bin")).unwrap(), rip(&[], &[]));
    let m = report(&s);
    assert_eq!(m.fixed.as_ref().map(|f| (f.fixed, f.left)), Some((1, 1)));
    assert_eq!(m.combined.as_ref().map(|c| (c.rips.len(), c.verified)), Some((2, true)));
    let log = check.record.log.iter().map(|l| l.line.as_str()).collect::<Vec<_>>().join("\n");
    assert!(log.contains("sector 5: no good copy in 10 of 10 tries: the same as the image's on every read"), "{log}");
    assert!(log.contains("combined: from rip 2: 1 sector (5)"), "{log}");
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn by_itself_what_never_matches_is_held_for_you_after_three_more_rips() {
    let base = base("fix-never");
    // Sector 5 is a byte off in every rip and on every read.
    let bad = rip(&[2], &[5]);
    let (run, _e) = start_with(&base, &bad, vec![rip(&[], &[5]), rip(&[4], &[5]), rip(&[], &[5])], true, "fix", drive(0, &[5]));
    let s = asks(&run).await;
    let check = &s.steps["check"];
    assert_eq!(check.items[0].note, "Tried by itself (1 sector re-read, 3 more rips, nothing repaired by error correction); still doesn't match Redump");
    assert_eq!(check.status, "Doesn't match Redump");
    let m = report(&s);
    assert_eq!(m.combined.as_ref().map(|c| c.rips.len()), Some(4), "the first and three more");
    assert_eq!(labels(&s), ["Rip again and combine again", "Keep the best combination (not verified)", "Skip this disc"], "error correction tried, last");
    assert!(m.reading.iter().any(|l| l.contains("1 sector is a form 2 sector, which has no error correction.")), "{:?}", m.reading);
    assert!(run.work().join("rip-4/G.bin").is_file() && !run.work().join("rip-5").exists());
    retry(&run, "keep").await;
    let s = finish(run).await;
    let item = &s.steps["check"].items[0];
    assert!(item.state == ItemState::Ok && !item.verified, "kept only because you said");
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn by_itself_a_game_redump_doesnt_have_is_fixed_and_still_held() {
    let base = base("fix-unlisted");
    let (run, _e) = start_with(&base, &rip(&[4], &[]), vec![rip(&[], &[])], false, "fix", drive(0, &[]));
    let s = asks(&run).await;
    let check = &s.steps["check"];
    assert_eq!(check.items[0].note, "Tried by itself (1 sector re-read); not verified: no Redump entry");
    assert_eq!(check.status, "Not verified");
    assert!(!run.work().join("rip-2").exists(), "nothing to stop on, so no rips by itself");
    let m = report(&s);
    assert!(m.reading[0].contains("Every sector passes its own check now, but Redump has no copy to compare with, so it can't be called verified."), "{:?}", m.reading);
    assert_eq!(std::fs::read(base.join("dump/G.bin")).unwrap(), rip(&[], &[]));
    retry(&run, "leave_out").await;
    let s = finish(run).await;
    assert!(!s.steps["check"].items[0].verified);
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn asked_first_fix_bad_sectors_is_a_choice() {
    let base = base("fix-asked");
    let (run, _e) = start(&base, &rip(&[3, 7], &[]), vec![], true);
    let s = held(&run, 0).await;
    assert_eq!(labels(&s), ["Fix bad sectors", "Rip again and combine", "Repair with the disc's error correction", "Keep it anyway", "Skip this disc"]);
    retry(&run, "fix").await;
    let s = finish(run).await;
    assert_eq!(s.steps["check"].items[0].note, "Verified: Lantern Keep (Europe) (Disc 1), 2 sectors re-read");
    std::fs::remove_dir_all(base).unwrap();
}

// ---- the disc's own error correction, last --------------------------------------------------

/// A Mode 2 form 1 sector at `lba`, with its EDC and its P and Q parity,
/// as the disc holds it.
fn form1(lba: i64) -> Vec<u8> {
    let mut s = vec![0u8; SECTOR];
    s[1..11].fill(0xFF);
    s[12..15].copy_from_slice(&msf(lba));
    s[15] = 2;
    s[16..24].copy_from_slice(&[0, 0, 8, 0, 0, 0, 8, 0]);
    for (i, b) in s[24..2072].iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(13) ^ (lba as u8).wrapping_mul(29);
    }
    let e = edc(&s[16..2072]);
    s[2072..2076].copy_from_slice(&e.to_le_bytes());
    ecc::parity(&mut s, true);
    s
}

/// Sector `lba` with the bytes at `at` wrong, by `by`.
fn spoilt(lba: i64, at: &[usize], by: u8) -> Vec<u8> {
    let mut s = form1(lba);
    for &i in at {
        s[i] ^= by;
    }
    s
}

/// Sector 7 as pressed: one byte off its EDC, on purpose, the same on
/// every read and in Redump's copy.
fn pressed() -> Vec<u8> {
    spoilt(7, &[1000], 0x40)
}

/// A form 1 disc: each sector as it should be, but for those in `odd`.
fn disc1(odd: &[(i64, Vec<u8>)]) -> Vec<u8> {
    (0..SECTORS).flat_map(|l| odd.iter().find(|o| o.0 == l).map_or_else(|| form1(l), |o| o.1.clone())).collect()
}

/// A stand-in drive: each listed sector's reads in turn, the last over
/// again; the rest as `disc` has them.
struct Reads {
    disc: Vec<u8>,
    odd: std::collections::HashMap<i64, Vec<Vec<u8>>>,
}

impl Source for Reads {
    fn read(&mut self, lba: i64) -> std::io::Result<Vec<u8>> {
        match self.odd.get_mut(&lba) {
            Some(q) if q.len() > 1 => Ok(q.remove(0)),
            Some(q) => Ok(q[0].clone()),
            None => Ok(self.disc[lba as usize * SECTOR..][..SECTOR].to_vec()),
        }
    }

    fn clear_cache(&mut self, _near: i64) -> std::io::Result<()> {
        Ok(())
    }
}

fn reads(disc: Vec<u8>, odd: Vec<(i64, Vec<Vec<u8>>)>) -> SourceFn {
    Arc::new(move |_dump: &Item| Ok(Box::new(Reads { disc: disc.clone(), odd: odd.iter().cloned().collect() }) as Box<dyn Source>))
}

/// Sector 3 read wrong a different way each time: never right, but each
/// time few enough bytes off for its error correction.
fn misread(k: u8) -> Vec<u8> {
    spoilt(3, &[40 + k as usize, 900 + 7 * k as usize, 2100], 0x11 + k)
}

fn log_of(s: &Snapshot) -> String {
    s.steps["check"].record.log.iter().map(|l| l.line.as_str()).collect::<Vec<_>>().join("\n")
}

fn map_of(s: &Snapshot) -> trip_steps::map::SectorMap {
    serde_json::from_value(s.steps["check"].record.map.clone().expect("a map")).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn by_itself_a_sector_never_read_right_is_repaired_last_and_the_pressed_one_left_alone() {
    let base = base("ecc-by-itself");
    // Redump's copy has sector 7 as pressed. Sector 3 never reads right,
    // on a re-read or in another rip, and differently each time.
    let good = disc1(&[(7, pressed())]);
    let first = disc1(&[(3, misread(0)), (7, pressed())]);
    let later = (1..=3).map(|k| disc1(&[(3, misread(k)), (7, pressed())])).collect();
    let drive = reads(good.clone(), vec![(3, (4..14).map(misread).collect()), (7, vec![pressed()])]);
    let (run, _e) = start_disc(&base, &good, &first, later, true, "fix", drive);
    let s = finish(run).await;
    let check = &s.steps["check"];
    assert_eq!(check.items[0].note, "Verified: Lantern Keep (Europe) (Disc 1), combined from 4 rips, nothing taken from the later rips, 1 sector repaired by the disc's error correction");
    assert!(check.items[0].verified);
    let now = std::fs::read(base.join("dump/G.bin")).unwrap();
    assert_eq!(now, good, "sector 3 put right, sector 7 as pressed");
    assert_eq!(&now[7 * SECTOR..8 * SECTOR], &pressed()[..]);
    let log = log_of(&s);
    let order: Vec<usize> = [
        "by itself, as Check's settings say: Fix bad sectors",
        "by itself, as Check's settings say: Rip again and combine",
        "by itself, as Check's settings say: Repair with the disc's error correction",
    ]
    .iter()
    .map(|l| log.find(l).unwrap_or_else(|| panic!("{l}: {log}")))
    .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "the error correction last: {log}");
    assert!(log.contains("sector 3: repaired by its error correction, 3 bytes changed"), "{log}");
    assert!(log.contains("sector 7: its error correction could repair it (1 byte), but 5 reads gave it back the same, so it may be pressed so"), "{log}");
    assert!(log.contains(": all four the same"), "{log}");
    let m = report(&s);
    assert_eq!(m.repaired.as_ref().map(|r| (r.bad, r.repaired, r.held_back, r.verified)), Some((2, 1, 1, true)));
    let map = map_of(&s);
    assert_eq!((map.total(Mark::Repaired), map.total(Mark::Wrong)), (1, 1));
    assert_eq!(map.repaired, [Stretch { from: 3, to: 3 }]);
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn asked_a_repair_that_doesnt_make_it_redump_s_stays_not_verified_and_puts_back_the_pressed_one() {
    let base = base("ecc-no-match");
    // As above, but sector 9 is far past correcting, and sector 10 was
    // never read at all (filled in).
    let good = disc1(&[(7, pressed())]);
    let wrecked = spoilt(9, &(100..400).collect::<Vec<_>>(), 0x5A);
    let first = disc1(&[(3, misread(0)), (7, pressed()), (9, wrecked.clone()), (10, vec![0x55; SECTOR])]);
    let drive = reads(good.clone(), vec![(3, (4..14).map(misread).collect()), (7, vec![pressed()]), (9, vec![wrecked.clone()]), (10, vec![vec![0x55; SECTOR]])]);
    let (run, _e) = start_disc(&base, &good, &first, vec![], true, "ask", drive);
    let s = held(&run, 0).await;
    assert_eq!(labels(&s).last().map(String::as_str), Some("Skip this disc"));
    assert_eq!(labels(&s)[labels(&s).len() - 3], "Repair with the disc's error correction", "the last way to put it right");
    retry(&run, "fix").await;
    let s = asks(&run).await;
    assert!(labels(&s).contains(&"Repair with the disc's error correction".to_owned()), "{:?}", labels(&s));
    retry(&run, "repair").await;
    let mut rx = run.snapshot.clone();
    let s = loop {
        let s = rx.borrow_and_update().clone();
        if s.steps["check"].items[0].state == ItemState::Bad && report(&s).repaired.is_some() {
            break s;
        }
        rx.changed().await.unwrap();
    };
    let check = &s.steps["check"];
    assert_eq!(check.items[0].note, "Not verified: 1 sector repaired by the disc's error correction");
    assert_eq!(check.detail, "1 sector repaired by error correction, 3 sectors still bad");
    assert!(!labels(&s).contains(&"Repair with the disc's error correction".to_owned()), "once");
    let m = report(&s);
    assert_eq!(m.repaired.as_ref().map(|r| (r.bad, r.repaired, r.held_back, r.unreadable, r.beyond, r.verified)), Some((4, 1, 1, 1, 1, false)));
    let log = log_of(&s);
    assert!(log.contains("sector 3: repaired by its error correction, 3 bytes changed"), "{log}");
    assert!(log.contains("sector 7: repaired by its error correction, 1 byte changed, to see whether the image then matches Redump"), "{log}");
    assert!(log.contains("error correction: put back as read, since the image with them repaired isn't Redump's: 1 sector (7)"), "{log}");
    assert!(log.contains("sector 9: can't be repaired: too much of it is wrong to correct"), "{log}");
    assert!(log.contains("sector 10: can't be repaired: nothing was read, so there's nothing to correct"), "{log}");
    let now = std::fs::read(base.join("dump/G.bin")).unwrap();
    assert_eq!(&now[3 * SECTOR..4 * SECTOR], &form1(3)[..]);
    assert_eq!(&now[7 * SECTOR..8 * SECTOR], &pressed()[..], "put back as read");
    assert_eq!(&now[9 * SECTOR..10 * SECTOR], &wrecked[..], "left as read");
    retry(&run, "keep").await;
    let s = finish(run).await;
    let item = &s.steps["check"].items[0];
    assert_eq!(item.note, "kept with 1 sector repaired by the disc's error correction; doesn't match Redump's copies of SLES-04711");
    assert!(item.state == ItemState::Ok && !item.verified, "kept only because you said");
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn asked_first_a_sector_read_once_is_repaired_only_because_redump_then_matches() {
    let base = base("ecc-once");
    let good = disc1(&[]);
    let first = disc1(&[(4, spoilt(4, &[77, 1500], 0x21))]);
    let (run, _e) = start_disc(&base, &good, &first, vec![], true, "ask", reads(good.clone(), vec![]));
    held(&run, 0).await;
    retry(&run, "repair").await;
    let s = finish(run).await;
    let check = &s.steps["check"];
    assert_eq!(check.items[0].note, "Verified: Lantern Keep (Europe) (Disc 1), 1 sector repaired by the disc's error correction");
    assert!(check.items[0].verified);
    assert_eq!(std::fs::read(base.join("dump/G.bin")).unwrap(), good);
    let log = log_of(&s);
    assert!(log.contains("sector 4: its error correction could repair it (2 bytes), but it's been read only once"), "{log}");
    assert!(log.contains("with 1 sector that may be pressed so repaired too, it matches Redump"), "{log}");
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn by_itself_a_pressed_sector_on_a_disc_redump_doesnt_have_is_never_repaired() {
    let base = base("ecc-unlisted");
    let first = disc1(&[(3, misread(0)), (7, pressed())]);
    // Sector 7 slips once among its re-reads: two reads alike still say pressed.
    let drive = reads(disc1(&[(7, pressed())]), vec![(3, (4..14).map(misread).collect()), (7, vec![pressed(), form1(6), pressed()])]);
    let (run, _e) = start_disc(&base, &disc1(&[]), &first, vec![], false, "fix", drive);
    let s = asks(&run).await;
    let check = &s.steps["check"];
    assert_eq!(check.items[0].note, "Not verified: 1 sector repaired by the disc's error correction; tried by itself (0 sectors re-read); no Redump entry");
    let m = report(&s);
    assert!(m.reading.iter().any(|l| l.ends_with("Redump has no copy to compare with, so it can't be called verified.")), "{:?}", m.reading);
    let now = std::fs::read(base.join("dump/G.bin")).unwrap();
    assert_eq!(&now[3 * SECTOR..4 * SECTOR], &form1(3)[..], "read wrong: repaired");
    assert_eq!(&now[7 * SECTOR..8 * SECTOR], &pressed()[..], "maybe pressed so, and nothing to say otherwise: left alone");
    assert!(!log_of(&s).contains("to see whether the image then matches Redump"), "no Redump copy to try it against");
    retry(&run, "keep").await;
    let s = finish(run).await;
    let item = &s.steps["check"].items[0];
    assert_eq!(item.note, "kept with 1 sector repaired by the disc's error correction; not verified");
    assert!(!item.verified);
    std::fs::remove_dir_all(base).unwrap();
}
