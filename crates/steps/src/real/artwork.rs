//! The Cover Art Archive beyond one front: the images a release has, a
//! bigger front from another edition of the same album, and *Artwork
//! files*, which saves every scan of the album beside its tracks.
//!
//! Every answer that can be kept (a release's list of images, an album's
//! editions, how big an edition's front is) is kept in the cache for a
//! week, so Cover art and Artwork files on the same disc ask once.
//! MusicBrainz is asked through its one-a-second throttle; the archive one
//! request at a time, through its own lane. See `docs/cover-art-editions.md`.

use super::{media, musicbrainz, place, Taken, There};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use drt_engine::{Cx, Group, Inlet, Item, ItemKind, ItemState, LaneKind, Outlet, Step, StepInfo, StepState, Subject};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SOURCE: &str = "Cover Art Archive";
const CAA: &str = "https://coverartarchive.org";
/// The folder inside the album's that the scans go in.
pub const FOLDER: &str = "Artwork";
/// How many of an album's releases are read, and how many editions' fronts
/// are measured at most: some albums have hundreds of releases.
const MAX_RELEASES: usize = 300;
const MAX_MEASURED: usize = 12;
/// What's read of a front to learn its size: its header is near the start.
const HEAD: usize = 256 * 1024;
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

/// The facts Cover art leaves on the disc when its front came from another
/// edition: that edition's id, and the words that tell it apart.
pub const COVER_RELEASE: &str = "cover_release";
pub const COVER_FROM: &str = "cover_from";

// ---- what the archive and MusicBrainz say -----------------------------------------------------

/// One image the archive holds for a release.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Image {
    #[serde(deserialize_with = "id_text")]
    pub id: String,
    #[serde(default)]
    pub types: Vec<String>,
    /// The release's own front: the one `…/front` gives.
    #[serde(default)]
    pub front: bool,
    /// Its address, as the archive gives it (sometimes plain HTTP).
    pub image: String,
}

/// The archive gives an image's id as a number, and once gave it as text.
fn id_text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        v => v.to_string(),
    })
}

impl Image {
    /// Its extension, from its address: `png`, `jpg`.
    pub fn ext(&self) -> String {
        let last = self.image.rsplit('/').next().unwrap_or("");
        match last.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()) {
            Some(e) if !e.is_empty() && e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()) => e,
            _ => "jpg".into(),
        }
    }

    /// The original, always over HTTPS: built from the release and the
    /// image's own id, never followed from the listing's address.
    pub fn url(&self, release: &str) -> String {
        format!("{CAA}/release/{release}/{}.{}", self.id, self.ext())
    }
}

#[derive(Deserialize)]
struct Listing {
    #[serde(default)]
    images: Vec<Image>,
}

/// The images in the archive's answer for a release.
pub fn parse_images(json: &[u8]) -> Result<Vec<Image>> {
    Ok(serde_json::from_slice::<Listing>(json)?.images)
}

/// One release of an album, as MusicBrainz lists them by release group.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Edition {
    pub id: String,
    pub year: String,
    pub country: String,
    pub label: String,
    pub catno: String,
    pub disambiguation: String,
    /// The archive has a front for it, and how many images in all.
    pub front: bool,
    pub count: u32,
}

#[derive(Deserialize)]
struct Browse {
    #[serde(default, rename = "release-count")]
    count: usize,
    #[serde(default)]
    releases: Vec<BrowseRelease>,
}

#[derive(Deserialize)]
struct BrowseRelease {
    id: String,
    date: Option<String>,
    country: Option<String>,
    disambiguation: Option<String>,
    #[serde(default, rename = "label-info")]
    label_info: Vec<LabelInfo>,
    #[serde(rename = "cover-art-archive")]
    caa: Option<CaaFlags>,
}

#[derive(Deserialize)]
struct LabelInfo {
    #[serde(rename = "catalog-number")]
    catalog_number: Option<String>,
    label: Option<Label>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
struct CaaFlags {
    #[serde(default)]
    front: bool,
    #[serde(default)]
    darkened: bool,
    #[serde(default)]
    count: u32,
}

/// One page of MusicBrainz's releases in a release group: the editions,
/// and how many there are in all.
pub fn parse_editions(json: &[u8]) -> Result<(Vec<Edition>, usize)> {
    let b: Browse = serde_json::from_slice(json)?;
    let editions = b
        .releases
        .into_iter()
        .map(|r| {
            let caa = r.caa.unwrap_or(CaaFlags { front: false, darkened: false, count: 0 });
            Edition {
                id: r.id,
                year: r.date.as_deref().and_then(|d| d.get(..4)).unwrap_or("").to_owned(),
                country: r.country.unwrap_or_default(),
                label: r.label_info.iter().find_map(|l| l.label.as_ref().map(|x| x.name.clone())).unwrap_or_default(),
                catno: r.label_info.iter().find_map(|l| l.catalog_number.clone()).unwrap_or_default(),
                disambiguation: r.disambiguation.unwrap_or_default(),
                // A darkened release's images can't be fetched.
                front: caa.front && !caa.darkened,
                count: if caa.darkened { 0 } else { caa.count },
            }
        })
        .collect();
    Ok((editions, b.count))
}

// ---- asking -------------------------------------------------------------------------------------

/// Where the answers come from: the archive and MusicBrainz, or, in tests,
/// answers recorded from them.
#[async_trait]
pub trait Archive: Send + Sync {
    /// The images the archive holds for a release; none when it has none.
    async fn images(&self, release: &str) -> Result<Vec<Image>>;
    /// Every release in a release group (up to [`MAX_RELEASES`]).
    async fn editions(&self, group: &str) -> Result<Vec<Edition>>;
    /// The size of a release's original front, from its first bytes.
    async fn front_size(&self, release: &str) -> Result<Option<(u32, u32)>>;
}

/// The real archive, through the cache, one request at a time in the
/// archive's lane.
pub struct Online {
    cx: Option<Cx>,
    cache: Option<PathBuf>,
}

impl Online {
    pub fn new(cx: &Cx) -> Online {
        Online { cx: Some(cx.clone()), cache: drt_engine::cache_dir().map(|d| d.join("cache").join("cover-art")) }
    }

    /// Room in the archive's lane, when there's a run to have one.
    async fn lane(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        match &self.cx {
            Some(cx) => Some(cx.lane(LaneKind::Network(SOURCE.into())).await),
            None => None,
        }
    }
}

fn stale(path: &Path) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_none_or(|age| age > MAX_AGE)
}

/// What `name` in the cache holds while it's fresh, otherwise what `fetch`
/// gives (kept for next time). A failed fetch falls back to a stale copy.
async fn cached<T, F>(cache: Option<&Path>, name: &str, fetch: F) -> Result<T>
where
    T: Serialize + DeserializeOwned,
    F: std::future::Future<Output = Result<T>>,
{
    let path = cache.map(|c| c.join(name));
    let kept = || path.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<T>(&b).ok());
    if path.as_deref().is_some_and(|p| !stale(p)) {
        if let Some(v) = kept() {
            return Ok(v);
        }
    }
    match fetch.await {
        Ok(v) => {
            if let (Some(p), Ok(json)) = (&path, serde_json::to_vec(&v)) {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = place::write(p, json);
            }
            Ok(v)
        }
        Err(e) => kept().ok_or(e),
    }
}

/// Only what's safe in a file name: MusicBrainz ids are hex and dashes.
fn key(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

fn client(secs: u64) -> Result<reqwest::Client> {
    super::net::client(Duration::from_secs(secs))
}

#[async_trait]
impl Archive for Online {
    async fn images(&self, release: &str) -> Result<Vec<Image>> {
        cached(self.cache.as_deref(), &format!("release-{}.json", key(release)), async {
            let _net = self.lane().await;
            let r = client(30)?.get(format!("{CAA}/release/{release}")).send().await?;
            match r.status().as_u16() {
                // No images: an answer, not a failure.
                404 => Ok(vec![]),
                200 => parse_images(&super::net::body(r, super::net::JSON_MAX).await?),
                s => Err(anyhow!("Cover Art Archive said {s}")),
            }
        })
        .await
    }

    async fn editions(&self, group: &str) -> Result<Vec<Edition>> {
        cached(self.cache.as_deref(), &format!("group-{}.json", key(group)), async {
            let mut all = vec![];
            loop {
                let url = format!("{}/release?release-group={group}&inc=labels&fmt=json&limit=100&offset={}", musicbrainz::API, all.len());
                let (page, total) = parse_editions(&musicbrainz::fetch_bytes(&url).await?)?;
                let got = page.len();
                all.extend(page);
                if got == 0 || all.len() >= total || all.len() >= MAX_RELEASES {
                    return Ok(all);
                }
            }
        })
        .await
    }

    async fn front_size(&self, release: &str) -> Result<Option<(u32, u32)>> {
        cached(self.cache.as_deref(), &format!("front-size-{}.json", key(release)), async {
            let _net = self.lane().await;
            let mut r = client(30)?.get(format!("{CAA}/release/{release}/front")).header(reqwest::header::RANGE, format!("bytes=0-{}", HEAD - 1)).send().await?;
            match r.status().as_u16() {
                404 => return Ok(None),
                200 | 206 => {}
                s => return Err(anyhow!("Cover Art Archive said {s}")),
            }
            // A server that ignores the range sends the lot: stop at the header.
            let mut head = Vec::new();
            while let Some(chunk) = r.chunk().await.map_err(super::net::quiet)? {
                head.extend_from_slice(&chunk);
                if head.len() >= HEAD {
                    break;
                }
            }
            Ok(media::picture_size_of(&head))
        })
        .await
    }
}

// ---- a bigger front ---------------------------------------------------------------------------------

/// Another edition's front, bigger than the chosen release's own.
#[derive(Clone, Debug, PartialEq)]
pub struct Elsewhere {
    pub edition: Edition,
    pub size: (u32, u32),
    /// What tells it apart from the chosen release: *Polydor, Europe 1995*.
    pub words: String,
    /// How many editions' fronts were measured to find it.
    pub measured: usize,
}

impl Elsewhere {
    /// Its original's longer side.
    pub fn px(&self) -> u32 {
        self.size.0.max(self.size.1)
    }
}

/// The other editions' fronts, measured, best first: the longest side,
/// then more scans, then the lower id. Also the chosen release's own entry
/// in the group, to tell the others apart from.
async fn measure_others(a: &dyn Archive, release: &str, group: &str) -> Result<(Option<Edition>, Vec<(Edition, (u32, u32))>)> {
    let editions = a.editions(group).await?;
    let chosen = editions.iter().find(|e| e.id == release).cloned();
    let mut others: Vec<&Edition> = editions.iter().filter(|e| e.id != release && e.front).collect();
    others.sort_by(|x, y| y.count.cmp(&x.count).then_with(|| x.id.cmp(&y.id)));
    others.truncate(MAX_MEASURED);
    let mut measured = vec![];
    for e in others {
        // One that can't be measured is passed over, not the end of it.
        let Ok(Some(size)) = a.front_size(&e.id).await else { continue };
        measured.push((e.clone(), size));
    }
    let longer = |s: (u32, u32)| s.0.max(s.1);
    measured.sort_by(|(x, xs), (y, ys)| (longer(*ys), y.count, std::cmp::Reverse(&y.id)).cmp(&(longer(*xs), x.count, std::cmp::Reverse(&x.id))));
    Ok((chosen, measured))
}

/// The largest front among the other releases in `group`, if its longer
/// side is over `longer_than` (the chosen release's own, or 0 when it has
/// none). Ties go to the edition with more scans, then the lower id.
pub async fn bigger_front(a: &dyn Archive, release: &str, group: &str, longer_than: u32) -> Result<Option<Elsewhere>> {
    let (chosen, measured) = measure_others(a, release, group).await?;
    let n = measured.len();
    Ok(measured.into_iter().next().filter(|(_, s)| s.0.max(s.1) > longer_than).map(|(e, size)| Elsewhere { words: edition_words(chosen.as_ref(), &e), edition: e, size, measured: n }))
}

/// A front to pick from: the chosen release's own or another edition's.
#[derive(Clone, Debug, PartialEq)]
pub struct Front {
    pub edition: Edition,
    pub size: (u32, u32),
    /// *This release*, or what tells the edition apart.
    pub words: String,
    pub own: bool,
}

impl Front {
    pub fn px(&self) -> u32 {
        self.size.0.max(self.size.1)
    }

    /// As Cover art says a front from elsewhere, when it is.
    pub fn elsewhere(&self, measured: usize) -> Option<Elsewhere> {
        (!self.own).then(|| Elsewhere { edition: self.edition.clone(), size: self.size, words: self.words.clone(), measured })
    }
}

/// Every front there is to pick from: the chosen release's own first, then
/// the other editions', biggest first.
pub async fn fronts(a: &dyn Archive, release: &str, group: Option<&str>) -> Result<Vec<Front>> {
    let (chosen, others) = match group {
        Some(g) => measure_others(a, release, g).await?,
        None => (None, vec![]),
    };
    let mut out = vec![];
    if let Ok(Some(size)) = a.front_size(release).await {
        let edition = chosen.clone().unwrap_or_else(|| Edition { id: release.to_owned(), ..Default::default() });
        out.push(Front { edition, size, words: "This release".into(), own: true });
    }
    out.extend(others.into_iter().map(|(e, size)| Front { words: edition_words(chosen.as_ref(), &e), edition: e, size, own: false }));
    Ok(out)
}

/// The front Cover art would take by itself: the release's own unless it
/// falls short of `want` and `bigger` lets a bigger one stand in; or, with
/// `biggest`, the biggest of all, the release's own on a tie.
pub fn best_front(fronts: &[Front], want: Option<u32>, bigger: bool, biggest: bool) -> usize {
    if biggest {
        return fronts.iter().enumerate().max_by_key(|(k, f)| (f.px(), f.own, std::cmp::Reverse(*k))).map_or(0, |(k, _)| k);
    }
    let own = fronts.iter().position(|f| f.own);
    let biggest = fronts.iter().enumerate().filter(|(_, f)| !f.own).max_by_key(|(k, f)| (f.px(), std::cmp::Reverse(*k))).map(|(k, _)| k);
    match (own, biggest) {
        (Some(o), Some(b)) if bigger && super::falls_short(Some(fronts[o].size), want) && fronts[b].px() > fronts[o].px() => b,
        (Some(o), _) => o,
        (None, Some(b)) => b,
        (None, None) => 0,
    }
}

/// The question Cover art asks when you pick the cover: each front as a
/// picture (`thumbs`, beside them), with where it's from and its size.
pub fn cover_question(fronts: &[Front], thumbs: &[Option<PathBuf>], preselected: usize) -> drt_engine::Question {
    let options = fronts
        .iter()
        .zip(thumbs)
        .map(|(f, t)| drt_engine::Choice { label: f.words.clone(), detail: format!("{} × {}", f.size.0, f.size.1), destructive: false, picture: t.as_ref().map(|p| p.display().to_string()) })
        .collect();
    drt_engine::Question { prompt: "Which cover?".into(), shape: drt_engine::QuestionShape::PickOne { options, preselected, details: None, skip: false } }
}

/// A country as a person would say it, from MusicBrainz's code.
pub fn country(code: &str) -> String {
    let name = match code {
        "XE" => "Europe",
        "XW" => "Worldwide",
        "GB" => "UK",
        "US" => "US",
        "DE" => "Germany",
        "FR" => "France",
        "NL" => "Netherlands",
        "IT" => "Italy",
        "ES" => "Spain",
        "SE" => "Sweden",
        "JP" => "Japan",
        "KR" => "South Korea",
        "CA" => "Canada",
        "AU" => "Australia",
        "NZ" => "New Zealand",
        "BR" => "Brazil",
        "MX" => "Mexico",
        "AR" => "Argentina",
        "RU" => "Russia",
        "PL" => "Poland",
        "TW" => "Taiwan",
        "HK" => "Hong Kong",
        "XU" | "" => "",
        other => other,
    };
    name.to_owned()
}

/// Another edition, in the words that tell it from the chosen one: its
/// label, its catalogue number when that differs, where and when.
/// *Polydor, Europe 1995*; *A&M Records 31452 3787 2, US 1995*. When even
/// those are the same, its MusicBrainz note is added.
pub fn edition_words(chosen: Option<&Edition>, other: &Edition) -> String {
    let differs = |f: fn(&Edition) -> &str| chosen.is_none_or(|c| f(c) != f(other));
    let mut who = other.label.clone();
    if !other.catno.is_empty() && differs(|e| &e.catno) {
        who = format!("{who} {}", other.catno).trim().to_owned();
    }
    let when = [country(&other.country), other.year.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    let mut parts: Vec<String> = [who, when].into_iter().filter(|s| !s.is_empty()).collect();
    let same = chosen.is_some_and(|c| c.label == other.label && c.catno == other.catno && c.country == other.country && c.year == other.year);
    if (same || parts.is_empty()) && !other.disambiguation.is_empty() {
        parts.push(other.disambiguation.clone());
    }
    if parts.is_empty() {
        "another release of this album".into()
    } else {
        parts.join(", ")
    }
}

/// Where a front from another edition came from, in Cover art's record:
/// *From another edition: Polydor, Europe 1995, 2813 px*.
pub fn from_another(words: &str, px: u32) -> String {
    format!("From another edition: {words}, {px} px")
}

// ---- names ----------------------------------------------------------------------------------------

/// What an image shows, as a file name: the archive's type, in the words
/// a CD's parts go by.
fn base(kind: &str) -> String {
    match kind {
        "Medium" => "Disc".into(),
        "Matrix/Runout" => "Matrix".into(),
        "Raw/Unedited" => "Raw".into(),
        k => super::safe(k),
    }
}

/// Each image's file name, with its own extension: the release's front is
/// `Front`; a name shared by several is numbered in the archive's order
/// (`Booklet 01`, `Booklet 02`), other fronts after it from `Front 02`.
pub fn names(images: &[Image]) -> Vec<String> {
    let bases: Vec<String> = images.iter().map(|i| if i.front { "Front".to_owned() } else { i.types.first().map(|t| base(t)).filter(|b| !b.is_empty()).unwrap_or_else(|| "Other".into()) }).collect();
    let has_front = images.iter().any(|i| i.front);
    let mut seen: std::collections::HashMap<&str, usize> = Default::default();
    images
        .iter()
        .zip(&bases)
        .map(|(img, b)| {
            let total = bases.iter().filter(|x| *x == b).count();
            let n = seen.entry(b).or_default();
            *n += 1;
            let stem = if b == "Front" && has_front {
                if img.front {
                    b.clone()
                } else {
                    // The release's own front is the first; the rest follow it.
                    let k = images.iter().zip(&bases).filter(|(i, x)| *x == b && !i.front).position(|(i, _)| std::ptr::eq(i, img)).unwrap_or(0) + 2;
                    format!("{b} {k:02}")
                }
            } else if total > 1 {
                format!("{b} {:02}", *n)
            } else {
                b.clone()
            };
            format!("{stem}.{}", img.ext())
        })
        .collect()
}

// ---- downloading ----------------------------------------------------------------------------------

/// Downloads `url` to `dest` through a temporary file, saying how far it
/// has got (bytes so far, and the whole if known). Gives its size.
pub async fn download(url: &str, dest: &Path, mut said: impl FnMut(u64, Option<u64>)) -> Result<u64> {
    use tokio::io::AsyncWriteExt;
    let mut r = client(600)?.get(url).send().await?;
    if !r.status().is_success() {
        return Err(anyhow!("Cover Art Archive said {}", r.status().as_u16()));
    }
    let total = r.content_length();
    let max = super::net::FILE_MAX as u64;
    if total.is_some_and(|n| n > max) {
        return Err(anyhow!("it's over {} MB", super::net::FILE_MAX / super::net::MB));
    }
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let tmp = place::part(dest);
    let _ = tokio::fs::remove_file(&tmp).await;
    let mut f = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&tmp).await?;
    let mut got = 0u64;
    while let Some(chunk) = r.chunk().await.map_err(super::net::quiet)? {
        got += chunk.len() as u64;
        if got > max {
            drop(f);
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(anyhow!("it's over {} MB", super::net::FILE_MAX / super::net::MB));
        }
        f.write_all(&chunk).await?;
        said(got, total);
    }
    f.flush().await?;
    drop(f);
    tokio::fs::rename(&tmp, dest).await?;
    Ok(got)
}

// ---- the step -------------------------------------------------------------------------------------

/// Artwork files: every image of the chosen release, at original size, in
/// an `Artwork` folder beside the filed tracks. Needs the files filed and
/// an album chosen, and has no settings.
pub fn files_info() -> StepInfo {
    StepInfo::new("artwork-files", "Artwork files", Group::Make, ItemKind::AudioFile, ItemKind::PictureFile).needs(&["filed"]).needs(&["release"]).from("every scan of the album, beside the tracks")
}

/// Its outcome: *29 pictures*, and where they came from.
pub fn files_say(cx: &Cx, saved: usize, bytes: u64, from: Option<&str>) {
    let what = if saved == 1 { "1 picture".to_owned() } else { format!("{saved} pictures") };
    let size = if bytes > 0 { media::size(bytes) } else { String::new() };
    cx.outcome(what, size, from_words(from));
}

/// Where the pictures came from, for its card.
pub fn from_words(from: Option<&str>) -> String {
    match from {
        Some(w) => format!("From another edition: {w}"),
        None => "From this release".into(),
    }
}

pub struct ArtworkFiles;

#[async_trait]
impl Step for ArtworkFiles {
    fn info(&self) -> StepInfo {
        files_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let Some(first) = inlet.recv().await else { return Ok(()) };
        // The rest of the tracks are taken in as they come, so the Folder
        // before is never held up, while the first says where the album is.
        let rest = async { while inlet.recv().await.is_some() {} };
        let (saved, ()) = tokio::join!(save_all(&first, &outlet, &cx), rest);
        saved
    }
}

async fn save_all(first: &Item, outlet: &Outlet, cx: &Cx) -> Result<()> {
    let c = first.context();
    let Some(release) = c.get(Subject::Disc, "musicbrainz_albumid").map(str::to_owned) else {
        cx.status("Nothing to save", "no album chosen");
        return Ok(());
    };
    let Some(album) = first.path.as_deref().and_then(Path::parent).map(Path::to_path_buf) else {
        cx.status("Nothing to save", "the tracks weren't saved in a folder");
        return Ok(());
    };
    let archive = Online::new(cx);
    cx.status("", "Asking the Cover Art Archive");
    let found = tokio::select! {
        f = find_images(&archive, c, &release) => f,
        _ = cx.cancelled() => return Ok(()),
    };
    let (images, from_release, from) = match found {
        Ok(f) => f,
        Err(e) => {
            cx.item(first, ItemState::Skipped, "no answer");
            cx.status("No answer", e.to_string());
            return Ok(());
        }
    };
    if images.is_empty() {
        cx.status("None found", "the archive has no pictures of this album");
        return Ok(());
    }
    if let Some(w) = &from {
        cx.log(format!("this release has no pictures; saving another edition's ({w}, {from_release})"));
    }
    let dir = album.join(FOLDER);
    let work = cx.work()?.join("artwork").join(key(&from_release));
    let names = names(&images);
    let n = images.len();
    cx.expect(n);
    let items: Vec<Item> = names.iter().enumerate().map(|(k, name)| Item::new(format!("picture-{}", k + 1), ItemKind::PictureFile, name.clone(), None, None).knowing(first)).collect();
    for it in &items {
        cx.item(it, ItemState::Waiting, "");
    }
    let (mut saved, mut bytes) = (0usize, 0u64);
    let mut taken: Vec<(Item, PathBuf, PathBuf)> = vec![];
    let lane = LaneKind::File(album.display().to_string());
    for (k, ((img, name), it)) in images.iter().zip(&names).zip(&items).enumerate() {
        let got = work.join(format!("{}.{}", key(&img.id), img.ext()));
        cx.status(format!("{k} of {n}"), name.clone());
        cx.item(it, ItemState::Busy, "downloading");
        let fetched = if got.is_file() {
            Ok(std::fs::metadata(&got).map(|m| m.len()).unwrap_or(0))
        } else {
            let _net = cx.lane(LaneKind::Network(SOURCE.into())).await;
            let said = |sofar: u64, whole: Option<u64>| {
                let part = whole.filter(|w| *w > 0).map(|w| sofar as f32 / w as f32);
                cx.progress(Some((k as f32 + part.unwrap_or(0.0)) / n as f32));
                let note = match part {
                    Some(p) => format!("downloading {}%, {}", (p * 100.0).round() as u32, mb(whole.unwrap_or(0))),
                    None => format!("downloading, {} so far", mb(sofar)),
                };
                cx.item(it, ItemState::Busy, note);
            };
            let url = img.url(&from_release);
            tokio::select! {
                r = download(&url, &got, said) => r,
                _ = cx.cancelled() => return Ok(()),
            }
        };
        let size = match fetched {
            Ok(s) => s,
            Err(e) => {
                cx.item(it, ItemState::Bad, e.to_string());
                cx.log(format!("couldn't download {name}: {e}"));
                continue;
            }
        };
        let dest = dir.join(name);
        let _f = cx.lane(lane.clone()).await;
        // Never over a different file without asking.
        match super::already_there(&got, &dest).await {
            Ok(There::Nothing) => match super::copy_in(&got, &dest).await {
                Ok(()) => {
                    saved += 1;
                    bytes += size;
                    cx.item(it, ItemState::Ok, mb(size));
                    outlet.send(it.becomes(ItemKind::PictureFile, Some(dest))).await;
                }
                Err(e) => {
                    cx.item(it, ItemState::Bad, e.to_string());
                    cx.log(format!("couldn't save {}: {e}", dest.display()));
                }
            },
            Ok(There::Same) => {
                saved += 1;
                bytes += size;
                cx.item(it, ItemState::Ok, "already there, the same");
                outlet.send(it.becomes(ItemKind::PictureFile, Some(dest))).await;
            }
            Ok(There::Different) => {
                cx.item(it, ItemState::Waiting, "a different file has this name");
                taken.push((it.clone(), got, dest));
            }
            Err(e) => {
                cx.item(it, ItemState::Bad, e.to_string());
                cx.log(format!("couldn't save {}: {e}", dest.display()));
            }
        }
    }
    cx.progress(None);
    if !taken.is_empty() && !cx.stopped() {
        let place = format!("{} / {FOLDER}", album.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let Ok(answer) = cx.ask(super::name_taken(taken.len(), &place)).await else { return Ok(()) };
        let choice = super::taken_choice(answer);
        let _f = cx.lane(lane.clone()).await;
        for (it, got, dest) in taken {
            if choice == Taken::Skip {
                cx.item(&it, ItemState::Skipped, "not saved: a different file has this name");
                continue;
            }
            let put = match choice {
                Taken::Replace => super::copy_in(&got, &dest).await.map(|()| dest.clone()),
                _ => super::keep_both(&got, &dest).await,
            };
            match put {
                Ok(dest) => {
                    saved += 1;
                    let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
                    bytes += size;
                    cx.item(&it, ItemState::Ok, super::file_name(&dest));
                    outlet.send(it.becomes(ItemKind::PictureFile, Some(dest))).await;
                }
                Err(e) => {
                    cx.item(&it, ItemState::Bad, e.to_string());
                    cx.log(format!("couldn't save {}: {e}", dest.display()));
                }
            }
        }
        cx.state(StepState::Busy);
    }
    cx.log(format!("saved {saved} of {n} pictures in {}", dir.display()));
    files_say(cx, saved, bytes, from.as_deref());
    Ok(())
}

/// A size in megabytes, to one place under ten: *2.4 MB*, *14 MB*.
fn mb(bytes: u64) -> String {
    let m = bytes as f64 / 1e6;
    if m < 10.0 {
        format!("{m:.1} MB")
    } else {
        format!("{} MB", m.round() as u64)
    }
}

/// The images to save: the chosen release's, or when it has none, those of
/// the edition Cover art took its front from (or the same search, without
/// it). Gives them, the release they're of, and, for another edition, the
/// words that tell it apart.
pub async fn find_images(a: &dyn Archive, c: &drt_engine::Context, release: &str) -> Result<(Vec<Image>, String, Option<String>)> {
    let own = a.images(release).await?;
    if !own.is_empty() {
        return Ok((own, release.to_owned(), None));
    }
    let said = c.get(Subject::Disc, COVER_RELEASE).filter(|r| *r != release).map(|r| (r.to_owned(), c.get(Subject::Disc, COVER_FROM).unwrap_or("").to_owned()));
    let other = match said {
        Some(o) => Some(o),
        None => match c.get(Subject::Disc, "musicbrainz_releasegroupid") {
            Some(g) => bigger_front(a, release, g, 0).await?.map(|e| (e.edition.id, e.words)),
            None => None,
        },
    };
    match other {
        Some((id, words)) => Ok((a.images(&id).await?, id, Some(words))),
        None => Ok((vec![], release.to_owned(), None)),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;

    pub const CHOSEN: &str = "63ace95a-d699-46cb-9460-3672e434e552";
    pub const GROUP: &str = "2d20a8c4-83f8-35de-9adc-c4c46e8b862e";
    pub const BIGGEST: &str = "887fdba3-6599-4c00-80a5-ee30bcdde469";

    /// The archive and MusicBrainz for *Ocean Drive* (Lighthouse Family,
    /// 1995), recorded 2026-11-20: the browse of its release group, each
    /// release's list of images, and each front's size as measured then.
    pub struct Recorded {
        pub asked: std::sync::Mutex<Vec<String>>,
    }

    pub fn recorded() -> Recorded {
        Recorded { asked: Default::default() }
    }

    fn listing(release: &str) -> Option<&'static str> {
        Some(match release.get(..8)? {
            "63ace95a" => include_str!("fixtures/cover-art/release-63ace95a.json"),
            "887fdba3" => include_str!("fixtures/cover-art/release-887fdba3.json"),
            "118878e6" => include_str!("fixtures/cover-art/release-118878e6.json"),
            "af4c39d7" => include_str!("fixtures/cover-art/release-af4c39d7.json"),
            "e51f6e37" => include_str!("fixtures/cover-art/release-e51f6e37.json"),
            _ => return None,
        })
    }

    fn sizes() -> HashMap<&'static str, (u32, u32)> {
        HashMap::from([("63ace95a", (491, 500)), ("887fdba3", (2813, 2805)), ("118878e6", (599, 594)), ("af4c39d7", (599, 601)), ("e51f6e37", (1902, 1858))])
    }

    #[async_trait]
    impl Archive for Recorded {
        async fn images(&self, release: &str) -> Result<Vec<Image>> {
            self.asked.lock().unwrap().push(format!("images {release}"));
            listing(release).map_or(Ok(vec![]), |j| parse_images(j.as_bytes()))
        }
        async fn editions(&self, group: &str) -> Result<Vec<Edition>> {
            self.asked.lock().unwrap().push(format!("editions {group}"));
            Ok(parse_editions(include_bytes!("fixtures/cover-art/group-2d20a8c4.json"))?.0)
        }
        async fn front_size(&self, release: &str) -> Result<Option<(u32, u32)>> {
            self.asked.lock().unwrap().push(format!("size {release}"));
            Ok(sizes().get(release.get(..8).unwrap_or("")).copied())
        }
    }

    #[tokio::test]
    async fn the_biggest_front_among_the_other_editions() {
        let a = recorded();
        let e = bigger_front(&a, CHOSEN, GROUP, 500).await.unwrap().unwrap();
        assert_eq!(e.edition.id, BIGGEST);
        assert_eq!((e.size, e.px()), ((2813, 2805), 2813));
        assert_eq!(e.measured, 4, "every other edition with a front, not the chosen one nor the one with none");
        let asked = a.asked.lock().unwrap().clone();
        assert!(!asked.iter().any(|q| q.contains(CHOSEN)), "{asked:?}");
        assert!(!asked.iter().any(|q| q.contains("8d6d22fc")), "no front, not measured: {asked:?}");
        // The most scans are measured first.
        assert_eq!(asked[1], format!("size {BIGGEST}"));
    }

    #[tokio::test]
    async fn no_other_edition_bigger_than_the_own_is_none() {
        assert!(bigger_front(&recorded(), CHOSEN, GROUP, 2813).await.unwrap().is_none());
        // With none of its own, any front will do.
        assert_eq!(bigger_front(&recorded(), CHOSEN, GROUP, 0).await.unwrap().unwrap().edition.id, BIGGEST);
    }

    #[test]
    fn the_editions_from_the_browse() {
        let (eds, total) = parse_editions(include_bytes!("fixtures/cover-art/group-2d20a8c4.json")).unwrap();
        assert_eq!((eds.len(), total), (6, 6));
        let big = eds.iter().find(|e| e.id == BIGGEST).unwrap();
        assert_eq!((big.label.as_str(), big.catno.as_str(), big.country.as_str(), big.year.as_str(), big.front, big.count), ("Polydor", "523787-2", "XE", "1995", true, 28));
        assert!(!eds.iter().find(|e| e.id.starts_with("8d6d22fc")).unwrap().front);
    }

    #[test]
    fn an_edition_is_told_apart_by_what_differs() {
        let (eds, _) = parse_editions(include_bytes!("fixtures/cover-art/group-2d20a8c4.json")).unwrap();
        let get = |p: &str| eds.iter().find(|e| e.id.starts_with(p)).unwrap();
        let chosen = get("63ace95a");
        // The same label and catalogue number: where it's from is the difference.
        assert_eq!(edition_words(Some(chosen), get("887fdba3")), "Polydor, Europe 1995");
        assert_eq!(from_another("Polydor, Europe 1995", 2813), "From another edition: Polydor, Europe 1995, 2813 px");
        // Another catalogue number is named.
        assert_eq!(edition_words(Some(chosen), get("e51f6e37")), "A&M Records 31452 3787 2, US 1995");
        // No year: just the rest.
        assert_eq!(edition_words(Some(chosen), get("af4c39d7")), "Polydor 537 882-2, Brazil");
        // The same everything: the note MusicBrainz keeps for it.
        assert_eq!(edition_words(Some(chosen), get("118878e6")), "Polydor, UK 1995, Matrix: “IFPI L002”");
    }

    #[test]
    fn scans_are_named_for_what_they_show() {
        let images = parse_images(include_bytes!("fixtures/cover-art/release-887fdba3.json")).unwrap();
        let names = names(&images);
        assert_eq!(names.len(), 28);
        assert_eq!(names[0], "Front.png");
        assert_eq!(&names[1..3], ["Booklet 01.png", "Booklet 02.png"]);
        assert!(names.contains(&"Disc 01.png".to_owned()) && names.contains(&"Disc 03.jpg".to_owned()), "{names:?}");
        assert!(names.contains(&"Tray 01.png".to_owned()) && names.contains(&"Tray 02.jpg".to_owned()));
        assert!(names.contains(&"Back 01.png".to_owned()) && names.contains(&"Back 02.jpg".to_owned()));
        assert!(names.contains(&"Matrix.jpg".to_owned()), "no slash: {names:?}");
        assert!(names.contains(&"Booklet 16.jpg".to_owned()));
        assert!(names.contains(&"Sticker 01.jpg".to_owned()) && names.contains(&"Sticker 02.jpg".to_owned()));
        // A second front with a sticker on it doesn't take the front's name.
        assert!(names.contains(&"Front 02.jpg".to_owned()), "{names:?}");
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "every name once: {names:?}");
        // One of its own: no number.
        let one = parse_images(include_bytes!("fixtures/cover-art/release-63ace95a.json")).unwrap();
        assert_eq!(super::names(&one), ["Front.jpg"]);
        // No type at all.
        let odd = vec![Image { id: "1".into(), types: vec![], front: false, image: "x/1.gif".into() }];
        assert_eq!(super::names(&odd), ["Other.gif"]);
    }

    #[test]
    fn an_image_is_always_fetched_over_https() {
        let images = parse_images(include_bytes!("fixtures/cover-art/release-af4c39d7.json")).unwrap();
        assert!(images[0].image.starts_with("http://"), "the archive's own address is plain");
        assert_eq!(images[0].url("af4c39d7-2682-4190-ad19-0e53a2cbcde1"), "https://coverartarchive.org/release/af4c39d7-2682-4190-ad19-0e53a2cbcde1/20982255789.jpg");
    }

    #[tokio::test]
    async fn the_scans_of_the_release_or_failing_that_the_edition_cover_art_used() {
        let mut c = drt_engine::Context::default();
        c.add(Subject::Disc, "musicbrainz_releasegroupid", GROUP, "MusicBrainz");
        // Its own, even just a front.
        let (imgs, of, from) = find_images(&recorded(), &c, CHOSEN).await.unwrap();
        assert_eq!((imgs.len(), of.as_str(), from), (1, CHOSEN, None));
        // None of its own: the edition Cover art said.
        let none = "00000000-0000-0000-0000-000000000000";
        c.add(Subject::Disc, COVER_RELEASE, "e51f6e37-2876-43a1-90b4-784e9dbd1d55", SOURCE);
        c.add(Subject::Disc, COVER_FROM, "A&M Records 31452 3787 2, US 1995", SOURCE);
        let (imgs, of, from) = find_images(&recorded(), &c, none).await.unwrap();
        assert_eq!((imgs.len(), of.as_str(), from.as_deref()), (2, "e51f6e37-2876-43a1-90b4-784e9dbd1d55", Some("A&M Records 31452 3787 2, US 1995")));
        // Without Cover art: the same search it would make.
        let mut bare = drt_engine::Context::default();
        bare.add(Subject::Disc, "musicbrainz_releasegroupid", GROUP, "MusicBrainz");
        let (imgs, of, from) = find_images(&recorded(), &bare, none).await.unwrap();
        assert_eq!((imgs.len(), of.as_str()), (28, BIGGEST));
        // Not one of the group's own releases: nothing to compare with, so its catalogue number too.
        assert_eq!(from.as_deref(), Some("Polydor 523787-2, Europe 1995"));
    }

    #[tokio::test]
    async fn a_kept_answer_is_used_and_a_failed_ask_falls_back_to_it() {
        let dir = std::env::temp_dir().join(format!("drt-caa-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let v: Vec<u32> = cached(Some(&dir), "x.json", async { Ok(vec![1, 2]) }).await.unwrap();
        assert_eq!(v, [1, 2]);
        let again: Vec<u32> = cached(Some(&dir), "x.json", async { Err(anyhow!("not asked")) }).await.unwrap();
        assert_eq!(again, [1, 2], "fresh: not asked again");
        // Out of date: asked, and when that fails, the old answer.
        let f = std::fs::File::options().write(true).open(dir.join("x.json")).unwrap();
        f.set_modified(std::time::UNIX_EPOCH).unwrap();
        let old: Vec<u32> = cached(Some(&dir), "x.json", async { Err(anyhow!("offline")) }).await.unwrap();
        assert_eq!(old, [1, 2]);
        assert!(cached::<Vec<u32>, _>(Some(&dir), "y.json", async { Err(anyhow!("offline")) }).await.is_err());
    }

    #[test]
    fn a_small_front_falls_short_of_the_size_asked_for() {
        use super::super::{cover_says, falls_short, wanted_px};
        assert!(falls_short(Some((491, 500)), wanted_px("1200 px")));
        assert!(!falls_short(Some((491, 500)), wanted_px("500 px")), "500 px asked, 500 there");
        assert!(!falls_short(Some((1200, 1197)), wanted_px("1200 px")), "big enough: the release's own is kept");
        // The original asks for no size: only none at all falls short.
        assert!(!falls_short(Some((491, 500)), wanted_px("Original")));
        assert!(falls_short(None, wanted_px("Original")));
        let e = Elsewhere { edition: Edition { id: BIGGEST.into(), ..Default::default() }, size: (2813, 2805), words: "Polydor, Europe 1995".into(), measured: 4 };
        assert_eq!(cover_says(Some((1200, 1197)), Some(&e)), ("1200 × 1197".to_owned(), "From another edition".to_owned()));
        assert_eq!(cover_says(Some((491, 500)), None), ("491 × 500".to_owned(), String::new()));
    }

    #[tokio::test]
    async fn every_front_to_pick_from_and_the_one_it_would_take() {
        let f = fronts(&recorded(), CHOSEN, Some(GROUP)).await.unwrap();
        let shown: Vec<(&str, u32)> = f.iter().map(|f| (f.words.as_str(), f.px())).collect();
        assert_eq!(
            shown,
            [("This release", 500), ("Polydor, Europe 1995", 2813), ("A&M Records 31452 3787 2, US 1995", 1902), ("Polydor 537 882-2, Brazil", 601), ("Polydor, UK 1995, Matrix: “IFPI L002”", 599)]
        );
        // 1200 px asked: the biggest; 500 px: its own is enough; told not to look elsewhere: its own.
        assert_eq!(best_front(&f, Some(1200), true, false), 1);
        assert_eq!(best_front(&f, Some(500), true, false), 0);
        assert_eq!(best_front(&f, Some(1200), false, false), 0);
        assert_eq!(best_front(&f[1..], Some(1200), false, false), 0, "none of its own: the biggest there is");
        // The biggest of any edition, whatever size was asked for.
        assert_eq!(best_front(&f, Some(500), false, true), 1);
        assert_eq!(best_front(&f[..1], Some(500), false, true), 0, "only its own");
        let thumbs: Vec<Option<PathBuf>> = (0..f.len()).map(|k| Some(PathBuf::from(format!("/w/{k}.jpg")))).collect();
        let q = cover_question(&f, &thumbs, 1);
        assert_eq!(q.prompt, "Which cover?");
        let drt_engine::QuestionShape::PickOne { options, preselected, .. } = q.shape else { panic!() };
        assert_eq!(preselected, 1);
        assert_eq!((options[1].label.as_str(), options[1].detail.as_str(), options[1].picture.as_deref()), ("Polydor, Europe 1995", "2813 × 2805", Some("/w/1.jpg")));
        assert!(f[1].elsewhere(4).is_some() && f[0].elsewhere(4).is_none());
    }

    #[test]
    fn its_wiring() {
        let i = files_info();
        assert_eq!((i.kind, i.input, i.output, i.group), ("artwork-files", ItemKind::AudioFile, ItemKind::PictureFile, Group::Make));
        assert!(i.params.is_empty());
        assert!(i.needs.iter().any(|n| n.contains(&"filed")) && i.needs.iter().any(|n| n.contains(&"release")));
        assert_eq!(from_words(None), "From this release");
        assert_eq!(from_words(Some("Polydor, Europe 1995")), "From another edition: Polydor, Europe 1995");
    }

    /// The real archive and MusicBrainz for the same album. Needs the
    /// network: `cargo test -p drt-steps ocean_drive -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn ocean_drive_for_real() {
        let cache = std::env::temp_dir().join(format!("drt-caa-real-{}", std::process::id()));
        let a = Online { cx: None, cache: Some(cache) };
        let e = bigger_front(&a, CHOSEN, GROUP, 500).await.unwrap().unwrap();
        assert_eq!((e.edition.id.as_str(), e.size), (BIGGEST, (2813, 2805)));
        assert_eq!(e.words, "Polydor, Europe 1995");
        let own = a.images(CHOSEN).await.unwrap();
        assert_eq!(own.len(), 1);
        // Its original, downloaded as Artwork files would.
        let dest = std::env::temp_dir().join(format!("drt-caa-real-{}", std::process::id())).join("Front.jpg");
        let mut said = 0;
        let n = download(&own[0].url(CHOSEN), &dest, |_, _| said += 1).await.unwrap();
        assert!(n > 0 && said > 0);
        assert_eq!(media::picture_size(&dest), Some((491, 500)));
        // Asked again: from the cache.
        assert_eq!(bigger_front(&a, CHOSEN, GROUP, 500).await.unwrap().unwrap().edition.id, BIGGEST);
    }
}
