# VeilShell Architecture

VeilShell is a modular, pure-Rust Wayland tablet & touch shell designed around
strict crate boundaries so core UX logic stays distro- and compositor-agnostic.

## Crate Dependency Graph

```text
veilshell-core  (zero I/O, zero GPU, zero Wayland)
  ├── veilshell-theme       (ThemeProvider: Aether colors.toml, XDG fallback)
  ├── veilshell-compositor  (CompositorBackend: Hyprland, Mock, future Niri/Sway)
  ├── veilshell-hw          (udev keyboard/SW_TABLET_MODE, iio-sensor-proxy, D-Bus)
  ├── veilshell-wayland     (wlr-layer-shell, touch events, IME/virtual-keyboard)
  └── veilshell-render      (wgpu 2D/SDF renderer, pre-blurred glass, PNG snapshots)
        └── veilshell-shell (Pill, Home, Recents, Shade, OSK, `veilshell`, `veilctl`)
```

## Design Rules

1. **`veilshell-core` has zero external runtime dependencies.** Every spring
   integrator, gesture classifier, mode transition, orientation transform, and
   WCAG contrast check is deterministically tested via `cargo test`.
2. **Distro integrations stay in `adapters/`.** Core crates never hardcode
   Omarchy paths as mandatory requirements; `adapters/omarchy/` supplies hooks,
   optional Hyprland Lua fragments, and packaging.
3. **Zero-idle rendering.** No busy loops or fixed redraw timers when dormant.
   Redraws are driven strictly by input events, state transitions, and Wayland
   `wl_surface.frame` callbacks.
4. **First-class visual testability.** `veilshell-render` supports rendering
   any scene graph into an offscreen RGBA texture -> PNG so agents and CI can
   inspect real pixels without a physical display.
