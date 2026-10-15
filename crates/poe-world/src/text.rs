//! Helpers for the terrain text formats (UTF-16LE with a BOM, whitespace-separated words,
//! double-quoted strings).

/// Decodes UTF-16LE (with or without a BOM); falls back to UTF-8 for files that are not.
pub fn decode_utf16(bytes: &[u8]) -> String {
    let utf16 = bytes.len() >= 2 && (bytes[..2] == [0xff, 0xfe] || bytes.get(1) == Some(&0));
    if !utf16 {
        return String::from_utf8_lossy(bytes)
            .trim_start_matches('\u{feff}')
            .to_string();
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_start_matches('\u{feff}')
        .to_string()
}

/// Splits a line into words; a double-quoted string is one word (without the quotes, possibly
/// empty).
pub fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut s = String::new();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                s.push(c);
            }
            out.push(s);
        } else {
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                s.push(c);
                chars.next();
            }
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_words() {
        assert_eq!(
            tokens(r#"12 "" "a b" (any) 3"#),
            vec!["12", "", "a b", "(any)", "3"]
        );
    }
}
