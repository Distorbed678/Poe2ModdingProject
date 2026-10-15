//! Status tab: mode and read-only latch, session folder, core reload, connections, Vulkan.

use imgui::Ui;

fn mode_line() -> String {
    let cfg = crate::config();
    format!(
        "mode {:?}, read-only {}",
        cfg.mode,
        if crate::host::read_only() { "ON" } else { "off" }
    )
}

pub fn draw(ui: &Ui) {
    if crate::host::read_only() {
        ui.text_colored(
            [1.0, 0.75, 0.2, 1.0],
            "Read-only: nothing is sent, altered or injected; the DLL only observes.",
        );
    }
    ui.text(mode_line());
    ui.text(format!(
        "debug core {} (load #{})",
        env!("CARGO_PKG_VERSION"),
        crate::host::reload_count()
    ));
    ui.text(format!("session folder: {}", crate::host::session_dir().display()));
    ui.text(format!("{:.0} fps", ui.io().framerate()));
    if ui.button("Reload core") {
        crate::host::request_reload();
    }
    ui.same_line();
    if ui.button("Snapshot") {
        crate::snapshot::request();
    }
    ui.same_line();
    if ui.button("Hide overlay") {
        crate::overlay::set_visible(false);
    }
    let cfg = &crate::config().overlay;
    ui.text_disabled(format!(
        "keys: {:#04x} shows/hides the overlay, {:#04x} saves a snapshot",
        cfg.toggle_key, cfg.snapshot_key
    ));
    ui.separator();
    ui.text("Connections");
    if let Some(_t) = ui.begin_table_header(
        "conns",
        [
            imgui::TableColumnSetup::new("#"),
            imgui::TableColumnSetup::new("socket"),
            imgui::TableColumnSetup::new("address"),
            imgui::TableColumnSetup::new("kind"),
            imgui::TableColumnSetup::new("time"),
        ],
    ) {
        for c in crate::host::connections().iter().rev() {
            ui.table_next_column();
            ui.text(c.seq.to_string());
            ui.table_next_column();
            ui.text(format!("{:#x}", c.socket));
            ui.table_next_column();
            ui.text(format!(
                "{}.{}.{}.{}:{}",
                c.ip[0], c.ip[1], c.ip[2], c.ip[3], c.port
            ));
            ui.table_next_column();
            ui.text(if c.external { "external" } else { "local" });
            ui.table_next_column();
            ui.text(crate::util::clock(c.unix_ms));
        }
    }
    ui.separator();
    ui.text(vulkan_line());
}

fn vulkan_line() -> String {
    match crate::host::vulkan() {
        Some(v) => format!(
            "Vulkan: swapchain {}x{} format {:?}, {} images, usage {:#x}",
            v.width,
            v.height,
            ash::vk::Format::from_raw(v.format),
            v.image_count,
            v.usage
        ),
        None => "Vulkan: no swapchain yet".into(),
    }
}

pub fn state_text() -> String {
    let mut s = String::new();
    s.push_str(&mode_line());
    s.push('\n');
    s.push_str(&format!(
        "debug core {} (load #{})\n",
        env!("CARGO_PKG_VERSION"),
        crate::host::reload_count()
    ));
    s.push_str(&vulkan_line());
    s.push('\n');
    for c in crate::host::connections() {
        s.push_str(&format!(
            "connection #{} socket {:#x} {}.{}.{}.{}:{} {} at {}\n",
            c.seq,
            c.socket,
            c.ip[0],
            c.ip[1],
            c.ip[2],
            c.ip[3],
            c.port,
            if c.external { "external" } else { "local" },
            crate::util::clock(c.unix_ms)
        ));
    }
    s.push_str(&crate::inspect::player_text());
    s
}
