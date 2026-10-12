"""Find code that constructs packet objects from vtables in a range, and the opcode it
stores at +8.

For every `lea reg, [rip+X]` with X in [vt_lo, vt_hi), disassembles a window around the
site and reports immediate values moved into 16/32-bit registers or word stores nearby
(the opcode candidates).

Usage: find_packet_senders.py <vt_lo> <vt_hi>   (hex)
"""
import re
import sys

import capstone

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image

vt_lo, vt_hi = int(sys.argv[1], 16), int(sys.argv[2], 16)
img = Image()
rva, text = img.section(".text")
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
lea = re.compile(rb"[\x48\x4c]\x8d[\x05\x0d\x15\x1d\x25\x2d\x35\x3d](....)", re.S)
imm_re = re.compile(r"^(?:mov|movw)\s*$")
results = []
for m in lea.finditer(text):
    site_rva = rva + m.start()
    target = img.va(site_rva + 7) + int.from_bytes(m.group(1), "little", signed=True)
    if not vt_lo <= target < vt_hi:
        continue
    site = img.va(site_rva)
    window = img.read(site - 0x40, 0x80)
    cands = []
    for ins in md.disasm(window, site - 0x40):
        if ins.mnemonic == "mov" and ("word ptr" in ins.op_str or ins.op_str.split(",")[0] in ("eax", "ecx", "edx", "r8d", "r9d", "ax", "cx", "dx")):
            parts = ins.op_str.split(", ")
            if len(parts) == 2 and parts[1].startswith("0x") or (len(parts) == 2 and parts[1].isdigit()):
                v = int(parts[1], 0)
                if 0 < v < 0x400:
                    cands.append(f"{ins.address:#x}:{v:#x}")
    results.append((target, site, cands))
for target, site, cands in sorted(results):
    print(f"vt {target:#x} site {site:#x} imm {cands}")
