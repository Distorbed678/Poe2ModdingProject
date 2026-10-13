#!/bin/sh
# Screenshot of the game monitor (middle of three) for test runs: tools/shot.sh <name>
# Writes data/shots/<name>.png (full) and <name>_game.png (game monitor, 1280 wide).
set -e
cd "$(dirname "$0")/.."
mkdir -p data/shots
spectacle -b -n -f -o "data/shots/$1.png" >/dev/null 2>&1
magick "data/shots/$1.png" -crop 2560x1440+1920+0 +repage -resize 1280x "data/shots/$1_game.png"
echo "data/shots/$1_game.png"
