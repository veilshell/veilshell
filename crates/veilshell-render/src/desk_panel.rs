//! Built-in Desk Mode pills. Layout and hit-testing use the same bounds.

use veilshell_core::ThemePalette;

use crate::{DrawList, HitRect, RoundedRect, SceneContext, SceneKind, TextAlign};

pub const DESK_PANEL_HEIGHT: u32 = 44;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeskPanelAction {
    pub scene: SceneKind,
    pub bounds: HitRect,
}

#[must_use]
pub fn desk_panel_actions(width: f32) -> [DeskPanelAction; 3] {
    let gap = 6.0;
    let w = ((width - 4.0 * gap) / 3.0).clamp(0.0, 112.0).floor();
    [
        (SceneKind::Launcher, gap),
        (SceneKind::Recents, gap + w + gap),
        (SceneKind::Shade, width - w - gap),
    ]
    .map(|(scene, x)| DeskPanelAction {
        scene,
        bounds: HitRect::new(x, 0.0, w, DESK_PANEL_HEIGHT as f32),
    })
}

#[must_use]
pub fn build_desk_panel(width: f32, palette: &ThemePalette, ctx: &SceneContext) -> DrawList {
    let mut out = DrawList::default();
    for (action, label) in desk_panel_actions(width)
        .into_iter()
        .zip(["Apps", "Recents", "Controls"])
    {
        let r = action.bounds;
        out.push_rect(
            RoundedRect::filled(r.x, 5.0, r.w, 34.0, 17.0, palette.surface.with_alpha(0.94))
                .with_glass_border(palette.foreground.with_alpha(0.16), 1.0, 0.10),
        );
        out.push_text(
            r.x + r.w * 0.5,
            17.0,
            crate::font::fit_text(label, 1, r.w - 8.0),
            1,
            palette.foreground,
            TextAlign::Center,
        );
    }
    if width >= 700.0 {
        out.push_rect(
            RoundedRect::filled(
                width * 0.5 - 51.0,
                5.0,
                102.0,
                34.0,
                17.0,
                palette.surface.with_alpha(0.94),
            )
            .with_glass_border(palette.foreground.with_alpha(0.16), 1.0, 0.10),
        );
        out.push_text(
            width * 0.5,
            17.0,
            &ctx.clock_text,
            1,
            palette.foreground,
            TextAlign::Center,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_fit_without_overlap_at_tablet_and_desktop_sizes() {
        for width in [192.0, 390.0, 853.0, 1280.0] {
            let [apps, recents, controls] = desk_panel_actions(width);
            assert!(apps.bounds.w >= 44.0);
            assert!(apps.bounds.x + apps.bounds.w < recents.bounds.x);
            assert!(recents.bounds.x + recents.bounds.w < controls.bounds.x);
            assert!(controls.bounds.x + controls.bounds.w < width);
        }
    }
}
