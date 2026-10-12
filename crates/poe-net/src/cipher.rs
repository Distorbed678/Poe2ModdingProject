//! Stream cipher abstraction.
//!
//! The official client encrypts both directions with a stream cipher (Salsa20 in PoE1-era
//! research). The client DLL bypasses or fixes the key exchange; which implementation a
//! connection uses is decided by the protocol layer, so the transport only sees this trait.

/// A keystream cipher applied in place. Encryption and decryption are the same operation.
pub trait StreamCipher: Send {
    fn apply(&mut self, data: &mut [u8]);
}

/// No encryption (the client DLL disabled it).
#[derive(Debug, Default, Clone, Copy)]
pub struct Plain;

impl StreamCipher for Plain {
    fn apply(&mut self, _data: &mut [u8]) {}
}
