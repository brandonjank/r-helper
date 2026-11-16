use crate::profile_activity as activity;
use crate::{
    blade_desktop::{Action, Desktop},
    blade_profiles::{self, Settings},
};
use eframe::egui;
use librazer::{
    blade18::{self, Change, Profile, State},
    types::*,
};
use std::{path::PathBuf, sync::mpsc};
mod batch;
mod curves;
mod extended;
mod view;

struct Job {
    receiver: mpsc::Receiver<Result<State, String>>,
    save_as: Option<bool>,
    label: String,
    activity: Option<activity::Activity>,
}
struct BladeApp {
    section: view::Section,
    activity: activity::Activity,
    benchmark_exit: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    vendor: crate::blade_vendor::Ui,
    extended: Option<serde_json::Value>,
    controller_telemetry: Option<serde_json::Value>,
    platform_report: Option<serde_json::Value>,
    updates_report: Option<serde_json::Value>,
    lamp_report: Option<serde_json::Value>,
    lamp_color: [u8; 3],
    lamp_highlight: bool,
    lamp_index: u8,
    batch: batch::BatchUi,
    state: Option<State>,
    job: Option<Job>,
    message: String,
    report: String,
    settings: Settings,
    settings_path: Option<PathBuf>,
    settings_error: Option<String>,
    desktop: Option<Desktop>,
    desktop_error: Option<String>,
    last_power: Option<bool>,
    quitting: bool,
    manual_rpm: u16,
    shared_settings: std::sync::Arc<std::sync::Mutex<Settings>>,
    power_watcher: Option<crate::blade_power::Watcher>,
    power_updates: mpsc::Receiver<crate::blade_power::Update>,
    power_error: Option<String>,
    sensor_job: Option<mpsc::Receiver<Result<serde_json::Value, String>>>,
    sensor_report: Option<serde_json::Value>,
    capability_report: Option<serde_json::Value>,
    lighting: Option<blade18::lighting::Lighting>,
    keyboard_color: [u8; 3],
    underside_brightness: u8,
    keyboard_effect: blade18::lighting::KeyboardEffect,
    display: Option<librazer::display::Info>,
    preview: bool,
    curve_settings: blade18::CurveSettings,
    curve_error: Option<String>,
    curve_events: Option<mpsc::Receiver<serde_json::Value>>,
    curve_status: Option<serde_json::Value>,
    curve_control: crate::blade_curves::Shared,
}
impl BladeApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        view::configure(ctx);
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let hwnd = match cc.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
            _ => 0,
        };
        let settings_path = blade_profiles::path().ok();
        let loaded = settings_path.as_ref().map(|path| blade_profiles::load(path));
        let (settings, settings_error) = match loaded {
            Some(Ok(settings)) => (settings, None),
            Some(Err(error)) => (Settings::default(), Some(format!("{error:#}"))),
            None => (Settings::default(), Some("Settings directory is unavailable".into())),
        };
        let (desktop, desktop_error) = match Desktop::new(ctx, hwnd) {
            Ok(desktop) => (Some(desktop), None),
            Err(error) => (None, Some(format!("Tray/hotkeys unavailable: {error:#}"))),
        };
        let shared_settings = std::sync::Arc::new(std::sync::Mutex::new(settings.clone()));
        let curve_control = crate::blade_curves::shared();
        let (curve_settings, curve_error) = match crate::blade_curves::load() {
            Ok(value) => (value, None),
            Err(error) => (
                blade18::CurveSettings::default(),
                Some(format!("Curve settings preserved: {error:#}")),
            ),
        };
        let (power_watcher, power_updates) = crate::blade_power::Watcher::new(
            shared_settings.clone(),
            ctx.clone(),
            curve_control.clone(),
        );
        let mut app = Self {
            section: view::Section::Overview,
            activity: activity::Activity::default(),
            benchmark_exit: None,
            vendor: crate::blade_vendor::Ui::default(),
            extended: None,
            controller_telemetry: None,
            platform_report: None,
            updates_report: None,
            lamp_report: None,
            lamp_color: [32, 64, 255],
            lamp_highlight: false,
            lamp_index: 1,
            batch: batch::BatchUi::default(),
            state: None,
            job: None,
            message: String::new(),
            report: String::new(),
            settings,
            settings_path,
            settings_error,
            desktop,
            desktop_error,
            last_power: blade18::ac_connected().ok(),
            quitting: false,
            manual_rpm: blade18::FAN_MIN,
            shared_settings,
            power_watcher: Some(power_watcher),
            power_updates,
            power_error: None,
            sensor_job: None,
            sensor_report: None,
            capability_report: None,
            lighting: None,
            keyboard_color: [32, 64, 255],
            underside_brightness: 64,
            keyboard_effect: blade18::lighting::KeyboardEffect::Static,
            display: None,
            preview: false,
            curve_settings,
            curve_error,
            curve_events: None,
            curve_status: None,
            curve_control,
        };
        let (watcher, updates) = crate::blade_policy::Watcher::new(
            app.shared_settings.clone(),
            ctx.clone(),
            app.curve_control.clone(),
        );
        app.batch.watcher = Some(watcher);
        if app.settings.manage_razer_software && !crate::blade_benchmark::requested() {
            app.vendor.start(ctx, app.curve_control.clone());
        }
        app.batch.updates = Some(updates);
        app.start(ctx, "Read device state", None, None, blade18::status);
        app
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        label: &str,
        save_as: Option<bool>,
        activity: Option<activity::Activity>,
        operation: impl FnOnce() -> anyhow::Result<State> + Send + 'static,
    ) {
        if self.preview {
            return;
        }
        if self.job.is_some() || self.sensor_job.is_some() {
            self.message = "Wait for the current device operation".into();
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.job = Some(Job { receiver, save_as, label: label.into(), activity: activity.clone() });
        self.message = format!("{label}…");
        if activity.is_some() {
            self.display = None;
            self.lighting = None;
        }
        let repaint = ctx.clone();
        let curves = self.curve_control.clone();
        let policy = self.batch.watcher.as_ref().map(|w| w.sender());
        let manual = activity.is_some();
        std::thread::spawn(move || {
            let _ = sender.send(
                crate::blade_curves::stop_and_wait(&curves)
                    .and_then(|_| crate::blade_profiles::operation_lock())
                    .and_then(|_lock| {
                        crate::blade_curves::stop_and_wait(&curves)?;
                        let result = operation()?;
                        if manual {
                            if let (Some(sender), Ok(snapshot)) =
                                (policy, blade_profiles::Named::capture("Manual settings".into()))
                            {
                                let _ = sender.send(crate::blade_power::Event::Baseline(snapshot));
                            }
                        }
                        Ok(result)
                    })
                    .map_err(|error| format!("{error:#}")),
            );
            repaint.request_repaint();
        });
    }
    fn persist(&mut self) {
        if self.preview {
            return;
        }
        if self.settings_error.is_some() {
            self.message = "Settings file needs repair; original preserved".into();
            return;
        }
        self.message = match self
            .settings_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No settings path"))
            .and_then(|path| blade_profiles::save(path, &self.settings))
        {
            Ok(()) => {
                if let Ok(mut shared) = self.shared_settings.lock() {
                    *shared = self.settings.clone();
                }
                if let Some(watcher) = &self.batch.watcher {
                    watcher.wake();
                }
                "Profiles saved".into()
            }
            Err(error) => {
                if let Ok(shared) = self.shared_settings.lock() {
                    self.settings = shared.clone();
                }
                format!("Could not save profiles: {error:#}")
            }
        };
    }
    fn apply_saved(&mut self, ctx: &egui::Context, ac: bool) {
        self.display = None;
        let profile = if ac { self.settings.ac.clone() } else { self.settings.battery.clone() };
        let display = if ac {
            self.settings.display_ac.clone()
        } else {
            self.settings.display_battery.clone()
        };
        let lighting = if ac {
            self.settings.lighting_ac.clone()
        } else {
            self.settings.lighting_battery.clone()
        };
        if let Some(profile) = profile {
            self.start(
                ctx,
                if ac { "Apply AC profile" } else { "Apply battery profile" },
                None,
                Some(activity::Activity::power(
                    ac,
                    &self.settings,
                    "Applied manually or with a shortcut",
                )),
                move || {
                    let state = blade_profiles::apply(&profile, display.as_ref())?;
                    if let Some(p) = lighting {
                        p.apply(&blade_profiles::journal_path("manual-power-lighting")?)?;
                    }
                    Ok(state)
                },
            );
        } else {
            self.message = "Save that profile first".into();
        }
    }
    fn action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Show => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            Action::Ac => self.apply_saved(ctx, true),
            Action::Battery => self.apply_saved(ctx, false),
            Action::Cycle => {
                if let Some(state) = &self.state {
                    let modes = if self.last_power == Some(true) {
                        blade18::MODES
                    } else {
                        &blade18::MODES[..2]
                    };
                    let next = modes[(modes
                        .iter()
                        .position(|mode| *mode == state.performance)
                        .unwrap_or(0)
                        + 1)
                        % modes.len()];
                    self.start(
                        ctx,
                        "Cycle performance mode",
                        None,
                        Some(activity::Activity::new(
                            "Manual settings",
                            "Performance shortcut",
                            "performance",
                        )),
                        move || blade18::change(Change::Performance(next)),
                    );
                }
            }
            Action::Quit => {
                if self.job.is_some() || self.sensor_job.is_some() {
                    self.message = "Wait for the current operation before exiting".into();
                } else {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
    fn complete(&mut self) {
        let mut curve_finished = None;
        if let Some(receiver) = &self.curve_events {
            while let Ok(event) = receiver.try_recv() {
                if !event["finished"].is_null() {
                    curve_finished = Some(event["finished"].clone());
                } else {
                    if event["phase"] == "running"
                        && self.curve_status.as_ref().is_none_or(|old| old["phase"] != "running")
                        && self.job.is_none()
                    {
                        self.message = "Custom curves active · supervised session".into();
                    }
                    if let Ok(sample) =
                        serde_json::from_value::<librazer::thermal::Sample>(event["sample"].clone())
                    {
                        if let Some(state) = &mut self.state {
                            state.fan_actual = [sample.actual_rpm[0], sample.actual_rpm[1]];
                            state.fan_setpoints = [sample.target_rpm[0], sample.target_rpm[1]];
                            state.fan_mode = if sample.modes[0][1] == 1 {
                                FanMode::Manual
                            } else {
                                FanMode::Auto
                            };
                        }
                        let sensors =
                            self.sensor_report.get_or_insert_with(|| serde_json::json!({}));
                        sensors["sensor_candidates"]["getters"]["ec_temperature_fields"] = serde_json::json!({"ok":true,"response_args":[2,sample.ec_celsius[0],sample.ec_celsius[1],0]});
                        sensors["sensor_candidates"]["getters"]["fan_zone_3_actual_rpm"] = serde_json::json!({"ok":true,"response_args":[0,3,sample.actual_rpm[2]/100]});
                    }
                    self.curve_status = Some(event);
                }
            }
        }
        if let Some(result) = curve_finished {
            if let Some(watcher) = &self.batch.watcher {
                watcher.wake();
            }
            self.curve_events = None;
            self.message = if result["recovery_verified"] == true {
                format!(
                    "Curves stopped · automatic cooling verified. {}",
                    result["status"]["reason"].as_str().or(result["reason"].as_str()).unwrap_or("")
                )
            } else {
                format!("Curve recovery needs attention: {result}")
            };
            if let Ok(sample) =
                serde_json::from_value::<librazer::thermal::Sample>(result["recovered"].clone())
            {
                if let Some(state) = &mut self.state {
                    state.fan_mode = FanMode::Auto;
                    state.fan_actual = [sample.actual_rpm[0], sample.actual_rpm[1]];
                    state.fan_setpoints = [sample.target_rpm[0], sample.target_rpm[1]];
                }
            }
            self.curve_status = Some(result);
        }
        if let Some(receiver) = &self.sensor_job {
            match receiver.try_recv() {
                Ok(Ok(report)) => {
                    if let Ok(activity) = serde_json::from_value(report["applied_activity"].clone())
                    {
                        self.activity = activity;
                    }
                    let completion_message = if report["lamp_preview"].is_object() {
                        "Auxiliary test ended · saved firmware lighting reapplied · confirm appearance"
                    } else if report["keyboard_batch"]["passed"] == true {
                        "Keyboard checks passed · original values restored"
                    } else {
                        "Controller query complete · exported report available"
                    };
                    self.report = serde_json::to_string_pretty(&report).unwrap_or_default();
                    if report["device_state"].is_object() {
                        self.state = serde_json::from_value(report["device_state"].clone()).ok();
                    }
                    if report["platform_inventory"].is_object() {
                        self.platform_report = Some(report["platform_inventory"].clone());
                    }
                    if report["updates_inventory"].is_object() {
                        self.updates_report = Some(report["updates_inventory"].clone());
                    }
                    if report["lighting_controls"].is_object() {
                        self.lighting =
                            serde_json::from_value(report["lighting_controls"].clone()).ok();
                        if let Some(lighting) = &self.lighting {
                            self.underside_brightness = lighting.underside_brightness;
                        }
                        if let Some(lighting) = &self.lighting {
                            if let Ok(preset) = blade18::lighting::Preset::capture(
                                lighting,
                                self.state.as_ref().map(|s| s.logo).unwrap_or(LogoMode::Static),
                            ) {
                                self.batch.draft = preset;
                            }
                        }
                        if let Some(id) =
                            self.lighting.as_ref().and_then(|s| s.keyboard_effect.get(2))
                        {
                            self.keyboard_effect = match id {
                                2 => blade18::lighting::KeyboardEffect::Breathing,
                                3 => blade18::lighting::KeyboardEffect::Spectrum,
                                _ => blade18::lighting::KeyboardEffect::Static,
                            };
                        }
                        if let Some(effect) = self.lighting.as_ref().map(|v| &v.keyboard_effect) {
                            if effect.len() == 9 && matches!(effect[2], 1 | 2) {
                                self.keyboard_color.copy_from_slice(&effect[6..9]);
                            }
                        }
                    } else if report["extended_controls"].is_array() {
                        self.extended = Some(report["extended_controls"].clone());
                        self.controller_telemetry = Some(report["controller_telemetry"].clone());
                        if report["lamp_inventory"].is_object() {
                            self.lamp_report = Some(report["lamp_inventory"].clone());
                        }
                    } else if report["display_controls"].is_object() {
                        self.batch.display_mode = None;
                        self.display =
                            serde_json::from_value(report["display_controls"].clone()).ok();
                        if let Some(path) = report["display_pending"].as_str() {
                            self.batch.pending =
                                Some((PathBuf::from(path), std::time::Instant::now()));
                        }
                    } else if report["named_update"].is_object() {
                        let updated = (|| -> anyhow::Result<()> {
                            let index = report["named_update"]["index"]
                                .as_u64()
                                .ok_or_else(|| anyhow::anyhow!("Invalid profile index"))?
                                as usize;
                            let p: blade_profiles::Named =
                                serde_json::from_value(report["named_update"]["profile"].clone())?;
                            p.validate()?;
                            anyhow::ensure!(
                                index < self.settings.named.len(),
                                "Profile no longer exists"
                            );
                            self.settings.named[index] = p;
                            Ok(())
                        })();
                        match updated {
                            Ok(()) => self.persist(),
                            Err(e) => self.message = format!("Profile update failed: {e:#}"),
                        }
                    } else if report["named_capture"].is_object() {
                        match serde_json::from_value::<blade_profiles::Named>(
                            report["named_capture"].clone(),
                        ) {
                            Ok(profile) => {
                                self.settings.named.push(profile);
                                self.persist();
                            }
                            Err(e) => self.message = format!("Profile capture failed: {e}"),
                        }
                    } else if let Some(active) = report["charge_once"].as_bool() {
                        self.batch.charge = Some(active);
                    } else if report["thermal_capabilities"].is_object() {
                        self.capability_report = Some(report);
                    } else {
                        self.sensor_report = Some(report);
                    }
                    self.sensor_job = None;
                    self.message = completion_message.into();
                }
                Ok(Err(error)) => {
                    self.display = None;
                    self.lighting = None;
                    self.sensor_report = None;
                    self.sensor_job = None;
                    self.message = error;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.lighting = None;
                    self.sensor_job = None;
                    self.sensor_report = None;
                    self.message = "Sensor worker stopped; retry the sensor query".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let result = self.job.as_ref().map(|job| job.receiver.try_recv());
        match result {
            Some(Ok(result)) => {
                let job = self.job.take().unwrap();
                match result {
                    Ok(state) => {
                        if let Some(activity) = job.activity {
                            self.activity = activity;
                        }
                        self.report = serde_json::to_string_pretty(&state).unwrap_or_default();
                        self.message = format!("{}: readback verified", job.label);
                        if let Some(ac) = job.save_as {
                            let profile = Profile::capture(&state);
                            match profile.validate(!ac) {
                                Ok(()) => {
                                    if ac {
                                        self.settings.ac = Some(profile);
                                    } else {
                                        self.settings.battery = Some(profile);
                                    }
                                    self.persist();
                                }
                                Err(error) => {
                                    self.message = format!("Profile not saved: {error:#}")
                                }
                            }
                        }
                        self.state = Some(state);
                    }
                    Err(error) => {
                        self.message = error;
                        self.state = None;
                    }
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.job = None;
                self.state = None;
                self.message = "Device worker stopped unexpectedly; refresh state".into();
            }
            _ => {}
        }
    }
}
impl eframe::App for BladeApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.benchmark_exit.as_ref().is_some_and(|v| v.load(std::sync::atomic::Ordering::SeqCst))
        {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.complete();
        self.batch_updates(ctx);
        if let Some(desktop) = &self.desktop {
            let events = desktop.events();
            for action in events {
                self.action(ctx, action);
            }
        }
        while let Ok(update) = self.power_updates.try_recv() {
            if update.ac != self.last_power {
                self.display = None;
            }
            self.last_power = update.ac;
            if let Some(error) = update.monitor_error {
                self.power_error = Some(error);
            }
            if let Some(result) = update.result {
                match result {
                    Ok(state) => {
                        if let Some(activity) = update.activity {
                            self.activity = activity;
                        }
                        self.report = serde_json::to_string_pretty(&state).unwrap_or_default();
                        self.state = Some(state);
                        self.message = if update.ac == Some(true) {
                            "Automatic AC profile: readback verified"
                        } else {
                            "Automatic battery profile: readback verified"
                        }
                        .into();
                    }
                    Err(error) => {
                        self.message = format!("Automatic profile failed: {error}");
                        self.state = None;
                    }
                }
            }
        }
        if ctx.input(|input| input.viewport().close_requested())
            && (self.job.is_some() || self.sensor_job.is_some())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.message = "Wait for the current device operation before closing".into();
        } else if ctx.input(|input| input.viewport().close_requested())
            && !self.quitting
            && self.desktop.is_some()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        let idle = self.job.is_none() && self.sensor_job.is_none();
        self.render(ctx, idle);
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        if self.preview {
            return;
        }
        if let Some((path, _)) = self.batch.pending.take() {
            let _ = crate::blade_display::finish(&path, false);
        }
        if let Some(watcher) = &mut self.power_watcher {
            watcher.stop();
        }
        let _ = crate::blade_curves::stop_and_wait(&self.curve_control);
        if let Some(watcher) = &mut self.batch.watcher {
            watcher.stop();
        }
        if self.state.as_ref().is_some_and(|state| state.fan_mode == FanMode::Manual) {
            let _ = blade18::change(Change::AutoFans);
        }
        self.vendor.shutdown();
        blade18::lamp_array::release();
    }
}
pub fn run() -> Result<(), eframe::Error> {
    run_with_preview(false)
}
pub fn run_preview() -> Result<(), eframe::Error> {
    run_with_preview(true)
}
fn run_with_preview(preview: bool) -> Result<(), eframe::Error> {
    let benchmark = if preview {
        None
    } else {
        crate::blade_benchmark::config().map_err(|e| eframe::Error::AppCreation(e.into()))?
    };
    let hidden = benchmark.as_ref().is_some_and(|b| b.hidden);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(crate::load_icon())
            .with_visible(!hidden)
            .with_title(if preview { "R-Helper · UI Preview" } else { "R-Helper · Blade 18" })
            .with_inner_size([540.0, 790.0])
            .with_min_inner_size([480.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "R-Helper Blade 18",
        options,
        Box::new(move |cc| {
            let mut app = if preview { BladeApp::preview(cc) } else { BladeApp::new(cc) };
            if let Some(config) = benchmark {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let hwnd = match cc.window_handle().map(|h| h.as_raw()) {
                    Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
                    _ => 0,
                };
                app.benchmark_exit =
                    Some(crate::blade_benchmark::start(config, cc.egui_ctx.clone(), hwnd));
            }
            Ok(Box::new(app))
        }),
    )
}
