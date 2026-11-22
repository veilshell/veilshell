//! Live `Linux` hardware telemetry readers, Quick Settings controllers, and `iio-sensor-proxy` watcher.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use veilshell_core::Orientation;

use crate::{
    HardwareBackend, HardwareController, HardwareError, HardwareProfile, HardwareSnapshot,
};

const SYSFS_POWER_SUPPLY: &str = "/sys/class/power_supply";
const SYSFS_BACKLIGHT: &str = "/sys/class/backlight";
const SYSFS_USB_DEVICES: &str = "/sys/bus/usb/devices";
const SYSFS_DMI_VENDOR: &str = "/sys/class/dmi/id/sys_vendor";
const SYSFS_DMI_PRODUCT: &str = "/sys/class/dmi/id/product_name";
const SYSFS_RFKILL: &str = "/sys/class/rfkill";
const OMARCHY_NIGHTLIGHT_BIN: &str = "/usr/share/omarchy/bin/omarchy-toggle-nightlight";

/// Live `Linux` hardware telemetry and control implementation using `sysfs` and standard `CLI` tools.
#[derive(Clone, Debug)]
pub struct LinuxHardware {
    profile: Option<HardwareProfile>,
    usb_keyboard_id: Option<(String, String)>,
}

impl Default for LinuxHardware {
    fn default() -> Self {
        Self::auto_detect()
    }
}

impl LinuxHardware {
    /// Construct a [`LinuxHardware`] instance with an optional explicit [`HardwareProfile`].
    #[must_use]
    pub fn new(profile: Option<HardwareProfile>) -> Self {
        let usb_keyboard_id = profile
            .as_ref()
            .and_then(HardwareProfile::usb_keyboard_ids)
            .map(|(v, p)| (v.to_owned(), p.to_owned()));
        Self {
            profile,
            usb_keyboard_id,
        }
    }

    /// Detect the host device via `DMI` and match any user or built-in [`HardwareProfile`].
    #[must_use]
    pub fn auto_detect() -> Self {
        Self::new(load_matching_profile())
    }

    /// Return the matched [`HardwareProfile`] if this host was recognized.
    #[must_use]
    pub const fn profile(&self) -> Option<&HardwareProfile> {
        self.profile.as_ref()
    }

    /// Return whether a detachable `USB` Type Cover / keyboard dock is currently enumerated.
    #[must_use]
    pub fn detect_keyboard_attached(&self) -> bool {
        match &self.usb_keyboard_id {
            Some((vid, pid)) => is_usb_device_present_in(Path::new(SYSFS_USB_DEVICES), vid, pid),
            None => true,
        }
    }

    /// Return whether the host exposes an active `WWAN` / cellular device or `nmcli` gsm slot.
    #[must_use]
    pub fn has_wwan_hardware(&self) -> bool {
        if let Ok(entries) = fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("ww") {
                    return true;
                }
            }
        }
        run_stdout("nmcli", &["-t", "-f", "TYPE", "dev", "status"])
            .is_some_and(|out| out.lines().any(|l| l.trim() == "gsm"))
    }
}

impl HardwareBackend for LinuxHardware {
    fn snapshot(&self) -> HardwareSnapshot {
        let radios = run_stdout("nmcli", &["-t", "-f", "WIFI,WWAN", "general"])
            .and_then(|out| parse_nmcli_general(&out));

        let devices = run_stdout(
            "nmcli",
            &["-t", "-f", "TYPE,STATE,CONNECTION", "dev", "status"],
        )
        .unwrap_or_default();
        let (wifi_ssid, gsm_signal) = parse_nmcli_devices(&devices);
        let wifi_radio = radios
            .map(|(wifi, _)| wifi)
            .filter(|_| devices.lines().any(|line| line.starts_with("wifi:")));
        let mobile_radio = radios.map(|(_, mobile)| mobile).filter(|_| {
            devices
                .lines()
                .any(|line| line.starts_with("gsm:") || line.starts_with("wwan:"))
        });

        let wifi_connected = wifi_radio == Some(true) && wifi_ssid.is_some();
        let lte_signal_pct = if mobile_radio == Some(true) {
            gsm_signal
        } else {
            None
        };

        let bluetooth = read_bluetooth_enabled();
        let (battery_pct, charging) = read_sysfs_battery().unwrap_or((100, false));
        let brightness = read_sysfs_backlight();
        let volume = run_stdout("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .and_then(|out| parse_wpctl_volume(&out));
        let (volume_pct, muted) = volume.unwrap_or((50, false));
        let night_light = read_hyprsunset_enabled();
        let keyboard_attached = self.detect_keyboard_attached();

        HardwareSnapshot {
            wifi_connected,
            wifi_radio,
            wifi_ssid,
            bluetooth_enabled: bluetooth.unwrap_or(false),
            bluetooth_available: bluetooth.is_some(),
            lte_signal_pct,
            mobile_radio,
            battery_pct,
            charging,
            brightness_pct: brightness.unwrap_or(80),
            brightness_available: brightness.is_some(),
            volume_pct,
            volume_available: volume.is_some(),
            muted,
            night_light,
            keyboard_attached,
            orientation: Orientation::Normal,
        }
    }
}

impl HardwareController for LinuxHardware {
    fn set_wifi(&mut self, enabled: bool) -> Result<(), HardwareError> {
        let arg = if enabled { "on" } else { "off" };
        run_status("nmcli", &["radio", "wifi", arg])
    }

    fn set_bluetooth(&mut self, enabled: bool) -> Result<(), HardwareError> {
        if enabled {
            let _ = run_status("rfkill", &["unblock", "bluetooth"]);
        }
        let arg = if enabled { "on" } else { "off" };
        run_status("bluetoothctl", &["power", arg])
    }

    fn set_wwan(&mut self, enabled: bool) -> Result<(), HardwareError> {
        let arg = if enabled { "on" } else { "off" };
        run_status("nmcli", &["radio", "wwan", arg])
    }

    fn set_brightness(&mut self, pct: u8) -> Result<(), HardwareError> {
        let clamped = pct.clamp(5, 100);
        run_status("brightnessctl", &["-q", "set", &format!("{clamped}%")])
    }

    fn set_volume(&mut self, pct: u8) -> Result<(), HardwareError> {
        let clamped = pct.min(100);
        run_status(
            "wpctl",
            &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{clamped}%")],
        )
    }

    fn set_night_light(&mut self, enabled: bool) -> Result<(), HardwareError> {
        if read_hyprsunset_enabled() == enabled {
            return Ok(());
        }
        if Path::new(OMARCHY_NIGHTLIGHT_BIN).exists() {
            return run_status(OMARCHY_NIGHTLIGHT_BIN, &[]);
        }
        if run_stdout("which", &["omarchy-toggle-nightlight"]).is_some() {
            return run_status("omarchy-toggle-nightlight", &[]);
        }
        if enabled {
            run_status("hyprctl", &["hyprsunset", "temperature", "4000"])
        } else {
            run_status("hyprctl", &["hyprsunset", "identity"])
        }
    }
}

/// Read `(sys_vendor, product_name)` from `/sys/class/dmi/id/`.
#[must_use]
pub fn detect_dmi_device() -> Option<(String, String)> {
    let vendor = fs::read_to_string(SYSFS_DMI_VENDOR).ok()?;
    let product = fs::read_to_string(SYSFS_DMI_PRODUCT).ok()?;
    let v = vendor.trim().to_owned();
    let p = product.trim().to_owned();
    if v.is_empty() || p.is_empty() {
        return None;
    }
    Some((v, p))
}

/// Match user-supplied (`$HOME/.config/veilshell/hardware-profiles/*.toml`) or built-in [`HardwareProfile`].
#[must_use]
pub fn load_matching_profile() -> Option<HardwareProfile> {
    let (vendor, product) = detect_dmi_device()?;
    let mut profiles = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let user_dir = PathBuf::from(home)
            .join(".config")
            .join("veilshell")
            .join("hardware-profiles");
        if let Ok(entries) = fs::read_dir(user_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    if let Ok(raw) = fs::read_to_string(path) {
                        if let Ok(parsed) = HardwareProfile::parse_toml(&raw) {
                            profiles.push(parsed);
                        }
                    }
                }
            }
        }
    }
    profiles.extend(HardwareProfile::built_in_profiles());
    HardwareProfile::match_dmi(&profiles, &vendor, &product).cloned()
}

/// Read system battery `(percentage, charging)` from `/sys/class/power_supply/`, ignoring peripheral `Device` batteries.
#[must_use]
pub fn read_sysfs_battery() -> Option<(u8, bool)> {
    read_sysfs_battery_from(Path::new(SYSFS_POWER_SUPPLY))
}

/// Inner testable implementation of [`read_sysfs_battery`] over an arbitrary directory.
#[must_use]
pub fn read_sysfs_battery_from(root: &Path) -> Option<(u8, bool)> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();

    for dir in dirs {
        let kind = fs::read_to_string(dir.join("type")).unwrap_or_default();
        if kind.trim() != "Battery" {
            continue;
        }
        // Skip peripheral/stylus batteries (e.g. Surface Pen HID reports `scope = Device`, `capacity = 0`).
        let scope = fs::read_to_string(dir.join("scope")).unwrap_or_default();
        if scope.trim().eq_ignore_ascii_case("Device") {
            continue;
        }
        let Ok(cap_raw) = fs::read_to_string(dir.join("capacity")) else {
            continue;
        };
        let Ok(pct) = cap_raw.trim().parse::<u8>() else {
            continue;
        };
        let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
        let charging = status.trim().eq_ignore_ascii_case("Charging");
        return Some((pct.min(100), charging));
    }
    None
}

/// Read primary backlight percentage (`0..=100`) from `/sys/class/backlight/`.
#[must_use]
pub fn read_sysfs_backlight() -> Option<u8> {
    read_sysfs_backlight_from(Path::new(SYSFS_BACKLIGHT))
}

/// Inner testable implementation of [`read_sysfs_backlight`] over an arbitrary directory.
#[must_use]
pub fn read_sysfs_backlight_from(root: &Path) -> Option<u8> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();

    for dir in dirs {
        let Ok(cur_raw) = fs::read_to_string(dir.join("brightness")) else {
            continue;
        };
        let Ok(max_raw) = fs::read_to_string(dir.join("max_brightness")) else {
            continue;
        };
        let Ok(cur) = cur_raw.trim().parse::<u64>() else {
            continue;
        };
        let Ok(max) = max_raw.trim().parse::<u64>() else {
            continue;
        };
        if max == 0 {
            continue;
        }
        let pct = ((cur * 100) + (max / 2)) / max;
        return Some(u8::try_from(pct.min(100)).unwrap_or(100));
    }
    None
}

/// Return whether a `USB` device matching `(vendor_hex, product_hex)` is currently present under `root`.
#[must_use]
pub fn is_usb_device_present_in(root: &Path, vendor_hex: &str, product_hex: &str) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(vid) = fs::read_to_string(dir.join("idVendor")) else {
            continue;
        };
        let Ok(pid) = fs::read_to_string(dir.join("idProduct")) else {
            continue;
        };
        if vid.trim().eq_ignore_ascii_case(vendor_hex.trim())
            && pid.trim().eq_ignore_ascii_case(product_hex.trim())
        {
            return true;
        }
    }
    false
}

/// Parse `wpctl get-volume @DEFAULT_AUDIO_SINK@` output into `(volume_pct, muted)`.
#[must_use]
pub fn parse_wpctl_volume(output: &str) -> Option<(u8, bool)> {
    let muted = output.contains("[MUTED]");
    let rest = output.split("Volume:").nth(1)?;
    let num_token = rest.split_whitespace().next()?;
    let level: f32 = num_token.parse().ok()?;
    let pct_f = (level * 100.0).round().clamp(0.0, 100.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pct = pct_f as u8;
    Some((pct, muted))
}

/// Parse `nmcli -t -f WIFI,WWAN general` output into `(wifi_radio_enabled, wwan_radio_enabled)`.
#[must_use]
pub fn parse_nmcli_general(output: &str) -> Option<(bool, bool)> {
    let line = output.lines().find(|l| !l.trim().is_empty())?.trim();
    let (wifi, wwan) = line.split_once(':')?;
    Some((wifi.trim() == "enabled", wwan.trim() == "enabled"))
}

/// Parse `nmcli -t -f TYPE,STATE,CONNECTION dev status` into `(wifi_ssid, gsm_signal_pct)`.
#[must_use]
pub fn parse_nmcli_devices(output: &str) -> (Option<String>, Option<u8>) {
    let mut ssid = None;
    let mut gsm_signal = None;
    for line in output.lines() {
        let mut parts = line.splitn(3, ':');
        let Some(dev_type) = parts.next() else {
            continue;
        };
        let Some(state) = parts.next() else {
            continue;
        };
        let conn = parts.next().unwrap_or("").trim();
        if dev_type.trim() == "wifi" && state.trim() == "connected" && !conn.is_empty() {
            ssid = Some(conn.to_owned());
        } else if dev_type.trim() == "gsm" && state.trim() == "connected" {
            gsm_signal = Some(80);
        }
    }
    (ssid, gsm_signal)
}

/// Parse `bluetoothctl show` output into controller `Powered` state.
#[must_use]
pub fn parse_bluetoothctl_powered(output: &str) -> Option<bool> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Powered:") {
            return match rest.trim() {
                "yes" => Some(true),
                "no" => Some(false),
                _ => None,
            };
        }
    }
    None
}

/// Parse `hyprctl hyprsunset temperature` output into Kelvin temperature when available.
#[must_use]
pub fn parse_hyprsunset_temp(output: &str) -> Option<u32> {
    let trimmed = output.trim();
    let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Parse a single line emitted by `monitor-sensor --accel` into an [`Orientation`].
#[must_use]
pub fn parse_monitor_sensor_line(line: &str) -> Option<Orientation> {
    let trimmed = line.trim();
    if let Some(raw) = trimmed.strip_prefix("Accelerometer orientation changed:") {
        return Orientation::from_sensor_str(raw.trim());
    }
    if let Some(after) = trimmed.split("orientation:").nth(1) {
        let token = after.split([',', ')']).next()?.trim();
        return Orientation::from_sensor_str(token);
    }
    None
}

/// Background guard owning a `monitor-sensor --accel` child process that terminates cleanly on drop.
pub struct SensorGuard {
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    thread: Option<JoinHandle<()>>,
}

impl SensorGuard {
    /// Spawn `monitor-sensor --accel` and invoke `on_orientation` whenever orientation changes.
    ///
    /// Returns `None` if `monitor-sensor` cannot be launched on this host.
    pub fn spawn<F>(mut on_orientation: F) -> Option<Self>
    where
        F: FnMut(Orientation) + Send + 'static,
    {
        let mut proc_child = Command::new("monitor-sensor")
            .arg("--accel")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let stdout = proc_child.stdout.take()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let child_slot = Arc::new(Mutex::new(Some(proc_child)));

        let thread = thread::Builder::new()
            .name("veilshell-sensor".into())
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(line) = line else {
                        break;
                    };
                    if let Some(ori) = parse_monitor_sensor_line(&line) {
                        on_orientation(ori);
                    }
                }
            })
            .ok()?;

        Some(Self {
            stop,
            child: child_slot,
            thread: Some(thread),
        })
    }
}

impl Drop for SensorGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn read_bluetooth_enabled() -> Option<bool> {
    // Fast-path check via rfkill sysfs: if hard or soft blocked, BlueZ cannot be powered.
    if let Ok(entries) = fs::read_dir(SYSFS_RFKILL) {
        let mut saw_bt = false;
        let mut all_unblocked = true;
        for entry in entries.flatten() {
            let dir = entry.path();
            let kind = fs::read_to_string(dir.join("type")).unwrap_or_default();
            if kind.trim() == "bluetooth" {
                saw_bt = true;
                let soft = fs::read_to_string(dir.join("soft")).unwrap_or_default();
                let hard = fs::read_to_string(dir.join("hard")).unwrap_or_default();
                if soft.trim() != "0" || hard.trim() != "0" {
                    all_unblocked = false;
                }
            }
        }
        if saw_bt && !all_unblocked {
            return Some(false);
        }
    }

    run_stdout("bluetoothctl", &["show"]).and_then(|out| parse_bluetoothctl_powered(&out))
}

fn read_hyprsunset_enabled() -> bool {
    run_stdout("hyprctl", &["hyprsunset", "temperature"])
        .and_then(|out| parse_hyprsunset_temp(&out))
        .is_some_and(|kelvin| kelvin < 6000)
}

fn run_stdout(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run_status(bin: &str, args: &[&str]) -> Result<(), HardwareError> {
    let tool = leaked_static_name(bin);
    let out = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                HardwareError::Unavailable(tool)
            } else {
                HardwareError::CommandFailed {
                    tool,
                    detail: err.to_string(),
                }
            }
        })?;
    if !out.status.success() {
        let detail = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(HardwareError::CommandFailed {
            tool,
            detail: if detail.is_empty() {
                format!("exited with status {}", out.status)
            } else {
                detail
            },
        });
    }
    Ok(())
}

fn leaked_static_name(bin: &str) -> &'static str {
    if bin.ends_with("omarchy-toggle-nightlight") {
        "omarchy-toggle-nightlight"
    } else if bin.ends_with("brightnessctl") {
        "brightnessctl"
    } else if bin.ends_with("bluetoothctl") {
        "bluetoothctl"
    } else if bin.ends_with("hyprctl") {
        "hyprctl"
    } else if bin.ends_with("rfkill") {
        "rfkill"
    } else if bin.ends_with("nmcli") {
        "nmcli"
    } else if bin.ends_with("wpctl") {
        "wpctl"
    } else {
        "hardware-cli"
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("veilshell-hw-test-{name}-{nanos}"));
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn parses_wpctl_volume_and_mute() {
        assert_eq!(parse_wpctl_volume("Volume: 0.44\n"), Some((44, false)));
        assert_eq!(
            parse_wpctl_volume("Volume: 0.00 [MUTED]\n"),
            Some((0, true))
        );
        assert_eq!(parse_wpctl_volume("Volume: 1.25\n"), Some((100, false)));
        assert_eq!(parse_wpctl_volume("garbage"), None);
    }

    #[test]
    fn parses_nmcli_general_and_devices() {
        assert_eq!(
            parse_nmcli_general("enabled:disabled\n"),
            Some((true, false))
        );
        assert_eq!(
            parse_nmcli_general("disabled:enabled\n"),
            Some((false, true))
        );

        let dev_out = "\
wifi:connected:VOYAGER
loopback:connected (externally):lo
gsm:disconnected:
wifi-p2p:disconnected:
";
        assert_eq!(
            parse_nmcli_devices(dev_out),
            (Some("VOYAGER".to_owned()), None)
        );
        assert_eq!(
            parse_nmcli_devices("wifi:disconnected:\ngsm:connected:T-Mobile\n"),
            (None, Some(80))
        );
    }

    #[test]
    fn parses_bluetoothctl_and_hyprsunset() {
        let bt_on = "Controller AA:BB:CC:DD:EE:FF\n\tName: surface-go3\n\tPowered: yes\n";
        let bt_off = "Controller AA:BB:CC:DD:EE:FF\n\tPowered: no\n";
        assert_eq!(parse_bluetoothctl_powered(bt_on), Some(true));
        assert_eq!(parse_bluetoothctl_powered(bt_off), Some(false));
        assert_eq!(parse_bluetoothctl_powered("No default controller"), None);

        assert_eq!(parse_hyprsunset_temp("4000\n"), Some(4000));
        assert_eq!(parse_hyprsunset_temp("6500K"), Some(6500));
        assert_eq!(parse_hyprsunset_temp("Couldn't connect"), None);
    }

    #[test]
    fn parses_monitor_sensor_lines() {
        assert_eq!(
            parse_monitor_sensor_line("    Accelerometer orientation changed: normal"),
            Some(Orientation::Normal)
        );
        assert_eq!(
            parse_monitor_sensor_line("    Accelerometer orientation changed: left-up"),
            Some(Orientation::Rotate90)
        );
        assert_eq!(
            parse_monitor_sensor_line(
                "=== Has accelerometer (orientation: right-up, tilt: tilted-up)"
            ),
            Some(Orientation::Rotate270)
        );
        assert_eq!(
            parse_monitor_sensor_line(
                "=== Has accelerometer (orientation: undefined, tilt: undefined)"
            ),
            None
        );
        assert_eq!(
            parse_monitor_sensor_line("    Tilt changed: tilted-down"),
            None
        );
    }

    #[test]
    fn sysfs_battery_ignores_stylus_device_scope() {
        let root = temp_dir("bat");
        let bat1 = root.join("BAT1");
        let stylus = root.join("hid-0018:04F3:2A1C.0005-battery-7");
        fs::create_dir_all(&bat1).expect("bat1 dir");
        fs::create_dir_all(&stylus).expect("stylus dir");

        fs::write(stylus.join("type"), "Battery\n").expect("write");
        fs::write(stylus.join("scope"), "Device\n").expect("write");
        fs::write(stylus.join("capacity"), "0\n").expect("write");
        fs::write(stylus.join("status"), "Unknown\n").expect("write");

        fs::write(bat1.join("type"), "Battery\n").expect("write");
        fs::write(bat1.join("scope"), "\n").expect("write");
        fs::write(bat1.join("capacity"), "73\n").expect("write");
        fs::write(bat1.join("status"), "Charging\n").expect("write");

        // Ensure alphabetical priority still skips stylus even if stylus directory sorts before BAT1.
        let adapter = root.join("AC");
        fs::create_dir_all(&adapter).expect("ac dir");
        fs::write(adapter.join("type"), "Mains\n").expect("write");

        assert_eq!(read_sysfs_battery_from(&root), Some((73, true)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sysfs_backlight_and_usb_presence_work() {
        let root = temp_dir("bl_usb");
        let bl = root.join("backlight").join("intel_backlight");
        fs::create_dir_all(&bl).expect("bl dir");
        fs::write(bl.join("brightness"), "6000\n").expect("write");
        fs::write(bl.join("max_brightness"), "7500\n").expect("write");
        assert_eq!(read_sysfs_backlight_from(&root.join("backlight")), Some(80));

        let usb = root.join("usb").join("1-7");
        fs::create_dir_all(&usb).expect("usb dir");
        fs::write(usb.join("idVendor"), "045e\n").expect("write");
        fs::write(usb.join("idProduct"), "09b5\n").expect("write");
        assert!(is_usb_device_present_in(&root.join("usb"), "045E", "09B5"));
        assert!(!is_usb_device_present_in(&root.join("usb"), "045e", "9999"));
        let _ = fs::remove_dir_all(&root);
    }
}
