//! Source-backed controls kept separate from ordinary profiles until behavior is verified.
use super::{c, open};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Control {
    FunctionKeys,
    PersistentFunctionKeys,
    GamingKeys,
    ForceChroma,
}
impl Control {
    pub const ALL: [Self; 4] =
        [Self::FunctionKeys, Self::PersistentFunctionKeys, Self::GamingKeys, Self::ForceChroma];
    pub fn label(self) -> &'static str {
        match self {
            Self::FunctionKeys => "Function keys · current session",
            Self::PersistentFunctionKeys => "Function keys · firmware profile",
            Self::GamingKeys => "Gaming mode",
            Self::ForceChroma => "Firmware Chroma control · experimental",
        }
    }
    pub fn getter(self) -> (u16, Vec<u8>) {
        match self {
            Self::FunctionKeys => (0x0286, vec![0, 0]),
            Self::PersistentFunctionKeys => (0x0286, vec![1, 0]),
            Self::GamingKeys => (0x0088, vec![0; 4]),
            Self::ForceChroma => (0x0f90, vec![0]),
        }
    }
    pub fn validate(self, value: &[u8]) -> Result<()> {
        let valid = match self {
            Self::FunctionKeys => matches!(value, [0, 0 | 1]),
            Self::PersistentFunctionKeys => matches!(value, [1, 0 | 1]),
            Self::GamingKeys => matches!(value, [0 | 1, 0, 0, 0]),
            Self::ForceChroma => matches!(value, [0 | 1]),
        };
        ensure!(valid, "Unverified {} value: {value:?}", self.label());
        Ok(())
    }
    pub fn read(self) -> Result<Vec<u8>> {
        let (command, args) = self.getter();
        let packet = c::send_command(&open()?, command, &args)?;
        ensure!(
            packet.payload().len() == args.len(),
            "Unexpected {} response length",
            self.label()
        );
        let value = packet.payload().to_vec();
        self.validate(&value)?;
        Ok(value)
    }
    fn write(self, value: &[u8]) -> Result<()> {
        self.validate(value)?;
        let command = match self {
            Self::FunctionKeys | Self::PersistentFunctionKeys => 0x0206,
            Self::GamingKeys => 0x0008,
            Self::ForceChroma => 0x0f10,
        };
        c::send_command(&open()?, command, value)?;
        let observed = self.read()?;
        ensure!(
            observed == value,
            "{} readback mismatch: requested {value:?}, observed {observed:?}",
            self.label()
        );
        Ok(())
    }
    pub fn apply(self, value: &[u8], path: &Path) -> Result<Vec<u8>> {
        self.validate(value)?;
        super::lamp_array::release();
        let mut file = std::fs::File::options().create_new(true).write(true).open(path)?;
        let original = self.read()?;
        super::journal(
            &mut file,
            &serde_json::json!({"model":"RZ09-0529","control":self,"original":original,"requested":value}),
        )?;
        let result = self.write(value);
        let rollback = if result.is_err() { Some(self.write(&original)) } else { None };
        super::journal(
            &mut file,
            &serde_json::json!({"model":"RZ09-0529","control":self,"original":original,"requested":value,"error":result.as_ref().err().map(|e|format!("{e:#}")),"rollback":rollback.as_ref().map(|v|v.as_ref().map(|_|true).map_err(|e|format!("{e:#}")))}),
        )?;
        if let Some(Err(error)) = rollback {
            return Err(error.context("Extended control recovery failed"));
        }
        result?;
        Ok(value.to_vec())
    }
    pub fn test_value(self, original: &[u8]) -> Result<Vec<u8>> {
        self.validate(original)?;
        let mut value = original.to_vec();
        match self {
            Self::FunctionKeys | Self::PersistentFunctionKeys => value[1] ^= 1,
            Self::GamingKeys => value[0] = if value[0] == 0 { 1 } else { 0 },
            Self::ForceChroma => value[0] ^= 1,
        }
        Ok(value)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub model: String,
    pub values: Vec<(Control, Vec<u8>)>,
}
impl Snapshot {
    pub fn capture() -> Result<Self> {
        Ok(Self {
            model: "RZ09-0529".into(),
            values: Control::ALL.into_iter().map(|c| Ok((c, c.read()?))).collect::<Result<_>>()?,
        })
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.model == "RZ09-0529" && self.values.len() == Control::ALL.len(),
            "Invalid control recovery"
        );
        for (index, (control, value)) in self.values.iter().enumerate() {
            ensure!(*control == Control::ALL[index], "Recovery control order mismatch");
            control.validate(value)?;
        }
        Ok(())
    }
    pub fn restore(&self) -> Result<()> {
        self.validate()?;
        let mut errors = vec![];
        for (control, value) in &self.values {
            if let Err(e) = control.write(value) {
                errors.push(format!("{}: {e:#}", control.label()));
            }
        }
        ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }
}
pub fn status() -> serde_json::Value {
    serde_json::Value::Array(Control::ALL.into_iter().map(|control|match control.read(){Ok(value)=>serde_json::json!({"control":control,"label":control.label(),"value":value}),Err(e)=>serde_json::json!({"control":control,"label":control.label(),"error":format!("{e:#}")})}).collect())
}
pub fn telemetry() -> serde_json::Value {
    let result = (|| -> Result<serde_json::Value> {
        let device = open()?;
        let adapter = c::send_command(&device, 0x078c, &[0, 0])?;
        let raw = &adapter.get_args()[..2];
        let watts = |v: u8| -> Option<u16> {
            match v {
                0 => Some(0),
                1 => Some(40),
                2 => Some(45),
                3 => Some(60),
                4 => Some(65),
                5 => Some(80),
                6 => Some(90),
                7 => Some(100),
                8 => Some(120),
                9 => Some(130),
                10 => Some(150),
                11 => Some(180),
                12 => Some(200),
                13 => Some(230),
                14 => Some(250),
                15 => Some(310),
                17 => Some(280),
                18 => Some(330),
                19 => Some(400),
                _ => None,
            }
        };
        let flags = c::send_command(&device, 0x078f, &[0])?.get_args()[0];
        let sku = c::send_command(&device, 0x0d8f, &[0])?.get_args()[0];
        Ok(
            serde_json::json!({"adapter_class_watts":watts(raw[0]),"recommended_adapter_class_watts":watts(raw[1]),"adapter_raw":raw,"shared_power_flags":flags,"sku_flags":sku,"dds_advertised":sku&1!=0,"mini_led_resolution_advertised":sku&2!=0}),
        )
    })();
    match result {
        Ok(value) => value,
        Err(e) => serde_json::json!({"error":format!("{e:#}")}),
    }
}
pub fn inventory() -> Result<serde_json::Value> {
    use serde_json::json;
    let device = crate::diagnostics::open_blade18_diagnostic()?;
    let mut readings = serde_json::Map::new();
    for (name, command, data) in [
        ("device_mode", 0x0084, vec![0, 0]),
        ("legacy_led_inventory", 0x0389, vec![0; 80]),
        ("gaming_key_flags", 0x0088, vec![0; 4]),
        ("fn_primary_volatile", 0x0286, vec![0, 0]),
        ("fn_primary_persistent", 0x0286, vec![1, 0]),
        ("function_key_swap", 0x02a8, vec![0]),
        ("keyboard_layout", 0x02a4, vec![0]),
        ("gaming_custom_keys", 0x00dd, vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        ("gaming_key_selection", 0x00da, vec![1, 0, 0]),
        ("fn_lighting_overlay", 0x0f86, vec![1, 0]),
        ("force_chroma", 0x0f90, vec![0]),
        ("lighting_quick_switch", 0x0f94, vec![1, 0]),
        ("maximum_brightness_on_battery", 0x0782, vec![0]),
        ("firmware_sleep_on_battery", 0x0783, vec![0, 0]),
        ("firmware_auto_sleep", 0x0788, vec![0, 0]),
        ("firmware_dim_timeout", 0x078b, vec![0; 5]),
        ("indicator_configuration", 0x0790, vec![0]),
        ("low_power_state", 0x0795, vec![0]),
        ("smart_illumination", 0x0793, vec![1, 0, 0]),
    ] {
        let result = crate::command::send_command(&device, command, &data);
        readings.insert(name.into(), match result {
            Ok(packet) => json!({"ok":true,"command":format!("{command:04x}"),"request":data,"response":packet.get_args(),"packet":packet}),
            Err(error) => json!({"ok":false,"command":format!("{command:04x}"),"request":data,"error":format!("{error:#}")}),
        });
    }
    Ok(json!({"model":"RZ09-0529","getters":readings}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_reject_unknown_bits_profiles_and_modes() {
        assert!(Control::FunctionKeys.validate(&[1, 0]).is_err());
        assert!(Control::GamingKeys.validate(&[16, 0, 0, 0]).is_err());
        assert!(Control::GamingKeys.validate(&[3, 1, 0, 0]).is_err());
        assert!(Control::ForceChroma.validate(&[2]).is_err());
        for c in Control::ALL {
            let (_, v) = c.getter();
            c.validate(&v).unwrap();
            c.validate(&c.test_value(&v).unwrap()).unwrap();
        }
    }
    #[test]
    fn recovery_requires_complete_unique_known_controls() {
        let mut s = Snapshot {
            model: "RZ09-0529".into(),
            values: Control::ALL.into_iter().map(|c| (c, c.getter().1)).collect(),
        };
        s.validate().unwrap();
        s.values[1].0 = Control::FunctionKeys;
        assert!(s.validate().is_err());
        s.values.clear();
        assert!(s.validate().is_err());
    }
}
