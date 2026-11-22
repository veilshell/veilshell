# Contributing to VeilShell

Welcome! VeilShell is built so a new contributor can clone the repository, run
`cargo test --workspace` in seconds, and contribute to a single focused crate
without needing to master Wayland internals.

## Development Workflow

All workspace crates enforce `#![forbid(unsafe_code)]` and `clippy::pedantic`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Visual & Wayland UX Verification

Every UI or rendering change should be verified with the built-in snapshot and
compositor harnesses:

1. **Tier 2 — Offscreen PNG snapshots** (works on any machine or headless CI):
   ```bash
   ./scripts/test-snapshots.sh /tmp/veilshell-snapshots
   # Or render a single scene/resolution:
   cargo run -p veilshell-shell --bin veilctl -- snapshot \
       --scene home --width 1280 --height 853 --out /tmp/home.png
   ```
   Verify landscape (`1280×853`), portrait (`853×1280`), layout balance, and WCAG AA contrast.
2. **Tier 3 — Nested Hyprland + `grim` visual harness** (requires `jq` and a parent Wayland compositor compatible with Hyprland's nested backend):
   ```bash
   ./scripts/test-headless-hyprland.sh /tmp/veilshell-hypr-test
   ```
   Starts a private compositor with a temporary config and socket. It only
   changes a virtual output inside that compositor, never in the parent
   desktop. It checks `veilctl` mode isolation and captures the live surfaces
   at 100%, 150%, and 200% in portrait and landscape. Inspect these captures
   for sharp text and real installed app icons; set
   `VEILSHELL_TEST_RENDERER=software` or `wgpu` to test a specific backend.

## Great Starting Areas ("Good First Issues")

- **Hardware profiles (`hardware-profiles/`):** Add a TOML quirk file for your
  tablet (see [docs/HARDWARE_PROFILES.md](docs/HARDWARE_PROFILES.md)).
- **Compositor backends (`crates/veilshell-compositor/`):** Implement
  `CompositorBackend` for Niri, Sway, River, or labwc (see
  [docs/PORTING_A_COMPOSITOR.md](docs/PORTING_A_COMPOSITOR.md)).
- **Theme providers (`crates/veilshell-theme/`):** Add a `ThemeProvider` for
  XDG Desktop Portal, Pywal, Matugen, or Stylix.
- **Quick Settings tiles & OSK layouts (`crates/veilshell-render/` & `crates/veilshell-shell/`):**
  Add new tiles, international keyboard layouts, or `.desktop` app icon loaders.

## Licensing

All contributions are licensed under **`GPL-3.0-or-later`** (see [LICENSE](LICENSE)).
Keep third-party inspirations credited in [NOTICE.md](NOTICE.md).
