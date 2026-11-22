//! `veilctl` control, inspection, live hardware telemetry, `.desktop` app browser, and offscreen `PNG` snapshot `CLI`.

use std::env;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use veilshell_compositor::{CompositorBackend, HyprlandBackend, MockBackend};
use veilshell_core::Orientation;
use veilshell_hw::{detect_dmi_device, HardwareBackend, LinuxHardware};
use veilshell_render::{
    ActiveRenderer, RendererBackend, RendererPreference, SceneContext, SceneKind, SnapshotSpec,
    ViewportSize, WallpaperImage,
};
use veilshell_shell::apps::{apps_to_tiles, current_clock_hhmm, discover_desktop_apps};
use veilshell_shell::ipc::{default_socket_path, send_daemon_command};
use veilshell_shell::session::ShellSession;
use veilshell_shell::ShellState;
use veilshell_theme::format_hex;

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
        "hw" => {
            cmd_hw();
            ExitCode::SUCCESS
        }
        "show" => cmd_show(args.get(1).map(String::as_str)),
        "wallpaper" => cmd_wallpaper(args.get(1).map(String::as_str)),
        "reload-theme" => cmd_daemon_simple("reload-theme"),
        "quit" => cmd_daemon_simple("quit"),
        "apps" => {
            cmd_apps();
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

fn cmd_daemon_simple(verb: &str) -> ExitCode {
    let sock = default_socket_path();
    match send_daemon_command(&sock, verb) {
        Ok(reply) => {
            print!("{reply}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!(
                "veilshell daemon is not reachable at {}: {err}",
                sock.display()
            );
            ExitCode::FAILURE
        }
    }
}

fn cmd_show(arg: Option<&str>) -> ExitCode {
    let Some(raw) = arg else {
        eprintln!("usage: veilctl show <pill|home|recents|shade|osk>");
        return ExitCode::FAILURE;
    };
    let Some(scene) = SceneKind::parse_name(raw) else {
        eprintln!("invalid scene '{raw}' (expected: pill, home, recents, shade, osk)");
        return ExitCode::FAILURE;
    };
    let sock = default_socket_path();
    match send_daemon_command(&sock, &format!("show {}", scene.as_str())) {
        Ok(reply) => {
            print!("{reply}");
            if reply.starts_with("err") {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            eprintln!(
                "veilshell daemon is not running at {}: {err}",
                sock.display()
            );
            ExitCode::FAILURE
        }
    }
}

fn cmd_wallpaper(arg: Option<&str>) -> ExitCode {
    let Some(path) = arg else {
        eprintln!("usage: veilctl wallpaper <PNG_PATH|clear>");
        return ExitCode::FAILURE;
    };
    let sock = default_socket_path();
    match send_daemon_command(&sock, &format!("wallpaper {path}")) {
        Ok(reply) => {
            print!("{reply}");
            if reply.starts_with("err") {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            eprintln!(
                "veilshell daemon is not running at {}: {err}",
                sock.display()
            );
            ExitCode::FAILURE
        }
    }
}

fn cmd_apps() {
    let apps = discover_desktop_apps();
    let mut out = std::io::stdout().lock();
    if writeln!(out, "Discovered {} desktop applications:", apps.len()).is_err() {
        return;
    }
    for app in &apps {
        if writeln!(out, "  {:<34} {:<24} -> {}", app.id, app.name, app.exec).is_err() {
            return;
        }
    }
}

fn cmd_hw() {
    let hw = LinuxHardware::auto_detect();
    let snap = hw.snapshot();
    println!("Hardware telemetry:");
    match detect_dmi_device() {
        Some((vendor, product)) => println!("  dmi        : {vendor} — {product}"),
        None => println!("  dmi        : unavailable"),
    }
    match hw.profile() {
        Some(p) => println!(
            "  profile    : {} (scale {:.2}x, kbd strategy: {}, auto-rotate: {})",
            p.id, p.preferred_scale, p.mode_strategy, p.auto_rotate_in_tablet_mode
        ),
        None => println!("  profile    : generic default"),
    }
    println!(
        "  keyboard   : {}",
        if snap.keyboard_attached {
            "attached"
        } else {
            "detached"
        }
    );
    println!(
        "  battery    : {}%{}",
        snap.battery_pct,
        if snap.charging { " (charging)" } else { "" }
    );
    println!("  backlight  : {}%", snap.brightness_pct);
    println!(
        "  volume     : {}%{}",
        snap.volume_pct,
        if snap.muted { " (muted)" } else { "" }
    );
    println!(
        "  wifi       : {}",
        snap.wifi_ssid.as_deref().unwrap_or(if snap.wifi_connected {
            "connected"
        } else {
            "off"
        })
    );
    println!(
        "  bluetooth  : {}",
        if snap.bluetooth_enabled { "on" } else { "off" }
    );
    println!(
        "  wwan/lte   : {} (modem present: {})",
        snap.lte_signal_pct
            .map_or_else(|| "disconnected".to_owned(), |s| format!("{s}%")),
        hw.has_wwan_hardware()
    );
    println!(
        "  night_light: {}",
        if snap.night_light { "on" } else { "off" }
    );
}

fn cmd_status() {
    let palette = ShellSession::current().palette();
    let hw = LinuxHardware::auto_detect();
    let profile_id = hw.profile().map(|p| p.id.clone());
    if let Some(hypr) = HyprlandBackend::from_env().filter(HyprlandBackend::is_available) {
        let outputs = hypr.list_outputs().unwrap_or_default();
        let windows = hypr.list_windows().unwrap_or_default();
        let state = ShellState::new(hypr, hw, palette);
        print!("{}", state.status_report());
        if let Some(id) = profile_id {
            println!("  profile    : {id}");
        }
        println!("  outputs    : {}", outputs.len());
        for o in outputs {
            println!(
                "               - {} ({}x{} @ {:.2}x, {:?})",
                o.name, o.width, o.height, o.scale, o.orientation
            );
        }
        println!("  windows    : {}", windows.len());
    } else {
        let state = ShellState::new(MockBackend::default(), hw, palette);
        print!("{}", state.status_report());
        if let Some(id) = profile_id {
            println!("  profile    : {id}");
        }
        println!("  hyprland   : not active in current session");
    }

    let sock = default_socket_path();
    if let Ok(reply) = send_daemon_command(&sock, "status") {
        print!("  {}", reply.trim_start());
    } else {
        println!("  daemon     : not running ({})", sock.display());
    }
}

fn cmd_theme() {
    let session = ShellSession::current();
    let palette = session.palette();
    let contrast = palette.foreground.contrast_ratio(palette.background);
    println!("Active theme: {}", palette.name);
    if let Some(path) = session.desktop_theme_path() {
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
    let Some(mode_arg @ ("auto" | "laptop" | "tablet" | "toggle")) = arg else {
        eprintln!("usage: veilctl mode <auto|laptop|tablet|toggle>");
        return ExitCode::FAILURE;
    };

    // Switching VeilShell's visibility must never fall back to changing the
    // host compositor when there is no daemon.
    let sock = default_socket_path();
    match send_daemon_command(&sock, &format!("mode {mode_arg}")) {
        Ok(reply) => {
            print!("{reply}");
            if reply.starts_with("err") || reply.trim().is_empty() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            eprintln!(
                "could not contact the VeilShell daemon at {}: {err}",
                sock.display()
            );
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::too_many_lines)]
fn cmd_snapshot(args: &[String]) -> ExitCode {
    let demo = args.iter().any(|a| a == "--demo");
    let power_menu = args.iter().any(|a| a == "--power-menu");
    let terminal_osk = args.iter().any(|a| a == "--terminal-osk");
    let scene_name = parse_flag(args, "--scene").unwrap_or_else(|| "home".to_owned());
    let Some(scene) = SceneKind::parse_name(&scene_name) else {
        eprintln!("invalid --scene '{scene_name}' (expected: pill, home, recents, shade, osk)");
        return ExitCode::FAILURE;
    };
    if power_menu && (!demo || scene != SceneKind::Shade) {
        eprintln!("--power-menu requires --demo --scene shade");
        return ExitCode::FAILURE;
    }
    if terminal_osk && (!demo || scene != SceneKind::Osk) {
        eprintln!("--terminal-osk requires --demo --scene osk");
        return ExitCode::FAILURE;
    }

    let default_h = if scene == SceneKind::Pill {
        veilshell_wayland::EDGE_HANDLE_DEPTH
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
    let renderer_pref = match parse_flag(args, "--renderer") {
        Some(raw) => {
            let Some(parsed) = RendererPreference::parse_name(&raw) else {
                eprintln!("invalid --renderer '{raw}' (expected: auto, wgpu, software)");
                return ExitCode::FAILURE;
            };
            parsed
        }
        None => RendererPreference::Auto,
    };
    let wallpaper = match parse_flag(args, "--wallpaper") {
        Some(path) => match WallpaperImage::load_png(&path) {
            Ok(img) => Some(img),
            Err(err) => {
                eprintln!("failed to load --wallpaper '{path}': {err}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let out_path = PathBuf::from(
        parse_flag(args, "--out").unwrap_or_else(|| format!("veilshell-{}.png", scene.as_str())),
    );

    let session = ShellSession::current();
    let palette = if demo {
        veilshell_core::ThemePalette::default_dark()
    } else {
        session.palette()
    };
    let mut ctx = SceneContext::default();
    if let Some(raw) = parse_flag(args, "--progress") {
        let Ok(progress) = raw.parse::<f32>() else {
            eprintln!("invalid --progress '{raw}'");
            return ExitCode::FAILURE;
        };
        if !(0.0..=1.0).contains(&progress) || !matches!(scene, SceneKind::Home | SceneKind::Shade)
        {
            eprintln!("--progress requires a value in 0..=1 and a Home or Shade snapshot");
            return ExitCode::FAILURE;
        }
        ctx.overlay_progress = progress;
    }
    if let Some(raw) = parse_flag(args, "--home-position") {
        let Ok(position) = raw.parse::<f32>() else {
            eprintln!("invalid --home-position '{raw}'");
            return ExitCode::FAILURE;
        };
        if !position.is_finite() || position < 0.0 || scene != SceneKind::Home {
            eprintln!("--home-position requires a nonnegative page position and a Home snapshot");
            return ExitCode::FAILURE;
        }
        ctx.home_position = Some(position);
    }
    if power_menu {
        ctx.power_menu = Some(veilshell_render::power::PowerMenu::Actions);
    }
    ctx.osk_state.terminal = terminal_osk;
    if !demo {
        let snap = LinuxHardware::auto_detect().snapshot();
        ctx.clock_text = current_clock_hhmm();
        ctx.battery_pct = snap.battery_pct;
        ctx.wifi_connected = snap.wifi_connected;
        ctx.bluetooth_enabled = snap.bluetooth_enabled;
        ctx.lte_connected = snap.lte_signal_pct.is_some();
        ctx.night_light = snap.night_light;
        ctx.brightness_pct = snap.brightness_pct;
        ctx.volume_pct = snap.volume_pct;
        veilshell_shell::shade::apply_hardware(&mut ctx, &snap);
        ctx.shade.auto_rotate_available = false;
        if scene == SceneKind::Shade {
            ctx.shade.quiet = None;
            veilshell_shell::shade::refresh_optional(&mut ctx);
        }
        ctx.home_apps = apps_to_tiles(&discover_desktop_apps());
    }

    let mut renderer = match ActiveRenderer::new(renderer_pref, ctx) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("renderer init failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(wp) = &wallpaper {
        if let Err(err) = renderer.set_wallpaper(Some(wp)) {
            eprintln!("wallpaper blur failed: {err}");
            return ExitCode::FAILURE;
        }
    }
    let backend_label = renderer.backend_label();
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
                "Saved {} snapshot ({}x{}, renderer: {}, theme: {}) -> {}",
                scene.as_str(),
                frame.width,
                frame.height,
                backend_label,
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
         \tstatus                            Show shell mode, theme, hardware, compositor, and daemon state\n\
         \thw                                Show live Linux hardware telemetry & matched device profile\n\
         \tshow <pill|home|recents|shade|osk> Switch the running daemon's active surface\n\
         \tmode <auto|laptop|tablet|toggle>  Change the running daemon's visibility policy\n\
         \twallpaper <PNG_PATH|clear>        Update or clear the GPU pre-blurred glass wallpaper\n\
         \treload-theme                      Reload colors.toml in the running daemon\n\
         \tquit                              Stop the running daemon cleanly\n\
         \tapps                              List discovered .desktop applications\n\
         \ttheme                             Inspect the resolved color palette and WCAG contrast\n\
         \tsnapshot [OPTIONS]                Render an offscreen glassmorphic PNG snapshot\n\
         \t  --scene <pill|home|recents|shade|osk> --renderer <auto|wgpu|software> [--wallpaper <PNG>]\n\
         \t  --width <PX> --height <PX> --out <PATH> [--demo]\n\
         \t  --power-menu | --terminal-osk    Preview a demo subview (shade or osk)\n\
         \t  --progress <0..1>                Home or Shade reveal progress\n\
         \t  --home-position <PAGE>           Fractional position in the Home pager\n\
         \thelp                              Print this help message",
        env!("CARGO_PKG_VERSION")
    );
}
