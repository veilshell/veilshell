//! Test-only input for scripts/test-hyprland-actions.sh. Never use the ambient display.

use std::collections::HashMap;
use std::error::Error;
use std::os::unix::net::UnixStream;
use std::thread::sleep;
use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::{
    client::{
        globals::{registry_queue_init, GlobalListContents},
        protocol::{wl_output, wl_pointer::ButtonState, wl_registry},
        Connection, Dispatch, QueueHandle,
    },
    protocols_wlr::virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
        zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    },
};

const BTN_LEFT: u32 = 0x110;

#[derive(Default)]
struct State(HashMap<u32, String>);

smithay_client_toolkit::reexports::client::delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
smithay_client_toolkit::reexports::client::delegate_noop!(State: ignore ZwlrVirtualPointerV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            state.0.insert(*id, name);
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [socket, x, y, rest @ ..] = args.as_slice() else {
        return Err("usage: hyprland_smoke_pointer /private/wayland-N X Y [[X:]Y ...]".into());
    };
    let x: u32 = x.parse()?;
    let y: u32 = y.parse()?;
    let waypoints = rest
        .iter()
        .map(|point| -> Result<_, std::num::ParseIntError> {
            if let Some((next_x, next_y)) = point.split_once(':') {
                Ok((next_x.parse()?, next_y.parse()?))
            } else {
                Ok((x, point.parse()?))
            }
        })
        .collect::<Result<Vec<(u32, u32)>, _>>()?;
    let extent =
        std::env::var("VEILSHELL_TEST_POINTER_EXTENT").unwrap_or_else(|_| "960x640".into());
    // The default 120 ms per waypoint is used by the existing replay traces.
    // Faster paces allow a short, within-screen flick on the 52 px bottom band.
    let step_ms = match std::env::var("VEILSHELL_TEST_POINTER_STEP_MS") {
        Ok(raw) => raw.parse::<u64>()?,
        Err(std::env::VarError::NotPresent) => 20,
        Err(err) => return Err(err.into()),
    };
    if !(4..=50).contains(&step_ms) {
        return Err("VEILSHELL_TEST_POINTER_STEP_MS must be between 4 and 50".into());
    }
    let (width, height) = extent.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
    let (width, height) = (width.parse::<u32>()?, height.parse::<u32>()?);
    if std::iter::once((x, y))
        .chain(waypoints.iter().copied())
        .any(|(x, y)| x >= width || y >= height)
    {
        return Err("pointer coordinates outside the supplied extent".into());
    }
    let connection = Connection::from_socket(UnixStream::connect(socket)?)?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
    let qh = queue.handle();
    let manager: ZwlrVirtualPointerManagerV1 = globals.bind(&qh, 2..=2, ())?;
    let outputs: Vec<_> = globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|g| g.interface == "wl_output" && g.version >= 4)
        .map(|g| {
            (
                g.name,
                globals
                    .registry()
                    .bind::<wl_output::WlOutput, _, _>(g.name, 4, &qh, g.name),
            )
        })
        .collect();
    let mut state = State::default();
    queue.roundtrip(&mut state)?;
    let (_, output) = outputs
        .iter()
        .find(|(id, _)| state.0.get(id).is_some_and(|name| name == "veil-smoke"))
        .ok_or("private veil-smoke output not found")?;
    let pointer = manager.create_virtual_pointer_with_output(None, Some(output), &qh, ());
    queue.roundtrip(&mut state)?;
    // A headless seat has no pointer before this virtual device appears.
    // Let the shell process the capability change and subscribe to wl_pointer.
    sleep(Duration::from_millis(100));
    let start = Instant::now();
    let timestamp = || start.elapsed().as_millis() as u32;
    let move_to = |x, y| {
        pointer.motion_absolute(timestamp(), x, y, width, height);
        pointer.frame();
    };
    move_to(x.saturating_sub(1), y);
    queue.roundtrip(&mut state)?;
    move_to(x, y);
    queue.roundtrip(&mut state)?;
    pointer.button(timestamp(), BTN_LEFT, ButtonState::Pressed);
    pointer.frame();
    queue.roundtrip(&mut state)?;
    let mut prev = (x, y);
    for next in waypoints {
        for step in 1..=6_u64 {
            sleep(Duration::from_millis(step_ms));
            let lerp =
                |start, end| ((u64::from(start) * (6 - step) + u64::from(end) * step) / 6) as u32;
            move_to(lerp(prev.0, next.0), lerp(prev.1, next.1));
            queue.roundtrip(&mut state)?;
        }
        prev = next;
    }
    pointer.button(timestamp(), BTN_LEFT, ButtonState::Released);
    pointer.frame();
    queue.roundtrip(&mut state)?;
    sleep(Duration::from_millis(50));
    pointer.destroy();
    connection.flush()?;
    Ok(())
}
