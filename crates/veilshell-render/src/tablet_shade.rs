//! Touch Shade geometry and rendering for Tablet and the opt-in overlay.

use std::sync::{Arc, Mutex, OnceLock};

use veilshell_core::{Rgba, ThemePalette};

use crate::power::{draw_power_menu, PowerInput, PowerMenu};
use crate::scene::push_bottom_pill_bar;
use crate::shade::draw_tablet_details;
use crate::shade_glyphs::{draw, draw_battery, Glyph};
use crate::{
    font, DrawCommand, DrawList, HitRect, HitTarget, ImageQuad, RgbaImage, RoundedRect,
    SceneContext, SceneOutput, ShadeAction, ShadePage, TabletNotifications, TabletShadePage,
    TextAlign, TextLabel, TABLET_STATUS_HEIGHT,
};

const REFERENCE_SHORT_SIDE: f32 = 1220.0 / 3.0;
const INSET: f32 = 28.0;
const HEADER_BLOCK: f32 = 66.0 + 12.0 + 26.0;
const CAPSULE: f32 = 62.0;
const GUTTER: f32 = 14.0;
const MIN_GRID_WIDTH: f32 = CAPSULE * 4.0 + GUTTER * 3.0;
pub(super) const FROST_COMPLETE: f32 = 0.33;

/// Whether Tablet Shade uses a start/end split instead of full-width pages.
#[must_use]
pub fn tablet_shade_is_split(width: f32, height: f32) -> bool {
    width > height && height > 0.0 && width.is_finite()
}

/// Geometry scale for the Tablet Shade reference viewport (not buffer scale).
#[must_use]
pub fn tablet_shade_scale(width: f32, height: f32) -> f32 {
    if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
        return 1.0;
    }
    let scale = width.min(height) / REFERENCE_SHORT_SIDE;
    if tablet_shade_is_split(width, height) {
        scale.min(1.25)
    } else {
        scale
    }
}

pub(super) fn build_tablet_shade(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    if !(progress > 0.0 && width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
        return;
    }
    let view = ShadeViewport::new(width, height, progress);
    let status = &ctx.shade;
    let owns_status = ctx.uses_tablet_status();
    let frost = (progress / FROST_COMPLETE).min(1.0);
    // Keep the existing backdrop primitive so wgpu can use the wallpaper cache
    // and Shade-over-Home can blend into the Home draw list. This is a tint and
    // a scrim, not a per-tile blur or a sampler of the active Wayland window.
    out.draw_list.push_backdrop(
        palette.surface.with_alpha(0.87 * frost),
        palette.background.with_alpha(0.94 * frost),
    );
    out.draw_list.push_rect(RoundedRect::filled(
        0.0,
        0.0,
        width,
        height,
        0.0,
        Rgba::from_rgb_u8(0, 0, 0).with_alpha(0.20 * frost),
    ));
    let first_hit = out.hit_regions.len();
    let open = progress >= 1.0;
    if let Some(menu) = ctx.power_menu.filter(|menu| *menu != PowerMenu::Closed) {
        draw_power_menu(out, palette, width, height, menu);
    } else if owns_status && status.page != ShadePage::Main {
        if open && status.tablet_page_position.is_none() {
            out.push_hit(view.bounds, HitTarget::DismissOverlay);
        }
        draw_tablet_details(out, width, height, palette, ctx, progress);
        if height >= 20.0 {
            push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
        }
    } else {
        let interactive = open && status.tablet_page_position.is_none();
        let mut painter = TabletShade {
            out,
            view,
            palette,
            progress,
            interactive,
            // The overlay's existing service writes belong to the legacy
            // route, not the gap profile. Its tiles only show telemetry.
            control_actions: owns_status,
        };
        painter.hit(view.bounds, HitTarget::DismissOverlay);
        painter.header(ctx);
        let position = status
            .tablet_page_position
            .filter(|p| p.is_finite())
            .map_or_else(|| page_index(status.tablet_page), |p| p.clamp(0.0, 1.0));
        // The existing host notification action is not an in-Shade history
        // source. Only the standalone snapshot path can show a known-empty fixture.
        let notifications = if owns_status {
            status.tablet_notifications
        } else {
            TabletNotifications::Unavailable
        };
        for page in [TabletShadePage::Notifications, TabletShadePage::Controls] {
            let pane = view.pane(page, position);
            if painter.sees(pane) {
                match page {
                    TabletShadePage::Notifications => painter.notifications(pane, notifications),
                    TabletShadePage::Controls => painter.controls(pane, ctx),
                }
            }
        }
        if height >= 20.0 {
            push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
        }
        return;
    }
    // The shared modal renderers take priority over the pager. Suppress their
    // hits too during a partial pull, and reject any offscreen modal targets on
    // outputs too small for the shared layout.
    if open {
        // These modal layouts produce only a small number of actions and run
        // on a separate path from animated panes. Do not touch earlier hits.
        let mut index = 0;
        out.hit_regions.retain(|hit| {
            index += 1;
            index <= first_hit || fits(hit.rect, view.bounds)
        });
    } else {
        out.hit_regions.truncate(first_hit);
    }
}

fn page_index(page: TabletShadePage) -> f32 {
    match page {
        TabletShadePage::Notifications => 0.0,
        TabletShadePage::Controls => 1.0,
    }
}

#[derive(Clone, Copy)]
struct ShadeViewport {
    bounds: HitRect,
    split: bool,
    scale: f32,
    header_top: f32,
    header_fit: f32,
    slide: f32,
    translate: f32,
}

impl ShadeViewport {
    fn new(width: f32, height: f32, progress: f32) -> Self {
        let scale = tablet_shade_scale(width, height);
        let translate = (progress / 0.86).min(1.0);
        Self {
            bounds: HitRect::new(0.0, 0.0, width, height),
            split: tablet_shade_is_split(width, height),
            scale,
            header_top: 70.0 * scale, // 40 top + 18 status row + 12 gap
            header_fit: (height / 520.0).min(1.0),
            slide: -34.0 * scale * (1.0 - translate),
            translate,
        }
    }

    fn pane(self, page: TabletShadePage, position: f32) -> HitRect {
        let width = self.bounds.w;
        let (x, w) = if self.split {
            let w = (610.0 * self.scale).min(width * 0.5);
            let x = match page {
                TabletShadePage::Notifications => 0.0,
                TabletShadePage::Controls => width - w,
            };
            (x, w)
        } else {
            ((page_index(page) - position) * width, width)
        };
        HitRect::new(x, 0.0, w, self.bounds.h)
    }

    fn inset(self) -> f32 {
        INSET * self.scale
    }

    fn body_top(self) -> f32 {
        self.header_top + HEADER_BLOCK * self.header_fit + 16.0 * self.scale
    }
}

fn fits(rect: HitRect, bounds: HitRect) -> bool {
    rect.w > 0.0
        && rect.h > 0.0
        && rect.x >= bounds.x
        && rect.y >= bounds.y
        && rect.x + rect.w <= bounds.x + bounds.w
        && rect.y + rect.h <= bounds.y + bounds.h
}

fn smooth_reveal(progress: f32, start: f32, span: f32) -> f32 {
    let t = ((progress - start) / span).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

struct TabletShade<'a> {
    out: &'a mut SceneOutput,
    view: ShadeViewport,
    palette: &'a ThemePalette,
    progress: f32,
    interactive: bool,
    control_actions: bool,
}

impl TabletShade<'_> {
    fn sees(&self, rect: HitRect) -> bool {
        rect.w > 0.0
            && rect.h > 0.0
            && rect.x < self.view.bounds.w
            && rect.y < self.view.bounds.h
            && rect.x + rect.w > 0.0
            && rect.y + rect.h > 0.0
    }

    fn hit(&mut self, rect: HitRect, target: HitTarget) {
        if self.interactive && fits(rect, self.view.bounds) {
            self.out.push_hit(rect, target);
        }
    }

    fn surface(&mut self, rect: HitRect, radius: f32, active: bool, alpha: f32) {
        let fg = self.palette.foreground;
        let (fill, rim, sheen) = if active {
            (self.palette.accent.with_alpha(alpha), 0.0, 0.0)
        } else {
            (fg.with_alpha(0.065 * alpha), 0.09, 0.018)
        };
        self.out.draw_list.push_rect(
            RoundedRect::filled(rect.x, rect.y, rect.w, rect.h, radius, fill).with_glass_border(
                fg.with_alpha(rim * alpha),
                1.4,
                sheen * alpha,
            ),
        );
    }

    fn header(&mut self, ctx: &SceneContext) {
        let view = self.view;
        let t = view.translate;
        let (w, h) = (view.bounds.w, view.bounds.h);
        let owns_status = ctx.uses_tablet_status();
        // A desktop overlay has no idle row to morph from. Fade in its header
        // with the Shade so the desktop alone owns the status at rest.
        let ink_alpha = if owns_status {
            1.0
        } else {
            (self.progress / FROST_COMPLETE).min(1.0)
        };
        let closed_mid = (TABLET_STATUS_HEIGHT as f32 * 0.5).min(h * 0.16);
        let status_mid = lerp(closed_mid, 49.0 * view.scale, t);
        let right = self.status_cluster(ctx, status_mid, ink_alpha);
        let ink = self
            .palette
            .foreground
            .with_alpha(self.palette.foreground.a * ink_alpha);
        let list = &mut self.out.draw_list;
        let clock_x = lerp(16.0_f32.min(w * 0.12), view.inset(), t);
        let clock_y = lerp((closed_mid - 5.0).max(0.0), view.header_top, t);
        let time_size = lerp(13.5, 66.0 * view.header_fit, t);
        let clock_w = lerp(
            (right - 6.0 - clock_x).max(0.0),
            if view.split {
                view.pane(TabletShadePage::Notifications, 0.0).w - 2.0 * view.inset()
            } else if owns_status {
                // Reserve the trailing action row, even on the history page,
                // so the clock stays stationary during a horizontal swipe.
                w - 2.0 * view.inset() - 90.0 * view.scale
            } else {
                w - 2.0 * view.inset()
            },
            t,
        );
        // Optically compensate for the smaller cap height of our regular HUD
        // type; the clock still reserves the 66 px line of the reference.
        let optical_size = time_size + t * 10.0 * view.header_fit;
        let tier = fitting_tier(&ctx.clock_text, optical_size, clock_w);
        push_clock(
            list,
            &ctx.clock_text,
            clock_x,
            clock_y,
            clock_w,
            tier,
            ink,
            t,
        );
        let date = if ctx.home_date_text.trim().is_empty() {
            &ctx.shade.date
        } else {
            &ctx.home_date_text
        };
        let date_alpha = ((t - 0.60) / 0.40).clamp(0.0, 1.0);
        if date_alpha > 0.0 && h >= 90.0 {
            label(
                list,
                clock_x,
                clock_y + time_size + 12.0 * view.header_fit,
                date,
                fitting_tier(date, 26.0 * view.header_fit, clock_w),
                clock_w,
                self.palette.muted.with_alpha(date_alpha),
                TextAlign::Left,
            );
        }
    }

    fn status_cluster(&mut self, ctx: &SceneContext, cy: f32, ink_alpha: f32) -> f32 {
        let view = self.view;
        let (w, h) = (view.bounds.w, view.bounds.h);
        let t = view.translate;
        let size = 1.0_f32.min(w / 240.0).min(h / 190.0);
        let mut right = w - lerp(16.0_f32.min(w * 0.12), 35.0 * view.scale, t);
        let ink = self
            .palette
            .foreground
            .with_alpha(self.palette.foreground.a * ink_alpha);
        let list = &mut self.out.draw_list;
        if let Some((pct, charging)) = ctx.tablet_status.battery {
            // Translate the cluster without reordering it: changing sides
            // would carry the battery through the percentage mid-reveal.
            const PERCENT_WIDTH: f32 = 32.0;
            let battery_size = lerp(20.0, 34.0, t) * size;
            let show_percent = size >= 0.72;
            let percent_space = if show_percent {
                PERCENT_WIDTH + 7.0 * size
            } else {
                0.0
            };
            if show_percent {
                let pct_text = format!("{}%", pct.min(100));
                label(
                    list,
                    right,
                    cy - 5.0,
                    &pct_text,
                    1,
                    PERCENT_WIDTH,
                    ink,
                    TextAlign::Right,
                );
            }
            draw_battery(
                list,
                right - percent_space - battery_size * 0.5,
                cy,
                battery_size,
                pct,
                if charging {
                    self.palette
                        .accent
                        .with_alpha(self.palette.accent.a * ink_alpha)
                } else {
                    ink
                },
            );
            right -= percent_space + battery_size + 6.0 * size;
        }
        if let Some(connected) = ctx.tablet_status.wifi {
            draw(
                list,
                Glyph::Wifi,
                right - 10.0 * size,
                cy,
                lerp(18.0, 23.0, t) * size,
                ink.with_alpha(ink_alpha * if connected { 1.0 } else { 0.45 }),
            );
            right -= 22.0 * size;
        }
        right
    }

    fn header_actions(&mut self, pane: HitRect, settings: bool) {
        let alpha = smooth_reveal(self.progress, 0.40, 0.30);
        let view = self.view;
        let header_h = HEADER_BLOCK * view.header_fit;
        let size = (36.0 * view.scale).min(header_h * 0.77);
        let gap = 12.0 * view.scale;
        let top = view.header_top + (header_h - size) * 0.5 + view.slide;
        let end = pane.x + pane.w - view.inset();
        for (index, glyph, target) in [
            (0, Glyph::Power, HitTarget::Power(PowerInput::Open)),
            (
                1,
                Glyph::Settings,
                HitTarget::ShadeAction(ShadeAction::Settings),
            ),
        ] {
            if index == 1 && !settings {
                break;
            }
            let rect = HitRect::new(end - size - index as f32 * (size + gap), top, size, size);
            if alpha > 0.0 && self.sees(rect) {
                self.surface(rect, size * 0.5, false, alpha);
                draw(
                    &mut self.out.draw_list,
                    glyph,
                    rect.x + rect.w * 0.5,
                    rect.y + rect.h * 0.5,
                    20.0_f32.min(size * 0.62),
                    self.palette.foreground.with_alpha(alpha),
                );
                let grow = ((44.0 - size) * 0.5).max(0.0).min(gap * 0.45);
                self.hit(
                    HitRect::new(
                        rect.x - grow,
                        rect.y - grow,
                        size + 2.0 * grow,
                        size + 2.0 * grow,
                    ),
                    target,
                );
            }
        }
    }

    fn notifications(&mut self, pane: HitRect, notifications: TabletNotifications) {
        let view = self.view;
        let content = pane.h - view.body_top() - 4.0 * view.scale;
        let size = 32.0_f32.min(content * 0.20);
        let label_w = pane.w - 2.0 * view.inset();
        let (text, glyph) = match notifications {
            TabletNotifications::Empty => ("No notifications", Glyph::Bell),
            TabletNotifications::Unavailable => {
                ("Notification history unavailable", Glyph::BellOff)
            }
        };
        let center = pane.x + pane.w * 0.5;
        let area = HitRect::new(
            center - label_w * 0.5,
            view.body_top() + view.slide,
            label_w,
            content,
        );
        if size <= 0.0 || !self.sees(area) {
            return;
        }
        let alpha = (self.progress / 0.55).min(1.0);
        let gap = 10.0 * view.scale;
        let top = area.y + (content - size - gap - 18.0) * 0.5;
        let list = &mut self.out.draw_list;
        if center + size * 0.5 > 0.0 && center - size * 0.5 < view.bounds.w {
            draw(
                list,
                glyph,
                center,
                top + size * 0.5,
                size,
                self.palette.muted.with_alpha(alpha * 0.82),
            );
        }
        if content >= 28.0 {
            label(
                list,
                center,
                top + size + gap,
                text,
                fitting_tier(text, 18.5_f32.min(content * 0.35), label_w),
                label_w,
                self.palette.muted.with_alpha(alpha),
                TextAlign::Center,
            );
        }
    }

    fn controls(&mut self, pane: HitRect, ctx: &SceneContext) {
        // Avoid preparing any service text or level textures before the first
        // cell is revealed, even when the Controls page is on screen.
        if self.control_actions {
            self.header_actions(pane, ctx.settings.is_some());
        }
        if self.progress <= 0.33 {
            return;
        }
        let status = &ctx.shade;
        let footer = footer_tiles(ctx, self.control_actions);
        let extra_row = footer.iter().map(|(_, columns)| columns).sum::<usize>() > 4;
        let grid = Grid::new(self.view, pane, extra_row);
        for (index, tile) in connections(ctx, self.control_actions)
            .into_iter()
            .enumerate()
        {
            let (row, column) = [(0, 0), (0, 2), (1, 2)][index];
            self.tile(grid, row, column, 2, tile);
        }
        self.level(
            grid,
            0,
            "brightness",
            Glyph::Sun,
            status.brightness_available.then_some(ctx.brightness_pct),
        );
        self.level(
            grid,
            1,
            "volume",
            Glyph::Volume,
            status.volume_available.then_some(ctx.volume_pct),
        );
        if self.control_actions {
            self.session_tiles(grid, ctx);
        }

        let (mut row, mut column) = (3, 0);
        for (tile, columns) in footer {
            if column + columns > 4 {
                row += 1;
                column = 0;
            }
            self.tile(grid, row, column, columns, tile);
            column += columns;
        }
        // The gap profile exposes telemetry, not actions that change the host.
        let message = if self.control_actions {
            status
                .feedback
                .as_deref()
                .or(status.pending.map(|_| "Working..."))
        } else {
            Some("Controls are read-only")
        };
        if let Some(message) = message {
            if pane.h >= 80.0 {
                label(
                    &mut self.out.draw_list,
                    grid.x,
                    pane.h - footer_reserve(pane.h) + 3.0 + self.view.slide,
                    message,
                    1,
                    grid.width,
                    if self.control_actions && status.feedback.is_some() {
                        self.palette.error
                    } else {
                        self.palette.muted
                    }
                    .with_alpha(self.progress),
                    TextAlign::Left,
                );
            }
        }
    }

    fn session_tiles(&mut self, grid: Grid, ctx: &SceneContext) {
        let status = &ctx.shade;
        if let Some(profile) = status.power_profile.as_deref() {
            self.tile(
                grid,
                2,
                2,
                1,
                Tile::small(
                    profile,
                    Glyph::Profile,
                    profile != "Balanced",
                    Some(toggle("profile")),
                ),
            );
        }
        let quiet = status.quiet;
        let silent = Tile::small(
            "Silent",
            Glyph::BellOff,
            quiet == Some(true),
            (quiet.is_some() && status.extras.notifications && status.pending.is_none())
                .then_some(HitTarget::ShadeAction(ShadeAction::Quiet)),
        );
        self.tile(
            grid,
            2,
            if status.power_profile.is_some() { 3 } else { 2 },
            1,
            silent,
        );
    }

    fn tile(&mut self, grid: Grid, row: usize, column: usize, span: usize, tile: Tile<'_>) {
        let alpha = tile_reveal(self.progress, row, column);
        let rect = grid.cell(row, column, span);
        if alpha <= 0.0 || !self.sees(rect) {
            return;
        }
        let radius = tile_radius(rect, self.view.scale);
        self.surface(rect, radius, tile.active, alpha);
        let fg = self.palette.foreground;
        let ink = if tile.active {
            self.palette.background
        } else {
            fg
        }
        .with_alpha(alpha * if tile.target.is_some() { 1.0 } else { 0.83 });
        if span == 1 {
            let cy = rect.y + rect.h * 0.5;
            let cx = rect.x + rect.w * 0.5;
            draw(
                &mut self.out.draw_list,
                tile.glyph,
                cx,
                cy,
                30.0_f32.min(rect.w * 0.66).min(rect.h * 0.64),
                ink,
            );
            if grid.caption_gap(row) >= 7.0 {
                label(
                    &mut self.out.draw_list,
                    cx,
                    rect.y + rect.h + 1.5,
                    tile.title,
                    1,
                    rect.w,
                    fg.with_alpha(alpha * 0.91),
                    TextAlign::Center,
                );
            }
            if let Some(target) = tile.target {
                self.hit(rect, target);
            }
        } else {
            self.wide_face(rect, &tile, ink);
        }
    }

    fn wide_face(&mut self, rect: HitRect, tile: &Tile<'_>, ink: Rgba) {
        let s = self.view.scale;
        let cy = rect.y + rect.h * 0.5;
        let inset = (56.0 * s).min(rect.w * 0.36);
        let icon_x = rect.x + (31.0 * s).min(inset * 0.56);
        let end = rect.x + rect.w;
        let details_width = if tile.details.is_some() {
            (30.0 * s).max(44.0).min(rect.w * 0.38)
        } else {
            0.0
        };
        let text_width = rect.w - inset - (8.0 * s).min(rect.w * 0.05) - details_width;
        let list = &mut self.out.draw_list;
        draw(
            list,
            tile.glyph,
            icon_x,
            cy,
            24.0_f32.min(inset * 0.64).min(rect.h * 0.60),
            ink,
        );
        if rect.h >= 14.0 {
            let two_lines = rect.h >= 32.0 && !tile.subtitle.is_empty();
            label(
                list,
                rect.x + inset,
                cy - if two_lines { 14.0 } else { 6.0 },
                tile.title,
                fitting_tier(tile.title, 18.5_f32.min(rect.h * 0.40), text_width),
                text_width,
                ink,
                TextAlign::Left,
            );
            if two_lines {
                label(
                    list,
                    rect.x + inset,
                    cy + 3.0,
                    tile.subtitle,
                    1,
                    text_width,
                    ink.with_alpha(ink.a * 0.73),
                    TextAlign::Left,
                );
            }
        }
        if let Some((page, available)) = tile.details {
            let opacity = if available { ink.a } else { ink.a * 0.56 };
            draw(
                list,
                Glyph::ChevronRight,
                end - (18.0 * s).min(details_width * 0.5),
                cy,
                15.0_f32.min(rect.h * 0.50).min(details_width * 0.66),
                ink.with_alpha(opacity),
            );
            if available {
                self.hit(
                    HitRect::new(end - details_width, rect.y, details_width, rect.h),
                    HitTarget::ShadeAction(ShadeAction::Show(page)),
                );
            }
        }
        if let Some(target) = &tile.target {
            let boundary = if details_width > 0.0 { 0.01 } else { 0.0 };
            self.hit(
                HitRect::new(rect.x, rect.y, rect.w - details_width - boundary, rect.h),
                target.clone(),
            );
        }
    }

    fn level(
        &mut self,
        grid: Grid,
        column: usize,
        id: &'static str,
        glyph: Glyph,
        value: Option<u8>,
    ) {
        let alpha = tile_reveal(self.progress, 1, column);
        let rect = grid.level(column);
        if alpha <= 0.0 || !self.sees(rect) {
            return;
        }
        let radius = tile_radius(rect, self.view.scale);
        self.surface(rect, radius, false, alpha);
        let pct = (f32::from(value.unwrap_or(0).min(100)) * alpha).round() as u8;
        if pct > 0 {
            self.out.draw_list.push_image(ImageQuad {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
                radius,
                alpha,
                image: liquid_fill(self.palette.accent, pct),
            });
        }
        let icon_y = (22.0 * self.view.scale).min(rect.h * 0.28);
        let ink = if f32::from(pct) * 0.01 * rect.h > icon_y {
            self.palette.background
        } else {
            self.palette.foreground
        };
        let list = &mut self.out.draw_list;
        let cx = rect.x + rect.w * 0.5;
        draw(
            list,
            glyph,
            cx,
            rect.y + rect.h - icon_y,
            24.0_f32.min(icon_y * 1.6).min(rect.w * 0.7),
            ink.with_alpha(if value.is_some() { alpha } else { alpha * 0.4 }),
        );
        if self.control_actions && value.is_some() {
            // Use the same full-height slider mapping as the hardware path.
            self.hit(rect, HitTarget::ShadeSlider { id, rect });
        } else if value.is_none() && rect.h >= 64.0 {
            label(
                list,
                cx,
                rect.y + 13.0,
                "Unavailable",
                1,
                rect.w - 6.0,
                self.palette.muted.with_alpha(alpha * 0.79),
                TextAlign::Center,
            );
        }
    }
}

fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

fn tile_radius(rect: HitRect, scale: f32) -> f32 {
    (32.0 * scale).min(rect.w * 0.5).min(rect.h * 0.5)
}

fn tile_reveal(progress: f32, row: usize, column: usize) -> f32 {
    smooth_reveal(
        progress,
        0.33 + 0.05 * row as f32 + 0.01 * column as f32,
        0.45,
    )
}

fn footer_reserve(height: f32) -> f32 {
    36.0_f32.min(height * 0.18)
}

#[derive(Clone, Copy)]
struct Grid {
    x: f32,
    top: f32,
    width: f32,
    column: f32,
    gutter: f32,
    capsule: f32,
    v_gap: f32,
    extra_capsule: f32,
    extra_gap: f32,
}

impl Grid {
    fn new(view: ShadeViewport, pane: HitRect, extra_row: bool) -> Self {
        let scale = view.scale;
        let width = pane.w - 2.0 * view.inset();
        let unit = scale.min(width / MIN_GRID_WIDTH);
        let gutter = GUTTER * unit;
        let capsule = CAPSULE * unit * if view.split { 1.5 } else { 1.0 };
        let extra_capsule = capsule.min(50.0);
        let extra_gap = (gutter * 0.65).min(14.0);
        let desired_height = 4.0 * (capsule + gutter)
            + gutter
            + if extra_row {
                extra_capsule + extra_gap + gutter
            } else {
                0.0
            };
        let reserve = footer_reserve(pane.h);
        let available = (pane.h - reserve - view.body_top()).max(0.0);
        // The reference pane scrolls; keep controls on short viewports by
        // compressing the vertical metrics, without changing the four slots.
        let fit = (available / desired_height).min(1.0);
        Self {
            x: pane.x + view.inset(),
            top: view.body_top() + view.slide,
            width,
            column: (width - 3.0 * gutter) * 0.25,
            gutter,
            capsule: capsule * fit,
            v_gap: gutter * fit,
            extra_capsule: extra_capsule * fit,
            extra_gap: extra_gap * fit,
        }
    }

    fn cell(self, row: usize, col: usize, span: usize) -> HitRect {
        let pitch = self.capsule + self.v_gap;
        let (y, h) = match row {
            0..=2 => (row as f32 * pitch, self.capsule),
            3 => (3.0 * pitch + self.v_gap, self.capsule),
            _ => (
                4.0 * pitch + self.v_gap + self.extra_gap,
                self.extra_capsule,
            ),
        };
        HitRect::new(
            self.x + col as f32 * (self.column + self.gutter),
            self.top + y,
            span as f32 * self.column + (span - 1) as f32 * self.gutter,
            h,
        )
    }

    fn level(self, col: usize) -> HitRect {
        let rect = self.cell(1, col, 1);
        HitRect {
            h: self.capsule * 2.0 + self.v_gap,
            ..rect
        }
    }

    fn caption_gap(self, row: usize) -> f32 {
        self.v_gap * if row == 2 { 2.0 } else { 1.0 }
    }
}

struct Tile<'a> {
    title: &'a str,
    subtitle: &'a str,
    glyph: Glyph,
    active: bool,
    target: Option<HitTarget>,
    details: Option<(ShadePage, bool)>,
}

impl<'a> Tile<'a> {
    fn small(title: &'a str, glyph: Glyph, active: bool, target: Option<HitTarget>) -> Self {
        Self {
            title,
            subtitle: "",
            glyph,
            active,
            target,
            details: None,
        }
    }

    fn radio(
        title: &'a str,
        subtitle: &'a str,
        glyph: Glyph,
        value: Option<bool>,
        target: Option<HitTarget>,
        details: Option<(ShadePage, bool)>,
    ) -> Self {
        Self {
            title,
            subtitle,
            glyph,
            active: value == Some(true),
            target,
            details,
        }
    }
}

const fn toggle(id: &'static str) -> HitTarget {
    HitTarget::ShadeToggle { id }
}

fn connections(ctx: &SceneContext, control_actions: bool) -> [Tile<'_>; 3] {
    let status = &ctx.shade;
    let extra = &status.extras;
    let wifi = status.wifi_radio;
    let bluetooth = status.bluetooth_available.then_some(ctx.bluetooth_enabled);
    let mobile = status.mobile_radio;
    let ready = status.pending.is_none();
    let action =
        |id, value: Option<bool>| (control_actions && value.is_some()).then_some(toggle(id));
    let details = |page, available| control_actions.then_some((page, ready && available));
    let wifi_detail = match wifi {
        Some(true) => status
            .wifi_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(if ctx.wifi_connected {
                "Connected"
            } else {
                "On"
            }),
        Some(false) => "Off",
        None => "Network unavailable",
    };
    let bt_detail = match bluetooth {
        Some(true) => extra
            .bluetooth_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("On"),
        Some(false) => "Off",
        None => "Bluetooth unavailable",
    };
    let mobile_detail = match mobile {
        Some(true) if ctx.lte_connected => "Connected",
        Some(true) => "On",
        Some(false) => "Off",
        None => "Mobile network unavailable",
    };
    [
        Tile::radio(
            "Wi-Fi",
            wifi_detail,
            Glyph::Wifi,
            wifi,
            action("wifi", wifi),
            details(
                ShadePage::Wifi,
                wifi.is_some() && (extra.wifi || extra.host_wifi),
            ),
        ),
        Tile::radio(
            "Bluetooth",
            bt_detail,
            Glyph::Bluetooth,
            bluetooth,
            action("bluetooth", bluetooth),
            details(
                ShadePage::Bluetooth,
                bluetooth.is_some() && (extra.bluetooth || extra.host_bluetooth),
            ),
        ),
        Tile::radio(
            "Mobile data",
            mobile_detail,
            Glyph::Cellular,
            mobile,
            action("lte", mobile),
            None,
        ),
    ]
}

fn footer_tiles(ctx: &SceneContext, can_control: bool) -> Vec<(Tile<'static>, usize)> {
    let status = &ctx.shade;
    let extra = &status.extras;
    let ready = status.pending.is_none();
    let night_light = status.night_light_available.then(|| {
        (
            Tile::small(
                "Night light",
                Glyph::Moon,
                ctx.night_light,
                can_control.then_some(toggle("night_light")),
            ),
            1,
        )
    });
    if !can_control {
        return night_light.into_iter().collect();
    }
    let switch = Tile {
        title: "Desktop view",
        subtitle: "Off",
        ..Tile::small(
            "Desktop view",
            Glyph::Desktop,
            false,
            Some(toggle("desk_mode")),
        )
    };
    let action = |action| ready.then_some(HitTarget::ShadeAction(action));
    [
        status.auto_rotate_available.then(|| {
            (
                Tile::small(
                    "Auto-rotate",
                    Glyph::Rotate,
                    !ctx.rotation_locked,
                    Some(toggle("autorotate")),
                ),
                1,
            )
        }),
        Some((switch, 2)),
        night_light,
        extra.muted.map(|muted| {
            (
                Tile::small(
                    if muted { "Unmute" } else { "Mute" },
                    Glyph::Volume,
                    muted,
                    action(ShadeAction::Mute),
                ),
                1,
            )
        }),
        (status.volume_available && (extra.audio || extra.host_audio)).then(|| {
            (
                Tile::small(
                    "Audio output",
                    Glyph::Volume,
                    false,
                    action(ShadeAction::Show(ShadePage::Audio)),
                ),
                1,
            )
        }),
        extra.has_more().then(|| {
            (
                Tile::small(
                    "More",
                    Glyph::ChevronRight,
                    false,
                    action(ShadeAction::Show(ShadePage::More)),
                ),
                1,
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

// Keep type at the shell's native size rather than multiplying it by the
// viewport geometry scale. The tiers are shared by GPU, software and fallback.
fn fitting_tier(text: &str, px: f32, width: f32) -> u32 {
    let max = if px < 16.0 {
        1
    } else if px < 23.5 {
        2
    } else {
        (px / 9.5).round() as u32
    };
    (2..=max)
        .rev()
        .find(|&tier| font::fit_text(text, tier, width) == text)
        .unwrap_or(1)
}

#[allow(clippy::too_many_arguments)] // Same coordinates/ink contract as the text primitive.
fn label(
    list: &mut DrawList,
    x: f32,
    y: f32,
    text: &str,
    tier: u32,
    width: f32,
    ink: Rgba,
    align: TextAlign,
) {
    let text = font::fit_text(text, tier, width);
    if !text.is_empty() {
        list.push_text(x, y, text, tier, ink, align);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_clock(
    list: &mut DrawList,
    text: &str,
    x: f32,
    y: f32,
    width: f32,
    tier: u32,
    ink: Rgba,
    amount: f32,
) {
    let fitted = font::fit_text(text, tier, width);
    if fitted.is_empty() {
        return;
    }
    let mut clock = TextLabel {
        x,
        y,
        text: fitted,
        scale: tier,
        color: ink,
        align: TextAlign::Left,
    };
    if clock.text == text && amount > 0.0 {
        if let Some((hour, minute)) = text.split_once(':').filter(|(hour, minute)| {
            !hour.is_empty()
                && !minute.is_empty()
                && hour
                    .bytes()
                    .chain(minute.bytes())
                    .all(|c| c.is_ascii_digit())
        }) {
            let minute_index = hour.len() + 1;
            // Reuse the current font layout to get the split position without
            // approximating TrueType advances or adding a second font API.
            let minute_x = match font::layout_text_glyphs(&clock) {
                font::TextLayout::TrueType(glyphs) => glyphs
                    .get(minute_index)
                    .map_or(x, |g| (g.x - g.glyph.offset_x) as f32),
                font::TextLayout::Bitmap5x7 => {
                    x + minute_index as f32 * font::GLYPH_ADVANCE as f32 * tier as f32
                }
            };
            if minute_x > x {
                clock.text.truncate(minute_index);
                list.commands.push(DrawCommand::Text(clock));
                list.push_text(
                    minute_x,
                    y,
                    minute,
                    tier,
                    ink.with_alpha(lerp(ink.a, ink.a * 0.62, amount)),
                    TextAlign::Left,
                );
                return;
            }
        }
    }
    list.commands.push(DrawCommand::Text(clock));
}

// Only the normalized liquid surface is a texture; the capsule and clipping
// still use the renderer's analytic rounded geometry. Cache one small mask per
// integer percentage in the current accent, independent of viewport or slide.
fn liquid_fill(accent: Rgba, pct: u8) -> Arc<RgbaImage> {
    struct LiquidCache {
        rgb: [u8; 3],
        levels: [Option<Arc<RgbaImage>>; 101],
    }
    static CACHE: OnceLock<Mutex<LiquidCache>> = OnceLock::new();
    let rgb = [accent.r, accent.g, accent.b].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    let mut cache = CACHE
        .get_or_init(|| {
            Mutex::new(LiquidCache {
                rgb,
                levels: [const { None }; 101],
            })
        })
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.rgb != rgb {
        cache.rgb = rgb;
        cache.levels.fill(None);
    }
    Arc::clone(cache.levels[usize::from(pct)].get_or_insert_with(|| {
        const W: u32 = 40;
        const H: u32 = 192;
        let level = f32::from(pct) / 100.0;
        let bulge = 0.055_f32.min(level * 0.6).min((1.0 - level) * 0.6);
        let mut rgba = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H {
            for x in 0..W {
                let edge = 2.0 * (x as f32 + 0.5) / W as f32 - 1.0;
                let surface = (1.0 - level + bulge * (edge * edge - 1.0 / 3.0)) * H as f32;
                let alpha = (y as f32 + 1.0 - surface).clamp(0.0, 1.0) * 255.0;
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha.round() as u8]);
            }
        }
        Arc::new(RgbaImage::new(W, H, rgba).expect("fixed liquid fill dimensions"))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::PowerAction;
    use crate::{
        build_scene_layout, SceneKind, SettingsView, ShadeChoice, ShadeExtras, TabletStatus,
        WallpaperPreset,
    };

    fn tablet() -> SceneContext {
        SceneContext {
            clock_text: "10:08".into(),
            home_date_text: "Sunday 22 November".into(),
            settings: Some(SettingsView {
                startup: "Tablet",
                panel: true,
                wallpaper: WallpaperPreset::Midnight,
                follow_omarchy_colors: false,
                bindings: "Preview",
                feedback: None,
            }),
            power_menu: Some(PowerMenu::Closed),
            ..SceneContext::default()
        }
    }

    fn scene(w: f32, h: f32, ctx: &SceneContext) -> SceneOutput {
        build_scene_layout(SceneKind::Shade, w, h, &ThemePalette::default_dark(), ctx)
    }

    fn text<'a>(out: &'a SceneOutput, value: &str) -> Option<&'a TextLabel> {
        out.draw_list.commands.iter().find_map(|cmd| match cmd {
            DrawCommand::Text(label) if label.text == value => Some(label),
            _ => None,
        })
    }

    fn hit(out: &SceneOutput, target: &HitTarget) -> Option<HitRect> {
        out.hit_regions
            .iter()
            .find(|entry| &entry.target == target)
            .map(|entry| entry.rect)
    }

    fn slider(out: &SceneOutput, id: &str) -> Option<HitRect> {
        out.hit_regions.iter().find_map(|entry| match entry.target {
            HitTarget::ShadeSlider { id: slider, rect } if slider == id => {
                assert_eq!(rect, entry.rect, "input uses the full painted capsule");
                Some(rect)
            }
            _ => None,
        })
    }

    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
    }

    #[test]
    fn landscape_anchors_the_reference_columns_to_both_edges() {
        let mut ctx = tablet();
        ctx.shade.tablet_notifications = TabletNotifications::Empty;
        for (width, pane_start, pane_width) in [(1280.0, 640.0, 640.0), (2048.0, 1285.5, 762.5)] {
            assert!(tablet_shade_is_split(width, 800.0));
            near(tablet_shade_scale(width, 800.0), 1.25);
            let view = ShadeViewport::new(width, 800.0, 1.0);
            let history = view.pane(TabletShadePage::Notifications, 1.0);
            let controls = view.pane(TabletShadePage::Controls, 1.0);
            near(history.x, 0.0);
            near(history.w, pane_width);
            near(controls.x, pane_start);
            near(controls.x + controls.w, width);

            let output = scene(width, 800.0, &ctx);
            let wifi = hit(&output, &toggle("wifi")).unwrap();
            let bt = hit(&output, &toggle("bluetooth")).unwrap();
            let brightness = slider(&output, "brightness").unwrap();
            let volume = slider(&output, "volume").unwrap();
            let mobile = hit(&output, &toggle("lte")).unwrap();
            near(wifi.x, pane_start + 35.0);
            near(wifi.y, 211.5);
            near(wifi.h, 116.25);
            near(bt.y, wifi.y);
            near(brightness.y, 345.25);
            near(brightness.h, 250.0);
            near(brightness.x, wifi.x);
            near(volume.x, brightness.x + brightness.w + 17.5);
            near(mobile.x, bt.x);
            near(mobile.y, brightness.y);
            near(mobile.w, brightness.w * 2.0 + 17.5);
            let empty = text(&output, "No notifications").unwrap();
            near(empty.x, history.w * 0.5);
            near(empty.y, 516.5);
            assert_eq!(
                output.hit_test(empty.x, empty.y),
                Some(&HitTarget::DismissOverlay)
            );

            ctx.shade.tablet_page = TabletShadePage::Notifications;
            assert_eq!(scene(width, 800.0, &ctx), output, "landscape is not paged");
        }
    }

    #[test]
    fn portrait_pages_slide_independently_without_targets_until_settled() {
        let mut ctx = tablet();
        let (width, height) = (800.0, 1280.0);
        assert!(!tablet_shade_is_split(width, height));
        assert!(!tablet_shade_is_split(width, width));
        near(tablet_shade_scale(width, height), 800.0 * 3.0 / 1220.0);
        let controls = scene(width, height, &ctx);
        let wifi = text(&controls, "Wi-Fi").unwrap();
        let clock = text(&controls, "10:").unwrap();
        let date = text(&controls, "Sunday 22 November").unwrap();
        let wifi_hit = hit(&controls, &toggle("wifi")).unwrap();
        near(wifi_hit.x, 55.082);
        near(wifi_hit.y, 273.18);
        near(wifi_hit.h, 121.97);
        assert!(!controls.draw_list.commands.iter().any(|command| {
            matches!(command, DrawCommand::Text(label) if label.text.contains("history unavailable"))
        }));

        ctx.shade.tablet_page = TabletShadePage::Notifications;
        let runtime = scene(width, height, &ctx);
        assert!(text(&runtime, "No notifications").is_none());
        assert!(text(&runtime, "Notification history unavailable").is_some());
        ctx.shade.tablet_notifications = TabletNotifications::Empty;
        let notifications = scene(width, height, &ctx);
        assert!(text(&notifications, "Wi-Fi").is_none());
        assert_eq!(notifications.hit_regions.len(), 1);
        assert_eq!(
            notifications.hit_test(400.0, 800.0),
            Some(&HitTarget::DismissOverlay)
        );
        let empty = text(&notifications, "No notifications").unwrap();
        near(empty.x, width * 0.5);
        assert_eq!(text(&notifications, "10:"), Some(clock));

        ctx.shade.tablet_page_position = Some(0.37);
        let slide = scene(width, height, &ctx);
        near(
            text(&slide, "No notifications").unwrap().x,
            empty.x - 0.37 * width,
        );
        let sliding_wifi = text(&slide, "Wi-Fi").unwrap();
        near(sliding_wifi.x, wifi.x + 0.63 * width);
        near(sliding_wifi.y, wifi.y);
        assert_eq!(
            text(&slide, "10:"),
            Some(clock),
            "shared header stays fixed"
        );
        assert_eq!(text(&slide, "Sunday 22 November"), Some(date));
        assert_eq!(slide.hit_regions, []);
        ctx.shade.tablet_page = TabletShadePage::Controls;
        assert_eq!(scene(width, height, &ctx), slide, "position is absolute");
        for (position, endpoint) in [(0.0, notifications), (1.0, controls)] {
            ctx.shade.tablet_page_position = Some(position);
            let settling = scene(width, height, &ctx);
            assert_eq!(
                settling.draw_list, endpoint.draw_list,
                "no offstage pane paint"
            );
            assert_eq!(settling.hit_regions, []);
        }
    }

    #[test]
    fn reveal_morph_staggers_cells_and_keeps_battery_clear_of_its_percent() {
        let mut ctx = tablet();
        ctx.tablet_status = TabletStatus {
            battery: Some((100, true)),
            wifi: Some(true),
        };
        let (width, height) = (1280.0, 800.0);
        ctx.overlay_progress = 0.0;
        assert_eq!(scene(width, height, &ctx), SceneOutput::default());
        for progress in (1..=100).map(|step| step as f32 / 100.0) {
            ctx.overlay_progress = progress;
            let out = scene(width, height, &ctx);
            let percentage = text(&out, "100%").unwrap();
            let battery = out
                .draw_list
                .commands
                .iter()
                .find_map(|cmd| match cmd {
                    DrawCommand::Image(glyph) => Some(glyph),
                    _ => None,
                })
                .unwrap();
            assert!(battery.x + battery.w <= percentage.x - 32.0, "{progress}");
            if progress < 1.0 {
                assert_eq!(out.hit_regions, []);
            }
        }
        ctx.overlay_progress = 0.25;
        let early = scene(width, height, &ctx);
        let clock = text(&early, "10:").unwrap();
        assert!(text(&early, "Sunday 22 November").is_none());
        assert!(text(&early, "Wi-Fi").is_none());
        ctx.overlay_progress = 0.34;
        let first_cell = scene(width, height, &ctx);
        assert!(text(&first_cell, "Wi-Fi").is_some());
        assert!(text(&first_cell, "Bluetooth").is_none());
        ctx.overlay_progress = 0.75;
        let later = scene(width, height, &ctx);
        let larger = text(&later, "10:").unwrap();
        assert!(larger.scale > clock.scale && larger.y > clock.y && larger.x > clock.x);
        assert!(text(&later, "08").unwrap().color.a < larger.color.a);
        assert!(text(&later, "Sunday 22 November").unwrap().color.a < 1.0);
        assert!(text(&later, "Bluetooth").is_some());
    }

    #[test]
    fn small_viewports_keep_controls_in_bounds_with_disjoint_touch_targets() {
        let mut ctx = tablet();
        ctx.shade.extras = ShadeExtras {
            wifi: true,
            bluetooth: true,
            audio: true,
            muted: Some(false),
            media: Some("Title".into()),
            ..ShadeExtras::default()
        };
        for (w, h) in [(360.0, 640.0), (480.0, 270.0), (240.0, 120.0), (1.0, 1.0)] {
            assert!(tablet_shade_scale(w, h) > 0.0);
            let out = scene(w, h, &ctx);
            let bounds = HitRect::new(0.0, 0.0, w, h);
            let controls = &out.hit_regions[1..];
            assert_eq!(out.hit_regions[0].target, HitTarget::DismissOverlay);
            for (i, region) in controls.iter().enumerate() {
                let a = region.rect;
                assert!(fits(a, bounds), "{region:?} at {w}x{h}");
                assert_eq!(
                    out.hit_test(a.x + a.w * 0.5, a.y + a.h * 0.5),
                    Some(&region.target)
                );
                for other in &controls[i + 1..] {
                    let b = other.rect;
                    assert!(
                        a.x + a.w <= b.x
                            || b.x + b.w <= a.x
                            || a.y + a.h <= b.y
                            || b.y + b.h <= a.y,
                        "{region:?} and {other:?} overlap at {w}x{h}"
                    );
                }
            }
            if w >= 240.0 {
                assert!(slider(&out, "brightness").is_some());
                assert!(slider(&out, "volume").is_some());
                assert!(hit(&out, &toggle("wifi")).is_some());
            }
        }
        assert_eq!(scene(0.0, 800.0, &ctx), SceneOutput::default());
    }

    #[test]
    fn only_supported_service_actions_claim_their_capsule_or_chevron() {
        let mut ctx = tablet();
        ctx.shade.extras = ShadeExtras {
            wifi: true,
            host_bluetooth: true,
            audio: true,
            muted: Some(false),
            notifications: true,
            ..ShadeExtras::default()
        };
        let render = |ctx: &SceneContext| scene(800.0, 1280.0, ctx);
        let ready = render(&ctx);
        let show = |page| HitTarget::ShadeAction(ShadeAction::Show(page));
        let action = HitTarget::ShadeAction;
        for (page, id) in [
            (ShadePage::Wifi, "wifi"),
            (ShadePage::Bluetooth, "bluetooth"),
        ] {
            let detail = show(page);
            let toggle = toggle(id);
            let body = hit(&ready, &toggle).unwrap();
            let chevron = hit(&ready, &detail).unwrap();
            assert!(body.x + body.w < chevron.x && chevron.w >= 44.0);
            for (target, rect) in [(toggle, body), (detail, chevron)] {
                assert_eq!(
                    ready.hit_test(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5),
                    Some(&target)
                );
            }
        }
        for target in [
            show(ShadePage::Audio),
            show(ShadePage::More),
            action(ShadeAction::Mute),
            action(ShadeAction::Quiet),
            action(ShadeAction::Settings),
            HitTarget::Power(PowerInput::Open),
            toggle("desk_mode"),
            toggle("profile"),
            toggle("autorotate"),
            toggle("night_light"),
        ] {
            assert!(hit(&ready, &target).is_some(), "{target:?}");
        }

        ctx.shade.pending = Some(3);
        let busy = render(&ctx);
        assert!(text(&busy, "Working...").is_some());
        assert!(
            hit(&busy, &toggle("wifi")).is_some(),
            "hardware toggle is not an optional request"
        );
        assert!(hit(&busy, &show(ShadePage::Wifi)).is_none());
        assert!(hit(&busy, &action(ShadeAction::Mute)).is_none());
        assert!(hit(&busy, &action(ShadeAction::Quiet)).is_none());

        ctx.shade.pending = None;
        ctx.shade.wifi_radio = Some(false);
        let off = render(&ctx);
        assert!(
            text(&off, "Home network").is_none(),
            "don't show stale SSID"
        );
        assert!(hit(&off, &toggle("wifi")).is_some());
        assert!(hit(&off, &show(ShadePage::Wifi)).is_some());

        ctx.shade.wifi_radio = None;
        ctx.shade.bluetooth_available = false;
        ctx.shade.mobile_radio = None;
        ctx.shade.brightness_available = false;
        ctx.shade.auto_rotate_available = false;
        ctx.shade.night_light_available = false;
        ctx.shade.power_profile = None;
        ctx.shade.quiet = None;
        let unavailable = render(&ctx);
        for id in [
            "wifi",
            "bluetooth",
            "lte",
            "profile",
            "autorotate",
            "night_light",
        ] {
            assert!(hit(&unavailable, &toggle(id)).is_none(), "{id}");
        }
        for title in [
            "Network unavailable",
            "Bluetooth unavailable",
            "Mobile network unavailable",
        ] {
            assert!(text(&unavailable, title).is_some(), "{title}");
        }
        assert!(slider(&unavailable, "brightness").is_none());
        assert!(slider(&unavailable, "volume").is_some());
        assert!(hit(&unavailable, &show(ShadePage::Wifi)).is_none());
        assert!(hit(&unavailable, &show(ShadePage::Bluetooth)).is_none());
        assert!(hit(&unavailable, &action(ShadeAction::Quiet)).is_none());
        assert!(text(&unavailable, "Silent").unwrap().x < text(&ready, "Silent").unwrap().x);
    }

    #[test]
    fn details_and_power_preempt_the_pager_and_do_not_leave_hidden_targets() {
        let mut ctx = tablet();
        ctx.shade.tablet_page_position = Some(0.3);
        ctx.shade.tablet_notifications = TabletNotifications::Empty;
        ctx.shade.page = ShadePage::Wifi;
        ctx.shade.extras.host_wifi = true;
        ctx.shade.choices = vec![ShadeChoice {
            id: "saved".into(),
            label: "Saved network".into(),
            active: false,
        }];
        let render = |ctx: &SceneContext| scene(800.0, 1280.0, ctx);
        let details = render(&ctx);
        let back = HitTarget::ShadeAction(ShadeAction::Show(ShadePage::Main));
        let choose = HitTarget::ShadeAction(ShadeAction::Select(ShadePage::Wifi, "saved".into()));
        assert!(hit(&details, &back).is_some());
        assert!(hit(&details, &choose).is_some());
        assert!(hit(&details, &toggle("wifi")).is_none());
        assert!(text(&details, "No notifications").is_none());
        ctx.shade.pending = Some(1);
        assert!(hit(&render(&ctx), &choose).is_none());

        ctx.power_menu = Some(PowerMenu::Confirm(PowerAction::Restart));
        let power = render(&ctx);
        assert!(hit(&power, &HitTarget::Power(PowerInput::Confirm)).is_some());
        assert!(hit(&power, &back).is_none());
        assert!(power
            .hit_regions
            .iter()
            .all(|entry| matches!(entry.target, HitTarget::Power(_) | HitTarget::PowerPanel)));
        ctx.overlay_progress = 0.85;
        assert_eq!(render(&ctx).hit_regions, []);
    }

    #[test]
    fn gap_shade_keeps_read_only_split_and_paged_geometry() {
        let mut ctx = SceneContext {
            overlay_gap_gestures: true,
            ..SceneContext::default()
        };
        ctx.shade.tablet_notifications = TabletNotifications::Empty;
        for (w, h, split) in [(1280.0, 800.0, true), (800.0, 1280.0, false)] {
            ctx.shade.tablet_page = TabletShadePage::Controls;
            ctx.shade.tablet_page_position = None;
            ctx.overlay_progress = 0.0;
            assert_eq!(scene(w, h, &ctx), SceneOutput::default());
            ctx.overlay_progress = 0.05;
            let early = scene(w, h, &ctx);
            assert_eq!(early.hit_regions, []);
            let clock = text(&early, "12:").unwrap();
            assert!((0.0..0.2).contains(&clock.color.a));

            ctx.overlay_progress = 1.0;
            let opened = scene(w, h, &ctx);
            let history = "Notification history unavailable";
            let wifi = text(&opened, "Wi-Fi").unwrap();
            let level = opened
                .draw_list
                .commands
                .iter()
                .find_map(|command| match command {
                    DrawCommand::Image(quad) if quad.w >= 44.0 && quad.h > quad.w => Some(quad),
                    _ => None,
                })
                .unwrap();
            let s = tablet_shade_scale(w, h);
            let pane_x = if split { w * 0.5 } else { 0.0 };
            near(level.x, pane_x + s * 28.0);
            near(wifi.x, level.x + s * 56.0);
            assert!(level.y > wifi.y && level.y + level.h < h);
            assert!(text(&opened, "Controls are read-only").is_some());
            if split {
                near(text(&opened, history).unwrap().x, w * 0.25);
            } else {
                assert!(text(&opened, history).is_none());
            }
            assert_eq!(
                opened.hit_regions,
                [crate::HitRegion {
                    rect: HitRect::new(0.0, 0.0, w, h),
                    target: HitTarget::DismissOverlay,
                }],
            );
            assert!(!opened.draw_list.commands.iter().any(|command| {
                matches!(command, DrawCommand::Rect(rect) if rect.y >= h - 30.0 && rect.h <= 11.0)
            }));

            ctx.shade.tablet_page = TabletShadePage::Notifications;
            let notifications = scene(w, h, &ctx);
            assert!(text(&notifications, "No notifications").is_none());
            if split {
                assert_eq!(notifications, opened);
            } else {
                near(text(&notifications, history).unwrap().x, w * 0.5);
                assert_eq!(notifications.hit_regions.len(), 1);
                assert_eq!(
                    notifications.hit_test(400.0, 800.0),
                    Some(&HitTarget::DismissOverlay)
                );
                ctx.shade.tablet_page_position = Some(0.4);
                let sliding = scene(w, h, &ctx);
                assert_eq!(sliding.hit_regions, []);
                near(text(&sliding, history).unwrap().x, w * 0.1);
                near(
                    text(&sliding, "Wi-Fi").unwrap().x,
                    text(&opened, "Wi-Fi").unwrap().x + w * 0.6,
                );
            }
        }
    }

    #[test]
    fn gap_shade_does_not_expose_optional_or_session_actions() {
        let mut ctx = SceneContext {
            overlay_gap_gestures: true,
            ..SceneContext::default()
        };
        ctx.shade.extras = ShadeExtras {
            wifi: true,
            host_wifi: true,
            host_bluetooth: true,
            host_audio: true,
            screenshot: true,
            notifications: true,
            media: Some("Track title".into()),
            muted: Some(false),
            ..ShadeExtras::default()
        };
        let render = |ctx: &SceneContext| scene(800.0, 1280.0, ctx);
        ctx.shade.choices = vec![ShadeChoice {
            id: "saved".into(),
            label: "Saved network".into(),
            active: false,
        }];
        let main = render(&ctx);
        for title in ["Silent", "Balanced", "Auto-rotate", "Desktop view", "More"] {
            assert!(text(&main, title).is_none(), "{title}");
        }
        for page in [
            ShadePage::Wifi,
            ShadePage::Bluetooth,
            ShadePage::Audio,
            ShadePage::More,
        ] {
            ctx.shade.page = page;
            assert_eq!(render(&ctx), main, "{page:?} must not expose a detail view");
        }
    }

    #[test]
    fn tablet_state_does_not_change_overlay_or_standalone_desk() {
        let mut ctx = tablet();
        for power_menu in [None, Some(PowerMenu::Closed)] {
            ctx.tablet_mode = power_menu.is_none();
            ctx.power_menu = power_menu;
            let baseline = scene(1280.0, 800.0, &ctx);
            assert!(text(&baseline, "Connectivity").is_some());
            assert!(hit(&baseline, &toggle("wifi")).unwrap().x < 640.0);
            let mut pager = ctx.clone();
            pager.shade.tablet_notifications = TabletNotifications::Empty;
            pager.shade.tablet_page = TabletShadePage::Notifications;
            pager.shade.tablet_page_position = Some(0.7);
            assert_eq!(scene(1280.0, 800.0, &pager), baseline);
        }
    }
}
