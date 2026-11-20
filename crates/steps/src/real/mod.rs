//! The real steps: a real drive, cyanrip, MusicBrainz, ffmpeg and files.
//! Same names, parameters and outputs as the simulated ones, so a recipe
//! runs on either.

pub mod chd;
pub mod cue;
pub mod cyanrip;
pub mod data;
pub mod discogs;
pub mod encode;
pub mod fixtags;
pub mod flush;
pub mod game;
pub mod gnudb;
pub mod hash;
pub mod lyrics;
pub mod media;
pub mod musicbrainz;
pub mod net;
pub mod offset;
pub mod place;
pub mod playstation;
pub mod redump;
pub mod redumper;
pub mod replaygain;
pub mod tell;
pub mod tool;
pub mod video;

use crate::template;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use cyanrip::{RipOptions, Verdict};
use drt_engine::setting::ParamKind;
use drt_engine::{Context, Cx, Group, Inlet, Item, ItemKind, ItemState, LaneKind, Outlet, ParamSpec, Registry, Step, StepInfo, StepState, Subject};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn register(r: &mut Registry) {
    r.add(Drive);
    r.add(AudioCd);
    r.add(CdText);
    r.add(MusicBrainz);
    r.add(discogs::Discogs);
    r.add(gnudb::GnuDb);
    r.add(crate::choose::Choose);
    r.add(CoverArt);
    r.add(lyrics::Lyric);
    r.add(lyrics::LyricsFile);
    r.add(Rip::with_flush(std::sync::Arc::new(flush::read_sectors)));
    r.add(cue::CueSheet);
    r.add(replaygain::ReplayGain);
    r.add(tell::Tell);
    for c in encode::ALL {
        r.add(encode::Encode(c));
    }
    r.add(data::DataDisc);
    r.add(data::Image);
    r.add(data::Check);
    r.add(data::Chd);
    r.add(data::Keep);
    r.add(Place { kind: "folder", name: "Folder" });
    // Library was a second name for the same step; recipes saved with it still load.
    r.alias("library", "folder");
    r.add(crate::hold::Hold);
    r.add(NextDisc { real: true });
    video::register(r);
    game::register(r);
}

/// How many tracks the steps after Rip will see: the ones it was asked
/// for, or failing that the whole disc.
fn parts(c: &Context) -> u32 {
    c.get(Subject::Disc, "selected").and_then(|v| v.parse().ok()).unwrap_or(c.parts()).max(1)
}

/// `4:07`: minutes and seconds.
pub fn mmss(s: impl Into<u64>) -> String {
    let s = s.into();
    format!("{}:{:02}", s / 60, s % 60)
}

/// The drive the disc is in, which the Drive step put on it.
fn device(c: &Context) -> Result<String> {
    c.get(Subject::Disc, "device").map(str::to_owned).ok_or_else(|| anyhow!("no drive before this step"))
}

// ---- the drive ------------------------------------------------------------------------------

struct Drive;

/// Optical drives from sysfs: (vendor and model, device).
pub fn drives() -> Vec<(String, String)> {
    let Ok(dir) = std::fs::read_dir("/sys/class/block") else { return vec![] };
    let mut names: Vec<String> = dir.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.starts_with("sr")).collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let (vendor, model) = vendor_model(&n);
            (format!("{vendor} {model}").trim().to_owned(), format!("/dev/{n}"))
        })
        .collect()
}

/// A drive's vendor and model from sysfs, by its block name (`sr0`).
fn vendor_model(name: &str) -> (String, String) {
    let read = |f: &str| std::fs::read_to_string(format!("/sys/class/block/{name}/device/{f}")).unwrap_or_default().trim().to_owned();
    (read("vendor"), read("model"))
}

/// The drive a recipe names, matched on every word of the name: "MATSHITA
/// UJ240AS" is the drive sysfs calls "MATSHITA BD-MLT UJ240AS".
fn find_drive(name: &str) -> Option<String> {
    let want: Vec<String> = name.split_whitespace().map(str::to_lowercase).collect();
    drives().into_iter().find_map(|(model, dev)| {
        let words: Vec<String> = model.split_whitespace().map(str::to_lowercase).collect();
        want.iter().all(|w| words.contains(w)).then_some(dev)
    })
}

#[async_trait]
impl Step for Drive {
    fn info(&self) -> StepInfo {
        StepInfo::new("drive", "Drive", Group::Source, ItemKind::Nothing, ItemKind::Disc).from_param("drive").param(drive_param()).param(offset_param())
    }

    async fn find(&self, key: &str, drive: &str, measure: bool) -> Option<(Value, String)> {
        if key != "offset" {
            return None;
        }
        find_offset(drive, measure).await.map(|(o, source)| (Value::from(o), source))
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        inlet.recv().await;
        let name = cx.text("drive");
        let Some(dev) = find_drive(&name) else { bail!("{name} isn't connected") };
        while !media::disc_in(&dev) {
            cx.status("Empty", "Insert a disc");
            cx.sleep(1000).await?;
        }
        let mut disc = Item::disc("disc");
        disc.context_mut().add(Subject::Disc, "device", dev.clone(), "the drive");
        // The offset belongs to the drive; it goes on the disc, and Rip
        // reads it from there.
        // Known before the start: looked up as soon as the recipe had this drive.
        // Only an audio Rip needs it (and refuses to start without it); a
        // video or data disc doesn't.
        let offset = cx.param("offset").and_then(|v| v.as_i64());
        if let Some(offset) = offset {
            disc.context_mut().add(Subject::Disc, "offset", offset.to_string(), "the drive");
        }
        cx.status("Disc in", if offset.is_none() && cx.param("offset").is_none() { "Read offset unknown" } else { "" });
        outlet.send(disc).await;
        Ok(())
    }
}

/// Looks a drive's offset up, in order of trust: AccurateRip's drive list
/// (everyone with this model agreeing), then what the old app recorded.
/// With `measure`, and a disc AccurateRip knows in the drive, measures it.
/// Gives the offset and where it came from.
pub async fn find_offset(drive: &str, measure: bool) -> Option<(i64, String)> {
    let dev = find_drive(drive)?;
    let (vendor, model) = vendor_model(dev.rsplit('/').next().unwrap_or(""));
    let cache = drt_engine::cache_dir()?.join("cache");
    let listed = match offset::list(&cache).await {
        Ok(list) => offset::find(&list, &vendor, &model),
        Err(_) => None,
    };
    if measure {
        let dir = std::env::temp_dir().join("disc-rip-trip-measure");
        std::fs::create_dir_all(&dir).ok()?;
        let (m, confidence) = offset::measure(&dev, &dir, std::future::pending()).await.ok()?;
        return Some(judge_measurement(m, confidence, listed.as_ref()));
    }
    if let Some(l) = listed {
        return Some((l.offset, format!("AccurateRip, {} agree", l.submitted)));
    }
    offset::from_old_app(&vendor, &model).map(|o| (o, "an earlier version of the app".to_owned()))
}

/// What a measurement with one disc is worth beside AccurateRip's drive
/// list: nothing, for a drive the list has. The offset belongs to the
/// drive, and one disc can't tell the drive's offset from its pressing's:
/// Parquet Courts' *Wide Awake!* verifies at +103 (confidence 5) and at
/// +121 (confidence 43) on a drive the list gives +103, because
/// AccurateRip holds two pressings 18 samples apart. So a measurement only
/// counts for a drive the list doesn't have.
pub fn judge_measurement(measured: i64, confidence: u32, listed: Option<&offset::Listed>) -> (i64, String) {
    match listed {
        Some(l) => (l.offset, format!("AccurateRip, {} agree", l.submitted)),
        None => (measured, format!("measured, {confidence} match{}", if confidence == 1 { "" } else { "es" })),
    }
}

// ---- the kind step and CD-Text ------------------------------------------------------------------

struct AudioCd;

#[async_trait]
impl Step for AudioCd {
    fn info(&self) -> StepInfo {
        StepInfo::new("audio-cd", "Audio CD", Group::Kind, ItemKind::Disc, ItemKind::AudioDisc).gives(&["toc"]).from("read the track list")
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let dev = device(disc.context())?;
            cx.status("", "Reading the table of contents");
            let info = {
                let _drive = cx.lane(LaneKind::Drive).await;
                cyanrip::info(&dev, &cx.work()?, cx.cancelled()).await?
            };
            let total: u32 = info.tracks.iter().map(|t| t.seconds()).sum();
            {
                let c = disc.context_mut();
                let disc = "the disc";
                c.add(Subject::Disc, "parts", info.tracks.len().to_string(), disc);
                if !info.drive.is_empty() {
                    c.add(Subject::Disc, "drive", info.drive.clone(), disc);
                }
                c.add(Subject::Disc, "disc_id", info.disc_id.clone(), disc);
                c.add(Subject::Disc, "toc", info.toc.clone(), disc);
                if !info.mcn.is_empty() {
                    c.add(Subject::Disc, "barcode", info.mcn.clone(), disc);
                }
                // The drive won't read slower: *Read again slowly* knows.
                if info.fixed_speed {
                    c.add(Subject::Disc, "fixed_speed", "1", "the drive");
                }
                // What CD-Text said, kept aside: the CD-Text step offers it
                // as a candidate.
                if !info.album.is_empty() {
                    c.add(Subject::Disc, "cdtext.album", info.album.clone(), disc);
                }
                if !info.performer.is_empty() {
                    c.add(Subject::Disc, "cdtext.artist", info.performer.clone(), disc);
                }
                for t in &info.tracks {
                    c.add(Subject::Part(t.number), "length", t.seconds().to_string(), disc);
                    if !t.title.is_empty() {
                        c.add(Subject::Part(t.number), "cdtext.title", t.title.clone(), disc);
                    }
                    if !t.isrc.is_empty() {
                        c.add(Subject::Part(t.number), "isrc", t.isrc.clone(), disc);
                    }
                    if t.preemphasis {
                        c.add(Subject::Part(t.number), "preemphasis", "1", disc);
                    }
                }
            }
            cx.expect(info.tracks.len());
            cx.outcome(format!("{} tracks", info.tracks.len()), mmss(total), "");
            cx.log(format!("disc ID {}, TOC {}", info.disc_id, info.toc));
            outlet.send(disc).await;
        }
        Ok(())
    }
}

struct CdText;

#[async_trait]
impl Step for CdText {
    fn info(&self) -> StepInfo {
        StepInfo::new("cd-text", "CD-Text", Group::LookUp, ItemKind::AudioDisc, ItemKind::AudioDisc)
            .gives(&["candidates"])
            .unless(&["release"])
            .right_after(&["audio-cd"])
            .from("track names stored on the disc")
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            offer_cdtext(&cx, &mut disc);
            outlet.send(disc).await;
        }
        Ok(())
    }
}

/// CD-Text's work, real or simulated: what the disc's own text says it is,
/// offered on the disc as one candidate for Choose.
pub fn offer_cdtext(cx: &Cx, disc: &mut Item) {
    match crate::choose::cdtext_candidate(disc.context()) {
        None => {
            cx.status("None on the disc", "");
            cx.log("the disc has no CD-Text");
        }
        Some(c) => {
            let titles = c.facts.iter().filter(|f| f.key == "title").count();
            // The disc's own album line, as it carries it.
            let album = c.facts.iter().find(|f| f.subject == Subject::Disc && f.key == "album").map(|f| f.value.clone()).unwrap_or_default();
            cx.status(format!("{titles} track name{}", if titles == 1 { "" } else { "s" }), album);
            cx.log(format!("offered the disc's own text: {}", c.detail));
            disc.context_mut().offer([c]);
        }
    }
}

// ---- lookups ------------------------------------------------------------------------------------

struct MusicBrainz;

#[async_trait]
impl Step for MusicBrainz {
    fn info(&self) -> StepInfo {
        StepInfo::new("musicbrainz", "MusicBrainz", Group::LookUp, ItemKind::AudioDisc, ItemKind::AudioDisc)
            .gives(&["candidates"])
            .unless(&["release"])
            .right_after(&["audio-cd"])
            .from("free online music database")
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let (id, toc) = {
                let c = disc.context();
                (c.get(Subject::Disc, "disc_id").unwrap_or("").to_owned(), c.get(Subject::Disc, "toc").unwrap_or("").to_owned())
            };
            cx.status("", "Asking by disc ID");
            let found = {
                let _net = cx.lane(LaneKind::Network("MusicBrainz".into())).await;
                musicbrainz::lookup(&id, &toc).await
            };
            match found {
                Ok(c) if c.is_empty() => cx.status("Nothing found", ""),
                Ok(c) => {
                    // Only the weaker match is worth saying.
                    let exact = c.iter().any(|x| x.score >= 0.9);
                    cx.outcome(format!("{} found", c.len()), if exact { "" } else { "by track lengths" }, "");
                    disc.context_mut().offer(c);
                }
                // A lookup failing doesn't stop the disc; Choose decides with what the others found.
                Err(e) => {
                    lookup_skipped(&cx, &disc, e.to_string());
                    cx.log(format!("lookup failed: {e}"));
                }
            }
            outlet.send(disc).await;
        }
        Ok(())
    }
}

/// A lookup that got no answer: the disc goes on without it, so the step
/// was skipped rather than failed (its one item, the disc, left out).
pub fn lookup_skipped(cx: &Cx, disc: &Item, why: impl Into<String>) {
    cx.item(disc, ItemState::Skipped, "no answer");
    cx.status("No answer", why);
}

/// Drive's drive, shared with the simulated step.
pub fn drive_param() -> ParamSpec {
    ParamSpec::new("drive", "Drive", ParamKind::Text).on_card("Drive")
}

/// Drive's read offset, shared with the simulated step.
pub fn offset_param() -> ParamSpec {
    ParamSpec::new("offset", "Read offset", ParamKind::Number)
        .on_drive()
        .findable()
        .only_with(&["rip"])
        .on_card_as("Offset", &[], signed_offset)
        .help("How early or late this drive reads audio. Found for you, and needed to check each track.")
}

/// Rip's first read, shared with the simulated step.
pub fn first_read_param() -> ParamSpec {
    ParamSpec::new("first_read", "Ripping speed", ParamKind::Choice(&["fast", "careful"])).default("fast").on_card("Speed").labels(&["Fast, re-reads failed tracks", "Slow and careful"])
}

/// A read offset as the card shows it: with its sign (*+103*).
pub fn signed_offset(g: &dyn Fn(&str) -> Option<serde_json::Value>) -> String {
    match g("offset").and_then(|v| v.as_i64()) {
        Some(n) if n > 0 => format!("+{n}"),
        Some(n) => n.to_string(),
        None => String::new(),
    }
}

/// Cover art's description, shared with the simulated step. Its one
/// setting is about its own output: how big a picture to fetch, which is
/// the picture every encoder after it embeds.
pub fn cover_art_info() -> StepInfo {
    StepInfo::new("cover-art", "Cover art", Group::LookUp, ItemKind::AudioDisc, ItemKind::AudioDisc)
        .needs(&["release"])
        .right_after(&["choose"])
        .gives(&["cover"])
        .from("Cover Art Archive")
        .param(ParamSpec::new("size", "Cover size", ParamKind::Choice(&["1200 px", "500 px", "Original"])).default("1200 px").on_card("Cover size").labels(&[
            "Large (1200 px)",
            "Small (500 px)",
            "Original, as uploaded",
        ]))
        .summary(|g| match g("size").and_then(|v| v.as_str().map(str::to_owned)).as_deref() {
            Some("Original") => "original size".into(),
            Some(s) => format!("up to {s}"),
            None => String::new(),
        })
}

struct CoverArt;

#[async_trait]
impl Step for CoverArt {
    fn info(&self) -> StepInfo {
        cover_art_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            // The release Choose settled, on the disc.
            let mbid = disc.context().get(Subject::Disc, "musicbrainz_albumid").map(str::to_owned);
            match mbid {
                None => cx.status("Nothing found", "no album chosen"),
                Some(id) => {
                    cx.status("", "Fetching the front");
                    let dir = cx.work()?;
                    let got = {
                        let _net = cx.lane(LaneKind::Network("Cover Art Archive".into())).await;
                        musicbrainz::front_cover(&id, &cx.text("size"), &dir).await
                    };
                    match got {
                        Ok(Some((dest, bytes))) => {
                            disc.context_mut().add(Subject::Disc, "cover", dest.display().to_string(), "Cover Art Archive");
                            cx.made(&dest);
                            let _ = bytes;
                            cx.status(media::picture_size(&dest).map_or("Found".to_owned(), |(w, h)| format!("{w} × {h}")), "");
                        }
                        Ok(None) => cx.status("Nothing found", "the archive has no front cover"),
                        Err(e) => lookup_skipped(&cx, &disc, e.to_string()),
                    }
                }
            }
            outlet.send(disc).await;
        }
        Ok(())
    }
}

// ---- rip ----------------------------------------------------------------------------------------

/// Reads `count` sectors from `lba` on a drive and throws them away: what
/// clears its cache before a re-read.
pub type Flush = std::sync::Arc<dyn Fn(&str, u32, u32) -> std::io::Result<()> + Send + Sync>;

pub struct Rip {
    flush: Flush,
}

impl Rip {
    /// Rip, clearing the drive's cache with `flush` before every re-read:
    /// [`flush::read_sectors`] on a real drive, a stand-in in tests.
    pub fn with_flush(flush: Flush) -> Self {
        Rip { flush }
    }
}

/// Time left for a disc's first reads: audio still to read over how fast
/// it has read so far.
struct Eta {
    started: std::time::Instant,
    total: f32,
    read: std::sync::Mutex<f32>,
}

impl Eta {
    fn add(&self, secs: f32) {
        *self.read.lock().unwrap() += secs;
    }

    /// Seconds left, once there's enough to go on.
    fn left(&self, in_progress: f32) -> Option<f32> {
        let done = *self.read.lock().unwrap() + in_progress;
        let took = self.started.elapsed().as_secs_f32();
        (done > 20.0 && took > 5.0).then(|| (self.total - done).max(0.0) * took / done)
    }
}

/// "about 4 min", "40 s".
fn mins(secs: f32) -> String {
    if secs >= 90.0 {
        format!("about {} min", (secs / 60.0).round() as u32)
    } else {
        format!("{} s", ((secs / 10.0).round() * 10.0).max(10.0) as u32)
    }
}

/// How one track's reads went.
#[derive(Default)]
struct Reads {
    crcs: Vec<String>,
    /// For each read, as `crcs`: whether it surely came off the disc. The
    /// first read does; a re-read only if the drive's cache was cleared
    /// before it.
    fresh: Vec<bool>,
    last: PathBuf,
    /// Every read's file, so two can be compared sample by sample.
    files: Vec<PathBuf>,
    /// The files of the reads that came off the disc: the ones that vote.
    voters: Vec<PathBuf>,
    /// The drive refused to read slower, and read at its own speed.
    slower_refused: bool,
}

/// Where two reads of the same track differ: how many sample frames, and
/// the first and last second they're in. On a scratched disc this is a few
/// hundred frames in one stretch, which says far more than "the checksums
/// disagree".
pub fn differences(a: &Path, b: &Path) -> Result<Option<(usize, u32, u32)>> {
    fn pcm(p: &Path) -> Result<Vec<u8>> {
        let bytes = std::fs::read(p)?;
        let r = wav_data(&bytes).ok_or_else(|| anyhow!("{} isn't a WAV", p.display()))?;
        Ok(bytes[r].to_vec())
    }
    let (a, b) = (pcm(a)?, pcm(b)?);
    let frames = a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0).enumerate().filter(|(_, (x, y))| x != y).map(|(i, _)| i);
    let (mut n, mut first, mut last) = (0usize, usize::MAX, 0usize);
    for i in frames {
        n += 1;
        first = first.min(i);
        last = i;
    }
    Ok((n > 0).then_some((n, (first / 44_100) as u32, (last / 44_100) as u32)))
}

/// Where a WAV's samples are: its `data` chunk, found by walking the
/// chunks. Not by looking for the word: cyanrip writes the disc's CD-Text
/// into a LIST chunk before it, and a title like "Metadata" would put the
/// samples a few bytes off.
pub fn wav_data(b: &[u8]) -> Option<std::ops::Range<usize>> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12;
    while at + 8 <= b.len() {
        let size = u32::from_le_bytes([b[at + 4], b[at + 5], b[at + 6], b[at + 7]]) as usize;
        if &b[at..at + 4] == b"data" {
            return Some(at + 8..(at + 8).saturating_add(size).min(b.len()));
        }
        // Checked: a chunk's size is whatever the file says, up to 4 GB.
        at = at.checked_add(8)?.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}

/// The majority value of every sample across several reads of one track,
/// written as a WAV like the first. Gives how many sample frames the reads
/// disagreed on, and how many of those had no majority (then the most
/// common value is kept).
pub fn combine(files: &[PathBuf], out: &Path) -> Result<(usize, usize)> {
    if files.len() < 2 {
        bail!("needs at least two reads");
    }
    let raw: Vec<Vec<u8>> = files.iter().map(std::fs::read).collect::<std::io::Result<_>>()?;
    let ranges: Vec<std::ops::Range<usize>> = raw.iter().map(|b| wav_data(b).ok_or_else(|| anyhow!("not a WAV"))).collect::<Result<_>>()?;
    let at = ranges[0].start;
    let frames: Vec<Vec<u32>> = raw.iter().zip(&ranges).map(|(b, r)| b[r.clone()].as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()).collect();
    let n = frames.iter().map(Vec::len).min().unwrap_or(0);
    let mut voted = frames[0][..n].to_vec();
    let (mut contested, mut unsettled) = (0, 0);
    let mut counts: Vec<(u32, usize)> = Vec::with_capacity(frames.len());
    for i in 0..n {
        if frames.iter().all(|f| f[i] == frames[0][i]) {
            continue;
        }
        contested += 1;
        counts.clear();
        for f in &frames {
            match counts.iter_mut().find(|(v, _)| *v == f[i]) {
                Some((_, c)) => *c += 1,
                None => counts.push((f[i], 1)),
            }
        }
        // On a tie, the earliest read's value.
        let (best, c) = counts.iter().copied().fold(counts[0], |m, x| if x.1 > m.1 { x } else { m });
        if c * 2 <= frames.len() {
            unsettled += 1;
        }
        voted[i] = best;
    }
    let mut bytes = raw[0][..at].to_vec();
    bytes.extend(voted.iter().flat_map(|v| v.to_le_bytes()));
    std::fs::write(out, bytes)?;
    Ok((contested, unsettled))
}

impl Reads {
    /// The difference between the last two reads, as a note for the item.
    fn damage(&self) -> String {
        let real: Vec<&PathBuf> = self.files.iter().rev().take(2).collect();
        match real.as_slice() {
            [a, b] => match differences(a, b) {
                Ok(Some((n, from, to))) => format!("{n} samples differ, {}–{}", mmss(from), mmss(to.max(from))),
                _ => "reads disagree".into(),
            },
            _ => "reads disagree".into(),
        }
    }

    /// Two reads with the same checksum, the later one read after the
    /// drive's cache was cleared: it can't be the drive handing back what
    /// it read the time before.
    fn agree(&self) -> bool {
        self.crcs.iter().enumerate().any(|(j, b)| !b.is_empty() && self.fresh.get(j).copied().unwrap_or(false) && self.crcs[..j].contains(b))
    }

    /// Two reads with the same checksum, cache cleared or not.
    fn same_checksum(&self) -> bool {
        self.crcs.iter().enumerate().any(|(j, b)| !b.is_empty() && self.crcs[..j].contains(b))
    }

    /// Why a track isn't settled, as its note.
    fn unsettled(&self) -> String {
        if self.crcs.iter().all(String::is_empty) {
            "read failed".to_owned()
        } else if self.same_checksum() {
            NOT_CLEARED.to_owned()
        } else {
            self.damage()
        }
    }
}

#[async_trait]
impl Step for Rip {
    fn info(&self) -> StepInfo {
        StepInfo::new("rip", "Rip", Group::Read, ItemKind::AudioDisc, ItemKind::Track)
            // Something has come off the disc: what Next disc needs.
            .gives(&["read"])
            .from("read each track and check it")
            .param(first_read_param())
            .param(ParamSpec::new("tracks", "Tracks to rip", ParamKind::Text).default("all").help("all, or track numbers such as 3, 5."))
            .retry("two_reads", "Read again", "~1 min")
            .retry("combine", "Best of 3 reads", "~2 min")
            .retry("slowly", "Read again slowly", "~2 min")
            .retry("keep", "Keep it anyway", "")
            .retry("leave_out", "Skip this track", "")
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let Some(mut disc) = inlet.recv().await else { return Ok(()) };
        let dev = device(disc.context())?;
        let n = disc.context().parts().max(1);
        // From the drive, which put it on the disc. Never a silent zero:
        // reading at the wrong offset gives a clean rip AccurateRip can't
        // confirm, which is exactly what happened to every earlier rip of
        // the Tortoise disc.
        let offset = disc.context().get(Subject::Disc, "offset").and_then(|v| v.parse::<i64>().ok());
        let Some(offset) = offset else { bail!("the drive's read offset isn't known") };
        let offset = Some(offset);
        let first = if cx.text("first_read") == "careful" { 3 } else { 0 };
        let dir = cx.work()?.join("rip");
        // "all", or a list like "3,5".
        let only: Vec<u32> = cx.text("tracks").split(',').filter_map(|t| t.trim().parse().ok()).collect();
        let mut wanted: Vec<u32> = (1..=n).filter(|t| only.is_empty() || only.contains(t)).collect();
        let opts = |paranoia, speed| RipOptions { device: &dev, offset, paranoia, speed };
        let _drive = cx.lane(LaneKind::Drive).await;
        // Audio hidden before track 1, in its pregap: read once to see
        // whether there's anything there. Digital silence isn't kept; audio
        // is track 0, read and settled like the others from here on.
        let mut hidden_first: Option<(Reads, Result<Verdict>)> = None;
        let pregap = disc.context().get(Subject::Disc, "toc").and_then(hidden_track_frames);
        if let Some(frames) = pregap.filter(|_| only.is_empty() || only.contains(&0)) {
            let mut probe = Item::track(0, HIDDEN).knowing(&disc);
            probe.context_mut().add(Subject::Part(0), "length", (frames / 44_100).to_string(), "Rip");
            // No row of its own unless there's something there.
            rip_says(&cx, "Checking the gap before track 1");
            let eta0 = Eta { started: std::time::Instant::now(), total: 0.0, read: std::sync::Mutex::new(0.0) };
            let mut reads = Reads::default();
            let first_read = self.read(&cx, &opts(first, None), &mut probe, n, &dir, &mut reads, &eta0, "reading").await;
            if cx.stopped() {
                return Ok(());
            }
            match first_read.as_ref().map(|_| is_silent(&reads.last)) {
                Ok(Ok(true)) => {
                    cx.log(format!("track 1's pregap ({} s) is digital silence: not kept", frames / 44_100));
                    // Silence: nothing was found, so it isn't a track; the
                    // row the read made goes, and no count includes it.
                    let id = probe.id.clone();
                    cx.view(|v| v.items.retain(|i| i.id != id));
                }
                _ => {
                    wanted.insert(0, 0);
                    disc.context_mut().add(Subject::Part(0), "length", (frames / 44_100).to_string(), "Rip");
                    for f in probe.context().facts().iter().filter(|f| f.subject == Subject::Part(0) && f.key != "length") {
                        disc.context_mut().add(Subject::Part(0), &f.key, f.value.clone(), "Rip");
                    }
                    hidden_first = Some((reads, first_read));
                }
            }
        }
        disc.context_mut().add(Subject::Disc, "selected", wanted.len().to_string(), "Rip");
        // Each track knows what the disc knew, the chosen release with it.
        let mut items: Vec<Item> = wanted
            .iter()
            .map(|&t| Item::track(t, if t == 0 { HIDDEN.to_owned() } else { disc.context().get(Subject::Part(t), "title").map(str::to_owned).unwrap_or_else(|| format!("Track {t}")) }).knowing(&disc))
            .collect();
        for i in &items {
            cx.item(i, ItemState::Waiting, "");
        }
        let total = items.len();

        let eta = Eta {
            started: std::time::Instant::now(),
            total: wanted.iter().filter_map(|&t| disc.context().get(Subject::Part(t), "length").and_then(|l| l.parse::<f32>().ok())).sum(),
            read: std::sync::Mutex::new(0.0),
        };
        let mut bad: Vec<(Item, Reads)> = Vec::new();
        let mut done = 0u32;
        for item in items.iter_mut() {
            let t = item.index.unwrap();
            rip_says(&cx, track_line(t, &item.label));
            // The hidden track's first read is the one that found it.
            let (mut reads, verdict) = match hidden_first.take().filter(|_| t == 0) {
                Some(r) => r,
                None => {
                    let mut reads = Reads::default();
                    // First read, fast unless asked otherwise.
                    let v = self.read(&cx, &opts(first, None), item, n, &dir, &mut reads, &eta, "reading").await;
                    (reads, v)
                }
            };
            let verdict = match verdict {
                // A match is proof; nothing more to do.
                Ok(Verdict::Accurate { confidence }) => Some(format!("✓ {confidence}")),
                Ok(_) | Err(_) => {
                    // No match, nothing to compare with, or a failed read: read
                    // again carefully. Two reads that agree are consistent; a
                    // fast read's "no errors" on its own says nothing.
                    let v = self.read(&cx, &opts(3, None), item, n, &dir, &mut reads, &eta, "reading carefully").await;
                    settled_note(accurate(&v), reads.agree())
                }
            };
            match verdict {
                Some(note) => {
                    rip_verdict(&cx, item, ItemState::Ok, note);
                    done += 1;
                    outlet.send(item.becomes(ItemKind::Track, Some(reads.last.clone()))).await;
                }
                None => {
                    let note = reads.unsettled();
                    cx.log(format!("track {t}: {note}"));
                    rip_verdict(&cx, item, ItemState::Bad, note);
                    bad.push((item.clone(), reads));
                }
            }
            cx.progress(Some(t as f32 / n as f32));
        }
        cx.drive_says(StepState::Idle, "Waiting", "");

        // Whether this drive reads at one speed only: cyanrip says so when
        // it reads the disc, or a slow read finds out.
        let mut no_slower = disc.context().get(Subject::Disc, "fixed_speed") == Some("1");
        while !bad.is_empty() {
            cx.state(StepState::Bad);
            let notes: Vec<(u32, String)> = bad
                .iter()
                .map(|(i, _)| (i.index.unwrap(), i.context().get(Subject::Part(i.index.unwrap()), "verdict").and_then(|v| v.split_once(": ")).map(|(_, n)| n.to_owned()).unwrap_or_default()))
                .collect();
            rip_says(&cx, rip_needs(&notes));
            let Some(req) = cx.next_retry().await else { return Ok(()) };
            let Some(pos) = bad.iter().position(|(i, _)| i.id == req.item) else { continue };
            let (mut item, mut reads) = bad.remove(pos);
            let settled = match req.option.as_str() {
                "keep" if !reads.last.as_os_str().is_empty() => {
                    rip_verdict(&cx, &mut item, ItemState::Ok, KEPT);
                    true
                }
                "leave_out" | "keep" => {
                    rip_verdict(&cx, &mut item, ItemState::Skipped, "skipped");
                    done += 1;
                    continue;
                }
                "combine" => {
                    // Enough careful reads for a vote, then the majority of
                    // every sample. Better than any single read of a
                    // scratch, though not proof the way a match is.
                    cx.state(StepState::Busy);
                    let mut tries = 0;
                    while more_reads_for_vote(reads.voters.len(), tries) {
                        tries += 1;
                        if self.read(&cx, &opts(3, None), &mut item, n, &dir, &mut reads, &eta, "reading again (best of 3)").await.is_err() && cx.stopped() {
                            return Ok(());
                        }
                    }
                    cx.drive_says(StepState::Idle, "Waiting", "");
                    if reads.voters.len() < VOTE {
                        // Reads that may have been the drive's memory don't vote.
                        rip_verdict(&cx, &mut item, ItemState::Bad, if reads.files.len() >= VOTE { NOT_CLEARED } else { "read failed" });
                        bad.push((item, reads));
                        continue;
                    }
                    let out = dir.join(format!("{:02}-combined.wav", item.index.unwrap()));
                    match combine(&reads.voters, &out) {
                        Ok((_, 0)) => {
                            rip_verdict(&cx, &mut item, ItemState::Ok, format!("✓ best of {} reads", reads.voters.len()));
                            reads.last = out;
                            true
                        }
                        Ok((_, unsettled)) => {
                            rip_verdict(&cx, &mut item, ItemState::Ok, format!("best of {} reads, {unsettled} samples uncertain", reads.voters.len()));
                            reads.last = out;
                            true
                        }
                        Err(e) => {
                            rip_verdict(&cx, &mut item, ItemState::Bad, format!("couldn't combine: {e}"));
                            false
                        }
                    }
                }
                option => {
                    cx.state(StepState::Busy);
                    let slowly = option == "slowly";
                    // A drive that won't change speed reads at its own: the
                    // disc says so from the start, or the first try did.
                    if slowly && no_slower {
                        cx.log(format!("track {}: {NO_SLOWER}", item.index.unwrap()));
                    }
                    let mut speed = (slowly && !no_slower).then_some(4);
                    let what = if slowly && no_slower { "re-reading at the drive's usual speed" } else { "re-reading" };
                    // Two more careful reads, until any two agree. A scratch
                    // the drive reads differently every time won't converge,
                    // so this stops early rather than spending the evening.
                    let mut note = None;
                    for _ in 0..2 {
                        let v = self.read(&cx, &opts(3, speed), &mut item, n, &dir, &mut reads, &eta, what).await;
                        if reads.slower_refused {
                            no_slower = true;
                            speed = None;
                        }
                        note = settled_note(accurate(&v), reads.agree());
                        if note.is_some() {
                            break;
                        }
                    }
                    cx.drive_says(StepState::Idle, "Waiting", "");
                    match note {
                        Some(note) => {
                            rip_verdict(&cx, &mut item, ItemState::Ok, note);
                            true
                        }
                        None => {
                            rip_verdict(&cx, &mut item, ItemState::Bad, reads.unsettled());
                            false
                        }
                    }
                }
            };
            if settled {
                done += 1;
                outlet.send(item.becomes(ItemKind::Track, Some(reads.last.clone()))).await;
            } else {
                bad.push((item, reads));
            }
        }
        cx.state(StepState::Busy);
        let _ = (done, total);
        rip_done(&cx);
        Ok(())
    }
}

/// How many reads *Best of 3 reads* votes over.
const VOTE: usize = 3;

/// The hidden track's name, as its row and its file are called.
pub const HIDDEN: &str = "Hidden track";

/// How many sample frames of track 1's pregap the TOC gives, if any: track
/// 1 starting later than the usual two seconds (150 frames) leaves room
/// before it, from the disc's first sector.
pub fn hidden_track_frames(toc: &str) -> Option<u64> {
    let first = *gnudb::parse_toc(toc)?.offsets.first()?;
    (first > 150).then(|| u64::from(first - 150) * 588)
}

/// A read of the hidden track is whole only if cyanrip and the file both
/// hold exactly the TOC's length of it.
pub fn hidden_track_is_whole(file: &Path, samples: u64, want: u64) -> Result<()> {
    let bytes = std::fs::read(file)?;
    let got = wav_data(&bytes).map_or(0, |r| r.len() as u64 / 4);
    if want == 0 || samples != want || got != want {
        bail!("the hidden track should be {want} samples; cyanrip read {samples} and the file holds {got}");
    }
    Ok(())
}

/// Whether a WAV's samples are all zero: digital silence.
pub fn is_silent(file: &Path) -> Result<bool> {
    let bytes = std::fs::read(file)?;
    let r = wav_data(&bytes).ok_or_else(|| anyhow!("{} isn't a WAV", file.display()))?;
    Ok(bytes[r].iter().all(|&b| b == 0))
}

/// Whether *Best of 3 reads* should read again: until it has a vote's worth of
/// reads, but never more than twice that many tries, so a track the drive
/// can't read at all stops instead of holding the drive for ever.
pub fn more_reads_for_vote(files: usize, tries: usize) -> bool {
    files < VOTE && tries < 2 * VOTE
}

/// AccurateRip's confidence, when a read matched it.
fn accurate(v: &Result<Verdict>) -> Option<u32> {
    match v {
        Ok(Verdict::Accurate { confidence }) => Some(*confidence),
        _ => None,
    }
}

/// How a track was settled after a careful read, if it was: a match with
/// AccurateRip says so (and counts as verified) even when the reads agree
/// too; otherwise two agreeing reads.
pub fn settled_note(accurate: Option<u32>, agree: bool) -> Option<String> {
    match accurate {
        Some(c) => Some(format!("✓ {c}")),
        None if agree => Some("✓ two reads agree".to_owned()),
        None => None,
    }
}

/// A track's note when its reads agree but the drive's cache couldn't be
/// cleared before them, so they may be the drive's memory, not the disc.
pub const NOT_CLEARED: &str = "the drive's cache couldn't be cleared";

/// What Rip says when *Read again slowly* meets a drive that won't change
/// speed.
pub const NO_SLOWER: &str = "this drive won't read slower; read at its usual speed";

/// A track's note once *Keep it anyway* kept it unproven.
pub const KEPT: &str = "kept anyway, not verified";

/// Whether a note is a track kept unproven: *Keep it anyway*, or *kept,
/// unverified* as earlier runs said.
pub fn is_kept(note: &str) -> bool {
    let n = note.trim();
    n == KEPT || n.contains("unverified")
}

/// Whether a verdict note is AccurateRip's match (*✓ 12*), the only proof.
pub fn is_accurate(note: &str) -> bool {
    note.strip_prefix("✓ ").is_some_and(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()))
}

impl Rip {
    /// One read of one track, into its own file so earlier reads survive.
    #[allow(clippy::too_many_arguments)]
    async fn read(&self, cx: &Cx, o: &RipOptions<'_>, item: &mut Item, n: u32, dir: &Path, reads: &mut Reads, eta: &Eta, what: &str) -> Result<Verdict> {
        let t = item.index.unwrap();
        let attempt = reads.crcs.len() + 1;
        cx.item(item, ItemState::Busy, what);
        cx.drive_says(StepState::Busy, if attempt == 1 { "Reading" } else { "Re-reading" }, &format!("track {t}"));
        let into = dir.join(format!("{t:02}-{attempt}"));
        let len = item.context().get(Subject::Part(t), "length").and_then(|l| l.parse::<f32>().ok()).unwrap_or(0.0);
        let first = attempt == 1;
        // Every read after the first is compared with the ones before, so
        // the drive must not hand back what it still holds of them.
        let fresh = first || self.clear_cache(cx, o.device, item, t).await;
        let cache = match (first, fresh) {
            (true, _) => "",
            (false, true) => ", drive's cache cleared",
            (false, false) => ", drive's cache not cleared",
        };
        cx.item(item, ItemState::Busy, what);
        let shown = &*item;
        let mut o = RipOptions { ..*o };
        let r = loop {
            let r = cyanrip::rip_track(&o, t, &into, cx.cancelled(), |p| {
                cx.item(shown, ItemState::Busy, format!("{what} {}%", (p * 100.0) as u32));
                cx.progress(Some((t.saturating_sub(1) as f32 + p) / n as f32));
                // Time left for the whole disc, from how fast it's reading.
                if first {
                    if let Some(left) = eta.left(len * p) {
                        cx.view(|v| v.detail = format!("Track {t} · {} · {} left", shown.label, mins(left)));
                    }
                }
            })
            .await;
            match r {
                // The drive won't change speed: read at its own, and say so.
                Err(e) if o.speed.is_some() && e.is::<cyanrip::SpeedRefused>() => {
                    cx.log(format!("track {t}: {NO_SLOWER}"));
                    reads.slower_refused = true;
                    o.speed = None;
                }
                r => break r,
            }
        };
        let o = &o;
        if first {
            eta.add(len);
        }
        // The hidden track: exactly as long as the TOC says, and never
        // AccurateRip's to judge (it holds no checksum for it).
        let r = r.and_then(|mut rip| {
            if t == 0 {
                let want = item.context().get(Subject::Disc, "toc").and_then(hidden_track_frames).unwrap_or(0);
                hidden_track_is_whole(&rip.file, rip.samples, want)?;
                rip.verdict = Verdict::NotChecked;
            }
            Ok(rip)
        });
        match r {
            Ok(rip) => {
                // The disc's flag, once: the samples are as the disc holds them.
                if rip.preemphasis && item.context().get(Subject::Part(t), "preemphasis").is_none() {
                    item.context_mut().add(Subject::Part(t), "preemphasis", "1", "Rip");
                }
                cx.log(format!("track {t} read {attempt} (paranoia {}{cache}): {:?}, CRC {}", o.paranoia, rip.verdict, rip.crc));
                item.context_mut().add(Subject::Part(t), "read", format!("read {attempt}, paranoia {}{cache}: {:?}, CRC {}", o.paranoia, rip.verdict, rip.crc), "Rip");
                reads.crcs.push(rip.crc);
                reads.fresh.push(fresh);
                if fresh {
                    reads.voters.push(rip.file.clone());
                }
                reads.files.push(rip.file.clone());
                reads.last = rip.file;
                Ok(rip.verdict)
            }
            Err(e) => {
                if cx.stopped() {
                    return Err(e);
                }
                cx.log(format!("track {t} read {attempt}: {e}"));
                item.context_mut().add(Subject::Part(t), "read", format!("read {attempt}, paranoia {}{cache}: failed: {e}", o.paranoia), "Rip");
                reads.crcs.push(String::new());
                reads.fresh.push(fresh);
                Err(e)
            }
        }
    }
}

impl Rip {
    /// Clears the drive's cache before re-reading track `t`: reads
    /// [`flush::SECTORS`] from elsewhere on the disc and throws them away.
    /// Whether it did; if not, the log says why, and the read that follows
    /// can't settle the track by agreeing.
    async fn clear_cache(&self, cx: &Cx, device: &str, item: &Item, t: u32) -> bool {
        let toc = item.context().get(Subject::Disc, "toc").unwrap_or("").to_owned();
        let Some(w) = flush::window(&toc, t) else {
            cx.log(format!("track {t}: couldn't clear the drive's cache: the disc is too short to read far enough from this track"));
            return false;
        };
        cx.item(item, ItemState::Busy, "clearing the drive's cache");
        let (f, dev) = (self.flush.clone(), device.to_owned());
        let r = match tokio::task::spawn_blocking(move || f(&dev, w.lba, w.count)).await {
            Ok(r) => r,
            Err(e) => Err(std::io::Error::other(e.to_string())),
        };
        match r {
            Ok(()) => {
                cx.log(format!("track {t}: cleared the drive's cache: read {} sectors from {}", flush::thousands(w.count), flush::tracks_in(&toc, &w)));
                true
            }
            Err(e) => {
                cx.log(format!("track {t}: couldn't clear the drive's cache ({e}); reading again anyway, but this read can't settle the track by agreeing"));
                false
            }
        }
    }
}

/// What Rip says about a track, on its row and on the track itself, where
/// the rip log and Notify read it: the last one is the track's verdict.
pub fn rip_verdict(cx: &Cx, item: &mut Item, state: ItemState, note: impl Into<String>) {
    let note = note.into();
    if let Some(t) = item.index {
        let word = match state {
            ItemState::Ok => "ok",
            ItemState::Skipped => "skipped",
            _ => "bad",
        };
        item.context_mut().add(Subject::Part(t), "verdict", format!("{word}: {note}"), "Rip");
    }
    // Verified only by AccurateRip's match: "✓ 103", its confidence.
    if state == ItemState::Ok && is_accurate(&note) {
        cx.verified(item, note);
    } else {
        cx.item(item, state, note);
    }
}

/// Rip's outcome, from its own items: *16/17 verified*, and *1 kept* when
/// some were settled another way; with what it's on, or the one fact.
/// *Track 6 · Cannon*, or just *Track 6* when it has no title of its own.
pub fn track_line(n: impl std::fmt::Display, label: &str) -> String {
    let n = n.to_string();
    if label.is_empty() || label == format!("Track {n}") {
        format!("Track {n}")
    } else {
        format!("Track {n} · {label}")
    }
}

pub fn rip_says(cx: &Cx, detail: impl Into<String>) {
    let detail = detail.into();
    cx.view(|v| {
        (v.status, v.qualifier) = rip_outcome(&v.items);
        v.detail = detail;
    });
}

/// Counting only real verifications (AccurateRip's): *16/17 verified*,
/// and how the rest were settled, as one qualifier.
pub fn rip_outcome(items: &[drt_engine::ItemView]) -> (String, String) {
    let total = items.iter().filter(|i| i.state != ItemState::Skipped).count();
    let verified = items.iter().filter(|i| i.verified).count();
    let kept = items.iter().filter(|i| i.state == ItemState::Ok && is_kept(&i.note)).count();
    let other = items.iter().filter(|i| i.state == ItemState::Ok && !i.verified && !is_kept(&i.note)).count();
    let q = match (kept, other) {
        (0, 0) => String::new(),
        (k, 0) => format!("{k} kept"),
        (0, o) => format!("{o} by matching reads"),
        (k, o) => format!("{} not verified", k + o),
    };
    (format!("{verified}/{total} verified"), q)
}

/// What Rip says when it's done: which tracks were kept unverified.
pub fn rip_done(cx: &Cx) {
    let mut detail = String::new();
    cx.view(|v| {
        let kept: Vec<u32> = v.items.iter().filter(|i| i.state == ItemState::Ok && is_kept(&i.note)).filter_map(track_no).collect();
        detail = match kept.as_slice() {
            [] => String::new(),
            [t] => format!("track {t} kept, not verified"),
            ts => format!("tracks {} kept, not verified", ts.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")),
        };
    });
    rip_says(cx, detail);
}

/// What needs you on Rip: *Track 9 doesn't match AccurateRip*, or how many.
pub fn rip_needs(bad: &[(u32, String)]) -> String {
    match bad {
        [] => String::new(),
        [(t, note)] => {
            let why = if note == "no match" {
                "doesn't match AccurateRip"
            } else if note == "read failed" || note.is_empty() {
                "couldn't be read"
            } else if note == NOT_CLEARED {
                "couldn't be checked"
            } else {
                "reads disagree"
            };
            format!("Track {t} {why}")
        }
        b => format!("{} tracks to decide", b.len()),
    }
}

/// A track's number, from its item's id (`track-9`).
fn track_no(i: &drt_engine::ItemView) -> Option<u32> {
    i.id.rsplit('-').next()?.parse().ok()
}

// ---- eject ----------------------------------------------------------------------------------------

/// Ends the disc and leads back to the start: opens the tray once the last
/// track is read, and the recipe goes round again for the next disc. On
/// its own line from Rip, so it doesn't wait for encoding or a hold.
pub struct NextDisc {
    pub real: bool,
}

#[async_trait]
impl Step for NextDisc {
    fn info(&self) -> StepInfo {
        StepInfo::new("next-disc", "Next disc", Group::Source, ItemKind::Any, ItemKind::Nothing)
            .from("open the tray, start on the next disc")
            .loops_back()
            // Only after the disc has been read: straight after the Drive it
            // would open and close the tray for ever.
            .needs(&["read"])
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, cx: Cx) -> Result<()> {
        let all = inlet.all().await;
        let n = all.len();
        if cx.stopped() {
            return Ok(());
        }
        if self.real {
            let dev = all.iter().find_map(|i| i.context().get(Subject::Disc, "device").map(str::to_owned));
            match dev {
                Some(d) => {
                    // A few tries: a drive just done reading can refuse at
                    // first. If it never opens, taking the disc out by hand
                    // does the same.
                    let mut opened = Err(std::io::Error::other("not tried"));
                    for attempt in 0..3 {
                        if attempt > 0 {
                            cx.sleep(2000).await?;
                        }
                        opened = eject(&d);
                        if opened.is_ok() {
                            break;
                        }
                    }
                    match opened {
                        Ok(()) => cx.status("Ejected", "waiting for the next"),
                        Err(e) => {
                            cx.log(format!("the tray didn't open after 3 tries: {e}"));
                            while media::disc_in(&d) {
                                cx.state(StepState::Needs);
                                cx.status("Tray didn't open", "Take the disc out to go on");
                                cx.sleep(1000).await?;
                            }
                            cx.state(StepState::Busy);
                            cx.status("Disc out", "waiting for the next");
                        }
                    }
                }
                // Nothing was read: no disc to take out, and starting
                // again would only find the same empty drive.
                None => bail!("no disc was read, so it doesn't start on another"),
            }
        } else {
            cx.sleep(500).await?;
            cx.status("Ejected", "waiting for the next");
        }
        cx.log(format!("ejected after {n} tracks"));
        cx.drive_says(StepState::Idle, "Empty", "");
        Ok(())
    }
}

/// `CDROMEJECT`, after unlocking the door. No external program: Bluefin
/// has no `eject`.
#[cfg(target_os = "linux")]
pub fn eject(device: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    const CDROM_LOCKDOOR: libc::c_ulong = 0x5329;
    const CDROMEJECT: libc::c_ulong = 0x5309;
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(device)?;
    // SAFETY: two argument-less ioctls (an int for the lock) on a descriptor we hold open.
    let r = unsafe {
        libc::ioctl(f.as_raw_fd(), CDROM_LOCKDOOR, 0);
        libc::ioctl(f.as_raw_fd(), CDROMEJECT)
    };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
pub fn eject(_device: &str) -> std::io::Result<()> {
    Err(std::io::Error::other("can't open the tray here"))
}

// ---- folders ---------------------------------------------------------------------------------------

struct Place {
    kind: &'static str,
    name: &'static str,
}

/// What a track's path template can use, from what the track knows: the
/// chosen release's facts first.
pub fn fields_in(c: &Context, t: u32, fallback_title: &str) -> template::Fields {
    {
        let get = |s, k| c.get(s, k).unwrap_or("").to_owned();
        let albumartist = c.get(Subject::Disc, "album_artist").unwrap_or("Unknown artist").to_owned();
        template::Fields {
            artist: c.get(Subject::Part(t), "artist").map(str::to_owned).unwrap_or_else(|| albumartist.clone()),
            albumartist,
            album: c.get(Subject::Disc, "album").unwrap_or("Unknown album").to_owned(),
            title: c.get(Subject::Part(t), "title").map(str::to_owned).unwrap_or_else(|| fallback_title.to_owned()),
            track: t,
            tracktotal: c.parts(),
            year: get(Subject::Disc, "year"),
            disc: get(Subject::Disc, "disc"),
            label: get(Subject::Disc, "label"),
        }
    }
}

pub use crate::template::safe;

/// A Folder's outcome: *17 filed*, with what it's filing now
/// (*04 - Suzy Lee.flac*) or, once done, where (*The White Stripes / The
/// White Stripes*).
pub fn filed_says(cx: &Cx, done: usize, detail: impl Into<String>) {
    cx.status(format!("{done} filed"), detail);
}

/// Where a file went, under the folder: *The White Stripes / The White Stripes*.
pub fn where_filed(root: &Path, dest: &Path) -> String {
    let dir = dest.parent().unwrap_or(dest);
    let rel = dir.strip_prefix(root).unwrap_or(dir);
    rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join(" / ")
}

/// A file's name, for the detail line while it's filed.
pub fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `~/x` as a path.
pub fn expand(folder: &str) -> PathBuf {
    match folder.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => PathBuf::from(folder),
    }
}

#[async_trait]
impl Step for Place {
    fn info(&self) -> StepInfo {
        data::place_info(self.kind, self.name)
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let folder = cx.text("folder");
        let names = cx.text("names");
        let root = expand(&folder);
        let mut done = 0;
        let mut last = String::new();
        // Files whose name is taken by a different file: they wait here for
        // one question, once everything else is filed.
        let mut taken: Vec<(Item, PathBuf, PathBuf)> = Vec::new();
        while let Some(file) = inlet.recv().await {
            let src = file.path.clone().ok_or_else(|| anyhow!("{} has no file", file.label))?;
            // A DVD or Blu-ray title, named for Jellyfin unless the template says otherwise.
            if file.id.starts_with(video::ITEM_PREFIX) {
                let _f = cx.lane(LaneKind::File(folder.clone())).await;
                cx.item(&file, ItemState::Busy, "copying");
                filed_says(&cx, done, file.label.clone());
                match video::place(&names, &root, &file).await {
                    Ok(dest) => {
                        done += 1;
                        cx.item(&file, ItemState::Ok, dest.display().to_string());
                        last = where_filed(&root, &dest);
                        filed_says(&cx, done, "");
                        outlet.send(file.becomes(file.kind, Some(dest))).await;
                    }
                    Err(e) => {
                        cx.item(&file, ItemState::Bad, e.to_string());
                        cx.log(format!("couldn't file {}: {e}", file.label));
                    }
                }
                continue;
            }
            // A disc image: its files together, in the folder its template names.
            if file.id == data::DUMP {
                let dir = root.join(data::dump_folder(file.context(), &names));
                let _f = cx.lane(LaneKind::File(folder.clone())).await;
                cx.item(&file, ItemState::Busy, "copying");
                filed_says(&cx, done, file.label.clone());
                match data::place_dump(&src, &dir).await {
                    Ok(dest) => {
                        done += 1;
                        cx.item(&file, ItemState::Ok, dir.display().to_string());
                        last = where_filed(&root, &dir.join("x"));
                        filed_says(&cx, done, "");
                        outlet.send(file.becomes(file.kind, Some(dest))).await;
                    }
                    Err(e) => {
                        cx.item(&file, ItemState::Bad, e.to_string());
                        cx.log(format!("couldn't file {}: {e}", file.label));
                    }
                }
                continue;
            }
            let total = parts(file.context());
            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
            let dest = match file.index {
                Some(t) => {
                    let rel = template::render(&names, &fields_in(file.context(), t, &file.label));
                    root.join(format!("{}.{ext}", rel.display()))
                }
                // A file that isn't a track (a cue sheet) goes beside the
                // tracks, under its own name.
                None => {
                    let rel = template::render(&names, &fields_in(file.context(), 1, ""));
                    let dir = rel.parent().map(|p| root.join(p)).unwrap_or_else(|| root.clone());
                    dir.join(src.file_name().unwrap_or_default())
                }
            };
            let _f = cx.lane(LaneKind::File(folder.clone())).await;
            // Never over a different file without asking.
            match already_there(&src, &dest).await {
                Ok(There::Nothing) => {}
                Ok(There::Same) => {
                    done += 1;
                    cx.item(&file, ItemState::Ok, format!("{} (already there, the same)", dest.display()));
                    last = where_filed(&root, &dest);
                    filed_says(&cx, done, "");
                    cx.progress(Some(done as f32 / total as f32));
                    outlet.send(file.becomes(file.kind, Some(dest))).await;
                    continue;
                }
                Ok(There::Different) => {
                    cx.item(&file, ItemState::Waiting, "a different file has this name");
                    taken.push((file, src, dest));
                    continue;
                }
                Err(e) => {
                    cx.item(&file, ItemState::Bad, e.to_string());
                    cx.log(format!("couldn't file {}: {e}", file.label));
                    continue;
                }
            }
            cx.item(&file, ItemState::Busy, "copying");
            filed_says(&cx, done, file_name(&dest));
            match copy_in(&src, &dest).await {
                Ok(()) => {
                    done += 1;
                    cx.item(&file, ItemState::Ok, dest.display().to_string());
                    last = where_filed(&root, &dest);
                    filed_says(&cx, done, "");
                    cx.progress(Some(done as f32 / total as f32));
                    outlet.send(file.becomes(file.kind, Some(dest))).await;
                }
                Err(e) => {
                    cx.item(&file, ItemState::Bad, e.to_string());
                    cx.log(format!("couldn't file {}: {e}", file.label));
                }
            }
        }
        if !taken.is_empty() && !cx.stopped() {
            let dirs: std::collections::BTreeSet<String> = taken.iter().map(|(_, _, d)| where_filed(&root, d)).collect();
            let place = match dirs.iter().next() {
                Some(d) if dirs.len() == 1 && !d.is_empty() => d.clone(),
                _ => folder.clone(),
            };
            let Ok(answer) = cx.ask(name_taken(taken.len(), &place)).await else { return Ok(()) };
            let choice = match answer {
                drt_engine::Answer::Pick(0) => Taken::Replace,
                drt_engine::Answer::Pick(2) | drt_engine::Answer::Skip => Taken::Skip,
                _ => Taken::KeepBoth,
            };
            let _f = cx.lane(LaneKind::File(folder.clone())).await;
            for (file, src, dest) in taken {
                if choice == Taken::Skip {
                    cx.item(&file, ItemState::Skipped, "not filed: a different file has this name");
                    continue;
                }
                cx.item(&file, ItemState::Busy, "copying");
                let filed = match choice {
                    Taken::Replace => copy_in(&src, &dest).await.map(|()| dest),
                    _ => keep_both(&src, &dest).await,
                };
                match filed {
                    Ok(dest) => {
                        done += 1;
                        cx.item(&file, ItemState::Ok, dest.display().to_string());
                        last = where_filed(&root, &dest);
                        outlet.send(file.becomes(file.kind, Some(dest))).await;
                    }
                    Err(e) => {
                        cx.item(&file, ItemState::Bad, e.to_string());
                        cx.log(format!("couldn't file {}: {e}", file.label));
                    }
                }
            }
            cx.state(StepState::Busy);
        }
        if done > 0 {
            filed_says(&cx, done, last);
        }
        Ok(())
    }
}

/// What's already at a file's place in the library.
#[derive(Debug, PartialEq)]
pub enum There {
    Nothing,
    /// The same bytes: it's filed already.
    Same,
    Different,
}

pub async fn already_there(src: &Path, dest: &Path) -> Result<There> {
    if !tokio::fs::try_exists(dest).await? {
        return Ok(There::Nothing);
    }
    let (src, dest) = (src.to_owned(), dest.to_owned());
    let same = tokio::task::spawn_blocking(move || same_bytes(&src, &dest)).await??;
    Ok(if same { There::Same } else { There::Different })
}

/// Whether two files hold the same bytes.
pub fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let (fa, fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let (mut ra, mut rb) = (std::io::BufReader::new(fa), std::io::BufReader::new(fb));
    let (mut x, mut y) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let n = ra.read(&mut x)?;
        if n == 0 {
            return Ok(true);
        }
        rb.read_exact(&mut y[..n])?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
    }
}

/// Copies under a temporary name and renames, so a half-copied file never
/// sits in the library under its real name.
async fn copy_in(src: &Path, dest: &Path) -> Result<()> {
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    place::copy(src, dest).await?;
    Ok(())
}

/// A name beside `dest` with a number: *01 - Title (2).flac*.
pub fn numbered(dest: &Path, n: u32) -> PathBuf {
    let stem = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match dest.extension() {
        Some(e) => dest.with_file_name(format!("{stem} ({n}).{}", e.to_string_lossy())),
        None => dest.with_file_name(format!("{stem} ({n})")),
    }
}

/// Files the new copy under the first numbered name that's free, or that
/// already holds the same bytes. Never over a different file.
async fn keep_both(src: &Path, dest: &Path) -> Result<PathBuf> {
    for n in 2..1000 {
        let d = numbered(dest, n);
        match already_there(src, &d).await? {
            There::Nothing => {
                copy_in(src, &d).await?;
                return Ok(d);
            }
            There::Same => return Ok(d),
            There::Different => {}
        }
    }
    bail!("no free name beside {}", dest.display())
}

#[derive(Clone, Copy, PartialEq)]
enum Taken {
    Replace,
    KeepBoth,
    Skip,
}

/// The one question a Folder asks when names are taken: *3 files already
/// in Parquet Courts / Wide Awake!*. Keep both is picked: it loses nothing.
pub fn name_taken(n: usize, place: &str) -> drt_engine::Question {
    let choice = |l: &str, d: &str| drt_engine::Choice { label: l.into(), detail: d.into() };
    let files = if n == 1 { "1 file".to_owned() } else { format!("{n} files") };
    drt_engine::Question {
        prompt: format!("{files} already in {place}"),
        shape: drt_engine::QuestionShape::PickOne {
            options: vec![
                choice("Replace them", "The files there now are overwritten"),
                choice("Keep both", "The new files get \" (2)\" after their names"),
                choice("Skip", "The new files aren't filed"),
            ],
            preselected: 1,
            details: None,
            skip: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rip_counts_only_real_verifications() {
        let item = |n: u32, state, note: &str, verified| drt_engine::ItemView { id: format!("track-{n}"), label: format!("{n}"), state, note: note.into(), verified };
        let items =
            [item(1, ItemState::Ok, "✓ 104", true), item(2, ItemState::Ok, "✓ 98", true), item(3, ItemState::Ok, "kept, unverified", false), item(4, ItemState::Ok, "✓ two reads agree", false)];
        assert_eq!(rip_outcome(&items), ("2/4 verified".into(), "2 not verified".into()));
        assert_eq!(rip_outcome(&items[..3]), ("2/3 verified".into(), "1 kept".into()));
        // Left out isn't counted against it.
        assert_eq!(rip_outcome(&[item(1, ItemState::Ok, "✓ 104", true), item(2, ItemState::Skipped, "skipped", false)]).0, "1/1 verified");
        assert_eq!(rip_needs(&[(9, "no match".into())]), "Track 9 doesn't match AccurateRip");
    }

    fn wav(path: &Path, frames: &[[i16; 2]]) {
        let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        b.extend([16, 0, 0, 0, 1, 0, 2, 0, 0x44, 0xAC, 0, 0, 0x10, 0xB1, 2, 0, 4, 0, 16, 0]);
        b.extend(b"data");
        b.extend(((frames.len() * 4) as u32).to_le_bytes());
        for f in frames {
            b.extend(f[0].to_le_bytes());
            b.extend(f[1].to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn combining_reads_keeps_the_majority_of_each_sample() {
        let dir = std::env::temp_dir().join(format!("drt-combine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = vec![[1i16, 1]; 100];
        let (mut a, mut b, mut c) = (base.clone(), base.clone(), base.clone());
        a[10] = [9, 9]; // one read wrong: the other two win
        b[20] = [5, 5];
        c[20] = [6, 6]; // all three differ at 20: no majority
        for (n, f) in [("a", &a), ("b", &b), ("c", &c)] {
            wav(&dir.join(format!("{n}.wav")), f);
        }
        let files: Vec<PathBuf> = ["a", "b", "c"].iter().map(|n| dir.join(format!("{n}.wav"))).collect();
        let out = dir.join("out.wav");
        assert_eq!(combine(&files, &out).unwrap(), (2, 1));
        assert_eq!(differences(&out, &dir.join("c.wav")).unwrap().map(|d| d.0), Some(1), "frame 10 fixed; 20 left as one read had it");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_careful_read_that_matches_accuraterip_counts_as_verified() {
        // A re-read that matches says so, even though the reads agree too.
        assert_eq!(settled_note(Some(7), true).as_deref(), Some("✓ 7"));
        assert!(is_accurate(&settled_note(Some(7), false).unwrap()));
        assert_eq!(settled_note(None, true).as_deref(), Some("✓ two reads agree"));
        assert!(!is_accurate("✓ two reads agree") && !is_accurate("✓ combined from 3 reads"));
        assert_eq!(settled_note(None, false), None);
    }

    #[test]
    fn combine_stops_reading_a_track_that_never_reads() {
        assert!(more_reads_for_vote(1, 0));
        assert!(!more_reads_for_vote(3, 2), "a vote's worth");
        // Every read failing: it gives up rather than reading for ever.
        let tries = (0..).take_while(|&t| more_reads_for_vote(0, t)).count();
        assert_eq!(tries, 6);
    }

    #[test]
    fn samples_are_found_by_their_chunk_not_by_the_word_data() {
        let dir = std::env::temp_dir().join(format!("drt-chunks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // As cyanrip writes it: a LIST chunk of CD-Text before the samples,
        // here with "data" in a title.
        let with_list = |frames: &[[i16; 2]]| {
            let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
            b.extend([16, 0, 0, 0, 1, 0, 2, 0, 0x44, 0xAC, 0, 0, 0x10, 0xB1, 2, 0, 4, 0, 16, 0]);
            let info = b"INFOINAM\x09\0\0\0Metadata\0\0";
            b.extend(b"LIST");
            b.extend((info.len() as u32).to_le_bytes());
            b.extend(info);
            b.extend(b"data");
            b.extend(((frames.len() * 4) as u32).to_le_bytes());
            for f in frames {
                b.extend(f[0].to_le_bytes());
                b.extend(f[1].to_le_bytes());
            }
            b
        };
        let base = vec![[100i16, -100]; 50];
        let (mut a, b, mut c) = (base.clone(), base.clone(), base.clone());
        a[7] = [1, 2];
        c[30] = [3, 4];
        let files: Vec<PathBuf> = [("a", &a), ("b", &b), ("c", &c)]
            .iter()
            .map(|(n, f)| {
                let p = dir.join(format!("{n}.wav"));
                std::fs::write(&p, with_list(f)).unwrap();
                p
            })
            .collect();
        let bytes = std::fs::read(&files[1]).unwrap();
        assert_eq!(&bytes[wav_data(&bytes).unwrap()], &with_list(&b)[bytes.len() - 200..]);
        let out = dir.join("out.wav");
        assert_eq!(combine(&files, &out).unwrap(), (2, 0));
        assert_eq!(std::fs::read(&out).unwrap(), with_list(&base), "every sample voted back to the truth, header intact");
        assert_eq!(differences(&files[0], &files[1]).unwrap().map(|d| d.0), Some(1));
        assert_eq!(wav_data(b"RIFF\0\0\0\0WAVE"), None);
        // A chunk claiming 4 GB, then nothing: no data, no overflow.
        assert_eq!(wav_data(b"RIFF\0\0\0\0WAVELIST\xFF\xFF\xFF\xFFdata"), None);
        assert_eq!(wav_data(b"RIFF\0\0\0\0WAVEdata\xFF\xFF\xFF\xFF\x01\x02"), Some(20..22), "a data chunk longer than the file is cut to it");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_hidden_track_is_found_from_the_toc_and_must_be_exactly_its_length() {
        // Track 1 at the usual two seconds: no room before it.
        assert_eq!(hidden_track_frames("1 2 22000 150 11382"), None);
        // Track 1 at 0:32 (2400 frames, less the 150 of lead-in): 2250 sectors.
        assert_eq!(hidden_track_frames("1 2 22000 2400 11382"), Some(2250 * 588));
        assert_eq!(hidden_track_frames("nonsense"), None);
        let dir = std::env::temp_dir().join(format!("drt-hidden-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (quiet, music) = (dir.join("quiet.wav"), dir.join("music.wav"));
        wav(&quiet, &vec![[0i16, 0]; 588 * 3]);
        let mut m = vec![[0i16, 0]; 588 * 3];
        m[1000] = [0, 1];
        wav(&music, &m);
        assert!(is_silent(&quiet).unwrap());
        assert!(!is_silent(&music).unwrap(), "one sample is audio");
        assert!(hidden_track_is_whole(&music, 588 * 3, 588 * 3).is_ok());
        assert!(hidden_track_is_whole(&music, 588 * 3, 588 * 4).is_err(), "shorter than the TOC");
        assert!(hidden_track_is_whole(&music, 588 * 4, 588 * 3).is_err(), "cyanrip disagrees with the file");
        assert!(hidden_track_is_whole(&music, 0, 0).is_err(), "no length to check against");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_hidden_track_is_filed_as_track_00() {
        let mut c = Context::default();
        c.add(Subject::Disc, "album", "Wide Awake!", "test");
        c.add(Subject::Disc, "album_artist", "Parquet Courts", "test");
        let rel = template::render(template::DEFAULT, &fields_in(&c, 0, HIDDEN));
        assert_eq!(rel, Path::new("Parquet Courts/Wide Awake!/00 - Hidden track"));
    }

    #[test]
    fn a_numbered_name_keeps_its_extension() {
        assert_eq!(numbered(Path::new("/m/01 - A.flac"), 2), Path::new("/m/01 - A (2).flac"));
        assert_eq!(numbered(Path::new("/m/Wide Awake!.cue"), 3), Path::new("/m/Wide Awake! (3).cue"));
        let q = name_taken(1, "A / B");
        assert_eq!(q.prompt, "1 file already in A / B");
    }

    #[test]
    fn a_measurement_never_overrides_the_drive_list() {
        let listed = offset::Listed { offset: 103, submitted: 528, agree: 100 };
        assert_eq!(judge_measurement(121, 43, Some(&listed)), (103, "AccurateRip, 528 agree".to_owned()));
        assert_eq!(judge_measurement(6, 9, None), (6, "measured, 9 matches".to_owned()));
    }

    #[test]
    fn two_reads_are_compared_sample_by_sample() {
        let dir = std::env::temp_dir().join(format!("drt-diff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = vec![[0i16, 0]; 44_100 * 3];
        let b = a.clone();
        a[44_100 + 5] = [1, 0];
        a[2 * 44_100 + 9] = [0, 7];
        wav(&dir.join("a.wav"), &a);
        wav(&dir.join("b.wav"), &b);
        assert_eq!(differences(&dir.join("a.wav"), &dir.join("b.wav")).unwrap(), Some((2, 1, 2)));
        assert_eq!(differences(&dir.join("b.wav"), &dir.join("b.wav")).unwrap(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod track_lines {
    #[test]
    fn a_track_without_a_title_is_named_once() {
        assert_eq!(super::track_line(6, "Cannon"), "Track 6 · Cannon");
        assert_eq!(super::track_line(1, "Track 1"), "Track 1");
        assert_eq!(super::track_line(3, ""), "Track 3");
    }
}
