//! Encoders: each format with the settings someone who cares would look
//! for (DESIGN-UX.md, "Encoders"), turned into ffmpeg arguments here and
//! nowhere else. Tags come from the context and are written by ffmpeg.
//!
//! An encoder uses whatever arrived with the track: every tag the context
//! has, and the cover when Cover art put one there. Neither is a setting;
//! nor is checking a FLAC, which always happens.

use super::tool;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use trip_engine::setting::ParamKind;
use trip_engine::{Cx, Group, Inlet, Item, ItemKind, ItemState, LaneKind, Outlet, ParamSpec, Step, StepInfo, Subject};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Flac,
    Mp3,
    Opus,
    Aac,
    Vorbis,
    Alac,
    WavPack,
    Wav,
}

pub const ALL: [Codec; 8] = [Codec::Flac, Codec::Mp3, Codec::Opus, Codec::Aac, Codec::Vorbis, Codec::Alac, Codec::WavPack, Codec::Wav];

const ART: &str = "Tags and art";
const ADV: &str = "Advanced";

/// Reads a setting in effect.
type Get<'a> = &'a dyn Fn(&str) -> Option<Value>;

fn s(get: Get, k: &str) -> String {
    match get(k) {
        Some(Value::String(v)) => v,
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

/// An MP3 bitrate type in words, from its stored value.
fn mp3_mode(v: &str) -> &str {
    match v {
        "CBR" => "constant",
        "ABR" => "average",
        "VBR" => "variable",
        _ => v,
    }
}

/// An Opus bitrate type in words, from its stored value.
fn opus_mode(v: &str) -> &str {
    match v {
        "Constrained" => "mostly constant",
        "CBR" => "constant",
        "VBR" => "variable",
        _ => v,
    }
}

fn n(get: Get, k: &str) -> i64 {
    get(k).and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim_end_matches(" kbps").parse().ok()))).unwrap_or(0)
}

impl Codec {
    pub fn kind(self) -> &'static str {
        match self {
            Codec::Flac => "flac",
            Codec::Mp3 => "mp3",
            Codec::Opus => "opus",
            Codec::Aac => "aac",
            Codec::Vorbis => "vorbis",
            Codec::Alac => "alac",
            Codec::WavPack => "wavpack",
            Codec::Wav => "wav",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Codec::Flac => "flac",
            Codec::Mp3 => "mp3",
            Codec::Opus => "opus",
            Codec::Aac | Codec::Alac => "m4a",
            Codec::Vorbis => "ogg",
            Codec::WavPack => "wv",
            Codec::Wav => "wav",
        }
    }

    /// Whether the container holds a cover picture (Ogg and WAV don't,
    /// through ffmpeg).
    fn takes_cover(self) -> bool {
        matches!(self, Codec::Flac | Codec::Mp3 | Codec::Aac | Codec::Alac)
    }

    /// What it's for, in the + picker, where someone picks a format.
    pub fn about(self) -> &'static str {
        match self {
            Codec::Flac => "no quality lost, smaller than WAV",
            Codec::Mp3 => "plays on anything",
            Codec::Opus => "small files, good sound",
            Codec::Aac => "for Apple and most phones",
            Codec::Vorbis => "open format, like MP3",
            Codec::Alac => "no quality lost, for Apple",
            Codec::WavPack => "no quality lost",
            Codec::Wav => "no compression, biggest files",
        }
    }

    pub fn info(self) -> StepInfo {
        let i = |name| StepInfo::new(self.kind(), name, Group::Make, ItemKind::Track, ItemKind::AudioFile).from(self.about());
        let info = match self {
            Codec::Flac => i("FLAC")
                .param(
                    ParamSpec::new("level", "Compression", ParamKind::Range { min: 0, max: 8 })
                        .default(8)
                        .on_card("Level")
                        .help("Higher makes smaller files and takes longer. The sound is identical at every level."),
                )
                .summary(|g| format!("compression {}", n(g, "level"))),
            Codec::Mp3 => i("MP3")
                .param(ParamSpec::new("mode", "Bitrate type", ParamKind::Choice(&["VBR", "CBR", "ABR"])).default("VBR").labels(&["Variable, best for the size", "Constant", "Average"]))
                .param(
                    ParamSpec::new("quality", "Quality", ParamKind::Choice(&["V0", "V1", "V2", "V3", "V4", "V5", "V6", "V7", "V8", "V9"]))
                        .default("V0")
                        .when("mode", &["VBR"])
                        .on_card("Quality")
                        .labels(&[
                            "V0, best (about 245 kbps)",
                            "V1 (about 225 kbps)",
                            "V2 (about 190 kbps)",
                            "V3 (about 175 kbps)",
                            "V4 (about 165 kbps)",
                            "V5 (about 130 kbps)",
                            "V6 (about 115 kbps)",
                            "V7 (about 100 kbps)",
                            "V8 (about 85 kbps)",
                            "V9, smallest (about 65 kbps)",
                        ]),
                )
                .param(
                    ParamSpec::new("bitrate", "Bitrate", ParamKind::Choice(&["128 kbps", "160 kbps", "192 kbps", "224 kbps", "256 kbps", "320 kbps"]))
                        .default("320 kbps")
                        .when("mode", &["CBR", "ABR"])
                        .on_card_as("Bitrate", &["mode"], |g| format!("{} {}", s(g, "bitrate"), mp3_mode(&s(g, "mode")))),
                )
                .param(ParamSpec::new("stereo", "Stereo", ParamKind::Choice(&["Joint", "Full"])).default("Joint").on_card("Stereo").labels(&["Joint, best for the size", "Full"]))
                .param(
                    ParamSpec::new("id3", "Tag version", ParamKind::Choice(&["v2.3", "v2.4"]))
                        .default("v2.3")
                        .section(ART)
                        .labels(&["2.3, works with older players", "2.4, newer"])
                        .help("Choose 2.3 if an older car stereo or Windows shows missing or garbled track info."),
                )
                .param(ParamSpec::new("id3v1", "Extra tags for very old players", ParamKind::Toggle).default(false).section(ART))
                .summary(|g| {
                    let rate = if s(g, "mode") == "VBR" { s(g, "quality") } else { format!("{} {}", s(g, "bitrate"), mp3_mode(&s(g, "mode"))) };
                    format!("{rate}, {} stereo", s(g, "stereo").to_lowercase())
                }),
            Codec::Opus => i("Opus")
                .param(ParamSpec::new("bitrate", "Bitrate", ParamKind::Choice(&["64 kbps", "96 kbps", "128 kbps", "160 kbps", "192 kbps", "256 kbps"])).default("160 kbps").on_card_as(
                    "Bitrate",
                    &["mode"],
                    |g| format!("{} {}", s(g, "bitrate"), opus_mode(&s(g, "mode"))),
                ))
                .param(ParamSpec::new("mode", "Bitrate type", ParamKind::Choice(&["VBR", "Constrained", "CBR"])).default("VBR").labels(&["Variable, best for the size", "Mostly constant", "Constant"]))
                .param(ParamSpec::new("complexity", "Effort (higher is slower, sounds a little better)", ParamKind::Range { min: 0, max: 10 }).default(10).section(ADV))
                .param(ParamSpec::new("for", "Tuned for", ParamKind::Choice(&["Music", "Speech", "Low delay"])).default("Music").section(ADV).labels(&["Music", "Speech", "Live calls"]))
                .summary(|g| format!("{} {}", s(g, "bitrate"), opus_mode(&s(g, "mode")))),
            Codec::Aac => i("AAC")
                .param(ParamSpec::new("bitrate", "Bitrate", ParamKind::Choice(&["128 kbps", "192 kbps", "256 kbps", "320 kbps"])).default("256 kbps").on_card("Bitrate"))
                .summary(|g| s(g, "bitrate")),
            Codec::Vorbis => i("Vorbis")
                .param(ParamSpec::new("quality", "Quality (higher is better)", ParamKind::Range { min: -1, max: 10 }).default(6).on_card("Quality").help("6 is about 192 kbps."))
                .summary(|g| format!("quality {}", n(g, "quality"))),
            Codec::Alac => i("ALAC"),
            Codec::WavPack => i("WavPack")
                .param(
                    ParamSpec::new("compression", "Compression", ParamKind::Choice(&["Fast", "Normal", "High", "Very high"]))
                        .default("High")
                        .on_card("Level")
                        .help("Higher makes smaller files and takes longer. The sound is identical at every level."),
                )
                .summary(|g| format!("{} compression", s(g, "compression").to_lowercase())),
            Codec::Wav => i("WAV"),
        };
        info.param(ParamSpec::new("extra", "Extra ffmpeg arguments", ParamKind::Text).default("").section(ADV).help("Given to ffmpeg as written, split at spaces. Leave empty unless you know ffmpeg."))
    }

    /// ffmpeg's arguments for the audio, from the settings in effect.
    pub fn audio_args(self, get: Get) -> Vec<String> {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let kbps = |k| format!("{}k", n(get, k));
        match self {
            Codec::Flac => v(&["-c:a", "flac", "-compression_level", &n(get, "level").to_string()]),
            Codec::Mp3 => {
                let mut a = v(&["-c:a", "libmp3lame"]);
                match s(get, "mode").as_str() {
                    "CBR" => a.extend(v(&["-b:a", &kbps("bitrate")])),
                    "ABR" => a.extend(v(&["-abr", "1", "-b:a", &kbps("bitrate")])),
                    _ => a.extend(v(&["-q:a", s(get, "quality").trim_start_matches('V')])),
                }
                a.extend(v(&["-joint_stereo", if s(get, "stereo") == "Full" { "0" } else { "1" }]));
                a.extend(v(&["-id3v2_version", if s(get, "id3") == "v2.4" { "4" } else { "3" }]));
                if get("id3v1").and_then(|x| x.as_bool()).unwrap_or(false) {
                    a.extend(v(&["-write_id3v1", "1"]));
                }
                a
            }
            Codec::Opus => {
                let mode = match s(get, "mode").as_str() {
                    "CBR" => "off",
                    "Constrained" => "constrained",
                    _ => "on",
                };
                let app = match s(get, "for").as_str() {
                    "Speech" => "voip",
                    "Low delay" => "lowdelay",
                    _ => "audio",
                };
                v(&["-c:a", "libopus", "-b:a", &kbps("bitrate"), "-vbr", mode, "-compression_level", &n(get, "complexity").to_string(), "-application", app])
            }
            Codec::Aac => v(&["-c:a", "aac", "-b:a", &kbps("bitrate")]),
            Codec::Vorbis => v(&["-c:a", "libvorbis", "-q:a", &n(get, "quality").to_string()]),
            Codec::Alac => v(&["-c:a", "alac"]),
            Codec::WavPack => {
                let level = match s(get, "compression").as_str() {
                    "Fast" => "0",
                    "Normal" => "1",
                    "Very high" => "3",
                    _ => "2",
                };
                v(&["-c:a", "wavpack", "-compression_level", level])
            }
            Codec::Wav => v(&["-c:a", "pcm_s16le"]),
        }
    }

    /// Whether it keeps every sample: then the file is decoded after
    /// encoding and must give exactly the rip back.
    pub fn lossless(self) -> bool {
        matches!(self, Codec::Flac | Codec::Alac | Codec::WavPack | Codec::Wav)
    }

    /// The tags for this container, from [`tags`]' names, each where
    /// MusicBrainz Picard writes and reads it (checked against Picard's
    /// `formats/` modules, and by reading files back after the bundled
    /// ffmpeg wrote them). Gives what ffmpeg writes, and what it can't,
    /// written afterwards (see [`finish_tags`]).
    ///
    /// - Vorbis comments (FLAC, Opus, Vorbis): `TRACKNUMBER=1` with
    ///   `TRACKTOTAL=13`, `DISCNUMBER` with `DISCTOTAL`; the recording's
    ///   ID is `MUSICBRAINZ_TRACKID`, the release track's
    ///   `MUSICBRAINZ_RELEASETRACKID`.
    /// - ID3 (MP3): `TRCK 1/13`, `TPOS 1/2`, the ISRC in `TSRC`, the label
    ///   in `TPUB`, the MusicBrainz IDs in `TXXX:MusicBrainz Album Id` and
    ///   the like, the recording's in a `UFID` frame (afterwards: ffmpeg
    ///   can't write one).
    /// - MP4 (AAC, ALAC): ffmpeg writes only the iTunes atoms (`trkn`,
    ///   `disk`, `aART`, `©day`, `©lyr`); the rest are `----:com.apple.iTunes:`
    ///   atoms, afterwards.
    /// - APEv2 (WavPack): `Track 1/13`, `Disc 1/2`, `Year`, `Album Artist`.
    /// - WAV: only what RIFF INFO has a place for (title, artist, album,
    ///   date, track). INFO's `ISRC` chunk means *source*, so the ISRC
    ///   isn't written there.
    pub fn container_tags(self, tags: &[(String, String)]) -> (Vec<(String, String)>, Vec<Later>) {
        let get = |k: &str| tags.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
        let of = |n: &str, total: &str| {
            get(n).map(|n| match get(total) {
                Some(t) => format!("{n}/{t}"),
                None => n.to_owned(),
            })
        };
        let (mut now, mut later) = (vec![], vec![]);
        let mut put = |k: &str, v: Option<String>| {
            if let Some(v) = v {
                now.push((k.to_owned(), v));
            }
        };
        let g = |k: &str| get(k).map(str::to_owned);
        match self {
            Codec::Flac | Codec::Opus | Codec::Vorbis => {
                for (k, name) in [
                    ("title", "TITLE"),
                    ("artist", "ARTIST"),
                    ("album", "ALBUM"),
                    ("album_artist", "ALBUMARTIST"),
                    ("date", "DATE"),
                    ("tracknumber", "TRACKNUMBER"),
                    ("tracktotal", "TRACKTOTAL"),
                    ("discnumber", "DISCNUMBER"),
                    ("disctotal", "DISCTOTAL"),
                    ("label", "LABEL"),
                    ("catalognumber", "CATALOGNUMBER"),
                    ("barcode", "BARCODE"),
                    ("isrc", "ISRC"),
                    ("musicbrainz_albumid", "MUSICBRAINZ_ALBUMID"),
                    ("musicbrainz_releasegroupid", "MUSICBRAINZ_RELEASEGROUPID"),
                    ("musicbrainz_releasetrackid", "MUSICBRAINZ_RELEASETRACKID"),
                    ("musicbrainz_recordingid", "MUSICBRAINZ_TRACKID"),
                    ("lyrics", "LYRICS"),
                    ("preemphasis", "PREEMPHASIS"),
                ] {
                    put(name, g(k));
                }
            }
            Codec::Mp3 => {
                for k in ["title", "artist", "album", "album_artist", "date"] {
                    put(k, g(k));
                }
                put("track", of("tracknumber", "tracktotal"));
                put("disc", of("discnumber", "disctotal"));
                put("publisher", g("label"));
                put("TSRC", g("isrc"));
                for (k, name) in [
                    ("catalognumber", "CATALOGNUMBER"),
                    ("barcode", "BARCODE"),
                    ("musicbrainz_albumid", "MusicBrainz Album Id"),
                    ("musicbrainz_releasegroupid", "MusicBrainz Release Group Id"),
                    ("musicbrainz_releasetrackid", "MusicBrainz Release Track Id"),
                    ("preemphasis", "PREEMPHASIS"),
                ] {
                    put(name, g(k));
                }
                put("lyrics", g("lyrics"));
                if let Some(r) = g("musicbrainz_recordingid") {
                    later.push(Later::Ufid(r));
                }
            }
            Codec::Aac | Codec::Alac => {
                for k in ["title", "artist", "album", "album_artist", "date", "lyrics"] {
                    put(k, g(k));
                }
                put("track", of("tracknumber", "tracktotal"));
                put("disc", of("discnumber", "disctotal"));
                for (k, name) in [
                    ("label", "LABEL"),
                    ("catalognumber", "CATALOGNUMBER"),
                    ("barcode", "BARCODE"),
                    ("isrc", "ISRC"),
                    ("musicbrainz_albumid", "MusicBrainz Album Id"),
                    ("musicbrainz_releasegroupid", "MusicBrainz Release Group Id"),
                    ("musicbrainz_releasetrackid", "MusicBrainz Release Track Id"),
                    ("musicbrainz_recordingid", "MusicBrainz Track Id"),
                    ("preemphasis", "PREEMPHASIS"),
                ] {
                    if let Some(v) = g(k) {
                        later.push(Later::Freeform(name.to_owned(), v));
                    }
                }
            }
            Codec::WavPack => {
                for (k, name) in [("title", "Title"), ("artist", "Artist"), ("album", "Album"), ("album_artist", "Album Artist"), ("date", "Year")] {
                    put(name, g(k));
                }
                put("Track", of("tracknumber", "tracktotal"));
                put("Disc", of("discnumber", "disctotal"));
                for (k, name) in [
                    ("label", "Label"),
                    ("catalognumber", "CatalogNumber"),
                    ("barcode", "Barcode"),
                    ("isrc", "ISRC"),
                    ("musicbrainz_albumid", "MUSICBRAINZ_ALBUMID"),
                    ("musicbrainz_releasegroupid", "MUSICBRAINZ_RELEASEGROUPID"),
                    ("musicbrainz_releasetrackid", "MUSICBRAINZ_RELEASETRACKID"),
                    ("musicbrainz_recordingid", "MUSICBRAINZ_TRACKID"),
                    ("lyrics", "Lyrics"),
                    ("preemphasis", "PREEMPHASIS"),
                ] {
                    put(name, g(k));
                }
            }
            Codec::Wav => {
                for k in ["title", "artist", "album", "date"] {
                    put(k, g(k));
                }
                put("track", g("tracknumber"));
            }
        }
        (now, later)
    }

    /// Cover arguments for an input already added as `-i cover` (input 1):
    /// the picture as Cover art saved it, unchanged. Its size is Cover
    /// art's to choose. The comment is how ffmpeg is told the picture's
    /// type: without it a FLAC PICTURE block and an MP3 APIC frame say
    /// "Other" (0), not "Front cover" (3).
    fn cover_args(self) -> Vec<String> {
        ["-map", "0:a", "-map", "1:v", "-c:v", "copy", "-disposition:v", "attached_pic", "-metadata:s:v", "comment=Cover (front)"].iter().map(|s| s.to_string()).collect()
    }
}

/// A tag ffmpeg can't write, written into the file after it.
#[derive(Clone, Debug, PartialEq)]
pub enum Later {
    /// ID3's `UFID` frame owned by `http://musicbrainz.org`: the recording.
    Ufid(String),
    /// An MP4 `----:com.apple.iTunes:<name>` atom.
    Freeform(String, String),
}

/// Writes the tags ffmpeg couldn't into its file, changing nothing else:
/// read without lofty's conversions, the one tag rewritten, the ID3
/// version kept. The audio of a lossless file is checked after this.
pub fn finish_tags(codec: Codec, file: &std::path::Path, later: &[Later], id3v23: bool) -> Result<()> {
    use lofty::config::{ParseOptions, WriteOptions};
    use lofty::prelude::*;
    if later.is_empty() {
        return Ok(());
    }
    let parse = ParseOptions::new().implicit_conversions(false);
    let write = WriteOptions::default().use_id3v23(id3v23).preferred_padding(0).parse_options(parse);
    let mut f = std::fs::File::open(file)?;
    match codec {
        Codec::Mp3 => {
            let mut m = lofty::mpeg::MpegFile::read_from(&mut f, parse)?;
            drop(f);
            if m.id3v2().is_none() {
                m.set_id3v2(lofty::id3::v2::Id3v2Tag::new());
            }
            let tag = m.id3v2_mut().unwrap();
            for l in later {
                if let Later::Ufid(id) = l {
                    let ufid = lofty::id3::v2::UniqueFileIdentifierFrame::new("http://musicbrainz.org".to_owned(), id.as_bytes().to_vec());
                    tag.insert(lofty::id3::v2::Frame::UniqueFileIdentifier(ufid));
                }
            }
            tag.save_to_path(file, write)?;
        }
        Codec::Aac | Codec::Alac => {
            use lofty::mp4::{Atom, AtomData, AtomIdent};
            let mut m = lofty::mp4::Mp4File::read_from(&mut f, parse)?;
            drop(f);
            if m.ilst().is_none() {
                m.set_ilst(lofty::mp4::Ilst::new());
            }
            let ilst = m.ilst_mut().unwrap();
            for l in later {
                if let Later::Freeform(name, value) = l {
                    let ident = AtomIdent::Freeform { mean: "com.apple.iTunes".into(), name: name.clone().into() };
                    ilst.insert(Atom::new(ident, AtomData::UTF8(value.clone())));
                }
            }
            ilst.save_to_path(file, write)?;
        }
        _ => bail!("nothing to write afterwards in {}", codec.kind()),
    }
    Ok(())
}

/// Tags for one track, from what the track knows: the chosen release's
/// facts first, under names of our own that [`Codec::container_tags`]
/// turns into each container's. Written by ffmpeg (and [`finish_tags`]),
/// never by cyanrip, whose tag syntax mangles a title with a colon in it.
pub fn tags(c: &trip_engine::Context, t: u32) -> Vec<(String, String)> {
    {
        let n = c.parts();
        let mut out = vec![];
        let mut put = |k: &str, v: Option<&str>| {
            if let Some(v) = v.filter(|v| !v.is_empty()) {
                out.push((k.to_owned(), v.to_owned()));
            }
        };
        put("title", c.get(Subject::Part(t), "title"));
        put("artist", c.get(Subject::Part(t), "artist").or(c.get(Subject::Disc, "album_artist")));
        put("album", c.get(Subject::Disc, "album"));
        put("album_artist", c.get(Subject::Disc, "album_artist"));
        put("date", c.get(Subject::Disc, "date"));
        // The hidden track is track 0 of the same 13.
        put("tracknumber", Some(&t.to_string()));
        put("tracktotal", Some(&n.to_string()));
        put("discnumber", c.get(Subject::Disc, "discnumber"));
        put("disctotal", c.get(Subject::Disc, "disctotal").filter(|_| c.get(Subject::Disc, "discnumber").is_some()));
        for k in ["label", "catalognumber", "barcode", "musicbrainz_albumid", "musicbrainz_releasegroupid"] {
            put(k, c.get(Subject::Disc, k));
        }
        put("musicbrainz_releasetrackid", c.get(Subject::Part(t), "musicbrainz_releasetrackid"));
        put("musicbrainz_recordingid", c.get(Subject::Part(t), "musicbrainz_recordingid"));
        put("isrc", c.get(Subject::Part(t), "isrc"));
        put("lyrics", c.get(Subject::Part(t), "lyrics"));
        // Kept as the disc has it: a player that sees this undoes it.
        put("preemphasis", c.get(Subject::Part(t), "preemphasis").filter(|v| *v == "1"));
        out
    }
}

/// What an encoder's finished tracks add up to, for what it says about them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sums {
    /// The rip's WAVs that came in.
    pub in_bytes: u64,
    /// The audio in the files written: each file less the cover inside it.
    pub audio_bytes: u64,
    /// How long that audio is, as ffmpeg counted it while encoding.
    pub millis: u64,
}

impl Sums {
    pub fn add(&mut self, other: Sums) {
        self.in_bytes += other.in_bytes;
        self.audio_bytes += other.audio_bytes;
        self.millis += other.millis;
    }
}

/// The one fact about what an encoder made: for a lossless format, its
/// size beside the CD's (*58% of the CD's size*); for a lossy one, the
/// bitrate it came to (*average 245 kbps*). Empty until a track is done.
pub fn packed(codec: Codec, sums: Sums) -> String {
    match codec.lossless() {
        true if sums.in_bytes > 0 && sums.audio_bytes > 0 => format!("{}% of the CD's size", ((sums.audio_bytes * 100 + sums.in_bytes / 2) / sums.in_bytes).max(1)),
        // Bits a millisecond are kilobits a second.
        false if sums.millis > 0 && sums.audio_bytes > 0 => format!("average {} kbps", (sums.audio_bytes * 8 + sums.millis / 2) / sums.millis),
        _ => String::new(),
    }
}

/// An encoder's outcome: what it has written so far (*294 MB*), and what
/// it's on, or, between tracks and once done, how it packed ([`packed`]).
pub fn encoder_says(cx: &Cx, codec: Codec, sums: Sums, on: String) {
    cx.view(|v| {
        let out: u64 = v.record.files.iter().map(|f| f.size).sum();
        v.status = if out > 0 { super::media::size(out) } else { String::new() };
        v.qualifier = String::new();
        v.detail = if on.is_empty() { packed(codec, sums) } else { on };
    });
}

/// The track an encoder is on: *Track 6 · Wasting My Time*.
pub fn on_track(track: &Item) -> String {
    match track.index {
        Some(t) => format!("Track {t} · {}", track.label),
        None => track.label.clone(),
    }
}

/// The real encoder step for one format.
pub struct Encode(pub Codec);

#[async_trait]
impl Step for Encode {
    fn info(&self) -> StepInfo {
        self.0.info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let (busy, done) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        // How far each track in hand has got, 0 to 1, for the tile's bar.
        let partial: Arc<std::sync::Mutex<std::collections::HashMap<String, f32>>> = Arc::default();
        let mut tasks = Vec::new();
        let codec = self.0;
        let mut total = 1;
        // What the tracks done came to: for how well it packed.
        let sums: Arc<std::sync::Mutex<Sums>> = Arc::default();
        while let Some(track) = inlet.recv().await {
            total = super::parts(track.context());
            let (cx, outlet, busy, done, partial, sums) = (cx.clone(), outlet.clone(), busy.clone(), done.clone(), partial.clone(), sums.clone());
            // Its encoder's output is the step's, though it runs apart.
            let step = cx.clone();
            tasks.push(tokio::spawn(trip_engine::within(step, async move {
                let _core = cx.lane(LaneKind::Encode).await;
                let shown = |cx: &Cx| {
                    let (b, d) = (busy.load(Ordering::SeqCst), done.load(Ordering::SeqCst));
                    let part: f32 = partial.lock().unwrap().values().sum();
                    encoder_says(cx, codec, *sums.lock().unwrap(), if b > 0 { on_track(&track) } else { String::new() });
                    cx.progress(Some((d as f32 + part) / total as f32));
                };
                busy.fetch_add(1, Ordering::SeqCst);
                cx.item(&track, ItemState::Busy, "encoding 0%");
                shown(&cx);
                let started = std::time::Instant::now();
                let r = encode(codec, &cx, &track, |f| {
                    partial.lock().unwrap().insert(track.id.clone(), f);
                    let left = (f > 0.05).then(|| started.elapsed().as_secs_f32() * (1.0 - f) / f);
                    let note = match left {
                        Some(s) if s >= 1.5 => format!("encoding {}%, {}s left", (f * 100.0) as u32, s.round() as u32),
                        _ => format!("encoding {}%", (f * 100.0) as u32),
                    };
                    cx.item(&track, ItemState::Busy, note);
                    shown(&cx);
                })
                .await;
                partial.lock().unwrap().remove(&track.id);
                busy.fetch_sub(1, Ordering::SeqCst);
                match r {
                    Ok((path, verified, made)) => {
                        done.fetch_add(1, Ordering::SeqCst);
                        sums.lock().unwrap().add(made);
                        cx.item(&track, ItemState::Ok, if verified { "verified" } else { "" });
                        outlet.send(track.becomes(ItemKind::AudioFile, Some(path))).await;
                    }
                    Err(e) => {
                        cx.item(&track, ItemState::Bad, format!("Couldn't make the file: {e}"));
                        cx.log(format!("{}: {e}", track.label));
                    }
                }
                shown(&cx);
            })));
        }
        for t in tasks {
            t.await?;
        }
        // Each task writes the tile as it ends, and two ending together can
        // write in either order; say the final count once, after all of them.
        let _ = (done.load(Ordering::SeqCst), total);
        let sums = *sums.lock().unwrap();
        encoder_says(&cx, codec, sums, String::new());
        cx.progress(None);
        Ok(())
    }
}

/// ffmpeg's whole command line for one track. Only the tags given are
/// written: `-map_metadata -1` keeps whatever the rip's WAV carried (cyanrip
/// puts its own name in it) out of every file.
pub fn ffmpeg_args(codec: Codec, src: &std::path::Path, cover: Option<&std::path::Path>, get: Get, tags: &[(String, String)], extra: &str, out: &std::path::Path) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = Vec::new();
    let mut push = |x: &str| a.push(x.into());
    // -progress prints out_time_us=… as it goes.
    for x in ["-nostdin", "-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-y", "-i"] {
        push(x);
    }
    a.push(src.into());
    if let Some(c) = cover {
        a.push("-i".into());
        a.push(c.into());
        a.extend(codec.cover_args().into_iter().map(Into::into));
    }
    a.extend(["-map_metadata", "-1"].map(Into::into));
    a.extend(codec.audio_args(get).into_iter().map(Into::into));
    for (k, v) in codec.container_tags(tags).0 {
        a.push("-metadata".into());
        a.push(format!("{k}={v}").into());
    }
    // Yours, by design: arguments, not a shell line, so nothing in them is
    // run, but ffmpeg does with them whatever they say.
    a.extend(extra.split_whitespace().map(Into::into));
    a.push(out.into());
    a
}

/// Encodes one track; `progress` gets 0 to 1 as ffmpeg goes, measured
/// against the track's length. Gives the file, whether it was verified,
/// and what it came to.
async fn encode(codec: Codec, cx: &Cx, track: &Item, progress: impl Fn(f32)) -> Result<(PathBuf, bool, Sums)> {
    let t = track.index.unwrap_or(0);
    let src = track.path.clone().ok_or_else(|| anyhow!("no audio"))?;
    let dir = cx.work()?.join(codec.kind());
    std::fs::create_dir_all(&dir)?;
    let out = dir.join(format!("{t:02}.{}", codec.ext()));
    let get = |k: &str| cx.param(k);
    let c = track.context();
    // The cover, if one came with the track and the container can hold it.
    let cover = c.get(Subject::Disc, "cover").map(PathBuf::from).filter(|p| p.exists() && codec.takes_cover());
    let length = c.get(Subject::Part(t), "length").and_then(|l| l.parse::<f32>().ok()).unwrap_or(0.0);
    let job = Job::new(codec, &src, cover.as_deref(), &get, &tags(c, t), &cx.text("extra"), &out);
    let (verified, millis) = job.run_timed(|| cx.cancelled(), length, progress).await?;
    let size = |p: &std::path::Path| std::fs::metadata(p).map_or(0, |m| m.len());
    // The cover goes in as it is (`-c:v copy`), so its file's size is what it adds.
    let audio_bytes = size(&out).saturating_sub(cover.as_deref().map_or(0, size));
    Ok((out, verified, Sums { in_bytes: size(&src), audio_bytes, millis }))
}

/// One encode, worked out from the settings in effect before it runs.
pub struct Job {
    pub codec: Codec,
    pub src: PathBuf,
    pub out: PathBuf,
    pub args: Vec<std::ffi::OsString>,
    /// Tags ffmpeg can't write, written after it.
    pub later: Vec<Later>,
    pub id3v23: bool,
}

impl Job {
    #[allow(clippy::too_many_arguments)]
    pub fn new(codec: Codec, src: &std::path::Path, cover: Option<&std::path::Path>, get: Get, tags: &[(String, String)], extra: &str, out: &std::path::Path) -> Job {
        Job { codec, src: src.to_owned(), out: out.to_owned(), args: ffmpeg_args(codec, src, cover, get, tags, extra, out), later: codec.container_tags(tags).1, id3v23: s(get, "id3") != "v2.4" }
    }

    /// Encodes, writes the tags ffmpeg can't, then, for a lossless
    /// format, decodes the file and checks it gives exactly the source's
    /// samples: a mismatch is an error, and the track isn't filed. Gives
    /// whether it was checked.
    pub async fn run<F: std::future::Future<Output = ()>>(&self, stop: impl Fn() -> F, length: f32, progress: impl Fn(f32)) -> Result<bool> {
        Ok(self.run_timed(stop, length, progress).await?.0)
    }

    /// [`Job::run`], also giving how long the audio written is, in
    /// milliseconds, as ffmpeg counted it.
    pub async fn run_timed<F: std::future::Future<Output = ()>>(&self, stop: impl Fn() -> F, length: f32, progress: impl Fn(f32)) -> Result<(bool, u64)> {
        let dir = self.out.parent().unwrap_or(std::path::Path::new("."));
        let mut cmd = tool::command("ffmpeg");
        cmd.args(&self.args);
        let written = std::sync::atomic::AtomicU64::new(0);
        let (text, status) = tool::run(cmd, dir, stop(), |l| {
            if let Some(us) = l.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                if us > 0.0 {
                    written.store((us / 1000.0) as u64, Ordering::SeqCst);
                }
                if length > 0.0 {
                    progress(((us / 1e6) as f32 / length).clamp(0.0, 1.0));
                }
            }
        })
        .await?;
        if !status.success() {
            bail!("ffmpeg: {}", tool::last_meaningful_line(&text).unwrap_or("failed"));
        }
        // The tags ffmpeg can't write. A lossy file's audio isn't checked
        // against the rip, so it's checked against itself across this.
        let lossy_before = match !self.later.is_empty() && !self.codec.lossless() {
            true => Some(pcm_md5(&self.out, stop()).await?),
            false => None,
        };
        finish_tags(self.codec, &self.out, &self.later, self.id3v23).map_err(|e| anyhow!("couldn't write the tags ffmpeg can't: {e}"))?;
        if let Some(before) = lossy_before {
            let after = pcm_md5(&self.out, stop()).await?;
            if after != before {
                bail!("writing the tags changed the audio ({before} ≠ {after})");
            }
        }
        // A lossless file is always checked: it must decode to exactly the rip.
        let verified = self.codec.lossless();
        if verified {
            let (a, b) = (pcm_md5(&self.src, stop()).await?, pcm_md5(&self.out, stop()).await?);
            if a != b {
                bail!("the {} file doesn't decode to the rip ({a} ≠ {b})", self.codec.info().name);
            }
        }
        Ok((verified, written.load(Ordering::SeqCst)))
    }
}

/// The MD5 of a file's decoded audio, as 16-bit PCM.
pub async fn pcm_md5(file: &std::path::Path, stop: impl std::future::Future<Output = ()>) -> Result<String> {
    let mut cmd = tool::command("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"]).arg(file).args(["-map", "0:a", "-c:a", "pcm_s16le", "-f", "md5", "-"]);
    let (text, status) = tool::run(cmd, file.parent().unwrap_or(std::path::Path::new(".")), stop, |_| {}).await?;
    let md5 = text.lines().find_map(|l| l.trim().strip_prefix("MD5=")).map(str::to_owned);
    match md5 {
        Some(m) if status.success() => Ok(m),
        _ => bail!("couldn't check {}: {}", file.display(), tool::last_meaningful_line(&text).unwrap_or("no output")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn how_it_packed_is_said_in_words_for_each_kind() {
        use super::{packed, Codec, Sums};
        // Four minutes of CD audio is 42,336,000 bytes.
        let cd = Sums { in_bytes: 42_336_000, audio_bytes: 24_554_880, millis: 240_000 };
        assert_eq!(packed(Codec::Flac, cd), "58% of the CD's size");
        assert_eq!(packed(Codec::Alac, cd), "58% of the CD's size");
        assert_eq!(packed(Codec::Mp3, Sums { audio_bytes: 7_350_000, ..cd }), "average 245 kbps");
        assert_eq!(packed(Codec::Opus, Sums::default()), "", "nothing until a track is done");
    }

    use super::*;
    use std::collections::HashMap;

    fn with(info: &StepInfo, set: &[(&str, Value)]) -> impl Fn(&str) -> Option<Value> {
        let set: HashMap<String, Value> = set.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        let defaults: HashMap<String, Value> = info.params.iter().filter_map(|p| Some((p.key.to_string(), p.default.clone()?))).collect();
        move |k: &str| set.get(k).or(defaults.get(k)).cloned()
    }

    #[test]
    fn a_track_s_tags_come_from_what_it_knows() {
        let mut c = trip_engine::Context::default();
        for (k, v) in [("parts", "13"), ("album", "Wide Awake!"), ("album_artist", "Parquet Courts"), ("discnumber", "1"), ("disctotal", "2"), ("musicbrainz_albumid", "rel")] {
            c.add(Subject::Disc, k, v, "test");
        }
        c.add(Subject::Part(1), "title", "Total Football", "test");
        c.add(Subject::Part(1), "musicbrainz_releasetrackid", "trk", "test");
        c.add(Subject::Part(1), "musicbrainz_recordingid", "rec", "test");
        c.add(Subject::Part(1), "preemphasis", "1", "test");
        let get = |t: &[(String, String)], k: &str| t.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
        let one = tags(&c, 1);
        for (k, v) in [
            ("title", "Total Football"),
            ("tracknumber", "1"),
            ("tracktotal", "13"),
            ("discnumber", "1"),
            ("disctotal", "2"),
            ("musicbrainz_releasetrackid", "trk"),
            ("musicbrainz_recordingid", "rec"),
            ("preemphasis", "1"),
            ("artist", "Parquet Courts"),
        ] {
            assert_eq!(get(&one, k).as_deref(), Some(v), "{k}: {one:?}");
        }
        assert_eq!(get(&one, "musicbrainz_trackid"), None);
        // The hidden track: track 0 of the same 13, no flag of its own.
        let zero = tags(&c, 0);
        assert_eq!((get(&zero, "tracknumber").as_deref(), get(&zero, "tracktotal").as_deref()), (Some("0"), Some("13")));
        assert_eq!(get(&zero, "preemphasis"), None);
        let (mp3, later) = Codec::Mp3.container_tags(&one);
        assert!(mp3.contains(&("track".into(), "1/13".into())) && mp3.contains(&("disc".into(), "1/2".into())), "{mp3:?}");
        assert_eq!(later, [Later::Ufid("rec".into())]);
    }

    #[test]
    fn mp3_defaults_to_v0_joint_stereo_id3v2_3() {
        let info = Codec::Mp3.info();
        let g = with(&info, &[]);
        assert_eq!(Codec::Mp3.audio_args(&g).join(" "), "-c:a libmp3lame -q:a 0 -joint_stereo 1 -id3v2_version 3");
        assert_eq!((info.summary.unwrap())(&g), "V0, joint stereo");
    }

    #[test]
    fn mp3_cbr_uses_the_bitrate_and_hides_quality() {
        let info = Codec::Mp3.info();
        let g = with(&info, &[("mode", "CBR".into()), ("bitrate", "256 kbps".into()), ("id3", "v2.4".into()), ("id3v1", true.into())]);
        assert_eq!(Codec::Mp3.audio_args(&g).join(" "), "-c:a libmp3lame -b:a 256k -joint_stereo 1 -id3v2_version 4 -write_id3v1 1");
        let quality = info.params.iter().find(|p| p.key == "quality").unwrap();
        assert!(!quality.applies(&g), "quality is VBR's");
        assert!(info.params.iter().find(|p| p.key == "bitrate").unwrap().applies(&g));
        assert_eq!((info.summary.unwrap())(&g), "256 kbps constant, joint stereo");
    }

    #[test]
    fn opus_flac_and_wavpack_turn_into_ffmpeg_arguments() {
        let g = with(&Codec::Opus.info(), &[("mode", "Constrained".into()), ("for", "Speech".into())]);
        assert_eq!(Codec::Opus.audio_args(&g).join(" "), "-c:a libopus -b:a 160k -vbr constrained -compression_level 10 -application voip");
        let info = Codec::Flac.info();
        let g = with(&info, &[("level", 5.into())]);
        assert_eq!(Codec::Flac.audio_args(&g).join(" "), "-c:a flac -compression_level 5");
        assert_eq!((info.summary.unwrap())(&g), "compression 5");
        let g = with(&Codec::WavPack.info(), &[]);
        assert_eq!(Codec::WavPack.audio_args(&g).join(" "), "-c:a wavpack -compression_level 2");
    }

    #[test]
    fn lyrics_go_where_each_container_keeps_them() {
        let tags = [("lyrics".to_owned(), "la".to_owned()), ("title".to_owned(), "T".to_owned())];
        let keys = |c: Codec| c.container_tags(&tags).0.into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(keys(Codec::Flac), ["TITLE", "LYRICS"]);
        assert_eq!(keys(Codec::Mp3), ["title", "lyrics"], "USLT");
        assert_eq!(keys(Codec::Alac), ["title", "lyrics"], "©lyr");
        assert_eq!(keys(Codec::WavPack), ["Title", "Lyrics"]);
        assert_eq!(keys(Codec::Wav), ["title"]);
    }

    #[test]
    fn what_arrives_is_used_without_asking() {
        // No switch for the cover, the tags or checking a FLAC: each is
        // done when it can be.
        for c in ALL {
            let keys: Vec<_> = c.info().params.iter().map(|p| p.key).collect();
            for gone in ["cover", "cover_size", "tags", "verify"] {
                assert!(!keys.contains(&gone), "{c:?} has {gone}");
            }
        }
        assert_eq!(Codec::Flac.cover_args().join(" "), "-map 0:a -map 1:v -c:v copy -disposition:v attached_pic -metadata:s:v comment=Cover (front)");
    }

    #[test]
    fn only_the_tags_given_are_written_and_the_cover_is_the_front() {
        let info = Codec::Flac.info();
        let g = with(&info, &[]);
        let tags = [("title".to_owned(), "Symphony no. 3, op. 36: I. Lento".to_owned())];
        let a = ffmpeg_args(Codec::Flac, std::path::Path::new("01.wav"), Some(std::path::Path::new("cover.jpg")), &g, &tags, "", std::path::Path::new("01.flac"));
        let a: Vec<String> = a.iter().map(|x| x.to_string_lossy().into_owned()).collect();
        let at = |x: &str| a.iter().position(|y| y == x).unwrap_or_else(|| panic!("no {x} in {a:?}"));
        assert_eq!(a[at("-map_metadata") + 1], "-1", "the WAV's own tags are dropped");
        assert_eq!(a[at("-metadata:s:v") + 1], "comment=Cover (front)");
        assert_eq!(a[at("-compression_level") + 1], "8");
        assert_eq!(a[at("-metadata") + 1], "TITLE=Symphony no. 3, op. 36: I. Lento", "one argument, colon and all");
        assert_eq!(a.last().unwrap(), "01.flac");
        assert!(!a.iter().any(|x| x == "-af" || x == "-ar" || x == "-sample_fmt"), "nothing changes the audio");
    }
}
