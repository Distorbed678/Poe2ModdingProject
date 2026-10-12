//! Login key exchange (`KeyExchange` in both directions, then Salsa20).
//!
//! The client sends its 1024-bit DH public value and no signature. The official server
//! answers with its public value and a DSA signature. With the client DLL's `crypto_bypass`,
//! the client skips verification and derives its keys from `SHA-512(server public value)`,
//! so the server sends random bytes and no signature.

use poe_net::{Connection, KeyMaterial};
use poe_proto::{
    Protocol,
    login::{ClientKeyExchange, KeyExchange, ServerKeyExchange},
};
use rand::RngCore;
use tracing::{debug, info};

use crate::session::read_message;

/// Size of the DH public value the client expects (1024-bit group).
const PUBLIC_KEY_LEN: usize = 128;

/// Runs the key exchange and switches the connection to encrypted mode.
/// Returns `false` if the client disconnected first.
pub async fn perform(conn: &mut Connection, protocol: &Protocol) -> anyhow::Result<bool> {
    let Some(ClientKeyExchange(client)) = read_message::<ClientKeyExchange>(conn, protocol).await?
    else {
        return Ok(false);
    };
    debug!(
        key_len = client.public_key.len(),
        sig_len = client.signature.len(),
        "client key exchange"
    );

    let mut public_key = vec![0u8; PUBLIC_KEY_LEN];
    rand::rng().fill_bytes(&mut public_key);
    let reply = ServerKeyExchange(KeyExchange {
        public_key,
        signature: Vec::new(),
    });
    conn.send(&protocol.login.encode(&reply)?).await?;

    let keys = KeyMaterial::from_server_public_key(&reply.0.public_key);
    conn.set_ciphers(
        Box::new(keys.client_to_server()),
        Box::new(keys.server_to_client()),
    );
    info!("key exchange complete, stream encrypted");
    Ok(true)
}
