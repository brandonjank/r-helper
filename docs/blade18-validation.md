# Blade 18 (2025) validation

Development machine: Razer Blade 18, SKU `RZ09-05299ER9`, BIOS `2.01`
(BIOS date `08/04/2025`), RTX 5090 configuration reported by its owner.
Validation performed 2026-10-14. WMI and NVIDIA independently identify the GPU as
NVIDIA GeForce RTX 5090 Laptop GPU during the follow-up investigation.

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
compiled successfully but Application Control also initially blocked launching it.

The blocker was subsequently identified in Code Integrity events as Smart App
Control, not an antivirus detection. The owner chose to disable Smart App Control
through Windows Security for local development; the policy state was verified as
Off and the updated GUI launched successfully.

The actual GUI slider and Apply button were tested: 176 → 168 with device readback,
then restoration to 176 with matching readback. The application remains running.

## Extended batch on 2026-10-14

The owner authorized testing all remaining controls as a batch. The CLI synchronizes
a recovery snapshot before writes and restores after every case. All 23 command
readback/restoration cases passed:

| Category | Verified requests |
| --- | --- |
| Logo | Off, Static, Breathing |
| Battery care | Enable, Disable |
| Performance | Balanced, Silent, Performance, Custom |
| CPU/GPU | CPU Low/Medium/High/Boost × GPU Low/Medium/High (12 combinations) |
| Fan targets | 4000 and 4500 RPM, manual flag and both setpoint readbacks |

The initial snapshot was brightness 176, static logo, battery care disabled,
Balanced, automatic fans, CPU Medium/GPU Low, with stored fan setpoints 2000/2000.
All those settings were restored after the batch. Actual RPM fluctuates naturally.

### Physical fan response

A 15-second follow-up showed a slow ramp, so testing was extended to 60 seconds
at a 4000 target and 30 seconds at 4500. At 4000, zone 1 plateaued around 2900
while zone 2 reached 3900. At 4500, final observed speeds were 4100/4300.
Manual targets therefore remain explicitly experimental, bounded to 4000–4500,
and separate from displayed actual speeds. This is not a verified physical
minimum/maximum range or a thermal/load test. Only the two exposed fan zones are
queried; independent control of a third fan has not been established.

### GUI and desktop integration

- Actual GUI battery-care enable and logo Off were verified by an independent
  diagnostic process, survived app exit/relaunch, then were restored by the AC profile.
- AC Balanced and battery Silent profiles were saved and reloaded across app restarts.
- Ctrl+Shift+Alt+F5 changed mode; F6 applied the saved AC profile.
- F7 applied the saved battery profile; F6 then restored the AC profile.
- Hide-to-tray and Ctrl+Shift+Alt+F12 from Notepad reopened the app.
- A hidden-window wake bug was found and fixed by dispatching native tray/hotkey events
  directly rather than relying on hidden-window redraw polling.
- The owner physically unplugged/reconnected AC. Battery Silent and AC Balanced
  were verified by independent getters while the app remained hidden.
- Power-profile application runs in a background watcher independently of rendering;
  startup alone does not apply a profile. Unknown source values do not trigger switching.
- A transient zero battery-care response and mixed-zone modes were observed around
  power transitions. State reads now retry up to three times without inventing defaults.
- One battery transition later showed brightness 180 after the profile had confirmed 176.
  Razer Elevation Service, Game Manager Service 3 and Gaming Services Updater were running.
  This is a coexistence observation; which component changed brightness is unproven.

Snapshots are in the ignored `diagnostic-reports/` directory. Hyperboost, undervolting,
other fan ranges, sleep/reboot persistence, actual 80% charging cutoff, and thermal
stability under load remain unverified. The model-specific UI is intentionally separate
from the legacy GUI, whose assumptions do not apply to this Intel Blade 18.

Final validation: 14 automated tests passed, both optimized executables built locally,
and the optimized GUI launched with the original hardware settings restored. The
recovery command was also run against the captured original state and verified brightness
176, Static logo, battery care disabled, Balanced/Auto, CPU Medium/GPU Low, and stored
fan setpoints 2000/2000. Automatic profile switching is enabled with Balanced AC and
Silent battery profiles. Static analysis completed with warnings in legacy code.
## Follow-up discovery and idle optimization (2026-10-14)

- Unequal main-fan targets 4000/4500 and 4500/4000 RPM read back independently;
  actual RPM responded separately. The 60-second test restored the captured
  Performance/Auto state, brightness 176, static logo, battery care disabled and
  stored targets 2000/2000. Physical CPU/GPU zone mapping remains pending.
- Read-only selector 3 returns a stored target of 3000 RPM; actual samples changed
  from 0 to 1300 and 1800 RPM during follow-up observations.
  These are candidate third-fan readings; no third-zone writes are enabled.
- Controller firmware getter returns 1.6. The temperature getter returns two
  candidate EC fields, sampled at 69/45 C, after accepting an explicitly checked
  response-header variant. NVIDIA independently reported 45 C immediately afterward.
  CPU/GPU die accuracy, update rate and load correlation remain unverified.
- Standard LED 4/5 RGB getters and LED 5 effect getter return NotSupported on the
  verified endpoint. This does not establish that the physical zones lack RGB;
  model-specific Synapse traffic capture is the next investigation step.
- No present VID 1534 devices found. Passive inventory now includes both requested
  VIDs, while control access stays restricted to the verified 1532:02c7 endpoint.
- Removed the 200 ms idle UI repaint loop and replaced the 100 ms watcher wake loop
  with native Windows power notifications. An explicit 30-second fallback is used
  only if registration fails. Short AC-powered 30-second process samples used
  0.34375 CPU seconds before and 0.0625 after; these are not battery-energy tests.
- A subsequent hidden-in-tray measurement exposed an eframe busy loop consuming
  29.3125 CPU seconds in 30.016 seconds. Backporting upstream egui #8398 to the
  vendored 0.32.2 framework reduced this to 0.015625 CPU seconds in 30.012 seconds
  (0.052% of one core). This separate fix is essential for tray-idle efficiency.
- Nineteen automated tests pass, including native notification registration,
  shutdown, packet restrictions and the exact temperature response variant.
  The optimized Windows build and on-demand controller sensor GUI are verified.
  Physical power-transition revalidation with the new event backend is pending.

See [research and milestones](blade18-roadmap.md) for the feature plan and sources.

## Three-fan mapping follow-up (2026-10-14)

These observations qualify the earlier unequal-target result: register readback
does not establish independent physical actuator control. Independent curves
remain unverified and are not ready to enable.

- Mode getters for selectors 1, 2 and 3 all report manual after enabling manual
  mode through selector 1. The mode appears shared.
- Test A (selector 1 target 4500 RPM): the owner felt the left rear vent strengthen.
  Both main tachometers nevertheless ramped together, so this mapping is tentative.
- Test B (selector 2 target 4500 RPM): the owner reported both vents / unclear.
- Repeated Test C (selector 3 target 3000 to 3500 RPM, enable byte 1, 40 seconds):
  the owner reported both rear vents. Main tachometers settled at 2900/2900 RPM,
  and selector 3 at 3000 RPM despite its target reading 3500 RPM. This did not
  isolate the battery fan or establish its physical location.
- Test C restored and verified Balanced/Auto on all three selectors, stored
  targets 2000/2000/3000, brightness 176, static logo and battery care disabled.
  No restoration errors occurred. Raw report:
  `diagnostic-reports/research/map-fan-3-repeat-live.json` (ignored locally).
- A bounded 30-second two-thread CPU workload increased candidate EC field A
  from about 69 to 81 C while B stayed around 46–48 C. This supports CPU-load
  correlation for A, but neither CPU die accuracy nor the complete sensor map
  has been validated against independent simultaneous references.

Third-selector writes are confined to the explicit diagnostic lab path with
captured-state recovery. The normal application does not offer third-fan control.
Next investigation: compare Synapse's model-specific fan behavior and isolate
whether the setter encoding, shared mode or telemetry requests explain the
coupled ramps before implementing independent curve control.


## Post-update actuation correction (2026-10-14)

BIOS remains 2.01 because the owner skipped that updater. Controller getter is
1.7; NVIDIA VBIOS is 98.03.5c.40.25. Separate EC/PD versions remain unconfirmed.

Delayed target reassertion after entering shared Manual mode resolved the observed
3000-RPM default behavior in bounded tests. Two 90-second unequal-target cases
passed actual-RPM convergence checks in both directions; a third-selector 3500
request reached 3400 while the main fans reached 4000/3900. All captured targets
and Auto mode restored. Earlier owner airflow observations must be repeated with
the new isolated baseline; no physical CPU/GPU/battery mapping is claimed.

See [post-update details](blade18-post-update.md) for evidence, the four-logical-ID
capability result, unsupported battery temperature, and remaining curve gates.

## App curve sessions and additional capability reads

The subsequent integration offers explicit AC/Balanced curve sessions with a
three-fan editor and separate watchdog/controller processes. Stop, WorkerExit,
WatchdogExit, ParentExit and Timeout cases passed independent Auto/target recovery.
The first timeout's follow-up read failed transiently; the controller/watchdog
recovery succeeded, the failed report remains local, and the corrected harness's
rerun passed. Native UI validation covered invalid points, saving, launch, live
readings and serialized mode handoff; Balanced/Auto was restored.

39 tests and the optimized workspace build pass. Inactive tray-hidden CPU usage
remains 0.015625 seconds in roughly 30 seconds (0.052% of one core). Physical
sleep/AC transitions and independent CPU die validation remain pending.

Graphics, panel-mode and startup-animation getters are readable; their setters
remain unverified. The class-0 alternate cooling-mode getter returns NotSupported.
See [curve sessions](curve-sessions.md) and [capability inventory](razer-controller-capabilities.md).
