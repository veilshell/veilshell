# VeilShell

**An ultra-lightweight, pure-Rust tablet & touch shell for Wayland.**

VeilShell brings a fluid, touch-first tablet experience (bottom gesture pill,
paged home app grid, recents card deck, swipe-down quick settings shade, and
touch on-screen keyboard) to **any `wlr-layer-shell` Wayland compositor** without
replacing your compositor, keybindings, or desktop workflow — engineered
specifically for **older 5W / 4–8 GB RAM x86 & ARM tablets** (starting with the
Microsoft Surface Go 3).

- **Distro-agnostic foundation:** Clean Rust trait boundaries keep core UI,
  physics, rendering, and hardware adapters independent of any single Linux
  distribution.
- **First-class Omarchy & Hyprland compatibility:** Live Laptop ↔ Tablet mode
  switching (auto-triggered on detachable keyboard unplug) with native Omarchy
  Aether `colors.toml` theme sync and zero interference with your desktop session.
- **Edge performance budget:** `0%` idle CPU/GPU when dormant, `< 40 MiB` active
  RSS, `<= 16.6 ms` animation frame budget, and `#![forbid(unsafe_code)]` across
  the entire workspace.
- **Automated visual & headless UX testing:** Built-in offscreen glassmorphic PNG
  snapshots (`veilctl snapshot`) and a Hyprland virtual-output + `grim` visual
  harness so contributors and AI agents can visually verify UI changes in seconds.

## Workspace Layout

| Crate | Purpose | Status |
|---|---|---|
| [`veilshell-core`](crates/veilshell-core) | Pure-Rust state machine, critically damped spring physics, gesture classifier, orientation/rotation lock, WCAG theme primitives | Phase 0 (Done) |
| [`veilshell-theme`](crates/veilshell-theme) | `ThemeProvider` trait + live Omarchy Aether `colors.toml` loader + WCAG AA dark fallback | Phase 0 (Done) |
| [`veilshell-compositor`](crates/veilshell-compositor) | `CompositorBackend` trait + live Hyprland Unix-socket IPC (`j/clients`, `j/monitors`, `dispatch`, `[[BATCH]]`) + `MockBackend` | Phase 1 (Done) |
| [`veilshell-render`](crates/veilshell-render) | Renderer-agnostic glassmorphic `DrawList` for all 5 scenes, analytic antialiased SDF rasterizer, `ARGB8888` encoder, and `PNG` snapshot exporter (`wgpu` Vulkan backend planned on the same `DrawList`) | Phase 1 (Done) |
| [`veilshell-wayland`](crates/veilshell-wayland) | `smithay-client-toolkit` `wlr-layer-shell` client (`SlotPool` SHM buffers + `wl_touch`/`wl_pointer` gesture capture; `input-method-v2`/`virtual-keyboard-v1` in Phase 3) | Phase 1 (Done) |
| [`veilshell-hw`](crates/veilshell-hw) | `HardwareSnapshot`, `HardwareBackend` trait, and declarative `hardware-profiles/*.toml` parser (live `zbus` D-Bus backends in Phase 2) | Phase 0–1 (In Progress) |
| [`veilshell-shell`](crates/veilshell-shell) | `ShellState` orchestration and `veilshell` / `veilctl` binaries | Phase 1 (Done) |

Distro integration glue lives under [`adapters/`](adapters/omarchy/README.md),
and device quirk profiles live under [`hardware-profiles/`](hardware-profiles/microsoft-surface-go-3.toml).

## Quick Start

Requires Rust `1.86+` (`cargo`, `rustfmt`, `clippy`).

```bash
# Run Tier 1 unit tests & lints
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Inspect active theme (auto-loads Omarchy Aether colors.toml when present) & compositor status
cargo run -p veilshell-shell --bin veilctl -- status
cargo run -p veilshell-shell --bin veilctl -- theme

# Generate Tier 2 offscreen glassmorphic PNG snapshots (landscape + portrait)
./scripts/test-snapshots.sh /tmp/veilshell-snapshots

# Launch a live wlr-layer-shell surface inside a Wayland session
cargo run -p veilshell-shell --bin veilshell -- --scene pill

# Run Tier 3 Hyprland virtual-output + grim visual verification (inside Hyprland)
./scripts/test-headless-hyprland.sh /tmp/veilshell-hypr-test
```

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — Crate boundaries, data flow, and verification pyramid
- [docs/ROADMAP.md](docs/ROADMAP.md) — Full project plan, resolved architectural decisions, and phased milestones
- [docs/PERFORMANCE_BUDGET.md](docs/PERFORMANCE_BUDGET.md) — Hard CPU, RAM, and frame-time budgets on 5W tablets
- [docs/HARDWARE_PROFILES.md](docs/HARDWARE_PROFILES.md) — Adding declarative TOML profiles for new tablets
- [docs/PORTING_A_COMPOSITOR.md](docs/PORTING_A_COMPOSITOR.md) — Implementing `CompositorBackend` for Niri, Sway, River, etc.
- [CONTRIBUTING.md](CONTRIBUTING.md) — Development workflow, visual verification, and good first issues

## Acknowledgments & License

VeilShell is licensed under **`GPL-3.0-or-later`** (see [LICENSE](LICENSE)).
Inspired by **Denial** (`denialwm/denial`), **Droidloom**, **Omarchy**, and
**Hyprland** — see [NOTICE.md](NOTICE.md) for full credits.
