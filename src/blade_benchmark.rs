//! Explicit finite GUI measurement, with no GPU queries or hardware setters.
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
pub struct Config {
    pub hidden: bool,
    seconds: u64,
    file: std::fs::File,
}
pub fn requested() -> bool {
    std::env::args().nth(1).as_deref() == Some("--idle-benchmark")
}
pub fn config() -> Result<Option<Config>> {
    if !requested() {
        return Ok(None);
    }
    let args = std::env::args().collect::<Vec<_>>();
    ensure!(args.len() == 5, "Usage: --idle-benchmark hidden|visible seconds output");
    ensure!(matches!(args[2].as_str(), "hidden" | "visible"), "Choose hidden or visible");
    let seconds = args[3].parse()?;
    ensure!((30..=1200).contains(&seconds), "Measurement must be 30–1200 seconds");
    ensure!(
        librazer::thermal::sample()?.modes.iter().all(|mode| mode[1] == 0),
        "Use automatic cooling before an idle benchmark"
    );
    let file =
        std::fs::File::options().create_new(true).write(true).open(PathBuf::from(&args[4]))?;
    Ok(Some(Config { hidden: args[2] == "hidden", seconds, file }))
}
#[cfg(windows)]
fn counters() -> Result<(u64, u64, u64)> {
    use windows::Win32::{Foundation::FILETIME, System::Threading::*};
    let (mut created, mut exited, mut kernel, mut user) =
        (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    let mut io = IO_COUNTERS::default();
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user)?;
        GetProcessIoCounters(GetCurrentProcess(), &mut io)?;
    }
    let ticks = |v: FILETIME| (u64::from(v.dwHighDateTime) << 32) | u64::from(v.dwLowDateTime);
    Ok((
        ticks(kernel) + ticks(user),
        io.ReadOperationCount + io.WriteOperationCount + io.OtherOperationCount,
        io.ReadTransferCount + io.WriteTransferCount + io.OtherTransferCount,
    ))
}
#[cfg(not(windows))]
fn counters() -> Result<(u64, u64, u64)> {
    anyhow::bail!("Windows required")
}
pub fn start(config: Config, ctx: eframe::egui::Context, hwnd: isize) -> Arc<AtomicBool> {
    let exit = Arc::new(AtomicBool::new(false));
    let done = exit.clone();
    std::thread::spawn(move || {
        let measured = (|| -> Result<Value> {
            std::thread::sleep(Duration::from_secs(15));
            let before = counters()?;
            let started = Instant::now();
            std::thread::sleep(Duration::from_secs(config.seconds));
            let after = counters()?;
            let elapsed = started.elapsed().as_secs_f64();
            let cpu = (after.0 - before.0) as f64 / 10_000_000.0;
            Ok(
                json!({"mode":if config.hidden{"hidden"}else{"visible"},"warmup_seconds":15,"elapsed_seconds":elapsed,"cpu_seconds":cpu,"percent_of_one_core":cpu/elapsed*100.0,"io_operations":after.1-before.1,"io_bytes":after.2-before.2,"vendor_takeover_enabled":false,"note":"Finite diagnostic of normal GUI/watchers with vendor takeover skipped. No GPU telemetry, update network checks or battery-energy claim. Compare separately with elevated takeover during the guided batch."}),
            )
        })();
        let report = measured.unwrap_or_else(|e| json!({"error":format!("{e:#}")}));
        let saved = serde_json::to_writer_pretty(&config.file, &report)
            .and_then(|_| config.file.sync_all().map_err(serde_json::Error::io));
        if let Err(e) = saved {
            eprintln!("Idle benchmark report: {e}");
        }
        done.store(true, Ordering::SeqCst);
        crate::blade_desktop::wake(&ctx, hwnd);
    });
    exit
}
