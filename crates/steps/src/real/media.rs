//! What a data disc is, read straight off the device in milliseconds and
//! with no tool: its volume label, its filesystems, its size, whether it's a
//! CD, DVD or Blu-ray, and whether any of its tracks are audio.
//!
//! Lifted from the old app's `disc.rs` and widened: it read only the ISO
//! 9660 label, so a Blu-ray (UDF only) or a UDF-only DVD had no name at all.

// The bytes come off a disc, or a cover from the network: no indexing that
// can panic.
#![warn(clippy::indexing_slicing, clippy::string_slice)]

use std::path::Path;

/// One logical sector of a data disc.
pub const SECTOR: usize = 2048;

/// What the Data disc step found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// The volume label, or empty when the disc has none.
    pub label: String,
    /// "ISO 9660", "UDF", "ISO 9660 + UDF", or "none".
    pub filesystem: String,
    /// "CD", "DVD", "Blu-ray" or "HD DVD".
    pub media: String,
    pub bytes: u64,
    /// Tracks in the table of contents, and how many of them are audio.
    pub tracks: u32,
    pub audio_tracks: u32,
}

// ---- reading the filesystem, from bytes ------------------------------------------------------

/// Which filesystems the volume descriptors from sector 16 on announce.
/// ISO 9660 says "CD001"; UDF says "BEA01", then "NSR02" or "NSR03".
pub fn filesystems(from_16: &[u8]) -> (bool, bool) {
    let (mut iso, mut udf) = (false, false);
    for d in from_16.as_chunks::<SECTOR>().0 {
        match &d[1..6] {
            b"CD001" => iso = true,
            b"NSR02" | b"NSR03" => udf = true,
            b"BEA01" | b"TEA01" | b"BOOT2" | b"CDW02" => {}
            // The recognition sequence ends at the first sector that isn't one.
            _ => break,
        }
    }
    (iso, udf)
}

/// The volume identifier of an ISO 9660 primary volume descriptor (type 1).
pub fn iso_label(pvd: &[u8]) -> Option<String> {
    let pvd: &[u8; 72] = pvd.first_chunk()?;
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return None;
    }
    Some(String::from_utf8_lossy(&pvd[40..72]).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_owned())
}

/// Where the UDF anchor (sector 256) says the main volume descriptor
/// sequence is: (first sector, length in bytes).
pub fn udf_anchor(sector: &[u8]) -> Option<(u32, u32)> {
    let sector: &[u8; 24] = sector.first_chunk()?;
    if u16::from_le_bytes([sector[0], sector[1]]) != 2 {
        return None;
    }
    let len = u32::from_le_bytes(sector[16..20].try_into().ok()?);
    let loc = u32::from_le_bytes(sector[20..24].try_into().ok()?);
    Some((loc, len))
}

/// The label from a UDF volume descriptor sequence: the logical volume's
/// identifier, which is what file managers show, else the primary volume's.
pub fn udf_label(vds: &[u8]) -> Option<String> {
    let mut primary = None;
    for d in vds.as_chunks::<SECTOR>().0 {
        match u16::from_le_bytes([d[0], d[1]]) {
            // Logical volume descriptor: the identifier at 84, 128 bytes.
            6 => {
                if let Some(l) = dstring(&d[84..212]).filter(|l| !l.is_empty()) {
                    return Some(l);
                }
            }
            // Primary volume descriptor: the identifier at 24, 32 bytes.
            1 => primary = dstring(&d[24..56]).filter(|l| !l.is_empty()),
            // Terminating descriptor.
            8 => break,
            _ => {}
        }
    }
    primary
}

/// A UDF "dstring": a compression byte (8 for 8-bit, 16 for UTF-16 big
/// endian), the characters, and the length used in the last byte.
fn dstring(field: &[u8]) -> Option<String> {
    let used = *field.last()? as usize;
    if used == 0 {
        return Some(String::new());
    }
    let body = field.get(1..used.min(field.len().saturating_sub(1)))?;
    let s = match field.first()? {
        8 => body.iter().map(|&b| b as char).collect(),
        16 => String::from_utf16_lossy(&body.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect::<Vec<_>>()),
        _ => return None,
    };
    Some(s.trim().to_owned())
}

/// A MMC profile (from GET CONFIGURATION) as the media it names.
pub fn profile_media(profile: u16) -> Option<&'static str> {
    match profile {
        0x08..=0x0A => Some("CD"),
        0x10..=0x2B => Some("DVD"),
        0x40..=0x43 => Some("Blu-ray"),
        0x50..=0x5A => Some("HD DVD"),
        _ => None,
    }
}

/// The media guessed from the size alone, when the drive won't say.
pub fn media_by_size(bytes: u64) -> &'static str {
    match bytes {
        0..=900_000_000 => "CD",
        900_000_001..=9_500_000_000 => "DVD",
        _ => "Blu-ray",
    }
}

/// "4.3 GB", "650 MB": decimal, as discs are sold.
pub fn size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", (bytes as f64 / 1e6).round() as u64)
    }
}

/// The filesystem facts from the first sectors of the disc. `read(sector,
/// count)` reads 2048-byte sectors; a failed read is an empty one.
pub fn read_filesystem(read: impl Fn(u64, usize) -> Vec<u8>) -> (String, String) {
    let area = read(16, 16);
    let (iso, udf) = filesystems(&area);
    let iso_name = if iso { area.as_chunks::<SECTOR>().0.iter().find_map(|d| iso_label(d)) } else { None };
    let udf_name = if udf { read(256, 1).get(..SECTOR).and_then(udf_anchor).and_then(|(loc, len)| udf_label(&read(loc as u64, (len as usize / SECTOR).clamp(1, 32)))) } else { None };
    let filesystem = match (iso, udf) {
        (true, true) => "ISO 9660 + UDF",
        (true, false) => "ISO 9660",
        (false, true) => "UDF",
        (false, false) => "none",
    };
    // UDF's name can be longer than ISO 9660's 32 upper-case characters, so
    // it wins on a bridge disc.
    let label = udf_name.filter(|l| !l.is_empty()).or(iso_name).unwrap_or_default();
    (label, filesystem.to_owned())
}

// ---- reading the device ----------------------------------------------------------------------

/// Everything the Data disc step reports about the disc in `device`.
pub fn read(device: &str) -> Found {
    let bytes = sysfs_bytes(device);
    let read_sectors = |at: u64, n: usize| -> Vec<u8> {
        use std::io::{Read, Seek, SeekFrom};
        let mut buf = vec![0u8; n * SECTOR];
        let ok = std::fs::File::open(device)
            .and_then(|mut f| {
                f.seek(SeekFrom::Start(at * SECTOR as u64))?;
                f.read_exact(&mut buf)
            })
            .is_ok();
        if ok {
            buf
        } else {
            Vec::new()
        }
    };
    let (label, filesystem) = read_filesystem(read_sectors);
    let media = current_profile(device).and_then(profile_media).unwrap_or_else(|| media_by_size(bytes)).to_owned();
    let toc = toc(device);
    Found { label, filesystem, media, bytes, tracks: toc.len() as u32, audio_tracks: toc.iter().filter(|(_, data)| !data).count() as u32 }
}

/// The disc's size from sysfs, which counts 512-byte sectors.
/// Whether there's a disc in the drive, as the drive says just now.
/// Not the block device's size: that keeps the last disc's size after
/// the tray opens, until something reads the drive again.
#[cfg(target_os = "linux")]
pub fn disc_in(device: &str) -> bool {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    const CDROM_DRIVE_STATUS: libc::c_ulong = 0x5326;
    const CDS_NO_INFO: libc::c_int = 0;
    const CDS_DISC_OK: libc::c_int = 4;
    let Ok(f) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(device) else { return false };
    // SAFETY: an ioctl taking a slot number (CDSL_CURRENT) on a descriptor we hold open.
    let status = unsafe { libc::ioctl(f.as_raw_fd(), CDROM_DRIVE_STATUS, libc::c_int::MAX) };
    match status {
        CDS_DISC_OK => true,
        // A drive that can't say: fall back on the size.
        s if s < 0 || s == CDS_NO_INFO => sysfs_bytes(device) > 0,
        _ => false,
    }
}

#[cfg(not(target_os = "linux"))]
pub fn disc_in(device: &str) -> bool {
    sysfs_bytes(device) > 0
}

pub fn sysfs_bytes(device: &str) -> u64 {
    let real = std::fs::canonicalize(device).unwrap_or_else(|_| Path::new(device).to_path_buf());
    let name = real.file_name().and_then(|n| n.to_str()).unwrap_or("");
    std::fs::read_to_string(format!("/sys/class/block/{name}/size")).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0) * 512
}

/// The drive's current profile, from MMC GET CONFIGURATION through SG_IO:
/// what the drive itself says the disc is, rather than a guess from its size.
#[cfg(target_os = "linux")]
pub fn current_profile(device: &str) -> Option<u16> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;

    #[repr(C)]
    struct SgIoHdr {
        interface_id: libc::c_int,
        dxfer_direction: libc::c_int,
        cmd_len: u8,
        mx_sb_len: u8,
        iovec_count: u16,
        dxfer_len: u32,
        dxferp: *mut libc::c_void,
        cmdp: *const u8,
        sbp: *mut u8,
        timeout: u32,
        flags: u32,
        pack_id: libc::c_int,
        usr_ptr: *mut libc::c_void,
        status: u8,
        masked_status: u8,
        msg_status: u8,
        sb_len_wr: u8,
        host_status: u16,
        driver_status: u16,
        resid: libc::c_int,
        duration: u32,
        info: u32,
    }
    const SG_IO: libc::c_ulong = 0x2285;
    const SG_DXFER_FROM_DEV: libc::c_int = -3;

    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(device).ok()?;
    let mut data = [0u8; 16];
    let mut sense = [0u8; 32];
    // GET CONFIGURATION, RT=2 (one feature), from feature 0: the header
    // alone carries the current profile, in bytes 6 and 7.
    let cdb: [u8; 10] = [0x46, 0x02, 0, 0, 0, 0, 0, 0, data.len() as u8, 0];
    let mut h = SgIoHdr {
        interface_id: b'S' as libc::c_int,
        dxfer_direction: SG_DXFER_FROM_DEV,
        cmd_len: cdb.len() as u8,
        mx_sb_len: sense.len() as u8,
        iovec_count: 0,
        dxfer_len: data.len() as u32,
        dxferp: data.as_mut_ptr().cast(),
        cmdp: cdb.as_ptr(),
        sbp: sense.as_mut_ptr(),
        timeout: 5000,
        flags: 0,
        pack_id: 0,
        usr_ptr: std::ptr::null_mut(),
        status: 0,
        masked_status: 0,
        msg_status: 0,
        sb_len_wr: 0,
        host_status: 0,
        driver_status: 0,
        resid: 0,
        duration: 0,
        info: 0,
    };
    // SAFETY: `SgIoHdr` is laid out as the kernel's `sg_io_hdr_t`; the
    // command, data and sense pointers are to buffers that outlive the
    // call, and their lengths are the ones given in the header.
    let r = unsafe { libc::ioctl(f.as_raw_fd(), SG_IO, &mut h as *mut SgIoHdr) };
    (r == 0 && h.status == 0 && h.host_status == 0 && h.driver_status == 0).then(|| u16::from_be_bytes([data[6], data[7]])).filter(|p| *p != 0)
}

#[cfg(not(target_os = "linux"))]
pub fn current_profile(_device: &str) -> Option<u16> {
    None
}

/// The table of contents: each track's number and whether it's data, from
/// `CDROMREADTOCHDR` and `CDROMREADTOCENTRY`. Empty when there's none to read.
pub fn toc(device: &str) -> Vec<(u8, bool)> {
    toc_starts(device).into_iter().map(|(t, data, _)| (t, data)).collect()
}

/// The same, with where each track starts (its LBA).
#[cfg(target_os = "linux")]
pub fn toc_starts(device: &str) -> Vec<(u8, bool, u32)> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    const CDROMREADTOCHDR: libc::c_ulong = 0x5305;
    const CDROMREADTOCENTRY: libc::c_ulong = 0x5306;
    const CDROM_LBA: u8 = 0x01;

    #[repr(C)]
    struct TocHdr {
        first: u8,
        last: u8,
    }
    #[repr(C)]
    struct TocEntry {
        track: u8,
        /// `cdte_adr:4, cdte_ctrl:4`: the control nibble is the high one.
        adr_ctrl: u8,
        format: u8,
        lba: i32,
        datamode: u8,
    }

    let Ok(f) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(device) else { return vec![] };
    let mut hdr = TocHdr { first: 0, last: 0 };
    // SAFETY: both structs are laid out as the kernel's `cdrom_tochdr` and
    // `cdrom_tocentry` (an `int` in the address union), and live through each call.
    if unsafe { libc::ioctl(f.as_raw_fd(), CDROMREADTOCHDR, &mut hdr as *mut TocHdr) } != 0 || hdr.first > hdr.last {
        return vec![];
    }
    (hdr.first..=hdr.last)
        .filter_map(|track| {
            let mut e = TocEntry { track, adr_ctrl: 0, format: CDROM_LBA, lba: 0, datamode: 0 };
            (unsafe { libc::ioctl(f.as_raw_fd(), CDROMREADTOCENTRY, &mut e as *mut TocEntry) } == 0).then_some((track, is_data(e.adr_ctrl), e.lba.max(0) as u32))
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
pub fn toc_starts(_device: &str) -> Vec<(u8, bool, u32)> {
    vec![]
}

/// Whether a TOC entry's packed adr/control byte marks a data track: bit 2
/// of the control nibble, which the kernel's bitfield puts in the high half.
pub fn is_data(adr_ctrl: u8) -> bool {
    (adr_ctrl >> 4) & 0x04 != 0
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::string_slice)]
pub(crate) mod tests {
    use super::*;

    fn sector(id: &[u8; 5], kind: u8) -> Vec<u8> {
        let mut s = vec![0u8; SECTOR];
        s[0] = kind;
        s[1..6].copy_from_slice(id);
        s
    }

    /// A primary volume descriptor labelled `label`.
    pub fn pvd(label: &str) -> Vec<u8> {
        let mut s = sector(b"CD001", 1);
        let mut l = [b' '; 32];
        l[..label.len()].copy_from_slice(label.as_bytes());
        s[40..72].copy_from_slice(&l);
        s
    }

    fn tagged(id: u16) -> Vec<u8> {
        let mut s = vec![0u8; SECTOR];
        s[..2].copy_from_slice(&id.to_le_bytes());
        s
    }

    fn dstring8(s: &str, len: usize) -> Vec<u8> {
        let mut d = vec![0u8; len];
        d[0] = 8;
        d[1..1 + s.len()].copy_from_slice(s.as_bytes());
        d[len - 1] = (s.len() + 1) as u8;
        d
    }

    /// A disc's first 512 KiB and a bit: optionally ISO 9660 and UDF.
    pub fn image(iso: Option<&str>, udf: Option<&str>) -> Vec<u8> {
        let mut disc = vec![0u8; 16 * SECTOR];
        if let Some(l) = iso {
            disc.extend(pvd(l));
            disc.extend(sector(b"CD001", 255));
        }
        if udf.is_some() {
            disc.extend(sector(b"BEA01", 0));
            disc.extend(sector(b"NSR02", 0));
            disc.extend(sector(b"TEA01", 0));
        }
        disc.resize(256 * SECTOR, 0);
        if let Some(l) = udf {
            let mut avdp = tagged(2);
            avdp[16..20].copy_from_slice(&(4 * SECTOR as u32).to_le_bytes());
            avdp[20..24].copy_from_slice(&257u32.to_le_bytes());
            disc.extend(avdp);
            let mut pv = tagged(1);
            pv[24..56].copy_from_slice(&dstring8("SHORT", 32));
            disc.extend(pv);
            let mut lv = tagged(6);
            lv[84..212].copy_from_slice(&dstring8(l, 128));
            disc.extend(lv);
            disc.extend(tagged(8));
            disc.extend(vec![0u8; SECTOR]);
        }
        disc
    }

    pub fn reader(disc: &[u8]) -> impl Fn(u64, usize) -> Vec<u8> + '_ {
        move |at, n| {
            let from = at as usize * SECTOR;
            disc.get(from..from + n * SECTOR).map(<[u8]>::to_vec).unwrap_or_default()
        }
    }

    #[test]
    fn an_iso_9660_disc_gives_its_label() {
        let d = image(Some("DRIVERS_2024"), None);
        assert_eq!(read_filesystem(reader(&d)), ("DRIVERS_2024".into(), "ISO 9660".into()));
    }

    #[test]
    fn a_blu_ray_has_only_udf_and_its_label_comes_from_there() {
        let d = image(None, Some("BACKUP 2023 PHOTOS"));
        assert_eq!(read_filesystem(reader(&d)), ("BACKUP 2023 PHOTOS".into(), "UDF".into()));
    }

    #[test]
    fn a_bridge_disc_has_both_and_udf_names_it() {
        let d = image(Some("HOLIDAY"), Some("Holiday 2019"));
        assert_eq!(read_filesystem(reader(&d)), ("Holiday 2019".into(), "ISO 9660 + UDF".into()));
    }

    #[test]
    fn a_blank_label_is_still_a_filesystem() {
        let d = image(Some(""), None);
        assert_eq!(read_filesystem(reader(&d)), (String::new(), "ISO 9660".into()));
        assert_eq!(read_filesystem(reader(&[])), (String::new(), "none".into()), "an audio CD reads nothing");
    }

    #[test]
    fn utf16_labels_decode() {
        let mut f = vec![0u8; 32];
        f[0] = 16;
        for (i, c) in "Été".encode_utf16().enumerate() {
            f[1 + 2 * i..3 + 2 * i].copy_from_slice(&c.to_be_bytes());
        }
        f[31] = 7;
        assert_eq!(dstring(&f).as_deref(), Some("Été"));
    }

    #[test]
    fn media_from_the_drive_profile_or_the_size() {
        assert_eq!(profile_media(0x08), Some("CD"));
        assert_eq!(profile_media(0x10), Some("DVD"));
        assert_eq!(profile_media(0x2B), Some("DVD"), "DVD+R DL");
        assert_eq!(profile_media(0x40), Some("Blu-ray"));
        assert_eq!(profile_media(0x02), None, "a removable disk is not a disc");
        assert_eq!(media_by_size(700_000_000), "CD");
        assert_eq!(media_by_size(4_700_000_000), "DVD");
        assert_eq!(media_by_size(25_000_000_000), "Blu-ray");
        assert_eq!(size(4_300_000_000), "4.3 GB");
        assert_eq!(size(650_000_000), "650 MB");
    }

    #[test]
    fn the_control_nibble_says_data() {
        // adr 1, control 4 (data): 0x41. adr 1, control 0 or 2 (audio): 0x10, 0x12... as packed.
        assert!(is_data(0x41));
        assert!(!is_data(0x01));
        assert!(!is_data(0x21), "copy permitted is not data");
    }

    /// Reads whatever is in the drive. Run by hand with a disc in:
    /// `cargo test -p drt-steps real_drive -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_drive() {
        let dev = std::env::var("DRT_DEVICE").unwrap_or_else(|_| "/dev/sr0".into());
        println!("{:?} (profile {:?})", read(&dev), current_profile(&dev));
    }
}

/// A picture's width and height, from its header: PNG or JPEG (what the
/// Cover Art Archive gives). `None` for anything else.
pub fn picture_size(path: &Path) -> Option<(u32, u32)> {
    let b = std::fs::read(path).ok()?;
    if b.starts_with(b"\x89PNG") {
        let b: &[u8; 24] = b.first_chunk()?;
        let w = u32::from_be_bytes(b[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(b[20..24].try_into().ok()?);
        return Some((w, h));
    }
    if !b.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    // JPEG: walk the segments to a start-of-frame (SOF0..SOF15, not DHT,
    // JPG or DAC), whose payload has the height, then the width.
    let mut i = 2;
    // A segment's marker, its length, and the first bytes of a frame's.
    while let Some(s) = b.get(i..).and_then(<[u8]>::first_chunk::<9>).filter(|_| i + 9 < b.len()) {
        if s[0] != 0xFF {
            return None;
        }
        let marker = s[1];
        let len = u16::from_be_bytes([s[2], s[3]]) as usize;
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let h = u16::from_be_bytes([s[5], s[6]]) as u32;
            let w = u16::from_be_bytes([s[7], s[8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::string_slice)]
mod picture_tests {
    /// Cut short or lying about their lengths: no size, and no panic.
    #[test]
    fn a_truncated_or_hostile_picture_is_no_panic() {
        let dir = std::env::temp_dir().join(format!("drt-pic-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x");
        for b in [&b"\x89PNG"[..], b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0", b"\xFF\xD8", b"\xFF\xD8\xFF\xC0\0", b"\xFF\xD8\xFF\xE0\xFF\xFF\0\0\0\0\0\0", b"\xFF\xD8\xFF\xE0\0\0\0\0\0\0\0\0\0\0"] {
            std::fs::write(&p, b).unwrap();
            assert_eq!(super::picture_size(&p), None, "{b:?}");
        }
    }

    #[test]
    fn short_descriptors_are_no_panic() {
        assert_eq!(super::iso_label(b"\x01CD001"), None);
        assert_eq!(super::udf_anchor(&[2, 0, 0]), None);
        assert_eq!(super::udf_label(&[6u8; 100]), None);
    }

    #[test]
    fn a_pngs_size_from_its_header() {
        let dir = std::env::temp_dir().join(format!("drt-pic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.png");
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(1200u32.to_be_bytes());
        b.extend(1190u32.to_be_bytes());
        std::fs::write(&p, &b).unwrap();
        assert_eq!(super::picture_size(&p), Some((1200, 1190)));
        // A JPEG: SOI, an APP0 of 4 bytes, then SOF0 with 8 bits, 500 × 600.
        let j = dir.join("a.jpg");
        std::fs::write(&j, [0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 17, 8, 2, 0x58, 1, 0xF4, 3, 0, 0, 0]).unwrap();
        assert_eq!(super::picture_size(&j), Some((500, 600)));
    }
}
