use anyhow::{ensure, Context, Result};
use librazer::blade18::CurveSettings;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
pub type Shared = Arc<Mutex<Option<PathBuf>>>;
pub fn shared() -> Shared {
    Arc::new(Mutex::new(None))
}
pub fn config_path() -> Result<PathBuf> {
    Ok(crate::blade_profiles::path()?.with_file_name("curves.json"))
}
pub fn load() -> Result<CurveSettings> {
    let path = config_path()?;
    let config = match fs::read(path) {
        Ok(data) => serde_json::from_slice(&data)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => CurveSettings::default(),
        Err(e) => return Err(e.into()),
    };
    config.validate()?;
    Ok(config)
}
pub fn save(config: &CurveSettings) -> Result<()> {
    config.validate()?;
    let path = config_path()?;
    fs::create_dir_all(path.parent().context("No curve directory")?)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, serde_json::to_vec_pretty(config)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}
fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub fn stop_and_wait(active: &Shared) -> Result<()> {
    let directory =
        active.lock().map_err(|_| anyhow::anyhow!("Curve session lock failed"))?.clone();
    let Some(directory) = directory else {
        return Ok(());
    };
    fs::write(directory.join("stop"), b"stop")?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(result) = read_json(&directory.join("finished.json")) {
            ensure!(
                result["recovery_verified"] == true,
                "Curve recovery was not verified: {result}"
            );
            if let Ok(mut current) = active.lock() {
                if current.as_ref() == Some(&directory) {
                    *current = None;
                }
            }
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Waiting for curve watchdog recovery; do not apply another profile yet"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn command() -> Result<Command> {
    let mut cmd = Command::new(std::env::current_exe()?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    Ok(cmd)
}
pub fn launch(
    config: &CurveSettings,
    active: Shared,
    ctx: eframe::egui::Context,
) -> Result<mpsc::Receiver<Value>> {
    config.validate()?;
    ensure!(
        active.lock().map_err(|_| anyhow::anyhow!("Curve session lock failed"))?.is_none(),
        "A curve session is already active"
    );
    let directory = config_path()?.parent().unwrap().join("curve-sessions").join(format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir_all(&directory)?;
    fs::write(directory.join("config.json"), serde_json::to_vec_pretty(config)?)?;
    let mut supervisor = command()?
        .arg("--curve-supervisor")
        .arg(&directory)
        .arg(std::process::id().to_string())
        .spawn()?;
    *active.lock().map_err(|_| anyhow::anyhow!("Curve session lock failed"))? =
        Some(directory.clone());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            if let Ok(result) = read_json(&directory.join("finished.json")) {
                if let Ok(mut current) = active.lock() {
                    if current.as_ref() == Some(&directory) {
                        *current = None;
                    }
                }
                let _ = sender.send(json!({"finished":result}));
                ctx.request_repaint();
                break;
            }
            if let Ok(status) = read_json(&directory.join("status.json")) {
                let _ = sender.send(status);
                ctx.request_repaint();
            }
            if supervisor.try_wait().ok().flatten().is_some() {
                let _ = fs::write(directory.join("stop"), b"stop");
                let deadline = Instant::now() + Duration::from_secs(7);
                while Instant::now() < deadline
                    && !read_json(&directory.join("status.json"))
                        .is_ok_and(|status| status["phase"] == "stopped")
                {
                    std::thread::sleep(Duration::from_millis(100));
                }
                let recovered = if directory.join("recovery.json").exists() {
                    librazer::blade18::restore_curve_session(&directory).map(|value| json!(value))
                } else {
                    Ok(json!({"no_fan_writes":true}))
                };
                let result = json!({"recovery_verified":recovered.is_ok(),"recovered":recovered.as_ref().ok(),"error":recovered.as_ref().err().map(|e|format!("{e:#}")),"reason":"Watchdog exited unexpectedly; app attempted recovery"});
                let _ = fs::write(directory.join("finished.json"), result.to_string());
                if recovered.is_ok() {
                    if let Ok(mut current) = active.lock() {
                        if current.as_ref() == Some(&directory) {
                            *current = None;
                        }
                    }
                }
                let _ = sender.send(json!({"finished":result}));
                ctx.request_repaint();
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        let _ = supervisor.wait();
    });
    Ok(receiver)
}

struct GuardedWorker {
    child: std::process::Child,
    directory: PathBuf,
    armed: bool,
}
impl std::ops::Deref for GuardedWorker {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}
impl std::ops::DerefMut for GuardedWorker {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}
impl Drop for GuardedWorker {
    fn drop(&mut self) {
        if self.armed {
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
            if self.directory.join("recovery.json").exists() {
                let _ = librazer::blade18::restore_curve_session(&self.directory);
            }
        }
    }
}

#[cfg(windows)]
fn supervisor(directory: &Path, parent_pid: u32) -> Result<()> {
    use windows::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    let parent = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) }?;
    struct Parent(windows::Win32::Foundation::HANDLE);
    impl Drop for Parent {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let parent = Parent(parent);
    fs::write(directory.join("watchdog-heartbeat"), b"alive")?;
    let mut child = GuardedWorker {
        child: command()?.arg("--curve-worker").arg(directory).spawn()?,
        directory: directory.to_path_buf(),
        armed: true,
    };
    fs::write(
        directory.join("ready.json"),
        serde_json::to_vec(&json!({"worker_pid":child.id(),"supervisor_pid":std::process::id()}))?,
    )?;
    let started = Instant::now();
    let mut last_heartbeat = Instant::now();
    let mut stopping = None;
    let mut reason = String::new();
    let mut killed = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if last_heartbeat.elapsed() >= Duration::from_secs(1) {
            fs::write(directory.join("watchdog-heartbeat"), b"alive")?;
            last_heartbeat = Instant::now();
        }
        let parent_gone = unsafe { WaitForSingleObject(parent.0, 0) } == WAIT_OBJECT_0;
        let stale = match fs::metadata(directory.join("status.json")).and_then(|m| m.modified()) {
            Ok(time) => SystemTime::now()
                .duration_since(time)
                .map(|age| age > Duration::from_secs(12))
                .unwrap_or(true),
            Err(_) => started.elapsed() > Duration::from_secs(15),
        };
        if stopping.is_none() && (parent_gone || stale || directory.join("stop").exists()) {
            reason = if parent_gone {
                "App exited"
            } else if stale {
                "Controller heartbeat stopped"
            } else {
                "Stopped by request"
            }
            .into();
            fs::write(directory.join("stop"), b"stop")?;
            stopping = Some(Instant::now());
        }
        if stopping.is_some_and(|time| time.elapsed() > Duration::from_secs(5)) {
            child.kill()?;
            killed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let exit = child.wait()?;
    let recovery = if directory.join("recovery.json").exists() {
        librazer::blade18::restore_curve_session(directory).map(|value| json!(value))
    } else {
        // The worker creates recovery before its first setter.
        Ok(json!({"no_fan_writes":true}))
    };
    child.armed = false;
    let status = read_json(&directory.join("status.json")).ok();
    fs::write(
        directory.join("finished.json"),
        serde_json::to_vec_pretty(
            &json!({"recovery_verified":recovery.is_ok(),"recovered":recovery.as_ref().ok(),"error":recovery.as_ref().err().map(|e|format!("{e:#}")),"worker_error":fs::read_to_string(directory.join("worker-error.txt")).ok(),"worker_exit_success":exit.success(),"killed":killed,"reason":reason,"status":status}),
        )?,
    )?;
    recovery?;
    Ok(())
}
#[cfg(not(windows))]
fn supervisor(_: &Path, _: u32) -> Result<()> {
    anyhow::bail!("Windows required")
}

/// Headless roles run before device detection, tray creation or any GUI state.
pub fn dispatch_cli() -> bool {
    let mut args = std::env::args_os().skip(1);
    let Some(role) = args.next() else {
        return false;
    };
    if role != "--curve-worker" && role != "--curve-supervisor" && role != "--curve-recover" {
        return false;
    }
    let Some(directory) = args.next().map(PathBuf::from) else {
        std::process::exit(1);
    };
    let result = if role == "--curve-recover" {
        librazer::blade18::restore_curve_session(&directory).and_then(|value| {
            fs::write(directory.join("recovered.json"), serde_json::to_vec(&value)?)?;
            Ok(())
        })
    } else if role == "--curve-worker" {
        librazer::blade18::run_curve_session(&directory)
    } else {
        args.next()
            .and_then(|v| v.to_str().and_then(|v| v.parse::<u32>().ok()))
            .context("Missing parent PID")
            .and_then(|pid| supervisor(&directory, pid))
    };
    if let Err(error) = result {
        if role == "--curve-worker" {
            let _ = fs::write(directory.join("worker-error.txt"), format!("{error:#}"));
        } else if role == "--curve-supervisor" {
            let recovered = if directory.join("recovery.json").exists() {
                librazer::blade18::restore_curve_session(&directory).map(|sample| json!(sample))
            } else {
                Ok(json!({"no_fan_writes":true}))
            };
            let _ = fs::write(
                directory.join("finished.json"),
                json!({"recovery_verified":recovered.is_ok(),"recovered":recovered.as_ref().ok(),"worker_error":format!("Watchdog startup/runtime error: {error:#}"),"error":recovered.as_ref().err().map(|e|format!("{e:#}"))}).to_string(),
            );
        } else {
            let _ = fs::write(directory.join("recovery-error.txt"), format!("{error:#}"));
        }
        std::process::exit(1);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_handoff_requires_confirmed_curve_recovery() {
        let active = shared();
        assert!(stop_and_wait(&active).is_ok());
        let directory = std::env::temp_dir().join(format!(
            "rhelper-handoff-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        *active.lock().unwrap() = Some(directory.clone());
        fs::write(directory.join("finished.json"), b"{\"recovery_verified\":false}").unwrap();
        assert!(stop_and_wait(&active).is_err());
        assert!(active.lock().unwrap().is_some());
        fs::write(directory.join("finished.json"), b"{\"recovery_verified\":true}").unwrap();
        assert!(stop_and_wait(&active).is_ok());
        assert!(active.lock().unwrap().is_none());
        // Delete only the individual fixture files created by this test.
        for name in ["finished.json", "stop"] {
            fs::remove_file(directory.join(name)).unwrap();
        }
        fs::remove_dir(directory).unwrap();
    }
}
