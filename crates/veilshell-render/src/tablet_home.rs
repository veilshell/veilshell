//! Geometry and catalog indexing for the standalone Tablet Home's unsaved layout.
//! The overlay launcher has its own grid. See `docs/tablet/home.md` for the Denial reference.

use std::ops::Range;

use crate::HitRect;

const TOP: f32 = 66.0;
const BOTTOM_PAD: f32 = 4.0;
const INSET: f32 = 22.0;
const GAP: f32 = 22.0;
const PAGER_HEIGHT: f32 = 7.0;
const ROW_VISUAL_HEIGHT: f32 = 126.0;
const MIN_TILE_HEIGHT: f32 = 131.0;
const MIN_COMPACT_CELL_WIDTH: f32 = 44.0;

/// Clock placement and target typography, without measuring any text.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TabletHomeClock {
    pub(crate) cell: HitRect,
    /// Logical font size, weight 300 in the reference.
    pub(crate) time_px: f32,
    /// Logical font size, weight 500 in the reference.
    pub(crate) date_px: f32,
    pub(crate) gap: f32,
}

/// Each page uses the same settled, viewport-local coordinates.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TabletHomeGrid {
    columns: usize,
    rows: usize,
    inset: f32,
    cell_w: f32,
    cell_h: f32,
    bottom_row_y: f32,
}

impl TabletHomeGrid {
    pub(crate) fn new(width: f32, height: f32) -> Self {
        let viewport_bottom = height - BOTTOM_PAD;
        let rows_h = viewport_bottom - TOP - PAGER_HEIGHT;
        // Don't move content up into the status row to make a tiny viewport fit.
        if !width.is_finite() || !height.is_finite() || width < 1.0 || rows_h < 1.0 {
            return Self::default();
        }

        let inset = INSET.min(width * 0.1);
        let usable_w = width - 2.0 * inset;
        let columns = (((usable_w + GAP) / (224.0 + GAP)).ceil() as usize).clamp(4, 14);
        // Below the reference's useful width, fit fewer columns instead of
        // leaving them with no room between gaps. Keep one even below 44 px.
        let fitting_columns = ((usable_w + GAP) / (MIN_COMPACT_CELL_WIDTH + GAP)).floor() as usize;
        let columns = columns.min(fitting_columns.max(1));
        let cell_w = (usable_w - GAP * (columns - 1) as f32) / columns as f32;

        let phone_h = cell_w / 0.78;
        let min_h = MIN_TILE_HEIGHT.min(rows_h);
        let (rows, cell_h) = if phone_h > 240.0 {
            let rows = rows_for_height(rows_h, min_h);
            let cell_h = ((rows_h - GAP * (rows - 1) as f32) / rows as f32).max(min_h);
            (rows, cell_h)
        } else {
            let cell_h = phone_h.min((rows_h - 2.0 * GAP) / 3.0).max(min_h);
            (rows_for_height(rows_h, cell_h), cell_h)
        };
        let bottom_row_y = if rows == 1 {
            TOP
        } else {
            (TOP + (rows - 1) as f32 * (cell_h + GAP) + PAGER_HEIGHT).max(viewport_bottom - cell_h)
        };
        Self {
            columns,
            rows,
            inset,
            cell_w,
            cell_h,
            bottom_row_y,
        }
    }

    /// Number of slots on a page before reserving any for the clock.
    pub(crate) fn capacity(&self) -> usize {
        self.columns * self.rows
    }

    /// There is always a first page, even without apps or on an unusable viewport.
    pub(crate) fn page_count(&self, app_count: usize) -> usize {
        let capacity = self.capacity();
        if capacity == 0 {
            return 1;
        }
        let remaining = app_count.saturating_sub(capacity - self.first_app_slot(0));
        1_usize.saturating_add(remaining.div_ceil(capacity))
    }

    /// The contiguous range of catalog indices on this page, in slot order.
    pub(crate) fn app_range(&self, page: usize, app_count: usize) -> Range<usize> {
        let capacity = self.capacity();
        let start = if page == 0 {
            0
        } else {
            (page - 1)
                .saturating_mul(capacity)
                .saturating_add(capacity - self.first_app_slot(0))
                .min(app_count)
        };
        let end = start
            .saturating_add(capacity - self.first_app_slot(page))
            .min(app_count);
        start..end
    }

    /// Cell of an original catalog index on this page. Clock slots never map to apps.
    pub(crate) fn app_cell(&self, page: usize, index: usize, app_count: usize) -> Option<HitRect> {
        let apps = self.app_range(page, app_count);
        if !apps.contains(&index) {
            return None;
        }
        self.cell_for_slot(index - apps.start + self.first_app_slot(page))
    }

    /// Raw row-major slot geometry, including slots that aren't app targets.
    pub(crate) fn cell_for_slot(&self, slot: usize) -> Option<HitRect> {
        if slot >= self.capacity() {
            return None;
        }
        let column = slot % self.columns;
        let row = slot / self.columns;
        Some(HitRect::new(
            self.inset + column as f32 * (self.cell_w + GAP),
            self.row_y(row),
            self.cell_w,
            self.cell_h,
        ))
    }

    /// The display-only clock spans the first two columns of page zero, or one
    /// column on a very narrow grid.
    pub(crate) fn clock(&self, page: usize) -> Option<TabletHomeClock> {
        let span = self.first_app_slot(page);
        if span == 0 {
            return None;
        }
        let mut cell = self.cell_for_slot(0)?;
        cell.w = span as f32 * self.cell_w + (span - 1) as f32 * GAP;

        let width_size = cell.w * 0.36;
        let time_px = width_size.min(cell.h * 0.42).clamp(46.0, 124.0);
        let detail_scale = (time_px / 58.0).clamp(0.9, 1.28);
        let date_px = (15.0 * detail_scale).clamp(13.0, 19.0);
        let gap = (7.0 * detail_scale).clamp(5.0, 9.0);
        // The reference sizes are pre-fit. On a compact cell, scale the
        // whole block down if the nominal sizes don't fit. The renderer
        // still needs to fit the actual localized text to the cell.
        let fit = (width_size / time_px)
            .min(cell.h / (time_px + gap + date_px))
            .min(1.0);
        Some(TabletHomeClock {
            cell,
            time_px: time_px * fit,
            date_px: date_px * fit,
            gap: gap * fit,
        })
    }

    /// Logical center Y of the page markers, absent when there's only one page.
    pub(crate) fn pager_y(&self, app_count: usize) -> Option<f32> {
        if self.page_count(app_count) <= 1 {
            return None;
        }
        let visible_h = self.cell_h.min(ROW_VISUAL_HEIGHT);
        Some(if self.rows == 1 {
            TOP + visible_h + PAGER_HEIGHT * 0.5
        } else {
            let upper_bottom = self.row_y(self.rows - 2) + visible_h;
            upper_bottom.midpoint(self.bottom_row_y)
        })
    }

    fn row_y(&self, row: usize) -> f32 {
        if row == self.rows - 1 {
            self.bottom_row_y
        } else {
            TOP + row as f32 * (self.cell_h + GAP)
        }
    }

    fn first_app_slot(&self, page: usize) -> usize {
        if page == 0 {
            self.columns.min(2)
        } else {
            0
        }
    }
}

fn rows_for_height(height: f32, tile_h: f32) -> usize {
    // Keep the last row when cells fill the available height exactly.
    (((height + GAP) / (tile_h + GAP) + 1e-6).floor() as usize).clamp(1, 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
    }

    #[test]
    fn denial_portrait_fixture() {
        let grid = TabletHomeGrid::new(800.0, 1280.0);
        assert_eq!((grid.columns, grid.rows), (4, 5));
        let clock = grid.clock(0).unwrap();
        close(clock.cell.x, 22.0);
        close(clock.cell.y, 66.0);
        close(clock.cell.w, 367.0);
        close(clock.time_px, 92.885);
        close(clock.date_px, 19.0);
        close(clock.gap, 8.96);
        let app = grid.app_cell(0, 0, 19).unwrap();
        close(app.y, 66.0);
        close(app.w, 172.5);
        close(app.h, 221.15);
        close(app.x + app.w * 0.5, 497.25);
        let last_row = grid.cell_for_slot(16).unwrap();
        close(last_row.y + last_row.h, 1276.0);
        close(grid.pager_y(19).unwrap(), 988.154);
    }

    #[test]
    fn denial_landscape_fixture() {
        let grid = TabletHomeGrid::new(1280.0, 800.0);
        assert_eq!((grid.columns, grid.rows), (6, 4));
        let clock = grid.clock(0).unwrap();
        close(clock.cell.w, 397.333);
        close(clock.time_px, 68.985);
        close(clock.date_px, 17.841);
        close(clock.gap, 8.326);
        let app = grid.app_cell(0, 0, 23).unwrap();
        close(app.y, 66.0);
        close(app.w, 187.67);
        close(app.h, 164.25);
        close(app.x + app.w * 0.5, 535.17);
        let last_row = grid.cell_for_slot(18).unwrap();
        close(last_row.y + last_row.h, 796.0);
        close(grid.pager_y(23).unwrap(), 598.125);
    }

    #[test]
    fn page_boundaries_keep_catalog_order_without_gaps_or_duplicates() {
        let grid = TabletHomeGrid::new(800.0, 1280.0);
        assert_eq!(grid.page_count(18), 1);
        assert!(grid.pager_y(18).is_none());
        assert_eq!(grid.page_count(19), 2);
        assert_eq!(grid.page_count(38), 2);
        assert_eq!(grid.page_count(39), 3);
        assert_eq!(grid.app_range(0, 39), 0..18);
        assert_eq!(grid.app_range(1, 39), 18..38);
        assert_eq!(grid.app_range(2, 39), 38..39);
        assert_eq!(grid.app_range(3, 39), 39..39);
        assert_eq!(grid.app_cell(0, 17, 39), grid.cell_for_slot(19));
        assert_eq!(grid.app_cell(1, 18, 39), grid.cell_for_slot(0));
        assert_eq!(grid.app_cell(1, 37, 39), grid.cell_for_slot(19));
        assert!(grid.app_cell(0, 18, 39).is_none());
        assert!(grid.app_cell(1, 17, 39).is_none());
        assert!(grid.app_cell(2, 39, 39).is_none());
    }

    #[test]
    fn clock_slots_are_never_app_targets_on_page_zero() {
        let grid = TabletHomeGrid::new(800.0, 1280.0);
        let clock = grid.clock(0).unwrap().cell;
        assert!(grid.clock(1).is_none());
        assert_eq!(grid.app_cell(0, 0, 20), grid.cell_for_slot(2));
        for index in grid.app_range(0, 20) {
            let app = grid.app_cell(0, index, 20).unwrap();
            assert!(app.x > clock.x + clock.w || app.y > clock.y + clock.h);
        }
        assert_eq!(grid.app_cell(1, 18, 20), grid.cell_for_slot(0));
        assert_eq!(grid.app_cell(1, 19, 20), grid.cell_for_slot(1));
    }

    #[test]
    fn empty_catalog_still_has_a_clock_but_no_targets_or_pager() {
        let grid = TabletHomeGrid::new(800.0, 1280.0);
        assert_eq!(grid.page_count(0), 1);
        assert_eq!(grid.app_range(0, 0), 0..0);
        assert!(grid.app_cell(0, 0, 0).is_none());
        assert!(grid.clock(0).is_some());
        assert!(grid.pager_y(0).is_none());
    }

    #[test]
    fn compact_one_cell_page_keeps_clock_and_apps_below_status() {
        let grid = TabletHomeGrid::new(60.0, 100.0);
        assert_eq!((grid.columns, grid.rows), (1, 1));
        let clock = grid.clock(0).unwrap();
        assert_eq!(Some(clock.cell), grid.cell_for_slot(0));
        close(clock.cell.y, 66.0);
        close(clock.cell.y + clock.cell.h, 89.0);
        assert!(clock.cell.x + clock.cell.w <= 60.0);
        assert!(clock.time_px + clock.gap + clock.date_px <= clock.cell.h + 0.01);
        assert_eq!(grid.page_count(1), 2);
        assert!(grid.app_cell(0, 0, 1).is_none());
        assert_eq!(grid.app_cell(1, 0, 1), grid.cell_for_slot(0));
        close(grid.pager_y(1).unwrap(), 92.5);
    }

    #[test]
    fn unusable_viewports_have_no_cells_or_pager() {
        for (width, height) in [
            (0.0, 800.0),
            (-1.0, 800.0),
            (800.0, 48.0),
            (800.0, 77.0),
            (f32::NAN, 800.0),
            (800.0, f32::INFINITY),
        ] {
            let grid = TabletHomeGrid::new(width, height);
            assert_eq!(grid.capacity(), 0);
            assert_eq!(grid.page_count(10), 1);
            assert!(grid.app_range(0, 10).is_empty());
            assert!(grid.app_cell(0, 0, 10).is_none());
            assert!(grid.cell_for_slot(0).is_none());
            assert!(grid.clock(0).is_none());
            assert!(grid.pager_y(10).is_none());
        }
    }

    #[test]
    fn extreme_page_arguments_do_not_overflow() {
        let grid = TabletHomeGrid::new(800.0, 1280.0);
        let count = usize::MAX;
        let last = grid.page_count(count) - 1;
        assert!(grid.app_cell(last, count - 1, count).is_some());
        assert_eq!(grid.app_range(usize::MAX, count), count..count);
        assert!(grid.cell_for_slot(usize::MAX).is_none());
    }
}
