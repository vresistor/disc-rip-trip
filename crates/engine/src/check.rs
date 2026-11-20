//! The check: does each step get what it needs at its point in the flow?
//! Asked "what fits here?", the same check is the + picker, so it only
//! offers what follows from where you are.
//!
//! What a point in the flow has is a [`Flow`]: the kind of item arriving
//! (a disc nothing has looked at, an audio CD, tracks, files…) and what the
//! steps before it have found (candidates, titles, the chosen release…).
//! A step fits when it takes that kind and, if it needs something found
//! first, any one of those things has been. So MusicBrainz can't follow a
//! Drive (it needs an audio CD, which the Audio CD step says it is), and
//! Cover art can't come before Choose (it needs the release).

use crate::item::ItemKind;
use crate::recipe::{Node, Spot};
use crate::step::{Registry, StepInfo};
use std::collections::BTreeSet;
use std::fmt;

/// What arrives at a point in a recipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    pub kind: ItemKind,
    pub found: BTreeSet<&'static str>,
    /// The step kinds already on the way here: most steps make sense once
    /// on a path (Choose after Choose doesn't).
    pub seen: BTreeSet<&'static str>,
    /// The step just before, when there's exactly one.
    pub last: Option<&'static str>,
}

impl Flow {
    pub fn start() -> Flow {
        Flow { kind: ItemKind::Nothing, found: BTreeSet::new(), seen: BTreeSet::new(), last: None }
    }

    /// Whether a step fits here.
    pub fn fits(&self, info: &StepInfo) -> bool {
        info.input.accepts(self.kind) && self.unmet(info).is_none() && self.repeated(info).is_none() && self.settled(info).is_none() && self.not_right_after(info).is_none()
    }

    /// The steps this one must come straight after, if what's just before
    /// isn't one of them.
    pub fn not_right_after<'i>(&self, info: &'i StepInfo) -> Option<&'i [&'static str]> {
        let ok = info.right_after.is_empty() || self.last.is_some_and(|l| info.right_after.contains(&l));
        (!ok).then_some(info.right_after.as_slice())
    }

    /// The step is already on this path, and isn't one that may repeat.
    pub fn repeated(&self, info: &StepInfo) -> Option<&'static str> {
        (!info.repeatable && self.seen.contains(info.kind)).then_some(info.kind)
    }

    /// Something the step only makes sense before is already found (a
    /// lookup that identifies the disc, once Choose has settled it).
    pub fn settled<'i>(&self, info: &'i StepInfo) -> Option<&'i str> {
        info.unless.iter().find(|f| self.found.contains(*f)).copied()
    }

    /// The first of a step's needs that nothing before it has found: each
    /// is met by any one of its facts.
    pub fn unmet<'i>(&self, info: &'i StepInfo) -> Option<&'i [&'static str]> {
        info.needs.iter().find(|any_of| !any_of.iter().any(|n| self.found.contains(n))).map(Vec::as_slice)
    }

    /// What follows a step that fits here.
    pub fn after(&self, info: &StepInfo) -> Flow {
        let mut found = self.found.clone();
        found.extend(info.gives.iter().copied());
        let mut seen = self.seen.clone();
        seen.insert(info.kind);
        Flow { kind: if info.passes_through { self.kind } else { info.output }, found, seen, last: Some(info.kind) }
    }

    /// Where branches join: the join waits for every branch, so what any
    /// of them found is known after it; branches giving different kinds
    /// join into something that takes anything, or any file if they are
    /// all files.
    fn join(outs: &[Flow]) -> Flow {
        let kind = if outs.iter().all(|f| f.kind == outs[0].kind) {
            outs[0].kind
        } else if outs.iter().all(|f| f.kind.is_file()) {
            ItemKind::AnyFile
        } else {
            ItemKind::Any
        };
        Flow { kind, found: outs.iter().flat_map(|f| f.found.iter().copied()).collect(), seen: outs.iter().flat_map(|f| f.seen.iter().copied()).collect(), last: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CheckError {
    UnknownStep {
        id: String,
        step: String,
    },
    Mismatch {
        id: String,
        takes: ItemKind,
        given: ItemKind,
    },
    /// Needs something found first that nothing before it finds.
    Needs {
        id: String,
        any_of: Vec<&'static str>,
    },
    /// The same step is already on this path.
    Repeated {
        id: String,
        step: &'static str,
    },
    /// Comes after what it only makes sense before.
    Settled {
        id: String,
        fact: String,
    },
    /// Not straight after the step whose item it looks up.
    NotRightAfter {
        id: String,
        after: Vec<&'static str>,
    },
    Empty,
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CheckError::UnknownStep { id, step } => write!(f, "{id}: no step called {step}"),
            CheckError::Mismatch { id, takes, given } => write!(f, "{id} takes {takes:?} but is given {given:?}"),
            CheckError::Needs { id, any_of } => write!(f, "{id} needs {} found before it", any_of.join(" or ")),
            CheckError::Repeated { id, step } => write!(f, "{id}: {step} is already earlier on this path"),
            CheckError::Settled { id, fact } => write!(f, "{id} only makes sense before {fact} is found"),
            CheckError::NotRightAfter { id, after } => write!(f, "{id} goes straight after {}", after.join(" or ")),
            CheckError::Empty => write!(f, "an empty chain or set of branches"),
        }
    }
}

impl std::error::Error for CheckError {}

/// Checks a recipe from its first step, which is given nothing.
pub fn check(root: &Node, reg: &Registry) -> Result<ItemKind, CheckError> {
    walk(root, Flow::start(), reg).map(|f| f.kind)
}

/// The steps that can follow a point with this flow.
pub fn fits_flow<'a>(flow: &Flow, reg: &'a Registry) -> Vec<&'a StepInfo> {
    let mut v: Vec<_> = reg.infos().filter(|i| i.input != ItemKind::Nothing && flow.fits(i)).collect();
    v.sort_by_key(|i| (i.group as u8, i.name));
    v
}

/// The steps that can follow something giving `given`, whatever was found.
pub fn fits(given: ItemKind, reg: &Registry) -> Vec<&StepInfo> {
    let mut v: Vec<_> = reg.infos().filter(|i| i.input.accepts(given) && i.input != ItemKind::Nothing).collect();
    v.sort_by_key(|i| (i.group as u8, i.name));
    v
}

/// One step a + offers, and how clicking it places it.
#[derive(Clone, Copy, Debug)]
pub struct Offer<'a> {
    pub info: &'a StepInfo,
    /// Where a click puts it: beside what follows, on a line of its own,
    /// where the line already splits there (or where only that fits); in
    /// line anywhere else.
    pub beside: bool,
    /// It also fits the other way, and that makes a different recipe: the
    /// + offers that too.
    pub other: bool,
}

impl Offer<'_> {
    /// The ways this offer can be placed: the click's, then the other.
    pub fn ways(&self) -> Vec<bool> {
        if self.other {
            vec![self.beside, !self.beside]
        } else {
            vec![self.beside]
        }
    }
}

/// Every step that fits at a spot, with whether it checks in line and
/// beside what follows. A step that fits neither way isn't listed.
pub fn placements<'a>(root: &Node, at: &Spot, reg: &'a Registry) -> Vec<(&'a StepInfo, bool, bool)> {
    trials(root, at, reg).into_iter().map(|(info, a, b)| (info, a.is_some(), b.is_some())).collect()
}

/// What a + at this spot offers: the same for the window as for anything
/// that builds a recipe the way it does.
pub fn offers<'a>(root: &Node, at: &Spot, reg: &'a Registry) -> Vec<Offer<'a>> {
    let fork = root.forks_at(at);
    trials(root, at, reg)
        .into_iter()
        .map(|(info, inline, beside)| {
            let (first, second) = if fork { (beside, inline) } else { (inline, beside) };
            match (first, second) {
                (Some(x), Some(y)) => Offer { info, beside: fork, other: x != y },
                (Some(_), None) => Offer { info, beside: fork, other: false },
                _ => Offer { info, beside: !fork, other: false },
            }
        })
        .collect()
}

/// Each step tried at a spot, in line and beside: the recipe it makes, if
/// that checks. Sorted as the + lists them.
fn trials<'a>(root: &Node, at: &Spot, reg: &'a Registry) -> Vec<(&'a StepInfo, Option<Node>, Option<Node>)> {
    let mut v: Vec<_> = reg
        .infos()
        .filter(|i| i.input != ItemKind::Nothing)
        .filter_map(|info| {
            let trial = |beside| {
                let mut t = root.clone();
                (t.add_at(at, Node::step("__new", info.kind), beside) && check(&t, reg).is_ok()).then_some(t)
            };
            let (a, b) = (trial(false), trial(true));
            (a.is_some() || b.is_some()).then_some((info, a, b))
        })
        .collect();
    v.sort_by_key(|(i, _, _)| (i.group as u8, i.name));
    v
}

/// What arrives after the step `id`: what a step inserted straight after
/// it is fed. `None` if there's no such step, or the tree doesn't check up
/// to it.
pub fn flow_after(root: &Node, id: &str, reg: &Registry) -> Option<Flow> {
    fn find(node: &Node, given: Flow, id: &str, reg: &Registry) -> Result<Flow, Option<Flow>> {
        match node {
            Node::Step { id: x, step, .. } => {
                let out = reg.info(step).filter(|i| given.fits(i)).map(|i| given.after(&i));
                if x == id {
                    return Err(out);
                }
                out.ok_or(None)
            }
            Node::Chain { nodes } => nodes.iter().try_fold(given, |f, n| find(n, f, id, reg)),
            Node::Branches { branches } => {
                let outs = branches.iter().map(|b| find(b, given.clone(), id, reg)).collect::<Result<Vec<_>, _>>()?;
                if outs.is_empty() {
                    return Err(None);
                }
                Ok(Flow::join(&outs))
            }
            Node::Group { body, .. } => find(body, given, id, reg),
        }
    }
    find(root, Flow::start(), id, reg).err().flatten()
}

/// What arrives at each step, by id, as far as the tree checks: what a
/// setting that follows its step's input goes by.
pub fn flows_in(root: &Node, reg: &Registry) -> std::collections::HashMap<String, Flow> {
    fn go(node: &Node, given: Flow, reg: &Registry, out: &mut std::collections::HashMap<String, Flow>) -> Option<Flow> {
        match node {
            Node::Step { id, step, .. } => {
                let after = reg.info(step).filter(|i| given.fits(i)).map(|i| given.after(&i));
                out.insert(id.clone(), given);
                after
            }
            Node::Chain { nodes } => nodes.iter().try_fold(given, |f, n| go(n, f, reg, out)),
            Node::Branches { branches } => {
                let outs: Vec<Option<Flow>> = branches.iter().map(|b| go(b, given.clone(), reg, out)).collect();
                let outs = outs.into_iter().collect::<Option<Vec<_>>>()?;
                (!outs.is_empty()).then(|| Flow::join(&outs))
            }
            Node::Group { body, .. } => go(body, given, reg, out),
        }
    }
    let mut out = std::collections::HashMap::new();
    go(root, Flow::start(), reg, &mut out);
    out
}

/// The kind of item arriving after the step `id`.
pub fn kind_after(root: &Node, id: &str, reg: &Registry) -> Option<ItemKind> {
    flow_after(root, id, reg).map(|f| f.kind)
}

/// The first step that doesn't fit its point in the flow, by id. What a
/// step the registry doesn't know gives is unknown, and anything may follow
/// it, so it doesn't make a misfit after it: a step after it that passes
/// on what it's given passes on the unknown, and what it found is unknown,
/// so no step after it is a misfit for its needs. Empty chains and branches
/// don't count.
pub fn first_misfit(root: &Node, reg: &Registry) -> Option<String> {
    // The flow, `None` if its kind is unknown; and whether an unknown step
    // came before, so what was found is unknown too.
    type At = (Option<Flow>, bool);
    fn go(node: &Node, (given, unknown): At, reg: &Registry) -> Result<At, String> {
        match node {
            Node::Step { id, step, .. } => match (reg.info(step), given) {
                (None, _) => Ok((None, true)),
                (Some(info), Some(g)) if !info.input.accepts(g.kind) || (!unknown && !g.fits(&info)) => Err(id.clone()),
                (Some(info), Some(g)) => Ok((Some(g.after(&info)), unknown)),
                (Some(info), None) if info.passes_through => Ok((None, unknown)),
                (Some(info), None) => Ok((Some(Flow::start().after(&info)), unknown)),
            },
            Node::Chain { nodes } => nodes.iter().try_fold((given, unknown), |f, n| go(n, f, reg)),
            Node::Branches { branches } => {
                let outs = branches.iter().map(|b| go(b, (given.clone(), unknown), reg)).collect::<Result<Vec<_>, _>>()?;
                if outs.is_empty() {
                    return Ok((given, unknown));
                }
                let unknown = outs.iter().any(|o| o.1);
                if outs.iter().any(|o| o.0.is_none()) {
                    return Ok((None, unknown));
                }
                Ok((Some(Flow::join(&outs.into_iter().filter_map(|o| o.0).collect::<Vec<_>>())), unknown))
            }
            Node::Group { body, .. } => go(body, (given, unknown), reg),
        }
    }
    go(root, (Some(Flow::start()), false), reg).err()
}

fn walk(node: &Node, given: Flow, reg: &Registry) -> Result<Flow, CheckError> {
    match node {
        Node::Step { id, step, .. } => {
            let info = reg.info(step).ok_or_else(|| CheckError::UnknownStep { id: id.clone(), step: step.clone() })?;
            if !info.input.accepts(given.kind) {
                return Err(CheckError::Mismatch { id: id.clone(), takes: info.input, given: given.kind });
            }
            if let Some(any_of) = given.unmet(&info) {
                return Err(CheckError::Needs { id: id.clone(), any_of: any_of.to_vec() });
            }
            if let Some(step) = given.repeated(&info) {
                return Err(CheckError::Repeated { id: id.clone(), step });
            }
            if let Some(fact) = given.settled(&info) {
                return Err(CheckError::Settled { id: id.clone(), fact: fact.to_owned() });
            }
            if let Some(after) = given.not_right_after(&info) {
                return Err(CheckError::NotRightAfter { id: id.clone(), after: after.to_vec() });
            }
            Ok(given.after(&info))
        }
        Node::Chain { nodes } => {
            if nodes.is_empty() {
                return Err(CheckError::Empty);
            }
            nodes.iter().try_fold(given, |f, n| walk(n, f, reg))
        }
        Node::Branches { branches } => {
            let outs = branches.iter().map(|b| walk(b, given.clone(), reg)).collect::<Result<Vec<_>, _>>()?;
            if outs.is_empty() {
                return Err(CheckError::Empty);
            }
            Ok(Flow::join(&outs))
        }
        Node::Group { body, .. } => walk(body, given, reg),
    }
}
