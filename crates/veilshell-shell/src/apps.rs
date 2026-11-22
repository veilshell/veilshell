//! `.desktop` application discovery, icon theme resolution, command formatting, and live clock helpers.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use veilshell_core::{app_badge, parse_desktop_entry, DesktopApp, Rgba};
use veilshell_render::{AppTile, RgbaImage};

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

/// Convert [`DesktopApp`] records into visual [`AppTile`] items ready for `Home`,
/// resolving and caching `PNG` app icons from standard freedesktop icon themes.
#[must_use]
pub fn apps_to_tiles(apps: &[DesktopApp]) -> Vec<AppTile> {
    let mut icon_cache: HashMap<String, Option<Arc<RgbaImage>>> = HashMap::new();
    apps.iter()
        .enumerate()
        .map(|(idx, app)| {
            let (r, g, b) = TILE_PALETTE[idx % TILE_PALETTE.len()];
            let icon = app.icon.as_deref().and_then(|name| {
                icon_cache
                    .entry(name.to_owned())
                    .or_insert_with(|| load_app_icon(name))
                    .clone()
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

/// Resolve a freedesktop `.desktop` `Icon=` name or path to an existing `PNG` file.
#[must_use]
pub fn resolve_icon_path(icon_name: &str) -> Option<PathBuf> {
    let trimmed = icon_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let raw_path = Path::new(trimmed);
    if raw_path.is_absolute() {
        return resolve_absolute_png(raw_path);
    }

    let stem = trimmed
        .strip_suffix(".png")
        .or_else(|| trimmed.strip_suffix(".PNG"))
        .unwrap_or(trimmed);
    let lower = stem.to_ascii_lowercase();
    let mut candidates = Vec::with_capacity(2);
    candidates.push(stem.to_owned());
    if lower != stem {
        candidates.push(lower);
    }

    if let Some(found) = resolve_local_icon(&candidates) {
        return Some(found);
    }
    if let Some(found) = resolve_system_icon(&candidates) {
        return Some(found);
    }
    candidates
        .iter()
        .map(|cand| Path::new("/usr/share/pixmaps").join(format!("{cand}.png")))
        .find(|p| p.is_file())
}

fn resolve_absolute_png(raw_path: &Path) -> Option<PathBuf> {
    if raw_path.is_file()
        && raw_path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    {
        return Some(raw_path.to_path_buf());
    }
    let with_ext = raw_path.with_extension("png");
    with_ext.is_file().then_some(with_ext)
}

fn resolve_local_icon(candidates: &[String]) -> Option<PathBuf> {
    let home_dir = PathBuf::from(std::env::var_os("HOME")?);
    let local_icon_roots = [
        home_dir.join(".local").join("share").join("icons"),
        home_dir
            .join(".local")
            .join("share")
            .join("applications")
            .join("icons"),
    ];
    for cand in candidates {
        for root in &local_icon_roots {
            let direct = root.join(format!("{cand}.png"));
            if direct.is_file() {
                return Some(direct);
            }
        }
    }
    let local_hicolor = home_dir
        .join(".local")
        .join("share")
        .join("icons")
        .join("hicolor");
    for size in ["128x128", "256x256", "96x96", "64x64", "48x48", "512x512"] {
        for cand in candidates {
            let p = local_hicolor
                .join(size)
                .join("apps")
                .join(format!("{cand}.png"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn resolve_system_icon(candidates: &[String]) -> Option<PathBuf> {
    const THEMES: &[&str] = &[
        "hicolor",
        "Yaru",
        "Adwaita",
        "AdwaitaLegacy",
        "Papirus",
        "breeze",
    ];
    const SIZES: &[&str] = &[
        "128x128",
        "128x128@2",
        "128x128@2x",
        "256x256",
        "256x256@2",
        "96x96",
        "64x64",
        "64x64@2",
        "48x48",
        "48x48@2",
        "512x512",
        "192x192",
        "32x32",
    ];
    const CONTEXTS: &[&str] = &[
        "apps",
        "categories",
        "devices",
        "places",
        "actions",
        "mimetypes",
    ];

    for theme in THEMES {
        let theme_root = Path::new("/usr/share/icons").join(theme);
        if !theme_root.is_dir() {
            continue;
        }
        for size in SIZES {
            let size_dir = theme_root.join(size);
            if !size_dir.is_dir() {
                continue;
            }
            for ctx in CONTEXTS {
                let ctx_dir = size_dir.join(ctx);
                if !ctx_dir.is_dir() {
                    continue;
                }
                for cand in candidates {
                    let p = ctx_dir.join(format!("{cand}.png"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

/// Resolve, decode, and downscale a freedesktop `.desktop` `PNG` icon to `<= 96x96`.
#[must_use]
pub fn load_app_icon(icon_name: &str) -> Option<Arc<RgbaImage>> {
    let path = resolve_icon_path(icon_name)?;
    let decoded = RgbaImage::load_png(path).ok()?;
    Some(Arc::new(decoded.resize_fit(96, 96)))
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
        assert!(tiles[1].icon.is_none());
    }

    #[test]
    fn resolves_and_downscales_absolute_png_icons_with_missing_fallback() {
        assert!(resolve_icon_path("definitely-missing-veilshell-icon-xyz").is_none());
        assert!(load_app_icon("definitely-missing-veilshell-icon-xyz").is_none());

        let tmp = std::env::temp_dir().join("veilshell_test_app_icon.png");
        let big = RgbaImage::new(192, 192, vec![120; 192 * 192 * 4]).expect("valid image");
        let frame = veilshell_render::RenderFrame {
            width: big.width,
            height: big.height,
            rgba: big.rgba,
        };
        frame.save_png(&tmp).expect("write test icon");

        let resolved = resolve_icon_path(tmp.to_string_lossy().as_ref());
        assert_eq!(resolved.as_deref(), Some(tmp.as_path()));
        let loaded = load_app_icon(tmp.to_string_lossy().as_ref()).expect("loads PNG icon");
        assert!(loaded.width <= 96 && loaded.height <= 96);

        let _ = std::fs::remove_file(tmp);
    }
}
