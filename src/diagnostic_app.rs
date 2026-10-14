use eframe::egui;

struct DiagnosticsApp {
    report: String,
    pending: Option<std::sync::mpsc::Receiver<String>>,
    brightness: Option<u8>,
    brightness_pending: Option<std::sync::mpsc::Receiver<Result<u8, String>>>,
    brightness_message: String,
}

impl DiagnosticsApp {
    fn start_brightness(&mut self, value: Option<u8>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.brightness_pending = Some(receiver);
        self.brightness_message = "Reading keyboard brightness…".into();
        std::thread::spawn(move || {
            let result = match value {
                Some(value) => librazer::brightness::set_keyboard(value),
                None => librazer::brightness::read_keyboard(),
            }
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
    }

    fn inventory() -> String {
        match librazer::diagnostics::collect(None) {
            Ok(report) => format!("{report:#}"),
            Err(error) => format!("Inventory failed: {error:#}"),
        }
    }
}

impl eframe::App for DiagnosticsApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(receiver) = &self.brightness_pending {
            match receiver.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(value) => {
                            self.brightness = Some(value);
                            self.brightness_message = format!("Device readback: {value}/255");
                        }
                        Err(error) => {
                            self.brightness = None;
                            self.brightness_message = error;
                        }
                    }
                    self.brightness_pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.brightness_message = "Brightness worker stopped unexpectedly".into();
                    self.brightness_pending = None;
                    self.brightness = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(report) => {
                    self.report = report;
                    self.pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.report = "Status worker stopped unexpectedly".into();
                    self.pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("R-Helper · Blade 18 (2025)");
            ui.label("Keyboard brightness is validated. Other controls are being validated.");
            ui.label("Inventory is passive. Read device status sends getter requests only.");
            ui.horizontal(|ui| {
                if ui.button("Refresh inventory").clicked() {
                    self.report = Self::inventory();
                }
                if ui.button("Copy report").clicked() {
                    ctx.copy_text(self.report.clone());
                }
                if ui
                    .add_enabled(self.pending.is_none() && self.brightness_pending.is_none(), egui::Button::new("Read device status"))
                    .clicked()
                {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    self.pending = Some(receiver);
                    std::thread::spawn(move || {
                        let report = match librazer::diagnostics::collect_status() {
                            Ok(report) => format!("{report:#}"),
                            Err(error) => format!("Status read failed: {error:#}"),
                        };
                        let _ = sender.send(report);
                    });
                }
                if self.pending.is_some() {
                    ui.spinner();
                }
            });
            ui.separator();
            ui.label("Keyboard brightness");
            let idle = self.pending.is_none() && self.brightness_pending.is_none();
            ui.horizontal(|ui| {
                if ui.add_enabled(idle, egui::Button::new("Read brightness")).clicked() {
                    self.start_brightness(None);
                }
                if let Some(value) = self.brightness.as_mut() {
                    ui.add_enabled(idle, egui::Slider::new(value, 0..=255).suffix(" / 255"));
                }
                if ui.add_enabled(idle && self.brightness.is_some(), egui::Button::new("Apply brightness")).clicked() {
                    self.start_brightness(self.brightness);
                }
                if self.brightness_pending.is_some() { ui.spinner(); }
            });
            ui.label("Read the current value, adjust the slider, then apply. Each change is checked by reading the device.");
            ui.label(&self.brightness_message);
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.report)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .interactive(false)
                        .desired_rows(25),
                );
            });
            ui.label(
                "Use rhelper-diagnostics.exe --help for optional getter probes and JSON export.",
            );
        });
    }
}

pub fn run() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("R-Helper · Blade 18 diagnostics")
            .with_inner_size([760.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "R-Helper diagnostics",
        options,
        Box::new(|_| {
            Ok(Box::new(DiagnosticsApp {
                report: DiagnosticsApp::inventory(),
                pending: None,
                brightness: None,
                brightness_pending: None,
                brightness_message: String::new(),
            }))
        }),
    )
}
