//! Touch gesture classification for bottom pill and edge swipes.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GestureAction {
    None,
    Home,
    Recents,
    SwitchNext,
    SwitchPrev,
    OpenShade,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureSample {
    /// Total drag delta in logical pixels (positive X = right, positive Y = down).
    pub dx: f32,
    pub dy: f32,
    /// Release velocity in logical pixels per second.
    pub vx: f32,
    pub vy: f32,
    /// Logical viewport height.
    pub viewport_height: f32,
    /// Whether an application is currently in the foreground.
    pub has_foreground: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureThresholds {
    pub home_flick_velocity: f32,
    pub recents_arm_fraction: f32,
    pub switch_distance: f32,
    pub switch_flick_velocity: f32,
    pub axis_lock_ratio: f32,
}

impl Default for GestureThresholds {
    fn default() -> Self {
        Self {
            home_flick_velocity: 1100.0,
            recents_arm_fraction: 0.06,
            switch_distance: 82.0,
            switch_flick_velocity: 720.0,
            axis_lock_ratio: 1.18,
        }
    }
}

/// Classify a release gesture originating from the bottom gesture pill.
#[must_use]
pub fn classify_bottom_pill(sample: GestureSample, t: GestureThresholds) -> GestureAction {
    let horiz_dist = sample.dx.abs();
    let vert_dist = sample.dy.abs();
    let horizontal_dominant = horiz_dist > vert_dist * t.axis_lock_ratio;

    if horizontal_dominant
        && (horiz_dist >= t.switch_distance || sample.vx.abs() >= t.switch_flick_velocity)
    {
        return if sample.dx < 0.0 {
            GestureAction::SwitchNext
        } else {
            GestureAction::SwitchPrev
        };
    }

    let upward = -sample.dy;
    let recents_arm_px = sample.viewport_height * t.recents_arm_fraction;

    if sample.vy <= -t.home_flick_velocity {
        return if sample.has_foreground {
            GestureAction::Home
        } else {
            GestureAction::Recents
        };
    }

    if upward >= recents_arm_px {
        return GestureAction::Recents;
    }

    GestureAction::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_up_flick_goes_home_from_app_and_recents_from_home() {
        let t = GestureThresholds::default();
        let from_app = GestureSample {
            dx: 2.0,
            dy: -160.0,
            vx: 10.0,
            vy: -1800.0,
            viewport_height: 853.0,
            has_foreground: true,
        };
        assert_eq!(classify_bottom_pill(from_app, t), GestureAction::Home);

        let from_home = GestureSample {
            has_foreground: false,
            ..from_app
        };
        assert_eq!(classify_bottom_pill(from_home, t), GestureAction::Recents);
    }

    #[test]
    fn slow_upward_pull_opens_recents() {
        let t = GestureThresholds::default();
        let pull = GestureSample {
            dx: 0.0,
            dy: -90.0,
            vx: 0.0,
            vy: -120.0,
            viewport_height: 853.0,
            has_foreground: true,
        };
        assert_eq!(classify_bottom_pill(pull, t), GestureAction::Recents);
    }

    #[test]
    fn horizontal_swipes_switch_adjacent_apps() {
        let t = GestureThresholds::default();
        let swipe_left = GestureSample {
            dx: -120.0,
            dy: -10.0,
            vx: -900.0,
            vy: 0.0,
            viewport_height: 853.0,
            has_foreground: true,
        };
        assert_eq!(
            classify_bottom_pill(swipe_left, t),
            GestureAction::SwitchNext
        );

        let swipe_right = GestureSample {
            dx: 120.0,
            vx: 900.0,
            ..swipe_left
        };
        assert_eq!(
            classify_bottom_pill(swipe_right, t),
            GestureAction::SwitchPrev
        );
    }
}
