# DVD ripping: a built-in reader, with MakeMKV only for Blu-ray

28 November 2026. Proposed; nothing here is built yet.

**What the owner asked for:**
- DVDs rip with a reader built into Trip, with no setup at all.
- MakeMKV is kept for Blu-ray, where it's the only real option.
- MakeMKV's setup belongs on the Video disc tile, not in an app setting.
- There's no separate DVD and Blu-ray option unless one turns out to be needed.
- Nothing on the user's computer is run without their OK, and getting it set up never needs a terminal.
- It all works on Windows, macOS and Linux.

**Decided by the owner (28 November):**
- **libdvdcss is bundled** on all three OSes.
- Users install nothing extra for DVDs. MakeMKV stays the one exception.

Section 2 sets out what that decision means.

The decision came part way through the research. Two questions from the
brief were dropped as a result, not answered:
- how the user would fetch libdvdcss on each OS (RPM Fusion, `libdvd-pkg`, VideoLAN's DLL, Homebrew, VLC's copy), and whether the app could download it on their OK;
- whether a Flathub extension provides it.

Only 0.4 (the sandbox can see the host's `/usr`) was checked before that.

This builds on `cross-platform.md` (MakeMKV in §5, signing in §6.1) and
`portability.md` (§5, "is there an open alternative?"). It doesn't repeat them.

Marks used here, as in those two docs:
- *(ran)* means checked by running it on 28 November 2026;
- *(read)* means read in the source or the page linked, on the same day;
- **to find out** means not known yet, and the phase that needs it says so.

---

## 0. What running things showed first

Five findings change the starting point. Three of them correct what earlier docs and notes said.

**0.1 The flatpak already ships libdvdcss, inside ffmpeg.**
- The bundled ffmpeg (BtbN `autobuild-2026-10-31`, `N-128227-g4811625b5f`) contains libdvdcss's own messages: "cracking title key at block %i", "failed to crack the disc key", "Error cracking CSS key" *(ran: `strings`)*.
- Run on a test image with `DVDCSS_VERBOSE=2`, it prints `libdvdcss debug: … cracking title key at block 318 … unencrypted title` *(ran)*. So the code isn't only present; it runs.
- **Why:** BtbN's GPL builds compile libdvdcss as a static library and build libdvdread with `-Dlibdvdcss=enabled`, for every non-LGPL variant on Linux and Windows. See [`scripts.d/50-dvd/30-libdvdcss.sh`](https://github.com/BtbN/FFmpeg-Builds/blob/242c59f7f0/scripts.d/50-dvd/30-libdvdcss.sh) and [`40-libdvdread.sh`](https://github.com/BtbN/FFmpeg-Builds/blob/242c59f7f0/scripts.d/50-dvd/40-libdvdread.sh). (Pinned to 242c59f7f0, the last commit that touched them.) They were added together in commit [7e6ac5adfa](https://github.com/BtbN/FFmpeg-Builds/commit/7e6ac5adfa19bd64e25bfb15a13a440f2390e38f), 3 March 2024 *(read)*.
- **Why nobody noticed:**
  - `ldd` shows no libdvdcss because it's linked in.
  - CI's check matches file names (`find … -iname '*dvdcss*'`, `.github/workflows/build.yml`), and a static library has no file of its own.
  - So `THIRD-PARTY.md`'s "We do not ship one" is wrong, and has been since the flatpak first bundled this ffmpeg.
- **Who can download it:** the repository is public. CI has uploaded 63 `trip-flatpak` artifacts, and the latest from 27 November hasn't expired *(ran: `gh api …/actions/artifacts`)*. There are no Releases yet.
- **Now that bundling is decided,** this stops being a leak and becomes the plan. The text and the check still need fixing (phase D0).

**0.2 libdvdread and libdvdnav aren't in the GNOME 50 runtime. They're on the host.**
- Inside the flatpak, `find / -iname 'libdvd*'` turns up nothing under the runtime's `/usr`. The `org.gnome.Platform//50` and `org.gnome.Sdk//50` images have neither library *(ran)*.
- What can be seen is the host's: Fedora 44's `libdvdread-7.0.1` and `libdvdnav-7.0.0` (`libdvdread.so.8`, `libdvdnav.so.4`), at `/run/host/usr/lib64` *(ran: `rpm -q`, and `ls` inside the sandbox)*.
- The earlier note that the runtime ships them was looking at the host.

**0.3 The two ffmpegs differ.**
- The runtime's `/usr/bin/ffmpeg` is 7.1.3 and has no `dvdvideo` demuxer *(ran: `-demuxers`)*. That part of the brief is right.
- The ffmpeg Trip actually runs is the bundled one in `/app/bin`. It has `dvdvideo` and the `bluray` protocol *(ran)*. This matches `portability.md` §5.

**0.4 With `--filesystem=host`, the sandbox sees the host's `/usr` at `/run/host/usr`** *(ran)*. Loading a host libdvdcss would have been possible. With bundling decided, nothing needs it.

**0.5 The built-in path already works end to end on an unencrypted image.**
- An ISO made with `dvdauthor` held one title with two audio languages, a subtitle track and three chapters, plus a second title set with two more titles.
- The bundled ffmpeg remuxed title 1 to MKV with `-f dvdvideo -title 1 -map 0 -c copy` *(ran)*. It kept:
  - MPEG-2 video;
  - AC-3 audio tagged `eng` and `fre`;
  - `dvd_subtitle` tagged `ger`, carrying a 136-byte header that starts `palette: fefefe, 000000, …` (the disc's colours);
  - all three chapters at 0:00, 0:30 and 1:00.
- Titles 2 and 3 probed at 20 s and 90 s, and title 4 gave "Stream not found" *(ran)*.
- **What this doesn't show:** encryption, real authoring, or bad sectors. Those need real discs (section 7).

**0.6 libdvdcss writes a key cache to `~/.dvdcss` on the host** *(ran: the test run created one)*. That's because the flatpak sees the host's home. Trip should point `DVDCSS_CACHE` at its own cache folder (section 2.5).

---

## 1. Reading DVDs without MakeMKV

### 1.1 The options

| Option | What it is | For | Against |
|---|---|---|---|
| **A. ffmpeg's `dvdvideo` demuxer** | ffmpeg 7.0+ reads a DVD title through libdvdnav (playback) and libdvdread (structure) *([ffmpeg-formats](https://ffmpeg.org/ffmpeg-formats.html#dvdvideo), read)* | Already bundled and proven here (0.5). Output is MKV with no re-encode. Languages, flags, palette and chapters come for free. One more way of running a tool Trip already runs | It can't list titles: it opens one title by number, and "Defaulting to title #1 … validation suggested" if none is given ([dvdvideodec.c](https://github.com/FFmpeg/FFmpeg/blob/67240ca40f83da4a0160cce250bebb6adae4955c/libavformat/dvdvideodec.c), read). On a read error it stops: "Unable to read next block of PGC". It has no retry |
| **B. libdvdread / libdvdnav in-process, through Rust FFI** | Trip links the two C libraries and reads the disc's structure (IFO) itself | The only way to list titles, cells, title sets and stream attributes the way HandBrake and VLC do. libdvdnav can also run the disc's own first-play program (1.4) | There's no crate worth depending on (1.2), so it means writing a small binding. More C to build per OS |
| **C. Copy the files, then remux** (`dvdbackup`-style) | Decrypt and copy the VOBs to disk, then run A on the copy | One pass over the disc. Retries and a sector map, as for CDs. ffmpeg's own docs recommend a hard-drive copy for `-preindex` | Needs 4.7–8.5 GB of scratch space per disc. A whole-disc copy also reads the bad sectors that ARccOS and RipGuard plant (section 4) |

**Recommendation: B for the scan, A for the rip, and C only if real discs show it's needed.**
- **Video disc** lists titles in-process with libdvdread (B).
- **Rip** runs the bundled ffmpeg with `-f dvdvideo -title N` straight from the drive (A).
- C is held back as the fallback for discs that won't read straight through. It copies only the chosen title's cells, so it never touches the planted bad sectors (section 4).

### 1.2 Rust crates *(read: crates.io and each repository)*

| Crate | What | Verdict |
|---|---|---|
| [`dvdread-rs`](https://github.com/theflanman/dvdread-rs) 0.2.0 | bindgen against the system libdvdread. GPL-3.0, one star, last pushed December 2024 | Too thin and unmaintained to depend on. Its approach (bindgen on `dvdread/ifo_read.h`) is the one to copy |
| [`oxideav-dvd`](https://github.com/OxideAV/oxideav-dvd) 0.0.5 | Pure Rust ISO 9660 + UDF + IFO parsing (titles, PGCs, cells, colour table, stream attributes). MIT. "No CSS yet" | Interesting, but 0.0.x with one star. It can't read a CSS drive, since that needs authentication first. Not for now |
| [`libfreemkv`](https://github.com/freemkv/libfreemkv) | Rust drive access with CSS, and AACS with keys you supply. The GitHub README says MIT and "consumed by git tag", but crates.io has a `libfreemkv` 1.1.0 under AGPL-3.0-only. The two don't agree, and it isn't clear they're the same project | Unaudited by its own account, its licence is unclear, and it would replace libdvdcss with something less proven. No |
| [`dvdcss`](https://crates.io/crates/dvdcss) 0.1.2 | Bindings to libdvdcss, from 2021 | Not needed: libdvdread drives libdvdcss itself |

**Plan:** a small `dvd-sys` module in `crates/steps`, as `libchdman-rs` builds MAME's C.
- Its `build.rs` compiles libdvdcss, libdvdread and libdvdnav from pinned VideoLAN tarballs, checked against their published SHA-256.
- Bindings cover only what the scan uses: `DVDOpen`, `ifoOpen`, the title table (`tt_srpt`), each PGC's cells and playback time, the audio and subpicture attributes, and libdvdnav's VM for first-play.
- **Licences:** all three are GPL-2.0-or-later (libdvdcss's headers say "either version 2 of the License, or (at your option) any later version" *(read: 1.6.0 tarball)*). That's compatible with Trip's GPL-3.0-or-later.

### 1.3 What the MKV should hold, compared with MakeMKV

| | MakeMKV | ffmpeg `dvdvideo` → MKV | Gap |
|---|---|---|---|
| Video | MPEG-2 as on the disc | Same *(ran)* | — |
| Audio | Every track, language, commentary flag | Every track, language. Commentary, visually-impaired and karaoke are set from the IFO's code extension *(read: `dvdvideodec.c` sets `AV_DISPOSITION_COMMENT` etc.)* | Track names ("Director's commentary") aren't on a DVD; MakeMKV makes them up from the flags. Trip can do the same in a metadata pass |
| Subtitles | `dvd_subtitle`, palette and size, forced flag | `dvd_subtitle`, palette *(ran)*, forced flag from code extension 9 *(read)*, `VIEWPORT` (widescreen, letterbox, pan and scan) | No `size:` line in the header. Players default to 720×480/576, so probably harmless (**to find out** with Jellyfin and mpv) |
| Chapters | Yes | Yes, from the IFO *(ran)*. `-preindex` makes them exact but reads the title twice | Without `-preindex`, chapter times come from the IFO's cell times. Usually right to the frame; worth checking against MakeMKV on one real disc |
| Angles | Chosen angle | `-angle N` | Rare. Angle 1 by default |
| Title, duration tags | Yes | The MKV muxer writes `DURATION` per stream | `check_whole` (`makemkv.rs`) already reads exactly that |

Nothing is re-encoded. **Smaller video** (Transcode) and **Original video** (Remux) carry on as now.

### 1.4 How HandBrake and VLC do it

- **HandBrake** reads DVDs in-process with libdvdread and libdvdnav (`libhb/dvdnav.c`). It doesn't ship libdvdcss (2.1).
  - Its scan drops titles with "bogus cell addresses", titles too short or too long, and duplicate audio tracks *(read: [dvdnav.c at a1047ae](https://github.com/HandBrake/HandBrake/blob/a1047ae557f58fdb43f5836f29deb6219b3c861c/libhb/dvdnav.c))*.
  - Picking the main feature is in 3.1.
- **VLC** plays through libdvdnav with libdvdcss linked in. Flathub's VLC builds libdvdread 7.1.1, libdvdnav 7.0.0 and libdvdcss 1.6.0 from download.videolan.org as modules, with SHA-256 checksums ([manifest at a0f425f](https://github.com/flathub/org.videolan.VLC/blob/a0f425f6a9a8f2d11735cf6abf10c64821a50ea2/org.videolan.VLC.yaml), read).
- **Trip's split** follows HandBrake for listing and choosing, and leaves the stream copy to ffmpeg, which VLC and HandBrake both use underneath.

---

## 2. CSS decryption: bundling libdvdcss

### 2.1 What others do

| Project | Ships libdvdcss? | Source |
|---|---|---|
| VLC (Flathub, Windows, macOS) | Yes | Flathub manifest above *(read)*. Windows and macOS builds: checked by the coordinator, not re-read here. VideoLAN says libdvdcss "can find and guess keys from a DVD", and that a 2008 Conseil d'État decision on interoperability authorises it in France, "the only one [law] to be applicable to VideoLAN", which "is NOT a US-based organization" ([videolan.org/legal](https://www.videolan.org/legal.html), read) |
| HandBrake | **No, since 22 March 2008** | Commit [ecc54675dd](https://github.com/HandBrake/HandBrake/commit/ecc54675dddb56825215ce0c11358f81fc72f660) (svn r1354): "Banishes libdvdcss, removing DVD decryption from HandBrake's binaries". The Mac build then loaded VLC's copy if present *(read)*. The commit gives no reason. Its subject line is a quotation about being hanged by six honest lines, which suggests legal exposure (our inference, not their words). Its docs today: "HandBrake also does not defeat or circumvent copy protection of any kind", and they point to MakeMKV instead ([about.markdown](https://github.com/HandBrake/handbrake-docs/blob/master/source/docs/en/latest/introduction/about.markdown), read). Its Flathub manifest has no libdvdcss ([fr.handbrake.ghb at 7b5d323](https://github.com/flathub/fr.handbrake.ghb/blob/7b5d323dd037d69089b5dea5eb4bb78334b5d181/fr.handbrake.ghb.json), read) |
| BtbN's ffmpeg builds | Yes, statically, since March 2024 | 0.1 *(read)* |
| ffmpeg itself | No. Its docs say the demuxer "does NOT have decryption code of any kind" | ffmpeg-formats *(read)* |

### 2.2 What the law says, plainly

**UK, CDPA 1988 s.296ZB** ([legislation.gov.uk](https://www.legislation.gov.uk/ukpga/1988/48/section/296ZB), read):
- It's a crime to deal in a device "primarily designed, produced, or adapted for the purpose of enabling or facilitating the circumvention of effective technological measures". Dealing means making it for sale, importing it (other than for private use), or selling, offering or distributing it in the course of a business.
- It's also a crime to distribute such a device "otherwise than in the course of a business to such an extent as to affect prejudicially the copyright owner".
- Penalties run to two years' prison on indictment.

**UK, CDPA s.296ZD, the civil side** ([legislation.gov.uk](https://www.legislation.gov.uk/ukpga/1988/48/section/296ZD), read):
- Whoever "manufactures, imports, distributes, sells or lets for hire" a device "primarily designed … for the purpose of enabling or facilitating the circumvention" can be sued. That includes the film's copyright owner, whoever issues it to the public, and the owner of the protection itself.
- **Here "distributes" needs no business and no harm to the copyright owner.** So the civil route reaches a free download more plainly than the criminal one does.
- The court can grant what it would for copyright infringement, including delivery-up of the devices.
- **What applies to Trip, criminally:** it's not a business, so the second limb is the one that matters. A free download distributes libdvdcss. Whether that "affects prejudicially" a film studio, for a library they can already get from VLC, is a question for a lawyer. We didn't search for case law (**to find out**, if the owner wants a view).
- One caveat about the wording: the device test is about libdvdcss, not Trip. Trip as a whole isn't primarily designed to get round CSS, but the library inside it is.

**US, 17 U.S.C. §1201(a)(2)** ([Cornell LII](https://www.law.cornell.edu/uscode/text/17/1201), read):
- No one may "offer to the public, provide, or otherwise traffic in" technology "primarily designed or produced for the purpose of circumventing" an access control. "Circumvent" includes "to descramble a scrambled work".
- **No commercial purpose is needed for civil liability.** The crime in §1204 needs the act to be "willfully and for purposes of commercial advantage or private financial gain" ([§1204](https://www.law.cornell.edu/uscode/text/17/1204), read). A free, non-commercial project is outside that.
- **Precedent:**
  - In *Universal City Studios v. Corley* (2d Cir. 2001), 2600.com was barred from posting DeCSS or linking to it.
  - 321 Studios was barred from selling DVD X Copy in 2004 and shut down.
  - ([Corley](https://en.wikipedia.org/wiki/Universal_City_Studios,_Inc._v._Corley), [321 Studios](https://en.wikipedia.org/wiki/321_Studios), read: summaries, not the judgments.)
- **The realistic risk is a takedown, not a prosecution.** GitHub accepts §1201 claims through a form that must describe the measure and "how the accused project is designed to circumvent" it ([GitHub guide](https://docs.github.com/en/site-policy/content-removal-policies/guide-to-submitting-a-dmca-takedown-notice), read).
  - libdvdcss itself has been on VideoLAN's servers for many years (since 2001, from general knowledge, not checked) without that happening, as far as we know.
  - A claim would more likely land on Trip's releases than on its source, since the source only names a tarball.

**Why the earlier stance was "the user fetches it":**
- That way the project distributes nothing that circumvents, and the user's own act is the circumvention.
- The owner has weighed that against users having to install an extra thing, and chosen to bundle.
- So Trip now distributes a circumvention library, as VLC does, but without VideoLAN's French-law footing, and under a pseudonym rather than an organisation. That's the honest summary.

**What keeps the exposure as small as it can be:**
1. Ship libdvdcss unmodified, from VideoLAN's tarball, at a pinned version with its checksum. It's their library, not ours.
2. Don't market decryption. Trip's own words say "reads DVDs", never "removes copy protection" or "rips protected discs". (§1201(a)(2)(C) is about marketing.)
3. Keep the existing line in `THIRD-PARTY.md`'s "Your own use": discs you own, and know your own law.
4. Keep libdvdcss in the release, not in the repository. Source builds fetch the tarball, as Flathub's VLC does.

### 2.3 Building it, per OS

| | Linux flatpak (GitHub Releases) | Flathub | Windows | macOS |
|---|---|---|---|---|
| ffmpeg's libdvdcss | Already inside BtbN's ffmpeg (0.1) | Flathub builds from source and allows no binaries in a submission ([requirements](https://docs.flathub.org/docs/for-app-authors/requirements), read), and the runtime's ffmpeg has no `dvdvideo` (0.3). So the manifest builds libdvdcss 1.6.0, libdvdread, libdvdnav and an ffmpeg with `--enable-libdvdread --enable-libdvdnav` as modules, as VLC does | Already inside BtbN `win64-gpl`: the same scripts run for `win*` targets *(read)*. **To find out:** check it with `strings` as in 0.1 | Our own ffmpeg build (cross-platform §4.1) adds the three VideoLAN libraries, all meson since libdvdcss 1.5.0 *(read: NEWS)* |
| Trip's own (the scan, 1.2) | `build.rs` from the same pinned tarballs, static | The same, as a module | The same. libdvdcss has Windows `DeviceIoControl` SCSI pass-through *(read: `src/ioctl.c`)*. That it opens a drive given as `D:` is **to find out** | The same. libdvdcss has an IOKit path (`DARWIN_DVD_IOCTL`) *(read)* |
| Tarball | [download.videolan.org/pub/libdvdcss/1.6.0](https://download.videolan.org/pub/libdvdcss/1.6.0/): source only, with `.sha256` and `.asc` *(ran: listing; checksum `7ea556c8…0977c` matches)*. VideoLAN publishes no binaries for any recent version *(ran: listing)* | | | |

**Two copies of libdvdcss** (in ffmpeg and in Trip) are acceptable.
- Both read the same key cache when they're given the same `DVDCSS_CACHE` (2.5), so the second one doesn't crack keys again.
- **Pin the same version in both:** BtbN pins commit `811ce97a`, and Trip's `build.rs` should pin the 1.6.0 tarball. Record both in `PROVENANCE.txt`. **To find out:** whether BtbN's commit is 1.6.0 or later. If they differ, say so in `PROVENANCE.txt`.

**Licence obligations:** GPL-2.0-or-later, so its source goes in each release's source archive (cross-platform §4.3), and its licence text goes in `licenses/`. Already covered there; add the three libraries to the list.

### 2.4 The CI rule

- **Now:** fail on any file matching `*makemkv*`, `*dvdcss*` or `*libmmbd*`. As 0.1 shows, it never caught the static copy.
- **Change it to two checks:**
  1. **Still refuse MakeMKV:** `*makemkv*`, `*libmmbd*`, and also `strings` on every binary for `makemkvcon` / `libmmbd` / `GuinpinSoft`. Then a static copy can't slip through either.
  2. **Require the libdvdcss we meant to ship:** `PROVENANCE.txt` names its version and checksum for ffmpeg and for Trip, and `THIRD-PARTY.md` lists it. Fail if libdvdcss strings appear in a binary that `PROVENANCE.txt` doesn't account for.
- `THIRD-PARTY.md`'s "What the bundled ffmpeg does not contain" is rewritten to say what it does contain, and why.

### 2.5 Small things the bundling brings

- **Key cache.** Trip sets `DVDCSS_CACHE` to `<cache>/trip/dvdcss` for ffmpeg and for itself. Otherwise libdvdcss writes `~/.dvdcss` on the host (0.6), or a folder under `CSIDL_APPDATA` on Windows *(read: `libdvdcss.c` calls `SHGetFolderPathA` with it; the exact folder name wasn't read)*. Working files then stay in one place, which the Working files page already shows.
- **Logging.** `DVDCSS_VERBOSE=1` while Trip reads, so the run record says when a title key couldn't be found.
- **What the user sees when a key can't be found.** There are two cases *(read: libdvdcss 1.6.0 `css.c` and `libdvdcss.c`)*:
  - **The key search fails outright.** libdvdcss logs "fatal error in Video Title Set (VTS) Content Scrambling System (CSS) key", and `dvdcss_seek` returns -1, so the read fails. ffmpeg then stops with a read error, which Rip already reports.
  - **libdvdcss decides the title isn't scrambled.** It logs "unencrypted title" and uses a zero key, as in 0.5. If it decided that wrongly from the blocks it sampled, the sectors pass through still scrambled, and ffmpeg writes noise. The whole-title check passes because the length is right. Whether this happens on real discs is **to find out**.
  - For the second case, the Rip step also checks that the picture decodes:
  - decode a few seconds at three points with ffmpeg;
  - count decode errors;
  - fail the title with *"The picture didn't come out: this disc may be protected in a way Trip can't read"*.
  - **To find out** on a real disc: what scrambled MPEG-2 actually looks like to ffmpeg's decoder.

### 2.6 Flathub

- Flathub's requirements say nothing about DRM or copy protection. They ask that content "allow legal redistribution" *(read)*.
- VLC ships libdvdcss and libaacs there *(read)*. HandBrake doesn't.
- So VLC is the precedent, and Flathub's own reviewers make the call. **To find out**, by asking in the submission rather than assuming.
- If refused: the Flathub build leaves libdvdcss out. The tile then says *"This copy of Trip can't read encrypted DVDs; the one from the project's releases can"*. The GitHub Releases flatpak keeps it.

### 2.7 Windows signing

- `cross-platform.md` §6.1 and C1 recommended **SignPath Foundation**, the only free signature that keeps the pseudonym. Its terms ([signpath.org/terms](https://signpath.org/terms), read) say:
  - "The project may not contain any proprietary, non open-source component". libdvdcss is GPL, so that clause is fine.
  - "Software must not include features designed to … circumvent security measures of their execution environment". That's about the computer's own security, not copy protection, read literally.
  - "The project must not contain malware or potentially unwanted programs." Not at issue.
  - The page doesn't mention DRM or copy protection.
- **So the terms don't plainly forbid it, but a reviewer could read "circumvent security measures" broadly**, and SignPath's certificate carries its own name. Bundling libdvdcss makes refusal more likely than it was with MakeMKV left out.
- **What it means and the options:**
  1. **Ask SignPath first**, saying plainly that the Windows build contains libdvdcss, before 0.1.0. This replaces the question C1 already planned to ask.
  2. **If they say no:** Windows stays unsigned. Users click *More info → Run anyway* in SmartScreen, and Smart App Control blocks it where it's on. That's the same as C1's fallback.
  3. A paid OV or Certum certificate signs anything, but shows the owner's legal name. That breaks the pseudonym, so it isn't recommended.
  4. Shipping Windows without libdvdcss so it can be signed would go against the owner's decision. It's mentioned only because it's the one thing that would definitely keep SignPath open.
- **Recommended:** 1, with 2 as the expected result if they refuse. See decision D-1.

---

## 3. Choosing the main film among decoy titles

### 3.1 How others pick it

- **HandBrake** (`hb_dvdnav_main_feature`, *read*):
  1. Finds the longest title as a fallback.
  2. Runs the disc's own program the way a player would: first-play, then the root menu, then the title menu. It follows buttons (`try_menu`, `try_button`) to see which title actually plays.
  3. Keeps what the menus lead to, unless all three hold:
     - the longest title is more than 1.43 times as long (the code tests `longest × 0.7 > found`);
     - what the menus found is under 30 minutes;
     - more than ten titles are over 30 minutes, and their average is far from what the menus found.
  4. Only then does it take the longest.
  - In short: **trust what a player would play, and fall back to the longest only on discs full of long titles.** That last case is the Disney 99-title one.
- **ffmpeg** doesn't pick: title 1 unless told (1.1).
- **lsdvd** reports the "Longest track" (general knowledge; not read here).
- **MakeMKV** has its own closed handling of structure protection. Trip's scan today keeps the longest unique title for a film (`makemkv::select`).

### 3.2 What Trip does

The rule from `steps-know-only-their-wires`: Video disc *offers* candidates and facts; **Choose** weighs them and asks when unsure.

**Video disc records, per DVD title:**
- its length;
- its chapters;
- its title set (`TITLE_SET`, as MakeMKV's attribute 49 does now);
- its **cell list**;
- the audio and subtitle streams;
- whether it's **what the disc plays first**. That comes from libdvdnav's VM, as HandBrake does: first-play, then the title menu's default button, then the root menu's. It's a fact on the disc, `plays_first`.

**Duplicates:**
- Two titles with the same cell list are the same video, exactly. That's better than today's "same length, within 8 MB" test, which stays for Blu-ray.
- A title whose cells are all inside another's, in order, is an extract ("chapter 3 on its own").

**Decoys** (the Disney 99-title pattern): many titles within a few seconds of the same length, with the same chapters, using the same cells in different orders.
- Video disc doesn't call them decoys. It records `same_length_group = N`.
- **Choose** decides:
  - If `plays_first` is set and it's within 2% of the longest group, it's the film.
  - If not, Choose **asks**, with the existing per-title question. Each title gets its length, chapters, and "what a player plays first", if anything. A "Show me" button plays ten seconds from the middle of each (**to find out**: whether a preview fits the drawer; if not, just the facts).
- An episode disc's play-all title is already handled by `select`.

**Honest limits:** if the menus are built to mislead a player too, `plays_first` is wrong. A disc whose real title isn't what a player plays is rare (**to find out**). On those, Trip asks rather than guesses. That's slower than MakeMKV, never silently wrong, and the `rip` fact records which title was taken and why.

---

## 4. Read errors and structural protection

**What the protections do.** ARccOS (Sony, from about 2005) and RipGuard (Macrovision) plant unreadable sectors in cells that no menu path plays. A player never goes there. A tool that copies whole VOB files does.

**What the built-in reader does:**
- **ffmpeg `dvdvideo` follows the title's program chain through libdvdnav**, the same path as a player. So it should never read the planted sectors (**to find out** on a real ARccOS disc).
- If it does hit one, it stops with "Unable to read next block of PGC" (1.1), and the title fails. That's honest, but there's no recovery.
- **The Video disc scan** reads only IFO files, never VOBs, so planted sectors don't affect it.

**When a title fails, the Rip card already offers retries** (`rip_info`: *Read again*, *Skip this video*). Add:
- **Copy it first, then rip** (option C, held back from 1.1):
  - copy only the chosen title's cells to the work folder with libdvdread, retrying each bad sector a few times;
  - fill what can't be read with zeros, and record a sector map as the CD and game Rip already do;
  - then run `dvdvideo` on the copy.
  - The title then rips, with a note saying how many sectors were lost and roughly where ("2 seconds at 1:12:40").
- **Read it with MakeMKV.** This appears only if MakeMKV has been set up (section 5). If it hasn't, the option isn't there. The card doesn't nag about MakeMKV on a DVD.

**Real scratches versus planted sectors.** A planted sector sits in a cell the chosen title doesn't use, so the cell-only copy never touches it. A scratch is in cells the title does use, and shows up as a sector-map gap. Trip can say which kind of trouble a disc had.

---

## 5. The tiles

### 5.1 Options

**A. One Video disc tile that tells DVD from Blu-ray itself** (the owner's direction).
- `media::current_profile` already reads the drive's profile: 0x10–0x2B is DVD, 0x40–0x43 Blu-ray *(read: `media.rs`)*.
- Off Linux, or when the drive won't say, the fallback is opening the disc: `VIDEO_TS/VIDEO_TS.IFO` means DVD, and `BDMV/index.bdmv` means Blu-ray.

**B. Separate DVD and Blu-ray tiles or recipes.**
- **Against:** people own both, often in one box set. A recipe per disc type means choosing before the disc goes in, which is the question the owner hoped to avoid. Everything after the scan is the same for both: TMDb, Choose, Smaller video, Folder.
- **For:** nothing that A can't do. A **"Read with"** setting was the other version of this. It's only needed if MakeMKV must be forced on DVDs, and the retry option in section 4 covers that one title at a time.

**Recommendation: A, with no DVD/Blu-ray option and no "Read with" setting.**
- The disc decides.
- MakeMKV is reached in two ways: a Blu-ray going in, or a DVD title that failed with MakeMKV set up.
- If real use shows people want MakeMKV for every DVD, one setting can be added later: *DVDs: read with Trip / with MakeMKV*, shown only once MakeMKV is set up.

### 5.2 Where MakeMKV's setup lives

**Today:**
- the setting is app-wide (`app/src/makemkv.rs`, the `makemkv` file in the config folder);
- any recipe with `video-disc` or `rip-video` can't start until it's chosen (`Model::makemkv_blocks`);
- the step's sidebar shows the MakeMKV page first while it isn't set up (`tray.rs`, `makemkv_page`).

**The change:**
1. **A recipe never waits on MakeMKV to start.** `runs_makemkv` and `makemkv_blocks` stop blocking Start. A recipe with no Blu-ray in it never mentions MakeMKV.
2. **The Video disc tile asks when a Blu-ray goes in and MakeMKV isn't set up.**
   - The tile goes to *Needs*, with *"Blu-ray needs MakeMKV"* and the detail *"Choose where it is"*.
   - Its sidebar shows the MakeMKV page that exists today: what it is, what was found (*Use this*), *Choose…* and *Get MakeMKV*.
   - The disc waits in the tile, like a question. Once approved, the same disc carries on, and nothing is ejected.
   - **This is the OK:** choosing *Use this* or a file. MakeMKV is run once for its version, and nothing of it runs before that (today's rule, kept).
3. **The approval stays app-wide.** One install serves every recipe, and asking per recipe would mean asking again for the same file. Once given:
   - **On every Video disc tile's sidebar**, under its settings, a quiet row shows *MakeMKV 2.0.0, for Blu-ray* · *Change…* · *Don't use MakeMKV*, as `makemkv_page` shows it now. It's shown on each tile and chosen once, like TMDb's app-wide API key (`ParamSpec::app_wide`).
   - **The beta key** row sits under it: *Keep the free beta key up to date* (today's `key_updates`). Under that, the key's state: *Registered*, *Beta key, valid until end of November 2026*, or *Beta key expired*. These come from what MakeMKV says and from `key.rs` reading its settings. They're only read after the approval.
4. **Before any Blu-ray, and while MakeMKV isn't set up,** the Video disc sidebar shows only one quiet line: *Blu-ray discs need MakeMKV, which you'd choose when one goes in.* It has no buttons and nothing to scan for.
5. **The app-level MakeMKV page** (`Page::MakeMkv`) goes. The owner asked for the tile rather than an app setting, and the tile's row (3) shows the same controls. Revoking is *Don't use MakeMKV* on any Video disc tile.
6. **Rip** follows what Video disc found. It runs `media == "Blu-ray"` through MakeMKV, and a DVD through the built-in reader unless the user picked *Read with MakeMKV* for that title.

**The facts the tiles pass:**
- a `reader` fact (`trip` or `makemkv`) on the disc, set by Video disc;
- per title, `dvd_title` (the DVD title number), next to today's `makemkv_id`.

### 5.3 Sketch

The tile, then its sidebar, for a DVD (nothing to set up):

```
 ┌───────────────────────────┐
 │ ◉ Video disc              │
 │ 1 of 14 videos            │
 │ Video 1 · 1:57:30         │
 └───────────────────────────┘

 Video disc ─────────────────────────────── Done
 list the films and episodes on it

 Progress   Settings
 ─────────────────────────────────────────────────
 DVD · STAR_WARS_IV · 14 videos
  ✓ Video 1   1:57:30, 50 chapters, English 5.1,
              French, plays first
  · Video 2   1:57:31, duplicate of video 1
  · Video 3   0:04:12, 1 chapter
  …
 ─────────────────────────────────────────────────
 Skip videos shorter than      [ 60 ] s

 Blu-ray discs need MakeMKV, which you'd choose
 when one goes in.
```

A Blu-ray goes in, with MakeMKV not set up:

```
 ┌───────────────────────────┐
 │ ◉ Video disc        Needs │
 │ Blu-ray needs MakeMKV     │
 │ Choose where it is        │
 └───────────────────────────┘

 Video disc ────────────────────────── Needs you
 Blu-ray needs MakeMKV
 ─────────────────────────────────────────────────
 Blu-ray discs are read by MakeMKV, which doesn't
 come with this app: it isn't free software.
 Trip only runs it if you say so.

 Found on this computer
   MakeMKV (Flatpak)                  [ Use this ]

 [ Choose… ]   Get MakeMKV ↗

 The disc stays in the drive and carries on once
 MakeMKV is chosen.
```

Afterwards, on any Video disc tile's Settings:

```
 Skip videos shorter than      [ 60 ] s

 MakeMKV, for Blu-ray
   MakeMKV (Flatpak) · v2.0.0 · beta key, valid
   until end of November 2026
   [ Change… ]  Don't use MakeMKV
   [✓] Keep the free beta key up to date
```

The wording of `WHY` changes. Today it says MakeMKV "gets round copy
protection" as the reason it isn't bundled, but Trip now bundles a library
that does the same for DVDs. Its EULA does allow redistribution, and a
separate program beside a GPL app is allowed too (cross-platform §5.1).
The reasons left are these:
- **It's proprietary.** SignPath refuses "any proprietary, non open-source component", and Flathub builds from source.
- **It adds Blu-ray circumvention** (AACS, BD+) on top of DVD's.
- **Its beta key runs out** in step with MakeMKV's own releases, not ours.

So `WHY` says: *Blu-ray discs are read by MakeMKV, which doesn't come
with this app because it isn't free software. Trip only runs it if you say so.*

---

## 6. Blu-ray: MakeMKV stays required

- **AACS:** libaacs ships no keys. VideoLAN says it "DOES NOT provide any decryption key" *(read: videolan.org/legal)*.
  - Each disc needs its key from a community `KEYDB.cfg`, or keys derived from processing keys that newer discs revoke (MKB versions).
  - Coverage of recent discs is patchy (`portability.md` §5, not re-verified here).
- **BD+** needs libbdplus and its VM files, and works on a subset of discs.
- **UHD** needs a drive in LibreDrive mode as well, which is MakeMKV's own.
- **libaacs and libbdplus stay out of Trip.** Shipping them would add Blu-ray circumvention to the bundle, and still leave users hunting for keys. That's the worst of both.
- **What a Blu-ray shows with no MakeMKV:** the *Needs* tile in 5.3. If they choose *Don't use MakeMKV*, the disc is ejected and the run record says *"Blu-ray: not read, MakeMKV isn't set up"*. Nothing else is tried.

---

## 7. Testing without a pile of protected discs

| Layer | How | Proves |
|---|---|---|
| Scan logic (duplicates, decoys, `plays_first`, Choose's decisions) | Recorded scans as JSON fixtures, like `video/sim.rs`, through the same `record` and `select` | Choosing, with no disc |
| The built-in reader, end to end | **Test ISOs made in CI with `dvdauthor`**, then `genisoimage -dvd-video`. Proven here: three titles in two title sets, two audio languages, a `spumux` subtitle, chapters *(ran, in a Fedora 44 container; `xorriso -as mkisofs` lacks `-dvd-video`, so use `genisoimage`)*. Also add a 30-title image of one VOB cut into cells, to stand in for decoys (**to find out**: whether `dvdauthor` lets PGCs share cells; if not, write the IFO by hand) | Remux, streams, chapters, palette, title choice |
| Read errors | The test ISO on a loop device under device-mapper `dm-error` for chosen sectors, on a CI runner with sudo (**to find out**: GitHub's `ubuntu-24.04` allows `dmsetup`) | "Copy it first", the sector map, the error wording |
| CSS | Can't be faked: making an encrypted image needs the keys. Real discs only | That keys are found and the picture decodes |

**Real discs for the owner to try, in this order:**
1. An ordinary CSS film from before 2005: the plain case.
2. A TV season disc with a play-all title.
3. A Disney DVD from about 2007–2012, the 99-title kind.
4. A Sony DVD from about 2006–2010, for ARccOS.
5. A disc from another region, to see what an RPC-2 drive does with a CSS disc from elsewhere (**to find out**).
6. A scratched disc.

For each, rip with Trip and with MakeMKV, then compare:
- same title chosen;
- same streams and chapters;
- `ffprobe` durations;
- picture decodes all the way through.

Both drives here (MATSHITA UJ240AS, Lite-On eHAU324) read DVDs.

---

## 8. Phases

| Phase | What | Done when |
|---|---|---|
| **D0** Say what ships | `THIRD-PARTY.md` names libdvdcss (in ffmpeg now, and in Trip later), its licence and source, and its "MakeMKV is not shipped" reasons are rewritten as in 5.3. The same goes for `portability.md` §5 ("no libdvdcss … user-supplied"), `cross-platform.md` §5.1 and C1, and the comments at the top of `video/makemkv.rs` and `app/src/makemkv.rs`. The CI check becomes 2.4's two checks. `PROVENANCE.txt` records BtbN's libdvdcss commit. `WHY`'s wording (5.3) | CI fails a bundle with a stray `makemkvcon` string, and passes the current one with libdvdcss listed |
| **D1** Rip a DVD title with ffmpeg | For a DVD, Video disc lists titles by probing `-f dvdvideo -title 1, 2, …` with ffprobe until "Stream not found" (proven in 0.5). That gives each title's length, streams and chapters, and its DVD title number for Rip. Rip uses `-f dvdvideo -title N`, with `DVDCSS_CACHE` and the picture check (2.5). MakeMKV isn't touched. This is a bridge until D2: it has no cells, so duplicates fall back to today's length-and-size test | A test ISO title rips to MKV in CI, and one real CSS DVD rips on Linux with no MakeMKV run |
| **D2** The scan in-process | `dvd-sys` (1.2): titles, cells, streams, `plays_first`. Duplicates by cells. Video disc records them | Video disc lists a real DVD with the same titles as MakeMKV, on both drives, and Choose picks the same film on discs 1–3 of section 7 |
| **D3** MakeMKV only for Blu-ray, on the tile | 5.2's steps 1–6: no Start block, the *Needs* tile on a Blu-ray, the quiet line, the app-wide row on each tile, the key state. Tests with a stand-in MakeMKV check no process starts before *Use this* | A DVD recipe runs start to finish with MakeMKV never set up, and a Blu-ray in the same recipe asks on the tile and carries on after approval |
| **D4** Bad discs | *Copy it first, then rip*, the sector map, *Read with MakeMKV* when set up. dm-error tests | Discs 4 and 6 of section 7 either rip, or fail with words that say what happened |
| **D5** Windows and macOS | The libraries in each OS's ffmpeg and in `dvd-sys` (2.3). Drive names (`D:`, `/dev/rdisk2`) per cross-platform §2.3 | Disc 1 rips in the Windows VM. macOS per cross-platform C2 |
| **D6** Flathub | The modules in 2.3. Ask about libdvdcss in the submission (2.6) | Accepted, or the refusal is recorded and that build leaves it out |

D0 can start now. D1 is small: ffmpeg is already there.

---

## 9. Risks

| Risk | Fallback |
|---|---|
| A takedown claim against Trip's releases under §1201 | Releases come down. Source stays, since it only names VideoLAN's tarball. The owner decides then whether to ship builds without libdvdcss. Nothing in the code depends on it being there |
| SignPath refuses because of libdvdcss | Windows unsigned, as cross-platform C1's fallback (2.7) |
| Flathub refuses it | The Flathub build without libdvdcss, saying so on the tile. The GitHub flatpak keeps it (2.6) |
| A key isn't found and ffmpeg writes noise | The picture check (2.5) fails the title and says so |
| ffmpeg hits a bad sector and stops | *Copy it first, then rip* (section 4). Then *Read with MakeMKV*, if set up |
| `plays_first` is wrong on some discs | Choose asks whenever the long titles are crowded (3.2) |
| An RPC-2 drive won't read another region's disc without its region set | **To find out.** If so, the card says which region the disc is and that the drive is set to another. Trip never changes a drive's region, since drives allow only a few changes |
| BtbN changes or drops its libdvdcss | Pinned builds don't move. `PROVENANCE.txt`'s check catches a change. Our own ffmpeg build (needed for macOS and Flathub anyway) takes over |
| Two libdvdcss versions disagree on a disc | Same `DVDCSS_CACHE`. If they really differ, Trip's own build is moved to BtbN's commit |

---

## Decisions for the owner

**Settled:** libdvdcss is bundled on every OS (owner, 28 November). One Video disc tile tells DVD from Blu-ray. MakeMKV's OK is asked on that tile when a Blu-ray goes in, and remembered app-wide.

**D-1. Windows signing, now that libdvdcss is in the build.**
- **Recommended:** ask SignPath before 0.1.0, saying plainly that it contains libdvdcss.
- If they refuse, ship Windows unsigned with the SmartScreen steps in the README.
- Don't buy a certificate, which would put the owner's legal name on it.

**D-2. What happens to the releases if a takedown comes.**
- **Recommended:** decide now so it isn't decided in a hurry. Comply, and re-release without libdvdcss, which section 2.6's tile wording already covers.
- The other way is to dispute it under GitHub's process, which means arguing about the law under a pseudonym.

**D-3. Should Trip ever read every DVD with MakeMKV?**
- **Recommended: no setting for now.** MakeMKV for a DVD only as a retry on a title that failed (section 4).
- Add *DVDs: read with Trip / with MakeMKV* later if people ask.
