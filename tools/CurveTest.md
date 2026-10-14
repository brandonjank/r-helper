# Supervised three-fan curve test

This development test drives neutral fan selectors 1/2/3 from EC A, EC B and
max(A,B), respectively. A/B are CPU/GPU-associated controller readings; CPU die
accuracy and physical fan roles are not established. The third input is explicitly
a proxy, never a battery temperature. It is separate from the normal GUI and
does not enable persistent or startup fan control.

```powershell
cargo build --offline -p librazer --bin rhelper-curve-test --bin rhelper-diagnostics
.\tools\Test-Curves.ps1 -OutputPath diagnostic-reports\research\curve-new.json -Fault missing
```

Use AC, Balanced mode, all three fans initially automatic and clear vents. Do not
change fan controls in Synapse/R-Helper during the test. Main targets stay within
4000-4500 RPM, third within 3000-3500, in 100-RPM steps. These are a limited lab
envelope, not validated physical fan limits. Initial temperatures must be below
80/70 C. Curve cutoff values are 85 C for A, 75 C for B and 85 C for max(A,B).

The controller captures a new immutable recovery snapshot before writing. It
samples every two seconds plus query/command time, applies immediate increases
and damped decreases, skips unchanged targets and verifies target/mode readback.
Read age over three seconds, zero/missing temperatures and critical values request
firmware Auto. Actual-speed verification requires three consecutive readings
within 200 RPM of every target. The temperature policy remains an experiment.

By default, after 80 seconds it injects a missing B reading, executes Auto and
verifies all three mode getters before restoring captured settings. `-Fault stale`
instead injects a four-second age. `-Seconds` is bounded to 30-120; the fault must
precede that deadline. The total wall time includes startup and restoration.

Always launch through `Test-Curves.ps1`: this outer supervisor runs separately,
terminates its own child if it exceeds the deadline plus 20 seconds, and invokes
the recovery utility after exit, failure or termination. Recovery verifies all
three Auto modes and captured targets, and records a `.guardian.json` result.
`-KillAfterSeconds 20` tests abrupt controller termination while manual targets
are active. It terminates only the diagnostic child it created.

Reports, recovery snapshots and logs must all be new files. Keep local reports;
they contain captured machine settings. Neither the controller nor supervisor
protects against killing the supervisor itself, OS failure or a failed/unresponsive
hardware transport. This is not an unattended production watchdog. Suspend/resume,
profile conflicts, shutdown and GUI integration still require separate validation.
