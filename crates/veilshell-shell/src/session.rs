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

    /// Only the overlay follows the host desktop's palette, read-only.
    #[must_use]
    pub fn palette(self) -> ThemePalette {
        match self {
            Self::Overlay => load_best_available(),
            Self::Standalone => ThemePalette::default_dark(),
        }
    }

    #[must_use]
    pub fn desktop_theme_path(self) -> Option<PathBuf> {
        match self {
            Self::Overlay => OmarchyAetherProvider::default_path(),
            Self::Standalone => None,
        }
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

    #[test]
    fn standalone_does_not_load_the_omarchy_palette() {
        assert!(ShellSession::Standalone.desktop_theme_path().is_none());
        assert_eq!(
            ShellSession::Standalone.palette().name,
            ThemePalette::default_dark().name
        );
    }
}
