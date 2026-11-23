use super::*;
use serde_json::{json, Value};

const MONITORS: &str = r#"[
    {"id": 43, "name": "HEADLESS-12"},
    {"id": 17, "name": "HEADLESS-1"},
    {"id": 0, "name": "HDMI-A-1"},
    {"name": "missing-id"},
    {"id": -1, "name": "missing-monitor"},
    {"id": 11, "name": "disabled", "disabled": true}
]"#;

fn client(address: &str, monitor: i64, workspace: Value, rank: i32) -> Value {
    let mut client = json!({
        "address": address,
        "title": address,
        "class": "example",
        "floating": false,
        "mapped": true,
        "monitor": monitor,
        "focusHistoryID": rank,
        "stableId": format!("stable-{address}"),
    });
    client["workspace"] = workspace;
    client
}

fn normal(address: &str, monitor: i64, rank: i32) -> Value {
    client(
        address,
        monitor,
        json!({"id": 1, "type": "normal", "name": "1"}),
        rank,
    )
}

fn snapshot(clients: &[Value], active: &Value) -> String {
    format!(
        "{MONITORS}\n\n\n{}\n\n\n{active}",
        serde_json::to_string_pretty(clients).unwrap()
    )
}

fn ids(windows: &[WindowInfo]) -> Vec<&str> {
    windows.iter().map(|w| w.id.0.as_str()).collect()
}

#[test]
fn tablet_uses_exact_monitor_id_and_requires_a_connected_monitor() {
    assert_eq!(tablet_monitor_id(MONITORS, "HEADLESS-1").unwrap(), 17);
    assert_eq!(tablet_monitor_id(MONITORS, "HDMI-A-1").unwrap(), 0);
    for unknown in [
        "HEADLESS",
        "headless-1",
        "missing-id",
        "missing-monitor",
        "disabled",
    ] {
        assert!(matches!(
            tablet_monitor_id(MONITORS, unknown),
            Err(CompositorError::NotFound(name)) if name == unknown
        ));
    }
    assert!(matches!(
        parse_tablet_snapshot("invalid or incomplete reply", "HEADLESS-1"),
        Err(CompositorError::Protocol(_))
    ));
}

#[test]
fn tablet_filters_to_mapped_normal_windows_on_the_gesture_output() {
    let regular = normal("0x10", 17, 4);
    let named = client("0x20", 17, json!({"type": "normal", "name": "Notes"}), 2);
    let legacy_named = client("0x21", 17, json!({"id": -1337, "name": "Mail"}), 1);
    let special = client("0x30", 17, json!({"type": "special", "name": "Scratch"}), 0);
    let legacy_special = client("0x31", 17, json!({"name": "special:Scratch"}), 7);
    let numeric_special = client("0x32", 17, json!({"id": -99, "name": "Scratch"}), 5);
    let no_workspace = client("0x33", 17, json!({"type": "", "name": ""}), -1);
    let mut unmapped = normal("0x40", 17, 3);
    unmapped["mapped"] = json!(false);
    let mut missing_mapped = normal("0x41", 17, 6);
    missing_mapped.as_object_mut().unwrap().remove("mapped");
    let mut missing_monitor = normal("0x50", 17, 8);
    missing_monitor.as_object_mut().unwrap().remove("monitor");
    let detached = normal("0x51", -1, 9);
    let desk = normal("0x60", 43, 10);
    let clients = [
        regular.clone(),
        named,
        legacy_named,
        special.clone(),
        legacy_special,
        numeric_special,
        no_workspace,
        unmapped,
        missing_mapped,
        missing_monitor,
        detached,
        desk,
    ];

    let tablet = parse_tablet_snapshot(&snapshot(&clients, &special), "HEADLESS-1").unwrap();
    assert_eq!(ids(&tablet), ["0x21", "0x20", "0x10"]);
    assert!(tablet.iter().all(|w| !w.focused));
    assert_eq!(tablet[1].workspace, "Notes");
    assert_eq!(tablet[1].foreign_id.as_deref(), Some("stable-0x20"));

    let desk = parse_tablet_snapshot(&snapshot(&clients, &regular), "HEADLESS-12").unwrap();
    assert_eq!(ids(&desk), ["0x60"]);
    assert!(!desk[0].focused);

    let global = parse_hyprland_clients_json(&json!(clients).to_string()).unwrap();
    // The legacy list still includes special workspaces, other monitors,
    // and mapped-by-default clients, with its original history-based flag.
    assert_eq!(global.len(), clients.len() - 1);
    assert_eq!(global[0].id.0, "0x30");
    assert!(global[0].focused);
    assert!(global.iter().any(|w| w.id.0 == "0x41"));
}

#[test]
fn tablet_focus_is_explicit_and_unranked_order_is_stable() {
    let newest = normal("0x60", 17, 0);
    let ranked_last = normal("0x50", 17, i32::MAX);
    let unranked = normal("0x40", 17, -2);
    let mut missing_rank = normal("0x30", 17, 99);
    missing_rank
        .as_object_mut()
        .unwrap()
        .remove("focusHistoryID");
    let desk = normal("0x20", 43, 0);
    let mut clients = [
        unranked,
        newest.clone(),
        ranked_last.clone(),
        missing_rank,
        desk.clone(),
    ];

    for active in [json!({}), desk] {
        let windows = parse_tablet_snapshot(&snapshot(&clients, &active), "HEADLESS-1").unwrap();
        assert_eq!(ids(&windows), ["0x60", "0x50", "0x30", "0x40"]);
        assert!(windows.iter().all(|w| !w.focused));
    }
    clients.reverse();
    let windows = parse_tablet_snapshot(&snapshot(&clients, &ranked_last), "HEADLESS-1").unwrap();
    assert_eq!(ids(&windows), ["0x50", "0x60", "0x30", "0x40"]);
    assert!(windows[0].focused);
    assert!(!windows[1].focused);

    let mut same_address = newest;
    same_address["stableId"] = json!("different-identity");
    let windows = parse_tablet_snapshot(&snapshot(&clients, &same_address), "HEADLESS-1").unwrap();
    assert!(windows.iter().all(|w| !w.focused));
}
