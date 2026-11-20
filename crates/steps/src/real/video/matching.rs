//! Which title is which: the film's feature, or which episode, worked out
//! from runtimes and disc order. Lifted from the old app's `matching.rs`.
//!
//! Runtime is the strong signal (it's what tells a 94-minute pilot from a
//! 47-minute episode), and disc order the tie-breaker: discs are authored
//! in broadcast order, and a disc holds a run of consecutive episodes.

/// One title on the disc, as matching sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Disc {
    /// The part number (MakeMKV's id + 1).
    pub part: u32,
    pub secs: u32,
    /// Another title has the same content; never matched.
    pub duplicate: bool,
}

/// One episode of a season, from TMDb.
#[derive(Clone, Debug, PartialEq)]
pub struct Episode {
    pub season: u32,
    pub number: u32,
    pub name: String,
    /// Often missing; when there, the only thing that tells a feature-length
    /// pilot from a normal episode.
    pub runtime_mins: Option<u32>,
}

/// What a title is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Film,
    Episode(u32),
    /// Bonus material, filed under `extras`.
    Extra,
    /// Not ripped.
    Skip,
}

/// Shortest thing treated as content rather than bonus material.
const EXTRA_MAX: u32 = 15 * 60;

/// How far a disc runtime may sit from the published one: PAL discs run 4%
/// short, and published runtimes may or may not count adverts. 12% or four
/// minutes, whichever is more forgiving.
fn tolerance(published: u32) -> u32 {
    ((published as f32 * 0.12) as u32).max(4 * 60)
}

/// How well a season's episodes explain the titles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub matched: usize,
    /// Mean relative gap of the pairings; lower is tighter.
    pub error: f32,
}

/// Pairs titles with episodes by runtime, closest first so one bad guess
/// can't cascade. Gives the pairs (part, episode number) and the fit.
fn pair_up(titles: &[&Disc], episodes: &[Episode]) -> (Vec<(u32, u32)>, Fit) {
    let mut pairs: Vec<(u32, f32, u32)> = Vec::new();
    for t in titles.iter().filter(|t| t.secs > EXTRA_MAX) {
        for e in episodes {
            let Some(published) = e.runtime_mins.filter(|m| *m > 0).map(|m| m * 60) else { continue };
            let gap = t.secs.abs_diff(published);
            if gap <= tolerance(published) {
                pairs.push((t.part, gap as f32 / published as f32, e.number));
            }
        }
    }
    pairs.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (mut out, mut error) = (Vec::new(), 0.0);
    for (part, gap, ep) in pairs {
        if out.iter().any(|(p, e)| *p == part || *e == ep) {
            continue;
        }
        out.push((part, ep));
        error += gap;
    }
    let matched = out.len();
    (out, Fit { matched, error: if matched > 0 { error / matched as f32 } else { 1.0 } })
}

/// How well one season explains the disc, for telling which season a disc
/// labelled only `DISC_5` is.
pub fn season_fit(titles: &[Disc], episodes: &[Episode]) -> Fit {
    let titles: Vec<&Disc> = titles.iter().filter(|t| !t.duplicate).collect();
    pair_up(&titles, episodes).1
}

/// How much tighter a season must fit, absolutely, to win a tie on count.
/// Twin Peaks' Gold Box disc 2 fits season 1 at 0.0395 and season 2 at
/// 0.0312, and it's a season 1 disc: a longer season nearly always looks
/// closer by luck.
const DECISIVE: f32 = 0.03;

/// The season that clearly explains the disc best, or none. A season you
/// have to pick is a smaller problem than one confidently picked wrong.
pub fn best_season(fits: &[(u32, Fit)]) -> Option<u32> {
    let mut ranked: Vec<&(u32, Fit)> = fits.iter().filter(|(_, f)| f.matched > 0).collect();
    ranked.sort_by(|(_, a), (_, b)| b.matched.cmp(&a.matched).then_with(|| a.error.total_cmp(&b.error)));
    let (season, best) = ranked.first()?;
    match ranked.get(1) {
        None => Some(*season),
        Some((_, next)) if best.matched > next.matched || next.error - best.error >= DECISIVE => Some(*season),
        _ => None,
    }
}

/// The titles that are content: not duplicates, long enough, and not a
/// play-all (one title about as long as the rest put together).
fn content(titles: &[Disc], floor: u32) -> Vec<&Disc> {
    let mut v: Vec<&Disc> = titles.iter().filter(|t| !t.duplicate && t.secs > floor).collect();
    if v.len() > 2 {
        let total: u32 = v.iter().map(|t| t.secs).sum();
        v.retain(|t| (t.secs as f32) < (total - t.secs) as f32 * 0.75);
    }
    v.sort_by_key(|t| t.part);
    v
}

/// Which title is the film. Sure when the longest title clearly is the
/// feature: nothing else near its length (two cuts of a film are a
/// question), and its runtime fits TMDb's when that's known.
pub fn film(titles: &[Disc], runtime_mins: Option<u32>) -> (Vec<(u32, Role)>, bool) {
    let unique: Vec<&Disc> = titles.iter().filter(|t| !t.duplicate).collect();
    let Some(longest) = unique.iter().max_by_key(|t| (t.secs, std::cmp::Reverse(t.part))) else { return (vec![], false) };
    let rival = unique.iter().any(|t| t.part != longest.part && t.secs as f32 >= longest.secs as f32 * 0.75);
    let fits = runtime_mins.filter(|m| *m > 0).is_none_or(|m| longest.secs.abs_diff(m * 60) <= tolerance(m * 60));
    let roles = titles.iter().map(|t| (t.part, if t.part == longest.part { Role::Film } else { Role::Skip })).collect();
    (roles, !rival && fits)
}

/// Which title is which episode. Runtime settles which episodes these are;
/// disc order settles which is which, numbering the titles as one run from
/// the first episode the runtimes found. Sure only when every content title
/// was placed by its runtime and the run is in the season. Everything else
/// (duplicates, extras, a play-all) is skipped.
pub fn episodes(titles: &[Disc], eps: &[Episode]) -> (Vec<(u32, Role)>, bool) {
    let floor = eps.iter().filter_map(|e| e.runtime_mins).filter(|m| *m > 0).min().map_or(EXTRA_MAX, |m| ((m * 60) as f32 * 0.6) as u32).max(EXTRA_MAX);
    let content = content(titles, floor);
    let (pairs, _) = pair_up(&content, eps);
    // Where the run starts: the lowest episode the runtimes found, less how
    // far into the disc the first title they placed is; episode 1 when
    // they placed none.
    let first = content.iter().position(|t| pairs.iter().any(|(p, _)| *p == t.part));
    let low = pairs.iter().map(|(_, e)| *e).min();
    let start = match (first, low) {
        (Some(i), Some(e)) => (e as i64 - i as i64).max(1) as u32,
        _ => 1,
    };
    // Sure when every title's place in the run is an episode whose runtime
    // it fits.
    let fits = |t: &Disc, n: u32| eps.iter().find(|e| e.number == n).and_then(|e| e.runtime_mins).filter(|m| *m > 0).is_some_and(|m| t.secs.abs_diff(m * 60) <= tolerance(m * 60));
    let mut sure = !content.is_empty();
    let mut roles = Vec::new();
    for t in titles {
        let role = match content.iter().position(|c| c.part == t.part) {
            Some(i) => {
                let n = start + i as u32;
                sure &= fits(t, n);
                Role::Episode(n)
            }
            None => Role::Skip,
        };
        roles.push((t.part, role));
    }
    (roles, sure)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(part: u32, secs: u32) -> Disc {
        Disc { part, secs, duplicate: false }
    }

    fn ep(number: u32, mins: Option<u32>) -> Episode {
        Episode { season: 1, number, name: format!("Episode {number}"), runtime_mins: mins }
    }

    #[test]
    fn disc_two_of_a_season_starts_where_the_runtimes_say_and_goes_in_order() {
        // Twin Peaks season 1: the pilot at 94, the rest at 47, so runtime
        // alone shuffles them; disc order puts them back as one run.
        let eps: Vec<Episode> = std::iter::once(ep(1, Some(94))).chain((2..=8).map(|n| ep(n, Some(47)))).collect();
        let titles = [d(1, 2830), d(2, 2790), d(3, 2810), d(4, 2805), d(5, 240)];
        let (roles, sure) = episodes(&titles, &eps);
        assert_eq!(roles, [(1, Role::Episode(2)), (2, Role::Episode(3)), (3, Role::Episode(4)), (4, Role::Episode(5)), (5, Role::Skip)]);
        assert!(sure);
    }

    #[test]
    fn an_episode_without_a_runtime_is_placed_by_order_and_asked_about() {
        let eps = [ep(1, Some(50)), ep(2, Some(58)), ep(3, Some(56)), ep(4, None)];
        let mut titles = vec![d(1, 3012), d(2, 3460), d(3, 3351), d(4, 3238), d(5, 3460), d(6, 190)];
        titles[4].duplicate = true;
        let (roles, sure) = episodes(&titles, &eps);
        assert_eq!(roles.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(), [Role::Episode(1), Role::Episode(2), Role::Episode(3), Role::Episode(4), Role::Skip, Role::Skip]);
        assert!(!sure);
    }

    #[test]
    fn a_play_all_title_is_skipped() {
        let eps = [ep(1, Some(44)), ep(2, Some(44)), ep(3, Some(44))];
        let titles = [d(1, 2640), d(2, 2700), d(3, 2610), d(4, 7950)];
        let (roles, sure) = episodes(&titles, &eps);
        assert_eq!(roles[3], (4, Role::Skip));
        assert!(sure);
    }

    #[test]
    fn the_film_is_the_longest_and_two_cuts_are_a_question() {
        let mut titles = vec![d(1, 7050), d(2, 7050), d(3, 190), d(4, 840)];
        titles[1].duplicate = true;
        let (roles, sure) = film(&titles, None);
        assert_eq!(roles, [(1, Role::Film), (2, Role::Skip), (3, Role::Skip), (4, Role::Skip)]);
        assert!(sure);
        let cuts = [d(1, 7050), d(2, 6960)];
        assert!(!film(&cuts, None).1);
        assert!(!film(&[d(1, 7050)], Some(90)).1, "TMDb says 90 minutes; this is two hours");
    }

    #[test]
    fn a_season_is_only_picked_when_it_clearly_fits_best() {
        let s1 = Fit { matched: 4, error: 0.0395 };
        let s2 = Fit { matched: 4, error: 0.0312 };
        assert_eq!(best_season(&[(1, s1), (2, s2)]), None);
        assert_eq!(best_season(&[(1, Fit { matched: 4, error: 0.01 }), (2, Fit { matched: 3, error: 0.0 })]), Some(1));
        assert_eq!(best_season(&[(1, Fit { matched: 0, error: 1.0 }), (2, s2)]), Some(2));
        assert_eq!(best_season(&[]), None);
    }
}
