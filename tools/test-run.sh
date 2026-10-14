#!/bin/sh
# Unattended client test (server must already be running; Steam must be running):
# launches the client, which logs in via poe-hook auto_keys (Enter at 20 s), presses Play and
# waits. The client window is titled "PoE2 Test Client" (POE_HOOK_WINDOW_TITLE), so a desktop
# rule can keep it off the screen (see Documentation/client-dll.md). SHOT=1 also takes a
# screenshot of the game monitor.
# Usage: tools/test-run.sh <name> [seconds in game before returning]
set -e
cd "$(dirname "$0")/.."
name=$1; wait=${2:-12}
log() { sed 's/\x1b\[[0-9;]*m//g' data/server-run.log; }
start=$(log | wc -l)
rm -f GameFiles/poe-hook.log GameFiles/terrain_layout*.tsv
POE_HOOK_MOVEMENT_LOG=1 POE_HOOK_WINDOW_TITLE="PoE2 Test Client" \
    cargo xtask run-client > data/client-run.log 2>&1 &
i=0
until log | tail -n +"$start" | grep -q authenticated; do
    sleep 1; i=$((i+1)); [ $i -gt 90 ] && { echo "no login"; exit 1; }
done
sleep 3; echo 0x0d > GameFiles/poe-hook.keys
sleep "$wait"
if [ "${SHOT:-0}" = 1 ]; then tools/shot.sh "$name"; fi
echo "in game: $name"
