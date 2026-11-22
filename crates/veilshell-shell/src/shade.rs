//! Optional host services used by the mobile shade.

use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::Value;
use veilshell_hw::{linux::parse_wpctl_volume, HardwareSnapshot};
use veilshell_render::{SceneContext, ShadeAction, ShadeChoice, ShadeExtras, ShadePage};
use veilshell_wayland::{channel::Sender, DaemonCommand, ShadeReply};

use crate::session::ShellSession;

const PROFILES: [(&str, &str); 3] = [
    ("balanced", "Balanced"),
    ("power-saver", "Power saver"),
    ("performance", "Performance"),
];
const QUIET_MODE: &str = "do-not-disturb";
const SINK: &str = "@DEFAULT_AUDIO_SINK@";
static NEXT_TICKET: AtomicU64 = AtomicU64::new(0);

pub fn apply_hardware(ctx: &mut SceneContext, snapshot: &HardwareSnapshot) {
    ctx.shade.wifi_radio = snapshot.wifi_radio;
    ctx.shade.wifi_name.clone_from(&snapshot.wifi_ssid);
    ctx.shade.bluetooth_available = snapshot.bluetooth_available;
    ctx.shade.mobile_radio = snapshot.mobile_radio;
    ctx.shade.brightness_available = snapshot.brightness_available;
    ctx.shade.volume_available = snapshot.volume_available;
    ctx.shade.night_light_available = snapshot.night_light_available;
}

pub fn refresh_optional(ctx: &mut SceneContext) {
    ctx.shade.date = output("date", &["+%A, %B %e"]).unwrap_or_default();
    ctx.shade.power_profile = power_label();
}

fn output(bin: &str, args: &[&str]) -> Option<String> {
    bounded_output("2s", bin, args)
}

#[must_use]
pub fn bounded_output(limit: &str, bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("timeout")
        .args(["--kill-after=1s", limit, bin])
        .args(args)
        .env("LC_ALL", "C")
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

/// Queue a shade operation; subprocesses never block the Wayland loop.
pub fn request(
    ctx: &mut SceneContext,
    tx: &Sender<DaemonCommand>,
    session: ShellSession,
    action: Option<ShadeAction>,
) {
    if ctx.shade.pending.is_some() {
        return;
    }
    let ticket = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
    let supported = ctx.shade.extras.clone();
    let tx = tx.clone();
    let spawn = thread::Builder::new()
        .name("veilshell-shade".into())
        .spawn(move || {
            let result = match action {
                None => Ok(discover(session)),
                Some(action) => perform(session, &supported, action).ok_or_else(|| {
                    "That action did not complete. Check the service and try again.".to_owned()
                }),
            };
            let _ = tx.send(DaemonCommand::ShadeFinished { ticket, result });
        });
    if spawn.is_ok() {
        ctx.shade.pending = Some(ticket);
    } else {
        ctx.shade.feedback = Some("Could not start the settings operation".into());
    }
}

fn omarchy_session(session: ShellSession) -> bool {
    if session != ShellSession::Overlay || std::env::var_os("OMARCHY_PATH").is_none() {
        return false;
    }
    let Some(display) = std::env::var_os("WAYLAND_DISPLAY").filter(|name| !name.is_empty()) else {
        return false;
    };
    let display = PathBuf::from(display);
    let socket = if display.is_absolute() {
        display
    } else {
        let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
            return false;
        };
        PathBuf::from(runtime).join(display)
    };
    fs::metadata(socket).is_ok_and(|m| m.file_type().is_socket())
}

fn discover(session: ShellSession) -> ShadeReply {
    let mut extras = ShadeExtras {
        wifi: choices(ShadePage::Wifi).is_some(),
        bluetooth: choices(ShadePage::Bluetooth).is_some(),
        audio: choices(ShadePage::Audio).is_some(),
        muted: muted(),
        ..ShadeExtras::default()
    };
    extras.bluetooth_name =
        connected_bluetooth().and_then(|devices| devices.into_iter().next().map(|d| d.label));
    let omarchy = omarchy_session(session);
    let mut quiet = None;
    if omarchy && output("omarchy-shell", &["shell", "ping"]).as_deref() == Some("ok") {
        let plugins = output("omarchy-shell", &["shell", "listPlugins"])
            .and_then(|json| serde_json::from_str::<Vec<Value>>(&json).ok())
            .unwrap_or_default();
        let enabled = |id| {
            plugins
                .iter()
                .any(|p| p["id"].as_str() == Some(id) && p["enabled"] == true)
        };
        extras.host_wifi = enabled("omarchy.network");
        extras.host_bluetooth = enabled("omarchy.bluetooth");
        extras.host_audio = enabled("omarchy.audio");
        quiet = omarchy_quiet("isDnd");
        extras.notifications = quiet.is_some();
        extras.screenshot = ["omarchy-capture-screenshot", "omasnap"]
            .into_iter()
            .all(executable);
    }
    if quiet.is_none() && session == ShellSession::Overlay && quiet_configured() {
        quiet = quiet_status();
    }
    extras.media = media_status(omarchy);
    ShadeReply::Discovered(extras, quiet)
}

fn executable(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            fs::metadata(dir.join(name))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

fn omarchy_quiet(method: &str) -> Option<bool> {
    match output("omarchy-shell", &["notifications", method])?.as_str() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

fn muted() -> Option<bool> {
    output("wpctl", &["get-volume", SINK])
        .and_then(|s| parse_wpctl_volume(&s))
        .map(|(_, muted)| muted)
}

fn media_status(omarchy: bool) -> Option<String> {
    if omarchy {
        if let Some(status) = output("omarchy-shell", &["media", "status"])
            .and_then(|json| serde_json::from_str::<Value>(&json).ok())
            .filter(|v| v["hasPlayer"] == true)
        {
            return Some(safe_label(
                status["title"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or(status["identity"].as_str())
                    .unwrap_or("Media player"),
            ));
        }
    }
    output("playerctl", &["metadata", "--format", "{{title}}"])
        .filter(|s| !s.is_empty())
        .map(|s| safe_label(&s))
}

fn perform(
    session: ShellSession,
    supported: &ShadeExtras,
    action: ShadeAction,
) -> Option<ShadeReply> {
    let omarchy = omarchy_session(session);
    match action {
        ShadeAction::Show(page) => {
            let supported_native = match page {
                ShadePage::Wifi => supported.wifi,
                ShadePage::Bluetooth => supported.bluetooth,
                ShadePage::Audio => supported.audio,
                _ => false,
            };
            Some(ShadeReply::Choices(if supported_native {
                choices(page)?
            } else {
                Vec::new()
            }))
        }
        ShadeAction::Select(page, id) => select(page, &id),
        ShadeAction::Mute => {
            output(
                "wpctl",
                &["set-mute", SINK, if muted()? { "0" } else { "1" }],
            )?;
            muted().map(ShadeReply::Muted)
        }
        ShadeAction::Quiet if omarchy && supported.notifications => {
            omarchy_quiet("toggleDnd").map(ShadeReply::Quiet)
        }
        ShadeAction::Host(page) if omarchy && supported.host_panel(page) => {
            let id = match page {
                ShadePage::Wifi => "omarchy.network",
                ShadePage::Bluetooth => "omarchy.bluetooth",
                ShadePage::Audio => "omarchy.audio",
                _ => return None,
            };
            output("omarchy-shell", &[id, "open"])?;
            Some(ShadeReply::Dismiss)
        }
        ShadeAction::Notifications if omarchy && supported.notifications => {
            (output("omarchy-shell", &["notifications", "showHistory"])?.as_str() == "ok")
                .then_some(ShadeReply::Dismiss)
        }
        ShadeAction::Screenshot if omarchy && supported.screenshot => {
            // Let the compositor drop the shade before capturing the screen.
            thread::sleep(Duration::from_millis(200));
            bounded_output("10s", "omarchy-capture-screenshot", &["fullscreen", "save"])?;
            Some(ShadeReply::Dismiss)
        }
        ShadeAction::Previous | ShadeAction::PlayPause | ShadeAction::Next => {
            let (host, generic) = match action {
                ShadeAction::Previous => ("previous", "previous"),
                ShadeAction::Next => ("next", "next"),
                _ => ("playPause", "play-pause"),
            };
            let succeeded = if omarchy
                && output("omarchy-shell", &["media", "status"])
                    .and_then(|json| serde_json::from_str::<Value>(&json).ok())
                    .is_some_and(|v| v["hasPlayer"] == true)
            {
                output("omarchy-shell", &["media", host]).as_deref() == Some("ok")
            } else {
                output("playerctl", &[generic]).is_some()
            };
            succeeded.then(|| ShadeReply::Media(media_status(omarchy)))
        }
        _ => None,
    }
}

fn safe_label(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(120).collect()
}

fn uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn mac(id: &str) -> bool {
    id.len() == 17
        && id.bytes().enumerate().all(|(i, c)| {
            if i % 3 == 2 {
                c == b':'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn nmcli_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut escaped = false;
    for ch in line.chars() {
        if !escaped && ch == '\\' {
            escaped = true;
        } else if !escaped && ch == ':' {
            fields.push(String::new());
        } else {
            fields.last_mut().expect("nonempty").push(ch);
            escaped = false;
        }
    }
    fields
}

fn wifi_choices(text: &str) -> Vec<ShadeChoice> {
    text.lines()
        .filter_map(|line| {
            let fields = nmcli_fields(line);
            let [id, kind, label, device] = fields.as_slice() else {
                return None;
            };
            (uuid(id) && (kind == "802-11-wireless" || kind == "wifi")).then(|| ShadeChoice {
                id: id.clone(),
                label: safe_label(label),
                active: !device.is_empty() && device != "--",
            })
        })
        .take(128)
        .collect()
}

fn bluetooth_devices(text: &str) -> Vec<ShadeChoice> {
    text.lines()
        .filter_map(|line| {
            let (id, name) = line.strip_prefix("Device ")?.split_once(' ')?;
            mac(id).then(|| ShadeChoice {
                id: id.into(),
                label: safe_label(name),
                active: false,
            })
        })
        .take(128)
        .collect()
}

fn connected_bluetooth() -> Option<Vec<ShadeChoice>> {
    output("bluetoothctl", &["devices", "Connected"]).map(|s| bluetooth_devices(&s))
}

fn choices(page: ShadePage) -> Option<Vec<ShadeChoice>> {
    let mut items = match page {
        ShadePage::Wifi => wifi_choices(&output(
            "nmcli",
            &[
                "-t",
                "-e",
                "yes",
                "-f",
                "UUID,TYPE,NAME,DEVICE",
                "connection",
                "show",
            ],
        )?),
        ShadePage::Bluetooth => {
            let mut paired = bluetooth_devices(&output("bluetoothctl", &["devices", "Paired"])?);
            let connected = connected_bluetooth().unwrap_or_default();
            for device in &mut paired {
                device.active = connected.iter().any(|d| d.id == device.id);
            }
            paired
        }
        ShadePage::Audio => {
            let active = output("pactl", &["get-default-sink"])?;
            let json = output("pactl", &["--format=json", "list", "sinks"])?;
            serde_json::from_str::<Vec<Value>>(&json)
                .ok()?
                .into_iter()
                .filter_map(|v| {
                    let name = v["name"].as_str().filter(|name| !name.starts_with('-'))?;
                    Some(ShadeChoice {
                        id: name.to_owned(),
                        label: safe_label(v["description"].as_str().unwrap_or(name)),
                        active: name == active,
                    })
                })
                .take(128)
                .collect()
        }
        _ => return None,
    };
    items.sort_by_key(|item| !item.active);
    Some(items)
}

fn select(page: ShadePage, id: &str) -> Option<ShadeReply> {
    let current = choices(page)?;
    if !current
        .iter()
        .any(|choice| choice.id == id && !choice.active)
    {
        return None;
    }
    match page {
        ShadePage::Wifi if uuid(id) => bounded_output(
            "16s",
            "nmcli",
            &["--wait", "14", "connection", "up", "uuid", id],
        )?,
        ShadePage::Bluetooth if mac(id) => {
            bounded_output("16s", "bluetoothctl", &["--timeout", "14", "connect", id])?
        }
        ShadePage::Audio => output("pactl", &["set-default-sink", id])?,
        _ => return None,
    };
    let items = choices(page)?;
    items
        .iter()
        .any(|choice| choice.id == id && choice.active)
        .then(|| {
            if page == ShadePage::Audio {
                ShadeReply::AudioOutput(items, muted())
            } else {
                ShadeReply::Choices(items)
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_wifi_profiles_use_uuids_and_unescape_display_labels() {
        let choices = wifi_choices(concat!(
            "a3bfd632-98e3-44ff-9216-67c12385789a:802-11-wireless:Home\\: upstairs\\\\2:wlan0\n",
            "bc3d387a-a5a4-450e-bd7f-fdcaf7d2fbb1:802-11-wireless:Guest:\n",
            "not-an-id:802-11-wireless:Invalid:wlan0\n",
            "bc3d387a-a5a4-450e-bd7f-fdcaf7d2fbb1:ethernet:Wired:eth0\n",
        ));
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].label, r"Home: upstairs\2");
        assert!(choices[0].active);
        assert!(!choices[1].active);
    }

    #[test]
    fn bluetooth_list_ignores_service_noise_and_uses_the_device_address() {
        let devices = bluetooth_devices("Waiting to connect to bluetoothd...\nDevice AA:BB:CC:DD:EE:FF Test headphones\nDevice bad invalid");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "AA:BB:CC:DD:EE:FF");
        assert_eq!(devices[0].label, "Test headphones");
    }
}
