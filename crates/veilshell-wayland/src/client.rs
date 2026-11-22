//! Multi-surface event-driven `wlr-layer-shell` `Wayland` runtime powered by `calloop`.

use std::fmt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{
    CompositorHandler, CompositorState, FrameCallbackData, Region,
};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
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
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::touch::TouchHandler;
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor as WlrAnchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};

use veilshell_core::{
    classify_bottom_pill, GestureAction, GestureSample, GestureThresholds, Orientation, OskEmit,
    OskState, Spring, SpringParams, ThemePalette,
};
use veilshell_render::power::{PowerAction, PowerInput, PowerMenu};
use veilshell_render::{
    bottom_pill_bounds, build_scene, build_scene_layout, build_top_strip_scene, home_page_count,
    home_reveal_travel, recents_stride, shade_reveal_travel, top_handle_bounds, ActiveRenderer,
    DrawList, HitRect, HitTarget, SceneContext, SceneKind, SceneOutput, WallpaperImage,
};

use crate::capture::ToplevelCaptureState;
#[cfg(feature = "wgpu")]
use crate::gpu::{gpu_attempt, init_renderer, WgpuWaylandSurface};
use crate::motion::{
    edge_vertical, horizontal, slider_pct_for_y, vertical, DragMotion, DragVelocity, SliderDrag,
};
use crate::osk::{OskRepeat, OskWaylandState};
use crate::scale::{FractionalScaleGlobals, SurfaceFrame, SurfaceScaling};
use crate::{LayerRole, LayerSurfaceSpec, WaylandRuntimeConfig};

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
    /// Upload and pre-blur a new static wallpaper on the GPU (or `None` to clear).
    SetWallpaper(Option<WallpaperImage>),
    /// Deliver the result of a session action without blocking Wayland dispatch.
    PowerFinished { action: PowerAction, success: bool },
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
}

/// Interactive UI events emitted by the `Wayland` shell surfaces.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellAction {
    /// Sent immediately before opening a scene so state (e.g. Hyprland windows) can refresh.
    RequestRefresh(SceneKind),
    /// Focus and raise a compositor window.
    FocusWindow(String),
    /// Request closing a compositor window.
    CloseWindow(String),
    /// Launch a desktop application command.
    LaunchApp { exec: String },
    /// Switch to the adjacent workspace (`+1` next, `-1` prev).
    SwitchWorkspace(i32),
    /// Toggle a Quick Settings tile (`wifi`, `bluetooth`, `lte`, `autorotate`, `tablet_mode`, `night_light`).
    ToggleSetting(&'static str),
    /// Update a Quick Settings slider (`brightness`, `volume`) to `0..=100`.
    SetSlider { id: &'static str, pct: u8 },
    /// Notify that the active Home grid page changed.
    HomePageChanged(usize),
    /// Request an operation from the standalone session adapter.
    RequestPower(PowerAction),
}

/// Callback interface handling UI interactions in `veilshell-shell`.
pub trait ShellActionHandler {
    /// Respond to a [`ShellAction`], optionally mutating the live [`SceneContext`].
    fn on_action(&mut self, action: ShellAction, ctx: &mut SceneContext);

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
    Pill,
    TopStrip,
    Overlay,
}

struct ActiveDrag {
    touch_id: Option<i32>,
    surface: SurfaceKind,
    start_x: f32,
    start_y: f32,
    last_x: f32,
    last_y: f32,
    max_travel: f32,
    handle_tap_allowed: bool,
    overlay_tap_allowed: bool,
    recent_card: Option<String>,
    active_slider: Option<SliderDrag>,
    osk_repeat: Option<OskRepeat>,
    motion: Option<DragMotion>,
    velocity: DragVelocity,
    start_time: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegionRect(i32, i32, i32, i32);

#[derive(Clone, Copy)]
enum EdgeHandle {
    Bottom,
    Top,
}

impl EdgeHandle {
    fn bounds(self, width: f32, height: f32) -> HitRect {
        match self {
            Self::Bottom => bottom_pill_bounds(width, height),
            Self::Top => top_handle_bounds(width, height),
        }
    }

    fn input_rect(self, (width, height): (u32, u32)) -> Option<RegionRect> {
        // Stable surface-local target; the animated handle does not move the input zone.
        if width == 0 || height == 0 {
            return None;
        }
        let w = width as f32;
        let bounds = self.bounds(w, height as f32);
        let x0 = (bounds.x - 24.0).max(w * 0.04).floor() as i32;
        let x1 = (bounds.x + bounds.w + 24.0).min(w * 0.96).ceil() as i32;
        Some(RegionRect(x0, 0, x1 - x0, i32::try_from(height).ok()?))
    }

    fn allows_tap(self, width: u32, height: u32, x: f32, y: f32) -> bool {
        let b = self.bounds(width as f32, height as f32);
        // Allow a little finger jitter at the mark, but not in the swipe padding.
        HitRect::new(b.x - 4.0, b.y - 4.0, b.w + 8.0, b.h + 8.0).contains(x, y)
    }
}

fn handle_tap(allowed: bool, max_travel: f32) -> bool {
    allowed && max_travel < 8.0
}

struct HandleInputRegion {
    handle: EdgeHandle,
    region: Region,
    rect: Option<RegionRect>,
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
            rect: None,
        }
    }

    fn update(&mut self, surface: &wl_surface::WlSurface, size: (u32, u32)) {
        let rect = self.handle.input_rect(size);
        if rect == self.rect {
            return;
        }
        if let Some(RegionRect(x, y, w, h)) = self.rect {
            self.region.subtract(x, y, w, h);
        }
        if let Some(RegionRect(x, y, w, h)) = rect {
            self.region.add(x, y, w, h);
        }
        // This gates new contacts only; an implicit grab can leave the region.
        surface.set_input_region(Some(self.region.wl_region()));
        self.rect = rect;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelInput {
    Normal,
    Preview,
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

impl ManagedSurface {
    fn create(app: &LayerApp, role: LayerRole) -> Self {
        let spec = LayerSurfaceSpec::for_role(role, app.viewport_w, app.viewport_h);
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
            Layer::Overlay,
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
        let edge_preview = matches!(role, LayerRole::HomeGrid | LayerRole::QuickSettings)
            && app
                .drag
                .as_ref()
                .is_some_and(|drag| drag.surface != SurfaceKind::Overlay && drag.motion.is_some());
        layer.set_keyboard_interactivity(if spec.keyboard_interactivity && !edge_preview {
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
            app.fractional_scale.as_ref(),
            &app.qh,
            layer.wl_surface(),
            initial_factor,
        );
        let handle_input = match role {
            LayerRole::BottomPill => Some(EdgeHandle::Bottom),
            LayerRole::TopStrip => Some(EdgeHandle::Top),
            _ => None,
        }
        .map(|handle| HandleInputRegion::new(handle, &app.compositor, layer.wl_surface()));
        if edge_preview {
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
                        input.update(surface, size);
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
            input.update(surface, (self.width, self.height));
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
    pub(crate) visible: bool,
    pub(crate) active_scene: SceneKind,
    pill: Option<ManagedSurface>,
    top_strip: Option<ManagedSurface>,
    overlay: Option<ManagedSurface>,
    overlay_layout: Option<SceneOutput>,
    overlay_spring: Spring,
    home_spring: Option<Spring>,
    refresh_after_open: bool,
    last_anim_tick: Instant,
    palette: ThemePalette,
    pub(crate) renderer: ActiveRenderer,
    #[cfg(feature = "wgpu")]
    renderer_preference: veilshell_render::RendererPreference,
    render_failure: Option<String>,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
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
    fn sync_surfaces(&mut self) {
        self.sync_osk_visibility();
        if !self.visible {
            self.overlay = None;
            self.overlay_layout = None;
            self.top_strip = None;
            self.pill = None;
            return;
        }

        if self.active_scene == SceneKind::Osk {
            self.pill = None;
        } else if self.pill.is_none() {
            self.pill = Some(ManagedSurface::create(self, LayerRole::BottomPill));
        }

        if self.active_scene == SceneKind::Pill {
            self.overlay = None;
            self.overlay_layout = None;
            if self.top_strip.is_none() {
                self.top_strip = Some(ManagedSurface::create(self, LayerRole::TopStrip));
            }
        } else {
            // Destroying the origin of the pull would cancel the Wayland grab.
            if !self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.surface == SurfaceKind::TopStrip)
            {
                self.top_strip = None;
            }
            if self.overlay.is_none() {
                let role = LayerRole::from_scene(self.active_scene);
                self.overlay = Some(ManagedSurface::create(self, role));
            }
        }
    }

    pub(crate) fn show_scene(&mut self, scene: SceneKind, animate: bool) {
        self.drag = None;
        self.reset_handle_motion();
        if animate && scene == SceneKind::Pill && self.has_gesture_overlay() {
            self.overlay_spring.set_target(0.0);
            self.overlay_spring.velocity = 0.0;
            self.last_anim_tick = Instant::now();
            self.draw_pill();
            self.update_overlay_after_motion();
            return;
        }
        let progress = if scene == SceneKind::Pill { 0.0 } else { 1.0 };
        let mut spring = Spring::new(progress, SpringParams::snappy());
        if animate && scene != SceneKind::Pill && scene != self.active_scene {
            spring.value = 0.0;
        }
        self.show_scene_at(scene, spring, true);
    }

    fn show_scene_at(&mut self, scene: SceneKind, spring: Spring, refresh: bool) {
        if scene != self.active_scene {
            self.cancel_osk_drag();
            if self.active_scene == SceneKind::Osk {
                self.osk.auto_opened = false;
            }
            if self.active_scene == SceneKind::Recents {
                self.capture.invalidate();
            }
        }
        if scene != SceneKind::Shade {
            if let Some(menu) = &mut self.renderer.context_mut().power_menu {
                if !matches!(*menu, PowerMenu::Running(_)) {
                    *menu = PowerMenu::Closed;
                }
            }
        }
        if scene != SceneKind::Pill {
            self.visible = true;
            if refresh {
                self.handler.on_action(
                    ShellAction::RequestRefresh(scene),
                    self.renderer.context_mut(),
                );
            }
            if scene == SceneKind::Recents {
                self.renderer.context_mut().recents_index = 0;
                self.capture.invalidate();
            }
        }

        let prev_scene = self.active_scene;
        let changed = prev_scene != scene;
        self.active_scene = scene;
        self.renderer.context_mut().pill_drag_x = 0.0;
        self.renderer.context_mut().pill_drag_y = 0.0;
        self.renderer.context_mut().top_drag_y = 0.0;

        self.overlay_spring = spring;
        self.home_spring = None;
        self.renderer.context_mut().recents_throw = None;
        self.last_anim_tick = Instant::now();
        self.refresh_after_open = !refresh;

        // Only recreate the overlay LayerSurface when geometry role changes (TouchOsk vs full-screen).
        if changed && scene != SceneKind::Pill {
            self.overlay_layout = None;
            if (prev_scene == SceneKind::Osk) != (scene == SceneKind::Osk) {
                self.overlay = None;
            }
        }

        self.sync_surfaces();
        self.draw_pill();
        self.draw_top_strip();
        self.draw_overlay();
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            if self.active_scene == SceneKind::Recents {
                self.capture.invalidate();
            }
            self.drag = None;
            self.reset_handle_motion();
            self.home_spring = None;
            self.renderer.context_mut().recents_throw = None;
            self.refresh_after_open = false;
            self.active_scene = SceneKind::Pill;
        }
        self.sync_surfaces();
        if visible {
            self.draw_pill();
            self.draw_top_strip();
        }
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

    pub(crate) fn on_osk_input_lost(&mut self, auto_opened: bool) {
        self.cancel_osk_drag();
        if auto_opened && self.visible && self.active_scene == SceneKind::Osk {
            self.show_scene(SceneKind::Pill, false);
        }
    }

    fn draw_pill(&mut self) {
        let Some(surf) = &mut self.pill else {
            return;
        };
        if !surf.configured || surf.width == 0 || surf.height == 0 {
            return;
        }
        self.renderer.context_mut().pill_pressed = self
            .drag
            .as_ref()
            .is_some_and(|drag| drag.surface == SurfaceKind::Pill);
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
        let Some(surf) = &mut self.top_strip else {
            return;
        };
        if !surf.configured || surf.width == 0 || surf.height == 0 {
            return;
        }
        self.renderer.context_mut().top_pressed = self
            .drag
            .as_ref()
            .is_some_and(|drag| drag.surface == SurfaceKind::TopStrip);
        let list = build_top_strip_scene(
            surf.width as f32,
            surf.height as f32,
            &self.palette,
            self.renderer.context().top_drag_y,
            self.renderer.context().top_pressed,
        );
        self.present_surface(SurfaceKind::TopStrip, &list, false);
    }

    pub(crate) fn draw_overlay(&mut self) {
        let input_ready = self.overlay_input_ready()
            && self
                .drag
                .as_ref()
                .is_none_or(|drag| drag.surface == SurfaceKind::Overlay);
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
        if input_ready && surf.panel_input == PanelInput::Preview {
            surf.layer.wl_surface().set_input_region(None);
            surf.layer
                .set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
            surf.panel_input = PanelInput::Normal;
        }
        let (width, height) = (surf.width, surf.height);
        if self.active_scene == SceneKind::Recents {
            if let Some(frame) = surf.scaling.frame((width, height)) {
                if self.capture.needs_capture(frame.target) {
                    let ctx = self.renderer.context_mut();
                    ctx.recents_index = ctx
                        .recents_index
                        .min(ctx.recents_cards.len().saturating_sub(1));
                    self.capture.apply_cached_and_capture(
                        &mut ctx.recents_cards,
                        ctx.recents_index,
                        frame.target,
                        &self.qh,
                    );
                }
            }
        }
        if self.active_scene == SceneKind::Home {
            let ctx = self.renderer.context_mut();
            let total = home_page_count(&ctx.home_apps, width as f32, height as f32);
            ctx.home_page = ctx.home_page.min(total - 1);
        }
        let ctx = self.renderer.context_mut();
        ctx.overlay_progress = self.overlay_spring.value.clamp(0.0, 1.0);
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
        let layout = build_scene_layout(
            self.active_scene,
            width as f32,
            height as f32,
            &self.palette,
            self.renderer.context(),
        );
        let animating = !self.overlay_spring.is_settled()
            || self.home_spring.is_some()
            || self.drag.as_ref().is_some_and(|drag| drag.motion.is_some());
        if self.present_surface(SurfaceKind::Overlay, &layout.draw_list, animating) {
            if let Some(surf) = &mut self.overlay {
                surf.frame_pending = animating;
            }
            self.overlay_layout = Some(layout);
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
                for surface in [&mut self.pill, &mut self.top_strip, &mut self.overlay]
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
            None
        }
    }

    fn managed_surface_mut(&mut self, kind: SurfaceKind) -> Option<&mut ManagedSurface> {
        match kind {
            SurfaceKind::Pill => self.pill.as_mut(),
            SurfaceKind::TopStrip => self.top_strip.as_mut(),
            SurfaceKind::Overlay => self.overlay.as_mut(),
        }
    }

    fn draw_surface(&mut self, kind: SurfaceKind) {
        match kind {
            SurfaceKind::Pill => self.draw_pill(),
            SurfaceKind::TopStrip => self.draw_top_strip(),
            SurfaceKind::Overlay => self.draw_overlay(),
        }
    }

    pub(crate) fn fractional_scale_changed(&mut self, surface: &wl_surface::WlSurface, units: u32) {
        if let Some(kind) = self.surface_kind(surface) {
            if self
                .managed_surface_mut(kind)
                .is_some_and(|s| s.scaling.update_fractional(units))
            {
                self.draw_surface(kind);
            }
        }
    }

    #[cfg(feature = "wgpu")]
    fn swapchain_retries(&self) -> impl Iterator<Item = (SurfaceKind, Instant)> + '_ {
        [
            (SurfaceKind::Pill, &self.pill),
            (SurfaceKind::TopStrip, &self.top_strip),
            (SurfaceKind::Overlay, &self.overlay),
        ]
        .into_iter()
        .filter_map(|(kind, surface)| Some((kind, surface.as_ref()?.retry_at?)))
    }

    fn event_timeout(&self) -> Option<Duration> {
        let osk = self.osk_repeat_timeout();
        #[cfg(feature = "wgpu")]
        {
            let now = Instant::now();
            self.swapchain_retries()
                .map(|(_, retry_at)| retry_at.saturating_duration_since(now))
                .chain(osk)
                .min()
        }
        #[cfg(not(feature = "wgpu"))]
        osk
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
            DaemonCommand::ShowScene(scene) => self.show_scene(scene, false),
            DaemonCommand::SetVisible(vis) => self.set_visible(vis),
            DaemonCommand::UpdateContext(ctx) => {
                *self.renderer.context_mut() = *ctx;
                if self.active_scene == SceneKind::Recents {
                    self.capture.invalidate();
                }
                if self.visible {
                    self.draw_pill();
                    self.draw_top_strip();
                    self.draw_overlay();
                }
            }
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
            DaemonCommand::QueryStatus(reply) => {
                let _ = reply.send(RuntimeStatus {
                    active_scene: self.active_scene,
                    visible: self.visible,
                    output_name: self.output_name.clone(),
                    width: self.viewport_w,
                    height: self.viewport_h,
                    renderer: self.renderer.backend_label(),
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

    fn begin_drag(&mut self, touch_id: Option<i32>, kind: SurfaceKind, x: f32, y: f32) {
        let handle = match kind {
            SurfaceKind::Pill => self.pill.as_ref().map(|s| (EdgeHandle::Bottom, s)),
            SurfaceKind::TopStrip => self.top_strip.as_ref().map(|s| (EdgeHandle::Top, s)),
            SurfaceKind::Overlay => None,
        };
        let handle_tap_allowed =
            handle.is_some_and(|(handle, s)| handle.allows_tap(s.width, s.height, x, y));
        let overlay_tap_allowed = kind == SurfaceKind::Overlay && self.overlay_input_ready();
        let recent_card = if overlay_tap_allowed && self.active_scene == SceneKind::Recents {
            self.overlay_layout
                .as_ref()
                .and_then(|layout| match layout.hit_test(x, y) {
                    Some(HitTarget::FocusWindow { id }) => Some(id.clone()),
                    _ => None,
                })
        } else {
            None
        };
        let mut active_slider = None;
        if overlay_tap_allowed && self.active_scene == SceneKind::Shade {
            if let Some(layout) = &self.overlay_layout {
                if let Some(HitTarget::ShadeSlider { id, rect }) = layout.hit_test(x, y) {
                    let ctx = self.renderer.context();
                    let level = if *id == "brightness" {
                        ctx.brightness_pct
                    } else {
                        ctx.volume_pct
                    };
                    active_slider = Some(SliderDrag::new(id, *rect, level));
                }
            }
        }

        let now = Instant::now();
        let osk_repeat = if kind == SurfaceKind::Overlay && self.active_scene == SceneKind::Osk {
            self.overlay_layout.as_ref().and_then(|layout| {
                layout
                    .hit_regions
                    .iter()
                    .rev()
                    .find_map(|hit| match hit.target {
                        HitTarget::OskKey { label }
                            if hit.rect.contains(x, y) && OskState::can_repeat(label) =>
                        {
                            Some(OskRepeat::new(now, label, hit.rect))
                        }
                        _ => None,
                    })
            })
        } else {
            None
        };
        self.drag = Some(ActiveDrag {
            touch_id,
            surface: kind,
            start_x: x,
            start_y: y,
            last_x: x,
            last_y: y,
            max_travel: 0.0,
            handle_tap_allowed,
            overlay_tap_allowed,
            recent_card,
            active_slider,
            osk_repeat,
            motion: None,
            velocity: DragVelocity::new(now, x, y),
            start_time: now,
        });
        if kind != SurfaceKind::Overlay {
            self.draw_surface(kind);
        }
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
        home_page_count(&self.renderer.context().home_apps, w, h)
    }

    fn page_count(&self) -> usize {
        if self.active_scene == SceneKind::Recents {
            self.renderer.context().recents_cards.len().max(1)
        } else {
            self.home_pages()
        }
    }

    fn has_gesture_overlay(&self) -> bool {
        self.active_scene == SceneKind::Home
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
    }

    fn start_interactive_motion(
        &self,
        surface: SurfaceKind,
        start_y: f32,
        dx: f32,
        dy: f32,
    ) -> Option<(DragMotion, Option<SceneKind>)> {
        let (w, h) = self.overlay_size();
        if edge_vertical(dx, dy) {
            match (surface, self.active_scene) {
                (SurfaceKind::Pill, SceneKind::Pill) if dy < 0.0 => {
                    return Some((
                        DragMotion::edge_reveal(home_reveal_travel(h), -1.0),
                        Some(SceneKind::Home),
                    ))
                }
                (SurfaceKind::TopStrip, SceneKind::Pill) if dy > 0.0 => {
                    return Some((
                        DragMotion::edge_reveal(shade_reveal_travel(h), 1.0),
                        Some(SceneKind::Shade),
                    ))
                }
                _ => {}
            }
        }
        // Keep bottom-edge workspace/Recents gestures separate from content drags.
        if surface != SurfaceKind::Overlay || start_y >= h - 52.0 {
            return None;
        }
        let progress = self.overlay_spring.value.clamp(0.0, 1.0);
        let closing = self.overlay_spring.target <= 0.0;
        if self.has_gesture_overlay() && vertical(dx, dy) && self.home_spring.is_none() {
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

    fn update_interactive_drag(&mut self, dx: f32, dy: f32) -> bool {
        let Some(drag) = &self.drag else {
            return false;
        };
        if drag.active_slider.is_some() {
            return false;
        }
        let (surface, start_y, existing) = (drag.surface, drag.start_y, drag.motion);
        let Some((motion, opening)) = existing
            .map(|m| (m, None))
            .or_else(|| self.start_interactive_motion(surface, start_y, dx, dy))
        else {
            return false;
        };
        if let Some(drag) = &mut self.drag {
            drag.motion = Some(motion);
        }
        let position = motion.position(dx, dy, self.page_count());
        let ctx = self.renderer.context_mut();
        ctx.pill_drag_x = 0.0;
        ctx.pill_drag_y = 0.0;
        ctx.top_drag_y = 0.0;
        match motion {
            DragMotion::Reveal { .. } => {
                let spring = Spring::new(position, SpringParams::snappy());
                if let Some(scene) = opening {
                    self.show_scene_at(scene, spring, false);
                    return true;
                }
                self.overlay_spring = spring;
            }
            DragMotion::HomePage { .. } => {
                self.home_spring = Some(Spring::new(position, SpringParams::gentle()));
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
        };
        let pages = self.page_count();
        let spring = velocity.map_or_else(
            || motion.cancel(value, pages),
            |speed| motion.release(value, speed, pages),
        );
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
                    self.handler.on_action(
                        ShellAction::HomePageChanged(page),
                        self.renderer.context_mut(),
                    );
                }
            }
        }
        self.last_anim_tick = Instant::now();
        self.update_overlay_after_motion();
    }

    fn update_overlay_after_motion(&mut self) {
        if self.drag.as_ref().is_none_or(|drag| drag.motion.is_none()) {
            if self.home_spring.is_some_and(|spring| spring.is_settled()) {
                self.home_spring = None;
            }
            if self.overlay_spring.is_settled() {
                if self.overlay_spring.target <= 0.0 && self.active_scene != SceneKind::Pill {
                    self.show_scene(SceneKind::Pill, false);
                    return;
                }
                if std::mem::take(&mut self.refresh_after_open) {
                    self.handler.on_action(
                        ShellAction::RequestRefresh(self.active_scene),
                        self.renderer.context_mut(),
                    );
                }
                if self.active_scene != SceneKind::Pill
                    && !self
                        .drag
                        .as_ref()
                        .is_some_and(|drag| drag.surface == SurfaceKind::TopStrip)
                {
                    self.top_strip = None;
                }
            }
        }
        self.draw_overlay();
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
        let motion = self.drag.take().and_then(|drag| drag.motion);
        self.reset_handle_motion();
        self.renderer.context_mut().recents_throw = None;
        self.draw_pill();
        self.draw_top_strip();
        if let Some(motion) = motion {
            self.settle_interactive_drag(motion, None);
        } else {
            self.draw_overlay();
        }
    }

    fn update_drag(&mut self, touch_id: Option<i32>, x: f32, y: f32) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        if drag.touch_id != touch_id {
            return;
        }
        drag.last_x = x;
        drag.last_y = y;
        drag.velocity.push(Instant::now(), x, y);
        if let Some(repeat) = &mut drag.osk_repeat {
            repeat.position(x, y);
        }
        let (dx, dy) = (x - drag.start_x, y - drag.start_y);
        drag.max_travel = drag.max_travel.max(dx.hypot(dy));
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
        let (surface, start_x, start_y, has_slider) = {
            let Some(drag) = &self.drag else {
                return;
            };
            (
                drag.surface,
                drag.start_x,
                drag.start_y,
                drag.active_slider.is_some(),
            )
        };

        if has_slider {
            let pct = {
                let slider = self.drag.as_mut().and_then(|d| d.active_slider.as_mut());
                slider.and_then(|slider| {
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
                if start_y >= (self.viewport_h as f32 - 52.0) {
                    self.renderer.context_mut().pill_drag_x = x - start_x;
                    self.renderer.context_mut().pill_drag_y = y - start_y;
                    self.draw_overlay();
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn finish_drag(&mut self, touch_id: Option<i32>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.touch_id != touch_id {
            self.drag = Some(drag);
            return;
        }

        self.reset_handle_motion();
        if let Some((id, offset)) = self.renderer.context_mut().recents_throw.take() {
            let (_, vy) = drag.velocity.at_release(Instant::now());
            if offset <= -96.0 || (offset <= -32.0 && vy <= -650.0) {
                self.handler
                    .on_action(ShellAction::CloseWindow(id), self.renderer.context_mut());
                self.capture.invalidate();
            }
            self.draw_overlay();
            return;
        }
        if let Some(motion) = drag.motion {
            if drag.surface != SurfaceKind::Overlay {
                self.draw_surface(drag.surface);
            }
            self.settle_interactive_drag(motion, Some(drag.velocity.at_release(Instant::now())));
            return;
        }

        let elapsed = drag.start_time.elapsed().as_secs_f32().max(0.016);
        let dx = drag.last_x - drag.start_x;
        let dy = drag.last_y - drag.start_y;
        let vx = dx / elapsed;
        let vy = dy / elapsed;
        let dist = dx.hypot(dy);

        match drag.surface {
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
                if handle_tap(drag.handle_tap_allowed, drag.max_travel) {
                    self.show_scene(SceneKind::Shade, true);
                } else {
                    self.draw_top_strip();
                }
            }
            SurfaceKind::Overlay => {
                if let Some(slider) = drag.active_slider {
                    let pct = slider.finish(drag.last_y, dy);
                    self.handler.on_action(
                        ShellAction::SetSlider { id: slider.id, pct },
                        self.renderer.context_mut(),
                    );
                    self.draw_overlay();
                    return;
                }

                if self.active_scene == SceneKind::Osk {
                    if dy >= 60.0 && vy >= 220.0 {
                        self.osk.auto_opened = false;
                        self.show_scene(SceneKind::Pill, false);
                        return;
                    }
                    if drag.overlay_tap_allowed
                        && dist < 24.0
                        && !drag.osk_repeat.is_some_and(|repeat| repeat.fired)
                    {
                        self.handle_overlay_tap(drag.last_x, drag.last_y);
                    } else {
                        self.draw_overlay();
                    }
                    return;
                }

                // Bottom pill swipe inside overlay behaves identically to the bottom pill
                if drag.start_y >= (self.viewport_h as f32 - 52.0) && dist >= 20.0 {
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
                if !self.has_gesture_overlay() && dy <= -72.0 && vy <= -350.0 {
                    self.show_scene(SceneKind::Pill, false);
                    return;
                }

                // Short tap -> hit-test UI targets
                if drag.overlay_tap_allowed && dist < 18.0 {
                    self.handle_overlay_tap(drag.last_x, drag.last_y);
                } else {
                    self.draw_overlay();
                }
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
            .and_then(|layout| layout.hit_test(x, y))
            .cloned();

        let Some(hit) = target else {
            self.draw_overlay();
            return;
        };

        match hit {
            HitTarget::DismissOverlay | HitTarget::BottomPill => {
                self.show_scene(SceneKind::Pill, true);
            }
            HitTarget::LaunchApp { index } => {
                if let Some(app) = self.renderer.context().home_apps.get(index) {
                    let exec = app.exec.clone();
                    self.handler
                        .on_action(ShellAction::LaunchApp { exec }, self.renderer.context_mut());
                    self.show_scene(SceneKind::Pill, false);
                }
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
                if id == "tablet_mode" && !self.renderer.context().tablet_mode {
                    self.set_visible(false);
                } else {
                    self.draw_overlay();
                }
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
            HitTarget::PowerPanel => {}
            HitTarget::OskKey { label } => self.press_osk_key(label),
        }
    }

    fn press_osk_key(&mut self, label: &'static str) {
        let state = &mut self.renderer.context_mut().osk_state;
        let previous = *state;
        let action = state.press(label);
        let changed = *state != previous;
        if action == OskEmit::Dismiss {
            self.osk.auto_opened = false;
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

    fn osk_repeat_timeout(&self) -> Option<Duration> {
        if self.active_scene != SceneKind::Osk {
            return None;
        }
        self.drag
            .as_ref()?
            .osk_repeat
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
            .and_then(|drag| drag.osk_repeat.as_mut())
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
/// Allocates surfaces lazily: only the compact `BottomPill` + `TopStrip` exist while dormant
/// over apps, and full-screen overlays are dropped cleanly upon dismissal.
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
        visible: config.initial_visible,
        active_scene: initial_scene,
        pill: None,
        top_strip: None,
        overlay: None,
        overlay_layout: None,
        overlay_spring: settled_spring,
        home_spring: None,
        refresh_after_open: false,
        last_anim_tick: Instant::now(),
        palette,
        renderer,
        #[cfg(feature = "wgpu")]
        renderer_preference: config.renderer,
        render_failure: None,
        pointer: None,
        touch: None,
        drag: None,
        osk: OskWaylandState::bind_globals(&globals, &qh),
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
        if let Some((w, h)) = info.logical_size {
            if w > 0 && h > 0 {
                app.viewport_w = w as u32;
                app.viewport_h = h as u32;
            }
        }
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
        app.show_scene(initial_scene, false);
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
        // scale changes on older surfaces. Neither changes logical geometry.
        if let Some(kind) = self.surface_kind(surface) {
            if self
                .managed_surface_mut(kind)
                .is_some_and(|s| s.scaling.update_integer(new_factor))
            {
                self.draw_surface(kind);
            }
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
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
        let held = self.drag.as_ref().is_some_and(|drag| drag.motion.is_some());
        let settling = !held && (!self.overlay_spring.is_settled() || self.home_spring.is_some());
        if settling {
            let now = Instant::now();
            let dt = now.duration_since(self.last_anim_tick).as_secs_f32();
            self.last_anim_tick = now;
            self.overlay_spring.step(dt);
            if let Some(spring) = &mut self.home_spring {
                spring.step(dt);
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
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for LayerApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.target_output.as_ref() == Some(&output) {
            if let Some(info) = self.output_state.info(&output) {
                if let Some((w, h)) = info.logical_size {
                    if let (Ok(uw), Ok(uh)) = (u32::try_from(w), u32::try_from(h)) {
                        if uw > 0 && uh > 0 {
                            self.viewport_w = uw;
                            self.viewport_h = uh;
                        }
                    }
                }
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for LayerApp {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        match self.surface_kind(layer.wl_surface()) {
            Some(SurfaceKind::Overlay) => self.show_scene(SceneKind::Pill, false),
            Some(SurfaceKind::Pill) => self.exit = true,
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
        if kind == SurfaceKind::Overlay {
            self.cancel_osk_drag();
        }

        let mut new_w = None;
        let mut new_h = None;
        let mut resized = false;
        if let Some(target) = self.managed_surface_mut(kind) {
            if configure.new_size.0 > 0 {
                resized |= target.configured && target.width != configure.new_size.0;
                target.width = configure.new_size.0;
                new_w = Some(target.width);
            }
            if configure.new_size.1 > 0 {
                resized |= target.configured && target.height != configure.new_size.1;
                target.height = configure.new_size.1;
                new_h = Some(target.height);
            }
            target.configured = true;
        }

        match kind {
            SurfaceKind::Pill => {
                if let Some(w) = new_w {
                    self.viewport_w = w;
                }
                self.draw_pill();
            }
            SurfaceKind::TopStrip => self.draw_top_strip(),
            SurfaceKind::Overlay => {
                if let Some(w) = new_w {
                    self.viewport_w = w;
                }
                if let Some(h) = new_h {
                    if self.active_scene != SceneKind::Osk {
                        self.viewport_h = h;
                    }
                }
                if resized && self.drag.as_ref().is_some_and(|drag| drag.motion.is_some()) {
                    self.abort_drag();
                } else {
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
        }
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(pointer) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(pointer);
            }
        }
        if capability == Capability::Touch && self.touch.is_none() {
            if let Ok(touch) = self.seat_state.get_touch(qh, &seat) {
                self.touch = Some(touch);
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.touch_id.is_none())
            {
                self.abort_drag();
            }
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
        if capability == Capability::Touch {
            if self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.touch_id.is_some())
            {
                self.abort_drag();
            }
            if let Some(touch) = self.touch.take() {
                touch.release();
            }
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        let auto_opened = self.osk.auto_opened;
        if self.osk.detach_seat(&seat) {
            self.on_osk_input_lost(auto_opened);
            // SCTK removes the seat from its list after this callback.
            if self.visible {
                if let Some(next) = self.seat_state.seats().find(|next| next != &seat) {
                    self.osk.attach_seat(&next, qh);
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
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let Some(kind) = self.surface_kind(&event.surface) else {
                continue;
            };
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Press { button: 0x110, .. } if self.drag.is_none() => {
                    self.begin_drag(None, kind, x, y);
                }
                PointerEventKind::Motion { .. } => self.update_drag(None, x, y),
                PointerEventKind::Release { button: 0x110, .. } => self.finish_drag(None),
                PointerEventKind::Leave { .. }
                    if self
                        .drag
                        .as_ref()
                        .is_some_and(|drag| drag.touch_id.is_none() && drag.surface == kind) =>
                {
                    // The compositor may offer focus to our new overlay, but the
                    // grab that started on the handle ends only at release.
                    if !self
                        .drag
                        .as_ref()
                        .is_some_and(|drag| drag.motion.is_some() && kind != SurfaceKind::Overlay)
                    {
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
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        if self.drag.is_none() {
            if let Some(kind) = self.surface_kind(&surface) {
                self.begin_drag(Some(id), kind, position.0 as f32, position.1 as f32);
            }
        }
    }

    fn up(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        id: i32,
    ) {
        self.finish_drag(Some(id));
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        self.update_drag(Some(id), position.0 as f32, position.1 as f32);
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

    fn cancel(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _touch: &wl_touch::WlTouch) {
        self.abort_drag();
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
