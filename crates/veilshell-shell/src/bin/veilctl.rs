//! `veilctl` control and inspection CLI for `VeilShell`.

use std::env;
use std::process::ExitCode;

use veilshell_compositor::{HyprlandBackend, MockBackend};
use veilshell_core::ModePolicy;
use veilshell_hw::MockHardware;
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
        other => {
            eprintln!("unknown subcommand: {other}\nRun `veilctl --help` for usage.");
            ExitCode::FAILURE
        }
    }
}

fn cmd_status() {
    let state = ShellState::new(
        MockBackend::default(),
        MockHardware::default(),
        load_best_available(),
    );
    print!("{}", state.status_report());
    if let Some(hypr) = HyprlandBackend::from_env() {
        println!(
            "  hyprland   : {} (available: {})",
            hypr.socket_path().display(),
            hypr.is_available()
        );
    } else {
        println!("  hyprland   : not active in current session");
    }
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
    let mut state = ShellState::new(
        MockBackend::default(),
        MockHardware::default(),
        load_best_available(),
    );

    let result = match arg {
        Some("auto") => state.set_policy(ModePolicy::Auto),
        Some("laptop") => state.set_policy(ModePolicy::ForceLaptop),
        Some("tablet") => state.set_policy(ModePolicy::ForceTablet),
        Some("toggle") => state.toggle_mode(),
        _ => {
            eprintln!("usage: veilctl mode <auto|laptop|tablet|toggle>");
            return ExitCode::FAILURE;
        }
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

fn print_help() {
    println!(
        "veilctl {}\n\
         Control and inspect a VeilShell session.\n\n\
         USAGE:\n\
         \tveilctl <COMMAND>\n\n\
         COMMANDS:\n\
         \tstatus                            Show shell mode, theme, and compositor status\n\
         \ttheme                             Inspect the resolved color palette and WCAG contrast\n\
         \tmode <auto|laptop|tablet|toggle>  Evaluate or switch the Laptop/Tablet mode policy\n\
         \thelp                              Print this help message",
        env!("CARGO_PKG_VERSION")
    );
}
