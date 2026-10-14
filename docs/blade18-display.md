# Internal-panel refresh rates

The Display + NVIDIA section offers an on-demand refresh-rate inventory, direct
rate buttons and saved Plugged in / On battery choices. These use the existing
automatic power-profile switch and default to **Unchanged** for old settings.
There is no display poll or login-time enforcement.

The provider identifies exactly one active internal panel with QueryDisplayConfig
and its output technology, obtains its GDI source and monitor path, then enumerates
driver modes at the current resolution, depth, orientation and scan flags. Cloned
sources are refused. Rates must pass CDS_TEST. A write changes only
DM_DISPLAYFREQUENCY, uses no registry-persistence flag, and verifies panel identity,
dimensions and rate afterward. Failed writes/readbacks attempt the captured rate's
restoration. Saved profiles are rejected after panel or resolution changes, or
when a rate is no longer supported. Panel identifiers stay in local settings/reports.

On the development RZ09-0529, current mode was 2560 × 1600 at 60 Hz. The driver
advertised 60, 120 and 240 Hz at that resolution. Both 120 and 240 passed application,
two-second hold/readback and restoration to 60. No dual-resolution/MUX/HDR/color
setting or firmware mode is changed. Other resolutions/models remain untested.

Display writes are serialized across R-Helper processes. Manual and automatic
power-profile application stop active curve sessions first. If a hardware profile
fails after its display setting applies, the prior refresh rate is restored.
The UI discards its display snapshot after a power transition or profile apply;
use Read refresh rates for a fresh snapshot.

## Diagnostic

```powershell
target/release/rhelper-display.exe --output display-inventory.json
target/release/rhelper-display.exe --validate --output display-test.json
target/release/rhelper-display.exe --restore display-test.recovery.json --output display-restored.json
```

Files must be new. Validation journals the original before writes and starts a
separate 30-second recovery process. It waits for the guard's ready signal before
changing anything. The guard restores the captured rate unless validation has
verified restoration and written its completion token. Deliberately stopping the
test process left the guard running; its recovery report confirmed the original
60 Hz afterward. The normal GUI uses driver tests and failure rollback; it does
not launch this diagnostic guard or show a timed Keep/Revert confirmation.
Visual confirmation is separate from a successful driver readback.

The native 240 Hz button passed independent readback. Temporary profiles set
240 Hz on AC and 60 Hz on battery. With the app hidden, physical unplugging
applied 60 Hz plus Silent/Auto; reconnecting applied 240 Hz plus Balanced/Auto.
The prior 60 Hz setting, saved refresh choices (Unchanged) and captured hardware
preferences were restored after the test. Breathing was also visually confirmed
by the owner. All 49 automated workspace tests passed.

Local evidence: `display-inventory-20261014.json`,
`display-refresh-validation-20261014.json`, `display-guarded-final-20261014.json`,
`display-parent-exit-20261014.guard.json`.

## Primary API references

- [QueryDisplayConfig](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-querydisplayconfig)
- [Display source identity](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_source_device_name)
- [ChangeDisplaySettingsExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-changedisplaysettingsexw)

For physical power-switch tests, capture current hardware state with
`rhelper-diagnostics --capture-state --output state.json` and restore with
`--restore-state state.json` afterward. Keep saved-profile backups separately.
