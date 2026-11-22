//! Glassmorphic scene layout builders and interactive hit-testing for all `VeilShell` surfaces.

use std::sync::Arc;

use veilshell_core::{OskState, Rgba, ShiftState, ThemePalette};

use crate::power::PowerMenu;
use crate::primitives::{
    DrawList, HitRect, HitTarget, ImageQuad, RgbaImage, RoundedRect, SceneOutput, TextAlign,
};
use crate::recents::build_recents;
use crate::shade::{build_shade, ShadeStatus};
use crate::SceneKind;

/// Visual application tile shown in the paged `Home` app grid.
#[derive(Clone, Debug, PartialEq)]
pub struct AppTile {
    pub id: String,
    pub badge: String,
    pub label: String,
    pub exec: String,
    pub tint: Rgba,
    pub icon: Option<Arc<RgbaImage>>,
}

/// Active window card shown in the `Recents` deck.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentsCard {
    pub id: String,
    pub app_id: String,
    pub title: String,
    pub focused: bool,
    pub foreign_id: Option<String>,
    pub thumbnail: Option<Arc<RgbaImage>>,
}

/// Dynamic data injected into scene layout builders (live windows, apps, hardware state, gestures).
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SceneContext {
    pub clock_text: String,
    pub battery_pct: u8,
    pub wifi_connected: bool,
    pub bluetooth_enabled: bool,
    pub lte_connected: bool,
    pub rotation_locked: bool,
    pub tablet_mode: bool,
    pub night_light: bool,
    pub brightness_pct: u8,
    pub volume_pct: u8,
    pub shade: ShadeStatus,
    /// Session controls are absent in the in-desktop overlay mode.
    pub power_menu: Option<PowerMenu>,
    /// Horizontal drag indicator in logical pixels for live pill gesture feedback.
    pub pill_drag_x: f32,
    /// Vertical drag indicator in logical pixels (negative = upward pull).
    pub pill_drag_y: f32,
    /// Downward drag indicator in logical pixels for the top shade handle.
    pub top_drag_y: f32,
    /// Contact feedback before the bottom pill starts moving.
    pub pill_pressed: bool,
    /// Contact feedback before the top handle starts moving.
    pub top_pressed: bool,
    /// Spring progress `0.0..=1.0` for overlay enter/exit transitions.
    pub overlay_progress: f32,
    /// Discovered/pinned applications displayed across pages of `Home`.
    pub home_apps: Vec<AppTile>,
    /// Active 0-based page index inside `Home`.
    pub home_page: usize,
    /// Fractional page position while the pager is being dragged or settling.
    pub home_position: Option<f32>,
    /// Open windows shown in the `Recents` deck.
    pub recents_cards: Vec<RecentsCard>,
    pub recents_index: usize,
    pub recents_position: Option<f32>,
    pub recents_throw: Option<(String, f32)>,
    /// Multi-layer touch keyboard state and modifier latches.
    pub osk_state: OskState,
}

impl Default for SceneContext {
    fn default() -> Self {
        Self {
            clock_text: "12:00".into(),
            battery_pct: 86,
            wifi_connected: true,
            bluetooth_enabled: true,
            lte_connected: true,
            rotation_locked: false,
            tablet_mode: true,
            night_light: false,
            brightness_pct: 72,
            volume_pct: 55,
            shade: ShadeStatus::default(),
            power_menu: None,
            pill_drag_x: 0.0,
            pill_drag_y: 0.0,
            top_drag_y: 0.0,
            pill_pressed: false,
            top_pressed: false,
            overlay_progress: 1.0,
            home_apps: default_demo_apps(),
            home_page: 0,
            home_position: None,
            recents_cards: vec![
                RecentsCard {
                    id: "0x1".into(),
                    app_id: "Foot".into(),
                    title: "Terminal".into(),
                    focused: true,
                    foreign_id: None,
                    thumbnail: Some(Arc::new(demo_thumbnail((28, 32, 40)))),
                },
                RecentsCard {
                    id: "0x2".into(),
                    app_id: "Chromium".into(),
                    title: "VeilShell".into(),
                    focused: false,
                    foreign_id: None,
                    thumbnail: Some(Arc::new(demo_thumbnail((40, 35, 52)))),
                },
                RecentsCard {
                    id: "0x3".into(),
                    app_id: "Obsidian".into(),
                    title: "Surface Notes".into(),
                    focused: false,
                    foreign_id: None,
                    thumbnail: Some(Arc::new(demo_thumbnail((24, 44, 40)))),
                },
            ],
            recents_index: 0,
            recents_position: None,
            recents_throw: None,
            osk_state: OskState::default(),
        }
    }
}

/// A simple synthetic window preview for offline snapshot verification.
fn demo_thumbnail(base: (u8, u8, u8)) -> RgbaImage {
    const W: u32 = 320;
    const H: u32 = 200;
    let mut rgba = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for _x in 0..W {
            let shade = (((y * 3 / H) as i16 - 1).unsigned_abs() * 18) as u8;
            rgba.extend_from_slice(&[
                base.0.saturating_sub(shade),
                base.1.saturating_sub(shade),
                base.2.saturating_sub(shade),
                255,
            ]);
        }
    }
    RgbaImage::new(W, H, rgba).expect("fixed demo thumbnail size")
}

/// Placeholder tiles for deterministic demo scenes only.
#[must_use]
pub fn default_demo_apps() -> Vec<AppTile> {
    [
        ("TERM", "Terminal", "foot", (122, 162, 247)),
        ("WEB", "Browser", "chromium", (166, 161, 231)),
        ("DIR", "Files", "nautilus", (168, 201, 99)),
        ("NOTE", "Notes", "obsidian", (231, 174, 109)),
        ("CONF", "Settings", "systemsettings", (173, 147, 234)),
        ("MUS", "Music", "spotify", (245, 123, 156)),
        ("CAL", "Calendar", "gnome-calendar", (63, 177, 186)),
        ("MAIL", "Mail", "thunderbird", (245, 154, 118)),
        ("IMG", "Photos", "loupe", (133, 185, 255)),
        ("CODE", "Editor", "nvim", (192, 226, 124)),
        ("SYS", "Monitor", "btop", (255, 142, 171)),
        ("APP", "Packages", "pacman", (189, 162, 251)),
    ]
    .into_iter()
    .map(|(badge, label, exec, (r, g, b))| AppTile {
        id: exec.to_owned(),
        badge: badge.to_owned(),
        label: label.to_owned(),
        exec: exec.to_owned(),
        tint: Rgba::from_rgb_u8(r, g, b),
        icon: None,
    })
    .collect()
}

/// Compute total page count (`>= 1`) for the current Home viewport.
#[must_use]
pub fn home_page_count(apps: &[AppTile], width: f32, height: f32) -> usize {
    HomeGrid::new(width, height).page_count(apps.len())
}

/// Logical travel shared by the gesture and the Home layout.
#[must_use]
pub fn home_reveal_travel(height: f32) -> f32 {
    (height * 0.42).clamp(120.0, 360.0)
}

const HOME_FOREGROUND: Rgba = Rgba::from_rgb_u8(247, 247, 248);
const HOME_BACKGROUND: Rgba = Rgba::from_rgb_u8(7, 9, 16);
const HOME_GRID_GAP: f32 = 22.0;
const PAGER_RESERVE: f32 = 28.0;

struct HomeGrid {
    columns: usize,
    rows: usize,
    inset: f32,
    top: f32,
    cell_w: f32,
    cell_h: f32,
    bottom_row_y: f32,
}

impl HomeGrid {
    fn new(width: f32, height: f32) -> Self {
        let inset = 22.0_f32.min(width * 0.06);
        let usable_w = width - inset * 2.0;
        let gap = HOME_GRID_GAP;
        let max_columns = ((usable_w + gap) / (64.0 + gap)).floor() as usize;
        let columns = (((usable_w + gap) / (224.0 + gap)).ceil() as usize)
            .clamp(4, 14)
            .min(max_columns.max(1));
        let cell_w = (usable_w - gap * (columns - 1) as f32) / columns as f32;
        let top = 66.0_f32.min(height * 0.25);
        let available_h = (height - top - 36.0_f32.min(height * 0.2)).max(1.0);
        let min_cell_h = 131.0_f32.min(available_h);
        let rows_h = (available_h - PAGER_RESERVE).max(min_cell_h);
        let phone_h = cell_w / 0.78;
        let preferred_h = if phone_h > 240.0 {
            min_cell_h
        } else {
            phone_h.min((rows_h - gap * 2.0) / 3.0).max(min_cell_h)
        };
        let rows = (((rows_h + gap) / (preferred_h + gap)).floor() as usize).clamp(1, 16);
        let cell_h = if phone_h > 240.0 && rows > 1 {
            (rows_h - gap * (rows - 1) as f32) / rows as f32
        } else {
            preferred_h
        };
        Self {
            columns,
            rows,
            inset,
            top,
            cell_w,
            cell_h,
            bottom_row_y: top + available_h - cell_h,
        }
    }

    fn capacity(&self) -> usize {
        self.columns * self.rows
    }

    fn page_count(&self, count: usize) -> usize {
        count.div_ceil(self.capacity()).max(1)
    }

    fn row_y(&self, row: usize) -> f32 {
        if self.rows > 1 && row == self.rows - 1 {
            self.bottom_row_y
        } else {
            self.top + row as f32 * (self.cell_h + HOME_GRID_GAP)
        }
    }

    fn pager_y(&self) -> f32 {
        if self.rows > 1 {
            let above = self.row_y(self.rows - 2) + self.cell_h.min(126.0);
            f32::midpoint(above, self.bottom_row_y)
        } else {
            self.bottom_row_y + self.cell_h + 12.0
        }
    }
}

/// Build the draw list AND interactive hit-test regions for a given [`SceneKind`].
#[must_use]
pub fn build_scene_layout(
    scene: SceneKind,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) -> SceneOutput {
    let mut out = SceneOutput::default();
    match scene {
        SceneKind::Pill => build_pill(&mut out, width, height, palette, ctx),
        SceneKind::Home => build_home(&mut out, width, height, palette, ctx),
        SceneKind::Recents => build_recents(&mut out, width, height, palette, ctx),
        SceneKind::Shade => build_shade(&mut out, width, height, palette, ctx),
        SceneKind::Osk => build_osk(&mut out, width, height, palette, ctx),
    }
    out
}

/// Build the renderer-agnostic [`DrawList`] for a given [`SceneKind`].
#[must_use]
pub fn build_scene(
    scene: SceneKind,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) -> DrawList {
    build_scene_layout(scene, width, height, palette, ctx).draw_list
}

/// Resting visual bounds of the top pull handle, in surface-local logical coordinates.
#[must_use]
pub fn top_handle_bounds(width: f32, height: f32) -> HitRect {
    let w = 104.0_f32.min(width * 0.8);
    HitRect::new((width - w) * 0.5, (height * 0.5 - 3.5).min(8.5), w, 7.0)
}

/// Resting visual bounds of the bottom pill, in surface-local logical coordinates.
#[must_use]
pub fn bottom_pill_bounds(width: f32, height: f32) -> HitRect {
    let w = 180.0_f32.min(width * 0.8);
    HitRect::new((width - w) * 0.5, height - 16.0, w, 8.0)
}

/// Build the top-center pull handle strip for opening Quick Settings in Tablet Mode.
#[must_use]
pub fn build_top_strip_scene(
    width: f32,
    height: f32,
    palette: &ThemePalette,
    drag_y: f32,
    pressed: bool,
) -> DrawList {
    let mut list = DrawList::default();
    let bounds = top_handle_bounds(width, height);
    let active = pressed || drag_y > 2.0;
    let handle_w = if active {
        (bounds.w * 1.08).min(width)
    } else {
        bounds.w
    };
    let handle_h = if active { 10.0_f32 } else { bounds.h }.min(height);
    let hx = (width - handle_w) * 0.5;
    let hy = (bounds.y + (bounds.h - handle_h) * 0.5 + drag_y.clamp(0.0, 16.0))
        .clamp(0.0, (height - handle_h).max(0.0));
    let fill = if active {
        palette.accent.with_alpha(0.92)
    } else {
        palette.foreground.with_alpha(0.65)
    };
    list.push_rect(
        RoundedRect::filled(hx, hy, handle_w, handle_h, handle_h * 0.5, fill).with_glass_border(
            Rgba::from_rgb_u8(255, 255, 255).with_alpha(0.20),
            0.8,
            0.14,
        ),
    );
    list
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

pub(crate) fn push_bottom_pill_bar(
    list: &mut DrawList,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let bounds = bottom_pill_bounds(width, height);
    let active = ctx.pill_pressed || ctx.pill_drag_x.abs() > 2.0 || ctx.pill_drag_y.abs() > 2.0;
    let pill_w = if active {
        (bounds.w * 1.08).min(width)
    } else {
        bounds.w
    };
    let pill_h = if active { 11.0 } else { bounds.h };
    let cx = width * 0.5 + ctx.pill_drag_x.clamp(-60.0, 60.0);
    let cy = (bounds.y + bounds.h * 0.5 + ctx.pill_drag_y.clamp(-16.0, 4.0)).max(6.0);

    let fill = if active {
        palette.accent.with_alpha(0.95)
    } else {
        Rgba::from_rgb_u8(138, 138, 138)
    };
    list.push_rect(RoundedRect::filled(
        cx - pill_w * 0.5,
        cy - pill_h * 0.5,
        pill_w,
        pill_h,
        pill_h * 0.5,
        fill,
    ));
}

fn build_pill(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
    out.push_hit(HitRect::new(0.0, 0.0, width, height), HitTarget::BottomPill);
}

fn build_home(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    let y_slide = (1.0 - progress) * home_reveal_travel(height);
    // A plain dark fallback also keeps the launcher legible over active windows.
    // The GPU backdrop may substitute the configured, cached wallpaper.
    let wallpaper_tint = HOME_BACKGROUND.with_alpha(progress);
    out.draw_list.push_backdrop(wallpaper_tint, wallpaper_tint);
    out.push_hit(
        HitRect::new(0.0, 0.0, width, height),
        HitTarget::DismissOverlay,
    );
    let fg = HOME_FOREGROUND.with_alpha(progress);
    push_home_status(&mut out.draw_list, width, height, ctx, fg);

    let apps = &ctx.home_apps;
    let grid = HomeGrid::new(width, height);
    let total_pages = grid.page_count(apps.len());
    let page = ctx.home_page.min(total_pages - 1);
    let position = ctx
        .home_position
        .unwrap_or(page as f32)
        .clamp(-0.2, (total_pages - 1) as f32 + 0.2);
    let first = position.floor() as usize;
    let last = (position.ceil() as usize).min(total_pages - 1);
    for visible_page in first..=last {
        push_home_page(out, &grid, ctx, visible_page, width, position, y_slide);
    }
    if apps.is_empty() {
        out.draw_list.push_text(
            width * 0.5,
            height * 0.5 + y_slide,
            "No applications found",
            1,
            fg,
            TextAlign::Center,
        );
    } else if total_pages > 1 {
        push_home_pager(
            out,
            width,
            grid.pager_y() + y_slide,
            position,
            total_pages,
            progress,
        );
    }
    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
    if progress < 1.0 || ctx.home_position.is_some() {
        out.hit_regions.clear();
    }
}

fn push_home_page(
    out: &mut SceneOutput,
    grid: &HomeGrid,
    ctx: &SceneContext,
    page: usize,
    width: f32,
    position: f32,
    y_slide: f32,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    let fg = HOME_FOREGROUND.with_alpha(progress);
    let x_slide = (page as f32 - position) * width;
    let start = page * grid.capacity();
    let icon_box = 92.0_f32.min(grid.cell_w).min(grid.cell_h * 0.73);
    let icon_size = 85.0_f32.min(icon_box);

    for (slot, app) in ctx
        .home_apps
        .iter()
        .skip(start)
        .take(grid.capacity())
        .enumerate()
    {
        let col = slot % grid.columns;
        let row = slot / grid.columns;
        let x = grid.inset + col as f32 * (grid.cell_w + HOME_GRID_GAP) + x_slide;
        if x >= width || x + grid.cell_w <= 0.0 {
            continue;
        }
        let y = grid.row_y(row) + y_slide;
        let cx = x + grid.cell_w * 0.5;
        let cy = y + icon_box * 0.5;
        if let Some(icon) = &app.icon {
            let scale = icon_size / icon.width.max(icon.height) as f32;
            let w = icon.width as f32 * scale;
            let h = icon.height as f32 * scale;
            out.draw_list.push_image(ImageQuad {
                x: cx - w * 0.5,
                y: cy - h * 0.5,
                w,
                h,
                radius: 0.0,
                alpha: progress,
                image: Arc::clone(icon),
            });
        } else {
            out.draw_list.push_rect(
                RoundedRect::filled(
                    cx - icon_size * 0.5,
                    cy - icon_size * 0.5,
                    icon_size,
                    icon_size,
                    icon_size * 0.24,
                    mix(HOME_BACKGROUND, app.tint, 0.28).with_alpha(progress),
                )
                .with_glass_border(
                    HOME_FOREGROUND.with_alpha(0.15 * progress),
                    1.0,
                    0.0,
                ),
            );
            out.draw_list.push_text(
                cx,
                cy - 7.0,
                crate::font::fit_text(&app.badge, 2, icon_size - 4.0),
                2,
                fg,
                TextAlign::Center,
            );
        }
        if grid.cell_w >= 40.0 && grid.cell_h >= 44.0 {
            out.draw_list.push_text(
                cx,
                y + icon_box + 9.0,
                crate::font::fit_text(&app.label, 1, grid.cell_w - 4.0),
                1,
                fg,
                TextAlign::Center,
            );
        }
        out.push_hit(
            HitRect::new(x, y, grid.cell_w, (icon_box + 30.0).min(grid.cell_h)),
            HitTarget::LaunchApp {
                index: start + slot,
            },
        );
    }
}

fn push_home_status(list: &mut DrawList, width: f32, height: f32, ctx: &SceneContext, fg: Rgba) {
    if width >= 160.0 {
        let y = 22.0_f32.min(height * 0.1);
        list.push_text(22.0, y, &ctx.clock_text, 1, fg, TextAlign::Left);
        list.push_text(
            width - 22.0,
            y,
            format!("{}%", ctx.battery_pct),
            1,
            fg,
            TextAlign::Right,
        );
    }
}

fn push_home_pager(
    out: &mut SceneOutput,
    width: f32,
    cy: f32,
    position: f32,
    total: usize,
    progress: f32,
) {
    // Keep the current page visible when the catalog outgrows the indicator.
    let dots = total.min(((width - 13.0) / 15.0) as usize).min(9);
    let position = position.clamp(0.0, (total - 1) as f32);
    let first = (position.round() as usize)
        .saturating_sub(dots / 2)
        .min(total - dots);
    let mut x = (width - (15.0 * dots as f32 + 13.0)) * 0.5;
    for p in first..first + dots {
        let weight = (1.0 - (position - p as f32).abs()).max(0.0);
        let w = 7.0 + 13.0 * weight;
        let alpha = 0.33 + 0.54 * weight;
        out.draw_list.push_rect(RoundedRect::filled(
            x + 4.0,
            cy - 3.5,
            w,
            7.0,
            3.5,
            HOME_FOREGROUND.with_alpha(alpha * progress),
        ));
        out.push_hit(
            HitRect::new(x, cy - 14.0, w + 8.0, 28.0),
            HitTarget::HomePage { page: p },
        );
        x += w + 8.0;
    }
}

fn build_osk(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    out.draw_list.push_rect(
        RoundedRect::filled(
            0.0,
            0.0,
            width,
            height,
            0.0,
            palette.surface.with_alpha(0.92),
        )
        .with_glass_border(palette.foreground.with_alpha(0.16), 1.5, 0.10),
    );

    let rows = ctx.osk_state.rows();
    let pad = 12.0_f32.min(height * 0.045);
    let row_count = rows.len().max(1);
    let gap = 8.0_f32.min((height - pad * 2.0) / (row_count as f32 * 5.0));
    let row_h = ((height - pad * 2.0 - gap * (row_count.saturating_sub(1)) as f32)
        / row_count as f32)
        .max(1.0);

    for (r_idx, row) in rows.iter().enumerate() {
        let y = pad + r_idx as f32 * (row_h + gap);
        let total_units: f32 = row.iter().map(|k| k.width_units).sum();
        let avail_w = width - pad * 2.0 - gap * (row.len().saturating_sub(1)) as f32;
        let unit_w = avail_w / total_units.max(1.0);
        let mut cursor_x = pad;

        for key in row {
            let kw = unit_w * key.width_units;
            let active_mod = (key.id == "shf" && ctx.osk_state.shift != ShiftState::Off)
                || (key.id == "ctl" && ctx.osk_state.ctrl)
                || (key.id == "term" && ctx.osk_state.terminal);
            let highlighted = key.is_accent || active_mod;
            let fill = if highlighted {
                palette.accent.with_alpha(0.90)
            } else {
                palette.background.with_alpha(0.78)
            };
            let fg = if highlighted {
                palette.background
            } else {
                palette.foreground
            };

            out.draw_list.push_rect(
                RoundedRect::filled(cursor_x, y, kw, row_h, 12.0, fill).with_glass_border(
                    palette.foreground.with_alpha(0.14),
                    1.0,
                    0.08,
                ),
            );
            let label_scale = if kw < 50.0 && key.label.len() > 2 {
                1
            } else {
                2
            };
            out.draw_list.push_text(
                cursor_x + kw * 0.5,
                y + (row_h - 7.0 * label_scale as f32) * 0.5,
                crate::font::fit_text(key.label, label_scale, kw - 4.0),
                label_scale,
                fg,
                TextAlign::Center,
            );
            out.push_hit(
                HitRect::new(cursor_x, y, kw, row_h),
                HitTarget::OskKey { label: key.id },
            );
            cursor_x += kw + gap;
        }
    }
}
