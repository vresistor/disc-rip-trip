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

use super::playstation::{self, Sectors, Volume};
use super::{data, media, redump};
use crate::template;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use drt_engine::snapshot::QuestionShape;
use drt_engine::{Answer, Candidate, Choice, Context, Cx, Fact, Group, Inlet, ItemKind, ItemState, LaneKind, Outlet, Question, Registry, Step, StepInfo, StepState, Subject};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

/// redumper's full dump, with Re-read bad parts and Keep it anyway: Image's code, run
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

/// A refine: re-reads the disc, keeps the new hashes on the dump, and gives
/// Redump's name for it if it now matches.
pub type Refine = for<'a> fn(&'a Cx, &'a mut drt_engine::Item) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<String>>> + Send + 'a>>;

/// Holds a suspect dump, Bad, until you say: Refine (re-read, and it goes
/// on if it now matches), Keep it anyway, or Skip this disc (it isn't filed). Filing
/// waits meanwhile. Gives the dump to send on, if any.
pub async fn hold(cx: &Cx, mut dump: drt_engine::Item, serial: &str, refine: Refine) -> Result<Option<drt_engine::Item>> {
    let bad = format!("Doesn't match Redump's dumps of {serial}");
    cx.log(&bad);
    loop {
        cx.item(&dump, ItemState::Bad, bad.clone());
        cx.state(StepState::Bad);
        cx.status("Doesn't match Redump", serial.to_owned());
        let req = loop {
            match cx.next_retry().await {
                None => return Ok(None),
                Some(r) if r.item == dump.id => break r,
                Some(_) => {}
            }
        };
        cx.state(StepState::Busy);
        match req.option.as_str() {
            "leave_out" => {
                cx.item(&dump, ItemState::Skipped, "skipped");
                cx.status("Skipped", serial.to_owned());
                return Ok(None);
            }
            "keep" => {
                cx.item(&dump, ItemState::Ok, format!("kept anyway; doesn't match Redump's copies of {serial}"));
                cx.status("Kept anyway", "doesn't match Redump");
                return Ok(Some(dump));
            }
            _ => {
                cx.item(&dump, ItemState::Busy, "re-reading bad parts");
                cx.status("Re-reading bad parts", serial.to_owned());
                if let Some(name) = refine(cx, &mut dump).await? {
                    cx.item(&dump, ItemState::Ok, format!("Verified: {name}"));
                    cx.status("Verified", "after re-reading bad parts");
                    return Ok(Some(dump));
                }
                cx.log("refined; still doesn't match");
            }
        }
    }
}

/// Check's hold for a game: whether the dump is suspect, and if so, holds
/// it. Gives the dump to send on, if any. A catalogue that can't be had
/// isn't a reason to hold anything.
pub async fn hold_if_suspect(cx: &Cx, dump: drt_engine::Item, roms: &[redump::Rom]) -> Result<Option<drt_engine::Item>> {
    let (system, serial, _) = known(dump.context());
    let (Some(serial), Some(slug)) = (serial, system.as_deref().and_then(slug_of)) else { return Ok(Some(dump)) };
    let Ok(cat) = game_catalogue(&data::redump_cache(&cx.work()?), slug).await else { return Ok(Some(dump)) };
    if !is_suspect(&cat, &serial, roms) {
        return Ok(Some(dump));
    }
    hold(cx, dump, &serial, |cx, dump| Box::pin(refine(cx, dump))).await
}

/// redumper's refine on the dump, a fresh split and check of it, then its
/// hashes again and Redump's name if they now match.
async fn refine(cx: &Cx, dump: &mut drt_engine::Item) -> Result<Option<String>> {
    use super::{hash, redumper};
    let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
    let dir = main.parent().ok_or_else(|| anyhow!("{} has no folder", main.display()))?.to_path_buf();
    let t = redumper::Target { dir, name: main.file_stem().unwrap_or_default().to_string_lossy().into_owned() };
    let opts = redumper::Options { drive: redumper::drive_access(&super::device(dump.context())?)?, retries: 20, speed: None };
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

/// Looks a disc up in its system's catalogue: the entry its hashes match,
/// when they do; otherwise every entry for its serial, the ones close to
/// its size first. Before the dump (no hashes), the serial's entries.
pub fn lookup(cat: &redump::Catalogue, system: &str, serial: Option<&str>, sectors: Option<u64>, roms: &[redump::Rom]) -> Lookup {
    if let Some(g) = (!roms.is_empty()).then(|| cat.by_hashes(roms)).flatten() {
        return Lookup { candidates: vec![candidate(g, system, BY_HASH, "exact match".into())], status: "1 match".into(), qualifier: String::new(), detail: String::new() };
    }
    let hashed = !roms.is_empty();
    let mut family: Vec<(bool, &redump::Game)> =
        serial.map(|s| cat.by_serial(s)).unwrap_or_default().into_iter().map(|g| (sectors.zip(first_track_sectors(g)).is_some_and(|(a, b)| a.abs_diff(b) <= SECTOR_SLACK), g)).collect();
    // The close ones first; a size that rules everything out rules nothing out.
    family.sort_by_key(|(close, _)| !close);
    let candidates: Vec<Candidate> = family
        .iter()
        .map(|(close, g)| {
            let why = if hashed { "same serial, but not an exact match" } else { "by serial" };
            let size = if *close { ", the same size" } else { "" };
            candidate(g, system, if *close { BY_SERIAL_CLOSE } else { BY_SERIAL }, format!("{why}{size}"))
        })
        .collect();
    let serial = serial.unwrap_or("no serial");
    let entries = |n: usize| format!("{n} match{}", if n == 1 { "" } else { "es" });
    let (status, qualifier, detail) = match (hashed, candidates.len()) {
        (true, 0) => ("Not in Redump".to_owned(), String::new(), serial.to_owned()),
        (true, n) => (entries(n), "by serial only".to_owned(), format!("no exact match for {serial}")),
        (false, 0) => ("Not listed".to_owned(), String::new(), serial.to_owned()),
        (false, n) => (entries(n), "by serial only".to_owned(), serial.to_owned()),
    };
    Lookup { candidates, status, qualifier, detail }
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
pub fn offer(cx: &Cx, dump: &mut drt_engine::Item, l: Lookup) {
    cx.log(format!("{}: {}", l.status, l.detail));
    for c in &l.candidates {
        cx.log(format!("  {} ({})", c.title, c.detail));
    }
    cx.item(dump, ItemState::Ok, [l.status.as_str(), l.qualifier.as_str(), l.detail.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", "));
    cx.outcome(l.status, l.qualifier, l.detail);
    dump.context_mut().offer(l.candidates);
}

pub struct Redump;

#[async_trait]
impl Step for Redump {
    fn info(&self) -> StepInfo {
        redump_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut dump) = inlet.recv().await {
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
                Ok(cat) => offer(&cx, &mut dump, lookup(&cat, &system, serial.as_deref(), sectors, &roms)),
                Err(e) => {
                    // Not reaching Redump isn't a bad dump: Choose asks.
                    cx.log(format!("the {system} catalogue: {e}"));
                    cx.item(&dump, ItemState::Ok, "couldn't reach Redump");
                    cx.status("Couldn't look up", system);
                }
            }
            outlet.send(dump).await;
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
                let options = cands.iter().map(|c| Choice { label: c.title.clone(), detail: c.detail.clone(), destructive: false }).collect();
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
        assert_eq!(l.qualifier, "by serial only");
        assert_eq!(l.candidates.len(), 2);
        assert_eq!(l.status, "2 matches");
        assert!(!sure(&l.candidates), "a serial alone never decides");
        // Hashed, matching nothing: the serial's entries, the one of this size first.
        let l = lookup(&cat, "PlayStation", Some("SLUS-00152"), Some(1_000), &[rom("x.bin", "0000")]);
        assert_eq!((l.status.as_str(), l.qualifier.as_str(), l.candidates[0].title.as_str()), ("2 matches", "by serial only", "Tomb Raider (USA) (Rev 1)"));
        assert_eq!(l.candidates[0].score, BY_SERIAL_CLOSE);
        assert!(l.candidates[0].detail.starts_with("same serial, but not an exact match"));
        // Nothing for the serial either.
        let l = lookup(&cat, "PlayStation", Some("SLES-99999"), None, &[rom("x.bin", "0000")]);
        assert!(l.candidates.is_empty());
        assert_eq!(l.status, "Not in Redump");
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
        let d = std::env::current_dir().unwrap().join("../../target/drt-test").join(name);
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
    /// `DRT_DAT=…/psx.dat cargo test -p drt-steps real_serials -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_serials() {
        let xml = std::fs::read(std::env::var("DRT_DAT").unwrap()).unwrap();
        let cat = redump::parse(&xml).unwrap();
        let with = cat.games.iter().filter(|g| !g.serials.is_empty()).count();
        println!("{} entries, {with} with serials", cat.games.len());
        for s in std::env::var("DRT_SERIALS").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
            println!("{s}: {:?}", cat.by_serial(s).iter().map(|g| &g.name).collect::<Vec<_>>());
        }
    }

    /// Reads whatever is in the drive, read-only. With an audio CD in, it's
    /// no game: `cargo test -p drt-steps real_game_disc -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_game_disc() {
        let dev = std::env::var("DRT_DEVICE").unwrap_or_else(|_| "/dev/sr0".into());
        println!("{:?}", media::toc_starts(&dev));
        println!("{:?}", identify_device(&dev));
    }
}
