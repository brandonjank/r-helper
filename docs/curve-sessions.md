# Experimental app curve sessions

The Blade 18 app now includes an independent three-fan curve editor, source
selection, saved points, live RPM/temperature readings while active, and Stop +
automatic. Sessions are opt-in on AC in Balanced/Auto, last 30 seconds to 8 hours,
and never start at login, from a power profile or on battery.

The editor uses neutral fan numbers. EC A/B are controller proxies; the third
default is their maximum, not battery temperature. Physical fan roles and CPU die
accuracy remain unresolved. Main targets are restricted to 4000-4500 RPM and the
third to 3000-3500, in 100-RPM steps. These are limited experimental envelopes,
not physical fan limits. Invalid/nonmonotonic points disable Save and Start.

Saved settings live in `%LOCALAPPDATA%\RHelper\curves.json`; per-session config,
immutable recovery and status files live under `RHelper\curve-sessions`. A corrupt
saved file is preserved and reported rather than silently overwritten.

## Runtime and recovery

The GUI launches a separate watchdog, which launches the controller. The
controller samples about every two seconds, skips unchanged targets and uses
immediate increases with damped decreases. Normal inactive app operation has no
curve timer, GPU telemetry loop, watchdog process or heartbeat traffic.

- Requires available nonzero EC readings, EC A below 85 C and EC B below 75 C.
  Startup also requires headroom below 80/70 C.
- Stops on AC loss/unknown power, cooling/target conflicts, a stale query, a worker
  time gap of 8 seconds (including resume/clock changes), or session timeout.
- Watchdog monitors app lifetime and controller heartbeat. A stalled controller
  is stopped, then terminated if cooperative exit fails, before fan-only recovery.
- Controller separately monitors watchdog heartbeat and returns to Auto if the
  watchdog is lost. The GUI also attempts recovery if its watchdog exits early.
- Stop, app exit and device/profile operations coordinate through recovery before
  applying another setting. Power-profile switching waits for verified recovery.
- Recovery changes only fan mode and captured target registers. It does not
  restore old brightness, battery, CPU/GPU boost or performance settings over a
  newer profile. All three Auto modes and original targets must read back.

Neither this nor a Windows app can guarantee cleanup after simultaneous process/OS
failure or an unresponsive hardware transport. No voltage/firmware writes occur.
Physical sleep/resume and AC-transition testing with this new session backend
remain pending; time-gap and profile-handoff guards are covered by unit tests.

## Validation on the development Blade 18

`tools/Test-AppCurves.ps1` tests Stop, Timeout, WorkerExit, WatchdogExit and ParentExit
against the actual app executable. Each case captures before/after controller
readings. All five passed Auto/target recovery. The first timeout verification
hit a transient independent-read failure; logs were preserved, the harness gained
bounded read retries and correctly awaited GUI-subsystem recovery, and the new
timeout run passed. Do not erase or relabel the original failed report.

Native UI checks verified chart rendering, invalid-point rejection, restoring a
valid point, saving config, starting a live session and live three-fan readings.
A performance-mode hotkey stopped/recovered the session before applying Silent;
Balanced/Auto was then restored and independently read back.

Headless roles run before GUI/device initialization: `--curve-supervisor`,
`--curve-worker`, `--curve-recover`. These are implementation/testing entry points;
ordinary use is through the app. The worker requires a live watchdog lease.

Inactive optimized-process samples after integration: visible 0.484375 CPU
seconds in 30.029 seconds (1.61% of one core); tray-hidden 0.015625 in 30.025
seconds (0.052%). Hidden overhead matches the earlier framework-fix baseline.
These short AC-powered process samples do not measure GPU wakeups or battery
energy, and do not prove a battery-life change.
