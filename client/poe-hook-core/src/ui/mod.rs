//! The overlay's windows: the main debug window with its tabs, the read-only badge and
//! short notices. Each tab keeps its own state in [`App`].

mod coverage_tab;
mod features_tab;
mod game_tab;
mod gamelog_tab;
mod hooks_tab;
mod inspect_tab;
mod log_tab;
mod minimap_tab;
mod packets_tab;
mod protocol_tab;
mod scanner_tab;
mod status_tab;
mod timing_tab;

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
    packets: packets_tab::PacketsView,
    coverage: coverage_tab::CoverageView,
    timing: timing_tab::TimingView,
    gamelog: gamelog_tab::GameLogView,
    entities: inspect_tab::EntitiesView,
    watches: inspect_tab::WatchesView,
    hooks: hooks_tab::HooksView,
    protocol: protocol_tab::ProtocolView,
    scanner: scanner_tab::ScannerView,
    stats: game_tab::StatsView,
    minimap: minimap_tab::MinimapView,
}

impl App {
    pub fn new() -> Self {
        Self {
            log: log_tab::LogView::new(),
            packets: packets_tab::PacketsView::new(),
            coverage: coverage_tab::CoverageView::new(),
            timing: timing_tab::TimingView::new(),
            gamelog: gamelog_tab::GameLogView::new(),
            entities: inspect_tab::EntitiesView::new(),
            watches: inspect_tab::WatchesView::new(),
            hooks: hooks_tab::HooksView::new(),
            protocol: protocol_tab::ProtocolView::new(),
            scanner: scanner_tab::ScannerView::new(),
            stats: game_tab::StatsView::new(),
            minimap: minimap_tab::MinimapView::new(),
        }
    }

    pub fn draw(&mut self, ui: &Ui, visible: bool, plots: &dear_implot::PlotContext) {
        // Logs are pulled every drawn frame so nothing is missed while hidden.
        self.log.pull();
        self.scanner.update();
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
                    if let Some(_t) = ui.tab_item("Packets") {
                        self.packets.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Protocol") {
                        self.protocol.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Coverage") {
                        self.coverage.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Timing") {
                        self.timing.draw(ui, plots);
                    }
                    if let Some(_t) = ui.tab_item("Game log") {
                        self.gamelog.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Player") {
                        inspect_tab::draw_player(ui);
                    }
                    if let Some(_t) = ui.tab_item("Minimap") {
                        self.minimap.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Stats") {
                        self.stats.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Actions") {
                        game_tab::draw_actions(ui);
                    }
                    if let Some(_t) = ui.tab_item("Entities") {
                        self.entities.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Watches") {
                        self.watches.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Scanner") {
                        self.scanner.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Hooks") {
                        self.hooks.draw(ui);
                    }
                    if let Some(_t) = ui.tab_item("Features") {
                        features_tab::draw(ui);
                    }
                }
            });
        let _ = toggle;
    }
}

/// Table options for the overlay's tables: columns fit their contents, resizable, striped.
pub fn table_opts() -> imgui::TableOptions {
    imgui::TableOptions::from(imgui::TableFlags::RESIZABLE | imgui::TableFlags::ROW_BG)
        .sizing_policy(imgui::TableSizingPolicy::FixedFit)
}

/// Shows a memory region in the Scanner tab.
pub fn scanner_request(expr: String, len: usize) {
    scanner_tab::request(expr, len);
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
    let longest = list
        .iter()
        .max_by_key(|s| s.len())
        .cloned()
        .unwrap_or_default();
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
