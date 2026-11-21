//! Damped harmonic spring physics for locked-refresh-rate UI transitions.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringParams {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    pub rest_delta: f32,
    pub rest_velocity: f32,
}

impl SpringParams {
    /// Snappy, critically damped curve suited for tablet pills and sheets.
    #[must_use]
    pub const fn snappy() -> Self {
        Self {
            stiffness: 576.0,
            damping: 48.0,
            mass: 1.0,
            rest_delta: 0.001,
            rest_velocity: 0.005,
        }
    }

    /// Smooth, slightly softer curve suited for paged grids and card decks.
    #[must_use]
    pub const fn gentle() -> Self {
        Self {
            stiffness: 220.0,
            damping: 28.0,
            mass: 1.0,
            rest_delta: 0.001,
            rest_velocity: 0.005,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub value: f32,
    pub target: f32,
    pub velocity: f32,
    pub params: SpringParams,
}

impl Spring {
    #[must_use]
    pub const fn new(initial: f32, params: SpringParams) -> Self {
        Self {
            value: initial,
            target: initial,
            velocity: 0.0,
            params,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Advance the spring by `dt_secs` using semi-implicit Euler sub-stepping
    /// for numerical stability across variable refresh rates (48-120 Hz).
    pub fn step(&mut self, dt_secs: f32) -> f32 {
        if dt_secs <= 0.0 || self.is_settled() {
            return self.value;
        }
        let clamped = dt_secs.min(0.05);
        let substeps = 4;
        let sub_dt = clamped / substeps as f32;

        for _ in 0..substeps {
            let displacement = self.value - self.target;
            let spring_force = -self.params.stiffness * displacement;
            let damping_force = -self.params.damping * self.velocity;
            let accel = (spring_force + damping_force) / self.params.mass;
            self.velocity += accel * sub_dt;
            self.value += self.velocity * sub_dt;
        }

        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
        }
        self.value
    }

    #[must_use]
    pub fn is_settled(&self) -> bool {
        (self.value - self.target).abs() <= self.params.rest_delta
            && self.velocity.abs() <= self.params.rest_velocity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snappy_spring_converges_within_half_second() {
        let mut s = Spring::new(0.0, SpringParams::snappy());
        s.set_target(1.0);
        for _ in 0..30 {
            s.step(1.0 / 60.0);
        }
        assert!(s.is_settled(), "value={} vel={}", s.value, s.velocity);
        assert!((s.value - 1.0).abs() < 1e-4);
    }

    #[test]
    fn gentle_spring_is_monotonic_without_overshoot_explosion() {
        let mut s = Spring::new(0.0, SpringParams::gentle());
        s.set_target(100.0);
        for _ in 0..90 {
            let v = s.step(1.0 / 60.0);
            assert!(v.is_finite() && (-1.0..=105.0).contains(&v));
        }
        assert!(s.is_settled());
    }
}
