//! Where a file goes: a path template, as the old app had.
//!
//! Slashes are folders; `{token}` is filled in from the track's facts; the
//! extension is added by the encoder. Each part is made safe on its own, so a
//! slash in an album title can't add a folder: the template decides the
//! shape, never the metadata. An empty part is dropped rather than left as an
//! empty folder, and an unknown token is left as written so a typo shows.
//!
//! `{token:sort}` sorts a name the way a record shop does: "The Cinematic
//! Orchestra" becomes "Cinematic Orchestra, The".

use std::path::PathBuf;

/// The layouts offered in the dropdown. Any other template can be typed.
pub const PRESETS: &[(&str, &str)] = &[
    ("Artist / Album / 01 - Title", "{albumartist}/{album}/{track} - {title}"),
    ("Sorted artist / Album / 01 Artist - Title", "{albumartist:sort}/{album}/{track} {artist} - {title}"),
    ("Album / 01 - Title", "{album}/{track} - {title}"),
    ("01 - Title", "{track} - {title}"),
];

pub const DEFAULT: &str = "{albumartist}/{album}/{track} - {title}";

// A disc image's layouts. A dump is several files that only mean
// anything together (the cue names its tracks; the log is the evidence
// behind the hashes), so a dump's template names its *folder*, and the
// files go in under the names they were made with. Renaming tracks would
// mean rewriting the cue, which changes the file Redump's hash describes.

/// A game's discs in one folder: the discs of a set together, with their
/// playlist beside them.
pub const GAME_FOLDER: &str = "{system}/{title}";

pub const DUMP_DEFAULT: &str = "{name}";

/// A game's discs, each in its own folder under its system, by Redump's
/// name: what the built-in Games recipe files them as.
pub const GAME_DEFAULT: &str = "{system}/{name}";

/// The layouts for a game disc's image: a system's games together.
pub const GAME_PRESETS: &[(&str, &str)] = &[("System / Name", GAME_DEFAULT), ("System / Game", GAME_FOLDER), ("Name", "{name}"), ("Date and disc name", "{date} {label}")];

/// The layouts for any other disc's image.
pub const DATA_PRESETS: &[(&str, &str)] = &[("Name", "{name}"), ("System / Name", GAME_DEFAULT), ("Date and disc name", "{date} {label}")];

/// What a Library offers: the music layouts and the image ones.
pub const LIBRARY_PRESETS: &[(&str, &str)] = &[
    ("Artist / Album / 01 - Title", "{albumartist}/{album}/{track} - {title}"),
    ("Sorted artist / Album / 01 Artist - Title", "{albumartist:sort}/{album}/{track} {artist} - {title}"),
    ("Album / 01 - Title", "{album}/{track} - {title}"),
    ("01 - Title", "{track} - {title}"),
    ("Name", "{name}"),
    ("System / Name", "{system}/{name}"),
    ("System / Game", GAME_FOLDER),
    ("Date and disc name", "{date} {label}"),
    ("Jellyfin film", MOVIE_DEFAULT),
    ("Jellyfin episode", EPISODE_DEFAULT),
];

/// Films and episodes, named the way Jellyfin reads them: the `[tmdbid-…]`
/// pins the match so it never guesses from the name.
pub const MOVIE_DEFAULT: &str = "Movies/{title} ({year}) [tmdbid-{tmdbid}]/{title} ({year})";
pub const EPISODE_DEFAULT: &str = "Shows/{title} ({year}) [tmdbid-{tmdbid}]/Season {season:02}/{title} ({year}) - S{season:02}E{episode:02} - {episode_title}";

/// The layouts for a DVD or Blu-ray title. *Jellyfin* is the pair: a film
/// with the film layout, an episode with the episode one, so one Library
/// files both.
pub const VIDEO_PRESETS: &[(&str, &str)] = &[(JELLYFIN, JELLYFIN), ("Jellyfin film", MOVIE_DEFAULT), ("Jellyfin episode", EPISODE_DEFAULT)];

/// The film layout for a film and the episode one for an episode.
pub const JELLYFIN: &str = "Jellyfin";

/// The tokens a video title's template understands.
pub const VIDEO_TOKENS: &[(&str, &str)] = &[
    ("title", "the film or the show"),
    ("year", "its year"),
    ("tmdbid", "its TMDb id"),
    ("season", "season number; {season:02} for two digits"),
    ("episode", "episode number; {episode:02} for two digits"),
    ("episode_title", "the episode's title"),
    ("label", "the name stored on the disc"),
];

/// What a video title's template can refer to.
#[derive(Clone, Debug, Default)]
pub struct VideoFields {
    pub title: String,
    pub year: String,
    pub tmdbid: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub episode_title: String,
    pub label: String,
    /// Bonus material, under this name: it goes in an `extras` folder beside
    /// the film or show, where Jellyfin keeps it out of the episode list.
    pub extra: Option<String>,
}

/// The template a Place uses for a video title: its own, unless it's
/// *Jellyfin*, which gives the film or the episode layout by what the
/// title is.
pub fn video_template(names: &str, episode: bool) -> &str {
    if names.is_empty() || names == JELLYFIN {
        if episode {
            EPISODE_DEFAULT
        } else {
            MOVIE_DEFAULT
        }
    } else {
        names
    }
}

/// A video title's template filled in, as a relative path without the
/// extension. Whatever is unknown leaves no trace: no `()` for a missing
/// year, no `[tmdbid-]`, no trailing ` - ` for an untitled episode.
pub fn render_video(template: &str, f: &VideoFields) -> PathBuf {
    let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
    let filled = fill(template, |name| {
        Some(match name {
            "title" => f.title.clone(),
            "year" => f.year.clone(),
            "tmdbid" => f.tmdbid.clone(),
            "season" => num(f.season),
            "episode" => num(f.episode),
            "episode_title" => f.episode_title.clone(),
            "label" => f.label.clone(),
            _ => return None,
        })
    });
    // Made safe again after tidying: taking out `()` can leave `..`.
    let mut out: PathBuf = filled.iter().map(|p| safe(&tidy(&p.to_string_lossy()))).filter(|p| !p.is_empty()).collect();
    if let Some(name) = &f.extra {
        // Beside the film, or beside the show rather than in a season.
        out.pop();
        if out.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Season")) {
            out.pop();
        }
        out.push("extras");
        out.push(safe(name));
    }
    out
}

/// One part of a video path with its empty brackets and dangling dashes gone.
fn tidy(part: &str) -> String {
    let mut s = part.to_owned();
    for empty in ["()", "[]", "[tmdbid-]"] {
        s = s.replace(empty, "");
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.replace(" - -", " -");
    s.trim_matches(|c: char| c == '-' || c.is_whitespace()).to_owned()
}

/// The tokens a disc image's template understands.
pub const DUMP_TOKENS: &[(&str, &str)] = &[
    ("name", "Redump's name, or else the name stored on the disc"),
    ("label", "the name stored on the disc"),
    ("system", "Redump's system, when it knows the disc"),
    ("date", "the day it was ripped, 2026-11-18"),
    ("size", "the disc's size, 4.3 GB"),
    ("title", "a game's name without its disc: Colony Wars (Europe)"),
    ("disc", "a game's disc number in its set"),
    ("serial", "a game's serial, SLES-00860"),
    ("region", "a game's region, Europe"),
];

/// The tokens any disc's image understands: [`DUMP_TOKENS`] less what
/// only a game has.
pub const DATA_TOKENS: &[(&str, &str)] = &[
    ("name", "Redump's name, or else the name stored on the disc"),
    ("label", "the name stored on the disc"),
    ("system", "Redump's system, when it knows the disc"),
    ("date", "the day it was ripped, 2026-11-18"),
    ("size", "the disc's size, 4.3 GB"),
];

/// What a Folder's names are for, from what reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Files {
    /// Tracks (and the cue sheets, lyrics and scans filed beside them).
    Music,
    /// A game disc's image: the Game disc step said which system.
    Game,
    /// Any other disc's image.
    Data,
    /// Films and episodes from a DVD or Blu-ray.
    Video,
}

impl Files {
    /// From what arrives at the Folder: what kind of file, and whether the
    /// steps before it found which system it's for. `None` when it could
    /// be anything (branches of different kinds joined).
    pub fn of(flow: &drt_engine::Flow) -> Option<Files> {
        use drt_engine::ItemKind as K;
        match flow.kind {
            K::ImageFile if flow.found.contains("system") => Some(Files::Game),
            K::ImageFile => Some(Files::Data),
            K::VideoFile => Some(Files::Video),
            K::AudioFile | K::TextFile | K::PictureFile => Some(Files::Music),
            _ => None,
        }
    }

    /// Its default, presets and tokens.
    pub fn fitted(self) -> drt_engine::Fitted {
        // Saved when every Folder started on the music layout: an image or
        // a video on it means its own default.
        const MUSIC_DEFAULT: &[&str] = &[DEFAULT, PRESETS[0].0];
        let (default, presets, tokens, was_default) = match self {
            Files::Music => (DEFAULT, PRESETS, TOKENS, &[][..]),
            Files::Game => (GAME_DEFAULT, GAME_PRESETS, DUMP_TOKENS, MUSIC_DEFAULT),
            Files::Data => (DUMP_DEFAULT, DATA_PRESETS, DATA_TOKENS, MUSIC_DEFAULT),
            Files::Video => (JELLYFIN, VIDEO_PRESETS, VIDEO_TOKENS, MUSIC_DEFAULT),
        };
        drt_engine::Fitted { default, presets, tokens, was_default }
    }
}

/// A Folder's names where it stands: [`Files::of`] what arrives, or every
/// layout when that's mixed.
pub fn names_for(flow: &drt_engine::Flow) -> drt_engine::Fitted {
    Files::of(flow).map_or(drt_engine::Fitted { default: DEFAULT, presets: LIBRARY_PRESETS, tokens: TOKENS, was_default: &[] }, Files::fitted)
}

/// A preset's name for a template or a name, from every layout there is;
/// anything else as it is: what a Folder's card says.
pub fn preset_name(names: &str) -> String {
    let t = renamed(names);
    LIBRARY_PRESETS.iter().chain(VIDEO_PRESETS).find(|(n, p)| *p == t || *n == t).map_or(t, |(n, _)| n).to_owned()
}

/// What a disc image's template can refer to.
#[derive(Clone, Debug, Default)]
pub struct DumpFields {
    pub name: String,
    pub label: String,
    pub system: String,
    pub date: String,
    pub size: String,
}

/// What a game's template can refer to as well. Kept apart from
/// [`DumpFields`] so code that builds those needn't change.
#[derive(Clone, Debug, Default)]
pub struct GameFields {
    /// The name without "(Disc N)": what a set's discs share.
    pub title: String,
    /// The disc number in its set, or empty.
    pub disc: String,
    pub serial: String,
    pub region: String,
}

impl GameFields {
    /// What a Redump name says: its title and disc number.
    pub fn from_name(name: &str) -> GameFields {
        let (title, disc) = split_disc(name).map_or((name.to_owned(), String::new()), |(t, n)| (t, n.to_string()));
        GameFields { title, disc, ..GameFields::default() }
    }
}

/// `Final Fantasy VII (USA) (Disc 2)` → `("Final Fantasy VII (USA)", 2)`;
/// what's after the disc number stays: `X (Disc 1) (Rev 1)` → `X (Rev 1)`.
pub fn split_disc(name: &str) -> Option<(String, u32)> {
    let at = name.rfind(" (Disc ")?;
    let rest = &name[at + " (Disc ".len()..];
    let close = rest.find(')')?;
    let n = rest[..close].trim().parse().ok()?;
    Some((format!("{}{}", &name[..at], &rest[close + 1..]).trim().to_owned(), n))
}

/// A dump's template filled in: the folder its files go in, relative.
pub fn render_dump(template: &str, f: &DumpFields) -> PathBuf {
    render_game(template, f, &GameFields::from_name(&f.name))
}

/// A game dump's template filled in.
pub fn render_game(template: &str, f: &DumpFields, g: &GameFields) -> PathBuf {
    fill(template, |name| {
        Some(match name {
            "title" => g.title.clone(),
            "disc" => g.disc.clone(),
            "serial" => g.serial.clone(),
            "region" => g.region.clone(),
            "name" => f.name.clone(),
            "label" => f.label.clone(),
            "system" => f.system.clone(),
            "date" => f.date.clone(),
            "size" => f.size.clone(),
            _ => return None,
        })
    })
}

/// Today's date as YYYY-MM-DD, in UTC.
pub fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    date(days)
}

/// Days since 1970-01-01 as a date (Howard Hinnant's civil-from-days).
pub fn date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The tokens a template understands, for the window to list.
pub const TOKENS: &[(&str, &str)] = &[
    ("albumartist", "the album's artist"),
    ("album", "the album"),
    ("artist", "this track's artist"),
    ("title", "the track's title"),
    ("track", "track number, two digits"),
    ("tracktotal", "how many tracks"),
    ("year", "release year"),
    ("disc", "disc number"),
    ("label", "record label"),
];

/// What a template can refer to, for one track.
#[derive(Clone, Debug, Default)]
pub struct Fields {
    pub albumartist: String,
    pub album: String,
    pub artist: String,
    pub title: String,
    pub track: u32,
    pub tracktotal: u32,
    pub year: String,
    pub disc: String,
    pub label: String,
}

/// A preset's name as it is now, from one a recipe was saved with before
/// presets were renamed; anything else as it is.
pub fn renamed(names: &str) -> &str {
    match names {
        "As on the NAS" => "Sorted artist / Album / 01 Artist - Title",
        "Date and label" => "Date and disc name",
        _ => names,
    }
}

/// What a template means: a preset's name stands for its template, so a
/// recipe saved with a name from the dropdown keeps working.
pub fn resolve(names: &str) -> &str {
    let names = renamed(names);
    LIBRARY_PRESETS.iter().find(|(n, _)| *n == names).map_or(names, |(_, t)| t)
}

/// The template filled in, as a relative path without the extension.
pub fn render(template: &str, f: &Fields) -> PathBuf {
    fill(template, |name| {
        Some(match name {
            "albumartist" => f.albumartist.clone(),
            "album" => f.album.clone(),
            "artist" => f.artist.clone(),
            "title" => f.title.clone(),
            "track" => format!("{:02}", f.track),
            "tracktotal" => f.tracktotal.to_string(),
            "year" => f.year.clone(),
            "disc" => f.disc.clone(),
            "label" => f.label.clone(),
            _ => return None,
        })
    })
}

/// Fills in a template with `value`, which knows the tokens; `:sort` works
/// on any of them, and an unknown one is left as written.
fn fill(template: &str, value: impl Fn(&str) -> Option<String>) -> PathBuf {
    let token = |token: &str| -> String {
        // `:sort`, or a width like `:02` that pads a number with zeros.
        let (name, sorted, width) = match token.split_once(':') {
            Some((n, "sort")) => (n, true, None),
            Some((n, w)) if w.starts_with('0') && w.parse::<usize>().is_ok() => (n, false, w.parse::<usize>().ok()),
            _ => (token, false, None),
        };
        match value(name) {
            Some(v) if sorted => sort_name(&v),
            Some(v) if width.is_some() && !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) => format!("{v:0>w$}", w = width.unwrap()),
            Some(v) => v,
            None => format!("{{{token}}}"),
        }
    };
    let mut out = PathBuf::new();
    for part in resolve(template).split('/') {
        let mut text = String::new();
        let mut rest = part;
        while let Some(open) = rest.find('{') {
            text.push_str(&rest[..open]);
            match rest[open..].find('}') {
                Some(close) => {
                    text.push_str(&token(&rest[open + 1..open + close]));
                    rest = &rest[open + close + 1..];
                }
                None => break,
            }
        }
        text.push_str(rest);
        let clean = safe(&text);
        if !clean.is_empty() {
            out.push(clean);
        }
    }
    out
}

/// "The Cinematic Orchestra" → "Cinematic Orchestra, The". Also "A" and
/// "An", which is what record shops and taggers do.
pub fn sort_name(name: &str) -> String {
    for article in ["The ", "A ", "An "] {
        if let Some(rest) = name.strip_prefix(article) {
            if !rest.is_empty() {
                return format!("{rest}, {}", article.trim_end());
            }
        }
    }
    name.to_owned()
}

/// The longest a name made by [`safe`] gets, in bytes. Filesystems allow
/// 255; this leaves room for what's added after: an extension, ` (2)` for a
/// second copy, `.part` while it's copied.
pub const MAX_NAME: usize = 200;

/// A name safe on every filesystem a library might live on, NAS shares
/// included. Whatever a title says, it is one name: no folder separators,
/// never `.` or `..`, no control characters, nothing Windows refuses
/// (`CON`, `NUL.txt`, a trailing dot), and not too long. The same name
/// twice through gives the same name.
pub fn safe(s: &str) -> String {
    let s: String = s.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let mut end = s.len().min(MAX_NAME);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let s = s[..end].trim_start().trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    // Windows (and so an SMB share a Windows machine reads) reserves these
    // names with any extension: `nul.flac` is the null device.
    let stem = s.split('.').next().unwrap_or("").trim_end();
    let reserved = matches!(stem.to_ascii_uppercase().as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && stem.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("COM") || p.eq_ignore_ascii_case("LPT")) && stem.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        format!("{stem}_{}", &s[stem.len()..])
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tortoise() -> Fields {
        Fields {
            albumartist: "The Cinematic Orchestra".into(),
            album: "Every Day".into(),
            artist: "The Cinematic Orchestra".into(),
            title: "All That You Give".into(),
            track: 1,
            tracktotal: 7,
            year: "2002".into(),
            ..Default::default()
        }
    }

    #[test]
    fn what_a_folder_files_comes_from_what_reaches_it() {
        use drt_engine::{Flow, ItemKind as K};
        let flow = |kind, found: &[&'static str]| Flow { kind, found: found.iter().copied().collect(), ..Flow::start() };
        assert_eq!(Files::of(&flow(K::AudioFile, &["release"])), Some(Files::Music));
        // A cue sheet, a lyrics file or a scan is filed beside its tracks.
        for k in [K::TextFile, K::PictureFile] {
            assert_eq!(Files::of(&flow(k, &[])), Some(Files::Music));
        }
        assert_eq!(Files::of(&flow(K::ImageFile, &["system", "name"])), Some(Files::Game));
        assert_eq!(Files::of(&flow(K::ImageFile, &["name"])), Some(Files::Data));
        assert_eq!(Files::of(&flow(K::VideoFile, &[])), Some(Files::Video));
        // Branches of different files joined: every layout, the music default.
        assert_eq!(Files::of(&flow(K::AnyFile, &[])), None);
        let any = names_for(&flow(K::AnyFile, &[]));
        assert_eq!((any.default, any.presets), (DEFAULT, LIBRARY_PRESETS));
        // Each family's default is one of its presets, and its presets' tokens are its own.
        for f in [Files::Music, Files::Game, Files::Data, Files::Video] {
            let x = f.fitted();
            assert!(x.presets.iter().any(|(_, t)| *t == x.default), "{f:?}");
            for (_, t) in x.presets.iter().filter(|(n, _)| !n.starts_with("Jellyfin")) {
                for tok in t.split('{').skip(1).map(|p| p.split(['}', ':']).next().unwrap()) {
                    assert!(x.tokens.iter().any(|(k, _)| *k == tok), "{f:?}: {tok}");
                }
            }
        }
    }

    #[test]
    fn the_sorted_artist_layout_moves_the_article() {
        assert_eq!(render(resolve("Sorted artist / Album / 01 Artist - Title"), &tortoise()), PathBuf::from("Cinematic Orchestra, The/Every Day/01 The Cinematic Orchestra - All That You Give"));
    }

    #[test]
    fn the_default_and_custom_templates() {
        assert_eq!(render(DEFAULT, &tortoise()), PathBuf::from("The Cinematic Orchestra/Every Day/01 - All That You Give"));
        assert_eq!(render("{year} {album}/{track}. {title}", &tortoise()), PathBuf::from("2002 Every Day/01. All That You Give"));
    }

    #[test]
    fn metadata_cannot_change_the_shape_and_empty_parts_go() {
        let mut f = tortoise();
        f.album = "Volume 1: A/B".into();
        f.disc = String::new();
        assert_eq!(render("{album}/{disc}/{track}", &f), PathBuf::from("Volume 1_ A_B/01"));
        assert_eq!(render("{albm}/{track}", &f), PathBuf::from("{albm}/01"), "a typo shows");
    }

    #[test]
    fn a_dump_is_filed_in_a_folder_named_by_its_template() {
        let f = DumpFields { name: "Encarta 98 Encyclopedia (USA) (Disc 1)".into(), label: "ENCARTA98".into(), system: "IBM PC".into(), date: "2026-11-18".into(), size: "650 MB".into() };
        assert_eq!(render_dump(DUMP_DEFAULT, &f), PathBuf::from("Encarta 98 Encyclopedia (USA) (Disc 1)"));
        assert_eq!(render_dump(resolve("System / Name"), &f), PathBuf::from("IBM PC/Encarta 98 Encyclopedia (USA) (Disc 1)"));
        assert_eq!(render_dump("Date and disc name", &f), PathBuf::from("2026-11-18 ENCARTA98"), "a preset's name stands for it");
        assert_eq!(render_dump("Date and label", &f), PathBuf::from("2026-11-18 ENCARTA98"), "and so does its old name");
        // Not in Redump: no system, and the folder doesn't get an empty level.
        let plain = DumpFields { name: "BACKUP: 2019/2020".into(), system: String::new(), ..f.clone() };
        assert_eq!(render_dump("{system}/{name} ({size})", &plain), PathBuf::from("BACKUP_ 2019_2020 (650 MB)"));
        assert_eq!(render_dump("{album}", &plain), PathBuf::from("{album}"), "a music token shows as a typo here");
        // A game's tokens.
        let cw = DumpFields { name: "Colony Wars (Europe) (Disc 2)".into(), system: "PlayStation".into(), ..f };
        let g = GameFields { serial: "SLES-10860".into(), region: "Europe".into(), ..GameFields::from_name(&cw.name) };
        assert_eq!(render_game(resolve("System / Game"), &cw, &g), PathBuf::from("PlayStation/Colony Wars (Europe)"));
        assert_eq!(render_game("{system}/{title}/{name}", &cw, &g), PathBuf::from("PlayStation/Colony Wars (Europe)/Colony Wars (Europe) (Disc 2)"));
        assert_eq!(render_game("{region}/{serial} {disc}", &cw, &g), PathBuf::from("Europe/SLES-10860 2"));
        assert_eq!(render_dump("{title}", &plain), PathBuf::from("BACKUP_ 2019_2020"), "one disc: its name");
    }

    fn sopranos() -> VideoFields {
        VideoFields {
            title: "The Sopranos".into(),
            year: "1999".into(),
            tmdbid: "1398".into(),
            season: Some(3),
            episode: Some(1),
            episode_title: "Mr. Ruggerio's Neighborhood".into(),
            ..Default::default()
        }
    }

    #[test]
    fn films_and_episodes_are_named_for_jellyfin() {
        let film = VideoFields { title: "Blade Runner".into(), year: "1982".into(), tmdbid: "78".into(), ..Default::default() };
        assert_eq!(render_video(video_template("Jellyfin", false), &film), PathBuf::from("Movies/Blade Runner (1982) [tmdbid-78]/Blade Runner (1982)"));
        assert_eq!(
            render_video(video_template("Jellyfin", true), &sopranos()),
            PathBuf::from("Shows/The Sopranos (1999) [tmdbid-1398]/Season 03/The Sopranos (1999) - S03E01 - Mr. Ruggerio's Neighborhood")
        );
        // The music default, as a Folder left alone has, means Jellyfin too.
        assert_eq!(video_template("Jellyfin film", true), "Jellyfin film", "a layout you picked is kept");
        assert_eq!(render_video("Jellyfin film", &film), PathBuf::from("Movies/Blade Runner (1982) [tmdbid-78]/Blade Runner (1982)"));
        assert_eq!(render_video("{title}/{season}x{episode:03}", &sopranos()), PathBuf::from("The Sopranos/3x001"));
    }

    #[test]
    fn what_isnt_known_leaves_no_trace_in_a_video_path() {
        let f = VideoFields { title: "Home Movies: 2019/2020".into(), episode_title: String::new(), ..sopranos() };
        let f = VideoFields { year: String::new(), tmdbid: String::new(), ..f };
        assert_eq!(render_video(EPISODE_DEFAULT, &f), PathBuf::from("Shows/Home Movies_ 2019_2020/Season 03/Home Movies_ 2019_2020 - S03E01"));
        let film = VideoFields { title: "BLADE_RUNNER".into(), ..Default::default() };
        assert_eq!(render_video(MOVIE_DEFAULT, &film), PathBuf::from("Movies/BLADE_RUNNER/BLADE_RUNNER"));
    }

    #[test]
    fn extras_go_beside_the_film_or_the_show() {
        let film = VideoFields { title: "Blade Runner".into(), year: "1982".into(), tmdbid: "78".into(), extra: Some("Title 3".into()), ..Default::default() };
        assert_eq!(render_video(MOVIE_DEFAULT, &film), PathBuf::from("Movies/Blade Runner (1982) [tmdbid-78]/extras/Title 3"));
        let ep = VideoFields { season: Some(3), episode: None, episode_title: String::new(), extra: Some("Title 6".into()), ..sopranos() };
        assert_eq!(render_video(EPISODE_DEFAULT, &ep), PathBuf::from("Shows/The Sopranos (1999) [tmdbid-1398]/extras/Title 6"));
    }

    /// Every part of `p` is a plain name: nothing that climbs out of the
    /// folder or starts at the root.
    fn stays_inside(p: &std::path::Path) {
        use std::path::Component;
        assert!(p.components().all(|c| matches!(c, Component::Normal(_))), "{p:?} leaves its folder");
        for c in p.components() {
            let n = c.as_os_str().to_string_lossy();
            assert!(n.len() <= MAX_NAME + 1 && !n.chars().any(char::is_control) && n.trim() == n && !n.ends_with('.'), "{n:?}");
        }
    }

    const HOSTILE: &[&str] = &[
        "..",
        "../../../../etc/passwd",
        "/etc/passwd",
        "\\\\server\\share\\x",
        "C:\\Windows\\System32",
        "..\\..\\x",
        ". .",
        "...",
        " .. ",
        "..()",
        "..-",
        "(..)",
        "[tmdbid-]..",
        "a\0b\nc\rd\te\u{1b}[31m",
        "CON",
        "nul.flac",
        "Lpt9",
        "com1.tar.gz",
        "-rf",
        "--help",
    ];

    #[test]
    fn hostile_titles_never_leave_the_folder() {
        let long = "ü".repeat(400);
        for t in HOSTILE.iter().copied().chain([long.as_str()]) {
            let f = Fields { albumartist: t.into(), album: t.into(), artist: t.into(), title: t.into(), year: t.into(), disc: t.into(), label: t.into(), ..tortoise() };
            for tpl in [DEFAULT, "{albumartist:sort}/{album}/{track} {artist} - {title}", "{title}", "{album}/{disc}/{year}/{label}/{title}"] {
                stays_inside(&render(tpl, &f));
            }
            let v = VideoFields { title: t.into(), year: String::new(), tmdbid: String::new(), episode_title: t.into(), label: t.into(), extra: None, ..sopranos() };
            for tpl in [MOVIE_DEFAULT, EPISODE_DEFAULT, "{title}", "{title}/{episode_title}", "{label}"] {
                stays_inside(&render_video(tpl, &v));
                stays_inside(&render_video(tpl, &VideoFields { extra: Some(t.into()), ..v.clone() }));
            }
            let d = DumpFields { name: t.into(), label: t.into(), system: t.into(), date: t.into(), size: t.into() };
            for tpl in [DUMP_DEFAULT, "{system}/{name}", GAME_FOLDER, "{label}"] {
                stays_inside(&render_dump(tpl, &d));
            }
        }
    }

    #[test]
    fn a_bare_dot_dot_title_is_dropped_not_climbed() {
        // `..()` tidies to `..` once the empty year is taken out.
        let v = VideoFields { title: "..".into(), ..Default::default() };
        assert_eq!(render_video(MOVIE_DEFAULT, &v), PathBuf::from("Movies"));
        let f = Fields { album: "..".into(), ..tortoise() };
        assert_eq!(render(DEFAULT, &f), PathBuf::from("The Cinematic Orchestra/01 - All That You Give"));
    }

    #[test]
    fn a_template_cannot_climb_either() {
        stays_inside(&render("../../{album}/./{title}", &tortoise()));
        stays_inside(&render("/{album}//{title}", &tortoise()));
        assert_eq!(render("../{album}", &tortoise()), PathBuf::from("Every Day"));
    }

    #[test]
    fn names_windows_refuses_are_changed() {
        assert_eq!(safe("CON"), "CON_");
        assert_eq!(safe("nul.flac"), "nul_.flac");
        assert_eq!(safe("com1.tar.gz"), "com1_.tar.gz");
        assert_eq!(safe("LPT9"), "LPT9_");
        assert_eq!(safe("Console"), "Console", "only the name itself");
        assert_eq!(safe("COMA"), "COMA");
        assert_eq!(safe("a€"), "a€", "four bytes that aren't COM or LPT");
        assert_eq!(safe("Gone."), "Gone");
    }

    #[test]
    fn long_names_are_cut_on_a_character() {
        let s = safe(&"ü".repeat(400));
        assert!(s.len() <= MAX_NAME && s.chars().all(|c| c == 'ü'), "{}", s.len());
        assert_eq!(safe(&"a".repeat(300)).len(), MAX_NAME);
    }

    #[test]
    fn safe_is_the_same_twice_through() {
        let long = format!("{}.", "x".repeat(250));
        for t in HOSTILE.iter().copied().chain(["Volume 1: A/B", "The Cinematic Orchestra", long.as_str()]) {
            assert_eq!(safe(&safe(t)), safe(t), "{t:?}");
        }
        assert_eq!(safe("All That You Give"), "All That You Give", "ordinary titles are left alone");
    }

    #[test]
    fn dates_from_days() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(20_775), "2026-11-18");
        assert_eq!(date(11_016), "2000-02-29");
    }

    #[test]
    fn sorting_names_moves_the_article() {
        assert_eq!(sort_name("The Cinematic Orchestra"), "Cinematic Orchestra, The");
        assert_eq!(sort_name("A Tribe Called Quest"), "Tribe Called Quest, A");
        assert_eq!(sort_name("Tortoise"), "Tortoise");
        assert_eq!(sort_name("Theatre of Tragedy"), "Theatre of Tragedy");
    }
}
