//! `.desktop` application discovery, command formatting, and live clock helpers.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use veilshell_core::{app_badge, parse_desktop_entry, DesktopApp, Rgba};
use veilshell_render::AppTile;

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
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("applications"),
        );
    }
    dirs.push(PathBuf::from("/usr/local/share/applications"));
    dirs.push(PathBuf::from("/usr/share/applications"));

    let mut seen_ids = HashSet::new();
    let mut apps = Vec::new();

    for dir in dirs {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !seen_ids.insert(file_name.to_owned()) {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            if let Some(app) = parse_desktop_entry(&raw, file_name) {
                apps.push(app);
            }
        }
    }

    apps.sort_by(|a, b| {
        let pa = pinned_rank(&a.id);
        let pb = pinned_rank(&b.id);
        pa.cmp(&pb)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    apps
}

fn pinned_rank(desktop_id: &str) -> usize {
    let stem = desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id);
    PINNED_ORDER
        .iter()
        .position(|p| stem.eq_ignore_ascii_case(p))
        .unwrap_or(usize::MAX)
}

/// Convert [`DesktopApp`] records into visual [`AppTile`] items ready for `Home`.
#[must_use]
pub fn apps_to_tiles(apps: &[DesktopApp]) -> Vec<AppTile> {
    apps.iter()
        .enumerate()
        .map(|(idx, app)| {
            let (r, g, b) = TILE_PALETTE[idx % TILE_PALETTE.len()];
            AppTile {
                badge: app_badge(&app.name),
                label: app.name.clone(),
                exec: format_exec_command(app),
                tint: Rgba::from_rgb_u8(r, g, b),
            }
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_desktop_apps_to_tiles_with_badges_and_exec() {
        let apps = vec![
            DesktopApp {
                id: "foot.desktop".into(),
                name: "Foot".into(),
                exec: "foot".into(),
                icon: Some("foot".into()),
                terminal: false,
                categories: vec![],
            },
            DesktopApp {
                id: "btop.desktop".into(),
                name: "btop++".into(),
                exec: "btop".into(),
                icon: None,
                terminal: true,
                categories: vec![],
            },
        ];
        let tiles = apps_to_tiles(&apps);
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].badge, "FOOT");
        assert_eq!(tiles[0].exec, "foot");
        assert!(tiles[1].exec.contains("btop"));
    }
}
