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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_mode: Option<blade18::lighting::NativeMode>,
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
fn restore(request: &Request, path: &Path) -> Result<blade18::lighting::Lighting> {
    let mode_result = request
        .native_mode
        .map(|mode| blade18::lighting::set_native_mode(mode, &path.with_extension("mode.json")))
        .transpose();
    // Attempt firmware restoration even when restoring driver mode reports an error.
    let lighting = request.preset.apply(path);
    mode_result?;
    lighting
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
    run_kind(rgb, highlight, path, false, None)
}
fn run_kind(
    rgb: [u8; 3],
    highlight: Option<u8>,
    path: &Path,
    native: bool,
    keyboard_override: Option<[u8; 3]>,
) -> Result<serde_json::Value> {
    let request = Request {
        preset: Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo)?,
        rgb,
        highlight,
        native_mode: if native { Some(blade18::lighting::native_mode()?) } else { None },
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
        powered.underside = !native;
        powered.underside_brightness = powered.underside_brightness.max(200);
        powered.apply(&path.with_extension("power-on.json"))?;
        if native {
            blade18::lighting::set_native_mode(
                blade18::lighting::NativeMode::Driver,
                &path.with_extension("driver-mode.json"),
            )?;
        }
        if native {
            ensure!(highlight.is_none(), "Individual native columns are not physically mapped yet");
            let report = blade18::lighting::native_static_zones(
                keyboard_override.unwrap_or(request.preset.color),
                rgb,
                &path.with_extension("native-frame.json"),
            )?;
            save(&output, &report)?;
            std::thread::sleep(Duration::from_secs(20));
            return Ok(report);
        }
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
    let restored = restore(&request, &path.with_extension("restore.json"));
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

fn native_maintenance(
    path: &Path,
    output: &Path,
    owner_exit: bool,
    guard_exit: bool,
) -> Result<()> {
    let original = Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo)?;
    let mut ownership = crate::blade_vendor::Session::start()?;
    let recovery = ownership.recovery_path().to_path_buf();
    let applied = (|| -> Result<serde_json::Value> {
        let mut preset = original.clone();
        preset.style = Style::Static;
        preset.speed = None;
        preset.color = [0, 0, 255];
        preset.underside_color = Some([255, 0, 0]);
        preset.underside_keys = None;
        preset.underside = true;
        preset.underside_brightness = 200;
        let mut cases = vec![];
        for index in 0..if owner_exit || guard_exit { 1 } else { 4 } {
            match index {
                1 => {
                    preset.style = Style::PerKey;
                    for (i, key) in preset.keys.iter_mut().enumerate() {
                        *key = if i / 19 % 2 == 0 { [0, 0, 255] } else { [255, 64, 0] };
                    }
                    preset.underside_keys = Some(
                        (0..19)
                            .map(|i| match i % 3 {
                                0 => [255, 0, 0],
                                1 => [0, 255, 0],
                                _ => [0, 0, 255],
                            })
                            .collect(),
                    );
                }
                2 => {
                    preset.brightness = 0;
                    preset.underside = false;
                }
                3 => {
                    preset = original.clone();
                    preset.style = Style::Static;
                    preset.speed = None;
                    preset.underside_color = None;
                    preset.underside_keys = None;
                }
                _ => {}
            }
            preset.apply(&output.with_extension(format!("case-{index}.json")))?;
            let captured = Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo)?;
            ensure!(captured == preset, "Native case {index} capture mismatch");
            let decoded: Preset = serde_json::from_slice(&serde_json::to_vec(&captured)?)?;
            ensure!(decoded == preset, "Native case {index} profile roundtrip mismatch");
            let mode = blade18::lighting::native_mode()?;
            ensure!(
                mode == if preset.underside_color.is_some() {
                    blade18::lighting::NativeMode::Driver
                } else {
                    blade18::lighting::NativeMode::Firmware
                },
                "Native case {index} mode mismatch"
            );
            cases.push(serde_json::json!({"case":index,"metadata_verified":true,"profile_roundtrip":true,"mode":mode,"physical_pixels_verified":false}));
            std::thread::sleep(Duration::from_secs(2));
        }
        Ok(serde_json::json!({"cases":cases}))
    })();
    if owner_exit && applied.is_ok() {
        save(
            output,
            &serde_json::json!({"owner_exit":true,"recovery":recovery,"applied":applied.as_ref().ok()}),
        )?;
        std::mem::forget(ownership); // Normal process exit without Drop simulates owner loss.
        return Ok(());
    }
    if guard_exit && applied.is_ok() {
        ownership.test_guard_exit()?;
    }
    let released = ownership.stop();
    let restored = Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo)?;
    let exact = restored == original
        && blade18::lighting::native_mode()? == blade18::lighting::NativeMode::Firmware;
    save(
        output,
        &serde_json::json!({"applied":applied.as_ref().ok(),"apply_error":applied.as_ref().err().map(|e|format!("{e:#}")),"restored":released.is_ok()&&exact,"restore_error":released.as_ref().err().map(|e|format!("{e:#}")),"guard_exit":guard_exit,"recovery":recovery}),
    )?;
    released?;
    applied?;
    ensure!(exact, "Native maintenance original lighting mismatch");
    if !guard_exit {
        let owner_output = output.with_extension("owner-exit.json");
        let mut child = command("--native-owner-exit", path, &owner_output)?.spawn()?;
        let start = Instant::now();
        while child.try_wait()?.is_none() {
            ensure!(start.elapsed() < Duration::from_secs(20), "Owner-exit test still running");
            std::thread::sleep(Duration::from_millis(100));
        }
        ensure!(std::fs::metadata(&owner_output)?.len() < 65536, "Owner report too large");
        let owner: serde_json::Value = serde_json::from_slice(&std::fs::read(&owner_output)?)?;
        let journal = PathBuf::from(
            owner["recovery"].as_str().ok_or_else(|| anyhow::anyhow!("Missing owner recovery"))?,
        );
        let start = Instant::now();
        while !journal.with_extension("done").exists() {
            ensure!(start.elapsed() < Duration::from_secs(20), "Owner recovery did not complete");
            std::thread::sleep(Duration::from_millis(100));
        }
        let done: serde_json::Value =
            serde_json::from_slice(&std::fs::read(journal.with_extension("done"))?)?;
        ensure!(done["ok"] == true, "Owner-exit native recovery failed");
        ensure!(
            Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo)? == original,
            "Owner-exit lighting mismatch"
        );
        let fallback_output = output.with_extension("guard-exit.json");
        native_maintenance(path, &fallback_output, false, true)?;
        let mut gpu_ownership = crate::blade_vendor::Session::start()?;
        let gpu_preview = crate::blade_gpu::preview(
            crate::blade_gpu::Offsets { gpu: -15, vram: 0 },
            &output.with_extension("gpu-preview.json"),
        );
        let gpu_release = gpu_ownership.stop();
        save(
            &output.with_extension("complete.json"),
            &serde_json::json!({"native_cases":4,"owner_exit_restored":true,"guard_exit_fallback_restored":true,"gpu_preview_restored":gpu_preview.is_ok(),"gpu_error":gpu_preview.as_ref().err().map(|e|format!("{e:#}")),"gpu_vendor_restored":gpu_release.is_ok(),"physical_mapping_confirmation_required":true}),
        )?;
        gpu_release?;
        gpu_preview?;
    }
    Ok(())
}
pub fn dispatch() -> Option<Result<()>> {
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1)?.as_str();
    if !matches!(
        mode,
        "--auxiliary-preview"
            | "--auxiliary-recover"
            | "--auxiliary-test"
            | "--native-lighting-test"
            | "--native-lighting-batch"
            | "--native-channel-test"
            | "--native-profile-test"
            | "--native-cell-markers-test"
            | "--native-maintenance-batch"
            | "--native-owner-exit"
            | "--native-lighting-inspect"
    ) {
        return None;
    }
    Some((|| {
        ensure!(args.len() == 4, "Invalid auxiliary helper arguments");
        ensure!(librazer::diagnostics::is_target_system(), "Auxiliary test is model scoped");
        let path = PathBuf::from(&args[2]);
        let output = PathBuf::from(&args[3]);
        let request = load(&path)?;
        if mode == "--native-maintenance-batch" || mode == "--native-owner-exit" {
            let result = native_maintenance(&path, &output, mode == "--native-owner-exit", false);
            if let Err(error) = &result {
                save(
                    &output.with_extension("failure.json"),
                    &serde_json::json!({"completed":false,"error":format!("{error:#}")}),
                )?;
            }
            return result;
        }
        if mode == "--native-profile-test" || mode == "--native-cell-markers-test" {
            let original = Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo)?;
            let mut ownership = crate::blade_vendor::Session::start()?;
            let applied = (|| -> Result<serde_json::Value> {
                let mut preset = original.clone();
                preset.style = Style::Static;
                preset.speed = None;
                preset.color = [0, 0, 255];
                preset.underside_color = Some([255, 0, 0]);
                preset.underside = true;
                preset.underside_brightness = 200;
                if mode == "--native-cell-markers-test" {
                    let mut cells = vec![[0, 0, 0]; 19];
                    cells[0] = [255, 0, 0];
                    cells[9] = [0, 255, 0];
                    cells[18] = [0, 0, 255];
                    preset.underside_keys = Some(cells);
                }
                preset.apply(&output.with_extension("apply.json"))?;
                let captured =
                    Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo)?;
                ensure!(captured == preset, "Independent underside profile capture lost color");
                let encoded = serde_json::to_vec(&captured)?;
                let decoded: Preset = serde_json::from_slice(&encoded)?;
                ensure!(decoded == preset, "Independent underside profile roundtrip lost color");
                save(
                    &output.with_extension("active.json"),
                    &serde_json::json!({"preset":captured,"native_mode":blade18::lighting::native_mode()?,"capture_roundtrip":true}),
                )?;
                std::thread::sleep(Duration::from_secs(20));
                Ok(
                    serde_json::json!({"profile_roundtrip":true,"keyboard":"blue","underside":if mode == "--native-cell-markers-test" {"cells 1 red, 10 green, 19 blue; others dark"} else {"red"},"physical_mapping_verified":false,"continuous_frames":false}),
                )
            })();
            let released = ownership.stop();
            let restored = Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo);
            let matches = restored.as_ref().is_ok_and(|p| p == &original);
            save(
                &output,
                &serde_json::json!({"applied":applied.as_ref().ok(),"apply_error":applied.as_ref().err().map(|e|format!("{e:#}")),"vendor_restored":released.is_ok(),"restore_error":released.as_ref().err().map(|e|format!("{e:#}")),"original_lighting_restored":matches,"native_mode":blade18::lighting::native_mode()?}),
            )?;
            released?;
            applied?;
            ensure!(matches, "Original lighting did not restore exactly");
            return Ok(());
        }
        if mode == "--native-lighting-inspect" {
            save(
                &output,
                &serde_json::json!({"read_only":true,"native_mode":blade18::lighting::native_mode()?}),
            )?;
            return Ok(());
        }
        if mode == "--auxiliary-test"
            || mode == "--native-lighting-test"
            || mode == "--native-lighting-batch"
            || mode == "--native-channel-test"
        {
            // Explicit, bounded diagnostic entry point. Keep reversible vendor
            // ownership local to this batch rather than a persistent GUI session.
            let mut ownership = crate::blade_vendor::Session::start()?;
            let preview = if mode == "--native-lighting-batch" {
                (|| -> Result<serde_json::Value> {
                    let mut cases = vec![];
                    for (name, rgb) in
                        [("red", [255, 32, 32]), ("green", [32, 255, 32]), ("blue", [32, 64, 255])]
                    {
                        let case_path = output.with_extension(format!("{name}.json"));
                        let report = run_kind(rgb, None, &case_path, true, None)?;
                        cases.push(serde_json::json!({"color":name,"report":report}));
                    }
                    Ok(serde_json::json!({"cases":cases,"visual_confirmation_required":true}))
                })()
            } else if mode == "--native-channel-test" {
                (|| -> Result<serde_json::Value> {
                    let mut cases = vec![];
                    for (name, keyboard, underside) in
                        [("pair-a", [255, 0, 0], [0, 0, 255]), ("pair-b", [0, 0, 255], [255, 0, 0])]
                    {
                        let report = run_kind(
                            underside,
                            None,
                            &output.with_extension(format!("{name}.json")),
                            true,
                            Some(keyboard),
                        )?;
                        cases.push(serde_json::json!({"phase":name,"requested_keyboard":keyboard,"requested_underside":underside,"report":report}));
                    }
                    Ok(serde_json::json!({"cases":cases,"visual_confirmation_required":true}))
                })()
            } else {
                run_kind(
                    request.rgb,
                    request.highlight,
                    &output,
                    mode == "--native-lighting-test",
                    None,
                )
            };
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
            let restored = restore(&request, &output.with_extension("restore.json"));
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
                underside_color: None,
                underside_keys: None,
                logo: librazer::types::LogoMode::Static,
                speed: None,
            },
            rgb: [255, 32, 32],
            highlight: None,
            native_mode: None,
        }
    }
    #[test]
    fn old_requests_remain_compatible_and_unknown_native_modes_are_rejected() {
        let mut value = serde_json::to_value(request()).unwrap();
        value.as_object_mut().unwrap().remove("native_mode");
        assert!(serde_json::from_value::<Request>(value.clone()).unwrap().native_mode.is_none());
        value["native_mode"] = serde_json::json!("Test");
        assert!(serde_json::from_value::<Request>(value).is_err());
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
