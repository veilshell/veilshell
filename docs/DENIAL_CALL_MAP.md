# Denial → VeilShell call map

Reference: Denial [`01b588e`][denial]. This is a behavior audit, not a
Flutter port. **Tablet** means Denial's `mobile` profile in either orientation;
**Desktop** maps to standalone VeilShell Desk Mode. VeilShell's overlay is
neither: it does not own the host's layout, lock, or services.

Trace each user or system trigger through the handler, state changes, native
calls, and completion, denial, or cancellation. Trace back from the protocol
and service entry points to catch non-UI triggers. Record visual states and
motion against the [parity reference](DENIAL_PARITY_SPEC.md), separately from
the calls that implement them. A row is not verified until its acceptance
check passes. This is the first batch, not a complete call inventory.

## First batch: SDDM round trip

| ID / trigger | Denial call path and observable behavior | VeilShell equivalent / acceptance |
|---|---|---|
| S01 · Both: choose session at SDDM | `denial.desktop` → `denial-session` → `deniald`. The launcher selects DRM, output configuration and shell bundle. Authenticated startup is separate from autologin. | **Reuse:** `veilshell.desktop` → `veilshell-session` → dedicated Hyprland and `veilshell --policy tablet --scene home`. Test stock SDDM login and failure back to greeter; leave Omarchy and the default session alone. |
| S02 · Both: startup readiness and autostart | [Startup contract][startup]: compositor publishes selected sockets to D-Bus activation; shell readiness and first output commits gate systemd publication and graphical/autostart targets; teardown restores prior systemd environment. | **Extend/delegate:** existing display handshake gates Hypridle and optional allowlisted autostart. It does not establish portal or polkit readiness. Prove file picker, screencast, and an authorization prompt against the standalone display in a fresh login; then log back into Omarchy and check its services. Don't blindly import Denial's systemd policy. |
| A01 · Tablet: tap Home icon | [`launchHomeApp`][home] records the app in launcher recents, derives expected window IDs, then checks the tablet's open windows and the desk. Calls `sendToPhone` for a desk match, `activateAppFromLauncher` for a tablet match, otherwise `beginAppLaunch` → `AppLauncher.launch`; failed dispatch calls `failAppLaunch`. | **Extend:** standalone Home now checks `StartupWMClass`, then desktop-file ID, against mapped windows before launching. Verify an existing app comes forward without a duplicate, an unrelated web app doesn't match, and an absent app starts on this session. MRU app suggestions, explicit output moves, and launch animation/feedback are not implemented by this slice. |
| A02 · Desktop: search and activate a result | [Desktop shell][desktop] `_launchApp` closes panels, records recency and moves a matching phone window to the desk when split roles are active; otherwise it calls `AppLauncher.launch`. [App launcher][launcher] parses `Exec`, then calls `DenialBridge.launchDesktopApplication` → `SessionClient.launchDesktopApplication` → native [system command][syscmd] queue. Local Flutter apps take a separate path. | **Extend/delegate:** keep freedesktop app discovery and launch in the owned compositor; add search, keyboard selection, Enter/Escape and conveyed launch failure. Test Terminal, browser, file manager and an Omarchy web app without Omarchy's shell/session manager, plus the same apps via the opted-in shortcut profile. No Flutter runtime. |
| W01 · Tablet: tap or throw a Recents card | [Controller][controller] `openFromRecents` resolves the focus target and focuses it; `leaveRecents` removes the card from the deck during motion and `dismissFromRecents` closes windows riding the card before its owner. The deck reorders and closes when empty. | **Reuse/extend:** VeilShell captures window previews and sends `focus_window` / `close_window` to Hyprland. Check cancellation, last-card behavior and grouped/transient windows separately; a card disappearing is not proof that the client accepted close. |
| W02 · Desktop: focus, switch workspace, move or close | [Desktop shell][desktop] `_activateWindow` and [shell services][services] `activateWindow` / `switchWorkspace` link desktop UI to focus and workspace controllers. [`WindowsClient`][windows] issues native requests such as `switchWorkspace`, `moveWindowToWorkspace`, and `closeWindow`. | **Delegate/extend:** Hyprland owns focus, tiling and workspace dispatch. Current relative switching is not desktop overview or move-to-workspace parity. Check focus/close/switch/move across workspaces with personal Omarchy chords taking precedence over shell defaults. |
| W03 · Both: move between tablet and desk | [Controller][controller] `moveToDesk` / `sendToPhone` resolve an output from split roles then call `DenialBridge.moveWindowToOutput`; a missing split is a no-op. Tablet Recents stays open when sending a window to the desk. | **Extend:** explicit per-output roles and a compositor move adapter. Check handoff in both directions and recover windows after unplug. Standalone Home's focus behavior does not claim output handoff. |
| P01 · Both: menu lock or power action | Mobile power menu and desktop session panel share [`SessionPowerController`][power]. `request` checks availability and requests confirmation where required; `confirm` dispatches, cancel doesn't. Lock delegates to authentication, logout/session restart to the compositor with a watchdog, and suspend/hibernate/system reboot/power off to logind. Errors clear busy state and surface a message. | **Reuse/extend:** standalone power sheet → `veilshell-session --power` or scoped daemon exit. Suspend waits for Hyprland lock acknowledgment before asking logind to sleep. Verify confirmation, cancellation, denial, and logout to SDDM. Session restart and hibernate aren't implemented. |
| P02 · Both: lock, wake, unlock | Screen/power requests go through the [session bridge][session] and [startup/locking contract][startup]; lock face and credential entry are separate from the app shell. Confirm the secure state and its failure path separately from animation. | **Delegate now; extend later:** Hypridle and Hyprlock in the preview; [private locker/PAM plan](TOUCH_LOCK.md) for touch unlock. Test wrong password, cancel, hotplug, locker failure, hardware sleep/wake and no unlocked frame. Never use the app OSK for credentials. |

### A01 validation

To exercise A01 without changing the host or logging out, run
`VEILSHELL_TEST_STANDALONE=1 bash scripts/test-hyprland-actions.sh` from an
existing Wayland session. The nested test launches a fixture, focuses another
window, then checks that a second Home tap focuses the original window
address without creating a duplicate. This is not an SDDM login test.

## Next audit batches

- Home layout/editing, dock, drawer, desktop Launcher/Dashboard, wallpaper
  and the input/animation paths for gestures.
- Shade and desktop controls: NetworkManager/iwd, BlueZ, audio/media, devices,
  notification history and actions, DND, permissions, screenshots.
- Text input and keyboard routing, hardware keys and OSD, rotation, idle and
  external monitor events; extend the lock trace through authentication and wake.
- Cross-check input/shortcut actions, [`denial.fbs`][wire] requests/events,
  platform channels not in that schema, and service APIs. Mark each remaining
  entry point as mapped, internal, delegated, blocked or out of scope.

Omarchy compatibility uses a separate mapping from the user's effective key
chords to the above actions. Retain the behavior of app and window shortcuts
without sourcing the whole Omarchy session; disclose conflicts and shell-only
bindings without an equivalent. For example, the preview's `Super+R` Recents
binding must not take precedence over a personal resize binding.

## Desktop launcher and native shortcut dispatch

| Trigger | Denial behavior at `01b588e` | VeilShell mapping |
|---|---|---|
| Open Applications | [`native_shortcut.rs`][native] maps `OpenApplications` to `RequestApplications`; the desktop shell toggles its launcher. Search takes focus, and the launcher gets installed apps and recent suggestions. | **Partial:** opt-in `Super+Space` toggles the keyboard-focused `launcher` scene, separate from Tablet Home. Recent suggestions are not implemented. |
| Type, navigate, submit, dismiss | [Launcher widget][launcher-ui] filters the visible targets; selection falls back to the first result and is absent on an empty list. Tab advances or reverses, arrows move within/across rows, Enter invokes `onLaunch(entry.id)` only for a valid target, Escape dismisses. The desktop shell closes panels before dispatching launch. | **Implemented baseline:** filter the shared app catalog by name or desktop-file ID, select with arrows or Tab/Shift+Tab, open with Enter or a click, dismiss with Escape or a click outside. Nested Hyprland verifies real keyboard events, no launch from empty results, and restoring hidden mode. Run `VEILSHELL_TEST_LAUNCHER=1 bash scripts/test-hyprland-actions.sh` with `wtype` installed. |
| Window switcher | Shortcut disposition enters/cycles the switcher and ends it on the owning key, modifier, or gesture release; overview is a distinct action. | **Delegate/partial:** Omarchy-style `Alt+Tab` cycles focus through Hyprland. No release-to-commit switcher UI. Recents is separate. |
| Focus, swap, close, fullscreen, workspaces | [Shortcut actions][shortcut-actions] include directional focus/swap, window close, fullscreen and numbered workspace switch/move. | **Delegate:** opt-in native Hyprland dispatch for these actions. Workspace IDs are global rather than Omarchy's per-monitor numbering. |
| Launch from a key | The configurable application target uses a desktop-file ID and the native launcher. | **Partial:** opted-in terminal, default browser and file manager chords call the scoped standalone launcher. Arbitrary Omarchy app/web-app helpers are not imported. |
| Lock, media and brightness | The native action list includes lock, volume and brightness. | **Delegate:** Hyprlock, WirePlumber and `brightnessctl`; playback uses optional `playerctl`. No Denial-style OSD yet. |
| Other configurable actions | Dashboard/clipboard/settings, region capture, keyboard layout cycling, pointer release, additional resize/maximize/minimize/pin actions, relative window moves and shutdown. | **Unmapped or partial:** not claimed by this profile. Audit each against the relevant UI or backend before exposing it. |

See the [standalone keybinding profile](../adapters/session/README.md#optional-omarchy-style-keybindings)
for the exact supported chords, overrides, and dependencies.

[denial]: https://github.com/denialwm/denial/tree/01b588e
[startup]: https://github.com/denialwm/denial/blob/01b588e/docs/SESSION_STARTUP.md
[home]: https://github.com/denialwm/denial/blob/01b588e/plugins/denial_desktop/lib/src/launcher/home_launch.dart
[desktop]: https://github.com/denialwm/denial/blob/01b588e/plugins/denial_desktop/lib/src/desktop/desktop_shell.dart
[controller]: https://github.com/denialwm/denial/blob/01b588e/plugins/denial_desktop/lib/src/state/reference_shell_controller.dart
[services]: https://github.com/denialwm/denial/blob/01b588e/plugins/denial_desktop/lib/src/core/shell_plugin_services.dart
[launcher]: https://github.com/denialwm/denial/blob/01b588e/packages/denial_flutter_sdk/lib/src/launcher/services/app_launcher.dart
[windows]: https://github.com/denialwm/denial/blob/01b588e/packages/denial_flutter_sdk/lib/src/platform/bridge/windows_client.dart
[power]: https://github.com/denialwm/denial/blob/01b588e/packages/denial_flutter_sdk/lib/src/state/session_power.dart
[session]: https://github.com/denialwm/denial/blob/01b588e/packages/denial_flutter_sdk/lib/src/platform/bridge/session_client.dart
[syscmd]: https://github.com/denialwm/denial/blob/01b588e/compositor/src/bin/deniald/flutter_runtime/system_command.rs
[wire]: https://github.com/denialwm/denial/blob/01b588e/protocol/denial.fbs
[native]: https://github.com/denialwm/denial/blob/01b588e/compositor/src/bin/deniald/native_shortcut.rs
[launcher-ui]: https://github.com/denialwm/denial/blob/01b588e/plugins/denial_launcher/lib/src/desktop_application_launcher.dart
[shortcut-actions]: https://github.com/denialwm/denial/blob/01b588e/packages/denial_flutter_sdk/lib/src/models/shortcut_configuration.dart
