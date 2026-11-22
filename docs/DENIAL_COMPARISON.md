# VeilShell vs. Denial — Touch UX and feature inventory

VeilShell uses [Denial](https://github.com/denialwm/denial) as a reference for
touch interactions, not as a feature or release checklist. A standalone
VeilShell session needs to cover essential tablet workflows, but it can
delegate services and make different choices for older hardware. The
[roadmap](ROADMAP.md) defines what is required for the first overlay release.

For pinned source references, distinguish Denial's mobile and desktop profiles
using the [parity spec](DENIAL_PARITY_SPEC.md). See the
[validation record](DENIAL_VALIDATION.md) for live evidence and known blockers.

## Architectural Difference at a Glance

| Dimension | Denial (`denialwm/denial`) | VeilShell (`veilshell/veilshell`) |
|---|---|---|
| **Architecture** | Wayland compositor (Rust + Smithay) embedding the **Flutter engine** in-process | Rust `wlr-layer-shell` client (`smithay-client-toolkit` + `wgpu`) with Hyprland as its first live backend |
| **Rendering** | Flutter Impeller GLES scene graph with live DMA-BUF window textures | Single-pass instanced `wgpu` Vulkan/GLES WGSL SDF renderer + shelf-packed GPU atlas + blur-once wallpaper cache + zero-`unsafe` CPU SDF fallback |
| **Target Footprint** | Modern ARM64 phones/tablets & desktops | Older fanless **5W / 4–8 GB RAM** x86 & ARM tablets; see the [performance targets](PERFORMANCE_BUDGET.md) |
| **Desktop Coexistence** | Runs as its own compositor | Overlay leaves host layout and monitor rules untouched; optional SDDM preview uses a separate Hyprland session |
| **License** | `GPL-3.0-or-later` | `GPL-3.0-or-later` |

---

## Status Legend

- **Done** — Implemented, wired end-to-end, and covered by tests/verification.
- **Partial** — Core feature works, but missing key UX details needed for full parity or standalone use.
- **Missing** — No VeilShell implementation; not necessarily a release requirement.
- **Host / Ecosystem** — Intentionally delegated to the host Wayland compositor (Hyprland/Niri/Sway) or separate OS components rather than coupled into VeilShell.

---

## Complete Feature Comparison Matrix

### 1. Core Shell, Gestures & Navigation

| Feature | Denial | VeilShell Status | Current VeilShell Implementation / Gap Notes |
|---|---|---|---|
| Bottom gesture pill & swipe navigation | Built-in touch/trackpad gestures & spring animations | **Partial** | Home and Shade follow the pull and spring to either endpoint. Home pages track horizontal drags; bottom-edge swipes still reach Recents and adjacent workspaces. Denial's liquid pill deformation and shared window motion remain open. |
| Home application launcher | Mobile paged Home with editable slots and a separate all-apps drawer | **Partial** | Responsive paged `.desktop` grid with freedesktop `PNG` icons and a badge fallback. No editable layout, folders, widgets, or separate drawer. |
| App search / instant filter | Search in the desktop launcher | **Missing** | VeilShell requires browsing pages to find an app. A search surface should accept both physical keyboard and touch input. |
| Live Recents / Window Overview | Live Wayland surfaces inside overview | **Partial** | Frameless, swipeable window previews with the app icon/name above each card, captured at higher resolution via `ext-foreign-toplevel-list-v1` + `ext-image-copy-capture-v1`. Tap to focus and swipe up to close; cached captures are limited to three nearby windows. |
| Portrait / Landscape & Auto-Rotation | Responsive mobile/tablet layout + rotation | **Done** | Layout follows the compositor's output changes in both modes. Sensor-driven rotation and Rotation Lock are scoped to standalone. |
| Split-screen / Side-by-side touch snapping | Native compositor tiling/stacking/scrolling layouts | **Partial** | Delegated to host compositor (`Hyprland`), but VeilShell does not yet expose a touch drag-to-split affordance from `Recents`. |

### 2. Standalone Boot & Session Lifecycle (Required for Standalone Display Manager / Boot Entry)

| Feature | Denial | VeilShell Status | Current VeilShell Implementation / Gap Notes |
|---|---|---|---|
| Standalone Wayland session entry | Ships as a standalone Wayland compositor session | **Partial** | [SDDM preview adapter](../adapters/session/README.md) starts a dedicated Hyprland. Stock guest login/logout passed with automatic software fallback. Portals, permission prompts, packaging, and hardware validation remain. |
| Touch Lock Screen (`ext-session-lock-v1`) | Native lock screen with password/PIN, fingerprint, and hidden sensitive notifications | **Missing** | Required for touch-only standalone use, not ordinary overlay use. The preview uses Hyprlock with a physical keyboard. See the [secure touch unlock plan](TOUCH_LOCK.md). |
| Power Menu (Lock, Sleep, Reboot, Shutdown, Logout) | Integrated session power actions + power-button handling | **Partial** | Touch Power menu in the standalone preview, including confirmation and a lock check before Suspend. Physical power-button behavior and hardware smoke tests remain. |
| Idle Management & DPMS / Screen Blank | Integrated `logind` + DPMS + Always-On Display / wake handling | **Partial** | The preview provides Hypridle lock, display blank and wake defaults; no always-on display, touchscreen unlock, or hardware sleep verification yet. |
| Hardware Key OSDs (Volume ±, Brightness ±, Mute) | On-screen visual overlays for hardware button presses | **Missing** | Tablets have physical volume rockers and power buttons; standalone mode needs a lightweight auto-hiding glass OSD pill when volume/brightness changes. |
| Auto-detected desktop wallpaper for glass blur | Automatically uses active wallpaper | **Partial** | GPU blur-once wallpaper cache (`WgpuRenderer::set_wallpaper`) and `veilctl wallpaper <PNG>` exist, **but** must be passed manually rather than auto-detecting the session wallpaper on startup/change. |
| Packaging (`PKGBUILD` / `systemd --user`) | Packaged for Arch / Denial OS | **Partial** | Ships `adapters/omarchy/systemd/veilshell.service`; still missing Arch/AUR `PKGBUILD` and standalone session packaging (Phase 4). |

### 3. Notifications & Media Controls

| Feature | Denial | VeilShell Status | Current VeilShell Implementation / Gap Notes |
|---|---|---|---|
| System Notifications (`org.freedesktop.Notifications`) | Native notification banners & history with lockscreen privacy | **Missing** | Standalone needs actionable notifications with lock-screen privacy. In overlay mode, notification delivery remains with the host desktop. |
| MPRIS Media Player Widget | Embedded MPRIS dashboard widget (track info, art, progress, Play/Pause/Prev/Next) | **Missing** | `Shade` currently renders quick toggles and brightness/volume sliders only; needs an MPRIS D-Bus card when media is active. |
| Per-Application Audio Mixer | Per-window volume mixing in control center | **Missing** | `Shade` has a single master PipeWire (`wpctl`) volume slider; per-stream mixing is not yet implemented. |

### 4. Quick Settings, Hardware & Connectivity

| Feature | Denial | VeilShell Status | Current VeilShell Implementation / Gap Notes |
|---|---|---|---|
| Quick Settings Tiles (Wi-Fi, Bluetooth, LTE, Night Light, Auto-Rotate, Tablet Mode) | Integrated control center tiles | **Done** | Adaptive Connectivity, Display & Sound, and Device groups. Controls pack without leaving slots for unavailable backends. Auto-rotate is limited to the standalone compositor; the overlay's Desktop Mode control only hides VeilShell. |
| Brightness & Volume Sliders | Interactive sliders in control center | **Done** | Vertical, liquid-style level bars like Denial's ColorOS shade. Tap to set an absolute level or drag from the current level; live `sysfs`/`brightnessctl` and `wpctl` backends. |
| Battery Telemetry | Live battery % & charging state | **Done** | Pure `sysfs` reader filtering out peripheral `scope=Device` (stylus/keyboard) batteries; can be expanded with charging icon & low-battery warning banner. |
| Power Profiles (`power-profiles-daemon`) | Power profile selector tile (Power Saver / Balanced / Performance) | **Done** | `powerprofilesctl` profile tile cycles Balanced / Power saver / Performance, shown only when the service answers. |
| Do Not Disturb | Silent mode tile in control center | **Done** | Toggles mako's `do-not-disturb` mode when a `[mode=do-not-disturb]` rule is configured. |
| Interactive Network & Bluetooth Picker Sheets | Visual settings UI to pick Wi-Fi networks & pair BT devices | **Missing** | **Important for standalone boot.** Today Wi-Fi/BT tiles toggle radios on/off, but picking a new SSID/password or pairing a Bluetooth device requires an external tool (`impala`/`bluetui`/`nm-connection-editor`). |
| Detachable Keyboard Auto-Switch | Mobile/desktop convergence | **Partial** | DMI-matched `hardware-profiles/*.toml` and USB Type Cover polling are implemented. Generic `SW_TABLET_MODE` hinge events are not supported yet. |
| Touch Haptics / Vibration Feedback | Hardware haptic feedback via companion stack | **Missing** | Optional `ff-memless` / `evdev` force-feedback pulse on OSK key taps & gesture thresholds for hardware that has a vibration motor. |

### 5. Touch Input & On-Screen Keyboard (OSK)

| Feature | Denial | VeilShell Status | Current VeilShell Implementation / Gap Notes |
|---|---|---|---|
| Built-in Touch OSK (`input-method-v2` + `virtual-keyboard-v1`) | Native mobile/tablet keyboard | **Done** | Multi-layer (`Letters`, `Numbers`, `Symbols`, Shift/Caps/Ctrl), `ContentPurpose` auto-layer switching, auto-pop on text focus in Tablet Mode, swipe-down dismiss. |
| Key Press Visual Feedback, Key Repeat & Long-Press Accents | Standard touch keyboard ergonomics | **Partial** | Hold Backspace or an arrow for 400 ms to repeat. Pressed-key popups, accents and international layouts remain open. |
| Arrow / Escape / Tab Utility Row (Terminal Mode) | Tablet/Linux terminal ergonomics | **Done** | `TERM` toggles a utility row while retaining the alphabetic keys. A terminal input hint enables it automatically. |

### 6. Rendering, Theming & Visual Polish

| Feature | Denial | VeilShell Status | Current VeilShell Implementation / Gap Notes |
|---|---|---|---|
| GPU Glassmorphic Renderer | Flutter Impeller GLES compositing | **Partial** | GPU Wayland presentation and software rendering worked in baseline nested tests. Surface-aware automatic fallback was verified in the Virtio SDDM guest. Static wallpaper glass is not compositor refraction. |
| TrueType / OpenType Typography | Native Skia text rendering | **Done** | `ab_glyph` system font atlas with automatic 5×7 HUD bitmap fallback. |
| App Icons (PNG + SVG) | Full desktop icon rendering | **Done** | Freedesktop themes and installed PNG/SVG assets supply the launcher artwork. A letter badge is the fallback for a genuinely missing or unusable icon. |
| Dynamic Theming & Wallpaper Accent Extraction | Light/dark themes + accent colors borrowed from wallpaper | **Partial** | Live Omarchy Aether `colors.toml` mtime watcher + WCAG AA dark fallback exist; missing automatic wallpaper-dominant-color accent extraction for non-Omarchy distros and a Light/Dark toggle. |

### 7. Out-of-Scope / Host Compositor & Ecosystem Responsibilities

These features exist in Denial because Denial **is** a full Wayland compositor and mobile OS ecosystem, whereas VeilShell deliberately stays modular and composable:

| Feature in Denial Ecosystem | Why It Stays Outside VeilShell |
|---|---|
| **Window tiling / stacking / scrolling engine** | Handled natively by the host Wayland compositor (**Hyprland**, and future **Niri** / **Sway** backends via `veilshell-compositor`). |
| **XDG Desktop Portals & Screencast plumbing** | Provided by `xdg-desktop-portal-hyprland` / `xdg-desktop-portal-wlr` / compositor stack. |
| **Live Dart / Flutter Hot-Reload Shell Editing** | VeilShell keeps its Rust renderer and offline snapshot workflow. |
| **Android App Runtime (`Droidloom`)** | Separate container/runtime layer; Android apps launched via Waydroid or Droidloom appear as normal Wayland windows and `.desktop` entries inside VeilShell automatically. |
| **Cellular Voice/SMS Stack (`Phonewave`)** | Separate telephony layer; VeilShell focuses on tablet WWAN data toggles (`nmcli` / ModemManager). |

---

## Standalone priorities

To move beyond the preview, prioritize compositor-enforced touch unlock,
reliable sleep and wake, essential touch workflows, and session-scoped
services. The ordinary app keyboard is not used for lock credentials. Shared
improvements to touch navigation and app input benefit both modes; fullscreen
defaults and Desk Mode belong to the dedicated session. See the
[standalone roadmap](ROADMAP.md#standalone-and-feature-follow-up) for progress.
