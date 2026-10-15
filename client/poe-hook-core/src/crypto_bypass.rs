//! Login key-exchange bypass.
//!
//! The official login handshake (client → `0x0002` ephemeral DH public key, server →
//! `0x0002` public key + DSA signature) ends in the "verify and derive" routine (Ghidra
//! `0x14219f010` in 0.5.5e). It checks the server signature against GGG's key ("General
//! Rhoa Failure #3"), runs a unified Diffie-Hellman agreement ("#1") and writes
//! `SHA-512(agreed value)` into the session's 64-byte key buffer. The Salsa20 key and IVs
//! are taken from that buffer.
//!
//! This feature replaces the routine: the key buffer becomes `SHA-512(server public key
//! bytes as received)`, which the private server can compute too, so no signature or
//! agreement is needed and every session still gets its own keys.
//! See Documentation/re/crypto.md.

use std::sync::OnceLock;

use ilhook::x64::{HookType, Registers};
use sha2::{Digest, Sha512};

use poe_hook_shared::module::Module;

/// Prologue of the verify-and-derive routine (unique in 0.5.5e).
const VERIFY_AND_DERIVE: &str = "40 55 53 56 57 41 54 41 56 41 57 48 8d ac 24 70 fe ff ff \
    48 81 ec 90 02 00 00 4d 8b f0 48 8b f2 4c 8b e1 33 db";

/// Offset of the pointer to the 64-byte key buffer inside the crypto session object.
const SESSION_KEY_BUFFER: usize = 0x60;

static MAIN: OnceLock<Module> = OnceLock::new();

pub fn init(main: &Module) -> Result<(), String> {
    MAIN.set(*main).ok();
    let hits = main.scan(VERIFY_AND_DERIVE);
    let [addr] = hits[..] else {
        return Err(format!(
            "verify-and-derive pattern matched {} times",
            hits.len()
        ));
    };
    log::info!("verify-and-derive at ghidra {:#x}", main.to_ghidra(addr));
    crate::hooks::install(addr, HookType::Retn(on_verify_and_derive), 0)
        .map_err(|e| format!("hook: {e}"))?;
    Ok(())
}

/// MSVC `std::string` (32 bytes): inline buffer or heap pointer, size, capacity.
unsafe fn msvc_string_bytes<'a>(s: *const u8) -> &'a [u8] {
    unsafe {
        let size = *(s.add(0x10) as *const usize);
        let capacity = *(s.add(0x18) as *const usize);
        let data = if capacity > 15 {
            *(s as *const *const u8)
        } else {
            s
        };
        std::slice::from_raw_parts(data, size)
    }
}

/// Replacement for `verify_and_derive(session, &server_public_key, &signature)`.
unsafe extern "win64" fn on_verify_and_derive(
    regs: *mut Registers,
    original: usize,
    _: usize,
) -> usize {
    let _guard = crate::hooks::enter();
    let r = unsafe { &*regs };
    // Official servers (read-only): the real handshake.
    if crate::host::read_only() {
        type VerifyFn = unsafe extern "win64" fn(u64, u64, u64) -> usize;
        let f: VerifyFn = unsafe { std::mem::transmute(original) };
        return unsafe { f(r.rcx, r.rdx, r.r8) };
    }
    let session = r.rcx as *const u8;
    let public_key = unsafe { msvc_string_bytes(r.rdx as *const u8) };
    let out = unsafe { *(session.add(SESSION_KEY_BUFFER) as *const *mut u8) };
    let digest = Sha512::digest(public_key);
    unsafe { std::ptr::copy_nonoverlapping(digest.as_ptr(), out, 64) };
    log::info!(
        "login key exchange bypassed: server key {} bytes, key material {:02x?}..",
        public_key.len(),
        &digest[..8]
    );
    0
}
