# VeilShell

**A lightweight, touch-first Wayland shell for Linux tablets.**

VeilShell adds gesture navigation, a paged app launcher, window previews,
Quick Settings, and an on-screen keyboard to a Linux desktop. It's built in Rust for
older, fanless tablets, starting with the Microsoft Surface Go 3. The goal is
one touch shell that works inside your existing desktop first and can later
serve as the main interface in a dedicated tablet session.

## Two ways to run one shell

- **Overlay — the `v0.1.0` goal.** Log into Hyprland as usual, with or without
  Omarchy. VeilShell adds touch controls without taking over the compositor,
  keybindings, or desktop services. Laptop Mode hides the shell without
  rewriting host layout or monitor rules. No separate login is required.
  Quick Settings actions you explicitly take still affect the device.
- **Standalone — an opt-in preview.** A separate SDDM entry starts the same
  VeilShell daemon in a dedicated Hyprland session. That session can own
  policies like fullscreen apps on small screens and a keyboard-and-mouse Desk
  Mode without changing the normal desktop. Those policies and touch-only
  daily use are longer-term goals, not requirements for `v0.1.0`. The preview
  still needs a physical keyboard for password unlock.

**Compatibility today:** Hyprland is the only live compositor backend.
Surface Go 3 + Hyprland is the first release validation target, not a claim of
completed hardware testing. Omarchy integration is optional and reads the
Aether theme without changing it. The overlay requires `wlr-layer-shell`, but
that alone does not enable every feature. See the [adapter contract] for
requirements, fallbacks, and how to add support for another environment.

The [performance budget] sets targets for older 5W, 4–8 GB x86 and ARM
tablets. VeilShell avoids redraws while hidden and has a software rendering
fallback. Offscreen PNG snapshots and a nested Hyprland harness let
contributors review UI changes without a tablet; real-device measurements
are tracked separately.

[adapter contract]: docs/PORTING_A_COMPOSITOR.md
[performance budget]: docs/PERFORMANCE_BUDGET.md

## Workspace Layout

| Crate | Purpose |
|---|---|
| [`veilshell-core`](crates/veilshell-core) | Shell state, gestures, spring physics, `.desktop` parsing, and keyboard state |
| [`veilshell-theme`](crates/veilshell-theme) | Theme adapter, Omarchy Aether reader, and built-in fallback |
| [`veilshell-compositor`](crates/veilshell-compositor) | Compositor adapter trait, live Hyprland IPC, and test fixtures |
| [`veilshell-render`](crates/veilshell-render) | Scene layout, hit-testing, GPU and software rendering, and PNG snapshots |
| [`veilshell-wayland`](crates/veilshell-wayland) | Layer surfaces, scaling, Wayland input and keyboard, and Recents capture |
| [`veilshell-hw`](crates/veilshell-hw) | Linux hardware telemetry, controls, and device profiles |
| [`veilshell-shell`](crates/veilshell-shell) | Daemon, `veilctl` CLI and IPC, app discovery, and adapter wiring |
| [`veilshell-lock-input`](crates/veilshell-lock-input) | Separate touch-lock input prototype; not used by the preview |

Distro integration glue lives under [`adapters/`](adapters/omarchy/README.md),
and device quirk profiles live under [`hardware-profiles/`](hardware-profiles/microsoft-surface-go-3.toml).

## Quick Start

Requires Rust `1.86+` and the `libxkbcommon` development files (`libxkbcommon`
on Arch, `libxkbcommon-dev` on Debian/Ubuntu). From a terminal in your existing
Hyprland session:

```bash
# Show the gesture pill and top handle
cargo run --release --locked -p veilshell-shell --bin veilshell -- --policy tablet
```

On a device with a supported detachable keyboard, start with `--policy auto`
instead to show the shell when the keyboard detaches and hide it on reattach.

The live daemon stops with a diagnostic if it cannot connect to Hyprland;
`--dry-run` is for offline inspection only.

In a second terminal, use `cargo run --release --locked -p veilshell-shell --bin veilctl -- status`
to inspect the session. Replace `status` with any of the commands below, or
run `veilctl` directly if it's on your `PATH`:

```bash
veilctl show home        # Open paged .desktop app launcher
veilctl show drawer      # Open All Apps in standalone Tablet Mode
veilctl show launcher    # Toggle the searchable desktop launcher
veilctl show recents     # Open the window deck (tap to focus, swipe up on a card to close)
veilctl show shade       # Open Quick Settings shade
veilctl show osk         # Open the app keyboard manually
veilctl show pill        # Dismiss overlays back to the bottom gesture pill
veilctl mode toggle      # Toggle Laptop (hidden) <-> Tablet Mode
veilctl reload-theme     # Hot-reload Aether colors.toml
veilctl quit             # Stop the daemon cleanly
```

If VeilShell is running in the foreground, Ctrl+C also shuts it down cleanly.

Tap the translucent **keyboard icon** near the bottom-right corner to type in
the focused app. The typing rows stay centered around a wide spacebar. Shortcuts sit
beside the keys in wide landscape views, or above them in portrait and on
narrower outputs. **TERM** adds Escape, Tab and all four arrows to the letter
layout. Hold Backspace or an arrow to repeat. Tap **HIDE** or swipe down to
dismiss.

Compatible text fields can also open the keyboard automatically. If you use
another input method, start VeilShell with `--osk manual` to leave it alone;
manual typing requires Wayland virtual-keyboard support.

The app keyboard reflows when the compositor rotates the output. In overlay
mode VeilShell follows that orientation without changing monitor rules. The
app keyboard is not a login or lock-screen keyboard.

In the **overlay**, swipe up from near the bottom pill for Home, or down from
near the top handle for Quick Settings. The handle brightens when grabbed and the panel follows
your finger. Tap on the mark itself to open; the larger area around it is
swipe-only. Pull back toward the edge before lifting to cancel. Swipe
down on Home or up on Quick Settings to close. Swipe across Home to slide to
another app page, or swipe up again from Home's bottom edge to open Recents.
In Recents, swipe across to browse windows and swipe up on a card to close it.
A horizontal swipe on the bottom pill still changes workspaces.

For a pill-free touchscreen overlay, opt into
[`--overlay-gap-gestures`](adapters/omarchy/README.md#optional-pill-free-touch-gestures).
It reuses the Tablet gestures from existing uncovered gaps behind the bar
and windows, without changing the host layout.

In the **standalone Tablet session**, Home has a clock and date in the grid.
Flick up from the bottom center to go Home from an app; pull up more slowly
and release to open Recents. A bottom-center swipe up from Home opens
Recents, while swiping up on Home **content** opens the scrollable **All
Apps** drawer. Swipe right along the bottom for an older running window
or left for a newer one on this display. These swipes don't wrap or
launch apps.

Pull down from anywhere on the status row in Home or an ordinary app to
open Shade. In portrait, pull from the left for Notifications or the right
for Controls, then swipe sideways to switch pages. Landscape shows both
panes. Notification history isn't connected yet. Swipe up to close Shade
and return to where you opened it.

**Quick Settings:** Use the separate **Wi-Fi Networks**, **Bluetooth Devices**,
and **Audio Output** buttons to reconnect to saved networks or paired devices,
or to change the default sound output. The radio switches are separate.
Available controls depend on NetworkManager (`nmcli`), BlueZ (`bluetoothctl`),
WirePlumber (`wpctl`), and the PulseAudio-compatible `pactl` interface.
**More** shows playback controls when a player is available. On Omarchy it
can also open notification history or take a screenshot. New network
credentials and Bluetooth pairing stay with the host settings panels.

For Omarchy users, optional instant theme-hook (`adapters/omarchy/hooks/30-veilshell-theme`)
and `systemd --user` (`adapters/omarchy/systemd/veilshell.service`) integration guides live in [`adapters/omarchy/README.md`](adapters/omarchy/README.md).
Set `VEILSHELL_ICON_THEME` before starting VeilShell to override icon lookup
for the shell only. It does not change your desktop's icon theme.
For SDDM, see the [standalone session preview](adapters/session/README.md).
It is an opt-in entry, not a greeter theme or an autologin setup, and currently
requires a physical keyboard for password unlock.

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — Crate boundaries, `calloop` multi-surface runtime, and verification pyramid
- [docs/ROADMAP.md](docs/ROADMAP.md) — `v0.1.0` overlay release gates, standalone goals, and development history
- [docs/OVERLAY_VALIDATION.md](docs/OVERLAY_VALIDATION.md) — Overlay install and Surface Go 3 smoke-test checklist
- [Tablet behavior audit](docs/tablet/README.md) — Denial behavior, Penumbra implementation references, and replay cases
- [docs/DENIAL_COMPARISON.md](docs/DENIAL_COMPARISON.md) — Touch UX reference and feature inventory, not a release checklist
- [docs/DENIAL_PARITY_SPEC.md](docs/DENIAL_PARITY_SPEC.md) — Pinned interaction and visual reference, and standalone acceptance gates
- [docs/DENIAL_VALIDATION.md](docs/DENIAL_VALIDATION.md) — Test evidence and its limits
- [docs/PERFORMANCE_BUDGET.md](docs/PERFORMANCE_BUDGET.md) — Hard CPU, RAM, and frame-time budgets on 5W tablets
- [docs/HARDWARE_PROFILES.md](docs/HARDWARE_PROFILES.md) — Adding declarative TOML profiles for new tablets
- [docs/PORTING_A_COMPOSITOR.md](docs/PORTING_A_COMPOSITOR.md) — Compatibility contract and contributor guide for new environments
- [CONTRIBUTING.md](CONTRIBUTING.md) — Tests, snapshot and nested compositor harnesses, and good first issues

## Acknowledgments & License

VeilShell is licensed under **`GPL-3.0-or-later`** (see [LICENSE](LICENSE)).
Inspired by **Denial** (`denialwm/denial`), **Droidloom**, **Omarchy**, and
**Hyprland** — see [NOTICE.md](NOTICE.md) for full credits.
