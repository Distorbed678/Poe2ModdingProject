#!/usr/bin/env python3
"""Validates servers.json (and servers.example.json) the way the Poe2 launcher reads it.

    python3 scripts/validate.py [servers.json ...]

Only the Python standard library is used. Exit code 0 = fine, 1 = problems (listed).
The launcher is more forgiving than this script on purpose: it skips a broken entry and keeps
the rest. A pull request must not contain a broken entry, so this script fails on the first
problem of every entry. `servers.schema.json` describes the same rules for editors.
"""
import ipaddress
import json
import re
import sys
from urllib.parse import urlsplit

ID = re.compile(r"^[a-z0-9](?:[a-z0-9-]{1,46})[a-z0-9]$")
TAG = re.compile(r"^[a-z0-9-]{1,24}$")
VERSION = re.compile(r"^[A-Za-z0-9._-]{1,16}$")
MODID = re.compile(r"^[A-Za-z0-9._-]{1,64}$")
KEYS = {"id", "name", "address", "description", "region", "game_version", "tags", "contact", "mods"}
REQUIRED = ("id", "name", "address", "game_version")


def local_host(host: str) -> bool:
    """This computer or a local network (the launcher refuses these in a public list)."""
    h = host.strip("[]").rstrip(".").lower()
    if h == "localhost" or h.endswith(".localhost"):
        return True
    try:
        ip = ipaddress.ip_address(h)
        return ip.is_loopback or ip.is_private or ip.is_link_local or ip.is_unspecified
    except ValueError:
        return "." not in h or h.endswith((".local", ".lan", ".home.arpa", ".internal"))


def check_entry(i, e, seen_ids, seen_addrs):
    p = []
    where = f"entry {i}" + (f" ({e.get('id')})" if isinstance(e, dict) and isinstance(e.get("id"), str) else "")
    if not isinstance(e, dict):
        return [f"{where}: must be an object"]
    for k in e:
        if k not in KEYS:
            p.append(f"{where}: unknown key {k!r}")
    for k in REQUIRED:
        if not isinstance(e.get(k), str) or not e[k].strip():
            p.append(f"{where}: {k} is required (text)")
    if p:
        return p
    if not ID.match(e["id"]):
        p.append(f"{where}: id must be 3-48 characters of a-z, 0-9 and - (no - at either end)")
    if e["id"] in seen_ids:
        p.append(f"{where}: duplicate id")
    seen_ids.add(e["id"])
    if len(e["name"].strip()) > 64:
        p.append(f"{where}: name is at most 64 characters")
    addr = e["address"].strip()
    u = urlsplit(addr if "://" in addr else "https://" + addr)
    if u.scheme not in ("http", "https") or not u.hostname:
        p.append(f"{where}: address must be an http(s) address")
    else:
        if local_host(u.hostname):
            p.append(f"{where}: address {u.hostname} is this computer or a local network; a public list cannot name one")
        if u.scheme == "http":
            print(f"  note: {where}: plain http address; players will be warned (prefer https)")
        if u.query or u.fragment or u.path.strip("/") not in ("", "api/v1"):
            p.append(f"{where}: address is the web API's base address (no path, query or fragment)")
        key = (u.hostname.lower(), u.port)
        if key in seen_addrs:
            p.append(f"{where}: same address as another entry")
        seen_addrs.add(key)
    if len(e.get("description", "")) > 400:
        p.append(f"{where}: description is at most 400 characters")
    if len(e.get("region", "")) > 24:
        p.append(f"{where}: region is at most 24 characters")
    if not VERSION.match(e["game_version"]):
        p.append(f"{where}: game_version looks like 0.5.5e")
    tags = e.get("tags", [])
    if not isinstance(tags, list) or len(tags) > 8 or not all(isinstance(t, str) and TAG.match(t) for t in tags) or len(set(tags)) != len(tags):
        p.append(f"{where}: tags: at most 8, unique, a-z 0-9 and - (24 characters each)")
    if len(e.get("contact", "")) > 120:
        p.append(f"{where}: contact is at most 120 characters")
    mods = e.get("mods", [])
    if not isinstance(mods, list) or len(mods) > 16:
        p.append(f"{where}: mods: a list of at most 16")
    else:
        for m in mods:
            if not isinstance(m, dict) or not isinstance(m.get("id"), str) or not MODID.match(m["id"]):
                p.append(f"{where}: every mod needs an id (a-z, 0-9, . _ -)")
                continue
            if set(m) - {"id", "version", "url"}:
                p.append(f"{where}: mod {m['id']}: unknown key")
            if "url" in m and not (isinstance(m["url"], str) and re.fullmatch(r"https://\S{1,292}", m["url"])):
                p.append(f"{where}: mod {m['id']}: url must be an https link")
            if "version" in m and (not isinstance(m["version"], str) or len(m["version"]) > 32):
                p.append(f"{where}: mod {m['id']}: version is at most 32 characters")
    return p


def check_file(path):
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError) as e:
        return [f"{path}: {e}"]
    problems = []
    if not isinstance(data, dict) or data.get("schema") != 1 or not isinstance(data.get("servers"), list):
        return [f'{path}: must be {{"schema": 1, "servers": [...]}}']
    if set(data) - {"schema", "servers"}:
        problems.append(f"{path}: unknown top-level key")
    if len(data["servers"]) > 500:
        problems.append(f"{path}: at most 500 servers")
    ids, addrs = set(), set()
    for i, e in enumerate(data["servers"]):
        problems += [f"{path}: {m}" for m in check_entry(i, e, ids, addrs)]
    return problems


def main(argv):
    files = argv[1:] or ["servers.json", "servers.example.json"]
    bad = []
    for f in files:
        print(f"checking {f}")
        bad += check_file(f)
    for b in bad:
        print("ERROR", b)
    print("ok" if not bad else f"{len(bad)} problem(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
