# VeilShell

**An ultra-lightweight, 60 FPS pure-Rust tablet & touch shell for Wayland.**

VeilShell brings a fluid, touch-first tablet experience (bottom gesture pill,
paged home app grid, live recents deck, swipe-down quick settings, and built-in
on-screen keyboard) to **any Wayland compositor** without replacing your
compositor, keybindings, or desktop workflow — engineered specifically to run
smoothly on older **5W / 4–8 GB RAM x86 & ARM tablets** (such as the Microsoft
Surface Go series).

- **Distro-agnostic foundation:** Clean Rust trait boundaries keep core UI,
  physics, rendering, and hardware adapters independent of any single distro.
- **First-class Omarchy & Hyprland compatibility:** Live Laptop ↔ Tablet mode
  switching (auto-triggered on detachable keyboard unplug) with native Aether
  `colors.toml` theme sync and zero interference with your desktop session.
- **Edge performance budget:** `0%` idle CPU/GPU when dormant, `< 40 MB` active
  RSS, `<= 16.6 ms` animation frame budget using pre-blurred static wallpaper
  glass instead of expensive per-frame live blur.
- **Automated visual & headless UX testing:** Built-in offscreen PNG snapshots
  (`veilctl snapshot`) and an isolated headless Hyprland E2E harness so both
  contributors and AI agents can visually verify UI changes in seconds.

## Workspace Layout

| Crate | Purpose |
|---|---|
| `veilshell-core` | Pure-Rust state machine, spring physics, gesture classifier, palette & WCAG contrast checks |
| `veilshell-compositor` | `CompositorBackend` trait + Hyprland & mock backends (Niri/Sway-ready) |
| `veilshell-hw` | Standard Linux D-Bus/udev hardware layer (`iio-sensor-proxy`, NM, BlueZ, ModemManager, UPower, PipeWire) |
| `veilshell-theme` | `ThemeProvider` trait (generic XDG + Omarchy Aether `colors.toml` loader) |
| `veilshell-wayland` | `wlr-layer-shell`, multi-touch tracking, `input-method-v2` & `virtual-keyboard-v1` |
| `veilshell-render` | `wgpu` 2D/glass renderer + offscreen PNG snapshot engine |
| `veilshell-shell` | UI surfaces (Pill, Home, Recents, Shade, OSK) and `veilshell` / `veilctl` binaries |

Distro glue lives under `adapters/` (starting with `adapters/omarchy/`), and
device quirks live under `hardware-profiles/`.

## Quick Start

```bash
cargo test --workspace
cargo run -p veilshell-shell --bin veilctl -- status
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md),
[docs/PERFORMANCE_BUDGET.md](docs/PERFORMANCE_BUDGET.md), and
[CONTRIBUTING.md](CONTRIBUTING.md) to get involved.

## Acknowledgments & License

VeilShell is licensed under **`GPL-3.0-or-later`** (see [LICENSE](LICENSE)).
Inspired by **Denial** (`denialwm/denial`), **Droidloom**, **Omarchy**, and
**Hyprland** — see [NOTICE.md](NOTICE.md) for full credits.
