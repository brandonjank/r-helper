use super::batch::BaselinePolicy;
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
        let mut platform = false;
        let mut gpu_preview = false;
        let mut display_read = false;
        let mut updates = None;
        let mut display_preview = None;
        let mut read_lamps = false;
        let mut preview_lamps = false;
        super::view::section_card(
            ui,
            self.section,
            super::view::Section::Lighting,
            "Keyboard behavior",
            |ui| {
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
            },
        );
        super::view::section_card(
            ui,
            self.section,
            super::view::Section::Lighting,
            "Auxiliary RGB lighting · experimental",
            |ui| {
                if blade18::lamp_array::preview_active() {
                    ui.label(
                        "Auxiliary preview active · automatic restoration after test window closes",
                    );
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
                super::view::hint(ui,"The logo is green-only. Use the native underside color controls above for verified RGB. The Windows auxiliary lamps remain unmapped. This test requires a static keyboard effect, temporarily powers the underside at a brighter setting, opens a test window, then restores your saved lighting and underside power. R-Helper stays hidden in the tray afterward. Physical RGB behavior is unverified.");
            });
            },
        );
        if read_lamps {
            self.batch_job(ctx,"Inspect Windows lighting",BaselinePolicy::Preserve, || {
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
            self.batch_job(
                ctx,
                "Test auxiliary RGB and restore lighting",
                BaselinePolicy::Preserve,
                move || {
                    crate::blade_auxiliary::run(
                        color,
                        highlight,
                        &blade_profiles::journal_path("auxiliary-test")?,
                    )
                },
            );
        }
        super::view::section_card(
            ui,
            self.section,
            super::view::Section::Display,
            "Display mode preview",
            |ui| {
                if ui
                    .add_enabled(enabled, egui::Button::new("Read supported display modes"))
                    .clicked()
                {
                    display_read = true;
                }
                if let Some(info) = &self.display {
                    let current = self.batch.display_mode.as_ref().unwrap_or(&info.current);
                    egui::ComboBox::from_id_salt("manual_display_mode")
                        .selected_text(format!(
                            "{} × {} · {} Hz",
                            current.width, current.height, current.hz
                        ))
                        .show_ui(ui, |ui| {
                            for mode in &info.modes {
                                ui.selectable_value(
                                    &mut self.batch.display_mode,
                                    Some(mode.clone()),
                                    format!("{} × {} · {} Hz", mode.width, mode.height, mode.hz),
                                );
                            }
                        });
                    if ui
                        .add_enabled(
                            enabled && !crate::blade_display::is_pending(),
                            egui::Button::new("Preview display mode"),
                        )
                        .clicked()
                    {
                        display_preview =
                            Some(self.batch.display_mode.clone().unwrap_or(info.current.clone()));
                    }
                }
                ui.small("Windows resolution + refresh rate, with Keep/Revert. This does not switch the panel's firmware mode. After keeping a new resolution, save refresh profiles again.");
            },
        );
        if display_read {
            self.batch_job(ctx, "Read display modes", BaselinePolicy::Preserve, || {
                Ok(serde_json::json!({"display_controls":librazer::display::inventory()?}))
            });
        }
        if let Some(target) = display_preview {
            self.batch_job(ctx, "Preview display mode", BaselinePolicy::Preserve, move || {
                crate::blade_display::begin_mode(&target)
            });
        }
        super::view::section_card(
            ui,
            self.section,
            super::view::Section::Updates,
            "BIOS, firmware + drivers",
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(enabled, egui::Button::new("Read installed versions"))
                        .clicked()
                    {
                        updates = Some(false);
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Check official catalog"))
                        .clicked()
                    {
                        updates = Some(true);
                    }
                });
                ui.hyperlink_to("Razer Blade 18 (2025) downloads", crate::blade_updates::SOURCE);
                if let Some(report) = &self.updates_report {
                    ui.small(
                        report["catalog"]["verified"]
                            .as_str()
                            .unwrap_or("Catalog source unavailable"),
                    );
                    if let Some(checked) = report["catalog"]["checked_unix"].as_u64() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        ui.small(format!(
                            "Last successful catalog check: {} minutes ago",
                            now.saturating_sub(checked) / 60
                        ));
                    }
                    if let Some(error) = report["check_error"].as_str() {
                        ui.label("Online check unavailable · previous catalog retained");
                        ui.collapsing("Check details", |ui| {
                            ui.small(error);
                        });
                    }
                    if report["firmware_error"].is_string() {
                        ui.small("Some installed firmware versions are unavailable; the installed Razer helper may be missing.");
                    }
                    for row in report["rows"].as_array().into_iter().flatten() {
                        ui.separator();
                        ui.label(format!(
                            "{} · installed {} → catalog {}",
                            row["component"].as_str().unwrap_or("Firmware"),
                            row["installed"].as_str().unwrap_or("unknown"),
                            row["available"].as_str().unwrap_or("unknown")
                        ));
                        ui.small(format!(
                            "{} · released {}",
                            row["status"].as_str().unwrap_or("Check official page"),
                            row["released"].as_str().unwrap_or("unknown")
                        ));
                        ui.small(row["notes"].as_str().unwrap_or(""));
                    }
                    ui.collapsing("Installed driver versions",|ui|{
                    for row in report["drivers"]["installed"].as_array().into_iter().flatten(){ui.label(format!("{} · {}",row["name"].as_str().unwrap_or("Driver"),row["version"].as_str().unwrap_or("unknown")));}
                    for source in report["driver_sources"].as_array().into_iter().flatten(){if let (Some(name),Some(url))=(source["name"].as_str(),source["url"].as_str()){ui.hyperlink_to(name,url);}}
                    ui.small("Choose the laptop GPU and Windows version on the official site; no automatic driver-version comparison yet.");
                });
                } else {
                    ui.small("Read installed versions to compare with the dated reference catalog, or check the official catalog on demand.");
                }
                ui.small("Model RZ09-0529 · manual updates only. Review the official instructions and exact SKU before installing. No background update checks.");
            },
        );
        if let Some(refresh) = updates {
            self.batch_job(ctx, "Check firmware and driver versions", BaselinePolicy::Preserve, move || {
                Ok(serde_json::json!({"updates_inventory":crate::blade_updates::inspect(refresh)?}))
            });
        }
        super::view::section_card(
            ui,
            self.section,
            super::view::Section::System,
            "Platform capabilities",
            |ui| {
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new("Read firmware, panel + tuning availability"),
                    )
                    .clicked()
                {
                    platform = true;
                }
                ui.small("On demand only. GPU diagnostics can wake the NVIDIA GPU. Installed Razer helpers are optional and isolated.");
                if let Some(report) = &self.platform_report {
                    let native = &report["native"];
                    if let Some(panel) =
                        native["GetBladeLEDPanelProducerShortName"]["value"].as_str()
                    {
                        ui.label(format!("Panel maker: {panel}"));
                    }
                    if let Some(firmware) = native["GetBladeFirmwareInfo"]["value"].as_object() {
                        for (name, value) in firmware {
                            ui.label(format!("{name}: {}", value.as_str().unwrap_or("unknown")));
                        }
                    }
                    if let Some(gpus) = report["gpu"]["gpus"].as_array() {
                        for gpu in gpus {
                            ui.label(format!(
                                "{} · driver {}",
                                gpu["name"].as_str().unwrap_or("NVIDIA"),
                                gpu["driver_version"].as_str().unwrap_or("unknown")
                            ));
                            ui.label(format!(
                                "GPU power limit: {} W · range {}–{} W",
                                gpu["power.limit"].as_str().unwrap_or("unavailable"),
                                gpu["power.min_limit"].as_str().unwrap_or("unavailable"),
                                gpu["power.max_limit"].as_str().unwrap_or("unavailable")
                            ));
                        }
                    }
                    if let Some(zones) = report["thermal_zones"]["zones"].as_array() {
                        ui.collapsing("Additional temperatures · unmapped",|ui|{for(index,zone)in zones.iter().enumerate(){if let Some(value)=zone["celsius"].as_f64(){ui.label(format!("Additional temperature {}: {value:.1} °C",index+1)).on_hover_text(zone["zone"].as_str().unwrap_or("Firmware zone"));}}ui.small("Not identified as CPU, GPU or battery sensors; not used for fan control.");});
                    }
                    if let Some(drivers) = report["windows_drivers"]["installed"].as_array() {
                        ui.collapsing("Installed drivers", |ui| {
                            for driver in drivers {
                                ui.label(format!(
                                    "{} · {}",
                                    driver["name"].as_str().unwrap_or("Driver"),
                                    driver["version"].as_str().unwrap_or("unknown")
                                ));
                            }
                            ui.small("Installed versions; check official downloads for updates.");
                        });
                    }
                    let tuning = &report["intel_history"]["tuning"];
                    if let Some(when) = tuning["recorded_at"].as_str() {
                        ui.collapsing(format!("Last Synapse tuning report · {when}"), |ui| {
                            ui.small("Historical report; values may have changed.");
                            for item in tuning["controls"].as_array().into_iter().flatten() {
                                ui.label(format!(
                                    "{}: {}",
                                    item["name"].as_str().unwrap_or("Control"),
                                    item["value"]
                                ));
                            }
                        });
                    }
                }
                ui.collapsing("NVIDIA clock preview · experimental",|ui|{
                ui.horizontal(|ui|{ui.label("Core offset");ui.add(egui::DragValue::new(&mut self.batch.gpu_offsets.gpu).range(-100..=100).suffix(" MHz"));});
                ui.horizontal(|ui|{ui.label("Memory offset");ui.add(egui::DragValue::new(&mut self.batch.gpu_offsets.vram).range(-200..=200).suffix(" MHz"));});
                if ui.add_enabled(enabled && crate::blade_gpu::administrator(),egui::Button::new("Preview offsets · restore after 15 seconds")).clicked(){gpu_preview=true;}
                ui.small("Administrator + AC only. Original offsets restore automatically. These preview limits are not a stability guarantee. Requires the installed Razer helper; voltage is unchanged.");
            });
                ui.collapsing("Available and remaining features", |ui| {
                    for item in
                        crate::blade_platform::capabilities().as_array().into_iter().flatten()
                    {
                        ui.label(format!(
                            "{} · {}",
                            item["name"].as_str().unwrap_or("Feature"),
                            item["status"].as_str().unwrap_or("Research")
                        ));
                        ui.small(item["detail"].as_str().unwrap_or(""));
                        ui.add_space(4.0);
                    }
                });
            },
        );
        if gpu_preview {
            let offsets = self.batch.gpu_offsets;
            self.batch_job(
                ctx,
                "Preview NVIDIA clock offsets",
                BaselinePolicy::Preserve,
                move || {
                    crate::blade_gpu::preview(
                        offsets,
                        &blade_profiles::journal_path("gpu-preview")?,
                    )
                },
            );
        }
        if platform {
            self.batch_job(ctx, "Read platform capabilities", BaselinePolicy::Preserve, || {
                Ok(serde_json::json!({"platform_inventory":crate::blade_platform::inspect()?}))
            });
        }
        if inspect {
            self.batch_job(ctx,"Inspect controller features",BaselinePolicy::Preserve, || {
                Ok(serde_json::json!({"extended_controls":blade18::extended::status(),"controller_telemetry":blade18::extended::telemetry(),"extended_inventory":blade18::extended::inventory()?}))
            });
        }
        if run_batch {
            self.batch_job(ctx,"Test keyboard controls",BaselinePolicy::Preserve, || {
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
            self.batch_job(ctx,if change.is_some(){"Set keyboard behavior"}else{"Read keyboard behavior"},BaselinePolicy::Preserve,move|| {
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
