"""The client version the scripts work on: the newest versions/<version>.toml of the repo root
(highest build_revision), or $POE_VERSION. One place, so a new build is a new version file and
not a search-and-replace of "0.5.5e" in the tools (docs/guides/new-game-version.md)."""
import os
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def current_version() -> str:
    if os.environ.get("POE_VERSION"):
        return os.environ["POE_VERSION"]
    newest = None
    for path in (ROOT / "versions").glob("*.toml"):
        data = tomllib.loads(path.read_text())
        key = int(data["build_revision"])
        if newest is None or key > newest[0]:
            newest = (key, data["version"])
    if newest is None:
        raise SystemExit("no versions/*.toml")
    return newest[1]


def dumps_dir() -> Path:
    return ROOT / "dumps" / current_version()
