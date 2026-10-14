# Blade 18 lighting controls

R-Helper's Lighting card has **Read lighting controls**, followed by independent
**Underside light** and **Startup animation** switches. Reads and writes are on
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

The keyboard currently reports CustomFrame with no colors in its effect reply.
Synapse's selected Quick Effect is Static, but it renders through CustomFrame:
the name shown by Synapse is not necessarily the firmware effect mode. The
effect getter does **not** recover the current per-key pixels. Consequently this
build does not enable color/effect writes or pretend that replaying effect 8
would restore the old appearance. Next work is to establish a recoverable
firmware-effect baseline and an explicit handoff from Synapse/Chroma, then test
Static/Breathing/Spectrum without a host frame loop.

Region 38 has an independently verified enable switch, but is absent from the
Chroma region list. This does not establish independent underside RGB colors.
The old standard RGB getters for IDs 4/5 still return NotSupported.

## Repeating or recovering the switch test

```powershell
target/release/rhelper-diagnostics.exe --probe-lighting --output lighting-inventory.json
target/release/rhelper-diagnostics.exe --validate-lighting-switches --output lighting-test.json
target/release/rhelper-diagnostics.exe --restore-lighting lighting-test.recovery.json
```

Output files must be new. Validation persists an immutable, model-tagged recovery
file before the first setter. Each switch is restored even when its test fails;
the dedicated recovery command attempts both original switches and leaves current
keyboard/logo effects and brightness alone. Ordinary app changes capture the
current state, verify readback and other regions, and attempt rollback on failure.
Unknown switch values, profiles, regions and all Chroma effect/frame setters remain
blocked by the model-scoped packet allowlist.

Raw local evidence remains ignored: `lighting-chroma-20261014.json`,
`lighting-switch-validation-20261014.json` and the `lighting-ui-*` reports.

## Primary protocol sources

- [Model-specific startup/VC callers](https://apps.razer.com/synapse/products/711/mw/6737.097a717c0f79e9e4ca4b.js)
- [Blade2 underside getter/setter selector](https://apps.razer.com/synapse/products/711/mw/rzDevice25LedMatrixBlade2.170063b45f6854daf8ea.js)
- [Shared packet formats](https://apps.razer.com/synapse/products/711/mw/rzDevice25.279816c47f47f21a061c.js)
- [Chroma effect/data-type enums](https://apps.razer.com/synapse/products/711/mw/3159.d10b9ea4a587313b3d6e.js)

Vendor bundles are research material only; their implementation is not copied
into the repository.
