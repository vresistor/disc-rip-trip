//! redumper: imaging a disc exactly, the way Redump's own dumps are made.
//!
//! The old app learned that `redumper disc`,
//! the aggregate that does everything, stops dead the moment `split` meets a
//! sector it can't read, and can't be resumed from there. So the phases are
//! run one at a time: `dump`, then `refine` if the drive reported errors,
//! then `split` for a CD. That also gives each of Image's retries a phase to
//! be: *Re-read bad parts* is `refine` and `split` again, *Keep it anyway* is `split
//! --force-split`.
//!
//! redumper takes a folder and a name and makes every file from them:
//! `NAME.scram`, `NAME.state`, `NAME.log`, `NAME.toc` and so on while it
//! works; then `NAME.cue` with `NAME.bin` (one track) or `NAME (Track
//! N).bin` (several) for a CD, and `NAME.iso` for a DVD or Blu-ray.

use super::tool;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// Where a dump's files are, and what they're called.
#[derive(Clone, Debug)]
pub struct Target {
    pub dir: PathBuf,
    pub name: String,
}

impl Target {
    pub fn file(&self, ext: &str) -> PathBuf {
        self.dir.join(format!("{}.{ext}", self.name))
    }

    /// The dump's main file: the cue sheet of a CD, the ISO of anything else.
    pub fn main(&self) -> Option<PathBuf> {
        ["cue", "iso"].into_iter().map(|e| self.file(e)).find(|p| p.is_file())
    }

    /// A CD is dumped scrambled first and split into tracks after; a DVD or
    /// Blu-ray is dumped straight to its ISO.
    pub fn is_cd(&self) -> bool {
        self.file("scram").is_file()
    }

    fn args(&self) -> [String; 2] {
        [format!("--image-path={}", self.dir.display()), format!("--image-name={}", self.name)]
    }
}

/// How a dump is read.
#[derive(Clone, Debug)]
pub struct Options {
    pub drive: String,
    /// Re-reads of a sector the drive reports bad.
    pub retries: u32,
    /// Read speed in redumper's units (×), or `None` for the drive's best.
    pub speed: Option<u32>,
}

pub fn dump_args(o: &Options, t: &Target) -> Vec<String> {
    let mut a = vec!["dump".into(), format!("--drive={}", o.drive), format!("--retries={}", o.retries)];
    // Generic drives have their sector order probed. Almost every drive is one.
    a.push("--auto-detect".into());
    // Only when there's nothing to resume: redumper refuses to start over a
    // `.state` without it, and a resumable dump shouldn't be read again.
    if !t.file("state").is_file() {
        a.push("--overwrite".into());
    }
    if let Some(s) = o.speed {
        a.push(format!("--speed={s}"));
    }
    a.extend(t.args());
    a
}

pub fn refine_args(o: &Options, t: &Target) -> Vec<String> {
    let mut a = vec!["refine".into(), format!("--drive={}", o.drive), format!("--retries={}", o.retries.max(1))];
    if let Some(s) = o.speed {
        a.push(format!("--speed={s}"));
    }
    a.extend(t.args());
    a
}

pub fn split_args(t: &Target, force: bool) -> Vec<String> {
    let mut a = vec!["split".into(), "--overwrite".into()];
    if force {
        a.push("--force-split".into());
    }
    a.extend(t.args());
    a
}

/// Runs one redumper command in the dump's folder. Gives what it printed
/// and whether it succeeded.
pub async fn run(args: &[String], t: &Target, stop: impl std::future::Future<Output = ()>, on_line: impl FnMut(&str)) -> Result<(String, bool)> {
    std::fs::create_dir_all(&t.dir)?;
    let mut cmd = tool::command("redumper");
    cmd.args(args);
    let (text, status) = tool::run(cmd, &t.dir, stop, on_line).await?;
    Ok((text, status.success()))
}

// ---- reading what it prints ----------------------------------------------------------------

/// How far along a phase is, from redumper's progress line.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// 0 to 1.
    pub fraction: f32,
    /// "sector 12345 of 317787", or what the line says ("splitting").
    pub detail: String,
    /// Read errors so far, summed over whatever counters the line has.
    pub errors: u64,
}

/// Reads a progress line. The formats (`cd/cd_dump.ixx`, `dvd/dvd_dump.ixx`):
///
/// ```text
/// ⠋ [ 42%] LBA:  12345/317787, errors: { SCSIs: 0, C2s: 0, Q: 0 }
/// ⠴ [ 7%] LBA: 1234567/2298496, errors: { SCSI: 0, EDC: 0 }
/// ⠹ [ 55%] splitting
/// ```
///
/// Read through a terminal, so the line may carry escape codes: the
/// percentage is found from its `%]` back to the `[` before it, rather than
/// from the first `[`, which may be an escape's.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let close = line.find("%]")?;
    let open = line[..close].rfind('[')?;
    let percent: f32 = line[open + 1..close].trim().parse().ok()?;
    let rest = line[close + 2..].trim();
    let errors = errors_in(line);
    let detail = match rest.strip_prefix("LBA:") {
        Some(lba) => {
            let pos = lba.split(',').next().unwrap_or("").trim();
            match pos.split_once('/') {
                Some((at, of)) => format!("sector {} of {}", at.trim(), of.trim()),
                None => format!("sector {pos}"),
            }
        }
        None => rest.to_owned(),
    };
    Some(Progress { fraction: (percent / 100.0).clamp(0.0, 1.0), detail, errors })
}

/// The sum of the counters in a progress line's braces. Each entry is
/// `name: count`; taking every number instead counts the 2 in `C2s`.
fn errors_in(line: &str) -> u64 {
    let Some(open) = line.find('{') else { return 0 };
    let Some(close) = line[open..].find('}') else { return 0 };
    line[open + 1..open + close].split(',').filter_map(|e| e.split(':').nth(1)?.trim().parse::<u64>().ok()).sum()
}

/// The phase a heading starts: `*** DUMP (time check: 0s)`.
pub fn phase(line: &str) -> Option<&'static str> {
    let t = line.trim().strip_prefix("*** ")?;
    Some(match t.split_whitespace().next()? {
        "DUMP" => "Reading",
        "REFINE" => "Re-reading bad parts",
        "SPLIT" => "Writing tracks",
        "HASH" => "Checking",
        "PROTECTION" => "Checking copy protection",
        _ => return None,
    })
}

/// Sector errors the drive reported and refining didn't correct: the
/// `media errors:` block of `dump`, less every `correction statistics:`
/// block of `refine` after it. The counts are in samples for a CD, and
/// what matters is only whether any are left.
pub fn errors_left(output: &str) -> u64 {
    #[derive(PartialEq)]
    enum Block {
        None,
        Errors,
        Corrected,
    }
    let (mut block, mut errors, mut corrected) = (Block::None, 0u64, 0u64);
    for line in output.lines() {
        let t = line.trim();
        if t.starts_with("media errors:") {
            block = Block::Errors;
            errors = 0;
            corrected = 0;
            continue;
        }
        if t.starts_with("correction statistics:") {
            block = Block::Corrected;
            continue;
        }
        let count = ["SCSI:", "C2:"].iter().find_map(|k| t.strip_prefix(k)).and_then(|r| r.split_whitespace().next()?.parse::<u64>().ok());
        match (count, &block) {
            (Some(n), Block::Errors) => errors += n,
            (Some(n), Block::Corrected) => corrected += n,
            // `Q:` belongs to the block; anything else ends it.
            (None, _) if t.starts_with("Q:") => {}
            (None, _) => block = Block::None,
            _ => {}
        }
    }
    errors.saturating_sub(corrected)
}

/// Whether a failed `split` is the kind `--force-split` gets past:
/// `error: data errors detected, unable to continue`.
pub fn split_needs_force(output: &str) -> bool {
    output.contains("data errors detected")
}

/// Sectors one `errors detected, track: 2, sectors: {SKIP: 1, C2: 0},
/// samples: {…}` line says were filled in rather than read. The samples
/// figure is the same thing in other units.
pub fn parse_skipped(line: &str) -> Option<u64> {
    let t = line.trim();
    if !t.starts_with("errors detected, track:") {
        return None;
    }
    let start = t.find("sectors: {")? + "sectors: {".len();
    let end = t[start..].find('}')? + start;
    Some(t[start..end].split(',').filter_map(|e| e.split(':').nth(1)?.trim().parse::<u64>().ok()).sum())
}

/// The lines worth keeping in the run's log: redumper's warnings, errors and
/// findings, without its progress.
pub fn worth_logging(line: &str) -> bool {
    let t = line.trim();
    ["warning:", "error:", "errors detected", "*** ", "media errors", "correction statistics", "SCSI:", "C2:", "Q:", "CD-ROM [", "sectors count:", "REDUMP.INFO"].iter().any(|p| t.starts_with(p))
        || PROBLEMS.iter().any(|(k, _)| t.starts_with(k))
}

/// What `redumper info` counts on each track of an image, by its name for
/// them and ours.
const PROBLEMS: &[(&str, &str)] = &[("invalid sync sectors:", "invalid sync"), ("MSF errors:", "MSF"), ("ECC errors:", "ECC"), ("EDC errors:", "EDC")];

/// Sectors `redumper info` found wrong in an image, summed over its tracks:
/// sectors the drive handed back wrong without reporting an error.
pub fn sector_problems(info: &str) -> Vec<(&'static str, u64)> {
    PROBLEMS
        .iter()
        .map(|(key, name)| {
            let n = info.lines().filter_map(|l| l.trim().strip_prefix(key)?.split_whitespace().next()?.parse::<u64>().ok()).sum();
            (*name, n)
        })
        .collect()
}

/// The last thing redumper said that explains a failure.
pub fn reason(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .rfind(|l| l.starts_with("error:") || l.starts_with("warning:"))
        .or_else(|| tool::last_meaningful_line(output))
        .unwrap_or("redumper failed")
        .trim_start_matches("error:")
        .trim()
        .to_owned()
}

// ---- can the drive be opened ------------------------------------------------------------------

/// The device to hand redumper, or why it can't be opened. redumper sends
/// raw SCSI commands; `/dev/srN` has the desktop's access rule and works
/// with them, `/dev/sgN` usually doesn't, and a permission error from
/// redumper reads like a broken drive. So the old app's pre-flight check is
/// kept: a drive that can't be opened says which group to join.
pub fn drive_access(device: &str) -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let open = |d: &str| std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(d).map(|_| ());
        let generic = || -> Option<String> {
            let name = device.strip_prefix("/dev/")?;
            let node = std::fs::read_dir(format!("/sys/block/{name}/device/scsi_generic")).ok()?.flatten().next()?.file_name().into_string().ok()?;
            Some(format!("/dev/{node}"))
        };
        let mut denied = None;
        for d in std::iter::once(device.to_owned()).chain(generic()) {
            match open(&d) {
                Ok(()) => return Ok(d),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    denied.get_or_insert(d);
                }
                Err(_) => {}
            }
        }
        match denied {
            Some(d) => {
                let gid = std::fs::metadata(&d).map(|m| m.gid()).ok();
                let groups = std::fs::read_to_string("/etc/group").unwrap_or_default();
                let group = gid.and_then(|g| group_name(&groups, g)).unwrap_or_else(|| "disk".into());
                Err(anyhow!("can't open {d}: join the {group} group (sudo usermod -aG {group} $USER) and log in again"))
            }
            None => Err(anyhow!("no drive at {device}")),
        }
    }
    #[cfg(not(target_os = "linux"))]
    Ok(device.to_owned())
}

/// A gid's name, from `/etc/group`'s contents: `disk` on most systems,
/// `optical` on Arch.
pub fn group_name(groups: &str, gid: u32) -> Option<String> {
    groups.lines().find_map(|l| {
        let mut f = l.split(':');
        let name = f.next()?;
        (f.nth(1)?.parse::<u32>().ok()? == gid).then(|| name.to_owned())
    })
}

/// A name redumper can use for the files, from the volume label: safe on
/// every filesystem, and never empty.
pub fn image_name(label: &str) -> String {
    let s = super::safe(label);
    if s.is_empty() {
        "disc".into()
    } else {
        s
    }
}

/// The files that make up a dump, from its main file: a cue sheet and the
/// tracks it names, or an ISO; with redumper's log beside it when there is
/// one. Its working files (`.scram`, `.state`, …) are not part of it.
pub fn dump_files(main: &Path) -> Vec<PathBuf> {
    let dir = main.parent().unwrap_or(Path::new("."));
    let mut files = vec![main.to_path_buf()];
    if main.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue")) {
        let cue = std::fs::read_to_string(main).unwrap_or_default();
        files.extend(cue_files(&cue).into_iter().map(|f| dir.join(f)));
    }
    if let Some(stem) = main.file_stem() {
        let log = dir.join(format!("{}.log", stem.to_string_lossy()));
        if log.is_file() {
            files.push(log);
        }
    }
    files
}

/// The files a cue sheet names, in order: `FILE "NAME (Track 1).bin" BINARY`.
pub fn cue_files(cue: &str) -> Vec<String> {
    cue.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("FILE ")?;
            let name = match rest.strip_prefix('"') {
                Some(q) => q.split('"').next()?,
                None => rest.rsplit_once(' ').map_or(rest, |(n, _)| n),
            };
            // Only a name beside the sheet, never a path elsewhere.
            (!name.is_empty() && !name.contains(['/', '\\']) && name != "..").then(|| name.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CD_LINE: &str = "⠋ [ 42%] LBA:  12345/317787, errors: { SCSIs: 0, C2s: 0, Q: 0 }";
    const DVD_LINE: &str = "⠴ [ 7%] LBA: 1234567/2298496, errors: { SCSI: 0, EDC: 0 }";

    #[test]
    fn progress_lines_from_a_cd_and_a_dvd() {
        let p = parse_progress(CD_LINE).unwrap();
        assert!((p.fraction - 0.42).abs() < 1e-4);
        assert_eq!(p.detail, "sector 12345 of 317787");
        assert_eq!(p.errors, 0, "the 2 in C2s is not an error");
        let p = parse_progress(DVD_LINE).unwrap();
        assert!((p.fraction - 0.07).abs() < 1e-4);
        assert_eq!(p.detail, "sector 1234567 of 2298496");
        let p = parse_progress("⠋ [ 88%] LBA:  99999/317787, errors: { SCSIs: 3, C2s: 12, Q: 0 }").unwrap();
        assert_eq!(p.errors, 15);
        assert_eq!(parse_progress("⠹ [ 55%] splitting").unwrap().detail, "splitting");
    }

    #[test]
    fn escape_codes_from_the_terminal_dont_hide_the_percentage() {
        let p = parse_progress("\x1b[2K\x1b[1G⠋ [100%] LBA: 317787/317787, errors: { SCSIs: 0, C2s: 0, Q: 0 }").unwrap();
        assert_eq!(p.fraction, 1.0);
        assert_eq!(p.detail, "sector 317787 of 317787");
    }

    #[test]
    fn lines_that_are_not_progress_are_left_alone() {
        assert_eq!(parse_progress("*** DUMP (time check: 0s)"), None);
        assert_eq!(parse_progress("error: drive not ready"), None);
        assert_eq!(parse_progress(""), None);
        assert_eq!(phase("*** DUMP (time check: 0s)"), Some("Reading"));
        assert_eq!(phase("*** REFINE (time check: 348s)"), Some("Re-reading bad parts"));
        assert_eq!(phase("dump"), None);
    }

    /// The Colony Wars (Europe) (Disc 1) log, from the old app: a drive that
    /// couldn't reach the last sector before the lead-out.
    const COLONY_WARS: &str = "\
redumper (build: b744)
warning: drive read offset not found in the database
warning: using generic drive
*** DUMP (time check: 348s)
media errors:
  SCSI: 588 samples
  C2: 12 samples
  Q: 2641

*** SPLIT (time check: 0s)
checking tracks
errors detected, track: 2, sectors: {SKIP: 1, C2: 0}, samples: {SKIP: 588, C2: 0}
error: data errors detected, unable to continue
";

    #[test]
    fn errors_left_after_dumping_and_refining() {
        assert_eq!(errors_left(COLONY_WARS), 600);
        let refined = format!("{COLONY_WARS}\n*** REFINE (time check: 0s)\ncorrection statistics: \n  SCSI: 588 samples\n  C2: 0 samples\n  Q: 0 sectors\n");
        assert_eq!(errors_left(&refined), 12, "refine corrected the SCSI ones");
        assert_eq!(errors_left("*** DUMP (time check: 0s)\nmedia errors: \n  SCSI: 0 samples\n  C2: 0 samples\n  Q: 276\n"), 0);
        assert_eq!(errors_left(""), 0);
    }

    #[test]
    fn a_split_that_meets_bad_sectors_says_how_many() {
        assert!(split_needs_force(COLONY_WARS));
        assert!(!split_needs_force("*** SPLIT (time check: 0s)\ndone\n"));
        let skipped: u64 = COLONY_WARS.lines().filter_map(parse_skipped).sum();
        assert_eq!(skipped, 1, "sectors, not samples");
        assert_eq!(reason(COLONY_WARS), "data errors detected, unable to continue");
    }

    #[test]
    fn info_says_which_sectors_came_back_wrong() {
        // From the old app's Silent Hill (Europe) log, and from `redumper
        // info` (b744) run here on a track of noise.
        let silent_hill = "CD-ROM [Silent Hill (Europe) (En,Fr,De,Es,It).bin]:\n  sectors count: 269931\n  MSF errors: 70 (LBA: 72759, 72866)\n  ECC errors: 6 (LBA: 194704, 198080)\n  EDC errors: 18 (LBA: 72759, 72866)\n\n  REDUMP.INFO errors: 19\n";
        assert_eq!(sector_problems(silent_hill), [("invalid sync", 0), ("MSF", 70), ("ECC", 6), ("EDC", 18)]);
        let noise = "*** INFO (time check: 0s)\n\nCD-ROM [D (Track 1).bin]:\n  sectors count: 300\n  invalid sync sectors: 300 (LBA: 0-299)\n\n  REDUMP.INFO errors: 0\n";
        assert_eq!(sector_problems(noise)[0], ("invalid sync", 300));
        assert!(worth_logging("  EDC errors: 18 (LBA: 72759, 72866)") && worth_logging("  SCSI: 588 samples"));
        assert!(!worth_logging("⠋ [ 42%] LBA:  12345/317787, errors: { SCSIs: 0, C2s: 0, Q: 0 }"));
    }

    #[test]
    fn the_commands_for_each_phase() {
        let t = Target { dir: PathBuf::from("/nonexistent/work/image"), name: "MY_DISC".into() };
        let o = Options { drive: "/dev/sr0".into(), retries: 20, speed: None };
        assert_eq!(dump_args(&o, &t), ["dump", "--drive=/dev/sr0", "--retries=20", "--auto-detect", "--overwrite", "--image-path=/nonexistent/work/image", "--image-name=MY_DISC"]);
        let o = Options { speed: Some(8), retries: 0, ..o };
        assert!(refine_args(&o, &t).contains(&"--retries=1".to_owned()), "refine with no retries does nothing");
        assert!(refine_args(&o, &t).contains(&"--speed=8".to_owned()));
        assert_eq!(split_args(&t, true)[..3], ["split", "--overwrite", "--force-split"]);
    }

    #[test]
    fn a_dumps_files_are_its_cue_its_tracks_and_its_log() {
        let dir = std::env::current_dir().unwrap().join("../../target/drt-test-dumpfiles");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("GAME.cue"), "FILE \"GAME (Track 1).bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\nFILE \"GAME (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n").unwrap();
        std::fs::write(dir.join("GAME.log"), "").unwrap();
        let files: Vec<String> = dump_files(&dir.join("GAME.cue")).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(files, ["GAME.cue", "GAME (Track 1).bin", "GAME (Track 2).bin", "GAME.log"]);
        assert_eq!(cue_files("FILE track01.bin BINARY\n"), ["track01.bin"], "unquoted names too");
        let hostile = "FILE \"../../.ssh/id_ed25519\" BINARY\nFILE \"/etc/shadow\" BINARY\nFILE \"..\" BINARY\nFILE \"a\\\\b.bin\" BINARY\nFILE \"ok.bin\" BINARY\n";
        assert_eq!(cue_files(hostile), ["ok.bin"], "nothing outside the sheet's folder");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn names_and_groups() {
        assert_eq!(image_name(""), "disc");
        assert_eq!(image_name("WIN/XP: SP2"), "WIN_XP_ SP2");
        assert_eq!(group_name("root:x:0:\ndisk:x:6:\ncdrom:x:11:\n", 6).as_deref(), Some("disk"));
        assert_eq!(group_name("optical:x:93:\n", 93).as_deref(), Some("optical"));
    }

    /// Runs the real redumper (`DRT_REDUMPER`) through `run`, as Image does,
    /// on a file made here, and checks its `hash` against ours.
    /// `DRT_REDUMPER="flatpak run --die-with-parent --command=redumper io.github.vresistor.DiscRipTrip" cargo test -p drt-steps real_redumper -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn real_redumper() {
        let dir = PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/disc-rip-trip/test-redumper");
        std::fs::create_dir_all(&dir).unwrap();
        let t = Target { dir: dir.clone(), name: "probe".into() };
        std::fs::write(t.file("iso"), (0..512 * 2048).map(|i| (i % 249) as u8).collect::<Vec<u8>>()).unwrap();
        let mut lines = 0;
        let args: Vec<String> = ["hash".to_owned()].into_iter().chain(t.args()).collect();
        let (out, ok) = run(&args, &t, std::future::pending(), |_| lines += 1).await.unwrap();
        assert!(ok && lines > 0, "{out}");
        let ours = super::super::hash::hash_files(&[&t.file("iso")], |_| {}, || false).unwrap().remove(0);
        let line = out.lines().find(|l| l.trim().starts_with("<rom ")).unwrap();
        assert!(line.contains(&ours.sha1) && line.contains(&ours.md5) && line.contains(&ours.crc), "{line} vs {ours:?}");
        assert_eq!(phase("*** HASH (time check: 0s)"), Some("Checking"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
