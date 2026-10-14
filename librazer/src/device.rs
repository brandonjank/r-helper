use crate::descriptor::{Descriptor, SUPPORTED};
use crate::packet::Packet;

use anyhow::{anyhow, Context, Result};
use std::{thread, time};

pub struct Device {
    device: hidapi::HidDevice,
    pub info: Descriptor,
    read_only: bool,
    keyboard_brightness_only: bool,
    blade18_control: bool,
    fan_lab: bool,
    standard_framing: bool,
    _control_lock: Option<std::fs::File>,
}

// Read the model id and clip to conform with https://mysupport.razer.com/app/answers/detail/a_id/5481
fn read_device_model() -> Result<String> {
    #[cfg(target_os = "windows")]
    {
        let hklm = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE);
        let bios = hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\BIOS")?;
        let system_sku: String = bios.get_value("SystemSKU")?;
        Ok(system_sku.chars().take(10).collect())
    }
    #[cfg(not(target_os = "windows"))]
    anyhow::bail!("Automatic model detection is not implemented for this platform")
}

impl Device {
    const RAZER_VID: u16 = 0x1532;

    pub fn info(&self) -> &Descriptor {
        &self.info
    }

    pub fn new(descriptor: Descriptor) -> Result<Device> {
        let api = hidapi::HidApi::new().context("Failed to create hid api")?;

        // there are multiple devices with the same pid, pick first that support feature report
        for info in api.device_list().filter(|info| {
            (info.vendor_id(), info.product_id()) == (Device::RAZER_VID, descriptor.pid)
        }) {
            let path = info.path();
            let device = api.open_path(path)?;
            if device.send_feature_report(&[0, 0]).is_ok() {
                return Ok(Device {
                    device,
                    info: descriptor.clone(),
                    read_only: false,
                    keyboard_brightness_only: false,
                    blade18_control: false,
                    fan_lab: false,
                    standard_framing: false,
                    _control_lock: None,
                });
            }
        }
        anyhow::bail!("Failed to open device {:?}", descriptor)
    }

    pub fn send(&self, report: Packet) -> Result<Packet> {
        let report = if self.standard_framing { report.standard_framing() } else { report };
        if self.read_only {
            if self.fan_lab {
                report.ensure_fan_lab_access()?;
            } else if self.blade18_control {
                report.ensure_blade18_access()?;
            } else if self.keyboard_brightness_only {
                report.ensure_brightness_access()?;
            } else {
                report.ensure_diagnostic_query()?;
            }
        }
        // extra byte for report id
        let mut response_buf: Vec<u8> = vec![0x00; 1 + std::mem::size_of::<Packet>()];
        //println!("Report {:?}", report);

        const MAX_RETRIES: usize = 5;

        for attempt in 0..MAX_RETRIES {
            thread::sleep(time::Duration::from_micros(1000));

            self.device
                .send_feature_report(
                    [0_u8; 1] // report id
                        .iter()
                        .copied()
                        .chain(Into::<Vec<u8>>::into(&report).into_iter())
                        .collect::<Vec<_>>()
                        .as_slice(),
                )
                .context("Failed to send feature report")?;

            thread::sleep(time::Duration::from_micros(2000));

            let response_size = self.device.get_feature_report(&mut response_buf)?;
            if response_buf.len() != response_size {
                return Err(anyhow!("Response size != {}", response_buf.len()));
            }

            // skip report id byte
            let response = <&[u8] as TryInto<Packet>>::try_into(&response_buf[1..])?;
            //println!("Response {:?}", response);

            match response.ensure_matches_report(&report) {
                Ok(()) => return Ok(response),
                Err(error) if attempt == MAX_RETRIES - 1 => {
                    return Err(
                        error.context(format!("Invalid response after {MAX_RETRIES} attempts"))
                    );
                }
                Err(_) => {}
            }

            // Add a small delay before retrying
            thread::sleep(time::Duration::from_millis(500));
        }

        Err(anyhow!("Failed to send feature report"))
    }

    /// Open a specific enumerated interface without the upstream test report.
    pub(crate) fn open_diagnostic(
        api: &hidapi::HidApi,
        info: &hidapi::DeviceInfo,
        descriptor: Descriptor,
    ) -> Result<Self> {
        anyhow::ensure!(info.vendor_id() == Self::RAZER_VID && info.product_id() == descriptor.pid);
        #[cfg(windows)]
        let control_lock = {
            use std::os::windows::fs::OpenOptionsExt;
            let path = std::env::temp_dir().join("rhelper-blade18-hardware.lock");
            let mut result = None;
            for _ in 0..50 {
                match std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .share_mode(0)
                    .open(&path)
                {
                    Ok(file) => {
                        result = Some(file);
                        break;
                    }
                    Err(error) if error.raw_os_error() == Some(32) => {
                        thread::sleep(time::Duration::from_millis(100))
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Some(result.context("Another R-Helper operation owns the hardware; try again")?)
        };
        #[cfg(not(windows))]
        let control_lock = None;
        Ok(Self {
            device: api.open_path(info.path())?,
            info: descriptor,
            read_only: true,
            keyboard_brightness_only: false,
            blade18_control: false,
            fan_lab: false,
            standard_framing: false,
            _control_lock: control_lock,
        })
    }

    pub(crate) fn enable_keyboard_brightness(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.info.model_number_prefix == "RZ09-0529" && self.info.pid == 0x02c7,
            "Brightness access is restricted to Blade 18 (2025)"
        );
        self.keyboard_brightness_only = true;
        Ok(())
    }

    pub(crate) fn enable_blade18_controls(&mut self) -> Result<()> {
        anyhow::ensure!(self.info.model_number_prefix == "RZ09-0529" && self.info.pid == 0x02c7);
        self.blade18_control = true;
        Ok(())
    }

    pub(crate) fn enable_fan_lab(&mut self) -> Result<()> {
        self.enable_blade18_controls()?;
        self.fan_lab = true;
        Ok(())
    }

    pub(crate) fn use_standard_framing(&mut self) -> Result<()> {
        anyhow::ensure!(self.fan_lab, "Alternate framing requires diagnostic fan lab access");
        self.standard_framing = true;
        Ok(())
    }

    pub fn enumerate() -> Result<(Vec<u16>, String)> {
        let razer_pid_list: Vec<_> = hidapi::HidApi::new()?
            .device_list()
            .filter(|info| info.vendor_id() == Device::RAZER_VID)
            .map(|info| info.product_id())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        if razer_pid_list.is_empty() {
            anyhow::bail!("No Razer devices found")
        }

        match read_device_model() {
            Ok(model) if model.starts_with("RZ09-") => Ok((razer_pid_list, model)),
            Ok(model) => anyhow::bail!("Detected model but it's not a Razer laptop: {}", model),
            Err(e) => anyhow::bail!("Failed to detect model: {}", e),
        }
    }

    pub fn detect() -> Result<Device> {
        let (pid_list, model_number_prefix) = Device::enumerate()?;

        match SUPPORTED
            .iter()
            .find(|supported| model_number_prefix.starts_with(&supported.model_number_prefix))
        {
            Some(supported) => Device::new(supported.clone()),
            None => anyhow::bail!(
                "Model {} with PIDs {:0>4x?} is not supported",
                model_number_prefix,
                pid_list
            ),
        }
    }
}
