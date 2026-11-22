# Session-scoped desktop services

`veilshell-services` is an optional companion to the standalone VeilShell
session. It starts available daily-driver desktop services directly inside the
standalone session's private Wayland and Hyprland environment and terminates
them on logout, without publishing environment variables to `systemd --user`
or disturbing Omarchy or Denial.

## Services managed

1. **XDG Desktop Portals (`portals`)** — starts `xdg-desktop-portal-hyprland`
   and `xdg-desktop-portal-gtk` followed by `xdg-desktop-portal --replace` so
   file pickers, screen sharing, and `xdg-open` work inside the standalone
   session.
2. **Polkit Authentication Agent (`polkit`)** — starts the first installed
   Wayland/XDG polkit agent (`hyprpolkitagent`,
   `polkit-gnome-authentication-agent-1`, `polkit-kde-authentication-agent-1`,
   `polkit-mate-authentication-agent-1`, or `lxqt-policykit-agent`).
3. **Notification Daemon (`notifications`)** — starts the first installed
   Wayland notification daemon (`mako`, `swaync`, `dunst`, or `fnott`).

## Configuration

Inspect what is detected on your system:

```bash
veilshell-services --check
```

To customize or disable services, create `$XDG_CONFIG_HOME/veilshell/services.conf`
(normally `~/.config/veilshell/services.conf`):

```ini
# Enable only portals and polkit (or write "none" to disable all):
portals
polkit

# Optional explicit executable overrides:
# polkit_agent=/usr/lib/hyprpolkitagent/hyprpolkitagent
# notification_daemon=/usr/bin/mako
```

You can also override for a single run with `VEILSHELL_SERVICES=none` or
`VEILSHELL_SERVICES=portals,polkit`.
