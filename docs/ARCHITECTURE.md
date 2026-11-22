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
- **`veilshell-hw`**: `HardwareBackend` and `HardwareController` traits, `HardwareSnapshot`, DMI auto-matching (`detect_dmi_device`, `load_matching_profile`) over user/built-in `hardware-profiles/*.toml`, pure Linux `sysfs` readers (filtering peripheral `scope=Device` stylus batteries from system batteries and matching USB Type Cover IDs), standard CLI backends (`nmcli`, `bluetoothctl`, `wpctl`, `brightnessctl`, `hyprsunset`/`omarchy-toggle-nightlight`), and `SensorGuard` wrapping `monitor-sensor --accel` with clean child reaping on drop.
- **`veilshell-render`**: Renderer-agnostic glassmorphic `DrawList` + interactive hit-test regions (`SceneOutput`) for all 5 scenes (`pill`, `home`, `recents`, `shade`, `osk`) and `TopStrip`, single-pass instanced `wgpu` Vulkan/GLES WGSL SDF backend (`WgpuRenderer`) with a shelf-packed GPU glyph/icon/thumbnail atlas and blur-once GPU static wallpaper frosted-glass cache, cached-backdrop CPU analytic SDF fallback (`SdfSoftwareRenderer`), `ActiveRenderer` (`auto`/`wgpu`/`software`), `ab_glyph` system TrueType typography (with built-in 5×7 HUD fallback), SDF-clipped `RgbaImage`/`ImageQuad` support, premultiplied `ARGB8888` encoder, and `PNG` snapshot export.
- **`veilshell-wayland`**: Event-driven `calloop` + `WaylandSource` runtime wrapping `smithay-client-toolkit 0.21` (`wlr-layer-shell`). Manages `BottomPill`, `TopStrip`, `TouchOsk`, and reused full-screen `Overlay` surfaces (dropping the bottom pill cleanly when `TouchOsk` is active) with zero-copy `wgpu::Surface` Vulkan/GLES swapchains (isolated in a single `raw-window-handle` bridge with automatic `wl_shm` `SlotPool` fallback, supporting `initial_visible: false` sleep in Laptop Mode for `0%` idle CPU and syncing logical dimensions on `wl_output`/configure rotation), drives built-in Touch OSK text input & auto-show via `zwp_input_method_v2` + `zwp_virtual_keyboard_v1`, captures live Recents window thumbnails via `ext-foreign-toplevel-list-v1` + `ext-image-copy-capture-v1`, binds optionally to a named `wl_output` (`--output`), and routes `wl_touch`/`wl_pointer` gestures and hit-tests back via `ShellActionHandler`.
- **`veilshell-shell`**: Discovers system/user `.desktop` applications and resolves freedesktop `PNG` app icons (`resolve_icon_path`, `load_app_icon`), runs the `$XDG_RUNTIME_DIR/veilshell.sock` Unix control listener (`show`, `status`, `wallpaper`, `mode`, `reload-theme`, `quit`), polls USB Type Cover presence, `sysfs` battery/backlight, and `colors.toml` mtime on a 1.5s lightweight watcher alongside `SensorGuard`, and ships `veilshell` (`--policy auto|laptop|tablet`, `--renderer auto|wgpu|software`, `--wallpaper`) + `veilctl` (`status`, `hw`, `show`, `mode`, `wallpaper`, `reload-theme`, `apps`, `theme`, `snapshot`).

## Automated Verification Pyramid

1. **Tier 1 (`cargo test --workspace`)**: Pure logic, `.desktop` parsing & icon path resolution, Touch OSK state & evdev keymap validation, Hyprland IPC JSON parsing (`stableId` + sorting), hit-testing precedence across all scenes & OSK layers, Linux hardware CLI/sysfs parser & profile matching tests, theme parsing, `ab_glyph` TrueType + `RgbaImage` SHM conversion/resize tests, `SdfSoftwareRenderer` PNG tests, and `wgpu` vs CPU pixel-parity (including icons, thumbnails & TrueType glyphs) + blur-once wallpaper tests (**50 passing tests**, verified with default `wgpu` and `--no-default-features`).
2. **Tier 2 (`./scripts/test-snapshots.sh` / `veilctl snapshot`)**: Deterministic offscreen PNG generation of every scene (`pill`, `home`, `recents`, `shade`, `osk`) in both landscape (`1280×853`) and portrait (`853×1280`) for visual inspection and CI artifacts.
3. **Tier 3 (`./scripts/test-headless-hyprland.sh`)**: Spins up a temporary isolated virtual output (`hyprctl output create headless`), configures `hl.layer_rule` `no_anim` on `"veilshell"`, runs a single persistent `veilshell --output HEADLESS-*` daemon, drives real transitions via `veilctl`, captures live compositor screenshots with `grim -o`, verifies non-blank output, tests `veilctl mode laptop/tablet`, and cleans up cleanly.

See also: [ROADMAP.md](ROADMAP.md), [PERFORMANCE_BUDGET.md](PERFORMANCE_BUDGET.md), [HARDWARE_PROFILES.md](HARDWARE_PROFILES.md), and [PORTING_A_COMPOSITOR.md](PORTING_A_COMPOSITOR.md).
