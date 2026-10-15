//! Protocol tab: per opcode, what the field tracer learned (the client's own reads and writes)
//! and the analyzer's view of each field across the captured samples.

use imgui::Ui;

use crate::{
    packets::Dir,
    protocol::{self, Key, OpInfo},
};

pub struct ProtocolView {
    filter: String,
    unnamed_only: bool,
    selected: Option<Key>,
    /// Snapshot of the per-opcode data, refreshed every second.
    ops: Vec<(Key, OpInfo)>,
    refreshed: Option<std::time::Instant>,
    /// Note editor for the selected opcode (and which opcode it was loaded for).
    note: String,
    note_for: Option<Key>,
}

impl ProtocolView {
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            unnamed_only: false,
            selected: None,
            ops: Vec::new(),
            refreshed: None,
            note: String::new(),
            note_for: None,
        }
    }

    pub fn draw(&mut self, ui: &Ui) {
        if self
            .refreshed
            .is_none_or(|t| t.elapsed().as_secs_f32() > 1.0)
        {
            self.ops = protocol::snapshot();
            self.refreshed = Some(std::time::Instant::now());
        }
        ui.set_next_item_width(220.0);
        ui.input_text("Filter##proto", &mut self.filter).build();
        ui.same_line();
        ui.checkbox("no name yet##proto", &mut self.unnamed_only);
        ui.same_line();
        if ui.button("Export now") {
            protocol::export();
            crate::ui::notice(format!(
                "protocol.tsv and ghidra_labels.tsv written to {}",
                crate::host::session_dir().display()
            ));
        }
        let f = self.filter.to_lowercase();
        let height = ui.content_region_avail()[1];
        ui.child_window("proto_list")
            .size([360.0, height])
            .border(true)
            .build(ui, || {
                for ((c, d, op), info) in &self.ops {
                    let name = crate::packets::name(*c, *d, *op);
                    if self.unnamed_only && name.is_some() {
                        continue;
                    }
                    let line = format!(
                        "{:4} {} {op:#06x} {}",
                        &c.name()[..4.min(c.name().len())],
                        match d {
                            Dir::In => "S>C",
                            Dir::Out => "C>S",
                        },
                        name.unwrap_or("?")
                    );
                    if !f.is_empty() && !line.to_lowercase().contains(&f) {
                        continue;
                    }
                    let key = (*c, *d, *op);
                    let mark = if crate::notes::has_opcode_note(*c, *d, *op) {
                        " *"
                    } else {
                        ""
                    };
                    let label = format!("{line} ({}){mark}##{c:?}{d:?}{op}", info.count);
                    if ui
                        .selectable_config(label)
                        .selected(self.selected == Some(key))
                        .build()
                    {
                        self.selected = Some(key);
                    }
                }
            });
        ui.same_line();
        ui.child_window("proto_detail")
            .size([0.0, height])
            .border(true)
            .build(ui, || {
                if let Some(key) = self.selected {
                    if self.note_for != Some(key) {
                        self.note = crate::notes::opcode_note(key.0, key.1, key.2);
                        self.note_for = Some(key);
                    }
                    ui.text("Your note on this opcode (what you think it does):");
                    ui.input_text_multiline("##proto_note", &mut self.note, [-1.0, 54.0])
                        .build();
                    if ui.button("Save note##proto") {
                        crate::notes::set_opcode_note(key.0, key.1, key.2, &self.note);
                        crate::ui::notice(format!("Note saved for {:#06x}", key.2));
                    }
                    ui.separator();
                }
                self.detail(ui)
            });
    }

    fn detail(&self, ui: &Ui) {
        let Some(key) = self.selected else {
            ui.text_disabled("Select an opcode.");
            return;
        };
        let Some((_, info)) = self.ops.iter().find(|(k, _)| *k == key) else {
            return;
        };
        let (c, d, op) = key;
        ui.text(format!(
            "{} {} {op:#06x} {}",
            c.name(),
            d.name(),
            crate::packets::name(c, d, op).unwrap_or("(no name in our protocol table)")
        ));
        ui.text(format!(
            "{} messages, size {}..{} bytes, {} samples kept",
            info.count,
            info.min_len,
            info.max_len,
            info.samples.len()
        ));
        let mut functions: Vec<_> = info.functions.iter().collect();
        functions.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        ui.text(match d {
            Dir::In => "Functions that read it (deserializer first):",
            Dir::Out => "Functions that wrote it (serializer first):",
        });
        for (f, n) in functions.iter().take(6) {
            ui.text(format!(
                "  {f:#x}  {}  ({n} reads)",
                crate::symbols::fmt(**f)
            ));
        }
        if functions.is_empty() {
            ui.text_disabled(
                "  (not traced: received before this core loaded, or sent outside the session)",
            );
        }
        let mut layouts: Vec<_> = info.layouts.iter().collect();
        layouts.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        ui.separator();
        ui.text(format!(
            "Field layouts (offset:size), {} variants:",
            layouts.len()
        ));
        for (l, n) in layouts.iter().take(5) {
            ui.text_wrapped(format!("  {n}x  {}", protocol::layout_text(l)));
        }
        let Some(layout) = info.main_layout() else {
            return;
        };
        let covered: u32 = layout.iter().map(|(_, l)| l).sum();
        ui.text_disabled(format!(
            "The main layout covers {covered} of {}..{} bytes (the rest is read without the stream primitive).",
            info.min_len, info.max_len
        ));
        ui.separator();
        let stats = protocol::analyze(layout, &info.samples);
        if let Some(_t) = ui.begin_table_header_with_flags(
            "proto_fields",
            [
                imgui::TableColumnSetup::new("offset"),
                imgui::TableColumnSetup::new("size"),
                imgui::TableColumnSetup::new("samples"),
                imgui::TableColumnSetup::new("distinct"),
                imgui::TableColumnSetup::new("looks like"),
                imgui::TableColumnSetup::new("last value"),
            ],
            crate::ui::table_opts(),
        ) {
            for s in stats {
                ui.table_next_column();
                ui.text(s.offset.to_string());
                ui.table_next_column();
                ui.text(s.len.to_string());
                ui.table_next_column();
                ui.text(s.samples.to_string());
                ui.table_next_column();
                ui.text(s.distinct.to_string());
                ui.table_next_column();
                ui.text(&s.kind);
                ui.table_next_column();
                ui.text(&s.example);
            }
        }
    }
}
