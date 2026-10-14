//! One-time charging override; preserve the shared power-control bits.
use super::{c, open};
use anyhow::{ensure, Result};
use std::path::Path;

pub fn active() -> Result<bool> {
    Ok(c::send_command(&open()?, 0x078f, &[0])?.get_args()[0] & 4 != 0)
}
pub fn set(enabled: bool, path: &Path) -> Result<bool> {
    let mut file = std::fs::File::options().create_new(true).write(true).open(path)?;
    let device = open()?;
    let original = c::send_command(&device, 0x078f, &[0])?.get_args()[0];
    // Only the non-override bit pattern observed on this unit is enabled.
    ensure!(original & !4 == 8, "Unverified shared power flags; no change sent");
    let target = if enabled { original | 4 } else { original & !4 };
    super::journal(
        &mut file,
        &serde_json::json!({"model":"RZ09-0529","operation":"charge_once","original":original,"target":target}),
    )?;
    let result = (|| {
        c::send_command(&device, 0x070f, &[target])?;
        let after = c::send_command(&device, 0x078f, &[0])?.get_args()[0];
        ensure!(after == target, "Charge-once readback mismatch");
        Ok(enabled)
    })();
    let rollback = if result.is_err() {
        Some(c::send_command(&device, 0x070f, &[original]).and_then(|_| {
            ensure!(
                c::send_command(&device, 0x078f, &[0])?.get_args()[0] == original,
                "Charge-once rollback mismatch"
            );
            Ok(())
        }))
    } else {
        None
    };
    super::journal(
        &mut file,
        &serde_json::json!({"model":"RZ09-0529","operation":"charge_once","original":original,"target":target,"error":result.as_ref().err().map(|e:&anyhow::Error|format!("{e:#}")),"rollback":rollback.as_ref().map(|r|r.as_ref().map(|_|true).map_err(|e|format!("{e:#}")))}),
    )?;
    if let Some(Err(error)) = rollback {
        return Err(error);
    }
    result
}
