use anyhow::{ensure, Context, Result};
use librazer::blade18::Profile;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema: u8,
    pub ac: Option<Profile>,
    pub battery: Option<Profile>,
    pub automatic: bool,
    #[serde(default)]
    pub display_ac: Option<librazer::display::Profile>,
    #[serde(default)]
    pub display_battery: Option<librazer::display::Profile>,
    #[serde(default)]
    pub lighting_ac: Option<librazer::blade18::lighting::Preset>,
    #[serde(default)]
    pub lighting_battery: Option<librazer::blade18::lighting::Preset>,
    #[serde(default)]
    pub named: Vec<Named>,
    #[serde(default)]
    pub application_switching: bool,
    #[serde(default)]
    pub lighting_policy: LightingPolicy,
    #[serde(default)]
    pub manage_razer_software: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Named {
    pub name: String,
    pub hardware: Profile,
    pub lighting: librazer::blade18::lighting::Preset,
    pub display: librazer::display::Profile,
    #[serde(default)]
    pub executable: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightingPolicy {
    pub enabled: bool,
    pub idle_ac_seconds: u32,
    pub idle_battery_seconds: u32,
    pub display_off: bool,
    pub low_battery_percent: u8,
    #[serde(default)]
    pub restore_on_resume: bool,
}
pub fn journal_path(kind: &str) -> Result<PathBuf> {
    let dir = path()?.parent().unwrap().join("changes");
    fs::create_dir_all(&dir)?;
    Ok(dir.join(format!(
        "{kind}-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()
    )))
}
static OPERATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub fn try_operation_lock() -> Result<std::sync::MutexGuard<'static, ()>> {
    OPERATION.try_lock().map_err(|_| {
        anyhow::anyhow!("Another device operation is in progress; retry after it finishes")
    })
}
pub fn operation_lock() -> Result<std::sync::MutexGuard<'static, ()>> {
    OPERATION.lock().map_err(|_| anyhow::anyhow!("Device operation lock failed"))
}
impl Named {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty() && self.name.chars().count() <= 60,
            "Profile name must contain 1–60 characters"
        );
        ensure!(
            self.executable.len() <= 128
                && !self.executable.contains(['\\', '/', ':'])
                && (self.executable.is_empty()
                    || self.executable.to_ascii_lowercase().ends_with(".exe")),
            "Application must be an executable name, such as game.exe"
        );
        self.hardware.validate(false)?;
        self.lighting.validate()?;
        self.display.validate()?;
        Ok(())
    }
    pub fn capture(name: String) -> Result<Self> {
        let state = librazer::blade18::status()?;
        Ok(Self {
            name,
            lighting: librazer::blade18::lighting::Preset::capture(
                &librazer::blade18::lighting::status()?,
                state.logo,
            )?,
            hardware: Profile::capture(&state),
            display: librazer::display::inventory()?.current,
            executable: String::new(),
        })
    }
    pub fn apply(&self) -> Result<librazer::blade18::State> {
        self.validate()?;
        self.hardware.validate(!librazer::blade18::ac_connected()?)?;
        let before = Self::capture("Recovery".into())?;
        let mut recovery = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(journal_path("profile-recovery")?)?;
        serde_json::to_writer_pretty(
            &mut recovery,
            &serde_json::json!({"model":"RZ09-0529","operation":"named_profile","original":before,"requested":self}),
        )?;
        recovery.sync_all()?;
        let result = (|| {
            apply(&self.hardware, Some(&self.display))?;
            self.lighting.apply(&journal_path("named-lighting")?)?;
            librazer::blade18::status()
        })();
        if result.is_err() {
            let hardware = apply(&before.hardware, Some(&before.display));
            let lighting = before.lighting.apply(&journal_path("named-recovery")?);
            ensure!(hardware.is_ok() && lighting.is_ok(),"Profile failed and recovery needs attention: hardware={hardware:?}, lighting={lighting:?}");
        }
        result
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: 1,
            ac: None,
            battery: None,
            automatic: false,
            display_ac: None,
            display_battery: None,
            lighting_ac: None,
            lighting_battery: None,
            named: vec![],
            application_switching: false,
            lighting_policy: LightingPolicy::default(),
            manage_razer_software: false,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "Unknown settings schema");
        ensure!(self.named.len() <= 32, "At most 32 named profiles");
        let mut names = std::collections::HashSet::new();
        let mut apps = std::collections::HashSet::new();
        for p in &self.named {
            p.validate()?;
            ensure!(names.insert(p.name.trim().to_lowercase()), "Duplicate profile name");
            if !p.executable.is_empty() {
                ensure!(
                    apps.insert(p.executable.to_lowercase()),
                    "Application is assigned to multiple profiles"
                );
            }
        }
        for p in [&self.lighting_ac, &self.lighting_battery].into_iter().flatten() {
            p.validate()?;
        }
        ensure!(
            self.lighting_policy.idle_ac_seconds <= 86400
                && self.lighting_policy.idle_battery_seconds <= 86400
                && self.lighting_policy.low_battery_percent <= 100,
            "Invalid lighting policy"
        );
        for profile in [&self.display_ac, &self.display_battery].into_iter().flatten() {
            profile.validate()?;
        }
        if let Some(profile) = &self.ac {
            profile.validate(false)?;
        }
        if let Some(profile) = &self.battery {
            profile.validate(true)?;
        }
        ensure!(
            !self.automatic || (self.ac.is_some() && self.battery.is_some()),
            "Save both profiles before enabling automatic switching"
        );
        Ok(())
    }
}
pub fn apply(
    profile: &Profile,
    display: Option<&librazer::display::Profile>,
) -> Result<librazer::blade18::State> {
    let before = if let Some(target) = display {
        let before = librazer::display::inventory()?.current;
        librazer::display::apply(target)?;
        Some(before)
    } else {
        None
    };
    let result = librazer::blade18::apply_profile(profile);
    if result.is_err() {
        if let Some(before) = before {
            librazer::display::apply(&before)
                .context("Display rollback after power-profile failure failed")?;
        }
    }
    result
}
pub fn path() -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?)
        .join("RHelper")
        .join("blade18.json"))
}
pub fn load(path: &Path) -> Result<Settings> {
    if let Ok(metadata) = fs::metadata(path) {
        ensure!(metadata.len() <= 2_097_152, "Settings file is too large; original preserved");
    }
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(e.into()),
    };
    ensure!(data.len() <= 2_097_152, "Settings file is too large; original preserved");
    let settings: Settings =
        serde_json::from_slice(&data).context("Settings are invalid; original file preserved")?;
    settings.validate()?;
    Ok(settings)
}
pub fn import(data: &str) -> Result<Settings> {
    ensure!(data.len() <= 2_097_152, "Import is too large");
    let mut settings: Settings = serde_json::from_str(data)?;
    settings.validate()?;
    settings.automatic = false;
    settings.application_switching = false;
    settings.lighting_policy.enabled = false;
    settings.manage_razer_software = false;
    Ok(settings)
}
pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    settings.validate()?;
    fs::create_dir_all(path.parent().context("No settings directory")?)?;
    let temporary = path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()
    ));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    serde_json::to_writer_pretty(&mut file, settings)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    Ok(())
}
/// No startup application, no guesses on unknown power source, no repeated enforcement.
pub fn transition(previous: Option<bool>, current: Option<bool>, enabled: bool) -> Option<bool> {
    if enabled && previous.is_some() && current.is_some() && previous != current {
        current
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn named() -> Named {
        Named {
            name: "Test".into(),
            hardware: Profile {
                brightness: 176,
                logo: librazer::types::LogoMode::Static,
                battery_care: librazer::types::BatteryCare::Enable,
                performance: librazer::types::PerfMode::Balanced,
                cpu: Some(librazer::types::CpuBoost::Medium),
                gpu: Some(librazer::types::GpuBoost::Low),
            },
            lighting: librazer::blade18::lighting::Preset {
                speed: None,
                style: librazer::blade18::lighting::Style::Static,
                color: [32, 64, 255],
                second: [255, 64, 32],
                keys: vec![[32, 64, 255]; 114],
                brightness: 176,
                underside: false,
                underside_brightness: 64,
                underside_color: None,
                logo: librazer::types::LogoMode::Static,
            },
            display: librazer::display::Profile {
                panel: "test-panel".into(),
                width: 2560,
                height: 1600,
                hz: 60,
            },
            executable: "game.exe".into(),
        }
    }
    #[test]
    fn independent_underside_profiles_roundtrip_and_reject_animated_keyboard() {
        let mut p = named();
        p.lighting.underside_color = Some([255, 0, 0]);
        p.lighting.underside = true;
        let encoded = serde_json::to_vec(&p).unwrap();
        let decoded: Named = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.lighting.underside_color, Some([255, 0, 0]));
        decoded.lighting.validate().unwrap();
        p.lighting.style = librazer::blade18::lighting::Style::WaveLeft;
        assert!(p.lighting.validate().is_err());
        let mut legacy = serde_json::to_value(named()).unwrap();
        legacy["lighting"].as_object_mut().unwrap().remove("underside_color");
        let legacy: Named = serde_json::from_value(legacy).unwrap();
        assert!(legacy.lighting.underside_color.is_none());
    }
    #[test]
    fn imported_profiles_cannot_enable_automation_and_maximum_export_roundtrips() {
        let p = named();
        let mut settings = Settings {
            ac: Some(p.hardware.clone()),
            battery: Some(p.hardware.clone()),
            automatic: true,
            application_switching: true,
            ..Default::default()
        };
        settings.lighting_policy.enabled = true;
        settings.manage_razer_software = true;
        for i in 0..32 {
            let mut p = p.clone();
            p.name = format!("Profile {i}");
            p.executable = format!("game{i}.exe");
            settings.named.push(p);
        }
        let imported = import(&serde_json::to_string_pretty(&settings).unwrap()).unwrap();
        assert_eq!(imported.named.len(), 32);
        assert!(
            !imported.automatic
                && !imported.application_switching
                && !imported.lighting_policy.enabled
                && !imported.manage_razer_software
        );
        settings.named.push(named());
        assert!(settings.validate().is_err());
    }
    #[test]
    fn profiles_reject_ambiguous_names_apps_and_malformed_maps() {
        let p = named();
        let mut settings = Settings { named: vec![p.clone(), p.clone()], ..Default::default() };
        assert!(settings.validate().is_err());
        settings.named[1].name = "Other".into();
        assert!(settings.validate().is_err());
        settings.named[1].executable = "other.exe".into();
        assert!(settings.validate().is_ok());
        settings.named[1].lighting.keys.pop();
        assert!(settings.validate().is_err());
        let mut p = p;
        p.executable = "C:\\game.exe".into();
        assert!(p.validate().is_err());
        assert!(import("{\"schema\":99}").is_err());
        assert!(import(&" ".repeat(2_097_153)).is_err());
    }
    #[test]
    fn old_profiles_leave_display_unchanged_and_invalid_display_is_rejected() {
        let mut settings: Settings =
            serde_json::from_str(r#"{"schema":1,"ac":null,"battery":null,"automatic":false}"#)
                .unwrap();
        assert!(settings.display_ac.is_none() && settings.display_battery.is_none());
        settings.display_ac = Some(librazer::display::Profile {
            panel: "panel".into(),
            width: 2560,
            height: 1600,
            hz: 0,
        });
        assert!(settings.validate().is_err());
    }
    #[test]
    fn power_transitions_ignore_startup_unknown_and_disabled_events() {
        assert_eq!(transition(None, Some(true), true), None);
        assert_eq!(transition(Some(true), None, true), None);
        assert_eq!(transition(Some(true), Some(false), false), None);
        assert_eq!(transition(Some(true), Some(true), true), None);
        assert_eq!(transition(Some(true), Some(false), true), Some(false));
    }
    #[test]
    fn settings_roundtrip_and_corruption_preservation() {
        let path =
            std::env::temp_dir().join(format!("rhelper-profile-test-{}.json", std::process::id()));
        save(&path, &Settings::default()).unwrap();
        assert!(!load(&path).unwrap().automatic);
        fs::write(&path, b"invalid").unwrap();
        assert!(load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid");
        fs::remove_file(path).unwrap();
    }
}
