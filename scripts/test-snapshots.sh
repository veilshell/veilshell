#!/usr/bin/env bash
# Tier 2 Automated Offscreen Visual Snapshot Suite (landscape + portrait).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-/tmp/veilshell-snapshots}"
mkdir -p "$OUT_DIR"

cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --bin veilctl
VEILCTL="$REPO_ROOT/target/debug/veilctl"

for scene in home shade recents pill osk; do
    "$VEILCTL" snapshot --demo --scene "$scene" --out "$OUT_DIR/$scene-landscape.png"
done

# Portrait Surface Go 3 logical resolution (853x1280)
for scene in home shade recents; do
    "$VEILCTL" snapshot --demo --scene "$scene" --width 853 --height 1280 --out "$OUT_DIR/$scene-portrait.png"
done

# Separate the standalone Tablet launcher from the overlay fixtures.
for viewport in 1280x800 800x1280; do
    "$VEILCTL" snapshot --demo --scene home --tablet-home \
        --width "${viewport%x*}" --height "${viewport#*x}" \
        --out "$OUT_DIR/tablet-home-$viewport.png"
done
"$VEILCTL" snapshot --demo --scene home --tablet-home --width 853 --height 1280 \
    --scale 1.5 --out "$OUT_DIR/tablet-home-portrait-1.5x.png"

# Standalone Tablet Shade, without borrowing the host's notification history.
tablet_shade() {
    local name="$1" viewport="$2"
    shift 2
    "$VEILCTL" snapshot --demo --scene shade \
        --width "${viewport%x*}" --height "${viewport#*x}" \
        --out "$OUT_DIR/tablet-shade-$name.png" "$@"
}
tablet_shade notifications 800x1280 --tablet-shade notifications --empty-notifications
tablet_shade unavailable 800x1280 --tablet-shade notifications
tablet_shade controls 800x1280 --tablet-shade controls
tablet_shade sliding 800x1280 --tablet-shade controls --empty-notifications --shade-position 0.5
tablet_shade split 1280x800 --tablet-shade controls --empty-notifications
for progress in 0.25 0.50 0.75; do
    tablet_shade "pull-$progress" 1280x800 --tablet-shade controls --empty-notifications --progress "$progress"
done
tablet_shade split-1.5x 1280x853 --tablet-shade controls --empty-notifications --scale 1.5
tablet_shade controls-1.5x 853x1280 --tablet-shade controls --scale 1.5

# Settled and finger-held standalone Tablet scenes, separate from overlay Recents.
tablet_nav() {
    local scene="$1" viewport="$2" name="$3"
    shift 3
    "$VEILCTL" snapshot --demo --scene "$scene" \
        --width "${viewport%x*}" --height "${viewport#*x}" \
        --out "$OUT_DIR/tablet-$scene-$name-$viewport.png" "$@"
}
for viewport in 800x1280 1280x800; do
    tablet_nav drawer "$viewport" settled
    tablet_nav drawer "$viewport" pull --progress 0.45
    for origin in app home; do
        tablet_nav recents "$viewport" "$origin" --tablet-recents "$origin"
        tablet_nav recents "$viewport" "$origin-pull" --tablet-recents "$origin" --progress 0.45
    done
done
tablet_nav drawer 1280x800 pull-1.5x --progress 0.45 --scale 1.5
tablet_nav recents 800x1280 home-pull-1.5x --tablet-recents home --progress 0.45 --scale 1.5

"$VEILCTL" snapshot --demo --scene osk --terminal-osk --out "$OUT_DIR/osk-terminal-landscape.png"
# The keyboard panel occupies a third of the full output height.
"$VEILCTL" snapshot --demo --scene osk --terminal-osk --width 853 --height 426 --out "$OUT_DIR/osk-terminal-portrait.png"

# A compact Home viewport exercises pagination with the 12 demo apps.
"$VEILCTL" snapshot --demo --scene home --width 390 --height 518 --out "$OUT_DIR/home-compact.png"
"$VEILCTL" snapshot --demo --scene shade --power-menu --width 390 --height 518 --out "$OUT_DIR/power-compact.png"
"$VEILCTL" snapshot --demo --scene osk --terminal-osk --width 390 --height 172 --out "$OUT_DIR/osk-terminal-compact.png"
"$VEILCTL" snapshot --demo --scene home --progress 0.55 --out "$OUT_DIR/home-pull.png"
"$VEILCTL" snapshot --demo --scene home --home-position 0.5 --width 390 --height 518 --out "$OUT_DIR/home-page-drag.png"
"$VEILCTL" snapshot --demo --scene shade --progress 0.55 --out "$OUT_DIR/shade-pull.png"

for scale in 1.5 2; do
    "$VEILCTL" snapshot --scene home --width 960 --height 640 --scale "$scale" \
        --out "$OUT_DIR/home-installed-${scale}x.png"
    "$VEILCTL" snapshot --demo --scene shade --width 390 --height 640 --scale "$scale" \
        --out "$OUT_DIR/shade-portrait-${scale}x.png"
done

echo "All Tier 2 snapshots generated in $OUT_DIR"
ls -lh "$OUT_DIR"
