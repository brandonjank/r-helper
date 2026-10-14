use crate::blade_profiles::{self, Settings};
use librazer::blade18::{self, State};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

pub struct Update {
    pub ac: Option<bool>,
    pub result: Option<Result<State, String>>,
    pub monitor_error: Option<String>,
}
pub(crate) enum Event {
    Power(Option<bool>),
    Stop,
    Wake,
    Display(bool),
    Baseline(crate::blade_profiles::Named),
    Resume,
    Tick,
}
pub struct Watcher {
    events: mpsc::Sender<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Watcher {
    pub fn new(
        settings: Arc<Mutex<Settings>>,
        ctx: eframe::egui::Context,
        curves: crate::blade_curves::Shared,
    ) -> (Self, mpsc::Receiver<Update>) {
        let (events, incoming) = mpsc::channel();
        let callbacks = events.clone();
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            // Native handles and callback storage stay on their owning thread.
            // Registration delivers an initial baseline event, without applying a profile.
            let registration = Notification::new(callbacks);
            let mut previous = None;
            if let Err(error) = &registration {
                previous = blade18::ac_connected().ok();
                let _ = sender.send(Update {
                    ac: previous,
                    result: None,
                    monitor_error: Some(format!(
                        "Power notifications unavailable; using 30-second fallback: {error:#}"
                    )),
                });
                ctx.request_repaint();
            }
            loop {
                let event = if registration.is_ok() {
                    incoming.recv().ok()
                } else {
                    match incoming.recv_timeout(Duration::from_secs(30)) {
                        Ok(event) => Some(event),
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            Some(Event::Power(blade18::ac_connected().ok()))
                        }
                        Err(_) => None,
                    }
                };
                let Some(Event::Power(mut current)) = event else { return };
                // Coalesce events queued during hardware work; don't replay stale transitions.
                for event in incoming.try_iter() {
                    match event {
                        Event::Power(source) => current = source,
                        Event::Stop => return,
                        _ => {}
                    }
                }
                if current == previous {
                    continue;
                }
                let saved = match settings.lock() {
                    Ok(saved) => saved.clone(),
                    Err(_) => return,
                };
                let stopped = if previous.is_some() {
                    crate::blade_curves::stop_and_wait(&curves)
                } else {
                    Ok(())
                };
                let result = blade_profiles::transition(previous, current, saved.automatic)
                    .and_then(|ac| {
                        let profile = if ac { saved.ac.clone() } else { saved.battery.clone() };
                        let display = if ac {
                            saved.display_ac.clone()
                        } else {
                            saved.display_battery.clone()
                        };
                        profile.map(|profile| {
                            stopped
                                .and_then(|_| {
                                    crate::blade_display::wait_for_confirmation()?;
                                    let _lock = blade_profiles::operation_lock()?;
                                    crate::blade_curves::stop_and_wait(&curves)?;
                                    if let Some(named) = crate::blade_policy::matched(&saved, ac) {
                                        return named.apply();
                                    }
                                    let state = blade_profiles::apply(&profile, display.as_ref())?;
                                    let lighting = if ac {
                                        &saved.lighting_ac
                                    } else {
                                        &saved.lighting_battery
                                    };
                                    if let Some(lighting) = lighting {
                                        lighting.apply(&blade_profiles::journal_path(
                                            "power-lighting",
                                        )?)?;
                                    }
                                    Ok(state)
                                })
                                .map_err(|error| format!("{error:#}"))
                        })
                    });
                previous = current;
                if sender.send(Update { ac: current, result, monitor_error: None }).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        (Self { events, thread: Some(thread) }, receiver)
    }
    pub fn stop(&mut self) {
        let _ = self.events.send(Event::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
pub(crate) struct Notification {
    handle: windows::Win32::System::Power::HPOWERNOTIFY,
    context: Option<Box<mpsc::Sender<Event>>>,
    _parameters: Option<Box<windows::Win32::System::Power::DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS>>,
    suspend_resume: bool,
}
#[cfg(windows)]
const AC_SOURCE: windows::core::GUID =
    windows::core::GUID::from_u128(0x5d3e9a59_e9d5_4b00_a6bd_ff34ff516548);
#[cfg(windows)]
const DISPLAY_STATE: windows::core::GUID =
    windows::core::GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47);
#[cfg(windows)]
impl Notification {
    pub(crate) fn new(sender: mpsc::Sender<Event>) -> anyhow::Result<Self> {
        Self::subscribe(sender, Some(AC_SOURCE))
    }
    pub(crate) fn display(sender: mpsc::Sender<Event>) -> anyhow::Result<Self> {
        Self::subscribe(sender, Some(DISPLAY_STATE))
    }
    pub(crate) fn resume(sender: mpsc::Sender<Event>) -> anyhow::Result<Self> {
        Self::subscribe(sender, None)
    }
    fn subscribe(
        sender: mpsc::Sender<Event>,
        guid: Option<windows::core::GUID>,
    ) -> anyhow::Result<Self> {
        use windows::Win32::{
            Foundation::HANDLE,
            System::Power::{
                PowerSettingRegisterNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY,
            },
            UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK,
        };
        let mut context = Box::new(sender);
        let parameters = Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(power_changed),
            Context: (&mut *context as *mut mpsc::Sender<Event>).cast(),
        });
        let mut handle = std::ptr::null_mut();
        unsafe {
            let recipient = HANDLE(
                (&*parameters as *const DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS).cast_mut().cast(),
            );
            if let Some(guid) = guid {
                PowerSettingRegisterNotification(
                    &guid,
                    DEVICE_NOTIFY_CALLBACK,
                    HANDLE(
                        (&*parameters as *const DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS)
                            .cast_mut()
                            .cast(),
                    ),
                    &mut handle,
                )
                .ok()?;
            } else {
                windows::Win32::System::Power::PowerRegisterSuspendResumeNotification(
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
            _parameters: Some(parameters),
            suspend_resume: guid.is_none(),
        })
    }
}
#[cfg(windows)]
impl Drop for Notification {
    fn drop(&mut self) {
        let result = unsafe {
            if self.suspend_resume {
                windows::Win32::System::Power::PowerUnregisterSuspendResumeNotification(self.handle)
            } else {
                windows::Win32::System::Power::PowerSettingUnregisterNotification(self.handle)
            }
        };
        if result.is_err() {
            // If unregister fails, Windows may still call back. Keep both allocations
            // valid until process exit rather than risking a dangling pointer.
            if let Some(context) = self.context.take() {
                let _ = Box::leak(context);
            }
            if let Some(parameters) = self._parameters.take() {
                let _ = Box::leak(parameters);
            }
        }
    }
}
#[cfg(windows)]
unsafe extern "system" fn power_changed(
    context: *const std::ffi::c_void,
    event_type: u32,
    setting: *const std::ffi::c_void,
) -> u32 {
    if event_type == 0x12 && !context.is_null() {
        let sender = &*context.cast::<mpsc::Sender<Event>>();
        let _ = sender.send(Event::Resume);
        return 0;
    }
    use windows::Win32::{
        System::Power::POWERBROADCAST_SETTING, UI::WindowsAndMessaging::PBT_POWERSETTINGCHANGE,
    };
    if event_type == PBT_POWERSETTINGCHANGE && !context.is_null() && !setting.is_null() {
        let setting = &*setting.cast::<POWERBROADCAST_SETTING>();
        if setting.PowerSetting == AC_SOURCE && setting.DataLength == 4 {
            let source = std::ptr::read_unaligned(setting.Data.as_ptr().cast::<u32>());
            let ac = match source {
                0 => Some(true),
                1 | 2 => Some(false),
                _ => None,
            };
            let sender = &*context.cast::<mpsc::Sender<Event>>();
            let _ = sender.send(Event::Power(ac));
        }
        if setting.PowerSetting == DISPLAY_STATE && setting.DataLength == 4 {
            let value = std::ptr::read_unaligned(setting.Data.as_ptr().cast::<u32>());
            if value <= 2 {
                let sender = &*context.cast::<mpsc::Sender<Event>>();
                let _ = sender.send(Event::Display(value != 0));
            }
        }
    }
    0
}
#[cfg(not(windows))]
pub(crate) struct Notification;
#[cfg(not(windows))]
impl Notification {
    pub(crate) fn new(_: mpsc::Sender<Event>) -> anyhow::Result<Self> {
        anyhow::bail!("Windows is required")
    }
    pub(crate) fn display(_: mpsc::Sender<Event>) -> anyhow::Result<Self> {
        anyhow::bail!("Windows is required")
    }
    pub(crate) fn resume(_: mpsc::Sender<Event>) -> anyhow::Result<Self> {
        anyhow::bail!("Windows is required")
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn native_registration_delivers_initial_source_without_polling() {
        let (sender, receiver) = mpsc::channel();
        let _registration = Notification::new(sender).unwrap();
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Power(Some(_))
        ));
    }
    #[test]
    fn stop_wakes_an_idle_watcher_without_a_timer() {
        let settings = Arc::new(Mutex::new(Settings::default()));
        let (mut watcher, _) =
            Watcher::new(settings, eframe::egui::Context::default(), crate::blade_curves::shared());
        watcher.stop();
        assert!(watcher.thread.is_none());
    }
    #[test]
    fn resume_callback_routes_only_the_resume_event_without_a_setting_pointer() {
        let (sender, receiver) = mpsc::channel();
        unsafe {
            power_changed((&sender as *const mpsc::Sender<Event>).cast(), 0x12, std::ptr::null());
        }
        assert!(matches!(receiver.try_recv().unwrap(), Event::Resume));
        unsafe {
            power_changed((&sender as *const mpsc::Sender<Event>).cast(), 0x04, std::ptr::null());
        }
        assert!(receiver.try_recv().is_err());
    }
}
