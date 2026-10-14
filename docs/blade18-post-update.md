# Blade 18 firmware transition validation

Prepared 2026-10-14 for RZ09-0529 / RTX 5090. Do not run setters, load tests or
power transitions while an updater is active or required restarts remain pending.

## Published target versions

These are vendor-published versions, not confirmation of installation.

| Component | Version | Relevant published change |
| --- | --- | --- |
| [BIOS](https://mysupport.razer.com/app/answers/detail/a_id/19839) | 4.01 | Sensor support, restart reliability, DC power-saving mode |
| [EC](https://mysupport.razer.com/app/answers/detail/a_id/20330) | 1.06 | Battery Saver, USB-C power modes, Synapse power-setting reliability |
| [MCU](https://mysupport.razer.com/app/answers/detail/a_id/19865) | 1.07 | Keyboard lighting reliability |
| [VBIOS](https://mysupport.razer.com/app/answers/detail/a_id/21227) | RTX 5090: 98.03.5C.40.25 | Battery Saver support |
| [PD](https://mysupport.razer.com/app/answers/detail/a_id/20413) | 2v00.14 | Both USB-C ports; Leviathan V2 X power-on fix |

The notes do not promise independent fan control. Battery Saver may involve
multiple firmware components; its HID command encoding is unknown.

## Read-only snapshot while waiting for BIOS update

With the owner's authorization, passive inventory and audited getters were run
while BitLocker decryption was pending. No setters or workload tests were run.

- BIOS remained 2.01 (08/04/2025).
- Controller getter `0081` changed from bytes `[1,6]` to `[1,7]`. This is a
  controller-version observation, not proof that the separately listed EC is 1.07.
- EC payload was `[2,69,0,0]`. Preserve the zero as raw evidence; do not use it as a
  valid GPU temperature or claim that its cause is known.
- Main fan readings: 2000/2000 RPM; selector 3 actual 0, stored target 3000.
- Balanced/Auto, brightness 176, static logo, battery care disabled.
- A subsequent single NVIDIA query reported VBIOS 98.03.5c.00.93 and GPU 48 C.
  It was not simultaneous with the EC read and may wake the discrete GPU.
- Raw local snapshots: `diagnostic-reports/research/pre-bios-decrypt-sensors.json`
  and `pre-bios-decrypt-inventory.json` (ignored).

## After every updater and required restart completes

1. Capture BIOS, controller and NVIDIA versions. Confirm EC/PD versions through
   their supported vendor status displays if no audited getter is available.
   Record unknown versions as unknown. Save a new snapshot; preserve old reports.
2. Read all three mode/target/actual fields and EC temperatures. Record whether
   Synapse is running, AC source and GPU activity. Compare raw payload shapes,
   errors and firmware-auto behavior with the baseline before enabling setters.
3. Inspect Synapse's Battery Saver and USB-C power options. Do not assume our
   existing Silent profile invokes the new firmware Battery Saver feature.
4. Validate CPU and GPU sources against independent references with timestamps,
   then separate bounded CPU/GPU workloads on AC with firmware automatic cooling.
   Measure missing values and latency; do not infer battery temperature from a
   fan's location or assign an EC field solely from one matching number.
5. Investigate shared manual-mode semantics and fan setter encoding. Separate
   target readback from actual RPM response. Capture Synapse behavior if needed.
6. Once actuation responds independently, repeat physical airflow mapping with
   an explicit owner-ready reply before each timed test and restore/verify all
   captured targets and automatic mode afterward.
7. Only then connect the offline curve planner to a guarded controller. Validate
   per-zone limits, missing/stale-sensor fallback, hidden-window operation,
   suspend/resume, profile conflicts, worker failure and shutdown restoration.
8. Recheck AC/battery profile switching and matched idle overhead after updating.
   Avoid continuous NVIDIA polling in the normal battery-idle path.

Offline tests validate calculation and command restrictions only. They do not
validate physical cooling, firmware behavior or an executed automatic fallback.

Offline verification: `cargo test --offline -p librazer --lib -j 2` passed all
20 library tests; `cargo check --offline -p librazer --bins -j 2` passed. No
hardware-control executable was launched by these checks.

## Installed-version baseline after the owner's updates

- The owner confirmed the BIOS update was skipped. Registry and Win32_BIOS both
  still report BIOS 2.01. Tests continue on that version; BIOS 4.01 is not assumed.
- NVIDIA now reports VBIOS 98.03.5c.40.25 and driver 610.74. The VBIOS matches the
  published RTX 5090 update. Controller getter remains 1.7; separate EC and PD
  installed versions have not been independently read.
- Initial audited readings: Balanced/Auto; main actual 1700/1700 RPM, selector 3
  actual 0; stored targets 4500/0/0. Do not treat these stored values as factory
  defaults or as physical speeds. EC payload `[2,67,46,0]`; subsequent NVIDIA
  temperature 46 C. Raw baseline: `post-update-sensors-20261014.json` and
  `post-update-thermal-baseline.jsonl` under ignored local research.
- Synapse's Blade 18 Performance page exposes Auto, Fixed Fan Speed and Manual
  Fan Curve. The curve interface offers CPU/GPU temperature selection and an
  option to use the higher temperature. This does not establish where the curve
  executes or independent physical fan mapping.
- A native pointer-applied fixed target of 3800 RPM changed all three target
  getters to 3800; actual readings rose through 3300/3300/3400. Merely setting the
  slider's accessibility value changed its displayed value without applying it;
  that attempted 4000 RPM phase is not a successful hardware test.
- The saved native fixed value was restored to 3000 and Synapse returned to Auto.
  The original hardware snapshot was then restored, including stored 4500/0/0.
- A nearby vendor telemetry sample labeled CPU 56 C and GPU 38.1 C, while the
  last EC sample was 57/38 C. This supports association at steady state, not
  independent die accuracy or transient-response validation.
- Source-backed mode/write-prefix and framing comparisons are diagnostic-only
  experiments. Standard transaction/checksum framing encountered a short report;
  the test stopped and verified restoration. It is not enabled in normal control.

Live comparisons revealed that a mode write after a fan target can leave actual
speed near the manual default of 3000 RPM while the target getter retains the
requested value. Per-selector command order and delayed target reassertion now pass the bounded
actual-RPM checks below. Live curves remain disabled pending sensor and physical mapping. All temporary
tests journal original settings and attempt restoration on completion or error.


## Verified actuation and remaining mapping

- `post-update-verified-primary-control.json`: 90 seconds per unequal-target case.
  Requested 4000-4500 settled near 3900/4400 RPM; reversing to 4500/4000 settled
  near 4400/3900-4000. Each case requires at least three consecutive five-second
  readings within 200 RPM. Actual-RPM verification passed; restoration passed.
- `post-update-paired-init-third-fan.json`: requested 4000/4000/3500, measured
  4000/3900/3400. Third target and all automatic modes restored without error.
  This establishes a responding logical actuator, not its physical location.
- Normal control reasserts both primary targets one second after entering Manual.
  Subsequent target updates avoid mode writes. Restoring a captured manual state
  uses the same delayed reassertion. Public targets remain AC/Balanced only,
  4000-4500 RPM; lower mapping baselines are diagnostic-only.
- The new isolated mapping option warms all three targets at 3000 RPM for 60
  seconds and requires convergence before changing only the selected target.
  Temperature, power, mode and target checks abort to captured-state restoration.
  This sequence is built and checked offline; owner-assisted physical mapping
  still requires a fresh ready reply before starting each test.
- Read-only capability getters succeeded after narrowly accepting their observed
  response headers. Fan-ID list is `[4,1,2,3,4]`: four logical IDs, **not evidence
  of four physical fans**. Selector 4 is neither queried nor writable. Timing/SKU
  fields remain raw metadata until model-specific interpretation is validated.
- Windows battery-class queries expose capacity, voltage, cycle count and rate,
  but the temperature query returns Win32 error 1 (unsupported). No validated
  battery temperature is available for a third-fan curve. An AC rate of zero is
  not a measurement of application power overhead.

Verification: all 30 workspace tests passed and the Windows debug build succeeded.
The new UI and original R icon are included. The pure curve planner remains
separate from hardware; no unattended software fan control is enabled.
