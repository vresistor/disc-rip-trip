//! Whether a folder can take a disc's files now: there, or makeable from
//! the nearest folder above it that is; writable; with room.
//!
//! A folder on a network share or a drive that's gone can make even a
//! `stat` hang, so the look runs on a thread of its own and is given up
//! after a timeout. A thread, not `spawn_blocking`: a runtime waits for its
//! blocking tasks when it closes, and one stuck on a dead mount would keep
//! the app from quitting.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a look may take before the folder counts as not answering.
pub const TIMEOUT: Duration = Duration::from_secs(3);

/// Why a folder can't take the files now.
#[derive(Clone, Debug, PartialEq)]
pub enum Unreachable {
    /// A removable drive's folder, and the drive isn't there.
    Unplugged { drive: PathBuf },
    /// There, or meant to be, and can't be read: a network folder that's
    /// gone, a folder the app was given that it can't open now.
    CantReach { path: PathBuf, network: bool },
    /// Didn't answer in time.
    NotAnswering { path: PathBuf },
    /// There, or makeable, but nothing can be written in it.
    CantWrite { path: PathBuf, missing: bool },
    /// A file where the folder, or one on the way to it, should be.
    NotAFolder { path: PathBuf },
    /// Not enough free space: bytes needed, bytes free.
    NoRoom { path: PathBuf, need: u64, free: u64 },
}

impl Unreachable {
    /// The folder it's about.
    pub fn path(&self) -> &Path {
        match self {
            Unreachable::Unplugged { drive: path }
            | Unreachable::CantReach { path, .. }
            | Unreachable::NotAnswering { path }
            | Unreachable::CantWrite { path, .. }
            | Unreachable::NotAFolder { path }
            | Unreachable::NoRoom { path, .. } => path,
        }
    }

    /// One sentence, with the folder and what exactly went wrong: for the
    /// log. The window says it as [`Unreachable::title`] and
    /// [`Unreachable::hint`].
    pub fn sentence(&self) -> String {
        match self {
            Unreachable::Unplugged { drive } => format!("The drive {} isn't plugged in", shown(drive)),
            Unreachable::CantReach { path, network: true } => format!("Can't reach the network folder {}", shown(path)),
            Unreachable::CantReach { path, network: false } => format!("Can't reach the folder {}", shown(path)),
            Unreachable::NotAnswering { path } => format!("The folder {} isn't answering", shown(path)),
            Unreachable::CantWrite { path, missing: true } => format!("Can't make the folder {}", shown(path)),
            Unreachable::CantWrite { path, missing: false } => format!("Can't write to the folder {}", shown(path)),
            Unreachable::NotAFolder { path } => format!("A file is where the folder {} should be", shown(path)),
            Unreachable::NoRoom { path, need, free } => format!("Not enough room in {}: {} needed, {} free", shown(path), gb(*need), gb(*free)),
        }
    }

    /// What you can do about it, which is all the window tells apart.
    pub fn trouble(&self) -> Trouble {
        match self {
            // A folder that can't be made is, nearly always, one on a share
            // or a drive that isn't mounted: its mount point is there, and
            // nothing can be written in it.
            Unreachable::Unplugged { .. } | Unreachable::CantReach { .. } | Unreachable::NotAnswering { .. } | Unreachable::CantWrite { missing: true, .. } | Unreachable::NotAFolder { .. } => {
                Trouble::OutOfReach
            }
            Unreachable::CantWrite { missing: false, .. } => Trouble::NotAllowed,
            Unreachable::NoRoom { .. } => Trouble::NoRoom,
        }
    }

    /// A few words, the folder called `name` (its last part, from
    /// [`name_of`]): *Can't reach FLAC*. The same on a card, in Start's
    /// reason, in the list and in the sidebar.
    pub fn title(&self, name: &str) -> String {
        match self.trouble() {
            Trouble::OutOfReach => format!("Can't reach {name}"),
            Trouble::NoRoom => format!("Not enough room in {name}"),
            Trouble::NotAllowed => format!("Not allowed to save in {name}"),
        }
    }

    /// One short line under the title: what to check, or the numbers.
    pub fn hint(&self) -> String {
        match self {
            Unreachable::NoRoom { need, free, .. } => format!("{} needed, {} free", gb(*need), gb(*free)),
            Unreachable::CantWrite { missing: false, .. } => "Check Trip may save in it, or choose another folder.".into(),
            Unreachable::CantReach { network: true, .. } => check_for(super::mounts::Kind::Network).into(),
            _ => check_for(super::mounts::kind_of(self.path())).into(),
        }
    }

    /// The title and the hint as one sentence: why Start can't.
    pub fn said(&self, name: &str) -> String {
        match self.trouble() {
            Trouble::NoRoom => format!("{}: {}", self.title(name), self.hint()),
            _ => format!("{}. {}", self.title(name), self.hint()),
        }
    }

    /// For a card: the title, and the hint as its detail line.
    pub fn card(&self, name: &str) -> (String, String) {
        (self.title(name), self.hint())
    }

    /// Whether it's only short of room: the folder itself is fine.
    pub fn full(&self) -> bool {
        matches!(self, Unreachable::NoRoom { .. })
    }
}

/// Why a folder can't take a disc, as far as you're concerned: each asks
/// something different of you.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// A share off or offline, a drive unplugged, a folder not answering or not
    /// mounted: bring it back.
    OutOfReach,
    /// There, and full: make room.
    NoRoom,
    /// There, and saving in it isn't allowed.
    NotAllowed,
}

/// What to check when a folder is out of reach, by what kind of place
/// it's on; neutral where that can't be told.
pub fn check_for(kind: super::mounts::Kind) -> &'static str {
    use super::mounts::Kind;
    match kind {
        Kind::Network => "Check the network folder is on and connected.",
        Kind::Drive => "Check the drive is plugged in.",
        Kind::Unknown => "Check the folder still exists.",
    }
}

/// A folder by name, as the window says it: its last part (*FLAC*), or
/// the whole path when it has none.
pub fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| shown(p))
}

/// Sizes as a CD ripper reads them: *2.1 GB*, *0.4 GB*, *650 MB*.
pub fn gb(bytes: u64) -> String {
    if bytes >= 100_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", (bytes as f64 / 1e6).ceil() as u64)
    }
}

/// A path as said: `~` for home; a long one by its start and its last
/// folder (*/run/…/FLAC*).
pub fn shown(p: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.components().count() > 1);
    if let Some(rest) = home.as_ref().and_then(|h| p.strip_prefix(h).ok()) {
        return if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) };
    }
    let parts: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    // A document-portal folder's middle is a number nobody knows.
    if parts.len() > 4 {
        return format!("/{}/…/{}", parts[1], parts[parts.len() - 1]);
    }
    p.display().to_string()
}

/// Free bytes on the filesystem holding a path, as `statvfs` says;
/// `None` when it can't say (some network and portal filesystems give
/// nothing).
pub type FreeFn = fn(&Path) -> io::Result<Option<u64>>;

/// [`FreeFn`] from `statvfs`.
pub fn statvfs_free(p: &Path) -> io::Result<Option<u64>> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(io::Error::other)?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and a statvfs to fill, both ours.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if st.f_blocks == 0 {
        return Ok(None);
    }
    Ok(Some(st.f_bavail as u64 * st.f_frsize as u64))
}

/// Where removable drives are mounted: a folder right under one of these
/// (or under its per-user folder) is a drive.
const DRIVE_ROOTS: &[&str] = &["/run/media", "/media"];

/// Whether a path is on a network share as GVFS mounts them.
fn network(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.contains("/gvfs/") && ["smb-share:", "nfs:", "sftp:", "dav:", "davs:", "afp-volume:", "ftp:"].iter().any(|k| s.contains(k))
}

/// The drive a missing folder would be on, when the nearest folder there
/// is a drives root: `/run/media/backup` for `/run/media/backup/Music`.
fn drive_of(there: &Path, path: &Path) -> Option<PathBuf> {
    // Right under a root, or under its per-user folder (/run/media/alice).
    let root = DRIVE_ROOTS.iter().map(Path::new).find(|r| there == *r || there.parent() == Some(*r))?;
    let mut rest = path.strip_prefix(there).ok()?.components().map(|c| c.as_os_str().to_owned());
    let next = rest.next()?;
    // The per-user folder itself missing (no drive plugged in since the
    // computer started): the drive is the folder after it.
    let user = std::env::var_os("USER").filter(|u| there == root && *u == next);
    match user.and_then(|_| rest.next()) {
        Some(drive) => Some(there.join(next).join(drive)),
        None => Some(there.join(next)),
    }
}

/// Looks at a folder now, blocking: there or makeable, writable, with
/// `need` bytes free. Writes and removes a small file in the folder, or in
/// the nearest folder above it that's there; never makes the folder itself.
pub fn look(path: &Path, need: u64, free: FreeFn) -> Result<(), Unreachable> {
    let mut there = path.to_path_buf();
    loop {
        match std::fs::metadata(&there) {
            Ok(m) if m.is_dir() => break,
            Ok(_) => return Err(Unreachable::NotAFolder { path: path.into() }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => match there.parent() {
                Some(up) => there = up.to_path_buf(),
                None => return Err(Unreachable::CantReach { path: path.into(), network: network(path) }),
            },
            // A file on the way: nothing can be made under it.
            Err(e) if e.raw_os_error() == Some(libc::ENOTDIR) => return Err(Unreachable::NotAFolder { path: path.into() }),
            Err(_) => return Err(Unreachable::CantReach { path: path.into(), network: network(path) }),
        }
    }
    let missing = there != path;
    if missing {
        if let Some(drive) = drive_of(&there, path) {
            return Err(Unreachable::Unplugged { drive });
        }
    }
    if let Err(e) = probe(&there) {
        // Refused on a disk here is the folder's own; anywhere else (a
        // network share, a folder the portal no longer gives) it's away.
        let remote = network(path) || portal(&there);
        let refused = matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem);
        return Err(if refused && !remote { Unreachable::CantWrite { path: path.into(), missing } } else { Unreachable::CantReach { path: path.into(), network: network(path) } });
    }
    // What the document portal or GVFS says is free is the computer's own
    // disk's, or nothing: not the network folder's. Not counted.
    if portal(&there) || network(path) {
        return Ok(());
    }
    match free(&there) {
        Ok(Some(f)) if f < need => Err(Unreachable::NoRoom { path: path.into(), need, free: f }),
        // Can't say: not counted against it.
        _ => Ok(()),
    }
}

/// Whether a folder is the document portal's, or its top: what a flatpak
/// sees of folders it was given.
fn portal(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.starts_with("/run/user/") && s.contains("/doc")
}

/// Writes and removes a small file in a folder.
fn probe(dir: &Path) -> io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let f = dir.join(format!(".trip-{}-{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&f)?;
    let wrote = file.write_all(b"x").and_then(|_| file.sync_all());
    drop(file);
    let removed = std::fs::remove_file(&f);
    wrote.and(removed)
}

/// [`look`] on a thread of its own, given up after `timeout`.
pub async fn check_with(path: &Path, need: u64, timeout: Duration, free: FreeFn) -> Result<(), Unreachable> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(look(&p, need, free));
    });
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(r)) => r,
        _ => Err(Unreachable::NotAnswering { path: path.into() }),
    }
}

/// [`check_with`] with `statvfs` and the usual timeout.
pub async fn check(path: &Path, need: u64) -> Result<(), Unreachable> {
    check_with(path, need, TIMEOUT, statvfs_free).await
}

/// [`look`] from a thread that isn't async, given up after `timeout`.
pub fn check_now(path: &Path, need: u64, timeout: Duration, free: FreeFn) -> Result<(), Unreachable> {
    let (tx, rx) = std::sync::mpsc::channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(look(&p, need, free));
    });
    rx.recv_timeout(timeout).unwrap_or_else(|_| Err(Unreachable::NotAnswering { path: path.into() }))
}

// ---- how much room a disc needs --------------------------------------------------------------

/// A full CD's length, in seconds: what's assumed before the disc is read.
pub const CD_SECONDS: u64 = 80 * 60;
/// A CD's audio as WAV, a second of it.
pub const WAV_RATE: u64 = 176_400;
/// A full CD's image.
pub const CD_IMAGE: u64 = 700_000_000;
/// A single-layer DVD.
pub const DVD_BYTES: u64 = 4_700_000_000;

/// Roughly how many bytes a second of CD audio takes, encoded by a step
/// of this kind: FLAC about 60% of the WAV, an MP3 at its highest 320 kbps.
pub fn rate_of(kind: &str) -> Option<u64> {
    Some(match kind {
        "flac" => 110_000,
        "wav" => WAV_RATE,
        "mp3" => 40_000,
        "opus" | "vorbis" => 24_000,
        "aac" => 32_000,
        "alac" | "wavpack" => 110_000,
        _ => return None,
    })
}

/// About how much a Folder needs for one disc, from the kinds of step on
/// the way to it and what reaches it, with the disc's length when known.
pub fn need_for(kind: trip_engine::ItemKind, seen: &[&str], seconds: Option<u64>) -> u64 {
    use trip_engine::ItemKind as K;
    let audio = || {
        let rate = seen.iter().filter_map(|k| rate_of(k)).max().unwrap_or(rate_of("flac").unwrap_or(WAV_RATE));
        seconds.unwrap_or(CD_SECONDS) * rate
    };
    match kind {
        K::AudioFile => audio(),
        K::ImageFile => CD_IMAGE,
        K::VideoFile if seen.contains(&"transcode") => DVD_BYTES / 2,
        K::VideoFile => DVD_BYTES,
        K::TextFile | K::PictureFile => 50_000_000,
        _ if seen.contains(&"rip-video") => DVD_BYTES,
        _ if seen.iter().any(|k| rate_of(k).is_some()) => audio(),
        _ => CD_IMAGE,
    }
}

/// About how much the working folder needs for one disc: the rip itself
/// (a CD's WAVs, a disc's image, a DVD's titles) and what's made from it
/// before it's filed, from the kinds of step in the recipe.
pub fn work_need(kinds: &[&str], seconds: Option<u64>) -> u64 {
    let secs = seconds.unwrap_or(CD_SECONDS);
    let rip = if kinds.contains(&"rip-video") {
        DVD_BYTES
    } else if kinds.contains(&"rip") {
        secs * WAV_RATE
    } else {
        CD_IMAGE
    };
    let encodes: u64 = kinds.iter().filter_map(|k| rate_of(k)).map(|r| secs * r).sum();
    let made = if kinds.contains(&"chd") { CD_IMAGE } else { 0 } + if kinds.contains(&"transcode") { DVD_BYTES / 2 } else { 0 };
    rip + encodes + made
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plenty(_: &Path) -> io::Result<Option<u64>> {
        Ok(Some(1 << 40))
    }

    fn little(_: &Path) -> io::Result<Option<u64>> {
        Ok(Some(400_000_000))
    }

    fn unknown(_: &Path) -> io::Result<Option<u64>> {
        Ok(None)
    }

    fn slow(_: &Path) -> io::Result<Option<u64>> {
        std::thread::sleep(Duration::from_secs(5));
        Ok(Some(1 << 40))
    }

    fn root() -> bool {
        // SAFETY: no arguments, no side effects.
        unsafe { libc::geteuid() == 0 }
    }

    #[test]
    fn a_folder_that_is_there_and_writable_is_fine_and_left_as_it_was() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(look(t.path(), 1_000, plenty), Ok(()));
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 0, "the probe's file is gone");
    }

    #[test]
    fn a_missing_folder_is_fine_when_the_folder_above_can_make_it_and_isnt_made() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("Music/FLAC");
        assert_eq!(look(&p, 1_000, plenty), Ok(()));
        assert!(!t.path().join("Music").exists());
    }

    #[test]
    fn a_missing_folder_under_a_read_only_one_cant_be_made() {
        if root() {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let ro = t.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        set_mode(&ro, 0o555);
        let p = ro.join("FLAC");
        let r = look(&p, 1_000, plenty);
        set_mode(&ro, 0o755);
        assert_eq!(r, Err(Unreachable::CantWrite { path: p.clone(), missing: true }));
        assert!(r.unwrap_err().sentence().starts_with("Can't make the folder "));
    }

    #[test]
    fn a_read_only_folder_cant_be_written_to() {
        if root() {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let ro = t.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        set_mode(&ro, 0o555);
        let r = look(&ro, 1_000, plenty);
        set_mode(&ro, 0o755);
        assert_eq!(r, Err(Unreachable::CantWrite { path: ro.clone(), missing: false }));
        let r = r.unwrap_err();
        assert_eq!(r.trouble(), Trouble::NotAllowed);
        assert_eq!(r.title("ro"), "Not allowed to save in ro");
    }

    #[test]
    fn a_file_where_the_folder_should_be_is_said_so() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("FLAC");
        std::fs::write(&f, "").unwrap();
        assert_eq!(look(&f, 1, plenty), Err(Unreachable::NotAFolder { path: f.clone() }));
        assert_eq!(look(&f.join("x"), 1, plenty), Err(Unreachable::NotAFolder { path: f.join("x") }));
        // As a folder gone: not a matter of being allowed.
        assert_eq!(look(&f, 1, plenty).unwrap_err().title("FLAC"), "Can't reach FLAC");
    }

    #[test]
    fn a_full_folder_says_how_much_is_needed_and_free() {
        let t = tempfile::tempdir().unwrap();
        let r = look(t.path(), 2_100_000_000, little).unwrap_err();
        assert_eq!(r, Unreachable::NoRoom { path: t.path().into(), need: 2_100_000_000, free: 400_000_000 });
        assert_eq!(r.card("FLAC"), ("Not enough room in FLAC".into(), "2.1 GB needed, 0.4 GB free".into()));
        assert_eq!(r.said("FLAC"), "Not enough room in FLAC: 2.1 GB needed, 0.4 GB free");
        assert!(r.sentence().ends_with(": 2.1 GB needed, 0.4 GB free"), "{}", r.sentence());
        // Enough is fine; a filesystem that can't say isn't counted against.
        assert_eq!(look(t.path(), 300_000_000, little), Ok(()));
        assert_eq!(look(t.path(), u64::MAX, unknown), Ok(()));
    }

    #[test]
    fn a_folder_that_hangs_is_given_up_on() {
        let t = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let at = std::time::Instant::now();
        let r = rt.block_on(check_with(t.path(), 1, Duration::from_millis(200), slow));
        assert_eq!(r, Err(Unreachable::NotAnswering { path: t.path().into() }));
        assert!(at.elapsed() < Duration::from_secs(2), "{:?}", at.elapsed());
        // And from a thread that isn't async.
        let at = std::time::Instant::now();
        assert!(matches!(check_now(t.path(), 1, Duration::from_millis(200), slow), Err(Unreachable::NotAnswering { .. })));
        assert!(at.elapsed() < Duration::from_secs(2));
        // The runtime closes at once: the stuck look isn't its to wait for.
        let at = std::time::Instant::now();
        drop(rt);
        assert!(at.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_drive_that_isnt_there_is_said_to_be_unplugged() {
        assert_eq!(drive_of(Path::new("/run/media"), Path::new("/run/media/backup/Music")), Some(PathBuf::from("/run/media/backup")));
        assert_eq!(drive_of(Path::new("/run/media/alice"), Path::new("/run/media/alice/backup/Music")), Some(PathBuf::from("/run/media/alice/backup")));
        assert_eq!(drive_of(Path::new("/home/alice"), Path::new("/home/alice/Music")), None);
        // No drive since the computer started: no per-user folder either.
        if let Some(me) = std::env::var_os("USER").map(|u| u.to_string_lossy().into_owned()) {
            let want = PathBuf::from(format!("/run/media/{me}/backup"));
            assert_eq!(drive_of(Path::new("/run/media"), &want.join("Music")), Some(want));
        }
        let r = Unreachable::Unplugged { drive: "/run/media/backup".into() };
        assert_eq!(r.sentence(), "The drive /run/media/backup isn't plugged in");
    }

    #[test]
    fn paths_are_said_short() {
        assert_eq!(shown(Path::new("/run/user/1000/doc/1a2b3c4d/FLAC")), "/run/…/FLAC");
        assert_eq!(shown(Path::new("/mnt/nas/music")), "/mnt/nas/music");
        let r = Unreachable::CantReach { path: "/run/user/1000/gvfs/smb-share:server=nas,share=music/FLAC".into(), network: true };
        assert_eq!(r.sentence(), "Can't reach the network folder /run/…/FLAC");
        assert!(network(r.path()));
    }

    #[test]
    fn what_you_see_is_grouped_by_what_you_can_do_about_it() {
        let flac = PathBuf::from("/mnt/nas/FLAC");
        let smb = PathBuf::from("/run/user/1000/gvfs/smb-share:server=nas,share=music/FLAC");
        // Unplugged, the network down, not answering, and not makeable
        // under a share's empty mount point: one title.
        let away = [
            Unreachable::Unplugged { drive: "/run/media/alice/backup".into() },
            Unreachable::CantReach { path: smb.clone(), network: true },
            Unreachable::CantReach { path: flac.clone(), network: false },
            Unreachable::NotAnswering { path: flac.clone() },
            Unreachable::CantWrite { path: flac.clone(), missing: true },
        ];
        for why in &away {
            assert_eq!(why.trouble(), Trouble::OutOfReach, "{why:?}");
            assert_eq!(why.title(&name_of(&flac)), "Can't reach FLAC", "{why:?}");
        }
        // The log keeps them apart.
        let said: std::collections::HashSet<String> = away.iter().map(Unreachable::sentence).collect();
        assert_eq!(said.len(), away.len());
        // What to check, by where the folder is.
        assert_eq!(away[0].hint(), "Check the drive is plugged in.");
        assert_eq!(away[1].hint(), "Check the network folder is on and connected.");
        assert_eq!(Unreachable::NotAnswering { path: smb }.hint(), "Check the network folder is on and connected.");
        assert_eq!(Unreachable::NotAnswering { path: "/nowhere/music/FLAC".into() }.hint(), "Check the folder still exists.");
        assert_eq!(away[3].said("FLAC"), "Can't reach FLAC. Check the drive is plugged in.");
        assert_eq!(away[3].card("FLAC"), ("Can't reach FLAC".into(), "Check the drive is plugged in.".into()));
        // The rest, apart.
        assert_eq!(Unreachable::CantWrite { path: flac.clone(), missing: false }.trouble(), Trouble::NotAllowed);
        assert_eq!(Unreachable::NoRoom { path: flac, need: 1, free: 0 }.trouble(), Trouble::NoRoom);
    }

    #[test]
    fn a_disc_needs_room_for_what_reaches_the_folder() {
        use trip_engine::ItemKind as K;
        assert_eq!(need_for(K::AudioFile, &["rip", "flac"], Some(2_400)), 2_400 * 110_000);
        assert_eq!(need_for(K::AudioFile, &["rip", "mp3"], None), CD_SECONDS * 40_000);
        assert_eq!(need_for(K::ImageFile, &["image", "chd"], None), CD_IMAGE);
        assert!(gb(need_for(K::AudioFile, &["flac"], None)).ends_with("GB"));
        // The working folder: the WAVs, and the FLACs and MP3s made from them.
        assert_eq!(work_need(&["drive", "rip", "flac", "mp3", "folder"], Some(100)), 100 * (WAV_RATE + 110_000 + 40_000));
        assert_eq!(work_need(&["drive", "image", "chd", "folder"], None), 2 * CD_IMAGE);
    }

    fn set_mode(p: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}
