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
mod curves;
mod view;

struct Job {
    receiver: mpsc::Receiver<Result<State, String>>,
    save_as: Option<bool>,
    label: String,
}
struct BladeApp {
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
            preview: false,
            curve_settings,
            curve_error,
            curve_events: None,
            curve_status: None,
            curve_control,
        };
        app.start(ctx, "Read device state", None, blade18::status);
        app
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        label: &str,
        save_as: Option<bool>,
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
        self.job = Some(Job { receiver, save_as, label: label.into() });
        self.message = format!("{label}…");
        let repaint = ctx.clone();
        let curves = self.curve_control.clone();
        std::thread::spawn(move || {
            let _ = sender.send(
                crate::blade_curves::stop_and_wait(&curves)
                    .and_then(|_| operation())
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
        let profile = if ac { self.settings.ac.clone() } else { self.settings.battery.clone() };
        if let Some(profile) = profile {
            self.start(
                ctx,
                if ac { "Apply AC profile" } else { "Apply battery profile" },
                None,
                move || blade18::apply_profile(&profile),
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
                    self.start(ctx, "Cycle performance mode", None, move || {
                        blade18::change(Change::Performance(next))
                    });
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
                    self.report = serde_json::to_string_pretty(&report).unwrap_or_default();
                    if report["lighting_controls"].is_object() {
                        self.lighting = serde_json::from_value(report["lighting_controls"].clone()).ok();
                    } else if report["thermal_capabilities"].is_object() {
                        self.capability_report = Some(report);
                    } else {
                        self.sensor_report = Some(report);
                    }
                    self.sensor_job = None;
                    self.message = "Controller query complete · exported report available".into();
                }
                Ok(Err(error)) => {
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
        self.complete();
        if let Some(desktop) = &self.desktop {
            let events = desktop.events();
            for action in events {
                self.action(ctx, action);
            }
        }
        while let Ok(update) = self.power_updates.try_recv() {
            self.last_power = update.ac;
            if let Some(error) = update.monitor_error {
                self.power_error = Some(error);
            }
            if let Some(result) = update.result {
                match result {
                    Ok(state) => {
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
        if let Some(watcher) = &mut self.power_watcher {
            watcher.stop();
        }
        let _ = crate::blade_curves::stop_and_wait(&self.curve_control);
        if self.state.as_ref().is_some_and(|state| state.fan_mode == FanMode::Manual) {
            let _ = blade18::change(Change::AutoFans);
        }
    }
}
pub fn run() -> Result<(), eframe::Error> {
    run_with_preview(false)
}
pub fn run_preview() -> Result<(), eframe::Error> {
    run_with_preview(true)
}
fn run_with_preview(preview: bool) -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(crate::load_icon())
            .with_title(if preview { "R-Helper · UI Preview" } else { "R-Helper · Blade 18" })
            .with_inner_size([540.0, 790.0])
            .with_min_inner_size([480.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "R-Helper Blade 18",
        options,
        Box::new(move |cc| {
            Ok(Box::new(if preview { BladeApp::preview(cc) } else { BladeApp::new(cc) }))
        }),
    )
}
