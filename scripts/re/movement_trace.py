#!/usr/bin/env python3
"""Finds movement hitches in a server movement trace (`[instance] movement_trace`).

For every client position report (one per client frame) it computes the step since the
previous frame. A frame that moves backwards (against the previous step) or nearly stalls
while walking is a hitch (unless the player was told to stop); each is printed with the direction replies (`out-dir`) sent shortly
before it, the cell they carried and where the client really was.

Usage: scripts/re/movement_trace.py runtime/movement-trace.tsv [--since MS] [--context N]
"""
import argparse
import math


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("trace")
    ap.add_argument("--since", type=float, default=0.0, help="ignore lines before this ms")
    ap.add_argument("--context", type=int, default=0, help="print N reports around hitches")
    args = ap.parse_args()

    events = []  # (ms, port, kind, fields)
    for line in open(args.trace):
        if line.startswith("#"):
            # A new trace (server restart): times restart at 0.
            events.append((None, None, "restart", []))
            continue
        parts = line.rstrip("\n").split("\t")
        ms, port, kind = float(parts[0]), int(parts[1]), parts[2]
        events.append((ms, port, kind, parts[3:]))

    # Split into segments per server run, keep the last one's lines after --since.
    seg = []
    for e in events:
        if e[2] == "restart":
            seg = []
        else:
            seg.append(e)
    seg = [e for e in seg if e[0] >= args.since]

    reports = [(ms, port, int(f[0]), float(f[1]), float(f[2]), f[3], f[4]) for ms, port, k, f in seg if k == "report"]
    outs = [(ms, port, k, f) for ms, port, k, f in seg if k.startswith("out-") or k.startswith("in-")]
    # De-duplicate reports by frame (keep the last).
    by_frame = {}
    for r in reports:
        by_frame[(r[1], r[2])] = r
    reports = sorted(by_frame.values(), key=lambda r: (r[1], r[2]))

    hitches = 0
    turns = sum(1 for o in outs if o[2] == "out-dir")
    for i in range(2, len(reports)):
        a, b, c = reports[i - 2], reports[i - 1], reports[i]
        if not (a[1] == b[1] == c[1] and b[2] == a[2] + 1 and c[2] == b[2] + 1):
            continue
        p = (b[3] - a[3], b[4] - a[4])
        s = (c[3] - b[3], c[4] - b[4])
        pm, sm = math.hypot(*p), math.hypot(*s)
        if pm < 0.5:
            continue  # not walking before
        if c[5] == "0xed4c" and dot > 0.9:
            continue  # a dodge roll slowing down at its end
        dot = (p[0] * s[0] + p[1] * s[1]) / (pm * sm) if sm > 0 else 1.0
        if sm < 0.6 or dot < -0.2:
            recent = [o for o in outs if o[1] == c[1] and c[0] - 250 <= o[0] <= c[0] and o[2] == "out-dir"]
            if recent and recent[-1][3][:2] == ["0", "0"]:
                continue  # a stop, not a hitch
            hitches += 1
            print(f"hitch at frame {c[2]} ({c[0]:.0f} ms): step {s[0]:+.2f},{s[1]:+.2f} after {p[0]:+.2f},{p[1]:+.2f}")
            for o in outs:
                if o[1] == c[1] and c[0] - 250 <= o[0] <= c[0]:
                    print(f"    {o[0]:.0f} ms {o[2]} {' '.join(o[3])}")
            for r in reports[max(0, i - 4 - args.context): i + 2 + args.context]:
                if r[1] == c[1]:
                    print(f"    {r[0]:.0f} ms frame {r[2]} at ({r[3]:.2f}, {r[4]:.2f}) cell ({math.floor(r[3])}, {math.floor(r[4])})")
    print(f"{len(reports)} frames, {turns} direction replies, {hitches} hitches")


if __name__ == "__main__":
    main()
