# Plan: proving it's right, then getting it to people

24 November 2026. Proposed; nothing here is built yet.

Disc Rip Trip rips well on one machine, two drives and a handful of discs,
and nobody else has run it. This plan does two things:

1. **Prove it**, in ways a stranger can check without taking our word.
2. **Release it**, on Flathub, with the things a first release needs.

Proof comes first. Flathub puts the app in front of people who will
judge it on whether their rips come out right, and a bad first
impression of a ripper doesn't wear off.

Marks used here: *(ran)* means checked by running it on 24 November;
*(read)* means read in the source or the docs linked; **to find out**
means not known yet, and the phase that needs it says so.

---

## Part 1: prove it

### 1.1 Where we stand

| What | Proven on real discs? | On what |
|---|---|---|
| Audio CDs, ripping and AccurateRip | Yes | Both drives (MATSHITA UJ240AS, offset +103; Lite-On eHAU324, offset +6); a few dozen albums; bit-for-bit checks against a second rip and against coreutils hashes |
| PlayStation discs (image, Check and fix, CHD) | Yes, with Redump matches | Koudelka Discs 1–3 matched Redump on all four counts (size, CRC-32, MD5, SHA-1); Disc 4 is physically damaged and doesn't match |
| Other data discs and games | No | Simulated discs only |
| DVD and Blu-ray video | No | Simulated discs only; needs MakeMKV |
| Windows, macOS | No | Doesn't build or find drives (see `portability.md`) |

The README's Status section is out of date: it still says games are
untested on real discs. Fixing it is the first change (phase P1).

### 1.2 Drive recordings in the tests

Every drive bug we've fixed came from a real drive doing something no
simulation predicted:

- the Lite-On hands over each sector's parts in an unusual order;
- the MATSHITA stalls on hard sectors and the USB link resets;
- the tray won't open;
- the drive's status reports are stale;
- redumper gets killed partway through a read.

Some of these already have tests with a stand-in drive or a stand-in
redumper (`sector_order.rs`, `re_reads.rs`, `redumper_dies.rs`,
`open_tray.rs`, `unreadable.rs`), each written by hand.

**What changes:** a drive's behaviour becomes a small recording any test
can replay.

- **What a recording holds:** for each read, where it was, what came back
  (good, which error, which sense code), how long it took, and the order
  the sector's parts arrived in.
- **What it doesn't hold:** the disc's content. Sectors are filled with
  made-up data that passes or fails its own checks as the recording says.
  The repo is public, and game and album data can't go in it.
- **Where recordings come from:** a `drt record-drive` command writes one
  from a real rip's logs and `.state` file. That way, when someone reports
  a drive problem, their recording becomes a test.

The first recordings are the five behaviours above, plus Koudelka Disc
4's two damaged patches, from the map we already have.

*Done when* each existing hand-written stand-in is a recording, and a new
drive quirk can be added as a test without new code.

### 1.3 A rip log others can check

There's a rip log already (`cue.rs`, `rip_log`), written beside every
album. It records the drive, the offset, the disc's TOC and IDs, and for
each track every read, with its AccurateRip result, the CRC and the
verdict *(read)*.

**Being honest about what a log can prove:** EAC signs its logs with a
secret method, and people use that signature to check a log hasn't been
edited. An open-source app can't keep a secret, so any signature we add
proves the file wasn't damaged, not that nobody edited it. The useful
promise instead is that **everything in the log can be checked again from
the files**: anyone can recompute each hash from the audio, and look each
AccurateRip or CTDB result up again.

**What's missing:**

| Missing | Why it matters |
|---|---|
| Each file's own hash, e.g. the FLAC's audio MD5 | Links the log to these exact files |
| AccurateRip v1 and v2 checksums, not just "Accurate (12)" | Lets someone else look the result up again |
| The gaps between tracks, and the cue sheet's start positions from them | The cue sheet says gaps aren't known (`cue.rs`). Without them it doesn't describe the disc, and CUETools can't check the whole disc against CTDB |
| How the drive read: cache cleared, C2 used, paranoia level, the read speed asked for and what was got | What anyone weighing a log looks at first |
| A CTDB result | Comes with 1.4 |
| A plain summary at the top: *"All 12 tracks verified by AccurateRip"*, or *"Track 7 kept from two reads that agree; not verified"* | Readable at a glance |
| A checksum of the log's own text at the end | Proves only that the file wasn't damaged, and says so |

**A test against other people's tools:** rip an album, then check the
files and cue sheet with CUETools, using its "Verify" mode under Wine or
Mono. Its AccurateRip and CTDB results must match ours. **To find out:**
whether CUETools runs well enough on Linux for this. If not, compare with
`arver` or another open AccurateRip checker.

The gaps are the biggest item. **To find out:** whether cyanrip reports
the pregaps it reads (it reads Q subchannel). If not, read them ourselves
with the SG_IO reads we already have for data discs.

*Done when* a log from a real album passes the CUETools comparison, and a
log edited by hand, like a changed CRC, is caught by recomputing from the
files.

### 1.4 CTDB check and repair

Already designed in `docs/ctdb.md`, from the CUETools source. Nothing to
add here except where it fits: Rip asks CTDB after AccurateRip, and its
result goes in the log (1.3). Repair comes after the check, because a
check alone already settles tracks that AccurateRip doesn't know.

*Done when* the cases in `ctdb.md` pass, including a real album that
AccurateRip doesn't hold but CTDB does.

### 1.5 Tidy what was added on

Much of the app was built quickly, one feature at a time; the Game Rip
sidebar rework already showed parts that didn't fit together. A single
review, checking structure rather than hunting bugs, of the parts that
grew fastest:
- `real/mod.rs`, at a few thousand lines, with most steps in one file;
- `app/src/app.rs`;
- the Check and fix flow.

The rule throughout: no change to what the app does, and every test still
passes.

*Done when* no file is over about 1,500 lines without a reason, and each
step lives in its own module.

---

## Part 2: release it

### 2.1 What Flathub requires, and where we fall short

From Flathub's rules (docs.flathub.org: *Requirements* and *Linter*)
*(read)*:

| Flathub requires | Where we are |
|---|---|
| **Built from source, from a public URL; no prebuilt binaries** in the submission | The manifest installs `payload/`, which holds binaries built beforehand, plus ffmpeg and redumper downloaded already built (`fetch-tools.sh`). This is the main change. |
| **No network during the build;** each Rust crate listed as a source, using `flatpak-cargo-generator` | Our build script fetches crates and tools over the network inside the SDK. |
| **App id matches the repo:** `io.github.<user>.<repo>` maps to `github.com/<user>/<repo>`, and that page must exist | `io.github.vresistor.DiscRipTrip` maps to `github.com/vresistor/DiscRipTrip`, which **doesn't exist** *(ran)*. The repo is `disc-rip-trip`. See decision D1. |
| **Valid metainfo,** with the licence matching the source, and a licence file for each module | We have a metainfo file; it hasn't been through `flatpak-builder-lint`. **To find out.** |
| **Stable releases only:** no snapshots or nightlies | We have no version tags yet. |
| **Static permissions kept to a minimum,** with portals where they fit | We ask for `--device=all`, `--filesystem=host` and `--talk-name=org.freedesktop.Flatpak`. See 2.3. |
| **No reliance on host programs for core features** | MakeMKV runs on the host, but only video needs it, so it's optional, not core. The description has to say so. |

### 2.2 The tools, built from source

| Tool | Today | For Flathub | Risk |
|---|---|---|---|
| **ffmpeg** | BtbN's ready-made build, 40 MB | **Use the runtime's own.** GNOME 50's ffmpeg 7.1.3 has every encoder the recipes use: libx264, libx265, libsvtav1, libmp3lame, libopus, libvorbis, flac, alac and aac *(ran)*. Nothing to build. | Low. Check the runtime's ffprobe too, and test each recipe's encode on it. |
| **cyanrip** | Built from pinned sources by `build-cyanrip.sh` | The same steps as a manifest module: it and its libraries (libcdio-paranoia, libmusicbrainz) as source modules | Low; we already build it inside the SDK. |
| **redumper** | Upstream's ready-made build | A module built with CMake and Ninja. It needs **Clang 18 or newer and CMake 3.28** for C++20 modules *(read: its README)*. The SDK's LLVM extension provides Clang. Its tests download GoogleTest, so build with tests off. | **High.** C++20 module builds are fragile, and it's the piece most likely to fail Flathub's offline build. This is phase P4's first step, with a fallback. |
| **The app itself** | `cargo build` inside the SDK, with the network | `flatpak-cargo-generator` turns `Cargo.lock` into a source list. No crate comes from git *(ran)*. `libchdman-rs` compiles MAME's CHD code from the crate. **To find out:** that its build script fetches nothing. | Medium: the build script is the unknown. |

**If redumper won't build offline:** game ripping is the app's second
main use, so dropping it from the Flathub version is a real loss.
Fallbacks, best first:
1. Help upstream make an offline build work, and wait for it.
2. Ask Flathub for an exception to use upstream's own released binary.
   Exceptions go to well-known vendors, so this may well be refused.
3. Ship audio and video on Flathub, and games from our own releases.

### 2.3 Permissions

| Permission | Why we need it | Precedent or alternative |
|---|---|---|
| `--device=all` | Raw access to `/dev/sr*`. Flatpak has no narrower permission for an optical drive. | **HandBrake on Flathub uses it** *(ran: its manifest)*. Keep. |
| `--filesystem=host` | Library folders on NAS mounts and external drives, and paths a recipe saved earlier, which a portal grant doesn't cover | **HandBrake uses it too** *(ran)*. Keep, but say why in the submission. **To find out:** whether `--filesystem=home` plus `/media`, `/run/media`, `/mnt` and `xdg-run/gvfs` covers every case, since reviewers prefer narrower. |
| `--talk-name=org.freedesktop.Flatpak` | Runs MakeMKV on the host | Only a linter *warning*, not an error *(read)*. Reviewers may still ask. The honest description: it lets the app run anything on the host, it's only used for MakeMKV, and video needs it. |

### 2.4 The first release

- **Version 0.1.0,** tagged, with the release notes in the metainfo's
  `<releases>`, which Flathub shows.
- **A changelog** in plain words, written for people who rip discs, not
  for developers.
- **"Copy a report"** in the app, for when something goes wrong. It
  copies the app version, the drive's model and firmware, the recipe, the
  last run's log and the tool versions. Folder paths are replaced with
  `~`, and NAS server names with `<server>`. The report shows what it
  will copy before copying.
- **Issue templates** on GitHub, "a rip went wrong" and "my drive isn't
  handled", each asking for that report.
- **Status in the README,** rewritten from 1.1.
- **Screenshots** for the metainfo: the CDs recipe mid-rip, the Check and
  fix map, the recipe editor.

### 2.5 After submitting

- Flathub then builds from a separate repo it owns
  (`github.com/flathub/<app id>`). Every release means updating the
  manifest there. A release workflow in our CI can open that pull request
  by itself.
- Dependabot keeps updating our crates. Each update also needs the
  cargo source list regenerating, so CI should fail if it's stale.

---

## Phases

Each phase ends with something working; none is half-done when the next
starts.

| Phase | What | Done when |
|---|---|---|
| **P1** | README Status rewritten (1.1); the app id decided (D1) | The README matches what's been proven; the id is settled |
| **P2** | Drive recordings (1.2) | Existing stand-ins replaced; Disc 4's patches replay |
| **P3** | Rip log gaps, including track gaps (1.3), then CTDB check (1.4) | A real album's log matches CUETools; an edited log is caught |
| **P4** | Flathub build: redumper offline first, then cyanrip, the app, and the runtime's ffmpeg (2.2) | `flatpak-builder --sandbox` builds with no network, and `flatpak-builder-lint` passes the manifest and the repo |
| **P5** | Release: 0.1.0, changelog, Copy a report, issue templates, screenshots (2.4) | A tagged release with notes; the report copies nothing private |
| **P6** | Submit to Flathub | Accepted, or the reviewers' requests answered |
| **P7** | Tidy (1.5), and CTDB repair (1.4) | As in those sections |

P4 can start alongside P2 and P3. Doing its riskiest step (redumper)
early means a refusal changes the plan sooner rather than later.

---

## Decisions for you

**D1. The app id.** Flathub reads `io.github.vresistor.DiscRipTrip` as
`github.com/vresistor/DiscRipTrip`, which doesn't exist. Two ways to
fix it:
- **Change the id to `io.github.vresistor.disc-rip-trip`.** This matches
  the repo exactly; dashes are allowed in the last part. Your recipes and
  settings move to a new folder under `~/.var/app/`, and the app would
  move them over once, on first start.
- **Rename the repo to `DiscRipTrip`.** GitHub redirects the old address,
  and the id stays.

Either is cheap now, and very hard to change once the app is on Flathub.
**To find out first:** whether Flathub really checks the repo name
itself, or only that the GitHub account `vresistor` is yours. Its docs
say the repo, but its verification may only look at the account.

**D2. Who submits, and when.** Submitting to Flathub happens under a
GitHub account, and the reviewers talk to it in the open. That would be
`vresistor`, in keeping with the pseudonym. Submission is P6, after the
proof work, not before.

**D3. Games if redumper won't build** (2.2). Only needed if P4's first
step fails.
