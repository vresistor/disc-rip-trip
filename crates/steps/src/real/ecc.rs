//! A CD-ROM data sector's error correction (ECMA-130, Annex A): making
//! its P and Q parity, and putting a sector right by them when a few of
//! its bytes were read wrong.
//!
//! A Mode 1 sector, or a Mode 2 form 1 sector (most of a PlayStation
//! disc), carries 276 bytes of parity after its EDC: P, 172 bytes, and
//! Q, 104. Taken from its header on, the sector's 2,340 bytes are a
//! Reed–Solomon product code over GF(2^8) (x^8 + x^4 + x^3 + x^2 + 1):
//! 86 P codewords of 26 bytes (24 of the sector and 2 of parity), one for
//! each column, and 52 Q codewords of 45 (43 and 2), one for each
//! diagonal, P's parity among them. A Mode 2 sector's parity is worked out
//! with its 4-byte header taken as zeros, so it can't vouch for the
//! header. A form 2 sector has no parity at all: nothing here can put one
//! right.
//!
//! Each codeword corrects one wrong byte by itself. P and Q cross, so a
//! byte P can't put right (two wrong in its column) Q may, and the other
//! way round: passes of P then Q go on until nothing changes. A sector is
//! taken as put right only when every P and Q codeword then fits and so
//! does its EDC, a CRC-32 the parity knows nothing of; else it's given up
//! and left as it was. Two codewords can be put "right" into a third
//! wrong byte; the EDC is what catches that.
//!
//! Put right by its parity is still not the same as read right. Only
//! Redump's checksums of the whole image say it's the disc; this module
//! never says so.

use super::combine::{self, Trust, SECTOR};
use super::fix::{self, Place};
use super::mismatch::{self, Repairing};
use std::io;
use std::path::{Path, PathBuf};

/// The sector from its header on: what the parity covers.
const LEN: usize = SECTOR - 12;
/// Where P and Q start in [`LEN`]'s bytes.
const P_AT: usize = 2064;
const Q_AT: usize = 2236;
/// P: 86 codewords of 26 bytes; Q: 52 of 45.
const P_COUNT: usize = 86;
const P_N: usize = 26;
const Q_COUNT: usize = 52;
const Q_N: usize = 45;
/// Passes of P then Q, at most.
const PASSES: usize = 8;

/// GF(2^8) with x^8 + x^4 + x^3 + x^2 + 1 (0x11D): powers of α = 2, to
/// 510 so a product needs no reducing, and their logarithms.
const EXP: [u8; 512] = {
    let mut t = [0u8; 512];
    let mut x: u32 = 1;
    let mut i = 0;
    while i < 512 {
        t[i] = x as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11D;
        }
        i += 1;
    }
    t
};
const LOG: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 255 {
        t[EXP[i] as usize] = i as u8;
        i += 1;
    }
    t
};

fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        EXP[LOG[a as usize] as usize + LOG[b as usize] as usize]
    }
}

fn times_alpha(a: u8) -> u8 {
    mul(a, 2)
}

/// Where byte `k` of P codeword `m` is: column `m`, row `k`; rows 24 and
/// 25 are its parity.
fn p_at(m: usize, k: usize) -> usize {
    m + P_COUNT * k
}

/// Where byte `k` of Q codeword `m` is: a diagonal of the sector's 16-bit
/// words (byte `m & 1` of each), stepping 44 words a row and wrapping at
/// 1,118; bytes 43 and 44 are its parity.
fn q_at(m: usize, k: usize) -> usize {
    if k < Q_N - 2 {
        ((m >> 1) * P_COUNT + 88 * k) % Q_AT + (m & 1)
    } else {
        Q_AT + m + Q_COUNT * (k - (Q_N - 2))
    }
}

/// A codeword's syndromes: the sum of its bytes, and the sum of each
/// times α to the power of how far it is from the end. Both zero when it
/// fits (ECMA-130's check matrix, rows of 1 and of α^(n-1) … α, 1).
fn syndromes(b: &[u8], n: usize, at: impl Fn(usize) -> usize) -> (u8, u8) {
    let (mut s0, mut s1) = (0u8, 0u8);
    for k in 0..n {
        let c = b[at(k)];
        s0 ^= c;
        s1 = times_alpha(s1) ^ c;
    }
    (s0, s1)
}

/// A codeword's two parity bytes, from the rest of it: they make both of
/// its syndromes zero. With A the sum of the others and B their weighted
/// sum, p0 + p1 = A and α·p0 + p1 = B, so p0 = (A + B) / (α + 1).
fn encode(b: &mut [u8], n: usize, at: impl Fn(usize) -> usize) {
    let (mut a, mut h) = (0u8, 0u8);
    for k in 0..n - 2 {
        let c = b[at(k)];
        a ^= c;
        h = times_alpha(h) ^ c;
    }
    let weighted = times_alpha(times_alpha(h));
    // 1 / (α + 1): α + 1 is 3.
    let p0 = mul(a ^ weighted, EXP[255 - LOG[3] as usize]);
    b[at(n - 2)] = p0;
    b[at(n - 1)] = a ^ p0;
}

/// The sector from its header on, the header zeroed for Mode 2.
fn covered(sec: &[u8], mode2: bool) -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b.copy_from_slice(&sec[12..]);
    if mode2 {
        b[..4].fill(0);
    }
    b
}

/// Writes a sector's P and Q parity, as ECMA-130 makes them: P over its
/// header (zeros for Mode 2), data and EDC, then Q over those and P.
pub fn parity(sec: &mut [u8], mode2: bool) {
    assert_eq!(sec.len(), SECTOR);
    let mut b = covered(sec, mode2);
    for m in 0..P_COUNT {
        encode(&mut b, P_N, |k| p_at(m, k));
    }
    for m in 0..Q_COUNT {
        encode(&mut b, Q_N, |k| q_at(m, k));
    }
    sec[12 + P_AT..].copy_from_slice(&b[P_AT..]);
}

fn all_fit(b: &[u8]) -> bool {
    (0..P_COUNT).all(|m| syndromes(b, P_N, |k| p_at(m, k)) == (0, 0)) && (0..Q_COUNT).all(|m| syndromes(b, Q_N, |k| q_at(m, k)) == (0, 0))
}

/// Whether a sector's P and Q parity fit it. A Mode 2 sector's are worked
/// out with its header taken as zeros.
pub fn fits(sec: &[u8], mode2: bool) -> bool {
    sec.len() == SECTOR && all_fit(&covered(sec, mode2))
}

/// One pass over `count` codewords of `n` bytes: each that has one wrong
/// byte has it put right, unless that byte is one the parity can't vouch
/// for (`fixed`). Gives how many it put right.
fn pass(b: &mut [u8], count: usize, n: usize, at: impl Fn(usize, usize) -> usize, fixed: impl Fn(usize) -> bool) -> usize {
    let mut put = 0;
    for m in 0..count {
        let (s0, s1) = syndromes(b, n, |k| at(m, k));
        if s0 == 0 || s1 == 0 {
            // Fits, or more than one byte is wrong: nothing to do here.
            continue;
        }
        // One wrong byte, e, at k: s0 = e and s1 = e·α^(n-1-k).
        let far = (LOG[s1 as usize] as usize + 255 - LOG[s0 as usize] as usize) % 255;
        if far >= n {
            continue;
        }
        let i = at(m, n - 1 - far);
        if fixed(i) {
            continue;
        }
        b[i] ^= s0;
        put += 1;
    }
    put
}

/// Why a sector can't be put right by its parity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cannot {
    /// It passes its checks already, or has nothing to check it by.
    NotBad,
    /// Nothing was read: the drive gave nothing, and it was filled in.
    Unreadable,
    /// No sync, or a mode no data sector has: not a data sector as read.
    NotData,
    /// Its header names another sector, or a Mode 2 header is wrong,
    /// which the parity doesn't cover: likely a neighbour read in its
    /// place.
    WrongAddress,
    /// A form 2 sector: it has no parity.
    Form2,
    /// Too many bytes wrong for its parity to put right.
    TooDamaged,
}

impl Cannot {
    /// Why, in a few words, for the log.
    pub fn words(self) -> &'static str {
        match self {
            Cannot::NotBad => "nothing wrong with it",
            Cannot::Unreadable => "nothing was read, so there's nothing to correct",
            Cannot::NotData => "it doesn't read as a data sector at all",
            Cannot::WrongAddress => "its header is wrong, which the error correction doesn't cover",
            Cannot::Form2 => "a form 2 sector, which has no error correction",
            Cannot::TooDamaged => "too much of it is wrong to correct",
        }
    }
}

/// A sector put right: as it should be, and how many of its bytes changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Repaired {
    pub sector: Vec<u8>,
    pub changed: usize,
}

/// Puts `sec`, read as sector `lba`, right by its P and Q parity, if it
/// can: on a copy, given back only when every codeword then fits, the EDC
/// fits, and the sector passes every check of its own (its address too).
/// `sec` is never changed.
pub fn repair(sec: &[u8], lba: i64) -> Result<Repaired, Cannot> {
    if sec.len() != SECTOR || sec.iter().all(|&b| b == 0x55) || sec.iter().all(|&b| b == 0) {
        return Err(Cannot::Unreadable);
    }
    if combine::trust(sec, lba, true) != Trust::Bad {
        return Err(Cannot::NotBad);
    }
    if !combine::has_sync(sec) {
        return Err(Cannot::NotData);
    }
    // A Mode 1 header, its mode byte too, is covered: any mode but 2 is
    // tried as Mode 1, and must come out as Mode 1.
    let mode2 = sec[15] == 2;
    let unknown = !mode2 && sec[15] != 1;
    if mode2 {
        if sec[12..15] != combine::msf(lba) {
            return Err(Cannot::WrongAddress);
        }
        // Both copies of the subheader say form 2: no parity to use.
        if sec[18] & sec[22] & 0x20 != 0 {
            return Err(Cannot::Form2);
        }
    }
    let mut b = covered(sec, mode2);
    // A Mode 2 header is zeros here, as the parity was made: never "put right".
    let fixed = |i: usize| mode2 && i < 4;
    for _ in 0..PASSES {
        let p = pass(&mut b, P_COUNT, P_N, p_at, fixed);
        let q = pass(&mut b, Q_COUNT, Q_N, q_at, fixed);
        if p + q == 0 {
            break;
        }
    }
    if !all_fit(&b) {
        return Err(if unknown { Cannot::NotData } else { Cannot::TooDamaged });
    }
    let mut out = sec.to_vec();
    let from = if mode2 { 4 } else { 0 };
    out[12 + from..].copy_from_slice(&b[from..]);
    let form1 = !mode2 || out[18] & 0x20 == 0;
    if !form1 || combine::trust(&out, lba, true) != Trust::Checked {
        return Err(match (mode2, out[15]) {
            (true, _) => Cannot::TooDamaged,
            (false, 1) => Cannot::WrongAddress,
            (false, _) => Cannot::NotData,
        });
    }
    let changed = out.iter().zip(sec).filter(|(a, b)| a != b).count();
    Ok(Repaired { sector: out, changed })
}

// ---- an image's bad sectors ---------------------------------------------------------------

/// A bad sector of an image, and what its error correction makes of it.
#[derive(Clone, Debug)]
pub struct Found {
    pub place: Place,
    /// As it is in the image.
    pub before: Vec<u8>,
    pub repair: Result<Repaired, Cannot>,
    /// How many reads of it gave back these same bytes, the image's own
    /// copy among them.
    pub same_reads: usize,
    /// Every read of it gave back these same bytes, or it's been read only
    /// once: it may be pressed so on purpose (copy protection), so it's
    /// not to be repaired unless Redump's checksums then say so.
    pub maybe_pressed: bool,
}

impl Found {
    pub fn lba(&self) -> i64 {
        self.place.lba
    }
}

/// What's known of an image's sectors' reads besides its own copy.
#[derive(Clone, Debug, Default)]
pub struct Reads {
    /// Every rip of the disc, by its cue sheet, first to last (`rip-1`
    /// on); none when it's been ripped only once.
    pub rips: Vec<PathBuf>,
    /// Sectors a re-read gave back the image's copy of.
    pub read_alike: Vec<i64>,
    /// Sectors a re-read gave back other bytes of.
    pub read_differently: Vec<i64>,
}

/// The data sectors of the image whose cue sheet is `cue` that fail their
/// own check, and what their error correction can do with each, beside
/// what every read of it so far gave back. Nothing is changed.
pub fn look(cue: &Path, reads: &Reads, progress: impl FnMut(f32), stop: &dyn Fn() -> bool) -> io::Result<Vec<Found>> {
    let dir = cue.parent().unwrap_or(Path::new("."));
    let tracks = combine::layout(cue)?;
    let (bad, _) = fix::find_bad(cue, progress, stop)?;
    let rips: Vec<(PathBuf, Vec<combine::TrackFile>)> = reads.rips.iter().filter_map(|c| Some((c.parent()?.to_path_buf(), combine::layout(c).ok()?))).collect();
    let mut out = vec![];
    for lba in bad {
        let Some(place) = fix::place(dir, &tracks, lba) else { continue };
        let before = fix::read_at(&place)?;
        let copies: Vec<Vec<u8>> = rips.iter().filter_map(|(d, t)| fix::read_at(&fix::place(d, t, lba)?).ok()).collect();
        // The image's own copy is one read, or one of the rips'.
        let mut same = if copies.is_empty() { 1 } else { copies.iter().filter(|c| **c == before).count() };
        let mut differed = copies.iter().any(|c| *c != before);
        // Re-reads were of the first rip's copy, as fixing left it.
        if copies.first().is_none_or(|c| *c == before) && reads.read_alike.contains(&lba) {
            same = same.max(1) + 1;
        }
        differed |= reads.read_differently.contains(&lba);
        // Two reads alike outweigh any that differ: a pressed sector can be
        // misread too.
        let repair = repair(&before, lba);
        out.push(Found { place, before, repair, same_reads: same, maybe_pressed: same >= 2 || !differed });
    }
    Ok(out)
}

/// What looking came to, in counts.
pub fn summary(found: &[Found]) -> Repairing {
    let n = |f: &dyn Fn(&Found) -> bool| found.iter().filter(|x| f(x)).count() as u64;
    Repairing {
        bad: found.len() as u64,
        repaired: n(&|f| f.repair.is_ok() && !f.maybe_pressed),
        held_back: n(&|f| f.repair.is_ok() && f.maybe_pressed),
        unreadable: n(&|f| f.repair == Err(Cannot::Unreadable)),
        form2: n(&|f| f.repair == Err(Cannot::Form2)),
        beyond: n(&|f| matches!(f.repair, Err(Cannot::TooDamaged | Cannot::WrongAddress | Cannot::NotData | Cannot::NotBad))),
        verified: false,
        secs: None,
    }
}

/// The log's lines for what looking found and what was done: each sector
/// repaired, with how many of its bytes changed, and each left, and why.
/// `pressed_too` when those that may be pressed so were repaired as well.
pub fn log_lines(found: &[Found], pressed_too: bool) -> Vec<String> {
    let mut out = vec![];
    if !pressed_too {
        out.push(format!("error correction: {} still fail their own check", mismatch::sectors(found.len() as u64)));
    }
    for f in found {
        let lba = f.lba();
        out.push(match (&f.repair, f.maybe_pressed) {
            (Ok(r), false) => format!("  sector {lba}: repaired by its error correction, {} changed (its reads differ, so it was read wrong)", bytes(r.changed)),
            (Ok(r), true) if pressed_too => format!("  sector {lba}: repaired by its error correction, {} changed, to see whether the image then matches Redump", bytes(r.changed)),
            (Ok(r), true) => {
                let why = if f.same_reads >= 2 { format!("{} reads gave it back the same", f.same_reads) } else { "it's been read only once".to_owned() };
                format!("  sector {lba}: its error correction could repair it ({}), but {why}, so it may be pressed so: left as read unless repairing it makes the image Redump's", bytes(r.changed))
            }
            (Err(e), _) => format!("  sector {lba}: can't be repaired: {}", e.words()),
        });
    }
    out
}

fn bytes(n: usize) -> String {
    format!("{n} byte{}", if n == 1 { "" } else { "s" })
}

/// Writes each found sector's repair over it in the image, keeping the
/// files it changes as they were in `keep` (see [`fix::patch`]). Gives the
/// sectors written.
pub fn apply(found: &[&Found], keep: &Path) -> io::Result<Vec<i64>> {
    let copies: Vec<(Place, Vec<u8>)> = found.iter().filter_map(|f| Some((f.place.clone(), f.repair.as_ref().ok()?.sector.clone()))).collect();
    if !copies.is_empty() {
        fix::patch(&copies, keep)?;
    }
    Ok(copies.iter().map(|(p, _)| p.lba).collect())
}

/// Keeps found sectors as read in `aside`, to be put back with [`put_back`].
pub fn keep_aside(found: &[&Found], aside: &Path) -> io::Result<()> {
    let mut b = Vec::with_capacity(found.len() * (8 + SECTOR));
    for f in found {
        b.extend_from_slice(&f.lba().to_le_bytes());
        b.extend_from_slice(&f.before);
    }
    let part = aside.with_extension("part");
    std::fs::write(&part, b)?;
    std::fs::rename(part, aside)
}

/// Puts the sectors kept in `aside` back in the image whose cue sheet is
/// `cue`, as they were read, and forgets them. Gives the sectors put back.
pub fn put_back(cue: &Path, aside: &Path, keep: &Path) -> io::Result<Vec<i64>> {
    let b = match std::fs::read(aside) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let dir = cue.parent().unwrap_or(Path::new("."));
    let tracks = combine::layout(cue)?;
    let mut copies = vec![];
    for c in b.chunks(8 + SECTOR) {
        if c.len() != 8 + SECTOR {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} is cut short", aside.display())));
        }
        let lba = i64::from_le_bytes(c[..8].try_into().unwrap_or_default());
        let p = fix::place(dir, &tracks, lba).ok_or_else(|| io::Error::other(format!("sector {lba} isn't in a data track")))?;
        copies.push((p, c[8..].to_vec()));
    }
    fix::patch(&copies, keep)?;
    std::fs::remove_file(aside)?;
    Ok(copies.iter().map(|(p, _)| p.lba).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// ECM's way of making P and Q, a separate implementation, as a check
    /// on [`parity`]'s.
    fn reference(b: &[u8], major_count: usize, minor_count: usize, major_mult: usize, minor_inc: usize, dest: &mut [u8]) {
        let f = |i: u8| ((i as u32) << 1 ^ if i & 0x80 != 0 { 0x11D } else { 0 }) as u8;
        let mut back = [0u8; 256];
        for i in 0..=255u8 {
            back[(i ^ f(i)) as usize] = i;
        }
        let size = major_count * minor_count;
        for major in 0..major_count {
            let mut index = (major >> 1) * major_mult + (major & 1);
            let (mut a, mut x) = (0u8, 0u8);
            for _ in 0..minor_count {
                let c = b[index];
                index += minor_inc;
                if index >= size {
                    index -= size;
                }
                a ^= c;
                x ^= c;
                a = f(a);
            }
            a = back[(f(a) ^ x) as usize];
            dest[major] = a;
            dest[major + major_count] = a ^ x;
        }
    }

    /// A simple seeded generator, so a failing case can be found again.
    pub struct Rng(pub u64);

    impl Rng {
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        pub fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// A Mode 2 form 1 sector at `lba`, as a PlayStation disc holds it,
    /// its data from `seed`; or a Mode 1 sector.
    pub fn form1(lba: i64, seed: u64, mode1: bool) -> Vec<u8> {
        let mut r = Rng(seed | 1);
        let mut s = vec![0u8; SECTOR];
        s[1..11].fill(0xFF);
        s[12..15].copy_from_slice(&combine::msf(lba));
        if mode1 {
            s[15] = 1;
            for b in &mut s[16..2064] {
                *b = r.next() as u8;
            }
            let e = combine::edc(&s[..2064]);
            s[2064..2068].copy_from_slice(&e.to_le_bytes());
        } else {
            s[15] = 2;
            s[16..24].copy_from_slice(&[0, 0, 8, 0, 0, 0, 8, 0]);
            for b in &mut s[24..2072] {
                *b = r.next() as u8;
            }
            let e = combine::edc(&s[16..2072]);
            s[2072..2076].copy_from_slice(&e.to_le_bytes());
        }
        parity(&mut s, !mode1);
        s
    }

    /// `n` bytes of `s` from `from` on made wrong, each at a different place.
    fn spoil(s: &[u8], n: usize, from: usize, r: &mut Rng) -> (Vec<u8>, Vec<usize>) {
        let mut out = s.to_vec();
        let mut at = vec![];
        while at.len() < n {
            let i = from + r.below(SECTOR - from);
            if at.contains(&i) {
                continue;
            }
            out[i] ^= 1 + r.below(255) as u8;
            at.push(i);
        }
        (out, at)
    }

    #[test]
    fn the_parity_is_ecma_130_s_and_ecm_s_alike() {
        for (lba, mode1) in [(0, false), (1234, false), (200_000, true), (16, true)] {
            let s = form1(lba, lba as u64 + 9, mode1);
            assert_eq!(combine::trust(&s, lba, true), Trust::Checked);
            let b = covered(&s, !mode1);
            let (mut p, mut q) = ([0u8; 172], [0u8; 104]);
            reference(&b, 86, 24, 2, 86, &mut p);
            reference(&b, 52, 43, 86, 88, &mut q);
            assert_eq!(&s[12 + P_AT..12 + Q_AT], &p[..], "P");
            assert_eq!(&s[12 + Q_AT..], &q[..], "Q");
            assert!(fits(&s, !mode1));
        }
    }

    #[test]
    fn every_byte_the_parity_covers_is_put_right_alone() {
        for mode1 in [false, true] {
            let lba = 4321;
            let good = form1(lba, 77, mode1);
            // Mode 2's header isn't covered; Mode 1's is.
            let from = if mode1 { 12 } else { 16 };
            for i in from..SECTOR {
                let mut s = good.clone();
                s[i] ^= 0xA5;
                let r = repair(&s, lba).unwrap_or_else(|e| panic!("byte {i}, mode 1 {mode1}: {e:?}"));
                assert_eq!((r.sector == good, r.changed), (true, 1), "byte {i}");
            }
        }
    }

    #[test]
    fn two_wrong_bytes_are_put_right_unless_they_share_a_q_codeword_s_own_parity() {
        let mut r = Rng(0x5eed);
        let lba = 7;
        let good = form1(lba, 3, false);
        let (mut ok, mut not) = (0, 0);
        for _ in 0..3000 {
            let (s, at) = spoil(&good, 2, 16, &mut r);
            // Q's parity is in no P codeword: two wrong in one Q codeword's are beyond it.
            let q_only = |i: usize| (i >= 12 + Q_AT).then(|| (i - 12 - Q_AT) % Q_COUNT);
            let beyond = matches!((q_only(at[0]), q_only(at[1])), (Some(a), Some(b)) if a == b);
            match repair(&s, lba) {
                Ok(x) => {
                    assert_eq!(x.sector, good, "{at:?}");
                    assert_eq!(x.changed, 2);
                    ok += 1;
                }
                Err(e) => {
                    assert!(beyond, "{at:?}: {e:?}");
                    assert_eq!(e, Cannot::TooDamaged);
                    not += 1;
                }
            }
        }
        assert!(ok > 2900, "{ok} put right, {not} not");
    }

    #[test]
    fn a_few_or_many_wrong_bytes_are_put_right_exactly_or_not_at_all() {
        let mut r = Rng(0xC0FFEE);
        for mode1 in [false, true] {
            let lba = 99_999;
            let good = form1(lba, 11, mode1);
            let from = if mode1 { 12 } else { 16 };
            for n in [3, 5, 8, 16, 40, 120, 600] {
                let mut fixed = 0;
                for _ in 0..200 {
                    let (s, at) = spoil(&good, n, from, &mut r);
                    match repair(&s, lba) {
                        // Never anything but the sector as it was.
                        Ok(x) => {
                            assert_eq!(x.sector, good, "{n} wrong at {at:?}");
                            assert_eq!(x.changed, n);
                            fixed += 1;
                        }
                        Err(e) => assert!(matches!(e, Cannot::TooDamaged | Cannot::WrongAddress | Cannot::NotData | Cannot::Form2), "{e:?}"),
                    }
                }
                match n {
                    3 | 5 => assert!(fixed > 190, "{n}: {fixed} of 200"),
                    120 | 600 => assert_eq!(fixed, 0, "{n} wrong bytes are past any correction"),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn what_the_parity_cant_cover_is_left_alone() {
        let lba = 500;
        let good = form1(lba, 5, false);
        assert_eq!(repair(&good, lba), Err(Cannot::NotBad));
        assert_eq!(repair(&[0x55; SECTOR], lba), Err(Cannot::Unreadable), "filled in where nothing was read");
        // A neighbour read in its place passes everything but its address.
        assert_eq!(repair(&form1(lba - 1, 5, false), lba), Err(Cannot::WrongAddress));
        let mut s = good.clone();
        s[13] ^= 1;
        assert_eq!(repair(&s, lba), Err(Cannot::WrongAddress), "a Mode 2 header isn't covered");
        let mut s = good.clone();
        s[3] = 0;
        s[900] ^= 1;
        assert_eq!(repair(&s, lba), Err(Cannot::NotData));
        // Form 2: no parity.
        let mut s = combine::tests::sector(lba, 22, 1, true);
        s[700] ^= 1;
        assert_eq!(repair(&s, lba), Err(Cannot::Form2));
        // A Mode 1 header is covered, so its address can be put right.
        let good1 = form1(lba, 5, true);
        let mut s = good1.clone();
        s[14] ^= 0x40;
        assert_eq!(repair(&s, lba).map(|r| r.sector), Ok(good1));
    }

    /// Real misreads: each sector where a real rip differs from the same
    /// disc's Redump-verified image is handed to [`repair`]; what it puts
    /// right must be the verified sector, byte for byte.
    /// `TRIP_ECC_BIN=<verified .bin> TRIP_ECC_RIPS=<rip .bin>:<rip .bin> cargo test --release -p trip-steps ecc::tests::real_misreads -- --ignored --exact --nocapture`
    #[test]
    #[ignore]
    fn real_misreads() {
        use std::io::Read;
        let good = std::env::var("TRIP_ECC_BIN").expect("TRIP_ECC_BIN");
        for rip in std::env::var("TRIP_ECC_RIPS").expect("TRIP_ECC_RIPS").split(':') {
            let mut g = std::io::BufReader::new(std::fs::File::open(&good).unwrap());
            let mut r = std::io::BufReader::new(std::fs::File::open(rip).unwrap());
            let (mut a, mut b) = (vec![0u8; SECTOR], vec![0u8; SECTOR]);
            let mut lba = 0i64;
            let mut tally: std::collections::BTreeMap<String, u64> = Default::default();
            while g.read_exact(&mut a).is_ok() && r.read_exact(&mut b).is_ok() {
                if a != b {
                    let what = match repair(&b, lba) {
                        Ok(x) => {
                            assert_eq!(x.sector, a, "sector {lba} of {rip}: repaired into something else");
                            "repaired, the verified sector byte for byte".to_owned()
                        }
                        Err(e) => format!("{e:?}"),
                    };
                    *tally.entry(what).or_default() += 1;
                }
                lba += 1;
            }
            println!("{rip}: {tally:?}");
        }
    }

    /// Every form 1 sector of a real, Redump-verified PlayStation image:
    /// its parity made again is its own, byte for byte, and wrong bytes
    /// put in it come out right or not at all.
    /// `TRIP_ECC_BIN=…bin cargo test -p trip-steps ecc::tests::real_sectors -- --ignored --exact --nocapture`
    #[test]
    #[ignore]
    fn real_sectors() {
        use std::io::Read;
        let path = std::env::var("TRIP_ECC_BIN").expect("TRIP_ECC_BIN");
        let mut f = std::io::BufReader::new(std::fs::File::open(path).unwrap());
        let mut s = vec![0u8; SECTOR];
        let (mut lba, mut form1s, mut form2s, mut tried, mut fixed) = (0i64, 0u64, 0u64, 0u64, 0u64);
        let mut r = Rng(0xD15C);
        while f.read_exact(&mut s).is_ok() {
            if s[15] == 2 && s[18] & 0x20 == 0 && combine::trust(&s, lba, true) == Trust::Checked {
                form1s += 1;
                let mut again = s.clone();
                again[12 + P_AT..].fill(0);
                parity(&mut again, true);
                assert_eq!(again, s, "sector {lba}: parity made again differs");
                if lba % 50 == 0 {
                    let n = [1, 2, 3, 6, 30, 400][r.below(6)];
                    let (bad, _) = spoil(&s, n, 16, &mut r);
                    tried += 1;
                    match repair(&bad, lba) {
                        Ok(x) => {
                            assert_eq!(x.sector, s, "sector {lba}, {n} wrong");
                            fixed += 1;
                        }
                        Err(e) => assert!(n > 2, "sector {lba}: {n} wrong: {e:?}"),
                    }
                }
            } else if s[15] == 2 {
                form2s += 1;
            }
            lba += 1;
        }
        println!("{lba} sectors: {form1s} form 1 with their parity made again byte for byte, {form2s} form 2 or other; {fixed} of {tried} spoiled copies put right, the rest given up");
        assert!(form1s > 1000);
    }
}
