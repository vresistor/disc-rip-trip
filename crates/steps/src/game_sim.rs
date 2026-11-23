//! Simulated game steps: the real ones' `StepInfo`, with real discs' data
//! (Redump's own entries and hashes) and no drive or network. The real
//! Redump lookup and Choose run on a datfile in Redump's shape.
//!
//! Each run is one disc. Game disc gives the discs it was made with in
//! turn, so a two-disc game is two runs on one engine, as at the drive.

use crate::real::game::{self, Identity};
use crate::real::mismatch::{Combination, Fixing, RipRead};
use crate::real::{data, media, redump};
use anyhow::Result;
use async_trait::async_trait;
use drt_engine::{Context, Cx, Inlet, Item, ItemKind, ItemState, Outlet, Registry, Step, StepInfo, Subject};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub fn register(r: &mut Registry) {
    r.add(GameDisc::new(vec![APE_ESCAPE, COLONY_WARS_1, COLONY_WARS_2]));
    r.add(Rip);
    r.add(Redump);
    r.add(game::Choose);
    r.add(M3u::default());
    r.add(LibCrypt);
}

/// Entries from Redump's PlayStation datfile (Colony Wars and Ape Escape
/// with their real hashes), a revision family sharing one serial, and a
/// Mega CD serial Redump spells without its letters.
pub const GAME_DAT: &str = r#"<?xml version="1.0"?>
<datafile>
  <header>
    <name>Sony - PlayStation</name>
    <version>2026-06-15 11-55-46</version>
  </header>
  <game name="Colony Wars (Europe) (Disc 1)">
    <category>Games</category>
    <serial>SLES-00860</serial>
    <description>Colony Wars (Europe) (Disc 1)</description>
    <rom name="Colony Wars (Europe) (Disc 1).cue" size="228" crc="066c0d19" md5="8bf48acf5b1c376919faa15fb1869a1e" sha1="5f27b78b589201d71e6b3cf741a88744372e83cc"/>
    <rom name="Colony Wars (Europe) (Disc 1) (Track 1).bin" size="525695520" crc="cb0b37d2" md5="47633e34beca44c2468e49b79654e972" sha1="7210064da9a542124d4e32d6202672cc6b5756d2"/>
    <rom name="Colony Wars (Europe) (Disc 1) (Track 2).bin" size="33908784" crc="857edda8" md5="369ff3006396bbca8d9a1fb9cc125af9" sha1="28a06c34d53442f736fed283c086340d7c56321e"/>
  </game>
  <game name="Colony Wars (Europe) (Disc 2)">
    <category>Games</category>
    <serial>SLES-10860</serial>
    <description>Colony Wars (Europe) (Disc 2)</description>
    <rom name="Colony Wars (Europe) (Disc 2).cue" size="228" crc="87779d74" md5="87e98c0d96fbb52c01e9eb2442c43435" sha1="95338ad0d28f3722fda890d7caf3174b7f67651b"/>
    <rom name="Colony Wars (Europe) (Disc 2) (Track 1).bin" size="642338256" crc="7b19d357" md5="1c434ca0e78dd4c00bbe0de4e7c43b79" sha1="4c1ee2aa8703e3d3d85d96fc82947dfd679bb9d6"/>
    <rom name="Colony Wars (Europe) (Disc 2) (Track 2).bin" size="33908784" crc="857edda8" md5="369ff3006396bbca8d9a1fb9cc125af9" sha1="28a06c34d53442f736fed283c086340d7c56321e"/>
  </game>
  <game name="Ape Escape (Europe)">
    <category>Games</category>
    <serial>SCES-01564, SCES-01564-P</serial>
    <rom name="Ape Escape (Europe).cue" size="85" crc="058a8e68" md5="1c91ffd1b70589cc160cc0ee004a00a5" sha1="694d806d56841a3547557867aa0e197e021f8674"/>
    <rom name="Ape Escape (Europe).bin" size="427198464" crc="fe56b133" md5="41b939b0d59bf39e5dc89ec38ea421fb" sha1="abe619ea253b6af1ceee54d31499f0db2919915d"/>
  </game>
  <game name="Tomb Raider (USA)">
    <serial>SLUS-00152</serial>
    <rom name="Tomb Raider (USA) (Track 01).bin" size="11760000" crc="01" md5="01" sha1="7700000000000000000000000000000000000001"/>
  </game>
  <game name="Tomb Raider (USA) (Rev 1)">
    <serial>SLUS-00152</serial>
    <version>Rev 1</version>
    <rom name="Tomb Raider (USA) (Rev 1) (Track 01).bin" size="2352000" crc="02" md5="02" sha1="7700000000000000000000000000000000000002"/>
  </game>
  <game name="Lantern Keep (Europe) (Disc 1)">
    <serial>SLES-04711</serial>
    <rom name="Lantern Keep (Europe) (Disc 1).cue" size="90" crc="04" md5="04" sha1="7700000000000000000000000000000000000004"/>
    <rom name="Lantern Keep (Europe) (Disc 1).bin" size="470400000" crc="05" md5="05" sha1="43c1fe2600000000000000000000000000000005"/>
  </game>
  <game name="Lantern Keep (Europe) (Disc 1) (Rev 1)">
    <serial>SLES-04711</serial>
    <rom name="Lantern Keep (Europe) (Disc 1) (Rev 1).bin" size="471105600" crc="06" md5="06" sha1="7700000000000000000000000000000000000006"/>
  </game>
  <game name="Sonic CD (Europe)">
    <serial>4407</serial>
    <rom name="Sonic CD (Europe).bin" size="2352" crc="03" md5="03" sha1="7700000000000000000000000000000000000003"/>
  </game>
</datafile>"#;

/// One simulated disc: what it says about itself, and its dump's hashes.
#[derive(Clone, Copy, Debug)]
pub struct Disc {
    pub label: &'static str,
    pub serial: &'static str,
    pub region: &'static str,
    pub sectors: u64,
    pub bytes: u64,
    /// The dump's files and their SHA-1s, cue first.
    pub roms: &'static [(&'static str, &'static str)],
    /// Their sizes, in the same order, when they matter (an empty list
    /// takes Redump's).
    pub sizes: &'static [u64],
    /// redumper's log of the read, when there's something to say.
    pub log: &'static str,
    /// Sectors the drive can't read: its read crawls over them, and so does
    /// re-reading them, which brings few back.
    pub unreadable: u64,
}

pub const APE_ESCAPE: Disc = Disc {
    label: "SCES_015.64",
    serial: "SCES-01564",
    region: "Europe",
    sectors: 181_632,
    bytes: 371_982_336,
    roms: &[("SCES_015.64.cue", "0000000000000000000000000000000000000000"), ("SCES_015.64.bin", "abe619ea253b6af1ceee54d31499f0db2919915d")],
    sizes: &[],
    log: "",
    unreadable: 0,
};

pub const COLONY_WARS_1: Disc = Disc {
    label: "SLES_008.60",
    serial: "SLES-00860",
    region: "Europe",
    sectors: 223_510,
    bytes: 457_748_480,
    roms: &[
        ("SLES_008.60.cue", "0000000000000000000000000000000000000000"),
        ("SLES_008.60 (Track 1).bin", "7210064da9a542124d4e32d6202672cc6b5756d2"),
        ("SLES_008.60 (Track 2).bin", "28a06c34d53442f736fed283c086340d7c56321e"),
    ],
    sizes: &[],
    log: "",
    unreadable: 0,
};

pub const COLONY_WARS_2: Disc = Disc {
    label: "SLES_108.60",
    serial: "SLES-10860",
    region: "Europe",
    sectors: 273_103,
    bytes: 559_314_944,
    roms: &[
        ("SLES_108.60.cue", "0000000000000000000000000000000000000000"),
        ("SLES_108.60 (Track 1).bin", "4c1ee2aa8703e3d3d85d96fc82947dfd679bb9d6"),
        ("SLES_108.60 (Track 2).bin", "28a06c34d53442f736fed283c086340d7c56321e"),
    ],
    sizes: &[],
    log: "",
    unreadable: 0,
};

/// A Tomb Raider whose hash Redump doesn't have: its serial names two
/// revisions, and its size is the second's.
pub const TOMB_RAIDER_UNKNOWN: Disc = Disc {
    label: "SLUS_001.52",
    serial: "SLUS-00152",
    region: "USA",
    sectors: 1_010,
    bytes: 2_068_480,
    roms: &[("SLUS_001.52.cue", "0000000000000000000000000000000000000000"), ("SLUS_001.52.bin", "1234000000000000000000000000000000000000")],
    sizes: &[],
    log: "",
    unreadable: 0,
};

/// A made-up game read badly by a drive redumper doesn't know: the same
/// size as Redump's copy, some sectors wrong, and the drive none the wiser.
pub const LANTERN_KEEP_DAMAGED: Disc = Disc {
    label: "SLES_047.11",
    serial: "SLES-04711",
    region: "Europe",
    sectors: 200_000,
    bytes: 409_600_000,
    roms: &[("SLES_047.11.cue", "0000000000000000000000000000000000000000"), ("SLES_047.11.bin", "9b07a5e100000000000000000000000000000000")],
    sizes: &[90, 470_400_000],
    log: LANTERN_KEEP_LOG,
    unreadable: 0,
};

/// A made-up game's second disc, scratched past reading in one place.
pub const LANTERN_KEEP_SCRATCHED: Disc = Disc {
    label: "SLES_147.11",
    serial: "SLES-14711",
    region: "Europe",
    sectors: 269_159,
    bytes: 551_237_632,
    roms: &[("SLES_147.11.cue", "0000000000000000000000000000000000000000"), ("SLES_147.11.bin", "9b07a5e200000000000000000000000000000000")],
    sizes: &[],
    log: "",
    unreadable: 774,
};

/// Its log, in the shape redumper (b744) writes one: trimmed, made up.
pub const LANTERN_KEEP_LOG: &str = "\
redumper (build: b744)
arguments: dump --drive=/dev/sr0 --retries=20 --auto-detect --overwrite --image-path=image --image-name=SLES_047.11
warning: drive read offset not found in the database
warning: drive not found in the database
warning: using generic drive
drive information
  inquiry: ACME - DVD-RW XY123 (revision level: 1.00)
  configuration: GENERIC (read offset: +0, C2 shift: 0, pre-gap start: +0, read method: BE, sector order: DATA_C2_SUB)
*** DUMP (time check: 0s)
disc TOC:
  track 1 {  data }
    index 01 { LBA:      0, MSF: 00:02:00 }
  track A {  data }
    index 01 { LBA: 200000, MSF: 44:28:50 }
media errors:
  SCSI: 0 samples
  C2: 0 samples
  Q: 120
*** END (time check: 300s)
*** INFO (time check: 0s)
CD-ROM [SLES_047.11.bin]:
  sectors count: 200000
  MSF errors: 140 (LBA: 22000, 31000, 98000-98001, 136000)
  ECC errors: 15 (LBA: 98000, 99000, 110000, 120000, 136000)
  EDC errors: 12 (LBA: 98000, 99000, 120000, 136000)

  REDUMP.INFO errors: 15
*** END (time check: 1s)
";

/// The dump's files as `name=sha1`, for Check.
const ROMS: &str = "sim roms";
/// redumper's log of the simulated read.
const LOG: &str = "sim log";
/// The sectors the simulated drive can't read.
const UNREADABLE: &str = "sim unreadable";

// ---- Game disc ----------------------------------------------------------------------------------

/// Gives `discs` in turn, one per run.
pub struct GameDisc {
    discs: Vec<Disc>,
    next: AtomicUsize,
}

impl GameDisc {
    pub fn new(discs: Vec<Disc>) -> Self {
        GameDisc { discs, next: AtomicUsize::new(0) }
    }
}

#[async_trait]
impl Step for GameDisc {
    fn info(&self) -> StepInfo {
        game::game_disc_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(disc) = inlet.recv().await {
            let d = self.discs[self.next.fetch_add(1, Ordering::SeqCst) % self.discs.len()];
            cx.sleep(200).await?;
            let id = Identity {
                system: Some("PlayStation"),
                serial: Some(d.serial.into()),
                region: Some(d.region.into()),
                boot: Some(d.label.into()),
                label: d.label.into(),
                sectors: Some(d.sectors),
                ..Identity::default()
            };
            let found = media::Found {
                label: d.label.into(),
                filesystem: "ISO 9660".into(),
                media: "CD".into(),
                bytes: d.bytes,
                tracks: (d.roms.len() as u32).saturating_sub(1).max(1),
                audio_tracks: (d.roms.len() as u32).saturating_sub(2),
            };
            let mut item = disc.becomes(ItemKind::GameDisc, None);
            let (status, system, detail) = game::record(item.context_mut(), &id, &found);
            let roms: Vec<String> = d.roms.iter().enumerate().map(|(i, (n, s))| format!("{n}={s}={}", d.sizes.get(i).copied().unwrap_or(0))).collect();
            item.context_mut().add(Subject::Disc, ROMS, roms.join("|"), "the disc");
            if !d.log.is_empty() {
                item.context_mut().add(Subject::Disc, LOG, d.log, "the disc");
            }
            if d.unreadable > 0 {
                item.context_mut().add(Subject::Disc, UNREADABLE, d.unreadable.to_string(), "the disc");
            }
            cx.outcome(status, system, detail);
            item.label = d.label.into();
            outlet.send(item).await;
        }
        Ok(())
    }
}

fn sim_roms(c: &Context) -> Option<Vec<redump::Rom>> {
    let all = c.get(Subject::Disc, ROMS)?;
    Some(
        all.split('|')
            .filter_map(|r| {
                let mut f = r.split('=');
                Some((f.next()?, f.next()?, f.next().and_then(|n| n.parse().ok()).unwrap_or(0)))
            })
            .map(|(name, sha1, size)| redump::Rom { name: name.into(), size, crc: String::new(), md5: String::new(), sha1: sha1.into() })
            .collect(),
    )
}

// ---- Rip ----------------------------------------------------------------------------------------

struct Rip;

#[async_trait]
impl Step for Rip {
    fn info(&self) -> StepInfo {
        game::rip_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let Some(disc) = inlet.recv().await else { return Ok(()) };
        let item = Item::new(data::DUMP, ItemKind::Dump, disc.label.clone(), None, None).knowing(&disc);
        // Read as the real one reads: Image's passes, its notes.
        let bytes: u64 = disc.context().get(Subject::Disc, "bytes").and_then(|b| b.parse().ok()).unwrap_or(0);
        let sectors: u64 = disc.context().get(Subject::Disc, "sectors").and_then(|b| b.parse().ok()).unwrap_or(bytes / 2048);
        if let Some(n) = disc.context().get(Subject::Disc, UNREADABLE).and_then(|n| n.parse().ok()) {
            if crate::sim::unreadable_dump(&cx, &item, sectors, n).await?.is_none() {
                return Ok(());
            }
        } else if crate::sim::dump(&cx, &item, bytes / 2048, false).await?.is_none() {
            return Ok(());
        }
        outlet.send(item.becomes(ItemKind::Dump, Some(PathBuf::from(format!("{}.cue", disc.label))))).await;
        Ok(())
    }
}

// ---- Check, for a game ----------------------------------------------------------------------------

/// What the simulated Check does with a game's dump: its hashes, looked up
/// in the game's own catalogue, with the real lookup, and held as the real
/// one holds it when Redump has its serial but not its hash; a re-read
/// doesn't change that here. Sends it on unless it's left out. `false` for
/// a dump that isn't a simulated game's, for the data disc Check to handle.
pub async fn check(cx: &Cx, dump: &Item, outlet: &Outlet) -> Result<bool> {
    let Some(roms) = sim_roms(dump.context()) else { return Ok(false) };
    let mut dump = dump.clone();
    cx.item(&dump, ItemState::Busy, "checking");
    cx.status("", format!("Checking {} files", roms.len()));
    cx.sleep(800).await?;
    for r in &roms {
        dump.context_mut().add(Subject::Disc, &format!("sha1 {}", r.name), r.sha1.clone(), "Check");
    }
    let media = dump.context().get(Subject::Disc, "media").unwrap_or("").to_owned();
    let systems = game::catalogues(dump.context(), &cx.text("systems"), &media);
    let found = systems.iter().find(|s| s.slug == "psx").and_then(|sys| {
        let cat = redump::parse(GAME_DAT.as_bytes()).ok()?;
        cat.by_hashes(&roms).map(|g| (g.name.clone(), sys.name))
    });
    if let Some((name, system)) = &found {
        let c = dump.context_mut();
        c.add(Subject::Disc, "name", name.clone(), "Redump");
        c.add(Subject::Disc, "system", *system, "Redump");
    }
    let (note, status, detail) = data::verdict(found.as_ref().map(|(n, s)| (n.as_str(), *s)), systems.len(), &[]);
    if found.is_some() {
        cx.verified(&dump, note);
    } else {
        cx.item(&dump, ItemState::Ok, note);
    }
    cx.status(status, detail);
    let serial = dump.context().get(Subject::Disc, "serial").unwrap_or("").to_owned();
    let cat = redump::parse(GAME_DAT.as_bytes())?;
    let dump = if found.is_none() && game::is_suspect(&cat, &serial, &roms) {
        let log = dump.context().get(Subject::Disc, LOG).map(str::to_owned);
        // The dump's files as they are now: a combination changes them.
        let explain = |d: &Item| {
            let now: Vec<redump::Rom> = roms.iter().map(|r| redump::Rom { sha1: d.context().get(Subject::Disc, &format!("sha1 {}", r.name)).unwrap_or(&r.sha1).to_owned(), ..r.clone() }).collect();
            crate::real::mismatch::explain(&serial, &cat.by_serial(&serial), &now, log.as_deref())
        };
        let reread = Reread { rips: Mutex::new(1), log: log.is_some(), roms: roms.clone(), last: Mutex::default(), fixed: Mutex::new(false) };
        game::hold(cx, dump, &serial, &reread, Some(&cat), game::Policy::of(cx), explain).await?
    } else {
        Some(dump)
    };
    if let Some(d) = dump {
        outlet.send(d).await;
    }
    Ok(true)
}

/// A simulated disc read again: a re-read changes nothing; fixing bad
/// sectors puts all but one right; each rip again is combined with those
/// before, and the third (or, after a fix, the second) puts every sector
/// right, so the combination is Redump's copy.
struct Reread {
    /// Whether bad sectors were fixed.
    fixed: Mutex<bool>,
    rips: Mutex<usize>,
    /// Only a read with a log (a CD image redumper made) can be combined.
    log: bool,
    roms: Vec<redump::Rom>,
    /// What the last combination came to.
    last: Mutex<Vec<redump::Rom>>,
}

/// Each simulated rip's sectors that fail their check.
const RIP_BAD: [u64; 4] = [140, 152, 131, 145];

#[async_trait]
impl game::Reread for Reread {
    async fn refine(&self, cx: &Cx, _: &mut Item) -> Result<Option<String>> {
        cx.sleep(2000).await?;
        Ok(None)
    }

    fn can_combine(&self, _: &Item) -> bool {
        self.log
    }

    async fn fix(&self, cx: &Cx, dump: &Item, tries: u32) -> Result<(Fixing, Vec<redump::Rom>)> {
        use crate::map::{Mark, SectorMap};
        use crate::real::fix::{self, Last, Outcome, Rereading, Sector};
        let bad = RIP_BAD[0];
        cx.item(dump, ItemState::Busy, "looking for bad sectors");
        cx.sleep(600).await?;
        // Each read again in turn, as the drive would: the first never
        // comes back, the rest do on their first try or two.
        let at = |k: u64| 98_000 + k as i64 * 731;
        let lbas: Vec<i64> = (0..bad).map(at).collect();
        let mut live = game::LiveMap::new(SectorMap::checked(200_000, &[], true));
        live.found(cx, &lbas);
        let mut r = Rereading { total: bad, tries, ..Rereading::default() };
        for (k, &lba) in lbas.iter().enumerate() {
            let takes = if k == 0 { tries } else { 1 + (k % 5 == 0) as u32 };
            r.at = lba;
            for t in 1..=takes {
                (r.attempt, r.last) = (t, (t > 1).then_some(if k == 0 { Last::Failed } else { Last::Wrong }));
                game::say_rereading(cx, dump, &r);
                if t == 1 {
                    live.at(cx, lba);
                }
                cx.sleep(if k == 0 { 60 } else { 40 }).await?;
            }
            r.done += 1;
            let outcome = if k == 0 {
                r.left += 1;
                Outcome::Left { tries, same: false, alike: 0, differed: false, errors: tries, last_error: Some("sense 3/11/05".into()), stalled: false }
            } else {
                r.fixed += 1;
                Outcome::Fixed { tries: takes }
            };
            if r.done >= fix::PACE_AFTER {
                r.left_secs = Some((bad - r.done) * 20);
            }
            let s = Sector { lba, why: "wrong address", outcome };
            cx.log(fix::sector_line(&s, tries));
            live.done(cx, &s);
        }
        cx.now(None);
        cx.progress(None);
        let found: Vec<(u64, Mark)> = lbas.iter().enumerate().map(|(k, l)| (*l as u64, if k == 0 { Mark::Wrong } else { Mark::Recovered })).collect();
        cx.map(serde_json::to_value(SectorMap::checked(200_000, &found, true)).ok());
        *self.fixed.lock().unwrap() = true;
        let f = Fixing { bad, fixed: bad - 1, left: 1, left_at: vec![98_000], same: 0, tries, other_disc: false, verified: false, read_differently: vec![98_000], ..Fixing::default() };
        cx.log(format!("fix bad sectors: {bad} fail their own check; {} read right and put in place; sector 98000: no good copy in {tries} tries", bad - 1));
        // Still not Redump's: one sector left.
        let roms: Vec<redump::Rom> = self.roms.iter().map(|r| if r.is_data() { redump::Rom { sha1: format!("f1ced000{}", &r.sha1[8..]), ..r.clone() } } else { r.clone() }).collect();
        *self.last.lock().unwrap() = roms.clone();
        Ok((f, roms))
    }

    async fn settled(&self, _: &Cx, _: &Item) -> Result<()> {
        Ok(())
    }

    async fn combine(&self, cx: &Cx, dump: &Item) -> Result<(Combination, Vec<redump::Rom>)> {
        let n = {
            let mut r = self.rips.lock().unwrap();
            *r += 1;
            *r
        };
        cx.sleep(1500).await?;
        cx.item(dump, ItemState::Busy, format!("combining {n} rips"));
        cx.sleep(500).await?;
        let fixed = *self.fixed.lock().unwrap();
        let right = n >= 3 || (fixed && n >= 2);
        let c = Combination {
            // After a fix, the first rip is the fixed one.
            rips: (1..=n).map(|k| RipRead { n: k, bad: if k == 1 && fixed { 1 } else { RIP_BAD[(k - 1) % RIP_BAD.len()] } }).collect(),
            sectors: 200_000,
            differing: if right { 262 } else { 248 },
            taken: match (right, fixed) {
                (true, true) => 1,
                (true, false) => 140,
                (false, _) => 131,
            },
            no_good: if right { 0 } else { 4 },
            disagree: 0,
            still_bad: if right { 0 } else { 4 },
            left: if right { vec![] } else { vec![98_000, 99_000, 120_000, 136_000] },
            left_out: vec![],
            verified: false,
            secs: None,
        };
        let cat = redump::parse(GAME_DAT.as_bytes())?;
        let theirs = cat.by_serial(dump.context().get(Subject::Disc, "serial").unwrap_or("")).first().map(|g| g.roms.clone()).unwrap_or_default();
        // The data track becomes Redump's once it's right; till then, another wrong copy.
        let roms: Vec<redump::Rom> = self
            .roms
            .iter()
            .map(|r| match theirs.iter().find(|t| t.is_data() && r.is_data()) {
                Some(t) if right => redump::Rom { name: r.name.clone(), ..t.clone() },
                _ if r.is_data() => redump::Rom { sha1: format!("{:08x}{}", 0x5eed_0000u32 + n as u32, &r.sha1[8..]), ..r.clone() },
                _ => r.clone(),
            })
            .collect();
        *self.last.lock().unwrap() = roms.clone();
        Ok((c, roms))
    }

    async fn adopt(&self, _: &Cx, _: &Item) -> Result<()> {
        Ok(())
    }

    async fn rehash(&self, _: &Cx, _: &Item) -> Result<Vec<redump::Rom>> {
        Ok(self.last.lock().unwrap().clone())
    }

    /// The disc's error correction: after a fix, it repairs the one sector
    /// left, and the copy is Redump's; before one, it repairs all but 3 of
    /// the first rip's bad sectors (form 2, with none), and it isn't. Its
    /// map of the disc says which.
    async fn repair(&self, cx: &Cx, dump: &Item, _fixed: Option<&Fixing>, which: game::Ecc) -> Result<(crate::real::mismatch::Repairing, Vec<redump::Rom>)> {
        use crate::map::{Mark, SectorMap};
        use crate::real::mismatch::Repairing;
        cx.item(dump, ItemState::Busy, "repairing by the disc's error correction");
        cx.sleep(600).await?;
        if which != game::Ecc::ReadWrong {
            return Ok((Repairing::default(), self.last.lock().unwrap().clone()));
        }
        let fixed = *self.fixed.lock().unwrap();
        let bad = if fixed { 1 } else { RIP_BAD[0] };
        let form2 = if fixed { 0 } else { 3 };
        let r = Repairing { bad, repaired: bad - form2, form2, ..Repairing::default() };
        let at = |k: u64| 98_000 + k * 731;
        let mut found: Vec<(u64, Mark)> = (0..RIP_BAD[0]).map(|k| (at(k), if fixed { Mark::Recovered } else { Mark::Repaired })).collect();
        found[0].1 = Mark::Repaired;
        for f in found.iter_mut().rev().take(form2 as usize) {
            f.1 = Mark::Wrong;
        }
        cx.map(serde_json::to_value(SectorMap::checked(200_000, &found, true)).ok());
        let cat = redump::parse(GAME_DAT.as_bytes())?;
        let theirs = cat.by_serial(dump.context().get(Subject::Disc, "serial").unwrap_or("")).first().map(|g| g.roms.clone()).unwrap_or_default();
        let roms: Vec<redump::Rom> = self
            .roms
            .iter()
            .map(|r| match theirs.iter().find(|t| t.is_data() && r.is_data()) {
                Some(t) if fixed => redump::Rom { name: r.name.clone(), ..t.clone() },
                _ if r.is_data() => redump::Rom { sha1: format!("ecc00000{}", &r.sha1[8..]), ..r.clone() },
                _ => r.clone(),
            })
            .collect();
        *self.last.lock().unwrap() = roms.clone();
        Ok((r, roms))
    }
}

// ---- Redump -------------------------------------------------------------------------------------

struct Redump;

#[async_trait]
impl Step for Redump {
    fn info(&self) -> StepInfo {
        game::redump_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let mut any = false;
        while let Some(mut dump) = inlet.recv().await {
            any = true;
            let (system, serial, sectors) = game::known(dump.context());
            let system = system.unwrap_or_default();
            cx.status("", format!("Looking up {system}"));
            cx.sleep(300).await?;
            // Hashed when Check has been: the hashes it recorded.
            let c = dump.context();
            let roms: Vec<redump::Rom> =
                sim_roms(c).unwrap_or_default().into_iter().filter_map(|r| Some(redump::Rom { sha1: c.get(Subject::Disc, &format!("sha1 {}", r.name))?.to_owned(), ..r })).collect();
            let cat = redump::parse(GAME_DAT.as_bytes())?;
            let l = game::after_check(dump.context(), game::lookup(&cat, &system, serial.as_deref(), sectors, &roms));
            game::offer(&cx, &mut dump, l);
            outlet.send(dump).await;
        }
        if !any {
            game::nothing_came(&cx);
        }
        Ok(())
    }
}

// ---- M3U ----------------------------------------------------------------------------------------

/// Remembers the discs of each set filed, across runs, as the folder would.
#[derive(Default)]
struct M3u {
    filed: Mutex<BTreeMap<String, BTreeSet<u32>>>,
}

#[async_trait]
impl Step for M3u {
    fn info(&self) -> StepInfo {
        game::m3u_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(file) = inlet.recv().await {
            cx.sleep(100).await?;
            match game::set_of(file.context()) {
                Some((set, n)) => {
                    let count = {
                        let mut f = self.filed.lock().unwrap();
                        let discs = f.entry(set.clone()).or_default();
                        discs.insert(n);
                        discs.len()
                    };
                    if count >= 2 {
                        let name = format!("{}.m3u", crate::template::safe(&set));
                        cx.item(&file, ItemState::Ok, format!("{name}, {count} discs"));
                        cx.outcome(name, format!("{count} discs"), "");
                    } else {
                        cx.item(&file, ItemState::Ok, format!("disc {n}; the playlist waits for another"));
                        cx.status(format!("Disc {n} of {set}"), "Waits for another disc");
                    }
                }
                None => {
                    cx.item(&file, ItemState::Ok, "one disc, no playlist");
                    cx.status("One disc", "no playlist");
                }
            }
            outlet.send(file).await;
        }
        Ok(())
    }
}

// ---- LibCrypt -----------------------------------------------------------------------------------

/// The games Redump's SBI pack has, of the simulated ones.
pub const PROTECTED: &[&str] = &["Ape Escape (Europe)"];

struct LibCrypt;

#[async_trait]
impl Step for LibCrypt {
    fn info(&self) -> StepInfo {
        game::libcrypt_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(file) = inlet.recv().await {
            cx.sleep(200).await?;
            let name = data::dump_fields(file.context()).name;
            if PROTECTED.contains(&name.as_str()) {
                let note = format!("{}.sbi beside it", crate::template::safe(&name));
                cx.item(&file, ItemState::Ok, note.clone());
                cx.status("Protection file saved", note);
            } else {
                cx.item(&file, ItemState::Ok, "not protected");
                cx.status("Not protected", "not protected");
            }
            outlet.send(file).await;
        }
        Ok(())
    }
}
