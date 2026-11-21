//! Compositor-agnostic traits plus live `Hyprland` Unix-socket IPC and mock backends.

use std::fmt;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
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
    /// Compositor returned an error reply or invalid JSON.
    Protocol(String),
    /// Low-level transport error.
    Io(std::io::Error),
}

impl fmt::Display for CompositorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected => write!(f, "compositor backend is not connected"),
            Self::NotFound(id) => write!(f, "compositor target not found: {id}"),
            Self::Unsupported(op) => write!(f, "operation not supported by backend: {op}"),
            Self::Protocol(msg) => write!(f, "compositor protocol error: {msg}"),
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
                    title: "Terminal — zsh".into(),
                    app_id: "foot".into(),
                    workspace: "1".into(),
                    focused: true,
                    floating: false,
                },
                WindowInfo {
                    id: WindowId("0x2".into()),
                    title: "VeilShell Architecture".into(),
                    app_id: "chromium".into(),
                    workspace: "2".into(),
                    focused: false,
                    floating: false,
                },
                WindowInfo {
                    id: WindowId("0x3".into()),
                    title: "Notes — Surface Go 3".into(),
                    app_id: "obsidian".into(),
                    workspace: "3".into(),
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

/// Live `Hyprland` Unix-socket IPC backend.
#[derive(Clone, Debug)]
pub struct HyprlandBackend {
    socket_path: PathBuf,
    timeout: Duration,
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
        Self {
            socket_path,
            timeout: Duration::from_secs(2),
        }
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

    /// Send a raw IPC request to `Hyprland`'s control socket and read the full reply.
    ///
    /// # Errors
    /// Returns [`CompositorError::NotConnected`] if the socket is missing, or [`CompositorError::Io`] on transport failure.
    pub fn send_request(&self, payload: &str) -> Result<String, CompositorError> {
        if !self.is_available() {
            return Err(CompositorError::NotConnected);
        }
        let mut stream = UnixStream::connect(&self.socket_path)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        stream.write_all(payload.as_bytes())?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response)
    }

    fn dispatch(&self, cmd: &str) -> Result<(), CompositorError> {
        let reply = self.send_request(&format!("dispatch {cmd}"))?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(CompositorError::Protocol(reply.trim().to_owned()))
        }
    }

    fn keyword(&self, key: &str, value: &str) -> Result<(), CompositorError> {
        let reply = self.send_request(&format!("keyword {key} {value}"))?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(CompositorError::Protocol(reply.trim().to_owned()))
        }
    }
}

#[derive(Deserialize)]
struct HyprWorkspaceRef {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HyprClient {
    address: String,
    title: String,
    class: String,
    workspace: HyprWorkspaceRef,
    floating: bool,
    #[serde(rename = "focusHistoryID", default = "default_focus_history")]
    focus_history_id: i32,
}

const fn default_focus_history() -> i32 {
    -1
}

#[derive(Deserialize)]
struct HyprMonitor {
    name: String,
    width: u32,
    height: u32,
    scale: f32,
    #[serde(default)]
    transform: u8,
}

/// Parse `hyprctl -j clients` JSON output into [`WindowInfo`] records.
///
/// # Errors
/// Returns [`CompositorError::Protocol`] if the JSON payload is invalid.
pub fn parse_hyprland_clients_json(raw: &str) -> Result<Vec<WindowInfo>, CompositorError> {
    let clients: Vec<HyprClient> =
        serde_json::from_str(raw).map_err(|e| CompositorError::Protocol(e.to_string()))?;
    Ok(clients
        .into_iter()
        .map(|c| WindowInfo {
            id: WindowId(c.address),
            title: c.title,
            app_id: c.class,
            workspace: c.workspace.name,
            focused: c.focus_history_id == 0,
            floating: c.floating,
        })
        .collect())
}

/// Parse `hyprctl -j monitors` JSON output into [`OutputInfo`] records.
///
/// # Errors
/// Returns [`CompositorError::Protocol`] if the JSON payload is invalid.
pub fn parse_hyprland_monitors_json(raw: &str) -> Result<Vec<OutputInfo>, CompositorError> {
    let monitors: Vec<HyprMonitor> =
        serde_json::from_str(raw).map_err(|e| CompositorError::Protocol(e.to_string()))?;
    Ok(monitors
        .into_iter()
        .map(|m| OutputInfo {
            name: m.name,
            width: m.width,
            height: m.height,
            scale: m.scale,
            orientation: orientation_from_hypr_transform(m.transform),
        })
        .collect())
}

const fn orientation_from_hypr_transform(transform: u8) -> Orientation {
    match transform % 4 {
        1 => Orientation::Rotate90,
        2 => Orientation::Rotate180,
        3 => Orientation::Rotate270,
        _ => Orientation::Normal,
    }
}

impl CompositorBackend for HyprlandBackend {
    fn backend_name(&self) -> &'static str {
        "hyprland"
    }

    fn list_windows(&self) -> Result<Vec<WindowInfo>, CompositorError> {
        let raw = self.send_request("j/clients")?;
        parse_hyprland_clients_json(&raw)
    }

    fn focus_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        self.dispatch(&format!("focuswindow address:{}", id.0))
    }

    fn close_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        self.dispatch(&format!("closewindow address:{}", id.0))
    }

    fn list_outputs(&self) -> Result<Vec<OutputInfo>, CompositorError> {
        let raw = self.send_request("j/monitors")?;
        parse_hyprland_monitors_json(&raw)
    }

    fn set_output_orientation(
        &mut self,
        output: &str,
        orientation: Orientation,
    ) -> Result<(), CompositorError> {
        let outputs = self.list_outputs()?;
        let Some(mon) = outputs.into_iter().find(|o| o.name == output) else {
            return Err(CompositorError::NotFound(output.to_owned()));
        };
        let transform = orientation.hyprland_transform();
        let rule = format!(
            "{output},preferred,auto,{:.2},transform,{transform}",
            mon.scale.max(0.5)
        );
        self.keyword("monitor", &rule)
    }

    fn apply_shell_mode(&mut self, mode: ShellMode) -> Result<(), CompositorError> {
        let batch = match mode {
            ShellMode::Tablet => {
                "[[BATCH]]keyword general:gaps_in 6;keyword general:gaps_out 10;keyword decoration:rounding 14"
            }
            ShellMode::Laptop => {
                "[[BATCH]]keyword general:gaps_in 4;keyword general:gaps_out 8;keyword decoration:rounding 8"
            }
        };
        let reply = self.send_request(batch)?;
        if reply.contains("ok") {
            Ok(())
        } else {
            Err(CompositorError::Protocol(reply.trim().to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_backend_manages_windows_outputs_and_mode() {
        let mut backend = MockBackend::default();
        assert_eq!(backend.list_windows().expect("windows").len(), 3);

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
        assert_eq!(backend.list_windows().expect("windows").len(), 2);

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

    #[test]
    fn parses_hyprland_clients_and_monitors_json() {
        let clients_json = r#"[
            {
                "address": "0x55a1bc",
                "title": "foot",
                "class": "foot",
                "workspace": { "id": 1, "name": "1" },
                "floating": false,
                "focusHistoryID": 0
            },
            {
                "address": "0x55a1ff",
                "title": "Chromium",
                "class": "chromium",
                "workspace": { "id": 2, "name": "2" },
                "floating": true,
                "focusHistoryID": 1
            }
        ]"#;
        let windows = parse_hyprland_clients_json(clients_json).expect("clients parsed");
        assert_eq!(windows.len(), 2);
        assert!(windows[0].focused);
        assert!(!windows[1].focused);
        assert!(windows[1].floating);
        assert_eq!(windows[1].app_id, "chromium");

        let monitors_json = r#"[
            {
                "name": "HEADLESS-2",
                "width": 1280,
                "height": 853,
                "scale": 1.5,
                "transform": 1
            }
        ]"#;
        let outputs = parse_hyprland_monitors_json(monitors_json).expect("monitors parsed");
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].name, "HEADLESS-2");
        assert_eq!(outputs[0].orientation, Orientation::Rotate90);
        assert!((outputs[0].scale - 1.5).abs() < 1e-4);
    }
}
