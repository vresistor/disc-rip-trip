//! Links to a looked-up entry's own page, built only from an id that names
//! exactly one entry: a MusicBrainz release's MBID, a Discogs release's
//! number, a TMDb film or season. Anything else gets no link.

use drt_engine::{Candidate, Context, Subject};

/// A MusicBrainz id: 36 characters, hex in 8-4-4-4-12.
pub fn is_mbid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5 && parts.iter().zip([8, 4, 4, 4, 12]).all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn number(s: &str) -> Option<&str> {
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())).then_some(s)
}

/// Where a candidate's entry is, and the site's name: *MusicBrainz*,
/// `https://musicbrainz.org/release/<mbid>`.
pub fn entry(c: &Candidate) -> Option<(&'static str, String)> {
    let fact = |k: &str| c.facts.iter().find(|f| f.subject == Subject::Disc && f.key == k).map(|f| f.value.as_str());
    match c.source.as_str() {
        "MusicBrainz" => {
            let id = Some(c.id.as_str()).filter(|i| is_mbid(i)).or(fact("musicbrainz_albumid").filter(|i| is_mbid(i)))?;
            Some(("MusicBrainz", format!("https://musicbrainz.org/release/{id}")))
        }
        "Discogs" => {
            let id = fact("discogs_releaseid").and_then(number).or(c.id.strip_prefix("discogs-").and_then(number))?;
            Some(("Discogs", format!("https://www.discogs.com/release/{id}")))
        }
        "TMDb" => {
            if let Some(id) = c.id.strip_prefix("tmdb-movie-").and_then(number) {
                return Some(("TMDb", format!("https://www.themoviedb.org/movie/{id}")));
            }
            let (id, season) = c.id.strip_prefix("tmdb-tv-")?.split_once("-s")?;
            Some(("TMDb", format!("https://www.themoviedb.org/tv/{}/season/{}", number(id)?, number(season)?)))
        }
        // GnuDB (a category and a disc ID) and Redump (a name) carry
        // nothing that names one page for certain.
        _ => None,
    }
}

/// The cover's page: the Cover Art Archive's images of the release it
/// was fetched for, as MusicBrainz shows them.
pub fn cover(c: &Context) -> Option<String> {
    let id = c.get(Subject::Disc, "musicbrainz_albumid").filter(|i| is_mbid(i))?;
    Some(format!("https://musicbrainz.org/release/{id}/cover-art"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: &str, source: &str) -> Candidate {
        Candidate { id: id.into(), source: source.into(), title: String::new(), detail: String::new(), score: 1.0, facts: vec![], step: String::new() }
    }

    #[test]
    fn only_ids_that_name_one_entry_get_a_link() {
        let mb = "f1d5ff3f-6bb6-4bb2-9ee8-7a6d7c9b1e3c";
        assert_eq!(entry(&cand(mb, "MusicBrainz")).unwrap().1, format!("https://musicbrainz.org/release/{mb}"));
        assert_eq!(entry(&cand("mb-1999", "MusicBrainz")), None, "not an MBID");
        assert_eq!(entry(&cand("discogs-249504", "Discogs")).unwrap().1, "https://www.discogs.com/release/249504");
        assert_eq!(entry(&cand("dg-1", "Discogs")), None);
        assert_eq!(entry(&cand("tmdb-movie-78", "TMDb")).unwrap().1, "https://www.themoviedb.org/movie/78");
        assert_eq!(entry(&cand("tmdb-tv-1399-s2", "TMDb")).unwrap().1, "https://www.themoviedb.org/tv/1399/season/2");
        assert_eq!(entry(&cand("gnudb-rock-940ac70c", "GnuDB")), None);
        assert_eq!(entry(&cand("redump:Ico (Europe)", "Redump")), None);
    }
}
