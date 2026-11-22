//! A game's Rip that ends with sectors it couldn't read: by default it
//! makes the image anyway, with them left blank (0x55), says so, and hands
//! it to Check and fix, which by itself reads them again from the drive,
//! then rips again and combines, and calls it verified only once it
//! matches Redump. Set to *Ask me*, Rip holds as before. A stand-in
//! redumper (`DRT_REDUMPER`, set once for this test binary) reads the
//! disc; stand-ins for the drive and the later rips serve Check and fix;
//! the "drive" is a plain file, so no drive is touched.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::chain;
use drt_engine::{Answer, Command, Cx, Engine, Group, Inlet, Item, ItemKind, ItemState, Node, Outlet, ParamKind, ParamSpec, Recipe, Run, Snapshot, Step, StepInfo, StepState, Subject};
use drt_steps::real::combine::{edc, msf, SECTOR};
use drt_steps::real::data;
use drt_steps::real::fix::Source;
use drt_steps::real::game::{RipFn, SourceFn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const SECTORS: i64 = 12;
/// The sectors the drive couldn't read in Rip's read.
const UNREAD: [i64; 2] = [3, 7];

/// A form 2 sector at `lba`, with its EDC, as the disc holds it.
fn sector(lba: i64) -> Vec<u8> {
    let mut s = vec![0u8; SECTOR];
    s[1..11].fill(0xFF);
    s[12..15].copy_from_slice(&msf(lba));
    s[15] = 2;
    s[16..24].copy_from_slice(&[0, 0, 0x20, 0, 0, 0, 0x20, 0]);
    for (i, b) in s[24..2348].iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(5) ^ lba as u8;
    }
    let e = edc(&s[16..2348]);
    s[2348..].copy_from_slice(&e.to_le_bytes());
    s
}

/// The disc, with `blank` sectors filled in as redumper fills those it
/// couldn't read.
fn image(blank: &[i64]) -> Vec<u8> {
    (0..SECTORS).flat_map(|l| if blank.contains(&l) { vec![0x55; SECTOR] } else { sector(l) }).collect()
}

fn hex(d: &[u8]) -> String {
    drt_steps::real::hash::hex(d)
}

/// Gives a PlayStation game disc in a "drive" that is a plain file.
struct Disc {
    device: PathBuf,
}

#[async_trait]
impl Step for Disc {
    fn info(&self) -> StepInfo {
        StepInfo::new("drive", "Drive", Group::Source, ItemKind::Nothing, ItemKind::GameDisc).param(ParamSpec::new("drive", "Drive", ParamKind::Text)).gives(&["serial", "label", "system"])
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, _cx: Cx) -> Result<()> {
        inlet.recv().await;
        let mut d = Item::new("disc", ItemKind::GameDisc, "SLES_047.11", None, None);
        let c = d.context_mut();
        c.add(Subject::Disc, "device", self.device.display().to_string(), "the disc");
        for (k, v) in [("label", "SLES_047.11"), ("media", "CD"), ("system", "PlayStation"), ("catalogue", "psx"), ("serial", "SLES-04711")] {
            c.add(Subject::Disc, k, v, "the disc");
        }
        outlet.send(d).await;
        Ok(())
    }
}

/// redumper's stand-in, for a drive it knows: the read leaves errors, the
/// re-read brings none back, and splitting stops at the 2 sectors it
/// couldn't read unless forced, when it makes the image from `made.bin`
/// beside it. `info` finds them filled in. Each command goes in `calls`;
/// what it prints goes in the dump's log too, as redumper's does.
const STAND_IN: &str = r#"#!/bin/sh
here=$(dirname "$0")
cmd=$1; shift
dir=.; name=x; force=no
for a in "$@"; do case $a in --image-path=*) dir=${a#*=};; --image-name=*) name=${a#*=};; --force-split) force=yes;; esac; done
echo "$cmd $*" >> "$here/calls"
mkdir -p "$dir"
log="$dir/$name.log"
{
case "$cmd" in
dump)
  echo "redumper (build: b744)"
  echo "  configuration: PLEXTOR (read offset: +30, C2 shift: 294, pre-gap start: -75, read method: D8, sector order: DATA_SUB_C2)"
  echo "*** DUMP (time check: 0s)"
  echo "disc TOC:"
  echo "  track 1 {  data }"
  echo "    index 01 { LBA:      0, MSF: 00:02:00 }"
  echo "  track A {  data }"
  echo "    index 01 { LBA:     12, MSF: 00:02:12 }"
  : > "$dir/$name.scram"
  echo "media errors: "
  echo "  SCSI: 1176 samples"
  echo "  C2: 0 samples"
  echo "  Q: 0"
  echo "*** END (time check: 1s)"
  ;;
refine)
  echo "*** REFINE (time check: 0s)"
  echo "correction statistics: "
  echo "  SCSI: 0 samples"
  echo "  C2: 0 samples"
  echo "*** END (time check: 1s)"
  ;;
split)
  echo "*** SPLIT (time check: 0s)"
  echo "errors detected, track: 1, sectors: {SKIP: 2, C2: 0}, samples: {SKIP: 1176, C2: 0}"
  if [ $force = no ]; then
    echo "error: data errors detected, unable to continue"
    exit 1
  fi
  cp "$here/made.bin" "$dir/$name.bin"
  printf 'FILE "%s.bin" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n' "$name" > "$dir/$name.cue"
  echo "*** END (time check: 1s)"
  ;;
info)
  echo "*** INFO (time check: 0s)"
  echo "CD-ROM [$name.bin]:"
  echo "  sectors count: 12"
  echo "  generated sectors (0x55): 2 (LBA: 3, 7)"
  echo "  invalid sync sectors: 2 (LBA: 3, 7)"
  echo "*** END (time check: 0s)"
  ;;
esac
} | tee -a "$log"
exit 0
"#;

/// Rips again for Check and fix: rip N (from its folder's name) is the
/// disc read whole.
fn later_rips() -> RipFn {
    Arc::new(move |_cx, _dump, dir: PathBuf| {
        Box::pin(async move {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("G.bin"), image(&[]))?;
            std::fs::write(dir.join("G.log"), drt_steps::game_sim::LANTERN_KEEP_LOG)?;
            let cue = dir.join("G.cue");
            std::fs::write(&cue, "FILE \"G.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n")?;
            Ok(cue)
        })
    })
}

/// The drive for Check and fix's re-reads: sector 3 refuses twice, then
/// reads; sector 7 never does.
struct Drive {
    tries: std::collections::HashMap<i64, u32>,
}

impl Source for Drive {
    fn read(&mut self, lba: i64) -> std::io::Result<Vec<u8>> {
        let n = self.tries.entry(lba).or_insert(0);
        *n += 1;
        match lba {
            3 if *n <= 2 => Err(std::io::Error::other("the drive refused the command (sense 3/11/05)")),
            7 => Err(std::io::Error::other("the drive refused the command (sense 3/11/05)")),
            _ => Ok(sector(lba)),
        }
    }

    fn clear_cache(&mut self, _near: i64) -> std::io::Result<()> {
        Ok(())
    }
}

fn drive() -> SourceFn {
    Arc::new(|_dump: &Item| Ok(Box::new(Drive { tries: Default::default() }) as Box<dyn Source>))
}

fn base() -> PathBuf {
    let b = std::env::current_dir().unwrap().join("../../target/drt-test/unreadable");
    let _ = std::fs::remove_dir_all(&b);
    std::fs::create_dir_all(&b).unwrap();
    b
}

/// The Games recipe's read and check, Redump's list holding the disc as
/// it really is.
fn start(b: &Path, if_unreadable: &str) -> Run {
    let _ = std::fs::remove_file(b.join("calls"));
    let good = image(&[]);
    let dat = format!(
        r#"<?xml version="1.0"?><datafile><header><version>test</version></header>
<game name="Lantern Keep (Europe) (Disc 1)"><serial>SLES-04711</serial>
<rom name="Lantern Keep (Europe) (Disc 1).cue" size="1" crc="00" md5="00" sha1="ffff"/>
<rom name="Lantern Keep (Europe) (Disc 1).bin" size="{}" crc="{:08x}" md5="{}" sha1="{}"/></game>
</datafile>"#,
        good.len(),
        crc32fast::hash(&good),
        hex(&<md5::Md5 as md5::Digest>::digest(&good)),
        hex(&<sha1::Sha1 as sha1::Digest>::digest(&good)),
    );
    std::fs::create_dir_all(b.join("cache/redump")).unwrap();
    std::fs::write(b.join("cache/redump/psx.dat"), dat).unwrap();
    let mut reg = drt_steps::real();
    reg.add(Disc { device: b.join("drive") });
    reg.add(data::Check { rip: later_rips(), source: drive() });
    let mut e = Engine::new(reg);
    e.work_root = b.join("runs");
    let recipe = Recipe {
        name: "Stand-in".into(),
        root: chain([
            Node::step("drive", "drive").with("drive", "PLEXTOR PX-W4012A"),
            Node::step("rip", "game-rip").with(data::IF_UNREADABLE, if_unreadable),
            Node::step("check", "check"),
            Node::step("redump", "redump"),
            Node::step("choose", "game-choose"),
        ]),
    };
    drt_engine::check(&recipe.root, &e.registry).unwrap();
    e.start(&recipe).unwrap()
}

/// Waits for the run to come to `cond`, for at most 30 s, answering any
/// question by skipping it.
async fn until(run: &Run, what: &str, cond: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let mut rx = run.snapshot.clone();
    let wait = async {
        loop {
            let s = rx.borrow_and_update().clone();
            if cond(&s) {
                return s;
            }
            if let Some((step, _)) = &s.question {
                run.send(Command::Answer { step: step.clone(), answer: Answer::Skip }).await;
            }
            rx.changed().await.unwrap();
        }
    };
    tokio::time::timeout(Duration::from_secs(30), wait).await.unwrap_or_else(|_| panic!("stuck waiting for {what}: {:#?}", run.snapshot.borrow().steps))
}

fn log(s: &Snapshot, step: &str) -> String {
    s.steps[step].record.log.iter().map(|l| l.line.as_str()).collect::<Vec<_>>().join("\n")
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_sectors_go_to_check_and_fix_unless_you_say_ask() {
    let b = base();
    std::fs::write(b.join("drive"), "").unwrap();
    std::fs::write(b.join("made.bin"), image(&UNREAD)).unwrap();
    let script = b.join("redumper");
    std::fs::write(&script, STAND_IN).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // SAFETY: set before any thread of this binary runs a tool, and this
    // binary's only test.
    unsafe { std::env::set_var("DRT_REDUMPER", &script) };

    // By default: made anyway, then Check and fix reads 3 again from the
    // drive, takes 7 from a second rip, and it matches Redump.
    let run = start(&b, data::MAKE_ANYWAY);
    let s = until(&run, "the run to end", |s| s.finished).await;
    let rip = &s.steps["rip"];
    let note = data::made_anyway_note(2);
    assert_eq!(note, "made the image with 2 unreadable sectors; Check and fix will try them");
    assert_eq!((rip.items[0].state, rip.items[0].note.as_str()), (ItemState::Ok, note.as_str()), "{}", log(&s, "rip"));
    assert_eq!(rip.detail, "Made the image with 2 unreadable sectors; Check and fix will try them");
    assert_ne!(rip.state, StepState::Bad);
    let calls = std::fs::read_to_string(b.join("calls")).unwrap();
    assert!(calls.lines().any(|l| l.starts_with("split ") && l.contains("--force-split")), "{calls}");
    let check = &s.steps["check"];
    let clog = log(&s, "check");
    assert_eq!(check.items[0].note, "Verified: Lantern Keep (Europe) (Disc 1), 1 sector re-read, 1 from a second rip", "{clog}");
    assert!(check.items[0].verified);
    let fix = clog.find("by itself, as Check's settings say: Fix bad sectors").expect(&clog);
    let combine = clog.find("by itself, as Check's settings say: Rip again and combine").expect(&clog);
    assert!(fix < combine, "{clog}");
    assert!(clog.contains("with the drive's cache cleared first"), "{clog}");

    // Asked: Rip holds, as it did, and nothing reaches Check.
    let run = start(&b, data::ASK_ME);
    let s = until(&run, "Rip to hold", |s| s.steps["rip"].items.first().is_some_and(|i| i.state == ItemState::Bad)).await;
    assert_eq!(s.steps["rip"].items[0].note, "2 unreadable sectors");
    assert!(s.steps["check"].items.is_empty());
    let calls = std::fs::read_to_string(b.join("calls")).unwrap();
    assert!(!calls.contains("--force-split"), "{calls}");
    run.send(Command::Stop).await;
    until(&run, "the run to end", |s| s.finished).await;
    std::fs::remove_dir_all(b).unwrap();
}
