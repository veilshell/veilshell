//! Hardware telemetry snapshots, live Linux backends, control actions, and declarative device profiles.

pub mod linux;

use std::fmt;

use veilshell_core::Orientation;

pub use linux::{detect_dmi_device, load_matching_profile, LinuxHardware, SensorGuard};

/// Errors returned by [`HardwareController`] operations when applying live settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HardwareError {
    /// Underlying system CLI or `sysfs` write failed.
    CommandFailed { tool: &'static str, detail: String },
    /// Required system tool or hardware device is unavailable on this host.
    Unavailable(&'static str),
}

impl fmt::Display for HardwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandFailed { tool, detail } => {
                write!(f, "hardware command '{tool}' failed: {detail}")
            }
            Self::Unavailable(msg) => write!(f, "hardware control unavailable: {msg}"),
        }
    }
}

impl std::error::Error for HardwareError {}

/// Point-in-time hardware status consumed by the shell and quick settings shade.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct HardwareSnapshot {
    pub wifi_connected: bool,
    pub wifi_radio: Option<bool>,
    pub wifi_ssid: Option<String>,
    pub bluetooth_enabled: bool,
    pub bluetooth_available: bool,
    pub lte_signal_pct: Option<u8>,
    pub mobile_radio: Option<bool>,
    pub battery_pct: u8,
    pub charging: bool,
    pub brightness_pct: u8,
    pub brightness_available: bool,
    pub volume_pct: u8,
    pub volume_available: bool,
    pub muted: bool,
    pub night_light: bool,
    pub night_light_available: bool,
    pub keyboard_attached: bool,
    pub orientation: Orientation,
}

impl Default for HardwareSnapshot {
    fn default() -> Self {
        Self {
            wifi_connected: true,
            wifi_radio: Some(true),
            wifi_ssid: Some("Offline".into()),
            bluetooth_enabled: false,
            bluetooth_available: true,
            lte_signal_pct: None,
            mobile_radio: None,
            battery_pct: 85,
            charging: false,
            brightness_pct: 60,
            brightness_available: true,
            volume_pct: 50,
            volume_available: true,
            muted: false,
            night_light: false,
            night_light_available: true,
            keyboard_attached: true,
            orientation: Orientation::Normal,
        }
    }
}

/// Read-only hardware telemetry source (`iio-sensor-proxy`, `sysfs`, `NetworkManager`, etc.).
pub trait HardwareBackend {
    /// Capture the latest [`HardwareSnapshot`].
    fn snapshot(&self) -> HardwareSnapshot;
}

/// Write/control interface for Quick Settings toggles and sliders.
pub trait HardwareController {
    /// Enable or disable the `Wi-Fi` radio.
    ///
    /// # Errors
    /// Returns [`HardwareError`] if the backend tool fails or is unavailable.
    fn set_wifi(&mut self, enabled: bool) -> Result<(), HardwareError>;

    /// Enable or disable `Bluetooth` controller power.
    ///
    /// # Errors
    /// Returns [`HardwareError`] if the backend tool fails or is unavailable.
    fn set_bluetooth(&mut self, enabled: bool) -> Result<(), HardwareError>;

    /// Enable or disable `WWAN` / cellular data radio.
    ///
    /// # Errors
    /// Returns [`HardwareError`] if the backend tool fails or is unavailable.
    fn set_wwan(&mut self, enabled: bool) -> Result<(), HardwareError>;

    /// Set display backlight brightness percentage (`0..=100`).
    ///
    /// # Errors
    /// Returns [`HardwareError`] if the backlight backend fails or is unavailable.
    fn set_brightness(&mut self, pct: u8) -> Result<(), HardwareError>;

    /// Set default audio sink volume percentage (`0..=100`).
    ///
    /// # Errors
    /// Returns [`HardwareError`] if the audio mixer fails or is unavailable.
    fn set_volume(&mut self, pct: u8) -> Result<(), HardwareError>;

    /// Enable or disable warm-color Night Light mode.
    ///
    /// # Errors
    /// Returns [`HardwareError`] if the display color backend fails or is unavailable.
    fn set_night_light(&mut self, enabled: bool) -> Result<(), HardwareError>;
}

/// Deterministic hardware backend for tests and offline previews.
#[derive(Clone, Debug, Default)]
pub struct MockHardware {
    state: HardwareSnapshot,
}

impl MockHardware {
    /// Create a mock backend seeded with a custom snapshot.
    #[must_use]
    pub const fn new(state: HardwareSnapshot) -> Self {
        Self { state }
    }

    /// Mutate the mock snapshot in place.
    pub fn state_mut(&mut self) -> &mut HardwareSnapshot {
        &mut self.state
    }
}

impl HardwareBackend for MockHardware {
    fn snapshot(&self) -> HardwareSnapshot {
        self.state.clone()
    }
}

impl HardwareController for MockHardware {
    fn set_wifi(&mut self, enabled: bool) -> Result<(), HardwareError> {
        self.state.wifi_connected = enabled;
        self.state.wifi_ssid = if enabled {
            Some("MockNet".into())
        } else {
            None
        };
        Ok(())
    }

    fn set_bluetooth(&mut self, enabled: bool) -> Result<(), HardwareError> {
        self.state.bluetooth_enabled = enabled;
        Ok(())
    }

    fn set_wwan(&mut self, enabled: bool) -> Result<(), HardwareError> {
        self.state.lte_signal_pct = if enabled { Some(80) } else { None };
        Ok(())
    }

    fn set_brightness(&mut self, pct: u8) -> Result<(), HardwareError> {
        self.state.brightness_pct = pct.clamp(5, 100);
        Ok(())
    }

    fn set_volume(&mut self, pct: u8) -> Result<(), HardwareError> {
        self.state.volume_pct = pct.min(100);
        Ok(())
    }

    fn set_night_light(&mut self, enabled: bool) -> Result<(), HardwareError> {
        self.state.night_light = enabled;
        Ok(())
    }
}

/// Errors encountered while parsing a declarative hardware profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileError {
    /// A required field was missing from the `TOML` file.
    MissingField(&'static str),
    /// A field value had an invalid numeric or boolean format.
    InvalidValue { field: &'static str, value: String },
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing required profile field: {field}"),
            Self::InvalidValue { field, value } => {
                write!(f, "invalid value for profile field {field}: {value}")
            }
        }
    }
}

impl std::error::Error for ProfileError {}

/// Declarative per-tablet tuning loaded from `hardware-profiles/*.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct HardwareProfile {
    pub id: String,
    pub vendor: String,
    pub product: String,
    pub connector_prefix: String,
    pub preferred_scale: f32,
    pub refresh_hz: u32,
    pub touch_device: Option<String>,
    pub mode_strategy: String,
    pub usb_vendor_id: Option<String>,
    pub usb_product_id: Option<String>,
    pub auto_rotate_in_tablet_mode: bool,
    pub swap_left_right: bool,
}

impl HardwareProfile {
    /// Parse a flat sectioned `TOML` hardware profile without external crates.
    ///
    /// # Errors
    /// Returns [`ProfileError`] if required keys are missing or invalid.
    pub fn parse_toml(src: &str) -> Result<Self, ProfileError> {
        let mut section = String::new();
        let mut id = None;
        let mut vendor = None;
        let mut product = None;
        let mut connector_prefix = "eDP-".to_owned();
        let mut preferred_scale = 1.0_f32;
        let mut refresh_hz = 60_u32;
        let mut touch_device = None;
        let mut mode_strategy = "manual".to_owned();
        let mut usb_vendor_id = None;
        let mut usb_product_id = None;
        let mut auto_rotate_in_tablet_mode = true;
        let mut swap_left_right = false;

        for raw_line in src.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                line[1..line.len() - 1].trim().clone_into(&mut section);
                continue;
            }
            let Some((key, raw_val)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let val = raw_val
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"');

            match (section.as_str(), key) {
                ("device", "id") => id = Some(val.to_owned()),
                ("device", "vendor") => vendor = Some(val.to_owned()),
                ("device", "product") => product = Some(val.to_owned()),
                ("display", "connector_prefix") => val.clone_into(&mut connector_prefix),
                ("display", "preferred_scale") => {
                    preferred_scale = val.parse().map_err(|_| ProfileError::InvalidValue {
                        field: "display.preferred_scale",
                        value: val.to_owned(),
                    })?;
                }
                ("display", "refresh_hz") => {
                    refresh_hz = val.parse().map_err(|_| ProfileError::InvalidValue {
                        field: "display.refresh_hz",
                        value: val.to_owned(),
                    })?;
                }
                ("touch", "device_name") => touch_device = Some(val.to_owned()),
                ("mode_switch", "strategy") => val.clone_into(&mut mode_strategy),
                ("mode_switch", "usb_vendor_id") if !val.is_empty() => {
                    usb_vendor_id = Some(val.to_ascii_lowercase());
                }
                ("mode_switch", "usb_product_id") if !val.is_empty() => {
                    usb_product_id = Some(val.to_ascii_lowercase());
                }
                ("sensors", "auto_rotate_in_tablet_mode") => {
                    auto_rotate_in_tablet_mode =
                        val.parse().map_err(|_| ProfileError::InvalidValue {
                            field: "sensors.auto_rotate_in_tablet_mode",
                            value: val.to_owned(),
                        })?;
                }
                ("sensors", "swap_left_right") => {
                    swap_left_right = val.parse().map_err(|_| ProfileError::InvalidValue {
                        field: "sensors.swap_left_right",
                        value: val.to_owned(),
                    })?;
                }
                _ => {}
            }
        }

        Ok(Self {
            id: id.ok_or(ProfileError::MissingField("device.id"))?,
            vendor: vendor.ok_or(ProfileError::MissingField("device.vendor"))?,
            product: product.ok_or(ProfileError::MissingField("device.product"))?,
            connector_prefix,
            preferred_scale,
            refresh_hz,
            touch_device,
            mode_strategy,
            usb_vendor_id,
            usb_product_id,
            auto_rotate_in_tablet_mode,
            swap_left_right,
        })
    }

    /// Built-in hardware profiles embedded in `veilshell-hw`.
    #[must_use]
    pub fn built_in_profiles() -> Vec<Self> {
        const SURFACE_GO_3: &str =
            include_str!("../../../hardware-profiles/microsoft-surface-go-3.toml");
        Self::parse_toml(SURFACE_GO_3).into_iter().collect()
    }

    /// Find the first profile matching a `DMI` `sys_vendor` and `product_name` pair.
    #[must_use]
    pub fn match_dmi<'a>(profiles: &'a [Self], vendor: &str, product: &str) -> Option<&'a Self> {
        let vendor = vendor.trim();
        let product = product.trim();
        profiles.iter().find(|p| {
            p.vendor.eq_ignore_ascii_case(vendor) && p.product.eq_ignore_ascii_case(product)
        })
    }

    /// Return the configured detachable `USB` `(vendor_id, product_id)` pair if present.
    #[must_use]
    pub fn usb_keyboard_ids(&self) -> Option<(&str, &str)> {
        match (&self.usb_vendor_id, &self.usb_product_id) {
            (Some(v), Some(p)) => Some((v.as_str(), p.as_str())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_surface_go_3_reference_profile() {
        let raw = include_str!("../../../hardware-profiles/microsoft-surface-go-3.toml");
        let profile = HardwareProfile::parse_toml(raw).expect("valid surface profile");
        assert_eq!(profile.id, "microsoft-surface-go-3");
        assert_eq!(profile.product, "Surface Go 3");
        assert_eq!(profile.connector_prefix, "eDP-");
        assert!((profile.preferred_scale - 1.5).abs() < 1e-4);
        assert_eq!(profile.refresh_hz, 60);
        assert_eq!(
            profile.touch_device.as_deref(),
            Some("ELAN9038:00 04F3:2A1C")
        );
        assert_eq!(profile.mode_strategy, "usb_keyboard_presence");
        assert_eq!(profile.usb_vendor_id.as_deref(), Some("045e"));
        assert_eq!(profile.usb_product_id.as_deref(), Some("09b5"));
        assert!(profile.auto_rotate_in_tablet_mode);
        assert!(!profile.swap_left_right);
    }

    #[test]
    fn matches_built_in_surface_go_3_dmi() {
        let profiles = HardwareProfile::built_in_profiles();
        let matched =
            HardwareProfile::match_dmi(&profiles, "Microsoft Corporation", "Surface Go 3")
                .expect("Surface Go 3 should match");
        assert_eq!(matched.usb_keyboard_ids(), Some(("045e", "09b5")));
        assert!(HardwareProfile::match_dmi(&profiles, "Acme", "Unknown Tablet").is_none());
    }

    #[test]
    fn mock_hardware_controller_mutates_snapshot() {
        let mut mock = MockHardware::default();
        mock.set_wifi(false).expect("wifi off");
        mock.set_bluetooth(true).expect("bt on");
        mock.set_wwan(true).expect("wwan on");
        mock.set_brightness(2).expect("brightness clamped");
        mock.set_volume(91).expect("volume set");
        mock.set_night_light(true).expect("night light on");

        let snap = mock.snapshot();
        assert!(!snap.wifi_connected);
        assert!(snap.wifi_ssid.is_none());
        assert!(snap.bluetooth_enabled);
        assert_eq!(snap.lte_signal_pct, Some(80));
        assert_eq!(snap.brightness_pct, 5);
        assert_eq!(snap.volume_pct, 91);
        assert!(snap.night_light);
    }
}
