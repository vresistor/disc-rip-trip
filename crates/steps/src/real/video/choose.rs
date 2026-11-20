//! Choose for video: which film or which season, then which title is the
//! film or which episode. Its own step rather than a wider Choose: a step
//! takes one kind of item, and a Choose taking anything would be offered
//! after a data disc too. Like Choose it only reads what the disc carries
//! in (TMDb's candidates, the scan's titles), so it's the same with real
//! and simulated lookups.
//!
//! It decides by itself when it can, and asks otherwise, always with the
//! details to type and Skip: *which film or show?* when no candidate is clearly ahead, and *which
//! episode is each title?* (a choice per title, the best guesses picked)
//! when the runtimes don't place every title. Duplicates, extras and a
//! play-all start out skipped.

use super::matching::{self, Episode, Role};
use super::{facts, hms, tmdb};
use anyhow::Result;
use async_trait::async_trait;
use drt_engine::snapshot::QuestionShape;
use drt_engine::{Answer, Candidate, Choice, Context, Cx, Fact, Group, Inlet, ItemChoice, ItemKind, Outlet, Question, Step, StepInfo, Subject};
use std::collections::BTreeMap;

pub fn choose_info() -> StepInfo {
    StepInfo::new("choose-video", "Choose", Group::Decide, ItemKind::VideoDisc, ItemKind::VideoDisc)
        .needs(&["candidates"])
        .gives(&["release"])
        .unless(&["release"])
        .from("pick the film or show, and its videos")
}

pub struct ChooseVideo;

/// What one title can be, as the per-title question offers it.
#[derive(Clone, Debug)]
struct Options {
    roles: Vec<Role>,
    choices: Vec<Choice>,
}

impl Options {
    fn of(c: Option<&Candidate>, series: bool, episodes: &[Episode], season: u32) -> Options {
        let mut roles = Vec::new();
        let mut choices = Vec::new();
        if series {
            for e in episodes {
                roles.push(Role::Episode(e.number));
                let mins = e.runtime_mins.map(|m| format!(", {m} min")).unwrap_or_default();
                choices.push(Choice { label: format!("S{season:02}E{:02}", e.number), detail: format!("{}{mins}", e.name) });
            }
        } else {
            roles.push(Role::Film);
            choices.push(Choice { label: c.map(|c| c.title.clone()).unwrap_or_else(|| "The film".into()), detail: "the film".into() });
        }
        roles.push(Role::Extra);
        choices.push(Choice { label: "Extra".into(), detail: "kept with the bonus material".into() });
        Options { roles, choices }
    }

    fn index(&self, r: &Role) -> Option<usize> {
        self.roles.iter().position(|x| x == r)
    }
}

#[async_trait]
impl Step for ChooseVideo {
    fn info(&self) -> StepInfo {
        choose_info()
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        while let Some(mut disc) = inlet.recv().await {
            let pick = self.identity(&cx, disc.context()).await?;
            match &pick {
                Some(c) => disc.context_mut().choose(c.clone()),
                None => disc.context_mut().choose_none(),
            }
            let roles = self.roles(&cx, disc.context(), pick.as_ref()).await?;
            let (summary, detail) = write(disc.context_mut(), &roles, pick.as_ref());
            cx.status(summary, detail);
            outlet.send(disc).await;
        }
        Ok(())
    }
}

impl ChooseVideo {
    /// Which film or season, asking when no candidate is clearly ahead.
    async fn identity(&self, cx: &Cx, disc: &Context) -> Result<Option<Candidate>> {
        let mut cands = disc.candidates().to_vec();
        cx.options(&cands);
        cands.sort_by(|a, b| b.score.total_cmp(&a.score));
        let clear = cands.len() == 1 || (cands.len() > 1 && cands[0].score - cands[1].score >= 0.05);
        if clear {
            let c = cands.first().cloned();
            let why = if cands.len() == 1 { "only one".to_owned() } else { format!("best match {:.2}", cands[0].score) };
            cx.decided(c.as_ref(), why);
            return Ok(c);
        }
        let options = cands.iter().map(|c| Choice { label: c.title.clone(), detail: c.detail.clone() }).collect();
        let label = disc.get(Subject::Disc, facts::LABEL).unwrap_or("").to_owned();
        let best = cands.first();
        let get = |k: &str| best.and_then(|b| b.facts.iter().find(|f| f.key == k)).map(|f| f.value.clone()).unwrap_or_default();
        let kind = disc.get(Subject::Disc, facts::DISC_KIND).unwrap_or("film").to_owned();
        let guess = tmdb::parse_label(&label);
        let details: BTreeMap<String, String> = [
            ("title", if best.is_some() { get("title") } else { super::from_label(&label) }),
            ("year", get("year")),
            ("kind", if best.is_some() { get("video_kind") } else { kind }),
            ("season", if best.is_some() { get("season") } else { guess.season.map(|s| s.to_string()).unwrap_or_default() }),
            ("tmdbid", get("tmdbid")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        cx.view(|v| v.detail = if cands.is_empty() { "nothing found".into() } else { format!("{} to choose from", cands.len()) });
        let prompt = if cands.is_empty() { "What is this disc?" } else { "Which film or show?" };
        let q = Question { prompt: prompt.into(), shape: QuestionShape::PickOne { options, preselected: 0, details: Some(details), skip: true } };
        let (pick, why) = match cx.ask(q).await? {
            Answer::Pick(i) => match cands.get(i) {
                Some(c) => (Some(c.clone()), "you picked"),
                None => (None, "nothing to pick from"),
            },
            Answer::Details(d) => (Some(typed(&d)), "entered by hand"),
            Answer::Skip => (None, "you skipped"),
            Answer::Value(_) | Answer::PerItem(_) => (cands.first().cloned(), "the likeliest"),
        };
        cx.decided(pick.as_ref(), why);
        Ok(pick)
    }

    /// What each title is: matched, then asked about when unsure.
    async fn roles(&self, cx: &Cx, disc: &Context, pick: Option<&Candidate>) -> Result<Vec<(u32, Role)>> {
        let (titles, guessed_series, label_season) = {
            let c = disc;
            let label = c.get(Subject::Disc, facts::LABEL).unwrap_or("").to_owned();
            (super::titles(c), c.get(Subject::Disc, facts::DISC_KIND) == Some("series"), tmdb::parse_label(&label).season)
        };
        let fact = |k: &str| pick.and_then(|c| c.facts.iter().find(|f| f.key == k)).map(|f| f.value.clone());
        let Some(c) = pick else {
            // Nobody said what it is: the scan's guess, unasked, since
            // there's nothing to match the titles against. Episodes aren't
            // numbered from a guess (disc 2 would be filed as episodes 1 to
            // 4 without a word); they're kept under their title numbers.
            let picked = super::to_rip(disc);
            let film = !guessed_series && picked.len() == 1;
            return Ok(titles
                .iter()
                .map(|t| {
                    let role = match picked.contains(&t.part) {
                        false => Role::Skip,
                        true if film => Role::Film,
                        true => Role::Extra,
                    };
                    (t.part, role)
                })
                .collect());
        };
        let series = fact("video_kind").as_deref() == Some("series");
        let season = fact("season").and_then(|s| s.parse().ok()).or(label_season).unwrap_or(1);
        let mut episodes = tmdb::episodes_of(c);
        if series && episodes.is_empty() {
            // Typed in, or a season TMDb knows nothing of: numbers only.
            episodes = (1..=40).map(|n| Episode { season, number: n, name: String::new(), runtime_mins: None }).collect();
        }
        let (mut roles, sure) = if series { matching::episodes(&titles, &episodes) } else { matching::film(&titles, fact("runtime").and_then(|m| m.parse().ok())) };
        if sure {
            return Ok(roles);
        }
        let opts = Options::of(Some(c), series, &episodes, season);
        let rows: Vec<ItemChoice> = titles
            .iter()
            .map(|t| {
                let role = roles.iter().find(|(p, _)| *p == t.part).map(|(_, r)| r.clone()).unwrap_or(Role::Skip);
                ItemChoice { id: format!("{}{}", super::ITEM_PREFIX, t.part), label: format!("Video {}", t.part), detail: row_detail(disc, t.part), preselected: opts.index(&role) }
            })
            .collect();
        let mut prompt = if series { "Which episode is each video?".to_owned() } else { "Which video is the film?".to_owned() };
        loop {
            let q = Question { prompt: prompt.clone(), shape: QuestionShape::PerItem { items: rows.clone(), options: opts.choices.clone() } };
            let Answer::PerItem(answer) = cx.ask(q).await? else { return Ok(roles) };
            let mut next = Vec::new();
            for row in &rows {
                let pick = answer.get(&row.id).copied().unwrap_or(row.preselected);
                let role = pick.and_then(|i| opts.roles.get(i).cloned()).unwrap_or(Role::Skip);
                next.push((row.id.trim_start_matches(super::ITEM_PREFIX).parse().unwrap_or(0), role));
            }
            // Two titles can't be one episode, nor two features one film.
            let claimed: Vec<&Role> = next.iter().map(|(_, r)| r).filter(|r| matches!(r, Role::Episode(_) | Role::Film)).collect();
            let clash = claimed.iter().enumerate().find(|(i, r)| claimed[i + 1..].contains(r)).map(|(_, r)| (*r).clone());
            match clash {
                None => {
                    roles = next;
                    return Ok(roles);
                }
                Some(r) => {
                    let what = match r {
                        Role::Episode(n) => format!("S{season:02}E{n:02}"),
                        _ => "the film".into(),
                    };
                    prompt = format!("Two videos can't both be {what}. Which episode is each video?");
                    if !series {
                        prompt = format!("Two videos can't both be {what}. Which video is the film?");
                    }
                }
            }
        }
    }
}

/// *1:57:30, 34 chapters, eng fra, duplicate of title 1*.
fn row_detail(c: &Context, part: u32) -> String {
    let p = Subject::Part(part);
    let mut v = vec![hms(c.get(p, facts::LENGTH).and_then(|l| l.parse().ok()).unwrap_or(0))];
    if let Some(ch) = c.get(p, facts::CHAPTERS).filter(|c| *c != "0") {
        v.push(format!("{ch} chapter{}", if ch == "1" { "" } else { "s" }));
    }
    if let Some(a) = c.get(p, facts::AUDIO).filter(|a| !a.is_empty()) {
        v.push(a.replace(',', " "));
    }
    if let Some(d) = c.get(p, facts::DUPLICATE_OF) {
        v.push(format!("duplicate of video {d}"));
    }
    v.join(", ")
}

/// Records what each title is and which to rip; gives the tile's lines.
fn write(c: &mut Context, roles: &[(u32, Role)], pick: Option<&Candidate>) -> (String, String) {
    let label_season = tmdb::parse_label(c.get(Subject::Disc, facts::LABEL).unwrap_or("")).season;
    let season = pick.and_then(|p| p.facts.iter().find(|f| f.key == "season")).map(|f| f.value.clone()).or(label_season.map(|s| s.to_string())).unwrap_or_else(|| "1".into());
    let episodes = pick.map(tmdb::episodes_of).unwrap_or_default();
    let mut rip = Vec::new();
    let mut eps = Vec::new();
    for (part, role) in roles {
        let p = Subject::Part(*part);
        let word = match role {
            Role::Film => "film",
            Role::Episode(_) => "episode",
            Role::Extra => "extra",
            Role::Skip => continue,
        };
        rip.push(part.to_string());
        c.add(p, facts::ROLE, word, "Choose");
        if let Role::Episode(n) = role {
            eps.push(*n);
            c.add(p, facts::SEASON, season.clone(), "Choose");
            c.add(p, facts::EPISODE, n.to_string(), "Choose");
            if let Some(e) = episodes.iter().find(|e| e.number == *n).filter(|e| !e.name.is_empty()) {
                c.add(p, facts::EPISODE_TITLE, e.name.clone(), "Choose");
            }
        }
    }
    c.add(Subject::Disc, facts::RIP, rip.join(","), "Choose");
    let title = c.get(Subject::Disc, "title").map(str::to_owned).unwrap_or_else(|| "Skipped".into());
    let detail = if !eps.is_empty() {
        eps.sort();
        let span = if eps.len() == 1 { format!("episode {}", eps[0]) } else { format!("episodes {}–{}", eps[0], eps[eps.len() - 1]) };
        format!("Season {season}, {span}")
    } else {
        let year = c.get(Subject::Disc, "year").unwrap_or("").to_owned();
        let n = rip.len();
        [year, format!("{n} video{}", if n == 1 { "" } else { "s" })].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")
    };
    (title, detail)
}

/// A candidate made of what you typed: `title`, `year`, `kind` (film or
/// series), `season`, `tmdbid`.
fn typed(d: &BTreeMap<String, String>) -> Candidate {
    let get = |k: &str| d.get(k).map(|v| v.trim().to_owned()).unwrap_or_default();
    let series = get("kind").to_lowercase().starts_with('s');
    let mut facts = vec![];
    for (k, v) in [("title", get("title")), ("year", get("year")), ("tmdbid", get("tmdbid")), ("video_kind", (if series { "series" } else { "film" }).to_owned())] {
        if !v.is_empty() {
            facts.push(Fact { step: String::new(), subject: Subject::Disc, key: k.into(), value: v, source: "you".into() });
        }
    }
    if series {
        let season = get("season");
        facts.push(Fact { step: String::new(), subject: Subject::Disc, key: "season".into(), value: if season.is_empty() { "1".into() } else { season }, source: "you".into() });
    }
    Candidate { step: String::new(), id: "typed".into(), source: "you".into(), title: get("title"), detail: "typed in".into(), score: 1.0, facts }
}
