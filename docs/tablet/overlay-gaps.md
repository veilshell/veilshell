# Overlay gap gestures (opt-in, pill-free, stock Hyprland)

[Tablet index](README.md) · [Navigation](navigation.md) · [Home](home.md) · [Recents](recents.md) · [Shade](shade.md)

The profile uses **stock Hyprland** and needs no plugin. It does not change
gaps, layout, bindings, or monitor settings. Standalone Tablet, Desk Mode,
the greeter and the default overlay are unaffected.

## 1. Use and behavior

```bash
veilshell --policy tablet --overlay-gap-gestures
```

Add `--output NAME` to select the touchscreen on a multi-output system.
When this profile is active, `veilctl status` reports the mounted strip depths
in logical pixels, not whether a given pixel is uncovered. If the strips are
unavailable, `veilctl show home` still opens Home on the connected target
output. The open Home and All Apps panels have explicit **Close** and
**Back** buttons. `veilctl show pill` returns to the desktop without drawing
a pill. The OSK is manual by default in this profile.

- One transparent layer-shell client, layer **BOTTOM** (below windows;
  BACKGROUND would also sit below windows, but BOTTOM is the documented
  slot checked immediately after windows), `exclusive_zone = 0`
  (non-reserving), keyboard interactivity None.
- Two thin strips anchored inside the compositor's current work area.
  The top strip sits just below the bar's reservation. The strip depth is
  capped from the configured outer gap, and Hyprland—not a window geometry
  cache—decides whether each point is actually uncovered.
  [Layer shell's zero exclusive zone](https://github.com/swaywm/wlr-protocols/blob/d1598e82240d6e8ca57729495a94d4e11d222033/unstable/wlr-layer-shell-unstable-v1.xml#L168-L180)
  requests placement that respects the bar without reserving more space.
- Input regions limited to the strips. Recognition consumes **`wl_touch`
  only**. Pointer/trackpad motion never triggers a gesture; the shared
  Wayland input region still physically absorbs a pointer that lands in an
  otherwise empty strip rect (see §6), so the honest claim is
  *no pointer-triggered action*, not *no pointer interception*.
- Reuse the existing standalone recognizer
  (`crates/veilshell-wayland/src/tablet_nav.rs`, `BottomContact`) and the
  `DragVelocity` estimator (`motion.rs`). Travel is measured against the
  full logical output height, not the strip's height.

Only finger contacts start these gestures. Taps in the strips do nothing.

| Entry | Motion | Result |
|---|---|---|
| Bottom gap strip | fast up flick (`vy < −1100` px/s, Denial value) | Home |
| Bottom gap strip | slow deliberate pull past arm, release while armed | Recents |
| Bottom gap strip | sideways past 82 px or 720 px/s flick with axis lock | right focuses the next older running window; left focuses the next newer one, on this output's active normal workspace; no wrap |
| Home-content area | upward pull meeting Drawer thresholds | All Apps (Drawer) |
| Top gap strip (below bar) | downward pull meeting Shade thresholds | read-only Shade; split in landscape, paged in portrait |

Authority ordering beats polling: the strips sit **behind** windows, so
the compositor's own hit test decides whether a gap is actually uncovered.
VeilShell reads native settings only to size and disarm the strips (see §7),
not to second-guess where a window can accept input.

## 2. Denial comparison (behavior reference, not a gap implementation)

Denial mobile owns its edges in its own compositor; there is **no
gap-strip mode** at the pin. Values below are the contract the overlay
recognizer reuses, not geometry Denial reserves for us.

- Hit box 200×60 with −8 px bottom inset, i.e. visible top at
  `height − 52`; 180×5 pill is feedback only. Status row 48 px tall;
  side bands 32 px wide from below the status row to `height − 52`.
  [metrics](https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_metrics.dart#L6-L55)
- Bottom controller: axis lock 14 px / ratio 1.18; arm 6% of viewport
  height, disarm 4.5%; Home flick `vy < −1100` px/s; sideways commit
  `|dx| > 82` px or `|vx| > 720` px/s flick; Recents dismiss `dy > 72`
  px or `vy > 650` px/s.
  [thresholds](https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L264-L276),
  [axis](https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1573-L1595),
  [release](https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/bottom_gesture_handle.dart#L222-L280)
- Row stepping is at most one card per commit with end clamping and a
  5 s frozen switch order; focus goes through the compositor window
  path, never a launcher or workspace switch.
  [release-row](https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L965-L994),
  [order-hold](https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L278-L278)
- Cancel/rotation: pointer-cancel settles the pill and resets without
  focus/launch side effects; arm travel is recomputed from current size,
  so a mid-gesture geometry change must be treated as cancel-to-origin.
  [reset](https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1253-L1290)

Adaptations (not Denial parity): Denial decks phone-profile app windows;
the overlay must scope sideways focus to already-running windows **on the
gesture output and its active normal workspace** (existing window IDs
plus bounded previews, no launcher/workspace dispatch). No launch-flight
tracking, haptics, transient grouping, live transforms, or elastic stretch
in the stock slice (see [navigation](navigation.md)).

## 3. Penumbra comparison (Rust reference, structurally different)

- `NavGestureRecognizer` defaults: 80 px overview travel, 220 ms hold
  with 6 px slop, 650 px/s flick with 24 px minimum, 35 px horizontal
  switch.
  [recognizer](https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-gestures/src/nav.rs#L28-L59)
- Native DRM side bands **intercept on down before the client sees the
  contact**; the top zone also intercepts on down and can open on tap.
  That is the opposite of this proposal (behind windows, client-first
  windows win).
  [native](https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-comp/src/native_input.rs#L566-L682)
- Do not mix Penumbra numbers with Denial's distance-hysteresis arm and
  1100/720 px/s flicks. Keep the Penumbra checkout local; it is not a
  dependency.

## 4. Stock Hyprland trace (v0.56.2, delta v0.57.2)

All links below are to public upstream source at the verified tags.

### 4.1 Touch routes through the same hit order as the pointer

`CInputManager::onTouchDown` records the touch, resolves the monitor,
then calls `refocus(TOUCH_COORDS)` — the shared pointer hit-test path —
and routes the contact to whichever of `m_foundWindowToFocus` /
`m_foundSurfaceToFocus` / `m_foundLSToFocus` that pass produced. If none
is found it returns without delivering the touch.

- v0.56.2 `src/managers/input/Touch.cpp`: `onTouchDown` → `refocus`
  → `m_found*ToFocus` routing; motion/up are delivered to the retained
  `touchFocus*` (implicit grab: later motion goes to the down surface,
  not re-hit-tested, except DnD repick).
  [Touch.cpp](https://raw.githubusercontent.com/hyprwm/Hyprland/v0.56.2/src/managers/input/Touch.cpp)
- v0.57.2 adds early `consumedTouches` suppression and a null-monitor
  guard around the same flow; the refocus-then-route structure is
  unchanged.
  [Touch.cpp](https://raw.githubusercontent.com/hyprwm/Hyprland/v0.57.2/src/managers/input/Touch.cpp)
- The refocus path (`src/managers/input/InputManager.cpp`,
  `mouseMoveUnified`) tests, in order: exclusive layer surfaces →
  layer popups → OVERLAY → TOP → fullscreen-window rules → windows →
  **BOTTOM** → **BACKGROUND**. A BOTTOM/BACKGROUND strip therefore only
  receives a down when no window (and no higher layer) claims the point.
  This is the authoritative below-window hit order the design relies on.
  [InputManager.cpp](https://raw.githubusercontent.com/hyprwm/Hyprland/v0.56.2/src/managers/input/InputManager.cpp)

Consequence: **delivery needs no window-rectangle polling**. If a tiled
window covers the strip rect, the window wins and the strip never sees
the down. If the gap is genuinely uncovered, the strip is the only
candidate left at its layer.

### 4.2 Layer hit testing respects the input region

`layerSurfaceAt` iterates topmost-first within the queried layer and
skips surfaces whose resolved input is empty (`inputIsInfinite` false
and `input` empty): a transparent strip with an empty region does not
capture. Conversely, any rect left *inside* the input region captures
both touch **and** pointer at that point — the region is shared across
seat capabilities.
v0.56.2 `src/desktop/state/ViewHitTester.cpp`, `layerSurfaceAt`.
[ViewHitTester.cpp](https://raw.githubusercontent.com/hyprwm/Hyprland/v0.56.2/src/desktop/state/ViewHitTester.cpp)

[`HandleInputRegion`](../../crates/veilshell-wayland/src/client.rs) starts
empty and adds only handle rectangles. This gates new contacts; implicit
touch grabs can move outside the region after a down.

### 4.3 Gaps are usually *resize* hit area when the option is on

`windowAt` with `INPUT_EXTENTS` expands window boxes by
`BORDER_GRAB_AREA = border_size + extend_border_grab_area` **only when**
`general:resize_on_border` is on; tiled windows near a work-area edge
extend that expansion into the adjacent gap (2 px stick threshold).
v0.56.2 `ViewHitTester.cpp`, `windowAt`.
[ViewHitTester.cpp](https://raw.githubusercontent.com/hyprwm/Hyprland/v0.56.2/src/desktop/state/ViewHitTester.cpp)

Stock defaults (`src/config/values/ConfigValues.cpp` at v0.56.2):

| Key | Default | Source |
|---|---|---|
| `general:gaps_in` | 5 | [ConfigValues.cpp#L169](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/values/ConfigValues.cpp#L169) |
| `general:gaps_out` | 20 | [same#L170](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/values/ConfigValues.cpp#L170) |
| `general:resize_on_border` | false | [same#L181](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/values/ConfigValues.cpp#L181) |
| `general:extend_border_grab_area` | 15 | [same#L182](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/values/ConfigValues.cpp#L182) |
| `gestures:workspace_swipe_touch` | false | [same#L401](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/values/ConfigValues.cpp#L401) |

The pinned Omarchy configuration example has a smaller outer gap:
`gaps_in = 5`, `gaps_out = 10`, `resize_on_border = false` in
`default/hypr/looknfeel.conf`. This is not an assumption about the user's
current effective settings.
[Omarchy looknfeel](https://github.com/basecamp/omarchy/blob/c7b6a7f8c4e977b2c0c5bc096477233fbfd6a989/default/hypr/looknfeel.conf#L6-L22)

Traps this proves:

1. **Never assume a usable gap.** `gaps_out` can be 0 (user config or
   the Super+Shift+Backspace no-gaps toggle), asymmetric per edge
   (`general:gaps_out` is a CSS-gap type: top/right/bottom/left), and
   overridden by workspace rules. Size strips from a bounded read and
   fail closed; never paint a strip over a window or the bar.
2. **With `resize_on_border = true`, don't arm the strips.** The expanded
   hit regions are conditional; layer focus can also prevent native resize
   from handling a click. Do not rely on layer order alone to preserve
   every resize grab or promote a strip to TOP/OVERLAY to win the gap.
   See [native resize dispatch](https://github.com/hyprwm/Hyprland/blob/v0.57.2/src/managers/input/InputManager.cpp#L845-L879).
3. The stock touch edge-swipe (`workspace_swipe_touch`, default off)
   uses `gapsOut + border_size` fractions of the output as its own
   edge bands. The overlay must not enable, emulate, or fight it; keep
   the overlay recognizer on the BELOW-layer strip path only.

### 4.4 Fullscreen / maximized / focus

- Exclusive-fullscreen logic in `mouseMoveUnified` restricts focus to
  the fullscreen window (plus floating-over-fullscreen and special
  workspace cases); BOTTOM strips are unreachable under true
  fullscreen. Maximized-fullscreen routes reserved-area points to
  BOTTOM layers, otherwise to the maximized window.
  [InputManager.cpp](https://raw.githubusercontent.com/hyprwm/Hyprland/v0.56.2/src/managers/input/InputManager.cpp)
- Fullscreen fades non-OVERLAY layers (`LayerSurface.cpp`: alpha 0 for
  layers below OVERLAY while fullscreen).
  [LayerSurface.cpp](https://raw.githubusercontent.com/hyprwm/Hyprland/v0.56.2/src/desktop/view/LayerSurface.cpp)
- Design consequence: under true fullscreen the strips get nothing
  (correct: app keeps all touches, no pull-through on stock). Under
  maximized windows the gap is normally gone, so the strip fails
  closed by hit order even if still mapped.

## 5. Minimal verified architecture

1. **One BELOW-layer client, two slab strips, `exclusive_zone = 0`,
   keyboard None, fully transparent buffers.** Anchor: bottom strip =
   left+bottom+right with a small fixed height; top strip =
   left+top+right with a small fixed height, offset below the bar's
   exclusive reservation (inside the work area, never overlapping the
   bar). The strips span the available work-area width; they add no
   dedicated side targets.
2. **Input region = strip rects only** (pattern already in
   `HandleInputRegion`). Everything else on the surfaces stays empty
   so it never captures.
3. **Delivery gating = compositor hit order** (§4.1). No polling of
   window rectangles on the hot path.
4. **Sizing = bounded, opt-in, active-only read** (§7): clamp each strip
   to the configured depth for that edge, with a 12 logical-pixel cap.
   A zero configured gap disables that edge; incompatible settings,
   absent touch capability, or unknown values disable both strips.
5. **Recognition = existing `BottomContact` + `DragVelocity`** with
   Denial thresholds from §2 (axis 14 px / 1.18; arm 6% / disarm 4.5%
   of current output height; flicks 1100 up / 720 sideways / 650 down
   dismiss; commits 82 sideways / 72 downward). New code is only the
   gap-strip skin: map a bottom strip down to the app origin or a top
   strip down to Shade (see [Shade](shade.md)). While a pull previews a
   panel, the original strip keeps the implicit touch grab and the preview
   is not clickable. The recognizer tracks one captured touch ID; other
   fingers cannot complete that gesture.
6. **Commit on release**. Reversing a pull can take it below its commit
   threshold. `wl_touch.cancel`, output geometry changes, and lost touch
   capability abort motion without navigation. Losing the selected output
   removes the profile's surfaces; it doesn't move them to another monitor.
   Window focus is revalidated against the active workspace before dispatch.
   Stock Hyprland does not make the IPC snapshot and focus request atomic.
7. **No compositor-policy writes**: no workspace commands, config writes,
   layout or binding changes. An app tile still explicitly launches its app;
   Recents can focus or close one of its presented windows. Shade reports
   notification history as unavailable, has no mutating controls, and does
   not claim host notifications, Power, or Settings.

Why BOTTOM and not TOP: TOP/OVERLAY would sit above windows and consume
their input. BOTTOM keeps windows and the bar in front. The separate
resize-policy check keeps the profile out of native border resizing.

## 6. Input-separation honesty (must not over-claim)

- **Recognition separation is real:** feed the recognizer exclusively
  from `wl_touch` down/motion/up. Pointer, trackpad-gesture
  (Omarchy 3-finger binds), and keyboard events are never inputs to
  `BottomContact`. State this in tests by injecting synthetic pointer
  motion over an armed strip and asserting no action.
- **Region interception is shared:** Wayland input regions are not
  per-seat-capability. A pointer click/hover whose point falls inside
  an armed strip rect *will* land on the strip surface while the
  strip is mapped with a non-empty region there. Do **not** claim
  "the pointer never touches the strip". Claim instead: pointer
   contacts produce **no VeilShell gesture or focus command** (tap on a
   strip is a no-op; pointer is not routed through the strip). Keep
   strips as small as the gap allows to minimize the
  absorption footprint.
- Trackpad multi-finger gestures remain compositor bindings. The overlay
  does not register or change those bindings.

## 7. Native-policy checks and refresh

VeilShell reads `general:gaps_out`, `general:resize_on_border`,
`gestures:workspace_swipe_touch`, and `input:follow_mouse_shrink` in one
read-only batch with a 100 ms deadline. The top and bottom gaps are parsed
independently. Unreadable options, border resize enabled, touchscreen
workspace swipes enabled, or a nonzero hitbox shrink disable both strips.
No touch capability means no strips and no option queries. Sampling is
rate-limited to once every two seconds while the strips are needed; it
pauses under a settled foreground panel. Live option changes are picked
up on refresh, not atomically with Hyprland's config change. If changing
input policy during use, hide the overlay first with `veilctl mode laptop`.

## 8. Geometry traps (verified)

1. **Resize grab can occupy the gap** when `resize_on_border = true`.
   Disable both strips while that mode is on; test an actual mouse
   resize, not just an unchanged shell scene.
2. **Asymmetric / zero gaps.** `gaps_out` is per-edge; only the edge
   under each strip matters. Zero on that edge → strip off.
3. **Bar reservation.** The top strip must start below the bar's
   exclusive zone (read from the work area, not a hardcoded Y).
   Never overlap the bar; never reserve its own zone.
4. **Floating windows overlap the strip rect freely.** They win by hit
   order (floating checked before tiled, both before BOTTOM). No
   client-side floating tracking required for correctness of
   delivery; only the sideways-focus candidate list needs
   output-filtered existing windows.
5. **Fullscreen/maximized.** True fullscreen: no strip delivery by
   construction. Maximized: gap normally gone; reserved-area carve-out
   may still route to BOTTOM — the recognizer must still require full
   arm/flick thresholds, and release must no-op when the only
   candidate is the maximized window itself.
6. **Fractional scale and both orientations.** Thresholds are in
   logical pixels against current output height (Denial: recomputed
   per build). Recompute per configure; mid-gesture geometry change =
   cancel-to-origin.
7. **Multi-output.** One strip set on the selected output. On other
   monitors there is no gap surface. Sideways focus is scoped to that
   output and its active normal workspace.

## 9. Acceptance cases

Conventions from the [Tablet index](README.md): label each run as
source-traced, unit, virtual-pointer, SDDM guest touch, or physical
device. Never capture passwords; keep logs out of the repo.

- **T1 widths:** 1280×800 landscape and 800×1280 portrait (then the
  target tablet scale): slow pull arms Recents; fast flick goes Home;
  reverse below disarm cancels; tap does nothing.
- **T2 top reservation:** with a reserved TOP bar, top-strip
  pull opens Shade; a pull starting on the bar stays with the bar;
  strip never overlaps bar pixels (screenshot + input-region dump).
- **T3 floating overlap:** floating window over the bottom strip —
  touches on it stay in the app; uncovered remainder of the strip
  still fires.
- **T4 resize:** `resize_on_border` on/off × gaps 0/10/20: with the
  option on, gap drag resizes the window (strip gets nothing);
  with it off and a real gap, the strip fires. No regression in
  native resize in any case.
- **T5 collapse:** `gaps_out = 0` removes the configured strips. True
  fullscreen keeps contacts on the window even if the strips are still
  configured. Check the real hit target under maximization, separately
  from true fullscreen.
- **T6 cancel:** mid-pull `wl_touch.cancel`, device hot-unplug,
  output reconfigure, daemon shutdown: origin restored, no focus
  change, no launch, no workspace/layout/host action.
- **T7 correctness:** sideways commits focus exactly one adjacent
  existing window on the same output, clamp at ends, honor the 5 s
  order hold; Home-content pull opens All Apps without launching;
  Shade pull/dismiss round-trips to the originating app.
- **T8 pointer honesty:** pointer click/drag over a mapped strip produces
  no VeilShell navigation (allowed: pointer absorption by the region).
  Check positive bar/app clicks. Trackpad binds remain with the compositor;
  the profile doesn't grab them.
- **T9 unit:** `BottomContact` arm/disarm/flick/commit matrix at
  800/1280 heights (already in `tablet_nav.rs` tests); add gap-origin
  mapping and cancellation coverage without mixing the evidence with
  physical-device results.

## 10. Cross-cutting scope (constraints, not features)

- Share the existing Rust Tablet recognizer; only skin/policy is new.
  No standalone-compositor settings, no Desk-mode changes, no
  power/session handling, no automatic rotations inside this opt-in.
- Sizing reads (if any) follow §7. Everything else about the session
  (workspaces, layout, binds, gaps, bar, greeter, Desk) is untouched.
- Unknowns stay unknown: exact uncovered-gap pixel maps on arbitrary
  user themes, Omarchy toggle overrides (no-gaps flag loads after
  personal files), and physical-finger feel are runtime evidence, not
  source claims. Record build, options, output, and scale with every
  runtime note.

## Validation and replay

The private nested replay checks the no-touch case with a virtual pointer:

```bash
VEILSHELL_TEST_OVERLAY_GAPS=1 bash scripts/test-hyprland-actions.sh
```

Run it with an isolated Wayland parent. The included `hyprland_gap_test_bar`
example is a reserved 32 px TOP bar with explicit input-delivery witnesses;
it does not use the ambient Wayland display. The replay checks actual bar
and app button delivery and compares the native settings, bindings, work
area, and layer inventory. It also checks that a visible opt-in daemon
never requests a layer surface without `wl_touch`. It is not positive
evidence about touch delivery.

Recorded on Hyprland `0.56.2`: no-touch replay passed, as did the separate
default overlay, standalone Tablet/Desk, and manual app-keyboard replays.
With two nested outputs, removing the selected output closed Home without
moving it to the other monitor. Recreating the same output allowed Home
to open there again.
Formatting, strict workspace Clippy, and 240 Rust tests passed. The 45
deterministic pre-profile snapshot files were byte-identical; installed-app
snapshots were excluded because they contain live status text.

For native `wl_touch` alongside a separate mouse, see the private
`~/veilshell-sddm-lab.RuxcGi/OVERLAY-GAPS.md` instructions for the named
disposable `vs-sddm-gaps` guest. This uses an authenticated SDDM Wayland
session and a test bar, not the complete Omarchy desktop. The replay
checks the guest identity, unlock state, binaries, and scanout size and
keeps input witnesses, frame hashes, and replay timing outside the
repository. It never takes a greeter or authentication capture.

### Authenticated mixed-input result

Release `14d6f057b08d` (VeilShell SHA-256 prefix) passed on Hyprland
`0.57.2` (`11cbc6c577b8`) at 1× in both `1280×800` and `800×1280`, using
the software renderer. The fixture starts with `dwindle`, outer gaps 10,
inner gaps 5, borders 2 logical pixels, and a 32 px TOP reservation.
The armed strips were 10 px each on BOTTOM, starting at `y=32` and
`y=height−10`. Comparing the armed and hidden desktop frames showed
no pixel changes in either strip.

| Case | Observed in both orientations |
|---|---|
| Touch navigation | Bottom flick → Home; deliberate pull → Recents; upward Home content → All Apps; reverse or output transform during a held pull → desktop without commit |
| Shade | Top-gap pull opened a landscape split or portrait pages; dismiss restored the same focused app. A pull starting on the bar was delivered to the bar instead. |
| Adjacent focus | Actual Hyprland addresses stepped `A → B → C → C → B → A → A`. A fourth window on workspace 9 stayed there and outside the sequence. No change to bindings, window placement, active workspace, or output work area. |
| Real pointer routing | USB tablet `BTN_LEFT` reached the bar and Foot. Dragging across armed strips or Home content did not open a panel. With border resizing enabled and both strips gone, a mouse drag from the old bottom strip depth resized a floating window from `280×210` to `332×160`. |
| Coverage and policy | A floating window won at its covered point; the uncovered remainder still worked. True fullscreen kept the touch. Outer gaps `0 10 30 10` produced `0/12` px strips; zero gaps, native touchscreen workspace swipes, and nonzero hitbox shrink each resulted in no strips. |
| No-gap fallback | `veilctl show home` and `veilctl show drawer` worked with no strips; explicit Back and Close clicks returned to Home and the desktop. |

The clone was shut down. The private record contains 56 touch contacts,
16 mouse cases and 19 authenticated frames across the two runs, including
the exact contact coordinates, timing, and window addresses. This is
not a full Omarchy bar compatibility test or physical validation.
Physical touch, trackpad hardware, fractional-scale gap delivery,
rotation sensors, and touchscreen hot-unplug during a hold remain
unverified.
