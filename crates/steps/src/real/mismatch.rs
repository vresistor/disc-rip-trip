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

    /// It, among sectors already counted as read wrong: *with the wrong
    /// address*.
    pub fn among(self) -> &'static str {
        match self {
            Kind::FilledIn => "couldn't be read and were filled in",
            Kind::NotData => "not data at all",
            Kind::Address => "with the wrong address",
            Kind::Ecc => "failing their error correction",
            Kind::Edc => "failing their checksum",
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

    /// Sectors that fail any check, as many as the commonest kind: a
    /// sector failing two counted once, as far as the counts can say.
    pub fn wrong(&self) -> u64 {
        self.sectors.iter().map(|s| s.count).max().unwrap_or(0)
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
pub(crate) fn range(s: &str) -> Option<(i64, i64)> {
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
    /// Whether ripping again and combining can help: a CD image redumper
    /// made, whose sectors have their own checks.
    #[serde(default)]
    pub can_combine: bool,
    /// The rips combined so far, once it's been ripped again.
    #[serde(default)]
    pub combined: Option<Combination>,
    /// What Fix bad sectors came to, once it's been tried.
    #[serde(default)]
    pub fixed: Option<Fixing>,
    /// What repairing by the disc's error correction came to, once it's
    /// been tried on the image as it is now.
    #[serde(default)]
    pub repaired: Option<Repairing>,
}

/// What Fix bad sectors came to: the sectors that failed their own check,
/// read again from the disc.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Fixing {
    /// Sectors that failed their own check.
    pub bad: u64,
    /// Those read again right and put in place.
    pub fixed: u64,
    /// Those still failing after every try.
    pub left: u64,
    /// The first few of those.
    pub left_at: Vec<i64>,
    /// Those of them that came back the same on every read.
    pub same: u64,
    /// Tries for each.
    pub tries: u32,
    /// Sectors that read right the first time came back different: the
    /// disc in the drive may not be this one, and nothing was changed.
    pub other_disc: bool,
    /// Its checksums are Redump's.
    pub verified: bool,
    /// Every sector left that a re-read gave back the image's own copy of,
    /// whatever other reads gave: two reads alike, so it may be pressed so.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_alike: Vec<i64>,
    /// Every sector left whose re-reads all gave back other bytes than the
    /// image's, or none: read wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_differently: Vec<i64>,
}

impl Fixing {
    pub fn of(f: &super::fix::Fixed, tries: u32) -> Fixing {
        let left: Vec<&super::fix::Sector> = f.left().collect();
        Fixing {
            bad: f.bad.len() as u64,
            fixed: f.fixed().count() as u64,
            left: left.len() as u64,
            left_at: left.iter().take(10).map(|s| s.lba).collect(),
            same: left.iter().filter(|s| matches!(s.outcome, super::fix::Outcome::Left { same: true, .. })).count() as u64,
            tries,
            other_disc: f.other_disc.is_some(),
            verified: false,
            read_alike: left.iter().filter(|s| matches!(s.outcome, super::fix::Outcome::Left { alike, .. } if alike > 0)).map(|s| s.lba).collect(),
            read_differently: left.iter().filter(|s| matches!(s.outcome, super::fix::Outcome::Left { alike: 0, differed: true, .. })).map(|s| s.lba).collect(),
        }
    }

    /// The item's note: *211 sectors re-read*.
    pub fn note(&self) -> String {
        format!("{} re-read", sectors(self.fixed))
    }
}

/// What fixing and combining came to, for the item's note: *211 sectors
/// re-read, 1 from a second rip*, or either alone.
pub fn result_note(fixed: Option<&Fixing>, combined: Option<&Combination>) -> String {
    match (fixed.filter(|f| f.fixed > 0), combined) {
        (Some(f), Some(c)) => {
            let from = match c.rips.len() {
                2 => "a second rip".to_owned(),
                n => format!("{} more rips", n.saturating_sub(1)),
            };
            format!("{}, {} from {from}", f.note(), c.taken)
        }
        (Some(f), None) => f.note(),
        (None, Some(c)) => c.note(),
        (None, None) => String::new(),
    }
}

/// What repairing by the disc's error correction came to: the sectors
/// that still failed their own check, each put right by its own P and Q
/// parity where it could be (see [`super::ecc`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Repairing {
    /// Sectors that failed their own check.
    pub bad: u64,
    /// Those put right by their error correction, and kept.
    pub repaired: u64,
    /// Those it could put right but that every read gave back the same,
    /// or that were read only once: they may be pressed so on purpose, so
    /// they're left as read, unless repairing them too made the image
    /// Redump's (then they're among `repaired`).
    pub held_back: u64,
    /// Those nothing was read of: nothing to correct.
    pub unreadable: u64,
    /// Those that are form 2 sectors, which have no error correction.
    pub form2: u64,
    /// Those too damaged to correct, or read with the wrong address.
    pub beyond: u64,
    /// Its checksums are Redump's.
    pub verified: bool,
}

impl Repairing {
    /// The note: *3 sectors repaired by the disc's error correction*.
    pub fn note(&self) -> String {
        format!("{} repaired by the disc's error correction", sectors(self.repaired))
    }

    /// Those still bad after it.
    pub fn left(&self) -> u64 {
        self.bad.saturating_sub(self.repaired)
    }
}

/// A note with what repairing came to after it, when it repaired any:
/// *2 sectors re-read, 3 sectors repaired by the disc's error correction*.
pub fn and_repaired(note: String, repaired: Option<&Repairing>) -> String {
    match repaired.filter(|r| r.repaired > 0) {
        Some(r) if note.is_empty() => r.note(),
        Some(r) => format!("{note}, {}", r.note()),
        None => note,
    }
}

/// One rip of the disc, as combining found it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RipRead {
    /// Its number: 1 is the first.
    pub n: usize,
    /// Its sectors that fail their own check.
    pub bad: u64,
}

/// What combining the rips so far came to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Combination {
    pub rips: Vec<RipRead>,
    pub sectors: u64,
    /// Sectors the rips didn't all agree on.
    pub differing: u64,
    /// Sectors taken from a rip other than the first.
    pub taken: u64,
    /// Sectors no rip read so that they pass their check.
    pub no_good: u64,
    /// Sectors where copies that each pass (or can't be checked) differ.
    pub disagree: u64,
    /// Sectors of the result that fail their check.
    pub still_bad: u64,
    /// The first few of those.
    pub left: Vec<i64>,
    /// Rips left out, and why.
    pub left_out: Vec<String>,
    /// Its checksums are Redump's.
    pub verified: bool,
}

impl Combination {
    pub fn of(c: &super::combine::Combined, verified: bool) -> Combination {
        let t = &c.tally;
        Combination {
            rips: c.used.iter().zip(&t.bad_in).map(|(&n, &bad)| RipRead { n, bad }).collect(),
            sectors: t.sectors,
            differing: t.differing,
            taken: t.taken,
            no_good: t.no_good.len() as u64,
            disagree: t.disagree.len() as u64,
            still_bad: t.still_bad.len() as u64,
            left: t.still_bad.iter().take(10).copied().collect(),
            left_out: c.left_out.iter().map(|(n, why)| format!("Rip {n} was left out: {why}.")).collect(),
            verified,
        }
    }

    /// The item's note: *combined from 3 rips, 565 sectors taken from the
    /// later rips*.
    pub fn note(&self) -> String {
        let taken = match self.taken {
            0 => "nothing taken from the later rips".to_owned(),
            n => format!("{} taken from the later rips", sectors(n)),
        };
        format!("combined from {} rips, {taken}", self.rips.len())
    }
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

    /// Whether Redump has a copy to compare with.
    pub fn in_redump(&self) -> bool {
        !self.entry.is_empty()
    }

    /// What it offers you, `(id, label)`: each only where it can help.
    /// *Re-read bad parts* goes back over what the drive reported bad, so
    /// it's no help when the drive reported nothing; ripping again and
    /// combining needs the same version of the disc, and sectors with
    /// checks of their own.
    pub fn options(&self) -> Vec<(String, String)> {
        let mut out = vec![];
        let o = |id: &str, label: &str| (id.to_owned(), label.to_owned());
        let refine = self.combined.is_none() && self.read.as_ref().is_none_or(Read::drive_reported);
        if refine {
            out.push(o("refine", "Re-read bad parts"));
        }
        if self.can_fix() {
            out.push(o("fix", "Fix bad sectors"));
        }
        if self.can_combine && (!self.in_redump() || self.same_size()) {
            out.push(o("combine", if self.combined.is_some() { "Rip again and combine again" } else { "Rip again and combine" }));
        }
        // The last resort, after every way of reading it again.
        if self.can_repair() {
            out.push(o("repair", "Repair with the disc's error correction"));
        }
        out.push(o("keep", if self.combined.is_some() { "Keep the best combination (not verified)" } else { "Keep it anyway" }));
        out.push(o("leave_out", "Skip this disc"));
        out
    }

    /// Whether Fix bad sectors can help: the sectors have checks of their
    /// own (as for combining), it hasn't been tried, nothing's been
    /// combined yet, the copy is the size of Redump's, and some of its
    /// sectors fail their check, as far as the read said.
    pub fn can_fix(&self) -> bool {
        self.can_combine && self.fixed.is_none() && self.combined.is_none() && (!self.in_redump() || self.same_size()) && self.read.as_ref().is_none_or(|r| r.bad().max(r.wrong()) > 0)
    }

    /// Whether repairing by the disc's error correction can help: the
    /// sectors have checks of their own, it hasn't been tried on the image
    /// as it is now, the copy is the size of Redump's, and some of its
    /// sectors still fail their check, as far as anything has said.
    pub fn can_repair(&self) -> bool {
        let bad = match (&self.combined, &self.fixed) {
            (Some(c), _) => c.still_bad > 0,
            (None, Some(f)) if !f.other_disc => f.left > 0,
            (None, Some(_)) => false,
            (None, None) => self.read.as_ref().is_none_or(|r| r.bad().max(r.wrong()) > 0),
        };
        self.can_combine && self.repaired.is_none() && (!self.in_redump() || self.same_size()) && bad
    }

    /// Says what repairing came to, and works out what it all means again.
    pub fn repairing(mut self, repaired: Option<Repairing>) -> Mismatch {
        self.repaired = repaired;
        self.reading = reading(&self);
        self
    }

    /// Says whether combining can help and what it's come to, and works
    /// out what it all means again.
    pub fn with(mut self, can_combine: bool, combined: Option<Combination>) -> Mismatch {
        self.can_combine = can_combine;
        self.combined = combined;
        self.reading = reading(&self);
        self
    }

    /// Says what fixing came to, and works out what it all means again.
    pub fn fixing(mut self, fixed: Option<Fixing>) -> Mismatch {
        self.fixed = fixed;
        self.reading = reading(&self);
        self
    }

    /// For the card, a few words: *same size, 15 bad sectors*.
    pub fn short(&self) -> String {
        if let Some(r) = &self.repaired {
            return match (r.repaired, r.left()) {
                (0, n) => format!("error correction repaired nothing, {} still bad", sectors(n)),
                (k, 0) => format!("{} repaired by error correction", sectors(k)),
                (k, n) => format!("{} repaired by error correction, {} still bad", sectors(k), sectors(n)),
            };
        }
        if let Some(f) = self.fixed.as_ref().filter(|_| self.combined.is_none()) {
            return match (f.other_disc, f.left) {
                (true, _) => "not fixed: another disc in the drive?".into(),
                (false, 0) => format!("{} re-read", sectors(f.fixed)),
                (false, n) => format!("{} re-read, {} still bad", sectors(f.fixed), sectors(n)),
            };
        }
        if let Some(c) = &self.combined {
            return match c.still_bad {
                0 => format!("combined from {} rips", c.rips.len()),
                n => format!("combined from {} rips, {} still bad", c.rips.len(), sectors(n)),
            };
        }
        let bad = self.read.as_ref().map_or(0, |r| r.bad().max(r.wrong()));
        if !self.in_redump() {
            return format!("not in Redump, {} bad", sectors(bad.max(self.read.as_ref().map_or(0, Read::wrong))));
        }
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
        can_combine: false,
        combined: None,
        fixed: None,
        repaired: None,
    };
    m.reading = reading(&m);
    Some(m)
}

/// A dump Redump has no copy of, whose own sectors say it was read wrong:
/// nothing to set it beside, only what the read reported.
pub fn unlisted(serial: &str, dumped: &[redump::Rom], log: &str) -> Mismatch {
    let files = dumped.iter().filter(|r| r.is_data()).map(|r| Pair { expected: None, got: Some(File::of(r)) }).collect();
    let mut m = Mismatch { serial: serial.to_owned(), files, read: Some(read_log(log)), ..Mismatch::default() };
    m.reading = reading(&m);
    m
}

pub fn sectors(n: u64) -> String {
    format!("{n} sector{}", if n == 1 { "" } else { "s" })
}

/// What a mismatch likely means, a sentence or two each: only what the
/// sizes, checksums and log support.
pub fn reading(m: &Mismatch) -> Vec<String> {
    let mut out = vec![];
    let read = m.read.as_ref();
    let bad = read.map_or(0, |r| r.bad().max(r.wrong()));
    let reported = read.is_some_and(Read::drive_reported);
    let tracks = |n: usize| format!("{n} track{}", if n == 1 { "" } else { "s" });
    // What fixing and ripping again do, for a misread the drive kept quiet about.
    let combine = "Fix bad sectors reads just those sectors again, one at a time, and puts a copy in place only when it passes its own check. If that isn't enough, Rip again and combine reads the whole disc again: misreads like these usually fall in different places each time.";
    if let Some(f) = &m.fixed {
        out.push(fix_reading(f, m.in_redump(), m.combined.is_some(), m.repaired.is_some()));
    }
    if let Some(r) = m.repaired.as_ref().filter(|_| m.fixed.is_none() && m.combined.is_none()) {
        out.push(repair_reading(r, m.in_redump()));
        return out;
    }
    if let Some(c) = &m.combined {
        let rips = c.rips.len();
        out.push(match (c.verified, c.still_bad, m.in_redump()) {
            (true, _, _) => format!("Combined from {rips} rips, it matches Redump's copy. {} came from the later rips.", cap(&sectors(c.taken))),
            (false, 0, false) => format!("Combined from {rips} rips, every sector passes its own check. Redump has no copy to compare with, so it can't be called verified."),
            (false, 0, true) if c.disagree > 0 => format!(
                "Combined from {rips} rips, every sector passes its own check, but it still doesn't match Redump's copy. In {}, rips that each pass their check read differently, and the most common was taken. Another rip may settle them.",
                sectors(c.disagree)
            ),
            (false, 0, true) => format!("Combined from {rips} rips, every sector passes its own check, but it still doesn't match Redump's copy. It may be a version of the disc Redump doesn't list."),
            (false, n, _) => format!("Combined from {rips} rips, {} still {} no copy that passes its check. Another rip may read {} right; cleaning the disc first can help.", sectors(n), if n == 1 { "has" } else { "have" }, if n == 1 { "it" } else { "them" }),
        });
        out.extend(c.left_out.iter().cloned());
        if let Some(r) = &m.repaired {
            out.push(repair_reading(r, m.in_redump()));
        }
    } else if let Some(r) = m.repaired.as_ref().filter(|_| m.fixed.is_some()) {
        out.push(repair_reading(r, m.in_redump()));
    } else if m.fixed.is_some() {
        // Said above.
    } else if !m.in_redump() {
        let wrong = read.map_or(0, |r| r.bad().max(r.wrong()));
        let first = format!("Redump has no copy of this disc to compare with, and {} of this copy fail their own check.", sectors(wrong));
        out.push(match (reported, m.can_combine) {
            (true, _) => format!("{first} The drive reported read errors it couldn't fix, so Re-read bad parts may help."),
            (false, true) => format!("{first} The drive didn't report them as bad, so Re-read bad parts can't find them. {combine}"),
            (false, false) => format!("{first} The drive didn't report them as bad. Clean the disc and rip it again, or try another drive."),
        });
        out.push("With nothing to compare with, a copy can only be kept, never called verified.".into());
    } else if !m.same_count() {
        let (e, g) = (m.files.iter().filter(|p| p.expected.is_some()).count(), m.files.iter().filter(|p| p.got.is_some()).count());
        out.push(format!("Redump's copy has {} and this one has {}, so this is likely a different version of the disc from Redump's.", tracks(e), tracks(g)));
    } else if !m.same_size() {
        out.push("The size differs from Redump's, so this is likely a different version of the disc from Redump's.".into());
    } else if m.files.len() > 1 && m.files[0].sha1_ok() && !m.files[1..].iter().all(Pair::sha1_ok) {
        out.push("The first track matches Redump's, so it's likely the same disc, but the music tracks don't. Music has no checksums of its own, so a drive can read it wrong without noticing. Clean the disc and rip it again, or try another drive.".into());
    } else if bad > 0 && !reported {
        // redumper re-reads only what the drive reported bad: these it won't.
        let next = if m.can_combine { combine.to_owned() } else { "Clean the disc and rip it again, or try another drive.".to_owned() };
        out.push(format!(
            "The size is right, so it's likely the same version of the disc, but {} read wrong. The drive didn't report them as bad, so Re-read bad parts can't find them. {next}",
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
        if let Some(a) = r.of(Kind::Address).filter(|_| m.combined.is_none() && m.fixed.is_none()) {
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

/// What repairing by the disc's error correction came to, in a few
/// sentences: what it put right, what it left and why, and that only
/// Redump can say it's right.
fn repair_reading(r: &Repairing, in_redump: bool) -> String {
    let are = |n: u64| if n == 1 { "is" } else { "are" };
    let mut out = match r.repaired {
        0 => "The disc's error correction repaired nothing.".to_owned(),
        n => format!(
            "The disc's error correction repaired {}: {} read wrong, and the error-correction bytes stored in {} put {} right.",
            sectors(n),
            if n == 1 { "it was" } else { "each was" },
            if n == 1 { "it" } else { "each" },
            "it"
        ),
    };
    if r.held_back > 0 {
        out.push_str(&format!(
            " {} could be repaired too, but every read gave {} back the same, so {} may be pressed that way on purpose and {} left as read.",
            sectors(r.held_back),
            if r.held_back == 1 { "it" } else { "them" },
            if r.held_back == 1 { "it" } else { "they" },
            are(r.held_back)
        ));
    }
    if r.unreadable > 0 {
        out.push_str(&format!(" {} couldn't be read at all, so there's nothing to repair.", sectors(r.unreadable)));
    }
    if r.form2 > 0 {
        out.push_str(&if r.form2 == 1 {
            " 1 sector is a form 2 sector, which has no error correction.".to_owned()
        } else {
            format!(" {} are form 2 sectors, which have no error correction.", r.form2)
        });
    }
    if r.beyond > 0 {
        out.push_str(&format!(" {} {} too damaged, or came back with the wrong address, for it to help.", sectors(r.beyond), are(r.beyond)));
    }
    out.push_str(match (r.verified, in_redump) {
        (true, _) => " It now matches Redump's copy.",
        (false, true) if r.repaired > 0 => " It still doesn't match Redump's copy, so it isn't verified.",
        (false, true) => " It still doesn't match Redump's copy.",
        (false, false) => " Redump has no copy to compare with, so it can't be called verified.",
    });
    out
}

/// What Fix bad sectors came to, in a sentence or two.
fn fix_reading(f: &Fixing, in_redump: bool, combined: bool, repaired: bool) -> String {
    if f.other_disc {
        return "Fix bad sectors changed nothing: sectors that read right the first time came back different, so the disc in the drive may not be this one.".into();
    }
    if f.bad == 0 {
        return "Fix bad sectors found no sector that fails its own check, so there was nothing to read again.".into();
    }
    let read = format!(
        "Fix bad sectors read {} again, up to {} times each, and put {} that passed {} own check in place.",
        sectors(f.bad),
        f.tries,
        if f.fixed == 1 { "the 1 copy".to_owned() } else { format!("the {} copies", f.fixed) },
        if f.fixed == 1 { "its" } else { "their" }
    );
    let left = match (f.left, f.same) {
        (0, _) => String::new(),
        (n, s) if s == n => format!(
            " {} came back the same on every read and {} left as {}: some discs are pressed with sectors like that on purpose.",
            sectors(n),
            if n == 1 { "is" } else { "are" },
            if n == 1 { "it is" } else { "they are" }
        ),
        (n, _) => format!(" {} still {} no copy that passes.", sectors(n), if n == 1 { "has" } else { "have" }),
    };
    let then = match (f.verified, in_redump, combined, f.left) {
        // What came after says the rest.
        _ if repaired && !combined => String::new(),
        (_, _, true, _) => " Then the disc was ripped again, and the rips combined.".to_owned(),
        (true, _, false, _) => " It now matches Redump's copy.".to_owned(),
        (false, false, false, 0) => " Every sector passes its own check now, but Redump has no copy to compare with, so it can't be called verified.".to_owned(),
        (false, false, false, _) => " Redump has no copy to compare with, so it can't be called verified.".to_owned(),
        (false, true, false, 0) => {
            " Every sector passes its own check now, but it still doesn't match Redump's copy: some may be wrong in a way their own check can't show. Rip again and combine compares whole rips."
                .to_owned()
        }
        (false, true, false, _) => " Rip again and combine may find a good copy in another rip.".to_owned(),
    };
    format!("{read}{left}{then}")
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PlayStation disc's log, trimmed, in the shape redumper (b744)
    /// writes it: a drive it doesn't know, no errors reported, and sectors
    /// `info` finds wrong. The names and numbers are made up.
    pub const LOG: &str = "\
=== 2026-01-01 12:00:00 ========================================================
redumper (build: b744)

arguments: dump --drive=/dev/sr0 --retries=20 --image-path=image --image-name=GAME_1
warning: drive read offset not found in the database
warning: drive not found in the database
warning: using generic drive

drive information
  path: /dev/sr0
  inquiry: ACME - DVD-RW XY123 (revision level: 1.00)
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
  Q: 120

*** END (time check: 300s)

*** SPLIT (time check: 0s)

final TOC:
  track 1 {  data }
    index 01 { LBA: [     0 .. 199999], length: 200000, MSF: 00:02:00-44:28:49 }
  track A {  data }
    index 01 { LBA: 200000, MSF: 44:28:50 }

*** END (time check: 10s)

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
        assert_eq!(m.short(), "same size, 40 bad sectors");
        assert!(m.reading[0].starts_with("The size is right, so it's likely the same version of the disc, but 40 sectors read wrong."), "{:?}", m.reading);
        assert!(m.reading[0].contains("Re-read bad parts can't find them. Clean the disc"), "{:?}", m.reading);
        let opts = |m: &Mismatch| m.options().into_iter().map(|o| o.0).collect::<Vec<_>>();
        assert_eq!(opts(&m), ["keep", "leave_out"], "nothing the drive reported: re-reading can't help, and this can't be combined");
        // A CD image redumper made: combining is offered, and said why.
        let m = m.with(true, None);
        assert!(m.reading[0].contains("Rip again and combine reads the whole disc again"), "{:?}", m.reading);
        assert_eq!(opts(&m), ["fix", "combine", "repair", "keep", "leave_out"], "the disc's error correction last");
        assert!(m.reading[0].contains("Fix bad sectors reads just those sectors again, one at a time"), "{:?}", m.reading);
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
        assert!(m.reading[0].starts_with("The first track matches Redump's, so it's likely the same disc, but the music tracks don't."));
        assert!(explain("SLUS-00002", &[], &dumped, None).is_none(), "nothing to compare with");
    }

    #[test]
    fn what_fixing_came_to_is_said_plainly() {
        let cat = dat();
        let copies = cat.by_serial("SLES-04711");
        let dumped = [rom("GAME_1.bin", BIN, "bbbb000000000000000000000000000000000000")];
        let m = explain("SLES-04711", &copies, &dumped, Some(LOG)).unwrap().with(true, None);
        let f = Fixing { bad: 212, fixed: 211, left: 1, left_at: vec![98000], same: 0, tries: 10, other_disc: false, verified: false, ..Fixing::default() };
        let fixed = m.clone().fixing(Some(f.clone()));
        assert_eq!(fixed.short(), "211 sectors re-read, 1 sector still bad");
        assert_eq!(
            fixed.reading[0],
            "Fix bad sectors read 212 sectors again, up to 10 times each, and put the 211 copies that passed their own check in place. 1 sector still has no copy that passes. Rip again and combine may find a good copy in another rip."
        );
        assert!(!fixed.reading.iter().any(|l| l.contains("wrong address")), "the first rip's addresses are old news: {:?}", fixed.reading);
        let opts = |m: &Mismatch| m.options().into_iter().map(|o| o.0).collect::<Vec<_>>();
        assert_eq!(opts(&fixed), ["combine", "repair", "keep", "leave_out"], "fixed once is enough");
        // Pressed so: the same on every read.
        let pressed = m.clone().fixing(Some(Fixing { same: 1, ..f.clone() }));
        assert!(pressed.reading[0].contains("1 sector came back the same on every read and is left as it is: some discs are pressed with sectors like that on purpose."), "{:?}", pressed.reading);
        let other = m.clone().fixing(Some(Fixing { other_disc: true, ..Fixing::default() }));
        assert!(other.reading[0].contains("the disc in the drive may not be this one"));
        assert_eq!(opts(&other), ["combine", "keep", "leave_out"], "nothing to repair when nothing's been looked at");
        // The notes.
        let c = Combination { rips: vec![RipRead { n: 1, bad: 1 }, RipRead { n: 2, bad: 200 }], taken: 1, ..Combination::default() };
        assert_eq!(result_note(Some(&f), Some(&c)), "211 sectors re-read, 1 from a second rip");
        assert_eq!(result_note(Some(&f), None), "211 sectors re-read");
        let c3 = Combination { rips: vec![RipRead::default(); 3], taken: 4, ..Combination::default() };
        assert_eq!(result_note(Some(&f), Some(&c3)), "211 sectors re-read, 4 from 2 more rips");
        assert_eq!(result_note(None, Some(&c3)), "combined from 3 rips, 4 sectors taken from the later rips");
        assert_eq!(result_note(Some(&Fixing::default()), Some(&c3)), c3.note(), "nothing re-read: the combination alone");
    }

    #[test]
    fn what_repairing_came_to_is_said_plainly() {
        let cat = dat();
        let copies = cat.by_serial("SLES-04711");
        let dumped = [rom("GAME_1.bin", BIN, "bbbb000000000000000000000000000000000000")];
        let m = explain("SLES-04711", &copies, &dumped, Some(LOG)).unwrap().with(true, None);
        let opts = |m: &Mismatch| m.options().into_iter().map(|o| o.0).collect::<Vec<_>>();
        let labels = |m: &Mismatch| m.options().into_iter().map(|o| o.1).collect::<Vec<_>>();
        assert_eq!(labels(&m)[2], "Repair with the disc's error correction");
        let r = Repairing { bad: 6, repaired: 3, held_back: 1, unreadable: 1, form2: 1, beyond: 0, verified: false };
        let rep = m.clone().repairing(Some(r.clone()));
        assert_eq!(rep.short(), "3 sectors repaired by error correction, 3 sectors still bad");
        assert_eq!(
            rep.reading[0],
            "The disc's error correction repaired 3 sectors: each was read wrong, and the error-correction bytes stored in each put it right. 1 sector could be repaired too, but every read gave it back the same, so it may be pressed that way on purpose and is left as read. 1 sector couldn't be read at all, so there's nothing to repair. 1 sector is a form 2 sector, which has no error correction. It still doesn't match Redump's copy, so it isn't verified."
        );
        assert_eq!(opts(&rep), ["fix", "combine", "keep", "leave_out"], "once is enough");
        let ok = m.clone().repairing(Some(Repairing { verified: true, held_back: 0, unreadable: 0, form2: 0, ..r.clone() }));
        assert!(ok.reading[0].ends_with("put it right. It now matches Redump's copy."), "{:?}", ok.reading);
        let alone = unlisted("SLES-04711", &dumped, LOG).with(true, None).repairing(Some(Repairing { bad: 3, repaired: 3, ..Repairing::default() }));
        assert!(alone.reading[0].ends_with("Redump has no copy to compare with, so it can't be called verified."), "{:?}", alone.reading);
        assert_eq!(r.note(), "3 sectors repaired by the disc's error correction");
        assert_eq!(and_repaired("2 sectors re-read".into(), Some(&r)), "2 sectors re-read, 3 sectors repaired by the disc's error correction");
        assert_eq!(and_repaired(String::new(), Some(&r)), "3 sectors repaired by the disc's error correction");
        assert_eq!(and_repaired("2 sectors re-read".into(), Some(&Repairing::default())), "2 sectors re-read");
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
