//! Hardware inventory is passive. Probing is explicit and permits only getter packets.
use crate::{command, descriptor::Descriptor, device::Device, types::FanZone};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

/// Separate from legacy model tables; the model-scoped Blade 18 GUI gates its own controls.
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
    collect_impl(probe_interface, false, false, false)
}

/// Select the interface signature verified on the development Blade 18.
pub fn collect_status() -> Result<Value> {
    collect_impl(None, true, false, false)
}

/// Additional source-backed lighting getters. No LED-ID sweep or lighting writes.
pub fn collect_lighting() -> Result<Value> {
    collect_impl(None, true, true, false)
}

/// Explicit, bounded sensor investigation. Third-zone reads are experimental;
/// temperature fields are not yet established as CPU/GPU die temperatures.
pub fn collect_sensors() -> Result<Value> {
    collect_impl(None, true, false, true)
}

pub(crate) fn open_blade18_diagnostic() -> Result<Device> {
    ensure!(is_target_system(), "Brightness access requires a Blade 18 (2025)");
    let api = hidapi::HidApi::new()?;
    let matches: Vec<_> = api
        .device_list()
        .filter(|d| {
            d.vendor_id() == 0x1532
                && d.product_id() == BLADE_18_2025.pid
                && d.interface_number() == 2
                && d.usage_page() == 1
                && d.usage() == 2
        })
        .collect();
    ensure!(matches.len() == 1, "Expected exactly one Blade 18 control interface");
    Device::open_diagnostic(&api, matches[0], BLADE_18_2025)
}

fn collect_impl(
    probe_interface: Option<usize>,
    preferred: bool,
    lighting: bool,
    sensors: bool,
) -> Result<Value> {
    let bios = bios();
    let api = hidapi::HidApi::new().context("Cannot enumerate HID interfaces")?;
    // Include 1534 in passive discovery at the owner's request. This does not
    // identify it as Razer or authorize sending Razer packets to it.
    let devices: Vec<_> =
        api.device_list().filter(|d| matches!(d.vendor_id(), 0x1532 | 0x1534)).collect();
    let probe_interface = if preferred {
        let matches: Vec<_> = devices
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                d.vendor_id() == 0x1532
                    && d.product_id() == BLADE_18_2025.pid
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
        "blade_18_2025": {"recognized": recognized, "expected_pid": "02c7", "control_support": "experimental_validated_subset"},
        "interfaces": interfaces,
        "inventory_vendor_ids": ["1532", "1534"],
    });
    if let Some(index) = probe_interface {
        ensure!(recognized, "Getter probe is restricted to the Blade 18 (2025)");
        let info = devices.get(index).context("Interface index is out of range")?;
        ensure!(
            info.vendor_id() == 0x1532 && info.product_id() == BLADE_18_2025.pid,
            "Selected interface is not the Blade 18"
        );
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
        if lighting {
            let mut candidates = serde_json::Map::new();
            // OpenRazer standard LED getters: class 03, RGB 81, effect 82.
            // IDs 04/05 are candidates for logo/backlight, not verified physical
            // RGB zones on this model. Preserve bytes without interpreting colors.
            for (name, cmd, args) in [
                ("led_04_rgb", 0x0381, vec![1, 4, 0, 0, 0]),
                ("led_05_rgb", 0x0381, vec![1, 5, 0, 0, 0]),
                ("led_05_effect", 0x0382, vec![1, 5, 0]),
            ] {
                let result = command::send_command(&device, cmd, &args).and_then(|response| {
                    ensure!(
                        response.get_args()[..2] == args[..2],
                        "LED response selector mismatch"
                    );
                    Ok(response.get_args()[..args.len()].to_vec())
                });
                candidates.insert(
                    name.into(),
                    match result {
                        Ok(bytes) => json!({"ok": true, "response_args": bytes}),
                        Err(error) => json!({"ok": false, "error": format!("{error:#}")}),
                    },
                );
            }
            report["lighting_candidates"] = json!({
                "source": "https://github.com/openrazer/openrazer/blob/master/driver/razerchromacommon.c",
                "physical_zone_mapping_verified": false,
                "getters": candidates,
            });
        }
        if sensors {
            let mut candidates = serde_json::Map::new();
            for (name, cmd, args) in [
                ("controller_firmware", 0x0081, vec![0, 0]),
                ("ec_temperature_fields", 0x0d85, vec![0, 1, 0, 0]),
                ("fan_zone_3_stored_rpm", 0x0d81, vec![0, 3, 0]),
                ("fan_zone_3_actual_rpm", 0x0d88, vec![0, 3, 0]),
            ] {
                let result = command::send_command(&device, cmd, &args).and_then(|response| {
                    if matches!(cmd, 0x0d81 | 0x0d88) {
                        ensure!(response.get_args()[1] == 3, "Third-zone selector mismatch");
                    }
                    if cmd == 0x0d85 {
                        let bytes = response.get_args();
                        ensure!(
                            bytes[0] == 2 && bytes[1] <= 125 && bytes[2] <= 125,
                            "Temperature response has an unreviewed shape or range"
                        );
                    }
                    Ok(response.get_args()[..args.len()].to_vec())
                });
                candidates.insert(
                    name.into(),
                    match result {
                        Ok(bytes) => json!({"ok": true, "response_args": bytes}),
                        Err(error) => json!({"ok": false, "error": format!("{error:#}")}),
                    },
                );
            }
            report["sensor_candidates"] = json!({
                "physical_sensor_mapping_verified": false,
                "temperature_source": "https://github.com/kgswx/R-helper/blob/main/app/Razer/RazerEC.cs",
                "third_fan_note": "Experimental selector 3 on known fan getter commands; no third-fan writes enabled",
                "getters": candidates,
            });
        }
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
