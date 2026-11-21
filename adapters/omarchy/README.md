# Omarchy Adapter for VeilShell

This directory holds optional, thin integration glue for [Omarchy](https://omarchy.org) users so that VeilShell works alongside an unmodified Omarchy + Hyprland installation.

Core VeilShell crates **never** hardcode Omarchy paths or assumptions:
- **Theme sync:** `veilshell-theme` reads `~/.local/state/omarchy/current/theme/colors.toml` via `OmarchyAetherProvider` when available, and falls back to the built-in dark theme on any other Linux distribution.
- **Session integration (Phase 1+):** Will provide an optional `hyprland.conf` drop-in snippet and `systemd --user` service unit so users can keep their existing Omarchy keybindings, Waybar/Omarchy Shell in Laptop Mode, and seamlessly switch to VeilShell in Tablet Mode.
