//! Blade 18 keyboard brightness access; no fan, performance, battery, or logo writes.
use crate::{command, device::Device, diagnostics::open_blade18_diagnostic};
use anyhow::{ensure, Context, Result};
use serde::Serialize;

trait BrightnessDevice {
    fn read(&self) -> Result<u8>;
    fn write(&self, value: u8) -> Result<()>;
}

impl BrightnessDevice for Device {
    fn read(&self) -> Result<u8> {
        command::get_keyboard_brightness(self)
    }
    fn write(&self, value: u8) -> Result<()> {
        command::set_keyboard_brightness(self, value)
    }
}

#[derive(Debug, Serialize)]
pub struct Validation {
    pub original: u8,
    pub requested: u8,
    pub observed: Option<u8>,
    pub restored: Option<u8>,
    pub passed: bool,
    pub test_error: Option<String>,
    pub restore_error: Option<String>,
}

fn test_value(original: u8) -> u8 {
    if original >= 8 {
        original - 8
    } else {
        original + 8
    }
}

fn validate(
    device: &impl BrightnessDevice,
    record_original: impl FnOnce(u8) -> Result<()>,
) -> Result<Validation> {
    let original = device.read().context("Cannot read original brightness; no write attempted")?;
    record_original(original).context("Cannot record recovery value; no write attempted")?;
    let requested = test_value(original);
    // Always attempt restoration even if the setter fails after a possible hardware write.
    let test = device.write(requested).and_then(|_| {
        std::thread::sleep(std::time::Duration::from_millis(250));
        device.read()
    });
    let restore = device.write(original).and_then(|_| {
        std::thread::sleep(std::time::Duration::from_millis(250));
        device.read()
    });
    let observed = test.as_ref().ok().copied();
    let restored = restore.as_ref().ok().copied();
    Ok(Validation {
        original,
        requested,
        observed,
        restored,
        passed: observed == Some(requested) && restored == Some(original),
        test_error: test.err().map(|e| format!("{e:#}")),
        restore_error: restore.err().map(|e| format!("{e:#}")),
    })
}

/// A small temporary adjustment followed by unconditional restoration and readback.
/// The callback must durably record the original value before any setter is sent.
pub fn validate_keyboard(record_original: impl FnOnce(u8) -> Result<()>) -> Result<Validation> {
    let mut device = open_blade18_diagnostic()?;
    device.enable_keyboard_brightness()?;
    validate(&device, record_original)
}

pub fn read_keyboard() -> Result<u8> {
    open_blade18_diagnostic()?.read()
}

/// Apply and verify a brightness value, restoring the previous value on failure.
pub fn set_keyboard(value: u8) -> Result<u8> {
    let mut device = open_blade18_diagnostic()?;
    device.enable_keyboard_brightness()?;
    let original = device.read()?;
    let result = device.write(value).and_then(|_| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let observed = device.read()?;
        ensure!(observed == value, "Brightness readback {observed} differs from requested {value}");
        Ok(observed)
    });
    if let Err(error) = result {
        let restore = device.write(original).and_then(|_| device.read());
        anyhow::bail!("Brightness update failed: {error:#}. Original: {original}; restore result: {restore:?}");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    struct Fake {
        value: RefCell<u8>,
        writes: RefCell<Vec<u8>>,
        fail_first: bool,
    }
    impl BrightnessDevice for Fake {
        fn read(&self) -> Result<u8> {
            Ok(*self.value.borrow())
        }
        fn write(&self, value: u8) -> Result<()> {
            self.writes.borrow_mut().push(value);
            *self.value.borrow_mut() = value;
            if self.fail_first && self.writes.borrow().len() == 1 {
                anyhow::bail!("Lost acknowledgment");
            }
            Ok(())
        }
    }
    #[test]
    fn restores_even_when_first_write_reports_failure() {
        let device =
            Fake { value: RefCell::new(176), writes: RefCell::new(vec![]), fail_first: true };
        let result = validate(&device, |_| Ok(())).unwrap();
        assert!(!result.passed);
        assert_eq!(result.restored, Some(176));
        assert_eq!(*device.writes.borrow(), vec![168, 176]);
    }
    #[test]
    fn refuses_to_write_without_a_recovery_record() {
        let device =
            Fake { value: RefCell::new(176), writes: RefCell::new(vec![]), fail_first: false };
        assert!(validate(&device, |_| anyhow::bail!("Disk full")).is_err());
        assert!(device.writes.borrow().is_empty());
    }
    #[test]
    fn temporary_step_is_small_and_valid_at_boundaries() {
        for original in 0..=255u8 {
            assert_eq!(original.abs_diff(test_value(original)), 8);
        }
    }
}
