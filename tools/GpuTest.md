# Bounded Blade 18 GPU temperature diagnostic

This separate development utility uses short DX12 compute batches on the one
discrete NVIDIA RTX 5090 adapter. It sends no cooling, tuning, voltage or power
limit setters. It is not a benchmark or a maximum-temperature stability test.
The normal GUI does not enable or execute it.

Build from the repository root:

```powershell
cargo build --offline -p librazer --features diagnostic-gpu --bin rhelper-gpu-test
.\target\debug\rhelper-gpu-test.exe --load-seconds 20 --output diagnostic-reports\research\gpu-new.jsonl
```

Use AC power, automatic cooling, clear vents and an otherwise steady workload.
The output must be a new file in an existing directory. The test records a
10-sample baseline, 1-60 seconds of compute submissions, then 20 recovery samples.
Each sample includes EC temperatures, all three fan speeds, modes and targets,
plus NVIDIA temperature/utilization/power. Query timing is recorded separately;
the two providers are not sampled simultaneously. Baseline/recovery samples are
one second apart plus query time. The load deadline includes telemetry overhead.

Guards require AC, all three Auto modes, nonzero EC/NVIDIA temperatures, EC A
below 90 C and EC B/NVIDIA below 78 C. A missing/invalid reading, I/O error or
guard violation exits without further compute submissions. At most one small
batch is outstanding; completion polling has a two-second deadline. NVIDIA
queries have a three-second timeout. Firmware automatic cooling remains active
on completion or error because this utility never changes it.

The dispatch cadence is a gentle workload and does not promise a utilization or
power level. Compare measured utilization and temperature movement before drawing
conclusions. Shared cooling, other applications and an earlier fan ramp can
confound fan-role inference. This does not establish battery temperature, physical
fan locations, CPU die accuracy, fan limits or software-controller fallback.

The initial default is one dispatch per batch. If this produces negligible GPU
utilization, `--dispatches-per-batch 64` increases work in each small submission.
The allowed range is 1-64; it does not alter temperature or duration limits. The
first JSONL record captures configuration; subsequent records contain telemetry.

API reference: [wgpu 25 compute pipeline](https://docs.rs/wgpu/25.0.2/wgpu/struct.ComputePipelineDescriptor.html).
