//! Hardware telemetry snapshots and declarative device profile parsing.

use std::fmt;

use veilshell_core::Orientation;

/// Point-in-time hardware status consumed by the shell and quick settings shade.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct HardwareSnapshot {
    pub wifi_connected: bool,
    pub wifi_ssid: Option<String>,
    pub bluetooth_enabled: bool,
    pub lte_signal_pct: Option<u8>,
    pub battery_pct: u8,
    pub charging: bool,
    pub brightness_pct: u8,
    pub volume_pct: u8,
    pub muted: bool,
    pub keyboard_attached: bool,
    pub orientation: Orientation,
}

impl Default for HardwareSnapshot {
    fn default() -> Self {
        Self {
            wifi_connected: true,
            wifi_ssid: Some("Offline".into()),
            bluetooth_enabled: false,
            lte_signal_pct: None,
            battery_pct: 85,
            charging: false,
            brightness_pct: 60,
            volume_pct: 50,
            muted: false,
            keyboard_attached: true,
            orientation: Orientation::Normal,
        }
    }
}

/// Read-only hardware telemetry source (`iio-sensor-proxy`, `UPower`, `NetworkManager`, etc.).
pub trait HardwareBackend {
    /// Capture the latest [`HardwareSnapshot`].
    fn snapshot(&self) -> HardwareSnapshot;
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
    pub auto_rotate_in_tablet_mode: bool,
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
        let mut auto_rotate_in_tablet_mode = true;

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
                ("sensors", "auto_rotate_in_tablet_mode") => {
                    auto_rotate_in_tablet_mode =
                        val.parse().map_err(|_| ProfileError::InvalidValue {
                            field: "sensors.auto_rotate_in_tablet_mode",
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
            auto_rotate_in_tablet_mode,
        })
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
        assert!(profile.auto_rotate_in_tablet_mode);
    }
}
