//! `Wayland` `wlr-layer-shell` surface descriptors, touch models, and multi-surface daemon runtime.

mod capture;
pub mod client;
#[cfg(feature = "wgpu")]
mod gpu;
mod motion;
mod osk;
mod scale;

pub use client::{
    run_layer_surface, run_shell_daemon, DaemonCommand, RuntimeStatus, ShadeReply, ShellAction,
    ShellActionHandler, WaylandError,
};
pub use smithay_client_toolkit::reexports::calloop::channel;

/// Logical role of a `VeilShell` `wlr-layer-shell` surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LayerRole {
    /// Thin bottom gesture indicator bar.
    BottomPill,
    /// Subtle top-center pull handle for Quick Settings.
    TopStrip,
    /// Full-screen spring-animated app launcher grid.
    HomeGrid,
    /// Horizontal recents card switcher.
    RecentsDeck,
    /// Top-pull quick settings and status shade.
    QuickSettings,
    /// Touch on-screen keyboard overlay.
    TouchOsk,
}

impl LayerRole {
    /// Map a [`veilshell_render::SceneKind`] to its corresponding `wlr-layer-shell` [`LayerRole`].
    #[must_use]
    pub const fn from_scene(scene: veilshell_render::SceneKind) -> Self {
        match scene {
            veilshell_render::SceneKind::Pill => Self::BottomPill,
            veilshell_render::SceneKind::Home => Self::HomeGrid,
            veilshell_render::SceneKind::Recents => Self::RecentsDeck,
            veilshell_render::SceneKind::Shade => Self::QuickSettings,
            veilshell_render::SceneKind::Osk => Self::TouchOsk,
        }
    }

    /// Map this [`LayerRole`] back to its rendered [`veilshell_render::SceneKind`].
    ///
    /// [`LayerRole::TopStrip`] maps to [`veilshell_render::SceneKind::Pill`] as a passive bar role.
    #[must_use]
    pub const fn to_scene(self) -> veilshell_render::SceneKind {
        match self {
            Self::BottomPill | Self::TopStrip => veilshell_render::SceneKind::Pill,
            Self::HomeGrid => veilshell_render::SceneKind::Home,
            Self::RecentsDeck => veilshell_render::SceneKind::Recents,
            Self::QuickSettings => veilshell_render::SceneKind::Shade,
            Self::TouchOsk => veilshell_render::SceneKind::Osk,
        }
    }
}

/// Edge anchoring flags matching `zwlr_layer_surface_v1`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Anchor {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Anchor {
    /// Anchor to all four edges (full-screen overlay).
    #[must_use]
    pub const fn all() -> Self {
        Self {
            top: true,
            bottom: true,
            left: true,
            right: true,
        }
    }

    /// Anchor to the bottom edge across the full width.
    #[must_use]
    pub const fn bottom_bar() -> Self {
        Self {
            top: false,
            bottom: true,
            left: true,
            right: true,
        }
    }

    /// Anchor to the top edge centered horizontally.
    #[must_use]
    pub const fn top_center() -> Self {
        Self {
            top: true,
            bottom: false,
            left: false,
            right: false,
        }
    }
}

/// Declarative geometry and interactivity spec for a layer surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayerSurfaceSpec {
    pub role: LayerRole,
    pub anchor: Anchor,
    pub width: u32,
    pub height: u32,
    pub exclusive_zone: i32,
    pub keyboard_interactivity: bool,
}

/// Logical depth of the top and bottom gesture zones.
pub const EDGE_HANDLE_DEPTH: u32 = 48;

impl LayerSurfaceSpec {
    /// Compute the default surface specification for a given [`LayerRole`] and logical viewport.
    #[must_use]
    pub fn for_role(role: LayerRole, viewport_w: u32, viewport_h: u32) -> Self {
        match role {
            LayerRole::BottomPill => Self {
                role,
                anchor: Anchor::bottom_bar(),
                width: viewport_w,
                height: EDGE_HANDLE_DEPTH,
                exclusive_zone: 0,
                keyboard_interactivity: false,
            },
            LayerRole::TopStrip => Self {
                role,
                anchor: Anchor::top_center(),
                width: 220,
                height: EDGE_HANDLE_DEPTH,
                exclusive_zone: 0,
                keyboard_interactivity: false,
            },
            LayerRole::TouchOsk => {
                let height = viewport_h / 3;
                Self {
                    role,
                    anchor: Anchor::bottom_bar(),
                    width: viewport_w,
                    height,
                    exclusive_zone: i32::try_from(height).unwrap_or(i32::MAX),
                    keyboard_interactivity: false,
                }
            }
            LayerRole::HomeGrid | LayerRole::RecentsDeck | LayerRole::QuickSettings => Self {
                role,
                anchor: Anchor::all(),
                width: viewport_w,
                height: viewport_h,
                exclusive_zone: -1,
                keyboard_interactivity: true,
            },
        }
    }
}

/// Lifecycle phase of a multi-touch contact point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}

/// Normalized touch event delivered from `wl_touch`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchEvent {
    pub id: i32,
    pub phase: TouchPhase,
    pub x: f32,
    pub y: f32,
    pub timestamp_ms: u32,
}

/// Runtime options for connecting to a `Wayland` display server.
#[derive(Clone, Debug, PartialEq)]
pub struct WaylandRuntimeConfig {
    pub display_override: Option<String>,
    pub output_name: Option<String>,
    /// Hardware profile hint retained for compatibility. The compositor's
    /// per-surface scale governs live Wayland buffers.
    pub preferred_scale: f32,
    pub exit_after_ms: Option<u64>,
    pub initial_visible: bool,
    pub renderer: veilshell_render::RendererPreference,
    pub wallpaper: Option<veilshell_render::WallpaperImage>,
}

impl Default for WaylandRuntimeConfig {
    fn default() -> Self {
        Self {
            display_override: None,
            output_name: None,
            preferred_scale: 1.0,
            exit_after_ms: None,
            initial_visible: true,
            renderer: veilshell_render::RendererPreference::Auto,
            wallpaper: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_pill_is_non_exclusive_bottom_bar() {
        let spec = LayerSurfaceSpec::for_role(LayerRole::BottomPill, 1280, 853);
        assert_eq!(spec.anchor, Anchor::bottom_bar());
        assert_eq!(spec.height, 48);
        assert_eq!(spec.exclusive_zone, 0);
        assert!(!spec.keyboard_interactivity);
    }

    #[test]
    fn top_strip_is_compact_centered_handle() {
        let spec = LayerSurfaceSpec::for_role(LayerRole::TopStrip, 1280, 853);
        assert_eq!(spec.anchor, Anchor::top_center());
        assert_eq!(spec.width, 220);
        assert_eq!(spec.height, 48);
        assert_eq!(spec.exclusive_zone, 0);
        assert!(!spec.keyboard_interactivity);
    }

    #[test]
    fn quick_settings_covers_full_viewport() {
        let spec = LayerSurfaceSpec::for_role(LayerRole::QuickSettings, 1280, 853);
        assert_eq!(spec.anchor, Anchor::all());
        assert_eq!(spec.width, 1280);
        assert_eq!(spec.height, 853);
        assert!(spec.keyboard_interactivity);
    }
}
