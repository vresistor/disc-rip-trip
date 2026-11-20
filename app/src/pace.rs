//! How fast Rip is reading, and how long is left: for the track it's on
//! and for the whole disc. Worked out here from what Rip says (each
//! track's progress, its own estimate for the disc) and the tracks'
//! lengths, at the speed it has read at so far; kept steady, so the
//! numbers count down rather than jump.

use drt_engine::{ItemState, ItemView};
use std::time::Instant;

/// What's shown while Rip reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    /// The track being read.
    pub track: u32,
    /// Its time left: *0:56*, *3 min*.
    pub track_left: Option<String>,
    /// The disc's: *12 min*.
    pub disc_left: Option<String>,
    /// The read speed: *8.2×*.
    pub speed: Option<String>,
}

impl Reading {
    /// The times and the speed, without the track: *0:56 left · disc 12
    /// min left · 8.2×*.
    pub fn times(&self) -> String {
        let mut parts = vec![];
        if let Some(t) = &self.track_left {
            parts.push(format!("{t} left"));
        }
        if let Some(d) = &self.disc_left {
            parts.push(format!("disc {d} left"));
        }
        if let Some(s) = &self.speed {
            parts.push(s.clone());
        }
        parts.join(" · ")
    }

    /// The card's line: *Track 6 · 0:56 left · disc 12 min left · 8.2×*.
    pub fn line(&self) -> String {
        let times = self.times();
        if times.is_empty() {
            format!("Track {}", self.track)
        } else {
            format!("Track {} · {times}", self.track)
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

/// How far along a note says an item is: *reading 43%* is 0.43.
pub fn percent(note: &str) -> Option<f32> {
    note.split(|c: char| !(c.is_ascii_digit() || c == '%' || c == '.')).find_map(|w| w.strip_suffix('%')?.parse::<f32>().ok()).map(|n| (n / 100.0).clamp(0.0, 1.0))
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
                (dt >= 3.0 && p1 - p0 > 0.02).then(|| l * (p1 - p0) / dt)
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
        let disc_left = disc.map(|d| self.disc.countdown(d, now));
        Some(Reading { track: n, track_left: track_left.map(left_text), disc_left: disc_left.map(left_text), speed: speed.map(speed_text) })
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
        ItemView { id: format!("track-{n}"), label: format!("Song {n}"), state, note: note.into(), verified: false }
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
