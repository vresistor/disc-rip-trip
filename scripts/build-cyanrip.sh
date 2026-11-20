#!/usr/bin/env bash
# Builds cyanrip for Linux x86_64, to ship in the flatpak. fetch-tools.sh runs
# it inside the GNOME SDK, which has every build tool it needs.
#
# Distribution packages are not an option, and it is worth recording why rather
# than rediscovering it. Debian's cyanrip needs **187** shared libraries and
# Alpine's needs 115 — not because cyanrip is heavy, but because it links the
# distribution's *full* ffmpeg, which drags in X11, cairo, OpenCL, SVT-AV1 and
# every video codec ever packaged. cyanrip itself has only ten direct
# dependencies.
#
# So the lever is ffmpeg's configuration, not cyanrip's. Built with
# `--disable-autodetect`, ffmpeg links nothing it was not explicitly told to,
# which leaves its own native codecs — flac, alac, tta, wavpack, pcm, aac,
# vorbis, opus — plus libmp3lame. That is cyanrip's entire format table, and it
# collapses the dependency tree to almost nothing.
#
# The result is linked statically against everything except the C library,
# libcurl and libxml2, which the runtime provides.
set -euo pipefail

# A host we don't run shouldn't lose a build by refusing one connection
# (ffmpeg.org once timed out mid-run). `-sS`: no progress spinner in CI logs.
curl_get() {
  curl -fL -sS --retry 5 --retry-delay 5 --retry-all-errors --connect-timeout 30 "$@"
}

# Pinned rather than tracking a branch: a dependency that builds today and
# not tomorrow turns a release into an investigation. Each with the sha256
# of its tarball (cyanrip: the commit its tag points at), checked before
# it's unpacked; a new version needs its new hash.
CYANRIP_VERSION="${CYANRIP_VERSION:-0.9.3}"
CYANRIP_COMMIT="${CYANRIP_COMMIT:-6f7f2384a5faafdeaa0b7476def6fe20ae69c138}"
FFMPEG_VERSION="${FFMPEG_VERSION:-7.1}"
FFMPEG_SHA256="${FFMPEG_SHA256:-40973d44970dbc83ef302b0609f2e74982be2d85916dd2ee7472d30678a7abe6}"
LAME_VERSION="${LAME_VERSION:-3.100}"
LAME_SHA256="${LAME_SHA256:-ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e}"
LIBCDIO_VERSION="${LIBCDIO_VERSION:-2.1.0}"
LIBCDIO_SHA256="${LIBCDIO_SHA256:-8550e9589dbd594bfac93b81ecf129b1dc9d0d51e90f9696f1b2f9b2af32712b}"
LIBCDIO_PARANOIA_VERSION="${LIBCDIO_PARANOIA_VERSION:-10.2+2.0.2}"
LIBCDIO_PARANOIA_SHA256="${LIBCDIO_PARANOIA_SHA256:-186892539dedd661276014d71318c8c8f97ecb1250a86625256abd4defbf0d0c}"

DEST="${1:?usage: $0 <dest-dir>}"
mkdir -p "$DEST"
DEST="$(cd "$DEST" && pwd)"

WORK="$(mktemp -d)"
PREFIX="$WORK/prefix"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$PREFIX"

export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/lib64/pkgconfig"
export CFLAGS="-O2 -fPIC"
export CXXFLAGS="-O2 -fPIC"
JOBS="$(nproc)"

# Every step's output goes to a log rather than to /dev/null, and the tail is
# printed if the step fails.
#
# This is not tidiness. `>/dev/null` on a build step is how the macOS ffmpeg
# leg cost forty minutes a guess, and meson is worse than most: it reports its
# errors on **stdout**, so redirecting stdout hides precisely the line that
# says what went wrong. The first run of this script failed with no output at
# all for exactly that reason.
run() {
  local name="$1"
  shift
  if ! "$@" >"$WORK/$name.log" 2>&1; then
    echo "=== $name failed. Last 40 lines: ===" >&2
    tail -40 "$WORK/$name.log" >&2
    exit 1
  fi
}

# fetch <name> <url> <file> <sha256>: stops the build unless it's the file pinned.
fetch() {
  echo "==> $1"
  curl_get "$2" -o "$WORK/$3"
  echo "$4  $WORK/$3" | sha256sum -c --quiet - || { echo "$3 isn't the file pinned here (sha256 $4)." >&2; exit 1; }
}

echo "==> lame $LAME_VERSION"
fetch "lame" "https://downloads.sourceforge.net/project/lame/lame/${LAME_VERSION}/lame-${LAME_VERSION}.tar.gz" lame.tar.gz "$LAME_SHA256"
tar -xzf "$WORK/lame.tar.gz" -C "$WORK"
(
  cd "$WORK/lame-$LAME_VERSION"
  # frontend is the `lame` CLI, which we have no use for and which fails to
  # build against modern glibc anyway.
  run lame-configure ./configure --prefix="$PREFIX" --enable-static --disable-shared \
    --disable-frontend --disable-decoder
  run lame-make make -j"$JOBS"
  run lame-install make install
)

echo "==> libcdio $LIBCDIO_VERSION"
fetch "libcdio" "https://ftp.gnu.org/gnu/libcdio/libcdio-${LIBCDIO_VERSION}.tar.bz2" libcdio.tar.bz2 "$LIBCDIO_SHA256"
tar -xjf "$WORK/libcdio.tar.bz2" -C "$WORK"
(
  cd "$WORK/libcdio-$LIBCDIO_VERSION"
  run cdio-configure ./configure --prefix="$PREFIX" --enable-static --disable-shared \
    --disable-cxx --disable-example-progs --without-cd-drive --without-cd-info \
    --without-cdda-player --without-cd-read --without-iso-info --without-iso-read
  run cdio-make make -j"$JOBS"
  run cdio-install make install
)

echo "==> libcdio-paranoia $LIBCDIO_PARANOIA_VERSION"
fetch "libcdio-paranoia" \
  "https://ftp.gnu.org/gnu/libcdio/libcdio-paranoia-${LIBCDIO_PARANOIA_VERSION}.tar.bz2" \
  paranoia.tar.bz2 "$LIBCDIO_PARANOIA_SHA256"
tar -xjf "$WORK/paranoia.tar.bz2" -C "$WORK"
(
  cd "$WORK/libcdio-paranoia-$LIBCDIO_PARANOIA_VERSION"
  run paranoia-configure ./configure --prefix="$PREFIX" --enable-static --disable-shared \
    --disable-example-progs
  run paranoia-make make -j"$JOBS"
  run paranoia-install make install
)

echo "==> ffmpeg $FFMPEG_VERSION (audio only)"
fetch "ffmpeg" "https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz" ffmpeg.tar.xz "$FFMPEG_SHA256"
tar -xJf "$WORK/ffmpeg.tar.xz" -C "$WORK"
(
  cd "$WORK/ffmpeg-$FFMPEG_VERSION"
  # --disable-autodetect is the whole trick: without it, configure links
  # whatever development headers happen to be installed on the build machine,
  # which is exactly how a distribution ffmpeg ends up needing OpenCL.
  #
  # No --enable-gpl here, deliberately. Nothing GPL-only is used, so this
  # ffmpeg is LGPL — which keeps cyanrip's own LGPL-2.1 terms intact rather
  # than pulling the whole binary up to GPL.
  ./configure \
    --prefix="$PREFIX" \
    --disable-autodetect \
    --enable-libmp3lame \
    --enable-static \
    --disable-shared \
    --disable-programs \
    --disable-doc \
    --disable-avdevice \
    --disable-swscale \
    --disable-postproc \
    --disable-network \
    --extra-cflags="-I$PREFIX/include" \
    --extra-ldflags="-L$PREFIX/lib" \
    || { echo "=== ffbuild/config.log (tail) ==="; tail -40 ffbuild/config.log; exit 1; }
  run ffmpeg-make make -j"$JOBS"
  run ffmpeg-install make install
)

echo "==> cyanrip $CYANRIP_VERSION"
run cyanrip-clone git clone --depth 1 --branch "v$CYANRIP_VERSION" \
  https://github.com/cyanreg/cyanrip.git "$WORK/cyanrip"
(
  cd "$WORK/cyanrip"
  # A tag can be moved; the commit can't.
  got="$(git rev-parse HEAD)"
  [[ "$got" == "$CYANRIP_COMMIT" ]] || { echo "cyanrip v$CYANRIP_VERSION is $got, not the pinned $CYANRIP_COMMIT." >&2; exit 1; }

  # Remove cyanrip's own MusicBrainz client.
  #
  # This application already looks releases up itself, with its own cache and
  # its own rate limiter. Letting cyanrip look the *same* release up again is
  # not merely redundant: MusicBrainz enforces one request per second **by IP**
  # and blocks rather than throttles, so a second client in the same process
  # tree — one that our limiter knows nothing about — is a way to get the user's
  # address blocked while working through a box set. Tags are passed in on the
  # command line instead, from the lookup that already happened.
  #
  # The coupling is one function called from one place, so this is a stub rather
  # than surgery. Cover art still works: it is fetched by URL, and the app
  # supplies that URL from the release it already chose.
  #
  # It also deletes the entire libmusicbrainz dependency, which is worth having
  # for its own sake — upstream has been unmaintained since 2016, Fedora no
  # longer packages it, and linking it statically drags in neon, GnuTLS, nettle,
  # p11-kit, idn2 and a dozen more, none of which this ever calls.
  cat > src/musicbrainz.c <<'STUB'
/* Replaced by scripts/build-cyanrip.sh. Disc Rip Trip performs its own
 * MusicBrainz lookups, with a cache and a rate limiter, and passes the result
 * in as tags. See THIRD-PARTY.md. */
#include "musicbrainz.h"

int crip_fill_metadata(cyanrip_ctx *ctx, int manual_metadata_specified,
                       int release_idx, char *release_str, int discnumber)
{
    (void)ctx; (void)manual_metadata_specified;
    (void)release_idx; (void)release_str; (void)discnumber;
    return 0;
}
STUB

  sed -i "/dependency('libmusicbrainz5'/d" src/meson.build
  grep -q 'libmusicbrainz5' src/meson.build && {
    echo "The libmusicbrainz removal did not apply; upstream has changed." >&2
    exit 1
  }
  # Static everywhere it can be. libc, libxml2 and libcurl stay dynamic: they
  # are present on every system, and statically linking a TLS stack means
  # shipping our own certificate handling, which is worse than depending on
  # the one the distribution maintains.
  run cyanrip-meson meson setup build \
    --prefix="$PREFIX" \
    --buildtype=release \
    -Ddefault_library=static \
    -Dc_link_args="-static-libgcc"
  run cyanrip-ninja ninja -C build
  install -m 755 build/src/cyanrip "$DEST/cyanrip"
)

echo "==> Stripping"
# Static-linking five libraries' worth of debug symbols produces a 100MB
# binary. Stripped it is a fraction of that, and this ships inside an installer.
strip "$DEST/cyanrip"

echo "==> Checking the result"
"$DEST/cyanrip" -o help >/dev/null || { echo "The built cyanrip does not run." >&2; exit 1; }

# The check that this was all worth doing. A build that quietly linked the
# system's ffmpeg instead of the minimal one is indistinguishable from success
# until it reaches a machine that hasn't got it.
#
# Deliberately a check for *codec libraries*, not a count. An earlier version
# failed the build above thirty shared libraries, which was wrong twice over: it
# tested a proxy rather than the thing that breaks, and the number depends on
# the build host's libcurl — Ubuntu's pulls in GnuTLS, SASL and Brotli where
# Fedora's does not — so the same good binary passed or failed by luck.
linked="$(ldd "$DEST/cyanrip" || true)"
if grep -qiE 'libav|libsw|x264|x265|vpx|aom|dav1d|svt|theora|vorbis|mp3lame|cdio' <<<"$linked"; then
  echo "This build links codec libraries dynamically, which means it found a" >&2
  echo "system ffmpeg rather than the minimal one built here. It would fail on" >&2
  echo "any machine without exactly those libraries." >&2
  echo "$linked" >&2
  exit 1
fi

# What remains is libcurl and its TLS and resolver chain, which cyanrip uses for
# AccurateRip and cover art. That is a deliberate boundary: statically linking a
# TLS stack means shipping our own certificate handling, which is worse than
# using the one the system already maintains and updates.
echo "    links $(wc -l <<<"$linked") shared libraries, none of them codecs"

# The formats the app actually asks for. A cyanrip that runs but cannot encode
# MP3 would pass every existence check and fail on the first rip.
formats="$("$DEST/cyanrip" -o help 2>&1)"
for want in flac mp3; do
  grep -qE "^\s+$want\s" <<<"$formats" || {
    echo "This build cannot produce $want." >&2
    echo "$formats" >&2
    exit 1
  }
  echo "    $want ok"
done

echo "Done: $DEST/cyanrip"
