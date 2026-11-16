//! Event-driven curve shutdown. Callbacks never open HID; the owning worker
//! restores Auto and acknowledges it before the bounded callback returns.
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BUDGET: Duration = Duration::from_millis(1500);
fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
fn read(path: &Path) -> Option<Value> {
    let mut bytes = Vec::new();
    fs::File::open(path).ok()?.take(16385).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 16384 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn acknowledged(status: &Value, original: &Value, requested: u64) -> bool {
    let (Ok(sample), Ok(original)) = (
        serde_json::from_value::<librazer::thermal::Sample>(status["sample"].clone()),
        serde_json::from_value::<librazer::thermal::Sample>(original.clone()),
    ) else {
        return false;
    };
    status["phase"] == "stopped"
        && status.as_object().is_some_and(|s| s.contains_key("recovery_error"))
        && status["recovery_error"].is_null()
        && status["captured_unix_ms"].as_u64().is_some_and(|t| {
            t >= requested && t <= requested.saturating_add(BUDGET.as_millis() as u64)
        })
        && sample.target_rpm == original.target_rpm
        && original.modes.iter().all(|mode| mode[1] == 0)
        && sample.modes == original.modes
}

struct Context {
    directory: PathBuf,
    first_request: Arc<AtomicU64>,
    callbacks: AtomicUsize,
}
impl Context {
    fn new(directory: &Path) -> Self {
        Self::with_request(directory, Arc::new(AtomicU64::new(0)))
    }
    fn with_request(directory: &Path, first_request: Arc<AtomicU64>) -> Self {
        Self { directory: directory.into(), first_request, callbacks: AtomicUsize::new(0) }
    }
    fn stop(&self, trigger: &str, budget: Duration) -> Result<Value> {
        let now = unix_ms();
        let requested = self
            .first_request
            .compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed)
            .unwrap_or_else(|old| old);
        let requested = if requested == 0 { now } else { requested };
        fs::write(self.directory.join("stop"), b"stop")?;
        let started = Instant::now();
        let mut automatic = false;
        let mut verified = None;
        loop {
            if let (Some(status), Some(recovery)) = (
                read(&self.directory.join("status.json")),
                read(&self.directory.join("recovery.json")),
            ) {
                if recovery["model"] == "RZ09-0529"
                    && recovery["operation"] == "curve_session"
                    && acknowledged(&status, &recovery["original"], requested)
                {
                    automatic = true;
                    verified = Some(status);
                    break;
                }
            }
            if started.elapsed() >= budget
                || unix_ms().saturating_sub(now) >= budget.as_millis() as u64
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let report = json!({"trigger":trigger,"event_received_unix_ms":now,"requested_unix_ms":requested,"completed_unix_ms":unix_ms(),"automatic_verified":automatic,"budget_ms":budget.as_millis(),"status":verified});
        fs::write(
            self.directory.join(format!("sleep-{trigger}.json")),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(report)
    }
}

pub(crate) fn simulate_suspend(directory: &Path) -> Result<()> {
    let recovery = read(&directory.join("recovery.json"))
        .ok_or_else(|| anyhow::anyhow!("No curve recovery record"))?;
    ensure!(
        recovery["model"] == "RZ09-0529" && recovery["operation"] == "curve_session",
        "Expected active curve session"
    );
    let report = Context::new(directory).stop("test-suspend", BUDGET)?;
    ensure!(
        report["automatic_verified"] == true,
        "Automatic cooling was not acknowledged within the suspend budget"
    );
    Ok(())
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        core::GUID,
        Win32::{
            Foundation::HANDLE,
            System::Power::*,
            UI::WindowsAndMessaging::{
                DEVICE_NOTIFY_CALLBACK, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE,
            },
        },
    };
    const DISPLAY: GUID = GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47);
    pub(super) struct Registration {
        handle: HPOWERNOTIFY,
        context: Option<Box<Context>>,
        parameters: Option<Box<DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS>>,
        suspend: bool,
    }
    impl Registration {
        pub(super) fn new(
            directory: &Path,
            suspend: bool,
            first_request: Arc<AtomicU64>,
        ) -> Result<Self> {
            let mut context = Box::new(Context::with_request(directory, first_request));
            let parameters = Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
                Callback: Some(changed),
                Context: (&mut *context as *mut Context).cast(),
            });
            let recipient = HANDLE(
                (&*parameters as *const DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS).cast_mut().cast(),
            );
            let mut handle = std::ptr::null_mut();
            unsafe {
                if suspend {
                    PowerRegisterSuspendResumeNotification(
                        DEVICE_NOTIFY_CALLBACK,
                        recipient,
                        &mut handle,
                    )
                    .ok()?;
                } else {
                    PowerSettingRegisterNotification(
                        &DISPLAY,
                        DEVICE_NOTIFY_CALLBACK,
                        recipient,
                        &mut handle,
                    )
                    .ok()?;
                }
            }
            Ok(Self {
                handle: HPOWERNOTIFY(handle as isize),
                context: Some(context),
                parameters: Some(parameters),
                suspend,
            })
        }
    }
    impl Drop for Registration {
        fn drop(&mut self) {
            let _result = unsafe {
                if self.suspend {
                    PowerUnregisterSuspendResumeNotification(self.handle)
                } else {
                    PowerSettingUnregisterNotification(self.handle)
                }
            };
            let deadline = Instant::now() + BUDGET;
            while self.context.as_ref().is_some_and(|c| c.callbacks.load(Ordering::Acquire) != 0)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            // This guard exists only in the short-lived curve supervisor. Keep
            // the two tiny callback allocations until process exit, including
            // failed/late unregistration, without assuming Windows drained every
            // queued callback. The OS reclaims them when that supervisor exits.
            if let Some(v) = self.context.take() {
                let _ = Box::leak(v);
            }
            if let Some(v) = self.parameters.take() {
                let _ = Box::leak(v);
            }
        }
    }
    unsafe extern "system" fn changed(
        context: *const std::ffi::c_void,
        event: u32,
        setting: *const std::ffi::c_void,
    ) -> u32 {
        if context.is_null() {
            return 0;
        }
        let trigger = if event == PBT_APMSUSPEND {
            Some("suspend")
        } else if event == PBT_POWERSETTINGCHANGE && !setting.is_null() {
            let s = &*setting.cast::<POWERBROADCAST_SETTING>();
            if s.PowerSetting == DISPLAY
                && s.DataLength == 4
                && std::ptr::read_unaligned(s.Data.as_ptr().cast::<u32>()) == 0
            {
                Some("display-off")
            } else {
                None
            }
        } else {
            None
        };
        if let Some(trigger) = trigger {
            let context = &*context.cast::<Context>();
            context.callbacks.fetch_add(1, Ordering::AcqRel);
            struct Active<'a>(&'a AtomicUsize);
            impl Drop for Active<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::Release);
                }
            }
            let _active = Active(&context.callbacks);
            // No panic may cross the Windows callback boundary.
            let _ = std::panic::catch_unwind(|| context.stop(trigger, BUDGET));
        }
        0
    }
    #[cfg(test)]
    pub(super) unsafe fn changed_for_test(
        context: *const std::ffi::c_void,
        event: u32,
        setting: *const std::ffi::c_void,
    ) {
        changed(context, event, setting);
    }
}

pub(crate) struct Guard {
    #[cfg(windows)]
    _registrations: [native::Registration; 2],
}
impl Guard {
    pub(crate) fn register(directory: &Path) -> Result<Self> {
        #[cfg(windows)]
        {
            let first_request = Arc::new(AtomicU64::new(0));
            Ok(Self {
                _registrations: [
                    native::Registration::new(directory, true, first_request.clone())?,
                    native::Registration::new(directory, false, first_request)?,
                ],
            })
        }
        #[cfg(not(windows))]
        {
            let _ = directory;
            anyhow::bail!("Windows sleep notifications required for curves")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn original() -> Value {
        json!({"actual_rpm":[2000,2000,0],"target_rpm":[4500,0,0],"modes":[[0,0],[0,0],[0,0]],"ec_celsius":[60,40]})
    }
    fn status(time: u64) -> Value {
        json!({"phase":"stopped","captured_unix_ms":time,"sample":original(),"recovery_error":null})
    }
    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rhelper-sleep-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        fs::write(
            path.join("recovery.json"),
            json!({"model":"RZ09-0529","operation":"curve_session","original":original()})
                .to_string(),
        )
        .unwrap();
        path
    }
    #[test]
    fn stale_late_missing_or_partial_auto_ack_is_rejected() {
        assert!(acknowledged(&status(1100), &original(), 1000));
        for t in [999, 2501] {
            assert!(!acknowledged(&status(t), &original(), 1000));
        }
        for field in ["captured_unix_ms", "sample", "recovery_error"] {
            let mut s = status(1100);
            s.as_object_mut().unwrap().remove(field);
            assert!(!acknowledged(&s, &original(), 1000));
        }
        for (pointer, value) in [
            ("/sample/modes/2/1", json!(1)),
            ("/sample/target_rpm/2", json!(3500)),
            ("/recovery_error", json!("failed")),
            ("/phase", json!("running")),
        ] {
            let mut s = status(1100);
            *s.pointer_mut(pointer).unwrap() = value;
            assert!(!acknowledged(&s, &original(), 1000));
        }
        assert!(!acknowledged(&status(1100), &json!({}), 1000));
    }
    #[test]
    fn stop_latches_before_ack_and_timeout_does_not_claim_success() {
        let path = directory();
        fs::write(path.join("status.json"), status(1).to_string()).unwrap();
        let report = Context::new(&path).stop("test", Duration::from_millis(20)).unwrap();
        assert!(path.join("stop").exists());
        assert_eq!(report["automatic_verified"], false);
    }
    #[test]
    fn fresh_worker_restoration_acknowledges_stop_without_hardware_queries() {
        let path = directory();
        let responder = path.clone();
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !responder.join("stop").exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            fs::write(responder.join("status.json"), status(unix_ms()).to_string()).unwrap();
        });
        let report = Context::new(&path).stop("test", BUDGET).unwrap();
        worker.join().unwrap();
        assert_eq!(report["automatic_verified"], true);
    }
    #[test]
    fn display_off_then_suspend_share_the_latched_restoration_ack() {
        let path = directory();
        let first = Arc::new(AtomicU64::new(0));
        let display = Context::with_request(&path, first.clone());
        let suspend = Context::with_request(&path, first);
        let responder = path.clone();
        let worker = std::thread::spawn(move || {
            while !responder.join("stop").exists() {
                std::thread::sleep(Duration::from_millis(1));
            }
            fs::write(responder.join("status.json"), status(unix_ms()).to_string()).unwrap();
        });
        let a = display.stop("display-off", BUDGET).unwrap();
        worker.join().unwrap();
        std::thread::sleep(Duration::from_millis(5));
        let b = suspend.stop("suspend", Duration::from_millis(20)).unwrap();
        assert_eq!(a["automatic_verified"], true);
        assert_eq!(b["automatic_verified"], true);
        assert_eq!(a["requested_unix_ms"], b["requested_unix_ms"]);
        assert!(
            b["event_received_unix_ms"].as_u64().unwrap()
                >= b["status"]["captured_unix_ms"].as_u64().unwrap()
        );
    }
    #[cfg(windows)]
    #[test]
    fn resume_notification_does_not_stop_or_restart_curves() {
        let path = directory();
        let context = Context::new(&path);
        unsafe {
            native::changed_for_test((&context as *const Context).cast(), 0x12, std::ptr::null());
        }
        assert!(!path.join("stop").exists());
    }
    #[cfg(windows)]
    #[test]
    fn native_suspend_and_display_off_route_to_verified_auto_handoff() {
        use windows::{
            core::GUID,
            Win32::UI::WindowsAndMessaging::{PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE},
        };
        #[repr(C)]
        struct Setting {
            guid: GUID,
            length: u32,
            value: u32,
        }
        let setting = Setting {
            guid: GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47),
            length: 4,
            value: 0,
        };
        for (event, data, trigger) in [
            (PBT_APMSUSPEND, std::ptr::null(), "suspend"),
            (PBT_POWERSETTINGCHANGE, (&setting as *const Setting).cast(), "display-off"),
        ] {
            let path = directory();
            let responder = path.clone();
            let context = Context::new(&path);
            let worker = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                while !responder.join("stop").exists() {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(1));
                }
                fs::write(responder.join("status.json"), status(unix_ms()).to_string()).unwrap();
            });
            unsafe {
                native::changed_for_test((&context as *const Context).cast(), event, data);
            }
            worker.join().unwrap();
            let report = read(&path.join(format!("sleep-{trigger}.json"))).unwrap();
            assert_eq!(report["automatic_verified"], true);
            assert_eq!(context.callbacks.load(Ordering::Acquire), 0);
        }
    }
    #[cfg(windows)]
    #[test]
    fn display_on_dim_and_malformed_notifications_leave_curves_alone() {
        use windows::{core::GUID, Win32::UI::WindowsAndMessaging::PBT_POWERSETTINGCHANGE};
        #[repr(C)]
        struct Setting {
            guid: GUID,
            length: u32,
            value: u32,
        }
        for (length, value) in [(4, 1), (4, 2), (1, 0), (4, 99)] {
            let path = directory();
            let context = Context::new(&path);
            let setting = Setting {
                guid: GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47),
                length,
                value,
            };
            unsafe {
                native::changed_for_test(
                    (&context as *const Context).cast(),
                    PBT_POWERSETTINGCHANGE,
                    (&setting as *const Setting).cast(),
                );
            }
            assert!(!path.join("stop").exists());
        }
    }
}
