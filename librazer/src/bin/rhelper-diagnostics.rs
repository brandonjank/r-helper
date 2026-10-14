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
    let report = if args.probe {
        librazer::diagnostics::collect_status()?
    } else {
        librazer::diagnostics::collect(args.probe_interface)?
    };
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(file) = output.as_mut() {
        use std::io::Write;
        writeln!(file, "{json}")?;
    } else {
        println!("{json}");
    }
    Ok(())
}
