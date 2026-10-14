//! Time-limited lab controller, separate from unattended GUI operation.
use super::*;
use crate::fan_curve::{Curve, DampedPlanner, Plan, Point, Reading, Validation};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum CurveFault {
    Missing,
    Stale,
}

fn lab_curves() -> Result<[Curve; 3]> {
    Ok([
        Curve::new(
            vec![Point { celsius: 50.0, rpm: 4000 }, Point { celsius: 80.0, rpm: 4500 }],
            4000,
            4500,
            85.0,
        )?,
        Curve::new(
            vec![Point { celsius: 35.0, rpm: 4000 }, Point { celsius: 70.0, rpm: 4500 }],
            4000,
            4500,
            75.0,
        )?,
        Curve::new(
            vec![Point { celsius: 50.0, rpm: 3000 }, Point { celsius: 80.0, rpm: 3500 }],
            3000,
            3500,
            85.0,
        )?,
    ])
}

/// Sources mean EC A, EC B, and their maximum, not physical CPU/GPU/battery fans.
fn inputs(sample: &crate::thermal::Sample, age: Duration) -> [Option<Reading>; 3] {
    let a = sample.ec_celsius[0];
    let b = sample.ec_celsius[1];
    [a, b, a.max(b)].map(|v| (v != 0).then_some(Reading { celsius: f32::from(v), age }))
}

pub fn test_curves(
    path: &Path,
    seconds: u16,
    fault_after: u16,
    fault: CurveFault,
) -> Result<Value> {
    ensure!(
        (30..=120).contains(&seconds) && (10..seconds).contains(&fault_after),
        "Curve lab needs 30-120 seconds and an in-range fault after at least 10 seconds"
    );
    let mut file = File::options().write(true).create_new(true).open(path)?;
    let mut device = open()?;
    device.enable_fan_lab()?;
    let original = read(&device)?;
    let thermal_original = crate::thermal::read(&device)?;
    ensure!(
        ac_connected()?
            && original.performance == PerfMode::Balanced
            && thermal_original.modes.iter().all(|m| m[1] == 0),
        "Curve lab requires AC, Balanced and all selectors automatic"
    );
    ensure!(
        thermal_original.ec_celsius[0] > 0
            && thermal_original.ec_celsius[0] < 80
            && thermal_original.ec_celsius[1] > 0
            && thermal_original.ec_celsius[1] < 70,
        "Curve lab requires available temperatures and initial thermal headroom"
    );
    ensure!(
        thermal_original.target_rpm[2] <= 3500 && thermal_original.target_rpm[2] % 100 == 0,
        "Original third target is outside recovery bounds"
    );
    let mut recovery =
        File::options().write(true).create_new(true).open(path.with_extension("recovery.json"))?;
    journal(
        &mut recovery,
        &json!({"model":"RZ09-0529", "original":original, "thermal_original":thermal_original}),
    )?;
    let mut report = json!({"operation":"bounded_curve_lab", "sources":["EC A (CPU-associated proxy)","EC B (GPU-associated proxy)","max(EC A, EC B), not battery temperature"], "physical_fan_roles_verified":false, "original":original,"thermal_original":thermal_original,"duration_seconds":seconds,"fault_after_seconds":fault_after,"fault":fault,"samples":[],"target_writes":0,"fallback_verified":false,"actual_rpm_verified":false,"complete":false});
    journal(&mut file, &report)?;
    let curves = lab_curves()?;
    let mut planner = DampedPlanner::new(Duration::from_secs(5))?;
    let result = (|| -> Result<()> {
        let start = Instant::now();
        let mut previous = None;
        let mut writes = 0u32;
        let mut settled = 0;
        let mut longest_settled = 0;
        loop {
            let query_started = Instant::now();
            let sample = crate::thermal::read(&device)?;
            let mut readings = inputs(&sample, query_started.elapsed());
            ensure!(ac_connected()?, "AC disconnected during curve lab");
            ensure!(
                start.elapsed() < Duration::from_secs(seconds.into()),
                "Curve lab deadline reached before fault verification"
            );
            let injected = start.elapsed() >= Duration::from_secs(fault_after.into());
            if injected {
                match fault {
                    CurveFault::Missing => readings[1] = None,
                    CurveFault::Stale => {
                        if let Some(reading) = readings[1].as_mut() {
                            reading.age = Duration::from_secs(4);
                        }
                    }
                }
            }
            // Explicit lab policy maps known EC channels to neutral numbered
            // selectors only. This is not approval of physical fan-role mapping
            // or a production battery-temperature policy.
            let planned = planner.update(
                [&curves[0], &curves[1], &curves[2]],
                readings,
                Duration::from_secs(3),
                Validation { sensors_mapped: true, independent_actuators: true },
                start.elapsed(),
            );
            match planned {
                Plan::Automatic(reason) => {
                    c::set_fan_mode(&device, FanMode::Auto)?;
                    thread::sleep(Duration::from_millis(250));
                    let after = crate::thermal::read(&device)?;
                    ensure!(
                        after.modes.iter().all(|m| m[1] == 0),
                        "Automatic fallback readback failed"
                    );
                    report["fallback"] = json!({"reason":reason,"injected":injected,"after":after,"elapsed_seconds":start.elapsed().as_secs_f64()});
                    report["fallback_verified"] = json!(true);
                    journal(&mut file, &report)?;
                    let expected = match fault {
                        CurveFault::Missing => "Missing temperature",
                        CurveFault::Stale => "Stale temperature",
                    };
                    ensure!(
                        injected && reason == expected,
                        "Unexpected sensor/thermal fallback; inspect captured report"
                    );
                    break;
                }
                Plan::Targets(targets) => {
                    let expected_mode = if previous.is_none() { 0 } else { 1 };
                    ensure!(
                        sample.modes.iter().all(|m| m[1] == expected_mode),
                        "Fan mode changed outside curve controller"
                    );
                    if let Some(previous) = previous {
                        ensure!(
                            sample.target_rpm == previous,
                            "Fan targets changed outside curve controller"
                        );
                    }
                    if previous.map(|p: [u16; 3]| p[..2] != targets[..2]).unwrap_or(true) {
                        apply(&device, &Change::ManualFanZones([targets[0], targets[1]]))?;
                        writes += 2;
                    }
                    if previous.map(|p: [u16; 3]| p[2] != targets[2]).unwrap_or(true) {
                        c::send_command(&device, 0x0d01, &[1, 3, (targets[2] / 100) as u8])?;
                        writes += 1;
                    }
                    let after = crate::thermal::read(&device)?;
                    ensure!(
                        after.target_rpm == targets && after.modes.iter().all(|m| m[1] == 1),
                        "Curve target/mode readback mismatch"
                    );
                    if after
                        .actual_rpm
                        .iter()
                        .zip(targets)
                        .all(|(rpm, target)| rpm.abs_diff(target) <= 200)
                    {
                        settled += 1;
                    } else {
                        settled = 0;
                    }
                    longest_settled = longest_settled.max(settled);
                    report["actual_rpm_verified"] = json!(longest_settled >= 3);
                    report["target_writes"] = json!(writes);
                    let row = json!({"elapsed_seconds":start.elapsed().as_secs_f64(),"targets":targets,"sample":after});
                    eprintln!("Curve sample: {row}");
                    report["samples"].as_array_mut().unwrap().push(row);
                    journal(&mut file, &report)?;
                    previous = Some(targets);
                }
            }
            thread::sleep(Duration::from_secs(2));
        }
        ensure!(
            longest_settled >= 3,
            "Curve targets did not show three consecutive actual-RPM matches"
        );
        Ok(())
    })();
    // Every operation below is attempted even after a controller or journal error.
    let auto = c::set_fan_mode(&device, FanMode::Auto);
    let third =
        c::send_command(&device, 0x0d01, &[1, 3, (thermal_original.target_rpm[2] / 100) as u8]);
    let restored = restore(&device, &original);
    let thermal_after = crate::thermal::read(&device);
    report["error"] = json!(result.as_ref().err().map(|e| format!("{e:#}")));
    report["auto_restore_error"] = json!(auto.as_ref().err().map(|e| format!("{e:#}")));
    report["third_restore_error"] = json!(third.as_ref().err().map(|e| format!("{e:#}")));
    report["restore_error"] = json!(restored.as_ref().err().map(|e| format!("{e:#}")));
    report["thermal_restored"] = serde_json::to_value(thermal_after.as_ref().ok())?;
    report["complete"] = json!(true);
    journal(&mut file, &report)?;
    auto?;
    third?;
    restored?;
    let after = thermal_after?;
    ensure!(
        after.modes.iter().all(|m| m[1] == 0) && after.target_rpm == thermal_original.target_rpm,
        "Curve lab restoration mismatch"
    );
    result?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_duration_is_rejected_before_opening_output_or_hardware() {
        for (seconds, fault_after) in [(29, 10), (121, 20), (30, 30), (60, 9)] {
            assert!(test_curves(
                Path::new("not-created-curve-test.json"),
                seconds,
                fault_after,
                CurveFault::Missing
            )
            .is_err());
        }
    }
    #[test]
    fn lab_sources_keep_zero_unavailable_and_auxiliary_explicitly_a_proxy() {
        let sample = crate::thermal::Sample {
            ec_celsius: [60, 45],
            actual_rpm: [0; 3],
            target_rpm: [0; 3],
            modes: [[0; 2]; 3],
        };
        let values = inputs(&sample, Duration::from_secs(1));
        assert_eq!(values.map(|r| r.unwrap().celsius), [60.0, 45.0, 60.0]);
        let unavailable = crate::thermal::Sample { ec_celsius: [0, 45], ..sample };
        assert!(inputs(&unavailable, Duration::ZERO)[0].is_none());
    }
}
