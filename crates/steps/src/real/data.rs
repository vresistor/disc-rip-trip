//! Data discs: an image (DESIGN-UX.md, "Other kinds of disc", 1).
//!
//! `Drive, Data disc, Image, Check, then CHD, Library; and Next disc`.
//! Each step's `StepInfo` is here once and shared by the real and the
//! simulated step, so a recipe runs on either.

use super::{chd, device, hash, media, redump, redumper};
use crate::template;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use drt_engine::setting::ParamKind;
use drt_engine::{Context, Cx, Group, Inlet, Item, ItemKind, ItemState, LaneKind, Outlet, ParamSpec, Step, StepInfo, StepState, Subject};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The id of the one item a data disc becomes: its dump, then the files
/// made from it. Place files an item with this id as a set, in a folder.
pub const DUMP: &str = "dump";

pub const SPEEDS: &[&str] = &["Drive's best", "1×", "2×", "4×", "8×", "16×", "24×", "48×"];

type Get<'a> = &'a dyn Fn(&str) -> Option<Value>;

fn s(get: Get, k: &str) -> String {
    match get(k) {
        Some(Value::String(v)) => v,
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

// ---- what each step is ------------------------------------------------------------------------

pub fn data_disc_info() -> StepInfo {
    StepInfo::new("data-disc", "Data disc", Group::Kind, ItemKind::Disc, ItemKind::DataDisc).gives(&["label"]).from("read the disc's name and size")
}

/// Bad sectors: the dump re-reads each up to *Retries* times, then a
/// refine pass goes back over whatever is still bad, always, since a
/// clean image is the job. If any remain after that, the dump needs you:
/// *Re-read bad parts* again, *Keep it anyway* or *Skip this disc*.
pub fn image_info() -> StepInfo {
    StepInfo::new("image", "Rip", Group::Read, ItemKind::DataDisc, ItemKind::Dump)
        .gives(&["read"])
        .from("copy the whole disc to an image")
        .param(ParamSpec::new("retries", "Retries on a bad spot", ParamKind::Range { min: 0, max: 50 }).default(20).on_card("Retries"))
        .param(ParamSpec::new("speed", "Read speed", ParamKind::Choice(SPEEDS)).default(SPEEDS[0]))
        .summary(|g| {
            let retries = g("retries").and_then(|v| v.as_i64()).unwrap_or(0);
            let speed = s(g, "speed");
            let speed = if speed == SPEEDS[0] || speed.is_empty() { String::new() } else { format!(", {speed}") };
            format!("{retries} retries{speed}")
        })
        .retry("refine", "Re-read bad parts", "")
        .retry("keep", "Keep it anyway", "")
        .retry("leave_out", "Skip this disc", "")
}

pub fn check_info() -> StepInfo {
    StepInfo::new("check", "Check and fix", Group::Check, ItemKind::Dump, ItemKind::Dump)
        .gives(&["hashes", "name"])
        .from("compare with Redump's known-good copies")
        .param(ParamSpec::new("systems", "Compare with Redump's list for", ParamKind::Choice(redump::CHOICES)).default(redump::ALL).on_card("Look in").labels(redump::LABELS))
        // A game Redump has whose copy doesn't match: fix bad sectors, then
        // rip again and combine, by itself; or ask first.
        .param(
            ParamSpec::new("if_no_match", "If it doesn't match Redump", ParamKind::Choice(super::game::IF_NO_MATCH))
                .default(super::game::BY_ITSELF)
                .labels(super::game::IF_NO_MATCH_LABELS)
                .help("By itself, it reads bad sectors again, then rips again, and asks only if that doesn't make it match."),
        )
        .param(ParamSpec::new("fix_tries", "Tries for each bad sector", ParamKind::Range { min: 1, max: 50 }).default(super::fix::TRIES as i64).section("Advanced"))
        .param(
            ParamSpec::new("extra_rips", "Extra rips at most", ParamKind::Range { min: 0, max: 10 })
                .default(super::game::EXTRA_RIPS as i64)
                .section("Advanced")
                .when("if_no_match", &[super::game::BY_ITSELF]),
        )
        // For a game whose serial Redump knows but whose hash it doesn't,
        // or a dump whose own sectors say it was read wrong. Each dump it
        // holds is offered only those that can help it.
        .retry("refine", "Re-read bad parts", "")
        .retry("fix", "Fix bad sectors", "")
        .retry("combine", "Rip again and combine", "")
        .retry("keep", "Keep it anyway", "")
        .retry("leave_out", "Skip this disc", "")
}

pub fn chd_info() -> StepInfo {
    StepInfo::new("chd", "CHD", Group::Make, ItemKind::Dump, ItemKind::ImageFile)
        .from("compressed image for emulators")
        .param(ParamSpec::new("codecs", "Compression", ParamKind::Choice(chd::CODECS)).default(chd::CODECS[0]).on_card("Method").labels(&[
            "Standard, works in every emulator",
            "zstd, loads faster, needs a recent emulator",
            "lzma",
            "zlib, bigger files",
        ]))
        .param(ParamSpec::new("hunk", "Block size", ParamKind::Choice(chd::HUNKS)).default(chd::HUNKS[0]).section("Advanced"))
        .summary(|g| {
            let c = s(g, "codecs");
            if c == chd::CODECS[0] || c.is_empty() {
                "standard compression".to_owned()
            } else {
                c
            }
        })
}

pub fn keep_info() -> StepInfo {
    StepInfo::new("keep", "Plain image", Group::Make, ItemKind::Dump, ItemKind::ImageFile).from("uncompressed image, with its log")
}

/// Folder and Library: one step for tracks and for disc images. Takes any
/// file and hands on the same kind, filed: what LibCrypt and M3U need. Its
/// names follow what reaches it ([`template::names_for`]): a track layout
/// for music, a folder by system for a game, by name for another disc, the
/// Jellyfin layout for films; [`template::DEFAULT`] only where it could be
/// anything.
pub fn place_info(kind: &'static str, name: &'static str) -> StepInfo {
    StepInfo::new(kind, name, Group::Place, ItemKind::AnyFile, ItemKind::AnyFile)
        .repeatable()
        .passes_through()
        .gives(&["filed"])
        .from_param("folder")
        .param(ParamSpec::new("folder", "Folder", ParamKind::Folder).on_card("Folder"))
        .param(
            ParamSpec::new("names", "Names", ParamKind::Template(template::LIBRARY_PRESETS))
                .default(template::DEFAULT)
                .fits(template::names_for)
                // A preset by its name (*System / Name*); your own, as you wrote it.
                .on_card_as("Names", &[], |g| template::preset_name(&s(g, "names"))),
        )
}

/// What a dump's template can use, from what the dump knows.
pub fn dump_fields(c: &Context) -> template::DumpFields {
    let get = |k| c.get(Subject::Disc, k).unwrap_or("").to_owned();
    let label = get("label");
    let name = c.get(Subject::Disc, "name").map(str::to_owned).filter(|n| !n.is_empty()).unwrap_or_else(|| if label.is_empty() { "Disc".into() } else { label.clone() });
    let bytes = get("bytes").parse().unwrap_or(0);
    template::DumpFields { name, label, system: get("system"), date: template::today(), size: if bytes > 0 { media::size(bytes) } else { String::new() } }
}

/// The folder a dump is filed in, relative, from a Place's template.
pub fn dump_folder(c: &Context, names: &str) -> PathBuf {
    template::render_game(names, &dump_fields(c), &super::game::game_fields(c))
}

/// The disc's label as an item label.
fn label_of(c: &Context) -> String {
    c.get(Subject::Disc, "label").map(str::to_owned).filter(|l| !l.is_empty()).unwrap_or_else(|| "Disc".into())
}

// ---- Data disc --------------------------------------------------------------------------------

pub struct DataDisc;

#[async_trait]
impl Step for DataDisc {
    fn info(&self) -> StepInfo {
        data_disc_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(disc) = inlet.recv().await {
            let dev = device(disc.context())?;
            cx.status("", "Reading the disc");
            let found = {
                let _drive = cx.lane(LaneKind::Drive).await;
                tokio::task::spawn_blocking(move || media::read(&dev)).await?
            };
            if found.bytes == 0 {
                bail!("the disc has no size: is it in?");
            }
            let mut d = disc.becomes(ItemKind::DataDisc, None);
            {
                let c = d.context_mut();
                let d = "the disc";
                c.add(Subject::Disc, "label", found.label.clone(), d);
                c.add(Subject::Disc, "filesystem", found.filesystem.clone(), d);
                c.add(Subject::Disc, "media", found.media.clone(), d);
                c.add(Subject::Disc, "bytes", found.bytes.to_string(), d);
                c.add(Subject::Disc, "parts", "1", d);
                c.add(Subject::Disc, "audio_tracks", found.audio_tracks.to_string(), d);
            }
            let label = if found.label.is_empty() { "No label".to_owned() } else { found.label.clone() };
            cx.status(label, [found.filesystem.as_str(), &media::size(found.bytes)].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · "));
            cx.log(format!("{} of {}, {}, label {:?}, {} track(s), {} audio", found.media, media::size(found.bytes), found.filesystem, found.label, found.tracks, found.audio_tracks));
            if found.tracks > 0 && found.audio_tracks == found.tracks {
                cx.log("every track is audio: this is an audio CD, imaged as one");
            }
            d.label = label_of(d.context());
            outlet.send(d).await;
        }
        Ok(())
    }
}

// ---- Image ------------------------------------------------------------------------------------

pub struct Image;

/// How a dump came out: its note when it's good, or what's wrong with it.
type Outcome = std::result::Result<String, String>;

impl Image {
    /// Runs one redumper command, showing its phase and progress on the tile
    /// and keeping its findings in the log.
    pub(crate) async fn phase(&self, cx: &Cx, item: &Item, t: &redumper::Target, args: Vec<String>, out: &mut String) -> Result<bool> {
        cx.log(format!("redumper {}", args.join(" ")));
        // Opening the tray stops it.
        let hold = cx.drive_read();
        // Where the drive reports errors, for the map of the disc.
        let mut trail = redumper::ErrorTrail::new();
        let (text, ok) = redumper::run(&args, t, hold.stop(), |line| {
            if let Some(l) = trail.line(line) {
                cx.log(l);
            }
            if let Some(p) = redumper::phase(line) {
                cx.status("", p);
                cx.drive_says(StepState::Busy, p, "");
            }
            if let Some(p) = redumper::parse_progress(line) {
                cx.progress(Some(p.fraction));
                cx.item(item, ItemState::Busy, redumper::progress_note(&p));
            }
            if redumper::worth_logging(line) {
                cx.log(line.trim());
            }
        })
        .await?;
        if let Some(l) = trail.finish() {
            cx.log(l);
        }
        out.push_str(&text);
        Ok(ok)
    }

    /// Makes the image from what's been read: splits a CD into its tracks
    /// (forcing past unreadable sectors with `force`), or takes a DVD's
    /// ISO; then has redumper check every sector of it. Gives the note for
    /// a good image, or what's still wrong.
    pub(crate) async fn settle(&self, cx: &Cx, item: &Item, t: &redumper::Target, out: &mut String, force: bool, refined: bool) -> Result<Outcome> {
        let unreadable = if t.is_cd() {
            let mut split = String::new();
            let ok = self.phase(cx, item, t, redumper::split_args(t, force), &mut split).await?;
            out.push_str(&split);
            let skipped: u64 = split.lines().filter_map(redumper::parse_skipped).sum();
            match (ok, t.main()) {
                (true, Some(_)) => skipped,
                _ if redumper::split_needs_force(&split) => return Ok(Err(plural(skipped.max(1), "unreadable sector"))),
                _ => bail!("{}", redumper::reason(&split)),
            }
        } else {
            if t.main().is_none() {
                bail!("{}", redumper::reason(out));
            }
            match redumper::errors_left(out) {
                0 => 0,
                n if force => n,
                n => return Ok(Err(plural(n, "unreadable sector"))),
            }
        };
        // Sectors the drive read wrong without saying so: only an inspection
        // of the image finds them (redumper's `info`, which `disc` runs).
        let mut info = String::new();
        self.phase(cx, item, t, vec!["info".into(), format!("--image-path={}", t.dir.display()), format!("--image-name={}", t.name)], &mut info).await?;
        out.push_str(&info);
        let size = t.main().and_then(|m| std::fs::metadata(m).ok()).map(|m| media::size(m.len())).unwrap_or_default();
        let problems = redumper::sector_problems(&info);
        // The error codes go to the log; the note says it in words.
        let detail = problems.iter().filter(|p| p.1 > 0).map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ");
        if !detail.is_empty() {
            cx.log(format!("sectors read wrong: {detail}"));
        }
        Ok(Ok(image_note(&size, force, unreadable, refined, &problems)))
    }

    /// One whole read of the disc into `t`: redumper's dump, a re-read of
    /// whatever the drive reported bad, then the image made and checked.
    /// Rip's read, and Check's when it rips again. The caller has the drive.
    pub(crate) async fn read_disc(&self, cx: &Cx, item: &Item, t: &redumper::Target, opts: &redumper::Options, out: &mut String) -> Result<Outcome> {
        if !self.phase(cx, item, t, redumper::dump_args(opts, t), out).await? {
            bail!("{}", redumper::reason(out));
        }
        let refine = redumper::errors_left(out) > 0;
        if refine {
            self.phase(cx, item, t, redumper::refine_args(opts, t), out).await?;
        }
        self.settle(cx, item, t, out, false, refine).await
    }
}

/// What Rip puts on the dump about how it read it, so a later step can
/// read the disc again the same way: by redumper, with these retries, at
/// this speed (none: the drive's best).
pub const RIPPED_WITH: &str = "ripped with";
pub const RIP_RETRIES: &str = "rip retries";
pub const RIP_SPEED: &str = "rip speed";

/// How Rip read the dump, to read it again the same way: the drive, the
/// retries and speed it used, and the drive's read offset.
pub fn rip_options(c: &Context) -> Result<redumper::Options> {
    let get = |k| c.get(Subject::Disc, k);
    Ok(redumper::Options {
        drive: redumper::drive_access(&device(c)?)?,
        retries: get(RIP_RETRIES).and_then(|v| v.parse().ok()).unwrap_or(20),
        speed: get(RIP_SPEED).and_then(|v| v.parse().ok()),
        read_offset: read_offset(c),
    })
}

/// Whether ripping a dump again and combining the rips can help: a CD
/// image redumper made, as a cue sheet and raw sectors, each of which has
/// checks of its own. An ISO has none, and a DVD or Blu-ray drive reports
/// what it can't read anyway.
pub fn combinable(dump: &Item) -> bool {
    dump.context().get(Subject::Disc, RIPPED_WITH) == Some("redumper") && dump.path.as_deref().is_some_and(|m| m.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue")))
}

/// Reads the disc again in full into `dir`, the way Rip read it, and gives
/// the new rip's cue sheet. Sectors it couldn't read are left filled in:
/// the other rips may have them. Its working files go once its tracks are
/// written, since only they are combined.
pub async fn rip_again(cx: &Cx, dump: &Item, dir: PathBuf) -> Result<PathBuf> {
    let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
    let t = redumper::Target { dir, name: main.file_stem().unwrap_or_default().to_string_lossy().into_owned() };
    let opts = rip_options(dump.context())?;
    let mut out = String::new();
    {
        let _drive = cx.lane(LaneKind::Drive).await;
        let read = Image.read_disc(cx, dump, &t, &opts, &mut out).await;
        let forced = match read {
            Ok(Ok(note)) => {
                cx.log(format!("rip again: {note}"));
                Ok(())
            }
            Ok(Err(bad)) => {
                cx.log(format!("rip again: {bad}; kept as it is, for the other rips to fill in"));
                Image.settle(cx, dump, &t, &mut out, true, false).await.map(|_| ())
            }
            Err(e) => Err(e),
        };
        cx.drive_says(StepState::Idle, "Waiting", "");
        forced?;
    }
    for ext in ["scram", "state", "subcode"] {
        let _ = std::fs::remove_file(t.file(ext));
    }
    t.main().filter(|m| m.extension().is_some_and(|e| e == "cue")).ok_or_else(|| anyhow!("the new rip made no cue sheet"))
}

/// The drive's read offset, which Drive put on the disc when it knew it.
pub fn read_offset(c: &Context) -> Option<i64> {
    c.get(Subject::Disc, "offset").and_then(|v| v.parse().ok())
}

fn plural(n: u64, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// What Image says about an image it made.
pub fn image_note(size: &str, forced: bool, unreadable: u64, refined: bool, problems: &[(&'static str, u64)]) -> String {
    let wrong: u64 = problems.iter().map(|p| p.1).max().unwrap_or(0);
    match (forced, unreadable, wrong) {
        (true, 0, 0) => format!("kept anyway, {size}"),
        (true, n, 0) => format!("kept anyway, {} left blank", plural(n, "unreadable sector")),
        (true, n, w) => format!("kept anyway, {} left blank, {} came back wrong", plural(n, "unreadable sector"), plural(w, "sector")),
        (false, _, 0) if refined => format!("✓ {size}, bad parts re-read"),
        (false, _, 0) => format!("✓ {size}, read cleanly"),
        // The drive didn't know: re-reading here won't help; another drive might.
        (false, _, w) => format!("read, but {} came back wrong", plural(w, "sector")),
    }
}

#[async_trait]
impl Step for Image {
    fn info(&self) -> StepInfo {
        image_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let Some(disc) = inlet.recv().await else { return Ok(()) };
        let dev = match redumper::drive_access(&device(disc.context())?) {
            Ok(d) => d,
            Err(e) => {
                cx.drive_says(StepState::Bad, "Can't open", &e.to_string());
                return Err(e);
            }
        };
        let label = label_of(disc.context());
        let name = redumper::image_name(disc.context().get(Subject::Disc, "label").unwrap_or(""));
        let t = redumper::Target { dir: cx.work()?.join("image"), name };
        let speed = cx.text("speed").trim_end_matches('×').parse().ok();
        let read_offset = read_offset(disc.context());
        let opts = redumper::Options { drive: dev, retries: cx.param("retries").and_then(|v| v.as_u64()).unwrap_or(20) as u32, speed, read_offset };
        if let Some(o) = read_offset {
            cx.log(format!("the drive's read offset, {o:+}, given to redumper"));
        }
        let mut item = Item::new(DUMP, ItemKind::Dump, label, None, None).knowing(&disc);
        {
            let c = item.context_mut();
            c.add(Subject::Disc, RIPPED_WITH, "redumper", "Rip");
            c.add(Subject::Disc, RIP_RETRIES, opts.retries.to_string(), "Rip");
            if let Some(s) = opts.speed {
                c.add(Subject::Disc, RIP_SPEED, s.to_string(), "Rip");
            }
        }
        cx.item(&item, ItemState::Busy, "reading");
        let mut out = String::new();
        let mut outcome = {
            let _drive = cx.lane(LaneKind::Drive).await;
            let o = self.read_disc(&cx, &item, &t, &opts, &mut out).await?;
            cx.drive_says(StepState::Idle, "Waiting", "");
            o
        };
        cx.progress(None);
        loop {
            match outcome {
                Ok(note) => {
                    let main = t.main().ok_or_else(|| anyhow!("redumper made no image"))?;
                    cx.item(&item, ItemState::Ok, note.clone());
                    cx.state(StepState::Busy);
                    cx.status(image_size(&main), if note.contains("re-read") || note.contains("refined") { note.trim_start_matches("✓ ").to_owned() } else { String::new() });
                    outlet.send(item.becomes(ItemKind::Dump, Some(main))).await;
                    return Ok(());
                }
                Err(bad) => {
                    cx.log(format!("image: {bad}"));
                    cx.item(&item, ItemState::Bad, bad.clone());
                    cx.state(StepState::Bad);
                    cx.status("", cap(&bad));
                }
            }
            let req = loop {
                match cx.next_retry().await {
                    None => return Ok(()),
                    Some(r) if r.item == item.id => break r,
                    Some(_) => {}
                }
            };
            cx.state(StepState::Busy);
            outcome = match req.option.as_str() {
                "leave_out" => {
                    cx.item(&item, ItemState::Skipped, "skipped");
                    cx.status("Skipped", "");
                    return Ok(());
                }
                "keep" => {
                    cx.item(&item, ItemState::Busy, "keeping anyway");
                    self.settle(&cx, &item, &t, &mut out, true, false).await?
                }
                _ => {
                    cx.item(&item, ItemState::Busy, "re-reading bad parts");
                    let _drive = cx.lane(LaneKind::Drive).await;
                    self.phase(&cx, &item, &t, redumper::refine_args(&opts, &t), &mut out).await?;
                    let o = self.settle(&cx, &item, &t, &mut out, false, true).await?;
                    cx.drive_says(StepState::Idle, "Waiting", "");
                    o
                }
            };
        }
    }
}

// ---- Check ------------------------------------------------------------------------------------

/// Check, with the rip it uses to read a held dump again.
pub struct Check {
    pub rip: super::game::RipFn,
    pub source: super::game::SourceFn,
}

impl Default for Check {
    fn default() -> Self {
        Check { rip: super::game::redumper_rip(), source: super::game::drive_source() }
    }
}

/// The data files of a dump, the ones Redump catalogues: its tracks or its
/// ISO, and its cue sheet. Not the log.
pub fn data_files(main: &Path) -> Vec<PathBuf> {
    redumper::dump_files(main).into_iter().filter(|f| !f.extension().is_some_and(|e| e.eq_ignore_ascii_case("log"))).collect()
}

/// The image's own data: its tracks in the cue's order, or its ISO.
pub fn image_files(main: &Path) -> Vec<PathBuf> {
    data_files(main).into_iter().filter(|f| !f.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue"))).collect()
}

/// The cache folder beside the runs: `…/disc-rip-trip/cache/redump`.
pub fn redump_cache(work: &Path) -> PathBuf {
    work.parent().and_then(Path::parent).unwrap_or(work).join("cache/redump")
}

/// What Check says about a dump and its catalogue lookups.
/// A dump's size on disk, all its files: *682 MB*.
pub fn image_size(main: &Path) -> String {
    let bytes: u64 = redumper::dump_files(main).iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum();
    let bytes = if bytes == 0 { std::fs::metadata(main).map(|m| m.len()).unwrap_or(0) } else { bytes };
    media::size(bytes)
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub fn verdict(found: Option<(&str, &str)>, looked: usize, failed: &[&str]) -> (String, String, String) {
    match found {
        Some((name, _)) => (format!("Verified: {name}"), "Matches Redump".into(), name.into()),
        None if looked == 0 => ("Couldn't check: nowhere to look".into(), "Couldn't check".into(), failed.join(", ")),
        None if !failed.is_empty() => ("Not in Redump".into(), "Not in Redump".into(), format!("{failed} not reached", failed = failed.join(", "))),
        None => ("Not in Redump".into(), "Not in Redump".into(), format!("looked in {looked} system{}", if looked == 1 { "" } else { "s" })),
    }
}

#[async_trait]
impl Step for Check {
    fn info(&self) -> StepInfo {
        check_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut dump) = inlet.recv().await {
            let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
            let files = data_files(&main);
            cx.item(&dump, ItemState::Busy, "checking");
            cx.status("", format!("Checking {} file{}", files.len(), if files.len() == 1 { "" } else { "s" }));
            let roms = {
                let cx2 = cx.clone();
                let d = dump.clone();
                tokio::task::spawn_blocking(move || {
                    let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
                    hash::hash_files(
                        &refs,
                        |p| {
                            cx2.progress(Some(p));
                            cx2.item(&d, ItemState::Busy, format!("checking {}%", (p * 100.0) as u32));
                        },
                        || cx2.stopped(),
                    )
                })
                .await??
            };
            cx.progress(None);
            for r in &roms {
                dump.context_mut().add(Subject::Disc, &format!("sha1 {}", r.name), r.sha1.clone(), "Check");
            }
            for r in &roms {
                cx.log(format!("<rom name=\"{}\" size=\"{}\" crc=\"{}\" md5=\"{}\" sha1=\"{}\" />", r.name, r.size, r.crc, r.md5, r.sha1));
            }
            let media = dump.context().get(Subject::Disc, "media").unwrap_or("").to_owned();
            let systems = super::game::catalogues(dump.context(), &cx.text("systems"), &media);
            let cache = redump_cache(&cx.work()?);
            let (mut found, mut looked, mut failed) = (None, 0, vec![]);
            for sys in &systems {
                cx.status("", format!("Looking up {}", sys.name));
                cx.item(&dump, ItemState::Busy, format!("in {}", sys.name));
                let cat = {
                    let _net = cx.lane(LaneKind::Network("Redump".into())).await;
                    redump::catalogue(&cache, sys.slug, redump::download).await
                };
                match cat {
                    Ok(c) => {
                        looked += 1;
                        if let Some(g) = c.by_hashes(&roms) {
                            found = Some((g.name.clone(), sys.name));
                            break;
                        }
                    }
                    Err(e) => {
                        cx.log(format!("the {} catalogue: {e}", sys.name));
                        failed.push(sys.name);
                    }
                }
            }
            if let Some((name, system)) = &found {
                let c = dump.context_mut();
                c.add(Subject::Disc, "name", name.clone(), "Redump");
                c.add(Subject::Disc, "system", *system, "Redump");
            }
            let (note, status, detail) = verdict(found.as_ref().map(|(n, s)| (n.as_str(), *s)), looked, &failed);
            cx.log(&note);
            // Not being in Redump isn't a bad dump: most data discs aren't.
            // Found, it's verified: the catalogue's hashes are independent.
            if found.is_some() {
                cx.verified(&dump, note);
            } else {
                cx.item(&dump, ItemState::Ok, note);
            }
            cx.status(status, detail);
            // A game Redump has under its serial, but not with this hash: suspect, held for you.
            let dump = if found.is_none() {
                let reread = super::game::Disc { rip: self.rip.clone(), source: self.source.clone() };
                super::game::hold_if_suspect(&cx, dump, &roms, looked > 0 && failed.is_empty(), &reread).await?
            } else {
                Some(dump)
            };
            if let Some(dump) = dump {
                outlet.send(dump).await;
            }
        }
        Ok(())
    }
}

// ---- CHD and Plain image -------------------------------------------------------------------------

pub struct Chd;

#[async_trait]
impl Step for Chd {
    fn info(&self) -> StepInfo {
        chd_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(dump) = inlet.recv().await {
            let src = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
            // Named for what Check found, else the label.
            let name = super::safe(&dump_fields(dump.context()).name);
            let dest = cx.work()?.join("chd").join(format!("{name}.chd"));
            let (codec, hunk) = (cx.text("codecs"), cx.text("hunk"));
            let _core = cx.lane(LaneKind::Encode).await;
            cx.item(&dump, ItemState::Busy, "compressing");
            cx.status("", "Compressing");
            cx.log(format!("compressing, {}", chd::describe(&chd::codecs(&codec, chd::is_cd(&src)))));
            let made = {
                let (cx2, d, src, dest) = (cx.clone(), dump.clone(), src.clone(), dest.clone());
                tokio::task::spawn_blocking(move || {
                    chd::create(
                        &src,
                        &dest,
                        &codec,
                        &hunk,
                        |p| {
                            cx2.progress(Some(p));
                            cx2.item(&d, ItemState::Busy, format!("compressing {}%", (p * 100.0) as u32));
                        },
                        &|| cx2.stopped(),
                    )
                })
                .await?
            };
            if let Err(e) = made {
                if cx.stopped() {
                    return Err(e);
                }
                cx.item(&dump, ItemState::Bad, format!("Couldn't make the file: {e}"));
                cx.log(format!("CHD: {e}"));
                continue;
            }
            let tracks = image_files(&src);
            let made_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            let image_size: u64 = tracks.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum();
            let mut note = format!("{}, {}% of the image", media::size(made_size), (made_size * 100).checked_div(image_size).unwrap_or(0));
            // Always: the CHD must give back exactly the image.
            {
                cx.status("", "Verifying");
                let (cx2, d, src, made, files) = (cx.clone(), dump.clone(), src.clone(), dest.clone(), tracks);
                let checked = tokio::task::spawn_blocking(move || {
                    let known = |name: &str| d.context().get(Subject::Disc, &format!("sha1 {name}")).map(str::to_owned);
                    chd::verify(
                        &made,
                        &src,
                        &files,
                        &known,
                        |p| {
                            cx2.progress(Some(p));
                            cx2.item(&d, ItemState::Busy, format!("verifying {}%", (p * 100.0) as u32));
                        },
                        &|| cx2.stopped(),
                    )
                })
                .await?;
                match checked {
                    Ok(v) => note = format!("{note}, {v}"),
                    Err(e) => {
                        if cx.stopped() {
                            return Err(e);
                        }
                        cx.item(&dump, ItemState::Bad, format!("Couldn't make the file: {e}"));
                        cx.log(format!("CHD verify: {e}"));
                        let _ = std::fs::remove_file(&dest);
                        continue;
                    }
                }
            }
            cx.progress(None);
            cx.log(format!("CHD: {} ({note})", dest.display()));
            cx.item(&dump, ItemState::Ok, note.clone());
            cx.outcome(media::size(made_size), (made_size * 100).checked_div(image_size).map(|p| format!("{p} %")).unwrap_or_default(), "");
            outlet.send(dump.becomes(ItemKind::ImageFile, Some(dest))).await;
        }
        Ok(())
    }
}

pub struct Keep;

#[async_trait]
impl Step for Keep {
    fn info(&self) -> StepInfo {
        keep_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(dump) = inlet.recv().await {
            let main = dump.path.clone().ok_or_else(|| anyhow!("{} has no file", dump.label))?;
            // Named for what Check found, else the label, as the CHD is.
            let name = super::safe(&dump_fields(dump.context()).name);
            let dir = cx.work()?.join("keep");
            let named = {
                let (main, dir) = (main.clone(), dir.clone());
                tokio::task::spawn_blocking(move || name_set(&main, &dir, &name)).await?
            };
            let named = match named {
                Ok(m) => m,
                Err(e) => {
                    if cx.stopped() {
                        return Err(e);
                    }
                    cx.item(&dump, ItemState::Bad, format!("Couldn't name the files: {e}"));
                    cx.log(format!("plain image: {e}"));
                    continue;
                }
            };
            let n = redumper::dump_files(&named).len();
            cx.log(format!("plain image: {} ({n} files)", named.display()));
            cx.item(&dump, ItemState::Ok, format!("{n} files"));
            cx.status(image_size(&named), "");
            outlet.send(dump.becomes(ItemKind::ImageFile, Some(named))).await;
        }
        Ok(())
    }
}

/// Each file of a dump and the name Redump gives it, for a disc called
/// `name`: `name.cue`; one track `name.bin`, several `name (Track 1).bin`
/// on, as `name (Track 01).bin` from ten tracks up (every entry in
/// Redump's PlayStation list is named so); `name.iso`; `name.log`.
pub fn redump_names(main: &Path, name: &str) -> Vec<(PathBuf, String)> {
    let files = redumper::dump_files(main);
    let ext = |f: &Path| f.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let tracks = files.iter().filter(|f| ext(f) == "bin").count();
    let width = if tracks >= 10 { 2 } else { 1 };
    let mut n = 0;
    files
        .into_iter()
        .map(|f| {
            let new = match ext(&f).as_str() {
                "bin" if tracks > 1 => {
                    n += 1;
                    format!("{name} (Track {n:0width$}).bin")
                }
                e => format!("{name}.{e}"),
            };
            (f, new)
        })
        .collect()
}

/// A cue sheet with each FILE line naming its file's new name: the name
/// between the quotes changed, every other byte as it was (line ends
/// included). Names not in `names` stay.
pub fn rename_in_cue(cue: &str, names: &[(String, String)]) -> String {
    let mut out = String::with_capacity(cue.len());
    for line in cue.split_inclusive('\n') {
        let t = line.trim_start();
        let lead = &line[..line.len() - t.len()];
        let renamed = t.strip_prefix("FILE \"").and_then(|rest| {
            let (old, after) = rest.split_once('"')?;
            let new = &names.iter().find(|(o, _)| o == old)?.1;
            Some(format!("{lead}FILE \"{new}\"{after}"))
        });
        out.push_str(renamed.as_deref().unwrap_or(line));
    }
    out
}

/// Puts a dump's files into `dir` under Redump's names for `name`
/// ([`redump_names`]): each linked where it can be (no second copy of a
/// disc), else copied; the cue sheet written anew to name the tracks by
/// theirs. The dump stays as it is, for the steps that read it. Gives the
/// new main file.
pub fn name_set(main: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let names = redump_names(main, name);
    let pairs: Vec<(String, String)> = names.iter().map(|(f, n)| (f.file_name().unwrap_or_default().to_string_lossy().into_owned(), n.clone())).collect();
    let mut new_main = None;
    for (f, n) in &names {
        let to = dir.join(n);
        let _ = std::fs::remove_file(&to);
        if f == main {
            new_main = Some(to.clone());
        }
        if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue")) {
            let cue = std::fs::read_to_string(f)?;
            std::fs::write(&to, rename_in_cue(&cue, &pairs))?;
        } else if std::fs::hard_link(f, &to).is_err() {
            std::fs::copy(f, &to)?;
        }
    }
    new_main.ok_or_else(|| anyhow!("{} isn't part of the dump", main.display()))
}

/// Copies a dump's files into `dir`, each under a temporary name first, so
/// a half-copied file never sits there under its real one. Gives where the
/// main file went.
pub async fn place_dump(main: &Path, dir: &Path) -> Result<PathBuf> {
    let files = if main.extension().is_some_and(|e| e.eq_ignore_ascii_case("chd")) { vec![main.to_path_buf()] } else { redumper::dump_files(main) };
    tokio::fs::create_dir_all(dir).await?;
    for f in &files {
        let name = f.file_name().ok_or_else(|| anyhow!("{} has no name", f.display()))?;
        super::place::copy(f, &dir.join(name)).await?;
    }
    Ok(dir.join(main.file_name().unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_check_says() {
        assert_eq!(verdict(Some(("Encarta 98 (USA)", "IBM PC")), 1, &[]).0, "Verified: Encarta 98 (USA)");
        assert_eq!(verdict(None, 3, &[]), ("Not in Redump".into(), "Not in Redump".into(), "looked in 3 systems".into()));
        assert_eq!(verdict(None, 0, &["IBM PC"]).1, "Couldn't check");
    }

    #[test]
    fn what_image_says_about_an_image() {
        assert_eq!(image_note("682 MB", false, 0, false, &[]), "✓ 682 MB, read cleanly");
        assert_eq!(image_note("682 MB", false, 0, true, &[("EDC", 0)]), "✓ 682 MB, bad parts re-read");
        // Silent Hill: no read the drive admitted to, 18 sectors wrong on inspection.
        let silent_hill = [("invalid sync", 0), ("MSF", 70), ("ECC", 6), ("EDC", 18)];
        assert_eq!(image_note("630 MB", false, 0, false, &silent_hill), "read, but 70 sectors came back wrong");
        assert_eq!(image_note("630 MB", true, 1, false, &[]), "kept anyway, 1 unreadable sector left blank");
        assert_eq!(image_note("630 MB", true, 0, false, &[]), "kept anyway, 630 MB", "forcing nothing isn't reading cleanly either");
    }

    #[test]
    fn the_cache_is_beside_the_runs() {
        assert_eq!(redump_cache(Path::new("/home/u/.cache/disc-rip-trip/runs/data-discs-1")), PathBuf::from("/home/u/.cache/disc-rip-trip/cache/redump"));
    }

    /// A Folder's names in a recipe, as the run reads them: from what
    /// reaches it, and what it was saved with.
    fn names_in(root: &drt_engine::Node, id: &str, saved: Option<&str>) -> (Vec<&'static str>, Option<Value>) {
        let reg = crate::real();
        let flows = drt_engine::flows_in(root, &reg);
        let flow = flows.get(id);
        let info = place_info("folder", "Folder");
        let spec = info.params.iter().find(|p| p.key == "names").unwrap();
        let presets = spec.fitted(flow).map(|f| f.presets.iter().map(|(n, _)| *n).collect()).unwrap_or_default();
        let set = saved.map_or(drt_engine::Setting::Default, |v| drt_engine::Setting::chosen(Value::from(v)));
        (presets, spec.resolve_at(&set, flow))
    }

    #[test]
    fn a_folder_s_names_follow_what_reaches_it() {
        use crate::recipes;
        let at = |r: drt_engine::Recipe, id: &str, saved| names_in(&r.root, id, saved);
        // Tracks: the music layouts.
        let (presets, v) = at(recipes::cd(), "lib1", None);
        assert_eq!(v, Some(Value::from(template::DEFAULT)));
        assert_eq!(presets, ["Artist / Album / 01 - Title", "Sorted artist / Album / 01 Artist - Title", "Album / 01 - Title", "01 - Title"]);
        // A game's image: a folder for the game under its system, as the
        // Games recipe files it.
        let (presets, v) = at(recipes::games(), "g_lib", None);
        assert_eq!(v, Some(Value::from("{system}/{title}")));
        assert_eq!(presets, ["System / Game", "System / Name", "Name", "Date and disc name"]);
        // Any other disc's image: by name.
        let (presets, v) = at(recipes::data(), "d_lib", None);
        assert_eq!(v, Some(Value::from("{name}")));
        assert!(!presets.contains(&"System / Game") && !presets.iter().any(|p| p.contains("Artist")));
        // Films and episodes: Jellyfin's.
        let (presets, v) = at(recipes::video(), "v_lib", None);
        assert_eq!(v, Some(Value::from("Jellyfin")));
        assert_eq!(presets, ["Jellyfin", "Jellyfin film", "Jellyfin episode"]);
    }

    #[test]
    fn an_image_saved_on_the_music_default_is_filed_by_its_own() {
        use crate::recipes;
        for saved in [template::DEFAULT, "Artist / Album / 01 - Title", ""] {
            assert_eq!(names_in(&recipes::data().root, "d_lib", Some(saved)).1, Some(Value::from(template::DUMP_DEFAULT)), "{saved:?}");
            assert_eq!(names_in(&recipes::games().root, "g_lib", Some(saved)).1, Some(Value::from(template::GAME_FOLDER)), "{saved:?}");
            assert_eq!(names_in(&recipes::video().root, "v_lib", Some(saved)).1, Some(Value::from("Jellyfin")), "{saved:?}");
        }
        // What you chose stays, another kind's default included, and a
        // game's disc in a folder of its own.
        assert_eq!(names_in(&recipes::games().root, "g_lib", Some("{system}/{name}")).1, Some(Value::from("{system}/{name}")));
        assert_eq!(names_in(&recipes::data().root, "d_lib", Some("System / Name")).1, Some(Value::from("System / Name")));
        assert_eq!(names_in(&recipes::games().root, "g_lib", Some("{name}")).1, Some(Value::from("{name}")));
        // And on music, the music default is just the default.
        assert_eq!(names_in(&recipes::cd().root, "lib1", Some(template::DEFAULT)).1, Some(Value::from(template::DEFAULT)));
    }

    #[test]
    fn a_folder_s_card_says_its_layout_by_name() {
        let info = place_info("folder", "Folder");
        let words = info.params.iter().find(|p| p.key == "names").and_then(|p| p.on_card).and_then(|c| c.value).unwrap();
        let says = |v: &str| words(&|k| (k == "names").then(|| Value::from(v)));
        assert_eq!(says("{system}/{name}"), "System / Name");
        assert_eq!(says("{system}/{title}"), "System / Game");
        assert_eq!(says("{name}"), "Name");
        assert_eq!(says("Jellyfin"), "Jellyfin");
        assert_eq!(says(template::DEFAULT), "Artist / Album / 01 - Title");
        assert_eq!(says("{year}/{title}"), "{year}/{title}");
    }

    #[test]
    fn a_plain_image_is_named_as_redump_names_it() {
        let base = std::env::current_dir().unwrap().join("../../target/drt-test/redump-names");
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("image");
        std::fs::create_dir_all(&src).unwrap();
        // One track, as redumper names it, the cue's lines ending CRLF.
        std::fs::write(src.join("KOUDELKA_1.cue"), "FILE \"KOUDELKA_1.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n").unwrap();
        std::fs::write(src.join("KOUDELKA_1.bin"), "data").unwrap();
        std::fs::write(src.join("KOUDELKA_1.log"), "log").unwrap();
        std::fs::write(src.join("KOUDELKA_1.scram"), "work").unwrap();
        let out = base.join("keep");
        let main = name_set(&src.join("KOUDELKA_1.cue"), &out, "Koudelka (Europe) (Disc 1)").unwrap();
        assert_eq!(main, out.join("Koudelka (Europe) (Disc 1).cue"));
        let cue = std::fs::read_to_string(&main).unwrap();
        assert_eq!(cue, "FILE \"Koudelka (Europe) (Disc 1).bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n");
        assert_eq!(std::fs::read_to_string(out.join("Koudelka (Europe) (Disc 1).bin")).unwrap(), "data");
        assert_eq!(std::fs::read_to_string(out.join("Koudelka (Europe) (Disc 1).log")).unwrap(), "log");
        let mut there: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        there.sort();
        assert_eq!(there.len(), 3, "{there:?}");
        assert!(src.join("KOUDELKA_1.bin").is_file(), "the dump stays for the steps that read it");
        // Done again (a second disc of the same name), it replaces them.
        name_set(&src.join("KOUDELKA_1.cue"), &out, "Koudelka (Europe) (Disc 1)").unwrap();
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn several_tracks_are_numbered_as_redump_numbers_them() {
        let base = std::env::current_dir().unwrap().join("../../target/drt-test/redump-tracks");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let cue = |n: usize| (1..=n).map(|k| format!("FILE \"disc (Track {k}).bin\" BINARY\n  TRACK {k:02} AUDIO\n")).collect::<String>();
        for n in [2, 12] {
            std::fs::write(base.join("disc.cue"), cue(n)).unwrap();
            let names: Vec<String> = redump_names(&base.join("disc.cue"), "Alien Trilogy (Germany)").into_iter().map(|(_, n)| n).collect();
            assert_eq!(names[0], "Alien Trilogy (Germany).cue");
            assert_eq!(names.len(), n + 1);
            if n == 2 {
                assert_eq!(names[1..], ["Alien Trilogy (Germany) (Track 1).bin", "Alien Trilogy (Germany) (Track 2).bin"]);
            } else {
                assert_eq!(names[1], "Alien Trilogy (Germany) (Track 01).bin");
                assert_eq!(names[12], "Alien Trilogy (Germany) (Track 12).bin");
            }
        }
        let pairs = [("disc (Track 1).bin".to_owned(), "G (Track 1).bin".to_owned())];
        assert_eq!(rename_in_cue("FILE \"disc (Track 1).bin\" BINARY\nFILE \"other.bin\" BINARY\n", &pairs), "FILE \"G (Track 1).bin\" BINARY\nFILE \"other.bin\" BINARY\n");
        // An ISO is the name and .iso.
        std::fs::write(base.join("DVD.iso"), "").unwrap();
        assert_eq!(redump_names(&base.join("DVD.iso"), "Encarta").into_iter().map(|(_, n)| n).collect::<Vec<_>>(), ["Encarta.iso"]);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn a_dump_set_is_filed_together() {
        let base = std::env::current_dir().unwrap().join("../../target/drt-test/place-dump");
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("work");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("D.cue"), "FILE \"D (Track 1).bin\" BINARY\n  TRACK 01 MODE1/2352\nFILE \"D (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n").unwrap();
        for f in ["D (Track 1).bin", "D (Track 2).bin", "D.log", "D.scram", "D.state"] {
            std::fs::write(src.join(f), f).unwrap();
        }
        let lib = base.join("lib/Disc");
        let placed = place_dump(&src.join("D.cue"), &lib).await.unwrap();
        assert_eq!(placed, lib.join("D.cue"));
        let mut there: Vec<String> = std::fs::read_dir(&lib).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        there.sort();
        assert_eq!(there, ["D (Track 1).bin", "D (Track 2).bin", "D.cue", "D.log"], "redumper's working files stay behind");
        std::fs::remove_dir_all(base).unwrap();
    }
}
