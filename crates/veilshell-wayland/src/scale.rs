//! Per-surface scale negotiation and logical-to-buffer geometry.

use std::num::NonZeroU32;

use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use veilshell_render::RenderTarget;

use crate::client::LayerApp;

const FRACTIONAL_DENOMINATOR: u64 = 120;

/// The scale preference for a single surface. An integer hint is useful until
/// the compositor sends a fractional hint, but never overrides one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreferredScale {
    Integer(i32),
    Fractional(NonZeroU32),
}

impl PreferredScale {
    fn update_integer(&mut self, factor: i32) -> bool {
        if matches!(*self, Self::Integer(previous) if factor > 0 && previous != factor) {
            *self = Self::Integer(factor);
            true
        } else {
            false
        }
    }

    fn update_fractional(&mut self, units: u32) -> bool {
        let Some(units) = NonZeroU32::new(units) else {
            return false;
        };
        let next = Self::Fractional(units);
        let changed = *self != next;
        *self = next;
        changed
    }

    fn frame(self, logical: (u32, u32)) -> Option<SurfaceFrame> {
        let (numerator, denominator, mapping) = match self {
            Self::Integer(factor) => (
                u64::try_from(factor).ok()?,
                1,
                BufferMapping {
                    scale: factor,
                    destination: None,
                },
            ),
            Self::Fractional(units) => (
                u64::from(units.get()),
                FRACTIONAL_DENOMINATOR,
                BufferMapping {
                    scale: 1,
                    destination: Some((
                        i32::try_from(logical.0).ok()?,
                        i32::try_from(logical.1).ok()?,
                    )),
                },
            ),
        };
        // Round each fractional dimension halfway away from zero, as specified
        // by fractional-scale-v1. Wayland buffer dimensions must fit signed ints.
        let physical = |size| {
            let pixels = (u64::from(size) * numerator + denominator / 2) / denominator;
            i32::try_from(pixels.max(1)).ok()
        };
        let width = physical(logical.0)?;
        let height = physical(logical.1)?;
        let target = RenderTarget::new(logical, (width as u32, height as u32)).ok()?;
        Some(SurfaceFrame { target, mapping })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BufferMapping {
    scale: i32,
    destination: Option<(i32, i32)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SurfaceFrame {
    pub(crate) target: RenderTarget,
    mapping: BufferMapping,
}

/// Optional protocol pair; a fractional preference is only useful with a viewport.
pub(crate) struct FractionalScaleGlobals {
    viewporter: WpViewporter,
    manager: WpFractionalScaleManagerV1,
}

impl FractionalScaleGlobals {
    pub(crate) fn bind(globals: &GlobalList, qh: &QueueHandle<LayerApp>) -> Option<Self> {
        let viewporter: WpViewporter = globals.bind(qh, 1..=1, NoScaleEvents).ok()?;
        let Ok(manager) = globals.bind(qh, 1..=1, NoScaleEvents) else {
            viewporter.destroy();
            return None;
        };
        Some(Self {
            viewporter,
            manager,
        })
    }
}

impl Drop for FractionalScaleGlobals {
    fn drop(&mut self) {
        self.manager.destroy();
        self.viewporter.destroy();
    }
}

struct FractionalSurface {
    preference: WpFractionalScaleV1,
    viewport: WpViewport,
}

impl Drop for FractionalSurface {
    fn drop(&mut self) {
        self.preference.destroy();
        self.viewport.destroy();
    }
}

/// Scale preferences and double-buffered protocol state for one managed surface.
pub(crate) struct SurfaceScaling {
    fractional: Option<FractionalSurface>,
    preferred: PreferredScale,
    applied: BufferMapping,
    supports_buffer_scale: bool,
}

impl SurfaceScaling {
    pub(crate) fn new(
        globals: Option<&FractionalScaleGlobals>,
        qh: &QueueHandle<LayerApp>,
        surface: &WlSurface,
        initial_factor: i32,
    ) -> Self {
        let supports_buffer_scale = surface.version() >= 3;
        let fractional = globals.map(|globals| FractionalSurface {
            preference: globals.manager.get_fractional_scale(
                surface,
                qh,
                FractionalScaleData(surface.clone()),
            ),
            viewport: globals.viewporter.get_viewport(surface, qh, NoScaleEvents),
        });
        Self {
            fractional,
            preferred: PreferredScale::Integer(if supports_buffer_scale {
                initial_factor.max(1)
            } else {
                1
            }),
            applied: BufferMapping {
                scale: 1,
                destination: None,
            },
            supports_buffer_scale,
        }
    }

    pub(crate) fn update_integer(&mut self, factor: i32) -> bool {
        self.supports_buffer_scale && self.preferred.update_integer(factor)
    }

    pub(crate) fn update_fractional(&mut self, units: u32) -> bool {
        self.preferred.update_fractional(units)
    }

    pub(crate) fn frame(&self, logical: (u32, u32)) -> Option<SurfaceFrame> {
        self.preferred.frame(logical)
    }

    /// Queue changes immediately before committing the corresponding buffer.
    pub(crate) fn apply(&mut self, surface: &WlSurface, frame: SurfaceFrame) {
        let mapping = frame.mapping;
        if self.supports_buffer_scale && mapping.scale != self.applied.scale {
            surface.set_buffer_scale(mapping.scale);
        }
        if mapping.destination != self.applied.destination {
            if let Some(fractional) = &self.fractional {
                let (width, height) = mapping.destination.unwrap_or((-1, -1));
                fractional.viewport.set_destination(width, height);
            }
        }
        self.applied = mapping;
    }
}

struct NoScaleEvents;

// Neither manager nor wp_viewport has any events.
impl<P: Proxy> Dispatch2<P, LayerApp> for NoScaleEvents {
    fn event(
        &self,
        _: &mut LayerApp,
        _: &P,
        _: P::Event,
        _: &Connection,
        _: &QueueHandle<LayerApp>,
    ) {
    }
}

struct FractionalScaleData(WlSurface);

impl Dispatch2<WpFractionalScaleV1, LayerApp> for FractionalScaleData {
    fn event(
        &self,
        state: &mut LayerApp,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        _: &QueueHandle<LayerApp>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.fractional_scale_changed(&self.0, scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_scale_tracks_compositor_and_preserves_logical_size() {
        let mut preferred = PreferredScale::Integer(1);
        assert!(preferred.update_integer(2));
        assert!(!preferred.update_integer(2));
        assert!(!preferred.update_integer(0));
        let frame = preferred.frame((301, 27)).unwrap();
        assert_eq!(frame.target.physical_size(), (602, 54));
        assert_eq!(frame.mapping.scale, 2);
        assert_eq!(frame.mapping.destination, None);

        assert!(preferred.update_integer(1));
        let frame = preferred.frame((301, 27)).unwrap();
        assert_eq!(frame.target.physical_size(), (301, 27));
        assert_eq!(frame.mapping.scale, 1);
    }

    #[test]
    fn fractional_hint_has_priority_and_rounds_half_up() {
        let mut preferred = PreferredScale::Integer(2);
        assert!(preferred.update_fractional(180));
        assert!(!preferred.update_integer(3));
        assert!(!preferred.update_fractional(180));
        let frame = preferred.frame((301, 27)).unwrap();
        assert_eq!(frame.target.physical_size(), (452, 41));
        assert_eq!(frame.mapping.scale, 1);
        assert_eq!(frame.mapping.destination, Some((301, 27)));

        assert!(preferred.update_fractional(150));
        assert_eq!(
            preferred.frame((301, 27)).unwrap().target.physical_size(),
            (376, 34)
        );
        assert!(preferred.update_fractional(120));
        assert!(!preferred.update_fractional(0));
        let frame = preferred.frame((301, 27)).unwrap();
        assert_eq!(frame.target.physical_size(), (301, 27));
        assert_eq!(frame.mapping.destination, Some((301, 27)));
    }

    #[test]
    fn fractional_mapping_uses_the_latest_logical_configure() {
        let mut preferred = PreferredScale::Integer(1);
        assert!(preferred.update_fractional(180));
        assert_eq!(
            preferred.frame((960, 640)).unwrap().target.physical_size(),
            (1440, 960)
        );
        let resized = preferred.frame((641, 959)).unwrap();
        assert_eq!(resized.target.physical_size(), (962, 1439));
        assert_eq!(resized.mapping.destination, Some((641, 959)));
    }

    #[test]
    fn buffer_sizes_do_not_wrap_or_round_to_zero() {
        let mut preferred = PreferredScale::Integer(2);
        assert!(preferred.frame((u32::MAX, 27)).is_none());
        assert!(!preferred.update_fractional(0));
        assert!(preferred.update_fractional(40));
        assert!(preferred.frame((0, 1)).is_none());
        assert_eq!(
            preferred.frame((1, 1)).unwrap().target.physical_size(),
            (1, 1)
        );
        assert!(preferred.update_fractional(u32::MAX));
        assert!(preferred.frame((u32::MAX, u32::MAX)).is_none());
    }
}
