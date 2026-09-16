#!/bin/sh
# Fetch the IBM Plex TTFs the UI bundles (OFL 1.1) into crates/ui_kit/fonts.
# Run once when bumping the font release; the files are committed.
set -eu
SANS_TAG='%40ibm%2Fplex-sans%401.1.0'
MONO_TAG='%40ibm%2Fplex-mono%402.5.0'
dest="$(git rev-parse --show-toplevel)/crates/ui_kit/fonts"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -sSL -o "$tmp/sans.zip" "https://github.com/IBM/plex/releases/download/$SANS_TAG/ibm-plex-sans.zip"
curl -sSL -o "$tmp/mono.zip" "https://github.com/IBM/plex/releases/download/$MONO_TAG/ibm-plex-mono.zip"
mkdir -p "$dest"
unzip -q -o -j "$tmp/sans.zip" \
  'ibm-plex-sans/fonts/complete/ttf/IBMPlexSans-Regular.ttf' \
  'ibm-plex-sans/fonts/complete/ttf/IBMPlexSans-Medium.ttf' \
  'ibm-plex-sans/fonts/complete/ttf/IBMPlexSans-SemiBold.ttf' \
  'ibm-plex-sans/LICENSE.txt' -d "$dest"
unzip -q -o -j "$tmp/mono.zip" \
  'ibm-plex-mono/fonts/complete/ttf/IBMPlexMono-Regular.ttf' \
  'ibm-plex-mono/fonts/complete/ttf/IBMPlexMono-Medium.ttf' -d "$dest"
ls -l "$dest"
