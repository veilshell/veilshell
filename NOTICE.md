# Notice & Acknowledgments

VeilShell is an independent, clean-room Rust implementation licensed under the
**GNU General Public License v3.0 or later (`GPL-3.0-or-later`)**.

We gratefully acknowledge the open-source projects whose design, engineering,
and ideas inspired VeilShell:

- **Denial (`denialwm/denial`) & Droidloom (`denialwm/droidloom`)** —
  Licensed under `GPL-3.0-or-later`. Denial's touch-first mobile shell,
  bottom liquid gesture pill, pre-blurred wallpaper glass optimization, and
  Linux-tablet ergonomics directly inspired VeilShell's UX direction and
  performance philosophy.
- **Omarchy** — Desktop workflow, Aether live theming contract (`colors.toml`
  and `theme-set.d`), and Hyprland conventions that VeilShell integrates with
  through its optional `adapters/omarchy/` layer.
- **Hyprland** — Wayland compositor, IPC sockets, and `hyprland-toplevel-export-v1`
  support powering VeilShell's Day-1 compositor backend.
- **Smithay (`smithay-client-toolkit`) & `wgpu`** — Rust Wayland client and
  portable GPU rendering foundations.
- **freedesktop.org / `iio-sensor-proxy` / `NetworkManager` / `ModemManager` / `PipeWire`** —
  Standard Linux D-Bus hardware interfaces that keep VeilShell distro-agnostic.

If any future file in this repository adapts protocol XML or derived code from
an upstream GPL-3.0-or-later project, its original copyright header and license
notice must be preserved verbatim.
