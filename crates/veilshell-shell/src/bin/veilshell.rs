//! `veilshell` `Wayland` tablet & touch shell daemon.

use std::env;
use std::process::ExitCode;

use veilshell_compositor::{CompositorBackend, HyprlandBackend, MockBackend};
use veilshell_hw::MockHardware;
use veilshell_render::{SceneContext, SceneKind};
use veilshell_shell::ShellState;
use veilshell_theme::load_best_available;
use veilshell_wayland::{run_layer_surface, LayerRole, WaylandRuntimeConfig};

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
    let mut scene_ctx = SceneContext::default();

    if let Some(hypr) = HyprlandBackend::from_env().filter(HyprlandBackend::is_available) {
        if let Ok(windows) = hypr.list_windows() {
            if !windows.is_empty() {
                scene_ctx.recents_titles = windows
                    .into_iter()
                    .map(|w| (w.app_id.to_uppercase(), w.title.to_uppercase(), w.focused))
                    .collect();
            }
        }
    }

    if dry_run || env::var_os("WAYLAND_DISPLAY").is_none() {
        let state = ShellState::new(MockBackend::default(), MockHardware::default(), palette);
        print!("{}", state.status_report());
        println!("  scene      : {}", scene.as_str());
        if !dry_run {
            println!("  wayland    : WAYLAND_DISPLAY not set (use inside a Wayland session)");
        }
        return ExitCode::SUCCESS;
    }

    let config = WaylandRuntimeConfig {
        display_override: None,
        preferred_scale: 1.0,
        exit_after_ms,
    };

    match run_layer_surface(&config, LayerRole::from_scene(scene), palette, scene_ctx) {
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
         \t--scene <pill|home|recents|shade|osk>  Surface scene to present (default: pill)\n\
         \t--exit-after-ms <MS>                   Exit cleanly after MS milliseconds (for CI tests)\n\
         \t--dry-run                              Print status without opening a Wayland surface\n\
         \t-h, --help                             Print this help message\n\
         \t-V, --version                          Print version information",
        env!("CARGO_PKG_VERSION")
    );
}
