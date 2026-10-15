//! Packets tab: every captured message (login and instance, both directions) with filters,
//! and the selected message decoded (known layouts) and as a hex dump. The same messages are in
//! `packets.jsonl` / `packets.txt` in the session folder.

use std::sync::Arc;

use imgui::{ListClipper, Ui};

use crate::packets::{Channel, Dir, Msg};

/// Opcodes hidden by "hide noise": frame reports, pings and per-tick messages.
fn noisy(m: &Msg) -> bool {
    matches!(
        m.name(),
        Some("FrameReport" | "PingReply" | "SimulationTick" | "KeepAlive" | "KeepAliveReply")
    )
}

pub struct PacketsView {
    msgs: Vec<Arc<Msg>>,
    last_seq: u64,
    filter: String,
    show_in: bool,
    show_out: bool,
    channels: [bool; 3],
    hide_noise: bool,
    paused: bool,
    follow: bool,
    selected: Option<u64>,
    shown: Vec<usize>,
    shown_key: (String, bool, bool, [bool; 3], bool),
    shown_upto: usize,
    detail: Option<(u64, String)>,
}

impl PacketsView {
    pub fn new() -> Self {
        Self {
            msgs: Vec::new(),
            last_seq: 0,
            filter: String::new(),
            show_in: true,
            show_out: true,
            channels: [true; 3],
            hide_noise: true,
            paused: false,
            follow: true,
            selected: None,
            shown: Vec::new(),
            shown_key: (String::new(), true, true, [true; 3], true),
            shown_upto: 0,
            detail: None,
        }
    }

    fn pull(&mut self) {
        if self.paused {
            return;
        }
        let new = crate::packets::since(self.last_seq);
        if let Some(m) = new.last() {
            self.last_seq = m.seq;
        }
        self.msgs.extend(new);
        let keep = crate::config().packets.keep.max(100);
        if self.msgs.len() > keep {
            let drop = self.msgs.len() - keep;
            self.msgs.drain(..drop);
            self.shown.clear();
            self.shown_upto = 0;
        }
    }

    fn passes(&self, m: &Msg) -> bool {
        let dir_ok = match m.dir {
            Dir::In => self.show_in,
            Dir::Out => self.show_out,
        };
        let chan_ok = self.channels[match m.channel {
            Channel::Login => 0,
            Channel::Instance => 1,
            Channel::Other => 2,
        }];
        if !dir_ok || !chan_ok || (self.hide_noise && noisy(m)) {
            return false;
        }
        if self.filter.is_empty() {
            return true;
        }
        let f = self.filter.to_lowercase();
        format!("{:#06x}", m.opcode).contains(&f)
            || m.name().is_some_and(|n| n.to_lowercase().contains(&f))
    }

    fn refresh(&mut self) {
        let key = (
            self.filter.clone(),
            self.show_in,
            self.show_out,
            self.channels,
            self.hide_noise,
        );
        if key != self.shown_key {
            self.shown_key = key;
            self.shown.clear();
            self.shown_upto = 0;
        }
        for i in self.shown_upto..self.msgs.len() {
            if self.passes(&self.msgs[i]) {
                self.shown.push(i);
            }
        }
        self.shown_upto = self.msgs.len();
    }

    pub fn draw(&mut self, ui: &Ui) {
        self.pull();
        ui.set_next_item_width(220.0);
        ui.input_text("Filter (name or 0x..)", &mut self.filter)
            .build();
        ui.same_line();
        ui.checkbox("in", &mut self.show_in);
        ui.same_line();
        ui.checkbox("out", &mut self.show_out);
        for (i, name) in ["login", "instance", "other"].iter().enumerate() {
            ui.same_line();
            ui.checkbox(name, &mut self.channels[i]);
        }
        ui.same_line();
        ui.checkbox("hide noise", &mut self.hide_noise);
        ui.same_line();
        ui.checkbox("pause", &mut self.paused);
        ui.same_line();
        ui.checkbox("follow", &mut self.follow);
        self.refresh();
        ui.text_disabled(format!(
            "{} of {} messages (in memory; all are in packets.jsonl / packets.txt){}",
            self.shown.len(),
            self.msgs.len(),
            if crate::config().packets.redact {
                "; secrets redacted"
            } else {
                ""
            }
        ));
        let avail = ui.content_region_avail();
        let list_height = (avail[1] * 0.55).max(120.0);
        ui.child_window("packet_list")
            .size([0.0, list_height])
            .border(true)
            .build(ui, || {
                let mut clipper = ListClipper::new(self.shown.len()).begin(ui);
                while clipper.step() {
                    for row in clipper.display_start()..clipper.display_end() {
                        let m = &self.msgs[self.shown[row as usize]];
                        let line = format!(
                            "{:6} {} {:3} {:8} {:#06x} {:28} {:6}{}",
                            m.seq,
                            crate::util::clock(m.unix_ms),
                            m.dir.name(),
                            m.channel.name(),
                            m.opcode,
                            m.name().unwrap_or("?"),
                            m.len,
                            if m.framed { "" } else { " unframed" }
                        );
                        let color = match m.dir {
                            Dir::In => [0.6, 0.85, 1.0, 1.0],
                            Dir::Out => [1.0, 0.85, 0.55, 1.0],
                        };
                        let _c = ui.push_style_color(imgui::StyleColor::Text, color);
                        let selected = self.selected == Some(m.seq);
                        if ui
                            .selectable_config(format!("{line}##{}", m.seq))
                            .selected(selected)
                            .build()
                        {
                            self.selected = Some(m.seq);
                            self.follow = false;
                        }
                    }
                }
                if self.follow && ui.scroll_y() >= ui.scroll_max_y() - 40.0 {
                    ui.set_scroll_here_y(1.0);
                }
            });
        let Some(seq) = self.selected else {
            ui.text_disabled("Select a message to see it decoded.");
            return;
        };
        let Some(m) = self.msgs.iter().find(|m| m.seq == seq) else {
            return;
        };
        if self.detail.as_ref().is_none_or(|(s, _)| *s != seq) {
            let mut text = format!(
                "#{} {} {} {} {:#06x} {} ({} bytes{}{})\n\n",
                m.seq,
                crate::util::clock(m.unix_ms),
                m.dir.name(),
                m.channel.name(),
                m.opcode,
                m.name().unwrap_or("unknown opcode"),
                m.len,
                if m.framed { "" } else { ", unframed" },
                if m.official { ", official" } else { "" }
            );
            match crate::packets::describe(m) {
                Some(d) => text.push_str(&d),
                None => text.push_str("(no decoder for this message)"),
            }
            text.push_str("\n\n");
            text.push_str(&crate::util::hex_lines(&m.data));
            self.detail = Some((seq, text));
        }
        let text = self
            .detail
            .as_ref()
            .map(|(_, t)| t.clone())
            .unwrap_or_default();
        if ui.button("Copy") {
            crate::ui::set_clipboard(&text);
        }
        ui.child_window("packet_detail")
            .border(true)
            .flags(imgui::WindowFlags::HORIZONTAL_SCROLLBAR)
            .build(ui, || {
                ui.text(&text);
            });
    }
}
