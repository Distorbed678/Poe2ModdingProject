#!/usr/bin/env python3
"""Validates index.json (and index.example.json) the way the Poe2 launcher reads it.

    python3 scripts/validate.py [index.json ...]
    python3 scripts/validate.py --online [index.json]   # also downloads every package, checks SHA-256 and size

Only the Python standard library is used. Exit code 0 = fine, 1 = problems (listed).
The launcher is more forgiving on purpose: it skips a broken entry or version and keeps the rest.
A pull request must not contain one, so this script fails on every problem.
`index.schema.json` describes the same rules for editors.
"""
import hashlib
import json
import re
import sys
import urllib.request
from urllib.parse import urlsplit

ID = re.compile(r"^[a-z0-9][a-z0-9_-]{1,63}$")
TAG = re.compile(r"^[a-z0-9-]{1,24}$")
SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
SHA = re.compile(r"^[0-9a-fA-F]{64}$")
SIDES = ("client", "server", "both")
MOD_KEYS = {"id", "name", "description", "author", "side", "tags", "homepage", "licence", "versions"}
VERSION_KEYS = {"version", "released", "game_versions", "hook_api", "server_api", "sha256", "size", "url", "yanked"}
TOP_KEYS = {"schema", "updated", "mods"}
MAX_PACKAGE = 512 << 20


def https_url(u: str) -> bool:
    p = urlsplit(u)
    return p.scheme == "https" and bool(p.hostname) and not p.username and not p.password and len(u) <= 500 \
        and not any(c.isspace() or ord(c) < 32 for c in u)


def is_int(x) -> bool:
    return isinstance(x, int) and not isinstance(x, bool)


def check_version(where, v, seen):
    p = []
    if not isinstance(v, dict):
        return [f"{where}: must be an object"]
    for k in v:
        if k not in VERSION_KEYS:
            p.append(f"{where}: unknown key {k!r}")
    ver = v.get("version")
    if not isinstance(ver, str) or not SEMVER.match(ver):
        p.append(f"{where}: version must be a semantic version (1.2.3)")
    elif ver in seen:
        p.append(f"{where}: version {ver} is listed twice")
    else:
        seen.add(ver)
    if not isinstance(v.get("sha256"), str) or not SHA.match(v["sha256"]):
        p.append(f"{where}: sha256 must be 64 hex digits (the SHA-256 of the whole zip)")
    if not is_int(v.get("size")) or not 1 <= v["size"] <= MAX_PACKAGE:
        p.append(f"{where}: size must be the zip's size in bytes (1 to 512 MiB)")
    if not isinstance(v.get("url"), str) or not https_url(v["url"]):
        p.append(f"{where}: url must be an https:// address of <id>-<version>.zip")
    gv = v.get("game_versions", [])
    if not isinstance(gv, list) or len(gv) > 16 or not all(isinstance(g, str) and 0 < len(g) <= 40 for g in gv):
        p.append(f"{where}: game_versions must be a list of up to 16 version texts")
    for k in ("hook_api", "server_api"):
        if k in v and v[k] is not None and (not is_int(v[k]) or v[k] < 0):
            p.append(f"{where}: {k} must be a non-negative integer")
    if "yanked" in v and not isinstance(v["yanked"], bool):
        p.append(f"{where}: yanked must be true or false")
    if "released" in v and (not isinstance(v["released"], str) or len(v["released"]) > 24):
        p.append(f"{where}: released is a short date text")
    return p


def check_mod(i, m, seen_ids):
    where = f"mods[{i}]" + (f" ({m.get('id')})" if isinstance(m, dict) and isinstance(m.get("id"), str) else "")
    if not isinstance(m, dict):
        return [f"{where}: must be an object"]
    p = []
    for k in m:
        if k not in MOD_KEYS:
            p.append(f"{where}: unknown key {k!r}")
    mid = m.get("id")
    if not isinstance(mid, str) or not ID.match(mid):
        p.append(f"{where}: id must match [a-z0-9][a-z0-9_-]{{1,63}} (the mod.toml id)")
    elif mid in seen_ids:
        p.append(f"{where}: duplicate id")
    else:
        seen_ids.add(mid)
    if not isinstance(m.get("name"), str) or not 1 <= len(m["name"].strip()) <= 80:
        p.append(f"{where}: name is required (up to 80 characters)")
    if m.get("side") not in SIDES:
        p.append(f"{where}: side must be client, server or both")
    for k, n in (("description", 600), ("author", 80), ("licence", 60)):
        if k in m and (not isinstance(m[k], str) or len(m[k]) > n):
            p.append(f"{where}: {k} must be text of up to {n} characters")
    hp = m.get("homepage", "")
    if hp != "" and (not isinstance(hp, str) or not https_url(hp)):
        p.append(f"{where}: homepage must be an https:// address")
    tags = m.get("tags", [])
    if not isinstance(tags, list) or len(tags) > 12 or len(set(map(str, tags))) != len(tags) \
            or not all(isinstance(t, str) and TAG.match(t) for t in tags):
        p.append(f"{where}: tags must be up to 12 different values of a-z, 0-9 and - (24 characters each)")
    versions = m.get("versions")
    if not isinstance(versions, list) or not versions or len(versions) > 200:
        p.append(f"{where}: versions must be a list of 1 to 200 versions")
        return p
    seen = set()
    for j, v in enumerate(versions):
        p += check_version(f"{where} versions[{j}]", v, seen)
    return p


def check_file(path):
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError) as e:
        return [f"{path}: cannot read as JSON: {e}"], None
    return check_data(data, path), data


def check_data(data, name="index"):
    if not isinstance(data, dict):
        return [f"{name}: must be an object"]
    p = []
    for k in data:
        if k not in TOP_KEYS:
            p.append(f"{name}: unknown key {k!r}")
    if data.get("schema") != 1 or isinstance(data.get("schema"), bool):
        p.append(f"{name}: schema must be 1")
    if "updated" in data and (not isinstance(data["updated"], str) or len(data["updated"]) > 40):
        p.append(f"{name}: updated is a short date text")
    mods = data.get("mods")
    if not isinstance(mods, list) or len(mods) > 5000:
        return p + [f"{name}: mods must be a list"]
    seen_ids = set()
    for i, m in enumerate(mods):
        p += [f"{name}: {x}" for x in check_mod(i, m, seen_ids)]
    return p


def check_online(data):
    """Downloads every package and compares size and SHA-256 (what the launcher does before it installs)."""
    p = []
    for m in data.get("mods", []):
        for v in m.get("versions", []):
            try:
                req = urllib.request.Request(v["url"], headers={"User-Agent": "poe2-mod-index-validate"})
                with urllib.request.urlopen(req, timeout=60) as r:
                    body = r.read(MAX_PACKAGE + 1)
            except Exception as e:  # noqa: BLE001 - report, do not crash
                p.append(f"{m['id']} {v['version']}: cannot download {v['url']}: {e}")
                continue
            if len(body) != v["size"]:
                p.append(f"{m['id']} {v['version']}: the file is {len(body)} bytes, the index says {v['size']}")
            if hashlib.sha256(body).hexdigest() != v["sha256"].lower():
                p.append(f"{m['id']} {v['version']}: SHA-256 mismatch")
    return p


def main(argv):
    online = "--online" in argv
    files = [a for a in argv if not a.startswith("--")] or ["index.json"]
    problems = []
    for f in files:
        p, data = check_file(f)
        problems += p
        if online and not p and data:
            problems += check_online(data)
    for line in problems:
        print(line)
    if problems:
        print(f"{len(problems)} problem(s)")
        return 1
    print("ok: " + ", ".join(files))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
