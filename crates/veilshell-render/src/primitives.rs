//! Renderer-agnostic 2D glassmorphic drawing primitives (`DrawList`).
//!
//! Both the analytic CPU SDF renderer and the future `wgpu` WGSL shader consume
//! the exact same [`DrawList`] so live frames and headless snapshots match.

use veilshell_core::Rgba;

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
    /// Pixel-aligned bitmap text label.
    Text(TextLabel),
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
}
