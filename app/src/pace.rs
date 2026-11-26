//! How fast Rip is reading, and how long is left: for the track it's on
//! and for the whole disc. Worked out here from what Rip says (each
//! track's progress, its own estimate for the disc) and the tracks'
//! lengths, at the speed it has read at so far; kept steady, so the
//! numbers count down rather than jump.

use std::time::Instant;
use trip_engine::{ItemState, ItemView};
use trip_steps::real::redumper::Live;

/// What's shown while Rip reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    /// What it reads one at a time: *Track* (the default), *Video*.
    pub unit: &'static str,
    /// The track being read.
    pub track: u32,
    /// Its time left: *0:56*, *3 min*.
    pub track_left: Option<String>,
    /// The disc's: *12 min*.
    pub disc_left: Option<String>,
    /// The read speed: *8.2×*, *stalled*.
    pub speed: Option<String>,
    /// There's a speed, but no telling the time left from it.
    pub left_unknown: bool,
    /// What's slowing it: *slowed by read errors*.
    pub why: Option<&'static str>,
}

impl Reading {
    /// The times and the speed, without the track: *0:56 left · disc 12
    /// min left · 8.2×*; *time left unknown · 1.2× · slowed by read
    /// errors*.
    pub fn times(&self) -> String {
        let mut parts = vec![];
        if let Some(t) = &self.track_left {
            parts.push(format!("{t} left"));
        } else if self.left_unknown {
            parts.push("time left unknown".into());
        }
        if let Some(d) = &self.disc_left {
            parts.push(format!("disc {d} left"));
        }
        if let Some(s) = &self.speed {
            parts.push(s.clone());
        }
        if let Some(w) = self.why {
            parts.push(w.into());
        }
        parts.join(" · ")
    }

    /// The card's line: *Track 6 · 0:56 left · disc 12 min left · 8.2×*.
    pub fn line(&self) -> String {
        let times = self.times();
        let unit = if self.unit.is_empty() { "Track" } else { self.unit };
        if times.is_empty() {
            format!("{unit} {}", self.track)
        } else {
            format!("{unit} {} · {times}", self.track)
        }
    }
}

/// A time left, for reading at a glance: to the minute (rounded up) from
/// 2 minutes, else minutes and seconds to the 5 seconds.
pub fn left_text(secs: f32) -> String {
    let secs = secs.max(0.0);
    if secs >= 120.0 {
        format!("{} min", (secs / 60.0).ceil() as u32)
    } else {
        let s = ((secs / 5.0).ceil() * 5.0) as u32;
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A speed: *8.2×*.
pub fn speed_text(x: f32) -> String {
    format!("{x:.1}×")
}

/// A drive's speed as it reads, however slow: *0.4×*, *below 0.1×*,
/// *stalled*; *stalled on read errors* when they're why.
pub fn drive_speed_text(x: f32, errors_rising: bool) -> String {
    match x {
        0.0 if errors_rising => "stalled on read errors".into(),
        0.0 => "stalled".into(),
        x if x < 0.05 => "below 0.1×".into(),
        x => speed_text(x),
    }
}

/// A read hasn't moved for this long, by the clock here: stalled, though
/// the drive has said nothing since (it says nothing while stuck on a
/// sector).
pub const STALLED_SECS: f32 = 20.0;

/// Rip's own estimate for the disc, from the end of its detail: *Track 6
/// · Fell in Love · about 4 min left* is 240 s; *40 s left*, 40.
pub fn rips_estimate(detail: &str) -> Option<f32> {
    let tail = detail.rsplit(" · ").next()?.strip_suffix(" left")?;
    let tail = tail.trim().trim_start_matches("about ").trim();
    let (n, unit) = tail.split_once(' ')?;
    let n: f32 = n.parse().ok()?;
    match unit {
        "min" => Some(n * 60.0),
        "s" => Some(n),
        _ => None,
    }
}

/// How far along a note says an item is: *reading 43%* is 0.43; *43%,
/// sector 143300 of 332999* is to the sector.
pub fn percent(note: &str) -> Option<f32> {
    if let Some((at, of)) = sectors(note).filter(|s| s.1 > 0) {
        return Some((at as f32 / of as f32).clamp(0.0, 1.0));
    }
    note.split(|c: char| !(c.is_ascii_digit() || c == '%' || c == '.')).find_map(|w| w.strip_suffix('%')?.parse::<f32>().ok()).map(|n| (n / 100.0).clamp(0.0, 1.0))
}

/// The sectors a note says it's at: *43%, sector 143300 of 332999*.
pub fn sectors(note: &str) -> Option<(u64, u64)> {
    let rest = &note[note.find("sector ")? + 7..];
    let (at, rest) = rest.split_once(" of ")?;
    let of: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    Some((at.trim().parse().ok()?, of.parse().ok()?))
}

/// How many bytes a second a drive reads at 1× for a kind of disc
/// (the disc's *media*): a DVD's 1.385 MB, a Blu-ray's 4.5 MB. A CD's is
/// 75 sectors a second, whatever their size: see `seconds_at_1x`.
pub fn one_x(media: &str) -> Option<f32> {
    match media {
        "DVD" | "HD DVD" => Some(1_385_000.0),
        "Blu-ray" => Some(4_495_500.0),
        _ => None,
    }
}

/// How long a disc of `sectors` takes to read at 1×: what a speed is a
/// multiple of. A CD reads 75 sectors a second; a DVD's or Blu-ray's
/// sectors are 2048 bytes.
pub fn seconds_at_1x(media: &str, sectors: u64) -> Option<f32> {
    match media {
        "CD" => Some(sectors as f32 / 75.0),
        m => one_x(m).map(|b| sectors as f32 * 2048.0 / b),
    }
}

/// The track number of an item: `track-9` is 9.
fn number(it: &ItemView, k: usize) -> u32 {
    it.id.rsplit('-').next().and_then(|n| n.parse().ok()).unwrap_or(k as u32 + 1)
}

/// A number shown that only moves when it has really changed: a time
/// that counts down by itself between estimates and is reset only when
/// an estimate is well off it.
#[derive(Clone, Debug, Default)]
struct Steady {
    at: Option<(f32, Instant)>,
}

impl Steady {
    fn countdown(&mut self, estimate: f32, now: Instant) -> f32 {
        if let Some((v, t)) = self.at {
            let predicted = v - now.duration_since(t).as_secs_f32();
            if (estimate - predicted).abs() <= (0.15 * predicted).max(8.0) {
                return predicted.max(0.0);
            }
        }
        self.at = Some((estimate, now));
        estimate
    }

    /// A level that moves only by more than `by`.
    fn level(&mut self, estimate: f32, by: f32, now: Instant) -> f32 {
        match self.at {
            Some((v, _)) if (estimate - v).abs() <= by => v,
            _ => {
                self.at = Some((estimate, now));
                estimate
            }
        }
    }
}

/// One Rip's pace, as it reads.
#[derive(Clone, Debug, Default)]
pub struct Pace {
    /// The item being read, when it started (as seen here), and how far
    /// along it was then.
    on: Option<(String, Instant, f32)>,
    /// How far along it is, when last seen to move.
    last: Option<(Instant, f32)>,
    /// Each track read through: audio seconds per second.
    finished: Vec<f32>,
    speed: Steady,
    track: Steady,
    disc: Steady,
    /// A drive's read: the sector it's at, and when it came to it.
    sector: Option<(u64, Instant)>,
}

impl Pace {
    /// What to show now, or nothing when no track is being read. `length`
    /// gives a track's length in seconds; `detail` is Rip's own.
    pub fn read(&mut self, items: &[ItemView], detail: &str, length: impl Fn(u32) -> Option<f32>, now: Instant) -> Option<Reading> {
        let (k, it) = items.iter().enumerate().find(|(_, i)| i.state == ItemState::Busy)?;
        let n = number(it, k);
        let len = length(n);
        let pct = percent(&it.note);
        // A new track: the last one's speed, from when it was first seen
        // here to now, if it was watched for long enough to tell.
        // The same item from the start again (a disc's image, its next
        // pass): begun afresh, nothing carried over.
        // The speed read so far is kept, as between tracks.
        if self.last.is_some_and(|(_, q)| pct.is_some_and(|p| p + 0.05 < q)) {
            self.on = None;
            self.last = None;
            self.track = Steady::default();
        }
        if self.on.as_ref().is_none_or(|(id, _, _)| *id != it.id) {
            if let Some((id, t0, p0)) = self.on.take() {
                let prev = items.iter().enumerate().find(|(_, i)| i.id == id);
                let took = now.duration_since(t0).as_secs_f32();
                if let Some(l) = prev.filter(|(_, i)| i.state == ItemState::Ok).and_then(|(k, i)| length(number(i, k))) {
                    if took > 0.5 && p0 < 0.9 {
                        self.finished.push(l * (1.0 - p0) / took);
                    }
                }
            }
            self.on = Some((it.id.clone(), now, pct.unwrap_or(0.0)));
            self.last = None;
            self.track = Steady::default();
        }
        if let Some(p) = pct {
            if self.last.is_none_or(|(_, q)| q != p) {
                self.last = Some((now, p));
            }
        }
        // The speed: this track's so far, once it has a few seconds to go
        // on; else the tracks read before it.
        let live = match (&self.on, self.last, len) {
            (Some((_, t0, p0)), Some((t1, p1)), Some(l)) => {
                let dt = t1.duration_since(*t0).as_secs_f32();
                // Far enough to tell: two percent, or (counted in sectors)
                // half a minute of the disc at 1×.
                (dt >= 3.0 && (p1 - p0 > 0.02 || l * (p1 - p0) > 30.0)).then(|| l * (p1 - p0) / dt)
            }
            _ => None,
        };
        let recent = self.finished.iter().rev().take(2).copied().collect::<Vec<_>>();
        let before = (!recent.is_empty()).then(|| recent.iter().sum::<f32>() / recent.len() as f32);
        let speed = live.or(before).filter(|s| s.is_finite() && *s > 0.0).map(|s| self.speed.level(s, 0.15, now));
        // The track: what's left of it at that speed; only when Rip says
        // how far along it is.
        let track_left = match (pct, len, speed) {
            (Some(p), Some(l), Some(s)) => Some(self.track.countdown(l * (1.0 - p) / s, now)),
            _ => None,
        };
        // The disc: Rip's own estimate, else what's left of this track and
        // the ones still to read, at that speed.
        let to_read: Option<f32> = items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.state == ItemState::Waiting)
            .map(|(k, i)| length(number(i, k)))
            .sum::<Option<f32>>()
            .and_then(|rest| Some(rest + len? * (1.0 - pct.unwrap_or(0.0))));
        let disc = rips_estimate(detail).or(match (to_read, speed) {
            (Some(a), Some(s)) => Some(a / s),
            _ => None,
        });
        // One item is the whole disc: its time left is the disc's.
        let disc = disc.filter(|_| items.len() > 1);
        let disc_left = disc.map(|d| self.disc.countdown(d, now));
        Some(Reading { unit: "", track: n, track_left: track_left.map(left_text), disc_left: disc_left.map(left_text), speed: speed.map(speed_text), ..Default::default() })
    }
}

impl Pace {
    /// What to show while redumper reads a disc through: the speed it's
    /// reading at now, from its last half minute ([`Live::speed`]), and
    /// the time left while that says anything; and whether read errors
    /// are slowing it. Stalled once it hasn't moved for [`STALLED_SECS`].
    pub fn drive(&mut self, l: &Live, now: Instant) -> Reading {
        let since = match self.sector {
            Some((at, t)) if at == l.at => t,
            _ => {
                self.sector = Some((l.at, now));
                now
            }
        };
        let stalled = now.duration_since(since).as_secs_f32() >= STALLED_SECS || l.speed == Some(0.0);
        let speed = match l.speed.filter(|s| s.is_finite() && *s >= 0.0) {
            _ if stalled => {
                self.speed = Steady::default();
                Some(0.0)
            }
            // Held within a tenth, so it doesn't flicker; a slow read's
            // own small steps still show.
            Some(s) => Some(self.speed.level(s, (0.1 * s).max(0.02), now)),
            None => None,
        };
        let left = l.left_secs.filter(|_| !stalled).map(|s| self.track.countdown(s as f32, now));
        if left.is_none() {
            self.track = Steady::default();
        }
        Reading {
            unit: "",
            track: 1,
            track_left: left.map(left_text),
            disc_left: None,
            left_unknown: speed.is_some() && left.is_none(),
            why: (l.errors_rising && speed.is_some_and(|s| s > 0.0)).then_some("slowed by read errors"),
            speed: speed.map(|s| drive_speed_text(s, l.errors_rising)),
        }
    }
}

/// The average speed over a whole read: the audio read (every track not
/// left out) over the time it took.
pub fn average(items: &[ItemView], length: impl Fn(u32) -> Option<f32>, took: u64) -> Option<f32> {
    let audio: f32 = items.iter().enumerate().filter(|(_, i)| i.state != ItemState::Skipped).map(|(k, i)| length(number(i, k))).sum::<Option<f32>>()?;
    (took > 0 && audio > 0.0).then(|| audio / took as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn item(n: u32, state: ItemState, note: &str) -> ItemView {
        ItemView { id: format!("track-{n}"), label: format!("Song {n}"), state, note: note.into(), verified: false, options: vec![] }
    }

    #[test]
    fn times_read_at_a_glance() {
        assert_eq!(left_text(56.0), "1:00");
        assert_eq!(left_text(52.0), "0:55");
        assert_eq!(left_text(119.0), "2:00");
        assert_eq!(left_text(121.0), "3 min");
        assert_eq!(left_text(720.0), "12 min");
        assert_eq!(speed_text(8.16), "8.2×");
        assert_eq!(rips_estimate("Track 6 · Fell in Love · about 4 min left"), Some(240.0));
        assert_eq!(rips_estimate("Track 1 · Song · 40 s left"), Some(40.0));
        assert_eq!(rips_estimate("Track 1 · Song"), None);
    }

    #[test]
    fn the_track_and_the_disc_from_the_speed_read_so_far() {
        let t0 = Instant::now();
        let len = |_: u32| Some(160.0);
        let mut pace = Pace::default();
        let mut items = vec![item(1, ItemState::Ok, "✓ 104"), item(2, ItemState::Busy, "reading 0%"), item(3, ItemState::Waiting, ""), item(4, ItemState::Waiting, "")];
        let r = pace.read(&items, "", len, t0).unwrap();
        assert_eq!((r.track, r.track_left.clone(), r.speed.clone()), (2, None, None), "nothing to go on yet");
        // 10 s on, 50% through 160 s of audio: 8× read speed.
        items[1].note = "reading 50%".into();
        let r = pace.read(&items, "", len, t0 + Duration::from_secs(10)).unwrap();
        assert_eq!(r.speed.as_deref(), Some("8.0×"));
        assert_eq!(r.track_left.as_deref(), Some("0:10"));
        // 80 s of this one and 320 s of the others at 8×: 50 s.
        assert_eq!(r.disc_left.as_deref(), Some("0:50"));
        assert_eq!(r.line(), "Track 2 · 0:10 left · disc 0:50 left · 8.0×");
        // Rip's own estimate for the disc, when it gives one.
        let r = pace.read(&items, "Track 2 · Song 2 · about 12 min left", len, t0 + Duration::from_secs(10)).unwrap();
        assert_eq!(r.disc_left.as_deref(), Some("12 min"));
    }

    #[test]
    fn a_disc_image_by_its_sectors_and_each_pass_afresh() {
        let t0 = Instant::now();
        // 332999 sectors of a CD: 4440 s at 1×.
        let len = |_: u32| seconds_at_1x("CD", 332_999);
        let dump = |note: &str| vec![ItemView { id: "dump".into(), label: "ENCARTA98".into(), state: ItemState::Busy, note: note.into(), verified: false, options: vec![] }];
        let mut pace = Pace::default();
        pace.read(&dump("42%, sector 141500 of 332999"), "Reading", len, t0).unwrap();
        // 4 s on, 2400 sectors more: 600 a second, 8×.
        let r = pace.read(&dump("43%, sector 143900 of 332999, 3 errors"), "Reading", len, t0 + Duration::from_secs(4)).unwrap();
        assert_eq!(r.speed.as_deref(), Some("8.0×"));
        assert_eq!(r.track_left.as_deref(), Some("6 min"), "189099 sectors at 600 a second");
        assert_eq!(r.disc_left, None, "one item: its time is the disc's");
        assert_eq!(r.times(), "6 min left · 8.0×");
        // The next pass starts from the beginning: nothing to go on.
        let r = pace.read(&dump("2%, sector 6000 of 332999"), "Re-reading bad parts", len, t0 + Duration::from_secs(5)).unwrap();
        assert_eq!((r.speed, r.track_left), (None, None));
        // A DVD's 2048-byte sectors: 2298496 of them are 3399 s at 1×.
        assert_eq!(seconds_at_1x("DVD", 2_298_496).map(|s| s.round()), Some(3399.0));
        assert_eq!(percent("7%, sector 160894 of 2298496"), Some(160_894.0 / 2_298_496.0));
    }

    #[test]
    fn a_drives_read_says_its_speed_through_errors_and_stalls() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let live = |at: u64, speed: Option<f32>, errors_rising: bool, left_secs: Option<u64>| Live { at, of: 269_159, speed, errors_rising, left_secs, ..Default::default() };
        let mut pace = Pace::default();
        // The real read's lines (see redumper's tests): its first few
        // seconds say nothing; into errors, half of 1×.
        assert_eq!(pace.drive(&live(147_262, None, false, None), at(0)).times(), "");
        assert_eq!(pace.drive(&live(153_834, Some(0.49), true, None), at(180)).times(), "time left unknown · 0.5× · slowed by read errors");
        // Past them, quickly, a time left.
        assert_eq!(pace.drive(&live(197_515, Some(4.853), false, Some(196)), at(300)).times(), "4 min left · 4.9×");
        // Into them again, hardly moving.
        assert_eq!(pace.drive(&live(197_913, Some(0.04), true, None), at(420)).times(), "time left unknown · below 0.1× · slowed by read errors");
        // Nothing from the drive for 20 s: stalled, by the clock here.
        assert_eq!(pace.drive(&live(197_913, Some(0.04), true, None), at(439)).speed.as_deref(), Some("below 0.1×"));
        let r = pace.drive(&live(197_913, Some(0.04), true, None), at(440));
        assert_eq!(r.times(), "time left unknown · stalled on read errors");
        assert_eq!(r.why, None);
        // Or by the drive's, no errors to blame.
        assert_eq!(pace.drive(&live(197_914, Some(0.0), false, None), at(441)).times(), "time left unknown · stalled");
        // Moving again: the speed straight away.
        assert_eq!(pace.drive(&live(198_500, Some(1.2), false, Some(3000)), at(450)).times(), "50 min left · 1.2×");
        // A slow read's small steps show; a wobble doesn't.
        assert_eq!(pace.drive(&live(198_510, Some(1.25), false, Some(3000)), at(451)).speed.as_deref(), Some("1.2×"));
        assert_eq!(pace.drive(&live(198_520, Some(0.6), false, Some(3000)), at(452)).speed.as_deref(), Some("0.6×"));
    }

    #[test]
    fn a_small_wobble_doesnt_move_what_is_shown() {
        let t0 = Instant::now();
        let mut s = Steady::default();
        assert_eq!(s.countdown(600.0, t0), 600.0);
        // 10 s later, an estimate of 597: still counting down from 600.
        assert_eq!(s.countdown(597.0, t0 + Duration::from_secs(10)), 590.0);
        // Well off: taken.
        assert_eq!(s.countdown(300.0, t0 + Duration::from_secs(11)), 300.0);
        let mut v = Steady::default();
        assert_eq!(v.level(8.2, 0.15, t0), 8.2);
        assert_eq!(v.level(8.3, 0.15, t0), 8.2);
        assert_eq!(v.level(8.5, 0.15, t0), 8.5);
    }
}
