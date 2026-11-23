//! Gesture geometry and release physics, independent of Wayland dispatch.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use veilshell_core::{Spring, SpringParams};
use veilshell_render::HitRect;

pub(crate) struct SliderDrag {
    pub(crate) id: &'static str,
    pub(crate) rect: HitRect,
    pub(crate) initial: u8,
    last_sent: Option<Instant>,
    dragging: bool,
}

impl SliderDrag {
    pub(crate) fn new(id: &'static str, rect: HitRect, initial: u8) -> Self {
        Self {
            id,
            rect,
            initial,
            last_sent: None,
            dragging: false,
        }
    }

    pub(crate) fn update(&mut self, dy: f32, now: Instant) -> Option<u8> {
        self.dragging |= dy.abs() >= 4.0;
        if !self.dragging
            || self
                .last_sent
                .is_some_and(|at| now.duration_since(at) < Duration::from_millis(40))
        {
            return None;
        }
        self.last_sent = Some(now);
        Some(self.relative(dy))
    }

    pub(crate) fn finish(&self, y: f32, dy: f32) -> u8 {
        if self.dragging {
            self.relative(dy)
        } else {
            slider_pct_for_y(y, self.rect)
        }
    }

    pub(crate) const fn is_dragging(&self) -> bool {
        self.dragging
    }

    fn relative(&self, dy: f32) -> u8 {
        (f32::from(self.initial) - dy * 100.0 / self.rect.h.max(1.0))
            .clamp(0.0, 100.0)
            .round() as u8
    }
}

pub(crate) fn slider_pct_for_y(y: f32, rect: HitRect) -> u8 {
    ((1.0 - (y - rect.y) / rect.h.max(1.0)).clamp(0.0, 1.0) * 100.0).round() as u8
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RevealPolicy {
    Panel,
    Edge,
    TabletStatus,
    TabletDismiss,
    Drawer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DragMotion {
    Reveal {
        from: f32,
        travel: f32,
        sign: f32,
        opening: bool,
        policy: RevealPolicy,
    },
    HomePage {
        from: f32,
        width: f32,
    },
    TabletShadePage {
        from: f32,
        width: f32,
        scale: f32,
    },
}

impl DragMotion {
    pub(crate) fn reveal(from: f32, travel: f32, sign: f32, opening: bool) -> Self {
        Self::Reveal {
            from,
            travel: travel.max(1.0),
            sign,
            opening,
            policy: RevealPolicy::Panel,
        }
    }

    pub(crate) fn edge_reveal(travel: f32, sign: f32) -> Self {
        Self::Reveal {
            from: 0.0,
            travel: travel.max(1.0),
            sign,
            opening: true,
            policy: RevealPolicy::Edge,
        }
    }

    pub(crate) fn tablet_shade(from: f32, height: f32, from_status: bool, opening: bool) -> Self {
        Self::Reveal {
            from,
            travel: height.max(1.0),
            sign: 1.0,
            opening,
            policy: if from_status {
                RevealPolicy::TabletStatus
            } else {
                RevealPolicy::TabletDismiss
            },
        }
    }

    pub(crate) const fn is_tablet_shade(self) -> bool {
        matches!(
            self,
            Self::Reveal {
                policy: RevealPolicy::TabletStatus | RevealPolicy::TabletDismiss,
                ..
            } | Self::TabletShadePage { .. }
        )
    }

    pub(crate) fn drawer(from: f32, height: f32, opening: bool) -> Self {
        Self::Reveal {
            from,
            travel: height.max(1.0),
            sign: -1.0,
            opening,
            policy: RevealPolicy::Drawer,
        }
    }

    pub(crate) const fn is_drawer(self) -> bool {
        matches!(
            self,
            Self::Reveal {
                policy: RevealPolicy::Drawer,
                ..
            }
        )
    }

    pub(crate) fn position(self, dx: f32, dy: f32, pages: usize) -> f32 {
        match self {
            Self::Reveal {
                from, travel, sign, ..
            } => (from + sign * dy / travel).clamp(0.0, 1.0),
            Self::HomePage { from, width } => {
                let origin = from.round();
                let value = (from - dx / width).clamp(origin - 1.0, origin + 1.0);
                let edge = value.clamp(0.0, pages.saturating_sub(1) as f32);
                // Resist pulling beyond the first or last page.
                edge + ((value - edge) * 0.22).clamp(-0.18, 0.18)
            }
            Self::TabletShadePage { from, width, .. } => (from - dx / width).clamp(0.0, 1.0),
        }
    }

    pub(crate) fn release(self, value: f32, velocity: (f32, f32), pages: usize) -> Spring {
        let (vx, vy) = velocity;
        let (target, speed, params) = match self {
            Self::Reveal {
                from,
                travel,
                sign,
                opening,
                policy,
            } => {
                let speed = (vy * sign / travel).clamp(-6.0, 6.0);
                let commit = match policy {
                    RevealPolicy::Edge => {
                        let distance = value * travel;
                        // A short pull can commit without speed; even a fast flick
                        // needs real travel. An inward reversal cancels a short pull.
                        let fling = (vy * sign * 0.075).clamp(-48.0, 48.0);
                        distance >= 22.0 && distance + fling >= 64.0
                    }
                    RevealPolicy::Panel => {
                        let projected = value + (speed * 0.10).clamp(-0.40, 0.40);
                        projected >= if opening { 0.36 } else { 0.64 }
                    }
                    RevealPolicy::TabletStatus | RevealPolicy::TabletDismiss => {
                        let dismiss = policy == RevealPolicy::TabletDismiss;
                        // Denial's status drag normalizes to the full output.
                        // The content dismiss area also accepts a short upward stroke.
                        let content_close =
                            dismiss && (value <= from - 48.0 / travel || vy <= -500.0);
                        !content_close && vy > -520.0 && (value >= 126.0 / 488.0 || vy >= 520.0)
                    }
                    RevealPolicy::Drawer => vy <= 600.0 && (vy < -600.0 || value > 0.4),
                };
                let params = if !commit
                    && matches!(
                        policy,
                        RevealPolicy::TabletStatus | RevealPolicy::TabletDismiss
                    ) {
                    SpringParams::gentle()
                } else {
                    SpringParams::snappy()
                };
                // Use the flick to choose the Drawer destination, not to
                // send the sheet beyond either endpoint.
                let speed = if policy == RevealPolicy::Drawer {
                    0.0
                } else {
                    speed
                };
                (f32::from(u8::from(commit)), speed, params)
            }
            Self::HomePage { from, width } => {
                let origin = from.round().clamp(0.0, pages.saturating_sub(1) as f32);
                let speed = (-vx / width).clamp(-6.0, 6.0);
                let projected = value + (speed * 0.14).clamp(-0.42, 0.42) - origin;
                let step = if projected.abs() >= 0.28 {
                    projected.signum()
                } else {
                    0.0
                };
                let target = (origin + step).clamp(0.0, pages.saturating_sub(1) as f32);
                (target, speed, SpringParams::gentle())
            }
            Self::TabletShadePage { from, width, scale } => {
                let reached = |px| {
                    let fraction = px * scale / width;
                    value <= from - fraction || value >= from + fraction
                };
                let fling = vx.abs() >= 250.0 * scale && reached(30.0);
                let target = if fling || reached(90.0) {
                    let direction = if fling {
                        -vx.signum()
                    } else {
                        (value - from).signum()
                    };
                    (from + direction).clamp(0.0, 1.0)
                } else {
                    from
                };
                (
                    target,
                    (-vx / width).clamp(-6.0, 6.0),
                    SpringParams::snappy(),
                )
            }
        };
        spring_to(value, target, speed, params)
    }

    pub(crate) fn cancel(self, value: f32, pages: usize) -> Spring {
        match self {
            Self::Reveal { opening, .. } => spring_to(
                value,
                f32::from(u8::from(!opening)),
                0.0,
                SpringParams::snappy(),
            ),
            Self::HomePage { from, .. } => spring_to(
                value,
                from.round().clamp(0.0, pages.saturating_sub(1) as f32),
                0.0,
                SpringParams::gentle(),
            ),
            Self::TabletShadePage { from, .. } => {
                spring_to(value, from, 0.0, SpringParams::snappy())
            }
        }
    }
}

pub(crate) fn spring_to(value: f32, target: f32, velocity: f32, params: SpringParams) -> Spring {
    let mut spring = Spring::new(value, params);
    spring.target = target;
    spring.velocity = velocity;
    spring
}

/// A very short settle can start inside the spring's rest tolerance. Commit
/// the exact endpoint so painting and hit testing agree on full openness.
pub(crate) fn snap_at_rest(spring: &mut Spring) -> bool {
    if !spring.is_settled() {
        return false;
    }
    spring.value = spring.target;
    spring.velocity = 0.0;
    true
}

/// While the finger is in All Apps, the list owns it until a downward
/// delta exceeds the remaining scroll. Only that excess pulls the sheet;
/// after handoff the list stays frozen for the rest of this contact.
pub(crate) struct DrawerContact {
    origin_scroll: f32,
    scroll: f32,
    scroll_max: f32,
    height: f32,
    last_dy: f32,
    pull: Option<f32>,
}

impl DrawerContact {
    pub(crate) fn new(scroll: f32, scroll_max: f32, height: f32) -> Self {
        let scroll = scroll.clamp(0.0, scroll_max);
        Self {
            origin_scroll: scroll,
            scroll,
            scroll_max,
            height: height.max(1.0),
            last_dy: 0.0,
            pull: None,
        }
    }

    /// Position is (list scroll in logical pixels, sheet openness).
    pub(crate) fn update(&mut self, dy: f32) -> (f32, f32) {
        let delta = dy - self.last_dy;
        self.last_dy = dy;
        if let Some(pull) = &mut self.pull {
            *pull = (*pull + delta).clamp(0.0, self.height);
        } else {
            let next = self.scroll - delta;
            self.scroll = next.clamp(0.0, self.scroll_max);
            if next < 0.0 {
                self.pull = Some((-next).min(self.height));
            }
        }
        (self.scroll, self.progress())
    }

    fn progress(&self) -> f32 {
        1.0 - self.pull.unwrap_or(0.0) / self.height
    }

    pub(crate) fn release(&self, velocity: (f32, f32)) -> Spring {
        if self.pull.is_some() {
            DragMotion::drawer(1.0, self.height, false).release(self.progress(), velocity, 1)
        } else {
            Spring::new(1.0, SpringParams::snappy())
        }
    }

    pub(crate) const fn origin_scroll(&self) -> f32 {
        self.origin_scroll
    }
}

/// Wayland can deliver multiple input samples in one callback. Use the
/// compositor's wrapped millisecond counter, not dispatch time, for new
/// Tablet gestures. `None` indicates a time before this contact began.
pub(crate) fn input_event_time(origin: Instant, first: u32, time: u32) -> Option<Instant> {
    let elapsed = time.wrapping_sub(first);
    (elapsed < 1 << 31).then(|| origin + Duration::from_millis(u64::from(elapsed)))
}

/// Lock the axis only after deliberate travel, not on tap jitter.
pub(crate) fn vertical(dx: f32, dy: f32) -> bool {
    dy.abs() >= 10.0 && dy.abs() > dx.abs() * 1.18
}

/// Pick up edge pulls earlier without changing in-panel scrolling or dismissal.
pub(crate) fn edge_vertical(dx: f32, dy: f32) -> bool {
    dy.abs() >= 6.0 && dy.abs() > dx.abs() * 1.18
}

pub(crate) fn horizontal(dx: f32, dy: f32) -> bool {
    dx.abs() >= 10.0 && dx.abs() > dy.abs() * 1.18
}

pub(crate) struct DragVelocity(VecDeque<(Instant, f32, f32)>);

impl DragVelocity {
    pub(crate) fn new(now: Instant, x: f32, y: f32) -> Self {
        Self(VecDeque::from([(now, x, y)]))
    }

    pub(crate) fn push(&mut self, now: Instant, x: f32, y: f32) {
        while self.0.len() >= 8
            || self
                .0
                .front()
                .is_some_and(|p| now.duration_since(p.0) > Duration::from_millis(95))
        {
            self.0.pop_front();
        }
        self.0.push_back((now, x, y));
    }

    pub(crate) fn push_input_event(&mut self, now: Instant, x: f32, y: f32) {
        if let Some(last) = self.0.back_mut() {
            if now < last.0 {
                return;
            }
            if now == last.0 {
                *last = (now, x, y);
                return;
            }
        }
        self.push(now, x, y);
    }

    pub(crate) fn accepts_input_event(&self, at: Instant) -> bool {
        self.0.back().is_none_or(|sample| at >= sample.0)
    }

    pub(crate) fn at_release(&self, now: Instant) -> (f32, f32) {
        self.speed(now, Duration::from_millis(8))
    }

    pub(crate) fn at_input_release(&self, now: Instant) -> (f32, f32) {
        // Integer protocol timestamps need only two distinct samples. At
        // high refresh a real motion may be delivered <8ms after down.
        self.speed(now, Duration::from_millis(1))
    }

    fn speed(&self, now: Instant, min_span: Duration) -> (f32, f32) {
        let (Some(&(start, x0, y0)), Some(&(end, x1, y1))) = (self.0.front(), self.0.back()) else {
            return (0.0, 0.0);
        };
        let quiet = now.duration_since(end);
        let elapsed = end.duration_since(start);
        if quiet >= Duration::from_millis(70) || elapsed < min_span {
            return (0.0, 0.0);
        }
        let dt = (elapsed + quiet).as_secs_f32();
        ((x1 - x0) / dt, (y1 - y0) / dt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_follows_travel_then_reverses_or_finishes_without_jumping() {
        let pull = DragMotion::reveal(0.0, 240.0, 1.0, true);
        assert_eq!(pull.position(0.0, 120.0, 1), 0.5);
        let reversed = pull.position(0.0, 60.0, 1);
        let cancelled = pull.release(reversed, (0.0, -200.0), 1);
        assert_eq!((cancelled.value, cancelled.target), (0.25, 0.0));
        assert_eq!(pull.release(0.5, (0.0, 0.0), 1).target, 1.0);
        let close = DragMotion::reveal(1.0, 240.0, 1.0, false);
        assert_eq!(close.position(0.0, -120.0, 1), 0.5);
        assert_eq!(close.cancel(0.5, 1).target, 1.0);
    }

    #[test]
    fn edge_opening_needs_direction_and_travel_not_just_a_flick() {
        assert!(!edge_vertical(0.0, 5.0));
        assert!(!edge_vertical(9.0, 8.0));
        assert!(edge_vertical(3.0, 6.0));
        for (sign, travel) in [(-1.0, 360.0), (1.0, 380.0)] {
            let pull = DragMotion::edge_reveal(travel, sign);
            let settle = |d, v| pull.release(pull.position(0.0, sign * d, 1), (0.0, sign * v), 1);
            assert_eq!(settle(70.0, 0.0).target, 1.0);
            assert_eq!(settle(30.0, 700.0).target, 1.0);
            assert_eq!(settle(12.0, 1000.0).target, 0.0);
            assert_eq!(settle(30.0, 0.0).target, 0.0);
            assert_eq!(settle(70.0, -500.0).target, 0.0);
        }
    }

    #[test]
    fn paging_resists_edges_and_snaps_to_a_neighbor() {
        let drag = DragMotion::HomePage {
            from: 1.0,
            width: 400.0,
        };
        assert_eq!(drag.position(-100.0, 0.0, 4), 1.25);
        assert_eq!(drag.release(1.25, (0.0, 0.0), 4).target, 1.0);
        assert_eq!(drag.release(1.25, (-500.0, 0.0), 4).target, 2.0);
        let edge = DragMotion::HomePage {
            from: 0.0,
            width: 400.0,
        };
        assert_eq!(edge.position(400.0, 0.0, 4), -0.18);
        assert_eq!(edge.release(-0.18, (0.0, 0.0), 4).target, 0.0);
    }

    #[test]
    fn tablet_content_dismiss_uses_upward_travel_or_flick() {
        let dismiss = DragMotion::tablet_shade(1.0, 960.0, false, false);
        let settle = |dy, vy| dismiss.release(dismiss.position(0.0, dy, 1), (0.0, vy), 1);
        assert_eq!(settle(-48.0, 0.0).target, 0.0);
        assert_eq!(settle(-47.0, 0.0).target, 1.0);
        assert_eq!(settle(-22.0, -500.0).target, 0.0);
        assert_eq!(settle(-22.0, -499.0).target, 1.0);
        assert_eq!(dismiss.cancel(0.4, 1).target, 1.0);

        // The independent status gesture keeps the longer reveal rule.
        let status = DragMotion::tablet_shade(1.0, 960.0, true, false);
        assert_eq!(status.release(0.9, (0.0, -500.0), 1).target, 1.0);
        assert_eq!(status.release(0.9, (0.0, -520.0), 1).target, 0.0);
    }

    #[test]
    fn tablet_paging_uses_scaled_distance_and_directional_fling() {
        let width = 800.0;
        let scale = 2.0;
        for (from, direction) in [(0.0, -1.0_f32), (1.0, 1.0)] {
            let swipe = DragMotion::TabletShadePage { from, width, scale };
            let other = 1.0 - from;
            let position = |dx| swipe.position(direction * dx, 0.0, 2);
            let settle = |dx, vx| swipe.release(position(dx), (direction * vx, 0.0), 2);
            assert_eq!(position(400.0), 0.5);
            assert_eq!(position(-400.0), from);
            assert_eq!(position(1100.0), other);
            assert_eq!(settle(179.0, 0.0).target, from);
            assert_eq!(settle(180.0, 0.0).target, other);
            assert_eq!(settle(59.0, 600.0).target, from);
            assert_eq!(settle(60.0, 499.0).target, from);
            assert_eq!(settle(60.0, 500.0).target, other);
            assert_eq!(settle(185.0, -500.0).target, from);
            assert_eq!(swipe.cancel(0.5, 2).target, from);
        }
    }

    #[test]
    fn a_pause_or_reversal_does_not_count_as_the_original_flick() {
        let now = Instant::now();
        let mut speed = DragVelocity::new(now, 0.0, 0.0);
        speed.push(now + Duration::from_millis(20), 0.0, 100.0);
        speed.push(now + Duration::from_millis(120), 0.0, 120.0);
        speed.push(now + Duration::from_millis(140), 0.0, 100.0);
        assert!(speed.at_release(now + Duration::from_millis(145)).1 < 0.0);
        assert_eq!(
            speed.at_release(now + Duration::from_millis(220)),
            (0.0, 0.0)
        );
    }

    #[test]
    fn tablet_velocity_survives_batched_and_wrapped_protocol_events() {
        let origin = Instant::now();
        let first = u32::MAX - 10;
        let at = |ms: u32| input_event_time(origin, first, first.wrapping_add(ms)).unwrap();
        let mut speed = DragVelocity::new(origin, 12.0, 700.0);
        speed.push_input_event(at(10), 12.0, 684.0);
        speed.push_input_event(at(10), 12.0, 680.0);
        speed.push_input_event(at(35), 12.0, 630.0);

        assert!(speed.at_release(at(40)).1 < -1100.0);
        assert_eq!(speed.at_release(at(106)), (0.0, 0.0));
        assert!(input_event_time(origin, first, first - 1).is_none());
        assert!(!speed.accepts_input_event(at(34)));

        let mut high_refresh = DragVelocity::new(origin, 12.0, 700.0);
        high_refresh.push_input_event(at(4), 12.0, 680.0);
        assert!(high_refresh.at_input_release(at(8)).1 < -1100.0);
    }

    #[test]
    fn drawer_follows_full_height_and_settles_without_overshooting() {
        for height in [800.0, 1280.0] {
            let open = DragMotion::drawer(0.0, height, true);
            let progress = open.position(0.0, -height * 0.38, 1);
            assert!((progress - 0.38).abs() < f32::EPSILON);
            assert_eq!(open.release(progress, (0.0, -600.0), 1).target, 0.0);
            assert_eq!(open.release(progress, (0.0, -601.0), 1).target, 1.0);
            assert_eq!(open.release(0.4, (0.0, 0.0), 1).target, 0.0);
            assert_eq!(open.release(0.41, (0.0, 0.0), 1).target, 1.0);
            assert_eq!(open.cancel(0.98, 1).target, 0.0);

            let mut settle = open.release(0.98, (0.0, -4000.0), 1);
            for _ in 0..172 {
                let before = settle.value;
                settle.step(1.0 / 240.0);
                assert!((before..=1.0).contains(&settle.value));
            }
            assert!(snap_at_rest(&mut settle));
            assert_eq!(settle.value, 1.0);

            let mut almost_open = open.release(0.9999, (0.0, 0.0), 1);
            assert!(snap_at_rest(&mut almost_open));
            assert_eq!(almost_open.value, 1.0);
        }
    }

    #[test]
    fn drawer_hands_only_excess_travel_to_the_sheet_and_can_revert() {
        let mut contact = DrawerContact::new(110.0, 200.0, 800.0);
        assert_eq!(contact.update(50.0), (60.0, 1.0));
        assert_eq!(contact.release((0.0, 1000.0)).target, 1.0);
        assert_eq!(contact.update(110.0), (0.0, 1.0));
        assert_eq!(contact.release((0.0, 1000.0)).target, 1.0);
        assert_eq!(contact.update(150.0), (0.0, 0.95));
        assert_eq!(contact.release((0.0, 601.0)).target, 0.0);
        assert_eq!(contact.update(0.0), (0.0, 1.0));
        assert_eq!(contact.update(-90.0), (0.0, 1.0), "handoff is latched");
        assert_eq!(contact.origin_scroll(), 110.0);
    }

    #[test]
    fn drawer_scroll_clamps_at_the_last_row_with_no_dead_overscroll() {
        let mut contact = DrawerContact::new(50.0, 200.0, 800.0);
        for (dy, scroll) in [
            (-50.0, 100.0),
            (-150.0, 200.0),
            (-750.0, 200.0),
            (-749.0, 199.0),
        ] {
            assert_eq!(contact.update(dy), (scroll, 1.0));
        }
        let mut short = DrawerContact::new(0.0, 0.0, 800.0);
        assert_eq!(short.update(-80.0), (0.0, 1.0));
        assert_eq!(short.update(0.0), (0.0, 0.9));
        assert_eq!(short.release((0.0, 600.0)).target, 1.0);
        assert_eq!(short.release((0.0, 601.0)).target, 0.0);
    }

    #[test]
    fn shade_slider_taps_absolute_but_drags_from_the_current_level() {
        let now = Instant::now();
        let mut slider = SliderDrag::new("volume", HitRect::new(0.0, 100.0, 60.0, 200.0), 30);
        assert_eq!(slider.finish(120.0, 0.0), 90);
        assert_eq!(slider.update(-2.0, now), None);
        assert_eq!(slider.update(-20.0, now), Some(40));
        assert_eq!(slider.update(-100.0, now + Duration::from_millis(15)), None);
        assert_eq!(slider.finish(120.0, -100.0), 80);
    }
}
