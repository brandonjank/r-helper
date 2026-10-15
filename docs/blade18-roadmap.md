# Blade 18 (2025) research and milestones

Research date: 2026-10-14. Target: RZ09-0529 / RZ09-05299ER9,
Intel Core Ultra 9 275HX, RTX 5090 Laptop GPU, BIOS 2.01.
This is a capability roadmap, not a promise that another laptop's controls work here.

Latest milestone: independent native keyboard/underside static RGB is physically
confirmed. Static per-key keyboard and 19-cell underside maps now save in profiles;
cells 1/10/18 were checked, intermediate positions are accepted by assumption,
and slot 19 stayed dark. New on-demand panel/firmware,
driver, GPU and thermal-zone diagnostics, guarded Windows resolution previews,
and an administrator-only timed GPU clock preview are implemented.
See [the autonomous batch, evidence and remaining blockers](blade18-autonomous-batch.md)
and [native lighting details](blade18-lighting.md). Older observations below are
retained as investigation history, not current feature status.

The next candidate adds the [fan/sensor, updates and battery validation batch](blade18-validation-batch.md):
CPU/GPU load-response evidence, renewed unequal-fan readback tests, an on-demand
cached official firmware catalog panel, and finite process/battery measurement
tools. Physical fan mapping, CPU die reference, curve sleep/unplug behavior and
real battery-energy overhead remain separate validation requirements.

## Findings on this machine

| Area | Evidence | Remaining work |
| --- | --- | --- |
| Main fans | Post-update 90-second cases verified actual speeds independently at 4000/4500 and 4500/4000 RPM after delayed target reassertion. | Repeat physical mapping and validate temperature response before enabling curves. |
| Third fan | With corrected initialization, targets 4000/4000/3500 produced actual speeds 4000/3900/3400 RPM. | Physical location and sensor policy remain unverified. Writes stay confined to the diagnostic lab. |
| Controller firmware | Getter `0081`, arguments `00 00`, now returns bytes `01 07` after updates. | Keep controller firmware separate from system BIOS version. |
| Controller temperature | Getter `0d85`, arguments `00 01 00 00`, returned `[2, 69, 45, 0]` after a narrowly checked parser correction for this model's header variant. | Candidate EC sensors A/B; validate units, mapping, update rate and load response. Do not assume either is a die sensor. |
| RGB | Standard RGB getters for LED IDs 4/5 and the backlight effect getter return NotSupported on the verified endpoint. Existing brightness and logo mode getters work. | Capture keyboard, logo and underside lighting separately in Synapse. |
| Device interfaces | Present-device inventory and HID enumeration show `1532:02c7`, interfaces 0–3; Razer virtual/control device nodes also exist. No present `1534` devices found. | Capture which interface/driver path handles each feature. A vendor ID alone does not establish compatibility. |
| Other temperature providers | WMI exposes a BatteryTemperature class but returned no instances. ACPI thermal-zone query returned access denied in the current account. No Razer-named WMI class appeared in root/wmi. The Windows battery-class temperature query also reports unsupported. | Obtain validated CPU, GPU, battery and board readings from appropriate providers; ACPI zones are not automatically die temperatures. |

Local raw evidence is in the ignored `diagnostic-reports/research/` directory.
`independent-fan-zones.json` includes a recovery companion and restoration result.
`sensor-header-variant.json` records the successful sensor query; earlier failed probes
and `lighting-getters.json` preserve the investigation history.
The fan test restored the user's captured **Performance / Auto** setting, brightness
176, static logo and disabled battery-care mode; it did not replace it with a default profile.

## Low overhead first

The Blade 18 UI previously requested a redraw every 200 ms, even without changes.
The watcher previously woke every 100 ms to check whether its two-second polling
interval had expired. Both loops are removed for the normal Windows path.

The subsequent tray-hidden check exposed a separate framework busy loop: 29.3125
CPU seconds in a 30.016-second sample. The pinned framework now carries a small
backport of [upstream egui #8398](https://github.com/emilk/egui/pull/8398), which
removes redraw polling and explicitly sleeps when no timer remains. The patched
crate and licenses are in `vendor/eframe`; its provenance is recorded in PATCHES.md.
The corrected tray-hidden sample consumed **0.015625 CPU seconds in 30.012 seconds**,
about **0.052% of one core**, versus roughly 97.7% of a core before the framework fix.
This is a process CPU measurement, not a battery-energy measurement.

Power changes now use `PowerSettingRegisterNotification` and a blocking event queue.
The initial notification establishes a baseline without applying a profile. Duplicate
events are ignored, queued source changes are coalesced, and shutdown wakes the queue
immediately. If registration fails, an explicit warning accompanies a 30-second polling
fallback. Device jobs and tray/hotkey handlers request repaint on completion/input;
only active UI animations should need repeated rendering. Sensor investigation is
explicit and on demand. No continuous NVAPI/NVML probing or per-key lighting streaming
is introduced.

One 30-second idle process sample before the change consumed 0.34375 CPU seconds;
the corresponding sample after consumed 0.0625 CPU seconds (about 82% less in these
samples). Working sets were approximately 133 and 116.5 MiB. These are short AC-powered
process measurements with other applications running, not a controlled battery-energy
benchmark or proof of a particular battery-life improvement. Native notification
registration and shutdown tests pass; physical AC/battery switching must be rechecked
with the new event backend.

Acceptance target: no periodic GUI redraws or power polling in the normal idle path,
no GPU wake caused by background telemetry, and a reproducible hidden/visible idle
benchmark. Compare on-battery discharge rates with/without R-Helper at matched panel
brightness, refresh rate, lighting, workload and GPU state. Repeat long enough to
separate app overhead from noise. Keep any sensor loop demand-based unless it is
needed for an explicitly enabled software fan controller.

Repeat process sampling using `./tools/measure-idle.ps1 -Seconds 30 -Label 'hidden'`
after hiding the app, and again while visible. Optional `-OutputPath` creates a new
JSON file without overwriting an existing result.

## Fan and temperature design

The owner describes two primary fans sharing the vapor chamber and a smaller fan
above the battery/central components. Treat that as a physical-layout reference;
HID selector numbers still require verification.

- Separate CPU and GPU curve editors, each with its own validated temperature input,
  minimum/maximum RPM, interpolation, hysteresis and response time.
- Account for heat sharing: allow a configurable cross-sensor policy rather than
  assuming the CPU fan can ignore heavy GPU load, or the reverse.
- Give the third fan its own battery/board policy once the relevant sensors and
  actuator are established. A zero tachometer can be a stopped fan; it does not prove
  that a control path exists or that stopping it under load is acceptable.
- Prefer firmware-stored curves if the Blade 18 supports them. If software control
  is necessary, it must work while hidden and fall back to firmware automatic control
  on missing/stale sensors, controller errors, resume or shutdown.
- Never label a slow EC board sensor as CPU package temperature without correlation.
  Verify the CPU source against trusted telemetry and the GPU source against NVIDIA
  telemetry during separate CPU/GPU loads. Measure latency as well as numeric agreement.

The current public manual-target envelope remains experimental: 4000–4500 RPM in
100 RPM increments. This is a tested command envelope, not physical fan limits.
The main GUI still uses one shared slider; the diagnostic API can test unequal targets.

The offline `librazer::fan_curve` planner now supports three independent inputs,
monotonic curve validation, linear interpolation and upward 100-RPM quantization.
It returns a shared automatic-mode plan for absent/stale/invalid readings, thermal
cutoffs or unverified capabilities. The planner itself has no hardware access.
A separate supervised lab controller and opt-in app sessions now exercise it using
neutral fan numbers and explicit EC proxies. The app editor supports source and
point editing, live readings while active, Stop/Auto and a session limit; it never
starts curves at login or on battery.
Its pure tests use synthetic envelopes, including for
the third curve; they do not establish physical fan limits. Per-zone downward
damping now holds decreases and steps them down by 100 RPM, with immediate
increases and undelayed safety fallback plans. Scheduling, cross-sensor policy,
full physical sensor/fan validation and real model limits remain future work.
The app watchdog and controller monitor each other's liveness and recover fan
settings on exit/failure. Real sleep/resume and physical power transitions with
this backend still need validation. See [curve sessions](curve-sessions.md).

See [post-update validation](blade18-post-update.md) before resuming live tests.

A one-time NVIDIA query taken immediately after the EC sample also reported 45 C,
matching EC sensor B. This supports a GPU association at that instant but does not
establish dynamic accuracy, latency or a CPU mapping for sensor A. That diagnostic
query reported GPU draw of 12.68 W; it is not an R-Helper power-consumption measurement.

## Milestones

### M1 — Efficient diagnostics and capability discovery

Deliver the event-driven idle path, three-zone read-only fan investigation,
controller firmware/temperature candidates and passive enumeration of the requested
vendor IDs. Preserve query errors and capability status in exported diagnostics.
Add a repeatable idle benchmark and physical event-backend transition check.
Completion requires validated results, not just packets that receive success status.

### M2 — Independent keyboard, underside and logo lighting

Build a per-zone capability model for color, brightness, effect and power state.
Capture Synapse traffic while changing exactly one setting on one zone; compare
before/after packets and repeat each change. Determine keyboard matrix geometry,
secondary legend LEDs, underside zone selectors and whether the lid supports full
RGB independently. Validate restoration, normal Fn/media keys, idle timeout, sleep
and battery behavior. Use firmware effects where possible; streaming effects remain
opt-in because of wakeups and power use.

### M3 — Display and refresh-rate profiles

Enumerate the actual internal-panel modes using Windows display APIs. Offer only
reported valid modes and save separate AC/battery selections. Add a timed revert
after manual changes. Handle dock/undock, external displays and resume. Treat a
firmware dual-resolution panel-mode switch separately from an ordinary refresh-rate
change, especially if it requires reboot. Do not hard-code 60/240/440 Hz availability.

### M4 — Three-fan telemetry and independent curves

Map physical fans to controller selectors, validate real CPU/GPU/battery/board
temperature sources, capture any firmware curve interface and establish per-fan
bounds. Deliver the separate curves and failure behavior described above, a clear
automatic-cooling restore action, logging, and AC/battery/load/sleep tests.
Third-fan target writes and supervised curve/fallback tests now work in the lab.
Opt-in GUI sessions include the third selector using an explicitly chosen EC
proxy; it is not labeled as a battery-temperature curve. Unattended startup and
on-battery curves remain disabled pending validation.

### M5 — NVIDIA tuning and CPU undervolt investigation

Add capability-aware NVIDIA telemetry and stock-setting capture first. Investigate
clock offsets, clock caps, power limits and V/F curve control separately; a supported
getter does not guarantee a working setter on this laptop/driver. Driver version
observed here: NVIDIA 32.0.16.1074; Intel graphics 32.0.101.6733. Avoid waking the dGPU
for a hidden status display. Implement reset/recovery and validation before adding
experimental tuning controls. Investigate CPU tuning through the supported platform
interface rather than equating the upstream enum named Undervolt with a voltage offset.

There is **no validated nonzero undervolt range for this unit yet**. CPU baseline is
0 mV offset; GPU baseline is the factory V/F curve and stock clock offsets. Chip
variation, BIOS locks and driver behavior prevent inferring a safe range from another
275HX or 5090. Intel documents that undervolt protection can prevent negative offsets;
OEM firmware decides which controls are exposed. Leave security/BIOS settings alone
during discovery. G-Helper's AMD CPU undervolt control is not an implementation for
this Intel CPU. Its GPU clock-offset/clock-cap approach is useful design inspiration,
but its suggested offsets are not a validated Blade 18 range.

For later owner-run experiments: capture stock readings and performance, expose the
actual supported units/bounds, change one parameter in small steps, and retest idle,
single-core, multicore, GPU, mixed loads and sleep/resume. Record WHEA errors, driver
resets, artifacts and performance regressions. Keep boot-time application disabled
until the owner's chosen setting has passed those tests; provide one-click stock reset.

### M6 — BIOS, firmware and driver information

Show installed BIOS, controller firmware and GPU/chipset driver versions with exact
model matching, source links and dates. Use Razer's RZ09-0529 downloads page and
official Intel/NVIDIA sources. Start with manual checks and cached results; avoid a
constant updater. Distinguish a newer available version from an applicable update.
Keep firmware flashing an explicit vendor-supported workflow with its own power and
reboot requirements; do not silently flash or install drivers.

## UI direction

Use G-Helper's compact grouping as inspiration: performance presets at the top,
then display, lighting and battery controls; separate windows/expanders for
Fans + Power, NVIDIA tuning and Updates. Show supported controls based on a device
capability report, and explain unavailable controls where the reason is useful.
Keep independent RGB zones visibly distinct, and put CPU/GPU/third-fan curves beside
their sensor source, actual RPM and target RPM. Hardware snapshots need a timestamp;
the UI must not imply that an on-demand reading is live telemetry.

## Protocol investigation workflow

The Blade 18's `0d85` reply preserves transaction/command identity and successful
status but has wire bytes `00 03` in the nominal remaining-packets field. The library
previously required that field to echo the request, rejecting otherwise plausible
sensor data. The exception now requires this exact command, field value, four-byte
payload and a two-sensor count. Adjacent variants remain rejected; unit tests cover
the boundary. This establishes a readable payload, not verified sensor semantics.

1. Inventory interfaces and capture a stock snapshot.
2. Compare community protocol facts with this model. Use bounded read-only probes
   with exact allowlists; never sweep arbitrary setter commands or EC addresses.
3. Capture USB/control traffic from one deliberate Synapse change at a time. Review
   capture-driver requirements before installation, and preserve private identifiers
   locally. Test lighting capture first, not voltage or firmware writes.
4. Record command class/id, transaction ID, report length, selector/storage bytes,
   response status and readback. Normal device mode and driver mode may behave differently.
5. Promote a feature only after independent readback, physical observation and
   restoration. Add packet tests that reject adjacent unreviewed values.

The existing `LightsAlwaysOn` name deserves a protocol audit: OpenRazer identifies
`0004` / `0084` as device mode. Changing it may affect Fn/media handling. It remains
blocked for Blade 18 writes and must not be reused as a vapor-chamber sleep-light toggle.

## References

- [LoLei/razer-cli](https://github.com/LoLei/razer-cli): Linux frontend using OpenRazer;
  useful zone/effect UX, not a standalone Windows HID implementation.
- [OpenRazer LED packet definitions](https://github.com/openrazer/openrazer/blob/477f4d6d3ab7e9deb6310d5bd3742560c5cbe80d/driver/razerchromacommon.c)
  and [keyboard model handling](https://github.com/openrazer/openrazer/blob/477f4d6d3ab7e9deb6310d5bd3742560c5cbe80d/driver/razerkbd_driver.c).
- [G-Helper features](https://github.com/seerge/g-helper) and
  [GPU tuning explanation](https://github.com/seerge/g-helper/wiki/FAQ).
- [Razer Blade 18 Synapse guide](https://dl.razerzone.com/master-guides/RazerSynapse4/Blade18-00000711-en.pdf):
  pages 19–24 describe fan, display and lighting controls; it does not document HID packets.
- [Razer sensor implementation](https://github.com/kgswx/R-helper/blob/3f666cee010ab7e1f10cf54995dc65aefcb3583c/app/Razer/RazerEC.cs)
  and [Blade Control command notes](https://github.com/jdsgnrinfo/blade-control/blob/main/src/BladeControl.Core/Hardware/RazerCommands.cs).
  Their temperature observations concern other models.
- [razer-ctl protocol cautions](https://github.com/sqmagellan/razer-ctl/blob/main/librazer/src/command.rs).
- [Intel undervolt protection](https://www.intel.com/content/www/us/en/support/articles/000094219/processors.html).
- [NVIDIA NVAPI](https://developer.nvidia.com/nvapi/get-started).
- [Windows power notification API](https://learn.microsoft.com/en-us/windows/win32/api/powersetting/nf-powersetting-powersettingregisternotification).
- [RZ09-0529 drivers and downloads](https://mysupport.razer.com/app/answers/detail/a_id/19866).

Protocol facts are documented here; no GPL source from these reference projects was
copied into this MIT repository.
