//! Isolated zero-copy `wgpu::Surface` bridge for `Wayland` `wlr-layer-shell` surfaces.
//!
//! This module contains the ONLY `unsafe` call in the entire `VeilShell` workspace: wrapping raw
//! `wl_display` and `wl_surface` pointers into `raw-window-handle` to construct a Vulkan/GLES
//! `wgpu::Surface<'static>`. Lifetime safety is enforced structurally by `ManagedSurface` (which
//! drops `gpu_surface` before `layer`) and `LayerApp` (which keeps `conn` alive until after all
//! surfaces are dropped).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::NonNull;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::reexports::client::protocol::wl_surface;
use smithay_client_toolkit::reexports::client::{Connection, Proxy};
use veilshell_render::{
    ActiveRenderer, DrawList, RenderTarget, RendererPreference, SceneContext, WgpuRenderer,
};

/// Turn a panicking GPU driver/HAL call into the same failure path as a returned WSI error.
/// After such a panic, the caller must stop using the backend rather than try it again.
pub(crate) fn gpu_attempt<T>(
    stage: &str,
    attempt: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(attempt)).map_err(|panic| {
        let reason = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic payload");
        format!("{stage}: {reason}")
    })?
}

/// Select a backend against this Wayland connection, not a headless adapter.
pub(crate) fn init_renderer(
    preference: RendererPreference,
    context: SceneContext,
    conn: &Connection,
    probe: impl FnOnce() -> smithay_client_toolkit::compositor::Surface,
) -> Result<ActiveRenderer, String> {
    select_renderer(preference, context, |context| {
        let probe = probe();
        let enabled = wgpu::Backends::from_env()
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| {
                // wgpu-hal 26 probes X11 even when targeting Wayland. XOpenDisplay can
                // block on lazy XWayland during session startup; keep GLES opt-in there.
                if std::env::var_os("DISPLAY").is_some() {
                    wgpu::Backends::VULKAN
                } else {
                    wgpu::Backends::VULKAN | wgpu::Backends::GL
                }
            });
        try_backends(enabled, |backend| {
            gpu_attempt("Wayland GPU initialization panicked", || {
                let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                    backends: backend,
                    ..Default::default()
                });
                let surface = WgpuWaylandSurface::create(&instance, conn, probe.wl_surface())?;
                WgpuRenderer::with_surface(
                    instance,
                    &surface.surface,
                    context.clone(),
                    preference == RendererPreference::Auto,
                )
                .map_err(|err| err.to_string())
            })
        })
    })
}

fn select_renderer(
    preference: RendererPreference,
    context: SceneContext,
    try_gpu: impl FnOnce(&SceneContext) -> Result<WgpuRenderer, String>,
) -> Result<ActiveRenderer, String> {
    if preference != RendererPreference::Software {
        match try_gpu(&context) {
            Ok(renderer) => return Ok(ActiveRenderer::Wgpu(Box::new(renderer))),
            Err(err) if preference == RendererPreference::Wgpu => return Err(err),
            Err(err) => eprintln!("veilshell: Wayland GPU unavailable ({err}); using sdf-software"),
        }
    }
    ActiveRenderer::new(RendererPreference::Software, context).map_err(|err| err.to_string())
}

fn try_backends<T>(
    enabled: wgpu::Backends,
    mut attempt: impl FnMut(wgpu::Backends) -> Result<T, String>,
) -> Result<T, String> {
    let mut errors = Vec::new();
    // Do not initialize GLES at all when Vulkan works. Separate instances also prevent GLES
    // surface creation from running when an explicit override enables both backends.
    for (backend, name) in [
        (wgpu::Backends::VULKAN, "Vulkan"),
        (wgpu::Backends::GL, "GLES"),
    ] {
        if enabled.contains(backend) {
            match attempt(backend) {
                Ok(renderer) => return Ok(renderer),
                Err(err) => errors.push(format!("{name}: {err}")),
            }
        }
    }
    if errors.is_empty() {
        return Err(format!("no supported Wayland GPU backend in {enabled:?}"));
    }
    Err(errors.join("; "))
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{gpu_attempt, select_renderer, try_backends};
    use veilshell_render::{RendererPreference, SceneContext};
    use wgpu::Backends;

    #[test]
    fn tries_vulkan_first_then_gles_and_honors_override() {
        let mut tried = Vec::new();
        let result = try_backends(Backends::VULKAN | Backends::GL, |backend| {
            tried.push(backend);
            Err::<(), _>("unavailable".to_owned())
        });
        assert_eq!(tried, [Backends::VULKAN, Backends::GL]);
        assert_eq!(
            result.unwrap_err(),
            "Vulkan: unavailable; GLES: unavailable"
        );

        let result = try_backends(Backends::GL, Ok);
        assert_eq!(result.unwrap(), Backends::GL);

        let result = try_backends(Backends::VULKAN | Backends::GL, |backend| {
            assert_eq!(backend, Backends::VULKAN);
            Ok(backend)
        });
        assert_eq!(result.unwrap(), Backends::VULKAN);
    }

    #[test]
    fn recovers_from_hal_panics_as_well_as_returned_errors() {
        assert_eq!(
            gpu_attempt::<()>("surface creation", || panic!("BadDisplay")),
            Err("surface creation: BadDisplay".to_owned())
        );
        assert_eq!(
            gpu_attempt::<()>("presentation", || Err("surface lost".to_owned())),
            Err("surface lost".to_owned())
        );
    }

    #[test]
    fn bad_display_falls_back_only_in_auto_mode() {
        let bad_display =
            |_: &SceneContext| gpu_attempt("surface creation", || panic!("BadDisplay"));
        let software = select_renderer(
            RendererPreference::Auto,
            SceneContext {
                home_page: 3,
                ..SceneContext::default()
            },
            bad_display,
        )
        .unwrap();
        assert_eq!(software.backend_label(), "sdf-software");
        assert_eq!(software.context().home_page, 3);
        assert_eq!(
            select_renderer(
                RendererPreference::Wgpu,
                SceneContext::default(),
                bad_display
            )
            .unwrap_err(),
            "surface creation: BadDisplay"
        );
        assert_eq!(
            select_renderer(
                RendererPreference::Software,
                SceneContext::default(),
                |_| { panic!("software must not probe wgpu") }
            )
            .unwrap()
            .backend_label(),
            "sdf-software"
        );
    }
}

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
