use veilshell_core::ThemePalette;

use crate::shade_glyphs::{draw, draw_battery, Glyph};
use crate::{DrawList, RoundedRect, SceneContext, TextAlign};

/// Logical height of the reserved Tablet status row, including in Home.
pub const TABLET_STATUS_HEIGHT: u32 = 48;

/// Real telemetry for the standalone tablet strip. Missing devices stay hidden.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TabletStatus {
    pub battery: Option<(u8, bool)>,
    pub wifi: Option<bool>,
}

#[must_use]
pub fn build_tablet_status(w: f32, h: f32, palette: &ThemePalette, ctx: &SceneContext) -> DrawList {
    let mut list = build_tablet_status_background(w, h, palette);
    push_tablet_status_content(&mut list, w, h, palette, ctx);
    list
}

/// Keep the reserved work-area strip painted while the Shade draws its moving telemetry.
#[must_use]
pub fn build_tablet_status_background(w: f32, h: f32, palette: &ThemePalette) -> DrawList {
    let mut list = DrawList::default();
    list.push_rect(RoundedRect::filled(
        0.0,
        0.0,
        w,
        h,
        0.0,
        palette.background.with_alpha(0.9),
    ));
    list
}

pub(crate) fn push_tablet_status_content(
    list: &mut DrawList,
    w: f32,
    h: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let cy = h * 0.5;
    let ink = palette.foreground;
    list.push_text(16.0, cy - 5.0, &ctx.clock_text, 1, ink, TextAlign::Left);
    let mut right = w - 16.0;
    if let Some((pct, charging)) = ctx.tablet_status.battery {
        list.push_text(right, cy - 5.0, format!("{pct}%"), 1, ink, TextAlign::Right);
        let battery_ink = if charging { palette.accent } else { ink };
        draw_battery(list, right - 49.0, cy, 20.0, pct, battery_ink);
        right -= 65.0;
    }
    if let Some(connected) = ctx.tablet_status.wifi {
        draw(
            list,
            Glyph::Wifi,
            right - 10.0,
            cy,
            18.0,
            if connected { ink } else { ink.with_alpha(0.45) },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DrawCommand;

    #[test]
    fn status_shows_only_telemetry_even_during_a_pull() {
        let mut ctx = SceneContext {
            clock_text: "08:15".into(),
            top_drag_y: 16.0,
            top_pressed: true,
            ..SceneContext::default()
        };
        let build = |ctx: &SceneContext| {
            build_tablet_status(
                960.0,
                TABLET_STATUS_HEIGHT as f32,
                &ThemePalette::default_dark(),
                ctx,
            )
        };
        assert!(matches!(
            build(&ctx).commands.as_slice(),
            [DrawCommand::Rect(background), DrawCommand::Text(clock)]
                if (background.w - 960.0).abs() < f32::EPSILON && clock.text == "08:15"
        ));

        ctx.tablet_status = TabletStatus {
            battery: Some((83, true)),
            wifi: Some(false),
        };
        assert!(matches!(
            build(&ctx).commands.as_slice(),
            [
                DrawCommand::Rect(_),
                DrawCommand::Text(_),
                DrawCommand::Text(battery),
                DrawCommand::Image(_),
                DrawCommand::Rect(_),
                DrawCommand::Image(_),
            ] if battery.text == "83%"
        ));
    }
}
