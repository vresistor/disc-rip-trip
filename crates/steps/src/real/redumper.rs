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
    /// The drive's read offset in samples, when the app knows it. redumper
    /// has its own list of drives; one not on it is read at +0. A data
    /// track comes out the same either way (redumper finds where its
    /// sectors start), but an audio-only disc doesn't, and the write
    /// offset redumper reports is only right with the right read offset.
    /// The same value for the dump and every re-read of it: redumper
    /// places what it reads by it.
    pub read_offset: Option<i64>,
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
    a.extend(o.offset_arg());
    a.extend(t.args());
    a
}

impl Options {
    fn offset_arg(&self) -> Option<String> {
        self.read_offset.map(|o| format!("--drive-read-offset={o}"))
    }
}

pub fn refine_args(o: &Options, t: &Target) -> Vec<String> {
    let mut a = vec!["refine".into(), format!("--drive={}", o.drive), format!("--retries={}", o.retries.max(1))];
    if let Some(s) = o.speed {
        a.push(format!("--speed={s}"));
    }
    a.extend(o.offset_arg());
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
    /// Each counter by its name, singular: `SCSI`, `C2`, `EDC`, `Q`.
    pub counts: Vec<(String, u64)>,
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
    let counts = counts_in(line);
    let errors = counts.iter().map(|c| c.1).sum();
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
    Some(Progress { fraction: (percent / 100.0).clamp(0.0, 1.0), detail, errors, counts })
}

/// The counters in a progress line's braces, each `name: count`; taking
/// every number instead counts the 2 in `C2s`. `SCSIs` is `SCSI`.
fn counts_in(line: &str) -> Vec<(String, u64)> {
    let Some(open) = line.find('{') else { return vec![] };
    let Some(close) = line[open..].find('}') else { return vec![] };
    line[open + 1..open + close]
        .split(',')
        .filter_map(|e| {
            let (name, n) = e.split_once(':')?;
            let name = name.trim();
            Some((name.strip_suffix('s').unwrap_or(name).to_owned(), n.trim().parse::<u64>().ok()?))
        })
        .collect()
}

/// What Image says of a dump while redumper works on it, from a progress
/// line: *43%, sector 1234567 of 2298496, 3 errors*.
pub fn progress_note(p: &Progress) -> String {
    let errors = match p.errors {
        0 => String::new(),
        1 => ", 1 error".into(),
        n => format!(", {n} errors"),
    };
    format!("{}%, {}{errors}", (p.fraction * 100.0) as u32, p.detail)
}

/// A progress line as redumper prints one: a CD's counters are `SCSIs`,
/// `C2s` and `Q`; a DVD's or a Blu-ray's `SCSI` and `EDC`. For the
/// simulated steps, which report as the real one does.
pub fn progress_line(at: u64, of: u64, cd: bool, scsi: u64, c2: u64) -> String {
    let pct = (at * 100).checked_div(of).unwrap_or(0);
    let errors = if cd { format!("SCSIs: {scsi}, C2s: {c2}, Q: 0") } else { format!("SCSI: {scsi}, EDC: {c2}") };
    format!("⠋ [{pct:>3}%] LBA: {at:>6}/{of}, errors: {{ {errors} }}")
}

/// The read errors a progress line or a log's block counts, in a few
/// words: *SCSI 2, C2 37*. The Q counter is the subchannel's, not the
/// data's: left out. Empty when there are none.
pub fn errors_text(counts: &[(String, u64)]) -> String {
    counts.iter().filter(|(k, n)| *n > 0 && k != "Q").map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
}

// ---- its passes, from the log ------------------------------------------------------------------

/// One of redumper's passes over a disc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassKind {
    /// `dump`: the disc read once, end to end.
    Read,
    /// `refine`: the sectors that came back with errors read again.
    Reread,
    /// `split`: a CD's tracks written out of what was read.
    Tracks,
    /// `info`: every sector of the image checked.
    Check,
    /// `protection`: copy protection looked for.
    Protection,
    /// `hash`: the image's checksums.
    Hash,
}

impl PassKind {
    fn of(heading: &str) -> Option<PassKind> {
        Some(match heading {
            "DUMP" => PassKind::Read,
            "REFINE" => PassKind::Reread,
            "SPLIT" => PassKind::Tracks,
            "INFO" => PassKind::Check,
            "PROTECTION" => PassKind::Protection,
            "HASH" => PassKind::Hash,
            _ => return None,
        })
    }

    /// Its name in a list of what was done.
    pub fn name(self) -> &'static str {
        match self {
            PassKind::Read => "Read the disc",
            PassKind::Reread => "Re-read bad parts",
            PassKind::Tracks => "Write the tracks",
            PassKind::Check => "Check every sector",
            PassKind::Protection => "Look for copy protection",
            PassKind::Hash => "Checksums",
        }
    }

    /// Whether it reads the drive, so has a read speed.
    pub fn reads(self) -> bool {
        matches!(self, PassKind::Read | PassKind::Reread)
    }
}

/// What one pass came to, from what it logged.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    pub kind: PassKind,
    /// The log line its heading is.
    pub at: usize,
    /// The drive's errors (`media errors:`), each counter by name: in
    /// samples for a CD.
    pub errors: Vec<(String, u64)>,
    /// What re-reading put right (`correction statistics:`).
    pub corrected: Vec<(String, u64)>,
    /// Sectors `split` had to fill in rather than read.
    pub skipped: u64,
    /// Sectors `info` found wrong: (kind, count).
    pub wrong: Vec<(&'static str, u64)>,
    /// What it stopped on, if it did.
    pub error: Option<String>,
}

/// The passes in a dump's log, in order, each with what it found.
pub fn passes<'a>(log: impl IntoIterator<Item = &'a str>) -> Vec<Pass> {
    #[derive(PartialEq)]
    enum Block {
        None,
        Errors,
        Corrected,
    }
    let mut out: Vec<Pass> = vec![];
    let mut block = Block::None;
    let mut info = String::new();
    for (k, line) in log.into_iter().enumerate() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("*** ") {
            // What `info` found, once its heading is closed (`*** END`).
            if let Some(last) = out.last_mut().filter(|p| p.kind == PassKind::Check && !info.is_empty()) {
                last.wrong = sector_problems(&info);
            }
            info.clear();
            block = Block::None;
            if let Some(kind) = h.split_whitespace().next().and_then(PassKind::of) {
                out.push(Pass { kind, at: k, errors: vec![], corrected: vec![], skipped: 0, wrong: vec![], error: None });
            }
            continue;
        }
        let Some(pass) = out.last_mut() else { continue };
        if t.starts_with("media errors:") {
            block = Block::Errors;
            continue;
        }
        if t.starts_with("correction statistics:") {
            block = Block::Corrected;
            continue;
        }
        let count = t.split_once(':').and_then(|(name, rest)| Some((name.trim().to_owned(), rest.split_whitespace().next()?.parse::<u64>().ok()?)));
        match (&block, count) {
            (Block::Errors, Some((name, n))) if !name.contains(' ') => pass.errors.push((name, n)),
            (Block::Corrected, Some((name, n))) if !name.contains(' ') => pass.corrected.push((name, n)),
            _ => {
                block = Block::None;
                if let Some(n) = parse_skipped(t) {
                    pass.skipped += n;
                } else if let Some(e) = t.strip_prefix("error:") {
                    pass.error = Some(e.trim().to_owned());
                } else if pass.kind == PassKind::Check {
                    info.push_str(t);
                    info.push('\n');
                }
            }
        }
    }
    if let Some(last) = out.last_mut().filter(|p| p.kind == PassKind::Check && !info.is_empty()) {
        last.wrong = sector_problems(&info);
    }
    out
}

/// The errors left after each pass: the read's, less what every re-read
/// since put right; as `errors_left` counts them.
pub fn left_after(passes: &[Pass], upto: usize) -> u64 {
    let data = |c: &[(String, u64)]| c.iter().filter(|(k, _)| k != "Q").map(|c| c.1).sum::<u64>();
    let mut left = 0u64;
    for p in passes.iter().take(upto + 1) {
        match p.kind {
            PassKind::Read => left = data(&p.errors),
            PassKind::Reread => left = left.saturating_sub(data(&p.corrected)),
            _ => {}
        }
    }
    left
}

/// The phase a heading starts: `*** DUMP (time check: 0s)`.
pub fn phase(line: &str) -> Option<&'static str> {
    let t = line.trim().strip_prefix("*** ")?;
    Some(match t.split_whitespace().next()? {
        "DUMP" => "Reading",
        "REFINE" => "Re-reading bad parts",
        "SPLIT" => "Writing tracks",
        "HASH" => "Checking",
        "INFO" => "Checking every sector",
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
    fn each_counter_on_a_progress_line_by_name() {
        let p = parse_progress("⠋ [ 88%] LBA:  99999/317787, errors: { SCSIs: 3, C2s: 12, Q: 0 }").unwrap();
        assert_eq!(p.counts, [("SCSI".to_owned(), 3), ("C2".to_owned(), 12), ("Q".to_owned(), 0)]);
        assert_eq!(errors_text(&p.counts), "SCSI 3, C2 12");
        assert_eq!(progress_note(&p), "88%, sector 99999 of 317787, 15 errors");
        let line = progress_line(1_234_567, 2_298_496, false, 0, 2);
        let p = parse_progress(&line).unwrap();
        assert_eq!((p.detail.as_str(), errors_text(&p.counts)), ("sector 1234567 of 2298496", "EDC 2".to_owned()));
        assert_eq!(parse_progress(&progress_line(12_345, 317_787, true, 1, 0)).unwrap().counts[0], ("SCSI".to_owned(), 1));
    }

    #[test]
    fn the_passes_in_a_log_and_what_each_found() {
        let log = format!("redumper dump --drive=/dev/sr0\n{COLONY_WARS}*** REFINE (time check: 0s)\ncorrection statistics: \n  SCSI: 588 samples\n  C2: 0 samples\n  Q: 0 sectors\n*** INFO (time check: 0s)\n  sectors count: 269931\n  EDC errors: 18 (LBA: 72759)\n");
        let p = passes(log.lines());
        let kinds: Vec<PassKind> = p.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [PassKind::Read, PassKind::Tracks, PassKind::Reread, PassKind::Check]);
        assert_eq!(errors_text(&p[0].errors), "SCSI 588, C2 12");
        assert_eq!((p[1].skipped, p[1].error.as_deref()), (1, Some("data errors detected, unable to continue")));
        assert_eq!(errors_text(&p[2].corrected), "SCSI 588");
        assert_eq!(p[3].wrong.iter().find(|w| w.0 == "EDC"), Some(&("EDC", 18)));
        assert_eq!((left_after(&p, 0), left_after(&p, 2)), (600, 12));
        assert_eq!(p[0].at, 4, "the heading's line");
        assert!(passes(["warning: no passes here"]).is_empty());
    }

    /// A PlayStation disc's log as Image keeps it (b744): each command's
    /// heading closed by `*** END`, which isn't a pass.
    #[test]
    fn a_real_log_s_passes() {
        let log = "*** DUMP (time check: 0s)\nwarning: unscrambled sector read\nmedia errors: \n  SCSI: 0 samples\n  C2: 0 samples\n  Q: 316\n*** END (time check: 376s)\n*** SPLIT (time check: 0s)\nwarning: shifting subchannel data (shift: +1)\n*** END (time check: 12s)\n*** INFO (time check: 0s)\nCD-ROM [KOUDELKA_1.bin]:\n  sectors count: 230571\n  MSF errors: 170 (LBA: 25378, 25997)\n  ECC errors: 15 (LBA: 112697)\n  EDC errors: 12 (LBA: 112697)\n  REDUMP.INFO errors: 15\n*** END (time check: 1s)\n";
        let p = passes(log.lines());
        assert_eq!(p.iter().map(|p| p.kind).collect::<Vec<_>>(), [PassKind::Read, PassKind::Tracks, PassKind::Check]);
        assert_eq!(errors_text(&p[0].errors), "", "Q is the subchannel's");
        assert_eq!((p[1].skipped, &p[1].error), (0, &None));
        assert_eq!(p[2].wrong, [("invalid sync", 0), ("MSF", 170), ("ECC", 15), ("EDC", 12)]);
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
        let o = Options { drive: "/dev/sr0".into(), retries: 20, speed: None, read_offset: None };
        assert_eq!(dump_args(&o, &t), ["dump", "--drive=/dev/sr0", "--retries=20", "--auto-detect", "--overwrite", "--image-path=/nonexistent/work/image", "--image-name=MY_DISC"]);
        // The drive's offset, when known, for the dump and its re-reads alike.
        let known = Options { read_offset: Some(-472), ..o.clone() };
        assert!(dump_args(&known, &t).contains(&"--drive-read-offset=-472".to_owned()));
        assert!(refine_args(&known, &t).contains(&"--drive-read-offset=-472".to_owned()));
        assert!(!dump_args(&o, &t).iter().any(|a| a.starts_with("--drive-read-offset")));
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
