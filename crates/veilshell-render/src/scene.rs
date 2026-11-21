//! Glassmorphic scene layout builders for all five `VeilShell` surfaces.

use veilshell_core::{Rgba, ThemePalette};

use crate::primitives::{DrawList, RoundedRect, TextAlign};
use crate::SceneKind;

/// Dynamic data injected into scene layout builders (live windows, hardware state, gestures).
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
    pub brightness_pct: u8,
    pub volume_pct: u8,
    /// Horizontal drag indicator in logical pixels for live gesture feedback.
    pub pill_drag_x: f32,
    /// Vertical drag indicator in logical pixels (negative = upward pull).
    pub pill_drag_y: f32,
    /// Window titles shown in the `Recents` deck.
    pub recents_titles: Vec<(String, String, bool)>,
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
            brightness_pct: 72,
            volume_pct: 55,
            pill_drag_x: 0.0,
            pill_drag_y: 0.0,
            recents_titles: vec![
                ("FOOT".into(), "TERMINAL".into(), true),
                ("CHROMIUM".into(), "VEILSHELL".into(), false),
                ("OBSIDIAN".into(), "SURFACE NOTES".into(), false),
            ],
        }
    }
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
    let mut list = DrawList::default();
    match scene {
        SceneKind::Pill => build_pill(&mut list, width, height, palette, ctx),
        SceneKind::Home => build_home(&mut list, width, height, palette, ctx),
        SceneKind::Recents => build_recents(&mut list, width, height, palette, ctx),
        SceneKind::Shade => build_shade(&mut list, width, height, palette, ctx),
        SceneKind::Osk => build_osk(&mut list, width, height, palette),
    }
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

fn push_glass_backdrop(list: &mut DrawList, width: f32, height: f32, palette: &ThemePalette) {
    let top = mix(palette.background, palette.accent, 0.08).with_alpha(0.94);
    let bottom = mix(palette.background, Rgba::from_rgb_u8(4, 6, 12), 0.35).with_alpha(0.96);
    list.push_backdrop(top, bottom);
    list.push_glow(
        width * 0.22,
        height * 0.18,
        width.max(height) * 0.48,
        palette.accent.with_alpha(0.14),
    );
    list.push_glow(
        width * 0.82,
        height * 0.82,
        width.max(height) * 0.42,
        palette.accent.with_alpha(0.09),
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
    list: &mut DrawList,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    // Transparent surface background so the pill floats cleanly over active apps.
    let tray_w = (width * 0.26).clamp(150.0, 290.0);
    let tray_h = (height - 6.0).clamp(16.0, 24.0);
    let tray_x = (width - tray_w) * 0.5;
    let tray_y = (height - tray_h) * 0.5;
    list.push_rect(
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
    push_bottom_pill_bar(list, width, height, palette, ctx);
}

fn build_status_bar(list: &mut DrawList, width: f32, palette: &ThemePalette, ctx: &SceneContext) {
    let bar_w = (width - 48.0).clamp(240.0, 1160.0);
    let bar_h = 44.0;
    let bar_x = (width - bar_w) * 0.5;
    let bar_y = 20.0;

    list.push_rect(
        RoundedRect::filled(
            bar_x,
            bar_y,
            bar_w,
            bar_h,
            22.0,
            palette.surface.with_alpha(0.76),
        )
        .with_glass_border(palette.foreground.with_alpha(0.14), 1.0, 0.08)
        .with_shadow(12.0, 4.0, 0.28),
    );

    list.push_text(
        bar_x + 22.0,
        bar_y + 15.0,
        &ctx.clock_text,
        2,
        palette.foreground,
        TextAlign::Left,
    );
    list.push_text(
        width * 0.5,
        bar_y + 15.0,
        "VEILSHELL",
        2,
        palette.accent,
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
        palette.foreground,
        TextAlign::Right,
    );
}

fn build_home(
    list: &mut DrawList,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    push_glass_backdrop(list, width, height, palette);
    build_status_bar(list, width, palette, ctx);

    let apps = [
        ("TERM", "TERMINAL", Rgba::from_rgb_u8(122, 162, 247)),
        ("WEB", "BROWSER", Rgba::from_rgb_u8(166, 161, 231)),
        ("DIR", "FILES", Rgba::from_rgb_u8(168, 201, 99)),
        ("NOTE", "NOTES", Rgba::from_rgb_u8(231, 174, 109)),
        ("CONF", "SETTINGS", Rgba::from_rgb_u8(173, 147, 234)),
        ("MUS", "MUSIC", Rgba::from_rgb_u8(245, 123, 156)),
        ("CAL", "CALENDAR", Rgba::from_rgb_u8(63, 177, 186)),
        ("MAIL", "MAIL", Rgba::from_rgb_u8(245, 154, 118)),
        ("IMG", "PHOTOS", Rgba::from_rgb_u8(133, 185, 255)),
        ("CODE", "EDITOR", Rgba::from_rgb_u8(192, 226, 124)),
        ("SYS", "MONITOR", Rgba::from_rgb_u8(255, 142, 171)),
        ("APP", "PACKAGES", Rgba::from_rgb_u8(189, 162, 251)),
    ];

    let cols = if width >= height { 4_usize } else { 3_usize };
    let rows = apps.len().div_ceil(cols);

    let grid_w = (width * 0.78).clamp(300.0, 920.0);
    let grid_h = (height - 170.0).max(260.0);
    let cell_w = grid_w / cols as f32;
    let cell_h = (grid_h / rows as f32).min(170.0);
    let start_x = (width - cell_w * cols as f32) * 0.5;
    let start_y = 96.0 + (grid_h - cell_h * rows as f32).max(0.0) * 0.35;

    for (idx, (badge, label, tint)) in apps.iter().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let cx = start_x + (col as f32 + 0.5) * cell_w;
        let cy = start_y + (row as f32 + 0.44) * cell_h;
        let tile = (cell_w.min(cell_h) * 0.52).clamp(58.0, 96.0);

        // Glass squircle tile
        list.push_rect(
            RoundedRect::filled(
                cx - tile * 0.5,
                cy - tile * 0.5,
                tile,
                tile,
                tile * 0.28,
                mix(palette.surface, *tint, 0.22).with_alpha(0.84),
            )
            .with_glass_border(tint.with_alpha(0.45), 1.5, 0.14)
            .with_shadow(16.0, 8.0, 0.32),
        );

        // Inner badge text
        list.push_text(
            cx,
            cy - 7.0,
            *badge,
            2,
            palette.foreground,
            TextAlign::Center,
        );
        // App label beneath tile
        list.push_text(
            cx,
            cy + tile * 0.5 + 12.0,
            *label,
            2,
            palette.foreground.with_alpha(0.90),
            TextAlign::Center,
        );
    }

    // Page dots
    let dot_y = height - 48.0;
    for i in 0..3 {
        let dx = (i as f32 - 1.0) * 18.0;
        let active = i == 0;
        let r = if active { 5.0 } else { 3.5 };
        let col = if active {
            palette.accent
        } else {
            palette.muted.with_alpha(0.55)
        };
        list.push_rect(RoundedRect::filled(
            width * 0.5 + dx - r,
            dot_y - r,
            r * 2.0,
            r * 2.0,
            r,
            col,
        ));
    }

    push_bottom_pill_bar(list, width, height, palette, ctx);
}

fn build_recents(
    list: &mut DrawList,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    push_glass_backdrop(list, width, height, palette);
    build_status_bar(list, width, palette, ctx);

    let cards = if ctx.recents_titles.is_empty() {
        vec![("NO APPS".into(), "OPEN AN APP FROM HOME".into(), false)]
    } else {
        ctx.recents_titles.clone()
    };

    let count = cards.len().min(3);
    let card_w = (width * 0.30).clamp(240.0, 380.0);
    let card_h = (height * 0.58).clamp(260.0, 480.0);
    let gap = 28.0;
    let total_w = card_w * count as f32 + gap * count.saturating_sub(1) as f32;
    let start_x = (width - total_w) * 0.5;
    let card_y = (height - card_h) * 0.5 + 16.0;

    for (idx, (app_id, title, focused)) in cards.iter().take(count).enumerate() {
        let x = start_x + idx as f32 * (card_w + gap);
        let y = if *focused { card_y - 12.0 } else { card_y };
        let border = if *focused {
            palette.accent.with_alpha(0.90)
        } else {
            palette.foreground.with_alpha(0.18)
        };
        let border_w = if *focused { 2.5 } else { 1.2 };

        list.push_rect(
            RoundedRect::filled(x, y, card_w, card_h, 24.0, palette.surface.with_alpha(0.86))
                .with_glass_border(border, border_w, 0.12)
                .with_shadow(22.0, 12.0, 0.40),
        );

        // Card header strip
        list.push_rect(RoundedRect::filled(
            x + 14.0,
            y + 14.0,
            card_w - 28.0,
            40.0,
            14.0,
            mix(
                palette.background,
                palette.accent,
                if *focused { 0.25 } else { 0.08 },
            )
            .with_alpha(0.85),
        ));
        list.push_text(
            x + 28.0,
            y + 27.0,
            app_id,
            2,
            palette.foreground,
            TextAlign::Left,
        );

        // Close pill badge
        list.push_rect(RoundedRect::filled(
            x + card_w - 46.0,
            y + 22.0,
            24.0,
            24.0,
            12.0,
            palette.error.with_alpha(0.82),
        ));
        list.push_text(
            x + card_w - 34.0,
            y + 28.0,
            "X",
            2,
            palette.background,
            TextAlign::Center,
        );

        // Card preview surface placeholder
        list.push_rect(
            RoundedRect::filled(
                x + 18.0,
                y + 68.0,
                card_w - 36.0,
                card_h - 118.0,
                16.0,
                palette.background.with_alpha(0.75),
            )
            .with_glass_border(palette.foreground.with_alpha(0.08), 1.0, 0.04),
        );
        list.push_text(
            x + card_w * 0.5,
            y + card_h * 0.5,
            title,
            2,
            palette.muted,
            TextAlign::Center,
        );
        list.push_text(
            x + card_w * 0.5,
            y + card_h - 30.0,
            if *focused { "ACTIVE" } else { "TAP TO FOCUS" },
            2,
            if *focused {
                palette.accent
            } else {
                palette.muted
            },
            TextAlign::Center,
        );
    }

    push_bottom_pill_bar(list, width, height, palette, ctx);
}

#[allow(clippy::too_many_lines)]
fn build_shade(
    list: &mut DrawList,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    push_glass_backdrop(list, width, height, palette);

    let panel_w = (width * 0.62).clamp(360.0, 680.0);
    let panel_h = (height * 0.62).clamp(440.0, 476.0);
    let panel_x = (width - panel_w) * 0.5;
    let panel_y = 32.0;

    // Main frosted Quick Settings sheet
    list.push_rect(
        RoundedRect::filled(
            panel_x,
            panel_y,
            panel_w,
            panel_h,
            32.0,
            palette.surface.with_alpha(0.86),
        )
        .with_glass_border(palette.foreground.with_alpha(0.18), 1.5, 0.14)
        .with_shadow(28.0, 14.0, 0.45),
    );

    // Header row inside sheet
    list.push_text(
        panel_x + 32.0,
        panel_y + 28.0,
        &ctx.clock_text,
        3,
        palette.foreground,
        TextAlign::Left,
    );
    list.push_text(
        panel_x + panel_w - 32.0,
        panel_y + 32.0,
        format!("BATTERY {}%", ctx.battery_pct),
        2,
        palette.accent,
        TextAlign::Right,
    );

    // 2x3 quick toggle tiles
    let tiles = [
        ("WI-FI", ctx.wifi_connected),
        ("BLUETOOTH", ctx.bluetooth_enabled),
        ("LTE MODEM", ctx.lte_connected),
        ("AUTO-ROTATE", !ctx.rotation_locked),
        ("TABLET MODE", ctx.tablet_mode),
        ("NIGHT LIGHT", false),
    ];

    let inner_x = panel_x + 28.0;
    let inner_w = panel_w - 56.0;
    let cols = 3_usize;
    let gap = 14.0;
    let tile_w = (inner_w - gap * (cols - 1) as f32) / cols as f32;
    let tile_h = 84.0;
    let grid_y = panel_y + 84.0;

    for (idx, (label, active)) in tiles.iter().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let tx = inner_x + col as f32 * (tile_w + gap);
        let ty = grid_y + row as f32 * (tile_h + gap);

        let fill = if *active {
            palette.accent.with_alpha(0.88)
        } else {
            palette.background.with_alpha(0.68)
        };
        let fg = if *active {
            palette.background
        } else {
            palette.foreground
        };
        let border = if *active {
            Rgba::from_rgb_u8(255, 255, 255).with_alpha(0.35)
        } else {
            palette.foreground.with_alpha(0.12)
        };

        list.push_rect(
            RoundedRect::filled(tx, ty, tile_w, tile_h, 20.0, fill)
                .with_glass_border(border, 1.2, 0.14)
                .with_shadow(10.0, 4.0, 0.25),
        );
        list.push_text(
            tx + tile_w * 0.5,
            ty + 24.0,
            *label,
            2,
            fg,
            TextAlign::Center,
        );
        list.push_text(
            tx + tile_w * 0.5,
            ty + 48.0,
            if *active { "ON" } else { "OFF" },
            2,
            fg.with_alpha(0.82),
            TextAlign::Center,
        );
    }

    // Sliders: Brightness & Volume
    let slider_start_y = grid_y + 2.0 * (tile_h + gap) + 22.0;
    push_slider(
        list,
        inner_x,
        slider_start_y,
        inner_w,
        "BRIGHTNESS",
        ctx.brightness_pct,
        palette,
    );
    push_slider(
        list,
        inner_x,
        slider_start_y + 72.0,
        inner_w,
        "VOLUME",
        ctx.volume_pct,
        palette,
    );

    // Bottom drag handle on sheet
    let handle_w = 72.0;
    list.push_rect(RoundedRect::filled(
        panel_x + (panel_w - handle_w) * 0.5,
        panel_y + panel_h - 18.0,
        handle_w,
        5.0,
        2.5,
        palette.muted.with_alpha(0.75),
    ));

    push_bottom_pill_bar(list, width, height, palette, ctx);
}

fn push_slider(
    list: &mut DrawList,
    x: f32,
    y: f32,
    w: f32,
    label: &str,
    pct: u8,
    palette: &ThemePalette,
) {
    let pct_clamped = pct.min(100);
    let h = 48.0;
    list.push_rect(
        RoundedRect::filled(x, y, w, h, 24.0, palette.background.with_alpha(0.72))
            .with_glass_border(palette.foreground.with_alpha(0.12), 1.0, 0.06),
    );
    let fill_w = (w * f32::from(pct_clamped) / 100.0).max(h);
    list.push_rect(
        RoundedRect::filled(x, y, fill_w, h, 24.0, palette.accent.with_alpha(0.84))
            .with_glass_border(Rgba::from_rgb_u8(255, 255, 255).with_alpha(0.25), 1.0, 0.14),
    );
    list.push_text(
        x + 20.0,
        y + 17.0,
        label,
        2,
        palette.background,
        TextAlign::Left,
    );
    list.push_text(
        x + w - 20.0,
        y + 17.0,
        format!("{pct_clamped}%"),
        2,
        palette.foreground,
        TextAlign::Right,
    );
}

fn build_osk(list: &mut DrawList, width: f32, height: f32, palette: &ThemePalette) {
    list.push_rect(
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

            list.push_rect(
                RoundedRect::filled(cursor_x, y, kw, row_h, 12.0, fill).with_glass_border(
                    palette.foreground.with_alpha(0.14),
                    1.0,
                    0.08,
                ),
            );
            list.push_text(
                cursor_x + kw * 0.5,
                y + (row_h - 14.0) * 0.5,
                *key,
                2,
                fg,
                TextAlign::Center,
            );
            cursor_x += kw + gap;
        }
    }
}
