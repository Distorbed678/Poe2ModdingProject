//! Function fingerprints: a hash of a function's bytes with everything a rebuild moves
//! wildcarded, for the opcode drift check of `cargo xtask check-build`.
//!
//! A packet class's deserializer reads the message field by field, so its code changes when the
//! layout changes and stays the same (up to moved addresses) when it does not. The wildcarded
//! bytes are what a relink changes without changing the function: call and jump targets,
//! RIP-relative displacements, stack-frame displacements and sizes, jump-table and image
//! addresses. Everything else, in particular member offsets (`[rcx+0x44c]`), constants and the
//! instruction sequence, is part of the fingerprint.

use std::collections::HashMap;

use iced_x86::{Decoder, DecoderOptions, Instruction, OpKind, Register};
use sha2::{Digest, Sha256};

use poe_hook_sig::Image;

/// The mask of one function: its bytes with the moved parts zeroed.
pub fn masked(code: &[u8], begin_va: u64, image_base: u64, image_size: u64) -> Vec<u8> {
    let mut out = code.to_vec();
    let mut decoder = Decoder::with_ip(64, code, begin_va, DecoderOptions::NONE);
    let zero = |out: &mut Vec<u8>, at: usize, len: usize| {
        if let Some(s) = out.get_mut(at..at + len) {
            s.fill(0);
        }
    };
    while decoder.can_decode() {
        let pos = decoder.position();
        let instr = decoder.decode();
        if instr.is_invalid() {
            // Data in the code stream (or a prefix we do not know): keep the bytes, move on.
            continue;
        }
        let offsets = decoder.get_constant_offsets(&instr);
        let mut wild_imm = false;
        let mut wild_disp = false;
        for op in 0..instr.op_count() {
            match instr.op_kind(op) {
                OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
                    wild_imm = true
                }
                OpKind::Memory => {
                    let base = instr.memory_base();
                    let index = instr.memory_index();
                    let disp = instr.memory_displacement64();
                    if instr.is_ip_rel_memory_operand()
                        || matches!(
                            base,
                            Register::RSP | Register::RBP | Register::ESP | Register::EBP
                        )
                    {
                        wild_disp = true;
                    } else if index != Register::None && disp < image_size && disp >= 0x1000 {
                        // `mov ecx, [rdx+rax*4+RVA]`: a jump table in the image.
                        wild_disp = true;
                    } else if base == Register::None
                        && index == Register::None
                        && (image_base..image_base + image_size).contains(&disp)
                    {
                        wild_disp = true;
                    }
                }
                OpKind::Immediate8
                | OpKind::Immediate8_2nd
                | OpKind::Immediate16
                | OpKind::Immediate32
                | OpKind::Immediate64
                | OpKind::Immediate8to16
                | OpKind::Immediate8to32
                | OpKind::Immediate8to64
                | OpKind::Immediate32to64 => {
                    if instr.op0_kind() == OpKind::Register
                        && matches!(instr.op0_register(), Register::RSP | Register::RBP)
                    {
                        // sub rsp, N / add rsp, N: the frame size.
                        wild_imm = true;
                    } else if in_image(&instr, op, image_base, image_size) {
                        wild_imm = true;
                    }
                }
                _ => {}
            }
        }
        if wild_imm && offsets.has_immediate() {
            zero(
                &mut out,
                pos + offsets.immediate_offset(),
                offsets.immediate_size(),
            );
        }
        if wild_disp && offsets.has_displacement() {
            zero(
                &mut out,
                pos + offsets.displacement_offset(),
                offsets.displacement_size(),
            );
        }
    }
    out
}

fn in_image(instr: &Instruction, op: u32, base: u64, size: u64) -> bool {
    let v = instr.immediate(op);
    // 64-bit and sign-extended 32-bit immediates that point into the image.
    (base..base + size).contains(&v)
        || (v < size && v >= 0x10_0000 && instr.op_kind(op) == OpKind::Immediate32)
}

/// 16 hex digits of the SHA-256 of the masked bytes and the length.
pub fn digest(masked: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update((masked.len() as u32).to_le_bytes());
    h.update(masked);
    hex::encode(&h.finalize()[..8])
}

/// Every `.pdata` function of the image: `(begin rva, end rva)`.
pub fn functions(image: &Image) -> Vec<(u32, u32)> {
    let (rva, size) = image.info.exception_dir;
    let Some(raw) = image.read(u64::from(rva), size as usize) else {
        return Vec::new();
    };
    raw.chunks_exact(12)
        .map(|e| {
            (
                u32::from_le_bytes(e[0..4].try_into().unwrap()),
                u32::from_le_bytes(e[4..8].try_into().unwrap()),
            )
        })
        .filter(|(b, e)| e > b)
        .collect()
}

/// Fingerprint of the function at `rva` (its whole `.pdata` extent), with its length.
pub fn of_function(image: &Image, begin: u32, end: u32) -> Option<(String, u32)> {
    let code = image.read(u64::from(begin), (end - begin) as usize)?;
    let m = masked(
        code,
        image.info.image_base + u64::from(begin),
        image.info.image_base,
        u64::from(image.info.size_of_image),
    );
    Some((digest(&m), end - begin))
}

/// Functions shorter than this (thunks, stubs) are identical everywhere and have no fingerprint.
pub const MIN_LEN: u64 = 16;

/// All function fingerprints of the image: fingerprint -> rvas of the functions that have it.
pub fn all(image: &Image) -> HashMap<String, Vec<u32>> {
    let mut map: HashMap<String, Vec<u32>> = HashMap::new();
    for (b, e) in functions(image) {
        // Tiny functions (thunks, stubs) are identical everywhere and say nothing.
        if u64::from(e - b) < MIN_LEN {
            continue;
        }
        if let Some((fp, _)) = of_function(image, b, e) {
            map.entry(fp).or_default().push(b);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 0x1_4000_0000;
    const SIZE: u64 = 0x100_0000;

    #[test]
    fn a_relink_does_not_change_the_fingerprint_but_a_layout_change_does() {
        // push rbx; sub rsp,0x20; mov eax,[rcx+0x44c]; call rel32; lea rdx,[rip+disp32]; add rsp,0x20; pop rbx; ret
        let f = |frame: u8, member: u8, call: u8, rip: u8| {
            vec![
                0x53, 0x48, 0x83, 0xec, frame, 0x8b, 0x81, member, 0x04, 0x00, 0x00, 0xe8, call, 0,
                0, 0, 0x48, 0x8d, 0x15, rip, 0x10, 0, 0, 0x48, 0x83, 0xc4, frame, 0x5b, 0xc3,
            ]
        };
        let a = digest(&masked(
            &f(0x20, 0x4c, 0x10, 0x20),
            BASE + 0x1000,
            BASE,
            SIZE,
        ));
        // Frame size, call target and RIP displacement moved: same fingerprint.
        let moved = digest(&masked(
            &f(0x38, 0x4c, 0x77, 0x31),
            BASE + 0x2000,
            BASE,
            SIZE,
        ));
        assert_eq!(a, moved);
        // A member offset changed (the layout): different.
        let changed = digest(&masked(
            &f(0x20, 0x50, 0x10, 0x20),
            BASE + 0x1000,
            BASE,
            SIZE,
        ));
        assert_ne!(a, changed);
    }

    #[test]
    fn image_addresses_in_immediates_are_wildcarded() {
        // mov rax, imm64 (image address); ret
        let f = |v: u64| {
            let mut c = vec![0x48, 0xb8];
            c.extend_from_slice(&v.to_le_bytes());
            c.push(0xc3);
            c
        };
        let a = digest(&masked(&f(BASE + 0x1234), BASE, BASE, SIZE));
        let b = digest(&masked(&f(BASE + 0x4321), BASE, BASE, SIZE));
        assert_eq!(a, b);
        // A plain constant is part of the function.
        assert_ne!(
            digest(&masked(&f(5), BASE, BASE, SIZE)),
            digest(&masked(&f(6), BASE, BASE, SIZE))
        );
    }
}
