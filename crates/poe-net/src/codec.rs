//! Byte codec for PoE messages.
//!
//! Conventions (from PoE1 research, to be confirmed per field for 0.5.5e; see
//! `Documentation/protocol/framing.md`):
//! - integers are big-endian,
//! - `wstring`: u16 length in UTF-16 code units, then UTF-16LE data,
//! - `astring`: u16 length in bytes, then raw bytes (ASCII),
//! - compact integers: a 1–4 byte prefix-coded varint (leading 1-bits give the extra byte count);
//!   the signed variant is added once its 0.5.5e encoding is confirmed.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("need {needed} more bytes at offset {offset}")]
    Incomplete { offset: usize, needed: usize },
    #[error("invalid {what} at offset {offset}")]
    Invalid { offset: usize, what: &'static str },
}

pub type DecodeResult<T> = Result<T, DecodeError>;

/// Cursor over a received byte slice.
///
/// [`DecodeError::Incomplete`] means "wait for more bytes"; the caller rewinds and retries
/// once more data arrives.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

macro_rules! read_be {
    ($($name:ident: $ty:ty),*) => {$(
        pub fn $name(&mut self) -> DecodeResult<$ty> {
            Ok(<$ty>::from_be_bytes(self.array()?))
        }
    )*};
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    pub fn bytes(&mut self, len: usize) -> DecodeResult<&'a [u8]> {
        if self.remaining() < len {
            return Err(DecodeError::Incomplete {
                offset: self.pos,
                needed: len - self.remaining(),
            });
        }
        let out = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> DecodeResult<[u8; N]> {
        Ok(self.bytes(N)?.try_into().expect("length checked"))
    }

    pub fn u8(&mut self) -> DecodeResult<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn i8(&mut self) -> DecodeResult<i8> {
        Ok(self.u8()? as i8)
    }

    pub fn bool(&mut self) -> DecodeResult<bool> {
        Ok(self.u8()? != 0)
    }

    read_be!(u16: u16, u32: u32, u64: u64, i16: i16, i32: i32, i64: i64, f32: f32, f64: f64);

    /// u16 length (UTF-16 code units) followed by UTF-16LE data.
    pub fn wstring(&mut self) -> DecodeResult<String> {
        let start = self.pos;
        let len = self.u16()? as usize;
        let raw = self.bytes(len * 2)?;
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units).map_err(|_| DecodeError::Invalid {
            offset: start,
            what: "utf-16 string",
        })
    }

    /// u16 byte length followed by raw (ASCII/UTF-8) bytes.
    pub fn astring(&mut self) -> DecodeResult<String> {
        let start = self.pos;
        let len = self.u16()? as usize;
        let raw = self.bytes(len)?;
        String::from_utf8(raw.to_vec()).map_err(|_| DecodeError::Invalid {
            offset: start,
            what: "utf-8 string",
        })
    }

    /// Prefix-coded unsigned varint: `0xxxxxxx` (1 byte), `10xxxxxx` (2), `110xxxxx` (3),
    /// `1110xxxx` (4), all big-endian.
    pub fn varuint(&mut self) -> DecodeResult<u32> {
        let start = self.pos;
        let first = self.u8()?;
        let extra = first.leading_ones() as usize;
        if extra > 3 {
            return Err(DecodeError::Invalid {
                offset: start,
                what: "varint prefix",
            });
        }
        let mut value = (first & (0x7f >> extra)) as u32;
        for &b in self.bytes(extra)? {
            value = (value << 8) | b as u32;
        }
        Ok(value)
    }
}

/// Growable output buffer with the same encodings as [`Reader`].
#[derive(Debug, Default, Clone)]
pub struct Writer {
    buf: Vec<u8>,
}

macro_rules! write_be {
    ($($name:ident: $ty:ty),*) => {$(
        pub fn $name(&mut self, v: $ty) -> &mut Self {
            self.buf.extend_from_slice(&v.to_be_bytes());
            self
        }
    )*};
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    pub fn bytes(&mut self, data: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(data);
        self
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn i8(&mut self, v: i8) -> &mut Self {
        self.u8(v as u8)
    }

    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(v as u8)
    }

    write_be!(u16: u16, u32: u32, u64: u64, i16: i16, i32: i32, i64: i64, f32: f32, f64: f64);

    pub fn wstring(&mut self, s: &str) -> &mut Self {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.u16(units.len() as u16);
        for u in units {
            self.buf.extend_from_slice(&u.to_le_bytes());
        }
        self
    }

    pub fn astring(&mut self, s: &str) -> &mut Self {
        self.u16(s.len() as u16);
        self.bytes(s.as_bytes())
    }

    pub fn varuint(&mut self, v: u32) -> &mut Self {
        match v {
            0..0x80 => self.u8(v as u8),
            0x80..0x4000 => self.u16(0x8000 | v as u16),
            0x4000..0x20_0000 => self.bytes(&[0xc0 | (v >> 16) as u8, (v >> 8) as u8, v as u8]),
            _ => {
                assert!(v < 0x1000_0000, "varuint out of range: {v}");
                self.u32(0xe000_0000 | v)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints_are_big_endian() {
        let mut w = Writer::new();
        w.u16(0x1234).u32(0xdeadbeef);
        assert_eq!(w.as_slice(), &[0x12, 0x34, 0xde, 0xad, 0xbe, 0xef]);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert_eq!(r.u32().unwrap(), 0xdeadbeef);
        assert!(r.is_empty());
    }

    #[test]
    fn wstring_roundtrip() {
        let mut w = Writer::new();
        w.wstring("Hëllo");
        assert_eq!(&w.as_slice()[..4], &[0, 5, b'H', 0]);
        assert_eq!(Reader::new(w.as_slice()).wstring().unwrap(), "Hëllo");
    }

    #[test]
    fn incomplete_reports_missing_bytes() {
        let mut r = Reader::new(&[0, 4, b'a', 0]);
        assert_eq!(
            r.wstring(),
            Err(DecodeError::Incomplete {
                offset: 2,
                needed: 6
            })
        );
    }

    #[test]
    fn varuint_roundtrip() {
        for v in [
            0,
            1,
            0x7f,
            0x80,
            0x3fff,
            0x4000,
            0x1f_ffff,
            0x20_0000,
            0x0fff_ffff,
        ] {
            let mut w = Writer::new();
            w.varuint(v);
            assert_eq!(Reader::new(w.as_slice()).varuint().unwrap(), v, "{v:#x}");
        }
    }
}
