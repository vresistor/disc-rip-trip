//! Films and TV, end to end on the simulated steps: the real parser,
//! matching, TMDb scoring and Jellyfin naming on canned answers.

use serde_json::Value;
use std::collections::BTreeMap;
use trip_engine::recipe::chain;
use trip_engine::snapshot::QuestionShape;
use trip_engine::{check, fits_flow, flow_after, Answer, Command, Engine, ItemKind, ItemState, Node, Question, Recipe, Setting, Snapshot};
use trip_steps::real::video::sim::{self, SimDisc};
use trip_steps::{recipes, simulated};

fn registry(disc: SimDisc) -> trip_engine::Registry {
    let mut r = simulated(None);
    sim::register(&mut r, disc);
    r
}

fn engine(disc: SimDisc) -> Engine {
    let mut e = Engine::new(registry(disc));
    e.time_scale = 0.0;
    e.work_root = std::env::temp_dir().join(format!("trip-video-{}", std::process::id()));
    e.settings.lock().unwrap().set_app("tmdb", "api_key", Setting::chosen(Value::from("test-key")));
    e
}

/// Runs to the end, answering each question with `answer`; gives the last
/// snapshot and the questions asked.
async fn run(e: &Engine, recipe: &Recipe, answer: impl Fn(&Question) -> Answer) -> (Snapshot, Vec<Question>) {
    let run = e.start(recipe).unwrap();
    let mut rx = run.snapshot.clone();
    let mut asked: Vec<Question> = Vec::new();
    loop {
        let s = rx.borrow_and_update().clone();
        if let Some((step, q)) = &s.question {
            if asked.last() != Some(q) {
                asked.push(q.clone());
                run.send(Command::Answer { step: step.clone(), answer: answer(q) }).await;
            }
        }
        if s.finished || rx.changed().await.is_err() {
            break;
        }
    }
    (run.finished().await, asked)
}

fn filed(s: &Snapshot) -> Vec<String> {
    let mut v: Vec<String> = s.steps["v_lib"].items.iter().filter(|i| i.state == ItemState::Ok).map(|i| i.note.clone()).collect();
    v.sort();
    v
}

#[test]
fn the_recipe_checks_and_its_steps_follow_from_one_another() {
    let reg = registry(SimDisc::Film);
    let r = recipes::video().root;
    assert_eq!(check(&r, &reg).unwrap(), ItemKind::Any);
    check(&r, &trip_steps::real()).expect("and with the real steps");
    let names = |id: &str| fits_flow(&flow_after(&r, id, &reg).unwrap(), &reg).iter().map(|i| (i.name, i.kind)).collect::<Vec<_>>();
    let has = |v: &[(&str, &str)], k: &str| v.iter().any(|(_, kind)| *kind == k);
    let after_drive = names("v_drive");
    assert!(has(&after_drive, "video-disc") && !has(&after_drive, "tmdb"), "{after_drive:?}");
    let after_disc = names("v_disc");
    assert!(has(&after_disc, "tmdb") && has(&after_disc, "rip-video"), "{after_disc:?}");
    assert!(!has(&after_disc, "choose-video") && !has(&after_disc, "choose") && !has(&after_disc, "musicbrainz") && !has(&after_disc, "rip"), "{after_disc:?}");
    assert!(has(&names("v_tmdb"), "choose-video"));
    let after_rip = names("v_rip");
    assert!(has(&after_rip, "transcode") && has(&after_rip, "remux") && has(&after_rip, "next-disc"), "{after_rip:?}");
    assert!(!has(&after_rip, "flac"), "a title is never offered FLAC");
    assert!(has(&names("v_make"), "folder"));
    // The audio Choose doesn't follow TMDb, nor the video one MusicBrainz.
    let bad = chain([Node::step("d", "drive"), Node::step("v", "video-disc"), Node::step("t", "tmdb"), Node::step("c", "choose")]);
    assert!(check(&bad, &reg).is_err());
    let bad = chain([Node::step("d", "drive"), Node::step("cd", "audio-cd"), Node::step("mb", "musicbrainz"), Node::step("c", "choose-video")]);
    assert!(check(&bad, &reg).is_err());
}

#[test]
fn the_api_key_is_asked_for_once_for_the_app() {
    let mut e = Engine::new(registry(SimDisc::Film));
    let missing = e.missing(&recipes::video());
    assert_eq!(missing.iter().map(|m| (m.step.as_str(), m.key)).collect::<Vec<_>>(), [("v_tmdb", "api_key")], "and not the drive's read offset");
    assert!(e.start(&recipes::video()).is_err());
    e.settings.lock().unwrap().set_app("tmdb", "api_key", Setting::chosen(Value::from("k")));
    assert!(e.missing(&recipes::video()).is_empty());
    e.time_scale = 0.0;
}

#[tokio::test]
async fn a_film_runs_to_the_end_without_a_question() {
    let e = engine(SimDisc::Film);
    let (s, asked) = run(&e, &recipes::video(), |q| panic!("asked {}", q.prompt)).await;
    assert!(asked.is_empty());
    assert!(s.finished && !s.stopped);
    // One of its three titles kept: which, and how long.
    assert_eq!(s.steps["v_disc"].status, "1 of 3 videos");
    assert!(s.steps["v_disc"].detail.starts_with("Video 1 · "), "{}", s.steps["v_disc"].detail);
    let disc = &s.steps["v_disc"].items;
    assert!(disc[1].note.contains("duplicate of video 1"), "{:?}", disc[1]);
    assert_eq!(s.steps["v_tmdb"].status, "3 films");
    assert!(s.steps["v_tmdb"].qualifier.contains("blade runner"), "the query: {}", s.steps["v_tmdb"].qualifier);
    assert_eq!(s.steps["v_rip"].status, "1 video");
    assert_eq!((s.steps["v_choose"].status.as_str(), s.steps["v_choose"].detail.as_str()), ("Blade Runner", "1982, 1 video"));
    // Only the feature is read: not its duplicate, not the trailer.
    assert_eq!(s.steps["v_rip"].items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["title-1"]);
    assert_eq!(s.steps["v_rip"].items[0].label, "Blade Runner");
    assert_eq!(filed(&s), ["~/Videos/Movies/Blade Runner (1982) [tmdbid-78]/Blade Runner (1982).mkv"]);
    assert_eq!(s.steps["v_next"].status, "Ejected");
}

/// The season disc: TMDb has no runtime for episode 4, so Choose asks.
fn season_question(q: &Question) -> (&[trip_engine::ItemChoice], &[trip_engine::Choice]) {
    match &q.shape {
        QuestionShape::PerItem { items, options } => (items, options),
        other => panic!("not a per-title question: {other:?}"),
    }
}

#[tokio::test]
async fn a_season_is_matched_title_by_title_with_the_best_guesses_picked() {
    let e = engine(SimDisc::Season);
    let (s, asked) = run(&e, &recipes::video(), |q| {
        let (items, options) = season_question(q);
        let pre: Vec<Option<&str>> = items.iter().map(|i| i.preselected.map(|o| options[o].label.as_str())).collect();
        assert_eq!(pre, [Some("S03E01"), Some("S03E02"), Some("S03E03"), Some("S03E04"), None, None], "duplicates and the trailer start skipped");
        assert!(items[4].detail.contains("duplicate of video 2"), "{}", items[4].detail);
        assert_eq!(options[1].detail, "Proshai, Livushka, 58 min");
        // Keep the trailer, as an extra; the rest as guessed.
        let extra = options.iter().position(|o| o.label == "Extra").unwrap();
        Answer::PerItem(BTreeMap::from([("title-6".to_owned(), Some(extra))]))
    })
    .await;
    assert_eq!(asked.len(), 1, "one question: which episode is each title (the show was clear)");
    assert_eq!(asked[0].prompt, "Which episode is each video?");
    assert!(s.finished && !s.stopped);
    assert_eq!((s.steps["v_choose"].status.as_str(), s.steps["v_choose"].detail.as_str()), ("The Sopranos", "Season 3, episodes 1–4"));
    // The duplicate of episode 2 is never read.
    let ripped: Vec<&str> = s.steps["v_rip"].items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ripped, ["title-1", "title-2", "title-3", "title-4", "title-6"]);
    assert_eq!(s.steps["v_rip"].items[1].label, "S03E02 Proshai, Livushka");
    let show = "~/Videos/Shows/The Sopranos (1999) [tmdbid-1398]";
    assert_eq!(
        filed(&s),
        [
            format!("{show}/Season 03/The Sopranos (1999) - S03E01 - Mr. Ruggerio's Neighborhood.mkv"),
            format!("{show}/Season 03/The Sopranos (1999) - S03E02 - Proshai, Livushka.mkv"),
            format!("{show}/Season 03/The Sopranos (1999) - S03E03 - Fortunate Son.mkv"),
            format!("{show}/Season 03/The Sopranos (1999) - S03E04 - Employee of the Month.mkv"),
            format!("{show}/extras/Title 6.mkv"),
        ]
    );
}

#[tokio::test]
async fn two_titles_claiming_one_episode_are_asked_about_again() {
    let e = engine(SimDisc::Season);
    let (s, asked) = run(&e, &recipes::video(), |q| if asked_again(q) { Answer::PerItem(BTreeMap::new()) } else { Answer::PerItem(BTreeMap::from([("title-4".to_owned(), Some(0))])) }).await;
    assert_eq!(asked.len(), 2);
    assert!(asked[1].prompt.starts_with("Two videos can't both be S03E01"), "{}", asked[1].prompt);
    assert_eq!(s.steps["v_rip"].items.len(), 4);
}

fn asked_again(q: &Question) -> bool {
    q.prompt.starts_with("Two videos")
}

#[tokio::test]
async fn a_disc_the_runtimes_place_runs_without_a_question() {
    let e = engine(SimDisc::SeasonDisc2);
    let (s, asked) = run(&e, &recipes::video(), |q| panic!("asked {}", q.prompt)).await;
    assert!(asked.is_empty());
    assert_eq!(s.steps["v_choose"].detail, "Season 3, episodes 5–8", "disc 2 starts where its runtimes say");
    let files = filed(&s);
    assert_eq!(files.len(), 4);
    assert!(files[0].ends_with("Season 03/The Sopranos (1999) - S03E05 - Another Toothpick.mkv"), "{files:?}");
    assert!(files[3].ends_with("S03E08 - He Is Risen.mkv"), "{files:?}");
}

#[tokio::test]
async fn a_box_set_disc_that_fits_two_seasons_asks_which() {
    let e = engine(SimDisc::BoxSet);
    let (s, asked) = run(&e, &recipes::video(), |q| match &q.shape {
        QuestionShape::PickOne { options, details, skip, .. } => {
            assert!(*skip);
            assert_eq!(options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["Twin Peaks — Season 1", "Twin Peaks — Season 2"]);
            let d = details.as_ref().expect("details offered");
            assert_eq!((d["title"].as_str(), d["kind"].as_str(), d["season"].as_str()), ("Twin Peaks", "series", "1"));
            Answer::Pick(0)
        }
        other => panic!("{other:?}"),
    })
    .await;
    assert_eq!(asked.len(), 1, "the season is asked; then the runtimes place every title");
    assert_eq!(asked[0].prompt, "Which film or show?");
    assert_eq!(s.steps["v_choose"].detail, "Season 1, episodes 2–5", "the pilot's 94 minutes isn't on this disc");
    assert_eq!(s.steps["v_rip"].items.len(), 4, "not the 4-minute extra");
    assert!(filed(&s)[0].ends_with("Twin Peaks (1990) [tmdbid-1920]/Season 01/Twin Peaks (1990) - S01E02 - Traces to Nowhere.mkv"), "{:?}", filed(&s));
}

#[tokio::test]
async fn a_skipped_season_disc_keeps_its_titles_unnumbered_under_the_label() {
    let e = engine(SimDisc::BoxSet);
    let (s, asked) = run(&e, &recipes::video(), |_| Answer::Skip).await;
    assert_eq!(asked.len(), 1, "skipping the identity leaves nothing to match titles against");
    assert_eq!(s.steps["v_choose"].status, "Skipped");
    assert_eq!(s.steps["v_rip"].items.len(), 4, "the scan's four episodes");
    assert_eq!(filed(&s), (1..=4).map(|n| format!("~/Videos/Shows/Twin Peaks Gold/extras/Title {n}.mkv")).collect::<Vec<_>>(), "numbered from a guess would be silently wrong on disc 2");
}
