# VeilShell Architecture

VeilShell is organized as a modular Rust workspace with strict boundary separation so core shell logic stays portable across Linux distributions, Wayland compositors, and tablet hardware.

```
                   +-----------------------+
                   |    veilshell-shell    |  veilshell daemon + veilctl CLI & IPC
                   +-----------+-----------+
         +------------+--------+--------+------------+
         |            |        |        |            |
         v            v        v        v            v
  veilshell-core  veilshell-  veilshell-  veilshell-  veilshell-
  (pure logic,    compositor  wayland     render      theme + hw
   .desktop AST)  (Hyprland   (calloop +  (DrawList,  (Omarchy Aether
                   Lua/legacy) layer-sh)   SDF+HitTst) + profiles)
```

## Crate Responsibilities

- **`veilshell-core`**: Zero-I/O pure Rust. Critically damped harmonic springs, bottom-pill & edge gesture classifier, Laptop↔Tablet mode state machine, orientation/rotation lock, WCAG contrast color primitives, freedesktop `.desktop` parser (`DesktopApp`, field-code stripping, badge formatting), and multi-layer Touch OSK state (`OskState`, `OskLayer`, `ShiftState`, evdev mapping, `US_QWERTY_XKB_KEYMAP`).
- **`veilshell-compositor`**: `CompositorBackend` trait isolating all compositor calls. Ships with `HyprlandBackend` (live Unix-socket IPC supporting recency-sorted `j/clients` with `stableId` `foreign_id` filtered to mapped windows, `j/monitors`, workspace switching, app launch `exec`, and dual Omarchy 4.5+ Lua `eval` with legacy `keyword` fallback) plus `MockBackend` for offline tests.
- **`veilshell-theme`**: `ThemeProvider` trait. Reads Omarchy Aether `colors.toml` via `OmarchyAetherProvider` when present, falling back to a built-in WCAG AA dark palette on any other distro.
- **`veilshell-hw`**: `HardwareBackend` and `HardwareController` traits, `HardwareSnapshot`, DMI auto-matching (`detect_dmi_device`, `load_matching_profile`) over user/built-in `hardware-profiles/*.toml`, pure Linux `sysfs` readers (filtering peripheral `scope=Device` stylus batteries from system batteries and matching USB Type Cover IDs), standard CLI backends (`nmcli`, `bluetoothctl`, `wpctl`, `brightnessctl`, the session's `hyprsunset`), and `SensorGuard` wrapping `monitor-sensor --accel` with clean child reaping on drop.
- **`veilshell-render`**: Renderer-agnostic glassmorphic `DrawList` + interactive hit-test regions (`SceneOutput`) for all 5 scenes (`pill`, `home`, `recents`, `shade`, `osk`) and `TopStrip`, single-pass instanced `wgpu` Vulkan/GLES WGSL SDF backend (`WgpuRenderer`) with a shelf-packed GPU glyph/icon/thumbnail atlas and blur-once GPU static wallpaper frosted-glass cache, cached-backdrop CPU analytic SDF fallback (`SdfSoftwareRenderer`), `ActiveRenderer` (`auto`/`wgpu`/`software`), `ab_glyph` system TrueType typography (with built-in 5×7 HUD fallback), SDF-clipped `RgbaImage`/`ImageQuad` support, premultiplied `ARGB8888` encoder, and `PNG` snapshot export.
- **`veilshell-wayland`**: Event-driven `calloop` + `WaylandSource` runtime wrapping `smithay-client-toolkit 0.21` (`wlr-layer-shell`). Manages `BottomPill`, `TopStrip`, `TouchOsk`, and reused full-screen `Overlay` surfaces (dropping the bottom pill cleanly when `TouchOsk` is active) with zero-copy `wgpu::Surface` Vulkan/GLES swapchains (isolated in a single `raw-window-handle` bridge with automatic `wl_shm` `SlotPool` fallback, supporting `initial_visible: false` sleep in Laptop Mode for `0%` idle CPU and syncing logical dimensions on `wl_output`/configure rotation), drives built-in Touch OSK text input & auto-show via `zwp_input_method_v2` + `zwp_virtual_keyboard_v1`, captures live Recents window thumbnails via `ext-foreign-toplevel-list-v1` + `ext-image-copy-capture-v1`, binds optionally to a named `wl_output` (`--output`), and routes `wl_touch`/`wl_pointer` gestures and hit-tests back via `ShellActionHandler`.
- **`veilshell-shell`**: Finds `.desktop` entries and freedesktop PNG/SVG icons in XDG data directories, runs the Unix control listener (`show`, `status`, `wallpaper`, `mode`, `reload-theme`, `quit`), polls USB Type Cover presence and `sysfs`, and ships the `veilshell` daemon and `veilctl` CLI.

## Session boundary

The overlay follows the host output's scale without changing monitor rules.
Scene layout and input use logical coordinates; the renderers use
physical-pixel buffers. The standalone adapter publishes its own Wayland and
Hyprland endpoints in a private runtime directory. Only a matching session may
apply compositor layout and sensor-driven rotation or expose session power
actions. The overlay reads Omarchy's theme; standalone uses the built-in
palette. Explicit radio, brightness, and volume controls are still shared
with the host system.

## Gesture transitions

Home and Quick Settings share gesture travel distances between the Wayland
runtime and scene layout. While a finger holds the overlay, motion updates its
position directly. On release it springs from that position; during a Home
page swipe only the adjacent pages are drawn. The runtime keeps the touch
origin mapped through the pull and coalesces overlay updates with Wayland frame
callbacks. Tiles and sliders are not actionable while their overlay is moving.

## Automated Verification Pyramid

1. **Tier 1 (`cargo test --workspace`, `python3 scripts/test-session.py`)**: Desktop entry and icon fixtures, Wayland buffer scaling, scene hit-testing and renderer parity, hardware and theme parsing, and mock SDDM session lifecycle and isolation.
2. **Tier 2 (`./scripts/test-snapshots.sh` / `veilctl snapshot`)**: Deterministic offscreen PNG generation of every scene (`pill`, `home`, `recents`, `shade`, `osk`) in both landscape (`1280×853`) and portrait (`853×1280`) for visual inspection and CI artifacts.
3. **Tier 3 (`./scripts/test-headless-hyprland.sh`)**: Starts a private nested compositor, drives live transitions, and captures its virtual output at 100%, 150%, and 200% in both orientations with `grim`. It verifies that `veilctl mode` does not change compositor settings with or without a daemon. No IPC goes to the parent compositor.

See also: [ROADMAP.md](ROADMAP.md), [DENIAL_COMPARISON.md](DENIAL_COMPARISON.md), [PERFORMANCE_BUDGET.md](PERFORMANCE_BUDGET.md), [HARDWARE_PROFILES.md](HARDWARE_PROFILES.md), and [PORTING_A_COMPOSITOR.md](PORTING_A_COMPOSITOR.md).
