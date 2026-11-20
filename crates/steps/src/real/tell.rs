//! Notify: once everything before it is done, says how the disc went
//! ("17 tracks filed, all verified") on this computer, on your phone
//! through an ntfy topic, or both.
//!
//! ntfy (https://ntfy.sh, or a server of your own) is one HTTP POST to
//! `<server>/<topic>` with the message as the body; whoever subscribes to
//! the topic in the ntfy app gets it. A disc that needs you goes at high
//! priority with a warning tag.
//!
//! How it reaches the desktop, checked on this machine (Bluefin, GNOME):
//! inside the flatpak, `org.freedesktop.Notifications` isn't on the bus the
//! sandbox sees (the manifest doesn't ask for it), but the Notification
//! portal is, so the flatpak uses the portal. Outside it, the freedesktop
//! interface. `gdbus` makes the call in both (glib ships it in the GNOME
//! runtime and on the host); `notify-send` is the last resort.

use anyhow::{bail, Result};
use async_trait::async_trait;
use drt_engine::setting::ParamKind;
use drt_engine::{Context, Cx, Group, Inlet, Item, ItemKind, Outlet, ParamSpec, Step, StepInfo, StepState, Subject};
use std::collections::BTreeMap;
use std::time::Duration;

pub const WHEN: &[&str] = &["Always", "Only when something needs you", "Never"];
const APP: &str = "Disc Rip Trip";

pub fn info() -> StepInfo {
    StepInfo::new("tell", "Notify", Group::Tell, ItemKind::Any, ItemKind::Nothing)
        .from("when all done")
        .param(ParamSpec::new("when", "When", ParamKind::Choice(WHEN)).default(WHEN[0]).labels(&["After every disc", "Only if it needed you or something isn't right", "Never"]).on_card_as(
            "When",
            &[],
            |g| match g("when").and_then(|v| v.as_str().map(str::to_owned)).as_deref() {
                Some(w) if w == WHEN[1] => "when it needs you".into(),
                Some(w) if w == WHEN[2] => "never".into(),
                _ => "always".into(),
            },
        ))
        .param(ParamSpec::new("desktop", "On this computer", ParamKind::Toggle).default(true).on_card_as("Where", &["ntfy_topic"], |g| {
            let topic = g("ntfy_topic").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
            match (g("desktop").and_then(|v| v.as_bool()).unwrap_or(true), topic.is_empty()) {
                (true, true) => "desktop".into(),
                (true, false) => "desktop + ntfy".into(),
                (false, false) => "ntfy".into(),
                (false, true) => "nowhere".into(),
            }
        }))
        .param(
            ParamSpec::new("ntfy_topic", "Phone alerts (ntfy topic)", ParamKind::Text)
                .default("")
                .suggest(made_up_topic)
                .help("Anyone who knows or guesses this topic can read your alerts: a disc's name and how it went."),
        )
        .param(ParamSpec::new("ntfy_server", "ntfy server", ParamKind::Text).default("https://ntfy.sh").section("Advanced"))
        .summary(|g| {
            let topic = g("ntfy_topic").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
            let desktop = g("desktop").and_then(|v| v.as_bool()).unwrap_or(true);
            match (desktop, topic.is_empty()) {
                (true, true) => "when all done".into(),
                (true, false) => "here and on ntfy".into(),
                (false, false) => "on ntfy".into(),
                (false, true) => "nowhere".into(),
            }
        })
}

const ADJECTIVES: [&str; 64] = [
    "amber", "azure", "bold", "brave", "breezy", "bright", "brisk", "bubbly", "calm", "cheery", "clever", "cool", "crisp", "curly", "dapper", "dizzy", "eager", "early", "fancy", "fluffy", "fond",
    "frosty", "fuzzy", "gentle", "giddy", "glad", "golden", "grand", "happy", "hazy", "humble", "jolly", "jumpy", "keen", "kind", "lively", "lucky", "mellow", "merry", "mighty", "misty", "nimble",
    "noble", "perky", "plucky", "polite", "proud", "quick", "quiet", "rosy", "rusty", "shiny", "silver", "sleepy", "snowy", "sunny", "swift", "tidy", "tiny", "velvet", "warm", "witty", "zany",
    "zesty",
];
const NOUNS: [&str; 64] = [
    "otter", "badger", "beaver", "bison", "crane", "cricket", "dolphin", "eagle", "falcon", "ferret", "finch", "fox", "gecko", "heron", "koala", "lemur", "lynx", "magpie", "marten", "moose", "newt",
    "owl", "panda", "parrot", "pelican", "penguin", "puffin", "rabbit", "raven", "robin", "salmon", "seal", "sparrow", "squirrel", "swan", "tapir", "tiger", "toucan", "turtle", "walrus", "wombat",
    "wren", "yak", "zebra", "acorn", "anchor", "banjo", "biscuit", "cactus", "comet", "cookie", "kettle", "lantern", "maple", "meadow", "muffin", "nutmeg", "pebble", "pickle", "pretzel", "rocket",
    "teapot", "tulip", "waffle",
];

/// A topic that's easy to type and unlikely to be taken already:
/// `rip-amber-otter-7342`. Not a secret: two words from short lists and
/// four digits are some 37 million topics, few enough to guess at.
pub fn made_up_topic() -> String {
    // The standard library's per-process random keys, from the OS: no
    // crate for something that needn't be cryptographic.
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let r = h.finish();
    format!("rip-{}-{}-{}", ADJECTIVES[(r & 63) as usize], NOUNS[(r >> 6 & 63) as usize], 1000 + (r >> 12) % 9000)
}

pub const TEST_TITLE: &str = "Disc Rip Trip test";
pub const TEST_BODY: &str = "If you can read this, alerts work.";

/// Sends a test through each way that's on: this computer if `desktop`,
/// and ntfy if there's a topic. Gives where it went (*this computer*,
/// *ntfy*); nowhere if neither is on. An error says what failed, after
/// what was sent, and never has the topic in it.
pub async fn send_test(desktop: bool, server: &str, topic: &str) -> Result<Vec<&'static str>> {
    let (mut sent, mut failed) = (vec![], vec![]);
    if desktop {
        match send(TEST_TITLE, TEST_BODY).await {
            Ok(_) => sent.push("this computer"),
            Err(e) => failed.push(format!("this computer: {e}")),
        }
    }
    if !topic.trim().is_empty() {
        match send_ntfy(server, topic, TEST_TITLE, TEST_BODY, false).await {
            Ok(()) => sent.push("ntfy"),
            Err(e) => failed.push(format!("ntfy: {}", ntfy_trouble(server, &e))),
        }
    }
    match (failed.is_empty(), sent.is_empty()) {
        (true, _) => Ok(sent),
        (false, true) => bail!("{}", failed.join("; ")),
        (false, false) => bail!("Sent to {}, but {}", sent.join(" and "), failed.join("; ")),
    }
}

/// Why ntfy couldn't be sent to, in a few words: *couldn't reach ntfy.sh*.
/// The server's host only, never its whole address.
fn ntfy_trouble(server: &str, e: &anyhow::Error) -> String {
    let host = reqwest::Url::parse(server.trim()).ok().and_then(|u| u.host_str().map(str::to_owned));
    match (e.downcast_ref::<reqwest::Error>(), host) {
        (_, None) => "the server isn't a web address".into(),
        (Some(r), Some(h)) if r.is_connect() || r.is_timeout() => format!("couldn't reach {h}"),
        _ => format!("{e:#}"),
    }
}

/// The ntfy request: where to POST, and its headers.
pub fn ntfy_request(server: &str, topic: &str, title: &str, needs_you: bool) -> (String, Vec<(&'static str, String)>) {
    let url = format!("{}/{}", server.trim_end_matches('/'), topic.trim().trim_start_matches('/'));
    let mut headers = vec![("Title", title.to_owned()), ("Tags", if needs_you { "warning,cd".into() } else { "cd".into() })];
    if needs_you {
        headers.push(("Priority", "high".into()));
    }
    (url, headers)
}

/// Sends the message to an ntfy topic.
pub async fn send_ntfy(server: &str, topic: &str, title: &str, body: &str, needs_you: bool) -> Result<()> {
    let (url, headers) = ntfy_request(server, topic, title, needs_you);
    // The topic is in the URL, and anyone who knows it reads the alerts: it
    // goes to the server you named and nowhere else, and into no error.
    let mut req = super::net::yours(Duration::from_secs(15))?.post(&url).body(body.to_owned());
    for (k, v) in headers {
        // Header values must be ASCII; ntfy reads RFC 2047 for the rest.
        let v = if v.is_ascii() { v } else { format!("=?UTF-8?B?{}?=", base64(v.as_bytes())) };
        req = req.header(k, v);
    }
    let r = req.send().await.map_err(super::net::quiet)?;
    if !r.status().is_success() {
        bail!("ntfy said {}", r.status().as_u16());
    }
    Ok(())
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// How the disc went, from the items that reached Tell (each once) and
/// what they carry: Rip's verdict on each track, and how many it was asked
/// for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub title: String,
    pub tracks: usize,
    pub verified: usize,
    pub unverified: usize,
    /// Tracks Rip was asked for that never arrived: left out, or lost on
    /// the way (an encode that failed).
    pub left_out: usize,
    /// Files that aren't checked tracks: an image, a film's titles, a cue
    /// sheet.
    pub other_files: usize,
    /// Something on the way waited on you: a question, or a track or dump
    /// held for *Retry*. From the run, not the items.
    pub needed_you: bool,
    /// The title is the disc's volume label: typed by whoever burned it,
    /// so it may be personal. Said on this computer only, never to ntfy.
    pub from_label: bool,
}

impl Outcome {
    pub fn of(items: &[Item]) -> Outcome {
        let mut seen = BTreeMap::new();
        let mut c = Context::default();
        for i in items {
            seen.entry(i.id.clone()).or_insert(i.index);
            c.merge(i.context());
        }
        let mut o = Outcome::default();
        for index in seen.values() {
            // A track is one Rip checked: it has a verdict. A film's titles
            // have numbers too, but nothing checks them, so they're files.
            match index.and_then(|t| c.all(Subject::Part(t), "verdict").last()).map(|f| f.value.as_str()) {
                Some(v) => {
                    o.tracks += 1;
                    if v.contains('✓') {
                        o.verified += 1;
                    } else {
                        o.unverified += 1;
                    }
                }
                None => o.other_files += 1,
            }
        }
        // Asked for and never here: the tracks that arrived say how many
        // Rip was asked for.
        if o.tracks > 0 {
            let asked = c.get(Subject::Disc, "selected").and_then(|v| v.parse().ok()).unwrap_or(c.parts()) as usize;
            o.left_out = asked.saturating_sub(o.tracks);
        }
        let get = |k| c.get(Subject::Disc, k).unwrap_or("").to_owned();
        let (artist, album, label) = (get("album_artist"), get("album"), get("label"));
        o.title = match (artist.is_empty(), album.is_empty()) {
            (false, false) => format!("{artist} — {album}"),
            (true, false) => album,
            _ if !c.get(Subject::Disc, "name").unwrap_or("").is_empty() => get("name"),
            _ if !label.is_empty() => {
                o.from_label = true;
                label
            }
            _ => "A disc".into(),
        };
        o
    }

    /// The title as sent to ntfy, where anyone who has the topic reads
    /// it: a catalogued name, never a volume label.
    pub fn ntfy_title(&self) -> &str {
        if self.from_label {
            "A disc"
        } else {
            &self.title
        }
    }

    /// Something you'd want to know about: the run waited on you, or it
    /// ended with something unresolved. A Hold step waiting for you to let
    /// files through isn't counted: you put it there.
    pub fn needs_you(&self) -> bool {
        self.needed_you || self.unresolved()
    }

    /// Something still wrong at the end: a track that isn't verified, one
    /// left out, or nothing filed at all. What ntfy marks as urgent.
    pub fn unresolved(&self) -> bool {
        self.unverified > 0 || self.left_out > 0 || self.tracks + self.other_files == 0
    }

    /// "17 tracks filed, all verified".
    pub fn body(&self) -> String {
        if self.tracks + self.other_files == 0 {
            return "Nothing filed".into();
        }
        if self.tracks == 0 {
            return format!("{} file{} filed", self.other_files, if self.other_files == 1 { "" } else { "s" });
        }
        let mut s = format!("{} track{} filed", self.tracks, if self.tracks == 1 { "" } else { "s" });
        if self.unverified == 0 {
            s.push_str(if self.tracks == 1 { ", verified" } else { ", all verified" });
        } else {
            s.push_str(&format!(", {} verified, {} not", self.verified, self.unverified));
        }
        if self.left_out > 0 {
            s.push_str(&format!("; {} skipped", self.left_out));
        }
        s
    }
}

/// Whether to say anything, given the setting.
pub fn should_tell(when: &str, o: &Outcome) -> bool {
    match when {
        "Never" => false,
        "Only when something needs you" => o.needs_you(),
        _ => true,
    }
}

/// A string in GVariant's text form, which is how `gdbus call` reads its
/// arguments.
pub fn gvariant(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Ways to put a notification up, in the order to try them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Route {
    Portal,
    Freedesktop,
    NotifySend,
}

pub fn routes(in_flatpak: bool) -> Vec<Route> {
    if in_flatpak {
        vec![Route::Portal, Route::NotifySend]
    } else {
        vec![Route::Freedesktop, Route::NotifySend]
    }
}

/// The program and arguments for one route.
pub fn command(route: Route, id: &str, title: &str, body: &str) -> (&'static str, Vec<String>) {
    let s = |x: &str| x.to_owned();
    match route {
        Route::Portal => (
            "gdbus",
            vec![
                s("call"),
                s("--session"),
                s("--dest"),
                s("org.freedesktop.portal.Desktop"),
                s("--object-path"),
                s("/org/freedesktop/portal/desktop"),
                s("--method"),
                s("org.freedesktop.portal.Notification.AddNotification"),
                gvariant(id),
                format!("{{'title': <{}>, 'body': <{}>}}", gvariant(title), gvariant(body)),
            ],
        ),
        Route::Freedesktop => (
            "gdbus",
            vec![
                s("call"),
                s("--session"),
                s("--dest"),
                s("org.freedesktop.Notifications"),
                s("--object-path"),
                s("/org/freedesktop/Notifications"),
                s("--method"),
                s("org.freedesktop.Notifications.Notify"),
                gvariant(APP),
                s("0"),
                gvariant("media-optical"),
                gvariant(title),
                gvariant(body),
                s("[]"),
                s("{}"),
                s("-1"),
            ],
        ),
        Route::NotifySend => ("notify-send", vec![s("--app-name"), s(APP), s("--icon"), s("media-optical"), s(title), s(body)]),
    }
}

/// Puts the notification up by the first route that works. Gives which.
pub async fn send(title: &str, body: &str) -> Result<Route> {
    let id = format!("disc-{}", drt_engine::setting::now());
    let mut why = vec![];
    for route in routes(std::path::Path::new("/.flatpak-info").exists()) {
        let (program, args) = command(route, &id, title, body);
        let run = tokio::process::Command::new(program).args(&args).stdin(std::process::Stdio::null()).kill_on_drop(true).output();
        match tokio::time::timeout(Duration::from_secs(5), run).await {
            Ok(Ok(o)) if o.status.success() => return Ok(route),
            Ok(Ok(o)) => why.push(format!("{route:?}: {}", String::from_utf8_lossy(&o.stderr).trim())),
            Ok(Err(e)) => why.push(format!("{route:?}: {e}")),
            Err(_) => why.push(format!("{route:?}: no answer")),
        }
    }
    bail!("{}", why.join("; "))
}

pub struct Tell;

#[async_trait]
impl Step for Tell {
    fn info(&self) -> StepInfo {
        info()
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, cx: Cx) -> Result<()> {
        let all = inlet.all().await;
        if cx.stopped() {
            return Ok(());
        }
        let o = Outcome { needed_you: cx.needed_you(), ..Outcome::of(&all) };
        let body = o.body();
        cx.log(format!("{}: {body}", o.title));
        if !should_tell(&cx.text("when"), &o) {
            cx.status("Not sent", body);
            return Ok(());
        }
        let topic = cx.text("ntfy_topic");
        let mut failed = vec![];
        if cx.flag("desktop") {
            if let Err(e) = send(&o.title, &body).await {
                failed.push(format!("desktop: {e}"));
            }
        }
        if !topic.trim().is_empty() {
            let _net = cx.lane(drt_engine::LaneKind::Network("ntfy".into())).await;
            if let Err(e) = send_ntfy(&cx.text("ntfy_server"), &topic, o.ntfy_title(), &body, o.unresolved()).await {
                failed.push(format!("ntfy: {e}"));
            }
        }
        if failed.is_empty() {
            cx.status("Sent", body);
        } else {
            cx.state(StepState::Bad);
            cx.status("Couldn't send", failed.join("; "));
            cx.log(format!("notification: {}", failed.join("; ")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn the_topic_is_in_no_error() {
        // Nothing listens on port 1.
        let e = send_ntfy("http://127.0.0.1:1", "private-topic-name", "t", "b", false).await.unwrap_err();
        let said = format!("{e:#} {e:?}");
        assert!(!said.contains("private-topic"), "{said}");
        // Nor in a test's, which says plainly what failed.
        let e = send_test(false, "http://127.0.0.1:1", "private-topic-name").await.unwrap_err();
        let said = format!("{e:#} {e:?}");
        assert!(!said.contains("private-topic"), "{said}");
        assert_eq!(e.to_string(), "ntfy: couldn't reach 127.0.0.1");
        let e = send_test(false, "not a server", "private-topic-name").await.unwrap_err();
        assert_eq!(e.to_string(), "ntfy: the server isn't a web address");
        // Nothing on: sent nowhere, and no error.
        assert!(send_test(false, "https://ntfy.sh", " ").await.unwrap().is_empty());
    }

    #[test]
    fn a_made_up_topic_is_two_words_and_four_digits_and_new_each_time() {
        for list in [&ADJECTIVES, &NOUNS] {
            let set: std::collections::BTreeSet<_> = list.iter().collect();
            assert_eq!(set.len(), 64, "no word twice");
            assert!(list.iter().all(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase())), "{list:?}");
        }
        let topics: std::collections::BTreeSet<String> = (0..20).map(|_| made_up_topic()).collect();
        assert!(topics.len() >= 19, "{topics:?}");
        for t in &topics {
            let parts: Vec<&str> = t.split('-').collect();
            assert_eq!(parts.len(), 4, "{t}");
            assert_eq!(parts[0], "rip");
            assert!(ADJECTIVES.contains(&parts[1]) && NOUNS.contains(&parts[2]), "{t}");
            assert!(parts[3].len() == 4 && parts[3].chars().all(|c| c.is_ascii_digit()), "{t}");
        }
        assert!(info().params.iter().find(|p| p.key == "ntfy_topic").unwrap().suggest.is_some());
    }

    use super::*;

    #[test]
    fn ntfy_posts_to_the_topic_and_marks_a_disc_that_needs_you() {
        let (url, h) = ntfy_request("https://ntfy.sh/", "my-rips", "The White Stripes — The White Stripes", false);
        assert_eq!(url, "https://ntfy.sh/my-rips");
        assert!(h.contains(&("Tags", "cd".into())) && !h.iter().any(|(k, _)| *k == "Priority"));
        let (_, h) = ntfy_request("https://ntfy.example.org", "x", "t", true);
        assert!(h.contains(&("Priority", "high".into())) && h.contains(&("Tags", "warning,cd".into())));
        assert_eq!(base64(b"\xe2\x80\x94"), "4oCU", "an em dash, for a non-ASCII title");
    }

    fn disc(verdicts: &[&str]) -> Context {
        let mut c = Context::default();
        c.add(Subject::Disc, "parts", verdicts.len().to_string(), "t");
        c.add(Subject::Disc, "album", "The White Stripes", "t");
        c.add(Subject::Disc, "album_artist", "The White Stripes", "t");
        for (i, v) in verdicts.iter().enumerate() {
            c.add(Subject::Part(i as u32 + 1), "verdict", *v, "Rip");
        }
        c
    }

    /// The files that reached Tell, each knowing what its track does.
    fn files(c: &Context, tracks: &[u32], twice: bool) -> Vec<Item> {
        let mut v: Vec<Item> = tracks
            .iter()
            .map(|&t| {
                let mut i = Item::track(t, "x").becomes(ItemKind::AudioFile, None);
                i.context = std::sync::Arc::new(c.clone());
                i
            })
            .collect();
        if twice {
            v.extend(v.clone());
        }
        v.push(Item::new("cue", ItemKind::TextFile, "Cue sheet", None, None));
        v
    }

    #[test]
    fn what_it_says() {
        let c = disc(&["ok: ✓ 12", "bad: reads disagree", "ok: ✓ two reads agree"]);
        // Track 2 failed first, then was read again and verified.
        let mut c2 = c.clone();
        c2.add(Subject::Part(2), "verdict", "ok: ✓ two reads agree", "Rip");
        let o = Outcome::of(&files(&c2, &[1, 2, 3], true));
        assert_eq!((o.tracks, o.other_files), (3, 1), "each item once");
        assert_eq!(o.title, "The White Stripes — The White Stripes");
        assert_eq!(o.body(), "3 tracks filed, all verified");
        assert!(!o.needs_you());

        let c = disc(&["ok: ✓ 12", "ok: kept, unverified", "skipped: left out"]);
        let o = Outcome::of(&files(&c, &[1, 2], false));
        assert_eq!(o.body(), "2 tracks filed, 1 verified, 1 not; 1 skipped");
        assert!(o.needs_you());
        assert!(should_tell("Only when something needs you", &o) && !should_tell("Never", &o));
        assert!(!should_tell("Only when something needs you", &Outcome { tracks: 1, verified: 1, ..Default::default() }));
    }

    #[test]
    fn a_data_disc_has_files_not_tracks() {
        let mut c = Context::default();
        c.add(Subject::Disc, "name", "Encarta 98 Encyclopedia (USA) (Disc 1)", "Redump");
        let mut dump = Item::new("dump", ItemKind::ImageFile, "x", None, None);
        dump.context = std::sync::Arc::new(c);
        let o = Outcome::of(&[dump]);
        assert_eq!((o.title.as_str(), o.body().as_str()), ("Encarta 98 Encyclopedia (USA) (Disc 1)", "1 file filed"));
        assert_eq!(o.ntfy_title(), o.title, "a catalogued name goes to ntfy");
    }

    /// A disc no list knows is titled by its volume label, which whoever
    /// burned it typed: that stays on this computer.
    #[test]
    fn a_volume_label_never_goes_to_ntfy() {
        let mut c = Context::default();
        c.add(Subject::Disc, "label", "SMITH_TAXES_2019", "Data disc");
        let mut dump = Item::new("dump", ItemKind::ImageFile, "x", None, None);
        dump.context = std::sync::Arc::new(c);
        let o = Outcome::of(&[dump]);
        assert_eq!((o.title.as_str(), o.ntfy_title()), ("SMITH_TAXES_2019", "A disc"));
        assert!(!o.body().contains("SMITH"));
    }

    /// "Only when something needs you" is about the run, not the kind of
    /// disc: a film's titles carry no verdict, so they aren't unverified
    /// tracks, and a data disc can need you too.
    #[test]
    fn only_when_it_needs_you_means_it() {
        let when = "Only when something needs you";
        // A film: three titles filed, nothing asked.
        let titles: Vec<Item> = (1..=3).map(|t| Item::track(t, "Title").becomes(ItemKind::VideoFile, None)).collect();
        let o = Outcome::of(&titles);
        assert_eq!((o.tracks, o.other_files, o.body().as_str()), (0, 3, "3 files filed"));
        assert!(!should_tell(when, &o), "a film that went smoothly says nothing");
        // The same film, where Choose had to ask which one it was.
        let asked = Outcome { needed_you: true, ..o.clone() };
        assert!(should_tell(when, &asked));
        assert!(!asked.unresolved(), "nothing wrong at the end, so ntfy doesn't mark it urgent");

        // A data disc held as suspect, then kept: it waited on you.
        let dump = Item::new("dump", ItemKind::ImageFile, "x", None, None);
        let o = Outcome::of(std::slice::from_ref(&dump));
        assert!(!should_tell(when, &o));
        assert!(should_tell(when, &Outcome { needed_you: true, ..o }));
        // Nothing reached Notify at all: the CHD failed, say.
        let o = Outcome::of(&[]);
        assert_eq!(o.body(), "Nothing filed");
        assert!(should_tell(when, &o));
    }

    #[test]
    fn gvariant_strings_survive_quotes_and_lines() {
        assert_eq!(gvariant("It's done\nall 17"), r"'It\'s done\nall 17'");
        assert_eq!(gvariant(r"a\b"), r"'a\\b'");
    }

    #[test]
    fn the_flatpak_uses_the_portal() {
        assert_eq!(routes(true)[0], Route::Portal);
        assert_eq!(routes(false)[0], Route::Freedesktop);
        let (p, a) = command(Route::Portal, "disc-1", "Bob's album", "17 tracks filed");
        assert_eq!(p, "gdbus");
        assert_eq!(a.last().unwrap(), r"{'title': <'Bob\'s album'>, 'body': <'17 tracks filed'>}");
        let (_, a) = command(Route::Freedesktop, "x", "T", "B");
        assert_eq!(a[8..], ["'Disc Rip Trip'", "0", "'media-optical'", "'T'", "'B'", "[]", "{}", "-1"]);
    }
}
