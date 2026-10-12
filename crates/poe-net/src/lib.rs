//! Transport layer shared by the login and instance servers.
//!
//! The PoE stream has no length prefix: a message is an opcode followed by its fields, so
//! framing is the job of `poe-proto`. This crate only provides the byte-level pieces:
//! - [`codec`]: big-endian reader/writer with PoE string encodings,
//! - [`cipher`]: the [`cipher::StreamCipher`] abstraction (plain or encrypted streams),
//! - [`conn`]: a buffered TCP connection that applies a cipher in both directions,
//! - [`hexdump`]: logging helper.

pub mod cipher;
pub mod codec;
pub mod conn;
pub mod hexdump;

pub use cipher::{KeyMaterial, Plain, Salsa20Stream, StreamCipher};
pub use codec::{DecodeError, Reader, Writer};
pub use conn::Connection;
