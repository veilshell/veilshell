# Standalone Tablet behavior

**Denial is the target; Penumbra is a Rust implementation reference.** Compare
Denial's **mobile** profile in both orientations. Landscape is not the desktop
profile. This work is for the authenticated VeilShell Wayland session launched
by SDDM, not the greeter, Desk Mode, or the Omarchy overlay.
The separate [opt-in overlay gap profile](overlay-gaps.md) reuses Tablet gestures
without taking over the host desktop.

| Reference | Pinned checkout | Role |
|---|---|---|
| [Denial v0.6.8](https://github.com/denialwm/denial/tree/01b588ec50831bce9e57aed623901f49846ec791) | `01b588ec50831bce9e57aed623901f49846ec791` | Visible behavior and visual target |
| [Penumbra](https://github.com/penumbra-mobile/penumbra-env/tree/c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb) | `c2b87c2453108ebbdbb036c5ff2e18a5ac03c4eb` | Alternatives for input, motion, rendering and services; phone dimensions and defaults are not tablet requirements |

Keep detached, unmodified upstream checkouts outside this repository. The
development lab uses `../denial-reference` and `../veilshell-lab/penumbra-env`.
Neither project is a build or runtime dependency. Cite source by commit, path,
and line; in runtime records, also identify the build and enabled options.
Record a newer upstream build separately rather than changing the target
silently.

## Feature pages

| Page | Owns |
|---|---|
| [Navigation](navigation.md) | Touch ownership and gesture entry, progress, release, cancellation and routing |
| [Home](home.md) | Clock/grid, app organization, drawer and launch/resume |
| [Recents](recents.md) | Window order, card motion, focus and close |
| [Shade](shade.md) | Portrait pages, landscape split, material, controls and notifications |
| [Keyboard](keyboard.md) | App editor focus, keyboard motion, insets and key handling |
| [Overlay gap gestures](overlay-gaps.md) | Pill-free entry from existing desktop gaps on stock Hyprland |

Lock/wake and SDDM session services are follow-up audits; the
[validation gates](../DENIAL_PARITY_SPEC.md#sddm-acceptance-gates) still apply.
Use the [call map](../DENIAL_CALL_MAP.md) for the cross-feature overview and the
[validation record](../DENIAL_VALIDATION.md) for live evidence and its limits.

## From source to acceptance

- **Source-traced**: found the reachable handler, state change, visible effect,
  and compositor or service call at the pin. Mark defaults, opt-ins, and
  platform-specific paths separately.
- **Runtime-verified**: exercised a described case on the named build and setup.
  A unit test, virtual pointer, virtual touch device, SDDM guest, and physical
  tablet each prove different things. Record which was used; a screenshot alone
  doesn't verify an interaction.
- **Unknown or blocked**: say what's missing or which compositor capability
  is needed. Don't silently approximate it with a different gesture or UI.

Follow the [analysis and implementation handoff](../../AGENTS.md) per slice:
pin the expected result, describe commit, cancel, and interruption cases, and
separate unsupported capabilities. For visual reviews, capture
both sessions with the same wallpaper, time, fonts, installed apps and logical
viewport, then repeat at the Surface Go's fractional scale. Keep credentials
and guest logs out of the repository.
