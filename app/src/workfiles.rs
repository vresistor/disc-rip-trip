//! The app clears its own working folders: each run gets one under the
//! cache's `runs/` (a CD's WAVs and encodes, a disc's image), and they're
//! big. The rules:
//!
//! - **Filed** (every Folder finished, nothing waiting): deleted when the
//!   run settles; if the canvas still offers a retry, when the next disc's
//!   run replaces it.
//! - **Raw** (stopped or failed before anything was encoded or finished):
//!   deleted once it's replaced, or at start-up once it's a day old.
//! - **Unfiled** (encoded or finished files that never reached a Folder,
//!   say the network share was down): never deleted by itself. It may be
//!   the only copy. *Working files* lists it, to open or delete.
//!
//! A folder an open or held run uses is never touched. Only folders right
//! under `runs/` are; the small run records beside them (`runs/*.json`)
//! are left. All of it runs off the window's thread.

use drt_engine::{ItemState, Snapshot, StepState};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// What a run settles to, written into its folder so a later start-up
/// knows without the run.
pub const MARK: &str = ".disc-rip-trip.json";

/// How old a raw folder must be before start-up deletes it.
pub const RAW_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Filed,
    Raw,
    Unfiled,
}

/// The mark: the class, and what to call the folder in the list.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mark {
    pub class: Class,
    #[serde(default)]
    pub title: String,
}

/// A working folder kept because it holds files never filed.
#[derive(Clone, Debug, PartialEq)]
pub struct Kept {
    pub path: PathBuf,
    pub bytes: u64,
    /// *Tortoise – TNT: 3 tracks, never filed*.
    pub line: String,
}

/// Every working folder, as last looked at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scan {
    pub total: u64,
    pub kept: Vec<Kept>,
}

/// Sub-folders that hold only what was read from the disc, or scratch:
/// never encoded or finished files.
fn raw_dir(name: &str) -> bool {
    matches!(name, "rip" | "image" | "scan" | "combined" | "artwork" | "replaygain") || name.strip_prefix("rip-").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
}

/// Whether a folder has encoded or finished files: a sub-folder that isn't
/// raw (an unknown one counts, to be safe) with a file in it.
pub fn has_finished(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        e.file_type().is_ok_and(|t| t.is_dir()) && !raw_dir(&name) && files_in(&e.path()) > 0
    })
}

fn files_in(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| if e.file_type().is_ok_and(|t| t.is_dir()) { files_in(&e.path()) } else { 1 }).sum()).unwrap_or(0)
}

/// How much a folder holds.
pub fn size_of(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn is_place(kind: &str) -> bool {
    matches!(kind, "folder" | "library")
}

/// Steps whose items are what was read from the disc, or what was looked
/// up or decided: none of them a finished file. An item anywhere else (an
/// encode, a checked image, something held) is.
fn reads_or_looks(kind: &str) -> bool {
    matches!(
        kind,
        "drive"
            | "next-disc"
            | "audio-cd"
            | "data-disc"
            | "game-disc"
            | "video-disc"
            | "rip"
            | "image"
            | "rip-video"
            | "musicbrainz"
            | "cd-text"
            | "discogs"
            | "gnudb"
            | "tmdb"
            | "redump"
            | "cover-art"
            | "lyrics"
            | "choose"
            | "choose-video"
            | "game-choose"
            | "tell"
            | "replaygain"
    )
}

/// Every Folder finished, nothing waiting or under way, and not stopped.
pub fn all_filed(snap: &Snapshot) -> bool {
    let places: Vec<_> = snap.steps.values().filter(|v| is_place(&v.step)).collect();
    snap.finished
        && !snap.stopped
        && snap.steps.values().all(|v| v.state != StepState::Busy && v.items.iter().all(|i| !matches!(i.state, ItemState::Waiting | ItemState::Busy)))
        && !places.is_empty()
        && places.iter().all(|v| v.state == StepState::Ok && v.count(ItemState::Skipped) == 0 && v.count(ItemState::Bad) == 0)
        && snap.steps.values().all(|v| v.count(ItemState::Waiting) == 0 && v.state != StepState::Hold)
        && snap.question.is_none()
}

/// Whether the canvas still offers to do something again: a step or an
/// item that went wrong.
pub fn offers_retry(snap: &Snapshot) -> bool {
    snap.steps.values().any(|v| v.state == StepState::Bad || v.count(ItemState::Bad) > 0)
}

/// What a settled run's folder is. `keep` is a step's own say that it
/// left something unfiled.
pub fn classify(snap: &Snapshot, keep: bool, dir: &Path) -> Class {
    if keep {
        return Class::Unfiled;
    }
    if all_filed(snap) {
        return Class::Filed;
    }
    // Something reached a Folder and wasn't filed: finished, whatever
    // the folder looks like.
    let made = snap.steps.values().any(|v| !reads_or_looks(&v.step) && !v.items.is_empty());
    if made || has_finished(dir) {
        Class::Unfiled
    } else {
        Class::Raw
    }
}

/// What a folder is, from its mark, else from what's in it.
pub fn class_of(dir: &Path) -> Class {
    match read_mark(dir) {
        Some(m) => m.class,
        None if has_finished(dir) => Class::Unfiled,
        None => Class::Raw,
    }
}

pub fn read_mark(dir: &Path) -> Option<Mark> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(MARK)).ok()?).ok()
}

/// Notes what a settled run's folder is. Only where the folder exists.
pub fn write_mark(dir: &Path, mark: &Mark) {
    if dir.is_dir() {
        if let Ok(text) = serde_json::to_string(mark) {
            let _ = std::fs::write(dir.join(MARK), text);
        }
    }
}

/// Deletes a working folder: only one right under `root`, and never one
/// in use.
pub fn delete(root: &Path, dir: &Path, in_use: &HashSet<PathBuf>) -> std::io::Result<bool> {
    if dir.parent() != Some(root) || in_use.contains(dir) || !dir.is_dir() {
        return Ok(false);
    }
    std::fs::remove_dir_all(dir)?;
    Ok(true)
}

/// A run that has gone from the window (the next disc replaced it): its
/// folder goes, unless it holds files never filed.
pub fn on_replaced(root: &Path, dir: &Path, in_use: &HashSet<PathBuf>) -> bool {
    match class_of(dir) {
        Class::Filed | Class::Raw => delete(root, dir, in_use).unwrap_or(false),
        Class::Unfiled => false,
    }
}

/// At start-up: folders marked filed go; raw ones a day old go; the rest
/// stay. Gives what was deleted.
pub fn sweep(root: &Path, in_use: &HashSet<PathBuf>, now: SystemTime) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root) else { return vec![] };
    let mut gone = Vec::new();
    for e in rd.flatten() {
        let dir = e.path();
        if !e.file_type().is_ok_and(|t| t.is_dir()) || in_use.contains(&dir) {
            continue;
        }
        let old = newest_change(&dir).and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age >= RAW_AGE);
        let go = match class_of(&dir) {
            Class::Filed => true,
            Class::Raw => old,
            Class::Unfiled => false,
        };
        if go && delete(root, &dir, in_use).unwrap_or(false) {
            gone.push(dir);
        }
    }
    gone
}

/// When anything in a folder last changed.
fn newest_change(p: &Path) -> Option<SystemTime> {
    let m = std::fs::symlink_metadata(p).ok()?;
    let own = m.modified().ok();
    if !m.is_dir() {
        return own;
    }
    std::fs::read_dir(p).ok()?.flatten().filter_map(|e| newest_change(&e.path())).chain(own).max()
}

/// Every working folder's size, and the ones kept because they hold files
/// never filed.
pub fn scan(root: &Path) -> Scan {
    let Ok(rd) = std::fs::read_dir(root) else { return Scan::default() };
    let mut out = Scan::default();
    let mut dirs: Vec<PathBuf> = rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
        let bytes = size_of(&dir);
        out.total += bytes;
        if class_of(&dir) == Class::Unfiled {
            // Unnamed (cut off before it knew the album): by when.
            let title = read_mark(&dir).map(|m| m.title).filter(|t| !t.is_empty()).unwrap_or_else(|| {
                let secs = newest_change(&dir).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                format!("A disc from {}", crate::tray::day_month(secs))
            });
            out.kept.push(Kept { line: format!("{title}: {}, never filed", what(&dir)), path: dir, bytes });
        }
    }
    out
}

/// What's waiting in a folder: *3 tracks*, *1 file*. Counted in its
/// fullest finished sub-folder.
fn what(dir: &Path) -> String {
    let audio = ["flac", "mp3", "opus", "aac", "vorbis", "alac", "wavpack", "wav"];
    let mut best: Option<(usize, bool)> = None;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !e.file_type().is_ok_and(|t| t.is_dir()) || raw_dir(&name) {
            continue;
        }
        let n = files_in(&e.path());
        if n > best.map_or(0, |b| b.0) {
            best = Some((n, audio.contains(&name.as_str())));
        }
    }
    match best {
        Some((n, true)) => format!("{n} track{}", if n == 1 { "" } else { "s" }),
        Some((n, false)) => format!("{n} file{}", if n == 1 { "" } else { "s" }),
        None => "files".into(),
    }
}

// ---- the worker -----------------------------------------------------------------------------

/// Something for the worker, done in the order asked: a run settling is
/// marked before its replacement is looked at.
pub enum Job {
    /// A run has ended: mark its folder, and delete it if it's filed and
    /// nothing is on offer to do again.
    Settled { dir: PathBuf, snap: Box<Snapshot>, keep: bool, title: String, in_use: HashSet<PathBuf> },
    /// The next disc replaced a run.
    Replaced { dir: PathBuf, in_use: HashSet<PathBuf> },
    /// Delete, as you asked.
    Delete { dir: PathBuf, in_use: HashSet<PathBuf> },
    /// The start-up sweep.
    Sweep { in_use: HashSet<PathBuf> },
    /// Look again.
    Scan,
}

/// A thread of its own that does the jobs one at a time, off the window's
/// thread, and sends what the folders come to after each.
pub fn worker(root: PathBuf, wake: std::sync::Arc<std::sync::OnceLock<egui::Context>>) -> (std::sync::mpsc::Sender<Job>, std::sync::mpsc::Receiver<Scan>) {
    let (jobs, rx) = std::sync::mpsc::channel::<Job>();
    let (tx, scans) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(job) = rx.recv() {
            match job {
                Job::Settled { dir, snap, keep, title, in_use } => {
                    let class = classify(&snap, keep, &dir);
                    write_mark(&dir, &Mark { class, title });
                    if class == Class::Filed && !offers_retry(&snap) {
                        if let Err(e) = delete(&root, &dir, &in_use) {
                            eprintln!("couldn't delete {}: {e}", dir.display());
                        }
                    }
                }
                Job::Replaced { dir, in_use } => {
                    on_replaced(&root, &dir, &in_use);
                }
                Job::Delete { dir, in_use } => {
                    if let Err(e) = delete(&root, &dir, &in_use) {
                        eprintln!("couldn't delete {}: {e}", dir.display());
                    }
                }
                Job::Sweep { in_use } => {
                    sweep(&root, &in_use, SystemTime::now());
                }
                Job::Scan => {}
            }
            let _ = tx.send(scan(&root));
            if let Some(c) = wake.get() {
                c.request_repaint();
            }
        }
    });
    (jobs, scans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use drt_engine::{ItemView, StepView};

    fn put(dir: &Path, rel: &str, bytes: usize) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; bytes]).unwrap();
    }

    fn age(dir: &Path, by: Duration) {
        let then = SystemTime::now() - by;
        fn walk(p: &Path, t: SystemTime) {
            if p.is_dir() {
                for e in std::fs::read_dir(p).unwrap().flatten() {
                    walk(&e.path(), t);
                }
            }
            let f = std::fs::File::open(p).unwrap();
            f.set_modified(t).unwrap();
        }
        walk(dir, then);
    }

    fn step(kind: &str, state: StepState, items: &[ItemState]) -> StepView {
        StepView {
            step: kind.into(),
            name: kind.into(),
            from: String::new(),
            state,
            status: String::new(),
            qualifier: String::new(),
            detail: String::new(),
            progress: None,
            now: None,
            items: items.iter().enumerate().map(|(k, s)| ItemView { id: format!("i{k}"), label: String::new(), state: *s, note: String::new(), verified: false, options: vec![] }).collect(),
            total: None,
            preds: vec![],
            hold: false,
            record: Default::default(),
        }
    }

    fn snap(steps: Vec<(&str, StepView)>) -> Snapshot {
        Snapshot { finished: true, steps: steps.into_iter().map(|(id, v)| (id.to_owned(), v)).collect(), ..Default::default() }
    }

    #[test]
    fn a_run_is_filed_when_every_folder_finished_and_nothing_waits() {
        let t = tempfile::tempdir().unwrap();
        let ok = snap(vec![("rip", step("rip", StepState::Ok, &[ItemState::Ok])), ("lib", step("folder", StepState::Ok, &[ItemState::Ok, ItemState::Ok]))]);
        assert_eq!(classify(&ok, false, t.path()), Class::Filed);
        assert!(!offers_retry(&ok));
        // A step's own say that it left something unfiled wins.
        assert_eq!(classify(&ok, true, t.path()), Class::Unfiled);
        // A track that failed but the rest filed: filed, with a retry on offer.
        let one_bad = snap(vec![("rip", step("rip", StepState::Bad, &[ItemState::Ok, ItemState::Bad])), ("lib", step("folder", StepState::Ok, &[ItemState::Ok]))]);
        assert_eq!(classify(&one_bad, false, t.path()), Class::Filed);
        assert!(offers_retry(&one_bad));
        // A Folder that waited (its share was down) and was stopped: unfiled.
        let waited = snap(vec![("lib", step("folder", StepState::Hold, &[ItemState::Waiting]))]);
        assert_eq!(classify(&waited, false, t.path()), Class::Unfiled);
        // Stopped with the Folder fine so far and an encode under way: not filed.
        let mid = Snapshot { stopped: true, ..snap(vec![("flac", step("flac", StepState::Busy, &[ItemState::Ok, ItemState::Busy])), ("lib", step("folder", StepState::Ok, &[ItemState::Ok]))]) };
        assert_eq!(classify(&mid, false, t.path()), Class::Unfiled);
        // A checked image held, then stopped: finished, so not raw, though
        // it's in image/.
        put(t.path(), "image/game.bin", 10);
        let held = Snapshot {
            stopped: true,
            ..snap(vec![("image", step("image", StepState::Ok, &[ItemState::Ok])), ("check", step("check", StepState::Ok, &[ItemState::Ok])), ("lib", step("folder", StepState::Idle, &[]))])
        };
        assert_eq!(classify(&held, false, t.path()), Class::Unfiled);
    }

    #[test]
    fn a_run_stopped_before_anything_was_encoded_is_raw_and_one_with_encodes_is_not() {
        let t = tempfile::tempdir().unwrap();
        let stopped = Snapshot { stopped: true, ..snap(vec![("rip", step("rip", StepState::Bad, &[ItemState::Ok])), ("lib", step("folder", StepState::Idle, &[]))]) };
        put(t.path(), "rip/01.wav", 10);
        put(t.path(), "tool-output.txt", 10);
        assert_eq!(classify(&stopped, false, t.path()), Class::Raw);
        put(t.path(), "flac/01.flac", 10);
        assert_eq!(classify(&stopped, false, t.path()), Class::Unfiled);
    }

    #[test]
    fn a_replaced_run_goes_if_filed_or_raw_and_never_if_it_holds_unfiled_files() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("runs");
        let none = HashSet::new();
        for (name, class) in [("filed", Class::Filed), ("raw", Class::Raw), ("unfiled", Class::Unfiled)] {
            let d = root.join(name);
            put(&d, "flac/01.flac", 10);
            write_mark(&d, &Mark { class, title: String::new() });
        }
        assert!(on_replaced(&root, &root.join("filed"), &none));
        assert!(on_replaced(&root, &root.join("raw"), &none));
        assert!(!on_replaced(&root, &root.join("unfiled"), &none));
        assert!(root.join("unfiled/flac/01.flac").exists());
        // Never one in use, never one outside runs/.
        let d = root.join("in-use");
        put(&d, "rip/01.wav", 10);
        assert!(!on_replaced(&root, &d, &HashSet::from([d.clone()])));
        assert!(d.exists());
        let outside = t.path().join("elsewhere");
        put(&outside, "rip/01.wav", 10);
        assert!(!on_replaced(&root, &outside, &none));
        assert!(outside.exists());
    }

    #[test]
    fn start_up_deletes_filed_and_day_old_raw_folders_and_keeps_unfiled_encodes_and_held_runs() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("runs");
        // Filed: goes, however new.
        let filed = root.join("cds-1");
        put(&filed, "flac/01.flac", 10);
        write_mark(&filed, &Mark { class: Class::Filed, title: String::new() });
        // Raw: goes once a day old.
        let raw_old = root.join("cds-2");
        put(&raw_old, "rip/01.wav", 10);
        age(&raw_old, Duration::from_secs(25 * 3600));
        let raw_new = root.join("cds-3");
        put(&raw_new, "rip/01.wav", 10);
        // Unfiled encodes, old and unmarked (the app was closed mid-run): kept.
        let unfiled = root.join("cds-4");
        put(&unfiled, "flac/01.flac", 10);
        put(&unfiled, "rip/01.wav", 10);
        age(&unfiled, Duration::from_secs(30 * 24 * 3600));
        // A held run's: kept, though it's old and raw.
        let held = root.join("cds-5");
        put(&held, "rip/01.wav", 10);
        age(&held, Duration::from_secs(25 * 3600));
        // The small run records beside them: kept.
        put(&root, "cd-abc.json", 10);
        let gone = sweep(&root, &HashSet::from([held.clone()]), SystemTime::now());
        assert_eq!(gone.len(), 2, "{gone:?}");
        assert!(!filed.exists() && !raw_old.exists());
        assert!(raw_new.exists() && unfiled.join("flac/01.flac").exists() && held.exists());
        assert!(root.join("cd-abc.json").exists());
    }

    #[test]
    fn working_files_lists_what_was_never_filed_with_its_size() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("runs");
        let d = root.join("cds-9");
        for n in 1..=3 {
            put(&d, &format!("flac/{n:02}.flac"), 100);
        }
        put(&d, "rip/01.wav", 1000);
        write_mark(&d, &Mark { class: Class::Unfiled, title: "Tortoise – TNT".into() });
        put(&root.join("cds-8"), "rip/01.wav", 50);
        let s = scan(&root);
        assert_eq!(s.kept.len(), 1);
        assert_eq!(s.kept[0].line, "Tortoise – TNT: 3 tracks, never filed");
        assert!(s.kept[0].bytes >= 1300);
        assert!(s.total >= 1350);
    }
}
