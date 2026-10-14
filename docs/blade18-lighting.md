# Blade 18 lighting controls

R-Helper's Lighting card has **Read lighting controls**, a static keyboard color
picker, an underside brightness slider, and independent **Underside light** and
**Startup animation** switches. Reads and writes are on
demand, outside active curve sessions. No animation/frame worker or periodic
lighting query is started. Startup-animation readback is verified; observing the
animation at boot remains a separate check. The switches are firmware settings,
not part of the app's saved AC/battery profiles.

## Verified protocol and hardware results

Model RZ09-0529, PID 1532:02c7, control interface 2, controller getter 1.7,
BIOS 2.01; tested 2026-10-14.

| Control | Getter | Setter | Result |
| --- | --- | --- | --- |
| Startup animation | `0f98 [0]`, response byte is **disableAnimation** | `0f18 [0, disabled]` | On → Off → On readback passed. Profile 0 comes from the model middleware's actual caller. Boot appearance has not been observed. |
| Underside/vapor-chamber light | `0380 [0,38,0]` | `0300 [1,38,enabled]` | Off → On → Off readback passed. Keyboard/logo effects and brightness, plus the other switch, stayed unchanged. Physical visual confirmation remains open. |

Both switches also passed through the native app UI, with independent diagnostics
confirming the changed controller setting. Original settings were restored:
startup animation On, underside Off, keyboard brightness 176, logo brightness 255.

Underside brightness uses getter `0383 [1,38,0]` and setter
`0303 [1,38,brightness]`. A 64 → 56 → 64 test passed, preserving light enable,
startup animation, keyboard and logo settings. Its standard RGB getter
`0381 [0,38,0,0,0]` returned NotSupported. Brightness is independent of the
keyboard's 176/255 value; this does not establish independent underside RGB.

The model's Chroma inventory getter `0f80` returns these five-byte entries:

| Region | Update rate | Data type | Matrix | Interpretation |
| --- | --- | --- | --- | --- |
| 5 | 25 | 3: RGB format | 6 × 19 | Keyboard RGB matrix |
| 4 | 25 | 5: single-color PWM | 1 × 1 | Logo is single-color; no arbitrary RGB claim |

`0f81` advertises keyboard effects 0,1,2,3,4,5,7,6,8 (Off, Static,
Breathing, Spectrum, Wave, Reactive, Starlite, BasicRipple, CustomFrame).
Logo advertises only 0,1,2 (Off, Static, Breathing). Effect getter `0f82`
uses profile 1 and an 80-byte buffer, with the region in byte 1. Brightness
getter `0f84 [1,region,0]` works for keyboard and logo.

The keyboard initially reported CustomFrame with no colors in its effect reply.
Synapse's selected Quick Effect was Static, but it rendered through CustomFrame:
the name shown by Synapse is not necessarily the firmware effect mode. The
effect getter does **not** recover the current per-key pixels. **Apply color** is
therefore an explicit replacement of the current effect. The owner approved that
replacement after being informed that the old custom pixels cannot be backed up.

Static RGB uses `0f02 [1,5,1,0,1,1,R,G,B]`. The firmware accepts rate 1 but
reports rate 0; readback accepts only that exact normalization while checking
the other fields and colors. The first strict-rate comparison failure remains
in the local evidence; the color had applied. The owner visually confirmed blue
with logo and underside off. Red, green and blue primary tests subsequently
passed readback, preserved other lighting settings and restored static #2040FF
after each case. Blue held for several minutes with Synapse still open and no
R-Helper frame worker. This does not establish persistence across boot, power
changes or Synapse profile changes. Reapplying a Synapse preset can reclaim the
keyboard. Other firmware effects and custom-frame streaming remain blocked.

Region 38 has independently verified enable and brightness controls, but is absent from the
Chroma region list. This does not establish independent underside RGB colors.
The old standard RGB getters for IDs 4/5 still return NotSupported.

## Repeating or recovering the switch test

```powershell
target/release/rhelper-diagnostics.exe --probe-lighting --output lighting-inventory.json
target/release/rhelper-diagnostics.exe --validate-lighting-switches --output lighting-test.json
target/release/rhelper-diagnostics.exe --restore-lighting lighting-test.recovery.json
target/release/rhelper-diagnostics.exe --keyboard-static 2040FF --take-keyboard-control --output keyboard-change.json
target/release/rhelper-diagnostics.exe --validate-keyboard-static --output keyboard-test.json
target/release/rhelper-diagnostics.exe --restore-keyboard keyboard-test.recovery.json
```

Output files must be new. Validation persists an immutable, model-tagged recovery
file before the first setter. Each switch is restored even when its test fails;
the dedicated recovery command attempts both original switches and captured
underside brightness; legacy records without brightness leave it alone. It leaves
current keyboard/logo effects and brightness alone. Ordinary app changes capture the
current state, verify readback and other regions, and attempt rollback on failure.
The static test refuses to write unless the keyboard already has a recoverable
firmware static color. It journals recovery before testing primaries. Keyboard
recovery accepts only the exact static-color shape and leaves other regions alone.
Ordinary color changes journal metadata before writing and can roll back a prior
verified static color; custom-frame pixels are never claimed as recoverable.
App journals live in `%LOCALAPPDATA%\RHelper\lighting-changes`.
Unknown switch values, profiles, regions, other effects and Chroma frame setters
remain blocked by the model-scoped packet allowlist.

Raw local evidence remains ignored: `lighting-chroma-20261014.json`,
`lighting-switch-validation-20261014.json` and the `lighting-ui-*` reports.
RGB evidence includes `keyboard-rgb-validation-20261014.json`,
`keyboard-stable-underside-inventory-20261014.json` and
`underside-brightness-validation-20261014.json`.

The native color picker applied #8040FF with independent diagnostic confirmation,
then the static recovery command restored #2040FF. The native underside slider
applied 56/255 with independent readback, then lighting recovery restored 64/255.
All 45 automated workspace tests and the optimized Windows build passed.
With static lighting and the controls loaded, a 30.028-second tray-hidden sample
recorded 0.0 process CPU seconds and a 139,186,176-byte working set. This is a
short process CPU check, not a GPU-activity or battery-energy measurement.

## Primary protocol sources

- [Model-specific startup/VC callers](https://apps.razer.com/synapse/products/711/mw/6737.097a717c0f79e9e4ca4b.js)
- [Blade2 underside getter/setter selector](https://apps.razer.com/synapse/products/711/mw/rzDevice25LedMatrixBlade2.170063b45f6854daf8ea.js)
- [Shared packet formats](https://apps.razer.com/synapse/products/711/mw/rzDevice25.279816c47f47f21a061c.js)
- [Chroma effect/data-type enums](https://apps.razer.com/synapse/products/711/mw/3159.d10b9ea4a587313b3d6e.js)

Vendor bundles are research material only; their implementation is not copied
into the repository.
