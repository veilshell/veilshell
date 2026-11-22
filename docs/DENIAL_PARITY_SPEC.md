# Denial parity reference

Reference: [Denial at `01b588e`](https://github.com/denialwm/denial/tree/01b588ec50831bce9e57aed623901f49846ec791).
This is a source-derived reference, not a claim of visual or functional parity.
VeilShell reimplements the behavior in Rust and uses Hyprland for compositor
responsibilities; it does not need Denial's binaries, Flutter engine, or artwork.

## Which Denial UI to match

Use the **mobile** profile as the primary reference for touch and tablet. A
tablet rotated to landscape still uses the mobile layout, not the desktop
profile. The desktop [launcher] and [Dashboard] are separate reference surfaces
for future keyboard/mouse use, not screenshots of the mobile launcher and shade.

| Surface | Denial reference | VeilShell status |
|---|---|---|
| Mobile Home | [Home grid]: 22 px gaps, 85 px icons in 92 px boxes, 9 px label gap, 13 px semibold wallpaper-facing text. Four base columns with extra columns on wider panels. [Pager]: active marker 20×7, inactive dots 7×7, hidden for a single page. | Adaptive paged grid, plain icons, quiet status row, and pager geometry implemented. Still a layer-shell overlay, not a persistent wallpaper desktop. No widget slots, folders, shared dock row, grid editing, or [all-apps drawer]. |
| Mobile navigation | [Liquid pill]: 180×5 at rest, neutral fill with accent during motion. Upward and horizontal gestures drive Home/overview/navigation. | Resting size and color implemented; existing VeilShell gesture thresholds and motion remain. |
| Desktop launcher | [Launcher]: focused search field, alphabetical catalog, recency suggestions, scrollable grid, keyboard selection and Return to launch. Tiles are 112 px high (96 px for suggestions), with 8 px spacing. | Search and keyboard navigation are not implemented. Add as a dedicated launcher flow rather than treating search as part of Denial's mobile Home. |
| Mobile shade | [Shade]: fullscreen frosted backdrop, separate notifications and controls pages; origin side selects the page and a horizontal swipe switches pages. Header and tiles adapt to landscape. | One centered Quick Settings pane with working toggles/sliders; no two-page shade, notification history, or media card yet. |
| Overview | [Desktop overview] and the mobile window deck are distinct; the mobile deck moves with the gesture. Compositor previews are intrinsic to upstream. | Hyprland window capture works where supported. Remove the visible-card and capture limits together; verify every window is reachable and focus/close targets stay attached to the correct window during scrolling. |
| Lock and power | [Lock face] uses wallpaper (default blur sigma 8, dim 24% in [lock settings]), clock/date and status, with a mobile bottom swipe affordance. [PIN stage] is a separate reveal in portrait; landscape can dock the keypad alongside the face. PIN entry auto-submits at the configured length, with an explicit password fallback. [Power menu] and [desktop session controls] are separate presentations. | SDDM preview delegates secure password lock to Hyprlock and exposes a touch power menu. Native touch authentication, PIN, biometrics, lock editor, and lock-screen widgets are still pending. |
| Touch keyboard | [Mobile OSK]: Backspace and arrow repeat, long-press accents, and optional word suggestions. | App OSK supports Backspace/arrow repeat and an optional terminal row. Accents, suggestions and physical-layout parity are still open. |
| Tablet and external monitor | [Phone and monitor]: mobile UI on the built-in panel, desktop on the external display, a move-to-monitor action in mobile Recents, and window recovery on unplug. Optional [Input Deck] dedicates the touch panel to a keyboard and relative touchpad. | Surface pinning exists. Per-output roles, window handoff and recovery, and Input Deck are not implemented. |

The Home implementation intentionally uses an opaque dark fallback when no
wallpaper is configured, rather than showing active windows behind launcher
icons. The existing GPU wallpaper cache is an approximation; Denial paints the
actual wallpaper with an 8% black Home scrim. The software path currently has
no wallpaper cache. Do not compare the two products over different backgrounds
and count that as a layout comparison.

## Material and type targets

Values come from upstream's [shell colors], [shell tokens], [glass config], and
[application theme]. Pixels below are logical pixels. Material roles apply to
specific surfaces; do not substitute application colors for the shell palette.

| Token | Dark | Light |
|---|---|---|
| Shell background | `#0F1115` | `#F7F7FB` |
| Shell panel tint, RGB before opacity | `#14171D` | `#F5F5F9` |
| Shell primary / secondary text | `#F4EFF4` / `#CAC4D0` | `#1B1B21` / `#45454F` |
| App content / raised content | `#22211F` / `#2C2B29` | `#F7F6F3` / `#FFFFFF` |
| App glass tint | Black | White |

- Shell accent default: `#D0BCFF`; wallpaper-facing text: `#F7F7F8`.
- Window and panel radius: 28; notification: 18; tile: 24; chip: 21.
- Standard panel opacity: 0.75. The configurable glass defaults are distinct:
  shell/window/app-panel opacity 0.4, blur sigma 10, refraction 0.2, dispersion
  0. A static wallpaper blur and SDF border are not compositor refraction.
- Base type: 14 px. System bar value: 13 px bold monospace. Lock clock/date
  base styles: 124 px light / 24 px semibold; the lock face scales and stretches
  the clock according to profile and available area.
- VeilShell imports the dark shell colors as fallback tokens only; Omarchy
  theme discovery remains in place. Light mode, independent glass settings,
  exact fonts, shadows, and reversible gesture motion remain parity work.

## SDDM acceptance gates

Denial [ships a Wayland session entry], not a greeter theme. Following its
[startup contract], an authenticated SDDM login should launch VeilShell
**unlocked**. An unauthenticated autologin path must be separately designed and
must not use an ordinary fullscreen layer surface to imitate secure locking.

1. **Session lifecycle:** Install a world-traversable launcher and
   `wayland-sessions/veilshell.desktop` with `Exec` and `TryExec`. Start a
   dedicated Hyprland config and VeilShell with an isolated control socket.
   Publish activation state after readiness, do not disrupt an existing user
   session's services, scope cleanup to the session, and return to SDDM on
   normal exit and startup failures (Denial remaps launch status 1 to 2).
2. **Security:** Use Hyprland's `ext-session-lock-v1` with lock surfaces on all
   outputs, including hotplug. Only trusted authentication may unlock;
   logind `Unlock`, shell IPC, focus loss, and UI crashes must not. Validate
   password success, denial, cancellation, rate limiting, and PAM account policy
   before implementing PIN or optional fingerprint. Exercise lock-before-sleep
   and no desktop flash on wake. Do not enable autologin until the startup lock
   is verified on real hardware.
3. **Daily use:** Verify touchscreen login-to-app launch, on-screen input,
   networking, app switching, idle/wake, logout, and failure recovery. Keep
   polkit, portals, and desktop autostart explicit; prove they work in a fresh
   session rather than inheriting them from Omarchy.
4. **Visual acceptance:** Compare portrait and landscape at the same logical
   size and scaling with matching wallpaper and stable app fixtures. Check
   spacing, touch hit targets, translucency, and gesture transitions on the
   Surface Go. Offscreen CPU/GPU snapshots establish a baseline, not proof of
   live session or pixel parity.

See [the feature comparison](DENIAL_COMPARISON.md) for the broader backlog.

[Mobile OSK]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/osk/shell_osk_panel.dart
[Phone and monitor]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/docs/PHONE_AND_MONITOR.md
[application theme]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/theme/application_theme.dart
[shell colors]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/theme/shell_color_scheme.dart
[shell tokens]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/theme/tokens.dart
[glass config]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/theme/glass_configuration.dart
[Home grid]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/home_grid_geometry.dart
[Pager]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/widgets/page_dots.dart
[all-apps drawer]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/launcher/widgets/home_app_drawer.dart
[Liquid pill]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/liquid_pill.dart
[Launcher]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_launcher/lib/src/desktop_application_launcher.dart
[Dashboard]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/desktop/desktop_dashboard.dart
[Shade]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/shade/quick_settings_panel.dart
[Desktop overview]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/desktop/desktop_overview_layout.dart
[Lock face]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/lock/lock_screen_background.dart
[lock settings]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packages/denial_flutter_sdk/lib/src/settings/shell_settings.dart#L510
[PIN stage]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/lock/lock_pin_stage.dart
[Power menu]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/session/mobile_power_menu.dart
[desktop session controls]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/plugins/denial_desktop/lib/src/widgets/session/power_session_surface.dart
[Input Deck]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/docs/INPUT_DECK.md
[ships a Wayland session entry]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/packaging/arch/denial.desktop
[startup contract]: https://github.com/denialwm/denial/blob/01b588ec50831bce9e57aed623901f49846ec791/docs/SESSION_STARTUP.md
