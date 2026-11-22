#!/usr/bin/env bash
# Live smoke: edge swipes, Home launch and Recents actions in disposable Hyprland.
# Requires Hyprland 0.56+, foot, jq, and a Wayland parent for display only.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
for tool in cargo Hyprland hyprctl foot jq timeout; do
    command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 1; }
done
PARENT="${WAYLAND_DISPLAY:?A parent Wayland display is required}"
if [[ "$PARENT" != /* ]]; then
    PARENT="${XDG_RUNTIME_DIR:?Parent XDG_RUNTIME_DIR is required}/$PARENT"
fi
[[ -S "$PARENT" ]] || { echo "Parent Wayland socket not found: $PARENT" >&2; exit 1; }

cd "$REPO_ROOT"
cargo build --locked --bin veilshell --bin veilctl --example hyprland_smoke_pointer
TARGET_DIR="$(cargo metadata --locked --no-deps --format-version 1 | jq -er .target_directory)"
BIN="$TARGET_DIR/debug"

# Keep the path short enough for Hyprland's Unix sockets.
ROOT="$(mktemp -d /tmp/veil.XXXXXX)"
PIDS=()
cleanup() {
    local rc=$? pid
    for (( pid=${#PIDS[@]}-1; pid>=0; pid-- )); do
        kill "${PIDS[pid]}" 2>/dev/null || true
        wait "${PIDS[pid]}" 2>/dev/null || true
    done
    if (( rc != 0 )); then
        echo "Smoke failed; logs: $ROOT" >&2
    else
        rm -rf -- "$ROOT"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

export HOME="$ROOT/home" XDG_RUNTIME_DIR="$ROOT/r" XDG_CONFIG_HOME="$ROOT/config"
export XDG_CACHE_HOME="$ROOT/cache" XDG_STATE_HOME="$ROOT/state"
export XDG_DATA_HOME="$ROOT/data" XDG_DATA_DIRS="$ROOT/empty" XDG_CONFIG_DIRS="$ROOT/empty"
mkdir -m 700 -p "$HOME" "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" \
    "$XDG_STATE_HOME" "$XDG_DATA_HOME/applications" "$XDG_DATA_DIRS"
export HYPRLAND_NO_SD_VARS=1 HYPRLAND_NO_SD_TARGET=1 HYPRLAND_NO_SD_NOTIFY=1
export HYPRLAND_NO_CRASHREPORTER=1
export LIBSEAT_BACKEND=seatd SEATD_SOCK="$ROOT/no-seat"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$ROOT/no-user-bus"
export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$ROOT/no-system-bus" GSETTINGS_BACKEND=memory
export XDG_CURRENT_DESKTOP=Hyprland VEILSHELL_SOCKET="$XDG_RUNTIME_DIR/veil.sock"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET HYPRLAND_INSTANCE_SIGNATURE NOTIFY_SOCKET VEILSHELL_SESSION_DIR OMARCHY_PATH

# Never send these test taps to real radios or audio. The fixture records the
# outgoing commands and reports the applied state to the same panel.
export VEIL_TEST_STATE="$ROOT"
mkdir "$ROOT/tools"
cat >"$ROOT/tools/nmcli" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  '-t -f WIFI,WWAN general') echo 'enabled:disabled' ;;
  '-t -f TYPE,STATE,CONNECTION dev status') echo 'wifi:connected:Home' ;;
  '-t -e yes -f UUID,TYPE,NAME,DEVICE connection show')
    if [[ -e "$VEIL_TEST_STATE/wifi-connected" ]]; then
      echo 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee:802-11-wireless:Home:'
      echo '11111111-2222-3333-4444-555555555555:802-11-wireless:Guest:wlan0'
    else
      echo 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee:802-11-wireless:Home:wlan0'
      echo '11111111-2222-3333-4444-555555555555:802-11-wireless:Guest:'
    fi ;;
  '--wait 14 connection up uuid 11111111-2222-3333-4444-555555555555') touch "$VEIL_TEST_STATE/wifi-connected" ;;
  'radio wifi '*) touch "$VEIL_TEST_STATE/wifi-radio-toggled" ;;
  *) exit 1 ;;
esac
EOF
cat >"$ROOT/tools/wpctl" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  get-volume)
    if [[ -e "$VEIL_TEST_STATE/muted" ]]; then echo 'Volume: 0.50 [MUTED]'; else echo 'Volume: 0.50'; fi ;;
  set-mute)
    [[ $2 == @DEFAULT_AUDIO_SINK@ && $3 == 1 ]] || exit 1
    touch "$VEIL_TEST_STATE/muted" ;;
  *) exit 1 ;;
esac
EOF
cat >"$ROOT/tools/pactl" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  'get-default-sink') if [[ -e "$VEIL_TEST_STATE/headphones" ]]; then echo headphones; else echo speakers; fi ;;
  '--format=json list sinks') echo '[{"name":"speakers","description":"Speakers"},{"name":"headphones","description":"Headphones"}]' ;;
  'set-default-sink headphones') touch "$VEIL_TEST_STATE/headphones" ;;
  *) exit 1 ;;
esac
EOF
for tool in bluetoothctl playerctl powerprofilesctl; do
    printf '#!/bin/sh\nexit 1\n' >"$ROOT/tools/$tool"
done
chmod +x "$ROOT"/tools/*
export PATH="$ROOT/tools:$PATH"

cat >"$ROOT/hyprland.lua" <<'EOF'
hl.config({
    cursor = { sync_gsettings_theme = false },
    input = { follow_mouse = 0 },
    animations = { enabled = false },
    xwayland = { enabled = false },
    misc = { disable_hyprland_logo = true, disable_splash_rendering = true, disable_watchdog_warning = true },
})
EOF
(ulimit -c 0; WAYLAND_DISPLAY="$PARENT" exec Hyprland --config "$ROOT/hyprland.lua") \
    >"$ROOT/hyprland.log" 2>&1 &
PIDS+=("$!")
shopt -s nullglob
for _ in {1..60}; do
    locks=("$XDG_RUNTIME_DIR"/hypr/*/hyprland.lock)
    if (( ${#locks[@]} == 1 )); then
        mapfile -t endpoint <"${locks[0]}" 2>/dev/null || true
        if [[ ${endpoint[0]:-} == "${PIDS[0]}" && ${endpoint[1]:-} =~ ^wayland-[0-9]+$ &&
              -S "$XDG_RUNTIME_DIR/${endpoint[1]}" && -S "${locks[0]%/*}/.socket.sock" ]]; then
            export WAYLAND_DISPLAY="${endpoint[1]}"
            instance_dir="${locks[0]%/*}"
            export HYPRLAND_INSTANCE_SIGNATURE="${instance_dir##*/}"
            break
        fi
    fi
    kill -0 "${PIDS[0]}" 2>/dev/null || break
    sleep 0.1
done
[[ -n ${HYPRLAND_INSTANCE_SIGNATURE:-} ]] || { echo "Nested Hyprland did not start" >&2; exit 1; }
nested_ctl() { timeout 5 hyprctl -i "$HYPRLAND_INSTANCE_SIGNATURE" "$@"; }
pointer() {
    timeout 5 "$BIN/examples/hyprland_smoke_pointer" "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" "$@"
}
wait_for() {
    local message="$1"
    shift
    for _ in {1..50}; do
        if "$@"; then return; fi
        sleep 0.1
    done
    echo "Timed out: $message" >&2
    return 1
}
json_matches() {
    local query="$1" filter="$2"
    shift 2
    nested_ctl -j "$query" | jq -e "$@" "$filter" >/dev/null
}
show() {
    timeout 5 "$BIN/veilctl" show "$1" >/dev/null
    # Allow the real layer surface to configure and its reveal animation to settle.
    sleep 0.8
}
in_scene() {
    timeout 5 "$BIN/veilctl" status | tee "$ROOT/status.log" |
        grep -F "scene: $1, visible: true, output: veil-smoke 960x640," >/dev/null
}

[[ $(nested_ctl output create headless veil-smoke) == ok ]]
[[ $(nested_ctl eval 'hl.monitor({ output = "veil-smoke", mode = "960x640@60", position = "3000x0", scale = 1 })') == ok ]]
wait_for "960x640 smoke output" json_matches monitors \
    'any(.[]; .name == "veil-smoke" and .width == 960 and .height == 640 and .scale == 1)'

if [[ ${VEILSHELL_TEST_KEYBOARD:-0} == 1 ]]; then
    # shellcheck source=scripts/test-hyprland-keyboard.inc.sh
    source "$REPO_ROOT/scripts/test-hyprland-keyboard.inc.sh"
    test_keyboard_workflow
    exit 0
fi

cat >"$XDG_DATA_HOME/applications/veil-home.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Veil Home Fixture
Exec=foot --hold --app-id=veil-home --title=VeilHome true
EOF
# Keep an app focused behind the handles. On an empty workspace Hyprland
# can end a mouse grab as soon as the pointer crosses a layer boundary.
[[ $(nested_ctl eval 'hl.dispatch(hl.dsp.focus({ monitor = "veil-smoke" }))') == ok ]]
foot --hold --app-id=veil-other --title=VeilOther true >"$ROOT/foot.log" 2>&1 &
PIDS+=("$!")
wait_for "underlying fixture focused" json_matches activewindow '.class == "veil-other"'

if [[ ${VEILSHELL_TEST_LAUNCHER:-0} == 1 ]]; then
    command -v wtype >/dev/null || { echo "Missing tool: wtype" >&2; exit 1; }
    cat >"$XDG_DATA_HOME/applications/veil-second.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Veil Home Second
Exec=foot --hold --app-id=veil-second true
EOF
    "$BIN/veilshell" --no-watchers --policy laptop --osk manual --output veil-smoke \
        --renderer software >"$ROOT/launcher.log" 2>&1 &
    PIDS+=("$!")
    wait_for "launcher test socket" test -S "$VEILSHELL_SOCKET"
    show launcher
    in_scene launcher
    wtype 'unmatched query'
    wtype -k Return
    in_scene launcher
    json_matches clients 'map(.class) == ["veil-other"]'
    wtype -k Escape
    wait_for "Escape restored hidden mode" bash -c \
        '"$1" status | grep -q "visible: false"' _ "$BIN/veilctl"
    show launcher
    in_scene launcher
    wtype 'veil home'
    wtype -k Right -k Return
    wait_for "launcher filtered, selected second result and launched it" json_matches clients \
        'map(.class) | sort == ["veil-other", "veil-second"]'
    timeout 5 "$BIN/veilctl" quit >/dev/null
    echo "Desktop launcher search, empty results, Escape and keyboard launch passed."
    exit 0
fi

if [[ ${VEILSHELL_TEST_STANDALONE:-0} == 1 ]]; then
    # Exercise standalone policy without starting SDDM or host services.
    export VEILSHELL_SESSION_DIR="$XDG_RUNTIME_DIR/standalone"
    mkdir -m 700 "$VEILSHELL_SESSION_DIR"
    printf '%s\n' "$WAYLAND_DISPLAY" "$HYPRLAND_INSTANCE_SIGNATURE" >"$VEILSHELL_SESSION_DIR/display"
    export VEILSHELL_SOCKET="$VEILSHELL_SESSION_DIR/control.sock"
fi

WAYLAND_DEBUG="${VEILSHELL_TEST_WAYLAND_DEBUG:-0}" \
    "$BIN/veilshell" --no-watchers --policy tablet --output veil-smoke --renderer software \
    >"$ROOT/veilshell.log" 2>&1 &
PIDS+=("$!")
wait_for "VeilShell socket" test -S "$VEILSHELL_SOCKET"
# The fixture terminal may auto-open the OSK once on initial IM activation.
show pill
show pill
wait_for "initial Pill on smoke output" in_scene pill
# A short drag from padding must cancel; a tap there is not a button press.
pointer 480 600
in_scene pill
pointer 480 600 588
wait_for "short Home pull canceled" in_scene pill
# Neither edge grabs the transparent sides.
pointer 105 600 516
in_scene pill
pointer 575 36 120
in_scene pill
pointer 480 36
in_scene pill
pointer 480 36 120
wait_for "short top swipe opened Quick Settings" in_scene shade
show pill
pointer 480 600 516
wait_for "short bottom swipe opened Home" in_scene home
echo "Edge grab zones, swipe-only padding and short pulls passed."
sleep 0.8
# The only discovered app is in the first Home grid cell.
pointer 110 110
wait_for "Home launched its fixture" json_matches clients \
    'map(.class) | sort == ["veil-home", "veil-other"]'
echo "Home launch passed."

if [[ ${VEILSHELL_TEST_STANDALONE:-0} == 1 ]]; then
    # Focus a different window first so that a no-op can't pass.
    home_address="$(nested_ctl -j clients | jq -er '.[] | select(.class == "veil-home") | .address')"
    other_address="$(nested_ctl -j clients | jq -er '.[] | select(.class == "veil-other") | .address')"
    nested_ctl eval "hl.dispatch(hl.dsp.focus({ window = \"address:$other_address\" }))" >/dev/null
    wait_for "other window focused before reactivation" json_matches activewindow '.address == $id' --arg id "$other_address"
    show pill
    pointer 480 600 516
    wait_for "second bottom swipe opened Home" in_scene home
    sleep 0.8
    pointer 110 110
    wait_for "standalone Home focused the existing address" json_matches activewindow \
        '.address == $id' --arg id "$home_address"
    json_matches clients 'map(.class) | sort == ["veil-home", "veil-other"]'
    echo "Standalone Home focused the existing app without a duplicate."
    timeout 5 "$BIN/veilctl" quit >/dev/null
    exit 0
fi

# Recents centers the most recently focused client. Tap its neighbor so a
# no-op focus dispatcher cannot pass.
mapfile -t addresses < <(nested_ctl -j clients | jq -r 'sort_by(.focusHistoryID) | .[].address')
[[ ${#addresses[@]} == 2 ]]
other="${addresses[0]}"
target="${addresses[1]}"
wait_for "initial fixture focused" json_matches activewindow '.address == $id' --arg id "$other"
show recents
# In the 960x640 landscape deck, the right-hand card is centered near x=934.
pointer 920 320
wait_for "Recents focused the second fixture" json_matches activewindow '.address == $id' --arg id "$target"
echo "Recents focus passed."

# The focused fixture is now centered when Recents is reopened.
show recents
pointer 480 320 145
wait_for "Recents closed only the focused fixture" json_matches clients \
    'map(.address) == [$id]' --arg id "$other"
echo "Recents close passed."

show shade
# Detail tiles and the actual device commands, against fixture services.
pointer 100 244
sleep 0.2
pointer 480 152
wait_for "saved Wi-Fi activated by UUID" test -e "$ROOT/wifi-connected"
[[ ! -e "$ROOT/wifi-radio-toggled" ]]
pointer 228 46
sleep 0.5
pointer 595 401
wait_for "audio muted" test -e "$ROOT/muted"
pointer 818 401
sleep 0.2
pointer 480 152
wait_for "default output changed" test -e "$ROOT/headphones"
echo "Quick Settings saved Wi-Fi, mute, and audio output passed."
timeout 5 "$BIN/veilctl" quit >/dev/null
