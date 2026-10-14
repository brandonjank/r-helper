//! Model-scoped controls and recoverable, journaled hardware validation.
use crate::{command as c, device::Device, diagnostics::open_blade18_diagnostic, types::*};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    path::Path,
    thread,
    time::Duration,
};

pub const MODES: &[PerfMode] =
    &[PerfMode::Balanced, PerfMode::Silent, PerfMode::Performance, PerfMode::Custom];
pub const FAN_MIN: u16 = 4000;
pub const FAN_MAX: u16 = 4500;
pub const MANUAL_FANS_VERIFIED: bool = true;

pub fn ac_connected() -> Result<bool> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut power = SYSTEM_POWER_STATUS::default();
        unsafe {
            GetSystemPowerStatus(&mut power)?;
        }
        match power.ACLineStatus {
            0 => Ok(false),
            1 => Ok(true),
            _ => anyhow::bail!("Power source is unknown"),
        }
    }
    #[cfg(not(windows))]
    anyhow::bail!("Windows is required")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub brightness: u8,
    pub logo: LogoMode,
    pub battery_care: BatteryCare,
    pub performance: PerfMode,
    pub fan_mode: FanMode,
    pub fan_setpoints: [u16; 2],
    pub fan_actual: [u16; 2],
    pub cpu: Option<CpuBoost>,
    pub gpu: Option<GpuBoost>,
}

fn open() -> Result<Device> {
    let mut device = open_blade18_diagnostic()?;
    device.enable_blade18_controls()?;
    Ok(device)
}

fn read(device: &Device) -> Result<State> {
    retry_read(|| read_once(device))
}

fn retry_read<T>(mut operation: impl FnMut() -> Result<T>) -> Result<T> {
    for attempt in 0..3 {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt == 2 => {
                return Err(error.context("State query failed after three attempts"))
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
    unreachable!()
}

fn read_once(device: &Device) -> Result<State> {
    let (performance, fan_mode) = c::get_perf_mode(device)?;
    Ok(State {
        brightness: c::get_keyboard_brightness(device)?,
        logo: c::get_logo_mode(device)?,
        battery_care: c::get_battery_care(device)?,
        performance,
        fan_mode,
        fan_setpoints: [
            c::get_fan_rpm(device, FanZone::Zone1)?,
            c::get_fan_rpm(device, FanZone::Zone2)?,
        ],
        fan_actual: [
            c::get_fan_actual_rpm(device, FanZone::Zone1)?,
            c::get_fan_actual_rpm(device, FanZone::Zone2)?,
        ],
        cpu: c::get_cpu_boost(device).ok(),
        gpu: c::get_gpu_boost(device).ok(),
    })
}

pub fn status() -> Result<State> {
    read(&open()?)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Change {
    Brightness(u8),
    Logo(LogoMode),
    BatteryCare(BatteryCare),
    Performance(PerfMode),
    Boosts(CpuBoost, GpuBoost),
    ManualFans(u16),
    ManualFanZones([u16; 2]),
    AutoFans,
}

fn validate_change(change: &Change) -> Result<()> {
    match change {
        Change::Performance(mode) => ensure!(MODES.contains(mode), "Unvalidated performance mode"),
        Change::Boosts(cpu, _) => {
            ensure!(*cpu != CpuBoost::Undervolt, "Undervolt is not supported")
        }
        Change::ManualFans(rpm) => {
            ensure!((FAN_MIN..=FAN_MAX).contains(rpm) && rpm % 100 == 0, "RPM outside tested range")
        }
        Change::ManualFanZones(targets) => {
            for rpm in targets {
                ensure!(
                    (FAN_MIN..=FAN_MAX).contains(rpm) && rpm % 100 == 0,
                    "RPM outside tested range"
                );
            }
        }
        _ => {}
    }
    Ok(())
}

fn apply(device: &Device, change: &Change) -> Result<()> {
    validate_change(change)?;
    match *change {
        Change::Brightness(value) => c::set_keyboard_brightness(device, value)?,
        Change::Logo(value) => c::set_logo_mode(device, value)?,
        Change::BatteryCare(value) => c::set_battery_care(device, value)?,
        Change::Performance(mode) => {
            ensure!(MODES.contains(&mode), "Unvalidated performance mode");
            if matches!(mode, PerfMode::Performance | PerfMode::Custom) {
                ensure!(ac_connected()?, "This mode requires AC power");
            }
            c::set_perf_mode(device, mode)?;
        }
        Change::Boosts(cpu, gpu) => {
            ensure!(cpu != CpuBoost::Undervolt, "Undervolt is not supported");
            c::set_cpu_boost(device, cpu)?;
            c::set_gpu_boost(device, gpu)?;
        }
        Change::ManualFans(_) | Change::ManualFanZones(_) => {
            let targets = match *change {
                Change::ManualFans(rpm) => [rpm; 2],
                Change::ManualFanZones(targets) => targets,
                _ => unreachable!(),
            };
            let (performance, fan_mode) = c::get_perf_mode(device)?;
            ensure!(ac_connected()?, "Manual fan targets currently require AC power");
            ensure!(performance == PerfMode::Balanced, "Manual fans require Balanced mode");
            for (command, args) in manual_fan_commands(targets, fan_mode == FanMode::Auto) {
                c::send_command(device, command, &args)?;
            }
            if fan_mode == FanMode::Auto {
                // The mode transition can acknowledge before the effective RPM
                // settles to its manual default. Reassert both targets afterward.
                thread::sleep(Duration::from_secs(1));
                for (command, args) in manual_fan_commands(targets, false) {
                    c::send_command(device, command, &args)?;
                }
            }
        }
        Change::AutoFans => c::set_fan_mode(device, FanMode::Auto)?,
    }
    thread::sleep(Duration::from_millis(250));
    let state = read(device)?;
    let matches = match *change {
        Change::Brightness(v) => state.brightness == v,
        Change::Logo(v) => state.logo == v,
        Change::BatteryCare(v) => state.battery_care == v,
        Change::Performance(v) => state.performance == v && state.fan_mode == FanMode::Auto,
        Change::Boosts(cpu, gpu) => state.cpu == Some(cpu) && state.gpu == Some(gpu),
        Change::ManualFans(v) => state.fan_mode == FanMode::Manual && state.fan_setpoints == [v, v],
        Change::ManualFanZones(v) => state.fan_mode == FanMode::Manual && state.fan_setpoints == v,
        Change::AutoFans => state.fan_mode == FanMode::Auto,
    };
    ensure!(matches, "Readback mismatch after {change:?}: {state:?}");
    Ok(())
}

fn manual_fan_commands(targets: [u16; 2], enter_manual: bool) -> Vec<(u16, Vec<u8>)> {
    let mut commands = Vec::new();
    // Preserve the source-backed read/mode/target sequence per selector. A mode
    // write after a selector's target resets that actuator toward 3000 RPM.
    for (zone, rpm) in (1..=2).zip(targets) {
        commands.push((0x0d82, vec![0, zone, 0, 0]));
        if enter_manual {
            commands.push((0x0d02, vec![0, zone, 0, 1]));
        }
        commands.push((0x0d01, vec![1, zone, (rpm / 100) as u8]));
    }
    commands
}

fn restore(device: &Device, original: &State) -> Result<State> {
    let mut errors = Vec::new();
    // Attempt every restoration even when one category fails.
    macro_rules! attempt {
        ($e:expr) => {
            if let Err(error) = $e {
                errors.push(format!("{error:#}"));
            }
        };
    }
    attempt!(c::set_keyboard_brightness(device, original.brightness));
    attempt!(c::set_logo_mode(device, original.logo));
    attempt!(c::set_battery_care(device, original.battery_care));
    if let (Some(cpu), Some(gpu)) = (original.cpu, original.gpu) {
        if c::get_cpu_boost(device).ok() != Some(cpu) || c::get_gpu_boost(device).ok() != Some(gpu)
        {
            attempt!(c::set_perf_mode(device, PerfMode::Custom));
            attempt!(c::set_cpu_boost(device, cpu));
            attempt!(c::set_gpu_boost(device, gpu));
        }
    }
    attempt!(c::set_perf_mode(device, original.performance));
    if original.performance == PerfMode::Balanced && original.fan_mode == FanMode::Manual {
        for (command, args) in manual_fan_commands(original.fan_setpoints, true) {
            attempt!(c::send_command(device, command, &args));
        }
        thread::sleep(Duration::from_secs(1));
        for (command, args) in manual_fan_commands(original.fan_setpoints, false) {
            attempt!(c::send_command(device, command, &args));
        }
    } else {
        for (zone, rpm) in [1u8, 2].into_iter().zip(original.fan_setpoints) {
            attempt!(c::send_command(device, 0x0d01, &[0, zone, (rpm / 100) as u8]));
        }
        attempt!(c::set_fan_mode(device, original.fan_mode));
    }
    thread::sleep(Duration::from_millis(250));
    let state = read(device)?;
    ensure!(errors.is_empty(), "Restoration errors: {errors:?}; current state: {state:?}");
    ensure!(
        state.brightness == original.brightness
            && state.logo == original.logo
            && state.battery_care == original.battery_care
            && state.performance == original.performance
            && state.fan_mode == original.fan_mode
            && (original.cpu.is_none() || state.cpu == original.cpu)
            && (original.gpu.is_none() || state.gpu == original.gpu)
            && state.fan_setpoints == original.fan_setpoints,
        "Restoration readback mismatch: {state:?}"
    );
    Ok(state)
}

fn journal(file: &mut File, report: &Value) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    serde_json::to_writer_pretty(&mut *file, report)?;
    file.write_all(b"\n")?;
    let length = file.stream_position()?;
    file.set_len(length)?;
    file.sync_all()?;
    Ok(())
}

fn recovery_record(path: &Path, original: &State) -> Result<()> {
    // Never rewrite the recovery snapshot while the mutable report grows.
    let recovery = path.with_extension("recovery.json");
    let mut file = File::options().write(true).create_new(true).open(&recovery)?;
    journal(&mut file, &json!({"model": "RZ09-0529", "original": original}))
}

pub fn restore_record(path: &Path) -> Result<State> {
    let record: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(record["model"] == "RZ09-0529", "Recovery record is not for Blade 18 (2025)");
    let original: State = serde_json::from_value(record["original"].clone())?;
    ensure!(original.cpu != Some(CpuBoost::Undervolt), "Undervolt recovery is not supported");
    ensure!(
        original.fan_setpoints.iter().all(|rpm| *rpm <= 5500 && rpm % 100 == 0),
        "Invalid fan setpoints in recovery record"
    );
    let mut device = open()?;
    if !record["thermal_original"].is_null() {
        let thermal: crate::thermal::Sample =
            serde_json::from_value(record["thermal_original"].clone())?;
        ensure!(
            thermal.modes.iter().all(|mode| mode[1] == 0)
                && thermal.target_rpm[2] <= 3500
                && thermal.target_rpm[2] % 100 == 0,
            "Unvalidated thermal recovery snapshot"
        );
        device.enable_fan_lab()?;
        if record["standard_framing"] == true {
            device.use_standard_framing()?;
        }
        let third = c::send_command(&device, 0x0d01, &[1, 3, (thermal.target_rpm[2] / 100) as u8]);
        let restored = restore(&device, &original);
        third?;
        let after = crate::thermal::read(&device)?;
        ensure!(
            after.target_rpm[2] == thermal.target_rpm[2]
                && after.modes.iter().all(|mode| mode[1] == 0),
            "Thermal recovery readback mismatch"
        );
        return restored;
    }
    restore(&device, &original)
}

pub fn change(change: Change) -> Result<State> {
    validate_change(&change)?;
    ensure!(
        !matches!(change, Change::ManualFans(_) | Change::ManualFanZones(_))
            || MANUAL_FANS_VERIFIED,
        "Manual fan control is pending actual RPM validation; use firmware automatic cooling"
    );
    let device = open()?;
    let original = read(&device)?;
    if let Err(error) = apply(&device, &change) {
        let restored = restore(&device, &original);
        anyhow::bail!("{error:#}; rollback: {restored:?}");
    }
    read(&device)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub brightness: u8,
    pub logo: LogoMode,
    pub battery_care: BatteryCare,
    pub performance: PerfMode,
    pub cpu: Option<CpuBoost>,
    pub gpu: Option<GpuBoost>,
}

impl Profile {
    pub fn capture(state: &State) -> Self {
        Self {
            brightness: state.brightness,
            logo: state.logo,
            battery_care: state.battery_care,
            performance: state.performance,
            cpu: state.cpu,
            gpu: state.gpu,
        }
    }
    pub fn validate(&self, battery: bool) -> Result<()> {
        ensure!(MODES.contains(&self.performance), "Unsupported profile performance mode");
        ensure!(self.cpu != Some(CpuBoost::Undervolt), "Undervolt is not supported");
        if battery {
            ensure!(
                matches!(self.performance, PerfMode::Balanced | PerfMode::Silent),
                "Battery profiles support Balanced or Silent only"
            );
        }
        if self.performance == PerfMode::Custom {
            ensure!(
                self.cpu.is_some() && self.gpu.is_some(),
                "Custom profiles require CPU and GPU levels"
            );
        }
        Ok(())
    }
}

pub fn apply_profile(profile: &Profile) -> Result<State> {
    profile.validate(!ac_connected()?)?;
    let device = open()?;
    let original = read(&device)?;
    let result = (|| -> Result<State> {
        apply(&device, &Change::Brightness(profile.brightness))?;
        apply(&device, &Change::Logo(profile.logo))?;
        apply(&device, &Change::BatteryCare(profile.battery_care))?;
        apply(&device, &Change::Performance(profile.performance))?;
        if profile.performance == PerfMode::Custom {
            apply(&device, &Change::Boosts(profile.cpu.unwrap(), profile.gpu.unwrap()))?;
        }
        read(&device)
    })();
    match result {
        Ok(state) => Ok(state),
        Err(error) => {
            let rollback = restore(&device, &original);
            anyhow::bail!("Profile failed: {error:#}; rollback: {rollback:?}");
        }
    }
}

pub fn validate_fans(path: &Path) -> Result<Value> {
    validate_fan_targets(path, &[([4000, 4000], 18), ([4500, 4500], 12)], false)
}

/// Test unequal zone targets in both directions, without assuming CPU/GPU mapping.
pub fn validate_fan_zones(path: &Path) -> Result<Value> {
    validate_fan_targets(path, &[([4000, 4500], 18), ([4500, 4000], 18)], true)
}

/// Briefly change one target for owner-assisted mapping. Manual mode is shared
/// by firmware; unchanged target registers remain captured and are restored.
#[derive(Clone, Copy, Debug)]
pub struct FanLabOptions {
    pub zone: u8,
    pub enabled_write: bool,
    pub alternate_mode_byte: bool,
    pub standard_framing: bool,
    pub latch_after_target: bool,
    pub paired_initialization: bool,
    pub isolated_comparison: bool,
    pub observe_seconds: u16,
}
pub fn identify_main_fan(path: &Path, options: FanLabOptions) -> Result<Value> {
    let FanLabOptions {
        zone,
        enabled_write,
        alternate_mode_byte,
        standard_framing,
        latch_after_target,
        paired_initialization,
        isolated_comparison,
        observe_seconds,
    } = options;
    ensure!((20..=120).contains(&observe_seconds), "Observation must be 20 to 120 seconds");
    ensure!(matches!(zone, 1..=3), "Expected fan selector 1/2/3");
    ensure!(!isolated_comparison || (paired_initialization && enabled_write && alternate_mode_byte && !standard_framing && !latch_after_target), "Isolated mapping requires the validated paired/prefix sequence without framing or latching experiments");
    ensure!(zone != 3 || enabled_write, "Third-fan test requires the explicit enable-byte variant");
    let mut file = File::options().write(true).create_new(true).open(path)?;
    let mut device = open()?;
    device.enable_fan_lab()?;
    if standard_framing {
        device.use_standard_framing()?;
    }
    let original = read(&device)?;
    ensure!(
        original.fan_mode == FanMode::Auto,
        "Return to automatic cooling before identification"
    );
    ensure!(ac_connected()?, "Identification requires AC power");
    let thermal_original = crate::thermal::read(&device)?;
    ensure!(
        thermal_original.modes.iter().all(|mode| mode[1] == 0),
        "All three selectors must report automatic cooling before identification"
    );
    ensure!(
        thermal_original.target_rpm[2] <= 3500 && thermal_original.target_rpm[2] % 100 == 0,
        "Third-fan original target cannot be restored with the bounded lab access"
    );
    let mut recovery =
        File::options().write(true).create_new(true).open(path.with_extension("recovery.json"))?;
    journal(
        &mut recovery,
        &json!({"model": "RZ09-0529", "original": original, "thermal_original": thermal_original, "standard_framing":standard_framing}),
    )?;
    let target = if zone == 3 { 3500 } else { 4500 };
    let write_enable = u8::from(enabled_write);
    let mode_byte = if alternate_mode_byte { 0 } else { 1 };
    let mut report = json!({"original": original, "thermal_original": thermal_original, "zone": zone, "target": target, "write_enable": write_enable, "mode_byte":mode_byte, "standard_framing":standard_framing, "latch_after_target":latch_after_target, "paired_initialization":paired_initialization, "isolated_comparison":isolated_comparison, "observe_seconds":observe_seconds, "warmup_samples":[], "samples": [], "complete": false});
    journal(&mut file, &report)?;
    let result = (|| -> Result<()> {
        c::set_perf_mode(&device, PerfMode::Balanced)?;
        let baseline = if isolated_comparison { 30 } else { 40 };
        if paired_initialization {
            for selector in 1..=2 {
                c::send_command(&device, 0x0d82, &[0, selector, 0, 0])?;
                c::send_command(&device, 0x0d02, &[mode_byte, selector, 0, 1])?;
                c::send_command(&device, 0x0d01, &[write_enable, selector, baseline])?;
            }
            thread::sleep(Duration::from_secs(1));
            for selector in 1..=2 {
                c::send_command(&device, 0x0d82, &[0, selector, 0, 0])?;
                c::send_command(&device, 0x0d01, &[write_enable, selector, baseline])?;
            }
        }
        if !paired_initialization {
            c::send_command(&device, 0x0d02, &[mode_byte, 1, 0, 1])?;
        }
        if isolated_comparison {
            c::send_command(&device, 0x0d01, &[write_enable, 3, baseline])?;
            eprintln!("WARMUP: all three targets 3000 RPM; selected-zone change follows only after convergence.");
            let mut settled = 0;
            for _ in 0..30 {
                thread::sleep(Duration::from_secs(2));
                let sample = crate::thermal::read(&device)?;
                ensure!(
                    ac_connected()?
                        && sample.ec_celsius[0] > 0
                        && sample.ec_celsius[0] < 90
                        && sample.ec_celsius[1] < 80,
                    "Warmup power/temperature guard reached"
                );
                ensure!(
                    sample.modes.iter().all(|mode| mode[1] == 1),
                    "Manual mode changed during warmup"
                );
                if sample.actual_rpm.iter().all(|rpm| rpm.abs_diff(3000) <= 200) {
                    settled += 1;
                } else {
                    settled = 0;
                }
                report["warmup_samples"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::to_value(sample)?);
                journal(&mut file, &report)?;
            }
            ensure!(settled >= 3, "Baseline did not settle; skipping physical comparison");
        }
        c::send_command(&device, 0x0d01, &[write_enable, zone, (target / 100) as u8])?;
        eprintln!("OBSERVE NOW: only zone {zone} target changed to {target} RPM.");
        if latch_after_target {
            c::send_command(&device, 0x0d02, &[mode_byte, 1, 0, 1])?;
        }
        let mut settled = 0;
        for _ in 0..observe_seconds.div_ceil(2) {
            thread::sleep(Duration::from_secs(2));
            let sample = crate::thermal::read(&device)?;
            ensure!(
                ac_connected()?
                    && sample.ec_celsius[0] > 0
                    && sample.ec_celsius[0] < 90
                    && sample.ec_celsius[1] < 80,
                "Identification thermal guard reached; restoring automatic cooling"
            );
            if isolated_comparison {
                ensure!(
                    sample.modes.iter().all(|mode| mode[1] == 1),
                    "Manual mode changed during observation"
                );
                let mut expected = [3000; 3];
                expected[usize::from(zone - 1)] = target;
                ensure!(sample.target_rpm == expected, "Fan targets changed during observation");
                if sample
                    .actual_rpm
                    .iter()
                    .zip(expected)
                    .all(|(rpm, target)| rpm.abs_diff(target) <= 200)
                {
                    settled += 1;
                } else {
                    settled = 0;
                }
            }
            let sample = serde_json::to_value(sample)?;
            eprintln!("Identification zone {zone}: {sample}");
            report["samples"].as_array_mut().unwrap().push(sample);
            journal(&mut file, &report)?;
        }
        if isolated_comparison {
            report["actual_rpm_verified"] = json!(settled >= 3);
            ensure!(settled >= 3, "Isolated actual speeds did not settle to the requested targets");
        }
        Ok(())
    })();
    let third_restore = c::send_command(
        &device,
        0x0d01,
        &[write_enable, 3, (thermal_original.target_rpm[2] / 100) as u8],
    );
    let restored = restore(&device, &original);
    let third_after = crate::thermal::read(&device);
    let passed = result.is_ok();
    report["error"] = json!(result.err().map(|e| format!("{e:#}")));
    report["restore_error"] = json!(restored.as_ref().err().map(|e| format!("{e:#}")));
    report["restored"] = serde_json::to_value(restored.as_ref().ok())?;
    report["third_restore_error"] =
        json!(third_restore.as_ref().err().map(|error| format!("{error:#}")));
    report["thermal_restored"] = serde_json::to_value(third_after.as_ref().ok())?;
    report["complete"] = json!(true);
    journal(&mut file, &report)?;
    restored?;
    third_restore?;
    let third_after = third_after?;
    ensure!(
        third_after.target_rpm[2] == thermal_original.target_rpm[2]
            && third_after.modes.iter().all(|mode| mode[1] == 0),
        "Third-fan target or automatic-mode restoration readback mismatch"
    );
    ensure!(passed, "Identification stopped; captured settings restored; inspect report");
    Ok(report)
}

fn validate_fan_targets(
    path: &Path,
    cases: &[([u16; 2], usize)],
    full_thermal: bool,
) -> Result<Value> {
    let mut file = File::options().write(true).create_new(true).open(path)?;
    let device = open()?;
    let original = read(&device)?;
    ensure!(
        original.fan_mode == FanMode::Auto && ac_connected()?,
        "Fan validation requires automatic cooling and AC power"
    );
    recovery_record(path, &original)?;
    let mut report = json!({"original": original, "samples": [], "thermal_samples": [], "case_results": [], "actual_rpm_verified":false, "complete": false});
    journal(&mut file, &report)?;
    let result = (|| -> Result<()> {
        apply(&device, &Change::Performance(PerfMode::Balanced))?;
        for &(targets, count) in cases {
            apply(&device, &Change::ManualFanZones(targets))?;
            let mut settled_samples = 0;
            for _ in 0..count {
                thread::sleep(Duration::from_secs(5));
                let observed = read(&device)?;
                if observed
                    .fan_actual
                    .iter()
                    .zip(targets)
                    .all(|(actual, target)| actual.abs_diff(target) <= 200)
                {
                    settled_samples += 1;
                } else {
                    settled_samples = 0;
                }
                if full_thermal {
                    let thermal = crate::thermal::read(&device)?;
                    ensure!(
                        thermal.ec_celsius[0] < 90 && thermal.ec_celsius[1] < 80,
                        "Thermal guard reached; restoring captured settings"
                    );
                    report["thermal_samples"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::to_value(thermal)?);
                }
                eprintln!(
                    "Fan request {targets:?}, setpoints {:?}, actual {:?}",
                    observed.fan_setpoints, observed.fan_actual
                );
                report["samples"].as_array_mut().unwrap().push(serde_json::to_value(observed)?);
                journal(&mut file, &report)?;
            }
            report["case_results"].as_array_mut().unwrap().push(json!({"targets":targets, "consecutive_settled_samples":settled_samples, "passed":settled_samples>=3}));
            journal(&mut file, &report)?;
            ensure!(settled_samples >= 3, "Actual fan speeds did not settle within 200 RPM for three consecutive samples; restoring captured settings");
        }
        report["actual_rpm_verified"] = json!(true);
        Ok(())
    })();
    let restored = restore(&device, &original);
    let passed = result.is_ok();
    report["error"] = json!(result.err().map(|e| format!("{e:#}")));
    report["restore_error"] = json!(restored.as_ref().err().map(|e| format!("{e:#}")));
    report["restored"] = serde_json::to_value(restored.as_ref().ok())?;
    report["complete"] = json!(true);
    journal(&mut file, &report)?;
    restored?;
    ensure!(passed, "Fan validation failed; settings restored; inspect saved report");
    Ok(report)
}

/// Run one batch, sync recovery state before writes, and restore after every case.
pub fn validate_batch(path: &Path) -> Result<Value> {
    let mut file = File::options().write(true).create_new(true).open(path)?;
    let device = open()?;
    let original = read(&device)?;
    ensure!(
        original.cpu.is_some() && original.gpu.is_some(),
        "Cannot capture custom boost settings; no writes attempted"
    );
    ensure!(
        original.cpu != Some(CpuBoost::Undervolt),
        "Cannot safely restore an undervolt preset; no writes attempted"
    );
    recovery_record(path, &original)?;
    let mut report = json!({"original": original, "cases": [], "complete": false});
    journal(&mut file, &report)?;
    let mut cases = vec![
        Change::Logo(LogoMode::Off),
        Change::Logo(LogoMode::Static),
        Change::Logo(LogoMode::Breathing),
        Change::BatteryCare(BatteryCare::Enable),
        Change::BatteryCare(BatteryCare::Disable),
    ];
    cases.extend(MODES.iter().copied().map(Change::Performance));
    for case in cases {
        let result = apply(&device, &case);
        let observed = read(&device);
        let restored = restore(&device, &original);
        report["cases"].as_array_mut().unwrap().push(
            json!({"change": case, "passed": result.is_ok() && restored.is_ok(),
            "observed": observed.as_ref().ok(), "error": result.err().map(|e|format!("{e:#}")),
            "restore_error": restored.as_ref().err().map(|e|format!("{e:#}"))}),
        );
        journal(&mut file, &report)?;
        restored.context("Batch stopped because restoration failed; use the recovery record")?;
    }
    // Map ordinary CPU/GPU combinations, never the undocumented undervolt preset.
    for cpu in [CpuBoost::Low, CpuBoost::Medium, CpuBoost::High, CpuBoost::Boost] {
        for gpu in [GpuBoost::Low, GpuBoost::Medium, GpuBoost::High] {
            let result = apply(&device, &Change::Performance(PerfMode::Custom))
                .and_then(|_| apply(&device, &Change::Boosts(cpu, gpu)));
            let observed = read(&device);
            let restored = restore(&device, &original);
            report["cases"].as_array_mut().unwrap().push(json!({"change": Change::Boosts(cpu,gpu),
                "passed": result.is_ok() && restored.is_ok(), "observed": observed.as_ref().ok(),
                "error": result.err().map(|e|format!("{e:#}")), "restore_error": restored.as_ref().err().map(|e|format!("{e:#}"))}));
            journal(&mut file, &report)?;
            restored.context("Boost test restoration failed")?;
        }
    }
    for rpm in [FAN_MIN, FAN_MAX] {
        let result = apply(&device, &Change::Performance(PerfMode::Balanced))
            .and_then(|_| apply(&device, &Change::ManualFans(rpm)));
        thread::sleep(Duration::from_secs(3));
        let observed = read(&device);
        let restored = restore(&device, &original);
        report["cases"].as_array_mut().unwrap().push(json!({"change": Change::ManualFans(rpm),
            "passed": result.is_ok() && restored.is_ok(), "observed": observed.as_ref().ok(),
            "error": result.err().map(|e|format!("{e:#}")), "restore_error": restored.as_ref().err().map(|e|format!("{e:#}"))}));
        journal(&mut file, &report)?;
        restored.context("Fan test restoration failed")?;
    }
    report["restored"] = serde_json::to_value(read(&device)?)?;
    report["complete"] = json!(true);
    journal(&mut file, &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_mode_resets_precede_targets_and_are_skipped_for_updates() {
        let initial = manual_fan_commands([4500, 4000], true);
        assert_eq!(initial.len(), 6);
        for (index, zone) in (1..=2).enumerate() {
            assert_eq!(initial[index * 3], (0x0d82, vec![0, zone, 0, 0]));
            assert_eq!(initial[index * 3 + 1], (0x0d02, vec![0, zone, 0, 1]));
            assert_eq!(initial[index * 3 + 2].0, 0x0d01);
            assert_eq!(initial[index * 3 + 2].1[..2], [1, zone]);
        }
        assert_eq!(initial[2].1[2], 45);
        assert_eq!(initial[5].1[2], 40);
        let update = manual_fan_commands([4000, 4500], false);
        assert_eq!(update.len(), 4);
        assert!(update.iter().all(|(cmd, _)| *cmd != 0x0d02));
    }
    #[test]
    fn recovery_snapshot_is_not_overwritten_and_rejects_other_models() {
        let path = std::env::temp_dir().join(format!(
            "rhelper-recovery-test-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let original = State {
            brightness: 176,
            logo: LogoMode::Static,
            battery_care: BatteryCare::Disable,
            performance: PerfMode::Balanced,
            fan_mode: FanMode::Auto,
            fan_setpoints: [2000, 2000],
            fan_actual: [2100, 2100],
            cpu: Some(CpuBoost::Medium),
            gpu: Some(GpuBoost::Low),
        };
        recovery_record(&path, &original).unwrap();
        let recovery = path.with_extension("recovery.json");
        let initial = std::fs::read(&recovery).unwrap();
        assert!(recovery_record(&path, &original).is_err());
        assert_eq!(std::fs::read(&recovery).unwrap(), initial);
        std::fs::write(&path, br#"{"model":"other"}"#).unwrap();
        assert!(restore_record(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(&recovery).unwrap();
    }
    #[test]
    fn transient_queries_retry_without_fabricating_values() {
        let mut attempts = 0;
        let value = retry_read(|| {
            attempts += 1;
            if attempts < 2 {
                anyhow::bail!("Transient response")
            }
            Ok(42)
        })
        .unwrap();
        assert_eq!((value, attempts), (42, 2));
        let mut failures = 0;
        assert!(retry_read::<u8>(|| {
            failures += 1;
            anyhow::bail!("Permanent failure")
        })
        .is_err());
        assert_eq!(failures, 3);
    }
    #[test]
    fn profiles_reject_battery_boost_and_undervolt_presets() {
        let mut profile = Profile {
            brightness: 176,
            logo: LogoMode::Static,
            battery_care: BatteryCare::Disable,
            performance: PerfMode::Custom,
            cpu: Some(CpuBoost::Medium),
            gpu: Some(GpuBoost::Low),
        };
        assert!(profile.validate(false).is_ok());
        assert!(profile.validate(true).is_err());
        profile.performance = PerfMode::Balanced;
        profile.cpu = Some(CpuBoost::Undervolt);
        assert!(profile.validate(false).is_err());
    }
    #[test]
    fn advertised_modes_and_fan_range_exclude_unvalidated_extremes() {
        assert!(!MODES.contains(&PerfMode::Hyperboost));
        assert!(!MODES.contains(&PerfMode::Battery));
        for rpm in [0, 2000, 3999, 4001, 4501, 5500] {
            assert!(validate_change(&Change::ManualFans(rpm)).is_err());
        }
        for rpm in [4000, 4200, 4500] {
            assert!(validate_change(&Change::ManualFans(rpm)).is_ok());
        }
        assert!(validate_change(&Change::ManualFanZones([4000, 4500])).is_ok());
        assert!(validate_change(&Change::ManualFanZones([4500, 4000])).is_ok());
        assert!(validate_change(&Change::ManualFanZones([4000, 3999])).is_err());
        assert!(validate_change(&Change::ManualFanZones([4501, 4000])).is_err());
    }
}
