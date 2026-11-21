# Contributing to VeilShell

Welcome! VeilShell is built so a new contributor can clone the repository, run
`cargo test --workspace` in seconds, and contribute to a single focused crate
without needing to master Wayland internals.

## Development Workflow

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Visual & Headless UX Verification

Once `veilshell-render` (Phase 1) is live, every visual change must be verified
using:

1. **Offscreen GPU snapshots:** `veilctl snapshot --scene all --out /tmp/shots/`
   (checks landscape, portrait, dark/light themes, and contrast).
2. **Headless Hyprland E2E harness:** `tests/e2e_hyprland/` (runs isolated
   `AQ_BACKENDS=headless` Hyprland + `grim` captures + RSS/frame-time assertions).

## Great Starting Areas ("Good First Issues")

- **Hardware profiles (`hardware-profiles/`):** Add a TOML quirk file for your
  tablet (detachable keyboard USB ID, preferred scale, rotation mount notes).
- **Compositor backends (`crates/veilshell-compositor/`):** Implement
  `CompositorBackend` for Niri, Sway, or River.
- **Theme providers (`crates/veilshell-theme/`):** Add a provider for Pywal,
  Matugen, Stylix, or GNOME/KDE color schemes.
- **Quick Settings tiles & OSK layouts (`crates/veilshell-shell/`):** Add new
  tiles (Night Light, Hotspot, Power Profile) or international keyboard layouts.

## Licensing

All contributions are licensed under **`GPL-3.0-or-later`** (see [LICENSE](LICENSE)).
Keep third-party inspirations credited in [NOTICE.md](NOTICE.md).
