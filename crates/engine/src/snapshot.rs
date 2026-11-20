//! What a run shows (the snapshot) and what can be said to it (commands).
//! Any window, the CLI and the tests see a run only through these.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    /// Not started: *Ready*, *After choose*.
    Idle,
    Busy,
    Ok,
    /// Asking you something, or a setting it needs is unset.
    Needs,
    /// Waiting on a step before it that needs you.
    Wait,
    /// Holding items until you approve.
    Hold,
    /// A problem with some items, or the step failed.
    Bad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Waiting,
    Busy,
    Ok,
    Bad,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemView {
    pub id: String,
    pub label: String,
    pub state: ItemState,
    /// The verdict or what's happening: *✓ 103*, *no match*, *re-reading 40%*.
    pub note: String,
    /// Done and checked against something independent (AccurateRip, a
    /// catalogue's hashes): only a step that verifies sets it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub verified: bool,
}

/// One tile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepView {
    /// The step kind in the registry: `rip`, `musicbrainz`.
    pub step: String,
    /// Its name on the tile.
    pub name: String,
    /// What it uses: *cyanrip, AccurateRip*; *~/Music/FLAC*.
    pub from: String,
    pub state: StepState,
    /// The outcome, in the job's unit: *17 tracks*, *294 MB*.
    pub status: String,
    /// At most one word or two that qualify the outcome: *43:45*, *58 %*,
    /// *by layout*. Cleared whenever the status is set without one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub qualifier: String,
    /// What it's on right now, or the one most useful fact about the result.
    pub detail: String,
    pub progress: Option<f32>,
    pub items: Vec<ItemView>,
    /// How many items it expects in all, once it knows (the disc's tracks),
    /// so the ones still to come can be shown waiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    /// The steps feeding this one.
    pub preds: Vec<String>,
    pub hold: bool,
    /// What it did: what came in and went out, what it decided, the files
    /// it made, its log, and when. (What it found and offered is in what it
    /// sent on, which the run keeps for the window, each fact marked with
    /// the step; its tool's output is `Run::raw`.)
    #[serde(default)]
    pub record: StepRecord,
}

/// One step's account of its run. See docs/ux/step-record.md.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    /// When it began work and when it was done, in seconds since the epoch.
    #[serde(default)]
    pub started: Option<u64>,
    #[serde(default)]
    pub finished: Option<u64>,
    /// What came in, by kind (from the steps in `preds`).
    #[serde(default)]
    pub ins: Vec<Flow>,
    /// What went out, by kind, to the steps after it.
    #[serde(default)]
    pub outs: Vec<Flow>,
    /// For Choose: the candidates that reached it, as they arrived (none
    /// in a record kept from before Choose kept them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<crate::context::Candidate>>,
    /// Choose's choice, and why.
    #[serde(default)]
    pub decided: Option<Decision>,
    /// What the step found, in its own shape, for its sidebar to show:
    /// Check's comparison of a dump with Redump's. Only the window reads
    /// it; no step does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<serde_json::Value>,
    /// Files it wrote, with their sizes.
    #[serde(default)]
    pub files: Vec<Made>,
    /// Its own lines, timestamped.
    #[serde(default)]
    pub log: Vec<LogLine>,
    /// How many lines its tool printed (they're in `Run::raw`).
    #[serde(default)]
    pub raw: usize,
}

/// Items of one kind passing in or out, and how many.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    pub kind: crate::item::ItemKind,
    pub count: usize,
}

/// What Choose settled on: the candidate (`None` when it went on without
/// one) and why, in a few words (*only one*, *best match 0.97*, *you picked*,
/// *entered by hand*).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub chosen: Option<String>,
    pub title: String,
    pub why: String,
}

/// A file a step wrote.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Made {
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogLine {
    /// Seconds since the epoch.
    pub at: u64,
    pub line: String,
}

impl StepRecord {
    fn bump(list: &mut Vec<Flow>, kind: crate::item::ItemKind) {
        match list.iter_mut().find(|f| f.kind == kind) {
            Some(f) => f.count += 1,
            None => list.push(Flow { kind, count: 1 }),
        }
    }

    pub(crate) fn came_in(&mut self, kind: crate::item::ItemKind) {
        Self::bump(&mut self.ins, kind);
    }

    pub(crate) fn went_out(&mut self, kind: crate::item::ItemKind) {
        Self::bump(&mut self.outs, kind);
    }
}

impl StepView {
    pub fn item_mut(&mut self, id: &str, label: &str) -> &mut ItemView {
        if let Some(i) = self.items.iter().position(|i| i.id == id) {
            return &mut self.items[i];
        }
        self.items.push(ItemView { id: id.into(), label: label.into(), state: ItemState::Waiting, note: String::new(), verified: false });
        self.items.last_mut().unwrap()
    }

    pub fn count(&self, state: ItemState) -> usize {
        self.items.iter().filter(|i| i.state == state).count()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub label: String,
    pub detail: String,
    /// Picking it loses something already there (Replace them): its
    /// button is drawn in the bad colour.
    #[serde(default)]
    pub destructive: bool,
    /// A picture of it, by path (a cover to pick): the answers are shown
    /// as pictures side by side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum QuestionShape {
    /// One of a few candidates, the best already picked.
    PickOne {
        options: Vec<Choice>,
        preselected: usize,
        /// Offers typing the details yourself, starting from these
        /// (`album`, `album_artist`, `year`, `title.3`, `artist.3`, …).
        #[serde(default)]
        details: Option<BTreeMap<String, String>>,
        /// Offers going on without an answer.
        #[serde(default)]
        skip: bool,
    },
    /// A value: a folder, an offset, a name.
    Value { label: String, suggested: String },
    /// A choice per item: which episode each DVD title is, or skip it.
    /// Every item picks from the same `options`; each comes with the best
    /// guess already picked (`None`: skipped unless you say otherwise).
    /// Answered with [`Answer::PerItem`].
    PerItem { items: Vec<ItemChoice>, options: Vec<Choice> },
}

/// One item in a [`QuestionShape::PerItem`] question.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemChoice {
    /// The item's id, which keys the answer: `title-3`.
    pub id: String,
    pub label: String,
    /// What helps you tell: *44:12, 12 chapters, duplicate of title 2*.
    pub detail: String,
    /// Index into the question's `options`, or `None` for skip.
    pub preselected: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub prompt: String,
    pub shape: QuestionShape,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Answer {
    Pick(usize),
    Value(String),
    /// Details you typed, keyed as in [`QuestionShape::PickOne`]'s `details`.
    Details(BTreeMap<String, String>),
    Skip,
    /// Per item id, the option picked or `None` to skip it. An item left
    /// out keeps its preselected choice.
    PerItem(BTreeMap<String, Option<usize>>),
}

/// A run as it is right now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub recipe: String,
    /// Step ids, left to right and top to bottom.
    pub order: Vec<String>,
    pub steps: BTreeMap<String, StepView>,
    /// The step asking, and what.
    pub question: Option<(String, Question)>,
    pub finished: bool,
    pub stopped: bool,
}

impl Snapshot {
    /// Steps that are idle because something before them needs you show it,
    /// so the tile reads *Waits for Choose* rather than just *Ready*. One
    /// straight after the step that needs you also says why (*Check needs
    /// you*), which its card shows; one further on only waits.
    pub(crate) fn derive_waits(&mut self) {
        for id in self.order.clone() {
            let view = &self.steps[&id];
            if !matches!(view.state, StepState::Idle | StepState::Wait) {
                continue;
            }
            let blocker = view.preds.iter().find_map(|p| {
                let pv = &self.steps[p];
                // Holding an item for you to settle (Check's dump Redump
                // disagrees with) is needing you too.
                let holding = pv.state == StepState::Bad && pv.record.finished.is_none() && pv.items.iter().any(|i| i.state == ItemState::Bad);
                match pv.state {
                    StepState::Needs => Some((pv.name.clone(), true)),
                    StepState::Hold => Some((pv.name.clone(), false)),
                    _ if holding => Some((pv.name.clone(), true)),
                    StepState::Wait => pv.status.strip_prefix("Waits for ").map(|n| (n.to_owned(), false)),
                    _ => None,
                }
            });
            let idle = idle_status(&view.preds, &self.steps);
            let view = self.steps.get_mut(&id).unwrap();
            match blocker {
                Some((name, asks)) => {
                    view.state = StepState::Wait;
                    view.status = format!("Waits for {name}");
                    view.detail = if asks { format!("{name} needs you") } else { String::new() };
                }
                None if view.state == StepState::Wait => {
                    view.state = StepState::Idle;
                    view.status = idle;
                    view.detail.clear();
                }
                None => {}
            }
        }
    }

    /// The run's one line: *about 4 min left*, *needs you*, *1 track*.
    pub fn headline(&self) -> String {
        if self.stopped {
            return "Stopped".into();
        }
        if self.question.is_some() {
            return "Needs you".into();
        }
        if self.steps.values().any(|s| s.state == StepState::Hold && s.count(ItemState::Waiting) > 0) {
            return "Holding".into();
        }
        let bad: usize = self.steps.values().map(|s| s.count(ItemState::Bad)).sum();
        if bad > 0 {
            return format!("{bad} need{} you", if bad == 1 { "s" } else { "" });
        }
        if self.finished {
            return "Done".into();
        }
        "Running".into()
    }
}

pub(crate) fn idle_status(preds: &[String], steps: &BTreeMap<String, StepView>) -> String {
    match preds.first().and_then(|p| steps.get(p)) {
        Some(_) if preds.len() > 1 => format!("After all {}", preds.len()),
        Some(p) => format!("After {}", p.name.to_lowercase()),
        None => "Ready".into(),
    }
}

/// What can be said to a running recipe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    Answer {
        step: String,
        answer: Answer,
    },
    /// Try an item again another way, or settle it (keep, leave out).
    Retry {
        step: String,
        item: String,
        option: String,
    },
    /// Let held items through.
    Approve {
        step: String,
    },
    Stop,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_per_item_question_and_its_answer_round_trip_as_json() {
        let q = Question {
            prompt: "Which episode is each title?".into(),
            shape: QuestionShape::PerItem {
                items: vec![
                    ItemChoice { id: "title-1".into(), label: "Title 1".into(), detail: "50:12".into(), preselected: Some(0) },
                    ItemChoice { id: "title-5".into(), label: "Title 5".into(), detail: "duplicate of title 2".into(), preselected: None },
                ],
                options: vec![Choice { label: "S03E01".into(), detail: "Mr. Ruggerio's Neighborhood".into(), destructive: false, picture: None }],
            },
        };
        let json = serde_json::to_string(&q).unwrap();
        assert!(json.contains(r#""shape":"per_item""#), "{json}");
        assert_eq!(serde_json::from_str::<Question>(&json).unwrap(), q);
        let a = Answer::PerItem([("title-1".to_owned(), Some(0)), ("title-5".to_owned(), None)].into());
        assert_eq!(serde_json::from_str::<Answer>(&serde_json::to_string(&a).unwrap()).unwrap(), a);
    }
}
