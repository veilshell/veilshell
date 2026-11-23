# Recents and adjacent-app motion

[Tablet index](README.md) · [Navigation](navigation.md) · [Home](home.md) · [Shade](shade.md)

**Evidence:** source-traced at the [reference pins](README.md). The
[navigation page](navigation.md) owns entry and gesture arbitration; this
page starts when the app row or Recents deck takes over the windows.

## Denial's deck

Denial uses **one motion model and real window surfaces** for the foreground
app, horizontal app switching, and Recents. Its four motion axes represent
opening from fullscreen to card size (`open`), row position (`row`),
travel to and from Home (`homeward`), and how non-center apps join the row
(`attached`). Taking a new drag starts at the interpolated state, not
at a separately painted screenshot. The normal app stage remains laid out
but stops painting while the deck presents the live surfaces. See [deck][deck],
[poses][pose], and [window stages][window-layers].

### Geometry and identity

| Part | Denial behavior |
|---|---|
| Preview fit | At most `0.63 × (width − horizontal safe area)` wide and `min(height − vertical safe area, 0.63 × height)` high; preserve the clamped viewport aspect. Center the fitted preview. |
| Portrait stride | `card width + 16` logical px at full Recents. |
| Landscape | Neighbor cards tuck under 25%, with exponential spread `0.35` and up to `0.08` reduction in scale. The side toward newer windows spreads; the other bunches. |
| App switch stride | In the fullscreen row, travel by one output width plus an 18 px gutter. The stride interpolates into the open Recents geometry. |
| Label | App icon and name above the card; 48 px icon/row, 12 px gap, 17 px regular, single-line ellipsis. Prefer resolved app identity, then the window title. Labels fade in late in the opening motion. |
| Task identity | A transient (e.g. a file chooser) riding on an open app is drawn over that app, not listed as its own card. Focus the top riding window when present. Launch history is not deck order. |

See [fit constants][fit], [pose and stride][pose], [labels][labels], and
[transient grouping][identity]. In the desk-beside-phone configuration,
desktop windows do not join the phone deck; the card label can offer a
separate move-to-monitor action. That output policy is out of scope.

### Interaction states

| Trigger | Motion, completion, and non-completion |
|---|---|
| Enter with zero open windows | Show the `No windows` state over the scrim. Tapping the backdrop closes Recents. |
| Enter with one window | One centered card, no next-card action. Tap to settle back to that existing app. |
| Horizontal drag with multiple windows | Drag the row; a flick over the 500 px/s threshold projects ~0.35 s of travel, then settles to a card (glide 0.3–1.1 s). Pull beyond the ends against 0.55 resistance and spring back. This is not the one-app-per-flick bottom gesture. |
| Tap a window | Resolve the card and its focus target, settle row to it, then close toward its full app pose. Invalid/non-user windows are ignored. |
| Drag a card vertically | Upward throw follows the contact. Downward drag stretches at most 25 px. Release past half its upward exit distance or fling up faster than 1000 px/s to throw; a strong down fling overrides the distance. A cancelled or insufficient throw settles back. |
| Throw completes | `leaveRecents` removes the slot from the scrolling row while the card is still departing; neighbors fill the gap. The card shrinks by up to 10%. `dismissFromRecents` dispatches close after the card is past the exit margin, including its riding windows before the owner. If no cards remain, Recents closes toward Home. |
| Window closes externally during motion | Reconcile the compositor snapshot, drop the departed ID and settle the row around the nearest surviving anchor. A focus event during the departure should not abruptly end Recents. |

See [card, row, and empty states][cards], [throw][throw], [controller actions][actions],
and [window reconciliation][snapshots]. Declining a close request is different
from an insufficient throw: a disappearing card alone does not prove the app
accepted close. Client refusal or delayed shutdown needs its own runtime
fixture rather than an assumption about Denial's response.

## Penumbra comparison

- Penumbra's DRM compositor also composes live window surfaces, so it is a
  useful Rust reference for keeping previews in the compositor and drawing
  shell chrome separately. Its phone canvas is 412×915, with a fixed
  276×563 card at Y=176 and 298 px stride. Mapping the canvas onto a larger
  panel is **not** adaptive landscape layout. See [phone constants][phone]
  and [native composition][live].
- Its native card gesture is tuned for that canvas: close at lift past
  140 px, or past 70 px with `vy < −800` px/s and `|vy| > 2.2 × |vx|`.
  Its task protocol publishes stable IDs in recency order and sends focus
  or close by ID. The Slint shell also has its own overlay with empty state
  and Clear All; don't infer the native result from a nested preview or a
  gesture-crate default. See [native release][pen-throw] and [task wire][phone].

## Verified adjacent-focus and overview-close contract

Horizontal bottom-gesture motion is owned by [Navigation](navigation.md); this
page owns what it commits to. On release the row settles to at most one
neighbor (`_takeRowEnds` clamps to `start ± 1`) and `_showRowApp` focuses that
card's window **through the compositor** (`_platform.focusWindow` on the
card's focus target — the newest riding window, or the app itself), recording
the frozen `appSwitchOrder` so rapid swipes walk the deck. That is a window
focus by stable ID, never a launcher entry, workspace switch, or desk-monitor
move; the desk-beside-phone configuration keeps desktop windows out of the
phone deck entirely. No wrap: out-of-range targets clamp to the row ends.
Direction (LTR; RTL mirrors via the deck `_sign`): the row order runs
least→most recent and `rowDelta` negates finger travel, so the focused card
moves opposite the finger while cards translate with it — rightward drag
toward older apps, leftward toward newer ones; from Home (row resting one
past the newest app) a rightward drag enters the most recent app. The frozen
order lasts 5 s of quiet (`_appSwitchOrderHold`); an in-progress swipe
re-arms the timer. Switching is refused unless `canSwitchApps` (locked,
launching, in Recents/Shade, keyboard panel dragging, or fewer than two
apps). See [release row][row-release], [row ends][row-ends],
[row focus][row-focus], [order hold][order-hold], [switch gate][switch-gate],
[recency order][denial-recency], [row delta][denial-row-delta], and
[transient grouping][identity] (reference only — see scope below).

VeilShell adaptation (bounded substitute, not parity): Denial presents live
window surfaces in the deck; VeilShell cannot do compositor-side live
transforms from stock layer-shell capture. The approved substitute is bounded
async thumbnails — one-shot captures for at most three nearby cards, size
capped (`MAX_RECENT_THUMBNAILS = 3`, area and edge caps in the local capture
surface) — with the deck chrome drawn separately and focus committed by window
ID on release. Do not claim a live app-to-card morph from static frames.
Adjacent focus must resolve to an already-running window **on the gesture
output**; a target on the wrong output, a stale ID, or an empty deck resolves
to nothing (settle back / show the `No windows` state), never to a launcher
dispatch or a workspace switch.

Stock-slice scope is actual output-filtered existing window IDs plus the
bounded preview. Reference/follow-up, not required of the stock slice:
transient-under-owner grouping (stock slice lists top-level windows as its
own cards), live window transforms, the elastic row stretch, haptics, and
last-card throw motion. The matrix tags follow-up rows explicitly.

Overview-down return: a downward drag over 72 px or a down flick over 650 px/s
on the bottom zone closes Recents **toward Home** — `closeOverview` clears the
foreground object ID while the cards slide out beside the screen. Card-level
downward motion is separate: at most 25 px of rubber-band stretch, and release
direction decides before distance (a strong down fling returns the card even
past the exit threshold). See [bottom release][nav-resolve] and
[close overview][close-overview].

#### Standalone Tablet implementation contract

1. Use existing mapped, non-scratchpad top-level windows from the gesture
   output; never use launch history. Recents uses the existing card layout
   and asynchronously captures up to three nearby thumbnails with pixel
   caps. The switch order is held independently of the visual Recents deck.
2. Bottom-zone horizontal commit: settle at most one neighbor, focus its live
   window by ID on the gesture output only; clamp at ends (no elastic curve
   required). VeilShell holds the order for five seconds after a successful
   switch. Changed membership, focus or recency at the next snapshot
   rebuilds it. Hyprland can follow an addressed window to its workspace,
   but VeilShell sends no workspace command. The pre-dispatch check and
   focus request aren't atomic; the shell confirms focus against the next
   output-local snapshot. Cancel and rotation dispatch neither focus nor
   close.
3. A bottom-down dismissal of settled Recents goes to Home without closing
   or changing focus to a window. In contrast, aborting an *opening* pull
   returns to its own origin. Card throws and how the deck tracks external
   window changes are separate behavior.

#### Acceptance matrix (adjacent focus and overview close)

| Case | Gesture / event | Expected |
|---|---|---|
| Sideways commit, target exists | `|dx| > 82` px or `|vx| > 720` px/s | Exactly one adjacent window on the same output focuses |
| Sideways at newest/oldest end | overpull | Focus unchanged; no wrap |
| Target on wrong output / stale ID | sideways commit | Nothing focuses; row settles back |
| Rapid swipes within 5 s | two/three horizontal commits | Deck walks; order frozen; third swipe still advances |
| 5 s quiet, then swipe | pause, then horizontal commit | Order rebuilt from recency; swipe behaves as first in sequence |
| Keyboard panel dragging / Shade open / locked | horizontal bottom gesture | Refused; no focus change |
| Overview bottom-zone `vy > 650` px/s | downward flick | Recents returns to Home; existing windows stay running |
| Overview bottom-zone short down drag, slow release | below both thresholds | Settles back in Recents; foreground unchanged |
| Output rotates during opening / contact cancelled | geometry change, pointer cancel | Return to originating scene (app or Home); no focus or close dispatched |

[row-release]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L965-L994
[denial-recency]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L102-L113
[denial-row-delta]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/app_deck.dart#L632-L638
[row-ends]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1063-L1070
[row-focus]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1119-L1150
[order-hold]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L278-L278
[switch-gate]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_state.dart#L352-L365
[nav-resolve]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/bottom_gesture_handle.dart#L222-L280
[close-overview]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L804-L819

## VeilShell decision and replay

[One-shot captures for up to three nearby cards](../../crates/veilshell-wayland/src/capture.rs)
provide previews, not a live deck or seamless app-to-card morph. For test
fixtures and results see [Tablet bottom navigation and All Apps](../DENIAL_VALIDATION.md#tablet-bottom-navigation-and-all-apps).

1. Open Recents with no apps, one app, and three apps. Check empty/centering/
   neighbors and app labels, then rotate and repeat with the same fixtures.
   A native transient riding on an app (grouped card, transient focus) is
   follow-up per the scope above, not part of this replay.
2. Hold the row halfway between cards, reverse, release; repeat with a flick
   and an overpull at the end. Record starting pose, release pose, settling
   time and final card ID in each trace.
3. Throw slowly below and above half exit travel, then fast at short travel.
   Check that close is dispatched once and only after departure, the correct
   live window closes, the neighbors reflow, and the last card returns Home.
4. While presenting the deck, externally close a different window and try a
   client that refuses or delays close. Compare final compositor IDs, not
   just painted cards. Also switch rapidly between two/three apps from the
   bottom zone, separate from carousel scrolling.

[deck]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/app_deck.dart#L48-L129
[pose]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/app_deck.dart#L563-L690
[window-layers]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/features/mobile/mobile_window_layers.dart
[fit]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/overview/overview_geometry.dart#L7-L61
[labels]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/overview/overview_card_label.dart#L15-L89
[identity]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_state.dart#L293-L350
[cards]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/overview/app_deck_layer.dart#L76-L354
[throw]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/overview/app_deck_layer.dart#L393-L550
[actions]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L845-L959
[snapshots]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L508-L562
[phone]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-comp/src/phone.rs#L3-L43
[live]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-comp/src/native.rs#L585-L656
[pen-throw]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-comp/src/native_input.rs#L879-L926
