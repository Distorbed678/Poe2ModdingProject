#!/usr/bin/env python3
"""Check the relative links and #anchors of the Markdown docs (stdlib only).

Usage: python3 scripts/check-links.py [file-or-dir ...]
Default: docs/, README.md, CLAUDE.md, CONTRIBUTING.md, SECURITY.md and dumps/README.md of the hub,
plus the Markdown of the server submodule (modules/server: README, CONTRIBUTING, SECURITY, CLAUDE.md,
docs/), skipped with a warning when it is not checked out.

Checks `[text](target)` links outside code blocks and inline code:
- a relative path must exist (files and directories);
- a `#anchor` must match a heading of the target Markdown file (GitHub slugs: lowercase,
  punctuation removed, spaces to hyphens, repeated headings get -1, -2, ...);
- http(s) and mailto links are not checked.
Targets under git-ignored or submodule directories (dumps/, runtime/, third_party/, GameFiles/,
modules/) that are missing only give a warning, because a fresh clone does not have them.
Exit status 1 when a link is broken, 0 otherwise.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULTS = ["docs", "README.md", "CLAUDE.md", "CONTRIBUTING.md", "SECURITY.md", "dumps/README.md"]
# The server repository's own docs: links between them are checked here too.
SERVER_DEFAULTS = [
    "modules/server/README.md",
    "modules/server/CLAUDE.md",
    "modules/server/CONTRIBUTING.md",
    "modules/server/SECURITY.md",
    "modules/server/docs",
]
SOFT_DIRS = {"dumps", "runtime", "third_party", "GameFiles", "modules"}

LINK_RE = re.compile(r"(?<!\!)\[((?:[^\[\]]|\[[^\]]*\])*)\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
HEADING_RE = re.compile(r"^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$")
FENCE_RE = re.compile(r"^ {0,3}(```|~~~)")


def strip_code(line):
    """Remove inline code spans (their text is not a link)."""
    return re.sub(r"`+[^`]*`+", "", line)


def iter_lines(path):
    """Yield (lineno, line) outside fenced code blocks."""
    fence = None
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        m = FENCE_RE.match(line)
        if m:
            if fence is None:
                fence = m.group(1)
            elif m.group(1) == fence:
                fence = None
            continue
        if fence is None:
            yield n, line


def slugify(heading):
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", heading)  # links -> text
    text = text.replace("`", "")
    text = text.lower().strip()
    text = re.sub(r"[^\w\s-]", "", text)
    return re.sub(r"\s", "-", text)


_anchor_cache = {}


def anchors(path):
    if path not in _anchor_cache:
        seen = {}
        slugs = set()
        for _, line in iter_lines(path):
            m = HEADING_RE.match(line)
            if not m:
                continue
            slug = slugify(m.group(2))
            count = seen.get(slug, 0)
            seen[slug] = count + 1
            slugs.add(slug if count == 0 else f"{slug}-{count}")
        _anchor_cache[path] = slugs
    return _anchor_cache[path]


def is_soft(target):
    try:
        rel = target.resolve().relative_to(ROOT)
    except ValueError:
        return False
    return bool(rel.parts) and rel.parts[0] in SOFT_DIRS


def collect(args):
    files = []
    for a in args or DEFAULTS + SERVER_DEFAULTS:
        p = (ROOT / a) if not Path(a).is_absolute() else Path(a)
        if p.is_dir():
            files += sorted(p.rglob("*.md"))
        elif p.exists():
            files.append(p)
        else:
            print(f"warning: {a} does not exist")
    return files


def main():
    errors, warnings, checked = [], [], 0
    for md in collect(sys.argv[1:]):
        rel_md = md.relative_to(ROOT) if md.is_relative_to(ROOT) else md
        for n, line in iter_lines(md):
            for m in LINK_RE.finditer(strip_code(line)):
                target = m.group(2)
                if re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", target):  # http:, mailto:, ...
                    continue
                checked += 1
                path_part, _, anchor = target.partition("#")
                dest = md if not path_part else (md.parent / path_part)
                where = f"{rel_md}:{n}: {target}"
                if not dest.exists():
                    (warnings if is_soft(dest) else errors).append(f"{where}: no such file")
                    continue
                if anchor and dest.is_file() and dest.suffix == ".md":
                    if anchor.lower() not in anchors(dest):
                        errors.append(f"{where}: no heading with this anchor in {dest.name}")
    for w in warnings:
        print("warning:", w)
    for e in errors:
        print("error:", e)
    print(f"{checked} links checked, {len(errors)} broken, {len(warnings)} warnings")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
