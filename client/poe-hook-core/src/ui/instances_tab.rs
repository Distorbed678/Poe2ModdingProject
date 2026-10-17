//! Instances tab (our server only): every instance on the server with join/close, and a form
//! to transfer to a new instance or create one without entering it (area, seed, topology,
//! orientation). Uses the server's `join`, `close`, `transfer` and `instance_new` commands.

use imgui::Ui;

use super::layout_form::LayoutForm;

pub struct InstancesView {
    form: LayoutForm,
}

impl InstancesView {
    pub fn new() -> Self {
        Self { form: LayoutForm::new() }
    }

    pub fn draw(&mut self, ui: &Ui) {
        let (_, me) = crate::devlink::registry();
        let Some((age, list, idle)) = crate::devlink::instances() else {
            ui.text_disabled("No instance list from the server yet (needs position sync and [instance] debug_link).");
            return;
        };
        if !me.as_ref().is_some_and(|m| m.admin) {
            ui.text_colored([1.0, 0.6, 0.4, 1.0], "Your account is not in [admin] accounts: the server refuses these commands.");
        }
        ui.text(format!("{} instances (list {} s old){}", list.len(), age.as_secs(), if idle > 0 { format!("; empty ones close after {idle} min") } else { String::new() }));
        if let Some(_t) = ui.begin_table_header_with_flags(
            "instances",
            ["id", "area", "name", "seed", "topology", "orientation", "players", "idle", ""].map(imgui::TableColumnSetup::new),
            super::table_opts(),
        ) {
            for inst in &list {
                ui.table_next_column();
                if inst.current {
                    ui.text_colored([0.5, 1.0, 0.5, 1.0], format!("{} (you)", inst.id));
                } else {
                    ui.text(inst.id.to_string());
                }
                ui.table_next_column();
                ui.text(&inst.area);
                ui.table_next_column();
                ui.text(crate::areas::find(&inst.area).map_or("", |a| a.name.as_str()));
                ui.table_next_column();
                ui.text(inst.seed.map_or("-".into(), |s| s.to_string()));
                ui.table_next_column();
                ui.text(inst.topology.map_or("natural".into(), |t| t.to_string()));
                ui.table_next_column();
                ui.text(inst.orientation.map_or("natural".into(), |o| o.to_string()));
                ui.table_next_column();
                ui.text(inst.players.join(", "));
                ui.table_next_column();
                match (inst.persistent, inst.empty_secs) {
                    (true, _) => ui.text_disabled("kept"),
                    (false, Some(s)) if idle > 0 => {
                        let left = (idle * 60).saturating_sub(s);
                        ui.text(format!("closes in {}:{:02}", left / 60, left % 60));
                    }
                    (false, Some(s)) => ui.text(format!("empty {s} s")),
                    (false, None) => ui.text_disabled("-"),
                }
                ui.table_next_column();
                let _id = ui.push_id(inst.id as usize);
                if !inst.current && ui.small_button("Join") {
                    crate::devlink::command(&format!("join {}", inst.id));
                }
                ui.same_line();
                if ui.small_button("Close") {
                    crate::devlink::command(&format!("close {}", inst.id));
                }
            }
        }
        ui.separator();
        ui.text("New instance");
        self.form.draw(ui, "new_instance");
        let ready = self.form.has_area();
        {
            let _d = ui.begin_disabled_with_cond(!ready);
            if ui.button("Transfer there") {
                crate::devlink::command(&format!("transfer {}", self.form.args()));
                self.form.used();
            }
            ui.same_line();
            if ui.button("Create without transferring") {
                crate::devlink::command(&format!("instance_new {}", self.form.args()));
                self.form.used();
            }
        }
        ui.text_disabled("Chat equivalent: !transfer <area> [seed] [topology] [orientation], !instance_new ..., !join <id>, !close <id>");
    }
}
