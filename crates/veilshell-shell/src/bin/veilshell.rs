//! `veilshell` multi-surface `Wayland` tablet & touch shell daemon.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use veilshell_compositor::{
    CompositorBackend, CompositorError, HyprlandBackend, MockBackend, OutputInfo, WindowId,
    WindowInfo,
};
use veilshell_core::{ModeEvent, ModePolicy, ModeStateMachine, Orientation, ShellMode};
use veilshell_hw::MockHardware;
use veilshell_render::{RecentsCard, SceneContext, SceneKind};
use veilshell_shell::apps::{apps_to_tiles, current_clock_hhmm, discover_desktop_apps};
use veilshell_shell::ipc::{default_socket_path, spawn_ipc_listener};
use veilshell_shell::ShellState;
use veilshell_theme::load_best_available;
use veilshell_wayland::channel;
use veilshell_wayland::{run_shell_daemon, ShellAction, ShellActionHandler, WaylandRuntimeConfig};

enum ActiveCompositor {
    Hyprland(HyprlandBackend),
    Mock(MockBackend),
}

impl CompositorBackend for ActiveCompositor {
    fn backend_name(&self) -> &'static str {
        match self {
            Self::Hyprland(b) => b.backend_name(),
            Self::Mock(b) => b.backend_name(),
        }
    }

    fn list_windows(&self) -> Result<Vec<WindowInfo>, CompositorError> {
        match self {
            Self::Hyprland(b) => b.list_windows(),
            Self::Mock(b) => b.list_windows(),
        }
    }

    fn focus_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        match self {
            Self::Hyprland(b) => b.focus_window(id),
            Self::Mock(b) => b.focus_window(id),
        }
    }

    fn close_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
        match self {
            Self::Hyprland(b) => b.close_window(id),
            Self::Mock(b) => b.close_window(id),
        }
    }

    fn list_outputs(&self) -> Result<Vec<OutputInfo>, CompositorError> {
        match self {
            Self::Hyprland(b) => b.list_outputs(),
            Self::Mock(b) => b.list_outputs(),
        }
    }

    fn set_output_orientation(
        &mut self,
        output: &str,
        orientation: Orientation,
    ) -> Result<(), CompositorError> {
        match self {
            Self::Hyprland(b) => b.set_output_orientation(output, orientation),
            Self::Mock(b) => b.set_output_orientation(output, orientation),
        }
    }

    fn apply_shell_mode(&mut self, mode: ShellMode) -> Result<(), CompositorError> {
        match self {
            Self::Hyprland(b) => b.apply_shell_mode(mode),
            Self::Mock(b) => b.apply_shell_mode(mode),
        }
    }

    fn switch_workspace_relative(&mut self, delta: i32) -> Result<(), CompositorError> {
        match self {
            Self::Hyprland(b) => b.switch_workspace_relative(delta),
            Self::Mock(b) => b.switch_workspace_relative(delta),
        }
    }

    fn launch_command(&self, command: &str) -> Result<(), CompositorError> {
        match self {
            Self::Hyprland(b) => b.launch_command(command),
            Self::Mock(b) => b.launch_command(command),
        }
    }
}

struct LiveActionHandler {
    compositor: ActiveCompositor,
    mode_sm: ModeStateMachine,
}

impl LiveActionHandler {
    fn refresh_context(&self, scene: SceneKind, ctx: &mut SceneContext) {
        ctx.clock_text = current_clock_hhmm();
        if scene == SceneKind::Recents {
            if let Ok(windows) = self.compositor.list_windows() {
                ctx.recents_cards = windows
                    .into_iter()
                    .map(|w| RecentsCard {
                        id: w.id.0,
                        app_id: if w.app_id.is_empty() {
                            "WINDOW".to_owned()
                        } else {
                            w.app_id
                        },
                        title: if w.title.is_empty() {
                            "UNTITLED".to_owned()
                        } else {
                            w.title
                        },
                        focused: w.focused,
                    })
                    .collect();
            }
        }
    }
}

impl ShellActionHandler for LiveActionHandler {
    fn on_action(&mut self, action: ShellAction, ctx: &mut SceneContext) {
        match action {
            ShellAction::RequestRefresh(scene) => self.refresh_context(scene, ctx),
            ShellAction::FocusWindow(id) => {
                if let Err(err) = self.compositor.focus_window(&WindowId(id)) {
                    eprintln!("veilshell: focus_window failed: {err}");
                }
            }
            ShellAction::CloseWindow(id) => {
                if let Err(err) = self.compositor.close_window(&WindowId(id)) {
                    eprintln!("veilshell: close_window failed: {err}");
                }
            }
            ShellAction::LaunchApp { exec } => {
                if let Err(err) = self.compositor.launch_command(&exec) {
                    eprintln!("veilshell: launch '{exec}' failed: {err}");
                }
            }
            ShellAction::SwitchWorkspace(delta) => {
                if let Err(err) = self.compositor.switch_workspace_relative(delta) {
                    eprintln!("veilshell: switch_workspace failed: {err}");
                }
            }
            ShellAction::ToggleSetting(id) => match id {
                "wifi" => ctx.wifi_connected = !ctx.wifi_connected,
                "bluetooth" => ctx.bluetooth_enabled = !ctx.bluetooth_enabled,
                "lte" => ctx.lte_connected = !ctx.lte_connected,
                "autorotate" => ctx.rotation_locked = !ctx.rotation_locked,
                "night_light" => ctx.night_light = !ctx.night_light,
                "tablet_mode" => {
                    if let Some(next_mode) = self.mode_sm.handle(ModeEvent::ToggleManual) {
                        let _ = self.compositor.apply_shell_mode(next_mode);
                    }
                    ctx.tablet_mode = self.mode_sm.effective_mode() == ShellMode::Tablet;
                }
                _ => {}
            },
            ShellAction::SetSlider { id, pct } => match id {
                "brightness" => ctx.brightness_pct = pct.min(100),
                "volume" => ctx.volume_pct = pct.min(100),
                _ => {}
            },
            ShellAction::HomePageChanged(page) => {
                ctx.home_page = page;
            }
        }
    }

    fn on_custom_command(&mut self, cmd: &str, ctx: &mut SceneContext) -> (String, Option<bool>) {
        let mut parts = cmd.split_whitespace();
        if parts.next() != Some("mode") {
            return (format!("err unknown command '{cmd}'"), None);
        }
        let event = match parts.next() {
            Some("auto") => ModeEvent::SetPolicy(ModePolicy::Auto),
            Some("laptop") => ModeEvent::SetPolicy(ModePolicy::ForceLaptop),
            Some("tablet") => ModeEvent::SetPolicy(ModePolicy::ForceTablet),
            Some("toggle") => ModeEvent::ToggleManual,
            _ => {
                return (
                    "err usage: mode <auto|laptop|tablet|toggle>".to_owned(),
                    None,
                )
            }
        };
        if let Some(next_mode) = self.mode_sm.handle(event) {
            if let Err(err) = self.compositor.apply_shell_mode(next_mode) {
                return (format!("err compositor rejected mode: {err}"), None);
            }
        }
        let mode = self.mode_sm.effective_mode();
        let policy = self.mode_sm.policy();
        let tablet = mode == ShellMode::Tablet;
        ctx.tablet_mode = tablet;
        (
            format!("Mode set to {mode:?} (policy: {policy:?})"),
            Some(tablet),
        )
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("veilshell {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let dry_run = args.iter().any(|a| a == "--dry-run");
    let scene = match parse_flag(&args, "--scene") {
        Some(raw) => {
            let Some(parsed) = SceneKind::parse_name(&raw) else {
                eprintln!("invalid --scene '{raw}' (expected: pill, home, recents, shade, osk)");
                return ExitCode::FAILURE;
            };
            parsed
        }
        None => SceneKind::Pill,
    };

    let output_name = parse_flag(&args, "--output");
    let socket_path = parse_flag(&args, "--socket").map_or_else(default_socket_path, PathBuf::from);

    let exit_after_ms = match parse_flag(&args, "--exit-after-ms") {
        Some(raw) => {
            let Ok(ms) = raw.parse::<u64>() else {
                eprintln!("invalid --exit-after-ms '{raw}'");
                return ExitCode::FAILURE;
            };
            Some(ms)
        }
        None => None,
    };

    let palette = load_best_available();
    let discovered = discover_desktop_apps();
    let mut scene_ctx = SceneContext {
        clock_text: current_clock_hhmm(),
        ..SceneContext::default()
    };
    if !discovered.is_empty() {
        scene_ctx.home_apps = apps_to_tiles(&discovered);
    }

    let compositor = match HyprlandBackend::from_env().filter(HyprlandBackend::is_available) {
        Some(hypr) => ActiveCompositor::Hyprland(hypr),
        None => ActiveCompositor::Mock(MockBackend::default()),
    };

    let handler = LiveActionHandler {
        compositor,
        mode_sm: ModeStateMachine::new(ModePolicy::ForceTablet, false),
    };
    handler.refresh_context(SceneKind::Recents, &mut scene_ctx);

    if dry_run || env::var_os("WAYLAND_DISPLAY").is_none() {
        let state = ShellState::new(MockBackend::default(), MockHardware::default(), palette);
        print!("{}", state.status_report());
        println!("  scene      : {}", scene.as_str());
        println!("  apps       : {}", scene_ctx.home_apps.len());
        println!("  windows    : {}", scene_ctx.recents_cards.len());
        println!("  socket     : {}", socket_path.display());
        if !dry_run {
            println!("  wayland    : WAYLAND_DISPLAY not set (use inside a Wayland session)");
        }
        return ExitCode::SUCCESS;
    }

    let (cmd_tx, cmd_rx) = channel::channel();
    if let Err(err) = spawn_ipc_listener(&socket_path, cmd_tx) {
        eprintln!(
            "veilshell: warning — could not bind IPC socket {}: {err}",
            socket_path.display()
        );
    }

    let config = WaylandRuntimeConfig {
        display_override: None,
        output_name,
        preferred_scale: 1.0,
        exit_after_ms,
    };

    let result = run_shell_daemon(&config, scene, palette, scene_ctx, cmd_rx, handler);
    let _ = fs::remove_file(&socket_path);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("veilshell: {err}");
            ExitCode::FAILURE
        }
    }
}

fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

fn print_help() {
    println!(
        "veilshell {}\n\
         Ultra-lightweight pure-Rust Wayland tablet & touch shell.\n\n\
         USAGE:\n\
         \tveilshell [OPTIONS]\n\n\
         OPTIONS:\n\
         \t--scene <pill|home|recents|shade|osk>  Initial surface scene (default: pill)\n\
         \t--output <NAME>                        Bind surfaces to a specific Wayland output\n\
         \t--socket <PATH>                        Override Unix control socket path\n\
         \t--exit-after-ms <MS>                   Exit cleanly after MS milliseconds (for CI tests)\n\
         \t--dry-run                              Print status without opening Wayland surfaces\n\
         \t-h, --help                             Print this help message\n\
         \t-V, --version                          Print version information",
        env!("CARGO_PKG_VERSION")
    );
}
