//! Explicit model-scoped thermal telemetry. EC fields remain separate from die references.
use crate::{command as c, device::Device, diagnostics::open_blade18_diagnostic};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub ec_celsius: [u8; 2],
    pub actual_rpm: [u16; 3],
    pub target_rpm: [u16; 3],
    pub modes: [[u8; 2]; 3],
}

pub fn sample() -> Result<Sample> {
    read(&open_blade18_diagnostic()?)
}

pub(crate) fn read(device: &Device) -> Result<Sample> {
    let response = c::send_command(device, 0x0d85, &[0, 1, 0, 0])?;
    let t = response.get_args();
    ensure!(t[0] == 2 && t[1] <= 125 && t[2] <= 125, "Unrecognized EC temperature payload");
    let ec_celsius = [t[1], t[2]];
    let mut actual_rpm = [0; 3];
    let mut target_rpm = [0; 3];
    let mut modes = [[0; 2]; 3];
    for index in 0..3 {
        let zone = index as u8 + 1;
        let actual = c::send_command(device, 0x0d88, &[0, zone, 0])?;
        let target = c::send_command(device, 0x0d81, &[0, zone, 0])?;
        let mode = c::send_command(device, 0x0d82, &[0, zone, 0, 0])?;
        ensure!(
            actual.get_args()[1] == zone
                && target.get_args()[1] == zone
                && mode.get_args()[1] == zone,
            "Thermal selector mismatch"
        );
        actual_rpm[index] = u16::from(actual.get_args()[2]) * 100;
        target_rpm[index] = u16::from(target.get_args()[2]) * 100;
        modes[index] = [mode.get_args()[2], mode.get_args()[3]];
    }
    Ok(Sample { ec_celsius, actual_rpm, target_rpm, modes })
}
