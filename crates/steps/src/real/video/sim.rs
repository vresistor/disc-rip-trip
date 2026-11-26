//! The video steps simulated: the real steps' infos, and the real parser,
//! matching and TMDb scoring run on canned MakeMKV and TMDb answers. The
//! discs:
//!
//! - *Blade Runner*: a film, its duplicate, a trailer. Nothing to ask.
//! - *The Sopranos* season 3 disc 1: four episodes, one TMDb has no runtime
//!   for, a duplicate and a trailer. Asks which episode each title is.
//! - *The Sopranos* season 3 disc 2: four episodes the runtimes place.
//!   Nothing to ask.
//! - *Twin Peaks* Gold Box disc 2: no season on the label, and four
//!   47-minute episodes that fit season 1 and season 2 alike. Asks which.

use super::{facts, makemkv, rip_items, tmdb, Tmdb};
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use trip_engine::{Cx, Inlet, ItemKind, ItemState, LaneKind, Outlet, Registry, Step, StepInfo, StepState, Subject};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimDisc {
    Film,
    Season,
    SeasonDisc2,
    BoxSet,
}

/// Registers the simulated video steps, with this disc in the drive.
pub fn register(r: &mut Registry, disc: SimDisc) {
    r.add(VideoDisc(disc));
    r.add(Tmdb { fixtures: Some(|| Box::new(Fixtures)) });
    r.add(super::choose::ChooseVideo);
    r.add(Rip);
    r.add(Make { video: true });
    r.add(Make { video: false });
}

/// `makemkvcon -r info` for the film disc.
pub const FILM: &str = r#"MSG:1005,0,1,"MakeMKV v1.17.7 linux(x64-release) started","%1 started","MakeMKV v1.17.7 linux(x64-release)"
CINFO:1,6206,"DVD disc"
CINFO:32,0,"BLADE_RUNNER"
TINFO:0,8,0,"34"
TINFO:0,9,0,"1:57:30"
TINFO:0,11,0,"7623458816"
TINFO:0,26,0,"1-34"
SINFO:0,0,1,6201,"Video"
SINFO:0,1,1,6202,"Audio"
SINFO:0,1,3,0,"eng"
SINFO:0,2,1,6202,"Audio"
SINFO:0,2,3,0,"fra"
SINFO:0,3,1,6203,"Subtitles"
SINFO:0,3,3,0,"eng"
TINFO:1,8,0,"34"
TINFO:1,9,0,"1:57:30"
TINFO:1,11,0,"7623458816"
TINFO:1,26,0,"1-34"
TINFO:2,8,0,"1"
TINFO:2,9,0,"0:03:10"
TINFO:2,11,0,"190000000"
TINFO:2,26,0,"40"
MSG:5011,0,0,"Operation successfully completed","Operation successfully completed"
"#;

/// `makemkvcon -r info` for the season disc.
pub const SEASON: &str = r#"CINFO:1,6206,"DVD disc"
CINFO:32,0,"THE_SOPRANOS_S3_D1"
TINFO:0,8,0,"8"
TINFO:0,9,0,"50:12"
TINFO:0,11,0,"2100000000"
TINFO:0,26,0,"1-8"
SINFO:0,1,1,6202,"Audio"
SINFO:0,1,3,0,"eng"
TINFO:1,8,0,"8"
TINFO:1,9,0,"57:40"
TINFO:1,11,0,"2400000000"
TINFO:1,26,0,"9-16"
SINFO:1,1,1,6202,"Audio"
SINFO:1,1,3,0,"eng"
TINFO:2,8,0,"8"
TINFO:2,9,0,"55:51"
TINFO:2,11,0,"2300000000"
TINFO:2,26,0,"17-24"
TINFO:3,8,0,"8"
TINFO:3,9,0,"53:58"
TINFO:3,11,0,"2250000000"
TINFO:3,26,0,"25-32"
TINFO:4,8,0,"8"
TINFO:4,9,0,"57:40"
TINFO:4,11,0,"2400000000"
TINFO:4,26,0,"9-16"
TINFO:5,8,0,"1"
TINFO:5,9,0,"3:10"
TINFO:5,11,0,"90000000"
TINFO:5,26,0,"33"
"#;

pub const MOVIE_SEARCH: &str = r#"{"page":1,"results":[
    {"id":78,"title":"Blade Runner","release_date":"1982-06-25","overview":"","vote_count":14000},
    {"id":335984,"title":"Blade Runner 2049","release_date":"2017-10-04","overview":"","vote_count":13000},
    {"id":999001,"title":"Blade Runner","release_date":"","overview":"","vote_count":1}]}"#;

pub const TV_SEARCH: &str = r#"{"page":1,"results":[
    {"id":1398,"name":"The Sopranos","first_air_date":"1999-01-10","overview":"","vote_count":3000},
    {"id":999002,"name":"The Sopranos 1999","first_air_date":"","vote_count":0}]}"#;

pub const SEASON_3: &str = r#"{"name":"Season 3","season_number":3,"episodes":[
    {"episode_number":1,"season_number":3,"name":"Mr. Ruggerio's Neighborhood","runtime":50},
    {"episode_number":2,"season_number":3,"name":"Proshai, Livushka","runtime":58},
    {"episode_number":3,"season_number":3,"name":"Fortunate Son","runtime":56},
    {"episode_number":4,"season_number":3,"name":"Employee of the Month","runtime":null},
    {"episode_number":5,"season_number":3,"name":"Another Toothpick","runtime":55},
    {"episode_number":6,"season_number":3,"name":"University","runtime":52},
    {"episode_number":7,"season_number":3,"name":"Second Opinion","runtime":49},
    {"episode_number":8,"season_number":3,"name":"He Is Risen","runtime":60}]}"#;

/// `makemkvcon -r info` for the season's second disc: episodes 5 to 8.
pub const SEASON_DISC_2: &str = r#"CINFO:1,6206,"DVD disc"
CINFO:32,0,"THE_SOPRANOS_S3_D2"
TINFO:0,9,0,"54:58"
TINFO:0,26,0,"1-8"
TINFO:1,9,0,"52:05"
TINFO:1,26,0,"9-16"
TINFO:2,9,0,"49:00"
TINFO:2,26,0,"17-24"
TINFO:3,9,0,"1:00:00"
TINFO:3,26,0,"25-32"
"#;

/// `makemkvcon -r info` for a box set's disc that only says which disc.
pub const BOX_SET: &str = r#"CINFO:1,6206,"DVD disc"
CINFO:32,0,"TWIN_PEAKS_GOLD_BOX_DISC_2"
TINFO:0,9,0,"47:10"
TINFO:0,26,0,"1-6"
TINFO:1,9,0,"46:30"
TINFO:1,26,0,"7-12"
TINFO:2,9,0,"46:50"
TINFO:2,26,0,"13-18"
TINFO:3,9,0,"46:45"
TINFO:3,26,0,"19-24"
TINFO:4,9,0,"4:00"
TINFO:4,26,0,"25"
"#;

pub const TWIN_PEAKS_SEARCH: &str = r#"{"page":1,"results":[
    {"id":1920,"name":"Twin Peaks","first_air_date":"1990-04-08","overview":"","vote_count":2640}]}"#;

/// Season 1: the pilot at 94 minutes, then 47s. Season 2 is 47s too.
pub const TWIN_PEAKS_SEASON_1: &str = r#"{"season_number":1,"episodes":[
    {"episode_number":1,"season_number":1,"name":"Pilot","runtime":94},
    {"episode_number":2,"season_number":1,"name":"Traces to Nowhere","runtime":47},
    {"episode_number":3,"season_number":1,"name":"Zen, or the Skill to Catch a Killer","runtime":47},
    {"episode_number":4,"season_number":1,"name":"Rest in Pain","runtime":47},
    {"episode_number":5,"season_number":1,"name":"The One-Armed Man","runtime":47}]}"#;

pub const TWIN_PEAKS_SEASON_2: &str = r#"{"season_number":2,"episodes":[
    {"episode_number":1,"season_number":2,"name":"May the Giant Be with You","runtime":94},
    {"episode_number":2,"season_number":2,"name":"Coma","runtime":47},
    {"episode_number":3,"season_number":2,"name":"The Man Behind Glass","runtime":47},
    {"episode_number":4,"season_number":2,"name":"Laura's Secret Diary","runtime":47},
    {"episode_number":5,"season_number":2,"name":"The Orchid's Curse","runtime":47}]}"#;

/// TMDb's answers for the two discs; anything else isn't there.
pub struct Fixtures;

#[async_trait]
impl tmdb::Source for Fixtures {
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Option<String>> {
        let q = query.iter().find(|(k, _)| *k == "query").map(|(_, v)| *v).unwrap_or("");
        let none = r#"{"page":1,"results":[]}"#;
        Ok(Some(
            match (path, q) {
                ("search/movie", "blade runner") => MOVIE_SEARCH,
                ("search/tv", "the sopranos") => TV_SEARCH,
                ("search/tv", "twin peaks") => TWIN_PEAKS_SEARCH,
                ("tv/1920", _) => r#"{"id":1920,"number_of_seasons":2}"#,
                ("tv/1920/season/1", _) => TWIN_PEAKS_SEASON_1,
                ("tv/1920/season/2", _) => TWIN_PEAKS_SEASON_2,
                ("search/movie" | "search/tv", _) => none,
                ("movie/78", _) => r#"{"id":78,"runtime":117}"#,
                ("tv/1398/season/3", _) => SEASON_3,
                _ => return Ok(None),
            }
            .to_owned(),
        ))
    }
}

struct VideoDisc(SimDisc);

#[async_trait]
impl Step for VideoDisc {
    fn info(&self) -> StepInfo {
        super::video_disc_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(disc) = inlet.recv().await {
            cx.status("", "Listing videos");
            cx.sleep(1500).await?;
            let info = makemkv::parse_info(match self.0 {
                SimDisc::Film => FILM,
                SimDisc::Season => SEASON,
                SimDisc::SeasonDisc2 => SEASON_DISC_2,
                SimDisc::BoxSet => BOX_SET,
            });
            let minlength = cx.param("min_length").and_then(|v| v.as_u64()).unwrap_or(60) as u32;
            let mut d = disc.becomes(ItemKind::VideoDisc, None);
            let line = super::record(&cx, d.context_mut(), &info.disc, minlength);
            cx.status(line.0, line.1);
            d.label = info.disc.label.clone();
            outlet.send(d).await;
        }
        Ok(())
    }
}

struct Rip;

#[async_trait]
impl Step for Rip {
    fn info(&self) -> StepInfo {
        super::rip_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let Some(mut disc) = inlet.recv().await else { return Ok(()) };
        let n = super::to_rip(disc.context()).len();
        disc.context_mut().add(Subject::Disc, facts::SELECTED, n.to_string(), "Rip");
        let items = rip_items(&disc);
        for i in &items {
            cx.item(i, ItemState::Waiting, "");
        }
        let _drive = cx.lane(LaneKind::Drive).await;
        for (n, item) in items.iter().enumerate() {
            let part = item.index.unwrap_or(0);
            cx.status(format!("{n} of {}", super::titles_word(items.len())), item.label.clone());
            cx.drive_says(StepState::Busy, "Reading", &format!("video {part}"));
            for pct in [25, 50, 75, 100] {
                cx.item(item, ItemState::Busy, format!("saving to MKV file {pct}%"));
                cx.progress(Some((n as f32 + pct as f32 / 100.0) / items.len() as f32));
                cx.sleep(600).await?;
            }
            let secs = item.context().get(Subject::Part(part), facts::LENGTH).and_then(|v| v.parse().ok()).unwrap_or(0);
            cx.item(item, ItemState::Ok, format!("✓ {}", super::hms(secs)));
            outlet.send(item.becomes(ItemKind::Title, Some(PathBuf::from(format!("title-{part:02}.mkv"))))).await;
        }
        cx.drive_says(StepState::Idle, "Waiting", "");
        cx.progress(None);
        cx.status(super::titles_word(items.len()), "");
        Ok(())
    }
}

struct Make {
    video: bool,
}

#[async_trait]
impl Step for Make {
    fn info(&self) -> StepInfo {
        if self.video {
            super::transcode::transcode_info()
        } else {
            super::transcode::remux_info()
        }
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let mut done = 0;
        let mut in_bytes = 0u64;
        while let Some(title) = inlet.recv().await {
            let _core = cx.lane(LaneKind::Encode).await;
            cx.item(&title, ItemState::Busy, "encoding");
            cx.status(super::transcode::made_size(&cx, in_bytes), title.label.clone());
            cx.sleep(if self.video { 3000 } else { 400 }).await?;
            done += 1;
            cx.item(&title, ItemState::Ok, "");
            let part = title.index.unwrap_or(0);
            // As big as the real one makes it, roughly: a transcode a
            // quarter or so, a remux the same.
            let bytes: u64 = title.context().get(Subject::Part(part), facts::BYTES).and_then(|b| b.parse().ok()).unwrap_or(0);
            in_bytes += bytes;
            let path = PathBuf::from(format!("title-{part:02}.mkv"));
            cx.made_sized(&path, if self.video { bytes * 28 / 100 } else { bytes });
            let _ = done;
            super::transcode::made_says(&cx, in_bytes, String::new());
            outlet.send(title.becomes(ItemKind::VideoFile, Some(path))).await;
        }
        Ok(())
    }
}
