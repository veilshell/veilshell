//! Conservative, read-only sizing of below-window overlay strips on Hyprland.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

const GAP_OPTION: &str = "general:gaps_out";
const RESIZE_OPTION: &str = "general:resize_on_border";
const SWIPE_OPTION: &str = "gestures:workspace_swipe_touch";
const SHRINK_OPTION: &str = "input:follow_mouse_shrink";
const QUERY: &str = concat!(
    "[[BATCH]]j/getoption general:gaps_out;",
    "j/getoption general:resize_on_border;",
    "j/getoption gestures:workspace_swipe_touch;",
    "j/getoption input:follow_mouse_shrink"
);
const MAX_REPLY_BYTES: usize = 4096;
const QUERY_TIMEOUT: Duration = Duration::from_millis(100);
const DEPTH_CAP: i64 = 12;

/// Exists only in the opt-in backend. The Wayland caller decides when the
/// strips need sizing; this does not start a poller or cache a failed query.
#[derive(Clone, Debug, Default)]
pub(super) struct GapQuery(Arc<Mutex<()>>);

impl GapQuery {
    pub(super) fn depths(&self, socket_path: &Path) -> (u32, u32) {
        let (tx, rx) = mpsc::sync_channel(1);
        let in_flight = Arc::clone(&self.0);
        let socket_path = socket_path.to_owned();
        let deadline = Instant::now() + QUERY_TIMEOUT;
        // UnixStream's I/O timeouts don't cover connect (e.g. a stalled
        // compositor with a full accept queue). Bound the caller's wait and
        // don't start another connection while one is stuck.
        let spawn = thread::Builder::new()
            .name("veilshell-gap-ipc".into())
            .spawn(move || {
                let Ok(_request) = in_flight.try_lock() else {
                    return;
                };
                let depths = remaining(deadline)
                    .and_then(|_| UnixStream::connect(socket_path).ok())
                    .and_then(|socket| query_depths(socket, deadline))
                    .unwrap_or_default();
                let _ = tx.send(depths);
            });
        if spawn.is_err() {
            return (0, 0);
        }
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_default()
    }
}

fn remaining(deadline: Instant) -> Option<Duration> {
    let budget = deadline.checked_duration_since(Instant::now())?;
    (!budget.is_zero()).then_some(budget)
}

fn query_depths(mut socket: UnixStream, deadline: Instant) -> Option<(u32, u32)> {
    socket.set_write_timeout(Some(remaining(deadline)?)).ok()?;
    socket.write_all(QUERY.as_bytes()).ok()?;
    let mut reply = [0; MAX_REPLY_BYTES];
    let mut filled = 0;
    while filled < reply.len() {
        socket.set_read_timeout(Some(remaining(deadline)?)).ok()?;
        let read = socket.read(&mut reply[filled..]).ok()?;
        if read == 0 {
            return parse_depths(std::str::from_utf8(&reply[..filled]).ok()?);
        }
        filled += read;
    }
    None
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionReply {
    option: String,
    #[serde(rename = "set")]
    _set: Option<bool>,
    #[serde(rename = "bool", default, deserialize_with = "present")]
    boolean: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    int: Option<i64>,
    // The legacy parser reports the same CSS-gap data as "custom".
    #[serde(alias = "custom", default, deserialize_with = "present")]
    css: Option<String>,
}

fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

enum OptionValue {
    Bool(bool),
    Int(i64),
    Css(String),
}

fn option_value(raw: &str, key: &str) -> Option<OptionValue> {
    let reply: OptionReply = serde_json::from_str(raw).ok()?;
    if reply.option != key {
        return None;
    }
    match (reply.boolean, reply.int, reply.css) {
        (Some(value), None, None) => Some(OptionValue::Bool(value)),
        (None, Some(value), None) => Some(OptionValue::Int(value)),
        (None, None, Some(value)) => Some(OptionValue::Css(value)),
        _ => None,
    }
}

fn parse_depths(raw: &str) -> Option<(u32, u32)> {
    let mut replies = raw.split("\n\n\n");
    let OptionValue::Css(css) = option_value(replies.next()?, GAP_OPTION)? else {
        return None;
    };
    let resize = option_value(replies.next()?, RESIZE_OPTION)?;
    let swipe = option_value(replies.next()?, SWIPE_OPTION)?;
    let shrink = option_value(replies.next()?, SHRINK_OPTION)?;
    let disabled = |value| matches!(value, OptionValue::Bool(false) | OptionValue::Int(0));
    // Even at Layer::Bottom, a click delivered to a layer surface can
    // suppress Hyprland's border resize. Don't arm either strip if a
    // conflicting native setting is enabled or cannot be verified.
    if !disabled(resize)
        || !disabled(swipe)
        || !matches!(shrink, OptionValue::Int(0))
        || replies.next().is_some()
    {
        return None;
    }
    css_depths(&css)
}

fn css_depths(raw: &str) -> Option<(u32, u32)> {
    let mut sides = raw.split_ascii_whitespace().map(str::parse::<i64>);
    let top = sides.next()?.ok()?;
    let _right = sides.next().transpose().ok()?;
    let bottom = sides.next().transpose().ok()?.unwrap_or(top);
    let _left = sides.next().transpose().ok()?;
    if sides.next().is_some() {
        return None;
    }
    let depth = |pixels: i64| pixels.clamp(0, DEPTH_CAP) as u32;
    Some((depth(top), depth(bottom)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_output_snapshot;
    use crate::{CompositorBackend, HyprlandBackend, MockBackend};
    use serde_json::{json, Value};

    const GAPS: &str = r#"{"option":"general:gaps_out","css":"10 22 8 2","set":true}"#;
    const RESIZE: &str = r#"{"option":"general:resize_on_border","bool":false,"set":false}"#;
    const SWIPE: &str = r#"{"option":"gestures:workspace_swipe_touch","int":0,"set":false}"#;
    const SHRINK: &str = r#"{"option":"input:follow_mouse_shrink","int":0,"set":false}"#;

    fn batch(replies: &[&str]) -> String {
        replies.join("\n\n\n")
    }

    #[test]
    fn overlay_gap_css_expands_and_clamps_each_edge() {
        for (css, depths) in [
            ("20", (12, 12)),
            ("8 3", (8, 8)),
            ("5 1 11", (5, 11)),
            (" 0 50 6 70 ", (0, 6)),
            ("6 50 -3 70", (6, 0)),
            ("-1", (0, 0)),
        ] {
            assert_eq!(css_depths(css), Some(depths), "{css}");
        }
        for css in ["", "7px", "7 no 9", "7 3 9 NaN", "1 2 3 4 5", "7 1e10"] {
            assert_eq!(css_depths(css), None, "{css}");
        }
    }

    #[test]
    fn overlay_gap_batch_rejects_native_conflicts_and_uncertain_replies() {
        assert_eq!(
            parse_depths(&batch(&[GAPS, RESIZE, SWIPE, SHRINK])),
            Some((10, 8))
        );
        assert_eq!(
            parse_depths(&batch(&[
                &GAPS.replace("css", "custom"),
                RESIZE,
                SWIPE,
                SHRINK
            ])),
            Some((10, 8))
        );
        assert_eq!(
            parse_depths(&batch(&[
                r#"{"option":"general:gaps_out","css":"\t10 22 8 2 "}"#,
                RESIZE,
                SWIPE,
                SHRINK,
            ])),
            Some((10, 8))
        );
        let good = [GAPS, RESIZE, SWIPE, SHRINK];
        for (index, unsafe_option) in [
            (0, r#"{"option":"general:gaps_out","int":12}"#),
            (1, r#"{"option":"general:resize_on_border","bool":true}"#),
            (
                1,
                r#"{"option":"general:resize_on_border","bool":false,"int":null}"#,
            ),
            (
                1,
                r#"{"option":"general:resize_on_border","bool":true,"bool":false}"#,
            ),
            (2, r#"{"option":"gestures:workspace_swipe_touch","int":1}"#),
            (
                2,
                r#"{"option":"gestures:workspace_swipe_touch","int":"0"}"#,
            ),
            (3, r#"{"option":"input:follow_mouse_shrink","int":1}"#),
            (3, r#"{"option":"input:follow_mouse_shrink","bool":false}"#),
        ] {
            let mut replies = good;
            replies[index] = unsafe_option;
            assert_eq!(parse_depths(&batch(&replies)), None, "{unsafe_option}");
        }
        for index in 0..good.len() {
            let mut unknown = good;
            unknown[index] = "no such option";
            assert_eq!(parse_depths(&batch(&unknown)), None);
        }
        assert_eq!(parse_depths(&batch(&[GAPS, RESIZE, SWIPE])), None);
        assert_eq!(parse_depths(&batch(&[GAPS, SWIPE, RESIZE, SHRINK])), None);
        assert_eq!(
            parse_depths(&batch(&[GAPS, RESIZE, SWIPE, SHRINK, "{}"])),
            None
        );
    }

    #[test]
    fn overlay_gap_query_is_one_read_only_batch() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let worker = thread::spawn(move || {
            let mut request = [0; QUERY.len()];
            server.read_exact(&mut request).unwrap();
            assert_eq!(
                std::str::from_utf8(&request).unwrap(),
                "[[BATCH]]j/getoption general:gaps_out;\
                 j/getoption general:resize_on_border;\
                 j/getoption gestures:workspace_swipe_touch;\
                 j/getoption input:follow_mouse_shrink"
            );
            server
                .write_all(batch(&[GAPS, RESIZE, SWIPE, SHRINK]).as_bytes())
                .unwrap();
        });
        assert_eq!(
            query_depths(client, Instant::now() + Duration::from_secs(1)),
            Some((10, 8))
        );
        worker.join().unwrap();
    }

    #[test]
    fn overlay_gap_query_bounds_reply_size_and_wait() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let mut reply = batch(&[GAPS, RESIZE, SWIPE, SHRINK]).into_bytes();
        reply.resize(MAX_REPLY_BYTES, b' ');
        server.write_all(&reply).unwrap();
        assert_eq!(
            query_depths(client, Instant::now() + Duration::from_secs(1)),
            None
        );
        let (client, _stalled_server) = UnixStream::pair().unwrap();
        assert_eq!(
            query_depths(client, Instant::now() + Duration::from_millis(5)),
            None
        );
    }

    #[test]
    fn unsupported_and_failed_gap_queries_disable_both_strips() {
        assert_eq!(MockBackend::default().overlay_gap_depths(), (0, 0));
        let backend = HyprlandBackend::from_parts("/dev/null", "not-a-hyprland-socket");
        assert_eq!(backend.overlay_gap_depths(), (0, 0));
        assert_eq!(
            backend.with_overlay_gap_gestures().overlay_gap_depths(),
            (0, 0)
        );
    }

    fn workspace_scope(active: &Value, special: &Value, other: &Value) {
        let window = |id, output, workspace, hidden, accepts_input| {
            json!({
                "address": id, "title": id, "class": "app",
                "workspace": workspace, "monitor": output, "mapped": true,
                "floating": false, "hidden": hidden, "acceptsInput": accepts_input,
            })
        };
        let visible = window("0xa", 3, active, false, true);
        let hidden = window("0xb", 3, active, true, true);
        let unfocusable = window("0xc", 3, active, false, false);
        let inactive_workspace = window("0xd", 3, other, false, true);
        let another_monitor = window("0xe", 8, active, false, true);
        let snapshot = |special: &Value| {
            let monitors = json!([{
                "id": 3, "name": "eDP-1", "activeWorkspace": active, "specialWorkspace": special
            }]);
            let clients = json!([
                visible,
                hidden,
                unfocusable,
                inactive_workspace,
                another_monitor
            ]);
            format!("{monitors}\n\n\n{clients}\n\n\n{visible}")
        };
        let scoped =
            |special: &Value| parse_output_snapshot(&snapshot(special), "eDP-1", true).unwrap();
        let local = scoped(special);
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].id.0, "0xa");
        assert!(local[0].focused);
        assert_eq!(scoped(&json!({"id": -3, "name": "special:scratch"})), []);
        assert_eq!(scoped(&Value::Null), []);
        assert_eq!(
            parse_output_snapshot(&snapshot(special), "eDP-1", false)
                .unwrap()
                .len(),
            4,
            "standalone Tablet must retain its existing window scope"
        );
    }

    #[test]
    fn overlay_snapshots_keep_only_focusable_windows_on_the_current_normal_workspace() {
        // v0.56.2 uses IDs, including a negative ID for a named workspace.
        workspace_scope(
            &json!({"id": -1337, "name": "Notes"}),
            &json!({"id": 0, "name": ""}),
            &json!({"id": 3, "name": "Notes"}),
        );
        // v0.57.2 has no numeric ID on named workspaces. The display name
        // alone is not enough to distinguish different workspace addresses.
        workspace_scope(
            &json!({"address": "notes", "type": "normal", "name": "Notes"}),
            &json!({"address": "", "type": "", "name": ""}),
            &json!({"address": "notes-2", "type": "normal", "name": "Notes"}),
        );
    }
}
