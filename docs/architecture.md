# Architecture and supported models

R-Helper aims to support more Razer laptops over time. Current support is the
upstream model catalog plus the Blade 18 (2025), RZ09-0529. Being a Razer device,
or sharing a USB vendor ID, does not establish that a control is compatible.

## Boundaries

- `src/main.rs`: application metadata and startup routing only.
- `src/cli.rs`: ordered command dispatch before GUI startup, ownership acquisition
  or background monitoring. A matching command ends dispatch even on failure.
- `src/legacy_app.rs` and `src/ui/`: the existing upstream model interface.
  `librazer/src/descriptor.rs` remains its source of model capabilities,
  initialization commands and permitted performance combinations.
- `src/blade_app/`: the Blade 18 interface. Background operations declare whether
  they replace the manual restore baseline; displayed text has no role in that
  decision.
- `src/blade_vendor/ui.rs`: ownership UI state. The parent module owns the
  service/process backend and recovery. Moving UI code does not widen takeover.
- `librazer/src/blade18/`: model-specific protocol, sensor and lighting behavior.
  Existing device checks, packet allowlists, recovery records and operation locks
  remain authoritative.

Do not add the Blade 18 to the legacy catalog as a shortcut: its audited access
path intentionally avoids upstream initialization and unrestricted commands.
Future models should receive explicit identification and capability evidence.
Do not inherit fan counts, sensor meaning, lamp maps, firmware catalogs or tuning
ranges merely from screen size or model year. Unsupported controls should remain
unavailable rather than use a similar model's packets.

## Change and validation standards

Keep formatting-only changes separate from behavioral changes. Format with
`cargo fmt --all`; CI checks formatting, compiles all targets with Clippy, runs
workspace tests, checks diagnostic analysis and PowerShell syntax, and builds
release binaries. Existing Clippy warnings remain visible; they are not suppressed
or treated as a clean warning baseline.

Prefer typed operation intent, bounded workers and explicit recovery over
interpreting UI labels or probing unknown setters. Add offline regression tests
for dispatch, validation and recovery behavior. Hardware observations remain
model-specific evidence and cannot be replaced by a passing build.

## Panels, profile history and distribution

The Blade 18 interface now routes cards to seven compact panels. Hidden card
bodies are not executed; switching panels performs no hardware query. Fixture
tests render every panel without background workers or hardware access. Upstream
models retain their existing interface.

`profile_activity` carries the last successful application, trigger and controlled
areas. Manual jobs, power transitions and foreground-application changes pass
explicit metadata. Reads, saved selections and failed jobs do not become successful
applications. This is session history: it cannot prove that external software has
left the hardware unchanged. It adds no monitoring timer or hardware reads.

`tools/Package-Portable.ps1` bundles the release binaries, required thermal and
controls helpers, licenses, user notes, source manifest and SHA-256 checksums.
CI verifies ZIP contents and hashes, overwrite refusal and missing-input failure
before uploading the portable archive. Developer workload binaries are excluded.

The deferred guided validation in `blade18-validation-batch.md` remains required
before merging the hardware-validation candidate. Offline rendering and package
checks do not establish live fan, sleep/resume or battery-energy behavior.
