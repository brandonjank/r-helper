# Blade 18 feature batch — R-Helper 0.5.0

## Delivered controls

- Firmware Static, Breathing, two-color Breathing, Wave left/right and Reactive.
  Spectrum remains excluded because the previously tested commands did not apply.
- Experimental static per-key colors: a 6 × 19 matrix editor, fill and alternate-row
  actions. One Apply sends six bounded rows; no animation thread or recurring frame
  traffic runs. Some matrix cells do not correspond to physical keys.
- Lighting presets include keyboard effect/colors/map/brightness, logo mode,
  underside enable and underside brightness. Startup animation remains global.
- Separate saved AC/battery lighting presets. Existing automatic power profiles
  apply them with performance and refresh settings; lighting policies can also
  select them independently of performance-profile automation.
- Opt-in lighting rules: separate AC/battery idle timeouts, display-off blanking,
  low-battery cutoff, and resume restoration. Zero disables a timeout or cutoff.
  Blanking changes brightness/logo/underside state without replacing keyboard effects.
- Up to 32 named local profiles: capture, update from current, rename, duplicate,
  remove, apply, and JSON copy/import. Profiles contain performance, battery care,
  lighting and a panel-specific refresh setting. Imports validate before replacement
  and leave all automation off. The existing AC/battery settings format still loads.
- Optional exact executable-name matching for the foreground application. Profiles
  that require AC are skipped on battery. Leaving the application returns to the
  enabled power profile, or the captured previous settings.
- Firmware charge-once request, status and cancellation. The normal charge-limit
  setting is not changed. Only observed shared power flag patterns `08` and `0c`
  can be written; other bit patterns are rejected.
- Manual refresh changes now have a 15-second Keep/Revert prompt. A separate process
  restores the original rate after 25 seconds without confirmation, including when
  the GUI exits. Automatic/named-profile changes remain explicit configured actions.
- On-demand current/associated Windows ICC profile names and a Windows Color
  Management shortcut. ICC selection uses Windows' own UI; R-Helper does not change
  ICC associations, HDR, MUX or native-resolution firmware settings in this batch.

## Automation and power use

New automation defaults off. Existing AC/battery profile automation keeps its saved
setting. Native notifications handle AC, display state and resume. Idle time,
battery percentage and foreground-process matching are sampled every five seconds
only when an enabled rule needs them. AC-notification failure uses a 30-second
fallback. Failed operations back off and retry rather than enforcing continuously.
No NVIDIA temperature query, GPU compute workload or RGB frame loop is added to
the normal application.

Application profiles take priority over power-source lighting choices. An explicit
manual application becomes the lighting baseline until the power source changes.
Idle/display-off/low-battery rules then control whether those lights are visible.
Disabling a rule restores its unblanked levels. Resume replay is opt-in and checks
the panel identity and driver-listed modes again before applying refresh settings.

Lighting/application policies defer during supervised custom fan sessions and
pending display confirmation. Curve launch and profile operations share a process
operation lock, with a second curve-state check after acquiring it. Explicit user
profile actions and power transitions stop curves and verify recovery before writing.

## Recovery and limits

Profile and lighting journals are written before hardware changes. Named-profile
failures attempt hardware/display/lighting rollback; partial recovery errors are
reported. Short HID replies on audited getter/idempotent-control paths receive
bounded retries. Unknown commands and unreviewed packet shapes remain blocked.

Per-key pixels cannot be read back from this controller. R-Helper saves its last
applied preset locally so its own map can be reused across restarts. That cache is
the last R-Helper write, not a hardware pixel backup: another lighting controller
can overwrite a custom frame without changing its effect metadata. Close competing
lighting controllers for per-key editing. Capturing an unknown external Chroma map
is rejected; apply an R-Helper firmware effect first. Per-key output remains marked
experimental until owner visual validation.

Charge-once command/readback and cancellation are distinct from proving that a
complete charging cycle reaches full and automatically clears the override.
Firmware controls the charging behavior. No arbitrary charge percentage, voltage,
firmware fan-table or undocumented graphics setter is enabled.

Host-rendered Fire/Ripple/audio/screen effects and Chroma device synchronization
remain outside this version. Firmware-native effects and static maps avoid their
continuous traffic. Logo RGB and underside RGB remain unestablished on this SKU.

## Validation

Local raw reports are ignored under `diagnostic-reports/research/`; they are not
included in public source or release artifacts.

- `feature-lighting-batch-20261014.json`: Static, Breathing, dual Breathing, both
  Waves, Reactive and per-key-mode command/metadata checks, restoration after each,
  and charge-once Start/Cancel. Pixel readback is unavailable.
- `integrated-feature-batch-20261014/result.json`: named apply/rollback, wrong-panel
  rejection, save/load/import, real idle blank/restore, foreground application match,
  display Keep/Revert/independent timeout, and final hardware/lighting/display recovery.
- The expanded resume run preserved a short-HID-reply failure and verified rollback
  in `integrated-feature-final-20261014/result.json`. It motivated the bounded reply
  retry and event-driven retry scheduling fixes; it is not recorded as a passing run.
- `integrated-feature-retry-20261014/result.json` passed all ten integration cases,
  including injected display-off/resume events and final recovery of hardware,
  lighting and display settings. The complete workspace suite passed 59 tests.
- UI tests independently read back dual Breathing, charge-once Start/Cancel, saved
  named-profile contents and a combined profile's 60 Hz display change. The Keep
  prompt retained 120 Hz in a separate test.
- Owner visual validation confirmed that two-color Breathing alternates blue and
  orange, and both Wave directions move across the keyboard. Direction flag `01`
  moves right and `02` moves left from the typing position; the initial labels were
  corrected accordingly. Reactive was also visually confirmed: pressed keys light
  blue and fade. Alternating blue/orange per-key rows were visually confirmed on
  the main keyboard and number pad. Individual matrix-cell mapping remains
  experimental; controller metadata cannot read back pixel colors.
- A sustained charge-once test observed the request become active and later clear.
  Battery-class readings remained at approximately 98%, with a zero charge rate;
  this does not establish that an override charged through the normal limit or
  completed a full charging cycle.
- Physical AC/battery lighting transitions passed with the optimized GUI build:
  unplugging selected orange at 64/255, and reconnecting restored blue at 176/255
  while hidden. Owner observations agreed with independent controller readback.
- Hidden on battery with event-driven lighting policies enabled, the release used
  0.015625 CPU seconds over 30.03 seconds (about 0.052% of one core), with a
  125 MB working set. This measures app CPU use, not whole-system battery drain.
- Physical sleep/resume produced a successful blue-preset recovery journal, and
  the owner saw blue return before the keyboard later appeared red. Subsequent
  firmware readback still reported blue, while underside enable had changed.
  The source of the later visible change is unresolved; this does not establish
  stable lighting ownership when vendor software/firmware also controls lighting.

Display-off and resume event injection verifies policy routing and real hardware
writes. Physical observations are recorded separately above. A full charging cycle
has not been validated.

Developer commands (use a new output file/directory each time):

```powershell
.\target\release\rhelper-features.exe --validate-lighting --output lighting-test.json
.\target\release\rhelper.exe --test-feature-batch new-batch-directory
```

Both are opt-in hardware tests. The GUI harness captures recovery records, restores
settings after failures and does not edit the user's saved profile file. The lighting
harness additionally launches a bounded recovery process before its first setter.
