use super::*;
use egui::{Color32, RichText, Stroke, Vec2};

const ACCENT: Color32 = Color32::from_rgb(105, 220, 145);
const MUTED: Color32 = Color32::from_rgb(155, 166, 176);
const SURFACE: Color32 = Color32::from_rgb(29, 33, 39);

pub(super) fn configure(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(19, 22, 27);
    visuals.selection.bg_fill = Color32::from_rgb(39, 88, 60);
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(42, 47, 55);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(53, 64, 65);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 8.0);
        style.spacing.slider_width = 225.0;
    });
}

fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(SURFACE).corner_radius(10).inner_margin(14).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong().size(14.0));
        ui.add_space(3.0);
        body(ui);
    });
    ui.add_space(4.0);
}

fn hint(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).small().color(MUTED));
}

fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        hint(ui, label);
        ui.label(RichText::new(value).size(20.0).strong());
    });
}

fn temperature(report: Option<&serde_json::Value>, index: usize) -> String {
    let Some(reading) = report.map(|r| &r["sensor_candidates"]["getters"]["ec_temperature_fields"])
    else {
        return "—".into();
    };
    if reading["ok"] != true {
        return "Unavailable".into();
    }
    match reading["response_args"][index].as_u64() {
        Some(value @ 1..=125) => format!("{value} °C"),
        _ => "Unavailable".into(),
    }
}

impl BladeApp {
    /// Fixture-only window: no settings, hardware reads, tray, hotkeys or power watcher.
    pub(super) fn preview(cc: &eframe::CreationContext<'_>) -> Self {
        configure(&cc.egui_ctx);
        let state = State {
            brightness: 176,
            logo: LogoMode::Static,
            battery_care: BatteryCare::Disable,
            performance: PerfMode::Balanced,
            fan_mode: FanMode::Auto,
            fan_setpoints: [2000, 2000],
            fan_actual: [2000, 2000],
            cpu: Some(CpuBoost::Medium),
            gpu: Some(GpuBoost::Low),
        };
        let settings = Settings { ac: Some(Profile::capture(&state)), ..Settings::default() };
        let (_, power_updates) = mpsc::channel();
        Self {
            state: Some(state),
            job: None,
            message: "UI preview · sample data · hardware controls disabled".into(),
            report: "Preview only. No hardware accessed.".into(),
            settings: settings.clone(),
            settings_path: None,
            settings_error: None,
            desktop: None,
            desktop_error: None,
            last_power: Some(true),
            quitting: false,
            manual_rpm: blade18::FAN_MIN,
            shared_settings: std::sync::Arc::new(std::sync::Mutex::new(settings)),
            power_watcher: None,
            power_updates,
            power_error: None,
            sensor_job: None,
            sensor_report: Some(serde_json::json!({"sensor_candidates":{"getters":{
                "ec_temperature_fields":{"ok":true,"response_args":[2,69,0,0]},
                "fan_zone_3_actual_rpm":{"ok":true,"response_args":[0,3,0]}
            }}})),
            preview: true,
        }
    }

    pub(super) fn render(&mut self, ctx: &egui::Context, idle: bool) {
        let enabled = idle && !self.preview;
        let footer_height = 105.0
            + 20.0
                * [&self.desktop_error, &self.power_error, &self.settings_error]
                    .into_iter()
                    .flatten()
                    .count() as f32;
        let mut requested = None;
        let mut read_sensors = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::from_rgb(19,22,27)).inner_margin(18)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("R-Helper").size(24.0).strong());
                    hint(ui, "BLADE 18  ·  2025  ·  RTX 5090");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(enabled, egui::Button::new("Refresh")).clicked() { self.start(ctx, "Read device state", None, blade18::status); }
                    ui.label(RichText::new(match self.last_power { Some(true) => "AC POWER", Some(false) => "BATTERY", None => "UNKNOWN POWER" }).small().color(ACCENT));
                });
            });
            ui.add_space(10.0);
            if self.preview { ui.colored_label(ACCENT, "PREVIEW · sample data · controls disabled"); }
            egui::ScrollArea::vertical().max_height((ui.available_height() - footer_height).max(180.0)).auto_shrink([false, false]).show(ui, |ui| {
                if let Some(state) = &mut self.state {
                    card(ui, "Performance", |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            ui.horizontal(|ui| {
                                let width = (ui.available_width() - 24.0) / 4.0;
                                for mode in [PerfMode::Silent, PerfMode::Balanced, PerfMode::Performance, PerfMode::Custom] {
                                    let available = self.last_power == Some(true) || matches!(mode, PerfMode::Balanced | PerfMode::Silent);
                                    if ui.add_enabled(available, egui::Button::new(format!("{mode:?}")).min_size(Vec2::new(width, 36.0)).selected(state.performance == mode)).clicked() { requested = Some(Change::Performance(mode)); }
                                }
                            });
                            if state.performance == PerfMode::Custom {
                                let mut cpu = state.cpu.unwrap_or(CpuBoost::Low);
                                let mut gpu = state.gpu.unwrap_or(GpuBoost::Low);
                                ui.horizontal_wrapped(|ui| { ui.label("CPU boost"); for level in [CpuBoost::Low, CpuBoost::Medium, CpuBoost::High, CpuBoost::Boost] { ui.selectable_value(&mut cpu, level, format!("{level:?}")); } });
                                ui.horizontal_wrapped(|ui| { ui.label("GPU boost"); for level in [GpuBoost::Low, GpuBoost::Medium, GpuBoost::High] { ui.selectable_value(&mut gpu, level, format!("{level:?}")); } });
                                if ui.button("Apply boost levels").clicked() { requested = Some(Change::Boosts(cpu, gpu)); }
                                state.cpu = Some(cpu); state.gpu = Some(gpu);
                            }
                        });
                        hint(ui, "Firmware modes · Performance and Custom require AC power");
                    });
                    card(ui, "Fans + temperatures", |ui| {
                        ui.columns(3, |columns| {
                            metric(&mut columns[0], "FAN 1", &format!("{} RPM", state.fan_actual[0]));
                            metric(&mut columns[1], "FAN 2", &format!("{} RPM", state.fan_actual[1]));
                            let third = self.sensor_report.as_ref().and_then(|r| {
                                let reading = &r["sensor_candidates"]["getters"]["fan_zone_3_actual_rpm"];
                                (reading["ok"] == true).then(|| reading["response_args"][2].as_u64()).flatten()
                            }).map(|rpm| format!("{} RPM", rpm * 100)).unwrap_or_else(|| "—".into());
                            metric(&mut columns[2], "FAN 3 · CANDIDATE", &third);
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("EC A: {}", temperature(self.sensor_report.as_ref(), 1)));
                            ui.label(format!("EC B: {}", temperature(self.sensor_report.as_ref(), 2)));
                            if ui.add_enabled(enabled, egui::Button::new("Read sensors")).clicked() { read_sensors = true; }
                        });
                        hint(ui, "Snapshots only · CPU/GPU sensor and physical fan mapping pending");
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("Cooling: {:?}", state.fan_mode));
                            if ui.add_enabled(enabled, egui::Button::new("Automatic").selected(state.fan_mode == FanMode::Auto)).clicked() { requested = Some(Change::AutoFans); }
                        });
                        ui.collapsing("Manual fan target · experimental", |ui| {
                            ui.add_enabled_ui(enabled && blade18::MANUAL_FANS_VERIFIED && state.performance == PerfMode::Balanced && self.last_power == Some(true), |ui| {
                                ui.horizontal(|ui| {
                                    ui.add(egui::Slider::new(&mut self.manual_rpm, blade18::FAN_MIN..=blade18::FAN_MAX).step_by(100.0).suffix(" RPM"));
                                    if ui.button("Apply").clicked() { requested = Some(Change::ManualFans(self.manual_rpm)); }
                                });
                            });
                            hint(ui, "AC power and Balanced mode required. Speeds ramp slowly; refresh to check.");
                        });
                        ui.collapsing("Independent curves · in development", |ui| {
                            hint(ui, "Curve calculations are ready offline. Live control awaits validated sensors and independent fan actuation.");
                        });
                    });
                    card(ui, "Lighting", |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Keyboard");
                                ui.add(egui::Slider::new(&mut state.brightness, 0..=255).show_value(false));
                                ui.label(format!("{}%", (u16::from(state.brightness) * 100 + 127) / 255));
                                if ui.button("Apply").clicked() { requested = Some(Change::Brightness(state.brightness)); }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Logo");
                                for mode in [LogoMode::Off, LogoMode::Static, LogoMode::Breathing] {
                                    if ui.selectable_label(state.logo == mode, format!("{mode:?}")).clicked() { requested = Some(Change::Logo(mode)); }
                                }
                            });
                        });
                        hint(ui, "Independent RGB colors and underside lighting are under investigation");
                    });
                    card(ui, "Battery care", |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Charge limit");
                                for (mode, label) in [(BatteryCare::Enable, "80%"), (BatteryCare::Disable, "Full charge")] {
                                    if ui.selectable_label(state.battery_care == mode, label).clicked() { requested = Some(Change::BatteryCare(mode)); }
                                }
                            });
                        });
                        hint(ui, "Battery care requests an 80% target; charging cutoff validation is pending");
                    });
                } else {
                    card(ui, "Device unavailable", |ui| {
                        ui.label("Refresh to read the Blade 18 controls.");
                        hint(ui, "No replacement values are used when a query fails.");
                    });
                }
                // Profile actions require mutable app access after the state borrow ends.
                card(ui, "Power profiles", |ui| {
                    for ac in [true, false] {
                        let saved = if ac { &self.settings.ac } else { &self.settings.battery };
                        let has_saved = saved.is_some();
                        let mode = saved.as_ref().map(|p| format!("{:?}", p.performance)).unwrap_or_else(|| "Not saved".into());
                        ui.horizontal(|ui| {
                            ui.label(if ac { "Plugged in" } else { "On battery" });
                            ui.label(RichText::new(mode).color(ACCENT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add_enabled(enabled && self.settings_error.is_none(), egui::Button::new("Save current")).clicked() { self.start(ctx, if ac { "Save AC profile" } else { "Save battery profile" }, Some(ac), blade18::status); }
                                if ui.add_enabled(enabled && has_saved, egui::Button::new("Apply")).clicked() { self.apply_saved(ctx, ac); }
                            });
                        });
                    }
                    if ui.add_enabled(enabled && self.settings_error.is_none() && self.settings.ac.is_some() && self.settings.battery.is_some(), egui::Checkbox::new(&mut self.settings.automatic, "Switch profiles when power source changes")).changed() { self.persist(); }
                });
                card(ui, "More", |ui| {
                    ui.collapsing("Display + NVIDIA tuning", |ui| {
                        hint(ui, "Refresh-rate switching and NVIDIA tuning are planned. Controls will appear after support is implemented.");
                    });
                    ui.collapsing("BIOS + drivers", |ui| {
                        if let Some(report) = &self.sensor_report {
                            if let Some(version) = report["bios"]["BIOSVersion"].as_str() { ui.label(format!("BIOS snapshot: {version}")); }
                            let firmware = &report["sensor_candidates"]["getters"]["controller_firmware"];
                            if firmware["ok"] == true {
                                let bytes = &firmware["response_args"];
                                if let (Some(major), Some(minor)) = (bytes[0].as_u64(), bytes[1].as_u64()) { ui.label(format!("Controller snapshot: {major}.{minor}")); }
                            }
                        }
                        ui.hyperlink_to("Official Blade 18 downloads", "https://mysupport.razer.com/app/answers/detail/a_id/19866");
                        hint(ui, "Vendor downloads · installed-version comparison is planned");
                    });
                    ui.collapsing("Diagnostics + shortcuts", |ui| {
                        if ui.button("Copy report").clicked() { ctx.copy_text(self.report.clone()); }
                        hint(ui, "Ctrl+Shift+Alt: F12 show · F5 mode · F6 AC · F7 battery");
                        ui.add(egui::TextEdit::multiline(&mut self.report).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).interactive(false).desired_rows(10));
                    });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if !idle { ui.spinner(); }
                ui.add(egui::Label::new(RichText::new(&self.message).small().color(MUTED)).truncate()).on_hover_text(&self.message);
            });
            for error in [&self.desktop_error, &self.power_error, &self.settings_error].into_iter().flatten() {
                ui.add(egui::Label::new(RichText::new(error).color(Color32::from_rgb(255, 160, 140))).truncate()).on_hover_text(error);
            }
            ui.horizontal(|ui| {
                hint(ui, format!("v{} · Blade 18 experimental support", env!("CARGO_PKG_VERSION")));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(idle, egui::Button::new("Exit")).clicked() { self.action(ctx, Action::Quit); }
                    if self.desktop.is_some() && ui.button("Hide").clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false)); }
                });
            });
        });
        if let Some(change) = requested {
            self.start(ctx, "Apply setting", None, move || blade18::change(change));
        }
        if read_sensors && !self.preview {
            let (sender, receiver) = mpsc::channel();
            self.sensor_job = Some(receiver);
            self.message = "Reading controller sensor candidates…".into();
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                let _ = sender.send(
                    librazer::diagnostics::collect_sensors().map_err(|error| format!("{error:#}")),
                );
                repaint.request_repaint();
            });
        }
    }
}
