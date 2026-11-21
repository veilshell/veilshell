#!/usr/bin/env bash
# Tier 3 Automated Hyprland + grim Visual Verification Harness for VeilShell.
#
# Modes:
#  1. Active Hyprland session (HYPRLAND_INSTANCE_SIGNATURE set): creates a temporary
#     virtual headless output (`hyprctl output create headless veilshell-test`), tests
#     live IPC via `veilctl`, presents `veilshell` surfaces, and captures via `grim`.
#  2. Nested Hyprland session (inside a wl_compositor v6 host Wayland session): spawns
#     an isolated nested Hyprland instance and captures each live `wlr-layer-shell` scene.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-/tmp/veilshell-hypr-test}"
mkdir -p "$OUT_DIR"

echo "[1/5] Building veilshell and veilctl..."
cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --bin veilshell --bin veilctl

VEILSHELL_BIN="$REPO_ROOT/target/debug/veilshell"
VEILCTL_BIN="$REPO_ROOT/target/debug/veilctl"

# Mode 1: Running inside an existing Hyprland session -> use virtual headless output
if [[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]]; then
    HEADLESS_NAME="HEADLESS-VEILTEST"
    cleanup_output() {
        hyprctl output remove "$HEADLESS_NAME" >/dev/null 2>&1 || true
    }
    trap cleanup_output EXIT

    echo "[2/5] Creating virtual headless Hyprland monitor ($HEADLESS_NAME)..."
    hyprctl output create headless "$HEADLESS_NAME" >/dev/null
    hyprctl keyword monitor "$HEADLESS_NAME,1280x853@60,auto,1" >/dev/null
    sleep 0.3

    echo "[3/5] Verifying live Hyprland IPC via veilctl..."
    "$VEILCTL_BIN" status

    echo "[4/5] Presenting live wlr-layer-shell surfaces & capturing grim screenshots..."
    for scene in home shade recents pill; do
        "$VEILSHELL_BIN" --scene "$scene" --exit-after-ms 1600 >"$OUT_DIR/veilshell-$scene.log" 2>&1 &
        SHELL_PID=$!
        sleep 0.6
        grim -o "$HEADLESS_NAME" "$OUT_DIR/hyprland-$scene.png"
        wait "$SHELL_PID"
        bytes="$(stat -c%s "$OUT_DIR/hyprland-$scene.png")"
        echo "  -> Captured $OUT_DIR/hyprland-$scene.png ($bytes bytes)"
    done

    echo "[5/5] Tier 3 Hyprland virtual-output visual test PASSED!"
    exit 0
fi

# Mode 2: Nested Hyprland inside an existing Wayland desktop (requires host wl_compositor >= v6)
HOST_RT="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
HOST_WL="${WAYLAND_DISPLAY:-}"
if [[ -z "$HOST_WL" ]]; then
    HOST_WL="$(find "$HOST_RT" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' 2>/dev/null | head -n 1 || true)"
fi

if [[ -z "$HOST_WL" ]]; then
    echo "ERROR: No active Hyprland or host Wayland session found." >&2
    echo "Tip: Run ./scripts/test-snapshots.sh for headless offscreen PNG testing." >&2
    exit 1
fi

TMP_CFG="$(mktemp /tmp/vs-cfg.XXXX.conf)"
HYPR_PID=""
cleanup_nested() {
    if [[ -n "$HYPR_PID" ]] && kill -0 "$HYPR_PID" 2>/dev/null; then
        kill "$HYPR_PID" 2>/dev/null || true
        wait "$HYPR_PID" 2>/dev/null || true
    fi
    rm -f "$TMP_CFG"
}
trap cleanup_nested EXIT

cat >"$TMP_CFG" <<'EOF'
monitor=,1280x853@60,0x0,1
misc {
    disable_hyprland_logo = true
    disable_splash_rendering = true
}
layerrule = noanim, veilshell
EOF

BEFORE_SOCKS="$(find "$HOST_RT" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' 2>/dev/null | sort || true)"
BEFORE_SIGS="$(find "$HOST_RT/hypr" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort || true)"

echo "[2/5] Starting isolated nested Hyprland (host Wayland: $HOST_WL)..."
env XDG_RUNTIME_DIR="$HOST_RT" WAYLAND_DISPLAY="$HOST_WL" \
    Hyprland --config "$TMP_CFG" >"$OUT_DIR/hyprland.log" 2>&1 &
HYPR_PID=$!

CHILD_WL=""
CHILD_SIG=""
for _ in $(seq 1 50); do
    kill -0 "$HYPR_PID" 2>/dev/null || break
    AFTER_SOCKS="$(find "$HOST_RT" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' 2>/dev/null | sort || true)"
    AFTER_SIGS="$(find "$HOST_RT/hypr" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort || true)"
    CHILD_WL="$(comm -13 <(printf '%s\n' "$BEFORE_SOCKS") <(printf '%s\n' "$AFTER_SOCKS") | head -n 1 || true)"
    CHILD_SIG="$(comm -13 <(printf '%s\n' "$BEFORE_SIGS") <(printf '%s\n' "$AFTER_SIGS") | head -n 1 || true)"
    if [[ -n "$CHILD_WL" && -n "$CHILD_SIG" && -S "$HOST_RT/hypr/$CHILD_SIG/.socket.sock" ]]; then
        break
    fi
    sleep 0.2
done

if [[ -z "$CHILD_WL" || -z "$CHILD_SIG" ]]; then
    echo "ERROR: Nested Hyprland failed to start (host compositor must support wl_compositor v6)." >&2
    tail -n 25 "$OUT_DIR/hyprland.log" >&2 || true
    exit 1
fi

export XDG_RUNTIME_DIR="$HOST_RT"
export WAYLAND_DISPLAY="$CHILD_WL"
export HYPRLAND_INSTANCE_SIGNATURE="$CHILD_SIG"

echo "[3/5] Verifying live Hyprland IPC via veilctl..."
"$VEILCTL_BIN" status
"$VEILCTL_BIN" mode tablet
"$VEILCTL_BIN" mode laptop

echo "[4/5] Presenting live wlr-layer-shell surfaces & capturing grim screenshots..."
for scene in home shade recents pill; do
    "$VEILSHELL_BIN" --scene "$scene" --exit-after-ms 1600 >"$OUT_DIR/veilshell-$scene.log" 2>&1 &
    SHELL_PID=$!
    sleep 0.6
    grim "$OUT_DIR/hyprland-$scene.png"
    wait "$SHELL_PID"
    bytes="$(stat -c%s "$OUT_DIR/hyprland-$scene.png")"
    echo "  -> Captured $OUT_DIR/hyprland-$scene.png ($bytes bytes)"
done

echo "[5/5] Tier 3 nested Hyprland visual test PASSED!"
