//! Explicit development diagnostic, never part of the GUI/background path.
use anyhow::{bail, ensure, Context, Result};
use clap::Parser;
use serde_json::json;
use std::{
    future::Future,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{mpsc, Arc},
    task::{Context as TaskContext, Poll, Wake, Waker},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    output: PathBuf,
    /// Ten-second baseline and twenty-second recovery are additional.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=60))]
    load_seconds: u32,
    /// Work per bounded submission. Start at 1; 64 provides a stronger response test.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=64))]
    dispatches_per_batch: u32,
}

struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
fn wait_future<T>(future: impl Future<Output = T>) -> Result<T> {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = TaskContext::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return Ok(value);
        }
        ensure!(Instant::now() < deadline, "GPU initialization timeout");
        std::thread::park_timeout(Duration::from_millis(10));
    }
}

fn nvidia() -> Result<serde_json::Value> {
    // Use the installed system utility, not a PATH-resolved executable.
    let exe = PathBuf::from(std::env::var_os("WINDIR").context("Missing Windows directory")?)
        .join("System32/nvidia-smi.exe");
    let mut child = Command::new(exe)
        .args([
            "--query-gpu=name,temperature.gpu,utilization.gpu,power.draw",
            "--format=csv,noheader,nounits",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("NVIDIA telemetry timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output()?;
    ensure!(output.status.success(), "NVIDIA telemetry failed");
    let text = String::from_utf8(output.stdout)?;
    parse_nvidia(&text)
}

fn parse_nvidia(text: &str) -> Result<serde_json::Value> {
    let matching: Vec<_> = text
        .lines()
        .filter(|line| line.split(',').next().is_some_and(|name| name.contains("5090")))
        .collect();
    ensure!(matching.len() == 1, "Expected one RTX 5090 NVIDIA reference");
    let fields: Vec<_> = matching[0].split(',').map(str::trim).collect();
    ensure!(fields.len() == 4, "Unexpected NVIDIA CSV shape");
    let utilization = fields[2].parse::<u8>()?;
    ensure!(utilization <= 100, "Invalid NVIDIA utilization");
    Ok(
        json!({"name":fields[0], "celsius":fields[1].parse::<u8>()?, "utilization_percent":utilization, "power_watts":fields[3].parse::<f32>().ok().filter(|v|v.is_finite() && *v >= 0.0)}),
    )
}

fn capture(file: &mut std::fs::File, start: Instant, phase: &str, batches: u64) -> Result<()> {
    let thermal = librazer::thermal::sample()?;
    let thermal_captured = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let reference_start = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let nv = nvidia()?;
    let row = json!({"utc_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(), "thermal_captured_unix_ms":thermal_captured, "nvidia_query_start_unix_ms":reference_start, "elapsed_seconds":start.elapsed().as_secs_f64(), "phase":phase, "completed_batches":batches, "sample":thermal, "nvidia":nv});
    writeln!(file, "{row}")?;
    file.sync_data()?;
    eprintln!("{row}");
    ensure!(librazer::blade18::ac_connected()?, "AC disconnected; stopping GPU work");
    ensure!(thermal.modes.iter().all(|mode| mode[1] == 0), "Fan mode changed; stopping GPU work");
    ensure!(
        thermal.ec_celsius[0] > 0
            && thermal.ec_celsius[0] < 90
            && thermal.ec_celsius[1] > 0
            && thermal.ec_celsius[1] < 78,
        "EC temperature unavailable or cutoff reached"
    );
    ensure!(
        nv["celsius"].as_u64().is_some_and(|v| v > 0 && v < 78),
        "NVIDIA temperature unavailable or cutoff reached"
    );
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut file = std::fs::File::options().write(true).create_new(true).open(args.output)?;
    writeln!(
        file,
        "{}",
        json!({"phase":"configuration", "load_seconds":args.load_seconds, "dispatches_per_batch":args.dispatches_per_batch, "minimum_batch_period_ms":10})
    )?;
    file.sync_data()?;
    let start = Instant::now();
    capture(&mut file, start, "preflight", 0)?;
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let mut adapters: Vec<_> = instance
        .enumerate_adapters(wgpu::Backends::DX12)
        .into_iter()
        .filter(|a| {
            let info = a.get_info();
            info.vendor == 0x10de
                && info.device_type == wgpu::DeviceType::DiscreteGpu
                && info.name.contains("5090")
        })
        .collect();
    ensure!(adapters.len() == 1, "Expected exactly one discrete NVIDIA RTX 5090 DX12 adapter");
    let adapter = adapters.remove(0);
    eprintln!("Selected GPU: {}", adapter.get_info().name);
    let (device, queue) = wait_future(adapter.request_device(&wgpu::DeviceDescriptor::default()))??;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("bounded diagnostic compute"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var<storage, read_write> values: array<u32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    var x = values[id.x] + id.x + 1u;
    for (var i = 0u; i < 1024u; i += 1u) {
        x = (x ^ (x >> 13u)) * 1664525u + 1013904223u;
    }
    values[id.x] = x;
}
"#
            .into(),
        ),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 65536 * 4,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
    });
    for _ in 0..10 {
        capture(&mut file, start, "baseline", 0)?;
        std::thread::sleep(Duration::from_secs(1));
    }
    let load_end = Instant::now() + Duration::from_secs(args.load_seconds.into());
    let mut batches = 0;
    let mut next_sample = Instant::now();
    while Instant::now() < load_end {
        if Instant::now() >= next_sample {
            capture(&mut file, start, "gpu_load", batches)?;
            next_sample = Instant::now() + Duration::from_secs(1);
        }
        if Instant::now() >= load_end {
            break;
        }
        let batch_start = Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            for _ in 0..args.dispatches_per_batch {
                pass.dispatch_workgroups(1024, 1, 1);
            }
        }
        queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        queue.on_submitted_work_done(move || {
            let _ = tx.send(());
        });
        // Only one small batch may be outstanding; never enqueue an unbounded workload.
        let wait_end = Instant::now() + Duration::from_secs(2);
        loop {
            device.poll(wgpu::PollType::Poll)?;
            if rx.try_recv().is_ok() {
                break;
            }
            ensure!(Instant::now() < wait_end, "GPU batch timeout; no further work submitted");
            std::thread::sleep(Duration::from_millis(1));
        }
        batches += 1;
        // Gentle initial test: at most one dispatch per 10 milliseconds.
        if let Some(rest) = Duration::from_millis(10).checked_sub(batch_start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    // No commands or cooling setters are submitted during recovery.
    for _ in 0..20 {
        capture(&mut file, start, "recovery", batches)?;
        std::thread::sleep(Duration::from_secs(1));
    }
    device.destroy();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_rejects_missing_ambiguous_or_unavailable_gpu() {
        for text in [
            "RTX 5080, 40, 0, 10",
            "RTX 5090, N/A, 0, 10",
            "RTX 5090, 40, 101, 10",
            "RTX 5090, 40, 0, 10\nRTX 5090, 41, 0, 12",
        ] {
            assert!(parse_nvidia(text).is_err());
        }
        let value = parse_nvidia("NVIDIA GeForce RTX 5090 Laptop GPU, 45, 30, N/A").unwrap();
        assert_eq!(value["celsius"], 45);
        assert!(value["power_watts"].is_null());
    }
    #[test]
    fn duration_is_bounded_and_output_required() {
        for args in [
            vec!["gpu"],
            vec!["gpu", "--output", "new.json", "--load-seconds", "0"],
            vec!["gpu", "--output", "new.json", "--load-seconds", "61"],
            vec!["gpu", "--output", "new.json", "--dispatches-per-batch", "65"],
        ] {
            assert!(Args::try_parse_from(args).is_err());
        }
    }
}
