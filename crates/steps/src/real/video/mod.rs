//! DVD and Blu-ray video (DESIGN-UX.md, "Other kinds of disc", 3):
//! `Drive, Video disc, TMDb, Choose, Rip, then Transcode, Folder; and Next
//! disc`.
//!
//! - **Video disc** lists the titles with MakeMKV, marks duplicates, and
//!   guesses film or episodes and which titles to rip.
//! - **TMDb** searches by the volume label and offers films, or a series'
//!   season with its episodes.
//! - **Choose** settles which, then which title is the film or which
//!   episode, asking only when unsure.
//! - **Rip** reads only those titles, and checks each is whole.
//! - **Transcode** or **Remux** make the files; **Folder** names them for
//!   Jellyfin.
//!
//! The step infos live here and are shared with the simulated steps, so a
//! recipe runs on either.
//!
//! What the steps leave in the context (the keys are in [`facts`]): the
//! disc's `label`, `media`, `parts` (how many titles), `minlength`, the
//! scan's guess `picked` and `disc_kind`; per title (`Subject::Part`, the
//! MakeMKV id + 1) its `length`, `chapters`, `bytes`, `audio`,
//! `subtitles`, `duplicate_of` and `makemkv_id`. A TMDb candidate carries
//! `title`, `year`, `tmdbid`, `video_kind`, and for a season `season`,
//! `ep.N` and `ep_runtime.N`. Choose adds `rip` (the parts to rip) and per
//! title `role` (film, episode, extra), `season`, `episode`,
//! `episode_title`.

pub mod choose;
pub mod key;
pub mod makemkv;
pub mod matching;
pub mod sim;
pub mod tmdb;
pub mod transcode;

use crate::template;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use drt_engine::setting::ParamKind;
use drt_engine::{Context, Cx, Group, Inlet, Item, ItemKind, ItemState, LaneKind, Outlet, ParamSpec, Registry, Step, StepInfo, StepState, Subject};
use makemkv::{Kind, Launcher};
use std::path::{Path, PathBuf};

pub mod facts {
    pub const LABEL: &str = "label";
    pub const MEDIA: &str = "media";
    pub const PARTS: &str = "parts";
    pub const MINLENGTH: &str = "minlength";
    pub const PICKED: &str = "picked";
    pub const RIP: &str = "rip";
    pub const DISC_KIND: &str = "disc_kind";
    pub const SELECTED: &str = "selected";
    pub const LENGTH: &str = "length";
    pub const CHAPTERS: &str = "chapters";
    pub const BYTES: &str = "bytes";
    pub const AUDIO: &str = "audio";
    pub const SUBTITLES: &str = "subtitles";
    pub const DUPLICATE_OF: &str = "duplicate_of";
    pub const MAKEMKV_ID: &str = "makemkv_id";
    pub const ROLE: &str = "role";
    pub const SEASON: &str = "season";
    pub const EPISODE: &str = "episode";
    pub const EPISODE_TITLE: &str = "episode_title";
}

/// Every video title item's id starts with this: `title-3`. How a Folder
/// tells a title from a track.
pub const ITEM_PREFIX: &str = "title-";

pub fn register(r: &mut Registry) {
    r.add(VideoDisc);
    r.add(Tmdb { fixtures: None });
    r.add(choose::ChooseVideo);
    r.add(RipVideo);
    r.add(transcode::Make { video: true });
    r.add(transcode::Make { video: false });
}

// ---- what each step is ------------------------------------------------------------------------

pub fn video_disc_info() -> StepInfo {
    StepInfo::new("video-disc", "Video disc", Group::Kind, ItemKind::Disc, ItemKind::VideoDisc)
        .gives(&["titles"])
        .from("list the films and episodes on it")
        // MakeMKV hides titles shorter than this; Rip uses the same, since
        // MakeMKV numbers titles within what it listed.
        .param(
            ParamSpec::new("min_length", "Skip videos shorter than (seconds)", ParamKind::Range { min: 0, max: 600 })
                .default(60)
                .on_card_as("Skips under", &[], |g| format!("{} s", g("min_length").and_then(|v| v.as_i64()).unwrap_or(0))),
        )
}

pub fn tmdb_info() -> StepInfo {
    StepInfo::new("tmdb", "TMDb", Group::LookUp, ItemKind::VideoDisc, ItemKind::VideoDisc)
        .gives(&["candidates"])
        .unless(&["release"])
        .right_after(&["video-disc"])
        .from("find the film or show online")
        // One key for the app, set on any TMDb tile.
        .param(ParamSpec::new("api_key", "API key", ParamKind::Text).app_wide().help("Free from themoviedb.org, under Settings, API."))
}

pub fn rip_info() -> StepInfo {
    StepInfo::new("rip-video", "Rip", Group::Read, ItemKind::VideoDisc, ItemKind::Title).gives(&["read"]).from("copy the chosen films and episodes").retry("again", "Read again", "a few min").retry(
        "leave_out",
        "Skip this video",
        "",
    )
}

// ---- reading the context ------------------------------------------------------------------------

fn parse_parts(s: &str) -> Vec<u32> {
    s.split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

/// The titles to rip: Choose's, or failing that the scan's guess.
pub fn to_rip(c: &Context) -> Vec<u32> {
    c.get(Subject::Disc, facts::RIP).or(c.get(Subject::Disc, facts::PICKED)).map(parse_parts).unwrap_or_default()
}

/// How many titles the steps after Rip will see.
pub fn selected(c: &Context) -> u32 {
    c.get(Subject::Disc, facts::SELECTED).and_then(|v| v.parse().ok()).unwrap_or_else(|| to_rip(c).len() as u32).max(1)
}

/// The titles as matching sees them.
pub fn titles(c: &Context) -> Vec<matching::Disc> {
    (1..=c.parts())
        .map(|p| matching::Disc { part: p, secs: c.get(Subject::Part(p), facts::LENGTH).and_then(|v| v.parse().ok()).unwrap_or(0), duplicate: c.get(Subject::Part(p), facts::DUPLICATE_OF).is_some() })
        .collect()
}

/// `1:57:30`, `44:12`.
pub fn hms(secs: u32) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// What a title is, to show and to tag: *S03E01 Mr. Ruggerio's
/// Neighborhood*, *Blade Runner*, or nothing before Choose.
pub fn display_title(c: &Context, part: u32) -> Option<String> {
    let p = Subject::Part(part);
    match c.get(p, facts::ROLE)? {
        "film" => c.get(Subject::Disc, "title").map(str::to_owned),
        "episode" => {
            let (s, e) = (c.get(p, facts::SEASON).unwrap_or("0"), c.get(p, facts::EPISODE).unwrap_or("0"));
            let name = c.get(p, facts::EPISODE_TITLE).unwrap_or("");
            Some(format!("S{:0>2}E{:0>2} {name}", s, e).trim().to_owned())
        }
        _ => None,
    }
}

/// A title's row label: what it is, or *Video 3*.
pub fn label(c: &Context, part: u32) -> String {
    display_title(c, part).unwrap_or_else(|| format!("Video {part}"))
}

/// The label's search words, in title case, for a disc nobody identified.
fn from_label(label: &str) -> String {
    let q = tmdb::parse_label(label).query;
    let q = if q.is_empty() { label.to_owned() } else { q };
    q.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a title's path template can use.
pub fn video_fields(c: &Context, part: u32) -> template::VideoFields {
    let p = Subject::Part(part);
    let get = |k| c.get(Subject::Disc, k).unwrap_or("").to_owned();
    let label = get(facts::LABEL);
    let title = c.get(Subject::Disc, "title").map(str::to_owned).unwrap_or_else(|| from_label(&label));
    let num = |k| c.get(p, k).and_then(|v| v.parse().ok());
    let role = c.get(p, facts::ROLE);
    let extra = match role {
        Some("film") | Some("episode") => None,
        // Ripped without Choose's say: the one title is the film; of
        // several, each is kept as an extra under its number.
        None if to_rip(c).len() <= 1 => None,
        _ => Some(format!("Title {part}")),
    };
    template::VideoFields {
        title,
        year: get("year"),
        tmdbid: get("tmdbid"),
        season: if role == Some("episode") { num(facts::SEASON) } else { None },
        episode: if role == Some("episode") { num(facts::EPISODE) } else { None },
        episode_title: c.get(p, facts::EPISODE_TITLE).unwrap_or("").to_owned(),
        label,
        extra,
    }
}

/// Where a title goes under `root`, by the Folder's template.
pub fn destination(c: &Context, names: &str, root: &Path, part: u32, ext: &str) -> PathBuf {
    let f = video_fields(c, part);
    // A series' extra goes beside the show, so it's the series layout too.
    let kind = c.get(Subject::Disc, "video_kind").or(c.get(Subject::Disc, facts::DISC_KIND));
    let series = f.episode.is_some() || kind == Some("series");
    let rel = template::render_video(template::video_template(names, series), &f);
    root.join(format!("{}.{ext}", rel.display()))
}

/// ` (2)`, ` (3)`… when something is there already, so filing never
/// destroys a file.
pub fn free(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_owned();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "mkv".into());
    (2..1000).map(|n| path.with_file_name(format!("{stem} ({n}).{ext}"))).find(|p| !p.exists()).unwrap_or_else(|| path.to_owned())
}

/// Files a title: copied under a temporary name and renamed, so a
/// half-copied file never sits in the library under its real name.
pub async fn place(names: &str, root: &Path, file: &Item) -> Result<PathBuf> {
    let src = file.path.clone().ok_or_else(|| anyhow!("{} has no file", file.label))?;
    let part = file.index.ok_or_else(|| anyhow!("{} isn't a title", file.label))?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("mkv").to_owned();
    let dest = free(&destination(file.context(), names, root, part, &ext));
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    super::place::copy(&src, &dest).await?;
    Ok(dest)
}

/// Writes what a scan found onto the disc and the tile; gives the tile's
/// line. Shared with the simulated Video disc.
pub fn record(cx: &Cx, c: &mut Context, disc: &makemkv::Disc, minlength: u32) -> (String, String) {
    let dups = makemkv::duplicates(&disc.titles);
    let guess = tmdb::parse_label(&disc.label);
    let kind = makemkv::guess_kind(&disc.titles, guess.season.is_some() || guess.disc.is_some());
    // Parts follow the listing (MakeMKV's own ids are kept for the rip).
    let part_of = |id: u32| disc.titles.iter().position(|t| t.id == id).map(|i| i as u32 + 1);
    let picked: Vec<String> = makemkv::select(&disc.titles, kind).iter().filter_map(|t| part_of(t.id)).map(|p| p.to_string()).collect();
    let src = "MakeMKV";
    {
        c.add(Subject::Disc, facts::LABEL, disc.label.clone(), src);
        c.add(Subject::Disc, facts::MEDIA, disc.media.clone(), src);
        c.add(Subject::Disc, facts::PARTS, disc.titles.len().to_string(), src);
        c.add(Subject::Disc, facts::MINLENGTH, minlength.to_string(), src);
        c.add(Subject::Disc, facts::DISC_KIND, if kind == Kind::Series { "series" } else { "film" }, src);
        c.add(Subject::Disc, facts::PICKED, picked.join(","), src);
        for (i, t) in disc.titles.iter().enumerate() {
            // Parts follow the listing; MakeMKV's own number is kept for the rip.
            let p = Subject::Part(i as u32 + 1);
            c.add(p, facts::MAKEMKV_ID, t.id.to_string(), src);
            c.add(p, facts::LENGTH, t.secs.to_string(), src);
            c.add(p, facts::CHAPTERS, t.chapters.to_string(), src);
            c.add(p, facts::BYTES, t.bytes.to_string(), src);
            c.add(p, facts::AUDIO, t.audio().join(","), src);
            c.add(p, facts::SUBTITLES, t.subtitles().join(","), src);
            if let Some(first) = dups.get(&t.id).and_then(|f| disc.titles.iter().position(|x| x.id == *f)) {
                c.add(p, facts::DUPLICATE_OF, (first + 1).to_string(), src);
            }
        }
    }
    for (i, t) in disc.titles.iter().enumerate() {
        let part = i as u32 + 1;
        let item = Item::new(format!("{ITEM_PREFIX}{part}"), ItemKind::Title, format!("Video {part}"), Some(part), None);
        let mut note = vec![hms(t.secs), format!("{} chapter{}", t.chapters, if t.chapters == 1 { "" } else { "s" })];
        if t.bytes > 0 {
            note.push(super::media::size(t.bytes));
        }
        if !t.audio().is_empty() {
            note.push(t.audio().join(" "));
        }
        let dup = dups.get(&t.id).and_then(|f| disc.titles.iter().position(|x| x.id == *f));
        if let Some(f) = dup {
            note.push(format!("duplicate of video {}", f + 1));
        }
        let state = if picked.contains(&part.to_string()) { ItemState::Ok } else { ItemState::Skipped };
        cx.item(&item, state, note.join(", "));
    }
    // How many of its titles it keeps; one kept, which and how long.
    let n = disc.titles.len();
    let detail = match picked.as_slice() {
        [one] => one.parse::<usize>().ok().and_then(|p| disc.titles.get(p - 1)).map(|t| format!("Video {one} · {}", hms(t.secs))).unwrap_or_default(),
        _ => String::new(),
    };
    (format!("{} of {n} video{}", picked.len(), if n == 1 { "" } else { "s" }), detail)
}

/// *1 title*, *3 titles*.
pub fn titles_word(n: usize) -> String {
    format!("{n} video{}", if n == 1 { "" } else { "s" })
}

fn device(c: &Context) -> Result<String> {
    c.get(Subject::Disc, "device").map(str::to_owned).ok_or_else(|| anyhow!("no drive before this step"))
}

/// Runs `f` with MakeMKV; if it says it isn't registered, refreshes the
/// beta key (only then, as the old app did) and tries once more.
async fn with_key<T, F, Fut>(cx: &Cx, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    match f().await {
        Err(e) if e.downcast_ref::<makemkv::NotRegistered>().is_some() => {
            cx.status("Refreshing MakeMKV's key", "the beta key has expired");
            let k = key::refresh().await.map_err(|r| anyhow!("MakeMKV isn't registered, and its key couldn't be refreshed: {r}"))?;
            cx.log(format!("installed MakeMKV beta key {}…", &k[..8.min(k.len())]));
            f().await
        }
        r => r,
    }
}

// ---- Video disc -------------------------------------------------------------------------------

pub struct VideoDisc;

#[async_trait]
impl Step for VideoDisc {
    fn info(&self) -> StepInfo {
        video_disc_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(disc) = inlet.recv().await {
            let dev = device(disc.context())?;
            let Some(launcher) = makemkv::locate().await else {
                cx.state(StepState::Bad);
                cx.status("MakeMKV isn't installed", makemkv::install_hint());
                return Ok(());
            };
            let minlength = cx.param("min_length").and_then(|v| v.as_u64()).unwrap_or(60) as u32;
            cx.status("", "Listing videos");
            let dir = cx.work()?.join("scan");
            let found = {
                let _drive = cx.lane(LaneKind::Drive).await;
                cx.drive_says(StepState::Busy, "Reading", "the videos");
                let r = with_key(&cx, || makemkv::scan(&launcher, &dev, minlength, &dir, cx.cancelled())).await;
                cx.drive_says(StepState::Idle, "Waiting", "");
                r
            };
            let found = match found {
                Ok(d) => d,
                Err(e) if cx.stopped() => return Err(e),
                Err(e) => {
                    cx.state(StepState::Bad);
                    cx.status("Couldn't read the videos", e.to_string());
                    cx.log(format!("scan: {e}"));
                    return Ok(());
                }
            };
            let mut found = found;
            if found.media.is_empty() {
                // MakeMKV didn't say; the drive does.
                found.media = super::media::current_profile(&dev).and_then(super::media::profile_media).unwrap_or("").to_owned();
            }
            let mut d = disc.becomes(ItemKind::VideoDisc, None);
            let line = record(&cx, d.context_mut(), &found, minlength);
            cx.status(line.0, line.1);
            cx.log(format!("{} titles on {} ({})", found.titles.len(), found.label, found.media));
            d.label = found.label.clone();
            outlet.send(d).await;
        }
        Ok(())
    }
}

// ---- TMDb -------------------------------------------------------------------------------------

/// TMDb, from the network, or (`fixtures`) from canned answers for the
/// simulation, through the same search, scoring and season fitting.
pub struct Tmdb {
    pub fixtures: Option<fn() -> Box<dyn tmdb::Source>>,
}

#[async_trait]
impl Step for Tmdb {
    fn info(&self) -> StepInfo {
        tmdb_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let (label, titles, kind) = {
                let c = disc.context();
                let kind = if c.get(Subject::Disc, facts::DISC_KIND) == Some("series") { Kind::Series } else { Kind::Film };
                (c.get(Subject::Disc, facts::LABEL).unwrap_or("").to_owned(), titles(c), kind)
            };
            let guess = tmdb::parse_label(&label);
            cx.status("", format!("Searching “{}”", guess.query));
            let src: Box<dyn tmdb::Source> = match self.fixtures {
                Some(f) => {
                    cx.sleep(800).await?;
                    f()
                }
                None => Box::new(tmdb::Http { key: cx.text("api_key") }),
            };
            let found = {
                let _net = cx.lane(LaneKind::Network("TMDb".into())).await;
                tmdb::candidates(src.as_ref(), &guess, &titles, kind).await
            };
            match found {
                Ok(c) if c.is_empty() => cx.status("Nothing found", if guess.query.is_empty() { "the label says nothing".to_owned() } else { format!("nothing for “{}”", guess.query) }),
                Ok(c) => {
                    // In the job's unit, with the query (guessed from the
                    // label, so worth seeing when the matches are bad).
                    let films = c.iter().filter(|x| x.id.starts_with("tmdb-movie")).count();
                    let say = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
                    let outcome = match (films, c.len() - films) {
                        (f, 0) => say(f, "film", "films"),
                        (0, s) => say(s, "season", "seasons"),
                        (f, s) => format!("{}, {}", say(f, "film", "films"), say(s, "season", "seasons")),
                    };
                    cx.outcome(outcome, if guess.query.is_empty() { String::new() } else { format!("“{}”", guess.query) }, "");
                    disc.context_mut().offer(c);
                }
                // A lookup failing doesn't stop the disc; Choose asks.
                Err(e) => {
                    cx.state(StepState::Bad);
                    cx.status("No answer", e.to_string());
                    cx.log(format!("TMDb: {e}"));
                }
            }
            outlet.send(disc).await;
        }
        Ok(())
    }
}

// ---- Rip --------------------------------------------------------------------------------------

/// The items Rip will make, labelled with what each is, each knowing
/// what the disc knows.
pub fn rip_items(disc: &Item) -> Vec<Item> {
    let c = disc.context();
    to_rip(c).into_iter().map(|p| Item::new(format!("{ITEM_PREFIX}{p}"), ItemKind::Title, label(c, p), Some(p), None).knowing(disc)).collect()
}

pub struct RipVideo;

#[async_trait]
impl Step for RipVideo {
    fn info(&self) -> StepInfo {
        rip_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let Some(mut disc) = inlet.recv().await else { return Ok(()) };
        let dev = device(disc.context())?;
        let launcher = makemkv::locate().await.ok_or(makemkv::Missing)?;
        let n = to_rip(disc.context()).len();
        if n == 0 {
            cx.status("Nothing to rip", "no videos chosen");
            return Ok(());
        }
        disc.context_mut().add(Subject::Disc, facts::SELECTED, n.to_string(), "Rip");
        let items = rip_items(&disc);
        for i in &items {
            cx.item(i, ItemState::Waiting, "");
        }
        let total = items.len();
        let mut done = 0;
        let mut attempts: std::collections::HashMap<String, u32> = Default::default();
        let mut bad: Vec<Item> = Vec::new();
        {
            let _drive = cx.lane(LaneKind::Drive).await;
            for (i, item) in items.iter().enumerate() {
                cx.status(titles_word(done), item.label.clone());
                match self.rip(&cx, &launcher, &dev, item, &mut attempts, i, total).await {
                    Ok(file) => {
                        done += 1;
                        outlet.send(item.becomes(ItemKind::Title, Some(file))).await;
                    }
                    Err(e) if cx.stopped() => return Err(e),
                    Err(e) => {
                        cx.item(item, ItemState::Bad, format!("Couldn't copy the video: {e}"));
                        cx.log(format!("{}: {e}", item.label));
                        bad.push(item.clone());
                    }
                }
            }
            cx.drive_says(StepState::Idle, "Waiting", "");
        }
        cx.progress(None);
        while !bad.is_empty() {
            cx.state(StepState::Bad);
            cx.status(titles_word(done), format!("{} couldn't be read", bad[0].label));
            let Some(req) = cx.next_retry().await else { return Ok(()) };
            let Some(pos) = bad.iter().position(|i| i.id == req.item) else { continue };
            let item = bad.remove(pos);
            if req.option == "leave_out" {
                cx.item(&item, ItemState::Skipped, "skipped");
                continue;
            }
            cx.state(StepState::Busy);
            let _drive = cx.lane(LaneKind::Drive).await;
            let r = self.rip(&cx, &launcher, &dev, &item, &mut attempts, 0, 1).await;
            cx.drive_says(StepState::Idle, "Waiting", "");
            match r {
                Ok(file) => {
                    done += 1;
                    outlet.send(item.becomes(ItemKind::Title, Some(file))).await;
                }
                Err(e) if cx.stopped() => return Err(e),
                Err(e) => {
                    cx.item(&item, ItemState::Bad, format!("Couldn't copy the video: {e}"));
                    bad.push(item);
                }
            }
        }
        cx.state(StepState::Busy);
        let _ = total;
        cx.status(titles_word(done), "");
        Ok(())
    }
}

impl RipVideo {
    /// One title, into a folder of its own for this attempt, then checked
    /// to be the whole title.
    #[allow(clippy::too_many_arguments)]
    async fn rip(&self, cx: &Cx, launcher: &Launcher, dev: &str, item: &Item, attempts: &mut std::collections::HashMap<String, u32>, at: usize, of: usize) -> Result<PathBuf> {
        let part = item.index.unwrap_or(0);
        let (id, minlength, length) = {
            let c = item.context();
            let p = Subject::Part(part);
            let num = |s, k| c.get(s, k).and_then(|v| v.parse::<u32>().ok());
            (num(p, facts::MAKEMKV_ID), num(Subject::Disc, facts::MINLENGTH).unwrap_or(60), num(p, facts::LENGTH).unwrap_or(0))
        };
        let Some(id) = id else { bail!("the scan didn't list this title") };
        let n = attempts.entry(item.id.clone()).or_default();
        *n += 1;
        let dir = cx.work()?.join("rip").join(format!("title-{part:02}-{n}"));
        cx.item(item, ItemState::Busy, "reading");
        cx.drive_says(StepState::Busy, "Reading", &format!("title {part}"));
        let file = with_key(cx, || {
            makemkv::rip(launcher, dev, id, minlength, &dir, cx.cancelled(), |f, doing| {
                cx.item(item, ItemState::Busy, format!("{} {}%", doing.to_lowercase(), (f * 100.0) as u32));
                cx.progress(Some((at as f32 + f) / of as f32));
            })
        })
        .await?;
        cx.item(item, ItemState::Busy, "checking");
        makemkv::check_whole(&file, length).await?;
        let size = std::fs::metadata(&file).map(|m| super::media::size(m.len())).unwrap_or_default();
        cx.item(item, ItemState::Ok, format!("✓ {}, {size}", hms(length)));
        Ok(file)
    }
}
