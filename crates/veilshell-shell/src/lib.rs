//! Top-level shell state orchestration combining core logic, compositor, hardware, and theme crates.

pub mod apps;
pub mod ipc;
pub mod session;
pub mod settings;
pub mod shade;

use std::fmt::Write as _;

use veilshell_compositor::{CompositorBackend, CompositorError};
use veilshell_core::{
    ModeEvent, ModePolicy, ModeStateMachine, Orientation, RotationLock, ShellMode, ThemePalette,
};
use veilshell_hw::HardwareBackend;
use veilshell_theme::format_hex;

/// Coordinated runtime state for the `veilshell` daemon.
#[derive(Debug)]
pub struct ShellState<C, H> {
    compositor: C,
    hardware: H,
    mode_sm: ModeStateMachine,
    rotation: RotationLock,
    palette: ThemePalette,
}

impl<C: CompositorBackend, H: HardwareBackend> ShellState<C, H> {
    /// Construct initial shell state from the current hardware snapshot and palette.
    #[must_use]
    pub fn new(compositor: C, hardware: H, palette: ThemePalette) -> Self {
        let snap = hardware.snapshot();
        let mode_sm = ModeStateMachine::new(ModePolicy::Auto, snap.keyboard_attached);
        let mut rotation = RotationLock::default();
        rotation.observe(Some(snap.orientation));
        Self {
            compositor,
            hardware,
            mode_sm,
            rotation,
            palette,
        }
    }

    /// Current effective [`ShellMode`].
    #[must_use]
    pub const fn mode(&self) -> ShellMode {
        self.mode_sm.effective_mode()
    }

    /// Current [`ModePolicy`].
    #[must_use]
    pub const fn policy(&self) -> ModePolicy {
        self.mode_sm.policy()
    }

    /// Current effective [`Orientation`].
    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.rotation.effective()
    }

    /// Active [`ThemePalette`].
    #[must_use]
    pub const fn palette(&self) -> &ThemePalette {
        &self.palette
    }

    /// Set the mode policy and propagate any resulting mode change to the compositor.
    ///
    /// # Errors
    /// Returns [`CompositorError`] if the compositor rejects the mode transition.
    pub fn set_policy(&mut self, policy: ModePolicy) -> Result<ShellMode, CompositorError> {
        if let Some(next_mode) = self.mode_sm.handle(ModeEvent::SetPolicy(policy)) {
            self.compositor.apply_shell_mode(next_mode)?;
        }
        Ok(self.mode())
    }

    /// Manually toggle between Laptop and Tablet mode.
    ///
    /// # Errors
    /// Returns [`CompositorError`] if the compositor rejects the mode transition.
    pub fn toggle_mode(&mut self) -> Result<ShellMode, CompositorError> {
        if let Some(next_mode) = self.mode_sm.handle(ModeEvent::ToggleManual) {
            self.compositor.apply_shell_mode(next_mode)?;
        }
        Ok(self.mode())
    }

    /// Poll the hardware backend and reconcile keyboard presence and display orientation.
    ///
    /// # Errors
    /// Returns [`CompositorError`] if applying the updated mode or rotation fails.
    pub fn refresh_hardware(&mut self) -> Result<(), CompositorError> {
        let snap = self.hardware.snapshot();
        if let Some(next_mode) = self
            .mode_sm
            .handle(ModeEvent::KeyboardAttached(snap.keyboard_attached))
        {
            self.compositor.apply_shell_mode(next_mode)?;
        }

        let prev_ori = self.rotation.effective();
        self.rotation.observe(Some(snap.orientation));
        let next_ori = self.rotation.effective();
        if next_ori != prev_ori && self.mode() == ShellMode::Tablet {
            if let Ok(outputs) = self.compositor.list_outputs() {
                if let Some(primary) = outputs.first() {
                    self.compositor
                        .set_output_orientation(&primary.name, next_ori)?;
                }
            }
        }
        Ok(())
    }

    /// Render a concise human-readable status summary.
    #[must_use]
    pub fn status_report(&self) -> String {
        let snap = self.hardware.snapshot();
        let mut out = String::new();
        let _ = writeln!(out, "VeilShell v{}", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(out, "  compositor : {}", self.compositor.backend_name());
        let _ = writeln!(
            out,
            "  mode       : {:?} (policy: {:?})",
            self.mode(),
            self.policy()
        );
        let _ = writeln!(
            out,
            "  orientation: {:?} (locked: {})",
            self.orientation(),
            self.rotation.is_locked()
        );
        let _ = writeln!(
            out,
            "  keyboard   : {}",
            if snap.keyboard_attached {
                "attached"
            } else {
                "detached"
            }
        );
        let _ = writeln!(
            out,
            "  battery    : {}%{}",
            snap.battery_pct,
            if snap.charging { " (charging)" } else { "" }
        );
        let _ = writeln!(
            out,
            "  radios     : wifi={} bt={} lte={}",
            snap.wifi_ssid
                .as_deref()
                .unwrap_or(if snap.wifi_connected { "on" } else { "off" }),
            if snap.bluetooth_enabled { "on" } else { "off" },
            snap.lte_signal_pct
                .map_or_else(|| "off".to_owned(), |pct| format!("{pct}%")),
        );
        let _ = writeln!(
            out,
            "  controls   : brightness={}% volume={}%{} night_light={}",
            snap.brightness_pct,
            snap.volume_pct,
            if snap.muted { " (muted)" } else { "" },
            if snap.night_light { "on" } else { "off" },
        );
        let _ = writeln!(
            out,
            "  theme      : {} (bg {}, fg {}, accent {}, WCAG AA: {})",
            self.palette.name,
            format_hex(self.palette.background),
            format_hex(self.palette.foreground),
            format_hex(self.palette.accent),
            if self.palette.meets_wcag_aa() {
                "pass"
            } else {
                "fail"
            },
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilshell_compositor::MockBackend;
    use veilshell_hw::{HardwareSnapshot, MockHardware};

    #[test]
    fn detaching_keyboard_switches_to_tablet_and_enables_rotation() {
        let compositor = MockBackend::default();
        let hardware = MockHardware::new(HardwareSnapshot {
            keyboard_attached: true,
            orientation: Orientation::Normal,
            ..HardwareSnapshot::default()
        });
        let mut shell = ShellState::new(compositor, hardware, ThemePalette::default_dark());
        assert_eq!(shell.mode(), ShellMode::Laptop);

        shell.hardware.state_mut().keyboard_attached = false;
        shell.hardware.state_mut().orientation = Orientation::Rotate90;
        shell.refresh_hardware().expect("refresh succeeds");

        assert_eq!(shell.mode(), ShellMode::Tablet);
        assert_eq!(shell.orientation(), Orientation::Rotate90);
        assert_eq!(shell.compositor.applied_mode(), ShellMode::Tablet);
        assert_eq!(
            shell.compositor.list_outputs().expect("outputs")[0].orientation,
            Orientation::Rotate90
        );
    }
}
