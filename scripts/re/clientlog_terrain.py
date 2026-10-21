"""Terrain generation test vectors from a PoE2 Client.txt (read only).

Usage: clientlog_terrain.py <Client.txt> [out.tsv]

Each area entry logs `Generating level L area "A" with seed S` followed by `Tile hash: T` and
`Doodad hash: D`. A server-side generator (`crates/poe-world`) is correct when it reproduces (T, D) for (A, S).
Writes `area<TAB>level<TAB>seed<TAB>tile_hash<TAB>doodad_hash<TAB>build`, de-duplicated; `build`
is the client's `Build Revision` at the time (game data, and so layouts, change with it).
"""
import re
import sys

GEN = re.compile(r'Generating level (\d+) area "([^"]+)" with seed (\d+)')
TILE = re.compile(r"Tile hash: (\d+)")
DOODAD = re.compile(r"Doodad hash: (\d+)")
BUILD = re.compile(r"Build Revision: (\d+)")

rows, cur, build = set(), None, "?"
with open(sys.argv[1], encoding="utf-8", errors="replace") as f:
    for line in f:
        if m := BUILD.search(line):
            build = m.group(1)
        elif m := GEN.search(line):
            cur = [m.group(2), m.group(1), m.group(3), None]
        elif cur and (m := TILE.search(line)):
            cur[3] = m.group(1)
        elif cur and cur[3] and (m := DOODAD.search(line)):
            rows.add((*cur, m.group(1), build))
            cur = None
out = open(sys.argv[2], "w") if len(sys.argv) > 2 else sys.stdout
for r in sorted(rows):
    print("\t".join(r), file=out)
print(f"{len(rows)} vectors, {len({r[0] for r in rows})} areas", file=sys.stderr)
