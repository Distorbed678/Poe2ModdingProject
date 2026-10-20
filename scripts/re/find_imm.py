"""Find instructions with a given 32-bit immediate operand: find_imm.py <imm hex> [min max]."""
import sys

import capstone
from capstone import x86

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image

imm = int(sys.argv[1], 16)
lo, hi = (int(sys.argv[2], 16), int(sys.argv[3], 16)) if len(sys.argv) > 3 else (0, 1 << 64)
img = Image()
rva, text = img.section(".text")
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
md.detail = True
needle = imm.to_bytes(4, "little")
pos = text.find(needle)
seen = set()
while pos != -1:
    for back in range(1, 8):
        start = pos - back
        va = img.va(rva + start)
        if va in seen or not lo <= va < hi:
            continue
        ins = next(md.disasm(text[start : start + 15], va), None)
        if ins is None or ins.size < back + 4:
            continue
        if any(op.type == x86.X86_OP_IMM and op.imm == imm for op in ins.operands):
            seen.add(va)
            print(f"{va:#x} {ins.mnemonic} {ins.op_str}")
            break
    pos = text.find(needle, pos + 1)
