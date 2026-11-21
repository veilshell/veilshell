#!/usr/bin/env bash
# Tier 2 Automated Offscreen Visual Snapshot Suite (all 5 scenes, landscape + portrait).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-/tmp/veilshell-snapshots}"
mkdir -p "$OUT_DIR"

cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --bin veilctl
VEILCTL="$REPO_ROOT/target/debug/veilctl"

for scene in home shade recents pill osk; do
    "$VEILCTL" snapshot --scene "$scene" --out "$OUT_DIR/$scene-landscape.png"
done

# Portrait Surface Go 3 logical resolution (853x1280)
for scene in home shade recents; do
    "$VEILCTL" snapshot --scene "$scene" --width 853 --height 1280 --out "$OUT_DIR/$scene-portrait.png"
done

echo "All Tier 2 snapshots generated in $OUT_DIR"
ls -lh "$OUT_DIR"
