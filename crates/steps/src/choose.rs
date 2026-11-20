//! Choose: settles which candidate is the identity, from the candidates the
//! lookups wired into it offered on the disc. The one step that may need
//! you. The same with real and simulated lookups, since it only reads what
//! the disc carries in.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::snapshot::QuestionShape;
use drt_engine::{Answer, Candidate, Choice, Context, Cx, Fact, Group, Inlet, ItemKind, Outlet, Question, Step, StepInfo, Subject};
use std::collections::BTreeMap;

pub struct Choose;

#[async_trait]
impl Step for Choose {
    fn info(&self) -> StepInfo {
        StepInfo::new("choose", "Choose", Group::Decide, ItemKind::AudioDisc, ItemKind::AudioDisc).needs(&["candidates"]).gives(&["release"]).unless(&["release"]).from("pick which album this is")
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            // What the lookups wired into it offered, in the order they
            // came; the best first, ties kept in that order.
            let mut cands = disc.context().candidates().to_vec();
            cx.options(&cands);
            cands.sort_by(|a, b| b.score.total_cmp(&a.score));
            // One release, or one clearly ahead: no question. Several
            // close ones, or none at all: ask, with the details to type and
            // Skip always among the answers. The disc's own CD-Text alone
            // is never enough to go on without you: it's offered, with the
            // details to type, as when nothing was found.
            let found = cands.first().is_some_and(|c| c.id != CD_TEXT_ID);
            let clear = found && (cands.len() == 1 || cands[0].score - cands[1].score >= 0.05);
            // Why, for its record.
            let why: String;
            let pick = if clear {
                why = if cands.len() == 1 { "only one".into() } else { format!("best match {:.2}", cands[0].score) };
                Some(cands[0].clone())
            } else {
                let options = cands.iter().map(|c| Choice { label: c.title.clone(), detail: c.detail.clone(), destructive: false, picture: None }).collect();
                let details = Some(prefill(disc.context(), &cands));
                let prompt = if found { "Which album is this?" } else { "What is this disc?" };
                let q = Question { prompt: prompt.into(), shape: QuestionShape::PickOne { options, preselected: 0, details, skip: true } };
                cx.view(|v| v.detail = if cands.is_empty() { "nothing found".into() } else { format!("{} to choose from", cands.len()) });
                let (pick, w) = match cx.ask(q).await? {
                    Answer::Pick(i) => match cands.get(i) {
                        Some(c) => (Some(c.clone()), "you picked"),
                        None => (None, "nothing to pick from"),
                    },
                    Answer::Details(d) => (Some(typed(d)), "entered by hand"),
                    Answer::Skip => (None, "you skipped"),
                    Answer::Value(_) | Answer::PerItem(_) => (cands.first().cloned(), "the likeliest"),
                };
                why = w.into();
                pick
            };
            cx.decided(pick.as_ref(), why.clone());
            cx.log(match &pick {
                Some(c) => format!("chose {} ({}): {why}", c.title, c.detail),
                None => format!("went on without a release: {why}"),
            });
            match pick {
                Some(c) => {
                    // The release, its year (which tells pressings apart), and
                    // the catalogue number first, so only the label's name
                    // can be cut short.
                    let fact = |k: &str| c.facts.iter().find(|f| f.subject == Subject::Disc && f.key == k).map(|f| f.value.clone()).unwrap_or_default();
                    let pressing = [fact("catalognumber"), fact("label")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                    cx.outcome(c.title.clone(), fact("year"), pressing);
                    disc.context_mut().choose(c);
                }
                None => {
                    cx.status("Skipped", "not named");
                    disc.context_mut().choose_none();
                }
            }
            outlet.send(disc).await;
        }
        Ok(())
    }
}

/// What the details form starts with: the best candidate that reached
/// Choose (none: empty), one entry per track of the disc.
pub fn prefill(disc: &Context, cands: &[Candidate]) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let best = cands.first();
    let get = |s: Subject, k: &str| best.and_then(|c| c.facts.iter().find(|f| f.subject == s && f.key == k)).map(|f| f.value.clone()).unwrap_or_default();
    for k in ["album", "album_artist", "year"] {
        m.insert(k.to_owned(), get(Subject::Disc, k));
    }
    for t in 1..=disc.parts() {
        m.insert(format!("title.{t}"), get(Subject::Part(t), "title"));
        m.insert(format!("artist.{t}"), get(Subject::Part(t), "artist"));
    }
    m
}

/// How CD-Text's candidate ranks: below every database's match. The disc's
/// own text is right about this disc, but often bare (capitals, no year),
/// so a release that matches wins without asking; alone, it's the one.
pub const CD_TEXT_SCORE: f32 = 0.3;

/// CD-Text's candidate's id.
pub const CD_TEXT_ID: &str = "cd-text";

/// What the disc's own CD-Text says it is, as one candidate: its titles,
/// and its artist and album when it has them. Audio CD read them with the
/// table of contents (`cdtext.*`); `None` when the disc has none.
pub fn cdtext_candidate(disc: &Context) -> Option<Candidate> {
    let source = "CD-Text";
    let fact = |subject, key: &str, value: &str| Fact { subject, key: key.into(), value: value.into(), source: source.into(), step: String::new() };
    let mut facts = Vec::new();
    let mut titles = 0;
    for p in 1..=disc.parts() {
        if let Some(t) = disc.get(Subject::Part(p), "cdtext.title") {
            facts.push(fact(Subject::Part(p), "title", t));
            titles += 1;
        }
    }
    let album = disc.get(Subject::Disc, "cdtext.album").unwrap_or("").to_owned();
    let artist = disc.get(Subject::Disc, "cdtext.artist").unwrap_or("").to_owned();
    if titles == 0 && album.is_empty() && artist.is_empty() {
        return None;
    }
    if !album.is_empty() {
        facts.push(fact(Subject::Disc, "album", &album));
    }
    if !artist.is_empty() {
        facts.push(fact(Subject::Disc, "album_artist", &artist));
    }
    let named = [artist, album].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" — ");
    let count = format!("{titles} track name{}", if titles == 1 { "" } else { "s" });
    Some(Candidate {
        step: String::new(),
        id: CD_TEXT_ID.into(),
        source: source.into(),
        title: "From the disc (CD-Text)".into(),
        detail: if named.is_empty() { count } else { format!("{named}, {count}") },
        score: CD_TEXT_SCORE,
        facts,
    })
}

/// A candidate made of what you typed.
fn typed(d: BTreeMap<String, String>) -> Candidate {
    let mut facts = Vec::new();
    for (k, v) in &d {
        if v.trim().is_empty() {
            continue;
        }
        let (subject, key) = match k.split_once('.') {
            Some((key, n)) => match n.parse() {
                Ok(n) => (Subject::Part(n), key),
                Err(_) => continue,
            },
            None => (Subject::Disc, k.as_str()),
        };
        facts.push(Fact { step: String::new(), subject, key: key.into(), value: v.trim().into(), source: "you".into() });
    }
    let album = d.get("album").cloned().unwrap_or_default();
    let artist = d.get("album_artist").cloned().unwrap_or_default();
    Candidate {
        step: String::new(),
        id: "typed".into(),
        source: "you".into(),
        title: [artist, album].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" — "),
        detail: "typed in".into(),
        score: 1.0,
        facts,
    }
}
