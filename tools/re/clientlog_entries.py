"""Per-area-entry timelines from a PoE2 Client.txt (read only).

Usage: clientlog_entries.py <Client.txt> [--area G1_1] [--max N] [--lines N]

Splits the log at "Got Instance Details" and prints, for each entry, the area, level and
seed, followed by the first log lines with their time offsets in ms. Comparing a live
session with ours shows which client events (and so which server messages) are missing.
Noisy messages that repeat every frame are skipped.
"""
import argparse
import re

NOISE = re.compile(
    r"GetArtVariationItemInternal|Tried to get position height|MavenJewelRadiusKeystones|"
    r"VULKAN\] Supported|Matching object found for InstanceClientActionUpdate|"
    r"\[WINDOW\]|ETrade|Error executing GEAL|Invalid source bone|has no vertex influences"
)
LINE = re.compile(r"^\S+ \S+ (\d+) \S+ \[(\w+) Client \d+\] (.*)$")
GEN = re.compile(r'Generating level (\d+) area "([^"]+)" with seed (\d+)')


def entries(path):
    current = None
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            m = LINE.match(raw.rstrip("\n"))
            if not m:
                continue
            tick, level, text = int(m.group(1)), m.group(2), m.group(3)
            if "Got Instance Details" in text:
                if current:
                    yield current
                current = {"start": tick, "area": None, "lines": []}
                continue
            if current is None:
                continue
            g = GEN.search(text)
            if g:
                current["area"] = (g.group(2), int(g.group(1)), int(g.group(3)))
            if not NOISE.search(text):
                current["lines"].append((tick - current["start"], level, text))
    if current:
        yield current


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("log")
    ap.add_argument("--area")
    ap.add_argument("--max", type=int, default=3)
    ap.add_argument("--lines", type=int, default=40)
    a = ap.parse_args()
    shown = 0
    for e in entries(a.log):
        if a.area and (not e["area"] or e["area"][0] != a.area):
            continue
        print(f"=== area={e['area']}")
        for dt, level, text in e["lines"][: a.lines]:
            print(f"  +{dt:6d}ms {level:5} {text[:150]}")
        shown += 1
        if shown >= a.max:
            break


if __name__ == "__main__":
    main()
