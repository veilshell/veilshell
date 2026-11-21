//! Compositor-agnostic traits plus reference `Hyprland` and mock backends.

use std::fmt;
use std::path::{Path, PathBuf};

use veilshell_core::{Orientation, ShellMode};

/// Opaque compositor window identifier.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WindowId(pub String);

/// Minimal window metadata needed by the recents deck and taskbar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowInfo {
    pub id: WindowId,
    pub title: String,
    pub app_id: String,
    pub workspace: String,
    pub focused: bool,
    pub floating: bool,
}

/// Physical output metadata reported by a compositor.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputInfo {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub orientation: Orientation,
}

/// Errors returned by a [`CompositorBackend`].
#[derive(Debug)]
pub enum CompositorError {
    /// No active compositor IPC socket was detected.
    NotConnected,
    /// Requested window or output does not exist.
    NotFound(String),
    /// Feature is not implemented yet for this backend.
    Unsupported(&'static str),
    /// Low-level transport error.
    Io(std::io::Error),
}

impl fmt::Display for CompositorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected => write!(f, "compositor backend is not connected"),
            Self::NotFound(id) => write!(f, "compositor target not found: {id}"),
            Self::Unsupported(op) => write!(f, "operation not supported by backend: {op}"),
            Self::Io(err) => write!(f, "compositor I/O error: {err}"),
        }
    }
}

impl std::error::Error for CompositorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CompositorError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Trait implemented by every Wayland compositor backend (`Hyprland`, `Niri`, `Sway`, etc.).
pub trait CompositorBackend {
    /// Short name of the compositor backend.
    fn backend_name(&self) -> &'static str;

    /// List open top-level windows across workspaces.
    ///
    /// # Errors
    /// Returns [`CompositorError`] if the compositor cannot be queried.
    fn list_windows(&self) -> Result<Vec<WindowInfo>, CompositorError>;

    /// Focus and raise a window by its [`WindowId`].
    ///
    /// # Errors
    /// Returns [`CompositorError::NotFound`] if the window does not exist.
    fn focus_window(&mut self, id: &WindowId) -> Result<(), CompositorError>;

    /// Gracefully request closing a window by its [`WindowId`].
    ///
    /// # Errors
    /// Returns [`CompositorError::NotFound`] if the window does not exist.
    fn close_window(&mut self, id: &WindowId) -> Result<(), CompositorError>;

    /// List connected outputs.
    ///
    /// # Errors
    /// Returns [`CompositorError`] if outputs cannot be enumerated.
    fn list_outputs(&self) -> Result<Vec<OutputInfo>, CompositorError>;

    /// Apply a display orientation to a named output.
    ///
    /// # Errors
    /// Returns [`CompositorError::NotFound`] if the output name is unknown.
    fn set_output_orientation(
        &mut self,
        output: &str,
        orientation: Orientation,
    ) -> Result<(), CompositorError>;

    /// Apply compositor layout rules appropriate for [`ShellMode`].
    ///
    /// # Errors
    /// Returns [`CompositorError`] if the mode transition fails.
    fn apply_shell_mode(&mut self, mode: ShellMode) -> Result<(), CompositorError>;
}

/// Deterministic in-memory backend used by unit tests and offline previews.
#[derive(Clone, Debug)]
pub struct MockBackend {
    windows: Vec<WindowInfo>,
    outputs: Vec<OutputInfo>,
    mode: ShellMode,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self {
            windows: vec![
                WindowInfo {
                    id: WindowId("0x1".into()),
                    title: "Terminal".into(),
                    app_id: "foot".into(),
                    workspace: "1".into(),
                    focused: true,
                    floating: false,
                },
                WindowInfo {
                    id: WindowId("0x2".into()),
                    title: "Browser".into(),
                    app_id: "chromium".into(),
                    workspace: "2".into(),
                    focused: false,
                    floating: false,
                },
            ],
            outputs: vec![OutputInfo {
                name: "eDP-1".into(),
                width: 1920,
                height: 1280,
                scale: 1.5,
                orientation: Orientation::Normal,
            }],
            mode: ShellMode::Laptop,
        }
    }
}

impl MockBackend {
    /// Current [`ShellMode`] applied to this mock compositor.
    #[must_use]
    pub const fn applied_mode(&self) -> ShellMode {
        self.mode
    }
}

impl CompositorBackend for MockBackend {
    fn backend_name(&self) -> &'static str {
        "mock"
    }

    fn list_windows(&self) -> Result<Vec<WindowInfo>, CompositorError> {
        Ok(self.windows.clone())
    }

    fn focus_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        if !self.windows.iter().any(|w| &w.id == id) {
            return Err(CompositorError::NotFound(id.0.clone()));
        }
        for window in &mut self.windows {
            window.focused = &window.id == id;
        }
        Ok(())
    }

    fn close_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        let before = self.windows.len();
        self.windows.retain(|w| &w.id != id);
        if self.windows.len() == before {
            return Err(CompositorError::NotFound(id.0.clone()));
        }
        Ok(())
    }

    fn list_outputs(&self) -> Result<Vec<OutputInfo>, CompositorError> {
        Ok(self.outputs.clone())
    }

    fn set_output_orientation(
        &mut self,
        output: &str,
        orientation: Orientation,
    ) -> Result<(), CompositorError> {
        let Some(target) = self.outputs.iter_mut().find(|o| o.name == output) else {
            return Err(CompositorError::NotFound(output.to_owned()));
        };
        target.orientation = orientation;
        Ok(())
    }

    fn apply_shell_mode(&mut self, mode: ShellMode) -> Result<(), CompositorError> {
        self.mode = mode;
        Ok(())
    }
}

/// `Hyprland` IPC backend (Phase 0 socket discovery; Phase 1 implements live IPC).
#[derive(Clone, Debug)]
pub struct HyprlandBackend {
    socket_path: PathBuf,
}

impl HyprlandBackend {
    /// Build a backend from explicit `XDG_RUNTIME_DIR` and `HYPRLAND_INSTANCE_SIGNATURE` values.
    #[must_use]
    pub fn from_parts(runtime_dir: impl AsRef<Path>, signature: &str) -> Self {
        let socket_path = runtime_dir
            .as_ref()
            .join("hypr")
            .join(signature)
            .join(".socket.sock");
        Self { socket_path }
    }

    /// Discover a running `Hyprland` instance from the process environment.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")?;
        let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        if signature.is_empty() {
            return None;
        }
        Some(Self::from_parts(PathBuf::from(runtime_dir), &signature))
    }

    /// Path to the `Hyprland` control Unix socket.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Whether the discovered socket currently exists on disk.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.socket_path.exists()
    }
}

impl CompositorBackend for HyprlandBackend {
    fn backend_name(&self) -> &'static str {
        "hyprland"
    }

    fn list_windows(&self) -> Result<Vec<WindowInfo>, CompositorError> {
        Err(CompositorError::Unsupported(
            "hyprland IPC list_windows arrives in Phase 1",
        ))
    }

    fn focus_window(&mut self, _id: &WindowId) -> Result<(), CompositorError> {
        Err(CompositorError::Unsupported(
            "hyprland IPC focus_window arrives in Phase 1",
        ))
    }

    fn close_window(&mut self, _id: &WindowId) -> Result<(), CompositorError> {
        Err(CompositorError::Unsupported(
            "hyprland IPC close_window arrives in Phase 1",
        ))
    }

    fn list_outputs(&self) -> Result<Vec<OutputInfo>, CompositorError> {
        Err(CompositorError::Unsupported(
            "hyprland IPC list_outputs arrives in Phase 1",
        ))
    }

    fn set_output_orientation(
        &mut self,
        _output: &str,
        _orientation: Orientation,
    ) -> Result<(), CompositorError> {
        Err(CompositorError::Unsupported(
            "hyprland IPC set_output_orientation arrives in Phase 1",
        ))
    }

    fn apply_shell_mode(&mut self, _mode: ShellMode) -> Result<(), CompositorError> {
        Err(CompositorError::Unsupported(
            "hyprland IPC apply_shell_mode arrives in Phase 1",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_backend_manages_windows_outputs_and_mode() {
        let mut backend = MockBackend::default();
        assert_eq!(backend.list_windows().expect("windows").len(), 2);

        backend
            .focus_window(&WindowId("0x2".into()))
            .expect("focus 0x2");
        let focused = backend
            .list_windows()
            .expect("windows")
            .into_iter()
            .find(|w| w.focused)
            .expect("one focused");
        assert_eq!(focused.id.0, "0x2");

        backend
            .close_window(&WindowId("0x1".into()))
            .expect("close 0x1");
        assert_eq!(backend.list_windows().expect("windows").len(), 1);

        backend
            .set_output_orientation("eDP-1", Orientation::Rotate90)
            .expect("rotate");
        assert_eq!(
            backend.list_outputs().expect("outputs")[0].orientation,
            Orientation::Rotate90
        );

        backend
            .apply_shell_mode(ShellMode::Tablet)
            .expect("tablet mode");
        assert_eq!(backend.applied_mode(), ShellMode::Tablet);
    }

    #[test]
    fn hyprland_socket_path_matches_convention() {
        let backend = HyprlandBackend::from_parts("/run/user/1000", "abc123");
        assert_eq!(
            backend.socket_path(),
            Path::new("/run/user/1000/hypr/abc123/.socket.sock")
        );
    }
}
