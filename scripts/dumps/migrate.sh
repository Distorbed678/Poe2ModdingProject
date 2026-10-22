#!/usr/bin/env bash
# Sorts the generated/datamined outputs that piled up in GameFiles/, runtime/ and docs/re/ into
# dumps/<version>/ (git-ignored; layout in dumps/README.md). Safe to run again: anything already
# at its destination is skipped and left where it is.
#
#   scripts/dumps/migrate.sh [--dry-run]
#
# Whitelist only: each pattern below names one of OUR generated outputs. Game files (Bundles2,
# the exe, DLLs, ...) are never matched. Generated outputs are moved; packet capture sessions
# are copied (the originals stay in GameFiles/logs/poe-hook/), except their `areas/` export
# folders, which are moved to terrain/area-exports/<session>/.
set -eu
cd "$(dirname "$0")/../.."
ROOT=$PWD
DRY=0
[ "${1:-}" = "--dry-run" ] && DRY=1

version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' modules/server/config/server.toml | head -n1)
D=${DUMPS_DIR:-$ROOT/dumps/$version}
GF=$ROOT/GameFiles
RT=$ROOT/runtime
SESS=$GF/logs/poe-hook

declare -A COUNT=()
note() { COUNT[$1]=$(( ${COUNT[$1]:-0} + 1 )); }
run() { if [ "$DRY" = 1 ]; then echo "  [dry] $*"; else "$@"; fi; }

# put <move|copy> <src> <dest dir relative to $D> : moves/copies src into $D/<dest>/ unless a
# file or folder of that name is already there.
put() {
    local how=$1 src=$2 dest=$3 name
    name=$(basename "$src")
    [ -e "$src" ] || return 0
    if [ -e "$D/$dest/$name" ]; then
        COUNT["skipped (already in dumps)"]=$(( ${COUNT["skipped (already in dumps)"]:-0} + 1 ))
        return 0
    fi
    run mkdir -p "$D/$dest"
    if [ "$how" = move ]; then run mv "$src" "$D/$dest/$name"; else run cp -a "$src" "$D/$dest/$name"; fi
    note "$dest"
}

echo "dumps: $D"
if pgrep -f 'PathOfExile' >/dev/null 2>&1; then
    echo "warning: a PathOfExile process is running; files it is writing may be incomplete"
fi

echo "== GameFiles/: terrain exports and client tables"
for f in "$GF"/terrain_grid_*.bin;   do [ -e "$f" ] && put move "$f" terrain/grids; done
for f in "$GF"/terrain_layout_*.tsv; do [ -e "$f" ] && put move "$f" terrain/layouts; done
for f in "$GF"/object_types.txt "$GF"/terrain_tiles.txt; do [ -e "$f" ] && put move "$f" client-tables; done

echo "== GameFiles/logs/poe-hook/<session>/areas -> terrain/area-exports/<session>"
for s in "$SESS"/20*/; do
    s=${s%/}
    [ -d "$s/areas" ] || continue
    session=$(basename "$s")
    if [ -e "$D/terrain/area-exports/$session" ]; then
        # Merge: move the export folders that are not there yet.
        for a in "$s"/areas/*; do [ -e "$a" ] && put move "$a" "terrain/area-exports/$session"; done
        [ "$DRY" = 1 ] || rmdir "$s/areas" 2>/dev/null || true
    else
        run mkdir -p "$D/terrain/area-exports"
        run mv "$s/areas" "$D/terrain/area-exports/$session"
        note terrain/area-exports
    fi
done

echo "== runtime/: terrain research outputs"
for f in "$RT"/act1_traces "$RT"/town_trace_*; do [ -e "$f" ] && put move "$f" terrain/traces; done
for f in "$RT"/town_export_* "$RT"/river_export_*; do [ -e "$f" ] && put move "$f" terrain/area-exports; done
for f in "$RT"/sweep-*.txt "$RT"/sweep-*.tsv "$RT"/terrain-vectors.tsv "$RT"/terrain_vectors.tsv \
         "$RT"/vectors-run*.txt; do
    [ -e "$f" ] && put move "$f" terrain/vectors
done
for f in "$RT"/ghidra_named_functions_*.json; do [ -e "$f" ] && put move "$f" ghidra; done
for f in "$RT"/clientstrings*.txt; do [ -e "$f" ] && put move "$f" strings; done

echo "== docs/re/*.json (factory/vtable maps) -> ghidra"
for f in docs/re/instance_factory_s2c.json docs/re/instance_factory2_s2c.json docs/re/packet_classes_*.json; do
    [ -e "$f" ] && put move "$ROOT/$f" ghidra
done

echo "== packet capture sessions (copied) -> captures/{official,local}"
# Official: the session ran in official mode, or connected to a non-loopback address (the
# DLL logs "connect #N ... (external)" once it latched read-only). Everything else is local.
# Sessions without a packet file are skipped.
for s in "$SESS"/20*/; do
    s=${s%/}
    session=$(basename "$s")
    if [ ! -s "$s/packets.jsonl" ] && [ ! -s "$s/packets.txt" ]; then
        note "captures skipped (no packets)"
        continue
    fi
    if grep -qE 'mode Official|\(external\)' "$s/poe-hook.log" 2>/dev/null; then kind=official; else kind=local; fi
    if [ -e "$D/captures/$kind/$session" ]; then
        COUNT["skipped (already in dumps)"]=$(( ${COUNT["skipped (already in dumps)"]:-0} + 1 ))
        continue
    fi
    run mkdir -p "$D/captures/$kind"
    run cp -a "$s" "$D/captures/$kind/$session"
    note "captures/$kind"
done

# Leave the empty tree in place for outputs that do not exist yet.
for d in terrain/grids terrain/layouts terrain/area-exports terrain/traces terrain/vectors \
         client-tables ghidra captures/official captures/local strings; do
    run mkdir -p "$D/$d"
done

echo "== summary"
for k in "${!COUNT[@]}"; do printf '  %-32s %s\n' "$k" "${COUNT[$k]}"; done | sort
