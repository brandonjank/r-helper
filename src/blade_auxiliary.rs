//! Isolate foreground lighting ownership from the persistent tray controller.
use anyhow::{ensure, Context, Result};
use eframe::egui;
use librazer::blade18::{
    self,
    lighting::{Preset, Style},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    preset: Preset,
    rgb: [u8; 3],
    highlight: Option<u8>,
}
impl Request {
    fn validate(&self) -> Result<()> {
        self.preset.validate()?;
        ensure!(self.preset.style == Style::Static,
            "Apply a static keyboard effect before an auxiliary test; other effects cannot be preserved exactly during Windows lighting control");
        ensure!(self.highlight.is_none_or(|i| (1..=18).contains(&i)), "Invalid auxiliary lamp");
        Ok(())
    }
    fn keyboard(&self) -> [u8; 3] {
        self.preset
            .color
            .map(|v| ((u16::from(v) * u16::from(self.preset.brightness) + 127) / 255) as u8)
    }
}
fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    let file = std::fs::File::options().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&file, value)?;
    file.sync_all()?;
    Ok(())
}
fn load(path: &Path) -> Result<Request> {
    ensure!(std::fs::metadata(path)?.len() < 65536, "Auxiliary request too large");
    let request: Request = serde_json::from_slice(&std::fs::read(path)?)?;
    request.validate()?;
    Ok(request)
}
fn command(mode: &str, request: &Path, output: &Path) -> Result<std::process::Command> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.arg(mode).arg(request).arg(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    Ok(command)
}
/// Called under the GUI's device operation lock. That lock keeps profile/idle
/// workers from writing while this bounded test and its restoration run.
pub fn run(rgb: [u8; 3], highlight: Option<u8>, path: &Path) -> Result<serde_json::Value> {
    let request = Request {
        preset: Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo)?,
        rgb,
        highlight,
    };
    request.validate()?;
    save(path, &request)?;
    let guard_output = path.with_extension("guard.json");
    let mut guard = command("--auxiliary-recover", path, &guard_output)?.spawn()?;
    let ready = path.with_extension("ready");
    let start = Instant::now();
    let ready_result = (|| -> Result<()> {
        while !ready.exists() {
            ensure!(
                guard.try_wait()?.is_none(),
                "Auxiliary recovery helper exited; no preview started"
            );
            ensure!(
                start.elapsed() < Duration::from_secs(5),
                "Recovery helper not ready; no preview started"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    })();
    if let Err(error) = ready_result {
        // No preview has been launched; disarm any late-starting guard instead
        // of allowing it to overwrite settings selected after this failed start.
        save(&path.with_extension("finished"), &true)?;
        return Err(error);
    }
    let output = path.with_extension("preview.json");
    let preview = (|| -> Result<serde_json::Value> {
        // LampArray writes do not enable the separate underside power gate.
        // Recovery retains its original power and brightness values.
        let mut powered = request.preset.clone();
        powered.underside = true;
        powered.underside_brightness = powered.underside_brightness.max(200);
        powered.apply(&path.with_extension("power-on.json"))?;
        let mut child = command("--auxiliary-preview", path, &output)?.spawn()?;
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() < Duration::from_secs(25) => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                other => {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("Auxiliary test did not finish within its deadline: {other:?}");
                }
            }
        };
        ensure!(status.success(), "Preview window failed; saved lighting will be reapplied");
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(&output)?)?;
        ensure!(report["accepted"] == true, "Windows did not accept the preview: {report}");
        Ok(report)
    })();
    // The foreground owner has exited before firmware restoration. Dropping a
    // LampArray inside the still-foreground tray process was insufficient on RB18.
    let restored = request.preset.apply(&path.with_extension("restore.json"));
    let log_result = save(
        &path.with_extension("result.json"),
        &serde_json::json!({
            "preview":preview.as_ref().ok(),"preview_error":preview.as_ref().err().map(|e|format!("{e:#}")),
            "firmware_restored":restored.is_ok(),"restore_error":restored.as_ref().err().map(|e|format!("{e:#}")),
            "visual_confirmation_required":true
        }),
    );
    if restored.is_ok() {
        save(&path.with_extension("finished"), &true)?;
        let start = Instant::now();
        while guard.try_wait()?.is_none() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    restored.context("Saved lighting restoration failed; recovery helper will retry")?;
    log_result?;
    let preview = preview?;
    Ok(
        serde_json::json!({"lamp_preview":preview,"firmware_restored":true,"lighting_controls":blade18::lighting::status()?}),
    )
}

pub fn dispatch() -> Option<Result<()>> {
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1)?.as_str();
    if !matches!(mode, "--auxiliary-preview" | "--auxiliary-recover" | "--auxiliary-test") {
        return None;
    }
    Some((|| {
        ensure!(args.len() == 4, "Invalid auxiliary helper arguments");
        ensure!(librazer::diagnostics::is_target_system(), "Auxiliary test is model scoped");
        let path = PathBuf::from(&args[2]);
        let output = PathBuf::from(&args[3]);
        let request = load(&path)?;
        if mode == "--auxiliary-test" {
            // Explicit, bounded diagnostic entry point. Keep reversible vendor
            // ownership local to this batch rather than a persistent GUI session.
            let mut ownership = crate::blade_vendor::Session::start()?;
            let preview = run(request.rgb, request.highlight, &output);
            let release = ownership.stop();
            save(
                &output.with_extension("batch-result.json"),
                &serde_json::json!({
                    "preview": preview.as_ref().ok(),
                    "preview_error": preview.as_ref().err().map(|e|format!("{e:#}")),
                    "vendor_restored": release.is_ok(),
                    "vendor_restore_error": release.as_ref().err().map(|e|format!("{e:#}"))
                }),
            )?;
            release?;
            preview?;
            return Ok(());
        }
        if mode == "--auxiliary-recover" {
            save(&path.with_extension("ready"), &true)?;
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(45) {
                if path.with_extension("finished").exists() {
                    save(
                        &output,
                        &serde_json::json!({"fallback":false,"restored_by_parent":true}),
                    )?;
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let restored = request.preset.apply(&output.with_extension("restore.json"));
            save(
                &output,
                &serde_json::json!({"fallback":true,"firmware_restored":restored.is_ok(),"error":restored.as_ref().err().map(|e|format!("{e:#}"))}),
            )?;
            restored?;
            return Ok(());
        }
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("R-Helper · Lighting test")
                .with_inner_size([420.0, 190.0])
                .with_active(true)
                .with_resizable(false),
            ..Default::default()
        };
        eframe::run_native(
            "R-Helper lighting test",
            options,
            Box::new(move |_| {
                Ok(Box::new(Preview {
                    request: Some(request),
                    path: output.clone(),
                    started: Instant::now(),
                    receiver: None,
                    accepted: false,
                    details: None,
                    message: "Preparing lighting test…".into(),
                }))
            }),
        )
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(())
    })())
}
struct Preview {
    request: Option<Request>,
    path: PathBuf,
    started: Instant,
    receiver: Option<std::sync::mpsc::Receiver<Result<serde_json::Value, String>>>,
    accepted: bool,
    details: Option<serde_json::Value>,
    message: String,
}
impl eframe::App for Preview {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.request.is_some()
            && self.started.elapsed() >= Duration::from_millis(250)
            && ctx.input(|input| input.viewport().focused == Some(true))
        {
            let request = self.request.take().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            self.receiver = Some(rx);
            let journal = self.path.with_extension("colors.json");
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                let result = blade18::lamp_array::preview_auxiliary(
                    request.rgb,
                    request.highlight,
                    request.keyboard(),
                    &journal,
                );
                let _ = tx.send(result.map_err(|e| format!("{e:#}")));
                repaint.request_repaint();
            });
        }
        if let Some(rx) = &self.receiver {
            if let Ok(result) = rx.try_recv() {
                self.receiver = None;
                self.started = Instant::now();
                self.accepted = result.is_ok();
                self.message = match result {
                    Ok(details) => {
                        self.details = Some(details);
                        "Watch the underside lights. The logo remains green-only.".into()
                    }
                    Err(e) => e,
                };
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Auxiliary lighting test");
            ui.label(&self.message);
            ui.label(format!(
                "Closes in {} seconds; saved lighting will be reapplied.",
                15u64.saturating_sub(self.started.elapsed().as_secs())
            ));
            if ui.button("End test and restore lighting").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        if self.started.elapsed() >= Duration::from_secs(15) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        blade18::lamp_array::release();
        let _ = save(
            &self.path,
            &serde_json::json!({"accepted":self.accepted,"message":self.message,"api_details":self.details,"owner_process_exiting":true}),
        );
        // Parent reads the report only after this process exits.
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request {
            preset: Preset {
                style: Style::Static,
                color: [32, 64, 255],
                second: [255, 64, 32],
                keys: vec![[32, 64, 255]; 114],
                brightness: 176,
                underside: false,
                underside_brightness: 64,
                logo: librazer::types::LogoMode::Static,
                speed: None,
            },
            rgb: [255, 32, 32],
            highlight: None,
        }
    }
    #[test]
    fn auxiliary_requires_recoverable_static_lighting() {
        let mut r = request();
        assert!(r.validate().is_ok());
        r.preset.style = Style::Breathing;
        assert!(r.validate().is_err());
        r.preset.style = Style::Static;
        r.highlight = Some(19);
        assert!(r.validate().is_err());
    }
    #[test]
    fn keyboard_color_includes_original_brightness() {
        let mut r = request();
        assert_eq!(r.keyboard(), [22, 44, 176]);
        r.preset.brightness = 0;
        assert_eq!(r.keyboard(), [0; 3]);
        r.preset.brightness = 255;
        assert_eq!(r.keyboard(), r.preset.color);
    }
}
