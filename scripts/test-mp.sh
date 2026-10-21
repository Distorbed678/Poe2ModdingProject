#!/bin/bash
# Local two-client multiplayer test (WP3.7, docs/guides/hosting.md "Local two-client test").
#
# Two clients run the same GameFiles exe in two Proton prefixes (third_party/proton-prefix and
# third_party/proton-prefix-2), each logged in as its own account (mp1, mp2) with a remember-me
# token, and both connect through the machine's LAN IP, not 127.0.0.1, so the trusted-remote path
# of the DLL runs (the deploy puts the LAN IP into trusted_servers). Each client has its own input
# file (GameFiles/poe-hook.<n>.keys via POE_HOOK_KEYS_FILE), window title ("PoE2 MP <n>") and
# session folder. Steam must be running (the client exits without it). Never touches the Steam
# install or compatdata/2694490, and never connects to official servers.
#
# Usage:
#   scripts/test-mp.sh setup        build, runtime config with the LAN IP, accounts + characters,
#                                   second prefix, deploy-client (standalone login, LAN trust)
#   scripts/test-mp.sh server       start poe-server (runtime/server-mp.log), detached
#   scripts/test-mp.sh clients      log both clients in and enter the game (tokens, Enter x2)
#   scripts/test-mp.sh client <n> [acct]   the same for one client (1 or 2); with acct, client n logs in
#                                   as mp<acct> (duplicate-login test: client 2 as mp1 after closing it)
#   scripts/test-mp.sh keys <n> <entries...>   send poe-hook.keys entries to client n
#   scripts/test-mp.sh shot <n>     F8 game screenshot in client n; prints the new file
#   scripts/test-mp.sh status       processes, memory, session lines of the server log
#   scripts/test-mp.sh stop         kill clients, then the server (run as its own call), and put
#                                   the deployed GameFiles back to the private steam-login state
#   scripts/test-mp.sh restore      only the last step of stop (deploy-client --login-mode steam)
# Environment: LAN_IP (default: the source address of the default route), MP_PASSWORD,
# MP_RES (window size, default 1280x720), MP_BORDERLESS (true|false, borderless_windowed_fullscreen, default false), MP_AREA (character start area, default G1_town),
# MP_CLASS (CharacterStartStates id, default DexFourDefault).
set -e
cd "$(dirname "$0")/.."
ROOT=$PWD
CFG=runtime/server-mp.toml
LOG=runtime/server-mp.log
PW=${MP_PASSWORD:-mp-test-password-1}
RES=${MP_RES:-1280x720}
BORDERLESS=${MP_BORDERLESS:-false}
AREA=${MP_AREA:-G1_town}
CLASS=${MP_CLASS:-DexFourDefault}
LAN_IP=${LAN_IP:-$(ip -4 route get 1.1.1.1 | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -n1)}
DOCS='pfx/drive_c/users/steamuser/Documents/My Games/Path of Exile 2'
prefix_of() { [ "$1" = 1 ] && echo third_party/proton-prefix || echo third_party/proton-prefix-2; }
ini_of() { echo "$(prefix_of "$1")/$DOCS/poe2_production_Config.ini"; }
shots_of() { echo "$(prefix_of "$1")/$DOCS/Screenshots"; }
xt() { cargo xtask --config "$CFG" "$@"; }
srv() { POE_CONFIG=$CFG target/debug/poe-server "$@"; }
slog() { sed 's/\x1b\[[0-9;]*m//g' "$LOG"; }
api_port() { sed -n 's/^bind *= *"[0-9.]*:\([0-9]*\)".*/\1/p' <<<"$(sed -n '/^\[web\]/,/^\[/p' "$CFG")" | head -n1; }

cmd_setup() {
    [ -n "$LAN_IP" ] || { echo "no LAN IP (set LAN_IP)"; exit 1; }
    cargo build -p poe-server -p xtask
    # Runtime copy of the config: LAN address, its own database (mp accounts stay out of the dev one).
    sed -e "s|^public_host = .*|public_host = \"$LAN_IP\"|" \
        -e 's|^path = "runtime/server.sqlite"|path = "runtime/server-mp.sqlite"|' \
        -e 's|^audit_log = .*|audit_log = "runtime/server-mp-audit.log"|' config/server.toml > $CFG
    grep -q "public_host = \"$LAN_IP\"" $CFG && echo "config: $CFG (public_host $LAN_IP)"
    for n in 1 2; do
        acct=mp$n
        srv account list 2>/dev/null | grep -qw "$acct" || srv account create "$acct" "$PW"
        srv character list "$acct" 2>/dev/null | grep -qP "^$acct\t" || srv character create "$acct" "Mp${n}Ranger" "$CLASS" "$AREA"
    done
    srv account list | tail -1; srv character list | grep -P "^mp"
    # Second prefix: a copy of the first (both are ours); its own login and screenshots.
    p2=$(prefix_of 2)
    if [ ! -d "$p2" ]; then
        cp -a --reflink=auto "$(prefix_of 1)" "$p2"
        rm -rf "$p2/$DOCS/Screenshots"/* "$p2/$DOCS/poe2_production_login.credentials"
        echo "created $p2"
    fi
    # Small windows (two clients at once): remember the original size of both prefixes.
    w=${RES%x*}; h=${RES#*x}
    for n in 1 2; do
        ini=$(ini_of $n)
        [ -f "$ini" ] || continue
        if [ "$n" = 1 ] && [ ! -f runtime/mp-orig-res.txt ]; then
            grep -E '^resolution_(width|height)=' "$ini" > runtime/mp-orig-res.txt
        fi
        sed -i -e "s|^resolution_width=.*|resolution_width=$w|" -e "s|^resolution_height=.*|resolution_height=$h|" \
            -e "s|^borderless_windowed_fullscreen=.*|borderless_windowed_fullscreen=$BORDERLESS|" "$ini"
    done
    xt deploy-client --no-build --login-mode standalone
    xt --prefix "$(prefix_of 2)" select-gateway
    rm -f GameFiles/poe-hook.keys GameFiles/poe-hook.1.keys GameFiles/poe-hook.2.keys
    grep -E "^(trusted_servers|login_mode|mode)" GameFiles/poe-hook.toml
    free -g | sed -n 2p
}

cmd_server() {
    pgrep -f '[t]arget/debug/poe-server' >/dev/null && { echo "server already running"; return; }
    : > $LOG
    POE_CONFIG=$CFG setsid nohup target/debug/poe-server > $LOG 2>&1 < /dev/null &
    for _ in $(seq 60); do slog | grep -q "web api\|listening" && break; sleep 1; done
    slog | grep -iE "listen|warn|error" | head -8
}

# Fresh remember-me token for mp<n>: web API launcher login, then the game-token exchange.
issue_token() {
    port=$(api_port); port=${port:-8080}
    l=$(curl -s -X POST "127.0.0.1:$port/api/v1/login" -H 'content-type: application/json' \
        -d "{\"username\":\"mp$1\",\"password\":\"$PW\",\"kind\":\"launcher\"}" | sed -n 's/.*"token":"\([0-9a-f]*\)".*/\1/p')
    [ -n "$l" ] || { echo "web login failed for mp$1"; exit 1; }
    curl -s -X POST "127.0.0.1:$port/api/v1/game-token" -H "authorization: Bearer $l" | sed -n 's/.*"token":"\([0-9a-f]*\)".*/\1/p'
}

n_auth() { slog | grep -c ' authenticated' || true; }
keys_file() { echo "GameFiles/poe-hook.$1.keys"; }

cmd_client() {
    n=$1; acct=${2:-$n}        # client n logs in as mp<acct> (acct != n: the duplicate-login test)
    tok=$(issue_token "$acct")
    [ ${#tok} = 64 ] || { echo "no game token for mp$acct"; exit 1; }
    xt --prefix "$(prefix_of "$n")" login-token "mp$acct" "$tok" | head -1
    before=$(n_auth)
    POE_HOOK_KEYS_FILE=poe-hook.$n.keys POE_HOOK_WINDOW_TITLE="PoE2 MP $n" POE_HOOK_AUTO_KEYS=20000:13 \
        POE_HOOK_FAKE_FOCUS=1 POE_HOOK_MOVEMENT_LOG=1 \
        setsid nohup cargo xtask --config "$CFG" --prefix "$(prefix_of "$n")" run-client > "runtime/client-mp$n.log" 2>&1 < /dev/null &
    i=0
    until [ "$(n_auth)" -gt "$before" ]; do
        sleep 1; i=$((i+1)); [ $i -gt 120 ] && { echo "client $n: no login"; exit 1; }
    done
    echo "client $n logged in after ${i}s"
    sleep 4; echo 0x0d > "$(keys_file "$n")"       # Play (the first character)
    sleep 20                                       # character select -> instance entry
}

cmd_clients() {
    cmd_client 1
    sleep 15
    cmd_client 2
}

cmd_keys() { n=$1; shift; printf '%s\n' "$@" > "$(keys_file "$n")"; }

cmd_shot() {
    n=$1; d=$(shots_of "$n"); mkdir -p "$d"
    before=$(ls "$d" 2>/dev/null | wc -l)
    echo 0x77 > "$(keys_file "$n")"
    for _ in $(seq 20); do
        [ "$(ls "$d" | wc -l)" -gt "$before" ] && break; sleep 0.5
    done
    ls -t "$d" | head -n1 | sed "s|^|$ROOT/$d/|"
}

cmd_status() {
    pgrep -af '[P]athOfExileSteam.exe' | cut -c1-120 || true
    pgrep -af '[t]arget/debug/poe-server' | cut -c1-100 || true
    free -g | sed -n 2p
    [ -f $LOG ] && slog | grep -E "authenticated|enter|AddObject|remote|leave|supersed|trusted" | tail -${LINES_N:-20}
}

cmd_restore() {
    # Window sizes of prefix 1 back to what they were.
    if [ -f runtime/mp-orig-res.txt ] && [ -f "$(ini_of 1)" ]; then
        while IFS= read -r l; do
            k=${l%%=*}; sed -i "s|^$k=.*|$l|" "$(ini_of 1)"
        done < runtime/mp-orig-res.txt
        sed -i 's|^borderless_windowed_fullscreen=.*|borderless_windowed_fullscreen=true|' "$(ini_of 1)"
    fi
    rm -f GameFiles/poe-hook.*.keys
    # Private deploy with the Steam login and the committed config (loopback).
    cargo xtask deploy-client --no-build --login-mode steam
}

cmd_stop() {
    pgrep -f '[P]athOfExileSteam.exe' | xargs -r kill || true
    sleep 3
    echo "clients stopped; stop the server separately: pkill -f 'target/debug/poe-server'"
    cmd_restore
}

c=${1:-}; shift || true
case "$c" in
    setup) cmd_setup ;; server) cmd_server ;; clients) cmd_clients ;;
    client) cmd_client "$@" ;; keys) cmd_keys "$@" ;; shot) cmd_shot "$1" ;;
    status) cmd_status ;; stop) cmd_stop ;; restore) cmd_restore ;;
    *) sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//' | head -n -1; exit 1 ;;
esac
