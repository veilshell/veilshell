//! Destructive-to-the-guest-session protocol test. No input or unlock path.
//! Run only in a disposable standalone `VeilShell` guest; recover by logging out.

use std::{
    env,
    error::Error,
    fs,
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::Path,
};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    reexports::client::{
        self,
        globals::registry_queue_init,
        protocol::{wl_buffer, wl_output, wl_shm, wl_surface},
        Connection, QueueHandle,
    },
    registry::{ProvidesRegistryState, RegistryState},
    session_lock::{
        SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
        SessionLockSurfaceConfigure,
    },
    shm::{raw::RawPool, Shm, ShmHandler},
};

struct Probe {
    registry: RegistryState,
    compositor: CompositorState,
    outputs: OutputState,
    shm: Shm,
    lock: SessionLock,
    surfaces: Vec<(wl_output::WlOutput, SessionLockSurface)>,
    failed: Option<Box<dyn Error>>,
}

fn connect() -> Result<Connection, Box<dyn Error>> {
    if !env::args_os().skip(1).eq(["--disposable-guest-no-unlock"]) {
        return Err("requires --disposable-guest-no-unlock; exits do not unlock".into());
    }
    let runtime = env::var("XDG_RUNTIME_DIR")?;
    let runtime = Path::new(&runtime);
    let directory = env::var("VEILSHELL_SESSION_DIR")?;
    let directory = Path::new(&directory);
    let display = env::var("WAYLAND_DISPLAY")?;
    let signature = env::var("HYPRLAND_INSTANCE_SIGNATURE")?;
    let endpoint = |name: &str| Path::new(name).file_name().and_then(|s| s.to_str()) == Some(name);
    let metadata = fs::symlink_metadata(directory)?;
    if !runtime.is_absolute()
        || directory.parent() != Some(runtime)
        || !directory.file_name().is_some_and(|name| {
            name.to_str()
                .is_some_and(|name| name.starts_with("veilshell."))
        })
        || !metadata.is_dir()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != fs::metadata(runtime)?.uid()
        || !endpoint(&display)
        || !endpoint(&signature)
        || fs::read_to_string(directory.join("display"))?
            .lines()
            .ne([&display, &signature])
    {
        return Err("expected the private standalone VeilShell display".into());
    }
    // Connect to the verified endpoint, never an inherited WAYLAND_SOCKET fd.
    Ok(Connection::from_socket(UnixStream::connect(
        runtime.join(display),
    )?)?)
}

fn run() -> Result<(), Box<dyn Error>> {
    let conn = connect()?;
    let (globals, mut queue) = registry_queue_init(&conn)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let outputs = OutputState::new(&globals, &qh);
    let shm = Shm::bind(&globals, &qh)?;
    if outputs.outputs().next().is_none() {
        return Err("refusing to start the probe with no advertised outputs".into());
    }
    let lock = SessionLockState::new(&globals, &qh).lock(&qh)?;
    let mut app = Probe {
        registry: RegistryState::new(&globals),
        compositor,
        outputs,
        shm,
        lock,
        surfaces: Vec::new(),
        failed: None,
    };
    for output in app.outputs.outputs().collect::<Vec<_>>() {
        app.add_output(output, &qh);
    }
    loop {
        queue.blocking_dispatch(&mut app)?;
        if let Some(err) = app.failed.take() {
            return Err(err);
        }
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("session-lock-probe: {err}");
        std::process::exit(1);
    }
}

impl Probe {
    fn add_output(&mut self, output: wl_output::WlOutput, qh: &QueueHandle<Self>) {
        if self.surfaces.iter().any(|(known, _)| *known == output) {
            return;
        }
        let surface = self.compositor.create_surface(qh);
        let lock_surface = self.lock.create_lock_surface(surface, &output, qh);
        self.surfaces.push((output, lock_surface));
    }

    fn paint(
        &self,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        size: (u32, u32),
    ) -> Result<(), Box<dyn Error>> {
        let (width, height) = size;
        let bytes = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || bytes > 16_777_216 {
            return Err("unsupported test framebuffer size".into());
        }
        let width = i32::try_from(width)?;
        let height = i32::try_from(height)?;
        let mut pool = RawPool::new(usize::try_from(bytes)? * 4, &self.shm)?;
        for pixel in pool.mmap().chunks_exact_mut(4) {
            pixel.copy_from_slice(&0xff34_205c_u32.to_ne_bytes());
        }
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            width * 4,
            wl_shm::Format::Xrgb8888,
            (),
            qh,
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        buffer.destroy();
        Ok(())
    }
}

impl SessionLockHandler for Probe {
    fn locked(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        eprintln!("session-lock-probe: compositor acknowledged lock");
    }

    fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        self.failed = Some("compositor refused or finished the lock".into());
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _: u32,
    ) {
        // SCTK acknowledges configure before dispatching this callback.
        if let Err(err) = self.paint(qh, surface.wl_surface(), configure.new_size) {
            self.failed = Some(err);
        } else {
            let (width, height) = configure.new_size;
            eprintln!("session-lock-probe: committed opaque {width}x{height} surface");
        }
    }
}

impl OutputHandler for Probe {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_output(output, qh);
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.surfaces.retain(|(known, _)| *known != output);
    }
}

impl CompositorHandler for Probe {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl ProvidesRegistryState for Probe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    smithay_client_toolkit::registry_handlers![OutputState];
}

impl ShmHandler for Probe {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

smithay_client_toolkit::delegate_registry!(Probe);
smithay_client_toolkit::delegate_dispatch2!(Probe);
client::delegate_noop!(Probe: ignore wl_buffer::WlBuffer);
