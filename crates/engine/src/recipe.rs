//! Recipes as saved, edited and run: a tree of nodes.

use crate::setting::Setting;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub name: String,
    pub root: Node,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Node {
    /// One step. `id` is unique in the recipe; `step` names it in the registry.
    Step {
        id: String,
        step: String,
        /// Overrides the step's name on its tile (*NAS* for a Library step).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// Recipe-scoped parameters that were chosen. Absent means default.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Setting<Value>>,
        /// Items wait before this step until you approve.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        hold: bool,
        /// What hears of it if it fails: a step that takes a message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_fail: Option<Box<Node>>,
        /// What hears of it if it needs you: a question, a hold.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_needs: Option<Box<Node>>,
    },
    /// Left to right: a strip, or a strip of strips.
    Chain { nodes: Vec<Node> },
    /// Side by side. Every branch gets every item; what follows joins them.
    Branches { branches: Vec<Node> },
    /// Several steps acting as one, named.
    Group { name: String, body: Box<Node> },
}

impl Node {
    pub fn step(id: &str, step: &str) -> Node {
        Node::Step { id: id.into(), step: step.into(), label: None, params: BTreeMap::new(), hold: false, on_fail: None, on_needs: None }
    }

    /// What hears of this step if it fails, or needs you.
    pub fn side(&self, s: Side) -> Option<&Node> {
        match (self, s) {
            (Node::Step { on_fail, .. }, Side::Fails) => on_fail.as_deref(),
            (Node::Step { on_needs, .. }, Side::NeedsYou) => on_needs.as_deref(),
            _ => None,
        }
    }

    fn side_slot(&mut self, s: Side) -> Option<&mut Option<Box<Node>>> {
        match (self, s) {
            (Node::Step { on_fail, .. }, Side::Fails) => Some(on_fail),
            (Node::Step { on_needs, .. }, Side::NeedsYou) => Some(on_needs),
            _ => None,
        }
    }

    /// The same step, with `node` hearing of it when `s` happens.
    pub fn on(mut self, s: Side, node: Node) -> Node {
        if let Some(slot) = self.side_slot(s) {
            *slot = Some(Box::new(node));
        }
        self
    }

    /// Every step on the main line, not on a path of its own from a step
    /// that fails or needs you: left to right and top to bottom.
    pub fn line_ids(&self) -> Vec<&str> {
        let mut out = Vec::new();
        fn go<'a>(n: &'a Node, out: &mut Vec<&'a str>) {
            match n {
                Node::Step { id, .. } => out.push(id),
                n => n.kids().iter().for_each(|k| go(k, out)),
            }
        }
        go(self, &mut out);
        out
    }

    /// Sets a recipe-scoped parameter, as chosen.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Node {
        if let Node::Step { params, .. } = &mut self {
            params.insert(key.into(), Setting::chosen(value.into()));
        }
        self
    }

    pub fn labelled(mut self, name: &str) -> Node {
        if let Node::Step { label, .. } = &mut self {
            *label = Some(name.into());
        }
        self
    }

    pub fn held(mut self) -> Node {
        if let Node::Step { hold, .. } = &mut self {
            *hold = true;
        }
        self
    }

    /// Every step node, left to right and top to bottom.
    pub fn steps(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        self.walk(&mut |n| {
            if matches!(n, Node::Step { .. }) {
                out.push(n)
            }
        });
        out
    }

    /// Whether the step `id` hangs off another step's *If it fails* or
    /// *If it needs you* path.
    pub fn on_a_path(&self, id: &str) -> bool {
        let mut found = false;
        self.walk(&mut |n| {
            if let Node::Step { on_fail, on_needs, .. } = n {
                found |= [on_fail, on_needs].into_iter().flatten().any(|p| p.ids().contains(&id));
            }
        });
        found
    }

    fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        match self {
            Node::Step { on_fail, on_needs, .. } => [on_fail, on_needs].into_iter().flatten().for_each(|n| n.walk(f)),
            Node::Chain { nodes } => nodes.iter().for_each(|n| n.walk(f)),
            Node::Branches { branches } => branches.iter().for_each(|n| n.walk(f)),
            Node::Group { body, .. } => body.walk(f),
        }
    }
}

/// When a step's own path is taken, besides its usual one: if it fails,
/// or if it needs you (asks something, holds, waits on a track).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Fails,
    NeedsYou,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Fails, Side::NeedsYou];
}

/// Where a + is: after a step, or where a set of branches comes
/// together. What's added after a join gets everything from every branch.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    /// After the step with this id.
    After(String),
    /// After the branches at this path: from the root, the index of each
    /// part of a chain, each branch, and 0 for a group's body.
    Join(Vec<usize>),
    /// On the step's own path for when it fails or needs you.
    On(String, Side),
}

impl Node {
    fn kids(&self) -> &[Node] {
        match self {
            Node::Step { .. } => &[],
            Node::Chain { nodes: v } | Node::Branches { branches: v } => v,
            Node::Group { body, .. } => std::slice::from_ref(&**body),
        }
    }

    fn kid_mut(&mut self, k: usize) -> Option<&mut Node> {
        match self {
            Node::Step { .. } => None,
            Node::Chain { nodes: v } | Node::Branches { branches: v } => v.get_mut(k),
            Node::Group { body, .. } => (k == 0).then_some(&mut **body),
        }
    }

    /// The node at this path (see [`Spot::Join`]).
    pub fn at(&self, path: &[usize]) -> Option<&Node> {
        path.iter().try_fold(self, |n, &k| n.kids().get(k))
    }

    fn at_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        path.iter().try_fold(self, |n, &k| n.kid_mut(k))
    }

    /// The path to the step `id`.
    pub fn path_of(&self, id: &str) -> Option<Vec<usize>> {
        match self {
            Node::Step { id: x, .. } => (x == id).then(Vec::new),
            n => n.kids().iter().enumerate().find_map(|(k, c)| {
                let mut p = c.path_of(id)?;
                p.insert(0, k);
                Some(p)
            }),
        }
    }

    /// Every set of two or more branches, by path, outermost first. (One
    /// left alone, after the others were removed, is just a line.)
    pub fn joins(&self) -> Vec<Vec<usize>> {
        fn go(n: &Node, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
            if matches!(n, Node::Branches { branches } if branches.len() > 1) {
                out.push(path.clone());
            }
            for (k, c) in n.kids().iter().enumerate() {
                path.push(k);
                go(c, path, out);
                path.pop();
            }
        }
        let mut out = vec![];
        go(self, &mut vec![], &mut out);
        out
    }

    /// The steps that end this node: its last step, or the last of each
    /// branch.
    pub fn ends(&self) -> Vec<&str> {
        match self {
            Node::Step { id, .. } => vec![id.as_str()],
            Node::Chain { nodes } => nodes.last().map(Node::ends).unwrap_or_default(),
            Node::Branches { branches } => branches.iter().flat_map(Node::ends).collect(),
            Node::Group { body, .. } => body.ends(),
        }
    }

    /// The steps that start this node: its first step, or the first of
    /// each branch.
    pub fn starts(&self) -> Vec<&str> {
        match self {
            Node::Step { id, .. } => vec![id.as_str()],
            Node::Chain { nodes } => nodes.first().map(Node::starts).unwrap_or_default(),
            Node::Branches { branches } => branches.iter().flat_map(Node::starts).collect(),
            Node::Group { body, .. } => body.starts(),
        }
    }

    /// The path of what a spot follows: the step, or the branches.
    fn spot_path(&self, at: &Spot) -> Option<Vec<usize>> {
        match at {
            Spot::After(id) => self.path_of(id),
            Spot::Join(p) => matches!(self.at(p), Some(Node::Branches { .. })).then(|| p.clone()),
            Spot::On(..) => None,
        }
    }

    /// What comes next on the line after a spot, if anything: going out
    /// of any chain it ends, as far as the chain around it.
    pub fn next_at(&self, at: &Spot) -> Option<&Node> {
        let mut path = self.spot_path(at)?;
        while let Some(k) = path.pop() {
            match self.at(&path)? {
                Node::Chain { nodes } if k + 1 < nodes.len() => return Some(&nodes[k + 1]),
                Node::Chain { .. } | Node::Group { .. } => {}
                _ => return None,
            }
        }
        None
    }

    /// Whether the line splits straight after this spot: a fork, where a
    /// new step goes beside the branches.
    pub fn forks_at(&self, at: &Spot) -> bool {
        let Some(mut path) = self.spot_path(at) else { return false };
        let Some(k) = path.pop() else { return false };
        matches!(self.at(&path), Some(Node::Chain { nodes }) if matches!(nodes.get(k + 1), Some(Node::Branches { .. })))
    }

    /// Puts `new` at a spot: in line, straight after what's there, or
    /// `beside` what follows it on a line of its own. After a join, in
    /// line, it gets everything from every branch, and goes in front of
    /// whatever already followed them. False if there's no such spot.
    pub fn add_at(&mut self, at: &Spot, new: Node, beside: bool) -> bool {
        let path = match at {
            // A path of its own, if it hasn't one yet.
            Spot::On(id, s) => {
                let Some(slot) = self.find_mut(id).and_then(|n| n.side_slot(*s)) else { return false };
                if slot.is_some() {
                    return false;
                }
                *slot = Some(Box::new(new));
                return true;
            }
            Spot::After(id) if beside => return self.add_branch_after(id, new),
            Spot::After(id) => return self.insert_after(id, new),
            Spot::Join(p) => p,
        };
        if !matches!(self.at(path), Some(Node::Branches { .. })) {
            return false;
        }
        let Some((&k, up)) = path.split_last() else {
            // The whole recipe is these branches.
            let me = std::mem::replace(self, Node::Chain { nodes: vec![] });
            *self = Node::Chain { nodes: vec![me, new] };
            return true;
        };
        match self.at_mut(up) {
            Some(Node::Chain { nodes }) => {
                match nodes.get_mut(k + 1) {
                    Some(Node::Branches { branches }) if beside => branches.push(new),
                    Some(_) if beside => {
                        let mut rest: Vec<Node> = nodes.drain(k + 1..).collect();
                        let rest = if rest.len() == 1 { rest.pop().unwrap() } else { Node::Chain { nodes: rest } };
                        nodes.push(Node::Branches { branches: vec![rest, new] });
                    }
                    _ => nodes.insert(k + 1, new),
                }
                true
            }
            // A branch, or a group, that is these branches: a chain of them
            // and the new step.
            Some(parent) => {
                let slot = parent.kid_mut(k).unwrap();
                let me = std::mem::replace(slot, Node::Chain { nodes: vec![] });
                *slot = Node::Chain { nodes: vec![me, new] };
                true
            }
            None => false,
        }
    }
}

impl Recipe {
    /// Brings a recipe saved by an older version up to date: a step that
    /// was marked to hold becomes a Hold step in front of it. Gives whether
    /// anything changed.
    pub fn upgrade(&mut self) -> bool {
        fn go(n: &mut Node) -> bool {
            match n {
                Node::Step { id, hold, .. } if *hold => {
                    *hold = false;
                    let hold_id = format!("{id}_hold");
                    let me = std::mem::replace(n, Node::Chain { nodes: vec![] });
                    *n = Node::Chain { nodes: vec![Node::step(&hold_id, "hold"), me] };
                    true
                }
                Node::Step { .. } => false,
                Node::Chain { nodes } => nodes.iter_mut().fold(false, |c, n| go(n) | c),
                Node::Branches { branches } => branches.iter_mut().fold(false, |c, n| go(n) | c),
                Node::Group { body, .. } => go(body),
            }
        }
        go(&mut self.root)
    }
}

impl Node {
    /// Every step's id, left to right and top to bottom.
    pub fn ids(&self) -> Vec<&str> {
        self.steps()
            .into_iter()
            .filter_map(|n| match n {
                Node::Step { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The step node with this id.
    pub fn find_mut(&mut self, id: &str) -> Option<&mut Node> {
        match self {
            Node::Step { id: x, .. } if x == id => Some(self),
            Node::Step { on_fail, on_needs, .. } => [on_fail, on_needs].into_iter().flatten().find_map(|n| n.find_mut(id)),
            Node::Chain { nodes } => nodes.iter_mut().find_map(|n| n.find_mut(id)),
            Node::Branches { branches } => branches.iter_mut().find_map(|n| n.find_mut(id)),
            Node::Group { body, .. } => body.find_mut(id),
        }
    }

    /// Puts `new` straight after the step `id`, in the same strip: into its
    /// chain, or, where the step stands alone (the root, a branch), into a
    /// new chain of the two. False if there's no such step.
    pub fn insert_after(&mut self, id: &str, new: Node) -> bool {
        fn ins(node: &mut Node, id: &str, slot: &mut Option<Node>) -> bool {
            match node {
                Node::Step { id: x, .. } if x == id => {
                    let me = std::mem::replace(node, Node::Chain { nodes: vec![] });
                    *node = Node::Chain { nodes: vec![me, slot.take().unwrap()] };
                    true
                }
                Node::Step { .. } => false,
                Node::Chain { nodes } => {
                    if let Some(i) = nodes.iter().position(|n| matches!(n, Node::Step { id: x, .. } if x == id)) {
                        nodes.insert(i + 1, slot.take().unwrap());
                        return true;
                    }
                    nodes.iter_mut().any(|n| ins(n, id, slot))
                }
                Node::Branches { branches } => branches.iter_mut().any(|n| ins(n, id, slot)),
                Node::Group { body, .. } => ins(body, id, slot),
            }
        }
        ins(self, id, &mut Some(new))
    }

    /// Puts `new` on a line of its own beside what follows the step `id`.
    /// If branches follow it, `new` joins them; otherwise the rest of its
    /// chain becomes one branch and `new` another. With nothing after it,
    /// that's the same as putting `new` straight after it. False if
    /// there's no such step.
    pub fn add_branch_after(&mut self, id: &str, new: Node) -> bool {
        /// Where `id` was: not here; placed; or last in a chain, so what
        /// follows it is whatever follows that chain, one level up.
        enum At {
            No,
            Done,
            End,
        }
        fn add(node: &mut Node, id: &str, slot: &mut Option<Node>) -> At {
            match node {
                Node::Step { id: x, .. } if x == id => At::End,
                Node::Step { .. } => At::No,
                Node::Chain { nodes } => {
                    // The step itself, or a chain inside this one ending with it.
                    let mut found = None;
                    for (k, n) in nodes.iter_mut().enumerate() {
                        match add(n, id, slot) {
                            At::No => {}
                            At::Done => return At::Done,
                            At::End => {
                                found = Some(k);
                                break;
                            }
                        }
                    }
                    let Some(k) = found else { return At::No };
                    if k + 1 == nodes.len() {
                        // Nothing follows it here: let the chain around decide.
                        return At::End;
                    }
                    let new = slot.take().unwrap();
                    match nodes.get_mut(k + 1) {
                        Some(Node::Branches { branches }) => branches.push(new),
                        _ => {
                            let mut rest: Vec<Node> = nodes.drain(k + 1..).collect();
                            let rest = if rest.len() == 1 { rest.pop().unwrap() } else { Node::Chain { nodes: rest } };
                            nodes.push(Node::Branches { branches: vec![rest, new] });
                        }
                    }
                    At::Done
                }
                // Last on a branch: nothing follows it within the branch, so
                // the new branch starts beside it, after what feeds it.
                Node::Branches { branches } => {
                    for b in branches.iter_mut() {
                        match add(b, id, slot) {
                            At::No => {}
                            At::Done => return At::Done,
                            At::End => {
                                let me = std::mem::replace(b, Node::Chain { nodes: vec![] });
                                *b = Node::Chain { nodes: vec![me, slot.take().unwrap()] };
                                return At::Done;
                            }
                        }
                    }
                    At::No
                }
                Node::Group { body, .. } => add(body, id, slot),
            }
        }
        let mut slot = Some(new);
        match add(self, id, &mut slot) {
            At::Done => true,
            At::No => false,
            // Last in the whole recipe: it simply follows.
            At::End => {
                let new = slot.take().unwrap();
                match self {
                    Node::Chain { nodes } => nodes.push(new),
                    other => {
                        let me = std::mem::replace(other, Node::Chain { nodes: vec![] });
                        *other = Node::Chain { nodes: vec![me, new] };
                    }
                }
                true
            }
        }
    }

    /// Takes the step `id` out, and any chain, branches or group left empty
    /// by it. False if there's no such step, or it's the only one.
    pub fn remove(&mut self, id: &str) -> bool {
        fn rm(node: &mut Node, id: &str) -> bool {
            let hit = |n: &Node| matches!(n, Node::Step { id: x, .. } if x == id);
            match node {
                // A step on a path of its own: that path goes with it.
                Node::Step { on_fail, on_needs, .. } => [on_fail, on_needs].into_iter().any(|slot| {
                    let Some(n) = slot else { return false };
                    if hit(n) {
                        *slot = None;
                        return true;
                    }
                    let found = rm(n, id);
                    if n.is_empty() {
                        *slot = None;
                    }
                    found
                }),
                Node::Chain { nodes: v } | Node::Branches { branches: v } => {
                    if let Some(i) = v.iter().position(hit) {
                        v.remove(i);
                        return true;
                    }
                    let found = v.iter_mut().any(|n| rm(n, id));
                    v.retain(|n| !n.is_empty());
                    found
                }
                Node::Group { body, .. } => rm(body, id),
            }
        }
        if self.ids().len() <= 1 {
            return false;
        }
        rm(self, id)
    }

    /// Takes out the step `id`, and then, one at a time, only what
    /// downstream no longer fits without it, until the recipe checks
    /// again: what still makes sense stays joined up. A step the registry
    /// doesn't know passes its items on as they are, so it never causes
    /// more to go. Gives the ids taken out, the step first; empty, and the
    /// tree unchanged, if nothing would be left.
    pub fn remove_with_downstream(&mut self, id: &str, reg: &crate::step::Registry) -> Vec<String> {
        let before: Vec<String> = self.ids().into_iter().map(str::to_owned).collect();
        // Just the step; then, one at a time, only what no longer makes
        // sense without it. Taking Cover art out from between Choose and
        // Rip leaves Rip, which still fits; taking Rip out takes the
        // encoders, which then have nothing to take.
        let mut work = self.clone();
        if !work.remove(id) {
            return vec![];
        }
        // Until it checks; if that would take everything, nothing goes.
        while let Some(bad) = crate::check::first_misfit(&work, reg) {
            if !work.remove(&bad) {
                return vec![];
            }
        }
        let after: Vec<&str> = work.ids();
        if after.is_empty() {
            return vec![];
        }
        let mut gone: Vec<String> = before.iter().filter(|b| !after.contains(&b.as_str())).cloned().collect();
        // The step asked for first.
        if let Some(k) = gone.iter().position(|g| g == id) {
            let first = gone.remove(k);
            gone.insert(0, first);
        }
        *self = work;
        gone
    }

    fn is_empty(&self) -> bool {
        match self {
            Node::Step { .. } => false,
            Node::Chain { nodes: v } | Node::Branches { branches: v } => v.iter().all(Node::is_empty),
            Node::Group { body, .. } => body.is_empty(),
        }
    }
}

/// A chain of nodes.
pub fn chain(nodes: impl IntoIterator<Item = Node>) -> Node {
    Node::Chain { nodes: nodes.into_iter().collect() }
}

/// Branches side by side.
pub fn branches(branches: impl IntoIterator<Item = Node>) -> Node {
    Node::Branches { branches: branches.into_iter().collect() }
}

pub fn group(name: &str, body: Node) -> Node {
    Node::Group { name: name.into(), body: Box::new(body) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipe_round_trips_through_json() {
        let r = Recipe {
            name: "CDs".into(),
            root: chain([Node::step("drive", "drive").with("drive", "Blu-ray drive"), branches([Node::step("mb", "musicbrainz"), Node::step("ct", "cd-text")]), Node::step("lib", "library").held()]),
        };
        let json = serde_json::to_string_pretty(&r).unwrap();
        let back: Recipe = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
        assert_eq!(back.root.steps().len(), 4);
    }

    fn cd() -> Node {
        chain([
            Node::step("drive", "drive"),
            Node::step("cd", "audio-cd"),
            branches([Node::step("mb", "musicbrainz"), Node::step("ct", "cd-text")]),
            Node::step("choose", "choose"),
            group("phone", chain([Node::step("opus", "opus"), Node::step("ph", "library")])),
        ])
    }

    #[test]
    fn inserting_after_a_step_in_a_chain_extends_its_strip() {
        let mut r = cd();
        assert!(r.insert_after("cd", Node::step("x", "musicbrainz")));
        assert_eq!(r.ids(), ["drive", "cd", "x", "mb", "ct", "choose", "opus", "ph"]);
        let Node::Chain { nodes } = &r else { panic!() };
        assert!(matches!(&nodes[2], Node::Step { id, .. } if id == "x"), "straight after, before the branches");
    }

    #[test]
    fn inserting_after_a_lone_step_makes_a_chain() {
        let mut r = Node::step("drive", "drive");
        assert!(r.insert_after("drive", Node::step("cd", "audio-cd")));
        assert_eq!(r, chain([Node::step("drive", "drive"), Node::step("cd", "audio-cd")]));
        // A bare step in branches becomes a chain inside that branch.
        let mut r = cd();
        assert!(r.insert_after("mb", Node::step("x", "discogs")));
        let Node::Chain { nodes } = &r else { panic!() };
        let Node::Branches { branches: b } = &nodes[2] else { panic!() };
        assert_eq!(b[0], chain([Node::step("mb", "musicbrainz"), Node::step("x", "discogs")]));
        assert_eq!(b[1], Node::step("ct", "cd-text"));
    }

    #[test]
    fn inserting_inside_a_group_and_at_the_end() {
        let mut r = cd();
        assert!(r.insert_after("opus", Node::step("rg", "replaygain")));
        assert!(r.insert_after("ph", Node::step("tell", "tell")));
        assert_eq!(r.ids(), ["drive", "cd", "mb", "ct", "choose", "opus", "rg", "ph", "tell"]);
        let Node::Chain { nodes } = &r else { panic!() };
        assert!(matches!(&nodes[4], Node::Group { .. }), "both stay in the group: {:?}", nodes[4]);
        assert!(!r.insert_after("nope", Node::step("y", "tell")));
    }

    #[test]
    fn a_new_branch_joins_the_branches_that_follow() {
        let mut r = cd();
        assert!(r.add_branch_after("cd", Node::step("dg", "discogs")));
        let Node::Chain { nodes } = &r else { panic!() };
        let Node::Branches { branches } = &nodes[2] else { panic!("{:?}", nodes[2]) };
        assert_eq!(branches.len(), 3);
        assert_eq!(branches[2], Node::step("dg", "discogs"));
        assert_eq!(nodes.len(), 5, "nothing else moved");
    }

    #[test]
    fn a_new_branch_splits_the_rest_of_the_line() {
        let mut r = chain([Node::step("rip", "rip"), Node::step("flac", "flac"), Node::step("lib", "library")]);
        assert!(r.add_branch_after("rip", Node::step("next", "next-disc")));
        assert_eq!(r, chain([Node::step("rip", "rip"), branches([chain([Node::step("flac", "flac"), Node::step("lib", "library")]), Node::step("next", "next-disc")]),]));
        // One step after: that step is the other branch.
        let mut r = chain([Node::step("a", "rip"), Node::step("b", "flac")]);
        assert!(r.add_branch_after("a", Node::step("c", "mp3")));
        assert_eq!(r, chain([Node::step("a", "rip"), branches([Node::step("b", "flac"), Node::step("c", "mp3")])]));
        // Nothing after: straight after it.
        let mut r = cd();
        assert!(r.add_branch_after("ph", Node::step("t", "tell")));
        assert_eq!(*r.ids().last().unwrap(), "t");
        assert!(!r.add_branch_after("nope", Node::step("x", "tell")));
    }

    #[test]
    fn removing_a_step_tidies_what_it_leaves_empty() {
        let mut r = cd();
        assert!(r.remove("ct"));
        assert!(r.remove("mb"));
        assert_eq!(r.ids(), ["drive", "cd", "choose", "opus", "ph"]);
        let Node::Chain { nodes } = &r else { panic!() };
        assert_eq!(nodes.len(), 4, "the empty branches went: {nodes:?}");
        assert!(r.remove("opus") && r.remove("ph"));
        assert_eq!(nodes_len(&r), 3, "and the empty group");
        assert!(!r.remove("nope"));
        let mut one = Node::step("drive", "drive");
        assert!(!one.remove("drive"), "never the last step");
    }

    fn nodes_len(n: &Node) -> usize {
        match n {
            Node::Chain { nodes } => nodes.len(),
            _ => 1,
        }
    }
}
