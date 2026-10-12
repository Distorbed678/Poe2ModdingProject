//! Login key exchange (`KeyExchange` in both directions, then Salsa20).
//!
//! Layout (both directions): `u16 opcode, u16 key_len, key[key_len], u16 sig_len,
//! sig[sig_len]`. The client sends its 1024-bit DH public value and no signature. The
//! official server answers with its public value and a DSA signature. With the client DLL's
//! `crypto_bypass`, the client skips verification and derives its keys from
//! `SHA-512(server public value)`, so the server sends random bytes and no signature.

use poe_net::{Connection, DecodeError, KeyMaterial, Reader, Writer};
use poe_proto::{Direction, Protocol};
use rand::RngCore;
use tracing::{debug, info};

/// Size of the DH public value the client expects (1024-bit group).
const PUBLIC_KEY_LEN: usize = 128;

pub struct KeyExchange {
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl KeyExchange {
    fn decode(r: &mut Reader<'_>, opcode: u16) -> Result<Self, DecodeError> {
        let at = r.position();
        if r.u16()? != opcode {
            return Err(DecodeError::Invalid {
                offset: at,
                what: "key exchange opcode",
            });
        }
        let len = r.u16()? as usize;
        let public_key = r.bytes(len)?.to_vec();
        let len = r.u16()? as usize;
        let signature = r.bytes(len)?.to_vec();
        Ok(Self {
            public_key,
            signature,
        })
    }

    fn encode(&self, opcode: u16) -> Vec<u8> {
        let mut w = Writer::new();
        w.u16(opcode)
            .u16(self.public_key.len() as u16)
            .bytes(&self.public_key)
            .u16(self.signature.len() as u16)
            .bytes(&self.signature);
        w.into_vec()
    }
}

/// Runs the key exchange and switches the connection to encrypted mode.
/// Returns `false` if the client disconnected first.
pub async fn perform(conn: &mut Connection, protocol: &Protocol) -> anyhow::Result<bool> {
    let client_op = protocol.login.id(Direction::Client, "KeyExchange")?;
    let server_op = protocol.login.id(Direction::Server, "KeyExchange")?;
    let Some(client) = conn.decode(|r| KeyExchange::decode(r, client_op)).await? else {
        return Ok(false);
    };
    debug!(
        key_len = client.public_key.len(),
        sig_len = client.signature.len(),
        "client key exchange"
    );

    let mut public_key = vec![0u8; PUBLIC_KEY_LEN];
    rand::rng().fill_bytes(&mut public_key);
    let reply = KeyExchange {
        public_key,
        signature: Vec::new(),
    };
    conn.send(&reply.encode(server_op)).await?;

    let keys = KeyMaterial::from_server_public_key(&reply.public_key);
    conn.set_ciphers(
        Box::new(keys.client_to_server()),
        Box::new(keys.server_to_client()),
    );
    info!("key exchange complete, stream encrypted");
    Ok(true)
}
