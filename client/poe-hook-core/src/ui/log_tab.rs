//! Log tab: every record of the shared log sink (loader and core), filterable by level and
//! text. The same records are in `poe-hook.log` (next to the exe and in the session folder).

use imgui::{ListClipper, Ui};

const KEEP: usize = 50_000;

struct Rec {
    level: u32,
    line: String,
}

pub struct LogView {
    records: Vec<Rec>,
    next: u64,
    filter: String,
    levels: [bool; 5],
    follow: bool,
    /// Indices into `records` that pass the filter (rebuilt when it changes).
    shown: Vec<usize>,
    shown_for: (String, [bool; 5]),
    shown_upto: usize,
}

impl LogView {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            next: 0,
            filter: String::new(),
            levels: [true, true, true, true, false],
            follow: true,
            shown: Vec::new(),
            shown_for: (String::new(), [true, true, true, true, false]),
            shown_upto: 0,
        }
    }

    pub fn pull(&mut self) {
        let records = &mut self.records;
        self.next = crate::host::logs_since(self.next, |r| {
            records.push(Rec {
                level: r.level,
                line: format!(
                    "{} {:5} [{}] {}",
                    crate::util::clock(r.unix_ms),
                    crate::util::level_name(r.level),
                    unsafe { r.target.as_str() },
                    unsafe { r.message.as_str() }
                ),
            });
        });
        if self.records.len() > KEEP {
            let drop = self.records.len() - KEEP;
            self.records.drain(..drop);
            self.shown_upto = 0;
            self.shown.clear();
        }
    }

    fn passes(&self, r: &Rec) -> bool {
        let level_ok = self
            .levels
            .get(r.level.saturating_sub(1) as usize)
            .copied()
            .unwrap_or(true);
        level_ok
            && (self.filter.is_empty()
                || r.line.to_lowercase().contains(&self.filter.to_lowercase()))
    }

    fn refresh(&mut self) {
        if self.shown_for != (self.filter.clone(), self.levels) {
            self.shown_for = (self.filter.clone(), self.levels);
            self.shown.clear();
            self.shown_upto = 0;
        }
        for i in self.shown_upto..self.records.len() {
            if self.passes(&self.records[i]) {
                self.shown.push(i);
            }
        }
        self.shown_upto = self.records.len();
    }

    pub fn draw(&mut self, ui: &Ui) {
        ui.set_next_item_width(320.0);
        ui.input_text("Filter", &mut self.filter).build();
        for (i, name) in ["Error", "Warn", "Info", "Debug", "Trace"]
            .iter()
            .enumerate()
        {
            ui.same_line();
            ui.checkbox(name, &mut self.levels[i]);
        }
        ui.same_line();
        ui.checkbox("Follow", &mut self.follow);
        ui.same_line();
        if ui.button("Copy shown") {
            let text: Vec<&str> = self
                .shown
                .iter()
                .map(|&i| self.records[i].line.as_str())
                .collect();
            crate::ui::set_clipboard(&text.join("\n"));
        }
        self.refresh();
        ui.text_disabled(format!(
            "{} of {} records; files: poe-hook.log next to the exe and in {}",
            self.shown.len(),
            self.records.len(),
            crate::host::session_dir().display()
        ));
        ui.child_window("log_lines")
            .flags(imgui::WindowFlags::HORIZONTAL_SCROLLBAR)
            .build(ui, || {
                let mut clipper = ListClipper::new(self.shown.len()).begin(ui);
                while clipper.step() {
                    for row in clipper.display_start()..clipper.display_end() {
                        let r = &self.records[self.shown[row as usize]];
                        let color = match r.level {
                            1 => [1.0, 0.35, 0.35, 1.0],
                            2 => [1.0, 0.8, 0.3, 1.0],
                            4 | 5 => [0.65, 0.65, 0.65, 1.0],
                            _ => [0.92, 0.92, 0.92, 1.0],
                        };
                        ui.text_colored(color, &r.line);
                    }
                }
                if self.follow && ui.scroll_y() >= ui.scroll_max_y() - 40.0 {
                    ui.set_scroll_here_y(1.0);
                }
            });
    }
}
