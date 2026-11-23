# Standalone Tablet virtual-pointer replay. Source only from the private
# Hyprland fixture in test-hyprland-actions.sh.

tablet_bottom_up() {
    local width="${VEILSHELL_TEST_POINTER_EXTENT%x*}" height="${VEILSHELL_TEST_POINTER_EXTENT#*x}"
    # 240 px over six 20 ms steps (~2000 px/s). An armed slow release
    # from the app would open Recents, not Home.
    pointer "$((width / 2))" "$((height - 40))" "$((height - 280))"
}

tablet_home_recents_flick() {
    # 36 px is below 6% of the 640 px landscape height. Only a fast
    # release can open Recents; arming alone cannot pass this check.
    VEILSHELL_TEST_POINTER_STEP_MS=4 pointer 480 600 564
    sleep 0.6 # A canceled preview can still report Recents until it retracts.
    wait_for "Fast unarmed Tablet bottom flick opened Recents from Home" in_scene recents
}

tablet_recents_down() {
    local width="${VEILSHELL_TEST_POINTER_EXTENT%x*}" height="${VEILSHELL_TEST_POINTER_EXTENT#*x}"
    # The on-screen hit zone is only 52 px tall. Test >650 px/s rather
    # than trying to travel >72 px outside the output with a mouse grab.
    VEILSHELL_TEST_POINTER_STEP_MS=7 \
        pointer "$((width / 2))" "$((height - 49))" "$((height - 2))"
    wait_for "Tablet Recents down flick returned to Home" in_scene home
}

tablet_expect_windows() {
    json_matches clients 'map(.address) | sort == $windows' --argjson windows "$1"
}

tablet_focus_fixture() {
    local id="$1"
    [[ $(nested_ctl eval "hl.dispatch(hl.dsp.focus({ window = \"address:$id\" }))") == ok ]]
    wait_for "fixture $id focused for test setup" json_matches activewindow '.address == $id' --arg id "$id"
}

tablet_monitor_id() {
    nested_ctl -j monitors | jq -er --arg output "$1" '.[] | select(.name == $output) | .id'
}

tablet_report_focus() {
    echo "Actual compositor focus:" >&2
    nested_ctl -j activewindow | jq '{class,address,monitor,workspace}' >&2
    nested_ctl -j clients | jq 'map({class,address,monitor,workspace,focusHistoryID})' >&2
    nested_ctl -j monitors | jq 'map({name,id,focused,activeWorkspace})' >&2
}

test_tablet_adjacent_apps() {
    local older="$1" middle="$2" newer="$3" local_monitor remote_monitor remote workspace windows
    local route direction target end_x
    [[ $(nested_ctl output create headless veil-remote) == ok ]]
    [[ $(nested_ctl eval 'hl.monitor({ output = "veil-remote", mode = "800x600@60", position = "5000x0", scale = 1 })') == ok ]]
    wait_for "second private output" json_matches monitors 'any(.[]; .name == "veil-remote")'
    remote_monitor="$(tablet_monitor_id veil-remote)"
    local_monitor="$(tablet_monitor_id veil-smoke)"
    [[ $remote_monitor != "$local_monitor" ]]
    [[ $(nested_ctl eval 'hl.dispatch(hl.dsp.focus({ monitor = "veil-remote" }))') == ok ]]
    wait_for "remote output focused for fixture placement" json_matches monitors \
        'any(.[]; .id == $id and .focused)' --argjson id "$remote_monitor"
    foot --hold --app-id=veil-remote --title=VeilRemote true >"$ROOT/foot-remote.log" 2>&1 &
    PIDS+=("$!")
    wait_for "fixture on another output" json_matches activewindow \
        '.class == "veil-remote" and .monitor == $id' --argjson id "$remote_monitor"
    remote="$(nested_ctl -j activewindow | jq -er .address)"
    # Cross-monitor focus can implicitly raise a workspace's last window.
    # Seed the three local entries after that; relative global history is
    # deliberately not part of the fixture.
    for target in "$older" "$middle" "$newer"; do
        tablet_focus_fixture "$target"
    done
    workspace="$(nested_ctl -j monitors | jq -er '.[] | select(.name == "veil-smoke") | .activeWorkspace.id')"
    if ! json_matches clients '
        [ .[] | select(.monitor == $monitor and .mapped) ]
        | all(.[]; .workspace.id == $workspace and .focusHistoryID >= 0)
          and (sort_by(.focusHistoryID) | map(.address) == [$newer, $middle, $older])
    ' --argjson monitor "$local_monitor" --argjson workspace "$workspace" \
        --arg newer "$newer" --arg middle "$middle" --arg older "$older"; then
        echo "Expected three local fixtures in newest→middle→oldest order on workspace $workspace" >&2
        tablet_report_focus
        return 1
    fi
    windows="$(nested_ctl -j clients | jq -c 'map(.address) | sort')"

    # No re-focusing or opening a scene between these swipes. Re-sorting the
    # deck after each new focus would bounce instead of walking three addresses.
    # The boundary swipes must also clamp with a real remote fixture present.
    for route in "right:$middle" "right:$older" "right:$older" \
                 "left:$middle" "left:$newer" "left:$newer"; do
        direction="${route%%:*}"
        target="${route#*:}"
        end_x=310
        [[ $direction != right ]] || end_x=650
        pointer 480 600 "$end_x:600"
        in_scene pill
        if ! wait_for "Tablet swipe $direction focused local window $target" json_matches activewindow \
            '.address == $id and .monitor == $monitor and .workspace.id == $workspace' \
            --arg id "$target" --argjson monitor "$local_monitor" --argjson workspace "$workspace" ||
            ! json_matches clients '
                (map(.address) | sort == $windows)
                and any(.[]; .address == $remote and .class == "veil-remote" and .mapped and .monitor == $monitor)
            ' --argjson windows "$windows" --arg remote "$remote" --argjson monitor "$remote_monitor" ||
            ! json_matches monitors 'any(.[]; .id == $monitor and .focused and .activeWorkspace.id == $workspace)' \
                --argjson monitor "$local_monitor" --argjson workspace "$workspace"; then
            echo "Swipe $direction to $target changed fixture identities, output, workspace, or focus unexpectedly" >&2
            tablet_report_focus
            return 1
        fi
    done
    echo "Tablet right/left swipes walked three local apps without wrap, remote focus, launch, or workspace changes."
}

test_tablet_navigation() {
    local other="$1" home="$2" windows history sheet held_pointer
    windows="$(nested_ctl -j clients | jq -c 'map(.address) | sort')"
    tablet_focus_fixture "$home"
    history="$(nested_ctl -j clients | jq -c 'sort_by(.focusHistoryID) | map(.address)')"

    # Cross 6% (38.4 px), then reverse below 4.5% (28.8 px) and hold.
    pointer 480 600 505 505 505 505 505 505 505 588 588 &
    held_pointer=$!
    PIDS+=("$held_pointer")
    wait_for "Recents preview started before reversal" in_scene recents
    wait "$held_pointer"
    unset 'PIDS[-1]'
    wait_for "Reversing below the disarm distance restored the same app" in_scene pill
    json_matches activewindow '.address == $id' --arg id "$home"
    tablet_expect_windows "$windows"
    json_matches clients 'sort_by(.focusHistoryID) | map(.address) == $history' --argjson history "$history"

    # Slow ~250 px/s ascent and two stationary waypoints past arm.
    pointer 480 600 570 540 510 510 510
    sleep 0.6
    in_scene recents
    tablet_recents_down
    sleep 0.6
    tablet_home_recents_flick
    tablet_recents_down
    sleep 0.6

    # End the opening drag where the second drawer tile will appear. Only
    # the later fresh tap may activate it. The short list starts at scroll=0.
    pointer 362 355 118
    wait_for "Landscape Home content flick opened Drawer" in_scene drawer
    sleep 0.6
    in_scene drawer
    tablet_expect_windows "$windows"
    pointer 362 118 360
    wait_for "Downward pull at the top of Drawer returned to Home" in_scene home
    tablet_expect_windows "$windows"

    # Check the supported live command independently of the gesture.
    show drawer
    in_scene drawer
    # Return through the top pull again, then enter via Home for the launch.
    pointer 362 118 360
    wait_for "Drawer opened by command has the same dismissal path" in_scene home
    sleep 0.6
    pointer 362 355 118
    wait_for "Drawer reopened via the Home content gesture" in_scene drawer
    sleep 0.6
    tablet_expect_windows "$windows"
    pointer 362 118
    wait_for "Drawer launched its own fixture" json_matches activewindow '.class == "veil-sheet"'
    wait_for "Drawer closed after launching the fixture" in_scene pill
    json_matches clients 'map(.class) | sort == ["veil-home", "veil-other", "veil-sheet"]'
    sheet="$(nested_ctl -j activewindow | jq -er .address)"

    test_tablet_adjacent_apps "$other" "$home" "$sheet"
    windows="$(nested_ctl -j clients | jq -c 'map(.address) | sort')"
    tablet_focus_fixture "$home"
    history="$(nested_ctl -j clients | jq -c 'sort_by(.focusHistoryID) | map(.address)')"

    # Repeated waypoints keep the bottom contact held while the geometry
    # changes. Wait for the actual preview, not just for the pointer process.
    pointer 480 600 465 465 465 465 465 465 465 465 465 465 465 465 &
    held_pointer=$!
    PIDS+=("$held_pointer")
    wait_for "Tablet Recents preview active before portrait rotation" in_scene recents
    [[ $(nested_ctl eval 'hl.monitor({ output = "veil-smoke", mode = "960x640@60", position = "3000x0", scale = 1, transform = 1 })') == ok ]]
    kill -0 "$held_pointer"
    VEILSHELL_TEST_POINTER_EXTENT=640x960
    wait_for "Rotation canceled the held Recents preview" in_scene pill
    wait "$held_pointer"
    unset 'PIDS[-1]'
    in_scene pill
    json_matches activewindow '.address == $id' --arg id "$home"
    tablet_expect_windows "$windows"
    json_matches clients 'sort_by(.focusHistoryID) | map(.address) == $history' --argjson history "$history"

    tablet_bottom_up
    wait_for "Portrait bottom flick opened Home from the app" in_scene home
    sleep 0.6
    pointer 242 355 118
    wait_for "Portrait Home content flick opened Drawer" in_scene drawer
    sleep 0.6
    tablet_expect_windows "$windows"
    pointer 242 118
    wait_for "Portrait Drawer resumed the existing fixture address" json_matches activewindow \
        '.address == $id' --arg id "$sheet"
    in_scene pill
    tablet_expect_windows "$windows"
    echo "Tablet bottom Home/Recents, reverse and rotation cancellation, Drawer launch and resume passed."
}
