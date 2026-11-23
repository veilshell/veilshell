//! The touch-first all-apps sheet, separate from the desktop keyboard launcher.

use veilshell_core::ThemePalette;

use crate::scene::{
    header_dismiss_bounds, push_bottom_pill_bar, push_header_dismiss, push_home_tile,
};
use crate::{
    AppTile, HitRect, HitTarget, RoundedRect, SceneContext, SceneOutput, TextAlign,
    TABLET_STATUS_HEIGHT,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabletDrawerState {
    order: Vec<usize>,
    pub scroll: f32,
}

impl TabletDrawerState {
    /// Start a new visit from the top of the catalog.
    pub fn open(&mut self, apps: &[AppTile]) {
        self.rebuild(apps);
        self.scroll = 0.0;
    }

    /// Keep original indices so Home and All Apps launch the same entry.
    /// Replacing the catalog does not reset a settled scroll position.
    pub fn rebuild(&mut self, apps: &[AppTile]) {
        self.order = sorted_indices(apps);
    }
}

fn sorted_indices(apps: &[AppTile]) -> Vec<usize> {
    let mut order: Vec<_> = (0..apps.len()).collect();
    order.sort_by_cached_key(|&i| (apps[i].label.to_lowercase(), &apps[i].id));
    order
}

struct DrawerGrid {
    columns: usize,
    inset: f32,
    height: f32,
    top: f32,
    bottom: f32,
    cell_w: f32,
    row_h: f32,
}

impl DrawerGrid {
    const GAP_X: f32 = 22.0;
    const GAP_Y: f32 = 18.0;

    fn new(width: f32, height: f32) -> Self {
        let width = width.max(0.0);
        let height = height.max(0.0);
        let inset = 20.0_f32.min(width * 0.10);
        let top = 72.0_f32.min(height * 0.28);
        let bottom = height - 52.0_f32.min(height * 0.20);
        let usable_width = width - 2.0 * inset;
        let columns = ((usable_width + Self::GAP_X) / 246.0).ceil() as usize;
        let fitting = ((usable_width + Self::GAP_X) / 96.0).floor() as usize;
        let columns = columns.clamp(4, 14).min(fitting.max(1));
        Self {
            columns,
            inset,
            height,
            top,
            bottom,
            cell_w: (usable_width - Self::GAP_X * (columns - 1) as f32) / columns as f32,
            row_h: 131.0_f32.min((bottom - top).max(0.0)),
        }
    }

    fn pitch(&self) -> f32 {
        self.row_h + Self::GAP_Y
    }

    fn max_scroll(&self, count: usize) -> f32 {
        let rows = count.div_ceil(self.columns);
        let content_h = self.pitch() * rows as f32 - Self::GAP_Y;
        (content_h - (self.bottom - self.top)).max(0.0)
    }
}

/// Maximum logical scroll shared by painting and drag/scroll arbitration.
#[must_use]
pub fn drawer_max_scroll(width: f32, height: f32, count: usize) -> f32 {
    DrawerGrid::new(width, height).max_scroll(count)
}

pub(super) fn build_drawer(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    if progress > 0.0 {
        draw_sheet(out, width, height, progress, palette, ctx);
    }
    if ctx.uses_tablet_status() {
        // Only an owned status row stays fixed above the moving header.
        crate::tablet_status::push_tablet_status_content(
            &mut out.draw_list,
            width,
            TABLET_STATUS_HEIGHT as f32,
            palette,
            ctx,
        );
    }
    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
}

fn draw_sheet(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    progress: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let sheet_y = height * (1.0 - progress);
    let grid = DrawerGrid::new(width, height);
    out.draw_list.push_rect(RoundedRect::filled(
        0.0,
        sheet_y,
        width,
        height,
        0.0,
        palette.background.with_alpha(0.96),
    ));
    let apps = &ctx.home_apps;
    let order = if ctx.drawer.order.len() == apps.len() {
        &ctx.drawer.order
    } else {
        // Offscreen fixtures need not explicitly open the live drawer first.
        &sorted_indices(apps)
    };
    draw_rows(out, &grid, sheet_y, progress, palette, ctx, order);
    // The translated header and navigation band clip the scrolling content.
    for (y, h) in [(0.0, grid.top), (grid.bottom, height - grid.bottom)] {
        out.draw_list.push_rect(RoundedRect::filled(
            0.0,
            sheet_y + y,
            width,
            h,
            0.0,
            palette.background,
        ));
    }
    let title_top = if ctx.uses_tablet_status() {
        TABLET_STATUS_HEIGHT as f32 + 2.0
    } else {
        26.0
    };
    let back = header_dismiss_bounds(width, grid.top).filter(|_| ctx.uses_overlay_gap_gestures());
    let title_width = back.map_or(width - 2.0 * grid.inset, |rect| rect.x - grid.inset - 12.0);
    if grid.top >= title_top + 22.0 {
        out.draw_list.push_text(
            grid.inset,
            sheet_y + title_top,
            crate::font::fit_text("All apps", 2, title_width),
            2,
            palette.foreground,
            TextAlign::Left,
        );
    }
    if let Some(mut rect) = back {
        rect.y += sheet_y;
        if rect.y < height {
            push_header_dismiss(&mut out.draw_list, rect, "Back", palette, 1.0);
        }
        if progress >= 1.0 {
            out.push_hit(rect, HitTarget::DismissOverlay);
        }
    }
    if apps.is_empty() && grid.bottom - grid.top >= 24.0 {
        out.draw_list.push_text(
            width * 0.5,
            sheet_y + f32::midpoint(grid.top, grid.bottom) - 8.0,
            crate::font::fit_text("No applications found", 1, width - 2.0 * grid.inset),
            1,
            palette.muted,
            TextAlign::Center,
        );
    }
}

fn draw_rows(
    out: &mut SceneOutput,
    grid: &DrawerGrid,
    sheet_y: f32,
    progress: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
    order: &[usize],
) {
    if grid.row_h <= 0.0 || grid.cell_w <= 0.0 {
        return;
    }
    let count = order.len();
    let scroll = ctx.drawer.scroll.clamp(0.0, grid.max_scroll(count));
    let first_row = (scroll / grid.pitch()) as usize;
    let first = first_row * grid.columns;
    let content_top = if ctx.uses_tablet_status() {
        grid.top.max(TABLET_STATUS_HEIGHT as f32)
    } else {
        grid.top
    };
    for (slot, &index) in order.iter().enumerate().skip(first) {
        let y = grid.top + (slot / grid.columns) as f32 * grid.pitch() - scroll;
        if y >= grid.bottom || sheet_y + y >= grid.height {
            break;
        }
        let x = grid.inset + (slot % grid.columns) as f32 * (grid.cell_w + DrawerGrid::GAP_X);
        let tile = HitRect::new(x, y + sheet_y, grid.cell_w, grid.row_h);
        let hit = push_home_tile(
            &mut out.draw_list,
            &ctx.home_apps[index],
            tile,
            1.0,
            palette.foreground,
        );
        if progress >= 1.0 {
            let start = hit.y.max(content_top);
            let end = (hit.y + hit.h).min(grid.bottom);
            if start < end {
                out.push_hit(
                    HitRect::new(hit.x, start, hit.w, end - start),
                    HitTarget::LaunchApp { index },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_scene_layout, power::PowerMenu, DrawCommand, SceneKind};

    #[test]
    fn drawer_sorts_catalog_indices_and_blocks_hits_until_settled() {
        let mut ctx = SceneContext {
            power_menu: Some(PowerMenu::Closed),
            ..SceneContext::default()
        };
        ctx.home_apps.truncate(3);
        for (tile, label) in ctx.home_apps.iter_mut().zip(["Zebra", "Alpha", "Beta"]) {
            label.clone_into(&mut tile.label);
        }
        ctx.drawer.open(&ctx.home_apps);
        let theme = ThemePalette::default_dark();
        let draw =
            |ctx: &SceneContext| build_scene_layout(SceneKind::Drawer, 800.0, 1280.0, &theme, ctx);
        let open = draw(&ctx);
        assert_eq!(
            open.hit_test(100.0, 105.0),
            Some(&HitTarget::LaunchApp { index: 1 })
        );
        ctx.overlay_progress = 0.5;
        assert_eq!(draw(&ctx).hit_regions, []);
        ctx.overlay_progress = 0.0;
        assert_eq!(draw(&ctx).hit_regions, []);
        ctx.overlay_progress = 1.0;
        ctx.home_apps.swap(0, 1);
        ctx.drawer.rebuild(&ctx.home_apps);
        assert_eq!(
            draw(&ctx).hit_test(100.0, 105.0),
            Some(&HitTarget::LaunchApp { index: 0 }),
        );
        ctx.power_menu = None;
        assert_eq!(draw(&ctx), SceneOutput::default());
    }

    #[test]
    fn empty_drawer_keeps_the_home_page_passive() {
        let ctx = SceneContext {
            power_menu: Some(PowerMenu::Closed),
            home_apps: Vec::new(),
            ..SceneContext::default()
        };
        let out = build_scene_layout(
            SceneKind::Drawer,
            800.0,
            1280.0,
            &ThemePalette::default_dark(),
            &ctx,
        );
        assert_eq!(out.hit_regions, []);
        assert!(out.draw_list.commands.iter().any(|cmd| matches!(
            cmd, crate::DrawCommand::Text(label) if label.text == "No applications found"
        )));
        assert_eq!(drawer_max_scroll(800.0, 1280.0, 0), 0.0);
    }

    #[test]
    fn gap_drawer_scrolls_the_same_catalog_without_status_or_pill_chrome() {
        let mut ctx = SceneContext {
            overlay_gap_gestures: true,
            clock_text: "12:26".into(),
            home_apps: crate::default_demo_apps()
                .into_iter()
                .cycle()
                .take(64)
                .collect(),
            ..SceneContext::default()
        };
        ctx.drawer.open(&ctx.home_apps);
        let last = *ctx.drawer.order.last().unwrap();
        for (w, h) in [(1280.0, 800.0), (800.0, 1280.0)] {
            let grid = DrawerGrid::new(w, h);
            ctx.drawer.scroll = drawer_max_scroll(w, h, ctx.home_apps.len());
            assert!(ctx.drawer.scroll > 0.0);
            let out =
                build_scene_layout(SceneKind::Drawer, w, h, &ThemePalette::default_dark(), &ctx);
            let title = out
                .draw_list
                .commands
                .iter()
                .find_map(|command| match command {
                    DrawCommand::Text(label) if label.text == "All apps" => Some(label),
                    _ => None,
                })
                .unwrap();
            assert!(title.y < TABLET_STATUS_HEIGHT as f32);
            assert!(!out.draw_list.commands.iter().any(|command| {
                matches!(command, DrawCommand::Text(label) if label.text == ctx.clock_text)
                    || matches!(command, DrawCommand::Rect(rect) if rect.y >= h - 30.0 && rect.h <= 11.0)
            }));
            assert!(out.hit_regions.iter().all(|hit| match hit.target {
                HitTarget::LaunchApp { .. } =>
                    hit.rect.y >= grid.top && hit.rect.y + hit.rect.h <= grid.bottom + 0.01,
                HitTarget::DismissOverlay => hit.rect.y + hit.rect.h < grid.top,
                _ => false,
            }));
            let last = out
                .hit_regions
                .iter()
                .find(|hit| hit.target == HitTarget::LaunchApp { index: last })
                .unwrap();
            assert_eq!(
                out.hit_test(
                    last.rect.x + last.rect.w * 0.5,
                    last.rect.y + last.rect.h * 0.5
                ),
                Some(&last.target)
            );
        }
    }

    #[test]
    fn scroll_limit_reaches_last_row_without_touching_header_or_navigation() {
        let count = 64;
        let mut ctx = SceneContext {
            power_menu: Some(PowerMenu::Closed),
            home_apps: crate::default_demo_apps()
                .into_iter()
                .cycle()
                .take(count)
                .collect(),
            ..SceneContext::default()
        };
        ctx.drawer.open(&ctx.home_apps);
        for (w, h) in [(1280.0, 800.0), (800.0, 1280.0), (390.0, 520.0)] {
            let grid = DrawerGrid::new(w, h);
            ctx.drawer.scroll = drawer_max_scroll(w, h, count);
            assert!(ctx.drawer.scroll > 0.0);
            let out =
                build_scene_layout(SceneKind::Drawer, w, h, &ThemePalette::default_dark(), &ctx);
            let last = *ctx.drawer.order.last().unwrap();
            assert!(out
                .hit_regions
                .iter()
                .any(|hit| hit.target == HitTarget::LaunchApp { index: last }));
            assert!(out.hit_regions.iter().all(|hit| {
                hit.rect.y >= grid.top && hit.rect.y + hit.rect.h <= grid.bottom + 0.01
            }));
        }
    }
}
