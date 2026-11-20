//! MusicBrainz, by disc ID with the table of contents as a fallback.
//!
//! MusicBrainz allows one request a second per address and blocks rather
//! than slows down, so every request goes through one process-wide throttle
//! and carries a descriptive User-Agent.

use anyhow::{anyhow, Result};
use drt_engine::{Candidate, Fact, Subject};
use serde::Deserialize;
use std::time::Duration;

pub const API: &str = "https://musicbrainz.org/ws/2";
pub const AGENT: &str = super::net::AGENT;
const SOURCE: &str = "MusicBrainz";

/// Releases with this disc's exact disc ID, or failing that, its track layout.
pub async fn lookup(disc_id: &str, toc: &str) -> Result<Vec<Candidate>> {
    let url = format!("{API}/discid/{disc_id}?toc={}&inc=artist-credits+recordings+labels+release-groups&fmt=json", toc.replace(' ', "+"));
    let body: DiscIdResponse = match fetch(&url).await {
        Ok(b) => b,
        // Nobody has submitted this disc and nothing has its layout.
        Err(e) if e.to_string().contains("404") => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let tracks = toc.split_whitespace().nth(1).and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
    let mut out: Vec<(bool, bool, u32, Candidate)> = body.releases.into_iter().map(|r| candidate(r, disc_id, tracks)).collect();
    // Official before bootleg, CD before anything else with the same layout,
    // earliest first: usually the original pressing.
    out.sort_by_key(|(official, cd, year, c)| (!official, !cd, *year, c.id.clone()));
    Ok(out.into_iter().map(|(_, _, _, c)| c).collect())
}

fn candidate(r: Release, disc_id: &str, tracks: u32) -> (bool, bool, u32, Candidate) {
    let medium = r.media.iter().find(|m| m.discs.iter().any(|d| d.id == disc_id)).or_else(|| r.media.iter().find(|m| m.track_count == tracks)).or_else(|| r.media.first());
    let exact = r.media.iter().any(|m| m.discs.iter().any(|d| d.id == disc_id));
    let year = r.date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()).unwrap_or(9999u32);
    let label = r.label_info.iter().find_map(|l| l.label.as_ref().map(|x| x.name.clone())).unwrap_or_default();
    let catno = r.label_info.iter().find_map(|l| l.catalog_number.clone()).unwrap_or_default();
    let artist = credit(&r.artist_credit);

    let mut facts = Vec::new();
    let mut put = |subject, key: &str, value: String| {
        if !value.trim().is_empty() {
            facts.push(Fact { step: String::new(), subject, key: key.into(), value, source: SOURCE.into() });
        }
    };
    put(Subject::Disc, "album", r.title.clone());
    put(Subject::Disc, "album_artist", artist.clone());
    put(Subject::Disc, "date", r.date.clone().unwrap_or_default());
    put(Subject::Disc, "year", if year == 9999 { String::new() } else { year.to_string() });
    put(Subject::Disc, "country", r.country.clone().unwrap_or_default());
    put(Subject::Disc, "label", label.clone());
    put(Subject::Disc, "catalognumber", catno.clone());
    put(Subject::Disc, "barcode", r.barcode.clone().unwrap_or_default());
    put(Subject::Disc, "musicbrainz_albumid", r.id.clone());
    if let Some(rg) = &r.release_group {
        put(Subject::Disc, "musicbrainz_releasegroupid", rg.id.clone());
    }
    if let Some(m) = medium {
        // Which disc of the release this is, for DISCNUMBER/DISCTOTAL.
        if let Some(p) = m.position {
            put(Subject::Disc, "discnumber", p.to_string());
            put(Subject::Disc, "disctotal", r.media.len().to_string());
        }
        for t in &m.tracks {
            put(Subject::Part(t.position), "title", t.title.clone());
            put(Subject::Part(t.position), "artist", credit(&t.artist_credit));
            // The track on this release, and the recording it is: Picard
            // keeps both, and calls the recording's MUSICBRAINZ_TRACKID.
            put(Subject::Part(t.position), "musicbrainz_releasetrackid", t.id.clone());
            if let Some(r) = &t.recording {
                put(Subject::Part(t.position), "musicbrainz_recordingid", r.id.clone());
            }
        }
    }
    let detail =
        [(year != 9999).then(|| year.to_string()), r.country.clone(), (!label.is_empty()).then_some(label), (!catno.is_empty()).then_some(catno), r.disambiguation.clone().filter(|d| !d.is_empty())]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
    let official = r.status.as_deref() == Some("Official");
    let cd = medium.and_then(|m| m.format.as_deref()) == Some("CD");
    let c = Candidate {
        step: String::new(),
        id: r.id,
        source: SOURCE.into(),
        title: format!("{} — {}", artist, r.title),
        detail,
        // An exact disc-ID match is the strongest evidence there is; a
        // layout match only says the tracks are the same lengths.
        score: if exact { 0.9 } else { 0.6 },
        facts,
    };
    (official, cd, year, c)
}

fn credit(c: &[ArtistCredit]) -> String {
    c.iter().map(|a| format!("{}{}", a.name, a.joinphrase.as_deref().unwrap_or(""))).collect::<String>().trim().to_owned()
}

async fn fetch<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T> {
    Ok(serde_json::from_slice(&fetch_bytes(url).await?)?)
}

/// A MusicBrainz answer, as it came: through the throttle, asking again
/// while it's busy.
pub async fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    let client = super::net::client(Duration::from_secs(20))?;
    // MusicBrainz answers 503 when it's busy or we've asked too quickly, and
    // asks for another try a little later: a few, waiting longer each time
    // (or as long as it says), before giving up.
    let mut tries = 0;
    loop {
        throttle().await;
        let r = client.get(url).send().await?;
        let status = r.status().as_u16();
        if r.status().is_success() {
            return super::net::body(r, super::net::JSON_MAX).await;
        }
        tries += 1;
        if !matches!(status, 429 | 503) || tries >= BUSY_TRIES {
            return Err(anyhow!("MusicBrainz said {status}"));
        }
        let said = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
        tokio::time::sleep(busy_wait(tries, said)).await;
    }
}

/// How many times to ask a busy MusicBrainz before giving up.
const BUSY_TRIES: u32 = 5;

/// How long to wait before asking again: what it said, up to 15 s, else
/// 1, 2, 4, 8 s.
fn busy_wait(tries: u32, said: Option<u64>) -> Duration {
    match said {
        Some(s) => Duration::from_secs(s.min(15)),
        None => Duration::from_secs(1 << (tries - 1).min(3)),
    }
}

/// One request a second, across the whole process.
async fn throttle() {
    use std::sync::OnceLock;
    use tokio::sync::Mutex;
    use tokio::time::Instant;
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let mut last = LAST.get_or_init(|| Mutex::new(None)).lock().await;
    if let Some(prev) = *last {
        let since = prev.elapsed();
        if since < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_secs(1) - since).await;
        }
    }
    *last = Some(Instant::now());
}

/// The Cover Art Archive's name for a size: it keeps 250, 500 and 1200 px
/// copies of every front as well as the original.
pub fn cover_suffix(size: &str) -> &'static str {
    match size {
        "Original" => "",
        "500 px" => "-500",
        // Only to show beside others, when you pick the cover.
        "250 px" => "-250",
        _ => "-1200",
    }
}

/// Downloads the release's front cover from the Cover Art Archive into
/// `dir`, at a size [`cover_suffix`] knows. Gives the file, named for what
/// it holds (an original can be a PNG), and its size in bytes.
pub async fn front_cover(mbid: &str, size: &str, dir: &std::path::Path) -> Result<Option<(std::path::PathBuf, u64)>> {
    let url = format!("https://coverartarchive.org/release/{mbid}/front{}", cover_suffix(size));
    let r = super::net::client(Duration::from_secs(60))?.get(&url).send().await?;
    if r.status().as_u16() == 404 {
        return Ok(None);
    }
    if !r.status().is_success() {
        return Err(anyhow!("Cover Art Archive said {}", r.status().as_u16()));
    }
    // Generous for a scanned original; small enough that a runaway response can't fill a disk.
    let bytes = super::net::body(r, super::net::FILE_MAX).await.map_err(|e| anyhow!("the cover: {e}"))?;
    // ffmpeg reads a picture by its extension, so name it for its bytes.
    let dest = dir.join(if bytes.starts_with(b"\x89PNG") { "cover.png" } else { "cover.jpg" });
    std::fs::write(&dest, &bytes)?;
    Ok(Some((dest, bytes.len() as u64)))
}

#[derive(Deserialize)]
struct DiscIdResponse {
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Deserialize)]
struct Release {
    id: String,
    title: String,
    status: Option<String>,
    date: Option<String>,
    country: Option<String>,
    barcode: Option<String>,
    disambiguation: Option<String>,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default, rename = "label-info")]
    label_info: Vec<LabelInfo>,
    #[serde(rename = "release-group")]
    release_group: Option<ReleaseGroup>,
    #[serde(default)]
    media: Vec<Medium>,
}

#[derive(Deserialize)]
struct ReleaseGroup {
    id: String,
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
struct Medium {
    format: Option<String>,
    position: Option<u32>,
    #[serde(default, rename = "track-count")]
    track_count: u32,
    #[serde(default)]
    discs: Vec<Disc>,
    #[serde(default)]
    tracks: Vec<Track>,
}

#[derive(Deserialize)]
struct Disc {
    id: String,
}

#[derive(Deserialize)]
struct Track {
    id: String,
    position: u32,
    recording: Option<Recording>,
    title: String,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
}

#[derive(Deserialize)]
struct Recording {
    id: String,
}

#[derive(Deserialize)]
struct ArtistCredit {
    name: String,
    joinphrase: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_gives_its_disc_number_and_both_track_ids() {
        let json = r#"{"id":"rel","title":"Two Discs","media":[
            {"position":1,"format":"CD","track-count":1,"discs":[],"tracks":[{"id":"t1","position":1,"title":"A","recording":{"id":"rec1"}}]},
            {"position":2,"format":"CD","track-count":2,"discs":[{"id":"DISC"}],"tracks":[{"id":"t2","position":1,"title":"B","recording":{"id":"rec2"}}]}]}"#;
        let r: Release = serde_json::from_str(json).unwrap();
        let (_, _, _, c) = candidate(r, "DISC", 2);
        let get = |s: Subject, k: &str| c.facts.iter().find(|f| f.subject == s && f.key == k).map(|f| f.value.as_str());
        assert_eq!((get(Subject::Disc, "discnumber"), get(Subject::Disc, "disctotal")), (Some("2"), Some("2")));
        assert_eq!(get(Subject::Part(1), "musicbrainz_releasetrackid"), Some("t2"));
        assert_eq!(get(Subject::Part(1), "musicbrainz_recordingid"), Some("rec2"));
        assert_eq!(get(Subject::Part(1), "musicbrainz_trackid"), None, "the name Picard gives the recording");
    }
}

#[cfg(test)]
mod busy {
    use super::*;

    #[test]
    fn a_busy_musicbrainz_is_asked_again_waiting_longer_each_time() {
        let waits: Vec<u64> = (1..BUSY_TRIES).map(|t| busy_wait(t, None).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8]);
        // As long as it says, within reason.
        assert_eq!(busy_wait(1, Some(3)), Duration::from_secs(3));
        assert_eq!(busy_wait(1, Some(600)), Duration::from_secs(15));
    }
}
