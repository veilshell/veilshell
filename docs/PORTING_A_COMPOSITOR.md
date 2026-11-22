# Compatibility and porting VeilShell

VeilShell targets capabilities, not distribution names. A distro that ships
Hyprland can reuse the Hyprland backend; it may need different packages or
service setup, not different shell code. A compositor with different window
management APIs needs a backend even if it runs on the same distro.
Layer-shell support alone is not full compatibility.

## What works today

Hyprland is the only live backend. Surface Go 3 + Hyprland is the
[`v0.1.0` validation target](ROADMAP.md#first-release);
Omarchy is optional. [Recorded guest and nested tests](DENIAL_VALIDATION.md)
are not certification for other distro, kernel, or hardware combinations.
The live daemon requires a reachable Hyprland backend and reports a startup
error if it cannot query it. Mock window data is for tests and offline tools,
not live window actions. Use `veilctl status` and `veilctl hw` for current
diagnostics.

## Adapter boundaries

| What differs | Where to add support |
|---|---|
| Window listing, focus, close, workspace switching, app launch, and compositor-specific layout or output control | Implement [`CompositorBackend`](../crates/veilshell-compositor/src/lib.rs) in `veilshell-compositor`. Prefer a suitable Wayland protocol where it supports the operation; otherwise use the compositor's IPC. |
| Layer surfaces, input, scaling, and window capture | Use or extend protocol support in [`veilshell-wayland`](../crates/veilshell-wayland). Protocol availability is separate from backend detection. |
| Audio, radios, brightness, sensors, and device quirks | Extend [`HardwareBackend` or `HardwareController`](../crates/veilshell-hw/src/lib.rs) for a new service or API. Add a [hardware profile](HARDWARE_PROFILES.md) for device quirks, not distro differences. |
| Desktop theme | Use [`ThemeProvider`](../crates/veilshell-theme/src/lib.rs) and keep theme reads optional. The built-in palette does not need a desktop theme service. |
| Packages and login/session integration | Document dependencies and permissions for the distro. Keep launchers, desktop hooks, and service configuration in [adapters](../adapters) or downstream packages. |

These Rust traits are the contributor API today. They are not a stable
external plugin API or wire protocol. Test the contract with a second live
compositor before designing either.

If a distro already ships a compatible Hyprland, start by testing a package
or source install rather than adding a backend. Map optional tools to local
packages (for example, `nmcli` for NetworkManager and `wpctl` for WirePlumber),
and check the services and permissions in a normal user session. Neither the
Omarchy adapter nor the standalone SDDM entry is required for the overlay.

## Capability contract

Use the active Wayland session and actual service availability to determine
what can be offered. A protocol being advertised or a command being installed
doesn't guarantee that the compositor grants access or that the service is
running. The table describes what ports should do; not every fallback is wired
up consistently today.

| Feature | Requirement | If unavailable |
|---|---|---|
| Overlay surfaces | A Wayland connection with `wl_compositor`, `wl_shm`, and `zwlr_layer_shell_v1` | Report the missing requirement instead of claiming the overlay works. |
| Home app launch | XDG app entries and a launch path in the active session | Report launch as unavailable; never acknowledge a mock launch as success. |
| Recents and workspace gestures | Backend support for the relevant window and workspace actions | Disable actions independently. If windows cannot be listed, show that Recents is unavailable, not an empty or fake list. |
| Live Recents thumbnails | Foreign toplevel list and matching window IDs, a toplevel image capture source, and `ext-image-copy-capture-v1` | Keep window titles and icons where listing works; omit live previews. |
| App keyboard | Input-method support for text-focus activation; virtual-keyboard support for injected keys | Offer only the input paths that work. Don't promise auto-show or typing on the strength of the keyboard drawing. |
| Quick Settings controls | The relevant system service or tool, a device it can control, and sufficient permissions | Hide unavailable controls or explain why an action failed. |
| GPU rendering | A usable GPU backend and Wayland presentation path | Use the software renderer in automatic mode. |

Keep fixtures in tests and offline previews. The [roadmap](ROADMAP.md#portability-follow-up)
tracks runtime capability fallbacks and a read-only `veilctl doctor` to
report detected capabilities and missing requirements.

## Contributing a port

1. **Identify the missing layer.** Record the distro and version, kernel,
   compositor and version, device/architecture, and session mode. Check
   whether the gap is packaging, a missing service or protocol, compositor
   IPC, or a device quirk before changing shared code.
2. **Implement or extend the narrowest adapter.** A compositor port belongs
   in `veilshell-compositor`, with detection wired in `veilshell-shell`.
   Return `CompositorError::Unsupported` for operations the backend cannot
   provide. Keep the UI, gesture logic, and rendering shared.
3. **Preserve the session boundary.** Overlay tests must verify that mode
   changes leave the host's layout and monitor rules untouched. Standalone
   layout and output changes belong only to the session VeilShell owns.
4. **Test and report the result.** Add unit tests for adapter responses and
   failure paths, and test app launch, window actions, scaling, and missing
   capabilities in a private compositor. Use the
   [Hyprland harness](../scripts/test-headless-hyprland.sh) as a reference.
   Smoke-test package install, launch, and removal on any distro you list as
   tested. Use real hardware for touch, sensors, power, and performance
   claims. Report which features passed, which were unavailable, and
   which were not tested.
