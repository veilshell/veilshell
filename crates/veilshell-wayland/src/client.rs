//! Multi-surface event-driven `wlr-layer-shell` `Wayland` runtime powered by `calloop`.

use std::fmt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{
    CompositorHandler, CompositorState, FrameCallbackData, Region,
};
use smithay_client_toolkit::output::{OutputHandler, OutputInfo, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{self, Channel, Event as ChannelEvent};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_callback, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch,
};
use smithay_client_toolkit::reexports::client::{Connection, Proxy, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{
    PointerData, PointerEvent, PointerEventKind, PointerHandler,
};
use smithay_client_toolkit::seat::touch::{TouchData, TouchHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor as WlrAnchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
#[path = "launcher_input.rs"]
mod launcher_input;
use launcher_input::LauncherInput;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};

use veilshell_core::{
    classify_bottom_pill, GestureAction, GestureSample, GestureThresholds, Orientation, OskEmit,
    OskState, ShiftState, Spring, SpringParams, ThemePalette,
};
use veilshell_render::power::{PowerAction, PowerInput, PowerMenu};
use veilshell_render::{
    bottom_pill_bounds, build_scene, build_scene_layout, build_shade_over_home,
    build_top_strip_scene, drawer_max_scroll, home_reveal_travel, keyboard_button_bounds,
    recents_stride, shade_reveal_travel, tablet_shade_is_split, tablet_shade_scale,
    top_handle_bounds, ActiveRenderer, DrawList, HitRect, HitTarget, SceneContext, SceneKind,
    SceneOutput, ShadeAction, ShadeChoice, ShadeExtras, ShadePage, TabletOverviewOrigin,
    TabletShadePage, WallpaperImage, TABLET_STATUS_HEIGHT,
};

use crate::capture::ToplevelCaptureState;
#[cfg(feature = "wgpu")]
use crate::gpu::{gpu_attempt, init_renderer, WgpuWaylandSurface};
use crate::motion::{
    edge_vertical, horizontal, input_event_time, slider_pct_for_y, snap_at_rest, spring_to,
    vertical, DragMotion, DragVelocity, DrawerContact, SliderDrag,
};
use crate::osk::{OskContact, OskWaylandState};
use crate::overlay_gaps::{self, GapEdge, GapRefresh};
use crate::scale::{FractionalScaleGlobals, SurfaceFrame, SurfaceScaling};
use crate::tablet_nav::{BottomContact, NavOrigin, NavRelease};
use crate::{LayerRole, LayerSurfaceSpec, WaylandRuntimeConfig, TABLET_BOTTOM_DEPTH};

/// Errors raised while connecting to or running the `Wayland` layer-shell client.
#[derive(Debug)]
pub enum WaylandError {
    /// Failed to connect to the `Wayland` compositor socket.
    Connect(String),
    /// Required `Wayland` protocol global (`zwlr_layer_shell_v1`, `wl_shm`, etc.) is unavailable.
    MissingGlobal(String),
    /// Error during event dispatch, output selection, or buffer allocation.
    Runtime(String),
}

impl fmt::Display for WaylandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(msg) => write!(f, "wayland connect error: {msg}"),
            Self::MissingGlobal(msg) => write!(f, "missing wayland global: {msg}"),
            Self::Runtime(msg) => write!(f, "wayland runtime error: {msg}"),
        }
    }
}

impl std::error::Error for WaylandError {}

/// Commands sent asynchronously into the running `Wayland` daemon event loop.
#[derive(Debug)]
pub enum DaemonCommand {
    /// Switch the presented shell scene (`Pill` dismisses full-screen overlays).
    ShowScene(SceneKind),
    /// Request the app keyboard and report when there is no usable input path.
    ShowKeyboard(mpsc::Sender<Result<(), &'static str>>),
    /// Show (`true`, Tablet Mode) or completely hide (`false`, Laptop Mode) all surfaces.
    SetVisible(bool),
    /// Replace the dynamic scene context (apps, windows, hardware state, clock).
    UpdateContext(Box<SceneContext>),
    /// Push live hardware state deltas (keyboard presence, accelerometer orientation).
    HardwareUpdate {
        keyboard_attached: Option<bool>,
        orientation: Option<Orientation>,
    },
    /// Apply a freshly loaded color palette and redraw active surfaces.
    UpdatePalette(ThemePalette),
    UpdateTabletStatus(veilshell_render::TabletStatus),
    /// Upload and pre-blur a new static wallpaper on the GPU (or `None` to clear).
    SetWallpaper(Option<WallpaperImage>),
    /// Deliver the result of a session action without blocking Wayland dispatch.
    PowerFinished {
        action: PowerAction,
        success: bool,
    },
    /// Deliver an optional shade service result from a background worker.
    ShadeFinished {
        ticket: u64,
        result: Result<ShadeReply, String>,
    },
    /// Query live daemon runtime status for `veilctl status`.
    QueryStatus(mpsc::Sender<RuntimeStatus>),
    /// Forward a domain command (such as `mode ...`) to [`ShellActionHandler::on_custom_command`].
    Custom {
        cmd: String,
        reply: mpsc::Sender<String>,
    },
    /// Exit the event loop cleanly.
    Quit,
}

/// Live snapshot of the running `Wayland` shell surfaces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStatus {
    pub active_scene: SceneKind,
    pub visible: bool,
    pub output_name: Option<String>,
    pub width: u32,
    pub height: u32,
    pub renderer: String,
    pub standalone_layout: Option<&'static str>,
    pub tablet_shade_layout: Option<&'static str>,
    /// Full-screen touch navigation, including opening the Drawer, is enabled.
    pub tablet_navigation: bool,
    /// Currently armed top/bottom logical depths; `None` outside the opt-in profile.
    pub overlay_gaps: Option<(u32, u32)>,
}

/// Partial state changes from a Quick Settings service operation.
#[derive(Debug)]
pub enum ShadeReply {
    Discovered(ShadeExtras, Option<bool>),
    Choices(Vec<ShadeChoice>),
    AudioOutput(Vec<ShadeChoice>, Option<bool>),
    Muted(bool),
    Quiet(bool),
    Media(Option<String>),
    Dismiss,
}

/// Interactive UI events emitted by the `Wayland` shell surfaces.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellAction {
    Settings(veilshell_render::SettingsAction),
    /// Sent immediately before opening a scene so state (e.g. Hyprland windows) can refresh.
    RequestRefresh(SceneKind),
    /// Focus and raise a compositor window.
    FocusWindow(String),
    /// Request closing a compositor window.
    CloseWindow(String),
    /// Activate a Home entry, retaining its desktop-file ID for session policy.
    LaunchApp {
        id: String,
        exec: String,
    },
    /// Switch to the adjacent workspace (`+1` next, `-1` prev).
    SwitchWorkspace(i32),
    /// Toggle a Quick Settings tile (`wifi`, `bluetooth`, `lte`, `autorotate`, `tablet_mode`, `night_light`).
    ToggleSetting(&'static str),
    /// Navigate to or activate a Quick Settings detail or desktop tool.
    Shade(ShadeAction),
    /// Update a Quick Settings slider (`brightness`, `volume`) to `0..=100`.
    SetSlider {
        id: &'static str,
        pct: u8,
    },
    /// Notify that the active Home grid page changed.
    HomePageChanged(usize),
    /// Request an operation from the standalone session adapter.
    RequestPower(PowerAction),
}

/// Callback interface handling UI interactions in `veilshell-shell`.
pub trait ShellActionHandler {
    /// Respond to a [`ShellAction`], optionally mutating the live [`SceneContext`].
    fn on_action(&mut self, action: ShellAction, ctx: &mut SceneContext);

    /// Read-only native bounds for the optional desktop strips, in logical pixels.
    ///
    /// Called at a bounded rate only while the opt-in is visible with a named
    /// output and a touch seat. Implementations must use bounded IPC and return
    /// zero for unavailable/unsafe edges (both zero for conflicting native
    /// resize or touchscreen workspace gestures, or an unsupported backend).
    fn overlay_gap_depths(&self) -> (u32, u32) {
        (0, 0)
    }

    /// Supply a newest-first snapshot of running windows on this output only.
    /// Called once on entry (including a held preview), not on settle.
    fn refresh_tablet_overview(&mut self, _output: &str, ctx: &mut SceneContext) {
        ctx.recents_cards.clear();
    }

    /// Focus one existing window on `output`: `+1` older, `-1` newer in LTR.
    /// Return `true` only on a successful focus, so Home can safely dismiss.
    fn switch_tablet_app(
        &mut self,
        _output: &str,
        _direction: i32,
        _from_home: bool,
        _ctx: &SceneContext,
    ) -> bool {
        false
    }

    /// React to live hardware events (keyboard attach/detach or accelerometer orientation).
    ///
    /// Returning `Some(visible)` shows (`true`) or hides (`false`) the shell surfaces.
    fn on_hardware_update(
        &mut self,
        _keyboard_attached: Option<bool>,
        _orientation: Option<Orientation>,
        _ctx: &mut SceneContext,
    ) -> Option<bool> {
        None
    }

    /// Handle an IPC command routed through `veilctl`; returns `(reply_line, optional_visibility)`.
    fn on_custom_command(&mut self, _cmd: &str, _ctx: &mut SceneContext) -> (String, Option<bool>) {
        ("unsupported".to_owned(), None)
    }
}

#[derive(Clone, Copy, Default)]
struct NoopActionHandler;

impl ShellActionHandler for NoopActionHandler {
    fn on_action(&mut self, _action: ShellAction, _ctx: &mut SceneContext) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurfaceKind {
    Wallpaper,
    Pill,
    TopStrip,
    Overlay,
}

impl GapEdge {
    const fn slot(self) -> SurfaceKind {
        match self {
            Self::Top => SurfaceKind::TopStrip,
            Self::Bottom => SurfaceKind::Pill,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TabletShadeOrigin {
    App,
    Home,
}

#[derive(Clone, Copy)]
enum SceneRefresh {
    Now,
    OnOpen,
    Unchanged,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabletContact {
    None,
    Shade,
    Content,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragPolicy {
    Regular,
    GapGesture,
    ClickOnly,
}

struct ActiveDrag {
    touch_id: Option<i32>,
    policy: DragPolicy,
    surface: SurfaceKind,
    // Unlike a handle or content drag, this contact may only open Shade.
    tablet_origin: Option<TabletShadeOrigin>,
    // Record ownership at contact down so a mode switch can cancel it.
    tablet_contact: TabletContact,
    tablet_bottom: Option<BottomContact>,
    drawer: Option<DrawerContact>,
    start_x: f32,
    start_y: f32,
    last_x: f32,
    last_y: f32,
    max_travel: f32,
    handle_tap_allowed: bool,
    keyboard_button: Option<RegionRect>,
    overlay_tap_allowed: bool,
    down_target: Option<HitTarget>,
    recent_card: Option<String>,
    active_slider: Option<SliderDrag>,
    osk_contact: Option<OskContact>,
    motion: Option<DragMotion>,
    velocity: DragVelocity,
    start_time: Instant,
    protocol_start: u32,
}

impl ActiveDrag {
    fn is_tablet_navigation(&self) -> bool {
        self.tablet_bottom.is_some() || self.tablet_contact == TabletContact::Content
    }

    fn tablet_event_time(&self, time: u32) -> Option<Instant> {
        input_event_time(self.start_time, self.protocol_start, time)
    }

    fn uses_input_time(&self) -> bool {
        self.is_tablet_navigation() || self.is_gap_gesture()
    }

    fn is_gap_gesture(&self) -> bool {
        self.policy == DragPolicy::GapGesture
    }

    fn track_position(&mut self, x: f32, y: f32) -> (f32, f32) {
        self.last_x = x;
        self.last_y = y;
        if let Some(contact) = &mut self.osk_contact {
            contact.position(x, y);
        }
        let (dx, dy) = (x - self.start_x, y - self.start_y);
        self.max_travel = self.max_travel.max(dx.hypot(dy));
        (dx, dy)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegionRect(i32, i32, i32, i32);

impl RegionRect {
    fn contains(self, x: f32, y: f32) -> bool {
        let Self(left, top, width, height) = self;
        x >= left as f32
            && x < (left + width) as f32
            && y >= top as f32
            && y < (top + height) as f32
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EdgeHandle {
    Bottom,
    TabletBottom,
    Top,
    Gap(GapEdge),
    TabletStatus,
    Desk,
}

impl EdgeHandle {
    fn input_rects(self, size: (u32, u32), show_keyboard: bool) -> [Option<RegionRect>; 3] {
        if matches!(self, Self::Desk) {
            return veilshell_render::desk_panel_actions(size.0 as f32).map(|action| {
                let b = action.bounds;
                (b.w > 0.0).then_some(RegionRect(b.x as i32, 0, b.w as i32, b.h as i32))
            });
        }
        let keyboard = if show_keyboard && matches!(self, Self::Bottom | Self::TabletBottom) {
            keyboard_button_bounds(size.0 as f32, size.1 as f32)
                .map(|b| RegionRect(b.x as i32, b.y as i32, b.w as i32, b.h as i32))
        } else {
            None
        };
        [self.input_rect(size), keyboard, None]
    }

    fn bounds(self, width: f32, height: f32) -> HitRect {
        match self {
            Self::Bottom | Self::TabletBottom => bottom_pill_bounds(width, height),
            Self::Top | Self::TabletStatus | Self::Desk => top_handle_bounds(width, height),
            Self::Gap(_) => HitRect::new(0.0, 0.0, width, height),
        }
    }

    fn input_rect(self, (width, height): (u32, u32)) -> Option<RegionRect> {
        // Stable surface-local target; the animated handle does not move the input zone.
        if width == 0 || height == 0 {
            return None;
        }
        if matches!(self, Self::Gap(_)) {
            return full_width_band((width, height), height);
        }
        if self == Self::TabletStatus {
            return Some(RegionRect(
                0,
                0,
                i32::try_from(width).ok()?,
                i32::try_from(height.min(TABLET_STATUS_HEIGHT)).ok()?,
            ));
        }
        if self == Self::TabletBottom {
            let target = width.min(200);
            let depth = height.min(TABLET_BOTTOM_DEPTH);
            return Some(RegionRect(
                i32::try_from((width - target) / 2).ok()?,
                i32::try_from(height - depth).ok()?,
                i32::try_from(target).ok()?,
                i32::try_from(depth).ok()?,
            ));
        }
        let w = width as f32;
        let bounds = self.bounds(w, height as f32);
        let x0 = (bounds.x - 24.0).max(w * 0.04).floor() as i32;
        let x1 = (bounds.x + bounds.w + 24.0).min(w * 0.96).ceil() as i32;
        Some(RegionRect(x0, 0, x1 - x0, i32::try_from(height).ok()?))
    }

    fn allows_tap(self, width: u32, height: u32, x: f32, y: f32) -> bool {
        if matches!(self, Self::TabletStatus | Self::TabletBottom | Self::Gap(_)) {
            return false;
        }
        let b = self.bounds(width as f32, height as f32);
        // Allow a little finger jitter at the mark, but not in the swipe padding.
        HitRect::new(b.x - 4.0, b.y - 4.0, b.w + 8.0, b.h + 8.0).contains(x, y)
    }
}

fn full_width_band((width, height): (u32, u32), depth: u32) -> Option<RegionRect> {
    let depth = depth.min(height);
    (width > 0 && depth > 0).then_some(RegionRect(
        0,
        i32::try_from(height - depth).ok()?,
        i32::try_from(width).ok()?,
        i32::try_from(depth).ok()?,
    ))
}

fn handle_tap(allowed: bool, max_travel: f32) -> bool {
    allowed && max_travel < 8.0
}

fn tablet_navigation_available(visible: bool, ctx: &SceneContext) -> bool {
    visible && ctx.uses_touch_navigation()
}

fn gesture_input_allowed(touch_only: bool, touch_id: Option<i32>) -> bool {
    !touch_only || touch_id.is_some()
}

fn bottom_contact_at(
    origin: NavOrigin,
    viewport_h: u32,
    bounds: Option<RegionRect>,
    position: (f32, f32),
    touch_only: bool,
    touch_id: Option<i32>,
) -> Option<BottomContact> {
    (viewport_h > 0 && gesture_input_allowed(touch_only, touch_id))
        .then_some(bounds?)
        .filter(|rect| rect.contains(position.0, position.1))
        .map(|_| BottomContact::new(origin, viewport_h as f32))
}

fn tablet_home_vertical_motion(
    dx: f32,
    dy: f32,
    height: f32,
) -> Option<(DragMotion, Option<SceneKind>)> {
    if !vertical(dx, dy) {
        return None;
    }
    if dy < 0.0 {
        Some((
            DragMotion::drawer(0.0, height, true),
            Some(SceneKind::Drawer),
        ))
    } else {
        // Preserve the existing downward content dismissal. Starting this
        // motion owns the contact; reversing cannot turn it into a Drawer pull.
        Some((
            DragMotion::reveal(1.0, home_reveal_travel(height), -1.0, false),
            None,
        ))
    }
}

fn tablet_content_tap(
    layout: Option<&SceneOutput>,
    down: Option<&HitTarget>,
    (x, y): (f32, f32),
    max_travel: f32,
) -> bool {
    handle_tap(down.is_some(), max_travel)
        && layout.and_then(|layout| layout.hit_test(x, y)) == down
}

fn switch_tablet_app(
    handler: &mut dyn ShellActionHandler,
    output: Option<&str>,
    offset: i32,
    from_home: bool,
    ctx: &SceneContext,
) -> bool {
    let focused =
        output.is_some_and(|output| handler.switch_tablet_app(output, offset, from_home, ctx));
    from_home && focused
}

fn tablet_status_reveal(dx: f32, dy: f32, height: f32) -> Option<DragMotion> {
    (dy > 0.0 && edge_vertical(dx, dy)).then(|| DragMotion::tablet_shade(0.0, height, true, true))
}

fn tablet_shade_page_from_down(x: f32, width: f32, height: f32) -> TabletShadePage {
    if !tablet_shade_is_split(width, height) && x < width * 0.5 {
        TabletShadePage::Notifications
    } else {
        TabletShadePage::Controls
    }
}

struct HandleInputRegion {
    handle: EdgeHandle,
    region: Region,
    rects: [Option<RegionRect>; 3],
}

impl HandleInputRegion {
    fn new(
        handle: EdgeHandle,
        compositor: &CompositorState,
        surface: &wl_surface::WlSurface,
    ) -> Self {
        let region = Region::new(compositor).expect("compositor is already bound");
        // An empty region protects the transparent layer until its first draw.
        surface.set_input_region(Some(region.wl_region()));
        Self {
            handle,
            region,
            rects: [None; 3],
        }
    }

    fn update(&mut self, surface: &wl_surface::WlSurface, size: (u32, u32), show_keyboard: bool) {
        let rects = self.handle.input_rects(size, show_keyboard);
        if rects == self.rects {
            return;
        }
        for RegionRect(x, y, w, h) in self.rects.into_iter().flatten() {
            self.region.subtract(x, y, w, h);
        }
        for RegionRect(x, y, w, h) in rects.into_iter().flatten() {
            self.region.add(x, y, w, h);
        }
        // This gates new contacts only; an implicit grab can leave the region.
        surface.set_input_region(Some(self.region.wl_region()));
        self.rects = rects;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelInput {
    Normal,
    Preview,
    PointerAndTouchOnly,
}

struct ManagedSurface {
    // SAFETY ORDERING: `gpu_surface` MUST be declared before `layer` so `wgpu::Surface`
    // (and its Vulkan/EGL swapchain) is always destroyed before the underlying `wl_surface`.
    #[cfg(feature = "wgpu")]
    gpu_surface: Option<WgpuWaylandSurface>,
    // The fractional-scale and viewport objects belong to this wl_surface.
    scaling: SurfaceScaling,
    layer: LayerSurface,
    handle_input: Option<HandleInputRegion>,
    panel_input: PanelInput,
    pool: Option<SlotPool>,
    #[cfg(feature = "wgpu")]
    retry_at: Option<Instant>,
    #[cfg(feature = "wgpu")]
    initial_acquire_timeouts: Option<u8>,
    // Layer configure sizes and all layout/input coordinates are logical.
    width: u32,
    height: u32,
    configured: bool,
    callback_queued: Option<wl_callback::WlCallback>,
    frame_pending: bool,
    redraw_pending: bool,
}

fn surface_layer(role: LayerRole) -> Layer {
    match role {
        LayerRole::DeskPanel | LayerRole::TabletStatus | LayerRole::TabletBottom => Layer::Top,
        LayerRole::OverlayTopGap | LayerRole::OverlayBottomGap => Layer::Bottom,
        LayerRole::Wallpaper => Layer::Background,
        _ => Layer::Overlay,
    }
}

impl ManagedSurface {
    fn create(app: &LayerApp, role: LayerRole) -> Self {
        let spec = LayerSurfaceSpec::for_role(role, app.viewport_w, app.viewport_h);
        Self::create_with_spec(app, spec)
    }

    fn create_with_spec(app: &LayerApp, spec: LayerSurfaceSpec) -> Self {
        let role = spec.role;
        let gap = GapEdge::for_role(role);
        // On older surfaces SCTK infers the scale from entered outputs. Seed
        // it from the known target so the first pre-enter buffer is sharp.
        // On v6+, the surface itself receives preferred_buffer_scale.
        let initial_factor = if app.compositor.wl_compositor().version() < 6 {
            app.target_output
                .as_ref()
                .and_then(|output| app.output_state.info(output))
                .map_or(1, |info| info.scale_factor.max(1))
        } else {
            1
        };
        let wl_surface = app
            .compositor
            .create_surface_with_data(&app.qh, None, initial_factor, ());
        let layer = app.layer_shell.create_layer_surface(
            &app.qh,
            wl_surface,
            surface_layer(role),
            Some("veilshell"),
            app.target_output.as_ref(),
        );

        let mut anchor = WlrAnchor::empty();
        if spec.anchor.top {
            anchor |= WlrAnchor::TOP;
        }
        if spec.anchor.bottom {
            anchor |= WlrAnchor::BOTTOM;
        }
        if spec.anchor.left {
            anchor |= WlrAnchor::LEFT;
        }
        if spec.anchor.right {
            anchor |= WlrAnchor::RIGHT;
        }

        layer.set_anchor(anchor);
        layer.set_exclusive_zone(spec.exclusive_zone);
        // While the finger pulls from a handle, the preview must not compete
        // for focus with that handle or become a clickable panel.
        let edge_preview = app.uses_preview_input(role);
        layer.set_keyboard_interactivity(if role == LayerRole::DesktopLauncher {
            KeyboardInteractivity::Exclusive
        } else if spec.keyboard_interactivity && !edge_preview {
            KeyboardInteractivity::OnDemand
        } else {
            KeyboardInteractivity::None
        });

        let req_w = if spec.anchor.left && spec.anchor.right {
            0
        } else {
            spec.width
        };
        let req_h = if spec.anchor.top && spec.anchor.bottom {
            0
        } else {
            spec.height
        };
        layer.set_size(req_w, req_h);
        let scaling = SurfaceScaling::new(
            app.fractional_scale.as_ref().filter(|_| gap.is_none()),
            &app.qh,
            layer.wl_surface(),
            initial_factor,
        );
        let handle_input = match role {
            LayerRole::BottomPill => Some(EdgeHandle::Bottom),
            LayerRole::TabletBottom => Some(EdgeHandle::TabletBottom),
            LayerRole::TopStrip => Some(EdgeHandle::Top),
            LayerRole::TabletStatus => Some(EdgeHandle::TabletStatus),
            LayerRole::DeskPanel => Some(EdgeHandle::Desk),
            _ => gap.map(EdgeHandle::Gap),
        }
        .map(|handle| HandleInputRegion::new(handle, &app.compositor, layer.wl_surface()));
        if edge_preview || role == LayerRole::Wallpaper {
            let empty = Region::new(&app.compositor).expect("compositor is already bound");
            layer.wl_surface().set_input_region(Some(empty.wl_region()));
        }
        layer.commit();

        Self {
            #[cfg(feature = "wgpu")]
            gpu_surface: None,
            scaling,
            layer,
            handle_input,
            panel_input: if edge_preview {
                PanelInput::Preview
            } else {
                PanelInput::Normal
            },
            pool: None,
            #[cfg(feature = "wgpu")]
            retry_at: None,
            #[cfg(feature = "wgpu")]
            initial_acquire_timeouts: Some(0),
            width: spec.width,
            height: spec.height,
            configured: false,
            callback_queued: None,
            frame_pending: false,
            redraw_pending: false,
        }
    }

    fn gap_edge(&self) -> Option<GapEdge> {
        match self.handle_input.as_ref()?.handle {
            EdgeHandle::Gap(edge) => Some(edge),
            _ => None,
        }
    }

    fn present_gap(&mut self, shm: &Shm) -> bool {
        // Invisible content needs no GPU, rasterization, extra-scale pixels,
        // or frame callbacks. A single scale-1 SHM buffer per mapped strip
        // suffices; configure/scene changes, not motion, own its lifetime.
        const MAX_BYTES: i32 = 4 * 1024 * 1024;
        let size = (self.width, self.height);
        let Some((width, height, stride, _)) = shm_buffer_geometry(size.0, size.1) else {
            return false;
        };
        let Some(bytes) = stride
            .checked_mul(height)
            .filter(|&bytes| bytes <= MAX_BYTES)
        else {
            return false;
        };
        let Some(input) = &mut self.handle_input else {
            return false;
        };
        let Ok(mut pool) = SlotPool::new(bytes as usize, shm) else {
            return false;
        };
        let Ok((buffer, canvas)) =
            pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888)
        else {
            return false;
        };
        canvas.fill(0);
        let surface = self.layer.wl_surface();
        if buffer.attach_to(surface).is_err() {
            return false;
        }
        input.update(surface, size, false);
        if surface.version() >= 4 {
            surface.damage_buffer(0, 0, width, height);
        } else {
            surface.damage(0, 0, width, height);
        }
        surface.commit();
        self.configured = true;
        true
    }

    #[cfg_attr(
        not(feature = "wgpu"),
        allow(
            clippy::unnecessary_wraps,
            reason = "keep the same presentation contract without GPU"
        )
    )]
    fn present_draw_list(
        &mut self,
        qh: &QueueHandle<LayerApp>,
        shm: &Shm,
        renderer: &mut ActiveRenderer,
        list: &DrawList,
        request_frame_callback: bool,
        #[cfg(feature = "wgpu")] conn: &Connection,
    ) -> Result<bool, String> {
        if !self.configured {
            return Ok(false);
        }
        #[cfg(feature = "wgpu")]
        {
            self.retry_at = None;
        }
        let Some(frame) = self.scaling.frame((self.width, self.height)) else {
            eprintln!(
                "veilshell: invalid buffer size for {}x{} surface",
                self.width, self.height
            );
            return Ok(false);
        };

        #[cfg(feature = "wgpu")]
        let show_keyboard = renderer.context().keyboard_button;
        #[cfg(feature = "wgpu")]
        if let Some(gpu_renderer) = renderer.as_wgpu_mut() {
            let surface = self.layer.wl_surface();
            let gpu_surface = match &mut self.gpu_surface {
                Some(surface) => surface,
                slot @ None => slot.insert(gpu_attempt(
                    "Wayland GPU surface creation panicked",
                    || WgpuWaylandSurface::create(gpu_renderer.instance(), conn, surface),
                )?),
            };
            let scaling = &mut self.scaling;
            let handle_input = &mut self.handle_input;
            let size = (self.width, self.height);
            let callback_queued = &mut self.callback_queued;
            let presented = gpu_attempt("Wayland GPU presentation panicked", || {
                gpu_surface.present_draw_list(gpu_renderer, frame.target, list, || {
                    scaling.apply(surface, frame);
                    if let Some(input) = handle_input {
                        input.update(surface, size, show_keyboard);
                    }
                    if request_frame_callback {
                        *callback_queued =
                            Some(surface.frame(qh, FrameCallbackData(surface.clone())));
                    }
                })
            })?;
            if presented {
                self.callback_queued = None;
                self.initial_acquire_timeouts = None;
            } else {
                if let Some(timeouts) = &mut self.initial_acquire_timeouts {
                    *timeouts += 1;
                    if *timeouts >= 3 {
                        return Err("wgpu timed out acquiring the first Wayland buffer".to_owned());
                    }
                }
                // A timeout committed nothing and cannot produce a callback.
                self.retry_at = Some(Instant::now() + Duration::from_millis(16));
            }
            return Ok(presented);
        }

        Ok(self.present_shm_draw_list(qh, shm, renderer, list, frame, request_frame_callback))
    }

    fn present_shm_draw_list(
        &mut self,
        qh: &QueueHandle<LayerApp>,
        shm: &Shm,
        renderer: &mut ActiveRenderer,
        list: &DrawList,
        frame: SurfaceFrame,
        request_frame_callback: bool,
    ) -> bool {
        let (width, height) = frame.target.physical_size();
        let Some((width, height, stride, pool_bytes)) = shm_buffer_geometry(width, height) else {
            eprintln!("veilshell: invalid wl_shm buffer size {width}x{height}");
            return false;
        };
        if self.pool.is_none() {
            match SlotPool::new(pool_bytes, shm) {
                Ok(pool) => self.pool = Some(pool),
                Err(err) => {
                    eprintln!("veilshell: lazy shm slot pool failed: {err}");
                    return false;
                }
            }
        }
        let Some(pool) = self.pool.as_mut() else {
            return false;
        };

        let pixels = match renderer.rasterize_scaled(frame.target, list) {
            Ok(pixels) => pixels,
            Err(err) => {
                eprintln!("veilshell: rasterize failed: {err}");
                return false;
            }
        };
        let (buffer, canvas) =
            match pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888) {
                Ok(pair) => pair,
                Err(err) => {
                    eprintln!("veilshell: shm create_buffer({width}x{height}) failed: {err}");
                    return false;
                }
            };
        pixels.write_argb8888_premultiplied(canvas);

        let surface = self.layer.wl_surface();
        if let Err(err) = buffer.attach_to(surface) {
            eprintln!("veilshell: buffer.attach_to failed: {err}");
            return false;
        }
        self.scaling.apply(surface, frame);
        if let Some(input) = &mut self.handle_input {
            input.update(
                surface,
                (self.width, self.height),
                renderer.context().keyboard_button,
            );
        }
        if surface.version() >= 4 {
            surface.damage_buffer(0, 0, width, height);
        } else {
            surface.damage(
                0,
                0,
                i32::try_from(self.width).unwrap_or(i32::MAX),
                i32::try_from(self.height).unwrap_or(i32::MAX),
            );
        }
        // A failed WSI present may already have queued this for the next commit.
        let callback_queued = std::mem::take(&mut self.callback_queued);
        if request_frame_callback && callback_queued.is_none() {
            surface.frame(qh, FrameCallbackData(surface.clone()));
        }
        surface.commit();
        true
    }
}

/// Dimensions and bytes accepted by `wl_shm` and its signed-size slot pool.
fn shm_buffer_geometry(width: u32, height: u32) -> Option<(i32, i32, i32, usize)> {
    if width == 0 || height == 0 {
        return None;
    }
    let width = i32::try_from(width).ok()?;
    let height = i32::try_from(height).ok()?;
    let stride = width.checked_mul(4)?;
    let len = u64::try_from(i64::from(stride) * i64::from(height)).ok()?;
    // SCTK rounds slots up to 64 bytes. Leave room for at least two buffers.
    let pool_bytes = i32::try_from((len.next_multiple_of(64) * 2).max(64 * 1024)).ok()?;
    Some((width, height, stride, pool_bytes as usize))
}

fn output_logical_size(info: &OutputInfo) -> Option<(u32, u32)> {
    // xdg-output accounts for fractional scaling; older compositors can fall
    // back to wl_output's current mode, transform, and integer scale.
    let (w, h) = if let Some(size) = info.logical_size {
        size
    } else {
        let (w, h) = info.modes.iter().find(|mode| mode.current)?.dimensions;
        let scale = info.scale_factor.max(1);
        match info.transform {
            wl_output::Transform::_90
            | wl_output::Transform::_270
            | wl_output::Transform::Flipped90
            | wl_output::Transform::Flipped270 => (h / scale, w / scale),
            _ => (w / scale, h / scale),
        }
    };
    (w > 0 && h > 0).then_some((w as u32, h as u32))
}

struct PresentedOverlay {
    layout: SceneOutput,
    interactive: bool,
}

pub(crate) struct LayerApp {
    qh: QueueHandle<Self>,
    registry_state: RegistryState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    seat_state: SeatState,
    output_state: OutputState,
    fractional_scale: Option<FractionalScaleGlobals>,
    pub(crate) shm: Shm,
    target_output: Option<wl_output::WlOutput>,
    output_name: Option<String>,
    viewport_w: u32,
    viewport_h: u32,
    output_transform: wl_output::Transform,
    gaps: GapRefresh,
    pub(crate) visible: bool,
    pub(crate) active_scene: SceneKind,
    pill: Option<ManagedSurface>,
    top_strip: Option<ManagedSurface>,
    overlay: Option<ManagedSurface>,
    wallpaper: Option<ManagedSurface>,
    overlay_layout: Option<PresentedOverlay>,
    overlay_spring: Spring,
    home_spring: Option<Spring>,
    tablet_shade_spring: Option<Spring>,
    // Held after the opening contact releases, until the gesture-driven close.
    tablet_shade_origin: Option<TabletShadeOrigin>,
    // Owns close/cancel routing independently of renderer context updates.
    tablet_overview: Option<TabletOverviewOrigin>,
    refresh_after_open: bool,
    last_anim_tick: Instant,
    palette: ThemePalette,
    pub(crate) renderer: ActiveRenderer,
    #[cfg(feature = "wgpu")]
    renderer_preference: veilshell_render::RendererPreference,
    render_failure: Option<String>,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
    launcher_input: LauncherInput,
    drag: Option<ActiveDrag>,
    pub(crate) osk: OskWaylandState,
    pub(crate) capture: ToplevelCaptureState,
    handler: Box<dyn ShellActionHandler>,
    exit: bool,
    // SAFETY ORDERING: `conn` MUST remain the last field so `wl_display` outlives all
    // `wgpu::Surface` instances held inside `pill`, `top_strip`, and `overlay`.
    #[cfg_attr(not(feature = "wgpu"), allow(dead_code))]
    conn: Connection,
}

impl LayerApp {
    fn uses_preview_input(&self, role: LayerRole) -> bool {
        (role == LayerRole::RecentsDeck && self.tablet_overview == Some(TabletOverviewOrigin::App))
            || (role == LayerRole::QuickSettings
                && self.gap_profile()
                && self.tablet_shade_origin == Some(TabletShadeOrigin::App))
            || matches!(
                role,
                LayerRole::HomeGrid | LayerRole::QuickSettings | LayerRole::RecentsDeck
            ) && self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.surface != SurfaceKind::Overlay && drag.motion.is_some())
    }

    fn sync_top_strip(&mut self) {
        if self.gap_profile() {
            self.sync_gap_strips();
            return;
        }
        let desk = self.uses_desk_panel();
        let tablet = self.uses_tablet_status();
        let keep_pull_origin = self
            .drag
            .as_ref()
            .is_some_and(|drag| drag.surface == SurfaceKind::TopStrip);
        if desk || tablet || self.active_scene == SceneKind::Pill {
            let (role, handle) = if desk {
                (LayerRole::DeskPanel, EdgeHandle::Desk)
            } else if tablet {
                (LayerRole::TabletStatus, EdgeHandle::TabletStatus)
            } else {
                (LayerRole::TopStrip, EdgeHandle::Top)
            };
            let right_role = self
                .top_strip
                .as_ref()
                .and_then(|s| s.handle_input.as_ref())
                .is_some_and(|input| input.handle == handle);
            if !right_role {
                self.top_strip = Some(ManagedSurface::create(self, role));
            }
        } else if !keep_pull_origin {
            self.top_strip = None;
        }
    }

    fn uses_tablet_status(&self) -> bool {
        self.renderer.context().uses_tablet_status()
    }

    fn gap_profile(&self) -> bool {
        overlay_gaps::requested(self.renderer.context())
    }

    fn gap_output_ready(&self) -> bool {
        overlay_gaps::output_ready(
            self.gaps.viewport_valid && self.target_output.as_ref().is_some_and(Proxy::is_alive),
            self.output_name.as_deref(),
        )
    }

    fn foreground_available(&self) -> bool {
        overlay_gaps::can_show_foreground(self.renderer.context(), self.gap_output_ready())
    }

    fn gap_refresh_eligible(&self) -> bool {
        overlay_gaps::can_arm(
            self.renderer.context(),
            self.visible,
            self.gap_output_ready(),
            self.touch.as_ref().is_some_and(Proxy::is_alive),
        )
    }

    fn gap_edge(&self, kind: SurfaceKind) -> Option<GapEdge> {
        match self.edge_handle(kind)?.0 {
            EdgeHandle::Gap(edge) => Some(edge),
            _ => None,
        }
    }

    fn clear_gap_strips(&mut self) {
        for edge in GapEdge::ALL {
            if self.gap_edge(edge.slot()).is_some() {
                *self.gap_slot(edge) = None;
            }
        }
        if self.drag.as_ref().is_some_and(ActiveDrag::is_gap_gesture) {
            self.abort_drag();
        }
    }

    fn invalidate_gap_configuration(&mut self) {
        self.gaps.disarm();
        self.clear_gap_strips();
        if self.gap_refresh_eligible() && self.gap_strips_needed() {
            self.gaps.arm(Instant::now());
        }
    }

    fn gap_slot(&mut self, edge: GapEdge) -> &mut Option<ManagedSurface> {
        match edge {
            GapEdge::Top => &mut self.top_strip,
            GapEdge::Bottom => &mut self.pill,
        }
    }

    fn held_gap_edge(&self) -> Option<GapEdge> {
        let drag = self.drag.as_ref().filter(|drag| drag.is_gap_gesture())?;
        self.gap_edge(drag.surface)
    }

    fn gap_strips_needed(&self) -> bool {
        self.active_scene == SceneKind::Pill || self.held_gap_edge().is_some()
    }

    fn sync_gap_strips(&mut self) {
        if !self.gap_refresh_eligible() {
            self.gaps.disarm();
            self.clear_gap_strips();
            // Never replace an unavailable gap with a visible legacy handle.
            self.pill = None;
            self.top_strip = None;
            return;
        }
        let held = self.held_gap_edge();
        if self.active_scene != SceneKind::Pill && held.is_none() {
            self.gaps.pause();
            self.pill = None;
            self.top_strip = None;
            return;
        }
        self.gaps.arm(Instant::now());
        for edge in GapEdge::ALL {
            // The implicit wl_touch grab retains the origin strip even when
            // the preview is on a different layer above it.
            let spec = if self.active_scene == SceneKind::Pill || held == Some(edge) {
                edge.surface_spec(self.viewport_w, self.gaps.depth(edge))
            } else {
                None
            };
            if let Some(spec) = spec {
                if self.gap_edge(edge.slot()).is_none() {
                    *self.gap_slot(edge) = Some(ManagedSurface::create_with_spec(self, spec));
                }
            } else {
                *self.gap_slot(edge) = None;
            }
        }
    }

    fn refresh_gap_strips(&mut self) {
        if self.gaps.deadline().is_none_or(|at| Instant::now() < at) {
            return;
        }
        if !self.gap_refresh_eligible() {
            self.invalidate_gap_configuration();
            return;
        }
        if !self.gap_strips_needed() {
            self.sync_gap_strips();
            return;
        }
        let depths = self.handler.overlay_gap_depths();
        if self.gaps.sampled(depths, self.viewport_h, Instant::now()) {
            self.clear_gap_strips();
        }
        self.sync_gap_strips();
    }

    fn configure_gap_strip(&mut self, edge: GapEdge, size: (u32, u32)) {
        if !self.gap_refresh_eligible()
            || !overlay_gaps::configure_fits(
                (self.viewport_w, self.viewport_h),
                self.gaps.depth(edge),
                size,
            )
        {
            self.invalidate_gap_configuration();
            return;
        }
        let shm = &self.shm;
        let slot = match edge {
            GapEdge::Top => &mut self.top_strip,
            GapEdge::Bottom => &mut self.pill,
        };
        let Some(strip) = slot else { return };
        if strip.configured {
            if (strip.width, strip.height) != size {
                self.invalidate_gap_configuration();
            }
            return;
        }
        (strip.width, strip.height) = size;
        if !strip.present_gap(shm) {
            self.invalidate_gap_configuration();
        }
    }

    fn overlay_gap_status(&self) -> Option<(u32, u32)> {
        self.gap_profile().then(|| {
            if self.active_scene != SceneKind::Pill || !self.gap_refresh_eligible() {
                return (0, 0);
            }
            let depth = |edge: GapEdge| {
                self.edge_handle(edge.slot())
                    .filter(|(role, s)| *role == EdgeHandle::Gap(edge) && s.configured)
                    .map_or(0, |(_, s)| s.height)
            };
            (depth(GapEdge::Top), depth(GapEdge::Bottom))
        })
    }

    fn tablet_navigation_available(&self) -> bool {
        tablet_navigation_available(self.visible, self.renderer.context())
            && self.foreground_available()
    }

    fn uses_desk_panel(&self) -> bool {
        self.in_desk_mode()
            && self
                .renderer
                .context()
                .settings
                .as_ref()
                .is_none_or(|s| s.panel)
    }

    fn sync_surfaces(&mut self) {
        if !self.visible {
            self.gaps.disarm();
            self.wallpaper = None;
            self.overlay = None;
            self.overlay_layout = None;
            self.top_strip = None;
            self.pill = None;
            return;
        }

        let gap_profile = self.gap_profile();
        if gap_profile {
            self.wallpaper = None;
            if !self.gap_output_ready() {
                self.overlay = None;
                self.overlay_layout = None;
                self.sync_gap_strips();
                return;
            }
        } else if self.wallpaper.is_none() && self.renderer.context().settings.is_some() {
            self.wallpaper = Some(ManagedSurface::create(self, LayerRole::Wallpaper));
        }
        if self.active_scene == SceneKind::Osk {
            // A keyboardless seat may not be advertising wl_keyboard yet.
            // Give the focused client time to subscribe before the first tap,
            // rather than creating the keyboard and typing in the same batch.
            if self
                .osk
                .seat()
                .and_then(|seat| self.seat_state.info(seat))
                .is_some_and(|info| !info.has_keyboard)
            {
                self.osk.ensure_virtual_keyboard(&self.qh);
            }
            self.pill = None;
        } else if !gap_profile && self.active_scene == SceneKind::Launcher {
            self.pill = None;
        } else if !gap_profile {
            let (role, handle) = if self.uses_tablet_status() {
                (LayerRole::TabletBottom, EdgeHandle::TabletBottom)
            } else {
                (LayerRole::BottomPill, EdgeHandle::Bottom)
            };
            let same_role = self
                .pill
                .as_ref()
                .and_then(|surface| surface.handle_input.as_ref())
                .is_some_and(|input| input.handle == handle);
            if !same_role {
                self.pill = Some(ManagedSurface::create(self, role));
            }
        }

        self.sync_top_strip();
        if self.active_scene == SceneKind::Pill {
            self.overlay = None;
            self.overlay_layout = None;
        } else if self.overlay.is_none() {
            let role = LayerRole::from_scene(self.active_scene);
            self.overlay = Some(ManagedSurface::create(self, role));
        }
    }

    fn show_keyboard(&mut self) -> Result<(), &'static str> {
        if !self.foreground_available() {
            return Err("the overlay target output is unavailable");
        }
        let available = if self.visible {
            self.osk.can_open_manually()
        } else {
            self.osk.has_virtual_keyboard() && self.seat_state.seats().next().is_some()
        };
        if !available {
            return Err("no virtual keyboard or active input method for this seat");
        }
        self.osk.auto_opened = false;
        self.show_scene(SceneKind::Osk, false);
        Ok(())
    }

    pub(crate) fn show_scene(&mut self, scene: SceneKind, animate: bool) {
        if scene != SceneKind::Pill && !self.foreground_available() {
            return;
        }
        if scene == SceneKind::Settings
            && (self.renderer.context().settings.is_none() || self.gap_profile())
        {
            return;
        }
        if scene == SceneKind::Drawer && !self.tablet_navigation_available() {
            return;
        }
        if scene == SceneKind::Pill
            && self.active_scene == SceneKind::Launcher
            && self.launcher_input.opened_from_hidden
        {
            self.set_visible(false);
            return;
        }
        if self.drag.as_ref().is_some_and(ActiveDrag::uses_input_time) {
            self.abort_drag();
        }
        self.drag = None;
        self.tablet_shade_origin = None;
        self.tablet_shade_spring = None;
        self.reset_handle_motion();
        if animate && scene == SceneKind::Pill && self.has_gesture_overlay() {
            self.overlay_spring.set_target(0.0);
            self.overlay_spring.velocity = 0.0;
            self.last_anim_tick = Instant::now();
            self.draw_pill();
            self.update_overlay_after_motion();
            return;
        }
        if scene == SceneKind::Shade && self.renderer.context().uses_touch_shade() {
            self.reset_shade_page();
            self.overlay_layout = None;
        }
        let progress = if scene == SceneKind::Pill { 0.0 } else { 1.0 };
        let mut spring = Spring::new(progress, SpringParams::snappy());
        if animate && scene != SceneKind::Pill && scene != self.active_scene {
            spring.value = 0.0;
        }
        self.show_scene_at(scene, spring, SceneRefresh::Now);
    }

    fn show_scene_at(&mut self, scene: SceneKind, spring: Spring, refresh: SceneRefresh) {
        // Gesture previews use this entry point directly. Neither kind of
        // Drawer request may show the shell after it was hidden.
        if (scene == SceneKind::Drawer && !self.tablet_navigation_available())
            || (scene != SceneKind::Pill && !self.foreground_available())
        {
            return;
        }
        let prev_scene = self.active_scene;
        let changed = prev_scene != scene;
        if changed {
            self.prepare_scene_change(scene);
            if prev_scene == SceneKind::Osk {
                self.osk.auto_opened = false;
                self.osk.release_virtual_keyboard();
                let osk = &mut self.renderer.context_mut().osk_state;
                osk.ctrl = false;
                osk.shift = ShiftState::Off;
            }
            if prev_scene == SceneKind::Recents {
                self.capture.invalidate();
            }
        }
        let tablet_recents =
            scene == SceneKind::Recents && self.renderer.context().uses_touch_navigation();
        self.tablet_overview = if tablet_recents {
            if prev_scene == SceneKind::Home {
                Some(TabletOverviewOrigin::Home)
            } else {
                self.tablet_overview.or(Some(TabletOverviewOrigin::App))
            }
        } else {
            None
        };
        if scene != SceneKind::Shade {
            self.reset_shade_page();
            if let Some(menu) = &mut self.renderer.context_mut().power_menu {
                if !matches!(*menu, PowerMenu::Running(_)) {
                    *menu = PowerMenu::Closed;
                }
            }
        }
        if scene != SceneKind::Pill {
            if !self.visible {
                self.visible = true;
                self.sync_osk_visibility();
            }
            if scene == SceneKind::Recents && (changed || matches!(refresh, SceneRefresh::Now)) {
                self.capture.invalidate();
                if !tablet_recents {
                    self.renderer.context_mut().recents_index = 0;
                }
            }
            if matches!(refresh, SceneRefresh::Now) {
                self.refresh_scene(scene);
            }
            if changed && scene == SceneKind::Drawer {
                let ctx = self.renderer.context_mut();
                ctx.drawer.open(&ctx.home_apps);
            }
        }

        self.active_scene = scene;
        self.renderer.context_mut().pill_drag_x = 0.0;
        self.renderer.context_mut().pill_drag_y = 0.0;
        self.renderer.context_mut().top_drag_y = 0.0;

        self.overlay_spring = spring;
        self.home_spring = None;
        self.tablet_shade_spring = None;
        self.renderer.context_mut().recents_throw = None;
        self.last_anim_tick = Instant::now();
        self.refresh_after_open = matches!(refresh, SceneRefresh::OnOpen);

        // Only recreate the overlay LayerSurface when geometry role changes (TouchOsk vs full-screen).
        if changed && scene != SceneKind::Pill {
            self.overlay_layout = None;
            if (prev_scene == SceneKind::Osk) != (scene == SceneKind::Osk)
                || prev_scene == SceneKind::Launcher
                || scene == SceneKind::Launcher
            {
                self.overlay = None;
            }
        }

        self.sync_surfaces();
        self.draw_pill();
        self.draw_top_strip();
        self.draw_overlay();
    }

    fn refresh_scene(&mut self, scene: SceneKind) {
        let ctx = self.renderer.context_mut();
        if scene == SceneKind::Recents && ctx.uses_touch_navigation() {
            if let Some(output) = self
                .output_name
                .as_deref()
                .filter(|_| self.target_output.is_some())
            {
                self.handler.refresh_tablet_overview(output, ctx);
            } else {
                ctx.recents_cards.clear();
            }
            ctx.recents_index = if self.tablet_overview == Some(TabletOverviewOrigin::App) {
                ctx.recents_cards
                    .iter()
                    .position(|card| card.focused)
                    .unwrap_or(0)
            } else {
                0
            };
        } else {
            self.handler
                .on_action(ShellAction::RequestRefresh(scene), ctx);
        }
    }

    fn prepare_scene_change(&mut self, scene: SceneKind) {
        self.cancel_osk_drag();
        if scene == SceneKind::Launcher || self.active_scene == SceneKind::Launcher {
            self.launcher_input.opened_from_hidden = scene == SceneKind::Launcher && !self.visible;
            self.launcher_input.clear_focus();
            self.renderer.context_mut().launcher = veilshell_render::LauncherState::default();
            self.drag = None;
        }
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        self.sync_osk_visibility();
        if !visible {
            self.launcher_input.opened_from_hidden = false;
            self.launcher_input.clear_focus();
            self.renderer.context_mut().launcher = veilshell_render::LauncherState::default();
            self.reset_shade_page();
            if self.active_scene == SceneKind::Recents {
                self.capture.invalidate();
            }
            self.drag = None;
            self.tablet_shade_origin = None;
            self.tablet_overview = None;
            self.reset_handle_motion();
            self.overlay_spring = Spring::new(0.0, SpringParams::snappy());
            self.home_spring = None;
            self.tablet_shade_spring = None;
            self.renderer.context_mut().recents_throw = None;
            self.renderer.context_mut().osk_state = OskState::default();
            self.refresh_after_open = false;
            self.active_scene = SceneKind::Pill;
        } else if !self.tablet_navigation_available() {
            // A mode change can replace the grabbed surface. No held preview
            // or deferred completion can become a Desk/overlay gesture.
            if self.tablet_shade_drag().is_some() || self.tablet_navigation_drag() {
                self.abort_drag();
            }
            if self.active_scene == SceneKind::Drawer || self.tablet_overview.is_some() {
                self.finish_overlay_close();
                return;
            }
            self.tablet_shade_origin = None;
            self.tablet_shade_spring = None;
        } else if self.active_scene == SceneKind::Recents && self.tablet_overview.is_none() {
            // A Recents scene inherited from Desk must not use its unfiltered
            // window list in the Tablet deck.
            self.show_scene(SceneKind::Recents, false);
            return;
        }
        self.sync_surfaces();
        if visible {
            if self.in_desk_mode() && self.active_scene == SceneKind::Osk && self.osk.auto_opened {
                self.show_scene(SceneKind::Pill, false);
                return;
            }
            self.draw_pill();
            self.draw_top_strip();
            self.draw_overlay();
        }
    }

    pub(crate) fn in_desk_mode(&self) -> bool {
        let ctx = self.renderer.context();
        ctx.power_menu.is_some() && !ctx.tablet_mode
    }

    fn sync_osk_visibility(&mut self) {
        if self.visible {
            if let Some(seat) = self.seat_state.seats().next() {
                self.osk.attach_seat(&seat, &self.qh);
            }
        } else {
            self.osk.destroy();
        }
    }

    pub(crate) fn on_osk_editor_lost(&mut self) {
        if self.active_scene == SceneKind::Osk {
            self.invalidate_osk_layout();
            if self.osk.auto_opened || !self.osk.can_open_manually() {
                self.show_scene(SceneKind::Pill, false);
            } else {
                self.draw_overlay();
            }
        } else {
            self.draw_pill();
        }
    }

    pub(crate) fn draw_pill(&mut self) {
        let Some(surf) = &mut self.pill else {
            return;
        };
        if surf.gap_edge().is_some() {
            return;
        }
        if !surf.configured || surf.width == 0 || surf.height == 0 {
            return;
        }
        let ctx = self.renderer.context_mut();
        if let Some(nav) = self
            .drag
            .as_ref()
            .and_then(|drag| drag.tablet_bottom.as_ref())
        {
            (ctx.pill_drag_x, ctx.pill_drag_y, ctx.pill_pressed) = nav.feedback();
        } else {
            ctx.pill_pressed = self.drag.as_ref().is_some_and(|drag| {
                drag.surface == SurfaceKind::Pill && drag.keyboard_button.is_none()
            });
        }
        ctx.keyboard_button = self.active_scene == SceneKind::Pill && self.osk.can_open_manually();
        let list = build_scene(
            SceneKind::Pill,
            surf.width as f32,
            surf.height as f32,
            &self.palette,
            self.renderer.context(),
        );
        self.present_surface(SurfaceKind::Pill, &list, false);
    }

    fn draw_top_strip(&mut self) {
        let desk = self.uses_desk_panel();
        let tablet = self.uses_tablet_status();
        let moving_status = self.tablet_shade_header_visible();
        let Some(surf) = &mut self.top_strip else {
            return;
        };
        if surf.gap_edge().is_some() {
            return;
        }
        if !surf.configured || surf.width == 0 || surf.height == 0 {
            return;
        }
        if desk {
            let list = veilshell_render::build_desk_panel(
                surf.width as f32,
                &self.palette,
                self.renderer.context(),
            );
            self.present_surface(SurfaceKind::TopStrip, &list, false);
            return;
        }
        if !tablet {
            self.renderer.context_mut().top_pressed = self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.surface == SurfaceKind::TopStrip);
        }
        let ctx = self.renderer.context();
        let (w, h) = (surf.width as f32, surf.height as f32);
        let list = if moving_status {
            veilshell_render::build_tablet_status_background(w, h, &self.palette)
        } else if tablet {
            veilshell_render::build_tablet_status(w, h, &self.palette, ctx)
        } else {
            build_top_strip_scene(w, h, &self.palette, ctx.top_drag_y, ctx.top_pressed)
        };
        self.present_surface(SurfaceKind::TopStrip, &list, false);
    }

    fn draw_wallpaper(&mut self) {
        if let Some(surface) = self.wallpaper.as_ref().filter(|s| s.configured) {
            let list = veilshell_render::build_wallpaper(
                self.renderer.context(),
                surface.width as f32,
                surface.height as f32,
            );
            self.present_surface(SurfaceKind::Wallpaper, &list, false);
        }
    }

    pub(crate) fn draw_overlay(&mut self) {
        let input_ready = self.overlay_input_ready()
            && self
                .drag
                .as_ref()
                .is_none_or(|drag| drag.surface == SurfaceKind::Overlay);
        let held = self.overlay_motion_held();
        let tablet_navigation = self.tablet_overlay_owns_navigation();
        let Some(surf) = &mut self.overlay else {
            return;
        };
        if !surf.configured || surf.width == 0 || surf.height == 0 {
            return;
        }
        if surf.frame_pending {
            surf.redraw_pending = true;
            return;
        }
        if input_ready && surf.panel_input != PanelInput::Normal {
            let preserve_app_focus = self.tablet_shade_origin == Some(TabletShadeOrigin::App)
                || self.tablet_overview == Some(TabletOverviewOrigin::App);
            if surf.panel_input == PanelInput::Preview {
                surf.layer.wl_surface().set_input_region(None);
            }
            if !preserve_app_focus {
                surf.layer
                    .set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
            }
            // An app-origin preview already disables keyboard interactivity;
            // enable control hits without taking the app's keyboard focus.
            surf.panel_input = if preserve_app_focus {
                PanelInput::PointerAndTouchOnly
            } else {
                PanelInput::Normal
            };
        }
        let (width, height) = (surf.width, surf.height);
        if self.active_scene == SceneKind::Recents {
            if let Some(frame) = surf.scaling.frame((width, height)) {
                self.capture_recents(frame.target);
            }
        }
        if matches!(self.active_scene, SceneKind::Home | SceneKind::Drawer) {
            let ctx = self.renderer.context_mut();
            let total = ctx.home_page_count(width as f32, height as f32);
            ctx.home_page = ctx.home_page.min(total - 1);
        }
        let ctx = self.renderer.context_mut();
        if self.active_scene == SceneKind::Osk {
            ctx.osk_output_size = Some((self.viewport_w, self.viewport_h));
        }
        ctx.overlay_progress = self.overlay_spring.value.clamp(0.0, 1.0);
        if self.active_scene == SceneKind::Drawer {
            let limit = drawer_max_scroll(width as f32, height as f32, ctx.home_apps.len());
            ctx.drawer.scroll = ctx.drawer.scroll.clamp(0.0, limit);
        }
        ctx.tablet_overview = self.tablet_overview;
        ctx.shade.tablet_page_position = self.tablet_shade_spring.map(|s| s.value.clamp(0.0, 1.0));
        ctx.home_position = if self.active_scene == SceneKind::Home {
            self.home_spring.map(|spring| spring.value)
        } else {
            None
        };
        ctx.recents_position = if self.active_scene == SceneKind::Recents {
            self.home_spring.map(|spring| spring.value)
        } else {
            None
        };
        let mut layout = if self.tablet_shade_origin == Some(TabletShadeOrigin::Home) {
            build_shade_over_home(
                width as f32,
                height as f32,
                &self.palette,
                self.renderer.context(),
            )
        } else {
            build_scene_layout(
                self.active_scene,
                width as f32,
                height as f32,
                &self.palette,
                self.renderer.context(),
            )
        };
        if tablet_navigation && !input_ready {
            layout.hit_regions.clear();
        }
        let animating = !self.overlay_spring.is_settled()
            || self.home_spring.is_some()
            || self.tablet_shade_spring.is_some()
            || held;
        if self.present_surface(SurfaceKind::Overlay, &layout.draw_list, animating) {
            if let Some(surf) = &mut self.overlay {
                surf.frame_pending = animating;
            }
            self.overlay_layout = Some(PresentedOverlay {
                layout,
                interactive: input_ready,
            });
        }
    }

    fn capture_recents(&mut self, target: veilshell_render::RenderTarget) {
        if self.capture.needs_capture(target) {
            let ctx = self.renderer.context_mut();
            ctx.recents_index = ctx
                .recents_index
                .min(ctx.recents_cards.len().saturating_sub(1));
            self.capture.apply_cached_and_capture(
                &mut ctx.recents_cards,
                ctx.recents_index,
                target,
                &self.qh,
            );
        }
    }

    fn present_surface(&mut self, kind: SurfaceKind, list: &DrawList, animate: bool) -> bool {
        if self.render_failure.is_some() {
            return false;
        }
        match self.try_present_surface(kind, list, animate) {
            Ok(presented) => presented,
            #[cfg(feature = "wgpu")]
            Err(err) if self.renderer_preference == veilshell_render::RendererPreference::Auto => {
                eprintln!("veilshell: Wayland GPU failed for {kind:?} ({err}); using sdf-software");
                // Tear down swapchains before dropping the device. Already committed buffers
                // may remain visible; future draws on all surfaces use wl_shm.
                for surface in [
                    &mut self.pill,
                    &mut self.top_strip,
                    &mut self.overlay,
                    &mut self.wallpaper,
                ]
                .into_iter()
                .flatten()
                {
                    surface.gpu_surface = None;
                }
                self.renderer = ActiveRenderer::Software(Box::new(
                    veilshell_render::SdfSoftwareRenderer::with_context(std::mem::take(
                        self.renderer.context_mut(),
                    )),
                ));
                self.try_present_surface(kind, list, animate)
                    .unwrap_or(false)
            }
            Err(err) => {
                self.render_failure = Some(format!("Wayland GPU failed for {kind:?}: {err}"));
                self.exit = true;
                false
            }
        }
    }

    fn try_present_surface(
        &mut self,
        kind: SurfaceKind,
        list: &DrawList,
        animate: bool,
    ) -> Result<bool, String> {
        let surface = match kind {
            SurfaceKind::Wallpaper => self.wallpaper.as_mut(),
            SurfaceKind::Pill => self.pill.as_mut(),
            SurfaceKind::TopStrip => self.top_strip.as_mut(),
            SurfaceKind::Overlay => self.overlay.as_mut(),
        };
        surface.map_or(Ok(false), |surf| {
            surf.present_draw_list(
                &self.qh,
                &self.shm,
                &mut self.renderer,
                list,
                animate,
                #[cfg(feature = "wgpu")]
                &self.conn,
            )
        })
    }

    fn surface_kind(&self, surface: &wl_surface::WlSurface) -> Option<SurfaceKind> {
        if self
            .pill
            .as_ref()
            .is_some_and(|s| s.layer.wl_surface() == surface)
        {
            Some(SurfaceKind::Pill)
        } else if self
            .top_strip
            .as_ref()
            .is_some_and(|s| s.layer.wl_surface() == surface)
        {
            Some(SurfaceKind::TopStrip)
        } else if self
            .overlay
            .as_ref()
            .is_some_and(|s| s.layer.wl_surface() == surface)
        {
            Some(SurfaceKind::Overlay)
        } else {
            self.wallpaper
                .as_ref()
                .filter(|s| s.layer.wl_surface() == surface)
                .map(|_| SurfaceKind::Wallpaper)
        }
    }

    fn managed_surface_mut(&mut self, kind: SurfaceKind) -> Option<&mut ManagedSurface> {
        match kind {
            SurfaceKind::Wallpaper => self.wallpaper.as_mut(),
            SurfaceKind::Pill => self.pill.as_mut(),
            SurfaceKind::TopStrip => self.top_strip.as_mut(),
            SurfaceKind::Overlay => self.overlay.as_mut(),
        }
    }

    fn draw_surface(&mut self, kind: SurfaceKind) {
        match kind {
            SurfaceKind::Wallpaper => self.draw_wallpaper(),
            SurfaceKind::Pill => self.draw_pill(),
            SurfaceKind::TopStrip => self.draw_top_strip(),
            SurfaceKind::Overlay => self.draw_overlay(),
        }
    }

    pub(crate) fn fractional_scale_changed(&mut self, surface: &wl_surface::WlSurface, units: u32) {
        if let Some(kind) = self.surface_kind(surface) {
            self.update_surface_scale(kind, |scaling| scaling.update_fractional(units));
        }
    }

    fn update_surface_scale(
        &mut self,
        kind: SurfaceKind,
        update: impl FnOnce(&mut SurfaceScaling) -> bool,
    ) {
        if self.gap_edge(kind).is_some() {
            return; // transparent scale-1 buffers do not need re-rasterizing
        }
        let Some(surf) = self.managed_surface_mut(kind) else {
            return;
        };
        let size = (surf.width, surf.height);
        let previous = surf.scaling.frame(size).map(|frame| frame.target);
        if update(&mut surf.scaling) {
            if previous != surf.scaling.frame(size).map(|frame| frame.target)
                && self
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.uses_input_time() && drag.surface == kind)
            {
                self.abort_drag();
            }
            self.draw_surface(kind);
        }
    }

    #[cfg(feature = "wgpu")]
    fn swapchain_retries(&self) -> impl Iterator<Item = (SurfaceKind, Instant)> + '_ {
        [
            (SurfaceKind::Wallpaper, &self.wallpaper),
            (SurfaceKind::Pill, &self.pill),
            (SurfaceKind::TopStrip, &self.top_strip),
            (SurfaceKind::Overlay, &self.overlay),
        ]
        .into_iter()
        .filter_map(|(kind, surface)| Some((kind, surface.as_ref()?.retry_at?)))
    }

    fn event_timeout(&self) -> Option<Duration> {
        let gap_timeout = self
            .gaps
            .deadline()
            .map(|at| at.saturating_duration_since(Instant::now()));
        let osk_and_gaps = self.osk_repeat_timeout().into_iter().chain(gap_timeout);
        #[cfg(feature = "wgpu")]
        {
            let now = Instant::now();
            self.swapchain_retries()
                .map(|(_, retry_at)| retry_at.saturating_duration_since(now))
                .chain(osk_and_gaps)
                .min()
        }
        #[cfg(not(feature = "wgpu"))]
        osk_and_gaps.min()
    }

    #[cfg(feature = "wgpu")]
    fn retry_swapchain_draws(&mut self) {
        let now = Instant::now();
        let retry = self
            .swapchain_retries()
            .filter_map(|(kind, at)| (at <= now).then_some(kind))
            .collect::<Vec<_>>();
        for kind in retry {
            self.draw_surface(kind);
        }
    }

    fn handle_daemon_command(&mut self, cmd: DaemonCommand) {
        match cmd {
            DaemonCommand::ShowScene(SceneKind::Launcher)
                if self.active_scene == SceneKind::Launcher =>
            {
                self.show_scene(SceneKind::Pill, false);
            }
            DaemonCommand::ShowScene(scene) => self.show_scene(scene, false),
            DaemonCommand::ShowKeyboard(reply) => {
                let _ = reply.send(self.show_keyboard());
            }
            DaemonCommand::SetVisible(vis) => self.set_visible(vis),
            DaemonCommand::UpdateContext(ctx) => self.update_context(*ctx),
            DaemonCommand::HardwareUpdate {
                keyboard_attached,
                orientation,
            } => {
                let next_vis = self.handler.on_hardware_update(
                    keyboard_attached,
                    orientation,
                    self.renderer.context_mut(),
                );
                if let Some(vis) = next_vis {
                    self.set_visible(vis);
                } else if self.visible {
                    self.draw_pill();
                    self.draw_top_strip();
                    self.draw_overlay();
                }
            }
            DaemonCommand::UpdatePalette(palette) => {
                self.palette = palette;
                if self.visible {
                    self.draw_pill();
                    self.draw_top_strip();
                    self.draw_overlay();
                }
            }
            DaemonCommand::UpdateTabletStatus(status) => self.update_tablet_status(status),
            DaemonCommand::SetWallpaper(wallpaper) => {
                if let Err(err) = self.renderer.set_wallpaper(wallpaper.as_ref()) {
                    eprintln!("veilshell: set_wallpaper failed: {err}");
                }
                self.draw_overlay();
            }
            DaemonCommand::PowerFinished { action, success } => {
                if let Some(menu) = &mut self.renderer.context_mut().power_menu {
                    menu.finish(action, success);
                    if self.active_scene == SceneKind::Shade {
                        self.draw_overlay();
                    }
                }
            }
            DaemonCommand::ShadeFinished { ticket, result } => self.finish_shade(ticket, result),
            DaemonCommand::QueryStatus(reply) => {
                let _ = reply.send(RuntimeStatus {
                    active_scene: self.active_scene,
                    visible: self.visible,
                    output_name: self.output_name.clone(),
                    width: self.viewport_w,
                    height: self.viewport_h,
                    renderer: self.renderer.backend_label(),
                    standalone_layout: self.renderer.context().power_menu.as_ref().map(|_| {
                        if self.in_desk_mode() {
                            "Desk"
                        } else {
                            "Tablet"
                        }
                    }),
                    tablet_shade_layout: self.tablet_shade_layout(),
                    tablet_navigation: self.tablet_navigation_available(),
                    overlay_gaps: self.overlay_gap_status(),
                });
            }
            DaemonCommand::Custom { cmd, reply } => {
                let (msg, next_vis) = self
                    .handler
                    .on_custom_command(&cmd, self.renderer.context_mut());
                if let Some(vis) = next_vis {
                    self.set_visible(vis);
                } else if self.visible {
                    self.draw_pill();
                    self.draw_overlay();
                }
                let _ = reply.send(msg);
            }
            DaemonCommand::Quit => self.exit = true,
        }
    }

    fn update_context(&mut self, mut ctx: SceneContext) {
        let gap_changed = self.gap_profile() != overlay_gaps::requested(&ctx);
        let layout_changed = gap_changed || self.uses_tablet_status() != ctx.uses_tablet_status();
        let apps_changed = self.renderer.context().home_apps != ctx.home_apps;
        if (self.tablet_navigation_drag() && (layout_changed || apps_changed))
            || (gap_changed && self.drag.is_some())
        {
            self.abort_drag();
        }
        if gap_changed {
            self.gaps.disarm();
            self.clear_gap_strips();
        }
        if !layout_changed && ctx.uses_touch_navigation() {
            let live = self.renderer.context_mut();
            ctx.home_page = live.home_page;
            if self.active_scene == SceneKind::Drawer {
                ctx.drawer = std::mem::take(&mut live.drawer);
                if apps_changed {
                    self.overlay_layout = None;
                    ctx.drawer.rebuild(&ctx.home_apps);
                }
            }
            if self.tablet_overview.is_some() {
                // A generic context replacement does not own the output-
                // filtered overview snapshot or its one-shot capture targets.
                ctx.recents_cards = std::mem::take(&mut live.recents_cards);
                ctx.recents_index = live.recents_index;
            }
        }
        *self.renderer.context_mut() = ctx;
        if layout_changed {
            if gap_changed {
                self.overlay_layout = None;
            }
            self.set_visible(self.visible);
        } else if self.visible {
            if self.active_scene == SceneKind::Recents && self.tablet_overview.is_none() {
                self.capture.invalidate();
            }
            self.draw_pill();
            self.draw_top_strip();
            self.draw_overlay();
        }
    }

    fn update_tablet_status(&mut self, status: veilshell_render::TabletStatus) {
        self.renderer.context_mut().tablet_status = status;
        if self.uses_tablet_status() {
            self.draw_top_strip();
            if matches!(self.active_scene, SceneKind::Home | SceneKind::Drawer)
                || self.tablet_shade_origin == Some(TabletShadeOrigin::Home)
                || self.tablet_overview == Some(TabletOverviewOrigin::Home)
            {
                self.draw_overlay();
            }
        }
    }

    fn home_status_at(&self, x: f32, y: f32) -> bool {
        self.active_scene == SceneKind::Home
            && self.uses_tablet_status()
            && self.overlay.as_ref().is_some_and(|surf| {
                EdgeHandle::TabletStatus
                    .input_rect((surf.width, surf.height))
                    .is_some_and(|rect| rect.contains(x, y))
            })
    }

    fn edge_handle(&self, kind: SurfaceKind) -> Option<(EdgeHandle, &ManagedSurface)> {
        let surface = match kind {
            SurfaceKind::Pill => self.pill.as_ref(),
            SurfaceKind::TopStrip => self.top_strip.as_ref(),
            _ => None,
        }?;
        Some((surface.handle_input.as_ref()?.handle, surface))
    }

    fn tablet_overlay_owns_navigation(&self) -> bool {
        self.tablet_navigation_available()
            && matches!(
                self.active_scene,
                SceneKind::Home | SceneKind::Recents | SceneKind::Drawer
            )
    }

    fn overlay_accepts_down(&self) -> bool {
        // Readiness includes the presented geometry, not just a spring
        // that may have reached its endpoint before the next frame.
        self.overlay_input_ready()
            && self
                .overlay_layout
                .as_ref()
                .is_some_and(|frame| frame.interactive)
    }

    fn tablet_overlay_accepts_down(&self, x: f32, y: f32) -> bool {
        // The system strips do not belong to the list or dismiss the sheet.
        self.overlay_accepts_down()
            && (self.active_scene != SceneKind::Drawer
                || ((!self.uses_tablet_status() || y >= TABLET_STATUS_HEIGHT as f32)
                    && self
                        .panel_bottom_band()
                        .is_none_or(|rect| !rect.contains(x, y))))
    }

    fn surface_accepts_down(&self, kind: SurfaceKind, (x, y): (f32, f32)) -> bool {
        match kind {
            SurfaceKind::Wallpaper => false,
            SurfaceKind::Overlay if self.tablet_shade_spring.is_some() => false,
            SurfaceKind::Overlay if self.tablet_overlay_owns_navigation() => {
                self.tablet_overlay_accepts_down(x, y)
            }
            SurfaceKind::Overlay if self.gap_profile() => self.overlay_accepts_down(),
            _ => true,
        }
    }

    fn tablet_shade_origin_at(
        &self,
        kind: SurfaceKind,
        handle: Option<EdgeHandle>,
        x: f32,
        y: f32,
    ) -> Option<TabletShadeOrigin> {
        if matches!(
            handle,
            Some(EdgeHandle::TabletStatus | EdgeHandle::Gap(GapEdge::Top))
        ) && self.active_scene == SceneKind::Pill
            && self.renderer.context().uses_touch_shade()
        {
            Some(TabletShadeOrigin::App)
        } else if kind == SurfaceKind::Overlay && self.home_status_at(x, y) {
            Some(TabletShadeOrigin::Home)
        } else {
            None
        }
    }

    fn gap_accepts_down(&self, strip: &ManagedSurface, gestures: bool, x: f32, y: f32) -> bool {
        gestures
            && self.active_scene == SceneKind::Pill
            && self.gap_refresh_eligible()
            && strip
                .handle_input
                .as_ref()
                .and_then(|input| input.rects[0])
                .is_some_and(|rect| rect.contains(x, y))
    }

    fn osk_accepts_down(&self, seat: Option<&wl_seat::WlSeat>, is_osk: bool) -> bool {
        seat.is_some()
            && seat == self.osk.seat()
            && !self.osk.editor_changing()
            && (!is_osk || self.overlay_layout.is_some())
    }

    fn begin_drag(
        &mut self,
        seat: Option<&wl_seat::WlSeat>,
        touch_id: Option<i32>,
        kind: SurfaceKind,
        x: f32,
        y: f32,
        time: u32,
    ) {
        if !self.surface_accepts_down(kind, (x, y)) {
            return;
        }
        let gap_profile = self.gap_profile();
        let tablet_overlay = kind == SurfaceKind::Overlay && self.tablet_overlay_owns_navigation();
        let shade_overlay = kind == SurfaceKind::Overlay && self.in_tablet_shade();
        let gestures = gesture_input_allowed(gap_profile, touch_id);
        let handle = self.edge_handle(kind);
        let role = handle.map(|(role, _)| role);
        let on_gap_strip = matches!(role, Some(EdgeHandle::Gap(_)));
        if on_gap_strip && !handle.is_some_and(|(_, s)| self.gap_accepts_down(s, gestures, x, y)) {
            return;
        }
        let is_osk = kind == SurfaceKind::Overlay && self.active_scene == SceneKind::Osk;
        let keyboard_button = (kind == SurfaceKind::Pill && self.active_scene == SceneKind::Pill)
            .then(|| self.keyboard_button_at(x, y))
            .flatten();
        if (is_osk || keyboard_button.is_some()) && !self.osk_accepts_down(seat, is_osk) {
            return;
        }
        let tablet_origin = gestures
            .then(|| self.tablet_shade_origin_at(kind, role, x, y))
            .flatten();
        if role == Some(EdgeHandle::TabletStatus) && tablet_origin.is_none() {
            return;
        }
        let handle_tap_allowed =
            handle.is_some_and(|(handle, s)| handle.allows_tap(s.width, s.height, x, y));
        let tablet_bottom = tablet_origin
            .is_none()
            .then(|| self.tablet_bottom_contact(kind, (x, y), touch_id))
            .flatten();
        if on_gap_strip && tablet_origin.is_none() && tablet_bottom.is_none() {
            return;
        }
        let tablet_content = tablet_overlay && tablet_origin.is_none() && tablet_bottom.is_none();
        let overlay_tap_allowed = tablet_origin.is_none()
            && tablet_bottom.is_none()
            && kind == SurfaceKind::Overlay
            && self.overlay_input_ready();
        let down_target = overlay_tap_allowed
            .then(|| self.overlay_layout.as_ref()?.layout.hit_test(x, y).cloned())
            .flatten();
        let recent_card = match &down_target {
            Some(HitTarget::FocusWindow { id }) if self.active_scene == SceneKind::Recents => {
                Some(id.clone())
            }
            _ => None,
        };
        let active_slider = (overlay_tap_allowed && self.active_scene == SceneKind::Shade)
            .then(|| self.shade_slider_at(x, y))
            .flatten();

        let now = Instant::now();
        let osk_contact = is_osk.then(|| self.osk_at(x, y, now)).flatten();
        let tablet_contact = if tablet_origin.is_some() || shade_overlay {
            TabletContact::Shade
        } else if tablet_content {
            TabletContact::Content
        } else {
            TabletContact::None
        };
        self.drag = Some(ActiveDrag {
            touch_id,
            policy: if !gestures {
                DragPolicy::ClickOnly
            } else if gap_profile && (on_gap_strip || tablet_overlay || shade_overlay) {
                DragPolicy::GapGesture
            } else {
                DragPolicy::Regular
            },
            surface: kind,
            tablet_origin,
            tablet_contact,
            tablet_bottom,
            drawer: None,
            start_x: x,
            start_y: y,
            last_x: x,
            last_y: y,
            max_travel: 0.0,
            handle_tap_allowed,
            keyboard_button,
            overlay_tap_allowed,
            down_target,
            recent_card,
            active_slider,
            osk_contact,
            motion: None,
            velocity: DragVelocity::new(now, x, y),
            start_time: now,
            protocol_start: time,
        });
        if kind != SurfaceKind::Overlay && keyboard_button.is_none() && tablet_origin.is_none() {
            self.draw_surface(kind);
        }
    }

    fn osk_at(&self, x: f32, y: f32, now: Instant) -> Option<OskContact> {
        self.overlay_layout
            .as_ref()?
            .layout
            .hit_regions
            .iter()
            .rev()
            .find_map(|hit| match hit.target {
                HitTarget::OskKey { label } if hit.rect.contains(x, y) => {
                    Some(OskContact::new(now, label, hit.rect))
                }
                _ => None,
            })
    }

    fn keyboard_button_at(&self, x: f32, y: f32) -> Option<RegionRect> {
        self.pill.as_ref()?.handle_input.as_ref()?.rects[1].filter(|rect| rect.contains(x, y))
    }

    fn panel_bottom_band(&self) -> Option<RegionRect> {
        let panel = self.overlay.as_ref()?;
        let size = (panel.width, panel.height);
        if self.gap_profile() {
            full_width_band(size, TABLET_BOTTOM_DEPTH)
        } else {
            EdgeHandle::TabletBottom.input_rect(size)
        }
    }

    fn tablet_bottom_contact(
        &self,
        kind: SurfaceKind,
        position: (f32, f32),
        touch_id: Option<i32>,
    ) -> Option<BottomContact> {
        if !self.tablet_navigation_available() {
            return None;
        }
        let origin = NavOrigin::for_scene(self.active_scene)?;
        let (bounds, height) = match (kind, origin) {
            (SurfaceKind::Pill, NavOrigin::App) => {
                let (handle, surface) = self.edge_handle(kind)?;
                // A down from the work-area gap is local to the strip (0..depth).
                // Travel is normalized to the full output, not the slab height.
                (
                    handle.input_rect((surface.width, surface.height)),
                    self.viewport_h,
                )
            }
            (SurfaceKind::Overlay, NavOrigin::Home | NavOrigin::Recents) => {
                (self.panel_bottom_band(), self.overlay.as_ref()?.height)
            }
            _ => return None,
        };
        bottom_contact_at(
            origin,
            height,
            bounds,
            position,
            self.gap_profile(),
            touch_id,
        )
    }

    fn shade_slider_at(&self, x: f32, y: f32) -> Option<SliderDrag> {
        let HitTarget::ShadeSlider { id, rect } =
            self.overlay_layout.as_ref()?.layout.hit_test(x, y)?
        else {
            return None;
        };
        let ctx = self.renderer.context();
        let level = if *id == "brightness" {
            ctx.brightness_pct
        } else {
            ctx.volume_pct
        };
        Some(SliderDrag::new(id, *rect, level))
    }

    fn overlay_size(&self) -> (f32, f32) {
        self.overlay
            .as_ref()
            .map_or((self.viewport_w as f32, self.viewport_h as f32), |surf| {
                (surf.width as f32, surf.height as f32)
            })
    }

    fn home_pages(&self) -> usize {
        let (w, h) = self.overlay_size();
        self.renderer.context().home_page_count(w, h)
    }

    fn page_count(&self) -> usize {
        if self.active_scene == SceneKind::Recents {
            self.renderer.context().recents_cards.len().max(1)
        } else {
            self.home_pages()
        }
    }

    fn has_gesture_overlay(&self) -> bool {
        matches!(self.active_scene, SceneKind::Home | SceneKind::Drawer)
            || (self.active_scene == SceneKind::Shade
                && self
                    .renderer
                    .context()
                    .power_menu
                    .is_none_or(|state| state == PowerMenu::Closed))
    }

    fn overlay_input_ready(&self) -> bool {
        self.overlay_spring.is_settled()
            && self.overlay_spring.target >= 1.0
            && self.home_spring.is_none()
            && self.tablet_shade_spring.is_none()
            && !self.overlay_motion_held()
    }

    fn overlay_motion_held(&self) -> bool {
        self.drag.as_ref().is_some_and(|drag| {
            drag.motion.is_some()
                || drag.drawer.is_some()
                || (self.active_scene == SceneKind::Recents && drag.tablet_bottom.is_some())
        })
    }

    fn in_tablet_shade(&self) -> bool {
        self.active_scene == SceneKind::Shade && self.renderer.context().uses_touch_shade()
    }

    fn tablet_shade_header_visible(&self) -> bool {
        self.in_tablet_shade() && self.overlay_spring.value > 0.0
    }

    fn tablet_shade_main(&self) -> bool {
        let ctx = self.renderer.context();
        ctx.shade.page == ShadePage::Main
            && ctx.power_menu.is_none_or(|menu| menu == PowerMenu::Closed)
    }

    fn tablet_shade_layout(&self) -> Option<&'static str> {
        if !self.visible || !self.in_tablet_shade() {
            return None;
        }
        let (w, h) = self.overlay_size();
        Some(if tablet_shade_is_split(w, h) {
            "split"
        } else if self.tablet_shade_spring.is_some() {
            "sliding"
        } else {
            match self.renderer.context().shade.tablet_page {
                TabletShadePage::Notifications => "notifications",
                TabletShadePage::Controls => "controls",
            }
        })
    }

    fn tablet_shade_drag(&self) -> Option<&ActiveDrag> {
        self.drag
            .as_ref()
            .filter(|drag| drag.tablet_contact == TabletContact::Shade)
    }

    fn tablet_navigation_drag(&self) -> bool {
        self.drag
            .as_ref()
            .is_some_and(ActiveDrag::is_tablet_navigation)
    }

    fn tablet_status_drag(&self) -> bool {
        self.tablet_shade_drag()
            .is_some_and(|drag| drag.surface == SurfaceKind::TopStrip)
    }

    fn tablet_shade_status_at(&self, y: f32) -> bool {
        let (w, h) = self.overlay_size();
        // The expanded status cluster occupies 40 + 18 reference px.
        y < (58.0 * tablet_shade_scale(w, h)).max(TABLET_STATUS_HEIGHT as f32)
    }

    fn tablet_shade_motion(&self, drag: &ActiveDrag, dx: f32, dy: f32) -> Option<DragMotion> {
        if !self.tablet_shade_main() {
            return None;
        }
        let (w, h) = self.overlay_size();
        let progress = self.overlay_spring.value.clamp(0.0, 1.0);
        let closing = self.overlay_spring.target <= 0.0;
        if vertical(dx, dy) && (dy < 0.0 || !self.overlay_spring.is_settled()) {
            let from_status =
                self.uses_tablet_status() && self.tablet_shade_status_at(drag.start_y);
            return Some(DragMotion::tablet_shade(progress, h, from_status, closing));
        }
        if !closing && progress >= 0.98 && !tablet_shade_is_split(w, h) && horizontal(dx, dy) {
            let page = self.renderer.context().shade.tablet_page;
            return Some(DragMotion::TabletShadePage {
                from: f32::from(u8::from(page == TabletShadePage::Controls)),
                width: w.max(1.0),
                scale: tablet_shade_scale(w, h),
            });
        }
        None
    }

    fn start_interactive_motion(
        &self,
        drag: &ActiveDrag,
        dx: f32,
        dy: f32,
    ) -> Option<(DragMotion, Option<SceneKind>)> {
        let (w, h) = self.overlay_size();
        if drag.tablet_origin.is_some() {
            // The embedded status row is swipe-only: never fall through to
            // Home's dismiss, pager or tap paths for this contact.
            let height =
                if drag.policy == DragPolicy::GapGesture && drag.surface == SurfaceKind::TopStrip {
                    self.viewport_h as f32
                } else {
                    h
                };
            return tablet_status_reveal(dx, dy, height)
                .map(|motion| (motion, Some(SceneKind::Shade)));
        }
        if edge_vertical(dx, dy) {
            match (drag.surface, self.active_scene) {
                (SurfaceKind::Pill, SceneKind::Pill)
                    if dy < 0.0 && !self.renderer.context().uses_touch_navigation() =>
                {
                    return Some((
                        DragMotion::edge_reveal(home_reveal_travel(h), -1.0),
                        Some(SceneKind::Home),
                    ))
                }
                (SurfaceKind::TopStrip, SceneKind::Pill)
                    if dy > 0.0 && !self.uses_desk_panel() && !self.gap_profile() =>
                {
                    return Some((
                        DragMotion::edge_reveal(shade_reveal_travel(h), 1.0),
                        Some(SceneKind::Shade),
                    ))
                }
                _ => {}
            }
        }
        // Keep bottom-edge workspace/Recents gestures separate from content drags.
        if drag.surface != SurfaceKind::Overlay
            || (!self.renderer.context().uses_touch_navigation() && drag.start_y >= h - 52.0)
        {
            return None;
        }
        if self.in_tablet_shade() {
            return self
                .tablet_shade_motion(drag, dx, dy)
                .map(|motion| (motion, None));
        }
        let progress = self.overlay_spring.value.clamp(0.0, 1.0);
        let closing = self.overlay_spring.target <= 0.0;
        if self.active_scene == SceneKind::Drawer {
            return None; // The Drawer list and its sheet share their own contact.
        }
        let tablet_home =
            self.tablet_navigation_available() && self.active_scene == SceneKind::Home;
        if tablet_home && self.tablet_navigation_available() && self.overlay_input_ready() {
            if let Some(motion) = tablet_home_vertical_motion(dx, dy, h) {
                return Some(motion);
            }
        }
        if self.has_gesture_overlay()
            && !tablet_home
            && vertical(dx, dy)
            && self.home_spring.is_none()
        {
            let (travel, sign) = if self.active_scene == SceneKind::Home {
                (home_reveal_travel(h), -1.0)
            } else {
                (shade_reveal_travel(h), 1.0)
            };
            if dy * sign < 0.0 || !self.overlay_spring.is_settled() {
                return Some((DragMotion::reveal(progress, travel, sign, closing), None));
            }
        }
        if matches!(self.active_scene, SceneKind::Home | SceneKind::Recents)
            && self.overlay_spring.is_settled()
            && progress >= 1.0
            && horizontal(dx, dy)
            && self.page_count() > 1
        {
            let ctx = self.renderer.context();
            let (page, stride) = if self.active_scene == SceneKind::Recents {
                (ctx.recents_index, recents_stride(w, h))
            } else {
                (ctx.home_page, w)
            };
            let from = self.home_spring.map_or(page as f32, |s| s.value);
            return Some((
                DragMotion::HomePage {
                    from,
                    width: stride.max(1.0),
                },
                None,
            ));
        }
        None
    }

    fn update_drawer_contact(&mut self, dx: f32, dy: f32) {
        let (w, h) = self.overlay_size();
        let Some(drag) = &mut self.drag else {
            return;
        };
        if drag.drawer.is_none() && vertical(dx, dy) {
            let ctx = self.renderer.context();
            let limit = drawer_max_scroll(w, h, ctx.home_apps.len());
            drag.drawer = Some(DrawerContact::new(ctx.drawer.scroll, limit, h));
        }
        if let Some(contact) = &mut drag.drawer {
            let (scroll, progress) = contact.update(dy);
            self.renderer.context_mut().drawer.scroll = scroll;
            self.overlay_spring = Spring::new(progress, SpringParams::snappy());
            self.draw_overlay();
        }
    }

    fn update_interactive_drag(&mut self, dx: f32, dy: f32) -> bool {
        let Some(drag) = &self.drag else {
            return false;
        };
        if drag.policy == DragPolicy::ClickOnly {
            return false;
        }
        if drag.active_slider.as_ref().is_some_and(|slider| {
            !self.in_tablet_shade() || slider.is_dragging() || !horizontal(dx, dy)
        }) {
            return false;
        }
        let tablet_origin = drag.tablet_origin;
        let down_x = drag.start_x;
        let Some((motion, opening)) = drag
            .motion
            .map(|m| (m, None))
            .or_else(|| self.start_interactive_motion(drag, dx, dy))
        else {
            return false;
        };
        if let Some(drag) = &mut self.drag {
            drag.motion = Some(motion);
            if matches!(motion, DragMotion::TabletShadePage { .. }) {
                drag.active_slider = None;
            }
        }
        let position = motion.position(dx, dy, self.page_count());
        let ctx = self.renderer.context_mut();
        ctx.pill_drag_x = 0.0;
        ctx.pill_drag_y = 0.0;
        ctx.top_drag_y = 0.0;
        match motion {
            DragMotion::Reveal { .. } => {
                let header_was_visible = self.tablet_shade_header_visible();
                let spring = Spring::new(position, SpringParams::snappy());
                if let Some(scene) = opening {
                    self.tablet_shade_origin = tablet_origin;
                    if tablet_origin.is_some() {
                        let (w, h) = self.overlay_size();
                        self.renderer.context_mut().shade.tablet_page =
                            tablet_shade_page_from_down(down_x, w, h);
                    }
                    let refresh = if scene == SceneKind::Drawer {
                        SceneRefresh::Unchanged
                    } else {
                        SceneRefresh::OnOpen
                    };
                    self.show_scene_at(scene, spring, refresh);
                    return true;
                }
                self.overlay_spring = spring;
                if header_was_visible != self.tablet_shade_header_visible() {
                    self.draw_top_strip();
                }
            }
            DragMotion::HomePage { .. } => {
                self.home_spring = Some(Spring::new(position, SpringParams::gentle()));
            }
            DragMotion::TabletShadePage { .. } => {
                self.tablet_shade_spring = Some(Spring::new(position, SpringParams::snappy()));
            }
        }
        self.draw_overlay();
        true
    }

    fn settle_interactive_drag(&mut self, motion: DragMotion, velocity: Option<(f32, f32)>) {
        let value = match motion {
            DragMotion::Reveal { .. } => self.overlay_spring.value,
            DragMotion::HomePage { from, .. } => {
                self.home_spring.map_or(from, |spring| spring.value)
            }
            DragMotion::TabletShadePage { from, .. } => {
                self.tablet_shade_spring.map_or(from, |spring| spring.value)
            }
        };
        let pages = self.page_count();
        let mut spring = velocity.map_or_else(
            || motion.cancel(value, pages),
            |speed| motion.release(value, speed, pages),
        );
        if velocity.is_none()
            && (motion.is_tablet_shade()
                || motion.is_drawer()
                || self.renderer.context().uses_touch_navigation()
                    && (self.active_scene == SceneKind::Home
                        || matches!(motion, DragMotion::HomePage { .. })))
        {
            // A lost grab can come with a hidden/rotating output. Don't depend
            // on another frame callback to restore the stable scene.
            spring.value = spring.target;
        }
        match motion {
            DragMotion::Reveal { .. } => self.overlay_spring = spring,
            DragMotion::HomePage { .. } => {
                self.home_spring = Some(spring);
                let page = spring.target as usize;
                if self.active_scene == SceneKind::Recents {
                    if self.renderer.context().recents_index != page {
                        self.renderer.context_mut().recents_index = page;
                        self.capture.invalidate();
                    }
                } else if self.renderer.context().home_page != page {
                    self.renderer.context_mut().home_page = page;
                    if velocity.is_some() {
                        self.handler.on_action(
                            ShellAction::HomePageChanged(page),
                            self.renderer.context_mut(),
                        );
                    }
                }
            }
            DragMotion::TabletShadePage { .. } => {
                self.tablet_shade_spring = Some(spring);
                self.renderer.context_mut().shade.tablet_page = if spring.target < 0.5 {
                    TabletShadePage::Notifications
                } else {
                    TabletShadePage::Controls
                };
            }
        }
        self.last_anim_tick = Instant::now();
        self.update_overlay_after_motion();
    }

    fn update_overlay_after_motion(&mut self) {
        if !self.overlay_motion_held() {
            if self.home_spring.is_some_and(|spring| spring.is_settled()) {
                self.home_spring = None;
            }
            if self
                .tablet_shade_spring
                .is_some_and(|spring| spring.is_settled())
            {
                self.tablet_shade_spring = None;
            }
            let settled = if self.tablet_overlay_owns_navigation()
                || (self.gap_profile() && self.in_tablet_shade())
            {
                snap_at_rest(&mut self.overlay_spring)
            } else {
                self.overlay_spring.is_settled()
            };
            if settled {
                if self.overlay_spring.target <= 0.0 && self.active_scene != SceneKind::Pill {
                    self.finish_overlay_close();
                    return;
                }
                if std::mem::take(&mut self.refresh_after_open) {
                    self.refresh_scene(self.active_scene);
                }
                self.sync_top_strip();
            }
        }
        self.draw_overlay();
    }

    fn finish_overlay_close(&mut self) {
        self.drag = None;
        let overview = self.tablet_overview.take();
        if self.tablet_shade_origin.take() == Some(TabletShadeOrigin::Home)
            || overview == Some(TabletOverviewOrigin::Home)
            || self.active_scene == SceneKind::Drawer
        {
            self.reset_handle_motion();
            self.show_scene_at(
                SceneKind::Home,
                Spring::new(1.0, SpringParams::snappy()),
                SceneRefresh::Unchanged,
            );
        } else {
            self.show_scene(SceneKind::Pill, false);
        }
    }

    fn update_tablet_bottom(&mut self, dx: f32, dy: f32) {
        let Some(nav) = self
            .drag
            .as_mut()
            .and_then(|drag| drag.tablet_bottom.as_mut())
        else {
            return;
        };
        nav.update(dx, dy);
        let origin = nav.origin;
        let progress = nav.overview_progress();
        self.draw_pill();

        if let Some(value) = progress {
            let spring = Spring::new(value, SpringParams::snappy());
            if self.active_scene == SceneKind::Recents {
                self.overlay_spring = spring;
            } else if value > 0.0 {
                self.tablet_overview = origin.overview();
                self.show_scene_at(SceneKind::Recents, spring, SceneRefresh::Now);
                return;
            }
        }
        self.draw_overlay();
    }

    fn finish_tablet_bottom(&mut self, nav: &BottomContact, velocity: (f32, f32)) {
        match nav.release(velocity) {
            NavRelease::Home => self.show_scene(SceneKind::Home, false),
            NavRelease::Overview => {
                self.draw_pill();
                if self.active_scene == SceneKind::Recents {
                    // Settle the deck we already refreshed and started
                    // capturing. Do not reopen it or change its anchor.
                    self.settle_tablet_overview(1.0);
                } else {
                    self.tablet_overview = nav.origin.overview();
                    let spring = spring_to(0.0, 1.0, 0.0, SpringParams::snappy());
                    self.show_scene_at(SceneKind::Recents, spring, SceneRefresh::Now);
                }
            }
            NavRelease::DismissOverview => {
                self.draw_pill();
                self.close_tablet_overview_to_home();
            }
            direction @ (NavRelease::Older | NavRelease::Newer) => {
                let offset = if direction == NavRelease::Older {
                    1
                } else {
                    -1
                };
                let output = self
                    .output_name
                    .as_deref()
                    .filter(|_| self.target_output.is_some());
                if switch_tablet_app(
                    self.handler.as_mut(),
                    output,
                    offset,
                    nav.origin == NavOrigin::Home,
                    self.renderer.context(),
                ) {
                    self.show_scene(SceneKind::Pill, false);
                } else {
                    self.draw_pill();
                    self.draw_overlay();
                }
            }
            NavRelease::Cancel => {
                self.draw_pill();
                if nav.origin != NavOrigin::Recents && self.active_scene == SceneKind::Recents {
                    self.settle_tablet_overview(0.0);
                } else {
                    self.draw_overlay();
                }
            }
        }
    }

    fn close_tablet_overview_to_home(&mut self) {
        // Unlike a canceled preview, an explicit close always reveals Home.
        self.tablet_overview = Some(TabletOverviewOrigin::Home);
        self.settle_tablet_overview(0.0);
    }

    fn settle_tablet_overview(&mut self, target: f32) {
        self.overlay_spring = spring_to(
            self.overlay_spring.value,
            target,
            0.0,
            SpringParams::snappy(),
        );
        self.last_anim_tick = Instant::now();
        self.update_overlay_after_motion();
    }

    fn reset_handle_motion(&mut self) {
        let ctx = self.renderer.context_mut();
        ctx.pill_drag_x = 0.0;
        ctx.pill_drag_y = 0.0;
        ctx.top_drag_y = 0.0;
        ctx.pill_pressed = false;
        ctx.top_pressed = false;
    }

    fn abort_drag(&mut self) {
        let drag = self.drag.take();
        self.reset_handle_motion();
        self.renderer.context_mut().recents_throw = None;
        self.draw_pill();
        self.draw_top_strip();
        if let Some(drag) = drag {
            if drag.tablet_bottom.is_some_and(|nav| {
                nav.origin != NavOrigin::Recents && self.tablet_overview.is_some()
            }) {
                // A lost grab/rotation cannot rely on another animation frame.
                self.finish_overlay_close();
                return;
            }
            if let Some(contact) = drag.drawer {
                self.renderer.context_mut().drawer.scroll = contact.origin_scroll();
                self.overlay_spring = Spring::new(1.0, SpringParams::snappy());
            }
            if let Some(motion) = drag.motion {
                self.settle_interactive_drag(motion, None);
                return;
            }
        }
        self.draw_overlay();
    }

    fn update_drag(&mut self, touch_id: Option<i32>, x: f32, y: f32, time: u32) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        if drag.touch_id != touch_id {
            return;
        }
        if drag.uses_input_time() {
            let Some(at) = drag.tablet_event_time(time) else {
                return;
            };
            if !drag.velocity.accepts_input_event(at) {
                return;
            }
            drag.velocity.push_input_event(at, x, y);
        } else {
            drag.velocity.push(Instant::now(), x, y);
        }
        let (dx, dy) = drag.track_position(x, y);
        if drag.keyboard_button.is_some()
            || (drag.policy == DragPolicy::ClickOnly && drag.active_slider.is_none())
        {
            return;
        }
        if drag.tablet_bottom.is_some() {
            self.update_tablet_bottom(dx, dy);
            return;
        }
        if let Some(id) = drag.recent_card.as_ref().filter(|_| drag.motion.is_none()) {
            let ctx = self.renderer.context_mut();
            if ctx.recents_throw.is_some() || (dy < 0.0 && vertical(dx, dy)) {
                ctx.recents_throw = Some((id.clone(), dy.min(0.0)));
                self.draw_overlay();
                return;
            }
        }
        if self.update_interactive_drag(dx, dy) {
            return;
        }
        if self.active_scene == SceneKind::Drawer {
            self.update_drawer_contact(dx, dy);
            return;
        }
        let (surface, start_x, start_y, has_slider) = {
            let Some(drag) = &self.drag else {
                return;
            };
            if drag.tablet_origin.is_some() {
                return;
            }
            (
                drag.surface,
                drag.start_x,
                drag.start_y,
                drag.active_slider.is_some(),
            )
        };

        if has_slider {
            let tablet_shade = self.in_tablet_shade();
            let pct = {
                let slider = self.drag.as_mut().and_then(|d| d.active_slider.as_mut());
                slider
                    .filter(|s| !tablet_shade || s.is_dragging() || dy.abs() > dx.abs() * 1.18)
                    .and_then(|slider| {
                        slider
                            .update(dy, Instant::now())
                            .map(|pct| (slider.id, pct))
                    })
            };
            if let Some((id, pct)) = pct {
                self.handler.on_action(
                    ShellAction::SetSlider { id, pct },
                    self.renderer.context_mut(),
                );
                self.draw_overlay();
            }
            return;
        }

        match surface {
            SurfaceKind::Wallpaper => {}
            SurfaceKind::Pill => {
                self.renderer.context_mut().pill_drag_x = x - start_x;
                self.renderer.context_mut().pill_drag_y = y - start_y;
                self.draw_pill();
            }
            SurfaceKind::TopStrip => {
                self.renderer.context_mut().top_drag_y = (y - start_y).max(0.0);
                self.draw_top_strip();
            }
            SurfaceKind::Overlay => {
                if !self.renderer.context().uses_touch_navigation()
                    && start_y >= (self.viewport_h as f32 - 52.0)
                {
                    self.renderer.context_mut().pill_drag_x = x - start_x;
                    self.renderer.context_mut().pill_drag_y = y - start_y;
                    self.draw_overlay();
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn finish_drag(&mut self, touch_id: Option<i32>, time: u32) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.touch_id != touch_id {
            self.drag = Some(drag);
            return;
        }

        let velocity = if drag.uses_input_time() {
            let Some(at) = drag
                .tablet_event_time(time)
                .filter(|&at| drag.velocity.accepts_input_event(at))
            else {
                // A queued up from before this contact cannot release it.
                self.drag = Some(drag);
                return;
            };
            drag.velocity.at_input_release(at)
        } else {
            drag.velocity.at_release(Instant::now())
        };
        if self.gap_edge(drag.surface).is_some() {
            // This release ends the strip's implicit grab. If there is a
            // foreground panel now, drop the strip and pause refresh.
            self.sync_gap_strips();
        }
        self.reset_handle_motion();
        if let Some(button) = drag.keyboard_button {
            if self.active_scene == SceneKind::Pill
                && handle_tap(true, drag.max_travel)
                && self.keyboard_button_at(drag.last_x, drag.last_y) == Some(button)
            {
                let _ = self.show_keyboard();
            }
            return;
        }
        if let Some(nav) = drag.tablet_bottom {
            self.finish_tablet_bottom(&nav, velocity);
            return;
        }
        if let Some(contact) = drag.drawer {
            self.overlay_spring = contact.release(velocity);
            self.last_anim_tick = Instant::now();
            self.update_overlay_after_motion();
            return;
        }
        if let Some((id, offset)) = self.renderer.context_mut().recents_throw.take() {
            let (_, vy) = velocity;
            if offset <= -96.0 || (offset <= -32.0 && vy <= -650.0) {
                self.handler
                    .on_action(ShellAction::CloseWindow(id), self.renderer.context_mut());
                self.capture.invalidate();
            }
            self.draw_overlay();
            return;
        }
        if let Some(motion) = drag.motion {
            if drag.surface != SurfaceKind::Overlay && drag.tablet_origin.is_none() {
                self.draw_surface(drag.surface);
            }
            self.settle_interactive_drag(motion, Some(velocity));
            return;
        }
        if drag.tablet_origin.is_some() {
            return;
        }

        let elapsed = drag.start_time.elapsed().as_secs_f32().max(0.016);
        let dx = drag.last_x - drag.start_x;
        let dy = drag.last_y - drag.start_y;
        let vx = dx / elapsed;
        let vy = dy / elapsed;
        let dist = dx.hypot(dy);

        match drag.surface {
            SurfaceKind::Wallpaper => {}
            SurfaceKind::Pill if self.renderer.context().uses_touch_navigation() => {
                self.draw_pill();
            }
            SurfaceKind::Pill => {
                if handle_tap(drag.handle_tap_allowed, drag.max_travel) {
                    let scene = if self.active_scene == SceneKind::Pill {
                        SceneKind::Home
                    } else {
                        SceneKind::Pill
                    };
                    self.show_scene(scene, true);
                    return;
                }
                if self.active_scene == SceneKind::Pill && !horizontal(dx, dy) {
                    self.draw_pill();
                    return;
                }
                let sample = GestureSample {
                    dx,
                    dy,
                    vx,
                    vy,
                    viewport_height: (self.viewport_h as f32).max(600.0),
                    has_foreground: self.active_scene == SceneKind::Pill,
                };
                match classify_bottom_pill(sample, GestureThresholds::default()) {
                    GestureAction::Home => {
                        self.show_scene(SceneKind::Home, true);
                    }
                    GestureAction::Recents => self.show_scene(SceneKind::Recents, true),
                    GestureAction::SwitchNext => {
                        self.handler.on_action(
                            ShellAction::SwitchWorkspace(1),
                            self.renderer.context_mut(),
                        );
                        self.draw_pill();
                    }
                    GestureAction::SwitchPrev => {
                        self.handler.on_action(
                            ShellAction::SwitchWorkspace(-1),
                            self.renderer.context_mut(),
                        );
                        self.draw_pill();
                    }
                    _ => self.draw_pill(),
                }
            }
            SurfaceKind::TopStrip => {
                if self.uses_desk_panel() {
                    self.finish_desk_panel_tap(&drag);
                } else if !self.gap_profile()
                    && handle_tap(drag.handle_tap_allowed, drag.max_travel)
                {
                    self.show_scene(SceneKind::Shade, true);
                } else {
                    self.draw_top_strip();
                }
            }
            SurfaceKind::Overlay => {
                let tablet_shade = self.in_tablet_shade();
                if let Some(slider) = drag.active_slider {
                    if !tablet_shade
                        || slider.is_dragging()
                        || (drag.max_travel < 8.0 && slider.rect.contains(drag.last_x, drag.last_y))
                    {
                        let pct = slider.finish(drag.last_y, dy);
                        self.handler.on_action(
                            ShellAction::SetSlider { id: slider.id, pct },
                            self.renderer.context_mut(),
                        );
                    }
                    self.draw_overlay();
                    return;
                }

                if self.active_scene == SceneKind::Osk {
                    if drag.policy != DragPolicy::ClickOnly
                        && dy >= 60.0
                        && vy >= 220.0
                        && vertical(dx, dy)
                    {
                        self.show_scene(SceneKind::Pill, false);
                    } else if drag.max_travel < 24.0 {
                        if let Some(label) = drag.osk_contact.and_then(OskContact::tap) {
                            self.press_osk_key(label);
                        }
                    }
                    return;
                }

                // Bottom pill swipe inside overlay behaves identically to the bottom pill
                if !self.renderer.context().uses_touch_navigation()
                    && drag.start_y >= (self.viewport_h as f32 - 52.0)
                    && dist >= 20.0
                {
                    let sample = GestureSample {
                        dx,
                        dy,
                        vx,
                        vy,
                        viewport_height: (self.viewport_h as f32).max(600.0),
                        has_foreground: false,
                    };
                    match classify_bottom_pill(sample, GestureThresholds::default()) {
                        GestureAction::Home | GestureAction::Recents => {
                            let next = if self.active_scene != SceneKind::Home
                                && dy < -70.0
                                && vy < -450.0
                            {
                                SceneKind::Pill
                            } else {
                                SceneKind::Recents
                            };
                            self.show_scene(next, true);
                            return;
                        }
                        GestureAction::SwitchNext => {
                            self.handler.on_action(
                                ShellAction::SwitchWorkspace(1),
                                self.renderer.context_mut(),
                            );
                            self.draw_overlay();
                            return;
                        }
                        GestureAction::SwitchPrev => {
                            self.handler.on_action(
                                ShellAction::SwitchWorkspace(-1),
                                self.renderer.context_mut(),
                            );
                            self.draw_overlay();
                            return;
                        }
                        _ => {}
                    }
                }

                // Upward flick closes open overlay back to the active app
                let has_content_motion = tablet_shade
                    || self.has_gesture_overlay()
                    || (self.renderer.context().uses_touch_navigation()
                        && self.active_scene == SceneKind::Recents);
                if drag.policy != DragPolicy::ClickOnly
                    && !has_content_motion
                    && dy <= -72.0
                    && vy <= -350.0
                {
                    self.show_scene(SceneKind::Pill, false);
                    return;
                }

                // Short tap -> hit-test UI targets
                let tapped = if drag.tablet_contact == TabletContact::Content || self.gap_profile()
                {
                    tablet_content_tap(
                        self.overlay_layout.as_ref().map(|frame| &frame.layout),
                        drag.down_target.as_ref(),
                        (drag.last_x, drag.last_y),
                        drag.max_travel,
                    )
                } else if self.uses_tablet_status() {
                    drag.max_travel < 18.0
                } else {
                    dist < 18.0
                };
                if drag.overlay_tap_allowed && tapped {
                    self.handle_overlay_tap(drag.last_x, drag.last_y);
                } else {
                    self.draw_overlay();
                }
            }
        }
    }

    fn finish_desk_panel_tap(&mut self, drag: &ActiveDrag) {
        if !handle_tap(true, drag.max_travel) {
            return;
        }
        let Some(panel) = &self.top_strip else { return };
        for action in veilshell_render::desk_panel_actions(panel.width as f32) {
            if action.bounds.contains(drag.start_x, drag.start_y)
                && action.bounds.contains(drag.last_x, drag.last_y)
            {
                let scene = if self.active_scene == action.scene {
                    SceneKind::Pill
                } else {
                    action.scene
                };
                self.show_scene(scene, false);
                break;
            }
        }
    }

    fn handle_overlay_tap(&mut self, x: f32, y: f32) {
        if !self.overlay_input_ready() {
            return;
        }
        let target = self
            .overlay_layout
            .as_ref()
            .and_then(|frame| frame.layout.hit_test(x, y))
            .cloned();

        let Some(hit) = target else {
            self.draw_overlay();
            return;
        };

        match hit {
            HitTarget::DismissOverlay => self.dismiss_from_control(y),
            HitTarget::BottomPill if self.renderer.context().uses_touch_navigation() => {}
            HitTarget::BottomPill => {
                self.show_scene(SceneKind::Pill, true);
            }
            HitTarget::LaunchApp { index } => {
                self.launch_app_at(index);
            }
            HitTarget::HomePage { page } => {
                self.renderer.context_mut().home_page = page;
                self.handler.on_action(
                    ShellAction::HomePageChanged(page),
                    self.renderer.context_mut(),
                );
                self.draw_overlay();
            }
            HitTarget::FocusWindow { id } => {
                self.handler
                    .on_action(ShellAction::FocusWindow(id), self.renderer.context_mut());
                self.show_scene(SceneKind::Pill, false);
            }
            HitTarget::ShadeToggle { id } => {
                self.handler
                    .on_action(ShellAction::ToggleSetting(id), self.renderer.context_mut());
                if id == "desk_mode" {
                    if !self.uses_tablet_status() {
                        self.tablet_shade_origin = None;
                    }
                    self.sync_surfaces();
                    self.draw_top_strip();
                }
                if id == "tablet_mode" && !self.renderer.context().tablet_mode {
                    self.set_visible(false);
                } else {
                    self.draw_overlay();
                }
            }
            HitTarget::ShadeAction(ShadeAction::Settings) => {
                self.show_scene(SceneKind::Settings, false);
            }
            HitTarget::ShadeAction(action) => self.apply_shade_action(action),
            HitTarget::Settings(veilshell_render::SettingsAction::Back) => {
                self.show_scene(SceneKind::Shade, false);
            }
            HitTarget::Settings(action) => {
                self.handler
                    .on_action(ShellAction::Settings(action), self.renderer.context_mut());
                self.sync_surfaces();
                if action == veilshell_render::SettingsAction::Wallpaper {
                    self.draw_wallpaper();
                }
                self.draw_top_strip();
                self.draw_overlay();
            }
            HitTarget::ShadeSlider { id, rect } => {
                let pct = slider_pct_for_y(y, rect);
                self.handler.on_action(
                    ShellAction::SetSlider { id, pct },
                    self.renderer.context_mut(),
                );
                self.draw_overlay();
            }
            HitTarget::Power(input) => self.handle_power_input(input),
            HitTarget::LauncherPanel
            | HitTarget::PowerPanel
            | HitTarget::OpenKeyboard
            | HitTarget::OskKey { .. } => {}
        }
    }

    fn dismiss_from_control(&mut self, y: f32) {
        if self.in_tablet_shade() {
            if self.uses_tablet_status()
                && self.tablet_shade_main()
                && self.tablet_shade_status_at(y)
            {
                return;
            }
            self.overlay_spring.set_target(0.0);
            self.overlay_spring.params = SpringParams::gentle();
            self.last_anim_tick = Instant::now();
            self.update_overlay_after_motion();
        } else if self.active_scene == SceneKind::Recents
            && self.renderer.context().uses_touch_navigation()
        {
            self.close_tablet_overview_to_home();
        } else if self.active_scene == SceneKind::Drawer {
            self.finish_overlay_close();
        } else {
            self.show_scene(SceneKind::Pill, true);
        }
    }

    fn launch_app_at(&mut self, index: usize) {
        if let Some(app) = self.renderer.context().home_apps.get(index) {
            let action = ShellAction::LaunchApp {
                id: app.id.clone(),
                exec: app.exec.clone(),
            };
            self.handler.on_action(action, self.renderer.context_mut());
            self.show_scene(SceneKind::Pill, false);
        }
    }

    fn reset_shade_page(&mut self) {
        let shade = &mut self.renderer.context_mut().shade;
        shade.page = ShadePage::Main;
        shade.tablet_page = TabletShadePage::default();
        shade.tablet_page_position = None;
        shade.page_offset = 0;
        shade.choices.clear();
        shade.pending = None;
    }

    fn apply_shade_action(&mut self, action: ShadeAction) {
        let shade = &mut self.renderer.context_mut().shade;
        if shade.pending.is_some() {
            return;
        }
        shade.feedback = None;
        match action {
            ShadeAction::Show(page) => {
                shade.page = page;
                shade.page_offset = 0;
                shade.choices.clear();
            }
            ShadeAction::Page(offset) => {
                shade.page_offset = offset;
                self.draw_overlay();
                return;
            }
            _ => {}
        }
        let capture = action == ShadeAction::Screenshot;
        self.handler
            .on_action(ShellAction::Shade(action), self.renderer.context_mut());
        if capture && self.renderer.context().shade.pending.is_some() {
            self.show_scene(SceneKind::Pill, false);
        } else {
            self.draw_overlay();
        }
    }

    fn finish_shade(&mut self, ticket: u64, result: Result<ShadeReply, String>) {
        let shade = &mut self.renderer.context_mut().shade;
        if self.active_scene != SceneKind::Shade || shade.pending != Some(ticket) {
            return;
        }
        shade.pending = None;
        match result {
            Ok(ShadeReply::Discovered(extras, quiet)) => {
                shade.extras = extras;
                shade.quiet = quiet;
            }
            Ok(ShadeReply::Choices(choices)) => {
                shade.page_offset = 0;
                shade.choices = choices;
            }
            Ok(ShadeReply::AudioOutput(choices, muted)) => {
                shade.page_offset = 0;
                shade.choices = choices;
                shade.extras.muted = muted;
            }
            Ok(ShadeReply::Muted(muted)) => shade.extras.muted = Some(muted),
            Ok(ShadeReply::Quiet(quiet)) => shade.quiet = Some(quiet),
            Ok(ShadeReply::Media(track)) => shade.extras.media = track,
            Ok(ShadeReply::Dismiss) => {
                self.show_scene(SceneKind::Pill, false);
                return;
            }
            Err(message) => shade.feedback = Some(message),
        }
        self.draw_overlay();
    }

    fn press_osk_key(&mut self, label: &'static str) {
        let state = &mut self.renderer.context_mut().osk_state;
        let previous = *state;
        let action = state.press(label);
        let changed = *state != previous;
        if action == OskEmit::Dismiss {
            self.show_scene(SceneKind::Pill, false);
        } else {
            self.osk.emit(action, &self.qh);
            if changed {
                self.draw_overlay();
            }
        }
    }

    pub(crate) fn cancel_osk_drag(&mut self) {
        if self.active_scene == SceneKind::Osk {
            self.drag = None;
        }
    }

    pub(crate) fn invalidate_osk_layout(&mut self) {
        if self.active_scene == SceneKind::Osk {
            self.cancel_osk_drag();
            self.overlay_layout = None;
        }
    }

    fn reflow_osk(&mut self) {
        self.invalidate_osk_layout();
        let spec =
            LayerSurfaceSpec::for_role(LayerRole::TouchOsk, self.viewport_w, self.viewport_h);
        if let Some(surface) = &mut self.overlay {
            if surface.height != spec.height {
                // Commit the new work-area reservation with the new buffer.
                // Keep the OSK surface and seat binding rather than closing it.
                surface.layer.set_size(0, spec.height);
                surface.layer.set_exclusive_zone(spec.exclusive_zone);
                surface.height = spec.height;
            }
        }
        self.draw_overlay();
    }

    fn osk_repeat_timeout(&self) -> Option<Duration> {
        if self.active_scene != SceneKind::Osk {
            return None;
        }
        self.drag
            .as_ref()?
            .osk_contact
            .as_ref()?
            .deadline()
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    fn tick_osk_repeat(&mut self) {
        if self.active_scene != SceneKind::Osk {
            return;
        }
        if let Some(key) = self
            .drag
            .as_mut()
            .and_then(|drag| drag.osk_contact.as_mut())
            .and_then(|repeat| repeat.tick(Instant::now()))
        {
            self.press_osk_key(key);
        }
    }

    fn handle_power_input(&mut self, input: PowerInput) {
        if self.active_scene != SceneKind::Shade {
            return;
        }
        let Some(menu) = &mut self.renderer.context_mut().power_menu else {
            return;
        };
        match menu.input(input) {
            Some(PowerAction::Logout) => self.exit = true,
            Some(action) => self.handler.on_action(
                ShellAction::RequestPower(action),
                self.renderer.context_mut(),
            ),
            None => {}
        }
        self.draw_overlay();
    }
}

/// Start the single-surface compatibility wrapper around [`run_shell_daemon`].
///
/// # Errors
/// Returns [`WaylandError`] if the `Wayland` connection or globals fail.
pub fn run_layer_surface(
    config: &WaylandRuntimeConfig,
    role: LayerRole,
    palette: ThemePalette,
    scene_ctx: SceneContext,
) -> Result<(), WaylandError> {
    let (_tx, rx) = channel::channel();
    run_shell_daemon(
        config,
        role.to_scene(),
        palette,
        scene_ctx,
        rx,
        NoopActionHandler,
    )
}

/// Run the full multi-surface `calloop` + `wlr-layer-shell` shell daemon.
///
/// While dormant over apps, only the configured edge surfaces are mapped;
/// full-screen overlays are dropped upon dismissal.
///
/// # Errors
/// Returns [`WaylandError`] if connecting to `Wayland`, binding required globals, or selecting the
/// requested output fails, or if the explicitly requested GPU renderer cannot present.
#[allow(clippy::too_many_lines)]
pub fn run_shell_daemon<H: ShellActionHandler + 'static>(
    config: &WaylandRuntimeConfig,
    initial_scene: SceneKind,
    palette: ThemePalette,
    mut initial_ctx: SceneContext,
    cmd_rx: Channel<DaemonCommand>,
    handler: H,
) -> Result<(), WaylandError> {
    if let Some(display) = &config.display_override {
        std::env::set_var("WAYLAND_DISPLAY", display);
    }
    let conn = Connection::connect_to_env().map_err(|e| WaylandError::Connect(e.to_string()))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|e| WaylandError::Runtime(e.to_string()))?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| WaylandError::MissingGlobal(format!("wl_compositor: {e}")))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|e| WaylandError::MissingGlobal(format!("zwlr_layer_shell_v1: {e}")))?;
    let shm = Shm::bind(&globals, &qh)
        .map_err(|e| WaylandError::MissingGlobal(format!("wl_shm: {e}")))?;

    initial_ctx.overlay_progress = 1.0;
    #[cfg(feature = "wgpu")]
    let renderer = init_renderer(config.renderer, initial_ctx, &conn, || {
        compositor.create_surface(&qh).into()
    });
    #[cfg(not(feature = "wgpu"))]
    let renderer = ActiveRenderer::new(config.renderer, initial_ctx);
    let mut renderer =
        renderer.map_err(|e| WaylandError::Runtime(format!("renderer init: {e}")))?;
    if let Some(wp) = &config.wallpaper {
        if let Err(err) = renderer.set_wallpaper(Some(wp)) {
            eprintln!("veilshell: failed to pre-blur wallpaper: {err}");
        }
    }

    let mut settled_spring = Spring::new(1.0, SpringParams::snappy());
    settled_spring.set_target(1.0);

    let mut app = LayerApp {
        qh: qh.clone(),
        registry_state: RegistryState::new(&globals),
        compositor,
        layer_shell,
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        fractional_scale: FractionalScaleGlobals::bind(&globals, &qh),
        shm,
        target_output: None,
        output_name: config.output_name.clone(),
        viewport_w: 1280,
        viewport_h: 853,
        output_transform: wl_output::Transform::Normal,
        gaps: GapRefresh::default(),
        visible: config.initial_visible,
        active_scene: if initial_scene == SceneKind::Drawer
            && !tablet_navigation_available(config.initial_visible, renderer.context())
        {
            SceneKind::Pill
        } else {
            initial_scene
        },
        pill: None,
        top_strip: None,
        overlay: None,
        wallpaper: None,
        overlay_layout: None,
        overlay_spring: settled_spring,
        home_spring: None,
        tablet_shade_spring: None,
        tablet_shade_origin: None,
        tablet_overview: None,
        refresh_after_open: false,
        last_anim_tick: Instant::now(),
        palette,
        renderer,
        #[cfg(feature = "wgpu")]
        renderer_preference: config.renderer,
        render_failure: None,
        pointer: None,
        touch: None,
        launcher_input: LauncherInput::default(),
        drag: None,
        osk: OskWaylandState::bind_globals(&globals, &qh, config.osk_policy),
        capture: ToplevelCaptureState::bind_globals(&globals, &qh),
        handler: Box::new(handler),
        exit: false,
        conn: conn.clone(),
    };

    // Perform two quick roundtrips so OutputState populates `xdg_output` names & logical sizes.
    event_queue
        .roundtrip(&mut app)
        .map_err(|e| WaylandError::Runtime(e.to_string()))?;
    event_queue
        .roundtrip(&mut app)
        .map_err(|e| WaylandError::Runtime(e.to_string()))?;

    let matched = app.output_state.outputs().find_map(|out| {
        let info = app.output_state.info(&out)?;
        if let Some(wanted) = &config.output_name {
            if info.name.as_deref() == Some(wanted.as_str()) {
                return Some((out, info));
            }
            return None;
        }
        Some((out, info))
    });

    if let Some((out, info)) = matched {
        if let Some((w, h)) = output_logical_size(&info) {
            (app.viewport_w, app.viewport_h) = (w, h);
            app.gaps.viewport_valid = true;
        }
        app.output_transform = info.transform;
        if app.output_name.is_none() {
            app.output_name = info.name;
        }
        app.target_output = Some(out);
    } else if let Some(wanted) = &config.output_name {
        return Err(WaylandError::Runtime(format!(
            "requested Wayland output '{wanted}' was not found"
        )));
    }

    if config.initial_visible {
        app.sync_osk_visibility();
        if !app.foreground_available() {
            app.active_scene = SceneKind::Pill;
        }
        if initial_scene == SceneKind::Osk {
            if let Err(reason) = app.show_keyboard() {
                eprintln!("veilshell: cannot show OSK: {reason}");
                app.show_scene(SceneKind::Pill, false);
            }
        } else {
            app.show_scene(app.active_scene, false);
        }
    }

    let mut event_loop: EventLoop<LayerApp> =
        EventLoop::try_new().map_err(|e| WaylandError::Runtime(e.to_string()))?;
    let loop_handle: LoopHandle<'static, LayerApp> = event_loop.handle();
    let flush_conn = conn.clone();

    WaylandSource::new(conn, event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| WaylandError::Runtime(e.to_string()))?;

    loop_handle
        .insert_source(cmd_rx, |event, (), state| {
            if let ChannelEvent::Msg(cmd) = event {
                state.handle_daemon_command(cmd);
            }
        })
        .map_err(|e| WaylandError::Runtime(e.to_string()))?;

    if let Some(ms) = config.exit_after_ms {
        loop_handle
            .insert_source(
                Timer::from_duration(Duration::from_millis(ms)),
                |_, (), state| {
                    state.exit = true;
                    TimeoutAction::Drop
                },
            )
            .map_err(|e| WaylandError::Runtime(e.to_string()))?;
    }

    while !app.exit {
        event_loop
            .dispatch(app.event_timeout(), &mut app)
            .map_err(|e| WaylandError::Runtime(e.to_string()))?;
        if !app.exit {
            app.tick_osk_repeat();
            #[cfg(feature = "wgpu")]
            app.retry_swapchain_draws();
            app.refresh_gap_strips();
        }
    }

    app.capture.destroy();
    app.osk.destroy();
    let _ = flush_conn.flush();

    // Explicitly tear down all Wayland + wgpu surfaces before the event loop drops.
    app.overlay = None;
    app.top_strip = None;
    app.pill = None;

    app.render_failure
        .map_or(Ok(()), |err| Err(WaylandError::Runtime(err)))
}

impl CompositorHandler for LayerApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        // SCTK delivers preferred_buffer_scale on v6 and output-derived
        // scale changes on older surfaces. Input stays in logical pixels.
        if let Some(kind) = self.surface_kind(surface) {
            self.update_surface_scale(kind, |scaling| scaling.update_integer(new_factor));
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        if self.drag.as_ref().is_some_and(|drag| {
            drag.uses_input_time() && self.surface_kind(surface) == Some(drag.surface)
        }) {
            self.abort_drag();
        }
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if self.surface_kind(surface) != Some(SurfaceKind::Overlay) {
            return;
        }
        let Some(surf) = &mut self.overlay else {
            return;
        };
        surf.frame_pending = false;
        let dirty = std::mem::take(&mut surf.redraw_pending);
        let held = self.overlay_motion_held();
        let settling = !held
            && (!self.overlay_spring.is_settled()
                || self.home_spring.is_some()
                || self.tablet_shade_spring.is_some());
        if settling {
            let now = Instant::now();
            let dt = now.duration_since(self.last_anim_tick).as_secs_f32();
            self.last_anim_tick = now;
            let header_was_visible = self.tablet_shade_header_visible();
            self.overlay_spring.step(dt);
            if let Some(spring) = &mut self.home_spring {
                spring.step(dt);
            }
            if let Some(spring) = &mut self.tablet_shade_spring {
                spring.step(dt);
            }
            if header_was_visible != self.tablet_shade_header_visible() {
                self.draw_top_strip();
            }
        }
        if settling || dirty {
            self.update_overlay_after_motion();
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        if self.target_output.as_ref() != Some(output) {
            return;
        }
        let kind = self.surface_kind(surface);
        if kind.and_then(|kind| self.gap_edge(kind)).is_some() {
            self.invalidate_gap_configuration();
        } else if self
            .drag
            .as_ref()
            .is_some_and(|drag| drag.uses_input_time() && kind == Some(drag.surface))
        {
            self.abort_drag();
        }
    }
}

impl OutputHandler for LayerApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.gap_profile() && self.target_output.is_none() && self.output_name.is_some() {
            let same_name = self
                .output_state
                .info(&output)
                .is_some_and(|info| info.name == self.output_name);
            if same_name {
                // Rebind only the lost named output, never another desktop.
                self.target_output = Some(output.clone());
                self.update_output(conn, qh, output);
            }
        }
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.target_output.as_ref() != Some(&output) {
            return;
        }
        let Some(info) = self.output_state.info(&output) else {
            return;
        };
        let size = output_logical_size(&info);
        let viewport_changed = self.gaps.viewport_valid != size.is_some();
        self.gaps.viewport_valid = size.is_some();
        let new_size = size.unwrap_or((self.viewport_w, self.viewport_h));
        let geometry_changed = new_size != (self.viewport_w, self.viewport_h)
            || info.transform != self.output_transform;
        if !(geometry_changed || self.gap_profile() && viewport_changed) {
            return;
        }
        (self.viewport_w, self.viewport_h) = new_size;
        self.output_transform = info.transform;
        if self.gap_profile() {
            self.overlay_layout = None;
            self.invalidate_gap_configuration();
            if !self.gap_output_ready() {
                self.show_scene(SceneKind::Pill, false);
                return;
            }
        }
        if self.active_scene == SceneKind::Osk {
            self.reflow_osk();
        } else if self.tablet_navigation_drag()
            || self.tablet_shade_drag().is_some()
            || self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.keyboard_button.is_some())
        {
            self.abort_drag();
        } else if self.tablet_shade_spring.take().is_some() {
            // Rotation after release keeps the committed page, not the
            // interpolated position from the old viewport.
            self.overlay_layout = None;
            self.draw_overlay();
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.target_output.as_ref() != Some(&output) {
            return;
        }
        if self.uses_tablet_status() {
            self.target_output = None;
            self.output_name = None;
            self.set_visible(false);
        } else if self.gap_profile() {
            self.target_output = None;
            self.gaps.viewport_valid = false;
            self.invalidate_gap_configuration();
            self.show_scene(SceneKind::Pill, false);
        }
    }
}

impl LayerShellHandler for LayerApp {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if self
            .surface_kind(layer.wl_surface())
            .and_then(|kind| self.gap_edge(kind))
            .is_some()
        {
            self.invalidate_gap_configuration();
            return;
        }
        match self.surface_kind(layer.wl_surface()) {
            Some(SurfaceKind::Overlay) => {
                self.overlay = None;
                self.finish_overlay_close();
            }
            Some(SurfaceKind::Pill) => {
                if self.tablet_navigation_drag() {
                    self.abort_drag();
                }
                self.exit = true;
            }
            Some(SurfaceKind::Wallpaper) => self.wallpaper = None,
            Some(SurfaceKind::TopStrip) if self.tablet_status_drag() => {
                self.top_strip = None;
                self.abort_drag();
                self.sync_top_strip();
            }
            Some(SurfaceKind::TopStrip) | None => {}
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(kind) = self.surface_kind(layer.wl_surface()) else {
            return;
        };
        if let Some(edge) = self.gap_edge(kind) {
            self.configure_gap_strip(edge, configure.new_size);
            return;
        }
        let is_osk = kind == SurfaceKind::Overlay && self.active_scene == SceneKind::Osk;
        let (w_hint, h_hint) = configure.new_size;
        let known_width = self.viewport_w;
        let mut resized = false;
        if let Some(target) = self.managed_surface_mut(kind) {
            let width = if w_hint > 0 {
                w_hint
            } else if is_osk {
                known_width
            } else {
                target.width
            };
            // A bottom-anchored OSK chooses its own height; a configure queued
            // before reflow must not restore the old height or reservation.
            let height = if !is_osk && h_hint > 0 {
                h_hint
            } else {
                target.height
            };
            resized = target.configured && (target.width, target.height) != (width, height);
            target.width = width;
            target.height = height;
            target.configured = true;
        }
        if resized && is_osk {
            self.invalidate_osk_layout();
        }

        match kind {
            SurfaceKind::Wallpaper => self.draw_wallpaper(),
            SurfaceKind::Pill => {
                if resized
                    && self.drag.as_ref().is_some_and(|drag| {
                        drag.keyboard_button.is_some()
                            || drag.surface == kind && drag.tablet_bottom.is_some()
                    })
                {
                    self.abort_drag();
                } else {
                    self.draw_pill();
                }
            }
            SurfaceKind::TopStrip => {
                if resized && self.tablet_status_drag() {
                    self.abort_drag();
                } else {
                    self.draw_top_strip();
                }
            }
            SurfaceKind::Overlay => {
                if self.target_output.is_none() && !is_osk {
                    if w_hint > 0 {
                        self.viewport_w = w_hint;
                    }
                    if h_hint > 0 {
                        self.viewport_h = h_hint;
                    }
                }
                if resized && (self.in_tablet_shade() || self.tablet_navigation_available()) {
                    self.overlay_layout = None;
                }
                if resized
                    && (self.tablet_navigation_drag()
                        || self.tablet_shade_drag().is_some()
                        || self.drag.as_ref().is_some_and(|drag| drag.motion.is_some()))
                {
                    if self.gap_profile() {
                        self.invalidate_gap_configuration();
                        if self.drag.is_some() {
                            self.abort_drag();
                        }
                    } else {
                        self.abort_drag();
                    }
                } else {
                    if resized {
                        self.tablet_shade_spring = None;
                    }
                    self.draw_overlay();
                }
            }
        }
    }
}

impl SeatHandler for LayerApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.visible {
            self.osk.attach_seat(&seat, qh);
            self.draw_pill();
        }
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Ok(keyboard) = self.seat_state.get_keyboard(qh, &seat, None) {
                self.launcher_input.add(seat.clone(), keyboard);
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(pointer) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(pointer);
            }
        }
        if capability == Capability::Touch && self.touch.is_none() {
            if let Ok(touch) = self.seat_state.get_touch(qh, &seat) {
                self.touch = Some(touch);
                if self.gap_profile() {
                    self.sync_gap_strips();
                }
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            self.launcher_input.remove(&seat);
        }
        if capability == Capability::Keyboard
            && self.active_scene == SceneKind::Osk
            && self.osk.seat() == Some(&seat)
        {
            self.osk.ensure_virtual_keyboard(qh);
        }
        if capability == Capability::Pointer
            && self
                .pointer
                .as_ref()
                .and_then(|pointer| pointer.data::<PointerData<()>>())
                .is_some_and(|data| data.seat() == &seat)
        {
            if self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.touch_id.is_none())
            {
                self.abort_drag();
            }
            if let Some(pointer) = self.pointer.take().filter(|p| p.version() >= 3) {
                pointer.release();
            }
        }
        if capability == Capability::Touch
            && self
                .touch
                .as_ref()
                .and_then(|touch| touch.data::<TouchData<()>>())
                .is_some_and(|data| data.seat() == &seat)
        {
            if let Some(touch) = self.touch.take().filter(|t| t.version() >= 3) {
                touch.release();
            }
            if self.gap_profile() {
                self.invalidate_gap_configuration();
            }
            if self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.touch_id.is_some())
            {
                self.abort_drag();
            }
            if self.gap_profile() {
                // SCTK invokes remove_seat before it removes that seat from
                // the iterator. Only other advertised touch seats qualify.
                self.touch = self
                    .seat_state
                    .seats()
                    .filter(|other| other != &seat)
                    .find_map(|other| self.seat_state.get_touch(qh, &other).ok());
                self.sync_gap_strips();
            }
        }
    }

    fn remove_seat(&mut self, conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.launcher_input.remove(&seat);
        self.remove_capability(conn, qh, seat.clone(), Capability::Touch);
        self.remove_capability(conn, qh, seat.clone(), Capability::Pointer);
        if self.osk.detach_seat(&seat) {
            self.renderer.context_mut().osk_state = OskState::default();
            self.on_osk_editor_lost();
            // SCTK removes the seat from its list after this callback.
            if self.visible {
                if let Some(next) = self.seat_state.seats().find(|next| next != &seat) {
                    self.osk.attach_seat(&next, qh);
                    self.draw_pill();
                }
            }
        }
    }
}

impl PointerHandler for LayerApp {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        if self.pointer.as_ref() != Some(pointer) {
            return;
        }
        let seat = pointer.data::<PointerData<()>>().map(PointerData::seat);
        for event in events {
            let Some(kind) = self.surface_kind(&event.surface) else {
                continue;
            };
            if self.gap_edge(kind).is_some()
                || self
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.touch_id.is_some())
            {
                continue;
            }
            let exact_surface = self.gap_profile() || self.tablet_navigation_drag();
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Press {
                    button: 0x110,
                    time,
                    ..
                } => {
                    if self.drag.is_some() && exact_surface {
                        // A second primary press means the previous release
                        // was lost. Cancel it, never commit on this press.
                        self.abort_drag();
                    } else if self.drag.is_none() {
                        self.begin_drag(seat, None, kind, x, y, time);
                    }
                }
                PointerEventKind::Motion { time } => {
                    if self
                        .drag
                        .as_ref()
                        .is_none_or(|drag| !exact_surface || drag.surface == kind)
                    {
                        self.update_drag(None, x, y, time);
                    }
                }
                PointerEventKind::Release {
                    button: 0x110,
                    time,
                    ..
                } => {
                    if self
                        .drag
                        .as_ref()
                        .is_some_and(|drag| exact_surface && drag.surface != kind)
                    {
                        self.abort_drag();
                    } else {
                        if self.gap_profile() {
                            // Use the release position too (without any swipe
                            // arbitration) so it cannot click across controls.
                            if let Some(drag) = &mut self.drag {
                                drag.track_position(x, y);
                            }
                        }
                        self.finish_drag(None, time);
                    }
                }
                PointerEventKind::Leave { .. }
                    if self
                        .drag
                        .as_ref()
                        .is_some_and(|drag| drag.touch_id.is_none() && drag.surface == kind) =>
                {
                    // Old edge handles keep their drag across a focus handoff.
                    // The Tablet preview keeps the origin surface/input intact.
                    if !self.drag.as_ref().is_some_and(|drag| {
                        drag.tablet_origin.is_none()
                            && drag.motion.is_some()
                            && kind != SurfaceKind::Overlay
                    }) {
                        self.abort_drag();
                    }
                }
                _ => {}
            }
        }
    }
}

impl TouchHandler for LayerApp {
    fn down(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        touch: &wl_touch::WlTouch,
        _serial: u32,
        time: u32,
        surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        if self.touch.as_ref() != Some(touch) {
            return;
        }
        if self
            .drag
            .as_ref()
            .is_some_and(|drag| drag.touch_id == Some(id) && drag.uses_input_time())
        {
            // A reused ID cannot complete the previous contact. Drain this
            // one too rather than interpreting it as a launch/tap.
            self.abort_drag();
        } else if self.drag.is_none() {
            let seat = touch.data::<TouchData<()>>().map(TouchData::seat);
            if let Some(kind) = self.surface_kind(&surface) {
                self.begin_drag(
                    seat,
                    Some(id),
                    kind,
                    position.0 as f32,
                    position.1 as f32,
                    time,
                );
            }
        }
    }

    fn up(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        touch: &wl_touch::WlTouch,
        _serial: u32,
        time: u32,
        id: i32,
    ) {
        if self.touch.as_ref() == Some(touch) {
            self.finish_drag(Some(id), time);
        }
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        touch: &wl_touch::WlTouch,
        time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        if self.touch.as_ref() == Some(touch) {
            self.update_drag(Some(id), position.0 as f32, position.1 as f32, time);
        }
    }

    fn shape(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }

    fn cancel(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, touch: &wl_touch::WlTouch) {
        if self.touch.as_ref() == Some(touch)
            && self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.touch_id.is_some())
        {
            self.abort_drag();
        }
    }
}

impl ShmHandler for LayerApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for LayerApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(LayerApp);
delegate_dispatch2!(LayerApp);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_targets_are_deeper_but_leave_the_sides_to_apps() {
        assert_eq!(
            EdgeHandle::Bottom.input_rect((1280, 48)),
            Some(RegionRect(526, 0, 228, 48))
        );
        assert_eq!(
            EdgeHandle::Bottom.input_rect((853, 48)),
            Some(RegionRect(312, 0, 229, 48))
        );
        assert_eq!(
            EdgeHandle::Bottom.input_rect((160, 48)),
            Some(RegionRect(6, 0, 148, 48))
        );
        assert_eq!(
            EdgeHandle::Top.input_rect((220, 48)),
            Some(RegionRect(34, 0, 152, 48))
        );
        assert_eq!(EdgeHandle::Top.input_rect((220, 0)), None);
    }

    #[test]
    fn tablet_bottom_uses_the_same_center_zone_on_apps_and_overlays() {
        for (w, h) in [(1280, 800), (801, 1280)] {
            let spec = LayerSurfaceSpec::for_role(LayerRole::TabletBottom, w, h);
            assert_eq!((spec.height, spec.exclusive_zone), (52, 0));
            assert!(!spec.keyboard_interactivity);
            let [Some(app), Some(keyboard), None] =
                EdgeHandle::TabletBottom.input_rects((w, spec.height), true)
            else {
                panic!("expected disjoint bottom and keyboard zones");
            };
            let Some(overlay) = EdgeHandle::TabletBottom.input_rect((w, h)) else {
                panic!("expected bottom zone in fullscreen shell scene");
            };
            let x = i32::try_from((w - 200) / 2).unwrap();
            assert_eq!(app, RegionRect(x, 0, 200, 52));
            assert_eq!(
                overlay,
                RegionRect(x, i32::try_from(h).unwrap() - 52, 200, 52)
            );
            assert!(keyboard.0 > x + 200);
            assert!(overlay.contains(x as f32, (h - 52) as f32));
            assert!(!overlay.contains(x as f32 - 0.5, (h - 1) as f32));
            assert!(!overlay.contains(x as f32, (h - 53) as f32));
            assert!(!EdgeHandle::TabletBottom.allows_tap(w, 52, x as f32 + 100.0, 40.0));
        }
    }

    #[test]
    fn gap_strips_are_below_windows_and_accept_only_fingers_inside_local_bounds() {
        for (edge, role) in [
            (GapEdge::Top, LayerRole::OverlayTopGap),
            (GapEdge::Bottom, LayerRole::OverlayBottomGap),
        ] {
            assert_eq!(surface_layer(role), Layer::Bottom);
            let handle = EdgeHandle::Gap(edge);
            assert_eq!(
                handle.input_rects((800, 9), true),
                [Some(RegionRect(0, 0, 800, 9)), None, None],
            );
            assert_eq!(handle.input_rects((800, 0), false), [None; 3]);
            assert!(!handle.allows_tap(800, 9, 400.0, 4.0));
        }

        let bounds = EdgeHandle::Gap(GapEdge::Bottom).input_rect((800, 9));
        let mut finger =
            bottom_contact_at(NavOrigin::App, 1280, bounds, (799.0, 4.0), true, Some(7))
                .expect("touch down uses the full width in the strip's local coordinates");
        finger.update(0.0, -60.0);
        assert_eq!(finger.release((0.0, 0.0)), NavRelease::Cancel);
        finger.update(0.0, -78.0);
        assert_eq!(finger.release((0.0, 0.0)), NavRelease::Overview);
        assert_eq!(finger.release((0.0, -1200.0)), NavRelease::Home);
        assert!(
            bottom_contact_at(NavOrigin::App, 1280, bounds, (799.0, 4.0), true, None).is_none()
        );
        assert!(
            bottom_contact_at(NavOrigin::App, 1280, bounds, (799.0, 9.0), true, Some(7)).is_none()
        );
        assert!(
            bottom_contact_at(NavOrigin::App, 1280, bounds, (799.0, 1279.0), true, Some(7))
                .is_none()
        );
    }

    #[test]
    fn gap_panel_bottom_band_uses_panel_coordinates_not_native_depth() {
        let band = full_width_band((800, 1280), TABLET_BOTTOM_DEPTH);
        assert_eq!(band, Some(RegionRect(0, 1228, 800, 52)));
        for origin in [NavOrigin::Home, NavOrigin::Recents] {
            assert!(bottom_contact_at(origin, 1280, band, (0.0, 1228.0), true, Some(0)).is_some());
            assert!(bottom_contact_at(origin, 1280, band, (0.0, 1228.0), true, None).is_none());
            assert!(bottom_contact_at(origin, 1280, band, (0.0, 1227.0), true, Some(0)).is_none());
        }
    }

    #[test]
    fn tablet_content_taps_cannot_escape_to_another_target_or_follow_a_drag() {
        let mut layout = SceneOutput::default();
        let first = HitTarget::LaunchApp { index: 3 };
        layout.push_hit(HitRect::new(0.0, 48.0, 100.0, 100.0), first.clone());
        layout.push_hit(
            HitRect::new(100.0, 48.0, 100.0, 100.0),
            HitTarget::LaunchApp { index: 1 },
        );
        let tap =
            |down, x, travelled| tablet_content_tap(Some(&layout), down, (x, 55.0), travelled);
        assert!(tap(Some(&first), 99.0, 6.0));
        assert!(!tap(Some(&first), 101.0, 2.0));
        assert!(!tap(None, 99.0, 5.0)); // down on a passive clock/blank area
        assert!(!tap(Some(&first), 99.0, 130.0)); // dragged and returned
        assert!(!tablet_content_tap(None, Some(&first), (99.0, 55.0), 0.0));
    }

    #[test]
    fn drawer_availability_requires_visible_touch_navigation_profile_not_armed_strips() {
        let mut ctx = SceneContext::default();
        assert!(!tablet_navigation_available(true, &ctx)); // in-desktop overlay
        ctx.power_menu = Some(PowerMenu::Closed);
        assert!(tablet_navigation_available(true, &ctx));
        assert!(!tablet_navigation_available(false, &ctx)); // hidden after request was queued
        ctx.tablet_mode = false;
        assert!(!tablet_navigation_available(true, &ctx)); // standalone Desk
        ctx.power_menu = None;
        ctx.overlay_gap_gestures = true;
        assert!(tablet_navigation_available(true, &ctx));
        assert!(!tablet_navigation_available(false, &ctx));
    }

    #[test]
    fn tablet_home_content_swipes_keep_downward_dismissal_separate_from_drawer() {
        let height = 800.0;
        assert_eq!(tablet_home_vertical_motion(20.0, 16.0, height), None);

        let (drawer, scene) = tablet_home_vertical_motion(0.0, -20.0, height).unwrap();
        assert_eq!(scene, Some(SceneKind::Drawer));
        assert!(drawer.is_drawer());
        assert!((drawer.position(0.0, -400.0, 1) - 0.5).abs() < f32::EPSILON);

        let (dismiss, scene) = tablet_home_vertical_motion(0.0, 20.0, height).unwrap();
        assert_eq!(scene, None);
        assert!(!dismiss.is_drawer());
        let pulled = dismiss.position(0.0, home_reveal_travel(height) * 0.75, 1);
        assert!((pulled - 0.25).abs() < f32::EPSILON);
        assert_eq!(dismiss.release(pulled, (0.0, 0.0), 1).target, 0.0);
        assert_eq!(dismiss.cancel(pulled, 1).target, 1.0);
        assert_eq!(dismiss.position(0.0, -400.0, 1), 1.0);
    }

    #[test]
    fn tablet_app_switch_passes_only_the_bound_output_and_needs_focus_to_leave_home() {
        #[derive(Default)]
        struct Handler(Vec<(String, i32, bool)>);
        impl ShellActionHandler for Handler {
            fn on_action(&mut self, action: ShellAction, _: &mut SceneContext) {
                panic!("must not emit a generic shell action: {action:?}");
            }

            fn switch_tablet_app(
                &mut self,
                output: &str,
                direction: i32,
                from_home: bool,
                _: &SceneContext,
            ) -> bool {
                self.0.push((output.to_owned(), direction, from_home));
                output == "tablet-panel" && direction == 1
            }
        }

        let mut handler = Handler::default();
        let ctx = SceneContext::default();
        assert!(!switch_tablet_app(&mut handler, None, 1, true, &ctx));
        assert_eq!(handler.0, []);
        assert!(!switch_tablet_app(
            &mut handler,
            Some("tablet-panel"),
            -1,
            true,
            &ctx
        ));
        assert!(!switch_tablet_app(
            &mut handler,
            Some("tablet-panel"),
            1,
            false,
            &ctx
        ));
        assert!(switch_tablet_app(
            &mut handler,
            Some("tablet-panel"),
            1,
            true,
            &ctx
        ));
        assert_eq!(
            handler.0,
            [
                ("tablet-panel".to_owned(), -1, true),
                ("tablet-panel".to_owned(), 1, false),
                ("tablet-panel".to_owned(), 1, true),
            ]
        );
    }

    #[test]
    fn tablet_status_captures_the_reserved_row_but_never_taps() {
        for (w, h) in [(960, 640), (640, 960)] {
            let spec = LayerSurfaceSpec::for_role(LayerRole::TabletStatus, w, h);
            assert_eq!((spec.height, spec.exclusive_zone), (48, 48));
            assert!(!spec.keyboard_interactivity);
            let rect = RegionRect(0, 0, i32::try_from(w).unwrap(), 48);
            assert_eq!(
                EdgeHandle::TabletStatus.input_rects((w, spec.height), true),
                [Some(rect), None, None]
            );
            // Home uses the same row geometry in its fullscreen surface.
            assert_eq!(EdgeHandle::TabletStatus.input_rect((w, h)), Some(rect));
            for x in [0.0, w as f32 * 0.5, w as f32 - 1.0] {
                assert!(rect.contains(x, 0.0));
                assert!(rect.contains(x, 47.99));
                assert!(!rect.contains(x, 48.0));
                assert!(!EdgeHandle::TabletStatus.allows_tap(w, spec.height, x, 12.0));
            }
            assert!(!rect.contains(w as f32, 12.0));
        }
    }

    #[test]
    fn tablet_status_pull_previews_and_can_reverse_or_cancel_before_opening() {
        for (dx, dy) in [(0.0, 0.0), (4.0, 2.0), (70.0, 25.0), (0.0, -24.0)] {
            assert!(tablet_status_reveal(dx, dy, 640.0).is_none());
        }
        let pull = tablet_status_reveal(0.0, 12.0, 640.0).unwrap();
        let preview = pull.position(5.0, 80.0, 1);
        assert_eq!(preview, 0.125);
        assert_eq!(pull.release(preview, (0.0, 519.0), 1).target, 0.0);
        assert_eq!(pull.release(preview, (0.0, 520.0), 1).target, 1.0);
        assert_eq!(pull.cancel(preview, 1).target, 0.0);

        let threshold = 126.0 / 488.0;
        assert_eq!(pull.release(threshold, (0.0, 0.0), 1).target, 1.0);
        assert_eq!(pull.release(threshold - 0.001, (0.0, 0.0), 1).target, 0.0);
        assert_eq!(pull.release(0.5, (0.0, -520.0), 1).target, 0.0);
        let reversed = pull.position(5.0, 24.0, 1);
        assert!(reversed < preview);
        assert_eq!(pull.release(reversed, (0.0, -500.0), 1).target, 0.0);
        assert_eq!(pull.position(5.0, -6.0, 1), 0.0);
    }

    #[test]
    fn tablet_shade_selects_the_contact_half_only_in_portrait() {
        assert_eq!(
            tablet_shade_page_from_down(399.0, 800.0, 1280.0),
            TabletShadePage::Notifications
        );
        assert_eq!(
            tablet_shade_page_from_down(400.0, 800.0, 1280.0),
            TabletShadePage::Controls
        );
        assert_eq!(
            tablet_shade_page_from_down(0.0, 1280.0, 800.0),
            TabletShadePage::Controls
        );
    }

    #[test]
    fn keyboard_button_only_adds_a_disjoint_region_to_the_resting_bottom_pill() {
        for w in [640, 960] {
            let [pill, keyboard, _] = EdgeHandle::Bottom.input_rects((w, 48), true);
            let keyboard = keyboard.unwrap();
            let RegionRect(x, y, width, _) = keyboard;
            let RegionRect(left, _, pill_width, _) = pill.unwrap();
            assert!(x > left + pill_width);
            assert_eq!(x + width, i32::try_from(w).unwrap() - 12);
            assert!(keyboard.contains((x + 22) as f32, (y + 22) as f32));
            assert!(!keyboard.contains((w - 4) as f32, (y + 22) as f32));
            assert_eq!(
                EdgeHandle::Bottom.input_rects((w, 48), false),
                [pill, None, None]
            );
            assert!(EdgeHandle::Top.input_rects((w, 48), true)[1].is_none());
        }
    }

    #[test]
    fn tap_requires_the_mark_and_never_follows_a_reversed_drag() {
        assert!(EdgeHandle::Bottom.allows_tap(960, 48, 480.0, 36.0));
        assert!(!EdgeHandle::Bottom.allows_tap(960, 48, 480.0, 10.0));
        assert!(EdgeHandle::Top.allows_tap(220, 48, 110.0, 12.0));
        assert!(!EdgeHandle::Top.allows_tap(220, 48, 110.0, 36.0));
        assert!(handle_tap(true, 3.0));
        assert!(!handle_tap(true, 10.0));
        assert!(!handle_tap(false, 0.0));
    }

    #[test]
    fn shm_geometry_uses_physical_width_for_stride_and_both_axes_for_pool() {
        assert_eq!(shm_buffer_geometry(452, 41), Some((452, 41, 1808, 148_352)));
        assert!(shm_buffer_geometry(0, 41).is_none());
        assert!(shm_buffer_geometry(1_000_000_000, 27).is_none());
        assert!(shm_buffer_geometry(500_000, 500_000).is_none());
    }
}
