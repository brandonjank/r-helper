#![windows_subsystem = "windows"]

mod blade_app;
mod blade_auxiliary;
mod blade_batch_test;
mod blade_benchmark;
mod blade_curves;
mod blade_desktop;
mod blade_display;
mod blade_gpu;
mod blade_platform;
mod blade_policy;
mod blade_power;
mod blade_profiles;
mod blade_sleep;
mod blade_updates;
mod blade_vendor;
mod cli;
mod device;
mod legacy_app;
mod messaging;
mod power;
mod profile_activity;
mod system;
mod ui;
mod utils;

use eframe::egui::IconData;

// Dynamic app metadata from Cargo
const APP_NAME: &str = "R-Helper";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn load_icon() -> IconData {
    const ICON_DATA: &[u8] = include_bytes!("../assets/r-helper-icon.ico");

    if let Ok(image) = image::load_from_memory(ICON_DATA) {
        let image = image.resize_exact(32, 32, image::imageops::FilterType::Lanczos3).to_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();

        IconData { rgba, width, height }
    } else {
        let size = 32;
        let mut rgba = vec![0u8; (size * size * 4) as usize];

        for i in 0..(size * size) as usize {
            let base = i * 4;
            rgba[base] = 0;
            rgba[base + 1] = 150;
            rgba[base + 2] = 255;
            rgba[base + 3] = 255;
        }

        IconData { rgba, width: size, height: size }
    }
}

#[cfg(windows)]
fn set_windows_app_id() {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    // Build a per-version AppUserModelID so taskbar grouping updates with releases
    let app_id =
        format!("RHelper.Application.{}\0", APP_VERSION).encode_utf16().collect::<Vec<u16>>();
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(PCWSTR(app_id.as_ptr()));
    }
}

#[cfg(not(windows))]
fn set_windows_app_id() {}

fn main() -> Result<(), eframe::Error> {
    if cli::dispatch() {
        return Ok(());
    }
    if blade_curves::dispatch_cli() {
        return Ok(());
    }
    set_windows_app_id();
    if std::env::args().any(|arg| arg == "--preview-blade18") {
        return blade_app::run_preview();
    }
    if librazer::diagnostics::is_target_system() {
        let _instance = match blade_vendor::UiInstance::acquire() {
            Ok(Some(lease)) => lease,
            Ok(None) => return Ok(()),
            Err(error) => return Err(eframe::Error::AppCreation(error.into())),
        };
        return blade_app::run();
    }
    legacy_app::run()
}
