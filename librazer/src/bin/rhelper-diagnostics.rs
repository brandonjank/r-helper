use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

/// Passive Razer hardware inventory; optionally send audited getters to one Blade 18 interface.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Restore only the two switches from a lighting validation recovery file.
    #[arg(long, conflicts_with_all = ["output", "probe", "probe_interface", "probe_lighting", "probe_sensors", "probe_thermal_capabilities", "validate_lighting_switches", "validate_brightness", "validate_batch", "validate_fans", "validate_fan_zones", "identify_main_fan", "restore_state"])]
    restore_lighting: Option<PathBuf>,
    /// Toggle startup/underside switches, verify independent regions, then restore.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface", "probe_lighting", "probe_sensors", "probe_thermal_capabilities", "validate_brightness", "validate_batch", "validate_fans", "validate_fan_zones", "identify_main_fan", "restore_state"])]
    validate_lighting_switches: bool,
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
    /// Read model-specific fan-ID, timing and SKU capability getters from vendor protocol metadata.
    #[arg(long, conflicts_with_all = ["probe", "probe_interface", "probe_lighting", "probe_sensors", "validate_brightness", "validate_batch", "validate_fans", "validate_fan_zones", "identify_main_fan", "restore_state"])]
    probe_thermal_capabilities: bool,
    /// Temporarily adjust brightness by 8/255, read back, then restore the original.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface"])]
    validate_brightness: bool,
    /// Test Blade 18 lighting, battery care, normal modes/boosts and bounded fans, restoring each case.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface", "validate_brightness"])]
    validate_batch: bool,
    /// Verify shared 4000/4500 RPM targets for up to 150 seconds, then restore.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface", "validate_brightness", "validate_batch"])]
    validate_fans: bool,
    /// Verify unequal targets at 4000/4500 RPM for 90 seconds per direction with three-selector telemetry, then restore.
    #[arg(long, requires = "output", conflicts_with_all = ["probe", "probe_interface", "probe_lighting", "validate_brightness", "validate_batch", "validate_fans", "restore_state"])]
    validate_fan_zones: bool,
    /// Restore a model-tagged recovery snapshot created alongside a batch/fan report.
    #[arg(long, conflicts_with_all = ["output", "probe", "probe_interface", "validate_brightness", "validate_batch", "validate_fans"])]
    restore_state: Option<PathBuf>,
    /// Test one target for 40 seconds in shared manual mode, then restore all captured settings.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=3), requires = "output", conflicts_with_all = ["probe", "probe_interface", "probe_lighting", "probe_sensors", "validate_brightness", "validate_batch", "validate_fans", "validate_fan_zones", "restore_state"])]
    identify_main_fan: Option<u8>,
    /// Investigate the source-backed fan setter enable byte (1), with captured-target restoration.
    #[arg(long, requires = "identify_main_fan")]
    fan_write_enable: bool,
    /// Compare the source-backed mode prefix 0 with upstream prefix 1; Balanced only.
    #[arg(long, requires = "identify_main_fan")]
    fan_alternate_mode_byte: bool,
    /// Compare transaction ID 1f and the protocol XOR checksum with upstream framing.
    #[arg(long, requires = "identify_main_fan")]
    fan_standard_framing: bool,
    /// Reapply the audited manual-mode command after setting the target to test latching.
    #[arg(long, requires = "identify_main_fan")]
    fan_latch_after_target: bool,
    /// Initialize both primary targets to 4000 RPM before the selected-zone comparison.
    #[arg(long, requires = "identify_main_fan")]
    fan_paired_initialization: bool,
    /// Settle all three fans at 3000 RPM for 60 seconds before changing only the selected target.
    #[arg(long, requires_all = ["identify_main_fan", "fan_paired_initialization", "fan_write_enable", "fan_alternate_mode_byte"], conflicts_with_all = ["fan_latch_after_target", "fan_standard_framing"])]
    fan_isolated_comparison: bool,
    /// Observation duration for the explicit fan lab (physical mapping defaults to 40 seconds).
    #[arg(long, requires = "identify_main_fan", default_value_t = 40, value_parser = clap::value_parser!(u16).range(20..=120))]
    fan_observe_seconds: u16,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(path) = args.restore_lighting {
        println!(
            "{}",
            serde_json::to_string_pretty(&librazer::blade18::lighting::restore_record(&path)?)?
        );
        return Ok(());
    }
    if args.validate_lighting_switches {
        let report = librazer::blade18::lighting::validate(args.output.as_ref().unwrap())?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if let Some(zone) = args.identify_main_fan {
        println!(
            "{}",
            librazer::blade18::identify_main_fan(
                args.output.as_ref().unwrap(),
                librazer::blade18::FanLabOptions {
                    zone,
                    enabled_write: args.fan_write_enable,
                    alternate_mode_byte: args.fan_alternate_mode_byte,
                    standard_framing: args.fan_standard_framing,
                    latch_after_target: args.fan_latch_after_target,
                    paired_initialization: args.fan_paired_initialization,
                    isolated_comparison: args.fan_isolated_comparison,
                    observe_seconds: args.fan_observe_seconds,
                }
            )?
        );
        return Ok(());
    }
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
    } else if args.probe_thermal_capabilities {
        librazer::diagnostics::collect_thermal_capabilities()?
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lighting_validation_requires_a_journal_and_excludes_other_actions() {
        assert!(Args::try_parse_from(["diag", "--validate-lighting-switches"]).is_err());
        assert!(Args::try_parse_from([
            "diag",
            "--validate-lighting-switches",
            "--output",
            "test.json"
        ])
        .is_ok());
        for other in ["--probe", "--probe-lighting", "--validate-fan-zones", "--validate-batch"] {
            assert!(Args::try_parse_from([
                "diag",
                "--validate-lighting-switches",
                "--output",
                "test.json",
                other
            ])
            .is_err());
        }
        assert!(
            Args::try_parse_from(["diag", "--restore-lighting", "test.json", "--probe"]).is_err()
        );
    }
    #[test]
    fn lab_options_cannot_enable_writes_without_an_explicit_scenario() {
        assert!(Args::try_parse_from(["diag", "--probe-sensors"]).is_ok());
        for flag in [
            "--fan-write-enable",
            "--fan-alternate-mode-byte",
            "--fan-standard-framing",
            "--fan-latch-after-target",
            "--fan-paired-initialization",
            "--fan-isolated-comparison",
        ] {
            assert!(Args::try_parse_from(["diag", flag]).is_err());
        }
        assert!(Args::try_parse_from(["diag", "--fan-observe-seconds", "90"]).is_err());
    }
    #[test]
    fn fan_scenarios_require_an_output_and_bounded_observation() {
        let isolated = [
            "diag",
            "--identify-main-fan",
            "3",
            "--output",
            "report.json",
            "--fan-paired-initialization",
            "--fan-write-enable",
            "--fan-alternate-mode-byte",
            "--fan-isolated-comparison",
        ];
        assert!(Args::try_parse_from(isolated).is_ok());
        assert!(
            Args::try_parse_from(isolated.into_iter().chain(["--fan-latch-after-target"])).is_err()
        );
        assert!(Args::try_parse_from(["diag", "--identify-main-fan", "1"]).is_err());
        for duration in ["19", "121"] {
            assert!(Args::try_parse_from([
                "diag",
                "--identify-main-fan",
                "1",
                "--output",
                "report.json",
                "--fan-observe-seconds",
                duration
            ])
            .is_err());
        }
        assert!(Args::try_parse_from([
            "diag",
            "--identify-main-fan",
            "2",
            "--output",
            "report.json",
            "--fan-paired-initialization",
            "--fan-observe-seconds",
            "90"
        ])
        .is_ok());
    }
}
