//! Persist standalone-only preferences without sourcing shell or Lua code.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
            _ => WallpaperPreset::Midnight,
        },
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
    fn defaults_and_all_four_preferences_round_trip() {
        let root = env::temp_dir().join(format!("veilshell-settings-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let start = load(&root);
        assert_eq!(start.startup, "Tablet");
        assert!(start.panel);
        assert_eq!(start.bindings, "Preview");
        assert_eq!(start.wallpaper, WallpaperPreset::Midnight);
        for action in [
            SettingsAction::Startup,
            SettingsAction::Panel,
            SettingsAction::Wallpaper,
            SettingsAction::Bindings,
        ] {
            change(&root, action, &load(&root)).unwrap();
        }
        let saved = load(&root);
        assert_eq!(saved.startup, "Desk");
        assert!(!saved.panel);
        assert_eq!(saved.wallpaper, WallpaperPreset::Forest);
        assert_eq!(saved.bindings, "Omarchy-style");
        for name in [
            "startup.mode",
            "desk-panel",
            "wallpaper.preset",
            "bindings.profile",
        ] {
            fs::remove_file(root.join(name)).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }
}
