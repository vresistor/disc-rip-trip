//! Discogs: releases by barcode, else by catalogue number, else by artist
//! and album from CD-Text, each read for its tracklist.
//!
//! Discogs' search needs a personal access token (discogs.com, Settings,
//! Developers). It's an app-wide setting, entered once for every recipe;
//! without one the step says "Not set up" and the disc goes on, so it
//! doesn't stop a recipe from starting. The token goes in a header, never
//! in a URL or the log.
//!
//! Written against Discogs' documented API (database/search and releases);
//! no live call has been made from here, so the parsers are tolerant: any
//! field may be missing.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;
use trip_engine::setting::ParamKind;
use trip_engine::{Candidate, Cx, Fact, Group, Inlet, ItemKind, LaneKind, Outlet, ParamSpec, Step, StepInfo, Subject};

const API: &str = "https://api.discogs.com";
const SOURCE: &str = "Discogs";
/// How many search results are read for their tracklists.
const READ: usize = 3;

pub fn info() -> StepInfo {
    StepInfo::new("discogs", "Discogs", Group::LookUp, ItemKind::AudioDisc, ItemKind::AudioDisc)
        .gives(&["candidates"])
        .unless(&["release"])
        .right_after(&["audio-cd"])
        .from("record collectors' database")
        .param(ParamSpec::new("token", "Personal access token", ParamKind::Text).app_wide().default("").help("Free from discogs.com, under Settings, Developers."))
}

/// What to search by, best first.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Barcode(String),
    CatNo(String),
    Text { artist: String, album: String },
}

impl Key {
    pub fn query(&self) -> Vec<(&'static str, String)> {
        let mut q = vec![("type", "release".to_owned())];
        match self {
            Key::Barcode(b) => q.push(("barcode", b.clone())),
            Key::CatNo(c) => q.push(("catno", c.clone())),
            Key::Text { artist, album } => {
                q.push(("artist", artist.clone()));
                q.push(("release_title", album.clone()));
            }
        }
        q
    }

    fn how(&self) -> &'static str {
        match self {
            Key::Barcode(_) => "by barcode",
            Key::CatNo(_) => "by catalogue number",
            Key::Text { .. } => "by artist and album",
        }
    }

    /// How much a match by this key says.
    fn score(&self) -> f32 {
        match self {
            Key::Barcode(_) => 0.85,
            Key::CatNo(_) => 0.75,
            Key::Text { .. } => 0.6,
        }
    }
}

/// What the disc and the lookups so far give to search by. A barcode the
/// disc carries (its MCN) is all zeros when unset, which isn't one.
pub fn keys(barcode: Option<&str>, catnos: &[String], artist: Option<&str>, album: Option<&str>) -> Vec<Key> {
    let mut k = vec![];
    if let Some(b) = barcode.map(str::trim).filter(|b| !b.is_empty() && b.chars().any(|c| c != '0')) {
        k.push(Key::Barcode(b.to_owned()));
    }
    k.extend(catnos.iter().filter(|c| !c.trim().is_empty()).map(|c| Key::CatNo(c.trim().to_owned())));
    if let (Some(a), Some(b)) = (artist.filter(|a| !a.is_empty()), album.filter(|a| !a.is_empty())) {
        k.push(Key::Text { artist: a.into(), album: b.into() });
    }
    k
}

#[derive(Debug, Default, Deserialize)]
pub struct Search {
    #[serde(default)]
    pub results: Vec<Hit>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Hit {
    pub id: u64,
}

#[derive(Debug, Default, Deserialize)]
pub struct Release {
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub identifiers: Vec<Identifier>,
    #[serde(default)]
    pub formats: Vec<Format>,
    #[serde(default)]
    pub tracklist: Vec<Track>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Artist {
    #[serde(default)]
    pub name: String,
    /// The name as credited on this release, when it differs.
    #[serde(default)]
    pub anv: String,
    #[serde(default)]
    pub join: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Label {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub catno: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Identifier {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Format {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Track {
    #[serde(default, rename = "type_")]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artists: Vec<Artist>,
}

/// "The White Stripes (2)" → "The White Stripes": Discogs numbers artists
/// who share a name.
pub fn plain_name(name: &str) -> String {
    let n = name.trim();
    match n.rsplit_once(" (") {
        Some((base, num)) if num.strip_suffix(')').is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())) => base.to_owned(),
        _ => n.to_owned(),
    }
}

fn credit(a: &[Artist]) -> String {
    let mut s = String::new();
    for x in a {
        s.push_str(&plain_name(if x.anv.is_empty() { &x.name } else { &x.anv }));
        let j = x.join.trim();
        match j {
            "" => s.push(' '),
            "," => s.push_str(", "),
            j => s.push_str(&format!(" {j} ")),
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches(',').to_owned()
}

/// A release as a candidate. Only real tracks count (headings and index
/// entries are in the tracklist too); the score drops when the number of
/// tracks isn't the disc's.
pub fn candidate(r: &Release, key: &Key, parts: u32) -> Candidate {
    let artist = credit(&r.artists);
    let tracks: Vec<&Track> = r.tracklist.iter().filter(|t| t.kind.is_empty() || t.kind == "track").collect();
    let label = r.labels.first().map(|l| plain_name(&l.name)).unwrap_or_default();
    let catno = r.labels.first().map(|l| l.catno.clone()).filter(|c| c != "none").unwrap_or_default();
    let barcode = r.identifiers.iter().find(|i| i.kind == "Barcode").map(|i| i.value.chars().filter(char::is_ascii_digit).collect::<String>()).unwrap_or_default();
    let year = r.year.filter(|y| *y > 0).map(|y| y.to_string()).unwrap_or_default();
    let mut facts = vec![];
    let mut put = |subject, key: &str, value: &str| {
        if !value.trim().is_empty() {
            facts.push(Fact { step: String::new(), subject, key: key.into(), value: value.trim().into(), source: SOURCE.into() });
        }
    };
    put(Subject::Disc, "album", &r.title);
    put(Subject::Disc, "album_artist", &artist);
    put(Subject::Disc, "year", &year);
    put(Subject::Disc, "country", r.country.as_deref().unwrap_or(""));
    put(Subject::Disc, "label", &label);
    put(Subject::Disc, "catalognumber", &catno);
    put(Subject::Disc, "barcode", &barcode);
    put(Subject::Disc, "discogs_releaseid", &r.id.to_string());
    for (i, t) in tracks.iter().enumerate() {
        put(Subject::Part(i as u32 + 1), "title", &t.title);
        if !t.artists.is_empty() {
            put(Subject::Part(i as u32 + 1), "artist", &credit(&t.artists));
        }
    }
    let format = r.formats.first().map(|f| f.name.clone()).unwrap_or_default();
    let detail = [year.as_str(), r.country.as_deref().unwrap_or(""), &label, &catno, &format].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
    let fits = parts == 0 || tracks.len() as u32 == parts;
    Candidate {
        step: String::new(),
        id: format!("discogs-{}", r.id),
        source: SOURCE.into(),
        title: format!("{artist} — {}", r.title),
        detail,
        score: key.score() - if fits { 0.0 } else { 0.25 },
        facts,
    }
}

async fn fetch<T: for<'de> Deserialize<'de>>(path: &str, query: &[(&str, String)], token: &str) -> Result<T> {
    // The token is a header, which reqwest drops on a redirect to another host.
    let r = super::net::client(Duration::from_secs(20))?.get(format!("{API}{path}")).query(query).header("Authorization", format!("Discogs token={token}")).send().await.map_err(super::net::quiet)?;
    match r.status().as_u16() {
        200 => super::net::json(r, super::net::JSON_MAX).await,
        401 => Err(anyhow!("Discogs refused the token")),
        429 => Err(anyhow!("Discogs asked us to slow down")),
        s => Err(anyhow!("Discogs said {s}")),
    }
}

/// Candidates by the first key that finds anything. One request at a time.
pub async fn lookup(keys: &[Key], parts: u32, token: &str) -> Result<(Vec<Candidate>, &'static str)> {
    for key in keys {
        let s: Search = fetch("/database/search", &key.query(), token).await?;
        if s.results.is_empty() {
            continue;
        }
        let mut out = vec![];
        for hit in s.results.iter().take(READ) {
            let r: Release = fetch(&format!("/releases/{}", hit.id), &[], token).await?;
            out.push(candidate(&r, key, parts));
        }
        return Ok((out, key.how()));
    }
    Ok((vec![], ""))
}

pub struct Discogs;

#[async_trait]
impl Step for Discogs {
    fn info(&self) -> StepInfo {
        info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let (k, parts) = {
                let c = disc.context();
                // Catalogue numbers offered by a lookup the disc has been
                // through before this one (none, on a branch beside them).
                let catnos: Vec<String> = c.candidates().iter().flat_map(|x| x.facts.iter()).filter(|f| f.key == "catalognumber").map(|f| f.value.clone()).collect();
                (keys(c.get(Subject::Disc, "barcode"), &catnos, c.get(Subject::Disc, "cdtext.artist"), c.get(Subject::Disc, "cdtext.album")), c.parts())
            };
            let token = Some(cx.text("token").trim().to_owned()).filter(|t| !t.is_empty());
            match (token, k.is_empty()) {
                (None, _) => cx.status("Not set up", "Needs a token"),
                (_, true) => cx.status("Nothing found", "nothing to search by"),
                (Some(token), false) => {
                    cx.status("", "Asking");
                    let found = {
                        let _net = cx.lane(LaneKind::Network(SOURCE.into())).await;
                        lookup(&k, parts, &token).await
                    };
                    match found {
                        Ok((c, _)) if c.is_empty() => cx.status("Nothing found", ""),
                        Ok((c, how)) => {
                            let _ = how;
                            cx.status(format!("{} found", c.len()), "");
                            disc.context_mut().offer(c);
                        }
                        Err(e) => {
                            // Like any lookup: the disc goes on without it.
                            super::lookup_skipped(&cx, &disc, e.to_string());
                            cx.log(format!("lookup failed: {e}"));
                        }
                    }
                }
            }
            outlet.send(disc).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release in the shape Discogs documents (abridged), not fetched.
    const RELEASE: &str = r#"{
      "id": 1105830, "title": "The White Stripes", "year": 1999, "country": "US",
      "artists": [{"name": "The White Stripes", "anv": "", "join": "", "id": 11515}],
      "labels": [{"name": "Sympathy For The Record Industry", "catno": "SFTRI 577", "id": 1}],
      "identifiers": [{"type": "Barcode", "value": "7 90276 05772 9"}, {"type": "Matrix / Runout", "value": "x"}],
      "formats": [{"name": "CD", "qty": "1", "descriptions": ["Album"]}],
      "tracklist": [
        {"position": "", "type_": "heading", "title": "Side One"},
        {"position": "1", "type_": "track", "title": "Jimmy The Exploder", "duration": "2:30"},
        {"position": "2", "type_": "track", "title": "Stop Breaking Down", "duration": "2:20",
         "artists": [{"name": "Robert Johnson (2)", "anv": "", "join": "&"}, {"name": "The White Stripes", "anv": "", "join": ""}]}
      ]
    }"#;

    #[test]
    fn a_release_becomes_a_candidate() {
        let r: Release = serde_json::from_str(RELEASE).unwrap();
        let c = candidate(&r, &Key::Barcode("790276057729".into()), 2);
        assert_eq!(c.title, "The White Stripes — The White Stripes");
        assert_eq!(c.detail, "1999, US, Sympathy For The Record Industry, SFTRI 577, CD");
        assert_eq!(c.score, 0.85);
        let fact = |s, k: &str| c.facts.iter().find(|f| f.subject == s && f.key == k).map(|f| f.value.as_str());
        assert_eq!(fact(Subject::Part(1), "title"), Some("Jimmy The Exploder"), "the heading isn't a track");
        assert_eq!(fact(Subject::Part(2), "artist"), Some("Robert Johnson & The White Stripes"));
        assert_eq!(fact(Subject::Disc, "barcode"), Some("790276057729"));
        // Seventeen tracks on the disc, two in this release: less likely.
        assert!(candidate(&r, &Key::Barcode("x".into()), 17).score < 0.85);
    }

    #[test]
    fn search_results_and_missing_fields() {
        let s: Search = serde_json::from_str(r#"{"pagination": {"items": 1}, "results": [{"id": 1105830, "type": "release", "title": "The White Stripes - The White Stripes"}]}"#).unwrap();
        assert_eq!(s.results[0].id, 1105830);
        let bare: Release = serde_json::from_str(r#"{"id": 5}"#).unwrap();
        assert_eq!(candidate(&bare, &Key::CatNo("A".into()), 0).title, " — ");
    }

    #[test]
    fn what_to_search_by() {
        assert_eq!(keys(Some("0000000000000"), &[], None, None), [], "an unset MCN");
        let k = keys(Some("790276057729"), &["SFTRI 577".into()], Some("The White Stripes"), Some("The White Stripes"));
        assert_eq!(k.len(), 3);
        assert_eq!(k[0].query(), [("type", "release".to_owned()), ("barcode", "790276057729".to_owned())]);
        assert!(matches!(k[2], Key::Text { .. }));
        assert_eq!(plain_name("Robert Johnson (2)"), "Robert Johnson");
        assert_eq!(plain_name("Sunn O)))"), "Sunn O)))");
    }
}
