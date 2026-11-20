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
        let dir = std::env::current_dir().unwrap().join("../../target/drt-test/hash");
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
}
