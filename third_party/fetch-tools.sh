#!/usr/bin/env bash
# Downloads every external tool used by the project into third_party/.
# Re-runnable: existing downloads are replaced. Requires: gh, git, unzip, cargo (rustup).
set -euo pipefail
cd "$(dirname "$0")"

POE_DATA_TOOLS_CLI=cli-v2.2.1
LIBGGPK3=v2.7.5

echo "== poe_data_tools CLI ($POE_DATA_TOOLS_CLI)"
mkdir -p poe_data_tools
gh release download "$POE_DATA_TOOLS_CLI" -R adamthedash/poe_data_tools -p poe_data_tools -D poe_data_tools --clobber
chmod +x poe_data_tools/poe_data_tools

echo "== dat-schema (rolling 'latest')"
mkdir -p dat-schema
gh release download latest -R poe-tool-dev/dat-schema -p 'schema-poe2.min.json' -p 'schema.min.json' -D dat-schema --clobber

echo "== LibGGPK3 ($LIBGGPK3)"
mkdir -p LibGGPK3
gh release download "$LIBGGPK3" -R aianlinb/LibGGPK3 -p linux-x64.zip -D LibGGPK3 --clobber
(cd LibGGPK3 && unzip -oq linux-x64.zip && rm linux-x64.zip)

echo "== reference repositories (read-only, for research)"
mkdir -p reference
for repo in adamthedash/poe_data_tools annalithic/poeformats Crypto137/KiwiNet ncatlin/exileSniffer poe-tool-dev/dat-schema; do
    dir="reference/$(basename "$repo")"
    if [ -d "$dir/.git" ]; then git -C "$dir" pull -q --ff-only; else git clone -q --depth 1 "https://github.com/$repo.git" "$dir"; fi
done

echo "== cargo-xwin (Windows MSVC cross-linking for the client DLL)"
cargo +stable install cargo-xwin --locked --root cargo-xwin

echo "== pyenv (Python venv for scripts/re: capstone, numpy, pefile, git-filter-repo)"
python3 -m venv --clear pyenv
pyenv/bin/pip install -q capstone==5.0.9 numpy==2.5.4 pefile==2024.8.26 git-filter-repo==2.47.0

echo "== nightly toolchain pinned by rust-toolchain.toml"
rustup toolchain install nightly-2026-08-10 --profile minimal -c rustfmt,clippy,rust-src -t x86_64-pc-windows-msvc

echo "done"
