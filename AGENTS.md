# VeilShell contributor workflow

## Feature analysis and implementation

1. **Analyze and plan with Muse Spark.** Delegate feature analysis and
   implementation planning to `opencode-go/muse-spark-1.3-contributor` before
   writing feature code. Base upstream behavior claims only on public upstream
   source. For Tablet parity, use the pins in
   [the Tablet index](docs/tablet/README.md). In one linked behavior document
   per feature, compare both pins, link the reachable source,
   and spell out defaults or opt-ins, commit and cancellation behavior,
   acceptance cases, and compositor limitations. Denial mobile is the
   behavior and visual reference; Penumbra is a Rust implementation reference
   only. Keep the Penumbra checkout local for inspection, not as a dependency.
2. **Implement with `general`.** Review the analysis and hand each verified
   implementation slice, its behavior document, and its acceptance cases to
   a `general` subagent. Keep phases separate: Muse Spark does not implement
   the feature, and `general` does not substitute for the source audit. If
   the analysis cannot be done with the specified model, report that rather
   than silently substituting another.
3. **Review and verify.** Review the changes against the scoped behavior and
   keep regression tests and evidence replayable. Distinguish source tracing,
   unit tests, virtual pointer, SDDM guest touch, and physical-device evidence.

## Standalone Tablet parity

- Target the authenticated SDDM Tablet session in portrait and landscape.
  Leave Desk Mode, the Omarchy overlay, and the SDDM greeter unchanged.
- Use isolated nested compositors and disposable SDDM guests with QMP touch.
  Do not change the active session, install on the host, or capture password
  entry. Keep credentials and guest logs out of the repository.
