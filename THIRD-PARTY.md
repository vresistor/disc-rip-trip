# Third-party components, and where their source is

Disc Rip Trip is [GPL-3.0-or-later](LICENSE). This file records what ships inside
a release, under what terms, and where to get the corresponding source — which
is an obligation, not a courtesy, for the GPL components below.

## What is in a release

| Component | Licence | Shipped? |
|---|---|---|
| Disc Rip Trip itself | GPL-3.0-or-later | yes — this repository |
| ffmpeg, ffprobe | GPL-3.0 as built | yes — in the flatpak's `/app/bin` |
| cyanrip | LGPL-2.1-or-later | yes — in the flatpak's `/app/bin` |
| redumper | GPL-3.0 | yes — in the flatpak's `/app/bin` |
| MAME's CHD core, via libchdman-rs | BSD-3-Clause | yes — linked |
| lofty (audio tag reading and writing) | MIT / Apache-2.0 | yes — linked |
| Schibsted Grotesk, JetBrains Mono (fonts) | SIL OFL 1.1 | yes — bundled in the window (`app/assets`, licences beside them) |
| **MakeMKV** | proprietary (GuinpinSoft EULA) | **no** — see below |

## ffmpeg: corresponding source

The bundled ffmpeg is built `--enable-gpl --enable-version3` with `libx264` and
`libx265`, which is what makes it GPL-3.0 and, by extension, what settles this
project's own licence. Distributing those binaries obliges us to make the source
they were built from available.

They are [BtbN's FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds),
whose build scripts and pinned upstream revisions are published in that
repository. Each binary reports the exact upstream commit it came from:

```
$ ffmpeg -version
ffmpeg version N-125881-g946272b79a-20260801
                          ^^^^^^^^^^ upstream git commit
```

`scripts/fetch-tools.sh` records that string in `PROVENANCE.txt`, which
the flatpak installs in `/app/share/doc/io.github.vresistor.disc-rip-trip/` and CI
attaches to the release as `tools-provenance.txt`, so every published build
says exactly which source corresponds to it.

**Written offer.** If those upstream sources are ever unavailable, open an issue
and we will provide the complete corresponding source for any binary we have
published, for at least three years from the date of that release.

### What the bundled ffmpeg does not contain

It is built with `libdvdread` and `libdvdnav`, which read DVD structure —
titles, chapters, playlists. Neither decrypts anything.

Grepping the binary does turn up strings like `DVDCSS_METHOD` and `DVDCSS_CACHE`.
Those are libdvdread's *optional runtime* interface: if a system `libdvdcss` is
already installed, libdvdread will use it. We do not ship one, nothing is linked
against one (`ldd` confirms), and the build carries no `--enable-libdvdcss`. The
strings are an interface to something absent.

## cyanrip: corresponding source

[cyanrip](https://github.com/cyanreg/cyanrip) is LGPL-2.1-or-later, shipped
unmodified as a standalone executable and driven over its documented command
line. Shipping it obliges us to identify the source each binary came from.

It is built by `scripts/build-cyanrip.sh` from versions pinned in that
script, inside the GNOME SDK. Distribution packages are not usable here —
Debian's cyanrip needs 187 shared libraries and Alpine's 115, because both link
the distribution's *full* ffmpeg. So the script builds a minimal audio-only
ffmpeg alongside it, along with lame, libcdio and libcdio-paranoia. The script
*is* the recipe.

It changes one thing in cyanrip's source: `src/musicbrainz.c` is replaced
by a stub, and the libmusicbrainz dependency is dropped. Disc Rip Trip does its
own MusicBrainz lookups, with a cache and a rate limiter, and passes the result
in as tags; a second client in the same process tree could get your address
blocked. The script fails loudly if that change stops applying.

That ffmpeg is deliberately built **without** `--enable-gpl`. cyanrip needs none
of ffmpeg's GPL-only components, so the ffmpeg inside cyanrip is LGPL and
cyanrip's own LGPL-2.1 terms are preserved rather than being pulled up to GPL by
something it never uses. It is a different build, under different terms, from
the GPL ffmpeg binaries above.

**Anyone receiving a build** may obtain the complete corresponding source for
cyanrip and everything statically linked into it, and may relink it against a
modified version, as LGPL-2.1 section 6 requires. The build scripts produce
exactly the object it ships; open an issue and we will provide the intermediate
objects for any binary we have published.

## redumper: corresponding source

[redumper](https://github.com/superg/redumper) is GPL-3.0 — the same licence as
this project — shipped as a standalone executable and driven over its documented
command line. Upstream publishes a static Linux build, so
`scripts/fetch-tools.sh` takes that, unmodified, pinned to a numbered release
rather than a moving tag, and records which one in `PROVENANCE.txt`.

## MAME's CHD core: linked rather than shipped alongside

CHD conversion goes through [`libchdman-rs`](https://crates.io/crates/libchdman-rs),
a wrapper around the compressed-hunks implementation from
[MAME](https://www.mamedev.org). It is linked into the app rather than spawned,
so it has no provenance file: the version is pinned in
`crates/steps/Cargo.toml` and recorded exactly in `Cargo.lock`, which is how
every other linked dependency is pinned. It is compiled from the source the
crate vendors (its `prebuilt` feature, which downloads a static archive, is off).

**The CHD core is BSD-3-Clause**, and MAME as a whole is GPL-2.0-or-later. Both
are compatible with this project's GPL-3.0-or-later: BSD-3-Clause is permissive,
and GPL-2.0-**or-later** permits the combination under version 3. A
GPL-2.0-only component would not have been usable here, which is worth stating
because it is the check that had to be made rather than assumed.

## lofty: linked

[`lofty`](https://crates.io/crates/lofty) reads and writes audio tags in
`crates/steps`: the tags ffmpeg can't write (an ID3 `UFID` frame, MP4
`----:com.apple.iTunes:` atoms) and `drt fix-tags`. It is MIT / Apache-2.0,
pinned to an exact version in `crates/steps/Cargo.toml` and recorded in
`Cargo.lock`. Its default feature (zlib, for compressed ID3 frames) is off.

## AccurateRip

cyanrip queries [AccurateRip](http://www.accuraterip.com) directly, under its
own identity, to check whether a rip matches other people's of the same
pressing. The database is copyrighted by Illustrate, who ask that applications
seek agreement before accessing it.

We are not the party making those requests — cyanrip is, as a separate program
running on your machine, exactly as it does when installed from any of the eight
distributions that package it. We do, however, ship it. The check can be turned
off in the **Read disc** step's options, which leaves ripping and cdparanoia's
own error reporting untouched.

## MakeMKV is not shipped

MakeMKV is proprietary, obtained by the user from
[makemkv.com](https://www.makemkv.com), governed by its own EULA, and driven here
as an ordinary external program over its documented `makemkvcon` interface.
**It is never bundled**: nothing in this repository, the flatpak or a release
contains, embeds or redistributes it, and CI fails any build that does. The app
runs it only from a place the user has approved in the app (or named in
`DRT_MAKEMKVCON`); until then it only checks which files exist.

Why: MakeMKV isn't free software, so it can't go where the rest of the bundle
goes (Flathub builds from source, and the bundle's other tools are all GPL or
LGPL with their source published). And its EULA's permission to redistribute
it unmodified doesn't settle the harder point: MakeMKV decrypts CSS, and distributing circumvention software
is a materially different act from distributing something that drives it —
CDPA s.296ZB in the UK, DMCA §1201 in the US. It is the same line HandBrake draws
with libdvdcss.

## Metadata

Series and film metadata comes from [TMDb](https://www.themoviedb.org) using an
API key you supply. This product uses the TMDb API but is not endorsed or
certified by TMDb.

Album and track metadata comes from [MusicBrainz](https://musicbrainz.org),
which needs no key. Its data is available under CC0 and CC BY-NC-SA depending on
the field; nothing from it is redistributed here — it is written into the tags of
files on your own machine. Requests identify this application and are held to
MusicBrainz's one-per-second limit.

## Your own use

This project assumes you are working with discs you own and that you know what
your jurisdiction permits you to do with them. Rules vary widely and are often
narrower than people expect: the UK has had no private copying exception since
the 2014 one was quashed in 2015, and US law provides no backup exemption for
video. Nothing here is legal advice.
