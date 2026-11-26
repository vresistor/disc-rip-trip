//! ReplayGain: how loud each track is, and the album as a whole, measured
//! with ffmpeg's `ebur128` filter (ReplayGain 2.0 is EBU R128 loudness
//! against −18 LUFS), and written as tags without re-encoding (`-c copy`).
//!
//! Opus is the exception: its own standard (RFC 7845) is `R128_TRACK_GAIN`
//! and `R128_ALBUM_GAIN`, a whole number of 1/256 dB against −23 LUFS, and
//! players that know Opus read those, not `REPLAYGAIN_*`.
//!
//! Waits for every file of the album, since album gain is of all of them.
//! The tagged copies go in its own folder: a file that came from an encoder
//! may be on its way somewhere else too, so it's never changed in place.
//! Checked with the bundled ffmpeg (8): FLAC, Vorbis, Opus, MP3 (kept at
//! its ID3 version), M4A and WavPack all keep the tags through `-c copy`;
//! WAV can't hold them and passes through untouched.

use super::tool;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use trip_engine::setting::ParamKind;
use trip_engine::{Cx, Group, Inlet, Item, ItemKind, ItemState, LaneKind, Outlet, ParamSpec, Step, StepInfo};

pub const MODES: &[&str] = &["Track and album", "Track only"];

pub fn info() -> StepInfo {
    StepInfo::new("replaygain", "ReplayGain", Group::Make, ItemKind::AudioFile, ItemKind::AudioFile)
        .from("even out volume between tracks")
        .param(
            ParamSpec::new("mode", "Even out volume", ParamKind::Choice(MODES))
                .default(MODES[0])
                .labels(&["Per track and per album", "Per track only"])
                .help("Players do the evening out. The sound in the files isn't changed.")
                .on_card_as("Gain", &[], |g| if g("mode").and_then(|v| v.as_str().map(str::to_owned)).as_deref() == Some(MODES[1]) { "track".into() } else { "track + album".into() }),
        )
        .summary(|g| if g("mode").and_then(|v| v.as_str().map(str::to_owned)).as_deref() == Some(MODES[1]) { "track".into() } else { "track and album".into() })
}

/// A gain as the card says it: *−7.3 dB*, with a true minus.
pub fn db(g: f64) -> String {
    format!("{g:+.1} dB").replacen('-', "\u{2212}", 1)
}

/// ReplayGain's outcome: the album's gain (or, track gain only, how many),
/// and the tracks' spread as the one fact.
pub fn gain_says(cx: &Cx, album: Option<f64>, tracks: &[f64]) {
    let lo = tracks.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = tracks.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let spread = if tracks.is_empty() { String::new() } else { format!("tracks {} to {}", db(lo).trim_end_matches(" dB"), db(hi)) };
    match album {
        Some(a) => cx.status(db(a), spread),
        None if tracks.is_empty() => cx.status("Done", ""),
        None => cx.status(format!("{} tracks", tracks.len()), spread),
    }
}

/// Loudness in LUFS and sample peak in dBFS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    pub lufs: f64,
    pub peak_db: f64,
}

/// Reads the summary `ebur128` prints at the end: the integrated loudness
/// (`I:`) and the sample peak (`Peak:` under `Sample peak:`). The lines it
/// prints while running also have an `I:`, so only what follows `Summary:`
/// counts.
pub fn parse_summary(text: &str) -> Option<Loudness> {
    let summary = &text[text.rfind("Summary:")?..];
    let num = |l: &str, key: &str| l.trim().strip_prefix(key)?.split_whitespace().next()?.parse::<f64>().ok();
    let lufs = summary.lines().find_map(|l| num(l, "I:"))?;
    let after = &summary[summary.find("Sample peak:")?..];
    let peak_db = after.lines().find_map(|l| num(l, "Peak:"))?;
    Some(Loudness { lufs, peak_db })
}

/// The tags for one file: its own gain and peak, and the album's.
pub fn tags(ext: &str, track: Loudness, album: Option<Loudness>) -> Vec<(String, String)> {
    let mut out = vec![];
    if ext.eq_ignore_ascii_case("opus") {
        // Q7.8 against −23 LUFS; there's no peak tag in Opus's scheme.
        let q = |l: Loudness| (((-23.0 - l.lufs) * 256.0).round() as i64).clamp(-32768, 32767).to_string();
        out.push(("R128_TRACK_GAIN".into(), q(track)));
        if let Some(a) = album {
            out.push(("R128_ALBUM_GAIN".into(), q(a)));
        }
        return out;
    }
    let gain = |l: Loudness| format!("{:+.2} dB", -18.0 - l.lufs);
    let peak = |l: Loudness| format!("{:.6}", 10f64.powf(l.peak_db / 20.0));
    out.push(("REPLAYGAIN_TRACK_GAIN".into(), gain(track)));
    out.push(("REPLAYGAIN_TRACK_PEAK".into(), peak(track)));
    if let Some(a) = album {
        out.push(("REPLAYGAIN_ALBUM_GAIN".into(), gain(a)));
        out.push(("REPLAYGAIN_ALBUM_PEAK".into(), peak(a)));
    }
    out
}

/// Whether a container can hold the tags.
pub fn taggable(ext: &str) -> bool {
    !ext.eq_ignore_ascii_case("wav")
}

/// What the remux needs to keep a file as it was: an MP3's ID3v2 version
/// (ffmpeg would write 2.4) and its ID3v1 tag; an M4A's freeform tags.
pub fn container_args(ext: &str, head: &[u8], tail: &[u8]) -> Vec<String> {
    match ext.to_ascii_lowercase().as_str() {
        "mp3" => {
            let mut a = vec![];
            if head.len() >= 4 && &head[..3] == b"ID3" && matches!(head[3], 3 | 4) {
                a.extend(["-id3v2_version".to_owned(), head[3].to_string()]);
            }
            if tail.len() >= 128 && &tail[tail.len() - 128..tail.len() - 125] == b"TAG" {
                a.extend(["-write_id3v1".to_owned(), "1".to_owned()]);
            }
            a
        }
        "m4a" | "mp4" => vec!["-movflags".into(), "use_metadata_tags".into()],
        _ => vec![],
    }
}

/// Measures the files together (one after another, as one programme) or
/// one alone.
pub async fn measure(files: &[&Path], stop: impl std::future::Future<Output = ()>) -> Result<Loudness> {
    let mut cmd = tool::command("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-nostats"]);
    for f in files {
        cmd.arg("-i").arg(f);
    }
    if files.len() == 1 {
        cmd.args(["-map", "0:a", "-af", "ebur128=peak=sample"]);
    } else {
        let inputs: String = (0..files.len()).map(|i| format!("[{i}:a]")).collect();
        cmd.args(["-filter_complex", &format!("{inputs}concat=n={}:v=0:a=1,ebur128=peak=sample", files.len())]);
    }
    cmd.args(["-f", "null", "-"]);
    let dir = files[0].parent().unwrap_or(Path::new("."));
    let (text, status) = tool::run(cmd, dir, stop, |_| {}).await?;
    if !status.success() {
        bail!("ffmpeg: {}", tool::last_meaningful_line(&text).unwrap_or("failed"));
    }
    parse_summary(&text).ok_or_else(|| anyhow!("ffmpeg gave no loudness"))
}

/// Copies `src` to `dest` with the tags added, the audio untouched.
pub async fn write(src: &Path, dest: &Path, tags: &[(String, String)], stop: impl std::future::Future<Output = ()>) -> Result<()> {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    let bytes = std::fs::read(src)?;
    let mut cmd = tool::command("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(src).args(["-map", "0", "-c", "copy"]);
    cmd.args(container_args(ext, &bytes[..bytes.len().min(10)], &bytes[bytes.len().saturating_sub(128)..]));
    for (k, v) in tags {
        cmd.arg("-metadata").arg(format!("{k}={v}"));
    }
    cmd.arg(tool::path_arg(dest));
    let (text, status) = tool::run(cmd, dest.parent().unwrap_or(Path::new(".")), stop, |_| {}).await?;
    if !status.success() {
        bail!("ffmpeg: {}", tool::last_meaningful_line(&text).unwrap_or("failed"));
    }
    Ok(())
}

pub struct ReplayGain;

#[async_trait]
impl Step for ReplayGain {
    fn info(&self) -> StepInfo {
        info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let mut files: Vec<Item> = vec![];
        while let Some(f) = inlet.recv().await {
            cx.status("", format!("Waits for all {}, {} so far", super::parts(f.context()), files.len() + 1));
            files.push(f);
        }
        if files.is_empty() || cx.stopped() {
            return Ok(());
        }
        let album_too = cx.param("mode").and_then(|v| v.as_str().map(str::to_owned)).as_deref() != Some(MODES[1]);
        let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path.clone()).collect();
        let _core = cx.lane(LaneKind::Encode).await;
        let album = if album_too {
            cx.status("", "Measuring the album");
            let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
            match measure(&refs, cx.cancelled()).await {
                Ok(l) => Some(l),
                Err(e) if cx.stopped() => return Err(e),
                Err(e) => {
                    cx.log(format!("album: {e}"));
                    None
                }
            }
        } else {
            None
        };
        let dir = cx.work()?.join("replaygain");
        std::fs::create_dir_all(&dir)?;
        let total = files.len();
        let mut gains: Vec<f64> = vec![];
        for (i, f) in files.iter().enumerate() {
            cx.status("", super::track_line(f.index.unwrap_or(i as u32 + 1), &f.label));
            cx.progress(Some(i as f32 / total as f32));
            let Some(src) = f.path.clone() else { continue };
            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_owned();
            if !taggable(&ext) {
                cx.item(f, ItemState::Ok, "WAV can't store volume info; left as is");
                outlet.send(f.clone()).await;
                continue;
            }
            cx.item(f, ItemState::Busy, "measuring");
            let done: Result<(PathBuf, Loudness)> = async {
                let track = measure(&[src.as_path()], cx.cancelled()).await?;
                let dest = dir.join(src.file_name().ok_or_else(|| anyhow!("no file name"))?);
                write(&src, &dest, &tags(&ext, track, album), cx.cancelled()).await?;
                Ok((dest, track))
            }
            .await;
            match done {
                Ok((dest, track)) => {
                    gains.push(-18.0 - track.lufs);
                    cx.item(f, ItemState::Ok, format!("{:+.2} dB", -18.0 - track.lufs));
                    outlet.send(f.becomes(ItemKind::AudioFile, Some(dest))).await;
                }
                Err(e) if cx.stopped() => return Err(e),
                Err(e) => {
                    // The file still goes on, untagged: loudness is a nicety.
                    cx.log(format!("{}: {e}", f.label));
                    cx.item(f, ItemState::Ok, format!("untagged: {e}"));
                    outlet.send(f.clone()).await;
                }
            }
        }
        cx.progress(None);
        let _ = total;
        gain_says(&cx, album.map(|a| -18.0 - a.lufs), &gains);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The end of `ffmpeg … -af ebur128=peak=sample -f null -` from the
    /// bundled ffmpeg on a 440 Hz tone, with a running line before it.
    const SUMMARY: &str = "[Parsed_ebur128_0 @ 0x55] t: 1.9 TARGET:-23 LUFS    M: -21.7 S: -21.7     I: -21.9 LUFS       LRA:   0.0 LU  SPK: -21.0 dBFS\n[Parsed_ebur128_0 @ 0x55] Summary:\n\n  Integrated loudness:\n    I:         -21.7 LUFS\n    Threshold: -31.7 LUFS\n\n  Loudness range:\n    LRA:         0.0 LU\n    Threshold:   0.0 LUFS\n    LRA low:     0.0 LUFS\n    LRA high:    0.0 LUFS\n\n  Sample peak:\n    Peak:      -21.0 dBFS\n[out#0/null @ 0x55] video:0KiB audio:375KiB\n";

    #[test]
    fn the_summary_not_the_running_lines() {
        assert_eq!(parse_summary(SUMMARY), Some(Loudness { lufs: -21.7, peak_db: -21.0 }));
        assert_eq!(parse_summary("I: -10 LUFS\n"), None, "no summary, no answer");
    }

    #[test]
    fn replaygain_and_opus_tags() {
        let t = Loudness { lufs: -14.79, peak_db: -1.0 };
        let a = Loudness { lufs: -15.0, peak_db: -0.5 };
        assert_eq!(
            tags("flac", t, Some(a)),
            [
                ("REPLAYGAIN_TRACK_GAIN".to_owned(), "-3.21 dB".to_owned()),
                ("REPLAYGAIN_TRACK_PEAK".to_owned(), "0.891251".to_owned()),
                ("REPLAYGAIN_ALBUM_GAIN".to_owned(), "-3.00 dB".to_owned()),
                ("REPLAYGAIN_ALBUM_PEAK".to_owned(), "0.944061".to_owned()),
            ]
        );
        assert_eq!(tags("mp3", Loudness { lufs: -20.0, peak_db: -6.0 }, None)[0].1, "+2.00 dB");
        // Opus: (−23 − −14.79) × 256 = −2101.76 → −2102.
        assert_eq!(tags("opus", t, Some(a)), [("R128_TRACK_GAIN".to_owned(), "-2102".to_owned()), ("R128_ALBUM_GAIN".to_owned(), "-2048".to_owned())]);
        assert!(!taggable("WAV") && taggable("wv"));
    }

    #[test]
    fn a_remux_keeps_an_mp3_as_it_was() {
        let mut tail = vec![0u8; 200];
        tail[72..75].copy_from_slice(b"TAG");
        assert_eq!(container_args("mp3", b"ID3\x03\x00", &tail), ["-id3v2_version", "3", "-write_id3v1", "1"]);
        assert_eq!(container_args("mp3", b"ID3\x04\x00", &[0; 128]), ["-id3v2_version", "4"]);
        assert_eq!(container_args("m4a", b"", b""), ["-movflags", "use_metadata_tags"]);
        assert!(container_args("flac", b"fLaC", b"").is_empty());
    }

    /// Measures and tags real files with the real ffmpeg (`TRIP_FFMPEG`),
    /// then reads the tags back with ffprobe (`TRIP_FFPROBE`).
    #[tokio::test]
    #[ignore]
    async fn real_ffmpeg() {
        let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/trip/test-replaygain");
        std::fs::create_dir_all(dir.join("out")).unwrap();
        let mut made = vec![];
        for (i, ext, codec) in [(1, "mp3", "libmp3lame"), (2, "opus", "libopus"), (3, "flac", "flac")] {
            let f = dir.join(format!("{i:02}.{ext}"));
            let mut cmd = tool::command("ffmpeg");
            cmd.args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("sine=frequency={}:duration=2", 300 * i), "-ac", "2", "-c:a", codec]);
            if ext == "mp3" {
                cmd.args(["-id3v2_version", "3"]);
            }
            cmd.arg(&f);
            assert!(tool::run(cmd, &dir, std::future::pending(), |_| {}).await.unwrap().1.success());
            made.push(f);
        }
        let refs: Vec<&Path> = made.iter().map(PathBuf::as_path).collect();
        let album = measure(&refs, std::future::pending()).await.unwrap();
        for f in &made {
            let ext = f.extension().unwrap().to_str().unwrap();
            let track = measure(&[f.as_path()], std::future::pending()).await.unwrap();
            let out = dir.join("out").join(f.file_name().unwrap());
            write(f, &out, &tags(ext, track, Some(album)), std::future::pending()).await.unwrap();
            let mut probe = tool::command("ffprobe");
            probe.args(["-v", "error", "-show_entries", "format_tags:stream_tags", "-of", "compact"]).arg(&out);
            let (text, _) = tool::run(probe, &dir, std::future::pending(), |_| {}).await.unwrap();
            println!("{ext}: {track:?} → {}", text.trim());
            let want = if ext == "opus" { "R128_ALBUM_GAIN" } else { "REPLAYGAIN_ALBUM_GAIN" };
            assert!(text.contains(want), "{ext}: {text}");
            if ext == "mp3" {
                assert_eq!(&std::fs::read(&out).unwrap()[..4], b"ID3\x03", "still ID3v2.3");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
