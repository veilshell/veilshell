# VeilShell standalone Wayland session (preview)

This adapter adds an optional **VeilShell (Preview)** entry to SDDM. SDDM
authenticates the user; it is not a greeter theme and **does not support
autologin**. The adapter uses a dedicated Hyprland Lua config and starts the
same VeilShell daemon used by the overlay, in Tablet Mode. You do not need
this entry to run the [overlay](../../README.md#two-ways-to-run-one-shell).
The preview does not load the user's normal Hyprland or Omarchy config,
start global user targets, or publish over another desktop's activation
environment. The daemon uses its built-in palette rather than syncing
Omarchy's theme, and it does not invoke Omarchy's Night Light or notification
helpers. The compositor also leaves the user's shared GSettings cursor
preferences alone. VeilShell can still read installed apps and icon themes.

Requires Bash 5.1+, Hyprland 0.57+ with Lua, `hypridle` with `inhibit_sleep = 3`,
`hyprlock` with a working distro PAM configuration, and util-linux `flock`.
Use only with a **physical keyboard connected**: the interim locker can accept
the account password from a keyboard, not the VeilShell OSK. Test the account's
password and locker before relying on idle or suspend behavior.

After login, the same app keyboard as the overlay is available. Tap the keyboard
icon to the right of the Home pill to open it without a shortcut, or tap a
compatible app's text field for auto-open. This does not change Hyprlock or SDDM.

## Preview installation

From the repository root, build and explicitly install the files below. Nothing
here changes SDDM settings or makes VeilShell the default.

```bash
cargo build --release --bin veilshell --bin veilctl
sudo install -Dm755 target/release/veilshell /usr/bin/veilshell
sudo install -Dm755 target/release/veilctl /usr/bin/veilctl
sudo install -Dm755 adapters/session/veilshell-session /usr/bin/veilshell-session
sudo install -Dm644 -t /usr/share/veilshell/session adapters/session/{hyprland.lua,bindings-omarchy.lua,hypridle.conf,hyprlock.conf}
veilshell-session --check
sudo install -Dm644 adapters/session/veilshell.desktop /usr/share/wayland-sessions/veilshell.desktop
```

At the next SDDM login, select **VeilShell (Preview)**. `Super+H` opens Home,
`Super+R` opens Recents, `Super+S` opens Quick Settings, `Super+L` locks, and
`Super+Shift+E` ends the preview session. A shell or idle-service failure also
ends the session instead of leaving the original greeter stopped.

The **Power** tile in Quick Settings opens Lock, Suspend, Log out, Restart, and
Shut down. Only Lock acts without a confirmation tap. Touch Suspend checks for
Hyprland's lock acknowledgment before requesting sleep. System power actions
still depend on the distribution's logind/polkit permissions.

`hypridle` requests the scoped `hyprlock` after five minutes of inactivity, turns
the display off after another 30 seconds, and requests locking before system
sleep. It requests a logind delay until Hyprland signals lock; logind's finite
delay is not a hard guarantee against resuming unlocked if startup fails. There
is no automatic suspend timer. An [opt-in autostart helper](AUTOSTART.md) can
launch explicitly allowed entries. Native touch unlock, portal activation, a
polkit agent, and SDDM login and wake tests on real hardware are still
required before calling this a daily-driver session.

Test it by logging **out** of Omarchy and choosing **VeilShell (Preview)** at
SDDM. After logging back into Omarchy, check that its layout, shortcuts,
and services are unchanged. This is a separate compositor, not a VM: hardware
settings (Wi-Fi, audio level, brightness) are shared, and the preview is not
designed for simultaneous desktops under the same user account.

## Optional Omarchy-style keybindings

Set the first line of `$XDG_CONFIG_HOME/veilshell/bindings.profile` (normally
`~/.config/veilshell/bindings.profile`) to `omarchy`, then run
`veilshell-session --check` before logging in. Use `preview` or remove the
profile file to restore the defaults. Neither profile loads Omarchy's Lua,
autostart, shell, or UWSM.

| Keys | Action |
|---|---|
| `Super+Space` | Toggle the searchable VeilShell desktop launcher |
| `Super+Enter`, `Super+Shift+Enter` / `Super+Shift+B`, `Super+Shift+F` | Terminal, default browser, default file manager |
| `Super+Q/W`, `Super+F`, `Super+T`, `Alt+Tab` / `Alt+Shift+Tab` | Close, fullscreen, toggle floating, cycle windows |
| `Super+1–0`, add `Shift` to move, or `Shift+Alt` to send without following | Hyprland workspaces 1–10; not Omarchy's per-monitor numbering |
| `Super+Tab` / `Super+Shift+Tab`, `Super+S` / `Super+Alt+S` | Cycle workspaces, toggle/send to scratchpad |
| `Super+Ctrl+L` | Lock with Hyprlock |
| `Super+Ctrl+Alt+R` / `Super+Ctrl+Alt+S` | VeilShell Recents / Quick Settings |
| Volume, brightness and playback keys | `wpctl`, `brightnessctl` and `playerctl` where available |

Log out from **Quick Settings → Power**. `Super+R` is left free for personal
bindings, and `Super+Shift+E` no longer logs out in this profile. Terminal
launch uses `xdg-terminal-exec` or an installed terminal. Browser launch uses
`xdg-settings` and `gtk-launch`; file manager launch uses `xdg-open`.
Applications that hand off to an existing process or require UWSM or portals
may need a separate launcher. Installed web apps remain in Home if their
desktop entries are discoverable; their shortcuts aren't imported.

In the desktop launcher, type to filter by app name or desktop-file ID.
Use arrows or Tab/Shift+Tab to select, Enter to launch, or Escape to dismiss.
You can also click a result. `veilctl show launcher` opens or closes the same
surface without the profile. Tablet Home still opens from the bottom swipe
and keeps its paged touch grid. Desk Mode layout policy is not required to
use the launcher and is not part of this change.

For personal bindings, add only the Lua you want to
`$XDG_CONFIG_HOME/veilshell/bindings.lua` (normally under `~/.config`). It
runs after the selected profile. Unbind a conflicting chord before binding
it, for example:

```lua
hl.unbind("SUPER + SHIFT + F")
hl.bind("SUPER + SHIFT + F", hl.dsp.exec_cmd("flea --gui"))
```

Re-run `veilshell-session --check` after edits. This profile does not port
Omarchy-specific panels, launch-or-focus helpers, theme toggles, or resize
submaps; personal Lua that depends on `o.*` needs to be adapted first.

To remove the **session picker entry** without touching SDDM or other sessions:

```bash
sudo rm /usr/share/wayland-sessions/veilshell.desktop
```
