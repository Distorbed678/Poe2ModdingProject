"""Dump a packet factory switch: opcode -> case address, allocation size, vtable, deserializer.

Usage: dump_factory.py <switch_base_opcode hex> <count hex> <table_rva hex> <default_va hex> [out.json] [--bytes <byte_table_rva hex>]
`find_switches.py <func> <len>` prints the numbers (base, count, table rvas, default) of a function's
switches. With --bytes the switch has an index table (MSVC `movzx idx, [bytes+k]; mov [dwords+idx*4]`).
Output goes to dumps/0.5.5e.hotfix1/ghidra/ (git-ignored), e.g. instance_factory_s2c.json.
For the instance factory (FUN_142170500): dump_factory.py d 35f 217305c 142173040
The vtable is the first rip-relative LEA into .rdata within the case that is stored as a
pointer; vtable slot 2 is the deserializer (slot 1 the serializer).
"""
import json
import sys

import capstone
from capstone import x86

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image

args = sys.argv[1:]
bytes_rva = None
if "--bytes" in args:
    i = args.index("--bytes")
    bytes_rva = int(args[i + 1], 16)
    del args[i : i + 2]
base_op, count, table_rva, default = (int(x, 16) for x in args[:4])
out = args[4] if len(args) > 4 else None
img = Image()
rdata_lo = img.va(img.sections[".rdata"].VirtualAddress)
rdata_hi = rdata_lo + img.sections[".rdata"].Misc_VirtualSize
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
md.detail = True
rows = []
for i in range(count):
    slot = img.read(img.base + bytes_rva + i, 1)[0] if bytes_rva is not None else i
    rel = int.from_bytes(img.read(img.base + table_rva + 4 * slot, 4), "little")
    case = img.base + rel
    if case == default:
        continue
    size = vtable = ctor = None
    for ins in md.disasm(img.read(case, 0x80), case):
        if ins.mnemonic == "mov" and ins.op_str.startswith("ecx, ") and size is None:
            size = ins.operands[1].imm if ins.operands[1].type == x86.X86_OP_IMM else None
        if ins.mnemonic == "lea" and ins.operands[1].type == x86.X86_OP_MEM and ins.operands[1].mem.base == x86.X86_REG_RIP:
            tgt = ins.address + ins.size + ins.operands[1].mem.disp
            if rdata_lo <= tgt < rdata_hi and vtable is None:
                vtable = tgt
        if ins.mnemonic == "call" and size is not None and ctor is None and ins.operands[0].type == x86.X86_OP_IMM:
            if ins.operands[0].imm != 0x142bba600:
                ctor = ins.operands[0].imm
        if ins.mnemonic in ("jmp", "ret") or (vtable and ctor):
            break
    row = {"opcode": base_op + i, "case": case, "size": size, "vtable": vtable, "ctor": ctor}
    if vtable:
        row["serialize"] = img.u64(vtable + 8)
        row["deserialize"] = img.u64(vtable + 16)
    rows.append(row)
for r in rows:
    print(" ".join(f"{k}={v:#x}" if isinstance(v, int) else f"{k}={v}" for k, v in r.items()))
if out:
    with open(out, "w") as f:
        json.dump([{k: (hex(v) if isinstance(v, int) else v) for k, v in r.items()} for r in rows], f, indent=1)
    print(f"wrote {len(rows)} entries to {out}", file=sys.stderr)
