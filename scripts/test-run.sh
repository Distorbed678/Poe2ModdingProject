#!/bin/bash
# Unattended client test (server must already be running with the dev config, POE_CONFIG=runtime/server.dev.toml
# (python3 scripts/dev-config.py); GameFiles deployed with `cargo xtask deploy-client --no-build --login-mode
# standalone`; Steam must be running): signs the test account (default `testrun`, admin, random password in
# runtime/test-accounts/, see scripts/lib/test-accounts.sh) in with a Remember Me token, launches the client, which
# presses Login via poe-hook auto_keys (POE_HOOK_AUTO_KEYS: Enter at 20 s), presses Play and waits. The account
# gets one character on first use (TEST_CLASS, TEST_AREA; defaults DexFourDefault, G1_town). It first deletes the terrain layouts/grids in dumps/<version>/terrain. The client window is titled "PoE2 Test Client" (POE_HOOK_WINDOW_TITLE), so a desktop
# rule can keep it off the screen (see docs/guides/setup.md). SHOT=1 also takes a
# screenshot of the game monitor.
# Usage: scripts/test-run.sh <name> [seconds in game before returning]
set -e
cd "$(dirname "$0")/.."
name=$1; wait=${2:-12}
CFG=${POE_CONFIG:-runtime/server.dev.toml}
ACCT=${TEST_ACCOUNT:-testrun}
srv() { POE_CONFIG=$CFG target/debug/poe-server "$@"; }
. scripts/lib/test-accounts.sh
grep -q '^login_mode *= *"standalone"' GameFiles/poe-hook.toml \
    || { echo "GameFiles is not in standalone login mode: cargo xtask deploy-client --no-build --login-mode standalone"; exit 1; }
ta_ensure "$ACCT" admin
srv character list "$ACCT" 2>/dev/null | grep -qP "^$ACCT\t" \
    || srv character create "$ACCT" "${ACCT}Ranger" "${TEST_CLASS:-DexFourDefault}" "${TEST_AREA:-G1_town}" >/dev/null
tok=$(ta_token "$ACCT" "$(ta_web_port "$CFG")")
[ ${#tok} = 64 ] || { echo "no game token for $ACCT (is the server running with the web API on?)"; exit 1; }
cargo xtask login-token "$ACCT" "$tok" | head -1
unset tok
log() { sed 's/\x1b\[[0-9;]*m//g' runtime/server-run.log; }
start=$(log | wc -l)
# Fresh terrain exports for this run: the server waits for the layout the client writes (by tile
# hash), so stale layouts and grids from earlier runs would hide a DLL that stopped exporting.
version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' config/server.toml | head -n1)
rm -f GameFiles/poe-hook.log GameFiles/terrain_layout*.tsv GameFiles/terrain_grid*.bin \
    "dumps/$version"/terrain/layouts/terrain_layout_*.tsv "dumps/$version"/terrain/grids/terrain_grid_*.bin
POE_HOOK_MOVEMENT_LOG=1 POE_HOOK_WINDOW_TITLE="PoE2 Test Client" POE_HOOK_AUTO_KEYS=20000:13 POE_HOOK_FAKE_FOCUS=1 \
    cargo xtask run-client > runtime/client-run.log 2>&1 &
i=0
until log | tail -n +"$start" | grep -q authenticated; do
    sleep 1; i=$((i+1)); [ $i -gt 90 ] && { echo "no login"; exit 1; }
done
sleep 3; echo 0x0d > GameFiles/poe-hook.keys
sleep "$wait"
if [ "${SHOT:-0}" = 1 ]; then scripts/shot.sh "$name"; fi
echo "in game: $name"
