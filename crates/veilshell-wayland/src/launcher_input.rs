//! Physical keyboard input, accepted only while the launcher surface owns focus.

use smithay_client_toolkit::reexports::client::protocol::{wl_keyboard, wl_seat, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, Proxy, QueueHandle};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::shell::WaylandSurface;
use veilshell_render::SceneKind;

use super::LayerApp;

struct SeatKeyboard {
    seat: wl_seat::WlSeat,
    keyboard: wl_keyboard::WlKeyboard,
    focused: bool,
    mods: Modifiers,
}

#[derive(Default)]
pub(super) struct LauncherInput {
    keyboards: Vec<SeatKeyboard>,
    pub(super) opened_from_hidden: bool,
}

impl LauncherInput {
    pub(super) fn add(&mut self, seat: wl_seat::WlSeat, keyboard: wl_keyboard::WlKeyboard) {
        self.remove(&seat);
        self.keyboards.push(SeatKeyboard {
            seat,
            keyboard,
            focused: false,
            mods: Modifiers::default(),
        });
    }

    pub(super) fn remove(&mut self, seat: &wl_seat::WlSeat) {
        if let Some(index) = self.keyboards.iter().position(|entry| &entry.seat == seat) {
            let keyboard = self.keyboards.swap_remove(index).keyboard;
            if keyboard.version() >= 3 {
                keyboard.release();
            }
        }
    }

    pub(super) fn clear_focus(&mut self) {
        for entry in &mut self.keyboards {
            entry.focused = false;
        }
    }

    fn get(&mut self, keyboard: &wl_keyboard::WlKeyboard) -> Option<&mut SeatKeyboard> {
        self.keyboards
            .iter_mut()
            .find(|entry| &entry.keyboard == keyboard)
    }
}

impl KeyboardHandler for LayerApp {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        if let Some(entry) = self.launcher_input.get(keyboard) {
            entry.focused = self.active_scene == SceneKind::Launcher
                && self
                    .overlay
                    .as_ref()
                    .is_some_and(|overlay| overlay.layer.wl_surface() == surface);
        }
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(entry) = self.launcher_input.get(keyboard) {
            entry.focused = false;
        }
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        let Some(entry) = self
            .launcher_input
            .get(keyboard)
            .filter(|entry| entry.focused)
        else {
            return;
        };
        if self.active_scene != SceneKind::Launcher
            || entry.mods.ctrl
            || entry.mods.logo
            || entry.mods.alt
        {
            return;
        }
        let width = self
            .overlay
            .as_ref()
            .map_or(self.viewport_w, |surf| surf.width) as f32;
        let ctx = self.renderer.context_mut();
        match event.keysym {
            Keysym::Escape => {
                self.show_scene(SceneKind::Pill, false);
                return;
            }
            Keysym::Return | Keysym::KP_Enter => {
                if let Some(index) = ctx.launcher.selected_app(&ctx.home_apps) {
                    self.launch_app_at(index);
                }
                return;
            }
            Keysym::BackSpace => ctx.launcher.backspace(),
            Keysym::Left | Keysym::ISO_Left_Tab => {
                ctx.launcher
                    .move_selection(&ctx.home_apps, width, false, false);
            }
            Keysym::Right => ctx
                .launcher
                .move_selection(&ctx.home_apps, width, true, false),
            Keysym::Tab => {
                ctx.launcher
                    .move_selection(&ctx.home_apps, width, !entry.mods.shift, false);
            }
            Keysym::Up | Keysym::Down => ctx.launcher.move_selection(
                &ctx.home_apps,
                width,
                event.keysym == Keysym::Down,
                true,
            ),
            _ => {
                if let Some(text) = event.utf8 {
                    ctx.launcher.type_text(&text);
                }
            }
        }
        self.draw_overlay();
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        if let Some(entry) = self.launcher_input.get(keyboard) {
            entry.mods = modifiers;
        }
    }
}
