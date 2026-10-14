//! Hardware inventory is passive. Probing is explicit and permits only getter packets.
use crate::{command, descriptor::Descriptor, device::Device, types::FanZone};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

/// Recognized for diagnostics only; deliberately absent from the GUI control table.
/// PID corroborated by https://github.com/openrazer/openrazer/issues/2488.
pub const BLADE_18_2025: Descriptor = Descriptor {
    model_number_prefix: "RZ09-0529",
    name: "Razer Blade 18 (2025)",
    pid: 0x02c7,
    features: &[],
    init_cmds: &[],
    perf_modes: Some(&[]),
    cpu_boosts: Some(&[]),
    gpu_boosts: Some(&[]),
    disallowed_boost_pairs: None,
};

#[derive(Serialize)]
pub struct Interface {
    pub index: usize,
    pub vendor_id: String,
    pub product_id: String,
    pub interface_number: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub product: Option<String>,
}

fn bios() -> Value {
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
        let key = match RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey("HARDWARE\\DESCRIPTION\\System\\BIOS")
        {
            Ok(key) => key,
            Err(e) => return json!({"error": e.to_string()}),
        };
        let mut result = serde_json::Map::new();
        // No serial number, machine name, user name, or HID paths in the report.
        for name in [
            "SystemManufacturer",
            "SystemProductName",
            "SystemSKU",
            "BIOSVersion",
            "BIOSReleaseDate",
        ] {
            let value = match key.get_value::<String, _>(name) {
                Ok(v) => json!(v),
                Err(e) => json!({"error": e.to_string()}),
            };
            result.insert(name.into(), value);
        }
        Value::Object(result)
    }
    #[cfg(not(windows))]
    json!({"error": "BIOS inventory requires Windows"})
}

fn is_blade_18(bios: &Value) -> bool {
    ["SystemSKU", "SystemProductName"].iter().any(|key| {
        bios[*key]
            .as_str()
            .is_some_and(|v| v.starts_with("RZ09-0529") || v.starts_with("Blade 18 - RZ09-0529"))
    })
}

pub fn is_target_system() -> bool {
    is_blade_18(&bios())
}

fn reading<T: std::fmt::Debug>(result: Result<T>) -> Value {
    match result {
        Ok(value) => json!({"ok": true, "value": format!("{value:?}")}),
        Err(e) => json!({"ok": false, "error": format!("{e:#}")}),
    }
}

/// `probe_interface` indexes the passive inventory from this invocation.
/// Does not initialize the device, change settings, or fall back to another interface.
pub fn collect(probe_interface: Option<usize>) -> Result<Value> {
    collect_impl(probe_interface, false)
}

/// Select the interface signature verified on the development Blade 18.
pub fn collect_status() -> Result<Value> {
    collect_impl(None, true)
}

fn collect_impl(probe_interface: Option<usize>, preferred: bool) -> Result<Value> {
    let bios = bios();
    let api = hidapi::HidApi::new().context("Cannot enumerate HID interfaces")?;
    let devices: Vec<_> = api.device_list().filter(|d| d.vendor_id() == 0x1532).collect();
    let probe_interface = if preferred {
        let matches: Vec<_> = devices
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                d.product_id() == BLADE_18_2025.pid
                    && d.interface_number() == 2
                    && d.usage_page() == 1
                    && d.usage() == 2
            })
            .map(|(index, _)| index)
            .collect();
        ensure!(
            matches.len() == 1,
            "Expected exactly one Blade 18 control interface; found {}",
            matches.len()
        );
        Some(matches[0])
    } else {
        probe_interface
    };
    let interfaces: Vec<_> = devices
        .iter()
        .enumerate()
        .map(|(index, d)| Interface {
            index,
            vendor_id: format!("{:04x}", d.vendor_id()),
            product_id: format!("{:04x}", d.product_id()),
            interface_number: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            product: d.product_string().map(str::to_owned),
        })
        .collect();
    let recognized = is_blade_18(&bios);
    let mut report = json!({
        "schema_version": 1, "library_version": env!("CARGO_PKG_VERSION"), "bios": bios,
        "mode": if probe_interface.is_some() { "getter_probe" } else { "passive_inventory" },
        "blade_18_2025": {"recognized": recognized, "expected_pid": "02c7", "control_support": "pending_validation"},
        "interfaces": interfaces,
    });
    if let Some(index) = probe_interface {
        ensure!(recognized, "Getter probe is restricted to the Blade 18 (2025)");
        let info = devices.get(index).context("Interface index is out of range")?;
        ensure!(info.product_id() == BLADE_18_2025.pid, "Selected interface is not the Blade 18");
        let device = Device::open_diagnostic(&api, info, BLADE_18_2025)?;
        report["probe_interface"] = json!(index);
        report["readings"] = json!({
            "keyboard_brightness": reading(command::get_keyboard_brightness(&device)),
            "battery_care": reading(command::get_battery_care(&device)),
            "performance_and_fan_mode": reading(command::get_perf_mode(&device)),
            "fan_1_actual_rpm": reading(command::get_fan_actual_rpm(&device, FanZone::Zone1)),
            "fan_2_actual_rpm": reading(command::get_fan_actual_rpm(&device, FanZone::Zone2)),
            "logo_mode": reading(command::get_logo_mode(&device)),
            "lights_always_on": reading(command::get_lights_always_on(&device)),
        });
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_target_without_enabling_unverified_controls() {
        assert!(is_blade_18(&json!({"SystemSKU": "RZ09-05299ER9"})));
        assert!(!is_blade_18(&json!({"SystemSKU": "RZ09-0528"})));
        assert!(!is_blade_18(&json!({})));
        assert!(BLADE_18_2025.features.is_empty());
        assert!(!crate::descriptor::SUPPORTED.iter().any(|d| d.model_number_prefix == "RZ09-0529"));
    }
}
