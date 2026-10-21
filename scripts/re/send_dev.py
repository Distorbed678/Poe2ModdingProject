#!/usr/bin/env python3
"""Send a server command over the debug link, as the DLL overlay does.

The debug link is authenticated per session (HMAC with keys only the client DLL and the server
know), so a script cannot send datagrams itself: it hands the command to the running client DLL,
which seals and sends it (`cmd:` entry of GameFiles/poe-hook.keys, see auto_keys.rs).

Usage: scripts/re/send_dev.py [auto] <command...>

Examples: `area_info G1_1 12345` (new AreaInfo on the live connection), `teleport 1100 300`.
The leading `auto` is accepted for old habits; the client DLL always answers for its own session.
"""
import os
import sys

KEYS = os.path.join(os.path.dirname(__file__), "..", "..", "GameFiles", "poe-hook.keys")


def main():
    args = sys.argv[1:]
    if args and args[0] == "auto":
        args = args[1:]
    if not args:
        sys.exit(__doc__)
    text = " ".join(args)
    with open(KEYS, "a") as f:
        f.write(f"cmd:{text}\n")
    print(f"queued for the client DLL: {text}")


if __name__ == "__main__":
    main()
