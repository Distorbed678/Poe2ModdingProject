#!/bin/sh
# Screenshot of the game monitor (middle of three) for test runs: scripts/shot.sh <name>
# Writes runtime/shots/<name>.png (full) and <name>_game.png (game monitor, 1280 wide).
set -e
cd "$(dirname "$0")/.."
mkdir -p runtime/shots
spectacle -b -n -f -o "runtime/shots/$1.png" >/dev/null 2>&1
magick "runtime/shots/$1.png" -crop 2560x1440+1920+0 +repage -resize 1280x "runtime/shots/$1_game.png"
echo "runtime/shots/$1_game.png"
