//! The real Check, CHD, Plain image and Library, run by the engine on a dump
//! made here, with no drive and no network: a test source stands in for
//! Drive, Data disc and Image, and the Redump catalogue is seeded in the
//! cache, so it's read rather than fetched.

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use trip_engine::recipe::{branches, chain};
use trip_engine::{Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Step, StepInfo, StepState, Subject};
use trip_steps::real::data::DUMP;

/// Gives the dump at `main`, as Image would, with what Data disc found.
struct Dumped {
    main: PathBuf,
}

#[async_trait]
impl Step for Dumped {
    fn info(&self) -> StepInfo {
        StepInfo::new("dumped", "Dumped", Group::Source, ItemKind::Nothing, ItemKind::Dump)
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        inlet.recv().await;
        let mut dump = Item::new(DUMP, ItemKind::Dump, "MYSTRIVEN", None, Some(self.main.clone()));
        let c = dump.context_mut();
        c.add(Subject::Disc, "label", "MYSTRIVEN", "the disc");
        c.add(Subject::Disc, "media", "CD", "the disc");
        c.add(Subject::Disc, "bytes", "2705600", "the disc");
        outlet.send(dump).await;
        Ok(())
    }
}

/// A two-track dump in redumper's layout, with its log.
fn dump(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("MYSTRIVEN (Track 1).bin"), (0..700 * 2352).map(|i| (i % 251) as u8).collect::<Vec<u8>>()).unwrap();
    std::fs::write(dir.join("MYSTRIVEN (Track 2).bin"), (0..450 * 2352).map(|i| (i % 241) as u8).collect::<Vec<u8>>()).unwrap();
    std::fs::write(dir.join("MYSTRIVEN.log"), "redumper (build: b744)\n").unwrap();
    std::fs::write(dir.join("MYSTRIVEN.scram"), "working file").unwrap();
    let cue = dir.join("MYSTRIVEN.cue");
    std::fs::write(
        &cue,
        "FILE \"MYSTRIVEN (Track 1).bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\nFILE \"MYSTRIVEN (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 00 00:00:00\n    INDEX 01 00:02:00\n",
    )
    .unwrap();
    cue
}

async fn run(base: &Path, known: bool) -> trip_engine::Snapshot {
    let _ = std::fs::remove_dir_all(base);
    let main = dump(&base.join("dump"));
    // Redump's PC catalogue, fresh in the cache beside the runs, naming the
    // dump by its tracks' real hashes (or by others, when it's not known).
    let tracks = [main.with_file_name("MYSTRIVEN (Track 1).bin"), main.with_file_name("MYSTRIVEN (Track 2).bin")];
    let refs: Vec<&Path> = tracks.iter().map(PathBuf::as_path).collect();
    let roms = trip_steps::real::hash::hash_files(&refs, |_| {}, || false).unwrap();
    let sha = |i: usize| if known { roms[i].sha1.clone() } else { format!("{:040}", i) };
    let dat = format!(
        r#"<?xml version="1.0"?><datafile><header><version>test</version></header>
<game name="Myst &amp; Riven (Europe)">
<rom name="Myst &amp; Riven (Europe) (Track 1).bin" size="1646400" crc="00" md5="00" sha1="{}"/>
<rom name="Myst &amp; Riven (Europe) (Track 2).bin" size="1058400" crc="00" md5="00" sha1="{}"/>
<rom name="Myst &amp; Riven (Europe).cue" size="1" crc="00" md5="00" sha1="ffff"/>
</game></datafile>"#,
        sha(0),
        sha(1)
    );
    std::fs::create_dir_all(base.join("cache/redump")).unwrap();
    std::fs::write(base.join("cache/redump/pc.dat"), dat).unwrap();

    let mut reg = trip_steps::real();
    reg.add(Dumped { main });
    let mut e = Engine::new(reg);
    e.work_root = base.join("runs");
    let lib = base.join("Images").display().to_string();
    let kept = base.join("Kept").display().to_string();
    let recipe = Recipe {
        name: "Fixture".into(),
        root: chain([
            Node::step("src", "dumped"),
            Node::step("check", "check").with("systems", "IBM PC"),
            branches([
                chain([Node::step("chd", "chd").with("codecs", "zstd"), Node::step("lib", "folder").with("folder", lib.as_str())]),
                chain([Node::step("keep", "keep"), Node::step("kept", "folder").with("folder", kept.as_str()).with("names", "Date and label")]),
            ]),
        ]),
    };
    trip_engine::check(&recipe.root, &e.registry).unwrap();
    e.start(&recipe).unwrap().finished().await
}

fn base(name: &str) -> PathBuf {
    std::env::current_dir().unwrap().join("../../target/trip-test").join(name)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dump_redump_knows_is_checked_made_into_a_chd_and_filed() {
    let b = base("real-image-known");
    let s = run(&b, true).await;
    for step in ["check", "chd", "lib", "keep", "kept"] {
        assert_eq!(s.steps[step].state, StepState::Ok, "{step}: {} {}", s.steps[step].status, s.steps[step].detail);
    }
    assert_eq!(s.steps["check"].items[0].note, "Verified: Myst & Riven (Europe)");
    let chd = &s.steps["chd"].items[0];
    assert!(chd.note.ends_with("verified: every track matches the dump (2)"), "{}", chd.note);
    // The CHD is named for Redump's name and filed in a folder of it.
    let filed = b.join("Images/Myst & Riven (Europe)/Myst & Riven (Europe).chd");
    assert!(filed.is_file(), "{}", s.steps["lib"].items[0].note);
    // Kept whole: the image and its log together, under Redump's names,
    // not redumper's working files.
    let kept = std::fs::read_dir(b.join("Kept")).unwrap().next().unwrap().unwrap().path();
    assert!(kept.file_name().unwrap().to_string_lossy().ends_with(" MYSTRIVEN"), "{kept:?}");
    let mut names: Vec<String> = std::fs::read_dir(&kept).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["Myst & Riven (Europe) (Track 1).bin", "Myst & Riven (Europe) (Track 2).bin", "Myst & Riven (Europe).cue", "Myst & Riven (Europe).log"]);
    let cue = std::fs::read_to_string(kept.join("Myst & Riven (Europe).cue")).unwrap();
    assert_eq!(trip_steps::real::redumper::cue_files(&cue), ["Myst & Riven (Europe) (Track 1).bin", "Myst & Riven (Europe) (Track 2).bin"]);
    std::fs::remove_dir_all(b).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dump_redump_doesnt_know_is_filed_by_its_label() {
    let b = base("real-image-unknown");
    let s = run(&b, false).await;
    assert_eq!(s.steps["check"].items[0].note, "Not in Redump");
    assert_eq!(s.steps["check"].items[0].state, ItemState::Ok, "not a failure");
    assert!(b.join("Images/MYSTRIVEN/MYSTRIVEN.chd").is_file(), "{}", s.steps["lib"].items[0].note);
    std::fs::remove_dir_all(b).unwrap();
}
