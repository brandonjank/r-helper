//! Reversible vendor-control ownership. Never changes service startup types.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const SERVICES: &[&str] = &[
    "Razer Game Manager Service 3",
    "Razer Game Manager Service",
    "RGS",
    "Razer Synapse Service",
    "Razer Synapse Service 3",
    "Razer Central Service",
    "RzActionSvc",
    "Razer Chroma SDK Service",
    "Razer Chroma SDK Server",
    "Razer Chroma Stream Server",
];
const APPS: &[&str] = &[
    "RazerAppEngine.exe",
    "Razer Synapse 3.exe",
    "Razer Synapse Service.exe",
    "Razer Synapse Service Process.exe",
    "Razer Central.exe",
    "RazerCentral.exe",
    "RzSDKServer.exe",
    "RzSDKService.exe",
    "RzChromaStreamServer.exe",
];
fn app_allowed(path: &Path) -> bool {
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
    let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let roots = [
        std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into()),
        std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| "C:\\Program Files (x86)".into()),
    ];
    APPS.iter().any(|n| name.eq_ignore_ascii_case(n))
        && roots.iter().any(|r| text.starts_with(&format!("{}\\razer\\", r.to_lowercase())))
        && !text.split('\\').any(|c| c == "..")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    model: String,
    services: Vec<String>,
    controlled_services: Vec<String>,
    apps: Vec<PathBuf>,
}
impl Record {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.model == "RZ09-0529"
                && self.services.len() <= SERVICES.len()
                && self.controlled_services.len() <= SERVICES.len()
                && self.apps.len() <= 32,
            "Invalid vendor recovery"
        );
        let mut names = std::collections::HashSet::new();
        for name in &self.services {
            ensure!(
                SERVICES.contains(&name.as_str()) && names.insert(name),
                "Unapproved/duplicate service"
            );
        }
        let mut paths = std::collections::HashSet::new();
        let mut controlled = std::collections::HashSet::new();
        for name in &self.controlled_services {
            ensure!(
                SERVICES.contains(&name.as_str()) && controlled.insert(name),
                "Unapproved/duplicate managed service"
            );
        }
        ensure!(
            self.services.iter().all(|name| self.controlled_services.contains(name)),
            "Recovery omits an original service"
        );
        for path in &self.apps {
            ensure!(
                app_allowed(path) && paths.insert(path),
                "Unapproved/duplicate vendor executable"
            );
        }
        Ok(())
    }
}
#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{
            Foundation::*,
            System::{Diagnostics::ToolHelp::*, Services::*, Threading::*},
        },
    };
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    pub struct Handle(pub HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    struct Service(SC_HANDLE);
    impl Drop for Service {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseServiceHandle(self.0);
            }
        }
    }
    fn manager() -> Result<Service> {
        Ok(Service(unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }?))
    }
    fn service(name: &str, access: u32) -> Result<Service> {
        let manager = manager()?;
        Ok(Service(unsafe { OpenServiceW(manager.0, PCWSTR(wide(name).as_ptr()), access) }?))
    }
    fn state(handle: &Service) -> Result<SERVICE_STATUS_PROCESS> {
        let mut value = SERVICE_STATUS_PROCESS::default();
        let mut size = 0;
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                (&mut value as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
                std::mem::size_of_val(&value),
            )
        };
        unsafe { QueryServiceStatusEx(handle.0, SC_STATUS_PROCESS_INFO, Some(bytes), &mut size) }?;
        Ok(value)
    }
    pub fn running(name: &str) -> Result<bool> {
        let state = state(&service(name, SERVICE_QUERY_STATUS)?)?.dwCurrentState;
        ensure!(
            state == SERVICE_RUNNING || state == SERVICE_STOPPED,
            "{name} is transitioning or paused; retry later"
        );
        Ok(state == SERVICE_RUNNING)
    }
    pub fn preflight(name: &str) -> Result<()> {
        let _ = service(name, SERVICE_QUERY_STATUS | SERVICE_START | SERVICE_STOP).with_context(
            || {
                format!(
                    "Cannot pause {name}. Run R-Helper as administrator to pause Razer services"
                )
            },
        )?;
        Ok(())
    }
    pub fn stop(name: &str) -> Result<()> {
        let handle = service(name, SERVICE_QUERY_STATUS | SERVICE_STOP)?;
        if state(&handle)?.dwCurrentState == SERVICE_STOPPED {
            return Ok(());
        }
        let mut status = SERVICE_STATUS::default();
        if state(&handle)?.dwCurrentState != SERVICE_STOP_PENDING {
            unsafe { ControlService(handle.0, SERVICE_CONTROL_STOP, &mut status) }?;
        }
        let start = Instant::now();
        while state(&handle)?.dwCurrentState != SERVICE_STOPPED {
            ensure!(start.elapsed() < Duration::from_secs(10), "{name} did not stop");
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
    pub fn start(name: &str) -> Result<()> {
        let handle = service(name, SERVICE_QUERY_STATUS | SERVICE_START)?;
        if state(&handle)?.dwCurrentState == SERVICE_RUNNING {
            return Ok(());
        }
        unsafe { StartServiceW(handle.0, None) }?;
        let start = Instant::now();
        while state(&handle)?.dwCurrentState != SERVICE_RUNNING {
            ensure!(start.elapsed() < Duration::from_secs(10), "{name} did not restart");
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
    pub fn processes() -> Result<Vec<(u32, PathBuf)>> {
        let snapshot = Handle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut result = vec![];
        let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) }.is_ok();
        while more {
            let end = entry.szExeFile.iter().position(|v| *v == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if APPS.iter().any(|n| name.eq_ignore_ascii_case(n)) {
                if let Ok(handle) = unsafe {
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, entry.th32ProcessID)
                } {
                    let handle = Handle(handle);
                    let mut text = vec![0u16; 32768];
                    let mut length = text.len() as u32;
                    if unsafe {
                        QueryFullProcessImageNameW(
                            handle.0,
                            PROCESS_NAME_WIN32,
                            PWSTR(text.as_mut_ptr()),
                            &mut length,
                        )
                    }
                    .is_ok()
                    {
                        let path =
                            PathBuf::from(String::from_utf16_lossy(&text[..length as usize]));
                        if app_allowed(&path) {
                            result.push((entry.th32ProcessID, path));
                        }
                    }
                }
            }
            more = unsafe { Process32NextW(snapshot.0, &mut entry) }.is_ok();
        }
        Ok(result)
    }
    pub fn terminate(pid: u32, expected: &Path) -> Result<()> {
        // Re-check the executable through the same handle used to stop it, avoiding PID reuse.
        let handle = Handle(unsafe {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE, false, pid)
        }?);
        let mut text = vec![0u16; 32768];
        let mut length = text.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle.0,
                PROCESS_NAME_WIN32,
                PWSTR(text.as_mut_ptr()),
                &mut length,
            )
        }?;
        let path = PathBuf::from(String::from_utf16_lossy(&text[..length as usize]));
        ensure!(path == expected && app_allowed(&path), "Vendor process changed identity");
        unsafe { TerminateProcess(handle.0, 0) }?;
        Ok(())
    }
    pub fn signal(event: &str) -> Result<()> {
        let handle =
            Handle(unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(wide(event).as_ptr())) }?);
        unsafe { SetEvent(handle.0) }?;
        Ok(())
    }
    pub fn wait_owner(owner: u32, event: &str, path: &Path, record: &Record) -> Result<()> {
        let mutex = Handle(unsafe {
            CreateMutexW(None, false, windows::core::w!("Local\\RHelperVendorControl"))
        }?);
        let acquired = unsafe { WaitForSingleObject(mutex.0, 0) };
        ensure!(
            acquired == WAIT_OBJECT_0 || acquired == WAIT_ABANDONED,
            "Another R-Helper instance owns Razer control"
        );
        let parent = Handle(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, owner) }?);
        let release =
            Handle(unsafe { CreateEventW(None, true, false, PCWSTR(wide(event).as_ptr())) }?);
        std::fs::File::options()
            .create_new(true)
            .write(true)
            .open(path.with_extension("ready"))?
            .sync_all()?;
        let wait_result = loop {
            let result = unsafe { WaitForMultipleObjects(&[parent.0, release.0], false, 30_000) };
            if result == WAIT_OBJECT_0 || result.0 == WAIT_OBJECT_0.0 + 1 {
                break Ok(());
            }
            if result != WAIT_TIMEOUT {
                break Err(anyhow::anyhow!("Vendor recovery wait failed"));
            }
            // Reclose matching app relaunches; preserve service startup configuration.
            let errors = pause(record);
            let _ = std::fs::write(
                path.with_extension("status"),
                serde_json::to_vec(&serde_json::json!({"errors":errors}))?,
            );
        };
        let result = restore_owned(record, path);
        unsafe {
            let _ = ReleaseMutex(mutex.0);
        }
        result?;
        wait_result
    }
}
fn capture() -> Result<Record> {
    #[cfg(windows)]
    {
        let mut services = vec![];
        let mut controlled_services = vec![];
        for name in SERVICES {
            match native::running(name) {
                Ok(true) => {
                    native::preflight(name)?;
                    services.push((*name).into());
                    controlled_services.push((*name).into());
                }
                Ok(false) => {
                    native::preflight(name)?;
                    controlled_services.push((*name).into());
                }
                Err(e)
                    if e.downcast_ref::<windows::core::Error>()
                        .is_some_and(|e| e.code() == windows::core::HRESULT::from_win32(1060)) => {}
                Err(e) => return Err(e),
            }
        }
        let mut apps = vec![];
        for (_, path) in native::processes()? {
            if !apps.contains(&path) {
                apps.push(path);
            }
        }
        let record = Record { model: "RZ09-0529".into(), services, controlled_services, apps };
        record.validate()?;
        Ok(record)
    }
    #[cfg(not(windows))]
    anyhow::bail!("Windows required")
}
fn pause(record: &Record) -> Vec<String> {
    let mut errors = vec![];
    if record.controlled_services.is_empty() && record.apps.is_empty() {
        return errors;
    }
    #[cfg(windows)]
    {
        // Stop the updater before the hardware-control service to avoid relaunch races.
        for name in record.controlled_services.iter().rev() {
            if let Err(e) = native::stop(name) {
                errors.push(format!("{name}: {e:#}"));
            }
        }
        match native::processes() {
            Ok(processes) => {
                for (pid, path) in processes {
                    if let Err(e) = native::terminate(pid, &path) {
                        errors.push(format!("{}: {e:#}", path.display()));
                    }
                }
            }
            Err(e) => errors.push(format!("{e:#}")),
        }
    }
    errors
}
fn restore_owned(record: &Record, path: &Path) -> Result<()> {
    record.validate()?;
    let lighting =
        librazer::blade18::lighting::restore_native_owner(&path.with_extension("native.json"));
    let vendor = restore(record);
    match (lighting, vendor) {
        (Ok(()), Ok(())) => Ok(()),
        (lighting, vendor) => anyhow::bail!(
            "Restoration: lighting={:?}; Razer software={:?}",
            lighting.err(),
            vendor.err()
        ),
    }
}
fn restore(record: &Record) -> Result<()> {
    record.validate()?;
    let mut errors: Vec<String> = vec![];
    #[cfg(windows)]
    {
        for name in record.controlled_services.iter().filter(|n| !record.services.contains(n)) {
            if let Err(e) = native::stop(name) {
                errors.push(format!("{name}: {e:#}"));
            }
        }
        for name in &record.services {
            if let Err(e) = native::start(name) {
                errors.push(format!("{name}: {e:#}"));
            }
        }
        let current = native::processes()?;
        for path in &record.apps {
            if !current.iter().any(|(_, p)| p == path) {
                if let Err(e) = std::process::Command::new(path).spawn() {
                    errors.push(format!("{}: {e:#}", path.display()));
                }
            }
        }
    }
    ensure!(errors.is_empty(), "Vendor restoration: {}", errors.join("; "));
    Ok(())
}
pub fn inventory() -> serde_json::Value {
    #[cfg(windows)]
    {
        serde_json::json!({"services":SERVICES.iter().map(|name|{let running=native::running(name);serde_json::json!({"name":name,"running":running.as_ref().ok(),"error":running.as_ref().err().map(|e|format!("{e:#}")),"can_pause":running.as_ref().is_ok_and(|on|!*on||native::preflight(name).is_ok())})}).collect::<Vec<_>>(),"applications":native::processes().map(|p|p.into_iter().map(|(_,p)|p.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect::<Vec<_>>()).unwrap_or_default()})
    }
    #[cfg(not(windows))]
    serde_json::json!({"error":"Windows required"})
}
pub struct Session {
    path: PathBuf,
    event: String,
    child: std::process::Child,
}
/// Keep GUI copies from becoming competing laptop controllers. CLI recovery
/// helpers dispatch before this lease and remain independent.
pub struct UiInstance {
    #[cfg(windows)]
    _handle: native::Handle,
}
impl UiInstance {
    pub fn acquire() -> Result<Option<Self>> {
        #[cfg(windows)]
        {
            use windows::Win32::{
                Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
                System::Threading::CreateMutexW,
                UI::WindowsAndMessaging::*,
            };
            let handle = native::Handle(unsafe {
                CreateMutexW(None, false, windows::core::w!("Local\\RHelperBlade18UI"))
            }?);
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                if let Ok(window) =
                    unsafe { FindWindowW(None, windows::core::w!("R-Helper · Blade 18")) }
                {
                    unsafe {
                        let _ = ShowWindow(window, SW_SHOW);
                        let _ = ShowWindow(window, SW_RESTORE);
                        let _ = SetForegroundWindow(window);
                    }
                }
                return Ok(None);
            }
            Ok(Some(Self { _handle: handle }))
        }
        #[cfg(not(windows))]
        Ok(Some(Self {}))
    }
}
impl Session {
    pub fn start() -> Result<Self> {
        ensure!(librazer::diagnostics::is_target_system(), "Vendor takeover is model scoped");
        let record = capture()?;
        librazer::blade18::lamp_array::release();
        let session = Self::start_record(record)?;
        librazer::blade18::lighting::configure_native_owner(Some(
            session.path.with_extension("native.json"),
        ))?;
        Ok(session)
    }
    fn start_record(record: Record) -> Result<Self> {
        record.validate()?;
        let path = crate::blade_profiles::journal_path("vendor-ownership")?;
        let file = std::fs::File::options().create_new(true).write(true).open(&path)?;
        serde_json::to_writer_pretty(&file, &record)?;
        file.sync_all()?;
        let event = format!(
            "Local\\RHelperVendorRelease-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()
        );
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.arg("--vendor-guard").arg(&path).arg(std::process::id().to_string()).arg(&event);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let child = command.spawn()?;
        let mut session = Self { path, event, child };
        for _ in 0..50 {
            ensure!(session.child.try_wait()?.is_none(), "Vendor recovery guard failed");
            if session.path.with_extension("ready").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        ensure!(
            session.path.with_extension("ready").exists(),
            "Vendor recovery guard not ready; no software stopped"
        );
        let errors = pause(&record);
        if !errors.is_empty() {
            let _ = session.stop();
            anyhow::bail!("Cannot take exclusive control: {}", errors.join("; "));
        }
        Ok(session)
    }
    pub fn stop(&mut self) -> Result<()> {
        librazer::blade18::lighting::configure_native_owner(None)?;
        #[cfg(windows)]
        if self.child.try_wait()?.is_none() {
            native::signal(&self.event)?;
        }
        let start = Instant::now();
        while self.child.try_wait()?.is_none() {
            ensure!(start.elapsed() < Duration::from_secs(15), "Vendor restoration is pending");
            std::thread::sleep(Duration::from_millis(100));
        }
        let result = std::fs::read(self.path.with_extension("done"))
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok());
        if result.as_ref().is_some_and(|v| v["ok"] == true) {
            return Ok(());
        }
        // If the guard itself failed, the surviving owner can still restore its
        // immutable, scoped recovery record. Preserve the failed guard evidence.
        ensure!(std::fs::metadata(&self.path)?.len() < 65536, "Vendor recovery too large");
        let record: Record = serde_json::from_slice(&std::fs::read(&self.path)?)?;
        restore_owned(&record, &self.path)
            .context("Both vendor recovery guard and parent recovery need attention")
    }
    pub(crate) fn recovery_path(&self) -> &Path {
        &self.path
    }
    /// Diagnostic only: terminate this session's own guard to exercise parent fallback.
    pub(crate) fn test_guard_exit(&mut self) -> Result<()> {
        self.child.kill()?;
        self.child.wait()?;
        Ok(())
    }
    pub fn health(&mut self) -> Result<()> {
        ensure!(
            self.child.try_wait()?.is_none(),
            "Razer control recovery process exited; restore vendor software and retry takeover"
        );
        if self.path.with_extension("status").exists() {
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(self.path.with_extension("status"))?)?;
            ensure!(
                value["errors"].as_array().is_some_and(|a| a.is_empty()),
                "Razer software pause needs attention: {value}"
            );
        }
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
pub fn dispatch() -> Option<Result<()>> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1)?.as_str() == "--vendor-validation-hold" {
        return Some((|| {
            ensure!(args.len() == 3, "Expected new validation directory");
            ensure!(
                crate::blade_gpu::administrator(),
                "Administrator required for validation takeover"
            );
            ensure!(
                librazer::diagnostics::is_target_system() && librazer::blade18::ac_connected()?,
                "Blade 18 on AC required"
            );
            let state = librazer::blade18::status()?;
            ensure!(state.performance==librazer::types::PerfMode::Balanced && state.fan_mode==librazer::types::FanMode::Auto,
                "Select Balanced/Auto before the validation hold; this helper does not change performance settings");
            let directory = PathBuf::from(&args[2]);
            std::fs::create_dir(&directory)?;
            let mut session = Session::start()?;
            let file = std::fs::File::options()
                .create_new(true)
                .write(true)
                .open(directory.join("ready.json"))?;
            serde_json::to_writer_pretty(
                &file,
                &serde_json::json!({"model":"RZ09-0529","vendor_recovery":session.path,"deadline_seconds":2700,"note":"No GUI or power-profile watcher. Fan tools retain their own recovery. Write release to end this hold before battery overhead measurement."}),
            )?;
            file.sync_all()?;
            let started = Instant::now();
            let mut health = Instant::now();
            let held = (|| -> Result<()> {
                while started.elapsed() < Duration::from_secs(2700)
                    && !directory.join("release").exists()
                {
                    if health.elapsed() >= Duration::from_secs(30) {
                        session.health()?;
                        health = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                Ok(())
            })();
            let released = session.stop();
            let file = std::fs::File::options()
                .create_new(true)
                .write(true)
                .open(directory.join("finished.json"))?;
            serde_json::to_writer_pretty(
                &file,
                &serde_json::json!({"vendor_restored":released.is_ok(),"restore_error":released.as_ref().err().map(|e|format!("{e:#}")),"hold_error":held.as_ref().err().map(|e|format!("{e:#}"))}),
            )?;
            file.sync_all()?;
            released?;
            held?;
            Ok(())
        })());
    }
    if args.get(1)?.as_str() == "--vendor-preflight" {
        return Some((|| {
            ensure!(args.len() == 3, "Expected output path");
            let result = capture();
            let file = std::fs::File::options().create_new(true).write(true).open(&args[2])?;
            serde_json::to_writer_pretty(
                &file,
                &serde_json::json!({"can_takeover":result.is_ok(),"error":result.err().map(|e|format!("{e:#}"))}),
            )?;
            file.sync_all()?;
            Ok(())
        })());
    }
    if matches!(
        args.get(1)?.as_str(),
        "--vendor-test" | "--vendor-test-crash" | "--vendor-test-guard-crash"
    ) {
        return Some((|| {
            ensure!(args.len() == 3, "Expected test output");
            let mut session = Session::start_record(Record {
                model: "RZ09-0529".into(),
                services: vec![],
                controlled_services: vec![],
                apps: vec![],
            })?;
            let recovery = session.path.clone();
            let crash = args[1] == "--vendor-test-crash";
            let guard_crash = args[1] == "--vendor-test-guard-crash";
            if guard_crash {
                session.child.kill()?;
                session.child.wait()?;
            }
            if !crash {
                session.stop()?;
            }
            let file = std::fs::File::options().create_new(true).write(true).open(&args[2])?;
            serde_json::to_writer_pretty(
                &file,
                &serde_json::json!({"recovery":recovery,"crash_simulation":crash,"guard_crash_simulation":guard_crash,"released":!crash}),
            )?;
            file.sync_all()?;
            if crash {
                std::mem::forget(session);
            }
            Ok(())
        })());
    }
    if args.get(1)?.as_str() == "--vendor-inventory" {
        return Some((|| {
            ensure!(args.len() == 3, "Expected output path");
            let file = std::fs::File::options().create_new(true).write(true).open(&args[2])?;
            serde_json::to_writer_pretty(&file, &inventory())?;
            file.sync_all()?;
            Ok(())
        })());
    }
    if args[1] != "--vendor-guard" {
        return None;
    }
    Some((|| {
        ensure!(args.len() == 5, "Invalid vendor recovery arguments");
        let path = PathBuf::from(&args[2]);
        ensure!(std::fs::metadata(&path)?.len() < 65536, "Vendor recovery too large");
        let record: Record = serde_json::from_slice(&std::fs::read(&path)?)?;
        record.validate()?;
        let owner: u32 = args[3].parse()?;
        ensure!(
            args[4].starts_with(&format!("Local\\RHelperVendorRelease-{owner}-")),
            "Invalid release event"
        );
        #[cfg(windows)]
        let result = native::wait_owner(owner, &args[4], &path, &record);
        #[cfg(not(windows))]
        let result: Result<()> = Err(anyhow::anyhow!("Windows required"));
        let file = std::fs::File::options()
            .create_new(true)
            .write(true)
            .open(path.with_extension("done"))?;
        serde_json::to_writer_pretty(
            &file,
            &serde_json::json!({"ok":result.is_ok(),"error":result.as_ref().err().map(|e|format!("{e:#}"))}),
        )?;
        file.sync_all()?;
        result
    })())
}
mod ui;
pub use ui::Ui;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_scope_excludes_drivers_security_and_arbitrary_paths() {
        assert!(!app_allowed(Path::new("C:\\Windows\\RazerAppEngine.exe")));
        assert!(!app_allowed(Path::new("C:\\Program Files\\RazerEvil\\RazerAppEngine.exe")));
        assert!(!app_allowed(Path::new("C:\\Program Files\\Razer\\..\\RazerAppEngine.exe")));
        assert!(!app_allowed(Path::new("C:\\Program Files\\Razer\\razer_elevation_service.exe")));
        let record = Record {
            model: "RZ09-0529".into(),
            services: vec!["WinDefend".into()],
            controlled_services: vec!["WinDefend".into()],
            apps: vec![],
        };
        assert!(record.validate().is_err());
    }
}
