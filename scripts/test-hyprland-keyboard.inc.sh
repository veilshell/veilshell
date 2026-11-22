# Sourced by test-hyprland-actions.sh after it creates an isolated compositor.

keyboard_output_state() {
    nested_ctl -j monitors | jq -ecS \
        'sort_by(.name) | map({name,width,height,refreshRate,x,y,scale,transform,reserved})'
}

keyboard_reserved() {
    json_matches monitors 'any(.[]; .name == "veil-smoke" and .reserved[3] == $height)' \
        --argjson height "$1"
}

keyboard_monitors_match() {
    [[ $(keyboard_output_state) == "$1" ]]
}

keyboard_rotate_fixture() {
    [[ $(nested_ctl eval "hl.monitor({ output = \"veil-smoke\", mode = \"960x640@60\", position = \"3000x0\", scale = 1, transform = $1 })") == ok ]]
}

keyboard_scene() {
    timeout 5 "$BIN/veilctl" status | tee "$ROOT/keyboard-status.log" |
        grep -F "scene: $1, visible: true, output: veil-smoke $2," >/dev/null
}

keyboard_has_typed() {
    cmp -s "$keys" <(printf '%s' "$1")
}

test_keyboard_workflow() {
    local mode keys baseline others log daemon terminal
    # Pin the nested parent output rather than comparing the position of an
    # auto-placed monitor, which Hyprland can relocate when the fixture rotates.
    [[ $(nested_ctl eval 'hl.monitor({ output = "WAYLAND-1", mode = "preferred", position = "0x0", scale = "auto" })') == ok ]]
    for mode in overlay standalone; do
        if [[ $mode == standalone ]]; then
            # Same session marker as the standalone launcher, without SDDM,
            # idle/power services or writes to any real session.
            export VEILSHELL_SESSION_DIR="$XDG_RUNTIME_DIR/standalone"
            mkdir -m 700 "$VEILSHELL_SESSION_DIR"
            printf '%s\n' "$WAYLAND_DISPLAY" "$HYPRLAND_INSTANCE_SIGNATURE" \
                >"$VEILSHELL_SESSION_DIR/display"
            export VEILSHELL_SOCKET="$VEILSHELL_SESSION_DIR/control.sock"
        fi
        keys="$ROOT/keys-$mode"
        log="$ROOT/keyboard-$mode.log"
        [[ $(nested_ctl eval 'hl.dispatch(hl.dsp.focus({ monitor = "veil-smoke" }))') == ok ]]
        WAYLAND_DEBUG=client foot --app-id=veil-typing --title=VeilTyping \
            bash -c 'stty -icanon -echo min 1 time 0; exec cat >"$1"' _ "$keys" \
            >"$ROOT/foot-$mode.log" 2>&1 &
        terminal=$!
        PIDS+=("$terminal")
        wait_for "typing fixture focused" json_matches activewindow '.class == "veil-typing"'
        wait_for "typing fixture ready" test -f "$keys"

        baseline="$(keyboard_output_state)"
        others="$(jq -ec 'map(select(.name != "veil-smoke"))' <<<"$baseline")"
        WAYLAND_DEBUG=client "$BIN/veilshell" --no-watchers --policy tablet --osk manual \
            --output veil-smoke --renderer software >"$log" 2>&1 &
        daemon=$!
        PIDS+=("$daemon")
        wait_for "resting pill in $mode" keyboard_scene pill 960x640
        if [[ $mode == standalone ]]; then
            baseline="$(jq -ec 'map(if .name == "veil-smoke" then .reserved[1] = 48 else . end)' <<<"$baseline")"
            wait_for "standalone tablet status reservation" keyboard_monitors_match "$baseline"
        fi
        # A drag starting on the keyboard button must not open Home or the OSK.
        pointer 624 616 525
        keyboard_scene pill 960x640
        pointer 624 616
        wait_for "keyboard opened by its button in $mode" keyboard_scene osk 960x640
        wait_for "landscape work area reserved" keyboard_reserved 213
        if [[ $mode == standalone ]]; then
            json_matches monitors 'any(.[]; .name == "veil-smoke" and .reserved[1] == 48)'
        fi
        pointer 54 458
        wait_for "q delivered to the original terminal" keyboard_has_typed q
        json_matches activewindow '.class == "veil-typing"'

        # Turn the actual output, not just a screenshot or fixture viewport.
        keyboard_rotate_fixture 1
        wait_for "same keyboard on the rotated output" keyboard_scene osk 640x960
        wait_for "portrait work area reserved" keyboard_reserved 320
        [[ $(keyboard_output_state | jq -ec 'map(select(.name != "veil-smoke"))') == "$others" ]]
        VEILSHELL_TEST_POINTER_EXTENT=640x960 pointer 102 686
        wait_for "w delivered after live rotation" keyboard_has_typed qw
        VEILSHELL_TEST_POINTER_EXTENT=640x960 pointer 523 914
        wait_for "HIDE returned to the pill" keyboard_scene pill 640x960
        wait_for "HIDE restored portrait work area" keyboard_reserved 0
        VEILSHELL_TEST_POINTER_EXTENT=640x960 pointer 464 936
        wait_for "keyboard reopened by touch in portrait" keyboard_scene osk 640x960
        wait_for "reopened keyboard reserves space" keyboard_reserved 320
        json_matches activewindow '.class == "veil-typing"'

        if [[ $mode == overlay ]]; then
            # Same visibility edge as an Auto-mode Cover attach, without
            # claiming a synthetic toggle validates the physical device.
            timeout 5 "$BIN/veilctl" mode laptop >/dev/null
            wait_for "hiding overlay releases its OSK reservation" keyboard_reserved 0
            timeout 5 "$BIN/veilctl" status | grep -F 'visible: false' >/dev/null
        else
            VEILSHELL_TEST_POINTER_EXTENT=640x960 pointer 523 914
            wait_for "standalone navigation remains visible" keyboard_scene pill 640x960
            wait_for "standalone work area restored" keyboard_reserved 0
        fi
        keyboard_rotate_fixture 0
        wait_for "baseline monitor state restored" keyboard_monitors_match "$baseline"
        json_matches activewindow '.class == "veil-typing"'
        if grep -Eq '\.bind\(.*"zwp_input_method_manager_v2"|\.get_input_method\(' "$log"; then
            echo "Manual keyboard claimed the input method in $mode." >&2
            return 1
        fi
        grep -Fq '.create_virtual_keyboard(' "$log"
        timeout 5 "$BIN/veilctl" quit >/dev/null
        wait "$daemon"
        kill "$terminal"
        wait "$terminal" || true
        unset 'PIDS[${#PIDS[@]}-1]' # daemon and terminal were reaped above
        unset 'PIDS[${#PIDS[@]}-1]'
        wait_for "typing fixture closed" json_matches clients \
            'all(.[]; .class != "veil-typing")'
        echo "Manual keyboard: $mode focus, typing, live rotation, reopen and work-area cleanup passed."
    done
}
