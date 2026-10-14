> [!WARNING]  
> Note from Fatalution: I returned my Blade 16.    

# R-Helper

This fork is maintained at [brandonjank/r-helper](https://github.com/brandonjank/r-helper),
based on [Fatalution/r-helper](https://github.com/Fatalution/r-helper).

## Blade 18 (2025) development

The `RZ09-0529` / USB `1532:02c7` is recognized in a dedicated diagnostics screen.
On this model, launching `rhelper.exe` opens diagnostics instead of the upstream control UI.
Inventory is passive; **Read device status** explicitly sends audited getter requests.
Setting commands are blocked on diagnostic device handles. A separate, narrowly scoped
brightness handle permits only the keyboard brightness setter plus audited getters.
No initialization commands are sent. Performance, fans, battery care, and logo setters
remain pending validation.

Keyboard brightness passed a live 176 → 168 → 176 round trip with matching readbacks.
Use **Read brightness**, adjust the slider, then **Apply brightness**. The app verifies
the result and attempts to restore the previous value if the update fails.

To repeat the temporary validation with a recovery record written before the first change:

```powershell
cargo run -p librazer --bin rhelper-diagnostics -- --validate-brightness --output brightness-test.json
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

A Windows application to control Razer Blade settings w/o Synapse.

<img width="332" height="388" alt="image" src="https://github.com/user-attachments/assets/3a4630d8-d79a-4e6b-b6a6-df4f1f52bdb9" />

## Features

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

Core device control via locally vendored `librazer` (derived from razer-ctl)


## License

MIT. Includes MIT-licensed portions derived from razer-ctl (see NOTICE and THIRD_PARTY_LICENSES.md).

## Support

If you really want to express gratitude: [PayPal Donation](https://www.paypal.com/paypalme/fatalutionDE)
