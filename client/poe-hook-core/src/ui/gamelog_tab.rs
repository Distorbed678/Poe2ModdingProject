//! Game log tab: this process's lines of the game's own `logs/Client.txt`.

use imgui::{ListClipper, Ui};

pub struct GameLogView {
    filter: String,
    follow: bool,
}

impl GameLogView {
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            follow: true,
        }
    }

    pub fn draw(&mut self, ui: &Ui) {
        ui.set_next_item_width(320.0);
        ui.input_text("Filter##gamelog", &mut self.filter).build();
        ui.same_line();
        ui.checkbox("Follow##gamelog", &mut self.follow);
        let f = self.filter.to_lowercase();
        let lines: Vec<String> = crate::gamelog::lines()
            .into_iter()
            .filter(|l| f.is_empty() || l.to_lowercase().contains(&f))
            .collect();
        ui.text_disabled(format!(
            "{} lines (logs/Client.txt, this process)",
            lines.len()
        ));
        ui.child_window("gamelog_lines")
            .flags(imgui::WindowFlags::HORIZONTAL_SCROLLBAR)
            .build(ui, || {
                let mut clipper = ListClipper::new(lines.len()).begin(ui);
                while clipper.step() {
                    for i in clipper.display_start()..clipper.display_end() {
                        let l = &lines[i as usize];
                        let color = if l.contains("[CRIT") || l.contains("[ERROR") {
                            [1.0, 0.4, 0.4, 1.0]
                        } else if l.contains("[WARN") {
                            [1.0, 0.8, 0.3, 1.0]
                        } else {
                            [0.9, 0.9, 0.9, 1.0]
                        };
                        ui.text_colored(color, l);
                    }
                }
                if self.follow && ui.scroll_y() >= ui.scroll_max_y() - 40.0 {
                    ui.set_scroll_here_y(1.0);
                }
            });
    }
}
