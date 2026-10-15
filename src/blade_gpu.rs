//! Explicit, short GPU clock-offset preview. No voltage control or automatic tuning.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offsets {
    pub gpu: i32,
    pub vram: i32,
}
impl Offsets {
    fn validate(self) -> Result<()> {
        ensure!(
            (-100..=100).contains(&self.gpu) && (-200..=200).contains(&self.vram),
            "Preview is limited to ±100 MHz core and ±200 MHz memory"
        );
        Ok(())
    }
    fn recoverable(self) -> Result<()> {
        ensure!(
            (-1000..=1000).contains(&self.gpu) && (-1000..=3000).contains(&self.vram),
            "Original offsets outside observed driver bounds"
        );
        Ok(())
    }
    fn read() -> Result<Self> {
        let value = crate::blade_platform::isolated_getter("GetGPUValueStr")?;
        let offsets: Self = serde_json::from_value(value["value"].clone())?;
        offsets.recoverable()?;
        Ok(offsets)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    model: String,
    original: Offsets,
    requested: Offsets,
}
fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    let file = std::fs::File::options().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&file, value)?;
    file.sync_all()?;
    Ok(())
}
fn load(path: &Path) -> Result<Recovery> {
    ensure!(std::fs::metadata(path)?.len() < 16384, "GPU recovery too large");
    let record: Recovery = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(record.model == "RZ09-0529", "GPU recovery model mismatch");
    record.original.recoverable()?;
    record.requested.validate()?;
    Ok(record)
}
fn restore(record: &Recovery, path: &Path) -> Result<()> {
    restore_with(record, Offsets::read, |offsets| {
        crate::blade_platform::set_gpu(offsets.gpu, offsets.vram, path).map(|_| ())
    })
}
fn restore_with(
    record: &Recovery,
    mut read: impl FnMut() -> Result<Offsets>,
    mut write: impl FnMut(Offsets) -> Result<()>,
) -> Result<()> {
    // A failed getter must not suppress the attempt to restore the saved offsets.
    if read().is_ok_and(|current| current == record.original) {
        return Ok(());
    }
    write(record.original)?;
    ensure!(read()? == record.original, "GPU offset recovery mismatch");
    Ok(())
}
pub fn administrator() -> bool {
    #[cfg(windows)]
    {
        unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() }
    }
    #[cfg(not(windows))]
    {
        false
    }
}
pub fn preview(requested: Offsets, path: &Path) -> Result<serde_json::Value> {
    ensure!(librazer::diagnostics::is_target_system(), "GPU preview is model scoped");
    requested.validate()?;
    ensure!(administrator(), "Run R-Helper as administrator for NVIDIA clock previews");
    ensure!(librazer::blade18::ac_connected()?, "Connect AC before a GPU clock preview");
    let record = Recovery { model: "RZ09-0529".into(), original: Offsets::read()?, requested };
    save(path, &record)?;
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("--gpu-preview-recover")
        .arg(path)
        .arg(path.with_extension("guard.json"))
        .arg(std::process::id().to_string());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut guard = command.spawn()?;
    let start = Instant::now();
    let armed = (|| -> Result<()> {
        while !path.with_extension("ready").exists() {
            ensure!(guard.try_wait()?.is_none(), "GPU recovery guard exited");
            ensure!(start.elapsed() < Duration::from_secs(5), "GPU recovery not ready");
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    })();
    if let Err(error) = armed {
        save(&path.with_extension("finished"), &true)?;
        return Err(error);
    }
    let applied = (|| -> Result<()> {
        ensure!(guard.try_wait()?.is_none(), "GPU recovery guard exited before write");
        crate::blade_platform::set_gpu(
            requested.gpu,
            requested.vram,
            &path.with_extension("apply.json"),
        )?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            ensure!(guard.try_wait()?.is_none(), "GPU recovery guard exited during preview");
            if !librazer::blade18::ac_connected()? {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Ok(())
    })();
    let restored = restore(&record, &path.with_extension("restore.json"));
    save(
        &path.with_extension("result.json"),
        &serde_json::json!({"applied":applied.is_ok(),"apply_error":applied.as_ref().err().map(|e|format!("{e:#}")),"original_offsets_restored":restored.is_ok(),"restore_error":restored.as_ref().err().map(|e|format!("{e:#}"))}),
    )?;
    if restored.is_ok() {
        save(&path.with_extension("finished"), &true)?;
    }
    restored.context("Original GPU offsets need recovery; independent guard will retry")?;
    applied?;
    Ok(
        serde_json::json!({"gpu_preview":{"requested":requested,"original":record.original,"restored":true,"voltage_control":false}}),
    )
}
pub fn dispatch() -> Option<Result<()>> {
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1)?;
    if mode != "--gpu-preview-recover" && mode != "--gpu-preview-test" {
        return None;
    }
    Some((|| {
        ensure!(librazer::diagnostics::is_target_system(), "Wrong model for GPU preview");
        if mode == "--gpu-preview-test" {
            ensure!(args.len() == 3, "Expected output");
            preview(Offsets { gpu: -15, vram: 0 }, Path::new(&args[2]))?;
            return Ok(());
        }
        ensure!(args.len() == 5, "Expected recovery record, result and owner");
        let path = PathBuf::from(&args[2]);
        let record = load(&path)?;
        #[cfg(windows)]
        let owner = {
            use windows::Win32::{
                Foundation::CloseHandle,
                System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE},
            };
            struct Owner(windows::Win32::Foundation::HANDLE);
            impl Drop for Owner {
                fn drop(&mut self) {
                    unsafe {
                        let _ = CloseHandle(self.0);
                    }
                }
            }
            Owner(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, args[4].parse()?) }?)
        };
        save(&path.with_extension("ready"), &true)?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(45) {
            if path.with_extension("finished").exists() {
                save(Path::new(&args[3]), &serde_json::json!({"restored_by_parent":true}))?;
                return Ok(());
            }
            #[cfg(windows)]
            {
                use windows::Win32::{
                    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
                    System::Threading::WaitForSingleObject,
                };
                let status = unsafe { WaitForSingleObject(owner.0, 100) };
                if status == WAIT_OBJECT_0 {
                    break;
                }
                if status != WAIT_TIMEOUT {
                    break;
                } // Still attempt restoration after a wait error.
            }
            #[cfg(not(windows))]
            std::thread::sleep(Duration::from_millis(100));
        }
        let result = restore(&record, &PathBuf::from(&args[3]).with_extension("restore.json"));
        save(
            Path::new(&args[3]),
            &serde_json::json!({"fallback_restored":result.is_ok(),"error":result.as_ref().err().map(|e|format!("{e:#}"))}),
        )?;
        result
    })())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_attempts_saved_offsets_after_getter_failure_and_verifies_them() {
        let record = Recovery {
            model: "RZ09-0529".into(),
            original: Offsets { gpu: 25, vram: 50 },
            requested: Offsets { gpu: -15, vram: 0 },
        };
        let mut reads = 0;
        let mut writes = vec![];
        restore_with(
            &record,
            || {
                reads += 1;
                if reads == 1 {
                    anyhow::bail!("Transient getter failure");
                }
                Ok(record.original)
            },
            |offsets| {
                writes.push(offsets);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(writes, vec![record.original]);
        assert_eq!(reads, 2);
        assert!(restore_with(&record, || Ok(record.requested), |_| Ok(())).is_err());
        restore_with(&record, || Ok(record.original), |_| panic!("Already restored")).unwrap();
    }
    #[test]
    fn previews_are_bounded_and_original_offsets_are_separate() {
        assert!(Offsets { gpu: 101, vram: 0 }.validate().is_err());
        assert!(Offsets { gpu: 0, vram: 201 }.validate().is_err());
        assert!(Offsets { gpu: -15, vram: 0 }.validate().is_ok());
        assert!(Offsets { gpu: 250, vram: 500 }.recoverable().is_ok());
        assert!(Offsets { gpu: 1001, vram: 0 }.recoverable().is_err());
    }
}
