//! Offline curve planning only: no device access or background worker.
//! Limits and critical temperatures are supplied by a future validated model policy.
use anyhow::{ensure, Result};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub celsius: f32,
    pub rpm: u16,
}

#[derive(Debug)]
pub struct Curve {
    points: Vec<Point>,
    critical_celsius: f32,
}

impl Curve {
    pub fn new(
        points: Vec<Point>,
        min_rpm: u16,
        max_rpm: u16,
        critical_celsius: f32,
    ) -> Result<Self> {
        ensure!(min_rpm > 0 && min_rpm <= max_rpm, "Invalid RPM envelope");
        ensure!(points.len() >= 2, "A curve needs at least two points");
        ensure!(
            critical_celsius.is_finite() && critical_celsius > 0.0 && critical_celsius <= 125.0,
            "Invalid thermal cutoff"
        );
        for point in &points {
            ensure!(
                point.celsius.is_finite()
                    && point.celsius > 0.0
                    && point.celsius < critical_celsius,
                "Invalid curve temperature"
            );
            ensure!(
                (min_rpm..=max_rpm).contains(&point.rpm) && point.rpm % 100 == 0,
                "Target outside envelope or not encodable in 100 RPM steps"
            );
        }
        ensure!(
            points
                .windows(2)
                .all(|pair| pair[0].celsius < pair[1].celsius && pair[0].rpm <= pair[1].rpm),
            "Curve temperatures must increase and RPM must not decrease"
        );
        Ok(Self { points, critical_celsius })
    }

    fn target(&self, celsius: f32) -> u16 {
        if celsius <= self.points[0].celsius {
            return self.points[0].rpm;
        }
        for pair in self.points.windows(2) {
            if celsius <= pair[1].celsius {
                let fraction = (celsius - pair[0].celsius) / (pair[1].celsius - pair[0].celsius);
                let rpm = f32::from(pair[0].rpm) + fraction * f32::from(pair[1].rpm - pair[0].rpm);
                // Round upward so quantization never undercuts the requested cooling.
                return ((rpm / 100.0).ceil() as u16) * 100;
            }
        }
        self.points.last().unwrap().rpm
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub celsius: f32,
    pub age: Duration,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Validation {
    pub sensors_mapped: bool,
    pub independent_actuators: bool,
}

#[derive(Debug, PartialEq)]
pub enum Plan {
    Automatic(&'static str),
    Targets([u16; 3]),
}

/// An automatic plan requires a future executor to restore shared firmware auto mode.
/// This function only calculates a plan; it never performs that restoration itself.
pub fn plan(
    curves: [&Curve; 3],
    readings: [Option<Reading>; 3],
    max_age: Duration,
    validation: Validation,
) -> Plan {
    if !validation.sensors_mapped || !validation.independent_actuators {
        return Plan::Automatic("Sensor mapping and independent actuator validation required");
    }
    if max_age.is_zero() {
        return Plan::Automatic("Invalid freshness policy");
    }
    let mut targets = [0; 3];
    for index in 0..3 {
        let Some(reading) = readings[index] else {
            return Plan::Automatic("Missing temperature");
        };
        if reading.age > max_age {
            return Plan::Automatic("Stale temperature");
        }
        if !reading.celsius.is_finite() || reading.celsius <= 0.0 || reading.celsius > 125.0 {
            return Plan::Automatic("Invalid temperature");
        }
        if reading.celsius >= curves[index].critical_celsius {
            return Plan::Automatic("Thermal cutoff reached");
        }
        targets[index] = curves[index].target(reading.celsius);
    }
    Plan::Targets(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn curve() -> Curve {
        Curve::new(
            vec![Point { celsius: 40.0, rpm: 4000 }, Point { celsius: 80.0, rpm: 4500 }],
            4000,
            4500,
            90.0,
        )
        .unwrap()
    }
    fn reading(celsius: f32) -> Option<Reading> {
        Some(Reading { celsius, age: Duration::ZERO })
    }
    const VERIFIED: Validation = Validation { sensors_mapped: true, independent_actuators: true };

    #[test]
    fn independent_inputs_interpolate_quantize_and_clamp() {
        let curve = curve();
        assert_eq!(
            plan(
                [&curve; 3],
                [reading(20.0), reading(60.0), reading(85.0)],
                Duration::from_secs(3),
                VERIFIED
            ),
            Plan::Targets([4000, 4300, 4500])
        );
        assert_eq!(curve.target(40.0), 4000);
        assert_eq!(curve.target(80.0), 4500);
    }

    #[test]
    fn one_bad_sensor_returns_shared_automatic_plan() {
        let curve = curve();
        for index in 0..3 {
            for bad in [
                None,
                reading(f32::NAN),
                reading(f32::INFINITY),
                reading(0.0),
                reading(-1.0),
                reading(90.0),
                reading(126.0),
                Some(Reading { celsius: 60.0, age: Duration::from_secs(4) }),
            ] {
                let mut readings = [reading(60.0); 3];
                readings[index] = bad;
                assert!(matches!(
                    plan([&curve; 3], readings, Duration::from_secs(3), VERIFIED),
                    Plan::Automatic(_)
                ));
            }
        }
    }

    #[test]
    fn validation_is_required_for_every_plan() {
        let curve = curve();
        for validation in [
            Validation::default(),
            Validation { sensors_mapped: true, independent_actuators: false },
            Validation { sensors_mapped: false, independent_actuators: true },
        ] {
            assert!(matches!(
                plan([&curve; 3], [reading(60.0); 3], Duration::from_secs(3), validation),
                Plan::Automatic(_)
            ));
        }
        assert!(matches!(
            plan([&curve; 3], [reading(60.0); 3], Duration::ZERO, VERIFIED),
            Plan::Automatic(_)
        ));
    }

    #[test]
    fn malformed_curves_cannot_be_constructed() {
        for points in [
            vec![],
            vec![Point { celsius: 40.0, rpm: 4000 }],
            vec![Point { celsius: 40.0, rpm: 4000 }, Point { celsius: 40.0, rpm: 4500 }],
            vec![Point { celsius: 80.0, rpm: 4500 }, Point { celsius: 40.0, rpm: 4000 }],
            vec![Point { celsius: 40.0, rpm: 4500 }, Point { celsius: 80.0, rpm: 4000 }],
            vec![Point { celsius: f32::NAN, rpm: 4000 }, Point { celsius: 80.0, rpm: 4500 }],
            vec![Point { celsius: 40.0, rpm: 4050 }, Point { celsius: 80.0, rpm: 4500 }],
            vec![Point { celsius: 40.0, rpm: 3900 }, Point { celsius: 80.0, rpm: 4500 }],
            vec![Point { celsius: 40.0, rpm: 4000 }, Point { celsius: 90.0, rpm: 4500 }],
        ] {
            assert!(Curve::new(points, 4000, 4500, 90.0).is_err());
        }
    }
}
