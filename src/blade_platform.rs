//! On-demand platform research. Native getters run in disposable child processes.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const GETTERS: &[&str] = &[
    "GetDLLVersion",
    "GetBladeLEDPanelProducerShortName",
    "GetBladeScreenResolution",
    "GetBladeFirmwareInfo",
    "GetGPUValueStr",
];
fn save(path: &Path, value: &Value) -> Result<()> {
    let file = std::fs::File::options().create_new(true).write(true).open(path)?;
    serde_json::to_writer_pretty(&file, value)?;
    file.sync_all()?;
    Ok(())
}
fn wait(child: &mut std::process::Child, seconds: u64) -> Result<()> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "Isolated helper exited unsuccessfully");
            return Ok(());
        }
        if start.elapsed() >= Duration::from_secs(seconds) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Isolated helper timed out and was terminated");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn hidden(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
}
pub(crate) fn isolated_getter(name: &str) -> Result<Value> {
    ensure!(GETTERS.contains(&name), "Native getter not permitted");
    let output = crate::blade_profiles::journal_path("platform-read")?;
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.arg("--platform-getter").arg(name).arg(&output);
    hidden(&mut command);
    let mut child = command.spawn()?;
    wait(&mut child, 10)?;
    ensure!(std::fs::metadata(&output)?.len() < 65536, "Native report too large");
    Ok(serde_json::from_slice(&std::fs::read(output)?)?)
}
fn parsed_event(text: &str, event: &str) -> Option<Value> {
    text.lines().rev().find_map(|line|{
        let offset=line.find('{')?;
        let value:Value=serde_json::from_str(&line[offset..]).ok()?;
        if value["dllFunc"]!=event || value["status"]!="SUCCESS" {return None;}
        let mut result=json!({"source":"Last recorded Synapse observation; not a live hardware query","recorded_at":line.get(1..24).unwrap_or("unknown")});
        if event=="GetAllTuningControlData" {
            let controls=value["controlDatas"].as_array()?.iter().take(128).map(|v|json!({"id":v["controlId"],"name":v["controlName"].as_str().filter(|v|v.len()<160),"value":if v["controlValue"].is_number()||v["controlValue"].is_boolean(){v["controlValue"].clone()}else{Value::Null},"enabled":v["controlEnable"].as_bool()})).collect::<Vec<_>>();
            result["controls"]=json!(controls);
        } else {result["incompatibility_flags"]=value["incompatibility"].as_u64().map_or(Value::Null,Value::from);}
        Some(result)
    })
}
fn intel_history() -> Result<Value> {
    let base =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context("Local app data unavailable")?)
            .join("Razer/RazerAppEngine/User Data/Logs");
    let read = |name: &str| -> Result<String> {
        let path = base.join(name);
        ensure!(std::fs::metadata(&path)?.len() < 8 * 1024 * 1024, "Synapse log too large");
        Ok(std::fs::read_to_string(path)?)
    };
    let service = read("RzIntelOverClock.log")?;
    let sdk = read("RzIntelOverClockDLL.log")?;
    let readonly = sdk
        .lines()
        .filter_map(|line| {
            let (id, name) = line.split_once('|')?;
            if !id.chars().all(|c| c.is_ascii_digit()) || !name.ends_with(" : ReadOnlyControl") {
                return None;
            }
            let name = name.strip_suffix(" : ReadOnlyControl")?;
            if name.len() > 160
                || !["Voltage", "Ratio", "Power"].iter().any(|part| name.contains(part))
            {
                return None;
            }
            Some(name.to_owned())
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(128)
        .collect::<Vec<_>>();
    let lock=sdk.lines().rev().find(|line|line.contains("'Overclocking Lock' is ")).map(|line|json!({"recorded_at":line.get(1..24).unwrap_or("unknown"),"locked":line.contains("not disable")}));
    Ok(
        json!({"tuning":parsed_event(&service,"GetAllTuningControlData"),"compatibility":parsed_event(&service,"CheckSystemIncompatibility"),"overclocking_lock_observation":lock,"read_only_controls_seen":readonly,"note":"Historical observations may be stale. UI slider bounds are not stable voltage ranges."}),
    )
}
fn gpu() -> Result<Value> {
    let system =
        PathBuf::from(std::env::var_os("SystemRoot").context("Windows directory unavailable")?)
            .join("System32/nvidia-smi.exe");
    let fields = [
        "name",
        "driver_version",
        "vbios_version",
        "pstate",
        "temperature.gpu",
        "power.draw",
        "power.limit",
        "power.default_limit",
        "power.min_limit",
        "power.max_limit",
        "clocks.max.graphics",
        "clocks.max.memory",
    ];
    let mut command = std::process::Command::new(system);
    command
        .arg(format!("--query-gpu={}", fields.join(",")))
        .arg("--format=csv,noheader,nounits")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    hidden(&mut command);
    let mut child = command.spawn()?;
    wait(&mut child, 10)?;
    let output = child.wait_with_output()?;
    ensure!(output.stdout.len() < 65536, "GPU report too large");
    let text = String::from_utf8(output.stdout)?;
    let mut rows = vec![];
    for line in text.lines().take(8) {
        let values = line.split(',').map(str::trim).collect::<Vec<_>>();
        ensure!(values.len() == fields.len(), "Unexpected GPU query format");
        let mut row = serde_json::Map::new();
        for (name, value) in fields.iter().zip(values) {
            row.insert((*name).into(), json!(value));
        }
        rows.push(Value::Object(row));
    }
    Ok(
        json!({"gpus":rows,"on_demand_only":true,"note":"A diagnostic GPU query can wake the GPU. Numeric limits do not establish writable tuning support."}),
    )
}
fn celsius(raw: f64) -> Option<f64> {
    let value = raw / 10.0 - 273.15;
    (raw.is_finite() && (-20.0..=130.0).contains(&value)).then_some(value)
}
#[cfg(windows)]
fn thermal_zones() -> Result<Value> {
    use windows::{core::PCWSTR, Win32::System::Performance::*};
    struct Query(PDH_HQUERY);
    impl Drop for Query {
        fn drop(&mut self) {
            unsafe { PdhCloseQuery(self.0) };
        }
    }
    let mut query = PDH_HQUERY::default();
    ensure!(
        unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } == 0,
        "Thermal query unavailable"
    );
    let query = Query(query);
    let mut counter = PDH_HCOUNTER::default();
    ensure!(
        unsafe {
            PdhAddEnglishCounterW(
                query.0,
                windows::core::w!("\\Thermal Zone Information(*)\\High Precision Temperature"),
                0,
                &mut counter,
            )
        } == 0,
        "Thermal counters unavailable"
    );
    ensure!(unsafe { PdhCollectQueryData(query.0) } == 0, "Thermal collection failed");
    let (mut bytes, mut count) = (0, 0);
    unsafe { PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, None) };
    ensure!(bytes > 0 && bytes <= 65536 && count <= 64, "Unexpected thermal array size");
    let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
    ensure!(
        unsafe {
            PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut bytes,
                &mut count,
                Some(buffer.as_mut_ptr().cast()),
            )
        } == 0,
        "Thermal array changed; retry"
    );
    ensure!(
        count <= 64
            && count as usize * std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>()
                <= buffer.len() * 8,
        "Invalid thermal count"
    );
    let rows = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
            count as usize,
        )
    };
    let mut zones = vec![];
    for row in rows {
        if row.FmtValue.CStatus > 1 {
            continue;
        }
        let raw = unsafe { row.FmtValue.Anonymous.doubleValue };
        if let Some(value) = celsius(raw) {
            zones.push(json!({"zone":unsafe{row.szName.to_string()}?,"celsius":value,"raw_tenths_kelvin":raw,"role_validated":false}));
        }
    }
    Ok(
        json!({"zones":zones,"source":"Windows Thermal Zone Information / High Precision Temperature","note":"Firmware thermal zones, not validated CPU die, GPU or battery sensors. Not used for curves."}),
    )
}
#[cfg(not(windows))]
fn thermal_zones() -> Result<Value> {
    anyhow::bail!("Windows required")
}
#[cfg(windows)]
fn windows_drivers() -> Result<Value> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let root = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Class")?;
    let mut drivers = vec![];
    for class in [
        "{4d36e968-e325-11ce-bfc1-08002be10318}",
        "{4d36e97d-e325-11ce-bfc1-08002be10318}",
        "{f2e7dd72-6468-4e36-b6f1-6488f42c1b52}",
    ] {
        let Ok(key) = root.open_subkey(class) else {
            continue;
        };
        for name in key
            .enum_keys()
            .filter_map(|v| v.ok())
            .filter(|v| v.len() == 4 && v.chars().all(|c| c.is_ascii_digit()))
        {
            let Ok(entry) = key.open_subkey(name) else {
                continue;
            };
            let Ok(description) = entry.get_value::<String, _>("DriverDesc") else {
                continue;
            };
            if description.len() > 256
                || !["Intel", "NVIDIA", "Razer", "USB4", "Thunderbolt", "Firmware"]
                    .iter()
                    .any(|v| description.contains(v))
            {
                continue;
            }
            let string =
                |name: &str| entry.get_value::<String, _>(name).ok().filter(|v| v.len() < 128);
            drivers.push(json!({"name":description,"version":string("DriverVersion"),"provider":string("ProviderName"),"date":string("DriverDate")}));
            ensure!(drivers.len() <= 128, "Unexpected driver inventory size");
        }
    }
    Ok(
        json!({"installed":drivers,"note":"Installed Windows driver metadata, not a comparison with available updates."}),
    )
}
#[cfg(not(windows))]
fn windows_drivers() -> Result<Value> {
    anyhow::bail!("Windows required")
}
pub fn inspect() -> Result<Value> {
    ensure!(
        librazer::diagnostics::is_target_system(),
        "Platform discovery is scoped to Blade 18 (2025)"
    );
    let mut native = serde_json::Map::new();
    for name in GETTERS {
        native.insert(
            (*name).into(),
            isolated_getter(name).unwrap_or_else(|e| json!({"error":format!("{e:#}")})),
        );
    }
    Ok(
        json!({"read_only":true,"native":native,"thermal_zones":thermal_zones().unwrap_or_else(|e|json!({"error":format!("{e:#}")})),"windows_drivers":windows_drivers().unwrap_or_else(|e|json!({"error":format!("{e:#}")})),"intel_history":intel_history().unwrap_or_else(|e|json!({"error":format!("{e:#}")})),"gpu":gpu().unwrap_or_else(|e|json!({"error":format!("{e:#}")})),"capabilities":capabilities()}),
    )
}
pub fn capabilities() -> Value {
    json!([
        {"name":"Independent static RGB","status":"Available","detail":"Keyboard and underside colors; optional 19-cell underside map. Cell positions need observation."},
        {"name":"Firmware fan curves","status":"Research","detail":"Setter format found; the fixed getter candidate returned unsupported; no validated table backup/readback. Synapse starts a monitoring timer for smart curves."},
        {"name":"CPU voltage / ratios","status":"Requires validation","detail":"Intel native tuning path identified. Last Synapse logs show locked/read-only controls; no established voltage range."},
        {"name":"NVIDIA clock / voltage tuning","status":"Research","detail":"On-demand clocks and power limits available. Timed clock-offset previews use the installed Razer helper and recovery guard; no voltage tuning."},
        {"name":"Panel / GPU switching","status":"Requires restart research","detail":"Panel and graphics getters work. Ordinary Windows refresh switching already available."},
        {"name":"Fn / mute / touchpad indicators","status":"Research","detail":"Model status callers found; native-frame overlay behavior and exact restoration need validation."},
        {"name":"Underside during sleep","status":"Research","detail":"0321 setter found with enable and duration. No recoverable getter identified."},
        {"name":"Battery Saver / USB-C modes","status":"Firmware requirement","detail":"Identified gate requires BIOS 4.01, EC 1.06 and model VBIOS. Check reported installed versions."},
        {"name":"Snap Tap / remapping / M1–M4","status":"Research","detail":"Mapping-engine path and fixed Snap Tap getter identified; no input-filter or remapping writes enabled."},
        {"name":"Battery / board temperatures","status":"Unavailable","detail":"Two additional Windows thermal zones are readable but unmapped; no validated battery sensor. Two controller temperatures are proxies."},
        {"name":"Logical fan 4","status":"Unknown purpose","detail":"Advertised getter returns zero RPM. No fourth physical fan or setter claimed."},
        {"name":"Audio EQ / THX","status":"Separate provider","detail":"Audio-service feature, not a laptop HID control."}
    ])
}
#[cfg(windows)]
fn native_library() -> Result<(libloading::os::windows::Library, String)> {
    use windows::{
        core::{GUID, PCWSTR},
        Win32::{Foundation::HWND, Security::WinTrust::*},
    };
    let base =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context("Local app data unavailable")?)
            .join("Razer/RazerAppEngine/User Data/Apps/Common/bladeCommon");
    let mut files = std::fs::read_dir(&base)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let version = name
                .strip_prefix("blade2Native_v")?
                .strip_suffix(".dll")?
                .split('.')
                .map(str::parse::<u32>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .ok()?;
            (version.len() == 4).then_some((version, e.path()))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let path = files.last().context("Installed Razer native helper unavailable")?.1.clone();
    let wide = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action = GUID::from_u128(0x00aac56b_cd44_11d0_8cc2_00c04fc295ee);
    let verified = unsafe {
        WinVerifyTrust(HWND::default(), &mut action, (&mut data as *mut WINTRUST_DATA).cast())
    };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        WinVerifyTrust(HWND::default(), &mut action, (&mut data as *mut WINTRUST_DATA).cast())
    };
    ensure!(
        verified == 0,
        "Installed native helper failed Windows signature verification: {verified:#x}"
    );
    // Only installed, signature-verified native code, in an isolated process. No initialization or setters.
    let library = unsafe { libloading::os::windows::Library::load_with_flags(&path, 0x1100) }?;
    Ok((library, path.file_name().unwrap_or_default().to_string_lossy().into_owned()))
}
#[cfg(windows)]
fn native_getter(name: &str) -> Result<Value> {
    ensure!(GETTERS.contains(&name), "Native getter not permitted");
    let (library, helper) = native_library()?;
    let function = unsafe {
        library.get::<unsafe extern "system" fn() -> *mut std::ffi::c_char>(name.as_bytes())
    }?;
    let free =
        unsafe { library.get::<unsafe extern "system" fn(*mut std::ffi::c_void)>(b"FreeMalloc") }?;
    let pointer = unsafe { function() };
    ensure!(!pointer.is_null(), "Native getter returned no data");
    let text = unsafe { std::ffi::CStr::from_ptr(pointer) }.to_bytes();
    let result = if text.len() < 65536 {
        String::from_utf8(text.to_vec()).map_err(anyhow::Error::from)
    } else {
        Err(anyhow::anyhow!("Native result too large"))
    };
    unsafe { free(pointer.cast()) };
    let text = result?;
    let value = match name {
        "GetBladeFirmwareInfo" => {
            let source: Value = serde_json::from_str(&text)?;
            let mut result = serde_json::Map::new();
            for key in ["BiosFW", "ECFW", "PDFW", "VBiosFW", "CameraFW"] {
                if let Some(v) = source[key].as_str().filter(|v| v.len() < 128) {
                    result.insert(key.into(), json!(v));
                }
            }
            Value::Object(result)
        }
        "GetGPUValueStr" => {
            let value: Value = serde_json::from_str(&text)?;
            ensure!(value["return"] == 1, "GPU offset getter failed");
            json!({"gpu":value["gpu"].as_i64(),"vram":value["vram"].as_i64()})
        }
        "GetBladeScreenResolution" => {
            let source: Value = serde_json::from_str(&text)?;
            json!({"width":source["ScreenWidth"].as_u64(),"height":source["ScreenHeight"].as_u64()})
        }
        _ => {
            ensure!(
                text.len() < 128 && !text.chars().any(char::is_control),
                "Unexpected native label"
            );
            json!(text)
        }
    };
    Ok(json!({"value":value,"helper":helper,"signature_verified":true}))
}
#[cfg(not(windows))]
fn native_getter(_name: &str) -> Result<Value> {
    anyhow::bail!("Windows required")
}
#[cfg(windows)]
fn native_set_gpu(gpu: i32, vram: i32) -> Result<Value> {
    ensure!(crate::blade_gpu::administrator(), "NVIDIA tuning requires administrator privileges");
    ensure!(
        (-1000..=1000).contains(&gpu) && (-1000..=3000).contains(&vram),
        "GPU restore outside observed driver bounds"
    );
    let (library, _helper) = native_library()?;
    let function =
        unsafe { library.get::<unsafe extern "system" fn(i32, i32) -> i32>(b"SetGPUValue") }?;
    let code = unsafe { function(gpu, vram) };
    let observed = native_getter("GetGPUValueStr")?;
    ensure!(
        observed["value"]["gpu"] == gpu && observed["value"]["vram"] == vram,
        "GPU offset readback mismatch; provider result {code}"
    );
    Ok(observed)
}
#[cfg(not(windows))]
fn native_set_gpu(_gpu: i32, _vram: i32) -> Result<Value> {
    anyhow::bail!("Windows required")
}
pub(crate) fn set_gpu(gpu: i32, vram: i32, path: &Path) -> Result<Value> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.arg("--platform-gpu-set").arg(gpu.to_string()).arg(vram.to_string()).arg(path);
    hidden(&mut command);
    let mut child = command.spawn()?;
    wait(&mut child, 10)?;
    ensure!(std::fs::metadata(path)?.len() < 65536, "GPU result too large");
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
pub fn dispatch() -> Option<Result<()>> {
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1)?;
    if mode != "--platform-getter" && mode != "--platform-inspect" && mode != "--platform-gpu-set" {
        return None;
    }
    Some((|| {
        ensure!(librazer::diagnostics::is_target_system(), "Wrong model for native research");
        if mode == "--platform-gpu-set" {
            ensure!(args.len() == 5, "Expected GPU/memory offsets and output");
            let result = native_set_gpu(args[2].parse()?, args[3].parse()?);
            save(
                Path::new(&args[4]),
                &result.as_ref().cloned().unwrap_or_else(|e| json!({"error":format!("{e:#}")})),
            )?;
            result?;
            return Ok(());
        }
        if mode == "--platform-getter" {
            ensure!(args.len() == 4, "Expected getter and output");
            let result = native_getter(&args[2]);
            save(
                Path::new(&args[3]),
                &result.as_ref().cloned().unwrap_or_else(|e| json!({"error":format!("{e:#}")})),
            )?;
            result?;
        } else {
            ensure!(args.len() == 3, "Expected output");
            save(Path::new(&args[2]), &inspect()?)?;
        }
        Ok(())
    })())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thermal_units_reject_invalid_and_extreme_values() {
        assert!((celsius(3182.0).unwrap() - 45.05).abs() < 0.001);
        assert!(celsius(0.0).is_none());
        assert!(celsius(f64::NAN).is_none());
        assert!(celsius(5000.0).is_none());
    }
    #[test]
    fn recorded_tuning_is_labelled_historical_and_excludes_unknown_fields() {
        let text = r#"[2026-10-14 05:37:38.720] reply: {"dllFunc":"GetAllTuningControlData","status":"SUCCESS","serial":"private","controlDatas":[{"controlId":48,"controlName":"Power","controlValue":150,"controlEnable":true,"serial":"private"}]}"#;
        let value = parsed_event(text, "GetAllTuningControlData").unwrap();
        assert_eq!(value["controls"][0]["value"], 150);
        assert!(value["source"].as_str().unwrap().contains("not a live"));
        assert!(!value.to_string().contains("private"));
        assert!(parsed_event(text, "CheckSystemIncompatibility").is_none());
    }
    #[test]
    fn native_probe_has_no_initializer_setter_or_hanging_driver_getter() {
        assert!(GETTERS.iter().all(|name| name.starts_with("Get")));
        assert!(!GETTERS.contains(&"GetBladeDriverInfo"));
        assert!(!GETTERS.contains(&"DeviceInit"));
    }
}
