//! Pure-Rust core logic for `VeilShell` (zero I/O, zero GPU, zero Wayland).

pub mod apps;
pub mod gesture;
pub mod mode;
pub mod orientation;
pub mod spring;
pub mod theme;

pub use apps::{app_badge, parse_desktop_entry, strip_exec_field_codes, DesktopApp};
pub use gesture::{classify_bottom_pill, GestureAction, GestureSample, GestureThresholds};
pub use mode::{ModeEvent, ModePolicy, ModeStateMachine, ShellMode};
pub use orientation::{Orientation, RotationLock};
pub use spring::{Spring, SpringParams};
pub use theme::{Rgba, ThemePalette};
