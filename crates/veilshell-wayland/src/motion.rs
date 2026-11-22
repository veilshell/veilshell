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
pub(crate) enum DragMotion {
    Reveal {
        from: f32,
        travel: f32,
        sign: f32,
        opening: bool,
        edge_open: bool,
    },
    HomePage {
        from: f32,
        width: f32,
    },
}

impl DragMotion {
    pub(crate) fn reveal(from: f32, travel: f32, sign: f32, opening: bool) -> Self {
        Self::Reveal {
            from,
            travel: travel.max(1.0),
            sign,
            opening,
            edge_open: false,
        }
    }

    pub(crate) fn edge_reveal(travel: f32, sign: f32) -> Self {
        Self::Reveal {
            from: 0.0,
            travel: travel.max(1.0),
            sign,
            opening: true,
            edge_open: true,
        }
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
        }
    }

    pub(crate) fn release(self, value: f32, velocity: (f32, f32), pages: usize) -> Spring {
        let (vx, vy) = velocity;
        let (target, speed, params) = match self {
            Self::Reveal {
                travel,
                sign,
                opening,
                edge_open,
                ..
            } => {
                let speed = (vy * sign / travel).clamp(-6.0, 6.0);
                let commit = if edge_open {
                    let distance = value * travel;
                    // A short pull can commit without speed; even a fast flick
                    // needs real travel. An inward reversal cancels a short pull.
                    let fling = (vy * sign * 0.075).clamp(-48.0, 48.0);
                    distance >= 22.0 && distance + fling >= 64.0
                } else {
                    let projected = value + (speed * 0.10).clamp(-0.40, 0.40);
                    projected >= if opening { 0.36 } else { 0.64 }
                };
                (f32::from(u8::from(commit)), speed, SpringParams::snappy())
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
        }
    }
}

fn spring_to(value: f32, target: f32, velocity: f32, params: SpringParams) -> Spring {
    let mut spring = Spring::new(value, params);
    spring.target = target;
    spring.velocity = velocity;
    spring
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

    pub(crate) fn at_release(&self, now: Instant) -> (f32, f32) {
        let (Some(&(start, x0, y0)), Some(&(end, x1, y1))) = (self.0.front(), self.0.back()) else {
            return (0.0, 0.0);
        };
        let quiet = now.duration_since(end);
        let elapsed = end.duration_since(start);
        if quiet >= Duration::from_millis(70) || elapsed < Duration::from_millis(8) {
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
