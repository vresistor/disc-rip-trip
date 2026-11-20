//! Simulated steps: the real steps' names, parameters, statuses and
//! outcomes, with a real disc's data (The White Stripes, 1999) and believable
//! timings, and no drive or network.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::{Candidate, Context, Cx, Fact, Group, Inlet, Item, ItemKind, ItemState, LaneKind, Outlet, Registry, Step, StepInfo, StepState, Subject};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const TRACKS: [(&str, u32); 17] = [
    ("Jimmy the Exploder", 150),
    ("Stop Breaking Down", 140),
    ("The Big Three Killed My Baby", 149),
    ("Suzy Lee", 201),
    ("Sugar Never Tasted So Good", 175),
    ("Wasting My Time", 133),
    ("Cannon", 151),
    ("Astro", 162),
    ("Broken Bricks", 111),
    ("When I Hear My Name", 114),
    ("Do", 185),
    ("Screwdriver", 194),
    ("One More Cup of Coffee", 194),
    ("Little People", 142),
    ("Slicker Drips", 91),
    ("St. James Infirmary Blues", 144),
    ("I Fought Piranhas", 189),
];

pub fn register(r: &mut Registry, bad_track: Option<u32>) {
    r.add(Drive);
    r.add(AudioCd);
    for l in [Lookup::MusicBrainz, Lookup::CdText, Lookup::Discogs, Lookup::GnuDb, Lookup::CoverArt, Lookup::Lyrics] {
        r.add(l);
    }
    r.add(crate::choose::Choose);
    r.add(Rip { bad_track });
    for c in crate::real::encode::ALL {
        r.add(Encode(c));
    }
    r.add(DataDisc);
    r.add(Image { bad: bad_track.is_some() });
    r.add(Check);
    r.add(Chd);
    r.add(Keep);
    r.add(Place { kind: "folder", name: "Folder" });
    // Library was a second name for the same step; recipes saved with it still load.
    r.alias("library", "folder");
    r.add(crate::hold::Hold);
    r.add(ReplayGain);
    r.add(CueSheet);
    r.add(LyricsFile);
    r.add(Tell);
    r.add(crate::real::NextDisc { real: false });
    crate::game_sim::register(r);
    crate::real::video::sim::register(r, crate::real::video::sim::SimDisc::Season);
}

fn parts(c: &Context) -> u32 {
    c.parts().max(1)
}

fn title(c: &Context, n: u32) -> String {
    c.get(Subject::Part(n), "title").map(str::to_owned).unwrap_or_else(|| format!("Track {n}"))
}

// ---- the drive ----------------------------------------------------------------------

struct Drive;

#[async_trait]
impl Step for Drive {
    fn info(&self) -> StepInfo {
        StepInfo::new("drive", "Drive", Group::Source, ItemKind::Nothing, ItemKind::Disc).from_param("drive").param(crate::real::drive_param()).param(crate::real::offset_param())
    }

    async fn find(&self, key: &str, drive: &str, _measure: bool) -> Option<(serde_json::Value, String)> {
        (key == "offset").then(|| {
            let o = if drive == crate::recipes::DVD { 6 } else { 103 };
            (serde_json::Value::from(o), "AccurateRip, 528 agree".to_owned())
        })
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        inlet.recv().await;
        cx.status("", "Reading the disc");
        cx.log(format!("waiting for a disc in the {}", cx.text("drive")));
        cx.sleep(400).await?;
        cx.log("a disc is in: audio, 17 tracks");
        cx.status("Disc in", "");
        outlet.send(Item::disc("The White Stripes")).await;
        Ok(())
    }
}

// ---- the kind step --------------------------------------------------------------------

struct AudioCd;

#[async_trait]
impl Step for AudioCd {
    fn info(&self) -> StepInfo {
        StepInfo::new("audio-cd", "Audio CD", Group::Kind, ItemKind::Disc, ItemKind::AudioDisc).gives(&["toc"]).from("read the track list")
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            cx.status("", "Reading the table of contents");
            cx.sleep(600).await?;
            let total: u32 = TRACKS.iter().map(|t| t.1).sum();
            {
                let c = disc.context_mut();
                c.add(Subject::Disc, "parts", TRACKS.len().to_string(), "the disc");
                // What the disc's CD-Text says, as the real one reads it:
                // kept aside for the CD-Text step to offer.
                c.add(Subject::Disc, "cdtext.album", "THE WHITE STRIPES", "the disc");
                c.add(Subject::Disc, "cdtext.artist", "THE WHITE STRIPES", "the disc");
                for (i, (t, len)) in TRACKS.iter().enumerate() {
                    c.add(Subject::Part(i as u32 + 1), "length", len.to_string(), "the disc");
                    c.add(Subject::Part(i as u32 + 1), "cdtext.title", t.to_uppercase(), "the disc");
                }
            }
            cx.log(format!("table of contents: {} tracks, {}", TRACKS.len(), crate::real::mmss(total)));
            cx.expect(TRACKS.len());
            cx.outcome(format!("{} tracks", TRACKS.len()), crate::real::mmss(total), "");
            outlet.send(disc).await;
        }
        Ok(())
    }
}

// ---- lookups ------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Lookup {
    MusicBrainz,
    CdText,
    Discogs,
    GnuDb,
    CoverArt,
    Lyrics,
}

/// A release's label and catalogue number, as MusicBrainz gives them.
trait Labelled {
    fn labelled(self, label: &str, catno: &str) -> Self;
}

impl Labelled for Candidate {
    fn labelled(mut self, label: &str, catno: &str) -> Self {
        for (k, v) in [("label", label), ("catalognumber", catno)] {
            self.facts.push(Fact { step: String::new(), subject: Subject::Disc, key: k.into(), value: v.into(), source: self.source.clone() });
        }
        self
    }
}

fn release(id: &str, source: &str, year: &str, detail: &str, score: f32) -> Candidate {
    let mut facts = vec![
        Fact { step: String::new(), subject: Subject::Disc, key: "album".into(), value: "The White Stripes".into(), source: source.into() },
        Fact { step: String::new(), subject: Subject::Disc, key: "artist".into(), value: "The White Stripes".into(), source: source.into() },
        Fact { step: String::new(), subject: Subject::Disc, key: "year".into(), value: year.into(), source: source.into() },
    ];
    for (i, (t, _)) in TRACKS.iter().enumerate() {
        facts.push(Fact { step: String::new(), subject: Subject::Part(i as u32 + 1), key: "title".into(), value: (*t).into(), source: source.into() });
    }
    Candidate { step: String::new(), id: id.into(), source: source.into(), title: "The White Stripes".into(), detail: detail.into(), score, facts }
}

#[async_trait]
impl Step for Lookup {
    fn info(&self) -> StepInfo {
        // The real steps' own, where there is one.
        match self {
            Lookup::Discogs => return crate::real::discogs::info(),
            Lookup::GnuDb => return crate::real::gnudb::info(),
            Lookup::Lyrics => return crate::real::lyrics::info(),
            Lookup::CoverArt => return crate::real::cover_art_info(),
            _ => {}
        }
        let (kind, name, from) = match self {
            Lookup::MusicBrainz => ("musicbrainz", "MusicBrainz", "free online music database"),
            Lookup::CdText => ("cd-text", "CD-Text", "track names stored on the disc"),
            Lookup::Discogs => ("discogs", "Discogs", "record collectors' database"),
            Lookup::GnuDb => ("gnudb", "GnuDB", "older CD database"),
            Lookup::CoverArt => ("cover-art", "Cover art", "Cover Art Archive"),
            Lookup::Lyrics => ("lyrics", "Lyrics", "words for each track, from LRCLIB"),
        };
        let info = StepInfo::new(kind, name, Group::LookUp, ItemKind::AudioDisc, ItemKind::AudioDisc).from(from);
        // Lookups that work out what the disc is have nothing to add once
        // Choose has settled it.
        // They read the disc, so they go straight after Audio CD.
        let info = if matches!(self, Lookup::CoverArt | Lookup::Lyrics) { info } else { info.unless(&["release"]).right_after(&["audio-cd"]) };
        match self {
            Lookup::MusicBrainz | Lookup::Discogs | Lookup::GnuDb => info.gives(&["candidates"]),
            Lookup::CdText => info.gives(&["candidates"]),
            // They look up the release Choose settled, so straight after it.
            Lookup::CoverArt => info.needs(&["release"]).right_after(&["choose"]).gives(&["cover"]),
            Lookup::Lyrics => info.needs(&["release"]).right_after(&["choose"]).gives(&["lyrics"]),
        }
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let _net = cx.lane(LaneKind::Network(self.info().name.into())).await;
            cx.status(
                "",
                match self {
                    Lookup::MusicBrainz => "Asking by disc ID",
                    Lookup::Discogs => "Asking by barcode",
                    Lookup::GnuDb => "Asking by freedb ID",
                    Lookup::CoverArt => "Fetching the front",
                    Lookup::Lyrics => "Asking LRCLIB",
                    Lookup::CdText => "Reading the disc's text",
                },
            );
            match self {
                Lookup::MusicBrainz => {
                    cx.sleep(900).await?;
                    disc.context_mut().offer([
                        release("mb-1999", "MusicBrainz", "1999", "1999, US, Sympathy for the Record Industry, SFTRI 577", 0.9).labelled("Sympathy for the Record Industry", "SFTRI 577"),
                        release("mb-2002", "MusicBrainz", "2002", "2002, UK, XL Recordings, XLCD 151", 0.9).labelled("XL Recordings", "XLCD 151"),
                    ]);
                    cx.log("asked by disc ID 1kXKLV1oMH.3W8xOYqFjM8kc7hQ-");
                    cx.log("2 releases: 1999 (Sympathy), 2002 (XL)");
                    cx.status("2 found", "");
                }
                Lookup::CdText => {
                    cx.sleep(300).await?;
                    crate::real::offer_cdtext(&cx, &mut disc);
                }
                Lookup::Discogs => {
                    cx.sleep(1400).await?;
                    disc.context_mut().offer([release("dg-1", "Discogs", "1999", "1999, US, CD", 0.85).labelled("Sympathy for the Record Industry", "SFTRI 577")]);
                    cx.log("asked by barcode 790377057721");
                    cx.status("1 found", "");
                }
                Lookup::GnuDb => {
                    cx.sleep(2500).await?;
                    // A lookup that gets no answer doesn't stop the disc:
                    // Choose decides with what the others found. It went on
                    // without doing its job, which is a skip, not a failure.
                    cx.item(&disc, ItemState::Skipped, "no answer");
                    cx.status("No answer", "after 10 s");
                    cx.log("GnuDB did not answer within 10 s");
                }
                Lookup::CoverArt => {
                    cx.sleep(700).await?;
                    // The release Choose settled, on the disc.
                    let found = disc.context().chosen().is_some();
                    disc.context_mut().add(Subject::Disc, "cover", "front.jpg", "Cover Art Archive");
                    cx.log("fetched the front cover, 1200 × 1200");
                    cx.made_sized(std::path::Path::new("front.jpg"), 243_712);
                    if found {
                        cx.status("1200 × 1200", "")
                    } else {
                        cx.status("Nothing found", "no album chosen")
                    }
                }
                Lookup::Lyrics => {
                    cx.sleep(1200).await?;
                    // Three of the disc's tracks have none.
                    let n = parts(disc.context());
                    let none = [3, 10, 15];
                    for t in 1..=n {
                        let track = Item::track(t, title(disc.context(), t));
                        if none.contains(&t) {
                            cx.item(&track, ItemState::Skipped, "none found");
                            continue;
                        }
                        let c = disc.context_mut();
                        c.add(Subject::Part(t), "lyrics", "[00:01.00]…", "LRCLIB");
                        c.add(Subject::Part(t), "lyrics.synced", "yes", "LRCLIB");
                        cx.item(&track, ItemState::Ok, "synced");
                    }
                    let found = n - none.iter().filter(|t| **t <= n).count() as u32;
                    cx.log(format!("{found} of {n} have synced lyrics; 3 have none"));
                    cx.status(format!("{found}/{n} synced"), crate::real::lyrics::none_for(&none));
                }
            }
            outlet.send(disc).await;
        }
        Ok(())
    }
}

// ---- rip ------------------------------------------------------------------------------------

struct Rip {
    bad_track: Option<u32>,
}

#[async_trait]
impl Step for Rip {
    fn info(&self) -> StepInfo {
        StepInfo::new("rip", "Rip", Group::Read, ItemKind::AudioDisc, ItemKind::Track)
            // Something has come off the disc: what Next disc needs.
            .gives(&["read"])
            .from("read each track and check it")
            .param(crate::real::first_read_param())
            .retry("two_reads", "Read again", "~1 min")
            .retry("combine", "Best of 3 reads", "~2 min")
            .retry("other_drive", "Try the other drive", "move disc")
            .retry("slowly", "Read again slowly", "~1 min")
            .retry("keep", "Keep it anyway", "")
            .retry("leave_out", "Skip this track", "")
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let Some(disc) = inlet.recv().await else { return Ok(()) };
        let n_tracks = parts(disc.context());
        // Each track knows what the disc knew, the chosen release with it.
        let mut items: Vec<Item> = (1..=n_tracks).map(|n| Item::track(n, title(disc.context(), n)).knowing(&disc)).collect();
        for i in &items {
            cx.item(i, ItemState::Waiting, "");
        }
        let mut bad: Vec<Item> = Vec::new();
        let mut done = 0;
        {
            let _drive = cx.lane(LaneKind::Drive).await;
            cx.log(format!("reading {n_tracks} tracks"));
            // Time left at 8×, from the audio still to read.
            let left = |from: u32| -> String {
                let secs: u32 = TRACKS.iter().skip(from as usize - 1).map(|t| t.1).sum::<u32>() / 8;
                if secs >= 90 {
                    format!("{} min left", (secs as f32 / 60.0).round())
                } else {
                    format!("{secs} s left")
                }
            };
            for item in items.iter_mut() {
                let n = item.index.unwrap();
                cx.drive_says(StepState::Busy, "Reading at 8×", "");
                cx.item(item, ItemState::Busy, "reading");
                crate::real::rip_says(&cx, format!("{} · {}", crate::real::track_line(n, &item.label), left(n)));
                cx.raw(format!("Ripping track {n}, 0.0% [------------------------------] (avg speed:  0.00x)"));
                cx.sleep(TRACKS.get(n as usize - 1).map_or(150, |t| t.1) as u64 * 12).await?;
                cx.raw(format!("Ripping track {n}, 100.0% [==============================] (avg speed:  8.12x)"));
                if Some(n) == self.bad_track {
                    cx.raw("    Accurip:         not found (Accurip v1: no match, v2: no match)");
                    cx.raw(format!("Track {n} ripped and encoded successfully!"));
                    cx.log(format!("track {n}: no match in AccurateRip"));
                    crate::real::rip_verdict(&cx, item, ItemState::Bad, "no match");
                    bad.push(item.clone());
                } else {
                    cx.raw(format!("    Accurip:         verified (v2, confidence {})", 103 + n % 2));
                    cx.raw(format!("Track {n} ripped and encoded successfully!"));
                    crate::real::rip_verdict(&cx, item, ItemState::Ok, format!("✓ {}", 103 + n % 2));
                    done += 1;
                    outlet.send(item.becomes(ItemKind::Track, Some(PathBuf::from(format!("{n:02}.wav"))))).await;
                }
                cx.progress(Some(n as f32 / n_tracks as f32));
            }
            cx.drive_says(StepState::Idle, "Waiting", "");
        }
        while !bad.is_empty() {
            cx.state(StepState::Bad);
            let notes: Vec<(u32, String)> = bad.iter().map(|i| (i.index.unwrap(), "no match".to_owned())).collect();
            crate::real::rip_says(&cx, crate::real::rip_needs(&notes));
            let Some(req) = cx.next_retry().await else { return Ok(()) };
            let Some(pos) = bad.iter().position(|i| i.id == req.item) else { continue };
            let mut item = bad[pos].clone();
            match req.option.as_str() {
                "keep" => {
                    crate::real::rip_verdict(&cx, &mut item, ItemState::Ok, crate::real::KEPT);
                    outlet.send(item.becomes(ItemKind::Track, None)).await;
                }
                "leave_out" => crate::real::rip_verdict(&cx, &mut item, ItemState::Skipped, "skipped"),
                _ => {
                    cx.state(StepState::Busy);
                    let _drive = cx.lane(LaneKind::Drive).await;
                    cx.drive_says(StepState::Busy, "Re-reading carefully", "");
                    crate::real::rip_says(&cx, format!("{} · reading again", crate::real::track_line(item.index.unwrap(), &item.label)));
                    cx.item(&item, ItemState::Busy, "re-reading");
                    cx.sleep(3000).await?;
                    crate::real::rip_verdict(&cx, &mut item, ItemState::Ok, "✓ two reads agree");
                    cx.drive_says(StepState::Idle, "Waiting", "");
                    outlet.send(item.becomes(ItemKind::Track, None)).await;
                }
            }
            bad.remove(pos);
            done += 1;
        }
        cx.log(format!("{done} of {n_tracks} tracks read; AccurateRip verified each"));
        cx.state(StepState::Busy);
        crate::real::rip_done(&cx);
        Ok(())
    }
}

// ---- encoders --------------------------------------------------------------------------------

/// A simulated encoder with the real one's name and settings.
#[derive(Clone, Copy)]
struct Encode(crate::real::encode::Codec);

#[async_trait]
impl Step for Encode {
    fn info(&self) -> StepInfo {
        self.0.info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let (busy, done) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let sums: Arc<std::sync::Mutex<crate::real::encode::Sums>> = Arc::default();
        let ext = self.0.ext();
        let codec = self.0;
        let mut tasks = Vec::new();
        while let Some(track) = inlet.recv().await {
            // Known once the kind step has listed the tracks, which is before any arrives.
            let total = parts(track.context());
            let (cx, outlet, busy, done, sums) = (cx.clone(), outlet.clone(), busy.clone(), done.clone(), sums.clone());
            tasks.push(tokio::spawn(async move {
                let _core = cx.lane(LaneKind::Encode).await;
                let shown = |cx: &Cx| {
                    let (b, d) = (busy.load(Ordering::SeqCst), done.load(Ordering::SeqCst));
                    let on = if b > 0 { crate::real::encode::on_track(&track) } else { String::new() };
                    crate::real::encode::encoder_says(cx, codec, *sums.lock().unwrap(), on);
                    cx.progress(Some(d as f32 / total as f32));
                };
                busy.fetch_add(1, Ordering::SeqCst);
                cx.item(&track, ItemState::Busy, "encoding");
                shown(&cx);
                let r = cx.sleep(1500).await;
                busy.fetch_sub(1, Ordering::SeqCst);
                if r.is_ok() {
                    done.fetch_add(1, Ordering::SeqCst);
                    cx.item(&track, ItemState::Ok, "");
                    let n = track.index.unwrap_or(0);
                    let secs = TRACKS.get(n as usize - 1).map_or(150, |t| t.1) as u64;
                    let size = secs * sim_rate(ext);
                    sums.lock().unwrap().add(crate::real::encode::Sums { in_bytes: secs * sim_rate("wav"), audio_bytes: size, millis: secs * 1000 });
                    cx.raw(format!("size={:>8}kB time=00:{:02}:{:02}.00 bitrate={:.1}kbits/s speed=46.2x", size / 1024, secs / 60, secs % 60, size as f64 * 8.0 / 1000.0 / secs as f64));
                    let path = PathBuf::from(format!("{n:02}.{ext}"));
                    cx.made_sized(&path, size);
                    outlet.send(track.becomes(ItemKind::AudioFile, Some(path))).await;
                }
                shown(&cx);
            }));
        }
        for t in tasks {
            t.await?;
        }
        // Each task writes the tile as it ends, and two ending together can
        // write in either order; say the final count once, after all of them.
        let sums = *sums.lock().unwrap();
        crate::real::encode::encoder_says(&cx, codec, sums, String::new());
        cx.progress(None);
        Ok(())
    }
}

/// Bytes a second, roughly, of a simulated file of this kind.
fn sim_rate(ext: &str) -> u64 {
    match ext {
        "flac" => 112_000,
        "wav" => 176_400,
        "mp3" => 30_000,
        "opus" | "ogg" => 20_000,
        "m4a" => 32_000,
        _ => 80_000,
    }
}

// ---- folders -----------------------------------------------------------------------------------

struct Place {
    kind: &'static str,
    name: &'static str,
}

#[async_trait]
impl Step for Place {
    fn info(&self) -> StepInfo {
        crate::real::data::place_info(self.kind, self.name)
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let folder = cx.text("folder");
        let mut done = 0;
        let mut last = String::new();
        let root = PathBuf::from(&folder);
        while let Some(file) = inlet.recv().await {
            if file.id.starts_with(crate::real::video::ITEM_PREFIX) {
                // Where it would go, from the real template.
                let (names, part) = (cx.text("names"), file.index.unwrap_or(0));
                let dest = crate::real::video::destination(file.context(), &names, std::path::Path::new(&folder), part, "mkv");
                cx.sleep(400).await?;
                done += 1;
                cx.item(&file, ItemState::Ok, dest.display().to_string());
                last = crate::real::where_filed(&root, &dest);
                crate::real::filed_says(&cx, done, "");
                outlet.send(file.becomes(file.kind, Some(dest))).await;
                continue;
            }
            if file.id == data::DUMP {
                // Where it would go, from the real template.
                let names = cx.text("names");
                let dir = PathBuf::from(&folder).join(data::dump_folder(file.context(), &names));
                cx.sleep(400).await?;
                done += 1;
                cx.item(&file, ItemState::Ok, dir.display().to_string());
                last = crate::real::where_filed(&root, &dir.join("x"));
                crate::real::filed_says(&cx, done, "");
                outlet.send(file).await;
                continue;
            }
            let total = parts(file.context());
            let _f = cx.lane(LaneKind::File(folder.clone())).await;
            cx.item(&file, ItemState::Busy, "copying");
            let name = file.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let shown = match file.index {
                Some(n) => format!("{n:02} - {}.{}", file.label, std::path::Path::new(&name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()),
                None => name.clone(),
            };
            crate::real::filed_says(&cx, done, shown);
            cx.sleep(250).await?;
            done += 1;
            cx.item(&file, ItemState::Ok, "");
            // Filed under its folder, as the real one names it (roughly).
            let name = file.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = std::path::Path::new(&name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            let dest = match file.index {
                Some(n) => PathBuf::from(&folder).join("The White Stripes/The White Stripes").join(format!("{n:02} - {}.{ext}", file.label)),
                None => PathBuf::from(&folder).join("The White Stripes/The White Stripes").join(&name),
            };
            let secs = file.index.and_then(|n| TRACKS.get(n as usize - 1)).map_or(1, |t| t.1) as u64;
            cx.made_sized(&dest, if file.index.is_some() { secs * sim_rate(&ext) } else { 2_048 });
            let file = file.becomes(file.kind, Some(dest));
            last = crate::real::where_filed(&root, file.path.as_deref().unwrap_or(&root));
            crate::real::filed_says(&cx, done, "");
            cx.progress(Some(done as f32 / total as f32));
            outlet.send(file).await;
        }
        if done > 0 {
            crate::real::filed_says(&cx, done, last);
        }
        Ok(())
    }
}

// ---- steps that need everything first -------------------------------------------------------------

struct ReplayGain;

#[async_trait]
impl Step for ReplayGain {
    fn info(&self) -> StepInfo {
        crate::real::replaygain::info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let mut files = Vec::new();
        while let Some(f) = inlet.recv().await {
            let total = parts(f.context());
            files.push(f);
            cx.status("", format!("Waits for all {total}, {} so far", files.len()));
        }
        cx.status("", "Measuring the album");
        cx.sleep(1500).await?;
        // Believable gains: louder tracks get less.
        let mut gains = vec![];
        for f in files {
            let n = f.index.unwrap_or(1);
            let g = -9.1 + ((n * 7) % 17) as f64 * 3.9 / 16.0;
            gains.push(g);
            cx.item(&f, ItemState::Ok, format!("{g:+.2} dB"));
            outlet.send(f).await;
        }
        crate::real::replaygain::gain_says(&cx, Some(-7.3), &gains);
        Ok(())
    }
}

struct CueSheet;

#[async_trait]
impl Step for CueSheet {
    fn info(&self) -> StepInfo {
        crate::real::cue::info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let tracks = inlet.all().await;
        cx.status("", format!("Writing for {} tracks", tracks.len()));
        cx.expect(2);
        cx.sleep(300).await?;
        let mut known = Context::default();
        for t in &tracks {
            known.merge(t.context());
        }
        let known = Arc::new(known);
        // Beside the files, as the real one writes them.
        let dir = tracks.iter().find_map(|t| t.path.as_ref()?.parent().map(PathBuf::from)).unwrap_or_default();
        for (id, label, file, size) in [("cue", "Cue sheet", "disc.cue", 2_311), ("log", "Rip log", "disc.log", 9_472)] {
            let path = dir.join(file);
            let mut item = Item::new(id, ItemKind::TextFile, label, None, Some(path.clone()));
            item.context = known.clone();
            cx.made_sized(&path, size);
            cx.item(&item, ItemState::Ok, file);
            outlet.send(item).await;
        }
        cx.status("disc.cue, disc.log", "");
        Ok(())
    }
}

struct LyricsFile;

#[async_trait]
impl Step for LyricsFile {
    fn info(&self) -> StepInfo {
        crate::real::lyrics::file_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let (mut written, mut none) = (0, 0);
        while let Some(f) = inlet.recv().await {
            let (Some(t), Some(dest)) = (f.index, f.path.clone()) else { continue };
            if f.context().get(Subject::Part(t), "lyrics.synced") == Some("yes") {
                written += 1;
                let lrc = dest.with_extension("lrc");
                cx.made_sized(&lrc, 1_900);
                cx.item(&f, ItemState::Ok, lrc.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                outlet.send(f.becomes(ItemKind::TextFile, Some(lrc))).await;
            } else {
                none += 1;
                cx.item(&f, ItemState::Skipped, "no synced lyrics");
            }
            let _ = none;
            cx.status(format!("{written} .lrc file{}", if written == 1 { "" } else { "s" }), "");
        }
        Ok(())
    }
}

struct Tell;

#[async_trait]
impl Step for Tell {
    fn info(&self) -> StepInfo {
        crate::real::tell::info()
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, cx: Cx) -> Result<()> {
        let all = inlet.all().await;
        // What the real one would say, without putting it on the desktop.
        let o = crate::real::tell::Outcome { needed_you: cx.needed_you(), ..crate::real::tell::Outcome::of(&all) };
        let told = crate::real::tell::should_tell(&cx.text("when"), &o);
        cx.status(if told { "Sent" } else { "Not sent" }, o.body());
        Ok(())
    }
}

// ---- data discs ---------------------------------------------------------------------------------

use crate::real::data;

/// A datfile in Redump's shape, with the simulated disc in it.
pub const REDUMP_DAT: &str = r#"<?xml version="1.0"?>
<datafile>
  <header>
    <name>IBM - PC compatible</name>
    <version>2026-06-15 06-43-57</version>
  </header>
  <game name="Encarta 98 Encyclopedia (USA) (Disc 1)">
    <rom name="Encarta 98 Encyclopedia (USA) (Disc 1).bin" size="681984000" crc="2a3b4c5d" md5="0123456789abcdef0123456789abcdef" sha1="AAAA000000000000000000000000000000000001"/>
    <rom name="Encarta 98 Encyclopedia (USA) (Disc 1).cue" size="104" crc="11111111" md5="22222222222222222222222222222222" sha1="cccc000000000000000000000000000000000003"/>
  </game>
  <game name="Myst &amp; Riven (Europe)">
    <rom name="Myst &amp; Riven (Europe) (Track 1).bin" size="100" crc="01" md5="01" sha1="bbbb000000000000000000000000000000000001"/>
    <rom name="Myst &amp; Riven (Europe) (Track 2).bin" size="100" crc="02" md5="02" sha1="bbbb000000000000000000000000000000000002"/>
  </game>
</datafile>"#;

/// The simulated data disc: Encarta 98, disc 1, a CD Redump knows.
pub const DATA_LABEL: &str = "ENCARTA98";
const DATA_BYTES: u64 = 681_984_000;
const DATA_SHA1: &str = "aaaa000000000000000000000000000000000001";

struct DataDisc;

#[async_trait]
impl Step for DataDisc {
    fn info(&self) -> StepInfo {
        data::data_disc_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(disc) = inlet.recv().await {
            cx.sleep(200).await?;
            let mut d = disc.becomes(ItemKind::DataDisc, None);
            {
                let c = d.context_mut();
                for (k, v) in [("label", DATA_LABEL), ("filesystem", "ISO 9660"), ("media", "CD"), ("parts", "1"), ("audio_tracks", "0")] {
                    c.add(Subject::Disc, k, v, "the disc");
                }
                c.add(Subject::Disc, "bytes", DATA_BYTES.to_string(), "the disc");
            }
            cx.status(DATA_LABEL, format!("ISO 9660 · {}", crate::real::media::size(DATA_BYTES)));
            d.label = DATA_LABEL.into();
            outlet.send(d).await;
        }
        Ok(())
    }
}

struct Image {
    /// The first read leaves sectors unreadable.
    bad: bool,
}

#[async_trait]
impl Step for Image {
    fn info(&self) -> StepInfo {
        data::image_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let Some(disc) = inlet.recv().await else { return Ok(()) };
        let item = Item::new(data::DUMP, ItemKind::Dump, DATA_LABEL, None, None).knowing(&disc);
        let main = PathBuf::from(format!("{DATA_LABEL}.cue"));
        {
            let _drive = cx.lane(LaneKind::Drive).await;
            for (phase, ms) in [("Reading", 3000), ("Writing tracks", 300)] {
                cx.status("", phase);
                cx.drive_says(StepState::Busy, phase, "");
                for i in 1..=4 {
                    cx.raw(format!("LBA: {:>6}/332999, errors: {{ SCSI: 0, C2: 0 }}", i * 83_250));
                    cx.item(&item, ItemState::Busy, format!("{}%, sector {} of 332999", i * 25, i * 83_250));
                    cx.progress(Some(i as f32 / 4.0));
                    cx.sleep(ms / 4).await?;
                }
            }
            cx.drive_says(StepState::Idle, "Waiting", "");
        }
        cx.progress(None);
        if self.bad {
            cx.raw("media errors: { SCSI: 3, C2: 0, Q: 0 }");
            cx.log("3 sectors couldn't be read");
            cx.item(&item, ItemState::Bad, "3 unreadable sectors");
            cx.state(StepState::Bad);
            cx.status("", "3 sectors unreadable");
            let req = loop {
                match cx.next_retry().await {
                    None => return Ok(()),
                    Some(r) if r.item == item.id => break r,
                    Some(_) => {}
                }
            };
            cx.state(StepState::Busy);
            match req.option.as_str() {
                "leave_out" => {
                    cx.item(&item, ItemState::Skipped, "skipped");
                    return Ok(());
                }
                "keep" => cx.item(&item, ItemState::Ok, "kept anyway, 3 unreadable sectors left blank"),
                _ => {
                    let _drive = cx.lane(LaneKind::Drive).await;
                    cx.drive_says(StepState::Busy, "Re-reading bad parts", "");
                    cx.item(&item, ItemState::Busy, "re-reading bad parts");
                    cx.sleep(2000).await?;
                    cx.drive_says(StepState::Idle, "Waiting", "");
                    cx.item(&item, ItemState::Ok, "✓ 682 MB, bad parts re-read");
                }
            }
            cx.status("682 MB", "3 sectors re-read");
        } else {
            cx.item(&item, ItemState::Ok, "✓ 682 MB, read cleanly");
            cx.status("682 MB", "");
        }
        outlet.send(item.becomes(ItemKind::Dump, Some(main))).await;
        Ok(())
    }
}

struct Check;

#[async_trait]
impl Step for Check {
    fn info(&self) -> StepInfo {
        data::check_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut dump) = inlet.recv().await {
            if crate::game_sim::check(&cx, &dump, &outlet).await? {
                continue;
            }
            cx.item(&dump, ItemState::Busy, "checking");
            cx.status("", "Checking 1 file");
            cx.sleep(800).await?;
            let roms = [crate::real::redump::Rom { name: format!("{DATA_LABEL}.bin"), size: DATA_BYTES, crc: "2a3b4c5d".into(), md5: String::new(), sha1: DATA_SHA1.into() }];
            dump.context_mut().add(Subject::Disc, &format!("sha1 {DATA_LABEL}.bin"), DATA_SHA1, "Check");
            // The real parser and lookup, on a datfile in Redump's shape.
            let media = dump.context().get(Subject::Disc, "media").unwrap_or("").to_owned();
            let systems = crate::real::redump::systems_for(&cx.text("systems"), &media);
            let found = systems.iter().find(|s| s.slug == "pc").and_then(|sys| {
                let cat = crate::real::redump::parse(REDUMP_DAT.as_bytes()).ok()?;
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
            outlet.send(dump).await;
        }
        Ok(())
    }
}

struct Chd;

#[async_trait]
impl Step for Chd {
    fn info(&self) -> StepInfo {
        data::chd_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(dump) = inlet.recv().await {
            let _core = cx.lane(LaneKind::Encode).await;
            let name = crate::template::safe(&data::dump_fields(dump.context()).name);
            for (what, ms) in [("compressing", 2000), ("verifying", 800)] {
                cx.status("", if what == "compressing" { "Compressing" } else { "Verifying" });
                for i in 1..=4 {
                    cx.item(&dump, ItemState::Busy, format!("{what} {}%", i * 25));
                    cx.progress(Some(i as f32 / 4.0));
                    cx.sleep(ms / 4).await?;
                }
            }
            cx.progress(None);
            cx.raw("Compression complete ... final ratio = 60.4%");
            cx.made_sized(&PathBuf::from(format!("{name}.chd")), 431_996_928);
            let note = "412 MB, 60% of the image, verified: every track matches the dump (1)";
            cx.item(&dump, ItemState::Ok, note);
            cx.outcome("412 MB", "60 %", "");
            outlet.send(dump.becomes(ItemKind::ImageFile, Some(PathBuf::from(format!("{name}.chd"))))).await;
        }
        Ok(())
    }
}

struct Keep;

#[async_trait]
impl Step for Keep {
    fn info(&self) -> StepInfo {
        data::keep_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(dump) = inlet.recv().await {
            cx.item(&dump, ItemState::Ok, "3 files");
            cx.status(crate::real::media::size(DATA_BYTES), "");
            let path = dump.path.clone();
            outlet.send(dump.becomes(ItemKind::ImageFile, path)).await;
        }
        Ok(())
    }
}
