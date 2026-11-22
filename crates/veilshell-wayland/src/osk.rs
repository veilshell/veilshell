//! `Wayland` `zwp_input_method_v2` and `zwp_virtual_keyboard_v1` integration for the Touch `OSK`.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::path::PathBuf;

use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle, WEnum};
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_v3::ContentPurpose;
use smithay_client_toolkit::reexports::protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use smithay_client_toolkit::reexports::protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::{
    self, ZwpInputMethodV2,
};
use smithay_client_toolkit::reexports::protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use smithay_client_toolkit::reexports::protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

use veilshell_core::{OskEmit, OskLayer, US_QWERTY_XKB_KEYMAP};
use veilshell_render::SceneKind;

use crate::client::LayerApp;

pub(crate) struct InputMethodManagerData;
pub(crate) struct InputMethodData;
pub(crate) struct VirtualKeyboardManagerData;
pub(crate) struct VirtualKeyboardData;

impl Dispatch2<ZwpInputMethodManagerV2, LayerApp> for InputMethodManagerData {
    fn event(
        &self,
        _state: &mut LayerApp,
        _proxy: &ZwpInputMethodManagerV2,
        _event: <ZwpInputMethodManagerV2 as smithay_client_toolkit::reexports::client::Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
    }
}

impl Dispatch2<ZwpVirtualKeyboardManagerV1, LayerApp> for VirtualKeyboardManagerData {
    fn event(
        &self,
        _state: &mut LayerApp,
        _proxy: &ZwpVirtualKeyboardManagerV1,
        _event: <ZwpVirtualKeyboardManagerV1 as smithay_client_toolkit::reexports::client::Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
    }
}

impl Dispatch2<ZwpVirtualKeyboardV1, LayerApp> for VirtualKeyboardData {
    fn event(
        &self,
        _state: &mut LayerApp,
        _proxy: &ZwpVirtualKeyboardV1,
        _event: <ZwpVirtualKeyboardV1 as smithay_client_toolkit::reexports::client::Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
    }
}

impl Dispatch2<ZwpInputMethodV2, LayerApp> for InputMethodData {
    fn event(
        &self,
        state: &mut LayerApp,
        _proxy: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => {
                state.osk.im_pending_active = true;
            }
            zwp_input_method_v2::Event::Deactivate => {
                state.osk.im_pending_active = false;
            }
            zwp_input_method_v2::Event::ContentType {
                hint: _,
                purpose:
                    WEnum::Value(
                        ContentPurpose::Number | ContentPurpose::Digits | ContentPurpose::Phone,
                    ),
            } => {
                state.renderer.context_mut().osk_state.layer = OskLayer::Numbers;
            }
            zwp_input_method_v2::Event::ContentType { .. } => {
                state.renderer.context_mut().osk_state.layer = OskLayer::Letters;
            }
            zwp_input_method_v2::Event::Done => {
                state.osk.im_serial = state.osk.im_serial.wrapping_add(1);
                let was_active = state.osk.im_active;
                let now_active = state.osk.im_pending_active;
                state.osk.im_active = now_active;
                if now_active
                    && !was_active
                    && state.visible
                    && state.active_scene == SceneKind::Pill
                {
                    state.osk.auto_opened = true;
                    state.show_scene(SceneKind::Osk, false);
                } else if !now_active
                    && was_active
                    && state.active_scene == SceneKind::Osk
                    && state.osk.auto_opened
                {
                    state.osk.auto_opened = false;
                    state.show_scene(SceneKind::Pill, false);
                }
            }
            zwp_input_method_v2::Event::Unavailable => {
                state.osk.input_method = None;
                state.osk.im_pending_active = false;
                state.osk.im_active = false;
                state.osk.auto_opened = false;
            }
            _ => {}
        }
    }
}

/// Live state for compositor `zwp_input_method_v2` and `zwp_virtual_keyboard_v1` integration.
pub(crate) struct OskWaylandState {
    im_manager: Option<ZwpInputMethodManagerV2>,
    vk_manager: Option<ZwpVirtualKeyboardManagerV1>,
    seat: Option<WlSeat>,
    pub(crate) input_method: Option<ZwpInputMethodV2>,
    pub(crate) virtual_keyboard: Option<ZwpVirtualKeyboardV1>,
    pub(crate) im_pending_active: bool,
    pub(crate) im_active: bool,
    pub(crate) im_serial: u32,
    pub(crate) auto_opened: bool,
    key_seq: u32,
}

impl OskWaylandState {
    /// Optionally bind `zwp_input_method_manager_v2` and `zwp_virtual_keyboard_manager_v1`.
    pub(crate) fn bind_globals(globals: &GlobalList, qh: &QueueHandle<LayerApp>) -> Self {
        let im_manager = globals
            .bind::<ZwpInputMethodManagerV2, LayerApp, _>(qh, 1..=1, InputMethodManagerData)
            .ok();
        let vk_manager = globals
            .bind::<ZwpVirtualKeyboardManagerV1, LayerApp, _>(qh, 1..=1, VirtualKeyboardManagerData)
            .ok();
        Self {
            im_manager,
            vk_manager,
            seat: None,
            input_method: None,
            virtual_keyboard: None,
            im_pending_active: false,
            im_active: false,
            im_serial: 0,
            auto_opened: false,
            key_seq: 1,
        }
    }

    /// Attach the input-method listener on the first available `wl_seat` (virtual keyboard
    /// is created lazily on the first OSK keystroke so startup never disturbs compositor keymaps).
    pub(crate) fn attach_seat(&mut self, seat: &WlSeat, qh: &QueueHandle<LayerApp>) {
        if self.seat.is_none() {
            self.seat = Some(seat.clone());
        }
        if self.input_method.is_none() {
            if let Some(mgr) = &self.im_manager {
                self.input_method = Some(mgr.get_input_method(seat, qh, InputMethodData));
            }
        }
    }

    fn ensure_virtual_keyboard(&mut self, qh: &QueueHandle<LayerApp>) {
        if self.virtual_keyboard.is_some() {
            return;
        }
        let (Some(mgr), Some(seat)) = (&self.vk_manager, &self.seat) else {
            return;
        };
        let vk = mgr.create_virtual_keyboard(seat, qh, VirtualKeyboardData);
        if upload_xkb_keymap(&vk).is_some() {
            self.virtual_keyboard = Some(vk);
        } else {
            vk.destroy();
        }
    }

    /// Emit text or virtual keystrokes to the active client/compositor.
    pub(crate) fn emit(&mut self, action: OskEmit, qh: &QueueHandle<LayerApp>) {
        match action {
            OskEmit::None | OskEmit::Dismiss => {}
            OskEmit::Text {
                text,
                evdev_code,
                shift,
                ctrl,
            } => {
                if !ctrl && self.im_active {
                    if let Some(im) = &self.input_method {
                        im.commit_string(text);
                        im.commit(self.im_serial);
                        return;
                    }
                }
                if let Some(code) = evdev_code {
                    self.ensure_virtual_keyboard(qh);
                    self.send_vk_stroke(code, shift, ctrl);
                }
            }
            OskEmit::SpecialKey { evdev_code, ctrl } => {
                self.ensure_virtual_keyboard(qh);
                self.send_vk_stroke(evdev_code, false, ctrl);
            }
        }
    }

    /// Release compositor-bound IME and virtual keyboard objects cleanly.
    pub(crate) fn destroy(&mut self) {
        if let Some(im) = self.input_method.take() {
            im.destroy();
        }
        if let Some(vk) = self.virtual_keyboard.take() {
            vk.destroy();
        }
        self.im_active = false;
        self.im_pending_active = false;
        self.auto_opened = false;
    }

    fn send_vk_stroke(&mut self, evdev_code: u32, shift: bool, ctrl: bool) {
        let Some(vk) = &self.virtual_keyboard else {
            return;
        };
        let mods = (u32::from(shift)) | (if ctrl { 4 } else { 0 });
        if mods != 0 {
            vk.modifiers(mods, 0, 0, 0);
        }
        let t_press = self.key_seq;
        self.key_seq = self.key_seq.wrapping_add(1);
        vk.key(t_press, evdev_code, 1);
        let t_release = self.key_seq;
        self.key_seq = self.key_seq.wrapping_add(1);
        vk.key(t_release, evdev_code, 0);
        if mods != 0 {
            vk.modifiers(0, 0, 0, 0);
        }
    }
}

fn upload_xkb_keymap(vk: &ZwpVirtualKeyboardV1) -> Option<()> {
    let runtime_dir =
        std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let tmp_path = runtime_dir.join(format!("veilshell-keymap-{}.xkb", std::process::id()));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)
        .ok()?;
    let _ = fs::remove_file(&tmp_path);

    let mut bytes = US_QWERTY_XKB_KEYMAP.as_bytes().to_vec();
    bytes.push(0);
    file.write_all(&bytes).ok()?;
    file.flush().ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;

    let len = u32::try_from(bytes.len()).ok()?;
    // format = 1 corresponds to `wl_keyboard::keymap_format::xkb_v1`.
    vk.keymap(1, file.as_fd(), len);
    Some(())
}
