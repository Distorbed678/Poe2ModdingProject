"""Rewrite the client addresses in text files (docs, code comments) from one build to a rebuilt one.

Usage: convert_addrs.py <old exe> <new exe> <addr_map.tsv> [--skip <file>:<first>-<last> ...]
                        [--dry-run] <file> ...

Rewrites `0x14xxxxxxx`, `FUN_14xxxxxxx`, `DAT_`, `LAB_`, `PTR_` and `thunk_FUN_` forms in place
(hex case kept). The map comes from `addr_map.py --out` (functions of a hotfix rebuild move by
deltas):
- code inside a `same` function moves with that function; the start of a `gap` function (a
  changed body) moves to the paired function; code between `.pdata` functions (padding, leaf
  functions without unwind data) moves when both neighbours have the same delta;
- data (`.rdata`, `.data`): the old code's RIP-relative references to the address are found, the
  same instruction in the moved function is read in the new exe, and its target is the new
  address (all references must agree); else a unique match of the old bytes near the address.
Addresses it cannot place are left as they are and listed (`--skip` keeps line ranges that are
already in the new build's addresses). Used for 0.5.5e -> 0.5.5e Hotfix 1 (2026-10-29).
"""
import argparse
import bisect
import re
import sys
from collections import Counter, defaultdict

import numpy as np
import pefile

BASE = 0x140000000
ADDR = re.compile(r"(?<![0-9A-Za-z_])(0x|thunk_FUN_|FUN_|DAT_|LAB_|PTR_)(14[0-9a-fA-F]{7})(?![0-9a-fA-F])")


class Exe:
    def __init__(self, path):
        pe = pefile.PE(path, fast_load=True)
        self.data = pe.__data__
        self.sections = []
        for s in pe.sections:
            va = BASE + s.VirtualAddress
            self.sections.append((s.Name.rstrip(b"\0").decode(), va, s.Misc_VirtualSize,
                                  s.PointerToRawData, s.SizeOfRawData))
        self.image_end = max(va + size for _, va, size, _, _ in self.sections)

    def section(self, va):
        for s in self.sections:
            if s[1] <= va < s[1] + s[2]:
                return s
        return None

    def read(self, va, n):
        s = self.section(va)
        if not s or va - s[1] + n > s[4]:
            return None
        off = s[3] + va - s[1]
        return bytes(self.data[off:off + n])

    def text(self):
        s = next(s for s in self.sections if s[0] == ".text")
        return s[1], np.frombuffer(self.data, dtype=np.uint8, count=min(s[2], s[4]), offset=s[3])


class Mapper:
    def __init__(self, old, new, tsv):
        self.old, self.new = old, new
        self.rows = []
        with open(tsv) as f:
            next(f)
            for line in f:
                ova, nva, osz, _nsz, kind = line.rstrip("\n").split("\t")
                self.rows.append((int(ova, 16), int(osz), None if nva == "-" else int(nva, 16), kind))
        self.starts = [r[0] for r in self.rows]
        self.data_cache = {}
        self.xrefs = None

    def code(self, va):
        i = bisect.bisect_right(self.starts, va) - 1
        if i < 0:
            return None, "before the first function"
        ova, osz, nva, kind = self.rows[i]
        if va < ova + osz:
            if nva is None:
                return None, "function removed in the new build"
            if kind == "same":
                return nva + va - ova, None
            if va == ova:
                return nva, None
            return None, "inside a changed function"
        if i + 1 < len(self.rows):
            n = self.rows[i + 1]
            if nva is not None and n[2] is not None and kind == n[3] == "same" and nva - ova == n[2] - n[0]:
                return va + nva - ova, None
        return None, "between functions of different deltas"

    def build_xrefs(self, wanted):
        tva, t = self.old.text()
        n = len(t) - 8
        disp = (t[0:n].astype(np.int64) | (t[1:n + 1].astype(np.int64) << 8)
                | (t[2:n + 2].astype(np.int64) << 16) | (t[3:n + 3].astype(np.int64) << 24))
        disp = np.where(disp >= 1 << 31, disp - (1 << 32), disp)
        pos = np.arange(n, dtype=np.int64)
        w = np.array(sorted(wanted), dtype=np.int64)
        self.xrefs = defaultdict(list)
        for tail in (0, 1, 2, 4):
            target = tva + pos + 4 + tail + disp
            hit = np.nonzero(np.isin(target, w))[0]
            for i in hit:
                self.xrefs[int(target[i])].append((tva + int(i), tail))

    def data(self, va):
        votes = Counter()
        for site, tail in self.xrefs.get(va, []):
            nsite, _ = self.code(site)
            if nsite is None:
                continue
            b = self.new.read(nsite, 4)
            if b is None:
                continue
            votes[nsite + 4 + tail + int.from_bytes(b, "little", signed=True)] += 1
        if votes:
            (best, n), = votes.most_common(1)
            if n * 2 > sum(votes.values()):
                return best, None
        old = self.old.read(va, 24)
        if old and len(set(old)) > 3:
            lo = va - 0x4000
            hay = self.new.read(lo, 0x8000)
            if hay:
                found = [m.start() for m in re.finditer(re.escape(old), hay)]
                if len(found) == 1:
                    return lo + found[0], None
        return None, "data without a usable reference or unique bytes"

    def map(self, va):
        s = self.old.section(va)
        if s is None:
            return None, "outside the image"
        if s[0] == ".text":
            return self.code(va)
        if va not in self.data_cache:
            self.data_cache[va] = self.data(va)
        return self.data_cache[va]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("tsv")
    ap.add_argument("files", nargs="+")
    ap.add_argument("--skip", action="append", default=[])
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    old, new = Exe(a.old), Exe(a.new)
    m = Mapper(old, new, a.tsv)
    skips = defaultdict(list)
    for s in a.skip:
        f, r = s.rsplit(":", 1)
        lo, hi = r.split("-")
        skips[f].append((int(lo), int(hi)))

    texts = {f: open(f, encoding="utf-8").read() for f in a.files}
    wanted = set()
    for t in texts.values():
        for mt in ADDR.finditer(t):
            va = int(mt.group(2), 16)
            sec = old.section(va)
            if sec and sec[0] != ".text":
                wanted.add(va)
    m.build_xrefs(wanted)

    total, unmapped = Counter(), defaultdict(set)
    for f, t in texts.items():
        out = []
        for ln, line in enumerate(t.split("\n"), 1):
            if any(lo <= ln <= hi for lo, hi in skips.get(f, [])):
                out.append(line)
                continue

            def sub(mt):
                digits = mt.group(2)
                va = int(digits, 16)
                nva, why = m.map(va)
                if nva is None:
                    unmapped[f].add(f"{mt.group(0)} ({why}, line {ln})")
                    return mt.group(0)
                if nva != va:
                    total[f] += 1
                h = f"{nva:09x}"
                return mt.group(1) + (h.upper() if digits.isupper() and not digits.isdigit() else h)

            out.append(ADDR.sub(sub, line))
        new_t = "\n".join(out)
        if new_t != t and not a.dry_run:
            open(f, "w", encoding="utf-8").write(new_t)
    for f in a.files:
        print(f"{f}: {total[f]} moved, {len(unmapped[f])} left")
        for u in sorted(unmapped[f]):
            print(f"    {u}")


if __name__ == "__main__":
    sys.exit(main())
