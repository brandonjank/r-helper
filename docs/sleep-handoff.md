# Automatic cooling before sleep

The curve supervisor registers native suspend/resume and console display-state
callbacks before spawning its worker. If either registration fails, curves do not
start. Suspend and display-off both latch the same stop request. Display-on, dim,
resume and malformed notifications do not restart curves or change settings.
Stopping when all displays turn off is deliberately conservative for Modern
Standby; a new curve session requires an explicit user action.

The callback does not open HID or race the controller for device ownership. It
signals the existing worker, which restores its captured targets and Auto modes,
then writes a timestamped stopped-state acknowledgement. The callback accepts
only a complete thermal snapshot with the exact original modes and targets,
explicitly successful recovery, and a fresh acknowledgement within 1500 ms of the
first stop request. Display-off and suspend share that request, so a subsequent
suspend notification can recognize cooling already restored by display-off.

Windows gives suspend handlers a short window before suspension. The 1500 ms
acknowledgement budget leaves margin within the documented two seconds. File
reads are capped at 16 KiB; errors or a missing/late acknowledgement are recorded
as unverified, not a successful pre-sleep handoff. The independent heartbeat and
post-wake recovery remain in place. The feature does not block Windows sleep or
hold an execution-state request to avoid it.

The two native callback allocations remain valid until the short-lived supervisor
exits, including failed or late unregistration. They do not accumulate in the GUI
process. The supervisor waits briefly for in-flight callback evidence before
publishing its final record. Reused session directories are rejected without
taking ownership or restoring an older session's record.

Evidence is saved as `sleep-display-off.json` and/or `sleep-suspend.json`, with the
received event time, shared stop-request time, acknowledged state time and final
result. `sleep-test-suspend.json` is a synthetic test and cannot substitute for a
real sleep observation. The live SleepResume harness requires a timely native
acknowledgement plus owner-confirmed sleep, followed by independent Auto/target
readback. A heartbeat recovery after wake alone is not sufficient.

`Test-AppCurves.ps1 -ExecutableDirectory PATH` can test the actual portable package
without replacing a running validation-hold executable. Its default remains the
local release build. Offline coverage includes stale/late/missing/corrupt records,
incomplete restoration, native event routing, ignored events, shared acknowledgement
and rejection of reused sessions.

Live validation of this fix is pending; see `blade18-validation-batch.md` for the
earlier observed Modern Standby limitation and preserved unsuccessful attempts.

Sources: [suspend notifications](https://learn.microsoft.com/en-us/windows/win32/api/powerbase/nf-powerbase-powerregistersuspendresumenotification),
[sleep timing](https://learn.microsoft.com/en-us/windows/win32/power/system-sleep-criteria),
[Desktop Activity Moderator](https://learn.microsoft.com/en-us/windows/win32/w8cookbook/desktop-activity-moderator).
