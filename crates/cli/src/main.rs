//! `tripr`: runs a recipe without a window.
//!
//! Prints the run's tiles as they change and asks its questions on the
//! terminal. Press Enter for the preselected answer. `--auto` answers
//! everything with the default, approves holds and keeps bad tracks.

use anyhow::{bail, Result};
use serde_json::Value;
use std::io::{IsTerminal, Write};
use std::time::Duration;
use tokio::sync::mpsc;
use trip_engine::snapshot::QuestionShape;
use trip_engine::{Answer, Command, Engine, ItemState, Node, Recipe, Setting, Snapshot, StepState, StepView};
use trip_steps::{recipes, simulated};

const HELP: &str = "tripr [--simulate] [--recipe cd|archive] [--out DIR] [--tracks 3,5] [--offset N]
    [--bad TRACK] [--speed N] [--auto] [--no-offset] [--log] [--drive NAME]
    [--next-disc]

  --simulate   simulated steps: no drive, no network, no files
  --recipe     cd (default) or archive, the most complex one
  --out DIR    file into DIR/<step> instead of the recipe's folders
  --tracks     rip only these tracks
  --offset     the drive's read offset, if it isn't known yet
  --bad        a track that fails to verify on the first read
  --speed      how many times faster than real time (default 4)
  --auto       answer every question with its default
  --no-offset  start with no read offset known, to see setup
  --log        print the log at the end
  --drive      the drive to read from, as the app names it; by default the
               recipe's, else the first one connected
  --next-disc  open the tray when done and wait for the next disc; without
               it the disc stays in

tripr fix-tags FOLDER [--apply]

  Mends files filed before the audio audit of 20 November 2026: removes a
  comment that is exactly cyanrip's name and version, and, in those files,
  marks a lone picture of type Other as the front cover. Tags only; each
  file is checked (audio and every other tag the same) before it replaces
  the original. Without --apply it only lists what it would change.

tripr rename-full-copies FOLDER [--dat FILE]... [--apply]

  Renames plain images (a cue sheet and its tracks, with their log) to
  Redump's names, when every track's SHA-1 is a track of one entry in a
  Redump list, and rewrites each cue sheet's FILE lines to match. The
  lists are the ones the app keeps (or each --dat given). Nothing is
  written over. Without --apply it only lists what it would do.";

struct Args {
    recipe: Recipe,
    simulate: bool,
    out: Option<String>,
    tracks: Option<String>,
    offset: Option<i64>,
    bad: Option<u32>,
    speed: f64,
    auto: bool,
    no_offset: bool,
    log: bool,
    drive: Option<String>,
    next_disc: bool,
}

fn args() -> Result<Args> {
    let mut a = Args { recipe: recipes::cd(), simulate: false, out: None, tracks: None, offset: None, bad: None, speed: 4.0, auto: false, no_offset: false, log: false, drive: None, next_disc: false };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--simulate" => a.simulate = true,
            "--out" => a.out = it.next(),
            "--tracks" => a.tracks = it.next(),
            "--offset" => a.offset = Some(it.next().unwrap_or_default().parse()?),
            "--recipe" => {
                a.recipe = match it.next().as_deref() {
                    Some("cd") => recipes::cd(),
                    Some("archive") => recipes::archive(),
                    other => bail!("no recipe {other:?}\n\n{HELP}"),
                }
            }
            "--bad" => a.bad = Some(it.next().unwrap_or_default().parse()?),
            "--speed" => a.speed = it.next().unwrap_or_default().parse()?,
            "--auto" => a.auto = true,
            "--no-offset" => a.no_offset = true,
            "--log" => a.log = true,
            "--drive" => a.drive = it.next(),
            "--next-disc" => a.next_disc = true,
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0)
            }
            other => bail!("unknown argument {other}\n\n{HELP}"),
        }
    }
    if !a.next_disc {
        // One disc: leave it in the drive.
        let next: Vec<String> = a
            .recipe
            .root
            .steps()
            .into_iter()
            .filter_map(|n| match n {
                Node::Step { id, step, .. } if step == "next-disc" => Some(id.clone()),
                _ => None,
            })
            .collect();
        for id in next {
            a.recipe.root.remove(&id);
        }
    }
    if !a.simulate {
        // The built-in recipes name a drive by kind; read from the one asked
        // for, else the recipe's if it's here, else the first connected.
        let here: Vec<String> = trip_steps::real::drives().into_iter().map(|(name, _)| name).collect();
        let named = Engine::drive_of(&a.recipe);
        let pick = a.drive.clone().or_else(|| (!here.contains(&named)).then(|| here.first().cloned()).flatten());
        if let Some(name) = pick {
            let ids: Vec<String> = a
                .recipe
                .root
                .steps()
                .into_iter()
                .filter_map(|n| match n {
                    Node::Step { id, step, .. } if step == "drive" => Some(id.clone()),
                    _ => None,
                })
                .collect();
            for id in ids {
                if let Some(Node::Step { params, .. }) = a.recipe.root.find_mut(&id) {
                    params.insert("drive".into(), Setting::chosen(Value::from(name.clone())));
                }
            }
        }
    }
    if let Some(out) = &a.out {
        redirect(&mut a.recipe.root, out);
    }
    if let Some(t) = &a.tracks {
        set_param(&mut a.recipe.root, "rip", "tracks", t);
    }
    Ok(a)
}

/// Every folder step files into `out/<step id>` instead.
fn redirect(n: &mut Node, out: &str) {
    match n {
        Node::Step { id, params, .. } => {
            if params.contains_key("folder") {
                params.insert("folder".into(), Setting::chosen(Value::from(format!("{out}/{id}"))));
            }
        }
        Node::Chain { nodes } => nodes.iter_mut().for_each(|n| redirect(n, out)),
        Node::Branches { branches } => branches.iter_mut().for_each(|n| redirect(n, out)),
        Node::Group { body, .. } => redirect(body, out),
    }
}

fn set_param(n: &mut Node, kind: &str, key: &str, value: &str) {
    match n {
        Node::Step { step, params, .. } if step == kind => {
            params.insert(key.into(), Setting::chosen(Value::from(value)));
        }
        Node::Step { .. } => {}
        Node::Chain { nodes } => nodes.iter_mut().for_each(|n| set_param(n, kind, key, value)),
        Node::Branches { branches } => branches.iter_mut().for_each(|n| set_param(n, kind, key, value)),
        Node::Group { body, .. } => set_param(body, kind, key, value),
    }
}

/// `tripr fix-tags FOLDER [--apply]`. Gives how many files it changed or
/// would change, and how many it couldn't.
async fn fix_tags(folder: &std::path::Path, apply: bool, out: &mut impl Write) -> Result<(usize, usize)> {
    use trip_steps::real::fixtags;
    let (mut changed, mut failed) = (0, 0);
    for f in fixtags::files(folder)? {
        let shown = f.strip_prefix(folder).unwrap_or(&f).display().to_string();
        let plan = match fixtags::plan(&f) {
            Ok(p) => p,
            Err(e) => {
                writeln!(out, "couldn't read   {shown}: {e}")?;
                failed += 1;
                continue;
            }
        };
        if plan.is_empty() {
            continue;
        }
        if !apply {
            writeln!(out, "would change    {shown}: {}", plan.describe())?;
            changed += 1;
            continue;
        }
        match fixtags::apply(&f, &plan).await {
            Ok(()) => {
                writeln!(out, "changed         {shown}: {}", plan.describe())?;
                changed += 1;
            }
            Err(e) => {
                writeln!(out, "left as it was  {shown}: {e}")?;
                failed += 1;
            }
        }
    }
    let files = |n: usize| if n == 1 { "1 file".to_owned() } else { format!("{n} files") };
    match (apply, changed) {
        (false, 0) => writeln!(out, "Nothing to change.")?,
        (false, n) => writeln!(out, "{} to change. Run again with --apply to change them.", files(n))?,
        (true, n) => writeln!(out, "{} changed.", files(n))?,
    }
    if failed > 0 {
        writeln!(out, "{} left as they were.", files(failed))?;
    }
    Ok((changed, failed))
}

/// `tripr rename-full-copies FOLDER [--dat FILE]... [--apply]`. Gives how
/// many sets it renamed or would rename, and how many it left because
/// something was in the way.
fn rename_full_copies(folder: &std::path::Path, dats: &[std::path::PathBuf], apply: bool, out: &mut impl Write) -> Result<(usize, usize)> {
    use trip_steps::real::renames::{self, Plan};
    let mut lists = vec![];
    for d in dats {
        let bytes = std::fs::read(d).map_err(|e| anyhow::anyhow!("{}: {e}", d.display()))?;
        lists.push(trip_steps::real::redump::parse(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", d.display()))?);
    }
    if lists.is_empty() {
        bail!("no Redump list to match against: rip a game first, or pass --dat FILE");
    }
    let (mut renamed, mut left) = (0, 0);
    for cue in renames::sheets(folder)? {
        let shown = cue.strip_prefix(folder).unwrap_or(&cue).display().to_string();
        let roms = match renames::hash_set(&cue) {
            Ok(r) => r,
            Err(e) => {
                writeln!(out, "left as it is   {shown}: {e}")?;
                left += 1;
                continue;
            }
        };
        match renames::plan(&cue, &roms, renames::find(&lists, &roms)) {
            Plan::AlreadyNamed { .. } => {}
            Plan::NoMatch => writeln!(out, "no match        {shown}: not in the Redump lists, left as it is")?,
            Plan::Left(why) => {
                writeln!(out, "left as it is   {shown}: {why}")?;
                left += 1;
            }
            Plan::Rename { game, moves } => {
                let said = moves.iter().filter(|(a, b)| a != b).map(|(a, b)| format!("\n    {} -> {}", name(a), name(b))).collect::<String>();
                if !apply {
                    writeln!(out, "would rename    {shown} as {game}:{said}")?;
                    renamed += 1;
                    continue;
                }
                match renames::apply(&moves) {
                    Ok(()) => {
                        writeln!(out, "renamed         {shown} as {game}:{said}")?;
                        renamed += 1;
                    }
                    Err(e) => {
                        writeln!(out, "couldn't rename {shown}: {e}")?;
                        left += 1;
                    }
                }
            }
        }
    }
    let sets = |n: usize| if n == 1 { "1 set".to_owned() } else { format!("{n} sets") };
    match (apply, renamed) {
        (_, 0) => writeln!(out, "Nothing to rename.")?,
        (false, n) => writeln!(out, "{} to rename. Run again with --apply to rename them.", sets(n))?,
        (true, n) => writeln!(out, "{} renamed.", sets(n))?,
    }
    if left > 0 {
        writeln!(out, "{} left as they were.", sets(left))?;
    }
    Ok((renamed, left))
}

fn name(p: &std::path::Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut argv = std::env::args().skip(1);
    let first = argv.next();
    if first.as_deref() == Some("rename-full-copies") {
        let rest: Vec<String> = argv.collect();
        let (mut apply, mut dats, mut folders) = (false, vec![], vec![]);
        let mut it = rest.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--apply" => apply = true,
                "--dat" => dats.push(std::path::PathBuf::from(it.next().ok_or_else(|| anyhow::anyhow!("--dat needs a file"))?)),
                s if s.starts_with("--") => bail!("unknown argument {s}\n\n{HELP}"),
                s => folders.push(s.to_owned()),
            }
        }
        let [folder] = folders.as_slice() else { bail!("tripr rename-full-copies FOLDER [--dat FILE]... [--apply]") };
        if dats.is_empty() {
            dats = trip_steps::real::renames::cached_lists();
        }
        let (_, left) = rename_full_copies(std::path::Path::new(folder), &dats, apply, &mut std::io::stdout())?;
        if left > 0 {
            std::process::exit(1);
        }
        return Ok(());
    }
    if first.as_deref() == Some("fix-tags") {
        let rest: Vec<String> = argv.collect();
        let apply = rest.iter().any(|a| a == "--apply");
        let folders: Vec<&String> = rest.iter().filter(|a| !a.starts_with("--")).collect();
        let (Some(folder), 1) = (folders.first(), folders.len()) else { bail!("tripr fix-tags FOLDER [--apply]") };
        if let Some(bad) = rest.iter().find(|a| a.starts_with("--") && *a != "--apply") {
            bail!("unknown argument {bad}\n\n{HELP}");
        }
        let (_, failed) = fix_tags(std::path::Path::new(folder.as_str()), apply, &mut std::io::stdout()).await?;
        if failed > 0 {
            std::process::exit(1);
        }
        return Ok(());
    }
    let a = args()?;
    let mut engine = if a.simulate { Engine::new(simulated(a.bad)) } else { Engine::new(trip_steps::real()) };
    engine.time_scale = if a.simulate { 1.0 / a.speed.max(0.01) } else { 1.0 };
    if let Some(o) = a.offset {
        engine.settings.lock().unwrap().set_drive(&Engine::drive_of(&a.recipe), "offset", Setting::chosen(Value::from(o)));
    }
    if a.simulate && !a.no_offset {
        let mut s = engine.settings.lock().unwrap();
        s.set_drive(recipes::BLU_RAY, "offset", Setting::detected(Value::from(103), "AccurateRip"));
        s.set_drive(recipes::DVD, "offset", Setting::detected(Value::from(6), "AccurateRip"));
    }

    // On a real drive, looked up as the app does (AccurateRip's list of
    // drives) before anything is asked.
    let missing = if a.simulate { engine.missing(&a.recipe) } else { engine.find_missing(&a.recipe, false).await };
    if !missing.is_empty() {
        print!("{}", frame(&engine.idle(&a.recipe)));
        for m in missing {
            let label = m.label;
            let line = if a.auto { String::new() } else { prompt(&format!("{label} for {}? ", m.step))? };
            let Some(value) = answer_or_default(line.trim(), a.simulate)? else { bail!("{label} for {} isn't known; pass --offset", m.step) };
            engine.settings.lock().unwrap().set_drive(&Engine::drive_of(&a.recipe), m.key, Setting::chosen(value));
        }
    }

    let run = engine.start(&a.recipe)?;
    let lines = stdin_lines();
    let tty = std::io::stdout().is_terminal();
    let mut rx = run.snapshot.clone();
    let mut asked: Option<Prompt> = None;
    let mut settled: Vec<String> = Vec::new();
    let mut lines = lines;
    loop {
        let snap = rx.borrow_and_update().clone();
        if tty {
            print!("\x1b[H\x1b[2J");
        }
        print!("{}", frame(&snap));
        if snap.finished {
            break;
        }
        // One thing to ask at a time: a question, then a bad item, then a hold.
        if asked.is_none() {
            asked = next_prompt(&snap, &settled);
            if let Some(p) = &asked {
                if a.auto {
                    run.send(p.default.clone()).await;
                    settled.push(p.key.clone());
                    asked = None;
                }
            }
        }
        if let Some(p) = &asked {
            println!("\n{}", p.text);
            let _ = std::io::stdout().flush();
        }
        tokio::select! {
            changed = rx.changed() => if changed.is_err() { break },
            line = lines.recv(), if asked.is_some() => {
                let p = asked.take().unwrap();
                let line = line.unwrap_or_default();
                run.send(p.answer(line.trim())).await;
                settled.push(p.key);
            }
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    let log = run.log();
    let snap = run.finished().await;
    println!("\n{}", snap.headline());
    if a.log {
        println!("\nLog:");
        for l in log {
            println!("  {l}");
        }
    }
    Ok(())
}

/// A missing setting's value from what was typed. Nothing typed takes the
/// simulated drive's +103 in a simulation, and nothing on a real drive:
/// reading at a guessed offset gives a clean rip AccurateRip can't confirm.
fn answer_or_default(line: &str, simulate: bool) -> Result<Option<Value>> {
    if line.is_empty() {
        return Ok(simulate.then(|| Value::from(103)));
    }
    Ok(Some(Value::from(line.parse::<i64>()?)))
}

struct Prompt {
    key: String,
    text: String,
    default: Command,
    /// For a numbered choice: the command for each number, from 1.
    choices: Vec<Command>,
}

impl Prompt {
    fn answer(&self, line: &str) -> Command {
        match line.parse::<usize>() {
            Ok(n) if n >= 1 && n <= self.choices.len() => self.choices[n - 1].clone(),
            _ => self.default.clone(),
        }
    }
}

fn next_prompt(snap: &Snapshot, settled: &[String]) -> Option<Prompt> {
    if let Some((step, q)) = &snap.question {
        let key = format!("q:{step}:{}", q.prompt);
        if !settled.contains(&key) {
            if let QuestionShape::PickOne { options, preselected, .. } = &q.shape {
                let mut text = format!("{}  ({})", q.prompt, snap.steps[step].name);
                for (i, o) in options.iter().enumerate() {
                    text += &format!("\n  {} {}  {}  {}", i + 1, if i == *preselected { "●" } else { "○" }, o.label, o.detail);
                }
                text += "\nEnter for the one marked ●, or a number: ";
                let choices = (0..options.len()).map(|i| Command::Answer { step: step.clone(), answer: Answer::Pick(i) }).collect();
                return Some(Prompt { key, text, default: Command::Answer { step: step.clone(), answer: Answer::Pick(*preselected) }, choices });
            }
        }
    }
    for id in &snap.order {
        let v = &snap.steps[id];
        if let Some(item) = v.items.iter().find(|i| i.state == ItemState::Bad) {
            let key = format!("r:{id}:{}", item.id);
            if settled.contains(&key) {
                continue;
            }
            let options = [
                ("two_reads", "Read again until two reads agree"),
                ("other_drive", "Try the other drive"),
                ("slowly", "Read again slowly"),
                ("keep", "Keep it anyway"),
                ("leave_out", "Skip this track"),
            ];
            let mut text = format!("{} {}: {}", item.label, item.note, v.name);
            for (i, (_, label)) in options.iter().enumerate() {
                text += &format!("\n  {} {}", i + 1, label);
            }
            text += "\nEnter to re-read until two reads agree, or a number: ";
            let cmd = |o: &str| Command::Retry { step: id.clone(), item: item.id.clone(), option: o.into() };
            return Some(Prompt { key, text, default: cmd("two_reads"), choices: options.iter().map(|(o, _)| cmd(o)).collect() });
        }
        if v.state == StepState::Hold && v.detail == "All here" {
            let key = format!("h:{id}");
            if !settled.contains(&key) {
                let text = format!("{} holds {} for you. Enter to let them through: ", v.name, v.detail);
                let c = Command::Approve { step: id.clone() };
                return Some(Prompt { key, text, default: c.clone(), choices: vec![c] });
            }
        }
    }
    None
}

fn frame(s: &Snapshot) -> String {
    let mut out = format!("{}  ·  {}\n\n", s.recipe, s.headline());
    for id in &s.order {
        out += &line(&s.steps[id]);
    }
    out
}

fn line(v: &StepView) -> String {
    let icon = match v.state {
        StepState::Idle => "○",
        StepState::Busy => "◐",
        StepState::Ok => "✓",
        StepState::Needs => "!",
        StepState::Wait => "◷",
        StepState::Hold => "⏸",
        StepState::Bad => "✕",
    };
    let bar = match v.progress {
        Some(p) => {
            let n = (p * 10.0).round() as usize;
            format!("{}{}", "█".repeat(n.min(10)), "░".repeat(10 - n.min(10)))
        }
        None => " ".repeat(10),
    };
    // The outcome with its qualifier: *17 tracks 43:45*, *The White Stripes 1999*.
    let outcome = if v.qualifier.is_empty() { v.status.clone() } else { format!("{} {}", v.status, v.qualifier) };
    format!("  {icon} {:<12} {:<20} {:<26} {bar}  {}\n", v.name, outcome, v.detail, v.from)
}

fn prompt(text: &str) -> Result<String> {
    print!("{text}");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s)
}

/// Lines typed on the terminal, read on a thread of their own.
fn stdin_lines() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(8);
    std::thread::spawn(move || {
        let mut s = String::new();
        while std::io::stdin().read_line(&mut s).map(|n| n > 0).unwrap_or(false) {
            if tx.blocking_send(std::mem::take(&mut s)).is_err() {
                break;
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_copies_are_listed_then_renamed_to_their_redump_names() {
        let dir = std::env::current_dir().unwrap().join("../../target/trip-test/rename-full-copies");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("PlayStation/Koudelka")).unwrap();
        let set = dir.join("PlayStation/Koudelka");
        std::fs::write(set.join("KOUDELKA_1.cue"), "FILE \"KOUDELKA_1.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n").unwrap();
        std::fs::write(set.join("KOUDELKA_1.bin"), "sectors").unwrap();
        std::fs::write(set.join("KOUDELKA_1.log"), "log").unwrap();
        // One nobody lists.
        std::fs::write(dir.join("other.cue"), "FILE \"other.bin\" BINARY\n").unwrap();
        std::fs::write(dir.join("other.bin"), "unknown").unwrap();
        let sha1 = trip_steps::real::hash::hash_files(&[&set.join("KOUDELKA_1.bin")], |_| {}, || false).unwrap().remove(0).sha1;
        let dat = dir.join("psx.dat");
        std::fs::write(
            &dat,
            format!("<?xml version=\"1.0\"?><datafile><header><name>Sony - PlayStation</name><version>1</version></header><game name=\"Koudelka (Europe) (Disc 1)\"><rom name=\"Koudelka (Europe) (Disc 1).cue\" size=\"1\" crc=\"0\" md5=\"0\" sha1=\"0\"/><rom name=\"Koudelka (Europe) (Disc 1).bin\" size=\"7\" crc=\"0\" md5=\"0\" sha1=\"{sha1}\"/></game></datafile>"),
        )
        .unwrap();
        let mut said = vec![];
        assert_eq!(rename_full_copies(&dir, std::slice::from_ref(&dat), false, &mut said).unwrap(), (1, 0));
        let said = String::from_utf8(said).unwrap();
        assert!(said.contains("would rename") && said.contains("KOUDELKA_1.bin -> Koudelka (Europe) (Disc 1).bin") && said.contains("no match"), "{said}");
        assert!(set.join("KOUDELKA_1.bin").is_file(), "listing changes nothing");
        assert_eq!(rename_full_copies(&dir, std::slice::from_ref(&dat), true, &mut vec![]).unwrap(), (1, 0));
        assert!(std::fs::read_to_string(set.join("Koudelka (Europe) (Disc 1).cue")).unwrap().starts_with("FILE \"Koudelka (Europe) (Disc 1).bin\" BINARY\n"));
        assert!(set.join("Koudelka (Europe) (Disc 1).log").is_file() && dir.join("other.bin").is_file());
        assert_eq!(rename_full_copies(&dir, &[dat], true, &mut vec![]).unwrap(), (0, 0), "once is enough");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A folder of files as they were filed before the audit, made the way
    /// they were made then: ffmpeg copying the rip WAV's INFO comment, and
    /// the cover with no type. Beside them, files that aren't ours.
    fn library(dir: &std::path::Path) -> bool {
        let ff = |args: &[&str]| std::process::Command::new("ffmpeg").args(["-nostdin", "-v", "error", "-y"]).args(args).current_dir(dir).status().is_ok_and(|s| s.success());
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir.join("Artist/Album")).unwrap();
        if !ff(&["-f", "lavfi", "-i", "sine=f=440:d=1", "-ac", "2", "-ar", "44100", "-c:a", "pcm_s16le", "-metadata", "comment=cyanrip 0.9.3", "rip.wav"]) {
            return false;
        }
        ff(&["-f", "lavfi", "-i", "color=red:s=32x32", "-frames:v", "1", "cover.jpg"]);
        let cover = ["-i", "cover.jpg", "-map", "0:a", "-map", "1:v", "-c:v", "copy", "-disposition:v", "attached_pic"];
        let old = |codec: &[&str], with_cover: bool, extra: &[&str], out: &str| {
            let mut a: Vec<&str> = vec!["-i", "rip.wav"];
            if with_cover {
                a.extend(cover);
            }
            a.extend(codec);
            a.extend(["-metadata", "title=Total Football", "-metadata", "track=1/13"]);
            a.extend(extra);
            a.push(out);
            assert!(ff(&a), "{out}");
        };
        old(&["-c:a", "flac"], true, &[], "Artist/Album/01.flac");
        old(&["-c:a", "libmp3lame", "-q:a", "0", "-id3v2_version", "3"], true, &[], "Artist/Album/01.mp3");
        old(&["-c:a", "libopus"], false, &[], "Artist/Album/01.opus");
        old(&["-c:a", "libvorbis"], false, &[], "Artist/Album/01.ogg");
        old(&["-c:a", "alac"], true, &[], "Artist/Album/01.m4a");
        old(&["-c:a", "wavpack"], false, &[], "Artist/Album/01.wv");
        old(&["-c:a", "pcm_s16le"], false, &[], "Artist/Album/01.wav");
        // Not ours: a cover of type Other and no signature, and a comment
        // of someone's own.
        old(&["-c:a", "flac", "-map_metadata", "-1"], true, &[], "Artist/Album/theirs.flac");
        old(&["-c:a", "flac", "-map_metadata", "-1"], false, &["-metadata", "comment=cyanrip 0.9.3 was used"], "Artist/Album/notes.flac");
        std::fs::remove_file(dir.join("rip.wav")).unwrap();
        std::fs::remove_file(dir.join("cover.jpg")).unwrap();
        true
    }

    fn snapshot(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<(String, Vec<u8>)> = trip_steps::real::fixtags::files(dir).unwrap().into_iter().map(|f| (f.display().to_string(), std::fs::read(&f).unwrap())).collect();
        v.sort();
        v
    }

    fn probe(f: &std::path::Path) -> String {
        let o = std::process::Command::new("ffprobe").args(["-v", "error", "-show_entries", "stream_tags:format_tags", "-of", "compact"]).arg(f).output().unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    fn md5(f: &std::path::Path) -> String {
        let o = std::process::Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(f).args(["-map", "0:a", "-c:a", "pcm_s16le", "-f", "md5", "-"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    #[tokio::test]
    async fn fix_tags_lists_by_default_and_changes_only_the_two_things_with_apply() {
        let dir = std::env::current_dir().unwrap().join("../../target/trip-test/fix-tags");
        if !library(&dir) {
            eprintln!("no ffmpeg: skipped");
            return;
        }
        let before = snapshot(&dir);
        let mut out = Vec::new();
        let got = fix_tags(&dir, false, &mut out).await.unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(got, (7, 0), "{text}");
        assert!(text.contains("would change    Artist/Album/01.flac: remove the cyanrip comment; mark the picture as the front cover\n"), "{text}");
        assert!(text.contains("would change    Artist/Album/01.mp3: remove the cyanrip comment; mark the picture as the front cover\n"), "{text}");
        assert!(text.contains("would change    Artist/Album/01.m4a: remove the cyanrip comment\n"), "{text}");
        assert!(text.contains("would change    Artist/Album/01.opus: remove the cyanrip comment\n"), "{text}");
        assert!(!text.contains("theirs") && !text.contains("notes"), "{text}");
        assert!(text.ends_with("7 files to change. Run again with --apply to change them.\n"), "{text}");
        assert_eq!(snapshot(&dir), before, "a dry run changes nothing");

        let audio: Vec<String> = trip_steps::real::fixtags::files(&dir).unwrap().iter().map(|f| md5(f)).collect();
        let tags: Vec<String> = trip_steps::real::fixtags::files(&dir).unwrap().iter().map(|f| probe(f)).collect();
        let mut out = Vec::new();
        // A relative folder, as typed: the tools run elsewhere.
        let rel = std::path::Path::new("../../target/trip-test/fix-tags");
        assert_eq!(fix_tags(rel, true, &mut out).await.unwrap(), (7, 0), "{}", String::from_utf8_lossy(&out));
        let files = trip_steps::real::fixtags::files(&dir).unwrap();
        for (i, f) in files.iter().enumerate() {
            assert_eq!(md5(f), audio[i], "{f:?}: the audio never changes");
            let name = f.file_name().unwrap().to_string_lossy();
            let want = if name.starts_with("01.") { tags[i].replace("|tag:comment=cyanrip 0.9.3", "").replace("tag:comment=Other", "tag:comment=Cover (front)") } else { tags[i].clone() };
            assert_eq!(probe(f), want, "{f:?}: only the two changes");
            assert!(trip_steps::real::fixtags::plan(f).unwrap().is_empty(), "{f:?}");
        }
        let after = snapshot(&dir);
        for (b, a) in before.iter().zip(&after) {
            if b.0.contains("theirs") || b.0.contains("notes") {
                assert_eq!(b, a, "not ours: untouched, byte for byte");
            }
        }
        assert!(!std::fs::read_dir(dir.join("Artist/Album")).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("fix-tags")), "no copies left behind");

        // Run again: nothing left to do.
        let mut out = Vec::new();
        assert_eq!(fix_tags(&dir, true, &mut out).await.unwrap(), (0, 0));
        // A plan that no longer fits the file is refused, and the file kept.
        let f = dir.join("Artist/Album/theirs.flac");
        let bytes = std::fs::read(&f).unwrap();
        let wrong = trip_steps::real::fixtags::Plan { signature: true, front_cover: true };
        assert!(trip_steps::real::fixtags::apply(&f, &wrong).await.is_err());
        assert_eq!(std::fs::read(&f).unwrap(), bytes);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_real_drive_never_gets_a_guessed_offset() {
        assert_eq!(answer_or_default("", false).unwrap(), None);
        assert_eq!(answer_or_default("", true).unwrap(), Some(Value::from(103)));
        assert_eq!(answer_or_default("+6", false).unwrap(), Some(Value::from(6)));
        assert!(answer_or_default("six", false).is_err());
    }
}
