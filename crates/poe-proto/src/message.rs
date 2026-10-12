//! Message trait and opcode-prefixed encoding.

use poe_net::{DecodeError, Reader, Writer};

use crate::{Direction, OpcodeTable, ProtocolError};

/// A message body. The opcode is not part of the body; it comes from the opcode table by
/// [`Message::NAME`].
pub trait Message: Sized {
    /// Name in `protocol/<version>/<channel>.toml`.
    const NAME: &'static str;
    /// Which side sends it.
    const DIRECTION: Direction;

    fn encode(&self, w: &mut Writer);
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError>;
}

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

impl OpcodeTable {
    /// Opcode followed by the encoded body.
    pub fn encode<M: Message>(&self, message: &M) -> Result<Vec<u8>, ProtocolError> {
        let mut w = Writer::new();
        w.u16(self.id(M::DIRECTION, M::NAME)?);
        message.encode(&mut w);
        Ok(w.into_vec())
    }

    /// Decodes the body of `M` after its opcode has been read.
    pub fn decode_body<M: Message>(&self, r: &mut Reader<'_>) -> Result<M, DecodeError> {
        M::decode(r)
    }
}
