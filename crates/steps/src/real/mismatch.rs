//! Why a game's dump doesn't match Redump: what Redump's list expects
//! beside what this copy has, file by file; what the read itself reported,
//! from redumper's log; and a plain reading of what that likely means.
//!
//! Check works it out when it holds a dump, and keeps it in its record for
//! its sidebar. The reading says only what the numbers support: a sector
//! that fails its own check was read wrong or is wrong on the disc, and
//! the log can't tell which.

use super::redump;
use serde::{Deserialize, Serialize};

/// One file of a dump, or of Redump's copy of it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub name: String,
    pub size: u64,
    pub sha1: String,
}

impl File {
    fn of(r: &redump::Rom) -> File {
        File { name: r.name.clone(), size: r.size, sha1: r.sha1.to_ascii_lowercase() }
    }

    /// How many sectors it holds: 2,352 bytes each on a CD, 2,048 in an ISO.
    pub fn sectors(&self) -> Option<u64> {
        let per = if self.name.to_ascii_lowercase().ends_with(".iso") { 2048 } else { 2352 };
        (self.size > 0 && self.size.is_multiple_of(per)).then_some(self.size / per)
    }
}

/// A file of Redump's copy beside the same track of this one. Either side
/// may be missing when they have different numbers of tracks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub expected: Option<File>,
    pub got: Option<File>,
}

impl Pair {
    pub fn size_ok(&self) -> bool {
        matches!((&self.expected, &self.got), (Some(e), Some(g)) if e.size == g.size)
    }

    pub fn sha1_ok(&self) -> bool {
        matches!((&self.expected, &self.got), (Some(e), Some(g)) if !e.sha1.is_empty() && e.sha1 == g.sha1)
    }
}

/// A kind of sector `redumper info` finds wrong in an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// Filled in with 0x55 because it couldn't be read (*generated sectors*).
    FilledIn,
    /// Not a data sector at all (*invalid sync*).
    NotData,
    /// Its header gives another sector's address (*MSF errors*).
    Address,
    /// Its error-correction bytes don't fit its data (*ECC errors*).
    Ecc,
    /// Its checksum doesn't fit its data (*EDC errors*).
    Edc,
}

impl Kind {
    /// What it is, in words.
    pub fn words(self) -> &'static str {
        match self {
            Kind::FilledIn => "Couldn't be read, filled in",
            Kind::NotData => "Not data at all",
            Kind::Address => "Wrong address",
            Kind::Ecc => "Fail their error correction",
            Kind::Edc => "Fail their checksum",
        }
    }

    /// redumper's name for it, for the curious.
    pub fn code(self) -> &'static str {
        match self {
            Kind::FilledIn => "generated",
            Kind::NotData => "sync",
            Kind::Address => "MSF",
            Kind::Ecc => "ECC",
            Kind::Edc => "EDC",
        }
    }

    const ALL: [(Kind, &'static str); 5] =
        [(Kind::FilledIn, "generated sectors (0x55):"), (Kind::NotData, "invalid sync sectors:"), (Kind::Address, "MSF errors:"), (Kind::Ecc, "ECC errors:"), (Kind::Edc, "EDC errors:")];
}

/// Sectors of one kind: how many, and the first and last of them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sectors {
    pub kind: Kind,
    pub count: u64,
    pub first: Option<i64>,
    pub last: Option<i64>,
}

/// What redumper's log says about the read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Read {
    /// The drive, as it names itself: *MATSHITA BD-MLT UJ240AS*.
    pub drive: String,
    /// redumper doesn't know the drive, and read it with general settings.
    pub generic: bool,
    /// The read offset redumper used.
    pub read_offset: Option<i64>,
    /// Errors the drive itself reported and re-reading didn't clear, in
    /// samples (588 to a sector).
    pub scsi: u64,
    pub c2: u64,
    /// What `info` found wrong in the image, each kind found.
    pub sectors: Vec<Sectors>,
    /// The count Redump keeps for a disc: sectors failing their own check,
    /// a sector failing both counted once.
    pub redump_errors: Option<u64>,
    /// The whole disc, in sectors: where its lead-out starts.
    pub disc_sectors: u64,
}

impl Read {
    pub fn of(&self, kind: Kind) -> Option<&Sectors> {
        self.sectors.iter().find(|s| s.kind == kind && s.count > 0)
    }

    /// Sectors that fail their own check, as Redump counts them.
    pub fn bad(&self) -> u64 {
        let most = [Kind::Ecc, Kind::Edc].iter().filter_map(|k| self.of(*k)).map(|s| s.count).max().unwrap_or(0);
        self.redump_errors.unwrap_or(most).max(most)
    }

    /// Whether the drive said anything went wrong.
    pub fn drive_reported(&self) -> bool {
        self.scsi + self.c2 > 0
    }

    /// Where a sector is, as a share of the disc: *49%*.
    pub fn share(&self, lba: i64) -> Option<u64> {
        (self.disc_sectors > 0).then(|| ((lba.max(0) as f64 * 100.0 / self.disc_sectors as f64).round() as u64).min(100))
    }

    /// Where sectors are on the disc, in words: *between 49% and 68% of
    /// the way through*.
    pub fn place(&self, s: &Sectors) -> String {
        match (s.first.and_then(|f| self.share(f)), s.last.and_then(|l| self.share(l))) {
            (Some(a), Some(b)) if a == b || s.count == 1 => format!("{a}% of the way through"),
            (Some(a), Some(b)) => format!("between {a}% and {b}% of the way through"),
            _ => String::new(),
        }
    }
}

/// Reads what matters from redumper's log: the drive, the errors it
/// reported, and what the last `info` found in the image.
pub fn read_log(log: &str) -> Read {
    let mut r = Read::default();
    let lines: Vec<&str> = log.lines().map(str::trim).collect();
    for (i, l) in lines.iter().enumerate() {
        if let Some(rest) = l.strip_prefix("inquiry:") {
            // "MATSHITA - BD-MLT UJ240AS (revision level: 1.01, …)"
            let name = rest.split(" (").next().unwrap_or("").replace(" - ", " ");
            r.drive = name.split_whitespace().collect::<Vec<_>>().join(" ");
        }
        if l.starts_with("warning: drive not found in the database") || l.starts_with("configuration: GENERIC") {
            r.generic = true;
        }
        if let Some(at) = l.find("read offset:").filter(|_| l.starts_with("configuration:")) {
            r.read_offset = l[at + "read offset:".len()..].split([',', ')']).next().and_then(|n| n.trim().trim_start_matches('+').parse().ok());
        }
        // The lead-out's start: the disc's length. The last TOC wins.
        if l.starts_with("track A {") {
            if let Some(n) = lines.get(i + 1).and_then(|n| n.strip_prefix("index 01 { LBA:")).and_then(|n| n.split(',').next()?.trim().parse::<u64>().ok()) {
                r.disc_sectors = n;
            }
        }
    }
    (r.scsi, r.c2) = drive_errors(&lines);
    // The last `info`: after a re-read, it's the one about this image.
    let start = lines.iter().rposition(|l| l.starts_with("*** INFO")).unwrap_or(0);
    let info = &lines[start..];
    let end = info.iter().position(|l| l.starts_with("*** END")).unwrap_or(info.len());
    let info = &info[..end];
    let mut counted = 0u64;
    for (kind, key) in Kind::ALL {
        let mut s = Sectors { kind, count: 0, first: None, last: None };
        for l in info.iter().filter_map(|l| l.strip_prefix(key)) {
            s.count += l.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
            let list = l.split_once("(LBA:").map(|(_, rest)| rest.trim_end_matches(')')).unwrap_or("");
            for (a, b) in list.split(',').filter_map(range) {
                s.first = Some(s.first.map_or(a, |f: i64| f.min(a)));
                s.last = Some(s.last.map_or(b, |x: i64| x.max(b)));
            }
        }
        if s.count > 0 {
            r.sectors.push(s);
        }
    }
    for l in info.iter().filter_map(|l| l.strip_prefix("REDUMP.INFO errors:")) {
        counted += l.trim().parse::<u64>().unwrap_or(0);
        r.redump_errors = Some(counted);
    }
    if r.disc_sectors == 0 {
        r.disc_sectors = info.iter().filter_map(|l| l.strip_prefix("sectors count:")?.trim().parse::<u64>().ok()).sum();
    }
    r
}

/// One of redumper's sector ranges: `112697`, `0-299`, `-150--1`.
fn range(s: &str) -> Option<(i64, i64)> {
    let s = s.trim();
    match s[1.min(s.len())..].find('-').map(|i| i + 1) {
        Some(i) => Some((s[..i].parse().ok()?, s[i + 1..].parse().ok()?)),
        None => s.parse().ok().map(|n| (n, n)),
    }
}

/// The SCSI and C2 errors the drive reported that re-reading didn't clear:
/// the last `media errors:` block, less the `correction statistics:`
/// after it.
fn drive_errors(lines: &[&str]) -> (u64, u64) {
    let (mut scsi, mut c2) = (0u64, 0u64);
    let mut block = 0; // 1: errors, -1: corrected
    for l in lines {
        if l.starts_with("media errors:") {
            (scsi, c2, block) = (0, 0, 1);
            continue;
        }
        if l.starts_with("correction statistics:") {
            block = -1;
            continue;
        }
        let n = |k: &str| l.strip_prefix(k).and_then(|r| r.split_whitespace().next()?.parse::<u64>().ok());
        match (n("SCSI:"), n("C2:")) {
            (Some(x), _) if block == 1 => scsi += x,
            (Some(x), _) if block == -1 => scsi = scsi.saturating_sub(x),
            (_, Some(x)) if block == 1 => c2 += x,
            (_, Some(x)) if block == -1 => c2 = c2.saturating_sub(x),
            _ if l.starts_with("Q:") => {}
            _ => block = 0,
        }
    }
    (scsi, c2)
}

/// Why a dump isn't any of Redump's copies of its serial.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mismatch {
    pub serial: String,
    /// The copy compared with: Redump's name for it.
    pub entry: String,
    /// The serial's other copies in Redump's list.
    pub others: Vec<String>,
    pub files: Vec<Pair>,
    /// What the log says about the read, when there is one.
    pub read: Option<Read>,
    /// What it likely means, a sentence each.
    pub reading: Vec<String>,
}

impl Mismatch {
    /// How many copies Redump has of the serial.
    pub fn copies(&self) -> usize {
        1 + self.others.len()
    }

    fn same_count(&self) -> bool {
        self.files.iter().all(|p| p.expected.is_some() && p.got.is_some())
    }

    fn same_size(&self) -> bool {
        self.same_count() && self.files.iter().all(Pair::size_ok)
    }

    /// For the card, a few words: *same size, 15 bad sectors*.
    pub fn short(&self) -> String {
        let bad = self.read.as_ref().map_or(0, Read::bad);
        match (self.same_size(), bad) {
            (false, _) => "a different size from Redump's".into(),
            (true, 0) => "same size, different contents".into(),
            (true, 1) => "same size, 1 bad sector".into(),
            (true, n) => format!("same size, {n} bad sectors"),
        }
    }
}

/// Compares a dump with the serial's copies in Redump's list: with the one
/// most like it (most tracks the same, then closest in size). `None` when
/// Redump has none.
pub fn explain(serial: &str, copies: &[&redump::Game], dumped: &[redump::Rom], log: Option<&str>) -> Option<Mismatch> {
    let got: Vec<File> = dumped.iter().filter(|r| r.is_data()).map(File::of).collect();
    let data = |g: &redump::Game| g.roms.iter().filter(|r| r.is_data()).map(File::of).collect::<Vec<_>>();
    let closeness = |g: &&redump::Game| {
        let theirs = data(g);
        let same = theirs.iter().zip(&got).filter(|(e, d)| e.sha1 == d.sha1).count();
        let size = |f: &[File]| f.iter().map(|f| f.size).sum::<u64>();
        (std::cmp::Reverse(same), theirs.len().abs_diff(got.len()), size(&theirs).abs_diff(size(&got)))
    };
    let best = copies.iter().copied().min_by_key(closeness)?;
    let expected = data(best);
    let files = (0..expected.len().max(got.len())).map(|i| Pair { expected: expected.get(i).cloned(), got: got.get(i).cloned() }).collect();
    let mut m = Mismatch {
        serial: serial.to_owned(),
        entry: best.name.clone(),
        others: copies.iter().filter(|g| g.name != best.name).map(|g| g.name.clone()).collect(),
        files,
        read: log.map(read_log),
        reading: vec![],
    };
    m.reading = reading(&m);
    Some(m)
}

fn sectors(n: u64) -> String {
    format!("{n} sector{}", if n == 1 { "" } else { "s" })
}

/// What a mismatch likely means, a sentence or two each: only what the
/// sizes, checksums and log support.
pub fn reading(m: &Mismatch) -> Vec<String> {
    let mut out = vec![];
    let read = m.read.as_ref();
    let bad = read.map_or(0, Read::bad);
    let reported = read.is_some_and(Read::drive_reported);
    let tracks = |n: usize| format!("{n} track{}", if n == 1 { "" } else { "s" });
    if !m.same_count() {
        let (e, g) = (m.files.iter().filter(|p| p.expected.is_some()).count(), m.files.iter().filter(|p| p.got.is_some()).count());
        out.push(format!("Redump's copy has {} and this one has {}, so this is likely a different version of the disc from Redump's.", tracks(e), tracks(g)));
    } else if !m.same_size() {
        out.push("The size differs from Redump's, so this is likely a different version of the disc from Redump's.".into());
    } else if m.files.len() > 1 && m.files[0].sha1_ok() && !m.files[1..].iter().all(Pair::sha1_ok) {
        out.push("The first track matches Redump's, so it's the same disc, but the music tracks don't. Music has no checksums of its own, so a drive can read it wrong without noticing. Clean the disc and rip it again, or try another drive.".into());
    } else if bad > 0 && !reported {
        out.push(format!(
            "The size is right, so it's likely the same version of the disc, but {} read wrong. Clean the disc and try Re-read bad parts. This drive didn't report the bad reads itself, so a re-read may not find them; another drive might.",
            sectors(bad)
        ));
    } else if reported {
        out.push("The size is right, so it's likely the same version of the disc, but the drive reported read errors it couldn't fix. Clean the disc and try Re-read bad parts.".into());
    } else if read.is_some() {
        out.push("The size is right, but the contents differ and redumper found nothing wrong with any sector. This may be a version of the disc Redump doesn't list yet.".into());
    } else {
        out.push("The size is right, but the contents differ. There's no rip log to say how the read went.".into());
    }
    if let Some(r) = read {
        if let Some(a) = r.of(Kind::Address) {
            out.push(format!("{} came back with the wrong address. A scratch can cause that, and so can a drive that loses its place while reading.", sectors(a.count)));
        }
        if r.generic {
            let drive = if r.drive.is_empty() { "this drive".to_owned() } else { format!("this drive ({})", r.drive) };
            let offset = r.read_offset.map(|o| format!(" and a read offset of {o:+}")).unwrap_or_default();
            out.push(format!("redumper doesn't know {drive}, so it read the disc with general settings{offset}. Redump's own copies are made on drives redumper knows."));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PlayStation disc's log, trimmed, in the shape redumper (b744)
    /// writes it: a drive it doesn't know, no errors reported, and sectors
    /// `info` finds wrong. The names and numbers are made up.
    pub const LOG: &str = "\
=== 2026-11-20 18:35:32 ========================================================
redumper (build: b744)

arguments: dump --drive=/dev/sr0 --retries=20 --auto-detect --overwrite --image-path=/tmp/image --image-name=GAME_1
warning: drive read offset not found in the database
warning: drive not found in the database
warning: using generic drive

drive information
  path: /dev/sr0
  inquiry: ACME - DVD-RW XY123 (revision level: 1.01, vendor specific: 090903 ....)
  configuration: GENERIC (read offset: +0, C2 shift: 0, pre-gap start: +0, read method: BE, sector order: DATA_C2_SUB)

*** DUMP (time check: 0s)

disc TOC:
  track 1 {  data }
    index 01 { LBA:      0, MSF: 00:02:00 }
  track A {  data }
    index 01 { LBA: 200000, MSF: 44:28:50 }

warning: unscrambled sector read
media errors:
  SCSI: 0 samples
  C2: 0 samples
  Q: 316

*** END (time check: 376s)

*** SPLIT (time check: 0s)

final TOC:
  track 1 {  data }
    index 01 { LBA: [     0 .. 199999], length: 200000, MSF: 00:02:00-44:28:49 }
  track A {  data }
    index 01 { LBA: 200000, MSF: 44:28:50 }

*** END (time check: 12s)

*** INFO (time check: 0s)

CD-ROM [GAME_1.bin]:
  sectors count: 200000
  mode2 (form 1) sectors: 180000
  mode2 (form 2) sectors: 20000
  mode2 (form 2) EDC: yes
  MSF errors: 40 (LBA: 20000, 20500, 98000-98001, 136000)
  ECC errors: 5 (LBA: 98000, 99000, 110000, 120000, 136000)
  EDC errors: 4 (LBA: 98000, 99000, 120000, 136000)

  REDUMP.INFO errors: 5

*** END (time check: 1s)
";

    const BIN: u64 = 200_000 * 2352;

    fn dat() -> redump::Catalogue {
        redump::parse(
            format!(
                r#"<?xml version="1.0"?><datafile><header><version>test</version></header>
<game name="Lantern Keep (Europe) (Disc 1)"><serial>SLES-04711</serial>
<rom name="Lantern Keep (Europe) (Disc 1).cue" size="90" crc="00" md5="00" sha1="ffff"/>
<rom name="Lantern Keep (Europe) (Disc 1).bin" size="{BIN}" crc="00" md5="00" sha1="aaaa111100000000000000000000000000000000"/>
</game>
<game name="Lantern Keep (Europe) (Disc 1) (Rev 1)"><serial>SLES-04711</serial>
<rom name="Lantern Keep (Europe) (Disc 1) (Rev 1).bin" size="{}" crc="00" md5="00" sha1="aaaa222200000000000000000000000000000000"/>
</game></datafile>"#,
                BIN + 2352 * 300
            )
            .as_bytes(),
        )
        .unwrap()
    }

    fn rom(name: &str, size: u64, sha1: &str) -> redump::Rom {
        redump::Rom { name: name.into(), size, crc: String::new(), md5: String::new(), sha1: sha1.into() }
    }

    #[test]
    fn the_log_says_which_drive_and_what_went_wrong_where() {
        let r = read_log(LOG);
        assert_eq!(r.drive, "ACME DVD-RW XY123");
        assert!(r.generic);
        assert_eq!(r.read_offset, Some(0));
        assert_eq!((r.scsi, r.c2), (0, 0));
        assert_eq!(r.disc_sectors, 200_000);
        assert_eq!(r.redump_errors, Some(5));
        assert_eq!(r.bad(), 5);
        let edc = r.of(Kind::Edc).unwrap();
        assert_eq!((edc.count, edc.first, edc.last), (4, Some(98000), Some(136000)));
        assert_eq!(r.place(edc), "between 49% and 68% of the way through");
        let msf = r.of(Kind::Address).unwrap();
        assert_eq!((msf.count, msf.first, msf.last), (40, Some(20000), Some(136000)), "a range counts by its ends");
        assert!(r.of(Kind::NotData).is_none());
    }

    #[test]
    fn errors_the_drive_reported_are_less_what_a_re_read_cleared() {
        let refined = format!("{LOG}\nmedia errors: \n  SCSI: 588 samples\n  C2: 12 samples\n  Q: 2\n*** REFINE\ncorrection statistics: \n  SCSI: 588 samples\n  C2: 0 samples\n");
        let r = read_log(&refined);
        assert_eq!((r.scsi, r.c2), (0, 12));
        assert!(r.drive_reported());
        // A later `info` is about the re-read image.
        let again = format!("{LOG}\n*** INFO (time check: 0s)\nCD-ROM [GAME_1.bin]:\n  sectors count: 200000\n\n  REDUMP.INFO errors: 0\n*** END\n");
        let r = read_log(&again);
        assert_eq!((r.bad(), r.sectors.len()), (0, 0));
        assert_eq!(range("-150--1"), Some((-150, -1)));
        assert_eq!(range(" 0-299"), Some((0, 299)));
    }

    #[test]
    fn same_size_bad_sectors_and_a_quiet_drive() {
        let cat = dat();
        let copies = cat.by_serial("SLES-04711");
        let dumped = [rom("GAME_1.cue", 80, "eeee"), rom("GAME_1.bin", BIN, "bbbb000000000000000000000000000000000000")];
        let m = explain("SLES-04711", &copies, &dumped, Some(LOG)).unwrap();
        assert_eq!(m.entry, "Lantern Keep (Europe) (Disc 1)", "the one the same size");
        assert_eq!(m.others, ["Lantern Keep (Europe) (Disc 1) (Rev 1)"]);
        assert_eq!(m.files.len(), 1, "no cue sheet");
        let p = &m.files[0];
        assert!(p.size_ok() && !p.sha1_ok());
        assert_eq!(p.expected.as_ref().unwrap().sectors(), Some(200_000));
        assert_eq!(m.short(), "same size, 5 bad sectors");
        assert!(m.reading[0].starts_with("The size is right, so it's likely the same version of the disc, but 5 sectors read wrong. Clean the disc"), "{:?}", m.reading);
        assert!(m.reading[0].contains("didn't report the bad reads"));
        assert!(m.reading[1].starts_with("40 sectors came back with the wrong address"));
        assert_eq!(
            m.reading[2],
            "redumper doesn't know this drive (ACME DVD-RW XY123), so it read the disc with general settings and a read offset of +0. Redump's own copies are made on drives redumper knows."
        );
        // It goes to the window as JSON, and back.
        let back: Mismatch = serde_json::from_value(serde_json::to_value(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn a_different_size_is_a_different_version() {
        let cat = dat();
        let copies = cat.by_serial("SLES-04711");
        let m = explain("SLES-04711", &copies, &[rom("GAME_1.bin", BIN - 2352 * 50, "cccc")], None).unwrap();
        assert_eq!(m.short(), "a different size from Redump's");
        assert_eq!(m.reading, ["The size differs from Redump's, so this is likely a different version of the disc from Redump's."]);
        // Tracks missing on one side.
        let two = [rom("a (Track 1).bin", BIN, "cccc"), rom("a (Track 2).bin", 2352, "dddd")];
        let m = explain("SLES-04711", &copies, &two, None).unwrap();
        assert!(m.files[1].expected.is_none());
        assert!(m.reading[0].starts_with("Redump's copy has 1 track and this one has 2"), "{:?}", m.reading);
    }

    #[test]
    fn the_data_track_matches_but_the_music_doesnt() {
        let cat = redump::parse(
            br#"<?xml version="1.0"?><datafile><header><version>t</version></header>
<game name="Racer (USA)"><serial>SLUS-00001</serial>
<rom name="Racer (USA) (Track 1).bin" size="23520" crc="0" md5="0" sha1="1111"/>
<rom name="Racer (USA) (Track 2).bin" size="47040" crc="0" md5="0" sha1="2222"/>
</game></datafile>"#,
        )
        .unwrap();
        let dumped = [rom("R (Track 1).bin", 23520, "1111"), rom("R (Track 2).bin", 47040, "9999")];
        let m = explain("SLUS-00001", &cat.by_serial("SLUS-00001"), &dumped, None).unwrap();
        assert!(m.files[0].sha1_ok() && !m.files[1].sha1_ok());
        assert!(m.reading[0].starts_with("The first track matches Redump's, so it's the same disc, but the music tracks don't."));
        assert!(explain("SLUS-00002", &[], &dumped, None).is_none(), "nothing to compare with");
    }

    /// Reads a real log. `DRT_LOG=… cargo test -p drt-steps real_log -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_log() {
        let text = std::fs::read_to_string(std::env::var("DRT_LOG").unwrap()).unwrap();
        let r = read_log(&text);
        println!("{r:#?}");
        for s in &r.sectors {
            println!("{} ({}): {}, {}", s.kind.words(), s.kind.code(), s.count, r.place(s));
        }
    }
}
