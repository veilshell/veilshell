# VeilShell — Project Plan & Roadmap

> **VeilShell** (`veilshell` / `veilctl`) is a distro-agnostic, ultra-lightweight, pure-Rust tablet & touch shell for Wayland — engineered for edge performance on older 4–8 GB / 5W x86 & ARM tablets while offering first-class, zero-interference compatibility with **Omarchy** and **Hyprland**.

## Core Principles

1. **Foundation First, Adapters Second:** Core crates have zero distro lock-in (Arch, Omarchy, Fedora, Debian, NixOS, postmarketOS). Distro-specific integration (`adapters/omarchy/`) and device quirks (`hardware-profiles/`) stay cleanly separated.
2. **Zero Interference with Desktop Mode:** Runs as a live Laptop ↔ Tablet mode toggle (auto-switching on `SW_TABLET_MODE` or detachable keyboard unplug) *or* as an optional standalone SDDM session. In Laptop Mode, VeilShell hides all surfaces and sleeps cleanly on `calloop` (`0%` CPU / `0` active redraws).
3. **Edge Performance on Older Hardware:** Reference test device is a fanless 5W dual-core tablet (**Microsoft Surface Go 3**, `i3-10100Y`, Intel UHD 615, `1920×1280@60Hz`, `7.7 GiB` RAM).
4. **Contributor- & Agent-Verifiable from Day 1:** Every UI surface renders deterministically offscreen to PNG snapshots (`veilctl snapshot`) and runs on live `wlr-layer-shell` surfaces in Hyprland (`scripts/test-headless-hyprland.sh`) so both human contributors and AI agents can inspect visual changes without physical tablet hardware.

## Resolved Architectural Decisions

- **Organization & Repository:** Hosted under the dedicated **`github.com/veilshell`** organization (`https://github.com/veilshell/veilshell`) to welcome community maintainers and future companion repos.
- **License & Clean-Room Policy:** **`GPL-3.0-or-later`** (matching `denialwm/denial`), written from scratch in pure Rust with `#![forbid(unsafe_code)]` across 6 of 7 workspace crates (and `unsafe_code = "deny"` in `veilshell-wayland` permitting only a single isolated `raw-window-handle` `wgpu::Surface` bridge) and explicit upstream attribution in [NOTICE.md](../NOTICE.md).
- **Renderer Architecture:** Shared renderer-agnostic glassmorphic `DrawList` + hit-test `SceneOutput` (`veilshell-render`), backed by both a hardware-accelerated `wgpu` Vulkan/GLES WGSL instanced SDF renderer (`WgpuRenderer`, zero-copy `wgpu::Surface` Wayland presentation + blur-once GPU static wallpaper glass cache) and a zero-`unsafe` CPU analytic SDF fallback (`SdfSoftwareRenderer`), selectable via `--renderer auto|wgpu|software` with automated GPU-vs-CPU pixel-parity tests.
- **Minimum Supported Rust Version:** `1.86+` (aligned with `smithay-client-toolkit 0.21`).

## Phased Roadmap

### Phase 0 — Foundation & Dev Harness (Complete)
- [x] Modular 7-crate Cargo workspace (`veilshell-core`, `veilshell-compositor`, `veilshell-hw`, `veilshell-theme`, `veilshell-wayland`, `veilshell-render`, `veilshell-shell`)
- [x] `GPL-3.0-or-later` `LICENSE`, `NOTICE.md`, `CONTRIBUTING.md`, `ARCHITECTURE.md`, `PERFORMANCE_BUDGET.md`, `HARDWARE_PROFILES.md`, `PORTING_A_COMPOSITOR.md`
- [x] `veilshell-core`: critically damped spring physics, bottom-pill gesture classifier, Laptop↔Tablet mode state machine, orientation/rotation lock, WCAG AA theme primitives
- [x] `veilshell-theme`: `ThemeProvider` trait + live Omarchy Aether `colors.toml` parser + dark fallback
- [x] Reference device profile: `hardware-profiles/microsoft-surface-go-3.toml` + zero-dep TOML parser
- [x] GitHub Actions CI (`rustfmt`, `clippy::pedantic -D warnings`, `cargo test --workspace`, Tier 2 visual snapshots)

### Phase 1 — Live Hyprland IPC, Glassmorphic SDF Renderer & `wlr-layer-shell` Client (Complete)
- [x] `veilshell-compositor`: live `HyprlandBackend` over Unix sockets (`j/clients`, `j/monitors`, `dispatch`) + real JSON fixture tests
- [x] `veilshell-render`: renderer-agnostic `DrawList`, glassmorphic layouts for all 5 scenes (`pill`, `home`, `recents`, `shade`, `osk`) in landscape & portrait, analytic SDF rasterizer, built-in HUD glyph renderer, `ARGB8888` SHM encoder, and `PNG` exporter
- [x] `veilshell-wayland`: `smithay-client-toolkit 0.21` `wlr-layer-shell` surface client with `SlotPool` SHM commits and `wl_touch` + `wl_pointer` gesture capture
- [x] `veilctl snapshot` CLI + Tier 2 snapshot script (`scripts/test-snapshots.sh`) + Tier 3 Hyprland headless-output + `grim` harness (`scripts/test-headless-hyprland.sh`)

### Phase 2 — Interactive Touch Shell & Live Hardware Intelligence

#### Sprint 2A — Interactive Multi-Surface Shell, `.desktop` Launcher & `veilctl` Daemon IPC (Complete)
- [x] Event-driven `calloop` + `WaylandSource` daemon managing persistent `BottomPill` + `TopStrip` and lazy full-screen `Overlay` (`Layer::Overlay`, output pinning via `--output`)
- [x] Interactive hit-testing (`SceneOutput::hit_test`) for Home tiles/pagination dots, Recents tap-to-focus & `X`-to-close, Quick Settings toggles & draggable sliders, and spring-animated touch transitions
- [x] Pure `.desktop` parser (`veilshell-core::apps`) + user/system application discovery (`veilctl apps`) launching via `HyprlandBackend::launch_command`
- [x] Dual Omarchy 4.5+ Lua `eval` (`hl.config` / `hl.monitor` / `hl.layer_rule`) and legacy `.conf` `keyword` Hyprland compatibility + `adapters/omarchy/veilshell.lua`
- [x] Live Unix control socket (`$XDG_RUNTIME_DIR/veilshell.sock`) powering `veilctl show`, `veilctl mode`, `veilctl status`, `veilctl reload-theme`, and `veilctl quit`
- [x] Cached-backdrop fast SDF rasterizer & verified non-blank Tier 3 live Hyprland screenshots (`home`, `shade`, `recents`, `pill`)

#### Sprint 2B — Live Hardware Intelligence & Omarchy Theme Auto-Reload (Next)
- [ ] `veilshell-hw` live Linux backends (`zbus` D-Bus & sysfs/udev): detachable USB keyboard / `SW_TABLET_MODE` auto-switching (matching `hardware-profiles/*.toml`), `iio-sensor-proxy` auto-rotation + Rotation Lock
- [ ] Live Quick Settings hardware actions: NetworkManager (Wi-Fi), BlueZ, ModemManager (LTE), UPower (battery/charging), PipeWire volume, Night Light, and sysfs backlight sliders
- [ ] Live Aether `colors.toml` file watcher (`inotify`) + Omarchy `adapters/omarchy/hooks/30-veilshell-theme` reload hook + optional `systemd --user` unit

### Phase 3 — GPU (`wgpu`) Acceleration & Built-In Touch OSK
- [x] `wgpu` Vulkan/GLES WGSL SDF renderer backend consuming the existing `DrawList` + pre-blurred static wallpaper glass cache (blur once on wallpaper change) + zero-copy `wgpu::Surface` Wayland presentation & automatic `SdfSoftwareRenderer` fallback
- [ ] TrueType/FreeType font atlas (`swash` / `cosmic-text`) and app icon theme loader replacing the bitmap HUD font & initials badges
- [ ] `zwp_input_method_v2` + `zwp_virtual_keyboard_v1` protocol integration in `veilshell-wayland` powering the multi-layout Touch OSK surface
- [ ] Live window thumbnails via `hyprland-toplevel-export-v1` in the Recents Deck

### Phase 4 — Packaging, Multi-Compositor Ports & `v0.1.0` Release
- [ ] Arch Linux / AUR `PKGBUILD` (`packaging/arch/PKGBUILD`) + optional standalone SDDM session entry
- [ ] Additional compositor backends (`Niri`, `Sway`) & community `hardware-profiles/*.toml`
- [ ] Tier 3 frame-time & RSS performance budget assertions in CI
- [ ] First tagged **`v0.1.0`** release
