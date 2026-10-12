//! Hexdump formatting for logs.

use std::fmt::Write;

/// Classic 16-bytes-per-line hexdump with an ASCII column.
pub fn hexdump(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4 + 16);
    for (i, chunk) in data.chunks(16).enumerate() {
        let _ = write!(out, "{:08x}  ", i * 16);
        for j in 0..16 {
            match chunk.get(j) {
                Some(b) => {
                    let _ = write!(out, "{b:02x} ");
                }
                None => out.push_str("   "),
            }
            if j == 7 {
                out.push(' ');
            }
        }
        out.push(' ');
        for &b in chunk {
            out.push(if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            });
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_partial_line() {
        let s = super::hexdump(b"AB\x00");
        assert!(s.starts_with("00000000  41 42 00"));
        assert!(s.trim_end().ends_with("AB."));
    }
}
