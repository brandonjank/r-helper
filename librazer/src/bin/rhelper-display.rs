use anyhow::Result;
use clap::Parser;
use std::{fs::File, io::Write, path::PathBuf};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    output: PathBuf,
    #[arg(long,conflicts_with_all=["restore","validate","recover_after"])]
    apply_rate: Option<u32>,
    #[arg(long,conflicts_with_all=["apply_rate","validate","recover_after"])]
    restore: Option<PathBuf>,
    #[arg(long,conflicts_with_all=["apply_rate","restore","recover_after"])]
    validate: bool,
    #[arg(long,requires="snapshot",value_parser=clap::value_parser!(u64).range(1..=60))]
    recover_after: Option<u64>,
    #[arg(long, requires = "recover_after")]
    snapshot: Option<PathBuf>,
}
fn save(file: &mut File, value: &serde_json::Value) -> Result<()> {
    serde_json::to_writer_pretty(&mut *file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
fn main() -> Result<()> {
    let a = Args::parse();
    let mut file = File::options().create_new(true).write(true).open(&a.output)?;
    if let Some(seconds) = a.recover_after {
        let snapshot = a.snapshot.unwrap();
        File::options()
            .create_new(true)
            .write(true)
            .open(snapshot.with_extension("ready"))?
            .sync_all()?;
        for _ in 0..seconds * 5 {
            if snapshot.with_extension("finished").exists() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        save(&mut file, &serde_json::to_value(librazer::display::restore(&snapshot)?)?)?;
        return Ok(());
    }
    if let Some(path) = a.restore {
        save(&mut file, &serde_json::to_value(librazer::display::restore(&path)?)?)?;
        return Ok(());
    }
    let before = librazer::display::inventory()?;
    if a.apply_rate.is_none() && !a.validate {
        save(&mut file, &serde_json::to_value(&before)?)?;
        return Ok(());
    }
    let snapshot = a.output.with_extension("recovery.json");
    save(
        &mut File::options().create_new(true).write(true).open(&snapshot)?,
        &serde_json::json!({"model":"RZ09-0529","operation":"internal_panel_refresh","original":before.current}),
    )?;
    if let Some(hz) = a.apply_rate {
        let mut profile = before.current;
        profile.hz = hz;
        save(&mut file, &serde_json::to_value(librazer::display::apply(&profile)?)?)?;
        return Ok(());
    }
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .args(["--recover-after", "30", "--snapshot"])
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
        anyhow::ensure!(
            guard.try_wait()?.is_none(),
            "Display recovery guard exited before becoming ready"
        );
        if snapshot.with_extension("ready").exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::ensure!(
        snapshot.with_extension("ready").exists(),
        "Display recovery guard did not become ready; no mode writes sent"
    );
    let started = std::time::Instant::now();
    let test = (|| {
        let mut cases = Vec::new();
        for hz in before.rates.iter().copied().filter(|hz| *hz != before.current.hz).take(6) {
            anyhow::ensure!(
                started.elapsed().as_secs() < 25 && guard.try_wait()?.is_none(),
                "Display guard deadline reached; aborting test"
            );
            let mut profile = before.current.clone();
            profile.hz = hz;
            let after = librazer::display::apply(&profile)?;
            std::thread::sleep(std::time::Duration::from_secs(2));
            anyhow::ensure!(
                started.elapsed().as_secs() < 25 && guard.try_wait()?.is_none(),
                "Display guard expired; restoring original rate"
            );
            anyhow::ensure!(
                librazer::display::inventory()?.current == profile,
                "Refresh setting did not hold"
            );
            cases.push(after);
            librazer::display::apply(&before.current)?;
        }
        Ok::<_, anyhow::Error>(cases)
    })();
    let restored = librazer::display::apply(&before.current);
    save(
        &mut file,
        &serde_json::json!({"original":before,"cases":test.as_ref().ok(),"error":test.as_ref().err().map(|e|format!("{e:#}")),"restored":restored.is_ok()}),
    )?;
    if restored.is_ok() {
        File::create(snapshot.with_extension("finished"))?.sync_all()?;
        let _ = guard.wait();
    }
    restored?;
    test?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_and_mutation_actions_cannot_mix() {
        assert!(Args::try_parse_from(["display", "--validate"]).is_err());
        assert!(Args::try_parse_from([
            "display",
            "--output",
            "out.json",
            "--validate",
            "--apply-rate",
            "120"
        ])
        .is_err());
        assert!(Args::try_parse_from([
            "display",
            "--output",
            "out.json",
            "--recover-after",
            "0",
            "--snapshot",
            "snapshot.json"
        ])
        .is_err());
        assert!(Args::try_parse_from(["display", "--output", "out.json", "--recover-after", "30"])
            .is_err());
        assert!(Args::try_parse_from([
            "display",
            "--output",
            "out.json",
            "--recover-after",
            "30",
            "--snapshot",
            "snapshot.json"
        ])
        .is_ok());
    }
}
