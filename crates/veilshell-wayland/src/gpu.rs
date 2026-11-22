//! Isolated zero-copy `wgpu::Surface` bridge for `Wayland` `wlr-layer-shell` surfaces.
//!
//! This module contains the ONLY `unsafe` call in the entire `VeilShell` workspace: wrapping raw
//! `wl_display` and `wl_surface` pointers into `raw-window-handle` to construct a Vulkan/GLES
//! `wgpu::Surface<'static>`. Lifetime safety is enforced structurally by `ManagedSurface` (which
//! drops `gpu_surface` before `layer`) and `LayerApp` (which keeps `conn` alive until after all
//! surfaces are dropped).

use std::ptr::NonNull;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::reexports::client::protocol::wl_surface;
use smithay_client_toolkit::reexports::client::{Connection, Proxy};
use veilshell_render::{DrawList, RenderTarget, WgpuRenderer};

/// Zero-copy Vulkan/GLES `Wayland` swapchain bound to a single `wlr-layer-shell` `wl_surface`.
pub(crate) struct WgpuWaylandSurface {
    surface: wgpu::Surface<'static>,
    config: Option<wgpu::SurfaceConfiguration>,
}

impl WgpuWaylandSurface {
    /// Create a `wgpu::Surface` targeting `wl_surface` on `conn`.
    ///
    /// # Safety invariant (enforced by caller)
    /// Both `conn` (`wl_display`) and `wl_surface` MUST remain alive until this
    /// `WgpuWaylandSurface` is dropped.
    pub(crate) fn create(
        instance: &wgpu::Instance,
        conn: &Connection,
        wl_surface: &wl_surface::WlSurface,
    ) -> Result<Self, String> {
        let display_ptr = NonNull::new(conn.backend().display_ptr().cast())
            .ok_or_else(|| "null Wayland wl_display pointer".to_owned())?;
        let surface_ptr = NonNull::new(wl_surface.id().as_ptr().cast())
            .ok_or_else(|| "null Wayland wl_surface pointer".to_owned())?;

        let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display_ptr));
        let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(surface_ptr));

        let surface = create_surface_raw(instance, raw_display_handle, raw_window_handle)?;
        Ok(Self {
            surface,
            config: None,
        })
    }

    /// Configure (or reconfigure) the swapchain for the current physical extent.
    fn configure(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Err("cannot configure zero-sized wgpu surface".to_owned());
        }
        let max = device.limits().max_texture_dimension_2d;
        if width > max || height > max {
            return Err(format!(
                "{width}x{height} wgpu surface exceeds GPU texture dimension limit {max}"
            ));
        }
        if let Some(cfg) = &mut self.config {
            if cfg.width == width && cfg.height == height {
                return Ok(());
            }
            cfg.width = width;
            cfg.height = height;
            self.surface.configure(device, cfg);
            return Ok(());
        }

        let caps = self.surface.get_capabilities(adapter);
        if caps.formats.is_empty() {
            return Err("wgpu adapter does not support presenting to Wayland surface".to_owned());
        }

        // Prefer non-sRGB linear-byte formats so GPU output matches SdfSoftwareRenderer 1:1.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .unwrap_or(caps.formats[0]);

        // Premultiplied alpha is required for crisp transparent pill/top-strip & frosted overlays.
        let alpha_mode = if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PostMultiplied)
        {
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };

        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };

        let cfg = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        self.surface.configure(device, &cfg);
        self.config = Some(cfg);
        Ok(())
    }

    /// Render `list` directly into the next physical `Wayland` swapchain image.
    ///
    /// `before_present` queues app-owned Wayland state for the WSI's commit.
    /// Returns `Ok(false)` on a temporary acquire timeout (no buffer committed).
    pub(crate) fn present_draw_list(
        &mut self,
        renderer: &mut WgpuRenderer,
        target: RenderTarget,
        list: &DrawList,
        before_present: impl FnOnce(),
    ) -> Result<bool, String> {
        let (width, height) = target.physical_size();
        self.configure(renderer.adapter(), renderer.device(), width, height)?;
        let format = self
            .config
            .as_ref()
            .ok_or_else(|| "wgpu surface unconfigured".to_owned())?
            .format;

        let frame = match self.surface.get_current_texture() {
            Ok(tex) => tex,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                if let Some(cfg) = &self.config {
                    self.surface.configure(renderer.device(), cfg);
                }
                match self.surface.get_current_texture() {
                    Ok(tex) => tex,
                    Err(wgpu::SurfaceError::Timeout) => return Ok(false),
                    Err(e) => return Err(format!("wgpu acquire after reconfigure failed: {e}")),
                }
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(false),
            Err(err) => return Err(format!("wgpu acquire failed: {err}")),
        };
        let actual = (frame.texture.width(), frame.texture.height());
        if actual != (width, height) {
            return Err(format!(
                "swapchain returned {}x{} for requested {width}x{height} buffer",
                actual.0, actual.1
            ));
        }

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .render_to_texture_view_scaled(&view, format, target, list)
            .map_err(|e| e.to_string())?;

        before_present();
        frame.present();
        Ok(true)
    }
}

#[allow(unsafe_code)]
fn create_surface_raw(
    instance: &wgpu::Instance,
    raw_display_handle: RawDisplayHandle,
    raw_window_handle: RawWindowHandle,
) -> Result<wgpu::Surface<'static>, String> {
    // SAFETY: The caller (`ManagedSurface` inside `LayerApp`) guarantees `wl_display` (`Connection`)
    // and `wl_surface` (`LayerSurface`) outlive the returned `wgpu::Surface<'static>`.
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle,
                raw_window_handle,
            })
            .map_err(|e| format!("create_surface_unsafe failed: {e}"))
    }
}
