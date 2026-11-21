//! Glassmorphic scene layout builders and interactive hit-testing for all `VeilShell` surfaces.

use veilshell_core::{Rgba, ThemePalette};

use crate::primitives::{DrawList, HitRect, HitTarget, RoundedRect, SceneOutput, TextAlign};
use crate::SceneKind;

/// Visual application tile shown in the paged `Home` app grid.
#[derive(Clone, Debug, PartialEq)]
pub struct AppTile {
    pub badge: String,
    pub label: String,
    pub exec: String,
    pub tint: Rgba,
}

/// Active window card shown in the `Recents` deck.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentsCard {
    pub id: String,
    pub app_id: String,
    pub title: String,
    pub focused: bool,
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
            pill_drag_x: 0.0,
            pill_drag_y: 0.0,
            top_drag_y: 0.0,
            overlay_progress: 1.0,
            home_apps: default_demo_apps(),
            home_page: 0,
            recents_cards: vec![
                RecentsCard {
                    id: "0x1".into(),
                    app_id: "FOOT".into(),
                    title: "TERMINAL".into(),
                    focused: true,
                },
                RecentsCard {
                    id: "0x2".into(),
                    app_id: "CHROMIUM".into(),
                    title: "VEILSHELL".into(),
                    focused: false,
                },
                RecentsCard {
                    id: "0x3".into(),
                    app_id: "OBSIDIAN".into(),
                    title: "SURFACE NOTES".into(),
                    focused: false,
                },
            ],
        }
    }
}

/// Built-in 12 placeholder tiles used when no `.desktop` apps are injected.
#[must_use]
pub fn default_demo_apps() -> Vec<AppTile> {
    [
        ("TERM", "TERMINAL", "foot", (122, 162, 247)),
        ("WEB", "BROWSER", "chromium", (166, 161, 231)),
        ("DIR", "FILES", "nautilus", (168, 201, 99)),
        ("NOTE", "NOTES", "obsidian", (231, 174, 109)),
        ("CONF", "SETTINGS", "systemsettings", (173, 147, 234)),
        ("MUS", "MUSIC", "spotify", (245, 123, 156)),
        ("CAL", "CALENDAR", "gnome-calendar", (63, 177, 186)),
        ("MAIL", "MAIL", "thunderbird", (245, 154, 118)),
        ("IMG", "PHOTOS", "loupe", (133, 185, 255)),
        ("CODE", "EDITOR", "nvim", (192, 226, 124)),
        ("SYS", "MONITOR", "btop", (255, 142, 171)),
        ("APP", "PACKAGES", "pacman", (189, 162, 251)),
    ]
    .into_iter()
    .map(|(badge, label, exec, (r, g, b))| AppTile {
        badge: badge.to_owned(),
        label: label.to_owned(),
        exec: exec.to_owned(),
        tint: Rgba::from_rgb_u8(r, g, b),
    })
    .collect()
}

/// Number of application tiles rendered per page on `Home`.
pub const HOME_APPS_PER_PAGE: usize = 12;

/// Compute total page count (`>= 1`) for the given app slice.
#[must_use]
pub fn home_page_count(apps: &[AppTile]) -> usize {
    apps.len().div_ceil(HOME_APPS_PER_PAGE).max(1)
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
        SceneKind::Osk => build_osk(&mut out, width, height, palette),
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

fn truncate_ascii_label(raw: &str, max_chars: usize) -> String {
    let upper = raw.trim().to_ascii_uppercase();
    let count = upper.chars().count();
    if count <= max_chars || max_chars < 4 {
        return upper;
    }
    let mut out: String = upper.chars().take(max_chars - 2).collect();
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
    let pill_w = (width * 0.20).clamp(110.0, 240.0);
    let pill_h = 6.0_f32;
    let cx = width * 0.5 + ctx.pill_drag_x.clamp(-60.0, 60.0);
    let cy = (height - 12.0 + ctx.pill_drag_y.clamp(-14.0, 4.0)).max(6.0);
    let active = ctx.pill_drag_x.abs() > 2.0 || ctx.pill_drag_y.abs() > 2.0;

    let fill = if active {
        palette.accent.with_alpha(0.95)
    } else {
        palette.foreground.with_alpha(0.84)
    };
    list.push_rect(
        RoundedRect::filled(
            cx - pill_w * 0.5,
            cy - pill_h * 0.5,
            pill_w,
            pill_h,
            pill_h * 0.5,
            fill,
        )
        .with_glass_border(Rgba::from_rgb_u8(255, 255, 255).with_alpha(0.32), 1.0, 0.20)
        .with_shadow(6.0, 2.0, 0.35),
    );
}

fn build_pill(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let tray_w = (width * 0.26).clamp(150.0, 290.0);
    let tray_h = (height - 6.0).clamp(16.0, 24.0);
    let tray_x = (width - tray_w) * 0.5;
    let tray_y = (height - tray_h) * 0.5;
    out.draw_list.push_rect(
        RoundedRect::filled(
            tray_x,
            tray_y,
            tray_w,
            tray_h,
            tray_h * 0.5,
            palette.surface.with_alpha(0.58),
        )
        .with_glass_border(palette.foreground.with_alpha(0.16), 1.0, 0.10),
    );
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
        "VEILSHELL",
        2,
        palette.accent.with_alpha(alpha),
        TextAlign::Center,
    );

    let net = match (ctx.wifi_connected, ctx.lte_connected) {
        (true, true) => "WIFI LTE",
        (true, false) => "WIFI",
        (false, true) => "LTE",
        (false, false) => "OFFLINE",
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

#[allow(clippy::too_many_lines)]
fn build_home(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    let y_slide = (1.0 - progress) * 44.0;
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

    let apps: Vec<AppTile> = if ctx.home_apps.is_empty() {
        default_demo_apps()
    } else {
        ctx.home_apps.clone()
    };

    let total_pages = home_page_count(&apps);
    let page = ctx.home_page.min(total_pages - 1);
    let start_idx = page * HOME_APPS_PER_PAGE;
    let end_idx = (start_idx + HOME_APPS_PER_PAGE).min(apps.len());

    let cols = if width >= height { 4_usize } else { 3_usize };
    let rows = HOME_APPS_PER_PAGE.div_ceil(cols);

    let grid_w = (width * 0.78).clamp(300.0, 920.0);
    let grid_h = (height - 170.0).max(260.0);
    let cell_w = grid_w / cols as f32;
    let cell_h = (grid_h / rows as f32).min(170.0);
    let start_x = (width - cell_w * cols as f32) * 0.5;
    let start_y = 96.0 + (grid_h - cell_h * rows as f32).max(0.0) * 0.35 + y_slide;

    for (slot_idx, app) in apps[start_idx..end_idx].iter().enumerate() {
        let global_idx = start_idx + slot_idx;
        let col = slot_idx % cols;
        let row = slot_idx / cols;
        let cx = start_x + (col as f32 + 0.5) * cell_w;
        let cy = start_y + (row as f32 + 0.44) * cell_h;
        let tile = (cell_w.min(cell_h) * 0.52).clamp(58.0, 96.0);
        let tile_x = cx - tile * 0.5;
        let tile_y = cy - tile * 0.5;

        out.draw_list.push_rect(
            RoundedRect::filled(
                tile_x,
                tile_y,
                tile,
                tile,
                tile * 0.28,
                mix(palette.surface, app.tint, 0.22).with_alpha(0.84 * progress),
            )
            .with_glass_border(app.tint.with_alpha(0.45 * progress), 1.5, 0.14 * progress)
            .with_shadow(16.0, 8.0, 0.32 * progress),
        );

        let badge = truncate_ascii_label(&app.badge, 4);
        let label = truncate_ascii_label(&app.label, 12);
        out.draw_list.push_text(
            cx,
            cy - 7.0,
            badge,
            2,
            palette.foreground.with_alpha(progress),
            TextAlign::Center,
        );
        out.draw_list.push_text(
            cx,
            cy + tile * 0.5 + 12.0,
            label,
            2,
            palette.foreground.with_alpha(0.90 * progress),
            TextAlign::Center,
        );

        out.push_hit(
            HitRect::new(tile_x - 10.0, tile_y - 6.0, tile + 20.0, tile + 38.0),
            HitTarget::LaunchApp { index: global_idx },
        );
    }

    // Page dots with generous touch hitboxes
    let dot_y = height - 48.0;
    let dots = total_pages.min(9);
    let span = (dots.saturating_sub(1)) as f32 * 22.0;
    for p in 0..dots {
        let dot_cx = width * 0.5 - span * 0.5 + p as f32 * 22.0;
        let active = p == page;
        let radius = if active { 5.0 } else { 3.5 };
        let col = if active {
            palette.accent.with_alpha(progress)
        } else {
            palette.muted.with_alpha(0.55 * progress)
        };
        out.draw_list.push_rect(RoundedRect::filled(
            dot_cx - radius,
            dot_y - radius,
            radius * 2.0,
            radius * 2.0,
            radius,
            col,
        ));
        out.push_hit(
            HitRect::new(dot_cx - 11.0, dot_y - 14.0, 22.0, 28.0),
            HitTarget::HomePage { page: p },
        );
    }

    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
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
            "NO OPEN WINDOWS",
            2,
            palette.foreground.with_alpha(progress),
            TextAlign::Center,
        );
        out.draw_list.push_text(
            width * 0.5,
            card_y + 102.0,
            "TAP BACKDROP TO RETURN",
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
            truncate_ascii_label(&card.app_id, 14),
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
        out.draw_list.push_rect(
            RoundedRect::filled(
                card_x + 18.0,
                card_y + 68.0,
                card_w - 36.0,
                card_h - 118.0,
                16.0,
                palette.background.with_alpha(0.75 * progress),
            )
            .with_glass_border(
                palette.foreground.with_alpha(0.08 * progress),
                1.0,
                0.04 * progress,
            ),
        );
        out.draw_list.push_text(
            card_x + card_w * 0.5,
            card_y + card_h * 0.5,
            truncate_ascii_label(&card.title, 18),
            2,
            palette.muted.with_alpha(progress),
            TextAlign::Center,
        );
        out.draw_list.push_text(
            card_x + card_w * 0.5,
            card_y + card_h - 30.0,
            if card.focused {
                "ACTIVE"
            } else {
                "TAP TO FOCUS"
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
        format!("BATTERY {}%", ctx.battery_pct),
        2,
        palette.accent.with_alpha(progress),
        TextAlign::Right,
    );

    let tiles = [
        ("wifi", "WI-FI", ctx.wifi_connected),
        ("bluetooth", "BLUETOOTH", ctx.bluetooth_enabled),
        ("lte", "LTE MODEM", ctx.lte_connected),
        ("autorotate", "AUTO-ROTATE", !ctx.rotation_locked),
        ("tablet_mode", "TABLET MODE", ctx.tablet_mode),
        ("night_light", "NIGHT LIGHT", ctx.night_light),
    ];

    let inner_x = panel_x + 28.0;
    let inner_w = panel_w - 56.0;
    let cols = 3_usize;
    let gap = 14.0;
    let tile_w = (inner_w - gap * (cols - 1) as f32) / cols as f32;
    let tile_h = 84.0;
    let grid_y = panel_y + 84.0;

    for (idx, (id, label, active)) in tiles.iter().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let tx = inner_x + col as f32 * (tile_w + gap);
        let ty = grid_y + row as f32 * (tile_h + gap);

        let fill = if *active {
            palette.accent.with_alpha(0.88 * progress)
        } else {
            palette.background.with_alpha(0.68 * progress)
        };
        let fg = if *active {
            palette.background.with_alpha(progress)
        } else {
            palette.foreground.with_alpha(progress)
        };
        let border = if *active {
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
            *label,
            2,
            fg,
            TextAlign::Center,
        );
        out.draw_list.push_text(
            tx + tile_w * 0.5,
            ty + 48.0,
            if *active { "ON" } else { "OFF" },
            2,
            fg.with_alpha(0.82 * progress),
            TextAlign::Center,
        );

        out.push_hit(
            HitRect::new(tx, ty, tile_w, tile_h),
            HitTarget::ShadeToggle { id },
        );
    }

    let slider_start_y = grid_y + 2.0 * (tile_h + gap) + 22.0;
    push_slider(
        out,
        inner_x,
        slider_start_y,
        inner_w,
        "brightness",
        "BRIGHTNESS",
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
        "VOLUME",
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

fn build_osk(out: &mut SceneOutput, width: f32, height: f32, palette: &ThemePalette) {
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

    let rows: [&[&str]; 4] = [
        &["Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P"],
        &["A", "S", "D", "F", "G", "H", "J", "K", "L"],
        &["SHF", "Z", "X", "C", "V", "B", "N", "M", "DEL"],
        &["123", "SPACE", "ENTER"],
    ];

    let pad = 12.0;
    let gap = 8.0;
    let row_h = ((height - pad * 2.0 - gap * 3.0) / 4.0).max(28.0);

    for (r_idx, row) in rows.iter().enumerate() {
        let y = pad + r_idx as f32 * (row_h + gap);
        let total_units: f32 = row
            .iter()
            .map(|k| match *k {
                "SPACE" => 5.0,
                "ENTER" | "123" | "SHF" | "DEL" => 1.5,
                _ => 1.0,
            })
            .sum();
        let avail_w = width - pad * 2.0 - gap * (row.len().saturating_sub(1)) as f32;
        let unit_w = avail_w / total_units;
        let mut cursor_x = pad;

        for key in *row {
            let units = match *key {
                "SPACE" => 5.0,
                "ENTER" | "123" | "SHF" | "DEL" => 1.5,
                _ => 1.0,
            };
            let kw = unit_w * units;
            let is_accent = *key == "ENTER";
            let fill = if is_accent {
                palette.accent.with_alpha(0.90)
            } else {
                palette.background.with_alpha(0.78)
            };
            let fg = if is_accent {
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
                *key,
                2,
                fg,
                TextAlign::Center,
            );
            out.push_hit(
                HitRect::new(cursor_x, y, kw, row_h),
                HitTarget::OskKey { label: key },
            );
            cursor_x += kw + gap;
        }
    }
}
