//! Every encoder, run for real on a generated track: each lossless file
//! decodes to exactly the rip (and one that doesn't fails), and each
//! container's tags are where Picard reads them, read back from the file.
//!
//! Needs ffmpeg and ffprobe on the PATH (or `DRT_FFMPEG`); without them it
//! says so and passes.

use drt_steps::real::encode::{Codec, Job, ALL};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn have_ffmpeg() -> bool {
    std::process::Command::new("ffprobe").arg("-version").output().is_ok_and(|o| o.status.success())
        && drt_steps::real::tool::command("ffmpeg").as_std().get_program().to_str().is_some_and(|p| std::process::Command::new(p).arg("-version").output().is_ok())
}

fn base(name: &str) -> PathBuf {
    let b = std::env::current_dir().unwrap().join("../../target/drt-test").join(name);
    let _ = std::fs::remove_dir_all(&b);
    std::fs::create_dir_all(&b).unwrap();
    b
}

/// A second of stereo noise, as cyanrip writes it: with a LIST INFO chunk
/// carrying its own comment, which must never reach the files.
fn wav(path: &Path) {
    let mut pcm = Vec::new();
    let mut x: u32 = 12345;
    for _ in 0..44_100 * 2 {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        pcm.extend(((x >> 16) as i16).to_le_bytes());
    }
    let info = b"INFOICMT\x0e\0\0\0cyanrip 0.9.3\0";
    let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    b.extend([16, 0, 0, 0, 1, 0, 2, 0, 0x44, 0xAC, 0, 0, 0x10, 0xB1, 2, 0, 4, 0, 16, 0]);
    b.extend(b"LIST");
    b.extend((info.len() as u32).to_le_bytes());
    b.extend(info);
    b.extend(b"data");
    b.extend((pcm.len() as u32).to_le_bytes());
    b.extend(pcm);
    let n = (b.len() - 8) as u32;
    b[4..8].copy_from_slice(&n.to_le_bytes());
    std::fs::write(path, b).unwrap();
}

fn tags() -> Vec<(String, String)> {
    [
        ("title", "Total Football"),
        ("artist", "Parquet Courts"),
        ("album", "Wide Awake!"),
        ("album_artist", "Parquet Courts"),
        ("date", "2018-05-18"),
        ("tracknumber", "1"),
        ("tracktotal", "13"),
        ("discnumber", "1"),
        ("disctotal", "1"),
        ("label", "Rough Trade"),
        ("catalognumber", "RT0001CD"),
        ("barcode", "191402000123"),
        ("isrc", "GBCEL1800001"),
        ("musicbrainz_albumid", "rel-id"),
        ("musicbrainz_releasegroupid", "rg-id"),
        ("musicbrainz_releasetrackid", "track-id"),
        ("musicbrainz_recordingid", "rec-id"),
        ("preemphasis", "1"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// Every tag ffprobe sees, file and streams together, keys lowercased.
fn probe(file: &Path) -> BTreeMap<String, String> {
    let o = std::process::Command::new("ffprobe").args(["-v", "error", "-show_entries", "format_tags:stream_tags", "-of", "default=nw=1"]).arg(file).output().unwrap();
    String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.strip_prefix("TAG:")).filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_lowercase(), v.to_owned())).collect()
}

fn defaults(c: Codec) -> impl Fn(&str) -> Option<serde_json::Value> {
    let info = c.info();
    move |k: &str| info.params.iter().find(|p| p.key == k).and_then(|p| p.default.clone())
}

#[tokio::test]
async fn every_lossless_file_decodes_to_the_rip_and_the_tags_are_where_picard_reads_them() {
    if !have_ffmpeg() {
        eprintln!("no ffmpeg: skipped");
        return;
    }
    let b = base("encode-tags");
    let src = b.join("01.wav");
    wav(&src);
    // A cover, as Cover art leaves one: every MP3 and M4A has it.
    let cover = b.join("cover.jpg");
    let ff = drt_steps::real::tool::command("ffmpeg").as_std().get_program().to_owned();
    assert!(std::process::Command::new(&ff).args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=red:s=32x32", "-frames:v", "1"]).arg(&cover).status().unwrap().success());
    for c in ALL {
        let out = b.join(format!("01-{}.{}", c.kind(), c.ext()));
        let job = Job::new(c, &src, matches!(c, Codec::Flac | Codec::Mp3 | Codec::Aac | Codec::Alac).then_some(&cover).map(PathBuf::as_path), &defaults(c), &tags(), "", &out);
        let verified = job.run(std::future::pending, 1.0, |_| {}).await.unwrap_or_else(|e| panic!("{c:?}: {e}"));
        assert_eq!(verified, c.lossless(), "{c:?}");
        let t = probe(&out);
        assert!(!t.values().any(|v| v.starts_with("cyanrip")), "{c:?} carries the rip's own tags: {t:?}");
        let want: &[(&str, &str)] = match c {
            Codec::Flac | Codec::Opus | Codec::Vorbis => &[
                ("track", "1"),
                ("tracktotal", "13"),
                ("disc", "1"),
                ("disctotal", "1"),
                ("isrc", "GBCEL1800001"),
                ("musicbrainz_albumid", "rel-id"),
                ("musicbrainz_releasetrackid", "track-id"),
                ("musicbrainz_trackid", "rec-id"),
                ("preemphasis", "1"),
            ],
            // TRCK and TPOS as ffprobe names them; TSRC and TPUB are their own.
            Codec::Mp3 => &[
                ("track", "1/13"),
                ("disc", "1/1"),
                ("tsrc", "GBCEL1800001"),
                ("publisher", "Rough Trade"),
                ("musicbrainz album id", "rel-id"),
                ("musicbrainz release track id", "track-id"),
                ("preemphasis", "1"),
            ],
            Codec::Aac | Codec::Alac => &[("track", "1/13"), ("disc", "1/1")],
            Codec::WavPack => &[("track", "1/13"), ("disc", "1/1"), ("isrc", "GBCEL1800001"), ("musicbrainz_trackid", "rec-id"), ("preemphasis", "1")],
            Codec::Wav => &[("title", "Total Football"), ("track", "1")],
        };
        for (k, v) in want {
            assert_eq!(t.get(*k).map(String::as_str), Some(*v), "{c:?} {k}: {t:?}");
        }
        if c == Codec::Mp3 {
            assert!(!t.contains_key("isrc"), "the ISRC is in TSRC, not a TXXX: {t:?}");
            assert_eq!(t.get("date").map(String::as_str), Some("2018-05-18"), "TYER and TDAT kept through the tag pass: {t:?}");
        }
        if matches!(c, Codec::Flac | Codec::Mp3 | Codec::Alac | Codec::Aac) {
            let streams =
                std::process::Command::new("ffprobe").args(["-v", "error", "-show_entries", "stream=codec_type:stream_disposition=attached_pic", "-of", "compact"]).arg(&out).output().unwrap();
            assert!(String::from_utf8_lossy(&streams.stdout).contains("video|disposition:attached_pic=1"), "{c:?}: the cover survives the tag pass");
        }
        // The tag pass, done again, changes nothing: neither the audio nor
        // any tag ffprobe sees.
        if matches!(c, Codec::Mp3 | Codec::Aac | Codec::Alac) {
            let md5 = |f: &Path| {
                String::from_utf8_lossy(&std::process::Command::new(&ff).args(["-v", "error", "-i"]).arg(f).args(["-map", "0:a", "-c:a", "pcm_s16le", "-f", "md5", "-"]).output().unwrap().stdout)
                    .into_owned()
            };
            let (before, tags_before) = (md5(&out), probe(&out));
            drt_steps::real::encode::finish_tags(c, &out, &c.container_tags(&tags()).1, true).unwrap();
            assert_eq!(md5(&out), before, "{c:?}: the audio is untouched by the tag pass");
            assert_eq!(probe(&out), tags_before, "{c:?}");
        }
        if c == Codec::Wav {
            assert!(!t.contains_key("isrc"), "INFO's ISRC is a source, not a recording code: {t:?}");
        }
    }
    // What ffprobe can't show: the recording in MP3's UFID frame, and the
    // MP4 freeform atoms.
    use lofty::prelude::*;
    let mp3 = std::fs::read(b.join("01-mp3.mp3")).unwrap();
    assert_eq!(&mp3[3..4], &[3], "still ID3v2.3");
    let at = mp3.windows(4).position(|w| w == b"UFID").expect("a UFID frame");
    assert!(mp3[at..at + 60].windows(30).any(|w| w == b"http://musicbrainz.org\0rec-id\0" as &[u8] || w.starts_with(b"http://musicbrainz.org\0rec-id")), "UFID owner and recording");
    for m4a in ["01-alac.m4a", "01-aac.m4a"] {
        let mut f = std::fs::File::open(b.join(m4a)).unwrap();
        let file = lofty::mp4::Mp4File::read_from(&mut f, lofty::config::ParseOptions::new().implicit_conversions(false)).unwrap();
        let ilst = file.ilst().unwrap();
        let free = |name: &str| {
            let id = lofty::mp4::AtomIdent::Freeform { mean: "com.apple.iTunes".into(), name: name.to_owned().into() };
            ilst.get(&id).and_then(|a| a.data().next().cloned())
        };
        for (n, v) in [("MusicBrainz Album Id", "rel-id"), ("MusicBrainz Track Id", "rec-id"), ("MusicBrainz Release Track Id", "track-id"), ("ISRC", "GBCEL1800001"), ("PREEMPHASIS", "1")] {
            assert_eq!(free(n), Some(lofty::mp4::AtomData::UTF8(v.into())), "{m4a} {n}");
        }
    }
    std::fs::remove_dir_all(b).unwrap();
}

#[tokio::test]
async fn a_lossless_file_that_doesnt_decode_to_the_rip_fails() {
    if !have_ffmpeg() {
        eprintln!("no ffmpeg: skipped");
        return;
    }
    let b = base("encode-mismatch");
    let src = b.join("01.wav");
    wav(&src);
    for c in [Codec::Flac, Codec::Alac, Codec::WavPack, Codec::Wav] {
        let out = b.join(format!("out.{}", c.ext()));
        // Anything that changes a sample: the check must catch it.
        let job = Job::new(c, &src, None, &defaults(c), &[], "-af volume=0.5", &out);
        let e = job.run(std::future::pending, 1.0, |_| {}).await.expect_err("a changed file passed");
        assert!(e.to_string().contains("doesn't decode to the rip"), "{c:?}: {e}");
    }
    std::fs::remove_dir_all(b).unwrap();
}
