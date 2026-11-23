//! The optional below-window strip policy. No refresh is driven by input or drawing.

use std::time::{Duration, Instant};

use veilshell_render::SceneContext;

use crate::{LayerRole, LayerSurfaceSpec, EDGE_HANDLE_DEPTH, TABLET_BOTTOM_DEPTH};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const ARM_DEBOUNCE: Duration = Duration::from_millis(100);

pub(super) fn requested(ctx: &SceneContext) -> bool {
    ctx.overlay_gap_gestures && ctx.power_menu.is_none()
}

pub(super) fn output_ready(viewport_ready: bool, output_name: Option<&str>) -> bool {
    viewport_ready && output_name.is_some_and(|name| !name.is_empty())
}

pub(super) fn can_show_foreground(ctx: &SceneContext, output_ready: bool) -> bool {
    !requested(ctx) || output_ready
}

pub(super) fn can_arm(
    ctx: &SceneContext,
    visible: bool,
    output_ready: bool,
    touch_bound: bool,
) -> bool {
    requested(ctx) && visible && output_ready && touch_bound
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GapEdge {
    Top,
    Bottom,
}

impl GapEdge {
    pub(super) const ALL: [Self; 2] = [Self::Top, Self::Bottom];

    pub(super) fn for_role(role: LayerRole) -> Option<Self> {
        match role {
            LayerRole::OverlayTopGap => Some(Self::Top),
            LayerRole::OverlayBottomGap => Some(Self::Bottom),
            _ => None,
        }
    }

    pub(super) fn surface_spec(self, width: u32, depth: u32) -> Option<LayerSurfaceSpec> {
        if width == 0 || depth == 0 {
            return None;
        }
        let role = match self {
            Self::Top => LayerRole::OverlayTopGap,
            Self::Bottom => LayerRole::OverlayBottomGap,
        };
        Some(LayerSurfaceSpec {
            height: depth,
            ..LayerSurfaceSpec::for_role(role, width, 0)
        })
    }
}

pub(super) fn configure_fits(
    (output_w, output_h): (u32, u32),
    depth: u32,
    (width, height): (u32, u32),
) -> bool {
    // With zone zero, the compositor chooses the width in the work area.
    // Do not guess it from the whole output if no usable rect came back.
    (1..=output_w).contains(&width) && (1..=output_h.min(depth)).contains(&height)
}

#[derive(Default)]
pub(super) struct GapRefresh {
    // Do not use the legacy guessed viewport to arm the gap surfaces.
    pub(super) viewport_valid: bool,
    depths: (u32, u32),
    due: Option<Instant>,
    // Keep the rate limit even if the profile or touch seat goes away and returns.
    next_allowed: Option<Instant>,
}

impl GapRefresh {
    pub(super) fn arm(&mut self, now: Instant) {
        if self.due.is_none() {
            // Returning from a foreground panel can reuse only a fresh sample.
            if self.next_allowed.is_none_or(|at| now >= at) {
                self.depths = (0, 0);
            }
            let debounced = now + ARM_DEBOUNCE;
            self.due = Some(self.next_allowed.map_or(debounced, |at| at.max(debounced)));
        }
    }

    pub(super) fn pause(&mut self) {
        self.due = None;
    }

    pub(super) fn disarm(&mut self) {
        self.pause();
        self.depths = (0, 0);
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.due
    }

    pub(super) fn sampled(&mut self, (top, bottom): (u32, u32), height: u32, now: Instant) -> bool {
        self.next_allowed = Some(now + REFRESH_INTERVAL);
        self.due = self.next_allowed;
        // Depths are independent. The backend normally imposes smaller caps;
        // also keep a malformed response or tiny logical viewport bounded.
        let depths = (
            top.min(EDGE_HANDLE_DEPTH).min(height / 2),
            bottom.min(TABLET_BOTTOM_DEPTH).min(height / 2),
        );
        std::mem::replace(&mut self.depths, depths) != depths
    }

    pub(super) fn depth(&self, edge: GapEdge) -> u32 {
        match edge {
            GapEdge::Top => self.depths.0,
            GapEdge::Bottom => self.depths.1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Anchor;

    #[test]
    fn a_named_sized_output_on_a_visible_touch_seat_is_required() {
        let mut ctx = SceneContext::default();
        let output = output_ready(true, Some("eDP-1"));
        assert!(output);
        assert!(!output_ready(false, Some("eDP-1")));
        assert!(!output_ready(true, None));
        assert!(!output_ready(true, Some("")));
        assert!(!can_arm(&ctx, true, output, true));
        assert!(can_show_foreground(&ctx, false)); // legacy unchanged
        ctx.overlay_gap_gestures = true;
        assert!(can_arm(&ctx, true, output, true));
        assert!(!can_arm(&ctx, true, output, false));
        assert!(!can_arm(&ctx, false, output, true));
        assert!(!can_arm(&ctx, true, false, true));
        assert!(can_show_foreground(&ctx, output)); // no touch required for IPC panels
        assert!(!can_show_foreground(&ctx, false)); // named output unplugged
        ctx.power_menu = Some(veilshell_render::power::PowerMenu::Closed);
        assert!(!can_arm(&ctx, true, output, true));
    }

    #[test]
    fn asymmetric_depths_are_separately_non_reserving_and_zero_does_not_map() {
        let top = GapEdge::Top.surface_spec(800, 9).unwrap();
        assert_eq!(top.anchor, Anchor::top_bar());
        assert_eq!((top.width, top.height), (800, 9));
        let bottom = GapEdge::Bottom.surface_spec(1280, 17).unwrap();
        assert_eq!(bottom.anchor, Anchor::bottom_bar());
        assert_eq!((bottom.width, bottom.height), (1280, 17));
        for spec in [top, bottom] {
            assert_eq!(spec.exclusive_zone, 0);
            assert!(!spec.keyboard_interactivity);
        }
        for edge in GapEdge::ALL {
            assert!(edge.surface_spec(800, 0).is_none());
            assert!(edge.surface_spec(0, 9).is_none());
        }
        assert!(configure_fits((800, 1280), 9, (760, 9)));
        assert!(!configure_fits((800, 1280), 9, (0, 9)));
        assert!(!configure_fits((800, 1280), 9, (800, 0)));
        assert!(!configure_fits((800, 1280), 9, (800, 10)));
        assert!(!configure_fits((800, 1280), 9, (801, 9)));
    }

    #[test]
    fn disarming_clears_caps_and_rearming_does_not_bypass_the_query_limit() {
        let mut refresh = GapRefresh::default();
        let start = Instant::now();
        assert_eq!(refresh.deadline(), None);
        refresh.arm(start);
        let first = start + ARM_DEBOUNCE;
        assert_eq!(refresh.deadline(), Some(first));
        assert!(refresh.sampled((9, 17), 800, first));
        assert_eq!(refresh.depth(GapEdge::Top), 9);
        assert_eq!(refresh.depth(GapEdge::Bottom), 17);

        // E.g. touch capability loss followed by a replacement seat.
        refresh.disarm();
        assert_eq!(refresh.deadline(), None);
        assert_eq!(refresh.depth(GapEdge::Top), 0);
        assert_eq!(refresh.depth(GapEdge::Bottom), 0);
        refresh.arm(first);
        assert_eq!(refresh.deadline(), Some(first + REFRESH_INTERVAL));
        assert!(refresh.sampled((0, 14), 800, first + REFRESH_INTERVAL));
        assert_eq!(refresh.depth(GapEdge::Top), 0);
        assert_eq!(refresh.depth(GapEdge::Bottom), 14);

        // An unavailable/unreadable native config fails closed on refresh.
        assert!(refresh.sampled((0, 0), 800, first + REFRESH_INTERVAL * 2));
        assert_eq!(refresh.depth(GapEdge::Bottom), 0);
    }

    #[test]
    fn foreground_pause_reuses_only_a_fresh_sample() {
        let mut refresh = GapRefresh::default();
        let start = Instant::now();
        refresh.sampled((5, 10), 800, start);
        refresh.pause();
        assert_eq!(refresh.deadline(), None);
        refresh.arm(start + Duration::from_millis(500));
        assert_eq!(refresh.depth(GapEdge::Bottom), 10);
        assert_eq!(refresh.deadline(), Some(start + REFRESH_INTERVAL));

        refresh.pause();
        refresh.arm(start + REFRESH_INTERVAL);
        assert_eq!(refresh.depth(GapEdge::Bottom), 0);
        assert_eq!(
            refresh.deadline(),
            Some(start + REFRESH_INTERVAL + ARM_DEBOUNCE)
        );
    }
}
