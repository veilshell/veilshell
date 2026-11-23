//! Glassmorphic scene layout builders and interactive hit-testing for all `VeilShell` surfaces.

use std::sync::Arc;

use veilshell_core::{OskState, Rgba, ThemePalette};

use crate::osk::build_osk;
use crate::power::PowerMenu;
use crate::primitives::{
    DrawCommand, DrawList, HitRect, HitTarget, ImageQuad, RgbaImage, RoundedRect, SceneOutput,
    TextAlign,
};
use crate::recents::build_recents;
use crate::shade::{build_shade, ShadeStatus};
use crate::shade_glyphs::{self, Glyph};
use crate::tablet_home::{TabletHomeClock, TabletHomeGrid};
use crate::{SceneKind, TABLET_STATUS_HEIGHT};

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

/// Underlay and entry pose used by the touch Recents transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabletOverviewOrigin {
    App,
    Home,
}

/// Dynamic data injected into scene layout builders (live windows, apps, hardware state, gestures).
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SceneContext {
    pub settings: Option<crate::SettingsView>,
    pub tablet_status: crate::TabletStatus,
    pub session_wallpaper: Option<Arc<RgbaImage>>,
    pub launcher: crate::LauncherState,
    /// Reuse the touch layouts in the in-desktop overlay, without owning the session edges.
    pub overlay_gap_gestures: bool,
    pub clock_text: String,
    pub home_date_text: String,
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
    pub drawer: crate::TabletDrawerState,
    /// Open windows shown in the `Recents` deck.
    pub recents_cards: Vec<RecentsCard>,
    pub recents_index: usize,
    pub recents_position: Option<f32>,
    pub recents_throw: Option<(String, f32)>,
    /// Which surface is under a touch bottom-swipe overview.
    pub tablet_overview: Option<TabletOverviewOrigin>,
    /// Multi-layer touch keyboard state and modifier latches.
    pub osk_state: OskState,
    /// Full logical output size, not the height of the keyboard surface.
    /// Offscreen previews assume their OSK panel occupies a third of the output.
    pub osk_output_size: Option<(u32, u32)>,
    /// Whether to expose the bottom-right keyboard opener.
    pub keyboard_button: bool,
}

impl Default for SceneContext {
    fn default() -> Self {
        Self {
            settings: None,
            tablet_status: crate::TabletStatus::default(),
            session_wallpaper: None,
            launcher: crate::LauncherState::default(),
            overlay_gap_gestures: false,
            clock_text: "12:00".into(),
            home_date_text: String::new(),
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
            drawer: crate::TabletDrawerState::default(),
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
            tablet_overview: None,
            osk_state: OskState::default(),
            osk_output_size: None,
            keyboard_button: true,
        }
    }
}

impl SceneContext {
    /// Whether the owned session is using the standalone Tablet layout.
    #[must_use]
    pub const fn uses_tablet_status(&self) -> bool {
        self.power_menu.is_some() && self.tablet_mode
    }

    pub(crate) const fn uses_overlay_gap_gestures(&self) -> bool {
        self.overlay_gap_gestures && self.power_menu.is_none()
    }

    /// Whether Home, Drawer, and Recents use the touch navigation layout.
    #[must_use]
    pub const fn uses_touch_navigation(&self) -> bool {
        self.uses_tablet_status() || self.uses_overlay_gap_gestures()
    }

    /// Whether Shade uses the touch split/paging layout, independently of status ownership.
    #[must_use]
    pub const fn uses_touch_shade(&self) -> bool {
        self.uses_tablet_status() || self.uses_overlay_gap_gestures()
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

impl SceneContext {
    /// Match gesture destinations to the grid that actually renders Home.
    #[must_use]
    pub fn home_page_count(&self, width: f32, height: f32) -> usize {
        HomeLayout::new(self.uses_touch_navigation(), width, height)
            .page_count(self.home_apps.len())
    }
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

    fn cell_for_slot(&self, slot: usize) -> HitRect {
        HitRect::new(
            self.inset + (slot % self.columns) as f32 * (self.cell_w + HOME_GRID_GAP),
            self.row_y(slot / self.columns),
            self.cell_w,
            self.cell_h,
        )
    }
}

enum HomeLayout {
    Overlay(HomeGrid),
    Tablet(TabletHomeGrid),
}

impl HomeLayout {
    fn new(tablet: bool, width: f32, height: f32) -> Self {
        if tablet {
            Self::Tablet(TabletHomeGrid::new(width, height))
        } else {
            Self::Overlay(HomeGrid::new(width, height))
        }
    }

    fn page_count(&self, count: usize) -> usize {
        match self {
            Self::Overlay(grid) => grid.page_count(count),
            Self::Tablet(grid) => grid.page_count(count),
        }
    }

    fn app_range(&self, page: usize, count: usize) -> std::ops::Range<usize> {
        match self {
            Self::Overlay(grid) => {
                let start = (page * grid.capacity()).min(count);
                start..(start + grid.capacity()).min(count)
            }
            Self::Tablet(grid) => grid.app_range(page, count),
        }
    }

    fn app_cell(&self, page: usize, index: usize, count: usize) -> Option<HitRect> {
        match self {
            Self::Overlay(grid) => Some(grid.cell_for_slot(index % grid.capacity())),
            Self::Tablet(grid) => grid.app_cell(page, index, count),
        }
    }

    fn clock(&self, page: usize) -> Option<TabletHomeClock> {
        match self {
            Self::Overlay(_) => None,
            Self::Tablet(grid) => grid.clock(page),
        }
    }

    fn pager_y(&self, count: usize) -> Option<f32> {
        match self {
            Self::Overlay(grid) => (grid.page_count(count) > 1).then(|| grid.pager_y()),
            Self::Tablet(grid) => grid.pager_y(count),
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
        SceneKind::Home => build_home(&mut out, width, height, palette, ctx, None),
        SceneKind::Drawer if ctx.uses_touch_navigation() => {
            build_home(
                &mut out,
                width,
                height,
                palette,
                ctx,
                Some(HomeCover::Drawer),
            );
            crate::tablet_drawer::build_drawer(&mut out, width, height, palette, ctx);
        }
        SceneKind::Launcher => {
            crate::launcher::build_launcher(&mut out, width, height, palette, ctx);
        }
        SceneKind::Settings if !ctx.uses_overlay_gap_gestures() => {
            crate::settings::build_settings(
                &mut out,
                width,
                height,
                palette,
                ctx.settings.as_ref(),
            );
        }
        SceneKind::Drawer | SceneKind::Settings => {}
        SceneKind::Recents => build_recents(&mut out, width, height, palette, ctx),
        SceneKind::Shade => build_shade(&mut out, width, height, palette, ctx),
        SceneKind::Osk => build_osk(&mut out, width, height, palette, ctx),
    }
    out
}

/// Paint the settled Home page beneath Shade on the same surface, with only Shade hit targets.
#[must_use]
pub fn build_shade_over_home(
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) -> SceneOutput {
    let mut out = SceneOutput::default();
    build_home(
        &mut out,
        width,
        height,
        palette,
        ctx,
        Some(HomeCover::Shade),
    );
    let shade_start = out.draw_list.commands.len();
    build_shade(&mut out, width, height, palette, ctx);
    // Backdrop gradients replace pixels in the software renderer. Blend a
    // scrim at Shade's frost opacity over Home instead of clearing that page.
    if let Some(DrawCommand::BackdropGradient { top, bottom }) =
        out.draw_list.commands.get(shade_start)
    {
        let scrim = RoundedRect::filled(0.0, 0.0, width, height, 0.0, mix(*top, *bottom, 0.5));
        out.draw_list.commands[shade_start] = DrawCommand::Rect(scrim);
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

/// Bottom-right keyboard target, inset from the edges so the corner itself
/// still belongs to apps. Omit it if it would crowd the pill's swipe zone.
#[must_use]
pub fn keyboard_button_bounds(width: f32, height: f32) -> Option<HitRect> {
    const TARGET: f32 = 44.0;
    const RIGHT_INSET: f32 = 12.0;
    let pill = bottom_pill_bounds(width, height);
    let x = (width - RIGHT_INSET - TARGET).floor();
    (height >= TARGET && x >= pill.x + pill.w + 28.0)
        .then(|| HitRect::new(x, (height - TARGET - 2.0).floor().max(0.0), TARGET, TARGET))
}

/// Build the legacy painted top pull handle, not the transparent overlay gap strip.
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
        palette.foreground.with_alpha(0.40)
    };
    let border_alpha = if active { 0.20 } else { 0.12 };
    list.push_rect(
        RoundedRect::filled(hx, hy, handle_w, handle_h, handle_h * 0.5, fill).with_glass_border(
            Rgba::from_rgb_u8(255, 255, 255).with_alpha(border_alpha),
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
    if ctx.uses_overlay_gap_gestures() {
        return;
    }
    let bounds = bottom_pill_bounds(width, height);
    if ctx.uses_tablet_status() {
        // Keep the 180×5 visual mark independent of the 200×52 input zone.
        let raise = 8.0 * (-ctx.pill_drag_y / 280.0).clamp(0.0, 1.0);
        let shift = (ctx.pill_drag_x * 0.15).clamp(-24.0, 24.0);
        let fill = if ctx.pill_pressed {
            palette.accent.with_alpha(0.95)
        } else {
            palette.foreground.with_alpha(0.55)
        };
        list.push_rect(RoundedRect::filled(
            (bounds.x + shift).clamp(0.0, (width - bounds.w).max(0.0)),
            (height - 14.5 - raise).max(0.0),
            bounds.w,
            5.0,
            2.5,
            fill,
        ));
        return;
    }
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
        Rgba::from_rgb_u8(138, 138, 138).with_alpha(0.55)
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
    if ctx.uses_overlay_gap_gestures() {
        return;
    }
    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
    if !ctx.uses_tablet_status() {
        out.push_hit(HitRect::new(0.0, 0.0, width, height), HitTarget::BottomPill);
    }
    if let Some(button) = keyboard_button_bounds(width, height).filter(|_| ctx.keyboard_button) {
        // Keep the full touch target without painting a tile behind the icon.
        shade_glyphs::draw(
            &mut out.draw_list,
            Glyph::Keyboard,
            button.x + button.w * 0.5,
            button.y + button.h * 0.5,
            24.0,
            palette.foreground.with_alpha(0.60),
        );
        out.push_hit(button, HitTarget::OpenKeyboard);
    }
}

pub(super) fn header_dismiss_bounds(width: f32, header_height: f32) -> Option<HitRect> {
    if !width.is_finite() || !header_height.is_finite() {
        return None;
    }
    let inset = 20.0_f32.min(width * 0.10);
    let w = 72.0_f32.min(width - 2.0 * inset);
    // Keep the full touch target unless the header is too short. Leave at
    // least 4 px of clearance on either side even in the compact layout.
    let h = 44.0_f32.min(header_height - 8.0);
    if w < 44.0 || h < 24.0 {
        return None;
    }
    let y = 12.0_f32.min(header_height - h - 4.0);
    Some(HitRect::new(width - inset - w, y, w, h))
}

pub(super) fn push_header_dismiss(
    list: &mut DrawList,
    rect: HitRect,
    label: &str,
    palette: &ThemePalette,
    alpha: f32,
) {
    list.push_rect(
        RoundedRect::filled(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            9.0,
            palette.surface.with_alpha(alpha),
        )
        .with_glass_border(palette.foreground.with_alpha(0.18 * alpha), 1.0, 0.0),
    );
    list.push_text(
        rect.x + rect.w * 0.5,
        rect.y + (rect.h - crate::font::GLYPH_H as f32) * 0.5,
        crate::font::fit_text(label, 1, rect.w - 8.0),
        1,
        palette.foreground.with_alpha(palette.foreground.a * alpha),
        TextAlign::Center,
    );
}

#[derive(Clone, Copy)]
enum HomeCover {
    Shade,
    Drawer,
    Recents,
}

pub(crate) fn build_home_under_recents(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    build_home(out, width, height, palette, ctx, Some(HomeCover::Recents));
}

fn build_home(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
    cover: Option<HomeCover>,
) {
    let underlay = cover.is_some();
    let progress = if underlay {
        1.0
    } else {
        ctx.overlay_progress.clamp(0.0, 1.0)
    };
    if ctx.uses_overlay_gap_gestures() && progress <= 0.0 {
        return;
    }
    let y_slide = (1.0 - progress) * home_reveal_travel(height);
    let touch = ctx.uses_touch_navigation();
    push_home_background(&mut out.draw_list, width, height, ctx, progress);
    let content_alpha = if ctx.uses_touch_shade() && matches!(cover, Some(HomeCover::Shade)) {
        // Approximate the opaque frost with a reversible fade of Home
        // content. The wallpaper stays; the launcher clock must not ghost
        // behind the moving Shade clock.
        1.0 - (ctx.overlay_progress / crate::tablet_shade::FROST_COMPLETE).clamp(0.0, 1.0)
    } else if underlay {
        1.0 - ctx.overlay_progress.clamp(0.0, 1.0)
    } else {
        progress
    };
    if underlay && content_alpha <= 0.0 {
        return;
    }
    let fg = HOME_FOREGROUND.with_alpha(content_alpha);
    push_home_status(out, (width, height), palette, ctx, cover, fg);

    let apps = &ctx.home_apps;
    let grid = HomeLayout::new(touch, width, height);
    let close = if ctx.uses_overlay_gap_gestures() {
        // The first clock cell starts at the top of the Tablet grid even in
        // an empty catalog. In a viewport too short for any grid, keep the
        // button above the empty-state text instead.
        header_dismiss_bounds(
            width,
            grid.clock(0).map_or(height * 0.5, |clock| clock.cell.y),
        )
    } else {
        None
    };
    let total_pages = grid.page_count(apps.len());
    let page = ctx.home_page.min(total_pages - 1);
    let position = (if underlay { None } else { ctx.home_position })
        .unwrap_or(page as f32)
        .clamp(-0.2, (total_pages - 1) as f32 + 0.2);
    let first = position.floor() as usize;
    let last = (position.ceil() as usize).min(total_pages - 1);
    for visible_page in first..=last {
        let x_slide = (visible_page as f32 - position) * width;
        if let Some(clock) = grid.clock(visible_page) {
            push_home_clock(
                &mut out.draw_list,
                clock,
                ctx,
                width,
                (x_slide, y_slide),
                fg,
            );
        }
        push_home_page(
            out,
            &grid,
            apps,
            visible_page,
            width,
            (x_slide, y_slide),
            content_alpha,
        );
    }
    if apps.is_empty() {
        let empty_y = close.map_or(height * 0.5, |rect| {
            (height * 0.5).max(rect.y + rect.h + 6.0)
        });
        out.draw_list.push_text(
            width * 0.5,
            empty_y + y_slide,
            "No applications found",
            1,
            fg,
            TextAlign::Center,
        );
    } else if let Some(pager_y) = grid.pager_y(apps.len()) {
        push_home_pager(
            out,
            width,
            pager_y + y_slide,
            position,
            total_pages,
            content_alpha,
        );
    }
    // Paint Close with the underlay so the retracted panel matches settled
    // Home exactly, but only expose its hit in the foreground.
    if let Some(rect) = close {
        push_header_dismiss(&mut out.draw_list, rect, "Close", palette, content_alpha);
        if !underlay {
            out.push_hit(rect, HitTarget::DismissOverlay);
        }
    }
    if !underlay {
        push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
    }
    if underlay || progress < 1.0 || ctx.home_position.is_some() {
        out.hit_regions.clear();
    }
}

fn push_home_background(
    list: &mut DrawList,
    width: f32,
    height: f32,
    ctx: &SceneContext,
    progress: f32,
) {
    if ctx.uses_overlay_gap_gestures() {
        // Home and its underlays share an opaque base once Home is revealed;
        // don't depend on session settings, a wallpaper cache, or a window capture.
        list.push_rect(RoundedRect::filled(
            0.0,
            0.0,
            width,
            height,
            0.0,
            HOME_BACKGROUND.with_alpha(progress),
        ));
        return;
    }
    // A plain dark fallback also keeps the launcher legible over active windows.
    if ctx.settings.is_none() {
        let tint = HOME_BACKGROUND.with_alpha(progress);
        list.push_backdrop(tint, tint);
        return;
    }
    crate::settings::push_session_wallpaper(list, ctx, width, height, progress);
    let scrim = if ctx.uses_tablet_status() {
        Rgba::from_rgb_u8(0, 0, 0).with_alpha((20.0 / 255.0) * progress)
    } else if ctx.session_wallpaper.is_some() {
        HOME_BACKGROUND.with_alpha(0.32 * progress)
    } else {
        return;
    };
    list.push_rect(RoundedRect::filled(0.0, 0.0, width, height, 0.0, scrim));
}

fn push_home_page(
    out: &mut SceneOutput,
    grid: &HomeLayout,
    apps: &[AppTile],
    page: usize,
    width: f32,
    (x_slide, y_slide): (f32, f32),
    progress: f32,
) {
    let fg = HOME_FOREGROUND.with_alpha(progress);
    for index in grid.app_range(page, apps.len()) {
        let app = &apps[index];
        let Some(cell) = grid.app_cell(page, index, apps.len()) else {
            continue;
        };
        let x = cell.x + x_slide;
        if x >= width || x + cell.w <= 0.0 {
            continue;
        }
        let cell = HitRect::new(x, cell.y + y_slide, cell.w, cell.h);
        let target = push_home_tile(&mut out.draw_list, app, cell, progress, fg);
        out.push_hit(target, HitTarget::LaunchApp { index });
    }
}

pub(crate) fn push_home_tile(
    list: &mut DrawList,
    app: &AppTile,
    cell: HitRect,
    progress: f32,
    fg: Rgba,
) -> HitRect {
    let icon_box = 92.0_f32.min(cell.w).min(cell.h * 0.73);
    let icon_size = 85.0_f32.min(icon_box);
    let cx = cell.x + cell.w * 0.5;
    let cy = cell.y + icon_box * 0.5;
    if let Some(icon) = &app.icon {
        let scale = icon_size / icon.width.max(icon.height) as f32;
        let w = icon.width as f32 * scale;
        let h = icon.height as f32 * scale;
        list.push_image(ImageQuad {
            x: cx - w * 0.5,
            y: cy - h * 0.5,
            w,
            h,
            radius: 0.0,
            alpha: progress,
            image: Arc::clone(icon),
        });
    } else {
        list.push_rect(
            RoundedRect::filled(
                cx - icon_size * 0.5,
                cy - icon_size * 0.5,
                icon_size,
                icon_size,
                icon_size * 0.24,
                mix(HOME_BACKGROUND, app.tint, 0.28).with_alpha(progress),
            )
            .with_glass_border(HOME_FOREGROUND.with_alpha(0.15 * progress), 1.0, 0.0),
        );
        list.push_text(
            cx,
            cy - 7.0,
            crate::font::fit_text(&app.badge, 2, icon_size - 4.0),
            2,
            fg,
            TextAlign::Center,
        );
    }
    if cell.w >= 40.0 && cell.h >= 44.0 {
        list.push_text(
            cx,
            cell.y + icon_box + 9.0,
            crate::font::fit_text(&app.label, 1, cell.w - 4.0),
            1,
            fg,
            TextAlign::Center,
        );
    }
    HitRect::new(cell.x, cell.y, cell.w, (icon_box + 30.0).min(cell.h))
}

fn push_home_clock(
    list: &mut DrawList,
    clock: TabletHomeClock,
    ctx: &SceneContext,
    width: f32,
    (x_slide, y_slide): (f32, f32),
    fg: Rgba,
) {
    let cell = clock.cell;
    let x = cell.x + x_slide;
    if x >= width || x + cell.w <= 0.0 || clock.time_px < 10.0 {
        return;
    }
    let cx = x + cell.w * 0.5;
    let date_space = if ctx.home_date_text.is_empty() {
        0.0
    } else {
        clock.date_px + clock.gap
    };
    let top = cell.y + (cell.h - clock.time_px - date_space) * 0.5 + y_slide;
    // HUD typography uses discrete font tiers in both renderers.
    let time_tier = (clock.time_px / 9.5).round().max(1.0) as u32;
    let text_w = cell.w * 0.93;
    list.push_text(
        cx,
        top,
        crate::font::fit_text(&ctx.clock_text, time_tier, text_w),
        time_tier,
        fg,
        TextAlign::Center,
    );
    if date_space > 0.0 {
        let date_tier = if clock.date_px >= 16.0 { 2 } else { 1 };
        list.push_text(
            cx,
            top + clock.time_px + clock.gap,
            crate::font::fit_text(&ctx.home_date_text, date_tier, text_w),
            date_tier,
            Rgba::from_rgb_u8(202, 196, 208).with_alpha(fg.a),
            TextAlign::Center,
        );
    }
}

fn push_home_status(
    out: &mut SceneOutput,
    (width, height): (f32, f32),
    palette: &ThemePalette,
    ctx: &SceneContext,
    cover: Option<HomeCover>,
    fg: Rgba,
) {
    if ctx.uses_tablet_status() {
        let status_in_front = matches!(cover, Some(HomeCover::Drawer))
            || (matches!(cover, Some(HomeCover::Shade)) && ctx.overlay_progress > 0.0);
        if !status_in_front {
            crate::tablet_status::push_tablet_status_content(
                &mut out.draw_list,
                width,
                TABLET_STATUS_HEIGHT as f32,
                palette,
                ctx,
            );
        }
        return;
    }
    if ctx.uses_touch_navigation() {
        return;
    }
    out.push_hit(
        HitRect::new(0.0, 0.0, width, height),
        HitTarget::DismissOverlay,
    );
    if width >= 160.0 {
        let y = 22.0_f32.min(height * 0.1);
        let list = &mut out.draw_list;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DrawCommand;

    #[test]
    fn touch_layout_predicates_do_not_change_status_ownership() {
        assert!(!SceneContext::default().overlay_gap_gestures);
        for owned in [false, true] {
            for tablet_mode in [false, true] {
                for overlay_gap_gestures in [false, true] {
                    let ctx = SceneContext {
                        power_menu: owned.then_some(PowerMenu::Closed),
                        tablet_mode,
                        overlay_gap_gestures,
                        ..SceneContext::default()
                    };
                    let status = owned && tablet_mode;
                    let touch = status || (!owned && overlay_gap_gestures);
                    assert_eq!(ctx.uses_tablet_status(), status);
                    assert_eq!(ctx.uses_touch_navigation(), touch);
                    assert_eq!(ctx.uses_touch_shade(), touch);
                }
            }
        }
    }

    #[test]
    fn gap_home_uses_the_tablet_grid_and_passive_clock_but_no_status_or_pill() {
        let theme = ThemePalette::default_dark();
        let mut ctx = SceneContext {
            overlay_gap_gestures: true,
            home_date_text: "Sunday 22 November".into(),
            home_apps: default_demo_apps().into_iter().cycle().take(30).collect(),
            tablet_status: crate::TabletStatus {
                battery: Some((47, true)),
                wifi: Some(true),
            },
            pill_pressed: true,
            ..SceneContext::default()
        };
        for (w, h, first_x, first_page_count) in
            [(1280.0, 800.0, 535.17, 22), (800.0, 1280.0, 497.25, 18)]
        {
            ctx.home_page = 0;
            assert_eq!(ctx.home_page_count(w, h), 2);
            let first = build_scene_layout(SceneKind::Home, w, h, &theme, &ctx);
            assert!(first.draw_list.commands.iter().any(|cmd| {
                matches!(cmd, DrawCommand::Text(label) if label.text == "Sunday 22 November")
            }));
            assert!(!first.draw_list.commands.iter().any(|cmd| {
                matches!(cmd, DrawCommand::Text(label)
                    if label.y < TABLET_STATUS_HEIGHT as f32 && label.text != "Close")
                    || matches!(cmd, DrawCommand::Image(_))
                    || matches!(cmd, DrawCommand::Rect(rect) if rect.y >= h - 30.0 && rect.h <= 11.0)
            }));
            assert_eq!(first.hit_test(100.0, 112.0), None, "the clock is passive");
            assert_eq!(
                first.hit_test(first_x, 112.0),
                Some(&HitTarget::LaunchApp { index: 0 })
            );
            ctx.home_page = 1;
            let next = build_scene_layout(SceneKind::Home, w, h, &theme, &ctx);
            assert_eq!(
                next.hit_test(100.0, 112.0),
                Some(&HitTarget::LaunchApp {
                    index: first_page_count
                })
            );
        }
    }

    #[test]
    fn gap_home_and_drawer_have_only_one_explicit_dismiss_each_even_without_apps() {
        let theme = ThemePalette::default_dark();
        let mut ctx = SceneContext {
            overlay_gap_gestures: true,
            home_apps: default_demo_apps().into_iter().cycle().take(70).collect(),
            ..SceneContext::default()
        };
        for (w, h) in [
            (1280.0, 800.0),
            (800.0, 1280.0),
            (240.0, 320.0),
            (320.0, 240.0),
        ] {
            let expected = HitRect::new(w - 92.0, 12.0, 72.0, 44.0);
            for (scene, label) in [(SceneKind::Home, "Close"), (SceneKind::Drawer, "Back")] {
                let open = build_scene_layout(scene, w, h, &theme, &ctx);
                assert!(open.draw_list.commands.iter().any(|cmd| matches!(
                    cmd, DrawCommand::Text(t) if t.text == label && expected.contains(t.x, t.y)
                )));
                assert_eq!(
                    open.hit_test(w - 56.0, 34.0),
                    Some(&HitTarget::DismissOverlay)
                );
                assert_eq!(open.hit_test(22.0, 34.0), None);
                let mut dismiss = open
                    .hit_regions
                    .iter()
                    .filter(|hit| hit.target == HitTarget::DismissOverlay);
                assert_eq!(dismiss.next().unwrap().rect, expected);
                assert!(dismiss.next().is_none());
                assert!(open.hit_regions.iter().all(|hit| {
                    hit.target == HitTarget::DismissOverlay || hit.rect.y > expected.y + expected.h
                }));

                ctx.overlay_progress = 0.6;
                let moving = build_scene_layout(scene, w, h, &theme, &ctx);
                assert_eq!(moving.hit_regions, []);
                ctx.overlay_progress = 1.0;

                let mut empty = ctx.clone();
                empty.home_apps.clear();
                let open_empty = build_scene_layout(scene, w, h, &theme, &empty);
                assert_eq!(
                    open_empty.hit_regions,
                    [crate::HitRegion {
                        rect: expected,
                        target: HitTarget::DismissOverlay
                    }]
                );
                assert!(open_empty.draw_list.commands.iter().any(|cmd| {
                    matches!(cmd, DrawCommand::Text(t) if t.text == "No applications found")
                }));

                empty.overlay_gap_gestures = false;
                for power_menu in [None, Some(PowerMenu::Closed)] {
                    empty.power_menu = power_menu;
                    let legacy = build_scene_layout(scene, w, h, &theme, &empty);
                    assert!(!legacy.draw_list.commands.iter().any(|cmd| matches!(
                        cmd, DrawCommand::Text(t) if t.text == label
                    )));
                }
            }
        }
    }

    #[test]
    fn compact_gap_drawer_keeps_back_inside_its_header() {
        let theme = ThemePalette::default_dark();
        let ctx = SceneContext {
            overlay_gap_gestures: true,
            ..SceneContext::default()
        };
        let out = build_scene_layout(SceneKind::Drawer, 240.0, 120.0, &theme, &ctx);
        let back = out
            .hit_regions
            .iter()
            .find(|hit| hit.target == HitTarget::DismissOverlay)
            .unwrap()
            .rect;
        assert!(back.y > 0.0 && back.y + back.h < 120.0 * 0.28);
        assert_eq!(
            out.hit_test(back.x + back.w * 0.5, back.y + back.h * 0.5),
            Some(&HitTarget::DismissOverlay)
        );
    }

    #[test]
    fn gap_home_underlays_preserve_the_page_to_the_last_pixel() {
        let theme = ThemePalette::default_dark();
        let renderer = crate::SdfSoftwareRenderer::default();
        let mut ctx = SceneContext {
            overlay_gap_gestures: true,
            tablet_overview: Some(TabletOverviewOrigin::Home),
            home_page: 1,
            home_apps: default_demo_apps().into_iter().cycle().take(30).collect(),
            ..SceneContext::default()
        };
        for (w, h) in [(1280, 800), (800, 1280)] {
            let (width, height) = (w as f32, h as f32);
            ctx.overlay_progress = 1.0;
            let home = build_scene_layout(SceneKind::Home, width, height, &theme, &ctx);
            assert_ne!(home.hit_regions, []);
            let underlays = |ctx: &SceneContext| {
                [
                    build_scene_layout(SceneKind::Drawer, width, height, &theme, ctx),
                    build_scene_layout(SceneKind::Recents, width, height, &theme, ctx),
                    build_shade_over_home(width, height, &theme, ctx),
                ]
            };
            ctx.overlay_progress = 0.0;
            for closing in underlays(&ctx) {
                assert_eq!(closing.draw_list, home.draw_list);
                assert_eq!(closing.hit_regions, []);
            }
            ctx.overlay_progress = 0.20;
            for partial in underlays(&ctx) {
                assert_eq!(partial.hit_regions, []);
                assert_eq!(partial.draw_list.commands[0], home.draw_list.commands[0]);
                assert!(partial.draw_list.commands.iter().any(|command| {
                    matches!(command, DrawCommand::Text(label)
                        if label.text == "Close" && label.color.a > 0.0
                            && label.color.a < theme.foreground.a)
                }));
                assert!(!partial
                    .draw_list
                    .commands
                    .iter()
                    .any(|command| matches!(command, DrawCommand::BackdropGradient { .. })));
                let frame = renderer.rasterize(w, h, &partial.draw_list).unwrap();
                assert!(
                    frame.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255),
                    "Home must hide the app behind the transitioning panel"
                );
            }
        }
    }

    #[test]
    fn tablet_clock_is_passive_and_displaces_app_targets_on_first_page_only() {
        let theme = ThemePalette::default_dark();
        let mut ctx = SceneContext {
            power_menu: Some(PowerMenu::Closed),
            home_date_text: "Sunday 22 November".into(),
            home_apps: default_demo_apps().into_iter().cycle().take(19).collect(),
            ..SceneContext::default()
        };
        assert_eq!(ctx.home_page_count(800.0, 1280.0), 2);
        let home = build_scene_layout(SceneKind::Home, 800.0, 1280.0, &theme, &ctx);
        let has_date = |home: &SceneOutput| {
            home.draw_list
                .commands
                .iter()
                .any(|draw| matches!(draw, DrawCommand::Text(t) if t.text == "Sunday 22 November"))
        };
        assert!(has_date(&home));
        assert!(home.hit_test(205.5, 146.0).is_none());
        assert_eq!(
            home.hit_test(497.25, 112.0),
            Some(&HitTarget::LaunchApp { index: 0 })
        );

        ctx.home_page = 1;
        let next = build_scene_layout(SceneKind::Home, 800.0, 1280.0, &theme, &ctx);
        assert!(!has_date(&next));
        assert_eq!(
            next.hit_test(108.25, 112.0),
            Some(&HitTarget::LaunchApp { index: 18 })
        );

        ctx.power_menu = None;
        ctx.home_page = 0;
        let overlay = build_scene_layout(SceneKind::Home, 800.0, 1280.0, &theme, &ctx);
        assert!(!has_date(&overlay));
        assert_eq!(
            overlay.hit_test(108.25, 112.0),
            Some(&HitTarget::LaunchApp { index: 0 })
        );
    }

    #[test]
    fn shade_over_home_keeps_the_settled_page_without_its_hit_targets() {
        let (w, h) = (390, 518);
        let (width, height) = (w as f32, h as f32);
        let theme = ThemePalette::default_dark();
        let mut ctx = SceneContext {
            power_menu: Some(PowerMenu::Closed),
            home_page: 1,
            ..SceneContext::default()
        };
        let home = build_scene_layout(SceneKind::Home, width, height, &theme, &ctx);
        assert!(home
            .hit_regions
            .iter()
            .any(|hit| matches!(hit.target, HitTarget::LaunchApp { index } if index > 0)));
        let row = crate::build_tablet_status(width, TABLET_STATUS_HEIGHT as f32, &theme, &ctx);
        assert!(home.draw_list.commands[1..].starts_with(&row.commands[1..]));

        ctx.overlay_progress = 0.1;
        let preview = build_shade_over_home(width, height, &theme, &ctx);
        let app_label = home.draw_list.commands[row.commands.len()..]
            .iter()
            .find_map(|draw| match draw {
                DrawCommand::Text(label) => Some(label),
                _ => None,
            })
            .expect("label from the second page");
        let same_app_label = |draw: &DrawCommand| {
            matches!(draw, DrawCommand::Text(label) if label.text == app_label.text
                && (label.x - app_label.x).abs() < f32::EPSILON
                && (label.y - app_label.y).abs() < f32::EPSILON
                && label.color.a > 0.0
                && label.color.a < app_label.color.a)
        };
        // The visible portion of Home keeps the page and fades with Shade.
        assert_eq!(preview.draw_list.commands[0], home.draw_list.commands[0]);
        assert!(preview.draw_list.commands.iter().any(same_app_label));
        assert_eq!(preview.hit_regions, [] as [crate::HitRegion; 0]);

        let renderer = crate::SdfSoftwareRenderer::default();
        let frame = renderer.rasterize(w, h, &preview.draw_list).unwrap();
        assert_eq!(frame.rgba[3], 255, "the foreground app cannot show through");

        ctx.overlay_progress = 1.0;
        let open = build_shade_over_home(width, height, &theme, &ctx);
        let controls = build_scene_layout(SceneKind::Shade, width, height, &theme, &ctx);
        assert_eq!(open.hit_regions, controls.hit_regions);
        assert_eq!(open.draw_list.commands[0], home.draw_list.commands[0]);
        assert!(!open.draw_list.commands.iter().any(|draw| {
            matches!(draw, DrawCommand::Text(label) if label.text == app_label.text)
        }));
    }

    #[test]
    fn shade_hides_the_home_clock_when_frost_is_established() {
        let theme = ThemePalette::default_dark();
        let ctx = SceneContext {
            clock_text: "08:51".into(),
            overlay_progress: crate::tablet_shade::FROST_COMPLETE,
            power_menu: Some(PowerMenu::Closed),
            ..SceneContext::default()
        };
        let (w, h) = (800.0, 1280.0);
        let shade = build_scene_layout(SceneKind::Shade, w, h, &theme, &ctx);
        let over_home = build_shade_over_home(w, h, &theme, &ctx);
        let labels = |out: SceneOutput| {
            out.draw_list
                .commands
                .into_iter()
                .filter_map(|draw| match draw {
                    DrawCommand::Text(label) => Some(label),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        // Only the moving Shade header and Shade labels remain.
        assert_eq!(labels(over_home), labels(shade));
    }
}
