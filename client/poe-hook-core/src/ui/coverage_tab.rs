//! Coverage tab: every opcode seen, per channel and direction, with counts on official and on
//! our servers (all sessions, `logs/poe-hook/coverage.tsv`) and this session. "Official only"
//! lists what official servers send that ours never did: the to-do list of later milestones.

use imgui::Ui;

use crate::packets::{Channel, Dir};

pub struct CoverageView {
    official_only: bool,
    unnamed_only: bool,
    filter: String,
}

impl CoverageView {
    pub fn new() -> Self {
        Self {
            official_only: false,
            unnamed_only: false,
            filter: String::new(),
        }
    }

    pub fn draw(&mut self, ui: &Ui) {
        ui.set_next_item_width(220.0);
        ui.input_text("Filter##cov", &mut self.filter).build();
        ui.same_line();
        ui.checkbox("official only (never on ours)", &mut self.official_only);
        ui.same_line();
        ui.checkbox("no name yet", &mut self.unnamed_only);
        let rows: Vec<_> = crate::packets::coverage()
            .into_iter()
            .filter(|((c, d, op), s)| {
                let name = crate::packets::name(*c, *d, *op);
                (!self.official_only || (s.official > 0 && s.private == 0))
                    && (!self.unnamed_only || name.is_none())
                    && (self.filter.is_empty()
                        || format!("{op:#06x} {}", name.unwrap_or(""))
                            .to_lowercase()
                            .contains(&self.filter.to_lowercase()))
            })
            .collect();
        ui.text_disabled(format!(
            "{} opcodes; file: logs/poe-hook/coverage.tsv (merged across sessions)",
            rows.len()
        ));
        if let Some(_t) = ui.begin_table_header_with_flags(
            "coverage",
            [
                imgui::TableColumnSetup::new("channel"),
                imgui::TableColumnSetup::new("dir"),
                imgui::TableColumnSetup::new("opcode"),
                imgui::TableColumnSetup::new("name"),
                imgui::TableColumnSetup::new("official"),
                imgui::TableColumnSetup::new("ours"),
                imgui::TableColumnSetup::new("session"),
                imgui::TableColumnSetup::new("size"),
            ],
            crate::ui::table_opts(),
        ) {
            for ((c, d, op), s) in rows {
                let name = crate::packets::name(c, d, op);
                ui.table_next_column();
                ui.text(c.name());
                ui.table_next_column();
                ui.text(match d {
                    Dir::In => "S->C",
                    Dir::Out => "C->S",
                });
                ui.table_next_column();
                ui.text(format!("{op:#06x}"));
                ui.table_next_column();
                match name {
                    Some(n) => ui.text(n),
                    None => ui.text_colored([1.0, 0.6, 0.4, 1.0], "?"),
                }
                ui.table_next_column();
                ui.text(s.official.to_string());
                ui.table_next_column();
                if s.private == 0 && s.official > 0 {
                    ui.text_colored([1.0, 0.6, 0.4, 1.0], "0");
                } else {
                    ui.text(s.private.to_string());
                }
                ui.table_next_column();
                ui.text(s.session.to_string());
                ui.table_next_column();
                ui.text(if s.min_len == s.max_len {
                    format!("{}", s.min_len)
                } else {
                    format!("{}..{}", s.min_len, s.max_len)
                });
            }
        }
        let _ = Channel::Login;
    }
}
