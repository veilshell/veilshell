# Shade, notifications and controls

[Tablet index](README.md) · [Navigation](navigation.md) · [Home](home.md) · [Recents](recents.md)

**Evidence:** source-traced at the [reference pins](README.md), with existing
empty/default reference captures. The [navigation page](navigation.md#status--shade)
defines pull ownership, cancel, and settle. This page covers what the pull
reveals, not the desktop Dashboard.

## Denial's layout

- **Portrait:** two full-width pages. Pull from the start half of the status
  row for notifications and the end half for controls (mirrored in RTL).
  Once progress is at least `0.98`, a horizontal drag can move between them.
  Settle to the neighbor at `90 × scale` px of travel or with a fling of
  `250 × scale` px/s and at least `30 × scale` px of travel. The other page
  stays laid out but offstage until dragged in and isn't independently
  tappable. See [page selection][selection] and [pager][pager].
- **Landscape:** show both at once, notifications at the start and controls
  at the end. Each column is `min(610, availableWidth/2)` in the reference
  viewport, **before** output scaling; control tile heights get a factor of
  `1.5`. Notification dismissal remains toward the start edge. See
  [columns][landscape], [tile height][controls], and [history gestures][history-actions].
- **Scale:** `shortSide/(1220/3)` in portrait, capped at `1.25` in
  landscape. Lay out at `output / scale` and scale the resulting geometry up;
  inset, type and glyph sizes compensate so they don't balloon along with
  spacing. At 1280×800 the scale is `1.25`, reference width `1024`, and the
  two `512` reference-width columns paint at `640` px apiece.
  See [reference geometry][scale] and [viewport transform][viewport].

### Header, tiles, and movement

| Element | Denial target |
|---|---|
| Header | The status clock moves and grows from `18.15` to `66` logical px. Its date emerges beneath the time (`26` px, `12` gap); status icons move into the header, and notification history scrolls over it. The foreground is derived from the app/status color when appropriate. |
| Grid | Four columns. Base capsule `62`, gutter `14`, pitch `76` in reference units (and the landscape height factor). If the available grid width is below `290`, reduce those units proportionally; `column=(width−3×gutter)/4`. A wide tile spans `2×column+gutter`; a tall slider is `2×capsule+gutter`. |
| Slot order | Wi-Fi and Bluetooth wide at top; brightness and volume vertically below the first two columns; Mobile data occupies the right two columns below Bluetooth. Sound/Silent and optional power profile follow; supported rotation and other options use footer slots. |
| Reveal | Blur fraction saturates at one third of travel; content translation saturates at `0.86`. Grid water starts at `0.33 + 0.05×row + 0.01×column`. It drains in the opposite order on close. Header Settings and Power actions also reveal with progress. |
| Levels | Vertical relative drag with 4 px slop; a tap selects the absolute level; scroll/keys move by 0.05. Clamp the level at the ends but stretch the surface with up to the 12 px rubber reach and spring back. Service updates shouldn't pull the level away while held. |

See [status clock][status], [clock metrics][scale], [grid and slots][tiles],
[expansion motion][motion], [water][water], and [level physics][level].

### Backdrop and service states

- A black 20%-alpha scrim follows the blur fraction. With a backdrop cache,
  the frosted wallpaper fades in and notification glass uses that same
  dimmed frost. Otherwise live blur can share a screen-space filter clipped
  to a union of relevant surfaces, with a separate early-out when effects
  are disabled. The opaque frost can hide the scene beneath it once the
  blur fraction is full. Lit tiles don't need another blur sample. Compare
  motion against a window as well as against wallpaper. See [material branches][material],
  [shared filtering][filter], and [frost coverage][covers].
- Wi-Fi/Bluetooth body buttons toggle through the connectivity controllers;
  the details chevrons open their respective lists without toggling a radio.
  Unavailable hardware/service, permission denial and busy state affect
  actionability; radio transitions show a busy indicator. DND isn't
  actionable until settings are loaded. Missing power profiles shift
  Silent into the spare slot. Rotation and recording are capability-gated;
  desktop-view requires a single output. See [control wiring][controls].
- Brightness targets the active output (`0.72` fallback before a level is
  available); volume uses the shared volume state (`0.46` initial fallback)
  and suppresses the separate volume HUD during adjustment. Settings closes
  Shade before opening the app; Power opens its own surface. Escape dismisses.
  See [level and action wiring][level-actions] and [volume controller][volume].
- History is scrollable when it overflows; it groups by app and can clear
  eligible notifications. Opening the Shade suppresses banners and a fully
  open Shade marks the history read. Tap a notification to open it, use its
  action buttons to dispatch the selected action, and swipe toward the start
  to dismiss where allowed. Locked previews have their own privacy policy.
  An empty history shows a centered bell and “No notifications.”
  See [history][history], [notification actions][history-actions], and
  [empty state][empty]. No consolidated in-shade media card is traced in
  this Denial revision; don't require the desktop or lock media UI here.

## Penumbra comparison

Penumbra's Slint control center and notifications are two sliding views on
a 412×915 phone canvas; there is no corresponding landscape split in the
audited path. Its MPRIS card and two-column quick tiles are not Denial parity
requirements. The useful renderer reference is a compositor-supplied
Dual-Kawase backdrop with a transparent shell panel above it, rather than
independent blur in every widget. Its progression and material values are
not VeilShell defaults. See [window canvas][pen-window], [panel and media][pen-panel],
and [backdrop shader][pen-backdrop].

## VeilShell implementation and replay

Standalone Tablet chooses the initial portrait page by contact-down X:
left for Notifications, right for Controls. An explicit `veilctl show shade`
starts on Controls. In landscape the two panes are shown together; a
settled portrait page is retained across rotation into landscape and back.
Horizontal drags page the main Shade, not a Wi-Fi, Bluetooth, audio, More, or
Power dialog. An upward swipe on the open Shade returns to the originating
app or Home page; a cancelled contact restores its starting state. The
existing Desk and overlay Shade paths are unchanged.

The notification pane models two distinct states. Live sessions say
**Notification history unavailable**: the optional banner daemon doesn't
provide in-Shade history, and this change doesn't take its D-Bus name.
Only a demo snapshot with `--empty-notifications` depicts known-empty
history; it doesn't generate notifications. VeilShell uses a tinted
backdrop, a staged tile fade, and curved slider fills.
Home tiles and its clock fade out as the tint takes over, leaving the
wallpaper and a single Shade clock. VeilShell doesn't sample or refract
the live app beneath Shade. Real history, actions, privacy, and Denial's
full material and liquid motion are separate work.
See [validation and capture notes](../DENIAL_VALIDATION.md#tablet-shade-pages-and-split).

1. In portrait pull from each half of the status area, then drag between
   pages once open. Verify undershoot returns to the same page; horizontal
   motion during a partial reveal doesn't switch pages.
2. In landscape show notifications and controls at the same time. Compare
   column widths, clock/cluster positions, grid alignment, and an empty
   notification pane at matching wallpaper, logical size and scale. Capture
   held drags at 25/50/75% as well as the two endpoints.
3. With fixture radios unavailable, changing, and ready, verify disabled,
   busy and ready states. Toggle Wi-Fi independently from opening details;
   test allowed and denied service replies. Drag across a level tile to
   change pages without writing a level; then tap or drag it vertically and
   verify final read-back. Detail and Power dialogs should not page.
4. Once a history backend is available, send clearable, persistent and
   actionable test notifications. Verify grouping, scroll-versus-dismiss
   ownership, action dispatch and last-row empty state. Check that sensitive
   content is not exposed when locked.
5. Compare cached versus live material with the same wallpaper and window
   fixture, and test reduced effects on the tablet. Separate static blur
   fidelity from compositor support for live refraction.

[selection]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/system_shade_layer.dart#L113-L122
[pager]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/quick_settings_panel.dart#L399-L470
[landscape]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/quick_settings_panel.dart#L472-L548
[scale]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/shade_reference_geometry.dart#L3-L68
[viewport]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/quick_settings_panel.dart#L578-L622
[status]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/status_bar.dart#L23-L136
[tiles]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/quick_settings_tiles.dart#L100-L271
[motion]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/shade_expansion_motion.dart#L8-L42
[water]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/liquid/water_reveal.dart#L19-L98
[level]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/liquid_range_bar.dart#L47-L190
[material]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/quick_settings_panel.dart#L153-L270
[filter]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/shade_backdrop_scene.dart#L90-L133
[covers]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/shade_progress.dart#L18-L35
[controls]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/quick_settings_panel.dart#L730-L864
[level-actions]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/quick_settings_panel.dart#L868-L933
[volume]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/quick_settings.dart#L12-L110
[history]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/mobile_notification_history.dart#L90-L265
[history-actions]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/mobile_notification_history.dart#L372-L583
[empty]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/quick_settings_panel.dart#L935-L964
[pen-window]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-shell/ui/quick_settings_window.slint#L7-L22
[pen-panel]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-shell/ui/quick_settings_slider.slint#L1105-L1157
[pen-backdrop]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-comp/src/render/quicksettings_backdrop.frag#L9-L52
