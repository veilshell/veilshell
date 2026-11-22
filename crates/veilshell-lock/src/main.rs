//! Dedicated `ext-session-lock-v1` touch + keyboard screen locker for `VeilShell`.
//!
//! Security properties:
//! - Completely isolated from the main `VeilShell` daemon, IPC, clipboard, and
//!   global virtual-keyboard/input-method OSK.
//! - Renders opaque `Xrgb8888` lock surfaces on every current and hotplugged output.
//! - Uses `veilshell-lock-input` for locker-local touch keyboard geometry, hit
//!   testing, and serialized retry/cancellation state.
//! - Authenticates against Linux-PAM (`pam_authenticate` + `pam_acct_mgmt`) on a
//!   dedicated worker thread, zeroizing secrets immediately.
//! - Fail-closed: never unlocks on cancel, error, signal, disconnect, `finished`,
//!   panic, or drop.

mod pam;
mod secret;
mod ui;
mod worker;

use std::env;
use std::error::Error;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{self, Event as ChannelEvent};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_buffer, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch,
};
use smithay_client_toolkit::reexports::client::{self, Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::touch::TouchHandler;
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::session_lock::{
    SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
    SessionLockSurfaceConfigure,
};
use smithay_client_toolkit::shm::raw::RawPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use veilshell_core::ThemePalette;
use veilshell_lock_input::attempt::Attempts;
use veilshell_lock_input::{Action, Keyboard};
use veilshell_render::{RenderTarget, SdfSoftwareRenderer};

use crate::pam::{current_username, default_pam_service, local_clock_strings, AuthOutcome};
use crate::secret::SecretPassword;
use crate::ui::{
    build_lock_draw_list, keyboard_rect, AuthBanner, KeyboardRect, LockStage, LockView,
};
use crate::worker::{AuthResult, AuthWorker};

const MAX_PIXELS: u64 = 16_777_216;
const POINTER_SYNTHETIC_TOUCH_ID: i32 = -1;
const SWIPE_UP_THRESHOLD: f64 = 48.0;
const TAP_SLOP: f64 = 18.0;

struct OutputSurface {
    output: wl_output::WlOutput,
    lock_surface: SessionLockSurface,
    logical_width: u32,
    logical_height: u32,
    scale: i32,
    configured: bool,
}

struct CoverGesture {
    touch_id: i32,
    start_x: f64,
    start_y: f64,
    last_x: f64,
    last_y: f64,
    max_travel: f64,
}

struct LockApp {
    registry: RegistryState,
    compositor: CompositorState,
    outputs: OutputState,
    seat_state: SeatState,
    shm: Shm,
    qh: QueueHandle<Self>,

    lock: Option<SessionLock>,
    locked: bool,
    unlocked_cleanly: bool,
    failed: Option<String>,

    surfaces: Vec<OutputSurface>,
    active_seat: Option<wl_seat::WlSeat>,
    wl_keyboard: Option<wl_keyboard::WlKeyboard>,
    wl_touch: Option<wl_touch::WlTouch>,
    wl_pointer: Option<wl_pointer::WlPointer>,
    pointer_surface: Option<wl_surface::WlSurface>,
    pointer_pos: (f64, f64),
    pointer_pressed: bool,

    stage: LockStage,
    banner: AuthBanner,
    banner_clear_at: Option<Instant>,
    cover_gesture: Option<CoverGesture>,
    keyboard: Keyboard,
    keyboard_rect: KeyboardRect,
    password: SecretPassword,
    attempts: Attempts,
    worker: AuthWorker,

    username: String,
    time_text: String,
    date_text: String,
    palette: ThemePalette,
    renderer: SdfSoftwareRenderer,
}

fn parse_pam_service() -> Result<String, Box<dyn Error>> {
    let mut service = default_pam_service().to_owned();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pam-service" => {
                let Some(val) = args.next() else {
                    return Err("--pam-service requires a value".into());
                };
                if val.is_empty() || val.contains('/') || val.contains('\0') {
                    return Err("invalid --pam-service name".into());
                }
                service = val;
            }
            "--help" | "-h" => {
                println!("veilshell-lock [--pam-service <name>]");
                std::process::exit(0);
            }
            other => {
                return Err(format!("unknown argument: {other}").into());
            }
        }
    }
    Ok(service)
}

fn connect_wayland() -> Result<Connection, Box<dyn Error>> {
    if let Ok(session_dir) = env::var("VEILSHELL_SESSION_DIR") {
        let runtime = env::var("XDG_RUNTIME_DIR")?;
        let runtime = Path::new(&runtime);
        let directory = Path::new(&session_dir);
        let display = env::var("WAYLAND_DISPLAY")?;
        let signature = env::var("HYPRLAND_INSTANCE_SIGNATURE")?;
        let safe_name =
            |name: &str| Path::new(name).file_name().and_then(|s| s.to_str()) == Some(name);
        let metadata = fs::symlink_metadata(directory)?;
        if !runtime.is_absolute()
            || directory.parent() != Some(runtime)
            || !directory
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("veilshell."))
            || !metadata.is_dir()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != fs::metadata(runtime)?.uid()
            || !safe_name(&display)
            || !safe_name(&signature)
            || fs::read_to_string(directory.join("display"))?
                .lines()
                .ne([&display, &signature])
        {
            return Err("invalid standalone VeilShell display handshake".into());
        }
        return Ok(Connection::from_socket(UnixStream::connect(
            runtime.join(display),
        )?)?);
    }
    Ok(Connection::connect_to_env()?)
}

fn run() -> Result<(), Box<dyn Error>> {
    let pam_service = parse_pam_service()?;
    let username = current_username()?;
    let conn = connect_wayland()?;

    let (globals, queue) = registry_queue_init::<LockApp>(&conn)?;
    let qh = queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let outputs = OutputState::new(&globals, &qh);
    let seat_state = SeatState::new(&globals, &qh);
    let shm = Shm::bind(&globals, &qh)?;

    let lock = SessionLockState::new(&globals, &qh).lock(&qh)?;

    let mut event_loop: EventLoop<LockApp> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();

    let (auth_tx, auth_rx) = channel::channel::<AuthResult>();
    let worker = AuthWorker::spawn(pam_service, username.clone(), auth_tx);
    let (time_text, date_text) = local_clock_strings();

    let mut app = LockApp {
        registry: RegistryState::new(&globals),
        compositor,
        outputs,
        seat_state,
        shm,
        qh: qh.clone(),
        lock: Some(lock),
        locked: false,
        unlocked_cleanly: false,
        failed: None,
        surfaces: Vec::new(),
        active_seat: None,
        wl_keyboard: None,
        wl_touch: None,
        wl_pointer: None,
        pointer_surface: None,
        pointer_pos: (0.0, 0.0),
        pointer_pressed: false,
        stage: LockStage::Cover,
        banner: AuthBanner::Idle,
        banner_clear_at: None,
        cover_gesture: None,
        keyboard: Keyboard::default(),
        keyboard_rect: keyboard_rect(1200, 800),
        password: SecretPassword::default(),
        attempts: Attempts::default(),
        worker,
        username,
        time_text,
        date_text,
        palette: ThemePalette::default_dark(),
        renderer: SdfSoftwareRenderer::default(),
    };

    for output in app.outputs.outputs().collect::<Vec<_>>() {
        app.add_output(output, &qh);
    }

    WaylandSource::new(conn.clone(), queue)
        .insert(loop_handle.clone())
        .map_err(|err| format!("failed to register Wayland source: {err}"))?;

    loop_handle
        .insert_source(auth_rx, |event, (), state| match event {
            ChannelEvent::Msg(result) => state.handle_auth_result(result),
            ChannelEvent::Closed => {
                // Fail closed if the PAM worker dies unexpectedly.
                state.attempts.cancel();
                state.banner = AuthBanner::Denied;
                state.redraw_all();
            }
        })
        .map_err(|err| format!("failed to register auth channel: {err}"))?;

    loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_secs(1)),
            |_, (), state| {
                state.on_tick();
                TimeoutAction::ToDuration(Duration::from_secs(1))
            },
        )
        .map_err(|err| format!("failed to register clock timer: {err}"))?;

    while !app.unlocked_cleanly && app.failed.is_none() {
        event_loop.dispatch(Some(Duration::from_millis(250)), &mut app)?;
        let _ = conn.flush();
    }

    if app.unlocked_cleanly {
        let _ = conn.flush();
        let _ = conn.roundtrip();
        Ok(())
    } else {
        let reason = app
            .failed
            .unwrap_or_else(|| "session lock exited without authentication".to_owned());
        Err(reason.into())
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("veilshell-lock: {err}");
        std::process::exit(1);
    }
}

impl LockApp {
    fn add_output(&mut self, output: wl_output::WlOutput, qh: &QueueHandle<Self>) {
        if self.surfaces.iter().any(|s| s.output == output) {
            return;
        }
        let Some(lock) = &self.lock else {
            return;
        };
        let scale = self
            .outputs
            .info(&output)
            .map_or(1, |info| info.scale_factor.max(1));
        let surface = self.compositor.create_surface(qh);
        surface.set_buffer_scale(scale);
        let lock_surface = lock.create_lock_surface(surface, &output, qh);
        self.surfaces.push(OutputSurface {
            output,
            lock_surface,
            logical_width: 0,
            logical_height: 0,
            scale,
            configured: false,
        });
    }

    fn is_primary_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.surfaces
            .first()
            .is_some_and(|first| first.lock_surface.wl_surface() == surface)
    }

    fn is_lock_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.surfaces
            .iter()
            .any(|entry| entry.lock_surface.wl_surface() == surface)
    }

    fn reconfigure_primary_keyboard(&mut self) {
        self.attempts.cancel();
        self.cover_gesture = None;
        if let Some(primary) = self.surfaces.first() {
            let rect = keyboard_rect(primary.logical_width, primary.logical_height);
            self.keyboard_rect = rect;
            self.keyboard.configure(rect.width, rect.height);
        } else {
            self.keyboard.configure(0, 0);
        }
    }

    fn enter_password_stage(&mut self) {
        if self.stage == LockStage::Password {
            return;
        }
        self.stage = LockStage::Password;
        self.cover_gesture = None;
        self.keyboard.disarm();
        if self.banner == AuthBanner::Denied {
            self.banner = AuthBanner::Idle;
        }
        self.redraw_all();
    }

    fn cancel_to_cover(&mut self) {
        self.password.clear();
        self.attempts.cancel();
        self.keyboard.disarm();
        self.cover_gesture = None;
        self.banner = AuthBanner::Idle;
        self.banner_clear_at = None;
        self.stage = LockStage::Cover;
        self.redraw_all();
    }

    fn submit_password(&mut self) {
        if !self.locked || self.stage != LockStage::Password {
            return;
        }
        if self.password.is_empty() {
            self.banner = AuthBanner::Idle;
            self.redraw_all();
            return;
        }
        let now = Instant::now();
        let Some(ticket) = self.attempts.begin(now) else {
            self.banner = AuthBanner::WaitRetry;
            self.banner_clear_at = Some(now + Duration::from_secs(2));
            self.redraw_all();
            return;
        };
        self.keyboard.disarm();
        let secret = self.password.take_for_auth();
        self.banner = AuthBanner::Checking;
        self.banner_clear_at = None;
        if self.worker.submit(ticket, secret).is_err() {
            let _ = self.attempts.complete(ticket, false, Instant::now());
            self.banner = AuthBanner::Denied;
            self.banner_clear_at = Some(Instant::now() + Duration::from_secs(3));
        }
        self.redraw_all();
    }

    fn apply_keyboard_action(&mut self, action: &Action) {
        if self.stage != LockStage::Password {
            return;
        }
        match *action {
            Action::Character(ch) => {
                if self.password.push_char(ch) && self.banner == AuthBanner::Denied {
                    self.banner = AuthBanner::Idle;
                }
                self.redraw_all();
            }
            Action::Backspace => {
                let _ = self.password.pop_char();
                self.redraw_all();
            }
            Action::Cancel => {
                self.cancel_to_cover();
            }
            Action::Submit => {
                self.submit_password();
            }
        }
    }

    fn handle_auth_result(&mut self, result: AuthResult) {
        let now = Instant::now();
        let authenticated = result.outcome == AuthOutcome::Authenticated;
        let accepted = self.attempts.complete(result.ticket, authenticated, now);

        // Unlock ONLY when the compositor has acknowledged the lock, the password
        // editor is still active, and `Attempts` confirms this is the current,
        // uncancelled PAM success.
        if accepted && self.locked && self.stage == LockStage::Password {
            self.password.clear();
            if let Some(lock) = self.lock.take() {
                lock.unlock();
                self.unlocked_cleanly = true;
            }
            return;
        }

        if self.banner == AuthBanner::Checking {
            self.banner = AuthBanner::Denied;
            self.banner_clear_at = Some(now + Duration::from_secs(3));
            self.redraw_all();
        }
    }

    fn on_tick(&mut self) {
        let mut dirty = false;
        let (time_text, date_text) = local_clock_strings();
        if time_text != self.time_text || date_text != self.date_text {
            self.time_text = time_text;
            self.date_text = date_text;
            dirty = true;
        }
        if let Some(until) = self.banner_clear_at {
            if Instant::now() >= until {
                self.banner_clear_at = None;
                if matches!(self.banner, AuthBanner::Denied | AuthBanner::WaitRetry) {
                    self.banner = AuthBanner::Idle;
                    dirty = true;
                }
            }
        }
        if dirty {
            self.redraw_all();
        }
    }

    fn redraw_all(&mut self) {
        let count = self.surfaces.len();
        for idx in 0..count {
            if let Err(err) = self.draw_surface(idx) {
                self.failed = Some(err);
                break;
            }
        }
    }

    fn draw_surface(&self, index: usize) -> Result<(), String> {
        let Some(entry) = self.surfaces.get(index) else {
            return Ok(());
        };
        if !entry.configured || entry.logical_width == 0 || entry.logical_height == 0 {
            return Ok(());
        }
        let scale = u32::try_from(entry.scale.max(1)).unwrap_or(1);
        let phys_w = entry.logical_width.saturating_mul(scale);
        let phys_h = entry.logical_height.saturating_mul(scale);
        let pixel_count = u64::from(phys_w) * u64::from(phys_h);
        if pixel_count == 0 || pixel_count > MAX_PIXELS {
            return Err("lock surface framebuffer size out of bounds".to_owned());
        }

        let drag_up = if index == 0 {
            self.cover_gesture
                .as_ref()
                .map_or(0.0, |g| (g.start_y - g.last_y).max(0.0) as f32)
        } else {
            0.0
        };
        let view = LockView {
            width: entry.logical_width,
            height: entry.logical_height,
            is_primary: index == 0,
            stage: self.stage,
            time_text: &self.time_text,
            date_text: &self.date_text,
            username: &self.username,
            masked_chars: self.password.char_count(),
            banner: self.banner,
            cover_drag_up: drag_up,
            keyboard: &self.keyboard,
            keyboard_rect: self.keyboard_rect,
        };
        let list = build_lock_draw_list(&view, &self.palette);
        let target = RenderTarget::new(
            (entry.logical_width, entry.logical_height),
            (phys_w, phys_h),
        )
        .map_err(|err| format!("invalid lock render target: {err}"))?;
        let frame = self
            .renderer
            .rasterize_scaled(target, &list)
            .map_err(|err| format!("lock surface rasterization failed: {err}"))?;

        let width_i32 = i32::try_from(phys_w).map_err(|_| "width overflow".to_owned())?;
        let height_i32 = i32::try_from(phys_h).map_err(|_| "height overflow".to_owned())?;
        let byte_len = usize::try_from(pixel_count)
            .map_err(|_| "pixel count overflow".to_owned())?
            .checked_mul(4)
            .ok_or_else(|| "buffer size overflow".to_owned())?;

        let mut pool = RawPool::new(byte_len, &self.shm)
            .map_err(|err| format!("failed to allocate lock SHM pool: {err}"))?;
        let dst = pool.mmap();
        // Write opaque little-endian XRGB8888 ([B, G, R, 0xFF]) for every pixel.
        for (src_px, dst_px) in frame.rgba.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
            dst_px[0] = src_px[2];
            dst_px[1] = src_px[1];
            dst_px[2] = src_px[0];
            dst_px[3] = 0xFF;
        }

        let buffer = pool.create_buffer(
            0,
            width_i32,
            height_i32,
            width_i32 * 4,
            wl_shm::Format::Xrgb8888,
            (),
            &self.qh,
        );
        let wl_surface = entry.lock_surface.wl_surface();
        wl_surface.set_buffer_scale(entry.scale.max(1));
        wl_surface.attach(Some(&buffer), 0, 0);
        wl_surface.damage_buffer(0, 0, width_i32, height_i32);
        wl_surface.commit();
        buffer.destroy();
        Ok(())
    }

    fn retire_touch_stream(&mut self) {
        self.keyboard.cancel_touches();
        self.cover_gesture = None;
        self.attempts.cancel();
        if let Some(touch) = self.wl_touch.take() {
            touch.release();
        }
    }
}

impl SessionLockHandler for LockApp {
    fn locked(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _session_lock: SessionLock) {
        self.locked = true;
    }

    fn finished(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _session_lock: SessionLock,
    ) {
        self.lock = None;
        self.failed = Some("compositor refused or terminated session lock".to_owned());
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(index) = self
            .surfaces
            .iter()
            .position(|entry| entry.lock_surface.wl_surface() == surface.wl_surface())
        else {
            return;
        };
        let (w, h) = configure.new_size;
        let entry = &mut self.surfaces[index];
        let resized = entry.configured && (entry.logical_width, entry.logical_height) != (w, h);
        entry.logical_width = w;
        entry.logical_height = h;
        entry.configured = true;

        if index == 0 && (!resized || w > 0 && h > 0) {
            self.reconfigure_primary_keyboard();
        }
        if let Err(err) = self.draw_surface(index) {
            self.failed = Some(err);
        }
    }
}

impl OutputHandler for LockApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.add_output(output, qh);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let Some(info) = self.outputs.info(&output) else {
            return;
        };
        let scale = info.scale_factor.max(1);
        if let Some(index) = self.surfaces.iter().position(|s| s.output == output) {
            if self.surfaces[index].scale != scale {
                self.surfaces[index].scale = scale;
                if let Err(err) = self.draw_surface(index) {
                    self.failed = Some(err);
                }
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let primary_lost = self
            .surfaces
            .first()
            .is_some_and(|first| first.output == output);
        self.surfaces.retain(|entry| entry.output != output);
        if primary_lost {
            self.keyboard.cancel_touches();
            self.reconfigure_primary_keyboard();
            self.redraw_all();
        }
    }
}

impl CompositorHandler for LockApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if let Some(index) = self
            .surfaces
            .iter()
            .position(|entry| entry.lock_surface.wl_surface() == surface)
        {
            self.surfaces[index].scale = new_factor.max(1);
            if let Err(err) = self.draw_surface(index) {
                self.failed = Some(err);
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
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
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

impl SeatHandler for LockApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.active_seat.is_none() {
            self.active_seat = Some(seat);
        }
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if self.active_seat.is_none() {
            self.active_seat = Some(seat.clone());
        }
        if self.active_seat.as_ref() != Some(&seat) {
            return;
        }
        match capability {
            Capability::Keyboard if self.wl_keyboard.is_none() => {
                if let Ok(kbd) = self.seat_state.get_keyboard(qh, &seat, None) {
                    self.wl_keyboard = Some(kbd);
                }
            }
            Capability::Touch if self.wl_touch.is_none() => {
                if let Ok(touch) = self.seat_state.get_touch(qh, &seat) {
                    self.wl_touch = Some(touch);
                }
            }
            Capability::Pointer if self.wl_pointer.is_none() => {
                if let Ok(ptr) = self.seat_state.get_pointer(qh, &seat) {
                    self.wl_pointer = Some(ptr);
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if self.active_seat.as_ref() != Some(&seat) {
            return;
        }
        match capability {
            Capability::Keyboard => {
                if let Some(kbd) = self.wl_keyboard.take() {
                    kbd.release();
                }
            }
            Capability::Touch => self.retire_touch_stream(),
            Capability::Pointer => {
                self.pointer_pressed = false;
                self.pointer_surface = None;
                if let Some(ptr) = self.wl_pointer.take() {
                    ptr.release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.active_seat.as_ref() != Some(&seat) {
            return;
        }
        self.retire_touch_stream();
        if let Some(kbd) = self.wl_keyboard.take() {
            kbd.release();
        }
        if let Some(ptr) = self.wl_pointer.take() {
            ptr.release();
        }
        self.pointer_pressed = false;
        self.pointer_surface = None;
        self.active_seat = self.seat_state.seats().find(|s| s != &seat);
    }
}

impl TouchHandler for LockApp {
    fn down(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        if self.wl_touch.as_ref() != Some(touch) || !self.is_lock_surface(&surface) {
            return;
        }
        let on_primary = self.is_primary_surface(&surface);
        match self.stage {
            LockStage::Cover => {
                if on_primary && self.cover_gesture.is_none() {
                    self.cover_gesture = Some(CoverGesture {
                        touch_id: id,
                        start_x: position.0,
                        start_y: position.1,
                        last_x: position.0,
                        last_y: position.1,
                        max_travel: 0.0,
                    });
                }
            }
            LockStage::Password => {
                if on_primary {
                    self.keyboard.down(
                        id,
                        position.0 - self.keyboard_rect.x,
                        position.1 - self.keyboard_rect.y,
                    );
                } else {
                    // Report secondary-surface contact outside the keyboard to block competing taps.
                    self.keyboard.down(id, -1.0, -1.0);
                }
            }
        }
    }

    fn up(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        id: i32,
    ) {
        if self.wl_touch.as_ref() != Some(touch) {
            return;
        }
        match self.stage {
            LockStage::Cover => {
                if self
                    .cover_gesture
                    .as_ref()
                    .is_some_and(|g| g.touch_id == id)
                {
                    let gesture = self.cover_gesture.take().expect("checked");
                    let dy = gesture.start_y - gesture.last_y;
                    if dy >= SWIPE_UP_THRESHOLD || gesture.max_travel <= TAP_SLOP {
                        self.enter_password_stage();
                    } else {
                        self.redraw_all();
                    }
                }
            }
            LockStage::Password => {
                self.keyboard.up(id);
                if let Some(action) = self.keyboard.frame() {
                    self.apply_keyboard_action(&action);
                } else {
                    // Shift or page toggle updates key legends.
                    self.redraw_all();
                }
            }
        }
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        touch: &wl_touch::WlTouch,
        _time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        if self.wl_touch.as_ref() != Some(touch) {
            return;
        }
        match self.stage {
            LockStage::Cover => {
                if let Some(gesture) = &mut self.cover_gesture {
                    if gesture.touch_id == id {
                        gesture.last_x = position.0;
                        gesture.last_y = position.1;
                        let travel =
                            (position.0 - gesture.start_x).hypot(position.1 - gesture.start_y);
                        gesture.max_travel = gesture.max_travel.max(travel);
                        if gesture.start_y - gesture.last_y >= SWIPE_UP_THRESHOLD * 1.5 {
                            self.enter_password_stage();
                        } else {
                            self.redraw_all();
                        }
                    }
                }
            }
            LockStage::Password => {
                self.keyboard.motion(
                    id,
                    position.0 - self.keyboard_rect.x,
                    position.1 - self.keyboard_rect.y,
                );
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

    fn cancel(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, touch: &wl_touch::WlTouch) {
        if self.wl_touch.as_ref() != Some(touch) {
            return;
        }
        self.keyboard.cancel_touches();
        self.cover_gesture = None;
        self.redraw_all();
    }
}

impl KeyboardHandler for LockApp {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if self.wl_keyboard.as_ref() != Some(keyboard) {
            return;
        }
        match event.keysym {
            Keysym::Escape => {
                if self.password.is_empty() {
                    self.cancel_to_cover();
                } else {
                    self.password.clear();
                    self.attempts.cancel();
                    self.banner = AuthBanner::Idle;
                    self.redraw_all();
                }
            }
            Keysym::BackSpace => {
                if self.stage == LockStage::Password {
                    let _ = self.password.pop_char();
                    self.redraw_all();
                }
            }
            Keysym::Return | Keysym::KP_Enter => {
                if self.stage == LockStage::Cover {
                    self.enter_password_stage();
                } else {
                    self.submit_password();
                }
            }
            _ => {
                if let Some(text) = event.utf8 {
                    let mut changed = false;
                    for ch in text.chars() {
                        if !ch.is_control() {
                            if self.stage == LockStage::Cover {
                                self.stage = LockStage::Password;
                            }
                            if self.password.push_char(ch) {
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        if self.banner == AuthBanner::Denied {
                            self.banner = AuthBanner::Idle;
                        }
                        self.redraw_all();
                    }
                }
            }
        }
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _raw_modifiers: smithay_client_toolkit::seat::keyboard::RawModifiers,
        _layout: u32,
    ) {
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if self.wl_keyboard.as_ref() != Some(keyboard) {
            return;
        }
        if event.keysym == Keysym::BackSpace && self.stage == LockStage::Password {
            let _ = self.password.pop_char();
            self.redraw_all();
        }
    }
}

impl PointerHandler for LockApp {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        if self.wl_pointer.as_ref() != Some(pointer) {
            return;
        }
        for event in events {
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_surface = Some(event.surface.clone());
                    self.pointer_pos = event.position;
                }
                PointerEventKind::Leave { .. } => {
                    if self.pointer_pressed {
                        self.pointer_pressed = false;
                        self.keyboard.cancel_touches();
                    }
                    self.pointer_surface = None;
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_pos = event.position;
                    if self.pointer_pressed && self.stage == LockStage::Password {
                        self.keyboard.motion(
                            POINTER_SYNTHETIC_TOUCH_ID,
                            event.position.0 - self.keyboard_rect.x,
                            event.position.1 - self.keyboard_rect.y,
                        );
                    }
                }
                PointerEventKind::Press { button: 0x110, .. } => {
                    if !self.is_primary_surface(&event.surface) {
                        continue;
                    }
                    self.pointer_pos = event.position;
                    match self.stage {
                        LockStage::Cover => self.enter_password_stage(),
                        LockStage::Password => {
                            self.pointer_pressed = true;
                            self.keyboard.down(
                                POINTER_SYNTHETIC_TOUCH_ID,
                                event.position.0 - self.keyboard_rect.x,
                                event.position.1 - self.keyboard_rect.y,
                            );
                        }
                    }
                }
                PointerEventKind::Release { button: 0x110, .. } if self.pointer_pressed => {
                    self.pointer_pressed = false;
                    self.keyboard.up(POINTER_SYNTHETIC_TOUCH_ID);
                    if let Some(action) = self.keyboard.frame() {
                        self.apply_keyboard_action(&action);
                    } else {
                        self.redraw_all();
                    }
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for LockApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for LockApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    smithay_client_toolkit::registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_registry!(LockApp);
smithay_client_toolkit::delegate_dispatch2!(LockApp);
client::delegate_noop!(LockApp: ignore wl_buffer::WlBuffer);
