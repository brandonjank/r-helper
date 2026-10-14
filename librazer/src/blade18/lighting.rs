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
    #[serde(default)]
    pub underside_brightness: u8,
    pub keyboard_effect: Vec<u8>,
    pub logo_effect: Vec<u8>,
    pub keyboard_brightness: u8,
    pub logo_brightness: u8,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Change {
    StartupAnimation(bool),
    Underside(bool),
    UndersideBrightness(u8),
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
        underside_brightness: {
            let reply = c::send_command(device, 0x0383, &[1, 38, 0])?;
            ensure!(reply.get_args()[..2] == [1, 38], "Underside brightness selector mismatch");
            reply.get_args()[2]
        },
        keyboard_effect: effect(device, 5)?,
        logo_effect: effect(device, 4)?,
        keyboard_brightness: brightness(device, 5)?,
        logo_brightness: brightness(device, 4)?,
    })
}

pub fn status() -> Result<Lighting> {
    read(&open()?)
}

fn static_matches(effect: &[u8], rgb: [u8; 3]) -> bool {
    // This Blade accepts rate 1 but canonicalizes the irrelevant static rate to 0.
    effect.len() == 9
        && effect[..4] == [1, 5, 1, 0]
        && effect[4] <= 1
        && effect[5] == 1
        && effect[6..] == rgb
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyboardEffect {
    Static,
    Breathing,
    Spectrum,
}
impl KeyboardEffect {
    fn packet(self, rgb: [u8; 3]) -> Vec<u8> {
        match self {
            Self::Static | Self::Breathing => vec![
                1,
                5,
                if self == Self::Static { 1 } else { 2 },
                0,
                1,
                1,
                rgb[0],
                rgb[1],
                rgb[2],
            ],
            Self::Spectrum => vec![1, 5, 3, 0, 0, 0, 0, 0, 0],
        }
    }
    fn matches(self, data: &[u8], rgb: [u8; 3]) -> bool {
        let args = self.packet(rgb);
        if self == Self::Spectrum {
            return data == &args[..6];
        }
        data.len() == args.len() && data[..4] == args[..4] && data[4] <= 1 && data[5..] == args[5..]
    }
}
fn recoverable_packet(data: &[u8]) -> Option<Vec<u8>> {
    let rgb = if data.len() == 9 { [data[6], data[7], data[8]] } else { [0; 3] };
    [KeyboardEffect::Static, KeyboardEffect::Breathing]
        .into_iter()
        .find(|effect| effect.matches(data, rgb))
        .map(|effect| effect.packet(rgb))
}

/// Explicit handoff: custom-frame pixels cannot be recovered from effect metadata.
/// Caller must make replacement of the current keyboard effect visible to the user.
pub fn keyboard_static(rgb: [u8; 3], journal_path: &Path) -> Result<Lighting> {
    keyboard_effect(KeyboardEffect::Static, rgb, journal_path)
}
pub fn keyboard_effect(
    effect: KeyboardEffect,
    rgb: [u8; 3],
    journal_path: &Path,
) -> Result<Lighting> {
    ensure!(
        effect != KeyboardEffect::Spectrum,
        "Spectrum is advertised but did not apply on this firmware; no write sent"
    );
    let mut file = File::options().create_new(true).write(true).open(journal_path)?;
    let device = open()?;
    let original = read(&device)?;
    let args = effect.packet(rgb);
    super::journal(
        &mut file,
        &serde_json::json!({"model":"RZ09-0529","operation":"keyboard_static",
        "original":original,"requested":args,"custom_frame_pixels_saved":false}),
    )?;
    let result = (|| {
        c::send_command(&device, 0x0f02, &args)?;
        thread::sleep(Duration::from_millis(250));
        let after = read(&device)?;
        ensure!(
            effect.matches(&after.keyboard_effect, rgb),
            "Keyboard static readback mismatch: {:?}",
            after.keyboard_effect
        );
        ensure!(
            after.startup_animation == original.startup_animation
                && after.underside == original.underside
                && after.underside_brightness == original.underside_brightness
                && after.logo_effect == original.logo_effect
                && after.logo_brightness == original.logo_brightness
                && after.keyboard_brightness == original.keyboard_brightness,
            "Another lighting setting changed"
        );
        Ok(after)
    })();
    let rollback = if result.is_err() && recoverable_packet(&original.keyboard_effect).is_some() {
        let previous = recoverable_packet(&original.keyboard_effect).unwrap();
        Some(c::send_command(&device, 0x0f02, &previous).and_then(|_| {
            thread::sleep(Duration::from_millis(250));
            let after = read(&device)?;
            ensure!(
                recoverable_packet(&after.keyboard_effect) == Some(previous.clone()),
                "Previous static color recovery mismatch"
            );
            Ok(())
        }))
    } else {
        None
    };
    super::journal(
        &mut file,
        &serde_json::json!({"model":"RZ09-0529","operation":"keyboard_static",
        "original":original,"requested":args,"custom_frame_pixels_saved":false,
        "result":result.as_ref().ok(),"error":result.as_ref().err().map(|e:&anyhow::Error|format!("{e:#}")),
        "static_rollback":rollback.as_ref().map(|v|v.as_ref().map(|_|true).map_err(|e|format!("{e:#}"))),
        "recovery_note":"A previous custom frame must be reapplied by its owner (for example Synapse); effect metadata is not a pixel backup."}),
    )?;
    if let Some(Err(error)) = rollback {
        return Err(error.context("Static keyboard rollback failed"));
    }
    result
}

pub fn validate_keyboard_effects(path: &Path) -> Result<serde_json::Value> {
    let mut file = File::options().create_new(true).write(true).open(path)?;
    let device = open()?;
    let original = read(&device)?;
    let restore = recoverable_packet(&original.keyboard_effect)
        .context("Effect test requires a recoverable firmware baseline")?;
    let mut recovery =
        File::options().create_new(true).write(true).open(path.with_extension("recovery.json"))?;
    super::journal(
        &mut recovery,
        &serde_json::json!({"model":"RZ09-0529","operation":"keyboard_static_recovery","original":original}),
    )?;
    let mut cases = Vec::new();
    for effect in [KeyboardEffect::Breathing] {
        let rgb = [32, 64, 255];
        let args = effect.packet(rgb);
        let outcome = (|| {
            c::send_command(&device, 0x0f02, &args)?;
            thread::sleep(Duration::from_secs(4));
            let after = read(&device)?;
            let mut expected = original.clone();
            expected.keyboard_effect = after.keyboard_effect.clone();
            ensure!(
                effect.matches(&after.keyboard_effect, rgb) && after == expected,
                "Effect or independent lighting readback mismatch: {after:?}"
            );
            Ok::<_, anyhow::Error>(after)
        })();
        let restored = c::send_command(&device, 0x0f02, &restore).and_then(|_| {
            thread::sleep(Duration::from_millis(250));
            let after = read(&device)?;
            ensure!(after == original, "Effect restoration mismatch");
            Ok(())
        });
        cases.push(serde_json::json!({"effect":effect,"passed":outcome.is_ok(),"readback":outcome.as_ref().ok(),"error":outcome.err().map(|e|format!("{e:#}")),"restored":restored.is_ok()}));
        super::journal(
            &mut file,
            &serde_json::json!({"keyboard_effect_validation":{"original":original,"cases":cases}}),
        )?;
        restored?;
    }
    ensure!(
        cases.iter().all(|v| v["passed"] == true),
        "Effect validation failed; original effect restored"
    );
    Ok(serde_json::json!({"keyboard_effect_validation":{"original":original,"cases":cases}}))
}

/// Requires an already recoverable firmware static effect; never adopts CustomFrame.
pub fn validate_keyboard_static(path: &Path) -> Result<serde_json::Value> {
    let mut file = File::options().create_new(true).write(true).open(path)?;
    let device = open()?;
    let original = read(&device)?;
    let data = &original.keyboard_effect;
    ensure!(
        data.len() == 9 && static_matches(data, [data[6], data[7], data[8]]),
        "Keyboard test requires a firmware static baseline; no custom-frame writes were sent"
    );
    let mut restore = data.clone();
    restore[4] = 1;
    let recovery = serde_json::json!({"model":"RZ09-0529","operation":"keyboard_static_recovery","original":original});
    let mut recovery_file =
        File::options().create_new(true).write(true).open(path.with_extension("recovery.json"))?;
    super::journal(&mut recovery_file, &recovery)?;
    super::journal(&mut file, &recovery)?;
    let mut cases = Vec::new();
    for rgb in [[255, 0, 0], [0, 255, 0], [0, 0, 255]] {
        let outcome = (|| {
            c::send_command(&device, 0x0f02, &[1, 5, 1, 0, 1, 1, rgb[0], rgb[1], rgb[2]])?;
            thread::sleep(Duration::from_secs(2));
            let after = read(&device)?;
            let mut expected = original.clone();
            expected.keyboard_effect = after.keyboard_effect.clone();
            ensure!(
                static_matches(&after.keyboard_effect, rgb) && after == expected,
                "RGB or independent-region mismatch: {after:?}"
            );
            Ok::<_, anyhow::Error>(after)
        })();
        let restored = c::send_command(&device, 0x0f02, &restore).and_then(|_| {
            thread::sleep(Duration::from_millis(250));
            let after = read(&device)?;
            ensure!(after == original, "Static color restoration mismatch");
            Ok(())
        });
        cases.push(serde_json::json!({"rgb":rgb,"passed":outcome.is_ok(),"readback":outcome.as_ref().ok(),
            "error":outcome.err().map(|e|format!("{e:#}")),"restored":restored.is_ok(),"restore_error":restored.as_ref().err().map(|e|format!("{e:#}"))}));
        super::journal(
            &mut file,
            &serde_json::json!({"keyboard_static_validation":{"original":original,"cases":cases}}),
        )?;
        restored?;
    }
    ensure!(
        cases.iter().all(|v| v["passed"] == true),
        "RGB validation failed; original static color restored"
    );
    Ok(serde_json::json!({"keyboard_static_validation":{"original":original,"cases":cases}}))
}

pub fn restore_keyboard_record(path: &Path) -> Result<Lighting> {
    let record: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        record["model"] == "RZ09-0529" && record["operation"] == "keyboard_static_recovery",
        "Not a Blade 18 static keyboard recovery record"
    );
    let original: Lighting = serde_json::from_value(record["original"].clone())?;
    let data = &original.keyboard_effect;
    let args = recoverable_packet(data).context("Recovery is not a verified firmware effect")?;
    let device = open()?;
    let before = read(&device)?;
    c::send_command(&device, 0x0f02, &args)?;
    thread::sleep(Duration::from_millis(250));
    let after = read(&device)?;
    let mut expected = before;
    expected.keyboard_effect = after.keyboard_effect.clone();
    ensure!(
        recoverable_packet(&after.keyboard_effect) == Some(args) && after == expected,
        "Keyboard effect recovery mismatch"
    );
    Ok(after)
}

fn write(device: &Device, change: Change) -> Result<()> {
    match change {
        Change::StartupAnimation(enabled) => {
            c::send_command(device, 0x0f18, &[0, u8::from(!enabled)])?;
        }
        Change::Underside(enabled) => {
            c::send_command(device, 0x0300, &[1, 38, u8::from(enabled)])?;
        }
        Change::UndersideBrightness(value) => {
            c::send_command(device, 0x0303, &[1, 38, value])?;
        }
    }
    thread::sleep(Duration::from_millis(250));
    Ok(())
}

fn matches_change(state: &Lighting, change: Change) -> bool {
    match change {
        Change::StartupAnimation(v) => state.startup_animation == v,
        Change::Underside(v) => state.underside == v,
        Change::UndersideBrightness(v) => state.underside_brightness == v,
    }
}

fn original_change(state: &Lighting, change: Change) -> Change {
    match change {
        Change::StartupAnimation(_) => Change::StartupAnimation(state.startup_animation),
        Change::Underside(_) => Change::Underside(state.underside),
        Change::UndersideBrightness(_) => Change::UndersideBrightness(state.underside_brightness),
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
                Change::UndersideBrightness(_) =>
                    after.startup_animation == original.startup_animation
                        && after.underside == original.underside,
            },
            "The other lighting switch changed during the operation"
        );
        ensure!(
            (matches!(change, Change::UndersideBrightness(_))
                || after.underside_brightness == original.underside_brightness)
                && after.keyboard_effect == original.keyboard_effect
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
        Change::UndersideBrightness(if original.underside_brightness >= 8 {
            original.underside_brightness - 8
        } else {
            original.underside_brightness + 8
        }),
    ] {
        let outcome = write(&device, change).and_then(|_| {
            let after = read(&device)?;
            let mut expected = original.clone();
            match change {
                Change::StartupAnimation(v) => expected.startup_animation = v,
                Change::Underside(v) => expected.underside = v,
                Change::UndersideBrightness(v) => expected.underside_brightness = v,
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

/// Restore switches and captured underside brightness, never keyboard frames.
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
    let has_brightness = record["original"]["underside_brightness"].is_number();
    let underside_brightness = if has_brightness {
        write(&device, Change::UndersideBrightness(original.underside_brightness))
    } else {
        Ok(())
    };
    startup?;
    underside?;
    underside_brightness?;
    let after = read(&device)?;
    ensure!(
        after.startup_animation == original.startup_animation
            && after.underside == original.underside,
        "Lighting-switch recovery readback mismatch"
    );
    ensure!(
        after.underside_brightness
            == if has_brightness {
                original.underside_brightness
            } else {
                before.underside_brightness
            },
        "Underside brightness recovery mismatch"
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
    fn only_verified_firmware_effects_can_be_recovered() {
        assert_eq!(
            recoverable_packet(&[1, 5, 2, 0, 0, 1, 32, 64, 255]),
            Some(vec![1, 5, 2, 0, 1, 1, 32, 64, 255])
        );
        assert!(recoverable_packet(&[1, 5, 8, 0, 0, 0]).is_none());
        assert!(recoverable_packet(&[1, 5, 3, 0, 0, 0]).is_none());
        assert!(recoverable_packet(&[1, 4, 2, 0, 0, 1, 32, 64, 255]).is_none());
        assert!(recoverable_packet(&[1, 5, 2, 0, 0, 2, 32, 64, 255, 0, 0, 0]).is_none());
    }
    #[test]
    fn switch_decoder_rejects_unknown_values_and_selectors() {
        assert!(!read_bool(&[0, 38, 0], &[0, 38]).unwrap());
        assert!(read_bool(&[0, 38, 1], &[0, 38]).unwrap());
        assert!(read_bool(&[0, 4, 1], &[0, 38]).is_err());
        assert!(read_bool(&[0, 38, 2], &[0, 38]).is_err());
        assert!(read_bool(&[], &[]).is_err());
    }
    #[test]
    fn static_readback_accepts_only_the_observed_rate_normalization() {
        let mut effect = [1, 5, 1, 0, 0, 1, 32, 64, 255];
        assert!(static_matches(&effect, [32, 64, 255]));
        effect[4] = 1;
        assert!(static_matches(&effect, [32, 64, 255]));
        effect[4] = 2;
        assert!(!static_matches(&effect, [32, 64, 255]));
        effect[4] = 0;
        effect[1] = 4;
        assert!(!static_matches(&effect, [32, 64, 255]));
        effect[1] = 5;
        effect[8] = 0;
        assert!(!static_matches(&effect, [32, 64, 255]));
        assert!(!static_matches(&effect[..8], [32, 64, 255]));
    }
}
