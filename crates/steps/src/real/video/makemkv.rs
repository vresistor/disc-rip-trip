//! MakeMKV in robot mode (`-r`): what titles a disc has, which are
//! duplicates, whether it looks like a film or episodes, and ripping one
//! title. Lifted from the old app and tidied:
//! it now reads the audio and subtitle tracks too, and runs through the pty
//! like every other tool, so progress arrives as it happens.
//!
//! MakeMKV isn't bundled: it isn't free software. It's run only from where
//! you said it may be ([`approve`]), or as `DRT_MAKEMKVCON` if you set
//! that. The usual places are looked at for you to choose from, by
//! checking that files exist; nothing is run until you've said yes. From
//! inside this app's flatpak it's on the host, reached through
//! `flatpak-spawn --host`, which the flatpak may only use if you let it.

use super::super::tool;
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// MakeMKV's attribute ids (`apdefs.h`). Only the ones read here.
mod attr {
    pub const TYPE: u32 = 1;
    pub const NAME: u32 = 2;
    pub const LANG_CODE: u32 = 3;
    pub const LANG_NAME: u32 = 4;
    pub const CODEC_SHORT: u32 = 6;
    pub const CHAPTER_COUNT: u32 = 8;
    pub const DURATION: u32 = 9;
    pub const DISK_SIZE_BYTES: u32 = 11;
    pub const AUDIO_CHANNELS: u32 = 14;
    pub const SOURCE_FILE_NAME: u32 = 16;
    pub const VIDEO_SIZE: u32 = 19;
    pub const SEGMENTS_MAP: u32 = 26;
    pub const OUTPUT_FILE_NAME: u32 = 27;
    pub const VOLUME_NAME: u32 = 32;
    /// The DVD title set a title lives in (`C1`): a feature and its own
    /// bonus material usually share one.
    pub const TITLE_SET: u32 = 49;
}

/// What MakeMKV's stream type codes mean.
const STREAM_VIDEO: &str = "6201";
const STREAM_AUDIO: &str = "6202";
const STREAM_SUBTITLES: &str = "6203";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitles,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub kind: StreamKind,
    /// ISO 639-2, as MakeMKV gives it: `eng`.
    pub lang: String,
    pub lang_name: String,
    pub codec: String,
    pub channels: u32,
    /// `720x480`, for video.
    pub size: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Title {
    /// MakeMKV's number for it, from 0, under the `--minlength` it was
    /// scanned with.
    pub id: u32,
    pub name: String,
    pub secs: u32,
    pub bytes: u64,
    pub chapters: u32,
    pub source_file: String,
    pub output_file: String,
    /// Which cells it's made of. Two titles with the same map are the same
    /// content reached by different routes.
    pub segments: String,
    pub title_set: String,
    pub streams: Vec<Stream>,
}

impl Title {
    fn langs(&self, kind: StreamKind) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in self.streams.iter().filter(|s| s.kind == kind && !s.lang.is_empty()) {
            if !v.contains(&s.lang) {
                v.push(s.lang.clone());
            }
        }
        v
    }

    /// The audio languages, in the disc's order: `eng`, `fra`.
    pub fn audio(&self) -> Vec<String> {
        self.langs(StreamKind::Audio)
    }

    pub fn subtitles(&self) -> Vec<String> {
        self.langs(StreamKind::Subtitles)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Disc {
    /// `DVD`, `Blu-ray`, or whatever MakeMKV says.
    pub media: String,
    pub label: String,
    pub titles: Vec<Title>,
}

/// What a scan's output said, besides the disc.
#[derive(Debug, Default)]
pub struct Info {
    pub disc: Disc,
    pub unregistered: bool,
    pub too_old: bool,
    /// The last message that wasn't *Operation successfully completed*.
    pub last_msg: Option<String>,
}

/// MakeMKV refused to work for want of a key. The one failure the app can
/// fix by itself: beta keys expire about every two months.
#[derive(Debug)]
pub struct NotRegistered;

impl std::fmt::Display for NotRegistered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MakeMKV isn't registered")
    }
}

impl std::error::Error for NotRegistered {}

/// The build itself has expired. No key fixes that; a download does.
#[derive(Debug)]
pub struct TooOld;

impl std::fmt::Display for TooOld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MakeMKV is too old and needs updating (makemkv.com/download)")
    }
}

impl std::error::Error for TooOld {}

/// Nobody has said where MakeMKV may be run from.
#[derive(Debug)]
pub struct NotApproved;

impl std::fmt::Display for NotApproved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MakeMKV isn't set up: choose where it is in Disc Rip Trip first")
    }
}

impl std::error::Error for NotApproved {}

// ---- robot mode -------------------------------------------------------------------------------

/// Splits a robot-mode value list on commas, keeping quoted strings whole.
/// A quote inside one is doubled.
fn split_robot(values: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = values.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// `TYPE:a,b,c` into its tag and values.
pub fn parse_line(line: &str) -> Option<(&str, Vec<String>)> {
    let (tag, rest) = line.trim().split_once(':')?;
    if tag.is_empty() || !tag.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    Some((tag, split_robot(rest)))
}

/// `H:MM:SS` or `MM:SS` into seconds.
pub fn duration(s: &str) -> u32 {
    let parts: Vec<u32> = s.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [h, m, sec] => h * 3600 + m * 60 + sec,
        [m, sec] => m * 60 + sec,
        [sec] => *sec,
        _ => 0,
    }
}

/// Registration trouble rather than trouble with the disc. Matched on the
/// wording, which has been stable for years; the codes have moved.
pub fn is_registration_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["not registered", "registration key", "evaluation period"].iter().any(|n| lower.contains(n))
}

/// The program itself has expired: a download, not a key.
pub fn is_too_old(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("too old") || (lower.contains("expired") && lower.contains("version"))
}

/// 5011, *Operation successfully completed*: never the reason anything failed.
const SUCCESS: &str = "5011";

/// Everything `makemkvcon -r info` printed, read.
pub fn parse_info(text: &str) -> Info {
    let mut info = Info::default();
    let mut titles: std::collections::BTreeMap<u32, Title> = Default::default();
    let mut streams: std::collections::BTreeMap<(u32, u32), Stream> = Default::default();
    for line in text.lines() {
        let Some((tag, v)) = parse_line(line) else { continue };
        match tag {
            "MSG" if v.len() >= 4 => {
                let text = &v[3];
                if is_registration_failure(text) {
                    info.unregistered = true;
                } else if is_too_old(text) {
                    info.too_old = true;
                }
                if v[0] != SUCCESS {
                    info.last_msg = Some(text.clone());
                }
            }
            // CINFO:id,code,value
            "CINFO" if v.len() >= 3 => match v[0].parse::<u32>() {
                Ok(attr::TYPE) => info.disc.media = media(&v[2]),
                Ok(attr::VOLUME_NAME) => info.disc.label = v[2].clone(),
                Ok(attr::NAME) if info.disc.label.is_empty() => info.disc.label = v[2].clone(),
                _ => {}
            },
            // TINFO:title,id,code,value
            "TINFO" if v.len() >= 4 => {
                let (Ok(t), Ok(a)) = (v[0].parse::<u32>(), v[1].parse::<u32>()) else { continue };
                let e = titles.entry(t).or_insert_with(|| Title { id: t, output_file: format!("title_t{t:02}.mkv"), ..Default::default() });
                let value = &v[3];
                match a {
                    attr::NAME => e.name = value.clone(),
                    attr::DURATION => e.secs = duration(value),
                    attr::DISK_SIZE_BYTES => e.bytes = value.parse().unwrap_or(0),
                    attr::CHAPTER_COUNT => e.chapters = value.parse().unwrap_or(0),
                    attr::SOURCE_FILE_NAME => e.source_file = value.clone(),
                    attr::OUTPUT_FILE_NAME => e.output_file = value.clone(),
                    attr::SEGMENTS_MAP => e.segments = value.clone(),
                    attr::TITLE_SET => e.title_set = value.clone(),
                    _ => {}
                }
            }
            // SINFO:title,stream,id,code,value
            "SINFO" if v.len() >= 5 => {
                let (Ok(t), Ok(s), Ok(a)) = (v[0].parse::<u32>(), v[1].parse::<u32>(), v[2].parse::<u32>()) else { continue };
                let e =
                    streams.entry((t, s)).or_insert_with(|| Stream { kind: StreamKind::Other, lang: String::new(), lang_name: String::new(), codec: String::new(), channels: 0, size: String::new() });
                let (code, value) = (&v[3], &v[4]);
                match a {
                    attr::TYPE => {
                        e.kind = match (code.as_str(), value.as_str()) {
                            (STREAM_VIDEO, _) | (_, "Video") => StreamKind::Video,
                            (STREAM_AUDIO, _) | (_, "Audio") => StreamKind::Audio,
                            (STREAM_SUBTITLES, _) | (_, "Subtitles") => StreamKind::Subtitles,
                            _ => StreamKind::Other,
                        }
                    }
                    attr::LANG_CODE => e.lang = value.clone(),
                    attr::LANG_NAME => e.lang_name = value.clone(),
                    attr::CODEC_SHORT => e.codec = value.clone(),
                    attr::AUDIO_CHANNELS => e.channels = value.parse().unwrap_or(0),
                    attr::VIDEO_SIZE => e.size = value.clone(),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    for ((t, _), s) in streams {
        if let Some(title) = titles.get_mut(&t) {
            title.streams.push(s);
        }
    }
    info.disc.titles = titles.into_values().collect();
    info
}

/// `DVD disc` → `DVD`, `Blu-ray disc` → `Blu-ray`.
fn media(s: &str) -> String {
    let lower = s.to_lowercase();
    if lower.contains("blu-ray") || lower.contains("bluray") {
        "Blu-ray".into()
    } else if lower.contains("hd-dvd") || lower.contains("hd dvd") {
        "HD DVD".into()
    } else if lower.contains("dvd") {
        "DVD".into()
    } else {
        s.trim_end_matches(" disc").to_owned()
    }
}

// ---- what's on the disc ---------------------------------------------------------------------------

/// The same content reached another way.
fn same_content(a: &Title, b: &Title) -> bool {
    (!a.segments.is_empty() && a.segments == b.segments) || (a.secs == b.secs && a.secs > 0 && a.bytes.abs_diff(b.bytes) < 8 * 1024 * 1024)
}

/// Each title that duplicates an earlier one, mapped to that one's id.
/// DVDs routinely offer one episode two or three ways.
pub fn duplicates(titles: &[Title]) -> std::collections::HashMap<u32, u32> {
    let mut first: Vec<&Title> = Vec::new();
    let mut map = std::collections::HashMap::new();
    for t in titles {
        match first.iter().find(|c| same_content(c, t)) {
            Some(c) => {
                map.insert(t.id, c.id);
            }
            None => first.push(t),
        }
    }
    map
}

fn unique(titles: &[Title]) -> Vec<Title> {
    let dups = duplicates(titles);
    titles.iter().filter(|t| !dups.contains_key(&t.id)).cloned().collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Film,
    Series,
}

/// Shortest thing believed to be an episode rather than an extra.
const EPISODE_MIN: u32 = 15 * 60;
/// Longest: generous, since feature-length pilots are common.
const EPISODE_MAX: u32 = 125 * 60;
/// Above this, two similar titles are more likely two cuts of a film.
const TYPICAL_EPISODE_MAX: u32 = 75 * 60;

fn episode_candidates(titles: &[Title]) -> Vec<Title> {
    unique(titles).into_iter().filter(|t| (EPISODE_MIN..=EPISODE_MAX).contains(&t.secs)).collect()
}

/// Similar-length titles, largest group first.
fn clusters(titles: &[Title]) -> Vec<Vec<Title>> {
    let mut sorted = titles.to_vec();
    sorted.sort_by_key(|t| t.secs);
    let mut out: Vec<Vec<Title>> = Vec::new();
    for t in sorted {
        let at = out.iter_mut().find(|c| {
            let median = c[c.len() / 2].secs as f32;
            (t.secs as f32 - median).abs() / median.max(1.0) <= 0.15
        });
        match at {
            Some(c) => c.push(t),
            None => out.push(vec![t]),
        }
    }
    out.sort_by_key(|c| std::cmp::Reverse((c.len(), c[0].secs)));
    out
}

/// Whether a disc holds a film or episodes. The tell is a group of
/// similar-length titles where episodes sit; a film has one long title and
/// short extras. A label with a season or disc number (`DISC_1`) settles a
/// pair, but never outweighs a feature twice as long as the group.
pub fn guess_kind(titles: &[Title], label_says_series: bool) -> Kind {
    let candidates = episode_candidates(titles);
    let Some(group) = clusters(&candidates).into_iter().next() else { return Kind::Film };
    let median = group[group.len() / 2].secs;
    // A play-all title isn't among the candidates (it's longer than any
    // episode), so it can't pass for a dominant feature here.
    if let Some(longest) = candidates.iter().map(|t| t.secs).max() {
        if median > 0 && longest as f32 >= median as f32 * 2.0 && longest >= 70 * 60 {
            return Kind::Film;
        }
    }
    if group.len() >= 3 {
        return Kind::Series;
    }
    if group.len() == 2 && (group.iter().all(|t| t.secs <= TYPICAL_EPISODE_MAX) || label_says_series) {
        return Kind::Series;
    }
    Kind::Film
}

/// The titles worth ripping, in disc order: the longest for a film; the
/// biggest group of similar lengths for episodes, without a play-all.
pub fn select(titles: &[Title], kind: Kind) -> Vec<Title> {
    let longest = || unique(titles).into_iter().max_by_key(|t| t.secs).into_iter().collect::<Vec<_>>();
    match kind {
        Kind::Film => longest(),
        Kind::Series => {
            let mut chosen = clusters(&episode_candidates(titles)).into_iter().next().unwrap_or_else(longest);
            let total: u32 = chosen.iter().map(|t| t.secs).sum();
            if chosen.len() > 1 {
                chosen.retain(|t| (t.secs as f32) < total as f32 * 0.75);
            }
            chosen.sort_by_key(|t| t.id);
            chosen
        }
    }
}

// ---- running it -----------------------------------------------------------------------------------

/// How makemkvcon is started here.
#[derive(Clone, Debug, PartialEq)]
pub enum Launcher {
    /// A program and its first arguments, run as they are: an approved
    /// install, or `DRT_MAKEMKVCON`.
    Direct(Vec<String>),
    /// MakeMKV's own flatpak, which can only write where it's let: each run
    /// is let into the folder it writes to.
    Flatpak { via_host: bool },
    /// makemkvcon at this path on the host, from inside this app's flatpak.
    Host(PathBuf),
}

pub const FLATPAK_ID: &str = "com.makemkv.MakeMKV";

/// Where MakeMKV is, as you chose it. Saved by the app.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Location {
    /// makemkvcon at this path: on this computer, or on the host from
    /// inside this app's flatpak.
    Program { path: PathBuf },
    /// MakeMKV's own flatpak, `com.makemkv.MakeMKV`.
    Flatpak,
}

impl Location {
    /// How it's shown: the path, or *MakeMKV's flatpak*.
    pub fn label(&self) -> String {
        match self {
            Location::Program { path } => path.display().to_string(),
            Location::Flatpak => format!("MakeMKV's flatpak ({FLATPAK_ID})"),
        }
    }

    /// Whether running it from here needs `flatpak-spawn --host`.
    pub fn needs_host(&self, sandboxed: bool) -> bool {
        sandboxed
    }
}

impl Launcher {
    /// How an approved location is run, from inside a flatpak or not.
    pub fn of(loc: &Location, sandboxed: bool) -> Launcher {
        match (loc, sandboxed) {
            (Location::Program { path }, false) => Launcher::Direct(vec![path.display().to_string()]),
            (Location::Program { path }, true) => Launcher::Host(path.clone()),
            (Location::Flatpak, via_host) => Launcher::Flatpak { via_host },
        }
    }
}

/// Where you said MakeMKV may be run from. Set by the app from its saved
/// setting; nothing else sets it.
static APPROVED: std::sync::RwLock<Option<Location>> = std::sync::RwLock::new(None);

/// Whether the free beta key may be fetched and written into MakeMKV's
/// settings when it runs out. Off unless you turn it on.
static KEY_UPDATES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Says where MakeMKV may be run from (or, `None`, that it may not).
pub fn approve(loc: Option<Location>) {
    *APPROVED.write().unwrap() = loc;
}

pub fn approved() -> Option<Location> {
    APPROVED.read().unwrap().clone()
}

pub fn set_key_updates(on: bool) {
    KEY_UPDATES.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn key_updates() -> bool {
    KEY_UPDATES.load(std::sync::atomic::Ordering::Relaxed)
}

/// `DRT_MAKEMKVCON`, a whole command line: set by you, so it counts as
/// your OK.
pub fn from_env() -> Option<Vec<String>> {
    let line = std::env::var("DRT_MAKEMKVCON").ok()?;
    let parts: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
    (!parts.is_empty()).then_some(parts)
}

/// The only way to a [`Launcher`]: `DRT_MAKEMKVCON`, else where you said.
/// `None` when you haven't said, and then MakeMKV is never run.
pub fn launcher() -> Option<Launcher> {
    if let Some(parts) = from_env() {
        return Some(Launcher::Direct(parts));
    }
    approved().map(|l| Launcher::of(&l, sandboxed()))
}

/// Whether this is running inside a flatpak.
pub fn sandboxed() -> bool {
    cfg!(target_os = "linux") && Path::new("/.flatpak-info").is_file()
}

/// Whether the flatpak may start programs on the host: its
/// `[Session Bus Policy]` lets it talk to `org.freedesktop.Flatpak`. Read
/// from the file flatpak writes when the app starts, so a change made
/// with `flatpak override` shows here only after the app is reopened.
pub fn host_allowed(flatpak_info: &str) -> bool {
    let mut in_policy = false;
    for line in flatpak_info.lines().map(str::trim) {
        if line.starts_with('[') {
            in_policy = line == "[Session Bus Policy]";
        } else if in_policy {
            if let Some((name, level)) = line.split_once('=') {
                if name.trim() == "org.freedesktop.Flatpak" && matches!(level.trim(), "talk" | "own") {
                    return true;
                }
            }
        }
    }
    false
}

/// [`host_allowed`] for this app, as it was started; true outside a flatpak.
pub fn host_allowed_here() -> bool {
    !sandboxed() || std::fs::read_to_string("/.flatpak-info").is_ok_and(|t| host_allowed(&t))
}

/// A command on the host when sandboxed, here otherwise.
fn host(bin: &str) -> tokio::process::Command {
    let mut cmd = if sandboxed() {
        let mut c = tokio::process::Command::new("flatpak-spawn");
        // Ties the host process to this one: if the app goes, so does it.
        c.args(["--host", "--watch-bus", bin]);
        c
    } else {
        tokio::process::Command::new(bin)
    };
    cmd.kill_on_drop(true);
    cmd
}

async fn quietly(mut cmd: tokio::process::Command) -> bool {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.status().await.map(|s| s.success()).unwrap_or(false)
}

/// Where each platform's installer leaves makemkvcon.
fn install_candidates() -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![PathBuf::from("/Applications/MakeMKV.app/Contents/MacOS/makemkvcon")]
    } else if cfg!(target_os = "windows") {
        ["ProgramFiles(x86)", "ProgramFiles"].iter().filter_map(std::env::var_os).map(|r| PathBuf::from(r).join("MakeMKV").join("makemkvcon64.exe")).collect()
    } else {
        vec![PathBuf::from("/usr/bin/makemkvcon"), PathBuf::from("/usr/local/bin/makemkvcon"), PathBuf::from("/opt/makemkv/bin/makemkvcon")]
    }
}

/// Where to look, as plain data, so a test can point it at its own folders.
#[derive(Clone, Debug, Default)]
pub struct Places {
    /// Folders a makemkvcon may be in, as the computer that runs it sees them.
    pub dirs: Vec<PathBuf>,
    /// Whole paths an installer leaves it at.
    pub programs: Vec<PathBuf>,
    /// Folders that exist when MakeMKV's flatpak is installed.
    pub flatpak: Vec<PathBuf>,
    /// Where this app sees the host's `/usr` (`/run/host` inside a
    /// flatpak, nothing outside).
    pub host_root: Option<PathBuf>,
}

impl Places {
    /// This computer's: the PATH and the usual places, or inside the
    /// flatpak, the host's usual places.
    pub fn here() -> Places {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let bin = if cfg!(windows) { "makemkvcon64.exe" } else { "makemkvcon" };
        let mut dirs: Vec<PathBuf> = if sandboxed() {
            // The sandbox's PATH isn't the host's: the host's usual ones.
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/usr/local/bin")]
        } else {
            std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
        };
        if let Some(h) = &home {
            let local = h.join(".local/bin");
            if !dirs.contains(&local) {
                dirs.push(local);
            }
        }
        let mut flatpak = vec![PathBuf::from(format!("/var/lib/flatpak/app/{FLATPAK_ID}/current"))];
        if let Some(h) = &home {
            flatpak.insert(0, h.join(format!(".local/share/flatpak/app/{FLATPAK_ID}/current")));
        }
        Places {
            dirs: dirs.into_iter().map(|d| d.join(bin)).collect(),
            programs: install_candidates(),
            flatpak: if cfg!(target_os = "linux") { flatpak } else { vec![] },
            host_root: sandboxed().then(|| PathBuf::from("/run/host")),
        }
    }

    /// Where this app can see a host path: the host's `/usr` is under
    /// `/run/host` inside a flatpak; the rest is where it is.
    fn visible(&self, p: &Path) -> PathBuf {
        match &self.host_root {
            Some(root) if ["/usr", "/bin", "/sbin", "/lib"].iter().any(|top| p.starts_with(top)) => root.join(p.strip_prefix("/").unwrap_or(p)),
            _ => p.to_owned(),
        }
    }
}

fn runnable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        p.metadata().is_ok_and(|m| m.is_file() && std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// The MakeMKVs that seem to be installed, for you to choose from. Only
/// looks at whether files exist: nothing is run.
pub fn candidates(places: &Places) -> Vec<Location> {
    let mut out: Vec<Location> = Vec::new();
    for p in places.dirs.iter().chain(&places.programs) {
        let seen = places.visible(p);
        // The same program by another route (/bin is /usr/bin) is listed once.
        let real = std::fs::canonicalize(&seen).unwrap_or(seen.clone());
        let dup = out.iter().any(|l| matches!(l, Location::Program { path } if std::fs::canonicalize(places.visible(path)).unwrap_or(places.visible(path)) == real));
        if runnable(&seen) && !dup {
            out.push(Location::Program { path: p.clone() });
        }
    }
    if places.flatpak.iter().any(|d| d.is_dir()) {
        out.push(Location::Flatpak);
    }
    out
}

/// A file chosen in the picker, as a location. From inside the flatpak, a
/// host path shown under `/run/host` is given as the host's own.
pub fn chosen_file(p: &Path) -> Result<Location> {
    let path = match p.strip_prefix("/run/host") {
        Ok(rest) => Path::new("/").join(rest),
        Err(_) => p.to_owned(),
    };
    if p.starts_with("/run/user") && p.components().any(|c| c.as_os_str() == "doc") {
        bail!("that file came through the file chooser's own folder, which can't be run; choose makemkvcon from its real place, such as /usr/bin");
    }
    if !runnable(p) {
        bail!("{} isn't a program that can be run", path.display());
    }
    Ok(Location::Program { path })
}

/// The version text in what makemkvcon printed: `MakeMKV v1.17.7`.
pub fn version_in(text: &str) -> Option<String> {
    let i = text.find("MakeMKV v")?;
    let v: String = text[i + "MakeMKV v".len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '.').collect();
    (!v.is_empty()).then(|| format!("MakeMKV v{v}"))
}

/// Runs makemkvcon once, to say which version it is: only for a location
/// you've just approved. It's asked for nothing a disc is needed for.
pub async fn version(launcher: &Launcher, dir: &Path) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    let mut cmd = launcher.command(dir);
    cmd.arg("--version").current_dir(dir).stdin(Stdio::null());
    let out =
        tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await.map_err(|_| anyhow!("it didn't answer within 30 seconds"))?.map_err(|e| anyhow!("couldn't start it: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if let Some(v) = version_in(&text) {
        return Ok(v);
    }
    match tool::last_meaningful_line(&text) {
        Some(l) if out.status.success() => Ok(l.chars().take(80).collect()),
        Some(l) => bail!("it didn't run: {}", l.chars().take(160).collect::<String>()),
        None if out.status.success() => Ok("version not given".into()),
        None => bail!("it didn't run ({})", out.status),
    }
}

impl Launcher {
    /// makemkvcon, let write into `dir` if it's sandboxed.
    pub fn command(&self, dir: &Path) -> tokio::process::Command {
        let flatpak_args = |c: &mut tokio::process::Command| {
            // --die-with-parent: killing `flatpak run` alone leaves the payload running.
            c.args(["run", "--die-with-parent", &format!("--filesystem={}", dir.display()), "--command=makemkvcon", FLATPAK_ID]);
        };
        match self {
            Launcher::Direct(parts) => {
                let mut c = tokio::process::Command::new(&parts[0]);
                c.args(&parts[1..]);
                c.kill_on_drop(true);
                c
            }
            Launcher::Host(path) => host(&path.display().to_string()),
            Launcher::Flatpak { via_host } => {
                let mut c = if *via_host { host("flatpak") } else { tokio::process::Command::new("flatpak") };
                c.kill_on_drop(true);
                flatpak_args(&mut c);
                c
            }
        }
    }

    /// After a Stop: killing what we started only kills a launcher when
    /// there is one (`flatpak-spawn`, `flatpak run`), and the makemkvcon
    /// behind it keeps the drive. So it's found by what it's reading.
    pub async fn kill(&self, device: &str) {
        if matches!(self, Launcher::Direct(_)) {
            return;
        }
        let mut c = host("pkill");
        c.args(["-f", &format!("makemkvcon.*dev:{device}")]);
        quietly(c).await;
    }
}

/// Lists a disc's titles. `minlength` hides titles shorter than it, and
/// the rip has to use the same one: MakeMKV numbers titles within what it
/// was asked to list.
pub async fn scan(launcher: &Launcher, device: &str, minlength: u32, dir: &Path, stop: impl std::future::Future<Output = ()>) -> Result<Disc> {
    std::fs::create_dir_all(dir)?;
    let mut cmd = launcher.command(dir);
    cmd.args(["-r", "--cache=1", &format!("--minlength={minlength}"), "info", &format!("dev:{device}")]);
    let stop_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop_flag.clone();
    let r = tool::run(
        cmd,
        dir,
        async move {
            stop.await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        },
        |_| {},
    )
    .await;
    let (text, status) = match r {
        Ok(x) => x,
        Err(e) => {
            if stop_flag.load(std::sync::atomic::Ordering::SeqCst) {
                launcher.kill(device).await;
            }
            return Err(e);
        }
    };
    let info = parse_info(&text);
    // Before "no titles": an unregistered or expired MakeMKV finds none either.
    if info.unregistered {
        bail!(NotRegistered);
    }
    if info.too_old {
        bail!(TooOld);
    }
    if !status.success() {
        bail!("MakeMKV failed: {}", info.last_msg.or_else(|| tool::last_meaningful_line(&text).map(str::to_owned)).unwrap_or_else(|| status.to_string()));
    }
    if info.disc.titles.is_empty() {
        bail!("MakeMKV found no videos: {}", info.last_msg.unwrap_or_else(|| "it said nothing".into()));
    }
    Ok(info.disc)
}

/// What MakeMKV said about a rip. Its exit code isn't the verdict: it
/// "always returns 0 as exit code, except for fatal errors", so a title
/// that fails with read errors leaves a partial file and exits 0. The
/// messages are where the truth is: 5003 *Failed to save title*, and the
/// 5004/5037 summary's failed count.
#[derive(Debug, Default)]
pub struct RipReport {
    last_error: Option<String>,
    last_msg: Option<String>,
    pub unregistered: bool,
    pub too_old: bool,
    failed: bool,
}

impl RipReport {
    /// One `MSG:` line's values: code, flags, count, text, ….
    pub fn note(&mut self, v: &[String]) {
        if v.len() < 4 {
            return;
        }
        let (code, text) = (v[0].as_str(), &v[3]);
        if code != SUCCESS {
            self.last_msg = Some(text.clone());
        }
        match code {
            "5003" | "5010" => {
                self.last_error = Some(text.clone());
                self.failed |= code == "5003";
            }
            "5004" | "5037" if failed_count(text).unwrap_or(0) > 0 => {
                self.failed = true;
                self.last_error.get_or_insert_with(|| text.clone());
            }
            _ => {}
        }
        if is_registration_failure(text) {
            self.unregistered = true;
        } else if is_too_old(text) {
            self.too_old = true;
        }
    }

    pub fn verdict(&self, exited_ok: bool) -> Result<()> {
        if self.unregistered {
            bail!(NotRegistered);
        }
        if self.too_old {
            bail!(TooOld);
        }
        if self.failed || !exited_ok {
            let why = self.last_error.clone().or_else(|| self.last_msg.clone().map(|m| format!("MakeMKV failed: {m}"))).unwrap_or_else(|| "MakeMKV failed and said nothing".into());
            bail!(why);
        }
        Ok(())
    }
}

/// The M in "N titles saved, M failed".
fn failed_count(text: &str) -> Option<u32> {
    let (_, after) = text.split_once("saved,")?;
    after.split_whitespace().next()?.parse().ok()
}

/// Rips one title into `dir`, which should be empty, giving progress 0 to
/// 1 and what MakeMKV is doing. Gives the file.
pub async fn rip(launcher: &Launcher, device: &str, id: u32, minlength: u32, dir: &Path, stop: impl std::future::Future<Output = ()>, mut progress: impl FnMut(f32, &str)) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let mut cmd = launcher.command(dir);
    cmd.args(["-r", "--progress=-same", "--noscan", &format!("--minlength={minlength}"), "mkv", &format!("dev:{device}"), &id.to_string()]).arg(dir);
    let mut report = RipReport::default();
    let mut doing = String::from("Starting");
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stopped.clone();
    let r = tool::run(
        cmd,
        dir,
        async move {
            stop.await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        },
        |line| {
            let Some((tag, v)) = parse_line(line) else { return };
            match tag {
                "PRGC" if v.len() >= 3 => doing = v[2].clone(),
                // current, total, max: the whole job's progress is total/max.
                "PRGV" if v.len() >= 3 => {
                    let (total, max) = (v[1].parse::<f32>().unwrap_or(0.0), v[2].parse::<f32>().unwrap_or(0.0));
                    if max > 0.0 {
                        progress((total / max).clamp(0.0, 1.0), &doing);
                    }
                }
                "MSG" => report.note(&v),
                _ => {}
            }
        },
    )
    .await;
    let status = match r {
        Ok((_, s)) => s,
        Err(e) => {
            if stopped.load(std::sync::atomic::Ordering::SeqCst) {
                launcher.kill(device).await;
            }
            return Err(e);
        }
    };
    report.verdict(status.success())?;
    let mkvs: Vec<PathBuf> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "mkv")).collect();
    match mkvs.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(anyhow!("MakeMKV finished but wrote no file")),
        _ => Err(anyhow!("MakeMKV wrote {} files for one video", mkvs.len())),
    }
}

// ---- is the file whole? ---------------------------------------------------------------------------

/// How long a file is, by two accounts that can disagree: the container's
/// (what a player's seek bar shows) and the video stream's own statistics
/// tag, written only when muxing finishes. *Zulu* was filed at 2h12m by
/// the container with two minutes of picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lengths {
    pub container: f64,
    pub video: Option<f64>,
}

/// The two out of `ffprobe -show_entries format=duration:stream_tags -of json`.
pub fn parse_lengths(json: &str) -> Option<Lengths> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let container: f64 = v.get("format")?.get("duration")?.as_str()?.trim().parse().ok()?;
    let video = v
        .get("streams")
        .and_then(|s| s.as_array())
        .and_then(|s| s.first())
        .and_then(|s| s.get("tags"))
        .and_then(|t| t.as_object())
        .and_then(|tags| tags.iter().find(|(k, _)| k.as_str() == "DURATION" || k.starts_with("DURATION-")).and_then(|(_, v)| v.as_str()).and_then(timestamp));
    Some(Lengths { container, video })
}

/// `HH:MM:SS.nnnnnnnnn` into seconds.
fn timestamp(s: &str) -> Option<f64> {
    let mut p = s.trim().split(':');
    let (h, m, sec): (f64, f64, f64) = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?);
    p.next().is_none().then_some(h * 3600.0 + m * 60.0 + sec)
}

/// Whether a file is the whole title: each length it has within 2% or 15 s
/// of what the disc said. A file cut off mid-write has no video tag and is
/// judged on the container.
pub fn whole(expected: u32, l: Lengths) -> Result<()> {
    let tolerance = (expected as f64 * 0.02).max(15.0);
    let hms = |s: f64| format!("{}:{:02}:{:02}", s as u64 / 3600, s as u64 / 60 % 60, s as u64 % 60);
    for (what, got) in [("the file", Some(l.container)), ("its picture", l.video)] {
        if let Some(got) = got {
            if (got - expected as f64).abs() > tolerance {
                bail!("{what} runs {} but the video is {}", hms(got), hms(expected as f64));
            }
        }
    }
    Ok(())
}

/// Probes a file and checks it's the whole title.
pub async fn check_whole(file: &Path, expected: u32) -> Result<()> {
    if expected == 0 {
        return Ok(());
    }
    let mut cmd = tool::command("ffprobe");
    cmd.args(["-v", "error", "-select_streams", "v:0", "-show_entries", "format=duration:stream_tags", "-of", "json", "-i"]).arg(file);
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    let out = cmd.output().await.map_err(|e| anyhow!("couldn't run ffprobe: {e}"))?;
    let lengths = parse_lengths(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| anyhow!("couldn't read how long {} is", file.display()))?;
    whole(expected, lengths)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A film disc in robot mode, shaped like makemkvcon's own output: the
    /// feature, a duplicate of it by another route, a trailer.
    pub const FILM: &str = r#"MSG:1005,0,1,"MakeMKV v1.17.7 linux(x64-release) started","%1 started","MakeMKV v1.17.7 linux(x64-release)"
DRV:0,2,999,1,"BD-RE MATSHITA BD-MLT UJ240AS","BLADE_RUNNER","/dev/sr0"
DRV:1,256,999,0,"","",""
TCOUNT:3
CINFO:1,6206,"DVD disc"
CINFO:2,0,"Blade Runner"
CINFO:32,0,"BLADE_RUNNER"
TINFO:0,2,0,"Blade Runner"
TINFO:0,8,0,"34"
TINFO:0,9,0,"1:57:30"
TINFO:0,10,0,"7.1 GB"
TINFO:0,11,0,"7623458816"
TINFO:0,16,0,"VTS_01_1.VOB"
TINFO:0,26,0,"1-34"
TINFO:0,27,0,"Blade_Runner_t00.mkv"
SINFO:0,0,1,6201,"Video"
SINFO:0,0,6,0,"Mpeg2"
SINFO:0,0,19,0,"720x480"
SINFO:0,1,1,6202,"Audio"
SINFO:0,1,3,0,"eng"
SINFO:0,1,4,0,"English"
SINFO:0,1,6,0,"DD"
SINFO:0,1,14,0,"6"
SINFO:0,2,1,6202,"Audio"
SINFO:0,2,3,0,"fra"
SINFO:0,2,4,0,"French"
SINFO:0,2,6,0,"DD"
SINFO:0,2,14,0,"2"
SINFO:0,3,1,6203,"Subtitles"
SINFO:0,3,3,0,"eng"
SINFO:0,3,4,0,"English"
SINFO:0,4,1,6203,"Subtitles"
SINFO:0,4,3,0,"spa"
TINFO:1,8,0,"34"
TINFO:1,9,0,"1:57:30"
TINFO:1,11,0,"7623458816"
TINFO:1,26,0,"1-34"
TINFO:1,27,0,"Blade_Runner_t01.mkv"
SINFO:1,0,1,6201,"Video"
TINFO:2,8,0,"1"
TINFO:2,9,0,"0:03:10"
TINFO:2,11,0,"190000000"
TINFO:2,26,0,"40"
SINFO:2,0,1,6201,"Video"
MSG:5011,0,0,"Operation successfully completed","Operation successfully completed"
"#;

    #[test]
    fn a_scan_gives_the_titles_with_their_tracks() {
        let info = parse_info(FILM);
        assert!(!info.unregistered && !info.too_old);
        assert_eq!(info.last_msg.as_deref(), Some("MakeMKV v1.17.7 linux(x64-release) started"), "the success message is never kept");
        let d = info.disc;
        assert_eq!((d.media.as_str(), d.label.as_str(), d.titles.len()), ("DVD", "BLADE_RUNNER", 3));
        let t = &d.titles[0];
        assert_eq!((t.secs, t.chapters, t.bytes, t.output_file.as_str()), (7050, 34, 7_623_458_816, "Blade_Runner_t00.mkv"));
        assert_eq!(t.audio(), ["eng", "fra"]);
        assert_eq!(t.subtitles(), ["eng", "spa"]);
        assert_eq!(t.streams[0].size, "720x480");
        assert_eq!(t.streams[1].channels, 6);
        assert_eq!(d.titles[2].output_file, "title_t02.mkv", "MakeMKV's name when it gave none");
        assert_eq!(duplicates(&d.titles).get(&1), Some(&0));
        assert_eq!(guess_kind(&d.titles, false), Kind::Film);
        assert_eq!(select(&d.titles, Kind::Film).iter().map(|t| t.id).collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn a_blu_ray_and_an_unregistered_makemkv() {
        let info = parse_info("CINFO:1,6209,\"Blu-ray disc\"\nMSG:5021,260,0,\"This application is not registered, please enter a registration key\",\"x\"\n");
        assert_eq!(info.disc.media, "Blu-ray");
        assert!(info.unregistered);
        let info = parse_info("MSG:5021,0,0,\"This version of MakeMKV is too old, please download a new version\",\"%1\"\n");
        assert!(info.too_old && !info.unregistered);
    }

    #[test]
    fn registration_and_age_are_told_apart_from_disc_errors() {
        assert!(is_registration_failure("Evaluation period has expired"));
        assert!(!is_registration_failure("Failed to open disc"));
        assert!(!is_registration_failure("This version of MakeMKV is too old, please download a new version"));
        assert!(is_too_old("This application version has expired, please download a new version"));
        assert!(!is_too_old("Evaluation period has expired"));
    }

    fn msg(line: &str) -> Vec<String> {
        parse_line(line).unwrap().1
    }

    #[test]
    fn a_failed_title_fails_even_when_makemkv_exits_zero() {
        let mut r = RipReport::default();
        r.note(&msg(r#"MSG:5003,0,2,"Failed to save title 3 to file /tmp/x.mkv","Failed to save title %1 to file %2","3","/tmp/x.mkv""#));
        r.note(&msg(r#"MSG:5037,516,2,"Copy complete. 0 titles saved, 1 failed.","Copy complete. %1 titles saved, %2 failed.","0","1""#));
        assert!(r.verdict(true).unwrap_err().to_string().contains("Failed to save title 3"));
        let mut r = RipReport::default();
        r.note(&msg(r#"MSG:5037,516,2,"Copy complete. 1 titles saved, 1 failed.","x","1","1""#));
        assert!(r.verdict(true).is_err(), "the summary alone is enough");
    }

    #[test]
    fn a_clean_rip_passes_and_success_is_never_the_reason() {
        let mut r = RipReport::default();
        r.note(&msg(r#"MSG:5037,0,1,"Copy complete. 1 titles saved.","Copy complete. %1 titles saved.","1""#));
        r.note(&msg(r#"MSG:5011,0,0,"Operation successfully completed","Operation successfully completed""#));
        assert!(r.verdict(true).is_ok());
        assert!(!r.verdict(false).unwrap_err().to_string().contains("successfully"));
        let mut r = RipReport::default();
        r.note(&msg(r#"MSG:5021,0,0,"This version of MakeMKV is too old, please download a new version","%1""#));
        assert!(r.verdict(false).unwrap_err().downcast_ref::<TooOld>().is_some());
    }

    #[test]
    fn robot_values_and_durations() {
        let v = split_robot(r#"0,0,999,0,"BD-RE, fancy","Say ""hi""",/dev/sr0"#);
        assert_eq!((v[4].as_str(), v[5].as_str(), v[6].as_str()), ("BD-RE, fancy", "Say \"hi\"", "/dev/sr0"));
        assert_eq!(duration("1:32:45"), 5565);
        assert_eq!(duration("22:31"), 1351);
        assert_eq!(failed_count("Copy complete. 3 titles saved, 2 failed."), Some(2));
        assert_eq!(failed_count("1 titles saved."), None);
        assert!(parse_line("Using direct disc access mode").is_none());
    }

    fn title(id: u32, secs: u32) -> Title {
        Title { id, secs, bytes: secs as u64 * 1_000_000, chapters: 5, segments: format!("seg-{id}"), ..Default::default() }
    }

    #[test]
    fn episodes_are_the_group_without_play_all_or_extras() {
        let titles = vec![title(0, 2640), title(1, 2700), title(2, 2610), title(3, 2680), title(4, 10630), title(5, 240)];
        assert_eq!(guess_kind(&titles, false), Kind::Series);
        assert_eq!(select(&titles, Kind::Series).iter().map(|t| t.id).collect::<Vec<_>>(), [0, 1, 2, 3]);
    }

    #[test]
    fn a_film_with_featurettes_is_not_a_series_whatever_the_label() {
        let titles = vec![title(0, 6600), title(1, 1200), title(2, 1180), title(3, 1210)];
        assert_eq!(guess_kind(&titles, true), Kind::Film);
    }

    #[test]
    fn feature_length_pilots_need_the_label() {
        // Twin Peaks Gold Box disc 1.
        let titles = vec![title(0, 5405), title(1, 5213), title(2, 77), title(3, 1277), title(4, 72)];
        assert_eq!(guess_kind(&titles, false), Kind::Film);
        assert_eq!(guess_kind(&titles, true), Kind::Series);
        assert_eq!(select(&titles, Kind::Series).iter().map(|t| t.id).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn a_rip_is_whole_only_if_both_lengths_agree_with_the_disc() {
        // What ffprobe says of a MakeMKV rip: the tag carries the language.
        let json = r#"{"streams":[{"tags":{"language":"eng","DURATION-eng":"02:12:39.000000000"}}],"format":{"duration":"7959.000000"}}"#;
        let l = parse_lengths(json).unwrap();
        assert_eq!(l, Lengths { container: 7959.0, video: Some(7959.0) });
        assert!(whole(7959, l).is_ok());
        // Zulu: the audio runs to the end, the picture stops at 2:15.
        let zulu = Lengths { container: 7958.976, video: Some(135.92) };
        assert_eq!(whole(7959, zulu).unwrap_err().to_string(), "its picture runs 0:02:15 but the video is 2:12:39");
        // Cut off mid-write: no tag, judged on the container.
        assert!(whole(7959, Lengths { container: 3000.0, video: None }).is_err());
    }

    /// A stand-in makemkvcon that writes a line to `ran` each time it's run.
    fn stand_in(dir: &Path, ran: &Path) -> PathBuf {
        let p = dir.join("makemkvcon");
        std::fs::write(&p, format!("#!/bin/sh\necho \"$@\" >> '{}'\necho 'MakeMKV v1.18.2 linux(x64-release) started'\n", ran.display())).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn looking_for_makemkv_only_checks_files_and_runs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ran = dir.path().join("ran");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let prog = stand_in(&bin, &ran);
        // A file that isn't a program isn't offered.
        std::fs::create_dir_all(dir.path().join("other")).unwrap();
        std::fs::write(dir.path().join("other/makemkvcon"), "not a program").unwrap();
        let fp = dir.path().join("flatpak/app/com.makemkv.MakeMKV/current");
        let places = || Places {
            dirs: vec![bin.join("makemkvcon"), dir.path().join("other/makemkvcon"), dir.path().join("none/makemkvcon")],
            programs: vec![prog.clone()],
            flatpak: vec![fp.clone()],
            host_root: None,
        };
        assert_eq!(candidates(&places()), vec![Location::Program { path: prog.clone() }], "listed once, though found twice");
        std::fs::create_dir_all(&fp).unwrap();
        assert_eq!(candidates(&places()), vec![Location::Program { path: prog.clone() }, Location::Flatpak]);
        assert!(!ran.exists(), "nothing was run");
    }

    #[test]
    fn inside_the_flatpak_the_hosts_usr_is_looked_at_under_run_host() {
        let dir = tempfile::tempdir().unwrap();
        let ran = dir.path().join("ran");
        std::fs::create_dir_all(dir.path().join("usr/bin")).unwrap();
        stand_in(&dir.path().join("usr/bin"), &ran);
        let places = Places { dirs: vec![PathBuf::from("/usr/bin/makemkvcon")], programs: vec![], flatpak: vec![], host_root: Some(dir.path().to_owned()) };
        // Offered as the host's own path, which is how it's run there.
        assert_eq!(candidates(&places), vec![Location::Program { path: PathBuf::from("/usr/bin/makemkvcon") }]);
        assert!(!ran.exists());
        assert_eq!(Launcher::of(&Location::Program { path: "/usr/bin/makemkvcon".into() }, true), Launcher::Host("/usr/bin/makemkvcon".into()));
        assert_eq!(Launcher::of(&Location::Flatpak, true), Launcher::Flatpak { via_host: true });
        assert_eq!(Launcher::of(&Location::Program { path: "/usr/bin/makemkvcon".into() }, false), Launcher::Direct(vec!["/usr/bin/makemkvcon".into()]));
    }

    #[test]
    fn a_file_chosen_in_the_picker() {
        let dir = tempfile::tempdir().unwrap();
        let prog = stand_in(dir.path(), &dir.path().join("ran"));
        assert_eq!(chosen_file(&prog).unwrap(), Location::Program { path: prog.clone() });
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        assert!(chosen_file(&dir.path().join("notes.txt")).unwrap_err().to_string().contains("isn't a program"));
        assert!(chosen_file(Path::new("/run/user/1000/doc/ab12/makemkvcon")).is_err());
        assert!(!dir.path().join("ran").exists());
    }

    /// Approving is when it may run: once, for its version.
    #[tokio::test]
    async fn the_version_is_asked_once_and_read_from_its_started_line() {
        let dir = tempfile::tempdir().unwrap();
        let ran = dir.path().join("ran");
        let prog = stand_in(dir.path(), &ran);
        let v = version(&Launcher::of(&Location::Program { path: prog }, false), &dir.path().join("work")).await.unwrap();
        assert_eq!(v, "MakeMKV v1.18.2");
        assert_eq!(std::fs::read_to_string(&ran).unwrap().lines().count(), 1, "run once");
        assert_eq!(version_in(FILM).as_deref(), Some("MakeMKV v1.17.7"));
        assert!(version(&Launcher::Direct(vec![dir.path().join("gone").display().to_string()]), dir.path()).await.is_err());
    }

    #[test]
    fn the_flatpak_may_start_host_programs_only_if_its_policy_says_so() {
        let granted = "[Application]\nname=x\n\n[Session Bus Policy]\norg.freedesktop.Flatpak=talk\n\n[Environment]\nA=b\n";
        assert!(host_allowed(granted));
        assert!(!host_allowed("[Application]\nname=x\n\n[Session Bus Policy]\norg.freedesktop.Notifications=talk\n"));
        assert!(!host_allowed("[Application]\norg.freedesktop.Flatpak=talk\n"), "only in the bus policy");
        assert!(!host_allowed(""));
    }

    /// Nothing approved and no DRT_MAKEMKVCON: no way to run it.
    #[test]
    fn with_no_ok_there_is_no_launcher() {
        if std::env::var_os("DRT_MAKEMKVCON").is_none() {
            assert_eq!(approved(), None, "nothing in this crate's tests approves one");
            assert_eq!(launcher(), None);
        }
    }
}
