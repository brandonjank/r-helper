# Extended Blade 18 controls and lighting ownership — 0.6.0 test candidate

Target: RZ09-0529, HID 1532:02c7. Research and local tests: 2026-10-14.
This is a source-backed inventory and a batch-test candidate, not proof that every
undocumented firmware command has been found. Shared-library commands for mice,
headsets, wireless chargers and analog keyboards are excluded.

## Newly implemented

| Control | Protocol / evidence | Test status |
| --- | --- | --- |
| Function-row primary action | `0286` / `0206`, fixed profiles 0 and 1, values 0 (F1–F12) and 1 (multimedia). Model task explicitly uses both profiles; profile 1 is called persistent in hardware. | Both profiles independently changed and restored with exact readback. Physical Fn behavior and reboot retention need owner checks. |
| Gaming mode switch | `0088` / `0008`, only `[0 or 1,0,0,0]`. | On/off round trip passed. This is not the custom Windows/Alt+Tab/Alt+F4 mask API; that separate getter is unsupported. Key blocking and indicator appearance need owner checks. |
| Firmware Chroma flag | `0f90` / `0f10`, Boolean. | Both values round-trip. Experimental: this is not an OS ownership lock and is not enabled automatically as a conflict fix. |
| Native keyboard Off | `0f02 [1,5,0,0,0,0]`. | Command/readback/restoration passed. |
| Native two-color Starlight | `0f02`, effect 7, fixed region 5/profile 1, two RGB colors, duration 1–3. | Default and alternate durations passed exact readback/restoration. Appearance needs owner confirmation. |
| Native Ripple | `0f02`, effect 6, one RGB color, fixed duration 2. | Exact command/readback/restoration passed. Physical key-trigger behavior needs owner confirmation. |
| Reactive duration choices | Existing effect 5, duration 1–4. | All four values passed exact readback/restoration. |
| Adapter information | Existing `078c` response decoded to adapter and recommended classes. | Both report 400 W on this unit. This is an advertised class, not measured power delivery. Read only and on demand. |

The ordinary GUI adds keyboard behavior controls, a recoverable keyboard batch
button, remaining-feature inspection and the above firmware effects. No keyboard
driver mode, key remapping, voltage offset, fan-table, graphics-mode or firmware
write is introduced. Existing profiles load without the new timing/ownership fields.

## A separate Windows lighting interface

Passive HID enumeration found interface 3, usage page `0059`, usage 1: the standard
HID Lighting and Illumination LampArray interface. Windows Dynamic Lighting is
enabled in this account. Its presence is a plausible additional source of the
post-resume color override; the exact writer of the earlier red color is unproven.

Read-only Windows metadata reports **115 RGB lamps**:

- Indices 0–96: 97 lamps with Control purpose, including keyboard key mappings.
- Indices 97–114: 18 RGB lamps with Branding purpose, placed along a horizontal line.
- RGB/gain level counts are all 255. Their physical correspondence to logo or
  underside is not yet confirmed. Do not relabel them from positions alone.

The Windows lighting map shows reported positions and key-to-lamp associations.
Inventory now runs in a background helper, not the foreground GUI process, and
sends no colors. The explicit **15-second auxiliary test** checks the exact
115-lamp / 97-Control / 18-Branding descriptor in an isolated temporary window.
It requires a recoverable Static keyboard preset. The 97 keyboard lamps receive
the saved uniform color scaled by saved brightness; the 18 auxiliary lamps receive
the test color, with an optional orange highlight confined to lamp 1–18.
This writes all 115 colors in one buffered update to avoid leaving keyboard lamps
uninitialized. Windows must report foreground lighting availability before a write.

The main GUI hides in the tray during the test. The temporary owner closes after
15 seconds (or the owner presses End test), and the parent waits for process exit
before reapplying the captured firmware lighting under its device operation lock.
A separate recovery helper retries restoration after 45 seconds if the parent
fails; a successful parent restoration disarms it. A failed startup before any
preview also disarms recovery so it cannot overwrite later settings. The parent
terminates a test that exceeds 25 seconds. Reports distinguish API acceptance,
firmware readback and required physical confirmation. Original LampArray pixels
cannot be captured, and exact Windows ambient colors are not claimed as backed up.
Static-only testing avoids inventing a backup of dynamic effects or per-key pixels.

This preview is not a complete background RGB implementation. Microsoft's ambient
background lighting contract requires packaged app identity and a lighting app
extension. The ordinary unpackaged app currently offers firmware effects plus a
foreground-only standard-interface test. It does not disable Windows lighting or
change global personalization settings. A shortcut opens Windows' lighting page.

## Unsupported and unresolved findings

### Physical auxiliary preview follow-up

With R-Helper elevated and Game Manager/RGS verified stopped, the owner tested
blue (`#2040FF`) and red (`#FF2020`) auxiliary requests. Windows accepted both,
but the owner saw only a green logo; the underside and keyboard were off during
the red test. The owner confirmed the physical logo is green-only with brightness
control, not RGB. Retain the existing single-color logo controls and do not expose
a logo color picker. This **fails physical RGB validation**. It does not establish that
the 18 reported Branding lamps correspond to the logo or that their advertised
RGB channels work. Foreground ownership may also affect unaddressed keyboard
lamps, so auxiliary-only indexing does not guarantee an unchanged keyboard.
The timed preview ended, and the verified firmware static-blue preset was
reapplied with exact readback (keyboard brightness 176, underside off). The owner
subsequently reported that the keyboard was not blue despite this readback;
visible restoration is therefore **not verified**. Resolve Windows/firmware
lighting ownership before sleep/wake testing. Follow-up: after the owner ended
the preview and hid R-Helper, reapplying the same firmware preset visibly restored
blue and it stayed blue. Game Manager/RGS remained stopped and the recovery guard
reported no errors. This implicates foreground-lighting interaction but does not
identify its exact cause; hidden steady-state restoration is now owner-verified.
The owner also confirmed blue returned and stayed blue after sleep/wake with
R-Helper hidden and AC connected.
Keep this interface experimental pending mapping and ownership investigation.

The isolated-window cleanup and keyboard seeding are implemented following these
observations. Their physical behavior still needs a fresh owner test; earlier
failed blue/red previews are not retroactively counted as passing.
The new standalone recovery helper passed both a completed-parent cancellation
check (no fallback write) and a real 45-second fallback reapplication of the saved
blue preset with firmware readback. This validates timed recovery, not yet the
complete isolated foreground test or its visible restoration.

The fixed inventory probes 19 audited getter shapes. It does not scan command IDs,
LED IDs or device modes. Results are saved privately in ignored local reports.

| Lead | Result / disposition |
| --- | --- |
| Smart illumination | Getter returns `[1,0,0]`; a source-shaped setter did not change it. Restored, then removed from the setter allowlist. No working ambient-light feature claimed. |
| Function-key swap, Mac/Windows layout switch | Exact shared-library getters returned NotSupported. No setters exposed. |
| Custom gaming key selection/masks | Both exact getters returned NotSupported. Keep separate from the verified gaming on/off switch. |
| Fn lighting overlay and quick-effect switch | Exact getters returned NotSupported. No setters exposed. |
| Firmware dim/sleep/battery-brightness/indicator/low-power getters | Successful replies contain zeros. These are not proof of usable policies; setters remain blocked. |
| Spectrum | Both the earlier padded request and a corrected six-byte request failed to apply. The last static effect remained. Excluded from presets and the setter allowlist. |
| Fast Wave | Speed 16 request read back 40. The alternate speed is ignored and remains blocked. Normal speed 40 is retained. |
| Legacy LED inventory | Two entries: keyboard RGB and logo PWM. It adds no extra legacy LED IDs; the separate LampArray carries the new auxiliary lead. |
| Firmware fan tables / voltage / MUX / panel-resolution writes | Existing leads still lack validated recovery or model semantics. Not enabled by this batch. |
| Battery Saver | The existing BIOS/EC/VBIOS gate is not satisfied by BIOS 2.01. No forced enable. |
| Other Razer device IDs | Present-device inventory found only VID 1532/PID 02c7; no present VID 1534 device. |

## Reversible Razer-software takeover

The new **Laptop control ownership** card provides opt-in takeover and conflict
inspection. It pauses only fixed Razer control services/app names, verifies app
paths under the Razer installation directories, and rechecks executable identity
through the handle used to end a process. Driver services, Windows services and
the Razer elevation service are excluded. Shared Razer peripheral features may
pause because Synapse/Chroma services serve more than the laptop.

Before any pause, it checks required service access and writes an immutable record
of installed managed services, their original running states, and original app
paths. A separate process holds an ownership mutex, waits on the owner/release
events, and restores original service states/apps on normal release or owner exit.
While active it checks for vendor relaunches every 30 seconds. It never changes
service startup types. A surviving parent can recover if the guard itself fails.
Imported profiles cannot enable takeover.
The GUI also holds a single-instance lease, so opening a second copy brings the
existing controller forward instead of starting competing policy workers.

The initial non-administrator preflight denied access to stop Razer Game Manager
Service 3 and left services unchanged. The owner subsequently approved the
administrator launch for the real takeover/restore test. No persistent service
disable change was issued.

Follow-up: the administrator GUI successfully paused Game Manager Service 3 and
RGS. Both retained Auto startup; the excluded Razer Elevation Service stayed
Running/Manual. The recovery guard reported no errors on repeated checks.
Normal Exit restoration passed with this real service record: both Game Manager
and RGS returned to Running/Auto, Elevation remained Running/Manual, and comparison
against the pre-takeover baseline found no differences. The guard reported
`ok: true` with no error; neither the GUI nor a recovery helper remained running.

## Validation and return-to-desk batch

The complete workspace suite passes **68 tests** after the ownership cleanup. The optimized Windows build
passes. Clippy completes with existing repository warnings; a strict warnings-as-
errors pass is not clean, and no new-module lint warning remains.

After controller/Windows-lighting inspection, the hidden release used 0.046875 CPU
seconds over 30.01 seconds (about 0.156% of one core), with a 140 MB working set.
This was on AC, with no active preview and vendor takeover blocked by service
permissions. It does not measure whole-system battery drain or the privileged
takeover helper's overhead.

- Four keyboard/control apply/readback/restore cases passed with an independent
  45-second recovery process. The GUI batch tool uses the same bounded path.
- Native lighting effects and supported duration choices passed command/readback
  checks, with restoration after each case and an independent recovery process.
- Ownership normal release, owner exit and recovery-guard exit were tested using
  empty records, without touching Razer services. These are recovery-mechanism
  tests, not evidence that a privileged service takeover succeeded.
- LampArray metadata enumeration and foreground availability passed in the GUI.
  A blue auxiliary color request returned success, and the GUI confirmed automatic
  ownership release after 15 seconds. This is API/lifetime evidence, not visual
  proof of the physical zone or pixel colors. Physical zone identity, Fn behavior,
  gaming-key behavior and new effects need owner checks.
- The release tools passed four keyboard/control cases and 16 lighting/charge
  cases. The native GUI keyboard batch also passed and restored its original values.
- The final release GUI also accepted the per-lamp orange-highlight request,
  restricted to the 18 auxiliary lamps. Explicit release passed. A physical
  observer is still needed; success means the Windows API accepted the request.
- All ten existing release integration cases passed, including profile rollback,
  idle/application policies, injected display/resume events, and display timeout
  recovery. Hardware, lighting and display were restored.
- Original performance/fan/charge-limit/lighting settings are restored after the
  automatic HID batches. Raw reports and machine-local configuration stay out of Git.

Remaining owner checks: confirm Fn defaults with and
without Fn; check gaming mode; visually check Ripple/Starlight and their durations;
run the 15-second auxiliary preview and identify the changed physical zone. Then
test AC switching with the intended lighting owner selected. Privileged takeover,
normal exit restoration and hidden sleep/wake lighting passed. Keep auxiliary
LampArray previews experimental until mapping and visible restoration are fixed.

## Primary protocol sources

- [Razer model-711 task bundle](https://apps.razer.com/synapse/products/711/mw/6737.097a717c0f79e9e4ca4b.js): actual model Fn/gaming callers.
- [Razer keyboard command constants](https://apps.razer.com/synapse/products/711/mw/3159.d10b9ea4a587313b3d6e.js): exact getter/setter shapes.
- [Razer shared protocol library](https://apps.razer.com/synapse/products/711/mw/rzDevice25.279816c47f47f21a061c.js): Chroma, power and LED formats.
- [OpenRazer protocol documentation in source](https://github.com/openrazer/openrazer/blob/master/driver/razerchromacommon.c): extended firmware effect formats. Protocol facts only; no implementation copied.
- [Microsoft Dynamic Lighting documentation](https://learn.microsoft.com/en-us/windows/apps/develop/devices-sensors/lighting-dynamic-lamparray): priority, autonomous behavior and background identity requirements.
- [Microsoft LampArray API](https://learn.microsoft.com/en-us/uwp/api/windows.devices.lights.lamparray): metadata and explicitly indexed colors.
- [Microsoft LampArray.IsEnabled](https://learn.microsoft.com/en-us/uwp/api/windows.devices.lights.lamparray.isenabled): buffered updates while disabled and flush on enable; disabling alone is not an ownership-release guarantee.
