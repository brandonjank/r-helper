use super::*;
use librazer::blade18::extended::Control;
impl BladeApp {
    pub(super) fn extended_controls(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        enabled: bool,
    ) {
        let mut requested = None;
        let mut run_batch = false;
        let mut inspect = false;
        let mut read_lamps = false;
        let mut preview_lamps = false;
        super::view::card(ui, "Keyboard behavior", |ui| {
            ui.add_enabled_ui(enabled,|ui| {
                if ui.button("Read keyboard controls").clicked(){requested=Some(None);}
                if ui.button("Run keyboard control checks").clicked(){run_batch=true;}
                if ui.button("Inspect remaining controller features").clicked(){inspect=true;}
                if let Some(details)=&self.controller_telemetry {
                    if let (Some(actual),Some(recommended))=(details["adapter_class_watts"].as_u64(),details["recommended_adapter_class_watts"].as_u64()){
                        ui.label(format!("Adapter class: {actual} W · recommended {recommended} W"));
                        ui.small("Firmware adapter rating; not a live wattage measurement.");
                    }
                }
                if let Some(rows)=self.extended.as_ref().and_then(|v|v.as_array()) {
                    for row in rows {
                        let Ok(control)=serde_json::from_value::<Control>(row["control"].clone()) else{continue;};
                        let Some(bytes)=row["value"].as_array() else{ui.label(format!("{}: unavailable",control.label()));continue;};
                        let value=bytes.iter().filter_map(|v|v.as_u64().and_then(|v|u8::try_from(v).ok())).collect::<Vec<_>>();
                        if control.validate(&value).is_err(){continue;}
                        ui.horizontal_wrapped(|ui| {
                            ui.label(control.label());
                            match control {
                                Control::FunctionKeys|Control::PersistentFunctionKeys=>{
                                    for (name,mode) in [("F1–F12",0),("Multimedia",1)] {if ui.selectable_label(value[1]==mode,name).clicked(){let mut next=value.clone();next[1]=mode;requested=Some(Some((control,next)));}}
                                },
                                Control::GamingKeys=>for (name,mode) in [("Off",0),("On",1)]{if ui.selectable_label(value[0]==mode,name).clicked(){requested=Some(Some((control,vec![mode,0,0,0])));}},
                                Control::ForceChroma=>{
                                    ui.label(if value[0]==1{"Flag on"}else{"Flag off"});
                                    if ui.button("Toggle for testing").clicked(){requested=Some(Some((control,vec![value[0]^1])));}
                                },
                            }
                        });
                    }
                }
                super::view::hint(ui,"Function-row and gaming switches passed controller round trips. Physical key behavior needs owner testing. Chroma flag semantics remain experimental; it is not an exclusive software lock.");
            });
        });
        super::view::card(ui, "Auxiliary RGB lighting · experimental", |ui| {
            if blade18::lamp_array::preview_active() {
                ui.label("Auxiliary preview active · automatic restoration after test window closes");
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
            } else {
                ui.small("Auxiliary preview inactive");
            }
            ui.add_enabled_ui(enabled,|ui| {
                if ui.button("Inspect Windows lighting map").clicked(){read_lamps=true;}
                if let Some(report)=&self.lamp_report {
                    for device in report["devices"].as_array().into_iter().flatten(){
                        let lamps=device["lamps"].as_array();
                        let auxiliary=lamps.map(|items|items.iter().filter(|item|item["purposes"]==4).count()).unwrap_or(0);
                        ui.label(format!("{} lamps · {auxiliary} auxiliary RGB lamps",device["lamp_count"]));
                        ui.small("Read-only metadata · foreground availability is checked by the test window");
                        ui.collapsing("View lighting layout",|ui|draw_lamp_map(ui,device));
                    }
                    if report["windows_dynamic_lighting"]==true{ui.label("Windows Dynamic Lighting is enabled and can compete with firmware lighting.");}
                }
                ui.horizontal_wrapped(|ui|{
                    ui.label("Preview color");
                    ui.color_edit_button_srgb(&mut self.lamp_color);
                    if ui.add_enabled(self.desktop.is_some(),egui::Button::new("Test auxiliary lamps · 15 seconds")).clicked(){preview_lamps=true;}
                });
                ui.horizontal_wrapped(|ui|{ui.checkbox(&mut self.lamp_highlight,"Highlight one lamp in orange");if self.lamp_highlight{ui.add(egui::DragValue::new(&mut self.lamp_index).range(1..=18).prefix("Lamp "));}});
                if ui.button("Open Windows lighting settings").clicked(){let _=std::process::Command::new("explorer.exe").arg("ms-settings:personalization-lighting").spawn();}
                super::view::hint(ui,"The logo is green-only. The 18 reported auxiliary lamps remain unmapped. This test requires a static keyboard effect, opens a temporary window, then reapplies your saved lighting. R-Helper stays hidden in the tray afterward. Physical RGB behavior is unverified.");
            });
        });
        if read_lamps {
            self.batch_job(ctx,"Inspect Windows lighting",|| {
                // A metadata-only helper never becomes the foreground lighting owner.
                let output=blade_profiles::journal_path("lamp-inventory")?;
                let mut command=std::process::Command::new(std::env::current_exe()?.with_file_name("rhelper-controls.exe"));
                command.arg("--lamp-array").arg("--output").arg(&output);
                #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
                anyhow::ensure!(command.output()?.status.success(),"Windows lighting inventory failed");
                let inventory:serde_json::Value=serde_json::from_slice(&std::fs::read(&output)?)?;
                Ok(serde_json::json!({"extended_controls":blade18::extended::status(),"controller_telemetry":blade18::extended::telemetry(),"lamp_inventory":inventory}))
            });
        }
        if preview_lamps {
            let color = self.lamp_color;
            let highlight = self.lamp_highlight.then_some(self.lamp_index);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.batch_job(ctx,"Test auxiliary RGB and restore lighting",move||crate::blade_auxiliary::run(color,highlight,&blade_profiles::journal_path("auxiliary-test")?));
        }
        if inspect {
            self.batch_job(ctx,"Inspect controller features",|| {
                Ok(serde_json::json!({"extended_controls":blade18::extended::status(),"controller_telemetry":blade18::extended::telemetry(),"extended_inventory":blade18::extended::inventory()?}))
            });
        }
        if run_batch {
            self.batch_job(ctx,"Test keyboard controls",|| {
                let output=blade_profiles::journal_path("keyboard-batch")?;
                let executable=std::env::current_exe()?.with_file_name("rhelper-controls.exe");
                anyhow::ensure!(executable.is_file(),"Keyboard test tool is missing beside R-Helper");
                let mut command=std::process::Command::new(executable);command.arg("--validate").arg("--output").arg(&output);
                #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
                let result=command.output()?;
                let report:serde_json::Value=serde_json::from_slice(&std::fs::read(output)?)?;
                anyhow::ensure!(result.status.success()&&report["passed"]==true,"Keyboard batch failed: {report}");
                Ok(serde_json::json!({"extended_controls":blade18::extended::status(),"controller_telemetry":blade18::extended::telemetry(),"keyboard_batch":report}))
            });
        }
        if let Some(change) = requested {
            self.batch_job(ctx,if change.is_some(){"Set keyboard behavior"}else{"Read keyboard behavior"},move|| {
                if let Some((control,value))=change {control.apply(&value,&blade_profiles::journal_path("keyboard-control")?)?;}
                Ok(serde_json::json!({"extended_controls":blade18::extended::status(),"controller_telemetry":blade18::extended::telemetry()}))
            });
        }
    }
}
fn draw_lamp_map(ui: &mut egui::Ui, device: &serde_json::Value) {
    let lamps = device["lamps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|lamp| {
            let x = lamp["position"][0].as_f64()? as f32;
            let y = lamp["position"][1].as_f64()? as f32;
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            Some((lamp, x, y))
        })
        .collect::<Vec<_>>();
    if lamps.is_empty() {
        return;
    }
    let min_x = lamps.iter().map(|(_, x, _)| *x).fold(f32::INFINITY, f32::min);
    let max_x = lamps.iter().map(|(_, x, _)| *x).fold(f32::NEG_INFINITY, f32::max);
    let min_y = lamps.iter().map(|(_, _, y)| *y).fold(f32::INFINITY, f32::min);
    let max_y = lamps.iter().map(|(_, _, y)| *y).fold(f32::NEG_INFINITY, f32::max);
    let (bounds, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 170.0), egui::Sense::hover());
    let inner = bounds.shrink(8.0);
    for (lamp, x, y) in lamps {
        let pos = egui::pos2(
            inner.left() + (x - min_x) / (max_x - min_x).max(0.0001) * inner.width(),
            inner.top() + (y - min_y) / (max_y - min_y).max(0.0001) * inner.height(),
        );
        let auxiliary = lamp["purposes"] == 4;
        let color = if auxiliary {
            egui::Color32::from_rgb(255, 150, 70)
        } else {
            egui::Color32::from_rgb(70, 135, 245)
        };
        let rect =
            egui::Rect::from_center_size(pos, egui::vec2(if auxiliary { 6.0 } else { 10.0 }, 10.0));
        ui.painter().rect_filled(rect, 2.0, color);
        let index = lamp["index"].as_u64().unwrap_or(0);
        ui.interact(rect, ui.id().with(("lamp", index)), egui::Sense::hover()).on_hover_text(
            format!("Lamp {index} · {}", if auxiliary { "auxiliary RGB" } else { "keyboard" }),
        );
    }
    ui.small("Reported positions · blue: keyboard · orange: auxiliary lamps. This map does not read current colors.");
}
