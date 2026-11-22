use veilshell_core::{Rgba, ThemePalette};

use crate::{DrawList, HitRect, HitTarget, RoundedRect, SceneOutput, TextAlign};

/// Standalone preferences exposed by the shell. File I/O lives in the session adapter.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsView {
    pub startup: &'static str,
    pub panel: bool,
    pub wallpaper: WallpaperPreset,
    pub bindings: &'static str,
    pub feedback: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WallpaperPreset {
    #[default]
    Midnight,
    Forest,
    Dusk,
}

impl WallpaperPreset {
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Midnight => Self::Forest,
            Self::Forest => Self::Dusk,
            Self::Dusk => Self::Midnight,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Midnight => "Midnight",
            Self::Forest => "Forest",
            Self::Dusk => "Dusk",
        }
    }

    pub(crate) const fn colors(self) -> (Rgba, Rgba) {
        match self {
            Self::Midnight => (Rgba::from_rgb_u8(30, 40, 68), Rgba::from_rgb_u8(7, 9, 16)),
            Self::Forest => (Rgba::from_rgb_u8(33, 72, 66), Rgba::from_rgb_u8(10, 18, 23)),
            Self::Dusk => (Rgba::from_rgb_u8(96, 49, 71), Rgba::from_rgb_u8(26, 19, 43)),
        }
    }
}

#[must_use]
pub fn build_wallpaper(preset: WallpaperPreset) -> DrawList {
    let (top, bottom) = preset.colors();
    let mut list = DrawList::default();
    list.push_backdrop(top, bottom);
    list
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsAction {
    Startup,
    Panel,
    Wallpaper,
    Bindings,
    Back,
}

pub(crate) fn build_settings(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    settings: Option<&SettingsView>,
) {
    out.draw_list
        .push_backdrop(palette.background, palette.background);
    let w = (width - 24.0).clamp(0.0, 620.0);
    let x = (width - w) * 0.5;
    out.draw_list.push_text(
        x,
        18.0,
        "VeilShell Settings",
        2,
        palette.foreground,
        TextAlign::Left,
    );
    let Some(settings) = settings else { return };
    let rows = [
        ("Startup mode", settings.startup, SettingsAction::Startup),
        (
            "Desk panel",
            if settings.panel { "Visible" } else { "Hidden" },
            SettingsAction::Panel,
        ),
        (
            "Wallpaper",
            settings.wallpaper.label(),
            SettingsAction::Wallpaper,
        ),
        (
            "Shortcut profile",
            settings.bindings,
            SettingsAction::Bindings,
        ),
        ("Back to Quick Settings", "", SettingsAction::Back),
    ];
    let step = ((height - 66.0) / 5.0).clamp(38.0, 67.0);
    for (i, (title, value, action)) in rows.into_iter().enumerate() {
        let rect = HitRect::new(x, 48.0 + i as f32 * step, w, step - 4.0);
        if rect.y + rect.h > height {
            break;
        }
        out.draw_list.push_rect(RoundedRect::filled(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            11.0,
            palette.surface,
        ));
        let y = rect.y + (rect.h - 12.0) * 0.5;
        out.draw_list.push_text(
            x + 12.0,
            y,
            crate::font::fit_text(title, 1, w * 0.54),
            1,
            palette.foreground,
            TextAlign::Left,
        );
        out.draw_list.push_text(
            x + w - 12.0,
            y,
            crate::font::fit_text(value, 1, w * 0.38),
            1,
            palette.accent,
            TextAlign::Right,
        );
        out.push_hit(rect, HitTarget::Settings(action));
    }
    if let Some(feedback) = &settings.feedback {
        out.draw_list.push_text(
            x,
            height - 16.0,
            crate::font::fit_text(feedback, 1, w),
            1,
            palette.foreground,
            TextAlign::Left,
        );
    }
}
