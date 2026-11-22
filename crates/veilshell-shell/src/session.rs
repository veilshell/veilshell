//! Distinguish an overlay from the compositor launched by the standalone adapter.

use std::{env, fs, path::PathBuf};

use veilshell_core::ThemePalette;
use veilshell_theme::{load_best_available, OmarchyAetherProvider};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShellSession {
    #[default]
    Overlay,
    Standalone,
}

impl ShellSession {
    /// Only the display and Hyprland instance published by `veilshell-session`
    /// may be managed. An inherited session directory alone is not enough.
    #[must_use]
    pub fn current() -> Self {
        if let (Some(dir), Ok(display), Ok(instance)) = (
            env::var_os("VEILSHELL_SESSION_DIR"),
            env::var("WAYLAND_DISPLAY"),
            env::var("HYPRLAND_INSTANCE_SIGNATURE"),
        ) {
            let dir = PathBuf::from(dir);
            if dir.is_absolute()
                && fs::read_to_string(dir.join("display"))
                    .is_ok_and(|ready| matches_display(&ready, &display, &instance))
            {
                return Self::Standalone;
            }
        }
        Self::Overlay
    }

    #[must_use]
    pub const fn owns_compositor(self) -> bool {
        matches!(self, Self::Standalone)
    }

    /// Follow the host desktop's palette read-only, by default in the overlay and
    /// only when selected in standalone Settings.
    #[must_use]
    pub fn palette(self) -> ThemePalette {
        if self.follows_omarchy_colors() {
            load_best_available()
        } else {
            ThemePalette::default_dark()
        }
    }

    fn follows_omarchy_colors(self) -> bool {
        self == Self::Overlay
            || crate::settings::directory()
                .is_some_and(|dir| crate::settings::follows_omarchy_colors(&dir))
    }

    #[must_use]
    pub fn desktop_theme_path(self) -> Option<PathBuf> {
        self.follows_omarchy_colors()
            .then(OmarchyAetherProvider::default_path)
            .flatten()
    }
}

fn matches_display(ready: &str, display: &str, instance: &str) -> bool {
    !display.is_empty() && !instance.is_empty() && ready.lines().eq([display, instance])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_requires_the_published_display_and_instance() {
        let ready = "wayland-3\nprivate-instance\n";
        assert!(matches_display(ready, "wayland-3", "private-instance"));
        assert!(!matches_display(ready, "wayland-1", "private-instance"));
        assert!(!matches_display(ready, "wayland-3", "omarchy-instance"));
        assert!(!matches_display("wayland-3\n\n", "wayland-3", ""));
        assert!(!matches_display(
            "wayland-3\nprivate-instance\nextra\n",
            "wayland-3",
            "private-instance"
        ));
    }
}
