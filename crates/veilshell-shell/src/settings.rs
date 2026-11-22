//! Persist standalone-only preferences without sourcing shell or Lua code.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use veilshell_render::{SettingsAction, SettingsView, WallpaperPreset};

pub fn directory() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|p| p.join(".config"))
        })
        .map(|path| path.join("veilshell"))
}

#[must_use]
pub fn load(dir: &Path) -> SettingsView {
    let read = |name| fs::read_to_string(dir.join(name)).ok();
    SettingsView {
        startup: match read("startup.mode")
            .as_deref()
            .and_then(|s| s.lines().next())
        {
            Some("desk") => "Desk",
            Some("auto") => "Automatic",
            _ => "Tablet",
        },
        panel: read("desk-panel").as_deref().and_then(|s| s.lines().next()) != Some("hidden"),
        wallpaper: match read("wallpaper.preset")
            .as_deref()
            .and_then(|s| s.lines().next())
        {
            Some("Forest") => WallpaperPreset::Forest,
            Some("Dusk") => WallpaperPreset::Dusk,
            Some("Omarchy wallpaper") => WallpaperPreset::Omarchy,
            _ => WallpaperPreset::Midnight,
        },
        follow_omarchy_colors: follows_omarchy_colors(dir),
        bindings: if read("bindings.profile")
            .as_deref()
            .and_then(|s| s.lines().next())
            == Some("omarchy")
        {
            "Omarchy-style"
        } else {
            "Preview"
        },
        feedback: None,
    }
}

pub(crate) fn follows_omarchy_colors(dir: &Path) -> bool {
    fs::read_to_string(dir.join("theme.source"))
        .is_ok_and(|value| value.lines().next() == Some("omarchy"))
}

/// Save the next value. Return the saved view only after the write succeeds.
///
/// # Errors
/// Returns filesystem errors without reporting the new preference as saved.
pub fn change(dir: &Path, action: SettingsAction, view: &SettingsView) -> io::Result<SettingsView> {
    let (name, value, feedback) = match action {
        SettingsAction::Startup => (
            "startup.mode",
            match view.startup {
                "Tablet" => "desk",
                "Desk" => "auto",
                _ => "tablet",
            },
            "Saved for next login.",
        ),
        SettingsAction::Panel => (
            "desk-panel",
            if view.panel { "hidden" } else { "visible" },
            "Desk panel preference saved.",
        ),
        SettingsAction::Wallpaper => (
            "wallpaper.preset",
            view.wallpaper.next().label(),
            "Wallpaper saved.",
        ),
        SettingsAction::Theme => (
            "theme.source",
            if view.follow_omarchy_colors {
                "builtin"
            } else {
                "omarchy"
            },
            "Theme saved.",
        ),
        SettingsAction::Bindings => (
            "bindings.profile",
            if view.bindings == "Preview" {
                "omarchy"
            } else {
                "preview"
            },
            "Saved for next login.",
        ),
        SettingsAction::Back => return Ok(view.clone()),
    };
    write_preference(dir, name, value)?;
    let mut updated = load(dir);
    updated.feedback = Some(feedback.into());
    Ok(updated)
}

#[must_use]
pub fn import_wallpaper() -> Option<Arc<veilshell_render::RgbaImage>> {
    import_wallpaper_from(&veilshell_theme::omarchy_current_dir()?)
}

fn import_wallpaper_from(current: &Path) -> Option<Arc<veilshell_render::RgbaImage>> {
    let image = crate::apps::decode_bitmap(&current.join("background"), 4096)?;
    Some(Arc::new(if image.width <= 1920 && image.height <= 1920 {
        image
    } else {
        image.resize_fit(1920, 1920)
    }))
}

fn write_preference(dir: &Path, name: &str, value: &str) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(dir)?;
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".settings-{}-{nonce}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        writeln!(file, "{value}")?;
        file.sync_all()?;
        fs::rename(&temp, dir.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_preferences_round_trip() {
        let root = env::temp_dir().join(format!("veilshell-settings-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let start = load(&root);
        assert_eq!(start.startup, "Tablet");
        assert!(start.panel);
        assert_eq!(start.bindings, "Preview");
        assert_eq!(start.wallpaper, WallpaperPreset::Midnight);
        assert!(!start.follow_omarchy_colors);
        for action in [
            SettingsAction::Startup,
            SettingsAction::Panel,
            SettingsAction::Wallpaper,
            SettingsAction::Bindings,
            SettingsAction::Theme,
        ] {
            change(&root, action, &load(&root)).unwrap();
        }
        let saved = load(&root);
        assert_eq!(saved.startup, "Desk");
        assert!(!saved.panel);
        assert_eq!(saved.wallpaper, WallpaperPreset::Forest);
        assert_eq!(saved.bindings, "Omarchy-style");
        assert!(saved.follow_omarchy_colors);
        for name in [
            "startup.mode",
            "desk-panel",
            "wallpaper.preset",
            "bindings.profile",
            "theme.source",
        ] {
            fs::remove_file(root.join(name)).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn imports_the_wallpaper_symlink_and_falls_back_if_missing_or_broken() {
        use std::os::unix::fs::symlink;
        let root = env::temp_dir().join(format!("veilshell-wallpaper-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let target = root.join("picture.png");
        let link = root.join("background");
        assert!(import_wallpaper_from(&root).is_none());
        symlink("picture.png", &link).unwrap();
        assert!(import_wallpaper_from(&root).is_none());
        fs::write(&target, "not an image").unwrap();
        assert!(import_wallpaper_from(&root).is_none());
        {
            let mut encoder = png::Encoder::new(fs::File::create(&target).unwrap(), 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[19, 97, 41, 255])
                .unwrap();
        }
        let image = import_wallpaper_from(&root).unwrap();
        assert_eq!(image.rgba, [19, 97, 41, 255]);
        assert!(link.is_symlink());
        fs::remove_file(link).unwrap();
        fs::remove_file(target).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
