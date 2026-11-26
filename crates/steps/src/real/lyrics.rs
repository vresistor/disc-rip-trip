//! Lyrics from LRCLIB: synced (LRC) when it has them, plain otherwise, one
//! track at a time, into the context per part. The lyrics travel on the
//! tracks, and every encoder writes them as a tag when they're there.
//! *Lyrics file*, after a Folder, puts an `.lrc` beside each filed track
//! that has synced lyrics, which is what players that don't read the tag
//! look for.
//!
//! Polite to a free service: one request at a time through its own lane, an
//! identifying User-Agent (the app and its version, nothing about the
//! user), a short timeout, and it gives up after a few failures in a row
//! rather than holding the disc up.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;
use trip_engine::{Cx, Group, Inlet, ItemKind, ItemState, LaneKind, Outlet, Step, StepInfo, Subject};

const API: &str = "https://lrclib.net/api/get";
const SOURCE: &str = "LRCLIB";
/// Failures in a row (not "no lyrics", which is an answer) before giving up.
const GIVE_UP: u32 = 3;

pub fn info() -> StepInfo {
    StepInfo::new("lyrics", "Lyrics", Group::LookUp, ItemKind::AudioDisc, ItemKind::AudioDisc).needs(&["release"]).right_after(&["choose"]).gives(&["lyrics"]).from("words for each track, from LRCLIB")
}

/// Lyrics file: an `.lrc` beside each filed track that has synced lyrics.
/// It needs the files filed, and Lyrics on the way, and has no settings.
pub fn file_info() -> StepInfo {
    StepInfo::new("lyrics-file", "Lyrics file", Group::Make, ItemKind::AudioFile, ItemKind::TextFile).needs(&["filed"]).needs(&["lyrics"]).from("a lyrics file beside each track")
}

/// The detail for tracks without lyrics: *none for 3, 10, 15*.
pub fn none_for(tracks: &[u32]) -> String {
    match tracks {
        [] => String::new(),
        t => format!("none for {}", t.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")),
    }
}

/// What LRCLIB says about one track.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    #[serde(default)]
    pub instrumental: bool,
    pub plain_lyrics: Option<String>,
    pub synced_lyrics: Option<String>,
}

/// The lyrics to keep, and whether they're synced.
#[derive(Clone, Debug, PartialEq)]
pub enum Lyrics {
    Synced(String),
    Plain(String),
    Instrumental,
    None,
}

pub fn choose(f: Option<Found>) -> Lyrics {
    let some = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
    match f {
        None => Lyrics::None,
        Some(f) if f.instrumental => Lyrics::Instrumental,
        Some(f) => match (some(f.synced_lyrics), some(f.plain_lyrics)) {
            (Some(s), _) => Lyrics::Synced(s),
            (None, Some(p)) => Lyrics::Plain(p),
            _ => Lyrics::None,
        },
    }
}

/// The query for one track: LRCLIB matches on all four, the duration
/// within a couple of seconds.
pub fn query(artist: &str, title: &str, album: &str, seconds: u32) -> Vec<(&'static str, String)> {
    vec![("artist_name", artist.into()), ("track_name", title.into()), ("album_name", album.into()), ("duration", seconds.to_string())]
}

/// One track's lyrics. `Ok(None)` is LRCLIB not having them.
pub async fn get(artist: &str, title: &str, album: &str, seconds: u32) -> Result<Option<Found>> {
    let r = super::net::client(Duration::from_secs(10))?.get(API).query(&query(artist, title, album, seconds)).send().await?;
    match r.status().as_u16() {
        404 => Ok(None),
        200 => Ok(Some(super::net::json(r, super::net::TEXT_MAX).await?)),
        s => Err(anyhow!("LRCLIB said {s}")),
    }
}

/// An `.lrc` file's text: synced lyrics as they are, with the tags LRC
/// players read first.
pub fn lrc(title: &str, artist: &str, album: &str, synced: &str) -> String {
    let mut s = String::new();
    for (k, v) in [("ti", title), ("ar", artist), ("al", album)] {
        if !v.is_empty() {
            // One line each: a title can't add lines of its own.
            s.push_str(&format!("[{k}:{}]\n", v.replace(['\r', '\n'], " ")));
        }
    }
    s.push_str(synced.trim_end());
    s.push('\n');
    s
}

/// The `.lrc` for a track filed at `dest`, when Lyrics found synced
/// lyrics for it: `01 - Title.flac` gets `01 - Title.lrc`.
pub fn lrc_file(c: &trip_engine::Context, t: u32, dest: &std::path::Path) -> Option<(std::path::PathBuf, String)> {
    if c.get(Subject::Part(t), "lyrics.synced") != Some("yes") {
        return None;
    }
    let text = c.get(Subject::Part(t), "lyrics")?;
    let artist = c.get(Subject::Part(t), "artist").or(c.get(Subject::Disc, "album_artist")).unwrap_or("");
    let body = lrc(c.get(Subject::Part(t), "title").unwrap_or(""), artist, c.get(Subject::Disc, "album").unwrap_or(""), text);
    Some((dest.with_extension("lrc"), body))
}

/// Writes the `.lrc` files: see [`file_info`].
pub struct LyricsFile;

#[async_trait]
impl Step for LyricsFile {
    fn info(&self) -> StepInfo {
        file_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let (mut written, mut none) = (0, 0);
        while let Some(f) = inlet.recv().await {
            let (Some(t), Some(dest)) = (f.index, f.path.clone()) else { continue };
            match lrc_file(f.context(), t, &dest) {
                Some((lrc, text)) => match super::place::write(&lrc, text) {
                    Ok(()) => {
                        written += 1;
                        cx.made(&lrc);
                        let name = lrc.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        cx.item(&f, ItemState::Ok, name);
                        outlet.send(f.becomes(ItemKind::TextFile, Some(lrc))).await;
                    }
                    Err(e) => {
                        cx.item(&f, ItemState::Bad, e.to_string());
                        cx.log(format!("couldn't write {}: {e}", lrc.display()));
                    }
                },
                None => {
                    none += 1;
                    cx.item(&f, ItemState::Skipped, "no synced lyrics");
                }
            }
            let _ = none;
            cx.status(format!("{written} .lrc file{}", if written == 1 { "" } else { "s" }), "");
        }
        Ok(())
    }
}

pub struct Lyric;

#[async_trait]
impl Step for Lyric {
    fn info(&self) -> StepInfo {
        info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let n = disc.context().parts();
            let (mut found, mut synced, mut failures, mut last_error) = (0, 0, 0, String::new());
            // Each track, as an item: synced, plain, or none (left out).
            let mut missing: Vec<u32> = vec![];
            let track = |disc: &trip_engine::Item, t: u32| trip_engine::Item::track(t, disc.context().get(Subject::Part(t), "title").unwrap_or("").to_owned());
            for t in 1..=n {
                // The release Choose settled, on the disc.
                let (artist, title, album, seconds) = {
                    let c = disc.context();
                    let artist = c.get(Subject::Part(t), "artist").or(c.get(Subject::Disc, "album_artist")).unwrap_or("").to_owned();
                    let title = c.get(Subject::Part(t), "title").unwrap_or("").to_owned();
                    (artist, title, c.get(Subject::Disc, "album").unwrap_or("").to_owned(), c.get(Subject::Part(t), "length").and_then(|l| l.parse().ok()).unwrap_or(0))
                };
                if title.is_empty() || artist.is_empty() {
                    missing.push(t);
                    cx.item(&track(&disc, t), ItemState::Skipped, "no title to ask with");
                    continue;
                }
                cx.item(&track(&disc, t), ItemState::Busy, "asking");
                cx.status(format!("{synced}/{n} synced"), super::track_line(t, &title));
                cx.progress(Some(t as f32 / n as f32));
                let answer = {
                    let _net = cx.lane(LaneKind::Network(SOURCE.into())).await;
                    tokio::select! {
                        a = get(&artist, &title, &album, seconds) => a,
                        _ = cx.cancelled() => return Ok(()),
                    }
                };
                match answer {
                    Ok(f) => {
                        failures = 0;
                        let (text, is_synced) = match choose(f) {
                            Lyrics::Synced(s) => (s, true),
                            Lyrics::Plain(p) => (p, false),
                            Lyrics::Instrumental | Lyrics::None => {
                                missing.push(t);
                                cx.item(&track(&disc, t), ItemState::Skipped, "none");
                                continue;
                            }
                        };
                        cx.item(&track(&disc, t), ItemState::Ok, if is_synced { "synced" } else { "plain" });
                        found += 1;
                        synced += is_synced as u32;
                        let c = disc.context_mut();
                        c.add(Subject::Part(t), "lyrics", text, SOURCE);
                        c.add(Subject::Part(t), "lyrics.synced", if is_synced { "yes" } else { "no" }, SOURCE);
                    }
                    Err(e) => {
                        failures += 1;
                        last_error = e.to_string();
                        missing.push(t);
                        cx.item(&track(&disc, t), ItemState::Skipped, "no answer");
                        cx.log(format!("track {t}: {e}"));
                        if failures >= GIVE_UP {
                            break;
                        }
                    }
                }
            }
            cx.progress(None);
            if failures >= GIVE_UP && found == 0 {
                // Like any lookup: the disc goes on without it.
                super::lookup_skipped(&cx, &disc, last_error);
            } else if found == 0 {
                cx.status("Nothing found", "LRCLIB has none");
            } else {
                let plain = found - synced;
                cx.outcome(format!("{synced}/{n} synced"), if plain > 0 { format!("{plain} plain") } else { String::new() }, none_for(&missing));
            }
            outlet.send(disc).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LRCLIB's answer for "Jimmy the Exploder" (The White Stripes, 1999),
    /// 2026-11-19, cut to two lines of each.
    const JIMMY: &str = r#"{"id":832125,"name":"Jimmy the Exploder","trackName":"Jimmy the Exploder","artistName":"The White Stripes","albumName":"The White Stripes","duration":149.0,"instrumental":false,"hasWordSync":true,"plainLyrics":"Now Jimmy\nWell do you want an explosion now","syncedLyrics":"[00:31.55]Now, Jimmy\n[00:34.23]Well, do you want an explosion now?","lyricsfile":"version: '1.0'"}"#;

    #[test]
    fn synced_lyrics_are_kept_over_plain() {
        let f: Found = serde_json::from_str(JIMMY).unwrap();
        assert_eq!(choose(Some(f.clone())), Lyrics::Synced("[00:31.55]Now, Jimmy\n[00:34.23]Well, do you want an explosion now?".into()));
        let plain = Found { synced_lyrics: None, ..f.clone() };
        assert!(matches!(choose(Some(plain)), Lyrics::Plain(_)));
        let empty = Found { synced_lyrics: Some("  ".into()), plain_lyrics: None, ..f };
        assert_eq!(choose(Some(empty)), Lyrics::None);
        assert_eq!(choose(Some(Found { instrumental: true, ..Default::default() })), Lyrics::Instrumental);
        assert_eq!(choose(None), Lyrics::None, "a 404");
    }

    #[test]
    fn the_query_and_an_lrc_file() {
        let q = query("The White Stripes", "Jimmy the Exploder", "The White Stripes", 149);
        assert_eq!(q[3], ("duration", "149".to_owned()));
        assert_eq!(lrc("Jimmy the Exploder", "The White Stripes", "", "[00:31.55]Now, Jimmy\n"), "[ti:Jimmy the Exploder]\n[ar:The White Stripes]\n[00:31.55]Now, Jimmy\n");
        assert_eq!(lrc("A\n[00:00.00]B", "", "", "[00:01.00]x\n"), "[ti:A [00:00.00]B]\n[00:01.00]x\n", "a title is one line");
    }

    #[test]
    fn lyrics_travel_on_the_tracks_to_the_tags_and_the_lrc_files() {
        let mut c = trip_engine::Context::default();
        c.add(Subject::Disc, "parts", "2", "test");
        c.add(Subject::Part(1), "title", "Jimmy the Exploder", "test");
        c.add(Subject::Part(1), "lyrics", "[00:31.55]Now, Jimmy", SOURCE);
        c.add(Subject::Part(1), "lyrics.synced", "yes", SOURCE);
        c.add(Subject::Part(2), "lyrics", "Plain words", SOURCE);
        // Every encoder writes what came, with no switch for it.
        assert!(super::super::encode::tags(&c, 1).contains(&("lyrics".into(), "[00:31.55]Now, Jimmy".into())));
        assert!(super::super::encode::tags(&c, 2).contains(&("lyrics".into(), "Plain words".into())));
        // An .lrc only for synced lyrics, named for the filed track.
        let (path, body) = lrc_file(&c, 1, std::path::Path::new("/m/01 - Jimmy the Exploder.flac")).unwrap();
        assert_eq!(path, std::path::Path::new("/m/01 - Jimmy the Exploder.lrc"));
        assert!(body.ends_with("[00:31.55]Now, Jimmy\n"), "{body}");
        assert!(lrc_file(&c, 2, std::path::Path::new("/m/02.flac")).is_none());
        assert!(info().params.is_empty() && file_info().params.is_empty());
    }
}
