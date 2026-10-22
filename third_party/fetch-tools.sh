#!/usr/bin/env bash
# Downloads every external tool used by the project into third_party/.
# Re-runnable: existing downloads are replaced. Requires: gh, git, unzip, cargo (rustup).
set -euo pipefail
cd "$(dirname "$0")"

POE_DATA_TOOLS_CLI=cli-v2.2.1
LIBGGPK3=v2.7.5
GITLEAKS=v8.30.1
ACTIONLINT=v1.7.12
CARGO_XWIN=0.23.1

echo "== poe_data_tools CLI ($POE_DATA_TOOLS_CLI)"
mkdir -p poe_data_tools
gh release download "$POE_DATA_TOOLS_CLI" -R adamthedash/poe_data_tools -p poe_data_tools -D poe_data_tools --clobber
chmod +x poe_data_tools/poe_data_tools

echo "== dat-schema (rolling 'latest'; the sha256 check below only warns, the schema is data, not code)"
mkdir -p dat-schema
gh release download latest -R poe-tool-dev/dat-schema -p 'schema-poe2.min.json' -p 'schema.min.json' -D dat-schema --clobber
# The schema is a rolling download: say when it is not the file the version files were checked with.
got=$(sha256sum dat-schema/schema-poe2.min.json | cut -d' ' -f1)
for f in ../modules/protocol/versions/*.toml; do
    want=$(sed -n 's/^schema_sha256 *= *"\([0-9a-f]*\)".*/\1/p' "$f")
    if [ -n "$want" ] && [ "$want" != "$got" ]; then
        echo "WARNING: dat-schema sha256 $got differs from $(basename "$f") (data.schema_sha256 $want):" \
             "run 'cargo xtask check-build GameFiles/PathOfExileSteam.exe' and update the pin" >&2
    fi
done

echo "== LibGGPK3 ($LIBGGPK3)"
mkdir -p LibGGPK3
gh release download "$LIBGGPK3" -R aianlinb/LibGGPK3 -p linux-x64.zip -D LibGGPK3 --clobber
(cd LibGGPK3 && unzip -oq linux-x64.zip && rm linux-x64.zip)

echo "== gitleaks ($GITLEAKS, secret scanner for the public-release audit; runs locally)"
mkdir -p gitleaks
gl_ver="${GITLEAKS#v}"
gh release download "$GITLEAKS" -R gitleaks/gitleaks -p "gitleaks_${gl_ver}_linux_x64.tar.gz" -p "gitleaks_${gl_ver}_checksums.txt" -D gitleaks --clobber
(cd gitleaks && sha256sum --check --ignore-missing "gitleaks_${gl_ver}_checksums.txt" \
    && tar -xzf "gitleaks_${gl_ver}_linux_x64.tar.gz" gitleaks && rm "gitleaks_${gl_ver}_linux_x64.tar.gz")

echo "== actionlint ($ACTIONLINT, GitHub Actions workflow linter; runs locally)"
mkdir -p actionlint
al_ver="${ACTIONLINT#v}"
gh release download "$ACTIONLINT" -R rhysd/actionlint -p "actionlint_${al_ver}_linux_amd64.tar.gz" -p "actionlint_${al_ver}_checksums.txt" -D actionlint --clobber
(cd actionlint && sha256sum --check --ignore-missing "actionlint_${al_ver}_checksums.txt" \
    && tar -xzf "actionlint_${al_ver}_linux_amd64.tar.gz" actionlint && rm "actionlint_${al_ver}_linux_amd64.tar.gz")

echo "== reference repositories (read-only research material; they track the default branch and are NOT pinned: nothing is built or run from them)"
mkdir -p reference
for repo in adamthedash/poe_data_tools annalithic/poeformats Crypto137/KiwiNet ncatlin/exileSniffer poe-tool-dev/dat-schema; do
    dir="reference/$(basename "$repo")"
    if [ -d "$dir/.git" ]; then git -C "$dir" pull -q --ff-only; else git clone -q --depth 1 "https://github.com/$repo.git" "$dir"; fi
done

echo "== cargo-xwin (Windows MSVC cross-linking for the client DLL)"
cargo +stable install cargo-xwin --version "$CARGO_XWIN" --locked --root cargo-xwin

echo "== pyenv (Python venv for scripts/re: capstone, numpy, pefile, git-filter-repo)"
python3 -m venv --clear pyenv
pyenv/bin/pip install -q capstone==5.0.9 numpy==2.5.4 pefile==2024.8.26 git-filter-repo==2.47.0

echo "== nightly toolchain pinned by rust-toolchain.toml"
rustup toolchain install nightly-2026-08-10 --profile minimal -c rustfmt,clippy,rust-src -t x86_64-pc-windows-msvc

echo "done"
