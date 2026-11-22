# Secure touch unlock

This is groundwork for the [Denial parity security gate](DENIAL_PARITY_SPEC.md#sddm-acceptance-gates),
not a new authentication option. The default session still runs Hyprlock.
`veilshell-lock-input` is an isolated Rust library with no dependencies, no
connection to the shell or compositor, and no ability to authenticate or unlock.
Its attempt state serializes a future PAM worker, rejects stale or cancelled
results, and imposes a two-second monotonic delay after a result. Nothing in
the shipping session calls it.

## Live protocol probe

`cargo build -p veilshell-wayland --example session-lock-probe --no-default-features`
builds an uninstalled test client. **Run it only in a disposable standalone
VeilShell guest with an out-of-band way to end the session.** It requires
`--disposable-guest-no-unlock` and verifies the standalone display handshake.
It does not read credentials, handle input, or have any way to unlock. Ending
the client is not a recovery method; end the guest session instead.

In the Arch/KVM guest with Hyprland `0.57.2`, the compositor acknowledged the
probe's lock and replaced Home with an opaque purple frame. After `SIGKILL`,
the framebuffer showed Hyprland's crashed-locker recovery screen, **not**
the desktop, and `hyprctl locked` still returned `true`. Ending the guest
session returned to SDDM. The development guest saved the two scanout images
as `native-lock-protected.png` and `native-lock-after-kill.png`.

A separate run added and removed a `1920×1080` headless output at 2× scale
while locked. The probe received a configure and committed a `960×540`
logical lock surface. Hyprland remained locked; this does not verify physical
hotplug visibility. Hyprland itself has compositor-level recovery controls;
do not expose them through VeilShell IPC or call them on locker failure.
The default launcher and idle hooks were not changed. PAM, the private
password editor, and wake on hardware remain unverified.

## Which process should own touch entry?

- **Current Hyprlock adapter:** `veilshell-session --lock` scopes the request to
  the session, prevents concurrent lockers, and launches Hyprlock with zero
  grace and immediate rendering. Hyprlock owns the session-lock role, not
  VeilShell's layer surfaces. Keep this interim password path unchanged.
  The adapter's `hyprctl locked` polling and hypridle's bounded sleep inhibitor
  are not evidence of a presented lock frame or a no-flash wake on the target
  build.
- **Extend Hyprlock:** This reuses its output/rendering and PAM conversation
  code. The [reviewed upstream revision][hyprlock] binds `ext-session-lock-v1`,
  tracks outputs and creates a lock surface when a new output becomes ready.
  Its [touch handling][seat] feeds clicks, not a private password OSK, and
  [clickable labels][label] can spawn commands; neither can be configured into
  secure text entry. This route needs an upstream change or a maintained
  downstream build. Audit that exact build: the reviewed revision also has a
  non-PAM `SIGUSR1` unlock path, and its [PAM code][pam] calls
  `pam_authenticate` but not `pam_acct_mgmt`. Do not certify the installed distro
  build from this source review.
- **Dedicated VeilShell locker:** A separate `ext-session-lock-v1` client
  with a locker-owned password editor and PAM is the path this Rust component
  prepares for. SCTK has session-lock protocol helpers, but they do not implement
  auth policy, trusted input routing, or per-output readiness. This path costs
  more to build and verify; don't switch the session over on the strength of
  input-only tests.

Never attach the global app OSK to either locker: no `zwp_virtual_keyboard_v1`,
`zwp_input_method_v2`, clipboard, helper commands, shell IPC, or input-event
injection for credentials. The lock process renders the keys on its own lock
surface and consumes the resulting intents in its private editor.

## Input component contract

The keyboard supplies matching draw and hit rectangles in logical pixels,
printable US ASCII (including Space), one-shot Shift, Backspace, Cancel, and
explicit Submit. Submit means **begin authentication**, not unlock. Cancel
means **discard the local entry or attempt**, not leave the session lock.
There is no password buffer, automatic submit, key repeat, or credentials in
debug formatting. The physical-keyboard path must still support the user's
normal layout; localized touch input and the lock-face/password-stage transition
are separate work.

Bind an instance to a selected `wl_seat` and current lock surface. Validate
identity and eligibility in the caller; use surface-local coordinates in the
keyboard's logical rectangle, the same mapping as rendering. Track all contact
IDs for that seat, including downs outside the keyboard. A contact on another
lock surface must block new taps until released; report it with coordinates
outside the keyboard or reject that touch sequence in the caller. Motion beyond
the original key invalidates the tap. A completed tap is emitted only at
`wl_touch.frame`.
Retire the stream on seat/surface removal, using `cancel_touches` and rejecting
further events from stale objects. Map Wayland touch cancellation to
`cancel_touches`, not to authentication success. On a layout, stage, or
eligibility transition, disarm queued taps without assuming currently held
fingers are released. Only apply intents while the owning lock and editor
are eligible.

## Required before enabling native touch auth

1. **Compositor boundary.** No fullscreen layer-shell fallback. Request the
   lock and create opaque lock surfaces for all current and newly advertised
   outputs, acking configure before attaching buffers. Distinguish lock requested
   from the compositor's `locked` event (which requires a protected frame on
   every output). A refusal, disconnect, or renderer failure is never success.
   Once locked, rely on compositor blanking if a surface is missing or the
   client dies; verify this on Hyprland, including the empty-output case.
   Don't use an unlock request as panic/drop/terminal-event cleanup. Provide
   recovery that does not reveal the live desktop, such as ending the session
   back to SDDM. A crash *before* lock acquisition is not covered by the
   protocol's fail-closed guarantee.
2. **Auth boundary.** Use the session account and a reviewed distro PAM
   service. Require both `pam_authenticate` and `pam_acct_mgmt` to succeed for
   the current, uncancelled password attempt. Unexpected prompts/errors fail
   closed. Serialize attempts and enforce a monotonic retry delay in addition
   to site PAM policy. Clear secrets on submit/denial/cancel and minimize and
   zeroize copies as their lifetimes end, including the PAM conversation where
   possible. No logging or serialization. Invalidate the attempt on editor
   cancellation or surface/seat loss so late positive results cannot unlock;
   never unlock from shell IPC, signals, logind `Unlock`, or a UI state change.
3. **Acceptance on an explicitly opted-in build.** Run a live compositor with
   a disposable account and reviewed PAM stack: test positive password entry,
   repeated wrong passwords and rate limits, PAM account denial, unavailable
   PAM, and cancellation while a response is pending. Confirm trusted success
   is the only transition that exposes the desktop. Exercise touch-only
   portrait and landscape with rotation/scaling, seat loss, multi-touch,
   dock/unplug and no outputs/replug, and kill/panic before and after `locked`.
   Check visibility on *all* outputs, reject stale taps or auth completions,
   and test lock-before-sleep and no desktop flash on wake on hardware.
   No unauthenticated startup/autologin, PIN, or fingerprint in this increment.

`cargo test -p veilshell-lock-input` exercises printable coverage, touch-frame
semantics, gaps and drags, competing contacts, reconfiguration, undersized
surfaces, serialized retries, and rejection of cancelled and replayed
authentication results. These unit tests do not exercise PAM or sleep.

[hyprlock]: https://github.com/hyprwm/hyprlock/blob/5e57c4b451d36db0d5a830b08c55891ab7c9c5ae/src/core/hyprlock.cpp
[seat]: https://github.com/hyprwm/hyprlock/blob/5e57c4b451d36db0d5a830b08c55891ab7c9c5ae/src/core/Seat.cpp
[label]: https://github.com/hyprwm/hyprlock/blob/5e57c4b451d36db0d5a830b08c55891ab7c9c5ae/src/renderer/widgets/Label.cpp
[pam]: https://github.com/hyprwm/hyprlock/blob/5e57c4b451d36db0d5a830b08c55891ab7c9c5ae/src/auth/Pam.cpp
