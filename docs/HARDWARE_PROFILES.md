# Hardware Profiles

VeilShell stays device-agnostic in its core crates and loads per-tablet defaults from declarative TOML files under `hardware-profiles/`.

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
2. Fill in your `libinput list-devices`, `lsusb`, and output connector details.
3. Add a unit test in `crates/veilshell-hw/src/lib.rs` (or run `cargo test -p veilshell-hw`) to validate your TOML file.
