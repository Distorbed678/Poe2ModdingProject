//! Stream ciphers for PoE connections.
//!
//! The client encrypts each direction with Salsa20 (Crypto++, 20 rounds). Both directions use
//! the same 32-byte key and differ only in the 8-byte IV. Key and IVs come from 64 bytes of key
//! material (see [`KeyMaterial`]); `Documentation/re/crypto.md` has the details.

use salsa20::{
    Salsa20,
    cipher::{KeyIvInit, StreamCipher as _},
};
use sha2::{Digest, Sha512};

/// A keystream cipher applied in place. Encryption and decryption are the same operation.
pub trait StreamCipher: Send {
    fn apply(&mut self, data: &mut [u8]);
}

/// No encryption.
#[derive(Debug, Default, Clone, Copy)]
pub struct Plain;

impl StreamCipher for Plain {
    fn apply(&mut self, _data: &mut [u8]) {}
}

/// One direction of an encrypted connection.
pub struct Salsa20Stream(Salsa20);

impl Salsa20Stream {
    pub fn new(key: &[u8; 32], iv: &[u8; 8]) -> Self {
        Self(Salsa20::new(key.into(), iv.into()))
    }
}

impl StreamCipher for Salsa20Stream {
    fn apply(&mut self, data: &mut [u8]) {
        self.0.apply_keystream(data);
    }
}

/// 64 bytes of session key material: `key = [0..32]`, client→server IV `[32..40]`,
/// server→client IV `[48..56]`.
#[derive(Clone)]
pub struct KeyMaterial(pub [u8; 64]);

impl KeyMaterial {
    /// Login session keys with the client DLL's key-exchange bypass: `SHA-512` of the public
    /// key the server sent in its key-exchange reply.
    pub fn from_server_public_key(public_key: &[u8]) -> Self {
        Self(Sha512::digest(public_key).into())
    }

    fn key(&self) -> &[u8; 32] {
        self.0[..32].try_into().unwrap()
    }

    fn iv(&self, offset: usize) -> &[u8; 8] {
        self.0[offset..offset + 8].try_into().unwrap()
    }

    pub fn client_to_server(&self) -> Salsa20Stream {
        Salsa20Stream::new(self.key(), self.iv(32))
    }

    pub fn server_to_client(&self) -> Salsa20Stream {
        Salsa20Stream::new(self.key(), self.iv(48))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_use_distinct_keystreams() {
        let keys = KeyMaterial::from_server_public_key(&[7; 128]);
        let (mut a, mut b) = (vec![0u8; 16], vec![0u8; 16]);
        keys.client_to_server().apply(&mut a);
        keys.server_to_client().apply(&mut b);
        assert_ne!(a, b);
        // Decrypting with a fresh stream of the same direction restores the plaintext.
        keys.client_to_server().apply(&mut a);
        assert_eq!(a, vec![0u8; 16]);
    }
}
