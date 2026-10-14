use crate::blade_profiles::{self, Settings};
use librazer::blade18::{self, State};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;

pub struct Update {
    pub ac: Option<bool>,
    pub result: Option<Result<State, String>>,
}
pub struct Watcher {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Watcher {
    pub fn new(
        settings: Arc<Mutex<Settings>>,
        ctx: eframe::egui::Context,
    ) -> (Self, mpsc::Receiver<Update>) {
        let stop = Arc::new(AtomicBool::new(false));
        let running = stop.clone();
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut previous = blade18::ac_connected().ok();
            while !running.load(Ordering::Relaxed) {
                for _ in 0..20 {
                    if running.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                let current = blade18::ac_connected().ok();
                if current == previous {
                    continue;
                }
                let saved = match settings.lock() {
                    Ok(saved) => saved.clone(),
                    Err(_) => return,
                };
                let result = blade_profiles::transition(previous, current, saved.automatic)
                    .and_then(|ac| {
                        let profile = if ac { saved.ac } else { saved.battery };
                        profile.map(|profile| {
                            blade18::apply_profile(&profile).map_err(|error| format!("{error:#}"))
                        })
                    });
                previous = current;
                if sender.send(Update { ac: current, result }).is_err() {
                    return;
                }
                // Hardware work runs independently of redraws, including while hidden.
                ctx.request_repaint();
            }
        });
        (Self { stop, thread: Some(thread) }, receiver)
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}
