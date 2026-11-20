//! cyanrip: reading a disc's table of contents, and ripping one track.
//!
//! One process per track. Measured on cyanrip 0.9.3: a track's file stays
//! empty until the whole process exits, however long after its "ripped
//! successfully" line, so a multi-track invocation can't hand tracks to the
//! encoders as they finish. One per track can, and retrying a track is the
//! same call. (Verdict reading is lifted from the old app's cyanrip.rs.)

use super::tool;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// What the disc says about itself, before anything is ripped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiscInfo {
    /// The drive, as cyanrip names it, spaces collapsed.
    pub drive: String,
    /// MusicBrainz disc ID, from the table of contents.
    pub disc_id: String,
    /// The TOC as MusicBrainz spells it: `1 6 193667 150 94460 …`.
    pub toc: String,
    pub mcn: String,
    /// The album title from CD-Text, if the disc has any.
    pub album: String,
    pub performer: String,
    pub tracks: Vec<TrackInfo>,
    /// The drive reads at one speed only: cyanrip's `Speed: default
    /// (unchangeable)`, and `-S` would fail.
    pub fixed_speed: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackInfo {
    pub number: u32,
    pub samples: u64,
    /// The title from CD-Text, if any.
    pub title: String,
    pub isrc: String,
    /// The disc says the track was mastered with pre-emphasis (in its TOC
    /// or its subcode).
    pub preemphasis: bool,
}

impl TrackInfo {
    pub fn seconds(&self) -> u32 {
        (self.samples / 44_100) as u32
    }
}

pub async fn info(device: &str, cwd: &Path, stop: impl std::future::Future<Output = ()>) -> Result<DiscInfo> {
    let mut cmd = tool::command("cyanrip");
    // No lookups of its own: -N MusicBrainz, -A AccurateRip, -U cover art.
    cmd.args(["-d", device, "-I", "-N", "-A", "-U"]);
    let (text, _) = tool::run(cmd, cwd, stop, |_| {}).await?;
    let info = parse_info(&text);
    if info.disc_id.is_empty() {
        return Err(anyhow!("cyanrip couldn't read a table of contents: {}", tool::last_meaningful_line(&text).unwrap_or("no output")));
    }
    Ok(info)
}

pub fn parse_info(text: &str) -> DiscInfo {
    let mut d = DiscInfo::default();
    for line in text.lines() {
        if let Some(v) = field(line, "Device model:") {
            d.drive = v.split_whitespace().collect::<Vec<_>>().join(" ");
        } else if let Some(v) = field(line, "DiscID:") {
            d.disc_id = v.into();
        } else if let Some(v) = line.split("cdtoc/attach?toc=").nth(1) {
            d.toc = v.split('&').next().unwrap_or("").replace('+', " ");
        } else if let Some(v) = field(line, "Disc MCN:") {
            if !v.trim_matches('0').is_empty() {
                d.mcn = v.into();
            }
        } else if let Some(v) = field(line, "Speed:") {
            d.fixed_speed = v.contains("unchangeable");
        } else if let Some(v) = field(line, "Album:") {
            if !v.starts_with("Unknown disc") {
                d.album = v.into();
            }
        }
    }
    for (number, block) in track_blocks(text) {
        let get = |k: &str| block.lines().find_map(|l| field(l, k)).unwrap_or("").to_owned();
        let title = get("title:");
        if d.performer.is_empty() {
            let p = get("album_artist:");
            if !p.is_empty() {
                d.performer = p;
            }
        }
        d.tracks.push(TrackInfo {
            number,
            samples: get("Samples:").parse().unwrap_or(0),
            title: if title == "Unknown track" { String::new() } else { title },
            isrc: get("isrc:"),
            preemphasis: preemphasis(block),
        });
    }
    d
}

/// What AccurateRip said about one track. The difference between
/// *disagreed* and *nothing to compare with* is the whole point: cyanrip
/// prints them the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accurate {
        confidence: u32,
    },
    /// The database holds this disc and the checksums disagreed.
    Mismatch,
    NotInDatabase,
    /// Checking was off, or the offset isn't known.
    NotChecked,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackRip {
    pub file: PathBuf,
    pub verdict: Verdict,
    /// The EAC-style CRC of the audio: two reads of the same track agree
    /// exactly when this does.
    pub crc: String,
    /// The disc flags this track as pre-emphasised. The samples are as
    /// the disc holds them either way: de-emphasis is off (`-W`).
    pub preemphasis: bool,
    /// How many sample frames cyanrip says the track has.
    pub samples: u64,
}

pub struct RipOptions<'a> {
    pub device: &'a str,
    pub offset: Option<i64>,
    /// cd-paranoia level: 0 fast, 3 careful.
    pub paranoia: u8,
    /// Read speed, or none for the drive's default.
    pub speed: Option<u32>,
}

/// cyanrip's arguments for one read of one track, named `NN.wav` in the
/// folder it runs in.
///
/// `-W`: no de-emphasis. cyanrip 0.9.3 applies it by default to a track
/// the disc flags as pre-emphasised, which changes every sample; the
/// archive keeps the samples as the disc holds them and records the flag
/// instead (`FLAGS PRE` in the cue sheet, `PREEMPHASIS=1` in the tags).
///
/// Track 0 is the audio hidden before track 1, in its pregap. cyanrip
/// drops that pregap by default; `-p 1=track` splits it off as a track of
/// its own, numbered 0, that `-l 0` selects. Only track 0's read passes
/// it, so tracks 1 to N are read exactly as before.
pub fn rip_args(o: &RipOptions<'_>, track: u32) -> Vec<String> {
    let mut a: Vec<String> = vec!["-d".into(), o.device.into(), "-s".into(), o.offset.unwrap_or(0).to_string(), "-P".into(), o.paranoia.min(3).to_string()];
    if let Some(s) = o.speed {
        a.extend(["-S".into(), s.to_string()]);
    }
    // -N -U -G: no lookups or art of its own (Choose already decided);
    // -K: no ReplayGain tags on a one-track run; -o wav: we encode.
    a.extend(["-N", "-U", "-G", "-K", "-W", "-o", "wav"].map(String::from));
    if track == 0 {
        a.extend(["-p", "1=track"].map(String::from));
    }
    a.extend(["-l".into(), track.to_string()]);
    let name = format!("{track:02}");
    a.extend(["-D".into(), ".".into(), "-F".into(), name.clone(), "-L".into(), name.clone(), "-M".into(), name]);
    // Every track is checked against AccurateRip; only an unknown offset,
    // which Rip refuses before it gets here, could make that meaningless.
    if o.offset.is_none() {
        a.push("-A".into());
    }
    a
}

/// The drive wouldn't change speed for `-S`. cyanrip 0.9.3 stops before
/// reading anything, exit 1, saying `Device does not support changing
/// speeds!` when the drive doesn't claim it can (`cyanrip_main.c` 158–162;
/// the MATSHITA UJ240AS does this), or `cdio error: 405: Option not
/// supported by drive` when it claims to and then refuses (165–171,
/// libcdio-paranoia's `cdio_cddap_speed_set`).
#[derive(Debug)]
pub struct SpeedRefused;

impl std::fmt::Display for SpeedRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the drive won't change speed")
    }
}

impl std::error::Error for SpeedRefused {}

/// Whether cyanrip's output says the drive refused the speed asked for.
pub fn speed_refused(text: &str) -> bool {
    text.contains("Device does not support changing speeds!") || text.contains("405: Option not supported by drive")
}

/// Rips one track to WAV in `dir`. `progress` gets 0 to 1 as it reads.
pub async fn rip_track(o: &RipOptions<'_>, track: u32, dir: &Path, stop: impl std::future::Future<Output = ()>, mut progress: impl FnMut(f32)) -> Result<TrackRip> {
    std::fs::create_dir_all(dir)?;
    let mut cmd = tool::command("cyanrip");
    cmd.args(rip_args(o, track));
    let (text, status) = tool::run(cmd, dir, stop, |l| {
        if let Some(p) = l.split("progress -").nth(1).and_then(|r| r.trim().split('%').next()).and_then(|p| p.trim().parse::<f32>().ok()) {
            progress((p / 100.0).clamp(0.0, 1.0));
        }
    })
    .await?;
    let file = dir.join(format!("{track:02}.wav"));
    if o.speed.is_some() && !status.success() && speed_refused(&text) {
        return Err(SpeedRefused.into());
    }
    if !status.success() || !text.contains("ripped and encoded successfully") || !file.exists() {
        return Err(anyhow!("cyanrip failed on track {track}: {}", tool::last_meaningful_line(&text).unwrap_or("no output")));
    }
    parse_rip(&text, track, o.offset.is_some(), file)
}

/// What one read printed, for track `track`.
pub fn parse_rip(text: &str, track: u32, checked: bool, file: PathBuf) -> Result<TrackRip> {
    // Never filed changed: if cyanrip ever says it de-emphasised, the
    // samples aren't the disc's.
    if text.contains("deemphasis applied") || text.contains("deemphasis forced") {
        return Err(anyhow!("cyanrip de-emphasised track {track}; the samples aren't the disc's"));
    }
    let block = track_blocks(text).into_iter().find(|(n, _)| *n == track).map(|(_, b)| b).unwrap_or("");
    let verdict = if !checked { Verdict::NotChecked } else { verdict(block) };
    let crc = block.lines().find_map(|l| field(l, "EAC CRC32:")).unwrap_or("").to_owned();
    let samples = block.lines().find_map(|l| field(l, "Samples:")).and_then(|v| v.parse().ok()).unwrap_or(0);
    Ok(TrackRip { file, verdict, crc, preemphasis: preemphasis(block), samples })
}

/// cyanrip's `Preemphasis:` line: *present (TOC)*, *present (subcode)* or
/// *none detected*.
fn preemphasis(block: &str) -> bool {
    block.lines().find_map(|l| field(l, "Preemphasis:")).is_some_and(|v| v.starts_with("present"))
}

/// A match is "some line says accurately ripped"; never "no line
/// complains". When v2 matches, cyanrip prints the v1 line with no note.
pub fn verdict(block: &str) -> Verdict {
    for line in block.lines() {
        if line.contains("accurately ripped") {
            let confidence =
                line.split("confidence").nth(1).and_then(|r| r.trim_start_matches([':', ' ']).split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty())).and_then(|n| n.parse().ok()).unwrap_or(0);
            return Verdict::Accurate { confidence };
        }
    }
    if block.lines().any(|l| l.contains("Accurip:") && l.contains("disc found in database")) {
        Verdict::Mismatch
    } else {
        Verdict::NotInDatabase
    }
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    Some(line.trim_start().strip_prefix(key)?.trim())
}

/// Per-track sections: `Track 3 info:` or `Track 3 ripped and encoded
/// successfully!`. A space after `Track` keeps `Tracks ripped accurately`
/// out.
fn track_blocks(text: &str) -> Vec<(u32, &str)> {
    let mut starts = Vec::new();
    let mut at = 0;
    for raw in text.split_inclusive('\n') {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Track ") {
            if let Ok(n) = rest.split_whitespace().next().unwrap_or("").parse::<u32>() {
                starts.push((n, at));
            }
        }
        at += raw.len();
    }
    starts.iter().enumerate().map(|(i, &(n, from))| (n, &text[from..starts.get(i + 1).map_or(text.len(), |s| s.1)])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "Device model:   MATSHITA BD-MLT UJ240AS   1.01 SCSI CD-ROM
https://musicbrainz.org/cdtoc/attach?toc=1+6+193667+150+94460+119047+131985+154632+167352&tracks=6&id=EmFbrW1HAf5UcXdMb5ZtVV40phs-
DiscID:         EmFbrW1HAf5UcXdMb5ZtVV40phs-
Disc MCN:       0000000000000
Album:          Unknown disc (EMFB)
Tracks:
Track 1 info:
  Properties:
    Samples:     55454280
  Metadata:
    title:      Unknown track
Track 2 info:
  Properties:
    Samples:     14457156
  Metadata:
    title:      Suzy Lee
Tracks ripped accurately: 0/6
";

    #[test]
    fn reads_what_a_real_disc_printed() {
        let d = parse_info(INFO);
        assert_eq!(d.drive, "MATSHITA BD-MLT UJ240AS 1.01 SCSI CD-ROM");
        assert_eq!(d.disc_id, "EmFbrW1HAf5UcXdMb5ZtVV40phs-");
        assert_eq!(d.toc, "1 6 193667 150 94460 119047 131985 154632 167352");
        assert_eq!(d.mcn, "");
        assert_eq!(d.album, "");
        assert_eq!(d.tracks.len(), 2);
        assert_eq!(d.tracks[0].seconds(), 1257);
        assert_eq!(d.tracks[0].title, "");
        assert_eq!(d.tracks[1].title, "Suzy Lee");
        assert!(!d.fixed_speed);
    }

    #[test]
    fn a_drive_that_wont_change_speed_is_known_by_what_cyanrip_says() {
        // What cyanrip 0.9.3 printed on the MATSHITA UJ240AS, `-I` without
        // and with `-S 4`.
        assert!(parse_info("DiscID:  x\nSpeed:          default (unchangeable)\n").fixed_speed);
        assert!(!parse_info("DiscID:  x\nSpeed:          default (changeable)\n").fixed_speed);
        assert!(speed_refused("Checking /dev/sr0 for cdrom...\n\nOpening drive...\nDevice does not support changing speeds!\n"));
        assert!(speed_refused("Opening drive...\ncdio error: 405: Option not supported by drive\n"));
        assert!(!speed_refused("Track 3 ripped and encoded successfully!\n"));
        let e: anyhow::Error = SpeedRefused.into();
        assert!(e.is::<SpeedRefused>());
    }

    #[test]
    fn the_samples_stay_as_the_disc_holds_them_and_the_flag_is_kept() {
        let o = RipOptions { device: "/dev/sr0", offset: Some(103), paranoia: 0, speed: None };
        let a = rip_args(&o, 3);
        assert!(a.contains(&"-W".to_owned()), "de-emphasis off: {a:?}");
        assert!(!a.contains(&"-E".to_owned()) && !a.contains(&"-p".to_owned()), "{a:?}");
        let out = "Track 3 ripped and encoded successfully!
  Preemphasis:   present (TOC)

  Properties:
    Samples:     588000
    EAC CRC32:   1A2B3C4D
";
        let r = parse_rip(out, 3, true, "03.wav".into()).unwrap();
        assert!(r.preemphasis);
        assert_eq!((r.samples, r.crc.as_str()), (588000, "1A2B3C4D"));
        assert!(!parse_rip("Track 3 ripped and encoded successfully!\n  Preemphasis:   none detected\n", 3, true, "03.wav".into()).unwrap().preemphasis);
        // If cyanrip ever changes the samples, the read fails.
        assert!(parse_rip("Track 3 ripped and encoded successfully!\n  Preemphasis:   present (TOC) (deemphasis applied)\n", 3, true, "03.wav".into()).is_err());
        let info = parse_info("DiscID:  x\nTrack 1 info:\n  Preemphasis:   present (subcode)\n  Properties:\n    Samples:     100\nTrack 2 info:\n  Preemphasis:   none detected\n");
        assert_eq!(info.tracks.iter().map(|t| t.preemphasis).collect::<Vec<_>>(), [true, false]);
    }

    #[test]
    fn only_the_hidden_track_is_split_off_from_the_pregap() {
        let o = RipOptions { device: "/dev/sr0", offset: Some(103), paranoia: 3, speed: None };
        let a = rip_args(&o, 0).join(" ");
        assert!(a.contains("-p 1=track -l 0 -D . -F 00 -L 00 -M 00"), "{a}");
        let one = rip_args(&o, 1).join(" ");
        assert_eq!(one, "-d /dev/sr0 -s 103 -P 3 -N -U -G -K -W -o wav -l 1 -D . -F 01 -L 01 -M 01", "track 1 read as before, plus -W");
    }

    #[test]
    fn a_disc_in_the_database_that_disagrees_is_a_mismatch() {
        let block = "Accurip:       disc found in database (max confidence: 86)
Accurip v1:  0F2791D1 (not found, either a new pressing, or bad rip)
Accurip v2:  7823A8E4 (not found, either a new pressing, or bad rip)";
        assert_eq!(verdict(block), Verdict::Mismatch);
        assert_eq!(verdict("Accurip v2:  1234 (accurately ripped, confidence 7)"), Verdict::Accurate { confidence: 7 });
        assert_eq!(verdict("Accurip:  disc not found in database"), Verdict::NotInDatabase);
    }
}
