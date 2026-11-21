//! Laptop <-> Tablet mode state machine.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellMode {
    Laptop,
    Tablet,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModePolicy {
    /// Follow physical keyboard presence / `SW_TABLET_MODE` automatically.
    Auto,
    ForceLaptop,
    ForceTablet,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeEvent {
    KeyboardAttached(bool),
    SetPolicy(ModePolicy),
    ToggleManual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModeStateMachine {
    policy: ModePolicy,
    keyboard_attached: bool,
}

impl ModeStateMachine {
    #[must_use]
    pub const fn new(policy: ModePolicy, keyboard_attached: bool) -> Self {
        Self {
            policy,
            keyboard_attached,
        }
    }

    #[must_use]
    pub const fn policy(&self) -> ModePolicy {
        self.policy
    }

    #[must_use]
    pub const fn effective_mode(&self) -> ShellMode {
        match self.policy {
            ModePolicy::Auto => {
                if self.keyboard_attached {
                    ShellMode::Laptop
                } else {
                    ShellMode::Tablet
                }
            }
            ModePolicy::ForceLaptop => ShellMode::Laptop,
            ModePolicy::ForceTablet => ShellMode::Tablet,
        }
    }

    /// Apply an event and return `Some(new_mode)` only if the effective mode changed.
    pub fn handle(&mut self, event: ModeEvent) -> Option<ShellMode> {
        let before = self.effective_mode();
        match event {
            ModeEvent::KeyboardAttached(attached) => self.keyboard_attached = attached,
            ModeEvent::SetPolicy(policy) => self.policy = policy,
            ModeEvent::ToggleManual => {
                self.policy = match before {
                    ShellMode::Laptop => ModePolicy::ForceTablet,
                    ShellMode::Tablet => ModePolicy::ForceLaptop,
                };
            }
        }
        let after = self.effective_mode();
        (after != before).then_some(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_policy_tracks_type_cover_detach() {
        let mut sm = ModeStateMachine::new(ModePolicy::Auto, true);
        assert_eq!(sm.effective_mode(), ShellMode::Laptop);
        assert_eq!(
            sm.handle(ModeEvent::KeyboardAttached(false)),
            Some(ShellMode::Tablet)
        );
        assert_eq!(sm.handle(ModeEvent::KeyboardAttached(false)), None);
        assert_eq!(
            sm.handle(ModeEvent::KeyboardAttached(true)),
            Some(ShellMode::Laptop)
        );
    }

    #[test]
    fn manual_override_holds_mode_until_returned_to_auto() {
        let mut sm = ModeStateMachine::new(ModePolicy::Auto, true);
        assert_eq!(sm.handle(ModeEvent::ToggleManual), Some(ShellMode::Tablet));
        assert_eq!(sm.handle(ModeEvent::KeyboardAttached(true)), None);
        assert_eq!(
            sm.handle(ModeEvent::SetPolicy(ModePolicy::Auto)),
            Some(ShellMode::Laptop)
        );
    }
}
