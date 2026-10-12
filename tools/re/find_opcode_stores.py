"""Find `mov word ptr [reg+8], imm16` stores (packet constructors set their opcode at +8).

Usage: Tools/pyenv/bin/python tools/re/find_opcode_stores.py [exe]
Prints hits grouped by 64 KiB region so constructor clusters stand out.
"""
import re
import sys
from collections import defaultdict

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image, DEFAULT_EXE

img = Image(sys.argv[1] if len(sys.argv) > 1 else DEFAULT_EXE)
text_rva, text = img.section(".text")
# 66 [REX.B] C7 /0 with disp8 = 08, register base (not rsp/rbp): modrm 0x40-0x47 except 44/45.
pattern = re.compile(rb"\x66(?:\x41)?\xc7[\x40-\x43\x46\x47]\x08(..)", re.S)
regions = defaultdict(list)
for m in pattern.finditer(text):
    va = img.va(text_rva + m.start())
    imm = int.from_bytes(m.group(1), "little")
    regions[va >> 16].append((va, imm))
for region, hits in sorted(regions.items(), key=lambda kv: -len(kv[1]))[:15]:
    imms = sorted({imm for _, imm in hits})
    print(f"{region << 16:#x}: {len(hits)} stores, imm range {imms[0]:#x}..{imms[-1]:#x}, sample {[hex(i) for i in imms[:12]]}")
