//! `veilshell` daemon binary (Phase 0 scaffold).

use std::env;

use veilshell_compositor::{HyprlandBackend, MockBackend};
use veilshell_hw::MockHardware;
use veilshell_render::{RendererBackend, SceneKind, SnapshotSpec, StubRenderer, ViewportSize};
use veilshell_shell::ShellState;
use veilshell_theme::load_best_available;
use veilshell_wayland::{LayerRole, LayerSurfaceSpec};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return;
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("veilshell {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let palette = load_best_available();
    let hypr_status = match HyprlandBackend::from_env() {
        Some(backend) if backend.is_available() => {
            format!("detected ({})", backend.socket_path().display())
        }
        Some(backend) => format!(
            "env set, socket missing ({})",
            backend.socket_path().display()
        ),
        None => "not detected (running offline / other compositor)".to_owned(),
    };

    let state = ShellState::new(MockBackend::default(), MockHardware::default(), palette);
    print!("{}", state.status_report());
    println!("  hyprland   : {hypr_status}");

    let pill_spec = LayerSurfaceSpec::for_role(LayerRole::BottomPill, 1280, 853);
    let mut renderer = StubRenderer;
    let snapshot = renderer.render_snapshot(
        &SnapshotSpec {
            scene: SceneKind::Pill,
            viewport: ViewportSize {
                width: pill_spec.width,
                height: pill_spec.height,
                scale: 1.0,
            },
            orientation: state.orientation(),
        },
        state.palette(),
    );
    if let Ok(frame) = snapshot {
        println!(
            "  renderer   : {} (verified {}x{} RGBA preview buffer)",
            renderer.name(),
            frame.width,
            frame.height
        );
    }
    println!("\nPhase 0 scaffold ready. Live wlr-layer-shell + wgpu loop lands in Phase 1.");
}

fn print_help() {
    println!(
        "veilshell {}\n\
         Ultra-lightweight pure-Rust Wayland tablet & touch shell.\n\n\
         USAGE:\n\
         \tveilshell [OPTIONS]\n\n\
         OPTIONS:\n\
         \t-h, --help       Print this help message\n\
         \t-V, --version    Print version information",
        env!("CARGO_PKG_VERSION")
    );
}
