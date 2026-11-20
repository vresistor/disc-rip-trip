#!/usr/bin/env bash
# Builds the flatpak bundle: dist/disc-rip-trip-<version>-x86_64.flatpak.
#
#   scripts/build-flatpak.sh [--install]
#
#   --install   also install it into your user flatpak installation
#
# Needs flatpak and flatpak-builder (or Flathub's org.flatpak.Builder). The
# first run installs the GNOME SDK and its Rust extension from Flathub.
#
# Everything is compiled inside the GNOME SDK, not on this machine, so the
# binaries need no newer glibc than the runtime has:
#   1. scripts/fetch-tools.sh: ffmpeg, ffprobe, redumper, cyanrip, into
#      target/flatpak/tools (once; delete that folder to fetch them again).
#   2. cargo build --release of disc-rip-trip and drt, into target/flatpak.
#   3. flatpak-builder packages the lot (packaging/flatpak/io.github.vresistor.DiscRipTrip.yml).
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

INSTALL=no
[ "${1:-}" = "--install" ] && INSTALL=yes

APP_ID=io.github.vresistor.DiscRipTrip
RUNTIME_VERSION=50
# The freedesktop base that GNOME $RUNTIME_VERSION is built on, for the Rust extension.
BASE_VERSION=25.08
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' app/Cargo.toml | head -1)
OUT="$ROOT/target/flatpak"
PAYLOAD="packaging/flatpak/payload"
BUNDLE="dist/disc-rip-trip-$VERSION-x86_64.flatpak"

command -v flatpak >/dev/null || { echo "error: flatpak is not installed" >&2; exit 1; }

flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
for ref in "org.gnome.Platform//$RUNTIME_VERSION" "org.gnome.Sdk//$RUNTIME_VERSION" "org.freedesktop.Sdk.Extension.rust-stable//$BASE_VERSION"; do
  flatpak info "$ref" >/dev/null 2>&1 || flatpak install --user --noninteractive flathub "$ref"
done

# Runs a command inside the SDK, with the network and this checkout.
in_sdk() {
  flatpak run --share=network --filesystem="$ROOT" \
    --env=PATH=/usr/lib/sdk/rust-stable/bin:/usr/bin \
    --env=CARGO_HOME="$OUT/cargo" --env=CARGO_TARGET_DIR="$OUT" \
    --env=LIBCHDMAN_CPU=portable \
    --command=bash "org.gnome.Sdk//$RUNTIME_VERSION" -c "cd '$ROOT' && $*"
}

if [ ! -f "$OUT/tools/PROVENANCE.txt" ]; then
  echo "==> Fetching the tools (in the SDK)"
  in_sdk scripts/fetch-tools.sh "$OUT/tools"
fi

echo "==> Building disc-rip-trip and drt (in the SDK)"
# LIBCHDMAN_CPU=portable: without it the CHD code is built for AVX2 and
# crashes on CPUs older than Haswell.
in_sdk cargo build --release --locked -p drt-app -p drt

echo "==> Packaging"
rm -rf "$PAYLOAD"
mkdir -p "$PAYLOAD"
cp "$OUT/release/disc-rip-trip" "$OUT/release/drt" "$OUT"/tools/* "$PAYLOAD/"
cp packaging/flatpak/$APP_ID.desktop packaging/flatpak/icons/*.png "$PAYLOAD/"
sed "s/@VERSION@/$VERSION/; s/@DATE@/$(date +%F)/" packaging/flatpak/$APP_ID.metainfo.xml >"$PAYLOAD/$APP_ID.metainfo.xml"

# Flathub's own builder when it's installed (CI installs it): a distro
# flatpak-builder can be too old for the current SDK.
if flatpak info org.flatpak.Builder >/dev/null 2>&1; then
  builder=(flatpak run org.flatpak.Builder --user --install-deps-from=flathub)
else
  builder=(flatpak-builder)
fi
"${builder[@]}" --force-clean --state-dir="$OUT/builder-state" --repo="$OUT/repo" "$OUT/build" packaging/flatpak/$APP_ID.yml

mkdir -p dist
rm -f "$BUNDLE"
flatpak build-bundle "$OUT/repo" "$BUNDLE" "$APP_ID"
echo "Built $BUNDLE"

if [ "$INSTALL" = yes ]; then
  flatpak install --user --reinstall --noninteractive "$BUNDLE"
  echo "Installed. Run it from the app grid, or: flatpak run $APP_ID"
else
  echo "Install it with: flatpak install --user ./$BUNDLE"
fi
