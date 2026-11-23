# Sourced by test-hyprland-actions.sh after it creates an isolated compositor.
# Virtual-pointer evidence for the no-wl_touch case only. Native gesture replay
# uses a disposable guest, not this pointer client or the active desktop.

gap_compositor_state() {
    local option
    nested_ctl -j monitors | jq -ecS \
        'sort_by(.name) | map({name,width,height,refreshRate,x,y,scale,transform,reserved,activeWorkspace})' || return
    for option in general:gaps_in general:gaps_out general:layout general:border_size \
        general:resize_on_border general:extend_border_grab_area gestures:workspace_swipe_touch; do
        nested_ctl -j getoption "$option" | jq -ecS 'select(has("option"))' || return
    done
    nested_ctl -j binds | jq -ecS 'select(type == "array")' || return
    nested_ctl -j clients | jq -ecS \
        'sort_by(.address) | map({address,at,size,workspace,monitor,mapped,hidden,floating,fullscreen})' || return
    nested_ctl -j layers | jq -ecS 'select(type == "object")'
}

gap_status() {
    timeout 5 "$BIN/veilctl" status >"$ROOT/gap-status.log" &&
        grep -F "visible: $1, output: veil-smoke $VEILSHELL_TEST_POINTER_EXTENT," "$ROOT/gap-status.log" >/dev/null
}

gap_zero_depths() {
    gap_status true &&
        grep -Fxq '  overlay gaps: top 0px, bottom 0px (logical)' "$ROOT/gap-status.log"
}

gap_daemon_status() {
    grep -F 'daemon     : running (scene:' "$ROOT/gap-status.log"
}

gap_assert_clicks() {
    local count="$1"
    local bar_press='^pointer press button=272 '
    local app_press='wl_pointer[@#][0-9]+\.button\([0-9]+, [0-9]+, 272, 1\)'

    pointer 480 16
    wait_for "bar received BTN_LEFT press $count" gap_log_count "$ROOT/gap-bar.log" "$bar_press" "$count"
    json_matches activewindow '.class == "veil-gap-test-app"'
    pointer 480 320
    wait_for "app received BTN_LEFT press $count" gap_log_count "$ROOT/gap-app.log" "$app_press" "$count"
    gap_log_count "$ROOT/gap-bar.log" "$bar_press" "$count"
    json_matches activewindow '.class == "veil-gap-test-app"'
}

gap_log_count() {
    [[ $(grep -Ec "$2" "$1" || true) == "$3" ]]
}

gap_assert_unchanged() {
    diff -u "$ROOT/gap-baseline.lua" "$ROOT/hyprland.lua"
    gap_compositor_state >"$ROOT/gap-current.jsonl"
    diff -u "$ROOT/gap-baseline.jsonl" "$ROOT/gap-current.jsonl"
}

test_overlay_gaps() {
    local daemon idle
    nested_ctl version >"$ROOT/gap-hyprland-version.log"
    "$BIN/examples/hyprland_gap_test_bar" "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" veil-smoke \
        >"$ROOT/gap-bar.log" 2>&1 &
    PIDS+=("$!")
    wait_for "test bar reserves exactly 32 px on top" json_matches monitors \
        'any(.[]; .name == "veil-smoke" and .reserved == [0,32,0,0])'
    wait_for "test bar mapped on the TOP layer" json_matches layers \
        'any(.["veil-smoke"].levels["2"][];
             .namespace == "veil-gap-test-bar" and .y == 0 and .w == 960 and .h == 32)'
    if grep -Eq '^seat capabilities: seats=[1-9][0-9]* touch=true$' "$ROOT/gap-bar.log"; then
        echo "SKIP: nested seat advertises wl_touch; this replay only covers the no-touch fallback."
        return 0
    fi
    grep -Eq '^seat capabilities: seats=[1-9][0-9]* touch=false$' "$ROOT/gap-bar.log"

    [[ $(nested_ctl eval 'hl.dispatch(hl.dsp.focus({ monitor = "veil-smoke" }))') == ok ]]
    WAYLAND_DEBUG=client foot --hold --app-id=veil-gap-test-app --title=VeilGapTestApp true \
        >"$ROOT/gap-app.log" 2>&1 &
    PIDS+=("$!")
    wait_for "normal app on the smoke output, between the two gap starts" json_matches activewindow \
        '.class == "veil-gap-test-app" and .floating == false and .fullscreen == 0
         and .at[0] >= 3000 and .at[0] < 3480 and .at[0] + .size[0] > 3480
         and .at[1] > 42 and .at[1] < 320 and .at[1] + .size[1] > 320 and .at[1] + .size[1] < 630'
    gap_assert_clicks 1
    cp "$ROOT/hyprland.lua" "$ROOT/gap-baseline.lua"
    gap_compositor_state >"$ROOT/gap-baseline.jsonl"

    WAYLAND_DEBUG=client "$BIN/veilshell" --overlay-gap-gestures --no-watchers --policy tablet --osk manual \
        --output veil-smoke --renderer software >"$ROOT/gap-veilshell.log" 2>&1 &
    daemon=$!
    PIDS+=("$daemon")
    wait_for "visible opt-in reports zero top and bottom strip depths" gap_zero_depths
    idle="$(gap_daemon_status)"
    # The layer inventory must still be identical: no gap strips, pill or other
    # replacement idle surface, even though the overlay policy is visible.
    gap_assert_unchanged
    gap_assert_clicks 2
    pointer 480 42 220
    gap_zero_depths
    [[ $(gap_daemon_status) == "$idle" ]]
    pointer 480 630 480
    gap_zero_depths
    [[ $(gap_daemon_status) == "$idle" ]]
    gap_assert_unchanged
    json_matches activewindow '.class == "veil-gap-test-app"'

    timeout 5 "$BIN/veilctl" mode laptop >/dev/null
    wait_for "overlay hidden" gap_status false
    gap_assert_unchanged
    timeout 5 "$BIN/veilctl" mode tablet >/dev/null
    wait_for "re-enabled overlay still reports zero strip depths" gap_zero_depths
    gap_assert_unchanged
    gap_assert_clicks 3
    timeout 5 "$BIN/veilctl" quit >/dev/null
    wait "$daemon"
    unset 'PIDS[-1]'
    gap_assert_unchanged
    # Also catch surfaces that were created and withdrawn between IPC snapshots.
    grep -Fq '.get_registry(' "$ROOT/gap-veilshell.log"
    if grep -Fq '.get_layer_surface(' "$ROOT/gap-veilshell.log"; then
        echo "The no-touch opt-in requested a layer surface." >&2
        return 1
    fi
    echo "Virtual-pointer/no-touch replay passed: no idle surfaces, unchanged Hyprland settings/work areas, and bar/app clicks delivered."
}
