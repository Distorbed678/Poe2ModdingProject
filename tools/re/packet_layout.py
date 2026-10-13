"""Recover packet layouts from their serializers, without Ghidra.

Usage: packet_layout.py <opcode> [opcode ...]   (hex)

For each opcode:
1. finds constructors that store it at +8 (`mov word ptr [reg+8], imm16`);
2. takes the vtable stored at +0 nearby (`lea rax, [rip+X]` ... `mov [rcx], rax`);
3. disassembles vtable slot 1 (serializer, `this, stream`) and slot 2 (deserializer) and
   lists, in order, the stream calls with their size and source field:
   `write(+off, n)` for the raw writer `0x142197e60` (`n` from r8), byte swaps
   (`htons`/`htonl` imports), and other calls (nested serializers, varint writers).
Conditional jumps are listed too, so optional fields stand out. This is a linear sweep: read
the output alongside the disassembly when branches matter.
"""
import sys

import capstone
from capstone import x86

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image

WRITE = 0x142197E60
READ = 0x142197D30
KNOWN = {
    0x142197E60: "write",
    0x142197D30: "read",
    0x140872560: "write_varuint",
    0x1408719A0: "read_varuint",
    0x141CFB3C0: "read_wstring",
    0x141CFBFE0: "read_varint",
}

img = Image()
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
md.detail = True
text_rva, text = img.section(".text")
text_va = img.va(text_rva)
text_end = text_va + len(text)


def imports():
    """IAT slot -> name, for ws2_32 ordinals (htons/htonl/ntohs/ntohl)."""
    img.pe.parse_data_directories()
    names = {}
    for entry in getattr(img.pe, "DIRECTORY_ENTRY_IMPORT", []):
        for imp in entry.imports:
            name = imp.name.decode() if imp.name else f"{entry.dll.decode()}#{imp.ordinal}"
            names[imp.address] = name
    return names


IMPORTS = imports()
WS2 = {"WS2_32.dll#8": "htonl", "WS2_32.dll#9": "htons", "WS2_32.dll#14": "ntohl", "WS2_32.dll#15": "ntohs"}


def ctor_sites(opcode):
    """`66 [41] c7 4x 08 imm16`: mov word ptr [reg+8], imm16."""
    imm = opcode.to_bytes(2, "little")
    hits = []
    for rex in (b"", b"\x41"):
        for modrm in range(0x40, 0x48):
            if modrm in (0x44, 0x45) and not rex:
                continue
            pat = b"\x66" + rex + b"\xc7" + bytes([modrm, 0x08]) + imm
            start = 0
            while (i := text.find(pat, start)) >= 0:
                hits.append(text_va + i)
                start = i + 1
    return sorted(hits)


def vtable_near(site):
    """The vtable LEA within 0x30 bytes around the opcode store."""
    for back in range(0x30, 0, -1):
        code = img.read(site - back, back + 0x30)
        for ins in md.disasm(code, site - back):
            if ins.mnemonic == "lea" and ins.operands[1].type == x86.X86_OP_MEM and ins.operands[1].mem.base == x86.X86_REG_RIP:
                target = ins.address + ins.size + ins.operands[1].mem.disp
                if 0x142E89000 <= target < 0x143A06000:
                    return target
        break
    return None


def sweep(func, limit=0x600):
    out = []
    regs = {}
    for ins in md.disasm(img.read(func, limit), func):
        m = ins.mnemonic
        if m in ("mov", "lea") and ins.operands[0].type == x86.X86_OP_REG:
            dst = ins.reg_name(ins.operands[0].reg)
            src = ins.operands[1]
            if src.type == x86.X86_OP_IMM:
                regs[dst] = ("imm", src.imm)
            elif src.type == x86.X86_OP_MEM and src.mem.base != x86.X86_REG_RIP:
                regs[dst] = ("mem", ins.reg_name(src.mem.base), src.mem.disp)
            else:
                regs.pop(dst, None)
        if m == "call":
            op = ins.operands[0]
            if op.type == x86.X86_OP_IMM:
                tgt = op.imm
                name = KNOWN.get(tgt, f"call {tgt:#x}")
                size = regs.get("r8d") or regs.get("r8")
                src = regs.get("rdx")
                desc = name
                if tgt in (WRITE, READ) and size and size[0] == "imm":
                    where = f"[{src[1]}+{src[2]:#x}]" if src and src[0] == "mem" else "local"
                    desc = f"{name} {size[1]} bytes {where}"
                out.append(f"{ins.address:#x}  {desc}")
            elif op.type == x86.X86_OP_MEM and op.mem.base == x86.X86_REG_RIP:
                slot = ins.address + ins.size + op.mem.disp
                name = IMPORTS.get(slot, f"[{slot:#x}]")
                out.append(f"{ins.address:#x}  {WS2.get(name, name)}")
            else:
                out.append(f"{ins.address:#x}  call {ins.op_str}")
            regs = {}
        elif m.startswith("j") and m != "jmp":
            out.append(f"{ins.address:#x}  {m} {ins.op_str}")
        elif m == "jmp" and ins.operands[0].type == x86.X86_OP_IMM and not (func <= ins.operands[0].imm < func + limit):
            out.append(f"{ins.address:#x}  tail-call {ins.operands[0].imm:#x}")
            break
        elif m in ("ret", "int3"):
            break
    return out


def main():
    for arg in sys.argv[1:]:
        opcode = int(arg, 16)
        sites = ctor_sites(opcode)
        seen = set()
        print(f"### opcode {opcode:#06x}: {len(sites)} store sites")
        for site in sites:
            vt = vtable_near(site)
            if not vt or vt in seen:
                continue
            seen.add(vt)
            ser, de = img.u64(vt + 8), img.u64(vt + 16)
            if not (text_va <= ser < text_end and text_va <= de < text_end):
                continue
            print(f"  ctor store {site:#x} vtable {vt:#x} serialize {ser:#x} deserialize {de:#x}")
            for label, fn in (("serialize", ser), ("deserialize", de)):
                print(f"    {label}:")
                for line in sweep(fn):
                    print(f"      {line}")


if __name__ == "__main__":
    main()
