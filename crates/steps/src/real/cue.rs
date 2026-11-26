//! Cue sheet and rip log: written beside the audio files a Folder filed,
//! once every one has arrived, from what they carry, so they describe the
//! disc as it was read and the files as they are named.
//!
//! It comes after a Folder (it needs `filed`) and takes the files that
//! Folder put there, so it has no settings: the names and the format are
//! the files' own. Each format's library that wants a cue sheet has its
//! own Cue after it.
//!
//! The cue sheet has one `FILE` per track, by its real name beside the
//! sheet, with the album's and each
//! track's title and performer, the ISRCs, and the disc's barcode as
//! `CATALOG`. The label's catalogue number has no cue command, so it's a
//! `REM`. Gaps between tracks aren't known, so each track starts at its
//! file's beginning (`INDEX 01 00:00:00`): the sheet describes the files,
//! not a burnable image.
//!
//! The rip log says what the drive, the offset and every read of every
//! track were, and each track's verdict, from what Rip recorded.

use crate::template;
use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use trip_engine::{Context, Cx, Group, Inlet, Item, ItemKind, ItemState, Outlet, Step, StepInfo, Subject};

pub fn info() -> StepInfo {
    StepInfo::new("cue", "Cue sheet", Group::Make, ItemKind::AudioFile, ItemKind::TextFile).needs(&["filed"]).from("track list and rip log, beside the files")
}

/// A file's name as the sheet beside it in `dir` refers to it.
pub fn file_ref(dir: &Path, file: &Path) -> String {
    file.strip_prefix(dir).unwrap_or(file).to_string_lossy().into_owned()
}

/// Text for inside a cue's quotes, which have no escape: a double quote
/// becomes a single one, and a line break a space.
pub fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "'").replace(['\r', '\n'], " ").trim())
}

/// A 13-digit EAN for `CATALOG`, from a barcode: a 12-digit UPC is the
/// same number with a leading zero. Anything else isn't one.
pub fn catalog(barcode: &str) -> Option<String> {
    let d: String = barcode.chars().filter(char::is_ascii_digit).collect();
    match d.len() {
        13 if d.chars().any(|c| c != '0') => Some(d),
        12 => Some(format!("0{d}")),
        _ => None,
    }
}

/// The cue sheet for `files`: each track and its file, as the sheet
/// refers to it.
pub fn cue_sheet(c: &Context, files: &[(u32, String)]) -> String {
    let get = |k| c.get(Subject::Disc, k).unwrap_or("").to_owned();
    let mut s = String::new();
    let mut line = |l: String| {
        s.push_str(&l);
        s.push_str("\r\n");
    };
    for (rem, key) in [("GENRE", "genre"), ("DATE", "year"), ("LABEL", "label"), ("CATALOGNUMBER", "catalognumber")] {
        let v = get(key);
        if !v.is_empty() {
            // A line break in the text (GnuDB turns `\n` into one) would
            // start a cue command of its own.
            let v: String = v.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_owned();
            line(format!("REM {rem} {}", if v.contains([' ', '"']) { quoted(&v) } else { v }));
        }
    }
    if let Some(toc) = c.get(Subject::Disc, "toc").and_then(super::gnudb::parse_toc) {
        line(format!("REM DISCID {:08X}", toc.disc_id()));
    }
    line(format!("REM COMMENT {}", quoted(concat!("Trip ", env!("CARGO_PKG_VERSION")))));
    if let Some(ean) = catalog(&get("barcode")) {
        line(format!("CATALOG {ean}"));
    }
    let album_artist = get("album_artist");
    if !album_artist.is_empty() {
        line(format!("PERFORMER {}", quoted(&album_artist)));
    }
    let album = get("album");
    if !album.is_empty() {
        line(format!("TITLE {}", quoted(&album)));
    }
    // The hidden track is track 1's pregap: its file holds track 1's
    // INDEX 00, and track 1's own file starts at INDEX 01. A cue sheet has
    // no track 0. Without track 1's file there's no INDEX 01 to go with it,
    // so it's left out.
    let has = |n: u32| files.iter().any(|(t, _)| *t == n);
    let hidden = has(0) && has(1);
    for (t, file) in files {
        let t = *t;
        if t == 0 && !hidden {
            continue;
        }
        // WAVE is any audio a player decodes; MP3 has its own word.
        let kind = if file.to_lowercase().ends_with(".mp3") { "MP3" } else { "WAVE" };
        line(format!("FILE {} {kind}", quoted(file)));
        if t == 1 && hidden {
            line("    INDEX 01 00:00:00".into());
            continue;
        }
        // Track 0's file opens track 1.
        let n = t.max(1);
        let f = super::fields_in(c, n, &format!("Track {n}"));
        line(format!("  TRACK {n:02} AUDIO"));
        // Pre-emphasis is kept as the disc has it; the flag tells a player
        // to undo it.
        if c.get(Subject::Part(n), "preemphasis") == Some("1") {
            line("    FLAGS PRE".into());
        }
        line(format!("    TITLE {}", quoted(&f.title)));
        if let Some(p) = c.get(Subject::Part(n), "artist").filter(|p| !p.is_empty()) {
            line(format!("    PERFORMER {}", quoted(p)));
        }
        if let Some(isrc) = c.get(Subject::Part(n), "isrc").filter(|i| i.len() == 12) {
            line(format!("    ISRC {isrc}"));
        }
        line(format!("    INDEX {} 00:00:00", if t == 0 { "00" } else { "01" }));
    }
    s
}

/// The rip log: the drive, the disc, and each track's reads and verdict.
pub fn rip_log(c: &Context, tracks: &[u32], date: &str) -> String {
    let get = |k| c.get(Subject::Disc, k).unwrap_or("").to_owned();
    let mut s = format!("Trip {} rip log, {date}\n\n", env!("CARGO_PKG_VERSION"));
    let mut kv = |k: &str, v: String| {
        if !v.is_empty() {
            s.push_str(&format!("{k:<20}{v}\n"));
        }
    };
    kv("Drive", get("drive"));
    kv("Read offset", get("offset").parse::<i64>().map(|o| format!("{o:+}")).unwrap_or_default());
    kv("Album", [get("album_artist"), get("album")].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" — "));
    kv("Year", get("year"));
    kv("MusicBrainz disc ID", get("disc_id"));
    kv("TOC", get("toc"));
    kv("CDDB ID", c.get(Subject::Disc, "toc").and_then(super::gnudb::parse_toc).map(|t| format!("{:08x}", t.disc_id())).unwrap_or_default());
    s.push('\n');
    let (mut good, mut accurate) = (0, 0);
    for &t in tracks {
        let title = if t == 0 { "hidden track (track 1's pregap)" } else { c.get(Subject::Part(t), "title").unwrap_or("") };
        let len = c.get(Subject::Part(t), "length").and_then(|l| l.parse::<u32>().ok()).map(|l| format!("  {}", super::mmss(l))).unwrap_or_default();
        let pre = if c.get(Subject::Part(t), "preemphasis") == Some("1") { "  (pre-emphasis, kept)" } else { "" };
        s.push_str(&format!("Track {t:2}  {title}{len}{pre}\n"));
        for r in c.all(Subject::Part(t), "read") {
            s.push_str(&format!("    {}\n", r.value));
        }
        let verdict = c.all(Subject::Part(t), "verdict").last().map(|f| f.value.clone()).unwrap_or_else(|| "no verdict recorded".into());
        if verdict.starts_with("ok") {
            good += 1;
        }
        // Verified means AccurateRip matched, as Rip counts it; agreeing
        // reads are kept, not verified.
        if verdict.strip_prefix("ok: ").is_some_and(super::is_accurate) {
            accurate += 1;
        }
        s.push_str(&format!("    verdict: {verdict}\n\n"));
    }
    s.push_str(&format!("{} track{}: {good} kept, {accurate} verified\n", tracks.len(), if tracks.len() == 1 { "" } else { "s" }));
    s
}

pub struct CueSheet;

#[async_trait]
impl Step for CueSheet {
    fn info(&self) -> StepInfo {
        info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let mut files: Vec<(u32, PathBuf)> = vec![];
        // What the tracks know, together: the disc's, and each one's reads.
        let mut known = Context::default();
        while let Some(f) = inlet.recv().await {
            if let (Some(t), Some(p)) = (f.index, f.path.clone()) {
                files.push((t, p));
            }
            known.merge(f.context());
            cx.status("", format!("Waits for the files, {} so far", files.len()));
        }
        if files.is_empty() || cx.stopped() {
            return Ok(());
        }
        files.sort_by_key(|(t, _)| *t);
        files.dedup_by_key(|(t, _)| *t);
        // Beside the first track; any filed elsewhere are named from there.
        let dir = files[0].1.parent().map(Path::to_path_buf).unwrap_or_default();
        let tracks: Vec<u32> = files.iter().map(|(t, _)| *t).collect();
        let refs: Vec<(u32, String)> = files.iter().map(|(t, p)| (*t, file_ref(&dir, p))).collect();
        let (sheet, log, stem) = {
            let c = &known;
            let album = c.get(Subject::Disc, "album").map(template::safe).filter(|a| !a.is_empty()).unwrap_or_else(|| "disc".into());
            (cue_sheet(c, &refs), rip_log(c, &tracks, &template::today()), album)
        };
        let known = std::sync::Arc::new(known);
        cx.expect(2);
        for (id, label, ext, text) in [("cue", "Cue sheet", "cue", sheet), ("log", "Rip log", "log", log)] {
            let path = dir.join(format!("{stem}.{ext}"));
            // Under a temporary name first, as Folder does, so a half-written
            // sheet never sits beside the files under its real name.
            super::place::write(&path, text)?;
            cx.made(&path);
            let mut item = Item::new(id, ItemKind::TextFile, label, None, Some(path));
            item.context = known.clone();
            cx.item(&item, ItemState::Ok, format!("{stem}.{ext}"));
            outlet.send(item).await;
        }
        cx.status(format!("{stem}.cue, {stem}.log"), "");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_break_in_a_genre_doesnt_start_a_cue_command() {
        let mut c = Context::default();
        c.add(Subject::Disc, "genre", "Rock\nFILE\t\"/etc/passwd\"\tWAVE", "GnuDB");
        let sheet = cue_sheet(&c, &[(1, "01.flac".into())]);
        assert_eq!(sheet.lines().filter(|l| l.trim_start().starts_with("FILE")).count(), 1, "{sheet}");
        assert!(sheet.lines().any(|l| l.starts_with("REM GENRE \"Rock FILE")), "{sheet}");
    }

    fn disc() -> Context {
        let mut c = Context::default();
        let d = "test";
        for (k, v) in [
            ("parts", "2"),
            ("album", "The White Stripes"),
            ("album_artist", "The White Stripes"),
            ("year", "1999"),
            ("label", "Sympathy for the Record Industry"),
            ("catalognumber", "SFTRI 577"),
            ("barcode", "790276057729"),
            ("drive", "MATSHITA BD-MLT UJ240AS"),
            ("offset", "103"),
            ("toc", "1 2 22000 150 11382"),
        ] {
            c.add(Subject::Disc, k, v, d);
        }
        c.add(Subject::Part(1), "title", "Jimmy the Exploder", d);
        c.add(Subject::Part(1), "length", "150", d);
        c.add(Subject::Part(1), "isrc", "USSYM9900001", d);
        c.add(Subject::Part(2), "title", "Say \"Stop\" Breaking Down", d);
        c.add(Subject::Part(2), "artist", "Robert Johnson & The White Stripes", d);
        c.add(Subject::Part(1), "read", "read 1, paranoia 0: Accurate { confidence: 12 }, CRC 1A2B3C4D", "Rip");
        c.add(Subject::Part(1), "verdict", "ok: ✓ 12", "Rip");
        c.add(Subject::Part(2), "read", "read 1, paranoia 0: Mismatch, CRC 11111111", "Rip");
        c.add(Subject::Part(2), "read", "read 2, paranoia 3: Mismatch, CRC 11111111", "Rip");
        c.add(Subject::Part(2), "verdict", "bad: reads disagree", "Rip");
        c.add(Subject::Part(2), "verdict", "ok: kept, unverified", "Rip");
        c
    }

    #[test]
    fn a_cue_sheet_names_the_files_as_they_were_filed() {
        let dir = Path::new("/music/The White Stripes/The White Stripes");
        let files = [(1, dir.join("01 - Jimmy the Exploder.flac")), (2, dir.join("02 - Say _Stop_ Breaking Down.flac"))];
        let refs: Vec<(u32, String)> = files.iter().map(|(t, p)| (*t, file_ref(dir, p))).collect();
        let s = cue_sheet(&disc(), &refs);
        let want = "REM DATE 1999\r\nREM LABEL \"Sympathy for the Record Industry\"\r\nREM CATALOGNUMBER \"SFTRI 577\"\r\nREM DISCID 09012302\r\nREM COMMENT \"Trip ";
        assert!(s.starts_with(want), "{s}");
        assert!(s.contains("CATALOG 0790276057729\r\nPERFORMER \"The White Stripes\"\r\nTITLE \"The White Stripes\"\r\n"), "{s}");
        assert!(s.contains("FILE \"01 - Jimmy the Exploder.flac\" WAVE\r\n  TRACK 01 AUDIO\r\n    TITLE \"Jimmy the Exploder\"\r\n    ISRC USSYM9900001\r\n    INDEX 01 00:00:00\r\n"), "{s}");
        assert!(
            s.contains("FILE \"02 - Say _Stop_ Breaking Down.flac\" WAVE\r\n  TRACK 02 AUDIO\r\n    TITLE \"Say 'Stop' Breaking Down\"\r\n    PERFORMER \"Robert Johnson & The White Stripes\"\r\n"),
            "{s}"
        );
    }

    #[test]
    fn an_mp3_is_called_one() {
        let s = cue_sheet(&disc(), &[(1, "01 - Jimmy the Exploder.mp3".into())]);
        assert!(s.contains("FILE \"01 - Jimmy the Exploder.mp3\" MP3\r\n"), "{s}");
        assert!(info().params.is_empty(), "the files say their names and format");
    }

    #[test]
    fn pre_emphasis_is_flagged_and_a_hidden_track_is_track_ones_index_00() {
        let mut c = disc();
        c.add(Subject::Part(1), "preemphasis", "1", "Rip");
        let files = [(0, "00 - Hidden track.flac".to_owned()), (1, "01 - Jimmy the Exploder.flac".to_owned()), (2, "02 - Say _Stop_ Breaking Down.flac".to_owned())];
        let s = cue_sheet(&c, &files);
        let want = "FILE \"00 - Hidden track.flac\" WAVE\r\n  TRACK 01 AUDIO\r\n    FLAGS PRE\r\n    TITLE \"Jimmy the Exploder\"\r\n    ISRC USSYM9900001\r\n    INDEX 00 00:00:00\r\nFILE \"01 - Jimmy the Exploder.flac\" WAVE\r\n    INDEX 01 00:00:00\r\nFILE \"02 - Say _Stop_ Breaking Down.flac\" WAVE\r\n  TRACK 02 AUDIO\r\n    TITLE";
        assert!(s.contains(want), "{s}");
        assert!(!s.contains("TRACK 00"), "{s}");
        assert_eq!(s.matches("FLAGS PRE").count(), 1, "only track 1 is flagged: {s}");
        // Without track 1's file there's nothing for its INDEX 00 to open.
        let s = cue_sheet(&c, &[(0, "00 - Hidden track.flac".into()), (2, "02.flac".into())]);
        assert!(!s.contains("Hidden track"), "{s}");
        let log = rip_log(&c, &[0, 1], "2026-11-20");
        assert!(log.contains("Track  0  hidden track (track 1's pregap)\n"), "{log}");
        assert!(log.contains("Track  1  Jimmy the Exploder  2:30  (pre-emphasis, kept)\n"), "{log}");
    }

    #[test]
    fn barcodes_become_catalog_numbers() {
        assert_eq!(catalog("790276057729").as_deref(), Some("0790276057729"));
        assert_eq!(catalog("5 033197 123456").as_deref(), Some("5033197123456"));
        assert_eq!(catalog("0000000000000"), None, "an unset MCN");
        assert_eq!(catalog("SFTRI 577"), None);
    }

    #[test]
    fn the_rip_log_has_every_read_and_the_last_verdict() {
        let log = rip_log(&disc(), &[1, 2], "2026-11-19");
        assert!(log.contains("Drive               MATSHITA BD-MLT UJ240AS\nRead offset         +103\n"), "{log}");
        assert!(log.contains("Track  1  Jimmy the Exploder  2:30\n    read 1, paranoia 0: Accurate { confidence: 12 }, CRC 1A2B3C4D\n    verdict: ok: ✓ 12\n"), "{log}");
        assert!(log.contains("    read 2, paranoia 3: Mismatch, CRC 11111111\n    verdict: ok: kept, unverified\n"), "the retry's verdict, not the first: {log}");
        assert!(log.ends_with("2 tracks: 2 kept, 1 verified\n"), "{log}");
    }

    #[test]
    fn the_rip_log_counts_only_accuraterip_matches_as_verified() {
        let mut c = disc();
        c.add(Subject::Part(3), "verdict", "ok: ✓ two reads agree", "Rip");
        c.add(Subject::Part(4), "verdict", "ok: ✓ combined from 3 reads", "Rip");
        let log = rip_log(&c, &[1, 2, 3, 4], "2026-11-20");
        assert!(log.ends_with("4 tracks: 4 kept, 1 verified\n"), "{log}");
    }
}
