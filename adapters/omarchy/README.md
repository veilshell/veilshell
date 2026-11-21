# Omarchy Adapter for VeilShell

This directory holds optional, thin integration glue for [Omarchy](https://omarchy.org) users so that VeilShell works alongside an unmodified Omarchy + Hyprland installation.

Core VeilShell crates **never** hardcode Omarchy paths or assumptions:

- **Theme sync:** `veilshell-theme` reads `~/.local/state/omarchy/current/theme/colors.toml` via `OmarchyAetherProvider` when available, and falls back to the built-in dark theme on any other Linux distribution.
- **Compositor IPC:** `HyprlandBackend` automatically speaks Omarchy 4.5+ Lua `eval` (`hl.config` / `hl.monitor`) and falls back to legacy `.conf` `keyword` commands on standard Hyprland setups.
- **Session integration:**
  - **Omarchy 4.5+ (Lua Hyprland):** Use [`veilshell.lua`](veilshell.lua) (sets `hl.layer_rule` `no_anim` for `veilshell` and shows optional `o.bind` keybindings for `veilctl show home/recents/shade` and `veilctl mode toggle`).
  - **Legacy Hyprland `.conf`:** Use [`veilshell.conf`](veilshell.conf).
  - **Phase 2 Sprint 2B:** Will add an Aether `theme-set.d` live reload hook (`veilctl reload-theme`) and optional `systemd --user` service unit.
