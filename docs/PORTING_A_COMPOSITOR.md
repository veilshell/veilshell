# Porting VeilShell to Another Wayland Compositor

VeilShell isolates every compositor-specific call inside `crates/veilshell-compositor`. Any `wlr-layer-shell` compositor (`Hyprland`, `Niri`, `Sway`, `River`, `labwc`, etc.) can be supported by implementing one trait:

```rust
pub trait CompositorBackend {
    fn backend_name(&self) -> &'static str;
    fn list_windows(&self) -> Result<Vec<WindowInfo>, CompositorError>;
    fn focus_window(&mut self, id: &WindowId) -> Result<(), CompositorError>;
    fn close_window(&mut self, id: &WindowId) -> Result<(), CompositorError>;
    fn list_outputs(&self) -> Result<Vec<OutputInfo>, CompositorError>;
    fn set_output_orientation(
        &mut self,
        output: &str,
        orientation: Orientation,
    ) -> Result<(), CompositorError>;
    fn apply_shell_mode(&mut self, mode: ShellMode) -> Result<(), CompositorError>;
}
```

## Steps

1. Add a new module under `crates/veilshell-compositor/src/` (e.g. `niri.rs` or `sway.rs`) behind an optional Cargo feature if it adds dependencies.
2. Implement `CompositorBackend` using the compositor's IPC socket or standard Wayland protocols (`ext-foreign-toplevel-list-v1` + `wlr-output-management-v1`).
3. Map `veilshell_core::Orientation` to your compositor's output transform values.
4. Wire backend auto-detection in `crates/veilshell-shell`.
5. Add unit tests for IPC parsing and a headless integration test if your compositor supports headless output backends.
