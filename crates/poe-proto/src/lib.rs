//! Protocol definitions.
//!
//! Opcode numbers change between client versions, so they are not written in code: each
//! version has `protocol/<version>/<channel>.toml` mapping message names to ids, per
//! direction. Code refers to messages by name and looks the id up once at startup.

pub mod opcodes;

pub use opcodes::{Direction, OpcodeTable, Protocol, ProtocolError};
