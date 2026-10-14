use super::*;
use blade18::lighting::{Preset, Style};
pub(super) struct BatchUi {
    pub draft: Preset,
    name: String,
    selected: usize,
    transfer: String,
    pub charge: Option<bool>,
    pub pending: Option<(PathBuf, std::time::Instant)>,
    pub watcher: Option<crate::blade_policy::Watcher>,
    pub updates: Option<mpsc::Receiver<crate::blade_policy::Update>>,
}
impl Default for BatchUi {
    fn default() -> Self {
        Self {
            draft: Preset {
                style: Style::Static,
                color: [32, 64, 255],
                second: [255, 64, 32],
                keys: vec![[32, 64, 255]; 114],
                brightness: 176,
                underside: false,
                underside_brightness: 64,
                logo: LogoMode::Static,
            },
            name: "My profile".into(),
            selected: 0,
            transfer: String::new(),
            charge: None,
            pending: None,
            watcher: None,
            updates: None,
        }
    }
}
impl BladeApp {
    fn batch_job(
        &mut self,
        ctx: &egui::Context,
        label: &str,
        work: impl FnOnce() -> anyhow::Result<serde_json::Value> + Send + 'static,
    ) {
        if self.preview || self.job.is_some() || self.sensor_job.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.sensor_job = Some(receiver);
        self.message = format!("{label}…");
        let repaint = ctx.clone();
        let curves = self.curve_control.clone();
        let policy = self.batch.watcher.as_ref().map(|w| w.sender());
        let manual = label.starts_with("Apply") || label.starts_with("Keep");
        std::thread::spawn(move || {
            let result: anyhow::Result<serde_json::Value> = (|| {
                crate::blade_curves::stop_and_wait(&curves)?;
                let _lock = blade_profiles::operation_lock()?;
                crate::blade_curves::stop_and_wait(&curves)?;
                let result = work()?;
                if manual {
                    if let (Some(sender), Ok(snapshot)) =
                        (policy, blade_profiles::Named::capture("Manual settings".into()))
                    {
                        let _ = sender.send(crate::blade_power::Event::Baseline(snapshot));
                    }
                }
                Ok(result)
            })();
            let _ = sender.send(result.map_err(|e| format!("{e:#}")));
            repaint.request_repaint();
        });
    }
    pub(super) fn batch_updates(&mut self, ctx: &egui::Context) {
        if let Some(updates) = &self.batch.updates {
            while let Ok(message) = updates.try_recv() {
                self.message = message.message;
                if let Some(state) = message.state {
                    self.state = Some(state);
                }
                self.lighting = None;
                self.display = None;
            }
        }
        if let Some((path, start)) = &self.batch.pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            let remaining = 15u64.saturating_sub(start.elapsed().as_secs());
            let mut keep = false;
            let mut revert = false;
            egui::Window::new("Keep this refresh rate?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Restoring the previous rate in {remaining} seconds."));
                    ui.horizontal(|ui| {
                        keep = ui.add_enabled(remaining > 0, egui::Button::new("Keep")).clicked();
                        revert = ui.button("Revert").clicked();
                    });
                });
            if keep || revert || remaining == 0 {
                let path = path.clone();
                self.batch.pending = None;
                self.batch_job(
                    ctx,
                    if keep { "Keep refresh rate" } else { "Restore refresh rate" },
                    move || {
                        crate::blade_display::finish(&path, keep)
                            .map(|s| serde_json::json!({"display_controls":s}))
                    },
                );
            }
        }
    }
    pub(super) fn batch_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, enabled: bool) {
        let enabled = enabled && self.batch.pending.is_none();
        let mut apply = None;
        let mut capture = None;
        let mut replace = None;
        let mut charge = None;
        let mut read_charge = false;
        let mut save = false;
        super::view::card(ui, "Keyboard effects + lighting profiles", |ui| {
            ui.add_enabled_ui(enabled,|ui|{
                egui::ComboBox::from_id_salt("full_keyboard_effect").selected_text(self.batch.draft.style.label()).show_ui(ui,|ui|{
                    for (style,name) in [(Style::Static,"Static"),(Style::Breathing,"Breathing"),(Style::DualBreathing,"Two-color breathing"),(Style::WaveLeft,"Wave left"),(Style::WaveRight,"Wave right"),(Style::Reactive,"Reactive"),(Style::PerKey,"Per-key colors")]{ui.selectable_value(&mut self.batch.draft.style,style,name);}
                });
                ui.horizontal(|ui|{ui.label("Color");ui.color_edit_button_srgb(&mut self.batch.draft.color);if matches!(self.batch.draft.style,Style::DualBreathing|Style::PerKey){ui.label("Second");ui.color_edit_button_srgb(&mut self.batch.draft.second);}});
                if self.batch.draft.style==Style::PerKey {
                    ui.label("Pick a color above, then paint cells. Apply sends one static map.");
                    ui.horizontal(|ui|{if ui.button("Fill all").clicked(){self.batch.draft.keys.fill(self.batch.draft.color);}if ui.button("Alternate rows").clicked(){for (i,c) in self.batch.draft.keys.iter_mut().enumerate(){*c=if i/19%2==0{self.batch.draft.color}else{self.batch.draft.second};}}});
                    egui::Grid::new("key_matrix").spacing([2.0,2.0]).show(ui,|ui|{for row in 0..6 {for col in 0..19 {let color=self.batch.draft.keys[row*19+col];if ui.add_sized([16.0,16.0],egui::Button::new("").min_size(egui::vec2(0.0,0.0)).fill(egui::Color32::from_rgb(color[0],color[1],color[2]))).on_hover_text(format!("Row {} · column {}",row+1,col+1)).clicked(){self.batch.draft.keys[row*19+col]=self.batch.draft.color;}}ui.end_row();}});
                    ui.small("Experimental 6 × 19 matrix · some cells have no physical key. Close other lighting controllers before editing.");
                }
                ui.add(egui::Slider::new(&mut self.batch.draft.brightness,0..=255).text("Keyboard brightness"));
                ui.checkbox(&mut self.batch.draft.underside,"Underside enabled");
                ui.add(egui::Slider::new(&mut self.batch.draft.underside_brightness,0..=255).text("Underside brightness"));
                ui.horizontal(|ui|{ui.label("Logo");for mode in [LogoMode::Off,LogoMode::Static,LogoMode::Breathing]{ui.selectable_value(&mut self.batch.draft.logo,mode,format!("{mode:?}"));}});
                if ui.button("Apply lighting preset").clicked(){apply=Some(self.batch.draft.clone());}
                ui.horizontal_wrapped(|ui|{if ui.button("Save lighting for AC").clicked(){self.settings.lighting_ac=Some(self.batch.draft.clone());save=true;}if ui.button("Save lighting for battery").clicked(){self.settings.lighting_battery=Some(self.batch.draft.clone());save=true;}});
                ui.small("Preset replaces the keyboard effect. Effects run in firmware; static per-key colors send no recurring frames.");
            });
        });
        ui.add_space(8.0);
        super::view::card(ui, "Lighting power policies", |ui| {
            ui.add_enabled_ui(enabled,|ui|{
                save|=ui.checkbox(&mut self.settings.lighting_policy.enabled,"Enable lighting power policies").changed();
                ui.horizontal(|ui|{ui.label("Idle timeout on AC");save|=ui.add(egui::DragValue::new(&mut self.settings.lighting_policy.idle_ac_seconds).range(0..=86400).suffix(" s")).changed();});
                ui.horizontal(|ui|{ui.label("Idle timeout on battery");save|=ui.add(egui::DragValue::new(&mut self.settings.lighting_policy.idle_battery_seconds).range(0..=86400).suffix(" s")).changed();});
                save|=ui.checkbox(&mut self.settings.lighting_policy.display_off,"Turn lights off with the display").changed();
                save|=ui.checkbox(&mut self.settings.lighting_policy.restore_on_resume,"Restore lighting and enabled power profiles after resume").changed();
                save|=ui.add(egui::Slider::new(&mut self.settings.lighting_policy.low_battery_percent,0..=100).text("Battery cutoff %")).changed();
                ui.small("0 disables a timeout/cutoff. Keyboard, logo and underside restore after activity. Policies pause during custom fan sessions.");
            });
        });
        ui.add_space(8.0);
        super::view::card(ui, "Named profiles", |ui| {
            ui.add_enabled_ui(enabled && self.settings_error.is_none(),|ui|{
                ui.horizontal(|ui|{ui.text_edit_singleline(&mut self.batch.name);if ui.add_enabled(self.settings.named.len()<32,egui::Button::new("Capture current")).clicked(){capture=Some(self.batch.name.trim().to_owned());}});
                if !self.settings.named.is_empty(){
                    self.batch.selected=self.batch.selected.min(self.settings.named.len()-1);
                    egui::ComboBox::from_id_salt("named_profiles").selected_text(&self.settings.named[self.batch.selected].name).show_ui(ui,|ui|{for (i,p) in self.settings.named.iter().enumerate(){ui.selectable_value(&mut self.batch.selected,i,&p.name);}});
                    let p=&mut self.settings.named[self.batch.selected];
                    ui.horizontal(|ui|{ui.label("Name");ui.text_edit_singleline(&mut p.name);});
                    ui.horizontal(|ui|{ui.label("Application");ui.text_edit_singleline(&mut p.executable).on_hover_text("Executable name only, e.g. game.exe. Empty means manual only.");});
                    let chosen=p.clone();
                    ui.horizontal_wrapped(|ui|{
                        if ui.button("Save profile edits").clicked(){save=true;}
                        if ui.button("Apply named profile").clicked(){self.batch.draft=chosen.lighting.clone();self.start(ctx,"Apply named profile",None,move||chosen.apply());}
                        if ui.button("Update from current").clicked(){replace=Some((self.batch.selected,self.settings.named[self.batch.selected].name.clone(),self.settings.named[self.batch.selected].executable.clone()));}
                        if ui.add_enabled(self.settings.named.len()<32,egui::Button::new("Duplicate")).clicked(){let mut p=self.settings.named[self.batch.selected].clone();p.name=format!("{} copy",p.name);p.executable.clear();self.settings.named.push(p);save=true;}
                        if ui.button("Remove").clicked(){self.settings.named.remove(self.batch.selected);save=true;}
                    });
                }
                save|=ui.checkbox(&mut self.settings.application_switching,"Switch profiles for the foreground application").changed();
                ui.small("Application profile takes priority. Battery-incompatible modes are skipped. On leaving the app, the power profile or previous settings return.");
                ui.collapsing("Import / export",|ui|{
                    if ui.button("Copy profiles JSON").clicked(){self.batch.transfer=serde_json::to_string_pretty(&self.settings).unwrap_or_default();ctx.copy_text(self.batch.transfer.clone());}
                    ui.add(egui::TextEdit::multiline(&mut self.batch.transfer).desired_rows(4).hint_text("Paste an exported R-Helper profile file here"));
                    if ui.button("Import profiles JSON").clicked(){
                        let result=blade_profiles::import(&self.batch.transfer);
                        match result{Ok(settings)=>{self.settings=settings;save=true;},Err(e)=>self.message=format!("Import rejected; current profiles preserved: {e:#}")}
                    }
                    ui.small("Import leaves automatic switching and lighting policies off until you enable them.");
                });
            });
        });
        ui.add_space(8.0);
        super::view::card(ui, "Charge fully once", |ui| {
            ui.label(match self.batch.charge {
                Some(true) => "One-time full charge requested",
                Some(false) => "One-time override inactive",
                None => "Read charging status to check the override",
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(enabled, egui::Button::new("Read charging status")).clicked() {
                    read_charge = true;
                }
                if ui
                    .add_enabled(
                        enabled && self.last_power == Some(true),
                        egui::Button::new("Charge fully once"),
                    )
                    .clicked()
                {
                    charge = Some(true);
                }
                if ui.add_enabled(enabled, egui::Button::new("Cancel full charge")).clicked() {
                    charge = Some(false);
                }
            });
            ui.small("Preserves your normal charge limit. Completion and clearing are managed by firmware; readback verifies the request, not a completed charging cycle.");
        });
        if save {
            self.persist();
        }
        if let Some(preset) = apply {
            self.batch_job(ctx, "Apply lighting preset", move || {
                let s = preset.apply(&blade_profiles::journal_path("lighting-preset")?)?;
                Ok(serde_json::json!({"lighting_controls":s,"device_state":blade18::status()?}))
            });
        }
        if let Some(name) = capture {
            self.batch_job(ctx, "Capture named profile", move || {
                let p = blade_profiles::Named::capture(name)?;
                p.validate()?;
                Ok(serde_json::json!({"named_capture":p}))
            });
        }
        if let Some((index, name, executable)) = replace {
            self.batch_job(ctx, "Update named profile", move || {
                let mut p = blade_profiles::Named::capture(name)?;
                p.executable = executable;
                p.validate()?;
                Ok(serde_json::json!({"named_update":{"index":index,"profile":p}}))
            });
        }
        if let Some(value) = charge {
            self.batch_job(ctx, "Set one-time charging", move || {
                blade18::charge::set(value, &blade_profiles::journal_path("charge-once")?)
                    .map(|v| serde_json::json!({"charge_once":v}))
            });
        } else if read_charge {
            self.batch_job(ctx, "Read charging status", || {
                blade18::charge::active().map(|v| serde_json::json!({"charge_once":v}))
            });
        }
    }
}
