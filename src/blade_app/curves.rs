use super::*;
use egui::{Color32, RichText, Stroke, Vec2};
impl BladeApp {
    pub(super) fn curve_editor(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, idle: bool) {
        let active = self.curve_events.is_some();
        egui::Frame::new().fill(Color32::from_rgb(29,33,39)).corner_radius(10).inner_margin(14).show(ui,|ui|{
            ui.set_width(ui.available_width());
            ui.horizontal(|ui|{
                ui.label(RichText::new("Independent fan curves").strong());
                ui.label(RichText::new(if active{"RUNNING"}else{"EXPERIMENTAL"}).small().color(Color32::from_rgb(105,220,145)));
            });
            if let Some(status)=&self.curve_status {
                if !status["recovery_verified"].is_null(){
                    ui.label(if status["recovery_verified"]==true{"Last session: automatic cooling verified"}else{"Last session: recovery needs attention"});
                    if let Some(reason)=status["worker_error"].as_str().or(status["status"]["error"].as_str()).or(status["status"]["reason"].as_str()).or(status["reason"].as_str()){ui.label(reason);}
                }
                if let Ok(sample)=serde_json::from_value::<librazer::thermal::Sample>(status["sample"].clone()){
                    ui.label(format!("Fan 1 {} · Fan 2 {} · Fan 3 {} RPM",sample.actual_rpm[0],sample.actual_rpm[1],sample.actual_rpm[2]));
                    ui.label(format!("EC A {}°C · EC B {}°C · targets {:?}",sample.ec_celsius[0],sample.ec_celsius[1],sample.target_rpm));
                }
            }
            ui.label(RichText::new("AC + Balanced · physical fan roles unresolved · EC readings are proxies").small().color(Color32::from_gray(170)));
            ui.collapsing("Edit curves",|ui|{
                ui.add_enabled_ui(!active&&idle,|ui|{
                    for (index,zone) in self.curve_settings.zones.iter_mut().enumerate(){
                        ui.push_id(index,|ui|{
                            ui.separator();ui.label(RichText::new(format!("Fan {}",index+1)).strong());
                            egui::ComboBox::from_id_salt("source").selected_text(zone.source.label()).show_ui(ui,|ui|{
                                for source in [blade18::CurveSource::EcA,blade18::CurveSource::EcB,blade18::CurveSource::Hottest]{ui.selectable_value(&mut zone.source,source,source.label());}
                            });
                            let (minimum,maximum)=if index==2{(3000,3500)}else{(4000,4500)};
                            let (rect,_)=ui.allocate_exact_size(Vec2::new(ui.available_width().min(420.0),75.0),egui::Sense::hover());
                            let painter=ui.painter();painter.rect_filled(rect,4.0,Color32::from_rgb(19,22,27));
                            let to_pos=|p:&blade18::CurvePoint|egui::pos2(rect.left()+((p.celsius-25.0)/65.0).clamp(0.0,1.0)*rect.width(),rect.bottom()-((p.rpm.saturating_sub(minimum))as f32/(maximum-minimum)as f32).clamp(0.0,1.0)*rect.height());
                            for pair in zone.points.windows(2){painter.line_segment([to_pos(&pair[0]),to_pos(&pair[1])],Stroke::new(2.0_f32,Color32::from_rgb(105,220,145)));}
                            for point in &zone.points {painter.circle_filled(to_pos(point),3.0,Color32::from_rgb(105,220,145));}
                            ui.label(RichText::new(format!("25–90°C → · {minimum}–{maximum} RPM")).small().color(Color32::from_gray(170)));
                            for point in &mut zone.points {ui.horizontal(|ui|{
                                ui.add(egui::DragValue::new(&mut point.celsius).range(26.0..=84.0).speed(1.0).suffix(" °C"));
                                ui.add(egui::DragValue::new(&mut point.rpm).range(minimum..=maximum).speed(100.0).suffix(" RPM"));
                                point.rpm=((point.rpm+50)/100*100).clamp(minimum,maximum);
                            });}
                            ui.horizontal(|ui|{if ui.add_enabled(zone.points.len()<8,egui::Button::new("Add point")).clicked(){if let Some(last)=zone.points.last().cloned(){zone.points.push(blade18::CurvePoint {celsius:(last.celsius+1.0).min(84.0),rpm:last.rpm});}}
                                if ui.add_enabled(zone.points.len()>2,egui::Button::new("Remove last")).clicked(){zone.points.pop();}
                            });
                            if index==2{ui.label(RichText::new("Higher A/B is a cooling proxy, not battery temperature.").small());}
                        });
                    }
                    ui.separator();
                    ui.horizontal(|ui|{ui.label("Downward hold");ui.add(egui::DragValue::new(&mut self.curve_settings.downward_hold_seconds).range(1..=60).suffix(" s"));});
                    ui.horizontal(|ui|{ui.label("Session limit");ui.add(egui::DragValue::new(&mut self.curve_settings.session_seconds).range(30..=28800).speed(60.0).suffix(" s"));});
                    ui.label(RichText::new("Saved curves never start automatically at login or on battery.").small());
                });
            });
            let valid=self.curve_settings.validate();
            if let Err(error)=&valid{ui.colored_label(Color32::from_rgb(255,160,140),error.to_string());}
            if let Some(error)=&self.curve_error{ui.colored_label(Color32::from_rgb(255,160,140),error);}
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!self.preview&&!active&&idle&&valid.is_ok()&&self.curve_error.is_none(),egui::Button::new("Save curves")).clicked(){self.message=match crate::blade_curves::save(&self.curve_settings){Ok(())=>"Curves saved; automatic startup remains off".into(),Err(error)=>format!("Could not save curves: {error:#}")};}
                let ready=!self.preview&&!active&&idle&&valid.is_ok()&&self.last_power==Some(true)&&self.state.as_ref().is_some_and(|s|s.performance==PerfMode::Balanced&&s.fan_mode==FanMode::Auto);
                if ui.add_enabled(ready,egui::Button::new("Start curves")).clicked(){match crate::blade_curves::launch(&self.curve_settings,self.curve_control.clone(),ctx.clone()){Ok(events)=>{self.curve_events=Some(events);self.curve_status=None;self.message="Starting supervised curve session…".into();},Err(error)=>self.message=format!("Could not start curves: {error:#}")}}
                if ui.add_enabled(active&&idle,egui::Button::new("Stop + automatic")).clicked(){self.start(ctx,"Stop curves",None,blade18::status);}
            });
            ui.label(RichText::new("Stops on sensor/power faults, resume or session timeout. Separate watchdog restores Auto if the app/controller exits.").small().color(Color32::from_gray(170)));
        });
        ui.add_space(4.0);
    }
}
