//! The overlay's windows: the main debug window with its tabs, the read-only badge and
//! short notices. Each tab keeps its own state in [`App`].

mod commands_tab;
mod coverage_tab;
mod features_tab;
mod game_tab;
mod gamelog_tab;
mod hooks_tab;
mod inspect_tab;
mod instances_tab;
mod layout_form;
mod log_tab;
mod minimap_tab;
mod notes_tab;
mod packets_tab;
mod protocol_tab;
mod scanner_tab;
mod server_tab;
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

/// Whether features that talk to our server are shown: never while the read-only latch is on
/// (an official server), so nothing in the overlay can send to it.
pub fn custom_server() -> bool {
    !crate::host::read_only()
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
    server: server_tab::ServerView,
    commands: commands_tab::CommandsView,
    instances: instances_tab::InstancesView,
    page: Page,
    page_search: String,
    palette_query: String,
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
            server: server_tab::ServerView::new(),
            commands: commands_tab::CommandsView::new(),
            instances: instances_tab::InstancesView::new(),
            page: load_page(),
            page_search: String::new(),
            palette_query: String::new(),
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
        ui.window("poe-hook debug##main2")
            .size([1500.0, 780.0], Condition::FirstUseEver)
            .position([40.0, 80.0], Condition::FirstUseEver)
            .build(|| {
                self.palette(ui);
                let avail = ui.content_region_avail();
                ui.child_window("nav").size([190.0, avail[1]]).border(true).build(ui, || {
                    self.nav(ui);
                });
                ui.same_line();
                ui.child_window("page").size([0.0, avail[1]]).build(ui, || {
                    self.draw_page(ui, plots);
                });
            });
        let _ = toggle;
    }

    /// The sidebar: groups and their pages (our-server-only ones hidden while read-only),
    /// filtered by the search box.
    fn nav(&mut self, ui: &Ui) {
        ui.set_next_item_width(-1.0);
        ui.input_text("##page_search", &mut self.page_search).hint("search pages").build();
        ui.text_disabled("Ctrl+P: palette");
        let f = self.page_search.to_lowercase();
        for (group, custom, pages) in GROUPS {
            if *custom && !custom_server() {
                continue;
            }
            let shown: Vec<&(Page, &str)> = pages.iter().filter(|(_, n)| f.is_empty() || n.to_lowercase().contains(&f)).collect();
            if shown.is_empty() {
                continue;
            }
            ui.spacing();
            ui.text_colored([0.6, 0.75, 1.0, 1.0], group);
            for (page, name) in shown {
                if ui.selectable_config(format!("  {name}")).selected(self.page == *page).build() {
                    self.go(*page);
                }
            }
        }
    }

    fn go(&mut self, page: Page) {
        self.page = page;
        save_page(page);
    }

    fn draw_page(&mut self, ui: &Ui, plots: &dear_implot::PlotContext) {
        // A page of our server only, while connected to an official one: back home.
        if page_is_custom(self.page) && !custom_server() {
            self.page = Page::Status;
        }
        match self.page {
            Page::Status => status_tab::draw(ui),
            Page::Log => self.log.draw(ui),
            Page::GameLog => self.gamelog.draw(ui),
            Page::Instances => self.instances.draw(ui),
            Page::Commands => self.commands.draw(ui),
            Page::Server => self.server.draw_server(ui),
            Page::Desync => self.server.draw_desync(ui, plots),
            Page::Packets => self.packets.draw(ui),
            Page::Protocol => self.protocol.draw(ui),
            Page::Coverage => self.coverage.draw(ui),
            Page::Notes => notes_tab::draw(ui),
            Page::Timing => self.timing.draw(ui, plots),
            Page::Player => inspect_tab::draw_player(ui),
            Page::Stats => self.stats.draw(ui),
            Page::Actions => game_tab::draw_actions(ui),
            Page::Entities => self.entities.draw(ui),
            Page::Watches => self.watches.draw(ui),
            Page::Minimap => self.minimap.draw(ui),
            Page::Scanner => self.scanner.draw(ui),
            Page::Hooks => self.hooks.draw(ui),
            Page::Features => features_tab::draw(ui),
        }
    }

    /// Ctrl+P: jump to a page, or run a server command line (`transfer G1_6 ...`).
    fn palette(&mut self, ui: &Ui) {
        if ui.io().key_ctrl() && ui.is_key_pressed(imgui::Key::P) {
            self.palette_query.clear();
            ui.open_popup("##palette");
        }
        let Some(_p) = ui.begin_popup("##palette") else { return };
        if ui.is_window_appearing() {
            let [w, _] = ui.io().display_size();
            ui.set_window_pos([w * 0.5 - 300.0, 120.0]);
            ui.set_window_size([600.0, 420.0]);
            ui.set_keyboard_focus_here();
        }
        ui.set_next_item_width(-1.0);
        let enter = ui
            .input_text("##palette_query", &mut self.palette_query)
            .hint("page name, or a server command line")
            .enter_returns_true(true)
            .build();
        let q = self.palette_query.trim().to_lowercase();
        let first_word = q.split_whitespace().next().unwrap_or("").to_string();
        let mut chosen: Option<PaletteItem> = None;
        let mut first: Option<PaletteItem> = None;
        for (_, custom, pages) in GROUPS {
            if *custom && !custom_server() {
                continue;
            }
            for (page, name) in pages.iter() {
                if !q.is_empty() && !name.to_lowercase().contains(&q) {
                    continue;
                }
                first.get_or_insert(PaletteItem::Page(*page));
                if ui.selectable(format!("Page: {name}")) {
                    chosen = Some(PaletteItem::Page(*page));
                }
            }
        }
        if custom_server() {
            let (cmds, me) = crate::devlink::registry();
            let admin = me.is_some_and(|m| m.admin);
            for c in cmds.iter().filter(|c| admin || !c.admin) {
                if !q.is_empty() && !c.name.starts_with(&first_word) && !c.name.contains(&q) {
                    continue;
                }
                let line = if q.starts_with(&c.name) { self.palette_query.trim().to_string() } else { c.name.clone() };
                first.get_or_insert(PaletteItem::Command(line.clone(), c.args.iter().any(|a| !a.optional)));
                let usage: Vec<String> = c.args.iter().map(|a| if a.optional { format!("[{}]", a.name) } else { format!("<{}>", a.name) }).collect();
                if ui.selectable(format!("Run: {} {}  - {}##cmd{}", c.name, usage.join(" "), c.help, c.name)) {
                    chosen = Some(PaletteItem::Command(line, false));
                }
            }
        }
        if enter {
            chosen = first;
        }
        if ui.is_key_pressed(imgui::Key::Escape) {
            ui.close_current_popup();
            return;
        }
        match chosen {
            Some(PaletteItem::Page(p)) => {
                self.go(p);
                ui.close_current_popup();
            }
            // A bare command that needs arguments: open its form instead of running it.
            Some(PaletteItem::Command(line, needs_args)) if needs_args && !line.contains(' ') => {
                self.go(Page::Commands);
                ui.close_current_popup();
            }
            Some(PaletteItem::Command(line, _)) => {
                crate::devlink::command(&line);
                ui.close_current_popup();
            }
            None => {}
        }
    }
}

enum PaletteItem {
    Page(Page),
    /// The command line to run, and whether it still needs arguments.
    Command(String, bool),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Status,
    Log,
    GameLog,
    Instances,
    Commands,
    Server,
    Desync,
    Packets,
    Protocol,
    Coverage,
    Notes,
    Timing,
    Player,
    Stats,
    Actions,
    Entities,
    Watches,
    Minimap,
    Scanner,
    Hooks,
    Features,
}

/// Sidebar groups: name, our server only, pages.
const GROUPS: &[(&str, bool, &[(Page, &str)])] = &[
    ("HOME", false, &[(Page::Status, "Status"), (Page::Log, "Log"), (Page::GameLog, "Game log")]),
    ("SERVER", true, &[(Page::Instances, "Instances"), (Page::Commands, "Commands"), (Page::Server, "Sync state"), (Page::Desync, "Desync")]),
    ("NETWORK", false, &[(Page::Packets, "Packets"), (Page::Protocol, "Protocol"), (Page::Coverage, "Coverage"), (Page::Notes, "Notes"), (Page::Timing, "Timing")]),
    ("GAME", false, &[(Page::Player, "Player"), (Page::Stats, "Stats"), (Page::Actions, "Actions"), (Page::Entities, "Entities"), (Page::Watches, "Watches"), (Page::Minimap, "Minimap")]),
    ("RE TOOLS", false, &[(Page::Scanner, "Scanner"), (Page::Hooks, "Hooks")]),
    ("SETTINGS", false, &[(Page::Features, "Features")]),
];

fn page_is_custom(page: Page) -> bool {
    GROUPS.iter().any(|(_, custom, pages)| *custom && pages.iter().any(|(p, _)| *p == page))
}

fn page_path() -> std::path::PathBuf {
    poe_hook_shared::exe_dir().join("logs").join("poe-hook").join("overlay_page.txt")
}

fn save_page(page: Page) {
    let _ = std::fs::write(page_path(), format!("{page:?}"));
}

/// The page selected last time (kept across sessions).
fn load_page() -> Page {
    let name = std::fs::read_to_string(page_path()).unwrap_or_default();
    GROUPS
        .iter()
        .flat_map(|(_, _, pages)| pages.iter())
        .map(|(p, _)| *p)
        .find(|p| format!("{p:?}") == name.trim())
        .unwrap_or(Page::Status)
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
