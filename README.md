# R-Helper

This fork is maintained at [brandonjank/r-helper](https://github.com/brandonjank/r-helper),
based on [Fatalution/r-helper](https://github.com/Fatalution/r-helper).

## Blade 18 (2025)

Version 0.5.0 adds firmware keyboard effects and experimental per-key colors,
AC/battery lighting presets and power policies, named application profiles,
charge-once request/cancel, and guarded display changes with color-profile information.
See [the feature batch and validation limits](docs/blade18-feature-batch.md).

The `RZ09-0529` / USB `1532:02c7` opens a model-specific control screen with:

- Keyboard brightness, logo Off/Static/Breathing, and battery-care toggle.
- Balanced, Silent, Performance, and Custom modes; CPU Low/Medium/High/Boost and GPU Low/Medium/High.
- Automatic cooling and experimental manual fan targets limited to 4000–4500 RPM in Balanced mode.
- Saved AC and battery profiles, including automatic switching while hidden in the tray.
- Tray menu and global shortcuts: Ctrl+Shift+Alt+F12 shows the app, F5 cycles modes,
  F6 applies the AC profile, and F7 applies the battery profile.

**Save current as AC/battery** captures a fresh hardware snapshot. Battery profiles
allow Balanced or Silent. Automatic switching requires both profiles and is off by
default. Close hides to the tray; **Exit** quits. Global commands reopen the window
to show their result. Profiles live in `%LOCALAPPDATA%\RHelper\blade18.json` and
survive application restarts. Invalid configuration is preserved and reported.

Setters are model-scoped and read back; failed changes attempt rollback. An exclusive
device lock coordinates R-Helper operations. No initialization or undervolt commands
are sent. Hyperboost and unverified fan ranges are excluded. Manual targets are not
guaranteed physical RPM: fans ramp slowly and the two zones can report different speeds.

Hardware batch testing verified 23 command/readback/restoration cases, plus longer fan
response, GUI, hotkey, profile, application-restart, and real AC/battery transition checks.
These checks do not establish thermal stability under load, sleep/reboot persistence,
or that charging physically stops at 80%. Razer background services remained active;
brightness drift was observed after one battery transition, so coexistence is not certified.

To repeat the temporary validation with a recovery record written before the first change:

```powershell
cargo run -p librazer --bin rhelper-diagnostics -- --validate-brightness --output brightness-test.json
# Full temporary test batch with a recovery record and restoration after each case:
cargo run -p librazer --bin rhelper-diagnostics -- --validate-batch --output batch-test.json
# Longer physical fan-response observation, followed by restoration:
cargo run -p librazer --bin rhelper-diagnostics -- --validate-fans --output fan-test.json
# Emergency restoration uses the separate, immutable recovery snapshot:
cargo run -p librazer --bin rhelper-diagnostics -- --restore-state batch-test.recovery.json
```

The console utility exports JSON without serial numbers, usernames, or HID device paths:

```powershell
cargo run -p librazer --bin rhelper-diagnostics -- --output inventory.json
cargo run -p librazer --bin rhelper-diagnostics -- --probe --output status.json
```

`--probe` selects the interface signature verified on the development laptop.
`--probe-interface INDEX` is available for explicit interface investigation; indices are
local to each inventory and can change. Output files must not already exist.
An individual failed getter is reported in JSON and does not imply a zero/default value.
Successful getter responses establish read support only, not safe setting ranges for other controls.

See [Blade 18 validation](docs/blade18-validation.md) for observed results and next steps.
See the [Blade 18 research and milestones](docs/blade18-roadmap.md) for independent
RGB, refresh-rate profiles, three-fan curves, tuning and BIOS/driver plans.

Additional explicit diagnostics (new output files required):

```powershell
cargo run -p librazer --bin rhelper-diagnostics -- --probe-lighting --output lighting.json
cargo run -p librazer --bin rhelper-diagnostics -- --probe-sensors --output sensors.json
cargo run -p librazer --bin rhelper-diagnostics -- --validate-fan-zones --output fan-zones.json
```

The sensor view queries on demand, includes the third fan's candidate readings and
controller firmware, and preserves unsupported temperature queries as errors. The
normal idle power watcher uses Windows notifications; it does not poll the controller.

A Windows application to control Razer Blade settings w/o Synapse.

<img width="332" height="388" alt="image" src="https://github.com/user-attachments/assets/3a4630d8-d79a-4e6b-b6a6-df4f1f52bdb9" />

## Upstream features for other models

- Performance modes: Battery, Silent, Balanced, Performance, Hyperboost, Custom
- Custom mode: CPU/GPU Low/Medium/High/Boost adjustments with experimental Undervolt option (no idea what it does as it's a preset)
- Fan control: Auto/Manual, with current RPM display
- Keyboard backlight brightness control
- Logo lighting: Static, Breathing, Off
- Battery care: Toggle charging threshold (80%)


## Installation

1. Download the latest release
2. Run `rhelper.exe`

## Building

Windows prerequisites: Rust via rustup, Visual Studio 2022 Build Tools with
**Desktop development with C++** and a Windows SDK. `rust-toolchain.toml` pins the
tested Rust version and installs rustfmt/clippy. Restart your terminal after installation.

```powershell
cargo build --release
```

Build both executables and run the workspace tests:

```powershell
cargo test --workspace --locked
cargo build --workspace --locked
# Optimized distribution build:
cargo build --workspace --release --locked
```

Executables are in `target/debug` or `target/release`. The Windows GitHub Actions
workflow tests the workspace and uploads optimized binaries as an artifact.

## Architecture

### Blade 18 interface preview

The Blade 18 window uses compact performance buttons, grouped fan and lighting
controls, AC/battery profiles, and expandable diagnostics and update links.
The experimental [three-fan curve editor](docs/curve-sessions.md) supports opt-in
supervised AC sessions. RGB zones and tuning features remain in development.
Sensors are snapshots outside an active curve session; active sessions show live readings.

Run `target/debug/rhelper.exe --preview-blade18` to inspect the interface with
sample data and disabled hardware controls. Preview mode skips device detection,
settings loading/saving, power notifications, automatic profile switching, tray
hooks and exit-time hardware restoration. Normal launch uses the real device.

Core device control via locally vendored `librazer` (derived from razer-ctl)

Developer-only [GPU temperature testing](tools/GpuTest.md) is an opt-in binary
behind the `diagnostic-gpu` feature. Normal R-Helper builds do not include its
graphics dependency or execute its workload. The [post-update validation notes](docs/blade18-post-update.md)
record measured sensor/fan behavior and the remaining curve-controller work.

The [supervised curve test](tools/CurveTest.md) exercises three numbered fans and
automatic recovery from injected missing/stale readings or controller termination.
It uses explicit EC proxies and is separate from the app session runtime.

See [controller capability inventory](docs/razer-controller-capabilities.md) for
other source-backed controls and their current verification status.

[Lighting controls and evidence](docs/blade18-lighting.md) cover the tested
startup-animation and independent underside switches/brightness, static keyboard RGB,
single-color logo, and remaining Chroma color-control work.

[Internal-panel refresh profiles](docs/blade18-display.md) support driver-tested
same-resolution rates with separate AC/battery choices. Firmware keyboard
Breathing is available; advertised Spectrum remains disabled after failed readback.


## License

MIT. Includes MIT-licensed portions derived from razer-ctl (see NOTICE and THIRD_PARTY_LICENSES.md).

## Support

If you really want to express gratitude: [PayPal Donation](https://www.paypal.com/paypalme/fatalutionDE)
