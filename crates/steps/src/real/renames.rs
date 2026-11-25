//! `drt rename-full-copies`: plain images filed before Plain image named
//! its files as Redump does, renamed to Redump's names.
//!
//! Each cue sheet in a folder (and the folders in it) is a set: the sheet,
//! the tracks it names, and beside it a log or a LibCrypt `.sbi` of the
//! same name. A set is renamed only when every one of its tracks is, by
//! SHA-1, a track of one entry in a Redump list, and then to that entry's
//! names; its sheet is written anew with the new track names, every other
//! byte as it was. Nothing is ever written over: a set whose new name is
//! taken is left as it is.

use super::redump::{Catalogue, Game};
use super::{data, hash, redumper};
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// What to do with one set.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    /// Each file and its new name, beside it; the sheet first.
    Rename { game: String, moves: Vec<(PathBuf, PathBuf)> },
    /// Already named as Redump names it.
    AlreadyNamed { game: String },
    /// Its tracks aren't any one entry's.
    NoMatch,
    /// Left as it is, and why.
    Left(String),
}

/// The cue sheets under `folder`, sorted.
pub fn sheets(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut out = vec![];
    let mut dirs = vec![folder.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d)? {
            let p = e?.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue")) {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The entry among `lists` whose tracks these are, by their hashes.
pub fn find<'a>(lists: &'a [Catalogue], roms: &[super::redump::Rom]) -> Option<&'a Game> {
    lists.iter().find_map(|c| c.by_hashes(roms))
}

/// A name from a list fetched over the network, if it's a plain file name:
/// one part, no folder above or below it, nothing a cue sheet's quotes or
/// a terminal would read as more than text. Redump's lists come over plain
/// HTTP, so a name can't be trusted to stay in its folder.
fn plain_name(name: &str) -> Option<&str> {
    let one = matches!(Path::new(name).components().collect::<Vec<_>>()[..], [std::path::Component::Normal(_)]);
    let clean = !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | '"'));
    (one && clean && !name.starts_with('.') && name.len() <= 240).then_some(name)
}

/// What to do with the set whose sheet is `cue`, given its tracks' hashes
/// (in the sheet's order) and the entry they are, if any.
pub fn plan(cue: &Path, roms: &[super::redump::Rom], game: Option<&Game>) -> Plan {
    let Some(game) = game else { return Plan::NoMatch };
    let dir = cue.parent().unwrap_or(Path::new("."));
    let tracks = data::image_files(cue);
    let mut moves: Vec<(PathBuf, PathBuf)> = vec![];
    let sheet = game.roms.iter().find(|r| !r.is_data()).map_or_else(|| format!("{}.cue", game.name), |r| r.name.clone());
    let Some(sheet) = plain_name(&sheet) else {
        return Plan::Left(format!("Redump's name for {} isn't a plain file name, so it's left as it is", game.name));
    };
    moves.push((cue.to_path_buf(), dir.join(sheet)));
    for (t, r) in tracks.iter().zip(roms) {
        let Some(theirs) = game.roms.iter().find(|g| g.is_data() && g.sha1.eq_ignore_ascii_case(&r.sha1)) else {
            return Plan::Left(format!("{} isn't one of {}'s tracks", r.name, game.name));
        };
        let Some(name) = plain_name(&theirs.name) else {
            return Plan::Left(format!("Redump's name for {} isn't a plain file name, so it's left as it is", r.name));
        };
        moves.push((t.clone(), dir.join(name)));
    }
    // Its log and LibCrypt file, named as the sheet is.
    let (old_stem, new_stem) = (cue.file_stem().unwrap_or_default().to_string_lossy().into_owned(), Path::new(sheet).file_stem().unwrap_or_default().to_string_lossy().into_owned());
    for ext in ["log", "sbi"] {
        let f = dir.join(format!("{old_stem}.{ext}"));
        if f.is_file() {
            moves.push((f, dir.join(format!("{new_stem}.{ext}"))));
        }
    }
    if moves.iter().all(|(a, b)| a == b) {
        return Plan::AlreadyNamed { game: game.name.clone() };
    }
    let ours: Vec<&PathBuf> = moves.iter().map(|m| &m.0).collect();
    for (from, to) in &moves {
        if from != to && to.exists() && !ours.contains(&to) {
            return Plan::Left(format!("{} is there already", to.file_name().unwrap_or_default().to_string_lossy()));
        }
    }
    let mut names: Vec<&PathBuf> = moves.iter().map(|m| &m.1).collect();
    names.sort();
    names.dedup();
    if names.len() != moves.len() {
        return Plan::Left("two of its files would get the same name".into());
    }
    Plan::Rename { game: game.name.clone(), moves }
}

/// Renames a set as planned: the tracks and the rest first (each by way of
/// a name of its own, so two that swap names don't meet), then the sheet,
/// written anew with the new track names, and the old one removed.
pub fn apply(moves: &[(PathBuf, PathBuf)]) -> Result<()> {
    let (cue, rest) = moves.split_first().ok_or_else(|| anyhow!("nothing to rename"))?;
    let text = std::fs::read_to_string(&cue.0)?;
    let names: Vec<(String, String)> = rest.iter().map(|(a, b)| (file_name(a), file_name(b))).collect();
    let mut parked = vec![];
    for (k, (from, to)) in rest.iter().enumerate().filter(|(_, (a, b))| a != b) {
        let tmp = to.with_file_name(format!(".drt-rename-{k}-{}", file_name(to)));
        std::fs::rename(from, &tmp)?;
        parked.push((tmp, to));
    }
    for (tmp, to) in parked {
        std::fs::rename(tmp, to)?;
    }
    let new = data::rename_in_cue(&text, &names);
    let tmp = cue.1.with_file_name(format!(".drt-rename-{}", file_name(&cue.1)));
    std::fs::write(&tmp, new)?;
    std::fs::rename(&tmp, &cue.1)?;
    if cue.0 != cue.1 {
        std::fs::remove_file(&cue.0)?;
    }
    Ok(())
}

fn file_name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// The set's tracks hashed, in its sheet's order.
pub fn hash_set(cue: &Path) -> Result<Vec<super::redump::Rom>> {
    let tracks = data::image_files(cue);
    if tracks.is_empty() || tracks.iter().any(|t| !t.is_file()) {
        return Err(anyhow!("a track it names isn't there"));
    }
    let refs: Vec<&Path> = tracks.iter().map(PathBuf::as_path).collect();
    hash::hash_files(&refs, |_| {}, || false)
}

/// The Redump lists the app keeps: under its cache, and under the
/// flatpak's.
pub fn cached_lists() -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Some(c) = drt_engine::run::cache_dir() {
        dirs.push(c.join("cache/redump"));
    }
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(h).join(".var/app/io.github.vresistor.disc-rip-trip/cache/disc-rip-trip/cache/redump"));
    }
    let mut out: Vec<PathBuf> = dirs.iter().filter_map(|d| std::fs::read_dir(d).ok()).flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "dat")).collect();
    out.sort();
    out
}

/// The files a sheet names, for the listing: `redumper::cue_files`.
pub fn named_in(cue: &Path) -> Vec<String> {
    redumper::cue_files(&std::fs::read_to_string(cue).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_from_redump_that_leaves_the_folder_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let cue = d.path().join("ours.cue");
        std::fs::write(&cue, "FILE \"ours.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n").unwrap();
        std::fs::write(d.path().join("ours.bin"), "x").unwrap();
        let roms = [super::super::redump::Rom { name: "ours.bin".into(), size: 1, crc: "0".into(), md5: "0".into(), sha1: "aa".into() }];
        for bad in ["../outside.bin", "/home/u/.profile", "a/b.bin", "x\"\nFILE \"y.bin", ".hidden.bin"] {
            let g = game("Evil (Europe)", &[(bad, "aa")]);
            assert!(matches!(plan(&cue, &roms, Some(&g)), Plan::Left(_)), "{bad:?} was taken");
        }
        let mut g = game("Evil (Europe)", &[("fine.bin", "aa")]);
        g.roms[0].name = "../../evil.cue".into();
        assert!(matches!(plan(&cue, &roms, Some(&g)), Plan::Left(_)), "a sheet name outside the folder was taken");
    }
    use crate::real::redump::Rom;

    fn game(name: &str, tracks: &[(&str, &str)]) -> Game {
        let mut roms = vec![Rom { name: format!("{name}.cue"), size: 0, crc: String::new(), md5: String::new(), sha1: "cue".into() }];
        roms.extend(tracks.iter().map(|(n, s)| Rom { name: (*n).into(), size: 0, crc: String::new(), md5: String::new(), sha1: (*s).into() }));
        Game { name: name.into(), serials: vec![], roms }
    }

    fn rom(name: &str, sha1: &str) -> Rom {
        Rom { name: name.into(), size: 0, crc: String::new(), md5: String::new(), sha1: sha1.into() }
    }

    fn folder(name: &str) -> PathBuf {
        let d = std::env::current_dir().unwrap().join("../../target/drt-test").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_set_is_renamed_to_its_entry_s_names_and_its_sheet_rewritten() {
        let d = folder("renames-one");
        std::fs::write(d.join("KOUDELKA_1.cue"), "FILE \"KOUDELKA_1.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n").unwrap();
        std::fs::write(d.join("KOUDELKA_1.bin"), "data").unwrap();
        std::fs::write(d.join("KOUDELKA_1.log"), "log").unwrap();
        let g = game("Koudelka (Europe) (Disc 1)", &[("Koudelka (Europe) (Disc 1).bin", "aa")]);
        let roms = [rom("KOUDELKA_1.bin", "AA")];
        let Plan::Rename { moves, .. } = plan(&d.join("KOUDELKA_1.cue"), &roms, Some(&g)) else { panic!() };
        let to: Vec<String> = moves.iter().map(|m| file_name(&m.1)).collect();
        assert_eq!(to, ["Koudelka (Europe) (Disc 1).cue", "Koudelka (Europe) (Disc 1).bin", "Koudelka (Europe) (Disc 1).log"]);
        apply(&moves).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("Koudelka (Europe) (Disc 1).cue")).unwrap(), "FILE \"Koudelka (Europe) (Disc 1).bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n");
        assert_eq!(std::fs::read_to_string(d.join("Koudelka (Europe) (Disc 1).bin")).unwrap(), "data");
        assert!(!d.join("KOUDELKA_1.cue").exists() && !d.join("KOUDELKA_1.bin").exists());
        // Once named, there's nothing to do.
        let p = plan(&d.join("Koudelka (Europe) (Disc 1).cue"), &roms, Some(&g));
        assert_eq!(p, Plan::AlreadyNamed { game: g.name.clone() });
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn several_tracks_take_their_own_names_and_nothing_is_written_over() {
        let d = folder("renames-tracks");
        std::fs::write(d.join("disc.cue"), "FILE \"disc (Track 1).bin\" BINARY\nFILE \"disc (Track 2).bin\" BINARY\n").unwrap();
        std::fs::write(d.join("disc (Track 1).bin"), "1").unwrap();
        std::fs::write(d.join("disc (Track 2).bin"), "2").unwrap();
        let g = game("Game (USA)", &[("Game (USA) (Track 1).bin", "s1"), ("Game (USA) (Track 2).bin", "s2")]);
        let roms = [rom("disc (Track 1).bin", "s1"), rom("disc (Track 2).bin", "s2")];
        // Someone's file is in the way: left alone.
        std::fs::write(d.join("Game (USA) (Track 2).bin"), "theirs").unwrap();
        assert!(matches!(plan(&d.join("disc.cue"), &roms, Some(&g)), Plan::Left(_)));
        std::fs::remove_file(d.join("Game (USA) (Track 2).bin")).unwrap();
        let Plan::Rename { moves, .. } = plan(&d.join("disc.cue"), &roms, Some(&g)) else { panic!() };
        apply(&moves).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("Game (USA).cue")).unwrap(), "FILE \"Game (USA) (Track 1).bin\" BINARY\nFILE \"Game (USA) (Track 2).bin\" BINARY\n");
        assert_eq!(std::fs::read_to_string(d.join("Game (USA) (Track 2).bin")).unwrap(), "2");
        // A track that isn't the entry's, and no entry at all.
        assert!(matches!(plan(&d.join("Game (USA).cue"), &[rom("a", "s1"), rom("b", "zz")], Some(&g)), Plan::Left(_)));
        assert_eq!(plan(&d.join("Game (USA).cue"), &roms, None), Plan::NoMatch);
        assert_eq!(sheets(&d).unwrap(), [d.join("Game (USA).cue")]);
        std::fs::remove_dir_all(d).unwrap();
    }
}
