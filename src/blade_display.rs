use anyhow::{ensure, Result};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
static PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
pub fn is_pending() -> bool {
    if let Ok(active) = ACTIVE.lock() {
        if active.as_ref().is_some_and(|p| p.with_extension("done").exists()) {
            PENDING.store(false, Ordering::SeqCst);
        }
    }
    PENDING.load(Ordering::SeqCst)
}
pub fn wait_for_confirmation() -> Result<()> {
    let start = Instant::now();
    while is_pending() {
        ensure!(start.elapsed() < Duration::from_secs(35), "Finish the display confirmation first");
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
pub fn begin(target: &librazer::display::Profile) -> Result<serde_json::Value> {
    ensure!(!PENDING.swap(true, Ordering::SeqCst), "A display change is awaiting confirmation");
    let result = (|| {
        let original = librazer::display::inventory()?.current;
        let path = crate::blade_profiles::journal_path("display-preview")?;
        *ACTIVE.lock().map_err(|_| anyhow::anyhow!("Display lock unavailable"))? =
            Some(path.clone());
        let mut file = File::options().create_new(true).write(true).open(&path)?;
        serde_json::to_writer_pretty(
            &mut file,
            &serde_json::json!({"model":"RZ09-0529","operation":"internal_panel_refresh","original":original,"requested":target}),
        )?;
        file.sync_all()?;
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.arg("--display-guard").arg(&path);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut guard = command.spawn()?;
        for _ in 0..50 {
            ensure!(guard.try_wait()?.is_none(), "Display guard exited");
            if path.with_extension("ready").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        ensure!(path.with_extension("ready").exists(), "Display guard not ready; no change sent");
        let state = librazer::display::apply(target)?;
        ensure!(
            guard.try_wait()?.is_none() && !path.with_extension("done").exists(),
            "Display recovery deadline reached; previous rate is being restored"
        );
        Ok(serde_json::json!({"display_controls":state,"display_pending":path}))
    })();
    if result.is_err() {
        let active = ACTIVE.lock().ok().and_then(|p| p.clone());
        if let Some(path) = active
            .filter(|p| p.with_extension("ready").exists() && !p.with_extension("done").exists())
        {
            let _ = finish(&path, false);
        }
        PENDING.store(false, Ordering::SeqCst);
    }
    result
}
pub fn finish(path: &Path, keep: bool) -> Result<librazer::display::Info> {
    let result = (|| {
        ensure!(
            !path.with_extension("done").exists(),
            "Display confirmation expired; refresh the panel state"
        );
        let token = if keep { "keep" } else { "revert" };
        File::options()
            .write(true)
            .create_new(true)
            .open(path.with_extension(token))?
            .sync_all()?;
        let start = Instant::now();
        while !path.with_extension("done").exists() {
            ensure!(start.elapsed() < Duration::from_secs(8), "Display recovery is still pending");
            std::thread::sleep(Duration::from_millis(100));
        }
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.with_extension("done"))?)?;
        ensure!(result["ok"] == true, "Display recovery failed: {result}");
        ensure!(
            !keep || result["kept"] == true,
            "Display confirmation expired; original mode restored"
        );
        librazer::display::inventory()
    })();
    PENDING.store(false, Ordering::SeqCst);
    result
}
pub fn dispatch() -> Option<Result<()>> {
    let mut args = std::env::args_os();
    args.next();
    if args.next()?.to_str()? != "--display-guard" {
        return None;
    }
    Some((|| {
        let path =
            PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("Missing display recovery"))?);
        ensure!(args.next().is_none(), "Unexpected display argument");
        guard(&path)
    })())
}
fn guard(path: &Path) -> Result<()> {
    let record: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        record["model"] == "RZ09-0529" && record["operation"] == "internal_panel_refresh",
        "Invalid display recovery"
    );
    let target: librazer::display::Profile = serde_json::from_value(record["requested"].clone())?;
    target.validate()?;
    let original: librazer::display::Profile = serde_json::from_value(record["original"].clone())?;
    original.validate()?;
    File::options().create_new(true).write(true).open(path.with_extension("ready"))?.sync_all()?;
    let start = Instant::now();
    let mut kept = false;
    while start.elapsed() < Duration::from_secs(25) {
        if path.with_extension("revert").exists() {
            break;
        }
        if path.with_extension("keep").exists() {
            kept = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let result = if kept {
        librazer::display::inventory().and_then(|info| {
            ensure!(info.current == target, "Display changed before confirmation");
            Ok(info)
        })
    } else {
        librazer::display::apply(&original)
    };
    let mut file =
        File::options().create_new(true).write(true).open(path.with_extension("done"))?;
    writeln!(
        file,
        "{}",
        serde_json::json!({"kept":kept,"ok":result.is_ok(),"error":result.as_ref().err().map(|e|format!("{e:#}"))})
    )?;
    file.sync_all()?;
    result?;
    Ok(())
}
