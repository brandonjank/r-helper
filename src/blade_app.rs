use crate::{
    blade_desktop::{Action, Desktop},
    blade_profiles::{self, Settings},
};
use eframe::egui;
use librazer::{
    blade18::{self, Change, Profile, State},
    types::*,
};
use std::{path::PathBuf, sync::mpsc, time::Duration};

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
    power_watcher: crate::blade_power::Watcher,
    power_updates: mpsc::Receiver<crate::blade_power::Update>,
}
impl BladeApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
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
        let (power_watcher, power_updates) =
            crate::blade_power::Watcher::new(shared_settings.clone(), ctx.clone());
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
            power_watcher,
            power_updates,
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
        if self.job.is_some() {
            self.message = "Wait for the current device operation".into();
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.job = Some(Job { receiver, save_as, label: label.into() });
        self.message = format!("{label}…");
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(operation().map_err(|error| format!("{error:#}")));
            repaint.request_repaint();
        });
    }
    fn persist(&mut self) {
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
                if self.job.is_some() {
                    self.message = "Wait for the current operation before exiting".into();
                } else {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
    fn complete(&mut self) {
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
        if ctx.input(|input| input.viewport().close_requested()) && self.job.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.message = "Wait for the current device operation before closing".into();
        } else if ctx.input(|input| input.viewport().close_requested())
            && !self.quitting
            && self.desktop.is_some()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        ctx.request_repaint_after(Duration::from_millis(200));
        let idle = self.job.is_none();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("R-Helper · Blade 18 (2025)");
            ui.horizontal(|ui| {
                ui.label(match self.last_power { Some(true) => "Plugged in", Some(false) => "On battery", None => "Power source unknown" });
                if ui.add_enabled(idle, egui::Button::new("Refresh device state")).clicked() { self.start(ctx, "Read device state", None, blade18::status); }
                if ui.button("Copy report").clicked() { ctx.copy_text(self.report.clone()); }
                if self.desktop.is_some() && ui.button("Hide to tray").clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false)); }
                if ui.add_enabled(idle, egui::Button::new("Exit")).clicked() { self.action(ctx, Action::Quit); }
            });
            ui.label(&self.message); if self.job.is_some() { ui.spinner(); }
            if let Some(error) = &self.desktop_error { ui.colored_label(egui::Color32::DARK_RED, error); }
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut requested = None;
                if let Some(state) = &mut self.state {
                    ui.add_enabled_ui(idle, |ui| {
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("Keyboard brightness"); ui.add(egui::Slider::new(&mut state.brightness, 0..=255).suffix(" / 255"));
                            if ui.button("Apply brightness").clicked() { requested = Some(Change::Brightness(state.brightness)); }
                        });
                        ui.horizontal(|ui| { ui.label("Logo"); for mode in [LogoMode::Off, LogoMode::Static, LogoMode::Breathing] {
                            if ui.selectable_label(state.logo == mode, format!("{mode:?}")).clicked() { requested = Some(Change::Logo(mode)); }
                        } });
                        ui.horizontal(|ui| { ui.label("Battery care (80% target)"); for mode in [BatteryCare::Enable, BatteryCare::Disable] {
                            if ui.selectable_label(state.battery_care == mode, format!("{mode:?}")).clicked() { requested = Some(Change::BatteryCare(mode)); }
                        } });
                        ui.horizontal(|ui| { ui.label("Performance"); for mode in blade18::MODES {
                            let available = self.last_power == Some(true) || matches!(mode, PerfMode::Balanced | PerfMode::Silent);
                            if ui.add_enabled(available, egui::Button::new(format!("{mode:?}")).selected(state.performance == *mode)).clicked() { requested = Some(Change::Performance(*mode)); }
                        } });
                        if state.performance == PerfMode::Custom {
                            let mut cpu = state.cpu.unwrap_or(CpuBoost::Low); let mut gpu = state.gpu.unwrap_or(GpuBoost::Low);
                            ui.horizontal(|ui| { ui.label("CPU"); for level in [CpuBoost::Low, CpuBoost::Medium, CpuBoost::High, CpuBoost::Boost] { ui.selectable_value(&mut cpu, level, format!("{level:?}")); } });
                            ui.horizontal(|ui| { ui.label("GPU"); for level in [GpuBoost::Low, GpuBoost::Medium, GpuBoost::High] { ui.selectable_value(&mut gpu, level, format!("{level:?}")); }
                                if ui.button("Apply CPU/GPU").clicked() { requested = Some(Change::Boosts(cpu, gpu)); } });
                            state.cpu = Some(cpu); state.gpu = Some(gpu);
                        }
                        ui.horizontal(|ui| { ui.label(format!("Fan zones: {} / {} RPM actual · {:?}", state.fan_actual[0], state.fan_actual[1], state.fan_mode));
                            if ui.button("Automatic fans").clicked() { requested = Some(Change::AutoFans); } });
                        ui.add_enabled_ui(blade18::MANUAL_FANS_VERIFIED && state.performance == PerfMode::Balanced, |ui| { ui.horizontal(|ui| {
                            ui.add(egui::Slider::new(&mut self.manual_rpm, blade18::FAN_MIN..=blade18::FAN_MAX).step_by(100.0).suffix(" RPM"));
                            if ui.button("Apply manual fans").clicked() { requested = Some(Change::ManualFans(self.manual_rpm)); }
                        }); });
                        ui.label("Manual fan targets are experimental. Actual RPM can differ and ramps slowly; use Refresh to check.");
                    });
                }
                if let Some(change) = requested { self.start(ctx, "Apply setting", None, move || blade18::change(change)); }
                ui.separator(); ui.strong("Saved profiles");
                ui.add_enabled_ui(idle && self.settings_error.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Save current as AC").clicked() { self.start(ctx, "Save AC profile", Some(true), blade18::status); }
                        if ui.button("Save current as battery").clicked() { self.start(ctx, "Save battery profile", Some(false), blade18::status); }
                        if ui.add_enabled(self.settings.ac.is_some(), egui::Button::new("Apply AC profile")).clicked() { self.apply_saved(ctx, true); }
                        if ui.add_enabled(self.settings.battery.is_some(), egui::Button::new("Apply battery profile")).clicked() { self.apply_saved(ctx, false); }
                    });
                    if ui.add_enabled(self.settings.ac.is_some() && self.settings.battery.is_some(), egui::Checkbox::new(&mut self.settings.automatic, "Automatically switch on AC/battery transitions")).changed() { self.persist(); }
                });
                if let Some(error) = &self.settings_error { ui.colored_label(egui::Color32::DARK_RED, error); }
                ui.label("Ctrl+Shift+Alt: F12 show · F5 cycle mode · F6 AC profile · F7 battery profile");
                ui.collapsing("Device report", |ui| { ui.add(egui::TextEdit::multiline(&mut self.report).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).interactive(false).desired_rows(18)); });
            });
        });
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.power_watcher.stop();
        if self.state.as_ref().is_some_and(|state| state.fan_mode == FanMode::Manual) {
            let _ = blade18::change(Change::AutoFans);
        }
    }
}
pub fn run() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("R-Helper · Blade 18")
            .with_inner_size([840.0, 650.0]),
        ..Default::default()
    };
    eframe::run_native("R-Helper Blade 18", options, Box::new(|cc| Ok(Box::new(BladeApp::new(cc)))))
}
