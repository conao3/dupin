#!/usr/bin/env bash
# Render each headless scenario in scripts/headless to target/screenshots/<name>.png. They show
# real results from this Mac (file paths included), so they stay out of the repository.
# The panel fits its content height, which headless snapshots don't follow, so each run
# first reads the panel's height from the element tree and snaps at that size.
# `--realtime` paces the run on the wall clock: results come from real providers on worker
# threads (Spotlight, the app scan, icon rasterisation), which a virtual clock would outrun.
set -euo pipefail
cd "$(dirname "$0")/.."
bin=${DUPIN_BIN:-target/debug/dupin}
out=${OUT_DIR:-target/screenshots}
mkdir -p "$out"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for script in scripts/headless/*.json; do
  name=$(basename "$script" .json)
  "$bin" --script "$script" --realtime --tree "$tmp/$name.json" --at mark:end 2>/dev/null
  height=$(jq '[.. | objects | select(.key? == "panel" and .rect != null) | .rect[3]] | first | ceil' "$tmp/$name.json")
  "$bin" --script "$script" --realtime --snap "$out/$name.png" --at mark:end --scale 2 --size "680x$height" 2>/dev/null
  echo "$out/$name.png (680x$height pt)"
done
