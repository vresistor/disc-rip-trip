//! What kind of place a folder is on: a network share, a drive, or one
//! the app can't tell. Only said where the system says so reliably; else
//! [`Kind::Unknown`], and the window's words stay neutral.
//!
//! - **Linux:** the mount table (`/proc/self/mountinfo`) gives the
//!   filesystem a path is on without touching it: cifs, smb3, nfs, sshfs
//!   and the like are network ones. A GVFS path names its backend
//!   (`smb-share:`, `sftp:`, `mtp:` …). A document-portal path (what a
//!   flatpak sees of a folder it was given) is followed to the folder it
//!   stands for first. A folder under `/run/media`, `/media` or `/mnt` is
//!   on a drive.
//! - **Windows:** a UNC path (`\\server\share`), or a drive letter Windows
//!   says is a network one (`GetDriveTypeW`: remote); removable is a drive.
//! - **macOS:** `statfs` without `MNT_LOCAL` is a network share; a folder
//!   under `/Volumes` is a drive.
//!
//! Asking can touch the folder (the portal, `statfs`), so it's done on a
//! thread of its own, given up after a moment, and kept for a while: the
//! window asks every frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What kind of place a folder is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A network share.
    Network,
    /// A drive of its own: removable, or mounted where drives go.
    Drive,
    /// Can't tell.
    Unknown,
}

/// How long an answer is kept.
const KEEP: Duration = Duration::from_secs(30);
/// How long asking may take.
const WAIT: Duration = Duration::from_millis(300);

static KNOWN: Mutex<Option<HashMap<PathBuf, (Kind, Instant)>>> = Mutex::new(None);

/// What kind of place `p` is on, as this system can tell. Kept a while;
/// [`Kind::Unknown`] if asking takes too long.
pub fn kind_of(p: &Path) -> Kind {
    let now = Instant::now();
    if let Some((k, at)) = KNOWN.lock().unwrap().as_ref().and_then(|m| m.get(p).copied()) {
        if now.duration_since(at) < KEEP {
            return k;
        }
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let q = p.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(kind_now(&q));
    });
    let k = rx.recv_timeout(WAIT).unwrap_or(Kind::Unknown);
    KNOWN.lock().unwrap().get_or_insert_with(HashMap::new).insert(p.to_path_buf(), (k, now));
    k
}

/// [`kind_of`], asked now, blocking.
#[cfg(target_os = "linux")]
fn kind_now(p: &Path) -> Kind {
    let p = portal_host(p).unwrap_or_else(|| p.to_path_buf());
    let table = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    linux_kind(&p, &table)
}

#[cfg(target_os = "macos")]
fn kind_now(p: &Path) -> Kind {
    use std::os::unix::ffi::OsStrExt;
    // The nearest folder there: statfs on one that's gone says nothing.
    let mut at = p;
    let local = loop {
        let Ok(c) = std::ffi::CString::new(at.as_os_str().as_bytes()) else { break None };
        let mut st: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: a NUL-terminated path and a statfs to fill, both ours.
        if unsafe { libc::statfs(c.as_ptr(), &mut st) } == 0 {
            break Some(st.f_flags & libc::MNT_LOCAL as u32 != 0);
        }
        match at.parent() {
            Some(up) => at = up,
            None => break None,
        }
    };
    macos_kind(p, local)
}

#[cfg(windows)]
fn kind_now(p: &Path) -> Kind {
    use std::os::windows::ffi::OsStrExt;
    let s = p.to_string_lossy();
    if unc(&s) {
        return Kind::Network;
    }
    let Some(root) = drive_root(&s) else { return Kind::Unknown };
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(&root).encode_wide().chain([0]).collect();
    // SAFETY: a NUL-terminated wide string of ours.
    windows_kind(unsafe { GetDriveTypeW(wide.as_ptr()) })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn kind_now(_: &Path) -> Kind {
    Kind::Unknown
}

/// The filesystem types that are network shares, as Linux names them.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const NETWORK_FS: &[&str] = &["cifs", "smb3", "smbfs", "nfs", "nfs4", "fuse.sshfs", "sshfs", "fuse.gvfsd-fuse", "davfs", "fuse.davfs2", "fuse.rclone", "afpfs", "ncpfs", "9p"];

/// Linux: by GVFS's backend in the path, else the filesystem of the
/// nearest mount in the table (mountinfo's text), else where drives go.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn linux_kind(p: &Path, mountinfo: &str) -> Kind {
    let s = p.to_string_lossy();
    if let Some(rest) = s.split("/gvfs/").nth(1) {
        // A phone or a camera is a drive; a share is a network one.
        if ["mtp:", "gphoto2:", "afc:"].iter().any(|k| rest.starts_with(k)) {
            return Kind::Drive;
        }
        if ["smb-share:", "smb-server:", "nfs:", "sftp:", "ftp:", "ftps:", "dav:", "davs:", "afp-volume:", "google-drive:", "onedrive:"].iter().any(|k| rest.starts_with(k)) {
            return Kind::Network;
        }
        return Kind::Unknown;
    }
    let fs = mountinfo
        .lines()
        .filter_map(|l| {
            // id parent major:minor root mount-point options … - type source …
            let (head, tail) = l.split_once(" - ")?;
            let at = head.split_whitespace().nth(4)?;
            let kind = tail.split_whitespace().next()?;
            Some((PathBuf::from(unescape(at)), kind))
        })
        .filter(|(at, _)| p.starts_with(at))
        .max_by_key(|(at, _)| at.components().count());
    if fs.is_some_and(|(_, kind)| NETWORK_FS.contains(&kind)) {
        return Kind::Network;
    }
    if ["/run/media", "/media", "/mnt"].iter().any(|r| p.starts_with(r)) {
        return Kind::Drive;
    }
    Kind::Unknown
}

/// mountinfo writes a space, tab, newline and backslash as `\ooo`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A document-portal path's own folder on the computer, as the portal
/// says (its `user.document-portal.host-path`), for the folder or the
/// nearest one above it that's there. `None` for any other path, or when
/// the portal doesn't say.
#[cfg(target_os = "linux")]
fn portal_host(p: &Path) -> Option<PathBuf> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let s = p.to_string_lossy();
    if !(s.starts_with("/run/user/") && s.contains("/doc/")) {
        return None;
    }
    let name = c"user.document-portal.host-path";
    let mut at = p;
    loop {
        let c = std::ffi::CString::new(at.as_os_str().as_bytes()).ok()?;
        let mut buf = vec![0u8; 4096];
        // SAFETY: a NUL-terminated path and name, and a buffer of its length.
        let n = unsafe { libc::getxattr(c.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            buf.truncate(n as usize);
            // Some portals end it with a NUL.
            while buf.last() == Some(&0) {
                buf.pop();
            }
            let host = PathBuf::from(std::ffi::OsString::from_vec(buf));
            return Some(host.join(p.strip_prefix(at).ok()?));
        }
        at = at.parent()?;
        if !at.to_string_lossy().contains("/doc/") {
            return None;
        }
    }
}

/// macOS: not local by `statfs` is a network share (`None`: it couldn't
/// say); else a folder under `/Volumes` is a drive.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn macos_kind(p: &Path, local: Option<bool>) -> Kind {
    match local {
        Some(false) => Kind::Network,
        _ if p.starts_with("/Volumes") => Kind::Drive,
        _ => Kind::Unknown,
    }
}

/// Whether a Windows path is a UNC one: `\\server\share`, not `\\?\C:`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn unc(s: &str) -> bool {
    let s = s.replace('/', "\\");
    if let Some(rest) = s.strip_prefix("\\\\?\\").or_else(|| s.strip_prefix("\\\\.\\")) {
        return rest.len() >= 4 && rest[..4].eq_ignore_ascii_case("UNC\\");
    }
    s.starts_with("\\\\")
}

/// A Windows path's drive root: `C:\` for `C:\Music`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn drive_root(s: &str) -> Option<String> {
    let s = s.strip_prefix("\\\\?\\").unwrap_or(s);
    let b = s.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':').then(|| format!("{}:\\", b[0] as char))
}

/// What `GetDriveTypeW` says, as a kind: 4 remote, 2 removable.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_kind(t: u32) -> Kind {
    match t {
        4 => Kind::Network,
        2 => Kind::Drive,
        _ => Kind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
22 1 0:21 / / rw,relatime - btrfs /dev/nvme0n1p3 rw
40 22 0:40 / /mnt/nas rw,relatime - nfs4 nas:/music rw
41 22 0:41 / /mnt/a\\040b rw,relatime - cifs //server/a\\040b rw
42 40 8:17 / /mnt/nas/usb rw,relatime - ext4 /dev/sdb1 rw
43 22 0:50 / /home/alice/remote rw - fuse.sshfs alice@box: rw
44 22 0:51 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw
";

    #[test]
    fn linux_tells_a_network_share_by_its_filesystem() {
        let k = |p: &str| linux_kind(Path::new(p), TABLE);
        assert_eq!(k("/mnt/nas/FLAC"), Kind::Network);
        assert_eq!(k("/mnt/a b/FLAC"), Kind::Network, "a space in the mount point");
        assert_eq!(k("/home/alice/remote/Music"), Kind::Network);
        // The nearest mount counts: a drive plugged into the share's box.
        assert_eq!(k("/mnt/nas/usb/FLAC"), Kind::Drive);
        // Where drives go, not mounted now: a drive.
        assert_eq!(k("/run/media/alice/backup/FLAC"), Kind::Drive);
        assert_eq!(k("/media/backup"), Kind::Drive);
        assert_eq!(k("/mnt/gone/FLAC"), Kind::Drive);
        // Anywhere else on a local disk: can't tell.
        assert_eq!(k("/home/alice/Music"), Kind::Unknown);
        assert_eq!(k("/srv/music"), Kind::Unknown);
    }

    #[test]
    fn linux_tells_gvfs_by_its_backend() {
        let k = |p: &str| linux_kind(Path::new(p), TABLE);
        assert_eq!(k("/run/user/1000/gvfs/smb-share:server=nas,share=music/FLAC"), Kind::Network);
        assert_eq!(k("/run/user/1000/gvfs/sftp:host=box/home/a"), Kind::Network);
        assert_eq!(k("/run/user/1000/gvfs/mtp:host=Phone/Music"), Kind::Drive);
        assert_eq!(k("/run/user/1000/gvfs/archive:host=x/y"), Kind::Unknown);
    }

    #[test]
    fn a_document_portal_path_not_given_by_the_portal_is_not_guessed() {
        // No portal here: it can't say, so neither can we.
        assert_eq!(linux_kind(Path::new("/run/user/1000/doc/1a2b3c/FLAC"), TABLE), Kind::Unknown);
    }

    #[test]
    fn macos_tells_by_statfs_and_volumes() {
        assert_eq!(macos_kind(Path::new("/Volumes/music/FLAC"), Some(false)), Kind::Network);
        assert_eq!(macos_kind(Path::new("/Volumes/Backup/FLAC"), Some(true)), Kind::Drive);
        assert_eq!(macos_kind(Path::new("/Volumes/Backup/FLAC"), None), Kind::Drive);
        assert_eq!(macos_kind(Path::new("/Users/alice/Music"), Some(true)), Kind::Unknown);
    }

    #[test]
    fn windows_tells_by_unc_and_the_drive_type() {
        assert!(unc(r"\\nas\music\FLAC"));
        assert!(unc(r"\\?\UNC\nas\music"));
        assert!(!unc(r"\\?\C:\Music"));
        assert!(!unc(r"C:\Music"));
        assert_eq!(drive_root(r"E:\Music"), Some(r"E:\".into()));
        assert_eq!(drive_root(r"\\?\E:\Music"), Some(r"E:\".into()));
        assert_eq!(drive_root(r"\\nas\music"), None);
        assert_eq!(windows_kind(4), Kind::Network);
        assert_eq!(windows_kind(2), Kind::Drive);
        assert_eq!(windows_kind(3), Kind::Unknown);
    }

    #[test]
    fn the_answer_is_kept_and_never_waited_on_long() {
        let at = Instant::now();
        let k = kind_of(Path::new("/run/media/nobody/backup"));
        assert!(at.elapsed() < Duration::from_secs(1));
        assert_eq!(kind_of(Path::new("/run/media/nobody/backup")), k);
    }
}
