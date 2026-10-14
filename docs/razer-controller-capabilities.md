# Blade 18 controller capability inventory

Target: RZ09-0529 / PID 1532:02c7. Research date: 2026-10-14. BIOS remains 2.01;
the owner skipped the 4.01 update. A command in a shared protocol library or a
Synapse feature flag is not proof that this SKU accepts its setter.

## Verified on this machine

| Capability | Current evidence / use |
| --- | --- |
| Performance and CPU/GPU boost presets | Existing app controls, readback and recovery tested. These are firmware presets, not voltage offsets. |
| Three numbered fan channels | Actual/target/mode getters; separate measured target response. App now offers explicit, supervised EC-based curve sessions on AC/Balanced. Physical fan roles remain unresolved. |
| Two controller temperatures | A is CPU-load associated; B tracked NVIDIA within 2 C during a moderate GPU response test. Independent CPU die accuracy remains open. |
| Keyboard brightness and logo modes | Existing brightness and Off/Static/Breathing controls. Full independent logo RGB has not been established. |
| Battery care | Existing 80%-target/full-charge request. Sustained charging cutoff validation is pending. |
| Controller firmware | Readable; do not equate it with separately unconfirmed EC/PD firmware versions. |
| Graphics configuration getter `0d89` | Returns `[2,0]`. Generic metadata describes platform/mode fields; graphics-mode writes and restart requirements remain unverified. |
| Panel-resolution getter `0d8e` | Returns `[0]` after accepting its exact observed one-byte header variant. Panel-mode writes and enum interpretation remain unverified. This is separate from ordinary Windows refresh-rate selection. |
| Startup-animation getter `0f98` | Returns `[0]`; vendor parser names this field `disableAnimation`. Getter works; setter profile/storage semantics have not been validated. |
| Logical fan ID 4 | Advertised and readable: actual 0, stored target 3000, Auto. Purpose unknown; no fourth-fan control or physical fan claim. |

The class-0 alternate cooling-mode getter `0d90` returned **NotSupported**. This
does not establish that a different platform/class has the same behavior, and
does not justify selector/class sweeps or claiming a working fan-stop control.
Raw local evidence: `extended-controller-getters-accepted-20261014.json`.

## Model-specific leads worth implementing next

The public model-711 middleware advertises the following. Some route through
Razer's native helper, Windows or NVIDIA rather than the laptop HID endpoint.

| Lead | What remains before an app control |
| --- | --- |
| Underside/vapor-chamber lighting | Model feature `hasVCLed` is enabled. Identify independent LED IDs/matrix regions, effects and restoration. Avoid continuous frame streaming for ordinary effects. |
| Fn and status lighting | Model configuration lists gaming mode, microphone mute, function key, speaker mute and touchpad-status indicators. Establish supported getters and overlay semantics. |
| Lighting on AC/battery, idle and startup | Two brightness modes, lighting task policies and startup-animation control exist in the model stack. Separate host policies from firmware-stored settings. |
| Battery charging override | `hasBatteryChargingOverrideEnabled` is advertised. Capture exact semantics and native-helper/controller path; do not infer arbitrary charge percentages. |
| Battery Saver and USB-C power modes | Razer's EC changelog adds these. Model middleware gates on-battery Battery Saver on BIOS 4.01, EC 1.06 and model-specific VBIOS versions. BIOS 2.01 does not satisfy that gate. Controller 1.7 is not proof of EC 1.06. |
| Native panel/dual-resolution mode | Model advertises resolution/native-display support and refresh-rate profiles. Enumerate actual panel modes and determine reboot/driver requirements before setters. |
| GPU/MUX/display switching | Graphics configuration is readable, and the model UI defines hybrid/discrete/DDS options. Determine supported SKU modes, provider and restart behavior; do not equate these with clock tuning. |
| Firmware fan tables | Generic protocol has a fan-table command. Readback, segment/fragmentation format and recoverability are not established. Prefer firmware execution if it can be verified. No table writes have been sent. |
| USB4 router and panel information | Native helper exposes router firmware and panel-producer getters. Exact invocation and applicable hardware must be identified before using them. |
| CPU tuning/core controls | Model stack advertises Intel overclocking and split core voltage offsets; generic ratio-limit commands also exist. No validated nonzero voltage-offset range exists for this unit. Shared UI limits are not safe CPU-specific limits. |

GPU frequency/voltage curves, power limits and clock caps generally require NVIDIA
interfaces. Refresh-rate enumeration and color profiles generally require Windows
display APIs. Audio/THX, mute and touchpad actions in Synapse are also not evidence
of EC controls. Keep these provider boundaries visible in implementation, not as
unnecessary detail in ordinary app flows.

No battery-temperature source has been found: Windows battery-class temperature
query returns unsupported. A central/battery-area fan is not a temperature sensor.
Present-device inventory still finds no 1534 devices on this laptop.

## Sources

- [Razer model-711 middleware](https://apps.razer.com/synapse/products/711/mw/main.c953ca79d6a9fe3975bd.js): model feature flags and firmware gates.
- [Razer generic device protocol module](https://apps.razer.com/synapse/products/711/mw/rzDevice25.279816c47f47f21a061c.js): getter shapes and shared command names.
- [Razer model-711 UI](https://apps.razer.com/synapse/products/711/ui/main.5fdb6531.js): display/graphics options; shared UI ranges are not hardware validation.
- [Razer Blade 18 EC update notes](https://mysupport.razer.com/app/answers/detail/a_id/20330): Battery Saver/USB-C changes.
- [Razer Battery Health Optimizer](https://mysupport.razer.com/app/answers/detail/a_id/9681): vendor battery-care behavior.

These are protocol facts and observations, not copied vendor implementations.
Downloaded vendor bundles and raw machine reports remain ignored local research.
