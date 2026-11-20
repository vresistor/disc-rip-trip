//! TMDb: what a volume label says, searching films and series by it, a
//! series' seasons and episodes, and turning what's found into candidates
//! for Choose. Lifted from the old app's `tmdb.rs`.

use super::makemkv::Kind;
use super::matching::{self, Episode};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use drt_engine::{Candidate, Fact, Subject};
use serde::Deserialize;

const API: &str = "https://api.themoviedb.org/3";

/// What a volume label suggests: a query, and a season or disc number.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelGuess {
    pub query: String,
    pub season: Option<u32>,
    pub disc: Option<u32>,
}

/// `THE_SOPRANOS_S3_D1` → *the sopranos*, season 3, disc 1. Labels are
/// wildly inconsistent, so this is a search to correct, never a decision.
pub fn parse_label(label: &str) -> LabelGuess {
    // Words about the box, never about what's in it.
    const NOISE: &[&str] = &[
        "dvd",
        "video",
        "ntsc",
        "pal",
        "disc",
        "disk",
        "vol",
        "volume",
        "region",
        "widescreen",
        "fullscreen",
        "bonus",
        "extras",
        "series",
        "season",
        "box",
        "boxset",
        "set",
        "collection",
        "complete",
        "edition",
        "special",
        "anniversary",
        "remastered",
        "definitive",
        "bd",
        "bluray",
        "uhd",
    ];
    let normalised = label.replace(['_', '.', '-'], " ").to_lowercase();
    let tokens: Vec<&str> = normalised.split_whitespace().collect();
    let (mut season, mut disc, mut words) = (None, None, Vec::new());
    let mut skip = false;
    for (i, tok) in tokens.iter().enumerate() {
        if std::mem::take(&mut skip) {
            continue;
        }
        let number = |prefixes: &[&str]| prefixes.iter().find_map(|p| tok.strip_prefix(p).and_then(|r| r.parse::<u32>().ok()));
        let next = tokens.get(i + 1).and_then(|t| t.parse::<u32>().ok());
        if let Some(n) = number(&["season", "s"]) {
            season = season.or(Some(n));
        } else if let Some(n) = number(&["disc", "disk", "d"]) {
            disc = disc.or(Some(n));
        } else if matches!(*tok, "season" | "series") && next.is_some() {
            season = season.or(next);
            skip = true;
        } else if matches!(*tok, "disc" | "disk") && next.is_some() {
            disc = disc.or(next);
            skip = true;
        } else if !NOISE.contains(tok) {
            // Numbers that belong to the title stay: se7en, district 9, 2012.
            words.push(*tok);
        }
    }
    LabelGuess { query: words.join(" "), season, disc }
}

/// A film or a series TMDb has.
#[derive(Clone, Debug, PartialEq)]
pub struct Show {
    pub id: u32,
    pub title: String,
    pub year: Option<u32>,
    pub overview: String,
    pub series: bool,
    /// A real catalogue entry rather than a thin duplicate someone added: it
    /// has a date or more than one vote. Searching *Twin Peaks* gives the
    /// 1990 series and, under it, "Twin Peaks 1990" with one vote and no date.
    pub verified: bool,
}

#[derive(Deserialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    id: u32,
    title: Option<String>,
    name: Option<String>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    overview: Option<String>,
    #[serde(default)]
    vote_count: u32,
}

/// The year of a date; TMDb sends `""` rather than nothing, so only a
/// parseable year counts.
fn year_of(date: &Option<String>) -> Option<u32> {
    date.as_ref().and_then(|d| d.get(0..4)).and_then(|y| y.parse().ok())
}

/// A search's results, the unverified sunk below the rest (kept, since an
/// obscure disc may really be one), TMDb's order kept within each.
pub fn parse_search(json: &str, series: bool) -> Result<Vec<Show>> {
    let body: SearchResponse = serde_json::from_str(json)?;
    let mut shows: Vec<Show> = body
        .results
        .into_iter()
        .map(|r| {
            let year = year_of(&r.release_date).or_else(|| year_of(&r.first_air_date));
            Show { id: r.id, title: r.title.or(r.name).unwrap_or_else(|| "Unknown".into()), year, overview: r.overview.unwrap_or_default(), series, verified: year.is_some() || r.vote_count > 1 }
        })
        .collect();
    shows.sort_by_key(|s| !s.verified);
    shows.truncate(8);
    Ok(shows)
}

#[derive(Deserialize)]
struct SeasonResponse {
    #[serde(default)]
    episodes: Vec<EpisodeResult>,
}

#[derive(Deserialize)]
struct EpisodeResult {
    episode_number: u32,
    season_number: u32,
    name: Option<String>,
    runtime: Option<u32>,
}

pub fn parse_season(json: &str) -> Result<Vec<Episode>> {
    let body: SeasonResponse = serde_json::from_str(json)?;
    Ok(body.episodes.into_iter().map(|e| Episode { season: e.season_number, number: e.episode_number, name: e.name.unwrap_or_default(), runtime_mins: e.runtime }).collect())
}

/// How many seasons a series has (specials, season 0, not counted).
pub fn parse_seasons(json: &str) -> Result<u32> {
    #[derive(Deserialize)]
    struct Detail {
        #[serde(default)]
        number_of_seasons: u32,
    }
    Ok(serde_json::from_str::<Detail>(json)?.number_of_seasons)
}

/// A film's runtime in minutes, from its details.
pub fn parse_runtime(json: &str) -> Result<Option<u32>> {
    #[derive(Deserialize)]
    struct Detail {
        runtime: Option<u32>,
    }
    Ok(serde_json::from_str::<Detail>(json)?.runtime.filter(|m| *m > 0))
}

/// Where TMDb's answers come from: the network, or fixtures.
#[async_trait]
pub trait Source: Send + Sync {
    /// `path` under the API (`search/movie`), with its query; the JSON.
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Option<String>>;
}

/// TMDb itself.
pub struct Http {
    pub key: String,
}

#[async_trait]
impl Source for Http {
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Option<String>> {
        let mut q = vec![("api_key", self.key.as_str())];
        q.extend_from_slice(query);
        // The URL carries the key, so it's never part of an error.
        let resp =
            super::super::net::client(std::time::Duration::from_secs(20))?.get(format!("{API}/{path}")).query(&q).send().await.map_err(|e| anyhow!("TMDb didn't answer: {}", e.without_url()))?;
        match resp.status() {
            reqwest::StatusCode::UNAUTHORIZED => Err(anyhow!("TMDb turned down the API key")),
            // A series without that season: not an error.
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            s if !s.is_success() => Err(anyhow!("TMDb said {s}")),
            _ => Ok(Some(super::super::net::text(resp, super::super::net::JSON_MAX).await.map_err(|e| anyhow!("TMDb's answer was cut short: {e}"))?)),
        }
    }
}

/// Searches, dropping the last word until something comes back: labels put
/// the title first and the packaging after, and TMDb finds nothing at all
/// for *twin peaks gold*.
pub async fn search(src: &dyn Source, series: bool, query: &str) -> Result<Vec<Show>> {
    let kind = if series { "search/tv" } else { "search/movie" };
    let words: Vec<&str> = query.split_whitespace().collect();
    for take in (1..=words.len()).rev() {
        let q = words[..take].join(" ");
        if let Some(json) = src.get(kind, &[("query", &q)]).await? {
            let found = parse_search(&json, series)?;
            if !found.is_empty() {
                return Ok(found);
            }
        }
    }
    Ok(vec![])
}

pub async fn season(src: &dyn Source, id: u32, n: u32) -> Result<Vec<Episode>> {
    match src.get(&format!("tv/{id}/season/{n}"), &[]).await? {
        Some(json) => parse_season(&json),
        None => Ok(vec![]),
    }
}

/// Lower case, letters and digits only, without a leading *the*.
fn plain(s: &str) -> String {
    let s: String = s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.strip_prefix("the ").map(str::to_owned).unwrap_or(s)
}

/// How well a search result fits the disc, 0 to 1: the name against the
/// label, film or series against what the titles look like, TMDb's own
/// rank, and whether it's a real entry.
pub fn score(show: &Show, rank: usize, query: &str, kind: Kind) -> f32 {
    let (name, q) = (plain(&show.title), plain(query));
    let mut s = 0.5;
    if !q.is_empty() && name == q {
        s += 0.3;
    } else if !q.is_empty() && q.split(' ').all(|w| name.split(' ').any(|n| n == w)) {
        s += 0.15;
    }
    s += if show.series == (kind == Kind::Series) { 0.1 } else { -0.2 };
    s -= 0.1 * rank.min(4) as f32;
    if !show.verified {
        s -= 0.2;
    }
    s.clamp(0.0, 1.0)
}

fn fact(key: &str, value: impl Into<String>) -> Fact {
    Fact { step: String::new(), subject: Subject::Disc, key: key.into(), value: value.into(), source: "TMDb".into() }
}

fn facts(show: &Show) -> Vec<Fact> {
    let mut f = vec![fact("title", &show.title), fact("tmdbid", show.id.to_string()), fact("video_kind", if show.series { "series" } else { "film" })];
    if let Some(y) = show.year {
        f.push(fact("year", y.to_string()));
    }
    f
}

pub fn film(show: &Show, runtime: Option<u32>, score: f32) -> Candidate {
    let mut facts = facts(show);
    if let Some(m) = runtime {
        facts.push(fact("runtime", m.to_string()));
    }
    let year = show.year.map(|y| y.to_string()).unwrap_or_else(|| "no year".into());
    let mins = runtime.map(|m| format!(", {m} min")).unwrap_or_default();
    Candidate { step: String::new(), id: format!("tmdb-movie-{}", show.id), source: "TMDb".into(), title: show.title.clone(), detail: format!("film, {year}{mins}"), score, facts }
}

/// A season of a series, with its episodes as `ep.N` and `ep_runtime.N`.
pub fn season_of(show: &Show, n: u32, episodes: &[Episode], score: f32) -> Candidate {
    let mut facts = facts(show);
    facts.push(fact("season", n.to_string()));
    for e in episodes {
        facts.push(fact(&format!("ep.{}", e.number), &e.name));
        if let Some(m) = e.runtime_mins {
            facts.push(fact(&format!("ep_runtime.{}", e.number), m.to_string()));
        }
    }
    let year = show.year.map(|y| format!(", {y}")).unwrap_or_default();
    Candidate {
        step: String::new(),
        id: format!("tmdb-tv-{}-s{n}", show.id),
        source: "TMDb".into(),
        title: format!("{} — Season {n}", show.title),
        detail: format!("series{year}, {} episodes", episodes.len()),
        score,
        facts,
    }
}

/// The episodes a season candidate carries.
pub fn episodes_of(c: &Candidate) -> Vec<Episode> {
    let season = c.facts.iter().find(|f| f.key == "season").and_then(|f| f.value.parse().ok()).unwrap_or(1);
    let mut eps: Vec<Episode> = c
        .facts
        .iter()
        .filter_map(|f| {
            let n: u32 = f.key.strip_prefix("ep.")?.parse().ok()?;
            let runtime = c.facts.iter().find(|r| r.key == format!("ep_runtime.{n}")).and_then(|r| r.value.parse().ok());
            Some(Episode { season, number: n, name: f.value.clone(), runtime_mins: runtime })
        })
        .collect();
    eps.sort_by_key(|e| e.number);
    eps
}

/// Most seasons tried when the label doesn't say which.
const MAX_SEASONS: u32 = 12;

/// Everything TMDb offers for a disc: films and series by the label's
/// query, each series as the season the label names, or the one whose
/// runtimes clearly fit the titles, or, when none clearly does, each that
/// fits at all (equal, so Choose asks).
pub async fn candidates(src: &dyn Source, label: &LabelGuess, titles: &[matching::Disc], kind: Kind) -> Result<Vec<Candidate>> {
    if label.query.is_empty() {
        return Ok(vec![]);
    }
    let films = search(src, false, &label.query).await?;
    let series = search(src, true, &label.query).await?;
    let mut out = Vec::new();
    for (rank, f) in films.iter().take(4).enumerate() {
        let s = score(f, rank, &label.query, kind);
        // The best film's runtime helps tell the feature from a second cut.
        let runtime = if rank == 0 && kind == Kind::Film {
            match src.get(&format!("movie/{}", f.id), &[]).await? {
                Some(json) => parse_runtime(&json)?,
                None => None,
            }
        } else {
            None
        };
        out.push(film(f, runtime, s));
    }
    let best_series = series.iter().enumerate().map(|(r, s)| score(s, r, &label.query, kind)).fold(0.0, f32::max);
    for (rank, show) in series.iter().enumerate().take(3) {
        let s = score(show, rank, &label.query, kind);
        // Seasons cost a request each: only for the series that might be it.
        if s + 0.1 < best_series {
            continue;
        }
        if let Some(n) = label.season {
            out.push(season_of(show, n, &season(src, show.id, n).await?, s));
            continue;
        }
        let count = match src.get(&format!("tv/{}", show.id), &[]).await? {
            Some(json) => parse_seasons(&json)?.min(MAX_SEASONS),
            None => 0,
        };
        let mut seasons = Vec::new();
        for n in 1..=count {
            let eps = season(src, show.id, n).await?;
            seasons.push((n, matching::season_fit(titles, &eps), eps));
        }
        let fits: Vec<(u32, matching::Fit)> = seasons.iter().map(|(n, f, _)| (*n, *f)).collect();
        match matching::best_season(&fits) {
            Some(n) => {
                let eps = &seasons.iter().find(|(m, ..)| *m == n).unwrap().2;
                out.push(season_of(show, n, eps, s));
            }
            None => {
                let fitting: Vec<_> = seasons.iter().filter(|(_, f, _)| f.matched > 0).collect();
                let offered = if fitting.is_empty() { seasons.iter().take(1).collect() } else { fitting };
                for (n, _, eps) in offered {
                    out.push(season_of(show, *n, eps, s - 0.1));
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn a_label_gives_a_query_and_a_season_and_disc() {
        assert_eq!(parse_label("THE_SOPRANOS_S3_D1"), LabelGuess { query: "the sopranos".into(), season: Some(3), disc: Some(1) });
        assert_eq!(parse_label("BLACKADDER SEASON 2 DISC 1 PAL"), LabelGuess { query: "blackadder".into(), season: Some(2), disc: Some(1) });
        assert_eq!(parse_label("THE_MATRIX_WIDESCREEN_DVD"), LabelGuess { query: "the matrix".into(), season: None, disc: None });
        assert_eq!(parse_label("TWIN_PEAKS_GOLD_BOX_DISC_5").query, "twin peaks gold");
        // Numbers that belong to the title.
        assert_eq!(parse_label("SE7EN").query, "se7en");
        assert_eq!(parse_label("DISTRICT_9").query, "district 9");
        assert_eq!(parse_label("2012_DVD").query, "2012");
    }

    /// Searches as TMDb answers them, trimmed.
    pub const MOVIE_SEARCH: &str = r#"{"page":1,"results":[
        {"id":78,"title":"Blade Runner","release_date":"1982-06-25","overview":"In the smog-choked dystopian Los Angeles of 2019...","vote_count":14000},
        {"id":335984,"title":"Blade Runner 2049","release_date":"2017-10-04","overview":"","vote_count":13000},
        {"id":999001,"title":"Blade Runner","release_date":"","overview":"","vote_count":1}],"total_results":3}"#;

    pub const TV_SEARCH: &str = r#"{"page":1,"results":[
        {"id":1398,"name":"The Sopranos","first_air_date":"1999-01-10","overview":"","vote_count":3000},
        {"id":999002,"name":"The Sopranos 1999","first_air_date":"","vote_count":0}]}"#;

    pub const SEASON_3: &str = r#"{"_id":"x","air_date":"2001-03-04","name":"Season 3","season_number":3,"episodes":[
        {"episode_number":1,"season_number":3,"name":"Mr. Ruggerio's Neighborhood","runtime":50},
        {"episode_number":2,"season_number":3,"name":"Proshai, Livushka","runtime":58},
        {"episode_number":3,"season_number":3,"name":"Fortunate Son","runtime":56},
        {"episode_number":4,"season_number":3,"name":"Employee of the Month","runtime":null},
        {"episode_number":5,"season_number":3,"name":"Another Toothpick","runtime":55}]}"#;

    #[test]
    fn searches_and_seasons_parse_and_thin_entries_sink() {
        let films = parse_search(MOVIE_SEARCH, false).unwrap();
        assert_eq!(films.iter().map(|f| (f.id, f.year, f.verified)).collect::<Vec<_>>(), [(78, Some(1982), true), (335984, Some(2017), true), (999001, None, false)]);
        let tv = parse_search(TV_SEARCH, true).unwrap();
        assert_eq!((tv[0].title.as_str(), tv[0].series, tv[1].verified), ("The Sopranos", true, false));
        let eps = parse_season(SEASON_3).unwrap();
        assert_eq!(eps.len(), 5);
        assert_eq!((eps[1].number, eps[1].name.as_str(), eps[1].runtime_mins), (2, "Proshai, Livushka", Some(58)));
        assert_eq!(eps[3].runtime_mins, None);
        assert_eq!(parse_seasons(r#"{"id":1398,"number_of_seasons":6}"#).unwrap(), 6);
        assert_eq!(parse_runtime(r#"{"id":78,"runtime":117}"#).unwrap(), Some(117));
        assert_eq!(year_of(&Some(String::new())), None);
    }

    #[test]
    fn the_exact_title_of_the_right_kind_scores_clearly_best() {
        let films = parse_search(MOVIE_SEARCH, false).unwrap();
        let tv = parse_search(TV_SEARCH, true).unwrap();
        let br = score(&films[0], 0, "blade runner", Kind::Film);
        let sequel = score(&films[1], 1, "blade runner", Kind::Film);
        assert!(br - sequel >= 0.05, "{br} {sequel}");
        assert!(score(&tv[0], 0, "the sopranos", Kind::Series) > score(&tv[0], 0, "the sopranos", Kind::Film));
        let c = season_of(&tv[0], 3, &parse_season(SEASON_3).unwrap(), 0.9);
        assert_eq!(c.title, "The Sopranos — Season 3");
        let back = episodes_of(&c);
        assert_eq!(back.len(), 5);
        assert_eq!((back[3].name.as_str(), back[3].runtime_mins, back[3].season), ("Employee of the Month", None, 3));
    }
}
