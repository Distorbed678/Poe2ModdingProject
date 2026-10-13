//! Byte codec for PoE messages.
//!
//! Conventions (see `Documentation/protocol/framing.md`):
//! - integers are big-endian (confirmed: the client byte-swaps with htons/htonl),
//! - `wstring` (0.5.5e, client reader `0x141cfb3c0`): u16 character count, then per
//!   character either one byte `< 0x80` (ASCII), `0x80` + u16 LE (BMP code unit) or `0xc0` +
//!   u32 LE (any code point),
//! - `astring`: u16 length in bytes, then raw bytes,
//! - compact integers: prefix-coded varints (leading 1-bits give the extra byte count), in
//!   unsigned and signed (sign-extended) flavours.

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

    /// Compact wide string: u16 count, then one entry per character (see module docs).
    pub fn wstring(&mut self) -> DecodeResult<String> {
        let count = self.u16()? as usize;
        let mut out = String::with_capacity(count);
        for _ in 0..count {
            let at = self.pos;
            let invalid = DecodeError::Invalid {
                offset: at,
                what: "wstring character",
            };
            let c = match self.u8()? {
                b @ 0..0x80 => b as u32,
                0x80 => u16::from_le_bytes(self.array()?) as u32,
                0xc0 => u32::from_le_bytes(self.array()?),
                _ => return Err(invalid),
            };
            out.push(char::from_u32(c).ok_or(invalid)?);
        }
        Ok(out)
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

    /// Prefix-coded unsigned varint (client reader `0x1408719a0`): `0xxxxxxx` (7 bits),
    /// `10xxxxxx` +1 byte (14), `110xxxxx` +2 (21), `1110xxxx` +3 (28), otherwise a prefix
    /// byte followed by a full big-endian u32.
    pub fn varuint(&mut self) -> DecodeResult<u32> {
        let first = self.u8()?;
        let extra = first.leading_ones();
        if extra >= 4 {
            return self.u32();
        }
        let mut value = (first & (0x7f >> extra)) as u32;
        for &b in self.bytes(extra as usize)? {
            value = (value << 8) | b as u32;
        }
        Ok(value)
    }

    /// Signed counterpart of [`Self::varuint`] (client reader `0x141cfbfe0`): same shapes, the
    /// top payload bit is the sign.
    pub fn varint(&mut self) -> DecodeResult<i32> {
        let first = self.u8()?;
        let extra = first.leading_ones();
        if extra >= 4 {
            return self.i32();
        }
        let mut value = (first & (0x7f >> extra)) as u32;
        for &b in self.bytes(extra as usize)? {
            value = (value << 8) | b as u32;
        }
        let bits = 7 + 7 * extra; // 7, 14, 21, 28 payload bits
        let shift = 32 - bits;
        Ok(((value << shift) as i32) >> shift)
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

    /// Little-endian integers and floats: entity state blobs are copied raw by the client.
    pub fn u16_le(&mut self, v: u16) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }

    pub fn i16_le(&mut self, v: i16) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }

    pub fn u32_le(&mut self, v: u32) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }

    pub fn i32_le(&mut self, v: i32) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }

    pub fn f32_le(&mut self, v: f32) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }

    /// Blob string: u32 LE UTF-16 unit count, then UTF-16LE units.
    pub fn utf16_le(&mut self, s: &str) -> &mut Self {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.u32_le(units.len() as u32);
        for u in units {
            self.u16_le(u);
        }
        self
    }

    pub fn wstring(&mut self, s: &str) -> &mut Self {
        self.u16(s.chars().count() as u16);
        for c in s.chars() {
            match c as u32 {
                c @ 0..0x80 => {
                    self.u8(c as u8);
                }
                c @ 0x80..0x10000 => {
                    self.u8(0x80).bytes(&(c as u16).to_le_bytes());
                }
                c => {
                    self.u8(0xc0).bytes(&c.to_le_bytes());
                }
            }
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
            0x20_0000..0x1000_0000 => self.u32(0xe000_0000 | v),
            _ => self.u8(0xf0).u32(v),
        }
    }

    pub fn varint(&mut self, v: i32) -> &mut Self {
        let fits = |bits: u32| (-(1i64 << (bits - 1))..(1i64 << (bits - 1))).contains(&(v as i64));
        let raw = v as u32;
        if fits(7) {
            self.u8((raw & 0x7f) as u8)
        } else if fits(14) {
            self.u16(0x8000 | (raw & 0x3fff) as u16)
        } else if fits(21) {
            let p = raw & 0x1f_ffff;
            self.bytes(&[0xc0 | (p >> 16) as u8, (p >> 8) as u8, p as u8])
        } else if fits(28) {
            self.u32(0xe000_0000 | (raw & 0x0fff_ffff))
        } else {
            self.u8(0xf0).i32(v)
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
        w.wstring("Hë😀");
        // count 3: 'H' as one byte, 'ë' as 0x80 + u16 LE, the emoji as 0xc0 + u32 LE.
        assert_eq!(
            w.as_slice(),
            &[0, 3, b'H', 0x80, 0xeb, 0x00, 0xc0, 0x00, 0xf6, 0x01, 0x00]
        );
        assert_eq!(Reader::new(w.as_slice()).wstring().unwrap(), "Hë😀");
    }

    #[test]
    fn incomplete_reports_missing_bytes() {
        let mut r = Reader::new(&[0, 4, b'a']);
        assert_eq!(
            r.wstring(),
            Err(DecodeError::Incomplete {
                offset: 3,
                needed: 1
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
            u32::MAX,
        ] {
            let mut w = Writer::new();
            w.varuint(v);
            assert_eq!(Reader::new(w.as_slice()).varuint().unwrap(), v, "{v:#x}");
        }
    }

    #[test]
    fn varint_roundtrip() {
        for v in [
            0,
            1,
            -1,
            63,
            -64,
            64,
            -65,
            8191,
            -8192,
            1 << 20,
            -(1 << 20),
            1 << 27,
            -(1 << 27),
            i32::MAX,
            i32::MIN,
        ] {
            let mut w = Writer::new();
            w.varint(v);
            assert_eq!(Reader::new(w.as_slice()).varint().unwrap(), v, "{v}");
        }
        // Client semantics: 0x7f is -1 in the one-byte form.
        assert_eq!(Reader::new(&[0x7f]).varint().unwrap(), -1);
    }
}
