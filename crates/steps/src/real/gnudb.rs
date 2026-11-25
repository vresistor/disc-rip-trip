//! GnuDB: the CDDB protocol over HTTP, from the table of contents Audio CD
//! already read.
//!
//! `cddb query <discid> <ntrks> <offsets…> <nsecs>` finds entries for the
//! disc's CDDB ID; `cddb read <category> <discid>` gives one in xmcd form.
//! A CDDB ID is only a checksum of the track layout, and different discs
//! share one, so a match here is weaker evidence than a MusicBrainz disc
//! ID, and scored lower.
//!
//! GnuDB's access policy (gnudb.org/howtognudb.php, read 2026-11-19)
//! refuses a `hello` without a contact email: a generic one gets `500
//! Unknown application / developer email`. The user's details are never
//! made up or taken from the system, so GnuDB asks for one in its
//! app-wide settings and says "Not set up" until it has it.

// What's parsed here comes from the network: no indexing that can panic.
#![warn(clippy::indexing_slicing, clippy::string_slice)]

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use drt_engine::setting::ParamKind;
use drt_engine::{Candidate, Cx, Fact, Group, Inlet, ItemKind, LaneKind, Outlet, ParamSpec, Step, StepInfo, Subject};
use std::time::Duration;

const URL: &str = "https://gnudb.gnudb.org/~cddb/cddb.cgi";
/// The application, as GnuDB's policy asks: a descriptive name, and its version.
const APP: &str = "disc-rip-trip";
const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// The `hello` GnuDB asks for, `user host app version`, from the contact
/// email the user gave: `name+host.com+PortoBigCDPlayer+v2.5.0` in its own
/// example.
pub fn hello(contact: &str) -> String {
    let (user, host) = contact.trim().split_once('@').unwrap_or((contact.trim(), "localhost"));
    format!("{user} {host} {APP} {VERSION}")
}
const SOURCE: &str = "GnuDB";

pub fn info() -> StepInfo {
    StepInfo::new("gnudb", "GnuDB", Group::LookUp, ItemKind::AudioDisc, ItemKind::AudioDisc)
        .gives(&["candidates"])
        .unless(&["release"])
        .right_after(&["audio-cd"])
        .from("older CD database")
        // GnuDB requires a way to reach whoever is asking: once, for the app.
        .param(ParamSpec::new("contact", "Contact email", ParamKind::Text).app_wide().default("").help("GnuDB won't answer without one."))
}

/// The disc's layout from the TOC in the context, MusicBrainz's spelling:
/// `first last leadout offset…`, in frames from the start, lead-in included.
#[derive(Clone, Debug, PartialEq)]
pub struct Toc {
    pub leadout: u32,
    pub offsets: Vec<u32>,
}

pub fn parse_toc(toc: &str) -> Option<Toc> {
    let n: Vec<u32> = toc.split([' ', '+']).filter(|s| !s.is_empty()).map(|s| s.parse().ok()).collect::<Option<_>>()?;
    let (first, last, leadout) = (*n.first()?, *n.get(1)?, *n.get(2)?);
    let offsets = n.get(3..)?.to_vec();
    let first_start = *offsets.first()?;
    (last >= first && offsets.len() == (last - first) as usize + 1 && leadout >= first_start).then_some(Toc { leadout, offsets })
}

impl Toc {
    /// The CDDB disc ID: the digit sum of each track's start in seconds,
    /// mod 255; the playing time in seconds; and the track count.
    pub fn disc_id(&self) -> u32 {
        let digits = |mut n: u32| {
            let mut s = 0;
            while n > 0 {
                s += n % 10;
                n /= 10;
            }
            s
        };
        let n: u32 = self.offsets.iter().map(|o| digits(o / 75)).sum();
        let t = (self.leadout / 75).saturating_sub(self.offsets.first().map_or(0, |o| o / 75));
        ((n % 255) << 24) | (t << 8) | self.offsets.len() as u32
    }

    /// `cddb query` with its arguments.
    pub fn query(&self) -> String {
        let offs: Vec<String> = self.offsets.iter().map(u32::to_string).collect();
        format!("cddb query {:08x} {} {} {}", self.disc_id(), self.offsets.len(), offs.join(" "), self.leadout / 75)
    }
}

/// One entry a query found.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub category: String,
    pub id: String,
    /// "Artist / Album".
    pub title: String,
    pub exact: bool,
}

/// Reads a `cddb query` reply: one exact match (200), a list of them (210
/// exact, 211 inexact), or none (202). Anything else is the server's error.
pub fn parse_query(reply: &str) -> Result<Vec<Match>> {
    let mut lines = reply.lines().map(str::trim_end);
    let head = lines.next().unwrap_or("");
    let code = head.get(..3).unwrap_or("");
    let entry = |l: &str, exact| {
        let mut p = l.splitn(3, ' ');
        Some(Match { category: p.next()?.into(), id: p.next()?.into(), title: p.next().unwrap_or("").trim().into(), exact })
    };
    match code {
        "200" => Ok(entry(head.get(4..).unwrap_or("").trim(), true).into_iter().collect()),
        "210" | "211" => {
            let mut out = vec![];
            for l in lines.take_while(|l| *l != ".") {
                let Some(m) = entry(l, code == "210") else { continue };
                // GnuDB answers an unregistered caller inside the list,
                // where the title would be.
                if let Some(msg) = m.title.strip_prefix("500 ") {
                    bail!("GnuDB refused: {msg}");
                }
                out.push(m);
            }
            Ok(out)
        }
        "202" => Ok(vec![]),
        _ => Err(anyhow!("GnuDB said {}", head.trim())),
    }
}

/// An xmcd entry, as `cddb read` gives it after its `210` line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    /// (artist, title) per track; the artist is empty unless the entry
    /// gives one per track ("Artist / Title" on a compilation).
    pub tracks: Vec<(String, String)>,
}

pub fn parse_read(reply: &str) -> Result<Entry> {
    let mut lines = reply.lines();
    let head = lines.next().unwrap_or("");
    if !head.starts_with("210") {
        bail!("GnuDB said {}", head.trim());
    }
    // A key may be split over several lines; the parts join up.
    let mut keys: Vec<(String, String)> = vec![];
    for l in lines.take_while(|l| l.trim_end() != ".") {
        if l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let v = unescape(v.trim_end_matches('\r'));
        match keys.iter_mut().find(|(x, _)| x == k) {
            Some((_, old)) => old.push_str(&v),
            None => keys.push((k.to_owned(), v)),
        }
    }
    let get = |k: &str| keys.iter().find(|(x, _)| x == k).map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
    let dtitle = get("DTITLE");
    let (artist, album) = match dtitle.split_once(" / ") {
        Some((a, b)) => (a.trim().to_owned(), b.trim().to_owned()),
        None => (dtitle.clone(), dtitle.clone()),
    };
    let various = artist.eq_ignore_ascii_case("various") || artist.eq_ignore_ascii_case("various artists");
    let mut tracks = vec![];
    for i in 0.. {
        let k = format!("TTITLE{i}");
        if !keys.iter().any(|(x, _)| *x == k) {
            break;
        }
        let t = get(&k);
        tracks.push(match t.split_once(" / ").filter(|_| various) {
            Some((a, b)) => (a.trim().to_owned(), b.trim().to_owned()),
            None => (String::new(), t),
        });
    }
    Ok(Entry { artist, album, year: get("DYEAR"), genre: get("DGENRE"), tracks })
}

fn unescape(v: &str) -> String {
    v.replace("\\n", "\n").replace("\\t", "\t").replace("\\\\", "\\")
}

/// A reply's text: UTF-8 at protocol level 6, but older entries can be
/// Latin-1, which is kept rather than lost.
pub fn text(body: &[u8]) -> String {
    match std::str::from_utf8(body) {
        Ok(s) => s.to_owned(),
        Err(_) => body.iter().map(|&b| b as char).collect(),
    }
}

pub fn candidate(m: &Match, e: &Entry) -> Candidate {
    let mut facts = vec![];
    let mut put = |subject, key: &str, value: &str| {
        if !value.trim().is_empty() {
            facts.push(Fact { step: String::new(), subject, key: key.into(), value: value.trim().into(), source: SOURCE.into() });
        }
    };
    put(Subject::Disc, "album", &e.album);
    put(Subject::Disc, "album_artist", &e.artist);
    put(Subject::Disc, "year", &e.year);
    put(Subject::Disc, "genre", &e.genre);
    for (i, (artist, title)) in e.tracks.iter().enumerate() {
        put(Subject::Part(i as u32 + 1), "title", title);
        put(Subject::Part(i as u32 + 1), "artist", artist);
    }
    let detail = ["GnuDB", &e.year, &e.genre].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
    Candidate {
        step: String::new(),
        id: format!("gnudb-{}-{}", m.category, m.id),
        source: SOURCE.into(),
        title: format!("{} — {}", e.artist, e.album),
        detail,
        // A checksum of the layout, which other discs share: below a disc ID.
        score: if m.exact { 0.5 } else { 0.35 },
        facts,
    }
}

async fn ask(cmd: &str, contact: &str) -> Result<String> {
    ask_at(URL, cmd, contact).await
}

/// The URL carries your email address, so it's kept out of any error.
async fn ask_at(url: &str, cmd: &str, contact: &str) -> Result<String> {
    let hello = hello(contact);
    let r = super::net::client(Duration::from_secs(15))?.get(url).query(&[("cmd", cmd), ("hello", &hello), ("proto", "6")]).send().await.map_err(super::net::quiet)?;
    if !r.status().is_success() {
        bail!("GnuDB said {}", r.status().as_u16());
    }
    Ok(text(&super::net::body(r, super::net::TEXT_MAX).await?))
}

/// The candidates GnuDB has for a TOC: every match, read one at a time.
pub async fn lookup(toc: &Toc, contact: &str) -> Result<Vec<Candidate>> {
    let found = parse_query(&ask(&toc.query(), contact).await?)?;
    let mut out = vec![];
    for m in found.iter().take(5) {
        let e = parse_read(&ask(&format!("cddb read {} {}", m.category, m.id), contact).await?)?;
        out.push(candidate(m, &e));
    }
    Ok(out)
}

pub struct GnuDb;

#[async_trait]
impl Step for GnuDb {
    fn info(&self) -> StepInfo {
        info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let contact = cx.text("contact");
            let toc = disc.context().get(Subject::Disc, "toc").and_then(parse_toc);
            match (toc, contact.contains('@')) {
                (_, false) => cx.status("Not set up", "Needs a contact email"),
                (None, _) => cx.status("Nothing found", "no track list"),
                (Some(toc), true) => {
                    cx.status("", "Asking by disc ID");
                    let found = {
                        let _net = cx.lane(LaneKind::Network(SOURCE.into())).await;
                        lookup(&toc, &contact).await
                    };
                    match found {
                        Ok(c) if c.is_empty() => cx.status("Nothing found", ""),
                        Ok(c) => {
                            cx.status(format!("{} found", c.len()), "");
                            disc.context_mut().offer(c);
                        }
                        Err(e) => {
                            // Like any lookup: the disc goes on without it.
                            super::lookup_skipped(&cx, &disc, e.to_string());
                            cx.log(format!("lookup failed: {e}"));
                        }
                    }
                }
            }
            outlet.send(disc).await;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::string_slice)]
mod tests {
    use super::*;

    /// The White Stripes (1999), from the old app's run record of the real disc.
    pub const WHITE_STRIPES: &str = "1 17 197195 150 11382 21905 33110 48217 61340 71352 82665 94840 103167 111745 125620 140187 154715 165380 172175 183005";

    #[test]
    fn the_disc_id_and_query_from_the_toc() {
        let t = parse_toc(WHITE_STRIPES).unwrap();
        assert_eq!(t.offsets.len(), 17);
        assert_eq!(format!("{:08x}", t.disc_id()), "da0a4311");
        assert_eq!(t.query(), "cddb query da0a4311 17 150 11382 21905 33110 48217 61340 71352 82665 94840 103167 111745 125620 140187 154715 165380 172175 183005 2629");
        assert_eq!(parse_toc(&WHITE_STRIPES.replace(' ', "+")), Some(t), "as the old app stored it");
        assert_eq!(parse_toc("1 3 1000 150"), None, "three tracks need three offsets");
    }

    #[tokio::test]
    async fn the_email_address_is_in_no_error() {
        // Nothing listens on port 1.
        let e = ask_at("https://127.0.0.1:1/~cddb/cddb.cgi", "cddb query x", "someone.private@example.org").await.unwrap_err();
        let said = format!("{e:#} {e:?}");
        assert!(!said.contains("someone") && !said.contains("example.org"), "{said}");
    }

    #[test]
    fn a_short_or_odd_reply_is_no_panic() {
        assert!(parse_query("200").unwrap().is_empty());
        assert!(parse_query("200é rock x y").is_ok());
        assert!(parse_query("").is_err());
        assert!(parse_query("2").is_err());
        assert!(parse_query("é").is_err());
        assert!(parse_toc("1 2 100 150 120").is_none(), "a leadout before the first track");
        assert!(parse_toc("1 4294967295 1 0").is_none());
        assert!(parse_toc("1").is_none());
        assert!(parse_toc("1 1 150").is_none(), "no tracks");
        assert_eq!(Toc { leadout: 0, offsets: vec![150] }.disc_id() & 0xFF, 1, "no underflow");
    }

    #[test]
    fn hello_is_the_contact_and_the_app() {
        assert_eq!(hello("someone@example.org"), format!("someone example.org disc-rip-trip v{}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn query_replies() {
        let one = parse_query("200 rock da0a4311 The White Stripes / The White Stripes\r\n").unwrap();
        assert_eq!(one, [Match { category: "rock".into(), id: "da0a4311".into(), title: "The White Stripes / The White Stripes".into(), exact: true }]);
        let list = parse_query("211 Found inexact matches, list follows (until terminating `.')\nrock da0a4311 A / B\nmisc da0a4312 C / D\n.\n").unwrap();
        assert_eq!(list.len(), 2);
        assert!(!list[0].exact);
        assert!(parse_query("202 No match for disc ID da0a4311.\n").unwrap().is_empty());
        assert!(parse_query("409 No handshake\n").is_err());
    }

    #[test]
    fn an_unregistered_caller_is_refused_and_says_so() {
        // GnuDB's reply on 2026-11-19 to a hello with no contact email.
        let reply =
            "210 Found exact matches, list follows (until terminating `.')\ndata da0a4217 500 Unknown application / developer email for disc-rip-trip 0.1 see https://gnudb.org/howtognudb.php\n.\n";
        let e = parse_query(reply).unwrap_err().to_string();
        assert!(e.starts_with("GnuDB refused: Unknown application"), "{e}");
    }

    const ENTRY: &str = "210 rock da0a4311 CD database entry follows (until terminating `.')\n# xmcd\n#\n# Track frame offsets:\n#\t150\nDISCID=da0a4311\nDTITLE=The White Stripes / The White Stripes\nDYEAR=1999\nDGENRE=Garage Rock\nTTITLE0=Jimmy the Exploder\nTTITLE1=Stop Breaking Down\nTTITLE2=The Big Three Killed My Baby\nTTITLE3=Suzy Lee and a title long enough that it \nTTITLE3=goes on\nEXTD=\nPLAYORDER=\n.\n";

    #[test]
    fn an_entry_becomes_a_candidate() {
        let e = parse_read(ENTRY).unwrap();
        assert_eq!((e.artist.as_str(), e.album.as_str(), e.year.as_str()), ("The White Stripes", "The White Stripes", "1999"));
        assert_eq!(e.tracks[3].1, "Suzy Lee and a title long enough that it goes on", "continued lines join");
        let m = Match { category: "rock".into(), id: "da0a4311".into(), title: String::new(), exact: true };
        let c = candidate(&m, &e);
        assert_eq!(c.title, "The White Stripes — The White Stripes");
        assert_eq!(c.detail, "GnuDB, 1999, Garage Rock");
        assert!(c.facts.iter().any(|f| f.subject == Subject::Part(1) && f.key == "title" && f.value == "Jimmy the Exploder"));
        assert!(parse_read("401 Specified CDDB entry not found.\n").is_err());
    }

    #[test]
    fn a_compilation_has_an_artist_per_track_and_latin1_survives() {
        let e = parse_read("210 misc 1 x\nDTITLE=Various / Hits\nTTITLE0=Blur / Song 2\n.\n").unwrap();
        assert_eq!(e.tracks[0], ("Blur".into(), "Song 2".into()));
        assert_eq!(text(b"DTITLE=Bj\xf6rk / Post"), "DTITLE=Björk / Post");
    }
}
