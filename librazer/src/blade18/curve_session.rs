use super::*;
use crate::fan_curve::{Curve, DampedPlanner, Plan, Point, Reading, Validation};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum CurveSource {
    EcA,
    EcB,
    Hottest,
}
impl CurveSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::EcA => "EC A · CPU-associated",
            Self::EcB => "EC B · GPU-associated",
            Self::Hottest => "Higher of EC A/B",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurvePoint {
    pub celsius: f32,
    pub rpm: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneCurve {
    pub source: CurveSource,
    pub points: Vec<CurvePoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurveSettings {
    pub schema: u8,
    pub zones: [ZoneCurve; 3],
    pub downward_hold_seconds: u16,
    pub session_seconds: u32,
}
impl Default for CurveSettings {
    fn default() -> Self {
        Self {
            schema: 1,
            zones: [
                ZoneCurve {
                    source: CurveSource::EcA,
                    points: vec![
                        CurvePoint { celsius: 50.0, rpm: 4000 },
                        CurvePoint { celsius: 80.0, rpm: 4500 },
                    ],
                },
                ZoneCurve {
                    source: CurveSource::EcB,
                    points: vec![
                        CurvePoint { celsius: 35.0, rpm: 4000 },
                        CurvePoint { celsius: 70.0, rpm: 4500 },
                    ],
                },
                ZoneCurve {
                    source: CurveSource::Hottest,
                    points: vec![
                        CurvePoint { celsius: 50.0, rpm: 3000 },
                        CurvePoint { celsius: 80.0, rpm: 3500 },
                    ],
                },
            ],
            downward_hold_seconds: 5,
            session_seconds: 3600,
        }
    }
}
impl CurveSettings {
    fn curves(&self) -> Result<[Curve; 3]> {
        ensure!(self.schema == 1, "Unknown curve settings schema");
        ensure!(
            (30..=28800).contains(&self.session_seconds),
            "Session must be 30 seconds to 8 hours"
        );
        DampedPlanner::new(Duration::from_secs(self.downward_hold_seconds.into()))?;
        let make = |i: usize| {
            let zone = &self.zones[i];
            ensure!((2..=8).contains(&zone.points.len()), "Each curve needs 2-8 points");
            let (min, max) = if i == 2 { (3000, 3500) } else { (4000, 4500) };
            let cutoff = if zone.source == CurveSource::EcB { 75.0 } else { 85.0 };
            Curve::new(
                zone.points.iter().map(|p| Point { celsius: p.celsius, rpm: p.rpm }).collect(),
                min,
                max,
                cutoff,
            )
        };
        Ok([make(0)?, make(1)?, make(2)?])
    }
    pub fn validate(&self) -> Result<()> {
        self.curves().map(|_| ())
    }
}

fn write_value(path: &Path, value: &Value) -> Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary)?;
    serde_json::to_writer(&mut file, value)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)?;
    Ok(())
}
fn valid_tick_gap(previous: SystemTime, current: SystemTime) -> bool {
    current.duration_since(previous).is_ok_and(|gap| gap < Duration::from_secs(8))
}
fn restore_targets(
    device: &Device,
    thermal: &crate::thermal::Sample,
) -> Result<crate::thermal::Sample> {
    let mut errors = Vec::new();
    if let Err(e) = c::set_fan_mode(device, FanMode::Auto) {
        errors.push(format!("{e:#}"));
    }
    for (i, rpm) in thermal.target_rpm.iter().enumerate() {
        if let Err(e) = c::send_command(
            device,
            0x0d01,
            &[if i == 2 { 1 } else { 0 }, i as u8 + 1, (rpm / 100) as u8],
        ) {
            errors.push(format!("{e:#}"));
        }
    }
    let after = crate::thermal::read(device)?;
    ensure!(
        errors.is_empty()
            && after.modes.iter().all(|m| m[1] == 0)
            && after.target_rpm == thermal.target_rpm,
        "Fan-only recovery failed: {errors:?}; {after:?}"
    );
    Ok(after)
}
pub fn restore_curve_session(directory: &Path) -> Result<crate::thermal::Sample> {
    let record: Value = serde_json::from_slice(&std::fs::read(directory.join("recovery.json"))?)?;
    ensure!(
        record["model"] == "RZ09-0529" && record["operation"] == "curve_session",
        "Wrong curve recovery record"
    );
    let original: crate::thermal::Sample = serde_json::from_value(record["original"].clone())?;
    ensure!(
        original.modes.iter().all(|m| m[1] == 0)
            && original.target_rpm[..2].iter().all(|v| *v <= 5500 && v % 100 == 0)
            && original.target_rpm[2] <= 3500
            && original.target_rpm[2] % 100 == 0,
        "Invalid curve recovery targets"
    );
    let mut device = open()?;
    device.enable_fan_lab()?;
    restore_targets(&device, &original)
}

/// Explicit opt-in session. No voltage/performance/lighting setters, no NVIDIA polling.
pub fn run_curve_session(directory: &Path) -> Result<()> {
    let settings: CurveSettings =
        serde_json::from_slice(&std::fs::read(directory.join("config.json"))?)?;
    let curves = settings.curves()?;
    let mut device = open()?;
    device.enable_fan_lab()?;
    let state = read(&device)?;
    let original = crate::thermal::read(&device)?;
    ensure!(
        ac_connected()?
            && state.performance == PerfMode::Balanced
            && original.modes.iter().all(|m| m[1] == 0),
        "Start curves on AC in Balanced/Auto"
    );
    ensure!(
        original.ec_celsius[0] > 0
            && original.ec_celsius[0] < 80
            && original.ec_celsius[1] > 0
            && original.ec_celsius[1] < 70,
        "Available temperatures and thermal headroom required"
    );
    ensure!(
        original.target_rpm[2] <= 3500 && original.target_rpm[2] % 100 == 0,
        "Third target cannot be recovered"
    );
    ensure!(
        original.target_rpm[..2].iter().all(|rpm| *rpm <= 5500),
        "Original primary targets cannot be recovered"
    );
    let mut recovery =
        File::options().write(true).create_new(true).open(directory.join("recovery.json"))?;
    journal(
        &mut recovery,
        &json!({"model":"RZ09-0529","operation":"curve_session","original":original}),
    )?;
    let status_path = directory.join("status.json");
    write_value(&status_path, &json!({"phase":"starting","sample":original}))?;
    let start = Instant::now();
    let mut last_wall = SystemTime::now();
    let mut planner =
        DampedPlanner::new(Duration::from_secs(settings.downward_hold_seconds.into()))?;
    let mut previous: Option<[u16; 3]> = None;
    let mut writes = 0;
    let result = (|| -> Result<String> {
        loop {
            if directory.join("stop").exists() {
                return Ok("Stopped by request".into());
            }
            ensure!(
                std::fs::metadata(directory.join("watchdog-heartbeat"))
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .is_some_and(|age| age < Duration::from_secs(4)),
                "Watchdog heartbeat lost; returning to Auto"
            );
            if start.elapsed() >= Duration::from_secs(settings.session_seconds.into()) {
                return Ok("Session time limit reached".into());
            }
            let wall = SystemTime::now();
            ensure!(
                valid_tick_gap(last_wall, wall),
                "Resume, delayed worker or clock change; returning to Auto"
            );
            last_wall = wall;
            let query_start = Instant::now();
            let sample = crate::thermal::read(&device)?;
            ensure!(ac_connected()?, "AC disconnected or unknown; returning to Auto");
            ensure!(
                sample.ec_celsius[0] > 0
                    && sample.ec_celsius[0] < 85
                    && sample.ec_celsius[1] > 0
                    && sample.ec_celsius[1] < 75,
                "Temperature missing or cutoff reached; returning to Auto"
            );
            ensure!(
                sample
                    .modes
                    .iter()
                    .all(|m| m[0] == 0 && m[1] == if previous.is_none() { 0 } else { 1 }),
                "Cooling mode changed outside controller"
            );
            if let Some(old) = previous {
                ensure!(sample.target_rpm == old, "Targets changed outside controller");
            }
            let readings = settings.zones.each_ref().map(|zone| {
                let value = match zone.source {
                    CurveSource::EcA => sample.ec_celsius[0],
                    CurveSource::EcB => sample.ec_celsius[1],
                    CurveSource::Hottest => sample.ec_celsius[0].max(sample.ec_celsius[1]),
                };
                Some(Reading { celsius: value.into(), age: query_start.elapsed() })
            });
            let planned = planner.update(
                [&curves[0], &curves[1], &curves[2]],
                readings,
                Duration::from_secs(3),
                Validation { sensors_mapped: true, independent_actuators: true },
                start.elapsed(),
            );
            let Plan::Targets(targets) = planned else {
                return Ok(format!("{planned:?}"));
            };
            if previous.is_none_or(|old| old[..2] != targets[..2]) {
                apply(&device, &Change::ManualFanZones([targets[0], targets[1]]))?;
                writes += 2;
            }
            if previous.is_none_or(|old| old[2] != targets[2]) {
                c::send_command(&device, 0x0d01, &[1, 3, (targets[2] / 100) as u8])?;
                writes += 1;
            }
            let after = crate::thermal::read(&device)?;
            ensure!(
                after.target_rpm == targets && after.modes.iter().all(|m| m[1] == 1),
                "Target/mode readback mismatch"
            );
            write_value(
                &status_path,
                &json!({"phase":"running","elapsed_seconds":start.elapsed().as_secs_f64(),"captured_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),"sample":after,"targets":targets,"target_writes":writes}),
            )?;
            previous = Some(targets);
            // Short sleeps allow a prompt cooperative stop without GUI wakeups.
            for _ in 0..20 {
                if directory.join("stop").exists() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    })();
    let restored = restore_targets(&device, &original);
    write_value(
        &status_path,
        &json!({"phase":"stopped","reason":result.as_ref().ok(),"error":result.as_ref().err().map(|e|format!("{e:#}")),"sample":restored.as_ref().ok(),"recovery_error":restored.as_ref().err().map(|e|format!("{e:#}"))}),
    )?;
    restored?;
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_and_clock_rollback_require_automatic_fallback() {
        let t = UNIX_EPOCH + Duration::from_secs(100);
        assert!(valid_tick_gap(t, t + Duration::from_secs(2)));
        assert!(!valid_tick_gap(t, t + Duration::from_secs(8)));
        assert!(!valid_tick_gap(t, t - Duration::from_secs(1)));
    }
    #[test]
    fn settings_reject_unbounded_or_invalid_curves() {
        let mut config = CurveSettings::default();
        assert!(config.validate().is_ok());
        config.zones[2].points[1].rpm = 4000;
        assert!(config.validate().is_err());
        config = CurveSettings::default();
        config.zones[0].points[0].celsius = 100.0;
        assert!(config.validate().is_err());
        config = CurveSettings::default();
        config.session_seconds = 28801;
        assert!(config.validate().is_err());
        config = CurveSettings::default();
        config.zones[1].points[1].celsius = f32::NAN;
        assert!(config.validate().is_err());
    }
}
