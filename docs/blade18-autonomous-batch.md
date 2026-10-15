# Blade 18 autonomous discovery and implementation batch

Date: 2026-10-14. Target RZ09-0529 / 1532:02c7, Core Ultra 9 275HX,
RTX 5090 Laptop GPU. This records an evidence-based pass over the identified
backlog, not a claim that every firmware command has been discovered.

## Implemented in this batch

- Independent native keyboard per-key and underside per-cell static maps. The
  native frame has 114 keyboard cells and 19 underside cells; these are logical
  cells, not a claim of 133 physical LEDs. Uniform independent RGB was previously
  confirmed visually. Individual underside positions/unused cells still need an
  observer. Both maps roundtrip through named and AC/battery lighting profiles.
- On-demand platform panel: installed firmware and panel information, Windows
  driver versions, NVIDIA power/clock telemetry, and last recorded Intel tuning
  observations. No new background poller or RGB animation stream is started.
- Windows resolution/refresh preview, with independent timed Keep/Revert recovery.
  Existing automatic profiles retain their same-resolution restriction. This does
  not switch native dual-resolution, MUX, HDR, or ICC settings.
- Experimental NVIDIA clock-offset preview: administrator + AC only, limited to
  ±100 MHz core and ±200 MHz memory, with a 15-second hold and original-offset
  recovery. The limits are deliberately small preview bounds, not validated
  stability limits. Voltage is never written. An independent helper restores on
  owner exit or its recovery deadline; an AC loss ends the preview early.
- Two additional Windows thermal-zone readings, labelled unmapped and excluded
  from fan control until their roles and responsiveness are validated.
- A bounded maintenance batch for native maps, profile capture, darkening,
  firmware-mode transition, owner-exit recovery, guard-failure fallback, and a
  short GPU underclock preview while Razer software is paused.

The optional platform/GPU provider uses the already-installed Razer native DLL;
no vendor DLL is distributed. Windows verifies its signature before loading it.
Calls run in disposable child processes with deadlines and a fixed API list.
Removing Razer's installed libraries removes these optional capabilities, not
R-Helper's existing HID controls. Independent direct GPU tuning remains a future
backend; this is not yet a wholly vendor-library-free replacement.

## New live evidence

| Probe | Observed result |
| --- | --- |
| Signed native display helper | Version 1.0.50.0; panel producer BOE; screen 2560 × 1600. |
| Firmware getter | BIOS 2.01, EC 1.06, camera 0002, VBIOS 98.3.5c.40.25. EC is now independently identified; MCU/control getter 1.7 is a different component. PD version is still unavailable. |
| Windows drivers | Intel graphics 32.0.101.6733; NVIDIA Windows package 32.0.16.1742, marketed driver version 617.42. Other installed Intel platform drivers are inventoried without serial/device IDs. |
| Windows mode preview | 2560 × 1600 / 60 Hz → 1920 × 1200 / 60 Hz → exact original mode. Native firmware panel mode was not changed. |
| Thermal counters | `\_TZ.TZ00`: 3010 tenths K (27.85 °C); `\_SB.PC00.LPCB.EC0.TZRZ`: 3182 tenths K (45.05 °C). Windows' own counter help confirms units. Names/temperatures do not identify physical sensor locations. |
| NVIDIA public GetPstates20 | Current P0 graphics/memory offsets zero; both marked editable. Driver bounds were returned, but do not prove safe offsets or permission to write. |
| Razer GPU offset getter | Independently reported core 0 / memory 0. |
| Elevated GPU preview | Core -15 MHz / memory 0 was accepted and read back, held for 15 seconds, then restored and read back as 0/0. The independent guard acknowledged parent restoration. This proves this preview and recovery path, not overclock stability or a voltage range. |
| Elevated native maintenance | All four cases passed preset capture/JSON roundtrip and expected Driver/Firmware mode checks: uniform independent RGB, simultaneous keyboard/underside cell maps, darkening, and firmware transition. Original lighting restored. Pixel positions were not visually checked. |
| Recovery fault tests | Owner exit restored the saved native lighting through the independent guard; stopping the guard exercised successful parent fallback. Both Razer services returned to Running with Automatic startup after the full batch; no R-Helper process remained. |
| Non-administrator GPU write test | NVIDIA returned `NVAPI_INVALID_USER_PRIVILEGE` (-137), documented as requiring administrator rights. No nonzero offset was established; original zero offsets were verified. The app now rejects previews before writing in a non-admin session. Zero-change guard completion and owner-exit recovery also passed without invoking a tuning setter. |
| Snap Tap getter | Fixed profile-1 `02a7` 15-byte request returned NotSupported. No Snap Tap setter enabled. This does not rule out Synapse's software mapping engine. |
| Fan-table getter candidate | Shared protocol declares an 80-byte `0d84` getter beside its table setter. One explicitly inferred header, class 0 / fan 1 / curve 0, returned NotSupported. No selector/class sweep or table writes. This does not rule out another documented table path. |
| Native driver inventory getter | Timed out and its child was terminated. Excluded from normal app probes; Windows registry metadata provides the driver inventory instead. |

Raw evidence stays under ignored `diagnostic-reports/research/`; exported platform
reports omit native helper serial/path fields. Full automated suite: 80 tests;
optimized build passes, and both GitHub build/test jobs passed for ad36a63.
Clippy completes with repository warnings. The elevated maintenance batch completed
successfully at 19:54 local time (report prefix `native-maintenance-20261014-195300`).
Individual underside cell positions remain physically unverified.

## Remaining controls and the reason they are not enabled

| Area | What is established | Next requirement |
| --- | --- | --- |
| Firmware fan curves | Shared setter encodes class/fan/curve plus temperature/16-bit target segments and a terminator. Synapse's model task starts a monitoring timer for smart curves, and no table-setter caller was found there. | A working getter/backup, exact table/fragment semantics and restoration. Existing supervised software curves remain available. |
| Wider fan limits / PWM / fan stop | Current RPM envelopes have been tested. PWM and alternate cooling getter forms returned unsupported. | Establish a model-supported wider range/path; do not turn shared enum names into controls. |
| Fn / mute / touchpad indicators | Model callers include audio-mute status `1804` and system multifunction status. | Getter or authoritative recoverable state plus native-frame overlay/physical checks. |
| Underside lighting during sleep | Model calls `0321 [0,38,enabled,duration]`. | No recoverable sleep-state getter found; duration/persistence and real sleep behavior need validation. |
| CPU undervolt / ratios / power limits | Installed Intel SDK/server route and control IDs found. Last Synapse query returned power maxima 150/175 W and window 56 seconds; compatibility logs reported Overclocking Lock and voltage controls read-only. | These observations are historical. Fresh compatibility/control enumeration, firmware lock handling and recovery are needed. No nonzero CPU voltage range is established and no CPU tuning writes were made. |
| Native panel / MUX / DDS | Relevant getters work; model features advertise display switching. | Confirm enum/SKU mappings and reboot/boot-safe restoration. Windows resolution changes are not this feature. |
| Battery Saver / USB-C power modes | EC 1.06 is confirmed; identified Battery Saver gate also requires BIOS 4.01 and model VBIOS. | BIOS is still 2.01. No BIOS flashing, lock changes or reboot was initiated by this batch. |
| Extra temperatures / physical fan roles | Two new thermal zones and existing EC A/B proxies are readable; three fan channels have measured target response. | Correlation against independent sensors/load and physical mapping. No battery sensor is identified. |
| Logical fan 4 | Advertised/readable, zero actual RPM. | Determine purpose. It is not evidence of a fourth physical fan. |
| M1–M4, remapping, Hypershift, Snap Tap masks | Model/default mappings and mapping-engine tasks exist. Some customization is host software, not a HID setting. | Establish input reports/filter integration and reliable keyboard recovery before replacing that engine. No arbitrary remapping writes. |
| Independent animations / Chroma Studio effects | Native static row 6 works. Firmware effect commands do not establish independent zone animations. | Determine zone-specific firmware effect support; any host-rendered fallback must be opt-in and battery-aware. |
| THX / audio EQ | Separate audio-service/provider path, not laptop HID. | Establish supported installed audio APIs and profile backup; no audio-driver or licensing changes. |
| Charge once completion / battery cost | Request/cancel and lighting policies already work. | Genuine charging/discharging and matched battery-energy measurements still need the physical power cycle; AC process CPU samples do not establish battery drain. |

There is no evidence here of a control that Synapse itself never implements.
Some APIs are shared with unrelated Razer devices or exist only as unused wrappers.
The present-device inventory previously found no 1534 devices on this unit.

## Repeating the prepared tests

Use a new output filename for every run. The main app helper entry points are:

```powershell
.\target\release\rhelper.exe --platform-inspect new-platform.json
.\target\release\rhelper.exe --display-mode-test new-display-test.json
.\target\release\rhelper.exe --gpu-preview-test new-gpu-test.json
.\target\release\rhelper.exe --native-maintenance-batch existing-static-auxiliary-request.json new-native-batch.json
```

GPU preview and native maintenance require administrator privileges. The auxiliary
request uses the existing strict preset/rgb/highlight schema; actual originals are
captured from the machine, not trusted from the supplied preset. The native batch
produces per-case journals, owner/guard recovery evidence and a completion/failure
report. A successful metadata roundtrip does not confirm physical pixel order.

Research-only Python tools use installed signed libraries and public NVIDIA getter
layouts: `tools/inspect-razer-native.py` and `tools/inspect-nvidia-clocks.py`.
No voltage or private NVIDIA setter is resolved by those tools.

## Primary references

- [Razer Blade 18 model guide](https://dl.razerzone.com/master-guides/RazerSynapse4/Blade18-00000711-en.pdf)
- [Razer model configuration](https://apps.razer.com/synapse/products/711/mw/main.c953ca79d6a9fe3975bd.js)
- [Razer model tasks](https://apps.razer.com/synapse/products/711/mw/6737.097a717c0f79e9e4ca4b.js)
- [Razer shared protocol](https://apps.razer.com/synapse/products/711/mw/rzDevice25.279816c47f47f21a061c.js)
- [Razer Blade2 native signatures](https://apps.razer.com/synapse/products/711/mw/rzDevice25LedMatrixBlade2.170063b45f6854daf8ea.js)
- [NVIDIA public SDK types](https://github.com/NVIDIA/nvapi/blob/main/nvapi.h)
- [NVIDIA status definitions](https://github.com/NVIDIA/nvapi/blob/main/nvapi_lite_common.h)
- [Intel undervolt protection](https://www.intel.com/content/www/us/en/support/articles/000094219/processors.html)

Protocol/API facts are independently implemented; vendor application code and
binaries are not copied into the project. Windows counter help and actual signed
helper/driver responses provide the machine-specific evidence.
