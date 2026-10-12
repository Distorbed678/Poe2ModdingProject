"""Disassemble a range: disasm.py <start_va> <length> (hex)."""
import sys

import capstone

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image

start, length = int(sys.argv[1], 16), int(sys.argv[2], 16)
img = Image()
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
for i in md.disasm(img.read(start, length), start):
    print(f"{i.address:#x} {i.mnemonic} {i.op_str}")
