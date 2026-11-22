//! Fullscreen mobile control center with connectivity capsules and vertical levels.

use veilshell_core::{Rgba, ThemePalette};

use crate::power::{draw_power_menu, PowerInput, PowerMenu};
use crate::scene::push_bottom_pill_bar;
use crate::shade_glyphs::{draw, Glyph};
use crate::{font, HitRect, HitTarget, RoundedRect, SceneContext, SceneOutput, TextAlign};

const REFERENCE_WIDTH: f32 = 1220.0 / 3.0;
const GUTTER: f32 = 14.0;
const CAPSULE: f32 = 62.0;

/// Host capabilities and state for the mobile control center.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ShadeStatus {
    pub date: String,
    pub wifi_radio: Option<bool>,
    pub wifi_name: Option<String>,
    pub bluetooth_available: bool,
    pub mobile_radio: Option<bool>,
    pub brightness_available: bool,
    pub volume_available: bool,
    pub power_profile: Option<String>,
    pub quiet: Option<bool>,
    pub feedback: Option<String>,
}

impl Default for ShadeStatus {
    fn default() -> Self {
        Self {
            date: "Saturday, November 21".into(),
            wifi_radio: Some(true),
            wifi_name: Some("Home network".into()),
            bluetooth_available: true,
            mobile_radio: Some(true),
            brightness_available: true,
            volume_available: true,
            power_profile: Some("Balanced".into()),
            quiet: Some(false),
            feedback: None,
        }
    }
}

/// Normalize the top-edge pull separately from the short content translation.
#[must_use]
pub fn shade_reveal_travel(height: f32) -> f32 {
    (height * 0.4).clamp(150.0, 380.0)
}

pub(crate) fn build_shade(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    let frost = (progress / 0.33).min(1.0);
    out.draw_list.push_backdrop(
        palette.background.with_alpha(0.90 * frost),
        palette.background.with_alpha(0.96 * frost),
    );
    if let Some(menu) = ctx.power_menu.filter(|menu| *menu != PowerMenu::Closed) {
        draw_power_menu(out, palette, width, height, menu);
        return;
    }
    out.push_hit(
        HitRect::new(0.0, 0.0, width, height),
        HitTarget::DismissOverlay,
    );
    let layout = ShadeLayout::new(width, height, progress, palette);
    layout.draw_header(out, ctx);
    layout.draw_controls(out, ctx);
    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
    if progress < 1.0 {
        out.hit_regions.clear();
    }
}

struct ShadeLayout<'a> {
    palette: &'a ThemePalette,
    progress: f32,
    width: f32,
    height: f32,
    landscape: bool,
    x: f32,
    grid_w: f32,
    y: f32,
    grid_y: f32,
    column: f32,
    gap: f32,
    capsule: f32,
}

impl<'a> ShadeLayout<'a> {
    fn new(width: f32, height: f32, progress: f32, palette: &'a ThemePalette) -> Self {
        let landscape = width > height;
        let scale = (width.min(height) / REFERENCE_WIDTH).max(0.35);
        let scale = if landscape { scale.min(1.25) } else { scale };
        let pane_w = if landscape {
            (REFERENCE_WIDTH * 1.5 * scale).min(width * 0.5)
        } else {
            width
        };
        let inset = (28.0 * scale).min(pane_w * 0.1);
        let grid_w = pane_w - inset * 2.0;
        let y = -(1.0 - (progress / 0.86).min(1.0)) * 34.0 * scale;
        let grid_top = (height * 0.30).clamp(70.0, 166.0);
        let gap = (GUTTER * scale).min(grid_w * 0.045).min(height * 0.034);
        let available = (height - grid_top - 50.0_f32.min(height * 0.13)).max(8.0);
        let nominal_h = CAPSULE * scale * if landscape { 1.5 } else { 1.0 };
        let capsule = nominal_h.min((available - gap * 3.0) / 4.0).max(2.0);
        Self {
            palette,
            progress,
            width,
            height,
            landscape,
            x: width - pane_w + inset,
            grid_w,
            y,
            grid_y: grid_top + y,
            column: (grid_w - gap * 3.0) * 0.25,
            gap,
            capsule,
        }
    }

    fn cell(&self, column: u8, row: u8, wide: u8, tall: u8) -> HitRect {
        HitRect::new(
            self.x + f32::from(column) * (self.column + self.gap),
            self.grid_y + f32::from(row) * (self.capsule + self.gap),
            self.column * f32::from(wide) + self.gap * f32::from(wide - 1),
            self.capsule * f32::from(tall) + self.gap * f32::from(tall - 1),
        )
    }

    fn draw_header(&self, out: &mut SceneOutput, ctx: &SceneContext) {
        let clock_scale = if self.height >= 480.0 { 7 } else { 4 };
        let date_scale = if self.height >= 480.0 { 3 } else { 1 };
        let text = self.palette.foreground.with_alpha(self.progress);
        let muted = self.palette.muted.with_alpha(self.progress);
        let y = self.y + 28.0_f32.min(self.height * 0.07);
        let end = self.x + self.grid_w;
        out.draw_list.push_text(
            self.x,
            y,
            &ctx.clock_text,
            clock_scale,
            text,
            TextAlign::Left,
        );
        out.draw_list.push_text(
            self.x,
            y + 7.5 * clock_scale as f32 + 10.0,
            font::fit_text(&ctx.shade.date, date_scale, self.grid_w),
            date_scale,
            muted,
            TextAlign::Left,
        );
        draw(
            &mut out.draw_list,
            Glyph::Battery,
            end - 53.0,
            y + 3.0,
            22.0,
            text,
        );
        out.draw_list.push_text(
            end,
            y,
            format!("{}%", ctx.battery_pct),
            1,
            text,
            TextAlign::Right,
        );
        if ctx.power_menu.is_some() {
            let size = 44.0_f32.min(self.grid_w * 0.16);
            let rect = HitRect::new(end - size, y + 24.0, size, size);
            self.surface(out, rect, false, true, size * 0.5);
            draw(
                &mut out.draw_list,
                Glyph::Power,
                rect.x + size * 0.5,
                rect.y + size * 0.5,
                23.0,
                text,
            );
            out.push_hit(rect, HitTarget::Power(PowerInput::Open));
        }
        if self.landscape {
            let center = (self.width - self.grid_w) * 0.25;
            let y = self.height * 0.43 + self.y;
            draw(&mut out.draw_list, Glyph::Bell, center, y, 30.0, muted);
            out.draw_list.push_text(
                center,
                y + 28.0,
                "Notifications are handled by your desktop",
                1,
                muted,
                TextAlign::Center,
            );
        }
        if let Some(feedback) = &ctx.shade.feedback {
            out.draw_list.push_text(
                end,
                self.height - 42.0 + self.y,
                font::fit_text(feedback, 1, self.grid_w),
                1,
                self.palette.error.with_alpha(self.progress),
                TextAlign::Right,
            );
        }
    }

    fn draw_controls(&self, out: &mut SceneOutput, ctx: &SceneContext) {
        let status = &ctx.shade;
        let wifi_detail = status
            .wifi_name
            .as_deref()
            .filter(|_| status.wifi_radio == Some(true))
            .unwrap_or_else(|| radio_label(status.wifi_radio));
        self.tile(
            out,
            self.cell(0, 0, 2, 1),
            Tile::new("wifi", Glyph::Wifi, "Wi-Fi", status.wifi_radio).wide(wifi_detail),
        );
        let bt = status.bluetooth_available.then_some(ctx.bluetooth_enabled);
        self.tile(
            out,
            self.cell(2, 0, 2, 1),
            Tile::new("bluetooth", Glyph::Bluetooth, "Bluetooth", bt).wide(radio_label(bt)),
        );
        self.slider(
            out,
            self.cell(0, 1, 1, 2),
            "brightness",
            Glyph::Sun,
            ctx.brightness_pct,
            status.brightness_available,
        );
        self.slider(
            out,
            self.cell(1, 1, 1, 2),
            "volume",
            Glyph::Volume,
            ctx.volume_pct,
            status.volume_available,
        );
        if let Some(mobile) = status.mobile_radio {
            let label = if ctx.lte_connected {
                "Connected"
            } else {
                radio_label(Some(mobile))
            };
            self.tile(
                out,
                self.cell(2, 1, 2, 1),
                Tile::new("lte", Glyph::Cellular, "Mobile data", Some(mobile)).wide(label),
            );
        }
        if let Some(profile) = &status.power_profile {
            self.tile(
                out,
                self.cell(2, 2, 1, 1),
                Tile::new(
                    "profile",
                    Glyph::Profile,
                    profile,
                    Some(profile != "Balanced"),
                ),
            );
        }
        if let Some(quiet) = status.quiet {
            let column = if status.power_profile.is_some() { 3 } else { 2 };
            self.tile(
                out,
                self.cell(column, 2, 1, 1),
                Tile::new("quiet", Glyph::Bell, "Silent", Some(quiet)),
            );
        }
        self.tile(
            out,
            self.cell(0, 3, 1, 1),
            Tile::new(
                "autorotate",
                Glyph::Rotate,
                "Auto-rotate",
                Some(!ctx.rotation_locked),
            ),
        );
        self.tile(
            out,
            self.cell(1, 3, 1, 1),
            Tile::new(
                "night_light",
                Glyph::Moon,
                "Night light",
                Some(ctx.night_light),
            ),
        );
        if ctx.power_menu.is_none() {
            self.tile(
                out,
                self.cell(2, 3, 2, 1),
                Tile::new(
                    "tablet_mode",
                    Glyph::Desktop,
                    "Desktop mode",
                    Some(!ctx.tablet_mode),
                )
                .wide("Return to desktop"),
            );
        }
    }

    fn surface(
        &self,
        out: &mut SceneOutput,
        rect: HitRect,
        active: bool,
        enabled: bool,
        radius: f32,
    ) {
        let fill = if active {
            self.palette.accent
        } else {
            Rgba::from_rgb_u8(48, 51, 60)
        };
        let alpha = if enabled { 0.96 } else { 0.44 };
        out.draw_list.push_rect(
            RoundedRect::filled(
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                radius,
                fill.with_alpha(alpha * self.progress),
            )
            .with_glass_border(
                self.palette.foreground.with_alpha(0.09 * self.progress),
                1.0,
                0.0,
            ),
        );
    }

    fn tile(&self, out: &mut SceneOutput, rect: HitRect, tile: Tile<'_>) {
        let active = tile.value == Some(true);
        let enabled = tile.value.is_some();
        let radius = if tile.wide {
            rect.h * 0.5
        } else {
            24.0_f32.min(rect.w.min(rect.h) * 0.27)
        };
        self.surface(out, rect, active, enabled, radius);
        let ink = if active {
            self.palette.background
        } else {
            self.palette.foreground
        };
        let ink = ink.with_alpha(self.progress * if enabled { 1.0 } else { 0.68 });
        let glyph_size = 24.0_f32.min(rect.h * 0.38);
        let (x, y, align, width) = if tile.wide {
            let inset = 16.0_f32.min(rect.w * 0.12);
            draw(
                &mut out.draw_list,
                tile.glyph,
                rect.x + inset + glyph_size * 0.5,
                rect.y + rect.h * 0.5,
                glyph_size,
                ink,
            );
            (
                rect.x + inset + glyph_size + 9.0,
                rect.y + rect.h * if rect.h >= 44.0 { 0.28 } else { 0.42 },
                TextAlign::Left,
                rect.w - inset * 2.0 - glyph_size - 9.0,
            )
        } else {
            draw(
                &mut out.draw_list,
                tile.glyph,
                rect.x + rect.w * 0.5,
                rect.y + rect.h * 0.36,
                glyph_size,
                ink,
            );
            (
                rect.x + rect.w * 0.5,
                rect.y + rect.h * 0.68,
                TextAlign::Center,
                rect.w - 6.0,
            )
        };
        let scale = if tile.wide && rect.w >= 190.0 { 2 } else { 1 };
        if rect.h >= 26.0 {
            out.draw_list.push_text(
                x,
                y,
                font::fit_text(tile.title, scale, width),
                scale,
                ink,
                align,
            );
        }
        if rect.h >= 44.0 && !tile.detail.is_empty() {
            out.draw_list.push_text(
                x,
                y + 17.0,
                font::fit_text(tile.detail, 1, width),
                1,
                ink.with_alpha(ink.a * 0.76),
                align,
            );
        }
        if enabled {
            out.push_hit(rect, HitTarget::ShadeToggle { id: tile.id });
        }
    }

    fn slider(
        &self,
        out: &mut SceneOutput,
        rect: HitRect,
        id: &'static str,
        glyph: Glyph,
        level: u8,
        available: bool,
    ) {
        let radius = 20.0_f32.min(rect.w * 0.5);
        self.surface(out, rect, false, available, radius);
        let fill = rect.h * f32::from(level.min(100)) / 100.0;
        if available && fill >= 1.0 {
            out.draw_list.push_rect(RoundedRect::filled(
                rect.x,
                rect.y + rect.h - fill,
                rect.w,
                fill,
                radius.min(fill * 0.5),
                self.palette.accent.with_alpha(self.progress),
            ));
        }
        let color = if available && fill >= 32.0 {
            self.palette.background
        } else {
            self.palette.foreground
        };
        draw(
            &mut out.draw_list,
            glyph,
            rect.x + rect.w * 0.5,
            rect.y + rect.h - 22.0_f32.min(rect.h * 0.20),
            24.0_f32.min(rect.w * 0.55),
            color.with_alpha(self.progress),
        );
        if available {
            out.push_hit(rect, HitTarget::ShadeSlider { id, rect });
        }
    }
}

#[derive(Clone, Copy)]
struct Tile<'a> {
    id: &'static str,
    glyph: Glyph,
    title: &'a str,
    detail: &'a str,
    value: Option<bool>,
    wide: bool,
}

impl<'a> Tile<'a> {
    fn new(id: &'static str, glyph: Glyph, title: &'a str, value: Option<bool>) -> Self {
        Self {
            id,
            glyph,
            title,
            value,
            detail: "",
            wide: false,
        }
    }

    fn wide(self, detail: &'a str) -> Self {
        Self {
            detail,
            wide: true,
            ..self
        }
    }
}

fn radio_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "On",
        Some(false) => "Off",
        None => "Unavailable",
    }
}
