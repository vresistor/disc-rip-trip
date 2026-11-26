//! Every built-in recipe can be made in the editor: from its first step
//! alone, one + at a time, each step one the + at that spot offers, placed
//! the way a click there places it. The window's + menu lists exactly
//! these offers (`trip_engine::offers`), so this can't drift from it.

use trip_engine::recipe::{branches, chain};
use trip_engine::{check, offers, Node, Recipe, Registry, Spot};
use trip_steps::{real, recipes, simulated};

fn registries() -> [(&'static str, Registry); 2] {
    [("simulated", simulated(None)), ("real", real())]
}

/// Chains inside chains spliced in, and a chain of one its one part: the
/// same recipe however the editor nested it.
fn normal(n: &Node) -> Node {
    match n {
        Node::Chain { nodes } => {
            let mut flat = vec![];
            for n in nodes.iter().map(normal) {
                match n {
                    Node::Chain { nodes } => flat.extend(nodes),
                    n => flat.push(n),
                }
            }
            if flat.len() == 1 {
                flat.pop().unwrap()
            } else {
                Node::Chain { nodes: flat }
            }
        }
        Node::Branches { branches } => Node::Branches { branches: branches.iter().map(normal).collect() },
        Node::Group { name, body } => Node::Group { name: name.clone(), body: Box::new(normal(body)) },
        step => step.clone(),
    }
}

/// A step as the + adds it: its kind and nothing chosen yet.
fn bare(n: &Node) -> Node {
    match n {
        Node::Step { id, step, .. } => Node::step(id, step),
        _ => unreachable!(),
    }
}

struct Build<'a> {
    reg: &'a Registry,
    tree: Node,
    /// Each op, for the failure message.
    ops: Vec<String>,
}

impl Build<'_> {
    /// Adds one step at a spot, `beside` or in line, as a click on a +
    /// there would: it must be offered, one of the ways the offer places
    /// it must be this one (or make the same recipe), and the result must
    /// check.
    fn add(&mut self, at: &Spot, step: &Node, beside: bool) {
        let Node::Step { id, step: kind, .. } = step else { unreachable!() };
        let offered = offers(&self.tree, at, self.reg);
        let o = offered
            .iter()
            .find(|o| o.info.kind == kind)
            .unwrap_or_else(|| panic!("{kind} isn't offered at {at:?}; offered: {:?}\nso far: {:#?}", offered.iter().map(|o| o.info.kind).collect::<Vec<_>>(), self.ops));
        let mut want = self.tree.clone();
        assert!(want.add_at(at, bare(step), beside), "no spot {at:?}");
        let ok = o.ways().into_iter().any(|way| {
            let mut t = self.tree.clone();
            t.add_at(at, bare(step), way) && t == want
        });
        assert!(ok, "{kind} at {at:?}: the + places it {}, not {}\nso far: {:#?}", if o.beside { "beside" } else { "in line" }, if beside { "beside" } else { "in line" }, self.ops);
        check(&want, self.reg).unwrap_or_else(|e| panic!("{kind} at {at:?}: {e}"));
        self.ops.push(format!("{id} ({kind}) {} {at:?}", if beside { "beside what follows" } else { "in line after" }));
        self.tree = want;
        // What the sidebar sets: its settings and its name.
        if let (Some(Node::Step { params, label, .. }), Node::Step { params: p, label: l, .. }) = (self.tree.find_mut(id), step) {
            *params = p.clone();
            *label = l.clone();
        }
    }

    /// Builds `node` starting at `at`, and gives the spot after it.
    fn emit(&mut self, node: &Node, at: &Spot, beside: bool) -> Spot {
        match node {
            Node::Step { id, .. } => {
                self.add(at, node, beside);
                Spot::After(id.clone())
            }
            Node::Chain { nodes } => {
                let mut at = self.emit(&nodes[0], at, beside);
                for n in &nodes[1..] {
                    at = self.emit(n, &at, false);
                }
                at
            }
            Node::Branches { branches } => {
                // The first in line, each other beside it from the same spot.
                self.emit(&branches[0], at, beside);
                for b in &branches[1..] {
                    self.emit(b, at, true);
                }
                // The branches just made: around the first one's first step,
                // holding the same steps.
                let want: Vec<&str> = node.ids();
                let first = self.tree.path_of(want[0]).unwrap();
                let path = (0..first.len()).rev().map(|k| first[..k].to_vec()).find(|p| matches!(self.tree.at(p), Some(n @ Node::Branches { .. }) if n.ids() == want)).expect("the branches");
                Spot::Join(path)
            }
            Node::Group { name, .. } => panic!("the editor can't make a group yet ({name})"),
        }
    }
}

/// Rebuilds a recipe by hand from its first step.
fn by_hand(r: &Recipe, reg: &Registry) -> Node {
    let target = normal(&r.root);
    let Node::Chain { nodes } = &target else { panic!("{}: one step", r.name) };
    let Node::Step { id, .. } = &nodes[0] else { panic!("{}: starts with a step", r.name) };
    let mut b = Build { reg, tree: nodes[0].clone(), ops: vec![] };
    let mut at = Spot::After(id.clone());
    for n in &nodes[1..] {
        at = b.emit(n, &at, false);
    }
    assert_eq!(normal(&b.tree), target, "{}: built by hand as {:#?}", r.name, b.ops);
    b.tree
}

#[test]
fn every_built_in_recipe_can_be_made_with_the_plus() {
    for (which, reg) in registries() {
        for r in recipes::all() {
            let made = by_hand(&r, &reg);
            check(&made, &reg).unwrap_or_else(|e| panic!("{which}: {}: {e}", r.name));
        }
    }
}

/// The lookups side by side, joined at Choose; the copies side by side,
/// joined at Notify; and a join with nothing after it yet.
#[test]
fn a_step_after_branches_gets_everything_from_all_of_them() {
    let reg = simulated(None);
    let mut t = chain([Node::step("d", "drive"), Node::step("cd", "audio-cd"), branches([Node::step("mb", "musicbrainz"), Node::step("ct", "cd-text")])]);
    let join = Spot::Join(vec![2]);
    let names: Vec<&str> = offers(&t, &join, &reg).iter().map(|o| o.info.kind).collect();
    assert!(names.contains(&"choose"), "{names:?}");
    // A lookup reads the disc, not the join; nothing needing the release yet.
    assert!(!names.contains(&"discogs") && !names.contains(&"cover-art"), "{names:?}");
    assert!(t.add_at(&join, Node::step("ch", "choose"), false));
    assert_eq!(t, chain([Node::step("d", "drive"), Node::step("cd", "audio-cd"), branches([Node::step("mb", "musicbrainz"), Node::step("ct", "cd-text")]), Node::step("ch", "choose")]));
    // Something already follows: the new step goes between.
    let mut u = chain([Node::step("a", "drive"), branches([Node::step("b", "data-disc"), Node::step("c", "audio-cd")]), Node::step("z", "tell")]);
    assert!(u.add_at(&Spot::Join(vec![1]), Node::step("h", "hold"), false));
    assert_eq!(u, chain([Node::step("a", "drive"), branches([Node::step("b", "data-disc"), Node::step("c", "audio-cd")]), Node::step("h", "hold"), Node::step("z", "tell")]));
    // A whole recipe of branches, and branches in a branch: a chain of them
    // and the new step.
    let mut v = branches([Node::step("a", "drive"), Node::step("b", "drive")]);
    assert!(v.add_at(&Spot::Join(vec![]), Node::step("t", "tell"), false));
    assert_eq!(v, chain([branches([Node::step("a", "drive"), Node::step("b", "drive")]), Node::step("t", "tell")]));
    // Not a set of branches there: no spot.
    assert!(!v.clone().add_at(&Spot::Join(vec![1]), Node::step("x", "tell"), false));
}

/// Whatever runs side by side in a built-in recipe can be followed by
/// something that waits for all of it: each set of branches has a + that
/// offers a step. In Games, a Notify once the CHD is filed and the tray
/// is open.
#[test]
fn every_set_of_branches_in_a_built_in_recipe_can_be_joined() {
    for (which, reg) in registries() {
        for r in recipes::all() {
            for path in r.root.joins() {
                let offered: Vec<&str> = offers(&r.root, &Spot::Join(path.clone()), &reg).iter().map(|o| o.info.kind).collect();
                assert!(!offered.is_empty(), "{which}: {}: nothing after the branches at {path:?}", r.name);
            }
        }
    }
    let reg = simulated(None);
    let games = recipes::games();
    let end = Spot::Join(games.root.joins()[0].clone());
    let mut t = games.root.clone();
    assert!(offers(&t, &end, &reg).iter().any(|o| o.info.kind == "tell" && !o.beside));
    assert!(t.add_at(&end, Node::step("t", "tell"), false));
    check(&t, &reg).unwrap();
}
