use crate::{
    blade_power::{Event, Notification},
    blade_profiles::{self, Named, Settings},
};
use librazer::blade18::{self, lighting, Change};
use std::{
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

#[cfg(windows)]
pub fn foreground_exe() -> Option<String> {
    use windows::Win32::{System::Threading::*, UI::WindowsAndMessaging::*};
    unsafe {
        let window = GetForegroundWindow();
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        result.ok()?;
        std::path::Path::new(&String::from_utf16_lossy(&buffer[..length as usize]))
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
    }
}
#[cfg(not(windows))]
pub fn foreground_exe() -> Option<String> {
    None
}
pub fn matched(settings: &Settings, ac: bool) -> Option<Named> {
    if !settings.application_switching {
        return None;
    }
    let exe = foreground_exe()?;
    settings
        .named
        .iter()
        .find(|p| {
            !p.executable.is_empty()
                && p.executable.eq_ignore_ascii_case(&exe)
                && p.hardware.validate(!ac).is_ok()
        })
        .cloned()
}
#[cfg(windows)]
fn inputs() -> anyhow::Result<(u64, Option<u8>)> {
    use windows::Win32::{
        System::{
            Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
            SystemInformation::GetTickCount,
        },
        UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
    };
    unsafe {
        let mut last = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            ..Default::default()
        };
        anyhow::ensure!(GetLastInputInfo(&mut last).as_bool(), "Cannot read idle time");
        let idle = u64::from(GetTickCount().wrapping_sub(last.dwTime)) / 1000;
        let mut power = SYSTEM_POWER_STATUS::default();
        GetSystemPowerStatus(&mut power)?;
        Ok((
            idle,
            if power.BatteryLifePercent <= 100 { Some(power.BatteryLifePercent) } else { None },
        ))
    }
}
#[cfg(not(windows))]
fn inputs() -> anyhow::Result<(u64, Option<u8>)> {
    anyhow::bail!("Windows required")
}
pub fn should_blank(
    p: &blade_profiles::LightingPolicy,
    ac: bool,
    idle: u64,
    display_on: bool,
    battery: Option<u8>,
) -> bool {
    let seconds = if ac { p.idle_ac_seconds } else { p.idle_battery_seconds };
    p.enabled
        && ((seconds > 0 && idle >= u64::from(seconds))
            || (p.display_off && !display_on)
            || (!ac
                && p.low_battery_percent > 0
                && battery.is_some_and(|v| v <= p.low_battery_percent)))
}
#[derive(Clone, Debug, PartialEq)]
struct Levels(u8, bool, librazer::types::LogoMode);
fn levels(p: &lighting::Preset, blank: bool) -> Levels {
    if blank {
        Levels(0, false, librazer::types::LogoMode::Off)
    } else {
        Levels(p.brightness, p.underside, p.logo)
    }
}
fn apply_levels(l: &Levels) -> anyhow::Result<()> {
    blade18::change(Change::Brightness(l.0))?;
    blade18::change(Change::Logo(l.2))?;
    lighting::change(lighting::Change::Underside(l.1))?;
    Ok(())
}
pub struct Watcher {
    events: mpsc::Sender<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}
pub struct Update {
    pub activity: Option<crate::profile_activity::Activity>,
    pub message: String,
    pub state: Option<blade18::State>,
}
struct Reporter(mpsc::Sender<Update>);
impl Reporter {
    fn send(&self, message: String) -> Result<(), mpsc::SendError<Update>> {
        self.send_activity(message, None)
    }
    fn send_activity(
        &self,
        message: String,
        activity: Option<crate::profile_activity::Activity>,
    ) -> Result<(), mpsc::SendError<Update>> {
        let state = if message.contains("unavailable") || message.starts_with("Automation:") {
            None
        } else {
            blade18::status().ok()
        };
        self.0.send(Update { activity, message, state })
    }
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
        let sender = Reporter(sender);
        let thread = std::thread::spawn(move || {
            let resume_registration = Notification::resume(callbacks.clone());
            let _power_registration = Notification::new(callbacks.clone());
            if let Err(error) = &_power_registration {
                let _ = sender.send(format!(
                    "Power notifications unavailable; using a 30-second fallback: {error:#}"
                ));
                ctx.request_repaint();
            }
            if let Err(error) = &resume_registration {
                let _ = sender.send(format!("Resume notifications unavailable: {error:#}"));
                ctx.request_repaint();
            }
            let registration = Notification::display(callbacks);
            if let Err(e) = &registration {
                let _ = sender.send(format!("Display-off policy unavailable: {e:#}"));
                ctx.request_repaint();
            }
            let mut display_on = true;
            let mut baseline: Option<Named> = None;
            let mut lighting_baseline: Option<Levels> = None;
            let mut applied: Option<Levels> = None;
            let mut active: Option<(String, bool)> = None;
            let mut manual: Option<(bool, lighting::Preset)> = None;
            let mut power_lighting: Option<String> = None;
            let mut unblank: Option<Levels> = None;
            let mut resumed = false;
            let mut retry_at = std::time::Instant::now();
            let mut retry_pending = false;
            loop {
                let saved = match settings.lock() {
                    Ok(s) => s.clone(),
                    Err(_) => return,
                };
                let timed = saved.application_switching
                    || (saved.lighting_policy.enabled
                        && (saved.lighting_policy.idle_ac_seconds > 0
                            || saved.lighting_policy.idle_battery_seconds > 0
                            || saved.lighting_policy.low_battery_percent > 0));
                let enabled = timed
                    || resumed
                    || retry_pending
                    || (_power_registration.is_err() && saved.lighting_policy.enabled);
                let event = if enabled {
                    match incoming.recv_timeout(Duration::from_secs(
                        if timed || resumed || retry_pending { 5 } else { 30 },
                    )) {
                        Ok(e) => e,
                        Err(mpsc::RecvTimeoutError::Timeout) => Event::Tick,
                        Err(_) => Event::Stop,
                    }
                } else {
                    match incoming.recv() {
                        Ok(e) => e,
                        Err(_) => Event::Stop,
                    }
                };
                let stopping = matches!(event, Event::Stop);
                let skip_retry =
                    matches!(event, Event::Tick) && std::time::Instant::now() < retry_at;
                if matches!(event, Event::Resume) {
                    resumed = true;
                }
                if let Event::Display(on) = event {
                    display_on = on;
                }
                if let Event::Baseline(p) = event {
                    if let Ok(ac) = blade18::ac_connected() {
                        manual = Some((ac, p.lighting.clone()));
                        lighting_baseline = Some(levels(&p.lighting, false));
                        baseline = Some(p);
                        active = None;
                        applied = None;
                        power_lighting = None;
                    }
                }
                let saved = match settings.lock() {
                    Ok(s) => s.clone(),
                    Err(_) => return,
                };
                let result = (|| -> anyhow::Result<()> {
                    if skip_retry {
                        return Ok(());
                    }
                    if crate::blade_display::is_pending() {
                        return Ok(());
                    }
                    if curves.lock().map_err(|_| anyhow::anyhow!("Curve lock failed"))?.is_some() {
                        return Ok(());
                    }
                    let _lock = blade_profiles::operation_lock()?;
                    if curves.lock().map_err(|_| anyhow::anyhow!("Curve lock failed"))?.is_some() {
                        return Ok(());
                    }
                    let ac = blade18::ac_connected()?;
                    if manual.as_ref().is_some_and(|(source, _)| *source != ac) {
                        manual = None;
                        power_lighting = None;
                    }
                    let selected = if stopping { None } else { matched(&saved, ac) };
                    let next = selected.as_ref().map(|p| (p.name.clone(), ac));
                    if resumed && !stopping {
                        if saved.lighting_policy.enabled && saved.lighting_policy.restore_on_resume
                        {
                            if let Some(p) = &selected {
                                p.apply()?;
                            } else if saved.automatic {
                                if let Some(p) = if ac { &saved.ac } else { &saved.battery } {
                                    blade_profiles::apply(
                                        p,
                                        if ac {
                                            saved.display_ac.as_ref()
                                        } else {
                                            saved.display_battery.as_ref()
                                        },
                                    )?;
                                }
                            }
                            let lighting = selected
                                .as_ref()
                                .map(|p| &p.lighting)
                                .or(manual.as_ref().map(|(_, p)| p))
                                .or(if ac {
                                    saved.lighting_ac.as_ref()
                                } else {
                                    saved.lighting_battery.as_ref()
                                })
                                .or(baseline.as_ref().map(|p| &p.lighting));
                            if let Some(p) = lighting {
                                p.apply(&blade_profiles::journal_path("resume-lighting")?)?;
                            }
                            active = next.clone();
                            applied = None;
                            power_lighting = None;
                            let _ = sender.send_activity(
                                "Resume settings restored · panel and modes revalidated".into(),
                                Some(crate::profile_activity::Activity::new(
                                    "Restored settings",
                                    "Resumed from sleep",
                                    "enabled profile and lighting settings",
                                )),
                            );
                            ctx.request_repaint();
                        }
                        resumed = false;
                    }
                    if let Some(profile) = selected.as_ref().filter(|_| active != next) {
                        if baseline.is_none() {
                            baseline = Some(Named::capture("Automation recovery".into())?);
                        }
                        profile.apply()?;
                        active = next;
                        applied = None;
                        let _ = sender.send_activity(
                            format!("Application profile: {}", profile.name),
                            Some(crate::profile_activity::Activity::named(
                                profile,
                                "Foreground application changed",
                            )),
                        );
                        ctx.request_repaint();
                    } else if selected.is_none() && active.is_some() {
                        if !stopping && saved.automatic {
                            let hardware = if ac { &saved.ac } else { &saved.battery };
                            let display =
                                if ac { &saved.display_ac } else { &saved.display_battery };
                            if let Some(hardware) = hardware {
                                blade_profiles::apply(hardware, display.as_ref())?;
                            }
                            let p = if ac { &saved.lighting_ac } else { &saved.lighting_battery };
                            if let Some(p) = p {
                                p.apply(&blade_profiles::journal_path("app-return")?)?;
                            } else if let Some(b) = &baseline {
                                b.lighting
                                    .apply(&blade_profiles::journal_path("app-return-lighting")?)?;
                            }
                        } else if let Some(b) = &baseline {
                            let mut b = b.clone();
                            if !ac && b.hardware.validate(true).is_err() {
                                b.hardware.performance = librazer::types::PerfMode::Balanced;
                            }
                            b.apply()?;
                        }
                        active = None;
                        applied = None;
                        let _ = sender.send_activity(
                            "Application profile ended · previous settings restored".into(),
                            Some(crate::profile_activity::Activity::new(
                                "Restored settings",
                                "Foreground application profile ended",
                                "power profile or previous settings",
                            )),
                        );
                        ctx.request_repaint();
                    }
                    if saved.lighting_policy.enabled && !stopping {
                        if lighting_baseline.is_none() {
                            let state = blade18::status()?;
                            let lighting = lighting::status()?;
                            lighting_baseline =
                                Some(Levels(state.brightness, lighting.underside, state.logo));
                        }
                        let (idle, battery) = inputs()?;
                        let p = selected
                            .as_ref()
                            .map(|p| &p.lighting)
                            .or(manual.as_ref().map(|(_, p)| p))
                            .or(if ac {
                                saved.lighting_ac.as_ref()
                            } else {
                                saved.lighting_battery.as_ref()
                            })
                            .or(baseline.as_ref().map(|p| &p.lighting));
                        let mut desired = p
                            .map(|p| levels(p, false))
                            .unwrap_or_else(|| lighting_baseline.as_ref().unwrap().clone());
                        if selected.is_none()
                            && manual.is_none()
                            && saved.automatic
                            && (if ac {
                                saved.lighting_ac.is_none()
                            } else {
                                saved.lighting_battery.is_none()
                            })
                        {
                            if let Some(hardware) = if ac { &saved.ac } else { &saved.battery } {
                                desired.0 = hardware.brightness;
                                desired.2 = hardware.logo;
                            }
                        }
                        let blank =
                            should_blank(&saved.lighting_policy, ac, idle, display_on, battery);
                        let target = if blank {
                            Levels(0, false, librazer::types::LogoMode::Off)
                        } else {
                            desired.clone()
                        };
                        unblank = Some(desired);
                        let key = serde_json::to_string(&(ac, p))?;
                        if selected.is_none()
                            && manual.is_none()
                            && (if ac {
                                saved.lighting_ac.is_some()
                            } else {
                                saved.lighting_battery.is_some()
                            })
                            && power_lighting.as_ref() != Some(&key)
                        {
                            let mut effective = p.expect("Saved lighting exists").clone();
                            effective.brightness = target.0;
                            effective.underside = target.1;
                            effective.logo = target.2;
                            effective
                                .apply(&blade_profiles::journal_path("lighting-policy-preset")?)?;
                            power_lighting = Some(key);
                            applied = None;
                        }
                        if applied.as_ref() != Some(&target) {
                            apply_levels(&target)?;
                            applied = Some(target);
                            let _ = sender.send(
                                if blank {
                                    "Lighting paused by power policy"
                                } else {
                                    "Lighting restored by power policy"
                                }
                                .into(),
                            );
                            ctx.request_repaint();
                        }
                    } else if applied.is_some() {
                        if let Some(target) = unblank.as_ref() {
                            apply_levels(target)?;
                        } else if let Some(b) = &baseline {
                            apply_levels(&levels(&b.lighting, false))?;
                        }
                        applied = None;
                        power_lighting = None;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    retry_pending = true;
                    retry_at = std::time::Instant::now() + Duration::from_secs(30);
                    let _ = sender.send(format!("Automation: {error:#}"));
                    ctx.request_repaint();
                } else if !skip_retry {
                    retry_pending = false;
                }
                if stopping {
                    return;
                }
            }
        });
        (Self { events, thread: Some(thread) }, receiver)
    }
    pub fn wake(&self) {
        let _ = self.events.send(Event::Wake);
    }
    pub fn sender(&self) -> mpsc::Sender<Event> {
        self.events.clone()
    }
    pub fn stop(&mut self) {
        let _ = self.events.send(Event::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policies_respect_power_unknown_battery_and_disabled_timeouts() {
        let p = blade_profiles::LightingPolicy {
            enabled: true,
            idle_ac_seconds: 60,
            idle_battery_seconds: 10,
            display_off: true,
            low_battery_percent: 20,
            restore_on_resume: false,
        };
        assert!(!should_blank(&p, true, 59, true, Some(5)));
        assert!(should_blank(&p, true, 60, true, None));
        assert!(should_blank(&p, false, 10, true, None));
        assert!(!should_blank(&p, false, 0, true, None));
        assert!(should_blank(&p, false, 0, true, Some(20)));
        assert!(should_blank(&p, true, 0, false, None));
        assert!(!should_blank(
            &blade_profiles::LightingPolicy::default(),
            false,
            10000,
            false,
            Some(0)
        ));
    }
}
