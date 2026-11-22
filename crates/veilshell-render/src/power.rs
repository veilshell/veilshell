//! Standalone session power-menu layout and confirmation state.

use veilshell_core::ThemePalette;

use crate::{HitRect, HitTarget, RoundedRect, SceneOutput, TextAlign};

/// A fixed set of session operations, never arbitrary command strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerAction {
    Lock,
    Suspend,
    Logout,
    Restart,
    Shutdown,
}

impl PowerAction {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lock => "Lock",
            Self::Suspend => "Suspend",
            Self::Logout => "Log out",
            Self::Restart => "Restart",
            Self::Shutdown => "Shut down",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerInput {
    Open,
    Back,
    Choose(PowerAction),
    Confirm,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PowerMenu {
    #[default]
    Closed,
    Actions,
    Confirm(PowerAction),
    Running(PowerAction),
    Failed(PowerAction),
}

impl PowerMenu {
    /// Return the action to run only after the required confirmation.
    pub fn input(&mut self, input: PowerInput) -> Option<PowerAction> {
        match (*self, input) {
            (Self::Closed, PowerInput::Open)
            | (Self::Confirm(_) | Self::Failed(_), PowerInput::Back) => *self = Self::Actions,
            (Self::Actions, PowerInput::Back) => *self = Self::Closed,
            (Self::Actions, PowerInput::Choose(PowerAction::Lock)) => {
                *self = Self::Running(PowerAction::Lock);
                return Some(PowerAction::Lock);
            }
            (Self::Actions, PowerInput::Choose(action)) => *self = Self::Confirm(action),
            (Self::Confirm(action), PowerInput::Confirm) => {
                *self = Self::Running(action);
                return Some(action);
            }
            _ => {}
        }
        None
    }

    pub fn finish(&mut self, action: PowerAction, success: bool) {
        if *self == Self::Running(action) {
            *self = if success {
                Self::Closed
            } else {
                Self::Failed(action)
            };
        }
    }
}

pub(crate) fn draw_power_menu(
    out: &mut SceneOutput,
    palette: &ThemePalette,
    width: f32,
    height: f32,
    menu: PowerMenu,
) {
    let panel_w = (width - 16.0).clamp(1.0, 404.0);
    let panel_h = (height - 16.0).clamp(1.0, 290.0);
    let panel_x = (width - panel_w) * 0.5;
    let panel_y = (height - panel_h) * 0.5;
    let panel = HitRect::new(panel_x, panel_y, panel_w, panel_h);
    out.draw_list.push_rect(
        RoundedRect::filled(panel.x, panel.y, panel.w, panel.h, 28.0, palette.surface)
            .with_glass_border(palette.foreground.with_alpha(0.14), 1.0, 0.0),
    );
    out.push_hit(
        HitRect::new(0.0, 0.0, width, height),
        HitTarget::Power(PowerInput::Back),
    );
    // Consume taps on the panel so the dialog doesn't close between buttons.
    out.push_hit(panel, HitTarget::PowerPanel);
    let title = match menu {
        PowerMenu::Closed | PowerMenu::Actions => "Power".to_owned(),
        PowerMenu::Confirm(action) => format!("{}?", action.label()),
        PowerMenu::Running(action) => format!("{} requested", action.label()),
        PowerMenu::Failed(action) => format!("{} failed", action.label()),
    };
    out.draw_list.push_text(
        width * 0.5,
        panel_y + 20.0,
        title,
        2,
        palette.foreground,
        TextAlign::Center,
    );
    let controls: &[_] = match menu {
        PowerMenu::Actions => &[
            ("Lock", PowerInput::Choose(PowerAction::Lock)),
            ("Suspend", PowerInput::Choose(PowerAction::Suspend)),
            ("Log out", PowerInput::Choose(PowerAction::Logout)),
            ("Restart", PowerInput::Choose(PowerAction::Restart)),
            ("Shut down", PowerInput::Choose(PowerAction::Shutdown)),
            ("Back", PowerInput::Back),
        ],
        PowerMenu::Confirm(_) => &[
            ("Cancel", PowerInput::Back),
            ("Confirm", PowerInput::Confirm),
        ],
        PowerMenu::Failed(_) => &[("Back", PowerInput::Back)],
        PowerMenu::Closed | PowerMenu::Running(_) => &[],
    };
    let rows = controls.len().div_ceil(2);
    let h = ((panel.h - 58.0 - 8.0 * rows.saturating_sub(1) as f32) / rows.max(1) as f32)
        .clamp(1.0, 62.0);
    let w = ((panel.w - 40.0) * 0.5).max(1.0);
    for (index, &(label, input)) in controls.iter().enumerate() {
        let x = panel.x + 16.0 + (index % 2) as f32 * (w + 8.0);
        let y = panel.y + 46.0 + (index / 2) as f32 * (h + 8.0);
        let rect = HitRect::new(x, y, w, h);
        out.draw_list.push_rect(RoundedRect::filled(
            x,
            y,
            w,
            h,
            14.0,
            if input == PowerInput::Confirm {
                palette.error
            } else {
                palette.background
            },
        ));
        out.draw_list.push_text(
            x + w * 0.5,
            y + (h - 14.0) * 0.5,
            crate::font::fit_text(label, 2, w - 8.0),
            2,
            if input == PowerInput::Confirm {
                palette.background
            } else {
                palette.foreground
            },
            TextAlign::Center,
        );
        out.push_hit(rect, HitTarget::Power(input));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_scene_layout, SceneContext, SceneKind};

    #[test]
    fn confirmation_cannot_be_skipped_or_replayed() {
        let mut menu = PowerMenu::Closed;
        assert_eq!(menu.input(PowerInput::Confirm), None);
        assert_eq!(menu.input(PowerInput::Open), None);
        for action in [
            PowerAction::Suspend,
            PowerAction::Logout,
            PowerAction::Restart,
            PowerAction::Shutdown,
        ] {
            assert_eq!(menu.input(PowerInput::Choose(action)), None);
            assert_eq!(menu.input(PowerInput::Back), None);
            assert_eq!(menu.input(PowerInput::Confirm), None);
            assert_eq!(menu.input(PowerInput::Choose(action)), None);
            assert_eq!(menu.input(PowerInput::Confirm), Some(action));
            assert_eq!(menu.input(PowerInput::Confirm), None);
            menu.finish(action, false);
            assert_eq!(menu, PowerMenu::Failed(action));
            menu.input(PowerInput::Back);
        }
        assert_eq!(
            menu.input(PowerInput::Choose(PowerAction::Lock)),
            Some(PowerAction::Lock)
        );
        menu.finish(PowerAction::Lock, true);
        assert_eq!(menu, PowerMenu::Closed);
    }

    #[test]
    fn preview_power_dialog_replaces_shade_controls() {
        let palette = ThemePalette::default_dark();
        let mut ctx = SceneContext::default();
        let shade =
            |ctx: &SceneContext| build_scene_layout(SceneKind::Shade, 390.0, 518.0, &palette, ctx);
        assert!(shade(&ctx)
            .hit_regions
            .iter()
            .all(|hit| !matches!(hit.target, HitTarget::Power(_))));
        ctx.power_menu = Some(PowerMenu::Closed);
        assert!(shade(&ctx)
            .hit_regions
            .iter()
            .any(|hit| hit.target == HitTarget::Power(PowerInput::Open)));
        ctx.power_menu = Some(PowerMenu::Confirm(PowerAction::Logout));
        let dialog = shade(&ctx);
        assert!(!dialog.hit_regions.iter().any(|hit| matches!(
            hit.target,
            HitTarget::ShadeToggle { .. } | HitTarget::ShadeSlider { .. }
        )));
        let confirm = dialog
            .hit_regions
            .iter()
            .find(|hit| hit.target == HitTarget::Power(PowerInput::Confirm))
            .unwrap();
        let rect = confirm.rect;
        assert_eq!(
            dialog.hit_test(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5),
            Some(&confirm.target)
        );
    }
}
