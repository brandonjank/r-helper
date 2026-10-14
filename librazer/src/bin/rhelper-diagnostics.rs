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
    /// Temporarily adjust brightness by 8/255, read back, then restore the original.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface"])]
    validate_brightness: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
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
