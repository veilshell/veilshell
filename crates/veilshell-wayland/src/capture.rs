//! `Wayland` `ext-foreign-toplevel-list-v1` + `ext-image-copy-capture-v1` live window thumbnail
//! capture for the `Recents` deck.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::client::backend::ObjectData;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm};
use smithay_client_toolkit::reexports::client::{Connection, Proxy, QueueHandle, WEnum};
use smithay_client_toolkit::reexports::protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use smithay_client_toolkit::reexports::protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use smithay_client_toolkit::reexports::protocols::ext::image_capture_source::v1::client::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1;
use smithay_client_toolkit::reexports::protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use smithay_client_toolkit::reexports::protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self, ExtImageCopyCaptureFrameV1,
};
use smithay_client_toolkit::reexports::protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    self, ExtImageCopyCaptureManagerV1,
};
use smithay_client_toolkit::reexports::protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self, ExtImageCopyCaptureSessionV1,
};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::Shm;

use veilshell_render::{RecentsCard, RgbaImage, SceneKind, ShmPixelFormat};

use crate::client::LayerApp;

const MAX_RECENT_THUMBNAILS: usize = 3;
const THUMB_MAX_W: u32 = 360;
const THUMB_MAX_H: u32 = 260;

pub(crate) struct ForeignToplevelListData;
pub(crate) struct ForeignToplevelHandleData;
pub(crate) struct CaptureSourceManagerData;
pub(crate) struct CaptureSourceData;
pub(crate) struct CaptureManagerData;
pub(crate) struct CaptureSessionData {
    capture_id: u64,
}
pub(crate) struct CaptureFrameData {
    capture_id: u64,
}

impl Dispatch2<ExtForeignToplevelListV1, LayerApp> for ForeignToplevelListData {
    fn event(
        &self,
        state: &mut LayerApp,
        _proxy: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                let obj_id = toplevel.id().protocol_id();
                state.capture.toplevels.insert(
                    obj_id,
                    TrackedToplevel {
                        handle: toplevel,
                        title: String::new(),
                        app_id: String::new(),
                        identifier: None,
                        pending_title: None,
                        pending_app_id: None,
                        pending_identifier: None,
                    },
                );
            }
            ext_foreign_toplevel_list_v1::Event::Finished => {
                state.capture.clear_toplevels();
                if let Some(list) = state.capture.toplevel_list.take() {
                    list.destroy();
                }
            }
            _ => {}
        }
    }

    fn event_created_child(opcode: u16, qh: &QueueHandle<LayerApp>) -> Arc<dyn ObjectData> {
        if opcode == ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE {
            qh.make_data::<ExtForeignToplevelHandleV1, _>(ForeignToplevelHandleData)
        } else {
            panic!(
                "unexpected child event opcode {opcode} on {}",
                ExtForeignToplevelListV1::interface().name
            );
        }
    }
}

impl Dispatch2<ExtForeignToplevelHandleV1, LayerApp> for ForeignToplevelHandleData {
    fn event(
        &self,
        state: &mut LayerApp,
        proxy: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
        let obj_id = proxy.id().protocol_id();
        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(tl) = state.capture.toplevels.get_mut(&obj_id) {
                    tl.pending_title = Some(title);
                }
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(tl) = state.capture.toplevels.get_mut(&obj_id) {
                    tl.pending_app_id = Some(app_id);
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                if let Some(tl) = state.capture.toplevels.get_mut(&obj_id) {
                    tl.pending_identifier = Some(identifier);
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {
                if let Some(tl) = state.capture.toplevels.get_mut(&obj_id) {
                    if let Some(t) = tl.pending_title.take() {
                        tl.title = t;
                    }
                    if let Some(a) = tl.pending_app_id.take() {
                        tl.app_id = a;
                    }
                    if let Some(id) = tl.pending_identifier.take() {
                        tl.identifier = Some(id);
                    }
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                if let Some(tl) = state.capture.toplevels.remove(&obj_id) {
                    if let Some(ident) = tl.identifier {
                        state.capture.remove_cached_by_identifier(&ident);
                    }
                    tl.handle.destroy();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch2<ExtForeignToplevelImageCaptureSourceManagerV1, LayerApp>
    for CaptureSourceManagerData
{
    fn event(
        &self,
        _state: &mut LayerApp,
        _proxy: &ExtForeignToplevelImageCaptureSourceManagerV1,
        _event: <ExtForeignToplevelImageCaptureSourceManagerV1 as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
    }
}

impl Dispatch2<ExtImageCaptureSourceV1, LayerApp> for CaptureSourceData {
    fn event(
        &self,
        _state: &mut LayerApp,
        _proxy: &ExtImageCaptureSourceV1,
        _event: <ExtImageCaptureSourceV1 as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
    }
}

impl Dispatch2<ExtImageCopyCaptureManagerV1, LayerApp> for CaptureManagerData {
    fn event(
        &self,
        _state: &mut LayerApp,
        _proxy: &ExtImageCopyCaptureManagerV1,
        _event: <ExtImageCopyCaptureManagerV1 as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
    }
}

impl Dispatch2<ExtImageCopyCaptureSessionV1, LayerApp> for CaptureSessionData {
    fn event(
        &self,
        state: &mut LayerApp,
        _proxy: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _conn: &Connection,
        qh: &QueueHandle<LayerApp>,
    ) {
        let cap_id = self.capture_id;
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                if let Some(cap) = state.capture.active.get_mut(&cap_id) {
                    cap.pending_size = Some((width, height));
                }
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(shm_fmt),
            } => {
                if let Some((prio, conv)) = map_shm_format(shm_fmt) {
                    if let Some(cap) = state.capture.active.get_mut(&cap_id) {
                        let replace = cap
                            .pending_format
                            .is_none_or(|(cur_prio, _, _)| prio > cur_prio);
                        if replace {
                            cap.pending_format = Some((prio, shm_fmt, conv));
                        }
                    }
                }
            }
            ext_image_copy_capture_session_v1::Event::Done => {
                state.capture.start_frame(cap_id, &state.shm, qh);
            }
            ext_image_copy_capture_session_v1::Event::Stopped => {
                state.capture.cancel_capture(cap_id);
            }
            _ => {}
        }
    }
}

impl Dispatch2<ExtImageCopyCaptureFrameV1, LayerApp> for CaptureFrameData {
    fn event(
        &self,
        state: &mut LayerApp,
        _proxy: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _conn: &Connection,
        _qh: &QueueHandle<LayerApp>,
    ) {
        let cap_id = self.capture_id;
        match event {
            ext_image_copy_capture_frame_v1::Event::Transform {
                transform: WEnum::Value(tf),
            } => {
                if let Some(cap) = state.capture.active.get_mut(&cap_id) {
                    cap.flip_y = matches!(
                        tf,
                        wl_output::Transform::Flipped | wl_output::Transform::Flipped180
                    );
                }
            }
            ext_image_copy_capture_frame_v1::Event::Ready => {
                if let Some((card_id, thumb)) = state.capture.finish_ready_capture(cap_id) {
                    let mut updated = false;
                    for card in &mut state.renderer.context_mut().recents_cards {
                        if card.id == card_id {
                            card.thumbnail = Some(Arc::clone(&thumb));
                            updated = true;
                        }
                    }
                    if updated && state.visible && state.active_scene == SceneKind::Recents {
                        state.draw_overlay();
                    }
                }
            }
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => {
                state.capture.cancel_capture(cap_id);
            }
            _ => {}
        }
    }
}

fn map_shm_format(fmt: wl_shm::Format) -> Option<(u8, ShmPixelFormat)> {
    match fmt {
        wl_shm::Format::Xrgb8888 => Some((4, ShmPixelFormat::Xrgb8888)),
        wl_shm::Format::Argb8888 => Some((3, ShmPixelFormat::Argb8888)),
        wl_shm::Format::Xbgr8888 => Some((2, ShmPixelFormat::Xbgr8888)),
        wl_shm::Format::Abgr8888 => Some((1, ShmPixelFormat::Abgr8888)),
        _ => None,
    }
}

struct TrackedToplevel {
    handle: ExtForeignToplevelHandleV1,
    title: String,
    app_id: String,
    identifier: Option<String>,
    pending_title: Option<String>,
    pending_app_id: Option<String>,
    pending_identifier: Option<String>,
}

struct ActiveCapture {
    card_id: String,
    foreign_id: Option<String>,
    source: ExtImageCaptureSourceV1,
    session: ExtImageCopyCaptureSessionV1,
    frame: Option<ExtImageCopyCaptureFrameV1>,
    pool: Option<SlotPool>,
    buffer: Option<Buffer>,
    pending_size: Option<(u32, u32)>,
    pending_format: Option<(u8, wl_shm::Format, ShmPixelFormat)>,
    captured_size: (u32, u32),
    captured_stride: usize,
    captured_format: Option<ShmPixelFormat>,
    flip_y: bool,
}

impl ActiveCapture {
    fn cleanup_protocol(mut self) {
        if let Some(frame) = self.frame.take() {
            frame.destroy();
        }
        self.session.destroy();
        self.source.destroy();
        drop(self.buffer.take());
        drop(self.pool.take());
    }
}

/// Optional compositor toplevel thumbnail capturer using `ext-image-copy-capture-v1`.
pub(crate) struct ToplevelCaptureState {
    toplevel_list: Option<ExtForeignToplevelListV1>,
    source_mgr: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    capture_mgr: Option<ExtImageCopyCaptureManagerV1>,
    toplevels: HashMap<u32, TrackedToplevel>,
    active: HashMap<u64, ActiveCapture>,
    cache_by_card: HashMap<String, Arc<RgbaImage>>,
    cache_by_identifier: HashMap<String, Arc<RgbaImage>>,
    next_capture_id: u64,
}

impl ToplevelCaptureState {
    /// Optionally bind `ext-foreign-toplevel-list-v1` and `ext-image-copy-capture-v1` globals.
    pub(crate) fn bind_globals(globals: &GlobalList, qh: &QueueHandle<LayerApp>) -> Self {
        let toplevel_list = globals
            .bind::<ExtForeignToplevelListV1, LayerApp, _>(qh, 1..=1, ForeignToplevelListData)
            .ok();
        let source_mgr = globals
            .bind::<ExtForeignToplevelImageCaptureSourceManagerV1, LayerApp, _>(
                qh,
                1..=1,
                CaptureSourceManagerData,
            )
            .ok();
        let capture_mgr = globals
            .bind::<ExtImageCopyCaptureManagerV1, LayerApp, _>(qh, 1..=1, CaptureManagerData)
            .ok();

        Self {
            toplevel_list,
            source_mgr,
            capture_mgr,
            toplevels: HashMap::new(),
            active: HashMap::new(),
            cache_by_card: HashMap::new(),
            cache_by_identifier: HashMap::new(),
            next_capture_id: 1,
        }
    }

    /// Populate any previously captured thumbnails onto `cards` and trigger fresh asynchronous
    /// one-shot captures for up to [`MAX_RECENT_THUMBNAILS`] visible cards.
    pub(crate) fn apply_cached_and_capture(
        &mut self,
        cards: &mut [RecentsCard],
        _shm: &Shm,
        qh: &QueueHandle<LayerApp>,
    ) {
        for card in cards.iter_mut() {
            if card.thumbnail.is_none() {
                let cached = card
                    .foreign_id
                    .as_ref()
                    .and_then(|fid| self.cache_by_identifier.get(fid))
                    .or_else(|| self.cache_by_card.get(&card.id));
                if let Some(thumb) = cached {
                    card.thumbnail = Some(Arc::clone(thumb));
                }
            }
        }

        self.cancel_all_active();

        let (Some(source_mgr), Some(capture_mgr)) = (&self.source_mgr, &self.capture_mgr) else {
            return;
        };

        let mut used_handles = HashSet::new();
        for card in cards.iter().take(MAX_RECENT_THUMBNAILS) {
            let Some((tl_obj_id, tl)) =
                find_toplevel_for_card(card, &self.toplevels, &used_handles)
            else {
                continue;
            };
            used_handles.insert(tl_obj_id);

            let capture_id = self.next_capture_id;
            self.next_capture_id = self.next_capture_id.wrapping_add(1).max(1);

            let source = source_mgr.create_source(&tl.handle, qh, CaptureSourceData);
            let session = capture_mgr.create_session(
                &source,
                ext_image_copy_capture_manager_v1::Options::empty(),
                qh,
                CaptureSessionData { capture_id },
            );

            self.active.insert(
                capture_id,
                ActiveCapture {
                    card_id: card.id.clone(),
                    foreign_id: card.foreign_id.clone().or_else(|| tl.identifier.clone()),
                    source,
                    session,
                    frame: None,
                    pool: None,
                    buffer: None,
                    pending_size: None,
                    pending_format: None,
                    captured_size: (0, 0),
                    captured_stride: 0,
                    captured_format: None,
                    flip_y: false,
                },
            );
        }
    }

    fn start_frame(&mut self, capture_id: u64, shm: &Shm, qh: &QueueHandle<LayerApp>) {
        let Some(cap) = self.active.get_mut(&capture_id) else {
            return;
        };
        // Only capture once per one-shot session if a frame is already in flight.
        if cap.frame.is_some() {
            return;
        }
        let (Some((w, h)), Some((_, wl_fmt, conv_fmt))) = (cap.pending_size, cap.pending_format)
        else {
            self.cancel_capture(capture_id);
            return;
        };
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            self.cancel_capture(capture_id);
            return;
        }

        let row_bytes = w * 4;
        let Some(byte_len) = (row_bytes as usize).checked_mul(h as usize) else {
            self.cancel_capture(capture_id);
            return;
        };
        let Ok(mut pool) = SlotPool::new(byte_len.max(4096), shm) else {
            self.cancel_capture(capture_id);
            return;
        };
        let (Ok(w_i32), Ok(h_i32), Ok(stride)) =
            (i32::try_from(w), i32::try_from(h), i32::try_from(row_bytes))
        else {
            self.cancel_capture(capture_id);
            return;
        };
        let Ok((buffer, _)) = pool.create_buffer(w_i32, h_i32, stride, wl_fmt) else {
            self.cancel_capture(capture_id);
            return;
        };

        let frame = cap
            .session
            .create_frame(qh, CaptureFrameData { capture_id });
        frame.attach_buffer(buffer.wl_buffer());
        frame.damage_buffer(0, 0, w_i32, h_i32);
        frame.capture();

        cap.captured_size = (w, h);
        cap.captured_stride = row_bytes as usize;
        cap.captured_format = Some(conv_fmt);
        cap.pool = Some(pool);
        cap.buffer = Some(buffer);
        cap.frame = Some(frame);
    }

    fn finish_ready_capture(&mut self, capture_id: u64) -> Option<(String, Arc<RgbaImage>)> {
        let mut cap = self.active.remove(&capture_id)?;
        let (w, h) = cap.captured_size;
        let stride = cap.captured_stride;
        let flip_y = cap.flip_y;
        let fmt = cap.captured_format;
        let card_id = cap.card_id.clone();
        let foreign_id = cap.foreign_id.clone();

        let thumb = match (fmt, cap.pool.as_mut(), cap.buffer.as_ref()) {
            (Some(pixel_fmt), Some(pool), Some(buf)) => buf.canvas(pool).and_then(|bytes| {
                let raw = RgbaImage::from_wayland_shm(w, h, stride, bytes, pixel_fmt, flip_y)?;
                Some(Arc::new(raw.resize_fit(THUMB_MAX_W, THUMB_MAX_H)))
            }),
            _ => None,
        };

        cap.cleanup_protocol();

        let thumb = thumb?;
        self.cache_by_card
            .insert(card_id.clone(), Arc::clone(&thumb));
        if let Some(fid) = foreign_id {
            self.cache_by_identifier.insert(fid, Arc::clone(&thumb));
        }
        Some((card_id, thumb))
    }

    fn cancel_capture(&mut self, capture_id: u64) {
        if let Some(cap) = self.active.remove(&capture_id) {
            cap.cleanup_protocol();
        }
    }

    fn cancel_all_active(&mut self) {
        for (_, cap) in self.active.drain() {
            cap.cleanup_protocol();
        }
    }

    fn clear_toplevels(&mut self) {
        for (_, tl) in self.toplevels.drain() {
            tl.handle.destroy();
        }
    }

    fn remove_cached_by_identifier(&mut self, ident: &str) {
        self.cache_by_identifier.remove(ident);
    }

    /// Cleanly release all compositor toplevel handles, active capture sessions, and globals.
    pub(crate) fn destroy(&mut self) {
        self.cancel_all_active();
        self.clear_toplevels();
        if let Some(list) = self.toplevel_list.take() {
            list.destroy();
        }
        if let Some(mgr) = self.source_mgr.take() {
            mgr.destroy();
        }
        if let Some(mgr) = self.capture_mgr.take() {
            mgr.destroy();
        }
        self.cache_by_card.clear();
        self.cache_by_identifier.clear();
    }
}

fn find_toplevel_for_card<'a>(
    card: &RecentsCard,
    toplevels: &'a HashMap<u32, TrackedToplevel>,
    used: &HashSet<u32>,
) -> Option<(u32, &'a TrackedToplevel)> {
    if let Some(fid) = card.foreign_id.as_deref() {
        if let Some((&id, tl)) = toplevels
            .iter()
            .find(|(&id, tl)| !used.contains(&id) && tl.identifier.as_deref() == Some(fid))
        {
            return Some((id, tl));
        }
    }

    if let Some((&id, tl)) = toplevels.iter().find(|(&id, tl)| {
        !used.contains(&id)
            && tl.app_id.eq_ignore_ascii_case(&card.app_id)
            && tl.title == card.title
    }) {
        return Some((id, tl));
    }

    toplevels
        .iter()
        .find(|(&id, tl)| !used.contains(&id) && tl.app_id.eq_ignore_ascii_case(&card.app_id))
        .map(|(&id, tl)| (id, tl))
}
