"""List jump-table switches in a function and the opcodes each case handles.

Usage: find_switches.py <func_start> <length>   (hex)
Recognises MSVC `sub/add eax, base; cmp eax, n; ja; [movzx idx table]; mov ecx, [rdx+rax*4+T]`.
"""
import sys

import capstone
from capstone import x86

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image

start, length = int(sys.argv[1], 16), int(sys.argv[2], 16)
img = Image()
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
md.detail = True
ins = list(md.disasm(img.read(start, length), start))
for idx, i in enumerate(ins):
    if not (i.mnemonic == "jmp" and i.operands[0].type == x86.X86_OP_REG):
        continue
    window = ins[max(0, idx - 16) : idx]
    base = 0
    count = None
    byte_table = dword_table = None
    for p in window:
        if p.mnemonic in ("sub", "add") and p.operands[0].type == x86.X86_OP_REG and p.operands[1].type == x86.X86_OP_IMM and p.operands[0].size == 4:
            v = p.operands[1].imm
            base = v if p.mnemonic == "sub" else -v
        if p.mnemonic == "cmp" and p.operands[0].type == x86.X86_OP_REG and p.operands[0].size == 4 and p.operands[1].type == x86.X86_OP_IMM:
            count = p.operands[1].imm + 1
        if p.mnemonic == "movzx" and "byte ptr" in p.op_str:
            byte_table = img.base + p.operands[1].mem.disp
        if p.mnemonic == "mov" and "dword ptr" in p.op_str and p.operands[1].type == x86.X86_OP_MEM and p.operands[1].mem.scale == 4:
            dword_table = img.base + p.operands[1].mem.disp
    if count is None or dword_table is None:
        continue
    cases = {}
    for k in range(count):
        slot = img.read(byte_table + k, 1)[0] if byte_table else k
        target = img.base + int.from_bytes(img.read(dword_table + 4 * slot, 4), "little")
        cases.setdefault(target, []).append(base + k)
    # The most common target is the default case.
    default = max(cases, key=lambda t: len(cases[t]))
    print(f"switch at {i.address:#x}: base {base:#x}, {count} entries, default {default:#x}")
    for target, ops in sorted(cases.items(), key=lambda kv: kv[1][0]):
        if target != default:
            print(f"  {','.join(hex(o) for o in ops)} -> {target:#x}")
