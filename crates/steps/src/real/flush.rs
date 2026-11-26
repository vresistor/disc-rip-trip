//! Clearing the drive's cache before a re-read that will be compared.
//!
//! Some drives keep the audio they've just read and hand it back on the
//! next read of the same sectors without going to the disc, which would
//! make two reads agree whatever the disc holds. So before each re-read,
//! Rip reads 2,400 sectors (5.6 MB, twice paranoia's model of a cache)
//! from somewhere else on the disc and throws them away: whatever the
//! drive held of the track is gone, and the re-read has to come off the
//! disc. See docs/audio-accuracy.md, *Re-reads and the drive's cache*.

use super::gnudb::{parse_toc, Toc};

/// How many sectors are read elsewhere: 5.6 MB, 32 s of audio.
pub const SECTORS: u32 = 2400;

/// The least distance, in sectors, between what's read and both ends of
/// the track to be re-read: its start, where the re-read begins, and its
/// end, where the last read stopped and the drive may hold what it read
/// ahead. 600 sectors is 8 s of audio, over three times the cache and
/// read-ahead measured on the one drive tried (184 sectors). A disc too
/// short to read that far away can't be cleared.
pub const LEAST_AWAY: u32 = 600;

/// What to read to clear the cache: `count` sectors from `lba`, `away`
/// sectors from the nearer end of the track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub lba: u32,
    pub count: u32,
    pub away: u32,
}

/// Where track `t` is on the disc, as LBAs `[start, end)`. Track 0 is the
/// hidden track, from the disc's first sector to track 1. The TOC is
/// MusicBrainz's, whose sectors count from 150 before LBA 0.
pub fn track_span(toc: &Toc, t: u32) -> Option<(u32, u32)> {
    let lba = |s: u32| s.saturating_sub(150);
    let end_of_disc = lba(toc.leadout);
    if t == 0 {
        return Some((0, lba(*toc.offsets.first()?)));
    }
    let i = t.checked_sub(1)? as usize;
    let start = lba(*toc.offsets.get(i)?);
    let end = toc.offsets.get(i + 1).map_or(end_of_disc, |&s| lba(s));
    (start < end).then_some((start, end))
}

/// What to read before re-reading track `t`: [`SECTORS`] of them (fewer
/// on a disc smaller than that), within the disc's audio: [`SECTORS`] after
/// the track if there's room, else as far before it; failing both, the
/// farthest from both ends of the track of those two and its middle (a
/// disc of one track, or a short one). `None` when the disc is too short
/// for anywhere [`LEAST_AWAY`] from both.
pub fn window(toc: &str, t: u32) -> Option<Window> {
    let toc = parse_toc(toc)?;
    let (s, e) = track_span(&toc, t)?;
    let extent = toc.leadout.saturating_sub(150);
    let n = SECTORS.min(extent);
    if n == 0 {
        return None;
    }
    let last = extent - n;
    // How far a window is from a point: 0 if the point is in it.
    let from = |w: u32, p: u32| if p < w { w - p } else { p.saturating_sub(w + n) };
    let away = |w: u32| from(w, s).min(from(w, e));
    let after = (e + SECTORS).min(last);
    let before = s.saturating_sub(SECTORS + n).min(last);
    let middle = ((s + e) / 2).saturating_sub(n / 2).min(last);
    let candidates = [after, before, middle];
    // Outside the track and the full distance away, after first; else the
    // farthest, the first of them on a tie.
    let best = match candidates.into_iter().take(2).find(|&w| away(w) >= SECTORS) {
        Some(w) => w,
        None => candidates.into_iter().fold(after, |b, w| if away(w) > away(b) { w } else { b }),
    };
    (away(best) >= LEAST_AWAY).then_some(Window { lba: best, count: n, away: away(best) })
}

/// Which tracks a window lies in, as the log says it: *track 9*, *tracks 9
/// to 10*, or *the gap before track 1*.
pub fn tracks_in(toc: &str, w: &Window) -> String {
    let Some(toc) = parse_toc(toc) else { return "the disc".into() };
    let track_of = |lba: u32| (0..=toc.offsets.len() as u32).rev().find(|&t| track_span(&toc, t).is_some_and(|(s, _)| s <= lba)).unwrap_or(0);
    let (a, b) = (track_of(w.lba), track_of(w.lba + w.count - 1));
    let name = |t: u32| if t == 0 { "the gap before track 1".to_owned() } else { format!("track {t}") };
    match (a, b) {
        (a, b) if a == b => name(a),
        (0, b) => format!("the gap before track 1 to track {b}"),
        (a, b) => format!("tracks {a} to {b}"),
    }
}

/// `2,400`: thousands with commas, as the log writes counts.
pub fn thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Reads `count` sectors from `lba` and throws them away, with MMC READ CD
/// (0xBE) through SG_IO, 26 sectors at a time (cd-paranoia's own size for
/// SG reads, well under any transfer limit). Any sector type, user data
/// only: 2,352 bytes a sector of audio. Read-only: nothing is written and
/// the tray stays as it is.
#[cfg(target_os = "linux")]
pub fn read_sectors(device: &str, lba: u32, count: u32) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    const AT_ONCE: u32 = 26;
    const RAW: usize = 2352;
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(device)?;
    let mut buf = vec![0u8; AT_ONCE as usize * RAW];
    let mut at = lba;
    let end = lba + count;
    while at < end {
        let n = AT_ONCE.min(end - at);
        let len = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        let a = at.to_be_bytes();
        // READ CD: expected sector type 0 (any), the LBA, the length in
        // sectors, 0x10 (user data), no subchannel.
        let cdb: [u8; 12] = [0xBE, 0, a[0], a[1], a[2], a[3], len[0], len[1], len[2], 0x10, 0, 0];
        let data = buf.get_mut(..n as usize * RAW).unwrap_or_default();
        let got = super::media::sg_read(f.as_raw_fd(), &cdb, data, 30_000).map_err(|e| std::io::Error::new(e.kind(), format!("sector {at}: {e}")))?;
        if got == 0 {
            return Err(std::io::Error::other(format!("sector {at}: the drive gave nothing back")));
        }
        at += n;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn read_sectors(_device: &str, _lba: u32, _count: u32) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "only on Linux"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real disc of 32 tracks, 51 minutes (230947 sectors with the 150).
    const DISC: &str = "1 32 230947 150 14072 19350 23052 29857 33640 36422 39435 45147 49187 53637 58460 62485 69827 81695 86530 109110 116502 120560 125285 130030 133772 143730 148485 152815 160622 187862 191752 197852 202607 207255 214002";

    fn span(toc: &str, t: u32) -> (u32, u32) {
        track_span(&parse_toc(toc).unwrap(), t).unwrap()
    }

    #[test]
    fn the_first_track_is_cleared_from_after_it() {
        let w = window(DISC, 1).unwrap();
        let (_, e) = span(DISC, 1);
        assert_eq!(w, Window { lba: e + SECTORS, count: SECTORS, away: SECTORS });
        assert_eq!(tracks_in(DISC, &w), "track 2");
    }

    #[test]
    fn a_middle_track_is_cleared_from_after_it() {
        let w = window(DISC, 16).unwrap();
        let (_, e) = span(DISC, 16);
        assert_eq!((w.lba, w.count), (e + SECTORS, SECTORS));
        assert_eq!(tracks_in(DISC, &w), "track 17");
    }

    #[test]
    fn the_last_track_is_cleared_from_before_it_within_the_disc() {
        let w = window(DISC, 32).unwrap();
        let (s, e) = span(DISC, 32);
        assert_eq!(e, 230947 - 150, "the last track ends at the lead-out");
        assert_eq!(w, Window { lba: s - SECTORS - SECTORS, count: SECTORS, away: SECTORS });
        assert!(w.lba + w.count <= e);
        assert_eq!(tracks_in(DISC, &w), "track 31");
        // A first track with little after it: as far after as the disc goes.
        let toc = "1 2 6150 150 3150";
        assert_eq!(window(toc, 1), Some(Window { lba: 3600, count: SECTORS, away: 600 }), "clamped to the disc's end");
    }

    #[test]
    fn the_hidden_track_is_cleared_from_well_after_it() {
        let toc = "1 2 60150 3150 30150";
        assert_eq!(span(toc, 0), (0, 3000));
        let w = window(toc, 0).unwrap();
        assert_eq!(w, Window { lba: 3000 + SECTORS, count: SECTORS, away: SECTORS });
        assert_eq!(tracks_in(toc, &w), "track 1");
    }

    #[test]
    fn a_disc_of_one_long_track_is_cleared_from_its_middle() {
        let toc = "1 1 135150 150";
        let w = window(toc, 1).unwrap();
        assert_eq!(w.lba, 135000 / 2 - SECTORS / 2);
        assert_eq!(w.away, 135000 / 2 - SECTORS / 2);
    }

    #[test]
    fn a_short_disc_is_read_as_far_away_as_it_allows_or_not_at_all() {
        // Two tracks, 3000 and 1500 sectors (1 minute): track 2 is cleared
        // from the start of the disc, 600 sectors from it.
        let toc = "1 2 4650 150 3150";
        assert_eq!(window(toc, 2), Some(Window { lba: 0, count: SECTORS, away: 600 }));
        assert_eq!(tracks_in(toc, &window(toc, 2).unwrap()), "track 1");
        // Track 1: anywhere is too close to one of its ends.
        assert_eq!(window(toc, 1), None);
        // Smaller than one window: never far enough.
        assert_eq!(window("1 2 400 160 260", 0), None);
        assert_eq!(window("1 1 150 150", 1), None, "a disc of nothing");
        assert_eq!(window("garbage", 1), None);
        assert_eq!(window(DISC, 33), None, "no such track");
    }

    #[test]
    fn counts_are_written_with_commas() {
        assert_eq!(thousands(2400), "2,400");
        assert_eq!(thousands(600), "600");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    /// Clears the cache on the drive with whatever disc is in, read-only.
    /// Run by hand: `TRIP_TOC='1 2 …' cargo test -p trip-steps real_flush --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_flush() {
        let dev = std::env::var("TRIP_DEVICE").unwrap_or_else(|_| "/dev/sr0".into());
        let toc = std::env::var("TRIP_TOC").expect("TRIP_TOC, the disc's MusicBrainz TOC");
        let t: u32 = std::env::var("TRIP_TRACK").ok().and_then(|t| t.parse().ok()).unwrap_or(1);
        let w = window(&toc, t).expect("somewhere to read");
        let at = std::time::Instant::now();
        let r = read_sectors(&dev, w.lba, w.count);
        println!("track {t}: read {} sectors from {} (LBA {}): {r:?} in {:.2?}", thousands(w.count), tracks_in(&toc, &w), w.lba, at.elapsed());
        r.unwrap();
    }
}
