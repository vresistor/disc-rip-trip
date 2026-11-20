//! The engine, run end to end on simulated steps.

use drt_engine::recipe::{branches, chain};
use drt_engine::{check, fits, Answer, Command, Engine, ItemKind, ItemState, Node, Recipe, Run, Setting, Snapshot, StepState};
use drt_steps::{recipes, simulated};
use serde_json::Value;

fn engine(bad_track: Option<u32>) -> Engine {
    let mut e = Engine::new(simulated(bad_track));
    e.time_scale = 0.0;
    {
        let mut s = e.settings.lock().unwrap();
        s.set_drive(recipes::BLU_RAY, "offset", Setting::detected(Value::from(103), "AccurateRip"));
        s.set_drive(recipes::DVD, "offset", Setting::detected(Value::from(6), "AccurateRip"));
    }
    e
}

/// How the test answers whatever the run asks.
struct Policy {
    pick: usize,
    retry: &'static str,
}

/// Runs to the end, answering questions, approving holds and settling bad
/// items as `policy` says. Gives the last snapshot and what was asked.
async fn drive(run: Run, policy: Policy) -> (Snapshot, Vec<String>) {
    let mut rx = run.snapshot.clone();
    let mut asked = Vec::new();
    let mut approved = Vec::new();
    let mut retried = Vec::new();
    loop {
        let snap = rx.borrow_and_update().clone();
        if snap.finished {
            break;
        }
        if let Some((step, q)) = &snap.question {
            if !asked.contains(&format!("{step}: {}", q.prompt)) {
                asked.push(format!("{step}: {}", q.prompt));
                run.send(Command::Answer { step: step.clone(), answer: Answer::Pick(policy.pick) }).await;
            }
        }
        for (id, v) in &snap.steps {
            if v.state == StepState::Hold && v.detail == "All here" && !approved.contains(id) {
                approved.push(id.clone());
                run.send(Command::Approve { step: id.clone() }).await;
            }
            for item in v.items.iter().filter(|i| i.state == ItemState::Bad) {
                let key = format!("{id}/{}", item.id);
                if !retried.contains(&key) {
                    retried.push(key);
                    run.send(Command::Retry { step: id.clone(), item: item.id.clone(), option: policy.retry.into() }).await;
                }
            }
        }
        if rx.changed().await.is_err() {
            break;
        }
    }
    (run.finished().await, asked)
}

fn ok_items(s: &Snapshot, step: &str) -> usize {
    s.steps[step].count(ItemState::Ok)
}

#[test]
fn the_built_in_recipes_type_check() {
    let reg = simulated(None);
    for r in recipes::all() {
        check(&r.root, &reg).unwrap_or_else(|e| panic!("{}: {e}", r.name));
    }
}

#[test]
fn steps_follow_only_what_gives_them_their_input() {
    use drt_engine::{fits_flow, flow_after};
    let reg = simulated(None);
    let r = recipes::cd().root;
    let names = |id: &str| fits_flow(&flow_after(&r, id, &reg).unwrap(), &reg).iter().map(|i| i.name).collect::<Vec<_>>();
    // After the drive: only a kind step, since nothing knows what the disc is.
    let after_drive = names("drive");
    assert!(after_drive.contains(&"Audio CD") && !after_drive.contains(&"MusicBrainz") && !after_drive.contains(&"Rip"), "{after_drive:?}");
    // After Audio CD: lookups and Rip, but not Choose (nothing to choose from) or Cover art (no release).
    let after_cd = names("cd");
    assert!(after_cd.contains(&"MusicBrainz") && after_cd.contains(&"Rip"), "{after_cd:?}");
    assert!(!after_cd.contains(&"Choose") && !after_cd.contains(&"Cover art"), "{after_cd:?}");
    // After Choose: Cover art.
    assert!(names("choose").contains(&"Cover art"));
    // Next disc only once something has read the disc, never straight
    // after the Drive (it would open and close the tray for ever).
    assert!(!after_drive.contains(&"Next disc"), "{after_drive:?}");
    assert!(!after_cd.contains(&"Next disc"), "{after_cd:?}");
    assert!(names("rip").contains(&"Next disc"));
    // Cover art straight after a lookup doesn't check.
    let bad = chain([Node::step("d", "drive"), Node::step("cd", "audio-cd"), Node::step("mb", "musicbrainz"), Node::step("a", "cover-art")]);
    assert!(check(&bad, &reg).unwrap_err().to_string().contains("a needs release"));
}

#[test]
fn flac_straight_after_the_kind_step_does_not_check() {
    let reg = simulated(None);
    let root = chain([Node::step("d", "drive"), Node::step("cd", "audio-cd"), Node::step("f", "flac")]);
    let err = check(&root, &reg).unwrap_err().to_string();
    assert!(err.contains("f takes Track but is given AudioDisc"), "{err}");
}

#[test]
fn the_kind_after_a_step_is_what_it_gives_in_place() {
    use drt_engine::kind_after;
    let reg = simulated(None);
    let r = recipes::cd().root;
    assert_eq!(kind_after(&r, "cd", &reg), Some(ItemKind::AudioDisc));
    assert_eq!(kind_after(&r, "rip", &reg), Some(ItemKind::Track));
    assert_eq!(kind_after(&r, "flac", &reg), Some(ItemKind::AudioFile));
    assert_eq!(kind_after(&r, "lib2", &reg), Some(ItemKind::AudioFile));
    assert_eq!(kind_after(&r, "nope", &reg), None);
    // Past a type error there's nothing to say.
    let bad = chain([Node::step("d", "drive"), Node::step("f", "flac"), Node::step("l", "folder")]);
    assert_eq!(kind_after(&bad, "l", &reg), None);
}

#[test]
fn removing_a_step_takes_what_depends_on_it_and_leaves_a_recipe_that_checks() {
    let reg = simulated(None);
    // Rip: and everything after it on its line, the three outputs with it.
    let mut r = recipes::cd().root;
    let gone = r.remove_with_downstream("rip", &reg);
    assert_eq!(gone[0], "rip");
    for id in ["flac", "review", "hold", "lib1", "mp3", "lib2", "next"] {
        assert!(gone.contains(&id.to_owned()), "{id} goes with Rip: {gone:?}");
    }
    assert_eq!(r.ids(), ["drive", "cd", "mb", "cdtext", "choose", "art"]);
    check(&r, &reg).unwrap();
    // One of two lookups: just its branch; Choose still has the other.
    let mut r = recipes::cd().root;
    assert_eq!(r.remove_with_downstream("mb", &reg), ["mb"]);
    check(&r, &reg).unwrap();
    // A step in the middle of a line: what follows still makes sense
    // without it (FLAC, then Hold, then Library), so it stays joined up.
    let mut r = recipes::cd().root;
    let gone = r.remove_with_downstream("review", &reg);
    assert_eq!(gone, ["review"]);
    check(&r, &reg).unwrap();
    // Cover art from between Choose and Rip: Rip still fits.
    let mut r = recipes::cd().root;
    assert_eq!(r.remove_with_downstream("art", &reg), ["art"]);
    check(&r, &reg).unwrap();
    // The first step: nothing would be left, so nothing goes.
    let mut r = recipes::cd().root;
    assert!(r.remove_with_downstream("drive", &reg).is_empty());
    assert_eq!(r, recipes::cd().root);
}

#[test]
fn a_join_that_loses_what_it_takes_goes_too() {
    let reg = simulated(None);
    // Choose needs candidates or titles from a lookup: take both lookups
    // and Choose has nothing to choose from, so it goes, and what follows
    // it on its line.
    let mut r = recipes::cd().root;
    r.remove_with_downstream("mb", &reg);
    let gone = r.remove_with_downstream("cdtext", &reg);
    assert_eq!(gone[0], "cdtext");
    assert!(gone.contains(&"choose".to_owned()) && gone.contains(&"art".to_owned()), "{gone:?}");
    check(&r, &reg).unwrap();
    // FLAC would be given a disc: it goes with Rip; Notify takes
    // anything, so it stays.
    let mut r = chain([Node::step("d", "drive"), Node::step("cd", "audio-cd"), Node::step("rip", "rip"), branches([Node::step("f", "flac"), Node::step("m", "mp3")]), Node::step("t", "tell")]);
    let gone = r.remove_with_downstream("rip", &reg);
    assert_eq!(gone, ["rip", "f", "m"]);
    check(&r, &reg).unwrap();
}

#[test]
fn an_unavailable_step_is_removed_with_its_line_and_doesnt_cascade() {
    let reg = simulated(None);
    let mut r = chain([Node::step("d", "drive"), Node::step("cd", "audio-cd"), branches([Node::step("x", "zzz"), Node::step("mb", "musicbrainz")]), Node::step("ch", "choose")]);
    assert_eq!(r.remove_with_downstream("x", &reg), ["x"]);
    check(&r, &reg).unwrap();
}

#[test]
fn the_picker_after_a_disc_offers_lookups_and_rip_but_no_encoders() {
    let reg = simulated(None);
    let names: Vec<_> = fits(ItemKind::AudioDisc, &reg).iter().map(|i| i.name).collect();
    assert!(names.contains(&"MusicBrainz") && names.contains(&"Choose") && names.contains(&"Rip"), "{names:?}");
    assert!(!names.contains(&"FLAC"), "{names:?}");
}

#[tokio::test]
async fn the_offset_is_looked_up_as_soon_as_the_recipe_has_a_drive() {
    let mut e = Engine::new(simulated(None));
    e.time_scale = 0.0;
    let r = recipes::cd();
    // Unknown, so the recipe can't start yet, and the Drive tile shows why.
    assert_eq!(e.missing(&r).len(), 1);
    assert_eq!(e.idle(&r).steps["drive"].state, StepState::Needs);
    // Looked up without running anything.
    assert!(e.find_missing(&r, false).await.is_empty());
    let found = e.settings.lock().unwrap().drive(recipes::BLU_RAY, "offset");
    assert_eq!(found.resolve(None), Some(Value::from(103)));
    assert_eq!(found.provenance(), "AccurateRip, 528 agree");
    assert!(e.start(&r).is_ok());
}

#[test]
fn next_disc_is_the_step_that_leads_back_to_the_start() {
    let reg = simulated(None);
    let info = reg.info("next-disc").unwrap();
    assert!(info.loops_back);
    assert_eq!(drt_engine::check(&recipes::cd().root, &reg).unwrap(), ItemKind::Any, "it ends the recipe");
}

#[test]
fn before_the_start_tiles_say_what_they_follow() {
    let e = engine(None);
    let idle = e.idle(&recipes::cd());
    assert_eq!(idle.steps["rip"].status, "After cover art");
    assert_eq!(idle.steps["hold"].state, StepState::Hold);
    assert_eq!(idle.steps["lib2"].from, "~/Music/MP3");
}

#[tokio::test]
async fn a_cd_goes_all_the_way_through() {
    let e = engine(None);
    let run = e.start(&recipes::cd()).unwrap();
    let (s, asked) = drive(run, Policy { pick: 1, retry: "keep" }).await;
    assert_eq!(asked, vec!["choose: Which album is this?"]);
    // The release, its year beside it, and its catalogue number and label.
    assert_eq!((s.steps["choose"].qualifier.as_str(), s.steps["choose"].detail.as_str()), ("2002", "XLCD 151 · XL Recordings"), "the second release was picked");
    for step in ["rip", "flac", "mp3", "review", "lib1", "lib2"] {
        assert_eq!(ok_items(&s, step), 17, "{step}");
        assert_eq!(s.steps[step].state, StepState::Ok, "{step}");
    }
    // Each in its job's unit.
    assert_eq!(s.steps["rip"].status, "17/17 verified");
    assert!(s.steps["rip"].items.iter().all(|i| i.verified), "every track matched AccurateRip");
    assert_eq!(s.steps["cd"].qualifier, "43:45");
    // An encoder's card is its size alone; how it packed is said in words.
    for step in ["flac", "mp3"] {
        assert!(s.steps[step].status.ends_with(" MB") && s.steps[step].qualifier.is_empty(), "{step}: {:?}", (&s.steps[step].status, &s.steps[step].qualifier));
    }
    assert_eq!(s.steps["flac"].detail, "63% of the CD's size");
    assert_eq!(s.steps["mp3"].detail, "average 240 kbps");
    for step in ["review", "lib1", "lib2"] {
        assert_eq!(s.steps[step].status, "17 filed", "{step}");
        assert_eq!(s.steps[step].detail, "The White Stripes / The White Stripes", "{step}");
    }
    assert_eq!(s.steps["next"].status, "Ejected");
    assert_eq!(s.headline(), "Done");
}

#[tokio::test]
async fn settings_saved_for_retired_switches_load_and_are_ignored() {
    // Encoders once had switches for the cover, the tags and checking a
    // FLAC. A recipe or settings file saved with them still loads and runs.
    fn stale(v: &mut Value) {
        match v {
            Value::Object(m) => {
                if matches!(m.get("step").and_then(Value::as_str), Some("flac" | "mp3")) {
                    let off = serde_json::to_value(Setting::chosen(Value::from(false))).unwrap();
                    let params = m.entry("params").or_insert_with(|| Value::Object(Default::default())).as_object_mut().unwrap();
                    params.insert("cover".into(), off.clone());
                    params.insert("verify".into(), off);
                    params.insert("cover_size".into(), serde_json::to_value(Setting::chosen(Value::from("500 px"))).unwrap());
                    params.insert("tags".into(), serde_json::to_value(Setting::chosen(Value::from("Basic"))).unwrap());
                }
                m.values_mut().for_each(stale);
            }
            Value::Array(a) => a.iter_mut().for_each(stale),
            _ => {}
        }
    }
    let mut json = serde_json::to_value(recipes::cd()).unwrap();
    stale(&mut json);
    let r: Recipe = serde_json::from_value(json).unwrap();
    assert!(serde_json::to_string(&r).unwrap().contains("cover_size"), "the stale values are there");
    let e = engine(None);
    e.settings.lock().unwrap().set_app("flac", "verify", Setting::chosen(Value::from(false)));
    let saved = serde_json::to_string(&*e.settings.lock().unwrap()).unwrap();
    *e.settings.lock().unwrap() = serde_json::from_str(&saved).unwrap();
    check(&r.root, &e.registry).unwrap();
    let (s, _) = drive(e.start(&r).unwrap(), Policy { pick: 0, retry: "keep" }).await;
    for step in ["flac", "mp3"] {
        assert_eq!((s.steps[step].state, ok_items(&s, step)), (StepState::Ok, 17), "{step}");
    }
}

#[tokio::test]
async fn titles_come_from_the_chosen_release_not_cd_text() {
    let e = engine(None);
    let run = e.start(&recipes::cd()).unwrap();
    let ctx_probe = run.commands();
    let (s, _) = drive(run, Policy { pick: 0, retry: "keep" }).await;
    drop(ctx_probe);
    let row = s.steps["rip"].items.iter().find(|i| i.id == "track-1").unwrap();
    assert_eq!(row.label, "Jimmy the Exploder");
}

#[tokio::test]
async fn a_track_that_does_not_verify_can_be_re_read() {
    let e = engine(Some(9));
    let run = e.start(&recipes::cd()).unwrap();
    let (s, _) = drive(run, Policy { pick: 0, retry: "two_reads" }).await;
    let nine = s.steps["rip"].items.iter().find(|i| i.id == "track-9").unwrap();
    assert_eq!(nine.note, "✓ two reads agree");
    assert_eq!(ok_items(&s, "lib2"), 17);
}

#[tokio::test]
async fn a_track_left_out_is_not_filed() {
    let e = engine(Some(9));
    let run = e.start(&recipes::cd()).unwrap();
    let (s, _) = drive(run, Policy { pick: 0, retry: "leave_out" }).await;
    assert_eq!(s.steps["rip"].count(ItemState::Skipped), 1);
    assert_eq!(ok_items(&s, "lib1"), 16);
    assert_eq!(ok_items(&s, "lib2"), 16);
}

#[tokio::test]
async fn a_held_library_waits_until_approved() {
    let e = engine(None);
    let run = e.start(&recipes::cd()).unwrap();
    let mut rx = run.snapshot.clone();
    // Answer the question but never approve: the review folder fills, the library doesn't.
    loop {
        let snap = rx.borrow_and_update().clone();
        if let Some((step, _)) = &snap.question {
            run.send(Command::Answer { step: step.clone(), answer: Answer::Pick(0) }).await;
        }
        let lib = &snap.steps["hold"];
        if lib.detail == "All here" {
            assert_eq!(ok_items(&snap, "review"), 17);
            assert_eq!(ok_items(&snap, "lib1"), 0);
            assert_eq!(lib.status, "17 held");
            break;
        }
        rx.changed().await.unwrap();
    }
    run.send(Command::Approve { step: "hold".into() }).await;
    let s = run.finished().await;
    assert_eq!(ok_items(&s, "lib1"), 17);
}

#[tokio::test]
async fn stop_keeps_what_was_finished() {
    let e = engine(None);
    let run = e.start(&recipes::cd()).unwrap();
    let mut rx = run.snapshot.clone();
    loop {
        let snap = rx.borrow_and_update().clone();
        if let Some((step, _)) = &snap.question {
            run.send(Command::Answer { step: step.clone(), answer: Answer::Pick(0) }).await;
        }
        if ok_items(&snap, "lib2") >= 3 {
            break;
        }
        rx.changed().await.unwrap();
    }
    run.send(Command::Stop).await;
    let s = run.finished().await;
    assert!(s.stopped);
    assert_eq!(s.headline(), "Stopped");
    // Nothing that hadn't got going turns green.
    for (id, v) in &s.steps {
        if v.status == "Didn't run" || v.status == "Stopped" {
            assert_ne!(v.state, StepState::Ok, "{id}");
        }
    }
    assert!(ok_items(&s, "lib2") >= 3, "finished tracks stay finished");
}

#[tokio::test]
async fn the_most_complex_recipe_runs_and_joins_at_the_end() {
    let e = engine(None);
    let run = e.start(&recipes::archive()).unwrap();
    let (s, asked) = drive(run, Policy { pick: 0, retry: "keep" }).await;
    assert_eq!(asked, vec!["a_ch: Which album is this?"]);
    // GnuDB timed out: skipped, not failed, its one item (the disc) left out.
    assert_eq!(s.steps["a_gn"].state, StepState::Ok, "GnuDB timed out");
    assert_eq!((s.steps["a_gn"].status.as_str(), s.steps["a_gn"].count(ItemState::Skipped)), ("No answer", 1));
    assert_eq!(s.steps["a_ch"].state, StepState::Ok, "and Choose went ahead without it");
    for step in ["a_nas", "a_bak", "a_mlib", "a_ph"] {
        assert_eq!(ok_items(&s, step), 17, "{step}");
    }
    assert_eq!(s.steps["a_cue"].count(ItemState::Ok), 2, "the cue sheet and the rip log");
    let cue = &s.steps["a_cue"].record.files;
    assert!(cue.iter().all(|f| f.path.starts_with("~/Music/FLAC/")), "beside the filed FLACs: {cue:?}");
    assert_eq!(s.steps["a_lrc"].status, "14 .lrc files", "an .lrc for each track with synced lyrics");
    assert_eq!(s.steps["a_lrc"].count(ItemState::Skipped), 3);
    // Every scan, in an Artwork folder beside the filed FLACs.
    let scans = &s.steps["a_scans"];
    assert_eq!((scans.status.as_str(), scans.detail.as_str(), scans.count(ItemState::Ok)), ("6 pictures", "From this release", 6));
    assert!(scans.record.files.iter().all(|f| f.path.starts_with("~/Music/FLAC/") && f.path.contains("/Artwork/")), "{:?}", scans.record.files);
    assert!(scans.record.files.iter().any(|f| f.path.ends_with("/Artwork/Front.jpg")));
    assert_eq!((s.steps["a_lyr"].status.as_str(), s.steps["a_lyr"].detail.as_str()), ("14/17 synced", "none for 3, 10, 15"));
    assert_eq!(s.steps["a_lyr"].count(ItemState::Skipped), 3, "tracks without lyrics are left out of its strip");
    assert_eq!(s.steps["a_rg"].status, "\u{2212}7.3 dB");
    assert!(s.steps["a_rg"].detail.starts_with("tracks \u{2212}"), "{}", s.steps["a_rg"].detail);
    assert_eq!(s.steps["a_rg"].state, StepState::Ok);
    // 17 tracks, each once however many branches filed it.
    assert_eq!(s.steps["a_tell"].status, "Sent");
    assert_eq!(s.steps["a_tell"].detail, "17 tracks filed, all verified");
    assert_eq!(s.headline(), "Done");
}

#[test]
fn every_step_the_archive_recipe_uses_is_real() {
    let real = drt_steps::real();
    for r in recipes::all() {
        check(&r.root, &real).unwrap_or_else(|e| panic!("{} under the real steps: {e}", r.name));
    }
}

#[tokio::test]
async fn tell_me_says_what_needs_you() {
    let e = engine(Some(9));
    let run = e.start(&recipes::archive()).unwrap();
    let (s, _) = drive(run, Policy { pick: 0, retry: "leave_out" }).await;
    assert_eq!(s.steps["a_tell"].detail, "16 tracks filed, all verified; 1 skipped");
}

#[tokio::test]
async fn two_drives_run_at_once() {
    let e = engine(None);
    let a = e.start(&recipes::cd()).unwrap();
    let b = e.start(&recipes::archive()).unwrap();
    let (sa, sb) = tokio::join!(drive(a, Policy { pick: 0, retry: "keep" }), drive(b, Policy { pick: 0, retry: "keep" }));
    assert_eq!(ok_items(&sa.0, "lib2"), 17);
    assert_eq!(ok_items(&sb.0, "a_mlib"), 17);
}

#[test]
fn recipes_save_as_json_and_load_back() {
    for r in recipes::all() {
        let json = serde_json::to_string(&r).unwrap();
        let back: Recipe = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}

// ---- data discs -----------------------------------------------------------------------------

#[test]
fn the_data_disc_recipe_type_checks_and_its_steps_follow_from_one_another() {
    use drt_engine::{fits_flow, flow_after};
    let reg = simulated(None);
    let r = recipes::data().root;
    assert_eq!(check(&r, &reg).unwrap(), ItemKind::Any);
    let names = |id: &str| fits_flow(&flow_after(&r, id, &reg).unwrap(), &reg).iter().map(|i| i.name).collect::<Vec<_>>();
    let after_drive = names("d_drive");
    // Image and CD Rip are both *Rip* in the window: told apart by kind.
    let kinds = |id: &str| fits_flow(&flow_after(&r, id, &reg).unwrap(), &reg).iter().map(|i| i.kind).collect::<Vec<_>>();
    assert!(after_drive.contains(&"Data disc") && after_drive.contains(&"Audio CD") && !kinds("d_drive").contains(&"image"), "{after_drive:?}");
    let after_kind = names("d_disc");
    assert!(kinds("d_disc").contains(&"image") && !after_kind.contains(&"MusicBrainz") && !kinds("d_disc").contains(&"rip"), "{after_kind:?}");
    let after_image = names("d_image");
    assert!(after_image.contains(&"Check") && after_image.contains(&"CHD") && after_image.contains(&"Plain image"), "{after_image:?}");
    assert!(!after_image.contains(&"FLAC") && !after_image.contains(&"Library"), "{after_image:?}");
    assert!(names("d_chd").contains(&"Folder"));
    // Next disc once the disc is imaged, not before.
    assert!(!after_kind.contains(&"Next disc") && after_image.contains(&"Next disc"), "{after_image:?}");
    // Image can't follow an audio CD, nor Rip a data disc.
    let bad = chain([Node::step("d", "drive"), Node::step("cd", "audio-cd"), Node::step("i", "image")]);
    assert!(check(&bad, &reg).is_err());
    let bad = chain([Node::step("d", "drive"), Node::step("dd", "data-disc"), Node::step("r", "rip")]);
    assert!(check(&bad, &reg).is_err());
}

#[test]
fn image_and_chd_share_their_settings_with_the_real_steps() {
    let (sim, real) = (simulated(None), drt_steps::real());
    for kind in ["data-disc", "image", "check", "chd", "keep", "folder", "discogs", "gnudb", "lyrics", "lyrics-file", "artwork-files", "cue", "replaygain", "tell", "cover-art", "flac", "mp3"] {
        let (a, b) = (sim.info(kind).unwrap(), real.info(kind).unwrap());
        let keys = |i: &drt_engine::StepInfo| i.params.iter().map(|p| p.key).collect::<Vec<_>>();
        assert_eq!((a.input, a.output, keys(&a)), (b.input, b.output, keys(&b)), "{kind}");
        assert_eq!(a.retries, b.retries, "{kind}");
    }
}

#[tokio::test]
async fn a_data_disc_goes_all_the_way_and_redump_names_it() {
    let e = engine(None);
    let run = e.start(&recipes::data()).unwrap();
    let (s, asked) = drive(run, Policy { pick: 0, retry: "keep" }).await;
    assert!(asked.is_empty(), "a data disc asks nothing: {asked:?}");
    assert_eq!(s.steps["d_disc"].status, "ENCARTA98");
    assert_eq!(s.steps["d_disc"].detail, "ISO 9660 · 682 MB");
    let checked = &s.steps["d_check"].items[0];
    assert_eq!(checked.note, "Verified: Encarta 98 Encyclopedia (USA) (Disc 1)");
    assert_eq!(s.steps["d_check"].status, "Matches Redump");
    assert!(checked.verified, "the catalogue's hashes are independent of the read");
    // Filed in a folder named for what Redump calls it, not the label.
    let filed = &s.steps["d_lib"].items[0];
    assert_eq!(filed.state, ItemState::Ok);
    assert_eq!(filed.note, "~/Images/Encarta 98 Encyclopedia (USA) (Disc 1)");
    for step in ["d_image", "d_check", "d_chd", "d_lib"] {
        assert_eq!(s.steps[step].state, StepState::Ok, "{step}");
    }
    assert_eq!(s.steps["d_next"].status, "Ejected");
    assert_eq!(s.headline(), "Done");
}

#[tokio::test]
async fn bad_sectors_can_be_refined() {
    let e = engine(Some(1));
    let run = e.start(&recipes::data()).unwrap();
    let (s, _) = drive(run, Policy { pick: 0, retry: "refine" }).await;
    let image = &s.steps["d_image"].items[0];
    assert_eq!((image.state, image.note.as_str()), (ItemState::Ok, "✓ 682 MB, bad parts re-read"));
    assert_eq!(s.steps["d_lib"].count(ItemState::Ok), 1);
}

#[tokio::test]
async fn a_disc_left_out_is_not_filed() {
    let e = engine(Some(1));
    let run = e.start(&recipes::data()).unwrap();
    let (s, _) = drive(run, Policy { pick: 0, retry: "leave_out" }).await;
    assert_eq!(s.steps["d_image"].count(ItemState::Skipped), 1);
    assert_eq!(s.steps["d_lib"].count(ItemState::Ok), 0);
    assert_eq!(s.steps["d_check"].count(ItemState::Ok), 0);
}

/// As `drive`, and what the run found and its tools printed.
async fn drive_keeping(run: Run, policy: Policy) -> (Snapshot, drt_engine::Context, std::collections::BTreeMap<String, Vec<String>>) {
    let mut rx = run.snapshot.clone();
    let (mut asked, mut approved) = (false, Vec::new());
    loop {
        let snap = rx.borrow_and_update().clone();
        if snap.finished {
            break;
        }
        if let Some((step, _)) = &snap.question {
            if !asked {
                asked = true;
                run.send(Command::Answer { step: step.clone(), answer: Answer::Pick(policy.pick) }).await;
            }
        }
        for (id, v) in &snap.steps {
            if v.state == StepState::Hold && v.detail == "All here" && !approved.contains(id) {
                approved.push(id.clone());
                run.send(Command::Approve { step: id.clone() }).await;
            }
        }
        if rx.changed().await.is_err() {
            break;
        }
    }
    let (ctx, raw) = (run.context(), run.raw_all());
    (run.finished().await, ctx, raw)
}

#[tokio::test]
async fn every_step_keeps_a_record_of_its_run() {
    let e = engine(None);
    let (s, ctx, raw) = drive_keeping(e.start(&recipes::cd()).unwrap(), Policy { pick: 1, retry: "keep" }).await;
    // What each lookup offered is marked as its own: CD-Text offers the
    // disc's own titles as one candidate, and adds no loose facts.
    assert!(ctx.offered_by("mb").count() == 2 && ctx.offered_by("mb").all(|c| c.facts.iter().all(|f| f.step == "mb")));
    let cdtext: Vec<_> = ctx.offered_by("cdtext").collect();
    assert_eq!(cdtext.len(), 1);
    assert_eq!(cdtext[0].facts.iter().filter(|f| f.key == "title").count(), 17);
    assert_eq!(ctx.found_by("cdtext").count(), 0);
    // Choose says what reached it, what it chose and why.
    let options: Vec<&str> = s.steps["choose"].record.options.iter().flatten().map(|c| c.id.as_str()).collect();
    assert_eq!(options, ["mb-1999", "mb-2002", "cd-text"]);
    let d = s.steps["choose"].record.decided.clone().unwrap();
    assert_eq!((d.chosen.as_deref(), d.why.as_str()), (Some("mb-2002"), "you picked"));
    // In and out, by kind.
    let rip = &s.steps["rip"].record;
    assert!(rip.ins[0].kind.is_disc() && rip.ins[0].count == 1, "{:?}", rip.ins);
    assert_eq!((rip.outs[0].kind, rip.outs[0].count), (ItemKind::Track, 17));
    assert!(rip.started.is_some() && rip.finished >= rip.started);
    // The files an encoder and a folder wrote, with sizes; Hold passes
    // files through and wrote none.
    assert_eq!(s.steps["flac"].record.files.len(), 17);
    assert!(s.steps["flac"].record.files.iter().all(|f| f.size > 1_000_000));
    assert_eq!(s.steps["lib1"].record.files.len(), 17);
    assert!(s.steps["hold"].record.files.is_empty());
    // Its log, and its tool's output.
    assert!(!s.steps["mb"].record.log.is_empty());
    assert!(raw["rip"].iter().any(|l| l.contains("Accurip")));
    assert!(!raw.contains_key("choose"), "no tool, no output");
}

#[tokio::test]
async fn choose_says_why_it_needed_no_question() {
    // Discogs alone: one release, so no question, and the reason says so.
    let mut e = engine(None);
    e.time_scale = 0.0;
    let r = Recipe {
        name: "One source".into(),
        root: chain([Node::step("drive", "drive").with("drive", recipes::BLU_RAY), Node::step("cd", "audio-cd"), Node::step("dg", "discogs"), Node::step("choose", "choose")]),
    };
    let s = e.start(&r).unwrap().finished().await;
    let d = s.steps["choose"].record.decided.clone().unwrap();
    assert_eq!((d.chosen.as_deref(), d.why.as_str()), (Some("dg-1"), "only one"));
}

/// A card has two settings rows: no step marks more for it, whatever mode
/// its settings are in (MP3's CBR shows its bitrate instead of its quality).
#[test]
fn no_card_shows_more_than_two_settings() {
    for reg in [drt_steps::real(), simulated(None)] {
        for info in reg.infos() {
            // Each value of every setting another's `when` depends on.
            let modes: Vec<(&str, Vec<String>)> = info
                .params
                .iter()
                .filter(|p| info.params.iter().any(|q| q.when.is_some_and(|(k, _)| k == p.key)))
                .map(|p| {
                    (
                        p.key,
                        match &p.kind {
                            drt_engine::ParamKind::Choice(o) => o.iter().map(|s| s.to_string()).collect(),
                            _ => vec![],
                        },
                    )
                })
                .collect();
            let mut cases: Vec<Vec<(&str, String)>> = vec![vec![]];
            for (k, vals) in &modes {
                cases = cases
                    .into_iter()
                    .flat_map(|c| {
                        vals.iter().map(move |v| {
                            let mut c = c.clone();
                            c.push((*k, v.clone()));
                            c
                        })
                    })
                    .collect();
            }
            for case in cases {
                let get = |k: &str| case.iter().find(|(x, _)| *x == k).map(|(_, v)| Value::from(v.clone())).or_else(|| info.params.iter().find(|p| p.key == k).and_then(|p| p.default.clone()));
                let n = info.params.iter().filter(|p| p.on_card.is_some() && p.applies(get)).count();
                assert!(n <= 2, "{} shows {n} settings on its card in {case:?}", info.kind);
            }
        }
    }
}
