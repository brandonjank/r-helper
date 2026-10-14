use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

/// Passive Razer hardware inventory; optionally send audited getters to one Blade 18 interface.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Save JSON to a new file (will not overwrite an existing report).
    #[arg(long)]
    output: Option<PathBuf>,
    /// Explicitly send getter requests to this inventory index. No setters or initialization.
    #[arg(long)]
    probe_interface: Option<usize>,
    /// Read status using the Blade 18 control interface verified on the development laptop.
    #[arg(long, conflicts_with = "probe_interface")]
    probe: bool,
    /// Query source-backed LED RGB/effect getters on the verified endpoint. No writes or LED-ID sweep.
    #[arg(long, conflicts_with_all = ["probe", "probe_interface", "validate_brightness", "validate_batch", "validate_fans", "restore_state"])]
    probe_lighting: bool,
    /// Read candidate controller temperatures, firmware, and third-fan getters. No writes or scan.
    #[arg(long, conflicts_with_all = ["probe", "probe_interface", "probe_lighting", "validate_brightness", "validate_batch", "validate_fans", "validate_fan_zones", "restore_state"])]
    probe_sensors: bool,
    /// Temporarily adjust brightness by 8/255, read back, then restore the original.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface"])]
    validate_brightness: bool,
    /// Test Blade 18 lighting, battery care, normal modes/boosts and bounded fans, restoring each case.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface", "validate_brightness"])]
    validate_batch: bool,
    /// Observe actual fan ramps at 4000/4500 RPM for up to 90 seconds, then restore.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface", "validate_brightness", "validate_batch"])]
    validate_fans: bool,
    /// Test unequal zone targets at 4000/4500 RPM in both directions for 60 seconds, then restore.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface", "probe_lighting", "validate_brightness", "validate_batch", "validate_fans", "restore_state"])]
    validate_fan_zones: bool,
    /// Restore a model-tagged recovery snapshot created alongside a batch/fan report.
    #[arg(long, conflicts_with_all = ["output", "probe", "probe_interface", "validate_brightness", "validate_batch", "validate_fans"])]
    restore_state: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.validate_fan_zones {
        println!("{}", librazer::blade18::validate_fan_zones(args.output.as_ref().unwrap())?);
        return Ok(());
    }
    if let Some(path) = args.restore_state {
        println!("{}", serde_json::to_string_pretty(&librazer::blade18::restore_record(&path)?)?);
        return Ok(());
    }
    if args.validate_fans {
        println!("{}", librazer::blade18::validate_fans(args.output.as_ref().unwrap())?);
        return Ok(());
    }
    if args.validate_batch {
        let report = librazer::blade18::validate_batch(args.output.as_ref().unwrap())?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    // Validate the output destination before accessing hardware.
    let mut output = args
        .output
        .as_ref()
        .map(|path| {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .with_context(|| format!("Cannot create {}", path.display()))
        })
        .transpose()?;
    let mut failed_validation = false;
    let report = if args.validate_brightness {
        let file = output.as_mut().expect("clap requires an output file");
        let validation = librazer::brightness::validate_keyboard(|original| {
            use std::io::Write;
            serde_json::to_writer_pretty(
                &mut *file,
                &serde_json::json!({
                    "operation": "brightness_validation_in_progress", "original_brightness": original
                }),
            )?;
            file.flush()?;
            file.sync_all()?;
            Ok(())
        })?;
        failed_validation = !validation.passed;
        serde_json::json!({"brightness_validation": validation})
    } else if args.probe_sensors {
        librazer::diagnostics::collect_sensors()?
    } else if args.probe_lighting {
        librazer::diagnostics::collect_lighting()?
    } else if args.probe {
        librazer::diagnostics::collect_status()?
    } else {
        librazer::diagnostics::collect(args.probe_interface)?
    };
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(file) = output.as_mut() {
        use std::io::{Seek, SeekFrom, Write};
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "{json}")?;
        let report_length = file.stream_position()?;
        file.set_len(report_length)?;
        file.sync_all()?;
    } else {
        println!("{json}");
    }
    anyhow::ensure!(!failed_validation, "Brightness validation failed; check the saved report for original value and restoration result");
    Ok(())
}
