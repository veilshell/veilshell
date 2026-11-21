# Omarchy Adapter for VeilShell

This directory holds optional, thin integration glue for [Omarchy](https://omarchy.org) users so that VeilShell works alongside an unmodified Omarchy + Hyprland installation.

Core VeilShell crates **never** hardcode Omarchy paths or assumptions:
- **Theme sync:** `veilshell-theme` reads `~/.local/state/omarchy/current/theme/colors.toml` via `OmarchyAetherProvider` when available, and falls back to the built-in dark theme on any other Linux distribution.
- **Session integration:** Ships [`veilshell.conf`](veilshell.conf), an optional Hyprland drop-in snippet you can `source` from `~/.config/hypr/hyprland.conf` without modifying `/usr/share/omarchy/`. Phase 2 adds an Aether `theme-set.d` reload hook and `systemd --user` service unit so you keep your existing Omarchy keybindings and desktop shell in Laptop Mode while seamlessly switching to VeilShell in Tablet Mode.
