use veilshell_core::ThemePalette;

use crate::shade_glyphs::{draw, draw_battery, Glyph};
use crate::{DrawList, RoundedRect, SceneContext, TextAlign};

/// Real telemetry for the standalone tablet strip. Missing devices stay hidden.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TabletStatus {
    pub battery: Option<(u8, bool)>,
    pub wifi: Option<bool>,
}

#[must_use]
pub fn build_tablet_status(w: f32, h: f32, palette: &ThemePalette, ctx: &SceneContext) -> DrawList {
    let mut list = DrawList::default();
    list.push_rect(RoundedRect::filled(
        0.0,
        0.0,
        w,
        h,
        0.0,
        palette.background.with_alpha(0.9),
    ));
    let cy = h * 0.5;
    let ink = palette.foreground;
    list.push_text(16.0, cy - 5.0, &ctx.clock_text, 1, ink, TextAlign::Left);
    let mut right = w - 16.0;
    if let Some((pct, charging)) = ctx.tablet_status.battery {
        list.push_text(right, cy - 5.0, format!("{pct}%"), 1, ink, TextAlign::Right);
        let battery_ink = if charging { palette.accent } else { ink };
        draw_battery(&mut list, right - 49.0, cy, 20.0, pct, battery_ink);
        right -= 65.0;
    }
    if let Some(connected) = ctx.tablet_status.wifi {
        draw(
            &mut list,
            Glyph::Wifi,
            right - 10.0,
            cy,
            18.0,
            if connected { ink } else { ink.with_alpha(0.45) },
        );
    }
    let handle = crate::build_top_strip_scene(w, h, palette, ctx.top_drag_y, ctx.top_pressed);
    list.commands.extend(handle.commands);
    list
}
