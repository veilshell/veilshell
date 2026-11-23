# Omarchy Adapter for VeilShell

This directory holds optional, thin integration glue for [Omarchy](https://omarchy.org) users so that VeilShell works alongside an unmodified Omarchy + Hyprland installation.

No adapter installation is required to run the overlay manually. The
VeilShell overlay does not change your window gaps, rounding, or monitor
rules, including when the keyboard detaches. `veilctl mode` requires the
running daemon; it never changes Hyprland settings on its own.

- **Theme sync:** `veilshell-theme` reads `~/.local/state/omarchy/current/theme/colors.toml` via `OmarchyAetherProvider` when available, and falls back to the built-in dark theme on any other Linux distribution. In addition to VeilShell's built-in 1.5s `colors.toml` mtime watcher, you can install the instant theme hook below.
- **Icons and scaling:** VeilShell reads installed icons and uses the output scale already chosen by the compositor; it does not set your icon theme or display scale.
- **Quick Settings:** Controls such as Wi-Fi, brightness, and volume still affect the actual device when you use them. The Wi-Fi, Bluetooth, and Audio detail pages can open the corresponding Omarchy panel without needing to tap the bar. VeilShell uses the running Omarchy shell for Do Not Disturb and notification history; it does not run a second notification daemon. Night Light only appears when this Hyprland instance's Hyprsunset is already running; VeilShell does not start or restart it.
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

For a separate login, see the [standalone SDDM preview](../session/README.md).
It does not use this adapter's theme hook or user service.

## Optional pill-free touch gestures

For an overlay on a touchscreen, start VeilShell with:

```bash
veilshell --policy tablet --overlay-gap-gestures
```

If you have multiple outputs, add `--output` with the touchscreen's output
name. Use these arguments when starting the daemon; don't start a second
copy on the existing control socket.

Pull down from the uncovered gap just below the bar to open the read-only
Shade. From the gap below windows, flick up for **Home**, pull up more
deliberately for **Recents**, or swipe sideways to switch between running
windows on the same output and workspace. Swipe up on Home content for
**All Apps**.

`veilctl status` shows the mounted strip depths. The bar and windows
still get first chance at input; a covered point isn't a gesture target.
Border resizing and Hyprland's own touchscreen workspace swipe take
precedence: the strips disarm when either setting is detected. If the
strips are unavailable, `veilctl show home` still opens Home directly on
the connected target output. Use **Close** on Home or `veilctl show pill`
to return to the desktop without drawing a pill.

Mouse and trackpad swipes don't activate these gestures. Stock Wayland still
sends pointer input in otherwise empty strip pixels to the transparent
surface, so this is not pointer pass-through on the wallpaper. A fullscreen
app keeps its touches. There are no changes to Hyprland, Omarchy's gap
sizes, or the default overlay profile.
See [the behavior and tests](../../docs/tablet/overlay-gaps.md).

If a test daemon is needed alongside an existing daemon, give it a different
`--socket` and point its `veilctl` commands at that socket with `VEILSHELL_SOCKET`.
VeilShell refuses to replace an existing socket. After an unclean exit,
remove the stale socket only after confirming no daemon is using it.
