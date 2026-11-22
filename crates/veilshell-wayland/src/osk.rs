//! `Wayland` `zwp_input_method_v2` and `zwp_virtual_keyboard_v1` integration for the Touch `OSK`.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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

use veilshell_core::{OskEmit, OskLayer, OskState, US_QWERTY_XKB_KEYMAP};
use veilshell_render::{HitRect, SceneKind};

use crate::client::LayerApp;
use crate::OskPolicy;

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
        proxy: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
        // Queued events from a released object must not change a newer binding.
        if state.osk.input_method.as_ref() != Some(proxy) {
            return;
        }
        match event {
            zwp_input_method_v2::Event::Activate => {
                state.cancel_osk_drag();
                // Activation resets the content purpose, even when no
                // ContentType event follows.
                state.osk.pending_layout = Some((OskLayer::Letters, false));
                state.osk.editor.pending = Some(EditorChange::Activated);
            }
            zwp_input_method_v2::Event::Deactivate => {
                state.cancel_osk_drag();
                state.osk.pending_layout = None;
                state.osk.editor.pending = Some(EditorChange::Deactivated);
            }
            zwp_input_method_v2::Event::ContentType { purpose, .. } => {
                let layer = match purpose {
                    WEnum::Value(
                        ContentPurpose::Number | ContentPurpose::Digits | ContentPurpose::Phone,
                    ) => OskLayer::Numbers,
                    _ => OskLayer::Letters,
                };
                let terminal = purpose == WEnum::Value(ContentPurpose::Terminal);
                state.osk.pending_layout = Some((layer, terminal));
            }
            zwp_input_method_v2::Event::Done => {
                state.osk.im_serial = state.osk.im_serial.wrapping_add(1);
                let was_available = state.osk.can_open_manually();
                let change = state.osk.editor.done();
                let osk = &mut state.renderer.context_mut().osk_state;
                let previous = *osk;
                if change.is_some() {
                    *osk = OskState::default();
                }
                if let Some((layer, terminal)) = state.osk.pending_layout.take() {
                    osk.layer = layer;
                    osk.terminal = terminal;
                }
                let layout_dirty = *osk != previous;
                if layout_dirty {
                    state.invalidate_osk_layout();
                }
                match change {
                    // Ordinary Done events (e.g. cursor or surrounding-text updates)
                    // never reopen a keyboard the user just dismissed. A deactivate
                    // and activate in the same batch still means a new editor.
                    Some(EditorChange::Activated)
                        if state.visible
                            && state.active_scene == SceneKind::Pill
                            && !state.in_desk_mode() =>
                    {
                        state.osk.auto_opened = true;
                        state.show_scene(SceneKind::Osk, false);
                    }
                    Some(EditorChange::Deactivated) => state.on_osk_editor_lost(),
                    _ if layout_dirty && state.active_scene == SceneKind::Osk => {
                        state.draw_overlay();
                    }
                    _ if was_available != state.osk.can_open_manually() => state.draw_pill(),
                    _ => {}
                }
            }
            zwp_input_method_v2::Event::Unavailable => {
                // Another IME can own this seat. Leave the virtual keyboard
                // fallback available for manual OSK use, without retrying here.
                state.osk.release_input_method();
                state.on_osk_editor_lost();
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditorChange {
    Activated,
    Deactivated,
}

#[derive(Default)]
struct EditorFocus {
    active: bool,
    pending: Option<EditorChange>,
}

impl EditorFocus {
    fn done(&mut self) -> Option<EditorChange> {
        let change = self.pending.take()?;
        self.active = change == EditorChange::Activated;
        Some(change)
    }
}

pub(crate) struct OskContact {
    label: &'static str,
    key: HitRect,
    deadline: Option<Instant>,
    left_key: bool,
    repeated: bool,
}

impl OskContact {
    pub(crate) fn new(now: Instant, label: &'static str, key: HitRect) -> Self {
        Self {
            label,
            key,
            deadline: OskState::can_repeat(label).then(|| now + Duration::from_millis(400)),
            left_key: false,
            repeated: false,
        }
    }

    pub(crate) fn position(&mut self, x: f32, y: f32) {
        if !self.key.contains(x, y) {
            self.left_key = true;
            self.deadline = None;
        }
    }

    pub(crate) fn tap(self) -> Option<&'static str> {
        (!self.left_key && !self.repeated).then_some(self.label)
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(crate) fn tick(&mut self, now: Instant) -> Option<&'static str> {
        if now < self.deadline? {
            return None;
        }
        self.repeated = true;
        // Do not catch up a burst if the compositor or renderer was busy.
        self.deadline = Some(now + Duration::from_millis(55));
        Some(self.label)
    }
}

/// Live state for compositor `zwp_input_method_v2` and `zwp_virtual_keyboard_v1` integration.
pub(crate) struct OskWaylandState {
    im_manager: Option<ZwpInputMethodManagerV2>,
    vk_manager: Option<ZwpVirtualKeyboardManagerV1>,
    seat: Option<WlSeat>,
    input_method: Option<ZwpInputMethodV2>,
    virtual_keyboard: Option<ZwpVirtualKeyboardV1>,
    editor: EditorFocus,
    im_serial: u32,
    pub(crate) auto_opened: bool,
    pending_layout: Option<(OskLayer, bool)>,
    key_seq: u32,
}

impl OskWaylandState {
    /// Optionally bind `zwp_input_method_manager_v2` and `zwp_virtual_keyboard_manager_v1`.
    pub(crate) fn bind_globals(
        globals: &GlobalList,
        qh: &QueueHandle<LayerApp>,
        policy: OskPolicy,
    ) -> Self {
        let im_manager = match policy {
            OskPolicy::Auto => globals.bind(qh, 1..=1, InputMethodManagerData).ok(),
            OskPolicy::Manual => None,
        };
        let vk_manager = globals
            .bind::<ZwpVirtualKeyboardManagerV1, LayerApp, _>(qh, 1..=1, VirtualKeyboardManagerData)
            .ok();
        Self {
            im_manager,
            vk_manager,
            seat: None,
            input_method: None,
            virtual_keyboard: None,
            editor: EditorFocus::default(),
            im_serial: 0,
            auto_opened: false,
            pending_layout: None,
            key_seq: 1,
        }
    }

    /// Claim the first seat while visible. The virtual keyboard stays lazy so
    /// merely showing the shell does not install a new keymap.
    pub(crate) fn attach_seat(&mut self, seat: &WlSeat, qh: &QueueHandle<LayerApp>) {
        // Keep the same seat for IM and VK, even if the compositor denied IM ownership.
        if self.seat.is_some() {
            return;
        }
        self.seat = Some(seat.clone());
        if let Some(mgr) = &self.im_manager {
            self.input_method = Some(mgr.get_input_method(seat, qh, InputMethodData));
        }
    }

    pub(crate) fn detach_seat(&mut self, seat: &WlSeat) -> bool {
        if self.seat.as_ref() != Some(seat) {
            return false;
        }
        self.destroy();
        true
    }

    pub(crate) fn seat(&self) -> Option<&WlSeat> {
        self.seat.as_ref()
    }

    pub(crate) fn has_virtual_keyboard(&self) -> bool {
        self.vk_manager.is_some()
    }

    pub(crate) fn can_open_manually(&self) -> bool {
        self.seat.is_some() && (self.has_virtual_keyboard() || self.editor.active)
    }

    pub(crate) fn editor_changing(&self) -> bool {
        self.editor.pending.is_some()
    }

    pub(crate) fn ensure_virtual_keyboard(&mut self, qh: &QueueHandle<LayerApp>) {
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
        if self.editor_changing() {
            return;
        }
        match action {
            OskEmit::None | OskEmit::Dismiss => {}
            OskEmit::Text {
                text,
                evdev_code,
                shift,
                ctrl,
            } => {
                if !ctrl && self.editor.active {
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
        self.release_input_method();
        self.release_virtual_keyboard();
        self.auto_opened = false;
        self.seat = None;
    }

    pub(crate) fn release_virtual_keyboard(&mut self) {
        if let Some(vk) = self.virtual_keyboard.take() {
            vk.destroy();
        }
    }

    fn release_input_method(&mut self) {
        if let Some(im) = self.input_method.take() {
            im.destroy();
        }
        self.editor = EditorFocus::default();
        self.im_serial = 0;
        self.pending_layout = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fresh_activation_can_auto_open_after_dismissal() {
        let mut editor = EditorFocus {
            pending: Some(EditorChange::Activated),
            ..EditorFocus::default()
        };
        assert!(!editor.active);
        assert_eq!(editor.done(), Some(EditorChange::Activated));
        assert!(editor.active);
        // Text updates in the same editor are not new activations.
        assert_eq!(editor.done(), None);
        // Switching editors can send both events before the next Done.
        editor.pending = Some(EditorChange::Deactivated);
        editor.pending = Some(EditorChange::Activated);
        assert_eq!(editor.done(), Some(EditorChange::Activated));
        editor.pending = Some(EditorChange::Deactivated);
        assert_eq!(editor.done(), Some(EditorChange::Deactivated));
        assert!(!editor.active);
    }

    #[test]
    fn hold_delays_its_first_stroke_and_does_not_catch_up() {
        let now = Instant::now();
        let mut repeat = OskContact::new(now, "del", HitRect::new(0.0, 0.0, 50.0, 50.0));
        assert_eq!(repeat.tick(now + Duration::from_millis(399)), None);
        assert_eq!(repeat.tick(now + Duration::from_millis(400)), Some("del"));
        assert_eq!(repeat.tick(now + Duration::from_secs(3)), Some("del"));
        assert_eq!(repeat.tick(now + Duration::from_secs(3)), None);
        assert_eq!(repeat.tap(), None);
    }

    #[test]
    fn leaving_the_key_cancels_repeat_even_if_the_finger_returns() {
        let now = Instant::now();
        let mut repeat = OskContact::new(now, "left", HitRect::new(0.0, 0.0, 50.0, 50.0));
        repeat.position(60.0, 20.0);
        repeat.position(20.0, 20.0);
        assert_eq!(repeat.deadline(), None);
        assert_eq!(repeat.tick(now + Duration::from_secs(1)), None);
        assert_eq!(repeat.tap(), None);
    }

    #[test]
    fn ordinary_keys_only_emit_on_a_tap_that_stays_on_the_key() {
        let now = Instant::now();
        let key = HitRect::new(0.0, 0.0, 50.0, 50.0);
        assert_eq!(OskContact::new(now, "a", key).tap(), Some("a"));
        let mut contact = OskContact::new(now, "a", key);
        assert_eq!(contact.deadline(), None);
        contact.position(51.0, 25.0);
        assert_eq!(contact.tap(), None);
    }
}
