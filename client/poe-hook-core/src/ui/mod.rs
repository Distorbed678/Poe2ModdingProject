//! The overlay's windows: the main debug window with its tabs, the read-only badge and
//! short notices. Each tab keeps its own state in [`App`].

mod log_tab;
mod status_tab;

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use imgui::{Condition, Ui, WindowFlags};

static NOTICES: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());
const NOTICE_TIME: Duration = Duration::from_secs(5);

/// A short message shown at the top of the screen for a few seconds (also logged).
pub fn notice(text: String) {
    log::info!("{text}");
    NOTICES.lock().unwrap().push((text, Instant::now()));
}

pub fn has_notices() -> bool {
    let mut n = NOTICES.lock().unwrap();
    n.retain(|(_, t)| t.elapsed() < NOTICE_TIME);
    !n.is_empty()
}

/// Plain-text summary of the client state (snapshots).
pub fn state_text() -> String {
    status_tab::state_text()
}

pub struct App {
    log: log_tab::LogView,
}

impl App {
    pub fn new() -> Self {
        Self {
            log: log_tab::LogView::new(),
        }
    }

    pub fn draw(&mut self, ui: &Ui, visible: bool) {
        // Logs are pulled every drawn frame so nothing is missed while hidden.
        self.log.pull();
        if crate::host::read_only() {
            badge(ui);
        }
        notices(ui);
        if !visible {
            return;
        }
        let toggle = crate::config().overlay.toggle_key;
        ui.window("poe-hook debug")
            .size([1100.0, 700.0], Condition::FirstUseEver)
            .position([40.0, 80.0], Condition::FirstUseEver)
            .build(|| {
                if let Some(_bar) = ui.tab_bar("tabs") {
                    if let Some(_t) = ui.tab_item("Status") {
                        status_tab::draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Log") {
                        self.log.draw(ui);
                    }
                }
                let _ = toggle;
            });
    }
}

/// Copies text to the system clipboard.
pub fn set_clipboard(text: &str) {
    let c = std::ffi::CString::new(text.replace('\0', "?")).unwrap_or_default();
    unsafe { imgui::sys::igSetClipboardText(c.as_ptr()) };
}

/// Top-left position that centres a one-line window of `text` horizontally.
fn centred(ui: &Ui, text: &str, y: f32) -> [f32; 2] {
    let [w, _] = ui.io().display_size();
    let tw = ui.calc_text_size(text)[0];
    [(w - tw) * 0.5 - 8.0, y]
}

const BADGE: &str = "OFFICIAL: read-only";

fn badge(ui: &Ui) {
    ui.window("##read-only")
        .position(centred(ui, BADGE, 6.0), Condition::Always)
        .flags(
            WindowFlags::NO_DECORATION
                | WindowFlags::NO_INPUTS
                | WindowFlags::ALWAYS_AUTO_RESIZE
                | WindowFlags::NO_SAVED_SETTINGS
                | WindowFlags::NO_FOCUS_ON_APPEARING
                | WindowFlags::NO_NAV,
        )
        .bg_alpha(0.75)
        .build(|| {
            ui.text_colored([1.0, 0.75, 0.2, 1.0], BADGE);
        });
}

fn notices(ui: &Ui) {
    let list: Vec<String> = {
        let mut n = NOTICES.lock().unwrap();
        n.retain(|(_, t)| t.elapsed() < NOTICE_TIME);
        n.iter().map(|(s, _)| s.clone()).collect()
    };
    if list.is_empty() {
        return;
    }
    let longest = list.iter().max_by_key(|s| s.len()).cloned().unwrap_or_default();
    ui.window("##notices")
        .position(centred(ui, &longest, 44.0), Condition::Always)
        .flags(
            WindowFlags::NO_DECORATION
                | WindowFlags::NO_INPUTS
                | WindowFlags::ALWAYS_AUTO_RESIZE
                | WindowFlags::NO_SAVED_SETTINGS
                | WindowFlags::NO_FOCUS_ON_APPEARING
                | WindowFlags::NO_NAV,
        )
        .bg_alpha(0.8)
        .build(|| {
            for s in &list {
                ui.text(s);
            }
        });
}
