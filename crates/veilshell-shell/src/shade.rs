//! Optional host services used by the mobile shade.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use veilshell_hw::HardwareSnapshot;
use veilshell_render::SceneContext;

const PROFILES: [(&str, &str); 3] = [
    ("balanced", "Balanced"),
    ("power-saver", "Power saver"),
    ("performance", "Performance"),
];
const QUIET_MODE: &str = "do-not-disturb";

pub fn apply_hardware(ctx: &mut SceneContext, snapshot: &HardwareSnapshot) {
    ctx.shade.wifi_radio = snapshot.wifi_radio;
    ctx.shade.wifi_name.clone_from(&snapshot.wifi_ssid);
    ctx.shade.bluetooth_available = snapshot.bluetooth_available;
    ctx.shade.mobile_radio = snapshot.mobile_radio;
    ctx.shade.brightness_available = snapshot.brightness_available;
    ctx.shade.volume_available = snapshot.volume_available;
}

pub fn refresh_optional(ctx: &mut SceneContext) {
    ctx.shade.date = output("date", &["+%A, %B %e"]).unwrap_or_default();
    ctx.shade.power_profile = power_label();
    ctx.shade.quiet = quiet_configured().then(quiet_status).flatten();
}

fn output(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn power_label() -> Option<String> {
    let current = output("powerprofilesctl", &["get"])?;
    PROFILES
        .iter()
        .find(|(id, _)| *id == current)
        .map(|(_, label)| (*label).to_owned())
}

#[must_use]
pub fn cycle_profile() -> Option<String> {
    let current = output("powerprofilesctl", &["get"])?;
    let profiles = output("powerprofilesctl", &["list"])?;
    let available = PROFILES
        .iter()
        .filter(|(id, _)| {
            profiles
                .lines()
                .any(|line| line.trim().trim_start_matches('*').trim() == format!("{id}:"))
        })
        .collect::<Vec<_>>();
    let index = available.iter().position(|(id, _)| *id == current)?;
    let (next, label) = available[(index + 1) % available.len()];
    output("powerprofilesctl", &["set", next]).map(|_| (*label).to_owned())
}

fn quiet_configured() -> bool {
    let config = std::env::var_os("XDG_CONFIG_HOME").map_or_else(
        || std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")),
        |dir| Some(PathBuf::from(dir)),
    );
    config
        .and_then(|dir| fs::read_to_string(dir.join("mako/config")).ok())
        .is_some_and(|text| {
            text.lines()
                .any(|line| line.trim().starts_with("[mode=do-not-disturb"))
        })
}

fn quiet_status() -> Option<bool> {
    let modes = output("makoctl", &["mode"])?;
    Some(modes.lines().any(|mode| mode.trim() == QUIET_MODE))
}

#[must_use]
pub fn toggle_quiet() -> Option<bool> {
    output("makoctl", &["mode", "-t", QUIET_MODE])?;
    quiet_status()
}
