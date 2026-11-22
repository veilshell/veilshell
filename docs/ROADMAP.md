# VeilShell — Project Plan & Roadmap

Build one touch-first shell for older Linux tablets. Ship it first as a
dependable overlay, then reuse it as the main interface in a dedicated
tablet session. VeilShell does not replace the Wayland compositor in either
mode.

## Core Principles

1. **One shell, separate session policies.** Share gestures, Home, Recents,
   Quick Settings, the app keyboard, and rendering. The overlay leaves host
   layout, monitor rules, and services alone. A dedicated session can choose
   its own window defaults and integrate its own services.
2. **Portable core, capability-based adapters.** Adapt to the active
   compositor, Wayland protocols, system services, and hardware, not to distro
   names. Keep packaging and optional desktop integration separate. See the
   [adapter contract](PORTING_A_COMPOSITOR.md).
3. **Older hardware comes first.** The Surface Go 3 is the reference device,
   not the minimum spec. Treat the [performance budget](PERFORMANCE_BUDGET.md)
   as measurable targets, not claimed results.
4. **Contributors can verify changes without a tablet.** Use unit tests,
   offscreen snapshots, and a private nested compositor. Use real hardware
   for touch latency, sensors, suspend/resume, and performance claims.
5. **Use Denial as a touch UX reference.** Use its mobile flows for comparison,
   adapting them to VeilShell's performance and session boundaries. Neither
   visual parity nor feature parity defines a release.

## First release

`v0.1.0` will ship a dependable **Hyprland overlay**. The initial validation
target is **Surface Go 3 + Hyprland**, with Omarchy as an optional
integration. Don't infer support for other distro, kernel, or compositor
versions from that target.

Follow the [overlay validation guide](OVERLAY_VALIDATION.md) for the first
functional path, from source install to a recorded reference-tablet run.

- [ ] Verify touch navigation, app launch, Recents focus and close, Quick
  Settings, and in-app text entry in portrait and landscape on the reference
  tablet.
- [ ] Verify Laptop ↔ Tablet transitions, including Type Cover attach/detach
  on the reference tablet. Confirm hidden surfaces stop redrawing and mode
  changes leave the host's monitor rules and layout unchanged.
- [ ] Measure the [performance budget](PERFORMANCE_BUDGET.md) on the reference
  tablet and record results, including hidden CPU/wakeups, active RSS, frame
  times, startup, and mode switching.
- [ ] Document an install and smoke-test path for the release environment,
  record the tested distro, kernel, and compositor versions, and list known
  limitations. Run the nested overlay harness and document the contributor
  path for testing new environments.

The SDDM preview may ship alongside the overlay. Touch-only unlock,
standalone sleep/wake sign-off, Desk Mode, and additional compositor ports
are not `v0.1.0` release gates.

## Resolved Architectural Decisions

- **Organization & Repository:** Hosted under the dedicated **`github.com/veilshell`** organization (`https://github.com/veilshell/veilshell`) to welcome community maintainers and future companion repos.
- **License & Clean-Room Policy:** **`GPL-3.0-or-later`** (matching `denialwm/denial`), written from scratch in Rust with `#![forbid(unsafe_code)]` outside `veilshell-wayland`, which permits one isolated `raw-window-handle` `wgpu::Surface` bridge. Upstream inspirations are credited in [NOTICE.md](../NOTICE.md).
- **Renderer Architecture:** Shared renderer-agnostic glassmorphic `DrawList` + hit-test `SceneOutput` (`veilshell-render`), backed by both a hardware-accelerated `wgpu` Vulkan/GLES WGSL instanced SDF renderer (`WgpuRenderer`, zero-copy `wgpu::Surface` Wayland presentation + blur-once GPU static wallpaper glass cache) and a zero-`unsafe` CPU analytic SDF fallback (`SdfSoftwareRenderer`), selectable via `--renderer auto|wgpu|software` with automated GPU-vs-CPU pixel-parity tests.
- **Minimum Supported Rust Version:** `1.86+` (aligned with `smithay-client-toolkit 0.21`).

## Implementation history

### Phase 0 — Foundation & Dev Harness (Complete)
- [x] Modular 7-crate Cargo workspace (`veilshell-core`, `veilshell-compositor`, `veilshell-hw`, `veilshell-theme`, `veilshell-wayland`, `veilshell-render`, `veilshell-shell`)
- [x] `GPL-3.0-or-later` `LICENSE`, `NOTICE.md`, `CONTRIBUTING.md`, `ARCHITECTURE.md`, `PERFORMANCE_BUDGET.md`, `HARDWARE_PROFILES.md`, `PORTING_A_COMPOSITOR.md`
- [x] `veilshell-core`: critically damped spring physics, bottom-pill gesture classifier, Laptop↔Tablet mode state machine, orientation/rotation lock, WCAG AA theme primitives
- [x] `veilshell-theme`: `ThemeProvider` trait + live Omarchy Aether `colors.toml` parser + dark fallback
- [x] Reference device profile: `hardware-profiles/microsoft-surface-go-3.toml` + zero-dep TOML parser
- [x] GitHub Actions CI (`rustfmt`, `clippy::pedantic -D warnings`, `cargo test --workspace`, Tier 2 visual snapshots)

### Phase 1 — Live Hyprland IPC, Glassmorphic SDF Renderer & `wlr-layer-shell` Client (Complete)
- [x] `veilshell-compositor`: live `HyprlandBackend` over Unix sockets (`j/clients`, `j/monitors`, `dispatch`) + real JSON fixture tests
- [x] `veilshell-render`: renderer-agnostic `DrawList`, glassmorphic layouts for all 5 scenes (`pill`, `home`, `recents`, `shade`, `osk`) in landscape & portrait, analytic SDF rasterizer, built-in HUD glyph renderer, `ARGB8888` SHM encoder, and `PNG` exporter
- [x] `veilshell-wayland`: `smithay-client-toolkit 0.21` `wlr-layer-shell` surface client with `SlotPool` SHM commits and `wl_touch` + `wl_pointer` gesture capture
- [x] `veilctl snapshot` CLI + Tier 2 snapshot script (`scripts/test-snapshots.sh`) + Tier 3 Hyprland headless-output + `grim` harness (`scripts/test-headless-hyprland.sh`)

### Phase 2 — Interactive Touch Shell & Live Hardware Intelligence

#### Sprint 2A — Interactive Multi-Surface Shell, `.desktop` Launcher & `veilctl` Daemon IPC (Complete)
- [x] Event-driven `calloop` + `WaylandSource` daemon managing persistent `BottomPill` + `TopStrip` and lazy full-screen `Overlay` (`Layer::Overlay`, output pinning via `--output`)
- [x] Interactive hit-testing (`SceneOutput::hit_test`) for Home tiles/pagination dots, Recents tap-to-focus & `X`-to-close, Quick Settings toggles & draggable sliders, and spring-animated touch transitions
- [x] Pure `.desktop` parser (`veilshell-core::apps`) + user/system application discovery (`veilctl apps`) launching via `HyprlandBackend::launch_command`
- [x] Dual Omarchy 4.5+ Lua `eval` (`hl.config` / `hl.monitor` / `hl.layer_rule`) and legacy `.conf` `keyword` Hyprland compatibility + `adapters/omarchy/veilshell.lua`
- [x] Live Unix control socket (`$XDG_RUNTIME_DIR/veilshell.sock`) powering `veilctl show`, `veilctl mode`, `veilctl status`, `veilctl reload-theme`, and `veilctl quit`
- [x] Cached-backdrop fast SDF rasterizer & verified non-blank Tier 3 live Hyprland screenshots (`home`, `shade`, `recents`, `pill`)

#### Sprint 2B — Live Hardware Intelligence, Quick Settings Controls & Theme Auto-Reload (Complete)
- [x] `veilshell-hw` Linux `sysfs`/CLI backend: detachable USB Type Cover auto-switching via DMI-matched profiles and `iio-sensor-proxy` auto-rotation with rotation lock in standalone mode.
- [x] Live Quick Settings hardware telemetry & controls: Wi-Fi and WWAN, Bluetooth, system battery and backlight, PipeWire volume, Night Light via the session's `hyprsunset` when available, and `veilctl hw` inspection.
- [x] Adaptive Laptop/Tablet startup (`veilshell --policy <auto|laptop|tablet>`, `initial_visible: false` in Laptop Mode), live Aether `colors.toml` mtime watcher, Omarchy `adapters/omarchy/hooks/30-veilshell-theme` reload hook, and optional `adapters/omarchy/systemd/veilshell.service` user unit

### Phase 3 — Built-In Touch OSK, GPU (`wgpu`) Acceleration & Native Visual Polish

#### Sprint 3A — Built-In Touch OSK (`zwp_input_method_v2` + `zwp_virtual_keyboard_v1`) (Complete)
- [x] `veilshell-core::osk`: pure-Rust `OskState` across `Letters` (Shift OneShot / CapsLock / one-shot Ctrl), `Numbers`, and `Symbols` layers, `char_to_evdev` mapping, and `xkbcli`-validated self-contained `US_QWERTY_XKB_KEYMAP`
- [x] `veilshell-render`: dynamic `ctx.osk_state` rendering + active modifier accent highlights and full 5×7 symbol glyph set (`@`, `#`, `$`, `&`, `*`, `=`, quotes, brackets, braces, `~`, `` ` ``, `^`, etc.)
- [x] `veilshell-wayland`: `zwp_input_method_v2` + `zwp_virtual_keyboard_v1` integration (auto-pop on text focus in Tablet Mode, numeric layer switch on `ContentPurpose`, direct `commit_string` + virtual-keyboard fallback for special keys & Ctrl combos, non-overlapping bottom-pill hide during `TouchOsk`, downward-swipe dismiss)
- [x] Tier 3 live Hyprland verification updated to capture & verify all 5 surfaces (`home`, `shade`, `recents`, `osk`, `pill`) cleanly

#### Sprint 3B — GPU (`wgpu`) Acceleration, Typography/Icons & Live Recents Thumbnails (Complete)
- [x] `wgpu` Vulkan/GLES WGSL SDF renderer backend consuming the existing `DrawList` + pre-blurred static wallpaper glass cache (blur once on wallpaper change) + zero-copy `wgpu::Surface` Wayland presentation & automatic `SdfSoftwareRenderer` fallback (preferring Vulkan before GLES so lazy XWayland sessions never stall)
- [x] System TrueType/OpenType fonts (with 5×7 HUD fallback) and freedesktop PNG/SVG app icons. Wayland surfaces render at the compositor's integer or fractional output scale without changing its settings.
- [x] Live window thumbnails in the Recents Deck via `ext-foreign-toplevel-list-v1` + `ext-image-copy-capture-v1` (with `wl_shm` format conversion & premultiplied area-box downscaling)

## Standalone and feature follow-up

The long-term standalone goal is a touch-only daily driver with its own
session policy: fullscreen defaults for small screens, a keyboard-and-mouse
Desk Mode, and touch access to essential desktop services. Reuse the overlay's
UI and keep these policies behind the dedicated session boundary. Standalone
Desk Mode is a layout choice inside that session, not the overlay's Laptop
Mode, which simply hides VeilShell.

### Standalone SDDM session, lock and power

- [x] [Preview SDDM entry](../adapters/session/README.md), dedicated Hyprland config, session-scoped daemon and launcher lifecycle tests. Keep the greeter, autologin, and existing desktop configuration unchanged.
- [x] Preview-only touch power menu with Lock, Suspend, Logout, Restart and Shut down. Confirm session-ending actions and refuse touch Suspend without compositor lock acknowledgment.
- [ ] Verify lock-before-suspend, resume and real power actions on hardware.
- [x] Interim Hyprlock password locker and Hypridle configuration scoped to the preview session.
- [x] Verify native session-lock acknowledgement and locker-death behavior on the guest's visible output with an uninstalled protocol probe.
- [x] SDDM guest login and touch logout smoke test with a guest-only software override.
- [x] Verify stock SDDM login and normal logout with automatic software fallback in the Virtio guest.
- [ ] Native Denial-style lock face: wallpaper, clock/date, swipe-up touch authentication, password fallback, PIN and optional fingerprint. Require compositor-enforced locking.
- [x] Optional session-scoped XDG autostart helper with an explicit allowlist.
- [ ] Finish session-scoped desktop services, including polkit prompts, notifications, and file-picker and screen-sharing portals.
- [x] Switch between standalone Desk and Tablet layouts without closing apps or changing the overlay. Suppress automatic app keyboard in Desk Mode.
- [x] Add a Desk Mode panel with launch, Recents and Quick Settings pills.
- [x] Built-in standalone Settings for startup mode, panel visibility, wallpaper presets and shortcut profile.
- [ ] Add standalone Tablet window defaults, taskbar, Dashboard, Welcome, and broader system settings. See the [Denial out-of-box gaps](DENIAL_CALL_MAP.md#out-of-box-gap-map).

### Shared tablet experience

- [x] Finger-following Home and Quick Settings reveals with reversible drags and spring settling, plus interactive Home paging.
- [x] Responsive control center: Connectivity, Display & Sound, and Device sections, compacting to the available controls and using both columns in landscape.
- [x] Denial-style Recents deck: frameless live window previews with app icon/name above each card, higher-resolution captures, swipe paging, and swipe-up-to-close.
- [ ] Notification center: actionable banners, history, dismissal, and lock-screen privacy.
- [ ] Touch Wi-Fi network selection and Bluetooth pairing, not just radio toggles.
- [x] Keyboard hold-to-repeat for Backspace and arrows, and a terminal utility row with Escape, Tab and all four arrows.
- [ ] Keyboard long-press accents and optional local suggestions that are disabled for sensitive fields.
- [ ] Editable Home: reorder icons, create folders, persist layout, share a dock across pages, and open a separate alphabetical app drawer.
- [x] Desktop launcher with search and physical-keyboard navigation, separate from Tablet Home.
- [ ] Desktop launcher recent suggestions and pointer scrolling.

### External displays

- [ ] Give the built-in touchscreen and external monitor separate mobile and desktop roles. Move windows between outputs from Recents; verify hotplug recovery.
- [ ] Optional Input Deck: use the built-in touchscreen as a keyboard and relative touchpad while an external display is active.

### Portability follow-up

- [x] Reject live startup without a working Hyprland backend instead of falling back to mock data.
- [ ] Expose capability-based fallbacks in the UI.
- [ ] Add a read-only `veilctl doctor` to report protocol, compositor, and service capabilities and why features are unavailable.
- [ ] Verify the adapter contract with a second live compositor (e.g. Niri or Sway) before considering a stable external plugin API.
- [ ] Arch Linux / AUR `PKGBUILD` (`packaging/arch/PKGBUILD`).
- [ ] Expand package smoke tests and community hardware profiles based on recorded test results.
- [ ] Add frame-time and RSS assertions where the test environment supports repeatable measurements.

See the [Denial reference](DENIAL_PARITY_SPEC.md) for upstream behavior and
standalone acceptance criteria, and the [validation record](DENIAL_VALIDATION.md)
for test evidence.
