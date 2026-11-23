//! Lightweight Unix-domain control socket bridging `veilctl` and the running `veilshell` daemon.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use veilshell_render::{SceneKind, WallpaperImage};
use veilshell_wayland::channel::Sender;
use veilshell_wayland::DaemonCommand;

use crate::session::ShellSession;

/// Resolve the active IPC socket path (`VEILSHELL_SOCKET`, else `$XDG_RUNTIME_DIR/veilshell.sock`).
#[must_use]
pub fn default_socket_path() -> PathBuf {
    if let Some(custom) = std::env::var_os("VEILSHELL_SOCKET") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime).join("veilshell.sock");
    }
    PathBuf::from("/tmp/veilshell.sock")
}

/// Send a single-line control command to a running `veilshell` daemon.
///
/// # Errors
/// Returns an `std::io::Error` if no daemon is listening or the socket fails.
pub fn send_daemon_command(socket_path: &Path, line: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(line.trim().as_bytes())?;
    stream.write_all(b"\n")?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

/// Bind `socket_path` and spawn a background thread forwarding client lines into `cmd_tx`.
///
/// # Errors
/// Returns an `std::io::Error` if binding the Unix socket fails.
pub fn spawn_ipc_listener(
    socket_path: &Path,
    cmd_tx: Sender<DaemonCommand>,
    session: ShellSession,
    overlay_gap_gestures: bool,
) -> std::io::Result<()> {
    // Do not replace a running overlay's endpoint (or an unrelated file).
    let listener = UnixListener::bind(socket_path)?;

    let result = thread::Builder::new()
        .name("veilshell-ipc".into())
        .spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else {
                    break;
                };
                if handle_client(stream, &cmd_tx, session, overlay_gap_gestures) {
                    break;
                }
            }
        });
    if result.is_err() {
        let _ = fs::remove_file(socket_path);
    }
    result.map(|_| ())
}

fn handle_client(
    mut stream: UnixStream,
    cmd_tx: &Sender<DaemonCommand>,
    session: ShellSession,
    overlay_gap_gestures: bool,
) -> bool {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(cloned) => cloned,
        Err(_) => return false,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return false;
    }
    let trimmed = line.trim();
    let mut parts = trimmed.split_whitespace();
    let Some(verb) = parts.next() else {
        let _ = writeln!(stream, "err empty command");
        return false;
    };

    match verb {
        "show" => {
            let Some(scene_str) = parts.next() else {
                let _ = writeln!(
                    stream,
                    "err usage: show <pill|home|drawer|launcher|settings|recents|shade|osk>"
                );
                return false;
            };
            let Some(scene) = SceneKind::parse_name(scene_str) else {
                let _ = writeln!(stream, "err unknown scene '{scene_str}'");
                return false;
            };
            if scene == SceneKind::Settings && !session.owns_compositor() {
                let _ = writeln!(stream, "err Settings requires a standalone session");
            } else if scene == SceneKind::Drawer {
                write_show_drawer(&mut stream, cmd_tx, session, overlay_gap_gestures);
            } else if scene == SceneKind::Osk {
                let (reply, result) = mpsc::channel();
                if cmd_tx.send(DaemonCommand::ShowKeyboard(reply)).is_err() {
                    let _ = writeln!(stream, "err daemon not running");
                } else {
                    let _ = match result.recv_timeout(Duration::from_secs(1)) {
                        Ok(Ok(())) => writeln!(stream, "ok scene osk"),
                        Ok(Err(reason)) => writeln!(stream, "err cannot show osk: {reason}"),
                        Err(_) => writeln!(stream, "err show timed out"),
                    };
                }
            } else {
                let _ = cmd_tx.send(DaemonCommand::ShowScene(scene));
                let _ = writeln!(stream, "ok scene {}", scene.as_str());
            }
        }
        "reload-theme" => {
            let palette = session.palette();
            let name = palette.name.clone();
            let _ = cmd_tx.send(DaemonCommand::UpdatePalette(palette));
            let _ = writeln!(stream, "ok theme {name}");
        }
        "wallpaper" => handle_wallpaper_cmd(&mut stream, trimmed, cmd_tx),
        "status" => write_status(&mut stream, cmd_tx),
        "mode" => {
            let (tx, rx) = mpsc::channel();
            if cmd_tx
                .send(DaemonCommand::Custom {
                    cmd: trimmed.to_owned(),
                    reply: tx,
                })
                .is_ok()
            {
                if let Ok(msg) = rx.recv_timeout(Duration::from_secs(2)) {
                    let _ = writeln!(stream, "{msg}");
                } else {
                    let _ = writeln!(stream, "err mode timeout");
                }
            }
        }
        "quit" => {
            let _ = cmd_tx.send(DaemonCommand::Quit);
            let _ = writeln!(stream, "ok quitting");
            return true;
        }
        other => {
            let _ = writeln!(stream, "err unknown command '{other}'");
        }
    }
    false
}

fn write_show_drawer(
    stream: &mut UnixStream,
    cmd_tx: &Sender<DaemonCommand>,
    session: ShellSession,
    overlay_gap_gestures: bool,
) {
    const UNAVAILABLE: &str = "err Drawer requires standalone Tablet mode";
    if !session.owns_compositor() && !overlay_gap_gestures {
        let _ = writeln!(stream, "{UNAVAILABLE}");
        return;
    }

    let (tx, rx) = mpsc::channel();
    if cmd_tx.send(DaemonCommand::QueryStatus(tx)).is_err() {
        let _ = writeln!(stream, "err daemon not running");
        return;
    }
    let Ok(status) = rx.recv_timeout(Duration::from_secs(1)) else {
        let _ = writeln!(stream, "err show timed out");
        return;
    };
    if !status.tablet_navigation {
        let reason = if overlay_gap_gestures {
            "err Drawer requires a visible touch navigation overlay"
        } else {
            UNAVAILABLE
        };
        let _ = writeln!(stream, "{reason}");
        return;
    }
    // The Wayland handler also gates Drawer at execution time (mode can change
    // after the status reply through hardware or the shell UI).
    if cmd_tx
        .send(DaemonCommand::ShowScene(SceneKind::Drawer))
        .is_ok()
    {
        let _ = writeln!(stream, "ok scene drawer");
    } else {
        let _ = writeln!(stream, "err daemon not running");
    }
}

fn write_status(stream: &mut UnixStream, cmd_tx: &Sender<DaemonCommand>) {
    let (tx, rx) = mpsc::channel();
    if cmd_tx.send(DaemonCommand::QueryStatus(tx)).is_err() {
        return;
    }
    let Ok(st) = rx.recv_timeout(Duration::from_secs(1)) else {
        let _ = writeln!(stream, "err status timeout");
        return;
    };
    let _ = writeln!(
        stream,
        "daemon     : running (scene: {}, visible: {}, output: {} {}x{}, renderer: {})",
        st.active_scene.as_str(),
        st.visible,
        st.output_name.as_deref().unwrap_or("default"),
        st.width,
        st.height,
        st.renderer
    );
    if let Some(layout) = st.standalone_layout {
        let _ = writeln!(stream, "  session    : standalone ({layout})");
    }
    if let Some(layout) = st.tablet_shade_layout {
        let _ = writeln!(stream, "  tablet shade: {layout}");
    }
    if let Some((top, bottom)) = st.overlay_gaps {
        let _ = writeln!(
            stream,
            "  overlay gaps: top {top}px, bottom {bottom}px (logical)"
        );
    }
}

fn handle_wallpaper_cmd(stream: &mut UnixStream, trimmed: &str, cmd_tx: &Sender<DaemonCommand>) {
    let arg = trimmed.strip_prefix("wallpaper").map_or("", str::trim);
    if arg.is_empty() {
        let _ = writeln!(stream, "err usage: wallpaper <PATH|clear>");
    } else if arg.eq_ignore_ascii_case("clear") || arg.eq_ignore_ascii_case("none") {
        let _ = cmd_tx.send(DaemonCommand::SetWallpaper(None));
        let _ = writeln!(stream, "ok wallpaper cleared");
    } else {
        match WallpaperImage::load_png(arg) {
            Ok(img) => {
                let (w, h) = (img.width, img.height);
                let _ = cmd_tx.send(DaemonCommand::SetWallpaper(Some(img)));
                let _ = writeln!(stream, "ok wallpaper {arg} ({w}x{h})");
            }
            Err(err) => {
                let _ = writeln!(stream, "err wallpaper '{arg}': {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use veilshell_wayland::{channel, RuntimeStatus};

    #[test]
    fn a_second_daemon_does_not_unlink_an_existing_control_socket() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let path = std::env::temp_dir().join(format!("veilshell-test-{}.sock", now.as_nanos()));
        let original = UnixListener::bind(&path).unwrap();
        let (commands, _) = channel::channel();
        let failure =
            spawn_ipc_listener(&path, commands, ShellSession::Overlay, false).unwrap_err();
        let can_connect = UnixStream::connect(&path).is_ok();
        drop(original);
        fs::remove_file(&path).unwrap();
        assert_eq!(failure.kind(), std::io::ErrorKind::AddrInUse);
        assert!(can_connect);
    }

    #[test]
    fn drawer_show_rejects_overlay_without_dispatching() {
        let (commands, events) = channel::channel();
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(b"show drawer\n").unwrap();
        assert!(!handle_client(
            server,
            &commands,
            ShellSession::Overlay,
            false
        ));
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        assert_eq!(reply, "err Drawer requires standalone Tablet mode\n");
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn drawer_show_requires_runtime_touch_navigation_capability() {
        for (session, layout, gap_depths, visible, allowed) in [
            (ShellSession::Standalone, Some("Desk"), None, true, false),
            (ShellSession::Standalone, Some("Tablet"), None, false, false),
            (ShellSession::Standalone, Some("Tablet"), None, true, true),
            (ShellSession::Overlay, None, Some((0, 0)), true, true),
            (ShellSession::Overlay, None, Some((0, 0)), false, false),
        ] {
            let (commands, events) = channel::channel();
            let (mut client, server) = UnixStream::pair().unwrap();
            client.write_all(b"show drawer\n").unwrap();
            let handle = thread::spawn(move || {
                assert!(!handle_client(
                    server,
                    &commands,
                    session,
                    gap_depths.is_some()
                ));
            });
            // Keep the test bounded even if the handler stops querying status.
            let DaemonCommand::QueryStatus(status) = events.recv().unwrap() else {
                panic!("Drawer must check whether Tablet navigation is enabled");
            };
            status
                .send(RuntimeStatus {
                    active_scene: SceneKind::Pill,
                    visible,
                    output_name: None,
                    width: 960,
                    height: 640,
                    renderer: "test".into(),
                    standalone_layout: layout,
                    tablet_shade_layout: None,
                    tablet_navigation: allowed,
                    overlay_gaps: gap_depths,
                })
                .unwrap();
            handle.join().unwrap();
            let mut reply = String::new();
            client.read_to_string(&mut reply).unwrap();
            if allowed {
                assert_eq!(reply, "ok scene drawer\n");
                assert!(matches!(
                    events.try_recv().unwrap(),
                    DaemonCommand::ShowScene(SceneKind::Drawer)
                ));
            } else if session.owns_compositor() {
                assert_eq!(reply, "err Drawer requires standalone Tablet mode\n");
            } else {
                assert_eq!(
                    reply,
                    "err Drawer requires a visible touch navigation overlay\n"
                );
            }
            assert!(events.try_recv().is_err());
        }
    }

    #[test]
    fn status_reports_logical_strip_depths_only_for_the_opt_in() {
        for (gap_depths, expected) in [
            (None, None),
            (
                Some((0, 0)),
                Some("  overlay gaps: top 0px, bottom 0px (logical)"),
            ),
            (
                Some((5, 12)),
                Some("  overlay gaps: top 5px, bottom 12px (logical)"),
            ),
        ] {
            let (commands, events) = channel::channel();
            let (mut client, server) = UnixStream::pair().unwrap();
            client.write_all(b"status\n").unwrap();
            let handle = thread::spawn(move || {
                assert!(!handle_client(
                    server,
                    &commands,
                    ShellSession::Overlay,
                    gap_depths.is_some()
                ));
            });
            let DaemonCommand::QueryStatus(status) = events.recv().unwrap() else {
                panic!("expected a status request");
            };
            status
                .send(RuntimeStatus {
                    active_scene: SceneKind::Pill,
                    visible: true,
                    output_name: Some("eDP-1".into()),
                    width: 960,
                    height: 640,
                    renderer: "test".into(),
                    standalone_layout: None,
                    tablet_shade_layout: None,
                    tablet_navigation: gap_depths.is_some(),
                    overlay_gaps: gap_depths,
                })
                .unwrap();
            handle.join().unwrap();
            let mut reply = String::new();
            client.read_to_string(&mut reply).unwrap();
            assert_eq!(
                reply.lines().find(|line| line.contains("overlay gaps")),
                expected
            );
        }
    }
}
