//! A disc as a map of its sectors: blocks of sectors in order, each with
//! the worst that happened to any sector in it, like an old defragmenter's
//! map of a hard disk. Where its data comes from is not its business: Rip
//! fills it from redumper's `.state` file as it reads
//! ([`crate::real::redumper::StateScan`]), Check and fix from the sectors it
//! found bad and fixed.
//!
//! With it, for a CD: how far from the centre each sector lies, and
//! whether bad sectors come back once per turn of the disc, which is a
//! single spot (a scratch, a speck) rather than a whole ring.
//!
//! Only the distance from the centre can be worked out. Where round the
//! disc a sector lies (its angle) can't be known well enough to draw: it
//! depends on the exact speed and track pitch the disc was pressed with,
//! and an error of a fraction of a percent in either adds up over tens of
//! thousands of turns to anywhere at all. So the map is drawn as rows of
//! blocks, never as a disc.

use serde::{Deserialize, Serialize};

/// What happened to a sector, from best to worst: a block shows the worst
/// of its sectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Mark {
    /// Not read yet.
    NotRead = 0,
    /// Read cleanly.
    Good = 1,
    /// Bad on the first read, good on a later one.
    Recovered = 2,
    /// Read, but the drive reported C2 errors in it.
    C2 = 3,
    /// Read without the drive saying anything, but its own checks fail.
    Wrong = 4,
    /// The drive couldn't read it.
    Unreadable = 5,
}

impl Mark {
    pub const ALL: [Mark; 6] = [Mark::NotRead, Mark::Good, Mark::Recovered, Mark::C2, Mark::Wrong, Mark::Unreadable];

    fn of(n: u8) -> Mark {
        Mark::ALL.get(n as usize).copied().unwrap_or(Mark::NotRead)
    }

    /// Whether it's a sector that's still wrong.
    pub fn is_bad(self) -> bool {
        matches!(self, Mark::C2 | Mark::Wrong | Mark::Unreadable)
    }

    /// Its letter in [`SectorMap::marks`].
    fn letter(self) -> char {
        (b'0' + self as u8) as char
    }
}

/// About this many blocks to a map: ~36 to a row, ~28 rows.
pub const BLOCKS: u64 = 1_000;

/// Each sector's mark, one by one, as whatever reads the disc learns them;
/// summed into a [`SectorMap`] to show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sectors {
    marks: Vec<Mark>,
}

impl Sectors {
    /// A disc of `n` sectors, none read yet.
    pub fn new(n: u64) -> Sectors {
        Sectors { marks: vec![Mark::NotRead; n as usize] }
    }

    pub fn len(&self) -> u64 {
        self.marks.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    pub fn get(&self, lba: u64) -> Mark {
        self.marks.get(lba as usize).copied().unwrap_or(Mark::NotRead)
    }

    /// Sets one sector's mark; none past the disc's end.
    pub fn set(&mut self, lba: u64, m: Mark) {
        if let Some(x) = self.marks.get_mut(lba as usize) {
            *x = m;
        }
    }

    /// The disc grown to `n` sectors, when a read finds it's longer.
    pub fn grow(&mut self, n: u64) {
        if n as usize > self.marks.len() {
            self.marks.resize(n as usize, Mark::NotRead);
        }
    }

    /// The sectors still bad, in order.
    pub fn bad(&self) -> impl Iterator<Item = u64> + '_ {
        self.marks.iter().enumerate().filter(|(_, m)| m.is_bad()).map(|(i, _)| i as u64)
    }

    /// The map to show: the sectors summed into about [`BLOCKS`] blocks,
    /// the drive at `at` when it's reading, and for a CD the distances
    /// from the centre (`cd`).
    pub fn map(&self, at: Option<u64>, cd: bool) -> SectorMap {
        let n = self.len();
        let per = n.div_ceil(BLOCKS).max(1);
        let blocks = n.div_ceil(per) as usize;
        let mut marks = String::with_capacity(blocks);
        let mut mixed = vec![];
        let mut totals = [0u64; 6];
        for b in 0..blocks {
            let mut c = [0u32; 6];
            for m in &self.marks[b * per as usize..((b + 1) * per as usize).min(n as usize)] {
                c[*m as usize] += 1;
            }
            for (t, x) in totals.iter_mut().zip(c) {
                *t += x as u64;
            }
            marks.push(worst(&c).letter());
            if c.iter().filter(|x| **x > 0).count() > 1 {
                mixed.push((b as u32, c));
            }
        }
        let runs = runs_of(self.marks.iter().enumerate().filter(|(_, m)| m.is_bad()).map(|(i, _)| i as u64));
        let recovered = runs_of(self.marks.iter().enumerate().filter(|(_, m)| **m == Mark::Recovered).map(|(i, _)| i as u64));
        SectorMap { sectors: n, per_block: per, marks, mixed, totals, at, cd, runs, recovered, checks_only: false }
    }
}

/// Sectors in order as stretches, the first [`MAX_RUNS`] of them.
fn runs_of(lbas: impl Iterator<Item = u64>) -> Vec<Run> {
    let mut runs: Vec<Run> = vec![];
    for lba in lbas {
        let full = runs.len() >= MAX_RUNS;
        match runs.last_mut() {
            Some(r) if r.to + 1 == lba => r.to = lba,
            _ if full => break,
            _ => runs.push(Run { from: lba, to: lba }),
        }
    }
    runs
}

/// The sectors of stretches, one by one.
fn each(runs: &[Run]) -> impl Iterator<Item = u64> + '_ {
    runs.iter().flat_map(|r| r.from..=r.to)
}

/// The worst mark of a block's counts; not read only when nothing in it is.
fn worst(c: &[u32; 6]) -> Mark {
    Mark::ALL.iter().rev().copied().find(|m| *m != Mark::NotRead && c[*m as usize] > 0).unwrap_or(Mark::NotRead)
}

/// At most this many stretches of bad sectors are kept with a map, for
/// where they are and whether they come once per turn.
const MAX_RUNS: usize = 20_000;

/// A stretch of bad sectors, `from` to `to`, both in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub from: u64,
    pub to: u64,
}

/// The disc's sectors in blocks, as a step saw them last: what the
/// window draws.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SectorMap {
    /// The disc's sectors, from LBA 0.
    pub sectors: u64,
    /// Sectors to a block; the last block may have fewer.
    pub per_block: u64,
    /// Each block's worst mark, as its digit ([`Mark`]'s number).
    pub marks: String,
    /// The blocks with more than one kind of sector in them, and how many
    /// of each, by [`Mark`]'s number. A block not here is all its mark.
    #[serde(default)]
    pub mixed: Vec<(u32, [u32; 6])>,
    /// How many sectors of each mark on the whole disc.
    pub totals: [u64; 6],
    /// Where the drive is, while it reads.
    #[serde(default)]
    pub at: Option<u64>,
    /// A CD, whose sectors' distances from the centre are known.
    #[serde(default)]
    pub cd: bool,
    /// Where the bad sectors are, as stretches (the first [`MAX_RUNS`]).
    #[serde(default)]
    pub runs: Vec<Run>,
    /// Where the sectors that came back on a later read are.
    #[serde(default)]
    pub recovered: Vec<Run>,
    /// Made from an image's own checks of its sectors, not from a read:
    /// only the sectors that failed them, and what came of each, say
    /// anything ([`SectorMap::checked`]).
    #[serde(default)]
    pub checks_only: bool,
}

impl SectorMap {
    pub fn blocks(&self) -> usize {
        self.marks.len()
    }

    pub fn mark(&self, b: usize) -> Mark {
        self.marks.as_bytes().get(b).map_or(Mark::NotRead, |c| Mark::of(c.wrapping_sub(b'0')))
    }

    /// The sectors block `b` covers, first and last.
    pub fn range(&self, b: usize) -> (u64, u64) {
        let from = b as u64 * self.per_block;
        (from, (from + self.per_block).min(self.sectors).saturating_sub(1).max(from))
    }

    /// How many sectors of each mark block `b` has.
    pub fn counts(&self, b: usize) -> [u32; 6] {
        if let Some((_, c)) = self.mixed.iter().find(|(k, _)| *k as usize == b) {
            return *c;
        }
        let (a, z) = self.range(b);
        let mut c = [0u32; 6];
        c[self.mark(b) as usize] = (z - a + 1) as u32;
        c
    }

    /// The block the drive is reading, while it reads.
    pub fn reading(&self) -> Option<usize> {
        self.at.filter(|a| *a < self.sectors.max(1)).map(|a| (a / self.per_block.max(1)) as usize)
    }

    pub fn total(&self, m: Mark) -> u64 {
        self.totals[m as usize]
    }

    /// The geometry to work out distances from the centre with: a CD's,
    /// when the map is of one no longer than a CD can be.
    pub fn geometry(&self) -> Option<Geometry> {
        (self.cd && self.sectors <= CD_MAX_SECTORS).then_some(Geometry::CD)
    }

    pub fn is_empty(&self) -> bool {
        self.sectors == 0
    }

    /// The map of an image of `sectors` checked sector by sector by its
    /// own checks (Check and fix): each in `found` failed them, and came
    /// to its mark (still [`Mark::Wrong`], or [`Mark::Recovered`] once read
    /// right again); the others passed.
    pub fn checked(sectors: u64, found: &[(u64, Mark)], cd: bool) -> SectorMap {
        let mut s = Sectors { marks: vec![Mark::Good; sectors as usize] };
        for (lba, m) in found {
            s.set(*lba, *m);
        }
        SectorMap { checks_only: true, ..s.map(None, cd) }
    }

    /// The same map with what another look at some sectors found: each to
    /// its mark. Where each was before is known only as bad, come back, or
    /// neither, from the map's stretches; which kind of bad, from its
    /// block's counts. One already bad that's found bad again stays as it
    /// was; one come back already stays so.
    pub fn overlay(&mut self, found: &[(u64, Mark)]) {
        if found.is_empty() {
            return;
        }
        let mut bad: std::collections::BTreeSet<u64> = each(&self.runs).collect();
        let mut back: std::collections::BTreeSet<u64> = each(&self.recovered).collect();
        for &(lba, to) in found {
            if lba >= self.sectors {
                continue;
            }
            let was_bad = bad.contains(&lba);
            let b = (lba / self.per_block.max(1)) as usize;
            let mut c = self.counts(b);
            let from = match to {
                _ if to.is_bad() && was_bad => continue,
                Mark::Recovered if back.contains(&lba) => continue,
                _ if was_bad => [Mark::Unreadable, Mark::Wrong, Mark::C2].into_iter().find(|m| c[*m as usize] > 0),
                _ if back.contains(&lba) => Some(Mark::Recovered),
                _ => [Mark::Good, Mark::NotRead].into_iter().find(|m| c[*m as usize] > 0),
            };
            let Some(from) = from.filter(|f| *f != to) else { continue };
            c[from as usize] -= 1;
            c[to as usize] += 1;
            self.totals[from as usize] -= 1;
            self.totals[to as usize] += 1;
            let mut marks = std::mem::take(&mut self.marks).into_bytes();
            marks[b] = worst(&c).letter() as u8;
            self.marks = String::from_utf8(marks).unwrap_or_default();
            self.mixed.retain(|(k, _)| *k as usize != b);
            if c.iter().filter(|x| **x > 0).count() > 1 {
                let at = self.mixed.partition_point(|x| (x.0 as usize) < b);
                self.mixed.insert(at, (b as u32, c));
            }
            bad.remove(&lba);
            back.remove(&lba);
            if to.is_bad() {
                bad.insert(lba);
            } else if to == Mark::Recovered {
                back.insert(lba);
            }
        }
        self.runs = runs_of(bad.into_iter());
        self.recovered = runs_of(back.into_iter());
    }

    /// Each sector a map made from checks found something in, and what.
    pub fn found(&self) -> Vec<(u64, Mark)> {
        let mut out: Vec<(u64, Mark)> = each(&self.recovered).map(|l| (l, Mark::Recovered)).collect();
        out.extend(each(&self.runs).map(|l| (l, Mark::Wrong)));
        out
    }
}

/// The map Rip's sidebar shows: Rip's own (`rip`), with the sectors its
/// check of every sector found wrong (`wrong`) over it; then Check and
/// fix's (`check`), if it has made one since. A map Check made from the
/// image's checks alone goes over Rip's; one it made from reading the disc
/// again takes its place, keeping where Rip's re-reads brought sectors
/// back unless Check finds them bad.
pub fn combined(rip: Option<SectorMap>, wrong: &[u64], check: Option<SectorMap>) -> Option<SectorMap> {
    let mut m = rip.filter(|m| !m.is_empty());
    if let Some(m) = m.as_mut().filter(|m| !m.checks_only) {
        m.overlay(&wrong.iter().map(|l| (*l, Mark::Wrong)).collect::<Vec<_>>());
    }
    match (check.filter(|c| !c.is_empty()), m) {
        (Some(c), Some(mut m)) if c.checks_only => {
            m.overlay(&c.found());
            Some(m)
        }
        (Some(mut c), Some(m)) => {
            let bad: std::collections::BTreeSet<u64> = each(&c.runs).collect();
            let back: Vec<(u64, Mark)> = each(&m.recovered).filter(|l| !bad.contains(l)).map(|l| (l, Mark::Recovered)).collect();
            c.overlay(&back);
            Some(c)
        }
        (c, m) => c.or(m),
    }
}

/// The most sectors a CD holds: 99 minutes and change, past any real one.
const CD_MAX_SECTORS: u64 = 100 * 60 * 75;

// ---- where on the disc ---------------------------------------------------------------------

/// How a disc's sectors lie on it, for their distance from the centre: a
/// spiral read at a constant speed along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// Where its first sector (LBA 0) lies, in metres from the centre.
    pub r0: f64,
    /// The speed along the spiral, in metres a second.
    pub speed: f64,
    /// From one turn of the spiral to the next, in metres.
    pub pitch: f64,
    /// Sectors read a second at that speed.
    pub per_sec: f64,
}

impl Geometry {
    /// A CD by the standard's nominal figures (IEC 60908): the program
    /// area from 25 mm, read at 1.3 m/s (the standard allows 1.2 to 1.4),
    /// tracks 1.6 µm apart, 75 sectors a second.
    pub const CD: Geometry = Geometry { r0: 0.025, speed: 1.3, pitch: 1.6e-6, per_sec: 75.0 };

    /// How long a sector is along the spiral, in metres.
    pub fn sector_len(&self) -> f64 {
        self.speed / self.per_sec
    }

    /// How far sector `lba` is from the centre, in metres: the area of the
    /// ring up to it is its length along the spiral times the pitch, so
    /// π(r² − r0²) = n · sector length · pitch.
    pub fn radius(&self, lba: f64) -> f64 {
        (self.r0 * self.r0 + lba.max(0.0) * self.sector_len() * self.pitch / std::f64::consts::PI).sqrt()
    }

    /// Sectors in one turn of the spiral at sector `lba`.
    pub fn per_turn(&self, lba: f64) -> f64 {
        std::f64::consts::TAU * self.radius(lba) / self.sector_len()
    }
}

/// A distance from the centre, rounded to the millimetre, in centimetres:
/// *4.4 cm*.
pub fn cm(r: f64) -> String {
    let mm = (r * 1000.0).round() as i64;
    format!("{}.{} cm", mm / 10, mm % 10)
}

/// The places the bad sectors are, by distance from the centre: stretches
/// of bad sectors no more than a millimetre apart gathered into one place,
/// as their nearest and furthest distances in metres, the places with the
/// most bad sectors first (at most `most`).
pub fn places(runs: &[Run], g: &Geometry, most: usize) -> Vec<(f64, f64, u64)> {
    let mut out: Vec<(f64, f64, u64)> = vec![];
    for r in runs {
        let (a, b) = (g.radius(r.from as f64), g.radius(r.to as f64 + 1.0));
        let n = r.to - r.from + 1;
        match out.last_mut() {
            Some(p) if a - p.1 <= 0.001 => {
                p.1 = b;
                p.2 += n;
            }
            _ => out.push((a, b, n)),
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.2));
    out.truncate(most);
    out.sort_by(|x, y| x.0.total_cmp(&y.0));
    out
}

/// Where the bad sectors are, in a line: *Bad sectors about 4.2–4.4 cm
/// from the centre*; *… 2.9 cm and 4.2–4.4 cm …*. None without any.
pub fn where_line(m: &SectorMap) -> Option<String> {
    let g = m.geometry()?;
    // The bad sectors; with none left, where the ones that came back were.
    let (runs, what) = if m.runs.is_empty() { (&m.recovered, "Sectors re-read") } else { (&m.runs, "Bad sectors") };
    let all = places(runs, &g, usize::MAX);
    if all.is_empty() {
        return None;
    }
    let one = |p: &(f64, f64, u64)| {
        let (a, b) = (cm(p.0), cm(p.1));
        if a == b {
            a
        } else {
            format!("{}–{b}", a.trim_end_matches(" cm"))
        }
    };
    if all.len() > 3 {
        // Many places: the nearest and furthest of them.
        let far = all.iter().map(|p| p.1).fold(0.0, f64::max);
        return Some(format!("{what} between about {} and {} from the centre", cm(all[0].0), cm(far)));
    }
    let shown: Vec<String> = all.iter().map(one).collect();
    let list = match shown.as_slice() {
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        _ => format!("{}, {} and {}", shown[0], shown[1], shown[2]),
    };
    Some(format!("{what} about {list} from the centre"))
}

// ---- once per turn -------------------------------------------------------------------------

/// Stretches of bad sectors further apart than this many turns start a
/// new place.
const APART_TURNS: f64 = 4.0;
/// A place needs this many gaps between its stretches to say anything.
const MIN_GAPS: usize = 5;
/// The gaps may differ from what one turn should be by this much: the
/// speed a disc was pressed at is 1.2 to 1.4 m/s, ±8% of 1.3.
const NOMINAL: f64 = 0.09;
/// A gap fits the turn when it's within this many sectors of a whole
/// number of turns: a stretch may start a sector early or late.
const SLACK: f64 = 1.5;
/// This share of the gaps must fit.
const FIT: f64 = 0.8;
/// This share of a place's stretches must be no longer than a third of
/// a turn.
const SHORT: f64 = 0.9;

/// The places where bad sectors come once per turn of the disc: one spot
/// met again each time round, not a whole ring. For each, how far from the
/// centre (metres) and the sectors in a turn there as measured.
///
/// The stretches are gathered into places (no more than four turns
/// apart). In each, the gaps from one stretch's start to the next must be
/// whole turns (one, or two or three where a turn read cleanly) and agree
/// with each other to within a sector or so over five turns and more; and
/// that turn must be about what the geometry says it is there, loosely,
/// since a disc's speed varies by up to 8% between pressings.
pub fn once_per_turn(runs: &[Run], g: &Geometry) -> Vec<(f64, f64)> {
    let mut out = vec![];
    let mut place: Vec<&Run> = vec![];
    let mut flush = |place: &mut Vec<&Run>| {
        if let Some(p) = periodic(place, g) {
            out.push(p);
        }
        place.clear();
    };
    for r in runs {
        if let Some(last) = place.last() {
            if (r.from - last.from) as f64 > APART_TURNS * g.per_turn(r.from as f64) {
                flush(&mut place);
            }
        }
        place.push(r);
    }
    flush(&mut place);
    out
}

/// Whether one place's stretches come once per turn: its distance and the
/// turn measured.
fn periodic(place: &[&Run], g: &Geometry) -> Option<(f64, f64)> {
    if place.len() < MIN_GAPS + 1 {
        return None;
    }
    let mid = (place[0].from + place[place.len() - 1].to) as f64 / 2.0;
    let want = g.per_turn(mid);
    let gaps: Vec<f64> = place.windows(2).map(|w| (w[1].from - w[0].from) as f64).collect();
    // The turn as the single gaps say it, then fitted to all that fit.
    let mut ones: Vec<f64> = gaps.iter().copied().filter(|x| (x / want - 1.0).abs() <= NOMINAL * 1.5).collect();
    if ones.len() < MIN_GAPS.min(gaps.len()) / 2 + 1 {
        return None;
    }
    ones.sort_by(f64::total_cmp);
    let mut turn = ones[ones.len() / 2];
    for _ in 0..2 {
        let (mut sum, mut turns) = (0.0, 0.0);
        for x in &gaps {
            let k = (x / turn).round();
            if (1.0..=3.0).contains(&k) && (x - k * turn).abs() <= SLACK * k {
                sum += x;
                turns += k;
            }
        }
        if turns == 0.0 {
            return None;
        }
        turn = sum / turns;
    }
    let fits = gaps
        .iter()
        .filter(|x| {
            let k = (*x / turn).round();
            (1.0..=3.0).contains(&k) && (*x - k * turn).abs() <= SLACK
        })
        .count();
    let singles = gaps.iter().filter(|x| (*x - turn).abs() <= SLACK).count();
    // A spot, not a band: its stretches short beside a turn, and none
    // as long as one (a whole turn bad is a ring, there).
    let lens: Vec<f64> = place.iter().map(|r| (r.to - r.from + 1) as f64).collect();
    let short = lens.iter().filter(|l| **l <= turn / 3.0).count();
    let spot = short as f64 >= SHORT * lens.len() as f64 && lens.iter().all(|l| *l < turn);
    let ok = spot && fits as f64 >= FIT * gaps.len() as f64 && singles >= MIN_GAPS.min(gaps.len()) && (turn / want - 1.0).abs() <= NOMINAL;
    ok.then(|| (g.radius(mid), turn))
}

/// What the map says of bad sectors that come once per turn, when they
/// do: *Errors repeat once per turn: a single spot about 4.4 cm out, not a
/// whole ring.* Nothing otherwise.
pub fn turn_line(m: &SectorMap) -> Option<String> {
    let g = m.geometry()?;
    // Every error as it was read, whether a re-read put it right or not.
    let mut all: Vec<Run> = m.runs.iter().chain(&m.recovered).copied().collect();
    all.sort_by_key(|r| r.from);
    let spots = once_per_turn(&runs_of(all.iter().flat_map(|r| r.from..=r.to)), &g);
    let mut at: Vec<String> = vec![];
    for (r, _) in &spots {
        let c = cm(*r);
        if !at.contains(&c) {
            at.push(c);
        }
    }
    match at.as_slice() {
        [] => None,
        [one] => Some(format!("Errors repeat once per turn: a single spot about {one} out, not a whole ring.")),
        many => {
            let list = format!("{} and {}", many[..many.len() - 1].join(", "), many[many.len() - 1]);
            Some(format!("Errors repeat once per turn: single spots about {list} out, not whole rings."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cd_s_first_sector_is_25_mm_out_and_an_hour_s_end_about_54() {
        let g = Geometry::CD;
        assert!((g.radius(0.0) - 0.025).abs() < 1e-9);
        let end = g.radius(60.0 * 60.0 * 75.0);
        assert!((0.053..=0.055).contains(&end), "{end}");
        // About 16 sectors a turn at 43 mm, as a real disc's errors repeat.
        let at = g.per_turn(147_107.0);
        assert!((15.0..17.0).contains(&at), "{at}");
        assert_eq!(cm(0.04386), "4.4 cm");
        assert_eq!(cm(0.025), "2.5 cm");
    }

    fn sectors(n: u64, bad: &[(u64, Mark)]) -> Sectors {
        let mut s = Sectors::new(n);
        for k in 0..n {
            s.set(k, Mark::Good);
        }
        for (k, m) in bad {
            s.set(*k, *m);
        }
        s
    }

    #[test]
    fn a_block_takes_its_worst_sector_and_counts_each_kind() {
        // 10,000 sectors: 10 to a block, 1,000 blocks.
        let mut s = sectors(10_000, &[(12, Mark::C2), (13, Mark::Unreadable), (25, Mark::Recovered), (31, Mark::C2)]);
        for k in 9_995..10_000 {
            s.set(k, Mark::NotRead);
        }
        let m = s.map(Some(9_995), true);
        assert_eq!((m.per_block, m.blocks()), (10, 1_000));
        assert_eq!([m.mark(0), m.mark(1), m.mark(2), m.mark(3)], [Mark::Good, Mark::Unreadable, Mark::Recovered, Mark::C2]);
        let c = m.counts(1);
        assert_eq!((c[Mark::Good as usize], c[Mark::C2 as usize], c[Mark::Unreadable as usize]), (8, 1, 1));
        assert_eq!(m.counts(0)[Mark::Good as usize], 10);
        assert_eq!(m.range(1), (10, 19));
        // Partly read: its read sectors say, not the ones still to come.
        assert_eq!(m.mark(999), Mark::Good);
        assert_eq!(m.reading(), Some(999));
        assert_eq!((m.total(Mark::Good), m.total(Mark::C2), m.total(Mark::Unreadable), m.total(Mark::NotRead)), (9_991, 2, 1, 5));
        assert_eq!(m.runs, [Run { from: 12, to: 13 }, Run { from: 31, to: 31 }]);
        // A block none of whose sectors is read yet isn't anything.
        assert_eq!(Sectors::new(100).map(None, false).mark(0), Mark::NotRead);
        // It goes to the window and back the same.
        let back: SectorMap = serde_json::from_value(serde_json::to_value(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    /// Bad stretches two sectors long, `turn` apart from `from`, `n` of
    /// them, with the turns in `skip` read cleanly.
    fn spot(from: u64, turn: f64, n: usize, skip: &[usize]) -> Vec<Run> {
        (0..n).filter(|k| !skip.contains(k)).map(|k| from + (k as f64 * turn).round() as u64).map(|a| Run { from: a, to: a + 1 }).collect()
    }

    #[test]
    fn errors_once_per_turn_are_a_single_spot() {
        let g = Geometry::CD;
        // As a real disc's: 16.3 apart at 147,107, some turns clean.
        let runs = spot(147_107, 16.3, 40, &[3, 9, 10, 20]);
        let found = once_per_turn(&runs, &g);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!((found[0].1 - 16.3).abs() < 0.2, "{found:?}");
        let m = SectorMap { sectors: 300_000, per_block: 300, cd: true, runs, ..Default::default() };
        assert_eq!(turn_line(&m).as_deref(), Some("Errors repeat once per turn: a single spot about 4.4 cm out, not a whole ring."));
        assert_eq!(where_line(&m).as_deref(), Some("Bad sectors about 4.4 cm from the centre"));
        // Put right on a re-read: still once per turn, and where they were.
        let m = SectorMap { recovered: m.runs.clone(), runs: vec![], ..m };
        assert!(turn_line(&m).is_some());
        assert_eq!(where_line(&m).as_deref(), Some("Sectors re-read about 4.4 cm from the centre"));
    }

    #[test]
    fn errors_that_dont_repeat_with_the_turn_say_nothing() {
        let g = Geometry::CD;
        // A ring: one long stretch.
        assert!(once_per_turn(&[Run { from: 147_000, to: 149_000 }], &g).is_empty());
        // Too few to tell.
        assert!(once_per_turn(&spot(147_107, 16.3, 4, &[]), &g).is_empty());
        // Regular, but not the turn: every 9 sectors.
        assert!(once_per_turn(&spot(147_107, 9.0, 40, &[]), &g).is_empty());
        // Once per turn, but some stretches a whole turn long: a band.
        let mut band = spot(147_107, 16.3, 40, &[]);
        band[20].to = band[20].from + 20;
        assert!(once_per_turn(&band, &g).is_empty());
        // Scattered.
        let scattered: Vec<Run> = [0u64, 7, 29, 31, 52, 90, 97, 133, 140, 171].iter().map(|k| Run { from: 147_000 + k, to: 147_000 + k }).collect();
        assert!(once_per_turn(&scattered, &g).is_empty());
        // Not a CD: no distances at all.
        let m = SectorMap { sectors: 2_000_000, cd: false, runs: spot(147_107, 16.3, 40, &[]), ..Default::default() };
        assert_eq!((turn_line(&m), where_line(&m)), (None, None));
    }

    #[test]
    fn what_a_check_finds_goes_over_what_a_read_found() {
        // A read: 3 unreadable in block 0; then its check finds 50 wrong,
        // and fixing brings back 2 of the unreadable and 50.
        let m0 = sectors(10_000, &[(1, Mark::Unreadable), (2, Mark::Unreadable), (3, Mark::Unreadable)]).map(None, true);
        let mut m = m0.clone();
        m.overlay(&[(50, Mark::Wrong), (3, Mark::Wrong)]);
        assert_eq!((m.mark(5), m.total(Mark::Wrong), m.total(Mark::Good)), (Mark::Wrong, 1, 9_996));
        let fixed = SectorMap::checked(10_000, &[(1, Mark::Recovered), (2, Mark::Recovered), (50, Mark::Recovered)], true);
        assert!(fixed.checks_only);
        assert_eq!(fixed.total(Mark::Recovered), 3);
        m.overlay(&fixed.found());
        assert_eq!((m.mark(0), m.mark(5)), (Mark::Unreadable, Mark::Recovered));
        assert_eq!((m.total(Mark::Unreadable), m.total(Mark::Recovered), m.total(Mark::Wrong)), (1, 3, 0));
        assert_eq!(m.runs, [Run { from: 3, to: 3 }]);
        assert_eq!(m.recovered, [Run { from: 1, to: 2 }, Run { from: 50, to: 50 }]);
        let c = m.counts(0);
        assert_eq!((c[Mark::Good as usize], c[Mark::Recovered as usize], c[Mark::Unreadable as usize]), (7, 2, 1));
    }

    #[test]
    fn rip_s_map_takes_check_s_after_it() {
        let rip = sectors(10_000, &[(1, Mark::Unreadable), (2, Mark::Recovered), (3, Mark::Unreadable)]).map(None, true);
        // The check of every sector found 50 wrong; Check and fix then put
        // 50 right and left 3.
        let fixes = SectorMap::checked(10_000, &[(50, Mark::Recovered), (3, Mark::Wrong)], true);
        let m = combined(Some(rip.clone()), &[50], Some(fixes)).unwrap();
        assert_eq!((m.total(Mark::Unreadable), m.total(Mark::Recovered), m.total(Mark::Wrong)), (2, 2, 0));
        // Check read the disc again itself: its map, with where Rip's
        // re-read brought sectors back, unless Check finds them bad.
        let again = sectors(10_000, &[(3, Mark::C2)]).map(None, true);
        let m = combined(Some(rip.clone()), &[], Some(again)).unwrap();
        assert_eq!((m.total(Mark::Unreadable), m.total(Mark::Recovered), m.total(Mark::C2)), (0, 1, 1));
        assert_eq!(m.recovered, [Run { from: 2, to: 2 }]);
        // Neither, or only one.
        assert_eq!(combined(None, &[], None), None);
        assert_eq!(combined(Some(rip.clone()), &[], None), Some(rip));
    }

    #[test]
    fn two_places_are_said_apart() {
        let m = SectorMap { sectors: 300_000, cd: true, runs: [spot(20_000, 10.0, 3, &[]), spot(147_107, 16.3, 3, &[])].concat(), ..Default::default() };
        let l = where_line(&m).unwrap();
        assert!(l.starts_with("Bad sectors about 2.") && l.contains(" and 4.4 cm") && l.ends_with(" from the centre"), "{l}");
    }
}
