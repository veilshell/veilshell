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
                if handle_client(stream, &cmd_tx, session) {
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
                let _ = writeln!(stream, "err usage: show <pill|home|recents|shade|osk>");
                return false;
            };
            let Some(scene) = SceneKind::parse_name(scene_str) else {
                let _ = writeln!(stream, "err unknown scene '{scene_str}'");
                return false;
            };
            let _ = cmd_tx.send(DaemonCommand::ShowScene(scene));
            let _ = writeln!(stream, "ok scene {}", scene.as_str());
        }
        "reload-theme" => {
            let palette = session.palette();
            let name = palette.name.clone();
            let _ = cmd_tx.send(DaemonCommand::UpdatePalette(palette));
            let _ = writeln!(stream, "ok theme {name}");
        }
        "wallpaper" => handle_wallpaper_cmd(&mut stream, trimmed, cmd_tx),
        "status" => {
            let (tx, rx) = mpsc::channel();
            if cmd_tx.send(DaemonCommand::QueryStatus(tx)).is_ok() {
                if let Ok(st) = rx.recv_timeout(Duration::from_secs(1)) {
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
                } else {
                    let _ = writeln!(stream, "err status timeout");
                }
            }
        }
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
    use veilshell_wayland::channel;

    #[test]
    fn a_second_daemon_does_not_unlink_an_existing_control_socket() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let path = std::env::temp_dir().join(format!("veilshell-test-{}.sock", now.as_nanos()));
        let original = UnixListener::bind(&path).unwrap();
        let (commands, _) = channel::channel();
        let failure = spawn_ipc_listener(&path, commands, ShellSession::Overlay).unwrap_err();
        let can_connect = UnixStream::connect(&path).is_ok();
        drop(original);
        fs::remove_file(&path).unwrap();
        assert_eq!(failure.kind(), std::io::ErrorKind::AddrInUse);
        assert!(can_connect);
    }
}
