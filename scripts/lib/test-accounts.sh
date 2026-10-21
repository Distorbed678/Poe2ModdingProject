# Test accounts for the unattended client tests (sourced by scripts/test-run.sh and scripts/test-mp.sh).
#
# The password-less machine-hash (Steam) login is gone: every test client signs in with the
# standalone login. Without typing a password it uses a Remember Me token: the account is made
# offline (`poe-server account create <name> --password-stdin`), the token comes from the web API (a launcher login, then
# POST /game-token) and `cargo xtask login-token` puts it where the client reads it (docs/guides/setup.md,
# "Remember-me token"). Each account gets a random password in runtime/test-accounts/<name>.pw
# (git-ignored, mode 600, never printed or logged); the web API must be enabled in the config.
#
# The caller defines `srv` (poe-server against its config) and, for ta_token, passes the web port.
TA_DIR=runtime/test-accounts

# ta_ensure <name> [role]: the account exists with the password from its .pw file (made on first use;
# an account that predates the file gets the new password), and has the role.
ta_ensure() {
    local name=$1 role=${2:-player} f="$TA_DIR/$1.pw" fresh=0
    mkdir -p "$TA_DIR"; chmod 700 "$TA_DIR"
    if [ ! -s "$f" ]; then
        (umask 077; LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 24 > "$f")
        fresh=1
    fi
    # The password goes in on standard input (--password-stdin), never on the command line.
    if srv account list 2>/dev/null | grep -qw "$name"; then
        [ "$fresh" = 1 ] && srv account passwd "$name" --password-stdin <"$f" >/dev/null
    else
        srv account create "$name" --password-stdin <"$f" >/dev/null
    fi
    [ "$role" = player ] || srv account role "$name" "$role" >/dev/null
    return 0
}

# ta_token <name> <web port>: a fresh Remember Me token (64 hex digits) on stdout; empty on failure.
# The password and the intermediate launcher token go through stdin and a header file descriptor,
# not through the command line.
ta_token() {
    local name=$1 port=$2 l
    l=$(printf '{"username":"%s","password":"%s","kind":"launcher"}' "$name" "$(cat "$TA_DIR/$name.pw")" \
        | curl -s -X POST "127.0.0.1:$port/api/v1/login" -H 'content-type: application/json' --data-binary @- \
        | sed -n 's/.*"token":"\([0-9a-f]*\)".*/\1/p')
    [ -n "$l" ] || return 0
    curl -s -X POST "127.0.0.1:$port/api/v1/game-token" -H @<(printf 'authorization: Bearer %s\n' "$l") \
        | sed -n 's/.*"token":"\([0-9a-f]*\)".*/\1/p'
}

# ta_web_port <config file>: the port of [web] bind (default 8080).
ta_web_port() {
    local p
    p=$(sed -n '/^\[web\]/,/^\[/p' "$1" | sed -n 's/^bind *= *"[0-9.]*:\([0-9]*\)".*/\1/p' | head -n1)
    echo "${p:-8080}"
}
