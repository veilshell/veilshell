//! Compositor-agnostic traits plus live `Hyprland` Unix-socket IPC and mock backends.

mod overlay_gaps;

use std::fmt;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
    pub foreign_id: Option<String>,
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

    /// Mapped normal-workspace windows on the named touch navigation output, most recent first.
    /// A window is marked focused only if it is the active window on that output.
    ///
    /// # Errors
    /// Returns an error if the output or its windows cannot be queried.
    fn list_tablet_windows(&self, _output: &str) -> Result<Vec<WindowInfo>, CompositorError> {
        Err(CompositorError::Unsupported("Tablet window query"))
    }

    /// Safe top and bottom overlay strip depths in logical pixels, or zero on
    /// either edge that cannot be verified. Called only for opt-in gap overlays.
    fn overlay_gap_depths(&self) -> (u32, u32) {
        (0, 0)
    }

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

    /// Switch to the next (`delta > 0`) or previous (`delta < 0`) workspace.
    ///
    /// # Errors
    /// Returns [`CompositorError`] if the compositor rejects the workspace switch.
    fn switch_workspace_relative(&mut self, delta: i32) -> Result<(), CompositorError>;

    /// Ask the compositor to launch an application command in the active session environment.
    ///
    /// # Errors
    /// Returns [`CompositorError`] if launching fails.
    fn launch_command(&self, command: &str) -> Result<(), CompositorError>;
}

/// Deterministic in-memory backend used by unit tests and offline previews.
#[derive(Clone, Debug)]
pub struct MockBackend {
    windows: Vec<WindowInfo>,
    outputs: Vec<OutputInfo>,
    tablet_mru: Vec<WindowId>,
    mode: ShellMode,
    last_workspace_delta: i32,
    launched_commands: Arc<Mutex<Vec<String>>>,
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
                    foreign_id: None,
                },
                WindowInfo {
                    id: WindowId("0x2".into()),
                    title: "VeilShell Architecture".into(),
                    app_id: "chromium".into(),
                    workspace: "2".into(),
                    focused: false,
                    floating: false,
                    foreign_id: None,
                },
                WindowInfo {
                    id: WindowId("0x3".into()),
                    title: "Notes — Surface Go 3".into(),
                    app_id: "obsidian".into(),
                    workspace: "3".into(),
                    focused: false,
                    floating: false,
                    foreign_id: None,
                },
            ],
            outputs: vec![OutputInfo {
                name: "eDP-1".into(),
                width: 1920,
                height: 1280,
                scale: 1.5,
                orientation: Orientation::Normal,
            }],
            tablet_mru: Vec::new(),
            mode: ShellMode::Laptop,
            last_workspace_delta: 0,
            launched_commands: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockBackend {
    /// Current [`ShellMode`] applied to this mock compositor.
    #[must_use]
    pub const fn applied_mode(&self) -> ShellMode {
        self.mode
    }

    /// Last relative workspace delta applied to this mock compositor.
    #[must_use]
    pub const fn last_workspace_delta(&self) -> i32 {
        self.last_workspace_delta
    }

    /// Commands dispatched to [`CompositorBackend::launch_command`].
    #[must_use]
    pub fn launched_commands(&self) -> Vec<String> {
        self.launched_commands
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

impl CompositorBackend for MockBackend {
    fn backend_name(&self) -> &'static str {
        "mock"
    }

    fn list_windows(&self) -> Result<Vec<WindowInfo>, CompositorError> {
        Ok(self.windows.clone())
    }

    fn list_tablet_windows(&self, output: &str) -> Result<Vec<WindowInfo>, CompositorError> {
        // The mock's windows all belong to its single fixture output.
        if !self.outputs.iter().any(|o| o.name == output) {
            return Err(CompositorError::NotFound(output.to_owned()));
        }
        let mut windows = self.windows.clone();
        windows.sort_by_key(|window| {
            self.tablet_mru
                .iter()
                .position(|id| *id == window.id)
                .unwrap_or(usize::MAX)
        });
        Ok(windows)
    }

    fn focus_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        if !self.windows.iter().any(|w| &w.id == id) {
            return Err(CompositorError::NotFound(id.0.clone()));
        }
        for window in &mut self.windows {
            window.focused = &window.id == id;
        }
        self.tablet_mru.retain(|window| window != id);
        self.tablet_mru.insert(0, id.clone());
        Ok(())
    }

    fn close_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        let before = self.windows.len();
        self.windows.retain(|w| &w.id != id);
        if self.windows.len() == before {
            return Err(CompositorError::NotFound(id.0.clone()));
        }
        self.tablet_mru.retain(|window| window != id);
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

    fn switch_workspace_relative(&mut self, delta: i32) -> Result<(), CompositorError> {
        self.last_workspace_delta = delta;
        Ok(())
    }

    fn launch_command(&self, command: &str) -> Result<(), CompositorError> {
        if let Ok(mut guard) = self.launched_commands.lock() {
            guard.push(command.to_owned());
        }
        Ok(())
    }
}

/// Live `Hyprland` Unix-socket IPC backend.
#[derive(Clone, Debug)]
pub struct HyprlandBackend {
    socket_path: PathBuf,
    timeout: Duration,
    overlay_gaps: Option<overlay_gaps::GapQuery>,
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
            overlay_gaps: None,
        }
    }

    /// Enable sizing queries and restrict touch navigation to the active normal
    /// workspace on the gesture output. In the overlay even a window focus
    /// must not intentionally switch workspaces.
    #[must_use]
    pub fn with_overlay_gap_gestures(mut self) -> Self {
        self.overlay_gaps.get_or_insert_with(Default::default);
        self
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

    fn dispatch_lua_or_legacy(&self, lua: &str, legacy: &str) -> Result<(), CompositorError> {
        if self.eval_lua(lua).is_ok() {
            return Ok(());
        }
        self.dispatch(legacy)
    }

    fn dispatch(&self, cmd: &str) -> Result<(), CompositorError> {
        let reply = self.send_request(&format!("dispatch {cmd}"))?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(CompositorError::Protocol(reply.trim().to_owned()))
        }
    }

    fn eval_lua(&self, lua: &str) -> Result<(), CompositorError> {
        let reply = self.send_request(&format!("eval {lua}"))?;
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
    id: Option<i64>,
    // 0.57+ distinguishes an addressable name from the display name.
    address: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

impl HyprWorkspaceRef {
    fn is_normal(&self) -> bool {
        match self.kind.as_deref() {
            Some(kind) => kind == "normal",
            // Pre-0.57 JSON has no workspace type. Named workspaces may have
            // negative IDs too, but the special workspace range is -99..=-2.
            None => {
                !self.name.is_empty()
                    && self.name != "special"
                    && !self.name.starts_with("special:")
                    && self.id.is_none_or(|id| !(-99..=-1).contains(&id))
            }
        }
    }

    fn is_same_workspace(&self, other: &Self) -> bool {
        self.id == other.id
            && match (&self.address, &other.address) {
                (Some(a), Some(b)) => !a.is_empty() && a == b,
                (None, None) => self.id.is_some_and(|id| id != 0),
                _ => false,
            }
    }

    fn is_empty_special_workspace(&self) -> bool {
        // v0.56: {id: 0, name: ""}; v0.57: {address: "", type: "", name: ""}.
        self.name.is_empty()
            && match &self.address {
                Some(address) => address.is_empty() && self.kind.as_deref() == Some(""),
                None => self.id == Some(0),
            }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HyprClient {
    address: String,
    title: String,
    class: String,
    workspace: HyprWorkspaceRef,
    floating: bool,
    mapped: Option<bool>,
    hidden: Option<bool>,
    accepts_input: Option<bool>,
    #[serde(rename = "focusHistoryID", default = "default_focus_history")]
    focus_history_id: i32,
    #[serde(default)]
    stable_id: Option<String>,
    monitor: Option<i64>,
}

impl HyprClient {
    fn into_window(self, focused: bool) -> WindowInfo {
        WindowInfo {
            id: WindowId(self.address),
            title: self.title,
            app_id: self.class,
            workspace: self.workspace.name,
            focused,
            floating: self.floating,
            foreign_id: self.stable_id,
        }
    }
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HyprTabletMonitor {
    name: String,
    id: Option<i64>,
    #[serde(default)]
    disabled: bool,
    active_workspace: Option<HyprWorkspaceRef>,
    special_workspace: Option<HyprWorkspaceRef>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HyprActiveWindow {
    // An empty object is a valid reply when no mapped window is active.
    address: Option<String>,
    monitor: Option<i64>,
    workspace: Option<HyprWorkspaceRef>,
    stable_id: Option<String>,
}

impl HyprActiveWindow {
    fn matches(&self, client: &HyprClient) -> bool {
        self.address.as_ref() == Some(&client.address)
            && self.monitor == client.monitor
            && self.stable_id == client.stable_id
            && self
                .workspace
                .as_ref()
                .is_some_and(HyprWorkspaceRef::is_normal)
    }
}

/// Parse `hyprctl -j clients` JSON output into [`WindowInfo`] records sorted by recency.
///
/// Unmapped helper surfaces are filtered out and negative `focusHistoryID` values are
/// ordered after recently focused windows.
///
/// # Errors
/// Returns [`CompositorError::Protocol`] if the JSON payload is invalid.
pub fn parse_hyprland_clients_json(raw: &str) -> Result<Vec<WindowInfo>, CompositorError> {
    let mut clients: Vec<HyprClient> =
        serde_json::from_str(raw).map_err(|e| CompositorError::Protocol(e.to_string()))?;
    clients.retain(|c| c.mapped.unwrap_or(true));
    clients.sort_by_key(|c| {
        if c.focus_history_id < 0 {
            i32::MAX
        } else {
            c.focus_history_id
        }
    });
    Ok(clients
        .into_iter()
        .map(|c| {
            let focused = c.focus_history_id == 0;
            c.into_window(focused)
        })
        .collect())
}

fn tablet_monitor(raw: &str, output: &str) -> Result<(i64, HyprTabletMonitor), CompositorError> {
    let monitors: Vec<HyprTabletMonitor> =
        serde_json::from_str(raw).map_err(|e| CompositorError::Protocol(e.to_string()))?;
    monitors
        .into_iter()
        .find(|m| m.name == output && !m.disabled)
        .and_then(|m| m.id.filter(|id| *id >= 0).map(|id| (id, m)))
        .ok_or_else(|| CompositorError::NotFound(output.to_owned()))
}

#[cfg(test)]
fn tablet_monitor_id(raw: &str, output: &str) -> Result<i64, CompositorError> {
    tablet_monitor(raw, output).map(|(id, _)| id)
}

fn parse_tablet_snapshot(raw: &str, output: &str) -> Result<Vec<WindowInfo>, CompositorError> {
    parse_output_snapshot(raw, output, false)
}

fn parse_output_snapshot(
    raw: &str,
    output: &str,
    overlay: bool,
) -> Result<Vec<WindowInfo>, CompositorError> {
    let mut replies = raw.splitn(3, "\n\n\n");
    let (Some(monitors_json), Some(clients_json), Some(active_json)) =
        (replies.next(), replies.next(), replies.next())
    else {
        return Err(CompositorError::Protocol(
            "incomplete Tablet window snapshot".into(),
        ));
    };
    let (monitor, topology) = tablet_monitor(monitors_json, output)?;
    let overlay_workspace = if overlay {
        let Some(active) = topology
            .active_workspace
            .filter(HyprWorkspaceRef::is_normal)
        else {
            return Ok(Vec::new());
        };
        if !topology
            .special_workspace
            .is_some_and(|ws| ws.is_empty_special_workspace())
        {
            // Focusing a normal client can dismiss a special workspace.
            return Ok(Vec::new());
        }
        Some(active)
    } else {
        None
    };
    let mut clients: Vec<HyprClient> =
        serde_json::from_str(clients_json).map_err(|e| CompositorError::Protocol(e.to_string()))?;
    let active: HyprActiveWindow =
        serde_json::from_str(active_json).map_err(|e| CompositorError::Protocol(e.to_string()))?;
    clients.retain(|c| {
        c.mapped == Some(true)
            && c.monitor == Some(monitor)
            && c.workspace.is_normal()
            && !c.address.is_empty()
            && c.address != "0x0"
            && overlay_workspace.as_ref().is_none_or(|active| {
                c.workspace.is_same_workspace(active)
                    && c.hidden != Some(true)
                    && c.accepts_input != Some(false)
            })
    });
    // Rank 0 denotes most recent, not necessarily currently focused. Anchor
    // on activewindow when it matches; put unranked clients after all ranks
    // with a consistent tie-break independent of the IPC enumeration order.
    let key = |c: &HyprClient| {
        (
            !active.matches(c),
            c.focus_history_id < 0,
            c.focus_history_id.max(0),
        )
    };
    clients.sort_by(|a, b| key(a).cmp(&key(b)).then_with(|| a.address.cmp(&b.address)));
    Ok(clients
        .into_iter()
        .map(|c| {
            let focused = active.matches(&c);
            c.into_window(focused)
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

    fn list_tablet_windows(&self, output: &str) -> Result<Vec<WindowInfo>, CompositorError> {
        // Read the topology, recency and focus together in one IPC request.
        let raw = self.send_request("[[BATCH]]j/monitors;j/clients;j/activewindow")?;
        if self.overlay_gaps.is_some() {
            parse_output_snapshot(&raw, output, true)
        } else {
            parse_tablet_snapshot(&raw, output)
        }
    }

    fn overlay_gap_depths(&self) -> (u32, u32) {
        self.overlay_gaps
            .as_ref()
            .map_or((0, 0), |query| query.depths(&self.socket_path))
    }

    fn focus_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        let escaped = lua_escape(&id.0);
        self.dispatch_lua_or_legacy(
            &format!("hl.dispatch(hl.dsp.focus({{ window = \"address:{escaped}\" }}))"),
            &format!("focuswindow address:{}", id.0),
        )
    }

    fn close_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        let escaped = lua_escape(&id.0);
        self.dispatch_lua_or_legacy(
            &format!("hl.dispatch(hl.dsp.window.close({{ window = \"address:{escaped}\" }}))"),
            &format!("closewindow address:{}", id.0),
        )
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
        if !output
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err(CompositorError::Protocol("unsafe output name".into()));
        }
        let outputs = self.list_outputs()?;
        let Some(mon) = outputs.into_iter().find(|o| o.name == output) else {
            return Err(CompositorError::NotFound(output.to_owned()));
        };
        let transform = orientation.hyprland_transform();
        let scale = mon.scale.max(0.5);
        let lua = format!(
            "hl.monitor({{ output = \"{output}\", mode = \"preferred\", position = \"auto\", scale = {scale:.2}, transform = {transform} }})"
        );
        if self.eval_lua(&lua).is_ok() {
            return Ok(());
        }
        let rule = format!("{output},preferred,auto,{scale:.2},transform,{transform}");
        self.keyword("monitor", &rule)
    }

    fn apply_shell_mode(&mut self, mode: ShellMode) -> Result<(), CompositorError> {
        let lua = match mode {
            ShellMode::Tablet => {
                "hl.config({ general = { layout = \"monocle\", gaps_in = 0, gaps_out = 0, border_size = 0 }, decoration = { rounding = 0 } })"
            }
            ShellMode::Laptop => {
                "hl.config({ general = { layout = \"dwindle\", gaps_in = 4, gaps_out = 8, border_size = 1 }, decoration = { rounding = 8 } })"
            }
        };
        if self.eval_lua(lua).is_ok() {
            return Ok(());
        }
        let batch = match mode {
            ShellMode::Tablet => {
                "[[BATCH]]keyword general:layout monocle;keyword general:gaps_in 0;keyword general:gaps_out 0;keyword general:border_size 0;keyword decoration:rounding 0"
            }
            ShellMode::Laptop => {
                "[[BATCH]]keyword general:layout dwindle;keyword general:gaps_in 4;keyword general:gaps_out 8;keyword general:border_size 1;keyword decoration:rounding 8"
            }
        };
        let reply = self.send_request(batch)?;
        if reply.contains("ok") {
            Ok(())
        } else {
            Err(CompositorError::Protocol(reply.trim().to_owned()))
        }
    }

    fn switch_workspace_relative(&mut self, delta: i32) -> Result<(), CompositorError> {
        match delta.cmp(&0) {
            std::cmp::Ordering::Greater => self.dispatch_lua_or_legacy(
                "hl.dispatch(hl.dsp.focus({ workspace = \"e+1\" }))",
                "workspace e+1",
            ),
            std::cmp::Ordering::Less => self.dispatch_lua_or_legacy(
                "hl.dispatch(hl.dsp.focus({ workspace = \"e-1\" }))",
                "workspace e-1",
            ),
            std::cmp::Ordering::Equal => Ok(()),
        }
    }

    fn launch_command(&self, command: &str) -> Result<(), CompositorError> {
        let escaped = lua_escape(command);
        self.dispatch_lua_or_legacy(
            &format!("hl.dispatch(hl.dsp.exec_cmd(\"{escaped}\"))"),
            &format!("exec {command}"),
        )
    }
}

fn lua_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tablet_tests;

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

        backend.switch_workspace_relative(1).expect("ws next");
        assert_eq!(backend.last_workspace_delta(), 1);
        backend.launch_command("foot").expect("launch");
        assert_eq!(backend.launched_commands(), vec!["foot".to_owned()]);
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
                "address": "0x55a1ff",
                "title": "Chromium",
                "class": "chromium",
                "workspace": { "id": 2, "name": "2" },
                "floating": true,
                "mapped": true,
                "focusHistoryID": 2
            },
            {
                "address": "0x55a100",
                "title": "HiddenHelper",
                "class": "helper",
                "workspace": { "id": 1, "name": "1" },
                "floating": true,
                "mapped": false,
                "focusHistoryID": 1
            },
            {
                "address": "0x55a1bc",
                "title": "foot",
                "class": "foot",
                "workspace": { "id": 1, "name": "1" },
                "floating": false,
                "mapped": true,
                "focusHistoryID": 0,
                "stableId": "1800000c"
            }
        ]"#;
        let windows = parse_hyprland_clients_json(clients_json).expect("clients parsed");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].id.0, "0x55a1bc");
        assert!(windows[0].focused);
        assert_eq!(windows[0].foreign_id.as_deref(), Some("1800000c"));
        assert!(!windows[1].focused);
        assert!(windows[1].floating);
        assert_eq!(windows[1].foreign_id, None);
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

    #[test]
    fn lua_escape_handles_quotes_and_backslashes() {
        assert_eq!(
            lua_escape("chromium --app=\"https://example.com\\path\""),
            "chromium --app=\\\"https://example.com\\\\path\\\""
        );
    }
}
