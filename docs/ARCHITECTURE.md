# VeilShell Architecture

VeilShell is organized as a modular Rust workspace with strict boundary separation so core shell logic stays portable across Linux distributions, Wayland compositors, and tablet hardware.

```
                   +-----------------------+
                   |    veilshell-shell    |  veilshell daemon + veilctl CLI
                   +-----------+-----------+
         +------------+--------+--------+------------+
         |            |        |        |            |
         v            v        v        v            v
  veilshell-core  veilshell-  veilshell-  veilshell-  veilshell-
  (pure logic,    compositor  wayland     render      theme + hw
   no I/O)        (Hyprland   (wlr-layer  (DrawList,  (Omarchy Aether
                   IPC trait)  + touch)    SDF + PNG)  + profiles)
```

## Crate Responsibilities

- **`veilshell-core`**: Zero-I/O pure Rust. Critically damped harmonic springs, bottom-pill & edge gesture classifier, Laptop↔Tablet mode state machine, orientation/rotation lock, and WCAG contrast color primitives.
- **`veilshell-compositor`**: `CompositorBackend` trait isolating all compositor calls. Ships with `HyprlandBackend` (live Unix socket IPC for `j/clients`, `j/monitors`, `dispatch`, and `[[BATCH]]` keywords) and `MockBackend` for offline tests.
- **`veilshell-theme`**: `ThemeProvider` trait. Reads Omarchy Aether `colors.toml` via `OmarchyAetherProvider` when present, falling back to a built-in WCAG AA dark palette on any other distro.
- **`veilshell-hw`**: `HardwareBackend` trait, `HardwareSnapshot`, and declarative TOML parser for `hardware-profiles/*.toml`.
- **`veilshell-render`**: Renderer-agnostic glassmorphic `DrawList` for all 5 scenes (`pill`, `home`, `recents`, `shade`, `osk`), analytic antialiased CPU SDF rasterizer (`SdfSoftwareRenderer`), premultiplied `ARGB8888` conversion for Wayland `wl_shm`, and `PNG` snapshot export.
- **`veilshell-wayland`**: `smithay-client-toolkit` `wlr-layer-shell` client managing `zwlr_layer_surface_v1` lifecycle, `SlotPool` SHM commits, and `wl_touch` + `wl_pointer` gesture capture.
- **`veilshell-shell`**: Orchestrates `ShellState`, provides the `veilshell` Wayland daemon and the `veilctl` CLI (`status`, `theme`, `mode`, `snapshot`).

## Automated Verification Pyramid

1. **Tier 1 (`cargo test --workspace`)**: Pure logic, IPC JSON parsing, hardware profile parsing, theme parsing, and all 5 scene render + PNG tests.
2. **Tier 2 (`veilctl snapshot`)**: Deterministic offscreen PNG generation of every scene (`pill`, `home`, `recents`, `shade`, `osk`) for visual inspection and regression diffs.
3. **Tier 3 (`scripts/test-headless-hyprland.sh`)**: Spins up an isolated `AQ_BACKENDS=headless Hyprland` compositor, verifies live IPC with `veilctl`, presents real `wlr-layer-shell` surfaces via `veilshell`, and captures real compositor screenshots with `grim`.
