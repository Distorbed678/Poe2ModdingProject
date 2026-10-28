"""Map code addresses of one client build to another (hotfix rebuilds), from the two exes' .pdata.

Usage: addr_map.py <old exe> <new exe> [--out <tsv>] [--map <va> ...]

Both exes list their functions in .pdata (start, end), in address order. A hotfix rebuild keeps
that order and moves runs of functions by a constant delta, so the function lists are aligned
on their sizes (difflib, longest equal runs first); inside an unmatched gap of the same length
on both sides the functions are paired in order (changed bodies). The TSV has one row per old
function: old_va, new_va (or -), old size, new size, kind (same = equal size inside a matched
run, gap = paired in a gap). `--map` prints the new address of any old code address: the
enclosing function's delta (only for kind `same`, else reported as unsure).
Data addresses (.rdata/.data globals) are not mapped; the signature table's `ref` entries
resolve those.
"""
import argparse
import bisect
import difflib
import struct
import sys

import pefile


def functions(path):
    pe = pefile.PE(path, fast_load=True)
    base = pe.OPTIONAL_HEADER.ImageBase
    pdata = next(s for s in pe.sections if s.Name.rstrip(b"\0") == b".pdata")
    raw = pdata.get_data()[: pdata.Misc_VirtualSize]
    out = []
    seen = set()
    for i in range(0, len(raw) - 11, 12):
        b, e, u = struct.unpack_from("<III", raw, i)
        if b == 0:
            break
        # Chained unwind entries describe parts of one function; keep the first start.
        if b in seen:
            continue
        seen.add(b)
        out.append((base + b, e - b))
    out.sort()
    return out


def align(old, new):
    sm = difflib.SequenceMatcher(None, [s for _, s in old], [s for _, s in new], autojunk=False)
    rows = []
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            for k in range(i2 - i1):
                rows.append((old[i1 + k], new[j1 + k], "same"))
        elif tag == "replace" and i2 - i1 == j2 - j1:
            for k in range(i2 - i1):
                rows.append((old[i1 + k], new[j1 + k], "gap"))
        elif tag in ("replace", "delete"):
            for k in range(i1, i2):
                rows.append((old[k], None, "lost"))
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("--out")
    ap.add_argument("--map", nargs="*", default=[])
    a = ap.parse_args()
    old, new = functions(a.old), functions(a.new)
    rows = align(old, new)
    kinds = {}
    for _, _, k in rows:
        kinds[k] = kinds.get(k, 0) + 1
    print(f"old {len(old)} functions, new {len(new)}; " + ", ".join(f"{k} {n}" for k, n in sorted(kinds.items())),
          file=sys.stderr)
    if a.out:
        with open(a.out, "w") as f:
            f.write("old_va\tnew_va\told_size\tnew_size\tkind\n")
            for (ova, osz), n, k in rows:
                f.write(f"{ova:#x}\t{n[0]:#x}\t{osz}\t{n[1]}\t{k}\n" if n else f"{ova:#x}\t-\t{osz}\t-\t{k}\n")
    starts = [o[0] for o, _, _ in rows]
    for s in a.map:
        va = int(s, 16)
        i = bisect.bisect_right(starts, va) - 1
        (ova, osz), n, k = rows[i]
        if i < 0 or va >= ova + osz or n is None:
            print(f"{va:#x}\t-\tnot in a mapped function")
        else:
            print(f"{va:#x}\t{va - ova + n[0]:#x}\t{k}{'' if k == 'same' else ' (unsure: body changed)'}")


if __name__ == "__main__":
    main()
