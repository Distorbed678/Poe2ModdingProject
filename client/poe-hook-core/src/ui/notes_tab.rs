//! Notes tab: every packet note (opcode notes first), as saved in
//! `logs/poe-hook/packet_notes.tsv`. Notes are written in the Packets and Protocol tabs.

use imgui::Ui;

pub fn draw(ui: &Ui) {
    let notes = crate::notes::all();
    ui.text_disabled(format!(
        "{} notes; file: logs/poe-hook/packet_notes.tsv (kept across sessions). Write them in the Packets or Protocol tab.",
        notes.len()
    ));
    if let Some(_t) = ui.begin_table_header_with_flags(
        "notes",
        [
            imgui::TableColumnSetup::new("on"),
            imgui::TableColumnSetup::new("channel"),
            imgui::TableColumnSetup::new("dir"),
            imgui::TableColumnSetup::new("opcode"),
            imgui::TableColumnSetup::new("name"),
            imgui::TableColumnSetup::new("when"),
            imgui::TableColumnSetup::new("note"),
        ],
        crate::ui::table_opts(),
    ) {
        for n in &notes {
            ui.table_next_column();
            ui.text(if n.message {
                format!("message #{}", n.seq)
            } else {
                "opcode".into()
            });
            ui.table_next_column();
            ui.text(n.channel.name());
            ui.table_next_column();
            ui.text(n.dir.name());
            ui.table_next_column();
            ui.text(format!("{:#06x}", n.opcode));
            ui.table_next_column();
            ui.text(crate::packets::name(n.channel, n.dir, n.opcode).unwrap_or("?"));
            ui.table_next_column();
            ui.text(if n.message {
                format!("{} {}", n.session, crate::util::clock(n.unix_ms))
            } else {
                crate::util::clock(n.unix_ms)
            });
            ui.table_next_column();
            ui.text_wrapped(&n.text);
        }
    }
}
