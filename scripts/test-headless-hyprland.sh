#!/usr/bin/env bash
# Tier 3 Automated Hyprland + grim Visual Verification Harness for VeilShell.
#
# Inside an active Hyprland session:
#  1. Creates a temporary isolated headless monitor (`hyprctl output create headless`)
#  2. Launches ONE persistent `veilshell` daemon pinned strictly to that headless monitor
#  3. Drives live transitions (`home`, `shade`, `recents`, `pill`) via `veilctl` Unix socket IPC
#  4. Captures real compositor screenshots of every surface with `grim -o`
#  5. Tests `veilctl mode laptop` / `tablet` (Lua eval + legacy fallback) and exits cleanly
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-/tmp/veilshell-hypr-test}"
mkdir -p "$OUT_DIR"

echo "[1/5] Building veilshell and veilctl..."
cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --bin veilshell --bin veilctl

VEILSHELL_BIN="$REPO_ROOT/target/debug/veilshell"
VEILCTL_BIN="$REPO_ROOT/target/debug/veilctl"

if [[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]]; then
    BEFORE_JSON="$(hyprctl -j monitors)"
    hyprctl output create headless >/dev/null
    sleep 0.3
    AFTER_JSON="$(hyprctl -j monitors)"
    HEADLESS_NAME="$(python3 -c '
import json, sys
before = {m["name"] for m in json.loads(sys.argv[1])}
after = {m["name"] for m in json.loads(sys.argv[2])}
added = sorted(n for n in (after - before) if n.startswith("HEADLESS-"))
print(added[-1] if added else "")
' "$BEFORE_JSON" "$AFTER_JSON")"

    if [[ -z "$HEADLESS_NAME" || "$HEADLESS_NAME" != HEADLESS-* ]]; then
        echo "ERROR: Failed to detect newly created Hyprland HEADLESS-* output." >&2
        exit 1
    fi

    TMP_SOCK="$(mktemp -u /tmp/veilshell-test-XXXXXX.sock)"
    DAEMON_PID=""

    cleanup() {
        if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
            VEILSHELL_SOCKET="$TMP_SOCK" "$VEILCTL_BIN" quit >/dev/null 2>&1 || true
            sleep 0.2
            kill "$DAEMON_PID" 2>/dev/null || true
            wait "$DAEMON_PID" 2>/dev/null || true
        fi
        rm -f "$TMP_SOCK"
        if [[ "$HEADLESS_NAME" == HEADLESS-* ]]; then
            hyprctl output remove "$HEADLESS_NAME" >/dev/null 2>&1 || true
        fi
    }
    trap cleanup EXIT

    echo "[2/5] Configured virtual headless monitor: $HEADLESS_NAME (1280x853@60)"
    if [[ "$(hyprctl eval "hl.monitor({ output = \"$HEADLESS_NAME\", mode = \"1280x853@60\", position = \"3000x0\", scale = 1 })" 2>/dev/null || true)" != "ok" ]]; then
        hyprctl keyword monitor "$HEADLESS_NAME,1280x853@60,3000x0,1" >/dev/null
    fi
    hyprctl eval 'hl.layer_rule({ match = { namespace = "veilshell" }, no_anim = true, animation = "none" })' >/dev/null 2>&1 \
        || hyprctl keyword layerrule "noanim, veilshell" >/dev/null 2>&1 || true
    sleep 0.4

    echo "[3/5] Starting persistent daemon pinned to $HEADLESS_NAME..."
    "$VEILSHELL_BIN" --output "$HEADLESS_NAME" --socket "$TMP_SOCK" --scene pill >"$OUT_DIR/veilshell.log" 2>&1 &
    DAEMON_PID=$!

    for _ in $(seq 1 40); do
        if [[ -S "$TMP_SOCK" ]]; then
            break
        fi
        kill -0 "$DAEMON_PID" 2>/dev/null || {
            echo "ERROR: veilshell daemon exited prematurely." >&2
            cat "$OUT_DIR/veilshell.log" >&2 || true
            exit 1
        }
        sleep 0.1
    done

    if [[ ! -S "$TMP_SOCK" ]]; then
        echo "ERROR: Timed out waiting for daemon socket $TMP_SOCK" >&2
        exit 1
    fi

    export VEILSHELL_SOCKET="$TMP_SOCK"
    "$VEILCTL_BIN" status

    echo "[4/5] Driving live surface transitions via veilctl & capturing grim screenshots..."
    for scene in home shade recents pill; do
        "$VEILCTL_BIN" show "$scene"
        bytes=0
        if [[ "$scene" == "pill" ]]; then
            # Two captures spaced apart ensure lazy headless screencopy flushes the dismissed overlay
            sleep 0.25
            grim -o "$HEADLESS_NAME" "$OUT_DIR/hyprland-$scene.png"
            sleep 0.25
            grim -o "$HEADLESS_NAME" "$OUT_DIR/hyprland-$scene.png"
            bytes="$(stat -c%s "$OUT_DIR/hyprland-$scene.png")"
            min_bytes=1500
        else
            min_bytes=25000
            for _ in $(seq 1 5); do
                sleep 0.25
                grim -o "$HEADLESS_NAME" "$OUT_DIR/hyprland-$scene.png"
                bytes="$(stat -c%s "$OUT_DIR/hyprland-$scene.png")"
                if (( bytes >= min_bytes )); then
                    break
                fi
            done
        fi
        echo "  -> Captured $OUT_DIR/hyprland-$scene.png ($bytes bytes)"
        if (( bytes < min_bytes )); then
            echo "ERROR: Screenshot $OUT_DIR/hyprland-$scene.png ($bytes bytes < $min_bytes) looks blank!" >&2
            hyprctl -j layers >&2 || true
            cat "$OUT_DIR/veilshell.log" >&2 || true
            exit 1
        fi
    done

    "$VEILCTL_BIN" mode laptop
    sleep 0.2
    "$VEILCTL_BIN" mode tablet
    sleep 0.2
    "$VEILCTL_BIN" quit
    wait "$DAEMON_PID" || true
    DAEMON_PID=""

    echo "[5/5] Tier 3 live Hyprland + wlr-layer-shell + grim visual test PASSED!"
    exit 0
fi

# Nested fallback when running outside Hyprland (requires host wl_compositor >= v6)
HOST_RT="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
HOST_WL="${WAYLAND_DISPLAY:-}"
if [[ -z "$HOST_WL" ]]; then
    HOST_WL="$(find "$HOST_RT" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' 2>/dev/null | head -n 1 || true)"
fi

if [[ -z "$HOST_WL" ]]; then
    echo "ERROR: No active Hyprland or host Wayland session found." >&2
    echo "Tip: Run ./scripts/test-snapshots.sh for offscreen PNG verification." >&2
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
EOF

BEFORE_SOCKS="$(LC_ALL=C find "$HOST_RT" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' 2>/dev/null | LC_ALL=C sort || true)"
BEFORE_SIGS="$(LC_ALL=C find "$HOST_RT/hypr" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | LC_ALL=C sort || true)"

echo "[2/5] Starting isolated nested Hyprland (host Wayland: $HOST_WL)..."
env XDG_RUNTIME_DIR="$HOST_RT" WAYLAND_DISPLAY="$HOST_WL" \
    Hyprland --config "$TMP_CFG" >"$OUT_DIR/hyprland.log" 2>&1 &
HYPR_PID=$!

CHILD_WL=""
CHILD_SIG=""
for _ in $(seq 1 50); do
    kill -0 "$HYPR_PID" 2>/dev/null || break
    AFTER_SOCKS="$(LC_ALL=C find "$HOST_RT" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' 2>/dev/null | LC_ALL=C sort || true)"
    AFTER_SIGS="$(LC_ALL=C find "$HOST_RT/hypr" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | LC_ALL=C sort || true)"
    CHILD_WL="$(LC_ALL=C comm -13 <(printf '%s\n' "$BEFORE_SOCKS") <(printf '%s\n' "$AFTER_SOCKS") | head -n 1 || true)"
    CHILD_SIG="$(LC_ALL=C comm -13 <(printf '%s\n' "$BEFORE_SIGS") <(printf '%s\n' "$AFTER_SIGS") | head -n 1 || true)"
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

echo "[4/5] Presenting live wlr-layer-shell surfaces & capturing grim screenshots..."
for scene in home shade recents pill; do
    "$VEILSHELL_BIN" --scene "$scene" --exit-after-ms 1600 >"$OUT_DIR/veilshell-$scene.log" 2>&1 &
    SHELL_PID=$!
    sleep 0.6
    grim "$OUT_DIR/hyprland-$scene.png"
    wait "$SHELL_PID"
done

echo "[5/5] Tier 3 nested Hyprland visual test PASSED!"
