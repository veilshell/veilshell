//! Live `wlr-layer-shell` `Wayland` client using `smithay-client-toolkit` and `wl_shm`.

use std::fmt;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
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
    classify_bottom_pill, GestureAction, GestureSample, GestureThresholds, Orientation,
    ThemePalette,
};
use veilshell_render::{
    RendererBackend, SceneContext, SdfSoftwareRenderer, SnapshotSpec, ViewportSize,
};

use crate::{LayerRole, LayerSurfaceSpec, WaylandRuntimeConfig};

/// Errors raised while connecting to or running the `Wayland` layer-shell client.
#[derive(Debug)]
pub enum WaylandError {
    /// Failed to connect to the `Wayland` compositor socket.
    Connect(String),
    /// Required `Wayland` protocol global (`zwlr_layer_shell_v1`, `wl_shm`, etc.) is unavailable.
    MissingGlobal(String),
    /// Error during event dispatch or buffer allocation.
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

struct ActiveDrag {
    touch_id: Option<i32>,
    start_x: f32,
    start_y: f32,
    last_x: f32,
    last_y: f32,
    start_time: Instant,
}

struct LayerApp {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    role: LayerRole,
    width: u32,
    height: u32,
    configured: bool,
    exit: bool,
    palette: ThemePalette,
    renderer: SdfSoftwareRenderer,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
    drag: Option<ActiveDrag>,
}

impl LayerApp {
    fn draw(&mut self, _qh: &QueueHandle<Self>) {
        if !self.configured || self.width == 0 || self.height == 0 {
            return;
        }
        let spec = SnapshotSpec {
            scene: self.role.to_scene(),
            viewport: ViewportSize {
                width: self.width,
                height: self.height,
                scale: 1.0,
            },
            orientation: Orientation::Normal,
        };
        let Ok(frame) = self.renderer.render_snapshot(&spec, &self.palette) else {
            return;
        };
        let argb = frame.to_argb8888_premultiplied();
        let stride = i32::try_from(self.width * 4).unwrap_or(i32::MAX);
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            i32::try_from(self.width).unwrap_or(1),
            i32::try_from(self.height).unwrap_or(1),
            stride,
            wl_shm::Format::Argb8888,
        ) else {
            return;
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
        if buffer.attach_to(surface).is_ok() {
            surface.commit();
        }
    }

    fn finish_drag(&mut self, qh: &QueueHandle<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let elapsed = drag.start_time.elapsed().as_secs_f32().max(0.016);
        let dx = drag.last_x - drag.start_x;
        let dy = drag.last_y - drag.start_y;
        let sample = GestureSample {
            dx,
            dy,
            vx: dx / elapsed,
            vy: dy / elapsed,
            viewport_height: (self.height as f32).max(600.0),
            has_foreground: self.role == LayerRole::BottomPill,
        };
        let action = classify_bottom_pill(sample, GestureThresholds::default());
        self.renderer.context.pill_drag_x = 0.0;
        self.renderer.context.pill_drag_y = 0.0;
        if action != GestureAction::None {
            eprintln!("veilshell: recognized gesture {action:?}");
        }
        self.draw(qh);
    }
}

/// Connect to the active `Wayland` compositor and present a `wlr-layer-shell` surface.
///
/// # Errors
/// Returns [`WaylandError`] if the `Wayland` socket or `zwlr_layer_shell_v1` global is unavailable.
#[allow(clippy::too_many_lines)]
pub fn run_layer_surface(
    config: &WaylandRuntimeConfig,
    role: LayerRole,
    palette: ThemePalette,
    scene_ctx: SceneContext,
) -> Result<(), WaylandError> {
    let conn = if let Some(display) = &config.display_override {
        std::env::set_var("WAYLAND_DISPLAY", display);
        Connection::connect_to_env()
    } else {
        Connection::connect_to_env()
    }
    .map_err(|e| WaylandError::Connect(e.to_string()))?;

    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|e| WaylandError::Runtime(e.to_string()))?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| WaylandError::MissingGlobal(format!("wl_compositor: {e}")))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|e| WaylandError::MissingGlobal(format!("zwlr_layer_shell_v1: {e}")))?;
    let shm = Shm::bind(&globals, &qh)
        .map_err(|e| WaylandError::MissingGlobal(format!("wl_shm: {e}")))?;

    let spec = LayerSurfaceSpec::for_role(role, 1280, 853);
    let wlr_layer = match role {
        LayerRole::BottomPill | LayerRole::QuickSettings => Layer::Overlay,
        LayerRole::HomeGrid | LayerRole::RecentsDeck | LayerRole::TouchOsk => Layer::Top,
    };

    let wl_surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, wl_surface, wlr_layer, Some("veilshell"), None);

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

    // When anchored to both opposing edges, pass 0 so the compositor fills the output dimension.
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

    let initial_bytes = (spec.width as usize) * (spec.height as usize) * 4;
    let pool = SlotPool::new(initial_bytes.max(1024 * 64), &shm)
        .map_err(|e| WaylandError::Runtime(format!("shm slot pool: {e}")))?;

    let mut app = LayerApp {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        role,
        width: spec.width,
        height: spec.height,
        configured: false,
        exit: false,
        palette,
        renderer: SdfSoftwareRenderer::with_context(scene_ctx),
        pointer: None,
        touch: None,
        drag: None,
    };

    let deadline = config
        .exit_after_ms
        .map(|ms| Instant::now() + Duration::from_millis(ms));

    while !app.exit {
        event_queue
            .roundtrip(&mut app)
            .map_err(|e| WaylandError::Runtime(e.to_string()))?;

        if let Some(limit) = deadline {
            if Instant::now() >= limit {
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
        } else {
            event_queue
                .blocking_dispatch(&mut app)
                .map_err(|e| WaylandError::Runtime(e.to_string()))?;
        }
    }

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
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.draw(qh);
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
        _output: wl_output::WlOutput,
    ) {
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
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if configure.new_size.0 > 0 {
            self.width = configure.new_size.0;
        }
        if configure.new_size.1 > 0 {
            self.height = configure.new_size.1;
        }
        self.configured = true;
        self.draw(qh);
    }
}

impl SeatHandler for LayerApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

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
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
        if capability == Capability::Touch {
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
        qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Press { .. } => {
                    self.drag = Some(ActiveDrag {
                        touch_id: None,
                        start_x: x,
                        start_y: y,
                        last_x: x,
                        last_y: y,
                        start_time: Instant::now(),
                    });
                }
                PointerEventKind::Motion { .. } => {
                    if let Some(drag) = &mut self.drag {
                        if drag.touch_id.is_none() {
                            drag.last_x = x;
                            drag.last_y = y;
                            self.renderer.context.pill_drag_x = x - drag.start_x;
                            self.renderer.context.pill_drag_y = y - drag.start_y;
                            self.draw(qh);
                        }
                    }
                }
                PointerEventKind::Release { .. } => {
                    self.finish_drag(qh);
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
        _surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        if self.drag.is_none() {
            let (x, y) = (position.0 as f32, position.1 as f32);
            self.drag = Some(ActiveDrag {
                touch_id: Some(id),
                start_x: x,
                start_y: y,
                last_x: x,
                last_y: y,
                start_time: Instant::now(),
            });
        }
    }

    fn up(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        id: i32,
    ) {
        if self.drag.as_ref().is_some_and(|d| d.touch_id == Some(id)) {
            self.finish_drag(qh);
        }
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        if let Some(drag) = &mut self.drag {
            if drag.touch_id == Some(id) {
                drag.last_x = position.0 as f32;
                drag.last_y = position.1 as f32;
                self.renderer.context.pill_drag_x = drag.last_x - drag.start_x;
                self.renderer.context.pill_drag_y = drag.last_y - drag.start_y;
                self.draw(qh);
            }
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

    fn cancel(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _touch: &wl_touch::WlTouch) {
        self.drag = None;
        self.renderer.context.pill_drag_x = 0.0;
        self.renderer.context.pill_drag_y = 0.0;
        self.draw(qh);
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
