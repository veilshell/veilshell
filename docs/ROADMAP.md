# VeilShell — Project Plan & Roadmap

> **VeilShell** (`veilshell` / `veilctl`) is a distro-agnostic, ultra-lightweight, pure-Rust tablet & touch shell for Wayland — engineered for edge performance on older 4–8 GB / 5W x86 & ARM tablets while offering first-class, zero-interference compatibility with **Omarchy** and **Hyprland**.

## Core Principles

1. **Foundation First, Adapters Second:** Core crates have zero distro lock-in (Arch, Omarchy, Fedora, Debian, NixOS, postmarketOS). Distro-specific integration (`adapters/omarchy/`) and device quirks (`hardware-profiles/`) stay cleanly separated.
2. **Zero Interference with Desktop Mode:** Runs as a live Laptop ↔ Tablet mode toggle (auto-switching on `SW_TABLET_MODE` or detachable keyboard unplug) *or* as an optional standalone SDDM session. In Laptop Mode, VeilShell is dormant (`0%` CPU / `0` active redraws).
3. **Edge Performance on Older Hardware:** Reference test device is a fanless 5W dual-core tablet (**Microsoft Surface Go 3**, `i3-10100Y`, Intel UHD 615, `1920×1280@60Hz`, `7.7 GiB` RAM).
4. **Contributor- & Agent-Verifiable from Day 1:** Every UI surface renders deterministically offscreen to PNG snapshots (`veilctl snapshot`) and runs on live `wlr-layer-shell` surfaces in Hyprland (`scripts/test-headless-hyprland.sh`) so both human contributors and AI agents can inspect visual changes without physical tablet hardware.

## Resolved Architectural Decisions

- **Organization & Repository:** Hosted under the dedicated **`github.com/veilshell`** organization (`github.com/veilshell/veilshell`) to welcome community maintainers and future companion repos.
- **License & Clean-Room Policy:** **`GPL-3.0-or-later`** (matching `denialwm/denial`), written from scratch in pure Rust with `#![forbid(unsafe_code)]` across all workspace crates and explicit upstream attribution in [NOTICE.md](../NOTICE.md).
- **Renderer Architecture:** Shared renderer-agnostic glassmorphic `DrawList` (`veilshell-render`), backed today by a zero-`unsafe` analytic antialiased CPU SDF rasterizer + PNG snapshot exporter (`SdfSoftwareRenderer`), with a `wgpu` Vulkan GPU shader backend consuming the exact same `DrawList` in Phase 3 so offscreen snapshots and live GPU frames stay pixel-consistent.
- **Minimum Supported Rust Version:** `1.86+` (aligned with `smithay-client-toolkit 0.21`).

## Phased Roadmap

### Phase 0 — Foundation & Dev Harness (Complete)
- [x] Modular 7-crate Cargo workspace (`veilshell-core`, `veilshell-compositor`, `veilshell-hw`, `veilshell-theme`, `veilshell-wayland`, `veilshell-render`, `veilshell-shell`)
- [x] `GPL-3.0-or-later` `LICENSE`, `NOTICE.md`, `CONTRIBUTING.md`, `ARCHITECTURE.md`, `PERFORMANCE_BUDGET.md`, `HARDWARE_PROFILES.md`, `PORTING_A_COMPOSITOR.md`
- [x] `veilshell-core`: critically damped spring physics, bottom-pill gesture classifier, Laptop↔Tablet mode state machine, orientation/rotation lock, WCAG AA theme primitives
- [x] `veilshell-theme`: `ThemeProvider` trait + live Omarchy Aether `colors.toml` parser + dark fallback
- [x] Reference device profile: `hardware-profiles/microsoft-surface-go-3.toml` + zero-dep TOML parser
- [x] GitHub Actions CI (`rustfmt`, `clippy::pedantic -D warnings`, `cargo test --workspace`)

### Phase 1 — Live Hyprland IPC, Glassmorphic SDF Renderer & `wlr-layer-shell` Client (Complete)
- [x] `veilshell-compositor`: live `HyprlandBackend` over Unix sockets (`j/clients`, `j/monitors`, `dispatch`, `[[BATCH]]` layout rules) + real JSON fixture tests
- [x] `veilshell-render`: renderer-agnostic `DrawList`, glassmorphic layouts for all 5 scenes (`pill`, `home`, `recents`, `shade`, `osk`) in landscape & portrait, analytic SDF rasterizer, built-in HUD glyph renderer, `ARGB8888` SHM encoder, and `PNG` exporter
- [x] `veilshell-wayland`: `smithay-client-toolkit 0.21` `wlr-layer-shell` surface client with `SlotPool` SHM commits and `wl_touch` + `wl_pointer` gesture capture
- [x] `veilctl snapshot` CLI + Tier 2 snapshot script (`scripts/test-snapshots.sh`) + Tier 3 Hyprland virtual-output/`grim` harness (`scripts/test-headless-hyprland.sh`)
- [x] Optional Omarchy Hyprland drop-in (`adapters/omarchy/veilshell.conf`)

### Phase 2 — Interactive Touch Shell & Live Hardware Intelligence (Next)
- [ ] Interactive surface state transitions (bottom pill swipe/flick -> Home Grid, Recents Deck, Quick Settings Shade, and tap-to-focus/swipe-to-close on live Hyprland windows)
- [ ] `.desktop` application discovery, pinned favorites, and async icon loading for the Home Grid
- [ ] `veilshell-hw` live Linux backends (`zbus` D-Bus): detachable USB keyboard / `SW_TABLET_MODE` auto-switching, `iio-sensor-proxy` auto-rotation + Rotation Lock, NetworkManager (Wi-Fi), BlueZ, ModemManager (LTE), UPower (battery), PipeWire volume, and sysfs backlight sliders
- [ ] Live `colors.toml` file watcher (`inotify`) + Omarchy `theme-set.d` hook in `adapters/omarchy/`

### Phase 3 — GPU (`wgpu`) Acceleration & Built-In Touch OSK
- [ ] `wgpu` Vulkan/GLES WGSL SDF renderer backend consuming the existing `DrawList` + pre-blurred static wallpaper glass cache (blur once on wallpaper change)
- [ ] TrueType/FreeType font atlas (`swash` / `cosmic-text`) replacing the Phase 1 bitmap HUD font
- [ ] `zwp_input_method_v2` + `zwp_virtual_keyboard_v1` protocol integration in `veilshell-wayland` powering the multi-layout Touch OSK surface
- [ ] Live window thumbnails via `hyprland-toplevel-export-v1` in the Recents Deck

### Phase 4 — Packaging, Multi-Compositor Ports & `v0.1.0` Release
- [ ] Arch Linux / AUR `PKGBUILD` (`packaging/arch/PKGBUILD`) + optional `systemd --user` service & SDDM session entry
- [ ] Additional compositor backends (`Niri`, `Sway`) & community `hardware-profiles/*.toml`
- [ ] Tier 3 frame-time & RSS performance budget assertions in CI
- [ ] First tagged **`v0.1.0`** release
