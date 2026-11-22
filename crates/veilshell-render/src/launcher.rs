use std::sync::Arc;

use veilshell_core::ThemePalette;

use crate::font::fit_text;
use crate::{
    AppTile, HitRect, HitTarget, ImageQuad, RoundedRect, SceneContext, SceneOutput, TextAlign,
};

/// Search and selection state for the keyboard-first desktop launcher.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LauncherState {
    query: String,
    selected: usize,
}

impl LauncherState {
    pub fn type_text(&mut self, text: &str) {
        let remaining = 128 - self.query.chars().count();
        self.query
            .extend(text.chars().filter(|c| !c.is_control()).take(remaining));
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    fn results(&self, apps: &[AppTile]) -> Vec<usize> {
        let query = self.query.to_lowercase();
        let mut results: Vec<_> = apps
            .iter()
            .enumerate()
            .filter_map(|(i, app)| {
                let name = app.label.to_lowercase();
                let id = app.id.to_lowercase();
                query
                    .split_whitespace()
                    .all(|s| name.contains(s) || id.contains(s))
                    .then_some((i, name))
            })
            .collect();
        results.sort_by(|(ai, a), (bi, b)| a.cmp(b).then_with(|| apps[*ai].id.cmp(&apps[*bi].id)));
        results.into_iter().map(|(i, _)| i).collect()
    }

    pub fn move_selection(&mut self, apps: &[AppTile], width: f32, forward: bool, vertical: bool) {
        let len = self.results(apps).len();
        if len == 0 {
            return;
        }
        let i = self.selected.min(len - 1);
        let cols = columns(panel_width(width));
        self.selected = if vertical {
            let row = i / cols;
            let rows = len.div_ceil(cols);
            let next_row = adjacent(row, rows, forward);
            (next_row * cols + i % cols).min(len - 1)
        } else {
            adjacent(i, len, forward)
        };
    }

    #[must_use]
    pub fn selected_app(&self, apps: &[AppTile]) -> Option<usize> {
        let results = self.results(apps);
        results
            .get(self.selected.min(results.len().saturating_sub(1)))
            .copied()
    }
}

fn adjacent(i: usize, len: usize, forward: bool) -> usize {
    if forward {
        (i + 1) % len
    } else {
        i.checked_sub(1).unwrap_or(len - 1)
    }
}

fn panel_width(width: f32) -> f32 {
    (width - 32.0).clamp(0.0, 760.0)
}

fn columns(width: f32) -> usize {
    ((width - 28.0) / 125.0).clamp(1.0, 5.0) as usize
}

pub(crate) fn build_launcher(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let w = panel_width(width);
    let h = (height - 32.0).clamp(0.0, 548.0);
    if w < 104.0 || h < 112.0 {
        return;
    }
    let x = (width - w) * 0.5;
    let y = (height - h) * 0.5;
    out.draw_list.push_rect(RoundedRect::filled(
        0.0,
        0.0,
        width,
        height,
        0.0,
        palette.background.with_alpha(0.28),
    ));
    out.push_hit(
        HitRect::new(0.0, 0.0, width, height),
        HitTarget::DismissOverlay,
    );
    out.draw_list.push_rect(
        RoundedRect::filled(x, y, w, h, 24.0, palette.surface.with_alpha(0.98)).with_glass_border(
            palette.foreground.with_alpha(0.22),
            1.0,
            0.12,
        ),
    );
    out.push_hit(HitRect::new(x, y, w, h), HitTarget::LauncherPanel);
    out.draw_list.push_rect(RoundedRect::filled(
        x + 14.0,
        y + 14.0,
        w - 28.0,
        46.0,
        14.0,
        palette.background,
    ));
    let query = &ctx.launcher.query;
    let text = if query.is_empty() {
        "Search applications…"
    } else {
        query
    };
    out.draw_list.push_text(
        x + 28.0,
        y + 30.0,
        fit_text(text, 2, w - 56.0),
        2,
        palette
            .foreground
            .with_alpha(if query.is_empty() { 0.76 } else { 1.0 }),
        TextAlign::Left,
    );

    let results = ctx.launcher.results(&ctx.home_apps);
    let cols = columns(w);
    let rows = ((h - 99.0) / 104.0).max(1.0) as usize;
    let selected = ctx.launcher.selected.min(results.len().saturating_sub(1));
    let first = (selected / cols + 1).saturating_sub(rows) * cols;
    let cell = (w - 28.0) / cols as f32;
    let tile_h = (h - 97.0).clamp(15.0, 96.0);
    for (slot, &index) in results.iter().skip(first).take(rows * cols).enumerate() {
        let rect = HitRect::new(
            x + 14.0 + (slot % cols) as f32 * cell,
            y + 72.0 + (slot / cols) as f32 * 104.0,
            cell - 7.0,
            tile_h,
        );
        draw_tile(
            out,
            rect,
            &ctx.home_apps[index],
            palette,
            first + slot == selected,
        );
        out.push_hit(rect, HitTarget::LaunchApp { index });
    }
    let status = if results.is_empty() {
        "No matching applications".to_owned()
    } else {
        format!(
            "{} / {}   ·   Enter to open   ·   Esc to close",
            selected + 1,
            results.len()
        )
    };
    out.draw_list.push_text(
        x + 14.0,
        y + h - 19.0,
        fit_text(&status, 1, w - 28.0),
        1,
        palette.foreground.with_alpha(0.76),
        TextAlign::Left,
    );
}

fn draw_tile(
    out: &mut SceneOutput,
    rect: HitRect,
    app: &AppTile,
    palette: &ThemePalette,
    selected: bool,
) {
    let fill = if selected {
        palette.accent.with_alpha(0.22)
    } else {
        palette.background
    };
    let cx = rect.x + rect.w * 0.5;
    out.draw_list.push_rect(
        RoundedRect::filled(rect.x, rect.y, rect.w, rect.h, 13.0, fill).with_glass_border(
            palette.accent.with_alpha(if selected { 0.9 } else { 0.0 }),
            1.0,
            0.1,
        ),
    );
    if rect.h >= 65.0 {
        if let Some(icon) = &app.icon {
            let scale = 38.0 / icon.width.max(icon.height) as f32;
            let w = icon.width as f32 * scale;
            let h = icon.height as f32 * scale;
            out.draw_list.push_image(ImageQuad {
                x: cx - w * 0.5,
                y: rect.y + 10.0 + (38.0 - h) * 0.5,
                w,
                h,
                radius: 0.0,
                alpha: 1.0,
                image: Arc::clone(icon),
            });
        } else {
            out.draw_list.push_text(
                cx,
                rect.y + 18.0,
                &app.badge,
                2,
                palette.accent,
                TextAlign::Center,
            );
        }
    }
    out.draw_list.push_text(
        cx,
        rect.y + (rect.h - 18.0).max(0.0),
        fit_text(&app.label, 1, rect.w - 8.0),
        1,
        palette.foreground,
        TextAlign::Center,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> SceneContext {
        let mut ctx = SceneContext::default();
        ctx.home_apps.truncate(3);
        for (app, label) in ctx.home_apps.iter_mut().zip(["Zulu", "Alpha", "Beta"]) {
            label.clone_into(&mut app.label);
            app.id = format!("{label}.desktop");
        }
        ctx
    }

    #[test]
    fn launcher_sorts_filters_and_keeps_original_app_indices() {
        let mut ctx = fixture();
        assert_eq!(ctx.launcher.selected_app(&ctx.home_apps), Some(1));
        ctx.launcher
            .move_selection(&ctx.home_apps, 900.0, true, false);
        assert_eq!(ctx.launcher.selected_app(&ctx.home_apps), Some(2));
        ctx.launcher.type_text("ZULU desktop");
        assert_eq!(ctx.launcher.selected_app(&ctx.home_apps), Some(0));
        ctx.launcher.type_text("x");
        assert_eq!(ctx.launcher.selected_app(&ctx.home_apps), None);
        ctx.launcher.backspace();
        assert_eq!(ctx.launcher.selected_app(&ctx.home_apps), Some(0));
    }

    #[test]
    fn launcher_backdrop_dismisses_but_panel_consumes_clicks() {
        let ctx = fixture();
        let mut out = SceneOutput::default();
        build_launcher(&mut out, 900.0, 640.0, &ThemePalette::default_dark(), &ctx);
        assert_eq!(out.hit_test(0.0, 0.0), Some(&HitTarget::DismissOverlay));
        assert_eq!(out.hit_test(450.0, 78.0), Some(&HitTarget::LauncherPanel));
        assert_eq!(
            out.hit_test(110.0, 145.0),
            Some(&HitTarget::LaunchApp { index: 1 })
        );
    }
}
