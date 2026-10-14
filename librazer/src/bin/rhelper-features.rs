use anyhow::{ensure, Result};
use clap::Parser;
use librazer::blade18::{
    self,
    lighting::{Preset, Style},
};
use std::{fs::File, io::Write, path::PathBuf};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    output: PathBuf,
    #[arg(long,conflicts_with_all=["preset","charge","recover_file"])]
    validate_lighting: bool,
    #[arg(long,conflicts_with_all=["charge","recover_file"])]
    preset: Option<PathBuf>,
    #[arg(long, conflicts_with = "recover_file")]
    charge: Option<bool>,
    #[arg(long, requires = "recover_after")]
    recover_file: Option<PathBuf>,
    #[arg(long,requires="recover_file",value_parser=clap::value_parser!(u64).range(1..=60))]
    recover_after: Option<u64>,
}
fn main() -> Result<()> {
    let a = Args::parse();
    let mut file = File::options().create_new(true).write(true).open(&a.output)?;
    if let Some(path) = a.recover_file {
        let snapshot: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        ensure!(snapshot["model"] == "RZ09-0529", "Wrong recovery model");
        let preset: Preset = serde_json::from_value(snapshot["preset"].clone())?;
        File::options()
            .create_new(true)
            .write(true)
            .open(path.with_extension("ready"))?
            .sync_all()?;
        for _ in 0..a.recover_after.unwrap() * 5 {
            if path.with_extension("finished").exists() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let restored = preset.apply(&a.output.with_extension("lighting.json"));
        let charge = blade18::charge::set(
            snapshot["charge"]
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("Invalid charge snapshot"))?,
            &a.output.with_extension("charge.json"),
        );
        writeln!(
            file,
            "{}",
            serde_json::json!({"lighting_restored":restored.is_ok(),"charge_restored":charge.is_ok()})
        )?;
        file.sync_all()?;
        restored?;
        charge?;
        return Ok(());
    }
    if let Some(path) = a.preset {
        let p: Preset = serde_json::from_slice(&std::fs::read(path)?)?;
        writeln!(
            file,
            "{}",
            serde_json::to_string_pretty(&p.apply(&a.output.with_extension("journal.json"))?)?
        )?;
    } else if let Some(value) = a.charge {
        writeln!(
            file,
            "{}",
            serde_json::json!({"charge_once":blade18::charge::set(value,&a.output.with_extension("journal.json"))?})
        )?;
    } else if a.validate_lighting {
        let original = Preset::capture(&blade18::lighting::status()?, blade18::status()?.logo)?;
        let charge = blade18::charge::active()?;
        let snapshot = a.output.with_extension("recovery.json");
        let mut recovery = File::options().create_new(true).write(true).open(&snapshot)?;
        serde_json::to_writer_pretty(
            &mut recovery,
            &serde_json::json!({"model":"RZ09-0529","preset":original,"charge":charge}),
        )?;
        recovery.sync_all()?;
        drop(recovery);
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command
            .args(["--recover-after", "60", "--recover-file"])
            .arg(&snapshot)
            .arg("--output")
            .arg(a.output.with_extension("guard.json"));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut guard = command.spawn()?;
        for _ in 0..50 {
            ensure!(guard.try_wait()?.is_none(), "Recovery worker stopped");
            if snapshot.with_extension("ready").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        ensure!(snapshot.with_extension("ready").exists(), "Recovery worker not ready");
        let start = std::time::Instant::now();
        let mut cases = vec![];
        let test = (|| {
            for (i, style) in [
                Style::Static,
                Style::Breathing,
                Style::DualBreathing,
                Style::WaveLeft,
                Style::WaveRight,
                Style::Reactive,
                Style::PerKey,
            ]
            .into_iter()
            .enumerate()
            {
                ensure!(
                    start.elapsed().as_secs() < 45 && guard.try_wait()?.is_none(),
                    "Recovery deadline reached"
                );
                let mut preset = original.clone();
                preset.style = style;
                preset.second = [255, 64, 32];
                for (k, color) in preset.keys.iter_mut().enumerate() {
                    *color = if (k / 19) % 2 == 0 { [32, 64, 255] } else { [255, 64, 32] };
                }
                let result = preset.apply(&a.output.with_extension(format!("case{i}.json")));
                std::thread::sleep(std::time::Duration::from_secs(1));
                ensure!(
                    start.elapsed().as_secs() < 50 && guard.try_wait()?.is_none(),
                    "Recovery deadline reached"
                );
                let restore = original.apply(&a.output.with_extension(format!("restore{i}.json")));
                cases.push(serde_json::json!({"style":style,"passed":result.is_ok(),"error":result.err().map(|e|format!("{e:#}")),"restored":restore.is_ok()}));
                restore?;
            }
            let enabled =
                blade18::charge::set(!charge, &a.output.with_extension("charge-test.json"));
            let restored =
                blade18::charge::set(charge, &a.output.with_extension("charge-restore.json"));
            cases.push(serde_json::json!({"charge_roundtrip":enabled.is_ok() && restored.is_ok(),"error":enabled.as_ref().err().map(|e|format!("{e:#}"))}));
            restored?;
            enabled?;
            Ok::<_, anyhow::Error>(())
        })();
        let restored = original.apply(&a.output.with_extension("final-restore.json"));
        let charge_restored =
            blade18::charge::set(charge, &a.output.with_extension("final-charge.json"));
        writeln!(
            file,
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"cases":cases,"restored":restored.is_ok(),"charge_restored":charge_restored.is_ok(),"error":test.as_ref().err().map(|e|format!("{e:#}"))})
            )?
        )?;
        file.sync_all()?;
        restored?;
        charge_restored?;
        File::options()
            .create_new(true)
            .write(true)
            .open(snapshot.with_extension("finished"))?
            .sync_all()?;
        let _ = guard.wait();
        test?;
    } else {
        writeln!(
            file,
            "{}",
            serde_json::json!({"lighting":blade18::lighting::status()?,"charge_once":blade18::charge::active()?})
        )?;
    }
    file.sync_all()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_and_mutations_require_separate_journals() {
        assert!(Args::try_parse_from(["features", "--validate-lighting"]).is_err());
        assert!(Args::try_parse_from([
            "features",
            "--output",
            "report.json",
            "--validate-lighting",
            "--charge",
            "true"
        ])
        .is_err());
        assert!(Args::try_parse_from([
            "features",
            "--output",
            "report.json",
            "--preset",
            "preset.json",
            "--charge",
            "true"
        ])
        .is_err());
        assert!(Args::try_parse_from([
            "features",
            "--output",
            "report.json",
            "--recover-file",
            "recovery.json"
        ])
        .is_err());
        assert!(Args::try_parse_from([
            "features",
            "--output",
            "report.json",
            "--recover-after",
            "61",
            "--recover-file",
            "recovery.json"
        ])
        .is_err());
    }
}
