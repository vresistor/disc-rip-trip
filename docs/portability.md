# Portability: what it takes to run Disc Rip Trip elsewhere

Written 2026-11-20 by reading the workspace (`crates/*`, `app/`), the
packaging, and CI, and updated the same day for the swap: the first app
(Tauri, in `src-tauri/`) is deleted and the flatpak ships this one. Things
checked by running them are marked *(ran)*.

**Headline.** The app's own code links almost nothing: just libc and
libstdc++ *(ran: `ldd`)*. CHD, tags, hashing and HTTP are built in. It runs
**three bundleable open tools** (cyanrip, redumper, ffmpeg/ffprobe), which
the flatpak bundles, and **one tool that can't be bundled** (MakeMKV). It
runs on Linux x86_64: from the flatpak, or from source with tools you find
yourself. It won't compile on Windows; on macOS it probably compiles but
finds no drives.

## 1. External programs the new app runs

Tools are found by `tool::command` (`crates/steps/src/real/tool.rs`):
`DRT_<NAME>` if set (a whole command line), otherwise the name on `PATH`.
It does **not** look beside its own binary. Inside the flatpak the tools are
in `/app/bin`, which is on `PATH`, so it finds them with no `DRT_*` set.

| Program | For | How found | Bundled today? | Licence | If it's missing |
|---|---|---|---|---|---|
| `cyanrip` | Audio CD: TOC, rip each track, AccurateRip, measuring the offset | `DRT_CYANRIP`, then PATH | In the flatpak (0.9.3) | LGPL-2.1+ | Read disc / Rip card turns red: *Failed*, `couldn't start "cyanrip": No such file or directory`. No install hint. **Measuring the offset fails silently** (`find_offset` returns `None` through `.ok()?`). |
| `redumper` | Image, Rip for games | `DRT_REDUMPER`, then PATH | In the flatpak (b744) | GPL-3.0 | Red card, `couldn't start "redumper"`. On Linux a drive-permission pre-check names the group to join first (`redumper.rs::drive_access`). |
| `ffmpeg` | Every encoder, ReplayGain (ebur128), Transcode, Remux, interlace check (idet), tag rewrite | `DRT_FFMPEG`, then PATH | In the flatpak (BtbN GPL build, with x265) | GPL-3.0 as built | Red card, `couldn't start "ffmpeg"`. If ffmpeg is there but lacks an encoder (e.g. `libx265`), the card shows ffmpeg's last line. |
| `ffprobe` | Checking a MakeMKV title is whole, field order, `drt fix-tags` | `DRT_FFPROBE`, then PATH | In the flatpak | GPL-3.0 | Whole-title check: red card, `couldn't run ffprobe`. In Transcode a missing ffprobe only loses the field-order hint; ffmpeg's idet still decides. |
| `makemkvcon` | Video disc (scan titles), Rip for DVD and Blu-ray | `DRT_MAKEMKVCON`, then PATH, then the installer's path for the OS (`/usr/bin`, `/Applications/MakeMKV.app/…`, `%ProgramFiles%\MakeMKV\makemkvcon64.exe`), then the `com.makemkv.MakeMKV` flatpak. Inside a flatpak it runs via `flatpak-spawn --host`. | **Never** (CI fails any artifact that contains it) | Proprietary; free while in beta (beta key) or paid | The one clear message: card says **"MakeMKV isn't installed"** with the hint *flatpak install flathub com.makemkv.MakeMKV, or makemkv.com*. |
| `gdbus` / `notify-send` | Desktop notification (Notify step): portal, then freedesktop, then notify-send | PATH | Present in the GNOME runtime *(ran)* | n/a (system) | Notify reports `desktop: …` as an error. ntfy over HTTP still works. |
| `gsettings` | Reduced motion (GNOME `enable-animations`) | PATH | GNOME runtime | n/a | Silent: animations stay on. Harmless. |
| `flatpak`, `flatpak-spawn`, `sh`, `pkill` | Finding and stopping MakeMKV across the sandbox | PATH / host | Host | n/a | MakeMKV reads as not installed; a Stop may leave makemkvcon holding the drive. |
| `chdman` | **Not run.** CHD is in-process through `libchdman-rs` (MAME's CHD core). | linked in | yes | BSD-3-Clause | n/a |
| `flac`, `metaflac`, `eject` | **Not used.** FLAC goes through ffmpeg, tags through `lofty`, eject is an ioctl. | n/a | n/a | n/a | n/a |

**At start-up** the window checks for cyanrip, redumper, ffmpeg and ffprobe
(`tool::missing`: the first word of `DRT_<NAME>`, else the name on `PATH`) and
says in the header which aren't there: *cyanrip isn't installed*. The cards
still fail with the raw error if you start a step anyway. MakeMKV isn't in
that check; its card says how to install it.

**ffmpeg encoders the recipes ask for:** `flac`, `alac`, `aac`, `wavpack`,
`pcm_s16le` (native), plus `libmp3lame`, `libopus`, `libvorbis`, `libx265`
(Transcode's default) and `libsvtav1`. A distro ffmpeg has to carry all of
these.

## 2. Libraries

| Need | What | Notes |
|---|---|---|
| Build: Rust | ≥ 1.89 (`rust-version`) | |
| Build: C/C++ | C++20 compiler (`libchdman-rs` `build.rs`: `build.std("c++20")`), plus C for vendored lzma, zlib, zstd, FLAC, utf8proc | Built from vendored source, offline (no `prebuilt` feature). CI uses ubuntu-24.04 for the compiler. |
| Build: CPU baseline | `libchdman-rs` builds its FLAC with `-mavx2 -mfma` by default on x86_64. Its own `build.rs` warns this **SIGILLs on pre-Haswell CPUs**. `LIBCHDMAN_CPU=portable` turns it off. | `scripts/build-flatpak.sh` sets it. A plain `cargo build` doesn't, which is fine for your own machine. |
| Link: dynamic | `libstdc++.so.6`, `libgcc_s`, `libm`, `libc` *(ran: `ldd`)* | A build on Fedora 44 needs **GLIBC_2.43** *(ran: `objdump -T`)*, so it won't load on Ubuntu 24.04 (2.39), Debian 12 (2.36), or the GNOME 50 runtime (2.42 *(ran)*). That's why the flatpak is compiled inside the GNOME SDK. |
| Runtime: dlopen | `libwayland-client` / `libxkbcommon` (+ `-x11`, `libX11`/`Xi`/`Xcursor` for X11), `libvulkan.so.1` or `libEGL` (wgpu: Vulkan or GLES on Linux, Metal on macOS, DX12/Vulkan on Windows) | Needs a working GPU driver or Mesa. Software Vulkan (lavapipe) is enough; CI's window tests use it. |
| Runtime: portals | `rfd` with `xdg-portal`: the folder picker uses `xdg-desktop-portal`, falling back to `zenity` (`rfd` source) | With neither, *pick folder* does nothing; the error only goes to the log. Opening folders goes through `opener` (xdg-open, `open`, ShellExecute). |
| Runtime: fonts | Schibsted Grotesk, JetBrains Mono and the symbols font are compiled in (`include_bytes!`) | System fonts only as a fallback for missing glyphs. |
| Pure Rust | `lofty` (tags), `reqwest` + rustls (no OpenSSL), `zip`, `quick-xml`, hashes, `zbus` (accesskit, portals) | No system libs. The binary does name `libdbus-1.so.3` in its strings, so some dependency probably dlopens it; the owner wasn't identified. |

## 3. Packaging

| Piece | What it does |
|---|---|
| `scripts/build-flatpak.sh` | Runs everything inside `org.gnome.Sdk//50` with `org.freedesktop.Sdk.Extension.rust-stable//25.08`, so glibc matches the runtime: `fetch-tools.sh` once into `target/flatpak/tools`, then `cargo build --release --locked -p drt-app -p drt` with `LIBCHDMAN_CPU=portable` into `target/flatpak`, then `flatpak-builder` and `flatpak build-bundle` to `dist/disc-rip-trip-<version>-x86_64.flatpak`. `--install` installs it. |
| `scripts/fetch-tools.sh` | ffmpeg and ffprobe from BtbN (static, GPL, checked for x264, x265, LAME, Opus, Vorbis and SVT-AV1), redumper b744 (static), cyanrip from `build-cyanrip.sh`. Writes `PROVENANCE.txt`. Linux x86_64 only. |
| `scripts/build-cyanrip.sh` | cyanrip 0.9.3 against an audio-only LGPL ffmpeg, lame and libcdio, static except glibc, libcurl and libxml2 (*(ran)*: needs GLIBC_2.38). |
| `packaging/flatpak/io.github.vresistor.disc-rip-trip.yml` | Packages the payload: `disc-rip-trip`, `drt` and the four tools into `/app/bin`, `PROVENANCE.txt` into `/app/share/doc`, the desktop file, metainfo and icons. GNOME 50 runtime. `--device=all` (`/dev/sr*` and the GPU), `--filesystem=host` (libraries anywhere, `xdg-music` included), `--share=network`, Wayland with X11 fallback, `--talk-name=org.freedesktop.Flatpak` (MakeMKV through `flatpak-spawn --host`). Notifications and the folder picker go through portals, which need no permission. |
| `.github/workflows/build.yml` | `check`: tests, clippy and rustfmt on ubuntu-24.04 with lavapipe. `flatpak`: builds the bundle, fails if anything MakeMKV- or libdvdcss-like is in it, uploads it as an artifact; on a `v*` tag, makes a draft release with the bundle and `tools-provenance.txt`. |

**Where the flatpak keeps things.** `$XDG_CONFIG_HOME` is
`~/.var/app/io.github.vresistor.disc-rip-trip/config`, so recipes and settings are in
`config/disc-rip-trip/`. The first app kept its `config.json` in the same
folder; this one only reads the drive offsets it recorded (`offset::from_old_app`). The first time the flatpak starts with no
`recipes.json` there, it copies `recipes.json` and `settings.json` from
`~/.config/disc-rip-trip/` (a build from source's), and leaves those where
they are (`Store::adopt`). Working files and downloaded lists go in
`~/.var/app/io.github.vresistor.disc-rip-trip/cache/disc-rip-trip/`.

**The app id** is `io.github.vresistor.disc-rip-trip`, so it maps to the
repository (`github.com/vresistor/disc-rip-trip`) the way Flathub expects.
Builds before the rename used `io.github.vresistor.DiscRipTrip`; flatpak
treats that as a different app, and nothing moves its folder in
`~/.var/app/` across. Move it by hand if you had one.

### What a user on a fresh machine would install

| OS | To run it |
|---|---|
| **Any Linux, x86_64** | The flatpak from Releases. MakeMKV from Flathub (`com.makemkv.MakeMKV`) for video. |
| **Fedora** (44, default repos *(ran: `dnf repoquery`)*), from source | Rust ≥ 1.89, `gcc-c++`, then `cargo build --release -p drt-app -p drt`. **cyanrip and redumper are not in Fedora's default repos**: build cyanrip with `scripts/build-cyanrip.sh`, take redumper's release build. `ffmpeg-free` has lame, opus, vorbis and svt-av1 but **no libx265**, so Transcode's default codec fails. You need full ffmpeg from RPM Fusion. |
| **Ubuntu 24.04** *(ran: `apt-cache policy`)*, from source | Rust (rustup; apt's may be too old), `build-essential`, `ffmpeg` (6.1.1, has x265). `cyanrip` **0.9.2** is in universe, older than the 0.9.3 the rip code was measured against, so it's untested. **No redumper package.** Debian 12 has **no cyanrip at all**. |
| **macOS** | Not tested and not built. If it compiles (unverified), the window opens, but **`drives()` reads `/sys/class/block`, so it lists no drives and no real recipe can start.** The TOC, disc-type and eject paths are stubbed (empty, `None`, `false`). |
| **Windows** | **Won't compile.** `tool.rs` uses `std::os::fd` and `libc::openpty` with no `cfg`. Config would land in `./disc-rip-trip` (no `HOME`/`XDG_*`). |

## 4. Platform reach

| Linux-only piece | Where | macOS would need | Windows would need |
|---|---|---|---|
| Drive listing (vendor and model from sysfs), disc present (`/sys/class/block/srN/size`) | `real/mod.rs::drives`, `has_disc`; `media.rs` | IOKit (`IOCDMedia`/`IODVDMedia`), or `drutil`/`diskutil` | `GetLogicalDrives` + `GetDriveType == DRIVE_CDROM`, `IOCTL_STORAGE_QUERY_PROPERTY` for the model |
| Disc type (MMC GET CONFIGURATION via `SG_IO`), TOC (`CDROMREADTOC*`), eject (`CDROMEJECT`) | `media.rs`, `mod.rs::eject` | SCSITaskInterface or DKIOCCD ioctls; `drutil eject` | `IOCTL_SCSI_PASS_THROUGH_DIRECT`, `IOCTL_CDROM_READ_TOC_EX`, `IOCTL_STORAGE_EJECT_MEDIA` |
| Pseudo-terminal for every tool (stdio is line-buffered only on a tty) | `tool.rs::pty` | `openpty` exists, so likely fine | ConPTY (`CreatePseudoConsole`), or a crate such as `portable-pty` |
| `/dev/srN` device names passed to cyanrip, redumper, makemkvcon (`dev:`) | throughout | `disk2`-style names; cyanrip takes libcdio names | Drive letters (`D:`). Each tool spells them differently. |
| Drive permission check (`disk`/`optical` group) | `redumper.rs::drive_access` | Not needed (no-op) | Not needed (no-op) |
| Notifications via D-Bus (`gdbus`, `notify-send`) | `tell.rs` | `UNUserNotificationCenter` or `osascript` | Toast (WinRT) |
| Config, cache, data under `HOME`/`XDG_*` | `store.rs`, `engine/run.rs` | `~/Library/Application Support` (the `dirs` crate) | `%APPDATA%` (the `dirs` crate) |
| Flatpak awareness (`/.flatpak-info`, `flatpak-spawn`) | `makemkv.rs`, `tell.rs` | n/a | n/a |

The first app worked around this off Linux by listing drives through
`makemkvcon` (`src-tauri/src/disc.rs`, in git history), so drive detection
depended on MakeMKV. This one has no fallback at all.

Signing is the other gap. The first app's `.dmg` was unsigned and its `.exe`
triggered SmartScreen. On macOS, notarising also needs every bundled
tool signed with the same identity.

## 5. MakeMKV: is there an open alternative?

**Verified here:** the ffmpeg bundled in the flatpak is built with
`--enable-libdvdread --enable-libdvdnav --enable-libbluray`. It has the
**`dvdvideo` demuxer** and the **`bluray:` protocol** *(ran)*. The GNOME
runtime ships **no libdvdcss and no libaacs** *(ran)*. CI deliberately fails
any artifact that contains `*dvdcss*` or `*libmmbd*`, for the circumvention
reasons in `THIRD-PARTY.md`. So whatever replaces MakeMKV, the decryption part
stays a **user-supplied** install, as MakeMKV is now.

The rows below come from general knowledge and were **not verified here**.
Check them before acting on them.

| Option | Covers | Open? | The user would still install | Cost to adopt |
|---|---|---|---|---|
| ffmpeg `dvdvideo` (libdvdnav/libdvdread) + libdvdcss | DVD-Video, by title, with chapters, audio and subtitle streams | Yes (GPL / LGPL) | libdvdcss for CSS discs (RPM Fusion tainted, Debian `libdvd-pkg`, Homebrew). The bundled ffmpeg names `libdvdcss.` but doesn't contain it *(ran: grep)*, so libdvdread seems to load it at runtime. **In the flatpak a host libdvdcss isn't visible** to the bundled ffmpeg (it sees the runtime's `/usr`). It would need a way in: run a host ffmpeg via `flatpak-spawn`, a user-installed extension, or a path the user points at. | **Medium, about 1–3 weeks.** A second Video-disc backend: list titles (ffprobe or libdvdread), pick the main one, map streams, rip to MKV, then the existing checks. Discs with structure protection (dozens of decoy titles) are where MakeMKV is better. Needs real-disc testing. |
| libbluray + libaacs + `KEYDB.cfg` (+ libbdplus) | Blu-ray via ffmpeg `bluray:` | Yes (LGPL) | libaacs and a key database (community VUK/processing-key lists), libbdplus for BD+ | **Large, with partial coverage.** Newer discs often need keys nobody has published, and UHD is mostly out of reach. That would be a worse experience than MakeMKV, not a replacement. |
| HandBrakeCLI | DVD scan and encode | Yes (GPL) | libdvdcss again; no AACS at all | Replaces Transcode as well. It moves the "install one thing" problem rather than removing it. |
| Keep MakeMKV | DVD, BD, UHD (with LibreDrive) | No | MakeMKV + beta key | Already done. The key is read from MakeMKV's own settings (`video/key.rs`). |

**Verdict:** an open DVD path is feasible, and the ffmpeg it needs is already
bundled. Getting libdvdcss into the sandbox is the awkward part. The user
still has to install libdvdcss, so it changes *which* thing they install
rather than whether they install one. For Blu-ray
there's no open path that matches MakeMKV.

## 6. Gaps against "mostly dependency free"

| Gap | Severity |
|---|---|
| Only Linux x86_64 is packaged (the flatpak). No tarball or AppImage, no aarch64. | Medium |
| Built from source, cyanrip and redumper aren't packaged by Fedora or Debian 12, and Ubuntu's cyanrip is older than the one tested | Medium (the flatpak has them) |
| Distro ffmpeg varies (Fedora's lacks x265) | Medium (the flatpak has x265) |
| The start-up check finds a missing tool, not an ffmpeg without an encoder, and offset measuring still fails silently | Low |
| Windows doesn't compile; macOS has no drive layer | Blocks two of the three OSes |
| MakeMKV for all video, closed, needs a key. This one is what the owner already expected. | Accepted |

## 7. What to do, ranked

Done: the flatpak ships this app, built inside the SDK with the tools
bundled, and CI builds it (was 1); the start-up tool check (was 2, without
the encoder list).

| # | Do | Effort |
|---|---|---|
| 3 | **Native Linux tarball / AppImage** from the same CI job, built on ubuntu-24.04 (glibc 2.39 floor; CI uses 24.04 for libchdman's C++20) with the tools beside the binary. 22.04 with a newer g++ would reach further back, but that's unverified. Teach `tool::command` to look next to `current_exe()` before PATH, as the first app did. | 1–2 days |
| 4 | **macOS:** a drive layer (IOKit listing + `drutil` eject; TOC from cyanrip/libcdio), `dirs` for paths, notifications, then bring back the macOS ffmpeg and cyanrip build scripts from git history (`scripts/build-ffmpeg-macos.sh`, `build-cyanrip-macos.sh`, deleted at the swap). Signing and notarisation need an Apple Developer account. Untestable without a Mac, so make every native path "unknown, not wrong". | 1–2 weeks + a signing account |
| 5 | **Windows:** `cfg` the pty out (ConPTY or a plain pipe; cyanrip and redumper progress may then arrive in bursts), a drive layer over `DeviceIoControl`, drive-letter device names per tool, `dirs`. | 2–3 weeks |
| 6 | **MakeMKV:** keep it as the video backend and keep it unbundled. Optionally add an open **DVD** backend through the bundled ffmpeg's `dvdvideo` + user-installed libdvdcss. For Blu-ray, don't try to replace it. | DVD: 1–3 weeks (incl. getting libdvdcss into the sandbox); BD: not worth it |
