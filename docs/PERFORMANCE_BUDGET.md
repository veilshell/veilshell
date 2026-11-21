# VeilShell Performance Budget

VeilShell targets **older, fanless, 5W x86 & ARM tablets** as first-class
hardware — not just flagship laptops and smartphones.

## Reference Low-End Target Device

- **Device:** Microsoft Surface Go 3
- **CPU:** Intel Core i3-10100Y (2C/4T, 5W TDP)
- **GPU:** Intel UHD Graphics 615 (`i915` / Vulkan `hasvk`)
- **Display:** 10.5" `1920x1280 @ 60Hz` (`scale 1.5` -> `1280x853` logical)
- **Memory:** `7.7 GiB` RAM

## Hard Budgets

| Metric | Target | Rationale |
|---|---|---|
| Idle CPU (Laptop Mode or hidden) | `0.0%` | Must never wake a fanless tablet CPU |
| Active RAM (RSS, all surfaces loaded) | `< 40 MiB` | Leave headroom for browsers & Android apps |
| Animation frame time (p95 / p99) | `<= 10 ms` / `<= 16.6 ms` | Locked 60 FPS on UHD 615 |
| Wallpaper glass cost | Blur **once** on change | Live per-frame Gaussian blur stalls low-end iGPUs |
| Mode transition (Laptop <-> Tablet) | `< 150 ms` | Instant feedback when detaching a keyboard |
| Cold startup time | `< 200 ms` | Snappy SDDM / session launch |

Every PR touching `veilshell-render`, `veilshell-wayland`, or `veilshell-shell`
should keep these budgets green and verify visual regressions via
`./scripts/test-snapshots.sh` and `./scripts/test-headless-hyprland.sh`.
