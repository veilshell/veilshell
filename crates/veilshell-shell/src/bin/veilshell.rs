//! `veilshell` multi-surface `Wayland` tablet & touch shell daemon.

#[path = "veilshell/app_switch.rs"]
mod app_switch;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use signal_hook::{
    consts::signal::{SIGINT, SIGTERM},
    iterator::Signals,
};

use veilshell_compositor::{CompositorBackend, HyprlandBackend, MockBackend, WindowId, WindowInfo};
use veilshell_core::{
    ModeEvent, ModePolicy, ModeStateMachine, Orientation, RotationLock, ShellMode, ThemePalette,
};
use veilshell_hw::linux::{read_sysfs_backlight, read_sysfs_battery};
use veilshell_hw::{HardwareBackend, HardwareController, LinuxHardware, SensorGuard};
use veilshell_render::power::{PowerAction, PowerMenu};
use veilshell_render::{
    ActiveRenderer, RecentsCard, RendererPreference, SceneContext, SceneKind, ShadeAction,
    ShadePage, WallpaperImage,
};
use veilshell_shell::apps::{
    apps_to_tiles, current_clock_hhmm, current_tablet_clock, discover_desktop_apps,
    running_app_window,
};
use veilshell_shell::ipc::{default_socket_path, spawn_ipc_listener};
use veilshell_shell::session::ShellSession;
use veilshell_shell::settings;
use veilshell_shell::shade;
use veilshell_shell::ShellState;
use veilshell_wayland::channel::{self, Sender};
use veilshell_wayland::{
    run_shell_daemon, DaemonCommand, OskPolicy, ShellAction, ShellActionHandler,
    WaylandRuntimeConfig,
};

struct LiveActionHandler<C> {
    compositor: C,
    desktop_apps: Vec<veilshell_core::DesktopApp>,
    hw: LinuxHardware,
    session: ShellSession,
    overlay_gap_gestures: bool,
    mode_sm: ModeStateMachine,
    rotation: RotationLock,
    target_output: Option<String>,
    auto_rotate: bool,
    session_tx: Option<Sender<DaemonCommand>>,
    shade_tx: Option<Sender<DaemonCommand>>,
    app_switch_order: app_switch::AppSwitchOrder,
    recents_output: Option<String>,
}

impl<C: CompositorBackend> LiveActionHandler<C> {
    fn overlay_gap_gestures(&self) -> bool {
        self.session == ShellSession::Overlay && self.overlay_gap_gestures
    }

    fn scoped_recents(&self, ctx: &SceneContext) -> bool {
        if self.session.owns_compositor() {
            ctx.uses_tablet_status()
        } else {
            self.overlay_gap_gestures() && ctx.uses_touch_navigation()
        }
    }

    fn recents_target_is_current(&self, id: &str, ctx: &SceneContext) -> bool {
        let Some((output, card)) = self
            .recents_output
            .as_deref()
            .zip(ctx.recents_cards.iter().find(|card| card.id == id))
        else {
            return false;
        };
        // A user can select a card long after the initial snapshot. Never
        // focus it if it moved to a different workspace or output, and
        // don't dispatch to a reused address if the stable ID changed.
        self.compositor.list_tablet_windows(output).is_ok_and(|ws| {
            ws.iter()
                .any(|w| w.id.0 == id && w.foreign_id == card.foreign_id)
        })
    }

    fn change_setting(&self, action: veilshell_render::SettingsAction, ctx: &mut SceneContext) {
        if !self.session.owns_compositor() {
            return;
        }
        if let (Some(dir), Some(view)) = (settings::directory(), &mut ctx.settings) {
            match settings::change(&dir, action, view) {
                Ok(updated) => {
                    *view = updated;
                    if action == veilshell_render::SettingsAction::Theme {
                        let palette = self.session.palette();
                        if view.follow_omarchy_colors
                            && palette.name == ThemePalette::default_dark().name
                        {
                            view.feedback =
                                Some("Omarchy colors unavailable; using VeilShell Dark.".into());
                        }
                        if let Some(tx) = &self.session_tx {
                            let _ = tx.send(DaemonCommand::UpdatePalette(palette));
                        }
                    } else if action == veilshell_render::SettingsAction::Wallpaper {
                        ctx.session_wallpaper = load_session_wallpaper(view);
                    }
                }
                Err(err) => {
                    eprintln!("veilshell: could not save preferences: {err}");
                    view.feedback = Some("Could not save this preference.".into());
                }
            }
        }
    }
    fn activate_home_app(&mut self, id: &str, exec: &str) {
        if self.session.owns_compositor() {
            if let Some(app) = self.desktop_apps.iter().find(|app| app.id == id) {
                match self.compositor.list_windows() {
                    Ok(windows) => {
                        if let Some(window) = running_app_window(app, &windows) {
                            match self.compositor.focus_window(window) {
                                Ok(()) => return,
                                Err(veilshell_compositor::CompositorError::NotFound(_)) => {}
                                Err(err) => {
                                    eprintln!("veilshell: could not focus '{id}': {err}");
                                    return;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("veilshell: could not check for an open '{id}': {err}");
                        return;
                    }
                }
            }
        }
        if let Err(err) = self.compositor.launch_command(exec) {
            eprintln!("veilshell: launch '{id}' failed: {err}");
        }
    }

    fn apply_mode_layout(
        &mut self,
        mode: ShellMode,
    ) -> Result<(), veilshell_compositor::CompositorError> {
        if self.session.owns_compositor() {
            self.compositor.apply_shell_mode(mode)?;
        }
        Ok(())
    }

    fn change_mode(
        &mut self,
        next: ModeStateMachine,
        ctx: &mut SceneContext,
    ) -> Result<(), veilshell_compositor::CompositorError> {
        let mode = next.effective_mode();
        if mode != self.mode_sm.effective_mode() {
            self.apply_mode_layout(mode)?;
        }
        self.mode_sm = next;
        ctx.tablet_mode = mode == ShellMode::Tablet;
        Ok(())
    }

    fn request_power(&self, action: PowerAction, ctx: &mut SceneContext) {
        if !self.session.owns_compositor() {
            return;
        }
        if !self
            .session_tx
            .as_ref()
            .is_some_and(|tx| spawn_power_action(tx.clone(), action))
        {
            if let Some(menu) = &mut ctx.power_menu {
                menu.finish(action, false);
            }
        }
    }

    fn sync_hardware_snapshot(&self, ctx: &mut SceneContext) {
        let snap = self.hw.snapshot();
        ctx.battery_pct = snap.battery_pct;
        ctx.wifi_connected = snap.wifi_connected;
        ctx.tablet_status.wifi = snap.wifi_radio.map(|_| snap.wifi_connected);
        ctx.bluetooth_enabled = snap.bluetooth_enabled;
        ctx.lte_connected = snap.lte_signal_pct.is_some();
        ctx.night_light = snap.night_light;
        ctx.brightness_pct = snap.brightness_pct;
        ctx.volume_pct = snap.volume_pct;
        shade::apply_hardware(ctx, &snap);
        ctx.rotation_locked = self.rotation.is_locked();
        ctx.tablet_mode = self.mode_sm.effective_mode() == ShellMode::Tablet;
    }

    fn sync_clock(&self, ctx: &mut SceneContext) {
        if self.session.owns_compositor() || self.overlay_gap_gestures() {
            (ctx.clock_text, ctx.home_date_text) = current_tablet_clock();
        } else {
            ctx.clock_text = current_clock_hhmm();
        }
    }

    fn sync_fast_sysfs(&self, ctx: &mut SceneContext) {
        self.sync_clock(ctx);
        if let Some((pct, _)) = read_sysfs_battery() {
            ctx.battery_pct = pct;
        }
        if let Some(bl) = read_sysfs_backlight() {
            ctx.brightness_pct = bl;
        }
        ctx.rotation_locked = self.rotation.is_locked();
        ctx.tablet_mode = self.mode_sm.effective_mode() == ShellMode::Tablet;
    }

    fn resolve_rotation_output(&self) -> Option<String> {
        if let Some(name) = &self.target_output {
            return Some(name.clone());
        }
        let outputs = self.compositor.list_outputs().ok()?;
        if let Some(profile) = self.hw.profile() {
            if let Some(found) = outputs
                .iter()
                .find(|o| o.name.starts_with(&profile.connector_prefix))
            {
                return Some(found.name.clone());
            }
        }
        outputs.into_iter().next().map(|o| o.name)
    }

    fn apply_effective_orientation(&mut self, next: Orientation) {
        if !self.session.owns_compositor() || !self.auto_rotate {
            return;
        }
        if let Some(output) = self.resolve_rotation_output() {
            if let Err(err) = self.compositor.set_output_orientation(&output, next) {
                eprintln!("veilshell: set_output_orientation({output}) failed: {err}");
            }
        }
    }

    fn refresh_context(&mut self, scene: SceneKind, ctx: &mut SceneContext) {
        if scene == SceneKind::Shade {
            self.sync_clock(ctx);
            self.sync_hardware_snapshot(ctx);
            ctx.shade.feedback = None;
            if !self.overlay_gap_gestures() {
                shade::refresh_optional(ctx);
                if let Some(tx) = &self.shade_tx {
                    shade::request(ctx, tx, self.session, None);
                }
            }
        } else {
            self.sync_fast_sysfs(ctx);
        }
        // Touch navigation uses refresh_tablet_overview with the gesture
        // output. A missing scope must not expose the global list.
        if scene == SceneKind::Recents {
            self.recents_output = None;
            if self.overlay_gap_gestures() || self.scoped_recents(ctx) {
                ctx.recents_cards.clear();
            } else if let Ok(windows) = self.compositor.list_windows() {
                replace_recents(ctx, windows);
            }
        }
    }

    fn toggle_setting(&mut self, id: &str, ctx: &mut SceneContext) {
        ctx.shade.feedback = None;
        let result = match id {
            "wifi" => {
                let Some(on) = ctx.shade.wifi_radio else {
                    return;
                };
                self.hw.set_wifi(!on).map(|()| {
                    ctx.shade.wifi_radio = Some(!on);
                    if on {
                        ctx.wifi_connected = false;
                        ctx.shade.wifi_name = None;
                    }
                })
            }
            "bluetooth" => self.hw.set_bluetooth(!ctx.bluetooth_enabled).map(|()| {
                ctx.bluetooth_enabled = !ctx.bluetooth_enabled;
            }),
            "lte" => {
                let Some(on) = ctx.shade.mobile_radio else {
                    return;
                };
                self.hw.set_wwan(!on).map(|()| {
                    ctx.shade.mobile_radio = Some(!on);
                    if on {
                        ctx.lte_connected = false;
                    }
                })
            }
            "night_light" if ctx.shade.night_light_available => {
                self.hw.set_night_light(!ctx.night_light).map(|()| {
                    ctx.night_light = !ctx.night_light;
                })
            }
            "autorotate" if self.auto_rotate && self.session.owns_compositor() => {
                let previous = self.rotation.effective();
                self.rotation.set_locked(!self.rotation.is_locked());
                ctx.rotation_locked = self.rotation.is_locked();
                let next = self.rotation.effective();
                if !ctx.rotation_locked
                    && next != previous
                    && self.mode_sm.effective_mode() == ShellMode::Tablet
                {
                    self.apply_effective_orientation(next);
                }
                return;
            }
            "tablet_mode" if !self.session.owns_compositor() => {
                self.mode_sm.handle(ModeEvent::ToggleManual);
                ctx.tablet_mode = self.mode_sm.effective_mode() == ShellMode::Tablet;
                return;
            }
            "desk_mode" if self.session.owns_compositor() => {
                let mut next = self.mode_sm;
                next.handle(ModeEvent::ToggleManual);
                if let Err(err) = self.change_mode(next, ctx) {
                    eprintln!("veilshell: could not switch session layout: {err}");
                    ctx.shade.feedback = Some("Could not switch session layout".into());
                }
                return;
            }
            "profile" => {
                if let Some(label) = shade::cycle_profile() {
                    ctx.shade.power_profile = Some(label);
                } else {
                    ctx.shade.feedback = Some("Could not change power profile".into());
                }
                return;
            }
            "quiet" if self.session == ShellSession::Overlay && ctx.shade.quiet.is_some() => {
                if let Some(quiet) = shade::toggle_quiet() {
                    ctx.shade.quiet = Some(quiet);
                } else {
                    ctx.shade.feedback = Some("Could not change Silent mode".into());
                }
                return;
            }
            _ => return,
        };
        if let Err(err) = result {
            eprintln!("veilshell: {id} toggle failed: {err}");
            ctx.shade.feedback = Some("Could not change this setting".into());
        }
    }

    fn set_slider(&mut self, id: &str, pct: u8, ctx: &mut SceneContext) {
        let result = match id {
            "brightness" => {
                let pct = pct.clamp(5, 100);
                if pct == ctx.brightness_pct {
                    return;
                }
                self.hw
                    .set_brightness(pct)
                    .map(|()| ctx.brightness_pct = pct)
            }
            "volume" => {
                let pct = pct.min(100);
                if pct == ctx.volume_pct {
                    return;
                }
                self.hw.set_volume(pct).map(|()| ctx.volume_pct = pct)
            }
            _ => return,
        };
        ctx.shade.feedback = result.err().map(|err| {
            eprintln!("veilshell: set {id} failed: {err}");
            format!("Could not change {id}")
        });
    }
}

fn replace_recents(ctx: &mut SceneContext, windows: Vec<WindowInfo>) {
    let mut previous: std::collections::HashMap<_, _> = ctx
        .recents_cards
        .drain(..)
        .filter_map(|card| {
            card.thumbnail
                .map(|thumb| ((card.id, card.foreign_id), thumb))
        })
        .collect();
    ctx.recents_cards = windows
        .into_iter()
        .map(|w| {
            let thumbnail = previous.remove(&(w.id.0.clone(), w.foreign_id.clone()));
            RecentsCard {
                id: w.id.0,
                app_id: if w.app_id.is_empty() {
                    "Window".to_owned()
                } else {
                    w.app_id
                },
                title: if w.title.is_empty() {
                    "Untitled".to_owned()
                } else {
                    w.title
                },
                focused: w.focused,
                foreign_id: w.foreign_id,
                thumbnail,
            }
        })
        .collect();
}

impl<C: CompositorBackend> ShellActionHandler for LiveActionHandler<C> {
    fn overlay_gap_depths(&self) -> (u32, u32) {
        if self.overlay_gap_gestures() {
            self.compositor.overlay_gap_depths()
        } else {
            (0, 0)
        }
    }

    fn refresh_tablet_overview(&mut self, output: &str, ctx: &mut SceneContext) {
        self.recents_output = None;
        if !self.scoped_recents(ctx) {
            ctx.recents_cards.clear();
            return;
        }
        match self.compositor.list_tablet_windows(output) {
            Ok(windows) => {
                if self.overlay_gap_gestures() {
                    self.recents_output = Some(output.to_owned());
                }
                replace_recents(ctx, windows);
            }
            Err(err) => {
                ctx.recents_cards.clear();
                eprintln!("veilshell: could not list windows on '{output}': {err}");
            }
        }
    }

    fn switch_tablet_app(
        &mut self,
        output: &str,
        direction: i32,
        from_home: bool,
        ctx: &SceneContext,
    ) -> bool {
        if !self.scoped_recents(ctx) {
            return false;
        }
        match self.app_switch_order.focus_adjacent(
            &mut self.compositor,
            output,
            direction,
            from_home,
            Instant::now,
        ) {
            Ok(focused) => focused,
            Err(err) => {
                eprintln!("veilshell: app switch failed: {err}");
                false
            }
        }
    }

    fn on_action(&mut self, action: ShellAction, ctx: &mut SceneContext) {
        if self.overlay_gap_gestures() {
            match &action {
                // This profile opens Shade but doesn't mutate host settings,
                // launch host helper panels, or dispatch workspace changes.
                ShellAction::SwitchWorkspace(_)
                | ShellAction::ToggleSetting(_)
                | ShellAction::SetSlider { .. }
                | ShellAction::Shade(_) => return,
                ShellAction::FocusWindow(id) | ShellAction::CloseWindow(id)
                    if !self.recents_target_is_current(id, ctx) =>
                {
                    return;
                }
                _ => {}
            }
        }
        match action {
            ShellAction::Settings(action) => self.change_setting(action, ctx),
            ShellAction::RequestRefresh(scene) => self.refresh_context(scene, ctx),
            ShellAction::FocusWindow(id) => {
                if let Err(err) = self.compositor.focus_window(&WindowId(id)) {
                    eprintln!("veilshell: focus_window failed: {err}");
                }
            }
            ShellAction::CloseWindow(id) => {
                if let Err(err) = self.compositor.close_window(&WindowId(id.clone())) {
                    eprintln!("veilshell: close_window failed: {err}");
                } else {
                    ctx.recents_cards.retain(|card| card.id != id);
                }
            }
            ShellAction::LaunchApp { id, exec } => self.activate_home_app(&id, &exec),
            ShellAction::SwitchWorkspace(delta) => {
                if let Err(err) = self.compositor.switch_workspace_relative(delta) {
                    eprintln!("veilshell: switch_workspace failed: {err}");
                }
            }
            ShellAction::ToggleSetting(id) => self.toggle_setting(id, ctx),
            ShellAction::Shade(action) => {
                if let Some(tx) = &self.shade_tx {
                    let action = if action == ShadeAction::Show(ShadePage::Main) {
                        self.sync_hardware_snapshot(ctx);
                        None
                    } else {
                        Some(action)
                    };
                    shade::request(ctx, tx, self.session, action);
                }
            }
            ShellAction::SetSlider { id, pct } => self.set_slider(id, pct, ctx),
            ShellAction::HomePageChanged(page) => {
                ctx.home_page = page;
            }
            ShellAction::RequestPower(action) => self.request_power(action, ctx),
        }
    }

    fn on_hardware_update(
        &mut self,
        keyboard_attached: Option<bool>,
        orientation: Option<Orientation>,
        ctx: &mut SceneContext,
    ) -> Option<bool> {
        let mut vis_override = None;
        if let Some(attached) = keyboard_attached {
            let mut next = self.mode_sm;
            let changed = next.handle(ModeEvent::KeyboardAttached(attached)).is_some();
            match self.change_mode(next, ctx) {
                Ok(()) if changed => {
                    vis_override = Some(self.session.owns_compositor() || ctx.tablet_mode);
                }
                Err(err) => eprintln!("veilshell: could not switch session layout: {err}"),
                _ => {}
            }
        }

        if let Some(ori) = orientation {
            ctx.shade.auto_rotate_available = self.auto_rotate && self.session.owns_compositor();
            let prev = self.rotation.effective();
            self.rotation.observe(Some(ori));
            let next = self.rotation.effective();
            if next != prev && self.mode_sm.effective_mode() == ShellMode::Tablet {
                self.apply_effective_orientation(next);
            }
        }

        self.sync_fast_sysfs(ctx);
        vis_override
    }

    fn on_custom_command(&mut self, cmd: &str, ctx: &mut SceneContext) -> (String, Option<bool>) {
        let mut parts = cmd.split_whitespace();
        if parts.next() != Some("mode") {
            return (format!("err unknown command '{cmd}'"), None);
        }
        let mut next = self.mode_sm;
        let event = match parts.next() {
            Some("auto") => {
                let kbd = self.hw.detect_keyboard_attached();
                let _ = next.handle(ModeEvent::KeyboardAttached(kbd));
                ModeEvent::SetPolicy(ModePolicy::Auto)
            }
            Some("laptop") => ModeEvent::SetPolicy(ModePolicy::ForceLaptop),
            Some("desk") if self.session.owns_compositor() => {
                ModeEvent::SetPolicy(ModePolicy::ForceLaptop)
            }
            Some("desk") => return ("err Desk Mode requires a standalone session".into(), None),
            Some("tablet") => ModeEvent::SetPolicy(ModePolicy::ForceTablet),
            Some("toggle") => ModeEvent::ToggleManual,
            _ => {
                return (
                    "err usage: mode <auto|laptop|tablet|desk|toggle>".to_owned(),
                    None,
                )
            }
        };
        next.handle(event);
        if let Err(err) = self.change_mode(next, ctx) {
            return (format!("err could not switch session layout: {err}"), None);
        }
        let policy = self.mode_sm.policy();
        let name = match (self.mode_sm.effective_mode(), self.session) {
            (ShellMode::Tablet, _) => "Tablet",
            (ShellMode::Laptop, ShellSession::Overlay) => "Laptop",
            (ShellMode::Laptop, ShellSession::Standalone) => "Desk",
        };
        (
            format!("Mode set to {name} (policy: {policy:?})"),
            Some(self.session.owns_compositor() || ctx.tablet_mode),
        )
    }
}

fn spawn_power_action(tx: Sender<DaemonCommand>, action: PowerAction) -> bool {
    let verb = match action {
        PowerAction::Lock => "lock",
        PowerAction::Suspend => "suspend",
        PowerAction::Restart => "reboot",
        PowerAction::Shutdown => "poweroff",
        // Logout is the ordinary daemon exit supervised by the session launcher.
        PowerAction::Logout => return false,
    };
    thread::Builder::new()
        .name("veilshell-power".into())
        .spawn(move || {
            let result = Command::new("veilshell-session")
                .args(["--power", verb])
                .status();
            let success = result.as_ref().is_ok_and(std::process::ExitStatus::success);
            if !success {
                eprintln!("veilshell: standalone {verb} failed: {result:?}");
            }
            let _ = tx.send(DaemonCommand::PowerFinished { action, success });
        })
        .is_ok()
}

fn spawn_hardware_watchers(
    cmd_tx: &Sender<DaemonCommand>,
    stop: Arc<AtomicBool>,
    hw: LinuxHardware,
    initial_keyboard_attached: bool,
    session: ShellSession,
    watch_rotation: bool,
) -> (Option<SensorGuard>, Option<thread::JoinHandle<()>>) {
    let sensor_guard = if watch_rotation && session.owns_compositor() {
        let sensor_tx = cmd_tx.clone();
        SensorGuard::spawn(move |ori| {
            let _ = sensor_tx.send(DaemonCommand::HardwareUpdate {
                keyboard_attached: None,
                orientation: Some(ori),
            });
        })
    } else {
        None
    };

    let poll_tx = cmd_tx.clone();
    let handle = thread::Builder::new()
        .name("veilshell-watchers".into())
        .spawn(move || {
            // Reconcile with the sample used by the mode state machine, not a new baseline.
            let mut last_kbd = initial_keyboard_attached;
            let mut last_theme_mtime = read_mtime(session.desktop_theme_path().as_ref());
            let mut last_bat = read_sysfs_battery().map(|(pct, _)| pct);
            let mut last_minute = unix_minute();
            let status_worker = session
                .owns_compositor()
                .then(|| spawn_tablet_status_watcher(Arc::clone(&stop), poll_tx.clone()))
                .flatten();

            let mut ticks = 0;
            while !stop.load(Ordering::Relaxed) {
                // Detect cover changes every wake; battery and theme can wait ~1.5s.
                let cur_kbd = hw.detect_keyboard_attached();
                let periodic = ticks == 6;
                let cur_bat = if periodic {
                    read_sysfs_battery().map(|(pct, _)| pct)
                } else {
                    last_bat
                };
                let minute = if periodic && session.owns_compositor() {
                    unix_minute()
                } else {
                    last_minute
                };
                if cur_kbd != last_kbd || cur_bat != last_bat || minute != last_minute {
                    let keyboard_attached = (cur_kbd != last_kbd).then_some(cur_kbd);
                    last_kbd = cur_kbd;
                    last_bat = cur_bat;
                    last_minute = minute;
                    let _ = poll_tx.send(DaemonCommand::HardwareUpdate {
                        keyboard_attached,
                        orientation: None,
                    });
                }

                if periodic {
                    ticks = 0;
                    let cur_mtime = read_mtime(session.desktop_theme_path().as_ref());
                    if cur_mtime != last_theme_mtime {
                        last_theme_mtime = cur_mtime;
                        let palette = session.palette();
                        let _ = poll_tx.send(DaemonCommand::UpdatePalette(palette));
                    }
                }
                thread::sleep(Duration::from_millis(250));
                ticks += 1;
            }
            if let Some(worker) = status_worker {
                worker.thread().unpark();
                let _ = worker.join();
            }
        })
        .ok();

    (sensor_guard, handle)
}

fn spawn_tablet_status_watcher(
    stop: Arc<AtomicBool>,
    tx: Sender<DaemonCommand>,
) -> Option<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("veilshell-tablet-status".into())
        .spawn(move || {
            let mut last = None;
            while !stop.load(Ordering::Relaxed) {
                let status = tablet_status();
                if last != Some(status) {
                    last = Some(status);
                    if tx.send(DaemonCommand::UpdateTabletStatus(status)).is_err() {
                        break;
                    }
                }
                thread::park_timeout(Duration::from_secs(4));
            }
        })
        .ok()
}

fn tablet_status() -> veilshell_render::TabletStatus {
    let wifi = shade::bounded_output(
        "1s",
        "nmcli",
        &["-t", "-f", "TYPE,STATE,CONNECTION", "dev", "status"],
    )
    .filter(|devices| devices.lines().any(|line| line.starts_with("wifi:")))
    .map(|devices| {
        veilshell_hw::linux::parse_nmcli_devices(&devices)
            .0
            .is_some()
    });
    veilshell_render::TabletStatus {
        battery: read_sysfs_battery(),
        wifi,
    }
}

fn unix_minute() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() / 60)
}

fn read_mtime(path: Option<&PathBuf>) -> Option<SystemTime> {
    fs::metadata(path?).ok()?.modified().ok()
}

fn parse_policy_flag(raw: &str) -> Option<ModePolicy> {
    match raw.to_ascii_lowercase().as_str() {
        "auto" => Some(ModePolicy::Auto),
        "laptop" => Some(ModePolicy::ForceLaptop),
        "tablet" => Some(ModePolicy::ForceTablet),
        _ => None,
    }
}

struct CliOptions {
    dry_run: bool,
    no_watchers: bool,
    overlay_gap_gestures: bool,
    scene: SceneKind,
    policy: ModePolicy,
    osk_policy: OskPolicy,
    renderer: RendererPreference,
    wallpaper: Option<WallpaperImage>,
    output_name: Option<String>,
    socket_path: PathBuf,
    exit_after_ms: Option<u64>,
}

enum CliOutcome {
    Run(CliOptions),
    Exit(ExitCode),
}

#[allow(clippy::too_many_lines)]
fn parse_cli(args: &[String], session: ShellSession) -> CliOutcome {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return CliOutcome::Exit(ExitCode::SUCCESS);
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("veilshell {}", env!("CARGO_PKG_VERSION"));
        return CliOutcome::Exit(ExitCode::SUCCESS);
    }

    let dry_run = args.iter().any(|a| a == "--dry-run");
    let no_watchers = args.iter().any(|a| a == "--no-watchers");
    let overlay_gap_gestures = args.iter().any(|a| a == "--overlay-gap-gestures");
    if overlay_gap_gestures && session.owns_compositor() {
        eprintln!("--overlay-gap-gestures is overlay-only; it cannot run in a standalone session");
        return CliOutcome::Exit(ExitCode::FAILURE);
    }
    let scene = match parse_flag(args, "--scene") {
        Some(raw) => {
            let Some(parsed) = SceneKind::parse_name(&raw) else {
                eprintln!("invalid --scene '{raw}'");
                return CliOutcome::Exit(ExitCode::FAILURE);
            };
            parsed
        }
        None => SceneKind::Pill,
    };
    if overlay_gap_gestures && scene == SceneKind::Settings {
        eprintln!("Settings requires a standalone session");
        return CliOutcome::Exit(ExitCode::FAILURE);
    }

    let policy = match parse_flag(args, "--policy") {
        Some(raw) => {
            let Some(parsed) = parse_policy_flag(&raw) else {
                eprintln!("invalid --policy '{raw}' (expected: auto, laptop, tablet)");
                return CliOutcome::Exit(ExitCode::FAILURE);
            };
            parsed
        }
        None if scene != SceneKind::Pill => ModePolicy::ForceTablet,
        None => ModePolicy::Auto,
    };

    let osk_policy = if args.iter().any(|arg| arg == "--osk") {
        match parse_flag(args, "--osk").as_deref() {
            Some("auto") => OskPolicy::Auto,
            Some("manual") => OskPolicy::Manual,
            _ => {
                eprintln!("--osk expects auto or manual");
                return CliOutcome::Exit(ExitCode::FAILURE);
            }
        }
    } else if overlay_gap_gestures {
        OskPolicy::Manual
    } else {
        OskPolicy::Auto
    };

    let renderer = match parse_flag(args, "--renderer") {
        Some(raw) => {
            let Some(parsed) = RendererPreference::parse_name(&raw) else {
                eprintln!("invalid --renderer '{raw}' (expected: auto, wgpu, software)");
                return CliOutcome::Exit(ExitCode::FAILURE);
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
                return CliOutcome::Exit(ExitCode::FAILURE);
            }
        },
        None => None,
    };

    let output_name = parse_flag(args, "--output");
    let socket_path = parse_flag(args, "--socket").map_or_else(default_socket_path, PathBuf::from);
    let exit_after_ms = match parse_flag(args, "--exit-after-ms") {
        Some(raw) => {
            let Ok(ms) = raw.parse::<u64>() else {
                eprintln!("invalid --exit-after-ms '{raw}'");
                return CliOutcome::Exit(ExitCode::FAILURE);
            };
            Some(ms)
        }
        None => None,
    };

    CliOutcome::Run(CliOptions {
        dry_run,
        no_watchers,
        overlay_gap_gestures,
        scene,
        policy,
        osk_policy,
        renderer,
        wallpaper,
        output_name,
        socket_path,
        exit_after_ms,
    })
}

fn live_backend() -> Result<HyprlandBackend, String> {
    if ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"]
        .iter()
        .all(|name| env::var_os(name).is_none_or(|value| value.is_empty()))
    {
        return Err("run inside an active Hyprland Wayland session or use --dry-run".into());
    }
    let backend = HyprlandBackend::from_env()
        .ok_or("Hyprland is required; start from a terminal inside that session")?;
    backend.list_outputs().map_err(|err| {
        format!(
            "cannot query Hyprland at {}: {err}",
            backend.socket_path().display()
        )
    })?;
    Ok(backend)
}

fn watch_shutdown_signals(cmd_tx: Sender<DaemonCommand>) -> std::io::Result<()> {
    // Queue the same shutdown command as veilctl, even during Wayland startup.
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    thread::Builder::new()
        .name("veilshell-shutdown".into())
        .spawn(move || {
            for _ in signals.forever() {
                if cmd_tx.send(DaemonCommand::Quit).is_err() {
                    break;
                }
            }
        })?;
    Ok(())
}

fn initial_scene(
    session: ShellSession,
    apps: &[veilshell_core::DesktopApp],
    visible: bool,
    overlay_gap_gestures: bool,
) -> SceneContext {
    let (clock_text, home_date_text) = if session.owns_compositor() || overlay_gap_gestures {
        current_tablet_clock()
    } else {
        (current_clock_hhmm(), String::new())
    };
    let mut ctx = SceneContext {
        settings: session
            .owns_compositor()
            .then(settings::directory)
            .flatten()
            .as_deref()
            .map(settings::load),
        clock_text,
        home_date_text,
        tablet_mode: visible,
        overlay_gap_gestures,
        home_apps: apps_to_tiles(apps),
        power_menu: session.owns_compositor().then_some(PowerMenu::default()),
        ..SceneContext::default()
    };
    ctx.shade.auto_rotate_available = false;
    ctx.shade.quiet = None;
    if overlay_gap_gestures {
        ctx.shade.power_profile = None;
    }
    if let Some(view) = &mut ctx.settings {
        ctx.session_wallpaper = load_session_wallpaper(view);
        ctx.tablet_status.battery = read_sysfs_battery();
    }
    ctx
}

fn load_session_wallpaper(
    view: &mut veilshell_render::SettingsView,
) -> Option<Arc<veilshell_render::RgbaImage>> {
    if view.wallpaper != veilshell_render::WallpaperPreset::Omarchy {
        return None;
    }
    let wallpaper = settings::import_wallpaper();
    if wallpaper.is_none() {
        view.feedback = Some("Omarchy wallpaper unavailable; using Midnight.".into());
    }
    wallpaper
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let session = ShellSession::current();
    let opts = match parse_cli(&args, session) {
        CliOutcome::Run(o) => o,
        CliOutcome::Exit(code) => return code,
    };

    let compositor = if opts.dry_run {
        None
    } else {
        match live_backend() {
            Ok(backend) if opts.overlay_gap_gestures => Some(backend.with_overlay_gap_gestures()),
            Ok(backend) => Some(backend),
            Err(err) => {
                eprintln!("veilshell: {err}");
                return ExitCode::FAILURE;
            }
        }
    };
    run_daemon(opts, session, compositor)
}

fn run_daemon(
    opts: CliOptions,
    session: ShellSession,
    compositor: Option<HyprlandBackend>,
) -> ExitCode {
    let palette = session.palette();
    let discovered = discover_desktop_apps();
    let hw = LinuxHardware::auto_detect();
    let preferred_scale = hw.profile().map_or(1.0, |p| p.preferred_scale);
    let watch_rotation = hw.profile().is_none_or(|p| p.auto_rotate_in_tablet_mode);
    let initial_keyboard_attached = hw.detect_keyboard_attached();
    let mode_sm = ModeStateMachine::new(opts.policy, initial_keyboard_attached);
    let initial_visible = session.owns_compositor()
        || mode_sm.effective_mode() == ShellMode::Tablet
        || opts.scene != SceneKind::Pill;

    let mut scene_ctx = initial_scene(
        session,
        &discovered,
        initial_visible,
        opts.overlay_gap_gestures,
    );

    let Some(compositor) = compositor else {
        print_dry_run(&opts, hw, palette, scene_ctx, initial_visible);
        return ExitCode::SUCCESS;
    };

    let (cmd_tx, cmd_rx) = channel::channel();
    if let Err(err) = watch_shutdown_signals(cmd_tx.clone()) {
        eprintln!("veilshell: could not watch shutdown signals: {err}");
        return ExitCode::FAILURE;
    }
    let mut handler = LiveActionHandler {
        compositor,
        desktop_apps: discovered,
        hw: hw.clone(),
        session,
        overlay_gap_gestures: opts.overlay_gap_gestures,
        mode_sm,
        rotation: RotationLock::default(),
        target_output: opts.output_name.clone(),
        auto_rotate: false,
        session_tx: session.owns_compositor().then(|| cmd_tx.clone()),
        shade_tx: Some(cmd_tx.clone()),
        app_switch_order: app_switch::AppSwitchOrder::default(),
        recents_output: None,
    };
    handler.sync_hardware_snapshot(&mut scene_ctx);
    if let Err(err) = handler.apply_mode_layout(mode_sm.effective_mode()) {
        eprintln!("veilshell: could not apply initial session layout: {err}");
    }

    if let Err(err) = spawn_ipc_listener(
        &opts.socket_path,
        cmd_tx.clone(),
        session,
        opts.overlay_gap_gestures,
    ) {
        eprintln!(
            "veilshell: could not bind IPC socket {}: {err}",
            opts.socket_path.display()
        );
        return ExitCode::FAILURE;
    }

    let stop_watchers = Arc::new(AtomicBool::new(false));
    let (sensor_guard, watcher_handle) = if opts.no_watchers {
        (None, None)
    } else {
        spawn_hardware_watchers(
            &cmd_tx,
            Arc::clone(&stop_watchers),
            hw,
            initial_keyboard_attached,
            session,
            watch_rotation,
        )
    };
    handler.auto_rotate = sensor_guard.is_some();

    let config = WaylandRuntimeConfig {
        display_override: None,
        output_name: opts.output_name,
        preferred_scale,
        exit_after_ms: opts.exit_after_ms,
        initial_visible,
        osk_policy: opts.osk_policy,
        renderer: opts.renderer,
        wallpaper: opts.wallpaper,
    };

    let result = run_shell_daemon(&config, opts.scene, palette, scene_ctx, cmd_rx, handler);

    stop_watchers.store(true, Ordering::Relaxed);
    drop(sensor_guard);
    let _ = watcher_handle.map(thread::JoinHandle::join);
    let _ = fs::remove_file(&opts.socket_path);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("veilshell: {err}");
            ExitCode::FAILURE
        }
    }
}

fn print_dry_run(
    opts: &CliOptions,
    hw: LinuxHardware,
    palette: ThemePalette,
    ctx: SceneContext,
    visible: bool,
) {
    let profile_id = hw.profile().map(|profile| profile.id.clone());
    let mock = MockBackend::default();
    let windows = mock.list_windows().map_or(0, |windows| windows.len());
    let state = ShellState::new(mock, hw, palette);
    print!("{}", state.status_report());
    if let Some(id) = profile_id {
        println!("  profile    : {id}");
    }
    println!("  scene      : {}", opts.scene.as_str());
    println!("  policy     : {:?}", opts.policy);
    println!("  app OSK    : {:?}", opts.osk_policy);
    if opts.overlay_gap_gestures {
        println!("  overlay gaps: requested (not queried in dry run)");
    }
    println!("  visible    : {visible}");
    let apps = ctx.home_apps.len();
    match ActiveRenderer::new(opts.renderer, ctx) {
        Ok(active) => println!("  renderer   : {}", active.backend_label()),
        Err(err) => println!("  renderer   : unavailable ({err})"),
    }
    println!("  apps       : {apps}");
    println!("  windows    : {windows} (mock)");
    println!("  socket     : {}", opts.socket_path.display());
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
         \t--policy <auto|laptop|tablet>          Initial Laptop/Tablet mode policy (default: auto)\n\
         \t--overlay-gap-gestures                 Overlay only: touch navigation from uncovered Hyprland gaps\n\
         \t                                       Use --policy tablet to keep the overlay visible\n\
         \t--osk <auto|manual>                    Manual never claims the input method (default: auto; manual with gap gestures)\n\
         \t--scene <pill|home|drawer|launcher|settings|recents|shade|osk>  Initial surface scene (default: pill)\n\
         \t--renderer <auto|wgpu|software>        Rendering backend (default: auto)\n\
         \t--wallpaper <PNG_PATH>                 Static PNG wallpaper for pre-blurred GPU glass\n\
         \t--output <NAME>                        Bind surfaces to a specific Wayland output\n\
         \t--socket <PATH>                        Override Unix control socket path\n\
         \t--no-watchers                          Disable background sensor/theme polling (for tests)\n\
         \t--exit-after-ms <MS>                   Exit cleanly after MS milliseconds (for CI tests)\n\
         \t--dry-run                              Print status without opening Wayland surfaces\n\
         \t-h, --help                             Print this help message\n\
         \t-V, --version                          Print version information",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_osk_is_an_independent_opt_in() {
        let CliOutcome::Run(options) = parse_cli(&[], ShellSession::Standalone) else {
            panic!("expected default options");
        };
        assert_eq!(options.osk_policy, OskPolicy::Auto);
        let args = ["--policy", "tablet", "--osk", "manual"].map(str::to_owned);
        let CliOutcome::Run(options) = parse_cli(&args, ShellSession::Overlay) else {
            panic!("expected manual OSK options");
        };
        assert_eq!(options.osk_policy, OskPolicy::Manual);
        assert_eq!(options.policy, ModePolicy::ForceTablet);
        for args in [vec!["--osk"], vec!["--osk", "unknown"]] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(matches!(
                parse_cli(&args, ShellSession::Overlay),
                CliOutcome::Exit(ExitCode::FAILURE)
            ));
        }
    }

    #[test]
    fn overlay_gap_flag_is_explicit_and_overlay_only() {
        let CliOutcome::Run(default) = parse_cli(&[], ShellSession::Overlay) else {
            panic!("expected default overlay options");
        };
        assert!(!default.overlay_gap_gestures);
        assert_eq!(default.osk_policy, OskPolicy::Auto);

        let args = ["--overlay-gap-gestures", "--dry-run", "--policy", "tablet"].map(str::to_owned);
        let CliOutcome::Run(gap) = parse_cli(&args, ShellSession::Overlay) else {
            panic!("expected overlay gap options");
        };
        assert!(gap.overlay_gap_gestures);
        assert_eq!(gap.policy, ModePolicy::ForceTablet);
        assert_eq!(gap.osk_policy, OskPolicy::Manual);
        assert!(matches!(
            parse_cli(&args, ShellSession::Standalone),
            CliOutcome::Exit(ExitCode::FAILURE)
        ));

        let args = ["--overlay-gap-gestures", "--osk", "auto"].map(str::to_owned);
        let CliOutcome::Run(gap) = parse_cli(&args, ShellSession::Overlay) else {
            panic!("expected overlay gap options with automatic OSK override");
        };
        assert_eq!(gap.policy, ModePolicy::Auto);
        assert_eq!(gap.osk_policy, OskPolicy::Auto);
        let args = ["--overlay-gap-gestures", "--scene", "settings"].map(str::to_owned);
        assert!(matches!(
            parse_cli(&args, ShellSession::Overlay),
            CliOutcome::Exit(ExitCode::FAILURE)
        ));
    }

    #[test]
    fn watcher_reports_a_keyboard_change_since_the_startup_sample() {
        let (tx, rx) = channel::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (_, watcher) = spawn_hardware_watchers(
            &tx,
            Arc::clone(&stop),
            LinuxHardware::new(None), // Always detects attached without a USB profile.
            false,
            ShellSession::Standalone,
            false,
        );
        let watcher = watcher.expect("spawn watcher");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let reported = loop {
            if matches!(
                rx.try_recv(),
                Ok(DaemonCommand::HardwareUpdate {
                    keyboard_attached: Some(true),
                    orientation: None,
                })
            ) {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(10));
        };
        stop.store(true, Ordering::Relaxed);
        watcher.join().expect("join watcher");
        assert!(reported, "the startup attachment change was lost");
    }

    fn handler(session: ShellSession) -> LiveActionHandler<MockBackend> {
        LiveActionHandler {
            compositor: MockBackend::default(),
            desktop_apps: Vec::new(),
            hw: LinuxHardware::new(None),
            session,
            overlay_gap_gestures: false,
            mode_sm: ModeStateMachine::new(ModePolicy::Auto, true),
            rotation: RotationLock::default(),
            target_output: Some("eDP-1".to_owned()),
            auto_rotate: true,
            session_tx: None,
            shade_tx: None,
            app_switch_order: app_switch::AppSwitchOrder::default(),
            recents_output: None,
        }
    }

    #[test]
    fn tablet_overview_uses_the_output_snapshot_and_clears_on_error() {
        let mut shell = handler(ShellSession::Standalone);
        shell.mode_sm = ModeStateMachine::new(ModePolicy::ForceTablet, false);
        let mut ctx = SceneContext {
            power_menu: Some(PowerMenu::default()),
            ..SceneContext::default()
        };

        shell.on_action(ShellAction::RequestRefresh(SceneKind::Recents), &mut ctx);
        assert_eq!(ctx.recents_cards, Vec::<RecentsCard>::new());

        shell
            .compositor
            .focus_window(&WindowId("0x2".into()))
            .unwrap();
        shell.refresh_tablet_overview("eDP-1", &mut ctx);
        assert_eq!(ctx.recents_cards[0].id, "0x2");
        assert!(ctx.recents_cards[0].focused);

        shell.refresh_tablet_overview("eDP", &mut ctx);
        assert_eq!(ctx.recents_cards, Vec::<RecentsCard>::new());
        assert!(!shell.switch_tablet_app("eDP", 1, true, &ctx));
    }

    #[test]
    fn desk_and_overlay_recents_keep_the_legacy_window_query() {
        for (session, policy) in [
            (ShellSession::Standalone, ModePolicy::ForceLaptop),
            (ShellSession::Overlay, ModePolicy::ForceTablet),
        ] {
            let mut shell = handler(session);
            shell.mode_sm = ModeStateMachine::new(policy, false);
            let mut ctx = SceneContext {
                power_menu: session.owns_compositor().then_some(PowerMenu::default()),
                recents_cards: Vec::new(),
                ..SceneContext::default()
            };
            shell
                .compositor
                .focus_window(&WindowId("0x2".into()))
                .unwrap();

            shell.on_action(ShellAction::RequestRefresh(SceneKind::Recents), &mut ctx);

            assert_eq!(ctx.recents_cards.len(), 3);
            assert_eq!(ctx.recents_cards[0].id, "0x1");
            assert!(ctx.recents_cards[1].focused);
            assert!(!shell.switch_tablet_app("eDP-1", 1, true, &ctx));
        }
    }

    #[test]
    fn overlay_gap_recents_and_switcher_require_the_named_output_and_never_fall_back() {
        let mut shell = handler(ShellSession::Overlay);
        shell.overlay_gap_gestures = true;
        let mut ctx = initial_scene(ShellSession::Overlay, &[], true, true);
        assert!(ctx.uses_touch_navigation());
        assert!(!ctx.uses_tablet_status());
        assert!(ctx.power_menu.is_none());
        assert!(ctx.settings.is_none());
        assert!(!ctx.shade.auto_rotate_available);

        shell.on_action(ShellAction::RequestRefresh(SceneKind::Recents), &mut ctx);
        assert_eq!(ctx.recents_cards, []);
        shell.refresh_tablet_overview("eDP", &mut ctx);
        assert_eq!(ctx.recents_cards, []);
        assert!(!shell.switch_tablet_app("eDP", 1, true, &ctx));

        assert!(shell.switch_tablet_app("eDP-1", 1, true, &ctx));
        assert!(shell.switch_tablet_app("eDP-1", 1, false, &ctx));
        shell.refresh_tablet_overview("eDP-1", &mut ctx);
        assert_eq!(ctx.recents_cards[0].id, "0x2");
        assert!(ctx.recents_cards[0].focused);
        assert_eq!(shell.compositor.last_workspace_delta(), 0);
        assert_eq!(shell.compositor.launched_commands(), Vec::<String>::new());
    }

    #[test]
    fn overlay_gap_actions_recheck_scoped_cards_and_do_not_run_host_controls() {
        let mut shell = handler(ShellSession::Overlay);
        shell.overlay_gap_gestures = true;
        let mut ctx = initial_scene(ShellSession::Overlay, &[], true, true);
        shell.refresh_tablet_overview("eDP-1", &mut ctx);
        shell.on_action(ShellAction::FocusWindow("0x2".into()), &mut ctx);
        assert!(shell.compositor.list_windows().unwrap()[1].focused);
        ctx.recents_cards[0].foreign_id = Some("stale-card".into());
        shell.on_action(ShellAction::CloseWindow("0x1".into()), &mut ctx);
        assert_eq!(shell.compositor.list_windows().unwrap().len(), 3);
        shell.refresh_tablet_overview("unknown-output", &mut ctx);
        shell.on_action(ShellAction::FocusWindow("0x1".into()), &mut ctx);
        assert!(shell.compositor.list_windows().unwrap()[1].focused);

        shell.on_action(ShellAction::SwitchWorkspace(1), &mut ctx);
        assert_eq!(shell.compositor.last_workspace_delta(), 0);
        shell.on_action(ShellAction::ToggleSetting("tablet_mode"), &mut ctx);
        assert_eq!(shell.mode_sm.policy(), ModePolicy::Auto);
        let (tx, rx) = channel::channel();
        shell.shade_tx = Some(tx);
        // Use an unsupported page so even a regression can't run a real
        // host tool. The opt-in must not queue the request at all.
        shell.on_action(
            ShellAction::Shade(ShadeAction::Host(ShadePage::Main)),
            &mut ctx,
        );
        assert!(ctx.shade.pending.is_none());
        assert!(rx.try_recv().is_err());

        // Keep session_tx absent: even a guard regression cannot run power.
        let menu = PowerMenu::Running(PowerAction::Lock);
        ctx.power_menu = Some(menu);
        shell.on_action(ShellAction::RequestPower(PowerAction::Lock), &mut ctx);
        assert_eq!(ctx.power_menu, Some(menu));
    }

    #[test]
    fn recents_keeps_thumbnails_only_for_the_same_card_identity() {
        let mut ctx = SceneContext::default();
        let mut windows = MockBackend::default().list_windows().unwrap();
        for (card, window) in ctx.recents_cards.iter_mut().zip(&mut windows) {
            let foreign_id = format!("stable-{}", card.id);
            card.foreign_id = Some(foreign_id.clone());
            window.foreign_id = Some(foreign_id);
        }
        let surviving_thumb = ctx.recents_cards[0].thumbnail.clone().unwrap();
        windows.pop();
        windows.swap(0, 1);
        windows[0].foreign_id = Some("replacement-same-address".into());
        windows[1].title = "Renamed terminal".into();

        replace_recents(&mut ctx, windows);

        assert_eq!(ctx.recents_cards.len(), 2);
        assert_eq!(ctx.recents_cards[0].id, "0x2");
        assert!(ctx.recents_cards[0].thumbnail.is_none());
        let kept = &ctx.recents_cards[1];
        assert_eq!(kept.title, "Renamed terminal");
        assert!(Arc::ptr_eq(
            kept.thumbnail.as_ref().unwrap(),
            &surviving_thumb
        ));
    }

    #[test]
    fn standalone_home_focuses_an_open_app_instead_of_launching_again() {
        let mut shell = handler(ShellSession::Standalone);
        let id = "example-browser.desktop";
        shell.desktop_apps.push(
            veilshell_core::parse_desktop_entry(
                "[Desktop Entry]\nType=Application\nName=Browser\nExec=browser\nStartupWMClass=Chromium",
                id,
            )
            .unwrap(),
        );

        shell.activate_home_app(id, "browser");

        let windows = shell.compositor.list_windows().unwrap();
        assert!(windows.iter().any(|w| w.id.0 == "0x2" && w.focused));
        assert_eq!(shell.compositor.launched_commands(), Vec::<String>::new());
    }

    #[test]
    fn standalone_home_launches_when_only_a_different_browser_app_is_open() {
        let mut shell = handler(ShellSession::Standalone);
        let id = "example-web-app.desktop";
        let exec = "chromium --app=https://example.org";
        shell.desktop_apps.push(
            veilshell_core::parse_desktop_entry(
                &format!("[Desktop Entry]\nType=Application\nName=Web App\nExec={exec}"),
                id,
            )
            .unwrap(),
        );

        shell.activate_home_app(id, exec);

        assert_eq!(shell.compositor.launched_commands(), [exec]);
    }

    #[test]
    fn overlay_home_keeps_launching_even_if_an_app_is_open() {
        let mut shell = handler(ShellSession::Overlay);
        let id = "chromium.desktop";
        shell.desktop_apps.push(
            veilshell_core::parse_desktop_entry(
                "[Desktop Entry]\nType=Application\nName=Chromium\nExec=chromium",
                id,
            )
            .unwrap(),
        );

        shell.activate_home_app(id, "chromium");

        assert_eq!(shell.compositor.launched_commands(), ["chromium"]);
    }

    #[test]
    fn overlay_mode_and_sensor_events_do_not_change_the_host_compositor() {
        let mut shell = handler(ShellSession::Overlay);
        let mut ctx = SceneContext::default();
        assert_eq!(
            shell.on_hardware_update(Some(false), Some(Orientation::Rotate90), &mut ctx),
            Some(true)
        );
        assert!(ctx.tablet_mode);
        assert!(!ctx.shade.auto_rotate_available);
        assert_eq!(
            shell.on_custom_command("mode laptop", &mut ctx).1,
            Some(false)
        );
        assert_eq!(
            shell.on_custom_command("mode tablet", &mut ctx).1,
            Some(true)
        );
        let mock = shell.compositor;
        assert_eq!(mock.applied_mode(), ShellMode::Laptop);
        assert_eq!(
            mock.list_outputs().unwrap()[0].orientation,
            Orientation::Normal
        );
    }

    #[test]
    fn standalone_can_manage_its_own_compositor() {
        let mut shell = handler(ShellSession::Standalone);
        let mut ctx = SceneContext::default();
        assert_eq!(
            shell.on_hardware_update(Some(false), Some(Orientation::Rotate90), &mut ctx),
            Some(true)
        );
        assert!(ctx.shade.auto_rotate_available);
        let mock = shell.compositor;
        assert_eq!(mock.applied_mode(), ShellMode::Tablet);
        assert_eq!(
            mock.list_outputs().unwrap()[0].orientation,
            Orientation::Rotate90
        );
    }

    #[test]
    fn desk_mode_is_standalone_only_and_keeps_the_shell_visible() {
        let mut overlay = handler(ShellSession::Overlay);
        let mut ctx = SceneContext::default();
        let (reply, visible) = overlay.on_custom_command("mode desk", &mut ctx);
        assert!(reply.starts_with("err "));
        assert_eq!(visible, None);

        let mut shell = handler(ShellSession::Standalone);
        assert_eq!(
            shell.on_custom_command("mode tablet", &mut ctx).1,
            Some(true)
        );
        assert_eq!(shell.compositor.applied_mode(), ShellMode::Tablet);
        let (reply, visible) = shell.on_custom_command("mode desk", &mut ctx);
        assert!(reply.starts_with("Mode set to Desk"));
        assert_eq!(visible, Some(true));
        assert_eq!(shell.compositor.applied_mode(), ShellMode::Laptop);
        assert!(!ctx.tablet_mode);

        shell.on_action(ShellAction::ToggleSetting("desk_mode"), &mut ctx);
        assert!(ctx.tablet_mode);
        assert_eq!(shell.compositor.applied_mode(), ShellMode::Tablet);
    }
}
