//! Freeze a sequence of adjacent-window swipes without reordering it on every raise.

use std::time::{Duration, Instant};

use veilshell_compositor::{CompositorBackend, CompositorError, WindowId, WindowInfo};

#[derive(Default)]
pub(super) struct AppSwitchOrder {
    held: Option<HeldOrder>,
}

struct HeldOrder {
    order: Vec<WindowIdentity>,
    at: Instant,
    output: String,
    // Only changes observable in compositor snapshots invalidate the hold.
    expected: Snapshot,
}

#[derive(Clone, Eq, PartialEq)]
struct WindowIdentity {
    id: WindowId,
    foreign_id: Option<String>,
}

impl From<&WindowInfo> for WindowIdentity {
    fn from(window: &WindowInfo) -> Self {
        Self {
            id: window.id.clone(),
            foreign_id: window.foreign_id.clone(),
        }
    }
}

#[derive(Eq, PartialEq)]
struct Snapshot {
    mru: Vec<WindowIdentity>,
    focused: Option<WindowIdentity>,
}

impl Snapshot {
    fn read(compositor: &impl CompositorBackend, output: &str) -> Result<Self, CompositorError> {
        let windows = compositor.list_tablet_windows(output)?;
        let mut focused = windows.iter().filter(|w| w.focused);
        let focused = focused
            .next()
            .filter(|_| focused.next().is_none())
            .map(WindowIdentity::from);
        Ok(Self {
            mru: windows.iter().map(WindowIdentity::from).collect(),
            focused,
        })
    }

    fn same_windows(&self, other: &Self) -> bool {
        self.mru.len() == other.mru.len() && self.mru.iter().all(|w| other.mru.contains(w))
    }
}

impl AppSwitchOrder {
    const HOLD: Duration = Duration::from_secs(5);

    pub(super) fn focus_adjacent<C: CompositorBackend>(
        &mut self,
        compositor: &mut C,
        output: &str,
        offset: i32,
        from_home: bool,
        clock: impl Fn() -> Instant,
    ) -> Result<bool, CompositorError> {
        if !matches!(offset, -1 | 1) {
            return Ok(false);
        }
        let previous = self.held.take();
        if from_home && offset == -1 {
            return Ok(false);
        }

        let before = Snapshot::read(compositor, output)?;
        let held = previous.filter(|held| {
            !from_home
                && held.output == output
                && clock().saturating_duration_since(held.at) < Self::HOLD
                && held.expected == before
        });
        let order = held.as_ref().map_or(&before.mru, |held| &held.order);
        let Some(target) = adjacent(order, before.focused.as_ref(), offset, from_home).cloned()
        else {
            // A swipe at the end does not extend the hold.
            self.held = held;
            return Ok(false);
        };

        // Reject a close, move to another output, address reuse, or other
        // snapshot change before dispatch. Query and focus are separate IPC
        // requests: Hyprland can still change between them.
        if Snapshot::read(compositor, output)? != before {
            return Ok(false);
        }
        compositor.focus_window(&target.id)?;
        let at = clock();
        let after = Snapshot::read(compositor, output)?;
        if after.focused.as_ref() != Some(&target) {
            return Ok(false);
        }
        // Keep the frozen order, but remember the actual post-focus MRU.
        // Hyprland may follow the target's workspace and update additional
        // history entries even though we sent one focus-by-address command.
        if after.same_windows(&before) {
            self.held = Some(HeldOrder {
                order: held.map_or(before.mru, |held| held.order),
                at,
                output: output.to_owned(),
                expected: after,
            });
        }
        Ok(true)
    }
}

fn adjacent<'a>(
    order: &'a [WindowIdentity],
    focused: Option<&WindowIdentity>,
    offset: i32,
    from_home: bool,
) -> Option<&'a WindowIdentity> {
    // The frozen row is MRU-first: +1 walks toward older, -1 toward newer.
    // Home sits just beyond the newest app, regardless of underlying focus.
    let index = if from_home {
        (offset == 1).then_some(0)?
    } else {
        let current = order.iter().position(|window| Some(window) == focused)?;
        current.checked_add_signed(offset as isize)?
    };
    order.get(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use veilshell_compositor::OutputInfo;
    use veilshell_core::{Orientation, ShellMode};

    const TABLET: &str = "Tablet";
    const DESK: &str = "Desk";

    #[derive(Clone)]
    struct Client {
        output: &'static str,
        window: WindowInfo,
    }

    impl Client {
        fn new(id: &str, output: &'static str, focused: bool) -> Self {
            Self {
                output,
                window: WindowInfo {
                    id: WindowId(id.into()),
                    title: id.into(),
                    app_id: "same-app".into(),
                    workspace: "1".into(),
                    focused,
                    floating: false,
                    foreign_id: Some(format!("stable-{id}")),
                },
            }
        }
    }

    struct MockCompositor {
        mru: Vec<Client>,
        focus_calls: Vec<String>,
        reads: Cell<usize>,
        second_snapshot: Option<Vec<Client>>,
        workspace_focus_candidate: Option<WindowId>,
        ignore_focus: bool,
    }

    impl MockCompositor {
        fn active(&self) -> Option<&str> {
            self.mru
                .iter()
                .find(|client| client.window.focused)
                .map(|client| client.window.id.0.as_str())
        }

        fn raise(&mut self, id: &WindowId) {
            let index = self.mru.iter().position(|c| &c.window.id == id).unwrap();
            self.mru[..=index].rotate_right(1);
            for (i, client) in self.mru.iter_mut().enumerate() {
                client.window.focused = i == 0;
            }
        }
    }

    impl CompositorBackend for MockCompositor {
        fn backend_name(&self) -> &'static str {
            "mock-tablet"
        }

        fn list_windows(&self) -> Result<Vec<WindowInfo>, CompositorError> {
            panic!("Tablet must never fall back to the global window list")
        }

        fn list_tablet_windows(&self, output: &str) -> Result<Vec<WindowInfo>, CompositorError> {
            if !matches!(output, TABLET | DESK) {
                return Err(CompositorError::NotFound(output.into()));
            }
            let reads = self.reads.get() + 1;
            self.reads.set(reads);
            let snapshot = if reads == 2 {
                self.second_snapshot.as_deref().unwrap_or(&self.mru)
            } else {
                &self.mru
            };
            Ok(snapshot
                .iter()
                .filter(|client| client.output == output)
                .map(|client| client.window.clone())
                .collect())
        }

        fn focus_window(&mut self, id: &WindowId) -> Result<(), CompositorError> {
            self.focus_calls.push(id.0.clone());
            if !self.ignore_focus {
                if let Some(candidate) = self.workspace_focus_candidate.take() {
                    self.raise(&candidate);
                }
                self.raise(id);
            }
            Ok(())
        }

        fn close_window(&mut self, _: &WindowId) -> Result<(), CompositorError> {
            panic!("a swipe must not close a client")
        }

        fn list_outputs(&self) -> Result<Vec<OutputInfo>, CompositorError> {
            panic!("output scope belongs to the backend's window query")
        }

        fn set_output_orientation(
            &mut self,
            _: &str,
            _: Orientation,
        ) -> Result<(), CompositorError> {
            panic!("a swipe must not change the output")
        }

        fn apply_shell_mode(&mut self, _: ShellMode) -> Result<(), CompositorError> {
            panic!("a swipe must not change mode")
        }

        fn switch_workspace_relative(&mut self, _: i32) -> Result<(), CompositorError> {
            panic!("a swipe must never dispatch a workspace command")
        }

        fn launch_command(&self, _: &str) -> Result<(), CompositorError> {
            panic!("a swipe must never dispatch a launcher command")
        }
    }

    struct Fixture {
        apps: AppSwitchOrder,
        compositor: MockCompositor,
        now: Instant,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                apps: AppSwitchOrder::default(),
                compositor: MockCompositor {
                    mru: vec![
                        Client::new("A", TABLET, true),
                        Client::new("B", TABLET, false),
                        Client::new("C", TABLET, false),
                        Client::new("D", DESK, false),
                    ],
                    reads: Cell::new(0),
                    focus_calls: Vec::new(),
                    second_snapshot: None,
                    workspace_focus_candidate: None,
                    ignore_focus: false,
                },
                now: Instant::now(),
            }
        }

        fn swipe(&mut self, offset: i32, from_home: bool) -> bool {
            self.apps
                .focus_adjacent(&mut self.compositor, TABLET, offset, from_home, || self.now)
                .unwrap()
        }
    }

    #[test]
    fn same_workspace_swipes_walk_the_frozen_row_without_bounce_or_wrap() {
        let mut f = Fixture::new();
        for (offset, accepted, expected) in [
            (-1, false, "A"),
            (1, true, "B"),
            (1, true, "C"),
            (1, false, "C"),
            (-1, true, "B"),
            (-1, true, "A"),
            (-1, false, "A"),
        ] {
            assert_eq!(f.swipe(offset, false), accepted);
            assert_eq!(f.compositor.active(), Some(expected));
            // The mock really reorders by MRU on each successful raise.
            assert_eq!(f.compositor.mru[0].window.id.0, expected);
        }
        assert_eq!(f.compositor.focus_calls, ["B", "C", "B", "A"]);
        assert!(f.compositor.mru.iter().all(|c| c.window.workspace == "1"));
    }

    #[test]
    fn a_local_neighbor_on_another_normal_workspace_is_still_a_window_focus() {
        let mut f = Fixture::new();
        f.compositor.mru[1].window.workspace = "2".into();
        f.compositor.mru[2].window.workspace = "2".into();
        f.compositor.workspace_focus_candidate = Some(WindowId("C".into()));
        assert!(f.swipe(1, false));
        assert_eq!(f.compositor.focus_calls, ["B"]);
        assert_eq!(f.compositor.mru[0].window.workspace, "2");
        assert!(f.swipe(1, false)); // Continue to C in the original deck.
        assert!(!f.swipe(1, false)); // A cannot become a second 'older' target.
        assert_eq!(f.compositor.focus_calls, ["B", "C"]);
    }

    #[test]
    fn hold_expires_five_seconds_after_the_last_success_not_an_end_swipe() {
        let mut f = Fixture::new();
        assert!(f.swipe(1, false)); // B
        f.now += Duration::from_secs(4);
        assert!(f.swipe(1, false)); // C, extending the hold
        f.now += Duration::from_secs(4);
        assert!(!f.swipe(1, false)); // still at the oldest end
        f.now += Duration::from_secs(1);
        assert!(f.swipe(1, false)); // rebuilt MRU = [C, B, A]
        assert_eq!(f.compositor.focus_calls, ["B", "C", "B"]);
    }

    #[test]
    fn external_focus_rebuilds_even_after_a_return_to_the_same_window() {
        let mut f = Fixture::new();
        assert!(f.swipe(1, false)); // B
        f.compositor.raise(&WindowId("C".into()));
        f.compositor.raise(&WindowId("B".into()));
        assert!(!f.swipe(-1, false)); // B is newest in the refreshed MRU
        assert!(f.swipe(1, false)); // C
        f.compositor.raise(&WindowId("A".into()));
        assert!(f.swipe(1, false)); // C again; A is now newest
        assert_eq!(f.compositor.focus_calls, ["B", "C", "C"]);
    }

    #[test]
    fn closed_and_added_windows_rebuild_even_at_the_same_count() {
        let mut f = Fixture::new();
        assert!(f.swipe(1, false)); // B
        f.compositor.mru.retain(|c| c.window.id.0 != "C");
        f.compositor.mru.push(Client::new("E", TABLET, false));
        assert!(f.swipe(1, false)); // A, from [B, A, E], not stale C
        assert!(f.swipe(1, false)); // E
        f.compositor.mru.retain(|c| c.window.id.0 != "A");
        assert!(f.swipe(1, false)); // B, from [E, B]
        assert_eq!(f.compositor.focus_calls, ["B", "A", "E", "B"]);
    }

    #[test]
    fn app_swipes_require_an_actual_focus_on_the_named_output() {
        let mut f = Fixture::new();
        f.compositor.raise(&WindowId("D".into()));
        assert!(!f.swipe(1, false)); // A is locally most recent, but not focused.
        f.compositor.mru[0].window.focused = false;
        assert!(!f.swipe(1, false));
        assert!(matches!(
            f.apps.focus_adjacent(&mut f.compositor, "Table", 1, true, || f.now),
            Err(CompositorError::NotFound(output)) if output == "Table"
        ));
        assert_eq!(f.compositor.focus_calls, Vec::<String>::new());
    }

    #[test]
    fn home_enters_only_the_local_most_recent_window_toward_older() {
        let mut f = Fixture::new();
        f.compositor.raise(&WindowId("B".into()));
        f.compositor.raise(&WindowId("D".into()));
        assert!(!f.swipe(-1, true));
        assert!(f.swipe(1, true));
        assert_eq!(f.compositor.active(), Some("B"));
        f.compositor.mru.retain(|c| c.output != TABLET);
        assert!(!f.swipe(1, true));
        assert_eq!(f.compositor.focus_calls, ["B"]);
    }

    #[test]
    fn target_closed_moved_or_replaced_before_dispatch_is_not_focused() {
        for change in 0..3 {
            let mut f = Fixture::new();
            let mut second = f.compositor.mru.clone();
            match change {
                0 => drop(second.remove(1)), // B closes
                1 => second[1].output = DESK,
                _ => second[1].window.foreign_id = Some("reused-address".into()),
            }
            f.compositor.second_snapshot = Some(second);
            assert!(!f.swipe(1, false));
            assert_eq!(f.compositor.focus_calls, Vec::<String>::new());
        }
    }

    #[test]
    fn a_dispatch_ack_without_the_requested_focus_does_not_leave_home() {
        let mut f = Fixture::new();
        f.compositor.raise(&WindowId("D".into()));
        f.compositor.ignore_focus = true;
        assert!(!f.swipe(1, true));
        assert_eq!(f.compositor.active(), Some("D"));
    }
}
