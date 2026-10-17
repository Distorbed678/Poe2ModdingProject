//! Commands tab (our server only): a form for every command in the server's registry (the
//! same commands as `!name` in chat), grouped as the server groups them, and their results.

use std::collections::HashMap;

use imgui::Ui;

use super::layout_form::LayoutForm;
use crate::devlink::{Cmd, CmdArg};

fn layout_kind(k: &str) -> bool {
    matches!(k, "area" | "seed" | "topology" | "orientation")
}

pub struct CommandsView {
    values: HashMap<String, Vec<String>>,
    forms: HashMap<String, LayoutForm>,
    last_request: Option<std::time::Instant>,
}

impl CommandsView {
    pub fn new() -> Self {
        Self { values: HashMap::new(), forms: HashMap::new(), last_request: None }
    }

    pub fn draw(&mut self, ui: &Ui) {
        let (cmds, me) = crate::devlink::registry();
        if cmds.is_empty() {
            ui.text_disabled("No command list from the server yet (needs position sync and [instance] debug_link).");
            // Ask again now and then (e.g. after a core reload).
            if self.last_request.is_none_or(|t| t.elapsed().as_secs() >= 3) && crate::devlink::state().is_some() {
                crate::devlink::command("registry");
                self.last_request = Some(std::time::Instant::now());
            }
            return;
        }
        let prefix = me.as_ref().map_or("!".to_string(), |m| m.prefix.clone());
        match &me {
            Some(m) => ui.text(format!(
                "Account {} ({}). In chat: {prefix}name args, {prefix}help.",
                m.account,
                if m.admin { "admin" } else { "not an admin: admin commands are refused" }
            )),
            None => ui.text_disabled("account unknown"),
        }
        ui.same_line();
        if ui.small_button("Reload list") {
            crate::devlink::command("registry");
        }
        let mut groups: Vec<&str> = Vec::new();
        for c in &cmds {
            if !groups.contains(&c.group.as_str()) {
                groups.push(&c.group);
            }
        }
        let admin = me.as_ref().is_some_and(|m| m.admin);
        let avail = ui.content_region_avail();
        ui.child_window("commands").size([0.0, (avail[1] * 0.7).max(200.0)]).build(ui, || {
            for g in &groups {
                if !ui.collapsing_header(format!("{g}##group"), imgui::TreeNodeFlags::DEFAULT_OPEN) {
                    continue;
                }
                for c in cmds.iter().filter(|c| c.group == *g) {
                    self.draw_command(ui, c, admin, &prefix);
                }
            }
        });
        ui.separator();
        ui.text("Results");
        ui.child_window("results").border(true).build(ui, || {
            for (ms, cmd, result) in crate::devlink::results().iter().rev() {
                let color = if result.starts_with("ok") { [0.6, 1.0, 0.6, 1.0] } else { [1.0, 0.6, 0.5, 1.0] };
                ui.text_colored(color, format!("{} {cmd} -> {result}", crate::util::clock(*ms)));
            }
        });
    }

    fn draw_command(&mut self, ui: &Ui, c: &Cmd, admin: bool, prefix: &str) {
        let _id = ui.push_id(&c.name);
        let values = self.values.entry(c.name.clone()).or_insert_with(|| vec![String::new(); c.args.len()]);
        ui.text(format!("{prefix}{}", c.name));
        if c.admin {
            ui.same_line();
            ui.text_colored(if admin { [0.8, 0.8, 0.4, 1.0] } else { [0.6, 0.6, 0.6, 1.0] }, "[admin]");
        }
        ui.same_line();
        ui.text_disabled(&c.help);
        let has_layout = c.args.iter().any(|a| a.kind == "area");
        for (i, a) in c.args.iter().enumerate() {
            if has_layout && layout_kind(&a.kind) {
                continue;
            }
            ui.same_line();
            arg_widget(ui, a, &mut values[i]);
        }
        if has_layout {
            let form = self.forms.entry(c.name.clone()).or_insert_with(LayoutForm::new);
            form.draw(ui, &c.name);
        }
        if !has_layout {
            ui.same_line();
        }
        if ui.button(format!("Run##{}", c.name)) {
            let line = build_line(c, &self.values[&c.name], self.forms.get(&c.name));
            crate::devlink::command(&line);
            if let Some(f) = self.forms.get(&c.name) {
                f.used();
            }
        }
        ui.spacing();
    }
}

fn arg_widget(ui: &Ui, a: &CmdArg, value: &mut String) {
    let label = format!("{}{}##{}", a.name, if a.optional { "?" } else { "" }, a.name);
    match a.kind.as_str() {
        "bool" => {
            let mut on = value == "on";
            if ui.checkbox(&label, &mut on) {
                *value = if on { "on".into() } else { "off".into() };
            }
            if value.is_empty() {
                *value = "off".into();
            }
        }
        k if k.starts_with("choice=") => {
            let choices: Vec<&str> = k["choice=".len()..].split('|').collect();
            let mut i = choices.iter().position(|c| c == value).unwrap_or(0);
            ui.set_next_item_width(120.0);
            if ui.combo_simple_string(&label, &mut i, &choices) || value.is_empty() {
                *value = choices[i].to_string();
            }
        }
        "instance" => {
            let ids: Vec<String> = crate::devlink::instances()
                .map(|(_, l, _)| l.iter().map(|i| format!("{} {}", i.id, i.area)).collect())
                .unwrap_or_default();
            ui.set_next_item_width(90.0);
            ui.input_text(&label, value).build();
            if !ids.is_empty() {
                ui.same_line();
                let mut pick = 0usize;
                let mut items = vec!["pick".to_string()];
                items.extend(ids);
                ui.set_next_item_width(140.0);
                if ui.combo_simple_string(format!("##pick{}", a.name), &mut pick, &items) && pick > 0 {
                    *value = items[pick].split(' ').next().unwrap_or("").to_string();
                }
            }
        }
        "rest" => {
            ui.set_next_item_width(320.0);
            ui.input_text(&label, value).build();
        }
        _ => {
            ui.set_next_item_width(110.0);
            ui.input_text(&label, value).build();
        }
    }
}

/// The command line from the form: arguments in order; an empty optional argument followed
/// by a filled one gets the kind's "default" word, trailing empty ones are left out.
fn build_line(c: &Cmd, values: &[String], form: Option<&LayoutForm>) -> String {
    let layout: Vec<String> = form.map(|f| f.args().split(' ').map(str::to_owned).collect()).unwrap_or_default();
    let mut words: Vec<String> = Vec::new();
    let mut li = 0;
    for (i, a) in c.args.iter().enumerate() {
        if form.is_some() && layout_kind(&a.kind) {
            words.push(layout.get(li).cloned().unwrap_or_default());
            li += 1;
            continue;
        }
        words.push(values.get(i).cloned().unwrap_or_default().trim().to_string());
    }
    while words.last().is_some_and(String::is_empty) {
        words.pop();
    }
    let words: Vec<String> = words
        .into_iter()
        .zip(&c.args)
        .map(|(w, a)| {
            if !w.is_empty() {
                return w;
            }
            match a.kind.as_str() {
                "seed" => "random".into(),
                "topology" | "orientation" => "natural".into(),
                _ => "-".into(),
            }
        })
        .collect();
    std::iter::once(c.name.clone()).chain(words).collect::<Vec<_>>().join(" ")
}
