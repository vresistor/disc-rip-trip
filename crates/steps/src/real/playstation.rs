//! What a PlayStation disc says about itself, read straight off its
//! filesystem in milliseconds.
//!
//! Sony required every PS1 and PS2 disc to carry `SYSTEM.CNF` in the ISO
//! 9660 root, naming the boot executable:
//!
//! ```text
//! BOOT2 = cdrom0:\SLUS_202.16;1
//! VER = 1.00
//! VMODE = NTSC
//! ```
//!
//! `SLUS_202.16` is the serial `SLUS-20216`, and which key carries it
//! (`BOOT` or `BOOT2`) says which console: `SLUS` is used by both, so the
//! serial can't. `redumper info` doesn't read this (checked by the old app).
//!
//! Lifted from the old app's `playstation.rs` and tidied: it reads sectors
//! through a function, so the same code reads a drive and a test's bytes,
//! and the ISO 9660 part is shared with the other systems' checks.

// The bytes come off a disc anyone could have made: no indexing that can panic.
#![warn(clippy::indexing_slicing, clippy::string_slice)]

use std::io;

/// Reads `count` 2048-byte sectors from an LBA.
pub type Sectors<'a> = &'a dyn Fn(u64, usize) -> io::Result<Vec<u8>>;

pub const SECTOR: usize = 2048;

/// An ISO 9660 volume, from its primary volume descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    /// `PLAYSTATION` on every disc for either console.
    pub system_id: String,
    pub label: String,
    /// How many 2048-byte blocks the filesystem says it has: the one number
    /// known before dumping that tells the discs of a set apart.
    pub sectors: u64,
    root: (u64, u64),
}

/// The volume whose descriptor is 16 sectors into a track starting at
/// `start`, or `None` when there's no ISO 9660 there (an audio track, or a
/// format a PC drive can't read).
pub fn volume(read: Sectors, start: u64) -> Option<Volume> {
    let pvd = read(start.checked_add(16)?, 1).ok()?;
    let pvd: &[u8; SECTOR] = pvd.first_chunk()?;
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return None;
    }
    // The root directory's record sits inside the descriptor, at 156.
    let root = 156;
    Some(Volume { system_id: ascii(&pvd[8..40]), label: ascii(&pvd[40..72]), sectors: le_u32(pvd, 80)? as u64, root: (le_u32(pvd, root + 2)? as u64, le_u32(pvd, root + 10)? as u64) })
}

/// Where a file or folder in the root starts and how long it is. Names
/// carry a `;1` version, matched loosely. A directory that fails to read is
/// an error, not an empty directory.
pub fn find(read: Sectors, v: &Volume, wanted: &str) -> io::Result<Option<(u64, u64)>> {
    let (extent, length) = v.root;
    let n = length.div_ceil(SECTOR as u64).clamp(1, 64) as usize;
    let dir = read(extent, n)?;
    for sector in dir.chunks(SECTOR) {
        let mut at = 0;
        // Records don't straddle sectors; a zero length is the padding after the last.
        while let Some(&len) = sector.get(at) {
            let len = len as usize;
            let Some(r) = sector.get(at..at + len).filter(|_| len >= 34) else { break };
            let name = r.get(33..33 + r.get(32).map_or(0, |&n| n as usize));
            if let (Some(name), Some(extent), Some(size)) = (name, le_u32(r, 2), le_u32(r, 10)) {
                if String::from_utf8_lossy(name).split(';').next().unwrap_or("").trim().eq_ignore_ascii_case(wanted) {
                    return Ok(Some((extent as u64, size as u64)));
                }
            }
            at += len;
        }
    }
    Ok(None)
}

/// A file in the root, at most `cap` bytes of it, if it's there.
pub fn read_file(read: Sectors, v: &Volume, name: &str, cap: usize) -> io::Result<Option<Vec<u8>>> {
    let Some((extent, length)) = find(read, v, name)? else { return Ok(None) };
    let want = (length as usize).min(cap);
    let mut bytes = read(extent, want.div_ceil(SECTOR).max(1))?;
    bytes.truncate(want);
    Ok(Some(bytes))
}

/// What `SYSTEM.CNF` says.
#[derive(Clone, Debug, PartialEq)]
pub struct Boot {
    /// `BOOT2`: a PlayStation 2 disc.
    pub ps2: bool,
    /// `SLUS-20216`.
    pub serial: String,
    /// The executable as the disc names it: `SLUS_202.16`.
    pub file: String,
}

/// Reads `SYSTEM.CNF`'s text. `BOOT2` wins over `BOOT`: a PS2 disc's file
/// sometimes carries both. `None` when neither names a serial.
pub fn parse_system_cnf(text: &str) -> Option<Boot> {
    let (mut ps1, mut ps2) = (None, None);
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let found = boot_file(value).and_then(|f| normalise_serial(&f).map(|s| (s, f)));
        match key.trim().to_ascii_uppercase().as_str() {
            "BOOT2" => ps2 = ps2.or(found),
            "BOOT" => ps1 = ps1.or(found),
            _ => {}
        }
    }
    match (ps2, ps1) {
        (Some((serial, file)), _) => Some(Boot { ps2: true, serial, file }),
        (None, Some((serial, file))) => Some(Boot { ps2: false, serial, file }),
        (None, None) => None,
    }
}

/// `cdrom0:\SLUS_202.16;1` → `SLUS_202.16`.
fn boot_file(value: &str) -> Option<String> {
    let tail = value.trim().rsplit(['\\', '/', ':']).next()?;
    let f = tail.split(';').next()?.trim();
    (!f.is_empty()).then(|| f.to_owned())
}

/// A serial in Redump's spelling: the letter code, a hyphen, the rest.
/// `SLUS_202.16`, `SLUS-20216`, `slus 20216` → `SLUS-20216`. `None` for
/// what isn't one (a letter code and digits both), which keeps `PSX.EXE`
/// and `VER = 1.00` from becoming serials.
pub fn normalise_serial(raw: &str) -> Option<String> {
    let s: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect();
    let letters = s.chars().take_while(char::is_ascii_alphabetic).count();
    let (code, number) = s.split_at_checked(letters)?;
    if code.is_empty() || !number.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{code}-{number}"))
}

/// The region a Sony serial's letter code implies: the third and fourth
/// letters. A hint; Redump's entry says what the game is catalogued as.
pub fn region_of(serial: &str) -> Option<&'static str> {
    let code: String = serial.chars().take_while(char::is_ascii_alphabetic).collect();
    Some(match code.get(2..4)? {
        "US" => "USA",
        "ES" | "ED" => "Europe",
        "PS" | "PM" | "KP" => "Japan",
        "KA" => "Korea",
        "AJ" | "AT" => "Asia",
        _ => return None,
    })
}

/// Space-padded ASCII, trimmed.
pub fn ascii(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_owned()
}

/// The little-endian `u32` at `at`, if `b` is long enough.
fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(*b.get(at..)?.first_chunk()?))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::string_slice)]
pub(crate) mod tests {
    use super::*;

    fn both(v: u32) -> [u8; 8] {
        let mut o = [0u8; 8];
        o[..4].copy_from_slice(&v.to_le_bytes());
        o[4..].copy_from_slice(&v.to_be_bytes());
        o
    }

    /// The smallest ISO 9660 image that answers what's asked here: a primary
    /// volume descriptor at 16, a root directory at 20 holding `files`, and
    /// their contents from 21 on.
    pub fn iso(system_id: &str, label: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut img = vec![0u8; SECTOR * (21 + files.len().max(1))];
        let pvd = SECTOR * 16;
        img[pvd] = 1;
        img[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
        img[pvd + 6] = 1;
        let pad = |s: &str, n| {
            let mut v = s.as_bytes().to_vec();
            v.resize(n, b' ');
            v
        };
        img[pvd + 8..pvd + 40].copy_from_slice(&pad(system_id, 32));
        img[pvd + 40..pvd + 72].copy_from_slice(&pad(label, 32));
        img[pvd + 80..pvd + 88].copy_from_slice(&both(240_000));
        let root = pvd + 156;
        img[root] = 34;
        img[root + 2..root + 10].copy_from_slice(&both(20));
        img[root + 10..root + 18].copy_from_slice(&both(SECTOR as u32));
        img[root + 32] = 1;
        let mut at = SECTOR * 20;
        for (i, (name, body)) in files.iter().enumerate() {
            let n = name.as_bytes();
            let len = 33 + n.len() + (n.len() % 2 == 0) as usize;
            img[at] = len as u8;
            img[at + 2..at + 10].copy_from_slice(&both(21 + i as u32));
            img[at + 10..at + 18].copy_from_slice(&both(body.len() as u32));
            img[at + 32] = n.len() as u8;
            img[at + 33..at + 33 + n.len()].copy_from_slice(n);
            let f = SECTOR * (21 + i);
            img[f..f + body.len()].copy_from_slice(body);
            at += len;
        }
        img
    }

    /// Reads a test image as a drive would: a short read is an error.
    pub fn reader(img: &[u8]) -> impl Fn(u64, usize) -> io::Result<Vec<u8>> + '_ {
        move |lba, n| {
            let from = lba as usize * SECTOR;
            img.get(from..from + n * SECTOR).map(<[u8]>::to_vec).ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "past the end"))
        }
    }

    fn boot(img: &[u8]) -> Option<Boot> {
        let r = reader(img);
        let v = volume(&r, 0)?;
        let text = read_file(&r, &v, "SYSTEM.CNF", 64 * 1024).unwrap()?;
        parse_system_cnf(&String::from_utf8_lossy(&text))
    }

    #[test]
    fn a_ps1_disc_gives_its_serial_and_boot_file() {
        let img = iso("PLAYSTATION", "SLES_008.60", &[("SYSTEM.CNF;1", b"BOOT = cdrom:\\SLES_008.60;1\r\nTCB = 4\r\nEVENT = 10\r\nSTACK = 801FFFF0\r\n")]);
        assert_eq!(boot(&img), Some(Boot { ps2: false, serial: "SLES-00860".into(), file: "SLES_008.60".into() }));
        let v = volume(&reader(&img), 0).unwrap();
        assert_eq!((v.system_id.as_str(), v.label.as_str(), v.sectors), ("PLAYSTATION", "SLES_008.60", 240_000));
    }

    #[test]
    fn a_ps2_disc_is_told_apart_by_its_boot_key_not_its_serial() {
        let img = iso("PLAYSTATION", "SLUS_202.16", &[("SYSTEM.CNF;1", b"BOOT2 = cdrom0:\\SLUS_202.16;1\nVER = 1.00\nVMODE = NTSC\n")]);
        assert_eq!(boot(&img).unwrap(), Boot { ps2: true, serial: "SLUS-20216".into(), file: "SLUS_202.16".into() });
        // Both keys: still a PS2 disc.
        let both = parse_system_cnf("BOOT = cdrom0:\\PSX.EXE;1\nBOOT2 = cdrom0:\\SLES_502.11;1\n").unwrap();
        assert!(both.ps2);
        assert_eq!(both.serial, "SLES-50211");
    }

    #[test]
    fn a_boot_line_naming_nothing_is_no_serial() {
        assert_eq!(parse_system_cnf("BOOT = cdrom:\\\nVER = 1.00\n"), None);
        assert_eq!(parse_system_cnf("BOOT = cdrom:\\PSX.EXE;1\n"), None, "a homebrew boot file is no serial");
    }

    #[test]
    fn a_disc_without_system_cnf_or_iso_9660() {
        let img = iso("", "MYBACKUP", &[]);
        assert_eq!(boot(&img), None);
        assert_eq!(volume(&reader(&vec![0u8; SECTOR * 24]), 0), None, "no filesystem");
    }

    #[test]
    fn a_directory_that_fails_to_read_is_an_error() {
        let mut img = iso("PLAYSTATION", "X", &[("SYSTEM.CNF;1", b"BOOT = cdrom:\\SLUS_007.79;1\n")]);
        let v = volume(&reader(&img), 0).unwrap();
        img.truncate(SECTOR * 20 + 10);
        assert!(find(&reader(&img), &v, "SYSTEM.CNF").is_err());
    }

    /// A short read, and directory records that lie about their lengths
    /// in every way: nothing panics, and nothing is found.
    #[test]
    fn a_hostile_disc_is_no_panic() {
        let short = |_: u64, _: usize| Ok(vec![1u8, b'C', b'D']);
        assert!(volume(&short, 0).is_none());
        assert!(volume(&|_, _| Ok(vec![]), u64::MAX).is_none());
        let mut img = iso("PLAYSTATION", "X", &[("SYSTEM.CNF;1", b"BOOT = cdrom:\\SLUS_000.01;1")]);
        let dir = SECTOR * 20;
        for (len, name_len) in [(255u8, 255u8), (34, 255), (33, 0), (1, 0), (0, 0)] {
            img[dir] = len;
            img[dir + 32] = name_len;
            // The last record runs to the sector's end.
            img[dir + SECTOR - 1] = 200;
            let r = reader(&img);
            let v = volume(&r, 0).unwrap();
            assert_eq!(find(&r, &v, "SYSTEM.CNF").unwrap(), None, "{len} {name_len}");
        }
    }

    #[test]
    fn serials_normalise_however_they_are_written() {
        for raw in ["SLUS_202.16", "SLUS-20216", "SLUS 20216", "slus20216"] {
            assert_eq!(normalise_serial(raw).as_deref(), Some("SLUS-20216"), "{raw}");
        }
        assert_eq!(normalise_serial("SLES-03428-GER").as_deref(), Some("SLES-03428GER"));
        for not in ["", "1.00", "PSXEXE", "PSX.EXE"] {
            assert_eq!(normalise_serial(not), None, "{not}");
        }
    }

    #[test]
    fn regions_come_from_the_letter_code() {
        assert_eq!(region_of("SLUS-00779"), Some("USA"));
        assert_eq!(region_of("SCES-01564"), Some("Europe"));
        assert_eq!(region_of("SLPM-86770"), Some("Japan"));
        assert_eq!(region_of("ABCD-00001"), None);
        assert_eq!(region_of("T-4507G"), None);
    }
}
