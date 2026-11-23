//! Standalone Tablet navigation. The legacy overlay gesture classifier is separate.

use veilshell_render::{SceneKind, TabletOverviewOrigin};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NavOrigin {
    App,
    Home,
    Recents,
}

impl NavOrigin {
    pub(super) const fn for_scene(scene: SceneKind) -> Option<Self> {
        match scene {
            SceneKind::Pill => Some(Self::App),
            SceneKind::Home => Some(Self::Home),
            SceneKind::Recents => Some(Self::Recents),
            _ => None,
        }
    }

    pub(super) const fn overview(self) -> Option<TabletOverviewOrigin> {
        match self {
            Self::App => Some(TabletOverviewOrigin::App),
            Self::Home => Some(TabletOverviewOrigin::Home),
            Self::Recents => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Axis {
    Pending,
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NavRelease {
    Cancel,
    Home,
    Overview,
    DismissOverview,
    Older,
    Newer,
}

pub(super) struct BottomContact {
    pub(super) origin: NavOrigin,
    axis: Axis,
    height: f32,
    dx: f32,
    dy: f32,
    armed: bool,
}

impl BottomContact {
    const AXIS_RATIO: f32 = 1.18;
    const AXIS_SLOP: f32 = 14.0;

    pub(super) fn new(origin: NavOrigin, height: f32) -> Self {
        Self {
            origin,
            axis: Axis::Pending,
            height: height.max(1.0),
            dx: 0.0,
            dy: 0.0,
            armed: false,
        }
    }

    pub(super) fn update(&mut self, dx: f32, dy: f32) {
        self.dx = dx;
        self.dy = dy;
        if self.axis == Axis::Pending {
            self.axis = if dx.abs() >= Self::AXIS_SLOP && dx.abs() > dy.abs() * Self::AXIS_RATIO {
                Axis::Horizontal
            } else if dy.abs() >= Self::AXIS_SLOP && dy.abs() > dx.abs() * Self::AXIS_RATIO {
                Axis::Vertical
            } else {
                Axis::Pending
            };
        }
        if self.axis == Axis::Vertical && self.origin != NavOrigin::Recents {
            let fraction = if self.armed { 0.045 } else { 0.06 };
            self.armed = -dy > self.height * fraction;
        }
    }

    /// The preview tracks a held pull even before it arms; reversing returns to rest.
    pub(super) fn overview_progress(&self) -> Option<f32> {
        if self.axis != Axis::Vertical || self.origin == NavOrigin::Recents {
            return None;
        }
        // Bottom of centered 0.63h preview is at .815h. Home has no app to lift.
        let travel = self.height
            * if self.origin == NavOrigin::App {
                0.185
            } else {
                0.45
            };
        Some((-self.dy / travel.max(1.0)).clamp(0.0, 1.0))
    }

    pub(super) fn feedback(&self) -> (f32, f32, bool) {
        match self.axis {
            Axis::Horizontal if self.origin != NavOrigin::Recents => {
                (self.dx, 0.0, self.dx.abs() > 82.0)
            }
            Axis::Vertical if self.origin == NavOrigin::Recents => {
                (0.0, self.dy.max(0.0), self.dy > 72.0)
            }
            Axis::Vertical => (0.0, self.dy.min(0.0), self.armed),
            _ => (0.0, 0.0, false),
        }
    }

    pub(super) fn release(&self, (vx, vy): (f32, f32)) -> NavRelease {
        match (self.origin, self.axis) {
            (NavOrigin::Recents, Axis::Vertical) if self.dy > 72.0 || vy > 650.0 => {
                NavRelease::DismissOverview
            }
            (NavOrigin::Recents, _) => NavRelease::Cancel,
            (_, Axis::Horizontal) => {
                let flick = vx.abs() > 720.0 && vx.abs() > vy.abs() * Self::AXIS_RATIO;
                // A snap back toward the origin must not focus an app on
                // the other side, nor should a vertical fling count here.
                if self.dx.abs() < Self::AXIS_SLOP
                    || (flick && vx.signum() != self.dx.signum())
                    || (self.dx.abs() <= 82.0 && !flick)
                {
                    NavRelease::Cancel
                } else if self.dx > 0.0 {
                    NavRelease::Older
                } else {
                    NavRelease::Newer
                }
            }
            (origin, Axis::Vertical) if vy < -1100.0 => {
                if origin == NavOrigin::App {
                    NavRelease::Home
                } else {
                    NavRelease::Overview
                }
            }
            (_, Axis::Vertical) if self.armed => NavRelease::Overview,
            _ => NavRelease::Cancel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_release_goes_home_but_held_pull_opens_overview() {
        let mut contact = BottomContact::new(NavOrigin::App, 800.0);
        contact.update(1.0, -64.0);
        assert_eq!(contact.release((0.0, -1120.0)), NavRelease::Home);
        assert_eq!(contact.release((0.0, -1100.0)), NavRelease::Overview);
        assert_eq!(contact.release((0.0, 0.0)), NavRelease::Overview);
        assert_eq!(contact.release((0.0, 700.0)), NavRelease::Overview);
        let mut home = BottomContact::new(NavOrigin::Home, 800.0);
        home.update(1.0, -35.0);
        assert_eq!(home.release((0.0, -1200.0)), NavRelease::Overview);
        assert_eq!(home.release((0.0, 0.0)), NavRelease::Cancel);
    }

    #[test]
    fn axis_and_arm_are_logical_and_distance_only() {
        for height in [800.0, 1280.0] {
            let mut contact = BottomContact::new(NavOrigin::Home, height);
            contact.update(0.0, -13.0);
            assert_eq!(contact.release((0.0, -1800.0)), NavRelease::Cancel);
            contact.update(50.0, -50.0);
            assert_eq!(contact.overview_progress(), None);
            assert_eq!(contact.release((0.0, -1800.0)), NavRelease::Cancel);
            contact.update(0.0, -height * 0.06);
            assert_eq!(contact.release((0.0, 0.0)), NavRelease::Cancel);
            contact.update(0.0, -height * 0.061);
            assert_eq!(contact.release((0.0, 0.0)), NavRelease::Overview);
            assert!((contact.overview_progress().unwrap() - 0.061 / 0.45).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn reversing_unarms_without_changing_axis() {
        let mut contact = BottomContact::new(NavOrigin::App, 1000.0);
        contact.update(0.0, -61.0);
        contact.update(0.0, -50.0);
        assert_eq!(contact.release((0.0, 0.0)), NavRelease::Overview);
        contact.update(180.0, -40.0);
        assert_eq!(contact.release((1400.0, 0.0)), NavRelease::Cancel);
        contact.update(0.0, 2.0);
        assert_eq!(contact.overview_progress(), Some(0.0));
    }

    #[test]
    fn sideways_release_uses_direction_and_ignores_reversal() {
        let mut contact = BottomContact::new(NavOrigin::App, 800.0);
        assert_eq!(contact.release((0.0, 0.0)), NavRelease::Cancel);
        contact.update(120.0, 8.0);
        assert_eq!(contact.release((0.0, 0.0)), NavRelease::Older);
        assert_eq!(contact.release((-800.0, 0.0)), NavRelease::Cancel);
        contact.update(-24.0, 8.0);
        assert_eq!(contact.release((-750.0, 0.0)), NavRelease::Newer);
        assert_eq!(contact.release((-700.0, 0.0)), NavRelease::Cancel);
        assert_eq!(contact.release((-750.0, 700.0)), NavRelease::Cancel);
    }

    #[test]
    fn overview_uses_downward_dismiss_not_a_carousel_swipe() {
        let mut contact = BottomContact::new(NavOrigin::Recents, 800.0);
        contact.update(0.0, 72.0);
        assert_eq!(contact.release((0.0, 650.0)), NavRelease::Cancel);
        assert_eq!(contact.release((0.0, 651.0)), NavRelease::DismissOverview);
        contact.update(0.0, 75.0);
        assert_eq!(contact.release((0.0, 0.0)), NavRelease::DismissOverview);
        contact.update(0.0, -140.0);
        assert_eq!(contact.release((0.0, -1500.0)), NavRelease::Cancel);
    }
}
