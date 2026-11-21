//! `veilctl` control, inspection, and offscreen PNG snapshot CLI for `VeilShell`.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use veilshell_compositor::{CompositorBackend, HyprlandBackend, MockBackend};
use veilshell_core::{ModePolicy, Orientation};
use veilshell_hw::MockHardware;
use veilshell_render::{
    RendererBackend, SceneContext, SceneKind, SdfSoftwareRenderer, SnapshotSpec, ViewportSize,
};
use veilshell_shell::ShellState;
use veilshell_theme::{format_hex, load_best_available, OmarchyAetherProvider};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        print_help();
        return ExitCode::SUCCESS;
    };

    match cmd {
        "-h" | "--help" | "help" => {
            print_help();
            ExitCode::SUCCESS
        }
        "-V" | "--version" => {
            println!("veilctl {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "status" => {
            cmd_status();
            ExitCode::SUCCESS
        }
        "theme" => {
            cmd_theme();
            ExitCode::SUCCESS
        }
        "mode" => cmd_mode(args.get(1).map(String::as_str)),
        "snapshot" => cmd_snapshot(&args[1..]),
        other => {
            eprintln!("unknown subcommand: {other}\nRun `veilctl --help` for usage.");
            ExitCode::FAILURE
        }
    }
}

fn cmd_status() {
    let palette = load_best_available();
    if let Some(hypr) = HyprlandBackend::from_env().filter(HyprlandBackend::is_available) {
        let outputs = hypr.list_outputs().unwrap_or_default();
        let windows = hypr.list_windows().unwrap_or_default();
        let state = ShellState::new(hypr, MockHardware::default(), palette);
        print!("{}", state.status_report());
        println!("  outputs    : {}", outputs.len());
        for o in outputs {
            println!(
                "               - {} ({}x{} @ {:.2}x, {:?})",
                o.name, o.width, o.height, o.scale, o.orientation
            );
        }
        println!("  windows    : {}", windows.len());
        return;
    }

    let state = ShellState::new(MockBackend::default(), MockHardware::default(), palette);
    print!("{}", state.status_report());
    println!("  hyprland   : not active in current session");
}

fn cmd_theme() {
    let palette = load_best_available();
    let contrast = palette.foreground.contrast_ratio(palette.background);
    println!("Active theme: {}", palette.name);
    if let Some(path) = OmarchyAetherProvider::default_path() {
        println!(
            "  source     : {} ({})",
            path.display(),
            if path.exists() {
                "found"
            } else {
                "not found, using fallback"
            }
        );
    }
    println!("  background : {}", format_hex(palette.background));
    println!(
        "  surface    : {} (alpha {:.2})",
        format_hex(palette.surface),
        palette.surface.a
    );
    println!("  foreground : {}", format_hex(palette.foreground));
    println!("  muted      : {}", format_hex(palette.muted));
    println!("  accent     : {}", format_hex(palette.accent));
    println!("  error      : {}", format_hex(palette.error));
    println!(
        "  contrast   : {contrast:.2}:1 (WCAG AA: {})",
        if palette.meets_wcag_aa() {
            "pass"
        } else {
            "fail"
        }
    );
}

fn cmd_mode(arg: Option<&str>) -> ExitCode {
    let policy_action = match arg {
        Some("auto") => Ok(Some(ModePolicy::Auto)),
        Some("laptop") => Ok(Some(ModePolicy::ForceLaptop)),
        Some("tablet") => Ok(Some(ModePolicy::ForceTablet)),
        Some("toggle") => Ok(None),
        _ => Err("usage: veilctl mode <auto|laptop|tablet|toggle>"),
    };

    let Ok(action) = policy_action else {
        eprintln!("usage: veilctl mode <auto|laptop|tablet|toggle>");
        return ExitCode::FAILURE;
    };

    let palette = load_best_available();
    if let Some(hypr) = HyprlandBackend::from_env().filter(HyprlandBackend::is_available) {
        let mut state = ShellState::new(hypr, MockHardware::default(), palette);
        let result = match action {
            Some(p) => state.set_policy(p),
            None => state.toggle_mode(),
        };
        return match result {
            Ok(mode) => {
                println!(
                    "Applied {:?} on live Hyprland (policy: {:?})",
                    mode,
                    state.policy()
                );
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("failed to apply mode on Hyprland: {err}");
                ExitCode::FAILURE
            }
        };
    }

    let mut state = ShellState::new(MockBackend::default(), MockHardware::default(), palette);
    let result = match action {
        Some(p) => state.set_policy(p),
        None => state.toggle_mode(),
    };
    match result {
        Ok(mode) => {
            println!("Mode set to {:?} (policy: {:?})", mode, state.policy());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("failed to switch mode: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_snapshot(args: &[String]) -> ExitCode {
    let scene_name = parse_flag(args, "--scene").unwrap_or_else(|| "home".to_owned());
    let Some(scene) = SceneKind::parse_name(&scene_name) else {
        eprintln!("invalid --scene '{scene_name}' (expected: pill, home, recents, shade, osk)");
        return ExitCode::FAILURE;
    };

    let default_h = if scene == SceneKind::Pill {
        28_u32
    } else if scene == SceneKind::Osk {
        284_u32
    } else {
        853_u32
    };
    let width: u32 = match parse_flag(args, "--width") {
        Some(raw) => {
            let Ok(v) = raw.parse() else {
                eprintln!("invalid --width '{raw}'");
                return ExitCode::FAILURE;
            };
            v
        }
        None => 1280,
    };
    let height: u32 = match parse_flag(args, "--height") {
        Some(raw) => {
            let Ok(v) = raw.parse() else {
                eprintln!("invalid --height '{raw}'");
                return ExitCode::FAILURE;
            };
            v
        }
        None => default_h,
    };
    let scale: f32 = match parse_flag(args, "--scale") {
        Some(raw) => {
            let Ok(v) = raw.parse() else {
                eprintln!("invalid --scale '{raw}'");
                return ExitCode::FAILURE;
            };
            v
        }
        None => 1.0,
    };
    let out_path = PathBuf::from(
        parse_flag(args, "--out").unwrap_or_else(|| format!("veilshell-{}.png", scene.as_str())),
    );

    let palette = load_best_available();
    let mut renderer = SdfSoftwareRenderer::with_context(SceneContext::default());
    let spec = SnapshotSpec {
        scene,
        viewport: ViewportSize {
            width,
            height,
            scale,
        },
        orientation: Orientation::Normal,
    };

    match renderer
        .render_snapshot(&spec, &palette)
        .and_then(|frame| frame.save_png(&out_path).map(|()| frame))
    {
        Ok(frame) => {
            println!(
                "Saved {} snapshot ({}x{}, theme: {}) -> {}",
                scene.as_str(),
                frame.width,
                frame.height,
                palette.name,
                out_path.display()
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("snapshot failed: {err}");
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
        "veilctl {}\n\
         Control, inspect, and snapshot a VeilShell session.\n\n\
         USAGE:\n\
         \tveilctl <COMMAND>\n\n\
         COMMANDS:\n\
         \tstatus                            Show shell mode, theme, and live compositor status\n\
         \ttheme                             Inspect the resolved color palette and WCAG contrast\n\
         \tmode <auto|laptop|tablet|toggle>  Evaluate or apply the Laptop/Tablet mode policy\n\
         \tsnapshot [OPTIONS]                Render an offscreen glassmorphic PNG snapshot\n\
         \t  --scene <pill|home|recents|shade|osk>\n\
         \t  --width <PX> --height <PX> --scale <F32> --out <PATH>\n\
         \thelp                              Print this help message",
        env!("CARGO_PKG_VERSION")
    );
}
