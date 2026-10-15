//! Features tab: what is on, runtime switches, and the read-only rules.

use std::sync::atomic::Ordering;

use imgui::Ui;

pub fn draw(ui: &Ui) {
    let cfg = crate::config();
    let read_only = crate::host::read_only();
    ui.text("Runtime switches");
    let mut movement = crate::dump_types::MOVEMENT_LOG.load(Ordering::Relaxed);
    if ui.checkbox(
        "movement log (per-frame player position in the log)",
        &mut movement,
    ) {
        crate::dump_types::MOVEMENT_LOG.store(movement, Ordering::Relaxed);
    }
    if read_only {
        ui.text_disabled("position sync: off (read-only)");
    } else {
        let mut sync = crate::position_sync::ENABLED.load(Ordering::Relaxed);
        if ui.checkbox("position sync (reports to our instance server)", &mut sync) {
            crate::position_sync::ENABLED.store(sync, Ordering::Relaxed);
        }
    }
    ui.separator();
    ui.text("From poe-hook.toml (edit it, then Reload core on the Status tab)");
    let f = &cfg.features;
    let rows = [
        ("mode", format!("{:?}", cfg.mode)),
        ("crash_log", f.crash_log.to_string()),
        (
            "crypto_bypass",
            format!(
                "{}{}",
                f.crypto_bypass,
                if read_only { " (off: read-only)" } else { "" }
            ),
        ),
        ("net_trace", f.net_trace.to_string()),
        ("block_external", f.block_external.to_string()),
        ("dump_object_types", f.dump_object_types.to_string()),
        ("position_sync_port", f.position_sync_port.to_string()),
        ("trace_addresses", format!("{:?}", f.trace_addresses)),
        ("packets.capture", cfg.packets.capture.to_string()),
        ("packets.redact", cfg.packets.redact.to_string()),
        ("packets.keep", cfg.packets.keep.to_string()),
        ("overlay.scale", cfg.overlay.scale.to_string()),
        ("reload.auto", cfg.reload.auto.to_string()),
    ];
    if let Some(_t) = ui.begin_table_header_with_flags(
        "features",
        [
            imgui::TableColumnSetup::new("setting"),
            imgui::TableColumnSetup::new("value"),
        ],
        crate::ui::table_opts(),
    ) {
        for (k, v) in rows {
            ui.table_next_column();
            ui.text(k);
            ui.table_next_column();
            ui.text(v);
        }
    }
    ui.separator();
    ui.text_wrapped(
        "Read-only (official mode or any external connection): no crypto bypass, no position \
         sync, no test input, no memory writes. The overlay only reads memory and captures \
         traffic.",
    );
}
