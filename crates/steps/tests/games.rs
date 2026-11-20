//! Games, end to end: the recipe on simulated steps (a PlayStation disc
//! Redump knows, one it doesn't, a two-disc game), and the real Check,
//! Redump, Choose, CHD, Library and M3U filing a two-disc game on disk.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::chain;
use drt_engine::{
    check, fits_flow, flow_after, Answer, Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, Recipe, Run, Setting, Snapshot, Step, StepInfo, StepState, Subject,
};
use drt_steps::game_sim::{self, Disc, APE_ESCAPE, COLONY_WARS_1, COLONY_WARS_2, TOMB_RAIDER_UNKNOWN};
use drt_steps::real::data::DUMP;
use drt_steps::{recipes, simulated};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn engine(discs: Vec<Disc>) -> Engine {
    let mut reg = simulated(None);
    reg.add(game_sim::GameDisc::new(discs));
    let mut e = Engine::new(reg);
    e.time_scale = 0.0;
    e.settings.lock().unwrap().set_drive(recipes::BLU_RAY, "offset", Setting::detected(Value::from(103), "AccurateRip"));
    e
}

/// Runs to the end, answering each question with `answer`. Gives the last
/// snapshot and what was asked.
async fn drive(run: Run, answer: Answer) -> (Snapshot, Vec<String>) {
    drive_retrying(run, answer, &[]).await
}

/// The same, settling a bad item with `retries` in turn, one each time it
/// turns bad.
async fn drive_retrying(run: Run, answer: Answer, retries: &[&str]) -> (Snapshot, Vec<String>) {
    let mut rx = run.snapshot.clone();
    let mut asked = Vec::new();
    let mut tried = 0;
    let mut was_bad = false;
    loop {
        let snap = rx.borrow_and_update().clone();
        if snap.finished {
            break;
        }
        if let Some((step, q)) = &snap.question {
            let key = format!("{step}: {}", q.prompt);
            if !asked.contains(&key) {
                asked.push(key);
                run.send(Command::Answer { step: step.clone(), answer: answer.clone() }).await;
            }
        }
        let bad = snap.steps.iter().find_map(|(id, v)| v.items.iter().find(|i| i.state == ItemState::Bad).map(|i| (id.clone(), i.id.clone())));
        if let (Some((step, item)), false) = (&bad, was_bad) {
            if let Some(option) = retries.get(tried) {
                tried += 1;
                run.send(Command::Retry { step: step.clone(), item: item.clone(), option: (*option).into() }).await;
            }
        }
        was_bad = bad.is_some();
        if rx.changed().await.is_err() {
            break;
        }
    }
    (run.finished().await, asked)
}

#[test]
fn the_games_recipe_checks_and_its_steps_follow_from_one_another() {
    for reg in [simulated(None), drt_steps::real()] {
        let r = recipes::games().root;
        check(&r, &reg).unwrap();
        let names = |id: &str| fits_flow(&flow_after(&r, id, &reg).unwrap(), &reg).iter().map(|i| (i.kind, i.name)).collect::<Vec<_>>();
        let has = |id: &str, kind: &str| names(id).iter().any(|(k, _)| *k == kind);
        assert!(has("g_drive", "game-disc") && has("g_drive", "data-disc"));
        assert!(has("g_disc", "game-rip") && !has("g_disc", "image") && !has("g_disc", "rip"), "{:?}", names("g_disc"));
        // Redump looks in the system Game disc found, by the hash Check gives.
        assert!(has("g_rip", "check") && !has("g_rip", "redump") && has("g_check", "redump"), "Redump goes straight after Check");
        // Choose only once something has offered candidates.
        assert!(!has("g_check", "game-choose") && has("g_redump", "game-choose"));
        assert!(has("g_chd", "folder") && has("g_lib", "m3u") && has("g_lib", "libcrypt"));
    }
    // Rip can't follow a data disc, nor Image a game disc.
    let reg = simulated(None);
    assert!(check(&chain([Node::step("d", "drive"), Node::step("dd", "data-disc"), Node::step("r", "game-rip")]), &reg).is_err());
    assert!(check(&chain([Node::step("d", "drive"), Node::step("g", "game-disc"), Node::step("i", "image")]), &reg).is_err());
    assert!(recipes::all().iter().any(|r| r.name == "Games"));
}

#[test]
fn the_simulated_game_steps_are_the_real_ones() {
    let (sim, real) = (simulated(None), drt_steps::real());
    for kind in ["game-disc", "game-rip", "redump", "game-choose", "m3u", "libcrypt"] {
        let (a, b) = (sim.info(kind).unwrap(), real.info(kind).unwrap());
        let keys = |i: &StepInfo| i.params.iter().map(|p| p.key).collect::<Vec<_>>();
        assert_eq!((a.name, a.input, a.output, keys(&a), &a.needs, &a.gives), (b.name, b.input, b.output, keys(&b), &b.needs, &b.gives), "{kind}");
        assert_eq!(a.retries, b.retries, "{kind}");
    }
    let rip = real.info("game-rip").unwrap();
    assert_eq!(rip.name, "Rip");
    assert_eq!(rip.retries.iter().map(|r| r.label).collect::<Vec<_>>(), ["Re-read bad parts", "Keep it anyway"]);
    assert_eq!(rip.params.iter().map(|p| p.key).collect::<Vec<_>>(), ["retries", "speed"], "Image's settings, so its code reads them");
}

#[tokio::test]
async fn a_game_goes_all_the_way_and_redump_names_it() {
    let e = engine(vec![APE_ESCAPE]);
    let (s, asked) = drive(e.start(&recipes::games()).unwrap(), Answer::Skip).await;
    assert!(asked.is_empty(), "the hash decides: nothing asked {asked:?}");
    // Its serial, the system beside it, the region as the one fact.
    assert_eq!((s.steps["g_disc"].status.as_str(), s.steps["g_disc"].qualifier.as_str()), ("SCES-01564", "PlayStation"));
    assert!(s.steps["g_disc"].detail.starts_with("Europe"), "{}", s.steps["g_disc"].detail);
    assert_eq!(s.steps["g_check"].items[0].note, "Verified: Ape Escape (Europe)");
    // By hash: one entry, nothing weaker to say.
    assert_eq!((s.steps["g_redump"].status.as_str(), s.steps["g_redump"].qualifier.as_str()), ("1 match", ""));
    assert_eq!((s.steps["g_choose"].status.as_str(), s.steps["g_choose"].detail.as_str()), ("Ape Escape (Europe)", "verified"));
    assert_eq!(s.steps["g_lib"].items[0].note, "~/Games/PlayStation/Ape Escape (Europe)");
    assert_eq!(s.steps["g_m3u"].status, "One disc");
    assert_eq!(s.steps["g_sbi"].detail, "Ape Escape (Europe).sbi beside it", "a PAL game LibCrypt protects");
    for step in ["g_disc", "g_rip", "g_check", "g_redump", "g_choose", "g_chd", "g_lib", "g_sbi", "g_m3u"] {
        assert_eq!(s.steps[step].state, StepState::Ok, "{step}: {}", s.steps[step].status);
    }
    assert_eq!(s.steps["g_next"].status, "Ejected");
    assert_eq!(s.headline(), "Done");
}

/// A Folder's names follow what reaches it: one added after the CHD with
/// nothing chosen, or one saved on the music layout when that was every
/// Folder's default, files a game by system and name, as its card says.
#[tokio::test]
async fn a_game_s_folder_on_no_names_or_the_old_music_default_files_by_system_and_name() {
    for saved in [None, Some(drt_steps::template::DEFAULT), Some("Artist / Album / 01 - Title")] {
        let e = engine(vec![APE_ESCAPE]);
        let mut r = recipes::games();
        if let Some(Node::Step { params, .. }) = r.root.find_mut("g_lib") {
            match saved {
                Some(v) => params.insert("names".into(), Setting::chosen(Value::from(v))),
                None => params.remove("names"),
            };
        }
        let (s, _) = drive(e.start(&r).unwrap(), Answer::Skip).await;
        assert_eq!(s.steps["g_lib"].items[0].note, "~/Games/PlayStation/Ape Escape (Europe)", "{saved:?}");
    }
}

#[tokio::test]
async fn a_hash_redump_doesnt_have_asks_with_the_serials_entries() {
    let e = engine(vec![TOMB_RAIDER_UNKNOWN]);
    let (s, asked) = drive_retrying(e.start(&recipes::games()).unwrap(), Answer::Pick(0), &["refine", "keep"]).await;
    assert_eq!(asked, ["g_choose: Which game? None matched exactly."]);
    assert_eq!(s.steps["g_check"].items[0].note, "kept anyway; doesn't match Redump's copies of SLUS-00152");
    assert_eq!((s.steps["g_redump"].status.as_str(), s.steps["g_redump"].qualifier.as_str(), s.steps["g_redump"].detail.as_str()), ("2 matches", "by serial only", "no exact match for SLUS-00152"));
    // The revision whose size the disc's is was offered first, and taken; unverified.
    assert_eq!((s.steps["g_choose"].status.as_str(), s.steps["g_choose"].detail.as_str()), ("Tomb Raider (USA) (Rev 1)", "your choice, unverified"));
    assert_eq!(s.steps["g_lib"].items[0].note, "~/Games/PlayStation/Tomb Raider (USA) (Rev 1)");
}

#[tokio::test]
async fn a_game_you_name_yourself_or_skip() {
    let e = engine(vec![TOMB_RAIDER_UNKNOWN]);
    let typed = Answer::Details([("name".to_owned(), "Tomb Raider (USA) (Prototype)".to_owned())].into());
    let (s, _) = drive_retrying(e.start(&recipes::games()).unwrap(), typed, &["keep"]).await;
    assert_eq!(s.steps["g_choose"].status, "Tomb Raider (USA) (Prototype)");
    assert_eq!(s.steps["g_lib"].items[0].note, "~/Games/PlayStation/Tomb Raider (USA) (Prototype)");
    let (s, _) = drive_retrying(e.start(&recipes::games()).unwrap(), Answer::Skip, &["keep"]).await;
    assert_eq!(s.steps["g_choose"].status, "Skipped");
    assert_eq!(s.steps["g_lib"].items[0].note, "~/Games/PlayStation/SLUS_001.52", "named by its label");
}

/// Redump has dumps of its serial and this matches none: suspect.
#[tokio::test]
async fn a_dump_redump_disagrees_with_is_held_until_you_say() {
    let e = engine(vec![TOMB_RAIDER_UNKNOWN]);
    let run = e.start(&recipes::games()).unwrap();
    // Held, Bad, and nothing filed while it waits.
    let mut rx = run.snapshot.clone();
    let held = loop {
        let s = rx.borrow_and_update().clone();
        if s.steps["g_check"].items.first().is_some_and(|i| i.state == ItemState::Bad) {
            break s;
        }
        rx.changed().await.unwrap();
    };
    let item = &held.steps["g_check"].items[0];
    assert_eq!(item.note, "Doesn't match Redump's dumps of SLUS-00152");
    assert_eq!(held.steps["g_check"].state, StepState::Bad);
    assert_eq!(held.steps["g_lib"].count(ItemState::Ok), 0);
    let check = drt_steps::real().info("check").unwrap();
    assert_eq!(check.retries.iter().map(|r| r.label).collect::<Vec<_>>(), ["Re-read bad parts", "Keep it anyway", "Skip this disc"]);
    // Left out: not filed, and nothing asked about it.
    run.send(Command::Retry { step: "g_check".into(), item: item.id.clone(), option: "leave_out".into() }).await;
    let (s, asked) = drive(run, Answer::Skip).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(s.steps["g_check"].count(ItemState::Skipped), 1);
    assert_eq!(s.steps["g_lib"].count(ItemState::Ok), 0);
}

/// A serial Redump has nothing for says nothing about the dump.
#[tokio::test]
async fn a_serial_redump_doesnt_know_is_only_a_question() {
    const UNCATALOGUED: Disc =
        Disc { label: "SLES_999.99", serial: "SLES-99999", region: "Europe", sectors: 1000, bytes: 2_048_000, roms: &[("SLES_999.99.bin", "1111000000000000000000000000000000000000")] };
    let e = engine(vec![UNCATALOGUED]);
    let (s, asked) = drive(e.start(&recipes::games()).unwrap(), Answer::Skip).await;
    assert_eq!(asked, ["g_choose: What is this game?"]);
    assert_eq!(s.steps["g_check"].items[0].note, "Not in Redump");
    assert_eq!(s.steps["g_check"].items[0].state, ItemState::Ok);
    assert_eq!(s.steps["g_lib"].count(ItemState::Ok), 1);
}

#[tokio::test]
async fn a_two_disc_game_gets_its_playlist_once_both_are_filed() {
    let e = engine(vec![COLONY_WARS_1, COLONY_WARS_2]);
    let (one, _) = drive(e.start(&recipes::games()).unwrap(), Answer::Skip).await;
    assert_eq!((one.steps["g_disc"].status.as_str(), one.steps["g_disc"].qualifier.as_str()), ("SLES-00860", "PlayStation"));
    assert_eq!(one.steps["g_lib"].items[0].note, "~/Games/PlayStation/Colony Wars (Europe) (Disc 1)");
    assert_eq!((one.steps["g_m3u"].status.as_str(), one.steps["g_m3u"].detail.as_str()), ("Disc 1 of Colony Wars (Europe)", "Waits for another disc"));
    let (two, asked) = drive(e.start(&recipes::games()).unwrap(), Answer::Skip).await;
    assert!(asked.is_empty());
    assert_eq!(two.steps["g_choose"].status, "Colony Wars (Europe) (Disc 2)");
    assert_eq!((two.steps["g_m3u"].status.as_str(), two.steps["g_m3u"].qualifier.as_str()), ("Colony Wars (Europe).m3u", "2 discs"));
}

// ---- the real steps, on disk -------------------------------------------------------------------------

/// Gives each run the next dump, as Game disc and Rip would, with what
/// Game disc found.
struct Dumped {
    discs: Vec<(PathBuf, &'static str, &'static str)>,
    next: AtomicUsize,
}

#[async_trait]
impl Step for Dumped {
    fn info(&self) -> StepInfo {
        StepInfo::new("dumped", "Dumped", Group::Source, ItemKind::Nothing, ItemKind::Dump).gives(&["serial", "label", "system", "read"])
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        inlet.recv().await;
        let (main, label, serial) = self.discs[self.next.fetch_add(1, Ordering::SeqCst)].clone();
        let mut dump = Item::new(DUMP, ItemKind::Dump, label, None, Some(main));
        for (k, v) in [("label", label), ("media", "CD"), ("bytes", "705600"), ("system", "PlayStation"), ("catalogue", "psx"), ("serial", serial), ("sectors", "300")] {
            dump.context_mut().add(Subject::Disc, k, v, "the disc");
        }
        outlet.send(dump).await;
        Ok(())
    }
}

/// A one-track PlayStation dump in redumper's layout.
fn dump(dir: &Path, label: &str, seed: u8) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{label}.bin")), (0..300 * 2352).map(|i| (i % 251) as u8 ^ seed).collect::<Vec<u8>>()).unwrap();
    std::fs::write(dir.join(format!("{label}.log")), "redumper\n").unwrap();
    let cue = dir.join(format!("{label}.cue"));
    std::fs::write(&cue, format!("FILE \"{label}.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n")).unwrap();
    cue
}

fn sha1(file: &Path) -> String {
    drt_steps::real::hash::hash_files(&[file], |_| {}, || false).unwrap()[0].sha1.clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_two_disc_game_is_verified_made_into_chds_filed_and_given_its_playlist() {
    // Each disc in a folder of its own: the playlist goes above them.
    two_discs(
        "real-games",
        "{system}/{name}",
        |name| format!("PlayStation/{name}/{name}"),
        "PlayStation/Colony Wars (Europe).m3u",
        "Colony Wars (Europe) (Disc 1)/Colony Wars (Europe) (Disc 1).chd\nColony Wars (Europe) (Disc 2)/Colony Wars (Europe) (Disc 2).chd\n",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_two_disc_game_filed_in_one_folder_has_its_playlist_beside_its_discs() {
    two_discs(
        "real-games-folder",
        "System / Game",
        |name| format!("PlayStation/Colony Wars (Europe)/{name}"),
        "PlayStation/Colony Wars (Europe)/Colony Wars (Europe).m3u",
        "Colony Wars (Europe) (Disc 1).chd\nColony Wars (Europe) (Disc 2).chd\n",
    )
    .await;
}

/// Two discs of a set through the real steps, filed with `names`: `at`
/// gives where a disc's files go (without extension), relative.
async fn two_discs(dir: &str, names: &str, at: fn(&str) -> String, m3u: &str, body: &str) {
    let base = std::env::current_dir().unwrap().join("../../target/drt-test").join(dir);
    let _ = std::fs::remove_dir_all(&base);
    let one = dump(&base.join("dump1"), "SLES_008.60", 0);
    let two = dump(&base.join("dump2"), "SLES_108.60", 0x5A);
    // Redump's PlayStation catalogue, fresh in the cache, with its serials.
    let dat = format!(
        r#"<?xml version="1.0"?><datafile><header><version>test</version></header>
<game name="Colony Wars (Europe) (Disc 1)"><serial>SLES-00860</serial>
<rom name="Colony Wars (Europe) (Disc 1).cue" size="1" crc="00" md5="00" sha1="ffff"/>
<rom name="Colony Wars (Europe) (Disc 1).bin" size="705600" crc="00" md5="00" sha1="{}"/></game>
<game name="Colony Wars (Europe) (Disc 2)"><serial>SLES-10860</serial>
<rom name="Colony Wars (Europe) (Disc 2).bin" size="705600" crc="00" md5="00" sha1="{}"/></game>
</datafile>"#,
        sha1(&one.with_extension("bin")),
        sha1(&two.with_extension("bin"))
    );
    std::fs::create_dir_all(base.join("cache/redump")).unwrap();
    std::fs::write(base.join("cache/redump/psx.dat"), dat).unwrap();
    // Redump's LibCrypt pack, fresh in the cache: disc 2 is protected.
    {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::fs::File::create(base.join("cache/redump/psx-sbi.zip")).unwrap());
        z.start_file("Colony Wars (Europe) (Disc 2).sbi", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"SBI\0").unwrap();
        z.finish().unwrap();
    }

    let mut reg = drt_steps::real();
    reg.add(Dumped { discs: vec![(one, "SLES_008.60", "SLES-00860"), (two, "SLES_108.60", "SLES-10860")], next: AtomicUsize::new(0) });
    let mut e = Engine::new(reg);
    e.work_root = base.join("runs");
    let lib = base.join("Games");
    let recipe = Recipe {
        name: "Fixture games".into(),
        root: chain([
            Node::step("src", "dumped"),
            Node::step("check", "check"),
            Node::step("redump", "redump"),
            Node::step("choose", "game-choose"),
            Node::step("chd", "chd"),
            Node::step("lib", "folder").with("folder", lib.display().to_string().as_str()).with("names", names),
            Node::step("sbi", "libcrypt"),
            Node::step("m3u", "m3u"),
        ]),
    };
    check(&recipe.root, &e.registry).unwrap();
    for disc in 1..=2 {
        let (s, asked) = drive(e.start(&recipe).unwrap(), Answer::Skip).await;
        assert!(asked.is_empty(), "{asked:?}");
        for step in ["check", "redump", "choose", "chd", "lib", "sbi", "m3u"] {
            assert_eq!(s.steps[step].state, StepState::Ok, "disc {disc}, {step}: {} {}", s.steps[step].status, s.steps[step].detail);
        }
        let name = format!("Colony Wars (Europe) (Disc {disc})");
        assert_eq!(s.steps["check"].items[0].note, format!("Verified: {name}"));
        assert_eq!(s.steps["choose"].status, name);
        assert!(s.steps["chd"].items[0].note.ends_with("verified: every track matches the dump (1)"), "{}", s.steps["chd"].items[0].note);
        assert!(lib.join(format!("{}.chd", at(&name))).is_file(), "{}", s.steps["lib"].items[0].note);
        assert_eq!(lib.join(format!("{}.sbi", at(&name))).is_file(), disc == 2, "{}", s.steps["sbi"].items[0].note);
        if disc == 1 {
            assert_eq!(s.steps["m3u"].detail, "Waits for another disc");
            assert!(!lib.join(m3u).exists());
        } else {
            assert_eq!((s.steps["m3u"].status.as_str(), s.steps["m3u"].qualifier.as_str()), ("Colony Wars (Europe).m3u", "2 discs"));
        }
    }
    assert_eq!(std::fs::read_to_string(lib.join(m3u)).unwrap(), body);
    std::fs::remove_dir_all(base).unwrap();
}
