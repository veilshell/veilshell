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

- **`veilshell-core`**: Zero-I/O pure Rust. Critically damped harmonic springs, bottom-pill & edge gesture classifier, Laptop↔Tablet mode state machine, orientation/rotation lock, WCAG contrast color primitives, and freedesktop `.desktop` parser (`DesktopApp`, field-code stripping, badge formatting).
- **`veilshell-compositor`**: `CompositorBackend` trait isolating all compositor calls. Ships with `HyprlandBackend` (live Unix-socket IPC supporting recency-sorted `j/clients` filtered to mapped windows, `j/monitors`, workspace switching, app launch `exec`, and dual Omarchy 4.5+ Lua `eval` with legacy `keyword` fallback) plus `MockBackend` for offline tests.
- **`veilshell-theme`**: `ThemeProvider` trait. Reads Omarchy Aether `colors.toml` via `OmarchyAetherProvider` when present, falling back to a built-in WCAG AA dark palette on any other distro.
- **`veilshell-hw`**: `HardwareBackend` trait, `HardwareSnapshot`, and declarative TOML parser for `hardware-profiles/*.toml`. Sprint 2B adds live Linux D-Bus backends (`iio-sensor-proxy`, NetworkManager, BlueZ, ModemManager, UPower, PipeWire).
- **`veilshell-render`**: Renderer-agnostic glassmorphic `DrawList` + interactive hit-test regions (`SceneOutput`) for all 5 scenes (`pill`, `home`, `recents`, `shade`, `osk`) and `TopStrip`, single-pass instanced `wgpu` Vulkan/GLES WGSL SDF backend (`WgpuRenderer`) with a blur-once GPU static wallpaper frosted-glass cache, cached-backdrop CPU analytic SDF fallback (`SdfSoftwareRenderer`), `ActiveRenderer` (`auto`/`wgpu`/`software`), built-in HUD glyph renderer, premultiplied `ARGB8888` encoder, and `PNG` snapshot export.
- **`veilshell-wayland`**: Event-driven `calloop` + `WaylandSource` runtime wrapping `smithay-client-toolkit 0.21` (`wlr-layer-shell`). Lazily manages `BottomPill`, `TopStrip`, and full-screen `Overlay` surfaces with zero-copy `wgpu::Surface` Vulkan/GLES swapchains (isolated in a single `raw-window-handle` bridge with automatic `wl_shm` `SlotPool` fallback, dropping overlay swapchains/pools immediately on dismissal and hiding all surfaces in Laptop Mode for `0%` idle CPU), binds optionally to a named `wl_output` (`--output`), and routes `wl_touch`/`wl_pointer` gestures and hit-tests back via `ShellActionHandler`.
- **`veilshell-shell`**: Discovers system/user `.desktop` applications, runs the `$XDG_RUNTIME_DIR/veilshell.sock` Unix control listener (`show`, `status`, `mode`, `reload-theme`, `quit`), and ships `veilshell` + `veilctl`.

## Automated Verification Pyramid

1. **Tier 1 (`cargo test --workspace`)**: Pure logic, `.desktop` parsing, Hyprland IPC JSON parsing & sorting, hit-testing precedence, hardware profile parsing, theme parsing, and all 5 scene render + PNG tests (**31 passing tests**).
2. **Tier 2 (`./scripts/test-snapshots.sh` / `veilctl snapshot`)**: Deterministic offscreen PNG generation of every scene (`pill`, `home`, `recents`, `shade`, `osk`) in both landscape (`1280×853`) and portrait (`853×1280`) for visual inspection and CI artifacts.
3. **Tier 3 (`./scripts/test-headless-hyprland.sh`)**: Spins up a temporary isolated virtual output (`hyprctl output create headless`), configures `hl.layer_rule` `no_anim` on `"veilshell"`, runs a single persistent `veilshell --output HEADLESS-*` daemon, drives real transitions via `veilctl`, captures live compositor screenshots with `grim -o`, verifies non-blank output, tests `veilctl mode laptop/tablet`, and cleans up cleanly.

See also: [ROADMAP.md](ROADMAP.md), [PERFORMANCE_BUDGET.md](PERFORMANCE_BUDGET.md), [HARDWARE_PROFILES.md](HARDWARE_PROFILES.md), and [PORTING_A_COMPOSITOR.md](PORTING_A_COMPOSITOR.md).
