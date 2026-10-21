#!/usr/bin/env bash
# Stages and archives a server release for one platform (used by .github/workflows/release.yml,
# runnable locally to check a package).
#
#   package-server.sh <linux|windows> <version> <poe-server binary> <web-admin dist dir> \
#                     <dat-schema json> <out dir>
#
# The package mirrors the repository layout the server expects, because every path in
# config/server.toml is relative to the working directory and the server reads versions/ and
# the opcode tables from there:
#
#   poe-server[.exe]  config/server.example.toml  versions/  modules/protocol/opcodes/
#   third_party/dat-schema/schema-poe2.min.json  web-admin/  README.md  LICENSE*  HOSTING.md
#   release.json
#
# Writes <out dir>/<name>.tar.gz (linux) or <name>.zip (windows) and <name>.sha256 (the line
# `sha256sum` prints, so `sha256sum -c` verifies it).
set -euo pipefail

if [ "$#" -ne 6 ]; then
    echo "usage: $0 <linux|windows> <version> <poe-server binary> <web-admin dist dir> <dat-schema json> <out dir>" >&2
    exit 2
fi
platform=$1 version=$2 binary=$3 dist=$4 schema=$5 out=$6

case "$platform" in
    linux | windows) ;;
    *) echo "platform must be linux or windows, not '$platform'" >&2; exit 2 ;;
esac

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

# The game build this server speaks: [game] version of the example config.
game=$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' config/server.example.toml | head -n 1)
[ -n "$game" ] || { echo "no [game] version in config/server.example.toml" >&2; exit 1; }
[ -f "versions/$game.toml" ] || { echo "versions/$game.toml is missing" >&2; exit 1; }
[ -x "$binary" ] || [ -f "$binary" ] || { echo "binary $binary not found" >&2; exit 1; }
[ -f "$dist/index.html" ] || { echo "web admin build $dist has no index.html" >&2; exit 1; }
[ -f "$schema" ] || { echo "dat schema $schema not found" >&2; exit 1; }

name="poe-server-v${version}-game-${game}-${platform}-x86_64"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stage="$work/$name"
exe=poe-server
[ "$platform" = windows ] && exe=poe-server.exe

mkdir -p "$stage/config" "$stage/modules/protocol" "$stage/third_party/dat-schema"
cp "$binary" "$stage/$exe"
chmod 755 "$stage/$exe"
cp config/server.example.toml "$stage/config/"
cp -r versions "$stage/versions"
cp -r modules/protocol/opcodes "$stage/modules/protocol/opcodes"
cp "$schema" "$stage/third_party/dat-schema/schema-poe2.min.json"
cp -r "$dist" "$stage/web-admin"
cp README.md "$stage/README.md"
cp docs/guides/hosting.md "$stage/HOSTING.md"
found=0
for f in LICENSE*; do
    [ -f "$f" ] && { cp "$f" "$stage/"; found=1; }
done
[ "$found" = 1 ] || echo "warning: no LICENSE file in the repository root; the package has none" >&2

commit=${GITHUB_SHA:-$(git rev-parse HEAD 2>/dev/null || echo unknown)}
cat > "$stage/release.json" <<JSON
{
  "name": "$name",
  "server_version": "$version",
  "game_version": "$game",
  "platform": "$platform",
  "arch": "x86_64",
  "commit": "$commit"
}
JSON

mkdir -p "$out"
out=$(cd "$out" && pwd)
case "$platform" in
    linux)
        archive="$name.tar.gz"
        # Reproducible enough: sorted names, no owner info.
        tar -C "$work" --sort=name --owner=0 --group=0 --numeric-owner -czf "$out/$archive" "$name"
        ;;
    windows)
        archive="$name.zip"
        rm -f "$out/$archive"
        if command -v 7z > /dev/null; then
            (cd "$work" && 7z a -tzip -bso0 -bsp0 "$out/$archive" "$name")
        elif command -v zip > /dev/null; then
            (cd "$work" && zip -qr "$out/$archive" "$name")
        else
            py=python3
            command -v "$py" > /dev/null || py=python
            (cd "$work" && "$py" -m zipfile -c "$out/$archive" "$name")
        fi
        ;;
esac
(cd "$out" && sha256sum "$archive" > "$name.sha256")
echo "$out/$archive"
cat "$out/$name.sha256"
