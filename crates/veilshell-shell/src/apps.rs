//! `.desktop` application discovery, icon theme resolution, command formatting, and live clock helpers.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use veilshell_compositor::{WindowId, WindowInfo};
use veilshell_core::{app_badge, parse_desktop_entry, DesktopApp, Rgba};
use veilshell_render::{AppTile, RgbaImage};

mod icons;
pub(crate) use icons::decode_bitmap;
mod xdg;

use icons::IconLookup;
use xdg::DataDirs;

/// Resolve a running Home app in compositor focus order. Do not guess from
/// its command, icon, or title: multiple web apps may share a browser.
#[must_use]
pub fn running_app_window<'a>(app: &DesktopApp, windows: &'a [WindowInfo]) -> Option<&'a WindowId> {
    let desktop_id = app.id.strip_suffix(".desktop").unwrap_or(&app.id);
    app.startup_wm_class
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(desktop_id))
        .filter(|id| !id.is_empty())
        .find_map(|id| {
            windows
                .iter()
                .find(|window| window.app_id.eq_ignore_ascii_case(id))
                .map(|window| &window.id)
        })
}

const PINNED_ORDER: &[&str] = &[
    "foot",
    "ghostty",
    "kitty",
    "alacritty",
    "chromium",
    "google-chrome",
    "firefox",
    "org.gnome.nautilus",
    "thunar",
    "obsidian",
    "code",
    "btop",
];

const TILE_PALETTE: [(u8, u8, u8); 8] = [
    (122, 162, 247),
    (166, 161, 231),
    (168, 201, 99),
    (231, 174, 109),
    (173, 147, 234),
    (245, 123, 156),
    (63, 177, 186),
    (245, 154, 118),
];

/// Discover user and system `.desktop` applications in priority order.
#[must_use]
pub fn discover_desktop_apps() -> Vec<DesktopApp> {
    discover_in(&DataDirs::from_env())
}

fn discover_in(data: &DataDirs) -> Vec<DesktopApp> {
    let mut seen_ids = HashSet::new();
    let mut apps = Vec::new();

    for root in data.paths() {
        let applications = root.join("applications");
        let mut dirs = vec![applications.clone()];
        while let Some(dir) = dirs.pop() {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    // Follow file symlinks (e.g. Flatpak exports), not directory symlinks.
                    dirs.push(path);
                    continue;
                }
                if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(relative) = path.strip_prefix(&applications).ok().and_then(Path::to_str)
                else {
                    continue;
                };
                let id = relative.replace('/', "-");
                if seen_ids.contains(&id) {
                    continue;
                }
                let Some(raw) = xdg::read_text(&path) else {
                    continue;
                };
                if let Some(mut app) = parse_desktop_entry(&raw, &id) {
                    if let Some(icon) = app.icon.as_mut().filter(|icon| icon.contains('\\')) {
                        *icon = unescape_icon(icon);
                    }
                    apps.push(app);
                }
                // An entry marked Hidden/NoDisplay still masks a system entry with this ID.
                seen_ids.insert(id);
            }
        }
    }

    apps.sort_by_cached_key(|app| {
        (
            pinned_rank(&app.id),
            app.name.to_lowercase(),
            app.id.clone(),
        )
    });
    apps
}

fn unescape_icon(icon: &str) -> String {
    let mut chars = icon.chars();
    let mut value = String::with_capacity(icon.len());
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            value.push(ch);
            continue;
        }
        let ch = match chars.next() {
            Some('s') => ' ',
            Some('n') => '\n',
            Some('t') => '\t',
            Some('r') => '\r',
            Some('\\') | None => '\\',
            Some(other) => {
                value.push('\\');
                other
            }
        };
        value.push(ch);
    }
    value
}

fn pinned_rank(desktop_id: &str) -> usize {
    let stem = desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id);
    PINNED_ORDER
        .iter()
        .position(|p| stem.eq_ignore_ascii_case(p))
        .unwrap_or(usize::MAX)
}

/// Convert [`DesktopApp`] records into visual [`AppTile`] items ready for `Home`,
/// resolving app icons for an 85-logical-pixel tile at up to 2× display scale.
#[must_use]
pub fn apps_to_tiles(apps: &[DesktopApp]) -> Vec<AppTile> {
    let mut lookup = None;
    make_tiles(apps, |name| {
        lookup.get_or_insert_with(IconLookup::from_env).load(name)
    })
}

fn make_tiles(
    apps: &[DesktopApp],
    mut load_icon: impl FnMut(&str) -> Option<Arc<RgbaImage>>,
) -> Vec<AppTile> {
    // Cache shared image identities (and misses) only during this refresh.
    const MAX_CACHED_ICONS: usize = 256;
    let mut icon_cache: HashMap<_, Option<Arc<RgbaImage>>> = HashMap::new();
    apps.iter()
        .enumerate()
        .map(|(idx, app)| {
            let (r, g, b) = TILE_PALETTE[idx % TILE_PALETTE.len()];
            let icon = app.icon.as_deref().and_then(|name| {
                if let Some(image) = icon_cache.get(name) {
                    return image.clone();
                }
                let image = load_icon(name);
                if icon_cache.len() < MAX_CACHED_ICONS {
                    icon_cache.insert(name, image.clone());
                }
                image
            });
            AppTile {
                id: app.id.clone(),
                badge: app_badge(&app.name),
                label: app.name.clone(),
                exec: format_exec_command(app),
                tint: Rgba::from_rgb_u8(r, g, b),
                icon,
            }
        })
        .collect()
}

/// Resolve a freedesktop `Icon=` name or absolute path to a `PNG` or `SVG` candidate.
#[must_use]
pub fn resolve_icon_path(icon_name: &str) -> Option<PathBuf> {
    IconLookup::from_env().resolve(icon_name)
}

/// Load the best usable `PNG` or `SVG` icon as straight `RGBA8`, at most 170×170 pixels.
/// JPEG data in a `.png` file is also supported for compatibility.
#[must_use]
pub fn load_app_icon(icon_name: &str) -> Option<Arc<RgbaImage>> {
    IconLookup::from_env().load(icon_name)
}

/// Prepare a shell launch command, wrapping terminal apps in a Wayland terminal emulator.
#[must_use]
pub fn format_exec_command(app: &DesktopApp) -> String {
    if !app.terminal {
        return app.exec.clone();
    }
    if command_exists("xdg-terminal-exec") {
        format!("xdg-terminal-exec {}", app.exec)
    } else if command_exists("foot") {
        format!("foot -e {}", app.exec)
    } else if command_exists("ghostty") {
        format!("ghostty -e {}", app.exec)
    } else {
        app.exec.clone()
    }
}

fn command_exists(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(bin).is_file())
}

/// Query current local `HH:MM` wall-clock string for the status bar.
#[must_use]
pub fn current_clock_hhmm() -> String {
    if let Ok(output) = Command::new("date").arg("+%H:%M").output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if s.len() == 5 {
                return s;
            }
        }
    }
    "12:00".to_owned()
}

/// Read time and long local date in one sample, so they agree across midnight.
#[must_use]
pub fn current_tablet_clock() -> (String, String) {
    Command::new("date")
        .arg("+%H:%M%n%A %-d %B")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| parse_tablet_clock(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_else(|| ("12:00".into(), String::new()))
}

fn parse_tablet_clock(output: &str) -> Option<(String, String)> {
    let (time, date) = output.trim().split_once('\n')?;
    let valid_time = time.len() == 5
        && time.bytes().enumerate().all(|(i, byte)| {
            if i == 2 {
                byte == b':'
            } else {
                byte.is_ascii_digit()
            }
        });
    (valid_time && !date.is_empty()).then(|| (time.to_owned(), date.to_owned()))
}

#[cfg(test)]
mod tests;
