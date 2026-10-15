//! Stats and Actions tabs: the player's stats by name (live, changed values highlighted) and
//! the trace of action starts and ends.

use std::{collections::HashMap, time::Instant};

use imgui::{ListClipper, Ui};

pub struct StatsView {
    filter: String,
    /// (set, key) → (value, when it last changed).
    seen: HashMap<(u32, u32), (i32, Instant)>,
}

impl StatsView {
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            seen: HashMap::new(),
        }
    }

    pub fn draw(&mut self, ui: &Ui) {
        ui.set_next_item_width(320.0);
        ui.input_text("Filter (stat id or key)", &mut self.filter)
            .build();
        let sets = crate::inspect::player_stats();
        if sets.is_empty() {
            ui.text_disabled("No player stats yet.");
            return;
        }
        let f = self.filter.to_lowercase();
        let now = Instant::now();
        let mut rows = Vec::new();
        for (set, pairs) in &sets {
            for &(key, value) in pairs {
                let name = crate::gamedata::stat_name(key).unwrap_or("?");
                let changed = match self.seen.get(&(*set, key)) {
                    Some((v, t)) if *v == value => *t,
                    Some(_) => now,
                    None => now - std::time::Duration::from_secs(3600),
                };
                self.seen.insert((*set, key), (value, changed));
                if f.is_empty() || name.contains(&f) || key.to_string() == f {
                    rows.push((*set, key, name, value, changed));
                }
            }
        }
        ui.text_disabled(format!(
            "{} sets, {} stats shown (set 0 = the player's own stats; names from Stats.dat)",
            sets.len(),
            rows.len()
        ));
        if let Some(_t) = ui.begin_table_header_with_flags(
            "stats",
            [
                imgui::TableColumnSetup::new("set"),
                imgui::TableColumnSetup::new("key"),
                imgui::TableColumnSetup::new("stat"),
                imgui::TableColumnSetup::new("value"),
            ],
            crate::ui::table_opts(),
        ) {
            let mut clipper = ListClipper::new(rows.len()).begin(ui);
            while clipper.step() {
                for i in clipper.display_start()..clipper.display_end() {
                    let (set, key, name, value, changed) = rows[i as usize];
                    let recent = now.duration_since(changed).as_secs_f32() < 3.0;
                    ui.table_next_column();
                    ui.text(set.to_string());
                    ui.table_next_column();
                    ui.text(key.to_string());
                    ui.table_next_column();
                    ui.text(name);
                    ui.table_next_column();
                    if recent {
                        ui.text_colored([1.0, 0.5, 0.3, 1.0], value.to_string());
                    } else {
                        ui.text(value.to_string());
                    }
                }
            }
        }
    }
}

pub fn draw_actions(ui: &Ui) {
    if ui.button("Clear") {
        crate::actions::clear();
    }
    ui.same_line();
    ui.text_disabled(
        "The player's Actor actions (slot 0 = +0x238, 1 = +0x240), names from ActionTypes.dat",
    );
    let events = crate::actions::events();
    ui.child_window("actions_list").build(ui, || {
        let mut clipper = ListClipper::new(events.len()).begin(ui);
        while clipper.step() {
            for i in clipper.display_start()..clipper.display_end() {
                let e = &events[i as usize];
                let line = format!(
                    "{} frame {:6} slot {} {:5} {:24} key {:5} action {:#x}",
                    crate::util::clock(e.unix_ms),
                    e.frame,
                    e.slot,
                    if e.started { "start" } else { "end" },
                    e.name(),
                    e.kind,
                    e.action
                );
                if e.started {
                    ui.text(line);
                } else {
                    ui.text_disabled(line);
                }
            }
        }
        if ui.scroll_y() >= ui.scroll_max_y() - 40.0 {
            ui.set_scroll_here_y(1.0);
        }
    });
}
