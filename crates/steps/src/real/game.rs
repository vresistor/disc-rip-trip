//! Games (DESIGN-UX.md, "Other kinds of disc", 2).
//!
//! `Drive, Game disc, Rip, Check, Redump, Choose, then CHD, Library, M3U;
//! and Next disc`. Game disc reads the system and serial off the disc; Rip
//! is Image's redumper dump; Check hashes it; Redump offers the entry the
//! hash matches, or failing that the entries for the serial; Choose only
//! asks when the hash matches nothing; M3U keeps a multi-disc game's
//! playlist beside its discs.
//!
//! A serial names a game, not a pressing (`SLUS-00152` is six Tomb Raider
//! revisions), so identity waits for the hash. Each step's `StepInfo` is
//! here once and shared by the real and the simulated step.

use super::mismatch::{self, Combination, Fixing, Mismatch, Repairing};
use super::playstation::{self, Sectors, Volume};
use super::{data, media, redump};
use crate::template;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use trip_engine::snapshot::QuestionShape;
use trip_engine::{Answer, Candidate, Choice, Context, Cx, Fact, Group, Inlet, ItemKind, ItemState, LaneKind, Outlet, Question, Registry, Step, StepInfo, StepState, Subject};

pub fn register(r: &mut Registry) {
    r.add(GameDisc);
    r.add(Rip);
    r.add(Redump);
    r.add(Choose);
    r.add(M3u);
    r.add(LibCrypt);
}

// ---- what each step is ------------------------------------------------------------------------

pub fn game_disc_info() -> StepInfo {
    StepInfo::new("game-disc", "Game disc", Group::Kind, ItemKind::Disc, ItemKind::GameDisc).gives(&["serial", "label", "system"]).from("which system, and its serial number")
}

/// Image's dump, settings and all, for a game: the same redumper code.
pub fn rip_info() -> StepInfo {
    let mut i = data::image_info();
    i.kind = "game-rip";
    i.name = "Rip";
    i.input = ItemKind::GameDisc;
    i.retries.retain(|r| r.id != "leave_out");
    i
}

pub fn redump_info() -> StepInfo {
    // It looks in the catalogue of the system Game disc found; without one
    // there's nowhere to look (Check already searches every catalogue by
    // hash), so it follows a game's dump only.
    StepInfo::new("redump", "Redump", Group::LookUp, ItemKind::Dump, ItemKind::Dump)
        .needs(&["system"])
        .gives(&["candidates"])
        .unless(&["release"])
        .right_after(&["check"])
        .from("find the game in Redump's list")
}

pub fn choose_info() -> StepInfo {
    StepInfo::new("game-choose", "Choose", Group::Decide, ItemKind::Dump, ItemKind::Dump).needs(&["candidates"]).gives(&["release"]).unless(&["release"]).from("pick which game this is")
}

pub fn m3u_info() -> StepInfo {
    // Beside the filed image, named for the set from Redump's name for it
    // (Check's match by hash, or the one Choose settled).
    StepInfo::new("m3u", "M3U", Group::Make, ItemKind::ImageFile, ItemKind::ImageFile).needs(&["filed"]).needs(&["name", "release"]).from("playlist for multi-disc games")
}

// ---- what the disc says -------------------------------------------------------------------------

/// What Game disc found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    /// Redump's name for the system (`PlayStation`), or `None`.
    pub system: Option<&'static str>,
    pub serial: Option<String>,
    pub region: Option<String>,
    /// The boot file, as the disc names it.
    pub boot: Option<String>,
    pub label: String,
    /// The title the disc's header gives (Saturn, Mega CD).
    pub title: String,
    /// The filesystem's size in 2048-byte sectors.
    pub sectors: Option<u64>,
    /// Why this disc can't usefully be dumped as a game.
    pub refusal: Option<String>,
}

impl Identity {
    /// The tile's line: "PlayStation, SLES-00860".
    pub fn headline(&self) -> String {
        match (self.system, &self.serial) {
            (Some(s), Some(n)) => format!("{s}, {n}"),
            (Some(s), None) => s.to_owned(),
            (None, _) => "Game disc (unknown system)".to_owned(),
        }
    }

    /// Redump's short name for the system's catalogue.
    pub fn slug(&self) -> Option<&'static str> {
        slug_of(self.system?)
    }
}

pub fn slug_of(system: &str) -> Option<&'static str> {
    redump::SYSTEMS.iter().find(|s| s.name == system).map(|s| s.slug)
}

/// Reads what a game disc says about itself, from the track at `start`
/// (its first data track). Each system its own way: the header in the first
/// sectors for Sega, NEC and 3DO; `SYSTEM.CNF` for PlayStation. An error is
/// a read that failed partway, worth retrying; a disc that's none of these
/// is an `Identity` with no system.
pub fn identify(read: Sectors, start: u64) -> std::io::Result<Identity> {
    let head = read(start, 2).unwrap_or_default();
    let at = |from: usize, to: usize| head.get(from..to).map(playstation::ascii).unwrap_or_default();
    let starts = |s: &[u8]| head.starts_with(s);
    let mut id = Identity::default();
    if starts(b"SEGA SEGASATURN ") {
        id.system = Some("Sega Saturn");
        id.serial = Some(at(0x20, 0x2A)).filter(|s| !s.is_empty());
        id.region = regions(&at(0x40, 0x50));
        id.title = at(0x60, 0xD0);
    } else if starts(b"SEGADISCSYSTEM") || starts(b"SEGABOOTDISC") {
        // The Mega Drive header at 0x100: "GM MK-4407 -00" at 0x180.
        id.system = Some("Sega Mega CD");
        id.serial = Some(at(0x183, 0x18B)).filter(|s| !s.is_empty());
        id.region = regions(&at(0x1F0, 0x1F3));
        id.title = Some(at(0x150, 0x180)).filter(|t| !t.is_empty()).unwrap_or_else(|| at(0x120, 0x150));
    } else if starts(b"PC-FX:Hu_CD-ROM") {
        id.system = Some("PC-FX");
    } else if head.len() > 6 && head[0] == 1 && head[1..6] == [0x5A; 5] {
        // 3DO's Opera filesystem: a record of type 1, five sync bytes.
        id.system = Some("3DO");
        id.label = at(0x28, 0x48);
    } else if head.get(SECTOR_..SECTOR_ + 0x100).is_some_and(|s| s.windows(23).any(|w| w == b"PC Engine CD-ROM SYSTEM")) {
        id.system = Some("PC Engine CD");
    }
    let Some(v) = playstation::volume(read, start) else { return Ok(id) };
    if id.label.is_empty() {
        id.label = v.label.clone();
    }
    id.sectors = Some(v.sectors).filter(|n| *n > 0);
    if id.system.is_none() {
        playstation_or_other(read, &v, &mut id)?;
    }
    Ok(id)
}

const SECTOR_: usize = playstation::SECTOR;

fn playstation_or_other(read: Sectors, v: &Volume, id: &mut Identity) -> std::io::Result<()> {
    if let Some(text) = playstation::read_file(read, v, "SYSTEM.CNF", 64 * 1024)? {
        if let Some(b) = playstation::parse_system_cnf(&String::from_utf8_lossy(&text)) {
            id.system = Some(if b.ps2 { "PlayStation 2" } else { "PlayStation" });
            id.region = playstation::region_of(&b.serial).map(str::to_owned);
            id.serial = Some(b.serial);
            id.boot = Some(b.file);
            return Ok(());
        }
    }
    if playstation::find(read, v, "PS3_DISC.SFB")?.is_some() {
        // A Blu-ray drive reads it; what comes off is encrypted per title.
        id.system = Some("PlayStation 3");
        id.refusal = Some("This is a PlayStation 3 disc. The drive can read it, but what comes off is locked to each game, and this app can't unlock it yet, so the copy wouldn't play.".into());
    } else if playstation::find(read, v, "IPL.TXT")?.is_some() {
        id.system = Some("Neo Geo CD");
    } else if playstation::find(read, v, "VIDEO_TS")?.is_some() {
        id.refusal = Some("This is a DVD-Video disc (it has a VIDEO_TS folder), not a game.".into());
    } else if v.system_id == "PLAYSTATION" {
        id.system = Some("PlayStation");
    }
    Ok(())
}

/// Sega's area symbols as regions: `JTUE` → "Japan, Asia, USA, Europe".
fn regions(symbols: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for c in symbols.chars() {
        let r = match c {
            'J' => "Japan",
            'T' | 'A' => "Asia",
            'U' => "USA",
            'E' => "Europe",
            'B' => "Brazil",
            'K' => "Korea",
            'L' => "Latin America",
            _ => continue,
        };
        if !out.contains(&r) {
            out.push(r);
        }
    }
    (!out.is_empty()).then(|| out.join(", "))
}

// ---- Game disc --------------------------------------------------------------------------------

pub struct GameDisc;

/// Writes what Game disc found into the disc's context, under the names
/// Data disc uses for what they share, and gives the tile's two lines.
pub fn record(c: &mut Context, id: &Identity, found: &media::Found) -> (String, String, String) {
    {
        let d = "the disc";
        let label = if id.label.is_empty() { found.label.clone() } else { id.label.clone() };
        c.add(Subject::Disc, "label", label, d);
        c.add(Subject::Disc, "media", found.media.clone(), d);
        c.add(Subject::Disc, "bytes", found.bytes.to_string(), d);
        c.add(Subject::Disc, "parts", "1", d);
        if let Some(s) = id.system {
            c.add(Subject::Disc, "system", s, d);
        }
        if let Some(s) = id.slug() {
            c.add(Subject::Disc, "catalogue", s, d);
        }
        for (k, v) in [("serial", &id.serial), ("region", &id.region), ("boot", &id.boot)] {
            if let Some(v) = v {
                c.add(Subject::Disc, k, v.clone(), d);
            }
        }
        if !id.title.is_empty() {
            c.add(Subject::Disc, "title", id.title.clone(), d);
        }
        if let Some(n) = id.sectors {
            c.add(Subject::Disc, "sectors", n.to_string(), d);
        }
    }
    // The serial, the system beside it (it decides which catalogues
    // apply), the region as the one fact.
    let label = if id.label.is_empty() { found.label.clone() } else { id.label.clone() };
    let outcome = id.serial.clone().unwrap_or_else(|| if label.is_empty() { "No serial".into() } else { label.clone() });
    let system = id.system.map_or_else(|| "unknown system".to_owned(), str::to_owned);
    let detail = [id.region.clone().unwrap_or_default(), if id.title.is_empty() { String::new() } else { id.title.clone() }].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    (outcome, system, detail)
}

/// The disc in `device`, read through the block device in 2048-byte sectors.
fn identify_device(device: &str) -> std::io::Result<Identity> {
    use std::io::{Read, Seek, SeekFrom};
    let read = |lba: u64, n: usize| -> std::io::Result<Vec<u8>> {
        let mut f = std::fs::File::open(device)?;
        f.seek(SeekFrom::Start(lba * SECTOR_ as u64))?;
        let mut buf = vec![0u8; n * SECTOR_];
        f.read_exact(&mut buf)?;
        Ok(buf)
    };
    // A PC Engine CD starts with an audio track; its header is on the first data one.
    let start = media::toc_starts(device).into_iter().find(|t| t.1).map_or(0, |t| t.2 as u64);
    // A drive still settling after the disc went in fails a read or two.
    let mut last = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        match identify(&read, start) {
            Ok(id) => return Ok(id),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap())
}

#[async_trait]
impl Step for GameDisc {
    fn info(&self) -> StepInfo {
        game_disc_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(disc) = inlet.recv().await {
            let dev = super::device(disc.context())?;
            cx.status("Reading", "the disc");
            let (found, id) = {
                let _drive = cx.lane(LaneKind::Drive).await;
                tokio::task::spawn_blocking(move || {
                    let found = media::read(&dev);
                    identify_device(&dev).map(|id| (found, id))
                })
                .await?
                .map_err(|e| anyhow!("couldn't read the disc's directory: {e}"))?
            };
            if found.bytes == 0 {
                bail!("the disc has no size: is it in?");
            }
            let mut id = id;
            if found.tracks > 0 && found.audio_tracks == found.tracks {
                id.refusal = Some("This is an audio CD: every track is audio. The CDs recipe rips it.".into());
            }
            if let Some(why) = &id.refusal {
                cx.state(StepState::Bad);
                cx.status(id.headline(), why.clone());
                bail!("{why}");
            }
            let mut d = disc.becomes(ItemKind::GameDisc, None);
            let (status, system, detail) = record(d.context_mut(), &id, &found);
            cx.log(format!("{status} ({system}): {:?}, {} of {}, boot {:?}, {:?} sectors", id.label, found.media, media::size(found.bytes), id.boot, id.sectors));
            cx.outcome(status, system, detail);
            d.label = if id.label.is_empty() { id.serial.clone().unwrap_or_else(|| "Game disc".into()) } else { id.label.clone() };
            outlet.send(d).await;
        }
        Ok(())
    }
}

// ---- Rip --------------------------------------------------------------------------------------

/// redumper's full dump, with Re-read bad parts and Make the image anyway: Image's code, run
/// under this step's own settings.
pub struct Rip;

#[async_trait]
impl Step for Rip {
    fn info(&self) -> StepInfo {
        rip_info()
    }

    async fn run(&self, inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        data::Image.run(inlet, outlet, cx).await
    }
}

/// The systems Check looks a dump up in: for a game disc whose system is
/// known, that system's catalogue alone, rather than every PC-readable one
/// (the PC catalogue is tens of megabytes).
pub fn catalogues(c: &Context, choice: &str, media: &str) -> Vec<redump::System> {
    let all = redump::systems_for(choice, media);
    let known = c.get(Subject::Disc, "catalogue").map(str::to_owned);
    match known {
        Some(slug) if choice == redump::ALL || choice.is_empty() => redump::SYSTEMS.iter().filter(|s| s.slug == slug).copied().collect(),
        _ => all,
    }
}

// ---- a dump Redump disagrees with ----------------------------------------------------------------

/// Whether a dump is suspect: Redump has dumps of its serial, and its
/// hashes match none of them. A damaged disc, a drive that misreads it, or
/// a pressing nobody has catalogued; the first two are worth a re-read. A
/// serial Redump has nothing for says nothing about the dump.
pub fn is_suspect(cat: &redump::Catalogue, serial: &str, roms: &[redump::Rom]) -> bool {
    !roms.is_empty() && cat.by_hashes(roms).is_none() && !cat.by_serial(serial).is_empty()
}

/// What Check can do with a dump it holds, besides keeping or skipping it:
/// the real disc, or a simulated one.
#[async_trait]
pub trait Reread: Send + Sync {
    /// Re-reads what the drive reported bad, keeps the new hashes on the
    /// dump, and gives Redump's name for it if it now matches.
    async fn refine(&self, cx: &Cx, dump: &mut trip_engine::Item) -> Result<Option<String>>;
    /// Whether ripping again and combining, or fixing bad sectors, can
    /// help this dump: a CD image whose sectors have checks of their own.
    fn can_combine(&self, dump: &trip_engine::Item) -> bool;
    /// Reads the dump's sectors that fail their own check again from the
    /// disc, up to `tries` times each, and puts each copy that passes its
    /// checks in place in the dump's own files, keeping them as they were
    /// until [`Reread::settled`]. Gives what that came to and the files'
    /// checksums, hashed again from disk.
    async fn fix(&self, cx: &Cx, dump: &trip_engine::Item, tries: u32) -> Result<(Fixing, Vec<redump::Rom>)>;
    /// The dump is verified: the copy of its files as they were before
    /// fixing can go.
    async fn settled(&self, cx: &Cx, dump: &trip_engine::Item) -> Result<()>;
    /// Rips the disc once more and combines every rip so far, sector by
    /// sector. Gives what that came to and the combined copy's checksums,
    /// under the dump's own file names. The dump's files stay as they are.
    async fn combine(&self, cx: &Cx, dump: &trip_engine::Item) -> Result<(Combination, Vec<redump::Rom>)>;
    /// Puts the last combination in the place of the dump's own files, so
    /// its cue sheet and everything made from it use it.
    async fn adopt(&self, cx: &Cx, dump: &trip_engine::Item) -> Result<()>;
    /// The dump's own files hashed again, from disk, as they are now.
    async fn rehash(&self, cx: &Cx, dump: &trip_engine::Item) -> Result<Vec<redump::Rom>>;
    /// Repairs the dump's sectors that still fail their own check by the
    /// disc's own error correction, as `which` says, in the dump's own
    /// files (the last combination put in their place first, if there's
    /// one), keeping them as they were until [`Reread::settled`]. What
    /// fixing found of each sector's re-reads (`fixed`) and every rip so
    /// far say which may be pressed so. Gives what that came to and the
    /// files' checksums, hashed again from disk.
    async fn repair(&self, cx: &Cx, dump: &trip_engine::Item, fixed: Option<&Fixing>, which: Ecc) -> Result<(Repairing, Vec<redump::Rom>)>;
}

/// Which sectors [`Reread::repair`] puts right by their error correction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ecc {
    /// Those read wrong: reads of them gave back different bytes. Gives
    /// what it found of every bad sector.
    ReadWrong,
    /// Those too that may be pressed so (every read the same, or read only
    /// once), each kept aside as read first. Gives how many it repaired.
    MaybePressed,
    /// Puts those back as read. Gives how many it put back.
    PutBack,
}

/// The choices for Check's *If it doesn't match Redump*.
pub const BY_ITSELF: &str = "fix";
pub const ASK: &str = "ask";
pub const IF_NO_MATCH: &[&str] = &[BY_ITSELF, ASK];
pub const IF_NO_MATCH_LABELS: &[&str] = &["Try to fix it by itself", "Ask me"];
/// Extra rips Check makes by itself, unless its settings say otherwise.
pub const EXTRA_RIPS: u32 = 3;

/// What Check does by itself with a dump that doesn't match Redump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Fix bad sectors, then rip again and combine, before asking.
    pub by_itself: bool,
    /// Tries for each bad sector.
    pub tries: u32,
    /// Extra rips at most.
    pub rips: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { by_itself: true, tries: super::fix::TRIES, rips: EXTRA_RIPS }
    }
}

impl Policy {
    /// Check's settings.
    pub fn of(cx: &Cx) -> Policy {
        let d = Policy::default();
        let n = |k: &str, or: u32| cx.param(k).and_then(|v| v.as_u64()).map_or(or, |v| v as u32);
        Policy { by_itself: cx.text("if_no_match") != ASK, tries: n("fix_tries", d.tries).max(1), rips: n("extra_rips", d.rips) }
    }
}

/// What Check does next by itself: fix bad sectors, if it can; then, for a
/// disc Redump has, rip again and combine, up to the policy's rips; then,
/// last, repair by the disc's error correction. `None` when that's all
/// done, or a different disc seems to be in the drive: then it asks.
fn next_by_itself(options: &[(String, String)], fixed: Option<&Fixing>, rips: u32, policy: &Policy, listed: bool) -> Option<&'static str> {
    let offers = |id: &str| options.iter().any(|o| o.0 == id);
    if fixed.is_some_and(|f| f.other_disc) {
        return None;
    }
    if offers("fix") {
        return Some("fix");
    }
    if listed && offers("combine") && rips < policy.rips {
        return Some("combine");
    }
    offers("repair").then_some("repair")
}

/// What `id` is called in the log when Check does it by itself.
fn by_itself_words(id: &str) -> &'static str {
    match id {
        "fix" => "Fix bad sectors",
        "combine" => "Rip again and combine",
        _ => "Repair with the disc's error correction",
    }
}

/// Puts the dump's files' checksums, as hashed from disk, on the dump.
fn note_hashes(dump: &mut trip_engine::Item, roms: &[redump::Rom]) {
    for r in roms {
        dump.context_mut().replace(Subject::Disc, &format!("sha1 {}", r.name), r.sha1.clone(), "Check");
    }
}

/// Redump's entry the dump's files, hashed from disk, are on all four
/// counts, if any; a SHA-1 match that fails the others is logged.
fn matched<'a>(cx: &Cx, cat: Option<&'a redump::Catalogue>, roms: &[redump::Rom]) -> Option<&'a redump::Game> {
    let entry = cat?.by_hashes(roms)?;
    match compare(cx, entry, roms) {
        Ok(()) => Some(entry),
        Err(why) => {
            cx.log(format!("not verified: {why}"));
            None
        }
    }
}

/// Repairs by the disc's error correction: first the sectors read wrong;
/// then, when that's not Redump's and Redump has the disc, those that may
/// be pressed so too, put back as read unless that makes it Redump's. The
/// dump's hashes are the files' as left. Gives what it came to and
/// Redump's entry, if it's now that.
async fn repair_all<'a>(
    cx: &Cx,
    reread: &dyn Reread,
    dump: &mut trip_engine::Item,
    fixed: Option<&Fixing>,
    cat: Option<&'a redump::Catalogue>,
    listed: bool,
) -> Result<(Repairing, Option<&'a redump::Game>)> {
    let (mut r, roms) = reread.repair(cx, dump, fixed, Ecc::ReadWrong).await?;
    note_hashes(dump, &roms);
    let mut entry = matched(cx, cat, &roms);
    if entry.is_none() && listed && r.held_back > 0 {
        let (more, roms) = match reread.repair(cx, dump, fixed, Ecc::MaybePressed).await {
            Ok(done) => done,
            // Stopped or failed partway: put them back as read before saying so.
            Err(e) => {
                if let Err(back) = reread.repair(cx, dump, fixed, Ecc::PutBack).await {
                    cx.log(format!("error correction: putting back the sectors that may be pressed so: {back}"));
                }
                return Err(e);
            }
        };
        note_hashes(dump, &roms);
        entry = matched(cx, cat, &roms);
        if entry.is_some() {
            cx.log(format!("error correction: with {} that may be pressed so repaired too, it matches Redump: they were read wrong after all", mismatch::sectors(more.repaired)));
            r.repaired += more.repaired;
            r.held_back = r.held_back.saturating_sub(more.repaired);
        } else {
            let (_, roms) = reread.repair(cx, dump, fixed, Ecc::PutBack).await?;
            note_hashes(dump, &roms);
        }
    }
    r.verified = entry.is_some();
    Ok((r, entry))
}

/// Holds a dump, Bad, until you say: one of the dump's own options (see
/// [`Mismatch::options`]: Re-read bad parts, Fix bad sectors, Rip again and
/// combine, Keep it anyway, Skip this disc). With `policy.by_itself`, it
/// first fixes bad sectors and then, for a disc Redump has, rips again and
/// combines up to `policy.rips` times, stopping at a match, before asking.
/// A fix, a re-read or a combination whose checksums are in `cat` goes on,
/// verified, once the dump's files are hashed again from disk; one that
/// isn't stays held, saying what it came to, and Keep then keeps the best
/// it has, not verified. Never verified without Redump's checksums, and
/// never kept without you. `explain` says why it's held, for the card and
/// the sidebar, again after each try. Filing waits meanwhile. Gives the
/// dump to send on, if any.
pub async fn hold(
    cx: &Cx,
    mut dump: trip_engine::Item,
    serial: &str,
    reread: &dyn Reread,
    cat: Option<&redump::Catalogue>,
    policy: Policy,
    explain: impl Fn(&trip_engine::Item) -> Option<Mismatch>,
) -> Result<Option<trip_engine::Item>> {
    let listed = cat.is_some_and(|c| !c.by_serial(serial).is_empty());
    let bad = if listed { format!("Doesn't match Redump's dumps of {serial}") } else { "Not in Redump, and some sectors fail their own check".to_owned() };
    cx.log(&bad);
    let mut combined: Option<Combination> = None;
    let mut fixed: Option<Fixing> = None;
    let mut repaired: Option<Repairing> = None;
    let mut trouble: Option<String> = None;
    let mut by_itself = policy.by_itself;
    let mut own_rips = 0u32;
    let mut tried: Vec<String> = vec![];
    let total_rips = 1 + policy.rips as usize;
    // Every rip again, timed together.
    let mut combine_secs = 0u64;
    // The first read's own counts, in the drive's and redumper's units:
    // for the log, not the sidebar.
    if let Some(r) = explain(&dump).and_then(|m| m.read) {
        let kinds: Vec<String> = r.sectors.iter().map(|s| format!("{} {}", s.kind.code(), s.count)).collect();
        let kinds = if kinds.is_empty() { String::new() } else { format!("; sectors {}", kinds.join(", ")) };
        cx.log(format!("the rip's log: SCSI {} and C2 {} samples{kinds}", r.scsi, r.c2));
    }
    loop {
        let why = explain(&dump).map(|m| m.with(reread.can_combine(&dump), combined.clone()).fixing(fixed.clone()).repairing(repaired.clone()));
        if let Some(m) = &why {
            cx.log(format!("compared with {}: {}", if m.in_redump() { m.entry.as_str() } else { "nothing in Redump" }, m.short()));
            for line in &m.reading {
                cx.log(format!("  {line}"));
            }
        }
        cx.report(why.as_ref().and_then(|m| serde_json::to_value(m).ok()));
        let head = if listed { "Doesn't match Redump" } else { "Not verified" };
        let options = why.as_ref().map(Mismatch::options).unwrap_or_default();
        let planned = if by_itself { next_by_itself(&options, fixed.as_ref(), own_rips, &policy, listed) } else { None };
        let option = match planned {
            Some(o) => {
                cx.log(format!("by itself, as Check's settings say: {}", by_itself_words(o)));
                o.to_owned()
            }
            None => {
                let repairs = repaired.as_ref().filter(|r| r.repaired > 0);
                if by_itself && (!tried.is_empty() || repairs.is_some()) && trouble.is_none() {
                    let not = if listed { "still doesn't match Redump" } else { "not verified: no Redump entry" };
                    let against = if listed { "doesn't match Redump" } else { "no Redump entry" };
                    trouble = Some(match repairs {
                        Some(r) if tried.is_empty() => format!("Not verified: {}; {against}", r.note()),
                        Some(r) => format!("Not verified: {}; tried by itself ({}); {against}", r.note(), tried.join(", ")),
                        None => format!("Tried by itself ({}); {not}", tried.join(", ")),
                    });
                    cx.log(format!("tried by itself ({}); {not}: asking", tried.join(", ")));
                }
                by_itself = false;
                cx.status(head, why.as_ref().map_or_else(|| serial.to_owned(), Mismatch::short));
                cx.state(StepState::Bad);
                // What it offers first, so it's there the moment it's marked bad.
                cx.item_options(&dump, options.clone());
                cx.item(&dump, ItemState::Bad, trouble.take().unwrap_or_else(|| bad.clone()));
                let req = loop {
                    match cx.next_retry().await {
                        None => return Ok(None),
                        // Only what it offered (or anything, when it offered the step's own).
                        Some(r) if r.item == dump.id && (options.is_empty() || options.iter().any(|o| o.0 == r.option)) => break r,
                        Some(_) => {}
                    }
                };
                req.option
            }
        };
        cx.state(StepState::Busy);
        match option.as_str() {
            "leave_out" => {
                cx.item(&dump, ItemState::Skipped, "skipped");
                cx.status("Skipped", serial.to_owned());
                return Ok(None);
            }
            "keep" if combined.is_some() => {
                reread.adopt(cx, &dump).await?;
                let against = if listed { format!("; doesn't match Redump's copies of {serial}") } else { String::new() };
                let with = repaired.as_ref().filter(|r| r.repaired > 0).map(|r| format!(", {}", r.note())).unwrap_or_default();
                cx.item(&dump, ItemState::Ok, format!("kept the best combination{with}, not verified{against}"));
                cx.status("Kept, not verified", mismatch::and_repaired(mismatch::result_note(fixed.as_ref(), combined.as_ref()), repaired.as_ref()));
                return Ok(Some(dump));
            }
            "keep" => {
                let against = if listed { format!("; doesn't match Redump's copies of {serial}") } else { "; not verified".to_owned() };
                let how = match repaired.as_ref().filter(|r| r.repaired > 0) {
                    Some(r) => format!("kept with {}", r.note()),
                    None if fixed.as_ref().is_some_and(|f| f.fixed > 0) => "kept with bad sectors fixed".to_owned(),
                    None => "kept anyway".to_owned(),
                };
                cx.item(&dump, ItemState::Ok, format!("{how}{against}"));
                cx.status("Kept anyway", if listed { "doesn't match Redump" } else { "not verified" });
                return Ok(Some(dump));
            }
            "fix" => {
                cx.item(&dump, ItemState::Busy, "looking for bad sectors");
                cx.status("Looking for bad sectors", serial.to_owned());
                let began = std::time::Instant::now();
                let (mut f, roms) = match reread.fix(cx, &dump, policy.tries).await {
                    Ok(mut done) => {
                        done.0.secs = Some(began.elapsed().as_secs());
                        done
                    }
                    Err(e) if cx.stopped() => return Err(e),
                    Err(e) => {
                        cx.log(format!("fix bad sectors: {e}"));
                        trouble = Some(format!("Couldn't fix bad sectors: {e}"));
                        by_itself = false;
                        // Not tried: it can be again.
                        continue;
                    }
                };
                tried.push(format!("{} re-read", mismatch::sectors(f.fixed)));
                for r in &roms {
                    dump.context_mut().replace(Subject::Disc, &format!("sha1 {}", r.name), r.sha1.clone(), "Check");
                }
                fixed = Some(f.clone());
                // What the re-reads found may settle sectors a repair held back.
                repaired = None;
                if f.fixed == 0 {
                    continue;
                }
                let Some(entry) = cat.and_then(|cat| cat.by_hashes(&roms)) else { continue };
                // Hashed from disk after the fix: verified on all four counts, or not at all.
                if let Err(why) = compare(cx, entry, &roms) {
                    cx.log(format!("not verified: {why}"));
                    trouble = Some("The fixed copy's SHA-1 is Redump's, but its other checksums aren't".into());
                    continue;
                }
                f.verified = true;
                reread.settled(cx, &dump).await?;
                let name = entry.name.clone();
                dump.context_mut().add(Subject::Disc, "name", name.clone(), "Redump");
                if let Some(r) = repaired.as_mut() {
                    r.verified = true;
                }
                let why = explain(&dump).map(|m| m.with(reread.can_combine(&dump), None).fixing(Some(f.clone())).repairing(repaired.clone()));
                cx.report(why.as_ref().and_then(|m| serde_json::to_value(m).ok()));
                let note = mismatch::and_repaired(f.note(), repaired.as_ref());
                cx.verified(&dump, format!("Verified: {name}, {note}"));
                cx.status("Verified", note);
                return Ok(Some(dump));
            }
            "combine" => {
                let n = combined.as_ref().map_or(1, |c| c.rips.len()) + 1;
                let doing = if by_itself {
                    own_rips += 1;
                    format!("Rip {n} of {}", total_rips.max(n))
                } else {
                    format!("Ripping again, rip {n}")
                };
                cx.item(&dump, ItemState::Busy, doing.to_lowercase());
                cx.status(doing, serial.to_owned());
                let began = std::time::Instant::now();
                let (mut c, roms) = match reread.combine(cx, &dump).await {
                    Ok(mut done) => {
                        combine_secs += began.elapsed().as_secs();
                        done.0.secs = Some(combine_secs);
                        done
                    }
                    Err(e) if cx.stopped() => return Err(e),
                    // The rips so far are kept; it can be tried again.
                    Err(e) => {
                        cx.log(format!("rip again: {e}"));
                        trouble = Some(format!("Couldn't rip again: {e}"));
                        by_itself = false;
                        continue;
                    }
                };
                if by_itself {
                    tried.retain(|t| !t.ends_with("more rip") && !t.ends_with("more rips"));
                    tried.push(format!("{own_rips} more rip{}", if own_rips == 1 { "" } else { "s" }));
                }
                for r in &roms {
                    dump.context_mut().replace(Subject::Disc, &format!("sha1 {}", r.name), r.sha1.clone(), "Check");
                }
                cx.log(format!("{}: {} sectors differed, {} with no good copy, {} where good copies disagree, {} still bad", c.note(), c.differing, c.no_good, c.disagree, c.still_bad));
                combined = Some(c.clone());
                // A new combination: what was repaired was another image.
                repaired = None;
                let Some(entry) = cat.and_then(|cat| cat.by_hashes(&roms)) else { continue };
                // Its SHA-1 is Redump's. Verified only once the dump's own
                // files, the combination in their place, are hashed again
                // from disk and match the entry on all four counts.
                reread.adopt(cx, &dump).await?;
                let now = reread.rehash(cx, &dump).await?;
                for r in &now {
                    dump.context_mut().replace(Subject::Disc, &format!("sha1 {}", r.name), r.sha1.clone(), "Check");
                }
                if let Err(why) = compare(cx, entry, &now) {
                    cx.log(format!("not verified: {why}"));
                    trouble = Some("The combination's SHA-1 is Redump's, but its other checksums aren't".into());
                    continue;
                }
                c.verified = true;
                if let Some(f) = fixed.as_mut() {
                    f.verified = true;
                }
                reread.settled(cx, &dump).await?;
                let name = entry.name.clone();
                dump.context_mut().add(Subject::Disc, "name", name.clone(), "Redump");
                let why = explain(&dump).map(|m| m.with(true, Some(c.clone())).fixing(fixed.clone()));
                cx.report(why.as_ref().and_then(|m| serde_json::to_value(m).ok()));
                let note = mismatch::result_note(fixed.as_ref(), Some(&c));
                // Nothing repaired is in this combination: `repaired` is gone.
                cx.verified(&dump, format!("Verified: {name}, {note}"));
                cx.status("Verified", note);
                return Ok(Some(dump));
            }
            "repair" => {
                cx.item(&dump, ItemState::Busy, "repairing by the disc's error correction");
                cx.status("Repairing with the disc's error correction", serial.to_owned());
                // No soft failure: partway, sectors that may be pressed so
                // could be left repaired, so a failure stops Check.
                let began = std::time::Instant::now();
                let (mut r, entry) = repair_all(cx, reread, &mut dump, fixed.as_ref(), cat, listed).await?;
                r.secs = Some(began.elapsed().as_secs());
                if by_itself && r.repaired == 0 {
                    tried.push("nothing repaired by error correction".into());
                }
                repaired = Some(r.clone());
                let Some(entry) = entry else {
                    if !by_itself {
                        trouble = Some(if r.repaired > 0 { format!("Not verified: {}", r.note()) } else { "Not verified: the disc's error correction repaired nothing".to_owned() });
                    }
                    continue;
                };
                // The copy is Redump's with the repairs: what came before
                // them, a fix or a combination, wasn't by itself.
                reread.settled(cx, &dump).await?;
                let name = entry.name.clone();
                dump.context_mut().add(Subject::Disc, "name", name.clone(), "Redump");
                let why = explain(&dump).map(|m| m.with(reread.can_combine(&dump), combined.clone()).fixing(fixed.clone()).repairing(repaired.clone()));
                cx.report(why.as_ref().and_then(|m| serde_json::to_value(m).ok()));
                let note = mismatch::and_repaired(mismatch::result_note(fixed.as_ref(), combined.as_ref()), Some(&r));
                cx.verified(&dump, format!("Verified: {name}, {note}"));
                cx.status("Verified", note);
                return Ok(Some(dump));
            }
            _ => {
                cx.item(&dump, ItemState::Busy, "re-reading bad parts");
                cx.status("Re-reading bad parts", serial.to_owned());
                match reread.refine(cx, &mut dump).await {
                    Ok(Some(name)) => {
                        cx.report(None);
                        cx.verified(&dump, format!("Verified: {name}"));
                        cx.status("Verified", "after re-reading bad parts");
                        return Ok(Some(dump));
                    }
                    Ok(None) => cx.log("refined; still doesn't match"),
                    Err(e) if cx.stopped() => return Err(e),
                    Err(e) => {
                        cx.log(format!("re-read: {e}"));
                        trouble = Some(format!("Couldn't re-read: {e}"));
                    }
                }
            }
        }
    }
}

/// The dump's files, hashed from disk, against Redump's entry on size,
/// CRC-32, MD5 and SHA-1, each comparison logged.
fn compare(cx: &Cx, entry: &redump::Game, roms: &[redump::Rom]) -> std::result::Result<(), String> {
    let lines = redump::same_files(entry, roms)?;
    for l in lines {
        cx.log(format!("checked {l} (hashed from disk with the crc32fast, md-5 and sha1 crates)"));
    }
    Ok(())
}

/// Check's hold for a game or a data disc that doesn't match Redump: held
/// when Redump has dumps of its serial and this is none of them, or, with
/// every list reached and none having it, when its own sectors say it was
/// read wrong. Gives the dump to send on, if any. A catalogue that can't
/// be had isn't a reason to hold anything.
pub async fn hold_if_suspect(cx: &Cx, dump: trip_engine::Item, roms: &[redump::Rom], reached_all: bool, reread: &dyn Reread) -> Result<Option<trip_engine::Item>> {
    let (system, serial, _) = known(dump.context());
    let Some(main) = dump.path.clone() else { return Ok(Some(dump)) };
    let log = |main: &Path| std::fs::read_to_string(main.with_extension("log")).ok();
    let cat = match system.as_deref().and_then(slug_of) {
        Some(slug) => game_catalogue(&data::redump_cache(&cx.work()?), slug).await.ok(),
        None => None,
    };
    let serial_s = serial.clone().unwrap_or_default();
    if let (Some(cat), Some(serial)) = (&cat, &serial) {
        if is_suspect(cat, serial, roms) {
            // Redump's copies beside the dump's files as they are now (a
            // re-read or a combination changes them), and what redumper's
            // log says about the first read.
            let explain = |d: &trip_engine::Item| {
                let main = d.path.as_deref()?;
                mismatch::explain(serial, &cat.by_serial(serial), &hashed(d.context(), main), log(main).as_deref())
            };
            return hold(cx, dump, serial, reread, Some(cat), Policy::of(cx), explain).await;
        }
    }
    // Nothing in Redump to compare with: held only when it can be helped,
    // and its own sectors say it was read wrong.
    let wrong = log(&main).map(|l| mismatch::read_log(&l)).is_some_and(|r| r.bad().max(r.wrong()) > 0);
    // Made with sectors Rip couldn't read: never let through unchecked,
    // whatever could or couldn't be looked up.
    let unreadable = dump.context().get(Subject::Disc, data::UNREADABLE).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0) > 0;
    if !unreadable && !(reached_all && wrong && reread.can_combine(&dump)) {
        return Ok(Some(dump));
    }
    let explain = |d: &trip_engine::Item| {
        let main = d.path.as_deref()?;
        Some(mismatch::unlisted(&serial_s, &hashed(d.context(), main), &log(main)?))
    };
    hold(cx, dump, &serial_s, reread, cat.as_deref(), Policy::of(cx), explain).await
}

/// A rip of the whole disc into a folder, giving its cue sheet: redumper's
/// in the app ([`data::rip_again`]), a stand-in in a test.
pub type RipFn = std::sync::Arc<dyn Fn(Cx, trip_engine::Item, PathBuf) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PathBuf>> + Send>> + Send + Sync>;

/// redumper's rip, as Rip reads.
pub fn redumper_rip() -> RipFn {
    std::sync::Arc::new(|cx, dump, dir| Box::pin(async move { data::rip_again(&cx, &dump, dir).await }))
}

/// The real disc: redumper's refine, and rips into the run's working
/// folder (`rip-1` the first rip's files, `rip-2` on), combined into
/// `combined`.
pub struct Disc {
    pub rip: RipFn,
    pub source: SourceFn,
    /// What fixing and repairing found of each sector that failed its own
    /// check, for the map.
    pub marks: std::sync::Mutex<BTreeMap<u64, crate::map::Mark>>,
}

/// Where Fix bad sectors reads from, for a dump: the drive it was ripped
/// on ([`drive_source`]), a stand-in in a test.
pub type SourceFn = std::sync::Arc<dyn Fn(&trip_engine::Item) -> Result<Box<dyn super::fix::Source>> + Send + Sync>;

/// The drive the dump was ripped on, with its disc in.
pub fn drive_source() -> SourceFn {
    std::sync::Arc::new(|dump: &trip_engine::Item| {
        let dev = super::device(dump.context())?;
        if !media::disc_in(&dev) {
            bail!("there's no disc in the drive");
        }
        let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
        let sectors = super::combine::layout(&main)?.iter().map(|t| (t.size / super::combine::SECTOR as u64) as i64).sum();
        Ok(Box::new(super::fix::Drive { device: dev, sectors }) as Box<dyn super::fix::Source>)
    })
}

impl Disc {
    /// The real disc, read again through `rip` and `source`.
    pub fn new(rip: RipFn, source: SourceFn) -> Disc {
        Disc { rip, source, marks: Default::default() }
    }

    fn folders(cx: &Cx) -> Result<PathBuf> {
        cx.work()
    }

    /// The map of the disc as fixing and repairing left it: its sectors as
    /// last read from the dump's own `.state` when it's there, with each
    /// that failed its checks over it, put right or not; else the checks
    /// alone.
    async fn show_map(&self, cx: &Cx, dump: &trip_engine::Item, main: &Path) {
        let map = self.map_now(dump, main).await;
        cx.map(serde_json::to_value(map).ok());
    }

    async fn map_now(&self, dump: &trip_engine::Item, main: &Path) -> crate::map::SectorMap {
        let found: Vec<(u64, crate::map::Mark)> = self.marks.lock().map(|m| m.iter().map(|(k, v)| (*k, *v)).collect()).unwrap_or_default();
        let sectors = super::combine::layout(main).map(|ts| ts.iter().map(|t| (t.start.max(0) as u64) + t.size / super::combine::SECTOR as u64).max().unwrap_or(0)).unwrap_or(0);
        let sectors = sectors.max(found.iter().map(|f| f.0 + 1).max().unwrap_or(0));
        let t = super::redumper::Target { dir: main.parent().unwrap_or(Path::new(".")).to_path_buf(), name: main.file_stem().unwrap_or_default().to_string_lossy().into_owned() };
        let offset = data::read_offset(dump.context()).unwrap_or(0);
        let read = tokio::task::spawn_blocking(move || {
            let mut s = super::redumper::StateScan::new(&t, sectors, offset, true);
            s.read_to(sectors, sectors, true).ok().map(|_| s.map(false))
        })
        .await
        .ok()
        .flatten();
        match read {
            Some(mut m) => {
                m.overlay(&found);
                m
            }
            None => crate::map::SectorMap::checked(sectors, &found, true),
        }
    }

    fn mark(&self, lbas: impl IntoIterator<Item = i64>, m: crate::map::Mark) {
        if let Ok(mut marks) = self.marks.lock() {
            for l in lbas {
                marks.insert(l.max(0) as u64, m);
            }
        }
    }
}

/// What Check and fix is doing while it reads bad sectors again, said the
/// same from the drive and in the simulation: the try it's on for the
/// window ([`Cx::now`]), how far through for the ring, and a line for the
/// card.
pub fn say_rereading(cx: &Cx, dump: &trip_engine::Item, r: &super::fix::Rereading) {
    cx.now(serde_json::to_value(r).ok());
    cx.progress(Some(r.fraction()));
    cx.item(dump, ItemState::Busy, r.note());
    cx.status(REREADING, r.card());
}

/// Check and fix's status while it reads bad sectors again.
pub const REREADING: &str = "Reading bad sectors again";

/// The map while bad sectors are read again: each found bad marked so
/// first, each put right as it comes back, and the sector being read
/// marked as where the drive is. Shown anew for each sector, never for
/// each try.
pub struct LiveMap {
    map: crate::map::SectorMap,
}

impl LiveMap {
    pub fn new(map: crate::map::SectorMap) -> LiveMap {
        LiveMap { map }
    }

    /// The bad sectors about to be read again.
    pub fn found(&mut self, cx: &Cx, bad: &[i64]) {
        let marks: Vec<(u64, crate::map::Mark)> = bad.iter().map(|l| ((*l).max(0) as u64, crate::map::Mark::Wrong)).collect();
        self.map.overlay(&marks);
        self.show(cx);
    }

    /// The drive is on `lba` now.
    pub fn at(&mut self, cx: &Cx, lba: i64) {
        self.map.at = Some(lba.max(0) as u64);
        self.show(cx);
    }

    /// What came of a sector: shown when it came back.
    pub fn done(&mut self, cx: &Cx, s: &super::fix::Sector) {
        if matches!(s.outcome, super::fix::Outcome::Fixed { .. }) {
            self.map.overlay(&[(s.lba.max(0) as u64, crate::map::Mark::Recovered)]);
            self.show(cx);
        }
    }

    fn show(&self, cx: &Cx) {
        cx.map(serde_json::to_value(&self.map).ok());
    }
}

/// Links (or, failing that, copies) `files` into `dir`.
fn keep_copy(files: &[PathBuf], dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for f in files {
        let to = dir.join(f.file_name().unwrap_or_default());
        if !to.exists() && std::fs::hard_link(f, &to).is_err() {
            std::fs::copy(f, &to)?;
        }
    }
    Ok(())
}

#[async_trait]
impl Reread for Disc {
    async fn refine(&self, cx: &Cx, dump: &mut trip_engine::Item) -> Result<Option<String>> {
        refine(cx, dump).await
    }

    fn can_combine(&self, dump: &trip_engine::Item) -> bool {
        data::combinable(dump)
    }

    async fn fix(&self, cx: &Cx, dump: &trip_engine::Item, tries: u32) -> Result<(Fixing, Vec<redump::Rom>)> {
        use super::fix;
        let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
        let keep = Self::folders(cx)?.join("before-fix");
        let mut source = (self.source)(dump)?;
        let base = self.map_now(dump, &main).await;
        let fixed = {
            let _drive = cx.lane(LaneKind::Drive).await;
            let (cx2, d, cue) = (cx.clone(), dump.clone(), main.clone());
            let done = tokio::task::spawn_blocking(move || {
                let mut live = LiveMap::new(base);
                let mut looked = None;
                fix::fix(
                    &cue,
                    &mut *source,
                    tries,
                    &keep,
                    |e| match e {
                        fix::Event::Looking(p) => {
                            cx2.progress(Some(p));
                            let pc = (p * 100.0) as u32;
                            if looked != Some(pc) {
                                looked = Some(pc);
                                cx2.item(&d, ItemState::Busy, format!("looking for bad sectors, {pc}%"));
                            }
                        }
                        fix::Event::Found(bad) => live.found(&cx2, bad),
                        fix::Event::Try(r) => {
                            say_rereading(&cx2, &d, r);
                            if r.attempt == 1 {
                                live.at(&cx2, r.at);
                            }
                        }
                        fix::Event::Done(s, r) => {
                            cx2.log(fix::sector_line(s, tries));
                            if r.given_up {
                                cx2.log(format!("fix bad sectors: {}", fix::STOPPED));
                            }
                            live.done(&cx2, s);
                        }
                    },
                    &|| cx2.stopped(),
                )
            })
            .await?;
            cx.drive_says(StepState::Idle, "Waiting", "");
            cx.now(None);
            cx.progress(None);
            match done {
                Ok(f) => f,
                Err(e) => {
                    // Nothing read in this pass went in: the map as it was.
                    self.show_map(cx, dump, &main).await;
                    return Err(e.into());
                }
            }
        };
        for l in fix::log_lines(&fixed, tries) {
            cx.log(l);
        }
        if fixed.other_disc.is_none() {
            self.mark(fixed.fixed().map(|s| s.lba), crate::map::Mark::Recovered);
            self.mark(fixed.left().map(|s| s.lba), crate::map::Mark::Wrong);
            // With where the drive was gone from it.
            self.show_map(cx, dump, &main).await;
        }
        let roms = self.rehash(cx, dump).await?;
        for r in &roms {
            cx.log(format!("after fixing: <rom name=\"{}\" size=\"{}\" crc=\"{}\" md5=\"{}\" sha1=\"{}\" />", r.name, r.size, r.crc, r.md5, r.sha1));
        }
        Ok((Fixing::of(&fixed, tries), roms))
    }

    async fn settled(&self, cx: &Cx, _dump: &trip_engine::Item) -> Result<()> {
        let keep = Self::folders(cx)?.join("before-fix");
        if keep.exists() {
            std::fs::remove_dir_all(&keep)?;
            cx.log("verified: the copy of the files as they were before fixing is gone");
        }
        let _ = std::fs::remove_file(Self::folders(cx)?.join("maybe-pressed.bin"));
        Ok(())
    }

    async fn combine(&self, cx: &Cx, dump: &trip_engine::Item) -> Result<(Combination, Vec<redump::Rom>)> {
        use super::{combine, hash, redumper};
        let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
        let work = Self::folders(cx)?;
        // The first rip, as it was: linked, so it costs no room, and stays
        // when the dump's own files are replaced.
        keep_copy(&redumper::dump_files(&main), &work.join("rip-1"))?;
        let mut rips: Vec<PathBuf> = (1..).map(|n| work.join(format!("rip-{n}")).join(main.file_name().unwrap_or_default())).take_while(|c| c.is_file()).collect();
        let dir = work.join(format!("rip-{}", rips.len() + 1));
        let _ = std::fs::remove_dir_all(&dir);
        let cue = (self.rip)(cx.clone(), dump.clone(), dir).await?;
        rips.push(cue);
        cx.item(dump, ItemState::Busy, format!("combining {} rips", rips.len()));
        cx.status(format!("Combining {} rips", rips.len()), "");
        let out = work.join("combined");
        let (cx2, d) = (cx.clone(), dump.clone());
        let done = tokio::task::spawn_blocking(move || -> Result<(combine::Combined, Vec<redump::Rom>)> {
            let c = combine::combine(
                &rips,
                &out,
                |p| {
                    cx2.progress(Some(p));
                    cx2.item(&d, ItemState::Busy, format!("combining {}%", (p * 100.0) as u32));
                },
                &|| cx2.stopped(),
            )?;
            let files = data::data_files(&c.cue);
            let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
            let roms = hash::hash_files(&refs, |_| {}, || cx2.stopped())?;
            Ok((c, roms))
        })
        .await??;
        cx.progress(None);
        let mut by_rip: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (lba, k) in &done.0.tally.from {
            by_rip.entry(done.0.used.get(*k).copied().unwrap_or(0)).or_default().push(lba.to_string());
        }
        for (n, lbas) in by_rip {
            cx.log(format!("combined: from rip {n}: {} sector{} ({})", lbas.len(), if lbas.len() == 1 { "" } else { "s" }, lbas.join(", ")));
        }
        Ok((Combination::of(&done.0, false), done.1))
    }

    async fn adopt(&self, cx: &Cx, dump: &trip_engine::Item) -> Result<()> {
        let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
        let from = Self::folders(cx)?.join("combined");
        for f in data::image_files(&main) {
            let name = f.file_name().unwrap_or_default();
            let src = from.join(name);
            // Already in place (kept after a check that didn't verify it).
            if !src.is_file() {
                continue;
            }
            if std::fs::rename(&src, &f).is_err() {
                std::fs::copy(&src, &f)?;
            }
        }
        cx.log(format!("the combined image is now {}", main.display()));
        Ok(())
    }

    async fn rehash(&self, cx: &Cx, dump: &trip_engine::Item) -> Result<Vec<redump::Rom>> {
        let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
        let files = data::data_files(&main);
        let cx2 = cx.clone();
        tokio::task::spawn_blocking(move || {
            let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
            super::hash::hash_files(&refs, |_| {}, || cx2.stopped())
        })
        .await?
    }

    async fn repair(&self, cx: &Cx, dump: &trip_engine::Item, fixed: Option<&Fixing>, which: Ecc) -> Result<(Repairing, Vec<redump::Rom>)> {
        use super::ecc;
        use crate::map::Mark;
        let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
        let work = Self::folders(cx)?;
        let name = main.file_name().unwrap_or_default().to_owned();
        // The best copy there is: the last combination, if it's not in place yet.
        if which == Ecc::ReadWrong && work.join("combined").join(&name).is_file() {
            self.adopt(cx, dump).await?;
        }
        let keep = work.join("before-fix");
        let aside = work.join("maybe-pressed.bin");
        let reads = ecc::Reads {
            rips: (1..).map(|n| work.join(format!("rip-{n}")).join(&name)).take_while(|c| c.is_file()).collect(),
            read_alike: fixed.map(|f| f.read_alike.clone()).unwrap_or_default(),
            read_differently: fixed.map(|f| f.read_differently.clone()).unwrap_or_default(),
        };
        let (cx2, m) = (cx.clone(), main.clone());
        let (r, lines, done, left) = tokio::task::spawn_blocking(move || -> std::io::Result<(Repairing, Vec<String>, Vec<i64>, Vec<i64>)> {
            if which == Ecc::PutBack {
                let back = ecc::put_back(&m, &aside, &keep)?;
                let line = format!(
                    "error correction: put back as read, since the image with them repaired isn't Redump's: {} ({})",
                    mismatch::sectors(back.len() as u64),
                    back.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
                );
                return Ok((Repairing { repaired: back.len() as u64, ..Repairing::default() }, vec![line], vec![], back));
            }
            let found = ecc::look(&m, &reads, |p| cx2.progress(Some(p)), &|| cx2.stopped())?;
            let pressed_too = which == Ecc::MaybePressed;
            let take: Vec<&ecc::Found> = found.iter().filter(|f| f.repair.is_ok() && f.maybe_pressed == pressed_too).collect();
            if pressed_too && !take.is_empty() {
                ecc::keep_aside(&take, &aside)?;
            }
            let done = ecc::apply(&take, &keep)?;
            let left = found.iter().map(ecc::Found::lba).filter(|l| !done.contains(l)).collect();
            let mut r = ecc::summary(&found);
            if pressed_too {
                r = Repairing { repaired: done.len() as u64, ..Repairing::default() };
            }
            Ok((r, ecc::log_lines(&found, pressed_too), done, left))
        })
        .await??;
        cx.progress(None);
        for l in lines {
            cx.log(l);
        }
        self.mark(done, Mark::Repaired);
        self.mark(left, Mark::Wrong);
        self.show_map(cx, dump, &main).await;
        let roms = self.rehash(cx, dump).await?;
        for r in &roms {
            cx.log(format!("after error correction: <rom name=\"{}\" size=\"{}\" crc=\"{}\" md5=\"{}\" sha1=\"{}\" />", r.name, r.size, r.crc, r.md5, r.sha1));
        }
        Ok((r, roms))
    }
}

/// redumper's refine on the dump, a fresh split and check of it, then its
/// hashes again and Redump's name if they now match.
async fn refine(cx: &Cx, dump: &mut trip_engine::Item) -> Result<Option<String>> {
    use super::{hash, redumper};
    let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
    let dir = main.parent().ok_or_else(|| anyhow!("{} has no folder", main.display()))?.to_path_buf();
    let t = redumper::Target { dir, name: main.file_stem().unwrap_or_default().to_string_lossy().into_owned() };
    // At the offset the dump was read at: redumper places what it reads by it.
    let opts = data::rip_options(dump.context())?;
    {
        let _drive = cx.lane(LaneKind::Drive).await;
        let mut out = String::new();
        data::Image.phase(cx, dump, &t, redumper::refine_args(&opts, &t), &mut out).await?;
        if let Err(bad) = data::Image.settle(cx, dump, &t, &mut out, false, true).await? {
            cx.log(format!("refine: {bad}"));
        }
        cx.drive_says(StepState::Idle, "Waiting", "");
    }
    let files = data::data_files(&main);
    let roms = tokio::task::spawn_blocking(move || {
        let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
        hash::hash_files(&refs, |_| {}, || false)
    })
    .await??;
    // What CHD verifies against, and Redump looks up, is the new read.
    for r in &roms {
        dump.context_mut().replace(Subject::Disc, &format!("sha1 {}", r.name), r.sha1.clone(), "Check");
    }
    let (system, _, _) = known(dump.context());
    let Some((system, slug)) = system.and_then(|s| Some((s.clone(), slug_of(&s)?))) else { return Ok(None) };
    let cat = game_catalogue(&data::redump_cache(&cx.work()?), slug).await?;
    Ok(cat.by_hashes(&roms).map(|g| {
        let c = dump.context_mut();
        c.add(Subject::Disc, "name", g.name.clone(), "Redump");
        c.add(Subject::Disc, "system", system, "Redump");
        g.name.clone()
    }))
}

// ---- Redump -------------------------------------------------------------------------------------

/// What Redump says about a disc.
#[derive(Clone, Debug, PartialEq)]
pub struct Lookup {
    pub candidates: Vec<Candidate>,
    pub status: String,
    /// *by serial only*, when that's all it went on.
    pub qualifier: String,
    pub detail: String,
}

/// Score of the entry the dump's hashes match: the identity, for certain.
pub const BY_HASH: f32 = 1.0;
/// Scores of the entries that only share the serial, close in size or not:
/// guesses, which Choose never takes by itself.
pub const BY_SERIAL_CLOSE: f32 = 0.3;
pub const BY_SERIAL: f32 = 0.2;

/// How many sectors the first data track of an entry holds: the track the
/// filesystem describes, so the one measurable before dumping.
pub fn first_track_sectors(g: &redump::Game) -> Option<u64> {
    let rom = g.roms.iter().find(|r| r.is_data())?;
    let per = if rom.name.to_ascii_lowercase().ends_with(".iso") { 2048 } else { 2352 };
    (rom.size > 0 && rom.size % per == 0).then(|| rom.size / per)
}

/// How far a disc's filesystem size may be from an entry's first track and
/// still be it: padding belongs to the track, not the filesystem. Tighter
/// than the gap between two discs of a set, which is tens of thousands.
const SECTOR_SLACK: u64 = 2_000;

pub use crate::template::split_disc;

/// What a game's template can use beyond a dump's: its title and disc from
/// the name, its serial and region from the context (Redump's, once chosen).
pub fn game_fields(c: &Context) -> template::GameFields {
    let mut g = template::GameFields::from_name(&data::dump_fields(c).name);
    g.serial = c.get(Subject::Disc, "serial").unwrap_or("").to_owned();
    g.region = c.get(Subject::Disc, "region").unwrap_or("").to_owned();
    g
}

/// The region Redump files an entry under: its first bracket that is one.
pub fn region_in(name: &str) -> Option<String> {
    const REGIONS: &[&str] = &["USA", "Europe", "Japan", "World", "Korea", "Asia", "Australia", "Brazil", "Canada", "France", "Germany", "Spain", "Italy", "UK"];
    name.split('(').skip(1).filter_map(|g| g.split(')').next()).find(|g| g.split(", ").all(|r| REGIONS.contains(&r))).map(str::to_owned)
}

/// One catalogue entry as a candidate identity.
pub fn candidate(g: &redump::Game, system: &str, score: f32, detail: String) -> Candidate {
    let source = "Redump";
    let mut facts = vec![("name", g.name.clone()), ("system", system.to_owned()), ("title", g.name.split(" (").next().unwrap_or(&g.name).trim().to_owned())];
    if let Some(s) = g.serials.first() {
        facts.push(("serial", s.clone()));
    }
    if let Some(r) = region_in(&g.name) {
        facts.push(("region", r));
    }
    if let Some((set, n)) = split_disc(&g.name) {
        facts.push(("disc", n.to_string()));
        facts.push(("set", set));
    }
    Candidate {
        step: String::new(),
        id: format!("redump:{}", g.name),
        source: source.into(),
        title: g.name.clone(),
        detail,
        score,
        facts: facts.into_iter().map(|(k, v)| Fact { step: String::new(), subject: Subject::Disc, key: k.into(), value: v, source: source.into() }).collect(),
    }
}

/// Looks a disc up in its system's catalogue by its serial, when Check
/// couldn't name it by its checksums; otherwise there's nothing to look
/// up, and the entry Check matched goes on to Choose as it is. The
/// serial's entries come close to its size first. Before the dump (no
/// hashes), the serial's entries.
pub fn lookup(cat: &redump::Catalogue, system: &str, serial: Option<&str>, sectors: Option<u64>, roms: &[redump::Rom]) -> Lookup {
    if let Some(g) = (!roms.is_empty()).then(|| cat.by_hashes(roms)).flatten() {
        return Lookup { candidates: vec![candidate(g, system, BY_HASH, "same checksums".into())], status: "Found the game".into(), qualifier: String::new(), detail: "by its checksums".into() };
    }
    let hashed = !roms.is_empty();
    let mut family: Vec<(bool, &redump::Game)> =
        serial.map(|s| cat.by_serial(s)).unwrap_or_default().into_iter().map(|g| (sectors.zip(first_track_sectors(g)).is_some_and(|(a, b)| a.abs_diff(b) <= SECTOR_SLACK), g)).collect();
    // The close ones first; a size that rules everything out rules nothing out.
    family.sort_by_key(|(close, _)| !close);
    let candidates: Vec<Candidate> = family
        .iter()
        .map(|(close, g)| {
            let why = if hashed { "same serial, different checksums" } else { "same serial" };
            let size = if *close { ", the same size" } else { "" };
            candidate(g, system, if *close { BY_SERIAL_CLOSE } else { BY_SERIAL }, format!("{why}{size}"))
        })
        .collect();
    let (status, detail) = match (serial, candidates.len()) {
        (None, _) => ("Nothing to look up".to_owned(), "no serial on the disc".to_owned()),
        (Some(s), 0) => ("Not in Redump".to_owned(), format!("no games with serial {s}")),
        (Some(s), n) => (format!("Found {n} game{}", if n == 1 { "" } else { "s" }), format!("with serial {s}")),
    };
    Lookup { candidates, status, qualifier: String::new(), detail }
}

/// Redump's card when Check already named the game.
pub const NOT_NEEDED: &str = "Not needed";
pub const CHECK_FOUND: &str = "Check and fix found the game";

/// A lookup as Redump says it: when Check already named the game (its
/// `name` is on the dump) and the checksums found that same entry, there
/// was nothing to look up; its entry still goes on to Choose.
pub fn after_check(c: &Context, mut l: Lookup) -> Lookup {
    let named = c.get(Subject::Disc, "name");
    if let Some(cand) = l.candidates.first_mut().filter(|x| x.score >= BY_HASH && named == Some(x.title.as_str())) {
        cand.detail = "found by Check and fix".into();
        (l.status, l.detail) = (NOT_NEEDED.into(), CHECK_FOUND.into());
    }
    l
}

/// A system's catalogue, with its serials: a copy cached before games
/// asked for them is fetched again.
pub async fn game_catalogue(cache: &Path, slug: &str) -> Result<std::sync::Arc<redump::Catalogue>> {
    static REFETCHED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let c = redump::catalogue(cache, slug, redump::download).await?;
    // Once a run of the app: a system with no serials at all isn't fetched every disc.
    if c.has_serials() || REFETCHED.lock().unwrap().iter().any(|s| s == slug) {
        return Ok(c);
    }
    REFETCHED.lock().unwrap().push(slug.to_owned());
    redump::expire(cache, slug);
    redump::catalogue(cache, slug, redump::download).await
}

/// The dump's data files as Check hashed them, from what the dump knows.
pub fn hashed(c: &Context, main: &Path) -> Vec<redump::Rom> {
    data::data_files(main)
        .into_iter()
        .filter_map(|f| {
            let name = f.file_name()?.to_string_lossy().into_owned();
            let sha1 = c.get(Subject::Disc, &format!("sha1 {name}")).map(str::to_owned)?;
            let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
            Some(redump::Rom { name, size, crc: String::new(), md5: String::new(), sha1 })
        })
        .collect()
}

/// What a disc is known as so far: its system, serial and size.
pub fn known(c: &Context) -> (Option<String>, Option<String>, Option<u64>) {
    let g = |k| c.get(Subject::Disc, k).map(str::to_owned);
    (g("system"), g("serial"), g("sectors").and_then(|s| s.parse().ok()))
}

/// Shows a lookup on the tile and offers its candidates on the dump.
pub fn offer(cx: &Cx, dump: &mut trip_engine::Item, l: Lookup) {
    cx.log(format!("{}: {}", l.status, l.detail));
    for c in &l.candidates {
        cx.log(format!("  {} ({})", c.title, c.detail));
    }
    let note = if l.status == NOT_NEEDED { CHECK_FOUND.to_owned() } else { [l.status.as_str(), l.detail.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ") };
    cx.item(dump, ItemState::Ok, note);
    cx.outcome(l.status, l.qualifier, l.detail);
    dump.context_mut().offer(l.candidates);
}

pub struct Redump;

/// Check sent nothing on (you skipped the disc there): nothing to look up.
pub fn nothing_came(cx: &Cx) {
    cx.status(NOT_NEEDED, "nothing came from Check and fix");
}

#[async_trait]
impl Step for Redump {
    fn info(&self) -> StepInfo {
        redump_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let mut any = false;
        while let Some(mut dump) = inlet.recv().await {
            any = true;
            let (system, serial, sectors) = known(dump.context());
            let Some((system, slug)) = system.as_deref().and_then(|s| Some((s.to_owned(), slug_of(s)?))) else {
                cx.item(&dump, ItemState::Ok, "no system to look in");
                cx.status("Nowhere to look", "unknown system");
                outlet.send(dump).await;
                continue;
            };
            let roms = dump.path.as_deref().map(|m| hashed(dump.context(), m)).unwrap_or_default();
            cx.item(&dump, ItemState::Busy, format!("in {system}"));
            cx.status("", format!("Looking up {system}"));
            let cat = {
                let _net = cx.lane(LaneKind::Network("Redump".into())).await;
                game_catalogue(&data::redump_cache(&cx.work()?), slug).await
            };
            match cat {
                Ok(cat) => {
                    let l = after_check(dump.context(), lookup(&cat, &system, serial.as_deref(), sectors, &roms));
                    offer(&cx, &mut dump, l)
                }
                Err(e) => {
                    // Not reaching Redump isn't a bad dump: Choose asks.
                    cx.log(format!("the {system} catalogue: {e}"));
                    cx.item(&dump, ItemState::Ok, "couldn't reach Redump");
                    cx.status("Couldn't look up", system);
                }
            }
            outlet.send(dump).await;
        }
        if !any {
            nothing_came(&cx);
        }
        Ok(())
    }
}

// ---- Choose -------------------------------------------------------------------------------------

/// Settles which game the dump is. The hash decides when it matches; only
/// when it matches nothing are you asked, with the serial's entries (the
/// one closest in size first), typing the name, or going on without one.
///
/// Its own step, rather than the CD's Choose taking any kind: that one
/// decides alone on a single candidate however weak, and its form is an
/// album's (artist, album, track titles). A game's identity is one name.
pub struct Choose;

/// Whether Choose can decide alone: the best candidate is the hash's.
pub fn sure(cands: &[Candidate]) -> bool {
    cands.first().is_some_and(|c| c.score >= BY_HASH)
}

#[async_trait]
impl Step for Choose {
    fn info(&self) -> StepInfo {
        choose_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut dump) = inlet.recv().await {
            // What the lookups wired into it offered on the dump.
            let mut cands = dump.context().candidates().to_vec();
            cx.options(&cands);
            cands.sort_by(|a, b| b.score.total_cmp(&a.score));
            let why: &str;
            let pick = if sure(&cands) {
                why = "its hash is Redump's";
                Some(cands[0].clone())
            } else {
                let options = cands.iter().map(|c| Choice { label: c.title.clone(), detail: c.detail.clone(), destructive: false, picture: None }).collect();
                let guess = cands.first().map(|c| c.title.clone()).unwrap_or_else(|| data::dump_fields(dump.context()).name);
                let details = Some(BTreeMap::from([("name".to_owned(), guess)]));
                let prompt = if cands.is_empty() { "What is this game?" } else { "Which game? None matched exactly." };
                let q = Question { prompt: prompt.into(), shape: QuestionShape::PickOne { options, preselected: 0, details, skip: true } };
                cx.view(|v| v.detail = if cands.is_empty() { "not in Redump".into() } else { format!("{} with this serial number", cands.len()) });
                let (pick, w) = match cx.ask(q).await? {
                    Answer::Pick(i) => match cands.get(i) {
                        Some(c) => (Some(c.clone()), "you picked"),
                        None => (None, "nothing to pick from"),
                    },
                    Answer::Details(d) => (typed(dump.context(), &d), "entered by hand"),
                    Answer::Skip => (None, "you skipped"),
                    Answer::Value(v) => (typed(dump.context(), &BTreeMap::from([("name".to_owned(), v)])), "entered by hand"),
                    // Answers to questions about each item aren't this one's.
                    _ => (None, "no answer"),
                };
                why = w;
                pick
            };
            cx.decided(pick.as_ref(), why);
            match pick {
                Some(c) => {
                    let how = if c.score >= BY_HASH {
                        "verified"
                    } else if c.source == "you" {
                        "as you typed it"
                    } else {
                        "your choice, unverified"
                    };
                    cx.item(&dump, ItemState::Ok, format!("{}, {how}", c.title));
                    cx.status(c.title.clone(), how);
                    dump.context_mut().choose(c);
                }
                None => {
                    cx.item(&dump, ItemState::Ok, "no name");
                    cx.status("Skipped", "named by its label");
                    dump.context_mut().choose_none();
                }
            }
            outlet.send(dump).await;
        }
        Ok(())
    }
}

/// A candidate made of the name you typed.
fn typed(c: &Context, d: &BTreeMap<String, String>) -> Option<Candidate> {
    let name = d.get("name").map(|n| n.trim().to_owned()).filter(|n| !n.is_empty())?;
    let system = c.get(Subject::Disc, "system").unwrap_or("").to_owned();
    let g = redump::Game { name, serials: vec![], roms: vec![] };
    let mut c = candidate(&g, &system, 1.0, "typed in".into());
    c.id = "typed".into();
    c.source = "you".into();
    for f in &mut c.facts {
        f.source = "you".into();
    }
    Some(c)
}

// ---- M3U ----------------------------------------------------------------------------------------

/// Writes or rewrites the playlist of a multi-disc game, from the discs of
/// it filed so far. A run is one disc, so the set gathers across runs: each
/// disc filed rewrites the playlist from what's on disk, so filing disc 2
/// again doesn't list it twice, and a deleted disc stops being listed.
///
/// The playlist goes where the discs are: beside them, or, when each disc
/// has a folder of its own (`{system}/{name}`), in the folder above, listing
/// `Disc folder/Disc.chd`. Relative paths, LF, no BOM: several emulators
/// read nothing else. Gives where it went and how many discs it lists, or
/// `None` for one disc, since a playlist of one is clutter.
pub fn write_playlist(placed: &Path, set: &str) -> Result<Option<(PathBuf, usize)>> {
    let ext = placed.extension().and_then(|e| e.to_str()).unwrap_or("chd").to_owned();
    let Some(dir) = placed.parent() else { return Ok(None) };
    let own_folder = dir.file_name().is_some_and(|d| Some(d) == placed.file_stem());
    let top = if own_folder { dir.parent().unwrap_or(dir) } else { dir };
    let set = template::safe(set);
    let disc_of = |file: &Path| -> Option<u32> {
        if !file.extension().is_some_and(|e| e.eq_ignore_ascii_case(&ext)) {
            return None;
        }
        let (base, n) = split_disc(&file.file_stem()?.to_string_lossy())?;
        (base == set).then_some(n)
    };
    let mut discs: Vec<(u32, String)> = Vec::new();
    for e in std::fs::read_dir(top)?.flatten() {
        let p = e.path();
        if p.is_dir() {
            for f in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                if let Some(n) = disc_of(&f.path()) {
                    discs.push((n, format!("{}/{}", e.file_name().to_string_lossy(), f.file_name().to_string_lossy())));
                }
            }
        } else if let Some(n) = disc_of(&p) {
            discs.push((n, e.file_name().to_string_lossy().into_owned()));
        }
    }
    discs.sort();
    discs.dedup_by_key(|d| d.0);
    if discs.len() < 2 {
        return Ok(None);
    }
    let body: String = discs.iter().map(|(_, f)| format!("{f}\n")).collect();
    let path = top.join(format!("{set}.m3u"));
    super::place::write(&path, body)?;
    Ok(Some((path, discs.len())))
}

/// The set a filed disc belongs to: its Redump name without "(Disc N)".
pub fn set_of(c: &Context) -> Option<(String, u32)> {
    split_disc(&data::dump_fields(c).name)
}

pub struct M3u;

#[async_trait]
impl Step for M3u {
    fn info(&self) -> StepInfo {
        m3u_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(file) = inlet.recv().await {
            match (set_of(file.context()), file.path.clone()) {
                (Some((set, n)), Some(placed)) => match write_playlist(&placed, &set) {
                    Ok(Some((path, count))) => {
                        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        cx.log(format!("{}: {count} discs", path.display()));
                        cx.made(&path);
                        cx.item(&file, ItemState::Ok, format!("{name}, {count} discs"));
                        cx.outcome(name, format!("{count} discs"), "");
                    }
                    Ok(None) => {
                        cx.item(&file, ItemState::Ok, format!("disc {n}; the playlist waits for another"));
                        cx.status(format!("Disc {n} of {set}"), "Waits for another disc");
                    }
                    Err(e) => {
                        cx.log(format!("the playlist for {set}: {e}"));
                        cx.item(&file, ItemState::Bad, e.to_string());
                    }
                },
                _ => {
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

pub fn libcrypt_info() -> StepInfo {
    // Beside the filed image, looked up by Redump's name, for a disc Game
    // disc said is a PlayStation one.
    StepInfo::new("libcrypt", "Copy protection", Group::Make, ItemKind::ImageFile, ItemKind::ImageFile)
        .needs(&["filed"])
        .needs(&["system"])
        .needs(&["name", "release"])
        .from("LibCrypt file, for European PlayStation")
}

/// Many PAL PlayStation games check deliberately corrupted subchannel that
/// neither a BIN nor a CHD carries, and freeze an hour in without it. The
/// fix emulators read is `<image>.sbi` beside the image. Redump publishes
/// them as one zip keyed by Redump's name (`Ape Escape (Europe).sbi`),
/// which is the name the CHD is filed under. Lifted from the old app.
pub const SBI_PACK: &str = "http://redump.org/sbi/psx/";

/// One game's entry in the pack, matched forgivingly on case.
pub fn from_pack(zipped: &[u8], name: &str) -> Result<Option<Vec<u8>>> {
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(zipped)).map_err(|e| anyhow!("the SBI pack isn't a zip: {e}"))?;
    let want = format!("{name}.sbi").to_ascii_lowercase();
    let Some(i) = (0..a.len()).find(|&i| a.by_index_raw(i).is_ok_and(|e| e.name().is_ok_and(|n| n.trim().to_ascii_lowercase() == want))) else { return Ok(None) };
    let e = a.by_index(i)?;
    // An SBI is a few hundred bytes.
    let claimed = e.size();
    Ok(Some(super::net::read_at_most(e, claimed, super::net::MB)?))
}

/// The pack, from the cache when it's there and under a fortnight old,
/// else from Redump; a failed fetch falls back to a stale copy. Checked to
/// be a zip before it's kept: the URL answers 200 with a page when
/// something's wrong.
pub async fn sbi_pack(cache: &Path) -> Result<Vec<u8>> {
    let path = cache.join("psx-sbi.zip");
    let fresh = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() < 14 * 24 * 3600);
    if !fresh {
        let fetched: Result<Vec<u8>> = async {
            let r = super::net::plain(std::time::Duration::from_secs(60))?.get(SBI_PACK).send().await?.error_for_status()?;
            let body = super::net::body(r, super::net::JSON_MAX).await?;
            zip::ZipArchive::new(std::io::Cursor::new(&body)).map_err(|_| anyhow!("redump.org sent something that isn't the SBI pack"))?;
            Ok(body)
        }
        .await;
        match fetched {
            Ok(body) => {
                std::fs::create_dir_all(cache)?;
                super::place::write(&path, &body)?;
                return Ok(body);
            }
            Err(e) if !path.is_file() => return Err(e),
            Err(_) => {}
        }
    }
    Ok(std::fs::read(&path)?)
}

/// What LibCrypt does with one filed image, given the pack's entry for it.
pub fn place_sbi(placed: &Path, sbi: Option<&[u8]>) -> Result<String> {
    match sbi {
        Some(bytes) => {
            let to = placed.with_extension("sbi");
            super::place::write(&to, bytes)?;
            Ok(format!("{} beside it", to.file_name().unwrap_or_default().to_string_lossy()))
        }
        None => Ok("not protected".into()),
    }
}

pub struct LibCrypt;

#[async_trait]
impl Step for LibCrypt {
    fn info(&self) -> StepInfo {
        libcrypt_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(file) = inlet.recv().await {
            let system = file.context().get(Subject::Disc, "system").unwrap_or("").to_owned();
            let (Some(placed), "PlayStation") = (file.path.clone(), system.as_str()) else {
                cx.item(&file, ItemState::Ok, "not a PlayStation disc");
                cx.status("Not needed", "not PlayStation");
                outlet.send(file).await;
                continue;
            };
            let name = data::dump_fields(file.context()).name;
            let pack = {
                let _net = cx.lane(LaneKind::Network("Redump".into())).await;
                sbi_pack(&data::redump_cache(&cx.work()?)).await
            };
            let done = pack.and_then(|p| from_pack(&p, &name)).and_then(|sbi| place_sbi(&placed, sbi.as_deref()));
            match done {
                Ok(note) => {
                    cx.item(&file, ItemState::Ok, note.clone());
                    cx.status(if note.ends_with("beside it") { "Protection file saved" } else { "Not protected" }, note);
                }
                Err(e) => {
                    // A protected game without its SBI freezes later; say so now.
                    cx.log(format!("LibCrypt for {name}: {e}"));
                    cx.item(&file, ItemState::Bad, format!("couldn't check: {e}"));
                }
            }
            outlet.send(file).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playstation::tests::{iso, reader};

    #[test]
    fn a_libcrypt_sidecar_is_found_by_name_and_placed_beside_the_image() {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        z.start_file("Ape Escape (Europe).sbi", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"SBI\0data").unwrap();
        let pack = z.finish().unwrap().into_inner();
        assert_eq!(from_pack(&pack, "ape escape (europe)").unwrap().as_deref(), Some(&b"SBI\0data"[..]));
        assert_eq!(from_pack(&pack, "Ape Escape (USA)").unwrap(), None);
        assert!(from_pack(b"<html>", "x").is_err());
        let top = dir("sbi");
        let placed = top.join("Ape Escape (Europe).chd");
        assert_eq!(place_sbi(&placed, Some(b"SBI")).unwrap(), "Ape Escape (Europe).sbi beside it");
        assert!(top.join("Ape Escape (Europe).sbi").is_file());
        std::fs::remove_dir_all(top).unwrap();
    }

    fn saturn() -> Vec<u8> {
        let mut img = iso("SEGA SEGASATURN", "PANZER", &[]);
        let mut h = vec![b' '; 0x100];
        h[..16].copy_from_slice(b"SEGA SEGASATURN ");
        h[0x20..0x2A].copy_from_slice(b"MK-81009  ");
        h[0x40..0x50].copy_from_slice(b"E               ");
        h[0x60..0x6E].copy_from_slice(b"PANZER DRAGOON");
        img[..0x100].copy_from_slice(&h);
        img
    }

    #[test]
    fn a_playstation_disc_says_its_system_serial_region_and_boot_file() {
        let img = iso("PLAYSTATION", "SLES_008.60", &[("SYSTEM.CNF;1", b"BOOT = cdrom:\\SLES_008.60;1\r\nTCB = 4\r\n")]);
        let id = identify(&reader(&img), 0).unwrap();
        assert_eq!(id.headline(), "PlayStation, SLES-00860");
        assert_eq!((id.region.as_deref(), id.boot.as_deref(), id.sectors, id.slug()), (Some("Europe"), Some("SLES_008.60"), Some(240_000), Some("psx")));
        let ps2 = iso("PLAYSTATION", "SLUS_202.16", &[("SYSTEM.CNF;1", b"BOOT2 = cdrom0:\\SLUS_202.16;1\nVER = 1.00\n")]);
        let id = identify(&reader(&ps2), 0).unwrap();
        assert_eq!((id.headline().as_str(), id.slug()), ("PlayStation 2, SLUS-20216", Some("ps2")));
    }

    #[test]
    fn a_saturn_disc_says_its_serial_from_its_header() {
        let id = identify(&reader(&saturn()), 0).unwrap();
        assert_eq!(id.headline(), "Sega Saturn, MK-81009");
        assert_eq!((id.region.as_deref(), id.title.as_str(), id.slug()), (Some("Europe"), "PANZER DRAGOON", Some("ss")));
    }

    #[test]
    fn a_mega_cd_disc_says_its_serial_from_its_header() {
        let mut img = vec![b' '; SECTOR_ * 2];
        img[..16].copy_from_slice(b"SEGADISCSYSTEM  ");
        img[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
        img[0x150..0x157].copy_from_slice(b"SONIC C");
        img[0x180..0x18E].copy_from_slice(b"GM MK-4407 -00");
        img[0x1F0..0x1F3].copy_from_slice(b"JUE");
        let id = identify(&reader(&img), 0).unwrap();
        assert_eq!(id.headline(), "Sega Mega CD, MK-4407");
        assert_eq!(id.region.as_deref(), Some("Japan, USA, Europe"));
    }

    #[test]
    fn a_pc_engine_cd_is_found_on_its_data_track() {
        // Track 1 is audio; the data track starts at 100.
        let mut img = vec![0u8; SECTOR_ * 102];
        let at = SECTOR_ * 101 + 0x20;
        img[at..at + 23].copy_from_slice(b"PC Engine CD-ROM SYSTEM");
        assert_eq!(identify(&reader(&img), 100).unwrap().headline(), "PC Engine CD");
        assert_eq!(identify(&reader(&img), 0).unwrap().headline(), "Game disc (unknown system)");
    }

    #[test]
    fn three_do_neo_geo_and_unknown_discs() {
        let mut img = vec![0u8; SECTOR_ * 2];
        img[0] = 1;
        img[1..6].copy_from_slice(&[0x5A; 5]);
        img[0x28..0x2E].copy_from_slice(b"CD-ROM");
        let id = identify(&reader(&img), 0).unwrap();
        assert_eq!((id.headline().as_str(), id.label.as_str()), ("3DO", "CD-ROM"));
        let ngcd = iso("", "NEOGEO", &[("IPL.TXT;1", b"FILE.PRG,0,0\r\n")]);
        assert_eq!(identify(&reader(&ngcd), 0).unwrap().headline(), "Neo Geo CD");
        let pc = iso("", "MYBACKUP", &[]);
        let id = identify(&reader(&pc), 0).unwrap();
        assert_eq!((id.headline().as_str(), id.label.as_str(), id.slug()), ("Game disc (unknown system)", "MYBACKUP", None));
        assert_eq!(identify(&reader(&[]), 0).unwrap(), Identity::default(), "nothing readable is no system, not an error");
    }

    #[test]
    fn a_ps3_disc_and_a_dvd_video_are_refused_by_name() {
        let ps3 = iso("PLAYSTATION", "BLUS30443", &[("PS3_DISC.SFB;1", b"\0")]);
        let id = identify(&reader(&ps3), 0).unwrap();
        assert_eq!(id.system, Some("PlayStation 3"));
        assert!(id.refusal.unwrap().contains("locked to each game"));
        let dvd = iso("", "MOVIE", &[("VIDEO_TS", b"")]);
        assert!(identify(&reader(&dvd), 0).unwrap().refusal.unwrap().contains("DVD-Video"));
    }

    #[test]
    fn a_failed_directory_read_is_an_error_to_retry() {
        let mut img = iso("PLAYSTATION", "X", &[("SYSTEM.CNF;1", b"BOOT = cdrom:\\SLUS_007.79;1\n")]);
        img.truncate(SECTOR_ * 21 + 10);
        assert!(identify(&reader(&img), 0).is_err());
    }

    #[test]
    fn disc_numbers_and_regions_come_from_redump_names() {
        assert_eq!(split_disc("Final Fantasy VII (USA) (Disc 2)"), Some(("Final Fantasy VII (USA)".into(), 2)));
        assert_eq!(split_disc("Game (Europe) (Disc 1) (Rev 1)"), Some(("Game (Europe) (Rev 1)".into(), 1)));
        assert_eq!(split_disc("Ape Escape (Europe)"), None);
        assert_eq!(region_in("Ape Escape (Europe) (En,Fr,De)").as_deref(), Some("Europe"));
        assert_eq!(region_in("Myst (USA, Europe)").as_deref(), Some("USA, Europe"));
    }

    const DAT: &str = crate::game_sim::GAME_DAT;

    fn rom(name: &str, sha1: &str) -> redump::Rom {
        redump::Rom { name: name.into(), size: 0, crc: String::new(), md5: String::new(), sha1: sha1.into() }
    }

    #[test]
    fn the_hash_decides_the_pressing() {
        let cat = redump::parse(DAT.as_bytes()).unwrap();
        let roms = [rom("a.cue", "ffff"), rom("a (Track 1).bin", "7210064da9a542124d4e32d6202672cc6b5756d2"), rom("a (Track 2).bin", "28a06c34d53442f736fed283c086340d7c56321e")];
        let l = lookup(&cat, "PlayStation", Some("SLES-00860"), None, &roms);
        assert_eq!(l.candidates.len(), 1);
        assert_eq!(l.candidates[0].title, "Colony Wars (Europe) (Disc 1)");
        assert!(sure(&l.candidates));
        let facts: BTreeMap<_, _> = l.candidates[0].facts.iter().map(|f| (f.key.as_str(), f.value.as_str())).collect();
        assert_eq!(facts["set"], "Colony Wars (Europe)");
        assert_eq!(facts["disc"], "1");
        assert_eq!(facts["system"], "PlayStation");
    }

    #[test]
    fn a_serial_names_a_game_not_a_pressing_and_size_breaks_the_tie() {
        let cat = redump::parse(DAT.as_bytes()).unwrap();
        // Tomb Raider: two revisions under one serial, a few sectors apart.
        let l = lookup(&cat, "PlayStation", Some("SLUS_001.52"), None, &[]);
        assert_eq!(l.candidates.len(), 2);
        assert_eq!((l.status.as_str(), l.detail.as_str()), ("Found 2 games", "with serial SLUS_001.52"));
        assert!(!sure(&l.candidates), "a serial alone never decides");
        // Hashed, matching nothing: the serial's entries, the one of this size first.
        let l = lookup(&cat, "PlayStation", Some("SLUS-00152"), Some(1_000), &[rom("x.bin", "0000")]);
        assert_eq!((l.status.as_str(), l.candidates[0].title.as_str()), ("Found 2 games", "Tomb Raider (USA) (Rev 1)"));
        assert_eq!(l.candidates[0].score, BY_SERIAL_CLOSE);
        assert_eq!(l.candidates[0].detail, "same serial, different checksums, the same size");
        // Nothing for the serial either.
        let l = lookup(&cat, "PlayStation", Some("SLES-99999"), None, &[rom("x.bin", "0000")]);
        assert!(l.candidates.is_empty());
        assert_eq!((l.status.as_str(), l.detail.as_str()), ("Not in Redump", "no games with serial SLES-99999"));
        // Check named it by its checksums: nothing to look up.
        let named = [rom("a (Track 1).bin", "7210064da9a542124d4e32d6202672cc6b5756d2"), rom("a (Track 2).bin", "28a06c34d53442f736fed283c086340d7c56321e")];
        let l = lookup(&cat, "PlayStation", Some("SLES-00860"), None, &named);
        assert_eq!((l.status.as_str(), l.detail.as_str()), ("Found the game", "by its checksums"), "found here: Check didn't look in this list");
        let mut c = Context::default();
        c.add(Subject::Disc, "name", "Colony Wars (Europe) (Disc 1)", "Redump");
        let l = after_check(&c, l);
        assert_eq!((l.status.as_str(), l.detail.as_str(), l.candidates[0].detail.as_str()), (NOT_NEEDED, CHECK_FOUND, "found by Check and fix"));
        assert!(sure(&l.candidates), "its entry still settles Choose");
    }

    #[test]
    fn serials_match_however_either_side_spells_them() {
        let cat = redump::parse(DAT.as_bytes()).unwrap();
        assert_eq!(cat.by_serial("SCES_015.64").len(), 1, "the disc's spelling");
        assert_eq!(cat.by_serial("SCES-01564-P").len(), 1, "the second of two serials");
        assert_eq!(cat.by_serial("MK-4407").len(), 1, "Redump writes 4407");
        assert!(cat.by_serial("ABCD-00152").is_empty(), "another prefix's number is another game");
        assert_eq!(cat.by_serial("SLUS-00152").len(), 2);
        assert!(cat.by_serial("").is_empty());
        assert!(cat.has_serials());
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::current_dir().unwrap().join("../../target/trip-test").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_playlist_lists_the_discs_on_disk_in_their_folders() {
        let top = dir("m3u-folders");
        for n in [2, 1] {
            let name = format!("Colony Wars (Europe) (Disc {n})");
            std::fs::create_dir_all(top.join(&name)).unwrap();
            std::fs::write(top.join(&name).join(format!("{name}.chd")), b"x").unwrap();
        }
        std::fs::create_dir_all(top.join("Koudelka (Europe) (Disc 1)")).unwrap();
        std::fs::write(top.join("Koudelka (Europe) (Disc 1)/Koudelka (Europe) (Disc 1).chd"), b"x").unwrap();
        let placed = top.join("Colony Wars (Europe) (Disc 2)/Colony Wars (Europe) (Disc 2).chd");
        let (path, n) = write_playlist(&placed, "Colony Wars (Europe)").unwrap().unwrap();
        assert_eq!((path, n), (top.join("Colony Wars (Europe).m3u"), 2));
        let body = std::fs::read_to_string(top.join("Colony Wars (Europe).m3u")).unwrap();
        assert_eq!(body, "Colony Wars (Europe) (Disc 1)/Colony Wars (Europe) (Disc 1).chd\nColony Wars (Europe) (Disc 2)/Colony Wars (Europe) (Disc 2).chd\n");
        // Rewritten, not appended to.
        write_playlist(&placed, "Colony Wars (Europe)").unwrap();
        assert_eq!(std::fs::read_to_string(top.join("Colony Wars (Europe).m3u")).unwrap(), body);
        std::fs::remove_dir_all(top).unwrap();
    }

    #[test]
    fn a_playlist_beside_its_discs_and_none_for_one_disc() {
        let top = dir("m3u-flat");
        std::fs::write(top.join("Game (USA) (Disc 1).chd"), b"x").unwrap();
        assert_eq!(write_playlist(&top.join("Game (USA) (Disc 1).chd"), "Game (USA)").unwrap(), None, "one disc is no set");
        std::fs::write(top.join("Game (USA) (Disc 2).chd"), b"x").unwrap();
        write_playlist(&top.join("Game (USA) (Disc 2).chd"), "Game (USA)").unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(top.join("Game (USA).m3u")).unwrap(), "Game (USA) (Disc 1).chd\nGame (USA) (Disc 2).chd\n");
        std::fs::remove_dir_all(top).unwrap();
    }

    /// Reads a real datfile fetched with its serials:
    /// `TRIP_DAT=…/psx.dat cargo test -p trip-steps real_serials -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_serials() {
        let xml = std::fs::read(std::env::var("TRIP_DAT").unwrap()).unwrap();
        let cat = redump::parse(&xml).unwrap();
        let with = cat.games.iter().filter(|g| !g.serials.is_empty()).count();
        println!("{} entries, {with} with serials", cat.games.len());
        for s in std::env::var("TRIP_SERIALS").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
            println!("{s}: {:?}", cat.by_serial(s).iter().map(|g| &g.name).collect::<Vec<_>>());
        }
    }

    /// Reads whatever is in the drive, read-only. With an audio CD in, it's
    /// no game: `cargo test -p trip-steps real_game_disc -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_game_disc() {
        let dev = std::env::var("TRIP_DEVICE").unwrap_or_else(|_| "/dev/sr0".into());
        println!("{:?}", media::toc_starts(&dev));
        println!("{:?}", identify_device(&dev));
    }
}
