# App on-screen keyboard

[Tablet index](README.md) · [Navigation](navigation.md) · [Shade](shade.md)

**Evidence:** source-traced at the [reference pins](README.md). This is the app
keyboard, not credential entry. Lock authentication belongs to the
[dedicated locker](../TOUCH_LOCK.md), never to the app input method.

## Denial's mobile app keyboard

| Trigger | Expected path |
|---|---|
| App editor requests input | The mobile controller reads the current seat/editor snapshot (`active`, `inputPanelVisible`, content hints/purpose, activation serial). It opens for eligible editors, leaves desk editors on the desk, and requires a registered app ID for legacy automatic input. |
| Editor hides or loses focus | A `24 ms` grace period absorbs a field-to-field handoff. If the replacement editor doesn't appear, close. Automatic hides don't send an explicit user dismissal. |
| Manual entry | An upward gesture in the bottom-right `220×28` hit area moves the panel independently of the Home pill. Open at 86 px of controller travel or a `≤ −520` px/s flick. A downward `≥ 520` px/s flick closes. Dragging the keyboard itself has a 36 px slop to avoid turning small key motion into a dismissal. |
| Key action | App text goes through the focused input transaction; named keys use the key path. Ctrl/Alt arm for the next key and then clear, including when used with Space/Enter. Never move editor focus to the keyboard to send a key. |
| Back or explicit user dismiss | The keyboard has first Back priority. A completed user dismissal is sent with the current activation serial so a stale or repeated dismissal cannot alter a different editor. An interrupted drag settles without repeating a key or sending an old dismissal. |

See [automatic entry][auto], [panel gesture][panel], [release][release],
[key bridging][keys], and [compositor text input][broker].

### Geometry and app insets

- Height is `min(0.30 × viewHeight, 338)` portrait or
  `min(0.40 × viewHeight, 338)` landscape, plus a 64 px strip and, if
  enabled, 52 px for portrait shortcuts. The final height is capped at the
  output height. With shortcuts enabled this is 454 px at 800×1280 and
  384 px at 1280×800. Landscape puts Esc/Tab/Ctrl/Alt on the left, arrows
  on the right, and keeps the typing rows at most 60% of the width.
  See [panel metrics][metrics] and [shortcut layout][shortcuts].
- An inset-aware client stays in place and receives the settled keyboard
  height and motion duration, rather than per-frame resizes. Drag height
  is reported at release; rotation updates the new height with zero
  duration. Otherwise the composition path can lift/pan content and clips
  touches at the keyboard top. See [app inset protocol][insets] and
  [input clip/pan][layout]. The custom Denial inset protocol is not a
  capability to assume for an ordinary Hyprland client.

### Key and language states

- Numeric input purpose starts on numeric keys. Plain taps or supported
  rollovers commit normal keys; Shift and Backspace have on-down behavior.
  Holding Backspace or arrows repeats after `400 ms` every `55 ms`.
  A `450 ms` hold can open accent choices where configured.
  The explicit accent table at this pin is Italian, not a complete
  international layout. Remaining shift/rollover behavior and accents
  should be tested with a real editor. See [accent/repeat constants][press]
  and [touch handlers][contacts].
- Suggestions start empty. Eligibility uses content purpose, privacy hints
  and editor capabilities; sensitive, hidden and legacy fields do not get
  the normal prediction flow. The chosen correction can commit on a space;
  Backspace may undo that correction in the same editor. Do not extrapolate
  prediction support to every language. See [predictions and key paths][predict].

## Penumbra comparison

The pinned `penumbra-keyboard` binary uses a 316 px Slint layer with
`KeyboardInteractivity::None` and sends normal keys through a Linux
`uinput` device. Its file-based show/hide state and hardware-keyboard
suppression are architectural comparisons, not authority for Denial's
seat/editor semantics. Do not replace VeilShell's seat-scoped Wayland
input method and virtual keyboard with a global input device. See
[layer and driver][pen-panel] and [physical keyboard handling][pen-hide].

## VeilShell decision and replay

Keep automatic entry, the manual affordance, and Wayland keyboard protocols
independent of credential input. Match fit, focus, and interrupted reflow
before adding predictions. Native Denial inset support, broad accents and
prediction parity are not established by the current OSK renderer.

1. Tap a regular editor, then move to a second field without dismissing.
   Verify no flicker/reopen, that typing remains with the focused field,
   and that leaving the editor restores the usable area.
2. Open the keyboard manually in a terminal, enable shortcuts, rotate, and
   type using held Backspace, arrows and a one-shot modifier. Compare row
   placement, repeat timing and restored focus in both orientations.
3. Drag the panel down and cancel; ensure no extra keystrokes and no
   dismissal. Then commit a dismissal and focus a different field. Verify
   the previous editor's dismissal or pending key can't affect the new one.
4. Compare ordinary, numeric and sensitive input. Verify the layer change
   and privacy gates, plus an eligible correction and undo if predictions
   are being tested. Never record credentials.

[auto]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L373-L419
[panel]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/edge_panel_layer.dart#L347-L419
[release]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L1524-L1550
[keys]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/edge_panel_layer.dart#L631-L717
[broker]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/compositor/src/bin/deniald/wayland_frontend/text_input.rs#L740-L812
[metrics]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_metrics.dart#L69-L98
[shortcuts]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/osk/shell_osk_panel.dart#L940-L1012
[insets]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/docs/MOBILE_APP_INSETS.md#L37-L94
[layout]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_input_layout_coordinator.dart#L137-L208
[press]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/osk/shell_osk_panel.dart#L189-L209
[contacts]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/osk/shell_osk_panel.dart#L580-L735
[predict]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/osk/shell_osk_panel.dart#L369-L549
[pen-panel]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-keyboard/src/main.rs#L98-L145
[pen-hide]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-keyboard/src/main.rs#L249-L351
