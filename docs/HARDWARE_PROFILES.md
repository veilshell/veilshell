# Hardware Profiles

VeilShell stays device-agnostic in its core crates and loads per-tablet defaults from declarative TOML files under `hardware-profiles/` (embedded at build time and overridable at runtime).

## Runtime DMI Matching & Overrides

On startup (`LinuxHardware::auto_detect()`), VeilShell reads `/sys/class/dmi/id/sys_vendor` and `/sys/class/dmi/id/product_name`:

1. Scans user override files first in `$HOME/.config/veilshell/hardware-profiles/*.toml` (letting users tune a tablet without recompiling).
2. Falls back to the built-in embedded profiles (`hardware-profiles/*.toml`).
3. Applies the matched `[display]` scale, `[mode_switch]` detachable USB keyboard `(usb_vendor_id, usb_product_id)` polling, and `[sensors].auto_rotate_in_tablet_mode` behavior automatically.

You can verify which profile and live hardware telemetry matched on your machine at any time:

```bash
veilctl hw
```

## Schema

```toml
[device]
id = "vendor-model"
vendor = "Vendor Name"
product = "Model Name"
architecture = "x86_64" # or "aarch64"

[display]
connector_prefix = "eDP-"
preferred_scale = 1.5
refresh_hz = 60

[touch]
device_name = "Exact libinput touch device name"
stylus_name = "Optional stylus device name"
double_tap_wake_dpms = false

[mode_switch]
# Supported strategies: "manual", "usb_keyboard_presence", "sw_tablet_mode"
strategy = "usb_keyboard_presence"
usb_vendor_id = "045e"
usb_product_id = "09b5"

[sensors]
backend = "iio-sensor-proxy"
auto_rotate_in_tablet_mode = true
swap_left_right = false
```

## Contributing a Profile

1. Copy `hardware-profiles/microsoft-surface-go-3.toml` to `hardware-profiles/<vendor>-<model>.toml`.
2. Fill in your `/sys/class/dmi/id/{sys_vendor,product_name}`, `lsusb`, `libinput list-devices`, and output connector details.
3. Register it in `HardwareProfile::built_in_profiles()` in `crates/veilshell-hw/src/lib.rs` and add a matching unit test (`cargo test -p veilshell-hw`).
