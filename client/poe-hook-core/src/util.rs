//! Small helpers shared by the overlay and the exports.

use std::sync::OnceLock;

/// Local time of day for a Unix timestamp: `HH:MM:SS.mmm`.
pub fn clock(unix_ms: u64) -> String {
    let local = (unix_ms as i64 + utc_offset_ms()).rem_euclid(86_400_000) as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        local / 3_600_000,
        local / 60_000 % 60,
        local / 1000 % 60,
        local % 1000
    )
}

/// Local time minus UTC, from the system clock (whole minutes).
fn utc_offset_ms() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
        let (mut local, mut utc) = unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
        unsafe {
            GetLocalTime(&mut local);
            GetSystemTime(&mut utc);
        }
        let minutes =
            |t: &windows_sys::Win32::Foundation::SYSTEMTIME| t.wHour as i64 * 60 + t.wMinute as i64;
        let mut diff = minutes(&local) - minutes(&utc);
        if diff > 12 * 60 {
            diff -= 24 * 60;
        } else if diff < -12 * 60 {
            diff += 24 * 60;
        }
        diff * 60_000
    })
}

pub fn level_name(level: u32) -> &'static str {
    match level {
        1 => "ERROR",
        2 => "WARN",
        3 => "INFO",
        4 => "DEBUG",
        _ => "TRACE",
    }
}

/// Hex dump: `00 01 02 ...`, 16 bytes per line with offsets.
pub fn hex_lines(data: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        out.push_str(&format!("{:04x}  {:<47}  {ascii}\n", i * 16, hex.join(" ")));
    }
    out
}

/// One-line hex: `00 01 02` (at most `max` bytes, then `..`).
pub fn hex(data: &[u8], max: usize) -> String {
    let mut s: Vec<String> = data.iter().take(max).map(|b| format!("{b:02x}")).collect();
    if data.len() > max {
        s.push("..".into());
    }
    s.join(" ")
}
