# Cross-platform: Trip on Windows, macOS and Linux

25 November 2026. Proposed; nothing here is built yet.

The goal is that Trip is built for Windows, macOS and Linux and
published on all three, with each tool it needs packaged with it under the
right licence notices. Where a tool can't be packaged (MakeMKV), the app
asks the user before it uses one that's already on their computer, and lets
them say where it is.

This builds on `portability.md`, which lists every tool the app runs and
every Linux-only piece of code. That isn't repeated here.

**The constraint that shapes all of it:** the only hardware here is a
Linux desktop and two USB drives (MATSHITA UJ240AS, Lite-On eHAU324).
Windows can be tested on that machine in a virtual machine. macOS can't be
tested on it at all, so the plan for macOS is about what can be proven
without a Mac, and who else can prove the rest.

Marks used here:
- *(read)* means read in the source or the page linked, on 25 November 2026;
- *(ran)* means checked by running it (very little here: this was research only);
- **to find out** means not known yet, and the phase that needs it says so.

The research behind each claim is linked where it's made.

---

## 1. Where each OS stands now

| | Linux | Windows | macOS |
|---|---|---|---|
| Builds | Yes: the flatpak, built in CI | **No.** `tool.rs` uses `std::os::fd` and `libc::openpty`; `reach.rs` uses `statvfs`; `redumper.rs` reads Unix signals; several modules use `std::os::unix` *(read)* | Probably compiles; never built |
| Finds drives | Yes (`/sys/class/block`) | No | No: `drives()` is Linux-only, and TOC, disc type and eject are stubs (empty, `None`, `false`) *(read: `media.rs`, `mod.rs`)* |
| Runs the tools | Yes, through a pseudo-terminal | No pseudo-terminal | `openpty` exists (see 3.2) |
| Tools packaged | cyanrip, redumper, ffmpeg, ffprobe in the flatpak | None | None |
| Tested on real discs | CDs and PlayStation discs, both drives | Never | Never |
| Packaging | `.flatpak` on Releases; Flathub planned | None | None |
| CI | Tests and the flatpak on ubuntu-24.04 | None | None |

The first app (Tauri, deleted at the swap) did build for all three,
including a Windows `.exe` and an unsigned macOS `.dmg`, built on GitHub's
runners. Its scripts are kept outside this repository, and four of its
lessons still hold:
- the console window, below;
- that the last Intel macOS runner had gone, so macOS needed its own ffmpeg build;
- where MakeMKV is installed on each OS;
- that Sequoia removed Control-click → Open.

---

## 2. Drive access

The app itself talks to the drive for five things. Each OS needs its own
way of doing each one.

| Need | Linux now | Windows | macOS |
|---|---|---|---|
| Listing drives, vendor and model | `/sys/class/block/sr*` | `GetLogicalDrives` + `GetDriveType == DRIVE_CDROM` (what redumper and libcdio both do *(read)*); model from `IOCTL_STORAGE_QUERY_PROPERTY` | IOKit: services with `kIOPropertySCSITaskAuthoringDevice`, by BSD name (what redumper does *(read)*) |
| Whether a disc is in | `CDROM_DRIVE_STATUS` | `IOCTL_STORAGE_CHECK_VERIFY`, or media size from `IOCTL_DISK_GET_LENGTH_INFO` (**to find out** which is reliable on USB drives) | DiskArbitration: a disc appears as a `DADisk` with media |
| Disc type (CD, DVD, BD) | MMC GET CONFIGURATION through `SG_IO` | The same command through `IOCTL_SCSI_PASS_THROUGH_DIRECT` | The IOKit media class (`IOCDMedia`, `IODVDMedia`, `IOBDMedia`) without raw commands (**to find out**) |
| TOC | `CDROMREADTOC*` | `IOCTL_CDROM_READ_TOC_EX` | `DKIOCCDREADTOC` on `/dev/rdiskN` (**to find out**) |
| Raw sector reads (cache flush before a rip, Check and fix, CHD read-back) | READ CD (0xBE) through `SG_IO` | The same command through `IOCTL_SCSI_PASS_THROUGH_DIRECT` | `DKIOCCDREAD`, as libcdio does (see 2.2); raw commands would need exclusive access |
| Eject | `CDROMEJECT` | `IOCTL_STORAGE_EJECT_MEDIA` | `DADiskEject`, after unmounting |

The command blocks (CDBs) the app already builds for `SG_IO` carry over to
Windows unchanged. Only the call that sends them differs.

### 2.1 Windows

**What the headers and Microsoft's pages say** *(read)*:
- **SCSI pass-through** is defined with `FILE_READ_ACCESS | FILE_WRITE_ACCESS`, so the drive has to be opened for both reading and writing ([ntddscsi.h in mingw-w64](https://github.com/mingw-w64/mingw-w64/blob/f0f3907625e5bf67bbb6f50ae821c16971023ab6/mingw-w64-headers/include/ntddscsi.h)).
- The request goes to the class driver: `\\.\D:` or `\\.\CdRom0`.
- The data buffer must match the adapter's alignment, which `IOCTL_STORAGE_QUERY_PROPERTY` reports ([IOCTL_SCSI_PASS_THROUGH_DIRECT](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddscsi/ni-ntddscsi-ioctl_scsi_pass_through_direct)).
- **Eject, load and TOC need read access only.** `IOCTL_STORAGE_QUERY_PROPERTY` needs none.

**Admin rights.**
- Microsoft's `CreateFile` page says admin is needed to open physical disks and volumes ([CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)). It says nothing about CD drives.
- The policy "Restrict CD-ROM access to locally logged-on user only" is off by default, which suggests a signed-in user can open the drive ([Microsoft](https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/devices-restrict-cd-rom-access-to-locally-logged-on-user-only)).
- Neither redumper nor libcdio asks for admin (below).

**To find out, and it matters:** whether a standard (non-admin) Windows
account can do SCSI pass-through on a USB drive. If it can't, the app has
to say so plainly rather than fail with "access denied".

**How each tool opens the drive** *(read)*:
- **redumper** opens `//./E:` with `GENERIC_READ | GENERIC_WRITE` and sends `IOCTL_SCSI_PASS_THROUGH_DIRECT`. Drives are named `--drive=E:`, and "the trailing colon is optional" ([sptd.ixx at b775](https://github.com/superg/redumper/blob/b775/scsi/sptd.ixx), [README](https://github.com/superg/redumper/blob/b775/README.md)).
- **cyanrip** passes `-d` straight to libcdio, so it's `-d D:` ([cyanrip_main.c](https://github.com/cyanreg/cyanrip/blob/v0.9.3.1/src/cyanrip_main.c)). libcdio's Windows driver opens `\\.\D:` read-write, falls back to read-only, and reads audio with `IOCTL_CDROM_RAW_READ`. No ASPI ([libcdio MSWindows driver](https://github.com/libcdio/libcdio-C/blob/4b788df85d99b231ac0f3d8f26589b94db8ef165/lib/driver/MSWindows/)).
- **ffmpeg** doesn't touch the drive. It reads files the rippers wrote.
- **makemkvcon**: the official usage page documents only `disc:N` ([usage.txt](https://www.makemkv.com/developers/usage.txt)). `dev:` comes from a Gentoo man page, not MakeMKV. On Windows the `DRV:` lines from `info disc:9999` end with the drive letter ([forum](https://forum.makemkv.com/forum/viewtopic.php?p=45223)).
  - **The reliable route on every OS:** run `info disc:9999`, find the line whose device matches, and rip `disc:<index>`. Whether `dev:D:` works is **to find out**.

**Other Windows details:**
- **No console windows.** The app is built as a windowed program, so each console tool it starts would open a console window of its own unless started with `CREATE_NO_WINDOW`. The first app hit this and fixed it in its one spawn function. Here that's `tool::command`, plus makemkv's launcher.
- **File names.** Windows refuses `< > : " / \ | ? *`, trailing dots and spaces, and names like `CON` and `NUL`. Album titles often have colons. **To find out:** whether `renames.rs` and the folder naming already make names safe for Windows; if not, it's a bug to fix before the first Windows rip.
- **Long paths.** Paths over 260 characters fail unless the app's manifest opts in to long paths. Deep library folders reach that.
- **Stopping a tool.** `start_kill` is `TerminateProcess`. Unix signals (`ExitStatusExt::signal` in `redumper.rs`) go behind `cfg(unix)`.
- **Free space.** `statvfs` becomes `GetDiskFreeSpaceExW`.
- **Config and cache** come from the `dirs` crate: `%APPDATA%`, `%LOCALAPPDATA%` ([dirs 7.0](https://docs.rs/dirs)). Today, with no `HOME` set, config would land in `./trip`.
- **Notifications:** Windows toasts. The ntfy path works as it is.

### 2.2 macOS

**Raw access needs the disc unmounted, and then nothing else can use the
drive.** Apple's `ObtainExclusiveAccess` "returns kIOReturnBusy" if media is
still mounted, and once granted, "all in-kernel Logical Unit Drivers are
quiesced" ([Apple](https://developer.apple.com/documentation/iokit/scsitaskdeviceinterface/1575374-obtainexclusiveaccess)) *(read)*.

**How redumper does it** *(read: [sptd.ixx](https://github.com/superg/redumper/blob/b775/scsi/sptd.ixx))*:
1. It claims the disc (`DADiskClaim`), waits if macOS is still mounting it, then unmounts it (`DADiskUnmount` with force and whole-disk).
2. It finds the drive by BSD name and takes exclusive access through `SCSITaskDeviceInterface`.
3. Drives are named `--drive=disk2`.
4. **Its README warns** that running it from Desktop, Documents or Downloads makes the drive interface fail with "resource shortage", because of macOS's privacy rules for those folders. So the app must start redumper with a working folder outside them (the cache under `~/Library/Caches` is fine), whatever the user's output folder is.

**How cyanrip (libcdio) does it** *(read: [osx.c](https://github.com/libcdio/libcdio-C/blob/4b788df85d99b231ac0f3d8f26589b94db8ef165/lib/driver/osx.c))*:
- It opens `/dev/rdiskN` read-only and reads audio with the `DKIOCCDREAD` ioctl. The source says this is because "MMC commands need exclusive access to the drive on macOS".
- Its raw MMC path is compiled out, so cyanrip's drive-speed and other MMC features won't work on macOS.
- It needs no unmount. **To find out:** whether reading works while macOS has the audio CD mounted (it mounts audio CDs as a folder of AIFF files).

**For the app itself:**
- Its raw reads, disc type, TOC and cache flush should use the same ioctls as libcdio (`DKIOCCDREAD`, `DKIOCCDREADTOC`), not exclusive access. Then it never fights a tool for the drive.
- It must let go of everything before starting redumper, which takes the drive for itself.
- Rust crates (none of them a ready-made optical-drive crate) *(read: docs.rs)*:
  - [`objc2-disk-arbitration`](https://docs.rs/objc2-disk-arbitration) has `DADiskUnmount`, `DADiskClaim` and the mount-approval callback;
  - [`io-kit-sys`](https://docs.rs/io-kit-sys) has the IOKit basics but not the SCSITask tables;
  - [`core-foundation`](https://docs.rs/core-foundation).

**Privacy prompts.** `NSRemovableVolumesUsageDescription` covers files on
removable volumes ([Apple](https://developer.apple.com/documentation/bundleresources/information-property-list/nsremovablevolumesusagedescription))
*(read)*. It matters when the library is on an external disk. Whether raw
drive access triggers any prompt is **to find out**. The `.app`'s
`Info.plist` should carry the usage strings either way, so a prompt never
appears unexplained.

**Apple Silicon.** redumper ships a macos-arm64 build *(read: release
b775)*. No Apple Silicon problem with drive access turned up.
arm64 code must be at least ad-hoc signed to run at all, though an ad-hoc
signature doesn't satisfy Gatekeeper ([Apple](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes))
*(read)*. That ties into section 6.

### 2.3 Drive names

Each tool spells the drive differently:

| | Linux | Windows | macOS |
|---|---|---|---|
| What the app lists | `/dev/sr0` | `D:` | `disk2` |
| cyanrip `-d` | `/dev/sr0` | `D:` | `/dev/rdisk2` |
| redumper `--drive=` | `/dev/sr0` | `D:` | `disk2` |
| makemkvcon | `disc:N`, found from `info disc:9999` | same | same |

So the app needs a drive type that knows each spelling, rather than passing
one string through. That's a change worth making on Linux first, where it
can be tested.

---

## 3. Running the tools

### 3.1 Why the app uses a pseudo-terminal

C's stdio buffers stdout fully when it isn't a terminal. The C standard
says so (C11 §7.21.3p7, [n1570](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)),
and Microsoft's C runtime has no line buffering at all: `_IOLBF` behaves as
`_IOFBF` ([setvbuf](https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/setvbuf))
*(read)*. So a tool that prints progress without flushing gives nothing
through a pipe until it exits, which is what was measured with cyanrip.

### 3.2 Which tools actually need it

From each tool's source *(read)*:

| Tool | Flushes its progress? | Through a pipe |
|---|---|---|
| **cyanrip 0.9.3 / 0.9.3.1** (what we ship) | **No.** `cyanrip_log` is a bare `vprintf` ([v0.9.3.1](https://github.com/cyanreg/cyanrip/blob/v0.9.3.1/src/cyanrip_log.c)) | Everything arrives at the end |
| cyanrip master since 25 Mar 2026 | Yes: `vprintf(...); fflush(stdout);`, commit c4740bcfad, "Flush stdout after writing line to buffer (#132)" | Live |
| **redumper b775** | Yes: progress is `\r` then `std::flush`; lines end with `std::endl` ([logger.ixx](https://github.com/superg/redumper/blob/b775/utils/logger.ixx)) | Live |
| **ffmpeg** | Yes: stats go to stderr with `fflush(stderr)` every 0.5 s ([ffmpeg.c at n8.0](https://github.com/FFmpeg/FFmpeg/blob/n8.0/fftools/ffmpeg.c)); `-progress` flushes too | Live |
| **makemkvcon -r** | Yes: "All output is line-based and output is flushed on line end" ([usage.txt](https://www.makemkv.com/developers/usage.txt)) | Live |

**So cyanrip is the only reason for the pseudo-terminal, and the fix is one
line upstream has already made.** We already patch cyanrip (the
MusicBrainz stub in `build-cyanrip.sh`), so adding upstream's `fflush`
commit as a second patch costs little. Taking a release that includes it
does the same, if cyanrip makes one.

### 3.3 Why not Windows' pseudo-console

Windows 10 1809 and later have one (ConPTY, `CreatePseudoConsole`), and the
[`portable-pty`](https://docs.rs/portable-pty) crate wraps it. It's the
wrong tool here *(read)*:
- It doesn't pass the tool's bytes through. It keeps its own screen buffer and "renders changed regions … as UTF-8 encoded text/VT", generating "the VT sequences necessary to move the cursor" ([Microsoft](https://devblogs.microsoft.com/commandline/windows-command-line-introducing-the-windows-pseudo-console-conpty/)). A tool's `\r` progress line can come back as cursor moves and redraws, which the line reader would have to interpret.
- `portable-pty` creates it with `PSEUDOCONSOLE_INHERIT_CURSOR`, and Microsoft warns that the caller must then answer a cursor-position request or the program "may hang" ([pseudocon.rs](https://github.com/wezterm/wezterm/blob/c8c592d19298cb68b8370f4e151f1591333b7f4a/pty/src/win/pseudocon.rs), [CreatePseudoConsole](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole)).

### 3.4 Recommendation

**Plain pipes on Windows, with a cyanrip that flushes.**
- Keep the pseudo-terminal on Linux and macOS (macOS's `openpty` is the same BSD call: [Apple Libc pty.c](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/util/pty.c), *(read)*) until the pipe path is proven.
- **The pipe path can be proven on Linux, with the real drives:** run a rip with the flushing cyanrip through pipes, and check the progress arrives as it happens. That's the one piece of Windows tool-running that can be tested on real hardware without Windows.
- If it works, using pipes everywhere removes code: one way to run tools, not two.

Reading two pipes (stdout and stderr) rather than one terminal means the
two can interleave differently. **To find out:** whether any parser relies
on the order between them. redumper and cyanrip print progress to stdout;
ffmpeg prints to stderr.

---

## 4. The tools on each OS, and their licences

### 4.1 Where each comes from

| Tool | Linux (now) | Windows x64 | macOS arm64 |
|---|---|---|---|
| **ffmpeg, ffprobe** | BtbN GPL build; Flathub would use the runtime's ffmpeg (see the plan) | BtbN `win64-gpl`. Its scripts enable x264, x265, SVT-AV1, LAME, Opus and Vorbis for every GPL variant ([scripts.d](https://github.com/FFmpeg-Builds/FFmpeg-Builds/tree/master/scripts.d)) *(read)*. Running `-encoders` on it is **to find out**, and `fetch-tools.sh`'s check would do it | **No build we'd want to depend on.** martin-riedl.de has a current arm64 GPL build with all six encoders and a public build script ([site](https://ffmpeg.martin-riedl.de/)). evermeet.cx is Intel only and lacks SVT-AV1. osxexperts says "for educational purposes only" *(read)*. **Recommended: build it ourselves on the macOS runner**, as the first app did (about forty minutes cold, then cached) |
| **cyanrip** | Built by `build-cyanrip.sh` | Upstream publishes `cyanrip-win64.exe`, but it's built from unpinned sources and **includes cyanrip's own MusicBrainz client**, which we remove on purpose ([mingw-build.sh](https://github.com/cyanreg/cyanrip/tree/master/.github)) *(read)*. **Recommended: build our own**, with the same stub and the `fflush` patch: MSYS2 on the Windows runner, or a MinGW cross-build on Linux (**to find out** which is less fragile) | No release binary and no Homebrew formula *(read)*. Upstream's master has a macOS static-build job that hasn't run yet. **Build our own**: the first app had a `build-cyanrip-macos.sh` that mirrors the Linux script |
| **redumper** | Upstream's static build | Upstream `windows-x64` zip *(read: b775)* | Upstream `macos-arm64` zip, which is `bin/redumper` plus its own C++ runtime in `lib/`. **Ship the whole folder**, not just the binary *(read: README)* |
| **makemkvcon** | Not bundled (section 5) | Not bundled | Not bundled |

**Note on BtbN.** It keeps "the last build of each month for two years" and
daily builds for 14 days *(read: README)*. That's fine for pinning, but it
isn't somewhere we can rely on for the source (4.3).

**Finding the tools.** `tool::command` today looks at `TRIP_<NAME>`, then
PATH. On Windows and macOS the tools sit beside the app (in its install
folder, or in `Trip.app/Contents/MacOS/`), so it has to look next
to `current_exe()` first. `portability.md` already lists that change.

### 4.2 The licences, and a correction

| Component | Licence | Source |
|---|---|---|
| ffmpeg, ffprobe (as built) | GPL-3.0 (`--enable-gpl --enable-version3`) | BtbN `variants/defaults-gpl.sh` *(read)* |
| x264, x265 inside ffmpeg | GPL-2.0-or-later | their headers *(read)* |
| SVT-AV1 inside ffmpeg | BSD-3-Clause-Clear, plus the AOM Patent License 1.0 | `LICENSE.md`, `PATENTS.md` *(read)* |
| LAME | LGPL | `COPYING` *(read; "or later" not checked)* |
| Opus, Vorbis, Ogg | BSD-style | *(read)* |
| redumper | GPL-3.0 | `LICENSE` *(read)* |
| cyanrip's own code | LGPL-2.1-or-later | `LICENSE.md`, `meson.build` *(read)* |
| **libcdio, libcdio-paranoia** (linked into cyanrip) | **GPL-3.0-or-later** | `COPYING` and file headers of [libcdio](https://github.com/libcdio/libcdio) and [libcdio-paranoia](https://github.com/rocky/libcdio-paranoia) *(read)* |

**Correction:** the cyanrip we ship statically links libcdio and
libcdio-paranoia, which are GPL-3.0-or-later. **So the cyanrip binary as a
whole is GPL-3.0-or-later, not LGPL-2.1.** That's lawful (LGPL code can be
used under the GPL), and our own licence is already GPL-3.0-or-later, so
nothing has to change in what we ship. But `THIRD-PARTY.md` says LGPL-2.1
and talks about relinking, and that should be corrected to say GPL-3.0 for
the binary. The same applies to upstream's Windows build and to any macOS
build. This is phase X0's first item.

### 4.3 What GPL compliance actually requires

From the licences themselves *(read: [GPL-3.0](https://www.gnu.org/licenses/gpl-3.0.txt), [LGPL-2.1](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt), [GPL FAQ](https://www.gnu.org/licenses/gpl-faq.html), [ffmpeg legal](https://ffmpeg.org/legal.html))*:

- **For a download, the route is GPL-3.0 section 6(d):** offer the Corresponding Source "in the same way through the same place". Sections 6(a) and 6(b), including the written offer, are for physical products. **A written offer alone isn't the right route for a download.** The one in `THIRD-PARTY.md` can stay as a backstop, but it isn't enough on its own.
- **The source can be on someone else's server**, but "regardless of what server hosts the Corresponding Source, you remain obligated to ensure that it is available for as long as needed to satisfy these requirements" (6(d)). The FAQ says the same, and that the source must match the binary exactly, "not an older version and not a newer version".
- **Corresponding Source includes the build scripts** (section 1: "scripts to control those activities").
- **Bundling separate programs is an aggregate** (section 5). It doesn't pull the app's licence onto the tools or the tools' onto each other. Each keeps its own.
- **ffmpeg's own checklist** says to distribute its source "no matter if you modified it or not", say how it was configured, and do the same for each external library compiled in.
- **A modified program** (our cyanrip, with the stub and the `fflush` patch) must carry "prominent notices stating that you modified it, and giving a relevant date" (GPL-3.0 section 5(a)).

**What that means for us:** linking to BtbN or upstream isn't enough on
its own.
- BtbN ships no source and deletes builds after two years.
- cyanrip's own Windows build can't be traced to exact sources.
- redumper's source is GitHub's automatic tag archive, which we don't control.

So **each release attaches a source archive of its own**, kept as long as
the release is up. For each bundled binary on each OS, it contains:
- the exact upstream sources;
- the build scripts and patches;
- the configure line.

For ffmpeg, that means every library BtbN compiled in, at the commits its
scripts pinned for that build. Collecting those is the awkward part, which
is one more reason to build ffmpeg ourselves from pinned tarballs, as
`build-cyanrip.sh` already does for its own ffmpeg. **To find out:**
whether BtbN's scripts can be run in our CI at a pinned commit with their
downloads saved, which would give the Windows build and its sources
together.

### 4.4 Licence checklist for each package

The same in every package (`.zip`/installer, `.dmg`, `.flatpak`):

| Item | Where it goes |
|---|---|
| Trip's `LICENSE` (GPL-3.0) | Windows: install folder. macOS: `Contents/Resources/`. Flatpak: `/app/share/licenses/<app id>/` (Flathub's place for it) |
| `THIRD-PARTY.md` (corrected), listing each tool, its version, its licence and the URL of the source archive | Beside the licence |
| `PROVENANCE.txt`: for each binary, where it came from, its sha256, its version string, and its configure line | Beside the licence (already done for the flatpak) |
| A `licenses/` folder with the full text of each licence named in 4.2: GPL-3.0, GPL-2.0 (x264, x265), LGPL-2.1 (cyanrip, LAME), BSD-3-Clause-Clear and the AOM patent licence (SVT-AV1), the BSD notices (Opus, Vorbis, Ogg), OFL-1.1 (fonts), BSD-3-Clause (MAME's CHD core) | Beside the licence |
| The Rust crates' licences, generated from `Cargo.lock` (`cargo about`, or `cargo deny`'s list) | `licenses/rust-crates.txt` (**to find out** which tool; `cargo deny` already checks them) |
| A notice that cyanrip is modified, with the two patches and the date | In `THIRD-PARTY.md` and in the source archive |
| **On the release page:** `trip-<ver>-sources-<os>.tar.xz` with every tool's corresponding source and build scripts, and a line saying where it is | GitHub Release assets |

The "About" window should link to these. CI should fail a release whose
package lacks any of them, just as it already fails one that contains
MakeMKV.

**Flathub** builds from source URLs in the manifest. ffmpeg would then
come from the runtime, so its source is the runtime's responsibility.
**To find out:** whether Flathub mirrors the sources it builds from, or
whether the release's source archive is still needed there.

---

## 5. MakeMKV

### 5.1 What its licence allows

From the EULA inside `makemkv-bin-2.0.0.tar.gz` *(read)*:
- "You are allowed to redistribute the Software in it original unmodified form." There's a further exception for repackaging the Linux part for a distribution package.
- "You may not sell, rent, lease or sublicense the Software", and may not modify or reverse-engineer it.
- "the Software may bypass certain copy protection mechanisms … You, the user, must check your local laws."

**Not all of makemkv-oss is LGPL.** Its `License.txt` is the same EULA,
with LGPL-2.1 applying only to files whose headers say so. makemkv-bin,
which holds `makemkvcon` itself, is proprietary *(read)*.

**So redistribution is allowed.** The comment in `makemkv.rs` that says it
isn't bundled because it "allows redistribution only unmodified" gives the
wrong reason. The real reasons not to bundle it are these:
- **It decrypts copy protection.** Distributing circumvention software is a different act from distributing a program that drives it (`THIRD-PARTY.md`; CDPA s.296ZB, DMCA §1201). CI already refuses any artifact that contains it.
- **Windows signing.** SignPath Foundation's terms (section 6) forbid "proprietary code" in what they sign, and software that can "circumvent security measures". Bundling `makemkvcon` would rule that out.
- **The key.** MakeMKV is free while in beta with a key that changes. The current one is "valid until end of November 2026" ([forum](https://forum.makemkv.com/forum/viewtopic.php?f=5&t=1053)) *(read)*. A bundled copy would age out of step with it.

**The Flathub `com.makemkv.MakeMKV` app bundles makemkv-bin; it doesn't
download it at install.**
- Its manifest builds both parts from makemkv.com's tarballs, noting "author allows redistribution as long as eula is referenced in package description" ([manifest](https://github.com/flathub/com.makemkv.MakeMKV/blob/HEAD/com.makemkv.MakeMKV.yaml)).
- It isn't verified on Flathub (its listing says "GuinpinSoft inc" but carries no verification) *(read)*.
- So on Linux, "install MakeMKV from Flathub" stays the easy answer, and it's someone else distributing it.

### 5.2 Downloading it on the user's OK, versus asking where it is

The closest we can get to "packaged with the app" is the app fetching
MakeMKV from makemkv.com when the user asks it to:
- We distribute nothing.
- The user gets the vendor's own file and accepts the vendor's own licence.

What makemkv.com offers *(read: [download page](https://www.makemkv.com/download/))*:
- **Windows:** `Setup_MakeMKV_v2.0.0.exe`, an NSIS installer. It installs for the whole machine, so Windows asks for admin (winget lists it as `Scope: machine`, `nullsoft`, *(read: [winget-pkgs](https://github.com/microsoft/winget-pkgs/blob/master/manifests/g/GuinpinSoft/MakeMKV/v2.0.0/GuinpinSoft.MakeMKV.installer.yaml))*).
- **macOS:** `makemkv_v2.0.0_osx.dmg`, which the page says isn't signed by Apple. **Homebrew's cask for it was disabled on 1 September 2026** for failing Gatekeeper *(read: [cask](https://github.com/Homebrew/homebrew-cask/blob/HEAD/Casks/m/makemkv.rb))*, so Homebrew is no longer a way to get it.
- **Linux:** source tarballs, plus the Flathub app.
- **Checksums:** a PGP-signed `makemkv-sha-2.0.0.txt` with the SHA-256 of each file ([file](https://www.makemkv.com/download/makemkv-sha-2.0.0.txt)).

| | User says where it is | App downloads it on the user's OK |
|---|---|---|
| Who installs | The user, from makemkv.com | The vendor's installer, started by the app, which the user watches and clicks through |
| What we distribute | Nothing | Nothing; we fetch from makemkv.com |
| What can break | Nothing of ours | makemkv.com's file names and checksum format change with each version. The app has to read the checksum file, not pin a hash. Checking the PGP signature needs GuinpinSoft's key, which is **to find out** |
| macOS | The user's own copy, opened once by them, so Gatekeeper has already been through it | A file we download has no quarantine flag, so it opens without Gatekeeper's prompt. The first app planned this and wrote it down as a deliberate choice; Homebrew has spent a year closing that gap. It looks like slipping past Apple's check, even though the checksum is a real check |
| Effort | Small: the setting is being built now | Medium, and per OS |

**Recommendation:**
- **For the first release, the user says where it is, helped by detection**, as below.
- Add **one button that opens makemkv.com's download page** in the browser.
- **Leave the in-app download for later**, and only for Windows, where it's clean: download, check against the SHA-256 file, then start MakeMKV's own installer visibly, so the user sees its licence and Windows' admin prompt.
- On macOS, don't do it. The user installs MakeMKV and opens it once, which deals with Gatekeeper in the normal way.

### 5.3 Consent and the fallback on Windows and macOS

The app id work adds, on Linux:
- an app-level setting for MakeMKV's approved location;
- a rule that nothing is run before it's approved;
- a `flatpak override` command for the permission the flatpak needs to reach a host MakeMKV.

**The same rule on the other two:**

1. **Looking is not running.** The app may check whether these files exist, without running anything:
   - Windows: `%ProgramFiles%\MakeMKV\makemkvcon64.exe`, `%ProgramFiles(x86)%\MakeMKV\makemkvcon64.exe` and `makemkvcon.exe`. The installer is 32-bit, so its uninstall entry in the registry is under `WOW6432Node`; reading it in both registry views finds installs in other folders (from the first app's research; **to find out** again).
   - macOS: `/Applications/MakeMKV.app/Contents/MacOS/makemkvcon`, and the same under `~/Applications`.
   - Linux: as the app id work defines it.
2. **What it finds is offered, not used.** In the Video-disc step's settings, the place where it says where MakeMKV is:
   - *"MakeMKV is installed at C:\Program Files (x86)\MakeMKV. Use it?"* with **Use this one**, **Choose another…** (a file picker), and **Get MakeMKV** (opens makemkv.com).
   - If nothing is found: only the last two, and a line saying video discs need it.
3. **Nothing runs until the user picks one.** That includes MakeMKV's version check and its drive list, and also the read of its key from MakeMKV's settings (`video/key.rs`). The approved path is saved in the app's settings, not per recipe.
4. **If the approved file changes or goes away**, the app says so and asks again. It never falls back to another copy it found. **To find out:** whether to also store the file's size and date, so a replaced binary is noticed.
5. **The `TRIP_MAKEMKVCON` variable** stays as the developer's way round all this. It counts as approval, since only someone at the command line can set it.
6. **macOS Gatekeeper.** If the user's MakeMKV has never been opened, running `makemkvcon` from inside it may be blocked by Gatekeeper with a message the app can't see (**to find out**). The safe advice in the app: *open MakeMKV once yourself before using it here*.

---

## 6. Packaging and signing

### 6.1 Signing and the pseudonym

Every route to a trusted signature has to be weighed against the
project's pseudonym, because **signing certificates show a legal name**.

| Route | Cost | Name users see | Can a UK individual use it? |
|---|---|---|---|
| **SignPath Foundation** (Windows) | Free for open source | **"SignPath Foundation is the publisher"**: no personal identification needed ([terms](https://signpath.org/terms), [signpath.org](https://signpath.org/)) *(read)* | Yes |
| Microsoft Artifact Signing (was Trusted Signing) | About $9.99 a month | The verified legal name, from government ID | **No**: individuals in the USA and Canada only ([Microsoft](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options), [quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart)) *(read)* |
| OV certificate, commercial | $150–300 a year, on a hardware token | Your legal name | Yes |
| Certum open-source certificate | €69 with a card and reader | "Open Source Developer" plus your legal name ([Certum](https://shop.certum.eu/open-source-code-signing.html)) *(read)* | Yes |
| EV certificate | More | Legal entity | No longer skips SmartScreen: "That behavior was removed in 2024" (Microsoft, above) *(read)* |
| **Apple Developer Program** (macOS) | $99 a year ([Apple](https://developer.apple.com/programs/)) | "Use your legal name … Your name will be displayed as the seller name"; certificates read `Developer ID Application: <name> (TEAMID)` ([enrol](https://developer.apple.com/programs/enroll/)) *(read)* | Yes, but only under the legal name. An organisation needs a real legal entity and a D-U-N-S number, and a UK company's directors are public at Companies House |

**SignPath's conditions** *(read: terms)*:
- an OSI licence for every component, and "no proprietary code";
- the team maintains all the source and build scripts;
- builds on CI, with MFA for everyone;
- a "Code signing policy" section on the home and download pages;
- "Released: the project must already be released in the form that should be signed". So **the first Windows release goes out unsigned, and signing starts after it.**
- One clause to ask them about before applying: "Software must not include features designed to … circumvent security measures of their execution environment". It's about the computer's own security, not copy protection, and the app contains no decryption code (CI checks), but an app that drives MakeMKV should ask rather than assume.

**What users see unsigned:**
- **Windows:** "Windows protected your PC". The user chooses *More info* → *Run anyway*. Smart App Control, where it's on, blocks unsigned files with no reputation outright ([Microsoft](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)) *(read)*. Even signed files show "unrecognized" until they build reputation.
- **macOS 15 and later:** Control-click → Open no longer works ([Apple](https://developer.apple.com/news/?id=saqachfa)) *(read)*. The steps are:
  1. Double-click the app; it's blocked.
  2. Open System Settings → Privacy & Security, and click **Open Anyway** (shown for about an hour after the attempt).
  3. Enter the password.

  ([Apple support](https://support.apple.com/guide/mac-help/mh40616/mac)) *(read)*. The release notes and README must give exactly these steps.

**Homebrew** won't take an unsigned app: casks "must pass Homebrew's
Gatekeeper checks" ([Acceptable Casks](https://docs.brew.sh/Acceptable-Casks)),
and Homebrew dropped `--no-quarantine` as circumvention ([5.0.0](https://brew.sh/2025/11/12/homebrew-5.0.0))
*(read)*. Our own tap is allowed. Since Homebrew 6.0 a user has to trust a
tap explicitly, and an unsigned app from it still meets Gatekeeper. So a
tap adds little until the app is notarised.

**winget** doesn't require signing in what it says. It scans each
submission with antivirus and Defender, and the installer has to come from
the project's own release location ([policies](https://learn.microsoft.com/en-us/windows/package-manager/package/windows-package-manager-policies))
*(read)*. GitHub Releases qualifies.

### 6.2 Windows

**Installer form:**
- **Inno Setup:** a per-user install with no admin prompt, Start menu entry and uninstaller. Widely used. Its terms for non-commercial open source are **to find out** (recent versions added a commercial licence).
- **MSI through `cargo-wix` (WiX):** MSI is what companies deploy. It's per-machine by default, which means an admin prompt. WiX's current terms (its "Open Source Maintenance Fee") are **to find out**.
- **A portable zip:** the same folder, unzipped anywhere. No admin, nothing to uninstall. winget accepts zips.

**Recommendation:** a per-user Inno Setup installer plus the same files
as a zip, both from one CI job.
- Target Windows 10 1809 or later and Windows 11, x64 only.
- Windows on Arm runs x64 programs through emulation. A native arm64 build can wait (redumper and BtbN have arm64 builds; cyanrip doesn't).

**Layout:** `Trip\trip.exe`, `tripr.exe`, `tools\` (ffmpeg,
ffprobe, cyanrip, redumper), `licenses\`, `THIRD-PARTY.md`,
`PROVENANCE.txt`.

### 6.3 macOS

- A `Trip.app` in a `.dmg`, with the tools in `Contents/MacOS/` and redumper's own `lib/` beside it.
- **arm64 only.** Reasons:
  - Apple stopped selling Intel Macs in 2023.
  - The only current Intel ffmpeg build we found is stale (martin-riedl's dates from January 2026).
  - GitHub's Intel runner (`macos-15-intel`) is "available … until August 2027" ([runner-images #13045](https://github.com/actions/runner-images/issues/13045)) *(read)*.

  A universal build would double the tool builds for a shrinking group. Add it only if people ask.
- **Lowest macOS version:** **to find out** what egui and wgpu need. Recommended: 14 (Sonoma) or later, since older versions get no security updates.
- **Signing.**
  - Unsigned: every binary is ad-hoc signed (arm64 needs it to run at all), and the README gives the Open Anyway steps.
  - Signed: all executables, the tools included, get the hardened runtime, a Developer ID signature and a secure timestamp, and the whole `.dmg` is notarised ([Apple](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)) *(read)*.

  That's decision C1.

### 6.4 Linux

As already planned:
- **Flathub** under `io.github.vresistor.trip`, built from source;
- the `.flatpak` bundle on GitHub Releases until then.

The tarball/AppImage in `portability.md` (item 3) is optional and not
part of the first release.

---

## 7. CI

### 7.1 The matrix

Free on public repositories ([GitHub](https://docs.github.com/en/actions/reference/runners/github-hosted-runners))
*(read)*.

| Job | Runner *(read: [runner-images](https://github.com/actions/runner-images))* | Does |
|---|---|---|
| check (exists) | `ubuntu-24.04` | deny, audit, fmt, clippy, tests |
| check-windows | `windows-2025` (x64) | clippy and tests |
| check-macos | `macos-15` (arm64) | clippy and tests |
| flatpak (exists) | `ubuntu-24.04` | the bundle |
| windows | `windows-2025` | tools (cached against the scripts' hashes), build, zip and installer, the "nothing we mustn't ship" check, the licence check |
| macos | `macos-15` | ffmpeg and cyanrip from source (cached), redumper, build, `.app`, `.dmg`, the same checks |
| sources | `ubuntu-24.04` | the source archives for every tool on every OS |
| release | `ubuntu-24.04` | draft release with all of it |

`macos-14` images were retired on 2 November 2026 *(read: runner-images
#15226)*, so `macos-15` it is, or `macos-26`.

### 7.2 Tests that only run on Linux today

Found by reading the tests, not by running them on another OS:

| Test | Why it's Linux-only | Change |
|---|---|---|
| `redumper_dies.rs`, `sector_order.rs`, `unreadable.rs`, `stop.rs` | Stand-in tools are `#!/bin/sh` scripts made executable with `PermissionsExt` | Replace the scripts with one small Rust stand-in program the tests build. This fits the drive recordings in the plan (P2): the stand-in replays a recording, so the same file tests every OS |
| `stop.rs`, `open_tray.rs`, `tool.rs`'s own test | Read `/proc` to check processes and open files | `cfg(target_os = "linux")` for the `/proc` parts; what they check is Linux behaviour |
| `hidden_track.rs`, `re_reads.rs`, `wires.rs`, `open_tray.rs`, `app/tests/ui.rs` | Use `/dev/sr0` names | Probably only labels; once the drive type (2.3) lands, make them names for the OS |
| `place.rs` tests | `std::os::unix::fs::symlink` | `cfg(unix)`, or the Windows symlink call |
| `app/tests/ui.rs` window tests | Draw frames with lavapipe | **To find out:** whether wgpu draws on the runners (WARP through DX12 on Windows; Metal on the macOS runners). If not, skip them there |
| `encode_tags.rs` | Needs ffmpeg on PATH; skips without it | Point it at the bundled ffmpeg on each runner |

---

## 8. Testing without the hardware

### 8.1 Windows: a virtual machine on this computer

A Windows 11 guest under KVM/virt-manager, with a USB drive given to it.
Two ways:
- **Pass the single USB device** (virt-manager's "USB host device"). Simple. One Proxmox user found it read files in Explorer, but "EAC didn't read the drive, and I couldn't even use eject" ([forum](https://forum.proxmox.com/threads/is-it-possible-to-pass-through-a-sata-dvd-drive-connected-over-a-usb-interface.106901/)) *(read)*.
- **Pass a whole USB controller** (PCI passthrough, which needs IOMMU on). Windows then has the real controller, and the drive is as real as on a Windows PC. The same thread says that "works fine" *(read)*. **To find out:** whether this machine's IOMMU groups let a USB controller go on its own.

**Is it realistic for real rips?** With a whole controller, yes, as far as
anything found says. With a single device, maybe not. The test settles it:
- an album already ripped on Linux must give the same AccurateRip results and the same file hashes;
- Koudelka Discs 1–3 must match Redump again.

These are exact checks, so a VM that reads differently can't pass by
accident. Timing-sensitive problems (the MATSHITA's stalls and USB resets)
may behave differently in a VM, so drive quirks found there go into the
drive recordings, not straight into code.

Windows itself: it installs and runs unactivated, with limits on
personalisation (**to find out** today's exact terms). It needs no
purchase for testing.

### 8.2 macOS

Running macOS in a VM on non-Apple hardware breaks Apple's licence: it
allows macOS only "on Apple-branded Systems", and extra virtual copies
only "on each Apple-branded computer you own or control" ([macOS licence](https://www.apple.com/legal/sla/docs/macOSSequoia.pdf))
*(read)*. So that's out.

The honest options:

| Option | Cost | Tests real discs? |
|---|---|---|
| CI runners (`macos-15`) | Free | No: builds, unit tests, simulated drives |
| AWS EC2 Mac (M1, `mac2.metal`) | $0.65 an hour, **24-hour minimum**, so about $16 a go ([AWS](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-mac-instances.html), pricing *(read)*) | No: no USB. It does show the app opening, Gatekeeper's prompts, and signing |
| MacStadium Mac mini M2 | $109 a month ([pricing](https://www.macstadium.com/pricing)) *(read)* | No: no USB |
| **Borrow a Mac** and plug in our own two USB drives | Free | **Yes**: the same drives, so results compare directly with Linux |
| Buy a second-hand Apple Silicon Mac mini | **To find out** (used M1 prices) | Yes |
| **Testers** with their own Mac and drive | Free | Yes, with the report from "Copy a report" (plan 2.4) |

**Recommendation:**
- Borrow a Mac for a day, or a week, before the first release, and run the same exact checks as for Windows on our own drives.
- Ask testers for more drives.
- Use the CI runner for everything else.
- A rented Mac adds little over CI.

### 8.3 What "tested" means for the first release

| OS | Tested means |
|---|---|
| Linux | As the plan's proof work: real CDs, PlayStation discs and, once MakeMKV is set up, a DVD, on both drives |
| Windows | In the VM, on a standard (non-admin) account, with at least one of the two drives: an album matching its Linux rip (AccurateRip and hashes); a PlayStation disc matching Redump; a DVD through MakeMKV; Stop mid-rip; eject. Installer and zip both tried |
| macOS | CI green. On a borrowed Mac with our drives: the same album and PlayStation checks, and a DVD through MakeMKV. If no Mac is found in time: the release says plainly "built and tested in CI; not yet tested on real discs on macOS", and the README's Status says the same |

---

## 9. Phases

Each ends with something working. X0 and X1 need no other OS's hardware
and can start now.

| Phase | What | Done when |
|---|---|---|
| **X0** Groundwork on Linux | `THIRD-PARTY.md`: cyanrip binary is GPL-3.0 (4.2), and MakeMKV's reason corrected (5.1). cyanrip gets the `fflush` patch. A drive type with each tool's spelling (2.3). `tool::command` looks beside the app first. `dirs` for config and cache. Pipes as a second way to run tools | A real album rips on Linux **through pipes**, with live progress and the same AccurateRip result as through the pseudo-terminal |
| **X1** Builds and tests on all three | `cfg` the Unix-only code; the Rust stand-in tool replacing the shell scripts; check jobs on Windows and macOS (7.1) | All three check jobs green, every test that isn't about Linux itself running on each |
| **X2** Windows | The drive layer (2.1), the tools (4.1), `CREATE_NO_WINDOW`, safe file names, long paths, the installer and zip, licence files (4.4), the source archive | **8.3's Windows row** passes in the VM |
| **X3** macOS | The drive layer (2.2), the tools built in CI, the `.app` and `.dmg`, `Info.plist` usage strings, licence files, the source archive | CI builds the `.dmg`; **8.3's macOS row** passes on a borrowed Mac, or the release says it hasn't (decision C2) |
| **X4** MakeMKV on Windows and macOS | 5.3: detection that only looks, the approval in settings, Get MakeMKV | On Windows (VM) and macOS: no `makemkvcon` process starts before Use this one is pressed (checked in a test with a stand-in), and a DVD rips after it |
| **X5** Release checks | CI fails a package missing any item in 4.4, or containing MakeMKV or libdvdcss; source archives attached to the draft release | A draft release with all three packages and their source archives, made by a tag |
| **X6** Signing | Apply to SignPath once 0.1.0 is out (they need a release first); macOS per decision C1 | Windows builds after 0.1.0 are signed by SignPath, or their refusal is recorded |
| **X7** Package managers | winget manifest pointing at our release; Homebrew only if the app is notarised | Listed in winget |

X2 and X3 can run side by side after X1. X4 needs X2 or X3 to have drives.
X6 and X7 come after the first release by their nature.

---

## 10. Risks

| Risk | Fallback |
|---|---|
| A standard Windows account can't do SCSI pass-through | Say so clearly in the app and the README, and tell the user to run it as administrator. Don't make the installer ask for admin up front |
| USB passthrough of one device doesn't rip properly | Pass a whole USB controller; failing that, Windows installed on a spare disk and started from it, with the same exact checks |
| BtbN's sources can't be collected exactly | Build ffmpeg ourselves on Windows too (MSYS2 or cross from Linux), from pinned tarballs, as on macOS |
| cyanrip won't build for Windows | Take upstream's `cyanrip-win64.exe` from a pinned nightly that has the `fflush` fix, and accept its MusicBrainz client. It's only reached if called with `-R`, which we never pass (**to find out**). The licence notes and the source archive then cover its sources as fetched |
| Pipes give a different line order that breaks a parser | Keep the pseudo-terminal on Linux and macOS; use pipes only on Windows, where there's no choice |
| No Mac to test on before release | Release macOS labelled as untested on real discs (C2); fix from testers' reports |
| SignPath refuses because of MakeMKV | Windows stays unsigned, with the SmartScreen steps in the README. Buying a certificate would put a legal name on the app |
| Apple signing needs the legal name | Unsigned, ad-hoc-signed `.dmg` with clear Open Anyway steps (C1) |
| makemkvcon from an unopened MakeMKV.app is blocked by Gatekeeper | The app tells the user to open MakeMKV once first, and says what it saw |
| A long rip in a VM hits USB resets that never happen on bare metal | Record them as drive recordings, and test the code paths with those rather than changing code for a VM |

---

## Decisions for you

**C1. Signing, and what to buy.**
- **Windows:** apply to **SignPath Foundation** (free, signs under its own name, so the pseudonym stays intact) after 0.1.0, having first asked them about the "circumvent security measures" clause. **Buy nothing for Windows.**
  - Artifact Signing isn't open to UK individuals.
  - An OV or Certum certificate costs €69–$300 a year and shows your legal name.
  - EV no longer helps.
- **macOS:** the $99 a year Apple programme is the only way to notarise, and it shows the seller's legal name on every copy. There's no Apple route that keeps the pseudonym.
  - **Recommended: don't buy it for now.** Ship an ad-hoc-signed `.dmg` with exact Open Anyway steps.
  - Revisit if the project ever has a maintainer or organisation willing to put its name on it.
  - What you give up: Homebrew, a smooth first launch, and some users who won't get past the prompt.

**C2. macOS testing.**
- **Recommended: borrow a Mac** for the checks in 8.3, using our own drives, and ask testers for more.
- If none is found before 0.1.0, release macOS labelled "not yet tested on real discs".
- The other way is to hold macOS back until it has been tested. That would put the first release on two OSes instead of three.

**C3. macOS versions and chips.**
- **Recommended: Apple Silicon only, macOS 14 or later** (the minimum is to be confirmed in X3).
- Intel would need a second set of tool builds on a runner that goes in August 2027.

**C4. Windows installer form.**
- **Recommended: a per-user Inno Setup installer plus a zip**, Windows 10 1809+ and 11, x64.
- MSI only if someone deploying it at work asks. Inno Setup's and WiX's current terms to be checked first.

**C5. MakeMKV fetched by the app.**
- **Recommended for 0.1.0:** detection that only looks, the user approving the location, and a Get MakeMKV button that opens makemkv.com.
- **Later, Windows only:** the app downloads the vendor's installer on the user's OK, checks it against makemkv.com's SHA-256 file, and starts it in front of them.
- **Not on macOS**, where it would look like getting round Gatekeeper.
