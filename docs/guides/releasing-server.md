# Releasing the server

How a `poe-server` release is made: a tag, the CI workflow, what ends up in the package, and the secrets the workflow needs. Users of a release: [hosting.md](hosting.md#getting-a-server-release). The workflow is WP F1 of [../plans/orchestration-2026-10b.md](../plans/orchestration-2026-10b.md); the launcher's "Host a server" page (WP F2) downloads these packages.

## Cutting a release

1. Pick the version. The binary reports the `[workspace.package] version` of `Cargo.toml`; the workflow refuses a tag that disagrees. Bump it (and `Cargo.lock`: run `cargo build`), commit and push to `main`.
2. Tag the commit and push the tag:
   ```sh
   git tag server-v0.1.0
   git push origin server-v0.1.0
   ```
   The tag is `server-v<major>.<minor>.<patch>`, optionally with a suffix (`server-v0.2.0-rc1`; a suffix makes it a pre-release on GitHub, the suffix is dropped when comparing with `Cargo.toml`).
3. Watch the run of [`.github/workflows/release.yml`](../../.github/workflows/release.yml) ("Release server"). When it is green the release `Server <version> (game <game version>)` has the archives, their `.sha256` files and `SHA256SUMS`.

A failed run can be re-run from the Actions page; if the release already exists the files are replaced (`gh release upload --clobber`). To try the build without a release, start the workflow by hand (Actions, "Release server", Run workflow): it builds and packages everything and keeps the archives as workflow artifacts for 14 days; the version is then `dev-<commit>`.

## What the workflow does

| Job | |
|---|---|
| `version` | derives the version from the tag and checks it against `Cargo.toml` |
| `web-admin` | submodule `modules/web-admin`; `npm ci`, `npm run check`, `npm test`, `npm run build`; uploads `dist` |
| `schema` | downloads the community data schema (`poe-tool-dev/dat-schema`, rolling `latest`) and **fails if its SHA-256 is not the one `versions/<game>.toml` pins** (`data.schema_sha256`). The server reads this file at run time and a changed schema can describe other column widths. When upstream moves on: run `cargo xtask check-build` with the new file, update the pin, then release |
| `build` (Linux `x86_64-unknown-linux-gnu` on `ubuntu-latest`, Windows `x86_64-pc-windows-msvc` on `windows-latest`) | submodules `modules/protocol` and `modules/client-hook` (the workspace has path dependencies on them; `poe-server` does not use the hook), the pinned nightly from `rust-toolchain.toml` (`rustup toolchain install`; `poe_data_tools` needs nightly features), `cargo build --release --locked -p poe-server`, on Linux also `cargo test -p poe-config -p poe-accounts -p poe-web-api`, then [`.github/scripts/package-server.sh`](../../.github/scripts/package-server.sh) |
| `release` (tags only) | combines the checksums into `SHA256SUMS`, verifies them and creates (or updates) the GitHub release with `gh` |

The packages are `poe-server-v<version>-game-<game version>-<linux|windows>-x86_64.tar.gz` / `.zip` plus a `.sha256` each. The game version is in the file name so that the launcher can pick the release that matches its installed client without opening an archive; `release.json` inside repeats server version, game version, platform and commit. Contents (the repository layout, since the server's paths are relative to its working directory): `poe-server[.exe]`, `config/server.example.toml`, `versions/`, `modules/protocol/opcodes/`, `third_party/dat-schema/schema-poe2.min.json`, `web-admin/`, `README.md`, `HOSTING.md`, `release.json` and any `LICENSE*` file at the repository root. No game files.

### Checking a package locally

```sh
cargo build --release -p poe-server
(cd modules/web-admin && npm ci && npm run build)
.github/scripts/package-server.sh linux 0.1.0 target/release/poe-server modules/web-admin/dist \
    third_party/dat-schema/schema-poe2.min.json /tmp/pkg
```

The Windows build can be cross-checked on Linux with the tool `third_party/fetch-tools.sh` installs: `XWIN_CACHE_DIR=$PWD/third_party/xwin-cache third_party/cargo-xwin/bin/cargo-xwin xwin build --release -p poe-server --target x86_64-pc-windows-msvc` (use `CARGO_TARGET_DIR=target/xwin-check` to keep it apart from the normal build). It builds `poe_data_tools`, SQLite and everything else without changes; the first run downloads the MSVC CRT and SDK into `third_party/xwin-cache`. `actionlint` (`third_party/actionlint/actionlint .github/workflows/release.yml`) checks the workflow syntax.

## Secrets

While the repositories are private, the workflow reads the three submodule repositories with read-only **deploy keys**, the same approach as the launcher's `ci.yml`. A deploy key can be added to one repository only, so each private repository has its own key pair; the *private* halves go into this repository's Actions secrets (Settings, Secrets and variables, Actions):

| Secret | Holds the private key of | Used by |
|---|---|---|
| `PROTOCOL_DEPLOY_KEY` | the deploy key of `Poe2Protocol` | `build` (`modules/protocol`) |
| `HOOK_DEPLOY_KEY` | the deploy key of `Poe2ClientHook` | `build` (`modules/client-hook`) |
| `WEB_ADMIN_DEPLOY_KEY` | the deploy key of `Poe2WebAdmin` | `web-admin` (`modules/web-admin`) |

The first two are the same keys the launcher repository stores under the same names (a key may be stored as a secret in several repositories, it is attached as a deploy key to one). The release step itself needs no secret: it uses the workflow's own `GITHUB_TOKEN` with `contents: write` for that job only. The composite action [`.github/actions/fetch-submodules`](../../.github/actions/fetch-submodules/action.yml) writes the keys to `~/.ssh` and rewrites the submodule URLs for the runner; a key that is empty is skipped and that submodule is fetched over plain https, so once the repositories are public (WP D3) the three `*-key:` lines in `release.yml` and the secrets can simply be deleted.

Nothing here signs the binaries; the checksums only protect against corrupted downloads and a swapped asset when `SHA256SUMS` comes from another channel.
