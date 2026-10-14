//! Model-711 lighting controls. Ordinary effects execute in firmware; no frame loop.
use super::{c, open};
use crate::device::Device;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs::File, path::Path, thread, time::Duration};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lighting {
    pub startup_animation: bool,
    pub underside: bool,
    pub keyboard_effect: Vec<u8>,
    pub logo_effect: Vec<u8>,
    pub keyboard_brightness: u8,
    pub logo_brightness: u8,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Change {
    StartupAnimation(bool),
    Underside(bool),
}

fn read_bool(bytes: &[u8], prefix: &[u8]) -> Result<bool> {
    ensure!(bytes.starts_with(prefix), "Lighting selector mismatch");
    match bytes.get(prefix.len()) {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => anyhow::bail!("Unrecognized lighting switch value"),
    }
}

fn effect(device: &Device, region: u8) -> Result<Vec<u8>> {
    let mut args = [0; 80];
    args[..2].copy_from_slice(&[1, region]);
    let reply = c::send_command(device, 0x0f82, &args)?;
    let data = reply.get_args();
    ensure!(data[..2] == [1, region], "Effect selector mismatch");
    ensure!(data[5] <= 24, "Effect has an unreviewed color count");
    Ok(data[..6 + usize::from(data[5]) * 3].to_vec())
}

fn brightness(device: &Device, region: u8) -> Result<u8> {
    let reply = c::send_command(device, 0x0f84, &[1, region, 0])?;
    ensure!(reply.get_args()[..2] == [1, region], "Brightness selector mismatch");
    Ok(reply.get_args()[2])
}

fn read(device: &Device) -> Result<Lighting> {
    Ok(Lighting {
        startup_animation: !read_bool(c::send_command(device, 0x0f98, &[0])?.get_args(), &[])?,
        underside: read_bool(c::send_command(device, 0x0380, &[0, 38, 0])?.get_args(), &[0, 38])?,
        keyboard_effect: effect(device, 5)?,
        logo_effect: effect(device, 4)?,
        keyboard_brightness: brightness(device, 5)?,
        logo_brightness: brightness(device, 4)?,
    })
}

pub fn status() -> Result<Lighting> {
    read(&open()?)
}

fn write(device: &Device, change: Change) -> Result<()> {
    match change {
        Change::StartupAnimation(enabled) => {
            c::send_command(device, 0x0f18, &[0, u8::from(!enabled)])?;
        }
        Change::Underside(enabled) => {
            c::send_command(device, 0x0300, &[1, 38, u8::from(enabled)])?;
        }
    }
    thread::sleep(Duration::from_millis(250));
    Ok(())
}

fn matches_change(state: &Lighting, change: Change) -> bool {
    match change {
        Change::StartupAnimation(v) => state.startup_animation == v,
        Change::Underside(v) => state.underside == v,
    }
}

fn original_change(state: &Lighting, change: Change) -> Change {
    match change {
        Change::StartupAnimation(_) => Change::StartupAnimation(state.startup_animation),
        Change::Underside(_) => Change::Underside(state.underside),
    }
}

pub fn change(change: Change) -> Result<Lighting> {
    let device = open()?;
    let original = read(&device)?;
    let result = write(&device, change).and_then(|_| {
        let after = read(&device)?;
        ensure!(matches_change(&after, change), "Lighting readback mismatch");
        ensure!(
            match change {
                Change::StartupAnimation(_) => after.underside == original.underside,
                Change::Underside(_) => after.startup_animation == original.startup_animation,
            },
            "The other lighting switch changed during the operation"
        );
        ensure!(
            after.keyboard_effect == original.keyboard_effect
                && after.logo_effect == original.logo_effect
                && after.keyboard_brightness == original.keyboard_brightness
                && after.logo_brightness == original.logo_brightness,
            "Another lighting region changed during the operation"
        );
        Ok(after)
    });
    if result.is_err() {
        write(&device, original_change(&original, change)).context("Lighting rollback failed")?;
        ensure!(read(&device)? == original, "Lighting rollback readback mismatch");
    }
    result
}

/// Persist both original switches before a bounded toggle/readback/restore test.
pub fn validate(path: &Path) -> Result<serde_json::Value> {
    let mut report_file = File::options().create_new(true).write(true).open(path)?;
    let device = open()?;
    let original = read(&device)?;
    let recovery = serde_json::json!({"model":"RZ09-0529","operation":"lighting_switches","original":original});
    let mut recovery_file =
        File::options().create_new(true).write(true).open(path.with_extension("recovery.json"))?;
    super::journal(&mut recovery_file, &recovery)?;
    super::journal(&mut report_file, &recovery)?;
    let mut cases = Vec::new();
    for change in [
        Change::StartupAnimation(!original.startup_animation),
        Change::Underside(!original.underside),
    ] {
        let outcome = write(&device, change).and_then(|_| {
            let after = read(&device)?;
            let mut expected = original.clone();
            match change {
                Change::StartupAnimation(v) => expected.startup_animation = v,
                Change::Underside(v) => expected.underside = v,
            }
            ensure!(after == expected, "Switch or independent-region readback mismatch: {after:?}");
            Ok(after)
        });
        // Always restore, including when the setter or following getter failed.
        let restoration = write(&device, original_change(&original, change)).and_then(|_| {
            let after = read(&device)?;
            ensure!(after == original, "Lighting restoration mismatch: {after:?}");
            Ok(after)
        });
        cases.push(serde_json::json!({"change":change,"passed":outcome.is_ok(),
            "observed":outcome.as_ref().ok(),"error":outcome.err().map(|e|format!("{e:#}")),
            "restored":restoration.is_ok(),"restore_error":restoration.as_ref().err().map(|e|format!("{e:#}"))}));
        let report = serde_json::json!({"lighting_validation":{"original":original,"cases":cases}});
        super::journal(&mut report_file, &report)?;
        restoration?;
    }
    let report = serde_json::json!({"lighting_validation":{"original":original,"cases":cases}});
    ensure!(
        cases.iter().all(|c| c["passed"] == true),
        "Lighting validation failed; original switches restored"
    );
    Ok(report)
}

/// Restore switches only. Never replay a host-controlled custom keyboard frame.
pub fn restore_record(path: &Path) -> Result<Lighting> {
    let record: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        record["model"] == "RZ09-0529" && record["operation"] == "lighting_switches",
        "Not a Blade 18 lighting-switch recovery record"
    );
    let original: Lighting = serde_json::from_value(record["original"].clone())?;
    let device = open()?;
    let before = read(&device)?;
    // Attempt both switches even if one write fails.
    let startup = write(&device, Change::StartupAnimation(original.startup_animation));
    let underside = write(&device, Change::Underside(original.underside));
    startup?;
    underside?;
    let after = read(&device)?;
    ensure!(
        after.startup_animation == original.startup_animation
            && after.underside == original.underside,
        "Lighting-switch recovery readback mismatch"
    );
    ensure!(
        after.keyboard_effect == before.keyboard_effect
            && after.logo_effect == before.logo_effect
            && after.keyboard_brightness == before.keyboard_brightness
            && after.logo_brightness == before.logo_brightness,
        "Another lighting region changed during recovery"
    );
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switch_decoder_rejects_unknown_values_and_selectors() {
        assert!(!read_bool(&[0, 38, 0], &[0, 38]).unwrap());
        assert!(read_bool(&[0, 38, 1], &[0, 38]).unwrap());
        assert!(read_bool(&[0, 4, 1], &[0, 38]).is_err());
        assert!(read_bool(&[0, 38, 2], &[0, 38]).is_err());
        assert!(read_bool(&[], &[]).is_err());
    }
}
