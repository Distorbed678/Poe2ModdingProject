"""Signature generator for modules/client-hook/signatures/<version>.toml (run with third_party/pyenv).

Bootstraps (and re-bootstraps, for a new client build) the signature table from a spec of
Ghidra addresses: for every function the shortest instruction prefix that is unique in the code
sections with wildcards over everything a rebuild moves (call/jump targets, RIP-relative
displacements, stack-frame sizes and offsets, image addresses); for mid-function sites and
referenced addresses a pattern unique inside the anchor function; for struct offsets a pattern
around the instruction with the displacement as a capture.

    sig_gen.py <spec.py> [exe] > signatures/<version>.toml

The Rust side (`cargo run --release -p poe-hook-sig --example resolve -- <exe>`) re-checks every
result with the matcher the DLL uses. See docs/re/update-compatibility.md.
"""
import bisect
import importlib.util
import re
import sys

import capstone
from capstone import x86

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from pe import Image

MIN_TOTAL = 24      # pattern length in bytes for a function
MIN_FIXED = 16      # bytes that must match
STACK = (x86.X86_REG_RSP, x86.X86_REG_RBP)


class Gen:
    def __init__(self, exe=None):
        self.img = Image(exe) if exe else Image()
        self.base = self.img.base
        rva, text = self.img.section(".text")
        self.text_rva, self.text = rva, text
        self.md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
        self.md.detail = True
        d = self.img.pe.OPTIONAL_HEADER.DATA_DIRECTORY[3]
        raw = self.img.data[d.VirtualAddress : d.VirtualAddress + d.Size]
        self.funcs = sorted(
            (int.from_bytes(raw[i : i + 4], "little"), int.from_bytes(raw[i + 4 : i + 8], "little"))
            for i in range(0, len(raw) - 11, 12)
        )
        self.starts = [b for b, _ in self.funcs]

    # ---- helpers ----
    def func_range(self, va):
        rva = va - self.base
        i = bisect.bisect_right(self.starts, rva) - 1
        if i < 0:
            return None
        b, e = self.funcs[i]
        return (b, e) if b <= rva < e else None

    def is_func_start(self, va):
        r = self.func_range(va)
        return bool(r) and r[0] == va - self.base

    def insns(self, va, end_va=None):
        """Decoded instructions from va to the end of its .pdata function (or end_va)."""
        if end_va is None:
            r = self.func_range(va)
            end_va = self.base + r[1] if r else va + 0x400
        data = self.img.read(va, end_va - va)
        return list(self.md.disasm(data, va))

    def wild(self, ins, capture=None):
        """Per-byte tokens of an instruction: hex strings, '??', or a capture token.
        capture = ('disp', name) captures the displacement bytes; ('rip', name) the rip target."""
        raw = bytes(ins.bytes)
        tok = ["%02x" % b for b in raw]
        w = []  # (offset, size)
        ops = ins.operands
        if (ins.group(capstone.CS_GRP_JUMP) or ins.group(capstone.CS_GRP_CALL)) and ops and ops[0].type == x86.X86_OP_IMM:
            w.append((ins.imm_offset, ins.imm_size))
        for op in ops:
            if op.type == x86.X86_OP_MEM:
                base = op.mem.base
                if base == x86.X86_REG_RIP and ins.disp_size:
                    w.append((ins.disp_offset, ins.disp_size))
                elif base in STACK and ins.disp_size == 4:
                    w.append((ins.disp_offset, ins.disp_size))
                elif ins.mnemonic == "lea" and base in STACK and ins.disp_size:
                    w.append((ins.disp_offset, ins.disp_size))
        if ins.mnemonic in ("sub", "add") and ops and ops[0].type == x86.X86_OP_REG and ops[0].reg == x86.X86_REG_RSP and ins.imm_size == 4:
            w.append((ins.imm_offset, ins.imm_size))
        if ins.imm_size == 8 and ops and ops[-1].type == x86.X86_OP_IMM and 0x140000000 <= (ops[-1].imm & 0xFFFFFFFFFFFF) < 0x150000000:
            w.append((ins.imm_offset, ins.imm_size))
        for off, size in w:
            for k in range(off, off + size):
                tok[k] = "??"
        if capture:
            kind, name = capture
            if kind == "disp":
                off, size = ins.disp_offset, ins.disp_size
                tok[off : off + size] = ["<%s:%d>" % (name, size)] + [None] * (size - 1)
            else:
                off = ins.disp_offset if ins.disp_size else ins.imm_offset
                extra = len(raw) - (off + 4)
                tok[off : off + 4] = ["<%s:rip4%s>" % (name, "+%d" % extra if extra else "")] + [None] * 3
            tok = [t for t in tok if t is not None]
        return tok

    @staticmethod
    def regex(tokens):
        out = b""
        i = 0
        for t in tokens:
            if t == "??":
                out += b"."
            elif t.startswith("<"):
                n = int(t[1:-1].split(":")[1].replace("rip4", "4").split("+")[0])
                out += b"." * n
            else:
                out += re.escape(bytes([int(t, 16)]))
        return re.compile(out, re.DOTALL)

    @staticmethod
    def tlen(tokens):
        n = 0
        for t in tokens:
            if t.startswith("<"):
                n += int(t[1:-1].split(":")[1].replace("rip4", "4").split("+")[0])
            else:
                n += 1
        return n

    @staticmethod
    def fixed(tokens):
        return sum(1 for t in tokens if t != "??" and not t.startswith("<"))

    def count_global(self, tokens, limit=3):
        rx = self.regex(tokens)
        n, first = 0, []
        for m in rx.finditer(self.text):
            n += 1
            if len(first) < limit:
                first.append(self.text_rva + m.start())
            if n > 2000:
                break
        return n, first

    # ---- patterns ----
    def function_pattern(self, va, min_total=MIN_TOTAL, min_fixed=MIN_FIXED, max_insns=200):
        """Shortest unique prefix of the function at va."""
        ins = self.insns(va)
        toks = []
        cand = None
        for k, i in enumerate(ins[:max_insns]):
            toks += self.wild(i)
            if self.tlen(toks) < min_total or self.fixed(toks) < min_fixed:
                continue
            if cand is None:
                cand = self._all(toks)
            else:
                rx = self.regex(toks)
                cand = [p for p in cand if rx.match(self.text, p - self.text_rva)]
            if len(cand) == 1:
                assert cand[0] == va - self.base, (hex(va), hex(cand[0] + self.base))
                return toks
            if len(cand) == 0:
                return None
        return None

    def _all(self, toks):
        rx = self.regex(toks)
        return [self.text_rva + m.start() for m in rx.finditer(self.text)]

    def local_matches(self, tokens, begin_rva, end_rva):
        data = self.img.data[begin_rva:end_rva]
        rx = self.regex(tokens)
        return [begin_rva + m.start() for m in rx.finditer(data)]

    def local_pattern(self, anchor_va, start_va, capture=None, min_fixed=6, forward_only=False,
                      captures=None, span=None):
        """Unique pattern inside the function containing anchor_va, around the instruction at
        start_va: grows forward and (unless forward_only) backward alternately. `captures` is
        {instruction address: (kind, name)} (all inside the window; `capture` is the shorthand for
        start_va). `span` = extra instruction addresses the window must include. Returns
        (tokens, address of the first instruction)."""
        b, e = self.func_range(anchor_va)
        ins = self.insns(self.base + b)
        index = {i.address: k for k, i in enumerate(ins)}
        caps = dict(captures or {})
        if capture:
            caps[start_va] = capture
        need = [index[start_va]] + [index[a] for a in caps] + [index[a] for a in (span or [])]
        lo, hi = min(need), max(need)
        turn = 0
        while True:
            toks = []
            for k in range(lo, hi + 1):
                toks += self.wild(ins[k], caps.get(ins[k].address))
            if self.fixed(toks) >= min_fixed and len(self.local_matches(toks, b, e)) == 1:
                return toks, ins[lo].address
            can_f, can_b = hi + 1 < len(ins), lo > 0 and not forward_only
            if not (can_f or can_b):
                return None
            if (turn % 2 == 0 and can_f) or not can_b:
                hi += 1
            else:
                lo -= 1
            turn += 1

    # ---- field and ref finders ----
    def find_disp(self, func_va, disp):
        out = []
        for i in self.insns(func_va):
            for op in i.operands:
                if op.type == x86.X86_OP_MEM and op.mem.disp == disp and op.mem.base != x86.X86_REG_RIP and i.disp_size:
                    out.append(i)
                    break
        return out

    def find_ref(self, func_va, target_va):
        out = []
        for i in self.insns(func_va):
            t = None
            for op in i.operands:
                if op.type == x86.X86_OP_MEM and op.mem.base == x86.X86_REG_RIP:
                    t = i.address + i.size + op.mem.disp
                elif op.type == x86.X86_OP_IMM and (i.group(capstone.CS_GRP_JUMP) or i.group(capstone.CS_GRP_CALL)):
                    t = op.imm
            if t == target_va:
                out.append(i)
        return out


def load_spec(path):
    s = importlib.util.spec_from_file_location("spec", path)
    m = importlib.util.module_from_spec(s)
    s.loader.exec_module(m)
    return m


def toml_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def build(spec, exe=None):
    g = Gen(exe)
    b = spec.BUILD
    out = ["# Signature table for client %s. Generated by scripts/re/sig_gen.py from" % b["version"],
           "# scripts/re/%s; read by poe-hook-sig (the DLLs and `cargo xtask check-build`)." % spec.SPEC_NAME,
           "# Format and rules: modules/client-hook/poe-hook-sig/src/table.rs, docs/architecture/client-dll.md.",
           "", "[build]"]
    for k, v in b.items():
        out.append("%s = %s" % (k, toml_str(v) if isinstance(v, str) else hex(v)))
    addr = {s["name"]: s["addr"] for s in spec.SIGS}
    problems = []
    for s in spec.SIGS:
        name, va = s["name"], s["addr"]
        kind = s.get("kind", "function")
        out += ["", "[[sig]]", "name = %s" % toml_str(name)]
        out.append("features = [%s]" % ", ".join(toml_str(f) for f in s.get("features", [])))
        if kind != "function":
            out.append("kind = %s" % toml_str(kind))
        if kind == "function":
            toks = g.function_pattern(va)
            if toks is None:
                problems.append("%s %#x: no unique function prefix" % (name, va))
                out.append("# NO UNIQUE PATTERN")
                out.append("pattern = \"\"")
            else:
                out.append("pattern = %s" % toml_str(" ".join(toks)))
            if not g.is_func_start(va) and s.get("pdata", True):
                problems.append("%s %#x is not a .pdata function start" % (name, va))
        elif kind == "site":
            anchor = s["anchor"]
            r = g.local_pattern(addr[anchor], va, forward_only=True)
            if r is None:
                problems.append("%s %#x: no site pattern" % (name, va))
                out.append("pattern = \"\"")
            else:
                toks, first = r
                assert first == va
                out.append("anchor = %s" % toml_str(anchor))
                out.append("pattern = %s" % toml_str(" ".join(toks)))
                first_ins = g.insns(va)[0]
                s.setdefault("expect", " ".join(g.wild(first_ins)))
        elif kind == "ref":
            anchor = s.get("anchor")
            site_funcs = addr[anchor]
            cands = g.find_ref(site_funcs, va)
            if not cands:
                problems.append("%s: no reference to %#x in %s" % (name, va, anchor))
                out.append("pattern = \"\"")
            else:
                ins = cands[s.get("index", 0)]
                r = g.local_pattern(site_funcs, ins.address, capture=("rip", "t"), min_fixed=s.get("min_fixed", 6))
                if r is None:
                    problems.append("%s: no unique reference pattern" % name)
                    out.append("pattern = \"\"")
                else:
                    out.append("anchor = %s" % toml_str(anchor))
                    out.append("pattern = %s" % toml_str(" ".join(r[0])))
                    out.append('capture = "t"')
        if s.get("expect"):
            out.append("expect = %s" % toml_str(s["expect"]))
        elif s.get("expect_fn"):
            ins = g.insns(va)
            toks, n = [], 0
            for i in ins:
                toks += g.wild(i)
                if len(toks) >= 6:
                    break
            out.append("expect = %s" % toml_str(" ".join(toks)))
        if s.get("patch"):
            out.append("patch = %s" % toml_str(s["patch"]))
        if kind == "function" and s.get("pdata", True) and g.is_func_start(va):
            out.append("pdata = true")
        out.append("ghidra = %s" % hex(va))
        if s.get("note"):
            out.append("note = %s" % toml_str(s["note"]))
    for grp in spec.FIELDS:
        anchor = grp["anchor"]
        func = grp.get("func", addr[anchor])
        items = grp["items"]
        picked = []
        for f in items:
            cands = g.find_disp(func, f["disp"])
            if not cands:
                problems.append("field %s: no instruction with disp %#x in %#x" % (f["name"], f["disp"], func))
                continue
            if "at" in f:
                hit = [c for c in cands if c.address == f["at"]]
                if not hit:
                    problems.append("field %s: no instruction with disp %#x at %#x" % (f["name"], f["disp"], f["at"]))
                    continue
                picked.append((f, hit[0], len(cands)))
            else:
                picked.append((f, cands[f.get("index", 0)], len(cands)))
        if len(picked) != len(items):
            for f in items:
                out += ["", "[[field]]", "name = %s" % toml_str(f["name"]), 'pattern = ""', 'capture = "v"']
            continue
        capname = lambda f: f["name"].replace(".", "_")
        caps = {ins.address: ("disp", capname(f)) for f, ins, _ in picked}
        if len(caps) != len(picked):
            problems.append("fields %s share an instruction" % [f["name"] for f, _, _ in picked])
        r = g.local_pattern(func, picked[0][1].address, captures=caps, min_fixed=grp.get("min_fixed", 8))
        for f, ins, n in picked:
            out += ["", "[[field]]", "name = %s" % toml_str(f["name"]),
                    "features = [%s]" % ", ".join(toml_str(x) for x in grp.get("features", []))]
            if r is None:
                problems.append("field %s: no unique pattern" % f["name"])
                out.append('pattern = ""')
                out.append('capture = "v"')
                continue
            out.append("anchor = %s" % toml_str(anchor))
            out.append("pattern = %s" % toml_str(" ".join(r[0])))
            out.append("capture = %s" % toml_str(capname(f)))
            out.append("value = %s" % hex(f["disp"]))
            for k in ("min", "max", "align"):
                if k in f:
                    out.append("%s = %s" % (k, hex(f[k])))
            if f.get("agrees_with"):
                out.append("agrees_with = %s" % toml_str(f["agrees_with"]))
            if f.get("note"):
                out.append("note = %s" % toml_str(f["note"]))
            if n > 1:
                out.append("# %d instructions in %s use this displacement; the one at %#x is used" % (n, anchor, ins.address))
    return "\n".join(out) + "\n", problems


if __name__ == "__main__":
    spec = load_spec(sys.argv[1])
    text, problems = build(spec, sys.argv[2] if len(sys.argv) > 2 else None)
    sys.stdout.write(text)
    for p in problems:
        sys.stderr.write("PROBLEM: " + p + "\n")
    sys.exit(1 if problems else 0)
