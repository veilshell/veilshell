//! Renderer-agnostic 2D glassmorphic drawing primitives (`DrawList`) and hit-testing regions.
//!
//! Both the analytic CPU SDF renderer and the `wgpu` WGSL shader consume
//! the exact same [`DrawList`] so live frames and headless snapshots match.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use veilshell_core::Rgba;

static NEXT_IMAGE_ID: AtomicU64 = AtomicU64::new(1);

fn next_image_id() -> u64 {
    NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Supported little-endian `Wayland` `wl_shm` 32-bit pixel formats for window thumbnail capture.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ShmPixelFormat {
    /// Premultiplied `[B, G, R, A]` bytes.
    Argb8888,
    /// Opaque `[B, G, R, X]` bytes.
    Xrgb8888,
    /// Premultiplied `[R, G, B, A]` bytes.
    Abgr8888,
    /// Opaque `[R, G, B, X]` bytes.
    Xbgr8888,
}

/// Decoded straight `RGBA8` image asset with a unique monotonic identity for GPU atlas caching.
#[derive(Clone, Debug)]
pub struct RgbaImage {
    id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl PartialEq for RgbaImage {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height && self.rgba == other.rgba
    }
}

impl Eq for RgbaImage {}

impl RgbaImage {
    /// Construct a validated straight `RGBA8` image.
    #[must_use]
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let expected = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(4)?;
        if width == 0 || height == 0 || rgba.len() != expected {
            return None;
        }
        Some(Self {
            id: next_image_id(),
            width,
            height,
            rgba,
        })
    }

    /// Unique monotonically increasing image identity (never reused within a process).
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Load a `PNG` icon or image from disk and normalize it to straight `RGBA8`.
    ///
    /// # Errors
    /// Returns [`crate::RenderError`] if reading, decoding, or dimension validation fails.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, crate::RenderError> {
        let (width, height, rgba) = crate::decode_png_rgba(path)?;
        Self::new(width, height, rgba)
            .ok_or_else(|| crate::RenderError::Encode("invalid PNG image dimensions".to_owned()))
    }

    /// Resize this image to fit inside `(max_w, max_h)` while preserving aspect ratio.
    ///
    /// Uses premultiplied area-box averaging when downscaling (avoiding dark fringes on icons)
    /// and returns an exact copy when the image already fits.
    #[must_use]
    pub fn resize_fit(&self, max_w: u32, max_h: u32) -> Self {
        let max_w = max_w.max(1);
        let max_h = max_h.max(1);
        if self.width <= max_w && self.height <= max_h {
            return Self {
                id: next_image_id(),
                width: self.width,
                height: self.height,
                rgba: self.rgba.clone(),
            };
        }

        let scale = (max_w as f32 / self.width as f32).min(max_h as f32 / self.height as f32);
        let dst_w = ((self.width as f32 * scale).round() as u32).clamp(1, max_w);
        let dst_h = ((self.height as f32 * scale).round() as u32).clamp(1, max_h);

        let src_w = self.width as usize;
        let src_h = self.height as usize;
        let out_w = dst_w as usize;
        let out_h = dst_h as usize;
        let mut out = vec![0_u8; out_w * out_h * 4];

        let scale_x = self.width as f32 / dst_w as f32;
        let scale_y = self.height as f32 / dst_h as f32;

        for dy in 0..out_h {
            let sy0 = ((dy as f32 * scale_y).floor() as usize).min(src_h - 1);
            let sy1 = (((dy + 1) as f32 * scale_y).ceil() as usize).clamp(sy0 + 1, src_h);
            for dx in 0..out_w {
                let sx0 = ((dx as f32 * scale_x).floor() as usize).min(src_w - 1);
                let sx1 = (((dx + 1) as f32 * scale_x).ceil() as usize).clamp(sx0 + 1, src_w);

                let mut sum_r = 0.0_f32;
                let mut sum_g = 0.0_f32;
                let mut sum_b = 0.0_f32;
                let mut sum_a = 0.0_f32;
                let mut count = 0_u32;

                for sy in sy0..sy1 {
                    let row_off = sy * src_w;
                    for sx in sx0..sx1 {
                        let off = (row_off + sx) * 4;
                        let a = f32::from(self.rgba[off + 3]) / 255.0;
                        sum_r += (f32::from(self.rgba[off]) / 255.0) * a;
                        sum_g += (f32::from(self.rgba[off + 1]) / 255.0) * a;
                        sum_b += (f32::from(self.rgba[off + 2]) / 255.0) * a;
                        sum_a += a;
                        count += 1;
                    }
                }

                let inv = 1.0 / (count.max(1) as f32);
                let avg_a = (sum_a * inv).clamp(0.0, 1.0);
                let dst_off = (dy * out_w + dx) * 4;
                if avg_a <= 0.0005 {
                    out[dst_off..dst_off + 4].fill(0);
                } else {
                    let r = ((sum_r * inv) / avg_a).clamp(0.0, 1.0);
                    let g = ((sum_g * inv) / avg_a).clamp(0.0, 1.0);
                    let b = ((sum_b * inv) / avg_a).clamp(0.0, 1.0);
                    out[dst_off] = (r * 255.0).round() as u8;
                    out[dst_off + 1] = (g * 255.0).round() as u8;
                    out[dst_off + 2] = (b * 255.0).round() as u8;
                    out[dst_off + 3] = (avg_a * 255.0).round() as u8;
                }
            }
        }

        Self {
            id: next_image_id(),
            width: dst_w,
            height: dst_h,
            rgba: out,
        }
    }

    /// Convert a little-endian `Wayland` `wl_shm` pixel buffer into straight `RGBA8`.
    #[must_use]
    pub fn from_wayland_shm(
        width: u32,
        height: u32,
        stride: usize,
        bytes: &[u8],
        format: ShmPixelFormat,
        flip_y: bool,
    ) -> Option<Self> {
        let w = usize::try_from(width).ok()?;
        let h = usize::try_from(height).ok()?;
        let row_bytes = w.checked_mul(4)?;
        if width == 0 || height == 0 || stride < row_bytes {
            return None;
        }
        let min_len = h
            .checked_sub(1)?
            .checked_mul(stride)?
            .checked_add(row_bytes)?;
        if bytes.len() < min_len {
            return None;
        }

        let mut rgba = vec![0_u8; w * h * 4];
        for y in 0..h {
            let src_y = if flip_y { h - 1 - y } else { y };
            let src_row = &bytes[src_y * stride..src_y * stride + row_bytes];
            let dst_row = &mut rgba[y * row_bytes..(y + 1) * row_bytes];
            for (sp, dp) in src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4)) {
                match format {
                    ShmPixelFormat::Xrgb8888 => {
                        dp[0] = sp[2];
                        dp[1] = sp[1];
                        dp[2] = sp[0];
                        dp[3] = 255;
                    }
                    ShmPixelFormat::Argb8888 => unpremultiply_pixel(sp[2], sp[1], sp[0], sp[3], dp),
                    ShmPixelFormat::Xbgr8888 => {
                        dp[0] = sp[0];
                        dp[1] = sp[1];
                        dp[2] = sp[2];
                        dp[3] = 255;
                    }
                    ShmPixelFormat::Abgr8888 => unpremultiply_pixel(sp[0], sp[1], sp[2], sp[3], dp),
                }
            }
        }

        Some(Self {
            id: next_image_id(),
            width,
            height,
            rgba,
        })
    }
}

#[inline]
fn unpremultiply_pixel(r: u8, g: u8, b: u8, a: u8, dst: &mut [u8]) {
    if a == 0 {
        dst.fill(0);
    } else if a == 255 {
        dst[0] = r;
        dst[1] = g;
        dst[2] = b;
        dst[3] = 255;
    } else {
        let a_u16 = u16::from(a);
        let half = a_u16 / 2;
        dst[0] = ((u16::from(r) * 255 + half) / a_u16).min(255) as u8;
        dst[1] = ((u16::from(g) * 255 + half) / a_u16).min(255) as u8;
        dst[2] = ((u16::from(b) * 255 + half) / a_u16).min(255) as u8;
        dst[3] = a;
    }
}

/// Textured rounded-corner image quad primitive for app icons and live window thumbnails.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    pub alpha: f32,
    pub image: Arc<RgbaImage>,
}

/// Axis-aligned logical-pixel hit-test rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl HitRect {
    /// Construct a new [`HitRect`].
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Return `true` if logical coordinate `(px, py)` falls inside this rectangle.
    #[must_use]
    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

/// Interactive action associated with a clickable/tappable UI region.
#[derive(Clone, Debug, PartialEq)]
pub enum HitTarget {
    /// Tapping empty backdrop area closes the active full-screen overlay.
    DismissOverlay,
    /// Tapping an application icon launches `home_apps[index]`.
    LaunchApp { index: usize },
    /// Tapping a pagination dot switches the Home grid page.
    HomePage { page: usize },
    /// Tapping a recents card focuses the window with identifier `id`.
    FocusWindow { id: String },
    /// Tapping a Quick Settings tile toggles `id` (`wifi`, `bluetooth`, etc.).
    ShadeToggle { id: &'static str },
    /// Tapping or dragging inside a Quick Settings slider updates `id`.
    ShadeSlider { id: &'static str, rect: HitRect },
    /// Standalone session power menu input.
    Power(crate::power::PowerInput),
    /// Non-interactive region that consumes taps inside a power dialog.
    PowerPanel,
    /// Tapping an OSK key emits the labeled key press.
    OskKey { label: &'static str },
    /// Bottom gesture pill region.
    BottomPill,
}

/// Positioned hit-test entry emitted back-to-front during scene layout.
#[derive(Clone, Debug, PartialEq)]
pub struct HitRegion {
    pub rect: HitRect,
    pub target: HitTarget,
}

/// Complete scene output combining draw commands with interactive hit-test regions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneOutput {
    pub draw_list: DrawList,
    pub hit_regions: Vec<HitRegion>,
}

impl SceneOutput {
    /// Register an interactive hit-test region (later regions take precedence).
    pub fn push_hit(&mut self, rect: HitRect, target: HitTarget) {
        self.hit_regions.push(HitRegion { rect, target });
    }

    /// Resolve the topmost [`HitTarget`] containing `(x, y)`.
    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> Option<&HitTarget> {
        self.hit_regions
            .iter()
            .rev()
            .find(|region| region.rect.contains(x, y))
            .map(|region| &region.target)
    }
}

/// Styling parameters for an antialiased rounded rectangle primitive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    pub fill: Rgba,
    pub border_color: Rgba,
    pub border_width: f32,
    pub shadow_radius: f32,
    pub shadow_offset_y: f32,
    pub shadow_alpha: f32,
    pub top_highlight_alpha: f32,
}

impl RoundedRect {
    /// Create a simple filled rounded rectangle without border or shadow.
    #[must_use]
    pub const fn filled(x: f32, y: f32, w: f32, h: f32, radius: f32, fill: Rgba) -> Self {
        Self {
            x,
            y,
            w,
            h,
            radius,
            fill,
            border_color: Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.0,
            },
            border_width: 0.0,
            shadow_radius: 0.0,
            shadow_offset_y: 0.0,
            shadow_alpha: 0.0,
            top_highlight_alpha: 0.0,
        }
    }

    /// Add a glass rim border and subtle top specular highlight.
    #[must_use]
    pub const fn with_glass_border(
        mut self,
        color: Rgba,
        width: f32,
        highlight_alpha: f32,
    ) -> Self {
        self.border_color = color;
        self.border_width = width;
        self.top_highlight_alpha = highlight_alpha;
        self
    }

    /// Add a soft drop shadow below the shape.
    #[must_use]
    pub const fn with_shadow(mut self, radius: f32, offset_y: f32, alpha: f32) -> Self {
        self.shadow_radius = radius;
        self.shadow_offset_y = offset_y;
        self.shadow_alpha = alpha;
        self
    }
}

/// Horizontal text alignment relative to `(x, y)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Crisp bitmap text label primitive.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLabel {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub scale: u32,
    pub color: Rgba,
    pub align: TextAlign,
}

/// Single command in a scene's ordered back-to-front draw list.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCommand {
    /// Vertical linear gradient covering the entire viewport.
    BackdropGradient { top: Rgba, bottom: Rgba },
    /// Soft radial accent glow behind glass panels.
    RadialGlow {
        cx: f32,
        cy: f32,
        radius: f32,
        color: Rgba,
    },
    /// Analytic SDF rounded rectangle with optional shadow and glass rim.
    Rect(RoundedRect),
    /// Pixel-aligned bitmap or TrueType text label.
    Text(TextLabel),
    /// Rounded-corner textured RGBA image quad (app icon or live thumbnail).
    Image(ImageQuad),
}

/// Ordered draw list produced by scene layout and consumed by a renderer backend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    pub commands: Vec<DrawCommand>,
}

impl DrawList {
    /// Append a vertical backdrop gradient.
    pub fn push_backdrop(&mut self, top: Rgba, bottom: Rgba) {
        self.commands
            .push(DrawCommand::BackdropGradient { top, bottom });
    }

    /// Append a radial accent glow.
    pub fn push_glow(&mut self, cx: f32, cy: f32, radius: f32, color: Rgba) {
        self.commands.push(DrawCommand::RadialGlow {
            cx,
            cy,
            radius,
            color,
        });
    }

    /// Append a rounded rectangle primitive.
    pub fn push_rect(&mut self, rect: RoundedRect) {
        self.commands.push(DrawCommand::Rect(rect));
    }

    /// Append a text label primitive.
    pub fn push_text(
        &mut self,
        x: f32,
        y: f32,
        text: impl Into<String>,
        scale: u32,
        color: Rgba,
        align: TextAlign,
    ) {
        self.commands.push(DrawCommand::Text(TextLabel {
            x,
            y,
            text: text.into(),
            scale: scale.max(1),
            color,
            align,
        }));
    }

    /// Append a textured rounded-corner image quad primitive.
    pub fn push_image(&mut self, quad: ImageQuad) {
        self.commands.push(DrawCommand::Image(quad));
    }
}
