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
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "Unknown settings schema");
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
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(e.into()),
    };
    let settings: Settings =
        serde_json::from_slice(&data).context("Settings are invalid; original file preserved")?;
    settings.validate()?;
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
