# Blade 18 (2025) validation

Development machine: Razer Blade 18, SKU `RZ09-05299ER9`, BIOS `2.01`
(BIOS date `08/04/2025`), RTX 5090 configuration reported by its owner.
Validation performed 2026-10-14. GPU identity has not yet been independently captured.

## Identity and transport

- Locally observed USB vendor/product: `1532:02c7`.
- [OpenRazer issue #2488](https://github.com/openrazer/openrazer/issues/2488)
  corroborates this model/PID and publishes interface descriptors.
- Getter transport works on interface 2, usage page 1, usage 2. Its descriptor
  includes the Razer 90-byte feature report. No initialization sequence was needed.
- Interface 3, usage page 89, usage 1 rejected upstream report framing with
  Windows error 87 (incorrect parameter). It is not the preferred control endpoint.
- Diagnostic opening skips upstream's `[0, 0]` feature-report test.

## Observations

All seven getter categories succeeded with matching packet responses on interface 2:

| Reading | Observed value |
| --- | --- |
| Performance / fan mode | Balanced / Auto |
| Fan 1 actual speed | 2300 RPM |
| Fan 2 actual speed | 2300 RPM |
| Keyboard brightness | 176 / 255 |
| Battery care | Disabled |
| Logo | Static |
| Lights always on | Enabled |

These are snapshots, not defaults. They do not validate setters, boost levels,
fan limits, or charge-limit semantics. Raw reports are kept in the ignored
`diagnostic-reports/` directory, not committed to GitHub.

## Keyboard brightness round trip

Live test on the development machine: original 176/255, temporary request 168/255,
readback 168/255, restoration request 176/255, restoration readback 176/255.
The test passed and left the original brightness in place.

The CLI writes and synchronizes the original value to its output file before
attempting the temporary change. Restoration is attempted even if the initial
setter fails. A failed or mismatched restoration is reported as a failed test.
The GUI now exposes keyboard brightness only, with explicit Apply and readback.
Other setting commands remain blocked on the brightness handle.

## Build validation

Rust 1.99.0 MSVC, Visual Studio 2022 C++ Build Tools, and Windows SDK installed.
The initial workspace tests and diagnostic executable build pass; the initial
GitHub PR build also passed, including optimized binaries. Optimized builds on
this machine hit Windows Application Control error 4551 when running the generated
`winapi` build script. No security-policy change was made. Use the development
build locally while release builds are checked in GitHub Actions. During brightness
development all seven library tests passed; a zero-test diagnostic binary test
was separately blocked by local Application Control error 4551. The updated GUI
compiled successfully but Application Control also blocked launching it. The prior
diagnostics window was closed for the rebuild; the updated GUI is not running.

## Next validation steps

1. Compare getter snapshots with Synapse's displayed values, including both fan zones.
2. Keyboard brightness round trip passed; visually confirm the new slider, then validate logo setters.
3. Validate battery care behavior and persistence across sleep and restart.
4. Record supported performance modes and CPU/GPU combinations for this Intel model.
5. Validate fan control limits and rollback before exposing manual controls.

Do not copy the AMD Blade 16's boost combinations or fan limits into the Blade 18
descriptor without evidence. Keep the Blade 18 out of the upstream GUI control
table until the relevant controls have been validated.
