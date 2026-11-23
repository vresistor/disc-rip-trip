//! Redump's catalogues: the hashes of known-good dumps, and their names.
//!
//! Every system's datfile is the same Logiqx XML, published as a zip at
//! `http://redump.org/datfile/<system>/`. One is fetched when Check first
//! needs it, kept in the cache for a fortnight, and compared offline after
//! that. A stale copy is better than none: a failed download falls back to
//! it.
//!
//! Lifted from the old app's `redump.rs`. A data disc is matched by the
//! SHA-1 of every track, which needs no knowledge of the system at all; a
//! game is also found by its serial before it's dumped, so the datfile is
//! fetched with its `<serial>` elements (`/serial,version`: the plain one
//! has none).

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// How long a datfile is used before a fresh one is fetched.
const MAX_AGE: Duration = Duration::from_secs(14 * 24 * 3600);

/// A system Redump catalogues whose discs an ordinary PC drive reads.
#[derive(Clone, Copy, Debug)]
pub struct System {
    /// Redump's name for it in URLs.
    pub slug: &'static str,
    pub name: &'static str,
    /// The media its discs come on.
    pub media: &'static [&'static str],
}

const CD: &[&str] = &["CD"];

/// Every PC-readable system, most likely first for a data disc. Left out:
/// GameCube, Wii, Xbox, Dreamcast GD-ROM and PSP UMD, which need special
/// drives.
pub const SYSTEMS: &[System] = &[
    System { slug: "pc", name: "IBM PC", media: &["CD", "DVD", "Blu-ray"] },
    System { slug: "mac", name: "Apple Macintosh", media: &["CD", "DVD"] },
    System { slug: "psx", name: "PlayStation", media: CD },
    System { slug: "ps2", name: "PlayStation 2", media: &["CD", "DVD"] },
    System { slug: "ps3", name: "PlayStation 3", media: &["Blu-ray"] },
    System { slug: "ss", name: "Sega Saturn", media: CD },
    System { slug: "mcd", name: "Sega Mega CD", media: CD },
    System { slug: "pce", name: "PC Engine CD", media: CD },
    System { slug: "pc-fx", name: "PC-FX", media: CD },
    System { slug: "3do", name: "3DO", media: CD },
    System { slug: "cdi", name: "Philips CD-i", media: CD },
    System { slug: "ngcd", name: "Neo Geo CD", media: CD },
    System { slug: "cd32", name: "Amiga CD32", media: CD },
    System { slug: "cdtv", name: "Commodore CDTV", media: CD },
    System { slug: "acd", name: "Amiga CD", media: CD },
    System { slug: "fmt", name: "FM Towns", media: CD },
    System { slug: "pc-98", name: "PC-98", media: CD },
    System { slug: "x68k", name: "Sharp X68000", media: CD },
    System { slug: "arch", name: "Acorn Archimedes", media: CD },
    System { slug: "pippin", name: "Bandai Pippin", media: CD },
    System { slug: "photo-cd", name: "Photo CD", media: CD },
];

pub const ALL: &str = "All PC-readable";

/// The choices for Check's setting: all of them, or one system.
pub const CHOICES: &[&str] = &[
    ALL,
    "IBM PC",
    "Apple Macintosh",
    "PlayStation",
    "PlayStation 2",
    "PlayStation 3",
    "Sega Saturn",
    "Sega Mega CD",
    "PC Engine CD",
    "PC-FX",
    "3DO",
    "Philips CD-i",
    "Neo Geo CD",
    "Amiga CD32",
    "Commodore CDTV",
    "Amiga CD",
    "FM Towns",
    "PC-98",
    "Sharp X68000",
    "Acorn Archimedes",
    "Bandai Pippin",
    "Photo CD",
];

/// [`CHOICES`] as shown: the first in words, the systems by name.
pub const LABELS: &[&str] = &[
    "All systems",
    "IBM PC",
    "Apple Macintosh",
    "PlayStation",
    "PlayStation 2",
    "PlayStation 3",
    "Sega Saturn",
    "Sega Mega CD",
    "PC Engine CD",
    "PC-FX",
    "3DO",
    "Philips CD-i",
    "Neo Geo CD",
    "Amiga CD32",
    "Commodore CDTV",
    "Amiga CD",
    "FM Towns",
    "PC-98",
    "Sharp X68000",
    "Acorn Archimedes",
    "Bandai Pippin",
    "Photo CD",
];

/// The systems to look in for a setting and a disc's media: a DVD can't be
/// a Saturn disc, so its catalogue isn't fetched.
pub fn systems_for(choice: &str, media: &str) -> Vec<System> {
    SYSTEMS.iter().filter(|s| if choice == ALL || choice.is_empty() { media.is_empty() || s.media.contains(&media) } else { s.name == choice }).copied().collect()
}

/// One file of a catalogued dump: a track, the ISO, or the cue sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rom {
    pub name: String,
    pub size: u64,
    pub crc: String,
    pub md5: String,
    pub sha1: String,
}

impl Rom {
    /// Whether it's data off the disc. A cue sheet is left out of every
    /// comparison: it names the tracks, so its hash depends on what the
    /// dump was called, which is the question being asked.
    pub fn is_data(&self) -> bool {
        !self.name.to_ascii_lowercase().ends_with(".cue")
    }
}

/// Whether the files `dumped` (hashed from disk) are `entry`'s, on all
/// four counts the datfile lists for each data file: size, CRC-32, MD5 and
/// SHA-1, in the cue's order. Gives a line per file saying what was
/// compared, or what differs.
pub fn same_files(entry: &Game, dumped: &[Rom]) -> std::result::Result<Vec<String>, String> {
    let theirs: Vec<&Rom> = entry.roms.iter().filter(|r| r.is_data()).collect();
    let ours: Vec<&Rom> = dumped.iter().filter(|r| r.is_data()).collect();
    if theirs.len() != ours.len() || theirs.is_empty() {
        return Err(format!("{} has {} data files and this copy {}", entry.name, theirs.len(), ours.len()));
    }
    let mut lines = vec![];
    for (t, o) in theirs.iter().zip(&ours) {
        let eq = |a: &str, b: &str| !a.is_empty() && a.eq_ignore_ascii_case(b);
        let checks = [("size", t.size == o.size), ("CRC-32", eq(&t.crc, &o.crc)), ("MD5", eq(&t.md5, &o.md5)), ("SHA-1", eq(&t.sha1, &o.sha1))];
        let said = format!(
            "{}: size {}, CRC-32 {}, MD5 {}, SHA-1 {} against Redump's {} \"{}\" (size {}, CRC-32 {}, MD5 {}, SHA-1 {})",
            o.name, o.size, o.crc, o.md5, o.sha1, entry.name, t.name, t.size, t.crc, t.md5, t.sha1
        );
        match checks.iter().filter(|c| !c.1).map(|c| c.0).collect::<Vec<_>>() {
            bad if bad.is_empty() => lines.push(format!("{said}: all four the same")),
            bad => return Err(format!("{said}: {} differ", bad.join(", "))),
        }
    }
    Ok(lines)
}

/// One catalogued disc.
#[derive(Clone, Debug)]
pub struct Game {
    /// Redump's name for it: `Encarta 98 (USA) (Disc 1)`.
    pub name: String,
    /// Every serial it's known by, as written (`SLPM 86770, SLPS 00222`
    /// is two). A serial names a game, not a pressing: `SLUS-00152` is six
    /// Tomb Raider revisions.
    pub serials: Vec<String>,
    pub roms: Vec<Rom>,
}

/// One system's datfile.
#[derive(Debug, Default)]
pub struct Catalogue {
    /// The header's version: the date it was built.
    pub version: String,
    pub games: Vec<Game>,
    by_sha1: HashMap<String, Vec<usize>>,
    by_serial: HashMap<String, Vec<usize>>,
    /// The serials Redump writes without letters, by their digits: Sonic
    /// CD's is `4407`, where the disc says `MK-4407`. Only those, so a
    /// serial missing from the catalogue doesn't find other games that
    /// share its number under another prefix.
    by_number: HashMap<String, Vec<usize>>,
}

impl Catalogue {
    /// The entry a dump is byte for byte: every data file's SHA-1 matches,
    /// and there are as many. A dump missing a track is not the same disc.
    pub fn by_hashes(&self, dumped: &[Rom]) -> Option<&Game> {
        let mut mine: Vec<&str> = dumped.iter().filter(|r| r.is_data()).map(|r| r.sha1.as_str()).collect();
        mine.sort_unstable();
        let first = mine.first()?;
        self.by_sha1.get(*first)?.iter().map(|&i| &self.games[i]).find(|g| {
            let mut theirs: Vec<&str> = g.roms.iter().filter(|r| r.is_data()).map(|r| r.sha1.as_str()).collect();
            theirs.sort_unstable();
            theirs == mine
        })
    }

    /// Every entry known by this serial, however either side spells it;
    /// failing that, by its digits.
    pub fn by_serial(&self, serial: &str) -> Vec<&Game> {
        let pick = |m: &HashMap<String, Vec<usize>>, k: String| m.get(&k).map(|v| v.iter().map(|&i| &self.games[i]).collect::<Vec<_>>());
        let key = serial_key(serial);
        if key.is_empty() {
            return vec![];
        }
        pick(&self.by_serial, key).or_else(|| pick(&self.by_number, serial_digits(serial)).filter(|_| serial_digits(serial).len() >= 4)).unwrap_or_default()
    }

    /// Whether it was fetched with its serials.
    pub fn has_serials(&self) -> bool {
        !self.by_serial.is_empty()
    }
}

/// A serial as both sides are compared: letters and digits, upper-cased.
/// `SLUS_202.16` (the disc), `SLUS-20216` and `slus 20216` are one serial.
pub fn serial_key(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect()
}

fn serial_digits(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_digit).collect()
}

/// Reads a Logiqx datfile.
pub fn parse(xml: &[u8]) -> Result<Catalogue> {
    use quick_xml::events::Event;
    // quick-xml refuses bytes that aren't UTF-8; a stray one in a datfile
    // costs that one character, not the whole catalogue.
    let xml = String::from_utf8_lossy(xml);
    let mut reader = quick_xml::Reader::from_reader(xml.as_bytes());
    reader.config_mut().trim_text(true);
    let mut out = Catalogue::default();
    let mut game: Option<Game> = None;
    let (mut in_header, mut in_version, mut in_serial) = (false, false, false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "header" => in_header = true,
                "game" | "machine" => game = Some(Game { name: attr(&e, "name").unwrap_or_default(), serials: vec![], roms: vec![] }),
                "version" => in_version = in_header,
                "serial" => in_serial = game.is_some(),
                _ => {}
            },
            Ok(Event::Empty(e)) if e.name().as_ref() == "rom" => {
                if let Some(g) = &mut game {
                    g.roms.push(Rom {
                        name: attr(&e, "name").unwrap_or_default(),
                        size: attr(&e, "size").and_then(|v| v.parse().ok()).unwrap_or(0),
                        crc: attr(&e, "crc").unwrap_or_default().to_ascii_lowercase(),
                        md5: attr(&e, "md5").unwrap_or_default().to_ascii_lowercase(),
                        sha1: attr(&e, "sha1").unwrap_or_default().to_ascii_lowercase(),
                    });
                }
            }
            Ok(Event::Text(t)) if in_version => {
                out.version = t.trim().to_owned();
                in_version = false;
            }
            Ok(Event::Text(t)) if in_serial => {
                let text = String::from(&*t);
                let text = quick_xml::escape::unescape(&text).map(|s| s.into_owned()).unwrap_or(text);
                if let Some(g) = &mut game {
                    g.serials.extend(text.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()));
                }
                in_serial = false;
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                "header" => in_header = false,
                "game" | "machine" => out.games.extend(game.take()),
                _ => (in_version, in_serial) = (false, false),
            },
            Ok(Event::Eof) => break,
            Err(e) => bail!("the datfile couldn't be read at byte {}: {e}", reader.buffer_position()),
            _ => {}
        }
        buf.clear();
    }
    if out.games.is_empty() {
        bail!("the datfile has no entries");
    }
    for (i, g) in out.games.iter().enumerate() {
        for r in g.roms.iter().filter(|r| r.is_data()) {
            out.by_sha1.entry(r.sha1.clone()).or_default().push(i);
        }
        for s in &g.serials {
            let key = serial_key(s);
            if key.is_empty() {
                continue;
            }
            let numeric = key.chars().all(|c| c.is_ascii_digit());
            // An entry can list one serial twice in two spellings.
            let entry = out.by_serial.entry(key).or_default();
            if entry.last() != Some(&i) {
                entry.push(i);
            }
            if numeric {
                let entry = out.by_number.entry(serial_digits(s)).or_default();
                if entry.last() != Some(&i) {
                    entry.push(i);
                }
            }
        }
    }
    Ok(out)
}

/// An attribute, with its entities resolved: names are full of `&amp;`.
fn attr(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    let a = e.attributes().flatten().find(|a| a.key.as_ref() == name)?;
    Some(a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.trim().to_owned()).unwrap_or_else(|_| a.value.trim().to_owned()))
}

// ---- the cache -----------------------------------------------------------------------------------

/// Parsed catalogues, kept for the life of the process: the PC one is
/// tens of megabytes of XML, and a box of discs asks for it every disc.
fn memo() -> &'static Mutex<HashMap<PathBuf, Arc<Catalogue>>> {
    static MEMO: OnceLock<Mutex<HashMap<PathBuf, Arc<Catalogue>>>> = OnceLock::new();
    MEMO.get_or_init(Mutex::default)
}

/// Marks a cached catalogue as out of date, so the next ask fetches it
/// again (and falls back to this copy if that fails): for one fetched
/// before its serials were asked for.
pub fn expire(cache: &Path, slug: &str) {
    let path = cache.join(format!("{slug}.dat"));
    memo().lock().unwrap().remove(&path);
    if let Ok(f) = std::fs::File::options().write(true).open(&path) {
        let _ = f.set_modified(std::time::UNIX_EPOCH);
    }
}

fn stale(path: &Path) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_none_or(|age| age > MAX_AGE)
}

/// One system's catalogue, from `cache` (`…/disc-rip-trip/cache/redump`)
/// when it's there and fresh, otherwise through `fetch`, which gives the
/// datfile's XML. A failed fetch falls back to a stale copy. Parsed on a
/// blocking thread.
pub async fn catalogue<F, Fut>(cache: &Path, slug: &str, fetch: F) -> Result<Arc<Catalogue>>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<Vec<u8>>>,
{
    let path = cache.join(format!("{slug}.dat"));
    let fresh = !stale(&path);
    if fresh {
        if let Some(c) = memo().lock().unwrap().get(&path).cloned() {
            return Ok(c);
        }
    } else {
        match fetch(slug.to_owned()).await {
            Ok(xml) => {
                std::fs::create_dir_all(cache)?;
                // Through a temporary, so a half-written file can't pass for
                // a catalogue with most of its discs missing.
                super::place::write(&path, &xml)?;
                memo().lock().unwrap().remove(&path);
            }
            Err(e) if !path.is_file() => return Err(e),
            Err(_) => {}
        }
    }
    if let Some(c) = memo().lock().unwrap().get(&path).cloned() {
        return Ok(c);
    }
    let file = path.clone();
    let parsed = tokio::task::spawn_blocking(move || -> Result<Catalogue> {
        let xml = std::fs::read(&file).with_context(|| format!("couldn't read {}", file.display()))?;
        parse(&xml)
    })
    .await??;
    let parsed = Arc::new(parsed);
    memo().lock().unwrap().insert(path, parsed.clone());
    Ok(parsed)
}

/// Fetches a datfile from redump.org, with its serials, and unpacks the
/// XML inside the zip.
pub async fn download(slug: String) -> Result<Vec<u8>> {
    let url = format!("http://redump.org/datfile/{slug}/serial,version");
    let r = super::net::plain(Duration::from_secs(120))?.get(&url).send().await?;
    if !r.status().is_success() {
        return Err(anyhow!("redump.org said {} for {slug}", r.status().as_u16()));
    }
    let zipped = super::net::body(r, super::net::FILE_MAX).await?;
    tokio::task::spawn_blocking(move || unzip(&zipped)).await?
}

/// The one file inside a datfile's zip.
pub fn unzip(zipped: &[u8]) -> Result<Vec<u8>> {
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(zipped)).context("redump.org sent something that isn't a zip")?;
    if a.is_empty() {
        bail!("the datfile's zip is empty");
    }
    let f = a.by_index(0)?;
    // The largest datfile unpacks to a few tens of MB.
    let claimed = f.size();
    super::net::read_at_most(f, claimed, 512 * super::net::MB)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A datfile in Redump's shape, with the disc the simulated steps image.
    pub use crate::sim::REDUMP_DAT as DAT;

    fn rom(name: &str, sha1: &str) -> Rom {
        Rom { name: name.into(), size: 0, crc: String::new(), md5: String::new(), sha1: sha1.into() }
    }

    #[test]
    fn a_datfile_becomes_a_catalogue() {
        let c = parse(DAT.as_bytes()).unwrap();
        assert_eq!(c.version, "2026-06-15 06-43-57");
        assert_eq!(c.games.len(), 2);
        assert_eq!(c.games[1].name, "Myst & Riven (Europe)", "entities resolved");
        assert_eq!(c.games[0].roms[0].sha1, "aaaa000000000000000000000000000000000001", "hashes lower-cased");
    }

    #[test]
    fn a_byte_that_isnt_utf8_costs_one_character_not_the_datfile() {
        let mut dat = DAT.replace("Myst &amp; Riven", "Myst \u{1} Riven").into_bytes();
        let at = dat.iter().position(|&b| b == 1).unwrap();
        dat[at] = 0xE9; // Latin-1 é
        let c = parse(&dat).unwrap();
        assert_eq!(c.games.len(), 2);
        assert_eq!(c.games[1].name, "Myst \u{FFFD} Riven (Europe)");
        assert_eq!(c.games[0].roms[0].sha1, "aaaa000000000000000000000000000000000001");
    }

    #[test]
    fn a_dump_matches_by_every_data_track_and_not_its_cue() {
        let c = parse(DAT.as_bytes()).unwrap();
        let dumped = [rom("MY_DISC.bin", "aaaa000000000000000000000000000000000001"), rom("MY_DISC.cue", "ffff")];
        assert_eq!(c.by_hashes(&dumped).unwrap().name, "Encarta 98 Encyclopedia (USA) (Disc 1)");
        // Both tracks, in any order.
        let two = [rom("b", "bbbb000000000000000000000000000000000002"), rom("a", "bbbb000000000000000000000000000000000001")];
        assert_eq!(c.by_hashes(&two).unwrap().name, "Myst & Riven (Europe)");
        // One track of two is not the disc.
        assert!(c.by_hashes(&two[..1]).is_none());
        assert!(c.by_hashes(&[rom("x.iso", "0000")]).is_none(), "not in Redump");
        assert!(c.by_hashes(&[]).is_none());
    }

    #[test]
    fn which_systems_are_looked_in() {
        let dvd: Vec<&str> = systems_for(ALL, "DVD").iter().map(|s| s.slug).collect();
        assert_eq!(dvd, ["pc", "mac", "ps2"]);
        assert_eq!(systems_for(ALL, "Blu-ray").iter().map(|s| s.slug).collect::<Vec<_>>(), ["pc", "ps3"]);
        assert!(systems_for(ALL, "CD").len() > 15);
        assert_eq!(systems_for("Sega Saturn", "CD")[0].slug, "ss");
        for s in SYSTEMS {
            assert!(CHOICES.contains(&s.name), "{} can be chosen", s.name);
        }
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::current_dir().unwrap().join("../../target/drt-test").join(name);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[tokio::test]
    async fn a_catalogue_is_fetched_once_then_read_from_the_cache() {
        let cache = dir("redump-cache");
        let c = catalogue(&cache, "pc", |_| async { Ok(DAT.as_bytes().to_vec()) }).await.unwrap();
        assert_eq!(c.games.len(), 2);
        assert!(cache.join("pc.dat").is_file());
        // Fresh in the cache: not fetched again.
        let c = catalogue(&cache, "pc", |_| async { Err(anyhow!("offline")) }).await.unwrap();
        assert_eq!(c.games.len(), 2);
        // Nothing cached and no network: an error, which Check reports as
        // "couldn't check", not as a bad dump.
        assert!(catalogue(&cache, "mac", |_| async { Err(anyhow!("offline")) }).await.is_err());
    }

    #[test]
    fn the_zip_redump_sends_is_unpacked() {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        z.start_file("IBM - PC compatible - Datfile.dat", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(DAT.as_bytes()).unwrap();
        let bytes = z.finish().unwrap().into_inner();
        assert_eq!(unzip(&bytes).unwrap(), DAT.as_bytes());
        assert!(unzip(b"<html>").is_err());
    }

    /// Downloads one real datfile. `cargo test -p drt-steps real_datfile -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn real_datfile() {
        let cache = dir("redump-real");
        let slug = std::env::var("DRT_SLUG").unwrap_or_else(|_| "pippin".into());
        let t = std::time::Instant::now();
        let c = catalogue(&cache, &slug, download).await.unwrap();
        println!("{slug}: {:?}", t.elapsed());
        println!("{} entries, version {}, first {:?}", c.games.len(), c.version, c.games.first().map(|g| &g.name));
    }
}
