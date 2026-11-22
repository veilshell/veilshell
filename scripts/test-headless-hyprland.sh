#!/usr/bin/env bash
# Tier 3: use a fresh nested Hyprland, never the host compositor's IPC.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-/tmp/veilshell-hypr-test}"
RENDERER="${VEILSHELL_TEST_RENDERER:-auto}"
case "$RENDERER" in auto|software|wgpu) ;; *) echo "Invalid renderer: $RENDERER" >&2; exit 2;; esac

for tool in cargo Hyprland hyprctl grim jq; do
    command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 1; }
done

# Use the host Wayland socket only as a display for the nested compositor.
PARENT="${WAYLAND_DISPLAY:-}"
if [[ -z "$PARENT" ]]; then
    echo "A parent Wayland display is required to run nested Hyprland." >&2
    exit 1
fi
if [[ "$PARENT" != /* ]]; then
    PARENT="${XDG_RUNTIME_DIR:?Parent XDG_RUNTIME_DIR is required}/$PARENT"
fi
[[ -S "$PARENT" ]] || { echo "Parent Wayland socket not found: $PARENT" >&2; exit 1; }

mkdir -p "$OUT_DIR"
echo "Building VeilShell..."
cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --bin veilshell --bin veilctl
TARGET_DIR="$(cargo metadata --locked --no-deps --format-version 1 --manifest-path "$REPO_ROOT/Cargo.toml" | jq -er .target_directory)"
BIN="$TARGET_DIR/debug"

# Hyprland embeds a long signature in its Unix socket path (108-byte limit).
TEST_ROOT="$(mktemp -d /tmp/veil.XXXXXX)"
HYPR_PID=""
DAEMON_PID=""
cleanup() {
    for pid in "$DAEMON_PID" "$HYPR_PID"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
    rm -rf -- "$TEST_ROOT"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

export XDG_RUNTIME_DIR="$TEST_ROOT/r"
export XDG_CONFIG_HOME="$TEST_ROOT/config"
export XDG_CACHE_HOME="$TEST_ROOT/cache"
export XDG_STATE_HOME="$TEST_ROOT/state"
mkdir -m 700 "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"
export HYPRLAND_NO_SD_VARS=1 HYPRLAND_NO_SD_TARGET=1 HYPRLAND_NO_SD_NOTIFY=1
export HYPRLAND_NO_CRASHREPORTER=1
# No real seat or host notification/activation bus, even inside Omarchy.
export LIBSEAT_BACKEND=seatd SEATD_SOCK="$TEST_ROOT/no-seat"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$TEST_ROOT/no-user-bus"
export GSETTINGS_BACKEND=memory
export XDG_CURRENT_DESKTOP=Hyprland
unset DISPLAY WAYLAND_SOCKET HYPRLAND_INSTANCE_SIGNATURE NOTIFY_SOCKET VEILSHELL_SESSION_DIR
export VEILSHELL_SOCKET="$XDG_RUNTIME_DIR/veil.sock"

cat >"$TEST_ROOT/hyprland.lua" <<'EOF'
hl.config({
    general = { gaps_in = 19, gaps_out = 23 },
    decoration = { rounding = 11 },
    cursor = { sync_gsettings_theme = false },
    animations = { enabled = false },
    xwayland = { enabled = false },
    misc = { disable_hyprland_logo = true, disable_splash_rendering = true, disable_watchdog_warning = true },
})
EOF

echo "Starting a private Hyprland compositor..."
(
    ulimit -c 0
    export WAYLAND_DISPLAY="$PARENT"
    exec Hyprland --config "$TEST_ROOT/hyprland.lua"
) >"$OUT_DIR/hyprland.log" 2>&1 &
HYPR_PID=$!
shopt -s nullglob
for _ in {1..50}; do
    locks=("$XDG_RUNTIME_DIR"/hypr/*/hyprland.lock)
    if (( ${#locks[@]} == 1 )); then
        mapfile -t endpoint <"${locks[0]}" 2>/dev/null || true
        if [[ ${endpoint[0]:-} == "$HYPR_PID" && ${endpoint[1]:-} == wayland-* &&
              -S "$XDG_RUNTIME_DIR/${endpoint[1]}" && -S "${locks[0]%/*}/.socket.sock" ]]; then
            export WAYLAND_DISPLAY="${endpoint[1]}"
            instance_dir="${locks[0]%/*}"
            export HYPRLAND_INSTANCE_SIGNATURE="${instance_dir##*/}"
            break
        fi
    fi
    kill -0 "$HYPR_PID" 2>/dev/null || break
    sleep 0.1
done
if [[ -z ${HYPRLAND_INSTANCE_SIGNATURE:-} ]]; then
    echo "Nested Hyprland could not start; check the parent compositor's protocol support and $OUT_DIR/hyprland.log" >&2
    exit 1
fi

nested_ctl() { hyprctl -i "$HYPRLAND_INSTANCE_SIGNATURE" "$@"; }
OUTPUT=veil-test
[[ $(nested_ctl output create headless "$OUTPUT") == ok ]]

compositor_state() {
    nested_ctl -j monitors | jq -c 'map({name, width, height, scale, transform, x, y})'
    for option in general:gaps_in general:gaps_out decoration:rounding; do
        nested_ctl -j getoption "$option" | jq -c '{option, value: (.int // .css)}'
    done
}

configure_output() {
    local mode="$1" scale="$2" width="${1%x*}" height="${1#*x}"
    [[ $(nested_ctl eval "hl.monitor({ output = \"$OUTPUT\", mode = \"${mode}@60\", position = \"3000x0\", scale = $scale })") == ok ]]
    for _ in {1..30}; do
        if nested_ctl -j monitors | jq -e --arg name "$OUTPUT" --argjson scale "$scale" \
            --argjson w "$width" --argjson h "$height" \
            'any(.[]; .name == $name and .scale == $scale and .width == $w and .height == $h)' >/dev/null; then
            return
        fi
        sleep 0.1
    done
    echo "Nested output did not reach ${mode} @ ${scale}x" >&2
    return 1
}

configure_output 960x640 1
before="$(compositor_state)"
if "$BIN/veilctl" mode tablet >"$OUT_DIR/mode-no-daemon.log" 2>&1; then
    echo "Mode command unexpectedly succeeded without a VeilShell daemon." >&2
    exit 1
fi
[[ $(compositor_state) == "$before" ]]

TRACE="${VEILSHELL_TEST_WAYLAND_DEBUG:-0}"
if [[ "$RENDERER" == software ]]; then
    TRACE=client
fi
WAYLAND_DEBUG="$TRACE" "$BIN/veilshell" --no-watchers --policy tablet \
    --output "$OUTPUT" --renderer "$RENDERER" >"$OUT_DIR/veilshell.log" 2>&1 &
DAEMON_PID=$!
for _ in {1..50}; do
    [[ -S "$VEILSHELL_SOCKET" ]] && break
    kill -0 "$DAEMON_PID" 2>/dev/null || break
    sleep 0.1
done
[[ -S "$VEILSHELL_SOCKET" ]] || { cat "$OUT_DIR/veilshell.log" >&2; exit 1; }

echo "Checking mode transitions don't change compositor settings..."
before="$(compositor_state)"
[[ $("$BIN/veilctl" mode laptop) == "Mode set to Laptop "* ]]
"$BIN/veilctl" status | grep -F 'visible: false'
[[ $("$BIN/veilctl" mode tablet) == "Mode set to Tablet "* ]]
"$BIN/veilctl" status | grep -F 'visible: true'
[[ $(compositor_state) == "$before" ]]

for orientation in landscape portrait; do
    for factor in 1 1.5 2; do
        case "$factor" in 1) w=960; h=640;; 1.5) w=1440; h=960;; 2) w=1920; h=1280;; esac
        if [[ "$orientation" == portrait ]]; then
            short="$h"
            h="$w"
            w="$short"
        fi
        configure_output "${w}x${h}" "$factor"
        echo "Capturing $orientation @ ${factor}x ($RENDERER)..."
        previous=""
        for scene in home shade recents osk pill; do
            "$BIN/veilctl" show "$scene" >/dev/null
            image="$OUT_DIR/${orientation}-${factor}x-${scene}.png"
            min_bytes=2000
            if [[ "$scene" == pill ]]; then
                min_bytes=500
            fi
            for _ in {1..8}; do
                sleep 0.25
                grim -o "$OUTPUT" -s "$factor" "$image"
                if (( $(stat -c%s "$image") >= min_bytes )) &&
                    { [[ -z "$previous" ]] || ! cmp -s "$image" "$previous"; }; then
                    break
                fi
            done
            if (( $(stat -c%s "$image") < min_bytes )) ||
                { [[ -n "$previous" ]] && cmp -s "$image" "$previous"; }; then
                echo "Blank or stale screenshot: $image" >&2
                exit 1
            fi
            # PNG's IHDR stores frame dimensions as two big-endian 32-bit integers.
            read -r actual_w actual_h < <(od --endian=big -An -j 16 -N 8 -t u4 "$image")
            [[ "$actual_w" == "$w" && "$actual_h" == "$h" ]]
            previous="$image"
        done
        if [[ "$RENDERER" == software ]] &&
            ! grep -Eq "wl_shm_pool.*create_buffer\\(.*,[[:space:]]*$w, $h, $((w * 4)), " "$OUT_DIR/veilshell.log"; then
            echo "No native ${w}x${h} SHM buffer for $orientation @ ${factor}x" >&2
            exit 1
        fi
    done
done

"$BIN/veilctl" quit >/dev/null
wait "$DAEMON_PID"
DAEMON_PID=""
if [[ "$RENDERER" == wgpu ]] && grep -E 'falling back|cannot present|using wl_shm' "$OUT_DIR/veilshell.log"; then
    echo "The requested wgpu swapchain fell back during testing." >&2
    exit 1
fi
echo "Nested visual checks passed. Screenshots: $OUT_DIR"
