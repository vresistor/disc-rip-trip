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
    /// The order the drive hands over each sector's data, subchannel and
    /// C2 in ([`ORDERS`]), when it was found by testing or chosen: for a
    /// drive redumper doesn't know, whose own guess can be wrong and then
    /// shows errors on every sector. `None`: redumper's own.
    pub sector_order: Option<String>,
}

pub fn dump_args(o: &Options, t: &Target) -> Vec<String> {
    let mut a = vec!["dump".into(), format!("--drive={}", o.drive), format!("--retries={}", o.retries)];
    // Generic drives have their sector order probed, unless it's known.
    // Almost every drive is one.
    if o.sector_order.is_none() {
        a.push("--auto-detect".into());
    }
    // Only when there's nothing to resume: redumper refuses to start over a
    // `.state` without it, and a resumable dump shouldn't be read again.
    if !t.file("state").is_file() {
        a.push("--overwrite".into());
    }
    if let Some(s) = o.speed {
        a.push(format!("--speed={s}"));
    }
    a.extend(o.offset_arg());
    a.extend(o.order_arg());
    a.extend(t.args());
    a
}

impl Options {
    fn offset_arg(&self) -> Option<String> {
        self.read_offset.map(|o| format!("--drive-read-offset={o}"))
    }

    fn order_arg(&self) -> Option<String> {
        self.sector_order.as_ref().map(|o| format!("--drive-sector-order={o}"))
    }
}

pub fn refine_args(o: &Options, t: &Target) -> Vec<String> {
    let mut a = vec!["refine".into(), format!("--drive={}", o.drive), format!("--retries={}", o.retries.max(1))];
    if let Some(s) = o.speed {
        a.push(format!("--speed={s}"));
    }
    a.extend(o.offset_arg());
    a.extend(o.order_arg());
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

// ---- finding a drive's sector order ----------------------------------------------------------

/// The orders a drive can hand over a sector's parts in, redumper's names.
pub const ORDERS: [&str; 4] = ["DATA_C2_SUB", "DATA_SUB_C2", "DATA_SUB", "DATA_C2"];

/// The orders tried, best first: with C2 (which says which samples the
/// drive couldn't read) before without. Without subchannel redumper can't
/// place what it reads, so DATA_C2 is never tried.
pub const TRY_ORDERS: [&str; 3] = ["DATA_SUB_C2", "DATA_C2_SUB", "DATA_SUB"];

/// The stretches of a disc tried, in turn: the first sectors, and further
/// in when none of the orders read those cleanly (a mark at the start).
pub const PROBE_RANGES: [(u64, u64); 2] = [(0, 300), (5000, 5300)];

/// An order in words: *data, then subchannel, then C2*.
pub fn order_words(order: &str) -> String {
    match order {
        "DATA_C2_SUB" => "data, then C2, then subchannel",
        "DATA_SUB_C2" => "data, then subchannel, then C2",
        "DATA_SUB" => "data, then subchannel, with no C2",
        "DATA_C2" => "data, then C2, with no subchannel",
        o => o,
    }
    .to_owned()
}

/// What the log and Rip say of the order used: *This drive hands over
/// data, then subchannel, then C2 (found by testing)*.
pub fn order_line(order: &str, from: &str) -> String {
    format!("{ORDER_LINE} {} ({from})", order_words(order))
}

/// How [`order_line`] starts, to find it in a log.
pub const ORDER_LINE: &str = "This drive hands over";

/// A short read for finding the order: `range` with no retries, in the
/// `order` given, or redumper's own (`None`).
pub fn probe_args(drive: &str, order: Option<&str>, range: (u64, u64), t: &Target) -> Vec<String> {
    let mut a = vec!["dump".into(), format!("--drive={drive}"), "--retries=0".into()];
    match order {
        Some(o) => a.push(format!("--drive-sector-order={o}")),
        None => a.push("--auto-detect".into()),
    }
    a.extend([format!("--lba-start={}", range.0), format!("--lba-end={}", range.1), "--overwrite".into()]);
    a.extend(t.args());
    a
}

/// Whether redumper said it doesn't know the drive, and so guessed.
pub fn unknown_drive(output: &str) -> bool {
    output.contains("drive not found in the database")
}

/// The order redumper used, from its `configuration:` line.
pub fn order_used(output: &str) -> Option<String> {
    let l = output.lines().map(str::trim).find(|l| l.starts_with("configuration:"))?;
    let at = l.find("sector order:")? + "sector order:".len();
    let o = l[at..].split([',', ')']).next()?.trim();
    ORDERS.contains(&o).then(|| o.to_owned())
}

/// A short read's errors by name (`SCSI`, `C2`, `Q`): its summary's, else
/// its last progress line's. `None` when it shows neither.
pub fn probe_errors(output: &str) -> Option<Vec<(String, u64)>> {
    if let Some(at) = output.find("media errors:") {
        let counts: Vec<(String, u64)> = output[at..]
            .lines()
            .skip(1)
            .map_while(|l| {
                let (name, n) = l.trim().split_once(':')?;
                Some((name.trim().to_owned(), n.split_whitespace().next()?.parse().ok()?))
            })
            .collect();
        if !counts.is_empty() {
            return Some(counts);
        }
    }
    output.lines().rev().find_map(parse_progress).map(|p| p.counts).filter(|c| !c.is_empty())
}

/// Whether a short read in an order read cleanly: it ran, and shows no
/// error of any kind. A wrong order shows Q (subchannel) or C2 errors on
/// nearly every sector.
pub fn probe_clean(output: &str, ok: bool) -> bool {
    ok && probe_errors(output).is_some_and(|c| c.iter().all(|(_, n)| *n == 0))
}

/// What finding the order came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// redumper knows the drive, and its order: nothing to find.
    Known,
    /// The order that read cleanly.
    Order(String),
    /// None did: redumper's guess it is.
    Nothing,
}

/// Finds the order a drive redumper doesn't know hands over sectors in, by
/// short reads through `read` (`order`, `range`; giving what redumper
/// printed and whether it succeeded): first in redumper's own, which
/// says whether it knows the drive; then each of [`TRY_ORDERS`] in turn,
/// over each of [`PROBE_RANGES`] until one reads cleanly. One with C2 is
/// taken as soon as it does; DATA_SUB only when neither with C2 does on
/// any stretch. Each read is told to `note`, for the log.
pub async fn find_order<F, Fut>(mut read: F, mut note: impl FnMut(String)) -> Result<Found>
where
    F: FnMut(Option<String>, (u64, u64)) -> Fut,
    Fut: std::future::Future<Output = Result<(String, bool)>>,
{
    let (first, ok) = read(None, PROBE_RANGES[0]).await?;
    if !unknown_drive(&first) {
        return Ok(Found::Known);
    }
    let guess = order_used(&first);
    let (a, b) = PROBE_RANGES[0];
    note(format!("test read of sectors {a}–{b} as redumper guessed ({}): {}", guess.as_deref().unwrap_or("its own order"), probe_summary(&first, ok)));
    let mut without_c2 = None;
    for range in PROBE_RANGES {
        for order in TRY_ORDERS {
            let clean = match (&guess, range == PROBE_RANGES[0]) {
                (Some(g), true) if g == order => probe_clean(&first, ok),
                _ => {
                    let (out, ok) = read(Some(order.to_owned()), range).await?;
                    let clean = probe_clean(&out, ok);
                    note(format!("test read of sectors {}–{} as {order}: {}", range.0, range.1, probe_summary(&out, ok)));
                    clean
                }
            };
            if clean && order == "DATA_SUB" {
                without_c2.get_or_insert(order);
            } else if clean {
                return Ok(Found::Order(order.to_owned()));
            }
        }
        if without_c2.is_some() {
            break;
        }
    }
    Ok(without_c2.map(|o| Found::Order(o.to_owned())).unwrap_or(Found::Nothing))
}

/// A short read's errors for the log: *SCSI 0, C2 6140, Q 299*.
fn probe_summary(out: &str, ok: bool) -> String {
    match probe_errors(out) {
        Some(c) => c.iter().map(|(n, k)| format!("{n} {k}")).collect::<Vec<_>>().join(", ") + if ok { "" } else { ", and redumper stopped" },
        None if ok => "no error counts".into(),
        None => "redumper stopped".into(),
    }
}

/// Runs one redumper command in the dump's folder. Gives what it printed
/// and whether it succeeded. One killed by a signal (from outside: none
/// of ours) has a line saying so added to what it printed
/// ([`UNEXPECTED`]), so whatever reads it next can't take it for an
/// ordinary end.
pub async fn run(args: &[String], t: &Target, stop: impl std::future::Future<Output = ()>, on_line: impl FnMut(&str)) -> Result<(String, bool)> {
    std::fs::create_dir_all(&t.dir)?;
    let mut cmd = tool::command("redumper");
    cmd.args(args);
    let (mut text, status) = tool::run(cmd, &t.dir, stop, on_line).await?;
    if let Some(sig) = std::os::unix::process::ExitStatusExt::signal(&status) {
        text.push_str(&format!("{UNEXPECTED}: {}\n", signal_words(sig)));
    }
    Ok((text, status.success()))
}

/// How a note starts when redumper ended other than by finishing.
pub const UNEXPECTED: &str = "redumper stopped unexpectedly";

/// A signal in words: *killed by signal 9 (SIGKILL)*.
pub fn signal_words(sig: i32) -> String {
    let name = match sig {
        1 => " (SIGHUP)",
        2 => " (SIGINT)",
        6 => " (SIGABRT)",
        9 => " (SIGKILL)",
        11 => " (SIGSEGV)",
        15 => " (SIGTERM)",
        _ => "",
    };
    format!("killed by signal {sig}{name}")
}

/// What a stage that didn't finish comes to, from what it printed: *redumper
/// stopped unexpectedly: killed by signal 9 (SIGKILL)*, or with the last
/// thing it said.
pub fn unexpected_note(text: &str) -> String {
    match text.lines().rev().find(|l| l.starts_with(UNEXPECTED)) {
        Some(l) => l.trim().to_owned(),
        None => format!("{UNEXPECTED}: {}", reason(text)),
    }
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
    /// The sector it's at and the disc's sectors, when the line has them.
    pub at: Option<(u64, u64)>,
    /// A re-read's try on the sector it's at: `retry: 19`. `None` on the
    /// first read of it, and on every line of a first read of the disc.
    pub retry: Option<u32>,
}

/// Reads a progress line. The formats (`cd/cd_dump.ixx`, `dvd/dvd_dump.ixx`):
///
/// ```text
/// ⠋ [ 42%] LBA:  12345/317787, errors: { SCSIs: 0, C2s: 0, Q: 0 }
/// ⠴ [ 7%] LBA: 1234567/2298496, errors: { SCSI: 0, EDC: 0 }
/// ⠹ [ 55%] splitting
/// ⠋ [ 54%] LBA: 147223/269159, errors: { SCSIs: 455112, C2s: 10553, Q: 2241 }, retry: 19, data:  97%
/// ```
///
/// The last is `refine`'s: one line for each read of each sector still
/// bad (and none for the good ones, which it passes over), the first read
/// of a sector without `retry:`. Its counters start at what's still bad,
/// in samples for a CD (588 to a sector), and go down as sectors come
/// back.
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
    let at = rest.strip_prefix("LBA:").and_then(|lba| {
        let (a, b) = lba.split_once('/')?;
        let of: String = b.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
        Some((a.trim().parse().ok()?, of.parse().ok()?))
    });
    let retry = line.find("retry:").and_then(|k| line[k + 6..].trim_start().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok());
    // From the sector it's at where the line says, not redumper's whole
    // percent: in a bad patch one percent is thousands of sectors and
    // minutes, and a whole percent sits still that long.
    let fraction = match at {
        Some((a, of)) if of > 0 => (a as f32 / of as f32).clamp(0.0, 1.0),
        _ => (percent / 100.0).clamp(0.0, 1.0),
    };
    Some(Progress { fraction, detail, errors, counts, at, retry })
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
                // Those it couldn't read at all, as Rip counts them.
                if let Some((n, _)) = split_errors(t) {
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

/// The sectors one `errors detected, track: 1, sectors: {SKIP: 774, C2:
/// 120}, samples: {…}` line counts: those the drive couldn't read at all,
/// and those it read with C2 errors. A sector may be in both (some of its
/// samples read, with errors), so the two aren't added up.
pub fn split_errors(line: &str) -> Option<(u64, u64)> {
    let t = line.trim();
    if !t.starts_with("errors detected, track:") {
        return None;
    }
    let start = t.find("sectors: {")? + "sectors: {".len();
    let end = t[start..].find('}')? + start;
    let get = |k: &str| t[start..end].split(',').find_map(|e| e.trim().strip_prefix(k)?.trim().parse::<u64>().ok()).unwrap_or(0);
    Some((get("SKIP:"), get("C2:")))
}

/// What Rip says when it couldn't read some sectors, after how many.
pub const UNREADABLE_ADVICE: &str =
    "Clean the disc and choose Re-read bad parts, or try another drive. Or choose Make the image anyway: the sectors it can't read are left blank, so Check and fix can try them again.";

/// The failure in a line: *Couldn't read 774 sectors, and 120 more came
/// back with errors*.
pub fn unreadable_line(unread: u64, c2: u64) -> String {
    let s = |n: u64| if n == 1 { "" } else { "s" };
    match c2 {
        0 => format!("Couldn't read {} sector{}", thousands(unread), s(unread)),
        c => format!("Couldn't read {} sector{}, and {} more came back with errors", thousands(unread), s(unread), thousands(c)),
    }
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

// ---- where on the disc the errors are ---------------------------------------------------------

/// Where the drive reported errors as it read, from its progress lines:
/// a counter that rises between two lines puts errors between their
/// sectors. Nearby ones are joined, so a disc with errors everywhere
/// gives a few hundred lines, not one per sector. Only the first read
/// (`dump`) is followed; a re-read's counters count what's left.
///
/// Fed every line a run prints; gives a line for Rip's log each time a
/// stretch of errors ends: [`ERRORS_AT`] (see [`disc_map`]).
#[derive(Debug, Default)]
pub struct ErrorTrail {
    reading: bool,
    /// The last progress line's sector and counters.
    prev: Option<(u64, Vec<(String, u64)>)>,
    /// The stretch being gathered: from, to, the disc's sectors, its counts.
    open: Option<Stretch>,
}

type Stretch = (u64, u64, u64, Vec<(String, u64)>);

/// How a stretch of the drive's errors starts in Rip's log: *read errors
/// at sectors 49120-49300 of 332999: SCSI 2, C2 37*. The counts are the
/// drive's, in samples for a CD.
pub const ERRORS_AT: &str = "read errors at sectors ";

/// Errors this close (a 400th of the disc) are one stretch.
fn near(of: u64) -> u64 {
    (of / 400).max(1)
}

impl ErrorTrail {
    pub fn new() -> ErrorTrail {
        ErrorTrail::default()
    }

    /// Takes one printed line; gives a line to log when a stretch ends.
    pub fn line(&mut self, line: &str) -> Option<String> {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("*** ") {
            let done = self.finish();
            self.reading = h.split_whitespace().next() == Some("DUMP");
            self.prev = None;
            return done;
        }
        let Some((at, of)) = lba_of(t) else {
            // Anything else it says ends a stretch, so it's logged first.
            return if parse_progress(t).is_some() { None } else { self.finish() };
        };
        if !self.reading {
            return None;
        }
        let counts: Vec<(String, u64)> = counts_in(t).into_iter().filter(|c| c.0 != "Q").collect();
        let rose: Vec<(String, u64)> = match &self.prev {
            Some((_, before)) => counts.iter().map(|(k, n)| (k.clone(), n.saturating_sub(before.iter().find(|b| b.0 == *k).map_or(*n, |b| b.1)))).filter(|c| c.1 > 0).collect(),
            None => vec![],
        };
        let from = self.prev.as_ref().map_or(at, |p| p.0.min(at));
        self.prev = Some((at, counts));
        let mut out = None;
        if self.open.as_ref().is_some_and(|o| at > o.1 + near(of)) {
            out = self.finish();
        }
        if !rose.is_empty() {
            match &mut self.open {
                Some(o) => {
                    o.1 = at;
                    for (k, n) in rose {
                        match o.3.iter_mut().find(|c| c.0 == k) {
                            Some(c) => c.1 += n,
                            None => o.3.push((k, n)),
                        }
                    }
                }
                None => self.open = Some((from, at, of, rose)),
            }
        }
        out
    }

    /// The stretch being gathered, if any, as a line to log.
    pub fn finish(&mut self) -> Option<String> {
        let (from, to, of, counts) = self.open.take()?;
        Some(format!("{ERRORS_AT}{from}-{to} of {of}: {}", errors_text(&counts)))
    }
}

/// The sector and the disc's sectors in a progress line: `LBA: 12345/317787`.
fn lba_of(line: &str) -> Option<(u64, u64)> {
    parse_progress(line)?.at
}

// ---- following a read as it goes ---------------------------------------------------------------

/// Samples to a CD sector, which is how redumper counts a CD's errors.
pub const SAMPLES: u64 = 588;

/// The first sector redumper's `.state` file has: the lead-in's.
const STATE_START: i64 = -45_150;

/// The sectors of a CD dump still bad, from redumper's `.state` file: one
/// byte a sample, 0 a sample it couldn't read, 1 one with a C2 error (b744,
/// `common.ixx`). A sector is bad with most of its samples unread, or any
/// with a C2 error; counted from the file's start, which may sit a few
/// samples off the sector's own (the drive's offset), so each stretch of
/// bad sectors is counted to within one. Only the disc's own sectors: none
/// before the first, where a drive that can't read the lead-in leaves the
/// file empty.
pub fn bad_sectors(state: &Path) -> std::io::Result<Vec<u64>> {
    use std::io::Read;
    let mut f = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(state)?);
    let mut sector = vec![0u8; SAMPLES as usize];
    let mut out = vec![];
    let mut lba = STATE_START;
    loop {
        match f.read_exact(&mut sector) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        if lba >= 0 {
            let unread = sector.iter().filter(|&&b| b == 0).count() as u64;
            if unread * 2 > SAMPLES || sector.contains(&1) {
                out.push(lba as u64);
            }
        }
        lba += 1;
    }
    Ok(out)
}

/// redumper's `.state` file as a map of the disc, kept up to date while it
/// reads ([`crate::map`]). redumper (b744, `common.ixx`) writes one byte a
/// sample for a CD, from the lead-in (LBA −45,150), and one a sector for a
/// DVD or Blu-ray, from LBA 0: 0 not read (`ERROR_SKIP`), 1 a C2 error,
/// 2 to 4 read (with C2 or SCSI errors switched off, or cleanly). A CD
/// sector's samples are written `read offset` samples early
/// (`cd_dump.ixx`, `write_entry` with the drive's offset), so sector *n*
/// starts at byte (n + 45,150) · 588 − offset.
///
/// Reading the file whole each time would be 150 MB every few seconds:
/// instead, while redumper reads the disc through, only what it's read
/// since last time is read, from a little behind where it was (what it
/// writes may not be on disk yet); while it re-reads, only the sectors
/// still bad. Not read past where the drive is, so a sector not reached
/// is never shown as unreadable.
pub struct StateScan {
    path: PathBuf,
    /// Bytes a sector, and the first sector the file has.
    per: u64,
    first: i64,
    /// Samples redumper wrote each sector early by: the drive's offset.
    shift: i64,
    cd: bool,
    pub sectors: crate::map::Sectors,
    /// Sectors before this are settled: read again only when re-read.
    settled: u64,
    /// Where the drive was at the last look.
    at: u64,
}

/// How far behind the drive a sector not in the file yet may still be
/// coming: redumper flushes each sector's states as it writes them
/// (`write_entry`, `utils/file_io.ixx`), and writes none for a sector it
/// couldn't read, so one past the file's end that the drive is well past
/// couldn't be read; the one it's on, and the one before, may be on
/// their way.
const UNSETTLED: u64 = 2;

impl StateScan {
    /// The `.state` of `t`'s dump, the disc `sectors` long, read at
    /// `read_offset`.
    pub fn new(t: &Target, sectors: u64, read_offset: i64, cd: bool) -> StateScan {
        let (per, first, shift) = if cd { (SAMPLES, STATE_START, read_offset) } else { (1, 0, 0) };
        StateScan { path: t.file("state"), per, first, shift, cd, sectors: crate::map::Sectors::new(sectors), settled: 0, at: 0 }
    }

    /// The map as it is, with the drive at `at` while it reads.
    pub fn map(&self, reading: bool) -> crate::map::SectorMap {
        self.sectors.map(reading.then_some(self.at), self.cd)
    }

    /// A read of the disc through, the drive now at `at` of `of`: what's
    /// been read since the last look. `done` once the read has ended,
    /// when what it couldn't read is surely so.
    pub fn read_to(&mut self, at: u64, _of: u64, done: bool) -> std::io::Result<()> {
        // The disc as long as it was first said to be: the read goes a
        // sector or so on into the lead-out, which isn't the disc.
        self.at = self.at.max(at).min(self.sectors.len());
        let from = self.settled;
        let to = self.at;
        if to > from {
            self.look(from, to, |m, lba, end| match m {
                // Past what's written, right by the drive: may be on its way.
                None if !done && lba + UNSETTLED >= end => crate::map::Mark::NotRead,
                None => crate::map::Mark::Unreadable,
                Some(m) => m,
            })?;
        }
        self.settled = if done { to } else { to.saturating_sub(UNSETTLED).max(from) };
        Ok(())
    }

    /// A re-read, the drive at `at`: the sectors still bad looked at
    /// again; those now read are marked as come back. A re-read goes
    /// through the disc in order, so only those it has passed since the
    /// last look; all of them once it's `done`. The first look reads the
    /// whole disc as far as it was read, if nothing has yet.
    pub fn reread(&mut self, at: u64, of: u64, done: bool) -> std::io::Result<()> {
        if self.settled == 0 {
            self.read_to(of, of, true)?;
            self.at = 0;
        }
        let (lo, hi) = if done { (0, u64::MAX) } else { (self.at.min(at).saturating_sub(UNSETTLED), at + 1) };
        self.at = at;
        let bad: Vec<u64> = self.sectors.bad().filter(|l| (lo..hi).contains(l)).collect();
        let mut k = 0;
        while k < bad.len() {
            // Bad sectors near each other read as one stretch.
            let mut j = k;
            while j + 1 < bad.len() && bad[j + 1] - bad[j] <= 32 {
                j += 1;
            }
            let (from, to) = (bad[k], bad[j] + 1);
            let was: Vec<crate::map::Mark> = (from..to).map(|l| self.sectors.get(l)).collect();
            self.look(from, to, |m, lba, _| {
                let before = was[(lba - from) as usize];
                match m.unwrap_or(crate::map::Mark::Unreadable) {
                    crate::map::Mark::Good if before.is_bad() => crate::map::Mark::Recovered,
                    // What was fine stays as it was marked.
                    _ if !before.is_bad() => before,
                    now => now,
                }
            })?;
            k = j + 1;
        }
        Ok(())
    }

    /// Reads sectors `from` to `to` (not in) of the file, marking each by
    /// `mark` with what the file says of it (None past its end), its LBA
    /// and `to`.
    fn look(&mut self, from: u64, to: u64, mark: impl Fn(Option<crate::map::Mark>, u64, u64) -> crate::map::Mark) -> std::io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(&self.path)?;
        let len = f.metadata()?.len() as i64;
        const CHUNK: u64 = 4096;
        let mut buf = vec![0u8; (CHUNK * self.per) as usize];
        let mut lba = from;
        while lba < to {
            let n = CHUNK.min(to - lba);
            let start = (lba as i64 - self.first) * self.per as i64 - self.shift;
            let want = (n * self.per) as usize;
            // The file's part of it: none before its start, none past its end.
            let lo = start.clamp(0, len);
            let hi = (start + want as i64).clamp(0, len);
            let got = (hi - lo) as usize;
            f.seek(SeekFrom::Start(lo as u64))?;
            f.read_exact(&mut buf[..got])?;
            for k in 0..n {
                let (a, b) = (start + (k * self.per) as i64, start + ((k + 1) * self.per) as i64);
                // Past the file's end: not written yet.
                let said = (b <= len).then(|| {
                    let s = &buf[(a.max(lo) - lo) as usize..(b - lo).max(0) as usize];
                    match s.iter().min().copied().unwrap_or(0) {
                        0 => crate::map::Mark::Unreadable,
                        1 => crate::map::Mark::C2,
                        _ => crate::map::Mark::Good,
                    }
                });
                let l = lba + k;
                self.sectors.set(l, mark(said, l, to));
            }
            lba += n;
        }
        Ok(())
    }
}

/// What Rip is doing just now, while redumper reads or re-reads: for the
/// card and the sidebar ([`drt_engine::StepView::now`]).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Live {
    /// Re-reading what was bad (`refine`), not reading the disc through.
    pub rereading: bool,
    /// The sector it's at, and the disc's sectors.
    pub at: u64,
    pub of: u64,
    /// Which try on that sector (0 its first read this pass), of how many
    /// re-reads each gets.
    pub retry: u32,
    pub retries: u32,
    /// Bad sectors gone through this pass, of how many there were; and how
    /// many of those came back. The total isn't known for a DVD.
    pub done: u64,
    pub total: Option<u64>,
    pub recovered: u64,
    /// The time left, once the pace says anything. A read's is left out
    /// while its speed says nothing of it: errors rising, or crawling.
    pub left_secs: Option<u64>,
    /// A read's speed over the last half minute, as a multiple of 1×; 0
    /// when it hasn't moved. Not known for its first few seconds.
    #[serde(default)]
    pub speed: Option<f32>,
    /// A read's errors (the drive's, not the subchannel's) rose in the
    /// last minute.
    #[serde(default)]
    pub errors_rising: bool,
    /// The read is crawling: the drive meets errors and hardly moves.
    pub crawling: bool,
    /// Bad sectors gone through, every try spent, since the last that came
    /// back (or since the re-read began).
    #[serde(default)]
    pub since_back: u64,
    /// The re-read is no use: [`GIVE_UP_AFTER`] bad sectors in a row had
    /// every try and none came back. It's stopped.
    #[serde(default)]
    pub given_up: bool,
    /// The last sector that came back, and on which try: kept in the log,
    /// so how many tries are worth it can be seen.
    #[serde(default)]
    pub last_back: Option<(u64, u32)>,
}

/// A re-read stops by itself after this many bad sectors in a row have
/// had every try with none coming back. A sector that comes back at all
/// tends to on its first few tries, so fifty that don't is a drive that
/// can't read this patch, and the rest would take hours to the same end.
/// Fifty is about 3% of a disc with 1,500 bad sectors, a few minutes at 20
/// tries each; one that comes back starts the count again, so a re-read
/// that's getting somewhere goes on.
pub const GIVE_UP_AFTER: u64 = 50;

/// What Rip adds to redumper's output when it stopped a re-read itself.
pub const GAVE_UP: &str = "drt: re-read stopped, nothing coming back";

/// Whether the last re-read in a run's output was stopped as no use.
pub fn gave_up(output: &str) -> bool {
    let last = output.rfind("*** REFINE").unwrap_or(0);
    output[last..].contains(GAVE_UP)
}

/// What Rip says when it stopped a re-read that was getting nowhere.
pub fn given_up_line() -> String {
    format!("Stopped: the drive couldn't recover any of the last {GIVE_UP_AFTER} bad sectors. Cleaning the disc or another drive may help.")
}

/// What a card says once a read is crawling.
pub const CRAWLING_CARD: &str = "The drive can't read parts of this disc";

/// What the sidebar says once a read is crawling.
pub const CRAWLING: &str = "This disc has areas the drive can't read. Cleaning it, or another drive, may help.";

impl Live {
    /// How far through a re-read is, by the bad sectors gone through (and
    /// the tries on the one it's at); through a read, by the sectors.
    pub fn fraction(&self) -> f32 {
        match self.total.filter(|t| *t > 0 && self.rereading) {
            Some(t) => ((self.done as f32 + self.retry as f32 / (self.retries + 1) as f32) / t as f32).clamp(0.0, 1.0),
            None => self.position(),
        }
    }

    /// How far through the disc it is.
    pub fn position(&self) -> f32 {
        if self.of == 0 {
            0.0
        } else {
            (self.at as f32 / self.of as f32).clamp(0.0, 1.0)
        }
    }

    /// Where it is, in a few words: *sector 147,223 (54%), try 19 of 20*.
    pub fn place(&self) -> String {
        let mut s = format!("sector {} ({}%)", thousands(self.at), (self.position() * 100.0) as u32);
        if self.rereading && self.retry > 0 {
            s.push_str(&format!(", try {} of {}", self.retry, self.retries));
        }
        s
    }

    /// What came back so far: *2 recovered so far*.
    pub fn recovered_text(&self) -> String {
        format!("{} recovered so far", thousands(self.recovered))
    }

    /// The card's line: *Re-reading bad parts: sector 147,223 (54%), try
    /// 19 of 20 · 2 recovered so far*.
    pub fn line(&self) -> String {
        let mut s = format!("Re-reading bad parts: {} · {}", self.place(), self.recovered_text());
        if let Some(l) = self.left_secs {
            s.push_str(&format!(" · {}", left_words(l)));
        }
        s
    }

    /// The card's line, its parts least needed first, since a card short
    /// of room drops them from the front: *Re-reading bad parts: sector
    /// 147,223 (54%) · 2 recovered so far · try 19 of 20*; crawling, what
    /// the drive can't do last.
    pub fn card(&self) -> String {
        let mut parts = vec![format!("Re-reading bad parts: sector {} ({}%)", thousands(self.at), (self.position() * 100.0) as u32), self.recovered_text()];
        if self.retry > 0 {
            parts.push(format!("try {} of {}", self.retry, self.retries));
        }
        if self.crawling {
            parts.push(CRAWLING_CARD.into());
        }
        parts.join(" · ")
    }

    /// The item's note: how far through the re-read, for the strip and
    /// the ring. *re-reading 4%, 31 of 774 bad sectors*.
    pub fn note(&self) -> String {
        let pc = (self.fraction() * 100.0) as u32;
        match self.total {
            Some(t) => format!("re-reading {pc}%, {} of {} bad sectors", thousands(self.done), thousands(t)),
            None => format!("re-reading {pc}%, {}", self.place()),
        }
    }
}

/// A time left, roughly, for a pass that may take hours: *about 40 min
/// left*, *about 3 h 20 min left*.
pub fn left_words(secs: u64) -> String {
    let min = secs.div_ceil(60);
    match min {
        0..=1 => "about a minute left".into(),
        2..=59 => format!("about {min} min left"),
        600.. => format!("about {} h left", (min + 30) / 60),
        _ => match min.div_ceil(10) * 10 {
            m if m % 60 == 0 => format!("about {} h left", m / 60),
            m => format!("about {} h {} min left", m / 60, m % 60),
        },
    }
}

/// A number with its thousands apart: 147,223.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (k, c) in s.chars().enumerate() {
        if k > 0 && (s.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// How long a crawl has to last before it's said: three minutes.
pub const CRAWL_SECS: f64 = 180.0;
/// A read's speed is taken over this long: half a minute of its lines.
pub const SPEED_SECS: f64 = 30.0;
/// Nor said before it has this long to go on.
pub const SPEED_MIN_SECS: f64 = 3.0;
/// A read's errors are rising if they rose in this long: a minute, since a
/// drive stuck on a sector may print nothing for most of one.
pub const ERRORS_SECS: f64 = 60.0;
/// A read crawls below this share of a 1× read (a CD's 75 sectors a
/// second): 5%, a twentieth of the slowest speed a drive reads at.
pub const CRAWL_SHARE: f64 = 0.05;
/// A re-read crawls once this many bad sectors have been gone through and
/// fewer than one in [`CRAWL_ONE_IN`] came back.
pub const CRAWL_TRIED: u64 = 10;
pub const CRAWL_ONE_IN: u64 = 10;

/// Follows a pass of redumper's from its progress lines, as [`Live`]: where
/// it is, which try, what's come back, the time left, and whether it's
/// crawling. A read's speed is its last half minute's, so a bad patch
/// shows as it's read, and whether its errors are rising.
///
/// A read (`dump`) crawls when for three minutes it has gone less than 5%
/// as far as a 1× read would, and the drive's errors rose meanwhile: a
/// sound disc read at the slowest speed still moves twenty times faster,
/// so this is the drive stuck on what it can't read. A re-read (`refine`)
/// is slow by nature, sector by sector with many tries each, so what
/// counts there is what comes back: after three minutes and ten bad
/// sectors, fewer than one in ten has.
///
/// A sector of a re-read came back when redumper left it before its last
/// try: it leaves a sector early only once it reads whole (`cd_dump.ixx`).
/// One that comes back on its very last try isn't counted: the pass's own
/// figures at its end say.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    live: Live,
    /// The bad sectors to re-read, sorted, when known.
    bad: Vec<u64>,
    /// The errors (SCSI and C2, in samples) when it came to the sector
    /// it's on, and on the line before this one.
    at_start: u64,
    last_errors: u64,
    /// When the first line came, and the lines of the last few minutes:
    /// when, which sector, and the errors so far.
    started: Option<f64>,
    window: std::collections::VecDeque<(f64, u64, u64)>,
    /// A 1× read, in sectors a second.
    one_x: f64,
}

impl Watch {
    /// A read of the disc through, at `one_x` sectors a second for 1×.
    pub fn read(one_x: f64) -> Watch {
        Watch { one_x, ..Default::default() }
    }

    /// A re-read of `bad` (sorted; empty when not known) with `retries`
    /// re-reads of each.
    pub fn reread(retries: u32, bad: Vec<u64>) -> Watch {
        Watch { live: Live { rereading: true, retries, ..Default::default() }, bad, ..Default::default() }
    }

    pub fn live(&self) -> &Live {
        &self.live
    }

    /// Takes one progress line, `now` seconds into the pass. Gives true the
    /// once it starts crawling.
    pub fn line(&mut self, p: &Progress, now: f64) -> bool {
        let Some((at, of)) = p.at else { return false };
        let errors: u64 = p.counts.iter().filter(|c| c.0 != "Q").map(|c| c.1).sum();
        let started = *self.started.get_or_insert(now);
        let l = &mut self.live;
        if l.rereading {
            if l.of == 0 && !self.bad.is_empty() {
                l.total = Some(self.bad.partition_point(|&b| b < of) as u64);
            }
            // On to another sector: the one before is gone through, and
            // came back if it was left before its last try.
            if l.of > 0 && at != l.at {
                l.done += 1;
                // Came back: left before its last try, and the counters
                // fell while it was on it. One left at once with nothing
                // falling was already read (redumper looks at it from two
                // offsets) and is neither.
                let fell = errors < self.at_start;
                if l.retry < l.retries && fell {
                    l.recovered += 1;
                    l.since_back = 0;
                    l.last_back = Some((l.at, l.retry));
                } else if l.retry >= l.retries {
                    l.since_back += 1;
                    l.given_up |= l.since_back >= GIVE_UP_AFTER;
                }
            }
            if l.of == 0 || at != l.at {
                self.at_start = errors;
            }
            self.last_errors = errors;
            l.retry = p.retry.unwrap_or(0);
            if let Some(t) = &mut l.total {
                *t = (*t).max(l.done + 1);
            }
        }
        l.at = at;
        l.of = of;
        let secs = now - started;
        if l.rereading {
            l.left_secs = match l.total {
                Some(t) if l.done >= 5 && secs >= 60.0 => Some(((t - l.done) as f64 * secs / l.done as f64) as u64),
                _ => None,
            };
        } else {
            // A line a second at most is enough to judge by.
            if self.window.back().is_none_or(|w| now - w.0 >= 1.0) {
                self.window.push_back((now, at, errors));
            }
            while self.window.len() > 2 && now - self.window[1].0 >= CRAWL_SECS {
                self.window.pop_front();
            }
            // The speed now: from the line half a minute back (or the
            // first, early on) to this one.
            let back = |secs: f64| self.window.iter().rev().find(|w| now - w.0 >= secs).or(self.window.front()).copied().unwrap_or((now, at, errors));
            let from = back(SPEED_SECS);
            let span = now - from.0;
            let sps = (span >= SPEED_MIN_SECS).then(|| at.saturating_sub(from.1) as f64 / span);
            l.speed = sps.map(|s| (s / self.one_x) as f32);
            l.errors_rising = errors > back(ERRORS_SECS).2;
            // The time left at that speed, while it says anything of it.
            l.left_secs = sps.filter(|s| *s >= CRAWL_SHARE * self.one_x && !l.errors_rising).map(|s| (of.saturating_sub(at) as f64 / s) as u64);
        }
        if l.crawling {
            return false;
        }
        let crawling = if l.rereading {
            secs >= CRAWL_SECS && l.done >= CRAWL_TRIED && l.recovered * CRAWL_ONE_IN < l.done
        } else {
            let first = self.window.front().copied().unwrap_or((now, at, errors));
            let span = now - first.0;
            span >= CRAWL_SECS && ((at.saturating_sub(first.1)) as f64) < CRAWL_SHARE * self.one_x * span && errors > first.2
        };
        l.crawling = crawling;
        crawling
    }
}

/// A stretch of sectors, and how much went wrong in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub from: u64,
    pub to: u64,
    /// The drive's count (samples on a CD), or the sectors found wrong.
    pub weight: u64,
}

/// The disc as Rip's log maps it: its length, where the drive reported
/// errors as it read, and where the check of every sector found sectors
/// wrong (`MSF`, `ECC`, `EDC`), by the lists redumper's `info` gives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiscMap {
    pub sectors: u64,
    pub read: Vec<Span>,
    pub wrong: Vec<Span>,
    /// Sectors found wrong: each listed once, however many checks it
    /// fails; at least as many as the commonest kind's count.
    pub wrong_count: u64,
}

impl DiscMap {
    pub fn is_empty(&self) -> bool {
        self.read.is_empty() && self.wrong.is_empty()
    }
}

/// Reads the map of the disc from Rip's log: the [`ERRORS_AT`] lines Rip
/// writes as it reads, and the last `info`'s lists of sectors.
pub fn disc_map<'a>(log: impl IntoIterator<Item = &'a str>) -> DiscMap {
    let mut m = DiscMap::default();
    let mut info: Vec<&str> = vec![];
    let mut in_info = false;
    for line in log {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("*** ") {
            let word = h.split_whitespace().next();
            if word == Some("INFO") {
                info.clear();
                in_info = true;
            } else {
                in_info = false;
            }
            continue;
        }
        if in_info {
            info.push(t);
        }
        let Some(rest) = t.strip_prefix(ERRORS_AT) else { continue };
        let Some((span, counts)) = rest.split_once(": ") else { continue };
        let Some((range, of)) = span.split_once(" of ") else { continue };
        let Some((a, b)) = range.split_once('-') else { continue };
        let (Ok(from), Ok(to), Ok(of)) = (a.parse::<u64>(), b.parse::<u64>(), of.parse::<u64>()) else { continue };
        let weight = counts.split(", ").filter_map(|c| c.rsplit_once(' ')?.1.parse::<u64>().ok()).sum::<u64>().max(1);
        m.sectors = m.sectors.max(of);
        m.read.push(Span { from, to, weight });
    }
    let mut listed: Vec<(u64, u64)> = vec![];
    let mut most = 0u64;
    for (key, _) in PROBLEMS.iter().filter(|(k, _)| *k != "invalid sync sectors:") {
        let mut count = 0;
        for l in info.iter().filter_map(|l| l.strip_prefix(key)) {
            count += l.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
            let list = l.split_once("(LBA:").map(|(_, r)| r.trim_end_matches(')')).unwrap_or("");
            listed.extend(list.split(',').filter_map(super::mismatch::range).map(|(a, b)| (a.max(0) as u64, b.max(0) as u64)));
        }
        most = most.max(count);
    }
    // Each sector once, however many checks it fails.
    listed.sort_unstable();
    for (a, b) in listed {
        match m.wrong.last_mut() {
            Some(w) if a <= w.to + 1 => w.to = w.to.max(b),
            _ => m.wrong.push(Span { from: a, to: b, weight: 0 }),
        }
    }
    for w in &mut m.wrong {
        w.weight = w.to - w.from + 1;
    }
    m.wrong_count = m.wrong.iter().map(|w| w.weight).sum::<u64>().max(most);
    if m.sectors == 0 {
        m.sectors = info.iter().filter_map(|l| l.strip_prefix("sectors count:")?.trim().parse::<u64>().ok()).sum();
    }
    m
}

/// The shortest stretch holding most (four in five) of the spans' weight,
/// as sectors; and whether it's the whole of where they are.
pub fn most_between(spans: &[Span]) -> Option<(u64, u64, bool)> {
    let mut s: Vec<&Span> = spans.iter().collect();
    s.sort_by_key(|x| x.from);
    let total: u64 = s.iter().map(|x| x.weight).sum();
    let (first, last) = (s.first()?.from, s.iter().map(|x| x.to).max()?);
    let want = (total * 4).div_ceil(5);
    let mut best = (first, last);
    let mut sum = 0u64;
    let mut lo = 0usize;
    for hi in 0..s.len() {
        sum += s[hi].weight;
        while sum - s[lo].weight >= want && lo < hi {
            sum -= s[lo].weight;
            lo += 1;
        }
        let to = s[lo..=hi].iter().map(|x| x.to).max().unwrap_or(s[hi].to);
        if sum >= want && to - s[lo].from < best.1 - best.0 {
            best = (s[lo].from, to);
        }
    }
    Some((best.0, best.1, best == (first, last)))
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
        // From the sector, not the whole percent beside it.
        assert!((p.fraction - 12345.0 / 317787.0).abs() < 1e-6);
        assert_eq!(p.detail, "sector 12345 of 317787");
        assert_eq!(p.errors, 0, "the 2 in C2s is not an error");
        let p = parse_progress(DVD_LINE).unwrap();
        assert!((p.fraction - 1234567.0 / 2298496.0).abs() < 1e-6);
        // A bad patch: the sector moves, the whole percent doesn't.
        let a = parse_progress("| [ 54%] LBA: 147262/269159, errors: { SCSIs: 8820, C2s: 630, Q: 220 }").unwrap();
        let b = parse_progress("| [ 54%] LBA: 147702/269159, errors: { SCSIs: 37632, C2s: 1385, Q: 331 }").unwrap();
        assert!(b.fraction > a.fraction);
        assert_eq!(p.detail, "sector 1234567 of 2298496");
        let p = parse_progress("⠋ [ 88%] LBA:  99999/317787, errors: { SCSIs: 3, C2s: 12, Q: 0 }").unwrap();
        assert_eq!(p.errors, 15);
        assert_eq!(parse_progress("⠹ [ 55%] splitting").unwrap().detail, "splitting");
    }

    /// A line from a real re-read of a damaged PlayStation disc (b744).
    const REFINE_LINE: &str = "[ 54%] LBA: 147223/269159, errors: { SCSIs: 455112, C2s: 10553, Q: 2241 }, retry: 19";

    #[test]
    fn a_re_read_s_line_says_where_and_which_try() {
        let p = parse_progress(REFINE_LINE).unwrap();
        assert_eq!((p.at, p.retry), (Some((147_223, 269_159)), Some(19)));
        assert_eq!(p.counts[0], ("SCSI".to_owned(), 455_112));
        assert_eq!(p.counts[0].1 / SAMPLES, 774, "samples, 588 to a sector");
        // From the terminal, with its spinner and escape codes, and the
        // share of the sector read so far after the try.
        let p = parse_progress(&format!("\x1b[2K\x1b[1G⠙ {REFINE_LINE}, data:  97%")).unwrap();
        assert_eq!((p.at, p.retry, (p.fraction * 100.0).round()), (Some((147_223, 269_159)), Some(19), 55.0), "147,223 of 269,159 is 54.7%");
        // The first read of a sector has no try, nor has a read of the disc.
        let p = parse_progress("⠋ [ 54%] LBA: 147223/269159, errors: { SCSIs: 455112, C2s: 10553, Q: 2241 }").unwrap();
        assert_eq!(p.retry, None);
        assert_eq!(parse_progress(CD_LINE).unwrap().retry, None);
    }

    fn at(lba: u64, retry: u32, scsi: u64) -> Progress {
        let r = if retry > 0 { format!(", retry: {retry}") } else { String::new() };
        parse_progress(&format!("[ 54%] LBA: {lba}/269159, errors: {{ SCSIs: {scsi}, C2s: 0, Q: 0 }}{r}")).unwrap()
    }

    #[test]
    fn a_re_read_counts_bad_sectors_gone_through_and_back() {
        // 774 bad sectors, re-read 20 times each.
        let bad: Vec<u64> = (147_200..147_200 + 774).collect();
        let mut w = Watch::reread(20, bad);
        let mut t = 0.0;
        let mut feed = |w: &mut Watch, lba, tries: u32, scsi| {
            for r in 0..=tries {
                w.line(&at(lba, r, scsi), t);
                t += 0.1;
            }
        };
        // The first never comes back; the second does on its third try,
        // and the counters fall by its 588 samples.
        feed(&mut w, 147_200, 20, 455_112);
        feed(&mut w, 147_201, 2, 455_112);
        w.line(&at(147_223, 19, 454_524), t);
        let l = w.live();
        assert_eq!((l.done, l.total, l.recovered, l.retry), (2, Some(774), 1, 19));
        assert_eq!(l.line(), "Re-reading bad parts: sector 147,223 (54%), try 19 of 20 · 1 recovered so far");
        assert_eq!(l.card(), "Re-reading bad parts: sector 147,223 (54%) · 1 recovered so far · try 19 of 20");
        assert_eq!(l.note(), "re-reading 0%, 2 of 774 bad sectors");
        assert!((l.fraction() - (2.0 + 19.0 / 21.0) / 774.0).abs() < 1e-6);
        assert!((l.position() - 0.547).abs() < 1e-3, "the place on the disc is the line's own");
        // Not known (a DVD): where it is, not how far through.
        let mut w = Watch::reread(20, vec![]);
        w.line(&at(147_223, 3, 1), 0.0);
        assert_eq!((w.live().total, w.live().note()), (None, "re-reading 54%, sector 147,223 (54%), try 3 of 20".to_owned()));
    }

    #[test]
    fn a_re_read_that_brings_nothing_back_is_crawling_and_says_so_once() {
        // Koudelka's: some 66 sectors in three minutes, 2 back.
        let mut w = Watch::reread(20, (0..774).map(|k| 147_000 + k * 3).collect());
        let mut said = 0;
        let mut scsi = 455_112;
        for k in 0..70u64 {
            let lba = 147_000 + k * 3;
            let last = if k % 33 == 5 { 4 } else { 20 };
            for r in 0..=last {
                said += w.line(&at(lba, r, scsi), (k * 21 + r as u64) as f64 * 0.13) as u32;
            }
            if last == 4 {
                scsi -= SAMPLES;
            }
        }
        assert!(w.live().crawling && said == 1, "{:?}", w.live());
        assert_eq!(w.live().recovered, 2);
        // 69 in 191 s: the other 705 in about half an hour.
        assert_eq!(w.live().left_secs.map(left_words).as_deref(), Some("about 33 min left"));
        // One where most come back isn't.
        let mut w = Watch::reread(20, (0..100).collect());
        for k in 0..60u64 {
            w.line(&at(k, 0, (100 - k) * SAMPLES), k as f64 * 10.0);
        }
        assert!(!w.live().crawling && w.live().recovered == 59, "{:?}", w.live());
    }

    #[test]
    fn a_re_read_that_brings_none_of_fifty_back_gives_up() {
        // Koudelka's, after 40 minutes: some 570 sectors, none back.
        let mut w = Watch::reread(20, (147_157..147_157 + 774).collect());
        let mut gave_up_at = None;
        for k in 0..60u64 {
            for r in 0..=20 {
                w.line(&at(147_157 + k, r, 455_112), (k * 21 + r as u64) as f64);
            }
            if w.live().given_up && gave_up_at.is_none() {
                gave_up_at = Some(k);
            }
        }
        // The 50th is known to have had every try once the 51st begins.
        assert_eq!(gave_up_at, Some(50));
        assert!(gave_up(&format!("*** REFINE\n{GAVE_UP}\n")) && !gave_up(&format!("*** REFINE\n{GAVE_UP}\n*** REFINE\n")));
        assert_eq!(given_up_line(), "Stopped: the drive couldn't recover any of the last 50 bad sectors. Cleaning the disc or another drive may help.");
        // One that comes back now and then starts the count again.
        let mut w = Watch::reread(20, (0..774).collect());
        let mut scsi = 100_000;
        for k in 0..200u64 {
            let last = if k % 40 == 39 { 3 } else { 20 };
            for r in 0..=last {
                w.line(&at(k, r, scsi), 0.0);
            }
            if last == 3 {
                scsi -= SAMPLES;
            }
        }
        assert!(!w.live().given_up && w.live().recovered == 4, "{:?}", w.live());
        // Sectors left at once with the counters flat (good from where
        // redumper reads them) are neither back nor a reason to give up;
        // nor do they hide fifty that had every try.
        let mut w = Watch::reread(20, (0..774).collect());
        for k in 0..120u64 {
            let last = if k % 2 == 0 { 0 } else { 20 };
            for r in 0..=last {
                w.line(&at(k, r, 455_112), 0.0);
            }
        }
        assert_eq!(w.live().recovered, 0);
        assert!(w.live().given_up, "{:?}", w.live());
    }

    /// Lines from a real read of a scratched CD, at the minutes they came.
    const SCRATCHED: [(f64, &str); 4] = [
        (0.0, "⠋ [ 54%] LBA: 147262/269159, errors: { SCSIs: 8820, C2s: 630, Q: 220 }"),
        (180.0, "⠙ [ 57%] LBA: 153834/269159, errors: { SCSIs: 49392, C2s: 1525, Q: 285 }"),
        (300.0, "⠹ [ 73%] LBA: 197515/269159, errors: { SCSIs: 49392, C2s: 1525, Q: 564 }"),
        (420.0, "⠸ [ 73%] LBA: 197913/269159, errors: { SCSIs: 72912, C2s: 2239, Q: 596 }"),
    ];

    #[test]
    fn a_scratched_reads_lines() {
        let p = parse_progress(SCRATCHED[0].1).unwrap();
        assert_eq!(p.at, Some((147_262, 269_159)));
        assert_eq!(p.counts, [("SCSI".to_owned(), 8820), ("C2".to_owned(), 630), ("Q".to_owned(), 220)]);
        assert_eq!(p.retry, None);
        let p = parse_progress(SCRATCHED[3].1).unwrap();
        assert_eq!((p.at, p.errors), (Some((197_913, 269_159)), 72_912 + 2239 + 596));
    }

    #[test]
    fn a_reads_speed_is_its_last_half_minutes_even_through_errors() {
        let mut w = Watch::read(75.0);
        let mut seen = vec![];
        for (t, l) in SCRATCHED {
            w.line(&parse_progress(l).unwrap(), t);
            let l = w.live();
            seen.push((l.speed.map(|s| (s * 100.0).round() / 100.0), l.errors_rising, l.left_secs));
        }
        assert_eq!(seen[0], (None, false, None), "nothing to go on yet");
        // 6572 sectors in three minutes, the drive's errors rising: half
        // of 1×, and no time left said.
        assert_eq!(seen[1], (Some(0.49), true, None));
        // Then quickly, only the subchannel's counter rising: 4.9×, and
        // 71644 sectors left at 364 a second.
        assert_eq!(seen[2], (Some(4.85), false, Some(196)));
        // Into errors again: 398 sectors in two minutes.
        assert_eq!(seen[3], (Some(0.04), true, None));
        // Stuck on one sector for 40 s: not moving, errors still risen.
        w.line(&parse_progress(SCRATCHED[3].1).unwrap(), 460.0);
        assert_eq!((w.live().speed, w.live().errors_rising, w.live().left_secs), (Some(0.0), true, None));
        // Still not crawling: it read well within three minutes.
        assert!(!w.live().crawling);
    }

    #[test]
    fn a_crawling_reads_speed_is_still_followed() {
        let line = |lba: u64, scsi: u64| parse_progress(&progress_line(lba, 269_159, true, scsi, 0)).unwrap();
        let mut w = Watch::read(75.0);
        for k in 0..10u64 {
            w.line(&line(100_000 + k, k * SAMPLES), k as f64 * 30.0);
        }
        assert!(w.live().crawling);
        // Off the scratch, at 8×: said as it is, the crawl still said.
        for s in 1..=80u64 {
            w.line(&line(100_009 + s * 600, 9 * SAMPLES), 270.0 + s as f64);
        }
        let l = w.live();
        assert_eq!((l.speed, l.errors_rising, l.crawling), (Some(8.0), false, true));
        assert!(l.left_secs.is_some());
    }

    #[test]
    fn a_read_crawls_when_it_hardly_moves_and_errors_rise() {
        let line = |lba: u64, scsi: u64| parse_progress(&progress_line(lba, 269_159, true, scsi, 0)).unwrap();
        // A sound disc at 1×: 75 sectors a second, no errors.
        let mut w = Watch::read(75.0);
        for s in 0..400u64 {
            assert!(!w.line(&line(s * 75, 0), s as f64));
        }
        // Stuck: a sector every 30 seconds, each an error.
        let mut w = Watch::read(75.0);
        let mut said = vec![];
        for k in 0..20u64 {
            said.push(w.line(&line(100_000 + k, k * SAMPLES), k as f64 * 30.0));
        }
        assert_eq!(said.iter().filter(|s| **s).count(), 1);
        assert!(said[..6].iter().all(|s| !s), "not before three minutes");
        // Slow without errors (a drive spinning up, say) isn't crawling.
        let mut w = Watch::read(75.0);
        for k in 0..20u64 {
            w.line(&line(100_000 + k, 0), k as f64 * 30.0);
        }
        assert!(!w.live().crawling);
    }

    #[test]
    fn bad_sectors_from_the_state_file() {
        let dir = std::env::current_dir().unwrap().join("../../target/drt-test/state-file");
        std::fs::create_dir_all(&dir).unwrap();
        // The lead-in unread (zeros), then 10 sectors: 3 and 4 unread a
        // few samples off, 7 with one C2 sample; the rest good (4).
        let mut b = vec![0u8; 45_150 * 588];
        let mut disc = vec![4u8; 10 * 588];
        disc[3 * 588 + 6..5 * 588 + 6].fill(0);
        disc[7 * 588 + 100] = 1;
        b.extend(disc);
        std::fs::write(dir.join("D.state"), b).unwrap();
        assert_eq!(bad_sectors(&dir.join("D.state")).unwrap(), [3, 4, 7]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A CD's `.state` as redumper writes it: the lead-in not read, then
    /// `marks` (a state byte a sector), each sector's samples `shift` early.
    fn state_file(path: &Path, marks: &[u8], shift: usize) {
        let mut b = vec![0u8; 45_150 * 588 - shift];
        for m in marks {
            b.extend(std::iter::repeat_n(*m, 588));
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn the_state_file_is_mapped_as_it_s_read_and_re_read() {
        use crate::map::Mark;
        let dir = std::env::current_dir().unwrap().join("../../target/drt-test/state-scan");
        std::fs::create_dir_all(&dir).unwrap();
        let t = Target { dir: dir.clone(), name: "D".into() };
        // Written as far as 1,490, the drive at 1,500 of 2,000, at +103:
        // 100 and 1,400 unreadable, 200 with a C2 error; 1,490 to 1,497
        // not written, the drive past them; 1,498 and 1,499 may be coming.
        let mut marks = vec![4u8; 1_490];
        marks[100] = 0;
        marks[200] = 1;
        marks[1_400] = 0;
        state_file(&t.file("state"), &marks, 103);
        let mut s = StateScan::new(&t, 2_000, 103, true);
        s.read_to(1_500, 2_000, false).unwrap();
        assert_eq!([s.sectors.get(99), s.sectors.get(100), s.sectors.get(101), s.sectors.get(200)], [Mark::Good, Mark::Unreadable, Mark::Good, Mark::C2]);
        assert_eq!((s.sectors.get(1_400), s.sectors.get(1_497), s.sectors.get(1_498)), (Mark::Unreadable, Mark::Unreadable, Mark::NotRead));
        let m = s.map(true);
        assert_eq!((m.total(Mark::Unreadable), m.total(Mark::C2), m.total(Mark::NotRead)), (10, 1, 502));
        assert_eq!(m.at, Some(1_500));
        // Read to its end, and a sector on into the lead-out, which isn't
        // the disc: 1,498 and on written good.
        marks.extend([0u8; 8]);
        marks.extend([4u8; 503]);
        state_file(&t.file("state"), &marks, 103);
        s.read_to(1_900, 2_001, false).unwrap();
        s.read_to(2_001, 2_001, true).unwrap();
        assert_eq!((s.sectors.get(1_498), s.sectors.get(1_999), s.sectors.len()), (Mark::Good, Mark::Good, 2_000));
        // A re-read: 100 comes back; 200 doesn't.
        marks[100] = 4;
        state_file(&t.file("state"), &marks, 103);
        s.reread(200, 2_000, true).unwrap();
        assert_eq!((s.sectors.get(100), s.sectors.get(200), s.sectors.get(99)), (Mark::Recovered, Mark::C2, Mark::Good));
        // A scan begun at the re-read reads the disc whole first.
        let mut fresh = StateScan::new(&t, 2_000, 103, true);
        fresh.reread(200, 2_000, false).unwrap();
        assert_eq!((fresh.sectors.get(100), fresh.sectors.get(200), fresh.sectors.get(1_492)), (Mark::Good, Mark::C2, Mark::Unreadable));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A real `.state`: a damaged PlayStation disc read to about sector
    /// 199,210 of 269,159 at +103, then stopped. Not in the repository
    /// (143 MB): `DRT_STATE_FIXTURE` names it, or it's looked for where
    /// the flatpak keeps its runs.
    #[test]
    #[ignore]
    fn a_real_state_file_shows_a_spot_once_per_turn() {
        use crate::map::Mark;
        let path = std::env::var("DRT_STATE_FIXTURE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".var/app/io.github.vresistor.DiscRipTrip/cache/disc-rip-trip/runs/playstation-1795362157/image/KOUDELKA_4.state"));
        let t = Target { dir: path.parent().unwrap().to_owned(), name: path.file_stem().unwrap().to_string_lossy().into_owned() };
        let began = std::time::Instant::now();
        let mut s = StateScan::new(&t, 269_159, 103, true);
        s.read_to(199_210, 269_159, true).unwrap();
        let m = s.map(false);
        eprintln!("scanned in {:?}", began.elapsed());
        // As redumper's own count: 355,152 samples unread, 604 sectors.
        assert_eq!(m.total(Mark::Unreadable), 604);
        assert_eq!(m.total(Mark::NotRead), 269_159 - 199_210);
        eprintln!("{} good, {} unreadable, {} with C2; first runs {:?}", m.total(Mark::Good), m.total(Mark::Unreadable), m.total(Mark::C2), &m.runs[..8]);
        let g = crate::map::Geometry::CD;
        eprintln!("spots: {:?}", crate::map::once_per_turn(&m.runs, &g));
        eprintln!("{:?}\n{:?}", crate::map::where_line(&m), crate::map::turn_line(&m));
        // Bad stretches two sectors long, 16.3 sectors apart at 4.4 cm:
        // one spot met every turn (2.5% over the nominal turn: a disc
        // pressed at about 1.27 m/s). The other place, at 4.9 cm, comes
        // once a turn too, but with stretches up to three turns long where
        // it's worst: a band there, not a spot, so not said.
        assert_eq!(crate::map::turn_line(&m).as_deref(), Some("Errors repeat once per turn: a single spot about 4.4 cm out, not a whole ring."));
    }

    #[test]
    fn hours_are_said_roughly() {
        assert_eq!(left_words(30), "about a minute left");
        assert_eq!(left_words(40 * 60), "about 40 min left");
        assert_eq!(left_words(3 * 3600 + 20 * 60), "about 3 h 20 min left");
        assert_eq!(left_words(5 * 3600 + 60), "about 5 h 10 min left");
        assert_eq!(left_words(3600 + 56 * 60), "about 2 h left");
        assert_eq!(left_words(3600 + 49 * 60), "about 1 h 50 min left");
        assert_eq!(left_words(11 * 3600), "about 11 h left");
        assert_eq!(thousands(147_223), "147,223");
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
        assert_eq!(progress_note(&p), "31%, sector 99999 of 317787, 15 errors");
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
    fn a_split_that_stops_says_which_sectors_it_couldn_t_read() {
        // Koudelka (Europe) (Disc 4), on a drive that couldn't read it.
        let line = "errors detected, track: 1, sectors: {SKIP: 774, C2: 120}, samples: {SKIP: 455112, C2: 10553}";
        assert_eq!(split_errors(line), Some((774, 120)));
        assert_eq!(split_errors("checking tracks"), None);
        assert_eq!(unreadable_line(774, 120), "Couldn't read 774 sectors, and 120 more came back with errors");
        assert_eq!(unreadable_line(1, 0), "Couldn't read 1 sector");
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
        let o = Options { drive: "/dev/sr0".into(), retries: 20, speed: None, read_offset: None, sector_order: None };
        assert_eq!(dump_args(&o, &t), ["dump", "--drive=/dev/sr0", "--retries=20", "--auto-detect", "--overwrite", "--image-path=/nonexistent/work/image", "--image-name=MY_DISC"]);
        // The drive's offset, when known, for the dump and its re-reads alike.
        let known = Options { read_offset: Some(-472), ..o.clone() };
        assert!(dump_args(&known, &t).contains(&"--drive-read-offset=-472".to_owned()));
        assert!(refine_args(&known, &t).contains(&"--drive-read-offset=-472".to_owned()));
        assert!(!dump_args(&o, &t).iter().any(|a| a.starts_with("--drive-read-offset")));
        // A sector order found or chosen, for both; redumper's own guess not asked for.
        let ordered = Options { sector_order: Some("DATA_SUB_C2".into()), ..o.clone() };
        assert!(dump_args(&ordered, &t).contains(&"--drive-sector-order=DATA_SUB_C2".to_owned()));
        assert!(!dump_args(&ordered, &t).contains(&"--auto-detect".to_owned()));
        assert!(refine_args(&ordered, &t).contains(&"--drive-sector-order=DATA_SUB_C2".to_owned()));
        assert!(!dump_args(&o, &t).iter().any(|a| a.starts_with("--drive-sector-order")));
        let o = Options { speed: Some(8), retries: 0, ..o };
        assert!(refine_args(&o, &t).contains(&"--retries=1".to_owned()), "refine with no retries does nothing");
        assert!(refine_args(&o, &t).contains(&"--speed=8".to_owned()));
        assert_eq!(split_args(&t, true)[..3], ["split", "--overwrite", "--force-split"]);
    }

    /// A short read's output, as redumper prints it, for a drive it does or
    /// doesn't know, in `order`, with these errors.
    fn probe_out(known: bool, order: &str, c2: u64, q: u64) -> String {
        let db = if known { "" } else { "warning: drive not found in the database\n" };
        let kind = if known { "PLEXTOR" } else { "GENERIC" };
        format!(
            "redumper (build: b744)\n{db}drive information\n  configuration: {kind} (read offset: +0, C2 shift: 0, pre-gap start: +0, read method: BE, sector order: {order})\n*** DUMP (time check: 0s)\n[100%] LBA:    300/300, errors: {{ SCSIs: 0, C2s: {c2}, Q: {q} }}\nmedia errors: \n  SCSI: 0 samples\n  C2: {c2} samples\n  Q: {q}\n*** END (time check: 3s)\n"
        )
    }

    /// The orders as the Lite-On eHAU324 reads a good disc in them.
    fn ehau324(order: Option<&str>) -> String {
        match order.unwrap_or("DATA_C2_SUB") {
            o @ "DATA_C2_SUB" => probe_out(false, o, 6140, 299),
            o @ ("DATA_SUB_C2" | "DATA_SUB") => probe_out(false, o, 0, 0),
            o => probe_out(false, o, 0, 299),
        }
    }

    async fn find_with(answer: impl Fn(Option<&str>, (u64, u64)) -> String) -> (Found, Vec<(Option<String>, (u64, u64))>, Vec<String>) {
        let tried = std::cell::RefCell::new(vec![]);
        let mut notes = vec![];
        let found = find_order(
            |o, r| {
                let out = answer(o.as_deref(), r);
                tried.borrow_mut().push((o, r));
                async move { Ok((out, true)) }
            },
            |n| notes.push(n),
        )
        .await
        .unwrap();
        (found, tried.into_inner(), notes)
    }

    #[test]
    fn a_short_read_s_errors_by_name() {
        let out = ehau324(None);
        assert!(unknown_drive(&out));
        assert_eq!(order_used(&out).as_deref(), Some("DATA_C2_SUB"));
        assert_eq!(probe_errors(&out).unwrap(), [("SCSI".to_owned(), 0), ("C2".to_owned(), 6140), ("Q".to_owned(), 299)]);
        assert!(!probe_clean(&out, true));
        assert!(probe_clean(&ehau324(Some("DATA_SUB_C2")), true));
        assert!(!probe_clean(&ehau324(Some("DATA_SUB_C2")), false), "one that stopped proves nothing");
        // Only a progress line: its counts.
        assert_eq!(probe_errors("[100%] LBA:    300/300, errors: { SCSIs: 0, C2s: 3, Q: 0 }").unwrap()[1], ("C2".to_owned(), 3));
        assert_eq!(probe_errors("error: no disc"), None);
        assert_eq!(order_line("DATA_SUB_C2", "found by testing"), "This drive hands over data, then subchannel, then C2 (found by testing)");
    }

    #[tokio::test]
    async fn the_order_is_found_by_testing_an_unknown_drive() {
        let (found, tried, notes) = find_with(|o, _| ehau324(o)).await;
        assert_eq!(found, Found::Order("DATA_SUB_C2".into()));
        // redumper's own guess first, then the best order, which read cleanly.
        assert_eq!(tried, [(None, (0, 300)), (Some("DATA_SUB_C2".into()), (0, 300))]);
        assert!(notes[0].contains("DATA_C2_SUB") && notes[0].contains("C2 6140, Q 299"), "{notes:?}");
        assert!(probe_args("/dev/sr0", Some("DATA_SUB_C2"), (0, 300), &Target { dir: "/t".into(), name: "test".into() }).starts_with(&[
            "dump".to_owned(),
            "--drive=/dev/sr0".into(),
            "--retries=0".into(),
            "--drive-sector-order=DATA_SUB_C2".into(),
            "--lba-start=0".into(),
            "--lba-end=300".into()
        ]));
    }

    #[tokio::test]
    async fn an_order_with_c2_is_preferred_and_one_without_is_the_last_resort() {
        // Only DATA_C2_SUB and DATA_SUB read cleanly: DATA_C2_SUB, with C2.
        let (found, _, _) = find_with(|o, _| match o.unwrap_or("DATA_C2") {
            o @ ("DATA_C2_SUB" | "DATA_SUB") => probe_out(false, o, 0, 0),
            o => probe_out(false, o, 10, 299),
        })
        .await;
        assert_eq!(found, Found::Order("DATA_C2_SUB".into()));
        // Only DATA_SUB: taken, once neither with C2 read cleanly.
        let (found, _, _) = find_with(|o, _| match o.unwrap_or("DATA_C2") {
            o @ "DATA_SUB" => probe_out(false, o, 0, 0),
            o => probe_out(false, o, 10, 299),
        })
        .await;
        assert_eq!(found, Found::Order("DATA_SUB".into()));
        // redumper's guess right: taken from its own read, with no other.
        let (found, tried, _) = find_with(|o, _| probe_out(false, o.unwrap_or("DATA_SUB_C2"), 0, 0)).await;
        assert_eq!((found, tried.len()), (Found::Order("DATA_SUB_C2".into()), 1));
    }

    #[tokio::test]
    async fn a_marked_start_is_tested_further_in_and_nothing_clean_is_said() {
        // Errors in the first sectors whatever the order: further in, DATA_SUB_C2.
        let (found, tried, _) = find_with(|o, r| probe_out(false, o.unwrap_or("DATA_C2_SUB"), if r.0 == 0 || o != Some("DATA_SUB_C2") { 5 } else { 0 }, 0)).await;
        assert_eq!(found, Found::Order("DATA_SUB_C2".into()));
        assert_eq!(tried.last().unwrap(), &(Some("DATA_SUB_C2".into()), PROBE_RANGES[1]));
        let (found, tried, _) = find_with(|o, _| probe_out(false, o.unwrap_or("DATA_C2_SUB"), 5, 5)).await;
        assert_eq!(found, Found::Nothing);
        // Every order over each stretch, redumper's guess standing for its own at the start.
        assert_eq!(tried.len(), TRY_ORDERS.len() * PROBE_RANGES.len());
    }

    #[tokio::test]
    async fn a_drive_redumper_knows_isn_t_tested() {
        let (found, tried, _) = find_with(|o, _| probe_out(true, o.unwrap_or("DATA_C2_SUB"), 0, 0)).await;
        assert_eq!((found, tried.len()), (Found::Known, 1));
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

    #[test]
    fn the_trail_puts_errors_where_the_counters_rose() {
        let mut t = ErrorTrail::new();
        let mut logged: Vec<String> = vec![];
        let mut feed = |l: &str| logged.extend(t.line(l));
        feed("*** DUMP (time check: 0s)");
        for (at, scsi, c2) in [(1000, 0, 0), (2000, 0, 0), (3000, 588, 10), (3100, 588, 30), (90_000, 588, 30), (90_100, 1176, 30), (200_000, 1176, 30)] {
            feed(&progress_line(at, 400_000, true, scsi, c2));
        }
        feed("media errors:");
        feed("SCSI: 1176 samples");
        // A re-read's counters aren't followed.
        feed("*** REFINE (time check: 0s)");
        feed(&progress_line(10, 400_000, true, 0, 0));
        feed(&progress_line(20, 400_000, true, 588, 0));
        assert_eq!(t.finish(), None);
        assert_eq!(logged, ["read errors at sectors 2000-3100 of 400000: SCSI 588, C2 30", "read errors at sectors 90000-90100 of 400000: SCSI 588"]);
        // passes() takes no notice of them.
        let log: Vec<&str> = ["*** DUMP (time check: 0s)", logged[0].as_str(), "media errors:", "SCSI: 588 samples", "C2: 30 samples"].to_vec();
        assert_eq!(passes(log.iter().copied())[0].errors, [("SCSI".to_owned(), 588), ("C2".to_owned(), 30)]);
    }

    #[test]
    fn the_map_from_rip_s_log() {
        let log = [
            "*** DUMP (time check: 0s)",
            "read errors at sectors 2000-3100 of 400000: SCSI 588, C2 30",
            "read errors at sectors 90000-90100 of 400000: SCSI 588",
            "*** INFO (time check: 0s)",
            "MSF errors: 40 (LBA: 20000, 20500, 98000-98001, 136000)",
            "ECC errors: 5 (LBA: 98000, 99000, 110000, 120000, 136000)",
            "EDC errors: 4 (LBA: 98000, 99000, 120000, 136000)",
            "*** END (time check: 1s)",
        ];
        let m = disc_map(log);
        assert_eq!(m.sectors, 400_000);
        assert_eq!(m.read, [Span { from: 2000, to: 3100, weight: 618 }, Span { from: 90_000, to: 90_100, weight: 588 }]);
        assert_eq!(m.wrong.len(), 7);
        assert_eq!(m.wrong[2], Span { from: 98_000, to: 98_001, weight: 2 });
        // Listed: 8 sectors; MSF says 40.
        assert_eq!(m.wrong_count, 40);
        assert!(disc_map(["*** DUMP"]).is_empty());
    }

    #[test]
    fn most_of_the_errors_are_where_they_bunch() {
        let one = |at: u64, w: u64| Span { from: at, to: at, weight: w };
        let mut spans: Vec<Span> = (0..20).map(|k| one(490 + k * 10, 1)).collect();
        spans.push(one(100, 1));
        spans.push(one(900, 1));
        let (a, b, whole) = most_between(&spans).unwrap();
        assert!(a >= 490 && b <= 680 && !whole, "{a}-{b}");
        assert_eq!(most_between(&[one(5, 1)]), Some((5, 5, true)));
        assert_eq!(most_between(&[]), None);
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
