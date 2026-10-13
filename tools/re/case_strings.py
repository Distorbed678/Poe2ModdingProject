"""For switch cases, print the ASCII/UTF-16 strings referenced near each case target.

Usage: case_strings.py <func_start> <len> [window]   (hex) — uses find_switches logic.
"""
import subprocess
import sys

import capstone
from capstone import x86

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image

start, length = sys.argv[1], sys.argv[2]
window = int(sys.argv[3], 16) if len(sys.argv) > 3 else 0x180
out = subprocess.run(
    [sys.executable, __file__.rsplit("/", 1)[0] + "/find_switches.py", start, length],
    capture_output=True, text=True,
).stdout
img = Image()
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
md.detail = True
rd = img.sections[".rdata"]
lo = img.va(rd.VirtualAddress)
hi = lo + rd.Misc_VirtualSize


def string_at(va):
    raw = img.read(va, 96)
    a = raw.split(b"\0")[0]
    if len(a) >= 4 and all(32 <= c < 127 for c in a):
        return a.decode()
    try:
        w = raw.decode("utf-16le").split("\0")[0]
        if len(w) >= 4 and w.isprintable():
            return "L" + w
    except UnicodeDecodeError:
        pass
    return None


for line in out.splitlines():
    if "->" not in line:
        continue
    ops, target = line.strip().split(" -> ")
    t = int(target, 16)
    strings = []
    for ins in md.disasm(img.read(t, window), t):
        for op in ins.operands:
            if op.type == x86.X86_OP_MEM and op.mem.base == x86.X86_REG_RIP:
                va = ins.address + ins.size + op.mem.disp
                if lo <= va < hi:
                    s = string_at(va)
                    if s and s not in strings:
                        strings.append(s)
        if ins.mnemonic == "jmp" and ins.op_str.startswith("0x"):
            break
    print(f"{ops:>12} {target}: {strings[:4]}")
