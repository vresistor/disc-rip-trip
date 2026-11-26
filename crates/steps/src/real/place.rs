//! Putting a file in a folder you chose: under a temporary name first and
//! renamed into place, so a half-written file never sits there under its
//! real name.
//!
//! The temporary file is always a new one. Whatever was left at its name (a
//! stale `.part`, or a link someone put there) is removed first, and the
//! file is made with `O_EXCL`, which refuses to follow a link. The rename
//! replaces a link at the real name rather than writing through it. So a
//! write lands at the path asked for, never wherever a link there pointed.

use std::io::Write;
use std::path::{Path, PathBuf};

/// `name.ext.part` beside `path`.
pub fn part(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".part");
    PathBuf::from(p)
}

/// A new, empty file at `path`; what was there is removed, not followed.
fn fresh(path: &Path) -> std::io::Result<std::fs::File> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(path)
}

/// Writes `bytes` to `path` through a temporary file.
pub fn write(path: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    let tmp = part(path);
    let mut f = fresh(&tmp)?;
    f.write_all(bytes.as_ref())?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// Copies `src` to `dest` through a temporary file.
pub fn copy_blocking(src: &Path, dest: &Path) -> std::io::Result<()> {
    let tmp = part(dest);
    let mut from = std::fs::File::open(src)?;
    let mut to = fresh(&tmp)?;
    std::io::copy(&mut from, &mut to)?;
    drop(to);
    std::fs::rename(&tmp, dest)
}

/// [`copy_blocking`], off the async threads.
pub async fn copy(src: &Path, dest: &Path) -> std::io::Result<()> {
    let (src, dest) = (src.to_owned(), dest.to_owned());
    tokio::task::spawn_blocking(move || copy_blocking(&src, &dest)).await.map_err(std::io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("trip-place-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_link_left_at_the_temporary_name_is_not_written_through() {
        let d = dir("part-link");
        let victim = d.join("victim");
        std::fs::write(&victim, "keep me").unwrap();
        let dest = d.join("01 - Title.flac");
        symlink(&victim, part(&dest)).unwrap();
        write(&dest, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new");
        assert!(!part(&dest).exists());

        let src = d.join("src");
        std::fs::write(&src, "copied").unwrap();
        let dest = d.join("02 - Title.flac");
        symlink(&victim, part(&dest)).unwrap();
        copy_blocking(&src, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "copied");
    }

    #[test]
    fn a_link_at_the_real_name_is_replaced_not_followed() {
        let d = dir("dest-link");
        let victim = d.join("victim");
        std::fs::write(&victim, "keep me").unwrap();
        let dest = d.join("Game.sbi");
        symlink(&victim, &dest).unwrap();
        write(&dest, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        assert!(!std::fs::symlink_metadata(&dest).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new");
    }

    #[test]
    fn a_stale_part_is_replaced() {
        let d = dir("stale");
        let dest = d.join("x.cue");
        std::fs::write(part(&dest), "half a file from last time").unwrap();
        write(&dest, "whole").unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "whole");
    }
}
