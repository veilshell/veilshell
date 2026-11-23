# Tablet Home and app launch

[Tablet index](README.md) · [Navigation](navigation.md) · [Recents](recents.md) · [Shade](shade.md)

**Evidence:** source-traced at the [reference pins](README.md), with static
reference captures of the default layout in both orientations. No matched
VeilShell capture yet. This is the mobile launcher, not Denial's desktop
search or Penumbra's phone layout.

## Default Home

- Home is a pager over the existing wallpaper, with a black `0x14000000`
  (~8%) scrim. Wallpaper and icons are separate, and Home's grid stays laid
  out offstage when an app is foregrounded. The visible and interactive
  flags are separate; motion can show Home without arming the grid for taps.
  Visited pages are retained to avoid decoding icons on the first swipe
  frame. See [Home composition][surface], [scrim][scrim], and [pager][pager].
- With no saved layout, the first item is a `2×1` clock/date widget, with
  app icons filling subsequent cells. The clock has no single-tap action.
  It is not a shortcut, app tile, or separate card. Its status section
  collapses when battery text and thermal readings are both absent, as in
  the reference guest. See [slot initialization][slots], [clock bounds][spans],
  [tile composition][tiles], and [clock status][clock-status].
- The clock's centered time uses weight 300 and a size of
  `clamp(min(0.36×width, 0.42×height), 46, 124)` logical px before fitting
  to the cell. `detailScale = clamp(timeSize/58, 0.9, 1.28)`; the date is
  `clamp(15×detailScale, 13, 19)` px at weight 500, separated by
  `clamp(7×detailScale, 5, 9)` px. Time and long date follow locale.
  See [clock face][clock] and [time style][clock-type].
- App/folder tiles center an 85 px icon in a 92 px box, followed by a 9 px
  gap and 13 px semibold text, at most two lines with ellipsis. The app's
  icon rect, not the whole cell, is also the launch-animation anchor. See
  [tile geometry][grid] and [tile typography][labels].

### Layout at logical scale

`width > height` selects landscape. At zero platform inset the grid viewport
starts 66 px down, ends 4 px above the bottom, and uses 22 px horizontal
insets. Page dots reserve 7 px above the last row. This is the [source sizing][grid]
and [pager layout][pager], not a device-specific density preset:

```text
W        = viewportWidth − 44
columns  = clamp(ceil((W + 22) / 246), 4, 14)
tileW    = (W − 22 × (columns − 1)) / columns
H        = viewportHeight − 7
rows(h)  = clamp(floor((H + 22) / (h + 22) + 1e−6), 1, 16)
if tileW / 0.78 > 240:
    n     = rows(131)
    tileH = max(131, (H − 22 × (n − 1)) / n)
else:
    tileH = max(131, min(tileW / 0.78, (H − 44) / 3))
    n     = rows(tileH)
```

Here `viewportHeight` excludes the 66 and 4 px outer offsets. The bottom row
uses the greater of its normal row position plus the 7 px reserve or the
viewport bottom minus `tileH`. Dots sit between the visible content of the
penultimate row and that last row. Dots are hidden for a single page; active
marker 20×7, inactive 7×7. See [grid limits][limits] and [page indicators][dots].

| Output, no platform inset | Grid | Cell | First app center X, after clock |
|---|---:|---:|---:|
| 800×1280 | 4×5 | 172.5×221.15 px | 497.25 px |
| 1280×800 | 6×4 | ≈187.67×164.25 px | ≈535.17 px |

These numbers are source-derived fixtures: compare the location and size
of the cell/icon independently of transparencies in a specific app image.

## Editor, pager, folders and drawer

| Entry | Behavior |
|---|---|
| Horizontal swipe on Home | Follow the finger between pages. The normal pager is locked during tile dragging or resizing. While dragging, an additional empty page is available past the last page. |
| Hold blank Home for 380 ms, with <18 px movement | Open the page editor and request haptics. It previews pages, supports add/remove (at least one), and exposes the drawer and shared bottom row options. Outside tap dismisses it. |
| Hold an app for 380 ms | Lift it; hover in the merge area (radius 42.5 px from icon center) for 260 ms to offer a folder. Reorder dwell is 220 ms; 480 ms within 36 px of a side turns the page. Cancelling returns the tile; opening a folder animates from its tile. |
| Hold the clock | Show its resize frame rather than launch. On the held clock, movement over 28 px can turn the hold into a drag. Clock spans clamp to 2–4 columns and 1–3 rows, subject to free cells and page limits. |
| Swipe up on Home content | Denial ships the drawer **off by default** (`drawer: false` in both the [controller default][drawer-default] and the [persisted layout default][drawer-saved]) — but the user explicitly requested swipe-up access, so the **VeilShell adaptation enables it in the standalone Tablet session** (Desk Mode and the overlay keep their own grids unchanged). When enabled, it accepts vertical motion only with no active editor, folder, drag or resize. The sheet follows the finger by `−dy/height`; release opens above 0.4 progress or on an upward flick faster than 600 px/s. A downward flick faster than 600 px/s closes; otherwise a downward release below 0.4 settles closed. A downward drag at the top of the list transfers to the sheet: while the list is at its top edge and the finger moves down, the list freezes (`_SheetPullPhysics` zeroes the scroll offset and suppresses ballistic simulation) and the sheet takes the drag. The system bottom zone is a [different gesture](navigation.md). |

### Verified drawer contract (touch-first separate All Apps)

The drawer is a full-screen glass sheet over Home listing every app, folders
opened, sorted by name — a separate All Apps surface, not a Home-page state.
Source default is opt-in (`drawer: false` in both controller and persisted
layout), but per the explicit swipe-up request the VeilShell adaptation turns
it **on in the standalone Tablet session**; Desk Mode and the overlay are
untouched. With the drawer off, a content swipe does nothing (it must not open
Recents or Home; those belong to the system bottom zone). Gating
(`_drawerAllowed`) additionally requires no editing, no active tile drag, no
resize, no open folder, and the background-hold timer is cancelled once the
sheet takes the drag. The sheet tracks the finger 1:1 in openness units
(`−dy / height`, sampled from the live widget size) and settles on release:
upward faster than 600 px/s opens, downward faster than 600 px/s closes,
otherwise 0.4 progress decides. Downward motion while the grid scrolls stays
with the list; only at the top edge (`pixels <= minScrollExtent` with downward
delta) does the finger transfer to the sheet, which then holds it until lift —
including the case of a short list. Release velocity comes from a
`VelocityTracker`, with zero on cancel. See [input and drawer][home-input],
[settle rule][drawer-settle], [drawer sheet][drawer], and
[pull physics][drawer-physics].

Penumbra comparison: its Slint launcher is also a background-Home plus
separate drawer split, which is useful ordering evidence, but its phone swipe
distances are not VeilShell targets. Denial's 600 px/s / 0.4 rule is the target
threshold set.

#### Standalone Tablet implementation contract

1. An upward swipe starting on Home content opens the Drawer. The
   bottom-center and status-row targets stay out of this route. Desk and
   overlay keep their existing launchers.
2. The sheet follows `−dy/height` and settles by the 600 px/s / 0.4 rule.
   Cancel and rotation restore the contact's originating scene without
   activating a tile.
3. The list scrolls until it reaches the top. The sheet then takes over the
   same downward contact and holds the list still until lift.

#### Acceptance matrix (drawer)

| Case | Gesture | Expected |
|---|---|---|
| Denial default (drawer off) — reference only | content swipe up with flag off | Nothing opens; no drawer, no Recents |
| Drawer on (VeilShell Tablet default), slow pull, release above 0.4 | upward drag, slow release | Sheet opens fully; grid list scrollable |
| Drawer on, pull to 0.2, slow release | short pull | Settles closed |
| Drawer on, short fast flick up | `vy < −600` px/s, low travel | Opens despite low progress |
| Drawer open, fast flick down | `vy > +600` px/s | Closes |
| Drawer open, drag down slowly from list middle | list not at top | List scrolls; sheet stays open |
| Drawer open, same contact drags down from list top | at `minScrollExtent`, downward delta, no lift | Sheet follows finger; list frozen until lift |
| Tap app icon in sheet | tap | Launches/resumes that app (existing window reused, no duplicate — see [launch][launch]); sheet closes |
| Cancel mid-pull / rotate mid-pull | pointer cancel, geometry change | Contact's originating scene restored; no launch, no stuck sheet |

See [tile recognizers][grid-page], [Home input and drawer][home-input],
[drag controller][drag], [editor][editor], and [drawer sheet][drawer].

The layout repository persists version 5 slots and page counts separately for
portrait and landscape; unsaved orientation reflows the visible items. **The
drawer and shared bottom row flags are global**, not per orientation. Optional
shared bottom row repeats page 0's bottom row. Saved folders may collapse to
one remaining app when the catalog changes. See [layout file][saved] and
[orientation arrangement][arrange]. The drawer ships off upstream; enabling it
here is the documented VeilShell Tablet adaptation for the requested swipe-up
access, not an attempt to look closer to Penumbra.

## Launch and return from an app

1. From a tile or drawer use its global icon rect and item ID; record launch
   recency. Discover expected window IDs from the desktop entry. If there is
   an existing matching local window, activate it and animate it from the
   icon instead of starting a duplicate. In a phone-with-desk split, a match
   on the desk can be handed to the phone instead.
2. Otherwise, register a launch request with expected IDs, the source rect,
   and already-mapped IDs. Animate a placeholder from the source; replace
   it with the matched app surface as it arrives. Lock and a pending request
   gate a second launch. Dispatch failure or a 15 s request timeout clears
   the matching request; do not dismiss a newer request for an old reply.
3. On return to Home, prefer the visible position of the source icon when
   available, including an open folder. Hide the static icon during the
   flight rather than rendering a duplicate. Verify icon visibility and the
   last app frame as part of the transition.

See [launch dispatch][launch], [request lifecycle][launch-state], and
[icon lookup][icon-lookup]. This uses real compositor windows; a successful
desktop command alone does not establish a visible first frame.

## Penumbra comparison

The split between a background Home and a separate app drawer is useful
implementation evidence, but the `412×915` phone canvas, permanent dock and
phone swipe distances are not VeilShell targets. The Rust launcher checks
existing task identity before spawning and waits for the compositor to take
the app stage on map; that ordering is useful even without Slint. Its palette
extraction is optional inspiration, not a substitute for Denial's plain
wallpaper treatment. See [Home launcher][pen-launch] and [phone geometry][pen-geometry].

## VeilShell decision and replay

The first **visual** slice is the standalone default grid, its clock/date and
wallpaper treatment. VeilShell now has that first-page span and responsive
grid, with the original app discovery and focus reuse. It keeps the overlay's
grid unchanged. The clock currently uses the renderer's existing font tiers,
not a matched Denial typeface. Slot editing, persistent layouts, folders
and shared icon-to-window motion remain separate changes; the drawer is in
this slice, enabled for standalone Tablet per the adaptation above.
See [the fixture notes](../DENIAL_VALIDATION.md#tablet-status-row-and-home-grid)
for the Home grid and [navigation replays](../DENIAL_VALIDATION.md#tablet-bottom-navigation-and-all-apps)
for the Drawer.

1. Clean layout, same wallpaper, fonts, time and app fixtures: compare
   800×1280 and 1280×800, then 150% on the tablet. Check first clock/app
   positions, no page marker for one page, and a continuous drag to page 2
   with a larger catalog.
2. Launch an app, then launch the same item from Home. Focus the existing
   window (no duplicate); for a cold launch verify placeholder-to-real-frame
   timing. Force a genuine launch failure and verify the next tap still works.
3. With editing implemented, hold A over B, commit a folder, cancel a second
   drag, rotate, then rotate back. Verify saved positions, page count, clock
   span, and one-app folder collapse.
4. With the drawer enabled (VeilShell Tablet default), swipe up on Home
   content: a committed pull opens the All Apps sheet, reversal above and
   below the threshold settles correctly, and launch from the sheet reuses
   the existing window. The drawer-off behavior is reference-only. Test the
   system bottom gesture separately.

[surface]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/home_surface.dart#L572-L701
[scrim]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/theme/tokens.dart#L15-L33
[pager]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/home_pager.dart#L90-L177
[slots]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/launcher/controllers/home_grid_layout.dart#L90-L144
[spans]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/launcher/models/home_grid_item.dart
[tiles]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/widgets/home_tiles.dart#L16-L37
[clock]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_clock/ui/lib/clock_face.dart#L48-L162
[clock-status]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_clock/ui/lib/clock_face.dart#L172-L253
[clock-type]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_clock/ui/lib/clock_face.dart#L458-L490
[grid]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/home_grid_geometry.dart#L8-L110
[labels]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/widgets/home_app_tile.dart
[limits]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/launcher/controllers/home_grid_layout.dart#L26-L79
[dots]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/widgets/page_dots.dart
[grid-page]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/widgets/home_grid_page.dart#L215-L250
[home-input]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/home_surface.dart#L267-L474
[drag]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/home_drag_controller.dart
[editor]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/widgets/home_page_editor.dart
[drawer]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/widgets/home_app_drawer.dart
[drawer-default]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/launcher/controllers/home_grid_controller.dart#L51-L51
[drawer-saved]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/launcher/repositories/home_layout_repository.dart#L12-L23
[drawer-settle]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/home_surface.dart#L454-L462
[drawer-physics]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/widgets/home_app_drawer.dart#L165-L186
[saved]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/launcher/repositories/home_layout_repository.dart#L12-L99
[arrange]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/launcher/controllers/home_grid_controller.dart
[launch]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/home_launch.dart#L12-L73
[launch-state]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart#L508-L674
[icon-lookup]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/home_surface.dart#L523-L563
[pen-launch]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-shell/src/bin/homescreen.rs
[pen-geometry]: https://github.com/penumbra-mobile/penumbra-env/blob/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb/crates/penumbra-comp/src/phone.rs#L15-L43
