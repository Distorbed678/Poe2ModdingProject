#!/usr/bin/env python3
"""Drive a terrain seed sweep (docs/re/terrain/README.md): the server (with `POE_SWEEP` or
`[dev] sweep`) generates one listed area/seed per instance session and then closes it; this
script logs the test client in again and presses Play for the next entry, through the DLL's
key file (posted to the game window only, never SendInput).

Usage: scripts/re/sweep_drive.py [max entries]
Start the server in sweep mode and the client with scripts/test-run.sh first; the first entry runs
on the test-run's own login. Results: dumps/0.5.5e.hotfix1/terrain/vectors/sweep-results.tsv; layouts: the DLL's area exports
(dumps/0.5.5e.hotfix1/terrain/area-exports/<session>/).
"""
import re
import sys
import time

LOG = "runtime/server-run.log"
KEYS = "GameFiles/poe-hook.keys"
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def log_count(pattern):
    with open(LOG, errors="replace") as f:
        return sum(1 for line in f if re.search(pattern, ANSI.sub("", line)))


def press_enter():
    with open(KEYS, "w") as f:
        f.write("0x0d\n")


def wait_for(pattern, before, timeout, retry=None):
    """Waits until `pattern` occurs more often than `before`; presses Enter every `retry` s."""
    start = last = time.time()
    while time.time() - start < timeout:
        if log_count(pattern) > before:
            return True
        if retry and time.time() - last > retry:
            press_enter()
            last = time.time()
        time.sleep(0.5)
    return False


def main():
    limit = int(sys.argv[1]) if len(sys.argv) > 1 else 10**9
    done_re = r"terrain sweep entry done|terrain sweep: no hashes"
    done = 0
    # The first entry runs on the test-run's own login.
    if not wait_for(done_re, 0, 120):
        sys.exit("no sweep entry finished (is the server in sweep mode?)")
    while True:
        done = log_count(done_re)
        if log_count(r"terrain sweep finished") or done >= limit:
            break
        print(f"{done} entries done", flush=True)
        auth = log_count(r"authenticated")
        time.sleep(2)
        press_enter()  # dismisses the disconnect message and logs in
        if not wait_for(r"authenticated", auth, 30, retry=6):
            sys.exit("the client did not log in again")
        time.sleep(2)
        press_enter()  # Play
        if not wait_for(done_re, done, 90, retry=15):
            sys.exit("no sweep result within 90 s")
    print(f"sweep finished: {log_count(done_re)} entries")


if __name__ == "__main__":
    main()
