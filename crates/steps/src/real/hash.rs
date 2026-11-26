//! CRC32, MD5 and SHA-1 of a dump's files, in one read of each.
//!
//! Done here rather than by `redumper hash`, which prints the same three in
//! Redump's own `<rom …/>` shape: an 8 GB ISO takes minutes, and here that
//! time has a progress bar and a Stop that works, and needs no tool. The
//! output is the same `Rom` a datfile holds, checked against `redumper
//! hash` on the same file (see the test).

use super::redump::Rom;
use anyhow::{bail, Result};
use md5::Digest;
use std::io::Read;
use std::path::Path;

/// Hashes `files`, calling `progress` with the fraction of all their bytes
/// read so far. Stops with an error as soon as `stopped` says so.
pub fn hash_files(files: &[&Path], mut progress: impl FnMut(f32), stopped: impl Fn() -> bool) -> Result<Vec<Rom>> {
    let total: u64 = files.iter().map(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)).sum::<u64>().max(1);
    let mut done = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    let mut out = Vec::new();
    for path in files {
        let mut f = std::fs::File::open(path)?;
        let (mut crc, mut md5, mut sha1) = (crc32fast::Hasher::new(), md5::Md5::new(), sha1::Sha1::new());
        let mut size = 0u64;
        loop {
            if stopped() {
                bail!("stopped");
            }
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            crc.update(&buf[..n]);
            md5.update(&buf[..n]);
            sha1.update(&buf[..n]);
            size += n as u64;
            done += n as u64;
            progress(done as f32 / total as f32);
        }
        out.push(Rom {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            size,
            crc: format!("{:08x}", crc.finalize()),
            md5: hex(&md5.finalize()),
            sha1: hex(&sha1.finalize()),
        });
    }
    Ok(out)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_hashes_of_a_file() {
        let dir = std::env::current_dir().unwrap().join("../../target/trip-test/hash");
        std::fs::create_dir_all(&dir).unwrap();
        // 40960 zero bytes. `redumper hash` (b744) on the same file printed
        // <rom name="test.iso" size="40960" crc="2c2bb90a"
        // md5="ab893875d697a3145af5eed5309bee26"
        // sha1="c90116149196cbf74ffb453ecb3b12945372ebfa" />.
        let f = dir.join("test.iso");
        std::fs::write(&f, vec![0u8; 40960]).unwrap();
        let mut seen = vec![];
        let r = hash_files(&[&f], |p| seen.push(p), || false).unwrap();
        assert_eq!(r[0].name, "test.iso");
        assert_eq!(r[0].size, 40960);
        assert_eq!(r[0].crc, "2c2bb90a");
        assert_eq!(r[0].md5, "ab893875d697a3145af5eed5309bee26");
        assert_eq!(r[0].sha1, "c90116149196cbf74ffb453ecb3b12945372ebfa");
        assert_eq!(seen.last(), Some(&1.0));
        assert!(hash_files(&[&f], |_| {}, || true).is_err(), "Stop stops it");
    }

    /// The same answers as coreutils' `sha1sum`, `md5sum` and `cksum -a
    /// crc32b` (the zlib CRC-32 Redump's datfiles list) on a file made
    /// here. A tool that isn't there skips its own comparison only.
    #[test]
    fn the_hashes_are_coreutils_own() {
        let dir = std::env::current_dir().unwrap().join("../../target/trip-test/hash-coreutils");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("sectors.bin");
        std::fs::write(&f, (0..2352 * 300u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect::<Vec<u8>>()).unwrap();
        let r = hash_files(&[&f], |_| {}, || false).unwrap().remove(0);
        let tool = |cmd: &str, args: &[&str]| std::process::Command::new(cmd).args(args).arg(&f).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        let mut compared = 0;
        if let Some(out) = tool("sha1sum", &[]) {
            assert_eq!(out.split_whitespace().next(), Some(r.sha1.as_str()));
            compared += 1;
        }
        if let Some(out) = tool("md5sum", &[]) {
            assert_eq!(out.split_whitespace().next(), Some(r.md5.as_str()));
            compared += 1;
        }
        if let Some(out) = tool("cksum", &["-a", "crc32b"]) {
            let n: u32 = out.split_whitespace().next().unwrap().parse().unwrap();
            assert_eq!(format!("{n:08x}"), r.crc);
        }
        assert!(compared == 2 || std::env::var("CI").is_ok(), "sha1sum and md5sum are on every Linux this is built on");
        assert_eq!(r.size, 2352 * 300);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
