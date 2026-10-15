#!/usr/bin/env python3
"""Send a dev command to the instance server's debug link (what the DLL overlay does).

Usage: tools/re/send_dev.py <client tcp port> <command...>
       tools/re/send_dev.py auto <command...>   (port of the last client in data/server-run.log)

Examples: `area G1_1 12345` (new AreaInfo on the live connection), `teleport 1100 300`.
"""
import re
import socket
import struct
import sys

SERVER = ("127.0.0.1", 6112)


def last_client_port(log="data/server-run.log"):
    port = None
    for line in open(log, errors="replace"):
        line = re.sub(r"\x1b\[[0-9;]*m", "", line)
        m = re.search(r"instance\{peer=127\.0\.0\.1:(\d+)\}.*client connected", line)
        if m:
            port = int(m.group(1))
    if port is None:
        sys.exit("no client in " + log)
    return port


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    port = last_client_port() if sys.argv[1] == "auto" else int(sys.argv[1])
    text = " ".join(sys.argv[2:]).encode()
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.sendto(b"PDC1" + struct.pack("<H", port) + text, SERVER)
    print(f"sent to client port {port}: {text.decode()}")


if __name__ == "__main__":
    main()
