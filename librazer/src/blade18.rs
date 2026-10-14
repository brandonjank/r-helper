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
            ensure!(
                c::get_perf_mode(device)?.0 == PerfMode::Balanced,
                "Manual fans require Balanced mode"
            );
            for (zone, rpm) in [FanZone::Zone1, FanZone::Zone2].into_iter().zip(targets) {
                c::set_fan_zone_rpm(device, zone, rpm, false)?;
            }
            c::set_fan_mode(device, FanMode::Manual)?;
            for (zone, rpm) in [FanZone::Zone1, FanZone::Zone2].into_iter().zip(targets) {
                c::set_fan_zone_rpm(device, zone, rpm, true)?;
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
    for (zone, rpm) in [1u8, 2].into_iter().zip(original.fan_setpoints) {
        attempt!(c::send_command(device, 0x0d01, &[0, zone, (rpm / 100) as u8]));
    }
    attempt!(c::set_fan_mode(device, original.fan_mode));
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
            && (original.fan_mode == FanMode::Auto
                || state.fan_setpoints == original.fan_setpoints),
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
    restore(&open()?, &original)
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
    validate_fan_targets(path, &[([4000, 4000], 12), ([4500, 4500], 6)])
}

/// Test unequal zone targets in both directions, without assuming CPU/GPU mapping.
pub fn validate_fan_zones(path: &Path) -> Result<Value> {
    validate_fan_targets(path, &[([4000, 4500], 6), ([4500, 4000], 6)])
}

fn validate_fan_targets(path: &Path, cases: &[([u16; 2], usize)]) -> Result<Value> {
    let mut file = File::options().write(true).create_new(true).open(path)?;
    let device = open()?;
    let original = read(&device)?;
    recovery_record(path, &original)?;
    let mut report = json!({"original": original, "samples": [], "complete": false});
    journal(&mut file, &report)?;
    let result = (|| -> Result<()> {
        apply(&device, &Change::Performance(PerfMode::Balanced))?;
        for &(targets, count) in cases {
            apply(&device, &Change::ManualFanZones(targets))?;
            for _ in 0..count {
                thread::sleep(Duration::from_secs(5));
                let observed = read(&device)?;
                eprintln!(
                    "Fan request {targets:?}, setpoints {:?}, actual {:?}",
                    observed.fan_setpoints, observed.fan_actual
                );
                report["samples"].as_array_mut().unwrap().push(serde_json::to_value(observed)?);
                journal(&mut file, &report)?;
            }
        }
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
