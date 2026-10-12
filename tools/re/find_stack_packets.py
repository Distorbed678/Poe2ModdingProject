"""Find stack-constructed packets: `mov word ptr [rsp/rbp+d], imm16` (the opcode at +8 of a
packet object), optionally restricted to an opcode list and an address range.

Usage: find_stack_packets.py <min_va> <max_va> [opcode ...]   (hex values)
"""
import re
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image

lo, hi = int(sys.argv[1], 16), int(sys.argv[2], 16)
wanted = {int(x, 16) for x in sys.argv[3:]}
img = Image()
rva, text = img.section(".text")
# 66 C7 44 24 d8 imm16  (rsp+disp8) | 66 C7 45 d8 imm16 (rbp+disp8) | 66 C7 85 d32 imm16 (rbp+disp32)
pat = re.compile(rb"\x66\xc7(?:\x44\x24(.)|\x45(.)|\x85(....))(..)", re.S)
for m in pat.finditer(text):
    va = img.va(rva + m.start())
    if not lo <= va < hi:
        continue
    imm = int.from_bytes(m.group(4), "little")
    if wanted and imm not in wanted:
        continue
    print(hex(va), hex(imm))
