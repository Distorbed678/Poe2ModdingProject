#!/usr/bin/env python3
"""Compare two packets.jsonl captures from the client DLL (e.g. an official session and one on
our server): which opcodes each side has, how often, and their sizes.

Usage: packets_diff.py <a/packets.jsonl> <b/packets.jsonl> [--channel instance] [--seq N]

--seq N also prints the first N non-noise messages of each capture side by side, to compare the
order of a sequence (login, area entry, ...).
"""

import argparse
import json
from collections import Counter, defaultdict

NOISE = {"FrameReport", "PingReply", "SimulationTick", "KeepAlive", "KeepAliveReply"}


def load(path, channel):
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            m = json.loads(line)
            if channel and m["chan"] != channel:
                continue
            out.append(m)
    return out


def summary(msgs):
    counts = Counter()
    sizes = defaultdict(set)
    names = {}
    for m in msgs:
        key = (m["chan"], m["dir"], m["op"])
        counts[key] += 1
        sizes[key].add(m["len"])
        names[key] = m["name"]
    return counts, sizes, names


def size_text(s):
    return f"{min(s)}" if len(s) == 1 else f"{min(s)}..{max(s)}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--channel")
    ap.add_argument("--seq", type=int, default=0)
    args = ap.parse_args()
    a, b = load(args.a, args.channel), load(args.b, args.channel)
    ca, sa, na = summary(a)
    cb, sb, nb = summary(b)
    keys = sorted(set(ca) | set(cb))
    print(f"{'chan':8} {'dir':3} {'op':6} {'name':28} {'A':>7} {'B':>7}  sizes A / B")
    for k in keys:
        name = na.get(k) or nb.get(k) or "?"
        mark = "" if (k in ca) == (k in cb) else ("  <- only A" if k in ca else "  <- only B")
        sizes = f"{size_text(sa[k]) if k in sa else '-'} / {size_text(sb[k]) if k in sb else '-'}"
        print(f"{k[0]:8} {k[1]:3} {k[2]:6} {name:28} {ca.get(k, 0):7} {cb.get(k, 0):7}  {sizes}{mark}")
    if args.seq:
        print()
        ra = [m for m in a if m["name"] not in NOISE][: args.seq]
        rb = [m for m in b if m["name"] not in NOISE][: args.seq]
        for i in range(max(len(ra), len(rb))):
            fa = f"{ra[i]['dir']:3} {ra[i]['op']} {ra[i]['name'] or '?':24} {ra[i]['len']:5}" if i < len(ra) else ""
            fb = f"{rb[i]['dir']:3} {rb[i]['op']} {rb[i]['name'] or '?':24} {rb[i]['len']:5}" if i < len(rb) else ""
            print(f"{i:4} {fa:44} | {fb}")


if __name__ == "__main__":
    main()
