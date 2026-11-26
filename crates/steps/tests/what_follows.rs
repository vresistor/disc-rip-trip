//! What follows what: at every + in every built-in recipe, only steps that
//! make sense there are offered. The steps are classed here by hand, by
//! name, and what came before by the steps on the way, so the test doesn't
//! just repeat the kinds the steps declare.

use trip_engine::{check, fits, fits_flow, flow_after, ItemKind, Node, Registry, StepInfo};
use trip_steps::{real, recipes, simulated};

/// Only make sense on a disc image: a game's or a data disc's.
const DUMP_ONLY: &[&str] = &["image", "game-rip", "check", "redump", "game-choose", "chd", "keep", "libcrypt", "m3u"];
/// Only make sense for a game (they need what Game disc found).
const GAME_ONLY: &[&str] = &["game-rip", "redump", "game-choose", "libcrypt"];
/// Only make sense on an audio CD, its tracks or their files.
const AUDIO_ONLY: &[&str] = &[
    "audio-cd",
    "musicbrainz",
    "cd-text",
    "discogs",
    "gnudb",
    "choose",
    "cover-art",
    "lyrics",
    "rip",
    "cue",
    "lyrics-file",
    "artwork-files",
    "replaygain",
    "flac",
    "mp3",
    "opus",
    "aac",
    "alac",
    "vorbis",
    "wav",
    "wavpack",
];
/// Look at the audio disc itself, so only before Rip.
const AUDIO_DISC_ONLY: &[&str] = &["musicbrainz", "cd-text", "discogs", "gnudb", "choose", "cover-art", "lyrics", "rip"];
/// May sensibly appear twice on one path (Review, then Library).
const REPEATABLE: &[&str] = &["folder", "hold"];
/// Work out what the disc is: nothing to add once a Choose has settled it.
const IDENTIFY: &[&str] = &["musicbrainz", "cd-text", "discogs", "gnudb", "tmdb", "redump", "choose", "choose-video", "game-choose"];
/// Look the disc up from what it is, so only straight after what read it.
const DISC_LOOKUPS: &[(&str, &str)] =
    &[("musicbrainz", "audio-cd"), ("cd-text", "audio-cd"), ("discogs", "audio-cd"), ("gnudb", "audio-cd"), ("tmdb", "video-disc"), ("redump", "check"), ("cover-art", "choose"), ("lyrics", "choose")];
const CHOOSE: &[&str] = &["choose", "choose-video", "game-choose"];
/// Make files: only after one of these is there anything to hold.
const MAKES_FILES: &[&str] = &["flac", "mp3", "opus", "aac", "alac", "vorbis", "wav", "wavpack", "cue", "lyrics-file", "artwork-files", "chd", "keep", "transcode", "remux"];
/// Write beside files a Folder filed, so only after one.
const BESIDE_FILED: &[&str] = &["cue", "lyrics-file", "artwork-files", "libcrypt", "m3u"];
/// Only make sense on a DVD or Blu-ray.
const VIDEO_ONLY: &[&str] = &["video-disc", "tmdb", "choose-video", "rip-video", "transcode", "remux"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Domain {
    Unknown,
    Audio,
    Video,
    Game,
    Data,
}

/// The step kinds before and including `id`, in the order they run: what
/// a step inserted after `id` comes after.
fn upstream(root: &Node, id: &str) -> Option<Vec<String>> {
    // Ok: everything in `node`, appended; Err: found `id`, the path to it.
    fn go(node: &Node, mut before: Vec<String>, id: &str) -> Result<Vec<String>, Vec<String>> {
        match node {
            Node::Step { id: x, step, .. } => {
                before.push(step.clone());
                if x == id {
                    Err(before)
                } else {
                    Ok(before)
                }
            }
            Node::Chain { nodes } => nodes.iter().try_fold(before, |b, n| go(n, b, id)),
            Node::Branches { branches } => {
                let mut all = before.clone();
                for b in branches {
                    let inner = go(b, before.clone(), id)?;
                    all.extend(inner.into_iter().skip(before.len()));
                }
                Ok(all)
            }
            Node::Group { body, .. } => go(body, before, id),
        }
    }
    go(root, vec![], id).err()
}

fn domain(up: &[String]) -> Domain {
    let has = |k: &str| up.iter().any(|s| s == k);
    if has("audio-cd") {
        Domain::Audio
    } else if has("video-disc") {
        Domain::Video
    } else if has("game-disc") {
        Domain::Game
    } else if has("data-disc") {
        Domain::Data
    } else {
        Domain::Unknown
    }
}

/// What the + picker offers after `id`: every step that fits the flow
/// there, and every step that, put in line after it or on a new branch
/// beside what follows, leaves the recipe still checking.
fn offered<'a>(root: &Node, id: &str, reg: &'a Registry) -> (ItemKind, Vec<&'a StepInfo>) {
    let flow = flow_after(root, id, reg).unwrap_or_else(|| panic!("no flow after {id}"));
    let mut out = fits_flow(&flow, reg);
    for info in fits(flow.kind, reg) {
        for branch in [false, true] {
            let mut trial = root.clone();
            let new = Node::step("__new", info.kind);
            let placed = if branch { trial.add_branch_after(id, new) } else { trial.insert_after(id, new) };
            if placed && check(&trial, reg).is_ok() && !out.iter().any(|i| i.kind == info.kind) {
                out.push(info);
            }
        }
    }
    (flow.kind, out)
}

fn registries() -> [(&'static str, Registry); 2] {
    [("simulated", simulated(None)), ("real", real())]
}

#[test]
fn every_built_in_recipe_checks() {
    for (which, reg) in registries() {
        for r in recipes::all() {
            check(&r.root, &reg).unwrap_or_else(|e| panic!("{which}: {}: {e}", r.name));
        }
    }
}

#[test]
fn every_plus_offers_only_what_makes_sense() {
    for (which, reg) in registries() {
        for r in recipes::all() {
            for id in r.root.ids() {
                let up = upstream(&r.root, id).unwrap();
                let (kind, offers) = offered(&r.root, id, &reg);
                let kinds: Vec<&str> = offers.iter().map(|i| i.kind).collect();
                let at = format!("{which}: {}: after {id} ({kind:?}), offered {kinds:?}", r.name);
                let any = |set: &[&str]| kinds.iter().any(|k| set.contains(k));
                let d = domain(&up);

                if kind == ItemKind::Nothing {
                    assert!(kinds.is_empty(), "{at}: nothing follows what gives nothing");
                }
                if matches!(d, Domain::Audio | Domain::Video) {
                    assert!(!any(DUMP_ONLY), "{at}: a game or data step on audio or video");
                }
                if matches!(d, Domain::Video | Domain::Game | Domain::Data) {
                    assert!(!any(AUDIO_ONLY), "{at}: an audio step on a video or an image");
                }
                if matches!(d, Domain::Audio | Domain::Game | Domain::Data) {
                    assert!(!any(VIDEO_ONLY), "{at}: a video step on audio or an image");
                }
                if d != Domain::Game {
                    assert!(!any(GAME_ONLY), "{at}: a game step off a game's path");
                }
                if up.iter().any(|s| s == "rip") {
                    assert!(!any(AUDIO_DISC_ONLY), "{at}: a step on the disc after its tracks are read");
                }
                for k in &kinds {
                    assert!(REPEATABLE.contains(k) || !up.iter().any(|s| s == k), "{at}: {k} is already on this path");
                }
                if up.iter().any(|s| CHOOSE.contains(&s.as_str())) {
                    assert!(!any(IDENTIFY), "{at}: a lookup or Choose after Choose settled it");
                }
                let just_before = up.last().map(String::as_str);
                for (k, src) in DISC_LOOKUPS {
                    assert!(!kinds.contains(k) || just_before == Some(src), "{at}: {k} chained after {just_before:?}, not straight after {src}");
                }
                if any(&["hold"]) {
                    assert!(up.iter().any(|s| MAKES_FILES.contains(&s.as_str())), "{at}: Hold before there are files to hold");
                }
                if any(BESIDE_FILED) {
                    assert!(up.iter().any(|s| s == "folder"), "{at}: writes beside filed files before any are filed");
                }
                if any(&["lyrics-file"]) {
                    assert!(up.iter().any(|s| s == "lyrics"), "{at}: Lyrics file with no Lyrics on the way");
                }
                if any(&["libcrypt", "m3u"]) {
                    let made = up.iter().any(|s| s == "chd" || s == "keep");
                    assert!(made && up.iter().any(|s| s == "folder") && kind == ItemKind::ImageFile, "{at}: LibCrypt or M3U before a filed image");
                }
            }
        }
    }
}

#[test]
fn the_plus_still_offers_what_belongs() {
    for (which, reg) in registries() {
        let at = |r: &trip_engine::Recipe, id: &str| offered(&r.root, id, &reg).1.iter().map(|i| i.kind).collect::<Vec<_>>();
        let (cd, games, data, video) = (recipes::cd(), recipes::games(), recipes::data(), recipes::video());

        // The bug that started this: a Library of audio files offered LibCrypt.
        let lib = at(&cd, "lib1");
        assert!(lib.contains(&"folder") && lib.contains(&"replaygain") && lib.contains(&"hold") && lib.contains(&"tell"), "{which}: {lib:?}");
        // A cue sheet goes beside the filed files; the CDs recipe has no
        // Lyrics, so no Lyrics file.
        assert!(lib.contains(&"cue") && !lib.contains(&"lyrics-file"), "{which}: {lib:?}");
        let alib = at(&recipes::archive(), "a_lib");
        assert!(alib.contains(&"lyrics-file"), "{which}: {alib:?}");
        // Every scan beside the filed tracks: once there are filed tracks
        // of a chosen album, and not before.
        assert!(lib.contains(&"artwork-files") && alib.contains(&"artwork-files"), "{which}: {lib:?}");
        assert!(!lib.contains(&"libcrypt") && !lib.contains(&"m3u"), "{which}: {lib:?}");

        // After Choose: no second Choose, no lookups that identify the
        // disc, nothing to hold yet; what uses the release still fits.
        let chose = at(&cd, "choose");
        assert!(chose.contains(&"lyrics"), "{which}: {chose:?}");
        for k in ["choose", "musicbrainz", "cd-text", "discogs", "gnudb", "hold"] {
            assert!(!chose.contains(&k), "{which}: {k} after Choose: {chose:?}");
        }
        let vchose = at(&video, "v_choose");
        assert!(!vchose.contains(&"choose-video") && !vchose.contains(&"tmdb") && !vchose.contains(&"hold"), "{which}: {vchose:?}");

        // Lookups go straight after Audio CD, not after one another.
        let disc = at(&cd, "cd");
        assert!(disc.contains(&"discogs") && disc.contains(&"gnudb"), "{which}: {disc:?}");
        let mb = at(&cd, "mb");
        assert!(!mb.contains(&"discogs") && !mb.contains(&"gnudb"), "{which}: a lookup chained after MusicBrainz: {mb:?}");

        let rip = at(&cd, "rip");
        assert!(rip.contains(&"flac") && rip.contains(&"next-disc"), "{which}: {rip:?}");
        assert!(!rip.contains(&"cue") && !rip.contains(&"lyrics-file") && !rip.contains(&"artwork-files"), "{which}: nothing filed to describe yet: {rip:?}");
        assert!(!rip.contains(&"replaygain") && !rip.contains(&"folder"), "{which}: nothing to file or level yet: {rip:?}");

        let flac = at(&cd, "flac");
        assert!(flac.contains(&"folder") && flac.contains(&"replaygain"), "{which}: {flac:?}");
        assert!(!flac.contains(&"cue"), "{which}: encoded but not filed: {flac:?}");

        let chd = at(&games, "g_chd");
        assert!(chd.contains(&"folder") && !chd.contains(&"libcrypt") && !chd.contains(&"m3u"), "{which}: not filed yet: {chd:?}");
        let filed = at(&games, "g_lib");
        assert!(filed.contains(&"libcrypt") && filed.contains(&"m3u"), "{which}: {filed:?}");

        // A data disc's image, filed under the name Check matched: a
        // playlist may go beside it, but the PlayStation step needs a game.
        let d = at(&data, "d_lib");
        assert!(d.contains(&"m3u") && !d.contains(&"libcrypt"), "{which}: {d:?}");
        let checked = at(&data, "d_check");
        assert!(checked.contains(&"chd") && !checked.contains(&"redump"), "{which}: {checked:?}");

        let made = at(&video, "v_make");
        assert!(made.contains(&"folder") && !made.contains(&"replaygain"), "{which}: {made:?}");

        assert!(at(&cd, "next").is_empty(), "{which}");
        assert!(at(&recipes::archive(), "a_tell").is_empty(), "{which}");
    }
}

/// The simulated steps stand in for the real ones in the window, so they
/// must fit the same places.
#[test]
fn simulated_and_real_steps_fit_the_same_places() {
    let (sim, real) = (simulated(None), real());
    let mut kinds: Vec<&str> = real.infos().map(|i| i.kind).collect();
    kinds.sort();
    let mut sim_kinds: Vec<&str> = sim.infos().map(|i| i.kind).collect();
    sim_kinds.sort();
    assert_eq!(kinds, sim_kinds);
    for kind in kinds {
        let (a, b) = (sim.info(kind).unwrap(), real.info(kind).unwrap());
        assert_eq!(
            (a.input, a.output, &a.needs, &a.gives, a.passes_through, &a.right_after, &a.unless, a.repeatable),
            (b.input, b.output, &b.needs, &b.gives, b.passes_through, &b.right_after, &b.unless, b.repeatable),
            "{kind}"
        );
    }
}

/// A step these steps don't know (from a newer version) hands on something
/// unknown, found unknown things, and so makes no misfit after it, even
/// through a Folder that hands on what it's given.
#[test]
fn nothing_after_an_unknown_step_is_a_misfit() {
    use trip_engine::first_misfit;
    use trip_engine::recipe::chain;
    for (which, reg) in registries() {
        let after_unknown = |rest: &[(&str, &str)]| {
            let mut v = vec![Node::step("d", "drive"), Node::step("c", "audio-cd"), Node::step("r", "rip"), Node::step("x", "from-the-future")];
            v.extend(rest.iter().map(|(id, s)| Node::step(id, s)));
            first_misfit(&chain(v), &reg)
        };
        assert_eq!(after_unknown(&[("f", "folder"), ("h", "hold"), ("f2", "folder")]), None, "{which}");
        assert_eq!(after_unknown(&[("f", "folder"), ("l", "libcrypt")]), None, "{which}");
        assert_eq!(after_unknown(&[("i", "chd"), ("f", "folder"), ("l", "libcrypt")]), None, "{which}");
        // Without one, the Library of FLAC files that offered LibCrypt.
        let lib = chain([Node::step("d", "drive"), Node::step("c", "audio-cd"), Node::step("r", "rip"), Node::step("e", "flac"), Node::step("f", "folder"), Node::step("l", "libcrypt")]);
        assert_eq!(first_misfit(&lib, &reg).as_deref(), Some("l"), "{which}");
    }
}
