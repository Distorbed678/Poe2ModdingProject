#!/usr/bin/env python3
"""Groups the errors of `area_probe` (dumps/0.5.5e/terrain/vectors/probe-results.tsv) by the
unported system that causes them, and ranks the systems by how many areas each unlocks.

Usage: scripts/re/probe_classes.py [probe-results.tsv] [--areas]

Prints, per class: failing topologies, areas touched, areas that only this class blocks
("solo"); then a greedy order (the class that makes the most areas fully clean first). With
`--areas` the areas of each class are listed. The classes are regexes over the generator's error
text (modules/server/crates/poe-world); an error that matches none is listed as `other: <text>`.
A probe checks one seed and one orientation per topology and stops at the first error, so the
counts are a lower bound: later errors stay hidden behind the first.
"""
import collections
import re
import sys

DEFAULT = "dumps/0.5.5e/terrain/vectors/probe-results.tsv"
CLASSES = [
    ("dungeon edge types with height change / virtual edge (chirality)", r"chirality not ported"),
    ("3x3 door spots (0x14209dc90)", r"door tile at"),
    ("FileGroups subgraphs", r"FileGroups"),
    ("height fallback (0x141d05560 / 0x14209fb00)", r"could not fix heights"),
    ("overlays: phase 2, SizeOrder, exclusions, MatchAllTileKeys", r"overlay"),
    ("special rooms: blank, injector, split", r"special room|injector"),
    ("lettered unique tags (session history)", r"lettered unique"),
    ("tgr edge routing / retries / tilising", r"failed to route|could not tilise"),
    ("virtual nodes and virtual edge pairs (node tiles)", r"virtual node|virtual edge|two virtual edges"),
    ("dungeon corridors: edge ends outside the grid, failed to connect", r"failed to connect|an end outside"),
    ("fill: orientation masks, no fill tile", r"orientation mask|fill"),
    ("room set / feature rooms missing", r"no RoomSet|GetFeatureRooms|no root key|no FillTiles"),
    ("data files not found (path handling)", r"FileNotFound"),
    ("graph parse errors (.dgr fields)", r"GraphError|edge field"),
    ("node tiles: no tile matches its edges", r"no tile matches its edges|no tile fits"),
    ("dungeon tile pass: no border tile / no tile for slot / overlap", r"no border tile|dungeon: no tile for|overlap tile"),
    ("dungeon rooms: no room fits / no suitable room", r"no room fits|no suitable room|placeholder"),
]


def classify(detail):
    for name, pattern in CLASSES:
        if re.search(pattern, detail):
            return name
    return "other: " + re.sub(r"\d+", "N", detail)[:70]


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    show_areas = "--areas" in sys.argv
    rows = [l.rstrip("\n").split("\t") for l in open(args[0] if args else DEFAULT)]
    areas, per_area, listed = set(), collections.defaultdict(collections.Counter), collections.defaultdict(set)
    topologies = ok = 0
    for r in rows:
        if len(r) < 6:
            continue
        areas.add(r[0])
        topologies += 1
        if r[5] == "ok":
            ok += 1
            continue
        c = classify(r[6] if len(r) > 6 else r[5])
        per_area[r[0]][c] += 1
        listed[c].add(r[0])
    clean = len(areas) - len(per_area)
    print(f"{len(areas)} areas, {topologies} topologies: {ok} run to the end; {clean} areas run for every topology")
    touched, tops, solo = collections.Counter(), collections.Counter(), collections.Counter()
    for a, d in per_area.items():
        for c, n in d.items():
            touched[c] += 1
            tops[c] += n
        if len(d) == 1:
            solo[next(iter(d))] += 1
    print("topologies\tareas\tsolo\tclass")
    for c, _ in touched.most_common():
        print(f"{tops[c]}\t{touched[c]}\t{solo[c]}\t{c}")
        if show_areas:
            print("\t\t\t  " + " ".join(sorted(listed[c])))
    print("\ngreedy order: class, areas that become clean, clean areas after")
    remaining = {a: set(d) for a, d in per_area.items()}
    done, clean_now = set(), clean
    while remaining:
        gains = {c: sum(1 for s in remaining.values() if s <= done | {c}) for c in touched if c not in done}
        c = max(gains, key=lambda k: (gains[k], touched[k]))
        done.add(c)
        before = len(remaining)
        remaining = {a: s for a, s in remaining.items() if not s <= done}
        clean_now += before - len(remaining)
        print(f"{c}\t+{before - len(remaining)}\t{clean_now}")


if __name__ == "__main__":
    main()
