//! Rendering scene models, analytic SDF glassmorphic renderers (`wgpu` GPU + CPU software),
//! pre-blurred static wallpaper glass cache, and `PNG` snapshot export.

pub mod font;
#[cfg(feature = "wgpu")]
pub mod gpu;
pub mod primitives;
pub mod scene;
pub mod software;

use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use veilshell_core::{Orientation, ThemePalette};

#[cfg(feature = "wgpu")]
pub use gpu::WgpuRenderer;
pub use primitives::{
    DrawCommand, DrawList, HitRect, HitRegion, HitTarget, ImageQuad, RgbaImage, RoundedRect,
    SceneOutput, ShmPixelFormat, TextAlign, TextLabel,
};
pub use scene::{
    build_scene, build_scene_layout, build_top_strip_scene, default_demo_apps, home_page_count,
    AppTile, RecentsCard, SceneContext, HOME_APPS_PER_PAGE,
};
pub use software::SdfSoftwareRenderer;

/// High-level shell scene rendered either live on `Wayland` or offscreen to a snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SceneKind {
    Pill,
    Home,
    Recents,
    Shade,
    Osk,
}

impl SceneKind {
    /// Parse a CLI scene identifier (`pill`, `home`, `recents`, `shade`, `osk`).
    #[must_use]
    pub fn parse_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pill" => Some(Self::Pill),
            "home" => Some(Self::Home),
            "recents" => Some(Self::Recents),
            "shade" => Some(Self::Shade),
            "osk" => Some(Self::Osk),
            _ => None,
        }
    }

    /// Canonical lowercase scene name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pill => "pill",
            Self::Home => "home",
            Self::Recents => "recents",
            Self::Shade => "shade",
            Self::Osk => "osk",
        }
    }
}

/// User or runtime preference for selecting the rendering backend.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum RendererPreference {
    /// Prefer `wgpu` Vulkan/GLES when available, falling back seamlessly to `SdfSoftwareRenderer`.
    #[default]
    Auto,
    /// Require the `wgpu` Vulkan/GLES GPU renderer (error if unavailable).
    Wgpu,
    /// Force the zero-`unsafe` CPU analytic SDF rasterizer.
    Software,
}

impl RendererPreference {
    /// Parse a CLI renderer flag (`auto`, `wgpu`, `gpu`, `software`, `cpu`).
    #[must_use]
    pub fn parse_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "wgpu" | "gpu" | "vulkan" => Some(Self::Wgpu),
            "software" | "cpu" | "sdf" => Some(Self::Software),
            _ => None,
        }
    }

    /// Canonical lowercase renderer preference name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Wgpu => "wgpu",
            Self::Software => "software",
        }
    }
}

/// Logical viewport dimensions and scale factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportSize {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

impl ViewportSize {
    /// Compute physical framebuffer dimensions after scaling.
    #[must_use]
    pub fn physical_pixels(self) -> (u32, u32) {
        let scale = self.scale.max(0.25);
        let w = (self.width as f32 * scale).round() as u32;
        let h = (self.height as f32 * scale).round() as u32;
        (w, h)
    }
}

/// Complete specification for an offscreen snapshot render.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapshotSpec {
    pub scene: SceneKind,
    pub viewport: ViewportSize,
    pub orientation: Orientation,
}

/// Raw `RGBA8` pixel buffer returned by a renderer snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Decoded static `RGBA8` wallpaper image fed into the GPU blur-once glass backdrop cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WallpaperImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl WallpaperImage {
    /// Load a `PNG` wallpaper image from disk and normalize it to straight `RGBA8`.
    ///
    /// # Errors
    /// Returns [`RenderError::Io`] or [`RenderError::Encode`] if reading or decoding fails.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, RenderError> {
        let (width, height, rgba) = decode_png_rgba(path)?;
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
}

pub(crate) fn decode_png_rgba(path: impl AsRef<Path>) -> Result<(u32, u32, Vec<u8>), RenderError> {
    let file = File::open(path)?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| RenderError::Encode(format!("png header: {e}")))?;
    let buf_len = reader
        .output_buffer_size()
        .ok_or_else(|| RenderError::Encode("png buffer size overflow".to_owned()))?;
    let mut buf = vec![0_u8; buf_len];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| RenderError::Encode(format!("png decode: {e}")))?;
    if info.width == 0 || info.height == 0 || info.width > 4096 || info.height > 4096 {
        return Err(RenderError::Encode(format!(
            "unsupported PNG dimensions {}x{}",
            info.width, info.height
        )));
    }
    let bytes = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for rgb in bytes.chunks_exact(3) {
                out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for &g in bytes {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for ga in bytes.chunks_exact(2) {
                out.extend_from_slice(&[ga[0], ga[0], ga[0], ga[1]]);
            }
            out
        }
        png::ColorType::Indexed => {
            return Err(RenderError::Encode(
                "indexed PNG could not be expanded".to_owned(),
            ));
        }
    };
    Ok((info.width, info.height, rgba))
}

/// Errors returned during rendering or offscreen snapshot capture.
#[derive(Debug)]
pub enum RenderError {
    /// Requested viewport had a zero dimension or invalid scale.
    InvalidViewport,
    /// Underlying filesystem error while loading or saving an image.
    Io(std::io::Error),
    /// Image encoding or decoding error.
    Encode(String),
    /// GPU (`wgpu`) initialization, surface, or readback error.
    Gpu(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidViewport => {
                write!(f, "viewport width, height, and scale must be positive")
            }
            Self::Io(err) => write!(f, "image I/O error: {err}"),
            Self::Encode(msg) => write!(f, "image codec error: {msg}"),
            Self::Gpu(msg) => write!(f, "wgpu renderer error: {msg}"),
        }
    }
}

impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for RenderError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Trait implemented by rendering backends ([`SdfSoftwareRenderer`] and [`WgpuRenderer`]).
pub trait RendererBackend {
    /// Human-readable renderer backend name.
    fn name(&self) -> &'static str;

    /// Render a single deterministic frame of the requested scene.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidViewport`] when dimensions are zero or [`RenderError::Gpu`]
    /// if GPU execution fails.
    fn render_snapshot(
        &mut self,
        spec: &SnapshotSpec,
        palette: &ThemePalette,
    ) -> Result<RenderFrame, RenderError>;
}

/// Runtime-selected renderer combining GPU (`wgpu`) and CPU (`SdfSoftwareRenderer`) backends.
#[derive(Debug)]
pub enum ActiveRenderer {
    /// Zero-`unsafe` CPU analytic SDF rasterizer.
    Software(SdfSoftwareRenderer),
    /// Hardware-accelerated Vulkan/GLES `wgpu` WGSL SDF renderer.
    #[cfg(feature = "wgpu")]
    Wgpu(Box<WgpuRenderer>),
}

impl ActiveRenderer {
    /// Construct an [`ActiveRenderer`] honoring [`RendererPreference`].
    ///
    /// In [`RendererPreference::Auto`] mode, initializes `WgpuRenderer` when compiled and a
    /// compatible GPU adapter is present, otherwise falls back cleanly to `SdfSoftwareRenderer`.
    ///
    /// # Errors
    /// Returns [`RenderError::Gpu`] only when [`RendererPreference::Wgpu`] is explicitly requested
    /// and cannot be initialized.
    pub fn new(pref: RendererPreference, context: SceneContext) -> Result<Self, RenderError> {
        match pref {
            RendererPreference::Software => {
                Ok(Self::Software(SdfSoftwareRenderer::with_context(context)))
            }
            RendererPreference::Auto => {
                #[cfg(feature = "wgpu")]
                {
                    match WgpuRenderer::with_context(context.clone()) {
                        Ok(gpu) => Ok(Self::Wgpu(Box::new(gpu))),
                        Err(err) => {
                            eprintln!(
                                "veilshell: wgpu unavailable ({err}); falling back to sdf-software"
                            );
                            Ok(Self::Software(SdfSoftwareRenderer::with_context(context)))
                        }
                    }
                }
                #[cfg(not(feature = "wgpu"))]
                {
                    Ok(Self::Software(SdfSoftwareRenderer::with_context(context)))
                }
            }
            RendererPreference::Wgpu => {
                #[cfg(feature = "wgpu")]
                {
                    WgpuRenderer::with_context(context).map(|gpu| Self::Wgpu(Box::new(gpu)))
                }
                #[cfg(not(feature = "wgpu"))]
                {
                    let _ = context;
                    Err(RenderError::Gpu(
                        "veilshell-render was compiled without the `wgpu` feature".to_owned(),
                    ))
                }
            }
        }
    }

    /// Borrow the active [`SceneContext`].
    #[must_use]
    pub const fn context(&self) -> &SceneContext {
        match self {
            Self::Software(sw) => &sw.context,
            #[cfg(feature = "wgpu")]
            Self::Wgpu(gpu) => &gpu.context,
        }
    }

    /// Mutably borrow the active [`SceneContext`].
    pub fn context_mut(&mut self) -> &mut SceneContext {
        match self {
            Self::Software(sw) => &mut sw.context,
            #[cfg(feature = "wgpu")]
            Self::Wgpu(gpu) => &mut gpu.context,
        }
    }

    /// Human-readable description of the active backend (including GPU adapter name when active).
    #[must_use]
    pub fn backend_label(&self) -> String {
        match self {
            Self::Software(_) => "sdf-software".to_owned(),
            #[cfg(feature = "wgpu")]
            Self::Wgpu(gpu) => format!("wgpu-sdf ({})", gpu.adapter_summary()),
        }
    }

    /// Access the underlying [`WgpuRenderer`] when running on the GPU backend.
    #[cfg(feature = "wgpu")]
    #[must_use]
    pub fn as_wgpu(&self) -> Option<&WgpuRenderer> {
        match self {
            Self::Wgpu(gpu) => Some(gpu),
            Self::Software(_) => None,
        }
    }

    /// Mutably access the underlying [`WgpuRenderer`] when running on the GPU backend.
    #[cfg(feature = "wgpu")]
    pub fn as_wgpu_mut(&mut self) -> Option<&mut WgpuRenderer> {
        match self {
            Self::Wgpu(gpu) => Some(gpu),
            Self::Software(_) => None,
        }
    }

    /// Upload and pre-blur a static wallpaper image when backed by `wgpu`.
    ///
    /// # Errors
    /// Returns [`RenderError`] if the image dimensions are invalid.
    #[cfg_attr(not(feature = "wgpu"), allow(unused_variables))]
    pub fn set_wallpaper(&mut self, wallpaper: Option<&WallpaperImage>) -> Result<(), RenderError> {
        match self {
            Self::Software(_) => Ok(()),
            #[cfg(feature = "wgpu")]
            Self::Wgpu(gpu) => {
                if let Some(img) = wallpaper {
                    gpu.set_wallpaper(img)
                } else {
                    gpu.clear_wallpaper();
                    Ok(())
                }
            }
        }
    }

    /// Build scene layout (hit-test regions) and rasterize an offscreen frame.
    ///
    /// # Errors
    /// Returns [`RenderError`] if dimensions are zero or GPU rendering fails.
    pub fn render_interactive(
        &mut self,
        scene: SceneKind,
        width: u32,
        height: u32,
        palette: &ThemePalette,
    ) -> Result<(RenderFrame, SceneOutput), RenderError> {
        match self {
            Self::Software(sw) => sw.render_interactive(scene, width, height, palette),
            #[cfg(feature = "wgpu")]
            Self::Wgpu(gpu) => gpu.render_interactive(scene, width, height, palette),
        }
    }

    /// Render the top pull handle strip offscreen.
    ///
    /// # Errors
    /// Returns [`RenderError`] if dimensions are zero or GPU rendering fails.
    pub fn render_top_strip(
        &mut self,
        width: u32,
        height: u32,
        palette: &ThemePalette,
    ) -> Result<RenderFrame, RenderError> {
        match self {
            Self::Software(sw) => sw.render_top_strip(width, height, palette),
            #[cfg(feature = "wgpu")]
            Self::Wgpu(gpu) => gpu.render_top_strip(width, height, palette),
        }
    }

    /// Rasterize a [`DrawList`] offscreen into an `RGBA8` [`RenderFrame`].
    ///
    /// # Errors
    /// Returns [`RenderError`] if dimensions are zero or GPU rendering fails.
    pub fn rasterize(
        &mut self,
        width: u32,
        height: u32,
        list: &DrawList,
    ) -> Result<RenderFrame, RenderError> {
        match self {
            Self::Software(sw) => sw.rasterize(width, height, list),
            #[cfg(feature = "wgpu")]
            Self::Wgpu(gpu) => gpu.rasterize(width, height, list),
        }
    }
}

impl RendererBackend for ActiveRenderer {
    fn name(&self) -> &'static str {
        match self {
            Self::Software(sw) => sw.name(),
            #[cfg(feature = "wgpu")]
            Self::Wgpu(gpu) => gpu.name(),
        }
    }

    fn render_snapshot(
        &mut self,
        spec: &SnapshotSpec,
        palette: &ThemePalette,
    ) -> Result<RenderFrame, RenderError> {
        match self {
            Self::Software(sw) => sw.render_snapshot(spec, palette),
            #[cfg(feature = "wgpu")]
            Self::Wgpu(gpu) => gpu.render_snapshot(spec, palette),
        }
    }
}

/// Backwards-compatible alias for [`SdfSoftwareRenderer`].
pub type StubRenderer = SdfSoftwareRenderer;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_all_five_scenes_with_non_empty_pixels() {
        let mut renderer = SdfSoftwareRenderer::default();
        let palette = ThemePalette::default_dark();
        for scene in [
            SceneKind::Pill,
            SceneKind::Home,
            SceneKind::Recents,
            SceneKind::Shade,
            SceneKind::Osk,
        ] {
            let spec = SnapshotSpec {
                scene,
                viewport: ViewportSize {
                    width: 320,
                    height: if scene == SceneKind::Pill { 28 } else { 220 },
                    scale: 1.0,
                },
                orientation: Orientation::Normal,
            };
            let frame = renderer
                .render_snapshot(&spec, &palette)
                .expect("scene renders");
            assert_eq!(frame.rgba.len(), (frame.width * frame.height * 4) as usize);
            assert!(
                frame.rgba.chunks_exact(4).any(|px| px[3] > 0),
                "scene {scene:?} produced fully transparent buffer"
            );
        }
    }

    #[test]
    fn rejects_zero_viewport() {
        let mut renderer = SdfSoftwareRenderer::default();
        let spec = SnapshotSpec {
            scene: SceneKind::Home,
            viewport: ViewportSize {
                width: 0,
                height: 100,
                scale: 1.0,
            },
            orientation: Orientation::Normal,
        };
        assert!(matches!(
            renderer.render_snapshot(&spec, &ThemePalette::default_dark()),
            Err(RenderError::InvalidViewport)
        ));
    }

    #[test]
    fn saves_valid_png_file_and_premultiplies_argb() {
        let mut renderer = SdfSoftwareRenderer::default();
        let spec = SnapshotSpec {
            scene: SceneKind::Pill,
            viewport: ViewportSize {
                width: 240,
                height: 28,
                scale: 1.0,
            },
            orientation: Orientation::Normal,
        };
        let frame = renderer
            .render_snapshot(&spec, &ThemePalette::default_dark())
            .expect("pill renders");
        let argb = frame.to_argb8888_premultiplied();
        assert_eq!(argb.len(), frame.rgba.len());

        let out = std::env::temp_dir().join("veilshell_unit_test_pill.png");
        frame.save_png(&out).expect("png saved");
        let bytes = std::fs::read(&out).expect("png readable");
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");

        let reloaded = WallpaperImage::load_png(&out).expect("png reloads cleanly");
        assert_eq!(reloaded.width, 240);
        assert_eq!(reloaded.height, 28);
        assert_eq!(reloaded.rgba, frame.rgba);
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn renderer_preference_parses_aliases() {
        assert_eq!(
            RendererPreference::parse_name("auto"),
            Some(RendererPreference::Auto)
        );
        assert_eq!(
            RendererPreference::parse_name("wgpu"),
            Some(RendererPreference::Wgpu)
        );
        assert_eq!(
            RendererPreference::parse_name("gpu"),
            Some(RendererPreference::Wgpu)
        );
        assert_eq!(
            RendererPreference::parse_name("software"),
            Some(RendererPreference::Software)
        );
        assert_eq!(
            RendererPreference::parse_name("cpu"),
            Some(RendererPreference::Software)
        );
        assert_eq!(RendererPreference::parse_name("unknown"), None);
    }

    #[test]
    fn recents_hit_test_prioritizes_close_badge_over_card_focus() {
        let ctx = SceneContext::default();
        let out = build_scene_layout(
            SceneKind::Recents,
            1280.0,
            853.0,
            &ThemePalette::default_dark(),
            &ctx,
        );
        let close_region = out
            .hit_regions
            .iter()
            .find(|r| matches!(r.target, HitTarget::CloseWindow { .. }))
            .expect("close region exists");
        let cx = close_region.rect.x + close_region.rect.w * 0.5;
        let cy = close_region.rect.y + close_region.rect.h * 0.5;
        assert!(matches!(
            out.hit_test(cx, cy),
            Some(HitTarget::CloseWindow { ref id }) if id == "0x1"
        ));

        let card_region = out
            .hit_regions
            .iter()
            .find(|r| matches!(r.target, HitTarget::FocusWindow { .. }))
            .expect("focus region exists");
        assert!(matches!(
            out.hit_test(
                card_region.rect.x + card_region.rect.w * 0.5,
                card_region.rect.y + card_region.rect.h * 0.5,
            ),
            Some(HitTarget::FocusWindow { ref id }) if id == "0x1"
        ));
    }

    #[test]
    fn home_hit_test_resolves_app_tiles_and_pager_dots() {
        let mut ctx = SceneContext::default();
        let mut extra = ctx.home_apps.clone();
        ctx.home_apps.append(&mut extra);
        let out = build_scene_layout(
            SceneKind::Home,
            1280.0,
            853.0,
            &ThemePalette::default_dark(),
            &ctx,
        );
        let tile = out
            .hit_regions
            .iter()
            .find(|r| matches!(r.target, HitTarget::LaunchApp { index: 0 }))
            .expect("first tile");
        assert_eq!(
            out.hit_test(
                tile.rect.x + tile.rect.w * 0.5,
                tile.rect.y + tile.rect.h * 0.5
            ),
            Some(&HitTarget::LaunchApp { index: 0 })
        );

        let page1 = out
            .hit_regions
            .iter()
            .find(|r| matches!(r.target, HitTarget::HomePage { page: 1 }))
            .expect("page 1 dot");
        assert_eq!(
            out.hit_test(
                page1.rect.x + page1.rect.w * 0.5,
                page1.rect.y + page1.rect.h * 0.5,
            ),
            Some(&HitTarget::HomePage { page: 1 })
        );
    }

    #[test]
    fn shade_hit_test_resolves_toggles_and_sliders() {
        let ctx = SceneContext::default();
        let out = build_scene_layout(
            SceneKind::Shade,
            1280.0,
            853.0,
            &ThemePalette::default_dark(),
            &ctx,
        );
        let wifi = out
            .hit_regions
            .iter()
            .find(|r| matches!(r.target, HitTarget::ShadeToggle { id: "wifi" }))
            .expect("wifi toggle");
        assert_eq!(
            out.hit_test(
                wifi.rect.x + wifi.rect.w * 0.5,
                wifi.rect.y + wifi.rect.h * 0.5
            ),
            Some(&HitTarget::ShadeToggle { id: "wifi" })
        );

        let brightness = out
            .hit_regions
            .iter()
            .find(|r| {
                matches!(
                    r.target,
                    HitTarget::ShadeSlider {
                        id: "brightness",
                        ..
                    }
                )
            })
            .expect("brightness slider");
        assert!(matches!(
            out.hit_test(
                brightness.rect.x + brightness.rect.w * 0.5,
                brightness.rect.y + brightness.rect.h * 0.5,
            ),
            Some(HitTarget::ShadeSlider {
                id: "brightness",
                ..
            })
        ));
    }

    #[cfg(feature = "wgpu")]
    #[test]
    fn wgpu_matches_software_sdf_pixel_parity() {
        use std::sync::Arc;

        let Ok(mut gpu) = WgpuRenderer::new() else {
            eprintln!("skipping wgpu GPU parity test: no Vulkan/GLES adapter available");
            return;
        };
        let mut sw = SdfSoftwareRenderer::default();
        let palette = ThemePalette::default_dark();

        let icon_w = 32_u32;
        let icon_h = 32_u32;
        let mut icon_rgba = vec![0_u8; (icon_w * icon_h * 4) as usize];
        for y in 0..icon_h {
            for x in 0..icon_w {
                let idx = ((y * icon_w + x) * 4) as usize;
                icon_rgba[idx] = ((x * 255) / (icon_w - 1)) as u8;
                icon_rgba[idx + 1] = ((y * 255) / (icon_h - 1)) as u8;
                icon_rgba[idx + 2] = 210;
                icon_rgba[idx + 3] = if x < 4 || x >= icon_w - 4 { 180 } else { 255 };
            }
        }
        let icon = Arc::new(RgbaImage::new(icon_w, icon_h, icon_rgba).expect("icon"));

        let thumb_w = 64_u32;
        let thumb_h = 44_u32;
        let mut thumb_rgba = vec![0_u8; (thumb_w * thumb_h * 4) as usize];
        for y in 0..thumb_h {
            for x in 0..thumb_w {
                let idx = ((y * thumb_w + x) * 4) as usize;
                thumb_rgba[idx] = if (x / 8 + y / 8) % 2 == 0 { 65 } else { 190 };
                thumb_rgba[idx + 1] = ((y * 255) / (thumb_h - 1)) as u8;
                thumb_rgba[idx + 2] = 235;
                thumb_rgba[idx + 3] = 255;
            }
        }
        let thumb = Arc::new(RgbaImage::new(thumb_w, thumb_h, thumb_rgba).expect("thumb"));

        sw.context.home_apps[0].icon = Some(Arc::clone(&icon));
        sw.context.recents_cards[0].thumbnail = Some(Arc::clone(&thumb));
        gpu.context.home_apps[0].icon = Some(icon);
        gpu.context.recents_cards[0].thumbnail = Some(thumb);

        for scene in [
            SceneKind::Pill,
            SceneKind::Home,
            SceneKind::Recents,
            SceneKind::Shade,
            SceneKind::Osk,
        ] {
            let spec = SnapshotSpec {
                scene,
                viewport: ViewportSize {
                    width: 480,
                    height: if scene == SceneKind::Pill { 28 } else { 320 },
                    scale: 1.0,
                },
                orientation: Orientation::Normal,
            };
            let sw_frame = sw.render_snapshot(&spec, &palette).expect("sw renders");
            let gpu_frame = gpu.render_snapshot(&spec, &palette).expect("gpu renders");
            assert_eq!(sw_frame.width, gpu_frame.width);
            assert_eq!(sw_frame.height, gpu_frame.height);
            assert_eq!(sw_frame.rgba.len(), gpu_frame.rgba.len());

            // Compare in premultiplied channel space so transparent RGB noise is weighted by alpha.
            let mut sum_abs_diff = 0_u64;
            let pixel_count = u64::from(sw_frame.width * sw_frame.height);
            for (s, g) in sw_frame
                .rgba
                .chunks_exact(4)
                .zip(gpu_frame.rgba.chunks_exact(4))
            {
                let sa = u32::from(s[3]);
                let ga = u32::from(g[3]);
                sum_abs_diff += u64::from(sa.abs_diff(ga));
                for c in 0..3 {
                    let sp = (u32::from(s[c]) * sa + 127) / 255;
                    let gp = (u32::from(g[c]) * ga + 127) / 255;
                    sum_abs_diff += u64::from(sp.abs_diff(gp));
                }
            }
            let mean_diff = sum_abs_diff as f64 / (pixel_count * 4) as f64;
            assert!(
                mean_diff < 2.0,
                "scene {scene:?} GPU vs CPU mean channel diff too high: {mean_diff:.3} (adapter: {})",
                gpu.adapter_summary()
            );
        }
    }

    #[cfg(feature = "wgpu")]
    #[test]
    fn wgpu_preblurred_wallpaper_applies_to_backdrop_only() {
        let Ok(mut gpu) = WgpuRenderer::new() else {
            eprintln!("skipping wgpu wallpaper test: no Vulkan/GLES adapter available");
            return;
        };
        let palette = ThemePalette::default_dark();

        let wp_w = 128_u32;
        let wp_h = 96_u32;
        let mut wp_rgba = vec![0_u8; (wp_w * wp_h * 4) as usize];
        for y in 0..wp_h {
            for x in 0..wp_w {
                let idx = ((y * wp_w + x) * 4) as usize;
                wp_rgba[idx] = if x < wp_w / 2 { 255 } else { 40 };
                wp_rgba[idx + 1] = if y < wp_h / 2 { 180 } else { 240 };
                wp_rgba[idx + 2] = 220;
                wp_rgba[idx + 3] = 255;
            }
        }
        let home_spec = SnapshotSpec {
            scene: SceneKind::Home,
            viewport: ViewportSize {
                width: 320,
                height: 220,
                scale: 1.0,
            },
            orientation: Orientation::Normal,
        };
        let plain_home = gpu
            .render_snapshot(&home_spec, &palette)
            .expect("plain home");

        gpu.set_wallpaper(&WallpaperImage {
            width: wp_w,
            height: wp_h,
            rgba: wp_rgba,
        })
        .expect("wallpaper blur succeeds");
        assert!(gpu.has_blurred_wallpaper());

        let frosted_home = gpu
            .render_snapshot(&home_spec, &palette)
            .expect("frosted home");
        assert_ne!(
            plain_home.rgba, frosted_home.rgba,
            "pre-blurred wallpaper should tint the overlay backdrop"
        );

        let pill_spec = SnapshotSpec {
            scene: SceneKind::Pill,
            viewport: ViewportSize {
                width: 320,
                height: 28,
                scale: 1.0,
            },
            orientation: Orientation::Normal,
        };
        let pill = gpu.render_snapshot(&pill_spec, &palette).expect("pill");
        assert_eq!(
            pill.rgba[3], 0,
            "pill surface corners must remain transparent even when wallpaper is cached"
        );

        gpu.clear_wallpaper();
        assert!(!gpu.has_blurred_wallpaper());
    }

    #[test]
    fn osk_hit_test_resolves_keys_across_all_layers() {
        for (layer, expected_id) in [
            (veilshell_core::OskLayer::Letters, "q"),
            (veilshell_core::OskLayer::Numbers, "@"),
            (veilshell_core::OskLayer::Symbols, "~"),
        ] {
            let mut ctx = SceneContext::default();
            ctx.osk_state.layer = layer;
            let out = build_scene_layout(
                SceneKind::Osk,
                1280.0,
                284.0,
                &ThemePalette::default_dark(),
                &ctx,
            );
            let region = out
                .hit_regions
                .iter()
                .find(|r| matches!(r.target, HitTarget::OskKey { label } if label == expected_id))
                .expect("expected OSK key hit region");
            assert_eq!(
                out.hit_test(
                    region.rect.x + region.rect.w * 0.5,
                    region.rect.y + region.rect.h * 0.5,
                ),
                Some(&HitTarget::OskKey { label: expected_id })
            );
        }
    }

    #[test]
    fn rgba_image_resize_and_shm_conversion() {
        // Xrgb8888 little-endian [B, G, R, X] -> straight RGBA [R, G, B, 255]
        let xrgb_bytes = [
            10, 20, 30, 0, // row 0 col 0 -> RGB(30, 20, 10)
            40, 50, 60, 0, // row 0 col 1 -> RGB(60, 50, 40)
            70, 80, 90, 0, // row 1 col 0 -> RGB(90, 80, 70)
            100, 110, 120, 0, // row 1 col 1 -> RGB(120, 110, 100)
        ];
        let flipped =
            RgbaImage::from_wayland_shm(2, 2, 8, &xrgb_bytes, ShmPixelFormat::Xrgb8888, true)
                .expect("xrgb flipped converts");
        assert_eq!(&flipped.rgba[0..4], &[90, 80, 70, 255]);
        assert_eq!(&flipped.rgba[8..12], &[30, 20, 10, 255]);

        // Argb8888 premultiplied [B, G, R, A] where A=128 (~0.5) -> straight RGB doubled cleanly
        let argb_bytes = [64, 32, 16, 128];
        let straight =
            RgbaImage::from_wayland_shm(1, 1, 4, &argb_bytes, ShmPixelFormat::Argb8888, false)
                .expect("argb unpremultiplies");
        assert_eq!(straight.rgba[3], 128);
        assert!(straight.rgba[0] >= 31 && straight.rgba[0] <= 33);
        assert!(straight.rgba[1] >= 63 && straight.rgba[1] <= 65);
        assert!(straight.rgba[2] >= 127 && straight.rgba[2] <= 129);

        let large = RgbaImage::new(200, 100, vec![200; 200 * 100 * 4]).expect("valid image");
        let shrunk = large.resize_fit(50, 50);
        assert_eq!(shrunk.width, 50);
        assert_eq!(shrunk.height, 25);
        assert_ne!(large.id(), shrunk.id());
    }

    #[test]
    fn truetype_font_layouts_and_renders_crisp_glyphs() {
        let label = TextLabel {
            x: 10.0,
            y: 10.0,
            text: "VeilShell".to_owned(),
            scale: 2,
            color: veilshell_core::Rgba::from_rgb_u8(255, 255, 255),
            align: TextAlign::Left,
        };
        if font::has_system_font() {
            match font::layout_text_glyphs(&label) {
                font::TextLayout::TrueType(glyphs) => {
                    assert!(!glyphs.is_empty(), "expected rasterized TrueType glyphs");
                    assert!(glyphs
                        .iter()
                        .any(|g| g.glyph.alpha.iter().any(|&a| a > 0 && a < 255)));
                }
                font::TextLayout::Bitmap5x7 => panic!("expected TrueType layout when font exists"),
            }
        }
        let mut list = DrawList::default();
        list.commands.push(DrawCommand::Text(label));
        let sw = SdfSoftwareRenderer::default();
        let frame = sw.rasterize(180, 48, &list).expect("text rasterizes");
        assert!(frame.rgba.chunks_exact(4).any(|px| px[3] > 0));
    }
}
