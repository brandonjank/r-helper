//! Standard Windows lighting inventory and a bounded, explicit auxiliary RGB preview.
use anyhow::{ensure, Result};
#[cfg(windows)]
static OWNER: std::sync::Mutex<Option<(u64, windows::Devices::Lights::LampArray)>> =
    std::sync::Mutex::new(None);
#[cfg(windows)]
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn release() {
    #[cfg(windows)]
    if let Ok(mut owner) = OWNER.lock() {
        *owner = None;
    }
}
pub fn preview_active() -> bool {
    #[cfg(windows)]
    {
        OWNER.lock().is_ok_and(|owner| owner.is_some())
    }
    #[cfg(not(windows))]
    false
}
pub fn dynamic_lighting_enabled() -> Option<bool> {
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Lighting")
            .ok()?
            .get_value::<u32, _>("AmbientLightingEnabled")
            .ok()
            .map(|v| v != 0)
    }
    #[cfg(not(windows))]
    None
}
#[cfg(windows)]
pub fn inventory() -> Result<serde_json::Value> {
    use windows::{
        Devices::{Enumeration::DeviceInformation, Lights::LampArray},
        System::VirtualKey,
        Win32::System::Com::*,
    };
    ensure!(crate::diagnostics::is_target_system(), "Lamp inventory is model scoped");
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    let selector = LampArray::GetDeviceSelector()?;
    macro_rules! finish {
        ($operation:expr) => {{
            let operation = $operation?;
            let start = std::time::Instant::now();
            while operation.Status()?.0 == 0 {
                if start.elapsed() > std::time::Duration::from_secs(5) {
                    let _ = operation.Cancel();
                    anyhow::bail!("LampArray enumeration timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            operation.GetResults()?
        }};
    }
    let devices = finish!(DeviceInformation::FindAllAsyncAqsFilter(&selector));
    ensure!(devices.Size()? <= 32, "Unexpectedly large LampArray inventory");
    let mut result = vec![];
    for index in 0..devices.Size()? {
        let info = devices.GetAt(index)?;
        let id = info.Id()?;
        if !id.to_string().to_lowercase().contains("vid_1532&pid_02c7") {
            continue;
        }
        let lamp = finish!(LampArray::FromIdAsync(&id));
        let count = lamp.LampCount()?;
        ensure!((1..=512).contains(&count), "Lamp count outside audited inventory bound");
        let mut lamps = vec![];
        for i in 0..count {
            let item = lamp.GetLampInfo(i)?;
            let p = item.Position()?;
            lamps.push(serde_json::json!({"index":item.Index()?,"purposes":item.Purposes()?.0,"position":[p.X,p.Y,p.Z],"levels":[item.RedLevelCount()?,item.GreenLevelCount()?,item.BlueLevelCount()?,item.GainLevelCount()?]}));
        }
        let mut keys = vec![];
        for key in (0x30..=0x39)
            .chain(0x41..=0x5a)
            .chain(0x60..=0x6f)
            .chain(0x70..=0x87)
            .chain(160..=165)
            .chain([
                8, 9, 13, 16, 17, 18, 19, 20, 27, 32, 33, 34, 35, 36, 37, 38, 39, 40, 44, 45, 46,
                91, 92, 93, 144, 145, 173, 174, 175, 176, 177, 178, 179, 186, 187, 188, 189, 190,
                191, 192, 219, 220, 221, 222, 226,
            ])
        {
            if let Ok(indices) = lamp.GetIndicesForKey(VirtualKey(key)) {
                if !indices.is_empty() {
                    ensure!(
                        indices.iter().all(|i| *i >= 0 && *i < count),
                        "Invalid reported key mapping"
                    );
                    keys.push(serde_json::json!({"virtual_key":key,"indices":indices.as_ref()}));
                }
            }
        }
        result.push(serde_json::json!({"name":info.Name()?.to_string(),"kind":lamp.LampArrayKind()?.0,"lamp_count":count,"connected":lamp.IsConnected()?,"available":lamp.IsAvailable().ok(),"supports_virtual_keys":lamp.SupportsVirtualKeys()?,"lamps":lamps,"key_map":keys}));
    }
    Ok(
        serde_json::json!({"model":"RZ09-0529","read_only":true,"windows_dynamic_lighting":dynamic_lighting_enabled(),"devices":result}),
    )
}
#[cfg(not(windows))]
pub fn inventory() -> Result<serde_json::Value> {
    anyhow::bail!("Windows required")
}

#[cfg(windows)]
pub fn preview_auxiliary(
    rgb: [u8; 3],
    highlight: Option<u8>,
    path: &std::path::Path,
) -> Result<serde_json::Value> {
    use windows::{
        Devices::{
            Enumeration::DeviceInformation,
            Lights::{LampArray, LampPurposes},
        },
        Win32::System::Com::*,
    };
    ensure!(crate::diagnostics::is_target_system(), "Auxiliary lighting preview is model scoped");
    validate_highlight(highlight)?;
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    macro_rules! finish {
        ($operation:expr) => {{
            let operation = $operation?;
            let start = std::time::Instant::now();
            while operation.Status()?.0 == 0 {
                if start.elapsed() > std::time::Duration::from_secs(5) {
                    let _ = operation.Cancel();
                    anyhow::bail!("LampArray timed out; no color sent");
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            operation.GetResults()?
        }};
    }
    let devices =
        finish!(DeviceInformation::FindAllAsyncAqsFilter(&LampArray::GetDeviceSelector()?));
    ensure!(devices.Size()? <= 32, "Unexpected LampArray inventory");
    let mut found = vec![];
    for i in 0..devices.Size()? {
        let id = devices.GetAt(i)?.Id()?;
        if id.to_string().to_lowercase().contains("vid_1532&pid_02c7") {
            found.push(id);
        }
    }
    ensure!(found.len() == 1, "Expected one Blade LampArray");
    let lamp = finish!(LampArray::FromIdAsync(&found[0]));
    ensure!(lamp.LampCount()?==115 && lamp.IsConnected()? && lamp.IsAvailable()?,"Bring R-Helper to the foreground and check Windows Dynamic Lighting priority; no color sent");
    let indices = (97..115).collect::<Vec<_>>();
    for i in 0..115 {
        let info = lamp.GetLampInfo(i)?;
        let purpose = info.Purposes()?;
        ensure!(
            (i < 97 && purpose == LampPurposes::Control)
                || (i >= 97 && purpose == LampPurposes::Branding),
            "Lamp layout changed; no color sent"
        );
        if i >= 97 {
            ensure!(
                info.RedLevelCount()? > 1
                    && info.GreenLevelCount()? > 1
                    && info.BlueLevelCount()? > 1,
                "Auxiliary lamp is not RGB; no color sent"
            );
        }
    }
    let file = std::fs::File::options().create_new(true).write(true).open(path)?;
    serde_json::to_writer_pretty(
        &file,
        &serde_json::json!({"model":"RZ09-0529","operation":"auxiliary_lamp_preview","indices":indices,"requested":rgb,"highlight_lamp":highlight,"duration_seconds":15,"color_readback_available":false,"recovery":"Windows releases this foreground lighting owner on timeout or process exit"}),
    )?;
    file.sync_all()?;
    let mut owner = OWNER.lock().map_err(|_| anyhow::anyhow!("Lamp ownership lock unavailable"))?;
    let mut colors = vec![windows::UI::Color { A: 255, R: rgb[0], G: rgb[1], B: rgb[2] }; 18];
    if let Some(index) = highlight {
        colors[usize::from(index - 1)] = windows::UI::Color { A: 255, R: 255, G: 64, B: 32 };
    }
    lamp.SetColorsForIndices(&colors, &indices)?;
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    *owner = Some((generation, lamp));
    drop(owner);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(15));
        if let Ok(mut owner) = OWNER.lock() {
            if owner.as_ref().is_some_and(|(id, _)| *id == generation) {
                *owner = None;
            }
        }
    });
    Ok(
        serde_json::json!({"indices":indices,"requested":rgb,"highlight_lamp":highlight,"preview_seconds":15,"color_readback_available":false,"foreground_only":true}),
    )
}
#[cfg(not(windows))]
pub fn preview_auxiliary(
    _: [u8; 3],
    _: Option<u8>,
    _: &std::path::Path,
) -> Result<serde_json::Value> {
    anyhow::bail!("Windows required")
}
fn validate_highlight(index: Option<u8>) -> Result<()> {
    ensure!(index.is_none_or(|i| (1..=18).contains(&i)), "Auxiliary lamp must be between 1 and 18");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auxiliary_highlight_cannot_address_keyboard_or_unknown_lamps() {
        assert!(validate_highlight(None).is_ok());
        for i in 1..=18 {
            validate_highlight(Some(i)).unwrap();
        }
        for i in [0, 19, 97, 114, 255] {
            assert!(validate_highlight(Some(i)).is_err());
        }
    }
}
