# Denial parity validation

The reference is [Denial v0.6.8](https://github.com/denialwm/denial/releases/tag/v0.6.8),
source commit `01b588ec50831bce9e57aed623901f49846ec791`. Keep its checkout
read-only and outside the VeilShell repository. Denial is a test reference,
not a VeilShell runtime or build dependency. Use its signed packages for the
reference guest and confirm that the release notes and tag resolve to the pin.
Record newer builds separately; don't silently move the reference.

## Capture protocol

Use separate disposable guests on the same base image, without mounting the
host home or sharing its clipboard, config, or devices. Record the commit,
renderer, viewport, logical scale, profile, palette, wallpaper, fonts,
icon theme, and test-app versions with each capture. Use one redistributable
wallpaper and deterministic app/notification fixtures in both guests.

Start at landscape `1280×800` and portrait `800×1280` at 100%, then exercise
fractional scaling on the reference tablet. Use Denial's **mobile** shell in
both orientations. Capture the desktop Launcher and Dashboard in a separate
run. Record the completed, cancelled and reversed gestures, not just their
resting states. A virtual multitouch device tests Wayland touch delivery,
not physical touch latency, rotation sensors, or hardware suspend.

| Flow | Evidence to record |
|---|---|
| Home and app drawer | Default page, drawer, drag to rearrange, folder creation, and layout after re-login |
| Quick Settings | Partial reveal and cancel, full reveal, landscape notification split, real notification action, radio setup |
| Recents and display handoff | Open three apps, drag through cards, swipe to close, move a window to an external output, unplug and recover |
| App and lock keyboards | App text entry, accents, locker-owned touch entry, wrong-password denial and successful unlock |
| Lock, sleep and logout | Compositor lock acknowledgment before sleep, visible lock face on wake without a desktop flash, and clean return to SDDM |
| Desktop and Omarchy | Denial desktop search and Dashboard independently; VeilShell logout followed by Omarchy login with existing bindings, theme, and desktop services intact |

Use [the feature inventory](DENIAL_COMPARISON.md) to track each outcome as
unverified, partial, verified, or blocked by a compositor capability. The
[Tablet behavior pages](tablet/README.md) contain source traces and replay
cases for individual features. For visual review use [the source-derived
tokens](DENIAL_PARITY_SPEC.md), matched screenshots, and gesture captures
together. Keep temporary images and logs out of Git unless curated and
license-checked. Never capture credentials.

### Initial Denial guest captures

The signed Arch packages `denial 0.6.8-1` and
`denial-flutter-engine 1:0.6.8-1` booted through SDDM in a separate Arch/KVM
guest with Mesa software rendering and `DENIAL_SHELL_PROFILE=mobile`.
Initial landscape (`1280×800`) and portrait (`800×1280`) captures at scale 1
are grouped in
`../veilshell-lab/denial-0.6.8-captures/` in the development lab. The stock
Home shows a large date/clock widget integrated into the grid. A virtual
touch swipe opens Quick Settings, with an empty notification panel on the
left and controls on the right. The wallpaper and app catalog are still the
guest defaults; these are reference captures, not matched VeilShell diffs.

Denial reports the logical portrait dimensions after applying
`transform=Virtual-1,90`; the raw QEMU scanout was rotated back for viewing.

## Verified VeilShell baseline

At VeilShell `1bdd1df`, the Arch/KVM guest with Hyprland `0.57.2` ran SDDM
login, touch launch and typing in Foot, swipe to Quick Settings, physical
keyboard password unlock, and touch logout with a **guest-only software
renderer override**. The stock renderer panicked in `wgpu-hal` GLES EGL
(`BadDisplay`) during login. With guest polkit installed, suspend and wake
left the compositor locked, but Virtio GPU errors left the screen black.
This is **not** a wake pass or a stock SDDM pass.

Commit `3a6667a` adds surface-aware GPU selection and automatic software
fallback for Wayland GPU presentation failures. With this change, two stock
SDDM logins in the same Virtio guest (no renderer override) reached Home with
`renderer: sdf-software`; both normal exits returned to SDDM with VeilShell
and Hypridle gone. This verifies the startup fix, not physical GPU or wake
behavior. All 105 Rust workspace tests and strict Clippy passed.

The isolated nested Hyprland harness separately exercised GPU and software
rendering at 100%, 150%, and 200% in landscape and portrait. It does not
replace SDDM or hardware tests. Touch unlock, portal/polkit integration,
notifications, and Wi-Fi/Bluetooth setup remain open.

The [locker prototype and validation record](TOUCH_LOCK.md) covers a separate
Wayland probe that remained locked after client death in the guest. The
locker-local input and attempt state are not yet connected to PAM or to the
probe. The launcher can use the
[opt-in XDG autostart helper](../adapters/session/AUTOSTART.md) if installed
and configured; this does not yet provide the rest of the desktop services.
In a third stock SDDM login, an allowlisted Foot fixture opened on VeilShell's
Wayland display and was gone on return to SDDM. The test fixture was removed
afterward. Omarchy coexistence still needs its own login regression test.

## Omarchy-style shortcuts in the SDDM guest

The standalone profile was tested in a disposable clone of the Arch/KVM
guest on Hyprland `0.57.2`, with no Omarchy installed. The updated session
scripts and bindings used the guest's existing VeilShell preview binaries.
SDDM login reached Home with software fallback; `Super+Enter` launched Foot
and a standalone-only `Super+Shift+F` override launched a distinct Foot
fixture. `Super+Ctrl+L` locked; the guest password unlocked; normal daemon
exit removed the shell and returned to SDDM. This does not verify a real
browser, file manager or web app, and does not replace the Omarchy coexistence
test or physical Surface Go 3 validation.

## Tablet status row and Home grid

The [navigation](tablet/navigation.md) and [Home](tablet/home.md) traces have
focused tests for the 48 px status-row bounds, gesture release/cancel, clock
slots and page boundaries. Run
`VEILSHELL_TEST_STANDALONE=1 bash scripts/test-hyprland-actions.sh` for the
live nested fixture. It checks full-width pulls and return to the same app,
no action on a tap, a short/horizontal/reversed pull or a pull from app
content, Home entry/return, portrait geometry, and true fullscreen keeping
ordinary input. The clock is passive, and tapping the moved app tile still
launches or focuses the same window. The default invocation separately
checks the narrow overlay handle.

The harness injects a **virtual pointer**, not hardware touch. It does not
verify `wl_touch.cancel`, screen rotation while a physical contact is held,
fresh SDDM login, or app-owned fullscreen pull-through. The last item
requires compositor-side input and is not implemented. For offscreen
portrait and landscape captures (including 150%), use
`bash scripts/test-snapshots.sh`; the `--tablet-home` snapshots show the
standalone layout. Different fixture icons, fonts and wallpaper make them
layout baselines, not matched Denial comparisons.

## Tablet Shade pages and split

The standalone nested replay passed for the two portrait entry halves
(including a pull crossing the midpoint), page swipes and undershoot,
reversed reveal, app/Home return, and rotation during and after a pull.
Dragging horizontally from a level tile paged without a service write;
dragging vertically from the same tile dispatched a fixture volume change.
Wi-Fi details and Power did not respond to sideways page gestures.
`veilctl status` adds a `tablet shade:` line (`notifications`, `controls`,
`sliding`, or `split`) only in Tablet Shade. This is still a **virtual
pointer** test. The separate overlay and both app-keyboard replays passed.
Workspace formatting, strict Clippy, all 175 Rust tests, and the nested
overlay captures at 100%, 150%, and 200% passed with both renderers. The
offscreen overlay Shade PNGs were byte-identical to the pre-slice fixtures.

```bash
VEILSHELL_TEST_STANDALONE=1 bash scripts/test-hyprland-actions.sh
bash scripts/test-snapshots.sh /tmp/opencode/veilshell-shade-snapshots
```

The `tablet-shade-*` PNGs exercise the closed-to-open reveal at 25%, 50%,
and 75%, both portrait pages, the in-between paging pose, and the split
at 100% and 150%. The demo's `--empty-notifications` represents known-empty
history. Without it, the pane intentionally says history is unavailable,
like live VeilShell. See the [Shade replay cases](tablet/shade.md#veilshell-implementation-and-replay).

### Authenticated VeilShell SDDM guest

The disposable Arch/KVM guest logged in normally through SDDM to the
`VeilShell (Preview)` Wayland session in Tablet mode. Hyprland 0.57.2 used the
software shell renderer. The guest's `Virtual-1` output ran at 1× with
native `1280×800` or `800×1280` scanout. Replay used QMP
`qemu-virtio-multitouch-1` touchscreen events, not a virtual mouse. No
password entry was captured.

- Build `83d7145e2b94` (VeilShell binary SHA-256 prefix) passed the full
  matrix from Home: portrait left/right, horizontal paging, landscape
  split, and upward dismissal. A reversed landscape pull returned to a
  byte-identical Home frame after distinct held preview frames.
- Build `1ecc179306cf` reconfirmed the portrait Notifications → Controls
  sequence after the header text and battery-spacing refinements.
- These captures exposed a faint second clock from Home behind the open
  panel. Build `18c619a1aa42` fades Home content as the tint settles; a
  regression test requires only the Shade header at full tint. The new
  portrait and landscape frames show a single Shade clock at rest. Earlier
  releases remain labeled with their own builds.

### Paired Denial v0.6.8 replay

The busy reference guest was left untouched. A fresh disposable overlay of
the same Arch base booted Denial through the stock SDDM entry. The exact
`denial 0.6.8-1` and `denial-flutter-engine 1:0.6.8-1` packages were verified
against the pinned signing fingerprint and installed only in the clone.
Denial ran its mobile profile in both orientations, with Impeller/GLES on
software Mesa. It did not use the desktop profile.

Both sessions replayed the same QMP touch paths from Home, at native
`800×1280` portrait and `1280×800` landscape, both at 1×. In portrait,
left and right pulls opened Notifications and Controls respectively, a
sideways swipe paged from Notifications to Controls, and an upward swipe
returned Home. In landscape, the pull opened both panes, upward dismissal
returned Home, and reversing a held pull before release returned each
guest to a byte-identical copy of its own starting frame.

Raw frames, hashes, contact timelines, and exact rerun/cleanup commands are
in the private lab directory `~/veilshell-sddm-lab.RuxcGi/` (see
`README.md` and `DENIAL-FRESH.md`). The labeled comparisons are
`paired-portrait-notifications.png`, `paired-portrait-controls.png`,
`paired-portrait-right-controls.png`, `paired-landscape-split.png`, and
`paired-landscape-reverse.png`. The settled pairs use VeilShell build
`18c619a1aa42`; the reverse sequence uses `83d7145e2b94`. Both scratch
VMs were shut down. No authentication frames were recorded.

The guests used the same source wallpaper image and no open app. They did
not match notification or device states: Denial had an actual empty list,
filled level bars and an Auto-rotate tile; VeilShell reported unavailable
history and levels and omitted Auto-rotate. Material, fonts, time, and
app catalog also differed. These are paired gesture and layout captures,
not pixel parity or physical-touch, sensor, or performance tests.

Paths are normalized `(x, y)` of an orientation-correct scanout. The driver
used 16 updates spaced 30 ms apart, pausing to take checkpoints with the
contact held and waiting 950 ms after release:

| Case | Contact path |
|---|---|
| Left pull, portrait or landscape | `(.25, .012) → (.25, .84)` |
| Right pull, portrait | `(.75, .012) → (.75, .84)` |
| Notifications → Controls, portrait | `(.80, .47) → (.16, .47)` |
| Dismiss from content, portrait | `(.48, .73) → (.48, .22)` |
| Dismiss from content, landscape | `(.26, .62) → (.26, .13)` |
| Reverse from Home, landscape | `(.25, .012) → (.25, .40) → (.25, .045)` before release |

Held checkpoints are fractions of the prescribed contact path, not measured
animation progress.

## Tablet bottom navigation and All Apps

The pinned behavior and the stock-Hyprland substitutions are documented in
[Navigation](tablet/navigation.md), [Home](tablet/home.md), and
[Recents](tablet/recents.md). Exercise the isolated nested suite and the
offscreen layout fixtures with:

```bash
VEILSHELL_TEST_STANDALONE=1 bash scripts/test-hyprland-actions.sh
bash scripts/test-snapshots.sh /tmp/opencode/veilshell-navigation-snapshots
```

The nested replay uses a **virtual pointer** and checks actual Hyprland
window addresses and the active workspace, not just scene names. It includes
an app on a second nested output that must not enter the switch sequence.
On Hyprland `0.56.2` the standalone replay passed Home/Recents entry,
reversal and rotation cancellation, Drawer launch/resume, and no-wrap
adjacent focus. The Desk and Shade cases, default overlay replay, and
both app-keyboard cases passed. Nested visual checks passed with software
and wgpu at 100%, 150%, and 200%. The separate legacy Launcher keyboard
smoke timed out at Escape on this headless parent; it also timed out with
the pre-navigation binaries.

The `tablet-drawer-*` and `tablet-recents-*` PNGs cover settled and partially
pulled scenes in portrait and landscape, including 150% examples. These are
offscreen layout fixtures, not captures of compositor window motion.

Workspace formatting, strict Clippy, and all 213 Rust tests passed. Of the
33 pre-navigation snapshot baselines, 18 were byte-identical. All 13 Tablet
Home and Shade diffs were confined to the intentional 5 px Tablet pill.
The two non-demo Home snapshots differed only in the live status text.

### Authenticated SDDM touch replay

The navigation replay uses the private lab's authenticated `VeilShell
(Preview)` session and QMP `qemu-virtio-multitouch-1` input. Only the
disposable VeilShell guest is used in this replay. Its three Foot fixtures
opt out of their input method so the ordinary-app paths do not compete
with the OSK. They start on the same workspace; the test compares the
focused addresses and the complete window set before/after swipes.

Coordinates are in logical pixels, with output height `h`; `x` and `y`
fractions are relative to the calibrated scanout. The main paths are:

| Case | Contact path |
|---|---|
| Fast up from app or Home | center `y=h−22 → h−225`, four updates 12 ms apart |
| Armed pull from app | center `y=h−22 → h−180`, 16 updates 24 ms apart, hold 140 ms before release |
| Reverse before release | same armed pull then return to within 1% of the starting height |
| Home content up | `(.46, .78) → (.46, .24)`; a separate touch on the first tile reuses the running fixture |
| Drawer top down | `(.42, .24) → (.42, .94)` |
| Recents bottom down | center `y=h−50 → h−1`, four updates 10 ms apart |
| Adjacent focus | `x=.50 → .67` (older) or `.50 → .33` (newer), `y=h−22` |

See `~/veilshell-sddm-lab.RuxcGi/TABLET-NAVIGATION.md` for guest setup,
exact commands, and frame/timeline records. No frames are taken at the
greeter or during authentication.

Build `8af453cab2a7` (VeilShell SHA-256 prefix) passed this matrix at 1× in
both `1280×800` landscape and `800×1280` portrait, on Hyprland `0.57.2`
with software rendering. In each run, focus walked all three windows in
order and back without wrapping or changing the workspace. The reversed
pull returned to the same focused window; the frames differed only at the
cursor and a transient Hyprland startup warning. An earlier landscape run
(`ec16bc1c89ed`) returned to a byte-identical starting frame. The 32 final
authenticated frames, hashes, and timed paths are retained privately. The
guest was shut down. This is not a paired Denial comparison, physical
touch latency test, or rotation-sensor test.

## Integration gates

Use the [first release checklist](ROADMAP.md#first-release) for overlay
sign-off. The SDDM, lock, sleep, and fresh-session service checks below apply
to standalone. Performance testing matters for both; record each mode
separately.

- Boot the unmodified optional SDDM entry and return to the greeter on logout
  or startup failure. Test with both accelerated and software-only graphics.
- Keep the lock active through failed authentication, locker failure, and
  output hotplug. Confirm lock before sleep and verify a visible lock frame on
  wake. The preview still requires a physical keyboard for unlock.
- Verify desktop services in a fresh session and then after a return to
  Omarchy. Do not change its kernel, boot, default session, keybindings, or
  shared desktop service configuration to achieve compatibility.
- Run the [performance budget](PERFORMANCE_BUDGET.md) on the Surface Go 3,
  measuring idle wakeups and total session memory in addition to shell RSS,
  first-frame time and frame-time percentiles. Test matching and reduced
  effects separately; guest software performance is not a substitute.
