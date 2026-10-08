#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ] || [ ! -e "$1" ]; then
  echo 'usage: check-appimage-media.sh <AppImage-or-AppDir>' >&2
  exit 2
fi
input="$(realpath "$1")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
if [ -d "$input" ]; then
  appdir="$input"
else
  (cd "$work" && "$input" --appimage-extract >/dev/null)
  appdir="$work/squashfs-root"
fi
plugins="$appdir/usr/lib/gstreamer-1.0"
scanner="$appdir/usr/lib/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner"
[ -x "$scanner" ] || { echo 'Bundled GStreamer plugin scanner is missing' >&2; exit 1; }
for entry in vp9parse:videoparsersbad vp9alphadecodebin:codecalpha vp9dec:vpx matroskademux:matroska; do
  element="${entry%%:*}"
  library="$plugins/libgst${entry#*:}.so"
  [ -f "$library" ] || { echo "Missing bundled media library: $library" >&2; exit 1; }
  details="$(env LD_LIBRARY_PATH="$appdir/usr/lib" \
    GST_PLUGIN_SYSTEM_PATH_1_0="$plugins" GST_PLUGIN_PATH_1_0="$plugins" \
    GST_PLUGIN_SCANNER_1_0="$scanner" GST_REGISTRY_1_0="$work/registry.bin" \
    LC_ALL=C gst-inspect-1.0 "$element")"
  filename="$(sed -n 's/^[[:space:]]*Filename[[:space:]]*//p' <<< "$details")"
  [ "$filename" = "$library" ] || { echo "$element did not load from its bundled library: $filename" >&2; exit 1; }
  echo "Verified bundled element: $element"
done
