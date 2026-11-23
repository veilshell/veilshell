//! `veilctl` control, inspection, live hardware telemetry, `.desktop` app browser, and offscreen `PNG` snapshot `CLI`.

use std::env;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;

use veilshell_compositor::{CompositorBackend, HyprlandBackend, MockBackend};
use veilshell_core::Orientation;
use veilshell_hw::{detect_dmi_device, HardwareBackend, LinuxHardware};
use veilshell_render::{
    tablet_shade_is_split, ActiveRenderer, RendererBackend, RendererPreference, RgbaImage,
    SceneContext, SceneKind, SettingsView, SnapshotSpec, TabletNotifications, TabletOverviewOrigin,
    TabletShadePage, TabletStatus, ViewportSize, WallpaperImage, WallpaperPreset,
};
use veilshell_shell::apps::{
    apps_to_tiles, current_clock_hhmm, current_tablet_clock, discover_desktop_apps,
};
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
        eprintln!("usage: veilctl show <pill|home|drawer|launcher|settings|recents|shade|osk>");
        return ExitCode::FAILURE;
    };
    let Some(scene) = SceneKind::parse_name(raw) else {
        eprintln!("invalid scene '{raw}'");
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
    let Some(mode_arg @ ("auto" | "laptop" | "tablet" | "desk" | "toggle")) = arg else {
        eprintln!("usage: veilctl mode <auto|laptop|tablet|desk|toggle>");
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
    let tablet_home = args.iter().any(|a| a == "--tablet-home");
    let (scene, viewport, tablet_overview, progress) = match snapshot_preview(args) {
        Ok(preview) => preview,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    if power_menu && (!demo || scene != SceneKind::Shade) {
        eprintln!("--power-menu requires --demo --scene shade");
        return ExitCode::FAILURE;
    }
    if terminal_osk && (!demo || scene != SceneKind::Osk) {
        eprintln!("--terminal-osk requires --demo --scene osk");
        return ExitCode::FAILURE;
    }
    if tablet_home && scene != SceneKind::Home {
        eprintln!("--tablet-home requires --scene home");
        return ExitCode::FAILURE;
    }

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
    let mut wallpaper = match parse_flag(args, "--wallpaper") {
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
    let mut ctx = SceneContext {
        overlay_progress: progress,
        tablet_overview,
        ..SceneContext::default()
    };
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
    let tablet_shade = match tablet_shade_preview(
        args,
        &mut ctx,
        scene,
        demo,
        (viewport.width, viewport.height),
    ) {
        Ok(tablet) => tablet,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let tablet_home_bg = tablet_home
        || scene == SceneKind::Drawer
        || tablet_overview == Some(TabletOverviewOrigin::Home);
    let tablet_layout = tablet_home_bg || tablet_shade || tablet_overview.is_some();
    if tablet_home_bg {
        ctx.session_wallpaper = wallpaper
            .take()
            .and_then(|image| RgbaImage::new(image.width, image.height, image.rgba))
            .map(Arc::new);
    }
    if tablet_layout {
        ctx.power_menu
            .get_or_insert(veilshell_render::power::PowerMenu::Closed);
        ctx.settings = Some(SettingsView {
            startup: "Tablet",
            panel: true,
            wallpaper: if ctx.session_wallpaper.is_some() {
                WallpaperPreset::Omarchy
            } else {
                WallpaperPreset::Midnight
            },
            follow_omarchy_colors: false,
            bindings: "Preview",
            feedback: None,
        });
        ctx.home_date_text = "Sunday 22 November".into();
    }
    ctx.osk_state.terminal = terminal_osk;
    if !demo {
        let snap = LinuxHardware::auto_detect().snapshot();
        if tablet_layout {
            (ctx.clock_text, ctx.home_date_text) = current_tablet_clock();
        } else {
            ctx.clock_text = current_clock_hhmm();
        }
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
        viewport,
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

fn snapshot_preview(
    args: &[String],
) -> Result<(SceneKind, ViewportSize, Option<TabletOverviewOrigin>, f32), String> {
    let name = snapshot_value(args, "--scene")?.unwrap_or("home");
    let scene = SceneKind::parse_name(name).ok_or_else(|| {
        format!("invalid --scene '{name}' (expected: pill, home, drawer, launcher, settings, recents, shade, osk)")
    })?;
    let tablet_overview = match snapshot_value(args, "--tablet-recents")? {
        None => None,
        Some(_) if scene != SceneKind::Recents => {
            return Err("--tablet-recents requires --scene recents".into());
        }
        Some("app") => Some(TabletOverviewOrigin::App),
        Some("home") => Some(TabletOverviewOrigin::Home),
        Some(_) => return Err("--tablet-recents expects app or home".into()),
    };
    let mut progress = 1.0;
    if let Some(raw) = snapshot_value(args, "--progress")? {
        progress = parse_snapshot_number("--progress", raw)?;
        if !(0.0..=1.0).contains(&progress)
            || !(matches!(
                scene,
                SceneKind::Home | SceneKind::Shade | SceneKind::Drawer
            ) || tablet_overview.is_some())
        {
            return Err(
                "--progress requires 0..=1 and a Home, Shade, Drawer or Tablet Recents snapshot"
                    .into(),
            );
        }
    }
    Ok((
        scene,
        snapshot_viewport(args, scene)?,
        tablet_overview,
        progress,
    ))
}

fn snapshot_viewport(args: &[String], scene: SceneKind) -> Result<ViewportSize, String> {
    let default_height = match scene {
        SceneKind::Pill => veilshell_wayland::EDGE_HANDLE_DEPTH,
        SceneKind::Osk => 284,
        _ => 853,
    };
    let viewport = ViewportSize {
        width: snapshot_value(args, "--width")?
            .map_or(Ok(1280), |raw| parse_snapshot_number("--width", raw))?,
        height: snapshot_value(args, "--height")?.map_or(Ok(default_height), |raw| {
            parse_snapshot_number("--height", raw)
        })?,
        scale: snapshot_value(args, "--scale")?
            .map_or(Ok(1.0), |raw| parse_snapshot_number("--scale", raw))?,
    };
    let (pixel_width, pixel_height) = viewport.physical_pixels();
    if viewport.width == 0
        || viewport.height == 0
        || !viewport.scale.is_finite()
        || viewport.scale <= 0.0
        || pixel_width == 0
        || pixel_height == 0
    {
        return Err("snapshot width, height, scale and scaled dimensions must be positive".into());
    }
    // Bound CPU allocations as well as GPU textures when using arbitrary CLI sizes.
    if pixel_width > 8192 || pixel_height > 8192 {
        return Err("snapshot scaled width and height must be at most 8192 px".into());
    }
    Ok(viewport)
}

fn parse_snapshot_number<T: FromStr>(flag: &str, raw: &str) -> Result<T, String> {
    raw.parse().map_err(|_| format!("invalid {flag} '{raw}'"))
}

fn snapshot_value<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, String> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    args.get(index + 1)
        .map(String::as_str)
        .filter(|value| !value.starts_with("--"))
        .map(Some)
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn tablet_shade_preview(
    args: &[String],
    ctx: &mut SceneContext,
    scene: SceneKind,
    demo: bool,
    (width, height): (u32, u32),
) -> Result<bool, &'static str> {
    let tablet = args.iter().any(|a| a == "--tablet-shade");
    let empty = args.iter().any(|a| a == "--empty-notifications");
    if empty && (!tablet || !demo) {
        return Err("--empty-notifications requires --demo --tablet-shade");
    }
    if args.iter().any(|a| a == "--shade-position") {
        let position = parse_flag(args, "--shade-position").and_then(|raw| raw.parse::<f32>().ok());
        if !tablet
            || tablet_shade_is_split(width as f32, height as f32)
            || ctx.overlay_progress < 0.98
            || !position.is_some_and(|value| (0.0..=1.0).contains(&value))
        {
            return Err("--shade-position requires 0..=1 and an open portrait Tablet Shade");
        }
        ctx.shade.tablet_page_position = position;
    }
    if !tablet {
        return Ok(false);
    }
    if scene != SceneKind::Shade {
        return Err("--tablet-shade requires --scene shade");
    }
    ctx.shade.tablet_page = match parse_flag(args, "--tablet-shade").as_deref() {
        Some("controls") => TabletShadePage::Controls,
        Some("notifications") => TabletShadePage::Notifications,
        _ => return Err("--tablet-shade expects controls or notifications (landscape shows both)"),
    };
    if empty {
        ctx.shade.tablet_notifications = TabletNotifications::Empty;
    }
    if demo {
        ctx.clock_text = "10:08".into();
        ctx.tablet_status = TabletStatus {
            wifi: Some(true),
            battery: Some((82, false)),
        };
    }
    Ok(true)
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
         \tshow <pill|home|drawer|launcher|settings|recents|shade|osk> Switch the active surface\n\
         \t                                  Drawer: visible standalone Tablet or --overlay-gap-gestures\n\
         \tmode <auto|laptop|tablet|desk|toggle>  Change shell mode (Desk is standalone only)\n\
         \twallpaper <PNG_PATH|clear>        Update or clear the GPU pre-blurred glass wallpaper\n\
         \treload-theme                      Reload colors.toml in the running daemon\n\
         \tquit                              Stop the running daemon cleanly\n\
         \tapps                              List discovered .desktop applications\n\
         \ttheme                             Inspect the resolved color palette and WCAG contrast\n\
         \tsnapshot [OPTIONS]                Render an offscreen glassmorphic PNG snapshot\n\
         \t  --scene <pill|home|drawer|launcher|settings|recents|shade|osk> --renderer <auto|wgpu|software> [--wallpaper <PNG>]\n\
         \t  --width <PX> --height <PX> --scale <FACTOR> --out <PATH> [--demo]\n\
         \t                                  Dimensions are logical; scaled dimensions must be 1..=8192 px\n\
         \t  --power-menu | --terminal-osk    Preview a demo subview (shade or osk)\n\
         \t  --progress <0..1>                Home, Shade, Drawer or Tablet Recents reveal progress\n\
         \t  --home-position <PAGE>           Fractional position in the Home pager\n\
         \t  --tablet-home                   Use the standalone Tablet Home layout\n\
         \t  --tablet-recents <app|home>      Preview standalone Tablet Recents from the chosen origin\n\
         \t  --tablet-shade <controls|notifications>  Tablet Shade page (landscape shows both)\n\
         \t  --shade-position <0..1>         Portrait Shade paging preview, notifications=0\n\
         \t  --empty-notifications          Known-empty history fixture (Tablet Shade demo only)\n\
         \thelp                              Print this help message",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(
        args: &[&str],
    ) -> Result<(SceneKind, ViewportSize, Option<TabletOverviewOrigin>, f32), String> {
        snapshot_preview(&args.iter().map(ToString::to_string).collect::<Vec<_>>())
    }

    #[test]
    fn tablet_snapshots_keep_recents_origin_opt_in() {
        assert_eq!(preview(&["--scene", "recents"]).unwrap().2, None);
        for (value, origin) in [
            ("app", TabletOverviewOrigin::App),
            ("home", TabletOverviewOrigin::Home),
        ] {
            let (_, _, parsed, progress) = preview(&[
                "--scene",
                "recents",
                "--tablet-recents",
                value,
                "--progress",
                "0.5",
            ])
            .unwrap();
            assert_eq!(parsed, Some(origin));
            assert_eq!(progress.to_bits(), 0.5_f32.to_bits());
        }
        let (scene, view, _, progress) = preview(&[
            "--scene", "drawer", "--width", "800", "--height", "1280", "--scale", "1.5",
        ])
        .unwrap();
        assert_eq!(scene, SceneKind::Drawer);
        assert_eq!(view.physical_pixels(), (1200, 1920));
        assert_eq!(progress.to_bits(), 1.0_f32.to_bits());
        assert!(preview(&["--scene", "drawer", "--tablet-recents", "home"]).is_err());
        assert!(preview(&["--scene", "recents", "--tablet-recents"]).is_err());
        assert!(preview(&["--scene", "recents", "--tablet-recents", "unknown"]).is_err());
        assert!(preview(&["--scene", "recents", "--progress", "0.5"]).is_err());
    }

    #[test]
    fn snapshot_rejects_bad_sizes_and_progress_for_all_preview_layouts() {
        for scene in ["home", "shade", "drawer", "recents"] {
            for invalid in [
                ["--width", "0"],
                ["--height", "-1"],
                ["--scale", "0"],
                ["--scale", "NaN"],
                ["--scale", "inf"],
                ["--scale", "1000"],
                ["--progress", "NaN"],
                ["--progress", "-0.01"],
                ["--progress", "1.01"],
            ] {
                let mut args = vec!["--scene", scene];
                if scene == "recents" {
                    args.extend(["--tablet-recents", "app"]);
                }
                let mut missing = args.clone();
                missing.push(invalid[0]);
                assert!(preview(&missing).is_err(), "{missing:?}");
                args.extend(invalid);
                assert!(preview(&args).is_err(), "{args:?}");
            }
        }
    }

    #[test]
    fn tablet_shade_empty_history_requires_an_explicit_demo_fixture() {
        let mut ctx = SceneContext::default();
        let parse = |ctx: &mut SceneContext, demo, flags: &[&str]| {
            let args = flags.iter().map(ToString::to_string).collect::<Vec<_>>();
            tablet_shade_preview(&args, ctx, SceneKind::Shade, demo, (800, 1280))
        };
        let live = ["--tablet-shade", "notifications"];
        assert_eq!(parse(&mut ctx, false, &live), Ok(true));
        assert_eq!(ctx.shade.tablet_page, TabletShadePage::Notifications);
        assert_eq!(
            ctx.shade.tablet_notifications,
            TabletNotifications::Unavailable
        );
        let empty = ["--tablet-shade", "notifications", "--empty-notifications"];
        assert!(parse(&mut ctx, false, &empty).is_err());
        assert_eq!(parse(&mut ctx, true, &empty), Ok(true));
        assert_eq!(ctx.shade.tablet_notifications, TabletNotifications::Empty);
    }

    #[test]
    fn tablet_shade_pager_preview_requires_open_portrait() {
        let args = ["--tablet-shade", "controls", "--shade-position", "0.5"].map(String::from);
        let layout = |ctx: &mut SceneContext, view| {
            tablet_shade_preview(&args, ctx, SceneKind::Shade, true, view)
        };
        let mut ctx = SceneContext {
            overlay_progress: 0.75,
            ..SceneContext::default()
        };
        assert!(layout(&mut ctx, (800, 1280)).is_err());
        ctx.overlay_progress = 1.0;
        assert!(layout(&mut ctx, (1280, 800)).is_err());
        assert_eq!(layout(&mut ctx, (800, 1280)), Ok(true));
        assert_eq!(ctx.shade.tablet_page_position, Some(0.5));
    }
}
