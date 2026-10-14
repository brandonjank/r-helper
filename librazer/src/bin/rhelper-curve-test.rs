use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
#[derive(Clone, Copy, ValueEnum)]
enum Fault {
    Missing,
    Stale,
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    output: PathBuf,
    #[arg(long,default_value_t=100,value_parser=clap::value_parser!(u16).range(30..=120))]
    seconds: u16,
    #[arg(long,default_value_t=80,value_parser=clap::value_parser!(u16).range(10..=119))]
    fault_after_seconds: u16,
    #[arg(long, value_enum, default_value = "missing")]
    fault: Fault,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let fault = match args.fault {
        Fault::Missing => librazer::blade18::CurveFault::Missing,
        Fault::Stale => librazer::blade18::CurveFault::Stale,
    };
    let report = librazer::blade18::test_curves(
        &args.output,
        args.seconds,
        args.fault_after_seconds,
        fault,
    )?;
    println!(
        "fallback_verified={} actual_rpm_verified={}",
        report["fallback_verified"], report["actual_rpm_verified"]
    );
    Ok(())
}
