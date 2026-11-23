# Touch navigation and gesture ownership

[Tablet index](README.md) · [Home](home.md) · [Recents](recents.md) · [Shade](shade.md)

**Evidence:** source-traced at the [reference pins](README.md).
Coordinates below are logical pixels unless noted; positive Y points down.

## Denial's mobile input contract

| State | Touch starts here | Result |
|---|---|---|
| Home or ordinary foreground app | Anywhere on the top status row, 48 px tall at the compositor. The Flutter bar also accounts for its top safe-area inset. | Downward drag opens Shade; a tap is not an open command. In LTR portrait, the left half selects notifications and the right half controls. Landscape [shows both](shade.md). See [input layout][denial-layout], [status gesture][denial-status], and [page selection][denial-page]. |
| App or Home | Bottom center: hit box 200×60 with a −8 px bottom inset (top is `height − 52`). The visible 180×5 pill is feedback, not the size of the target. | Fast up leaves the foreground app for Home, or opens Recents from Home. Slow pull arms Recents; horizontal drag scrubs adjacent **apps**, not workspaces. See [geometry][denial-metrics] and [bottom recognizer][denial-bottom]. |
| Recents | Same bottom zone | A downward drag over 72 px or a down flick over 650 px/s closes Recents. Content gestures on the cards are separate. |
| Shell or an eligible foreground app | The left or right 32 px, from the bottom of the status row to `height − 52`, limited further by the keyboard top when applicable. | Inward Back. Back targets, in order, are keyboard, Shade, Recents, then the focused app (`KEY_BACK`). See [strips][denial-back-bounds], [target][denial-back-target] and [dispatch][denial-back]. |
| True fullscreen app | Status visuals are hidden. The top row is a compositor edge candidate, not an opaque touch area. | The app keeps ordinary touches, including top-edge taps; only an inward stroke that qualifies is retargeted to the shell so it can continue the Shade pull. |
| Modal/lock | A system modal takes input in its own bounds; the shell owns the surrounding area. When locked the lock UI owns the canvas and keyboard, with a specific calls-window exception. | The underlying app doesn't receive ordinary touches. This is not permission to use the regular shell as a locker. See [modal and lock layout][denial-modal]. |

### Status → Shade

- The status widget passes `deltaY × (488 / output height)` to the controller.
  Thus each fraction of output height dragged produces that fraction of panel
  progress, clamped to `[0,1]`. Grabbing a settling Shade starts from its
  **painted** progress, not the spring's destination.
- On release, settle open at 126 of 488 controller units (about 26% of screen
  height) or at a down flick of at least 520 px/s. An up flick at or below
  −520 px/s overrides that. An unmoved drag that started fully open stays open.
  Pointer cancel on the status widget settles with zero velocity, which isn't
  necessarily the same as forcing it closed. See [controller constants][denial-thresholds]
  and [Shade controller][denial-shade-drag].
- The dismiss area has its own shortcut: upward travel of at least 48 px or
  release velocity at or below −500 px/s closes it. Scrollable children can
  own that gesture instead. Closing Shade doesn't select a different app.
  See [dismiss recognizer][denial-dismiss] and [Shade controller][denial-shade-drag].

### Bottom → Home, Recents, or adjacent app (verified contract)

The shell locks direction after 14 px with a 1.18 axis lock ratio ([axis][denial-axis]).
While an app is still launching, the corresponding up gesture goes Home. The pill
lifts at most 8 px over 280 px of upward travel. Arm Recents after 6% of viewport
height, pulse haptics at the crossing if supported, and disarm below 4.5%.
Release with an up flick faster than 1100 px/s for Home from an app (Recents from
Home); otherwise release while armed for Recents. Cancel settles back. See
[controller thresholds][denial-thresholds], [axis][denial-axis],
[deck grab][denial-deck], [row order][denial-order], and [release][denial-bottom].

There is **no time gate** on the Denial bottom gesture: arming is purely
distance-based with hysteresis (`_recentsArmFraction = 0.06`,
`_recentsDisarmFraction = 0.045`), recomputed from `MediaQuery.sizeOf` on each
build, with a haptic pulse on the arm crossing. Resting at the threshold does
not flicker; reversing back below the disarm travel before release cancels the
arm. A fast up-flick (`velocity.dy < −1100` px/s) from a foreground app calls
`goHome`; the same flick from Home calls `openOverview`. While
`launchTransitionActive`, either a fast flick or an armed pull goes Home and
anything smaller resets. A tap (no travel, no flick) does nothing. See the
[handle constants][denial-handle-const] and [release resolution][denial-resolve].

Cancellation and rotation: `onPanCancel` and pointer-cancel settle the pill and
call `resetGestureDrag`, whose only hold exit (`_endFingerGesture`) settles an
open lift back to 0, a homeward lift back to 1, or a held row back onto the app
it came from — no focus change, no launch, no hidden post-cancel action. Denial
recomputes arm travel from the current size each build; there is no pinned
mid-gesture rotation guarantee at this pin, so VeilShell must treat an output
geometry change mid-gesture as a cancel-to-origin, not a release. See
[reset/end][denial-reset] and [handle release][denial-resolve].

Horizontal: a sideways release is committed only while the deck holds the row
or a short sideways flick qualifies (`|vx| > 720` px/s with the axis ratio met);
commit requires `|dx| > 82` px or the flick. `releaseRow` steps at most one card
(`_takeRowEnds` clamps the row to `start ± 1`), then focuses that card's window
through the compositor (`_platform.focusWindow`), never a launcher entry or a
workspace switch. No wrap: the target clamps to the row ends. Direction (LTR;
RTL mirrors via `_sign`): the deck order runs least→most recent
(`_recencyOrder`), and `rowDelta(dx) = −dx × _sign / stride`, so the focused
index moves **opposite** the finger — a rightward finger drag moves toward
older apps, a leftward drag toward newer ones — while the cards themselves
translate **with** the finger. From Home the row rests one past the newest app
(`rowAt: order.length`, `homeInRow`), so a rightward drag enters the most
recent app. Repeated switches keep a frozen `appSwitchOrder` for 5 s of quiet
(`_appSwitchOrderHold`); a swipe in progress re-arms the timer, and a vertical
lift from an app releases the frozen order first. Horizontal switching is gated
by `canSwitchApps`: false while locked, launching, in Recents, in Shade, or
while the keyboard edge panel drags, and false with fewer than two apps when a
foreground app exists. Note the call chain: the gesture commits through the
deck order (`releaseRow` → `_showRowApp`); the platform `adjacentAppWindow`
index helper is a separate utility, not what the bottom gesture calls. See
[release row][denial-release-row], [row ends][denial-row-ends],
[row focus][denial-row-focus], [order hold][denial-order-hold],
[switch gate][denial-switch-gate], [recency order][denial-recency], and
[row delta][denial-row-delta].

VeilShell adaptations (not Denial parity): Denial's deck orders phone-profile
app windows and never mixes in desk-side desktop windows; VeilShell must scope
adjacent focus to already-running windows **on the gesture output** and must not
focus a window on the wrong output. Denial moves live surfaces; VeilShell
previews are bounded async thumbnails (see [Recents](recents.md)), so the
during-gesture motion is a chrome/placeholder animation and focus commits only
on release. Stock-slice scope is actual output-filtered existing window IDs
plus the bounded preview. The following stay reference/follow-up and are not
required of the stock slice: new launch-flight state (an up gesture during an
untracked launch resolves as an ordinary fast-up), haptics, transient grouping,
live window transforms, the elastic row stretch, and last-card throw motion.
Penumbra differs structurally — 220 ms stationary hold past 80 px for overview,
650 px/s flick with 24 px minimum, 35 px horizontal switch — versus Denial's
distance-hysteresis arm and 1100/720 px/s flicks; do not mix the numeric
defaults. See [recognizer][penumbra-nav] and [shell wiring][penumbra-shell].

#### Standalone Tablet implementation contract

1. The bottom-center 200×52 logical hit zone is swipe-only, as in the visible
   portion of Denial's target. Home content, status row, and app keyboard are
   separate routes. The standalone Tablet contact never falls through to the
   overlay's workspace gesture.
2. Axis lock at 14 px / 1.18; vertical arm 6% / disarm 4.5% of current output
   height, with no hold timer. An up-flick faster than 1100 px/s goes from an
   app to Home, or from Home to Recents. An armed pull opens Recents. A quick
   downward flick from the bottom on Recents returns to Home (see
   [Recents](recents.md)). Cancellation and output changes restore the
   contact's originating scene without dispatching an action.
3. Horizontal commit per the row rule above; focus exactly one adjacent
   already-running window on the gesture output; clamp at ends; hold the
   5 s switch order across rapid swipes; never dispatch launcher or
   workspace commands.

#### Acceptance matrix (bottom navigation)

| Case | Gesture | Expected |
|---|---|---|
| Fast up from app | `vy < −1100` px/s, any arm | Foreground app goes Home; no Recents |
| Slow pull, hold, release | past 6% height, release still armed | Recents opens on the previous app |
| Pull past 6%, reverse below 4.5%, release | armed then disarmed | Settles back; foreground unchanged, no focus/launcher side effect |
| Tap / short drag, slow release | below arm, below flick | Cancel; pill settles, nothing commits |
| Fast up from Home | `vy < −1100` px/s, no foreground | Recents opens (slides in from Home) |
| Up during app launch (follow-up: needs launch-flight tracking) | flick or armed pull | Goes Home (launch flight aborts to icon); without launch tracking, resolves as ordinary fast-up |
| Sideways right past 82 px or 720 px/s flick, target exists (LTR) | rightward horizontal commit | Focus moves to the next-older window on the same output |
| Sideways left past 82 px or 720 px/s flick, target exists | leftward horizontal commit | Focus moves to the next-newer window on the same output |
| Sideways right from Home | rightward commit from Home | Most recent local app focuses; with no candidate, stay on Home |
| Sideways at row end | overpull outward | No wrap and no focus change; elastic stretch is a follow-up |
| Two rapid sideways swipes | within 5 s quiet window | Second swipe walks the deck, not a bounce between two apps |
| Cancel mid-gesture / output rotates mid-gesture | pointer cancel, device removal | Origin restored; no partial Recents, no focus change |
| Keyboard open | bottom-zone up/sideways | Keyboard owns the contact; no Home/Recents/switch |
| True fullscreen app | bottom-zone gesture | App keeps ordinary touches. Retargeting a qualifying stroke needs compositor support and is not implemented. |

### Back → client or shell

The compositor forwards an app contact's down first and associates an edge
candidate with that contact. At ≥12 px inward and at most 1.5× that distance
along the edge, the compositor claims it. At ≥16 px outward, or ≥16 px along
the edge outside that cone, it drops the candidate and the contact stays in
the app. A claim cancels the client touch sequence and reconstructs the shell
contact from its original down position and timestamp. A `wl_touch.cancel` can
retire other contacts of that client; this is not a secondary gesture running
in parallel with app input. See [classifier][denial-edge] and
[retargeting][denial-claim].

Recognition and committing Back are different: the shell indicator arms at
44 px and disarms below 36 px. Release while armed, or fling inward at least
600 px/s with 16 px travel, to act; cancel retracts the indicator without
dispatch. See [Back gesture][denial-indicator]. Desktop multi-finger window
gestures are not a requirement for this tablet slice.

## Penumbra comparison

- **Shell/nested:** `penumbra-shell` wires the dependency-free
  `NavGestureRecognizer` with 80 px overview travel, 220 ms hold, 6 px hold
  slop, 650 px/s upward flick with at least 24 px travel, and 35 px horizontal
  app switch. Its tests cover flicks, holds, taps, and horizontal switches.
  See [recognizer][penumbra-nav] and [shell wiring][penumbra-shell].
- **Native DRM:** `--native` dispatches to a separate compositor input path.
  The 28 px side bands intercept on **down**, before a client sees that
  contact. The top status zone also intercepts on down, splits at half the
  fixed phone width, and can open on a tap. These are not substitutes for
  Denial's client-first Back or swipe-only status row. See [backend selection][penumbra-run],
  [routing][penumbra-native], and [release/cancel][penumbra-release].
- The other `GestureStateMachine` used in the compositor's headless policy
  validation has different thresholds. Do not use those tests as proof of a
  native route or mix the numeric defaults. None of the Penumbra paths have
  been runtime-verified in this audit.

## VeilShell decision and replay

Use the existing standalone 48 px work-area reservation for the ordinary
status row and the corresponding hit zone in Home, without adding a new visual
handle. Tablet now uses the full-height reveal and the source-derived
opening, content-dismiss, and portrait paging thresholds. Dismissal returns
to the originating app or Home page; Wayland cancellation intentionally
restores the starting state rather than applying a release threshold.
See [Shade](shade.md) and the
[nested fixture](../DENIAL_VALIDATION.md#tablet-shade-pages-and-split).
Stock layer-shell cannot reassign a contact that started on an app.
Fullscreen pull-through and side Back still need compositor support.

Replay with a fixture app that logs contacts and shows a visible target.
Start at 1280×800 landscape and 800×1280 portrait, then repeat at the target
tablet scale:

1. In an ordinary app and on Home, pull from Y=24 near the left, middle and
   right of the row. Try a long pull, a tap, a horizontal drag, a short pull,
   and a down-then-reverse. Only the committed pull opens Shade; dismissal
   restores the same app or Home page. Try a pull starting just below the
   status row in the app: it must stay in the app.
2. Remove the touch device, cancel the contact, or rotate the panel mid-pull.
   VeilShell should restore its origin without leaving a
   partial Shade. Record actual Denial cancellation separately from this
   VeilShell test condition.
3. In true fullscreen, tap at Y=24 (app receives it); then deliberately pull
   inward far enough to claim **and** meet the Shade release threshold in
   Denial. VeilShell must leave fullscreen app input alone;
   the re-routing part is not implemented.
4. For a future Back implementation: start at X=8 halfway down an app. A
   stationary touch goes to the app; motion by 3 inward and 16 along stays in
   the app; a deliberate inward swipe cancels the app sequence and commits Back
   only after the shell's own threshold. Repeat with a second contact.
5. For bottom navigation, start with three apps. Check a quick upward flick,
   an armed pull that reverses below disarm, and repeated right/left swipes.
   Observe Home, a cancelled Recents preview, and the exact focused window
   addresses, respectively. The [replay record](../DENIAL_VALIDATION.md#tablet-bottom-navigation-and-all-apps)
   distinguishes virtual-pointer evidence from SDDM guest touch.

[denial-handle-const]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/bottom_gesture_handle.dart#L33-L45
[denial-resolve]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/bottom_gesture_handle.dart#L222-L280
[denial-reset]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1253-L1290
[denial-release-row]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L965-L994
[denial-row-ends]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1063-L1070
[denial-row-focus]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1119-L1150
[denial-order-hold]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L278-L278
[denial-switch-gate]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_state.dart#L352-L365
[denial-recency]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L102-L113
[denial-row-delta]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/app_deck.dart#L632-L638

[denial-layout]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_input_layout_coordinator.dart#L148-L221
[denial-status]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/status_bar.dart#L146-L190
[denial-page]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/system_shade_layer.dart#L113-L122
[denial-metrics]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_metrics.dart#L6-L55
[denial-bottom]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/bottom_gesture_handle.dart#L93-L267
[denial-back-bounds]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_metrics.dart#L132-L159
[denial-back-target]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_state.dart#L274-L290
[denial-back]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L820-L844
[denial-modal]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_input_layout_coordinator.dart#L62-L135
[denial-thresholds]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L264-L276
[denial-shade-drag]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1320-L1421
[denial-dismiss]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/shade_dismiss_gesture.dart#L31-L70
[denial-axis]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1573-L1595
[denial-deck]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L996-L1033
[denial-order]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1189-L1221
[denial-edge]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/compositor/src/bin/deniald/wayland_frontend/input/edge_swipe.rs#L13-L74
[denial-claim]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/compositor/src/bin/deniald/wayland_frontend/input/flutter_route.rs#L57-L109
[denial-indicator]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/back_gesture_layer.dart#L185-L233
[penumbra-nav]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-gestures/src/nav.rs#L28-L59
[penumbra-shell]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-shell/src/main.rs#L804-L925
[penumbra-run]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-comp/src/server.rs#L2397-L2437
[penumbra-native]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-comp/src/native_input.rs#L566-L682
[penumbra-release]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-comp/src/native_input.rs#L1086-L1179
