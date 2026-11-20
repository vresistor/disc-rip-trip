//! A + in the middle of a line offers what fits beside what follows, too:
//! after Rip → FLAC → Library, another encoder goes on a branch from Rip.

use drt_engine::recipe::chain;
use drt_engine::{check, Node};
use drt_steps::{real, simulated};

fn rip_into_flac() -> Node {
    chain([
        Node::step("d", "drive"),
        Node::step("cd", "audio-cd"),
        Node::step("mb", "musicbrainz"),
        Node::step("ch", "choose"),
        Node::step("r", "rip"),
        Node::step("f", "flac"),
        Node::step("l", "folder"),
    ])
}

#[test]
fn a_second_encoder_goes_beside_the_first() {
    for reg in [simulated(None), real()] {
        let root = rip_into_flac();
        check(&root, &reg).unwrap();
        // In line between Rip and FLAC it can't go: FLAC would get files.
        let mut inline = root.clone();
        assert!(inline.insert_after("r", Node::step("m", "mp3")));
        assert!(check(&inline, &reg).is_err());
        // Beside FLAC, on a branch from Rip, it does.
        let mut beside = root.clone();
        assert!(beside.add_branch_after("r", Node::step("m", "mp3")));
        check(&beside, &reg).unwrap();
        let ids = beside.ids();
        let at = |id: &str| ids.iter().position(|x| *x == id).unwrap();
        assert!(at("r") < at("f") && at("f") < at("l") && at("r") < at("m"), "{ids:?}");
        // And a third.
        assert!(beside.add_branch_after("r", Node::step("o", "opus")));
        check(&beside, &reg).unwrap();
    }
}

/// A step last in a line inside a line (the shape the Hold upgrade leaves,
/// `[Hold, Library]`) still has what follows that line after it: a branch
/// added after it goes beside that, not in line before it.
#[test]
fn a_branch_after_the_end_of_a_nested_line_goes_beside_what_follows() {
    let mut root = chain([Node::step("r", "rip"), Node::step("f", "flac"), chain([Node::step("h", "hold"), Node::step("l", "folder")]), Node::step("t", "tell")]);
    assert!(root.add_branch_after("l", Node::step("x", "folder")));
    let want = chain([
        Node::step("r", "rip"),
        Node::step("f", "flac"),
        chain([Node::step("h", "hold"), Node::step("l", "folder")]),
        drt_engine::recipe::branches([Node::step("t", "tell"), Node::step("x", "folder")]),
    ]);
    assert_eq!(root, want);
    // Last of all: it simply follows.
    let mut root = chain([Node::step("r", "rip"), chain([Node::step("f", "flac"), Node::step("l", "folder")])]);
    assert!(root.add_branch_after("l", Node::step("x", "tell")));
    assert_eq!(root.ids(), ["r", "f", "l", "x"]);
}
