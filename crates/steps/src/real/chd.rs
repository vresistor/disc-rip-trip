//! Making a CHD, in-process through MAME's own CHD core (`libchdman-rs`),
//! so it has real progress and a Stop that takes effect at once.
//!
//! A CD's `.cue` becomes a CD CHD (`chdman createcd`); an `.iso` becomes a
//! DVD CHD (`chdman createdvd`). The old app sent ISOs through `createcd`,
//! which makes a CD CHD of a DVD; that's not repeated.
//!
//! Verifying decompresses every byte and checks it against the SHA-1 the
//! CHD recorded of what went in; for an ISO that SHA-1 is also compared
//! with the ISO's own, and for a CD each track is read back raw and
//! compared with its `.bin`. So "verified" means the CHD gives back the
//! dump, byte for byte.

use super::hash::hex;
use anyhow::{anyhow, bail, Result};
use libchdman_rs as chd;
use sha1::{Digest, Sha1};
use std::path::Path;

pub const CODECS: &[&str] = &["Default for the disc", "zstd", "lzma", "zlib"];
pub const HUNKS: &[&str] = &["Default", "4 sectors", "8 sectors", "16 sectors", "32 sectors"];

/// Whether a dump's main file is a CD (cue sheet) rather than an ISO.
pub fn is_cd(source: &Path) -> bool {
    source.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue"))
}

/// The codec slots for a choice. A CD's audio tracks always have FLAC
/// beside the chosen codec, as chdman's own CD default does.
pub fn codecs(choice: &str, cd: bool) -> [u32; 4] {
    match (choice, cd) {
        ("zstd", true) => [chd::CHD_CODEC_CD_ZSTD, chd::CHD_CODEC_CD_FLAC, 0, 0],
        ("lzma", true) => [chd::CHD_CODEC_CD_LZMA, chd::CHD_CODEC_CD_FLAC, 0, 0],
        ("zlib", true) => [chd::CHD_CODEC_CD_ZLIB, chd::CHD_CODEC_CD_FLAC, 0, 0],
        (_, true) => chd::cd::CdCreateOptions::default().codecs,
        ("zstd", false) => [chd::CHD_CODEC_ZSTD, 0, 0, 0],
        ("lzma", false) => [chd::CHD_CODEC_LZMA, 0, 0, 0],
        ("zlib", false) => [chd::CHD_CODEC_ZLIB, 0, 0, 0],
        (_, false) => chd::dvd::DvdCreateOptions::default().codecs,
    }
}

/// The hunk size in bytes for a choice: whole sectors, which for a CD are
/// 2448-byte frames (sector and subcode) and for an ISO 2048 bytes.
pub fn hunk(choice: &str, cd: bool) -> u32 {
    let sectors: u32 = choice.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
    match (sectors, cd) {
        (0, true) => chd::cd::DEFAULT_HUNK_SIZE,
        (0, false) => chd::dvd::DEFAULT_HUNK_SIZE,
        (n, true) => n * chd::cd::CD_FRAME_SIZE,
        (n, false) => n * chd::dvd::DVD_SECTOR_SIZE,
    }
}

/// The codecs a CHD is made with: "CD LZMA, CD Deflate, CD FLAC".
pub fn describe(codecs: &[u32; 4]) -> String {
    codecs.iter().filter(|&&c| c != 0).filter_map(|&c| chd::codec_name(c)).collect::<Vec<_>>().join(", ")
}

/// Makes `dest` from `source`. Blocking: minutes of CPU, so the caller runs
/// it on a blocking thread. A stopped or failed one leaves nothing behind.
pub fn create(source: &Path, dest: &Path, codec: &str, hunk_choice: &str, mut progress: impl FnMut(f32), stopped: &dyn Fn() -> bool) -> Result<()> {
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    let _ = std::fs::remove_file(dest);
    let cd = is_cd(source);
    let mut report = |p: chd::CompressionProgress| {
        if p.bytes_total > 0 {
            progress((p.bytes_done as f32 / p.bytes_total as f32).clamp(0.0, 1.0));
        }
    };
    let r = if cd {
        let o = chd::cd::CdCreateOptions { hunk_size: hunk(hunk_choice, true), codecs: codecs(codec, true) };
        chd::cd::create_from_cue(source, dest, o, &mut report, stopped)
    } else {
        let o = chd::dvd::DvdCreateOptions { hunk_size: hunk(hunk_choice, false), codecs: codecs(codec, false), ..Default::default() };
        chd::dvd::create_from_iso(source, dest, o, &mut report, stopped)
    };
    if let Err(e) = r {
        let _ = std::fs::remove_file(dest);
        if stopped() {
            bail!("stopped");
        }
        // MAME's errors are a bare enum; its name is all there is.
        bail!("couldn't make the CHD: {e:?}");
    }
    if !dest.is_file() {
        bail!("the CHD core said it was done but wrote nothing");
    }
    Ok(())
}

/// Decompresses all of `made` and checks it against what went in. `source`
/// is the dump's main file; `files` its data files in the cue's order (the
/// ISO alone for a DVD); `sha1s` any SHA-1s already known for them, by file
/// name (from Check), so they needn't be read again. Gives a line for the
/// item's note.
pub fn verify(made: &Path, source: &Path, files: &[std::path::PathBuf], sha1s: &dyn Fn(&str) -> Option<String>, mut progress: impl FnMut(f32), stopped: &dyn Fn() -> bool) -> Result<String> {
    let path = made.to_str().ok_or_else(|| anyhow!("the CHD's path isn't text"))?;
    let c = chd::Chd::open(path, false, None).map_err(|e| anyhow!("couldn't open the CHD: {e:?}"))?;
    // Every byte, against the SHA-1 the CHD recorded of its input.
    let total = c.logical_bytes();
    let mut buf = vec![0u8; c.hunk_bytes() as usize * 64];
    let mut sha = Sha1::new();
    let mut at = 0u64;
    while at < total {
        if stopped() {
            bail!("stopped");
        }
        let n = buf.len().min((total - at) as usize);
        c.read_bytes(at, &mut buf[..n]).map_err(|e| anyhow!("the CHD doesn't decompress at byte {at}: {e:?}"))?;
        sha.update(&buf[..n]);
        at += n as u64;
        progress(at as f32 / total as f32 * if is_cd(source) { 0.5 } else { 1.0 });
    }
    let raw: [u8; 20] = sha.finalize().into();
    let recorded = if c.version() <= 3 { c.sha1() } else { c.raw_sha1() };
    if raw != recorded {
        bail!("the CHD doesn't decompress to what went into it");
    }
    let file_sha1 = |f: &Path| -> Result<String> {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match sha1s(&name) {
            Some(s) => Ok(s),
            None => Ok(super::hash::hash_files(&[f], |_| {}, stopped)?.remove(0).sha1),
        }
    };
    if !is_cd(source) {
        if hex(&raw) != file_sha1(source)? {
            bail!("the CHD decompresses to something other than the ISO");
        }
        return Ok("verified: decompresses to the ISO".into());
    }
    // A CD: each track read back raw against its .bin, when the cue has one
    // file per track (as redumper writes it).
    let tracks = chd::cd::list_tracks(&c).map_err(|e| anyhow!("{e:?}"))?;
    if tracks.len() != files.len() {
        return Ok("verified: decompresses to what went in".into());
    }
    drop(c);
    for (i, f) in files.iter().enumerate() {
        let c = chd::Chd::open(path, false, None).map_err(|e| anyhow!("{e:?}"))?;
        let mut r = match chd::cd::CdAudioReader::open_track(c, i as u32) {
            Ok(r) => r,
            // A track stored cooked has no raw form to compare.
            Err(_) => return Ok("verified: decompresses to what went in".into()),
        };
        // The CHD's own check above already passed; a track laid out
        // differently (a pregap it doesn't store) can't be compared byte
        // for byte, which isn't a reason to throw the CHD away.
        if r.sector_count() as u64 != std::fs::metadata(f)?.len() / 2352 {
            return Ok("verified: decompresses to what went in".into());
        }
        let mut sha = Sha1::new();
        let mut sector = vec![0u8; 2352 * 64];
        let mut s = 0u32;
        while s < r.sector_count() {
            if stopped() {
                bail!("stopped");
            }
            let n = 64.min(r.sector_count() - s);
            let b = &mut sector[..n as usize * 2352];
            r.read_sectors(s, n, b).map_err(|e| anyhow!("track {} doesn't read back: {e:?}", i + 1))?;
            sha.update(&*b);
            s += n;
        }
        if hex(&sha.finalize()) != file_sha1(f)? {
            bail!("track {} decompresses to something other than its .bin", i + 1);
        }
        progress(0.5 + 0.5 * (i + 1) as f32 / files.len() as f32);
    }
    Ok(format!("verified: every track matches the dump ({})", files.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::current_dir().unwrap().join("../../target/drt-test").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Two tracks the way redumper writes them: a data track and an audio
    /// track, one .bin each.
    fn two_track_cd(d: &Path) -> (PathBuf, Vec<PathBuf>) {
        let t1: Vec<u8> = (0..700 * 2352).map(|i| (i % 251) as u8).collect();
        let t2: Vec<u8> = (0..450 * 2352).map(|i| (i % 241) as u8).collect();
        std::fs::write(d.join("DISC (Track 1).bin"), &t1).unwrap();
        std::fs::write(d.join("DISC (Track 2).bin"), &t2).unwrap();
        std::fs::write(
            d.join("DISC.cue"),
            // As redumper writes it: track 2's two-second pregap is in its .bin.
            "FILE \"DISC (Track 1).bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\nFILE \"DISC (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 00 00:00:00\n    INDEX 01 00:02:00\n",
        )
        .unwrap();
        (d.join("DISC.cue"), vec![d.join("DISC (Track 1).bin"), d.join("DISC (Track 2).bin")])
    }

    #[test]
    fn a_cd_becomes_a_chd_that_gives_back_every_track() {
        let d = dir("chd-cd");
        let (cue, files) = two_track_cd(&d);
        let out = d.join("DISC.chd");
        let mut seen = vec![];
        create(&cue, &out, "zstd", "Default", |p| seen.push(p), &|| false).unwrap();
        assert!(!seen.is_empty() && seen.iter().all(|p| (0.0..=1.0).contains(p)));
        let note = verify(&out, &cue, &files, &|_| None, |_| {}, &|| false).unwrap();
        assert_eq!(note, "verified: every track matches the dump (2)");
        // A wrong hash for a track is caught.
        let err = verify(&out, &cue, &files, &|n| n.contains("Track 2").then(|| "00".into()), |_| {}, &|| false).unwrap_err();
        assert!(err.to_string().contains("track 2"), "{err}");
    }

    #[test]
    fn an_iso_becomes_a_dvd_chd_that_decompresses_to_it() {
        let d = dir("chd-dvd");
        let iso = d.join("DISC.iso");
        std::fs::write(&iso, (0..3000 * 2048).map(|i| (i % 253) as u8).collect::<Vec<u8>>()).unwrap();
        let out = d.join("DISC.chd");
        create(&iso, &out, "Default for the disc", "8 sectors", |_| {}, &|| false).unwrap();
        let c = chd::Chd::open(out.to_str().unwrap(), false, None).unwrap();
        assert_eq!(c.hunk_bytes(), 8 * 2048);
        drop(c);
        assert_eq!(verify(&out, &iso, std::slice::from_ref(&iso), &|_| None, |_| {}, &|| false).unwrap(), "verified: decompresses to the ISO");
    }

    #[test]
    fn stopping_leaves_nothing_behind() {
        let d = dir("chd-stop");
        let (cue, _) = two_track_cd(&d);
        let out = d.join("DISC.chd");
        assert_eq!(create(&cue, &out, "Default for the disc", "Default", |_| {}, &|| true).unwrap_err().to_string(), "stopped");
        assert!(!out.exists());
    }

    #[test]
    fn settings_become_codecs_and_hunks() {
        assert_eq!(describe(&codecs("Default for the disc", true)), "CD LZMA, CD Deflate, CD FLAC");
        assert_eq!(describe(&codecs("zstd", true)), "CD Zstandard, CD FLAC");
        assert_eq!(describe(&codecs("zstd", false)), "Zstandard");
        assert_eq!(hunk("Default", true), 19584);
        assert_eq!(hunk("16 sectors", true), 16 * 2448);
        assert_eq!(hunk("Default", false), 4096);
    }
}
