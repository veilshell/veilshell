//! Multi-surface event-driven `wlr-layer-shell` `Wayland` runtime powered by `calloop`.

use std::fmt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{self, Channel, Event as ChannelEvent};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch,
};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
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
    build_scene, build_scene_layout, build_top_strip_scene, home_page_count, home_reveal_travel,
    shade_reveal_travel, ActiveRenderer, DrawList, HitTarget, SceneContext, SceneKind, SceneOutput,
    WallpaperImage,
};

use crate::capture::ToplevelCaptureState;
#[cfg(feature = "wgpu")]
use crate::gpu::WgpuWaylandSurface;
use crate::motion::{horizontal, vertical, DragMotion, DragVelocity};
use crate::osk::{OskRepeat, OskWaylandState};
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
    overlay_tap_allowed: bool,
    active_slider: Option<(&'static str, veilshell_render::HitRect)>,
    osk_repeat: Option<OskRepeat>,
    motion: Option<DragMotion>,
    velocity: DragVelocity,
    start_time: Instant,
}

struct ManagedSurface {
    // SAFETY ORDERING: `gpu_surface` MUST be declared before `layer` so `wgpu::Surface`
    // (and its Vulkan/EGL swapchain) is always destroyed before the underlying `wl_surface`.
    #[cfg(feature = "wgpu")]
    gpu_surface: Option<WgpuWaylandSurface>,
    layer: LayerSurface,
    pool: Option<SlotPool>,
    width: u32,
    height: u32,
    configured: bool,
    frame_pending: bool,
    redraw_pending: bool,
}

impl ManagedSurface {
    fn create(app: &LayerApp, role: LayerRole) -> Result<Self, WaylandError> {
        let spec = LayerSurfaceSpec::for_role(role, app.viewport_w, app.viewport_h);
        let wl_surface = app.compositor.create_surface(&app.qh);
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
        layer.set_keyboard_interactivity(if spec.keyboard_interactivity {
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
        layer.commit();

        #[cfg(feature = "wgpu")]
        let gpu_surface = app.renderer.as_wgpu().and_then(|gpu| {
            match WgpuWaylandSurface::create(gpu.instance(), &app.conn, layer.wl_surface()) {
                Ok(surf) => Some(surf),
                Err(err) => {
                    eprintln!(
                        "veilshell: wgpu surface creation failed ({err}); using wl_shm for {role:?}"
                    );
                    None
                }
            }
        });

        #[cfg(feature = "wgpu")]
        let need_shm = gpu_surface.is_none();
        #[cfg(not(feature = "wgpu"))]
        let need_shm = true;

        let pool = if need_shm {
            let initial_bytes = (spec.width.max(1) as usize) * (spec.height.max(1) as usize) * 4;
            Some(
                SlotPool::new((initial_bytes * 2).max(1024 * 64), &app.shm)
                    .map_err(|e| WaylandError::Runtime(format!("shm slot pool: {e}")))?,
            )
        } else {
            None
        };

        Ok(Self {
            #[cfg(feature = "wgpu")]
            gpu_surface,
            layer,
            pool,
            width: spec.width,
            height: spec.height,
            configured: false,
            frame_pending: false,
            redraw_pending: false,
        })
    }

    fn present_draw_list(
        &mut self,
        qh: &QueueHandle<LayerApp>,
        shm: &Shm,
        renderer: &mut ActiveRenderer,
        list: &DrawList,
        request_frame_callback: bool,
    ) -> bool {
        if !self.configured || self.width == 0 || self.height == 0 {
            return false;
        }

        #[cfg(feature = "wgpu")]
        if let (Some(gpu_surf), Some(gpu_renderer)) =
            (self.gpu_surface.as_mut(), renderer.as_wgpu_mut())
        {
            match gpu_surf.present_draw_list(
                gpu_renderer,
                self.width,
                self.height,
                list,
                self.layer.wl_surface(),
                qh,
                request_frame_callback,
            ) {
                Ok(()) => return true,
                Err(err) => {
                    eprintln!(
                        "veilshell: wgpu present failed ({err}); falling back to wl_shm for this surface"
                    );
                    self.gpu_surface = None;
                }
            }
        }

        match renderer.rasterize(self.width, self.height, list) {
            Ok(frame) => self.commit_shm_frame(qh, shm, &frame, request_frame_callback),
            Err(err) => {
                eprintln!("veilshell: rasterize failed: {err}");
                false
            }
        }
    }

    fn commit_shm_frame(
        &mut self,
        qh: &QueueHandle<LayerApp>,
        shm: &Shm,
        frame: &veilshell_render::RenderFrame,
        request_frame_callback: bool,
    ) -> bool {
        if !self.configured || self.width == 0 || self.height == 0 {
            return false;
        }
        if self.pool.is_none() {
            let initial_bytes = (self.width as usize) * (self.height as usize) * 4;
            match SlotPool::new((initial_bytes * 2).max(1024 * 64), shm) {
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

        let argb = frame.to_argb8888_premultiplied();
        let stride = i32::try_from(self.width * 4).unwrap_or(i32::MAX);
        let (buffer, canvas) = match pool.create_buffer(
            i32::try_from(self.width).unwrap_or(1),
            i32::try_from(self.height).unwrap_or(1),
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!(
                    "veilshell: shm create_buffer({}x{}) failed: {err}",
                    self.width, self.height
                );
                return false;
            }
        };

        let copy_len = canvas.len().min(argb.len());
        canvas[..copy_len].copy_from_slice(&argb[..copy_len]);

        let surface = self.layer.wl_surface();
        surface.damage_buffer(
            0,
            0,
            i32::try_from(self.width).unwrap_or(1),
            i32::try_from(self.height).unwrap_or(1),
        );
        if let Err(err) = buffer.attach_to(surface) {
            eprintln!("veilshell: buffer.attach_to failed: {err}");
            return false;
        }
        if request_frame_callback {
            surface.frame(qh, FrameCallbackData(surface.clone()));
        }
        surface.commit();
        true
    }
}

pub(crate) struct LayerApp {
    qh: QueueHandle<Self>,
    registry_state: RegistryState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    seat_state: SeatState,
    output_state: OutputState,
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
            match ManagedSurface::create(self, LayerRole::BottomPill) {
                Ok(surf) => self.pill = Some(surf),
                Err(err) => eprintln!("veilshell: failed to create bottom pill surface: {err}"),
            }
        }

        if self.active_scene == SceneKind::Pill {
            self.overlay = None;
            self.overlay_layout = None;
            if self.top_strip.is_none() {
                if let Ok(surf) = ManagedSurface::create(self, LayerRole::TopStrip) {
                    self.top_strip = Some(surf);
                }
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
                match ManagedSurface::create(self, role) {
                    Ok(surf) => self.overlay = Some(surf),
                    Err(err) => eprintln!("veilshell: failed to create overlay surface: {err}"),
                }
            }
        }
    }

    pub(crate) fn show_scene(&mut self, scene: SceneKind, animate: bool) {
        self.drag = None;
        if animate && scene == SceneKind::Pill && self.has_gesture_overlay() {
            self.overlay_spring.set_target(0.0);
            self.overlay_spring.velocity = 0.0;
            self.last_anim_tick = Instant::now();
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
                self.capture.apply_cached_and_capture(
                    &mut self.renderer.context_mut().recents_cards,
                    &self.shm,
                    &self.qh,
                );
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
        if scene != SceneKind::Pill && self.overlay.is_none() {
            self.show_scene(SceneKind::Pill, false);
            return;
        }
        self.draw_pill();
        self.draw_top_strip();
        self.draw_overlay();
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            self.drag = None;
            self.home_spring = None;
            self.refresh_after_open = false;
            self.active_scene = SceneKind::Pill;
        }
        self.sync_surfaces();
        if visible {
            self.draw_pill();
            self.draw_top_strip();
        }
    }

    fn draw_pill(&mut self) {
        let Some(surf) = &mut self.pill else {
            return;
        };
        if !surf.configured || surf.width == 0 || surf.height == 0 {
            return;
        }
        let list = build_scene(
            SceneKind::Pill,
            surf.width as f32,
            surf.height as f32,
            &self.palette,
            self.renderer.context(),
        );
        surf.present_draw_list(&self.qh, &self.shm, &mut self.renderer, &list, false);
    }

    fn draw_top_strip(&mut self) {
        let Some(surf) = &mut self.top_strip else {
            return;
        };
        if !surf.configured || surf.width == 0 || surf.height == 0 {
            return;
        }
        let list = build_top_strip_scene(
            surf.width as f32,
            surf.height as f32,
            &self.palette,
            self.renderer.context().top_drag_y,
        );
        surf.present_draw_list(&self.qh, &self.shm, &mut self.renderer, &list, false);
    }

    pub(crate) fn draw_overlay(&mut self) {
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
        if self.active_scene == SceneKind::Home {
            let ctx = self.renderer.context_mut();
            let total = home_page_count(&ctx.home_apps, surf.width as f32, surf.height as f32);
            ctx.home_page = ctx.home_page.min(total - 1);
        }
        let ctx = self.renderer.context_mut();
        ctx.overlay_progress = self.overlay_spring.value.clamp(0.0, 1.0);
        ctx.home_position = self.home_spring.map(|spring| spring.value);
        let layout = build_scene_layout(
            self.active_scene,
            surf.width as f32,
            surf.height as f32,
            &self.palette,
            self.renderer.context(),
        );
        let animating = !self.overlay_spring.is_settled()
            || self.home_spring.is_some()
            || self.drag.as_ref().is_some_and(|drag| drag.motion.is_some());
        if surf.present_draw_list(
            &self.qh,
            &self.shm,
            &mut self.renderer,
            &layout.draw_list,
            animating,
        ) {
            surf.frame_pending = animating;
            self.overlay_layout = Some(layout);
        }
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

    fn handle_daemon_command(&mut self, cmd: DaemonCommand) {
        match cmd {
            DaemonCommand::ShowScene(scene) => self.show_scene(scene, false),
            DaemonCommand::SetVisible(vis) => self.set_visible(vis),
            DaemonCommand::UpdateContext(ctx) => {
                *self.renderer.context_mut() = *ctx;
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
        let overlay_tap_allowed = kind == SurfaceKind::Overlay && self.overlay_input_ready();
        let mut active_slider = None;
        if overlay_tap_allowed && self.active_scene == SceneKind::Shade {
            if let Some(layout) = &self.overlay_layout {
                if let Some(HitTarget::ShadeSlider { id, rect }) = layout.hit_test(x, y) {
                    active_slider = Some((*id, *rect));
                    let pct = slider_pct_for_x(x, *rect);
                    self.handler.on_action(
                        ShellAction::SetSlider { id, pct },
                        self.renderer.context_mut(),
                    );
                    self.draw_overlay();
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
            overlay_tap_allowed,
            active_slider,
            osk_repeat,
            motion: None,
            velocity: DragVelocity::new(now, x, y),
            start_time: now,
        });
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
        if vertical(dx, dy) {
            match (surface, self.active_scene) {
                (SurfaceKind::Pill, SceneKind::Pill) if dy < 0.0 => {
                    return Some((
                        DragMotion::reveal(0.0, home_reveal_travel(h), -1.0, true),
                        Some(SceneKind::Home),
                    ))
                }
                (SurfaceKind::TopStrip, SceneKind::Pill) if dy > 0.0 => {
                    return Some((
                        DragMotion::reveal(0.0, shade_reveal_travel(h), 1.0, true),
                        Some(SceneKind::Shade),
                    ))
                }
                _ => {}
            }
        }
        // Keep bottom-edge workspace/Recents gestures separate from content drags.
        if surface != SurfaceKind::Overlay || start_y >= h - 52.0 || !self.has_gesture_overlay() {
            return None;
        }
        let progress = self.overlay_spring.value.clamp(0.0, 1.0);
        let closing = self.overlay_spring.target <= 0.0;
        if vertical(dx, dy) && self.home_spring.is_none() {
            let (travel, sign) = if self.active_scene == SceneKind::Home {
                (home_reveal_travel(h), -1.0)
            } else {
                (shade_reveal_travel(h), 1.0)
            };
            if dy * sign < 0.0 || !self.overlay_spring.is_settled() {
                return Some((DragMotion::reveal(progress, travel, sign, closing), None));
            }
        }
        if self.active_scene == SceneKind::Home
            && self.overlay_spring.is_settled()
            && progress >= 1.0
            && horizontal(dx, dy)
            && self.home_pages() > 1
        {
            let from = self
                .home_spring
                .map_or(self.renderer.context().home_page as f32, |s| s.value);
            return Some((
                DragMotion::HomePage {
                    from,
                    width: w.max(1.0),
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
        let Some((motion, opening)) = drag
            .motion
            .map(|m| (m, None))
            .or_else(|| self.start_interactive_motion(drag.surface, drag.start_y, dx, dy))
        else {
            return false;
        };
        if let Some(drag) = &mut self.drag {
            drag.motion = Some(motion);
        }
        let position = motion.position(dx, dy, self.home_pages());
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
        let pages = self.home_pages();
        let spring = velocity.map_or_else(
            || motion.cancel(value, pages),
            |speed| motion.release(value, speed, pages),
        );
        match motion {
            DragMotion::Reveal { .. } => self.overlay_spring = spring,
            DragMotion::HomePage { .. } => {
                self.home_spring = Some(spring);
                let page = spring.target as usize;
                if self.renderer.context().home_page != page {
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

    fn abort_drag(&mut self) {
        let motion = self.drag.take().and_then(|drag| drag.motion);
        let ctx = self.renderer.context_mut();
        ctx.pill_drag_x = 0.0;
        ctx.pill_drag_y = 0.0;
        ctx.top_drag_y = 0.0;
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
        if self.update_interactive_drag(dx, dy) {
            return;
        }
        let Some(drag) = &self.drag else {
            return;
        };

        match drag.surface {
            SurfaceKind::Pill => {
                self.renderer.context_mut().pill_drag_x = x - drag.start_x;
                self.renderer.context_mut().pill_drag_y = y - drag.start_y;
                self.draw_pill();
            }
            SurfaceKind::TopStrip => {
                self.renderer.context_mut().top_drag_y = (y - drag.start_y).max(0.0);
                self.draw_top_strip();
            }
            SurfaceKind::Overlay => {
                if let Some((id, rect)) = drag.active_slider {
                    let pct = slider_pct_for_x(x, rect);
                    self.handler.on_action(
                        ShellAction::SetSlider { id, pct },
                        self.renderer.context_mut(),
                    );
                    self.draw_overlay();
                } else if drag.start_y >= (self.viewport_h as f32 - 52.0) {
                    self.renderer.context_mut().pill_drag_x = x - drag.start_x;
                    self.renderer.context_mut().pill_drag_y = y - drag.start_y;
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

        self.renderer.context_mut().pill_drag_x = 0.0;
        self.renderer.context_mut().pill_drag_y = 0.0;
        self.renderer.context_mut().top_drag_y = 0.0;
        if let Some(motion) = drag.motion {
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
                if self.active_scene == SceneKind::Pill && vertical(dx, dy) {
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
                    _ => {
                        if dist < 16.0 {
                            let target = if self.active_scene == SceneKind::Pill {
                                SceneKind::Home
                            } else {
                                SceneKind::Pill
                            };
                            self.show_scene(target, true);
                        } else {
                            self.draw_pill();
                        }
                    }
                }
            }
            SurfaceKind::TopStrip => {
                if dist < 10.0 {
                    self.show_scene(SceneKind::Shade, true);
                } else {
                    self.draw_top_strip();
                }
            }
            SurfaceKind::Overlay => {
                if drag.active_slider.is_some() {
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
            HitTarget::CloseWindow { id } => {
                self.handler
                    .on_action(ShellAction::CloseWindow(id), self.renderer.context_mut());
                self.handler.on_action(
                    ShellAction::RequestRefresh(SceneKind::Recents),
                    self.renderer.context_mut(),
                );
                self.capture.apply_cached_and_capture(
                    &mut self.renderer.context_mut().recents_cards,
                    &self.shm,
                    &self.qh,
                );
                self.draw_overlay();
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
                let pct = slider_pct_for_x(x, rect);
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

fn slider_pct_for_x(x: f32, rect: veilshell_render::HitRect) -> u8 {
    if rect.w <= 1.0 {
        return 50;
    }
    let ratio = ((x - rect.x) / rect.w).clamp(0.0, 1.0);
    (ratio * 100.0).round() as u8
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
/// over apps (`< 1 MiB` SHM), and full-screen overlays are dropped cleanly upon dismissal.
///
/// # Errors
/// Returns [`WaylandError`] if connecting to `Wayland`, binding required globals, or selecting the
/// requested output fails.
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
    let mut renderer = ActiveRenderer::new(config.renderer, initial_ctx)
        .map_err(|e| WaylandError::Runtime(format!("renderer init: {e}")))?;
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

    if let Some(seat) = app.seat_state.seats().next() {
        app.osk.attach_seat(&seat, &qh);
    }

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
            .dispatch(app.osk_repeat_timeout(), &mut app)
            .map_err(|e| WaylandError::Runtime(e.to_string()))?;
        if !app.exit {
            app.tick_osk_repeat();
        }
    }

    app.capture.destroy();
    app.osk.destroy();
    let _ = flush_conn.flush();

    // Explicitly tear down all Wayland + wgpu surfaces before the event loop drops.
    app.overlay = None;
    app.top_strip = None;
    app.pill = None;

    Ok(())
}

impl CompositorHandler for LayerApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
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

        let surf = match kind {
            SurfaceKind::Pill => self.pill.as_mut(),
            SurfaceKind::TopStrip => self.top_strip.as_mut(),
            SurfaceKind::Overlay => self.overlay.as_mut(),
        };
        let mut new_w = None;
        let mut new_h = None;
        let mut resized = false;
        if let Some(target) = surf {
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
        self.osk.attach_seat(&seat, qh);
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

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
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
