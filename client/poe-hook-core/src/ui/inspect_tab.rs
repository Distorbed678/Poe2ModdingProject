//! Player, Entities and Watches tabs: live, read-only views of game memory.

use imgui::{ListClipper, Ui};

use crate::watches::{KINDS, Kind, Watch};

pub fn draw_player(ui: &Ui) {
    let Some(p) = crate::inspect::player() else {
        ui.text_disabled("No player entity yet (it is found when it is created or ticks).");
        return;
    };
    ui.text(format!("entity {:#x}  id {}", p.entity, p.id));
    ui.text(format!("grid ({}, {})", p.grid.0, p.grid.1));
    ui.text(format!(
        "world ({:.2}, {:.2})  =  cells ({:.2}, {:.2})",
        p.world.0,
        p.world.1,
        p.world.0 / (250.0 / 23.0),
        p.world.1 / (250.0 / 23.0)
    ));
    ui.text(format!(
        "facing {:.3} rad ({:.1} deg){}",
        p.facing,
        p.facing.to_degrees(),
        if p.facing_locked { "  locked" } else { "" }
    ));
    ui.text(format!("actions {:#06x} / {:#06x}", p.action, p.action2));
    ui.text(format!("WASD direction ({}, {})", p.wasd.0, p.wasd.1));
    ui.text(format!(
        "actor flags +0x2a2 = {:#04x}{}",
        p.flags,
        if p.flags & 0x20 != 0 {
            " (sprint held)"
        } else {
            ""
        }
    ));
}

pub struct EntitiesView {
    filter: String,
    alive_only: bool,
    selected: Option<usize>,
}

impl EntitiesView {
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            alive_only: true,
            selected: None,
        }
    }

    pub fn draw(&mut self, ui: &Ui) {
        ui.set_next_item_width(260.0);
        ui.input_text("Filter (path or id)", &mut self.filter)
            .build();
        ui.same_line();
        ui.checkbox("alive only", &mut self.alive_only);
        let f = self.filter.to_lowercase();
        let rows: Vec<_> = crate::entities::list()
            .into_iter()
            .map(|e| {
                let live = crate::entities::live(&e);
                (e, live)
            })
            .filter(|(e, l)| {
                (!self.alive_only || l.alive)
                    && (f.is_empty()
                        || e.path.to_lowercase().contains(&f)
                        || format!("{:#x}", e.id).contains(&f)
                        || e.id.to_string() == f)
            })
            .collect();
        ui.text_disabled(format!(
            "{} entities created in this area (from the initial-state reader)",
            rows.len()
        ));
        if let Some(_t) = ui.begin_table_header_with_flags(
            "entity_list",
            [
                imgui::TableColumnSetup::new("id").fixed_width(110.0),
                imgui::TableColumnSetup::new("address").fixed_width(260.0),
                imgui::TableColumnSetup::new("type").fixed_width(420.0),
                imgui::TableColumnSetup::new("grid").fixed_width(140.0),
                imgui::TableColumnSetup::new("components").fixed_width(110.0),
            ],
            crate::ui::table_opts(),
        ) {
            let mut clipper = ListClipper::new(rows.len()).begin(ui);
            while clipper.step() {
                for i in clipper.display_start()..clipper.display_end() {
                    let (e, l) = &rows[i as usize];
                    ui.table_next_column();
                    if ui
                        .selectable_config(format!("{:#x}##ent{}", e.id, e.ptr))
                        .selected(self.selected == Some(e.ptr))
                        .span_all_columns(true)
                        .build()
                    {
                        self.selected = Some(e.ptr);
                    }
                    ui.table_next_column();
                    if l.alive {
                        ui.text(format!("{:#x}", e.ptr));
                    } else {
                        ui.text_disabled(format!("{:#x} (gone)", e.ptr));
                    }
                    ui.table_next_column();
                    ui.text(if e.path.is_empty() {
                        format!("key {:#010x}", e.type_key)
                    } else {
                        e.path.clone()
                    });
                    ui.table_next_column();
                    ui.text(l.grid.map_or("-".into(), |g| format!("({}, {})", g.0, g.1)));
                    ui.table_next_column();
                    ui.text(e.components.len().to_string());
                    if ui.is_item_hovered() {
                        let list: Vec<String> =
                            e.components.iter().map(|v| format!("{v:#x}")).collect();
                        ui.tooltip_text(format!("component vtables:\n{}", list.join("\n")));
                    }
                }
            }
        }
        if let Some(ptr) = self.selected {
            ui.separator();
            components(ui, ptr);
        }
    }
}

/// The selected entity's components: name, address, vtable; "scan" sends one to the Scanner.
fn components(ui: &Ui, ptr: usize) {
    let names = crate::entities::component_names(ptr);
    let comps = crate::entities::components(ptr);
    ui.text(format!("Components of {ptr:#x} ({}):", comps.len()));
    let main = crate::dump_types::main_module();
    if let Some(_t) = ui.begin_table_header_with_flags(
        "components",
        [
            imgui::TableColumnSetup::new("#"),
            imgui::TableColumnSetup::new("name"),
            imgui::TableColumnSetup::new("address"),
            imgui::TableColumnSetup::new("vtable"),
            imgui::TableColumnSetup::new(""),
        ],
        crate::ui::table_opts(),
    ) {
        for (i, c) in comps.iter().enumerate() {
            let name = names
                .iter()
                .find(|(n, _)| *n == i)
                .map_or("?", |(_, s)| s.as_str());
            let vt = poe_hook_shared::mem::read::<usize>(*c).unwrap_or(0);
            ui.table_next_column();
            ui.text(i.to_string());
            ui.table_next_column();
            ui.text(name);
            ui.table_next_column();
            ui.text(format!("{c:#x}"));
            ui.table_next_column();
            ui.text(match main.filter(|m| m.contains(vt)) {
                Some(m) => format!("{:#x}", m.to_ghidra(vt)),
                None => format!("{vt:#x}"),
            });
            ui.table_next_column();
            if ui.small_button(format!("scan##c{i}")) {
                crate::ui::scanner_request(format!("{c:#x}"), 0x600);
            }
        }
    }
}

pub struct WatchesView {
    list: Vec<Watch>,
    new_expr: String,
    new_kind: usize,
}

impl WatchesView {
    pub fn new() -> Self {
        Self {
            list: crate::watches::load(),
            new_expr: String::new(),
            new_kind: 2,
        }
    }

    pub fn draw(&mut self, ui: &Ui) {
        ui.text_wrapped(
            "Read-only memory watches. Syntax: numbers (0x.. hex; 0x14.. = Ghidra address), \
             + and -, [expr] reads a pointer; names: player, pos, actor, exe. \
             Example: pos+0x44c (f32 facing).",
        );
        ui.set_next_item_width(360.0);
        ui.input_text("expression", &mut self.new_expr).build();
        ui.same_line();
        ui.set_next_item_width(90.0);
        let names: Vec<&str> = KINDS.iter().map(|k| k.name()).collect();
        ui.combo_simple_string("type", &mut self.new_kind, &names);
        ui.same_line();
        if ui.button("Add") && !self.new_expr.trim().is_empty() {
            self.list.push(Watch {
                expr: self.new_expr.trim().to_owned(),
                kind: KINDS[self.new_kind],
            });
            crate::watches::save(&self.list);
        }
        ui.separator();
        let mut remove = None;
        for (i, w) in self.list.iter().enumerate() {
            if ui.small_button(format!("x##w{i}")) {
                remove = Some(i);
            }
            ui.same_line();
            ui.text(format!(
                "{:4} {:32} = {}",
                w.kind.name(),
                w.expr,
                crate::watches::value(w)
            ));
        }
        if let Some(i) = remove {
            self.list.remove(i);
            crate::watches::save(&self.list);
        }
        let _ = Kind::U8;
    }
}
