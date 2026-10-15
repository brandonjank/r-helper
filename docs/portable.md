# R-Helper portable build

Extract the whole ZIP to a folder you can write to, then open `rhelper.exe`.
Keep all companion executables beside it: the controls and thermal helpers are
required by lighting and supervised fan sessions. Moving only the main executable
can leave these features unavailable.

This build supports the existing upstream model catalog and experimental Blade
18 (2025), RZ09-0529 controls. Other Razer devices are not automatically compatible.
The Blade 18 interface groups controls into Overview, Fans, Lighting, Profiles,
Display, Updates and GPU/diagnostics. Other supported models retain their existing
interface.

The status strip records the last successful application in this session and why
it ran. Reading or saving a profile does not apply it. External software can change
hardware afterward, so use Refresh to inspect current state. Application profiles
take priority when enabled; automatic power switching and lighting policies can
also change settings. Settings and recovery journals are stored under
`%LOCALAPPDATA%\RHelper`; the portable folder does not contain your saved profiles.

Ordinary controls do not require running the whole app as administrator. Pausing
Razer services requires administrator access. Use Exit to restore Razer software;
closing the window normally hides it in the tray. Supervised curves have separate
recovery and remain experimental. The guided checks verified post-wake recovery,
but manual speeds continued through Modern Standby. Stop custom curves before
putting the laptop to Sleep. The numbered fans map to left rear, right rear and
center/battery-area airflow on the tested RB18; EC readings remain proxies, and
no battery-temperature sensor has been validated.

If an operation fails, read the footer message and open GPU + diagnostics to copy
the latest report. Check that companion executables are present and retry a read.
Do not delete recovery journals to bypass an unfinished restoration.

`manifest.json` identifies the source revision and whether the source had local
changes when packaged. `SHA256SUMS.txt` covers all other files in the folder.
Checksums detect changed files; they are not a code-signing certificate. This is a
development package, not an installed or automatically updated application.

Maintainers: build with `cargo build --workspace --release --locked`, then run
`tools/Package-Portable.ps1`. Packaging refuses to overwrite existing destinations
and fails before copying if a required file is missing. Only package binaries from
that build; the manifest records source state, not a reproducible-build guarantee.
