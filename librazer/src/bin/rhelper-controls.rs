//! Bounded source-backed HID discovery. No generic raw-command escape hatch.
use anyhow::Result;
use clap::Parser;
use serde_json::json;
use std::{io::Write, path::PathBuf};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    output: PathBuf,
    #[arg(long, conflicts_with_all = ["recover","lamp_array"])]
    validate: bool,
    #[arg(long, conflicts_with_all = ["validate","lamp_array"])]
    recover: Option<PathBuf>,
    #[arg(long,conflicts_with_all=["validate","recover"])]
    lamp_array: bool,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let mut file = std::fs::File::options().write(true).create_new(true).open(&args.output)?;
    if args.lamp_array {
        serde_json::to_writer_pretty(&mut file, &librazer::blade18::lamp_array::inventory()?)?;
        file.sync_all()?;
        return Ok(());
    }
    if let Some(path) = args.recover {
        use librazer::blade18::extended::Snapshot;
        anyhow::ensure!(std::fs::metadata(&path)?.len() < 16384, "Recovery record too large");
        let snapshot: Snapshot = serde_json::from_slice(&std::fs::read(&path)?)?;
        snapshot.validate()?;
        std::fs::File::options()
            .create_new(true)
            .write(true)
            .open(path.with_extension("ready"))?
            .sync_all()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
        while std::time::Instant::now() < deadline && !path.with_extension("finished").exists() {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let restored =
            if path.with_extension("finished").exists() { Ok(()) } else { snapshot.restore() };
        serde_json::to_writer_pretty(
            &mut file,
            &json!({"fallback":!path.with_extension("finished").exists(),"restored":restored.is_ok(),"error":restored.as_ref().err().map(|e|format!("{e:#}"))}),
        )?;
        file.sync_all()?;
        return restored;
    }
    if args.validate {
        use librazer::blade18::extended::Snapshot;
        let snapshot = Snapshot::capture()?;
        let recovery = args.output.with_extension("recovery.json");
        let mut backup = std::fs::File::options().create_new(true).write(true).open(&recovery)?;
        serde_json::to_writer_pretty(&mut backup, &snapshot)?;
        backup.sync_all()?;
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command
            .arg("--recover")
            .arg(&recovery)
            .arg("--output")
            .arg(recovery.with_extension("guard-result.json"));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut guard = command.spawn()?;
        for _ in 0..30 {
            anyhow::ensure!(guard.try_wait()?.is_none(), "Control recovery guard exited");
            if recovery.with_extension("ready").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        anyhow::ensure!(
            recovery.with_extension("ready").exists(),
            "Control recovery guard not ready; no setter sent"
        );
        let start = std::time::Instant::now();
        let mut cases = vec![];
        for (index, (control, original)) in snapshot.values.iter().enumerate() {
            if start.elapsed().as_secs() > 30 {
                cases.push(json!({"control":control,"error":"Recovery deadline approaching"}));
                break;
            }
            let requested = control.test_value(original)?;
            let result =
                control.apply(&requested, &recovery.with_extension(format!("case{index}.json")));
            std::thread::sleep(std::time::Duration::from_secs(1));
            let restoration =
                control.apply(original, &recovery.with_extension(format!("restore{index}.json")));
            cases.push(json!({"control":control,"requested":requested,"applied":result.is_ok(),"error":result.err().map(|e|format!("{e:#}")),"restored":restoration.is_ok(),"restore_error":restoration.as_ref().err().map(|e|format!("{e:#}"))}));
            if restoration.is_err() {
                break;
            }
        }
        let restoration = snapshot.restore();
        if restoration.is_ok() {
            std::fs::File::options()
                .create_new(true)
                .write(true)
                .open(recovery.with_extension("finished"))?
                .sync_all()?;
        }
        let passed = cases.len() == snapshot.values.len()
            && cases.iter().all(|c| c["applied"] == true && c["restored"] == true)
            && restoration.is_ok();
        serde_json::to_writer_pretty(
            &mut file,
            &json!({"cases":cases,"restored":restoration.is_ok(),"error":restoration.as_ref().err().map(|e|format!("{e:#}")),"passed":passed}),
        )?;
        file.sync_all()?;
        restoration?;
        anyhow::ensure!(passed, "Some controls did not pass; inspect report");
        return Ok(());
    }
    serde_json::to_writer_pretty(&mut file, &librazer::blade18::extended::inventory()?)?;
    file.sync_all()?;
    writeln!(std::io::stdout(), "Getter inventory saved")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn explicit_output_and_exclusive_modes_are_required() {
        assert!(Args::try_parse_from(["controls"]).is_err());
        assert!(Args::try_parse_from([
            "controls",
            "--output",
            "x.json",
            "--validate",
            "--recover",
            "y.json"
        ])
        .is_err());
        assert!(Args::try_parse_from(["controls", "--output", "x.json", "--validate"]).is_ok());
    }
}
