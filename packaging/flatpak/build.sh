#!/usr/bin/env bash
# Build target/flatpak/effective-darkroom.flatpak, a single-file bundle that
# installs on any Flatpak distro with:
#   flatpak install --user effective-darkroom.flatpak
# Build state lives under target/ so the manifest's dir source skips it.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out=$root/target/flatpak
app_id=io.github.bkiu.EffectiveDarkroom

builder=flatpak-builder
command -v "$builder" >/dev/null || builder="nix run nixpkgs#flatpak-builder --"

mkdir -p "$out"
$builder --user --install-deps-from=flathub --force-clean \
    --state-dir="$out/state" --repo="$out/repo" \
    "$out/build" "$here/$app_id.yml"

flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$out/repo" "$out/effective-darkroom.flatpak" "$app_id"
echo "bundle: $out/effective-darkroom.flatpak"
