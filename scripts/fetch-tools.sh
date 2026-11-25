#!/usr/bin/env bash
# Puts the tools the flatpak bundles into one folder: ffmpeg and ffprobe,
# redumper, and cyanrip. Linux x86_64 only.
#
#   scripts/fetch-tools.sh <dest-dir>
#
# scripts/build-flatpak.sh runs this inside the GNOME SDK, so cyanrip (the one
# built here) links the runtime's glibc and libcurl, not the build machine's.
#
# - ffmpeg, ffprobe: BtbN's static GPL build, which has libx264 and libx265
#   (Transcode's default codec). Many distributions' ffmpeg lacks x265.
#   Pinned to a month-end build: BtbN deletes its daily builds after a
#   couple of weeks but keeps the last of each month.
# - redumper: upstream's static prebuilt, pinned by build number.
# - cyanrip: built from pinned sources by build-cyanrip.sh.
#
# Every download is checked against the sha256 recorded here before it is
# used. Changing a version means changing its hash with it.
#
# Writes <dest-dir>/PROVENANCE.txt, which says which source each binary came
# from. They are GPL, so a release has to say so (THIRD-PARTY.md).
set -euo pipefail

DEST="${1:?usage: $0 <dest-dir>}"
mkdir -p "$DEST"
DEST="$(cd "$DEST" && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"

# Pinned: the asset name carries the build number. Override both to try a newer one.
REDUMPER_BUILD="${REDUMPER_BUILD:-b744}"
REDUMPER_SHA256="${REDUMPER_SHA256:-61c408a106018712f55a3e778f0ce9c2af38de912c9d03199c7c7d827e1ba90b}"
BTBN_RELEASE="${BTBN_RELEASE:-autobuild-2026-10-31-12-59}"
BTBN_ASSET="${BTBN_ASSET:-ffmpeg-N-128227-g4811625b5f-linux64-gpl.tar.xz}"
BTBN_SHA256="${BTBN_SHA256:-67929be39bbb696f41d3295686597a439aeac902f61d82be660ccc1453c1fdfb}"
BTBN="https://github.com/BtbN/FFmpeg-Builds/releases/download/$BTBN_RELEASE/$BTBN_ASSET"
REDUMPER="https://github.com/superg/redumper/releases/download/$REDUMPER_BUILD/redumper-$REDUMPER_BUILD-linux-x64.zip"

# A host we don't run shouldn't lose a build by refusing one connection.
curl_get() {
  curl -fL -sS --retry 5 --retry-delay 5 --retry-all-errors --connect-timeout 30 "$@"
}

# Stops the build unless the file is the one pinned.
check_sha256() {
  echo "$2  $1" | sha256sum -c --quiet - || { echo "$1 isn't the file pinned here (sha256 $2)." >&2; exit 1; }
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> ffmpeg, ffprobe ($BTBN_RELEASE)"
curl_get "$BTBN" -o "$WORK/f.tar.xz"
check_sha256 "$WORK/f.tar.xz" "$BTBN_SHA256"
tar -xJf "$WORK/f.tar.xz" -C "$WORK" --strip-components=2 --wildcards '*/bin/ffmpeg' '*/bin/ffprobe'
install -m 755 "$WORK/ffmpeg" "$WORK/ffprobe" "$DEST/"
encoders="$("$DEST/ffmpeg" -hide_banner -encoders 2>/dev/null)"
for want in libx264 libx265 libmp3lame libopus libvorbis libsvtav1; do
  grep -q " $want " <<<"$encoders" || { echo "This ffmpeg is missing $want, which the recipes use." >&2; exit 1; }
done

echo "==> redumper $REDUMPER_BUILD"
curl_get "$REDUMPER" -o "$WORK/r.zip"
check_sha256 "$WORK/r.zip" "$REDUMPER_SHA256"
unzip -q "$WORK/r.zip" -d "$WORK"
install -m 755 "$WORK/redumper-$REDUMPER_BUILD-linux-x64/bin/redumper" "$DEST/"
redumper_version="$("$DEST/redumper" --version 2>&1)"
[[ "$redumper_version" == *"$REDUMPER_BUILD"* ]] || { echo "Expected $REDUMPER_BUILD in: $redumper_version" >&2; exit 1; }

echo "==> cyanrip"
"$HERE/build-cyanrip.sh" "$DEST"

# The ffmpeg version string carries the upstream commit
# (`N-128227-g4811625b5f-…` is commit 4811625b5f).
{
  echo "fetched:  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "ffmpeg:   $BTBN"
  echo "          sha256 $BTBN_SHA256"
  echo "          $("$DEST/ffmpeg" -version | head -1)"
  echo "redumper: $REDUMPER"
  echo "          sha256 $REDUMPER_SHA256"
  echo "          $redumper_version, not modified"
  echo "cyanrip:  built by scripts/build-cyanrip.sh from the versions pinned there"
} >"$DEST/PROVENANCE.txt"

echo "Done: $DEST"
