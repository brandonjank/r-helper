# Blade 18 controller discovery

Investigation: 2026-10-14, RZ09-0529 / 1532:02c7, BIOS 2.01.
This pass used static inspection of already-downloaded public vendor bundles and
bounded HID getters. No hardware settings, firmware, voltage or fan targets changed.

## What "hidden" means here

A method in the shared Razer library is not proof that the Blade implements it.
A model feature flag is stronger evidence of intent, but not proof of an accepted
setter. An absent caller in the inspected bundles does not establish absence from
every Synapse version, dynamically loaded module, native service or firmware.
We have not confirmed a new working control that Synapse itself never implements.

## Findings

| Feature | Evidence | Classification / next step |
| --- | --- | --- |
| Connected/recommended charger wattage | Model runtime enables `hasAdapterWattageLevel`; task bundle calls its getter. `078c [0,0]` returns `[19,19]`; the vendor enum maps both to 400 W. This is a declared adapter class, not measured power delivery. | Newly verified telemetry for R-Helper, already used internally by Synapse. Useful for explaining limited performance on smaller adapters. |
| One-time full-charge override | Model enables `hasBatteryChargingOverrideEnabled`. Task reads `078f`, modifies bit `04`, then writes the combined flags via `070f`. Current getter returns `08`, so override bit is clear. | Concrete implementation path for the requested charge-once feature. Preserve every other bit; test cancellation and completion before claiming one-time behavior. No setter sent. |
| Shared power flags | Vendor names bits `01` CPU boost, `02` maximum fan mode, `04` charge once and `08` local dimming. Current byte is `08`. | Names are shared-library semantics, not proof that all four functions apply to this SKU. In particular, `08` is not proof of a mini-LED panel. Do not expose maximum-fan mode from this evidence alone. |
| SKU display capability bits | `0d8f [0]` returns `01`. Vendor masks: DDS `01`, mini-LED resolution `02`, illegal-battery support `04`. Model runtime also sets `alwaysShowNativeDisplayMode`. | DDS is advertised; the other two bits are clear. Mode-switch writes and reboot requirements remain untested. A forced-visible UI option is not hardware validation. |
| Fan duty-cycle/PWM control | Shared library defines `0d8d` getter / `0d0d` setter. No getter caller found in the inspected task bundle. Exact getter `[0,selector,0]` returns NotSupported for selectors 1, 2 and 3. | Candidate not supported in these forms. No setters, extra classes or selector sweeps. Keep normal RPM-based controls. |
| Firmware fan tables | Shared `setThermalFanTable` encodes class/fan/curve and temperature/16-bit target segments with a terminator. No caller found in inspected model task bundle. | Promising firmware-executed curve lead, not a working feature. No recoverable table getter established; no table writes. |
| CPU active-core ratio limits | Shared getter exists; `setRatioLimit` public wrapper has an empty generator body. Model Intel tuning instead uses native tuning APIs, including a separate core-voltage-offset path. | Unfinished/legacy library surface, not an unlocked CPU feature. No CPU ratio or voltage probing/writing. |
| Fn/status lighting | Model enables gaming, microphone mute, speaker mute, Fn and touchpad status lighting. Blade2 audio-mute command is class `18`, ID `04`; task has speaker/mic event callers. Touchpad status uses system multifunction handling. | Already implemented internally in Synapse. Useful to retain status indicators when implementing keyboard effects; no writes during discovery. |
| Fn overlay / smart illumination | Generic getters exist, with no callers found in the inspected model task bundle. "Smart illumination" uses shared Half/Quarter/Off labels. | Unverified leads, not evidence of ambient-light sensing on this Blade. No probes or setters. |
| Battery discharge "curve" | Shared enum for this method is Alkaline/NiMH/Lithium. | Peripheral battery chemistry setting, not laptop discharge policy. Exclude from Blade probing. |
| USB4/router and panel information | Blade2 exposes native helper actions for USB4 router firmware and panel producer; task bundle has callers. | Existing Synapse internals useful for our future diagnostics/update screen. Native invocation not yet validated. |
| Battery Saver | Model gates it on BIOS 4.01, EC 1.06 and the model-specific VBIOS. | Firmware-gated existing feature, not hidden. BIOS 2.01 does not satisfy the gate. |
| Logical fan 4 | ID list still advertises 4; actual 0, stored target 3000, mode Auto. | Existing unexplained logical channel. No evidence of a fourth physical fan; no selector-4 writes. |

## Live evidence and parser handling

Raw reports stay local under ignored `diagnostic-reports/research/`:

- `controller-discovery-20261014.json`: initial charger header rejection, preserved.
- `controller-discovery-accepted-20261014.json`: accepted `[19,19]` charger result.
- `controller-discovery-duty-20261014.json`: repeated charger/power/SKU results and
  unsupported duty-cycle getters for the three known fans.

The charger reply uses wire remaining-packet bytes `00 02`, like several previously
observed capability replies that encode the returned-byte count in that field.
The new exception requires matching transaction and command identity, request and
response size 2, command `078c`, header `0200` in the stored little-endian value,
successful status and documented adapter enum values. It does not relax other
commands or authorize new setters. Tests cover malformed requests, adjacent
commands, enum values and header/length mismatches.

Validation: all 52 workspace tests passed with `librazer/diagnostic-gpu`, and the
diagnostic binary built and completed the live read-only queries. The initial
sandbox run denied a temporary-file profile test; the unchanged suite passed with
normal Windows access. The running release app was not replaced by this patch.

Repeat the read-only inventory with:

```powershell
.\target\debug\rhelper-diagnostics.exe --probe-thermal-capabilities --output new-report.json
```

## Requested implementation batch after discovery

1. Keyboard effects: extend verified firmware effects first, investigate per-key
   frames separately, retain status/Fn indicators, keep continuous animation optional.
2. Lighting policies: AC/battery brightness, idle/display-off and low-battery rules;
   coordinate with profiles and resume using events instead of a fast polling loop.
3. Profiles: named local profiles, duplicate/import/export and application matching;
   integrate lighting and display with deterministic AC/battery precedence.
4. Charge fully once: shared-bit-preserving command, visible active/cancel state,
   readback and sustained full-charge/reversion validation.
5. Display: timed Keep/Revert for manual refresh changes, dock/resume handling,
   color-profile inventory; investigate DDS/native-resolution modes separately.

These are accepted next tasks, not implemented by this discovery patch. Ordinary
60/120/240 Hz and AC/battery refresh switching already work.

## Sources and reproducibility

Public vendor files inspected (protocol facts only; no vendor implementation copied):

- [Model-711 runtime/configuration](https://apps.razer.com/synapse/products/711/mw/main.c953ca79d6a9fe3975bd.js)
  SHA-256 `DEE8FDD18F3953506DCF6DD93E0018E9CA50E88E874EFDA1E0774D444BD4B840`.
- [Shared protocol library](https://apps.razer.com/synapse/products/711/mw/rzDevice25.279816c47f47f21a061c.js)
  SHA-256 `24B693D9CB6597C50B93BD98DE2E6C4791C135F056B0604B3791FE168DA551B4`.
- [Task bundle](https://apps.razer.com/synapse/products/711/mw/6737.097a717c0f79e9e4ca4b.js)
  SHA-256 `7BDABDE3DAE57EF13AFF179B66DC14CD90EABB7B50BE14427786A2039A374556`.
- [Blade2 implementation](https://apps.razer.com/synapse/products/711/mw/rzDevice25LedMatrixBlade2.170063b45f6854daf8ea.js)
  SHA-256 `5E8624F5C97DC10BB26C6DE7F4905839F3E5A8DE45AF4F706C6932A83AAE8FE7`.
- [Blade 18 Synapse guide](https://dl.razerzone.com/master-guides/RazerSynapse4/Blade18-00000711-en.pdf)
  documents existing owner-facing lighting, keyboard, display and charge-once controls.
- [OpenRazer for Windows](https://github.com/Ar4ikov/openrazer-win) is an additional
  protocol-reference lead for our keyboard work. Its project describes emulator
  testing; a device listing is not verification on our machine. No package installed,
  code copied or daemon started.
