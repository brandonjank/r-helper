//! GUI ownership state; service/process recovery remains in the parent backend.
use super::*;

#[derive(Default)]
pub struct Ui {
    session: Option<Session>,
    job: Option<std::sync::mpsc::Receiver<Result<Option<Session>, String>>>,
    pub message: String,
    error_details: Option<String>,
    report: Option<serde_json::Value>,
    next_health: Option<Instant>,
}
impl Ui {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn start(&mut self, ctx: &eframe::egui::Context, curves: crate::blade_curves::Shared) {
        if self.busy() || self.session.is_some() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        self.job = Some(receiver);
        self.message = "Pausing Razer control software…".into();
        self.error_details = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                crate::blade_curves::stop_and_wait(&curves)?;
                let _lock = crate::blade_profiles::operation_lock()?;
                crate::blade_curves::stop_and_wait(&curves)?;
                Session::start().map(Some)
            })();
            let _ = sender.send(result.map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
    }
    pub fn stop(&mut self, ctx: &eframe::egui::Context) {
        if self.busy() {
            return;
        }
        let Some(mut session) = self.session.take() else {
            return;
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        self.job = Some(receiver);
        self.message = "Restoring Razer software…".into();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::blade_profiles::operation_lock()
                .and_then(|_lock| session.stop())
                .map(|_| None)
                .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    pub fn poll(&mut self, ctx: &eframe::egui::Context) {
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    self.job = None;
                    match result {
                        Ok(session) => {
                            self.session = session;
                            self.message = if self.session.is_some() {
                                "R-Helper owns laptop control · Razer software paused"
                            } else {
                                "Razer software restored"
                            }
                            .into();
                        }
                        Err(e) => {
                            self.message =
                                if e.contains("administrator") || e.contains("Access is denied") {
                                    "Administrator access is needed to pause Razer services"
                                } else {
                                    "Razer software takeover needs attention · see details"
                                }
                                .into();
                            self.error_details = Some(e);
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    self.message="Razer ownership worker failed; recovery guard remains responsible for restoration".into();
                }
                Err(_) => {}
            }
        }
        if let Some(session) = &mut self.session {
            if self.next_health.is_none_or(|t| Instant::now() >= t) {
                if let Err(e) = session.health() {
                    self.message = format!("{e:#}");
                }
                self.next_health = Some(Instant::now() + Duration::from_secs(30));
            }
            ctx.request_repaint_after(Duration::from_secs(30));
        }
    }
    pub fn render(
        &mut self,
        ui: &mut eframe::egui::Ui,
        ctx: &eframe::egui::Context,
        enabled: bool,
        desired: &mut bool,
        curves: crate::blade_curves::Shared,
    ) -> bool {
        let mut changed = false;
        ui.label("Pause Synapse/Chroma control software while R-Helper runs. Restore it on exit; service startup settings stay unchanged.");
        if ui
            .add_enabled(
                enabled && !self.busy(),
                eframe::egui::Checkbox::new(desired, "Let R-Helper manage Razer software"),
            )
            .changed()
        {
            changed = true;
            if *desired {
                self.start(ctx, curves.clone());
            } else {
                self.stop(ctx);
            }
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    enabled && !self.busy() && self.session.is_none(),
                    eframe::egui::Button::new("Take control now"),
                )
                .clicked()
            {
                *desired = true;
                changed = true;
                self.start(ctx, curves);
            }
            if ui
                .add_enabled(
                    enabled && !self.busy() && self.session.is_some(),
                    eframe::egui::Button::new("Restore Razer software"),
                )
                .clicked()
            {
                *desired = false;
                changed = true;
                self.stop(ctx);
            }
            if ui
                .add_enabled(enabled && !self.busy(), eframe::egui::Button::new("Check conflicts"))
                .clicked()
            {
                self.report = Some(inventory());
            }
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        if let Some(error) = &self.error_details {
            ui.collapsing("Error details", |ui| {
                ui.label(error);
            });
        }
        ui.label("Pausing services requires administrator access. Drivers and the Razer elevation service are excluded. Shared Razer peripheral features may pause too.");
        if let Some(report) = &self.report {
            ui.collapsing("Conflict details", |ui| {
                ui.label(serde_json::to_string_pretty(report).unwrap_or_default());
            });
        }
        changed
    }
    pub fn shutdown(&mut self) {
        if let Some(mut session) = self.session.take() {
            let _ = session.stop();
        }
    }
}
