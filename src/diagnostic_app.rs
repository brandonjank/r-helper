use eframe::egui;

struct DiagnosticsApp {
    report: String,
    pending: Option<std::sync::mpsc::Receiver<String>>,
}

impl DiagnosticsApp {
    fn inventory() -> String {
        match librazer::diagnostics::collect(None) {
            Ok(report) => format!("{report:#}"),
            Err(error) => format!("Inventory failed: {error:#}"),
        }
    }
}

impl eframe::App for DiagnosticsApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
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
            ui.label("Hardware recognized. Control support is being validated.");
            ui.label("Inventory is passive. Read device status sends getter requests only.");
            ui.horizontal(|ui| {
                if ui.button("Refresh inventory").clicked() {
                    self.report = Self::inventory();
                }
                if ui.button("Copy report").clicked() {
                    ctx.copy_text(self.report.clone());
                }
                if ui
                    .add_enabled(self.pending.is_none(), egui::Button::new("Read device status"))
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
            Ok(Box::new(DiagnosticsApp { report: DiagnosticsApp::inventory(), pending: None }))
        }),
    )
}
