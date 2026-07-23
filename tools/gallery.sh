#!/usr/bin/env bash
# Regenerate the demo gallery in out/ from examples/*.slab.
#
# Usage:  tools/gallery.sh            # all 12 examples, svg + png (+ 07 apng)
#         tools/gallery.sh 02-ops     # just one example
#
# Sizes/scales are the research gallery table (research/slab
# examples/build_gallery.py GALLERY) — the normative viewport per example.
# "-" for HEIGHT means unbounded (document decides). 10-settings renders at
# the research SettingsApp width (760); 11-unicode is a 1.0-only text-kernel
# fixture with no research row (defaults). 07-monitor additionally renders
# an APNG at the research monitor film's dur/fps (3 s @ 12 fps, scale 1.5).
set -euo pipefail
cd "$(dirname "$0")/.."

RENDER=(cargo run -q --release -p slab-cli -- render)
mkdir -p out

#        name         WIDTH  HEIGHT  PNG_SCALE
TABLE="\
00-player    360   -     2.0
01-settings  800   1120  1.5
02-ops       1920  1080  1.0
03-landing   1440  900   1.25
04-poster    1000  1500  1.5
05-railyard  800   512   2.0
06-jcard     1050  400   1.5
07-monitor   760   560   2.0
08-glass     900   560   2.0
09-widget    800   480   2.0
10-settings  760   -     2.0
11-unicode   800   -     2.0"

only="${1:-}"

while read -r name w h scale; do
  [[ -n "$only" && "$name" != "$only" ]] && continue
  size=(--width "$w")
  [[ "$h" != "-" ]] && size+=(--height "$h")
  "${RENDER[@]}" "examples/$name.slab" -o "out/$name.svg" "${size[@]}"
  "${RENDER[@]}" "examples/$name.slab" -o "out/$name.png" "${size[@]}" --scale "$scale"
  if [[ "$name" == "07-monitor" ]]; then
    "${RENDER[@]}" examples/07-monitor.slab -o out/07-monitor.apng \
      "${size[@]}" --scale 1.5 --dur 3 --fps 12
  fi
done <<<"$TABLE"
