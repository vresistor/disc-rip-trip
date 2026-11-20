//! What an item knows: the facts learned about it on its way here, where
//! each came from, the candidates the lookups it passed offered, and, once
//! it has been through Choose, the one chosen.
//!
//! It rides on the item (see `Item::context`), so a step sees only what
//! reached it along its wires. The engine also keeps a merge of everything
//! steps sent on, for the window; no step can read that.

use serde::{Deserialize, Serialize};

/// What a fact is about: the disc's identity as a whole, or one of its parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Subject {
    Disc,
    Part(u32),
}

/// One piece of metadata and the source it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub subject: Subject,
    /// What kind of fact: `title`, `artist`, `year`, `cover`, `length`, …
    pub key: String,
    pub value: String,
    /// The source's name: `MusicBrainz`, `CD-Text`, `the disc`.
    pub source: String,
    /// The id of the step that added it: the engine marks it as the item
    /// leaves that step (empty in data from before steps were recorded).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub step: String,
}

/// One identity a source offers as what the disc might be.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub source: String,
    pub title: String,
    pub detail: String,
    /// How well it fits what's known about the disc, 0 to 1.
    pub score: f32,
    pub facts: Vec<Fact>,
    /// The id of the step that offered it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub step: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Context {
    #[serde(default)]
    facts: Vec<Fact>,
    /// What the lookups on the way offered. Choose takes one and drops
    /// the rest.
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default)]
    chosen: Option<Candidate>,
}

impl Context {
    pub fn add(&mut self, subject: Subject, key: &str, value: impl Into<String>, source: &str) {
        self.facts.push(Fact { subject, key: key.into(), value: value.into(), source: source.into(), step: String::new() });
    }

    /// Every fact found, in order.
    pub fn facts(&self) -> &[Fact] {
        &self.facts
    }

    /// The facts a step found.
    pub fn found_by<'a>(&'a self, step: &'a str) -> impl Iterator<Item = &'a Fact> {
        self.facts.iter().filter(move |f| f.step == step)
    }

    /// The candidates a step offered.
    pub fn offered_by<'a>(&'a self, step: &'a str) -> impl Iterator<Item = &'a Candidate> {
        self.candidates.iter().filter(move |c| c.step == step)
    }

    /// The fact in effect for this subject and key: the chosen identity's
    /// if it has one, else the first found.
    fn effective(&self, subject: Subject, key: &str) -> Option<&Fact> {
        let from_chosen = self.chosen.as_ref().and_then(|c| c.facts.iter().find(|f| f.subject == subject && f.key == key));
        from_chosen.or_else(|| self.all(subject, key).next())
    }

    /// Every value given for this fact, in the order they were found.
    pub fn all(&self, subject: Subject, key: &str) -> impl Iterator<Item = &Fact> {
        let key = key.to_owned();
        self.facts.iter().filter(move |f| f.subject == subject && f.key == key)
    }

    /// The value to use: the chosen identity's if it has one, else the first
    /// found.
    pub fn get(&self, subject: Subject, key: &str) -> Option<&str> {
        self.effective(subject, key).map(|f| f.value.as_str())
    }

    /// Replaces every value found for this fact with one: what a re-read
    /// found instead (a dump's hashes after it's refined).
    pub fn replace(&mut self, subject: Subject, key: &str, value: impl Into<String>, source: &str) {
        self.facts.retain(|f| !(f.subject == subject && f.key == key));
        self.add(subject, key, value, source);
    }

    /// A lookup's answer: identities the disc might be, for Choose.
    pub fn offer(&mut self, candidates: impl IntoIterator<Item = Candidate>) {
        self.candidates.extend(candidates);
    }

    /// The candidates the lookups on the way offered.
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    /// Settles the identity: this one is chosen, and the other candidates
    /// go no further.
    pub fn choose(&mut self, c: Candidate) {
        self.chosen = Some(c);
        self.candidates.clear();
    }

    /// Goes on without an identity: the candidates go no further either.
    pub fn choose_none(&mut self) {
        self.candidates.clear();
    }

    pub fn chosen(&self) -> Option<&Candidate> {
        self.chosen.as_ref()
    }

    /// How many parts the disc has, from its table of contents.
    pub fn parts(&self) -> u32 {
        self.get(Subject::Disc, "parts").and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    /// Whether anything in it has no step yet: added by the step it's
    /// leaving.
    pub(crate) fn untagged(&self) -> bool {
        self.facts.iter().any(|f| f.step.is_empty())
            || self.candidates.iter().any(|c| c.step.is_empty() || c.facts.iter().any(|f| f.step.is_empty()))
            || self.chosen.as_ref().is_some_and(|c| c.step.is_empty() || c.facts.iter().any(|f| f.step.is_empty()))
    }

    /// Marks everything without a step as this step's.
    pub(crate) fn tag(&mut self, step: &str) {
        for f in self.facts.iter_mut().filter(|f| f.step.is_empty()) {
            f.step = step.to_owned();
        }
        for c in self.candidates.iter_mut().chain(self.chosen.as_mut()) {
            tag(c, step);
        }
    }

    /// Takes in what another copy of the same item learned: where branches
    /// join, each branch's copy. What both have isn't doubled; a chosen
    /// identity already here stays.
    pub fn merge(&mut self, other: &Context) {
        for f in &other.facts {
            if !self.facts.contains(f) {
                self.facts.push(f.clone());
            }
        }
        for c in &other.candidates {
            if !self.candidates.iter().any(|x| x.step == c.step && x.id == c.id) {
                self.candidates.push(c.clone());
            }
        }
        if self.chosen.is_none() {
            self.chosen = other.chosen.clone();
        }
    }
}

/// Marks a candidate (and its facts) as a step's, unless it says already.
fn tag(c: &mut Candidate, step: &str) {
    if c.step.is_empty() {
        c.step = step.to_owned();
    }
    for f in &mut c.facts {
        if f.step.is_empty() {
            f.step = c.step.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(subject: Subject, key: &str, value: &str, source: &str) -> Fact {
        Fact { subject, key: key.into(), value: value.into(), source: source.into(), step: String::new() }
    }

    fn release(id: &str, title: &str) -> Candidate {
        Candidate {
            id: id.into(),
            source: "MusicBrainz".into(),
            title: "The White Stripes".into(),
            detail: String::new(),
            score: 0.9,
            facts: vec![fact(Subject::Part(1), "title", title, "MusicBrainz")],
            step: String::new(),
        }
    }

    #[test]
    fn the_chosen_identity_is_what_gets_says_and_the_rest_go_no_further() {
        let mut cx = Context::default();
        cx.add(Subject::Part(1), "length", "150", "the disc");
        cx.offer([release("mb-1", "Jimmy the Exploder"), release("mb-2", "Jimmy The Exploder")]);
        assert_eq!(cx.get(Subject::Part(1), "title"), None, "an offer isn't a fact until it's chosen");
        cx.choose(cx.candidates()[0].clone());
        assert_eq!(cx.get(Subject::Part(1), "title"), Some("Jimmy the Exploder"));
        assert_eq!(cx.get(Subject::Part(1), "length"), Some("150"), "what the disc said still holds");
        assert!(cx.candidates().is_empty());
    }

    #[test]
    fn what_a_step_adds_is_marked_its_own_when_it_leaves() {
        let mut cx = Context::default();
        cx.add(Subject::Disc, "parts", "17", "the disc");
        cx.tag("cd");
        cx.offer([release("mb-1", "x")]);
        assert!(cx.untagged());
        cx.tag("mb");
        assert!(!cx.untagged());
        assert_eq!(cx.found_by("cd").count(), 1);
        assert_eq!(cx.offered_by("mb").next().unwrap().facts[0].step, "mb", "a candidate's facts are its offerer's");
        // Kept data from before steps were named still reads.
        let old: Fact = serde_json::from_str(r#"{"subject":"Disc","key":"album","value":"X","source":"CD-Text"}"#).unwrap();
        assert_eq!(old.step, "");
    }

    #[test]
    fn copies_of_one_item_join_without_doubling() {
        let mut disc = Context::default();
        disc.add(Subject::Disc, "offset", "103", "the drive");
        disc.tag("drive");
        let (mut a, mut b) = (disc.clone(), disc.clone());
        a.offer([release("mb-1", "x")]);
        a.tag("mb");
        b.offer([release("ct", "X")]);
        b.tag("cdtext");
        let mut joined = a.clone();
        joined.merge(&b);
        joined.merge(&a);
        assert_eq!(joined.all(Subject::Disc, "offset").count(), 1);
        assert_eq!(joined.candidates().iter().map(|c| c.step.as_str()).collect::<Vec<_>>(), ["mb", "cdtext"]);
    }
}
