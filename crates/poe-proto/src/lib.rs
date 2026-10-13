//! Protocol definitions.
//!
//! Opcode numbers change between client versions, so they are not written in code: each
//! version has `protocol/<version>/<channel>.toml` mapping message names to ids, per
//! direction. Message layouts are Rust types implementing [`Message`]; each names itself
//! and the opcode is looked up in the table.

pub mod entity;
pub mod instance;
pub mod login;
pub mod message;
pub mod opcodes;

pub use message::{Message, MessageError};
pub use opcodes::{Direction, OpcodeTable, Protocol, ProtocolError};
