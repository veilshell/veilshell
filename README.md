# VeilShell

**An ultra-lightweight, pure-Rust tablet & touch shell for Wayland.**

VeilShell brings a fluid, touch-first tablet experience (bottom gesture pill,
top quick-settings pull handle, paged `.desktop` home app grid, live recents
window deck, swipe-down quick settings shade, and touch on-screen keyboard) to
**any `wlr-layer-shell` Wayland compositor** without replacing your compositor,
keybindings, or desktop workflow — engineered specifically for **older 5W /
4–8 GB RAM x86 & ARM tablets** (starting with the Microsoft Surface Go 3).

- **Distro-agnostic foundation:** Clean Rust trait boundaries keep core UI,
  physics, rendering, and hardware adapters independent of any single Linux
  distribution.
- **First-class Omarchy & Hyprland compatibility:** Live Laptop ↔ Tablet mode
  switching (both Lua `eval` for Omarchy 4.5+ and legacy `keyword` IPC) with
  native Omarchy Aether `colors.toml` theme sync and zero interference with your
  desktop session.
- **Edge performance budget:** Event-driven `calloop` main loop (`0%` idle CPU
  when dormant), single-pass `wgpu` Vulkan/GLES instanced WGSL SDF rendering
  with zero-copy `Wayland` swapchains + blur-once GPU wallpaper glass cache,
  lazy overlay surface allocation, and automatic zero-`unsafe` CPU SDF fallback.
- **Automated visual & headless UX testing:** Built-in offscreen glassmorphic PNG
  snapshots (`veilctl snapshot`) and a live Hyprland virtual-output + `grim`
  visual harness so contributors and AI agents can visually verify UI changes
  in seconds.

## Workspace Layout

| Crate | Purpose | Status |
|---|---|---|
| [`veilshell-core`](crates/veilshell-core) | Pure-Rust state machine, critically damped spring physics, gesture classifier, `.desktop` parser, orientation/rotation lock, WCAG theme primitives, multi-layer Touch OSK state & XKB keymap | Phase 3 (Done) |
| [`veilshell-theme`](crates/veilshell-theme) | `ThemeProvider` trait + live Omarchy Aether `colors.toml` loader + WCAG AA dark fallback | Phase 0 (Done) |
| [`veilshell-compositor`](crates/veilshell-compositor) | `CompositorBackend` trait + live Hyprland Unix-socket IPC (Lua `eval` + legacy `keyword` fallback, recency-sorted `j/clients` with `stableId`, `j/monitors`, workspace switch, app launch) + `MockBackend` | Phase 3 (Done) |
| [`veilshell-render`](crates/veilshell-render) | Renderer-agnostic glassmorphic `DrawList` + `SceneOutput` hit-testing for all 5 scenes, `wgpu` Vulkan/GLES WGSL SDF backend with shelf-packed GPU glyph/icon/thumbnail atlas & blur-once wallpaper cache, CPU SDF fallback, `ab_glyph` TrueType typography (+ 5×7 HUD fallback), `ARGB8888` encoder, and `PNG` exporter | Phase 3 (Done) |
| [`veilshell-wayland`](crates/veilshell-wayland) | Event-driven `calloop` + `smithay-client-toolkit` multi-surface `wlr-layer-shell` runtime (`BottomPill`, `TopStrip`, `TouchOsk`, reused `Overlay`), zero-copy `wgpu::Surface` presentation + `wl_shm` fallback, `zwp_input_method_v2` + `zwp_virtual_keyboard_v1` OSK, `ext-foreign-toplevel-list-v1` + `ext-image-copy-capture-v1` live Recents thumbnails, output pinning (`--output`), `wl_touch`/`wl_pointer` gestures | Phase 3 (Done) |
| [`veilshell-hw`](crates/veilshell-hw) | `HardwareSnapshot`, `HardwareBackend` + `HardwareController` traits, pure zero-dep Linux `sysfs`/CLI telemetry (`LinuxHardware`, `SensorGuard`), DMI matcher, and declarative `hardware-profiles/*.toml` parser | Phase 2 (Done) |
| [`veilshell-shell`](crates/veilshell-shell) | `ShellState` orchestration, `.desktop` discovery & freedesktop `PNG` icon loader, `veilctl` Unix IPC socket, live hardware/theme watcher, `veilshell` daemon & `veilctl` CLI | Phase 3 (Done) |

Distro integration glue lives under [`adapters/`](adapters/omarchy/README.md),
and device quirk profiles live under [`hardware-profiles/`](hardware-profiles/microsoft-surface-go-3.toml).

## Quick Start

Requires Rust `1.86+` (`cargo`, `rustfmt`, `clippy`).

```bash
# Run Tier 1 unit tests & lints
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Inspect active theme (Omarchy Aether colors.toml when present), live hardware, compositor, & apps
cargo run -p veilshell-shell --bin veilctl -- status
cargo run -p veilshell-shell --bin veilctl -- hw
cargo run -p veilshell-shell --bin veilctl -- theme
cargo run -p veilshell-shell --bin veilctl -- apps

# Generate Tier 2 offscreen glassmorphic PNG snapshots (landscape + portrait)
./scripts/test-snapshots.sh /tmp/veilshell-snapshots

# Launch the adaptive daemon (--policy auto hides in Laptop Mode & wakes on keyboard detach)
cargo run -p veilshell-shell --bin veilshell -- --policy auto
# Or force visible tablet UI for testing (--scene pill|home|recents|shade|osk)
cargo run -p veilshell-shell --bin veilshell -- --policy tablet --scene pill

# Control the running daemon via veilctl Unix socket IPC
veilctl show home        # Open paged .desktop app launcher
veilctl show recents     # Open live Hyprland window deck (tap to focus, X to close)
veilctl show shade       # Open Quick Settings shade
veilctl show osk         # Open multi-layer Touch OSK (auto-opens on text focus in Tablet Mode)
veilctl show pill        # Dismiss overlays back to the bottom gesture pill
veilctl mode toggle      # Toggle Laptop (hidden / 0% CPU) <-> Tablet Mode
veilctl reload-theme     # Hot-reload Aether colors.toml
veilctl quit             # Stop the daemon cleanly

# Run Tier 3 live Hyprland virtual-output + grim visual verification (inside Hyprland)
./scripts/test-headless-hyprland.sh /tmp/veilshell-hypr-test
```

In the on-screen keyboard, tap **TERM** for Escape, Tab and the arrow keys.
Hold Backspace or an arrow to repeat.

For Omarchy users, optional instant theme-hook (`adapters/omarchy/hooks/30-veilshell-theme`)
and `systemd --user` (`adapters/omarchy/systemd/veilshell.service`) integration guides live in [`adapters/omarchy/README.md`](adapters/omarchy/README.md).
For SDDM, see the [standalone session preview](adapters/session/README.md).
It currently requires a physical keyboard for password unlock.

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — Crate boundaries, `calloop` multi-surface runtime, and verification pyramid
- [docs/ROADMAP.md](docs/ROADMAP.md) — Full project plan, resolved architectural decisions, and phased milestones
- [docs/DENIAL_COMPARISON.md](docs/DENIAL_COMPARISON.md) — Feature parity audit vs. Denial (`denialwm/denial`) & standalone boot-session checklist
- [docs/DENIAL_PARITY_SPEC.md](docs/DENIAL_PARITY_SPEC.md) — Pinned upstream visual reference, implementation gaps, and SDDM acceptance gates
- [docs/PERFORMANCE_BUDGET.md](docs/PERFORMANCE_BUDGET.md) — Hard CPU, RAM, and frame-time budgets on 5W tablets
- [docs/HARDWARE_PROFILES.md](docs/HARDWARE_PROFILES.md) — Adding declarative TOML profiles for new tablets
- [docs/PORTING_A_COMPOSITOR.md](docs/PORTING_A_COMPOSITOR.md) — Implementing `CompositorBackend` for Niri, Sway, River, etc.
- [CONTRIBUTING.md](CONTRIBUTING.md) — Development workflow, visual verification, and good first issues

## Acknowledgments & License

VeilShell is licensed under **`GPL-3.0-or-later`** (see [LICENSE](LICENSE)).
Inspired by **Denial** (`denialwm/denial`), **Droidloom**, **Omarchy**, and
**Hyprland** — see [NOTICE.md](NOTICE.md) for full credits.
