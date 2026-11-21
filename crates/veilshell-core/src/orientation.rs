//! Display orientation, rotation lock, and coordinate transforms.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Orientation {
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
}

impl Orientation {
    #[must_use]
    pub fn from_sensor_str(raw: &str) -> Option<Self> {
        match raw.trim() {
            "normal" => Some(Self::Normal),
            "left-up" => Some(Self::Rotate90),
            "bottom-up" => Some(Self::Rotate180),
            "right-up" => Some(Self::Rotate270),
            _ => None,
        }
    }

    /// Hyprland monitor transform integer (`0..=3`).
    #[must_use]
    pub const fn hyprland_transform(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Rotate90 => 1,
            Self::Rotate180 => 2,
            Self::Rotate270 => 3,
        }
    }

    /// Map normalized logical UI coordinates `(u, v)` into physical panel space.
    #[must_use]
    pub fn logical_to_physical(self, u: f32, v: f32) -> (f32, f32) {
        let u = u.clamp(0.0, 1.0);
        let v = v.clamp(0.0, 1.0);
        match self {
            Self::Normal => (u, v),
            Self::Rotate90 => (1.0 - v, u),
            Self::Rotate180 => (1.0 - u, 1.0 - v),
            Self::Rotate270 => (v, 1.0 - u),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RotationLock {
    latest: Orientation,
    locked: Option<Orientation>,
}

impl Default for RotationLock {
    fn default() -> Self {
        Self {
            latest: Orientation::Normal,
            locked: None,
        }
    }
}

impl RotationLock {
    pub fn observe(&mut self, reading: Option<Orientation>) {
        if let Some(ori) = reading {
            self.latest = ori;
        }
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = if locked { Some(self.effective()) } else { None };
    }

    #[must_use]
    pub const fn is_locked(&self) -> bool {
        self.locked.is_some()
    }

    #[must_use]
    pub const fn effective(&self) -> Orientation {
        match self.locked {
            Some(frozen) => frozen,
            None => self.latest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_freezes_current_and_applies_latest_on_unlock() {
        let mut lock = RotationLock::default();
        lock.observe(Some(Orientation::Rotate90));
        lock.set_locked(true);
        lock.observe(Some(Orientation::Rotate270));
        lock.observe(None);
        assert_eq!(lock.effective(), Orientation::Rotate90);
        lock.set_locked(false);
        assert_eq!(lock.effective(), Orientation::Rotate270);
    }
}
