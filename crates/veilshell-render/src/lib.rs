//! Rendering scene models and snapshot traits (`wgpu` backend arrives in Phase 1).

use std::fmt;

use veilshell_core::{Orientation, Rgba, ThemePalette};

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

/// Errors returned during rendering or offscreen snapshot capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// Requested viewport had a zero dimension or invalid scale.
    InvalidViewport,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidViewport => {
                write!(f, "viewport width, height, and scale must be positive")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// Trait implemented by rendering backends (`StubRenderer` in Phase 0, `WgpuRenderer` in Phase 1).
pub trait RendererBackend {
    /// Human-readable renderer backend name.
    fn name(&self) -> &'static str;

    /// Render a single deterministic frame of the requested scene.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidViewport`] when dimensions are zero.
    fn render_snapshot(
        &mut self,
        spec: &SnapshotSpec,
        palette: &ThemePalette,
    ) -> Result<RenderFrame, RenderError>;
}

/// Zero-dependency deterministic CPU stub renderer used for Phase 0 tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubRenderer;

impl RendererBackend for StubRenderer {
    fn name(&self) -> &'static str {
        "stub-cpu"
    }

    fn render_snapshot(
        &mut self,
        spec: &SnapshotSpec,
        palette: &ThemePalette,
    ) -> Result<RenderFrame, RenderError> {
        let (w, h) = spec.viewport.physical_pixels();
        if w == 0 || h == 0 || !spec.viewport.scale.is_finite() || spec.viewport.scale <= 0.0 {
            return Err(RenderError::InvalidViewport);
        }

        let bg = to_rgba8(palette.background);
        let accent = to_rgba8(palette.accent);
        let total_pixels = (w as usize) * (h as usize);
        let mut rgba = Vec::with_capacity(total_pixels * 4);

        let pill_top = h.saturating_sub(h / 16).max(1);
        let pill_left = w / 3;
        let pill_right = w.saturating_sub(w / 3);

        for y in 0..h {
            for x in 0..w {
                let in_pill = y >= pill_top && x >= pill_left && x <= pill_right;
                let px = if in_pill { accent } else { bg };
                rgba.extend_from_slice(&px);
            }
        }

        Ok(RenderFrame {
            width: w,
            height: h,
            rgba,
        })
    }
}

fn to_rgba8(c: Rgba) -> [u8; 4] {
    [
        (c.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.a.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_renderer_produces_expected_rgba_dimensions() {
        let mut renderer = StubRenderer;
        let spec = SnapshotSpec {
            scene: SceneKind::Pill,
            viewport: ViewportSize {
                width: 64,
                height: 32,
                scale: 1.0,
            },
            orientation: Orientation::Normal,
        };
        let frame = renderer
            .render_snapshot(&spec, &ThemePalette::default_dark())
            .expect("frame rendered");
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 32);
        assert_eq!(frame.rgba.len(), 64 * 32 * 4);
    }

    #[test]
    fn rejects_zero_viewport() {
        let mut renderer = StubRenderer;
        let spec = SnapshotSpec {
            scene: SceneKind::Home,
            viewport: ViewportSize {
                width: 0,
                height: 100,
                scale: 1.0,
            },
            orientation: Orientation::Normal,
        };
        assert_eq!(
            renderer.render_snapshot(&spec, &ThemePalette::default_dark()),
            Err(RenderError::InvalidViewport)
        );
    }
}
