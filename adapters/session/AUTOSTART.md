# Opt-in standalone XDG autostart

`veilshell-autostart` is an optional companion to the standalone session. It
does nothing without `$XDG_CONFIG_HOME/veilshell/autostart.list` (default
`~/.config/veilshell/autostart.list`). Add one exact `.desktop` filename per
line; blank lines and lines starting with `#` are ignored. For example, a
`my-widget.desktop` entry under `~/.config/autostart/` runs in VeilShell only if
`my-widget.desktop` is also listed in `autostart.list`. The normal XDG autostart
directories are still shared with other desktops; creating a new entry there
can affect those desktops. Give a new VeilShell-only entry
`OnlyShowIn=VeilShell;` to exclude other desktops.

Before opting in, review the entry and its executable. An allowlist entry
approves the current and future versions of that filename, not a specific
command. The user autostart directory takes precedence over the directories in
`$XDG_CONFIG_DIRS` (default `/etc/xdg`), in order. Even a disabled or malformed
higher-priority entry masks lower-priority copies. `Hidden`,
`X-GNOME-Autostart-enabled`, `OnlyShowIn`, `NotShowIn`, and `TryExec` are honored;
the desktop identifiers for these checks are `VeilShell` and `Hyprland`.
`NoDisplay` does not suppress autostart.

The helper starts the entry's `Exec` directly in the compositor's
environment; it does not evaluate `Exec` through a shell, activate a D-Bus
service, or publish environment variables to the user service manager. It
does not change the desktop identifier exported to applications. Terminal
entries, D-Bus activation, and desktop-specific conditions/phases/delays are not
supported. `%c`, `%i`, `%k`, and the file/URI field codes must each be a whole
argument. File/URI placeholders are dropped because there is no file or URI to
open.

This is lifecycle scoping, not a sandbox. Apps that detach from the process
group, talk to a single-instance process on the shared user bus, or alter the
user manager themselves are not suitable here. Omarchy does not invoke this
helper. Portals, a polkit agent, and a notification server are not provided.

## Session launcher integration

Install `adapters/session/veilshell-autostart` as an executable on `PATH`, e.g.
`/usr/bin/veilshell-autostart`. Runtime dependencies are Python 3.11+,
PyGObject with the GLib typelib (`python-gobject` on Arch), and GNU `sleep`.
No PyGObject import is needed if nothing is enabled.

The standalone launcher starts the helper if installed, after both compositor
sockets are ready and their endpoints have been exported. Its PID is tracked
for teardown but its exit does not end the session. The helper sends TERM and
then KILL to its own process group, not the launcher's group. `--check` never
starts it.

Run `python3 scripts/test-session-autostart.py` to test real child launches and
teardown using fake display sockets and temporary XDG directories. A real
SDDM login and actual compositor/application compatibility still need testing.
