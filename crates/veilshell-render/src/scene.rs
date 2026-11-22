//! Glassmorphic scene layout builders and interactive hit-testing for all `VeilShell` surfaces.

use std::sync::Arc;

use veilshell_core::{OskState, Rgba, ShiftState, ThemePalette};

use crate::power::{draw_power_menu, PowerInput, PowerMenu};
use crate::primitives::{
    DrawList, HitRect, HitTarget, ImageQuad, RgbaImage, RoundedRect, SceneOutput, TextAlign,
};
use crate::SceneKind;

/// Visual application tile shown in the paged `Home` app grid.
#[derive(Clone, Debug, PartialEq)]
pub struct AppTile {
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
    /// Session controls are absent in the in-desktop overlay mode.
    pub power_menu: Option<PowerMenu>,
    /// Horizontal drag indicator in logical pixels for live pill gesture feedback.
    pub pill_drag_x: f32,
    /// Vertical drag indicator in logical pixels (negative = upward pull).
    pub pill_drag_y: f32,
    /// Downward drag indicator in logical pixels for the top shade handle.
    pub top_drag_y: f32,
    /// Spring progress `0.0..=1.0` for overlay enter/exit transitions.
    pub overlay_progress: f32,
    /// Discovered/pinned applications displayed across pages of `Home`.
    pub home_apps: Vec<AppTile>,
    /// Active 0-based page index inside `Home`.
    pub home_page: usize,
    /// Open windows shown in the `Recents` deck.
    pub recents_cards: Vec<RecentsCard>,
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
            power_menu: None,
            pill_drag_x: 0.0,
            pill_drag_y: 0.0,
            top_drag_y: 0.0,
            overlay_progress: 1.0,
            home_apps: default_demo_apps(),
            home_page: 0,
            recents_cards: vec![
                RecentsCard {
                    id: "0x1".into(),
                    app_id: "Foot".into(),
                    title: "Terminal".into(),
                    focused: true,
                    foreign_id: None,
                    thumbnail: None,
                },
                RecentsCard {
                    id: "0x2".into(),
                    app_id: "Chromium".into(),
                    title: "VeilShell".into(),
                    focused: false,
                    foreign_id: None,
                    thumbnail: None,
                },
                RecentsCard {
                    id: "0x3".into(),
                    app_id: "Obsidian".into(),
                    title: "Surface Notes".into(),
                    focused: false,
                    foreign_id: None,
                    thumbnail: None,
                },
            ],
            osk_state: OskState::default(),
        }
    }
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

/// Build the subtle top-center pull handle strip for opening Quick Settings in Tablet Mode.
#[must_use]
pub fn build_top_strip_scene(
    width: f32,
    height: f32,
    palette: &ThemePalette,
    drag_y: f32,
) -> DrawList {
    let mut list = DrawList::default();
    let handle_w = (width * 0.34).clamp(56.0, 92.0);
    let handle_h = 4.5_f32;
    let hx = (width - handle_w) * 0.5;
    let hy = ((height - handle_h) * 0.5 + drag_y.clamp(0.0, 6.0)).clamp(2.0, height - handle_h);
    let fill = if drag_y > 2.0 {
        palette.accent.with_alpha(0.92)
    } else {
        palette.foreground.with_alpha(0.45)
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

fn truncate_label(raw: &str, max_chars: usize) -> String {
    let trimmed = raw.trim();
    let count = trimmed.chars().count();
    if count <= max_chars || max_chars < 4 {
        return trimmed.to_owned();
    }
    let mut out: String = trimmed.chars().take(max_chars - 2).collect();
    out.push_str("..");
    out
}

fn push_glass_backdrop(
    list: &mut DrawList,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    progress: f32,
) {
    let alpha = progress.clamp(0.0, 1.0);
    let top = mix(palette.background, palette.accent, 0.08).with_alpha(0.94 * alpha);
    let bottom =
        mix(palette.background, Rgba::from_rgb_u8(4, 6, 12), 0.35).with_alpha(0.96 * alpha);
    list.push_backdrop(top, bottom);
    list.push_glow(
        width * 0.22,
        height * 0.18,
        width.max(height) * 0.48,
        palette.accent.with_alpha(0.14 * alpha),
    );
    list.push_glow(
        width * 0.82,
        height * 0.82,
        width.max(height) * 0.42,
        palette.accent.with_alpha(0.09 * alpha),
    );
}

fn push_bottom_pill_bar(
    list: &mut DrawList,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let pill_w = 180.0_f32.min(width * 0.8);
    let pill_h = 5.0_f32;
    let cx = width * 0.5 + ctx.pill_drag_x.clamp(-60.0, 60.0);
    let cy = (height - 12.0 + ctx.pill_drag_y.clamp(-14.0, 4.0)).max(6.0);
    let active = ctx.pill_drag_x.abs() > 2.0 || ctx.pill_drag_y.abs() > 2.0;

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

fn build_status_bar(
    list: &mut DrawList,
    width: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
    y_offset: f32,
    alpha: f32,
) {
    let bar_w = (width - 48.0).clamp(240.0, 1160.0);
    let bar_h = 44.0;
    let bar_x = (width - bar_w) * 0.5;
    let bar_y = 20.0 + y_offset;

    list.push_rect(
        RoundedRect::filled(
            bar_x,
            bar_y,
            bar_w,
            bar_h,
            22.0,
            palette.surface.with_alpha(0.76 * alpha),
        )
        .with_glass_border(
            palette.foreground.with_alpha(0.14 * alpha),
            1.0,
            0.08 * alpha,
        )
        .with_shadow(12.0, 4.0, 0.28 * alpha),
    );

    list.push_text(
        bar_x + 22.0,
        bar_y + 15.0,
        &ctx.clock_text,
        2,
        palette.foreground.with_alpha(alpha),
        TextAlign::Left,
    );
    list.push_text(
        width * 0.5,
        bar_y + 15.0,
        "VeilShell",
        2,
        palette.accent.with_alpha(alpha),
        TextAlign::Center,
    );

    let net = match (ctx.wifi_connected, ctx.lte_connected) {
        (true, true) => "Wi-Fi LTE",
        (true, false) => "Wi-Fi",
        (false, true) => "LTE",
        (false, false) => "Offline",
    };
    let status = format!("{net}  {}%", ctx.battery_pct);
    list.push_text(
        bar_x + bar_w - 22.0,
        bar_y + 15.0,
        status,
        2,
        palette.foreground.with_alpha(alpha),
        TextAlign::Right,
    );
}

fn build_home(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    let y_slide = (1.0 - progress) * 24.0;
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
    let start = page * grid.capacity();
    let icon_box = 92.0_f32.min(grid.cell_w).min(grid.cell_h * 0.73);
    let icon_size = 85.0_f32.min(icon_box);

    for (slot, app) in apps.iter().skip(start).take(grid.capacity()).enumerate() {
        let col = slot % grid.columns;
        let row = slot / grid.columns;
        let x = grid.inset + col as f32 * (grid.cell_w + HOME_GRID_GAP);
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
    if apps.is_empty() {
        out.draw_list.push_text(
            width * 0.5,
            height * 0.5,
            "No applications found",
            1,
            fg,
            TextAlign::Center,
        );
    } else if total_pages > 1 {
        push_home_pager(out, width, grid.pager_y(), page, total_pages, progress);
    }
    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
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
    page: usize,
    total: usize,
    progress: f32,
) {
    // Keep the current page visible when the catalog outgrows the indicator.
    let dots = total.min(((width - 13.0) / 15.0) as usize).min(9);
    let first = page.saturating_sub(dots / 2).min(total - dots);
    let mut x = (width - (15.0 * dots as f32 + 13.0)) * 0.5;
    for p in first..first + dots {
        let (w, alpha) = if p == page { (20.0, 0.87) } else { (7.0, 0.33) };
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

#[allow(clippy::too_many_lines)]
fn build_recents(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    let y_slide = (1.0 - progress) * 48.0;
    push_glass_backdrop(&mut out.draw_list, width, height, palette, progress);
    out.push_hit(
        HitRect::new(0.0, 0.0, width, height),
        HitTarget::DismissOverlay,
    );
    build_status_bar(
        &mut out.draw_list,
        width,
        palette,
        ctx,
        y_slide * 0.25,
        progress,
    );

    if ctx.recents_cards.is_empty() {
        let card_w = (width * 0.42).clamp(280.0, 440.0);
        let card_h = 180.0;
        let card_x = (width - card_w) * 0.5;
        let card_y = (height - card_h) * 0.5 + y_slide;
        out.draw_list.push_rect(
            RoundedRect::filled(
                card_x,
                card_y,
                card_w,
                card_h,
                24.0,
                palette.surface.with_alpha(0.84 * progress),
            )
            .with_glass_border(
                palette.foreground.with_alpha(0.16 * progress),
                1.2,
                0.10 * progress,
            )
            .with_shadow(20.0, 10.0, 0.36 * progress),
        );
        out.draw_list.push_text(
            width * 0.5,
            card_y + 68.0,
            "No Open Windows",
            2,
            palette.foreground.with_alpha(progress),
            TextAlign::Center,
        );
        out.draw_list.push_text(
            width * 0.5,
            card_y + 102.0,
            "Tap backdrop to return",
            2,
            palette.muted.with_alpha(progress),
            TextAlign::Center,
        );
        push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
        return;
    }

    let count = ctx.recents_cards.len().min(3);
    let card_w = (width * 0.30).clamp(240.0, 380.0);
    let card_h = (height * 0.58).clamp(260.0, 480.0);
    let gap = 28.0;
    let total_w = card_w * count as f32 + gap * count.saturating_sub(1) as f32;
    let start_x = (width - total_w) * 0.5;
    let base_y = (height - card_h) * 0.5 + 16.0 + y_slide;

    for (idx, card) in ctx.recents_cards.iter().take(count).enumerate() {
        let card_x = start_x + idx as f32 * (card_w + gap);
        let card_y = if card.focused { base_y - 12.0 } else { base_y };
        let border = if card.focused {
            palette.accent.with_alpha(0.90 * progress)
        } else {
            palette.foreground.with_alpha(0.18 * progress)
        };
        let border_w = if card.focused { 2.5 } else { 1.2 };

        out.draw_list.push_rect(
            RoundedRect::filled(
                card_x,
                card_y,
                card_w,
                card_h,
                24.0,
                palette.surface.with_alpha(0.86 * progress),
            )
            .with_glass_border(border, border_w, 0.12 * progress)
            .with_shadow(22.0, 12.0, 0.40 * progress),
        );

        // Header strip
        out.draw_list.push_rect(RoundedRect::filled(
            card_x + 14.0,
            card_y + 14.0,
            card_w - 28.0,
            40.0,
            14.0,
            mix(
                palette.background,
                palette.accent,
                if card.focused { 0.25 } else { 0.08 },
            )
            .with_alpha(0.85 * progress),
        ));
        out.draw_list.push_text(
            card_x + 28.0,
            card_y + 27.0,
            truncate_label(&card.app_id, 16),
            2,
            palette.foreground.with_alpha(progress),
            TextAlign::Left,
        );

        // Close pill badge
        let close_x = card_x + card_w - 46.0;
        let close_y = card_y + 22.0;
        out.draw_list.push_rect(RoundedRect::filled(
            close_x,
            close_y,
            24.0,
            24.0,
            12.0,
            palette.error.with_alpha(0.86 * progress),
        ));
        out.draw_list.push_text(
            close_x + 12.0,
            close_y + 6.0,
            "X",
            2,
            palette.background.with_alpha(progress),
            TextAlign::Center,
        );

        // Card preview pane
        let pane_x = card_x + 18.0;
        let pane_y = card_y + 68.0;
        let pane_w = card_w - 36.0;
        let pane_h = card_h - 118.0;
        out.draw_list.push_rect(
            RoundedRect::filled(
                pane_x,
                pane_y,
                pane_w,
                pane_h,
                16.0,
                palette.background.with_alpha(0.75 * progress),
            )
            .with_glass_border(
                palette.foreground.with_alpha(0.08 * progress),
                1.0,
                0.04 * progress,
            ),
        );
        if let Some(thumb) = &card.thumbnail {
            let avail_w = (pane_w - 8.0).max(16.0);
            let avail_h = (pane_h - 8.0).max(16.0);
            let scale = (avail_w / thumb.width as f32).min(avail_h / thumb.height as f32);
            let iw = (thumb.width as f32 * scale).max(1.0);
            let ih = (thumb.height as f32 * scale).max(1.0);
            let ix = pane_x + (pane_w - iw) * 0.5;
            let iy = pane_y + (pane_h - ih) * 0.5;
            out.draw_list.push_image(ImageQuad {
                x: ix,
                y: iy,
                w: iw,
                h: ih,
                radius: 12.0,
                alpha: progress,
                image: Arc::clone(thumb),
            });
            let pill_h = 24.0_f32.min(pane_h * 0.3);
            let pill_y = pane_y + pane_h - pill_h - 8.0;
            out.draw_list.push_rect(RoundedRect::filled(
                pane_x + 10.0,
                pill_y,
                (pane_w - 20.0).max(24.0),
                pill_h,
                8.0,
                palette.background.with_alpha(0.76 * progress),
            ));
            out.draw_list.push_text(
                card_x + card_w * 0.5,
                pill_y + (pill_h - 7.0) * 0.5,
                truncate_label(&card.title, 26),
                1,
                palette.foreground.with_alpha(progress),
                TextAlign::Center,
            );
        } else {
            out.draw_list.push_text(
                card_x + card_w * 0.5,
                card_y + card_h * 0.5,
                truncate_label(&card.title, 20),
                2,
                palette.muted.with_alpha(progress),
                TextAlign::Center,
            );
        }
        out.draw_list.push_text(
            card_x + card_w * 0.5,
            card_y + card_h - 30.0,
            if card.focused {
                "Active"
            } else {
                "Tap to focus"
            },
            2,
            if card.focused {
                palette.accent.with_alpha(progress)
            } else {
                palette.muted.with_alpha(progress)
            },
            TextAlign::Center,
        );

        out.push_hit(
            HitRect::new(card_x, card_y, card_w, card_h),
            HitTarget::FocusWindow {
                id: card.id.clone(),
            },
        );
        // Close hit region registered AFTER card so tapping X always closes instead of focusing
        out.push_hit(
            HitRect::new(close_x - 8.0, close_y - 8.0, 40.0, 40.0),
            HitTarget::CloseWindow {
                id: card.id.clone(),
            },
        );
    }

    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
}

#[allow(clippy::too_many_lines)]
fn build_shade(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    push_glass_backdrop(&mut out.draw_list, width, height, palette, progress);
    if let Some(menu) = ctx.power_menu.filter(|menu| *menu != PowerMenu::Closed) {
        draw_power_menu(out, palette, width, height, menu);
        return;
    }
    out.push_hit(
        HitRect::new(0.0, 0.0, width, height),
        HitTarget::DismissOverlay,
    );

    let panel_w = (width * 0.62).clamp(360.0, 680.0);
    let panel_h = (height * 0.62).clamp(440.0, 476.0);
    let panel_x = (width - panel_w) * 0.5;
    let panel_y = 32.0 - (1.0 - progress) * (panel_h * 0.5);

    out.draw_list.push_rect(
        RoundedRect::filled(
            panel_x,
            panel_y,
            panel_w,
            panel_h,
            32.0,
            palette.surface.with_alpha(0.86 * progress),
        )
        .with_glass_border(
            palette.foreground.with_alpha(0.18 * progress),
            1.5,
            0.14 * progress,
        )
        .with_shadow(28.0, 14.0, 0.45 * progress),
    );

    out.draw_list.push_text(
        panel_x + 32.0,
        panel_y + 28.0,
        &ctx.clock_text,
        3,
        palette.foreground.with_alpha(progress),
        TextAlign::Left,
    );
    out.draw_list.push_text(
        panel_x + panel_w - 32.0,
        panel_y + 32.0,
        format!("Battery {}%", ctx.battery_pct),
        2,
        palette.accent.with_alpha(progress),
        TextAlign::Right,
    );

    let tiles = [
        ("wifi", "Wi-Fi", ctx.wifi_connected),
        ("bluetooth", "Bluetooth", ctx.bluetooth_enabled),
        ("lte", "LTE Modem", ctx.lte_connected),
        ("autorotate", "Auto-Rotate", !ctx.rotation_locked),
        ("tablet_mode", "Tablet Mode", ctx.tablet_mode),
        ("night_light", "Night Light", ctx.night_light),
    ];

    let inner_x = panel_x + 28.0;
    let inner_w = panel_w - 56.0;
    let cols = 3_usize;
    let gap = 14.0;
    let tile_w = (inner_w - gap * (cols - 1) as f32) / cols as f32;
    let tile_h = 84.0;
    let grid_y = panel_y + 84.0;

    for (idx, (id, label, active)) in tiles.iter().enumerate() {
        let is_power = *id == "tablet_mode" && ctx.power_menu.is_some();
        let active = *active && !is_power;
        let label = if is_power { "Power" } else { *label };
        let col = idx % cols;
        let row = idx / cols;
        let tx = inner_x + col as f32 * (tile_w + gap);
        let ty = grid_y + row as f32 * (tile_h + gap);

        let fill = if active {
            palette.accent.with_alpha(0.88 * progress)
        } else {
            palette.background.with_alpha(0.68 * progress)
        };
        let fg = if active {
            palette.background.with_alpha(progress)
        } else {
            palette.foreground.with_alpha(progress)
        };
        let border = if active {
            Rgba::from_rgb_u8(255, 255, 255).with_alpha(0.35 * progress)
        } else {
            palette.foreground.with_alpha(0.12 * progress)
        };

        out.draw_list.push_rect(
            RoundedRect::filled(tx, ty, tile_w, tile_h, 20.0, fill)
                .with_glass_border(border, 1.2, 0.14 * progress)
                .with_shadow(10.0, 4.0, 0.25 * progress),
        );
        out.draw_list.push_text(
            tx + tile_w * 0.5,
            ty + 24.0,
            label,
            2,
            fg,
            TextAlign::Center,
        );
        out.draw_list.push_text(
            tx + tile_w * 0.5,
            ty + 48.0,
            if is_power {
                "Session"
            } else if active {
                "On"
            } else {
                "Off"
            },
            2,
            fg.with_alpha(0.82 * progress),
            TextAlign::Center,
        );

        out.push_hit(
            HitRect::new(tx, ty, tile_w, tile_h),
            if is_power {
                HitTarget::Power(PowerInput::Open)
            } else {
                HitTarget::ShadeToggle { id }
            },
        );
    }

    let slider_start_y = grid_y + 2.0 * (tile_h + gap) + 22.0;
    push_slider(
        out,
        inner_x,
        slider_start_y,
        inner_w,
        "brightness",
        "Brightness",
        ctx.brightness_pct,
        palette,
        progress,
    );
    push_slider(
        out,
        inner_x,
        slider_start_y + 72.0,
        inner_w,
        "volume",
        "Volume",
        ctx.volume_pct,
        palette,
        progress,
    );

    let handle_w = 72.0;
    out.draw_list.push_rect(RoundedRect::filled(
        panel_x + (panel_w - handle_w) * 0.5,
        panel_y + panel_h - 18.0,
        handle_w,
        5.0,
        2.5,
        palette.muted.with_alpha(0.75 * progress),
    ));

    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
}

#[allow(clippy::too_many_arguments)]
fn push_slider(
    out: &mut SceneOutput,
    x: f32,
    y: f32,
    w: f32,
    id: &'static str,
    label: &str,
    pct: u8,
    palette: &ThemePalette,
    progress: f32,
) {
    let pct_clamped = pct.min(100);
    let h = 48.0;
    out.draw_list.push_rect(
        RoundedRect::filled(
            x,
            y,
            w,
            h,
            24.0,
            palette.background.with_alpha(0.72 * progress),
        )
        .with_glass_border(
            palette.foreground.with_alpha(0.12 * progress),
            1.0,
            0.06 * progress,
        ),
    );
    let fill_w = (w * f32::from(pct_clamped) / 100.0).max(h);
    out.draw_list.push_rect(
        RoundedRect::filled(
            x,
            y,
            fill_w,
            h,
            24.0,
            palette.accent.with_alpha(0.84 * progress),
        )
        .with_glass_border(
            Rgba::from_rgb_u8(255, 255, 255).with_alpha(0.25 * progress),
            1.0,
            0.14 * progress,
        ),
    );
    out.draw_list.push_text(
        x + 20.0,
        y + 17.0,
        label,
        2,
        palette.background.with_alpha(progress),
        TextAlign::Left,
    );
    out.draw_list.push_text(
        x + w - 20.0,
        y + 17.0,
        format!("{pct_clamped}%"),
        2,
        palette.foreground.with_alpha(progress),
        TextAlign::Right,
    );

    let rect = HitRect::new(x, y, w, h);
    out.push_hit(rect, HitTarget::ShadeSlider { id, rect });
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
    let pad = 12.0;
    let gap = 8.0;
    let row_count = rows.len().max(1);
    let row_h = ((height - pad * 2.0 - gap * (row_count.saturating_sub(1)) as f32)
        / row_count as f32)
        .max(28.0);

    for (r_idx, row) in rows.iter().enumerate() {
        let y = pad + r_idx as f32 * (row_h + gap);
        let total_units: f32 = row.iter().map(|k| k.width_units).sum();
        let avail_w = width - pad * 2.0 - gap * (row.len().saturating_sub(1)) as f32;
        let unit_w = avail_w / total_units.max(1.0);
        let mut cursor_x = pad;

        for key in row {
            let kw = unit_w * key.width_units;
            let active_mod = (key.id == "shf" && ctx.osk_state.shift != ShiftState::Off)
                || (key.id == "ctl" && ctx.osk_state.ctrl);
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
            out.draw_list.push_text(
                cursor_x + kw * 0.5,
                y + (row_h - 14.0) * 0.5,
                key.label,
                2,
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
