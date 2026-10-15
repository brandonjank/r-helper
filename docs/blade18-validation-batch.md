# Fan/sensor validation, update information and battery overhead

Date: 2026-10-14. Target RZ09-0529, Core Ultra 9 275HX / RTX 5090.
Raw evidence stays under ignored `diagnostic-reports/research/`.

## Implemented

- On-demand **BIOS, firmware + drivers** panel. Read installed firmware through
  the isolated signed provider and drivers from Windows. Compare against a dated
  model-specific catalog; explicitly check the official catalog when requested.
  Cache only a complete model-matched table. Failed/blocked/changed pages retain
  the reference/cache and show the error, not a successful check.
- BIOS/EC/PD/MCU and RTX 5090 VBIOS entries show available version, release date,
  comparison and the official model downloads page. MCU stays unknown rather
  than equating the separate 1.7 control-firmware getter with MCU firmware.
  VBIOS selection requires an installed RTX 5090 driver entry; zero-padding and
  hexadecimal case differences do not falsely report an update.
- Intel/NVIDIA links and installed driver versions are available. Automatic
  latest-driver comparison is not implemented; official sites require selecting
  the OS/GPU and reviewing applicability. No updater downloads, flashing or
  background network checks are introduced.
- CPU response tests now stop on AC loss or either missing EC temperature.
  Unequal fan-target validation stops on AC loss, changed mode/targets/performance,
  or missing/critical temperatures.
- `Test-FanValidation.ps1` adds an independent parent-exit/deadline restorer around
  the existing primary-zone validation, and verifies the original state afterward.
  It never treats successful target registers as physical fan mapping.
- Finite `--idle-benchmark hidden|visible SECONDS OUTPUT` measurements use the
  real GUI, tray and watchers, warm up for 15 seconds, measure process CPU/I/O,
  then exit normally. Vendor takeover is explicitly skipped; settings are not
  rewritten. No GPU or update queries are performed by the benchmark.
- Battery harness verifies absolute capacity units via the battery-class query,
  samples remaining energy every 15 seconds, rejects AC/state changes, sleep/gaps
  and low/invalid capacity, and compares display modes, brightness, power scheme
  and settings hash. It temporarily holds the display/system awake, then restores
  the thread's execution request; it does not edit Windows power settings. One ABBA
  batch compares Closed / Hidden / Hidden / Closed. AC dry runs never report
  battery-energy results. Other workload, GPU residency and lighting must still
  be controlled by the observer; one batch is an estimate, not a causal proof.

## Unattended evidence

### Sensor response

The 10-second baseline / 25-second two-thread CPU load / 15-second recovery run
kept firmware cooling automatic. EC A median rose from 61 to 64 C while EC B
remained 42 C. Background compilation overlapped part of recovery; its peak is
not interpreted as workload-only sensor response. This supports CPU association,
not CPU die accuracy. A separate repeat without compilation/GPU work overlapping
(`sensor-cpu-clean-20261014-204111`) recorded EC A medians 55 C baseline, 65 C under
load and 59 C recovery; EC B medians were 37, 39 and 40 C. Shared heating/lag is
visible, and physical fan roles are not inferred from it. No independent CPU die
provider has been validated here.

The bounded DX12 GPU test used 64 dispatches per batch for 20 seconds, with
baseline and recovery. NVIDIA utilization reached 28% and temperature ranged
43–49 C. EC B median moved from 42 C at baseline to 47 C under GPU load, with
0.76 C mean absolute difference / 3 C maximum difference against NVIDIA across
the run. Correlation was about 0.924; maximum sequential-provider query window
was 69 ms. EC A did not track that GPU reference (about 18.14 C mean difference).
This supports retaining EC B as a GPU-associated proxy, not assuming instantaneous
die accuracy under every workload. All three fan modes stayed automatic; the
third tachometer was zero under this light workload. Zero does not identify it
as a stopped battery fan or rule out a working actuator.

Reports: `sensor-validation-20261014-202626/`, with corrected analysis phase name
`gpu_load`. The first analyzer output used `load` and omitted the load-phase
summary; corrected analysis and regression tests retain the real GPU samples.

### Idle overhead and battery instrumentation

| Sample | Measured CPU seconds / 30 seconds | One-core average | Process I/O |
| --- | --- | --- | --- |
| Hidden | 0 at Windows counter resolution | 0 measured | 0 operations / bytes |
| Visible | 0.015625 | 0.052% | 0 operations / bytes |

Both AC dry runs ended normally and matched before/after display, brightness and
power scheme. Original Razer services remained running. This is the GUI without
vendor takeover; it does not measure the takeover guard or prove zero GPU wakeups,
battery drain, or a battery-life improvement. Remaining battery energy was
constant on AC; wattage output correctly stayed null/invalid.

The final 60-second hidden sample recorded 0.03125 CPU seconds (0.052% of one
core), zero process I/O and matching controls. A closed-app AC dry run also
passed. A real-energy invocation on AC was rejected before launching the GUI.
These checks exercise the instrumentation; no unplugged energy result exists yet.

Reports: `overhead-hidden-20261014-203100/` and
`overhead-visible-20261014-203200/`.

### Unequal fan-target validation

Both 90-second primary-zone directions (4000/4500 and 4500/4000 RPM) passed
three-consecutive-sample actual-speed checks within 200 RPM. Original performance,
automatic cooling and all captured targets restored. The separate recovery guard
acknowledged parent restoration; vendor service startup/running states were unchanged.
This reconfirms independent target response, not physical vent/fan assignment.
The new guardian's owner-exit fallback also restored an unchanged captured state
successfully, using a harmless child as the owner. That checks this recovery path
without claiming a new active-manual-fan crash test.
Earlier attempts are preserved: one preflight stopped at EC A = 80 C; two guard
startups failed because an absolute path was joined twice. No manual test started
on those attempts. Absolute-path handling is corrected for all affected harnesses.

### Update information

Installed readings: BIOS 2.01, EC 1.06, RTX 5090 VBIOS 98.3.5c.40.25; MCU and PD
version unavailable. The official model catalog reference lists BIOS 4.01,
EC 1.06, MCU 1.07, RTX 5090 VBIOS 98.03.5C.40.25 and PD 2v00.14.
The real HTTP fetch encountered Razer's unavailable/pre-page response, so no
successful live-check timestamp or new cache was claimed. The panel retains the
dated reference and official browser link. No firmware update was attempted.

Sources: [Razer model catalog](https://mysupport.razer.com/app/answers/detail/a_id/19866),
[PD release information](https://mysupport.razer.com/app/answers/detail/a_id/20413/kw/settings).

## One guided session still required

### Current UI build follow-up (2026-10-14)

PR #5 CI passed both Windows jobs. The live interface was inspected at its default
window size: Overview, Fans, Lighting, Profiles, Display, Updates and GPU/diagnostics
were navigable, and sensor/installed-firmware reads completed. One unsupported
arrow glyph in update comparisons was replaced with a plain separator. No display,
lighting, fan or performance setting was changed during this inspection. The app
was exited with its Exit button; original settings matched the captured snapshot.

New AC idle dry runs are **busier than the earlier baseline**, not confirmation
of the earlier zero-I/O result:

| Sample | CPU seconds / duration | One-core average | Process I/O operations / bytes | HID writes / reads |
| --- | --- | --- | --- | --- |
| Hidden initial | 0.359375 / 30 s | 1.198% | 16,166 / 2,332,860 | Not instrumented |
| Visible initial | 0.1875 / 30 s | 0.625% | 10,138 / 1,460,020 | Not instrumented |
| Hidden repeat | 0.46875 / 60 s | 0.781% | 19,454 / 2,810,544 | 0 / 0 |
| Visible repeat | 0.09375 / 30 s | 0.312% | 9,316 / 1,348,712 | 0 / 0 |

Display, brightness, power scheme and settings hashes matched in all four samples.
Passive process-local counters show no repeated HID reports during the repeat
windows. Other process I/O is not classified as file, network or graphics traffic
by this measurement. The CPU variation and unclassified I/O remain open for the
energy comparison; no battery-drain improvement is claimed. Agent activity and
the desktop environment were not fully controlled, so these are diagnostic AC
samples, not a causal comparison. Vendor takeover was skipped as designed.

A closed-app AC dry run also passed with matching display, brightness, power scheme
and settings hash, zero measured energy change and no battery-energy claim.

The measurement script's console summary now correctly converts its ordered map
to an object before selecting fields. The saved result JSON was already complete.
Raw files: `diagnostic-reports/research/current-build-validation/`.

The original snapshot for this session is Performance mode, all fan modes Auto,
targets `[4500, 0, 0]`, keyboard brightness 176, static logo and battery care enabled.
Restore this session's actual snapshot after the guided cooling checks; saved power
profiles are separate and may request different battery-care settings.

Perform physical/load tests on AC first, then the battery comparison last so
GPU load and fan ramps have time to settle. Save work and keep vents clear.

1. Capture the original performance/fan settings. Open R-Helper, select
   Balanced/Auto, then exit the GUI so its automatic power profiles cannot race
   the external curve tests. Approve one elevated `--vendor-validation-hold`
   helper launch; it pauses vendor software using the existing recovery guard,
   has a 45-minute limit, and sends no fan/performance commands itself. Its
   `ready.json` marks takeover. Write `release` to its directory after the fan
   section, confirm `finished.json`, and restore the original performance choice
   after reconnecting AC. This helper's elevated lifecycle remains a guided check.
2. Observe separate fan-zone tests, with left/right viewed from the keyboard.
   Use the existing supervised identification tool and its recovery supervisor;
   announce each observation phase only after telemetry confirms convergence.
   Do not increase unvalidated ranges to force an audible difference. If physical
   separation remains unclear, leave numbered fans and sensor proxies in the UI.
3. Run the release curve fault cases: Stop, Timeout, WorkerExit, WatchdogExit and
   ParentExit. Existing earlier runs passed; repeat after takeover on this build.
   `Test-AppCurves.ps1` now defaults to release and accepts absolute output paths.
4. Start `AcLoss`; after `awaiting-owner.json` appears, unplug and leave unplugged
   until Auto and original targets are verified. The harness requires the exact
   AC-loss error plus independently observed battery power. Reconnect afterward.
5. Start `SleepResume`; after the ready marker, sleep for at least 15 seconds and
   wake/unlock. Verify Auto/original targets and the recorded pause/heartbeat
   fallback. Owner confirmation establishes real sleep; the error alone does not.
6. Exit the GUI and allow temperatures/workload to settle. Unplug once and leave
   the machine idle for the ABBA energy comparison below (about 21 minutes at
   default settings). Keep screen brightness/refresh, lighting, power scheme,
   external displays and applications unchanged. Do not run GPU diagnostics or
   update checks during the measurement. Reconnect AC at completion or if the
   harness stops at low battery. Vendor services stay in the same state for all
   four phases; this isolates added GUI overhead rather than a Synapse-vs-R-Helper
   replacement comparison.

```powershell
./tools/Test-AppCurves.ps1 -Scenario AcLoss -OutputDirectory diagnostic-reports/research/new-ac-loss
./tools/Test-AppCurves.ps1 -Scenario SleepResume -OutputDirectory diagnostic-reports/research/new-sleep-resume
./tools/Test-BatteryOverhead.ps1 -OutputDirectory diagnostic-reports/research/new-battery-abba -SecondsPerPhase 300
```

Do not mark the physical fan layout, CPU die source, battery/board sensor, sleep
test or measured battery overhead verified until those distinct checks finish.
Firmware curve-table setters, wider fan limits and unattended/battery fan curves
remain disabled. BIOS 2.01 remains installed; BIOS work is a separate vendor flow.
