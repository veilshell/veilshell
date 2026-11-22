//! Responsive Quick Settings groups for the fullscreen touch shade.

use veilshell_core::{Rgba, ThemePalette};

use crate::power::{draw_power_menu, PowerInput, PowerMenu};
use crate::scene::push_bottom_pill_bar;
use crate::shade_glyphs::{draw, draw_battery, Glyph};
use crate::{
    font, DrawList, HitRect, HitTarget, RoundedRect, SceneContext, SceneOutput, TextAlign,
};

const TOUCH_SIZE: f32 = 44.0;

/// Pages of the touch shade. Details can be used without a desktop bar.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShadePage {
    #[default]
    Main,
    Wifi,
    Bluetooth,
    Audio,
    More,
}

impl ShadePage {
    fn title(self) -> &'static str {
        match self {
            Self::Main => "Quick Settings",
            Self::Wifi => "Wi-Fi Networks",
            Self::Bluetooth => "Bluetooth Devices",
            Self::Audio => "Audio Output",
            Self::More => "More",
        }
    }
}

/// Extra actions separate from the existing radio and slider hit targets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShadeAction {
    Settings,
    Show(ShadePage),
    Page(usize),
    Select(ShadePage, String),
    Host(ShadePage),
    Mute,
    Quiet,
    Previous,
    PlayPause,
    Next,
    Screenshot,
    Notifications,
}

/// Saved connection, paired device, or audio output.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadeChoice {
    pub id: String,
    pub label: String,
    pub active: bool,
}

/// Capabilities and state read from optional desktop services.
#[derive(Clone, Debug, Default, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // Independent service capabilities.
pub struct ShadeExtras {
    pub host_wifi: bool,
    pub host_bluetooth: bool,
    pub host_audio: bool,
    pub wifi: bool,
    pub bluetooth: bool,
    pub audio: bool,
    pub muted: Option<bool>,
    pub bluetooth_name: Option<String>,
    pub media: Option<String>,
    pub screenshot: bool,
    pub notifications: bool,
}

impl ShadeExtras {
    /// Whether the corresponding host panel was found.
    #[must_use]
    pub fn host_panel(&self, page: ShadePage) -> bool {
        match page {
            ShadePage::Wifi => self.host_wifi,
            ShadePage::Bluetooth => self.host_bluetooth,
            ShadePage::Audio => self.host_audio,
            _ => false,
        }
    }

    fn has_more(&self) -> bool {
        self.media.is_some() || self.screenshot || self.notifications
    }
}

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
    pub auto_rotate_available: bool,
    pub night_light_available: bool,
    pub power_profile: Option<String>,
    pub quiet: Option<bool>,
    pub feedback: Option<String>,
    pub extras: ShadeExtras,
    pub page: ShadePage,
    pub page_offset: usize,
    pub choices: Vec<ShadeChoice>,
    pub pending: Option<u64>,
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
            auto_rotate_available: true,
            night_light_available: true,
            power_profile: Some("Balanced".into()),
            quiet: Some(false),
            feedback: None,
            extras: ShadeExtras::default(),
            page: ShadePage::Main,
            page_offset: 0,
            choices: Vec::new(),
            pending: None,
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
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    out.push_hit(
        HitRect::new(0.0, 0.0, width, height),
        HitTarget::DismissOverlay,
    );

    let style = ShadeStyle { palette, progress };
    if ctx.shade.page != ShadePage::Main {
        draw_details(out, &style, ctx, width, height);
        push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
        if progress < 1.0 {
            out.hit_regions.clear();
        }
        return;
    }

    let connectivity = connectivity_group(ctx);
    let display = display_group(ctx);
    let device = device_group(ctx);
    let portrait = [&connectivity, &display, &device];
    // If Display & Sound is absent, move Device to the other pane rather than
    // reserving empty space on the right.
    let left = [&connectivity, &device];
    let display_empty = display.is_empty();
    let right = [if display_empty { &device } else { &display }];
    let split = width > height && width >= 360.0 && (!display_empty || !device.is_empty());
    let panes: [&[&Group<'_>]; 2] = if split {
        [&left[..if display_empty { 1 } else { 2 }], &right]
    } else {
        [&portrait, &[]]
    };
    let layout = ShadeLayout::new(width, height, split, panes, progress);
    layout.draw_header(out, &style, ctx);
    for (index, pane) in panes.into_iter().enumerate() {
        layout.draw_pane(out, &style, index, pane);
    }
    layout.draw_feedback(
        out,
        &style,
        ctx.shade
            .feedback
            .as_deref()
            .or(ctx.shade.pending.map(|_| "Working...")),
    );
    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
    if progress < 1.0 {
        out.hit_regions.clear();
    }
}

#[derive(Clone)]
struct Tile<'a> {
    id: &'static str,
    glyph: Glyph,
    title: &'static str,
    short_title: &'static str,
    detail: &'a str,
    value: Option<bool>,
    action: Option<ShadeAction>,
}

impl<'a> Tile<'a> {
    fn new(id: &'static str, glyph: Glyph, title: &'static str, value: Option<bool>) -> Self {
        let detail = match value {
            Some(true) => "On",
            Some(false) => "Off",
            None => "Unavailable",
        };
        Self {
            id,
            glyph,
            title,
            short_title: title,
            detail,
            value,
            action: None,
        }
    }

    fn short_title(self, short_title: &'static str) -> Self {
        Self {
            short_title,
            ..self
        }
    }

    fn detail(self, detail: &'a str) -> Self {
        Self { detail, ..self }
    }

    fn action(mut self, action: ShadeAction) -> Self {
        self.action = Some(action);
        self
    }
}

#[derive(Clone, Copy)]
struct Level {
    id: &'static str,
    glyph: Glyph,
    title: &'static str,
    short_title: &'static str,
    value: u8,
}

struct Group<'a> {
    title: &'static str,
    levels: [Option<Level>; 2],
    tiles: Vec<Option<Tile<'a>>>,
}

impl Group<'_> {
    fn is_empty(&self) -> bool {
        self.tiles.iter().all(Option::is_none) && self.levels.iter().all(Option::is_none)
    }

    fn tile_count(&self) -> usize {
        self.tiles.iter().flatten().count()
    }

    fn rows_height(&self, columns: usize, metrics: Metrics) -> f32 {
        let tiles = self.tile_count().div_ceil(columns);
        let levels = self.levels.iter().flatten().count();
        let gaps = (tiles + usize::from(levels > 0)).saturating_sub(1);
        tiles as f32 * metrics.tile_h + metrics.level_height(levels) + gaps as f32 * metrics.row_gap
    }
}

fn connectivity_group(ctx: &SceneContext) -> Group<'_> {
    let status = &ctx.shade;
    let extra = &status.extras;
    let mut wifi = Tile::new("wifi", Glyph::Wifi, "Wi-Fi", status.wifi_radio);
    if wifi.value == Some(true) {
        if let Some(name) = status
            .wifi_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
        {
            wifi = wifi.detail(name);
        } else if ctx.wifi_connected {
            wifi = wifi.detail("Connected");
        }
    }
    let mobile = status.mobile_radio.map(|on| {
        let tile = Tile::new("lte", Glyph::Cellular, "Mobile data", Some(on)).short_title("Mobile");
        if on && ctx.lte_connected {
            tile.detail("Connected")
        } else {
            tile
        }
    });
    let bluetooth = Tile::new(
        "bluetooth",
        Glyph::Bluetooth,
        "Bluetooth",
        status.bluetooth_available.then_some(ctx.bluetooth_enabled),
    )
    .short_title("BT");
    let wifi_details = status.wifi_radio.is_some() && (extra.wifi || extra.host_wifi);
    let bt_details = status.bluetooth_available && (extra.bluetooth || extra.host_bluetooth);
    let mut tiles = Vec::new();
    if wifi_details {
        tiles.push(Some(
            Tile::new("", Glyph::Wifi, "Wi-Fi Networks", Some(false))
                .short_title("Networks")
                .detail(wifi.detail)
                .action(ShadeAction::Show(ShadePage::Wifi)),
        ));
        wifi.title = "Wi-Fi Radio";
        wifi.detail = if wifi.value == Some(true) {
            "On"
        } else {
            "Off"
        };
        wifi.short_title = "Wi-Fi Radio";
    }
    let mut bluetooth = bluetooth;
    if bt_details {
        tiles.push(Some(
            Tile::new("", Glyph::Bluetooth, "Bluetooth Devices", Some(false))
                .short_title("BT Devices")
                .detail(
                    extra
                        .bluetooth_name
                        .as_deref()
                        .filter(|_| ctx.bluetooth_enabled)
                        .unwrap_or("Paired devices"),
                )
                .action(ShadeAction::Show(ShadePage::Bluetooth)),
        ));
        bluetooth.title = "Bluetooth Radio";
        bluetooth.short_title = "BT Radio";
    }
    tiles.extend([Some(wifi), Some(bluetooth), mobile]);
    Group {
        title: "Connectivity",
        levels: [None; 2],
        tiles,
    }
}

fn display_group(ctx: &SceneContext) -> Group<'_> {
    let status = &ctx.shade;
    Group {
        title: "Display & Sound",
        levels: [
            status.brightness_available.then_some(Level {
                id: "brightness",
                title: "Brightness",
                short_title: "Bright.",
                glyph: Glyph::Sun,
                value: ctx.brightness_pct,
            }),
            status.volume_available.then_some(Level {
                id: "volume",
                title: "Volume",
                short_title: "Volume",
                glyph: Glyph::Volume,
                value: ctx.volume_pct,
            }),
        ],
        tiles: vec![
            status.extras.muted.map(|muted| {
                Tile::new(
                    "",
                    Glyph::Volume,
                    if muted { "Unmute" } else { "Mute" },
                    Some(muted),
                )
                .action(ShadeAction::Mute)
            }),
            (status.volume_available && (status.extras.audio || status.extras.host_audio)).then(
                || {
                    Tile::new("", Glyph::Volume, "Audio Output", Some(false))
                        .short_title("Output")
                        .detail("Choose device")
                        .action(ShadeAction::Show(ShadePage::Audio))
                },
            ),
            status.night_light_available.then_some(Tile::new(
                "night_light",
                Glyph::Moon,
                "Night light",
                Some(ctx.night_light),
            )),
            status.quiet.map(|on| {
                let tile = Tile::new("quiet", Glyph::Bell, "Do not disturb", Some(on))
                    .short_title("Quiet");
                if status.extras.notifications {
                    tile.action(ShadeAction::Quiet)
                } else {
                    tile
                }
            }),
            None,
        ],
    }
}

fn device_group(ctx: &SceneContext) -> Group<'_> {
    let status = &ctx.shade;
    Group {
        title: "Device",
        levels: [None; 2],
        tiles: vec![
            status.auto_rotate_available.then_some(
                Tile::new(
                    "autorotate",
                    Glyph::Rotate,
                    "Auto-rotate",
                    Some(!ctx.rotation_locked),
                )
                .short_title("Rotate"),
            ),
            status.power_profile.as_deref().map(|profile| {
                Tile::new(
                    "profile",
                    Glyph::Profile,
                    "Power profile",
                    Some(profile != "Balanced"),
                )
                .short_title("Profile")
                .detail(profile)
            }),
            ctx.power_menu.is_none().then_some(
                Tile::new(
                    "tablet_mode",
                    Glyph::Desktop,
                    "Desktop mode",
                    Some(!ctx.tablet_mode),
                )
                .short_title("Desktop"),
            ),
            ctx.power_menu.is_some().then(|| {
                let title = if ctx.tablet_mode {
                    "Desk Mode"
                } else {
                    "Tablet Mode"
                };
                Tile::new("desk_mode", Glyph::Desktop, title, Some(false))
            }),
            ctx.settings.is_some().then(|| {
                Tile::new("", Glyph::Desktop, "Settings", Some(false)).action(ShadeAction::Settings)
            }),
            status.extras.has_more().then(|| {
                Tile::new("", Glyph::Desktop, "More", Some(false))
                    .detail("Media and tools")
                    .action(ShadeAction::Show(ShadePage::More))
            }),
        ],
    }
}

#[derive(Clone, Copy)]
struct Metrics {
    heading_h: f32,
    group_gap: f32,
    row_gap: f32,
    tile_h: f32,
    level_h: f32,
}

impl Metrics {
    const COMPACT: Self = Self {
        heading_h: 18.0,
        group_gap: 10.0,
        row_gap: 6.0,
        tile_h: TOUCH_SIZE,
        level_h: 66.0,
    };
    const ROOMY: Self = Self {
        heading_h: 31.0,
        group_gap: 27.0,
        row_gap: 12.0,
        tile_h: 70.0,
        level_h: 148.0,
    };
    const DENSE: Self = Self {
        heading_h: 13.0,
        group_gap: 7.0,
        row_gap: 5.0,
        tile_h: TOUCH_SIZE,
        level_h: 48.0,
    };

    fn level_height(self, count: usize) -> f32 {
        match count {
            0 => 0.0,
            1 => f32::midpoint(self.tile_h, self.level_h),
            _ => self.level_h,
        }
    }

    fn fit(panes: [&[&Group<'_>]; 2], columns: usize, available: f32) -> Self {
        let compact = pane_height(panes, columns, Self::COMPACT);
        if available < compact {
            // Use less space between controls before shrinking their hit targets.
            let dense = pane_height(panes, columns, Self::DENSE);
            return if available >= dense {
                Self::DENSE
            } else {
                Self::DENSE.scale((available / dense).max(0.0))
            };
        }
        let roomy = pane_height(panes, columns, Self::ROOMY);
        let t = ((available - compact) / (roomy - compact).max(1.0)).min(1.0);
        let lerp = |a, b| a + (b - a) * t;
        Self {
            heading_h: lerp(Self::COMPACT.heading_h, Self::ROOMY.heading_h),
            group_gap: lerp(Self::COMPACT.group_gap, Self::ROOMY.group_gap),
            row_gap: lerp(Self::COMPACT.row_gap, Self::ROOMY.row_gap),
            tile_h: lerp(Self::COMPACT.tile_h, Self::ROOMY.tile_h),
            level_h: lerp(Self::COMPACT.level_h, Self::ROOMY.level_h),
        }
    }

    fn scale(self, scale: f32) -> Self {
        Self {
            heading_h: self.heading_h * scale,
            group_gap: self.group_gap * scale,
            row_gap: self.row_gap * scale,
            tile_h: self.tile_h * scale,
            level_h: self.level_h * scale,
        }
    }
}

fn pane_height(panes: [&[&Group<'_>]; 2], columns: usize, metrics: Metrics) -> f32 {
    panes
        .into_iter()
        .map(|pane| {
            pane.iter()
                .map(|group| group.rows_height(columns, metrics))
                .filter(|height| *height > 0.0)
                .map(|height| height + metrics.heading_h + metrics.group_gap)
                .sum::<f32>()
                - metrics.group_gap
        })
        .fold(0.0, f32::max)
}

struct ShadeLayout {
    x: f32,
    width: f32,
    header_y: f32,
    header_h: f32,
    body_y: f32,
    footer_y: f32,
    pane_w: f32,
    pane_gap: f32,
    columns: usize,
    metrics: Metrics,
}

impl ShadeLayout {
    fn new(width: f32, height: f32, split: bool, panes: [&[&Group<'_>]; 2], progress: f32) -> Self {
        let inset = (width * 0.042).clamp(12.0, 32.0).min(width * 0.12);
        let frame_w = (width - inset * 2.0).min(if split { 1000.0 } else { 556.0 });
        let pane_gap = (frame_w * 0.045).clamp(14.0, 30.0);
        let pane_w = if split {
            (frame_w - pane_gap) * 0.5
        } else {
            frame_w
        };
        let top = (height * 0.045).clamp(12.0, 44.0).min(height * 0.12);
        let header_h = if height >= 540.0 {
            60.0_f32
        } else {
            TOUCH_SIZE
        }
        .min(height * 0.25);
        let header_gap = 10.0_f32.min(height * 0.055);
        let footer_y = height - 46.0_f32.min(height * 0.16);
        let available = footer_y - top - header_h - header_gap;
        let columns = if pane_w >= 342.0
            || (pane_w >= TOUCH_SIZE * 3.0 + 2.0 * Metrics::COMPACT.row_gap
                && available < pane_height(panes, 2, Metrics::COMPACT))
        {
            3
        } else {
            2
        };
        let metrics = Metrics::fit(panes, columns, available);
        let spare = (available - pane_height(panes, columns, metrics)).max(0.0);
        let slide = -(1.0 - (progress / 0.86).min(1.0)) * 34.0_f32.min(height * 0.075);
        let header_y = top + (spare * 0.25).min(92.0) + slide;
        Self {
            x: (width - frame_w) * 0.5,
            width: frame_w,
            header_y,
            header_h,
            body_y: header_y + header_h + header_gap,
            footer_y: footer_y + slide,
            pane_w,
            pane_gap,
            columns,
            metrics,
        }
    }

    fn draw_header(&self, out: &mut SceneOutput, style: &ShadeStyle<'_>, ctx: &SceneContext) {
        let end = self.x + self.width;
        let power_h = TOUCH_SIZE.min(self.header_h).min(self.width * 0.23);
        let power_w = if self.width >= 460.0 { 88.0 } else { power_h };
        let labeled_power = power_w > power_h;
        let battery_end = if ctx.power_menu.is_some() {
            let rect = HitRect::new(
                end - power_w,
                self.header_y + (self.header_h - power_h) * 0.5,
                power_w,
                power_h,
            );
            style.surface(out, rect, false, true);
            let cy = rect.y + rect.h * 0.5;
            draw(
                &mut out.draw_list,
                Glyph::Power,
                rect.x + if labeled_power { 22.0 } else { power_w * 0.5 },
                cy,
                22.0_f32.min(power_h * 0.55),
                style.foreground(),
            );
            if labeled_power {
                out.draw_list.push_text(
                    rect.x + 40.0,
                    cy - 4.0,
                    "Power",
                    1,
                    style.foreground(),
                    TextAlign::Left,
                );
            }
            out.push_hit(rect, HitTarget::Power(PowerInput::Open));
            rect.x - 12.0_f32.min(self.width * 0.035)
        } else {
            end
        };
        let battery_space = 58.0_f32.min(self.width * 0.27);
        let battery_y = self.header_y + self.header_h * 0.5;
        let battery_pct = ctx.battery_pct.min(100);
        let battery_size = 21.0_f32.min(battery_space * 0.43).min(self.header_h);
        draw_battery(
            &mut out.draw_list,
            battery_end - battery_space + battery_size * 0.5,
            battery_y,
            battery_size,
            battery_pct,
            style.foreground(),
        );
        out.draw_list.push_text(
            battery_end,
            battery_y - 4.0,
            font::fit_text(&format!("{battery_pct}%"), 1, battery_space * 0.56),
            1,
            style.foreground(),
            TextAlign::Right,
        );
        let label_w = battery_end - battery_space - self.x - 8.0;
        let title = "Quick Settings";
        let scale = if self.header_h >= 58.0 { 4 } else { 3 };
        let (title, scale) = fit_label(title, scale, label_w);
        out.draw_list.push_text(
            self.x,
            self.header_y + 1.0,
            title,
            scale,
            style.foreground(),
            TextAlign::Left,
        );
        let date = ctx.shade.date.trim();
        let time_and_date = if date.is_empty() {
            ctx.clock_text.clone()
        } else {
            format!("{}  -  {date}", ctx.clock_text)
        };
        if self.header_h >= 29.0 {
            out.draw_list.push_text(
                self.x,
                self.header_y + self.header_h - 15.0,
                font::fit_text(&time_and_date, 1, label_w),
                1,
                style.muted(),
                TextAlign::Left,
            );
        }
    }

    fn draw_pane(
        &self,
        out: &mut SceneOutput,
        style: &ShadeStyle<'_>,
        index: usize,
        groups: &[&Group<'_>],
    ) {
        let x = self.x + index as f32 * (self.pane_w + self.pane_gap);
        let mut y = self.body_y;
        let metrics = self.metrics;
        let gap = metrics.row_gap.min(self.pane_w * 0.06);
        for group in groups {
            let mut count = group.tile_count();
            let levels = group.levels.iter().flatten();
            let level_count = levels.clone().count();
            if count == 0 && level_count == 0 {
                continue;
            }
            if metrics.heading_h >= 9.0 {
                let scale = if metrics.heading_h >= 25.0 { 2 } else { 1 };
                out.draw_list.push_text(
                    x + 2.0,
                    y,
                    font::fit_text(group.title, scale, self.pane_w - 4.0),
                    scale,
                    style.muted(),
                    TextAlign::Left,
                );
            }
            y += metrics.heading_h;
            if level_count > 0 {
                let h = metrics.level_height(level_count);
                for (&level, rect) in levels.zip(row(x, y, self.pane_w, h, gap, level_count)) {
                    style.level(out, rect, level);
                }
                y += h;
                if count > 0 {
                    y += metrics.row_gap;
                }
            }
            let mut tiles = group.tiles.iter().flatten();
            while count > 0 {
                let columns = count.min(self.columns);
                for (rect, tile) in
                    row(x, y, self.pane_w, metrics.tile_h, gap, columns).zip(tiles.by_ref())
                {
                    style.tile(out, rect, tile);
                }
                count -= columns;
                y += metrics.tile_h;
                if count > 0 {
                    y += metrics.row_gap;
                }
            }
            y += metrics.group_gap;
        }
    }

    fn draw_feedback(&self, out: &mut SceneOutput, style: &ShadeStyle<'_>, feedback: Option<&str>) {
        if let Some(feedback) = feedback {
            out.draw_list.push_text(
                self.x,
                self.footer_y + 4.0,
                font::fit_text(feedback, 1, self.width),
                1,
                style.palette.error.with_alpha(style.progress),
                TextAlign::Left,
            );
        }
    }
}

fn row(
    x: f32,
    y: f32,
    width: f32,
    h: f32,
    gap: f32,
    columns: usize,
) -> impl Iterator<Item = HitRect> {
    let w = (width - gap * (columns - 1) as f32) / columns as f32;
    (0..columns).map(move |i| HitRect::new(x + i as f32 * (w + gap), y, w, h))
}

struct ShadeStyle<'a> {
    palette: &'a ThemePalette,
    progress: f32,
}

impl ShadeStyle<'_> {
    fn foreground(&self) -> Rgba {
        self.palette.foreground.with_alpha(self.progress)
    }

    fn muted(&self) -> Rgba {
        self.palette.muted.with_alpha(self.progress)
    }

    fn surface(&self, out: &mut SceneOutput, rect: HitRect, active: bool, enabled: bool) {
        let (fill, alpha) = if active {
            (self.palette.accent, 0.97)
        } else {
            (self.palette.foreground, if enabled { 0.11 } else { 0.065 })
        };
        let radius = (rect.h * 0.33).min(21.0).min(rect.w * 0.25);
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
                self.palette.foreground.with_alpha(0.10 * self.progress),
                1.0,
                0.0,
            ),
        );
    }

    fn tile(&self, out: &mut SceneOutput, rect: HitRect, tile: &Tile<'_>) {
        let active = tile.value == Some(true);
        let enabled = tile.value.is_some();
        self.surface(out, rect, active, enabled);
        let ink = if active {
            self.palette.background
        } else {
            self.palette.foreground
        }
        .with_alpha(self.progress * if enabled { 1.0 } else { 0.68 });
        let face = Face {
            title: tile.title,
            short_title: tile.short_title,
            detail: tile.detail,
            glyph: tile.glyph,
            ink,
        };
        draw_face(
            &mut out.draw_list,
            rect,
            face,
            rect.w >= 136.0 || rect.h < TOUCH_SIZE,
        );
        if enabled {
            out.push_hit(
                rect,
                tile.action.clone().map_or(
                    HitTarget::ShadeToggle { id: tile.id },
                    HitTarget::ShadeAction,
                ),
            );
        }
    }

    fn level(&self, out: &mut SceneOutput, rect: HitRect, level: Level) {
        self.surface(out, rect, false, true);
        let inset = 10.0_f32.min(rect.h * 0.14).min(rect.w * 0.10);
        let track_w = (rect.w * 0.11).clamp(12.0, 21.0).min(rect.w * 0.22);
        let track = HitRect::new(
            rect.x + rect.w - inset - track_w,
            rect.y + inset,
            track_w,
            rect.h - inset * 2.0,
        );
        let radius = track.w.min(track.h) * 0.5;
        out.draw_list.push_rect(RoundedRect::filled(
            track.x,
            track.y,
            track.w,
            track.h,
            radius,
            self.palette.foreground.with_alpha(self.progress * 0.12),
        ));
        let value = level.value.min(100);
        let fill = track.h * f32::from(value) / 100.0;
        if fill > 0.0 {
            out.draw_list.push_rect(RoundedRect::filled(
                track.x,
                track.y + track.h - fill,
                track.w,
                fill,
                radius.min(fill * 0.5),
                self.palette.accent.with_alpha(self.progress),
            ));
        }
        // Wayland maps the whole card from 100% at the top to 0% at the bottom.
        out.push_hit(rect, HitTarget::ShadeSlider { id: level.id, rect });
        let detail = format!("{value}%");
        let label_area = HitRect::new(rect.x, rect.y, track.x - rect.x, rect.h);
        let face = Face {
            title: level.title,
            short_title: level.short_title,
            detail: &detail,
            glyph: level.glyph,
            ink: self.foreground(),
        };
        draw_face(&mut out.draw_list, label_area, face, label_area.w >= 136.0);
    }
}

fn draw_details(
    out: &mut SceneOutput,
    style: &ShadeStyle<'_>,
    ctx: &SceneContext,
    width: f32,
    height: f32,
) {
    let status = &ctx.shade;
    let page = status.page;
    let w = (width * 0.92).min(580.0);
    let x = (width - w) * 0.5;
    let start = 20.0;
    let h = 52.0;
    let step = h + 8.0;
    let button = |title, glyph, action| Tile::new("", glyph, title, Some(false)).action(action);
    style.tile(
        out,
        HitRect::new(x, start, 76.0, h),
        &button("Back", Glyph::Desktop, ShadeAction::Show(ShadePage::Main)),
    );
    out.draw_list.push_text(
        x + 88.0,
        start + 17.0,
        font::fit_text(page.title(), 2, w - 90.0),
        2,
        style.foreground(),
        TextAlign::Left,
    );
    let mut y = start + step;
    if page == ShadePage::More {
        if let Some(track) = &status.extras.media {
            out.draw_list.push_text(
                x,
                y + 5.0,
                font::fit_text(track, 1, w),
                1,
                style.foreground(),
                TextAlign::Left,
            );
            y += 25.0;
            for (rect, (title, action)) in row(x, y, w, h, 8.0, 3).zip([
                ("Previous", ShadeAction::Previous),
                ("Play/Pause", ShadeAction::PlayPause),
                ("Next", ShadeAction::Next),
            ]) {
                style.tile(out, rect, &button(title, Glyph::Volume, action));
            }
            y += step;
        }
        let tools = [
            status.extras.screenshot.then_some((
                "Screenshot",
                Glyph::Desktop,
                ShadeAction::Screenshot,
            )),
            status.extras.notifications.then_some((
                "Notifications",
                Glyph::Bell,
                ShadeAction::Notifications,
            )),
        ];
        let count = tools.iter().flatten().count();
        if count > 0 {
            for (rect, (title, glyph, action)) in
                row(x, y, w, h, 8.0, count).zip(tools.into_iter().flatten())
            {
                style.tile(out, rect, &button(title, glyph, action));
            }
        }
    } else {
        draw_choices(out, style, ctx, (x, y, w, height));
    }
    if let Some(message) = status
        .feedback
        .as_deref()
        .or(status.pending.map(|_| "Working..."))
    {
        out.draw_list.push_text(
            x,
            height - 41.0,
            font::fit_text(message, 1, w),
            1,
            style.foreground(),
            TextAlign::Left,
        );
    }
}

fn draw_choices(
    out: &mut SceneOutput,
    style: &ShadeStyle<'_>,
    ctx: &SceneContext,
    (x, mut y, w, height): (f32, f32, f32, f32),
) {
    let status = &ctx.shade;
    let page = status.page;
    let h = 44.0;
    let step = h + 6.0;
    let glyph = match page {
        ShadePage::Wifi => Glyph::Wifi,
        ShadePage::Bluetooth => Glyph::Bluetooth,
        _ => Glyph::Volume,
    };
    let host_available = status.extras.host_panel(page);
    if host_available {
        let host = Tile::new("", glyph, "Open Omarchy Settings", Some(false))
            .short_title("Omarchy Settings")
            .action(ShadeAction::Host(page));
        style.tile(out, HitRect::new(x, y, w, h), &host);
        y += step;
    }
    if status.choices.is_empty()
        || (page == ShadePage::Wifi && status.wifi_radio == Some(false))
        || (page == ShadePage::Bluetooth && !ctx.bluetooth_enabled)
    {
        let message = match page {
            _ if status.pending.is_some() => "Loading devices...",
            ShadePage::Wifi if status.wifi_radio == Some(false) => {
                "Wi-Fi is off. Turn it on in Quick Settings."
            }
            ShadePage::Wifi => "No saved Wi-Fi connections available.",
            ShadePage::Bluetooth => "No paired devices available. Check that Bluetooth is on.",
            _ => "No audio outputs available.",
        };
        out.draw_list.push_text(
            x,
            y + 8.0,
            font::fit_text(message, 1, w),
            1,
            style.muted(),
            TextAlign::Left,
        );
        return;
    }
    let capacity = ((height - y - 46.0) / step).max(0.0) as usize;
    let paged = status.choices.len() > capacity;
    let count = if paged {
        capacity.saturating_sub(1)
    } else {
        capacity
    };
    if count == 0 {
        return;
    }
    let offset = status
        .page_offset
        .min(status.choices.len().saturating_sub(1));
    for choice in status.choices.iter().skip(offset).take(count) {
        let rect = HitRect::new(x, y, w, h);
        style.choice(out, rect, glyph, choice, status.pending.is_none());
        if !choice.active && status.pending.is_none() {
            out.push_hit(
                rect,
                HitTarget::ShadeAction(ShadeAction::Select(page, choice.id.clone())),
            );
        }
        y += step;
    }
    if paged {
        for (rect, (title, destination)) in row(x, y, w, h, 8.0, 2).zip([
            (
                "Previous",
                (offset > 0).then(|| offset.saturating_sub(count)),
            ),
            (
                "Next",
                (offset + count < status.choices.len()).then_some(offset + count),
            ),
        ]) {
            let mut tile = Tile::new("", Glyph::Desktop, title, destination.map(|_| false));
            if let Some(destination) = destination {
                tile = tile.action(ShadeAction::Page(destination));
            }
            style.tile(out, rect, &tile);
        }
    }
}

impl ShadeStyle<'_> {
    fn choice(
        &self,
        out: &mut SceneOutput,
        rect: HitRect,
        glyph: Glyph,
        choice: &ShadeChoice,
        enabled: bool,
    ) {
        self.surface(out, rect, choice.active, enabled);
        let ink = if choice.active {
            self.palette.background.with_alpha(self.progress)
        } else {
            self.foreground()
        };
        draw(
            &mut out.draw_list,
            glyph,
            rect.x + 22.0,
            rect.y + rect.h * 0.5,
            21.0,
            ink,
        );
        let label = if choice.active {
            format!("{} (active)", choice.label)
        } else {
            choice.label.clone()
        };
        out.draw_list.push_text(
            rect.x + 42.0,
            rect.y + 16.0,
            font::fit_text(&label, 1, rect.w - 52.0),
            1,
            ink,
            TextAlign::Left,
        );
    }
}

#[derive(Clone, Copy)]
struct Face<'a> {
    title: &'static str,
    short_title: &'static str,
    detail: &'a str,
    glyph: Glyph,
    ink: Rgba,
}

fn draw_face(list: &mut DrawList, rect: HitRect, face: Face<'_>, inline: bool) {
    let glyph_size = 24.0_f32
        .min(rect.w * 0.28)
        .min(rect.h * if inline { 0.50 } else { 0.36 });
    let inset = (rect.w * 0.085).clamp(4.0, 14.0).min(rect.w * 0.16);
    let detail = rect.h >= if inline { 32.0 } else { TOUCH_SIZE };
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    let width = rect.w - inset * 2.0 - if inline { glyph_size + 6.0 } else { 0.0 };
    let max_scale = if rect.h >= 58.0 && width >= 106.0 {
        2
    } else {
        1
    };
    let (title, scale) = fit_label(face.title, max_scale, width);
    let title = if title == face.title || face.short_title == face.title {
        title
    } else {
        font::fit_text(face.short_title, scale, width)
    };
    let line_height = 7.0 * scale as f32 + 4.0;
    let text_height = line_height + if detail { 13.0 } else { 0.0 };
    let (glyph_x, glyph_y, x, y, align) = if inline {
        let glyph_x = rect.x + inset + glyph_size * 0.5;
        (
            glyph_x,
            cy,
            rect.x + inset + glyph_size + 6.0,
            cy - text_height * 0.5,
            TextAlign::Left,
        )
    } else {
        let top = cy - (glyph_size + 3.0 + text_height) * 0.5;
        (
            cx,
            top + glyph_size * 0.5,
            cx,
            top + glyph_size + 3.0,
            TextAlign::Center,
        )
    };
    draw(list, face.glyph, glyph_x, glyph_y, glyph_size, face.ink);
    if rect.h >= 21.0 && width > 0.0 {
        list.push_text(x, y, title, scale, face.ink, align);
        if detail {
            list.push_text(
                x,
                y + line_height + 2.0,
                font::fit_text(face.detail, 1, width),
                1,
                face.ink.with_alpha(face.ink.a * 0.80),
                align,
            );
        }
    }
}

fn fit_label(text: &str, max: u32, width: f32) -> (String, u32) {
    for scale in (2..=max).rev() {
        let fitted = font::fit_text(text, scale, width);
        if fitted == text {
            return (fitted, scale);
        }
    }
    (font::fit_text(text, 1, width), 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_scene_layout, DrawCommand, HitRegion, SceneKind, TextLabel};

    fn scene(width: f32, height: f32, ctx: &SceneContext) -> SceneOutput {
        build_scene_layout(
            SceneKind::Shade,
            width,
            height,
            &ThemePalette::default_dark(),
            ctx,
        )
    }

    fn label<'a>(scene: &'a SceneOutput, text: &str) -> Option<&'a TextLabel> {
        scene.draw_list.commands.iter().find_map(|cmd| match cmd {
            DrawCommand::Text(label) if label.text == text => Some(label),
            _ => None,
        })
    }

    fn control_id(hit: &HitRegion) -> Option<&'static str> {
        match hit.target {
            HitTarget::ShadeToggle { id } | HitTarget::ShadeSlider { id, .. } => Some(id),
            _ => None,
        }
    }

    fn control<'a>(scene: &'a SceneOutput, id: &str) -> &'a HitRegion {
        scene
            .hit_regions
            .iter()
            .find(|hit| control_id(hit) == Some(id))
            .unwrap()
    }

    #[test]
    fn daily_details_are_distinct_from_radio_toggles() {
        let mut ctx = SceneContext::default();
        ctx.shade.extras = ShadeExtras {
            host_bluetooth: true,
            wifi: true,
            audio: true,
            muted: Some(false),
            notifications: true,
            ..ShadeExtras::default()
        };
        for (w, h) in [
            (390.0, 518.0),
            (518.0, 390.0),
            (853.0, 1280.0),
            (1280.0, 853.0),
        ] {
            let output = scene(w, h, &ctx);
            for (page, toggle) in [
                (ShadePage::Wifi, "wifi"),
                (ShadePage::Bluetooth, "bluetooth"),
                (ShadePage::Audio, "volume"),
            ] {
                let target = HitTarget::ShadeAction(ShadeAction::Show(page));
                let rect = output
                    .hit_regions
                    .iter()
                    .find(|r| r.target == target)
                    .unwrap()
                    .rect;
                assert!(
                    rect.h >= TOUCH_SIZE - 0.01 && rect.w >= TOUCH_SIZE,
                    "{page:?} {rect:?} at {w}x{h}"
                );
                assert_eq!(
                    output.hit_test(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0),
                    Some(&target)
                );
                assert_ne!(rect, control(&output, toggle).rect);
            }
            assert!(output
                .hit_regions
                .iter()
                .any(|r| r.target == HitTarget::ShadeAction(ShadeAction::Mute)));
            assert!(output
                .hit_regions
                .iter()
                .any(|r| r.target == HitTarget::ShadeAction(ShadeAction::Quiet)));
        }
    }

    #[test]
    fn saved_connections_page_claims_only_inactive_rows_and_disallows_writes_when_busy() {
        let mut ctx = SceneContext::default();
        ctx.shade.page = ShadePage::Wifi;
        ctx.shade.extras.host_wifi = true;
        ctx.shade.choices = (0..16)
            .map(|i| ShadeChoice {
                id: i.to_string(),
                label: format!("Network {i}"),
                active: i == 0,
            })
            .collect();
        let actions = |ctx: &SceneContext| {
            scene(320.0, 480.0, ctx)
                .hit_regions
                .into_iter()
                .map(|hit| hit.target)
                .collect::<Vec<_>>()
        };
        let first = actions(&ctx);
        assert!(first.contains(&HitTarget::ShadeAction(ShadeAction::Host(ShadePage::Wifi))));
        assert!(!first.contains(&HitTarget::ShadeAction(ShadeAction::Select(
            ShadePage::Wifi,
            "0".into()
        ))));
        assert!(first
            .iter()
            .any(|hit| matches!(hit, HitTarget::ShadeAction(ShadeAction::Page(_)))));
        ctx.shade.pending = Some(1);
        assert!(!actions(&ctx)
            .iter()
            .any(|hit| matches!(hit, HitTarget::ShadeAction(ShadeAction::Select(_, _)))));
    }

    #[test]
    fn shade_groups_stack_in_portrait_and_fill_both_landscape_panes() {
        let ctx = SceneContext::default();
        let portrait = scene(390.0, 518.0, &ctx);
        let connectivity = label(&portrait, "Connectivity").unwrap();
        let display = label(&portrait, "Display & Sound").unwrap();
        let device = label(&portrait, "Device").unwrap();
        assert!(connectivity.y < display.y && display.y < device.y);
        assert!((connectivity.x - display.x).abs() < 0.01);
        assert!((display.x - device.x).abs() < 0.01);

        for (width, height) in [(518.0, 390.0), (1280.0, 853.0)] {
            let landscape = scene(width, height, &ctx);
            let connectivity = label(&landscape, "Connectivity").unwrap();
            let display = label(&landscape, "Display & Sound").unwrap();
            assert!((connectivity.y - display.y).abs() < 0.01);
            for id in [
                "wifi",
                "bluetooth",
                "lte",
                "autorotate",
                "profile",
                "tablet_mode",
            ] {
                let rect = control(&landscape, id).rect;
                assert!(rect.x + rect.w < width * 0.5, "{id} is in the left pane");
            }
            for id in ["brightness", "volume", "night_light", "quiet"] {
                let rect = control(&landscape, id).rect;
                assert!(rect.x > width * 0.5, "{id} is in the right pane");
            }
        }
    }

    #[test]
    fn shade_omits_optional_controls_and_expands_the_rest_of_each_row() {
        let ctx = SceneContext {
            shade: ShadeStatus {
                mobile_radio: None,
                auto_rotate_available: false,
                night_light_available: false,
                power_profile: None,
                ..ShadeStatus::default()
            },
            ..SceneContext::default()
        };
        for (width, height) in [(390.0, 518.0), (518.0, 390.0)] {
            let shade = scene(width, height, &ctx);
            let ids: Vec<_> = shade.hit_regions.iter().filter_map(control_id).collect();
            assert_eq!(ids.len(), 6);
            for id in ["lte", "autorotate", "night_light", "profile"] {
                assert!(!ids.contains(&id), "{id} should be absent");
            }
            for text in ["Mobile data", "Auto-rotate", "Night light", "Power profile"] {
                assert!(label(&shade, text).is_none());
            }
            let wifi = control(&shade, "wifi").rect;
            let bluetooth = control(&shade, "bluetooth").rect;
            let desktop = control(&shade, "tablet_mode").rect;
            assert!((wifi.y - bluetooth.y).abs() < 0.01);
            assert!((wifi.x - desktop.x).abs() < 0.01);
            assert!((bluetooth.x + bluetooth.w - desktop.x - desktop.w).abs() < 0.01);
            let brightness = control(&shade, "brightness").rect;
            let volume = control(&shade, "volume").rect;
            let quiet = control(&shade, "quiet").rect;
            assert!((brightness.x - quiet.x).abs() < 0.01);
            assert!((volume.x + volume.w - quiet.x - quiet.w).abs() < 0.01);

            let complete = scene(width, height, &SceneContext::default());
            assert!(
                label(&shade, "Do not disturb").is_some(),
                "packing lets the remaining control use its full label"
            );
            assert!(desktop.w >= control(&complete, "tablet_mode").rect.w);
            if width > height {
                assert!(desktop.y < control(&complete, "tablet_mode").rect.y);
            } else {
                assert!(wifi.w > control(&complete, "wifi").rect.w);
            }
        }
    }

    #[test]
    fn shade_core_controls_distinguish_unavailable_from_off() {
        let mut ctx = SceneContext {
            power_menu: Some(PowerMenu::Closed),
            shade: ShadeStatus {
                wifi_radio: None,
                bluetooth_available: false,
                mobile_radio: None,
                brightness_available: false,
                volume_available: false,
                auto_rotate_available: false,
                night_light_available: false,
                power_profile: None,
                quiet: None,
                ..ShadeStatus::default()
            },
            ..SceneContext::default()
        };
        let disabled = scene(390.0, 518.0, &ctx);
        assert!(disabled
            .hit_regions
            .iter()
            .all(|hit| matches!(control_id(hit), None | Some("desk_mode"))));
        for text in ["Wi-Fi", "Bluetooth", "Unavailable"] {
            assert!(label(&disabled, text).is_some(), "{text} stays visible");
        }
        for text in ["Display & Sound", "Brightness", "Volume"] {
            assert!(label(&disabled, text).is_none(), "{text} is omitted");
        }
        assert!(label(&disabled, "Home network").is_none(), "no stale SSID");
        assert!(disabled
            .hit_regions
            .iter()
            .any(|hit| hit.target == HitTarget::Power(PowerInput::Open)));

        ctx.shade.wifi_radio = Some(false);
        ctx.shade.bluetooth_available = true;
        ctx.bluetooth_enabled = false;
        ctx.shade.mobile_radio = Some(false);
        // Status refreshes can momentarily lag behind the radio toggle.
        ctx.lte_connected = true;
        let off = scene(390.0, 518.0, &ctx);
        for id in ["wifi", "bluetooth", "lte"] {
            let rect = control(&off, id).rect;
            let status = off.draw_list.commands.iter().find(|cmd| match cmd {
                DrawCommand::Text(text) if text.text == "Off" => rect.contains(text.x, text.y),
                _ => false,
            });
            assert!(status.is_some(), "{id} is labelled Off and can be enabled");
        }
        assert!(label(&off, "Connected").is_none());
    }

    #[test]
    fn shade_expands_the_remaining_slider_when_the_other_level_is_unavailable() {
        for (width, height) in [(390.0, 518.0), (518.0, 390.0), (1280.0, 853.0)] {
            let mut ctx = SceneContext::default();
            let both = scene(width, height, &ctx);
            for brightness_available in [true, false] {
                ctx.shade.brightness_available = brightness_available;
                ctx.shade.volume_available = !brightness_available;
                let single = scene(width, height, &ctx);
                let (present, missing, title) = if brightness_available {
                    ("brightness", "volume", "Volume")
                } else {
                    ("volume", "brightness", "Brightness")
                };
                assert!(single
                    .hit_regions
                    .iter()
                    .all(|hit| control_id(hit) != Some(missing)));
                assert!(label(&single, title).is_none());
                let hit = control(&single, present);
                let a = hit.rect;
                let night_light = control(&single, "night_light").rect;
                let quiet = control(&single, "quiet").rect;
                assert!(a.w > control(&both, present).rect.w);
                assert!((night_light.x - a.x).abs() < 0.01);
                assert!((quiet.x + quiet.w - a.x - a.w).abs() < 0.01);
                assert!(a.h >= TOUCH_SIZE && a.y + a.h < night_light.y);
                assert_eq!(
                    hit.target,
                    HitTarget::ShadeSlider {
                        id: present,
                        rect: a
                    }
                );
                assert_eq!(single.hit_test(a.x + a.w * 0.5, a.y), Some(&hit.target));
            }
        }
    }

    #[test]
    fn shade_puts_device_in_the_right_pane_when_display_and_sound_are_absent() {
        let ctx = SceneContext {
            shade: ShadeStatus {
                brightness_available: false,
                volume_available: false,
                night_light_available: false,
                quiet: None,
                ..ShadeStatus::default()
            },
            ..SceneContext::default()
        };
        let landscape = scene(518.0, 390.0, &ctx);
        assert!(label(&landscape, "Display & Sound").is_none());
        assert!(control(&landscape, "wifi").rect.x < 259.0);
        assert!(control(&landscape, "tablet_mode").rect.x > 259.0);
    }

    #[test]
    fn shade_touch_targets_are_in_bounds_and_do_not_overlap() {
        for (width, height) in [
            (320.0, 480.0),
            (390.0, 518.0),
            (853.0, 1280.0),
            (480.0, 270.0),
            (518.0, 390.0),
            (1280.0, 853.0),
        ] {
            for power_menu in [None, Some(PowerMenu::Closed)] {
                let ctx = SceneContext {
                    power_menu,
                    ..SceneContext::default()
                };
                let shade = scene(width, height, &ctx);
                let hits: Vec<_> = shade
                    .hit_regions
                    .iter()
                    .filter(|hit| hit.target != HitTarget::DismissOverlay)
                    .collect();
                assert_eq!(hits.len(), 10 + usize::from(power_menu.is_some()));

                for (i, hit) in hits.iter().enumerate() {
                    let a = hit.rect;
                    assert!(
                        a.w >= TOUCH_SIZE - 0.01 && a.h >= TOUCH_SIZE - 0.01,
                        "{hit:?} at {width}×{height}"
                    );
                    assert!(a.x >= 0.0 && a.y >= 0.0);
                    assert!(a.x + a.w <= width && a.y + a.h <= height - 30.0);
                    for (x, y) in [
                        (a.x + 2.0, a.y + 2.0),
                        (a.x + a.w * 0.5, a.y + a.h * 0.5),
                        (a.x + a.w - 2.0, a.y + a.h - 2.0),
                    ] {
                        assert_eq!(shade.hit_test(x, y), Some(&hit.target));
                    }
                    for other in &hits[i + 1..] {
                        let b = other.rect;
                        assert!(
                            a.x + a.w < b.x
                                || b.x + b.w < a.x
                                || a.y + a.h < b.y
                                || b.y + b.h < a.y,
                            "{hit:?} overlaps {other:?}",
                        );
                    }
                    if let HitTarget::ShadeSlider { rect, .. } = hit.target {
                        assert_eq!(rect, a, "vertical input maps the full card height");
                        let cx = a.x + a.w * 0.5;
                        assert_eq!(shade.hit_test(cx, a.y), Some(&hit.target));
                        assert_eq!(shade.hit_test(cx, a.y + a.h), Some(&hit.target));
                    }
                }
            }
        }
    }
}
