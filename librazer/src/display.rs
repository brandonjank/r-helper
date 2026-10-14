//! Driver-listed refresh rates for one uniquely identified internal panel.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub panel: String,
    pub width: u32,
    pub height: u32,
    pub hz: u32,
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.panel.is_empty() && self.panel.len() <= 512, "Invalid panel identity");
        ensure!(
            (640..=16384).contains(&self.width)
                && (480..=16384).contains(&self.height)
                && (24..=1000).contains(&self.hz),
            "Invalid display profile"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Info {
    pub current: Profile,
    pub rates: Vec<u32>,
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        core::PCWSTR,
        Win32::{Devices::Display::*, Graphics::Gdi::*},
    };
    fn text(raw: &[u16]) -> String {
        String::from_utf16_lossy(&raw[..raw.iter().position(|v| *v == 0).unwrap_or(raw.len())])
    }
    fn mode(name: PCWSTR, index: ENUM_DISPLAY_SETTINGS_MODE) -> Result<DEVMODEW> {
        let mut mode = DEVMODEW::default();
        mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        ensure!(
            unsafe { EnumDisplaySettingsW(name, index, &mut mode) }.as_bool(),
            "Cannot read display mode"
        );
        Ok(mode)
    }
    fn panel() -> Result<(String, Vec<u16>)> {
        for _ in 0..3 {
            let (mut count, mut modes) = (0, 0);
            unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut count, &mut modes) }
                .ok()?;
            ensure!(count <= 64 && modes <= 256, "Unexpected display topology size");
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); count as usize];
            let mut mode_info = vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize];
            if unsafe {
                QueryDisplayConfig(
                    QDC_ONLY_ACTIVE_PATHS,
                    &mut count,
                    paths.as_mut_ptr(),
                    &mut modes,
                    mode_info.as_mut_ptr(),
                    None,
                )
            }
            .is_err()
            {
                continue;
            }
            paths.truncate(count as usize);
            let all = paths;
            let candidates: Vec<_> = all
                .iter()
                .filter(|p| {
                    matches!(
                        p.targetInfo.outputTechnology,
                        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
                            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
                            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS
                    )
                })
                .collect();
            ensure!(candidates.len() == 1, "Expected exactly one active internal panel");
            let path = candidates[0];
            ensure!(
                all.iter()
                    .filter(|p| p.sourceInfo.id == path.sourceInfo.id
                        && p.sourceInfo.adapterId.HighPart == path.sourceInfo.adapterId.HighPart
                        && p.sourceInfo.adapterId.LowPart == path.sourceInfo.adapterId.LowPart)
                    .count()
                    == 1,
                "Refresh changes are disabled for a cloned display source"
            );
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: std::mem::size_of_val(&source) as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            };
            ensure!(
                unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } == 0,
                "Cannot identify internal display source"
            );
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            target.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: std::mem::size_of_val(&target) as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            };
            ensure!(
                unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } == 0,
                "Cannot identify internal panel"
            );
            let identity = text(&target.monitorDevicePath);
            ensure!(!identity.is_empty(), "Panel identity is unavailable");
            let mut name = source.viewGdiDeviceName.to_vec();
            if !name.contains(&0) {
                name.push(0);
            }
            return Ok((identity, name));
        }
        anyhow::bail!("Display topology changed during enumeration; retry")
    }
    fn compatible(a: &DEVMODEW, b: &DEVMODEW) -> bool {
        a.dmPelsWidth == b.dmPelsWidth
            && a.dmPelsHeight == b.dmPelsHeight
            && a.dmBitsPerPel == b.dmBitsPerPel
            && unsafe {
                a.Anonymous1.Anonymous2.dmDisplayOrientation
                    == b.Anonymous1.Anonymous2.dmDisplayOrientation
                    && a.Anonymous2.dmDisplayFlags == b.Anonymous2.dmDisplayFlags
            }
    }
    pub fn inventory() -> Result<Info> {
        let (panel, name) = panel()?;
        let name = PCWSTR(name.as_ptr());
        let current = mode(name, ENUM_CURRENT_SETTINGS)?;
        let mut rates = Vec::new();
        for index in 0..4096 {
            let Ok(candidate) = mode(name, ENUM_DISPLAY_SETTINGS_MODE(index)) else { break };
            if compatible(&current, &candidate)
                && (24..=1000).contains(&candidate.dmDisplayFrequency)
            {
                let mut trial = current;
                trial.dmDisplayFrequency = candidate.dmDisplayFrequency;
                trial.dmFields = DM_DISPLAYFREQUENCY;
                if unsafe { ChangeDisplaySettingsExW(name, Some(&trial), None, CDS_TEST, None) }
                    == DISP_CHANGE_SUCCESSFUL
                {
                    rates.push(candidate.dmDisplayFrequency);
                }
            }
        }
        rates.sort_unstable();
        rates.dedup();
        let current = Profile {
            panel,
            width: current.dmPelsWidth,
            height: current.dmPelsHeight,
            hz: current.dmDisplayFrequency,
        };
        current.validate()?;
        ensure!(
            rates.contains(&current.hz),
            "Current rate is not in the supported same-resolution list"
        );
        Ok(Info { current, rates })
    }
    pub fn apply(profile: &Profile) -> Result<Info> {
        profile.validate()?;
        let before = inventory()?;
        ensure!(
            before.current.panel == profile.panel
                && before.current.width == profile.width
                && before.current.height == profile.height,
            "Panel or resolution changed; refresh the profile before applying"
        );
        ensure!(
            before.rates.contains(&profile.hz),
            "Refresh rate is no longer supported at this resolution"
        );
        let (identity, name) = panel()?;
        ensure!(identity == profile.panel, "Panel changed during refresh selection");
        let name = PCWSTR(name.as_ptr());
        let original = mode(name, ENUM_CURRENT_SETTINGS)?;
        ensure!(
            original.dmPelsWidth == profile.width && original.dmPelsHeight == profile.height,
            "Resolution changed before apply"
        );
        if original.dmDisplayFrequency == profile.hz {
            return Ok(before);
        }
        let mut requested = original;
        requested.dmFields = DM_DISPLAYFREQUENCY;
        requested.dmDisplayFrequency = profile.hz;
        ensure!(
            unsafe { ChangeDisplaySettingsExW(name, Some(&requested), None, CDS_TEST, None) }
                == DISP_CHANGE_SUCCESSFUL,
            "Driver rejected the refresh rate test"
        );
        let result = (|| {
            let code = unsafe {
                ChangeDisplaySettingsExW(name, Some(&requested), None, CDS_TYPE(0), None)
            };
            ensure!(code == DISP_CHANGE_SUCCESSFUL, "Display apply failed: {}", code.0);
            std::thread::sleep(std::time::Duration::from_millis(500));
            let after = inventory()?;
            ensure!(after.current == *profile, "Display refresh readback mismatch");
            Ok(after)
        })();
        if result.is_err() {
            let mut restore = original;
            restore.dmFields = DM_DISPLAYFREQUENCY;
            ensure!(
                unsafe { ChangeDisplaySettingsExW(name, Some(&restore), None, CDS_TYPE(0), None) }
                    == DISP_CHANGE_SUCCESSFUL,
                "Display rollback failed"
            );
            ensure!(inventory()?.current == before.current, "Display rollback readback mismatch");
        }
        result
    }
}
pub fn inventory() -> Result<Info> {
    #[cfg(windows)]
    {
        native::inventory()
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("Windows is required")
    }
}
pub fn apply(profile: &Profile) -> Result<Info> {
    ensure!(
        crate::diagnostics::is_target_system(),
        "Display writes require the development Blade 18 model"
    );
    #[cfg(windows)]
    let _lock = {
        use std::os::windows::fs::OpenOptionsExt;
        let mut lock = None;
        for _ in 0..50 {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .share_mode(0)
                .open(std::env::temp_dir().join("rhelper-blade18-display.lock"))
            {
                Ok(file) => {
                    lock = Some(file);
                    break;
                }
                Err(e) if e.raw_os_error() == Some(32) => {
                    std::thread::sleep(std::time::Duration::from_millis(100))
                }
                Err(e) => return Err(e.into()),
            }
        }
        lock.context("Another display operation is active; retry")?
    };
    #[cfg(windows)]
    {
        native::apply(profile)
    }
    #[cfg(not(windows))]
    {
        let _ = profile;
        anyhow::bail!("Windows is required")
    }
}
pub fn restore(path: &std::path::Path) -> Result<Info> {
    let record: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        record["operation"] == "internal_panel_refresh" && record["model"] == "RZ09-0529",
        "Not a Blade 18 display snapshot"
    );
    let profile: Profile =
        serde_json::from_value(record["original"].clone()).context("Invalid display snapshot")?;
    apply(&profile)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_saved_modes_without_hardware() {
        let mut p = Profile { panel: "panel".into(), width: 3840, height: 2400, hz: 240 };
        assert!(p.validate().is_ok());
        p.hz = 0;
        assert!(p.validate().is_err());
        p.hz = 2000;
        assert!(p.validate().is_err());
        p.hz = 60;
        p.panel.clear();
        assert!(p.validate().is_err());
    }
}
