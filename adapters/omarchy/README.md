# Omarchy Adapter for VeilShell

This directory holds optional, thin integration glue for [Omarchy](https://omarchy.org) users so that VeilShell works alongside an unmodified Omarchy + Hyprland installation.

Core VeilShell crates **never** hardcode Omarchy paths or assumptions:

- **Theme sync:** `veilshell-theme` reads `~/.local/state/omarchy/current/theme/colors.toml` via `OmarchyAetherProvider` when available, and falls back to the built-in dark theme on any other Linux distribution. In addition to VeilShell's built-in 1.5s `colors.toml` mtime watcher, you can install the instant theme hook below.
- **Compositor IPC:** `HyprlandBackend` automatically speaks Omarchy 4.5+ Lua `eval` (`hl.config` / `hl.monitor`) and falls back to legacy `.conf` `keyword` commands on standard Hyprland setups.
- **Night Light sync:** Quick Settings' Night Light tile invokes `/usr/share/omarchy/bin/omarchy-toggle-nightlight` when present, falling back cleanly to direct `hyprctl hyprsunset` commands on non-Omarchy systems.
- **Session integration:**
  - **Omarchy 4.5+ (Lua Hyprland):** Use [`veilshell.lua`](veilshell.lua) (sets `hl.layer_rule` `no_anim` for `veilshell` and shows optional `o.bind` keybindings for `veilctl show home/recents/shade` and `veilctl mode toggle`).
  - **Legacy Hyprland `.conf`:** Use [`veilshell.conf`](veilshell.conf).
  - **Aether instant theme hook:** Copy [`hooks/30-veilshell-theme`](hooks/30-veilshell-theme) into `~/.config/omarchy/hooks/theme-set.d/30-veilshell-theme` (`chmod +x`) to trigger `veilctl reload-theme` immediately on Omarchy theme switches.
  - **Optional user systemd service:** Install `veilshell` and `veilctl` (`cargo install --path crates/veilshell-shell`), copy [`systemd/veilshell.service`](systemd/veilshell.service) into `~/.config/systemd/user/`, then run:
    ```bash
    systemctl --user daemon-reload
    systemctl --user enable --now veilshell.service
    ```
    Started with `--policy auto`, `veilshell` uses zero surfaces in Laptop Mode when a detachable keyboard/Type Cover is present and springs open automatically upon detach.
