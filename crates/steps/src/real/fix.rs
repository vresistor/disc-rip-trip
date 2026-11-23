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
use super::redumper::{left_words, thousands, GIVE_UP_AFTER};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Tries for each bad sector, unless Check's settings say otherwise.
pub const TRIES: u32 = 10;
/// Sectors that read right the first time, read again to be sure the disc
/// in the drive is the one the image is of.
const SAME_DISC: usize = 8;
/// How far from a sector the cache is cleared, in sectors.
const AWAY: i64 = 10_000;
/// A try that takes longer than this is a drive stalling on the sector.
pub const SLOW_TRY: Duration = Duration::from_secs(30);
/// A sector is given up on once this many tries in a row have each come
/// back with the same error, the tries after the first (each with the
/// cache cleared first) each slower than [`SLOW_TRY`]: a drive that stalls
/// on it the same way every time won't read it on its tenth go either, and
/// ten tries would be ten minutes or more. The first try isn't timed: it
/// has no cache to clear, and the drive's own time-out (20 s) may end it.
pub const STALLED_AFTER: u32 = 3;
/// The time left is said once this many bad sectors have been gone through:
/// before that the pace says nothing.
pub const PACE_AFTER: u64 = 10;

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
    /// The time now, for how long tries take: the clock, or a test's own.
    fn now(&self) -> Instant {
        Instant::now()
    }
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
    /// tries the drive refused. `stalled` when it was given up on early,
    /// the drive stalling on it the same way every time
    /// ([`STALLED_AFTER`]).
    Left { tries: u32, same: bool, alike: u32, differed: bool, errors: u32, last_error: Option<String>, stalled: bool },
    /// Not read again: the pass stopped before it, getting nowhere
    /// ([`GIVE_UP_AFTER`]). Still bad.
    NotTried,
}

/// One bad sector: where, why it was bad, and what came of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sector {
    pub lba: i64,
    pub why: &'static str,
    pub outcome: Outcome,
}

/// How a try on a sector went, when it didn't come back right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Last {
    /// The drive couldn't read it.
    Failed,
    /// It came back, but wrong (or, only its address to check it by, not
    /// yet twice the same).
    Wrong,
}

/// Reads sector `lba` again, up to `tries` times, clearing the drive's
/// cache before each try after the first, until a copy passes its checks.
/// A copy only its address can check (a form 2 sector stored without an
/// EDC) is taken only when two reads agree. `before` is the image's copy.
/// `each` hears of each try as it starts, and how the one before it went.
/// A sector the drive stalls on the same way every time is given up on
/// early ([`STALLED_AFTER`]).
pub fn reread(source: &mut dyn Source, lba: i64, before: &[u8], tries: u32, stop: &dyn Fn() -> bool, each: &mut dyn FnMut(u32, Option<Last>)) -> io::Result<(Option<Vec<u8>>, Outcome)> {
    let (mut errors, mut last_error) = (0, None);
    let (mut same, mut alike, mut differed) = (true, 0, false);
    let mut unchecked: Option<Vec<u8>> = None;
    let mut last = None;
    // Tries in a row that came back with the same error, the timed ones
    // each slow.
    let (mut stalls, mut stall_error): (u32, Option<String>) = (0, None);
    let tries = tries.max(1);
    for t in 1..=tries {
        if stop() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
        }
        each(t, last);
        let began = source.now();
        if t > 1 {
            // A copy it's read already may still be in the cache.
            if let Err(e) = source.clear_cache(lba) {
                last_error = Some(format!("clearing the cache: {e}"));
            }
        }
        let read = source.read(lba);
        let slow = t == 1 || source.now().saturating_duration_since(began) > SLOW_TRY;
        let s = match read {
            Ok(s) if s.len() == SECTOR => s,
            Ok(s) => {
                (errors, last_error, same, last) = (errors + 1, Some(format!("{} bytes back", s.len())), false, Some(Last::Failed));
                stalls = 0;
                continue;
            }
            Err(e) => {
                let e = e.to_string();
                (errors, last_error, same, last) = (errors + 1, Some(e.clone()), false, Some(Last::Failed));
                // A slow try with another error starts the count again at itself.
                stalls = match (slow, stall_error.as_deref() == Some(&e)) {
                    (false, _) => 0,
                    (true, true) => stalls + 1,
                    (true, false) => 1,
                };
                stall_error = Some(e);
                if stalls >= STALLED_AFTER && t < tries {
                    return Ok((None, Outcome::Left { tries: t, same, alike, differed, errors, last_error, stalled: true }));
                }
                continue;
            }
        };
        stalls = 0;
        last = Some(Last::Wrong);
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
    Ok((None, Outcome::Left { tries, same, alike, differed, errors, last_error, stalled: false }))
}

/// Where a pass of reading bad sectors again is, for the window: the
/// sector it's on and which try, how far through, what's come of it so
/// far, and the time left once the pace says anything.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rereading {
    /// The sector being read.
    pub at: i64,
    /// Bad sectors gone through, of how many.
    pub done: u64,
    pub total: u64,
    /// Which try on this sector, of how many it may get.
    #[serde(rename = "try")]
    pub attempt: u32,
    pub tries: u32,
    /// How the try before this one went.
    pub last: Option<Last>,
    /// Put right so far, and still bad.
    pub fixed: u64,
    pub left: u64,
    /// Given up on early, the drive stalling on each the same way.
    pub stalled: u64,
    /// The time left, from the time each bad sector has taken so far, once
    /// [`PACE_AFTER`] have been gone through.
    pub left_secs: Option<u64>,
    /// Bad sectors in a row that got every try and didn't come back.
    pub since_back: u64,
    /// Stopped: [`GIVE_UP_AFTER`] in a row didn't come back.
    pub given_up: bool,
}

impl Rereading {
    /// How far through: the bad sectors gone through, and the tries on the
    /// one it's at.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let part = if self.attempt > 0 && self.tries > 0 { (self.attempt - 1) as f32 / self.tries as f32 } else { 0.0 };
        ((self.done as f32 + part) / self.total as f32).clamp(0.0, 1.0)
    }

    /// The headline's line: *Reading bad sectors again: sector 147,402 ·
    /// 12 of 1,226 · try 3 of 10*.
    pub fn line(&self) -> String {
        format!("Reading bad sectors again: {}", self.place())
    }

    /// Where it is: *sector 147,402 · 12 of 1,226 · try 3 of 10*.
    pub fn place(&self) -> String {
        let mut s = format!("sector {} · {} of {}", thousands(self.at.max(0) as u64), thousands((self.done + 1).min(self.total)), thousands(self.total));
        if self.attempt > 0 {
            s.push_str(&format!(" · try {} of {}", self.attempt, self.tries));
        }
        s
    }

    /// What's come of it so far: *9 put right so far · 3 still bad*, and
    /// how many were given up on early.
    pub fn so_far(&self) -> String {
        let mut s = format!("{} put right so far · {} still bad", thousands(self.fixed), thousands(self.left));
        if self.stalled > 0 {
            s.push_str(&format!(" ({} skipped: the drive stalled on every try)", thousands(self.stalled)));
        }
        s
    }

    /// How the try before this one went: *Try 2 came back wrong*.
    pub fn last_text(&self) -> Option<String> {
        let t = self.attempt.checked_sub(1).filter(|t| *t > 0)?;
        Some(match self.last? {
            Last::Failed => format!("Try {t}: the drive couldn't read it"),
            Last::Wrong => format!("Try {t} came back wrong"),
        })
    }

    /// The time left in words, or that it isn't known yet.
    pub fn left_text(&self) -> String {
        self.left_secs.map_or_else(|| "time left unknown".to_owned(), left_words)
    }

    /// The card's line: *9 put right so far · 12 of 1,226 · try 3 of 10 ·
    /// about 40 min left*. A card short of room drops parts from the
    /// front, and keeps a time left at the end.
    pub fn card(&self) -> String {
        let mut s = format!("{} put right so far · {} of {}", thousands(self.fixed), thousands((self.done + 1).min(self.total)), thousands(self.total));
        if self.attempt > 0 {
            s.push_str(&format!(" · try {} of {}", self.attempt, self.tries));
        }
        if let Some(l) = self.left_secs {
            s.push_str(&format!(" · {}", left_words(l)));
        }
        s
    }

    /// The item's note, for the ring's figure: *re-reading 4%, 12 of 1,226
    /// bad sectors*.
    pub fn note(&self) -> String {
        format!("re-reading {}%, {} of {} bad sectors", (self.fraction() * 100.0) as u32, thousands(self.done), thousands(self.total))
    }
}

/// What a pass of reading bad sectors again said, as it went.
pub enum Event<'a> {
    /// Looking for bad sectors in the image: how far.
    Looking(f32),
    /// The bad sectors it found, and is about to read again.
    Found(&'a [i64]),
    /// A try is starting.
    Try(&'a Rereading),
    /// A sector is done with: what came of it.
    Done(&'a Sector, &'a Rereading),
}

/// What Check and fix says when it stopped a pass getting nowhere.
pub const STOPPED: &str = "Stopped: none of the last 50 came back. This drive can't read these; another drive may.";

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
    /// The pass stopped, [`GIVE_UP_AFTER`] in a row not coming back.
    pub given_up: bool,
}

impl Fixed {
    pub fn fixed(&self) -> impl Iterator<Item = &Sector> {
        self.bad.iter().filter(|s| matches!(s.outcome, Outcome::Fixed { .. }))
    }

    /// The sectors still bad: read again with no good copy, or not read
    /// again at all.
    pub fn left(&self) -> impl Iterator<Item = &Sector> {
        self.bad.iter().filter(|s| matches!(s.outcome, Outcome::Left { .. } | Outcome::NotTried))
    }

    /// Those given up on early, the drive stalling on them.
    pub fn stalled(&self) -> impl Iterator<Item = &Sector> {
        self.bad.iter().filter(|s| matches!(s.outcome, Outcome::Left { stalled: true, .. }))
    }

    /// Those not read again, the pass stopped first.
    pub fn not_tried(&self) -> impl Iterator<Item = &Sector> {
        self.bad.iter().filter(|s| s.outcome == Outcome::NotTried)
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
/// kept as they were in `keep`. `on` hears how it goes, try by try. Once
/// [`GIVE_UP_AFTER`] bad sectors in a row have had every try and none came
/// back, the rest aren't read again: the drive can't read them. (A sector
/// that came back the same as the image's on every read doesn't count
/// either way: the drive reads it, and it may be pressed so.)
pub fn fix(cue: &Path, source: &mut dyn Source, tries: u32, keep: &Path, mut on: impl FnMut(Event), stop: &dyn Fn() -> bool) -> io::Result<Fixed> {
    let dir = cue.parent().unwrap_or(Path::new("."));
    let tracks = combine::layout(cue)?;
    on(Event::Looking(0.0));
    let (bad, sectors) = find_bad(cue, |p| on(Event::Looking(p)), stop)?;
    let mut out = Fixed { sectors, ..Fixed::default() };
    if bad.is_empty() {
        return Ok(out);
    }
    if let Some(why) = other_disc(source, dir, &tracks, &bad)? {
        out.other_disc = Some(why);
        return Ok(out);
    }
    on(Event::Found(&bad));
    let mut copies = vec![];
    let tries = tries.max(1);
    let mut r = Rereading { total: bad.len() as u64, tries, ..Rereading::default() };
    let began = source.now();
    for &lba in &bad {
        let p = place(dir, &tracks, lba).ok_or_else(|| io::Error::other(format!("sector {lba} isn't in a data track")))?;
        let before = read_at(&p)?;
        let why = why_bad(&before, lba);
        if r.given_up {
            out.bad.push(Sector { lba, why, outcome: Outcome::NotTried });
            continue;
        }
        (r.at, r.attempt, r.last) = (lba, 0, None);
        let (copy, outcome) = reread(source, lba, &before, tries, stop, &mut |t, last| {
            (r.attempt, r.last) = (t, last);
            on(Event::Try(&r));
        })?;
        if let Some(c) = copy {
            copies.push((p, c));
        }
        r.done += 1;
        match &outcome {
            Outcome::Fixed { .. } => (r.fixed, r.since_back) = (r.fixed + 1, 0),
            Outcome::Left { same: true, .. } => r.left += 1,
            Outcome::Left { stalled, .. } => {
                (r.left, r.since_back) = (r.left + 1, r.since_back + 1);
                r.stalled += u64::from(*stalled);
            }
            Outcome::NotTried => {}
        }
        if r.done >= PACE_AFTER {
            let each = source.now().saturating_duration_since(began).as_secs_f64() / r.done as f64;
            r.left_secs = Some((each * (r.total - r.done) as f64).round() as u64);
        }
        r.given_up = r.since_back >= GIVE_UP_AFTER && r.done < r.total;
        let s = Sector { lba, why, outcome };
        on(Event::Done(&s, &r));
        out.bad.push(s);
    }
    out.given_up = r.given_up;
    if !copies.is_empty() {
        patch(&copies, keep)?;
        out.kept = Some(keep.to_path_buf());
    }
    Ok(out)
}

/// The log's line for one bad sector once it's been read again: *sector
/// 5: read right on try 2*. Only what was read: a copy goes in place once
/// the pass is done.
pub fn sector_line(s: &Sector, tries: u32) -> String {
    let how = match &s.outcome {
        Outcome::Fixed { tries } => format!("read right on try {tries}"),
        Outcome::NotTried => "not read again".to_owned(),
        Outcome::Left { stalled: true, tries: t, last_error, .. } => {
            format!("skipped after {t} tries, the drive stalling on each the same way ({}); left as it is", last_error.clone().unwrap_or_default())
        }
        Outcome::Left { tries: t, same, errors, last_error, .. } => {
            let how = match (same, errors) {
                (true, _) => "the same as the image's on every read, so it's left as it is (it may be pressed so)".to_owned(),
                (false, 0) => "no read passed its check; left as it is".to_owned(),
                (false, e) => format!("{e} of the reads failed ({}); left as it is", last_error.clone().unwrap_or_default()),
            };
            format!("no good copy in {t} of {tries} tries: {how}")
        }
    };
    format!("fix bad sectors: sector {}: {how}", s.lba)
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
    let tried: Vec<String> = f.left().filter(|s| matches!(s.outcome, Outcome::Left { stalled: false, .. })).map(|s| s.lba.to_string()).collect();
    if !tried.is_empty() {
        out.push(format!("  no good copy in {tries} tries, and left as they are: {} ({})", tried.len(), list(tried)));
    }
    let stalled: Vec<String> = f.stalled().map(|s| s.lba.to_string()).collect();
    if !stalled.is_empty() {
        out.push(format!("  skipped early, the drive stalling on every try the same way, and left as they are: {} ({})", stalled.len(), list(stalled)));
    }
    if f.given_up {
        let n = f.not_tried().count();
        out.push(format!("  stopped with {n} not read again: none of the last {GIVE_UP_AFTER} came back, so this drive can't read these; another drive may"));
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
        /// Its own clock: each read of a sector takes what `takes` says.
        began: Instant,
        gone: Duration,
        takes: Box<dyn Fn(i64) -> Duration + Send>,
    }

    impl Source for Fake {
        fn now(&self) -> Instant {
            self.began + self.gone
        }

        fn read(&mut self, lba: i64) -> io::Result<Vec<u8>> {
            self.reads.push(lba);
            self.gone += (self.takes)(lba);
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
        Fake { disc: disc.into_iter().collect(), right: Box::new(|l| good(l, false)), reads: vec![], clears: vec![], began: Instant::now(), gone: Duration::ZERO, takes: Box::new(|_| Duration::ZERO) }
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
        let (copy, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false, &mut |_, _| {}).unwrap();
        assert_eq!(copy, Some(good(lba, false)));
        assert_eq!(o, Outcome::Fixed { tries: 4 });
        assert_eq!(f.clears, [lba, lba, lba], "the cache cleared before each try after the first");
        // Right first time: no cache clearing at all.
        let mut f = fake(vec![]);
        assert_eq!(reread(&mut f, 9, &slipped(9), 10, &|| false, &mut |_, _| {}).unwrap().1, Outcome::Fixed { tries: 1 });
        assert!(f.clears.is_empty());
    }

    #[test]
    fn a_sector_the_same_on_every_read_stays_as_it_is() {
        // Pressed to fail its EDC: every read gives the same bytes.
        let lba = 77;
        let pressed = flipped(lba);
        let mut f = fake(vec![(lba, vec![Ok(pressed.clone())])]);
        let (copy, o) = reread(&mut f, lba, &pressed, 10, &|| false, &mut |_, _| {}).unwrap();
        assert_eq!(copy, None);
        assert_eq!(o, Outcome::Left { tries: 10, same: true, alike: 10, differed: false, errors: 0, last_error: None, stalled: false });
        assert_eq!(f.reads.len(), 10);
        // Reads that fail or differ are left too, but not called the same.
        let mut f = fake(vec![(lba, vec![Ok(slipped(lba)), Err(io::Error::other("sense 3/11/05"))])]);
        let (copy, o) = reread(&mut f, lba, &pressed, 3, &|| false, &mut |_, _| {}).unwrap();
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
        let (copy, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false, &mut |_, _| {}).unwrap();
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
        let r = fix(&cue, &mut f, 4, &keep, |_| {}, &|| false).unwrap();
        assert_eq!(r.other_disc, None);
        assert_eq!(
            r.bad.iter().map(|s| (s.lba, s.outcome.clone())).collect::<Vec<_>>(),
            [(5, Outcome::Fixed { tries: 2 }), (17, Outcome::Fixed { tries: 1 }), (30, Outcome::Left { tries: 4, same: true, alike: 4, differed: false, errors: 0, last_error: None, stalled: false })]
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
        assert_eq!(lines[3], "  no good copy in 4 tries, and left as they are: 1 (30)");
        assert_eq!(sector_line(&r.bad[2], 4), "fix bad sectors: sector 30: no good copy in 4 of 4 tries: the same as the image's on every read, so it's left as it is (it may be pressed so)");
        assert_eq!(sector_line(&r.bad[0], 4), "fix bad sectors: sector 5: read right on try 2");
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
        let r = fix(&cue, &mut f, 10, &b.join("keep"), |_| {}, &|| false).unwrap();
        assert!(r.other_disc.as_deref().is_some_and(|w| w.contains("came back different from the image's, though both pass their checks")), "{r:?}");
        assert!(r.bad.is_empty() && r.kept.is_none());
        assert_eq!(std::fs::read(b.join("G.bin")).unwrap(), image.concat());
        assert!(!f.reads.contains(&3), "the bad sector wasn't read");
        std::fs::remove_dir_all(b).unwrap();
    }

    /// An image of `n` sectors, those in `bad` slipped, and its cue sheet.
    fn image(name: &str, n: i64, bad: &[i64]) -> (PathBuf, PathBuf) {
        let b = base(name);
        let image: Vec<Vec<u8>> = (0..n).map(|l| if bad.contains(&l) { slipped(l) } else { good(l, false) }).collect();
        std::fs::write(b.join("G.bin"), image.concat()).unwrap();
        let cue = b.join("G.cue");
        std::fs::write(&cue, "FILE \"G.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n").unwrap();
        (b, cue)
    }

    fn failing(lbas: impl IntoIterator<Item = i64>, error: &str) -> Vec<(i64, Vec<io::Result<Vec<u8>>>)> {
        lbas.into_iter().map(|l| (l, vec![Err(io::Error::other(error.to_owned()))])).collect()
    }

    #[test]
    fn each_try_is_told_as_it_starts_with_how_the_one_before_went() {
        let lba = 500;
        let mut f = fake(vec![(lba, vec![Ok(slipped(lba)), Err(io::Error::other("sense 3/11/05")), Ok(good(lba, false))])]);
        let mut heard = vec![];
        let (_, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false, &mut |t, last| heard.push((t, last))).unwrap();
        assert_eq!(o, Outcome::Fixed { tries: 3 });
        assert_eq!(heard, [(1, None), (2, Some(Last::Wrong)), (3, Some(Last::Failed))]);
    }

    #[test]
    fn a_pass_says_where_it_is_what_has_come_back_and_the_time_left_once_the_pace_says() {
        // Twelve bad: sector 21 comes back on its third try, 23 never; each
        // read takes two seconds by the drive's own clock.
        let bad: Vec<i64> = (20..32).collect();
        let (b, cue) = image("fix-live", 64, &bad);
        let mut f = fake(vec![(21, vec![Ok(slipped(21)), Ok(slipped(21)), Ok(good(21, false))]), (23, vec![Err(io::Error::other("sense 3/11/05"))])]);
        f.takes = Box::new(|_| Duration::from_secs(2));
        let (mut tries, mut done, mut found) = (vec![], vec![], vec![]);
        let r = fix(
            &cue,
            &mut f,
            4,
            &b.join("keep"),
            |e| match e {
                Event::Looking(_) => {}
                Event::Found(l) => found = l.to_vec(),
                Event::Try(r) => tries.push(r.clone()),
                Event::Done(s, r) => done.push((s.lba, r.clone())),
            },
            &|| false,
        )
        .unwrap();
        assert_eq!(found, bad);
        assert_eq!(r.bad.len(), 12);
        // Sector 21: three tries, the last two after a wrong copy.
        let on21: Vec<(u32, Option<Last>)> = tries.iter().filter(|r| r.at == 21).map(|r| (r.attempt, r.last)).collect();
        assert_eq!(on21, [(1, None), (2, Some(Last::Wrong)), (3, Some(Last::Wrong))]);
        let third = tries.iter().find(|r| r.at == 21 && r.attempt == 3).unwrap();
        assert_eq!(third.line(), "Reading bad sectors again: sector 21 · 2 of 12 · try 3 of 4");
        assert_eq!(third.last_text().as_deref(), Some("Try 2 came back wrong"));
        assert_eq!(third.fraction(), (1.0 + 2.0 / 4.0) / 12.0);
        let on23 = tries.iter().find(|r| r.at == 23 && r.attempt == 2).unwrap();
        assert_eq!(on23.last_text().as_deref(), Some("Try 1: the drive couldn't read it"));
        assert_eq!(on23.so_far(), "3 put right so far · 0 still bad");
        // After 23: one still bad. No time left until ten are done.
        let after = |lba: i64| done.iter().find(|d| d.0 == lba).map(|d| d.1.clone()).unwrap();
        assert_eq!((after(23).fixed, after(23).left), (3, 1));
        assert_eq!(after(28).left_secs, None);
        assert_eq!(after(28).left_text(), "time left unknown");
        // Ten done in 2 + 6 + 2 + 8 + 2×6 = 30 s, 3 s each: 6 s for the last two.
        assert_eq!((after(29).done, after(29).left_secs), (10, Some(6)));
        assert_eq!(third.card(), "1 put right so far · 2 of 12 · try 3 of 4");
        assert_eq!(Rereading { left_secs: Some(2400), ..third.clone() }.card(), "1 put right so far · 2 of 12 · try 3 of 4 · about 40 min left");
        assert_eq!(after(31).fraction(), 1.0);
        std::fs::remove_dir_all(b).unwrap();
    }

    #[test]
    fn a_sector_the_drive_stalls_on_the_same_way_every_time_is_skipped_early() {
        let lba = 40;
        // A minute a try, the same error each time: three tries, not ten.
        let mut f = fake(failing([lba], "sense 3/02/00, timed out"));
        f.takes = Box::new(|_| Duration::from_secs(60));
        let (copy, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false, &mut |_, _| {}).unwrap();
        assert!(copy.is_none());
        assert!(matches!(o, Outcome::Left { tries: STALLED_AFTER, stalled: true, errors: STALLED_AFTER, .. }), "{o:?}");
        assert_eq!(f.reads.len(), STALLED_AFTER as usize);
        // Quick errors get every try.
        let mut f = fake(failing([lba], "sense 3/11/05"));
        f.takes = Box::new(|_| Duration::from_secs(1));
        let (_, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false, &mut |_, _| {}).unwrap();
        assert!(matches!(o, Outcome::Left { tries: 10, stalled: false, .. }), "{o:?}");
        // Slow, but not the same error each time: every try too.
        let mut f = fake(vec![(lba, (0..10).map(|k| Err(io::Error::other(if k % 2 == 0 { "a" } else { "b" }))).collect())]);
        f.takes = Box::new(|_| Duration::from_secs(60));
        let (_, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false, &mut |_, _| {}).unwrap();
        assert!(matches!(o, Outcome::Left { tries: 10, stalled: false, .. }), "{o:?}");
        // Another error, then the same one three times, all slow: the count
        // starts again at the new one, so given up after the fourth.
        let mut f = fake(vec![(lba, vec![Err(io::Error::other("a")), Err(io::Error::other("b"))])]);
        f.takes = Box::new(|_| Duration::from_secs(60));
        let (_, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false, &mut |_, _| {}).unwrap();
        assert!(matches!(o, Outcome::Left { tries: 4, stalled: true, .. }), "{o:?}");
        // Slow and failing, then right: taken, as ever, before giving up.
        let mut f = fake(vec![(lba, vec![Err(io::Error::other("a")), Err(io::Error::other("a")), Ok(good(lba, false))])]);
        f.takes = Box::new(|_| Duration::from_secs(60));
        let (_, o) = reread(&mut f, lba, &slipped(lba), 10, &|| false, &mut |_, _| {}).unwrap();
        assert_eq!(o, Outcome::Fixed { tries: 3 });
    }

    #[test]
    fn a_pass_where_fifty_in_a_row_never_come_back_stops_and_says_so() {
        // Sixty bad, from 10; none comes back.
        let bad: Vec<i64> = (10..70).collect();
        let (b, cue) = image("fix-give-up", 100, &bad);
        let mut f = fake(failing(bad.clone(), "sense 3/11/05"));
        let mut last = None;
        let r = fix(
            &cue,
            &mut f,
            3,
            &b.join("keep"),
            |e| {
                if let Event::Done(_, r) = e {
                    last = Some(r.clone());
                }
            },
            &|| false,
        )
        .unwrap();
        assert!(r.given_up);
        assert_eq!(r.left().count(), 60, "every one still bad");
        assert_eq!(r.not_tried().map(|s| s.lba).collect::<Vec<_>>(), (60..70).collect::<Vec<_>>());
        assert!(!f.reads.iter().any(|l| (60..70).contains(l)), "the rest weren't read");
        let last = last.unwrap();
        assert!(last.given_up && last.since_back == GIVE_UP_AFTER && last.done == 50);
        assert!(r.kept.is_none(), "nothing changed");
        let lines = log_lines(&r, 3);
        assert!(lines.iter().any(|l| l == "  stopped with 10 not read again: none of the last 50 came back, so this drive can't read these; another drive may"), "{lines:?}");
        assert_eq!(sector_line(&r.bad[55], 3), "fix bad sectors: sector 65: not read again");
        assert!(STOPPED.contains(&GIVE_UP_AFTER.to_string()));
        let f2 = super::super::mismatch::Fixing::of(&r, 3);
        assert_eq!((f2.left, f2.not_tried, f2.given_up, f2.same), (60, 10, true, 0), "none of them called pressed");
        std::fs::remove_dir_all(b).unwrap();
    }

    #[test]
    fn one_coming_back_starts_the_count_again_and_pressed_ones_dont_count() {
        // 49 never come back, then 54 does; 55 to 64 read the same as the
        // image's every time (pressed so), then 49 more never come back:
        // never fifty in a row.
        let bad: Vec<i64> = (5..114).collect();
        let (b, cue) = image("fix-count-again", 130, &bad);
        let mut disc = failing(bad.iter().copied().filter(|l| *l != 54 && !(55..65).contains(l)), "sense 3/11/05");
        disc.push((54, vec![Ok(good(54, false))]));
        for l in 55..65 {
            disc.push((l, vec![Ok(slipped(l))]));
        }
        let mut f = fake(disc);
        let r = fix(&cue, &mut f, 2, &b.join("keep"), |_| {}, &|| false).unwrap();
        assert!(!r.given_up, "{:?}", r.not_tried().count());
        assert_eq!(r.not_tried().count(), 0);
        std::fs::remove_dir_all(b).unwrap();
    }

    #[test]
    fn sectors_skipped_early_count_towards_the_fifty() {
        let bad: Vec<i64> = (10..65).collect();
        let (b, cue) = image("fix-stalls", 100, &bad);
        let mut f = fake(failing(bad.clone(), "timed out"));
        f.takes = Box::new(|l| if (10..65).contains(&l) { Duration::from_secs(45) } else { Duration::ZERO });
        let r = fix(&cue, &mut f, 10, &b.join("keep"), |_| {}, &|| false).unwrap();
        assert!(r.given_up);
        assert_eq!((r.stalled().count(), r.not_tried().count()), (50, 5));
        assert_eq!(f.reads.iter().filter(|l| (10..65).contains(*l)).count(), 50 * STALLED_AFTER as usize);
        let lines = log_lines(&r, 10);
        assert!(lines.iter().any(|l| l.starts_with("  skipped early, the drive stalling on every try the same way, and left as they are: 50 (")), "{lines:?}");
        let f2 = super::super::mismatch::Fixing::of(&r, 10);
        assert_eq!((f2.stalled, f2.not_tried), (50, 5));
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
        let r = fix(&cue, &mut d, TRIES, &keep, |_| {}, &|| false).unwrap();
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
