//! Several rips of a CD image combined, sector by sector, into one.
//!
//! Some drives misread a data sector without reporting it: no C2 error, no
//! SCSI error, just the wrong bytes, most often a neighbouring sector's
//! (a slip of one) or a few bytes wrong. The misreads fall in different
//! places on each rip, so where the rips differ, the copy to keep is the
//! one that passes its own checks: its header names the right address, and
//! its EDC (and, for a form 1 or Mode 1 sector, its ECC) fits its data.
//! Among copies that pass, the most common one wins.
//!
//! Only the rips' disagreements are looked at closely: a run of sectors the
//! same in every rip is taken as it is (compared a chunk at a time), but
//! every sector of the result is checked once more at the end, so what
//! still fails its check is counted however many rips agree on it.
//!
//! A sector is never repaired: no EDC, ECC or header is ever worked out
//! and written. Some PlayStation discs are pressed with sectors that fail
//! their checks on purpose (copy protection), and Redump's copy keeps them
//! as pressed; so what every rip agrees on is kept byte for byte, and where
//! the rips differ and none passes, the most common copy is taken as read.
//!
//! Passing every check is not the same as being right. Only Redump's
//! checksums say the result is the disc; this module never says so.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// A raw CD sector.
pub const SECTOR: usize = 2352;
/// Sectors compared at a time.
const CHUNK: usize = 1024;
/// The sync pattern that starts every data sector.
const SYNC: [u8; 12] = [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0];

// ---- a sector's own checks --------------------------------------------------------------------

/// The EDC table: CRC-32 with the CD-ROM polynomial, reflected (0xD8018001).
const EDC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xD801_8001 } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// A sector's EDC over `data`.
pub fn edc(data: &[u8]) -> u32 {
    data.iter().fold(0u32, |c, &x| EDC_TABLE[((c ^ x as u32) & 0xFF) as usize] ^ (c >> 8))
}

/// Whether a sector's P and Q parity fit it (see [`super::ecc`]). A Mode
/// 2 sector's are worked out with its header taken as zeros.
pub fn ecc_ok(sec: &[u8], zero_header: bool) -> bool {
    super::ecc::fits(sec, zero_header)
}

/// Whether a sector starts with the sync pattern every data sector has.
pub fn has_sync(sec: &[u8]) -> bool {
    sec.len() >= 12 && sec[..12] == SYNC
}

fn bcd(v: u32) -> u8 {
    (((v / 10) << 4) | (v % 10)) as u8
}

/// The address a sector's header gives for `lba`: minutes, seconds and
/// frames of `lba + 150`, in BCD.
pub fn msf(lba: i64) -> [u8; 3] {
    let a = (lba + 150).max(0) as u32;
    [bcd(a / 4500), bcd(a / 75 % 60), bcd(a % 75)]
}

fn le(s: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([s[at], s[at + 1], s[at + 2], s[at + 3]])
}

/// How far one copy of a sector can be trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Trust {
    /// It fails a check: the wrong address, an EDC or ECC that doesn't fit,
    /// or no sync in a data track.
    Bad,
    /// Nothing to check it by: a music sector, or a form 2 sector stored
    /// without an EDC (only its address is checked).
    Unchecked,
    /// It passes its address, EDC and ECC.
    Checked,
}

/// How far `sec`, read as sector `lba`, can be trusted. `data` is whether
/// it's in a data track.
pub fn trust(sec: &[u8], lba: i64, data: bool) -> Trust {
    if sec.len() != SECTOR {
        return Trust::Bad;
    }
    if !data {
        return Trust::Unchecked;
    }
    if sec[..12] != SYNC || sec[12..15] != msf(lba) {
        return Trust::Bad;
    }
    let fits = |ok: bool| if ok { Trust::Checked } else { Trust::Bad };
    match sec[15] {
        1 => fits(edc(&sec[..2064]) == le(sec, 2064) && ecc_ok(sec, false)),
        // Form 2 (bit 5 of the submode): 2,324 bytes, an EDC that may be left out, no ECC.
        2 if sec[18] & 0x20 != 0 => match le(sec, 2348) {
            0 => Trust::Unchecked,
            stored => fits(edc(&sec[16..2348]) == stored),
        },
        2 => fits(edc(&sec[16..2072]) == le(sec, 2072) && ecc_ok(sec, true)),
        // Mode 0: zeros, with nothing but its address to check.
        0 => Trust::Unchecked,
        _ => Trust::Bad,
    }
}

// ---- combining one track ----------------------------------------------------------------------

/// What combining found, in sectors.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tally {
    pub sectors: u64,
    /// The rips didn't all agree on it.
    pub differing: u64,
    /// The result isn't the first rip's.
    pub taken: u64,
    /// The rips differ and none passes its check.
    pub no_good: Vec<i64>,
    /// Copies that each pass (or can't be checked) differ: the most common
    /// was taken, or the first rip's on a tie.
    pub disagree: Vec<i64>,
    /// The result fails its check: those with no good copy, and those
    /// every rip agrees on and that fail anyway.
    pub still_bad: Vec<i64>,
    /// Each rip's sectors that fail their check.
    pub bad_in: Vec<u64>,
    /// The sectors not taken from the first rip, and which copy (by its
    /// place among those combined) each was taken from.
    pub from: Vec<(i64, usize)>,
}

/// Fills `buf` from `r` as far as it goes.
fn fill(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// Combines one track's copies (the same length, one per rip) into `out`.
/// `start` is the track's first sector on the disc; `data` whether it's a
/// data track. `progress` gets sectors done. Stops with an error when
/// `stop` says so.
pub fn combine_track<R: Read>(copies: &mut [R], start: i64, data: bool, out: &mut impl Write, tally: &mut Tally, progress: &mut dyn FnMut(u64), stop: &dyn Fn() -> bool) -> io::Result<()> {
    let n = copies.len();
    if n == 0 {
        return Ok(());
    }
    tally.bad_in.resize(n.max(tally.bad_in.len()), 0);
    let mut bufs = vec![vec![0u8; SECTOR * CHUNK]; n];
    let mut result = vec![0u8; SECTOR * CHUNK];
    let mut lba = start;
    loop {
        if stop() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
        }
        let mut len = None;
        for (c, b) in copies.iter_mut().zip(bufs.iter_mut()) {
            let got = fill(c, b)?;
            if *len.get_or_insert(got) != got {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "the rips aren't the same length"));
            }
        }
        let len = len.unwrap_or(0);
        if len == 0 {
            break;
        }
        let same = bufs[1..].iter().all(|b| b[..len] == bufs[0][..len]);
        for k in (0..len).step_by(SECTOR) {
            let end = (k + SECTOR).min(len);
            let secs: Vec<&[u8]> = bufs.iter().map(|b| &b[k..end]).collect();
            let agree = same || secs[1..].iter().all(|s| *s == secs[0]);
            let chosen = if agree {
                // All agree: the final check below says whether it passes.
                0
            } else {
                tally.differing += 1;
                let trusts: Vec<Trust> = secs.iter().map(|s| trust(s, lba, data)).collect();
                for (i, t) in trusts.iter().enumerate() {
                    if *t == Trust::Bad {
                        tally.bad_in[i] += 1;
                    }
                }
                let best = *trusts.iter().max().unwrap();
                let pick = majority(&secs, |i| trusts[i] == best);
                if best == Trust::Bad {
                    tally.no_good.push(lba);
                } else if (0..n).any(|i| trusts[i] == best && secs[i] != secs[pick]) {
                    tally.disagree.push(lba);
                }
                if secs[pick] != secs[0] {
                    tally.taken += 1;
                    tally.from.push((lba, pick));
                }
                pick
            };
            result[k..end].copy_from_slice(secs[chosen]);
            if trust(&result[k..end], lba, data) == Trust::Bad {
                tally.still_bad.push(lba);
                if agree {
                    for b in tally.bad_in.iter_mut().take(n) {
                        *b += 1;
                    }
                }
            }
            tally.sectors += 1;
            lba += 1;
        }
        out.write_all(&result[..len])?;
        progress(len.div_ceil(SECTOR) as u64);
    }
    Ok(())
}

/// The copy most of those `eligible` agree with; on a tie, the earliest.
fn majority(secs: &[&[u8]], eligible: impl Fn(usize) -> bool) -> usize {
    let mut best = (0usize, usize::MAX);
    for i in (0..secs.len()).filter(|&i| eligible(i)) {
        let votes = (0..secs.len()).filter(|&j| eligible(j) && secs[j] == secs[i]).count();
        if votes > best.0 {
            best = (votes, i);
        }
    }
    if best.1 == usize::MAX {
        0
    } else {
        best.1
    }
}

// ---- combining whole rips ---------------------------------------------------------------------

/// One file of a rip, as its cue sheet lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackFile {
    pub name: String,
    pub size: u64,
    /// Its first sector on the disc: the sectors of the files before it.
    pub start: i64,
    pub data: bool,
}

/// The files a rip's cue sheet names, with their sizes, where each starts
/// on the disc, and whether it's a data track (its first `TRACK` isn't
/// `AUDIO`). redumper writes a file per track, each starting where the one
/// before ends.
pub fn layout(cue: &Path) -> io::Result<Vec<TrackFile>> {
    let dir = cue.parent().unwrap_or(Path::new("."));
    let text = std::fs::read_to_string(cue)?;
    let names = super::redumper::cue_files(&text);
    let mut kinds: Vec<bool> = vec![];
    let mut want_track = false;
    for l in text.lines().map(str::trim) {
        if l.starts_with("FILE ") {
            want_track = true;
        } else if let Some(rest) = l.strip_prefix("TRACK ").filter(|_| want_track) {
            kinds.push(!rest.split_whitespace().nth(1).is_some_and(|t| t.eq_ignore_ascii_case("AUDIO")));
            want_track = false;
        }
    }
    let mut start = 0i64;
    let mut out = vec![];
    for (i, name) in names.into_iter().enumerate() {
        let size = std::fs::metadata(dir.join(&name))?.len();
        out.push(TrackFile { name, size, start, data: kinds.get(i).copied().unwrap_or(true) });
        start += (size / SECTOR as u64) as i64;
    }
    Ok(out)
}

/// What combining rips came to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Combined {
    pub tally: Tally,
    /// The rips used, by number (1 is the first).
    pub used: Vec<usize>,
    /// Rips left out, by number, and why.
    pub left_out: Vec<(usize, String)>,
    /// The combined image's cue sheet.
    pub cue: PathBuf,
}

/// Combines the rips whose cue sheets are `rips` (the first is the one the
/// others are measured by) into `out`: its cue sheet and tracks, under the
/// first rip's names. A rip whose tracks aren't the same sizes is left out.
/// `progress` gets the fraction done.
pub fn combine(rips: &[PathBuf], out: &Path, mut progress: impl FnMut(f32), stop: &dyn Fn() -> bool) -> anyhow::Result<Combined> {
    let first = rips.first().ok_or_else(|| anyhow::anyhow!("no rips to combine"))?;
    let tracks = layout(first)?;
    let mut used = vec![0usize];
    let mut layouts = vec![tracks.clone()];
    let mut left_out = vec![];
    for (i, r) in rips.iter().enumerate().skip(1) {
        let shape = |t: &[TrackFile]| t.iter().map(|f| f.size).collect::<Vec<_>>();
        match layout(r) {
            Ok(t) if shape(&t) == shape(&tracks) => {
                used.push(i);
                layouts.push(t);
            }
            Ok(_) => left_out.push((i + 1, "its tracks aren't the same sizes as the first rip's".into())),
            Err(e) => left_out.push((i + 1, format!("it can't be read: {e}"))),
        }
    }
    std::fs::create_dir_all(out)?;
    let total: u64 = tracks.iter().map(|t| t.size / SECTOR as u64).sum::<u64>().max(1);
    let mut done = 0u64;
    let mut tally = Tally { bad_in: vec![0; used.len()], ..Tally::default() };
    for (k, t) in tracks.iter().enumerate() {
        let mut copies = used
            .iter()
            .zip(&layouts)
            .map(|(&i, l)| {
                let dir = rips[i].parent().unwrap_or(Path::new("."));
                Ok(io::BufReader::with_capacity(1 << 20, std::fs::File::open(dir.join(&l[k].name))?))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let part = out.join(format!("{}.part", t.name));
        let mut w = io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&part)?);
        combine_track(
            &mut copies,
            t.start,
            t.data,
            &mut w,
            &mut tally,
            &mut |n| {
                done += n;
                progress(done as f32 / total as f32);
            },
            stop,
        )?;
        w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        std::fs::rename(&part, out.join(&t.name))?;
    }
    let cue = out.join(first.file_name().unwrap_or_default());
    std::fs::copy(first, &cue)?;
    Ok(Combined { tally, used: used.into_iter().map(|i| i + 1).collect(), left_out, cue })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A sector as a disc holds it at `lba`: `mode` 1, 2 (form 1) or 22
    /// (form 2, with an EDC unless `edc` is false), filled from `seed`.
    pub fn sector(lba: i64, mode: u8, seed: u8, with_edc: bool) -> Vec<u8> {
        let mut s = vec![0u8; SECTOR];
        s[..12].copy_from_slice(&SYNC);
        s[12..15].copy_from_slice(&msf(lba));
        match mode {
            1 => {
                s[15] = 1;
                for (i, b) in s[16..2064].iter_mut().enumerate() {
                    *b = (i as u8).wrapping_mul(7) ^ seed;
                }
                let e = edc(&s[..2064]);
                s[2064..2068].copy_from_slice(&e.to_le_bytes());
                parity(&mut s, false);
            }
            2 => {
                s[15] = 2;
                s[16..24].copy_from_slice(&[0, 0, 8, 0, 0, 0, 8, 0]);
                for (i, b) in s[24..2072].iter_mut().enumerate() {
                    *b = (i as u8).wrapping_mul(13) ^ seed;
                }
                let e = edc(&s[16..2072]);
                s[2072..2076].copy_from_slice(&e.to_le_bytes());
                parity(&mut s, true);
            }
            _ => {
                s[15] = 2;
                s[16..24].copy_from_slice(&[0, 0, 0x20, 0, 0, 0, 0x20, 0]);
                for (i, b) in s[24..2348].iter_mut().enumerate() {
                    *b = (i as u8).wrapping_mul(3) ^ seed;
                }
                if with_edc {
                    let e = edc(&s[16..2348]);
                    s[2348..2352].copy_from_slice(&e.to_le_bytes());
                }
            }
        }
        s
    }

    /// Writes P and Q parity, the header taken as zeros for Mode 2.
    fn parity(s: &mut [u8], zero_header: bool) {
        super::super::ecc::parity(s, zero_header);
    }

    fn image(secs: &[Vec<u8>]) -> Vec<u8> {
        secs.concat()
    }

    fn run(copies: &[Vec<u8>], start: i64, data: bool) -> (Vec<u8>, Tally) {
        let mut readers: Vec<&[u8]> = copies.iter().map(Vec::as_slice).collect();
        let mut out = vec![];
        let mut t = Tally::default();
        combine_track(&mut readers, start, data, &mut out, &mut t, &mut |_| {}, &|| false).unwrap();
        (out, t)
    }

    #[test]
    fn the_checks_know_a_good_sector_from_a_bad_one() {
        assert_eq!(msf(0), [0x00, 0x02, 0x00]);
        assert_eq!(msf(230_570), [0x51, 0x16, 0x20], "LBA 230570 is 51:16:20");
        for mode in [1, 2, 22] {
            let s = sector(100, mode, 9, true);
            assert_eq!(trust(&s, 100, true), Trust::Checked, "mode {mode}");
            assert_eq!(trust(&s, 101, true), Trust::Bad, "mode {mode}: another sector's address");
            let mut flipped = s.clone();
            flipped[1000] ^= 1;
            assert_eq!(trust(&flipped, 100, true), Trust::Bad, "mode {mode}: one bit wrong in the data");
            assert_eq!(trust(&flipped, 100, false), Trust::Unchecked, "music has nothing to check");
        }
        // The ECC alone catches a byte its parity covers and the EDC doesn't.
        let mut s = sector(5, 2, 1, true);
        s[2200] ^= 0x40;
        assert_eq!(trust(&s, 5, true), Trust::Bad);
        // Form 2 stored without an EDC: only its address.
        let s = sector(7, 22, 3, false);
        assert_eq!(trust(&s, 7, true), Trust::Unchecked);
        assert_eq!(trust(&s, 8, true), Trust::Bad);
        // No sync in a data track: filled in, or not data at all.
        assert_eq!(trust(&[0x55; SECTOR], 7, true), Trust::Bad);
    }

    #[test]
    fn a_sector_slipped_in_one_rip_is_taken_from_another() {
        let good: Vec<Vec<u8>> = (0..5).map(|l| sector(l, 2, l as u8, true)).collect();
        // Rip 1 has sector 3 slipped: sector 2's bytes where 3's belong.
        let mut one = good.clone();
        one[3] = good[2].clone();
        let (out, t) = run(&[image(&one), image(&good)], 0, true);
        assert_eq!(out, image(&good));
        assert_eq!((t.sectors, t.differing, t.taken), (5, 1, 1));
        assert!(t.no_good.is_empty() && t.disagree.is_empty() && t.still_bad.is_empty());
        assert_eq!(t.bad_in, [1, 0]);
    }

    #[test]
    fn an_edc_failure_in_each_rip_is_put_right_from_the_others() {
        // Three rips, each wrong somewhere else: an EDC failure in rip 1, a
        // slip in rip 2, a form 2 sector with a bad EDC in rip 3.
        let good: Vec<Vec<u8>> = (0..6).map(|l| sector(1000 + l, if l == 5 { 22 } else { 2 }, l as u8, true)).collect();
        let (mut a, mut b, mut c) = (good.clone(), good.clone(), good.clone());
        a[1][500] ^= 0x10;
        b[4] = good[3].clone();
        c[5][900] ^= 0x01;
        let (out, t) = run(&[image(&a), image(&b), image(&c)], 1000, true);
        assert_eq!(out, image(&good));
        assert_eq!((t.differing, t.taken, t.bad_in.clone()), (3, 1, vec![1, 1, 1]));
        assert!(t.still_bad.is_empty());
    }

    #[test]
    fn form_2_with_no_edc_goes_by_its_address_then_the_majority() {
        let good: Vec<Vec<u8>> = (0..3).map(|l| sector(l, 22, 40 + l as u8, false)).collect();
        // Rip 1 slipped (the wrong address); rips 2 and 3 agree.
        let mut a = good.clone();
        a[2] = good[1].clone();
        let (out, t) = run(&[image(&a), image(&good), image(&good)], 0, true);
        assert_eq!(out, image(&good));
        assert!(t.disagree.is_empty() && t.no_good.is_empty());
        // The right address in all three, the bytes different in one: the two that agree win.
        let mut c = good.clone();
        c[1][300] ^= 0xFF;
        let (out, t) = run(&[image(&c), image(&good), image(&good)], 0, true);
        assert_eq!(out, image(&good));
        assert_eq!(t.disagree, [1], "copies nothing can check disagreed");
    }

    #[test]
    fn a_tie_takes_the_first_rip_and_says_so() {
        // Music: nothing to check, two rips, two answers.
        let a: Vec<u8> = (0..SECTOR * 2).map(|i| i as u8).collect();
        let mut b = a.clone();
        b[SECTOR + 10] ^= 1;
        let (out, t) = run(&[a.clone(), b], 0, false);
        assert_eq!(out, a);
        assert_eq!((t.differing, t.taken, t.disagree.clone()), (1, 0, vec![1]));
        assert!(t.still_bad.is_empty(), "music can't fail a check");
    }

    #[test]
    fn a_sector_no_rip_read_right_is_counted() {
        let good: Vec<Vec<u8>> = (0..4).map(|l| sector(l, 1, l as u8, true)).collect();
        let (mut a, mut b) = (good.clone(), good.clone());
        a[2][100] ^= 1;
        b[2][200] ^= 1;
        let (_, t) = run(&[image(&a), image(&b)], 0, true);
        assert_eq!((t.no_good.clone(), t.still_bad.clone(), t.bad_in.clone()), (vec![2], vec![2], vec![1, 1]));
        // Wrong the same way in every rip: they agree, and the final check still finds it.
        let (_, t) = run(&[image(&a), image(&a)], 0, true);
        assert_eq!((t.differing, t.still_bad.clone(), t.bad_in.clone()), (0, vec![2], vec![1, 1]));
    }

    /// Some PlayStation discs are pressed with sectors that fail their EDC
    /// and ECC on purpose, as copy protection, and Redump's copy keeps them
    /// so. Nothing is ever repaired or regenerated: what every rip agrees
    /// on is kept byte for byte, and where they differ and none passes,
    /// the most common copy is taken as it was read.
    #[test]
    fn a_sector_bad_in_every_rip_alike_is_kept_byte_for_byte() {
        let good: Vec<Vec<u8>> = (0..4).map(|l| sector(l, 2, l as u8, true)).collect();
        let mut pressed = good.clone();
        pressed[1][2073] ^= 0xA5; // a wrong EDC, as pressed
        pressed[2][2100] ^= 0x5A; // and a wrong ECC
        assert_eq!(trust(&pressed[1], 1, true), Trust::Bad);
        assert_eq!(trust(&pressed[2], 2, true), Trust::Bad);
        let (out, t) = run(&[image(&pressed), image(&pressed), image(&pressed)], 0, true);
        assert_eq!(out, image(&pressed), "kept exactly as read");
        assert_eq!((t.differing, t.taken, t.still_bad.clone()), (0, 0, vec![1, 2]));
        // One rip slipped on a protected sector: the two that agree win,
        // still failing their check, never rewritten.
        let mut slipped = pressed.clone();
        slipped[1] = pressed[0].clone();
        let (out, t) = run(&[image(&slipped), image(&pressed), image(&pressed)], 0, true);
        assert_eq!(out, image(&pressed));
        assert_eq!((t.no_good.clone(), t.taken), (vec![1], 1));
    }

    #[test]
    fn whole_rips_combine_track_by_track_and_a_rip_of_another_size_is_left_out() {
        let base = std::env::current_dir().unwrap().join("../../target/drt-test/combine");
        let _ = std::fs::remove_dir_all(&base);
        let good: Vec<Vec<u8>> = (0..4).map(|l| sector(l, 2, l as u8, true)).collect();
        let music: Vec<u8> = (0..SECTOR * 3).map(|i| (i % 7) as u8).collect();
        // Track 2 starts at sector 4: its sectors' addresses follow track 1's.
        let data2: Vec<Vec<u8>> = (7..9).map(|l| sector(l, 1, 0, true)).collect();
        let cue = "FILE \"G (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"G (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\nFILE \"G (Track 3).bin\" BINARY\n  TRACK 03 MODE1/2352\n    INDEX 01 00:00:00\n";
        let write = |n: usize, t1: &[Vec<u8>], t3: &[Vec<u8>]| {
            let d = base.join(format!("rip-{n}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("G.cue"), cue).unwrap();
            std::fs::write(d.join("G (Track 1).bin"), image(t1)).unwrap();
            std::fs::write(d.join("G (Track 2).bin"), &music).unwrap();
            std::fs::write(d.join("G (Track 3).bin"), image(t3)).unwrap();
            d.join("G.cue")
        };
        let mut bad = good.clone();
        bad[1] = good[0].clone();
        let mut bad3 = data2.clone();
        bad3[1][40] ^= 2;
        let one = write(1, &bad, &data2);
        let two = write(2, &good, &bad3);
        let three = write(3, &good[..3], &data2);
        let tracks = layout(&one).unwrap();
        assert_eq!(tracks.iter().map(|t| (t.start, t.data)).collect::<Vec<_>>(), [(0, true), (4, false), (7, true)]);
        let mut seen = 0.0;
        let c = combine(&[one, two, three], &base.join("combined"), |p| seen = p, &|| false).unwrap();
        assert_eq!((c.used.clone(), c.left_out.len()), (vec![1, 2], 1));
        assert_eq!(std::fs::read(base.join("combined/G (Track 1).bin")).unwrap(), image(&good));
        assert_eq!(std::fs::read(base.join("combined/G (Track 3).bin")).unwrap(), image(&data2));
        assert_eq!(std::fs::read_to_string(&c.cue).unwrap(), cue);
        assert_eq!((c.tally.sectors, c.tally.differing, c.tally.taken), (9, 2, 1));
        assert_eq!(seen, 1.0);
        std::fs::remove_dir_all(base).unwrap();
    }

    /// Combines real rips: `DRT_RIPS=a.cue,b.cue,c.cue DRT_WANT=<sha1>
    /// cargo test -p drt-steps real_rips -- --ignored --nocapture`. Each
    /// rip's own count of sectors failing their checks is printed, to set
    /// beside redumper's `info` for the same file.
    #[test]
    #[ignore]
    fn real_rips() {
        let rips: Vec<PathBuf> = std::env::var("DRT_RIPS").unwrap().split(',').map(PathBuf::from).collect();
        let out = std::env::temp_dir().join("drt-real-rips");
        let t0 = std::time::Instant::now();
        let c = combine(&rips, &out, |_| {}, &|| false).unwrap();
        let t = &c.tally;
        println!("{:?} in {:.1?}", c.used, t0.elapsed());
        println!(
            "sectors {}, differing {}, taken {}, no good copy {}, disagree {}, still bad {}, bad in each {:?}",
            t.sectors,
            t.differing,
            t.taken,
            t.no_good.len(),
            t.disagree.len(),
            t.still_bad.len(),
            t.bad_in
        );
        let files: Vec<PathBuf> = layout(&c.cue).unwrap().iter().map(|f| out.join(&f.name)).collect();
        let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
        let roms = super::super::hash::hash_files(&refs, |_| {}, || false).unwrap();
        for r in &roms {
            println!("{} size {} crc {} md5 {} sha1 {}", r.name, r.size, r.crc, r.md5, r.sha1);
        }
        if let Ok(want) = std::env::var("DRT_WANT") {
            assert_eq!(roms[0].sha1, want);
        }
        std::fs::remove_dir_all(out).unwrap();
    }
}
