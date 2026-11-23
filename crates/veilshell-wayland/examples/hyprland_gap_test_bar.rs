//! Click/touch witness for an isolated Hyprland test. Never consults the ambient display.
//! Run as `hyprland_gap_test_bar /private/wayland-N OUTPUT`; presses print to stdout.

use std::{collections::HashMap, error::Error, os::unix::net::UnixStream, path::Path};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_dispatch2, delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::client::{
        globals::registry_queue_init,
        protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch},
        Connection, Proxy, QueueHandle,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        touch::TouchHandler,
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};

const HEIGHT: i32 = 32;
const FILL: [u8; 4] = 0xffbe_2873_u32.to_ne_bytes();

struct Bar {
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    shm: Shm,
    pool: SlotPool,
    layer: Option<LayerSurface>,
    width: i32,
    pointers: HashMap<wl_seat::WlSeat, wl_pointer::WlPointer>,
    touches: HashMap<wl_seat::WlSeat, wl_touch::WlTouch>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [socket, output_name] = args.as_slice() else {
        return Err("usage: hyprland_gap_test_bar /private/wayland-N OUTPUT".into());
    };
    if !Path::new(socket).is_absolute() || output_name.is_empty() {
        return Err(
            "an absolute private Wayland socket path and explicit output are required".into(),
        );
    }
    let conn = Connection::from_socket(UnixStream::connect(socket)?)?;
    let (globals, mut queue) = registry_queue_init::<Bar>(&conn)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let mut bar = Bar {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seats: SeatState::new(&globals, &qh),
        pool: SlotPool::new(4096, &shm)?,
        shm,
        layer: None,
        width: 0,
        pointers: HashMap::new(),
        touches: HashMap::new(),
    };
    queue.roundtrip(&mut bar)?;
    let output = bar
        .outputs
        .outputs()
        .find(|output| {
            bar.outputs
                .info(output)
                .is_some_and(|info| info.name.as_deref() == Some(output_name.as_str()))
        })
        .ok_or_else(|| format!("Wayland output not found: {output_name}"))?;
    println!(
        "seat capabilities: seats={} touch={}",
        bar.seats.seats().count(),
        !bar.touches.is_empty()
    );

    let layer = shell.create_layer_surface(
        &qh,
        compositor.create_surface(&qh),
        Layer::Top,
        Some("veil-gap-test-bar"),
        Some(&output),
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, HEIGHT as u32);
    layer.set_exclusive_zone(HEIGHT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.commit();
    bar.layer = Some(layer);
    while bar.layer.is_some() {
        queue.blocking_dispatch(&mut bar)?;
    }
    Ok(())
}

impl Bar {
    fn owns(&self, surface: &wl_surface::WlSurface) -> bool {
        self.layer
            .as_ref()
            .is_some_and(|layer| layer.wl_surface() == surface)
    }
}

impl LayerShellHandler for Bar {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.layer = None;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        // Left/right anchors plus requested width 0 mean the compositor sets the width.
        // A solid fill only needs a scale-1 buffer, even on a scaled output.
        let width = i32::try_from(configure.new_size.0).expect("bar configure width");
        assert!(width > 0, "bar configure must provide a width");
        if self.width == width {
            return;
        }
        let stride = width.checked_mul(4).expect("bar buffer stride");
        let (buffer, pixels) = self
            .pool
            .create_buffer(width, HEIGHT, stride, wl_shm::Format::Xrgb8888)
            .expect("bar SHM buffer");
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&FILL);
        }
        let surface = layer.wl_surface();
        surface.damage(0, 0, width, HEIGHT);
        buffer.attach_to(surface).expect("attach bar buffer");
        layer.commit();
        self.width = width;
    }
}

impl SeatHandler for Bar {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => {
                let pointer = self.seats.get_pointer(qh, &seat).expect("seat pointer");
                self.pointers.insert(seat, pointer);
            }
            Capability::Touch => {
                let touch = self.seats.get_touch(qh, &seat).expect("seat touch");
                self.touches.insert(seat, touch);
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => {
                if let Some(pointer) = self.pointers.remove(&seat).filter(|p| p.version() >= 3) {
                    pointer.release();
                }
            }
            Capability::Touch => {
                if let Some(touch) = self.touches.remove(&seat).filter(|t| t.version() >= 3) {
                    touch.release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.remove_capability(conn, qh, seat.clone(), Capability::Pointer);
        self.remove_capability(conn, qh, seat, Capability::Touch);
    }
}

impl PointerHandler for Bar {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events.iter().filter(|event| self.owns(&event.surface)) {
            if let PointerEventKind::Press { button, .. } = event.kind {
                let (x, y) = event.position;
                println!("pointer press button={button} x={x:.1} y={y:.1}");
            }
        }
    }
}

impl TouchHandler for Bar {
    fn down(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: u32,
        _: u32,
        surface: wl_surface::WlSurface,
        id: i32,
        (x, y): (f64, f64),
    ) {
        if self.owns(&surface) {
            println!("touch down id={id} x={x:.1} y={y:.1}");
        }
    }

    fn up(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: u32,
        _: u32,
        _: i32,
    ) {
    }
    fn motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: u32,
        _: i32,
        _: (f64, f64),
    ) {
    }
    fn shape(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: i32,
        _: f64,
        _: f64,
    ) {
    }
    fn orientation(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: i32,
        _: f64,
    ) {
    }
    fn cancel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch) {}
}

impl CompositorHandler for Bar {
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

impl OutputHandler for Bar {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Bar {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Bar {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(Bar);
delegate_dispatch2!(Bar);
