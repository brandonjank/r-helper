//! Explicit development-only hardware harness; never called during normal startup.
use crate::{
    blade_display, blade_policy,
    blade_profiles::{self, Named, Settings},
};
use anyhow::{ensure, Result};
use librazer::blade18;
use std::{
    fs::File,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub fn dispatch() -> Option<Result<()>> {
    let mut args = std::env::args_os();
    args.next();
    if args.next()?.to_str()? != "--test-feature-batch" {
        return None;
    }
    Some((|| {
        let directory =
            PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("Missing test directory"))?);
        ensure!(args.next().is_none(), "Unexpected test argument");
        run(directory)
    })())
}
fn wait_for(mut check: impl FnMut() -> Result<bool>) -> Result<()> {
    let start = Instant::now();
    loop {
        if check()? {
            return Ok(());
        }
        ensure!(start.elapsed() < Duration::from_secs(15), "Hardware transition timed out");
        std::thread::sleep(Duration::from_millis(500));
    }
}
fn run(directory: PathBuf) -> Result<()> {
    ensure!(
        librazer::diagnostics::is_target_system() && blade18::ac_connected()?,
        "Blade 18 on AC required"
    );
    std::fs::create_dir(&directory)?;
    let original_state = blade18::status()?;
    ensure!(
        original_state.fan_mode == librazer::types::FanMode::Auto,
        "Automatic cooling required"
    );
    let original = Named::capture("Original".into())?;
    let mut recovery = File::options()
        .create_new(true)
        .write(true)
        .open(directory.join("hardware.recovery.json"))?;
    serde_json::to_writer_pretty(
        &mut recovery,
        &serde_json::json!({"model":"RZ09-0529","original":original_state}),
    )?;
    recovery.sync_all()?;
    std::fs::write(
        directory.join("lighting.recovery.json"),
        serde_json::to_vec_pretty(&original.lighting)?,
    )?;
    let mut cases = vec![];
    let result = (|| {
        let mut p = original.clone();
        p.name = "Batch profile".into();
        p.hardware.brightness = original.hardware.brightness.saturating_sub(8);
        p.lighting.brightness = p.hardware.brightness;
        p.lighting.style = blade18::lighting::Style::DualBreathing;
        p.lighting.speed = None;
        p.apply()?;
        ensure!(
            blade18::status()?.brightness == p.hardware.brightness,
            "Named profile brightness mismatch"
        );
        original.apply()?;
        cases.push("named profile apply + restoration");
        let mut invalid = p.clone();
        invalid.display.panel = "invalid-panel".into();
        ensure!(invalid.apply().is_err(), "Wrong panel was accepted");
        ensure!(
            blade18::status()?.brightness == original.hardware.brightness,
            "Failed profile altered brightness"
        );
        cases.push("invalid panel rejection + profile rollback");
        let settings = Settings { named: vec![p.clone()], ..Default::default() };
        let path = directory.join("settings.json");
        blade_profiles::save(&path, &settings)?;
        let loaded = blade_profiles::load(&path)?;
        ensure!(loaded.named.len() == 1, "Named profile roundtrip failed");
        let imported = blade_profiles::import(&serde_json::to_string(&loaded)?)?;
        ensure!(
            !imported.application_switching
                && !imported.automatic
                && !imported.lighting_policy.enabled,
            "Import enabled automation"
        );
        cases.push("profile save/load + import/export");
        let shared = Arc::new(Mutex::new(Settings::default()));
        let (mut watcher, _) = blade_policy::Watcher::new(
            shared.clone(),
            eframe::egui::Context::default(),
            crate::blade_curves::shared(),
        );
        {
            let mut s = shared.lock().unwrap();
            s.lighting_policy.enabled = true;
            s.lighting_policy.idle_ac_seconds = 1;
        }
        watcher.wake();
        wait_for(|| Ok(blade18::status()?.brightness == 0))?;
        {
            shared.lock().unwrap().lighting_policy.enabled = false;
        }
        watcher.wake();
        wait_for(|| Ok(blade18::status()?.brightness == original.hardware.brightness))?;
        watcher.stop();
        cases.push("real idle policy blank + disable restoration");
        let shared = Arc::new(Mutex::new(Settings {
            lighting_ac: Some(original.lighting.clone()),
            ..Default::default()
        }));
        {
            let mut s = shared.lock().unwrap();
            s.lighting_policy.enabled = true;
            s.lighting_policy.display_off = true;
            s.lighting_policy.restore_on_resume = true;
        }
        let (mut watcher, updates) = blade_policy::Watcher::new(
            shared.clone(),
            eframe::egui::Context::default(),
            crate::blade_curves::shared(),
        );
        watcher.wake();
        std::thread::sleep(Duration::from_secs(1));
        watcher.sender().send(crate::blade_power::Event::Display(false))?;
        wait_for(|| Ok(blade18::status()?.brightness == 0))?;
        watcher.sender().send(crate::blade_power::Event::Display(true))?;
        wait_for(|| Ok(blade18::status()?.brightness == original.hardware.brightness))?;
        cases.push("display-state policy event blank + restoration (injected event)");
        blade18::change(blade18::Change::Brightness(
            original.hardware.brightness.saturating_sub(8),
        ))?;
        watcher.sender().send(crate::blade_power::Event::Resume)?;
        wait_for(|| Ok(blade18::status()?.brightness == original.hardware.brightness))?;
        cases.push("resume policy replay (injected event)");
        watcher.stop();
        ensure!(
            !updates
                .try_iter()
                .any(|u| u.message.contains("unavailable") || u.message.starts_with("Automation:")),
            "Policy notification registration or replay reported an error"
        );
        if let Some(exe) = blade_policy::foreground_exe() {
            p.executable = exe;
            let shared = Arc::new(Mutex::new(Settings {
                named: vec![p.clone()],
                application_switching: true,
                ..Default::default()
            }));
            let (mut watcher, _) = blade_policy::Watcher::new(
                shared.clone(),
                eframe::egui::Context::default(),
                crate::blade_curves::shared(),
            );
            watcher.wake();
            wait_for(|| Ok(blade18::status()?.brightness == p.hardware.brightness))?;
            shared.lock().unwrap().application_switching = false;
            watcher.wake();
            wait_for(|| Ok(blade18::status()?.brightness == original.hardware.brightness))?;
            watcher.stop();
            cases.push("foreground application match + disable restoration");
        } else {
            anyhow::bail!("Foreground application unavailable; application test incomplete");
        }
        let info = librazer::display::inventory()?;
        let mut target = info.current.clone();
        target.hz = *info
            .rates
            .iter()
            .find(|hz| **hz != info.current.hz)
            .ok_or_else(|| anyhow::anyhow!("No alternate refresh rate"))?;
        let preview = blade_display::begin(&target)?;
        let pending = PathBuf::from(preview["display_pending"].as_str().unwrap());
        blade_display::finish(&pending, false)?;
        ensure!(
            librazer::display::inventory()?.current == original.display,
            "Display Revert failed"
        );
        cases.push("guarded refresh Revert");
        let preview = blade_display::begin(&target)?;
        let pending = PathBuf::from(preview["display_pending"].as_str().unwrap());
        blade_display::finish(&pending, true)?;
        ensure!(librazer::display::inventory()?.current == target, "Display Keep failed");
        librazer::display::apply(&original.display)?;
        cases.push("guarded refresh Keep + restoration");
        let preview = blade_display::begin(&target)?;
        let pending = PathBuf::from(preview["display_pending"].as_str().unwrap());
        let start = Instant::now();
        while !pending.with_extension("done").exists() {
            ensure!(start.elapsed() < Duration::from_secs(35), "Display guard did not recover");
            std::thread::sleep(Duration::from_millis(500));
        }
        ensure!(
            librazer::display::inventory()?.current == original.display
                && !blade_display::is_pending(),
            "Display timeout did not restore original"
        );
        cases.push("independent display timeout recovery");
        Ok::<_, anyhow::Error>(())
    })();
    let lighting = original.lighting.apply(&blade_profiles::journal_path("batch-final-lighting")?);
    let display = librazer::display::apply(&original.display);
    let hardware = blade18::restore_record(&directory.join("hardware.recovery.json"));
    let mut report =
        File::options().create_new(true).write(true).open(directory.join("result.json"))?;
    writeln!(
        report,
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"passed":result.is_ok(),"cases":cases,"error":result.as_ref().err().map(|e|format!("{e:#}")),"lighting_restored":lighting.is_ok(),"display_restored":display.is_ok(),"hardware_restored":hardware.is_ok()})
        )?
    )?;
    report.sync_all()?;
    lighting?;
    display?;
    hardware?;
    result
}
