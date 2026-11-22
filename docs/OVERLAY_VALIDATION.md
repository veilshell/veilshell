# v0.1 Hyprland overlay validation

This is the [first-release](ROADMAP.md#first-release) test procedure for the
standard overlay in an existing Hyprland login. The reference device is a
Microsoft Surface Go 3 with its Type Cover. Omarchy is optional; do not
install its hooks or user service for this run. Do not use the standalone
SDDM entry or test session power actions here. These are expected results,
not a record of a successful tablet run.

## Build, install, and start

Use a source checkout at the revision under test. You need Rust/Cargo
(workspace minimum 1.86), a native linker, and the usual Linux build tools
for your distro. At runtime you need your normal user login in Hyprland with
Wayland layer-shell and working Hyprland IPC. System controls and app
keyboard support have additional [capability requirements](PORTING_A_COMPOSITOR.md#capability-contract).
Use Bash for the examples below.

From the repository root, build both release binaries and install them into a
fresh user-owned directory. If the build or install fails, stop and record
the error; don't test binaries from a previous build. No `sudo`, session
files, or host config changes are needed:

```bash
git rev-parse HEAD
git status --short
rustc --version
cargo --version
(
    set -e
    cargo build --locked --release -p veilshell-shell --bin veilshell --bin veilctl --target-dir target
    prefix="$(mktemp -d "$HOME/veilshell-v0.1.XXXXXX")"
    install -D -m 755 target/release/veilshell "$prefix/bin/veilshell"
    install -D -m 755 target/release/veilctl "$prefix/bin/veilctl"
    printf 'Install directory: %s\n' "$prefix"
)
```

Open two terminals in your **normal Hyprland session**, not a TTY, an SSH
login, or the standalone preview. In both terminals set `prefix` to the
printed path and run:

```bash
export PATH="$prefix/bin:$PATH"
unset VEILSHELL_SESSION_DIR VEILSHELL_SOCKET
printf '%s\n' "$WAYLAND_DISPLAY" "$HYPRLAND_INSTANCE_SIGNATURE" "$XDG_RUNTIME_DIR"
hyprctl version
hyprctl monitors
veilctl status
```

Expect nonempty session variables, a successful `hyprctl` query, a live
Hyprland backend (not Mock), and `daemon: not running` before launch. Use
only one overlay daemon; if one is already running, stop your existing
overlay through its normal launcher first or defer the test. Do not remove
an occupied socket or run two overlays on the same display. `veilctl status`
can print compositor and hardware data without a daemon; that output alone
is not a smoke-test pass.

In terminal 1, leave the daemon in the foreground and keep its stderr:

```bash
veilshell --policy tablet --renderer auto 2>"$prefix/veilshell-tablet.log"
```

If more than one output is connected, add `--output NAME` using the internal
panel name reported by `hyprctl monitors`. In terminal 2, `veilctl status`
should now show a responding daemon with `visible: true`.
Check that the bottom pill and top handle actually appear on the touch panel.
If it fails, save the log. If needed, repeat with `--renderer software` and
record it as a separate run, not a pass for the original configuration.

## Reference tablet: touch flows

Save work first, and keep a physical input/recovery path available. Run the
touch checks in **landscape and portrait**. Set each orientation through the
host's normal display controls, confirm touch mapping and note output
transform/scale before testing, then restore the original orientation when
done. The overlay should follow the host output; it is not responsible for
rotating it. If the host cannot provide usable touch in one orientation,
record that blocker rather than substituting a screenshot.

| Check | Touch steps and expected result |
|---|---|
| Home and app launch | Pull up from the bottom pill. Reverse a short pull to cancel; a full pull should settle on Home. Page sideways if there are multiple pages, then tap an installed `.desktop` app. The actual app opens under the host's normal window policy. Reopen Home and swipe down to return to the pill. Check that taps outside the pill and the inset keyboard button reach the app. |
| Recents list and focus | Open two distinguishable disposable windows with no unsaved work. Open Home, then swipe up from its bottom edge into Recents. Swipe sideways to browse and tap one card. The deck closes and the selected window, not its neighbor, has focus. Record live thumbnails separately from listing and focus: missing capture protocols are not proof that focus is broken, and a drawn card isn't proof that its preview is live. |
| Recents close | Reopen Recents and swipe up on one disposable card. The matching real window closes and disappears from Recents; the other window stays open. Do not swipe away a window with valuable work. |
| Quick Settings gesture | Pull down from the top handle; reverse a short pull to cancel, then fully open the shade. Check the controls fit and can be reached. Swipe up to dismiss. Compare a real control and its restored value using the procedure below. |
| App keyboard (OSK) | Open a native Wayland app with working text-input support (e.g. a GTK text editor running natively, not under XWayland). Tap a blank editable field; the keyboard should open. Type a scratch string using letters, Shift, a digit and a symbol, then Backspace and an arrow. Check the **app's** text and caret, not just highlighted keys. Tap **HIDE** or swipe down, check that the work area is restored and that ordinary updates to the same editor don't reopen it. Tap the bottom-right keyboard icon to reopen without losing app focus. Also check that a fresh editor activation can reopen it automatically. |
| Manual keyboard and live reflow | Start a separate run with `--osk manual`. Focus a terminal, tap the keyboard icon, and enable **TERM**; this mode must not take the seat's input-method slot. Check Escape and Tab on the left and the arrows on the right in landscape, or in the top row on a compact output. Rotate to portrait with the keyboard open: the shortcuts should move to the top, **TERM** should stay on, and the same app should receive typing and shortcuts. Check the new reservation. In Auto posture mode, repeat with the Type Cover and reattach while the OSK is open: it should hide and give the space back without changing the host's monitor rules. |

As a diagnostic only, `veilctl show` with `home`, `recents`, `shade`, `osk`,
or `pill` can separate surface visibility problems from gesture or text-focus
problems. For the OSK, record the tested app, toolkit/backend and any
competing input method. `wayland-info`, if installed, can help check
`zwp_input_method_v2` and `zwp_virtual_keyboard_v1`; advertisement does not
guarantee access or working app text input. A manually shown keyboard is not
a pass for automatic activation or text entry. If another input method
already owns the seat, VeilShell cannot auto-open its own keyboard; the
manual keyboard can still work via the virtual-keyboard protocol. Without a
virtual keyboard or active input method, the keyboard button is hidden and
`veilctl show osk` reports that it is unavailable.

For both edge handles, start a pull slightly inside the display, next to
the visible mark. The mark should brighten and grow as soon as it's touched;
the panel should move before you lift your finger. Try a short pull and
reversal, a quick flick, and a tap directly on the mark. A tap in the extra
swipe padding should not open a panel, and contacts outside the centered
zone and the keyboard button should still reach the underlying app.

### Safe, reversible Quick Settings (QS) check

From the shade, open **Wi-Fi Networks**, **Bluetooth Devices**, and **Audio
Output** where available. Opening a details page must not toggle a radio.
Check that the current device is marked active. If it's safe to do so,
reconnect a paired headset, switch to it, and restore the original output;
do not switch Wi-Fi networks while testing over a remote connection. On
Omarchy, verify that **Open Omarchy Settings** opens the matching panel
without tapping the bar, and that **Do Not Disturb** agrees with the bar.

The shade can change **real host state**. Use a small brightness or volume
change with a known restore path. Don't toggle Wi-Fi, WWAN, Bluetooth, Night
Light, power profiles, or Do Not Disturb for this smoke test. Never depend on
a radio, audio stream, or remote session that the test could disrupt.

1. With one controllable built-in backlight and enough brightness headroom,
   record the default `brightnessctl -m info` output, the device it names,
   and its raw value (`brightnessctl get`). Make sure that device is the
   panel under test. Keep the display comfortably readable.
2. Move the shade's brightness slider a small amount. Check a real change
   with `brightnessctl get` and confirm the shade reflects the new value on
   reopening. Return the slider near the baseline, then restore the recorded
   raw value with `brightnessctl -d DEVICE set RAW_VALUE` if needed. Read
   back the original value before leaving the test.
3. Alternatively, if the normal PipeWire/WirePlumber sink is working,
   record its identity (`wpctl status`) and level/mute state
   (`wpctl get-volume @DEFAULT_AUDIO_SINK@`). With no loud playback and a
   stable default sink, lower the volume a little using the shade. Verify
   with `wpctl get-volume @DEFAULT_AUDIO_SINK@`, restore the recorded level
   with `wpctl set-volume @DEFAULT_AUDIO_SINK@ ORIGINAL_LEVEL` (the numeric
   level from `get-volume`, e.g. `0.35`), and verify level and mute state.
4. If the control or restore tool is missing, access is denied, the target
   changes, or neither test is safe, record the reason. A moving slider
   without device readback is not a control pass. Do not use `sudo` or change
   service configuration to force a result.

## Type Cover and host preservation

1. Stop the first run with `veilctl quit` in terminal 2. Keep the same
   output/orientation and a fixed set of scratch windows. In terminal 1,
   repeat the launch with `--policy auto` and a new log (e.g.
   `2>"$prefix/veilshell-auto.log"`). Leave watchers enabled. With the cover
   attached at startup, `veilctl hw` should identify the
   `microsoft-surface-go-3` profile and report `keyboard: attached`;
   `veilctl status` should show `visible: false` (Laptop Mode).
2. Detach the Type Cover. The overlay checks USB presence about every 250 ms
   after the kernel reports the device; physical transitions can take longer.
   Check that the pill and handle become usable and status shows
   `visible: true` (Tablet Mode).
   Reattach while Home is open: the shell surfaces should disappear, status
   should show `visible: false` (Laptop Mode), and the cover should work
   in a normal app. If the desktop uses another input method, verify it
   works with the overlay hidden. Start a fresh `--policy auto` run with the
   cover detached to check the initial Tablet state. Record each physical
   transition and its latency separately from manual commands. If detection
   fails, save attached/detached `veilctl hw` and, if available, `lsusb`
   output. A manual workaround does not pass the Type Cover check.
3. With fixed cover state, also run `veilctl mode laptop`, `veilctl mode
   tablet`, then `veilctl mode auto`; check visibility after each, including
   that `auto` follows the actual cover state again. Confirm the desktop
   is usable throughout and host settings are unchanged through
   both physical and manual transitions. Compare before, during each settled
   mode on the pill/hidden state, and after quit. Do not rotate the output,
   open an OSK, move windows, change gaps, or switch workspaces during these
   comparisons. The OSK may temporarily reserve space while visible.
4. In settled Laptop Mode, check for continuing shell frame/buffer activity
   (e.g. in a separate run started with `WAYLAND_DEBUG=client`, inspect
   VeilShell surface activity over a timed idle interval). Expect no steady
   hidden redraw loop; background watcher wakeups are a separate measure.
   Record evidence and duration. Don't infer no redraws from a static
   screenshot, or use protocol tracing for performance numbers. Measure
   reference-device performance separately against the
   [budget](PERFORMANCE_BUDGET.md).

For the host comparison, save `hyprctl -j monitors`, `hyprctl -j clients`, and
`hyprctl -j getoption` for `general:layout`, `general:gaps_in`,
`general:gaps_out`, and `decoration:rounding` at each checkpoint. Compare
output identity, mode/refresh, position, scale, transform and reserved area;
compare the scratch windows' workspace, position, size, floating/fullscreen
state and the layout options. Ignore transient focus and animation fields.
Also check that the same host keybindings, panel, and normal services work
after quit and that no host config files were rewritten (compare checksums of
the active Hyprland config and sourced files if collecting evidence). Explicit
app/window actions and the reversible QS change are not mode-switch tests.

Finish with `veilctl quit`, confirm the foreground process exits, and check
`veilctl status` reports no daemon. `veilctl mode tablet` should now fail
without changing the host. Restore the host display and input state and
recheck any QS value changed. To uninstall, remove the two binaries from
`"$prefix/bin"`; keep logs until results are recorded.

## Contributor smoke test (no tablet)

From the repo root in a compatible parent Wayland session with Cargo,
`Hyprland` (with Lua configuration support), `hyprctl`, `grim`, and `jq` on
`PATH`:

```bash
cargo test --locked --workspace
./scripts/test-headless-hyprland.sh /tmp/veilshell-hypr-test
```

With `foot` installed, `./scripts/test-hyprland-actions.sh` also drives the
overlay with a virtual pointer and checks the real windows it opens, focuses,
and closes in a private Hyprland instance.

Run `VEILSHELL_TEST_KEYBOARD=1 ./scripts/test-hyprland-actions.sh` to exercise app
typing, focus, live output rotation, and keyboard dismissal in both session
policies. It checks the bytes delivered to a disposable terminal, including
shortcuts before and after rotation. It does not validate physical touch,
Type Cover events, or SDDM.

The harness uses its own compositor config, IPC and virtual output. It checks
that startup rejects missing or stale Hyprland IPC, that mode changes leave
compositor settings alone, and that live surfaces render at several scales in
both orientations. Save logs and inspect the images; see
[CONTRIBUTING.md](../CONTRIBUTING.md) for renderer options and offscreen
snapshots. A nested pass does **not** validate physical touch, Type Cover,
host hardware controls, in-app text entry, or tablet performance.

### Recorded nested check

Hyprland 0.56.2 on Omarchy 4.5.0 (kernel `7.2.15-1-omarchy`) passed the
startup and mode checks with both software and Intel Vulkan rendering. Both
release binaries also built and installed into a temporary user directory.
Each run captured five scenes in portrait and landscape at 100%, 150%,
and 200%. The software run also traced input-method registration on show
and release on hide. The virtual-pointer smoke test passed: a swipe outside
the pill left the overlay closed, a swipe on the pill opened Home, and the
overlay launched, focused, and closed real Wayland windows. This is
nested evidence, not Surface Go 3 sign-off.

The Cover watcher still polls; the 250 ms interval does not guarantee the
150 ms [mode transition target](PERFORMANCE_BUDGET.md#hard-budgets). Measure
physical transitions separately from a `veilctl mode` command.

## Validation record template

Use `pass`, `fail`, `unavailable` (missing capability/blocked), or `not run`
for each check. Include the observed result and evidence, not just a tick.
Record nested and real-hardware runs separately.

```text
Date / tester:
Commit / dirty changes / install method:
Run type (Surface Go 3 overlay, other hardware overlay, or nested):
Device / CPU / RAM / GPU / Type Cover and profile:
Distro and version / kernel / Hyprland version / Rust version:
Optional integrations and relevant versions/services:
Daemon flags / requested renderer / observed renderer or fallback:
Output / mode / scale / transform per orientation:
Apps and toolkit/backend / protocol and service limitations:

Install, live IPC, visible surfaces, quit:
Home + launch (landscape / portrait):
Recents listing, live previews, focus, close (landscape / portrait):
QS gesture and control (landscape / portrait; before / changed / restored):
OSK auto-show, text, special keys, dismiss (landscape / portrait; app):
Cover startup attached/detached, repeated attach/detach and latency:
Manual modes and host preservation (before / during / after):
Hidden redraw evidence and duration:
Performance (method / samples: hidden CPU and wakeups, active RSS,
  frame p95/p99, cold startup, mode transition):
Nested harness / screenshots / logs (if applicable):
Known limitations, failures, untested items and reproduction steps:
```

Review logs and captures for private window titles, network names, and other
personal data before sharing them.
