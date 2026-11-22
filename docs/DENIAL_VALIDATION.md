# Denial parity validation

The reference is [Denial v0.6.8](https://github.com/denialwm/denial/releases/tag/v0.6.8),
source commit `01b588ec50831bce9e57aed623901f49846ec791`. Keep its checkout
read-only and outside the VeilShell repository. Denial is a test reference,
not a VeilShell runtime or build dependency. Use its signed packages for the
reference guest and confirm that the release notes and tag resolve to the pin.
Record newer builds separately; don't silently move the reference.

## Capture protocol

Use separate disposable guests on the same base image, without mounting the
host home or sharing its clipboard, config, or devices. Record the commit,
renderer, viewport, logical scale, profile, palette, wallpaper, fonts,
icon theme, and test-app versions with each capture. Use one redistributable
wallpaper and deterministic app/notification fixtures in both guests.

Start at landscape `1280×800` and portrait `800×1280` at 100%, then exercise
fractional scaling on the reference tablet. Use Denial's **mobile** shell in
both orientations. Capture the desktop Launcher and Dashboard in a separate
run. Record the completed, cancelled and reversed gestures, not just their
resting states. A virtual multitouch device tests Wayland touch delivery,
not physical touch latency, rotation sensors, or hardware suspend.

| Flow | Evidence to record |
|---|---|
| Home and app drawer | Default page, drawer, drag to rearrange, folder creation, and layout after re-login |
| Quick Settings | Partial reveal and cancel, full reveal, landscape notification split, real notification action, radio setup |
| Recents and display handoff | Open three apps, drag through cards, swipe to close, move a window to an external output, unplug and recover |
| App and lock keyboards | App text entry, accents, locker-owned touch entry, wrong-password denial and successful unlock |
| Lock, sleep and logout | Compositor lock acknowledgment before sleep, visible lock face on wake without a desktop flash, and clean return to SDDM |
| Desktop and Omarchy | Denial desktop search and Dashboard independently; VeilShell logout followed by Omarchy login with existing bindings, theme, and desktop services intact |

Use [the feature inventory](DENIAL_COMPARISON.md) to track each outcome as
unverified, partial, verified, or blocked by a compositor capability. For
visual review use [the source-derived tokens](DENIAL_PARITY_SPEC.md), matched
screenshots, and gesture captures together. Keep temporary images and logs
out of Git unless curated and license-checked. Never capture credentials.

### Initial Denial guest captures

The signed Arch packages `denial 0.6.8-1` and
`denial-flutter-engine 1:0.6.8-1` booted through SDDM in a separate Arch/KVM
guest with Mesa software rendering and `DENIAL_SHELL_PROFILE=mobile`.
Initial landscape (`1280×800`) and portrait (`800×1280`) captures at scale 1
are grouped in
`../veilshell-lab/denial-0.6.8-captures/` in the development lab. The stock
Home shows a large date/clock widget integrated into the grid. A virtual
touch swipe opens Quick Settings, with an empty notification panel on the
left and controls on the right. The wallpaper and app catalog are still the
guest defaults; these are reference captures, not matched VeilShell diffs.

Denial reports the logical portrait dimensions after applying
`transform=Virtual-1,90`; the raw QEMU scanout was rotated back for viewing.

## Verified VeilShell baseline

At VeilShell `1bdd1df`, the Arch/KVM guest with Hyprland `0.57.2` ran SDDM
login, touch launch and typing in Foot, swipe to Quick Settings, physical
keyboard password unlock, and touch logout with a **guest-only software
renderer override**. The stock renderer panicked in `wgpu-hal` GLES EGL
(`BadDisplay`) during login. With guest polkit installed, suspend and wake
left the compositor locked, but Virtio GPU errors left the screen black.
This is **not** a wake pass or a stock SDDM pass.

Commit `3a6667a` adds surface-aware GPU selection and automatic software
fallback for Wayland GPU presentation failures. With this change, two stock
SDDM logins in the same Virtio guest (no renderer override) reached Home with
`renderer: sdf-software`; both normal exits returned to SDDM with VeilShell
and Hypridle gone. This verifies the startup fix, not physical GPU or wake
behavior. All 105 Rust workspace tests and strict Clippy passed.

The isolated nested Hyprland harness separately exercised GPU and software
rendering at 100%, 150%, and 200% in landscape and portrait. It does not
replace SDDM or hardware tests. Touch unlock, portal/polkit integration,
notifications, and Wi-Fi/Bluetooth setup remain open.

The [locker prototype and validation record](TOUCH_LOCK.md) covers a separate
Wayland probe that remained locked after client death in the guest. The
locker-local input and attempt state are not yet connected to PAM or to the
probe. The launcher can use the
[opt-in XDG autostart helper](../adapters/session/AUTOSTART.md) if installed
and configured; this does not yet provide the rest of the desktop services.
In a third stock SDDM login, an allowlisted Foot fixture opened on VeilShell's
Wayland display and was gone on return to SDDM. The test fixture was removed
afterward. Omarchy coexistence still needs its own login regression test.

## Omarchy-style shortcuts in the SDDM guest

The standalone profile was tested in a disposable clone of the Arch/KVM
guest on Hyprland `0.57.2`, with no Omarchy installed. The updated session
scripts and bindings used the guest's existing VeilShell preview binaries.
SDDM login reached Home with software fallback; `Super+Enter` launched Foot
and a standalone-only `Super+Shift+F` override launched a distinct Foot
fixture. `Super+Ctrl+L` locked; the guest password unlocked; normal daemon
exit removed the shell and returned to SDDM. This does not verify a real
browser, file manager or web app, and does not replace the Omarchy coexistence
test or physical Surface Go 3 validation.

## Integration gates

Use the [first release checklist](ROADMAP.md#first-release) for overlay
sign-off. The SDDM, lock, sleep, and fresh-session service checks below apply
to standalone. Performance testing matters for both; record each mode
separately.

- Boot the unmodified optional SDDM entry and return to the greeter on logout
  or startup failure. Test with both accelerated and software-only graphics.
- Keep the lock active through failed authentication, locker failure, and
  output hotplug. Confirm lock before sleep and verify a visible lock frame on
  wake. The preview still requires a physical keyboard for unlock.
- Verify desktop services in a fresh session and then after a return to
  Omarchy. Do not change its kernel, boot, default session, keybindings, or
  shared desktop service configuration to achieve compatibility.
- Run the [performance budget](PERFORMANCE_BUDGET.md) on the Surface Go 3,
  measuring idle wakeups and total session memory in addition to shell RSS,
  first-frame time and frame-time percentiles. Test matching and reduced
  effects separately; guest software performance is not a substitute.
