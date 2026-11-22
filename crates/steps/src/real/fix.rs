//! Fix bad sectors: the sectors of a CD image that fail their own checks,
//! read again from the disc one at a time, and put in place when a copy
//! that passes its checks comes back.
//!
//! Some drives misread data sectors without saying so while redumper reads
//! the disc as one long stream: a sector slips by one, or a few bytes come
//! back wrong. Read again on its own, as a data sector, the same sector
//! usually comes back right. A sector is found bad by its own checks (see
//! [`combine::trust`]): its header names another address, or its EDC (and,
//! where it has one, its ECC) doesn't fit its data.
//!
//! Nothing is repaired here. A copy is put in place only when it was read
//! from the disc and passes its own checks; a sector that never comes back
//! so stays as it is, which is what copy protection needs, since some
//! discs are pressed with sectors that fail on purpose. (Putting a sector
//! right by its own error correction is [`super::ecc`]'s, and Check's last
//! resort.) Before anything is
//! changed, a few sectors that read right the first time are read again
//! and must come back the same: a different disc in the drive changes
//! nothing. The image's files as they were are kept until Redump's
//! checksums say the fixed one is the disc; only they can say so.

use super::combine::{self, Trust, SECTOR};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Tries for each bad sector, unless Check's settings say otherwise.
pub const TRIES: u32 = 10;
/// Sectors that read right the first time, read again to be sure the disc
/// in the drive is the one the image is of.
const SAME_DISC: usize = 8;
/// How far from a sector the cache is cleared, in sectors.
const AWAY: i64 = 10_000;

/// READ CD (MMC-6, 6.19) for `count` raw sectors from `lba`. Byte 1, bits
/// 4 to 2, the expected sector type: 0, any, since a PlayStation track
/// mixes Mode 2 form 1 and form 2. Byte 9, what each sector gives back:
/// 0x80 sync, 0x60 every header (the 4-byte header and a Mode 2 sector's
/// 8-byte subheader), 0x10 user data, 0x08 EDC and ECC, as on the disc;
/// no C2 bits (bits 2 to 1 zero). Byte 10: no subchannel. 2,352 bytes a
/// sector.
pub fn read_cd_cdb(lba: u32, count: u32) -> [u8; 12] {
    let a = lba.to_be_bytes();
    [0xBE, 0, a[0], a[1], a[2], a[3], (count >> 16) as u8, (count >> 8) as u8, count as u8, 0xF8, 0, 0]
}

/// `count` raw sectors from `lba`, read with [`read_cd_cdb`] through SG_IO.
/// Read-only: nothing is written, and the tray stays as it is.
#[cfg(target_os = "linux")]
pub fn read_raw(device: &str, lba: u32, count: u32) -> io::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(device)?;
    let mut buf = vec![0u8; count as usize * SECTOR];
    let got = super::media::sg_read(f.as_raw_fd(), &read_cd_cdb(lba, count), &mut buf, 20_000)?;
    if got != buf.len() {
        return Err(io::Error::other(format!("the drive gave {got} bytes, not {}", buf.len())));
    }
    Ok(buf)
}

#[cfg(not(target_os = "linux"))]
pub fn read_raw(_device: &str, _lba: u32, _count: u32) -> io::Result<Vec<u8>> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "only on Linux"))
}

/// Where sectors are read again from: the drive, or a stand-in in a test.
pub trait Source: Send {
    /// One raw sector, 2,352 bytes.
    fn read(&mut self, lba: i64) -> io::Result<Vec<u8>>;
    /// Reads somewhere else, so a read of `near` that follows has to come
    /// off the disc and not out of the drive's cache.
    fn clear_cache(&mut self, near: i64) -> io::Result<()>;
}

/// The drive, and how many sectors its disc has.
pub struct Drive {
    pub device: String,
    pub sectors: i64,
}

impl Source for Drive {
    fn read(&mut self, lba: i64) -> io::Result<Vec<u8>> {
        read_raw(&self.device, u32::try_from(lba).map_err(|_| io::Error::other("before the disc's start"))?, 1)
    }

    fn clear_cache(&mut self, near: i64) -> io::Result<()> {
        let (n, _) = clear_window(near, self.sectors);
        super::flush::read_sectors(&self.device, n as u32, super::flush::SECTORS.min(self.sectors.max(0) as u32))
    }
}

/// Where to read to clear the cache before `near` is read again: [`AWAY`]
/// after it if the disc goes that far, else as far before it. Gives the
/// first sector and how far that is from `near`.
pub fn clear_window(near: i64, sectors: i64) -> (i64, i64) {
    let n = super::flush::SECTORS as i64;
    let at = if near + AWAY + n <= sectors { near + AWAY } else { (near - AWAY - n).max(0) };
    (at, (at - near).abs())
}

/// Why a sector fails its own check, in a few words, for the log.
pub fn why_bad(sec: &[u8], lba: i64) -> &'static str {
    if sec.len() != SECTOR || sec[1..11].iter().any(|&b| b != 0xFF) || sec[0] != 0 || sec[11] != 0 {
        "no sync"
    } else if sec[12..15] != combine::msf(lba) {
        "wrong address"
    } else {
        "fails its checksum"
    }
}

/// A data sector of the image, as the cue sheet lays the image out.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub lba: i64,
    /// Its track's file, and the byte it starts at in it.
    pub file: PathBuf,
    pub offset: u64,
}

/// Where sector `lba` is in the image, if it's in a data track.
pub fn place(dir: &Path, tracks: &[combine::TrackFile], lba: i64) -> Option<Place> {
    let t = tracks.iter().find(|t| t.data && lba >= t.start && lba < t.start + (t.size / SECTOR as u64) as i64)?;
    Some(Place { lba, file: dir.join(&t.name), offset: (lba - t.start) as u64 * SECTOR as u64 })
}

/// The data sectors of the image whose cue sheet is `cue` that fail their
/// own check, in order, and how many data sectors there are. `progress`
/// gets the fraction done.
pub fn find_bad(cue: &Path, mut progress: impl FnMut(f32), stop: &dyn Fn() -> bool) -> io::Result<(Vec<i64>, u64)> {
    let dir = cue.parent().unwrap_or(Path::new("."));
    let tracks = combine::layout(cue)?;
    let total: u64 = tracks.iter().filter(|t| t.data).map(|t| t.size / SECTOR as u64).sum::<u64>().max(1);
    let (mut bad, mut seen) = (vec![], 0u64);
    let mut buf = vec![0u8; SECTOR * 1024];
    for t in tracks.iter().filter(|t| t.data) {
        let mut f = io::BufReader::with_capacity(1 << 20, std::fs::File::open(dir.join(&t.name))?);
        let mut lba = t.start;
        loop {
            if stop() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
            }
            let n = fill(&mut f, &mut buf)?;
            if n == 0 {
                break;
            }
            for s in buf[..n].chunks(SECTOR) {
                if combine::trust(s, lba, true) == Trust::Bad {
                    bad.push(lba);
                }
                lba += 1;
                seen += 1;
            }
            progress(seen as f32 / total as f32);
        }
    }
    Ok((bad, seen))
}

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

/// Sector `p` of the image, as it is on disk.
pub fn read_at(p: &Place) -> io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(&p.file)?;
    f.seek(SeekFrom::Start(p.offset))?;
    let mut s = vec![0u8; SECTOR];
    f.read_exact(&mut s)?;
    Ok(s)
}

/// What came of one bad sector.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// A copy that passes its checks came back on this try.
    Fixed { tries: u32 },
    /// None did in `tries`. `same` when every read gave back exactly the
    /// image's copy: likely as pressed. `alike`: reads that gave back the
    /// image's copy; `differed` when one gave back other bytes. `errors`:
    /// tries the drive refused.
    Left { tries: u32, same: bool, alike: u32, differed: bool, errors: u32, last_error: Option<String> },
}

/// One bad sector: where, why it was bad, and what came of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sector {
    pub lba: i64,
    pub why: &'static str,
    pub outcome: Outcome,
}

/// Reads sector `lba` again, up to `tries` times, clearing the drive's
/// cache before each try after the first, until a copy passes its checks.
/// A copy only its address can check (a form 2 sector stored without an
/// EDC) is taken only when two reads agree. `before` is the image's copy.
pub fn reread(source: &mut dyn Source, lba: i64, before: &[u8], tries: u32, stop: &dyn Fn() -> bool) -> io::Result<(Option<Vec<u8>>, Outcome)> {
    let (mut errors, mut last_error) = (0, None);
    let (mut same, mut alike, mut differed) = (true, 0, false);
    let mut unchecked: Option<Vec<u8>> = None;
    for t in 1..=tries.max(1) {
        if stop() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
        }
        if t > 1 {
            // A copy it's read already may still be in the cache.
            if let Err(e) = source.clear_cache(lba) {
                last_error = Some(format!("clearing the cache: {e}"));
            }
        }
        let s = match source.read(lba) {
            Ok(s) if s.len() == SECTOR => s,
            Ok(s) => {
                (errors, last_error, same) = (errors + 1, Some(format!("{} bytes back", s.len())), false);
                continue;
            }
            Err(e) => {
                (errors, last_error, same) = (errors + 1, Some(e.to_string()), false);
                continue;
            }
        };
        same &= s == before;
        differed |= s != before;
        alike += u32::from(s == before);
        match combine::trust(&s, lba, true) {
            Trust::Checked => return Ok((Some(s), Outcome::Fixed { tries: t })),
            Trust::Unchecked if unchecked.as_ref() == Some(&s) => return Ok((Some(s), Outcome::Fixed { tries: t })),
            Trust::Unchecked => unchecked = Some(s),
            Trust::Bad => {}
        }
    }
    Ok((None, Outcome::Left { tries: tries.max(1), same, alike, differed, errors, last_error }))
}

/// What fixing came to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fixed {
    /// Data sectors looked at.
    pub sectors: u64,
    /// Each that failed its check, and what came of it.
    pub bad: Vec<Sector>,
    /// The disc in the drive isn't the one the image is of, by this
    /// sector: nothing was read again or changed.
    pub other_disc: Option<String>,
    /// Where the image's files as they were are kept, when any changed.
    pub kept: Option<PathBuf>,
}

impl Fixed {
    pub fn fixed(&self) -> impl Iterator<Item = &Sector> {
        self.bad.iter().filter(|s| matches!(s.outcome, Outcome::Fixed { .. }))
    }

    pub fn left(&self) -> impl Iterator<Item = &Sector> {
        self.bad.iter().filter(|s| matches!(s.outcome, Outcome::Left { .. }))
    }
}

/// Reads a few sectors that pass their check in the image, spread over
/// it, and gives the first that comes back different: then the disc in
/// the drive isn't this one. `None` when they're all the same.
pub fn other_disc(source: &mut dyn Source, dir: &Path, tracks: &[combine::TrackFile], bad: &[i64]) -> io::Result<Option<String>> {
    let data: Vec<(i64, i64)> = tracks.iter().filter(|t| t.data).map(|t| (t.start, t.start + (t.size / SECTOR as u64) as i64)).collect();
    let total: i64 = data.iter().map(|(a, b)| b - a).sum();
    if total == 0 {
        return Ok(None);
    }
    let mut checked = 0;
    for k in 0..SAME_DISC as i64 * 4 {
        if checked == SAME_DISC {
            break;
        }
        // Spread over the data, a little in from each end.
        let mut at = (total * (2 * k + 1)) / (SAME_DISC as i64 * 8);
        let Some(lba) = data.iter().find_map(|&(a, b)| {
            if at < b - a {
                Some(a + at)
            } else {
                at -= b - a;
                None
            }
        }) else {
            continue;
        };
        if bad.contains(&lba) {
            continue;
        }
        let Some(p) = place(dir, tracks, lba) else { continue };
        let ours = read_at(&p)?;
        if combine::trust(&ours, lba, true) != Trust::Checked {
            continue;
        }
        // A read that fails, or a misread (it fails its own check), says
        // nothing either way; a different sector that passes its checks
        // at the same address does.
        match source.read(lba) {
            Ok(theirs) if theirs == ours => checked += 1,
            Ok(theirs) if combine::trust(&theirs, lba, true) == Trust::Checked => {
                return Ok(Some(format!("sector {lba} came back different from the image's, though both pass their checks")));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Copies `file` into `keep`, unless a copy is there already (the first
/// is the original), and gives `file` an inode of its own if it shares
/// one (a rip kept by hard link), so changing it changes nothing else.
fn keep_original(file: &Path, keep: &Path) -> io::Result<()> {
    std::fs::create_dir_all(keep)?;
    let to = keep.join(file.file_name().unwrap_or_default());
    if !to.exists() {
        let part = to.with_extension("part");
        std::fs::copy(file, &part)?;
        std::fs::rename(&part, &to)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if std::fs::metadata(file)?.nlink() > 1 {
            let own = file.with_extension("own");
            std::fs::copy(file, &own)?;
            std::fs::rename(&own, file)?;
        }
    }
    Ok(())
}

/// Writes each copy over its sector in the image, after keeping the
/// files it changes as they were in `keep`.
pub fn patch(places: &[(Place, Vec<u8>)], keep: &Path) -> io::Result<()> {
    let mut files: Vec<&Path> = places.iter().map(|(p, _)| p.file.as_path()).collect();
    files.dedup();
    for f in &files {
        keep_original(f, keep)?;
    }
    for f in files {
        let mut w = std::fs::OpenOptions::new().write(true).open(f)?;
        for (p, s) in places.iter().filter(|(p, _)| p.file == f) {
            assert_eq!(s.len(), SECTOR);
            w.seek(SeekFrom::Start(p.offset))?;
            w.write_all(s)?;
        }
        w.sync_all()?;
    }
    Ok(())
}

/// Puts back the image's files as they were before fixing, from `keep`.
pub fn restore(dir: &Path, keep: &Path) -> io::Result<()> {
    for e in std::fs::read_dir(keep)? {
        let e = e?;
        if e.path().extension().is_some_and(|x| x == "part") {
            continue;
        }
        std::fs::rename(e.path(), dir.join(e.file_name()))?;
    }
    std::fs::remove_dir_all(keep)
}

/// Fixes the bad sectors of the image whose cue sheet is `cue`, reading
/// them from `source` up to `tries` times each; the files it changes are
/// kept as they were in `keep`. `progress` gets the fraction done and what
/// it's doing.
pub fn fix(cue: &Path, source: &mut dyn Source, tries: u32, keep: &Path, mut progress: impl FnMut(f32, String), stop: &dyn Fn() -> bool) -> io::Result<Fixed> {
    let dir = cue.parent().unwrap_or(Path::new("."));
    let tracks = combine::layout(cue)?;
    progress(0.0, "looking for bad sectors".into());
    let (bad, sectors) = find_bad(cue, |p| progress(p * 0.2, "looking for bad sectors".into()), stop)?;
    let mut out = Fixed { sectors, ..Fixed::default() };
    if bad.is_empty() {
        return Ok(out);
    }
    if let Some(why) = other_disc(source, dir, &tracks, &bad)? {
        out.other_disc = Some(why);
        return Ok(out);
    }
    let mut copies = vec![];
    let n = bad.len();
    for (i, &lba) in bad.iter().enumerate() {
        progress(0.2 + 0.8 * i as f32 / n as f32, format!("re-reading {n} bad sector{}", if n == 1 { "" } else { "s" }));
        let p = place(dir, &tracks, lba).ok_or_else(|| io::Error::other(format!("sector {lba} isn't in a data track")))?;
        let before = read_at(&p)?;
        let why = why_bad(&before, lba);
        let (copy, outcome) = reread(source, lba, &before, tries, stop)?;
        if let Some(c) = copy {
            copies.push((p, c));
        }
        out.bad.push(Sector { lba, why, outcome });
    }
    if !copies.is_empty() {
        patch(&copies, keep)?;
        out.kept = Some(keep.to_path_buf());
    }
    progress(1.0, String::new());
    Ok(out)
}

/// The log's lines for what fixing came to: each bad sector's address and
/// what came of it, grouped.
pub fn log_lines(f: &Fixed, tries: u32) -> Vec<String> {
    let list = |v: Vec<String>| v.join(", ");
    let mut out = vec![];
    if let Some(why) = &f.other_disc {
        out.push(format!("fix bad sectors: stopped before changing anything: {why}, so the disc in the drive may not be this one"));
        return out;
    }
    out.push(format!("fix bad sectors: {} of {} data sectors fail their own check: {}", f.bad.len(), f.sectors, list(f.bad.iter().map(|s| format!("{} ({})", s.lba, s.why)).collect())));
    let first: Vec<String> = f.bad.iter().filter(|s| s.outcome == Outcome::Fixed { tries: 1 }).map(|s| s.lba.to_string()).collect();
    if !first.is_empty() {
        out.push(format!("  read right on the first try, and put in place: {} ({})", first.len(), list(first)));
    }
    let later: Vec<String> = f
        .bad
        .iter()
        .filter_map(|s| match s.outcome {
            Outcome::Fixed { tries } if tries > 1 => Some(format!("{} (try {tries})", s.lba)),
            _ => None,
        })
        .collect();
    if !later.is_empty() {
        out.push(format!("  read right on a later try, with the drive's cache cleared first, and put in place: {} ({})", later.len(), list(later)));
    }
    for s in f.left() {
        if let Outcome::Left { tries: t, same, errors, last_error, .. } = &s.outcome {
            let how = match (same, errors) {
                (true, _) => "the same as the image's on every read, so it's left as it is (it may be pressed so)".to_owned(),
                (false, 0) => "no read passed its check; left as it is".to_owned(),
                (false, e) => format!("{e} of the reads failed ({}); left as it is", last_error.clone().unwrap_or_default()),
            };
            out.push(format!("  sector {}: no good copy in {t} of {tries} tries: {how}", s.lba));
        }
    }
    if let Some(k) = &f.kept {
        out.push(format!("  the image's files as they were are kept in {} until Redump's checksums settle it", k.display()));
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A form 1 sector at `lba` as the disc holds it (Mode 2, as on a
    /// PlayStation disc), or form 2 with an EDC.
    pub fn good(lba: i64, form2: bool) -> Vec<u8> {
        let mut s = vec![0u8; SECTOR];
        s[1..11].fill(0xFF);
        s[12..15].copy_from_slice(&combine::msf(lba));
        s[15] = 2;
        if form2 {
            s[16..24].copy_from_slice(&[0, 0, 0x20, 0, 0, 0, 0x20, 0]);
            for (i, b) in s[24..2348].iter_mut().enumerate() {
                *b = (i as u8).wrapping_mul(5) ^ lba as u8;
            }
            let e = combine::edc(&s[16..2348]);
            s[2348..].copy_from_slice(&e.to_le_bytes());
        } else {
            // Form 1 has an ECC: make it with the combiner's own tests' maker.
            s = combine::tests::sector(lba, 2, lba as u8, true);
        }
        s
    }

    /// A disc in a stand-in drive: each sector's reads, in turn, the last
    /// repeated; a sector not listed reads right.
    struct Fake {
        disc: HashMap<i64, Vec<io::Result<Vec<u8>>>>,
        right: Box<dyn Fn(i64) -> Vec<u8> + Send>,
        reads: Vec<i64>,
        clears: Vec<i64>,
    }

    impl Source for Fake {
        fn read(&mut self, lba: i64) -> io::Result<Vec<u8>> {
            self.reads.push(lba);
            match self.disc.get_mut(&lba) {
                Some(q) if q.len() > 1 => q.remove(0),
                Some(q) => match &q[0] {
                    Ok(s) => Ok(s.clone()),
                    Err(e) => Err(io::Error::other(e.to_string())),
                },
                None => Ok((self.right)(lba)),
            }
        }

        fn clear_cache(&mut self, near: i64) -> io::Result<()> {
            self.clears.push(near);
            Ok(())
        }
    }

    fn fake(disc: Vec<(i64, Vec<io::Result<Vec<u8>>>)>) -> Fake {
        Fake { disc: disc.into_iter().collect(), right: Box::new(|l| good(l, false)), reads: vec![], clears: vec![] }
    }

    fn slipped(lba: i64) -> Vec<u8> {
        good(lba - 1, false)
    }

    fn flipped(lba: i64) -> Vec<u8> {
        let mut s = good(lba, false);
        s[700] ^= 0x10;
        s
    }

    fn base(name: &str) -> PathBuf {
        let b = std::env::current_dir().unwrap().join("../../target/drt-test").join(name);
        let _ = std::fs::remove_dir_all(&b);
        std::fs::create_dir_all(&b).unwrap();
        b
    }

    #[test]
    fn the_read_cd_command_asks_for_the_whole_raw_sector() {
        assert_eq!(read_cd_cdb(0x0001_2345, 1), [0xBE, 0, 0, 1, 0x23, 0x45, 0, 0, 1, 0xF8, 0, 0]);
        // Sync, all headers, user data, EDC and ECC; no C2, no subchannel.
        assert_eq!(0x80 | 0x60 | 0x10 | 0x08, 0xF8);
        assert_eq!(read_cd_cdb(7, 0x01_0203)[6..9], [1, 2, 3]);
    }

    #[test]
    fn bad_sectors_are_found_by_their_own_checks_and_placed_by_track() {
        let b = base("fix-find");
        // Track 1: 6 sectors, 2 slipped and 4 flipped; track 2 music; track
        // 3 starts at sector 9, its sector 10 slipped.
        let mut t1: Vec<Vec<u8>> = (0..6).map(|l| good(l, l == 5)).collect();
        t1[2] = slipped(2);
        t1[4] = flipped(4);
        let music = vec![0x33u8; SECTOR * 3];
        let mut t3: Vec<Vec<u8>> = (9..12).map(|l| good(l, false)).collect();
        t3[1] = slipped(10);
        std::fs::write(b.join("G (Track 1).bin"), t1.concat()).unwrap();
        std::fs::write(b.join("G (Track 2).bin"), &music).unwrap();
        std::fs::write(b.join("G (Track 3).bin"), t3.concat()).unwrap();
        let cue = b.join("G.cue");
        std::fs::write(&cue, "FILE \"G (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"G (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\nFILE \"G (Track 3).bin\" BINARY\n  TRACK 03 MODE2/2352\n    INDEX 01 00:00:00\n").unwrap();
        let (bad, n) = find_bad(&cue, |_| {}, &|| false).unwrap();
        assert_eq!((bad.clone(), n), (vec![2, 4, 10], 9), "music has no checks");
        assert_eq!(why_bad(&t1[2], 2), "wrong address");
        assert_eq!(why_bad(&t1[4], 4), "fails its checksum");
        assert_eq!(why_bad(&[0x55; SECTOR], 4), "no sync");
        let tracks = combine::layout(&cue).unwrap();
        assert_eq!(place(&b, &tracks, 10), Some(Place { lba: 10, file: b.join("G (Track 3).bin"), offset: SECTOR as u64 }));
        assert_eq!(place(&b, &tracks, 4).unwrap().offset, 4 * SECTOR as u64);
        assert_eq!(place(&b, &tracks, 7), None, "a music sector is never patched");
        assert_eq!(place(&b, &tracks, 12), None);
        std::fs::remove_dir_all(b).unwrap();
    }

    #[test]
    fn a_sector_that_misreads_then_reads_right_is_taken_on_that_try() {
        // Wrong twice (a slip, then a flip), an error, then right.
        let lba = 500;
        let mut f = fake(vec![(lba, vec![Ok(slipped(lba)), Ok(flipped(lba)), Err(io::Error::other("sense 3/11/05")), Ok(good(lba, false))])]);
        let (copy, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false).unwrap();
        assert_eq!(copy, Some(good(lba, false)));
        assert_eq!(o, Outcome::Fixed { tries: 4 });
        assert_eq!(f.clears, [lba, lba, lba], "the cache cleared before each try after the first");
        // Right first time: no cache clearing at all.
        let mut f = fake(vec![]);
        assert_eq!(reread(&mut f, 9, &slipped(9), 10, &|| false).unwrap().1, Outcome::Fixed { tries: 1 });
        assert!(f.clears.is_empty());
    }

    #[test]
    fn a_sector_the_same_on_every_read_stays_as_it_is() {
        // Pressed to fail its EDC: every read gives the same bytes.
        let lba = 77;
        let pressed = flipped(lba);
        let mut f = fake(vec![(lba, vec![Ok(pressed.clone())])]);
        let (copy, o) = reread(&mut f, lba, &pressed, 10, &|| false).unwrap();
        assert_eq!(copy, None);
        assert_eq!(o, Outcome::Left { tries: 10, same: true, alike: 10, differed: false, errors: 0, last_error: None });
        assert_eq!(f.reads.len(), 10);
        // Reads that fail or differ are left too, but not called the same.
        let mut f = fake(vec![(lba, vec![Ok(slipped(lba)), Err(io::Error::other("sense 3/11/05"))])]);
        let (copy, o) = reread(&mut f, lba, &pressed, 3, &|| false).unwrap();
        assert!(copy.is_none());
        assert!(matches!(o, Outcome::Left { tries: 3, same: false, errors: 2, .. }), "{o:?}");
    }

    #[test]
    fn a_copy_only_its_address_can_check_needs_two_reads_that_agree() {
        // Form 2 stored with no EDC: the address is all there is.
        let lba = 30;
        let mut s = good(lba, true);
        s[2348..].fill(0);
        let mut other = s.clone();
        other[100] ^= 1;
        assert_eq!(combine::trust(&s, lba, true), Trust::Unchecked);
        let mut f = fake(vec![(lba, vec![Ok(other.clone()), Ok(s.clone()), Ok(s.clone())])]);
        let (copy, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false).unwrap();
        assert_eq!((copy, o), (Some(s), Outcome::Fixed { tries: 3 }));
    }

    #[test]
    fn fixing_patches_only_the_bad_sectors_and_keeps_the_original_until_told() {
        let b = base("fix-patch");
        let dir = b.join("dump");
        std::fs::create_dir_all(&dir).unwrap();
        let disc: Vec<Vec<u8>> = (0..40).map(|l| good(l, l % 7 == 3)).collect();
        let mut image = disc.clone();
        image[5] = slipped(5);
        image[17] = flipped(17);
        image[30] = flipped(30); // pressed so: the drive gives it back the same every time
        let first = image.concat();
        std::fs::write(dir.join("G.bin"), &first).unwrap();
        // A rip kept by hard link, as Rip again and combine keeps rip 1.
        std::fs::create_dir_all(b.join("rip-1")).unwrap();
        std::fs::hard_link(dir.join("G.bin"), b.join("rip-1/G.bin")).unwrap();
        let cue = dir.join("G.cue");
        std::fs::write(&cue, "FILE \"G.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n").unwrap();
        let mut f = fake(vec![(5, vec![Ok(slipped(5)), Ok(disc[5].clone())]), (30, vec![Ok(image[30].clone())])]);
        f.right = Box::new(|l| good(l, l % 7 == 3));
        let keep = b.join("before-fix");
        let r = fix(&cue, &mut f, 4, &keep, |_, _| {}, &|| false).unwrap();
        assert_eq!(r.other_disc, None);
        assert_eq!(
            r.bad.iter().map(|s| (s.lba, s.outcome.clone())).collect::<Vec<_>>(),
            [(5, Outcome::Fixed { tries: 2 }), (17, Outcome::Fixed { tries: 1 }), (30, Outcome::Left { tries: 4, same: true, alike: 4, differed: false, errors: 0, last_error: None })]
        );
        let now = std::fs::read(dir.join("G.bin")).unwrap();
        let mut want = disc.clone();
        want[30] = image[30].clone();
        assert_eq!(now, want.concat(), "5 and 17 put right at their own offsets, 30 left as pressed");
        assert_eq!(std::fs::read(keep.join("G.bin")).unwrap(), first, "the original kept");
        assert_eq!(std::fs::read(b.join("rip-1/G.bin")).unwrap(), first, "a linked rip isn't changed with it");
        let lines = log_lines(&r, 4);
        assert_eq!(lines[0], "fix bad sectors: 3 of 40 data sectors fail their own check: 5 (wrong address), 17 (fails its checksum), 30 (fails its checksum)");
        assert_eq!(lines[1], "  read right on the first try, and put in place: 1 (17)");
        assert_eq!(lines[2], "  read right on a later try, with the drive's cache cleared first, and put in place: 1 (5 (try 2))");
        assert!(lines[3].starts_with("  sector 30: no good copy in 4 of 4 tries: the same as the image's on every read"), "{lines:?}");
        // Not verified: the original goes back.
        restore(&dir, &keep).unwrap();
        assert_eq!(std::fs::read(dir.join("G.bin")).unwrap(), first);
        assert!(!keep.exists());
        std::fs::remove_dir_all(b).unwrap();
    }

    #[test]
    fn another_disc_in_the_drive_changes_nothing() {
        let b = base("fix-other");
        let mut image: Vec<Vec<u8>> = (0..64).map(|l| good(l, false)).collect();
        image[3] = slipped(3);
        std::fs::write(b.join("G.bin"), image.concat()).unwrap();
        let cue = b.join("G.cue");
        std::fs::write(&cue, "FILE \"G.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n").unwrap();
        // Every sector passes its check on this other disc, at the right address.
        let mut f = fake(vec![]);
        f.right = Box::new(|l| good(l, true));
        let r = fix(&cue, &mut f, 10, &b.join("keep"), |_, _| {}, &|| false).unwrap();
        assert!(r.other_disc.as_deref().is_some_and(|w| w.contains("came back different from the image's, though both pass their checks")), "{r:?}");
        assert!(r.bad.is_empty() && r.kept.is_none());
        assert_eq!(std::fs::read(b.join("G.bin")).unwrap(), image.concat());
        assert!(!f.reads.contains(&3), "the bad sector wasn't read");
        std::fs::remove_dir_all(b).unwrap();
    }

    #[test]
    fn the_cache_is_cleared_well_away_and_within_the_disc() {
        assert_eq!(clear_window(100, 230_000), (10_100, 10_000));
        assert_eq!(clear_window(229_000, 230_000), (229_000 - 10_000 - 2_400, 12_400));
        assert_eq!(clear_window(5_000, 6_000).0, 0);
    }

    /// Fixes a real redumper image from the disc in the drive, read-only on
    /// the drive: `DRT_CUE=…cue DRT_WANT=<sha1> cargo test -p drt-steps
    /// real_fix -- --ignored --nocapture`. Work on a copy: it patches the
    /// image in place, keeping its original beside it.
    #[test]
    #[ignore]
    fn real_fix() {
        let dev = std::env::var("DRT_DEVICE").unwrap_or_else(|_| "/dev/sr0".into());
        assert!(super::super::media::disc_in(&dev), "no disc in {dev}");
        let cue = PathBuf::from(std::env::var("DRT_CUE").unwrap());
        let sectors: i64 = combine::layout(&cue).unwrap().iter().map(|t| (t.size / SECTOR as u64) as i64).sum();
        let mut d = Drive { device: dev, sectors };
        let keep = cue.with_extension("before-fix");
        let t0 = std::time::Instant::now();
        let r = fix(&cue, &mut d, TRIES, &keep, |_, _| {}, &|| false).unwrap();
        println!("in {:.1?}", t0.elapsed());
        for l in log_lines(&r, TRIES) {
            println!("{l}");
        }
        let files: Vec<PathBuf> = combine::layout(&cue).unwrap().iter().map(|t| cue.parent().unwrap().join(&t.name)).collect();
        let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
        let roms = super::super::hash::hash_files(&refs, |_| {}, || false).unwrap();
        for r in &roms {
            println!("{} size {} crc {} md5 {} sha1 {}", r.name, r.size, r.crc, r.md5, r.sha1);
        }
        if let Ok(want) = std::env::var("DRT_WANT") {
            assert_eq!(roms[0].sha1, want);
        }
    }
}
