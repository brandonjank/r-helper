use anyhow::{ensure, Result};
use clap::Parser;
use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=600))]
    seconds: u32,
    /// Bounded CPU load threads; zero keeps the machine at its normal workload.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=4))]
    cpu_threads: u8,
}
fn main() -> Result<()> {
    let args = Args::parse();
    if args.cpu_threads > 0 {
        ensure!(librazer::blade18::ac_connected()?, "Load validation requires AC power");
    }
    let mut file = std::fs::File::options().write(true).create_new(true).open(args.output)?;
    let initial = librazer::thermal::sample()?;
    if args.cpu_threads > 0 {
        ensure!(
            initial.ec_celsius[0] > 0 && initial.ec_celsius[0] < 85,
            "CPU load requires a valid controller temperature below 85 C"
        );
        ensure!(
            initial.modes.iter().all(|mode| mode[1] == 0),
            "Load validation requires automatic fans"
        );
    }
    let start = Instant::now();
    let deadline = start + Duration::from_secs(args.seconds.into());
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut threads = Vec::new();
    for index in 0..args.cpu_threads {
        let stop = stop.clone();
        threads.push(std::thread::spawn(move || {
            let mut value = 0.5 + f64::from(index);
            while Instant::now() < deadline && !stop.load(std::sync::atomic::Ordering::Relaxed) {
                for _ in 0..10000 {
                    value = std::hint::black_box(value.mul_add(1.0000001, 0.1).sin());
                }
            }
            std::hint::black_box(value);
        }));
    }
    let result = (|| -> Result<()> {
        while Instant::now() < deadline {
            let sample = librazer::thermal::sample()?;
            if args.cpu_threads > 0 {
                ensure!(librazer::blade18::ac_connected()?, "AC disconnected; CPU workload stopped");
                ensure!(sample.ec_celsius.iter().all(|t| *t > 0), "Controller temperature unavailable; workload stopped");
                ensure!(
                    sample.modes.iter().all(|mode| mode[1] == 0),
                    "Fan mode changed; workload stopped"
                );
            }
            let row = serde_json::json!({"utc_unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis(), "elapsed_seconds": start.elapsed().as_secs_f64(), "cpu_threads": args.cpu_threads, "sample": sample});
            writeln!(file, "{row}")?;
            file.sync_data()?;
            eprintln!("{row}");
            ensure!(
                sample.ec_celsius[0] < 90 && sample.ec_celsius[1] < 80,
                "EC thermal guard reached; workload stopped"
            );
            std::thread::sleep(Duration::from_secs(1));
        }
        Ok(())
    })();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for thread in threads {
        let _ = thread.join();
    }
    result
}
