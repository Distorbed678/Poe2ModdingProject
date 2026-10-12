//! Buffered TCP connection with per-direction stream ciphers.
//!
//! Because messages carry no length prefix, the protocol layer drives decoding: it inspects
//! [`Connection::buffer`], consumes what it could decode and calls [`Connection::read_more`]
//! when a message is incomplete.

use std::net::SocketAddr;

use bytes::{Buf, BytesMut};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing::trace;

use crate::{
    Plain,
    cipher::StreamCipher,
    codec::{DecodeError, Reader},
    hexdump::hexdump,
};

pub struct Connection {
    stream: TcpStream,
    peer: SocketAddr,
    inbound: BytesMut,
    recv_cipher: Box<dyn StreamCipher>,
    send_cipher: Box<dyn StreamCipher>,
}

impl Connection {
    pub fn new(stream: TcpStream) -> std::io::Result<Self> {
        let _ = stream.set_nodelay(true);
        let peer = stream.peer_addr()?;
        Ok(Self {
            stream,
            peer,
            inbound: BytesMut::with_capacity(16 * 1024),
            recv_cipher: Box::new(Plain),
            send_cipher: Box::new(Plain),
        })
    }

    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// Switches ciphers. Bytes already buffered stay as they were decrypted; only data
    /// received from now on uses the new cipher.
    pub fn set_ciphers(&mut self, recv: Box<dyn StreamCipher>, send: Box<dyn StreamCipher>) {
        self.recv_cipher = recv;
        self.send_cipher = send;
    }

    /// Decrypted bytes received but not yet consumed.
    pub fn buffer(&self) -> &[u8] {
        &self.inbound
    }

    pub fn consume(&mut self, len: usize) {
        self.inbound.advance(len);
    }

    /// Reads at least one more byte from the socket. Returns `Ok(0)` when the peer closed.
    pub async fn read_more(&mut self) -> std::io::Result<usize> {
        let start = self.inbound.len();
        self.inbound.reserve(8 * 1024);
        let n = self.stream.read_buf(&mut self.inbound).await?;
        if n > 0 {
            self.recv_cipher.apply(&mut self.inbound[start..]);
            trace!(peer = %self.peer, len = n, "recv\n{}", hexdump(&self.inbound[start..]));
        }
        Ok(n)
    }

    /// Decodes one message with `decode`, reading from the socket until it has enough bytes.
    ///
    /// `decode` must return [`DecodeError::Incomplete`] when the buffer ends early. Returns
    /// `Ok(None)` if the peer closed the connection before a full message arrived.
    pub async fn decode<T>(
        &mut self,
        mut decode: impl FnMut(&mut Reader<'_>) -> Result<T, DecodeError>,
    ) -> std::io::Result<Option<T>> {
        loop {
            let mut reader = Reader::new(&self.inbound);
            match decode(&mut reader) {
                Ok(value) => {
                    let used = reader.position();
                    self.consume(used);
                    return Ok(Some(value));
                }
                Err(DecodeError::Incomplete { .. }) => {
                    if self.read_more().await? == 0 {
                        return Ok(None);
                    }
                }
                Err(e) => {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                }
            }
        }
    }

    /// Encrypts (if a cipher is set) and writes a complete message.
    pub async fn send(&mut self, data: &[u8]) -> std::io::Result<()> {
        trace!(peer = %self.peer, len = data.len(), "send\n{}", hexdump(data));
        let mut out = data.to_vec();
        self.send_cipher.apply(&mut out);
        self.stream.write_all(&out).await
    }
}
