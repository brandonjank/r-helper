use anyhow::Result;
use eframe::egui;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIcon, TrayIconBuilder,
};

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Show,
    Ac,
    Battery,
    Cycle,
    Quit,
}
pub struct Desktop {
    _tray: TrayIcon,
    _hotkeys: GlobalHotKeyManager,
    events: std::sync::mpsc::Receiver<Action>,
}
impl Desktop {
    pub fn new(ctx: &egui::Context, hwnd: isize) -> Result<Self> {
        let menu = Menu::new();
        let show = MenuItem::with_id("show", "Show R-Helper", true, None);
        let ac = MenuItem::with_id("ac", "Apply AC profile", true, None);
        let battery = MenuItem::with_id("battery", "Apply battery profile", true, None);
        let quit = MenuItem::with_id("quit", "Exit", true, None);
        menu.append_items(&[&show, &ac, &battery, &quit])?;
        let source =
            image::load_from_memory(include_bytes!("../assets/r-helper-icon.ico"))?.into_rgba8();
        let icon = tray_icon::Icon::from_rgba(source.to_vec(), source.width(), source.height())?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("R-Helper · Blade 18")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()?;
        let manager = GlobalHotKeyManager::new()?;
        let mut bindings = Vec::new();
        for (code, action) in [
            (Code::F12, Action::Show),
            (Code::F5, Action::Cycle),
            (Code::F6, Action::Ac),
            (Code::F7, Action::Battery),
        ] {
            let key =
                HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT), code);
            manager.register(key)?;
            bindings.push((key.id(), action));
        }
        let (sender, events) = std::sync::mpsc::channel();
        let menu_sender = sender.clone();
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.as_ref() {
                "show" => Action::Show,
                "ac" => Action::Ac,
                "battery" => Action::Battery,
                "quit" => Action::Quit,
                _ => return,
            };
            let _ = menu_sender.send(action);
            wake(&menu_ctx, hwnd);
        }));
        let hotkey_ctx = ctx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                if let Some((_, action)) = bindings.iter().find(|(id, _)| *id == event.id) {
                    let _ = sender.send(*action);
                    wake(&hotkey_ctx, hwnd);
                }
            }
        }));
        Ok(Self { _tray: tray, _hotkeys: manager, events })
    }
    pub fn events(&self) -> Vec<Action> {
        self.events.try_iter().collect()
    }
}

pub(crate) fn wake(ctx: &egui::Context, hwnd: isize) {
    // Hidden winit windows do not redraw. Restore our own window before waking egui.
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{ShowWindow, SW_RESTORE},
        };
        let _ = ShowWindow(HWND(hwnd as *mut std::ffi::c_void), SW_RESTORE);
    }
    ctx.request_repaint();
}
