//! Rendering scene models, analytic SDF glassmorphic renderer, and `PNG` snapshot export.

pub mod font;
pub mod primitives;
pub mod scene;
pub mod software;

use std::fmt;

use veilshell_core::{Orientation, ThemePalette};

pub use primitives::{DrawCommand, DrawList, RoundedRect, TextAlign, TextLabel};
pub use scene::{build_scene, SceneContext};
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
#[derive(Debug)]
pub enum RenderError {
    /// Requested viewport had a zero dimension or invalid scale.
    InvalidViewport,
    /// Underlying filesystem error while saving a snapshot.
    Io(std::io::Error),
    /// Image encoding error.
    Encode(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidViewport => {
                write!(f, "viewport width, height, and scale must be positive")
            }
            Self::Io(err) => write!(f, "snapshot I/O error: {err}"),
            Self::Encode(msg) => write!(f, "snapshot encode error: {msg}"),
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

/// Trait implemented by rendering backends (`SdfSoftwareRenderer`, and `WgpuRenderer` next).
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
        let _ = std::fs::remove_file(out);
    }
}
